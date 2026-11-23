//! Logical release tracks and the audio ranges that can play them.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use anyhow::{Context, Result};
use metadata::model::{
    AudioQuality, AudioVariant, AudioVariantSource, ReleaseTrackMetadata, TrackSegment,
};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, QueryResult, TransactionSession, TransactionTrait,
};
use uuid::Uuid;

use super::{statement, uuid};

pub(super) const LOCAL_QUALITY: &str = "a.codec,
    CASE WHEN a.codec IS NULL THEN NULL
         WHEN lower(a.codec) IN ('flac','alac','wav','pcm','aiff','dsd','wavpack') THEN 1
         WHEN lower(a.codec) IN ('mp3','aac','opus','vorbis','wma') THEN 0 END AS is_lossless,
    a.sample_rate, a.bits_per_sample, a.bitrate, f.local_available AS available";

pub(super) fn title_key(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

struct TrackEvidence {
    file: i32,
    audio: Uuid,
    current: Option<Uuid>,
    number: Option<u32>,
    title: String,
    title_key: String,
    mbid: Option<String>,
}

impl TrackEvidence {
    fn read(row: &QueryResult) -> Result<Self> {
        Ok(Self {
            file: row.try_get("", "file_id")?,
            audio: uuid(row, "id")?,
            current: super::optional_uuid(row, "current_track")?,
            number: number(row, "track_number")?,
            title: row.try_get("", "title")?,
            title_key: row.try_get("", "track_title_key")?,
            mbid: row.try_get("", "release_track_mbid")?,
        })
    }

    fn slot(&self) -> Option<(u32, &str)> {
        self.number.map(|number| (number, self.title_key.as_str()))
    }
}

pub(super) async fn reconcile(db: &impl ConnectionTrait) -> Result<()> {
    let dirty = db
        .query_all_raw(statement(
            "SELECT disc_id FROM music_metadata_dirty_tracks",
            [],
        ))
        .await?;
    for disc in dirty {
        reconcile_disc(db, uuid(&disc, "disc_id")?).await?;
    }
    Ok(())
}

async fn reconcile_disc(db: &impl ConnectionTrait, disc: Uuid) -> Result<()> {
    // Exclude explicit layouts and keep the last logical membership as a retention candidate.
    let rows = db.query_all_raw(statement(
        "SELECT a.*, (SELECT t.id FROM music_metadata_track_assets x
            JOIN music_metadata_release_tracks t ON t.id = x.track_id
            WHERE x.file_id = a.file_id AND t.disc_id = a.disc_id AND x.end_ms IS NULL LIMIT 1) AS current_track
         FROM music_metadata_audio a WHERE a.disc_id = ? AND NOT EXISTS (
            SELECT 1 FROM music_metadata_audio_layouts l WHERE l.file_id = a.file_id)
         ORDER BY a.track_number IS NULL, a.track_number, a.id",
        [disc.to_string().into()],
    )).await?;
    let records = rows
        .iter()
        .map(TrackEvidence::read)
        .collect::<Result<Vec<_>>>()?;
    let mut claims: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for record in &records {
        if let (Some(slot), Some(mbid)) = (record.slot(), record.mbid.as_deref()) {
            claims.entry(slot).or_default().insert(mbid);
        }
    }

    // A weak slot joins a strong track only when exactly one identifier claims the slot.
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for record in &records {
        let strong = record.mbid.as_deref().or_else(|| {
            let candidates = claims.get(&record.slot()?)?;
            candidates
                .first()
                .copied()
                .filter(|_| candidates.len() == 1)
        });
        let key = match (strong, record.slot()) {
            (Some(mbid), _) => (0, format!("mb:{mbid}")),
            (None, Some((number, title))) => (1, format!("slot:{number}:{title}")),
            (None, None) => (2, format!("file:{}", record.audio)),
        };
        groups.entry(key).or_default().push(record);
    }

    // Reserve existing IDs before linking any assets. Explicit layouts use a separate ID space.
    let mut used = BTreeSet::new();
    let mut assignments = Vec::new();
    for ((_, key), group) in groups {
        let mut votes = BTreeMap::<_, (usize, usize)>::new();
        for row in &group {
            if let Some(id) = row.current.filter(|id| !used.contains(id)) {
                let vote = votes.entry(id).or_default();
                vote.0 += usize::from(row.mbid.is_some());
                vote.1 += 1;
            }
        }
        let proposed = Uuid::new_v5(&disc, key.as_bytes());
        let preferred = votes
            .into_iter()
            .max_by_key(|(id, vote)| (*vote, Reverse(*id)))
            .map(|(id, _)| id);
        let id = match preferred {
            Some(id) => id,
            None if !used.contains(&proposed)
                && !records.iter().any(|row| row.current == Some(proposed)) =>
            {
                proposed
            }
            None => Uuid::new_v4(),
        };
        used.insert(id);
        assignments.push((id, group));
    }

    // Insert the desired links first so moving the final encoding cannot delete a retained track.
    for (id, group) in &assignments {
        let display = group
            .iter()
            .find(|row| row.mbid.is_some())
            .unwrap_or(&group[0]);
        put_track(db, *id, disc, display.number, &display.title).await?;
        for row in group {
            if row.current != Some(*id) {
                bind_asset(db, *id, row.file, 0, None).await?;
            }
        }
    }
    for (id, group) in assignments {
        for row in group.into_iter().filter(|row| row.current != Some(id)) {
            db.execute_raw(statement(
                "DELETE FROM music_metadata_track_assets WHERE file_id = ? AND track_id <> ?",
                [row.file.into(), id.to_string().into()],
            ))
            .await?;
        }
    }
    db.execute_raw(statement(
        "DELETE FROM music_metadata_dirty_tracks WHERE disc_id = ?",
        [disc.to_string().into()],
    ))
    .await?;
    Ok(())
}

