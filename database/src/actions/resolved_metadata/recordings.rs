//! Recordings shared by files and provider resources.

use anyhow::Result;
use metadata::model::{
    AudioQuality, AudioVariantSource, Credit, MetadataBase, MetadataSourceKind,
    RecordingIdentifiers, RecordingMetadata, RecordingSource,
};
use sea_orm::{ConnectionTrait, DatabaseTransaction, TransactionSession, TransactionTrait};
use uuid::Uuid;

use super::{
    details::Selection,
    recording_identity::{ensure_exists, isrc, reconcile, remote_fallback},
    statement, tracks,
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

    let fallback = remote_fallback(provider, resource);
    ensure_exists(&tx, fallback).await?;

    // Store the provider's current assertion. A later refresh can remove either identifier.
    tx.execute_raw(statement(
        "INSERT INTO music_metadata_recording_sources
            (provider, resource_id, recording_id, recording_mbid, recording_isrc) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(provider, resource_id) DO UPDATE SET
            recording_id = CASE WHEN recording_mbid IS excluded.recording_mbid
              AND recording_isrc IS excluded.recording_isrc THEN recording_id ELSE excluded.recording_id END,
            recording_mbid = excluded.recording_mbid, recording_isrc = excluded.recording_isrc",
        [provider.to_owned().into(), resource.to_owned().into(), fallback.to_string().into(),
         identifiers.musicbrainz_id.map(|id| id.to_string()).into(), identifiers.isrc.into()],
    )).await?;
    reconcile(&tx).await?;
    let row = tx.query_one_raw(statement(
        "SELECT recording_id FROM music_metadata_recording_sources WHERE provider = ? AND resource_id = ?",
        [provider.to_owned().into(), resource.to_owned().into()],
    )).await?.ok_or_else(|| anyhow::anyhow!("missing recording source after registration"))?;
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
    let result = db.execute_raw(statement(
        "UPDATE music_metadata_recording_sources SET available = ?, codec = ?, is_lossless = ?,
             sample_rate = ?, bits_per_sample = ?, bitrate = ? WHERE provider = ? AND resource_id = ?",
        [available.into(), quality.codec.clone().into(), quality.is_lossless.into(),
         quality.sample_rate.map(i64::from).into(), quality.bits_per_sample.map(i64::from).into(),
         quality.bitrate.map(i64::try_from).transpose()?.into(),
         source.provider.trim().to_owned().into(), source.resource_id.trim().to_owned().into()],
    )).await?;
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
    tx.execute_raw(statement(
        "DELETE FROM music_metadata_recording_credits WHERE source_kind = ?2 AND source_provider = ?3
         AND (recording_id = ?1 OR recording_id IN (
             SELECT source_id FROM music_metadata_recording_aliases WHERE target_id = ?1))",
        [id.clone().into(), kind.into(), provider.to_owned().into()],
    )).await?;
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
        tx.execute_raw(statement(
            "INSERT INTO music_metadata_recording_credits (recording_id, position, source_kind,
                source_provider, credited_as, role, artist_id, instrument, source_key, source_path,
                source_pattern, source_entity_id, source_confidence) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            [id.clone().into(), i64::try_from(position)?.into(), kind.into(), provider.to_owned().into(),
             credit.credited_as.clone().into(), credit.role.clone().into(), credit.artist_id.map(|id| id.to_string()).into(),
             credit.instrument.clone().into(), evidence.key.clone().into(), evidence.path.clone().into(),
             evidence.pattern.clone().into(), evidence.entity_id.map(|id| id.to_string()).into(), evidence.confidence.into()],
        )).await?;
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
        .query_one_raw(statement(
            "SELECT musicbrainz_id, isrc FROM music_metadata_recordings WHERE id = ?",
            [key.clone().into()],
        ))
        .await?
    else {
        return Ok(None);
    };
    let identifiers = RecordingIdentifiers {
        musicbrainz_id: super::optional_uuid(&row, "musicbrainz_id")?,
        isrc: row.try_get("", "isrc")?,
    };

    // Playback ranking applies after identity has been established.
    let mut versions = tx
        .query_all_raw(statement(
            format!(
                "SELECT a.id AS audio_id, 0 AS start_ms, NULL AS end_ms, {quality}
             FROM music_metadata_current_recording_audio a JOIN media_files f ON f.id = a.file_id
             WHERE a.recording_id = ?1
             UNION
             SELECT a.id, v.start_ms, v.end_ms, {quality}
             FROM music_metadata_track_recordings r JOIN music_metadata_track_assets v ON v.track_id = r.track_id
             JOIN music_metadata_current_recording_audio a ON a.file_id = v.file_id
             JOIN media_files f ON f.id = a.file_id WHERE r.recording_id = ?1
             ORDER BY audio_id, start_ms",
                quality = tracks::LOCAL_QUALITY
            ),
            [key.clone().into()],
        ))
        .await?
        .iter()
        .map(tracks::local_variant)
        .collect::<Result<Vec<_>>>()?;
    let sources = tx
        .query_all_raw(statement(
            "SELECT provider, resource_id, available, codec, is_lossless, sample_rate,
            bits_per_sample, bitrate, 0 AS start_ms, NULL AS end_ms
         FROM music_metadata_recording_sources WHERE recording_id = ?
         ORDER BY provider, resource_id",
            [key.clone().into()],
        ))
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
    let direct = tx
        .query_all_raw(statement(
            "SELECT c.credited_as, c.role, COALESCE(r.target_uuid, c.artist_id) AS artist_id,
                c.instrument, c.source_kind, c.source_provider, c.source_key, c.source_path,
                c.source_pattern, c.source_entity_id, c.source_confidence
         FROM music_metadata_recording_credits c
         LEFT JOIN music_metadata_artist_redirects r ON r.source_uuid = c.artist_id
         WHERE c.recording_id = ?1 OR (c.recording_id IN (
            SELECT source_id FROM music_metadata_recording_aliases WHERE target_id = ?1)
            AND NOT EXISTS (SELECT 1 FROM music_metadata_recording_credits current
                WHERE current.recording_id = ?1 AND current.source_kind = c.source_kind
                  AND current.source_provider = c.source_provider))
         ORDER BY c.source_kind, c.source_provider, c.position",
            [key.into()],
        ))
        .await?;
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
        .query_one_raw(statement(
            "SELECT COALESCE(a.target_id, r.id) AS id FROM music_metadata_recordings r
         LEFT JOIN music_metadata_recording_aliases a ON a.source_id = r.id WHERE r.id = ?",
            [id.to_string().into()],
        ))
        .await?;
    row.map(|row| super::uuid(&row, "id")).transpose()
}
