use std::sync::Arc;

use anyhow::Result;
use log::{error, warn};

use ::database::{
    connection::MainDbConnection, playing_item::dispatcher::PlayingItemActionDispatcher,
};
use ::fsio::FsIo;
use ::lyric::{
    lrc::parse_lrc, parser::parse_audio_lyrics, srt::parse_srt, ttml::parse_ttml, types::LyricFile,
    vtt::parse_vtt,
};
use ::metadata::reader::get_lyrics;
use ::playback::player::PlayingItem;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for GetLyricByTrackIdRequest {
    type Params = (Arc<FsIo>, Arc<String>, Arc<MainDbConnection>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.lib_path),
            Arc::clone(&all_params.main_db),
        )
    }
}

impl Signal for GetLyricByTrackIdRequest {
    type Params = (Arc<FsIo>, Arc<String>, Arc<MainDbConnection>);
    type Response = GetLyricByTrackIdResponse;

    async fn handle(
        &self,
        (fsio, lib_path, main_db): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let item = &dart_signal.item;

        let Some(item) = item else {
            return Ok(None);
        };

        let lines = match load_lyric_lines(&fsio, &lib_path, &main_db, item).await {
            Ok(lines) => lines,
            Err(err) => {
                error!("Unable to load lyric: item={item:#?}: {err:#}");
                Vec::new()
            }
        };

        Ok(Some(GetLyricByTrackIdResponse {
            item: item.clone(),
            lines,
        }))
    }
}

async fn load_lyric_lines(
    fsio: &Arc<FsIo>,
    lib_path: &Arc<String>,
    main_db: &Arc<MainDbConnection>,
    item: &PlayingItemRequest,
) -> Result<Vec<LyricContentLine>> {
    let parsed_item: PlayingItem = item.clone().into();
    let dispatcher = PlayingItemActionDispatcher::new();

    let paths = dispatcher
        .get_file_path(
            fsio,
            lib_path.as_ref(),
            main_db,
            [parsed_item.clone()].as_ref(),
        )
        .await?;

    let Some(path) = paths.get(&parsed_item) else {
        return Ok(Vec::new());
    };

    let embedded = match get_lyrics(fsio, path) {
        Ok(x) => x.filter(|x| !x.trim().is_empty()),
        Err(err) => {
            error!("Unable to read embedded lyric: path={path:?}: {err:#}");
            None
        }
    };

    let mut lyric = None;

    if let Some(content) = embedded {
        match parse_embedded_lyric(&content) {
            Ok(file) if !file.lyrics.is_empty() => lyric = Some(file),
            Ok(_) => warn!("Embedded lyric has no timed lines, falling back: path={path:?}"),
            Err(err) => {
                warn!("Unable to parse embedded lyric, falling back: path={path:?}: {err:#}")
            }
        }
    }

    let lyric = match lyric {
        Some(x) => x,
        None => match parse_audio_lyrics(path.to_path_buf()) {
            Some(Ok(x)) => x,
            Some(Err(err)) => return Err(err.context("Unable to parse sidecar lyric")),
            None => return Ok(Vec::new()),
        },
    };

    Ok(content_lines(lyric))
}

fn content_lines(lyric: LyricFile) -> Vec<LyricContentLine> {
    lyric
        .lyrics
        .into_iter()
        .map(|x| {
            // A line without word timing is one section as long as the line
            let word_time_tags = if x.word_time_tags.is_empty() && !x.text.is_empty() {
                vec![(x.start_time.clone(), x.end_time.clone(), x.text)]
            } else {
                x.word_time_tags
            };

            LyricContentLine {
                start_time: x.start_time.into(),
                end_time: x.end_time.into(),
                sections: word_time_tags
                    .into_iter()
                    .map(|tag| LyricContentLineSection {
                        start_time: tag.0.into(),
                        end_time: tag.1.into(),
                        content: tag.2,
                    })
                    .collect(),
            }
        })
        .collect()
}

fn parse_embedded_lyric(content: &str) -> Result<LyricFile> {
    let content = content.trim_start_matches('\u{feff}');
    let trimmed = content.trim_start();

    // LRC lines start with `[`, so anything starting with markup is TTML
    if trimmed.starts_with('<') {
        parse_ttml(content)
    } else if trimmed.starts_with("WEBVTT") {
        parse_vtt(trimmed)
    } else if looks_like_srt(trimmed) {
        parse_srt(trimmed)
    } else {
        parse_lrc(content)
    }
}

fn looks_like_srt(content: &str) -> bool {
    let mut lines = content.lines();
    let first = lines.next().map(str::trim).unwrap_or_default();
    let second = lines.next().map(str::trim).unwrap_or_default();
    !first.is_empty()
        && first.chars().all(|c| c.is_ascii_digit())
        && second.contains("-->")
        && second.contains(',')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_untimed_words_become_one_section() {
        let lyric = parse_ttml(
            r#"<tt><body><div><p begin="00:01.000" end="00:02.000">Hello</p></div></body></tt>"#,
        )
        .unwrap();
        let lines = content_lines(lyric);
        assert_eq!(lines.len(), 1);
        let sections = &lines[0].sections;
        assert_eq!(sections.len(), 1);
        assert_eq!(
            (
                sections[0].start_time,
                sections[0].end_time,
                sections[0].content.as_str()
            ),
            (1000, 2000, "Hello")
        );
    }
}
