use std::{path::Path, sync::Arc};

use anyhow::{bail, Context, Result};
use dunce::canonicalize;
use lyric::parser::parse_audio_lyrics;
use rinf::DartSignal;

use database::{actions::file::get_ordered_files_by_ids, connection::MainDbConnection};

use crate::{GetLyricByTrackIdRequest, GetLyricByTrackIdResponse, LyricContentLine, LyricContentLineSection};

pub async fn get_lyric_by_track_id_request(
    lib_path: Arc<String>,
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetLyricByTrackIdRequest>,
) -> Result<()> {
    let track_id = dart_signal.message.id;

    let media_file = get_ordered_files_by_ids(&main_db, &[track_id])
        .await
        .with_context(|| format!("Unable to get media file by id={}", track_id))?;

    if media_file.is_empty() {
        bail!("No track found from the given track id");
    }

    let media_file = media_file[0].clone();

    let path = canonicalize(
        Path::new(lib_path.as_ref())
            .join(&media_file.directory)
            .join(&media_file.file_name),
    )?;

    let lyric = parse_audio_lyrics(path);

    match lyric {
        Some(lyric) => match lyric {
            Ok(lyric) => GetLyricByTrackIdResponse {
                id: track_id,
                lines: lyric
                    .lyrics
                    .into_iter()
                    .map(|x| LyricContentLine {
                        sections: x
                            .word_time_tags
                            .into_iter()
                            .map(|x| LyricContentLineSection {
                                start_time: x.0.into(),
                                end_time: x.1.into(),
                                content: x.2,
                            })
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart(),
            Err(err) => return Err(err.context(format!("Unable to parse lyric: id={}", track_id))),
        },
        None => GetLyricByTrackIdResponse {
            id: track_id,
            lines: [].to_vec(),
        }
        .send_signal_to_dart(),
    }

    Ok(())
}
