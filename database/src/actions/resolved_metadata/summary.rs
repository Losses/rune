//! The indexed display values derived from a release's current track evidence.

use anyhow::Result;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, ExprTrait,
    QueryFilter, QuerySelect, Set,
    sea_query::{Expr, OnConflict, Order, Query},
};
use uuid::Uuid;

use crate::entities::music_metadata::{
    album_summaries, albums, dirty_summaries, live_summaries, release_identity, release_keys,
    synced_album_groups,
};

use super::{column, scalar};

const EDITION_NAMESPACE: Uuid = Uuid::from_u128(0xbe54f834_1696_5fb8_b4f5_4df4a58d1821);

pub(super) async fn refresh_directory(db: &DatabaseTransaction, directory: &str) -> Result<()> {
    let albums = albums::Entity::find()
        .select_only()
        .column(albums::Column::Id)
        .filter(albums::Column::DirectoryScope.eq(directory))
        .into_tuple::<String>()
        .all(db)
        .await?;
    for album in albums {
        refresh(db, album.parse()?).await?;
    }
    Ok(())
}

pub(super) async fn refresh(db: &DatabaseTransaction, album: Uuid) -> Result<()> {
    let album = album.to_string();
    // The same indexed projection also serves reads while a summary is dirty.
    use dirty_summaries::Column as D;
    let existing = Query::select()
        .column(albums::Column::Id)
        .from(albums::Entity)
        .and_where(albums::Column::Id.eq(&album))
        .to_owned();
    db.execute(
        Query::insert()
            .into_table(dirty_summaries::Entity)
            .columns([D::AlbumId])
            .select_from(existing)?
            .on_conflict(OnConflict::column(D::AlbumId).do_nothing().to_owned()),
    )
    .await?;

    use album_summaries::Column as C;
    use live_summaries::Column as L;
    let current = Query::select()
        .columns([
            L::AlbumId,
            L::Title,
            L::Version,
            L::CatalogNumber,
            L::ReleaseDate,
            L::ReleaseType,
            L::TrackCount,
        ])
        .from(live_summaries::Entity)
        .and_where(L::AlbumId.eq(&album))
        .to_owned();
    let updated = db
        .execute(
            Query::insert()
                .into_table(album_summaries::Entity)
                .columns([
                    C::AlbumId,
                    C::Title,
                    C::Version,
                    C::CatalogNumber,
                    C::ReleaseDate,
                    C::ReleaseType,
                    C::TrackCount,
                ])
                .select_from(current)?
                .on_conflict(
                    OnConflict::column(C::AlbumId)
                        .update_columns([
                            C::Title,
                            C::Version,
                            C::CatalogNumber,
                            C::ReleaseDate,
                            C::ReleaseType,
                            C::TrackCount,
                        ])
                        .to_owned(),
                ),
        )
        .await?;
    if updated.rows_affected() == 0 {
        // Retain the description of a manually grouped edition while its media is absent.
        album_summaries::Entity::update_many()
            .col_expr(C::TrackCount, Expr::val(0))
            .filter(C::AlbumId.eq(&album))
            .exec(db)
            .await?;
    } else {
        refresh_identity(db, &album).await?;
    }

    // Apply edits received before this edition was indexed, including explicit removals.
    project_album_groups(db, Condition::all().add(albums::Column::Id.eq(&album))).await?;
    dirty_summaries::Entity::delete_by_id(album)
        .exec(db)
        .await?;
    Ok(())
}

pub(super) async fn project_album_groups(
    db: &DatabaseTransaction,
    affected: Condition,
) -> Result<(), DbErr> {
    use synced_album_groups::Column as G;
    let group = Query::select()
        .column(G::TargetUuid)
        .from(synced_album_groups::Entity)
        .and_where(column(G::AlbumId).eq(column(albums::Column::Id)))
        .order_by(G::Generation, Order::Desc)
        .order_by(G::OwnerUuid, Order::Asc)
        .limit(1)
        .to_owned();
    albums::Entity::update_many()
        .col_expr(albums::Column::GroupId, scalar(group))
        .filter(affected)
        .exec(db)
        .await?;
    Ok(())
}

async fn refresh_identity(db: &DatabaseTransaction, album: &str) -> Result<()> {
    // A unique release MBID yields the same edition identity even on disjoint replicas.
    use release_keys::Column as K;
    let values = release_keys::Entity::find()
        .select_only()
        .column(K::Value)
        .filter(K::AlbumId.eq(album))
        .filter(K::Field.eq("release"))
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
