//! Scenes drawn on the GPU (skipped without an adapter).

use super::*;
use crate::bind;
use image::RgbaImage;
use naga::TypeInner;
use std::env;
use std::fs;
use std::process as std_process;
use std::time::Duration;
use toml::Value as TomlValue;
use wgpu::BufferUsages;
use wgpu::DeviceDescriptor;
use wgpu::Instance;
use wgpu::MapMode;
use wgpu::PollType;
use wgpu::RequestAdapterOptions;
use wgpu::TextureFormat;
use wgpu::TextureUsages;

const SIZE: (u32, u32) = (64, 32);
const FORMAT: wgpu::TextureFormat = TextureFormat::Rgba8Unorm;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&DeviceDescriptor::default())).ok()
}

/// A scene folder of its own under the temp directory, with `files` in it.
fn folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = env::temp_dir().join(format!("rune-scene-{name}-{}", std_process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

/// Plain output: no tone mapping or effects, so a pixel is the sRGB of what `out` holds.
const MANIFEST_TEXT: &str = r#"
format = 1
name = "Counter"
license = "MPL-2.0"

[state]
file = "state.wgsl"
update = "update.wgsl"

[[param]]
id = "step"
default = 0.1

[[buffer]]
id = "trail"
persistent = true

[[pass]]
shader = "trail.wgsl"
target = "trail"
inputs = ["prev:trail"]

[[pass]]
shader = "final.wgsl"
target = "out"
inputs = ["trail"]

[post]
tonemap = "none"
grain = 0.0
chromatic = 0.0
vignette = 0.0
bloom = { intensity = 0.0 }
"#;

const STATE: &str = "struct State {\n  frames: u32,\n  beats: u32,\n}\n";
const UPDATE: &str = "fn update() {\n  state.frames += 1u;\n  state.beats += music.beats;\n}\n";
/// The trail grows by `step` in red each frame.
const TRAIL: &str = "fn shade(px: Pixel) -> vec4f {\n  let last = textureLoad(prev_trail, vec2i(px.coord), 0);\n  return last + vec4f(params.step, 0.0, 0.0, 0.0);\n}\n";
/// Red from the trail, green counts frames (in tenths), blue counts beats (in tenths).
const FINAL: &str = "fn shade(px: Pixel) -> vec4f {\n  let t = textureLoad(trail, vec2i(px.coord), 0);\n  return vec4f(t.r, f32(state.frames) * 0.1, f32(state.beats) * 0.1, 1.0);\n}\n";

fn counter(name: &str) -> PathBuf {
    folder(
        name,
        &[
            (MANIFEST, MANIFEST_TEXT),
            ("state.wgsl", STATE),
            ("update.wgsl", UPDATE),
            ("trail.wgsl", TRAIL),
            ("final.wgsl", FINAL),
        ],
    )
}

fn srgb_to_linear(byte: u8) -> f32 {
    let c = f32::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

struct Screen {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Screen {
    fn new(device: &wgpu::Device) -> Self {
        let texture = plain_texture(
            device,
            "test screen",
            SIZE,
            FORMAT,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            1,
        );
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }

    /// The linear colour of the middle pixel.
    fn middle(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> [f32; 3] {
        self.pixel_srgb(device, queue, SIZE.0 / 2, SIZE.1 / 2)
            .map(srgb_to_linear)
    }

    fn pixel_srgb(&self, device: &wgpu::Device, queue: &wgpu::Queue, x: u32, y: u32) -> [u8; 3] {
        let row = (SIZE.0 * 4).next_multiple_of(256);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * SIZE.1),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(SIZE.1),
                },
            },
            self.texture.size(),
        );
        queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(MapMode::Read, |_| {});
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let at = (y * row + x * 4) as usize;
        [data[at], data[at + 1], data[at + 2]]
    }
}

fn frame(
    scene: &mut Scene,
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    screen: &Screen,
    time: f64,
    beat: f64,
) {
    let rhythm = Rhythm {
        bpm: 120.0,
        beat,
        confidence: 1.0,
        ..Default::default()
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    scene.render(
        device,
        queue,
        &mut encoder,
        &screen.view,
        &FrameInput {
            time,
            rhythm: &rhythm,
            chart: None,
            song: (time, true),
            song_chart: None,
            song_seed: None,
            accent: None,
            wall: Instant::now(),
        },
        None,
    );
    queue.submit(Some(encoder.finish()));
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02)
}

#[test]
fn the_state_and_a_persistent_buffer_carry_from_frame_to_frame() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = counter("carry");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    assert_eq!(scene.name, "Counter");
    let screen = Screen::new(&device);
    // Beats 0.5, 1.2, 3.1: three beats went by, two of them in one frame.
    for (i, beat) in [0.5, 1.2, 3.1].into_iter().enumerate() {
        frame(&mut scene, gpu, &screen, i as f64 / 60.0, beat);
    }
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.3, 0.3, 0.3]), "{c:?}");
}

#[test]
fn a_lower_level_keeps_the_state_and_the_trail() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = counter("level");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    for i in 0..3 {
        frame(&mut scene, gpu, &screen, f64::from(i) * 0.1, 0.0);
    }
    let half = Level {
        factor: 0.5,
        quality: 0.25,
    };
    scene.set_level(&device, &queue, half);
    assert_eq!(scene.internal_size(), (32, 16));
    // The trail carried over to the smaller buffers and grows on; the frames count on.
    frame(&mut scene, gpu, &screen, 0.3, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.4, 0.4, 0.0]), "{c:?}");
    // And back up, also across a resize, which keeps the level.
    scene
        .resize(&device, &queue, SIZE.0 * 2, SIZE.1 * 2)
        .unwrap();
    assert_eq!(scene.internal_size(), SIZE);
    scene.resize(&device, &queue, SIZE.0, SIZE.1).unwrap();
    scene.set_level(&device, &queue, Level::FULL);
    assert_eq!(scene.internal_size(), SIZE);
    frame(&mut scene, gpu, &screen, 0.4, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.5, 0.5, 0.0]), "{c:?}");
}

fn state_offsets<const N: usize>(files: &Files, names: [&str; N]) -> [u64; N] {
    let (name, update) = files.update.as_ref().unwrap();
    let module = wgsl::update_source(&files.manifest, files.state(), (name, update))
        .check()
        .unwrap();
    let state = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("State"))
        .unwrap()
        .1;
    let TypeInner::Struct { members, .. } = &state.inner else {
        panic!("State must be a struct");
    };
    names.map(|name| {
        u64::from(
            members
                .iter()
                .find(|m| m.name.as_deref() == Some(name))
                .unwrap()
                .offset,
        )
    })
}

