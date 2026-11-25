//! The fallback level the music is at, said in one place: how much of the music the runtime has
//! to go on (see "Fallbacks at a glance" in `docs/music.md`). Every binding reads it, so that a
//! scene and the runtime agree on what "no beat" means.

use crate::chart::ChartState;
use crate::scene::MusicValues;
use crate::switching::FOLLOWED;

/// What the runtime has of the music.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Level {
    /// A chart of the song, and its beat is followed right now.
    A,
    /// A chart, but the beat is not followed (the song has none, or it was lost).
    B,
    /// No chart: only what the live analysis hears.
    #[default]
    C,
}

impl Level {
    /// All three, best first.
    pub const ALL: [Level; 3] = [Self::A, Self::B, Self::C];

    /// The level of a frame's music: A with a chart and a followed beat (`locked` above
    /// [`FOLLOWED`]), B with a chart alone, C without one. This is the one place that says it.
    pub fn of(music: &MusicValues) -> Self {
        if music.has_chart == 0 {
            Self::C
        } else if music.locked > FOLLOWED {
            Self::A
        } else {
            Self::B
        }
    }

    /// The letter people read it by.
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
        }
    }

    /// 0 for A, 1 for B, 2 for C: the place in a table of the three.
    pub fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
        }
    }

    /// The level a lab cycles to from `forced`: automatic, then A, B and C, then automatic again.
    pub fn cycle(forced: Option<Self>) -> Option<Self> {
        match forced {
            None => Some(Self::A),
            Some(Self::A) => Some(Self::B),
            Some(Self::B) => Some(Self::C),
            Some(Self::C) => None,
        }
    }
}

/// The chart a lab that forces `level` hands the music: a forced level can only take the music
/// down from what the song has, since it cannot make a beat or a chart up. A (or none) leaves the
/// chart as it is; B keeps it but without its beat (so the rhythm is zeroed as
/// [`crate::chart::trusted_rhythm`] does for a song with no pulse); C has no chart at all.
pub fn forced_chart(level: Option<Level>, chart: Option<ChartState>) -> Option<ChartState> {
    match level {
        None | Some(Level::A) => chart,
        Some(Level::B) => chart.map(|chart| ChartState {
            bpm: 0.0,
            beat_confidence: 0.0,
            ..chart
        }),
        Some(Level::C) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::trusted_rhythm;
    use crate::rhythm::Rhythm;

    #[test]
    fn the_level_follows_the_chart_and_the_beat() {
        let music = |has_chart, locked| MusicValues {
            has_chart,
            locked,
            ..MusicValues::default()
        };
        assert_eq!(Level::of(&music(1, 1.0)), Level::A);
        assert_eq!(Level::of(&music(1, 0.5)), Level::B);
        assert_eq!(Level::of(&music(1, 0.0)), Level::B);
        // Without a chart the beat does not matter: the runtime trusts none.
        assert_eq!(Level::of(&music(0, 1.0)), Level::C);
        assert_eq!(Level::of(&MusicValues::default()), Level::C);
    }

    #[test]
    fn a_forced_level_only_takes_the_music_down() {
        let live = Rhythm {
            bpm: 120.0,
            beat: 8.5,
            confidence: 0.9,
            kick: 0.4,
            ..Default::default()
        };
        let chart = ChartState {
            bpm: 100.0,
            beat: 5.5,
            beat_confidence: 0.7,
            section_energy: 0.6,
            ..Default::default()
        };
        let rhythm = |level| {
            let chart = forced_chart(level, Some(chart.clone()));
            (trusted_rhythm(live, chart.as_ref()), chart)
        };
        let (r, c) = rhythm(None);
        assert_eq!((r.bpm, c), (100.0, Some(chart.clone())));
        assert_eq!(rhythm(Some(Level::A)).0.bpm, 100.0);
        // B keeps the chart (its sections and levels) and loses the beat, as a chart without
        // pulse does; the live onsets stay.
        let (r, c) = rhythm(Some(Level::B));
        assert_eq!((r.bpm, r.beat, r.confidence, r.kick), (0.0, 0.0, 0.0, 0.4));
        assert_eq!(c.map(|c| c.section_energy), Some(0.6));
        // C has no chart, and so no beat either.
        let (r, c) = rhythm(Some(Level::C));
        assert_eq!((r.bpm, r.kick, c), (0.0, 0.4, None));
        // Nothing is made up: with no chart, B and A stay without one.
        assert_eq!(forced_chart(Some(Level::B), None), None);
    }

    #[test]
    fn a_lab_cycles_auto_then_a_b_c() {
        let mut forced = None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            forced = Level::cycle(forced);
            seen.push(forced);
        }
        assert_eq!(seen, [Some(Level::A), Some(Level::B), Some(Level::C), None]);
    }
}
