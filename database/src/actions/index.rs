use std::collections::{HashMap, HashSet};

use anyhow::{Error, Result};
use chrono::Utc;
use log::{error, info};
use migration::OnConflict;
use sea_orm::{DatabaseConnection, Set, TransactionTrait};
use sea_orm::{DatabaseTransaction, QuerySelect, prelude::*};
use tokio_util::sync::CancellationToken;

use crate::actions::collection::CollectionQueryType;
use crate::actions::search::{add_term, remove_term};
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
) -> Result<()> {
    info!("Indexing media: {file_ids:?}");

    // Check for cancellation before starting any processing.
    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        info!("Operation cancelled before starting.");
        return Ok(());
    }

    // Retrieve metadata summaries for the given file IDs.
    let metadata_summaries = get_metadata_summary_by_file_ids(main_db, file_ids.clone()).await?;

    for summary in metadata_summaries {
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

        // Process artists for the current media file.
        let artist_result = process_artists(&txn, node_id, &summary, cancel_token).await;
        // Process album for the current media file.
        let album_result = process_album(&txn, node_id, &summary).await;
        // Process genres for the current media file.
        let genre_result = process_genres(&txn, node_id, &summary, cancel_token).await;

        // Commit transaction if all processing is successful, otherwise rollback.
        match (artist_result, album_result, genre_result) {
            (Ok(_), Ok(_), Ok(_)) => {
                if let Err(e) = txn.commit().await {
                    error!("Commit failed for file {}: {}", summary.id, e);
                }
            }
            _ => {
                let _ = txn.rollback().await; // Rollback transaction if any processing failed.
                error!("Processing failed for file {}, rolled back", summary.id);
            }
        }
    }

    Ok(())
}

