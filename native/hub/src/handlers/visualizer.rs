use crate::node::NodeContext;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

use anyhow::Result;
use tokio::sync::Mutex;

use ::playback::player::Playable;
use ::visualizer::{
    Inspect, LabCommand, LabEvent, LabEventKind, LabTransition, Level, Location, PcmInput,
    Transport,
};

use crate::{
    Session, Signal, flutter_bridge,
    library_pool::LibraryHandle,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, require_library},
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
        Option<LibraryHandle>,
        Arc<NodeContext>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.player),
            all_params.library.clone(),
            all_params.node.clone(),
        )
    }
}

impl Signal for StartVisualizerRequest {
    type Params = (
        Arc<Mutex<dyn Playable>>,
        Option<LibraryHandle>,
        Arc<NodeContext>,
    );
    type Response = ();

    async fn handle(
        &self,
        (player, library, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let _turn = STARTING_OR_STOPPING.lock().await;
        crate::visualizer_log::write_to(Arc::clone(&library.main_db));
        provide_covers(
            Arc::clone(&library.fsio),
            Arc::clone(&library.main_db),
            Arc::clone(&library.recommend_db),
            node,
            library.library_id,
        );
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
            // A panic in the task is an answer too: the request must not end in a crash response.
            tokio::task::spawn_blocking(move || ::visualizer::package::import(&file, &dir))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("The import stopped unexpectedly: {e}")))
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
            // A panic in the task is an answer too: the request must not end in a crash response.
            tokio::task::spawn_blocking(move || ::visualizer::package::remove(&path, &dir))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("The removal stopped unexpectedly: {e}")))
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
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for SetVisualizerLogRequest {
    type Params = Option<LibraryHandle>;
    type Response = ();

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        crate::visualizer_log::set_diagnostics(dart_signal.enabled);
        crate::visualizer_log::write_to(Arc::clone(&library.main_db));
        Ok(Some(()))
    }
}

/// What [`visualizer::probe_gpu`] found, once: the GPU does not change while the app runs.
static GPU: tokio::sync::OnceCell<std::result::Result<String, String>> = OnceCell::const_new();

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

impl ParamsExtractor for SetVisualizerLabRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for SetVisualizerLabRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        VISUALIZER.set_inspecting(dart_signal.enabled);
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
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for VisualizerLabCommandRequest {
    type Params = ();
    type Response = ();

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        // A visualizer that is not running has nothing to tell.
        if let Err(e) = VISUALIZER.lab(dart_signal.command.clone().into()) {
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
            size,
            fill,
            strong,
            dress,
            first,
        },
        LabEventKind::Look { look } => VisualizerLabEventKind::Look { look },
        LabEventKind::Moment { strength, effect } => {
            VisualizerLabEventKind::Moment { strength, effect }
        }
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
                    size: shot.size,
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

impl ParamsExtractor for VisualizerLabRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for VisualizerLabRequest {
    type Params = ();
    type Response = VisualizerLabResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let inspect = VISUALIZER.inspect();
        // The samples are drained before the catalogue is read: a switch between the two calls
        // leaves samples of the old catalogue, which the app resets, not ones it would drop.
        let samples = VISUALIZER.drain_lab_samples();
        let columns = VISUALIZER
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
                        group: column.group.as_str().to_string(),
                        kind: column.kind.as_str().to_string(),
                        lo: column.lo,
                        hi: column.hi,
                        unit: column.unit.clone(),
                        categories: column.categories.clone(),
                        plan: column.plan.as_str().to_string(),
                        binds: column.binds.clone(),
                        rest: column.rest,
                    })
                    .collect(),
            });
        Ok(Some(VisualizerLabResponse {
            inspecting: VISUALIZER.is_inspecting(),
            song_time: inspect
                .as_ref()
                .map_or(0.0, |inspect| f64::from(inspect.music.time)),
            plan_version: VISUALIZER.plan_version(),
            events: VISUALIZER
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
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for VisualizerPlanRequest {
    type Params = ();
    type Response = VisualizerPlanResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        // The version first: a plan made in between is then fetched again for its own version.
        let version = VISUALIZER.plan_version();
        let plan = VISUALIZER.plan().map(|plan| VisualizerPlan {
            length: plan.length,
            sections: plan
                .sections
                .iter()
                .map(|section| VisualizerPlanSection {
                    start: section.start,
                    end: section.end,
                    label: section.label.clone(),
                    tier: section.tier.clone(),
                })
                .collect(),
            cuts: plan
                .cuts
                .iter()
                .map(|cut| VisualizerPlanCut {
                    time: cut.time,
                    shot: cut.shot.clone(),
                    size: cut.size.clone(),
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
                    strength: moment.strength.clone(),
                    effect: moment.effect.clone(),
                })
                .collect(),
            series: plan.series.as_ref().map(|series| VisualizerPlanSeries {
                columns_version: series.columns_version,
                rate: ::visualizer::LAB_RATE,
                buckets: series.buckets,
                values: series.values.clone(),
            }),
        });
        Ok(Some(VisualizerPlanResponse { version, plan }))
    }
}
