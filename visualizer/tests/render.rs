//! Headless GPU tests. They skip (and say so) when the machine has no GPU adapter.

use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rtrb::{Producer, RingBuffer};
use visualizer::{Frame, LabCommand, Location, PcmInput, RUNTIME_TOML, Visualizer};

const SAMPLE_RATE: u32 = 48_000;

/// A copy of the built-in scene under the name `name`, in a folder of its own that the test
/// removes afterwards.
fn copied_scene(name: &str) -> std::path::PathBuf {
    let from = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/armillary");
    let dir = std::env::temp_dir()
        .join(format!("rune-visualizer-{name}-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for file in std::fs::read_dir(from).unwrap() {
        let file = file.unwrap().path();
        let mut text = std::fs::read_to_string(&file).unwrap();
        if file.file_name().unwrap() == "scene.toml" {
            text = text.replacen("name = \"Armillary\"", &format!("name = \"{name}\""), 1);
        }
        std::fs::write(dir.join(file.file_name().unwrap()), text).unwrap();
    }
    dir
}

/// Feeds a stereo test signal (a decaying 110 Hz tone burst four times per second plus a little
/// noise) at real time speed.
struct Feeder {
    producer: Producer<f32>,
    samples_sent: u64,
    started: Instant,
    noise: u32,
    /// The test's turn at the GPU, held as long as it feeds the visualizer (see [`GPU`]).
    _turn: MutexGuard<'static, ()>,
}

/// The tests take turns at the GPU: several devices made at once on macOS were lost now and then
/// ("the driver implementation is at fault"), and a test that gets no GPU is skipped, so seven or
/// eight of the thirteen passed without running whenever they ran side by side.
static GPU: Mutex<()> = Mutex::new(());

impl Feeder {
    fn pump(&mut self) {
        let due = (self.started.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64;
        while self.samples_sent < due {
            let t = self.samples_sent as f32 / SAMPLE_RATE as f32;
            self.noise = self
                .noise
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let noise = (self.noise >> 16) as f32 / 65_536.0 - 0.5;
            let beat = (-(t * 4.0).fract() * 6.0).exp();
            let value = 0.8 * beat * (t * 110.0 * std::f32::consts::TAU).sin() + 0.05 * noise;
            let _ = self.producer.push(value);
            let _ = self.producer.push(value);
            self.samples_sent += 1;
        }
    }
}

fn start(width: u32, height: u32) -> Option<(Visualizer, Feeder)> {
    start_with(width, height, false)
}

/// `readback`: read frames back even on macOS (see `Visualizer::prefer_readback`).
fn start_with(width: u32, height: u32, readback: bool) -> Option<(Visualizer, Feeder)> {
    // A test that failed holding its turn poisons the lock; the next may still go.
    let turn = GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (producer, consumer) = RingBuffer::new(1 << 16);
    let input = PcmInput {
        consumer,
        channels: Arc::new(AtomicU32::new(2)),
        sample_rate: Arc::new(AtomicU32::new(SAMPLE_RATE)),
    };
    let visualizer = Visualizer::new();
    visualizer.prefer_readback(readback);
    visualizer.start(input, width, height, 60).unwrap();
    let feeder = Feeder {
        producer,
        samples_sent: 0,
        started: Instant::now(),
        noise: 1,
        _turn: turn,
    };
    // Wait for the first frame or for the failure to get a GPU.
    let deadline = Instant::now() + Duration::from_secs(30);
    while visualizer.latest_frame().is_none() {
        let status = visualizer.status();
        if !status.running {
            eprintln!(
                "SKIPPED: no usable GPU: {}",
                status.last_error.unwrap_or_default()
            );
            return None;
        }
        assert!(Instant::now() < deadline, "no frame within 30 s");
        std::thread::sleep(Duration::from_millis(10));
    }
    Some((visualizer, feeder))
}

/// Pull frames for `count` new frame ids while feeding audio. Returns the frames and the fps.
/// A frame's pixels, copied out right away: on macOS frames are IOSurface-backed pixel buffers
/// from a small pool, so holding on to them would stall the renderer like a slow reader would.
struct Captured {
    id: u64,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Captured {
    fn new(frame: &Frame) -> Self {
        Self {
            id: frame.id,
            width: frame.width,
            height: frame.height,
            pixels: frame.to_packed().unwrap(),
        }
    }
}

fn collect(visualizer: &Visualizer, feeder: &mut Feeder, count: usize) -> (Vec<Captured>, f64) {
    let mut frames: Vec<Captured> = Vec::new();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(60);
    while frames.len() < count {
        assert!(Instant::now() < deadline, "frames stopped coming");
        feeder.pump();
        if let Some(frame) = visualizer.latest_frame()
            && frames.last().is_none_or(|last| last.id < frame.id)
        {
            frames.push(Captured::new(&frame));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let fps = (frames.last().unwrap().id - frames[0].id) as f64 / started.elapsed().as_secs_f64();
    (frames, fps)
}

fn is_black(pixels: &[u8]) -> bool {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p[..3] == [0, 0, 0])
}

fn check_frames(frames: &[Captured], width: u32, height: u32) {
    for pair in frames.windows(2) {
        assert!(pair[0].id < pair[1].id);
    }
    for frame in frames {
        assert_eq!((frame.width, frame.height), (width, height));
        assert_eq!(frame.pixels.len(), (width * height * 4) as usize);
    }
    let last = frames.last().unwrap();
    assert!(!is_black(&last.pixels), "the last frame is black");
    assert!(
        frames.iter().any(|f| f.pixels != last.pixels),
        "the picture never changes"
    );
}

#[test]
fn default_and_loaded_scenes_render_changing_frames() {
    let Some((visualizer, mut feeder)) = start(640, 360) else {
        return;
    };
    let status = visualizer.status();
    assert!(status.running);
    assert_eq!(
        status.preset_name,
        visualizer::DEFAULT_PRESET_NAME,
        "{:?}",
        status.last_error
    );
    // macOS renders straight into IOSurface-backed pixel buffers.
    #[cfg(target_vendor = "apple")]
    assert_eq!(status.transport, Some(visualizer::Transport::IoSurface));

    let (frames, fps) = collect(&visualizer, &mut feeder, 60);
    check_frames(&frames, 640, 360);
    println!("default scene 640x360: {fps:.1} fps on {}", status.adapter);

    let dir = copied_scene("copy");
    visualizer.load_preset_path(&dir).unwrap();
    let (frames, fps) = collect(&visualizer, &mut feeder, 60);
    check_frames(&frames, 640, 360);
    let status = visualizer.status();
    assert_eq!(status.preset_name, "copy", "{:?}", status.last_error);
    println!("copied scene 640x360: {fps:.1} fps");

    visualizer.resize(1280, 720).unwrap();
    // Frames of the old size may still be in flight.
    let (frames, fps) = collect(&visualizer, &mut feeder, 90);
    check_frames(&frames[30..], 1280, 720);
    println!("copied scene 1280x720: {fps:.1} fps");

    let input = visualizer.stop();
    assert!(input.is_some());
    assert!(visualizer.latest_frame().is_none());
    assert!(!visualizer.status().running);
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn readback_renders_changing_frames() {
    // 640 is a multiple of 64, so rows have no padding; 630 has padding.
    for (width, height) in [(640, 360), (630, 354)] {
        let Some((visualizer, mut feeder)) = start_with(width, height, true) else {
            return;
        };
        assert_eq!(
            visualizer.status().transport,
            Some(visualizer::Transport::Readback)
        );
        let (frames, fps) = collect(&visualizer, &mut feeder, 60);
        check_frames(&frames, width, height);
        let latest = visualizer.latest_frame().unwrap();
        let (_, stride) = latest.pixels().unwrap();
        assert_eq!(stride == width as usize * 4, width % 64 == 0);
        println!("readback {width}x{height}: {fps:.1} fps, stride {stride}");
        visualizer.stop();
    }
}

#[test]
fn frame_listener_hears_every_frame() {
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    let visualizer = Arc::new(visualizer);
    let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
    visualizer.set_frame_listener(Some(Arc::new({
        let heard = Arc::clone(&heard);
        let visualizer = Arc::downgrade(&visualizer);
        move |id| {
            // Told after the frame became the latest one.
            let latest = visualizer.upgrade().and_then(|v| v.latest_frame());
            heard
                .lock()
                .unwrap()
                .push((id, latest.map(|frame| frame.id)));
        }
    })));
    collect(&visualizer, &mut feeder, 10);
    visualizer.set_frame_listener(None);
    let heard = std::mem::take(&mut *heard.lock().unwrap());
    assert!(heard.len() >= 9, "heard {} frames", heard.len());
    for pair in heard.windows(2) {
        assert_eq!(pair[1].0, pair[0].0 + 1, "a frame was not heard");
    }
    for (id, latest) in &heard {
        assert!(latest.is_some_and(|latest| latest >= *id));
    }
    visualizer.stop();
}

#[test]
fn transport_listener_hears_the_fall_back_to_readback() {
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
    visualizer.set_transport_listener(Some(Arc::new({
        let heard = Arc::clone(&heard);
        move |transport| heard.lock().unwrap().push(transport)
    })));
    let before = visualizer.status().transport.unwrap();
    visualizer.fall_back_to_readback();
    collect(&visualizer, &mut feeder, 5);
    let heard = std::mem::take(&mut *heard.lock().unwrap());
    if before == visualizer::Transport::Readback {
        // Nothing to fall back from on this machine.
        assert!(heard.is_empty());
    } else {
        assert_eq!(heard, [visualizer::Transport::Readback]);
    }
    assert_eq!(
        visualizer.status().transport,
        Some(visualizer::Transport::Readback)
    );
    visualizer.stop();
}

#[test]
fn paused_music_holds_the_picture() {
    let Some((visualizer, mut feeder)) = start(640, 360) else {
        return;
    };
    collect(&visualizer, &mut feeder, 10);
    visualizer.set_playing(false);
    // At most a frame in flight and one still frame, then nothing new.
    std::thread::sleep(Duration::from_millis(300));
    let held = visualizer.latest_frame().unwrap().id;
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        visualizer.latest_frame().unwrap().id,
        held,
        "drew while paused"
    );
    visualizer.set_playing(true);
    let (frames, _) = collect(&visualizer, &mut feeder, 10);
    assert!(frames.last().unwrap().id > held, "did not resume");
    visualizer.stop();
}

#[test]
fn hidden_window_holds_the_picture() {
    let Some((visualizer, mut feeder)) = start(640, 360) else {
        return;
    };
    collect(&visualizer, &mut feeder, 10);
    visualizer.set_visible(false);
    std::thread::sleep(Duration::from_millis(300));
    let held = visualizer.latest_frame().unwrap().id;
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        visualizer.latest_frame().unwrap().id,
        held,
        "drew while hidden"
    );
    visualizer.set_visible(true);
    let (frames, _) = collect(&visualizer, &mut feeder, 10);
    assert!(frames.last().unwrap().id > held, "did not resume");
    visualizer.stop();
}

#[test]
fn missing_scene_keeps_running_and_reports_the_error() {
    let Some((visualizer, _feeder)) = start(320, 180) else {
        return;
    };
    visualizer
        .load_preset_path(std::path::Path::new("/nonexistent/scene"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while visualizer.status().last_error.is_none() {
        assert!(Instant::now() < deadline, "no error reported");
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = visualizer.status();
    assert!(status.running);
    assert_eq!(status.preset_name, visualizer::DEFAULT_PRESET_NAME);
    visualizer.stop();
}

#[test]
fn commands_before_start_are_errors() {
    let visualizer = Visualizer::new();
    assert!(visualizer.resize(10, 10).is_err());
    assert!(visualizer.next_preset().is_err());
    assert!(visualizer.latest_frame().is_none());
}

/// A folder adds scenes to the built-in ones; an empty list of folders brings the first built-in
/// scene back.
#[test]
fn no_folders_show_the_default_scene() {
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    let dir = copied_scene("listed");
    let root = dir.parent().unwrap().to_path_buf();
    let builtin = visualizer.list_presets().len();
    assert!(builtin >= 7);
    assert_eq!(visualizer.status().preset_count, builtin);
    assert_eq!(
        visualizer
            .set_preset_dirs(&[root.clone(), root.clone()])
            .unwrap(),
        builtin + 1
    );
    assert_eq!(visualizer.status().preset_count, builtin + 1);
    let wait_for = |feeder: &mut Feeder, name: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while visualizer.status().preset_name != name && Instant::now() < deadline {
            feeder.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(visualizer.status().preset_name, name);
    };
    visualizer
        .load_preset(Location::Folder(dir.clone()))
        .unwrap();
    wait_for(&mut feeder, "listed");
    assert_eq!(visualizer.set_preset_dirs(&[]).unwrap(), builtin);
    assert_eq!(visualizer.status().preset_count, builtin);
    wait_for(&mut feeder, visualizer::DEFAULT_PRESET_NAME);
    visualizer.random_preset().unwrap();
    visualizer.stop();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saving_the_scene_on_screen_reloads_it() {
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    let dir = copied_scene("mine");
    visualizer.load_preset_path(&dir).unwrap();

    let wait = |feeder: &mut Feeder, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() && Instant::now() < deadline {
            feeder.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done(), "{:?}", visualizer.status());
    };
    wait(&mut feeder, &|| visualizer.status().preset_name == "mine");

    // A broken save shows its error; fixing it clears the error. Both go through the watcher,
    // nothing else loads the files.
    let shader = dir.join("view.wgsl");
    let good = std::fs::read_to_string(&shader).unwrap();
    std::fs::write(&shader, "this is not wgsl").unwrap();
    wait(&mut feeder, &|| visualizer.status().last_error.is_some());
    // Not the very text the scene still has: only a change makes it look again.
    std::fs::write(&shader, format!("{good}\n// fixed\n")).unwrap();
    wait(&mut feeder, &|| visualizer.status().last_error.is_none());

    visualizer.stop();
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// Every built-in scene loads and draws, on the readback path.
#[test]
fn every_builtin_scene_renders() {
    let Some((visualizer, mut feeder)) = start_with(320, 180, true) else {
        println!("SKIPPED: no GPU adapter");
        return;
    };
    let scenes: Vec<_> = visualizer
        .list_presets()
        .into_iter()
        .filter(|p| matches!(p.location, Location::Builtin(_)))
        .collect();
    assert!(scenes.len() >= 7, "{scenes:?}");
    for scene in scenes {
        visualizer.load_preset(scene.location.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while visualizer.status().preset_name != scene.name {
            assert!(
                Instant::now() < deadline,
                "{} did not load: {:?}",
                scene.name,
                visualizer.status().last_error
            );
            feeder.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        let (frames, fps) = collect(&visualizer, &mut feeder, 40);
        let status = visualizer.status();
        assert_eq!(status.last_error, None, "{}", scene.name);
        assert_eq!(status.preset_name, scene.name);
        check_frames(&frames[20..], 320, 180);
        println!("{}: {fps:.1} fps", scene.name);
    }
    visualizer.stop();
}

/// The source the runtime's `director.cadence` reads, as the last frame saw it.
fn cadence_source(visualizer: &Visualizer, feeder: &mut Feeder) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        feeder.pump();
        if let Some(inspect) = visualizer.inspect()
            && let Some(row) = inspect
                .runtime_bindings
                .iter()
                .find(|row| row.target == "director.cadence")
        {
            return row.source.clone();
        }
        assert!(Instant::now() < deadline, "no runtime bindings inspected");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_runtime_bindings_are_replaced_whole_and_a_wrong_text_keeps_the_old() {
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    visualizer.set_inspecting(true);
    assert_eq!(cadence_source(&visualizer, &mut feeder), "section_level");

    // A good text replaces them from the next frame.
    let drive = RUNTIME_TOML.replacen(
        "source = \"section_level\"\nhow = \"follow\"",
        "source = \"drive\"\nhow = \"follow\"",
        1,
    );
    assert_ne!(drive, RUNTIME_TOML);
    visualizer
        .lab(LabCommand::RuntimeBindings(drive.clone()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while cadence_source(&visualizer, &mut feeder) != "drive" {
        assert!(Instant::now() < deadline, "the bindings were not replaced");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(visualizer.status().last_error, None);

    // A wrong one is refused, naming the bind, and the old stay.
    let wrong = drive.replacen("source = \"drive\"", "source = \"loudness\"", 1);
    visualizer.lab(LabCommand::RuntimeBindings(wrong)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        feeder.pump();
        if let Some(error) = visualizer.status().last_error {
            break error;
        }
        assert!(Instant::now() < deadline, "no error reported");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        error.contains("director.cadence") && error.contains("loudness"),
        "{error}"
    );
    assert_eq!(cadence_source(&visualizer, &mut feeder), "drive");
    visualizer.stop();
}

/// Wait, feeding audio, until what the visualizer inspected is what `ready` wants.
fn inspected(
    visualizer: &Visualizer,
    feeder: &mut Feeder,
    what: &str,
    ready: impl Fn(&visualizer::Inspect) -> bool,
) -> visualizer::Inspect {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        feeder.pump();
        if let Some(inspect) = visualizer.inspect()
            && ready(&inspect)
        {
            return inspect;
        }
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A scene folder of one plain pass that declares the inputs of [`LAB_INPUTS`] and no others
/// (every built-in scene declares inputs of its own now, so the lab's cannot be added to one of
/// them). Its shader reads none of them, so they may all be taken out again.
fn input_scene(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("rune-visualizer-{name}-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = format!(
        "format = 1\nname = \"{name}\"\nlicense = \"MPL-2.0\"\n[[param]]\nid = \"unused\"\ndefault = 0.0\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n{LAB_INPUTS}"
    );
    std::fs::write(dir.join("scene.toml"), manifest).unwrap();
    let shader = "fn shade(px: Pixel) -> vec4f {\n  return vec4f(vec3f(0.1), 1.0);\n}\n";
    std::fs::write(dir.join("a.wgsl"), shader).unwrap();
    dir
}

const LAB_INPUTS: &str = r#"
[[input]]
id = "surge"
label = { de = "Schub", en = "Surge" }
range = [0.0, 2.0]
rest = 0.5

[[input]]
id = "wave"

[[bind]]
target = "surge"
source = "impacts"
how = "trigger"
when = { release = 0.5 }
fallback = "hits"
"#;

#[test]
fn a_lab_replaces_the_bindings_of_the_scene_on_screen_and_inspects_what_it_has() {
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    visualizer.set_inspecting(true);
    let dir = input_scene("lab-binds");
    visualizer.load_preset_path(&dir).unwrap();
    let seen = inspected(&visualizer, &mut feeder, "the scene did not show", |i| {
        i.scene == "lab-binds" && i.bindings.len() == 1
    });
    // What the lab reads: the inputs as declared, and each bind as written.
    assert_eq!(seen.inputs.len(), 2);
    assert_eq!(
        (seen.inputs[0].id.as_str(), seen.inputs[0].label.as_str()),
        ("surge", "Surge")
    );
    assert_eq!(seen.inputs[0].range, [0.0, 2.0]);
    assert_eq!(seen.inputs[0].rest, 0.5);
    assert!(
        seen.inputs[0]
            .declared
            .contains("label = { de = \"Schub\", en = \"Surge\" }")
    );
    assert_eq!(seen.inputs[1].label, "wave");
    assert_eq!(seen.bindings[0].when, "{ release = 0.5 }");
    assert_eq!(seen.bindings[0].fallback, "\"hits\"");

    // A new text for the binds (and the same inputs) is on screen within a frame or two.
    let edited = LAB_INPUTS.replace("release = 0.5", "release = 1.5, strength = 0.25");
    visualizer
        .lab(LabCommand::SceneBindings(edited.clone()))
        .unwrap();
    let seen = inspected(
        &visualizer,
        &mut feeder,
        "the binds were not replaced",
        |i| i.bindings.first().is_some_and(|b| b.when.contains("1.5")),
    );
    assert_eq!(seen.bindings[0].when, "{ release = 1.5, strength = 0.25 }");
    assert_eq!(visualizer.status().last_error, None);
    // A bind added for the other input shows too.
    let more =
        format!("{edited}\n[[bind]]\ntarget = \"wave\"\nsource = \"drive\"\nhow = \"follow\"\n");
    visualizer.lab(LabCommand::SceneBindings(more)).unwrap();
    inspected(
        &visualizer,
        &mut feeder,
        "the second bind did not show",
        |i| i.bindings.len() == 2,
    );
    // The watcher looks at the folder twice a second: it is as it was read, so what the lab
    // changed stays on screen.
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        feeder.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(visualizer.inspect().unwrap().bindings.len(), 2);

    // A wrong text is refused, naming the bind, and what was on screen stays.
    visualizer
        .lab(LabCommand::SceneBindings(
            edited.replace("source = \"impacts\"", "source = \"loudness\""),
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let error = loop {
        feeder.pump();
        if let Some(error) = visualizer.status().last_error {
            break error;
        }
        assert!(Instant::now() < deadline, "no error reported");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        error.contains("surge") && error.contains("loudness"),
        "{error}"
    );
    let seen = visualizer.inspect().unwrap();
    assert_eq!(seen.scene, "lab-binds");
    assert_eq!(seen.bindings.len(), 2);

    // Taking an input out rebuilds the scene, which stays the scene on screen.
    visualizer
        .lab(LabCommand::SceneBindings(String::new()))
        .unwrap();
    let seen = inspected(
        &visualizer,
        &mut feeder,
        "the inputs were not taken out",
        |i| i.inputs.is_empty() && i.bindings.is_empty(),
    );
    assert_eq!(seen.scene, "lab-binds");
    visualizer.stop();
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
