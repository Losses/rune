use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Result;
use sea_orm::{DatabaseConnection, DbErr};

use crate::entities::{
    albums, artists, media_file_albums, media_file_artists, media_file_playlists, playlists,
};
use crate::get_cover_ids;

get_cover_ids!(get_album_cover_ids, albums, media_file_albums, AlbumUuid);
get_cover_ids!(
    get_artist_cover_ids,
    artists,
    media_file_artists,
    ArtistUuid
);
get_cover_ids!(
    get_playlist_cover_ids,
    playlists,
    media_file_playlists,
    PlaylistUuid
);

/// Remove bounded batches of local rows whose file was explicitly removed.
pub async fn clean_local_orphans(db: &DatabaseConnection) -> Result<()> {
    use crate::entities::{media_analysis, media_files};
    use sea_orm::{
        ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait, TransactionTrait,
    };
    let tx = db.begin().await?;
    let analysis = media_analysis::Entity::find()
        .select_only()
        .column(media_analysis::Column::Id)
        .filter(
            media_analysis::Column::FileId.not_in_subquery(
                media_files::Entity::find()
                    .select_only()
                    .column(media_files::Column::Id)
                    .into_query(),
            ),
        )
        .limit(64)
        .into_tuple::<i32>()
        .all(&tx)
        .await?;
    if !analysis.is_empty() {
        media_analysis::Entity::delete_many()
            .filter(media_analysis::Column::Id.is_in(analysis))
            .exec(&tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
