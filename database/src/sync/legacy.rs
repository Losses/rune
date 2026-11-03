//! One-time discard of the alpha catalog namespace, before its provenance is lost.
use super::{
    SeaOrmSyncStore,
    model::{Collection, ObjectKey, Payload, RuneModel},
};
use crate::entities::{media_cover_art, media_file_stats, media_files};
use anyhow::{Context, Result, ensure};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect, TransactionTrait,
};
use std::collections::BTreeSet;
use sync::SyncModel;
use sync_orm::{causal_schema, row_policy::ScanLimits};
use uuid::Uuid;

pub(super) fn namespace() -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://rune.audio/catalog")
}

pub(super) async fn reset(db: &DatabaseConnection, actor: Uuid) -> Result<()> {
    let store = SeaOrmSyncStore::open(db.clone(), namespace(), actor).await?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let local_tracks: BTreeSet<Uuid> = media_files::Entity::find()
        .select_only()
        .column(media_files::Column::HlcUuid)
        .filter(media_files::Column::LocalAvailable.eq(true))
        .into_tuple::<String>()
        .all(&tx)
        .await?
        .into_iter()
        .map(|id| id.parse())
        .collect::<Result<_, _>>()?;
    let local_art: BTreeSet<Uuid> = media_cover_art::Entity::find()
        .select_only()
        .column(media_cover_art::Column::HlcUuid)
        .filter(media_cover_art::Column::LocalAvailable.eq(true))
        .filter(media_cover_art::Column::HlcUuid.ne(""))
        .filter(media_cover_art::Column::HlcUuid.ne(Uuid::nil().to_string()))
        .into_tuple::<String>()
        .all(&tx)
        .await?
        .into_iter()
        .map(|id| id.parse())
        .collect::<Result<_, _>>()?;
    let mut retained = BTreeSet::new();
    let mut rows = Vec::new();
    for collection in RuneModel::collections() {
        let mut after = None;
        loop {
            let page = RuneModel::scan_on(
                &tx,
                &collection,
                after,
                ScanLimits {
                    max_records: 128,
                    max_rows: 1024,
                    max_bytes: 4 * 1024 * 1024,
                    max_mutations: 128,
                },
            )
            .await
            .with_context(|| {
                format!("inspect legacy {}", RuneModel::collection_tag(&collection))
            })?;
            for row in page.records {
                let key = ObjectKey {
                    collection,
                    id: row.id,
                };
                let record = store.get_on(&tx, &key).await?;
                // Unpublished source rows predate replication and are local input.
                let authored_here = record.is_none_or(|record| record.revision.actor == actor);
                let keep = match &row.payload {
                    Payload::Track(_) => local_tracks.contains(&key.id),
                    Payload::PlaybackStatistics(value) => value.device_id == actor,
                    Payload::FileArtist(value) => local_tracks.contains(&value.file_id),
                    Payload::FileGenre(value) => local_tracks.contains(&value.file_id),
                    Payload::TrackAlbum(value) => local_tracks.contains(&value.track_id),
                    // Own personal rows about a peer-only track would dangle once it is removed.
                    Payload::Liked(value) => {
                        local_tracks.contains(&value.track_id) && authored_here
                    }
                    Payload::PlaylistEntry(value) => {
                        local_tracks.contains(&value.track_id) && authored_here
                    }
                    Payload::Artwork(_) => local_art.contains(&key.id) || authored_here,
                    Payload::Fingerprint(value) => {
                        local_tracks.contains(&value.file_id) && authored_here
                    }
                    Payload::Similarity(value) => {
                        local_tracks.contains(&value.first_file_id)
                            && local_tracks.contains(&value.second_file_id)
                            && authored_here
                    }
                    _ => authored_here,
                };
                if keep {
                    retained.insert(key.clone());
                    // Keep catalog descriptions required by locally backed files,
                    // without pulling remote tracks in through personal lists.
                    if matches!(
                        collection,
                        Collection::Tracks
                            | Collection::FileArtists
                            | Collection::FileGenres
                            | Collection::TrackAlbums
                    ) {
                        for (tag, id) in RuneModel::references(&row.payload) {
                            if let Some(collection) = RuneModel::collection_from_tag(tag)
                                && collection != Collection::Tracks
                            {
                                retained.insert(ObjectKey { collection, id });
                            }
                        }
                    }
                }
                rows.push(key);
            }
            if page.done {
                break;
            }
            ensure!(page.next != after, "legacy traversal made no progress");
            after = page.next;
        }
    }
    let mut removed = 0;
    for key in &rows {
        if !retained.contains(key) {
            if key.collection == Collection::PlaybackStatistics {
                // This is a local namespace discard, not a replicated owner removal.
                media_file_stats::Entity::delete_many()
                    .filter(media_file_stats::Column::HlcUuid.eq(key.id.to_string()))
                    .exec(&tx)
                    .await?;
            } else {
                RuneModel::project_on(&tx, key, None)
                    .await
                    .with_context(|| {
                        format!(
                            "remove peer-only legacy {} {}",
                            RuneModel::collection_tag(&key.collection),
                            key.id
                        )
                    })?;
            }
            removed += 1;
        }
    }
    // Source cleanup and every engine-owned relation change atomically. A crash
    // after commit leaves an empty engine that the requested namespace can open;
    // the normal bounded import can resume after any subsequent interruption.
    let engine_rows = causal_schema::clear_namespace_on(&tx, namespace()).await?;
    tx.commit().await?;
    log::info!(
        "Reset legacy library namespace: kept {} source rows ({} local tracks), removed {removed} peer-only rows and {engine_rows} engine rows",
        rows.len() - removed,
        local_tracks.len()
    );
    Ok(())
}