#[test]
fn torii_sea_spirit_stays_with_the_formation_at_long_frames() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let source = crate::builtin::find("Torii Sea").unwrap().source;
    let files = Files::read(&source).unwrap();
    let [flight, spirit, velocity] = state_offsets(&files, ["s", "spirit", "spirit_vel"]);
    let screen = Screen::new(&device);
    let frames = 61;
    let stride = velocity + 16;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Torii Sea spirit motion"),
        size: frames * stride,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    for tuned in [false, true] {
        let mut files = files.clone();
        if tuned {
            for id in ["stiff", "damp"] {
                let param = files
                    .manifest
                    .params
                    .iter_mut()
                    .find(|p| p.id == id)
                    .unwrap();
                param.default = TomlValue::Float(param.max.unwrap());
            }
        }
        let mut scene =
            Scene::build(&device, &queue, source.clone(), files, FORMAT, SIZE, 48_000).unwrap();
        let start = Instant::now();
        // A loud passage with live kicks every half second and no chart. The scene's own
        // bindings and shader drive the moving targets.
        for i in 0..frames {
            let time = i as f64 / 20.0;
            let rhythm = Rhythm {
                kick: (-(time * 2.0).fract() * 8.0).exp(),
                energy: 2.0,
                ..Default::default()
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            scene.render(
                &device,
                &queue,
                &mut encoder,
                &screen.view,
                &FrameInput {
                    time,
                    rhythm: &rhythm,
                    chart: None,
                    song: (time, true),
                    song_chart: None,
                    song_seed: Some(473),
                    accent: None,
                    wall: start + Duration::from_secs_f64(time),
                },
                None,
            );
            encoder.copy_buffer_to_buffer(&scene.state, 0, &readback, i * stride, stride);
            queue.submit(Some(encoder.finish()));
        }
        readback
            .slice(..)
            .map_async(MapMode::Read, |result| result.unwrap());
        device.poll(PollType::wait_indefinitely()).unwrap();
        let data = readback.slice(..).get_mapped_range().unwrap();
        let read = |i, offset| {
            let at = (i * stride + offset) as usize;
            f32::from_le_bytes(data[at..at + 4].try_into().unwrap())
        };
        let z = |i| read(i, spirit + 8);
        for i in 0..frames {
            let [x, y, lead] = [read(i, spirit), read(i, spirit + 4), z(i) - read(i, flight)];
            let v = [0, 4, 8].map(|axis| read(i, velocity + axis));
            // At these settings the leader flies within a few gates of the track and travels
            // far less than a gate per frame. Leave room for its wander, excursions and leaps.
            assert!(
                [x, y, lead].into_iter().all(|p| p.abs() < 50.0)
                    && v.into_iter().all(|speed| speed.abs() < 200.0),
                "tuned={tuned}, frame {i}: spirit (x={x}, y={y}, lead={lead}), velocity {v:?}"
            );
        }
        assert!(
            z(frames - 1) - z(0) > 10.0,
            "the spirit did not fly along the gates"
        );
        drop(data);
        readback.unmap();
    }
}

#[test]
fn torii_sea_trails_span_the_configured_duration() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let source = crate::builtin::find("Torii Sea").unwrap().source;
    let screen = Screen::new(&device);
    for (fps, short) in [(60_u32, false), (60, true), (30, true)] {
        let mut scene =
            Scene::load_from(&device, &queue, source.clone(), FORMAT, SIZE, 48_000).unwrap();
        let manifest = &scene.files.manifest;
        let trail = manifest
            .params
            .iter()
            .position(|p| p.id == "trail_time")
            .unwrap();
        let mut params = scene.files.param_values();
        if short {
            params[trail] = Value::Float(manifest.params[trail].min.unwrap() as f32);
            queue.write_buffer(&scene.params_buffer, 0, &scene.params.pack(&params));
        }
        let Value::Float(duration) = params[trail] else {
            panic!("trail_time must be in seconds");
        };
        let [path, slot, spirit] = state_offsets(&scene.files, ["path", "slot", "spirit"]);

        let last = fps * 4;
        let points = 40;
        let stride = path + points * 16;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Torii Sea history"),
            size: u64::from(last + 1) * stride,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let start = Instant::now();
        let mut rhythm = Rhythm {
            bpm: 120.0,
            confidence: 1.0,
            ..Default::default()
        };
        for i in 0..=last {
            let time = f64::from(i) / f64::from(fps);
            rhythm.beat = time * 2.0;
            let mut encoder = device.create_command_encoder(&Default::default());
            scene.render(
                &device,
                &queue,
                &mut encoder,
                &screen.view,
                &FrameInput {
                    time,
                    rhythm: &rhythm,
                    chart: None,
                    song: (time, true),
                    song_chart: None,
                    song_seed: Some(473),
                    accent: None,
                    wall: start + Duration::from_secs_f64(time),
                },
                None,
            );
            // Read the state up to the end of the first spirit's history.
            encoder.copy_buffer_to_buffer(
                &scene.state,
                0,
                &readback,
                u64::from(i) * stride,
                stride,
            );
            queue.submit(Some(encoder.finish()));
        }
        readback
            .slice(..)
            .map_async(MapMode::Read, |result| result.unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        let data = readback.slice(..).get_mapped_range().unwrap();
        let word = |at: u64| data[at as usize..at as usize + 4].try_into().unwrap();
        let end = u64::from(last) * stride;
        let slot = u64::from(u32::from_le_bytes(word(end + slot)));
        // Find the closest observed spirit position to each end of the stored history.
        // Read all three coordinates because spirits can double back along any one axis.
        let position = |at| [0, 4, 8].map(|delta| f32::from_le_bytes(word(at + delta)));
        let sample_frame = |slot| {
            let point = position(end + path + slot * 16);
            let distance = |i| {
                position(u64::from(i) * stride + spirit)
                    .into_iter()
                    .zip(point)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
            };
            (0..=last)
                .min_by(|&i, &j| distance(i).total_cmp(&distance(j)))
                .unwrap() as f32
        };
        // After the ring fills, the slot after the newest is the oldest.
        let span_frames = sample_frame(slot) - sample_frame((slot + 1) % points);
        let span = span_frames / fps as f32;
        assert!(
            (span_frames - duration * fps as f32).abs() <= 1.0,
            "Torii Sea at {fps} fps: history spans {span:.3} s, configured for {duration:.3} s",
        );
        eprintln!(
            "Torii Sea at {fps} fps: history spans {span:.3} s, configured for {duration:.3} s"
        );
    }
}

#[test]
fn a_scene_sees_its_resolution_and_quality() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let dir = folder(
        "quality",
        &[
            (
                MANIFEST,
                "format = 1\nname = \"Quality\"\nlicense = \"MPL-2.0\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = { intensity = 0.0 }\n",
            ),
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  return vec4f(frame.quality, frame.resolution / vec2f(64.0, 32.0), 1.0);\n}\n",
            ),
        ],
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    frame(&mut scene, (&device, &queue), &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [1.0, 1.0, 1.0]), "{c:?}");
    let level = Level {
        factor: 0.5,
        quality: 0.25,
    };
    scene.set_level(&device, &queue, level);
    frame(&mut scene, (&device, &queue), &screen, 0.1, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.25, 0.5, 0.5]), "{c:?}");
}

#[test]
fn a_reload_keeps_the_state_unless_it_changed() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = counter("reload");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    frame(&mut scene, gpu, &screen, 0.1, 0.0);
    assert!(!scene.reload(&device, &queue).unwrap(), "nothing changed");

    // A new default only updates the parameters: the trail now grows by 0.2.
    fs::write(dir.join(MANIFEST), MANIFEST_TEXT.replace("0.1", "0.2")).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    frame(&mut scene, gpu, &screen, 0.2, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.4, 0.3, 0.0]), "{c:?}");

    // A new shader rebuilds the scene; the state reads the same, so it is kept.
    fs::write(
        dir.join("final.wgsl"),
        FINAL.replace("1.0);", "1.0) * 1.0;"),
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    frame(&mut scene, gpu, &screen, 0.3, 0.0);
    let c = screen.middle(&device, &queue);
    assert!((c[1] - 0.4).abs() < 0.02, "frames counted on: {c:?}");

    // A different State starts over.
    fs::write(
        dir.join("state.wgsl"),
        STATE.replace("beats: u32,", "beats: u32,\n  more: f32,"),
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    frame(&mut scene, gpu, &screen, 0.4, 0.0);
    let c = screen.middle(&device, &queue);
    assert!((c[1] - 0.1).abs() < 0.02, "frames counted from 0: {c:?}");
}

#[test]
fn a_broken_shader_names_its_file_and_line_and_keeps_the_scene() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let dir = counter("broken");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    fs::write(
        dir.join("trail.wgsl"),
        TRAIL.replace("params.step", "params.stpe"),
    )
    .unwrap();
    let message = format!("{:#}", scene.reload(&device, &queue).unwrap_err());
    assert!(message.starts_with("trail.wgsl:3:"), "{message}");
    // Still the working scene.
    let screen = Screen::new(&device);
    frame(&mut scene, (&device, &queue), &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.1, 0.1, 0.0]), "{c:?}");
}

