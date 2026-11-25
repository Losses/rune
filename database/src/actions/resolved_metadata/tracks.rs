//! Logical release tracks and the audio ranges that can play them.

use crate::actions::resolved_metadata::queries::{
    TRACK_RECORDINGS, field, track_recordings, verified_audio,
};
use crate::entities::music_metadata::audio as orm_audio;
use crate::entities::music_metadata::audio_layouts as orm_audio_layouts;
use crate::entities::music_metadata::dirty_tracks as orm_dirty_tracks;
use crate::entities::music_metadata::release_tracks as orm_release_tracks;
use crate::entities::music_metadata::track_assets as orm_track_assets;
use sea_orm::ExprTrait;
use sea_orm::sea_query::{
    Alias, BinOper, Expr, Func, OnConflict, Order, Query, SelectStatement, WindowStatement,
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use anyhow::Result;
use metadata::model::{
    AudioQuality, AudioVariant, AudioVariantSource, ReleaseTrackMetadata, TrackSegment,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, QueryResult, TransactionSession,
    TransactionTrait,
};
use uuid::Uuid;

use crate::entities::{
    media_files,
    music_metadata::{audio as verified, discs},
};

use super::{column, scalar, uuid};

pub(super) fn local_quality(query: &mut SelectStatement) {
    use verified::Column as V;
    let codec: Expr = Func::lower(column(V::Codec)).into();
    let lossless = Expr::case(
        codec
            .clone()
            .is_in(["flac", "alac", "wav", "pcm", "aiff", "dsd", "wavpack"]),
        1,
    )
    .case(codec.is_in(["mp3", "aac", "opus", "vorbis", "wma"]), 0);
    query
        .column(V::Codec.as_column_ref())
        .expr_as(lossless, "is_lossless")
        .columns([V::SampleRate, V::BitsPerSample, V::Bitrate].map(|c| c.as_column_ref()))
        .expr_as(column(media_files::Column::LocalAvailable), "available");
}

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
    recording: Uuid,
    anonymous: bool,
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
            recording: uuid(row, "recording_id")?,
            anonymous: row
                .try_get::<Option<String>>("", "recording_mbid")?
                .is_none()
                && row
                    .try_get::<Option<String>>("", "recording_isrc")?
                    .is_none(),
        })
    }

    fn slot(&self) -> Option<(u32, &str)> {
        self.number.map(|number| (number, self.title_key.as_str()))
    }
}

pub(super) async fn reconcile(db: &impl ConnectionTrait) -> Result<()> {
    let dirty = db
        .query_all(
            Query::select()
                .column(orm_dirty_tracks::Column::DiscId)
                .from(orm_dirty_tracks::Entity),
        )
        .await?;
    for disc in dirty {
        reconcile_disc(db, uuid(&disc, "disc_id")?).await?;
    }
    Ok(())
}