/// Processes artist information from metadata summary, updating artist records and associations.
///
/// This function parses artist names from the metadata summary, identifies existing artists,
/// inserts new artists if necessary, and updates the relationships between media files and artists
/// in the database. It also handles search term indexing for new artists.
///
/// # Arguments
///
/// * `txn`: A reference to the database transaction.
/// * `summary`: A reference to the metadata summary of the media file.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if artist processing is successful, or an `Err(Error)` if any error occurs.
async fn process_artists(
    txn: &DatabaseTransaction,
    node_id: &str,
    summary: &MetadataSummary,
    cancel_token: Option<&CancellationToken>,
) -> Result<()> {
    // Split and deduplicate artist names from the metadata summary.
    let artist_names: Vec<String> = {
        let names = metadata::artist::split_artists(&summary.artist);
        names
            .into_iter()
            .collect::<HashSet<_>>() // Deduplicate artist names using HashSet.
            .into_iter()
            .collect() // Convert HashSet back to Vec for ordered processing.
    };

    // If no artist names are found, return early.
    if artist_names.is_empty() {
        return Ok(());
    }

    // Check for cancellation token.
    if let Some(token) = cancel_token
        && token.is_cancelled()
    {
        return Err(Error::msg("Operation cancelled"));
    }

    // Fetch existing artists from the database that match the artist names from metadata.
    let existing_artists = artists::Entity::find()
        .filter(artists::Column::Name.is_in(&artist_names))
        .all(txn)
        .await?;

    // Create a HashMap for efficient lookup of existing artists by name.
    let existing_map: HashMap<_, _> = existing_artists
        .into_iter()
        .map(|a| (a.name, a.id)) // Map artist name to artist ID for quick lookup.
        .collect();

    // Identify new artist names that do not exist in the database yet.
    let new_artist_names: Vec<_> = artist_names
        .iter()
        .filter(|name| !existing_map.contains_key(*name)) // Filter out names that are already in existing_map.
        .cloned()
        .collect();

    // Batch insert new artists into the database with conflict handling (do nothing if artist name already exists).
    if !new_artist_names.is_empty() {
        let insert_operation =
            artists::Entity::insert_many(new_artist_names.into_iter().map(|name| {
                artists::ActiveModel {
                    name: Set(name.clone()),                // Set artist name.
                    group: Set(generate_group_name(&name)), // Generate group name for artist.
                    hlc_uuid: Set(Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()).to_string()),
                    created_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    updated_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    created_at_hlc_ver: Set(0),
                    updated_at_hlc_ver: Set(0),
                    created_at_hlc_nid: Set(node_id.to_owned()),
                    updated_at_hlc_nid: Set(node_id.to_owned()),
                    ..Default::default()
                }
            }))
            .on_conflict(
                OnConflict::column(artists::Column::Name) // Define conflict handling on the 'name' column.
                    .do_nothing() // If conflict occurs, do nothing (skip insertion).
                    .to_owned(),
            );

        // Execute the insert operation without expecting any return value.
        insert_operation.exec_without_returning(txn).await?;
    }

    // Retrieve the final set of artists from the database, including newly inserted ones.
    let final_artists = artists::Entity::find()
        .filter(artists::Column::Name.is_in(&artist_names))
        .all(txn)
        .await?;

    // Collect artist IDs and add search terms for newly inserted artists.
    let mut artist_ids = Vec::new();
    for artist in final_artists {
        // Add search term only for artists that were newly inserted in this process.
        if !existing_map.contains_key(&artist.name) {
            add_term(txn, CollectionQueryType::Artist, artist.id, &artist.name).await?;
        }
        artist_ids.push((artist.id, artist.hlc_uuid));
    }

    // Clean up existing artist associations for the media file before creating new ones.
    media_file_artists::Entity::delete_many()
        .filter(media_file_artists::Column::MediaFileId.eq(summary.id)) // Delete associations for the current media file.
        .exec(txn)
        .await?;

    // Insert new artist associations for the media file.
    if !artist_ids.is_empty() {
        media_file_artists::Entity::insert_many(artist_ids.into_iter().map(
            |(artist_id, artist_hlc_uuld)| {
                media_file_artists::ActiveModel {
                    media_file_id: Set(summary.id), // Set media file ID.
                    artist_id: Set(artist_id),      // Set artist ID.
                    hlc_uuid: Set(Uuid::new_v5(
                        &Uuid::NAMESPACE_OID,
                        format!("RUNE_ARTIST_FILE::{artist_hlc_uuld}::{}", summary.file_hash)
                            .as_bytes(),
                    )
                    .to_string()),
                    created_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    updated_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    created_at_hlc_ver: Set(0),
                    updated_at_hlc_ver: Set(0),
                    created_at_hlc_nid: Set(node_id.to_owned()),
                    updated_at_hlc_nid: Set(node_id.to_owned()),
                    ..Default::default()
                }
            },
        ))
        .exec(txn)
        .await?;
    }

    Ok(())
}

