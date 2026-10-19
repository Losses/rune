//! Headless GPU tests. They skip (and say so) when the machine has no GPU adapter.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

use rtrb::{Producer, RingBuffer};
use visualizer::{Frame, PcmInput, Source, Transition, Visualizer};

const SAMPLE_RATE: u32 = 48_000;
const NEBULA_FORGE: &[u8] = include_bytes!("fixtures/nebula_forge.milk");

/// Feeds a stereo test signal (a decaying 110 Hz tone burst four times per second plus a little
/// noise) at real time speed.
struct Feeder {
    producer: Producer<f32>,
    samples_sent: u64,
    started: Instant,
    noise: u32,
}

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
    let (producer, consumer) = RingBuffer::new(1 << 16);
    let input = PcmInput {
        consumer,
        channels: Arc::new(AtomicU32::new(2)),
        sample_rate: Arc::new(AtomicU32::new(SAMPLE_RATE)),
    };
    let visualizer = Visualizer::new();
    visualizer.start(input, width, height, 60).unwrap();
    let feeder = Feeder {
        producer,
        samples_sent: 0,
        started: Instant::now(),
        noise: 1,
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
fn collect(visualizer: &Visualizer, feeder: &mut Feeder, count: usize) -> (Vec<Arc<Frame>>, f64) {
    let mut frames: Vec<Arc<Frame>> = Vec::new();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(60);
    while frames.len() < count {
        assert!(Instant::now() < deadline, "frames stopped coming");
        feeder.pump();
        if let Some(frame) = visualizer.latest_frame()
            && frames.last().is_none_or(|last| last.id < frame.id)
        {
            frames.push(frame);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let fps = (frames.last().unwrap().id - frames[0].id) as f64 / started.elapsed().as_secs_f64();
    (frames, fps)
}

fn is_black(frame: &Frame) -> bool {
    frame.bgra.chunks_exact(4).all(|p| p[..3] == [0, 0, 0])
}

fn check_frames(frames: &[Arc<Frame>], width: u32, height: u32) {
    for pair in frames.windows(2) {
        assert!(pair[0].id < pair[1].id);
    }
    for frame in frames {
        assert_eq!((frame.width, frame.height), (width, height));
        assert_eq!(frame.bgra.len(), (width * height * 4) as usize);
    }
    let last = frames.last().unwrap();
    assert!(!is_black(last), "the last frame is black");
    assert!(
        frames.iter().any(|f| f.bgra != last.bgra),
        "the picture never changes"
    );
}

#[test]
fn idle_and_real_preset_render_changing_frames() {
    let Some((visualizer, mut feeder)) = start(640, 360) else {
        return;
    };
    let status = visualizer.status();
    assert!(status.running);
    assert_eq!(status.preset_name, lactea::IDLE_PRESET_NAME);

    let (frames, fps) = collect(&visualizer, &mut feeder, 60);
    check_frames(&frames, 640, 360);
    println!("idle preset 640x360: {fps:.1} fps on {}", status.adapter);

    visualizer
        .load_preset(NEBULA_FORGE.to_vec(), "nebula_forge", Transition::Cut)
        .unwrap();
    let (frames, fps) = collect(&visualizer, &mut feeder, 60);
    check_frames(&frames, 640, 360);
    let status = visualizer.status();
    assert_eq!(
        status.preset_name, "nebula_forge",
        "{:?}",
        status.last_error
    );
    println!("nebula_forge 640x360: {fps:.1} fps");

    visualizer.resize(1280, 720).unwrap();
    // Frames of the old size may still be in flight.
    let (frames, fps) = collect(&visualizer, &mut feeder, 90);
    check_frames(&frames[30..], 1280, 720);
    println!("nebula_forge 1280x720: {fps:.1} fps");

    let input = visualizer.stop();
    assert!(input.is_some());
    assert!(visualizer.latest_frame().is_none());
    assert!(!visualizer.status().running);
}

#[test]
fn bad_preset_keeps_running_and_reports_the_error() {
    let Some((visualizer, _feeder)) = start(320, 180) else {
        return;
    };
    visualizer
        .load_preset(b"\0binary".to_vec(), "junk", Transition::Cut)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while visualizer.status().last_error.is_none() {
        assert!(Instant::now() < deadline, "no error reported");
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = visualizer.status();
    assert!(status.running);
    assert_ne!(status.preset_name, "junk");
    visualizer.stop();
}

#[test]
fn commands_before_start_are_errors() {
    let visualizer = Visualizer::new();
    assert!(visualizer.resize(10, 10).is_err());
    assert!(visualizer.next_preset(Transition::Cut).is_err());
    assert!(visualizer.latest_frame().is_none());
}

/// Loads presets from the real built-in pack. Skips unless the build embedded one
/// (`RUNE_BUILTIN_PRESETS` at build time).
#[test]
fn builtin_pack_presets_render() {
    if visualizer::builtin_pack().is_none() {
        eprintln!("SKIPPED: RUNE_BUILTIN_PRESETS was not set at build time");
        return;
    }
    let Some((visualizer, mut feeder)) = start(320, 180) else {
        return;
    };
    // Nothing chosen: the built-in pack is the default source.
    let status = visualizer.status();
    assert!(status.builtin_available);
    assert_eq!(status.source, Source::Builtin);
    assert!(status.preset_count > 100, "{}", status.preset_count);

    for _ in 0..12 {
        let before = visualizer.status().preset_name;
        visualizer.random_preset(Transition::Cut).unwrap();
        // A load either changes the name or reports an error; the random pick may also be the
        // preset that is already loaded, so do not wait forever.
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            feeder.pump();
            let status = visualizer.status();
            if status.last_error.is_some()
                || status.preset_name != before
                || Instant::now() > deadline
            {
                break status;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(status.last_error, None, "{}", status.preset_name);
    }
    let (frames, _) = collect(&visualizer, &mut feeder, 30);
    assert!(!is_black(frames.last().unwrap()));
    visualizer.stop();
}
