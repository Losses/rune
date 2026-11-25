//! Recordings shared by files and provider resources.

use crate::actions::resolved_metadata::queries::{
    RECORDING_ALIASES, TRACK_RECORDINGS, field, recording_aliases, track_recordings, verified_audio,
};
use crate::entities::music_metadata::recording_credits as orm_recording_credits;
use crate::entities::music_metadata::recording_sources as orm_recording_sources;
use crate::entities::music_metadata::recordings as orm_recordings;
use anyhow::Result;
use metadata::model::{
    AudioQuality, AudioVariantSource, Credit, MetadataBase, MetadataSourceKind,
    RecordingIdentifiers, RecordingMetadata, RecordingSource,
};
use sea_orm::ExprTrait;
use sea_orm::sea_query::{Alias, Expr, Func, OnConflict, Order, Query, SelectStatement, UnionType};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, TransactionSession, TransactionTrait,
};
use uuid::Uuid;

use crate::entities::{
    media_files,
    music_metadata::{audio as verified, track_assets},
};

use super::{
    column,
    details::Selection,
    recording_identity::{isrc, prepare_source, reconcile, remote_fallback},
    tracks,
};

/// Register a provider resource after obtaining its recording identifiers.
pub async fn register_recording_source(
    db: &DatabaseTransaction,
    source: &RecordingSource,
    identifiers: &RecordingIdentifiers,
) -> Result<Uuid> {
    let provider = source.provider.trim();
    let resource = source.resource_id.trim();
    anyhow::ensure!(
        !provider.is_empty() && !resource.is_empty(),
        "recording source needs a provider and resource ID"
    );
    let tx = db.begin().await?;
    let identifiers = RecordingIdentifiers {
        musicbrainz_id: identifiers.musicbrainz_id.filter(|id| !id.is_nil()),
        isrc: identifiers.isrc.as_deref().and_then(isrc),
    };

    // Read the previous assertion before replacing its identity evidence.
    use orm_recording_sources::Column as S;
    let before = tx
        .query_one(
            Query::select()
                .columns([S::RecordingId, S::RecordingMbid, S::RecordingIsrc])
                .from(orm_recording_sources::Entity)
                .and_where(S::Provider.eq(provider))
                .and_where(S::ResourceId.eq(resource)),
        )
        .await?;
    let fallback = remote_fallback(provider, resource);
    let recording = prepare_source(&tx, before.as_ref(), fallback, &identifiers, false).await?;

    // A later provider refresh can remove either identifier.
    tx.execute(
        Query::insert()
            .into_table(orm_recording_sources::Entity)
            .columns([
                S::Provider,
                S::ResourceId,
                S::RecordingId,
                S::FallbackId,
                S::RecordingMbid,
                S::RecordingIsrc,
            ])
            .values([
                Expr::val(provider),
                Expr::val(resource),
                Expr::val(recording.to_string()),
                Expr::val(fallback.to_string()),
                Expr::val(identifiers.musicbrainz_id.map(|id| id.to_string())),
                Expr::val(identifiers.isrc),
            ])?
            .on_conflict(
                OnConflict::columns([S::Provider, S::ResourceId])
                    .update_columns([S::RecordingMbid, S::RecordingIsrc, S::RecordingId])
                    .to_owned(),
            ),
    )
    .await?;
    reconcile(&tx).await?;
    let row = tx
        .query_one(
            Query::select()
                .column(orm_recording_sources::Column::RecordingId)
                .from(orm_recording_sources::Entity)
                .and_where(
                    Expr::col(orm_recording_sources::Column::Provider)
                        .eq(Expr::val(provider.to_owned())),
                )
                .and_where(
                    Expr::col(orm_recording_sources::Column::ResourceId)
                        .eq(Expr::val(resource.to_owned())),
                ),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing recording source after registration"))?;
    let id = super::uuid(&row, "recording_id")?;
    tx.commit().await?;
    Ok(id)
}

/// Set provider availability and quality independently of its identifier assertions.
pub async fn update_recording_source_quality(
    db: &impl ConnectionTrait,
    source: &RecordingSource,
    available: bool,
    quality: &AudioQuality,
) -> Result<bool> {
    let result = db
        .execute(
            Query::update()
                .table(orm_recording_sources::Entity)
                .value(
                    orm_recording_sources::Column::Available,
                    Expr::val(available),
                )
                .value(
                    orm_recording_sources::Column::Codec,
                    Expr::val(quality.codec.clone()),
                )
                .value(
                    orm_recording_sources::Column::IsLossless,
                    Expr::val(quality.is_lossless),
                )
                .value(
                    orm_recording_sources::Column::SampleRate,
                    Expr::val(quality.sample_rate.map(i64::from)),
                )
                .value(
                    orm_recording_sources::Column::BitsPerSample,
                    Expr::val(quality.bits_per_sample.map(i64::from)),
                )
                .value(
                    orm_recording_sources::Column::Bitrate,
                    Expr::val(quality.bitrate.map(i64::try_from).transpose()?),
                )
                .and_where(
                    Expr::col(orm_recording_sources::Column::Provider)
                        .eq(Expr::val(source.provider.trim().to_owned())),
                )
                .and_where(
                    Expr::col(orm_recording_sources::Column::ResourceId)
                        .eq(Expr::val(source.resource_id.trim().to_owned())),
                ),
        )
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Replace one provider's recording credits without changing any file's tag evidence.
pub async fn replace_recording_credit_source(
    db: &DatabaseTransaction,
    recording: Uuid,
    source: MetadataSourceKind,
    provider: &str,
    credits: &[Credit],
) -> Result<bool> {
    let provider = provider.trim();
    anyhow::ensure!(
        !provider.is_empty() && source != MetadataSourceKind::AudioTag,
        "recording credits require a user or external source"
    );
    let tx = db.begin().await?;
    reconcile(&tx).await?;
    let kind = super::credits::source_name(source)?;
    let Some(recording) = canonical_id(&tx, recording).await? else {
        return Ok(false);
    };
    let id = recording.to_string();

    // Replace the entire ordered contribution of this source in one savepoint.
    use orm_recording_credits::Column as C;
    tx.execute(
        Query::delete()
            .from_table(orm_recording_credits::Entity)
            .and_where(C::SourceKind.eq(kind))
            .and_where(C::SourceProvider.eq(provider))
            .and_where(
                C::RecordingId
                    .eq(&id)
                    .or(column(C::RecordingId).in_subquery(aliases_to(&id))),
            ),
    )
    .await?;
    for (position, credit) in credits.iter().enumerate() {
        let evidence = &credit.source;
        anyhow::ensure!(
            evidence.kind == source
                && credit
                    .source_provider
                    .as_deref()
                    .is_none_or(|name| name == provider),
            "credit source does not match its provider"
        );
        tx.execute(
            Query::insert()
                .into_table(orm_recording_credits::Entity)
                .columns([
                    orm_recording_credits::Column::RecordingId,
                    orm_recording_credits::Column::Position,
                    orm_recording_credits::Column::SourceKind,
                    orm_recording_credits::Column::SourceProvider,
                    orm_recording_credits::Column::CreditedAs,
                    orm_recording_credits::Column::Role,
                    orm_recording_credits::Column::ArtistId,
                    orm_recording_credits::Column::Instrument,
                    orm_recording_credits::Column::SourceKey,
                    orm_recording_credits::Column::SourcePath,
                    orm_recording_credits::Column::SourcePattern,
                    orm_recording_credits::Column::SourceEntityId,
                    orm_recording_credits::Column::SourceConfidence,
                ])
                .values_panic([
                    Expr::val(id.clone()),
                    Expr::val(i64::try_from(position)?),
                    Expr::val(kind),
                    Expr::val(provider.to_owned()),
                    Expr::val(credit.credited_as.clone()),
                    Expr::val(credit.role.clone()),
                    Expr::val(credit.artist_id.map(|id| id.to_string())),
                    Expr::val(credit.instrument.clone()),
                    Expr::val(evidence.key.clone()),
                    Expr::val(evidence.path.clone()),
                    Expr::val(evidence.pattern.clone()),
                    Expr::val(evidence.entity_id.map(|id| id.to_string())),
                    Expr::val(evidence.confidence),
                ]),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Load shared credits and copies, with available lossless files first.
pub async fn recording_metadata(
    db: &impl TransactionTrait,
    id: Uuid,
) -> Result<Option<RecordingMetadata>> {
    let tx = db.begin().await?;
    reconcile(&tx).await?;
    let Some(id) = canonical_id(&tx, id).await? else {
        tx.commit().await?;
        return Ok(None);
    };
    let key = id.to_string();
    let Some(row) = tx
        .query_one(
            Query::select()
                .columns([
                    orm_recordings::Column::MusicbrainzId,
                    orm_recordings::Column::Isrc,
                ])
                .from(orm_recordings::Entity)
                .and_where(Expr::col(orm_recordings::Column::Id).eq(Expr::val(key.clone()))),
        )
        .await?
    else {
        return Ok(None);
    };
    let identifiers = RecordingIdentifiers {
        musicbrainz_id: super::optional_uuid(&row, "musicbrainz_id")?,
        isrc: row.try_get("", "isrc")?,
    };

    // Playback ranking applies after identity has been established.
    use orm_recording_sources::Column as S;
    let mut versions = tx
        .query_all(&local_versions(&key))
        .await?
        .iter()
        .map(tracks::local_variant)
        .collect::<Result<Vec<_>>>()?;
    let sources = tx
        .query_all(
            Query::select()
                .columns([
                    S::Provider,
                    S::ResourceId,
                    S::Available,
                    S::Codec,
                    S::IsLossless,
                    S::SampleRate,
                    S::BitsPerSample,
                    S::Bitrate,
                ])
                .expr_as(Expr::val(0), "start_ms")
                .expr_as(Expr::val(None::<i64>), "end_ms")
                .from(orm_recording_sources::Entity)
                .and_where(S::RecordingId.eq(&key))
                .order_by(S::Provider, Order::Asc)
                .order_by(S::ResourceId, Order::Asc),
        )
        .await?;
    for row in &sources {
        versions.push(tracks::variant(
            row,
            AudioVariantSource::Provider {
                provider: row.try_get("", "provider")?,
                resource_id: row.try_get("", "resource_id")?,
            },
        )?);
    }
    tracks::rank(&mut versions);
    let audio_ids = versions
        .iter()
        .filter_map(|version| match &version.source {
            AudioVariantSource::File { audio_id } => Some(*audio_id),
            AudioVariantSource::Provider { .. } => None,
        })
        .collect();
    let remote_sources = versions
        .iter()
        .filter_map(|version| match &version.source {
            AudioVariantSource::Provider {
                provider,
                resource_id,
            } => Some(RecordingSource {
                provider: provider.clone(),
                resource_id: resource_id.clone(),
            }),
            AudioVariantSource::File { .. } => None,
        })
        .collect();

    let mut base = MetadataBase::default();
    super::credits::load(
        &tx,
        Selection::Recording(id),
        None,
        &mut base,
        &mut Default::default(),
    )
    .await?;
    let direct = tx.query_all(&recording_credits(&key)).await?;
    let mut credits = direct
        .iter()
        .map(super::credits::read)
        .collect::<Result<Vec<_>>>()?;
    credits.extend(base.credits.into_iter().flatten());
    tx.commit().await?;
    Ok(Some(RecordingMetadata {
        id,
        identifiers,
        credits,
        audio_ids,
        remote_sources,
        versions,
    }))
}

async fn canonical_id(db: &impl ConnectionTrait, id: Uuid) -> Result<Option<Uuid>> {
    let row = db
        .query_one(
            Query::select()
                .expr_as(
                    Func::coalesce([
                        field(RECORDING_ALIASES, "target_id"),
                        column(orm_recordings::Column::Id),
                    ]),
                    "id",
                )
                .from(orm_recordings::Entity)
                .left_join(
                    recording_aliases(),
                    field(RECORDING_ALIASES, "source_id").eq(column(orm_recordings::Column::Id)),
                )
                .and_where(orm_recordings::Column::Id.eq(id.to_string())),
        )
        .await?;
    row.map(|row| super::uuid(&row, "id")).transpose()
}

fn aliases_to(id: &str) -> SelectStatement {
    Query::select()
        .column((Alias::new(RECORDING_ALIASES), Alias::new("source_id")))
        .from(recording_aliases())
        .and_where(field(RECORDING_ALIASES, "target_id").eq(id))
        .to_owned()
}

fn recording_credits(id: &str) -> SelectStatement {
    use orm_recording_credits::Column as C;
    let current = Alias::new("current_credit");
    let same_source = Query::select()
        .column((current.clone(), C::RecordingId))
        .from_as(orm_recording_credits::Entity, current.clone())
        .and_where(Expr::col((current.clone(), C::RecordingId)).eq(id))
        .and_where(Expr::col((current.clone(), C::SourceKind)).eq(column(C::SourceKind)))
        .and_where(Expr::col((current, C::SourceProvider)).eq(column(C::SourceProvider)))
        .to_owned();

    // Fall back to an alias only when no current contribution supersedes its source.
    Query::select()
        .columns(
            [
                C::CreditedAs,
                C::Role,
                C::Instrument,
                C::SourceKind,
                C::SourceProvider,
                C::SourceKey,
                C::SourcePath,
                C::SourcePattern,
                C::SourceEntityId,
                C::SourceConfidence,
            ]
            .map(|c| c.as_column_ref()),
        )
        .column(C::ArtistId.as_column_ref())
        .from(orm_recording_credits::Entity)
        .and_where(
            C::RecordingId.eq(id).or(column(C::RecordingId)
                .in_subquery(aliases_to(id))
                .and(Expr::not_exists(same_source))),
        )
        .order_by_columns(
            [C::SourceKind, C::SourceProvider, C::Position]
                .map(|c| (c.as_column_ref(), Order::Asc)),
        )
        .to_owned()
}

fn local_versions(id: &str) -> SelectStatement {
    use media_files::Column as F;
    use track_assets::Column as V;
    use verified::Column as A;

    // The union deduplicates assets reachable through both file and track identities.
    let mut whole = Query::select();
    whole
        .expr_as(column(A::Id), "audio_id")
        .expr_as(Expr::val(0), "start_ms")
        .expr_as(Expr::val(None::<i64>), "end_ms");
    tracks::local_quality(&mut whole);
    whole
        .from(verified_audio())
        .inner_join(media_files::Entity, column(F::Id).eq(column(A::FileId)))
        .and_where(A::RecordingId.eq(id));

    let mut segments = Query::select();
    segments
        .column(A::Id.as_column_ref())
        .columns([V::StartMs, V::EndMs].map(|c| c.as_column_ref()));
    tracks::local_quality(&mut segments);
    segments
        .from(track_recordings())
        .inner_join(
            track_assets::Entity,
            column(V::TrackId).eq(field(TRACK_RECORDINGS, "track_id")),
        )
        .inner_join(verified_audio(), column(A::FileId).eq(column(V::FileId)))
        .inner_join(media_files::Entity, column(F::Id).eq(column(A::FileId)))
        .and_where(field(TRACK_RECORDINGS, "recording_id").eq(id));
    whole
        .union(UnionType::Distinct, segments)
        .order_by(Alias::new("audio_id"), Order::Asc)
        .order_by(Alias::new("start_ms"), Order::Asc);
    whole
}
