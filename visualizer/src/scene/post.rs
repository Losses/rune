//! The built-in post-processing (`post.wgsl`): bloom, tone mapping and the rest, from the
//! scene's HDR `out` into the frame that leaves the GPU.

use super::manifest::Tonemap;
use super::wgsl;

const SHADER: &str = include_str!("post.wgsl");
/// Bloom levels at most; each half the size of the one before, the first half the internal size.
const MAX_LEVELS: u32 = 6;

pub struct Post {
    levels: Vec<(wgpu::Texture, wgpu::TextureView)>,
    first_down: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    globals: wgpu::BindGroup,
    textures: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

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
        let count = levels_for(size.0, size.1);
        let tonemap = match tonemap {
            Tonemap::Agx => 0,
            Tonemap::Aces => 1,
            Tonemap::None => 2,
        };
        let source = format!(
            "{}{}const LEVELS: u32 = {count}u;\nconst TONEMAP: u32 = {tonemap}u;\n{SHADER}",
            wgsl::FRAME,
            wgsl::POST
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene post"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
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
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene post"),
            bind_group_layouts: &[Some(&globals_layout), Some(&textures)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
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
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            levels.push((texture, view));
            w /= 2;
            h /= 2;
        }
        Self {
            first_down: pipeline("first_down", BLOOM_FORMAT, None),
            down: pipeline("down", BLOOM_FORMAT, None),
            up: pipeline("up", BLOOM_FORMAT, add),
            composite: pipeline("composite", output, None),
            levels,
            globals,
            textures,
            sampler,
        }
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        extra: (&wgpu::TextureView, &wgpu::TextureView),
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
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals, &[]);
        pass.set_bind_group(1, textures, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Bloom from `out`, then everything into `target`.
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        out: &wgpu::TextureView,
        noise: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let first = &self.levels[0].1;
        // The bloom binding is only read by the composite; the noise stands in elsewhere, as a
        // level cannot be bound while it is drawn into.
        let group = self.bind(device, out, (noise, noise));
        self.draw(encoder, &self.first_down, &group, first, true);
        for pair in self.levels.windows(2) {
            let group = self.bind(device, &pair[0].1, (noise, noise));
            self.draw(encoder, &self.down, &group, &pair[1].1, true);
        }
        for pair in self.levels.windows(2).rev() {
            let group = self.bind(device, &pair[1].1, (noise, noise));
            self.draw(encoder, &self.up, &group, &pair[0].1, false);
        }
        let group = self.bind(device, out, (first, noise));
        self.draw(encoder, &self.composite, &group, target, true);
    }
}
