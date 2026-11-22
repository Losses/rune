use std::time::Duration;

use playback::player::{PlaybackState, PlayingItem};
use tokio::time::Instant;

use super::ListenProgress;

fn item(name: &str) -> PlayingItem {
    PlayingItem::IndependentFile(name.to_string())
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}

fn assert_about(actual: Duration, expected: Duration) {
    let difference = actual.abs_diff(expected);
    assert!(
        difference <= Duration::from_millis(100),
        "expected about {expected:?}, got {actual:?}"
    );
}

#[test]
fn seek_to_the_end_counts_only_the_audio_that_played() {
    let start = Instant::now();
    let track = item("180-second-track");
    let mut listen = ListenProgress::default();

    listen.observe(Some(&track), PlaybackState::Playing, Duration::ZERO, start);
    for second in 1..=4 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(f64::from(second)),
        );
    }
    listen.observe(
        Some(&track),
        PlaybackState::Playing,
        seconds(178.0),
        start + seconds(4.1),
    );
    listen.observe(
        Some(&track),
        PlaybackState::Playing,
        seconds(179.0),
        start + seconds(5.1),
    );
    listen.observe(
        Some(&track),
        PlaybackState::Playing,
        seconds(180.0),
        start + seconds(6.1),
    );

    assert_about(listen.finish(&track), seconds(6.0));
}

#[test]
fn a_completed_track_counts_all_of_its_played_time() {
    let start = Instant::now();
    let track = item("180-second-track");
    let mut listen = ListenProgress::default();

    for second in 0..=180 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(f64::from(second)),
        );
    }

    assert_about(listen.finish(&track), seconds(180.0));
}

#[test]
fn a_short_track_counts_all_of_its_played_time() {
    let start = Instant::now();
    let track = item("20-second-track");
    let mut listen = ListenProgress::default();

    for second in 0..=20 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(f64::from(second)),
        );
    }

    assert_about(listen.finish(&track), seconds(20.0));
}

#[test]
fn a_pause_does_not_count_as_listening() {
    let start = Instant::now();
    let track = item("20-second-track");
    let mut listen = ListenProgress::default();

    for second in 0..=10 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(f64::from(second)),
        );
    }
    listen.observe(
        Some(&track),
        PlaybackState::Paused,
        seconds(10.0),
        start + seconds(10.1),
    );
    listen.observe(
        Some(&track),
        PlaybackState::Paused,
        seconds(10.0),
        start + seconds(110.0),
    );
    listen.observe(
        Some(&track),
        PlaybackState::Playing,
        seconds(10.0),
        start + seconds(110.0),
    );
    for second in 11..=20 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(100.0 + f64::from(second)),
        );
    }

    assert_about(listen.finish(&track), seconds(20.0));
}

#[test]
fn seeking_backward_does_not_remove_or_add_listened_time() {
    let start = Instant::now();
    let track = item("20-second-track");
    let mut listen = ListenProgress::default();

    for second in 0..=10 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(f64::from(second)),
        );
    }
    listen.observe(
        Some(&track),
        PlaybackState::Playing,
        Duration::ZERO,
        start + seconds(10.1),
    );
    for second in 1..=5 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(10.0 + f64::from(second)),
        );
    }

    assert_about(listen.finish(&track), seconds(15.0));
}

#[test]
fn a_new_item_starts_a_new_listen() {
    let start = Instant::now();
    let first = item("first");
    let second = item("second");
    let mut listen = ListenProgress::default();

    for index in 0..=10 {
        listen.observe(
            Some(&first),
            PlaybackState::Playing,
            seconds(f64::from(index)),
            start + seconds(f64::from(index)),
        );
    }
    listen.observe(
        Some(&second),
        PlaybackState::Playing,
        Duration::ZERO,
        start + seconds(10.1),
    );

    assert_eq!(listen.finish(&first), Duration::ZERO);
    for index in 1..=5 {
        listen.observe(
            Some(&second),
            PlaybackState::Playing,
            seconds(f64::from(index)),
            start + seconds(10.0 + f64::from(index)),
        );
    }

    assert_about(listen.finish(&second), seconds(5.0));
}

#[test]
fn a_repeated_item_starts_a_new_listen_after_completion() {
    let start = Instant::now();
    let track = item("repeated");
    let mut listen = ListenProgress::default();

    for second in 0..=10 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(f64::from(second)),
        );
    }
    assert_about(listen.finish(&track), seconds(10.0));

    for second in 0..=5 {
        listen.observe(
            Some(&track),
            PlaybackState::Playing,
            seconds(f64::from(second)),
            start + seconds(10.0 + f64::from(second)),
        );
    }
    assert_about(listen.finish(&track), seconds(5.0));
}
