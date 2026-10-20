//! Render a preset with a synthetic beat and save frames as BMP files, for working on presets.
//!
//!   cargo run -p visualizer --example snapshot -- <out-dir> [preset.milk] [seconds,...]
//!
//! Without a preset it shows the default one (`presets/Rune.milk`). The seconds default to
//! 2, 6 and 12.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

use rtrb::RingBuffer;
use visualizer::{PcmInput, Transition, Visualizer};

const SAMPLE_RATE: u32 = 48_000;
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map(String::as_str).unwrap_or(".");
    let preset = args.get(1).filter(|a| !a.is_empty());
    let times: Vec<f64> = args
        .get(2)
        .map(|t| t.split(',').filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_else(|| vec![2.0, 6.0, 12.0]);

    let (mut producer, consumer) = RingBuffer::new(1 << 16);
    let visualizer = Visualizer::new();
    visualizer.start(
        PcmInput {
            consumer,
            channels: Arc::new(AtomicU32::new(2)),
            sample_rate: Arc::new(AtomicU32::new(SAMPLE_RATE)),
        },
        WIDTH,
        HEIGHT,
        60,
    )?;
    if let Some(path) = preset {
        visualizer.load_preset(std::fs::read(path)?, path, Transition::Cut)?;
    }

    // A kick at 120 bpm under a chord, so bass and treble both move.
    let started = Instant::now();
    let mut sent = 0u64;
    for &at in &times {
        while started.elapsed().as_secs_f64() < at {
            let due = (started.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64;
            while sent < due {
                let t = sent as f32 / SAMPLE_RATE as f32;
                let kick =
                    (-(t * 2.0).fract() * 10.0).exp() * (t * 55.0 * std::f32::consts::TAU).sin();
                let chord = [220.0, 277.2, 329.6, 1760.0]
                    .iter()
                    .map(|f| (t * f * std::f32::consts::TAU).sin())
                    .sum::<f32>()
                    * 0.08;
                let value = 0.7 * kick + chord;
                let _ = producer.push(value);
                let _ = producer.push(value);
                sent += 1;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let frame = visualizer
            .latest_frame()
            .ok_or_else(|| anyhow::anyhow!("no frame"))?;
        let path = format!("{out}/snapshot-{at:05.1}s.bmp");
        write_bmp(&path, frame.width, frame.height, &frame.to_packed()?)?;
        println!("{path}: {}", visualizer.status().preset_name);
    }
    visualizer.stop();
    Ok(())
}

/// A top-down 32-bit BMP; `pixels` are packed BGRA (macOS) or RGBA (elsewhere, swapped here).
fn write_bmp(path: &str, width: u32, height: u32, pixels: &[u8]) -> std::io::Result<()> {
    let size = pixels.len() as u32;
    let mut out = Vec::with_capacity(54 + pixels.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + size).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for pixel in pixels.chunks_exact(4) {
        if cfg!(target_os = "macos") {
            out.extend_from_slice(pixel);
        } else {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    std::fs::write(path, out)
}
