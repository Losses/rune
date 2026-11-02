use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::ActiveValue;
use sea_orm::{QueryOrder, QuerySelect};
use sea_orm::{TransactionTrait, prelude::*};
use tokio::fs::read_to_string;

use crate::actions::collection::CollectionQuery;
use crate::actions::search::{add_term, remove_term};
use crate::connection::MainDbConnection;
use crate::entities::media_file_playlists as entries;
use crate::entities::{media_file_playlists, media_files, playlists};
use crate::{collection_query, get_by_id};

use super::collection::CollectionQueryType;
use super::utils::{CollectionDefinition, DatabaseExecutor};

impl CollectionDefinition for playlists::Entity {
    fn group_column() -> Self::Column {
        playlists::Column::Group
    }

    fn id_column() -> Self::Column {
        playlists::Column::Id
    }
}

get_by_id!(get_playlist_by_id, playlists);

collection_query!(
    playlists,
    CollectionQueryType::Playlist,
    "lib::playlist".to_owned(),
    media_file_playlists,
    PlaylistUuid
);

use crate::sync::{
    SeaOrmSyncStore,
    model::{Collection, ObjectKey, author_source_on},
};
use anyhow::{Context, ensure};
use sea_orm::{Condition, DatabaseTransaction, Set};
/// Get all playlists.
///
/// # Arguments
/// * `db` - A reference to the database connection.
///
/// # Returns
/// * `Result<Vec<Model>>` - The playlists or an error.
pub async fn get_all_playlists(db: &DatabaseConnection) -> Result<Vec<playlists::Model>> {
    use playlists::Entity as PlaylistEntity;

    // Find the playlist by ID
    let playlist = PlaylistEntity::find()
        .order_by_asc(playlists::Column::Group)
        .all(db)
        .await?;

    Ok(playlist)
}

/// Stable continuation over source occurrences, including repeated files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccurrenceCursor {
    pub position: i32,
    pub occurrence_id: String,
}

#[derive(Clone, Debug)]
pub struct Occurrence {
    pub occurrence_id: String,
    pub position: i32,
    pub track_id: Uuid,
    pub file: Option<media_files::Model>,
}

#[derive(Clone, Debug)]
pub struct OccurrencePage {
    pub entries: Vec<Occurrence>,
    pub next: Option<OccurrenceCursor>,
}

