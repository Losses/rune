use log::info;
use sea_orm::{prelude::*, ActiveValue};
use sea_orm::{DatabaseConnection, Set, TransactionTrait};

use crate::entities::{albums, artists, media_file_albums, media_file_artists};

use super::metadata::get_metadata_summary_by_file_ids;

pub async fn index_media_files(
    db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<(), sea_orm::DbErr> {
    info!("Indexing media: {:?}", file_ids);
    // Fetch metadata summary for provided file_ids
    let metadata_summaries = get_metadata_summary_by_file_ids(db, file_ids.clone()).await?;

    let txn = db.begin().await?;

    for summary in metadata_summaries {
        // Process artists
        let artists = metadata::artist::split_artists(&summary.artist);
        let mut artist_ids = Vec::new();

        for artist_name in artists {
            let artist = artists::ActiveModel {
                name: Set(artist_name.clone()),
                ..Default::default()
            };

            let existing_artist = artists::Entity::find()
                .filter(artists::Column::Name.eq(artist_name.clone()))
                .one(&txn)
                .await?;

            let artist_id = if let Some(existing) = existing_artist {
                existing.id
            } else {
                let inserted_artist = artists::Entity::insert(artist).exec(&txn).await?;
                inserted_artist.last_insert_id
            };

            artist_ids.push(artist_id);
        }

        // Clean up old artist relationships
        media_file_artists::Entity::delete_many()
            .filter(media_file_artists::Column::MediaFileId.eq(summary.id))
            .exec(&txn)
            .await?;

        // Insert new artist relationships
        for artist_id in artist_ids {
            let media_file_artist = media_file_artists::ActiveModel {
                id: ActiveValue::NotSet,
                media_file_id: Set(summary.id),
                artist_id: Set(artist_id),
            };
            media_file_artists::Entity::insert(media_file_artist)
                .exec(&txn)
                .await?;
        }

        // Process album
        let album_name = summary.album;
        let album = albums::ActiveModel {
            name: Set(album_name.clone()),
            ..Default::default()
        };

        let existing_album = albums::Entity::find()
            .filter(albums::Column::Name.eq(album_name.clone()))
            .one(&txn)
            .await?;

        let album_id = if let Some(existing) = existing_album {
            existing.id
        } else {
            let inserted_album = albums::Entity::insert(album).exec(&txn).await?;
            inserted_album.last_insert_id
        };

        // Clean up old album relationships
        media_file_albums::Entity::delete_many()
            .filter(media_file_albums::Column::MediaFileId.eq(summary.id))
            .exec(&txn)
            .await?;

        // Insert new album relationship
        let media_file_album = media_file_albums::ActiveModel {
            id: ActiveValue::NotSet,
            media_file_id: Set(summary.id),
            album_id: Set(album_id),
        };
        media_file_albums::Entity::insert(media_file_album)
            .exec(&txn)
            .await?;
    }

    txn.commit().await?;
    Ok(())
}
