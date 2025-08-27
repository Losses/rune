use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use ::playback::sfx_player::SfxPlayer;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for SfxPlayRequest {
    type Params = (Arc<Mutex<SfxPlayer>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.sfx_player),)
    }
}

impl Signal for SfxPlayRequest {
    type Params = (Arc<Mutex<SfxPlayer>>,);
    type Response = ();

    async fn handle(
        &self,
        (sfx_player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        sfx_player
            .lock()
            .await
            .load(dart_signal.path.clone().into());
        Ok(Some(()))
    }
}
