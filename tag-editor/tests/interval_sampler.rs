use std::sync::mpsc::TryRecvError;

use anyhow::Result;
use fsio::FsIo;
use tag_editor::sampler::interval_sampler::IntervalSampler;
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
