//! Edition-level corrections and reviewed values from external sources.

use anyhow::{Context, Result, ensure};
use metadata::{
    model::{AlbumMetadata, FieldSource, MetadataSourceKind},
    overlay::{OverlayField, OverlayMetadata},
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, ExprTrait, QueryFilter, QueryOrder, Set,
    TransactionSession, TransactionTrait,
    sea_query::{Alias, Expr, OnConflict},
};
use uuid::Uuid;

use crate::entities::music_metadata::{
    album_effective_fields as effective, album_field_values as proposals,
};
use proposals::Column as FieldColumn;

use super::overlays::{self, OverlayTarget};
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
    // A provider can refresh the same proposal without resetting an unchanged decision.
    let decision = Expr::case(
        Expr::col(FieldColumn::Value).eq(Expr::col((Alias::new("excluded"), FieldColumn::Value))),
        Expr::col(FieldColumn::Decision),
    )
    .finally(CandidateDecision::Pending as i32);
    let conflict = OnConflict::columns([
        FieldColumn::AlbumId,
        FieldColumn::Field,
        FieldColumn::Provider,
        FieldColumn::Resource,
    ])
    .update_column(FieldColumn::Value)
    .value(FieldColumn::Decision, decision)
    .to_owned();

    proposals::Entity::insert(proposals::ActiveModel {
        album_id: Set(album.to_string()),
        field: Set(field.key().to_owned()),
        provider: Set(provider.to_owned()),
        resource: Set(resource.to_owned()),
        value: Set(Some(value)),
        decision: Set(CandidateDecision::Pending as i32),
    })
    .on_conflict(conflict)
    .exec_without_returning(db)
    .await?;
    Ok(())
}

/// Read the proposals for a field, including pending and rejected values.
pub async fn album_field_candidates(
    db: &impl ConnectionTrait,
    album: Uuid,
    field: AlbumField,
) -> Result<Vec<AlbumFieldCandidate>> {
    let rows = proposals::Entity::find()
        .filter(FieldColumn::AlbumId.eq(album.to_string()))
        .filter(FieldColumn::Field.eq(field.key()))
        .filter(FieldColumn::Provider.ne(""))
        .order_by_asc(FieldColumn::Provider)
        .order_by_asc(FieldColumn::Resource)
        .all(db)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(AlbumFieldCandidate {
                provider: row.provider,
                resource: row.resource,
                value: row.value.context("album field proposal has no value")?,
                decision: match row.decision {
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
        proposals::Entity::update_many()
            .col_expr(
                FieldColumn::Decision,
                Expr::val(CandidateDecision::Pending as i32),
            )
            .filter(FieldColumn::AlbumId.eq(&album))
            .filter(FieldColumn::Field.eq(field.key()))
            .filter(FieldColumn::Provider.ne(""))
            .filter(FieldColumn::Decision.eq(CandidateDecision::Accepted as i32))
            .filter(
                Condition::any()
                    .add(FieldColumn::Provider.ne(provider))
                    .add(FieldColumn::Resource.ne(resource)),
            )
            .exec(&tx)
            .await?;
    }
    let updated = proposals::Entity::update_many()
        .col_expr(FieldColumn::Decision, Expr::val(decision as i32))
        .filter(FieldColumn::AlbumId.eq(album))
        .filter(FieldColumn::Field.eq(field.key()))
        .filter(FieldColumn::Provider.eq(provider))
        .filter(FieldColumn::Resource.eq(resource))
        .exec(&tx)
        .await?
        .rows_affected
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
    Ok(proposals::Entity::delete_many()
        .filter(FieldColumn::AlbumId.eq(album.to_string()))
        .filter(FieldColumn::Provider.eq(provider))
        .exec(db)
        .await?
        .rows_affected)
}

pub(super) async fn apply(db: &impl ConnectionTrait, album: &mut AlbumMetadata) -> Result<()> {
    let rows = effective::Entity::find()
        .filter(effective::Column::AlbumId.eq(album.base.id.to_string()))
        .filter(effective::Column::Provider.ne(""))
        .all(db)
        .await?;
    for row in rows {
        let field =
            OverlayField::from_key(&row.field).context("unknown external metadata field")?;
        let source = FieldSource {
            kind: MetadataSourceKind::External,
            key: Some(row.provider),
            path: Some(row.resource),
            pattern: None,
            entity_id: None,
            confidence: None,
        };
        album.apply_field(field, row.value, source);
    }
    Ok(())
}