/// Processes album information from metadata summary, updating album records and associations.
///
/// This function checks if an album exists in the database, inserts it if not,
/// and updates the relationship between the media file and the album.
/// It also handles search term indexing for new albums.
///
/// # Arguments
///
/// * `txn`: A reference to the database transaction.
/// * `summary`: A reference to the metadata summary of the media file.
///
/// # Returns
///
/// Returns `Ok(())` if album processing is successful, or an `Err(Error)` if any error occurs.
async fn process_album(
    txn: &DatabaseTransaction,
    node_id: &str,
    summary: &MetadataSummary,
) -> Result<()> {
    let album_name = &summary.album;

    // Check if the album already exists in the database.
    let existing_album = albums::Entity::find()
        .filter(albums::Column::Name.eq(album_name)) // Filter by album name.
        .one(txn)
        .await?;

    let (album_id, album_hlc_uuid) = match existing_album {
        Some(existing) => (existing.id, existing.hlc_uuid), // Use existing album ID if found.
        None => {
            let hlc_uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_ALBUM::{album_name}").as_bytes(),
            )
            .to_string();

            let album = albums::ActiveModel {
                name: Set(album_name.clone()),               // Set album name.
                group: Set(generate_group_name(album_name)), // Generate group name for album.
                hlc_uuid: Set(hlc_uuid.clone()),
                created_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                updated_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                created_at_hlc_ver: Set(0),
                updated_at_hlc_ver: Set(0),
                created_at_hlc_nid: Set(node_id.to_owned()),
                updated_at_hlc_nid: Set(node_id.to_owned()),
                ..Default::default()
            };

            // Insert the new album if it doesn't exist.
            let inserted_album = albums::Entity::insert(album).exec(txn).await?;
            // Add search term for the newly inserted album.
            add_term(
                txn,
                CollectionQueryType::Album,
                inserted_album.last_insert_id, // Get the ID of the newly inserted album.
                album_name,
            )
            .await?;
            (inserted_album.last_insert_id, hlc_uuid) // Return the new album ID.
        }
    };

    // Clean up existing album associations for the media file before creating new ones.
    media_file_albums::Entity::delete_many()
        .filter(media_file_albums::Column::MediaFileId.eq(summary.id)) // Delete associations for the current media file.
        .exec(txn)
        .await?;

    // Insert new album association for the media file.
    media_file_albums::Entity::insert(media_file_albums::ActiveModel {
        media_file_id: Set(summary.id),                // Set media file ID.
        album_id: Set(album_id),                       // Set album ID.
        track_number: Set(Some(summary.track_number)), // Set track number from metadata.
        hlc_uuid: Set(Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("RUNE_ALBUM_FILE::{album_hlc_uuid}::{}", summary.file_hash).as_bytes(),
        )
        .to_string()),
        created_at_hlc_ts: Set(Utc::now().to_rfc3339()),
        updated_at_hlc_ts: Set(Utc::now().to_rfc3339()),
        created_at_hlc_ver: Set(0),
        updated_at_hlc_ver: Set(0),
        created_at_hlc_nid: Set(node_id.to_owned()),
        updated_at_hlc_nid: Set(node_id.to_owned()),
        ..Default::default()
    })
    .exec(txn)
    .await?;

    Ok(())
}

