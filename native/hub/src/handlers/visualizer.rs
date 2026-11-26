use crate::node::NodeContext;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;
use uuid::Uuid;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use ::playback::player::Playable;
use ::visualizer::{
    Inspect, LabCommand, LabEvent, LabEventKind, LabTransition, Level, Location, PcmInput,
    SceneInputs,
};

use crate::{
    Session, Signal, flutter_bridge,
    messages::*,
    request_error::ExpectedRequestError,
    utils::{Broadcaster, GlobalParams, LocalLibrary, ParamsExtractor},
    visualizer::provide_covers,
};
use std::path as std_path;
use std::result;
use tokio::sync as tokio_sync;
use tokio::task;
use visualizer::package;

fn message_runtime_status(status: &::visualizer::Status) -> VisualizerRuntimeStatus {
    match status.transport {
        Some(transport) if status.running => VisualizerRuntimeStatus::Running {
            adapter: status.adapter.clone(),
            transport: transport.into(),
        },
        Some(transport) => VisualizerRuntimeStatus::Stopped {
            adapter: status.adapter.clone(),
            transport: transport.into(),
        },
        None => VisualizerRuntimeStatus::NotRunningYet,
    }
}

fn message_support(result: &Result<String, String>) -> VisualizerSupport {
    match result {
        Ok(adapter) => VisualizerSupport::Supported {
            adapter: adapter.clone(),
        },
        Err(problem) => VisualizerSupport::Unsupported {
            problem: problem.clone(),
        },
    }
}

/// Errors of the visualizer controls go to the status (`last_error`) and the Log page, under
/// `operation`, instead of crashing the request loop, since a missing scene or GPU is an ordinary
/// condition.
fn report(
    operation: &'static str,
    result: Result<()>,
    visualizer: &::visualizer::Visualizer,
) -> Result<Option<()>> {
    if let Err(e) = result {
        visualizer.report_error(operation, format!("{e:#}"));
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
        LocalLibrary,
        Option<Uuid>,
        Arc<NodeContext>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.player),
            LocalLibrary::from_global(all_params),
            all_params.library_id,
            all_params.node.clone(),
        )
    }
}

impl Signal for StartVisualizerRequest {
    type Params = (
        Arc<Mutex<dyn Playable>>,
        LocalLibrary,
        Option<Uuid>,
        Arc<NodeContext>,
    );
    type Response = ();

    async fn handle(
        &self,
        (player, library, library_id, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = library.or_service()?;
        let _turn = STARTING_OR_STOPPING.lock().await;
        let library_id = match &library {
            Some(library) => {
                crate::visualizer_log::write_to(Arc::clone(&library.main_db));
                library.library_id
            }
            // A service library keeps no log on this device, and the binding of the playing
            // track's library finds the covers.
            None => {
                crate::visualizer_log::detach();
                library_id.context("The open service library has no id")?
            }
        };
        provide_covers(&node, library_id, player.lock().await.get_status().item);
        let tap = player.lock().await.pcm_tap();
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        // A restart: take the audio back from the running visualizer first.
        if let Some(input) = visualizer.stop() {
            // Restore the consumer back to the tap, just like StopVisualizerRequest does.
            // The callback (on_consumer_abandoned) is no longer used for restarting.
            tap.restore_consumer(input.consumer);
        }
        let Some(consumer) = tap.take_consumer() else {
            return report(
                "start",
                Err(anyhow::anyhow!("The PCM tap is in use")),
                &visualizer,
            );
        };
        tap.set_enabled(true);
        let on_consumer_abandoned = {
            let tap = Arc::clone(&tap);
            move || tap.take_consumer()
        };
        let input = PcmInput {
            consumer,
            channels: tap.channels(),
            sample_rate: tap.sample_rate(),
            on_consumer_abandoned: Box::new(on_consumer_abandoned),
        };
        flutter_bridge::prepare_visualizer(
            Arc::clone(&visualizer),
            dart_signal.width,
            dart_signal.height,
        )
        .await;
        let result = visualizer.start(
            input,
            dart_signal.width,
            dart_signal.height,
            dart_signal.fps,
        );
        if result.is_err() {
            tap.set_enabled(false);
            return report("start", result, &visualizer);
        }
        flutter_bridge::visualizer_started(visualizer).await;
        Ok(Some(()))
    }
}

impl ParamsExtractor for StopVisualizerRequest {
    type Params = (Arc<Mutex<dyn Playable>>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player), all_params.node.clone())
    }
}

