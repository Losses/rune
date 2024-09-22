use std::collections::HashSet;

use anyhow::bail;
use anyhow::Result;
use sea_orm::prelude::*;
use sea_orm::ActiveValue;
use sea_orm::QueryOrder;
use chrono::Utc;

use crate::actions::search::CollectionType;
use crate::actions::search::{add_term, remove_term};
use crate::connection::SearchDbConnection;
use crate::entities::{media_file_playlists, playlists};
use crate::{get_all_ids, get_by_id, get_by_ids, get_first_n, get_groups};

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for playlists::Entity {
    fn group_column() -> Self::Column {
        playlists::Column::Group
    }

    fn id_column() -> Self::Column {
        playlists::Column::Id
    }
}

get_groups!(
    get_playlists_groups,
    playlists,
    media_file_playlists,
    PlaylistId
);
get_all_ids!(
    get_media_file_ids_of_playlist,
    media_file_playlists,
    PlaylistId
);
get_by_ids!(get_playlists_by_ids, playlists);
get_by_id!(get_playlist_by_id, playlists);
get_first_n!(list_playlists, playlists);

/// Create a new playlist.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `name` - The name of the new playlist.
/// * `group` - The group to which the playlist belongs.
///
/// # Returns
/// * `Result<Model>` - The created playlist model or an error.
pub async fn create_playlist(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    name: String,
    group: String,
) -> Result<playlists::Model> {
    use playlists::ActiveModel;

    // Create a new playlist active model
    let new_playlist = ActiveModel {
        name: ActiveValue::Set(name.clone()),
        group: ActiveValue::Set(group),
        created_at: ActiveValue::Set(Utc::now().to_rfc3339()),
        updated_at: ActiveValue::Set(Utc::now().to_rfc3339()),
        ..Default::default()
    };

    // Insert the new playlist into the database
    let inserted_playlist = new_playlist.insert(main_db).await?;

    add_term(
        search_db,
        CollectionType::Playlist,
        inserted_playlist.id,
        &name.clone(),
    );

    search_db.w.commit().unwrap();

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
/// * `playlist_id` - The ID of the playlist to update.
/// * `name` - The new name for the playlist.
/// * `group` - The new group for the playlist.
///
/// # Returns
/// * `Result<Model>` - The updated playlist model or an error.
pub async fn update_playlist(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    playlist_id: i32,
    name: Option<String>,
    group: Option<String>,
) -> Result<playlists::Model> {
    use playlists::Entity as PlaylistEntity;

    // Find the playlist by ID
    let playlist = PlaylistEntity::find_by_id(playlist_id).one(main_db).await?;

    if let Some(playlist) = playlist {
        let mut active_model: playlists::ActiveModel = playlist.into();

        // Update the fields if provided
        if let Some(name) = name {
            active_model.name = ActiveValue::Set(name);
        }
        if let Some(group) = group {
            active_model.group = ActiveValue::Set(group);
        }

        active_model.updated_at = ActiveValue::Set(Utc::now().to_rfc3339());

        // Update the playlist in the database
        let updated_playlist = active_model.update(main_db).await?;

        add_term(
            search_db,
            CollectionType::Playlist,
            updated_playlist.id,
            &updated_playlist.name.clone(),
        );

        search_db.w.commit().unwrap();

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
pub async fn remove_playlist(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    playlist_id: i32,
) -> Result<()> {
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
    remove_term(search_db, CollectionType::Playlist, playlist_id);

    search_db.w.commit().unwrap();

    Ok(())
}

/// Add a media file to a playlist.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `playlist_id` - The ID of the playlist to add the item to.
/// * `media_file_id` - The ID of the media file to add.
/// * `position` - The optional position of the media file in the playlist.
///
/// # Returns
/// * `Result<Model>` - The created media file playlist model or an error.
pub async fn add_item_to_playlist(
    main_db: &DatabaseConnection,
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
        let mut active_model: playlists::ActiveModel = playlist.into();
        active_model.updated_at = ActiveValue::Set(Utc::now().to_rfc3339());
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
        // Update the position
        let mut active_model: media_file_playlists::ActiveModel = item.into();
        active_model.position = ActiveValue::Set(new_position);
        let _ = active_model.update(main_db).await?;

        Ok(())
    } else {
        bail!("Media file not found in playlist")
    }
}
