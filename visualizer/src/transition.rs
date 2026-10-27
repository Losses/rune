//! Scene switches that are not a cut: the old scene fades to black and the new one fades in, or the
//! old picture dissolves into the new scene. While a transition runs, scenes draw into an
//! intermediate texture and one fullscreen pass (`transition.wgsl`) writes the output from it;
//! when none runs, nothing extra is drawn or held.
//!
//! This module owns the GPU side and the arithmetic of the phases. The [`crate::pipeline`] decides
//! when scenes are installed, since that needs the scene.

use std::cell::Cell;

use crate::covers::{CELL, Cell as CoverCell};
use crate::scene::Scene;

/// The pass and, appended, the cut-in it draws for one of its kinds.
const SHADER: &str = concat!(include_str!("transition.wgsl"), include_str!("cutin.wgsl"));

/// How a scene switch looks. Fades and dissolves are chosen by `crate::switching`; a cut is what
/// [`crate::pipeline::Pipeline::install`] does and only the placeholder of a scene built ahead
/// names it.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// The new scene replaces the old one at once.
    Cut,
    /// The old scene fades to black, then the new one fades in from black.
    Fade,
    /// The old scene's last picture dissolves into the new scene as it draws.
    Dissolve,
    /// A band cuts in across the frozen old picture, shows the cover of the song playing and
    /// tears away to the new scene (see [`CutInStyle`] for the accents).
    CutIn(CutInStyle),
}

/// The accents of a cut-in, picked by the kind of section it lands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CutInStyle {
    /// Speed lines burst from behind the band as it lands, and a glint runs across the cover.
    #[default]
    Streaks,
    /// The band's edges fade from solid to coarse halftone dots.
    Halftone,
    /// Offset translucent copies of the band trail behind it.
    Echo,
}

impl CutInStyle {
    pub const ALL: [CutInStyle; 3] = [Self::Streaks, Self::Halftone, Self::Echo];

    /// The style after this one, round and round (for a scene lab).
    pub fn next(self) -> Self {
        match self {
            Self::Streaks => Self::Halftone,
            Self::Halftone => Self::Echo,
            Self::Echo => Self::Streaks,
        }
    }

    /// A short name to show.
    pub fn name(self) -> &'static str {
        match self {
            Self::Streaks => "speed lines + glint",
            Self::Halftone => "halftone",
            Self::Echo => "echo",
        }
    }

    /// The style of the section with this label: the same kind of section always gets the same
    /// one. Without a chart there are no sections and the default stands.
    pub fn of_section(label: f32, has_chart: bool) -> Self {
        if !has_chart {
            return Self::default();
        }
        Self::ALL[label_hash(label) as usize % Self::ALL.len()]
    }

    /// `style` in `transition.wgsl`.
    fn index(self) -> f32 {
        match self {
            Self::Streaks => 0.0,
            Self::Halftone => 1.0,
            Self::Echo => 2.0,
        }
    }
}

/// A number that is the same for the same section label and looks unrelated for the next one, to
/// pick among things by the kind of section.
pub fn label_hash(label: f32) -> u32 {
    let mut x = label.max(0.0) as u32;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// How long the band takes to open at the start of a cut-in: two frames at 60 fps.
pub const CUTIN_BAND_SECONDS: f64 = 2.0 / 60.0;
/// The shortest the band holds before it tears, in seconds.
pub const CUTIN_MIN_HOLD_SECONDS: f64 = 0.1;

/// How long a transition takes, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Durations {
    /// A fade: the old scene fading out. A dissolve does not use it. A cut-in: the band opening
    /// and holding, up to the moment it starts to tear (so the new scene shows from there).
    pub out: f64,
    /// A fade: the new scene fading in. A dissolve: the whole crossfade. A cut-in: the band
    /// tearing away.
    pub into: f64,
}

