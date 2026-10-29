//! Scenes in Rune's own format (`docs/scenes.md`): a folder with
//! `scene.toml` and WGSL files. The scene's `update()` runs once per frame on the GPU, its passes
//! draw into HDR buffers, and the runtime's post-processing makes the frame.

mod audio;
mod director;
mod inputs;
mod lens;
mod looks;
pub use inputs::MusicValues;
mod manifest;
mod noise;
mod post;
mod signals;
mod wgsl;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::adaptive::Level;
use crate::chart::ChartState;
use crate::covers::{self, Atlas, Cell};
use crate::rhythm::Rhythm;
use audio::{Analyzer, SPECTRUM_ROWS, SPECTRUM_WIDTH, WAVEFORM_WIDTH};
use director::Director;
pub use director::DirectorView;
use inputs::EventCounter;
pub use lens::LensView;
use looks::Looks;
pub use looks::LooksView;
pub use manifest::{Bind, Effects, FallbackDecl, InputDecl, Pace, Param, ParamKind};
use manifest::{Format, Manifest, OUT, PREV, ShotMove, ShotSize, Value};
use wgsl::{InputLayout, ParamsLayout};

pub use manifest::{MANIFEST, argb_to_linear};

/// A camera cut the director made in a frame, for the transition layer to dress later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutInfo {
    /// The size and the move of the shot cut to.
    pub size: ShotSize,
    pub motion: ShotMove,
    /// The strength of the music's cue in that frame (0 to 1): a cut on a section start is 0.5
    /// or 1, one on the beat or the cadence is whatever the cue's envelope has left.
    pub cue: f32,
    /// 0 for a normal cut, else the position in a burst (1 to 4, see `ShotState::fill`).
    pub fill: u32,
    /// The transition the moment of the cut dresses it with, if the cut is hard and at a moment
    /// that plays an effect and the scene lists one that suits it (see [`crate::moments`]); the
    /// cut a burst leads to always has one. A glide (`motion`) has none.
    pub dress: Option<crate::transition::Transition>,
    /// The cut lands on a strong transition of the scene's list (see `director::Cut::strong`).
    pub strong: bool,
}

/// Buffers of a scene may take this much memory together.
const MAX_BUFFER_BYTES: u64 = 256 << 20;
/// The format of `out`, which the post-processing reads.
const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Whether `path` is a scene's folder.
pub fn is_scene(path: &Path) -> bool {
    path.join(MANIFEST).is_file()
}

/// The texts a scene is made of: (file name, contents).
#[derive(Clone, PartialEq)]
struct Files {
    manifest: Manifest,
    state: Option<(String, String)>,
    update: Option<(String, String)>,
    common: Vec<(String, String)>,
    shaders: Vec<(String, String)>,
}

// A scene is built on a thread of its own and drawn on the render thread (see
// `crate::loading`).
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<Scene>();
};

/// Where a scene's files are: a folder, or a fixed list of (path, contents) pairs compiled into
/// the binary (files may be images), which never changes and so is never reloaded.
#[derive(Debug, Clone)]
pub enum Source {
    Dir(PathBuf),
    Embedded(&'static [(&'static str, &'static [u8])]),
}

impl Source {
    fn read(&self, name: &str) -> Result<Vec<u8>> {
        match self {
            Self::Dir(dir) => std::fs::read(dir.join(name)),
            Self::Embedded(files) => files
                .iter()
                .find(|(path, _)| *path == name)
                .map(|(_, bytes)| bytes.to_vec())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound)),
        }
        .with_context(|| format!("Failed to read {name}"))
    }

    /// The `name` in the scene's manifest.
    pub fn scene_name(&self) -> Result<String> {
        let (_, text) = self.read_text(MANIFEST)?;
        Ok(Manifest::parse(&text).context(MANIFEST)?.name)
    }

    /// The `source` of each of the scene's `[[asset]]`s that says where it comes from.
    pub fn asset_sources(&self) -> Result<Vec<String>> {
        let (_, text) = self.read_text(MANIFEST)?;
        let manifest = Manifest::parse(&text).context(MANIFEST)?;
        Ok(manifest
            .assets
            .into_iter()
            .filter_map(|asset| asset.source)
            .collect())
    }

    fn read_text(&self, name: &str) -> Result<(String, String)> {
        let text = String::from_utf8(self.read(name)?)
            .with_context(|| format!("{name} is not UTF-8 text"))?;
        Ok((name.to_string(), text))
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dir(dir) => dir.display().fmt(f),
            Self::Embedded(_) => f.write_str("(built in)"),
        }
    }
}

impl Files {
    fn read(source: &Source) -> Result<Self> {
        let (_, text) = source.read_text(MANIFEST)?;
        let manifest = Manifest::parse(&text).context(MANIFEST)?;
        let state = match &manifest.state {
            Some(s) => Some(source.read_text(&s.file)?),
            None => None,
        };
        let update = match manifest.state.as_ref().and_then(|s| s.update.as_ref()) {
            Some(file) => Some(source.read_text(file)?),
            None => None,
        };
        let common = manifest
            .common
            .iter()
            .map(|file| source.read_text(file))
            .collect::<Result<_>>()?;
        let shaders = manifest
            .passes
            .iter()
            .map(|p| source.read_text(&p.shader))
            .collect::<Result<_>>()?;
        Ok(Self {
            manifest,
            state,
            update,
            common,
            shaders,
        })
    }

    /// Whether `other` differs from these only in parameter defaults.
    fn same_but_defaults(&self, other: &Self) -> bool {
        self.manifest.without_defaults() == other.manifest.without_defaults()
            && self.state == other.state
            && self.update == other.update
            && self.common == other.common
            && self.shaders == other.shaders
    }

    fn state(&self) -> (&str, &str) {
        match &self.state {
            Some((name, text)) => (name, text),
            None => ("<runtime>", wgsl::state_text(None)),
        }
    }

    fn param_values(&self) -> Vec<Value> {
        // Checked when the manifest was parsed.
        self.manifest
            .params
            .iter()
            .map(|p| p.value().unwrap_or(Value::Float(0.0)))
            .collect()
    }

    /// The values of the parameters a bind can read (`{ param = "id" }`): the numeric ones as
    /// they are now, 0 for the rest, in the manifest's order.
    fn param_numbers(&self) -> Vec<f32> {
        self.param_values()
            .iter()
            .map(|value| match *value {
                Value::Float(v) => v,
                Value::Int(v) => v as f32,
                _ => 0.0,
            })
            .collect()
    }
}

/// What the scene hears this frame.
pub struct FrameInput<'a> {
    /// The visualizer's clock (it stops while paused), in seconds.
    pub time: f64,
    pub rhythm: &'a Rhythm,
    pub chart: Option<&'a ChartState>,
    /// The song time and whether it plays.
    pub song: (f64, bool),
    /// The app's accent colour (linear), once the app has told it.
    pub accent: Option<[f32; 3]>,
}

/// A buffer of the manifest: one texture, or two taking turns when it is persistent.
struct Target {
    spec: manifest::Buffer,
    /// Per texture: the view drawn into (mip 0) and the one sampled (all mips).
    views: Vec<(wgpu::TextureView, wgpu::TextureView)>,
    /// Per texture, with mips: one view per mip level.
    mip_views: Vec<Vec<wgpu::TextureView>>,
    current: usize,
    _textures: Vec<wgpu::Texture>,
}

enum Input {
    Current(usize),
    Prev(usize),
}

struct Pass {
    /// The shader's file name, which labels the pass (in GPU traces).
    label: String,
    pipeline: wgpu::RenderPipeline,
    inputs_layout: wgpu::BindGroupLayout,
    inputs: Vec<Input>,
    /// `None`: `out`.
    target: Option<usize>,
}