impl Signal for StopVisualizerRequest {
    type Params = (Arc<Mutex<dyn Playable>>, Arc<NodeContext>);
    type Response = ();

    async fn handle(
        &self,
        (player, node): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let _turn = STARTING_OR_STOPPING.lock().await;
        let tap = player.lock().await.pcm_tap();
        tap.set_enabled(false);
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        if let Some(input) = flutter_bridge::stop_visualizer(visualizer).await {
            tap.restore_consumer(input.consumer);
        }
        Ok(Some(()))
    }
}

impl ParamsExtractor for ResizeVisualizerRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for ResizeVisualizerRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let _turn = STARTING_OR_STOPPING.lock().await;
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        report(
            "resize",
            flutter_bridge::resize_visualizer(&visualizer, dart_signal.width, dart_signal.height)
                .await,
            &visualizer,
        )
    }
}

impl ParamsExtractor for SetVisualizerPresetDirsRequest {
    type Params = (Arc<dyn Broadcaster>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.node.gui_broadcaster),
            all_params.node.clone(),
        )
    }
}

impl Signal for SetVisualizerPresetDirsRequest {
    type Params = (Arc<dyn Broadcaster>, Arc<NodeContext>);
    type Response = ();

    async fn handle(
        &self,
        (broadcaster, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let dirs: Vec<PathBuf> = dart_signal.paths.iter().map(PathBuf::from).collect();
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        let result = report(
            "scan_scenes",
            visualizer.set_preset_dirs(&dirs).map(|_| ()),
            &visualizer,
        );
        // The list is ready now, also when a folder could not be read: the other scenes are.
        announce_scenes(broadcaster.as_ref(), &node);
        result
    }
}

impl ParamsExtractor for SetVisualizerAccentRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for SetVisualizerAccentRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        visualizer.set_accent(dart_signal.argb);
        Ok(Some(()))
    }
}

impl ParamsExtractor for SetVisualizerVisibleRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for SetVisualizerVisibleRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        visualizer.set_visible(dart_signal.visible);
        Ok(Some(()))
    }
}

impl ParamsExtractor for LoadVisualizerPresetRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for LoadVisualizerPresetRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        report(
            "load_scene",
            visualizer.load_preset(Location::parse(&dart_signal.path)),
            &visualizer,
        )
    }
}

impl ParamsExtractor for NextVisualizerPresetRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for NextVisualizerPresetRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        report("switch_scene", visualizer.next_preset(), &visualizer)
    }
}

impl ParamsExtractor for PreviousVisualizerPresetRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for PreviousVisualizerPresetRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        report("switch_scene", visualizer.previous_preset(), &visualizer)
    }
}

impl ParamsExtractor for RandomVisualizerPresetRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for RandomVisualizerPresetRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        report("switch_scene", visualizer.random_preset(), &visualizer)
    }
}

impl ParamsExtractor for VisualizerStatusRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for VisualizerStatusRequest {
    type Params = Arc<NodeContext>;
    type Response = VisualizerStatusResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        let status = visualizer.status();
        Ok(Some(VisualizerStatusResponse {
            call_id: None,
            runtime: message_runtime_status(&status),
            width: status.width,
            height: status.height,
            target_fps: status.target_fps,
            measured_fps: status.measured_fps,
            preset_name: status.preset_name,
            preset_path: status.preset_path,
            preset_count: u32::try_from(status.preset_count).unwrap_or(u32::MAX),
            inputs: status.inputs.into(),
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
        Ok(Some(ListVisualizerCreditsResponse {
            call_id: None,
            credits,
        }))
    }
}

impl ParamsExtractor for SetVisualizerScenesRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for SetVisualizerScenesRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        let parse = |paths: &[String]| paths.iter().map(|path| Location::parse(path)).collect();
        visualizer.set_scene_choice(parse(&dart_signal.order), parse(&dart_signal.disabled));
        if let Some(last) = &dart_signal.last {
            visualizer.resume_from(Location::parse(last));
        }
        Ok(Some(()))
    }
}

