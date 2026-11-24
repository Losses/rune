//! `analyze_file` on a file on disk: it finds the pulse of a WAV, and stops early when told to.

use std::f32::consts::TAU;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chart_analyzer::decode::{Cancelled, decode};
use chart_analyzer::{Chart, analyze_file};
use fsio::FsIo;
use std::env;
use std::fs;
use std::process as std_process;
use std::thread;

const RATE: u32 = 22050;

/// Clicks with a low thump under each, four to the second: the pulse of 240 beats per minute
/// for as long as `seconds`.
fn clicks(seconds: usize) -> Vec<i16> {
    let step = RATE as usize / 4;
    let mut x = vec![0i16; seconds * RATE as usize];
    for start in (0..x.len()).step_by(step) {
        for i in 0..2000.min(x.len() - start) {
            let t = i as f64 / f64::from(RATE);
            let thump = (2.0 * PI * 60.0 * t).sin() * (-t * 30.0).exp();
            let click = if i < 40 { 0.5 } else { 0.0 };
            x[start + i] = ((thump * 0.8 + click) * 30000.0) as i16;
        }
    }
    x
}

/// A mono WAV file of already encoded samples in a folder of its own.
fn write_wav<const N: usize>(
    name: &str,
    format: u16,
    samples: impl ExactSizeIterator<Item = [u8; N]>,
) -> PathBuf {
    let dir = env::temp_dir().join(format!("chart_analyzer_{name}_{}", std_process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clicks.wav");
    let len = samples.len() as u32;
    let sample_bytes = N as u16;
    let data = len * u32::from(sample_bytes);
    let float = format == 3;
    let fmt_len: u32 = if float { 18 } else { 16 };
    let header = 20 + fmt_len + if float { 12 } else { 0 };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(header + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&fmt_len.to_le_bytes());
    bytes.extend_from_slice(&format.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * u32::from(sample_bytes)).to_le_bytes());
    bytes.extend_from_slice(&sample_bytes.to_le_bytes());
    bytes.extend_from_slice(&(sample_bytes * 8).to_le_bytes());
    if float {
        bytes.extend_from_slice(&0u16.to_le_bytes()); // cbSize
        bytes.extend_from_slice(b"fact");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&len.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s);
    }
    fs::write(&path, bytes).unwrap();
    path
}

fn remove(path: &Path) {
    fs::remove_dir_all(path.parent().unwrap()).ok();
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../playback/tests/fixtures/chained_ogg")
        .join(name)
}

/// Writes `parts` one after another into a temporary Ogg file, as a chained physical stream.
fn chained_ogg(name: &str, parts: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chart_analyzer_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chained.ogg");
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(&std::fs::read(fixture(part)).unwrap());
    }
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The frequency of `samples` at `rate`, from rising zero crossings.
fn frequency(samples: &[f32], rate: u32) -> f64 {
    let crossings = samples
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    crossings as f64 * f64::from(rate) / samples.len() as f64
}

#[test]
fn a_wav_is_analysed() {
    let path = write_wav("analysed", 1, clicks(30).into_iter().map(i16::to_le_bytes));
    let chart = analyze_file(&FsIo::new(), &path, &AtomicBool::new(false)).unwrap();
    remove(&path);
    assert_eq!(chart.title, "clicks");
    assert!((chart.duration - 30.0).abs() < 0.1, "{}", chart.duration);
    assert!(chart.beats.len() > 20, "{}", chart.beats.len());
}