/// Everything that depends on the size.
struct Sized {
    size: (u32, u32),
    targets: Vec<Target>,
    out: (wgpu::Texture, wgpu::TextureView),
    post: post::Post,
}

/// Mip chains: each level drawn from the one above with a linear sampler (a 2x2 box). Also copies
/// a texture into one of another size (a buffer that carries over a change of resolution).
struct Mipper {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// For formats that cannot be filtered: the nearest texel, without a sampler.
    load_layout: wgpu::BindGroupLayout,
    load_pipeline_layout: wgpu::PipelineLayout,
    load_pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

const MIP_SHADER: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
struct V { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vertex(@builtin(vertex_index) i: u32) -> V {
  let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return V(vec4f(xy * 2.0 - 1.0, 0.0, 1.0), vec2f(xy.x, 1.0 - xy.y));
}
@fragment fn fragment(v: V) -> @location(0) vec4f {
  return textureSampleLevel(source, smp, v.uv, 0.0);
}
@fragment fn fragment_load(v: V) -> @location(0) vec4f {
  let size = vec2f(textureDimensions(source));
  return textureLoad(source, vec2i(min(v.uv * size, size - 1.0)), 0);
}
";

impl Mipper {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene mips"),
            source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene mips"),
            entries: &[
                texture_entry(0, wgpu::ShaderStages::FRAGMENT, true),
                sampler_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene mips"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = linear_sampler(device, wgpu::AddressMode::ClampToEdge);
        let load_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene copy"),
            entries: &[texture_entry(0, wgpu::ShaderStages::FRAGMENT, false)],
        });
        let load_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene copy"),
            bind_group_layouts: &[Some(&load_layout)],
            immediate_size: 0,
        });
        Self {
            module,
            layout,
            pipeline_layout,
            sampler,
            pipelines: Vec::new(),
            load_layout,
            load_pipeline_layout,
            load_pipelines: Vec::new(),
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> usize {
        if let Some(i) = self.pipelines.iter().position(|(f, _)| *f == format) {
            return i;
        }
        let pipeline = fullscreen_pipeline(
            device,
            "scene mips",
            &self.pipeline_layout,
            &self.module,
            ("vertex", "fragment"),
            format,
        );
        self.pipelines.push((format, pipeline));
        self.pipelines.len() - 1
    }

    fn load_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> usize {
        if let Some(i) = self.load_pipelines.iter().position(|(f, _)| *f == format) {
            return i;
        }
        let pipeline = fullscreen_pipeline(
            device,
            "scene copy",
            &self.load_pipeline_layout,
            &self.module,
            ("vertex", "fragment_load"),
            format,
        );
        self.load_pipelines.push((format, pipeline));
        self.load_pipelines.len() - 1
    }

    /// Draw `source` (its first mip) stretched over `target`, both in `format`: filtered, or the
    /// nearest texel for a format that cannot be filtered.
    fn copy(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        format: wgpu::TextureFormat,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let filterable = format != wgpu::TextureFormat::Rgba32Float;
        let source = wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(source),
        };
        let (pipeline, group) = if filterable {
            let i = self.pipeline(device, format);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene copy"),
                layout: &self.layout,
                entries: &[
                    source,
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            (&self.pipelines[i].1, group)
        } else {
            let i = self.load_pipeline(device, format);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene copy"),
                layout: &self.load_layout,
                entries: &[source],
            });
            (&self.load_pipelines[i].1, group)
        };
        let mut pass = begin_pass(encoder, target, "scene copy", None);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn generate(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        format: wgpu::TextureFormat,
        levels: &[wgpu::TextureView],
    ) {
        let i = self.pipeline(device, format);
        for pair in levels.windows(2) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene mips"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&pair[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = begin_pass(encoder, &pair[1], "scene mips", None);
            pass.set_pipeline(&self.pipelines[i].1);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

pub struct Scene {
    pub name: String,
    source: Source,
    files: Files,
    /// The files as they were last read from the source. A lab that replaces the bindings changes
    /// `files` and not this, so that the scene is reloaded when the folder changes, not because
    /// the scene on screen differs from it.
    on_disk: Files,
    params: ParamsLayout,
    frame: wgpu::Buffer,
    music: wgpu::Buffer,
    /// `struct Shot`, the camera director's word to the scene.
    shot: wgpu::Buffer,
    /// `struct Look`, the runtime's word on the scene's look.
    look: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    /// `struct Inputs`, the values the scene's `[[bind]]`s drive, and what drives them.
    input_layout: InputLayout,
    input_buffer: wgpu::Buffer,
    bindings: crate::bind::Bindings,
    state: wgpu::Buffer,
    state_size: u64,
    post_buffer: wgpu::Buffer,
    post_defaults: Vec<u8>,
    spectrum: wgpu::Texture,
    waveform: wgpu::Texture,
    noise: wgpu::TextureView,
    /// The covers atlas (mip views of it) if the scene asked for one, else a 1x1 placeholder.
    covers: Option<(wgpu::Texture, Vec<wgpu::TextureView>)>,
    /// The atlas last given, kept for a rebuild of the scene, and how many of its cells are valid.
    covers_atlas: Option<Arc<Atlas>>,
    /// The cell of the track playing last given, which stays over cell 0 of any atlas (none: the
    /// atlas's own cell 0 stands).
    covers_playing: Option<Arc<Cell>>,
    covers_count: u32,
    pass_globals: wgpu::BindGroup,
    update: Option<(wgpu::ComputePipeline, wgpu::BindGroup)>,
    passes: Vec<Pass>,
    sized: Sized,
    mipper: Mipper,
    output_format: wgpu::TextureFormat,
    output_size: (u32, u32),
    /// The resolution factor (against the scene's own scale) and quality it draws with.
    level: Level,
    analyzer: Analyzer,
    events: EventCounter,
    /// The values `struct Music` got for the last frame.
    music_values: inputs::MusicValues,
    /// Decides the camera cuts, if the scene declares shots.
    director: Option<Director>,
    /// Decides the look changes, if the scene declares looks.
    looks: Option<Looks>,
    /// The shot a scene lab holds (see [`Scene::hold_shot`]), kept to hold it again when the
    /// scene is rebuilt from its saved files.
    held_shot: Option<u32>,
    /// The transitions a switch to the scene may take, by name (its `effects.switch`).
    switch_transitions: Vec<String>,
    /// The transitions that may dress a cut of the director, by name (its `effects.cut`).
    cut_transitions: Vec<String>,
    /// The cut the director made in the last frame.
    cut: Option<CutInfo>,
    /// A cut that [`Scene::cut_now`] made, to be reported with the next frame.
    forced_cut: Option<CutInfo>,
    /// The lens effects the scene listed (`effects.lens`) and when each is on.
    lens: lens::Lens,
    /// The glitch or mosaic over the picture that started in the last frame.
    overlay: Option<crate::transition::Overlay>,
    /// The moments of the music that play an effect, and their chances (see [`crate::moments`]).
    moments: crate::moments::Scheduler,
    chances: crate::moments::Chances,
    /// The visualizer's time when the scene started, and of the last frame.
    started: Option<f64>,
    last: Option<f64>,
    /// The song time of the last frame.
    last_song: f64,
    index: u32,
    first: bool,
    /// Whether the bindings heard the director's cut in the last frame (`cut`): a cut, or the
    /// scene's first frame when it has a director.
    bound_cut: bool,
    /// Tests only: each frame's seed comes from this and the frame's index instead of at random,
    /// so that the same timeline draws the same pictures (see [`Scene::fix_seed`]).
    #[cfg(test)]
    fixed_seed: Option<u32>,
    _keep: Vec<wgpu::Texture>,
}

fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn linear_sampler(device: &wgpu::Device, address: wgpu::AddressMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("scene"),
        address_mode_u: address,
        address_mode_v: address,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    (vertex, fragment): (&str, &str),
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
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
    })
}

fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    label: &str,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// The timestamps a render pass writes into `set`: the frame's beginning (query 0) at its
/// beginning if `begin`, the frame's end (query 1) at its end if `end`.
fn timestamp_writes(
    set: Option<&wgpu::QuerySet>,
    begin: bool,
    end: bool,
) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
    set.filter(|_| begin || end)
        .map(|query_set| wgpu::RenderPassTimestampWrites {
            query_set,
            beginning_of_pass_write_index: begin.then_some(0),
            end_of_pass_write_index: end.then_some(1),
        })
}

/// `size` times `scale`, as the shaders compute it: `floor(resolution * scale)`, at least 1.
fn scaled((w, h): (u32, u32), scale: f32) -> (u32, u32) {
    (
        ((w as f32 * scale).floor() as u32).max(1),
        ((h as f32 * scale).floor() as u32).max(1),
    )
}

fn plain_texture(
    device: &wgpu::Device,
    label: &str,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    mips: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn write_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &[u8], bytes_per_pixel: u32) {
    let size = texture.size();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * bytes_per_pixel),
            rows_per_image: Some(size.height),
        },
        size,
    );
}

/// Write a cell into cell 0 (mip 0, the top left corner) of the covers atlas.
fn write_cell(queue: &wgpu::Queue, texture: &wgpu::Texture, cell: &Cell) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &cell.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(covers::CELL * 4),
            rows_per_image: Some(covers::CELL),
        },
        wgpu::Extent3d {
            width: covers::CELL,
            height: covers::CELL,
            depth_or_array_layers: 1,
        },
    );
}

fn f16_bytes(values: impl Iterator<Item = f32>) -> Vec<u8> {
    values
        .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
        .collect()
}

/// The memory the buffers of `manifest` take at the internal size `size`.
fn buffer_bytes(manifest: &Manifest, size: (u32, u32)) -> u64 {
    manifest
        .buffers
        .iter()
        .map(|spec| {
            let (w, h) = scaled(size, spec.scale);
            u64::from(w)
                * u64::from(h)
                * spec.format.bytes_per_pixel()
                * if spec.persistent { 2 } else { 1 }
                * if spec.mips { 4 } else { 3 }
                / 3
        })
        .sum()
}

impl Sized {
    /// At `factor` of the scene's own scale. The post-processing's pipelines carry over from
    /// `post`, if given (they are built only when the number of bloom levels changes).
    fn new(
        device: &wgpu::Device,
        manifest: &Manifest,
        (output_format, output_size): (wgpu::TextureFormat, (u32, u32)),
        factor: f32,
        buffers: (&wgpu::Buffer, &wgpu::Buffer),
        post: Option<&post::Post>,
    ) -> Result<Self> {
        // Checked at the scene's own scale, so that a lower factor can always go back up.
        let total = buffer_bytes(manifest, scaled(output_size, manifest.render.scale));
        ensure!(
            total <= MAX_BUFFER_BYTES,
            "the buffers need {} MB at this size, more than {} MB",
            total >> 20,
            MAX_BUFFER_BYTES >> 20
        );
        let size = scaled(output_size, manifest.render.scale * factor);
        let mut targets = Vec::new();
        for spec in &manifest.buffers {
            let (w, h) = scaled(size, spec.scale);
            let mips = if spec.mips {
                32 - w.max(h).leading_zeros()
            } else {
                1
            };
            let count = if spec.persistent { 2 } else { 1 };
            let mut textures = Vec::new();
            let mut views = Vec::new();
            let mut mip_views = Vec::new();
            for _ in 0..count {
                let texture = plain_texture(
                    device,
                    &spec.id,
                    (w, h),
                    spec.format.wgpu(),
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    mips,
                );
                let level = |i| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        base_mip_level: i,
                        mip_level_count: Some(1),
                        ..Default::default()
                    })
                };
                views.push((level(0), texture.create_view(&Default::default())));
                mip_views.push(if spec.mips {
                    (0..mips).map(level).collect()
                } else {
                    Vec::new()
                });
                textures.push(texture);
            }
            targets.push(Target {
                spec: spec.clone(),
                views,
                mip_views,
                current: 0,
                _textures: textures,
            });
        }
        let out = plain_texture(
            device,
            "scene out",
            size,
            OUT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            1,
        );
        let out_view = out.create_view(&Default::default());
        let post = match post {
            Some(post) => post.resized(device, size),
            None => post::Post::new(device, manifest.post.tonemap, output_format, buffers, size),
        };
        Ok(Self {
            size,
            targets,
            out: (out, out_view),
            post,
        })
    }
}

/// The default `struct Post` from the manifest.
fn post_defaults(manifest: &Manifest) -> Vec<u8> {
    let p = &manifest.post;
    let mut bytes = Vec::with_capacity(wgsl::POST_SIZE as usize);
    for v in [
        p.exposure,
        p.bloom.intensity,
        p.bloom.threshold,
        p.bloom.radius,
        p.chromatic,
        p.vignette,
        p.grain,
        0.0,
        1.0,
        1.0,
        1.0,
        1.0,
        p.contrast,
        p.saturation,
        0.0,
        0.0,
    ] {
        bytes.extend(v.to_le_bytes());
    }
    bytes
}

impl Scene {
    /// Build the scene in `dir`, drawing frames of `output_size` in `output_format`.
    #[cfg(test)]
    pub fn load(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dir: &Path,
        output_format: wgpu::TextureFormat,
        output_size: (u32, u32),
        sample_rate: u32,
    ) -> Result<Self> {
        Self::load_from(
            device,
            queue,
            Source::Dir(dir.to_path_buf()),
            output_format,
            output_size,
            sample_rate,
        )
    }

