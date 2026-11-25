//! Maintain album display summaries from the indexed file evidence.

use std::collections::HashMap;

use anyhow::Result;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, ExprTrait, FromQueryResult,
    QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::{Expr, Func, OnConflict, Order, Query, TableRef, UnionType},
};
use uuid::Uuid;

use crate::entities::music_metadata::{
    album_evidence, album_summaries, albums, audio, dirty_summaries, discs, release_identity,
    release_tracks, track_release_keys,
};

use crate::entities::portable_albums;

use super::{column, queries::subquery, query_models};

const EDITION_NAMESPACE: Uuid = Uuid::from_u128(0xbe54f834_1696_5fb8_b4f5_4df4a58d1821);

pub(super) async fn reconcile(db: &impl ConnectionTrait) -> Result<()> {
    for dirty in dirty_summaries::Entity::find().all(db).await? {
        refresh(db, &dirty.album_id).await?;
    }
    Ok(())
}

pub(super) async fn refresh_directory(db: &DatabaseTransaction, directory: &str) -> Result<()> {
    let albums = albums::Entity::find()
        .select_only()
        .column(albums::Column::Id)
        .filter(albums::Column::DirectoryScope.eq(directory))
        .into_tuple::<String>()
        .all(db)
        .await?;
    for album in albums {
        refresh(db, &album).await?;
    }
    Ok(())
}

#[derive(FromQueryResult)]
struct Evidence {
    title: String,
    title_priority: i32,
    version: Option<String>,
    version_priority: i32,
    catalog_number: Option<String>,
    release_date: Option<String>,
    release_type: Option<String>,
}