impl Durations {
    /// Whether a switch of `transition` with these durations shows anything.
    pub fn is_instant(&self, transition: Transition) -> bool {
        match transition {
            Transition::Cut => true,
            Transition::Fade => self.out <= 0.0 && self.into <= 0.0,
            Transition::Dissolve => self.into <= 0.0,
            Transition::CutIn(_) => self.out <= 0.0 && self.into <= 0.0,
        }
    }
}

/// Where a fade is, `seconds` after its start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FadePhase {
    /// The old scene on its way to black, `progress` of the way (0 to 1).
    Out(f32),
    /// The new scene on its way from black, `progress` of the way.
    In(f32),
    /// Over.
    Done,
}

/// The phase of a fade of `out` seconds out and `into` seconds in, `seconds` after its start.
pub fn fade_phase(seconds: f64, out: f64, into: f64) -> FadePhase {
    let seconds = seconds.max(0.0);
    if seconds < out {
        FadePhase::Out((seconds / out) as f32)
    } else if seconds < out + into {
        FadePhase::In(((seconds - out) / into) as f32)
    } else {
        FadePhase::Done
    }
}

/// How far a dissolve of `duration` seconds is, `seconds` after the new scene went in (0 to 1,
/// 1 once it is over).
pub fn dissolve_progress(seconds: f64, duration: f64) -> f32 {
    if duration <= 0.0 {
        1.0
    } else {
        (seconds / duration).clamp(0.0, 1.0) as f32
    }
}

/// Where a cut-in is, `seconds` after the new scene went in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutInFrame {
    pub style: CutInStyle,
    pub seconds: f32,
    /// How long the band holds after it opened, and how long it takes to tear.
    pub hold: f32,
    pub tear: f32,
}

/// The cut-in of `out` seconds up to the tear and `into` seconds of tearing, `seconds` after the
/// new scene went in; none once it is over.
pub fn cutin_frame(seconds: f64, out: f64, into: f64, style: CutInStyle) -> Option<CutInFrame> {
    let seconds = seconds.max(0.0);
    (seconds < out + into).then(|| CutInFrame {
        style,
        seconds: seconds as f32,
        hold: (out - CUTIN_BAND_SECONDS).max(CUTIN_MIN_HOLD_SECONDS) as f32,
        tear: into as f32,
    })
}

/// What the pass does in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plan {
    /// No transition: the scene draws into the output.
    None,
    /// A fade: the scene's frame `amount` (0 to 1) of the way to black.
    Fade(f32),
    /// A dissolve's first frame: the scene before draws the picture to hold, and it is shown.
    Capture,
    /// A dissolve: `progress` (0 to 1) of the way from the held picture to the scene's frame.
    Dissolve(f32),
    /// A cut-in over the held picture, with the new scene's frame to reveal.
    CutIn(CutInFrame),
}

/// A transition under way.
pub struct Running {
    pub kind: Transition,
    pub durations: Durations,
    /// When it started: the time of its first frame, and for a dissolve the time the new scene
    /// went in.
    pub start: Option<f64>,
    /// The scene to install, until it is.
    pub pending: Option<Scene>,
}

/// The transition pass and the textures it needs while one runs.
pub struct Transitions {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    format: wgpu::TextureFormat,
    /// The cover of the song playing, as the bytes of its cell (not decoded from sRGB, since the
    /// frames are blended as bytes), for the band of a cut-in.
    cover: wgpu::Texture,
    cover_view: wgpu::TextureView,
    has_cover: bool,
    sampler: wgpu::Sampler,
    pub running: Option<Running>,
    targets: Option<Targets>,
    /// Whether the dissolve's picture was drawn into the held texture since it was asked for.
    captured: Cell<bool>,
}

/// The textures of a running transition.
struct Targets {
    size: (u32, u32),
    fresh: (wgpu::Texture, wgpu::TextureView),
    held: Option<(wgpu::Texture, wgpu::TextureView)>,
    group: wgpu::BindGroup,
}