    /// Build the scene `source` holds, drawing frames of `output_size` in `output_format`.
    pub fn load_from(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: Source,
        output_format: wgpu::TextureFormat,
        output_size: (u32, u32),
        sample_rate: u32,
    ) -> Result<Self> {
        let files = Files::read(&source)?;
        Self::build(
            device,
            queue,
            source,
            files,
            output_format,
            output_size,
            sample_rate,
        )
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: Source,
        files: Files,
        output_format: wgpu::TextureFormat,
        output_size: (u32, u32),
        sample_rate: u32,
    ) -> Result<Self> {
        let manifest = &files.manifest;
        for buffer in &manifest.buffers {
            ensure!(
                !(buffer.mips && buffer.format == Format::Rgba32float),
                "{}: rgba32float cannot be filtered, so it cannot have mips",
                buffer.id
            );
        }
        let state = files.state();

        // Check every module first: errors name the author's file and line.
        let update_module = match &files.update {
            Some((name, text)) => {
                let source = wgsl::update_source(manifest, state, (name, text));
                let module = source.check()?;
                Some((source.text, module))
            }
            None => None,
        };
        let mut pass_sources = Vec::new();
        for (pass, (name, text)) in manifest.passes.iter().zip(&files.shaders) {
            let scale = match manifest.buffers.iter().find(|b| b.id == pass.target) {
                Some(b) => b.scale,
                None => 1.0,
            };
            let source = wgsl::pass_source(
                manifest,
                state,
                &files.common,
                &pass.inputs,
                (name, text),
                scale,
            );
            let module = source.check()?;
            pass_sources.push((source.text, module));
        }
        let state_size = match (&update_module, pass_sources.first()) {
            (Some((_, module)), _) | (None, Some((_, module))) => wgsl::state_size(module)?,
            (None, None) => 16,
        };

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let params = ParamsLayout::new(manifest);
        let frame = uniform("scene frame", wgsl::FRAME_SIZE);
        let music = uniform(
            "scene music",
            inputs::MusicValues::default().bytes().len() as u64,
        );
        let shot = uniform("scene shot", wgsl::SHOT_SIZE);
        let look = uniform("scene look", wgsl::LOOK_SIZE);
        let params_buffer = uniform("scene params", params.size);
        queue.write_buffer(&params_buffer, 0, &params.pack(&files.param_values()));
        let input_layout = InputLayout::new(manifest);
        let input_buffer = uniform("scene inputs", input_layout.size);
        let bindings =
            crate::bind::Bindings::with_scope(&manifest.inputs, &manifest.params, &manifest.binds)?;
        queue.write_buffer(&input_buffer, 0, &input_layout.pack(bindings.values()));
        let storage = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let state_buffer = storage("scene state", state_size);
        let post_buffer = storage("scene post", wgsl::POST_SIZE);
        let post_defaults = post_defaults(manifest);
        queue.write_buffer(&post_buffer, 0, &post_defaults);

        let spectrum = plain_texture(
            device,
            "scene spectrum",
            (SPECTRUM_WIDTH as u32, SPECTRUM_ROWS as u32),
            wgpu::TextureFormat::R16Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            1,
        );
        let waveform = plain_texture(
            device,
            "scene waveform",
            (WAVEFORM_WIDTH as u32, 1),
            wgpu::TextureFormat::Rg16Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            1,
        );
        let noise_texture = plain_texture(
            device,
            "scene blue noise",
            (noise::SIZE as u32, noise::SIZE as u32),
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            1,
        );
        write_texture(queue, &noise_texture, noise::rgba(), 4);
        let mut keep = vec![noise_texture.clone()];
        // The covers: an atlas that fills in later if the scene asked for it, else one clear texel.
        let covers_size = if manifest.covers { covers::SIZE } else { 1 };
        let covers_mips = 32 - covers_size.leading_zeros();
        let covers_texture = plain_texture(
            device,
            "scene covers",
            (covers_size, covers_size),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            covers_mips,
        );
        let covers_levels: Vec<wgpu::TextureView> = (0..covers_mips)
            .map(|i| {
                covers_texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let covers_view = covers_texture.create_view(&Default::default());
        let mut assets = Vec::new();
        let mut mipper = Mipper::new(device);
        for asset in &manifest.assets {
            let bytes = source.read(&asset.file)?;
            let image = image::load_from_memory(&bytes)
                .with_context(|| format!("Failed to decode {}", asset.file))?
                .to_rgba8();
            let format = if asset.linear {
                wgpu::TextureFormat::Rgba8Unorm
            } else {
                wgpu::TextureFormat::Rgba8UnormSrgb
            };
            let (w, h) = image.dimensions();
            let mips = 32 - w.max(h).leading_zeros();
            let texture = plain_texture(
                device,
                &asset.id,
                (w, h),
                format,
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                mips,
            );
            write_texture(queue, &texture, image.as_raw(), 4);
            // Mips, so an image seen small does not shimmer.
            let levels: Vec<wgpu::TextureView> = (0..mips)
                .map(|i| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        base_mip_level: i,
                        mip_level_count: Some(1),
                        ..Default::default()
                    })
                })
                .collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            mipper.generate(device, &mut encoder, format, &levels);
            queue.submit(Some(encoder.finish()));
            assets.push(texture.create_view(&Default::default()));
            keep.push(texture);
        }
        let views = (
            spectrum.create_view(&Default::default()),
            waveform.create_view(&Default::default()),
            noise_texture.create_view(&Default::default()),
        );
        let samplers = [
            linear_sampler(device, wgpu::AddressMode::ClampToEdge),
            linear_sampler(device, wgpu::AddressMode::Repeat),
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("scene nearest"),
                ..Default::default()
            }),
        ];

        // Group 0: what every module sees; the update also writes the state and Post.
        let globals = |stage: wgsl::Stage| {
            let visibility = match stage {
                wgsl::Stage::Update => wgpu::ShaderStages::COMPUTE,
                wgsl::Stage::Pass => wgpu::ShaderStages::FRAGMENT,
            };
            let uniform = wgpu::BufferBindingType::Uniform;
            let mut entries = vec![
                buffer_entry(wgsl::BIND_FRAME, visibility, uniform),
                buffer_entry(wgsl::BIND_MUSIC, visibility, uniform),
                buffer_entry(wgsl::BIND_PARAMS, visibility, uniform),
                buffer_entry(wgsl::BIND_SHOT, visibility, uniform),
                buffer_entry(wgsl::BIND_LOOK, visibility, uniform),
                buffer_entry(wgsl::BIND_INPUT, visibility, uniform),
                buffer_entry(
                    wgsl::BIND_STATE,
                    visibility,
                    wgpu::BufferBindingType::Storage {
                        read_only: stage == wgsl::Stage::Pass,
                    },
                ),
                texture_entry(wgsl::BIND_SPECTRUM, visibility, true),
                texture_entry(wgsl::BIND_WAVEFORM, visibility, true),
                texture_entry(wgsl::BIND_BLUE_NOISE, visibility, true),
                texture_entry(wgsl::BIND_COVERS, visibility, true),
                sampler_entry(wgsl::BIND_LINEAR_CLAMP, visibility),
                sampler_entry(wgsl::BIND_LINEAR_REPEAT, visibility),
                sampler_entry(wgsl::BIND_NEAREST_CLAMP, visibility),
            ];
            if stage == wgsl::Stage::Update {
                entries.push(buffer_entry(
                    wgsl::BIND_POST,
                    visibility,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ));
            }
            for i in 0..assets.len() {
                entries.push(texture_entry(
                    wgsl::BIND_ASSETS + i as u32,
                    visibility,
                    true,
                ));
            }
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene globals"),
                entries: &entries,
            });
            let mut resources = vec![
                (wgsl::BIND_FRAME, frame.as_entire_binding()),
                (wgsl::BIND_MUSIC, music.as_entire_binding()),
                (wgsl::BIND_PARAMS, params_buffer.as_entire_binding()),
                (wgsl::BIND_SHOT, shot.as_entire_binding()),
                (wgsl::BIND_LOOK, look.as_entire_binding()),
                (wgsl::BIND_INPUT, input_buffer.as_entire_binding()),
                (wgsl::BIND_STATE, state_buffer.as_entire_binding()),
                (
                    wgsl::BIND_SPECTRUM,
                    wgpu::BindingResource::TextureView(&views.0),
                ),
                (
                    wgsl::BIND_WAVEFORM,
                    wgpu::BindingResource::TextureView(&views.1),
                ),
                (
                    wgsl::BIND_BLUE_NOISE,
                    wgpu::BindingResource::TextureView(&views.2),
                ),
                (
                    wgsl::BIND_COVERS,
                    wgpu::BindingResource::TextureView(&covers_view),
                ),
                (
                    wgsl::BIND_LINEAR_CLAMP,
                    wgpu::BindingResource::Sampler(&samplers[0]),
                ),
                (
                    wgsl::BIND_LINEAR_REPEAT,
                    wgpu::BindingResource::Sampler(&samplers[1]),
                ),
                (
                    wgsl::BIND_NEAREST_CLAMP,
                    wgpu::BindingResource::Sampler(&samplers[2]),
                ),
            ];
            if stage == wgsl::Stage::Update {
                resources.push((wgsl::BIND_POST, post_buffer.as_entire_binding()));
            }
            for (i, view) in assets.iter().enumerate() {
                resources.push((
                    wgsl::BIND_ASSETS + i as u32,
                    wgpu::BindingResource::TextureView(view),
                ));
            }
            let entries: Vec<wgpu::BindGroupEntry> = resources
                .into_iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
                .collect();
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene globals"),
                layout: &layout,
                entries: &entries,
            });
            (layout, group)
        };

        let update = match update_module {
            Some((text, _)) => {
                let (layout, group) = globals(wgsl::Stage::Update);
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("scene update"),
                    source: wgpu::ShaderSource::Wgsl(Cow::Owned(text)),
                });
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("scene update"),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("scene update"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("rv_update"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                Some((pipeline, group))
            }
            None => None,
        };

        let (pass_layout, pass_globals) = globals(wgsl::Stage::Pass);
        let target_index = |id: &str| manifest.buffers.iter().position(|b| b.id == id);
        let mut passes = Vec::new();
        for (pass, (text, _)) in manifest.passes.iter().zip(pass_sources) {
            let mut inputs = Vec::new();
            let mut entries = Vec::new();
            for (i, input) in pass.inputs.iter().enumerate() {
                let (index, prev) = match input.strip_prefix(PREV) {
                    Some(id) => (target_index(id), true),
                    None => (target_index(input), false),
                };
                let index = index.ok_or_else(|| anyhow!("no buffer {input}"))?;
                let filterable = manifest.buffers[index].format != Format::Rgba32float;
                entries.push(texture_entry(
                    i as u32,
                    wgpu::ShaderStages::FRAGMENT,
                    filterable,
                ));
                inputs.push(if prev {
                    Input::Prev(index)
                } else {
                    Input::Current(index)
                });
            }
            let inputs_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene pass inputs"),
                entries: &entries,
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&pass.shader),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(text)),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&pass.shader),
                bind_group_layouts: &[Some(&pass_layout), Some(&inputs_layout)],
                immediate_size: 0,
            });
            let target = if pass.target == OUT {
                None
            } else {
                target_index(&pass.target)
            };
            let format = target.map_or(OUT_FORMAT, |i| manifest.buffers[i].format.wgpu());
            let pipeline = fullscreen_pipeline(
                device,
                &pass.shader,
                &pipeline_layout,
                &module,
                ("rv_vertex", "rv_fragment"),
                format,
            );
            passes.push(Pass {
                label: format!("scene pass {}", pass.shader),
                pipeline,
                inputs_layout,
                inputs,
                target,
            });
        }
        let sized = Sized::new(
            device,
            manifest,
            (output_format, output_size),
            1.0,
            (&post_buffer, &frame),
            None,
        )?;
        if let Some(error) = pollster::block_on(scope.pop()) {
            bail!("{error}");
        }
        keep.push(spectrum.clone());
        Ok(Self {
            name: manifest.name.clone(),
            source,
            params,
            frame,
            music,
            shot,
            look,
            params_buffer,
            input_layout,
            input_buffer,
            bindings,
            state: state_buffer,
            state_size,
            post_buffer,
            post_defaults,
            spectrum,
            waveform,
            noise: views.2,
            covers: manifest.covers.then_some((covers_texture, covers_levels)),
            covers_atlas: None,
            covers_playing: None,
            covers_count: 0,
            pass_globals,
            update,
            passes,
            sized,
            mipper,
            output_format,
            output_size,
            level: Level::FULL,
            analyzer: Analyzer::new(sample_rate),
            events: EventCounter::default(),
            music_values: inputs::MusicValues::default(),
            director: Director::for_manifest(manifest),
            looks: Looks::for_manifest(manifest),
            held_shot: None,
            switch_transitions: manifest.effects.switch.clone(),
            cut_transitions: manifest.effects.cut.clone(),
            cut: None,
            forced_cut: None,
            lens: lens::Lens::new(&manifest.effects.lens),
            overlay: None,
            moments: crate::moments::Scheduler::default(),
            chances: crate::moments::Chances::default(),
            started: None,
            last: None,
            last_song: 0.0,
            index: 0,
            first: true,
            bound_cut: false,
            on_disk: files.clone(),
            files,
            #[cfg(test)]
            fixed_seed: None,
            _keep: keep,
        })
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    /// The size of the frames it draws.
    pub fn output_size(&self) -> (u32, u32) {
        self.output_size
    }

    /// Take over what this scene has heard of the music from `previous`, the scene it replaces:
    /// the analyzer's state and the counters and signals of the events, so that a switch of
    /// scenes does not restart the music's clock, cue gaps and drive (as [`Scene::reload`] does
    /// for a new version of a scene). Only if both were built for the same sample rate; else
    /// this scene keeps its own.
    pub fn take_music_from(&mut self, previous: &mut Scene) {
        if self.analyzer.sample_rate() != previous.analyzer.sample_rate() {
            return;
        }
        std::mem::swap(&mut self.analyzer, &mut previous.analyzer);
        std::mem::swap(&mut self.events, &mut previous.events);
        self.music_values = previous.music_values;
        self.last_song = previous.last_song;
    }

    /// The inputs the scene declares.
    pub fn inputs(&self) -> Vec<crate::inspect::InspectInput> {
        self.files
            .manifest
            .inputs
            .iter()
            .map(|input| crate::inspect::InspectInput {
                id: input.id.clone(),
                label: input
                    .label
                    .as_ref()
                    .map(|label| label.get("en").to_string())
                    .filter(|label| !label.is_empty())
                    .unwrap_or_else(|| input.id.clone()),
                range: input.range,
                rest: input.rest(),
                declared: input.declared(),
            })
            .collect()
    }

    /// Replace the scene's inputs and binds with those of `text` (see
    /// [`crate::LabCommand::SceneBindings`]). The scene is checked as a manifest is and nothing
    /// changes if that fails. Binds that stay the same keep their state; if the inputs changed
    /// the scene is rebuilt, as a reload would (then this says so).
    pub fn set_bindings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
    ) -> Result<bool> {
        let manifest = self.files.manifest.with_bindings(text)?;
        if manifest.inputs != self.files.manifest.inputs {
            let mut files = self.files.clone();
            files.manifest = manifest;
            let on_disk = self.on_disk.clone();
            self.rebuild(device, queue, files, on_disk)?;
            return Ok(true);
        }
        if manifest.binds != self.files.manifest.binds {
            self.bindings =
                self.bindings
                    .renewed_in(&manifest.inputs, &manifest.params, &manifest.binds)?;
            queue.write_buffer(
                &self.input_buffer,
                0,
                &self.input_layout.pack(self.bindings.values()),
            );
            // What a reload of the saved text reads is what is on screen: it finds nothing new.
            self.files.manifest = manifest;
        }
        Ok(false)
    }

    /// What each of the scene's bindings said in the last frame drawn: which source and level
    /// produced its input's value.
    pub fn bindings(&self) -> Vec<crate::bind::Report> {
        self.bindings.report()
    }

    /// The values `struct Music` got for the last frame drawn (the defaults before the first),
    /// for the scene switching that follows the music.
    pub fn music(&self) -> &inputs::MusicValues {
        &self.music_values
    }

    /// The camera cut the director made in the last frame drawn, if any: for the transition layer
    /// to dress (see [`Scene::cut_transitions`]).
    pub fn cut_this_frame(&self) -> Option<CutInfo> {
        self.cut
    }

    /// Whether the bindings heard a cut of the director in the last frame drawn: what the `cut`
    /// source reads (see [`crate::bind::Source::read_in`]).
    pub fn bound_cut(&self) -> bool {
        self.bound_cut
    }

    /// The glitch or mosaic over the picture that started in the last frame drawn, if one did:
    /// for the transition layer to draw (see `crate::transition::Overlay`).
    pub fn overlay_this_frame(&self) -> Option<crate::transition::Overlay> {
        self.overlay
    }

    /// Where the music is in its phrase (see [`crate::course::Phrase`]): what a glitch or a
    /// mosaic over the picture takes its length from.
    pub fn phrase(&self) -> crate::course::Phrase {
        self.lens.phrase()
    }

    /// The transitions a switch to this scene may take, by name (its `effects.switch`).
    pub fn switch_transitions(&self) -> &[String] {
        &self.switch_transitions
    }

    /// The transitions that may dress a cut of the director in this scene, by name (its
    /// `effects.cut`); empty for plain cuts.
    pub fn cut_transitions(&self) -> &[String] {
        &self.cut_transitions
    }

    /// What a readout shows of the camera director (none for a scene without shots), with the
    /// music of the last frame.
    pub fn director_view(&self) -> Option<DirectorView> {
        self.director
            .as_ref()
            .map(|director| director.view(&self.music_values))
    }

    /// Which lens effects the scene has on, for a readout.
    pub fn lens_view(&self) -> LensView {
        self.lens.view()
    }

    /// Start a lens flare now, at full strength (a scene lab).
    pub fn start_flare(&mut self) {
        self.lens.start_flare(1.0);
    }

    /// Start a light leak now, at full strength (a scene lab).
    pub fn start_leak(&mut self) {
        self.lens.start_leak(1.0);
    }

    /// Keep a flare at its full swell, called before each frame (a probe).
    #[cfg(test)]
    pub fn hold_flare(&mut self) {
        self.lens.hold_flare();
    }

    /// Play a glitch or a mosaic over the picture from the next frame (a scene lab).
    pub fn start_overlay(&mut self, kind: crate::transition::Overlay) {
        self.lens.start_overlay(kind);
    }

    /// The runtime's decisions this scene reads from now on: what a moment is, when the director
    /// cuts, how the effects' course reads the music (`runtime.toml`).
    pub fn set_runtime(&mut self, runtime: &crate::bind::RuntimeBindings) {
        self.moments.set_binds(runtime.moments.clone());
        if let Some(director) = &mut self.director {
            director.set_binds(runtime.director.clone(), runtime.moments.clone());
        }
        if let Some(looks) = &mut self.looks {
            looks.set_binds(runtime.looks.clone(), runtime.director.clone());
        }
        self.lens.set_binds(runtime);
    }

    /// How the effects' course reads the music in this scene (see [`lens::Lens::course_binds`]).
    pub fn course_binds(&self) -> crate::bind::CourseBinds {
        self.lens.course_binds()
    }

    /// The chances the music's moments play an effect with (the user's setting, or a lab's).
    pub fn set_effect_chances(&mut self, chances: crate::moments::Chances) {
        self.chances = chances;
    }

    /// Cut to the next shot now, by the director's rules for the size (see
    /// `Director::force_cut`); the scene hears of it with the next frame, which reports it as its
    /// cut ([`Scene::cut_this_frame`]). Nothing without shots.
    pub fn cut_now(&mut self) {
        if let Some(director) = &mut self.director {
            let cut = director.force_cut(&self.music_values);
            self.forced_cut = Some(CutInfo {
                size: cut.size,
                motion: cut.motion,
                cue: self.music_values.cue,
                fill: 0,
                dress: None,
                strong: cut.strong,
            });
        }
    }

    /// Start a burst of cuts now, as the one before a lift (see `Director::start_burst`); each
    /// cut is reported with its `fill` ([`Scene::cut_this_frame`]). Nothing without shots.
    pub fn start_burst(&mut self) {
        if let Some(director) = &mut self.director {
            director.start_burst();
        }
    }

    /// Cut to shot `index` and hold it, or with none let go of a held shot (see
    /// `Director::hold_shot`). Whether the scene has the shot.
    pub fn hold_shot(&mut self, index: Option<u32>) -> bool {
        let Some(director) = &mut self.director else {
            return false;
        };
        match index {
            Some(index) => {
                let held = director.hold_shot(index, &self.music_values).is_some();
                if held {
                    self.held_shot = Some(index);
                }
                held
            }
            None => {
                director.release_shot();
                self.held_shot = None;
                true
            }
        }
    }

    /// What a readout shows of the scene's looks (none for a scene without looks).
    pub fn looks_view(&self) -> Option<LooksView> {
        self.looks.as_ref().map(Looks::view)
    }

    /// Change to look `index` now with the usual fade and hold it, so that the runtime changes
    /// the look no more, or with none hand the look back to the runtime (see `Looks::hold`).
    /// Whether the scene has the look.
    pub fn hold_look(&mut self, index: Option<u32>) -> bool {
        let Some(looks) = &mut self.looks else {
            return false;
        };
        match index {
            Some(index) => looks.hold(index, &self.music_values),
            None => {
                looks.release();
                true
            }
        }
    }

    /// Step the director's pace for a scene lab (see `Director::cycle_pace`). Nothing without
    /// shots.
    pub fn cycle_pace(&mut self) {
        if let Some(director) = &mut self.director {
            director.cycle_pace();
        }
    }

    /// Use `pace` for the director in place of the scene's own, or with none the scene's own
    /// again (a scene lab). Nothing without shots.
    pub fn set_pace(&mut self, pace: Option<Pace>) {
        if let Some(director) = &mut self.director {
            director.set_pace_override(pace);
        }
    }

    /// Make every lifting section start burst, or none, or with none leave it to the runtime's
    /// chance (a scene lab; see `Director::set_burst_override`). Nothing without shots.
    pub fn set_bursts(&mut self, bursts: Option<bool>) {
        if let Some(director) = &mut self.director {
            director.set_burst_override(bursts);
        }
    }

    /// Make the scene act as if it listed these lens effects too (a scene lab).
    pub fn set_lens_forced(&mut self, forced: crate::inspect::LabLens) {
        self.lens.set_forced(forced);
    }

    /// The transitions and lens effects the scene declares.
    pub fn effects(&self) -> &manifest::Effects {
        &self.files.manifest.effects
    }

    /// Read the scene's files again. When only parameter defaults changed, only those are
    /// updated; otherwise the scene is rebuilt, keeping its clock and, if `struct State` reads
    /// the same, its state. Returns whether anything changed; on an error the scene stays as it
    /// was. Files that are as they were last read change nothing, even if a lab changed the
    /// bindings on screen since; files that now say what is on screen change nothing either.
    pub fn reload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<bool> {
        if matches!(self.source, Source::Embedded(_)) {
            return Ok(false);
        }
        let files = Files::read(&self.source)?;
        if files == self.on_disk {
            return Ok(false);
        }
        if files == self.files {
            self.on_disk = files;
            return Ok(false);
        }
        if files.same_but_defaults(&self.files) {
            queue.write_buffer(
                &self.params_buffer,
                0,
                &self.params.pack(&files.param_values()),
            );
            // The binds are only evaluated on the CPU: new ones start over, and the pipelines
            // stay.
            if files.manifest.binds != self.files.manifest.binds {
                self.bindings = crate::bind::Bindings::with_scope(
                    &files.manifest.inputs,
                    &files.manifest.params,
                    &files.manifest.binds,
                )?;
                queue.write_buffer(
                    &self.input_buffer,
                    0,
                    &self.input_layout.pack(self.bindings.values()),
                );
            }
            self.on_disk = files.clone();
            self.files = files;
            return Ok(true);
        }
        self.rebuild(device, queue, files.clone(), files)?;
        Ok(true)
    }

    /// Build the scene again from `files`, keeping its clock and, if `struct State` reads the
    /// same, its state, and the bindings that did not change with theirs.
    ///
    /// `on_disk` is what the source said when `files` were made (they are not the same when a
    /// lab replaced the bindings).
    fn rebuild(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        files: Files,
        on_disk: Files,
    ) -> Result<()> {
        // A bind reads its parameters by place, so the same binds over other parameters are not
        // the same.
        let same_params = files
            .manifest
            .params
            .iter()
            .map(|p| (&p.id, p.kind))
            .eq(self.files.manifest.params.iter().map(|p| (&p.id, p.kind)));
        let same_binds = same_params
            && files.manifest.inputs == self.files.manifest.inputs
            && files.manifest.binds == self.files.manifest.binds;
        let same_state = files.state == self.files.state;
        let mut next = Self::build(
            device,
            queue,
            self.source.clone(),
            files,
            self.output_format,
            self.output_size,
            self.analyzer.sample_rate(),
        )?;
        if same_state && next.state_size == self.state_size {
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&self.state, 0, &next.state, 0, self.state_size);
            queue.submit(Some(encoder.finish()));
            next.first = false;
        }
        next.take_music_from(self);
        // Bindings that did not change keep their state (an accumulated distance, an envelope).
        if same_binds {
            std::mem::swap(&mut next.bindings, &mut self.bindings);
        }
        // What a scene lab set on the scene survives its being saved: the director is new, the
        // hold and the pace are the lab's.
        if let (Some(old), Some(new)) = (&self.director, &mut next.director) {
            new.set_pace_override(old.pace_override());
            new.set_burst_override(old.burst_override());
        }
        if let Some(index) = self.held_shot {
            next.hold_shot(Some(index));
        }
        // The look on, its fade and a hold stay too.
        if let (Some(old), Some(new)) = (&self.looks, &mut next.looks) {
            new.inherit(old);
        }
        next.lens.set_forced(self.lens.forced());
        next.chances = self.chances;
        next.on_disk = on_disk;
        next.set_covers(
            device,
            queue,
            self.covers_atlas.clone(),
            self.covers_playing.clone(),
        );
        next.started = self.started;
        next.last = self.last;
        next.index = self.index;
        *self = next;
        Ok(())
    }

    /// Whether the scene reads the covers atlas (`covers = true`).
    pub fn wants_covers(&self) -> bool {
        self.files.manifest.covers
    }

    /// Give the scene the atlas of covers (none: no covers) and the cell of the track playing,
    /// which takes the place of the atlas's cell 0 if there is one. Does nothing for a scene that
    /// did not ask for covers.
    pub fn set_covers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: Option<Arc<Atlas>>,
        playing: Option<Arc<Cell>>,
    ) {
        let Some((texture, _)) = &self.covers else {
            return;
        };
        self.covers_count = covers::valid_cells(atlas.as_deref(), playing.as_deref());
        if let Some(atlas) = &atlas {
            write_texture(queue, texture, &atlas.rgba, 4);
        }
        if let Some(cell) = &playing {
            write_cell(queue, texture, cell);
        }
        if atlas.is_some() || playing.is_some() {
            self.mip_covers(device, queue);
        }
        self.covers_atlas = atlas;
        self.covers_playing = playing;
    }

    /// Replace cell 0 of the atlas with the cell of the track playing (none: forget it, the
    /// atlas's own cell 0 comes back with the next atlas) without touching the other cells.
    pub fn set_playing_cover(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        playing: Option<Arc<Cell>>,
    ) {
        let Some((texture, _)) = &self.covers else {
            return;
        };
        self.covers_count = covers::valid_cells(self.covers_atlas.as_deref(), playing.as_deref());
        if let Some(cell) = &playing {
            write_cell(queue, texture, cell);
            self.mip_covers(device, queue);
        }
        self.covers_playing = playing;
    }

    fn mip_covers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some((_, levels)) = &self.covers else {
            return;
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        self.mipper.generate(
            device,
            &mut encoder,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            levels,
        );
        queue.submit(Some(encoder.finish()));
    }

    /// Draw frames of `width` by `height` from now on, at the same resolution factor. The
    /// buffers that keep the last frame carry over, stretched.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let output_size = (width.max(1), height.max(1));
        self.resize_buffers(device, queue, output_size, self.level.factor)?;
        self.output_size = output_size;
        Ok(())
    }

    /// Draw at `level`: the internal resolution is `level.factor` of the scene's own scale (the
    /// buffers, which keep their scales against it, with it; those that keep the last frame
    /// carry over, stretched), and `frame.quality` is `level.quality`. The scene's state and clock
    /// stay; this costs about as much as one frame.
    pub fn set_level(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, level: Level) {
        let level = Level {
            factor: level.factor.clamp(0.01, 1.0),
            quality: level.quality.clamp(0.0, 1.0),
        };
        if level.factor != self.level.factor {
            // Only the size of the output can fail the check, and it did not change.
            if let Err(e) = self.resize_buffers(device, queue, self.output_size, level.factor) {
                log::warn!("Scene {}: {e:#}", self.name);
                return;
            }
        }
        self.level = level;
    }

    /// Tests only: seed each frame from `seed` and its index instead of at random (none: at random
    /// again). The seed is all that differs between two runs of the same timeline: the scene's
    /// choices and the grain.
    #[cfg(test)]
    pub(crate) fn fix_seed(&mut self, seed: Option<u32>) {
        self.fixed_seed = seed;
    }

    /// Start the scene's music from nothing and let every frame after the first count as `dt`
    /// wall seconds (see [`inputs::EventCounter::with_wall_dt`]): the music's signals then
    /// depend on the timeline alone, not on what scene was drawn before or how fast the GPU was.
    #[cfg(test)]
    pub(crate) fn fix_wall_dt(&mut self, dt: f64) {
        self.events = inputs::EventCounter::with_wall_dt(dt);
    }

    /// The internal resolution (that of `out`, `frame.resolution`).
    pub fn internal_size(&self) -> (u32, u32) {
        self.sized.size
    }

    fn resize_buffers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_size: (u32, u32),
        factor: f32,
    ) -> Result<()> {
        let mut sized = Sized::new(
            device,
            &self.files.manifest,
            (self.output_format, output_size),
            factor,
            (&self.post_buffer, &self.frame),
            Some(&self.sized.post),
        )?;
        let mut encoder = device.create_command_encoder(&Default::default());
        for (old, new) in self.sized.targets.iter().zip(&mut sized.targets) {
            if !old.spec.persistent {
                continue;
            }
            new.current = old.current;
            let format = new.spec.format.wgpu();
            // Into mip 0 of each texture, from all of the old one's.
            for (from, to) in old.views.iter().zip(&new.views) {
                self.mipper
                    .copy(device, &mut encoder, format, &from.1, &to.0);
            }
            if new.spec.mips {
                for levels in &new.mip_views {
                    self.mipper.generate(device, &mut encoder, format, levels);
                }
            }
        }
        queue.submit(Some(encoder.finish()));
        self.sized = sized;
        Ok(())
    }

    pub fn push_pcm(&mut self, interleaved: &[f32], channels: usize) {
        self.analyzer.push(interleaved, channels);
    }

    /// Draw a frame into `target` (the output format and size). With `timestamps`, the first pass
    /// writes query 0 at its beginning and the last query 1 at its end.
    #[cfg(test)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        input: &FrameInput,
        timestamps: Option<&wgpu::QuerySet>,
    ) {
        self.render_timed(device, queue, encoder, target, input, timestamps, true);
    }

    /// [`Scene::render`], where the frame's end timestamp (query 1) is written at the end of the
    /// last pass only if `stamp_end`: a pass drawn after the scene (see `crate::transition`) writes
    /// it instead, so that its cost is part of the frame's.
    #[allow(clippy::too_many_arguments)]
    pub fn render_timed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        input: &FrameInput,
        timestamps: Option<&wgpu::QuerySet>,
        stamp_end: bool,
    ) {
        let started = *self.started.get_or_insert(input.time);
        let time = input.time - started;
        let dt = match self.last {
            Some(last) => (input.time - last).clamp(0.0, 0.1),
            None => 1.0 / 60.0,
        };
        self.last = Some(input.time);
        // The song time going back is a new song, or a seek back.
        if input.song.0 < self.last_song - 1.0 {
            self.analyzer.new_song();
        }
        self.last_song = input.song.0;
        let levels = self.analyzer.update(dt as f32);

        let (w, h) = self.sized.size;
        let mut frame = Vec::with_capacity(wgsl::FRAME_SIZE as usize);
        frame.extend((time as f32).to_le_bytes());
        frame.extend((dt as f32).to_le_bytes());
        frame.extend(self.index.to_le_bytes());
        frame.extend(u32::from(self.first).to_le_bytes());
        frame.extend((w as f32).to_le_bytes());
        frame.extend((h as f32).to_le_bytes());
        frame.extend((w as f32 / h as f32).to_le_bytes());
        let seed = rand::random::<u32>();
        #[cfg(test)]
        let seed = self.fixed_seed.map_or(seed, |fixed| {
            fixed.wrapping_add(self.index.wrapping_mul(0x9e37_79b9))
        });
        frame.extend(seed.to_le_bytes());
        let [r, g, b] = input.accent.unwrap_or_default();
        for v in [r, g, b, f32::from(u8::from(input.accent.is_some()))] {
            frame.extend(v.to_le_bytes());
        }
        frame.extend(self.covers_count.to_le_bytes());
        frame.extend(self.level.quality.to_le_bytes());
        frame.resize(wgsl::FRAME_SIZE as usize, 0);
        queue.write_buffer(&self.frame, 0, &frame);
        let music = inputs::music(
            input.rhythm,
            input.chart,
            levels,
            input.song,
            &mut self.events,
        );
        queue.write_buffer(&self.music, 0, &music.bytes());
        self.music_values = music;
        // The director decides first, so that the bindings can hear of its cut (`cut`).
        self.cut = None;
        let shot = match &mut self.director {
            Some(director) => {
                director.set_song_ahead(input.chart.map(|c| c.later_rise as f32));
                director.set_song(input.chart.and_then(|c| c.song.as_ref()));
                self.cut = director
                    .tick(dt as f32, &music)
                    .map(|cut| CutInfo {
                        size: cut.size,
                        motion: cut.motion,
                        cue: music.cue,
                        fill: cut.fill,
                        dress: None,
                        strong: cut.strong,
                    })
                    .or(self.forced_cut.take());
                director.state().bytes()
            }
            None => [0; wgsl::SHOT_SIZE as usize],
        };
        let params = self.files.param_numbers();
        // The scene's first frame begins its first shot, so it counts as a cut too (a drift that
        // starts over with each shot starts there); a reload that keeps the state does not.
        let cut = self.cut.is_some() || (self.first && self.director.is_some());
        self.bound_cut = cut;
        let inputs = self
            .bindings
            .tick_cut(dt as f32, &music, &params, seed, cut);
        queue.write_buffer(&self.input_buffer, 0, &self.input_layout.pack(inputs));
        queue.write_buffer(&self.shot, 0, &shot);
        let look = match &mut self.looks {
            Some(looks) => {
                looks.set_song(input.chart.and_then(|c| c.song.as_ref()));
                looks.tick(dt as f32, &music, seed);
                looks.state().bytes()
            }
            None => [0; wgsl::LOOK_SIZE as usize],
        };
        queue.write_buffer(&self.look, 0, &look);
        // The lens effects hear where the music is in its phrase before a moment starts one.
        self.lens.listen(&music);
        // A moment that plays an effect: a hard cut in it is dressed with a transition of the
        // scene's list; with no cut, a glide, or no transition that suits, a lens effect plays
        // over the picture.
        if let Some(played) = self.moments.tick(&music, self.chances) {
            // What a lab forced on plays at every moment, on top of what the moment does.
            let forced = self.lens.start_forced(played);
            // A cut the scene glides to is not dressed: the moment plays a light lens effect
            // instead (no glitch or mosaic over a smooth move).
            let glides = self
                .cut
                .as_ref()
                .is_some_and(|cut| cut.motion == ShotMove::Glide);
            let dress = self.cut.as_ref().and_then(|_| {
                crate::moments::dressing_of_cut(&self.cut_transitions, played, glides)
            });
            match (&mut self.cut, dress) {
                (Some(cut), Some(kind)) => cut.dress = Some(kind),
                _ if forced => {}
                _ if glides => self.lens.start_over_glide(played),
                _ => self.lens.start_at(played),
            }
        }
        // The cut a burst leads to, and the start of a high section, land on a strong transition of
        // the scene's list, whatever the roll: four hard cuts ending on a plain one stopped short,
        // and a climax is always entered on a cut. A light one the moment gave it (a high section
        // after a high one does not lift, so its moment is a medium one) makes way for it.
        if let Some(cut) = &mut self.cut
            && cut.strong
            && !cut
                .dress
                .is_some_and(crate::transition::Transition::is_strong)
        {
            let played = crate::moments::Played {
                strength: crate::moments::Strength::Strong,
                label: music.section_label,
            };
            if let Some(kind) = crate::moments::cut_dressing(&self.cut_transitions, played) {
                cut.dress = Some(kind);
            }
        }
        let lens = self.lens.tick(dt as f32, &music, self.level.quality);
        self.overlay = self.lens.take_overlay();
        self.sized.post.write_lens(queue, &lens);
        queue.write_buffer(&self.post_buffer, 0, &self.post_defaults);
        write_texture(
            queue,
            &self.spectrum,
            &f16_bytes(self.analyzer.spectrum.iter().copied()),
            2,
        );
        write_texture(
            queue,
            &self.waveform,
            &f16_bytes(self.analyzer.waveform.iter().flatten().copied()),
            4,
        );

        // The first pass writes the frame's beginning.
        let mut begin = timestamps.is_some();
        if let Some((pipeline, group)) = &self.update {
            let timestamp_writes = timestamps.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: None,
            });
            begin = false;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scene update"),
                timestamp_writes,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        let sized = &self.sized;
        for pass in &self.passes {
            let views: Vec<&wgpu::TextureView> = pass
                .inputs
                .iter()
                .map(|input| match *input {
                    Input::Current(i) => {
                        let t = &sized.targets[i];
                        &t.views[t.current].1
                    }
                    Input::Prev(i) => {
                        let t = &sized.targets[i];
                        &t.views[1 - t.current].1
                    }
                })
                .collect();
            let entries: Vec<wgpu::BindGroupEntry> = views
                .iter()
                .enumerate()
                .map(|(i, view)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                })
                .collect();
            let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene pass inputs"),
                layout: &pass.inputs_layout,
                entries: &entries,
            });
            let view = match pass.target {
                None => &sized.out.1,
                Some(i) => {
                    let t = &sized.targets[i];
                    &t.views[t.current].0
                }
            };
            {
                let stamps = timestamp_writes(timestamps, begin, false);
                begin = false;
                let mut render = begin_pass(encoder, view, &pass.label, stamps);
                render.set_pipeline(&pass.pipeline);
                render.set_bind_group(0, &self.pass_globals, &[]);
                render.set_bind_group(1, &inputs, &[]);
                render.draw(0..3, 0..1);
            }
            if let Some(i) = pass.target {
                let t = &sized.targets[i];
                if t.spec.mips {
                    self.mipper.generate(
                        device,
                        encoder,
                        t.spec.format.wgpu(),
                        &t.mip_views[t.current],
                    );
                }
            }
        }
        sized.post.render(
            device,
            encoder,
            &sized.out.1,
            &self.noise,
            target,
            timestamps,
            begin,
            stamp_end,
            &lens,
        );

        for t in &mut self.sized.targets {
            if t.spec.persistent {
                t.current = 1 - t.current;
            }
        }
        self.index = self.index.wrapping_add(1);
        self.first = false;
    }
}

#[cfg(test)]
mod tests;
