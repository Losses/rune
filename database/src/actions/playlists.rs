use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::Utc;
use log::info;
use sea_orm::ActiveValue;
use sea_orm::QueryOrder;
use sea_orm::{TransactionTrait, prelude::*};
use tokio::fs::read_to_string;

use crate::actions::collection::CollectionQuery;
use crate::actions::search::{add_term, remove_term};
use crate::connection::MainDbConnection;
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
    PlaylistId
);

/// Create a new playlist.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `node_id` - The id of the client that triggers the operation.
/// * `name` - The name of the new playlist.
/// * `group` - The group to which the playlist belongs.
///
/// # Returns
/// * `Result<Model>` - The created playlist model or an error.
pub async fn create_playlist<E>(
    main_db: &E,
    node_id: &str,
    name: String,
    group: String,
) -> Result<playlists::Model>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
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
        created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        ..Default::default()
    };

    // Insert the new playlist into the database
    let inserted_playlist = new_playlist.insert(main_db).await?;

    add_term(
        main_db,
        CollectionQueryType::Playlist,
        inserted_playlist.id,
        &name.clone(),
    )
    .await?;

    Ok(inserted_playlist)
}

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

/// Update an existing playlist.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `node_id` - The id of the client that triggers the operation.
/// * `playlist_id` - The ID of the playlist to update.
/// * `name` - The new name for the playlist.
/// * `group` - The new group for the playlist.
///
/// # Returns
/// * `Result<Model>` - The updated playlist model or an error.
pub async fn update_playlist(
    main_db: &DatabaseConnection,
    node_id: &str,
    playlist_id: i32,
    name: Option<String>,
    group: Option<String>,
) -> Result<playlists::Model> {
    use playlists::Entity as PlaylistEntity;

    // Find the playlist by ID
    let playlist = PlaylistEntity::find_by_id(playlist_id).one(main_db).await?;

    if let Some(playlist) = playlist {
        let ver = playlist.updated_at_hlc_ver;
        let mut active_model: playlists::ActiveModel = playlist.into();

        // Update the fields if provided
        if let Some(name) = name {
            active_model.name = ActiveValue::Set(name);
        }
        if let Some(group) = group {
            active_model.group = ActiveValue::Set(group);
        }

        active_model.updated_at_hlc_ts = ActiveValue::Set(Utc::now().to_rfc3339());
        active_model.updated_at_hlc_ver = ActiveValue::Set(ver + 1);
        active_model.updated_at_hlc_nid = ActiveValue::Set(node_id.to_owned());

        // Update the playlist in the database
        let updated_playlist = active_model.update(main_db).await?;

        add_term(
            main_db,
            CollectionQueryType::Playlist,
            updated_playlist.id,
            &updated_playlist.name.clone(),
        )
        .await?;

        Ok(updated_playlist)
    } else {
        bail!("Playlist not found");
    }
}

/// Remove a playlist by its ID.
///
/// # Arguments
/// * `main_db` - A reference to the main database connection.
/// * `search_db` - A mutable reference to the search database connection.
/// * `playlist_id` - The ID of the playlist to delete.
///
/// # Returns
/// * `Result<()>` - An empty result or an error.
pub async fn remove_playlist(main_db: &DatabaseConnection, playlist_id: i32) -> Result<()> {
    use media_file_playlists::Entity as MediaFilePlaylistEntity;
    use playlists::Entity as PlaylistEntity;

    // Check if the playlist exists
    let playlist = PlaylistEntity::find_by_id(playlist_id).one(main_db).await?;
    if playlist.is_none() {
        bail!("Playlist not found");
    }

    // Delete all media file associations with this playlist
    MediaFilePlaylistEntity::delete_many()
        .filter(media_file_playlists::Column::PlaylistId.eq(playlist_id))
        .exec(main_db)
        .await?;

    // Delete the playlist itself
    PlaylistEntity::delete_by_id(playlist_id)
        .exec(main_db)
        .await?;

    // Remove the playlist term from the search database
    remove_term(main_db, CollectionQueryType::Playlist, playlist_id).await?;

    Ok(())
}

