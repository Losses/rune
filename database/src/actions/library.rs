use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Result;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, ExprTrait, sea_query::Query,
};

use crate::entities::{
    audio_artists, media_cover_art, media_file_playlists, media_files,
    music_metadata::{audio, discs},
    playlists, portable_albums, portable_artists, track_album_sources,
};
use crate::get_cover_ids;

use super::resolved_metadata::column;

pub async fn get_album_cover_ids(
    db: &DatabaseConnection,
    entities: &[portable_albums::Model],
) -> Result<HashMap<i32, HashSet<i32>>, DbErr> {
    use {audio::Column as A, discs::Column as D, media_cover_art::Column as C};
    use {media_files::Column as F, track_album_sources::Column as S};

    let ids: HashMap<_, _> = entities
        .iter()
        .map(|album| (album.source_uuid.as_str(), album.id))
        .collect();
    let keys: Vec<_> = ids.keys().copied().collect();
    let mut covers = HashMap::<i32, HashSet<i32>>::new();
    for keys in keys.chunks(400) {
        // The resolved edition owns art from its current tracks.
        let mut tracks = Query::select();
        tracks
            .from(audio::Entity)
            .inner_join(discs::Entity, column(D::Id).eq(column(A::DiscId)))
            .inner_join(media_files::Entity, column(F::Id).eq(column(A::FileId)))
            .inner_join(
                media_cover_art::Entity,
                column(C::SourceUuid).eq(column(F::CoverArtUuid)),
            )
            .and_where(C::Id.ne(-1))
            .distinct();

        // Saved album IDs can cover tracks that now belong to several editions.
        let saved = tracks
            .clone()
            .columns([S::AlbumUuid.as_column_ref(), C::Id.as_column_ref()])
            .inner_join(
                track_album_sources::Entity,
                column(S::TrackUuid).eq(column(A::Id)),
            )
            .and_where(S::AlbumUuid.is_in(keys.iter().copied()))
            .to_owned();
        tracks
            .columns([D::AlbumId.as_column_ref(), C::Id.as_column_ref()])
            .and_where(D::AlbumId.is_in(keys.iter().copied()))
            .union(sea_orm::sea_query::UnionType::All, saved);
        for row in db.query_all(&tracks).await? {
            let album: String = row.try_get_by_index(0)?;
            covers
                .entry(ids[album.as_str()])
                .or_default()
                .insert(row.try_get_by_index(1)?);
        }
    }
    Ok(covers)
}

get_cover_ids!(
    get_artist_cover_ids,
    portable_artists,
    audio_artists,
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