pub(super) async fn prepare_move(db: &DatabaseTransaction, file: i32, disc: &str) -> Result<()> {
    // Move explicit image layouts before deleting an empty old disc.
    db.execute_raw(statement(
        "UPDATE music_metadata_release_tracks SET disc_id = ?1 WHERE disc_id <> ?1 AND id IN (
            SELECT l.track_id FROM music_metadata_track_assets l
            JOIN music_metadata_audio_layouts layout ON layout.file_id = l.file_id WHERE l.file_id = ?2)",
        [disc.to_owned().into(), file.into()],
    )).await?;
    Ok(())
}

pub(super) async fn duration_rows(
    db: &impl ConnectionTrait,
    id: Uuid,
    album: bool,
) -> Result<Vec<QueryResult>> {
    let filter = if album { "d.album_id" } else { "t.disc_id" };
    // Each logical track contributes only one duration, even with multiple encodings.
    Ok(db.query_all_raw(statement(
        format!("WITH variants AS (
            SELECT a.id AS audio_id,
                CASE WHEN x.end_ms IS NOT NULL THEN x.end_ms - x.start_ms ELSE a.duration_ms END AS duration_ms,
                ROW_NUMBER() OVER (PARTITION BY t.id
                    ORDER BY a.duration_ms IS NULL, f.local_available DESC, f.id) AS pick
            FROM music_metadata_release_tracks t JOIN music_metadata_discs d ON d.id = t.disc_id
            JOIN music_metadata_track_assets x ON x.track_id = t.id
            JOIN music_metadata_audio a ON a.file_id = x.file_id JOIN media_files f ON f.id = a.file_id
            WHERE {filter} = ?
         )
         SELECT audio_id, duration_ms FROM variants WHERE pick = 1"),
        [id.to_string().into()],
    )).await?)
}

async fn put_track(
    db: &impl ConnectionTrait,
    id: Uuid,
    disc: Uuid,
    track: Option<u32>,
    title: &str,
) -> Result<()> {
    let recording = Uuid::new_v5(&id, b"release-track-recording");
    super::recording_identity::ensure_exists(db, recording).await?;
    db.execute_raw(statement(
        "INSERT INTO music_metadata_release_tracks (id, disc_id, recording_id, track_number, title) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET track_number = excluded.track_number, title = excluded.title
         WHERE track_number IS NOT excluded.track_number OR title <> excluded.title",
        [id.to_string().into(), disc.to_string().into(), recording.to_string().into(), track.map(i64::from).into(), title.to_owned().into()],
    )).await?;
    Ok(())
}

async fn bind_asset(
    db: &impl ConnectionTrait,
    track: Uuid,
    file: i32,
    start: i64,
    end: Option<i64>,
) -> Result<()> {
    db.execute_raw(statement(
        "INSERT INTO music_metadata_track_assets (track_id, file_id, start_ms, end_ms) VALUES (?, ?, ?, ?)
         ON CONFLICT(track_id, file_id, start_ms) DO UPDATE SET end_ms = excluded.end_ms",
        [track.to_string().into(), file.into(), start.into(), end.into()],
    )).await?;
    Ok(())
}

