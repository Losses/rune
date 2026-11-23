//! Reconcile normalized identifier evidence without letting a partial tag seed a match.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use metadata::model::RecordingIdentifiers;
use sea_orm::{ConnectionTrait, QueryResult};
use uuid::Uuid;

use super::{statement, uuid};

pub(super) const NAMESPACE: Uuid = Uuid::from_u128(0x59e7e858_d294_50cc_9468_d2b63f436799);

pub(super) fn audio_fallback(audio: Uuid) -> Uuid {
    Uuid::new_v5(&audio, b"recording")
}

pub(super) fn remote_fallback(provider: &str, resource: &str) -> Uuid {
    let provider = Uuid::new_v5(&NAMESPACE, provider.as_bytes());
    Uuid::new_v5(&provider, resource.as_bytes())
}

pub(super) fn isrc(value: &str) -> Option<String> {
    let value = value.trim().replace('-', "").to_ascii_uppercase();
    let bytes = value.as_bytes();
    (bytes.len() == 12
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..5].iter().all(u8::is_ascii_alphanumeric)
        && bytes[5..].iter().all(u8::is_ascii_digit))
    .then_some(value)
}

pub(super) async fn ensure_exists(db: &impl ConnectionTrait, id: Uuid) -> Result<()> {
    db.execute_raw(statement(
        "INSERT INTO music_metadata_recordings (id) VALUES (?) ON CONFLICT(id) DO NOTHING",
        [id.to_string().into()],
    ))
    .await?;
    Ok(())
}

pub(super) async fn prepare_audio(
    db: &impl ConnectionTrait,
    audio: Uuid,
    ids: &RecordingIdentifiers,
) -> Result<Uuid> {
    // Reuse an unchanged assignment. A changed or new source starts on its private UUID.
    if let Some(row) = db
        .query_one_raw(statement(
            "SELECT recording_id FROM music_metadata_audio WHERE id = ?
         AND recording_mbid IS ? AND recording_isrc IS ?",
            [
                audio.to_string().into(),
                ids.musicbrainz_id.map(|id| id.to_string()).into(),
                ids.isrc.clone().into(),
            ],
        ))
        .await?
    {
        return uuid(&row, "recording_id");
    }
    let fallback = audio_fallback(audio);
    ensure_exists(db, fallback).await?;
    Ok(fallback)
}

#[derive(Debug)]
enum Owner {
    Audio { file: i32, id: Uuid },
    Remote { provider: String, resource: String },
}

struct Evidence {
    owner: Owner,
    current: Uuid,
    mbid: Option<String>,
    isrc: Option<String>,
}

impl Evidence {
    fn read(row: &QueryResult) -> Result<Self> {
        let owner = if let Some(file) = row.try_get::<Option<i32>>("", "file_id")? {
            Owner::Audio {
                file,
                id: uuid(row, "audio_id")?,
            }
        } else {
            Owner::Remote {
                provider: row.try_get("", "provider")?,
                resource: row.try_get("", "resource_id")?,
            }
        };
        Ok(Self {
            owner,
            current: uuid(row, "recording_id")?,
            mbid: row.try_get("", "mbid")?,
            isrc: row.try_get("", "isrc")?,
        })
    }

    fn fallback(&self) -> Uuid {
        match &self.owner {
            Owner::Audio { id, .. } => audio_fallback(*id),
            Owner::Remote { provider, resource } => remote_fallback(provider, resource),
        }
    }
}

type Claims<'a> = BTreeMap<&'a str, BTreeSet<&'a str>>;