async fn reconcile_disc(db: &impl ConnectionTrait, disc: Uuid) -> Result<()> {
    use orm_audio::Column as A;
    use orm_release_tracks::Column as T;
    use orm_track_assets::Column as V;
    let key = disc.to_string();

    // Moves can leave a former asset linked to a track on the previous disc.
    let old_tracks = Query::select()
        .column(T::Id)
        .from(orm_release_tracks::Entity)
        .and_where(T::DiscId.eq(&key))
        .to_owned();
    let current_files = Query::select()
        .column(A::FileId)
        .from(orm_audio::Entity)
        .and_where(A::DiscId.eq(&key))
        .to_owned();
    db.execute(
        Query::delete()
            .from_table(orm_track_assets::Entity)
            .and_where(column(V::TrackId).in_subquery(old_tracks))
            .and_where(column(V::FileId).not_in_subquery(current_files)),
    )
    .await?;

    // Find the current file assignment only for whole-file, automatically assigned tracks.
    let current = Query::select()
        .column(T::Id.as_column_ref())
        .from(orm_track_assets::Entity)
        .inner_join(
            orm_release_tracks::Entity,
            column(T::Id).eq(column(V::TrackId)),
        )
        .and_where(column(V::FileId).eq(column(A::FileId)))
        .and_where(column(T::DiscId).eq(column(A::DiscId)))
        .and_where(V::EndMs.is_null())
        .limit(1)
        .to_owned();
    let layout = Query::select()
        .column(orm_audio_layouts::Column::FileId.as_column_ref())
        .from(orm_audio_layouts::Entity)
        .and_where(column(orm_audio_layouts::Column::FileId).eq(column(A::FileId)))
        .to_owned();
    let rows = db
        .query_all(
            Query::select()
                .columns(
                    [
                        A::FileId,
                        A::Id,
                        A::TrackNumber,
                        A::Title,
                        A::TrackTitleKey,
                        A::ReleaseTrackMbid,
                        A::RecordingId,
                        A::RecordingMbid,
                        A::RecordingIsrc,
                    ]
                    .map(|c| c.as_column_ref()),
                )
                .expr_as(scalar(current), "current_track")
                .from(orm_audio::Entity)
                .and_where(A::DiscId.eq(disc.to_string()))
                .and_where(Expr::not_exists(layout))
                .order_by_expr(A::TrackNumber.is_null(), Order::Asc)
                .order_by(A::TrackNumber.as_column_ref(), Order::Asc)
                .order_by(A::Id.as_column_ref(), Order::Asc),
        )
        .await?;
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
    let mut replaced_recordings = BTreeSet::new();
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
        let fallback = track_fallback(*id);
        let recording = if let Some(row) = group.iter().find(|row| row.anonymous) {
            // A migration or a directory move can change the track UUID while keeping its performance.
            let shared = db
                .query_one(
                    Query::select()
                        .expr(Expr::val(1))
                        .from(orm_audio::Entity)
                        .and_where(A::RecordingId.eq(row.recording.to_string()))
                        .and_where(A::FileId.is_not_in(group.iter().map(|row| row.file)))
                        .limit(1),
                )
                .await?;
            if shared.is_some() {
                Uuid::new_v5(&row.recording, id.as_bytes())
            } else {
                row.recording
            }
        } else {
            fallback
        };
        put_track(db, *id, disc, recording, display.number, &display.title).await?;

        // Whole-file variants without identifiers share their release track's fallback.
        let unassigned = group
            .iter()
            .filter(|row| row.anonymous && row.recording != recording);
        for row in unassigned {
            replaced_recordings.insert(row.recording);
            db.execute(
                Query::update()
                    .table(orm_audio::Entity)
                    .value(A::RecordingId, Expr::val(recording.to_string()))
                    .and_where(A::FileId.eq(row.file)),
            )
            .await?;
        }
        for row in group {
            if row.current != Some(*id) {
                bind_asset(db, *id, row.file, 0, None).await?;
            }
        }
    }
    for (id, group) in assignments {
        for row in group.into_iter().filter(|row| row.current != Some(id)) {
            db.execute(
                Query::delete()
                    .from_table(orm_track_assets::Entity)
                    .and_where(Expr::col(orm_track_assets::Column::FileId).eq(Expr::val(row.file)))
                    .and_where(
                        Expr::col(orm_track_assets::Column::TrackId).ne(Expr::val(id.to_string())),
                    ),
            )
            .await?;
        }
    }
    // Keep logical tracks only while a file or segment still supplies an asset.
    let active = Query::select()
        .column(V::TrackId)
        .from(orm_track_assets::Entity)
        .and_where(column(V::TrackId).eq(column(T::Id)))
        .to_owned();
    db.execute(
        Query::delete()
            .from_table(orm_release_tracks::Entity)
            .and_where(T::DiscId.eq(&key))
            .and_where(Expr::not_exists(active)),
    )
    .await?;
    super::lifecycle::finish_disc(db, &key).await?;
    super::recording_identity::prune_unreferenced(db, replaced_recordings).await?;
    db.execute(
        Query::delete()
            .from_table(orm_dirty_tracks::Entity)
            .and_where(Expr::col(orm_dirty_tracks::Column::DiscId).eq(Expr::val(disc.to_string()))),
    )
    .await?;
    Ok(())
}

pub(super) async fn prepare_move(db: &DatabaseTransaction, file: i32, disc: &str) -> Result<()> {
    super::lifecycle::queue_audio_move(db, file, disc).await?;
    // Move explicit image layouts before deleting an empty old disc.
    use orm_release_tracks::Column as T;
    use orm_track_assets::Column as V;
    let tracks = Query::select()
        .column(V::TrackId.as_column_ref())
        .from(orm_track_assets::Entity)
        .inner_join(
            orm_audio_layouts::Entity,
            column(orm_audio_layouts::Column::FileId).eq(column(V::FileId)),
        )
        .and_where(V::FileId.eq(file))
        .to_owned();
    db.execute(
        Query::update()
            .table(orm_release_tracks::Entity)
            .value(T::DiscId, Expr::val(disc))
            .and_where(T::DiscId.ne(disc))
            .and_where(column(T::Id).in_subquery(tracks)),
    )
    .await?;
    Ok(())
}