async fn refresh(db: &impl ConnectionTrait, album: &str) -> Result<()> {
    use album_evidence::Column as E;
    use album_summaries::Column as S;
    use audio::Column as A;
    use discs::Column as D;
    use release_tracks::Column as T;

    // Stable release order breaks ties between equally strong sources.
    let evidence = Query::select()
        .columns(
            [
                E::Title,
                E::TitlePriority,
                E::Version,
                E::VersionPriority,
                E::CatalogNumber,
                E::ReleaseDate,
                E::ReleaseType,
            ]
            .map(|c| c.as_column_ref()),
        )
        .from(discs::Entity)
        .inner_join(audio::Entity, column(A::DiscId).eq(column(D::Id)))
        .inner_join(
            album_evidence::Entity,
            column(E::FileId).eq(column(A::FileId)),
        )
        .and_where(D::AlbumId.eq(album))
        .order_by(D::DiscNumber.as_column_ref(), Order::Asc)
        .order_by_expr(A::TrackNumber.is_null(), Order::Asc)
        .order_by(A::TrackNumber.as_column_ref(), Order::Asc)
        .order_by(A::FileId.as_column_ref(), Order::Asc)
        .to_owned();
    let evidence = query_models::<Evidence>(db, &evidence).await?;

    // Select each field independently. An absent version cannot win on priority.
    if let Some(title) = evidence.iter().min_by_key(|row| row.title_priority) {
        let version = evidence
            .iter()
            .filter(|row| row.version.is_some())
            .min_by_key(|row| row.version_priority)
            .and_then(|row| row.version.clone());
        let count = Query::select()
            .expr_as(Func::count(Expr::val(1)), "track_count")
            .from(discs::Entity)
            .inner_join(release_tracks::Entity, column(T::DiscId).eq(column(D::Id)))
            .and_where(D::AlbumId.eq(album))
            .to_owned();
        let track_count = db
            .query_one(&count)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing count for album {album}"))?
            .try_get("", "track_count")?;

        // Count logical positions once, even when a track has several encodings.
        album_summaries::Entity::insert(album_summaries::ActiveModel {
            album_id: Set(album.to_owned()),
            title: Set(title.title.clone()),
            version: Set(version),
            catalog_number: Set(evidence.iter().find_map(|row| row.catalog_number.clone())),
            release_date: Set(evidence.iter().find_map(|row| row.release_date.clone())),
            release_type: Set(evidence.iter().find_map(|row| row.release_type.clone())),
            track_count: Set(track_count),
        })
        .on_conflict(
            OnConflict::column(S::AlbumId)
                .update_columns([
                    S::Title,
                    S::Version,
                    S::CatalogNumber,
                    S::ReleaseDate,
                    S::ReleaseType,
                    S::TrackCount,
                ])
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    } else {
        album_summaries::Entity::delete_by_id(album.to_owned())
            .exec(db)
            .await?;
    }

    // Refresh the stable edition key and clear the work marker in the same transaction.
    refresh_identity(db, album).await?;
    dirty_summaries::Entity::delete_by_id(album.to_owned())
        .exec(db)
        .await?;
    Ok(())
}

async fn refresh_identity(db: &impl ConnectionTrait, album: &str) -> Result<()> {
    use track_release_keys::Column as K;
    let values = track_release_keys::Entity::find()
        .select_only()
        .column(K::Value)
        .filter(K::AlbumId.eq(album))
        .filter(K::Field.eq("release"))
        .order_by_asc(K::Value)
        .distinct()
        .limit(2)
        .into_tuple::<String>()
        .all(db)
        .await?;
    if let [mbid] = values.as_slice() {
        let mbid: Uuid = mbid.parse()?;
        let edition = Uuid::new_v5(&EDITION_NAMESPACE, mbid.as_bytes());
        release_identity::Entity::insert(release_identity::ActiveModel {
            album_id: Set(album.to_owned()),
            musicbrainz_id: Set(mbid.to_string()),
            edition_uuid: Set(edition.to_string()),
        })
        .on_conflict(
            OnConflict::column(release_identity::Column::AlbumId)
                .update_columns([
                    release_identity::Column::MusicbrainzId,
                    release_identity::Column::EditionUuid,
                ])
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    } else {
        release_identity::Entity::delete_by_id(album.to_owned())
            .exec(db)
            .await?;
    }
    Ok(())
}

pub(in crate::actions) const TITLES: &str = "album_titles";

pub(in crate::actions) fn titles() -> TableRef {
    use album_summaries::Column as S;
    use albums::Column as A;
    use portable_albums::Column as P;
    let mut rows = Query::select()
        .column(A::Id.as_column_ref())
        .expr_as(column(P::Id), "legacy_id")
        .expr_as(column(P::Name), "portable_title")
        .expr_as(
            Func::coalesce([column(P::Name), column(S::Title), column(A::DirectoryPath)]),
            "title",
        )
        .expr_as(
            Func::coalesce([column(S::TrackCount), Expr::val(0)]),
            "track_count",
        )
        .from(albums::Entity)
        .left_join(
            portable_albums::Entity,
            column(P::SourceUuid).eq(column(A::Id)),
        )
        .left_join(
            album_summaries::Entity,
            column(S::AlbumId).eq(column(A::Id)),
        )
        .to_owned();

    // A synchronized album remains browsable even before any track has arrived.
    let edition = Query::select()
        .column(A::Id)
        .from(albums::Entity)
        .and_where(column(A::Id).eq(column(P::SourceUuid)))
        .to_owned();
    let empty = Query::select()
        .columns([P::SourceUuid, P::Id, P::Name, P::Name].map(|c| c.as_column_ref()))
        .expr(Expr::val(0))
        .from(portable_albums::Entity)
        .and_where(Expr::not_exists(edition))
        .to_owned();
    rows.union(UnionType::All, empty);
    subquery(rows, TITLES)
}

pub(in crate::actions) async fn track_titles(
    db: &impl ConnectionTrait,
    files: &[i32],
) -> Result<HashMap<i32, (String, Option<String>)>> {
    use album_evidence::Column as E;
    use album_summaries::Column as S;
    use audio::Column as A;
    use discs::Column as D;
    use portable_albums::Column as P;
    let mut found = HashMap::new();
    for files in files.chunks(400) {
        let rows = Query::select()
            .columns([A::FileId, A::Title].map(|c| c.as_column_ref()))
            .expr_as(
                Func::coalesce([column(P::Name), column(S::Title), column(E::Title)]),
                "album_title",
            )
            .from(audio::Entity)
            .inner_join(discs::Entity, column(D::Id).eq(column(A::DiscId)))
            .left_join(
                portable_albums::Entity,
                column(P::SourceUuid).eq(column(D::AlbumId)),
            )
            .left_join(
                album_summaries::Entity,
                column(S::AlbumId).eq(column(D::AlbumId)),
            )
            .left_join(
                album_evidence::Entity,
                column(E::FileId).eq(column(A::FileId)),
            )
            .and_where(A::FileId.is_in(files.iter().copied()))
            .to_owned();
        for row in db.query_all(&rows).await? {
            found.insert(
                row.try_get("", "file_id")?,
                (row.try_get("", "title")?, row.try_get("", "album_title")?),
            );
        }
    }
    Ok(found)
}