#[test]
fn the_post_processing_makes_a_sensible_frame() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    // A bright square in the middle of black, through AgX, bloom and the rest.
    let dir = folder(
        "post",
        &[
            (
                MANIFEST,
                "format = 1\nname = \"Glow\"\nlicense = \"MPL-2.0\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n",
            ),
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  let inside = step(abs(px.p.x), 0.1) * step(abs(px.p.y), 0.1);\n  return vec4f(vec3f(8.0, 4.0, 2.0) * inside, 1.0);\n}\n",
            ),
        ],
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    frame(&mut scene, (&device, &queue), &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    // Bright but not clipped to white: AgX keeps the hue.
    assert!(
        c[0] > 0.5 && c[0] < 1.0 && c[0] > c[1] && c[1] > c[2],
        "{c:?}"
    );
}

#[test]
fn paperclip_exposes_the_picture_once_before_choosing_its_palette() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let mut manifest: toml::Table = include_str!("../../scenes/paperclip/scene.toml")
        .parse()
        .unwrap();
    manifest.remove("effects");
    let dir = folder(
        "paperclip-exposure",
        &[
            (
                "state.wgsl",
                include_str!("../../scenes/paperclip/state.wgsl"),
            ),
            (
                "update.wgsl",
                include_str!("../../scenes/paperclip/update.wgsl"),
            ),
            (
                "clip.wgsl",
                include_str!("../../scenes/paperclip/clip.wgsl"),
            ),
            (
                "pixel.wgsl",
                include_str!("../../scenes/paperclip/pixel.wgsl"),
            ),
            // Replace only the geometry pass with a midtone, an HDR highlight and black.
            (
                "view.wgsl",
                "fn shade(px: Pixel) -> vec4f { return vec4f(0.125, 3.0, 0.0, 1.0); }",
            ),
        ],
    );
    let screen = Screen::new(&device);
    for ev in [0.0_f64, 1.0, -1.0] {
        let exposure = manifest
            .get_mut("param")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|param| param["id"].as_str() == Some("exposure"))
            .unwrap();
        exposure["default"] = TomlValue::Float(ev);
        fs::write(dir.join(MANIFEST), toml::to_string(&manifest).unwrap()).unwrap();
        let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
        frame(&mut scene, (&device, &queue), &screen, 0.0, 0.0);

        // At quarter resolution these sample cells (8, 4) and (9, 4), the first two
        // entries of the Bayer pattern. The default seven levels divide sRGB into sixths.
        for (x, bayer) in [(32, 0.0), (36, 8.0)] {
            let exposed = 0.125 * ev.exp2();
            let srgb = 1.055 * exposed.powf(1.0 / 2.4) - 0.055;
            let level = (srgb * 6.0 + (bayer + 0.5) / 16.0).floor();
            let expected = [(255.0 * level / 6.0).round() as u8, 255, 0];
            let actual = screen.pixel_srgb(&device, &queue, x, 16);
            // The compositor's final 8-bit dither can move a channel by one byte.
            assert!(
                actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "Paperclip at {ev:+} EV, x={x}: got {actual:?}, expected {expected:?}"
            );
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_linear_asset_keeps_its_values_until_the_scene_rebuilds() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let manifest = r#"
format = 1
name = "Data"
license = "MPL-2.0"

[[asset]]
id = "field"
file = "field.png"
linear = true

[[param]]
id = "gain"
default = 1.0

[[pass]]
shader = "a.wgsl"
target = "out"

[post]
tonemap = "none"
grain = 0.0
chromatic = 0.0
vignette = 0.0
bloom = { intensity = 0.0 }
"#;
    let shader = r#"
fn shade(px: Pixel) -> vec4f {
    let field = textureSampleLevel(asset_field, linear_clamp, px.uv, 0.0).rgb;
    return vec4f(field.r, field.g * params.gain, field.b, 1.0);
}
"#;
    let dir = folder("asset", &[(MANIFEST, manifest), ("a.wgsl", shader)]);
    let image_path = dir.join("field.png");
    RgbaImage::from_pixel(8, 8, image::Rgba([128, 128, 128, 255]))
        .save(&image_path)
        .unwrap();
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    let pixel = screen.middle(&device, &queue);
    // Mid grey as data reads about 0.502.
    assert!(close(pixel, [0.502; 3]), "original asset: {pixel:?}");

    RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
        .save(&image_path)
        .unwrap();
    assert!(!scene.reload(&device, &queue).unwrap(), "image-only edit");
    frame(&mut scene, gpu, &screen, 0.1, 0.0);
    let pixel = screen.middle(&device, &queue);
    assert!(close(pixel, [0.502; 3]), "after image edit: {pixel:?}");

    fs::write(
        dir.join(MANIFEST),
        manifest.replace("default = 1.0", "default = 0.5"),
    )
    .unwrap();
    assert!(
        scene.reload(&device, &queue).unwrap(),
        "new parameter default"
    );
    frame(&mut scene, gpu, &screen, 0.2, 0.0);
    let pixel = screen.middle(&device, &queue);
    assert!(
        close(pixel, [0.502, 0.251, 0.502]),
        "after defaults edit: {pixel:?}"
    );

    fs::write(dir.join("a.wgsl"), format!("{shader}\n// rebuild\n")).unwrap();
    assert!(scene.reload(&device, &queue).unwrap(), "shader edit");
    frame(&mut scene, gpu, &screen, 0.3, 0.0);
    let pixel = screen.middle(&device, &queue);
    assert!(
        close(pixel, [1.0, 0.0, 0.0]),
        "replacement asset: {pixel:?}"
    );
}

#[test]
fn a_scene_sees_the_accent_colour() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let dir = folder(
        "accent",
        &[
            (
                MANIFEST,
                "format = 1\nname = \"Accent\"\nlicense = \"MPL-2.0\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = { intensity = 0.0 }\n",
            ),
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  return vec4f(frame.accent.rgb * frame.accent.a, 1.0);\n}\n",
            ),
        ],
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    let rhythm = Rhythm::default();
    let mut encoder = device.create_command_encoder(&Default::default());
    // #0080ff: blue, green at half (0.216 once linear), no red.
    scene.render(
        &device,
        &queue,
        &mut encoder,
        &screen.view,
        &FrameInput {
            time: 0.0,
            rhythm: &rhythm,
            chart: None,
            song: (0.0, true),
            song_chart: None,
            song_seed: None,
            accent: Some(argb_to_linear(0xff0080ff)),
            wall: Instant::now(),
        },
        None,
    );
    queue.submit(Some(encoder.finish()));
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.0, 0.216, 1.0]), "{c:?}");
}

