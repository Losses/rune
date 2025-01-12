use std::{path::Path, sync::Arc};

use anyhow::Result;

use ::database::{
    connection::MainDbConnection, playing_item::dispatcher::PlayingItemActionDispatcher,
};
use ::lyric::parser::parse_audio_lyrics;
use ::playback::player::PlayingItem;

use crate::{
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    Signal,
};

impl ParamsExtractor for GetLyricByTrackIdRequest {
    type Params = (Arc<String>, Arc<MainDbConnection>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.lib_path),
            Arc::clone(&all_params.main_db),
        )
    }
}

impl Signal for GetLyricByTrackIdRequest {
    type Params = (Arc<String>, Arc<MainDbConnection>);
    type Response = GetLyricByTrackIdResponse;

    async fn handle(
        &self,
        (lib_path, main_db): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let item = &dart_signal.item;

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
                                item: Some(item.clone()),
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
                            item: Some(item.clone()),
                            lines: [].to_vec(),
                        })),
                    }
                }
                None => Ok(Some(GetLyricByTrackIdResponse {
                    item: Some(item.clone()),
                    lines: [].to_vec(),
                })),
            }
        } else {
            Ok(None)
        }
    }
}
