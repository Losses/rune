//! Apply sidecar CUE layouts to already indexed audio images.

use crate::actions::resolved_metadata::queries::verified_audio;
use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use fsio::{FsIo, OpenMode};
use metadata::model::TrackSegment;
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};

use crate::entities::{
    media_files,
    music_metadata::{audio as verified, audio_layouts as layouts},
};
use sea_orm::{
    ColumnTrait, ExprTrait,
    sea_query::{Expr, Query},
};

use super::{column, tracks, uuid};

#[derive(Debug, PartialEq, Eq)]
struct CueTrack {
    number: u32,
    title: Option<String>,
    index: Option<u64>,
}

type Images = BTreeMap<String, Vec<CueTrack>>;

fn parse_sheet(text: &str) -> Result<Images> {
    let mut images = Images::new();
    let mut file = None;
    let mut audio_track = false;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let mut line = line.trim().splitn(2, char::is_whitespace);
        let word = line.next().unwrap_or_default();
        let args = line.next().unwrap_or_default().trim();

        // A sidecar can reference files in its own directory only.
        if word.eq_ignore_ascii_case("FILE") {
            let name = argument(args).context("missing CUE filename")?;
            ensure!(
                !name.is_empty()
                    && !name.contains(['/', '\\', ':', '\0'])
                    && !matches!(name, "." | ".."),
                "CUE file must name a sibling audio file"
            );
            file = Some(name.to_owned());
            audio_track = false;
        } else if word.eq_ignore_ascii_case("TRACK") {
            let mut parts = args.split_whitespace();
            let number = parts
                .next()
                .context("missing CUE track number")?
                .parse::<u32>()?;
            audio_track = parts
                .next()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("AUDIO"));
            if audio_track {
                let image = images
                    .entry(file.clone().context("CUE track without FILE")?)
                    .or_default();
                ensure!(
                    number > 0 && image.last().is_none_or(|track| track.number < number),
                    "invalid CUE track order"
                );
                image.push(CueTrack {
                    number,
                    title: None,
                    index: None,
                });
            }
        } else if audio_track {
            let track = file
                .as_ref()
                .and_then(|file| images.get_mut(file))
                .and_then(|image| image.last_mut())
                .context("missing CUE track")?;
            read_track_line(track, word, args)?;
        }
    }
    ensure!(
        images.values().flatten().all(|track| track.index.is_some()),
        "CUE audio track has no INDEX 01"
    );
    Ok(images)
}

fn read_track_line(track: &mut CueTrack, word: &str, args: &str) -> Result<()> {
    if word.eq_ignore_ascii_case("TITLE") {
        let title = if args.starts_with('"') {
            argument(args)
        } else {
            Some(args)
        };
        track.title = title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    } else if word.eq_ignore_ascii_case("INDEX") {
        let mut parts = args.split_whitespace();
        if parts.next() == Some("01") {
            ensure!(track.index.is_none(), "duplicate CUE INDEX 01");
            track.index = Some(index_ms(
                parts.next().context("missing CUE index position")?,
            )?);
        }
    }
    Ok(())
}

fn argument(args: &str) -> Option<&str> {
    if let Some(quoted) = args.strip_prefix('"') {
        return quoted.split_once('"').map(|(value, _)| value);
    }
    args.split_whitespace().next()
}

fn index_ms(position: &str) -> Result<u64> {
    let mut fields = position.split(':');
    let minutes = fields
        .next()
        .context("missing CUE minutes")?
        .parse::<u32>()?;
    let seconds = fields
        .next()
        .context("missing CUE seconds")?
        .parse::<u32>()?;
    let frames = fields
        .next()
        .context("missing CUE frames")?
        .parse::<u32>()?;
    ensure!(
        seconds < 60 && frames < 75 && fields.next().is_none(),
        "invalid CUE position"
    );
    Ok((u64::from(minutes) * 60 + u64::from(seconds)) * 1000 + u64::from(frames) * 1000 / 75)
}

fn segments(tracks: &[CueTrack], duration: u64) -> Result<Vec<TrackSegment>> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let start = track.index.context("missing CUE start")?;
            let end = tracks
                .get(index + 1)
                .and_then(|next| next.index)
                .unwrap_or(duration);
            ensure!(
                start < end && end <= duration,
                "CUE index exceeds the audio image bounds"
            );
            Ok(TrackSegment {
                track_number: track.number,
                title: track
                    .title
                    .clone()
                    .unwrap_or_else(|| format!("Track {}", track.number)),
                start_ms: start,
                end_ms: end,
            })
        })
        .collect()
}

fn read_sheets(fs: &FsIo, paths: Vec<PathBuf>) -> Result<Images> {
    const MAX_SHEET_BYTES: u64 = 1024 * 1024;
    let mut images = Images::new();
    for path in paths {
        let mut bytes = Vec::new();
        fs.open(&path, OpenMode::Read)?
            .take(MAX_SHEET_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SHEET_BYTES,
            "CUE sheet is too large"
        );
        let text = decode_sheet(bytes)?;

        // Two different sheets for the same image require an explicit layout choice.
        for (name, tracks) in
            parse_sheet(&text).with_context(|| format!("invalid CUE sheet {path:?}"))?
        {
            if let Some(previous) = images.get(&name) {
                ensure!(previous == &tracks, "conflicting CUE layouts for {name}");
            } else {
                images.insert(name, tracks);
            }
        }
    }
    Ok(images)
}