#[test]
fn a_scene_reads_the_covers_atlas() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let manifest = |covers: bool| {
        format!(
            "format = 1\nname = \"Covers\"\nlicense = \"MPL-2.0\"\ncovers = {covers}\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = {{ intensity = 0.0 }}\n"
        )
    };
    // Red from the middle of cell 0, green is half its green plus the number of covers in tenths,
    // blue is from the middle of cell 1.
    let shader = "fn shade(px: Pixel) -> vec4f {\n  let c = textureSampleLevel(covers, linear_clamp, vec2f(0.0625, 0.0625), 0.0);\n  let d = textureSampleLevel(covers, linear_clamp, vec2f(0.1875, 0.0625), 0.0);\n  return vec4f(c.r, c.g * 0.5 + f32(frame.covers) * 0.1, d.b, 1.0);\n}\n";
    let screen = Screen::new(&device);
    let draw = |scene: &mut Scene| {
        let rhythm = Rhythm::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        scene.render(
            &device,
            &queue,
            &mut encoder,
            &screen.view,
            &FrameInput {
                time: 0.0,
                rhythm: &rhythm,
                chart: None,
                song: (0.0, true),
                song_chart: None,
                song_seed: None,
                accent: None,
                wall: Instant::now(),
            },
            None,
        );
        queue.submit(Some(encoder.finish()));
        screen.middle(&device, &queue)
    };
    let red = RgbaImage::from_pixel(50, 50, image::Rgba([255, 0, 0, 255]));
    let blue = RgbaImage::from_pixel(50, 50, image::Rgba([0, 0, 255, 255]));
    let atlas = Arc::new(covers::pack(Some(&red), &[blue.clone(), blue]).unwrap());
    assert_eq!(atlas.count, 3);

    let dir = folder("covers", &[(MANIFEST, &manifest(true)), ("a.wgsl", shader)]);
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    assert!(scene.wants_covers());
    // Without covers: nothing valid, a clear texture.
    assert!(close(draw(&mut scene), [0.0, 0.0, 0.0]));
    scene.set_covers(&device, &queue, Some(Arc::clone(&atlas)), None);
    assert!(close(draw(&mut scene), [1.0, 0.3, 1.0]));

    // The playing cover replaces cell 0 only: the green cell 1 stays, and a later atlas does not
    // bring the old cell 0 back.
    let green = RgbaImage::from_pixel(50, 50, image::Rgba([0, 255, 0, 255]));
    let cell = Arc::new(covers::Cell::new(Some(&green)));
    scene.set_playing_cover(&device, &queue, Some(Arc::clone(&cell)));
    assert!(close(draw(&mut scene), [0.0, 0.8, 1.0]));
    scene.set_covers(&device, &queue, Some(Arc::clone(&atlas)), Some(cell));
    assert!(close(draw(&mut scene), [0.0, 0.8, 1.0]));
    // A track without a cover leaves cell 0 clear, still counted.
    scene.set_playing_cover(&device, &queue, Some(Arc::new(covers::Cell::new(None))));
    assert!(close(draw(&mut scene), [0.0, 0.3, 1.0]));

    // A scene that did not ask sees none, whatever it is given.
    let dir = folder(
        "no-covers",
        &[(MANIFEST, &manifest(false)), ("a.wgsl", shader)],
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    assert!(!scene.wants_covers());
    scene.set_covers(&device, &queue, Some(atlas), None);
    assert!(close(draw(&mut scene), [0.0, 0.0, 0.0]));
}

#[test]
fn a_scene_takes_over_the_music_of_the_scene_it_replaces() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let screen = Screen::new(&device);
    let mut old =
        Scene::load(&device, &queue, &counter("music-old"), FORMAT, SIZE, 48_000).unwrap();
    for (i, beat) in [0.5, 1.2, 3.1].into_iter().enumerate() {
        frame(&mut old, gpu, &screen, i as f64 / 60.0, beat);
    }
    assert!(old.music().song_time > 0.0);
    let heard = *old.music();

    // The new scene counts the beat across 4.0 from where the old one was (3.1); a scene with
    // a counter of its own would start there and count nothing.
    let mut next =
        Scene::load(&device, &queue, &counter("music-new"), FORMAT, SIZE, 48_000).unwrap();
    next.take_music_from(&mut old);
    assert_eq!(*next.music(), heard);
    frame(&mut next, gpu, &screen, 3.0 / 60.0, 4.2);
    let c = screen.middle(&device, &queue);
    assert!(c[2] > 0.0, "the beat across 4.0 was not counted: {c:?}");

    // Another sample rate: the new scene keeps its own.
    let mut other = Scene::load(
        &device,
        &queue,
        &counter("music-rate"),
        FORMAT,
        SIZE,
        44_100,
    )
    .unwrap();
    other.take_music_from(&mut next);
    assert_eq!(other.analyzer.sample_rate(), 44_100);
    assert_eq!(*other.music(), inputs::MusicValues::default());

    // The audio has moved to the rate the takeover is at, as the pipeline sets on a scene
    // built while another rate played (see `Pipeline::place`): the takeover swaps the analyzer
    // again, so what the music was heard as goes on in the scene coming in.
    let heard_next = *next.music();
    other.set_sample_rate(48_000);
    other.take_music_from(&mut next);
    assert_eq!(other.analyzer.sample_rate(), 48_000);
    assert_eq!(*other.music(), heard_next);
}

/// A scene whose update copies the shot's index and size into the state, and whose pass shows
/// the state's index in red, its size in green and the pass's own view of the index in blue.
fn shots_scene(name: &str, shots: &str) -> PathBuf {
    folder(
        name,
        &[
            (
                MANIFEST,
                &format!(
                    "format = 1\nname = \"Shots\"\nlicense = \"MPL-2.0\"\n[state]\nfile = \"state.wgsl\"\nupdate = \"update.wgsl\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = {{ intensity = 0.0 }}\n{shots}"
                ),
            ),
            (
                "state.wgsl",
                "struct State {\n  index: u32,\n  size: u32,\n}\n",
            ),
            (
                "update.wgsl",
                "fn update() {\n  state.index = shot.index;\n  state.size = shot.size;\n}\n",
            ),
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  return vec4f(f32(state.index) * 0.5, f32(state.size) * 0.25, f32(shot.index) * 0.5, 1.0);\n}\n",
            ),
        ],
    )
}

#[test]
fn a_scene_with_shots_reads_them_and_one_without_reads_zeros() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let screen = Screen::new(&device);
    // The director starts on the wide shot, which is the second one here.
    let dir = shots_scene(
        "shots",
        "[camera]\ndutch = 0.3\n[[shot]]\nname = \"near\"\nsize = \"medium\"\n[[shot]]\nsize = \"wide\"\nmove = \"glide\"\n",
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.5, 0.0, 0.5]), "{c:?}");
    assert_eq!(scene.cut_this_frame(), None);

    let dir = shots_scene("no-shots", "");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.0, 0.0, 0.0]), "{c:?}");
    assert_eq!(scene.cut_this_frame(), None);
}

#[test]
fn a_scene_cut_is_reported_for_the_frame_it_happens_in() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let dir = shots_scene(
        "cut",
        "[[shot]]\nsize = \"wide\"\nmove = \"glide\"\n[[shot]]\nsize = \"close\"\nopens = true\nmove = \"glide\"\n",
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    // The section start cuts to the other shot that opens one. Cuts come from the music's values, which a test cannot choose here, so tick the
    // director the way the frame does and see that what it said reaches the scene's report.
    let director = scene.decider.director_mut().unwrap();
    let music = MusicValues {
        cues: 1,
        cue: 1.0,
        ..MusicValues::default()
    };
    let on = director.state().index;
    let cut = director.tick(0.016, &music).unwrap();
    // Both open a section, so it cuts to the one it was not on.
    assert_eq!(cut.index, 1 - on);
    assert_eq!(
        cut.size,
        [ShotSize::Wide, ShotSize::Close][cut.index as usize]
    );
    assert_eq!(cut.motion, ShotMove::Glide);
}

