//! Publish the resolved catalog through the current sync protocol.

use std::{collections, collections::HashMap};

use anyhow::{Error, Result};
use metadata::{artist, genre, resolve::LocalMetadata};
use sea_orm::{
    DatabaseTransaction, Set,
    prelude::*,
    sea_query::{Expr, OnConflict},
};
use tokio_util::sync::CancellationToken;

use crate::actions::{
    collection::CollectionQueryType,
    metadata::{MetadataSummary, author_indexed_track_row, get_metadata_summary_by_file_ids},
    search::add_term,
    utils::generate_group_name,
};
use crate::entities::{
    audio_artists, genres, media_file_genres, media_files,
    music_metadata::{album_evidence, credits},
    portable_albums, portable_artists, track_album_sources,
};
use crate::sync::model::adopt_portable_artist;

use super::CreditScope;

pub(in crate::actions) async fn publish_track(
    txn: &DatabaseTransaction,
    file_id: i32,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
    resolved: Option<&LocalMetadata>,
) -> Result<bool> {
    let Some(mut summary) = get_metadata_summary_by_file_ids(txn, vec![file_id])
        .await?
        .into_iter()
        .next()
    else {
        return Ok(false);
    };

    if summary.file_uuid.is_empty() {
        let id = Uuid::new_v4().to_string();
        media_files::Entity::update_many()
            .col_expr(media_files::Column::SourceUuid, Expr::value(id.clone()))
            .filter(media_files::Column::Id.eq(summary.id))
            .exec(txn)
            .await?;
        summary.file_uuid = id;
    }

    let artist_names = match resolved {
        Some(metadata) => metadata.artist_names().collect(),
        None => tag_artists(txn, file_id).await?,
    };
    process_artists(txn, &summary, artist_names, cancel_token, store).await?;
    process_album(txn, &summary).await?;
    process_genres(txn, &summary, cancel_token, store).await?;
    let file = media_files::Entity::find_by_id(summary.id)
        .one(txn)
        .await?
        .ok_or_else(|| Error::msg("indexed file missing"))?;
    author_indexed_track_row(txn, store, file).await?;
    author_album_assignment(txn, store, &summary).await?;
    Ok(true)
}

async fn tag_artists(
    txn: &DatabaseTransaction,
    file_id: i32,
) -> Result<collections::BTreeSet<String>> {
    use credits::Column as C;
    use sea_orm::QuerySelect;

    // Match inline scanning by splitting each source credit independently.
    let credits = credits::Entity::find()
        .select_only()
        .column(C::CreditedAs)
        .filter(C::FileId.eq(file_id))
        .filter(C::Scope.eq(CreditScope::Audio as i32))
        .filter(C::Role.eq("Primary Artist"))
        .filter(C::SourceKind.eq("audio_tag"))
        .into_tuple::<String>()
        .all(txn)
        .await?;
    Ok(credits
        .iter()
        .flat_map(|name| artist::split_artists(name))
        .collect())
}