impl Transitions {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let texture = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transition"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(1, texture),
                entry(2, texture),
                entry(
                    3,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transition"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("transition"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("transition"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transition"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cover = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("transition cover"),
            size: wgpu::Extent3d {
                width: CELL,
                height: CELL,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let cover_view = cover.create_view(&Default::default());
        // The cover pans past the edge of its cell while the band holds: mirrored there.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("transition cover"),
            address_mode_u: wgpu::AddressMode::MirrorRepeat,
            address_mode_v: wgpu::AddressMode::MirrorRepeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            uniform,
            format,
            cover,
            cover_view,
            has_cover: false,
            sampler,
            running: None,
            targets: None,
            captured: Cell::new(false),
        }
    }

    /// The cover of the song playing for the band of a cut-in (none: the band shows a crop of the
    /// new scene).
    pub fn set_cover(&mut self, queue: &wgpu::Queue, cell: Option<&CoverCell>) {
        self.has_cover = cell.is_some_and(|cell| cell.has_cover);
        if let Some(cell) = cell.filter(|cell| cell.has_cover) {
            queue.write_texture(
                self.cover.as_image_copy(),
                &cell.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(CELL * 4),
                    rows_per_image: Some(CELL),
                },
                wgpu::Extent3d {
                    width: CELL,
                    height: CELL,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Whether any texture is held (none should be once a transition is over).
    #[cfg(test)]
    pub fn holds_textures(&self) -> bool {
        self.targets.is_some()
    }

    /// Whether the picture of a dissolve was held (see [`Plan::Capture`]).
    pub fn captured(&self) -> bool {
        self.captured.get()
    }

    /// The transition is over, or never was: nothing is held any more.
    pub fn end(&mut self) -> Option<Scene> {
        self.targets = None;
        self.captured.set(false);
        self.running.take().and_then(|running| running.pending)
    }

    /// The textures of `size`, for a frame of `plan`: created when the transition starts (or the
    /// size changed) and dropped when it ends.
    pub fn prepare(&mut self, device: &wgpu::Device, size: (u32, u32), plan: Plan) {
        if plan == Plan::None {
            self.targets = None;
            return;
        }
        let needs_held = matches!(plan, Plan::Capture | Plan::Dissolve(_) | Plan::CutIn(_));
        if let Some(targets) = &self.targets
            && targets.size == size
            && (targets.held.is_some() || !needs_held)
        {
            return;
        }
        let make = |label| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            (texture, view)
        };
        let fresh = make("transition fresh");
        let held = needs_held.then(|| make("transition held"));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&fresh.1),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &held.as_ref().unwrap_or(&fresh).1,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.cover_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.targets = Some(Targets {
            size,
            fresh,
            held,
            group,
        });
    }

    /// Where the scene draws in a frame of `plan`: the output itself without a transition,
    /// else the texture the pass reads (the held one in a dissolve's first frame).
    pub fn scene_target<'a>(
        &'a self,
        plan: Plan,
        output: &'a wgpu::TextureView,
    ) -> &'a wgpu::TextureView {
        let Some(targets) = &self.targets else {
            return output;
        };
        match (plan, &targets.held) {
            (Plan::None, _) => output,
            (Plan::Capture, Some(held)) => {
                self.captured.set(true);
                &held.1
            }
            _ => &targets.fresh.1,
        }
    }

    /// The pass of `plan` into `output`, which writes the frame's end timestamp (query 1) in
    /// `timestamps`, where there is one. Nothing for [`Plan::None`]. `accent` (linear) is the
    /// theme colour that a cut-in draws in.
    pub fn composite(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        plan: Plan,
        accent: Option<[f32; 3]>,
        timestamps: Option<&wgpu::QuerySet>,
    ) {
        let Some(targets) = &self.targets else {
            return;
        };
        let mut params = Params {
            color: [0.0, 0.0, 0.0, 1.0],
            ..Params::default()
        };
        let (kind, progress) = match plan {
            Plan::None => return,
            Plan::Fade(amount) => (KIND_FADE, amount),
            // The held picture is also what `fresh` reads as in this frame; the pass shows it.
            Plan::Capture => (KIND_DISSOLVE, 0.0),
            Plan::Dissolve(progress) => (KIND_DISSOLVE, progress),
            Plan::CutIn(frame) => {
                let [r, g, b] = accent.unwrap_or(FALLBACK_ACCENT).map(encode_srgb);
                params.color = [r, g, b, 1.0];
                params.cutin = [
                    frame.seconds,
                    CUTIN_BAND_SECONDS as f32,
                    frame.hold,
                    frame.tear,
                ];
                params.extra = [
                    frame.style.index(),
                    f32::from(u8::from(self.has_cover)),
                    0.0,
                    0.0,
                ];
                (KIND_CUTIN, 0.0)
            }
        };
        queue.write_buffer(&self.uniform, 0, &params.bytes(kind, progress));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps.map(|query_set| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &targets.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// `kind` in `transition.wgsl`.
const KIND_FADE: u32 = 0;
const KIND_DISSOLVE: u32 = 1;
const KIND_CUTIN: u32 = 2;
/// Bytes of `struct Transition`: the kind and progress, two words of padding, and three vec4f.
const UNIFORM_SIZE: u64 = 64;

/// The theme colour (linear) of a cut-in when the app has not told one: a calm blue.
const FALLBACK_ACCENT: [f32; 3] = [0.08, 0.25, 0.8];

/// A linear value as the byte value (0 to 1) it is displayed as: the frames are blended as bytes.
fn encode_srgb(linear: f32) -> f32 {
    let v = linear.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// What `struct Transition` holds besides the kind and progress.
#[derive(Default)]
struct Params {
    /// The colour a fade goes to, the theme colour of a cut-in (display values).
    color: [f32; 4],
    /// A cut-in: seconds since it started, how long the band opens, holds and tears.
    cutin: [f32; 4],
    /// A cut-in: the style, whether the band shows a cover.
    extra: [f32; 4],
}

impl Params {
    /// The bytes of `struct Transition`.
    fn bytes(&self, kind: u32, progress: f32) -> [u8; UNIFORM_SIZE as usize] {
        let mut bytes = [0; UNIFORM_SIZE as usize];
        bytes[0..4].copy_from_slice(&kind.to_le_bytes());
        bytes[4..8].copy_from_slice(&progress.to_le_bytes());
        for (at, vec) in [(16, self.color), (32, self.cutin), (48, self.extra)] {
            for (i, v) in vec.iter().enumerate() {
                bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_goes_out_then_in_then_is_done() {
        assert_eq!(fade_phase(0.0, 0.5, 1.0), FadePhase::Out(0.0));
        assert_eq!(fade_phase(0.25, 0.5, 1.0), FadePhase::Out(0.5));
        assert_eq!(fade_phase(0.5, 0.5, 1.0), FadePhase::In(0.0));
        assert_eq!(fade_phase(1.0, 0.5, 1.0), FadePhase::In(0.5));
        assert_eq!(fade_phase(1.5, 0.5, 1.0), FadePhase::Done);
        // Before the start counts as the start; no fade-out begins with the fade-in.
        assert_eq!(fade_phase(-1.0, 0.5, 1.0), FadePhase::Out(0.0));
        assert_eq!(fade_phase(0.0, 0.0, 1.0), FadePhase::In(0.0));
        assert_eq!(fade_phase(0.4, 0.4, 0.0), FadePhase::Done);
    }

    #[test]
    fn a_dissolve_runs_from_0_to_1_and_stays() {
        assert_eq!(dissolve_progress(0.0, 2.0), 0.0);
        assert_eq!(dissolve_progress(0.5, 2.0), 0.25);
        assert_eq!(dissolve_progress(2.0, 2.0), 1.0);
        assert_eq!(dissolve_progress(9.0, 2.0), 1.0);
        assert_eq!(dissolve_progress(-1.0, 2.0), 0.0);
        assert_eq!(dissolve_progress(0.0, 0.0), 1.0);
    }

    #[test]
    fn a_transition_without_a_duration_is_a_cut() {
        let none = Durations {
            out: 0.0,
            into: 0.0,
        };
        let short = Durations {
            out: 0.0,
            into: 0.2,
        };
        assert!(none.is_instant(Transition::Fade));
        assert!(!short.is_instant(Transition::Fade));
        assert!(short.is_instant(Transition::Cut));
        assert!(none.is_instant(Transition::Dissolve));
        assert!(!short.is_instant(Transition::Dissolve));
    }

    #[test]
    fn a_cutin_runs_until_its_tear_is_over() {
        let style = CutInStyle::Echo;
        // 0.5 s up to the tear, 0.05 s of tearing: the band opens in the first two frames and
        // holds for the rest of the 0.5 s.
        let frame = cutin_frame(0.25, 0.5, 0.05, style).unwrap();
        assert_eq!(
            (frame.style, frame.seconds, frame.tear),
            (style, 0.25, 0.05)
        );
        assert!((f64::from(frame.hold) - (0.5 - CUTIN_BAND_SECONDS)).abs() < 1e-6);
        assert!(cutin_frame(0.549, 0.5, 0.05, style).is_some());
        assert_eq!(cutin_frame(0.55, 0.5, 0.05, style), None);
        // Before the start counts as the start.
        assert_eq!(cutin_frame(-1.0, 0.5, 0.05, style).unwrap().seconds, 0.0);
        // A cut-in that gives no time for the band to hold still holds a moment.
        let short = cutin_frame(0.0, 0.01, 0.05, style).unwrap();
        assert!(f64::from(short.hold) >= CUTIN_MIN_HOLD_SECONDS);
        let none = Durations {
            out: 0.0,
            into: 0.0,
        };
        assert!(none.is_instant(Transition::CutIn(style)));
        let some = Durations {
            out: 0.5,
            into: 0.05,
        };
        assert!(!some.is_instant(Transition::CutIn(style)));
    }

    #[test]
    fn the_same_kind_of_section_gets_the_same_cutin_style() {
        for label in 0..8 {
            let label = label as f32;
            assert_eq!(
                CutInStyle::of_section(label, true),
                CutInStyle::of_section(label, true)
            );
        }
        // Every style is used by some label, and without a chart there is the default.
        for style in CutInStyle::ALL {
            assert!(
                (0..26).any(|label| CutInStyle::of_section(label as f32, true) == style),
                "{style:?} is never picked"
            );
        }
        assert_eq!(CutInStyle::of_section(5.0, false), CutInStyle::default());
        // The lab goes through all of them and back.
        let mut style = CutInStyle::default();
        for _ in 0..CutInStyle::ALL.len() {
            style = style.next();
        }
        assert_eq!(style, CutInStyle::default());
    }

    #[test]
    fn the_uniform_keeps_each_vector_where_the_shader_reads_it() {
        let params = Params {
            color: [1.0, 2.0, 3.0, 4.0],
            cutin: [5.0, 6.0, 7.0, 8.0],
            extra: [9.0, 10.0, 11.0, 12.0],
        };
        let bytes = params.bytes(KIND_CUTIN, 0.5);
        let word = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            KIND_CUTIN
        );
        assert_eq!(word(4), 0.5);
        assert_eq!((word(16), word(28)), (1.0, 4.0));
        assert_eq!((word(32), word(44)), (5.0, 8.0));
        assert_eq!((word(48), word(60)), (9.0, 12.0));
        assert_eq!(bytes.len() as u64, UNIFORM_SIZE);
    }
}
