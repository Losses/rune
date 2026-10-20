use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use ::playback::player::Playable;
use ::visualizer::{PcmInput, Source, Transition, Transport};

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    visualizer::VISUALIZER,
};

/// Errors of the visualizer controls go to the status (`last_error`) instead of crashing the
/// request loop, since a missing preset directory or GPU is an ordinary condition.
fn report(result: Result<()>) -> Result<Option<()>> {
    if let Err(e) = result {
        VISUALIZER.report_error(format!("{e:#}"));
    }
    Ok(Some(()))
}

fn transition(seconds: f64) -> Transition {
    if seconds > 0.0 {
        Transition::Soft { seconds }
    } else {
        Transition::Cut
    }
}

impl ParamsExtractor for StartVisualizerRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for StartVisualizerRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
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
        let result = VISUALIZER.start(
            input,
            dart_signal.width,
            dart_signal.height,
            dart_signal.fps,
        );
        if result.is_err() {
            tap.set_enabled(false);
        }
        report(result)
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
        if let Some(input) = VISUALIZER.stop() {
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
        report(VISUALIZER.resize(dart_signal.width, dart_signal.height))
    }
}

impl ParamsExtractor for SetVisualizerPresetDirRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerPresetDirRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(
            VISUALIZER
                .set_preset_dir(Path::new(&dart_signal.path))
                .map(|_| ()),
        )
    }
}

impl ParamsExtractor for SetVisualizerSourceRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerSourceRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let source = match dart_signal.source {
            VisualizerSource::None => return Ok(Some(())),
            VisualizerSource::Builtin => Source::Builtin,
            VisualizerSource::Folder => Source::Folder,
        };
        report(VISUALIZER.set_source(source))
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
        report(VISUALIZER.load_preset_path(
            Path::new(&dart_signal.path),
            transition(dart_signal.transition_seconds),
        ))
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
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.next_preset(transition(dart_signal.transition_seconds)))
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
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.previous_preset(transition(dart_signal.transition_seconds)))
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
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        report(VISUALIZER.random_preset(transition(dart_signal.transition_seconds)))
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
            source: match status.source {
                Source::None => VisualizerSource::None,
                Source::Builtin => VisualizerSource::Builtin,
                Source::Folder => VisualizerSource::Folder,
            },
            builtin_available: status.builtin_available,
            adapter: status.adapter,
            transport: match status.transport {
                Some(Transport::IoSurface) => "iosurface",
                Some(Transport::Readback) => "readback",
                None => "",
            }
            .to_string(),
            last_error: status.last_error,
        }))
    }
}
