use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::fsio::FsIo;
use ::playback::player::Playable;
use ::visualizer::{PcmInput, Transport};

use crate::{
    Session, Signal, flutter_bridge,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    visualizer::{VISUALIZER, provide_covers},
};

/// Errors of the visualizer controls go to the status (`last_error`) instead of crashing the
/// request loop, since a missing preset directory or GPU is an ordinary condition.
fn report(result: Result<()>) -> Result<Option<()>> {
    if let Err(e) = result {
        VISUALIZER.report_error(format!("{e:#}"));
    }
    Ok(Some(()))
}

impl ParamsExtractor for StartVisualizerRequest {
    type Params = (
        Arc<Mutex<dyn Playable>>,
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.player),
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
        )
    }
}

impl Signal for StartVisualizerRequest {
    type Params = (
        Arc<Mutex<dyn Playable>>,
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
    );
    type Response = ();

    async fn handle(
        &self,
        (player, fsio, main_db, recommend_db): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        provide_covers(fsio, main_db, recommend_db);
        let tap = player.lock().await.pcm_tap();
        // A restart: take the audio back from the running visualizer first.
        if let Some(input) = VISUALIZER.stop() {
            tap.restore_consumer(input.consumer);
        }
        let Some(consumer) = tap.take_consumer() else {
            return report(Err(anyhow::anyhow!("The PCM tap is in use")));
        };
        tap.set_enabled(true);
        let input = PcmInput {
            consumer,
            channels: tap.channels(),
            sample_rate: tap.sample_rate(),
        };
        flutter_bridge::prepare_visualizer(dart_signal.width, dart_signal.height).await;
        let result = VISUALIZER.start(
            input,
            dart_signal.width,
            dart_signal.height,
            dart_signal.fps,
        );
        if result.is_err() {
            tap.set_enabled(false);
            return report(result);
        }
        flutter_bridge::visualizer_started().await;
        Ok(Some(()))
    }
}

impl ParamsExtractor for StopVisualizerRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for StopVisualizerRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let tap = player.lock().await.pcm_tap();
        tap.set_enabled(false);
        if let Some(input) = flutter_bridge::stop_visualizer().await {
            tap.restore_consumer(input.consumer);
        }
        Ok(Some(()))
    }
}

impl ParamsExtractor for ResizeVisualizerRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for ResizeVisualizerRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(flutter_bridge::resize_visualizer(dart_signal.width, dart_signal.height).await)
    }
}

impl ParamsExtractor for SetVisualizerPresetDirsRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerPresetDirsRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let dirs: Vec<PathBuf> = dart_signal.paths.iter().map(PathBuf::from).collect();
        report(VISUALIZER.set_preset_dirs(&dirs).map(|_| ()))
    }
}

impl ParamsExtractor for SetVisualizerAccentRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerAccentRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        VISUALIZER.set_accent(dart_signal.argb);
        Ok(Some(()))
    }
}

impl ParamsExtractor for SetVisualizerVisibleRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerVisibleRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        VISUALIZER.set_visible(dart_signal.visible);
        Ok(Some(()))
    }
}

impl ParamsExtractor for LoadVisualizerPresetRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for LoadVisualizerPresetRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.load_preset_path(Path::new(&dart_signal.path)))
    }
}

impl ParamsExtractor for NextVisualizerPresetRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for NextVisualizerPresetRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.next_preset())
    }
}

impl ParamsExtractor for PreviousVisualizerPresetRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for PreviousVisualizerPresetRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.previous_preset())
    }
}

impl ParamsExtractor for RandomVisualizerPresetRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for RandomVisualizerPresetRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.random_preset())
    }
}

impl ParamsExtractor for SetVisualizerAutoSwitchPausedRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerAutoSwitchPausedRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        VISUALIZER.set_auto_switch_paused(dart_signal.paused);
        Ok(Some(()))
    }
}

impl ParamsExtractor for VisualizerStatusRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for VisualizerStatusRequest {
    type Params = ();
    type Response = VisualizerStatusResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let status = VISUALIZER.status();
        Ok(Some(VisualizerStatusResponse {
            running: status.running,
            width: status.width,
            height: status.height,
            target_fps: status.target_fps,
            measured_fps: status.measured_fps,
            preset_name: status.preset_name,
            preset_count: u32::try_from(status.preset_count).unwrap_or(u32::MAX),
            adapter: status.adapter,
            transport: match status.transport {
                Some(Transport::IoSurface) => "iosurface",
                Some(Transport::Readback) => "readback",
                Some(Transport::DxgiShared) => "dxgi-shared",
                Some(Transport::VulkanShared) => "vulkan-shared",
                Some(Transport::Surface) => "surface",
                None => "",
            }
            .to_string(),
            last_error: status.last_error,
        }))
    }
}

impl ParamsExtractor for VisualizerTextureShownRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for VisualizerTextureShownRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        flutter_bridge::texture_shown(dart_signal.texture_id).await;
        Ok(Some(()))
    }
}

impl ParamsExtractor for ListVisualizerPresetsRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for ListVisualizerPresetsRequest {
    type Params = ();
    type Response = ListVisualizerPresetsResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let presets = VISUALIZER
            .list_presets()
            .into_iter()
            .map(|info| VisualizerPresetEntry {
                name: info.name,
                path: info.path.to_string_lossy().into_owned(),
                group: info.group,
            })
            .collect();
        Ok(Some(ListVisualizerPresetsResponse { presets }))
    }
}