fn decode_sheet(bytes: Vec<u8>) -> Result<String> {
    match bytes.as_slice() {
        [first @ (0xff | 0xfe), second, rest @ ..]
            if *first != *second && matches!(*second, 0xff | 0xfe) =>
        {
            let (pairs, remainder) = rest.as_chunks::<2>();
            ensure!(remainder.is_empty(), "truncated UTF-16 CUE sheet");
            let units = pairs
                .iter()
                .map(|&pair| {
                    if *first == 0xff {
                        u16::from_le_bytes(pair)
                    } else {
                        u16::from_be_bytes(pair)
                    }
                })
                .collect::<Vec<_>>();
            Ok(String::from_utf16(&units)?)
        }
        _ => Ok(String::from_utf8(bytes)?),
    }
}

pub(in crate::actions) async fn reconcile_scan(
    fs: &FsIo,
    db: &DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    root: &Path,
    dirs: &HashSet<String>,
    paths: Vec<PathBuf>,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let mut directories: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        let Some(parent) = path
            .parent()
            .and_then(|path| path.strip_prefix(root).ok())
            .and_then(Path::to_str)
        else {
            continue;
        };
        if dirs.contains(parent) {
            directories.entry(parent.to_owned()).or_default().push(path);
        }
    }

    // Include previously indexed sidecars that disappeared during a complete directory walk.
    let existing = db
        .query_all(
            Query::select()
                .column(media_files::Column::Directory.as_column_ref())
                .from(layouts::Entity)
                .inner_join(
                    media_files::Entity,
                    column(media_files::Column::Id).eq(column(layouts::Column::FileId)),
                )
                .distinct(),
        )
        .await?;
    for row in existing {
        let directory: String = row.try_get("", "directory")?;
        if dirs.contains(&directory) {
            directories.entry(directory).or_default();
        }
    }
    for (directory, paths) in directories {
        if cancellation.is_some_and(tokio_util::sync::CancellationToken::is_cancelled) {
            break;
        }
        if let Err(error) = reconcile_directory(fs, db, store, &directory, paths).await {
            log::warn!("Cannot index CUE layouts in {directory:?}: {error:#}");
        }
    }
    Ok(())
}

pub(in crate::actions) async fn reconcile_directory(
    fs: &FsIo,
    db: &DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    directory: &str,
    paths: Vec<PathBuf>,
) -> Result<()> {
    if paths.is_empty()
        && db
            .query_one(
                Query::select()
                    .expr(Expr::val(1))
                    .from(media_files::Entity)
                    .inner_join(
                        layouts::Entity,
                        column(layouts::Column::FileId).eq(column(media_files::Column::Id)),
                    )
                    .and_where(media_files::Column::Directory.eq(directory))
                    .limit(1),
            )
            .await?
            .is_none()
    {
        return Ok(());
    }
    let fs = fs.clone();
    let images = tokio::task::spawn_blocking(move || read_sheets(&fs, paths)).await??;

    // Read file durations and layout ownership after acquiring the scan's write lock.
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let files = tx
        .query_all(
            Query::select()
                .columns(
                    [
                        verified::Column::FileId,
                        verified::Column::Id,
                        verified::Column::DiscId,
                        verified::Column::DurationMs,
                    ]
                    .map(|c| c.as_column_ref()),
                )
                .column(media_files::Column::FileName.as_column_ref())
                .expr_as(column(layouts::Column::FileId).is_not_null(), "has_cue")
                .from(media_files::Entity)
                .inner_join(
                    verified_audio(),
                    column(verified::Column::FileId).eq(column(media_files::Column::Id)),
                )
                .left_join(
                    layouts::Entity,
                    column(layouts::Column::FileId).eq(column(verified::Column::FileId)),
                )
                .and_where(media_files::Column::Directory.eq(directory))
                .and_where(media_files::Column::LocalAvailable.eq(true)),
        )
        .await?;
    let mut names = BTreeMap::new();
    for row in &files {
        let name: String = row.try_get("", "file_name")?;
        *names.entry(name.to_ascii_lowercase()).or_insert(0u32) += 1;
    }
    for row in files {
        let file = row.try_get::<i32>("", "file_id")?;
        let name = row.try_get::<String>("", "file_name")?;
        let duration = row
            .try_get::<Option<i64>>("", "duration_ms")?
            .map(u64::try_from)
            .transpose()?;
        let has_cue: bool = row.try_get("", "has_cue")?;
        // Case-fold only if neither the sidecars nor the audio directory are ambiguous.
        let match_name = images.get(&name).or_else(|| {
            if names.get(&name.to_ascii_lowercase()) != Some(&1) {
                return None;
            }
            let mut matches = images
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(&name));
            let (_, first) = matches.next()?;
            matches.next().is_none().then_some(first)
        });
        let layout = match (match_name, duration) {
            (Some(tracks), Some(duration)) => segments(tracks, duration)?,
            (Some(_), None) => continue,
            (None, _) if !has_cue => continue,
            (None, _) => Vec::new(),
        };
        if !has_cue || !tracks::cue_layout_matches(&tx, file, &layout).await? {
            tracks::write_layout(
                &tx,
                file,
                uuid(&row, "id")?,
                uuid(&row, "disc_id")?,
                duration,
                &layout,
            )
            .await?;
        }
    }
    tracks::reconcile(&tx).await?;
    tx.commit().await?;
    Ok(())
}
