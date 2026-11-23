//! Edition-level corrections and reviewed values from external sources.

use anyhow::{Context, Result, ensure};
use metadata::{
    model::{AlbumMetadata, FieldSource, MetadataSourceKind},
    overlay::{OverlayField, OverlayMetadata},
};
use sea_orm::{ConnectionTrait, TransactionSession, TransactionTrait};
use uuid::Uuid;

use super::{
    overlays::{self, OverlayTarget},
    statement,
};
pub use metadata::overlay::OverlayEdit as AlbumOverride;

/// Scalar edition fields that can be edited independently of the audio tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumField {
    Title,
    Version,
    CatalogNumber,
    ReleaseDate,
    OriginalReleaseDate,
    Label,
    ReleaseType,
    Barcode,
    ReleaseCountry,
    Description,
}

impl AlbumField {
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Version => "version",
            Self::CatalogNumber => "catalog_number",
            Self::ReleaseDate => "release_date",
            Self::OriginalReleaseDate => "original_release_date",
            Self::Label => "labels",
            Self::ReleaseType => "release_type",
            Self::Barcode => "barcode",
            Self::ReleaseCountry => "release_country",
            Self::Description => "description",
        }
    }
}

/// An external value only becomes effective after explicit acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CandidateDecision {
    Pending = 0,
    Accepted = 1,
    Rejected = 2,
}

/// One proposed value and its review state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumFieldCandidate {
    pub provider: String,
    pub resource: String,
    pub value: String,
    pub decision: CandidateDecision,
}

/// Set or remove a manual correction through the model's local overlay source.
pub async fn set_album_override(
    db: &impl ConnectionTrait,
    album: Uuid,
    field: AlbumField,
    edit: AlbumOverride,
) -> Result<bool> {
    let field = OverlayField::from_key(field.key()).context("unknown album field")?;
    overlays::set_local_overlay(db, OverlayTarget::Album(album), field, edit).await
}

/// Submit a provider contribution. An unchanged refresh retains its review state.
pub async fn propose_album_field(
    db: &impl ConnectionTrait,
    album: Uuid,
    field: AlbumField,
    provider: &str,
    resource: &str,
    value: &str,
) -> Result<()> {
    let provider = provider.trim();
    let resource = resource.trim();
    ensure!(
        !provider.is_empty() && !resource.is_empty(),
        "candidate needs a provider and resource"
    );
    let value = OverlayField::from_key(field.key())
        .context("unknown album field")?
        .normalize(value)?;
    db.execute_raw(statement(
        "INSERT INTO music_metadata_album_field_values (album_id, field, provider, resource, value, decision)
         VALUES (?, ?, ?, ?, ?, 0) ON CONFLICT(album_id, field, provider, resource) DO UPDATE SET
             value = excluded.value, decision = CASE WHEN value = excluded.value THEN decision ELSE 0 END",
        [album.to_string().into(), field.key().into(), provider.into(), resource.into(), value.into()],
    )).await?;
    Ok(())
}

/// Read the proposals for a field, including pending and rejected values.
pub async fn album_field_candidates(
    db: &impl ConnectionTrait,
    album: Uuid,
    field: AlbumField,
) -> Result<Vec<AlbumFieldCandidate>> {
    let rows = db
        .query_all_raw(statement(
            "SELECT provider, resource, value, decision FROM music_metadata_album_field_values
         WHERE album_id = ? AND field = ? AND provider <> '' ORDER BY provider, resource",
            [album.to_string().into(), field.key().into()],
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok(AlbumFieldCandidate {
                provider: row.try_get("", "provider")?,
                resource: row.try_get("", "resource")?,
                value: row.try_get("", "value")?,
                decision: match row.try_get::<i32>("", "decision")? {
                    0 => CandidateDecision::Pending,
                    1 => CandidateDecision::Accepted,
                    2 => CandidateDecision::Rejected,
                    _ => anyhow::bail!("invalid album candidate decision"),
                },
            })
        })
        .collect()
}

/// Change a candidate's decision. Accepting a value supersedes the prior accepted proposal.
pub async fn decide_album_field(
    db: &impl TransactionTrait,
    album: Uuid,
    field: AlbumField,
    provider: &str,
    resource: &str,
    decision: CandidateDecision,
) -> Result<bool> {
    let tx = db.begin().await?;
    let album = album.to_string();
    let provider = provider.trim();
    let resource = resource.trim();
    ensure!(
        !provider.is_empty() && !resource.is_empty(),
        "missing album field candidate identity"
    );

    // The savepoint keeps the old selection if a caller references a missing proposal.
    if decision == CandidateDecision::Accepted {
        tx.execute_raw(statement(
            "UPDATE music_metadata_album_field_values SET decision = 0
             WHERE album_id = ?1 AND field = ?2 AND provider <> '' AND decision = 1
               AND (provider <> ?3 OR resource <> ?4)",
            [
                album.clone().into(),
                field.key().into(),
                provider.into(),
                resource.into(),
            ],
        ))
        .await?;
    }
    let updated = tx
        .execute_raw(statement(
            "UPDATE music_metadata_album_field_values SET decision = ?
         WHERE album_id = ? AND field = ? AND provider = ? AND resource = ?",
            [
                (decision as i32).into(),
                album.into(),
                field.key().into(),
                provider.into(),
                resource.into(),
            ],
        ))
        .await?
        .rows_affected()
        != 0;
    if updated {
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    Ok(updated)
}

/// Withdraw all fields from one source for an edition without touching manual overrides.
pub async fn withdraw_album_provider(
    db: &impl ConnectionTrait,
    album: Uuid,
    provider: &str,
) -> Result<u64> {
    let provider = provider.trim();
    ensure!(!provider.is_empty(), "missing album provider");
    Ok(db
        .execute_raw(statement(
            "DELETE FROM music_metadata_album_field_values WHERE album_id = ? AND provider = ?",
            [album.to_string().into(), provider.into()],
        ))
        .await?
        .rows_affected())
}

pub(super) async fn apply(db: &impl ConnectionTrait, album: &mut AlbumMetadata) -> Result<()> {
    let rows = db
        .query_all_raw(statement(
            "SELECT field, provider, resource, value FROM music_metadata_album_effective_fields
         WHERE album_id = ? AND provider <> ''",
            [album.base.id.to_string().into()],
        ))
        .await?;
    for row in rows {
        let key: String = row.try_get("", "field")?;
        let field = OverlayField::from_key(&key).context("unknown external metadata field")?;
        let source = FieldSource {
            kind: MetadataSourceKind::External,
            key: Some(row.try_get("", "provider")?),
            path: Some(row.try_get("", "resource")?),
            pattern: None,
            entity_id: None,
            confidence: None,
        };
        album.apply_field(field, row.try_get("", "value")?, source);
    }
    Ok(())
}
