//! Scene switches that are not a cut: the old scene fades to black and the new one fades in, or the
//! old picture dissolves into the new scene. While a transition runs, scenes draw into an
//! intermediate texture and one fullscreen pass (`transition.wgsl`) writes the output from it;
//! when none runs, nothing extra is drawn or held.
//!
//! This module owns the GPU side and the arithmetic of the phases. The [`crate::pipeline`] decides
//! when scenes are installed, since that needs the scene.

use std::cell::Cell;

use crate::scene::Scene;

const SHADER: &str = include_str!("transition.wgsl");

/// How a scene switch looks. Only fades are used so far (see `crate::MANUAL_SWITCH`); cuts are
/// what [`crate::pipeline::Pipeline::install`] does and dissolves are tested.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// The new scene replaces the old one at once.
    Cut,
    /// The old scene fades to black, then the new one fades in from black.
    Fade,
    /// The old scene's last picture dissolves into the new scene as it draws.
    Dissolve,
}

/// How long a transition takes, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Durations {
    /// A fade: the old scene fading out. A dissolve does not use it.
    pub out: f64,
    /// A fade: the new scene fading in. A dissolve: the whole crossfade.
    pub into: f64,
}

impl Durations {
    /// Whether a switch of `transition` with these durations shows anything.
    pub fn is_instant(&self, transition: Transition) -> bool {
        match transition {
            Transition::Cut => true,
            Transition::Fade => self.out <= 0.0 && self.into <= 0.0,
            Transition::Dissolve => self.into <= 0.0,
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
        Self {
            pipeline,
            layout,
            uniform,
            format,
            running: None,
            targets: None,
            captured: Cell::new(false),
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
        let needs_held = matches!(plan, Plan::Capture | Plan::Dissolve(_));
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
    /// `timestamps`, where there is one. Nothing for [`Plan::None`].
    pub fn composite(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        plan: Plan,
        timestamps: Option<&wgpu::QuerySet>,
    ) {
        let Some(targets) = &self.targets else {
            return;
        };
        let (kind, progress) = match plan {
            Plan::None => return,
            Plan::Fade(amount) => (KIND_FADE, amount),
            // The held picture is also what `fresh` reads as in this frame; the pass shows it.
            Plan::Capture => (KIND_DISSOLVE, 0.0),
            Plan::Dissolve(progress) => (KIND_DISSOLVE, progress),
        };
        queue.write_buffer(
            &self.uniform,
            0,
            &uniform(kind, progress, [0.0, 0.0, 0.0, 1.0]),
        );
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
/// Bytes of `struct Transition`: the kind and progress, two words of padding, a colour.
const UNIFORM_SIZE: u64 = 32;

/// The bytes of `struct Transition`.
fn uniform(kind: u32, progress: f32, color: [f32; 4]) -> [u8; UNIFORM_SIZE as usize] {
    let mut bytes = [0; UNIFORM_SIZE as usize];
    bytes[0..4].copy_from_slice(&kind.to_le_bytes());
    bytes[4..8].copy_from_slice(&progress.to_le_bytes());
    for (i, v) in color.iter().enumerate() {
        bytes[16 + 4 * i..20 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    bytes
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
}