#[test]
fn a_cancelled_analysis_stops_early() {
    // Ten minutes take several seconds to analyse.
    let path = write_wav(
        "cancelled",
        1,
        clicks(600).into_iter().map(i16::to_le_bytes),
    );
    let fsio = FsIo::new();
    let started = Instant::now();
    let flag = AtomicBool::new(true);
    let error = analyze_file(&fsio, &path, &flag).unwrap_err();
    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(1));

    // And while it runs.
    let flag = AtomicBool::new(false);
    let started = Instant::now();
    let error = thread::scope(|scope| {
        let job = scope.spawn(|| analyze_file(&fsio, &path, &flag));
        thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::Relaxed);
        job.join().unwrap().unwrap_err()
    });
    remove(&path);
    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn float_wav_analysis_returns_a_finite_chart_or_an_error() {
    let fsio = FsIo::new();
    let cancel = AtomicBool::new(false);
    let mut invalid = Vec::new();
    for (name, peak) in [("normal", 1.125), ("nan", f32::NAN), ("max", f32::MAX)] {
        let samples = (0..RATE).map(|i| {
            let sample = if i == RATE / 2 {
                peak
            } else {
                0.25 * (TAU * 440.0 * i as f32 / RATE as f32).sin()
            };
            sample.to_le_bytes()
        });
        let path = write_wav(name, 3, samples); // IEEE float
        let result = analyze_file(&fsio, &path, &cancel);
        remove(&path);
        match result {
            Ok(chart) => {
                if name == "normal" {
                    assert!(!chart.curves.loudness.is_empty());
                }
                // JSON encodes non-finite floats as null, which cannot be read
                // back into the chart's numeric fields.
                let json = serde_json::to_value(chart).unwrap();
                if let Err(err) = serde_json::from_value::<Chart>(json) {
                    invalid.push(format!("{name}: {err}"));
                }
            }
            Err(err) if name == "normal" => panic!("{name}: {err:#}"),
            Err(_) => {}
        }
    }
    assert!(invalid.is_empty(), "{}", invalid.join("\n"));
}

#[test]
fn a_chained_ogg_is_analysed_through_the_second_stream() {
    let fsio = FsIo::new();
    let cancel = AtomicBool::new(false);
    let single = analyze_file(&fsio, &fixture("tone_440_44100.ogg"), &cancel).unwrap();
    let path = chained_ogg(
        "chained_ogg_two_streams",
        &["tone_440_44100.ogg", "tone_660_44100.ogg"],
    );
    let chained = analyze_file(&fsio, &path, &cancel).unwrap();
    let samples = decode(&fsio, &path).unwrap();
    remove(&path);

    assert!(
        chained.duration > single.duration * 1.5,
        "the chained chart lasts {} s, the single stream {} s",
        chained.duration,
        single.duration
    );

    // The second stream is a 660 Hz tone, not a repeat of the first stream's 440 Hz.
    let first = frequency(&samples[..samples.len() / 4], RATE);
    let second = frequency(&samples[samples.len() * 3 / 4..], RATE);
    assert!((first - 440.0).abs() < 10.0, "first stream is {first} Hz");
    assert!(
        (second - 660.0).abs() < 10.0,
        "second stream is {second} Hz"
    );
}

#[test]
fn a_single_stream_ogg_is_analysed_to_its_end() {
    let fsio = FsIo::new();
    let cancel = AtomicBool::new(false);
    let path = fixture("tone_440_44100.ogg");
    let chart = analyze_file(&fsio, &path, &cancel).unwrap();
    let samples = decode(&fsio, &path).unwrap();

    assert!((chart.duration - 2.0).abs() < 0.1, "{}", chart.duration);
    assert!(
        (samples.len() as f64 - 44_100.0).abs() < 1_000.0,
        "decoded {} samples",
        samples.len()
    );
    let tone = frequency(&samples[..samples.len() / 2], RATE);
    assert!((tone - 440.0).abs() < 10.0, "stream is {tone} Hz");
}

#[test]
fn an_unsupported_stream_change_is_an_error_not_a_partial_chart() {
    let fsio = FsIo::new();
    let cancel = AtomicBool::new(false);
    let path = chained_ogg(
        "chained_ogg_rate_change",
        &["tone_440_44100.ogg", "tone_660_48000.ogg"],
    );
    let result = analyze_file(&fsio, &path, &cancel);
    remove(&path);

    let error = result.expect_err("a sample-rate change must not produce a first-stream chart");
    assert!(
        error.to_string().contains("sample rate"),
        "unexpected error: {error:#}"
    );
}
