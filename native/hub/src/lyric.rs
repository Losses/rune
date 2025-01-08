use std::{path::Path, sync::Arc};

use anyhow::Result;
use lyric::parser::parse_audio_lyrics;
use playback::player::PlayingItem;
use rinf::DartSignal;

use database::{
    connection::MainDbConnection, playing_item::dispatcher::PlayingItemActionDispatcher,
};

use crate::{
    GetLyricByTrackIdRequest, GetLyricByTrackIdResponse, LyricContentLine, LyricContentLineSection,
};

pub async fn get_lyric_by_track_id_request(
    lib_path: Arc<String>,
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetLyricByTrackIdRequest>,
) -> Result<Option<GetLyricByTrackIdResponse>> {
    let item = dart_signal.message.item;

    if let Some(item) = item {
        let parsed_item: PlayingItem = item.clone().into();
        let dispatcher = PlayingItemActionDispatcher::new();

        let path = dispatcher
            .get_file_path(
                Path::new(lib_path.as_ref()),
                &main_db,
                [parsed_item.clone()].as_ref(),
            )
            .await?;

        let path = path.get(&parsed_item);

        match path {
            Some(path) => {
                let lyric = parse_audio_lyrics(path.to_path_buf());

                match lyric {
                    Some(lyric) => match lyric {
                        Ok(lyric) => Ok(Some(GetLyricByTrackIdResponse {
                            item: Some(item),
                            lines: lyric
                                .lyrics
                                .into_iter()
                                .map(|x| LyricContentLine {
                                    start_time: x.start_time.into(),
                                    end_time: x.end_time.into(),
                                    sections: x
                                        .word_time_tags
                                        .into_iter()
                                        .map(|tag| LyricContentLineSection {
                                            start_time: tag.0.into(),
                                            end_time: tag.1.into(),
                                            content: tag.2,
                                        })
                                        .collect(),
                                })
                                .collect(),
                        })),
                        Err(err) => {
                            Err(err.context(format!("Unable to parse lyric: item={:#?}", item)))
                        }
                    },
                    None => Ok(Some(GetLyricByTrackIdResponse {
                        item: Some(item),
                        lines: [].to_vec(),
                    })),
                }
            }
            None => Ok(Some(GetLyricByTrackIdResponse {
                item: Some(item),
                lines: [].to_vec(),
            })),
        }
    } else {
        Ok(None)
    }
}