/// Read materialized membership without filtering on local audio availability.
pub async fn list_occurrences(
    db: &DatabaseConnection,
    playlist_id: i32,
    after: Option<OccurrenceCursor>,
    limit: u32,
) -> Result<OccurrencePage> {
    ensure!(
        (1..=128).contains(&limit),
        "playlist page size must be 1..=128"
    );
    let parent = playlists::Entity::find_by_id(playlist_id)
        .one(db)
        .await?
        .context("playlist not found")?;
    let mut query =
        entries::Entity::find().filter(entries::Column::PlaylistUuid.eq(parent.hlc_uuid));
    if let Some(after) = after {
        let id: Uuid = after
            .occurrence_id
            .parse()
            .context("invalid occurrence cursor")?;
        ensure!(!id.is_nil(), "invalid occurrence cursor");
        query = query.filter(
            Condition::any()
                .add(entries::Column::Position.gt(after.position))
                .add(
                    Condition::all()
                        .add(entries::Column::Position.eq(after.position))
                        .add(entries::Column::HlcUuid.gt(after.occurrence_id)),
                ),
        );
    }
    let mut rows = query
        .order_by_asc(entries::Column::Position)
        .order_by_asc(entries::Column::HlcUuid)
        .limit(u64::from(limit) + 1)
        .find_also_related(media_files::Entity)
        .all(db)
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if more {
        rows.last().map(|(row, _)| OccurrenceCursor {
            position: row.position,
            occurrence_id: row.hlc_uuid.clone(),
        })
    } else {
        None
    };
    let entries = rows
        .into_iter()
        .map(|(row, file)| {
            Ok(Occurrence {
                occurrence_id: row.hlc_uuid,
                position: row.position,
                track_id: row.track_uuid.parse()?,
                file,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(OccurrencePage { entries, next })
}

#[derive(Debug)]
pub struct PlaylistImportResult {
    pub matched_ids: Vec<i32>,
    pub unmatched_paths: Vec<String>,
    pub ambiguous_paths: Vec<String>,
}

pub async fn parse_m3u8_playlist<E>(
    main_db: &E,
    playlist_path: &Path,
) -> Result<PlaylistImportResult>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let content = read_to_string(playlist_path).await?;
    resolve_m3u8(main_db, &parse_m3u8_lines(&content)).await
}

fn parse_m3u8_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

async fn resolve_m3u8<E>(main_db: &E, lines: &[String]) -> Result<PlaylistImportResult>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut matched_ids = Vec::new();
    let mut unmatched_paths = Vec::new();
    let mut ambiguous_paths = Vec::new();
    for line in lines {
        // Convert the line into a PathBuf object
        let path = PathBuf::from(line);
        // Extract the file name from the path, if possible
        let file_name = path.file_name().and_then(|n| n.to_str()).map(String::from);

        // If a file name was successfully extracted
        if let Some(file_name) = file_name {
            // Query the database for files with the same file name
            let matching_files = media_files::Entity::find()
                .filter(media_files::Column::FileName.eq(file_name.clone()))
                .all(main_db)
                .await?;

            // Handle different cases based on the number of matching files found
            match matching_files.len() {
                0 => {
                    // No matching files found, add the path to unmatched_paths
                    unmatched_paths.push(line.to_string());
                }
                1 => {
                    // Exactly one matching file found, add its ID to matched_ids
                    matched_ids.push(matching_files[0].id);
                }
                _ => {
                    // More than one matching file found, need to disambiguate using directory paths
                    let mut path_components: Vec<String> = path
                        .parent()
                        .map(|p| {
                            // Split the path into components and reverse them for comparison
                            p.components()
                                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                                .collect()
                        })
                        .unwrap_or_default();
                    path_components.reverse();

                    // Prepare a vector of tuples containing each file and its directory components
                    let mut matches_with_paths: Vec<_> = matching_files
                        .into_iter()
                        .map(|file| {
                            let file_path = PathBuf::from(&file.directory);
                            let mut components: Vec<String> = file_path
                                .components()
                                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                                .collect();
                            components.reverse();
                            (file, components)
                        })
                        .collect();

                    let mut matched = false;
                    // Iterate over the components of the path to find the best match
                    for (i, component) in path_components.iter().enumerate() {
                        // Retain only the files whose directory components match the current component
                        matches_with_paths.retain(|(_, file_components)| {
                            file_components
                                .get(i)
                                .map(|c| c == component)
                                .unwrap_or(false)
                        });

                        // If no matches are left, add the path to unmatched_paths and break
                        if matches_with_paths.is_empty() {
                            unmatched_paths.push(line.to_string());
                            matched = true;
                            break;
                        }

                        // If only one match is left, add its ID to matched_ids and break
                        if matches_with_paths.len() == 1 {
                            matched_ids.push(matches_with_paths[0].0.id);
                            matched = true;
                            break;
                        }
                    }

                    // If still multiple matches exist, select the one with the highest ID
                    if !matched {
                        if matches_with_paths.len() > 1 {
                            ambiguous_paths.push(line.to_string());
                        }
                        if let Some((file, _)) =
                            matches_with_paths.into_iter().max_by_key(|(f, _)| f.id)
                        {
                            matched_ids.push(file.id);
                        } else {
                            unmatched_paths.push(line.to_string());
                        }
                    }
                }
            }
        } else {
            // If no file name could be extracted, add the path to unmatched_paths
            unmatched_paths.push(line.to_string());
        }
    }

    // Return the result containing matched file IDs and unmatched paths
    Ok(PlaylistImportResult {
        matched_ids,
        unmatched_paths,
        ambiguous_paths,
    })
}

async fn create_on(
    tx: &DatabaseTransaction,
    actor: Uuid,
    name: String,
    group: String,
) -> Result<playlists::Model> {
    use playlists::ActiveModel;

    // Create a new playlist active model
    let new_playlist = ActiveModel {
        name: ActiveValue::Set(name.clone()),
        group: ActiveValue::Set(group),
        hlc_uuid: ActiveValue::Set(
            Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("RUNE_PLAYLIST::{name}::{}", Utc::now().to_rfc3339()).as_bytes(),
            )
            .to_string(),
        ),
        created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        created_at_hlc_ver: ActiveValue::Set(0),
        updated_at_hlc_ver: ActiveValue::Set(0),
        created_at_hlc_nid: ActiveValue::Set(actor.to_string()),
        updated_at_hlc_nid: ActiveValue::Set(actor.to_string()),
        ..Default::default()
    };

    // Insert the new playlist into the database
    let inserted_playlist = new_playlist.insert(tx).await?;

    add_term(
        tx,
        CollectionQueryType::Playlist,
        inserted_playlist.id,
        &name.clone(),
    )
    .await?;

    Ok(inserted_playlist)
}
async fn author(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    collection: Collection,
    id: &str,
) -> Result<()> {
    author_source_on(
        tx,
        store,
        ObjectKey {
            collection,
            id: id.parse()?,
        },
    )
    .await
}

pub async fn create_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    name: String,
    group: String,
) -> Result<playlists::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = create_on(&tx, actor, name, group).await?;
    author(&tx, store, Collection::Playlists, &row.hlc_uuid).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn update_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    id: i32,
    name: Option<String>,
    group: Option<String>,
) -> Result<playlists::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let old = playlists::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .context("playlist not found")?;
    let ver = old.updated_at_hlc_ver;
    let mut active: playlists::ActiveModel = old.into();
    if let Some(v) = name {
        active.name = Set(v);
    }
    if let Some(v) = group {
        active.group = Set(v);
    }
    active.updated_at_hlc_ts = Set(Utc::now().to_rfc3339());
    active.updated_at_hlc_ver = Set(ver.checked_add(1).context("playlist version exhausted")?);
    active.updated_at_hlc_nid = Set(actor.to_string());
    let row = active.update(&tx).await?;
    add_term(&tx, CollectionQueryType::Playlist, id, &row.name).await?;
    author(&tx, store, Collection::Playlists, &row.hlc_uuid).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn remove_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    id: i32,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = playlists::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .context("playlist not found")?;
    playlists::Entity::delete_by_id(id).exec(&tx).await?;
    remove_term(&tx, CollectionQueryType::Playlist, id).await?;
    store
        .author_on(
            &tx,
            ObjectKey {
                collection: Collection::Playlists,
                id: row.hlc_uuid.parse()?,
            },
            None,
        )
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn add_on(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    actor: Option<Uuid>,
    parent: &playlists::Model,
    file: &media_files::Model,
    position: i32,
) -> Result<media_file_playlists::Model> {
    let mut entry = media_file_playlists::ActiveModel {
        playlist_uuid: Set(parent.hlc_uuid.clone()),
        track_uuid: Set(file.hlc_uuid.clone()),
        position: Set(position),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    };
    if let Some(actor) = actor {
        entry.created_at_hlc_nid = Set(actor.to_string());
        entry.updated_at_hlc_nid = Set(actor.to_string());
    }
    let row = media_file_playlists::Entity::insert(entry)
        .exec_with_returning(tx)
        .await?;
    author(tx, store, Collection::PlaylistEntries, &row.hlc_uuid).await?;
    Ok(row)
}

