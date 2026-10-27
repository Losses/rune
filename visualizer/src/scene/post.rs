//! The built-in post-processing (`post.wgsl`): bloom, tone mapping and the rest, from the
//! scene's HDR `out` into the frame that leaves the GPU.

use super::lens::{LENS_SIZE, LensFrame};
use super::manifest::Tonemap;
use super::wgsl;

const SHADER: &str = include_str!("post.wgsl");
/// Bloom levels at most; each half the size of the one before, the first half the internal size.
const MAX_LEVELS: u32 = 6;

pub struct Post {
    levels: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// What the lens flare makes from a bloom level, in the size of that level; the composite
    /// adds it. Only drawn while the flare is on.
    flare: (wgpu::Texture, wgpu::TextureView),
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
    flare: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The bloom level the flare is made from: the second (a quarter of the internal size, or the
/// only one at a tiny size). A small texture keeps the flare cheap.
const FLARE_LEVEL: usize = 1;

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
            flare: flare_target(device, size, count),
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
            flare: flare_target(device, size, count),
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

    fn bind(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        extra: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene post textures"),
            layout: &self.textures,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(extra.0),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(extra.1),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(extra.2),
                },
            ],
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

    /// Bloom from `out`, then everything into `target`. With `flare` (the frame's lens has a
    /// flare, see [`Self::write_lens`]) the flare is drawn from the bloom first; nothing of it
    /// costs anything otherwise. With `timestamps`, the first pass writes the frame's beginning
    /// if `begin` (when no pass of the scene did) and the last its end if `end`.
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
        flare: bool,
    ) {
        let p = &self.pipelines;
        let first = &self.levels[0].1;
        // The bloom and flare bindings are only read by the composite; the noise stands in
        // elsewhere, as a level cannot be bound while it is drawn into.
        let group = self.bind(device, out, (noise, noise, noise));
        let stamps = super::timestamp_writes(timestamps, begin, false);
        self.draw(encoder, &p.first_down, &group, first, true, stamps);
        for pair in self.levels.windows(2) {
            let group = self.bind(device, &pair[0].1, (noise, noise, noise));
            self.draw(encoder, &p.down, &group, &pair[1].1, true, None);
        }
        for pair in self.levels.windows(2).rev() {
            let group = self.bind(device, &pair[1].1, (noise, noise, noise));
            self.draw(encoder, &p.up, &group, &pair[0].1, false, None);
        }
        if flare {
            let from = &self.levels[FLARE_LEVEL.min(self.levels.len() - 1)].1;
            let group = self.bind(device, from, (noise, noise, noise));
            self.draw(encoder, &p.flare, &group, &self.flare.1, true, None);
        }
        let flare_view = if flare { &self.flare.1 } else { noise };
        let group = self.bind(device, out, (first, noise, flare_view));
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

/// The texture the flare is drawn into: the size of the level it is made from.
fn flare_target(
    device: &wgpu::Device,
    size: (u32, u32),
    count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let shift = FLARE_LEVEL.min(count as usize - 1) as u32 + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene flare"),
        size: wgpu::Extent3d {
            width: (size.0 >> shift).max(1),
            height: (size.1 >> shift).max(1),
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
        flare: pipeline("flare", BLOOM_FORMAT, None),
        composite: pipeline("composite", output, None),
    }
}