#[test]
fn a_held_shot_and_a_lab_pace_survive_a_reload_of_the_scene() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let shots = "[[shot]]\nsize = \"wide\"\n[[shot]]\nsize = \"close\"\nmove = \"glide\"\n";
    let dir = shots_scene("held-reload", shots);
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    assert!(scene.hold_shot(Some(1)));
    scene.cycle_pace();
    // A change that rebuilds the scene (a third shot) gets a new director, which holds again.
    fs::write(
        dir.join(MANIFEST),
        fs::read_to_string(dir.join(MANIFEST)).unwrap() + "[[shot]]\nsize = \"top\"\n",
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    let view = scene.director_view().unwrap();
    assert_eq!(view.shots.len(), 3);
    assert!(view.held);
    assert_eq!(view.state.index, 1);
    assert!(view.pace_overridden);
    // Released, it stays released through the next reload.
    scene.hold_shot(None);
    fs::write(
        dir.join(MANIFEST),
        fs::read_to_string(dir.join(MANIFEST)).unwrap() + "[[shot]]\nsize = \"medium\"\n",
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    assert!(!scene.director_view().unwrap().held);
}

/// A scene whose update copies the look into the state, and whose pass shows the state's `to`
/// in red (a half for each) and the pass's own view of `to` in green.
fn looks_scene(name: &str, looks: &str) -> PathBuf {
    folder(
        name,
        &[
            (
                MANIFEST,
                &format!(
                    "format = 1\nname = \"Looks\"\nlicense = \"MPL-2.0\"\n[state]\nfile = \"state.wgsl\"\nupdate = \"update.wgsl\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = {{ intensity = 0.0 }}\n{looks}"
                ),
            ),
            ("state.wgsl", "struct State {\n  to: u32,\n}\n"),
            ("update.wgsl", "fn update() {\n  state.to = look.to;\n}\n"),
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  return vec4f(f32(state.to) * 0.5, f32(look.to) * 0.5, look.mix, 1.0);\n}\n",
            ),
        ],
    )
}

#[test]
fn a_scene_with_looks_reads_them_and_one_without_reads_zeros() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let screen = Screen::new(&device);
    let dir = looks_scene(
        "looks",
        "[[look]]\nname = \"a\"\n[[look]]\nname = \"b\"\n[[look]]\nname = \"c\"\n",
    );
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    assert_eq!(scene.looks_view().unwrap().names, ["a", "b", "c"]);
    // A lab holds look 2 before the first frame: the scene opens in it, and the update and the
    // pass both read it.
    assert!(scene.hold_look(Some(2)));
    assert!(!scene.hold_look(Some(3)), "no such look");
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [1.0, 1.0, 1.0]), "{c:?}");
    // Held to another look, it is a change: the fade starts at 0, and `to` is told at once.
    assert!(scene.hold_look(Some(1)));
    frame(&mut scene, gpu, &screen, 0.016, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.5, 0.5, 0.0]), "{c:?}");
    let view = scene.looks_view().unwrap();
    assert!(view.held);
    assert_eq!(
        (view.state.prev, view.state.to, view.state.changes),
        (2, 1, 1)
    );
    assert!(scene.hold_look(None));
    assert!(!scene.looks_view().unwrap().held);

    let dir = looks_scene("no-looks", "");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.0, 0.0, 0.0]), "{c:?}");
    assert!(scene.looks_view().is_none());
    assert!(!scene.hold_look(Some(0)));
}

#[test]
fn a_held_look_survives_a_reload_of_the_scene() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let screen = Screen::new(&device);
    let dir = looks_scene("held-look", "[[look]]\n[[look]]\n");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    assert!(scene.hold_look(Some(1)));
    fs::write(
        dir.join(MANIFEST),
        fs::read_to_string(dir.join(MANIFEST)).unwrap() + "[[look]]\n[[look]]\n",
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    // Another number of looks: it starts over, not held.
    let view = scene.looks_view().unwrap();
    assert_eq!(view.names.len(), 4);
    assert!(!view.held);
    // The same number of looks (a change elsewhere): the look on and the hold stay.
    assert!(scene.hold_look(Some(3)));
    frame(&mut scene, gpu, &screen, 0.0, 0.0);
    let text = fs::read_to_string(dir.join(MANIFEST)).unwrap();
    fs::write(
        dir.join(MANIFEST),
        text.replace("tonemap = \"none\"", "tonemap = \"aces\""),
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    let view = scene.looks_view().unwrap();
    assert!(view.held);
    assert_eq!(view.state.to, 3);
}

/// The counter with two inputs and two binds: `level` follows the meter (4 beats in a bar, from
/// 0 to 8: one half), `dist` adds the meter per second (so 4 each second), and `idle` has no
/// bind and stays at its rest.
const INPUT_TEXT: &str = r#"
[[input]]
id = "level"
range = [0.0, 1.0]

[[input]]
id = "dist"
range = [0.0, 100.0]

[[input]]
id = "idle"
range = [0.0, 1.0]
rest = 0.75

[[bind]]
target = "level"
source = "meter"
how = "follow"
when = { from = [0, 8] }

[[bind]]
target = "dist"
source = "meter"
how = "accumulate"
when = { per = "second" }
"#;
/// Red from the trail, green is `level`, blue is `idle`.
const INPUT_FINAL: &str = "fn shade(px: Pixel) -> vec4f {\n  let t = textureLoad(trail, vec2i(px.coord), 0);\n  return vec4f(t.r, input.level, input.idle, 1.0);\n}\n";

fn with_inputs(name: &str) -> PathBuf {
    folder(
        name,
        &[
            (MANIFEST, &format!("{MANIFEST_TEXT}{INPUT_TEXT}")),
            ("state.wgsl", STATE),
            ("update.wgsl", UPDATE),
            ("trail.wgsl", TRAIL),
            ("final.wgsl", INPUT_FINAL),
        ],
    )
}

fn distance(scene: &Scene) -> f32 {
    scene
        .bindings()
        .iter()
        .find(|r| r.target == "dist")
        .unwrap()
        .tick
        .output
}

#[test]
fn the_inputs_the_binds_drive_reach_the_shaders() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = with_inputs("inputs");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    for i in 0..3 {
        frame(&mut scene, gpu, &screen, f64::from(i) / 60.0, 0.0);
    }
    // The trail has grown three steps; `level` is half the meter's range; `idle` is at rest.
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.3, 0.5, 0.75]), "{c:?}");
    // What a lab reads: no chart, so level C, each bind with its source and its output.
    let report = scene.bindings();
    assert_eq!(report.len(), 2);
    assert_eq!(
        (report[0].target.as_str(), report[0].source, report[0].how),
        ("level", "meter", "follow")
    );
    assert_eq!(report[0].tick.level, bind::Level::C);
    assert_eq!(report[0].tick.used, bind::Used::Source);
    assert!((report[0].tick.output - 0.5).abs() < 1e-6);
    // Four per second, over three frames of a sixtieth.
    assert!(
        (distance(&scene) - 0.2).abs() < 1e-3,
        "{}",
        distance(&scene)
    );
}