pub async fn add_item_to_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    playlist_id: i32,
    file_id: i32,
    position: Option<i32>,
) -> Result<media_file_playlists::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let parent = playlists::Entity::find_by_id(playlist_id)
        .one(&tx)
        .await?
        .context("playlist not found")?;
    let file = media_files::Entity::find_by_id(file_id)
        .one(&tx)
        .await?
        .context("file not found")?;
    let position = match position {
        Some(p) => p,
        None => media_file_playlists::Entity::find()
            .filter(media_file_playlists::Column::PlaylistUuid.eq(&parent.hlc_uuid))
            .order_by_desc(media_file_playlists::Column::Position)
            .one(&tx)
            .await?
            .map(|r| {
                r.position
                    .checked_add(1)
                    .context("playlist position exhausted")
            })
            .transpose()?
            .unwrap_or(0),
    };
    let row = add_on(&tx, store, None, &parent, &file, position).await?;
    let ver = parent.updated_at_hlc_ver;
    let mut active: playlists::ActiveModel = parent.into();
    active.updated_at_hlc_ts = Set(Utc::now().to_rfc3339());
    active.updated_at_hlc_ver = Set(ver.checked_add(1).context("playlist version exhausted")?);
    active.updated_at_hlc_nid = Set(actor.to_string());
    active.update(&tx).await?;
    tx.commit().await?;
    Ok(row)
}

