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
        },
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
