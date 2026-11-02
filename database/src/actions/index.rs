use std::collections::HashMap;

use anyhow::{Error, Result};
use log::{error, info};
use migration::OnConflict;
use sea_orm::{DatabaseConnection, Set, TransactionTrait};
use sea_orm::{DatabaseTransaction, prelude::*};
use tokio_util::sync::CancellationToken;

use crate::actions::collection::CollectionQueryType;
use crate::actions::search::add_term;
use crate::actions::utils::generate_group_name;
use crate::entities::{
    albums, artists, genres, media_file_albums, media_file_artists, media_file_genres, media_files,
};

use super::metadata::{MetadataSummary, get_metadata_summary_by_file_ids};

/// Indexes media files by processing their metadata and updating database records for artists, albums, and genres.
///
/// This function processes a list of media file IDs, retrieves metadata summaries for each file,
/// and then iterates through each summary to update artist, album, and genre information in the database.
/// Each file is processed within its own database transaction to ensure atomicity.
/// It also supports cancellation via a `CancellationToken`.
///
/// # Arguments
///
/// * `main_db`: A reference to the database connection.
/// * `file_ids`: A vector of media file IDs to index.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if the indexing process is successful, or an `Err(Error)` if any error occurs.
pub async fn index_media_files(
    main_db: &DatabaseConnection,
    node_id: &str,
    file_ids: Vec<i32>,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    info!("Indexing media: {file_ids:?}");

    // Check for cancellation before starting any processing.
    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        info!("Operation cancelled before starting.");
        return Ok(());
    }

    for file_id in file_ids {
        // Start a new transaction for each file to ensure individual processing atomicity.
        let txn = match main_db.begin().await {
            Ok(txn) => txn,
            Err(e) => {
                error!("Failed to start transaction: {e}");
                continue; // Proceed to the next file if transaction start fails.
            }
        };

        // Check for cancellation during processing of each file.
        if let Some(token) = &cancel_token
            && token.is_cancelled()
        {
            info!("Operation cancelled during processing.");
            let _ = txn.rollback().await; // Rollback the current transaction if cancelled.
            return Ok(());
        }

        // Derive relationships from the current source inside the writer transaction.
        store.lock(&txn).await?;
        let Some(mut summary) = get_metadata_summary_by_file_ids(&txn, vec![file_id])
            .await?
            .into_iter()
            .next()
        else {
            txn.rollback().await?;
            continue;
        };

        if summary.file_uuid.is_empty() {
            let id = Uuid::new_v4().to_string();
            media_files::Entity::update_many()
                .col_expr(
                    media_files::Column::HlcUuid,
                    sea_orm::sea_query::Expr::value(id.clone()),
                )
                .filter(media_files::Column::Id.eq(summary.id))
                .exec(&txn)
                .await?;
            summary.file_uuid = id;
        }

        // Process artists for the current media file.
        let artist_result = process_artists(&txn, node_id, &summary, cancel_token, store).await;
        // Process album for the current media file.
        let album_result = process_album(&txn, node_id, &summary).await;
        // Process genres for the current media file.
        let genre_result = process_genres(&txn, node_id, &summary, cancel_token, store).await;

        // Commit transaction if all processing is successful, otherwise rollback.
        match (&artist_result, &album_result, &genre_result) {
            (Ok(_), Ok(_), Ok(_)) => {
                let file = media_files::Entity::find_by_id(summary.id)
                    .one(&txn)
                    .await?
                    .ok_or_else(|| Error::msg("indexed file missing"))?;
                if let Err(error) =
                    crate::actions::metadata::author_indexed_track_row(&txn, store, file).await
                {
                    txn.rollback().await?;
                    return Err(error);
                }
                if let Err(error) = author_album_assignment(&txn, store, &summary).await {
                    txn.rollback().await?;
                    return Err(error);
                }

                txn.commit().await?;
            }
            _ => {
                let _ = txn.rollback().await; // Rollback transaction if any processing failed.
                return Err(Error::msg(format!(
                    "Indexing file {} failed: artist={:?}, album={:?}, genre={:?}",
                    summary.id,
                    artist_result.err(),
                    album_result.err(),
                    genre_result.err()
                )));
            }
        }
    }

    Ok(())
}

