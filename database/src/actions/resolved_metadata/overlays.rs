//! Store local metadata overlays by catalog identity.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use metadata::overlay::{MetadataOverlay, OverlayMetadata};
pub use metadata::overlay::{OverlayEdit, OverlayField};
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use super::statement;

/// The catalog entity whose fields are locally overridden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTarget {
    Album(Uuid),
    Disc(Uuid),
    Audio(Uuid),
    ReleaseTrack(Uuid),
}

impl OverlayTarget {
    fn column(self) -> (&'static str, Uuid) {
        match self {
            Self::Album(id) => ("album_id", id),
            Self::Disc(id) => ("disc_id", id),
            Self::Audio(id) => ("audio_id", id),
            Self::ReleaseTrack(id) => ("track_id", id),
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
            let update = db
                .execute_raw(statement(
                    format!(
                        "DELETE FROM music_metadata_local_overlays WHERE {column} = ? AND field = ?"
                    ),
                    [id.to_string().into(), field.key().into()],
                ))
                .await?;
            return Ok(update.rows_affected() != 0);
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
    let update = db.execute_raw(statement(
        format!("INSERT INTO music_metadata_local_overlays ({column}, field, value) VALUES (?, ?, ?)
            ON CONFLICT({column}, field) DO UPDATE SET value = excluded.value"),
        [id.to_string().into(), field.key().into(), value.into()],
    )).await?;
    Ok(update.rows_affected() != 0)
}

/// Read only the local assertions, including explicit clears.
pub async fn local_overlay(
    db: &impl ConnectionTrait,
    target: OverlayTarget,
) -> Result<MetadataOverlay> {
    let (column, id) = target.column();
    let rows = db
        .query_all_raw(statement(
            format!("SELECT field, value FROM music_metadata_local_overlays WHERE {column} = ?"),
            [id.to_string().into()],
        ))
        .await?;
    let mut overlay = MetadataOverlay::default();
    for row in rows {
        let key: String = row.try_get("", "field")?;
        let field = OverlayField::from_key(&key).context("unknown local metadata field")?;
        overlay.insert(field, row.try_get("", "value")?);
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
        let ids = vec!["?"; files.len()].join(", ");
        // Whole-file summaries may show a logical title. Disc images retain their own display title.
        let rows = db.query_all_raw(statement(format!(
            "SELECT a.file_id, COALESCE(audio.value, (
                SELECT song.value FROM music_metadata_track_assets asset
                JOIN music_metadata_local_overlays song ON song.track_id = asset.track_id AND song.field = 'title'
                WHERE asset.file_id = a.file_id AND asset.start_ms = 0 AND asset.end_ms IS NULL
                ORDER BY asset.track_id LIMIT 1)) AS title, album.value AS album_title
             FROM music_metadata_audio a JOIN music_metadata_discs d ON d.id = a.disc_id
             LEFT JOIN music_metadata_local_overlays audio ON audio.audio_id = a.id AND audio.field = 'title'
             LEFT JOIN music_metadata_album_effective_fields album ON album.album_id = d.album_id AND album.field = 'title'
             WHERE a.file_id IN ({ids})"), files.iter().copied().map(Into::into))).await?;
        for row in rows {
            titles.insert(
                row.try_get("", "file_id")?,
                (row.try_get("", "title")?, row.try_get("", "album_title")?),
            );
        }
    }
    Ok(titles)
}
