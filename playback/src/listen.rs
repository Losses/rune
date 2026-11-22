use std::time::Duration;

use tokio::time::Instant;

/// How much of the current track actually played. Callers rebase it when a seek moves the
/// position and reset it when another track becomes current, so only playback advances count.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListenProgress {
    heard: Duration,
    position: Duration,
    playing: bool,
    observed_at: Instant,
}

impl Default for ListenProgress {
    fn default() -> Self {
        Self {
            heard: Duration::ZERO,
            position: Duration::ZERO,
            playing: false,
            observed_at: Instant::now(),
        }
    }
}

impl ListenProgress {
    /// Starts a new listen at `position`.
    pub(crate) fn reset(&mut self, position: Duration, playing: bool, observed_at: Instant) {
        self.heard = Duration::ZERO;
        self.rebase(position, playing, observed_at);
    }

    /// Moves the baseline without counting the move, as a seek does.
    pub(crate) fn rebase(&mut self, position: Duration, playing: bool, observed_at: Instant) {
        self.position = position;
        self.playing = playing;
        self.observed_at = observed_at;
    }

    /// Records a progress report. A forward move counts when the previous report was playing,
    /// but never more than the time that actually passed. The report itself may already be
    /// paused, which keeps the last played segment.
    pub(crate) fn update(&mut self, position: Duration, playing: bool, observed_at: Instant) {
        if self.playing && position >= self.position {
            let advanced = position - self.position;
            let elapsed = observed_at.saturating_duration_since(self.observed_at);
            self.heard += advanced.min(elapsed);
        }
        self.rebase(position, playing, observed_at);
    }

    pub(crate) fn heard(&self) -> Duration {
        self.heard
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::ListenProgress;

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn playing_progress_counts() {
        let start = Instant::now();
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true, start);
        listen.update(seconds(1), true, start + seconds(1));
        listen.update(seconds(2), true, start + seconds(2));

        assert_eq!(listen.heard(), seconds(2));
    }

    #[test]
    fn a_pause_keeps_what_played_before_it_and_adds_nothing_while_paused() {
        let start = Instant::now();
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true, start);
        listen.update(seconds(10), true, start + seconds(10));
        listen.update(seconds(10), false, start + Duration::from_millis(10_100));
        listen.update(seconds(20), false, start + seconds(110));
        listen.update(seconds(20), true, start + seconds(110));
        listen.update(seconds(30), true, start + seconds(120));

        assert_eq!(listen.heard(), seconds(20));
    }

    #[test]
    fn a_seek_rebase_counts_only_playback_after_the_seek() {
        let start = Instant::now();
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true, start);
        listen.update(seconds(4), true, start + seconds(4));
        listen.rebase(seconds(178), true, start + Duration::from_millis(4_100));
        listen.update(seconds(179), true, start + Duration::from_millis(5_100));
        listen.update(seconds(180), true, start + Duration::from_millis(6_100));

        assert_eq!(listen.heard(), seconds(6));
    }

    #[test]
    fn a_reset_starts_a_new_track() {
        let start = Instant::now();
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true, start);
        listen.update(seconds(10), true, start + seconds(10));
        listen.reset(seconds(0), true, start + Duration::from_millis(10_100));
        listen.update(seconds(5), true, start + Duration::from_millis(15_100));

        assert_eq!(listen.heard(), seconds(5));
    }

    #[test]
    fn the_last_report_before_the_end_counts() {
        let start = Instant::now();
        let mut listen = ListenProgress::default();
        listen.reset(Duration::from_millis(179_900), true, start);
        listen.update(seconds(180), true, start + Duration::from_millis(100));

        assert_eq!(listen.heard(), Duration::from_millis(100));
    }

    #[test]
    fn repeated_small_forward_jumps_are_capped_at_elapsed_time() {
        let start = Instant::now();
        let mut listen = ListenProgress::default();
        listen.reset(Duration::ZERO, true, start);
        for step in 1..=100 {
            let elapsed = Duration::from_millis(100 * step);
            let position = Duration::from_millis(200 * step);
            listen.update(position, true, start + elapsed);
        }

        assert_eq!(listen.heard(), seconds(10));
    }
}
