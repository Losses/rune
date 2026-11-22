use std::{env, fs, sync::mpsc::TryRecvError};

use anyhow::Result;
use fsio::FsIo;
use std::iter;
use tag_editor::sampler::interval_sampler::{IntervalSampler, SampleEvent};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const AUDIO_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");

fn assert_channel_closed(sampler: &IntervalSampler) {
    assert_eq!(
        sampler.receiver.try_recv().err(),
        Some(TryRecvError::Disconnected)
    );
}

#[test]
fn completed_sampler_closes_after_the_last_event() -> Result<()> {
    let fsio = FsIo::new();
    let mut sampler = IntervalSampler::new(AUDIO_FIXTURE, 4.0, 4.0, 16_000, None);
    sampler.process(&fsio)?;

    let windows: Vec<_> = sampler
        .receiver
        .try_iter()
        .map(|event| (event.start_time, event.end_time))
        .collect();
    assert_eq!(windows, [(0.0, 4.0), (4.0, 8.0)]);
    assert_channel_closed(&sampler);
    assert!(sampler.process(&fsio).is_err());
    Ok(())
}

#[test]
fn completed_sampler_with_no_events_closes() -> Result<()> {
    let mut sampler = IntervalSampler::new(AUDIO_FIXTURE, 12.0, 12.0, 16_000, None);
    sampler.process(&FsIo::new())?;

    assert_channel_closed(&sampler);
    Ok(())
}

#[test]
fn cancelled_sampler_closes() -> Result<()> {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut sampler = IntervalSampler::new(AUDIO_FIXTURE, 4.0, 4.0, 16_000, Some(cancellation));
    sampler.process(&FsIo::new())?;

    assert_channel_closed(&sampler);
    Ok(())
}

#[test]
fn sampler_closes_on_processing_error() {
    let not_audio = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let mut sampler = IntervalSampler::new(not_audio, 4.0, 4.0, 16_000, None);

    assert!(sampler.process(&FsIo::new()).is_err());
    assert_channel_closed(&sampler);
}

fn assert_completes_with_samples(path: &str) -> Result<()> {
    let mut sampler = IntervalSampler::new(path, 0.1, 0.1, 16_000, None);
    sampler.process(&FsIo::new())?;

    assert!(sampler.receiver.try_iter().count() > 0);
    assert_channel_closed(&sampler);
    Ok(())
}

#[test]
fn opus_with_pre_skip_completes_at_eof() -> Result<()> {
    // Generated 0.2 seconds of a 440 Hz tone with FFmpeg's libopus encoder.
    assert_completes_with_samples(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/interval-pre-skip.opus"
    ))
}

#[test]
fn vbr_mp3_without_a_frame_count_completes_at_eof() -> Result<()> {
    // Generated half a second of silence followed by a tone with
    // libmp3lame at -q:a 2 and -write_xing 0.
    assert_completes_with_samples(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/interval-vbr-no-xing.mp3"
    ))
}

fn with_wav(rate: u32, samples: &[i16], check: impl FnOnce(&str) -> Result<()>) -> Result<()> {
    let data_len = samples.len() as u32 * 2;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data_len).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16_u32.to_le_bytes());
    wav.extend(1_u16.to_le_bytes()); // PCM
    wav.extend(1_u16.to_le_bytes()); // Mono
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 2).to_le_bytes());
    wav.extend(2_u16.to_le_bytes()); // Bytes per frame
    wav.extend(16_u16.to_le_bytes()); // Bits per sample
    wav.extend(b"data");
    wav.extend(data_len.to_le_bytes());
    for sample in samples {
        wav.extend(sample.to_le_bytes());
    }

    let path = env::temp_dir().join(format!("rune-interval-{}.wav", Uuid::new_v4()));
    fs::write(&path, wav)?;
    let result = check(path.to_str().unwrap());
    fs::remove_file(path)?;
    result
}

fn sample(path: &str, duration: f64, interval: f64, rate: u32) -> Result<Vec<SampleEvent>> {
    let mut sampler = IntervalSampler::new(path, duration, interval, rate, None);
    sampler.process(&FsIo::new())?;
    Ok(sampler.receiver.try_iter().collect())
}

#[test]
fn resampled_audio_matches_both_ends_of_its_window() -> Result<()> {
    let rate = 8192_u32;
    let target_rate = 6000_u32;
    let input: Vec<_> = [4096, 8192, 12288]
        .into_iter()
        .flat_map(|level| iter::repeat_n(level, rate as usize))
        .collect();

    with_wav(rate, &input, |path| {
        for (interval, expected) in [
            (1.0, &[(0.0, 0.125), (1.0, 0.25), (2.0, 0.375)][..]),
            (2.0, &[(0.0, 0.125), (2.0, 0.375)][..]),
        ] {
            let events = sample(path, 1.0, interval, target_rate)?;
            assert_eq!(events.len(), expected.len());
            for (event, &(start, level)) in events.iter().zip(expected) {
                let len = target_rate as usize;
                let margin = len / 16;
                assert_eq!((event.start_time, event.end_time), (start, start + 1.0));
                assert_eq!(event.data.len(), len);
                for edge in [
                    &event.data[margin..2 * margin],
                    &event.data[len - 2 * margin..len - margin],
                ] {
                    let mean = edge.iter().sum::<f64>() / edge.len() as f64;
                    assert!(
                        (mean - level).abs() < 0.001,
                        "window starting at {start}s has level {mean}, expected {level}"
                    );
                }
            }
        }
        Ok(())
    })
}

#[test]
fn overlapping_and_fractional_windows_match_source_frames() -> Result<()> {
    let rate = 8192_u32;
    let frames: Vec<_> = (1..=16).map(|value| value * 1024).collect();
    let seconds = |source_frames| source_frames / f64::from(rate);
    with_wav(rate, &frames, |path| {
        for (duration, interval, ranges) in [
            (5.0, 3.0, &[0..5, 3..8, 6..11, 9..14][..]),
            (3.25, 3.25, &[0..4, 4..7, 7..10, 10..13][..]),
            (3.25, 5.25, &[0..4, 6..9, 11..14][..]),
        ] {
            let events = sample(path, seconds(duration), seconds(interval), rate)?;
            assert_eq!(events.len(), ranges.len(), "interval {interval}");
            for (event, range) in events.iter().zip(ranges) {
                let samples: Vec<_> = frames[range.clone()]
                    .iter()
                    .map(|&value| f64::from(value) / 32768.0)
                    .collect();
                assert_eq!(event.data, samples, "source frames {range:?}");
            }
        }
        Ok(())
    })
}