impl ParamsExtractor for ImportVisualizerSceneRequest {
    type Params = (Arc<dyn Broadcaster>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.node.gui_broadcaster),
            all_params.node.clone(),
        )
    }
}

impl Signal for ImportVisualizerSceneRequest {
    type Params = (Arc<dyn Broadcaster>, Arc<NodeContext>);
    type Response = ImportVisualizerSceneResponse;

    async fn handle(
        &self,
        (broadcaster, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let file = PathBuf::from(&dart_signal.file);
        let dir = PathBuf::from(&dart_signal.dir);
        let imported = {
            let (file, dir) = (file.clone(), dir.clone());
            // A panic in the task is an answer too: the request must not end in a crash response.
            task::spawn_blocking(move || package::import(&file, &dir))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("The import stopped unexpectedly: {e}")))
        };
        let path = match imported {
            Ok(path) => {
                let imported = format!("Imported {} into {}", file.display(), path.display());
                log::info!("{imported}");
                crate::visualizer_log::diagnostic("import_scene", imported);
                rescan(&dir, broadcaster.as_ref(), &node);
                path.to_string_lossy().into_owned()
            }
            Err(e) => {
                log::error!("Failed to import the scene {}: {e:#}", file.display());
                crate::visualizer_log::error("import_scene", format!("{}: {e:#}", file.display()));
                return Err(ExpectedRequestError::invalid_input(format!("{e:#}")).into());
            }
        };
        Ok(Some(ImportVisualizerSceneResponse {
            call_id: None,
            file: dart_signal.file.clone(),
            path,
        }))
    }
}

impl ParamsExtractor for RemoveVisualizerSceneRequest {
    type Params = (Arc<dyn Broadcaster>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.node.gui_broadcaster),
            all_params.node.clone(),
        )
    }
}

impl Signal for RemoveVisualizerSceneRequest {
    type Params = (Arc<dyn Broadcaster>, Arc<NodeContext>);
    type Response = RemoveVisualizerSceneResponse;

    async fn handle(
        &self,
        (broadcaster, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let path = PathBuf::from(&dart_signal.path);
        let dir = PathBuf::from(&dart_signal.dir);
        let removed = {
            let (path, dir) = (path.clone(), dir.clone());
            // A panic in the task is an answer too: the request must not end in a crash response.
            task::spawn_blocking(move || package::remove(&path, &dir))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("The removal stopped unexpectedly: {e}")))
        };
        match removed {
            Ok(()) => {
                log::info!("Deleted the scene {}", path.display());
                rescan(&dir, broadcaster.as_ref(), &node);
            }
            Err(e) => {
                log::error!("Failed to delete the scene {}: {e:#}", path.display());
                crate::visualizer_log::error("remove_scene", format!("{}: {e:#}", path.display()));
                return Err(e);
            }
        }
        Ok(Some(RemoveVisualizerSceneResponse {
            call_id: None,
            path: dart_signal.path.clone(),
        }))
    }
}

/// List the scenes of `dir` again after a scene came or went, and tell the app.
fn rescan(dir: &std_path::Path, broadcaster: &dyn Broadcaster, node: &NodeContext) {
    let visualizer = Arc::clone(node.node_visualizer().visualizer());
    if let Err(e) = visualizer.set_preset_dirs(&[dir.to_path_buf()]) {
        visualizer.report_error("scan_scenes", format!("{e:#}"));
    }
    announce_scenes(broadcaster, node);
}

/// Every scene the visualizer lists now.
fn scene_entries(visualizer: &::visualizer::Visualizer) -> Vec<VisualizerPresetEntry> {
    visualizer
        .list_presets()
        .into_iter()
        .map(|info| VisualizerPresetEntry {
            name: info.name,
            path: info.location.to_string(),
            group: info.group,
            builtin: matches!(info.location, Location::Builtin(_)),
            inputs: match info.inputs {
                SceneInputs::Spectrum => VisualizerSceneInputs::Spectrum,
                SceneInputs::SpectrumAndChart => VisualizerSceneInputs::SpectrumAndChart,
            },
        })
        .collect()
}

