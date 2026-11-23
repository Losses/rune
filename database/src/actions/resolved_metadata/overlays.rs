//! Store local metadata overlays by catalog identity.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use metadata::overlay::{MetadataOverlay, OverlayMetadata};
pub use metadata::overlay::{OverlayEdit, OverlayField};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, ExprTrait, JoinType, QueryFilter,
    Set,
    sea_query::{Alias, Expr, Func, OnConflict, Order, Query},
};
use uuid::Uuid;

use crate::entities::music_metadata::{
    album_effective_fields as effective, audio, discs, local_overlays as storage, track_assets,
};
use storage::Column as OverlayColumn;

/// The catalog entity whose fields are locally overridden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTarget {
    Album(Uuid),
    Disc(Uuid),
    Audio(Uuid),
    ReleaseTrack(Uuid),
}

impl OverlayTarget {
    fn column(self) -> (OverlayColumn, Uuid) {
        match self {
            Self::Album(id) => (OverlayColumn::AlbumId, id),
            Self::Disc(id) => (OverlayColumn::DiscId, id),
            Self::Audio(id) => (OverlayColumn::AudioId, id),
            Self::ReleaseTrack(id) => (OverlayColumn::TrackId, id),
        }
    }

    fn supports(self, field: OverlayField) -> bool {
        use OverlayField::*;
        match self {
            Self::Album(_) => field != Subtitle,
            Self::Disc(_) | Self::Audio(_) => {
                matches!(
                    field,
                    Title
                        | Version
                        | ReleaseDate
                        | OriginalReleaseDate
                        | Description
                        | Comment
                        | SortTitle
                ) || (matches!(self, Self::Audio(_)) && field == Subtitle)
            }
            Self::ReleaseTrack(_) => field == Title,
        }
    }
}

/// Change one field in the highest-priority local source.
pub async fn set_local_overlay(
    db: &impl ConnectionTrait,
    target: OverlayTarget,
    field: OverlayField,
    edit: OverlayEdit,
) -> Result<bool> {
    ensure!(
        target.supports(field),
        "field is unavailable for this metadata entity"
    );
    let (column, id) = target.column();
    let value = match edit {
        OverlayEdit::Restore => {
            let update = storage::Entity::delete_many()
                .filter(column.eq(id.to_string()))
                .filter(OverlayColumn::Field.eq(field.key()))
                .exec(db)
                .await?;
            return Ok(update.rows_affected != 0);
        }
        OverlayEdit::Set(value) => Some(field.normalize(&value)?),
        OverlayEdit::Clear => {
            ensure!(
                field != OverlayField::Title || matches!(target, OverlayTarget::Disc(_)),
                "this metadata title cannot be cleared"
            );
            None
        }
    };
    let mut row = storage::ActiveModel {
        field: Set(field.key().to_owned()),
        value: Set(value),
        ..Default::default()
    };
    row.set(column, id.to_string().into());
    let updated = storage::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([column, OverlayColumn::Field])
                .update_column(OverlayColumn::Value)
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(updated != 0)
}

/// Read only the local assertions, including explicit clears.
pub async fn local_overlay(
    db: &impl ConnectionTrait,
    target: OverlayTarget,
) -> Result<MetadataOverlay> {
    let (column, id) = target.column();
    let rows = storage::Entity::find()
        .filter(column.eq(id.to_string()))
        .all(db)
        .await?;
    let mut overlay = MetadataOverlay::default();
    for row in rows {
        let field = OverlayField::from_key(&row.field).context("unknown local metadata field")?;
        overlay.insert(field, row.value);
    }
    Ok(overlay)
}

pub(super) async fn apply(
    db: &impl ConnectionTrait,
    target: OverlayTarget,
    metadata: &mut impl OverlayMetadata,
) -> Result<()> {
    local_overlay(db, target).await?.apply_to(metadata);
    Ok(())
}

pub(in crate::actions) async fn summary_titles(
    db: &impl ConnectionTrait,
    files: &[i32],
) -> Result<HashMap<i32, (Option<String>, Option<String>)>> {
    let mut titles = HashMap::new();
    for files in files.chunks(400) {
        // Whole-file summaries may show a logical title. Disc images retain their own display title.
        let file_id = Expr::col((audio::Entity, audio::Column::FileId));
        let track_title = Query::select()
            .column((storage::Entity, OverlayColumn::Value))
            .from(track_assets::Entity)
            .join(
                JoinType::InnerJoin,
                storage::Entity,
                Expr::col(OverlayColumn::TrackId.as_column_ref())
                    .eq(Expr::col((
                        track_assets::Entity,
                        track_assets::Column::TrackId,
                    )))
                    .and(OverlayColumn::Field.eq("title")),
            )
            .and_where(Expr::col(track_assets::Column::FileId.as_column_ref()).eq(file_id.clone()))
            .and_where(track_assets::Column::StartMs.eq(0))
            .and_where(track_assets::Column::EndMs.is_null())
            .order_by(track_assets::Column::TrackId.as_column_ref(), Order::Asc)
            .limit(1)
            .to_owned();

        // Join the audio and album overrides directly, retaining the track fallback above.
        let audio_title = Alias::new("audio_title");
        let query = Query::select()
            .column((audio::Entity, audio::Column::FileId))
            .expr_as(
                Func::coalesce([
                    Expr::col((audio_title.clone(), OverlayColumn::Value)),
                    Expr::SubQuery(None, Box::new(track_title.into())),
                ]),
                Alias::new("title"),
            )
            .expr_as(
                Expr::col((effective::Entity, effective::Column::Value)),
                Alias::new("album_title"),
            )
            .from(audio::Entity)
            .join(
                JoinType::InnerJoin,
                discs::Entity,
                Expr::col(discs::Column::Id.as_column_ref())
                    .eq(Expr::col((audio::Entity, audio::Column::DiscId))),
            )
            .join_as(
                JoinType::LeftJoin,
                storage::Entity,
                audio_title.clone(),
                Expr::col((audio_title.clone(), OverlayColumn::AudioId))
                    .eq(Expr::col((audio::Entity, audio::Column::Id)))
                    .and(Expr::col((audio_title, OverlayColumn::Field)).eq("title")),
            )
            .join(
                JoinType::LeftJoin,
                effective::Entity,
                Expr::col(effective::Column::AlbumId.as_column_ref())
                    .eq(Expr::col((discs::Entity, discs::Column::AlbumId)))
                    .and(effective::Column::Field.eq("title")),
            )
            .and_where(file_id.is_in(files.iter().copied()))
            .to_owned();
        let rows = db.query_all(&query).await?;
        for row in rows {
            titles.insert(
                row.try_get("", "file_id")?,
                (row.try_get("", "title")?, row.try_get("", "album_title")?),
            );
        }
    }
    Ok(titles)
}