#[test]
fn a_labs_binds_read_params_and_earlier_inputs_and_follow_the_params_live() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = with_inputs("bind-scoped");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    let mut time = 0.0;
    let mut next = |scene: &mut Scene| {
        frame(scene, gpu, &screen, time, 0.0);
        time += 1.0 / 60.0;
    };
    let output = |scene: &Scene, target: &str| {
        scene
            .bindings()
            .iter()
            .find(|r| r.target == target)
            .unwrap()
            .tick
            .output
    };
    // `level` is the meter (4) over 0..8 mapped to 0..step; `idle`, listed first, reads `level`.
    let text = INPUT_TEXT.split("[[bind]]").next().unwrap().to_string()
        + "[[bind]]\ntarget = \"idle\"\nsource = \"input:level\"\nhow = \"follow\"\nwhen = { from = [0, 0.1], to = [0, 1] }\n\n[[bind]]\ntarget = \"level\"\nsource = \"meter\"\nhow = \"follow\"\nwhen = { from = [0, 8], to = [0, { param = \"step\" }] }\n";
    assert!(!scene.set_bindings(&device, &queue, &text).unwrap());
    next(&mut scene);
    assert!((output(&scene, "level") - 0.05).abs() < 1e-5);
    assert!((output(&scene, "idle") - 0.5).abs() < 1e-4);
    // A param that is not there, or an input that comes later, is refused and the old stay.
    for wrong in [
        text.replace("param = \"step\"", "param = \"nope\""),
        text.replace("source = \"meter\"", "source = \"input:idle\""),
    ] {
        assert!(scene.set_bindings(&device, &queue, &wrong).is_err());
    }
    assert_eq!(scene.bindings().len(), 2);
    // The param changes in the app: the files on disk say so, and the next frame reads it.
    let manifest = format!("{MANIFEST_TEXT}{text}").replace("default = 0.1", "default = 0.2");
    fs::write(dir.join(MANIFEST), manifest).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    next(&mut scene);
    assert!((output(&scene, "level") - 0.1).abs() < 1e-5);
    assert!((output(&scene, "idle") - 1.0).abs() < 1e-4);
}

#[test]
fn changing_only_the_binds_keeps_the_pipelines_and_changing_an_input_rebuilds() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = with_inputs("bind-reload");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    let mut time = 0.0;
    let mut next = |scene: &mut Scene| {
        frame(scene, gpu, &screen, time, 0.0);
        time += 1.0 / 60.0;
        screen.middle(&device, &queue)
    };
    next(&mut scene);
    let c = next(&mut scene);
    assert!(close(c, [0.2, 0.5, 0.75]), "{c:?}");

    // A new bind (the range of the meter is now 16) is taken at once. The pipelines stay: the
    // trail, which a rebuilt scene starts again, grows on.
    let text = format!("{MANIFEST_TEXT}{INPUT_TEXT}").replace("from = [0, 8]", "from = [0, 16]");
    fs::write(dir.join(MANIFEST), &text).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    let c = next(&mut scene);
    assert!(close(c, [0.3, 0.25, 0.75]), "{c:?}");
    // The binds were remade, so the distance started over.
    assert!((distance(&scene) - 4.0 / 60.0).abs() < 1e-3);

    // A change to an input is a change of the struct: the scene is rebuilt (its trail starts
    // again), and the binds, which were made for the old input, start over.
    let rest = text.replace("rest = 0.75", "rest = 0.5");
    fs::write(dir.join(MANIFEST), rest).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    let c = next(&mut scene);
    assert!(close(c, [0.1, 0.25, 0.5]), "{c:?}");
    assert!((distance(&scene) - 4.0 / 60.0).abs() < 1e-3);

    // A change to a shader rebuilds too, and binds that did not change keep their state.
    fs::write(dir.join("final.wgsl"), format!("{INPUT_FINAL}\n")).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    next(&mut scene);
    let kept = distance(&scene);
    assert!((kept - 8.0 / 60.0).abs() < 1e-3, "{kept}");
}

#[test]
fn a_lab_replaces_the_binds_live_and_a_save_of_the_same_text_reloads_nothing() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let gpu = (&device, &queue);
    let dir = with_inputs("lab-binds");
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    let mut time = 0.0;
    let mut next = |scene: &mut Scene| {
        frame(scene, gpu, &screen, time, 0.0);
        time += 1.0 / 60.0;
        screen.middle(&device, &queue)
    };
    next(&mut scene);
    next(&mut scene);
    let before = distance(&scene);
    assert!(before > 0.0);
    // What a lab reads of the inputs and of the binds as written.
    let inputs = scene.inputs();
    assert_eq!(inputs.len(), 3);
    assert_eq!((inputs[2].id.as_str(), inputs[2].rest), ("idle", 0.75));
    assert!(
        inputs[2].declared.contains("rest = 0.75"),
        "{}",
        inputs[2].declared
    );
    let report = scene.bindings();
    assert_eq!(report[0].when, "{ from = [0, 8] }");
    assert_eq!(report[1].when, "{ per = \"second\" }");
    assert_eq!(report[0].fallback, "");

    // A new `level` (the meter over 0 to 16) is taken at once; `dist` is the same bind and keeps
    // its distance; the pipelines stay (the trail grows on).
    let text = INPUT_TEXT.replace("from = [0, 8]", "from = [0, 16]");
    let inputs_and_binds = text.clone();
    assert!(
        !scene
            .set_bindings(&device, &queue, &inputs_and_binds)
            .unwrap()
    );
    let c = next(&mut scene);
    assert!(close(c, [0.3, 0.25, 0.75]), "{c:?}");
    assert!(distance(&scene) > before, "the distance was kept");
    // The folder is watched: files that are as they were read do not undo what the lab changed.
    assert!(!scene.reload(&device, &queue).unwrap());
    assert_eq!(scene.bindings()[0].when, "{ from = [0, 16] }");
    // The file the lab saves says the same: the scene finds nothing to reload, and a later
    // change on disk still does.
    fs::write(dir.join(MANIFEST), format!("{MANIFEST_TEXT}{text}")).unwrap();
    assert!(!scene.reload(&device, &queue).unwrap());
    assert!(!scene.reload(&device, &queue).unwrap());

    // A wrong text is refused, naming the bind, and the old stay.
    let wrong = text.replace(
        "source = \"meter\"\nhow = \"follow\"",
        "source = \"loudness\"\nhow = \"follow\"",
    );
    assert_ne!(wrong, text);
    let message = format!(
        "{:#}",
        scene.set_bindings(&device, &queue, &wrong).unwrap_err()
    );
    assert!(
        message.contains("level") && message.contains("loudness"),
        "{message}"
    );
    let message = format!(
        "{:#}",
        scene
            .set_bindings(&device, &queue, "title = \"x\"")
            .unwrap_err()
    );
    assert!(message.contains("title"), "{message}");
    assert_eq!(scene.bindings().len(), 2);
    let c = next(&mut scene);
    assert!(close(c, [0.4, 0.25, 0.75]), "{c:?}");

    // A bind taken out leaves its input at rest; an input changed rebuilds the scene.
    let without = text.replace("[[bind]]\ntarget = \"dist\"\nsource = \"meter\"\nhow = \"accumulate\"\nwhen = { per = \"second\" }\n", "");
    assert_ne!(without, text);
    assert!(!scene.set_bindings(&device, &queue, &without).unwrap());
    assert_eq!(scene.bindings().len(), 1);
    let rest = without.replace("rest = 0.75", "rest = 0.5");
    assert!(scene.set_bindings(&device, &queue, &rest).unwrap());
    let c = next(&mut scene);
    assert!(close(c, [0.1, 0.25, 0.5]), "{c:?}");
    // The disk is what it was: the rebuilt scene does not go back to it. A change of the disk
    // does take the scene back to what it says.
    assert!(!scene.reload(&device, &queue).unwrap());
    assert_eq!(scene.inputs()[2].rest, 0.5);
    fs::write(
        dir.join(MANIFEST),
        format!("{MANIFEST_TEXT}{text}").replace("from = [0, 16]", "from = [0, 32]"),
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    assert_eq!(scene.inputs()[2].rest, 0.75);
}

#[test]
fn a_scene_without_inputs_or_binds_is_as_it_was() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let dir = counter("no-inputs");
    let scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    assert!(scene.bindings().is_empty());
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
        Self::load_from(
            device,
            queue,
            Source::Dir(dir.to_path_buf()),
            output_format,
            output_size,
            sample_rate,
        )
    }

    /// Keep a flare at its full swell, called before each frame (a probe).
    pub fn hold_flare(&mut self) {
        self.stepper.taint();
        self.decider.lens_mut().hold_flare();
    }

    /// Draw a frame into `target` (the output format and size). With `timestamps`, the first pass
    /// writes query 0 at its beginning and the last query 1 at its end.
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
}