/// Tell the app that the list of scenes is ready or changed, with the list.
fn announce_scenes(broadcaster: &dyn Broadcaster, node: &NodeContext) {
    broadcaster.broadcast(&VisualizerScenesChanged {
        presets: scene_entries(node.node_visualizer().visualizer()),
    });
}

impl ParamsExtractor for SetVisualizerLogRequest {
    type Params = LocalLibrary;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        LocalLibrary::from_global(all_params)
    }
}

impl Signal for SetVisualizerLogRequest {
    type Params = LocalLibrary;
    type Response = ();

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = library.or_service()?;
        crate::visualizer_log::set_diagnostics(dart_signal.enabled);
        match library {
            Some(library) => crate::visualizer_log::write_to(Arc::clone(&library.main_db)),
            None => crate::visualizer_log::detach(),
        }
        Ok(Some(()))
    }
}

/// What [`visualizer::probe_gpu`] found, once: the GPU does not change while the app runs.
static GPU: tokio_sync::OnceCell<result::Result<String, String>> = OnceCell::const_new();

impl ParamsExtractor for VisualizerSupportRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for VisualizerSupportRequest {
    type Params = Arc<NodeContext>;
    type Response = VisualizerSupportResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        let gpu = GPU
            .get_or_init(|| async {
                let probed = task::spawn_blocking(::visualizer::probe_gpu).await;
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
        Ok(Some(VisualizerSupportResponse {
            call_id: None,
            support: message_support(gpu),
            inputs: visualizer.status().inputs.into(),
        }))
    }
}

impl ParamsExtractor for ListVisualizerPresetsRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for ListVisualizerPresetsRequest {
    type Params = Arc<NodeContext>;
    type Response = ListVisualizerPresetsResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        Ok(Some(ListVisualizerPresetsResponse {
            call_id: None,
            presets: scene_entries(&Arc::clone(node.node_visualizer().visualizer())),
        }))
    }
}

impl ParamsExtractor for SetVisualizerLabRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for SetVisualizerLabRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        visualizer.set_inspecting(dart_signal.enabled);
        Ok(Some(()))
    }
}

impl From<VisualizerLabLevel> for Level {
    fn from(level: VisualizerLabLevel) -> Self {
        match level {
            VisualizerLabLevel::A => Self::A,
            VisualizerLabLevel::B => Self::B,
            VisualizerLabLevel::C => Self::C,
        }
    }
}

impl From<Level> for VisualizerLabLevel {
    fn from(level: Level) -> Self {
        match level {
            Level::A => Self::A,
            Level::B => Self::B,
            Level::C => Self::C,
        }
    }
}

impl From<VisualizerLabTransition> for LabTransition {
    fn from(transition: VisualizerLabTransition) -> Self {
        match transition {
            VisualizerLabTransition::Fade => Self::Fade,
            VisualizerLabTransition::Dissolve => Self::Dissolve,
            VisualizerLabTransition::Flash => Self::Flash,
            VisualizerLabTransition::Glitch => Self::Glitch,
            VisualizerLabTransition::Mosaic => Self::Mosaic,
        }
    }
}

impl From<VisualizerLabCommand> for LabCommand {
    fn from(command: VisualizerLabCommand) -> Self {
        match command {
            VisualizerLabCommand::Cut => Self::Cut,
            VisualizerLabCommand::Shot { index } => Self::Shot(index),
            VisualizerLabCommand::Look { index } => Self::Look(index),
            VisualizerLabCommand::CutDressed { transition } => Self::CutDressed(transition.into()),
            VisualizerLabCommand::Flare => Self::Flare,
            VisualizerLabCommand::Leak => Self::Leak,
            VisualizerLabCommand::LensGlitch => Self::LensGlitch,
            VisualizerLabCommand::LensMosaic => Self::LensMosaic,
            VisualizerLabCommand::ForceLevel { level } => Self::ForceLevel(level.map(Into::into)),
            VisualizerLabCommand::Burst => Self::Burst,
        }
    }
}

impl ParamsExtractor for VisualizerLabCommandRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for VisualizerLabCommandRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        // A visualizer that is not running has nothing to tell.
        if let Err(e) = visualizer.lab(dart_signal.command.clone().into()) {
            log::warn!("The visualizer lab command was dropped: {e:#}");
        }
        Ok(Some(()))
    }
}