/// Add a media file to a playlist.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `node_id` - The id of the client that triggers the operation.
/// * `playlist_id` - The ID of the playlist to add the item to.
/// * `media_file_id` - The ID of the media file to add.
/// * `position` - The optional position of the media file in the playlist.
///
/// # Returns
/// * `Result<Model>` - The created media file playlist model or an error.
pub async fn add_item_to_playlist(
    main_db: &DatabaseConnection,
    node_id: &str,
    playlist_id: i32,
    media_file_id: i32,
    position: Option<i32>,
) -> Result<media_file_playlists::Model> {
    use media_file_playlists::Entity as MediaFilePlaylistEntity;
    use playlists::Entity as PlaylistEntity;

    // Determine the position to insert the item
    let position = match position {
        Some(pos) => pos,
        _none => {
            // If no position is provided, find the current maximum position and insert at the end
            MediaFilePlaylistEntity::find()
                .filter(media_file_playlists::Column::PlaylistId.eq(playlist_id))
                .order_by_desc(media_file_playlists::Column::Position)
                .one(main_db)
                .await?
                .map_or(0, |item| item.position + 1)
        }
    };

    // Create a new media file playlist active model
    let new_media_file_playlist = media_file_playlists::ActiveModel {
        playlist_id: ActiveValue::Set(playlist_id),
        media_file_id: ActiveValue::Set(media_file_id),
        position: ActiveValue::Set(position),
        ..Default::default()
    };

    // Insert the new media file playlist into the database
    let media_file_playlist = new_media_file_playlist.insert(main_db).await?;

    // Find the playlist by ID
    let playlist = PlaylistEntity::find_by_id(playlist_id).one(main_db).await?;

    if let Some(playlist) = playlist {
        let ver = playlist.updated_at_hlc_ver;
        let mut active_model: playlists::ActiveModel = playlist.into();
        active_model.updated_at_hlc_ts = ActiveValue::Set(Utc::now().to_rfc3339());
        active_model.created_at_hlc_ver = ActiveValue::Set(ver + 1);
        active_model.updated_at_hlc_nid = ActiveValue::Set(node_id.to_owned());
        let _ = active_model.update(main_db).await?;
    } else {
        bail!("Playlist not found")
    }

    Ok(media_file_playlist)
}

/// Reorder a media file in a playlist.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `playlist_id` - The ID of the playlist containing the item to reorder.
/// * `media_file_id` - The ID of the media file to reorder.
/// * `new_position` - The new position for the media file.
///
/// # Returns
/// * `Result<()>` - An empty result or an error.
pub async fn reorder_playlist_item_position(
    main_db: &DatabaseConnection,
    node_id: &str,
    playlist_id: i32,
    media_file_id: i32,
    new_position: i32,
) -> Result<()> {
    use media_file_playlists::Entity as MediaFilePlaylistEntity;

    // Find the media file playlist item
    let item = MediaFilePlaylistEntity::find()
        .filter(media_file_playlists::Column::PlaylistId.eq(playlist_id))
        .filter(media_file_playlists::Column::MediaFileId.eq(media_file_id))
        .one(main_db)
        .await?;

    if let Some(item) = item {
        let ver = item.updated_at_hlc_ver;
        // Update the position
        let mut active_model: media_file_playlists::ActiveModel = item.into();
        active_model.position = ActiveValue::Set(new_position);
        active_model.updated_at_hlc_ts = ActiveValue::Set(Utc::now().to_rfc3339());
        active_model.updated_at_hlc_ver = ActiveValue::Set(ver + 1);
        active_model.updated_at_hlc_nid = ActiveValue::Set(node_id.to_owned());

        // Update the media file playlist item in the database
        let _ = active_model.update(main_db).await?;

        Ok(())
    } else {
        bail!("Media file not found in playlist")
    }
}

#[derive(Debug)]
pub struct PlaylistImportResult {
    pub matched_ids: Vec<i32>,
    pub unmatched_paths: Vec<String>,
}

pub async fn parse_m3u8_playlist<E>(
    main_db: &E,
    playlist_path: &Path,
) -> Result<PlaylistImportResult>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    // Read the content of the M3U8 file asynchronously into a string
    let content = read_to_string(playlist_path).await?;
    // Initialize vectors to store matched file IDs and unmatched paths
    let mut matched_ids = Vec::new();
    let mut unmatched_paths = Vec::new();

    // Iterate over each line in the content, filtering out empty lines
    for line in content.lines().filter_map(|l| {
        // Trim whitespace from the line
        let trimmed = l.trim();
        // If the line is empty after trimming, return None; otherwise, return the trimmed line
        if trimmed.is_empty() || trimmed.starts_with("#") {
            None
        } else {
            Some(trimmed)
        }
    }) {
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
    })
}