async fn find_occurrence(
    tx: &DatabaseTransaction,
    playlist_id: i32,
    occurrence_id: Uuid,
) -> Result<media_file_playlists::Model> {
    ensure!(!occurrence_id.is_nil(), "invalid playlist occurrence");
    let parent = playlists::Entity::find_by_id(playlist_id)
        .one(tx)
        .await?
        .context("playlist not found")?;
    media_file_playlists::Entity::find()
        .filter(media_file_playlists::Column::PlaylistUuid.eq(parent.hlc_uuid))
        .filter(media_file_playlists::Column::HlcUuid.eq(occurrence_id.to_string()))
        .one(tx)
        .await?
        .context("playlist occurrence not found")
}

pub async fn remove_item_from_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    playlist_id: i32,
    occurrence_id: Uuid,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = find_occurrence(&tx, playlist_id, occurrence_id).await?;
    media_file_playlists::Entity::delete_by_id(row.id)
        .exec(&tx)
        .await?;
    store
        .author_on(
            &tx,
            ObjectKey {
                collection: Collection::PlaylistEntries,
                id: occurrence_id,
            },
            None,
        )
        .await?;
    let parent = playlists::Entity::find_by_id(playlist_id)
        .one(&tx)
        .await?
        .context("playlist not found")?;
    let ver = parent.updated_at_hlc_ver;
    let mut active: playlists::ActiveModel = parent.into();
    active.updated_at_hlc_ts = Set(Utc::now().to_rfc3339());
    active.updated_at_hlc_ver = Set(ver.checked_add(1).context("playlist version exhausted")?);
    active.updated_at_hlc_nid = Set(actor.to_string());
    active.update(&tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reorder_playlist_item_position(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    playlist_id: i32,
    occurrence_id: Uuid,
    position: i32,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = find_occurrence(&tx, playlist_id, occurrence_id).await?;
    if row.position != position {
        let ver = row.updated_at_hlc_ver;
        let mut active: media_file_playlists::ActiveModel = row.into();
        active.position = Set(position);
        active.updated_at_hlc_ts = Set(Utc::now().to_rfc3339());
        active.updated_at_hlc_ver =
            Set(ver.checked_add(1).context("occurrence version exhausted")?);
        active.updated_at_hlc_nid = Set(actor.to_string());
        let row = active.update(&tx).await?;
        author(&tx, store, Collection::PlaylistEntries, &row.hlc_uuid).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn import_on(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    parent: &playlists::Model,
    lines: &[String],
) -> Result<PlaylistImportResult> {
    let result = resolve_m3u8(tx, lines).await?;
    for (position, id) in result.matched_ids.iter().enumerate() {
        let file = media_files::Entity::find_by_id(*id)
            .one(tx)
            .await?
            .context("file not found")?;
        add_on(
            tx,
            store,
            Some(actor),
            parent,
            &file,
            i32::try_from(position)?,
        )
        .await?;
    }
    Ok(result)
}

pub async fn import_m3u8_to_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    playlist_id: i32,
    path: &Path,
) -> Result<PlaylistImportResult> {
    let lines = parse_m3u8_lines(&read_to_string(path).await?);
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let parent = playlists::Entity::find_by_id(playlist_id)
        .one(&tx)
        .await?
        .context("playlist not found")?;
    let result = import_on(&tx, store, actor, &parent, &lines).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn create_m3u8_playlist(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    actor: Uuid,
    name: String,
    group: String,
    path: &Path,
) -> Result<(playlists::Model, PlaylistImportResult)> {
    let lines = parse_m3u8_lines(&read_to_string(path).await?);
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = create_on(&tx, actor, name, group).await?;
    author(&tx, store, Collection::Playlists, &row.hlc_uuid).await?;
    let result = import_on(&tx, store, actor, &row, &lines).await?;
    tx.commit().await?;
    Ok((row, result))
}