pub(super) async fn duration_rows(
    db: &impl ConnectionTrait,
    id: Uuid,
    album: bool,
) -> Result<Vec<QueryResult>> {
    use media_files::Column as F;
    use orm_audio::Column as A;
    use orm_release_tracks::Column as T;
    use orm_track_assets::Column as V;
    let key = id.to_string();
    let filter = if album {
        discs::Column::AlbumId.eq(key)
    } else {
        T::DiscId.eq(key)
    };

    // Rank versions once for each logical track so only one duration contributes.
    let mut window = WindowStatement::partition_by(T::Id.as_column_ref());
    window
        .order_by_expr(A::DurationMs.is_null(), Order::Asc)
        .order_by(F::LocalAvailable.as_column_ref(), Order::Desc)
        .order_by(F::Id.as_column_ref(), Order::Asc);
    let duration = Expr::case(
        V::EndMs.is_not_null(),
        column(V::EndMs).sub(column(V::StartMs)),
    )
    .finally(column(A::DurationMs));
    let variants = Query::select()
        .expr_as(column(A::Id), "audio_id")
        .expr_as(duration, "duration_ms")
        .expr_window_as(Func::cust("ROW_NUMBER"), window, "pick")
        .from(orm_release_tracks::Entity)
        .inner_join(
            discs::Entity,
            column(discs::Column::Id).eq(column(T::DiscId)),
        )
        .inner_join(
            orm_track_assets::Entity,
            column(V::TrackId).eq(column(T::Id)),
        )
        .inner_join(orm_audio::Entity, column(A::FileId).eq(column(V::FileId)))
        .inner_join(media_files::Entity, column(F::Id).eq(column(A::FileId)))
        .and_where(filter)
        .to_owned();
    Ok(db
        .query_all(
            Query::select()
                .columns([Alias::new("audio_id"), Alias::new("duration_ms")])
                .from_subquery(variants, "variants")
                .and_where(Expr::col(Alias::new("pick")).eq(1)),
        )
        .await?)
}

fn track_fallback(track: Uuid) -> Uuid {
    Uuid::new_v5(&track, b"release-track-recording")
}

async fn put_track(
    db: &impl ConnectionTrait,
    id: Uuid,
    disc: Uuid,
    recording: Uuid,
    track: Option<u32>,
    title: &str,
) -> Result<()> {
    super::recording_identity::ensure_exists(db, recording).await?;
    use orm_release_tracks::Column as T;
    let excluded = Alias::new("excluded");
    let changed = column(T::TrackNumber)
        .binary(
            BinOper::IsNot,
            Expr::col((excluded.clone(), T::TrackNumber)),
        )
        .or(column(T::Title).ne(Expr::col((excluded.clone(), T::Title))))
        .or(column(T::RecordingId).ne(Expr::col((excluded, T::RecordingId))));
    db.execute(
        Query::insert()
            .into_table(orm_release_tracks::Entity)
            .columns([T::Id, T::DiscId, T::RecordingId, T::TrackNumber, T::Title])
            .values([
                Expr::val(id.to_string()),
                Expr::val(disc.to_string()),
                Expr::val(recording.to_string()),
                Expr::val(track.map(i64::from)),
                Expr::val(title),
            ])?
            .on_conflict(
                OnConflict::column(T::Id)
                    .update_columns([T::RecordingId, T::TrackNumber, T::Title])
                    .action_and_where(changed)
                    .to_owned(),
            ),
    )
    .await?;
    Ok(())
}

async fn bind_asset(
    db: &impl ConnectionTrait,
    track: Uuid,
    file: i32,
    start: i64,
    end: Option<i64>,
) -> Result<()> {
    db.execute(
        Query::insert()
            .into_table(orm_track_assets::Entity)
            .columns([
                orm_track_assets::Column::TrackId,
                orm_track_assets::Column::FileId,
                orm_track_assets::Column::StartMs,
                orm_track_assets::Column::EndMs,
            ])
            .values_panic([
                Expr::val(track.to_string()),
                Expr::val(file),
                Expr::val(start),
                Expr::val(end),
            ])
            .on_conflict(
                OnConflict::columns([
                    orm_track_assets::Column::TrackId,
                    orm_track_assets::Column::FileId,
                    orm_track_assets::Column::StartMs,
                ])
                .update_column(orm_track_assets::Column::EndMs)
                .to_owned(),
            ),
    )
    .await?;
    Ok(())
}

