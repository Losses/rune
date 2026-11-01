use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::fsio::FsIo;
use ::playback::player::Playable;
use ::visualizer::{Location, PcmInput, Transport};

use crate::{
    Session, Signal, flutter_bridge,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    visualizer::{VISUALIZER, provide_covers},
};

/// Errors of the visualizer controls go to the status (`last_error`) and the Log page, under
/// `operation`, instead of crashing the request loop, since a missing scene or GPU is an ordinary
/// condition.
fn report(operation: &'static str, result: Result<()>) -> Result<Option<()>> {
    if let Err(e) = result {
        VISUALIZER.report_error(operation, format!("{e:#}"));
    }
    Ok(Some(()))
}

/// Held through a start, a stop and a resize, which are handled concurrently: a surface that
/// replaces another sends a stop and then a start, and the start must wait for the stop to give
/// the audio back; a resize that lands while a start prepares its window must wait for the start,
/// or it finds nothing to resize and is lost.
static STARTING_OR_STOPPING: Mutex<()> = Mutex::const_new(());

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
        let _turn = STARTING_OR_STOPPING.lock().await;
        crate::visualizer_log::write_to(Arc::clone(&main_db));
        provide_covers(fsio, main_db, recommend_db);
        let tap = player.lock().await.pcm_tap();
        // A restart: take the audio back from the running visualizer first.
        if let Some(input) = VISUALIZER.stop() {
            tap.restore_consumer(input.consumer);
        }
        let Some(consumer) = tap.take_consumer() else {
            return report("start", Err(anyhow::anyhow!("The PCM tap is in use")));
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
            return report("start", result);
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
        let _turn = STARTING_OR_STOPPING.lock().await;
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
        let _turn = STARTING_OR_STOPPING.lock().await;
        report(
            "resize",
            flutter_bridge::resize_visualizer(dart_signal.width, dart_signal.height).await,
        )
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
        report("scan_scenes", VISUALIZER.set_preset_dirs(&dirs).map(|_| ()))
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
        report(
            "load_scene",
            VISUALIZER.load_preset(Location::parse(&dart_signal.path)),
        )
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
        report("switch_scene", VISUALIZER.next_preset())
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
        report("switch_scene", VISUALIZER.previous_preset())
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
        report("switch_scene", VISUALIZER.random_preset())
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
            preset_path: status.preset_path,
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

impl ParamsExtractor for ListVisualizerCreditsRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for ListVisualizerCreditsRequest {
    type Params = ();
    type Response = ListVisualizerCreditsResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let credits = ::visualizer::builtin_credits()
            .into_iter()
            .map(|credit| VisualizerCredit {
                scene: credit.scene,
                assets: credit.assets,
            })
            .collect();
        Ok(Some(ListVisualizerCreditsResponse { credits }))
    }
}

impl ParamsExtractor for SetVisualizerScenesRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerScenesRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let parse = |paths: &[String]| paths.iter().map(|path| Location::parse(path)).collect();
        VISUALIZER.set_scene_choice(parse(&dart_signal.order), parse(&dart_signal.disabled));
        if let Some(last) = &dart_signal.last {
            VISUALIZER.resume_from(Location::parse(last));
        }
        Ok(Some(()))
    }
}

impl ParamsExtractor for ImportVisualizerSceneRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for ImportVisualizerSceneRequest {
    type Params = ();
    type Response = ImportVisualizerSceneResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let file = PathBuf::from(&dart_signal.file);
        let dir = PathBuf::from(&dart_signal.dir);
        let imported = {
            let (file, dir) = (file.clone(), dir.clone());
            tokio::task::spawn_blocking(move || ::visualizer::package::import(&file, &dir)).await?
        };
        let (path, error) = match imported {
            Ok(path) => {
                let imported = format!("Imported {} into {}", file.display(), path.display());
                log::info!("{imported}");
                crate::visualizer_log::diagnostic("import_scene", imported);
                rescan(&dir);
                (Some(path.to_string_lossy().into_owned()), None)
            }
            Err(e) => {
                log::error!("Failed to import the scene {}: {e:#}", file.display());
                crate::visualizer_log::error("import_scene", format!("{}: {e:#}", file.display()));
                (None, Some(format!("{e:#}")))
            }
        };
        Ok(Some(ImportVisualizerSceneResponse {
            file: dart_signal.file.clone(),
            path,
            error,
        }))
    }
}

impl ParamsExtractor for RemoveVisualizerSceneRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for RemoveVisualizerSceneRequest {
    type Params = ();
    type Response = RemoveVisualizerSceneResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let path = PathBuf::from(&dart_signal.path);
        let dir = PathBuf::from(&dart_signal.dir);
        let removed = {
            let (path, dir) = (path.clone(), dir.clone());
            tokio::task::spawn_blocking(move || ::visualizer::package::remove(&path, &dir)).await?
        };
        let error = match removed {
            Ok(()) => {
                log::info!("Deleted the scene {}", path.display());
                rescan(&dir);
                None
            }
            Err(e) => {
                log::error!("Failed to delete the scene {}: {e:#}", path.display());
                crate::visualizer_log::error("remove_scene", format!("{}: {e:#}", path.display()));
                Some(format!("{e:#}"))
            }
        };
        Ok(Some(RemoveVisualizerSceneResponse {
            path: dart_signal.path.clone(),
            error,
        }))
    }
}

/// List the scenes of `dir` again after a scene came or went.
fn rescan(dir: &std::path::Path) {
    if let Err(e) = VISUALIZER.set_preset_dirs(&[dir.to_path_buf()]) {
        VISUALIZER.report_error("scan_scenes", format!("{e:#}"));
    }
}

impl ParamsExtractor for SetVisualizerLogRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for SetVisualizerLogRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = ();

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        crate::visualizer_log::set_diagnostics(dart_signal.enabled);
        crate::visualizer_log::write_to(main_db);
        Ok(Some(()))
    }
}

/// What [`visualizer::probe_gpu`] found, once: the GPU does not change while the app runs.
static GPU: tokio::sync::OnceCell<std::result::Result<String, String>> =
    tokio::sync::OnceCell::const_new();

impl ParamsExtractor for VisualizerSupportRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for VisualizerSupportRequest {
    type Params = ();
    type Response = VisualizerSupportResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let gpu = GPU
            .get_or_init(|| async {
                let probed = tokio::task::spawn_blocking(::visualizer::probe_gpu).await;
                let gpu = match probed {
                    Ok(Ok(adapter)) => Ok(adapter),
                    Ok(Err(e)) => Err(format!("{e:#}")),
                    Err(e) => Err(format!("The GPU probe failed: {e}")),
                };
                match &gpu {
                    Ok(adapter) => {
                        log::info!("The visualizer can run on {adapter}");
                        crate::visualizer_log::diagnostic(
                            "probe_gpu",
                            format!("The visualizer can run on {adapter}"),
                        );
                    }
                    Err(problem) => {
                        log::error!("The visualizer cannot run: {problem}");
                        crate::visualizer_log::error("probe_gpu", problem.clone());
                    }
                }
                gpu
            })
            .await;
        Ok(Some(match gpu {
            Ok(adapter) => VisualizerSupportResponse {
                supported: true,
                adapter: adapter.clone(),
                problem: None,
            },
            Err(problem) => VisualizerSupportResponse {
                supported: false,
                adapter: String::new(),
                problem: Some(problem.clone()),
            },
        }))
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
                path: info.location.to_string(),
                group: info.group,
                builtin: matches!(info.location, Location::Builtin(_)),
            })
            .collect();
        Ok(Some(ListVisualizerPresetsResponse { presets }))
    }
}
