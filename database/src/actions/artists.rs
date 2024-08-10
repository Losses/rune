use std::collections::HashMap;
use std::collections::HashSet;

use sea_orm::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QuerySelect};

use crate::entities::media_cover_art;
use crate::entities::{artists, media_file_artists, media_files};

pub async fn count_artists_by_first_letter(
    db: &DatabaseConnection,
) -> Result<Vec<(String, i32)>, DbErr> {
    let results = artists::Entity::find()
        .select_only()
        .column(artists::Column::Group)
        .column_as(artists::Column::Id.count(), "count")
        .group_by(artists::Column::Group)
        .into_tuple::<(String, i32)>()
        .all(db)
        .await?;

    Ok(results)
}

type ArtistGroupParseResult = (artists::Model, HashSet<i32>);

pub async fn get_artists_groups(
    db: &DatabaseConnection,
    groups: Vec<String>,
) -> Result<Vec<(String, Vec<ArtistGroupParseResult>)>, sea_orm::DbErr> {
    // Step 0: Get the magic coverart ID
    let magic_cover_art = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::FileHash.eq(String::new()))
        .one(db)
        .await;

    let magic_cover_art_id = magic_cover_art.ok().flatten().map_or(-1, |s| s.id);

    // Step 1: Fetch artists belonging to the specified groups
    let artists: Vec<artists::Model> = artists::Entity::find()
        .filter(artists::Column::Group.is_in(groups.clone()))
        .all(db)
        .await?;

    // Step 2: Collect artist IDs
    let artist_ids: Vec<i32> = artists.iter().map(|x| x.id).collect();

    // Step 3: Fetch related media files for these artists
    let media_files = artists::Entity::find()
        .filter(artists::Column::Id.is_in(artist_ids.clone()))
        .find_with_related(media_file_artists::Entity)
        .all(db)
        .await?
        .into_iter()
        .flat_map(|(artist, media_file_artist_vec)| {
            media_file_artist_vec
                .into_iter()
                .map(move |media_file_artist| (artist.id, media_file_artist))
        })
        .collect::<Vec<_>>();

    // Step 4: Map artist IDs to their media file IDs
    let mut artist_to_media_file_ids: HashMap<i32, Vec<i32>> = HashMap::new();
    for (artist_id, media_file_artist) in media_files {
        artist_to_media_file_ids
            .entry(artist_id)
            .or_default()
            .push(media_file_artist.media_file_id);
    }

    // Step 5: Map artist IDs to their cover IDs
    let mut artist_to_cover_ids: HashMap<i32, HashSet<i32>> = HashMap::new();
    for (artist_id, media_file_ids) in artist_to_media_file_ids {
        let media_files = media_files::Entity::find()
            .filter(media_files::Column::Id.is_in(media_file_ids))
            .filter(media_files::Column::CoverArtId.ne(magic_cover_art_id))
            .all(db)
            .await?;

        let cover_ids = media_files
            .into_iter()
            .filter_map(|media_file| media_file.cover_art_id)
            .collect::<HashSet<i32>>();

        artist_to_cover_ids.insert(artist_id, cover_ids);
    }

    // Step 6: Group artists by their group and associate cover IDs
    let mut grouped_artists: HashMap<String, Vec<ArtistGroupParseResult>> = HashMap::new();
    for artist in artists {
        let cover_ids = artist_to_cover_ids
            .get(&artist.id)
            .cloned()
            .unwrap_or_default();
        grouped_artists
            .entry(artist.group.clone())
            .or_default()
            .push((artist, cover_ids));
    }

    // Step 7: Prepare the final result
    let result = groups
        .into_iter()
        .map(|group| {
            let artists_in_group = grouped_artists.remove(&group).unwrap_or_default();
            (group, artists_in_group)
        })
        .collect();

    Ok(result)
}