pub(super) async fn write_layout(
    tx: &DatabaseTransaction,
    file: i32,
    audio_id: Uuid,
    disc: Uuid,
    duration: Option<u64>,
    segments: &[TrackSegment],
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
        .query_all(
            Query::select()
                .columns([
                    orm_track_assets::Column::TrackId,
                    orm_track_assets::Column::StartMs,
                ])
                .from(orm_track_assets::Entity)
                .and_where(Expr::col(orm_track_assets::Column::FileId).eq(Expr::val(file))),
        )
        .await?;
    tx.execute(
        Query::delete()
            .from_table(orm_audio_layouts::Entity)
            .and_where(Expr::col(orm_audio_layouts::Column::FileId).eq(Expr::val(file))),
    )
    .await?;
    tx.execute(
        Query::insert()
            .into_table(orm_dirty_tracks::Entity)
            .columns([orm_dirty_tracks::Column::DiscId])
            .values_panic([Expr::val(disc.to_string())])
            .on_conflict(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;

    // Segment identity is independent of its mutable title and range.
    let mut ids = BTreeMap::new();
    if !segments.is_empty() {
        // A disc image is independent of the recordings represented by its segments.
        use orm_audio::Column as A;
        let recording = super::recording_identity::audio_fallback(audio_id);
        super::recording_identity::ensure_exists(tx, recording).await?;
        tx.execute(
            Query::update()
                .table(orm_audio::Entity)
                .value(A::RecordingId, Expr::val(recording.to_string()))
                .and_where(A::FileId.eq(file))
                .and_where(A::RecordingMbid.is_null())
                .and_where(A::RecordingIsrc.is_null()),
        )
        .await?;
        tx.execute(
            Query::insert()
                .into_table(orm_audio_layouts::Entity)
                .columns([orm_audio_layouts::Column::FileId])
                .values_panic([Expr::val(file)]),
        )
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
                track_fallback(id),
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
            tx.execute(
                Query::delete()
                    .from_table(orm_track_assets::Entity)
                    .and_where(
                        Expr::col(orm_track_assets::Column::TrackId)
                            .eq(Expr::val(track.to_string())),
                    )
                    .and_where(Expr::col(orm_track_assets::Column::FileId).eq(Expr::val(file)))
                    .and_where(Expr::col(orm_track_assets::Column::StartMs).eq(Expr::val(start))),
            )
            .await?;
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
    use orm_release_tracks::Column as T;
    use orm_track_assets::Column as V;
    let rows = tx
        .query_all(
            Query::select()
                .columns([T::TrackNumber, T::Title].map(|c| c.as_column_ref()))
                .columns([V::StartMs, V::EndMs].map(|c| c.as_column_ref()))
                .from(orm_track_assets::Entity)
                .inner_join(
                    orm_release_tracks::Entity,
                    column(T::Id).eq(column(V::TrackId)),
                )
                .and_where(V::FileId.eq(file))
                .order_by(T::TrackNumber.as_column_ref(), Order::Asc),
        )
        .await?;
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
        .query_all(
            Query::select()
                .column(orm_release_tracks::Column::Id)
                .from(orm_release_tracks::Entity)
                .and_where(
                    Expr::col(orm_release_tracks::Column::DiscId).eq(Expr::val(disc.to_string())),
                )
                .order_by_expr(
                    Expr::col(orm_release_tracks::Column::TrackNumber).is_null(),
                    Order::Asc,
                )
                .order_by(orm_release_tracks::Column::TrackNumber, Order::Asc)
                .order_by(orm_release_tracks::Column::Id, Order::Asc),
        )
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
    use orm_release_tracks::Column as T;
    use orm_track_assets::Column as V;
    let Some(row) = tx
        .query_one(
            Query::select()
                .columns([T::DiscId, T::TrackNumber, T::Title].map(|c| c.as_column_ref()))
                .column((Alias::new(TRACK_RECORDINGS), Alias::new("recording_id")))
                .from(orm_release_tracks::Entity)
                .inner_join(
                    track_recordings(),
                    field(TRACK_RECORDINGS, "track_id").eq(column(T::Id)),
                )
                .and_where(T::Id.eq(&key)),
        )
        .await?
    else {
        return Ok(None);
    };

    // Resolve only verified audio linked to this release position.
    let mut query = Query::select();
    query
        .expr_as(column(verified::Column::Id), "audio_id")
        .columns([V::StartMs, V::EndMs].map(|c| c.as_column_ref()));
    local_quality(&mut query);
    query
        .from(orm_track_assets::Entity)
        .inner_join(
            verified_audio(),
            column(verified::Column::FileId).eq(column(V::FileId)),
        )
        .inner_join(
            media_files::Entity,
            column(media_files::Column::Id).eq(column(V::FileId)),
        )
        .and_where(V::TrackId.eq(key))
        .order_by(verified::Column::Id.as_column_ref(), Order::Asc);
    let rows = tx.query_all(&query).await?;
    let mut variants = rows.iter().map(local_variant).collect::<Result<Vec<_>>>()?;
    rank(&mut variants);
    let result = ReleaseTrackMetadata {
        id,
        disc_id: uuid(&row, "disc_id")?,
        recording_id: uuid(&row, "recording_id")?,
        track_number: number(&row, "track_number")?,
        title: row.try_get("", "title")?,
        variants,
        provenance: None,
    };
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