async fn author_album_assignment(
    txn: &DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    summary: &MetadataSummary,
) -> Result<()> {
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    let link = track_album_sources::Entity::find()
        .filter(track_album_sources::Column::TrackUuid.eq(&summary.file_uuid))
        .one(txn)
        .await?
        .ok_or_else(|| Error::msg("missing album assignment"))?;
    let album = portable_albums::Entity::find()
        .filter(portable_albums::Column::SourceUuid.eq(&link.album_uuid))
        .one(txn)
        .await?
        .ok_or_else(|| Error::msg("missing album"))?;
    let album_id = Uuid::parse_str(&album.source_uuid)?;
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
    summary: &MetadataSummary,
    names: collections::BTreeSet<String>,
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
    anyhow::ensure!(names.len() <= 128, "too many artists for a single track");
    let file_id: Uuid = summary.file_uuid.parse()?;
    let mut desired = HashMap::new();
    for name in names {
        let id = catalog_identity::parent(ParentKind::Artist, &name);
        portable_artists::Entity::insert(portable_artists::ActiveModel {
            name: Set(name.clone()),
            group: Set(generate_group_name(&name)),
            source_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(portable_artists::Column::SourceUuid)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(tx)
        .await?;
        adopt_portable_artist(tx, id).await?;
        let row = portable_artists::Entity::find()
            .filter(portable_artists::Column::SourceUuid.eq(id.to_string()))
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
    let existing = audio_artists::Entity::find()
        .filter(audio_artists::Column::TrackUuid.eq(&summary.file_uuid))
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
                    id: row.source_uuid.parse()?,
                },
            )
            .await?;
        } else {
            store
                .author_on(
                    tx,
                    ObjectKey {
                        collection: Collection::FileArtists,
                        id: row.source_uuid.parse()?,
                    },
                    None,
                )
                .await?;

            audio_artists::Entity::delete_by_id(row.id).exec(tx).await?;
        }
    }
    for (parent, parent_id) in desired {
        let id = catalog_identity::membership(file_id, ParentKind::Artist, parent_id);
        audio_artists::Entity::insert(audio_artists::ActiveModel {
            track_uuid: Set(summary.file_uuid.clone()),
            artist_uuid: Set(parent),
            source_uuid: Set(id.to_string()),
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

async fn process_album(tx: &DatabaseTransaction, summary: &MetadataSummary) -> Result<()> {
    use migration::catalog_identity::{self, ParentKind};
    use sea_orm::QuerySelect;
    // Existing devices key their album assignments by the original tag spelling.
    let title = album_evidence::Entity::find_by_id(summary.id)
        .select_only()
        .column(album_evidence::Column::TagTitle)
        .into_tuple::<Option<String>>()
        .one(tx)
        .await?
        .flatten()
        .unwrap_or_default();
    let album_id = catalog_identity::parent(ParentKind::Album, &title);
    portable_albums::Entity::insert(portable_albums::ActiveModel {
        group: Set(generate_group_name(&title)),
        name: Set(title),
        source_uuid: Set(album_id.to_string()),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(portable_albums::Column::SourceUuid)
            .update_columns([
                portable_albums::Column::Name,
                portable_albums::Column::Group,
            ])
            .action_and_where(portable_albums::Column::Name.is_null())
            .to_owned(),
    )
    .exec_without_returning(tx)
    .await?;
    let album = portable_albums::Entity::find()
        .filter(portable_albums::Column::SourceUuid.eq(album_id.to_string()))
        .one(tx)
        .await?
        .ok_or_else(|| Error::msg("missing indexed album"))?;
    add_term(tx, CollectionQueryType::Album, album.id, &album.name).await?;
    track_album_sources::Entity::insert(track_album_sources::ActiveModel {
        track_uuid: Set(summary.file_uuid.clone()),
        album_uuid: Set(album_id.to_string()),
        track_number: Set(summary.track_number),
        source_uuid: Set(summary.file_uuid.clone()),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(track_album_sources::Column::TrackUuid)
            .update_columns([
                track_album_sources::Column::AlbumUuid,
                track_album_sources::Column::TrackNumber,
            ])
            .to_owned(),
    )
    .exec_without_returning(tx)
    .await?;
    Ok(())
}

async fn process_genres(
    tx: &DatabaseTransaction,
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
    let names: collections::BTreeSet<_> = genre::split_genres(&summary.genre).into_iter().collect();
    anyhow::ensure!(names.len() <= 128, "too many genres for a single track");
    let file_id: Uuid = summary.file_uuid.parse()?;
    let mut desired = HashMap::new();
    for name in names {
        let id = catalog_identity::parent(ParentKind::Genre, &name);
        genres::Entity::insert(genres::ActiveModel {
            name: Set(name.clone()),
            group: Set(generate_group_name(&name)),
            source_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(genres::Column::SourceUuid)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(tx)
        .await?;
        let row = genres::Entity::find()
            .filter(genres::Column::SourceUuid.eq(id.to_string()))
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
                    id: row.source_uuid.parse()?,
                },
            )
            .await?;
        } else {
            store
                .author_on(
                    tx,
                    ObjectKey {
                        collection: Collection::FileGenres,
                        id: row.source_uuid.parse()?,
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
            source_uuid: Set(id.to_string()),
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
