//! Maintain catalog work queues in the same transaction as evidence changes.

use anyhow::Result;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, ExprTrait, QueryFilter, QuerySelect,
    sea_query::{Expr, OnConflict, Query, SelectStatement},
};

use crate::entities::music_metadata::{
    albums, audio, audio_layouts, browse_handles, dirty_tracks, discs,
};

use super::column;
pub(crate) use crate::actions::browse::Kind as BrowseKind;

pub(crate) async fn allocate_handle(
    db: &impl ConnectionTrait,
    kind: BrowseKind,
    id: &str,
) -> Result<(), DbErr> {
    use browse_handles::Column as H;
    let kind = kind.label();
    let existing = Query::select()
        .column(H::Id)
        .from(browse_handles::Entity)
        .and_where(H::Kind.eq(kind))
        .and_where(H::SourceUuid.eq(id))
        .to_owned();
    let missing = Query::select()
        .exprs([Expr::val(kind), Expr::val(id)])
        .and_where(Expr::not_exists(existing))
        .to_owned();
    // Avoid consuming a new AUTOINCREMENT value for an already allocated handle.
    let mut insert = Query::insert();
    insert
        .into_table(browse_handles::Entity)
        .columns([H::Kind, H::SourceUuid])
        .select_from(missing)
        .map_err(|error| DbErr::Custom(error.to_string()))?
        .on_conflict(
            OnConflict::columns([H::Kind, H::SourceUuid])
                .do_nothing()
                .to_owned(),
        );
    db.execute(&insert).await?;
    Ok(())
}

pub(super) async fn queue_discs(db: &impl ConnectionTrait, discs: SelectStatement) -> Result<()> {
    db.execute(
        Query::insert()
            .into_table(dirty_tracks::Entity)
            .columns([dirty_tracks::Column::DiscId])
            .select_from(discs)?
            .on_conflict(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    Ok(())
}

pub(super) async fn queue_audio_move(
    db: &impl ConnectionTrait,
    file: i32,
    disc: &str,
) -> Result<()> {
    let previous = Query::select()
        .column(audio::Column::DiscId)
        .from(audio::Entity)
        .and_where(audio::Column::FileId.eq(file))
        .to_owned();
    let discs = Query::select()
        .column(discs::Column::Id)
        .from(discs::Entity)
        .and_where(
            discs::Column::Id
                .eq(disc)
                .or(column(discs::Column::Id).in_subquery(previous)),
        )
        .to_owned();
    queue_discs(db, discs).await
}

pub(super) async fn audio_disc(
    db: &impl ConnectionTrait,
    track: &str,
) -> Result<Option<String>, DbErr> {
    audio::Entity::find()
        .select_only()
        .column(audio::Column::DiscId)
        .filter(audio::Column::Id.eq(track))
        .into_tuple::<String>()
        .one(db)
        .await
}

pub(super) async fn finish_portable_projection(
    db: &impl ConnectionTrait,
    file: i32,
    previous: Option<String>,
) -> Result<()> {
    use audio::Column as A;
    use discs::Column as D;
    let row = db
        .query_one(
            Query::select()
                .column(D::AlbumId.as_column_ref())
                .expr_as(column(D::Id), "disc_id")
                .from(audio::Entity)
                .inner_join(discs::Entity, column(D::Id).eq(column(A::DiscId)))
                .and_where(A::FileId.eq(file)),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing projected audio {file}"))?;

    // A remote retag can move the file to a different disc. Reconcile both sides.
    let album: String = row.try_get("", "album_id")?;
    let current: String = row.try_get("", "disc_id")?;
    audio_layouts::Entity::delete_by_id(file).exec(db).await?;
    allocate_handle(db, BrowseKind::Album, &album).await?;
    if let Some(old) = previous.as_deref().filter(|old| *old != current) {
        // A partial sync can finish before the next browse. Retire empty parents now.
        finish_disc(db, old).await?;
    }
    let discs = Query::select()
        .column(D::Id)
        .from(discs::Entity)
        .and_where(D::Id.is_in(previous.into_iter().chain([current])))
        .to_owned();
    queue_discs(db, discs).await
}

pub(super) async fn finish_disc(db: &impl ConnectionTrait, disc: &str) -> Result<()> {
    use albums::Column as A;
    use audio::Column as T;
    use discs::Column as D;
    let row = db
        .query_one(
            Query::select()
                .column(D::AlbumId.as_column_ref())
                .column(A::DirectoryScope.as_column_ref())
                .from(discs::Entity)
                .inner_join(albums::Entity, column(A::Id).eq(column(D::AlbumId)))
                .and_where(D::Id.eq(disc)),
        )
        .await?;
    let Some(row) = row else { return Ok(()) };
    let album: String = row.try_get("", "album_id")?;
    let scope: String = row.try_get("", "directory_scope")?;
    super::release_group::queue(db, &scope).await?;

    // Remove empty parents after assets and layouts have been moved or removed.
    let media = Query::select()
        .column(T::FileId)
        .from(audio::Entity)
        .and_where(T::DiscId.eq(disc))
        .to_owned();
    let removed = discs::Entity::delete_by_id(disc.to_owned())
        .filter(Expr::not_exists(media))
        .exec(db)
        .await?;
    if removed.rows_affected != 0 {
        let remaining = Query::select()
            .column(D::Id)
            .from(discs::Entity)
            .and_where(D::AlbumId.eq(&album))
            .to_owned();
        albums::Entity::delete_by_id(album.clone())
            .filter(Expr::not_exists(remaining))
            .exec(db)
            .await?;
    }

    // An album might have disappeared in the previous delete. Queue only live parents.
    let existing = Query::select()
        .column(A::Id)
        .from(albums::Entity)
        .and_where(A::Id.eq(album))
        .to_owned();
    use crate::entities::music_metadata::dirty_summaries as dirty;
    db.execute(
        Query::insert()
            .into_table(dirty::Entity)
            .columns([dirty::Column::AlbumId])
            .select_from(existing)?
            .on_conflict(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    Ok(())
}