pub async fn import_m3u8_to_playlist<E>(
    main_db: &E,
    node_id: &str,
    playlist_id: i32,
    playlist_path: &Path,
) -> Result<PlaylistImportResult>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let import_result = parse_m3u8_playlist(main_db, playlist_path).await?;

    let models: Vec<media_file_playlists::ActiveModel> = import_result
        .matched_ids
        .iter()
        .enumerate()
        .map(
            |(index, &media_file_id)| media_file_playlists::ActiveModel {
                playlist_id: ActiveValue::Set(playlist_id),
                media_file_id: ActiveValue::Set(media_file_id),
                position: ActiveValue::Set(index as i32),
                hlc_uuid: ActiveValue::Set(
                    Uuid::new_v5(
                        &Uuid::NAMESPACE_URL,
                        format!("RUNE_PLAYLIST::{playlist_id}::{media_file_id}").as_bytes(),
                    )
                    .to_string(),
                ),
                created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                created_at_hlc_ver: ActiveValue::Set(0),
                updated_at_hlc_ver: ActiveValue::Set(0),
                created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                ..Default::default()
            },
        )
        .collect();

    if !models.is_empty() {
        media_file_playlists::Entity::insert_many(models)
            .exec(main_db)
            .await?;
    }

    Ok(import_result)
}

pub async fn create_m3u8_playlist(
    main_db: &MainDbConnection,
    node_id: &str,
    name: String,
    group: String,
    m3u8_path: &Path,
) -> Result<(playlists::Model, PlaylistImportResult)> {
    let txn = main_db.begin().await?;

    // Create the playlist
    let playlist: playlists::Model =
        create_playlist(&txn, node_id, name.clone(), group.clone()).await?;

    // Import the M3U8 file contents into the playlist
    let import_result = import_m3u8_to_playlist(&txn, node_id, playlist.id, m3u8_path).await;

    // Check if the import was successful
    match import_result {
        Ok(result) => {
            // Commit the transaction if successful
            txn.commit().await?;
            Ok((playlist, result))
        }
        Err(e) => {
            // Rollback the transaction if there was an error
            txn.rollback().await?;
            Err(e)
        }
    }
}

/// Remove a specific item from a playlist by position.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `node_id` - The id of the client that triggers the operation.
/// * `playlist_id` - The ID of the playlist containing the item.
/// * `media_file_id` - The ID of the media file to remove.
/// * `position` - The exact position of the item in the playlist.
///
/// # Returns
/// * `Result<()>` - An empty result or an error.
pub async fn remove_item_from_playlist(
    main_db: &DatabaseConnection,
    node_id: &str,
    playlist_id: i32,
    media_file_id: i32,
    position: i32,
) -> Result<()> {
    use media_file_playlists::Entity as MediaFilePlaylistEntity;
    use playlists::Entity as PlaylistEntity;

    let txn = main_db.begin().await?;

    info!("Removing item {media_file_id}(pos: {position}) from playlist {playlist_id}");
    let delete_result = MediaFilePlaylistEntity::delete_many()
        .filter(media_file_playlists::Column::PlaylistId.eq(playlist_id))
        .filter(media_file_playlists::Column::MediaFileId.eq(media_file_id))
        .filter(media_file_playlists::Column::Position.eq(position))
        .exec(&txn)
        .await?;

    if delete_result.rows_affected == 0 {
        bail!("Playlist item not found at specified position");
    }

    MediaFilePlaylistEntity::update_many()
        .col_expr(
            media_file_playlists::Column::Position,
            Expr::col(media_file_playlists::Column::Position).sub(1),
        )
        .filter(media_file_playlists::Column::PlaylistId.eq(playlist_id))
        .filter(media_file_playlists::Column::Position.gt(position))
        .exec(&txn)
        .await?;

    PlaylistEntity::update_many()
        .col_expr(
            playlists::Column::UpdatedAtHlcTs,
            Expr::value(Utc::now().to_rfc3339()),
        )
        .col_expr(
            playlists::Column::UpdatedAtHlcNid,
            Expr::value(node_id.to_owned()),
        )
        .col_expr(
            playlists::Column::UpdatedAtHlcVer,
            Expr::add(Expr::col(playlists::Column::UpdatedAtHlcVer), 1),
        )
        .filter(playlists::Column::Id.eq(playlist_id))
        .exec(&txn)
        .await?;

    txn.commit().await?;

    Ok(())
}
