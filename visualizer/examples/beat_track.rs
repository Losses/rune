//! Runs the beat tracker over interleaved `f32` samples from stdin, as a player would feed it,
//! and prints what it found: `tick <seconds>` for every beat of its clock and, twice a second,
//! `state <seconds> <bpm> <confidence> <kick> <snare> <hat> <energy>`.
//!
//! ```sh
//! ffmpeg -loglevel error -i song.flac -f f32le -ac 2 -ar 44100 - |
//!     cargo run -q -p visualizer --example beat_track -- 44100 2
//! ```
//!
//! The sample rate and channel count default to 44100 Hz and 2. The samples are fed a frame of
//! video at a time, so a rate below 60 Hz or no channels is refused before anything is read.

use std::io::{Read, Write};

use anyhow::{Context, Result};
use std::env;
use std::io;
use visualizer::BeatTracker;

/// Frames of video a second; the tracker is fed the samples of one frame at a time.
const FRAMES_PER_SECOND: u32 = 60;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    run(&args, io::stdin().lock(), io::stdout().lock())
}

fn run(args: &[String], mut input: impl Read, mut output: impl Write) -> Result<()> {
    let rate = match args.first() {
        None => 44_100,
        Some(arg) => arg
            .parse::<u32>()
            .ok()
            .filter(|rate| *rate >= FRAMES_PER_SECOND)
            .with_context(|| {
                format!("The sample rate must be a whole number of at least {FRAMES_PER_SECOND} Hz, not {arg:?}")
            })?,
    };
    let channels = match args.get(1) {
        None => 2,
        Some(arg) => arg
            .parse::<usize>()
            .ok()
            .filter(|channels| *channels >= 1)
            .with_context(|| {
                format!("The channel count must be a whole number of at least 1, not {arg:?}")
            })?,
    };
    // A frame of video, which holds at least one sample of every channel.
    let chunk = ((rate / FRAMES_PER_SECOND) as usize)
        .checked_mul(channels)
        .with_context(|| format!("{channels} channels are too many for a frame of video"))?;
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .context("Failed to read stdin")?;
    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();

    let mut tracker = BeatTracker::new(rate);
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
            writeln!(output, "tick {cross:.4}")?;
        }
        last_beat = r.beat;
        if t >= next_state {
            next_state += 0.5;
            writeln!(
                output,
                "state {t:.2} {:.2} {:.3} {:.2} {:.2} {:.2} {:.2}",
                r.bpm, r.confidence, r.kick, r.snare, r.hat, r.energy
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::run;

    /// What the example prints for `args` and `samples` on stdin.
    fn beat_track(args: &[&str], samples: &[f32]) -> Result<String> {
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut output = Vec::new();
        run(&args, bytes.as_slice(), &mut output)?;
        Ok(String::from_utf8(output)?)
    }

    #[test]
    fn invalid_rates_and_channel_counts_are_refused() {
        let too_many = usize::MAX.to_string();
        for (args, refused) in [
            (["0", "2"], "sample rate"),
            (["59", "2"], "sample rate"),
            (["fast", "2"], "sample rate"),
            (["44100", "0"], "channel count"),
            (["44100", too_many.as_str()], "channels are too many"),
        ] {
            let error = beat_track(&args, &[]).expect_err("an empty frame of video");
            assert!(error.to_string().contains(refused), "{args:?}: {error}");
        }
    }

    #[test]
    fn the_lowest_rate_and_a_single_channel_are_accepted() {
        // Two seconds of one sample a frame.
        let output = beat_track(&["60", "1"], &[0.0; 120]).unwrap();
        let states = output.lines().filter(|line| line.starts_with("state "));
        assert_eq!(states.count(), 4, "{output}");
    }

    #[test]
    fn stereo_at_44100_hz_reports_its_state_twice_a_second() {
        const RATE: usize = 44_100;
        // Two seconds of a click every half second, the same on both channels.
        let samples: Vec<f32> = (0..2 * RATE)
            .flat_map(|i| {
                let click = if i % (RATE / 2) < 200 { 0.8 } else { 0.0 };
                [click, click]
            })
            .collect();
        let output = beat_track(&["44100", "2"], &samples).unwrap();
        let states: Vec<&str> = output
            .lines()
            .filter(|line| line.starts_with("state "))
            .collect();
        assert_eq!(states.len(), 4, "{output}");
        assert!(
            states.iter().all(|line| line.split(' ').count() == 8),
            "{output}"
        );
    }
}