/// Replace a disc image's logical track layout. Pass an empty slice to restore automatic indexing.
pub async fn set_track_layout(
    db: &DatabaseTransaction,
    audio_id: Uuid,
    segments: &[TrackSegment],
) -> Result<Vec<Uuid>> {
    let tx = db.begin().await?;
    let row = tx
        .query_one_raw(statement(
            "SELECT file_id, disc_id, duration_ms FROM music_metadata_audio WHERE id = ?",
            [audio_id.to_string().into()],
        ))
        .await?
        .context("audio asset was not indexed")?;
    let file: i32 = row.try_get("", "file_id")?;
    let disc = uuid(&row, "disc_id")?;
    let duration = time(&row, "duration_ms")?;
    let ids = write_layout(&tx, file, audio_id, disc, duration, segments, "manual").await?;
    reconcile_disc(&tx, disc).await?;
    tx.commit().await?;
    Ok(ids)
}

pub(super) async fn write_layout(
    tx: &DatabaseTransaction,
    file: i32,
    audio_id: Uuid,
    disc: Uuid,
    duration: Option<u64>,
    segments: &[TrackSegment],
    kind: &str,
) -> Result<Vec<Uuid>> {
    let mut previous = (0, 0);

    // Validate the entire layout before changing any existing track or asset link.
    for segment in segments {
        anyhow::ensure!(
            segment.track_number > previous.0
                && segment.start_ms >= previous.1
                && segment.end_ms > segment.start_ms
                && duration.is_none_or(|end| segment.end_ms <= end)
                && !segment.title.trim().is_empty(),
            "invalid or overlapping track layout"
        );
        previous = (segment.track_number, segment.end_ms);
    }
    let previous_links = tx
        .query_all_raw(statement(
            "SELECT track_id, start_ms FROM music_metadata_track_assets WHERE file_id = ?",
            [file.into()],
        ))
        .await?;
    tx.execute_raw(statement(
        "DELETE FROM music_metadata_audio_layouts WHERE file_id = ?",
        [file.into()],
    ))
    .await?;
    tx.execute_raw(statement(
        "INSERT INTO music_metadata_dirty_tracks VALUES (?) ON CONFLICT DO NOTHING",
        [disc.to_string().into()],
    ))
    .await?;

    // Segment identity is independent of its mutable title and range.
    let mut ids = BTreeMap::new();
    if !segments.is_empty() {
        tx.execute_raw(statement(
            "INSERT INTO music_metadata_audio_layouts (file_id, kind) VALUES (?, ?)",
            [file.into(), kind.into()],
        ))
        .await?;
        for segment in segments {
            let id = Uuid::new_v5(
                &audio_id,
                format!("cue:{}", segment.track_number).as_bytes(),
            );
            put_track(
                tx,
                id,
                disc,
                Some(segment.track_number),
                segment.title.trim(),
            )
            .await?;
            bind_asset(
                tx,
                id,
                file,
                segment.start_ms.try_into()?,
                Some(segment.end_ms.try_into()?),
            )
            .await?;
            ids.insert((id, i64::try_from(segment.start_ms)?), segment.track_number);
        }
    }

    // A retained segment must never lose its last link while moving its range.
    for row in previous_links {
        let track = uuid(&row, "track_id")?;
        let start: i64 = row.try_get("", "start_ms")?;
        if !ids.contains_key(&(track, start)) {
            tx.execute_raw(statement(
                "DELETE FROM music_metadata_track_assets WHERE track_id = ? AND file_id = ? AND start_ms = ?",
                [track.to_string().into(), file.into(), start.into()],
            )).await?;
        }
    }
    let mut ordered: Vec<_> = ids
        .into_iter()
        .map(|((id, _), number)| (number, id))
        .collect();
    ordered.sort_unstable();
    Ok(ordered.into_iter().map(|(_, id)| id).collect())
}