async fn author_album_assignment(
    txn: &DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    summary: &MetadataSummary,
) -> Result<()> {
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    let link = media_file_albums::Entity::find()
        .filter(media_file_albums::Column::TrackUuid.eq(&summary.file_uuid))
        .one(txn)
        .await?
        .ok_or_else(|| Error::msg("missing album assignment"))?;
    let album = albums::Entity::find()
        .filter(albums::Column::HlcUuid.eq(&link.album_uuid))
        .one(txn)
        .await?
        .ok_or_else(|| Error::msg("missing album"))?;
    let album_id = Uuid::parse_str(&album.hlc_uuid)?;
    let track_id = Uuid::parse_str(&summary.file_uuid)?;
    let album_key = ObjectKey {
        collection: Collection::Albums,
        id: album_id,
    };
    author_source_on(txn, store, album_key).await?;
    author_source_on(
        txn,
        store,
        ObjectKey {
            collection: Collection::TrackAlbums,
            id: track_id,
        },
    )
    .await?;
    Ok(())
}

async fn process_artists(
    tx: &DatabaseTransaction,
    _node_id: &str,
    summary: &MetadataSummary,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    use migration::catalog_identity::{self, ParentKind};
    use sea_orm::QuerySelect;
    anyhow::ensure!(
        !cancel_token.is_some_and(|token| token.is_cancelled()),
        "operation cancelled"
    );
    let names: std::collections::BTreeSet<_> = metadata::artist::split_artists(&summary.artist)
        .into_iter()
        .collect();
    anyhow::ensure!(names.len() <= 128, "too many artists for a single track");
    let file_id: Uuid = summary.file_uuid.parse()?;
    let mut desired = HashMap::new();
    for name in names {
        let id = catalog_identity::parent(ParentKind::Artist, &name);
        artists::Entity::insert(artists::ActiveModel {
            name: Set(name.clone()),
            group: Set(generate_group_name(&name)),
            hlc_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(artists::Column::HlcUuid)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(tx)
        .await?;
        let row = artists::Entity::find()
            .filter(artists::Column::HlcUuid.eq(id.to_string()))
            .one(tx)
            .await?
            .ok_or_else(|| Error::msg("missing indexed artist"))?;
        add_term(tx, CollectionQueryType::Artist, row.id, &row.name).await?;

        author_source_on(
            tx,
            store,
            ObjectKey {
                collection: Collection::Artists,
                id,
            },
        )
        .await?;

        desired.insert(id.to_string(), id);
    }
    let existing = media_file_artists::Entity::find()
        .filter(media_file_artists::Column::TrackUuid.eq(&summary.file_uuid))
        .limit(129)
        .all(tx)
        .await?;
    anyhow::ensure!(
        existing.len() <= 128,
        "too many existing artists for a single track"
    );
    for row in existing {
        if desired.remove(&row.artist_uuid).is_some() {
            author_source_on(
                tx,
                store,
                ObjectKey {
                    collection: Collection::FileArtists,
                    id: row.hlc_uuid.parse()?,
                },
            )
            .await?;
        } else {
            store
                .author_on(
                    tx,
                    ObjectKey {
                        collection: Collection::FileArtists,
                        id: row.hlc_uuid.parse()?,
                    },
                    None,
                )
                .await?;

            media_file_artists::Entity::delete_by_id(row.id)
                .exec(tx)
                .await?;
        }
    }
    for (parent, parent_id) in desired {
        let id = catalog_identity::membership(file_id, ParentKind::Artist, parent_id);
        media_file_artists::Entity::insert(media_file_artists::ActiveModel {
            track_uuid: Set(summary.file_uuid.clone()),
            artist_uuid: Set(parent),
            hlc_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .exec(tx)
        .await?;

        author_source_on(
            tx,
            store,
            ObjectKey {
                collection: Collection::FileArtists,
                id,
            },
        )
        .await?;
    }
    Ok(())
}

async fn process_album(
    tx: &DatabaseTransaction,
    _node_id: &str,
    summary: &MetadataSummary,
) -> Result<()> {
    use migration::catalog_identity::{self, ParentKind};
    let album_id = catalog_identity::parent(ParentKind::Album, &summary.album);
    albums::Entity::insert(albums::ActiveModel {
        name: Set(summary.album.clone()),
        group: Set(generate_group_name(&summary.album)),
        hlc_uuid: Set(album_id.to_string()),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(albums::Column::HlcUuid)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(tx)
    .await?;
    let album = albums::Entity::find()
        .filter(albums::Column::HlcUuid.eq(album_id.to_string()))
        .one(tx)
        .await?
        .ok_or_else(|| Error::msg("missing indexed album"))?;
    add_term(tx, CollectionQueryType::Album, album.id, &album.name).await?;
    media_file_albums::Entity::insert(media_file_albums::ActiveModel {
        track_uuid: Set(summary.file_uuid.clone()),
        album_uuid: Set(album_id.to_string()),
        track_number: Set(Some(summary.track_number)),
        hlc_uuid: Set(summary.file_uuid.clone()),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(media_file_albums::Column::TrackUuid)
            .update_columns([
                media_file_albums::Column::AlbumUuid,
                media_file_albums::Column::TrackNumber,
            ])
            .to_owned(),
    )
    .exec_without_returning(tx)
    .await?;
    Ok(())
}

async fn process_genres(
    tx: &DatabaseTransaction,
    _node_id: &str,
    summary: &MetadataSummary,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    use migration::catalog_identity::{self, ParentKind};
    use sea_orm::QuerySelect;
    anyhow::ensure!(
        !cancel_token.is_some_and(|token| token.is_cancelled()),
        "operation cancelled"
    );
    let names: std::collections::BTreeSet<_> = metadata::genre::split_genres(&summary.genre)
        .into_iter()
        .collect();
    anyhow::ensure!(names.len() <= 128, "too many genres for a single track");
    let file_id: Uuid = summary.file_uuid.parse()?;
    let mut desired = HashMap::new();
    for name in names {
        let id = catalog_identity::parent(ParentKind::Genre, &name);
        genres::Entity::insert(genres::ActiveModel {
            name: Set(name.clone()),
            group: Set(generate_group_name(&name)),
            hlc_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(genres::Column::HlcUuid)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(tx)
        .await?;
        let row = genres::Entity::find()
            .filter(genres::Column::HlcUuid.eq(id.to_string()))
            .one(tx)
            .await?
            .ok_or_else(|| Error::msg("missing indexed genre"))?;
        add_term(tx, CollectionQueryType::Genre, row.id, &row.name).await?;

        author_source_on(
            tx,
            store,
            ObjectKey {
                collection: Collection::Genres,
                id,
            },
        )
        .await?;

        desired.insert(id.to_string(), id);
    }
    let existing = media_file_genres::Entity::find()
        .filter(media_file_genres::Column::TrackUuid.eq(&summary.file_uuid))
        .limit(129)
        .all(tx)
        .await?;
    anyhow::ensure!(
        existing.len() <= 128,
        "too many existing genres for a single track"
    );
    for row in existing {
        if desired.remove(&row.genre_uuid).is_some() {
            author_source_on(
                tx,
                store,
                ObjectKey {
                    collection: Collection::FileGenres,
                    id: row.hlc_uuid.parse()?,
                },
            )
            .await?;
        } else {
            store
                .author_on(
                    tx,
                    ObjectKey {
                        collection: Collection::FileGenres,
                        id: row.hlc_uuid.parse()?,
                    },
                    None,
                )
                .await?;

            media_file_genres::Entity::delete_by_id(row.id)
                .exec(tx)
                .await?;
        }
    }
    for (parent, parent_id) in desired {
        let id = catalog_identity::membership(file_id, ParentKind::Genre, parent_id);
        media_file_genres::Entity::insert(media_file_genres::ActiveModel {
            track_uuid: Set(summary.file_uuid.clone()),
            genre_uuid: Set(parent),
            hlc_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .exec(tx)
        .await?;

        author_source_on(
            tx,
            store,
            ObjectKey {
                collection: Collection::FileGenres,
                id,
            },
        )
        .await?;
    }
    Ok(())
}
/// Indexes the entire audio library in batches, processing media files in parallel.
///
/// This function fetches media files in batches from the database and processes them
/// using `index_media_files`. It utilizes an asynchronous channel to pass files from
/// a producer task (fetching files) to a consumer task (indexing files), allowing for
/// concurrent processing. After indexing is complete, it performs library maintenance.
///
/// # Arguments
///
/// * `main_db`: A reference to the database connection.
/// * `batch_size`: The number of media files to process in each batch.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if the library indexing is successful, or an `Err(Error)` if any error occurs.
pub async fn index_audio_library(
    main_db: &DatabaseConnection,
    node_id: &str,
    batch_size: usize,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    anyhow::ensure!(batch_size > 0, "index batch size must be positive");
    let mut cursor = media_files::Entity::find().cursor_by(media_files::Column::Id); // Create a cursor for batch fetching.
    let (tx, rx) = async_channel::bounded(batch_size); // Create an async channel for file batching.

    // Producer task: Fetches media files in batches from the database and sends them to the channel.
    let producer = async {
        loop {
            // Fetch the next batch of files from the database using the cursor.
            let files: Vec<media_files::Model> =
                cursor.first(batch_size.try_into()?).all(main_db).await?;

            if files.is_empty() {
                info!("No more files to process. Exiting loop.");
                break; // Exit loop if no more files are found.
            }

            // Send each file to the consumer via the channel.
            for file in &files {
                tx.send(file.clone()).await?;
            }

            // Move the cursor to the next batch based on the last file's ID.
            if let Some(last_file) = files.last() {
                info!("Moving cursor after file ID: {}", last_file.id);
                cursor.after(last_file.id);
            } else {
                break; // Exit loop if there was an issue getting the last file.
            }
        }

        drop(tx); // Close the channel to signal consumers to stop after all files are sent.
        Ok::<(), Error>(())
    };

    // Consumer task: Receives media files from the channel and processes them in batches.
    let consumer = async move {
        let mut file_ids = Vec::with_capacity(batch_size); // Initialize vector to hold file IDs for batch processing.

        // Receive files from the channel until the channel is closed.
        while let Ok(file) = rx.recv().await {
            file_ids.push(file.id); // Add file ID to the current batch.

            // Process the batch when it reaches the specified batch size.
            if file_ids.len() >= batch_size {
                process_batch(main_db, node_id, &mut file_ids, cancel_token, store).await?; // Process the current batch.
            }
        }

        // Process any remaining files in the last batch after the channel is closed.
        if !file_ids.is_empty() {
            process_batch(main_db, node_id, &mut file_ids, cancel_token, store).await?; // Process the last batch.
        }

        Ok::<(), Error>(())
    };

    // Run the producer and consumer tasks concurrently.
    let (producer_result, consumer_result) = futures::join!(producer, consumer);
    producer_result?; // Propagate errors from producer task.
    consumer_result?; // Propagate errors from consumer task.

    info!("Audio indexing analysis completed.");

    Ok(())
}

/// Processes a batch of file IDs by calling `index_media_files`.
///
/// This is a helper function to handle batch processing of media files. It takes a mutable
/// vector of file IDs, processes them using `index_media_files`, and clears the vector.
///
/// # Arguments
///
/// * `db`: A reference to the database connection.
/// * `file_ids`: A mutable vector of file IDs to process.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if batch processing is successful, or an `Err(Error)` if any error occurs.
async fn process_batch(
    db: &DatabaseConnection,
    node_id: &str,
    file_ids: &mut Vec<i32>,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    let batch = std::mem::take(file_ids);
    index_media_files(db, node_id, batch, cancel_token, store).await
}