#[test]
fn emoji_tunnel_flake_culling_keeps_visible_pixels() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/emoji-tunnel");
    let dir = folder("flake-bounds", &[]);
    fs::create_dir_all(dir.join("assets")).unwrap();
    for file in [
        "state.wgsl",
        "badge.wgsl",
        "assets/crests.png",
        "assets/emoji.png",
    ] {
        fs::copy(source.join(file), dir.join(file)).unwrap();
    }
    let mut manifest = fs::read_to_string(source.join("scene.toml"))
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    manifest.insert("name".into(), "Flake bounds".into());
    manifest.remove("render");
    manifest["post"] = r#"
tonemap = "none"
grain = 0.0
chromatic = 0.0
vignette = 0.0
bloom = { intensity = 0.0 }
"#
    .parse::<toml::Table>()
    .unwrap()
    .into();
    fs::write(dir.join("scene.toml"), toml::to_string(&manifest).unwrap()).unwrap();
    let update = fs::read_to_string(source.join("update.wgsl"))
        .unwrap()
        .replace("fn update()", "fn tunnel_update()");
    fs::write(
        dir.join("update.wgsl"),
        format!(
            "{update}\n{}",
            include_str!("../../tests/fixtures/flake-bounds-update.wgsl")
        ),
    )
    .unwrap();
    fs::write(
        dir.join("tunnel.wgsl"),
        include_str!("../../tests/fixtures/flake-bounds-view.wgsl"),
    )
    .unwrap();
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    frame(&mut scene, (&device, &queue), &screen, 0.0, 0.0);
    fs::remove_dir_all(dir).unwrap();

    // Red is a direct box hit, green means the circle kept it, blue means the tile listed it.
    let witnesses = [
        ("face-on corner (35, 12)", 35, 12),
        ("rotated corner across a tile edge (57, 1)", 57, 1),
        ("near-camera fallback (8, 28)", 8, 28),
    ];
    let observed = witnesses.map(|(label, x, y)| {
        (
            label,
            screen
                .pixel_srgb(&device, &queue, x, y)
                .map(|channel| channel > 128),
        )
    });
    assert!(
        observed.iter().all(|(_, channels)| *channels == [true; 3]),
        "expected (box hit, inside circle, listed in tile) for every witness: {observed:?}"
    );
}

#[test]
fn emoji_tunnel_keeps_answered_moments_until_the_move_ends() {
    use crate::chart::LatestOnset;
    use std::time::Duration;

    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/emoji-tunnel");
    let dir = folder("emoji-moment-history", &[]);
    let state = fs::read_to_string(source.join("state.wgsl")).unwrap();
    let update = fs::read_to_string(source.join("update.wgsl")).unwrap();
    let probe = include_str!("../../tests/fixtures/emoji-moment-history.wgsl");
    let (update_probe, view_probe) = probe.split_once("fn shade").unwrap();
    fs::write(
        dir.join("state.wgsl"),
        state
            .replace(
                "struct State {",
                "struct State {
  witness: array<vec2f, 3>,",
            )
            // The probe says which emoji answers which moment (see its `hash`).
            .replace(
                "fn hash(p: vec2f) -> f32 {",
                "fn scene_hash(p: vec2f) -> f32 {",
            ),
    )
    .unwrap();
    fs::write(
        dir.join("update.wgsl"),
        format!(
            "{}
{update_probe}",
            update.replace("fn update()", "fn tunnel_update()")
        ),
    )
    .unwrap();
    fs::write(dir.join("tunnel.wgsl"), format!("fn shade{view_probe}")).unwrap();
    let mut manifest = fs::read_to_string(source.join(MANIFEST))
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    // The probe uses the scene's update and bindings, without the artwork.
    manifest.remove("asset");
    manifest.remove("common");
    manifest.remove("render");
    manifest["post"] = r#"
tonemap = "none"
grain = 0.0
chromatic = 0.0
vignette = 0.0
bloom = { intensity = 0.0 }
"#
    .parse::<toml::Table>()
    .unwrap()
    .into();

    let screen = Screen::new(&device);
    let mut failures = Vec::new();
    for long_move in [false, true] {
        if long_move {
            let move_time = manifest["param"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|param| param["id"].as_str() == Some("move_time"))
                .unwrap();
            move_time["default"] = 2.0.into();
        }
        fs::write(dir.join(MANIFEST), toml::to_string(&manifest).unwrap()).unwrap();
        let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
        let start = Instant::now();
        let mut chart = ChartState {
            lead: 1.0, // Low onsets count as hits; harmonic onsets count as notes.
            loudness: 0.5,
            ..Default::default()
        };
        let mut notes = 0;
        for tick in 0u32..=323 {
            let time = f64::from(tick) / 100.0;
            if (111..=309).contains(&tick) && (tick - 111).is_multiple_of(11) {
                // Prime the melody rate once, then give 18 notes 0.11 seconds apart.
                chart.latest[3] = LatestOnset {
                    count: chart.latest[3].count + 1,
                    time,
                    strength: if tick == 122 { 0.4 } else { 0.5 },
                };
            }
            if tick == 309 {
                chart.latest[0] = LatestOnset {
                    count: 1,
                    time,
                    strength: 1.0,
                };
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            scene.render(
                &device,
                &queue,
                &mut encoder,
                &screen.view,
                &FrameInput {
                    time,
                    rhythm: &Rhythm::default(),
                    chart: Some(&chart),
                    song: (time, true),
                    song_chart: None,
                    song_seed: Some(1),
                    accent: None,
                    wall: start + Duration::from_millis(u64::from(tick) * 10),
                },
                None,
            );
            queue.submit(Some(encoder.finish()));
            notes += scene.music_values.notes;
            let expected: &[(u32, f32, f32)] = match (long_move, tick) {
                (_, 122) => &[(0, 0.0, 0.4), (1, 0.0, 0.4)],
                (false, 181) => &[(0, 0.59, 0.4)],
                (false, 183) => &[(0, 2.0, 0.0)],
                (true, 309) => &[(0, 1.87, 0.4), (1, 0.0, 0.5), (2, 0.0, 0.6)],
                (true, 321) => &[(0, 1.99, 0.4)],
                (true, 323) => &[(0, 2.0, 0.0)],
                _ => &[],
            };
            for &(emoji, age, strength) in expected {
                let colour = screen
                    .pixel_srgb(&device, &queue, 10 + 21 * emoji, SIZE.1 / 2)
                    .map(srgb_to_linear);
                // The same-frame counted hit has strength 0.6 to 1.1 depending on energy.
                let strength_matches = if emoji == 2 {
                    (0.59..=1.11).contains(&(2.0 * colour[1]))
                } else {
                    (2.0 * colour[1] - strength).abs() < 0.02
                };
                if (2.0 * colour[0] - age).abs() >= 0.02 || !strength_matches {
                    failures.push(format!(
                        "long move {long_move}, t={time:.2}s, emoji {emoji}: expected age {age:.2}s and strength {strength:.2}, got age {:.2}s and strength {:.2}",
                        2.0 * colour[0],
                        2.0 * colour[1],
                    ));
                }
            }
        }
        assert_eq!(notes, 19, "admitted notes, including the warm-up");
    }
    fs::remove_dir_all(dir).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn growth_camera_settles_after_a_growth_step() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let source = crate::builtin::find("Growth").unwrap().source;
    let files = Files::read(&source).unwrap();
    let follow_index = files
        .manifest
        .params
        .iter()
        .position(|param| param.id == "follow")
        .unwrap();
    let follow_param = &files.manifest.params[follow_index];
    let default_follow = follow_param.default.as_float().unwrap();
    let (update_name, update) = files.update.as_ref().unwrap();
    let module = wgsl::update_source(&files.manifest, files.state(), (update_name, update))
        .check()
        .unwrap();
    let state = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("State"))
        .unwrap()
        .1;
    let TypeInner::Struct { members, .. } = &state.inner else {
        panic!("Growth state must be a struct");
    };
    let offsets = ["top_to", "top", "cam_top", "cam_rise", "cam_pos"].map(|name| {
        members
            .iter()
            .find(|member| member.name.as_deref() == Some(name))
            .unwrap()
            .offset as usize
    });
    let screen = Screen::new(&device);

    // Compare the short follow at normal cadence and the default at slow cadence before
    // checking the short follow at 20 fps. Include the long end of the scene's range.
    for (setting, dt) in [
        (Some(0.05), 1.0 / 60.0),
        (None, 0.05),
        (follow_param.max, 0.05),
        (Some(0.05), 0.05),
    ] {
        let mut files = files.clone();
        let follow = setting.unwrap_or(default_follow);
        if let Some(value) = setting {
            assert!((follow_param.min.unwrap()..=follow_param.max.unwrap()).contains(&value));
            files.manifest.params[follow_index].default = TomlValue::Float(value);
        }
        let mut scene =
            Scene::build(&device, &queue, source.clone(), files, FORMAT, SIZE, 48_000).unwrap();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Growth state readback"),
            size: scene.state.size(),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read = |scene: &Scene| {
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&scene.state, 0, &readback, 0, scene.state.size());
            queue.submit(Some(encoder.finish()));
            readback
                .slice(..)
                .map_async(MapMode::Read, |result| result.unwrap());
            device.poll(PollType::wait_indefinitely()).unwrap();
            let data = readback.slice(..).get_mapped_range().unwrap();
            let [target, tip, height, velocity, y] = offsets;
            let values = [target, tip, height, velocity, y + 4]
                .map(|offset| f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()));
            drop(data);
            readback.unmap();
            values
        };

        let draw = |scene: &mut Scene, i, beat| {
            frame(scene, (&device, &queue), &screen, f64::from(i) * dt, beat);
        };
        assert!(scene.hold_shot(Some(0)));
        draw(&mut scene, 0, 0.5);
        let [start, _, _, _, _] = read(&scene);
        let steps = ((6.0 * follow).max(2.0) / dt).ceil() as u32;
        let mut highest_error = 0.0_f32;
        let mut fastest = 0.0_f32;
        for i in 1..=steps {
            // One beat crossing, then no other growth.
            draw(&mut scene, i, 1.2);
            let [_, tip, height, velocity, _] = read(&scene);
            if setting.is_none() && i == 1 {
                assert!(
                    start < height && height < tip,
                    "the default camera begins to follow without jumping to the tip"
                );
            }
            highest_error = highest_error.max((height - start).abs());
            fastest = fastest.max(velocity.abs());
        }
        let [target, tip, height, velocity, y] = read(&scene);
        let rise = target - start;
        assert!(
            rise > 0.0 && (tip - target).abs() < 0.01,
            "tower did not grow and settle"
        );
        let context =
            format!("follow {follow}s, dt {dt}s: tip {tip}, camera {height}, velocity {velocity}");
        assert!(
            (height - tip).abs() < 0.01,
            "camera did not settle: {context}"
        );
        assert!(
            velocity.abs() < 0.01,
            "camera did not come to rest: {context}"
        );
        assert!(
            highest_error <= rise + 0.01 && fastest <= 20.0 * rise,
            "camera did not stay bounded: {context}"
        );

        if setting.is_none() {
            assert!(scene.hold_shot(Some(3)));
            draw(&mut scene, steps + 1, 1.2);
            let [_, _, after, _, after_y] = read(&scene);
            assert!(
                (after - height).abs() < 0.01 && after_y > y + 5.0,
                "the cut must change the view immediately without moving its follow height"
            );
        }
    }
}

