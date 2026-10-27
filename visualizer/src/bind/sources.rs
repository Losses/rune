//! The registry of what a binding can take its value from: the semantic signals of
//! `docs/music.md` and the runtime's own moments. Each source says how to read it from a frame's
//! [`MusicValues`], whether it is an event (a count this frame as well as a value), at which
//! fallback levels it means something, and what it is, for the scene lab.
//!
//! A source "means something" at a level when the music at that level really measures it. The
//! signals themselves fall back as the standard says (a `tempo` from `feel` without a beat), so
//! they always have a value; what the table records is whether that value is the thing the name
//! promises. `bars` never fires without a followed beat, `cue_in` is -1 without a chart. A
//! binding that wants the real thing declares what to do where the source does not mean
//! anything (see [`super::eval`]).

use super::level::Level;
use crate::moments::{self, Strength};
use crate::scene::MusicValues;

/// What a source says in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// The signal's value; for an event source, the strength of the event (the envelope that
    /// jumps to it), 0 to 1.
    pub value: f32,
    /// How many events came since the last frame; 0 for a source that is not an event.
    pub count: u32,
}

/// One entry of the registry.
pub struct Source {
    pub name: &'static str,
    /// Whether it counts events (a source a `trigger` can use).
    pub event: bool,
    /// Whether it means something at A, B and C (see [`Level::index`]).
    pub levels: [bool; 3],
    /// One line for the lab.
    pub about: &'static str,
    read: fn(&MusicValues) -> Reading,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Source({})", self.name)
    }
}

/// Names are unique in the registry, so two sources are the same when they are called the same.
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Source {
    /// What the source says in this frame's music.
    pub fn read(&self, music: &MusicValues) -> Reading {
        (self.read)(music)
    }

    /// Whether the source means something at `level`.
    pub fn meaningful_at(&self, level: Level) -> bool {
        self.levels[level.index()]
    }
}

const ALL: [bool; 3] = [true, true, true];
const A_ONLY: [bool; 3] = [true, false, false];
const A_AND_B: [bool; 3] = [true, true, false];

const fn level_source(
    name: &'static str,
    levels: [bool; 3],
    about: &'static str,
    read: fn(&MusicValues) -> Reading,
) -> Source {
    Source {
        name,
        event: false,
        levels,
        about,
        read,
    }
}

const fn event_source(
    name: &'static str,
    levels: [bool; 3],
    about: &'static str,
    read: fn(&MusicValues) -> Reading,
) -> Source {
    Source {
        name,
        event: true,
        levels,
        about,
        read,
    }
}

fn value(value: f32) -> Reading {
    Reading { value, count: 0 }
}

fn event(value: f32, count: u32) -> Reading {
    Reading { value, count }
}

/// Every source, in the order the docs list them.
pub static SOURCES: &[Source] = &[
    level_source(
        "tempo",
        ALL,
        "The speed of the music in beats per minute, never 0.",
        |m| value(m.tempo),
    ),
    level_source(
        "clock",
        ALL,
        "A beat clock that always runs; the fraction is the phase in the beat.",
        |m| value(m.clock),
    ),
    level_source(
        "flow",
        ALL,
        "A phase in beats that moves on as much as the music does.",
        |m| value(m.flow),
    ),
    level_source("meter", ALL, "Beats per bar.", |m| value(m.meter)),
    level_source(
        "locked",
        ALL,
        "1 while the beat is followed, else 0.",
        |m| value(m.locked),
    ),
    level_source(
        "drive",
        ALL,
        "Slow energy, at the scale of a section.",
        |m| value(m.drive),
    ),
    level_source(
        "intensity",
        ALL,
        "Fast energy, at the scale of a phrase.",
        |m| value(m.intensity),
    ),
    event_source(
        "hits",
        ALL,
        "Moments to move on: every beat when it is followed, else the onsets heard.",
        |m| event(m.hit, m.hits),
    ),
    level_source("hit", ALL, "The envelope of the last hit.", |m| {
        value(m.hit)
    }),
    event_source(
        "bars",
        A_ONLY,
        "A bar line passed; only while the beat is followed.",
        |m| event(1.0, m.bars),
    ),
    event_source(
        "beats",
        A_ONLY,
        "A beat passed; only while the beat is followed.",
        |m| event(1.0, m.beats),
    ),
    event_source(
        "cues",
        ALL,
        "Big moments: section starts, or every 8 bars, or the first hit after 16 s.",
        |m| event(m.cue, m.cues),
    ),
    level_source("cue", ALL, "The envelope of the last cue.", |m| {
        value(m.cue)
    }),
    level_source(
        "cue_in",
        A_AND_B,
        "Seconds until the next section starts, -1 when unknown.",
        |m| value(m.cue_in),
    ),
    event_source(
        "impacts",
        ALL,
        "A hard hit inside a section, at most one in 6 s.",
        |m| event(m.impact, m.impacts),
    ),
    level_source("impact", ALL, "The envelope of the last impact.", |m| {
        value(m.impact)
    }),
    event_source("notes", ALL, "A melody note was played.", |m| {
        event(m.note, m.notes)
    }),
    level_source("note", ALL, "The envelope of the last note.", |m| {
        value(m.note)
    }),
    level_source(
        "section_label",
        A_AND_B,
        "Which kind of section this is: the same number for every chorus.",
        |m| value(m.section_label),
    ),
    level_source(
        "section_energy",
        A_AND_B,
        "The loudness of the section, 0 to 1.",
        |m| value(m.section_energy),
    ),
    level_source(
        "next_energy",
        A_AND_B,
        "The loudness of the section that comes next, 0 to 1.",
        |m| value(m.next_energy),
    ),
    level_source(
        "band_low",
        ALL,
        "The bass level, about 1 on average, quick to rise and slow to fall.",
        |m| value(m.band_low),
    ),
    level_source("band_mid", ALL, "The mids level, the same way.", |m| {
        value(m.band_mid)
    }),
    level_source("band_high", ALL, "The treble level, the same way.", |m| {
        value(m.band_high)
    }),
    event_source(
        "moment.strong",
        A_AND_B,
        "A section start that lifts the energy: the moment that plays the strong effects.",
        |m| match moments::moment_of(m, 0) {
            Some((Strength::Strong, _)) => event(m.cue, 1),
            _ => value(m.cue),
        },
    ),
    event_source(
        "moment.medium",
        ALL,
        "Another section start or an impact: the moment that plays the light effects.",
        |m| match moments::moment_of(m, 0) {
            // Whether it is a section start (the cue's strength) or an impact (its own).
            Some((Strength::Medium, _)) if m.cues > 0 => event(m.cue, 1),
            Some((Strength::Medium, _)) => event(m.impact, 1),
            _ => value(m.impact),
        },
    ),
];

