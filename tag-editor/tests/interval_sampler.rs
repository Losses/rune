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
