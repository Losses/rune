use std::time::Duration;

/// How much of the current track actually played. Callers rebase it when a seek moves the
/// position and reset it when another track becomes current, so only playback advances count.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ListenProgress {
    heard: Duration,
    position: Duration,
    playing: bool,
}

impl ListenProgress {
    /// Starts a new listen at `position`.
    pub(crate) fn reset(&mut self, position: Duration, playing: bool) {
        self.heard = Duration::ZERO;
        self.rebase(position, playing);
    }

    /// Moves the baseline without counting the move, as a seek does.
    pub(crate) fn rebase(&mut self, position: Duration, playing: bool) {
        self.position = position;
        self.playing = playing;
    }

    /// Records a progress report. A forward move counts when the previous report was playing;
    /// the report itself may already be paused, which keeps the last played segment.
    pub(crate) fn update(&mut self, position: Duration, playing: bool) {
        if self.playing && position >= self.position {
            self.heard += position - self.position;
        }
        self.rebase(position, playing);
    }

    pub(crate) fn heard(&self) -> Duration {
        self.heard
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::ListenProgress;

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn playing_progress_counts() {
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true);
        listen.update(seconds(1), true);
        listen.update(seconds(2), true);

        assert_eq!(listen.heard(), seconds(2));
    }

    #[test]
    fn a_pause_keeps_what_played_before_it_and_adds_nothing_while_paused() {
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true);
        listen.update(seconds(10), true);
        listen.update(seconds(10), false);
        listen.update(seconds(20), false);
        listen.update(seconds(20), true);
        listen.update(seconds(30), true);

        assert_eq!(listen.heard(), seconds(20));
    }

    #[test]
    fn a_seek_rebase_counts_only_playback_after_the_seek() {
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true);
        listen.update(seconds(4), true);
        listen.rebase(seconds(178), true);
        listen.update(seconds(179), true);
        listen.update(seconds(180), true);

        assert_eq!(listen.heard(), seconds(6));
    }

    #[test]
    fn a_reset_starts_a_new_track() {
        let mut listen = ListenProgress::default();
        listen.reset(seconds(0), true);
        listen.update(seconds(10), true);
        listen.reset(seconds(0), true);
        listen.update(seconds(5), true);

        assert_eq!(listen.heard(), seconds(5));
    }

    #[test]
    fn the_last_report_before_the_end_counts() {
        let mut listen = ListenProgress::default();
        listen.reset(Duration::from_millis(179_900), true);
        listen.update(seconds(180), true);

        assert_eq!(listen.heard(), Duration::from_millis(100));
    }
}