/// The source called `name`.
pub fn find(name: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|source| source.name == name)
}

/// The names of all sources, for messages.
pub fn names() -> String {
    SOURCES
        .iter()
        .map(|source| source.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str, music: &MusicValues) -> Reading {
        find(name).unwrap().read(music)
    }

    #[test]
    fn names_are_unique_and_every_one_is_found() {
        for (i, source) in SOURCES.iter().enumerate() {
            assert!(find(source.name).is_some_and(|f| std::ptr::eq(f, source)));
            assert!(SOURCES[..i].iter().all(|other| other.name != source.name));
            assert!(!source.about.is_empty());
        }
        assert!(find("loudness").is_none());
    }

    #[test]
    fn a_source_reads_its_field_and_an_event_its_count() {
        let music = MusicValues {
            tempo: 128.0,
            hits: 2,
            hit: 0.7,
            bars: 1,
            cues: 1,
            cue: 0.5,
            ..Default::default()
        };
        assert_eq!(read("tempo", &music), value(128.0));
        assert_eq!(read("hits", &music), event(0.7, 2));
        assert_eq!(read("hit", &music), value(0.7));
        assert_eq!(read("bars", &music), event(1.0, 1));
        assert_eq!(read("cues", &music), event(0.5, 1));
        assert!(find("hits").unwrap().event && !find("hit").unwrap().event);
    }

    #[test]
    fn what_means_something_follows_the_fallback_table() {
        let at = |name: &str| find(name).unwrap().levels;
        assert_eq!(at("bars"), [true, false, false]);
        assert_eq!(at("beats"), [true, false, false]);
        assert_eq!(at("cue_in"), [true, true, false]);
        assert_eq!(at("section_label"), [true, true, false]);
        assert_eq!(at("section_energy"), [true, true, false]);
        assert_eq!(at("hits"), [true, true, true]);
        assert_eq!(at("drive"), [true, true, true]);
        assert!(find("clock").unwrap().meaningful_at(Level::C));
        assert!(!find("bars").unwrap().meaningful_at(Level::B));
    }

    #[test]
    fn the_runtimes_moments_are_those_of_the_effects() {
        // A section start that lifts the energy is strong; another one, medium.
        let lift = MusicValues {
            cues: 1,
            cue: 1.0,
            ..Default::default()
        };
        assert_eq!(read("moment.strong", &lift), event(1.0, 1));
        assert_eq!(read("moment.medium", &lift).count, 0);
        let start = MusicValues {
            cues: 1,
            cue: 0.5,
            ..Default::default()
        };
        assert_eq!(read("moment.strong", &start).count, 0);
        assert_eq!(read("moment.medium", &start), event(0.5, 1));
        // A weak cue is no moment, and an impact is a medium one with its own strength.
        let weak = MusicValues {
            cues: 1,
            cue: 0.3,
            ..Default::default()
        };
        assert_eq!(read("moment.medium", &weak).count, 0);
        let impact = MusicValues {
            impacts: 1,
            impact: 0.9,
            ..Default::default()
        };
        assert_eq!(read("moment.medium", &impact), event(0.9, 1));
        assert_eq!(read("moment.strong", &impact).count, 0);
    }
}