#[test]
fn neon_street_drops_the_old_trail_on_a_cut() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    let source = crate::builtin::find("Neon Street").unwrap().source;
    let mut files = Files::read(&source).unwrap();
    // Feed the real compose pass white, black starting on the cut, then mid-gray
    // after a few frames. Read its output without grading to see small amounts of history.
    for (name, shader) in &mut files.shaders {
        match name.as_str() {
            "ink.wgsl" => {
                *shader = "fn shade(px: Pixel) -> vec4f {
                    let phase = frame.index % 11u;
                    let red = select(select(0.0, 0.5, phase == 10u), 1.0, phase == 0u);
                    return vec4f(vec3f(red), 1.0);
                }"
                .into();
            }
            "grade.wgsl" => {
                *shader = "fn shade(px: Pixel) -> vec4f {
                    return textureSampleLevel(trail, linear_clamp, px.uv, 0.0);
                }"
                .into();
            }
            _ => {}
        }
    }
    files.manifest.post.tonemap = manifest::Tonemap::None;
    files.manifest.post.bloom.intensity = 0.0;
    files.manifest.post.grain = 0.0;
    files.manifest.post.vignette = 0.0;
    files.manifest.post.chromatic = 0.0;
    files.manifest.post.contrast = 1.0;
    files.manifest.post.saturation = 1.0;

    let mut scene = Scene::build(&device, &queue, source, files, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    let mut params = scene.files.param_values();
    let blur = scene
        .files
        .manifest
        .params
        .iter()
        .position(|p| p.id == "blur")
        .unwrap();
    let default_blur = params[blur];
    let mut time = 0.0;
    let red = || screen.pixel_srgb(&device, &queue, SIZE.0 / 2, SIZE.1 / 2)[0];
    let mut draw = |fps, shot, no_blur| {
        params[blur] = if no_blur {
            Value::Float(0.0)
        } else {
            default_blur
        };
        queue.write_buffer(&scene.params_buffer, 0, &scene.params.pack(&params));
        let mut render = |scene: &mut Scene| {
            time += 1.0 / f64::from(fps);
            frame(scene, (&device, &queue), &screen, time, 0.5);
        };
        assert!(scene.hold_shot(Some(0)));
        render(&mut scene);
        let before = red();
        assert!(
            before >= 220,
            "the previous trail should be bright: {before}"
        );
        assert!(scene.hold_shot(Some(shot)));
        render(&mut scene);
        let cut = red();
        // Give the blur time to become visible above the output's quantization.
        for _ in 0..8 {
            render(&mut scene);
        }
        render(&mut scene);
        [cut, red()]
    };

    let shots = [1, 3, 7];
    let samples = [60, 120].map(|fps| {
        let on = shots.map(|shot| draw(fps, shot, false));
        (fps, on, draw(fps, 1, true))
    });
    // The output shader dithers by up to one sRGB code.
    assert!(
        samples
            .iter()
            .all(|(_, on, off)| on.iter().chain([off]).all(|sample| sample[0] <= 1)),
        "Neon Street left old trail on the cut (fps, blur on for shots {shots:?}, blur off; cut and recovery): {samples:?}"
    );
    for (fps, on, off) in samples {
        // Linear 0.5 encodes to sRGB 188; two dithered samples may differ by two codes.
        assert!(
            off[1].abs_diff(188) <= 1,
            "Neon Street at {fps} fps should have no trail with blur off: {off:?}"
        );
        for (shot, high) in shots.into_iter().zip(on).skip(1) {
            assert!(
                on[0][1] < high[1] && off[1].saturating_sub(high[1]) > 2,
                "Neon Street at {fps} fps should recover with less blur in shot {shot} than at street level: street {:?}, high {high:?}, blur off {off:?}",
                on[0]
            );
        }
    }
}
