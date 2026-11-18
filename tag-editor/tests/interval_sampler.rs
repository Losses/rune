use std::{env, fs, sync::mpsc::TryRecvError};

use anyhow::Result;
use fsio::FsIo;
use rubato::{FftFixedInOut, Resampler};
use tag_editor::sampler::interval_sampler::{IntervalSampler, SampleEvent};
use tokio_util::sync::CancellationToken;

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

#[test]
fn sample_audio_matches_its_window_with_and_without_a_gap() -> Result<()> {
    // A power-of-two rate makes the one-second boundaries exact.
    let rate = 8192_u32;
    let data_len = 3 * rate * 2;
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
    for level in [4096_i16, 8192, 12288] {
        for _ in 0..rate {
            wav.extend(level.to_le_bytes());
        }
    }

    let path = env::temp_dir().join(format!("rune-interval-{}.wav", uuid::Uuid::new_v4()));
    fs::write(&path, wav)?;
    let collect = |interval| -> Result<Vec<SampleEvent>> {
        let mut sampler = IntervalSampler::new(path.to_str().unwrap(), 1.0, interval, rate, None);
        sampler.process(&FsIo::new())?;
        Ok(sampler.receiver.try_iter().collect())
    };
    let contiguous = collect(1.0);
    let gapped = collect(2.0);
    fs::remove_file(path)?;

    let rate = rate as usize;
    let delay = FftFixedInOut::<f64>::new(rate, rate, rate, 1)?.output_delay();
    let margin = rate / 8;
    for (events, expected) in [
        (contiguous?, &[(0.0, 0.125), (1.0, 0.25), (2.0, 0.375)][..]),
        (gapped?, &[(0.0, 0.125), (2.0, 0.375)][..]),
    ] {
        assert_eq!(events.len(), expected.len());
        for (event, &(start, level)) in events.iter().zip(expected) {
            assert_eq!((event.start_time, event.end_time), (start, start + 1.0));
            assert_eq!(event.data.len(), rate);
            // Measure the steady portion after the streaming resampler's delay.
            let steady = &event.data[delay + margin..rate - margin];
            let mean = steady.iter().sum::<f64>() / steady.len() as f64;
            assert!(
                (mean - level).abs() < 0.001,
                "window starting at {start}s has level {mean}, expected {level}"
            );
        }
    }
    Ok(())
}