pub(super) async fn cue_layout_matches(
    tx: &DatabaseTransaction,
    file: i32,
    segments: &[TrackSegment],
) -> Result<bool> {
    let rows = tx.query_all_raw(statement(
        "SELECT t.track_number, t.title, a.start_ms, a.end_ms FROM music_metadata_track_assets a
         JOIN music_metadata_release_tracks t ON t.id = a.track_id
         WHERE a.file_id = ? ORDER BY t.track_number",
        [file.into()],
    )).await?;
    if rows.len() != segments.len() {
        return Ok(false);
    }
    for (row, segment) in rows.iter().zip(segments) {
        if number(row, "track_number")? != Some(segment.track_number)
            || row.try_get::<String>("", "title")? != segment.title
            || time(row, "start_ms")? != Some(segment.start_ms)
            || time(row, "end_ms")? != Some(segment.end_ms)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Read edition track IDs in playback order.
pub async fn disc_track_ids(db: &impl TransactionTrait, disc: Uuid) -> Result<Vec<Uuid>> {
    let tx = db.begin().await?;
    reconcile(&tx).await?;
    let ids = tx
        .query_all_raw(statement(
            "SELECT id FROM music_metadata_release_tracks WHERE disc_id = ?
         ORDER BY track_number IS NULL, track_number, id",
            [disc.to_string().into()],
        ))
        .await?
        .iter()
        .map(|row| uuid(row, "id"))
        .collect::<Result<Vec<_>>>()?;
    tx.commit().await?;
    Ok(ids)
}

/// Return the assets of a logical track, ranked uniformly with recording copies.
pub async fn release_track_metadata(
    db: &impl TransactionTrait,
    id: Uuid,
) -> Result<Option<ReleaseTrackMetadata>> {
    let tx = db.begin().await?;
    super::recording_identity::reconcile(&tx).await?;
    reconcile(&tx).await?;
    let key = id.to_string();
    let Some(row) = tx
        .query_one_raw(statement(
            "SELECT t.disc_id, t.track_number, t.title, r.recording_id
             FROM music_metadata_release_tracks t JOIN music_metadata_track_recordings r ON r.track_id = t.id
             WHERE t.id = ?",
            [key.clone().into()],
        ))
        .await?
    else {
        return Ok(None);
    };
    let rows = tx
        .query_all_raw(statement(
            format!(
                "SELECT a.id AS audio_id, l.start_ms, l.end_ms, {LOCAL_QUALITY}
         FROM music_metadata_track_assets l JOIN music_metadata_current_recording_audio a ON a.file_id = l.file_id
         JOIN media_files f ON f.id = l.file_id WHERE l.track_id = ? ORDER BY a.id"
            ),
            [key.into()],
        ))
        .await?;
    let mut variants = rows.iter().map(local_variant).collect::<Result<Vec<_>>>()?;
    rank(&mut variants);
    let mut result = ReleaseTrackMetadata {
        id,
        disc_id: uuid(&row, "disc_id")?,
        recording_id: uuid(&row, "recording_id")?,
        track_number: number(&row, "track_number")?,
        title: row.try_get("", "title")?,
        variants,
        provenance: None,
    };
    super::overlays::apply(
        &tx,
        super::overlays::OverlayTarget::ReleaseTrack(id),
        &mut result,
    )
    .await?;
    tx.commit().await?;
    Ok(Some(result))
}

pub(super) fn local_variant(row: &QueryResult) -> Result<AudioVariant> {
    variant(
        row,
        AudioVariantSource::File {
            audio_id: uuid(row, "audio_id")?,
        },
    )
}

pub(super) fn variant(row: &QueryResult, source: AudioVariantSource) -> Result<AudioVariant> {
    Ok(AudioVariant {
        source,
        available: row.try_get("", "available")?,
        quality: AudioQuality {
            codec: row.try_get("", "codec")?,
            is_lossless: row.try_get("", "is_lossless")?,
            sample_rate: number(row, "sample_rate")?,
            bits_per_sample: number(row, "bits_per_sample")?,
            bitrate: time(row, "bitrate")?,
        },
        start_ms: time(row, "start_ms")?.unwrap_or_default(),
        end_ms: time(row, "end_ms")?,
    })
}

pub(super) fn rank(variants: &mut [AudioVariant]) {
    variants.sort_by_key(|v| {
        std::cmp::Reverse((
            v.available,
            v.quality.is_lossless == Some(true),
            v.quality.bits_per_sample,
            v.quality.sample_rate,
            v.quality.bitrate,
        ))
    });
}

fn number(row: &QueryResult, field: &str) -> Result<Option<u32>> {
    Ok(row
        .try_get::<Option<i64>>("", field)?
        .map(u32::try_from)
        .transpose()?)
}

fn time(row: &QueryResult, field: &str) -> Result<Option<u64>> {
    Ok(row
        .try_get::<Option<i64>>("", field)?
        .map(u64::try_from)
        .transpose()?)
}