/// Rebuild only identifier components touched by source changes in the current transaction.
pub(in crate::actions) async fn reconcile(db: &impl ConnectionTrait) -> Result<()> {
    if db
        .query_one_raw(statement(
            "SELECT 1 FROM music_metadata_recording_dirty LIMIT 1",
            [],
        ))
        .await?
        .is_none()
    {
        return Ok(());
    }
    // The bipartite identifier graph expands equally from the old and new identifiers.
    let rows = db.query_all_raw(statement(
        "WITH RECURSIVE keys(kind, value) AS (
            SELECT kind, value FROM music_metadata_recording_dirty WHERE kind <> 'recording'
            UNION
            SELECT 'isrc', e.isrc FROM keys k JOIN music_metadata_recording_evidence e ON e.mbid = k.value
                WHERE k.kind = 'mbid' AND e.isrc IS NOT NULL
            UNION
            SELECT 'mbid', e.mbid FROM keys k JOIN music_metadata_recording_evidence e ON e.isrc = k.value
                WHERE k.kind = 'isrc' AND e.mbid IS NOT NULL
         )
         SELECT e.* FROM keys k JOIN music_metadata_recording_evidence e ON e.mbid = k.value WHERE k.kind = 'mbid'
         UNION
         SELECT e.* FROM keys k JOIN music_metadata_recording_evidence e ON e.isrc = k.value WHERE k.kind = 'isrc'",
        [],
    )).await?;
    let evidence = rows
        .iter()
        .map(Evidence::read)
        .collect::<Result<Vec<_>>>()?;
    let (mut by_mbid, mut by_isrc) = (Claims::new(), Claims::new());
    for entry in &evidence {
        if let (Some(mbid), Some(isrc)) = (entry.mbid.as_deref(), entry.isrc.as_deref()) {
            by_mbid.entry(mbid).or_default().insert(isrc);
            by_isrc.entry(isrc).or_default().insert(mbid);
        }
    }

    // Clear derived identifiers while retaining the identity and its credited history.
    db.execute_raw(statement(
        "UPDATE music_metadata_recordings SET musicbrainz_id = NULL, isrc = NULL
         WHERE id IN (SELECT value FROM music_metadata_recording_dirty WHERE kind = 'recording')
            OR musicbrainz_id IN (SELECT value FROM music_metadata_recording_dirty WHERE kind = 'mbid')
            OR isrc IN (SELECT value FROM music_metadata_recording_dirty WHERE kind = 'isrc')",
        [],
    )).await?;
    let mut known = BTreeSet::new();
    for entry in &evidence {
        let (id, mbid, isrc) = classify(entry, &by_mbid, &by_isrc);
        if known.insert(id) {
            update_projection(db, id, mbid, isrc).await?;
        }
        if id != entry.current {
            attach(db, &entry.owner, id).await?;
        }
    }
    db.execute_raw(statement("DELETE FROM music_metadata_recording_dirty", []))
        .await?;
    Ok(())
}

fn classify<'a>(
    entry: &'a Evidence,
    by_mbid: &Claims<'a>,
    by_isrc: &Claims<'a>,
) -> (Uuid, Option<&'a str>, Option<&'a str>) {
    let isrc = entry.isrc.as_deref();
    let mbid = match (
        entry.mbid.as_deref(),
        isrc.and_then(|isrc| by_isrc.get(isrc)),
    ) {
        (Some(mbid), _) => Some(mbid),
        (None, Some(candidates)) if candidates.len() == 1 => candidates.first().copied(),
        (None, Some(_)) => return (entry.fallback(), None, None),
        (None, None) => None,
    };

    // Exact contradictory pairs stay distinct. Unqualified evidence cannot decide between them.
    if let Some(mbid) = mbid {
        let candidates = by_mbid.get(mbid);
        if candidates.is_some_and(|isrcs| isrcs.len() > 1) {
            let Some(isrc) = isrc else {
                return (entry.fallback(), None, None);
            };
            return (
                Uuid::new_v5(&NAMESPACE, format!("mb:{mbid}:{isrc}").as_bytes()),
                Some(mbid),
                Some(isrc),
            );
        }
        let isrc = candidates.and_then(BTreeSet::first).copied().or(isrc);
        return (
            Uuid::new_v5(&NAMESPACE, format!("mb:{mbid}:").as_bytes()),
            Some(mbid),
            isrc,
        );
    }
    if let Some(isrc) = isrc {
        return (
            Uuid::new_v5(&NAMESPACE, format!("isrc:{isrc}").as_bytes()),
            None,
            Some(isrc),
        );
    }
    (entry.fallback(), None, None)
}

async fn update_projection(
    db: &impl ConnectionTrait,
    id: Uuid,
    mbid: Option<&str>,
    isrc: Option<&str>,
) -> Result<()> {
    db.execute_raw(statement(
        "INSERT INTO music_metadata_recordings (id, musicbrainz_id, isrc, legacy_isrc) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET musicbrainz_id = excluded.musicbrainz_id, isrc = excluded.isrc,
            legacy_isrc = COALESCE(legacy_isrc, excluded.legacy_isrc)",
        [id.to_string().into(), mbid.into(), isrc.into(), isrc.filter(|_| mbid.is_none()).into()],
    )).await?;
    Ok(())
}

async fn attach(db: &impl ConnectionTrait, owner: &Owner, id: Uuid) -> Result<()> {
    let id = id.to_string();
    let query = match owner {
        Owner::Audio { file, .. } => statement(
            "UPDATE music_metadata_audio SET recording_id = ? WHERE file_id = ?",
            [id.into(), (*file).into()],
        ),
        Owner::Remote { provider, resource } => statement(
            "UPDATE music_metadata_recording_sources SET recording_id = ? WHERE provider = ? AND resource_id = ?",
            [id.into(), provider.clone().into(), resource.clone().into()],
        ),
    };
    db.execute_raw(query).await?;
    Ok(())
}
