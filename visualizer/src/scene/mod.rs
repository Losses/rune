//! Scenes in Rune's own format: a folder with
//! `scene.toml` and WGSL files. The scene's `update()` runs once per frame on the GPU, its passes
//! draw into HDR buffers, and the runtime's post-processing makes the frame.

mod audio;
mod inputs;
mod manifest;
mod noise;
mod post;
mod wgsl;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::chart::ChartState;
use crate::covers::{self, Atlas, Cell};
use crate::rhythm::Rhythm;
use audio::{Analyzer, SPECTRUM_ROWS, SPECTRUM_WIDTH, WAVEFORM_WIDTH};
use inputs::EventCounter;
use manifest::{Format, Manifest, OUT, PREV, Value};
use wgsl::ParamsLayout;

pub use manifest::{MANIFEST, argb_to_linear};

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

fn read_text(dir: &Path, name: &str) -> Result<(String, String)> {
    let text = std::fs::read_to_string(dir.join(name))
        .with_context(|| format!("Failed to read {name}"))?;
    Ok((name.to_string(), text))
}

impl Files {
    fn read(dir: &Path) -> Result<Self> {
        let (_, text) = read_text(dir, MANIFEST)?;
        let manifest = Manifest::parse(&text).context(MANIFEST)?;
        let state = match &manifest.state {
            Some(s) => Some(read_text(dir, &s.file)?),
            None => None,
        };
        let update = match manifest.state.as_ref().and_then(|s| s.update.as_ref()) {
            Some(file) => Some(read_text(dir, file)?),
            None => None,
        };
        let common = manifest
            .common
            .iter()
            .map(|file| read_text(dir, file))
            .collect::<Result<_>>()?;
        let shaders = manifest
            .passes
            .iter()
            .map(|p| read_text(dir, &p.shader))
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

/// Mip chains: each level drawn from the one above with a linear sampler (a 2x2 box).
struct Mipper {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
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
        Self {
            module,
            layout,
            pipeline_layout,
            sampler,
            pipelines: Vec::new(),
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
            let mut pass = begin_pass(encoder, &pair[1], "scene mips");
            pass.set_pipeline(&self.pipelines[i].1);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

pub struct Scene {
    pub name: String,
    dir: PathBuf,
    files: Files,
    params: ParamsLayout,
    frame: wgpu::Buffer,
    music: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
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
    analyzer: Analyzer,
    events: EventCounter,
    /// The visualizer's time when the scene started, and of the last frame.
    started: Option<f64>,
    last: Option<f64>,
    /// The song time of the last frame.
    last_song: f64,
    index: u32,
    first: bool,
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
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
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

impl Sized {
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: &wgpu::Device,
        manifest: &Manifest,
        output_format: wgpu::TextureFormat,
        output_size: (u32, u32),
        buffers: (&wgpu::Buffer, &wgpu::Buffer),
    ) -> Result<Self> {
        let size = scaled(output_size, manifest.render.scale);
        let mut total = 0u64;
        let mut targets = Vec::new();
        for spec in &manifest.buffers {
            let (w, h) = scaled(size, spec.scale);
            let mips = if spec.mips {
                32 - w.max(h).leading_zeros()
            } else {
                1
            };
            let count = if spec.persistent { 2 } else { 1 };
            total += u64::from(w)
                * u64::from(h)
                * spec.format.bytes_per_pixel()
                * count
                * if spec.mips { 4 } else { 3 }
                / 3;
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
        ensure!(
            total <= MAX_BUFFER_BYTES,
            "the buffers need {} MB at this size, more than {} MB",
            total >> 20,
            MAX_BUFFER_BYTES >> 20
        );
        let out = plain_texture(
            device,
            "scene out",
            size,
            OUT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            1,
        );
        let out_view = out.create_view(&Default::default());
        let post = post::Post::new(device, manifest.post.tonemap, output_format, buffers, size);
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
    pub fn load(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dir: &Path,
        output_format: wgpu::TextureFormat,
        output_size: (u32, u32),
        sample_rate: u32,
    ) -> Result<Self> {
        let files = Files::read(dir)?;
        Self::build(
            device,
            queue,
            dir,
            files,
            output_format,
            output_size,
            sample_rate,
        )
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dir: &Path,
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
        let params_buffer = uniform("scene params", params.size);
        queue.write_buffer(&params_buffer, 0, &params.pack(&files.param_values()));
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
            let bytes = std::fs::read(dir.join(&asset.file))
                .with_context(|| format!("Failed to read {}", asset.file))?;
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
                pipeline,
                inputs_layout,
                inputs,
                target,
            });
        }
        let sized = Sized::new(
            device,
            manifest,
            output_format,
            output_size,
            (&post_buffer, &frame),
        )?;
        if let Some(error) = pollster::block_on(scope.pop()) {
            bail!("{error}");
        }
        keep.push(spectrum.clone());
        Ok(Self {
            name: manifest.name.clone(),
            dir: dir.to_path_buf(),
            params,
            frame,
            music,
            params_buffer,
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
            analyzer: Analyzer::new(sample_rate),
            events: EventCounter::default(),
            started: None,
            last: None,
            last_song: 0.0,
            index: 0,
            first: true,
            files,
            _keep: keep,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Read the scene's files again. When only parameter defaults changed, only those are
    /// updated; otherwise the scene is rebuilt, keeping its clock and, if `struct State` reads
    /// the same, its state. Returns whether anything changed; on an error the scene stays as it
    /// was.
    pub fn reload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<bool> {
        let files = Files::read(&self.dir)?;
        if files == self.files {
            return Ok(false);
        }
        if files.same_but_defaults(&self.files) {
            queue.write_buffer(
                &self.params_buffer,
                0,
                &self.params.pack(&files.param_values()),
            );
            self.files = files;
            return Ok(true);
        }
        let same_state = files.state == self.files.state;
        let mut next = Self::build(
            device,
            queue,
            &self.dir,
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
        std::mem::swap(&mut next.analyzer, &mut self.analyzer);
        std::mem::swap(&mut next.events, &mut self.events);
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
        Ok(true)
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

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> Result<()> {
        self.output_size = (width.max(1), height.max(1));
        self.sized = Sized::new(
            device,
            &self.files.manifest,
            self.output_format,
            self.output_size,
            (&self.post_buffer, &self.frame),
        )?;
        Ok(())
    }

    pub fn push_pcm(&mut self, interleaved: &[f32], channels: usize) {
        self.analyzer.push(interleaved, channels);
    }

    /// Draw a frame into `target` (the output format and size).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        input: &FrameInput,
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
        frame.extend(rand::random::<u32>().to_le_bytes());
        let [r, g, b] = input.accent.unwrap_or_default();
        for v in [r, g, b, f32::from(u8::from(input.accent.is_some()))] {
            frame.extend(v.to_le_bytes());
        }
        frame.extend(self.covers_count.to_le_bytes());
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

        if let Some((pipeline, group)) = &self.update {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scene update"),
                timestamp_writes: None,
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
                let mut render = begin_pass(encoder, view, "scene pass");
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
        sized
            .post
            .render(device, encoder, &sized.out.1, &self.noise, target);

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
