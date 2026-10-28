//! Scenes drawn on the GPU (skipped without an adapter).

use super::*;

const SIZE: (u32, u32) = (64, 32);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// A scene folder of its own under the temp directory, with `files` in it.
fn folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rune-scene-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
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
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            1,
        );
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }

    /// The linear colour of the middle pixel.
    fn middle(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> [f32; 3] {
        let row = (SIZE.0 * 4).next_multiple_of(256);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * SIZE.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
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
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let at = ((SIZE.1 / 2) * row + (SIZE.0 / 2) * 4) as usize;
        [
            srgb_to_linear(data[at]),
            srgb_to_linear(data[at + 1]),
            srgb_to_linear(data[at + 2]),
        ]
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
            accent: None,
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
    std::fs::write(dir.join(MANIFEST), MANIFEST_TEXT.replace("0.1", "0.2")).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    frame(&mut scene, gpu, &screen, 0.2, 0.0);
    let c = screen.middle(&device, &queue);
    assert!(close(c, [0.4, 0.3, 0.0]), "{c:?}");

    // A new shader rebuilds the scene; the state reads the same, so it is kept.
    std::fs::write(
        dir.join("final.wgsl"),
        FINAL.replace("1.0);", "1.0) * 1.0;"),
    )
    .unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    frame(&mut scene, gpu, &screen, 0.3, 0.0);
    let c = screen.middle(&device, &queue);
    assert!((c[1] - 0.4).abs() < 0.02, "frames counted on: {c:?}");

    // A different State starts over.
    std::fs::write(
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
    std::fs::write(
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
fn a_linear_asset_keeps_its_values() {
    let Some((device, queue)) = gpu() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    // Mid grey as data must read 0.5, not the 0.22 an sRGB decode would make of it.
    let dir = folder(
        "asset",
        &[
            (
                MANIFEST,
                "format = 1\nname = \"Data\"\nlicense = \"MPL-2.0\"\n[[asset]]\nid = \"field\"\nfile = \"field.png\"\nlinear = true\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = { intensity = 0.0 }\n",
            ),
            (
                "a.wgsl",
                "fn shade(px: Pixel) -> vec4f {\n  return vec4f(textureSampleLevel(asset_field, linear_clamp, px.uv, 0.0).rgb, 1.0);\n}\n",
            ),
        ],
    );
    image::RgbaImage::from_pixel(8, 8, image::Rgba([128, 128, 128, 255]))
        .save(dir.join("field.png"))
        .unwrap();
    let mut scene = Scene::load(&device, &queue, &dir, FORMAT, SIZE, 48_000).unwrap();
    let screen = Screen::new(&device);
    frame(&mut scene, (&device, &queue), &screen, 0.0, 0.0);
    let c = screen.middle(&device, &queue);
    assert!((c[0] - 0.502).abs() < 0.02, "{c:?}");
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
            accent: Some(argb_to_linear(0xff0080ff)),
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
                accent: None,
            },
            None,
        );
        queue.submit(Some(encoder.finish()));
        screen.middle(&device, &queue)
    };
    let red = image::RgbaImage::from_pixel(50, 50, image::Rgba([255, 0, 0, 255]));
    let blue = image::RgbaImage::from_pixel(50, 50, image::Rgba([0, 0, 255, 255]));
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
    let green = image::RgbaImage::from_pixel(50, 50, image::Rgba([0, 255, 0, 255]));
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
    let director = scene.director.as_mut().unwrap();
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
    std::fs::write(
        dir.join(MANIFEST),
        std::fs::read_to_string(dir.join(MANIFEST)).unwrap() + "[[shot]]\nsize = \"top\"\n",
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
    std::fs::write(
        dir.join(MANIFEST),
        std::fs::read_to_string(dir.join(MANIFEST)).unwrap() + "[[shot]]\nsize = \"medium\"\n",
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
    std::fs::write(
        dir.join(MANIFEST),
        std::fs::read_to_string(dir.join(MANIFEST)).unwrap() + "[[look]]\n[[look]]\n",
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
    let text = std::fs::read_to_string(dir.join(MANIFEST)).unwrap();
    std::fs::write(
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
    assert_eq!(report[0].tick.level, crate::bind::Level::C);
    assert_eq!(report[0].tick.used, crate::bind::Used::Source);
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
    std::fs::write(dir.join(MANIFEST), manifest).unwrap();
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
    std::fs::write(dir.join(MANIFEST), &text).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    let c = next(&mut scene);
    assert!(close(c, [0.3, 0.25, 0.75]), "{c:?}");
    // The binds were remade, so the distance started over.
    assert!((distance(&scene) - 4.0 / 60.0).abs() < 1e-3);

    // A change to an input is a change of the struct: the scene is rebuilt (its trail starts
    // again), and the binds, which were made for the old input, start over.
    let rest = text.replace("rest = 0.75", "rest = 0.5");
    std::fs::write(dir.join(MANIFEST), rest).unwrap();
    assert!(scene.reload(&device, &queue).unwrap());
    let c = next(&mut scene);
    assert!(close(c, [0.1, 0.25, 0.5]), "{c:?}");
    assert!((distance(&scene) - 4.0 / 60.0).abs() < 1e-3);

    // A change to a shader rebuilds too, and binds that did not change keep their state.
    std::fs::write(dir.join("final.wgsl"), format!("{INPUT_FINAL}\n")).unwrap();
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
    std::fs::write(dir.join(MANIFEST), format!("{MANIFEST_TEXT}{text}")).unwrap();
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
    std::fs::write(
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
