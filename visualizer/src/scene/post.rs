//! The built-in post-processing (`post.wgsl`): bloom, tone mapping and the rest, from the
//! scene's HDR `out` into the frame that leaves the GPU.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::lens::{LENS_SIZE, LensFrame};
use super::manifest::Tonemap;
use super::wgsl;

const SHADER: &str = include_str!("post.wgsl");
/// Bloom levels at most; each half the size of the one before, the first half the internal size.
const MAX_LEVELS: u32 = 6;

pub struct Post {
    levels: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// The lens flare's textures, drawn only while it is on: see [`FlareTargets`].
    flare: FlareTargets,
    /// Which of `flare.bright` the next frame's highlights are drawn into (the other holds the
    /// last), and whether the last frame drew the flare.
    bright_next: AtomicUsize,
    flared: AtomicBool,
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
    flare_bright: wgpu::RenderPipeline,
    streak_down: wgpu::RenderPipeline,
    streak_up: wgpu::RenderPipeline,
    ghost_down: wgpu::RenderPipeline,
    ghost_blur: wgpu::RenderPipeline,
    flare: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

/// The textures of the lens flare: small, to keep it cheap. They are a quarter of the internal
/// width but half its height, as the streaks are thin, and shrink only in width after that.
struct FlareTargets {
    /// The highlights, of this frame and the last, which take turns.
    bright: [(wgpu::Texture, wgpu::TextureView); 2],
    /// The streaks' pyramid: each level half the width of the one before (the first half that
    /// of `bright`), all of its height, down to a few texels across.
    levels: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// The highlights folded into an eighth of the internal width and height, and then blurred
    /// over a disc, for the ghosts and the halo (see `ghost_down` in `post.wgsl`).
    ghost: [(wgpu::Texture, wgpu::TextureView); 2],
    /// The flare, which the composite lays over the picture; the size of `bright`.
    flare: (wgpu::Texture, wgpu::TextureView),
}

const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The streaks' pyramid ends at this width or below, and has at most this many levels.
const STREAK_MIN_WIDTH: u32 = 4;
const STREAK_MAX_LEVELS: usize = 7;

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
            flare: FlareTargets::new(device, size),
            bright_next: AtomicUsize::new(0),
            flared: AtomicBool::new(false),
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
            flare: FlareTargets::new(device, size),
            bright_next: AtomicUsize::new(0),
            flared: AtomicBool::new(false),
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

    /// The textures of a pass: `source` and the sampler, and `extra` at bindings 2 to 4 (the
    /// bloom, the blue noise and the flare, as `post.wgsl` names them).
    fn bind(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        extra: [&wgpu::TextureView; 3],
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

    /// Tell the pass which lens effects are on in the next frame (see [`super::lens::Lens`]),
    /// and whether the flare follows the highlights of the last.
    pub fn write_lens(&self, queue: &wgpu::Queue, lens: &LensFrame) {
        let lens = LensFrame {
            flare_history: if self.flared.load(Ordering::Relaxed) {
                1.0
            } else {
                0.0
            },
            ..*lens
        };
        queue.write_buffer(&self.lens, 0, &lens.bytes());
    }

    /// Bloom from `out`, then everything into `target`. With the flare on in `lens` (the
    /// frame's lens, see [`Self::write_lens`]), its passes are drawn first; nothing of them
    /// costs anything otherwise. With `timestamps`, the first pass writes the frame's
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
        let none = [noise; 3];
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

        let mut composite = [first, noise, noise];
        self.flared.store(lens.flare > 0.0, Ordering::Relaxed);
        if lens.flare > 0.0 {
            let t = &self.flare;
            let next = self.bright_next.fetch_xor(1, Ordering::Relaxed);
            let (bright, last) = (&t.bright[next].1, &t.bright[1 - next].1);
            let group = self.bind(device, out, [noise, noise, last]);
            self.draw(encoder, &p.flare_bright, &group, bright, true, None);
            let mut from = bright;
            for level in &t.levels {
                let group = self.bind(device, from, none);
                self.draw(encoder, &p.streak_down, &group, &level.1, true, None);
                from = &level.1;
            }
            for pair in t.levels.windows(2).rev() {
                let group = self.bind(device, &pair[1].1, none);
                self.draw(encoder, &p.streak_up, &group, &pair[0].1, false, None);
            }
            let group = self.bind(device, bright, none);
            self.draw(encoder, &p.ghost_down, &group, &t.ghost[0].1, true, None);
            let group = self.bind(device, &t.ghost[0].1, none);
            self.draw(encoder, &p.ghost_blur, &group, &t.ghost[1].1, true, None);
            let group = self.bind(device, bright, [&t.ghost[1].1, noise, &t.levels[0].1]);
            self.draw(encoder, &p.flare, &group, &t.flare.1, true, None);
            composite[2] = &t.flare.1;
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

impl FlareTargets {
    /// For the internal size `size`.
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let quarter = ((size.0 / 4).max(1), (size.1 / 2).max(1));
        let eighth = ((size.0 / 8).max(1), (size.1 / 8).max(1));
        let mut levels = Vec::new();
        let mut width = quarter.0;
        while levels.is_empty() || (width > STREAK_MIN_WIDTH && levels.len() < STREAK_MAX_LEVELS) {
            width = (width / 2).max(1);
            levels.push(lens_target(
                device,
                "scene flare streaks",
                (width, quarter.1),
            ));
        }
        Self {
            bright: [
                lens_target(device, "scene flare highlights", quarter),
                lens_target(device, "scene flare highlights", quarter),
            ],
            levels,
            ghost: [
                lens_target(device, "scene flare ghosts", eighth),
                lens_target(device, "scene flare ghosts", eighth),
            ],
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
        flare_bright: pipeline("flare_bright", BLOOM_FORMAT, None),
        streak_down: pipeline("streak_down", BLOOM_FORMAT, None),
        streak_up: pipeline("streak_up", BLOOM_FORMAT, add),
        ghost_down: pipeline("ghost_down", BLOOM_FORMAT, None),
        ghost_blur: pipeline("ghost_blur", BLOOM_FORMAT, None),
        flare: pipeline("flare", BLOOM_FORMAT, None),
        composite: pipeline("composite", output, None),
    }
}