/// The lab's events as messages; an event of a kind this build does not know is left out.
fn lab_event(event: LabEvent) -> Option<VisualizerLabEvent> {
    let kind = match event.kind {
        LabEventKind::Cut {
            shot,
            size,
            fill,
            strong,
            dress,
            first,
            ..
        } => VisualizerLabEventKind::Cut {
            shot,
            size: size.into(),
            fill,
            strong,
            dress,
            first,
        },
        LabEventKind::Look { look } => VisualizerLabEventKind::Look { look },
        LabEventKind::Moment { strength, effect } => VisualizerLabEventKind::Moment {
            strength: strength.into(),
            effect,
        },
        _ => return None,
    };
    Some(VisualizerLabEvent {
        time: event.time,
        kind,
    })
}

/// The live readout of a frame the visualizer saw.
fn lab_now(inspect: Inspect) -> VisualizerLabNow {
    let overrides = inspect.overrides;
    VisualizerLabNow {
        director: inspect.director.map(|director| VisualizerLabDirector {
            shot_index: director.shot_index,
            held: director.held,
            shots: director
                .shots
                .into_iter()
                .map(|shot| VisualizerLabShot {
                    name: shot.name,
                    size: shot.size.into(),
                })
                .collect(),
        }),
        looks: inspect.looks.map(|looks| VisualizerLabLooks {
            names: looks.names,
            on: looks.on,
            held: looks.held,
        }),
        effects: VisualizerLabEffects {
            cut: inspect.effects.cut,
            lens: inspect.effects.lens,
        },
        forced_level: overrides.level.map(Into::into),
    }
}

fn message_plan(plan: &::visualizer::SongPlan) -> VisualizerPlan {
    VisualizerPlan {
        length: plan.length,
        sections: plan
            .sections
            .iter()
            .map(|section| VisualizerPlanSection {
                start: section.start,
                end: section.end,
                label: section.label.clone(),
                tier: section.tier.into(),
            })
            .collect(),
        cuts: plan
            .cuts
            .iter()
            .map(|cut| VisualizerPlanCut {
                time: cut.time,
                shot: cut.shot.clone(),
                size: cut.size.into(),
                fill: cut.fill,
                strong: cut.strong,
                dress: cut.dress.clone(),
                first: cut.first,
            })
            .collect(),
        looks: plan
            .looks
            .iter()
            .map(|look| VisualizerPlanLook {
                time: look.time,
                look: look.look.clone(),
            })
            .collect(),
        moments: plan
            .moments
            .iter()
            .map(|moment| VisualizerPlanMoment {
                time: moment.time,
                strength: moment.strength.into(),
                effect: moment.effect.clone(),
            })
            .collect(),
        series: plan.series.as_ref().map(|series| VisualizerPlanSeries {
            columns_version: series.columns_version,
            rate: ::visualizer::LAB_RATE,
            buckets: series.buckets,
            values: series.values.clone(),
        }),
    }
}

impl ParamsExtractor for VisualizerLabRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for VisualizerLabRequest {
    type Params = Arc<NodeContext>;
    type Response = VisualizerLabResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        let inspect = visualizer.inspect();
        // The samples are drained before the catalogue is read: a switch between the two calls
        // leaves samples of the old catalogue, which the app resets, not ones it would drop.
        let samples = visualizer.drain_lab_samples();
        let columns = visualizer
            .lab_columns()
            .filter(|columns| columns.version != dart_signal.columns_version)
            .map(|columns| VisualizerLabColumns {
                version: columns.version,
                rate: ::visualizer::LAB_RATE,
                columns: columns
                    .columns
                    .iter()
                    .map(|column| VisualizerLabColumn {
                        id: column.id.clone(),
                        name: column.name.clone(),
                        group: column.group.into(),
                        kind: column.kind.into(),
                        lo: column.lo,
                        hi: column.hi,
                        unit: column.unit.clone(),
                        categories: column.categories.clone(),
                        plan: column.plan.into(),
                        binds: column.binds.clone(),
                        rest: column.rest,
                    })
                    .collect(),
            });
        Ok(Some(VisualizerLabResponse {
            call_id: None,
            inspecting: visualizer.is_inspecting(),
            song_time: inspect
                .as_ref()
                .map_or(0.0, |inspect| f64::from(inspect.music.time)),
            plan_version: visualizer.plan_version(),
            events: visualizer
                .drain_lab_events()
                .into_iter()
                .filter_map(lab_event)
                .collect(),
            now: inspect.map(lab_now),
            columns,
            samples: samples
                .into_iter()
                .map(|sample| VisualizerLabSample {
                    bucket: sample.bucket,
                    columns_version: sample.columns_version,
                    values: sample.values,
                })
                .collect(),
        }))
    }
}

