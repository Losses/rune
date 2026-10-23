//! The GPU side: a headless device, the lactea engine and where frames go: on macOS straight into
//! IOSurface-backed pixel buffers ([`crate::surface`]), elsewhere (or if that fails) an offscreen
//! target that is read back into mapped buffers ([`crate::readback`]).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use lactea::{Engine, EngineConfig, FrameClock, Transition};
use log::warn;

use crate::chart::ChartState;
use crate::readback::{MappedFrame, Target};
use crate::rhythm::Rhythm;
#[cfg(target_os = "macos")]
use crate::surface::{PixelBuffer, SurfaceTargets};
use crate::textures::FolderTextures;

/// `kCVPixelFormatType_32BGRA` on macOS. Windows' `FlutterDesktopPixelBuffer` and Linux's
/// `FlPixelBufferTexture` take RGBA. The engine only compensates for `*Srgb` targets, so with a
/// linear format the bytes are exactly what the preset drew.
#[cfg(target_os = "macos")]
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
#[cfg(not(target_os = "macos"))]
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct Pipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    engine: Engine,
    output: Output,
    readback: bool,
    pub adapter_name: String,
}

/// Rune's own preset (`presets/Rune.milk`, MPL-2.0 like the rest of Rune), shown before a preset
/// is chosen.
const DEFAULT_PRESET: &[u8] = include_bytes!("../presets/Rune.milk");
/// The same preset drawn on a light background, for the app's light appearance.
const DEFAULT_PRESET_LIGHT: &[u8] = include_bytes!("../presets/Rune Light.milk");

/// The default preset for the app's appearance.
fn default_preset(light: bool) -> &'static [u8] {
    if light {
        DEFAULT_PRESET_LIGHT
    } else {
        DEFAULT_PRESET
    }
}
/// The name [`DEFAULT_PRESET`] is loaded under.
pub const DEFAULT_PRESET_NAME: &str = "Rune";

/// What a rendered frame became.
pub enum Rendered {
    /// A read-back frame (on the readback path a frame comes out a call or two after it was
    /// drawn).
    Mapped(MappedFrame),
    #[cfg(target_os = "macos")]
    PixelBuffer(PixelBuffer),
    /// Nothing new: every buffer is still in use, or the GPU has not finished one yet.
    Nothing,
}

/// How frames leave the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Readback,
    IoSurface,
}

enum Output {
    Readback(Target),
    #[cfg(target_os = "macos")]
    Surfaces(SurfaceTargets),
}

impl Output {
    /// Pixel buffers on macOS unless `readback`, else (or if they fail) the readback target.
    fn new(device: &wgpu::Device, width: u32, height: u32, readback: bool) -> Self {
        #[cfg(target_os = "macos")]
        if !readback {
            match SurfaceTargets::new(device, width, height) {
                Ok(surfaces) => return Self::Surfaces(surfaces),
                Err(e) => warn!("Falling back to reading frames back: {e:#}"),
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = readback;
        Self::Readback(Target::new(device, width, height))
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Readback(target) => (target.width, target.height),
            #[cfg(target_os = "macos")]
            Self::Surfaces(surfaces) => surfaces.size(),
        }
    }
}

impl Pipeline {
    pub fn new(
        width: u32,
        height: u32,
        fps: u32,
        sample_rate: u32,
        readback: bool,
        light: bool,
        on_gpu_error: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| anyhow!("No GPU adapter: {e}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .context("Failed to create the GPU device")?;
        // Without a handler wgpu panics on validation errors.
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| on_gpu_error(e.to_string())));

