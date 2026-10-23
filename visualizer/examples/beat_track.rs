//! Runs the beat tracker over interleaved `f32` samples from stdin, as a player would feed it,
//! and prints what it found: `tick <seconds>` for every beat of its clock and, twice a second,
//! `state <seconds> <bpm> <confidence> <kick> <snare> <hat> <energy>`.
//!
//! ```sh
//! ffmpeg -loglevel error -i song.flac -f f32le -ac 2 -ar 44100 - |
//!     cargo run -q -p visualizer --example beat_track -- 44100 2
//! ```

use std::io::Read;

use visualizer::BeatTracker;

fn main() {
    let mut args = std::env::args().skip(1);
    let rate: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(44_100);
    let channels: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(2);
    let mut bytes = Vec::new();
    std::io::stdin()
        .read_to_end(&mut bytes)
        .expect("read stdin");
    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();

    let mut tracker = BeatTracker::new(rate);
    // A frame of video at 60 fps.
    let chunk = (rate as usize / 60) * channels;
    let mut frames = 0usize;
    let mut last_beat = 0.0f64;
    let mut next_state = 0.5;
    for part in samples.chunks(chunk) {
        tracker.push(part, channels);
        frames += part.len() / channels;
        let t = frames as f64 / f64::from(rate);
        let r = tracker.rhythm();
        if r.beat.floor() > last_beat.floor() {
            let cross = t
                - (r.beat - r.beat.floor()) / (r.beat - last_beat) * (part.len() / channels) as f64
                    / f64::from(rate);
            println!("tick {cross:.4}");
        }
        last_beat = r.beat;
        if t >= next_state {
            next_state += 0.5;
            println!(
                "state {t:.2} {:.2} {:.3} {:.2} {:.2} {:.2} {:.2}",
                r.bpm, r.confidence, r.kick, r.snare, r.hat, r.energy
            );
        }
    }
}