/// Processes genre information from metadata summary, updating genre records and associations.
///
/// This function parses genre names from the metadata summary, identifies existing genres,
/// inserts new genres if necessary, and updates the relationships between media files and genres
/// in the database. It also handles search term indexing for new genres.
///
/// # Arguments
///
/// * `txn`: A reference to the database transaction.
/// * `summary`: A reference to the metadata summary of the media file.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if genre processing is successful, or an `Err(Error)` if any error occurs.
async fn process_genres(
    txn: &DatabaseTransaction,
    node_id: &str,
    summary: &MetadataSummary,
    cancel_token: Option<&CancellationToken>,
) -> Result<()> {
    // Split and deduplicate genre names from the metadata summary.
    let genre_names: Vec<String> = {
        let names = metadata::genre::split_genres(&summary.genre);
        names
            .into_iter()
            .collect::<HashSet<_>>() // Deduplicate genre names using HashSet.
            .into_iter()
            .collect() // Convert HashSet back to Vec for ordered processing.
    };

    // If no genre names are found, return early.
    if genre_names.is_empty() {
        return Ok(());
    }

    // Check for cancellation token.
    if let Some(token) = cancel_token
        && token.is_cancelled()
    {
        return Err(Error::msg("Operation cancelled"));
    }

    // Fetch existing genres from the database that match the genre names from metadata.
    let existing_genres = genres::Entity::find()
        .filter(genres::Column::Name.is_in(&genre_names))
        .all(txn)
        .await?;

    // Create a HashMap for efficient lookup of existing genres by name.
    let existing_map: HashMap<_, _> = existing_genres
        .into_iter()
        .map(|g| (g.name, g.id)) // Map genre name to genre ID for quick lookup.
        .collect();

    // Identify new genre names that do not exist in the database yet.
    let new_genre_names: Vec<_> = genre_names
        .iter()
        .filter(|name| !existing_map.contains_key(*name)) // Filter out names that are already in existing_map.
        .cloned()
        .collect();

    // Batch insert new genres into the database with conflict handling (do nothing if genre name already exists).
    if !new_genre_names.is_empty() {
        let insert_operation =
            genres::Entity::insert_many(new_genre_names.into_iter().map(|name| {
                genres::ActiveModel {
                    name: Set(name.clone()),                // Set genre name.
                    group: Set(generate_group_name(&name)), // Generate group name for genre.
                    hlc_uuid: Set(Uuid::new_v5(
                        &Uuid::NAMESPACE_OID,
                        format!("RUNE_GENRES::{name}").as_bytes(),
                    )
                    .to_string()),
                    created_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    updated_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    created_at_hlc_ver: Set(0),
                    updated_at_hlc_ver: Set(0),
                    created_at_hlc_nid: Set(node_id.to_owned()),
                    updated_at_hlc_nid: Set(node_id.to_owned()),
                    ..Default::default()
                }
            }))
            .on_conflict(
                OnConflict::column(genres::Column::Name) // Define conflict handling on the 'name' column.
                    .do_nothing() // If conflict occurs, do nothing (skip insertion).
                    .to_owned(),
            );

        // Execute the insert operation without expecting any return value.
        insert_operation.exec_without_returning(txn).await?;
    }

    // Retrieve the final set of genres from the database, including newly inserted ones.
    let final_genres = genres::Entity::find()
        .filter(genres::Column::Name.is_in(&genre_names))
        .all(txn)
        .await?;

    // Collect genre IDs and add search terms for newly inserted genres.
    let mut genre_ids = Vec::new();
    for genre in final_genres {
        // Add search term only for genres that were newly inserted in this process.
        if !existing_map.contains_key(&genre.name) {
            add_term(txn, CollectionQueryType::Genre, genre.id, &genre.name).await?;
        }
        genre_ids.push((genre.id, genre.hlc_uuid));
    }

    // Clean up existing genre associations for the media file before creating new ones.
    media_file_genres::Entity::delete_many()
        .filter(media_file_genres::Column::MediaFileId.eq(summary.id)) // Delete associations for the current media file.
        .exec(txn)
        .await?;

    // Insert new genre associations for the media file.
    if !genre_ids.is_empty() {
        media_file_genres::Entity::insert_many(genre_ids.into_iter().map(
            |(genre_id, genre_hlc_uuid)| {
                media_file_genres::ActiveModel {
                    media_file_id: Set(summary.id), // Set media file ID.
                    genre_id: Set(genre_id),        // Set genre ID.
                    hlc_uuid: Set(Uuid::new_v5(
                        &Uuid::NAMESPACE_OID,
                        format!("RUNE_GENRES_FILE::{genre_hlc_uuid}::{}", summary.file_hash)
                            .as_bytes(),
                    )
                    .to_string()),
                    created_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    updated_at_hlc_ts: Set(Utc::now().to_rfc3339()),
                    created_at_hlc_ver: Set(0),
                    updated_at_hlc_ver: Set(0),
                    created_at_hlc_nid: Set(node_id.to_owned()),
                    updated_at_hlc_nid: Set(node_id.to_owned()),
                    ..Default::default()
                }
            },
        ))
        .exec(txn)
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
) -> Result<()> {
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
    let consumer = async {
        let mut file_ids = Vec::with_capacity(batch_size); // Initialize vector to hold file IDs for batch processing.

        // Receive files from the channel until the channel is closed.
        while let Ok(file) = rx.recv().await {
            file_ids.push(file.id); // Add file ID to the current batch.

            // Process the batch when it reaches the specified batch size.
            if file_ids.len() >= batch_size {
                process_batch(main_db, node_id, &mut file_ids, cancel_token).await?; // Process the current batch.
            }
        }

        // Process any remaining files in the last batch after the channel is closed.
        if !file_ids.is_empty() {
            process_batch(main_db, node_id, &mut file_ids, cancel_token).await?; // Process the last batch.
        }

        Ok::<(), Error>(())
    };

    // Run the producer and consumer tasks concurrently.
    let (producer_result, consumer_result) = futures::join!(producer, consumer);
    producer_result?; // Propagate errors from producer task.
    consumer_result?; // Propagate errors from consumer task.

    info!("Audio indexing analysis completed.");

    // Perform library maintenance after indexing is completed.
    perform_library_maintenance(main_db, cancel_token).await?;

    info!("Full library indexing and maintenance completed.");

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
) -> Result<()> {
    let batch = std::mem::take(file_ids); // Take ownership of the file IDs for processing and clear the original vector.
    if let Err(e) = index_media_files(db, node_id, batch, cancel_token).await {
        error!("Batch processing failed: {e}"); // Log error if batch processing fails.
    }
    Ok(())
}

/// Cleans up orphaned artist, album, and genre records from the database.
///
/// This function identifies artists, albums, and genres that are no longer associated with any media files
/// and removes them from the database. It also removes associated search terms for these orphaned records.
///
/// # Arguments
///
/// * `db`: A reference to the database connection.
///
/// # Returns
///
/// Returns `Ok(())` if cleanup is successful, or an `Err(Error)` if any error occurs.
pub async fn cleanup_orphaned_records(db: &DatabaseConnection) -> Result<()> {
    info!("Starting cleanup of orphaned artists, albums, and genres");

    // Start a transaction to ensure atomicity of the cleanup process.
    let txn = db.begin().await?;

    // 1. Query all artist IDs that are linked to media files through media_file_artists table.
    let linked_artist_ids: Vec<i32> = media_file_artists::Entity::find()
        .select_only()
        .column(media_file_artists::Column::ArtistId) // Select only the artist_id column.
        .into_tuple() // Convert the result into a tuple of artist IDs.
        .all(&txn)
        .await?;

    // 2. Query all album IDs that are linked to media files through media_file_albums table.
    let linked_album_ids: Vec<i32> = media_file_albums::Entity::find()
        .select_only()
        .column(media_file_albums::Column::AlbumId) // Select only the album_id column.
        .into_tuple() // Convert the result into a tuple of album IDs.
        .all(&txn)
        .await?;

    // 3. Query all genre IDs that are linked to media files through media_file_genres table.
    let linked_genre_ids: Vec<i32> = media_file_genres::Entity::find()
        .select_only()
        .column(media_file_genres::Column::GenreId) // Select only the genre_id column.
        .into_tuple() // Convert the result into a tuple of genre IDs.
        .all(&txn)
        .await?;

    // 4. Find orphaned artists (artists not in the linked_artist_ids list).
    let orphaned_artists = if linked_artist_ids.is_empty() {
        // If no artists are linked, all artists are considered orphaned.
        artists::Entity::find().all(&txn).await?
    } else {
        artists::Entity::find()
            .filter(artists::Column::Id.is_not_in(linked_artist_ids)) // Filter out artists with IDs in linked_artist_ids.
            .all(&txn)
            .await?
    };

    // 5. Find orphaned albums (albums not in the linked_album_ids list).
    let orphaned_albums = if linked_album_ids.is_empty() {
        // If no albums are linked, all albums are considered orphaned.
        albums::Entity::find().all(&txn).await?
    } else {
        albums::Entity::find()
            .filter(albums::Column::Id.is_not_in(linked_album_ids)) // Filter out albums with IDs in linked_album_ids.
            .all(&txn)
            .await?
    };

    // 6. Find orphaned genres (genres not in the linked_genre_ids list).
    let orphaned_genres = if linked_genre_ids.is_empty() {
        // If no genres are linked, all genres are considered orphaned.
        genres::Entity::find().all(&txn).await?
    } else {
        genres::Entity::find()
            .filter(genres::Column::Id.is_not_in(linked_genre_ids)) // Filter out genres with IDs in linked_genre_ids.
            .all(&txn)
            .await?
    };

    info!(
        "Found {} orphaned artists, {} orphaned albums, and {} orphaned genres",
        orphaned_artists.len(),
        orphaned_albums.len(),
        orphaned_genres.len()
    );

    // 7. Delete orphaned artists.
    if !orphaned_artists.is_empty() {
        let artist_ids: Vec<i32> = orphaned_artists.iter().map(|a| a.id).collect(); // Collect IDs of orphaned artists.

        // 7.1 Remove search terms associated with orphaned artists.
        for artist_id in &artist_ids {
            if let Err(e) = remove_term(&txn, CollectionQueryType::Artist, *artist_id).await {
                error!("Failed to remove search terms for artist {artist_id}: {e}");
            }
        }

        // 7.2 Delete orphaned artist records from the database.
        let delete_result = artists::Entity::delete_many()
            .filter(artists::Column::Id.is_in(artist_ids)) // Filter artists to delete by their IDs.
            .exec(&txn)
            .await?;

        info!("Deleted {} orphaned artists", delete_result.rows_affected);
    }

    // 8. Delete orphaned albums.
    if !orphaned_albums.is_empty() {
        let album_ids: Vec<i32> = orphaned_albums.iter().map(|a| a.id).collect(); // Collect IDs of orphaned albums.

        // 8.1 Remove search terms associated with orphaned albums.
        for album_id in &album_ids {
            if let Err(e) = remove_term(&txn, CollectionQueryType::Album, *album_id).await {
                error!("Failed to remove search terms for album {album_id}: {e}");
            }
        }

        // 8.2 Delete orphaned album records from the database.
        let delete_result = albums::Entity::delete_many()
            .filter(albums::Column::Id.is_in(album_ids)) // Filter albums to delete by their IDs.
            .exec(&txn)
            .await?;

        info!("Deleted {} orphaned albums", delete_result.rows_affected);
    }

    // 9. Delete orphaned genres.
    if !orphaned_genres.is_empty() {
        let genre_ids: Vec<i32> = orphaned_genres.iter().map(|g| g.id).collect(); // Collect IDs of orphaned genres.

        // 9.1 Remove search terms associated with orphaned genres.
        for genre_id in &genre_ids {
            if let Err(e) = remove_term(&txn, CollectionQueryType::Genre, *genre_id).await {
                error!("Failed to remove search terms for genre {genre_id}: {e}");
            }
        }

        // 9.2 Delete orphaned genre records from the database.
        let delete_result = genres::Entity::delete_many()
            .filter(genres::Column::Id.is_in(genre_ids)) // Filter genres to delete by their IDs.
            .exec(&txn)
            .await?;

        info!("Deleted {} orphaned genres", delete_result.rows_affected);
    }

    // Commit the transaction to apply all changes.
    txn.commit().await?;
    info!("Cleanup of orphaned records completed successfully");

    Ok(())
}

/// Performs library maintenance tasks, such as cleaning up orphaned records.
///
/// This function serves as an entry point for library maintenance operations.
/// Currently, it only includes cleaning up orphaned artist, album, and genre records.
/// It supports cancellation via a `CancellationToken`.
///
/// # Arguments
///
/// * `db`: A reference to the database connection.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if maintenance is successful, or an `Err(Error)` if any error occurs.
pub async fn perform_library_maintenance(
    db: &DatabaseConnection,
    cancel_token: Option<&CancellationToken>,
) -> Result<()> {
    info!("Starting library maintenance");

    // Check for cancellation before starting maintenance.
    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        info!("Maintenance cancelled before starting");
        return Ok(());
    }

    // Clean up orphaned records.
    if let Err(e) = cleanup_orphaned_records(db).await {
        error!("Failed to clean up orphaned records: {e}");
        return Err(e);
    }

    info!("Library maintenance completed successfully");
    Ok(())
}