        let (width, height) = (width.max(1), height.max(1));
        let mut engine = Engine::new(
            &device,
            &queue,
            EngineConfig {
                width,
                height,
                output_format: FORMAT,
                sample_rate,
                fps,
                // Pick projectM or MilkDrop 3 semantics per preset (and allow `.milk2`).
                auto_compat: true,
                ..Default::default()
            },
        )
        .map_err(|e| anyhow!("Failed to create the engine: {e}"))?;
        // Rune's own preset is what shows before anything else is chosen; lactea's idle preset
        // is the fallback should it ever fail to load.
        match engine.load_preset(default_preset(light), DEFAULT_PRESET_NAME, Transition::Cut) {
            Ok(report) if !report.warnings.is_empty() || !report.shader_fallbacks.is_empty() => {
                warn!(
                    "The default preset loaded with problems: {:?}, fallbacks {:?}",
                    report.warnings, report.shader_fallbacks
                );
            }
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to load the default preset: {e}");
                engine
                    .load_idle_preset()
                    .map_err(|e| anyhow!("Failed to load the idle preset: {e}"))?;
            }
        }
        let output = Output::new(&device, width, height, readback);
        Ok(Self {
            adapter_name: adapter.get_info().name,
            device,
            queue,
            engine,
            output,
            readback,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.output.size()
    }

    /// The name of the loaded preset.
    pub fn preset_name(&self) -> &str {
        self.engine
            .preset_name()
            .unwrap_or(lactea::IDLE_PRESET_NAME)
    }

    pub fn transport(&self) -> Transport {
        match self.output {
            Output::Readback(_) => Transport::Readback,
            #[cfg(target_os = "macos")]
            Output::Surfaces(_) => Transport::IoSurface,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        self.engine
            .resize(width, height)
            .map_err(|e| anyhow!("Failed to resize the engine: {e}"))?;
        self.output = Output::new(&self.device, width, height, self.readback);
        Ok(())
    }

    pub fn load_preset(&mut self, bytes: &[u8], name: &str, transition: Transition) -> Result<()> {
        self.load_preset_reporting(bytes, name, transition)
            .map(|_| ())
    }

    /// Like [`Self::load_preset`], with what lactea found: warnings, and shaders it replaced
    /// because they did not compile.
    pub fn load_preset_reporting(
        &mut self,
        bytes: &[u8],
        name: &str,
        transition: Transition,
    ) -> Result<lactea::PresetReport> {
        self.engine
            .load_preset(bytes, name, transition)
            .map_err(|e| anyhow!("Failed to load preset {name}: {e}"))
    }

    pub fn load_default_preset(&mut self, light: bool, transition: Transition) -> Result<()> {
        self.load_preset(default_preset(light), DEFAULT_PRESET_NAME, transition)
    }

    /// The textures of the preset folders `dirs`, none without one.
    pub fn set_textures(&mut self, dirs: &[PathBuf]) -> Result<()> {
        self.engine
            .set_texture_provider(Box::new(FolderTextures::new(dirs)));
        Ok(())
    }

    /// Hand the beat tracker's findings to the presets' per-frame code, as the variables listed
    /// in the README.
    pub fn set_rhythm(&mut self, rhythm: &Rhythm) {
        let count = rhythm.beat.floor();
        for (name, value) in [
            ("rune_bpm", rhythm.bpm),
            ("rune_beat_count", count),
            ("rune_beat_phase", rhythm.beat - count),
            ("rune_beat_conf", rhythm.confidence),
            ("rune_kick", rhythm.kick),
            ("rune_snare", rhythm.snare),
            ("rune_hat", rhythm.hat),
            ("rune_energy", rhythm.energy),
        ] {
            self.engine.set_host_var(name, value);
        }
    }

    /// The song's chart at this frame, or zeros (and `rune_chart` 0) without one.
    pub fn set_chart(&mut self, chart: Option<&ChartState>) {
        let c = chart.copied().unwrap_or_default();
        let bar = c.bar.floor();
        for (name, value) in [
            ("rune_chart", if chart.is_some() { 1.0 } else { 0.0 }),
            ("rune_bar_count", bar),
            ("rune_bar_phase", c.bar - bar),
            ("rune_beats_per_bar", c.beats_per_bar),
            ("rune_meter_conf", c.meter_confidence),
            ("rune_section", c.section),
            ("rune_section_label", c.section_label),
            ("rune_section_repeat", c.section_repeat),
            ("rune_section_energy", c.section_energy),
            ("rune_next_label", c.next_label),
            ("rune_next_energy", c.next_energy),
            ("rune_section_time", c.section_time),
            ("rune_section_left", c.section_left),
            ("rune_section_conf", c.section_confidence),
            ("rune_ev_low", c.events[0]),
            ("rune_ev_mid", c.events[1]),
            ("rune_ev_high", c.events[2]),
            ("rune_ev_harm", c.events[3]),
            ("rune_ev_phrase", c.events[4]),
            ("rune_lead", c.lead),
            ("rune_lead_ev", c.lead_event),
            ("rune_loud", c.loudness),
            ("rune_bright", c.brightness),
            ("rune_perc", c.percussive),
            ("rune_pulse", c.pulse),
        ] {
            self.engine.set_host_var(name, value);
        }
    }

    pub fn push_pcm(&mut self, interleaved: &[f32], channels: usize) {
        self.engine.push_pcm_f32(interleaved, channels);
    }

    /// Render one frame: into the next pixel buffer, or into the target that is read back.
    pub fn render(&mut self, clock: FrameClock) -> Result<Rendered> {
        #[cfg(target_os = "macos")]
        if let Output::Surfaces(surfaces) = &mut self.output {
            match surfaces.next(&self.device) {
                Ok(Some((buffer, view))) => {
                    let mut encoder = self.device.create_command_encoder(&Default::default());
                    self.engine.render(&mut encoder, &view, clock);
                    let submission = self.queue.submit(Some(encoder.finish()));
                    // Hand the buffer out only once the GPU has finished drawing into it.
                    self.device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: None,
                        })
                        .map_err(|e| anyhow!("GPU poll failed: {e:?}"))?;
                    surfaces.keep(buffer.clone());
                    return Ok(Rendered::PixelBuffer(buffer));
                }
                Ok(None) => return Ok(Rendered::Nothing),
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = surfaces.size();
                    self.output = Output::Readback(Target::new(&self.device, width, height));
                }
            }
        }
        let Output::Readback(target) = &mut self.output else {
            unreachable!("only macOS has other outputs")
        };
        let engine = &mut self.engine;
        let frame = target.render(&self.device, &self.queue, |encoder, view| {
            engine.render(encoder, view, clock);
        })?;
        Ok(frame.map_or(Rendered::Nothing, Rendered::Mapped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rune's preset must load cleanly: a shader that fails to translate would be silently
    /// replaced by a default one.
    #[test]
    fn default_preset_loads_without_fallbacks() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut engine = Engine::new(
            &device,
            &queue,
            EngineConfig {
                width: 640,
                height: 360,
                output_format: FORMAT,
                auto_compat: true,
                ..Default::default()
            },
        )
        .unwrap();
        for preset in [DEFAULT_PRESET, DEFAULT_PRESET_LIGHT] {
            let report = engine
                .load_preset(preset, DEFAULT_PRESET_NAME, Transition::Cut)
                .unwrap();
            assert!(report.warnings.is_empty(), "{:?}", report.warnings);
            assert!(
                report.shader_fallbacks.is_empty(),
                "{:?}",
                report.shader_fallbacks
            );
        }
    }

    /// What the beat tracker found reaches a preset's per-frame code.
    #[test]
    fn presets_see_the_rhythm() {
        let Ok(mut pipeline) = Pipeline::new(64, 36, 60, 44_100, true, false, Arc::new(|_| {}))
        else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let preset = b"MILKDROP_PRESET_VERSION=201\n[preset00]\n\
            per_frame_1=seen_bpm=rune_bpm; seen_phase=rune_beat_phase; seen_count=rune_beat_count;\n\
            per_frame_2=seen_kick=rune_kick; seen_conf=rune_beat_conf; seen_energy=rune_energy;\n";
        pipeline.load_preset(preset, "p", Transition::Cut).unwrap();
        pipeline.set_rhythm(&Rhythm {
            bpm: 128.0,
            beat: 41.25,
            confidence: 0.75,
            kick: 0.5,
            energy: 1.5,
            ..Default::default()
        });
        pipeline
            .render(FrameClock {
                time: 0.0,
                frame: 0,
            })
            .unwrap();
        let vars = pipeline.engine.frame_vars();
        let var = |name: &str| vars.iter().find(|(n, _)| n == name).map(|(_, v)| *v);
        assert_eq!(var("seen_bpm"), Some(128.0));
        assert_eq!(var("seen_count"), Some(41.0));
        assert_eq!(var("seen_phase"), Some(0.25));
        assert_eq!(var("seen_kick"), Some(0.5));
        assert_eq!(var("seen_conf"), Some(0.75));
        assert_eq!(var("seen_energy"), Some(1.5));
    }

    /// A song's chart reaches a preset's per-frame code, and without one it reads zeros.
    #[test]
    fn presets_see_the_chart() {
        let Ok(mut pipeline) = Pipeline::new(64, 36, 60, 44_100, true, false, Arc::new(|_| {}))
        else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let preset = b"MILKDROP_PRESET_VERSION=201\n[preset00]\n\
            per_frame_1=seen_chart=rune_chart; seen_bar=rune_bar_count; seen_phase=rune_bar_phase;\n\
            per_frame_2=seen_left=rune_section_left; seen_next=rune_next_label; seen_low=rune_ev_low;\n";
        pipeline.load_preset(preset, "p", Transition::Cut).unwrap();
        let mut frame = 0;
        let mut seen = |pipeline: &mut Pipeline, chart: Option<&ChartState>| {
            pipeline.set_chart(chart);
            pipeline.render(FrameClock { time: 0.0, frame }).unwrap();
            frame += 1;
            let vars = pipeline.engine.frame_vars();
            let var = |name: &str| {
                vars.iter()
                    .find(|(n, _)| n == name)
                    .map_or(f64::NAN, |(_, v)| *v)
            };
            [
                "seen_chart",
                "seen_bar",
                "seen_phase",
                "seen_left",
                "seen_next",
                "seen_low",
            ]
            .map(var)
        };
        let mut events = [0.0; 5];
        events[0] = 0.5;
        let chart = ChartState {
            bar: 7.25,
            section_left: 3.5,
            next_label: 2.0,
            events,
            ..Default::default()
        };
        assert_eq!(
            seen(&mut pipeline, Some(&chart)),
            [1.0, 7.0, 0.25, 3.5, 2.0, 0.5]
        );
        assert_eq!(seen(&mut pipeline, None), [0.0; 6]);
    }
}
