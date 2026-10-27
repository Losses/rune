//! The built-in post-processing (`post.wgsl`): bloom, tone mapping and the rest, from the
//! scene's HDR `out` into the frame that leaves the GPU.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::lens::{LENS_SIZE, LensFrame};
use super::manifest::Tonemap;
use super::wgsl;

const SHADER: &str = include_str!("post.wgsl");
/// Bloom levels at most; each half the size of the one before, the first half the internal size.
const MAX_LEVELS: u32 = 6;

pub struct Post {
    levels: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// The lens effects' textures, drawn only while the flare or a block frame is on: see
    /// [`LensTargets`].
    lens_targets: LensTargets,
    /// Which of `lens_targets.keys` the next frame's key is drawn into (the other holds the
    /// last, which it follows).
    key_next: AtomicUsize,
    /// `struct Lens` of `post.wgsl`: which lens effects are on in this frame.
    lens: wgpu::Buffer,
    /// For as many levels as there are.
    pipelines: Pipelines,
    tonemap: u32,
    output: wgpu::TextureFormat,
    layout: wgpu::PipelineLayout,
    globals: wgpu::BindGroup,
    textures: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// The pipelines, whose shader knows the number of levels.
#[derive(Clone)]
struct Pipelines {
    first_down: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    lens_source: wgpu::RenderPipeline,
    lens_cells: wgpu::RenderPipeline,
    lens_key: wgpu::RenderPipeline,
    flare_wide: wgpu::RenderPipeline,
    flare: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

/// The textures of the lens effects. All but the cells and the keys are a quarter of the
/// internal size: small textures keep the effects cheap, and the picture at that size has no
/// halftone dots or lone bright pixels left to make specks of.
struct LensTargets {
    /// The exposed picture's luma in alpha, what the key and the block frame start from, and in
    /// red, while the flare is on, what throws its streaks.
    source: (wgpu::Texture, wgpu::TextureView),
    /// Of 8 by 8 cells of it: the brightest luma, the mean and where the brightest is.
    cells: (wgpu::Texture, wgpu::TextureView),
    /// The frame's key (1 by 1): that of the whole frame, following the last smoothly, so two
    /// to take turns.
    keys: [(wgpu::Texture, wgpu::TextureView); 2],
    /// The flare's wide streaks, first step.
    wide: (wgpu::Texture, wgpu::TextureView),
    /// The flare, which the composite lays over the picture.
    flare: (wgpu::Texture, wgpu::TextureView),
}

const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// How many cells the frame's key divides it into each way (`CELLS` of `post.wgsl`).
const KEY_CELLS: u32 = 8;

fn levels_for(width: u32, height: u32) -> u32 {
    let mut n = 0;
    let (mut w, mut h) = (width / 2, height / 2);
    while n < MAX_LEVELS && w >= 4 && h >= 4 {
        n += 1;
        w /= 2;
        h /= 2;
    }
    n.max(1)
}

impl Post {
    /// `post` is the scene's Post buffer and `frame` its Frame uniform; `size` the internal size.
    pub fn new(
        device: &wgpu::Device,
        tonemap: Tonemap,
        output: wgpu::TextureFormat,
        (post, frame): (&wgpu::Buffer, &wgpu::Buffer),
        size: (u32, u32),
    ) -> Self {
        let tonemap = match tonemap {
            Tonemap::Agx => 0,
            Tonemap::Aces => 1,
            Tonemap::None => 2,
        };
        let uniform = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene post globals"),
            entries: &[
                uniform(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                uniform(
                    1,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                uniform(
                    2,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let texture = |binding| {
            uniform(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        };
        let textures = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene post textures"),
            entries: &[
                texture(0),
                uniform(
                    1,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                texture(2),
                texture(3),
                texture(4),
                texture(5),
                texture(6),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene post"),
            bind_group_layouts: &[Some(&globals_layout), Some(&textures)],
            immediate_size: 0,
        });
        // Zeros until the first frame writes it: no effect on.
        let lens = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("scene lens"),
                contents: &[0; LENS_SIZE as usize],
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene post globals"),
            layout: &globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: post.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lens.as_entire_binding(),
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let count = levels_for(size.0, size.1);
        Self {
            levels: levels(device, size, count),
            lens_targets: LensTargets::new(device, size),
            key_next: AtomicUsize::new(0),
            lens,
            pipelines: pipelines(device, &layout, tonemap, output, count),
            tonemap,
            output,
            layout,
            globals,
            textures,
            sampler,
        }
    }

    /// The same post-processing for the internal size `size`. The pipelines carry over unless
    /// the number of bloom levels changes, so this is cheap.
    pub fn resized(&self, device: &wgpu::Device, size: (u32, u32)) -> Self {
        let count = levels_for(size.0, size.1);
        let pipelines = if count as usize == self.levels.len() {
            self.pipelines.clone()
        } else {
            pipelines(device, &self.layout, self.tonemap, self.output, count)
        };
        Self {
            levels: levels(device, size, count),
            lens_targets: LensTargets::new(device, size),
            key_next: AtomicUsize::new(0),
            lens: self.lens.clone(),
            pipelines,
            tonemap: self.tonemap,
            output: self.output,
            layout: self.layout.clone(),
            globals: self.globals.clone(),
            textures: self.textures.clone(),
            sampler: self.sampler.clone(),
        }
    }

    /// The textures of a pass: `source` and the sampler, and `extra` at bindings 2 to 6 (the
    /// bloom, the blue noise, the flare, the key and the quarter-size picture, as `post.wgsl`
    /// names them).
    fn bind(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        extra: [&wgpu::TextureView; 5],
    ) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        entries.extend(
            (2..)
                .zip(extra)
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene post textures"),
            layout: &self.textures,
            entries: &entries,
        })
    }

    fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        textures: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        clear: bool,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals, &[]);
        pass.set_bind_group(1, textures, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Tell the pass which lens effects are on in the next frame (see [`super::lens::Lens`]).
    pub fn write_lens(&self, queue: &wgpu::Queue, lens: &LensFrame) {
        queue.write_buffer(&self.lens, 0, &lens.bytes());
    }

    /// Bloom from `out`, then everything into `target`. With the flare or a block frame on in
    /// `lens` (the frame's lens, see [`Self::write_lens`]), their passes are drawn first; nothing
    /// of them costs anything otherwise. With `timestamps`, the first pass writes the frame's
    /// beginning if `begin` (when no pass of the scene did) and the last its end if `end`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        out: &wgpu::TextureView,
        noise: &wgpu::TextureView,
        target: &wgpu::TextureView,
        timestamps: Option<&wgpu::QuerySet>,
        begin: bool,
        end: bool,
        lens: &LensFrame,
    ) {
        let p = &self.pipelines;
        let first = &self.levels[0].1;
        // A pass reads only some bindings; the noise stands in for the rest, as a texture cannot
        // be bound while it is drawn into.
        let none = [noise; 5];
        let group = self.bind(device, out, none);
        let stamps = super::timestamp_writes(timestamps, begin, false);
        self.draw(encoder, &p.first_down, &group, first, true, stamps);
        for pair in self.levels.windows(2) {
            let group = self.bind(device, &pair[0].1, none);
            self.draw(encoder, &p.down, &group, &pair[1].1, true, None);
        }
        for pair in self.levels.windows(2).rev() {
            let group = self.bind(device, &pair[1].1, none);
            self.draw(encoder, &p.up, &group, &pair[0].1, false, None);
        }

        let t = &self.lens_targets;
        let flare = lens.flare > 0.0;
        let mut composite = [first, noise, noise, noise, noise];
        if flare || lens.blocks > 0.0 {
            let next = self.key_next.fetch_xor(1, Ordering::Relaxed);
            let (key, last) = (&t.keys[next].1, &t.keys[1 - next].1);
            // The source finds the flare's highlights against the last key.
            let group = self.bind(device, out, [noise, noise, noise, last, noise]);
            self.draw(encoder, &p.lens_source, &group, &t.source.1, true, None);
            let group = self.bind(device, &t.source.1, none);
            self.draw(encoder, &p.lens_cells, &group, &t.cells.1, true, None);
            let group = self.bind(device, &t.cells.1, [noise, noise, noise, last, noise]);
            self.draw(encoder, &p.lens_key, &group, key, true, None);
            if flare {
                let group = self.bind(device, &t.source.1, none);
                self.draw(encoder, &p.flare_wide, &group, &t.wide.1, true, None);
                let group = self.bind(device, &t.source.1, [noise, noise, &t.wide.1, key, noise]);
                self.draw(encoder, &p.flare, &group, &t.flare.1, true, None);
                composite[2] = &t.flare.1;
            }
            composite[3] = key;
            composite[4] = &t.source.1;
        }
        let group = self.bind(device, out, composite);
        let stamps = super::timestamp_writes(timestamps, false, end);
        self.draw(encoder, &p.composite, &group, target, true, stamps);
    }
}

/// The bloom levels for the internal size `size`: half of it, a quarter, and so on.
fn levels(
    device: &wgpu::Device,
    size: (u32, u32),
    count: u32,
) -> Vec<(wgpu::Texture, wgpu::TextureView)> {
    let mut levels = Vec::new();
    let (mut w, mut h) = (size.0 / 2, size.1 / 2);
    for _ in 0..count {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene bloom"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BLOOM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        levels.push((texture, view));
        w /= 2;
        h /= 2;
    }
    levels
}

impl LensTargets {
    /// For the internal size `size`.
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let quarter = ((size.0 / 4).max(1), (size.1 / 4).max(1));
        Self {
            source: lens_target(device, "scene lens source", quarter),
            cells: lens_target(device, "scene lens cells", (KEY_CELLS, KEY_CELLS)),
            keys: [
                lens_target(device, "scene lens key", (1, 1)),
                lens_target(device, "scene lens key", (1, 1)),
            ],
            wide: lens_target(device, "scene flare wide", quarter),
            flare: lens_target(device, "scene flare", quarter),
        }
    }
}

fn lens_target(
    device: &wgpu::Device,
    label: &str,
    (width, height): (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: BLOOM_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

/// The pipelines for `count` bloom levels.
fn pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    tonemap: u32,
    output: wgpu::TextureFormat,
    count: u32,
) -> Pipelines {
    let source = format!(
        "{}{}const LEVELS: u32 = {count}u;\nconst TONEMAP: u32 = {tonemap}u;\n{SHADER}",
        wgsl::FRAME,
        wgsl::POST
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene post"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = |entry: &str, format, blend| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let add = Some(wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent::REPLACE,
    });
    Pipelines {
        first_down: pipeline("first_down", BLOOM_FORMAT, None),
        down: pipeline("down", BLOOM_FORMAT, None),
        up: pipeline("up", BLOOM_FORMAT, add),
        lens_source: pipeline("lens_source", BLOOM_FORMAT, None),
        lens_cells: pipeline("lens_cells", BLOOM_FORMAT, None),
        lens_key: pipeline("lens_key", BLOOM_FORMAT, None),
        flare_wide: pipeline("flare_wide", BLOOM_FORMAT, None),
        flare: pipeline("flare", BLOOM_FORMAT, None),
        composite: pipeline("composite", output, None),
    }
}