impl ParamsExtractor for VisualizerPlanRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for VisualizerPlanRequest {
    type Params = Arc<NodeContext>;
    type Response = VisualizerPlanResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let visualizer = Arc::clone(node.node_visualizer().visualizer());
        // The version first: a plan made in between is then fetched again for its own version.
        let version = visualizer.plan_version();
        let plan = visualizer.plan().map(|plan| message_plan(&plan));
        Ok(Some(VisualizerPlanResponse {
            call_id: None,
            version,
            plan,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync as std_sync;
    use visualizer::Status;
    use visualizer::Transport;
    use visualizer::{ColumnGroup, ColumnKind, ColumnPlan, ShotSize, Strength, Tier};

    use crate::Signal;
    use crate::messages::{
        VisualizerLabColumnGroup, VisualizerLabColumnKind, VisualizerLabColumnPlan,
        VisualizerMomentStrength, VisualizerRuntimeStatus, VisualizerSectionTier,
        VisualizerShotSize, VisualizerSupport, VisualizerTransport,
    };
    use crate::node::NodeContext;
    use crate::server::utils::device::RunMode;

    use super::{Arc, Broadcaster, SetVisualizerPresetDirsRequest, VisualizerScenesChanged};
    use super::{VisualizerStatusRequest, message_runtime_status, message_support};

    #[derive(Default)]
    struct Capture(std_sync::Mutex<Vec<VisualizerScenesChanged>>);

    impl Broadcaster for Capture {
        fn broadcast(&self, message: &dyn crate::utils::RinfRustSignal) {
            if message.name() == "VisualizerScenesChanged"
                && let Ok(bytes) = message.encode_to_vec()
                && let Ok(event) = rinf::deserialize::<VisualizerScenesChanged>(&bytes)
            {
                self.0.lock().unwrap().push(event);
            }
        }
    }

    #[tokio::test]
    async fn setting_the_scene_folders_announces_the_scene_list() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_node::disable_media_server(dir.path()).ok();
        let path = dir.path().to_string_lossy();
        let capture = Arc::new(Capture::default());
        let node = NodeContext::new(
            &path,
            &path,
            capture.clone(),
            false,
            RunMode::App,
            Arc::clone(&crate::visualizer::VISUALIZER),
        )
        .await
        .unwrap();
        let request = SetVisualizerPresetDirsRequest {
            paths: vec![dir.path().to_string_lossy().into_owned()],
            call_id: None,
        };
        request
            .handle((capture.clone(), node), None, &request)
            .await
            .unwrap();

        let announced = capture.0.lock().unwrap();
        let [event] = announced.as_slice() else {
            panic!("expected one announcement, got {}", announced.len());
        };
        assert!(event.presets.iter().any(|preset| preset.builtin));
        assert!(
            event
                .presets
                .iter()
                .any(|preset| preset.path == "app:cover-wall")
        );
    }

    #[test]
    fn visualizer_support_handler_encodes_each_case() {
        assert_eq!(
            message_support(&Ok("Adapter".into())),
            VisualizerSupport::Supported {
                adapter: "Adapter".into()
            }
        );
        assert_eq!(
            message_support(&Err("No GPU".into())),
            VisualizerSupport::Unsupported {
                problem: "No GPU".into()
            }
        );
    }

    #[test]
    fn hub_choice_messages_follow_visualizer_enums() {
        for (source, message) in [
            (ShotSize::Wide, VisualizerShotSize::Wide),
            (ShotSize::Medium, VisualizerShotSize::Medium),
            (ShotSize::Close, VisualizerShotSize::Close),
            (ShotSize::Detail, VisualizerShotSize::Detail),
            (ShotSize::Top, VisualizerShotSize::Top),
        ] {
            assert_eq!(VisualizerShotSize::from(source), message);
        }
        for (source, message) in [
            (Strength::Medium, VisualizerMomentStrength::Medium),
            (Strength::Strong, VisualizerMomentStrength::Strong),
        ] {
            assert_eq!(VisualizerMomentStrength::from(source), message);
        }
        for (source, message) in [
            (ColumnGroup::Director, VisualizerLabColumnGroup::Director),
            (ColumnGroup::Looks, VisualizerLabColumnGroup::Looks),
            (ColumnGroup::Signals, VisualizerLabColumnGroup::Signals),
            (ColumnGroup::Inputs, VisualizerLabColumnGroup::Inputs),
        ] {
            assert_eq!(VisualizerLabColumnGroup::from(source), message);
        }
        for (source, message) in [
            (ColumnKind::Number, VisualizerLabColumnKind::Number),
            (ColumnKind::Event, VisualizerLabColumnKind::Event),
            (ColumnKind::Bool, VisualizerLabColumnKind::Bool),
            (ColumnKind::Category, VisualizerLabColumnKind::Category),
            (ColumnKind::Phase, VisualizerLabColumnKind::Phase),
        ] {
            assert_eq!(VisualizerLabColumnKind::from(source), message);
        }
        for (source, message) in [
            (ColumnPlan::Yes, VisualizerLabColumnPlan::Yes),
            (ColumnPlan::No, VisualizerLabColumnPlan::No),
        ] {
            assert_eq!(VisualizerLabColumnPlan::from(source), message);
        }
        for (source, message) in [
            (Tier::Quiet, VisualizerSectionTier::Quiet),
            (Tier::Normal, VisualizerSectionTier::Normal),
            (Tier::High, VisualizerSectionTier::High),
        ] {
            assert_eq!(VisualizerSectionTier::from(source), message);
        }
    }

    #[test]
    fn visualizer_status_handler_encodes_started_runtime_states() {
        let status = |running, transport| visualizer::Status {
            running,
            adapter: "Adapter".into(),
            transport: Some(transport),
            ..Default::default()
        };
        for (engine_transport, wire_transport) in [
            (Transport::Readback, VisualizerTransport::Readback),
            (Transport::IoSurface, VisualizerTransport::IoSurface),
            (Transport::DxgiShared, VisualizerTransport::DxgiShared),
            (Transport::VulkanShared, VisualizerTransport::VulkanShared),
            (Transport::Surface, VisualizerTransport::Surface),
        ] {
            assert_eq!(
                message_runtime_status(&status(true, engine_transport)),
                VisualizerRuntimeStatus::Running {
                    adapter: "Adapter".into(),
                    transport: wire_transport,
                }
            );
            assert_eq!(
                message_runtime_status(&status(false, engine_transport)),
                VisualizerRuntimeStatus::Stopped {
                    adapter: "Adapter".into(),
                    transport: wire_transport,
                }
            );
        }
        assert_eq!(
            message_runtime_status(&Status::default()),
            VisualizerRuntimeStatus::NotRunningYet
        );
    }

    #[tokio::test]
    async fn visualizer_status_handler_returns_the_render_thread_state() {
        // For this test to work with the node-based handler, we need a node.
        // Since we're in a test and don't have a full node setup, we use the global VISUALIZER
        // which is what the app node uses in production.
        // The handler will use the node's visualizer, which for the app node is VISUALIZER.
        // So we can create a minimal node for testing.
        use crate::node::NodeContext;
        use crate::server::utils::device::RunMode;
        use crate::utils::Broadcaster;

        #[derive(Default)]
        struct TestBroadcaster;
        impl Broadcaster for TestBroadcaster {
            fn broadcast(&self, _message: &dyn crate::utils::RinfRustSignal) {}
        }

        // Create a test node with the global visualizer
        // This simulates the app node in production
        let dir = tempfile::tempdir().unwrap();
        crate::test_node::disable_media_server(dir.path()).ok();
        let path = dir.path().to_string_lossy();
        let node = NodeContext::new(
            &path,
            &path,
            Arc::new(TestBroadcaster),
            false,
            RunMode::App,
            Arc::clone(&crate::visualizer::VISUALIZER),
        )
        .await
        .unwrap();

        let request = VisualizerStatusRequest { call_id: None };
        let response = request.handle(node, None, &request).await.unwrap().unwrap();
        assert_eq!(
            response.runtime,
            message_runtime_status(&crate::visualizer::VISUALIZER.status())
        );
    }

    /// Regression test for issue #497: handlers route through the node's visualizer,
    /// not the global VISUALIZER. Each node maintains its own inspecting state, and the
    /// SetVisualizerLabRequest handler affects only the targeted node's visualizer.
    #[tokio::test]
    async fn visualizer_controls_use_node_instance() {
        use crate::server::utils::device::RunMode;
        use crate::utils::Broadcaster;
        use ::visualizer::Visualizer;

        #[derive(Default)]
        struct TestBroadcaster;
        impl Broadcaster for TestBroadcaster {
            fn broadcast(&self, _message: &dyn crate::utils::RinfRustSignal) {}
        }

        // Save the original global VISUALIZER state to verify it remains unchanged
        let original_global_inspecting = crate::visualizer::VISUALIZER.is_inspecting();
        let original_global_preset_dirs = crate::visualizer::VISUALIZER.list_presets();

        // Create two separate visualizers
        let visualizer1 = Arc::new(Visualizer::new());
        let visualizer2 = Arc::new(Visualizer::new());

        // Create nodes with different visualizers and separate folders
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        crate::test_node::disable_media_server(dir1.path()).ok();
        crate::test_node::disable_media_server(dir2.path()).ok();
        let path1 = dir1.path().to_string_lossy();
        let path2 = dir2.path().to_string_lossy();

        let node1 = NodeContext::new(
            &path1,
            &path1,
            Arc::new(TestBroadcaster),
            false,
            RunMode::App,
            Arc::clone(&visualizer1),
        )
        .await
        .unwrap();

        let node2 = NodeContext::new(
            &path2,
            &path2,
            Arc::new(TestBroadcaster),
            false,
            RunMode::App,
            Arc::clone(&visualizer2),
        )
        .await
        .unwrap();

        // Verify that the nodes have different visualizers
        let node1_vis = Arc::clone(node1.node_visualizer().visualizer());
        let node2_vis = Arc::clone(node2.node_visualizer().visualizer());
        assert!(!Arc::ptr_eq(&node1_vis, &node2_vis));

        // Test: different values per node - inspecting state
        node1_vis.set_inspecting(true);
        node2_vis.set_inspecting(true);
        assert!(node1_vis.is_inspecting());
        assert!(node2_vis.is_inspecting());

        // Test: SetVisualizerLabRequest through handler affects only node1
        // Node1 starts true, node2 starts true
        let request = crate::messages::SetVisualizerLabRequest {
            enabled: false,
            call_id: None,
        };
        request
            .handle(Arc::clone(&node1), None, &request)
            .await
            .unwrap();
        // Node1 should now be false, node2 should remain unchanged (true)
        assert!(!node1_vis.is_inspecting());
        assert!(node2_vis.is_inspecting());

        // Now set node1 to false and node2 to true
        node1_vis.set_inspecting(false);
        node2_vis.set_inspecting(true);
        assert!(!node1_vis.is_inspecting());
        assert!(node2_vis.is_inspecting());

        // Apply request to node2
        let request2 = crate::messages::SetVisualizerLabRequest {
            enabled: false,
            call_id: None,
        };
        request2
            .handle(Arc::clone(&node2), None, &request2)
            .await
            .unwrap();
        // Node2 should now be false, node1 should remain unchanged (false)
        assert!(!node1_vis.is_inspecting());
        assert!(!node2_vis.is_inspecting());

        // Test: global VISUALIZER remains unchanged
        assert_eq!(
            crate::visualizer::VISUALIZER.is_inspecting(),
            original_global_inspecting
        );
        assert_eq!(
            crate::visualizer::VISUALIZER.list_presets(),
            original_global_preset_dirs
        );
    }
}
