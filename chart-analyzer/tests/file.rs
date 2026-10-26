//! `analyze_file` on a file on disk: it finds the pulse of a WAV, and stops early when told to.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chart_analyzer::analyze_file;
use chart_analyzer::decode::Cancelled;
use fsio::FsIo;

const RATE: u32 = 22050;

/// Clicks with a low thump under each, four to the second: the pulse of 240 beats per minute
/// for as long as `seconds`.
fn clicks(seconds: usize) -> Vec<i16> {
    let step = RATE as usize / 4;
    let mut x = vec![0i16; seconds * RATE as usize];
    for start in (0..x.len()).step_by(step) {
        for i in 0..2000.min(x.len() - start) {
            let t = i as f64 / f64::from(RATE);
            let thump = (2.0 * std::f64::consts::PI * 60.0 * t).sin() * (-t * 30.0).exp();
            let click = if i < 40 { 0.5 } else { 0.0 };
            x[start + i] = ((thump * 0.8 + click) * 30000.0) as i16;
        }
    }
    x
}

/// A mono 16-bit WAV file of `samples` in a folder of its own.
fn write_wav(name: &str, samples: &[i16]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chart_analyzer_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clicks.wav");
    let data = samples.len() as u32 * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&[1, 0, 1, 0]);
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&[2, 0, 16, 0]);
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&path, bytes).unwrap();
    path
}

fn remove(path: &Path) {
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_wav_is_analysed() {
    let path = write_wav("analysed", &clicks(30));
    let chart = analyze_file(&FsIo::new(), &path, &AtomicBool::new(false)).unwrap();
    remove(&path);
    assert_eq!(chart.title, "clicks");
    assert!((chart.duration - 30.0).abs() < 0.1, "{}", chart.duration);
    assert!(chart.beats.len() > 20, "{}", chart.beats.len());
}

#[test]
fn a_cancelled_analysis_stops_early() {
    // Ten minutes take several seconds to analyse.
    let path = write_wav("cancelled", &clicks(600));
    let fsio = FsIo::new();
    let started = Instant::now();
    let flag = AtomicBool::new(true);
    let error = analyze_file(&fsio, &path, &flag).unwrap_err();
    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(1));

    // And while it runs.
    let flag = AtomicBool::new(false);
    let started = Instant::now();
    let error = std::thread::scope(|scope| {
        let job = scope.spawn(|| analyze_file(&fsio, &path, &flag));
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::Relaxed);
        job.join().unwrap().unwrap_err()
    });
    remove(&path);
    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(2));
}
