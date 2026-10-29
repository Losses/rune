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

/// The loudness (0 to 1, against the song's own range with a chart, the live level without one)
/// from which the music counts as fully heard for `audible`: quieter than that it is fading or
/// silent.
const AUDIBLE_FULL: f32 = 0.25;

/// How much is heard in this frame's music: 1 at a usual loudness, falling with a fade-out to 0 in
/// silence (the `audible` source).
pub fn audible(music: &MusicValues) -> f32 {
    (music.loudness / AUDIBLE_FULL).clamp(0.0, 1.0)
}

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
    read: fn(&MusicValues, &Frame) -> Reading,
}

/// What a frame has besides the music, for the sources that are not in it.
#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    /// Whether the scene's director cut to another shot in this frame.
    cut: bool,
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
    /// What the source says in this frame's music, in a frame where the scene did not cut (the
    /// runtime's own bindings have no scene, so `cut` never counts there).
    pub fn read(&self, music: &MusicValues) -> Reading {
        self.read_in(music, false)
    }

    /// As [`Source::read`], in a frame where the scene's director did (`cut`) or did not cut.
    pub fn read_in(&self, music: &MusicValues, cut: bool) -> Reading {
        (self.read)(music, &Frame { cut })
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
    read: fn(&MusicValues, &Frame) -> Reading,
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
    read: fn(&MusicValues, &Frame) -> Reading,
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
        |m, _| value(m.tempo),
    ),
    level_source(
        "clock",
        ALL,
        "A beat clock that always runs; the fraction is the phase in the beat.",
        |m, _| value(m.clock),
    ),
    level_source(
        "flow",
        ALL,
        "A phase in beats that moves on as much as the music does.",
        |m, _| value(m.flow),
    ),
    level_source("meter", ALL, "Beats per bar.", |m, _| value(m.meter)),
    level_source(
        "locked",
        ALL,
        "1 while the beat is followed, else 0.",
        |m, _| value(m.locked),
    ),
    level_source(
        "drive",
        ALL,
        "Slow energy, at the scale of a section.",
        |m, _| value(m.drive),
    ),
    level_source(
        "intensity",
        ALL,
        "Fast energy, at the scale of a phrase.",
        |m, _| value(m.intensity),
    ),
    level_source(
        "section_level",
        ALL,
        "How high the section is in the song: half its loudness, half how it ranks among the song's sections (the loudest 1), so every song has quiet and high parts and a loud one more high ones.",
        |m, _| value(m.level),
    ),
    level_source(
        "audible",
        ALL,
        "How much is heard: 1 at a usual loudness, falling with a fade-out to 0 in silence.",
        |m, _| value(audible(m)),
    ),
    event_source(
        "hits",
        ALL,
        "Moments to move on: every beat when it is followed, else the onsets heard.",
        |m, _| event(m.hit, m.hits),
    ),
    level_source("hit", ALL, "The envelope of the last hit.", |m, _| {
        value(m.hit)
    }),
    level_source(
        "bar",
        A_ONLY,
        "The bar number since the chart's start, the fraction the phase in the bar.",
        |m, _| value(m.bar),
    ),
    level_source(
        "beat",
        A_ONLY,
        "The beat number since the song's first beat, the fraction the phase in the beat.",
        |m, _| value(m.beat),
    ),
    event_source(
        "bars",
        A_ONLY,
        "A bar line passed; only while the beat is followed.",
        |m, _| event(1.0, m.bars),
    ),
    event_source(
        "beats",
        A_ONLY,
        "A beat passed; only while the beat is followed.",
        |m, _| event(1.0, m.beats),
    ),
    event_source(
        "cues",
        ALL,
        "Big moments: section starts, or every 8 bars, or the first hit after 16 s.",
        |m, _| event(m.cue, m.cues),
    ),
    level_source("cue", ALL, "The envelope of the last cue.", |m, _| {
        value(m.cue)
    }),
    level_source(
        "cue_in",
        A_AND_B,
        "Seconds until the next section starts, -1 when unknown.",
        |m, _| value(m.cue_in),
    ),
    event_source(
        "impacts",
        ALL,
        "A hard hit inside a section, at most one in 6 s.",
        |m, _| event(m.impact, m.impacts),
    ),
    level_source("impact", ALL, "The envelope of the last impact.", |m, _| {
        value(m.impact)
    }),
    event_source("notes", ALL, "A melody note was played.", |m, _| {
        event(m.note, m.notes)
    }),
    level_source("note", ALL, "The envelope of the last note.", |m, _| {
        value(m.note)
    }),
    level_source(
        "section_label",
        A_AND_B,
        "Which kind of section this is: the same number for every chorus.",
        |m, _| value(m.section_label),
    ),
    level_source(
        "section",
        A_AND_B,
        "The index of the section in the chart, counting from 0: every section has its own number, unlike its label.",
        |m, _| value(m.section),
    ),
    level_source(
        "section_energy",
        A_AND_B,
        "The loudness of the section, 0 to 1.",
        |m, _| value(m.section_energy),
    ),
    level_source(
        "next_energy",
        A_AND_B,
        "The loudness of the section that comes next, 0 to 1.",
        |m, _| value(m.next_energy),
    ),
    level_source(
        "low",
        ALL,
        "The percussive low band: jumps to an onset's strength, then decays (the kick without a chart).",
        |m, _| value(m.low),
    ),
    level_source(
        "mid",
        ALL,
        "The percussive mid band the same way (the snare without a chart).",
        |m, _| value(m.mid),
    ),
    level_source(
        "high",
        ALL,
        "The percussive high band the same way (the hat without a chart).",
        |m, _| value(m.high),
    ),
    level_source(
        "band_low",
        ALL,
        "The bass level, about 1 on average, quick to rise and slow to fall.",
        |m, _| value(m.band_low),
    ),
    level_source("band_mid", ALL, "The mids level, the same way.", |m, _| {
        value(m.band_mid)
    }),
    level_source(
        "band_high",
        ALL,
        "The treble level, the same way.",
        |m, _| value(m.band_high),
    ),
    event_source(
        "cut",
        ALL,
        "The scene's own camera cut to another shot in this frame, or took its first (strength 1); never in a scene without shots.",
        |_, frame| event(1.0, u32::from(frame.cut)),
    ),
    event_source(
        "moment.strong",
        A_AND_B,
        "A section start that lifts the energy: the moment that plays the strong effects.",
        |m, _| match moments::moment_of(m, 0) {
            Some((Strength::Strong, _)) => event(m.cue, 1),
            _ => value(m.cue),
        },
    ),
    event_source(
        "moment.medium",
        ALL,
        "Another section start or an impact: the moment that plays the light effects.",
        |m, _| match moments::moment_of(m, 0) {
            // Whether it is a section start (the cue's strength) or an impact (its own).
            Some((Strength::Medium, _)) if m.cues > 0 => event(m.cue, 1),
            Some((Strength::Medium, _)) => event(m.impact, 1),
            _ => value(m.impact),
        },
    ),
];

/// The names of the sources that count events, for messages.
pub fn event_names() -> String {
    SOURCES
        .iter()
        .filter(|source| source.event)
        .map(|source| source.name)
        .collect::<Vec<_>>()
        .join(", ")
}

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
    fn audible_is_full_at_a_usual_loudness_and_falls_with_a_fade() {
        let at = |loudness| {
            read(
                "audible",
                &MusicValues {
                    loudness,
                    ..Default::default()
                },
            )
            .value
        };
        assert_eq!(at(0.7), 1.0);
        assert_eq!(at(0.25), 1.0);
        assert_eq!(at(0.125), 0.5);
        assert_eq!(at(0.0), 0.0);
    }

    #[test]
    fn what_means_something_follows_the_fallback_table() {
        let at = |name: &str| find(name).unwrap().levels;
        assert_eq!(at("bars"), [true, false, false]);
        assert_eq!(at("beats"), [true, false, false]);
        assert_eq!(at("bar"), [true, false, false]);
        assert_eq!(at("beat"), [true, false, false]);
        assert_eq!(at("low"), [true, true, true]);
        assert_eq!(at("cue_in"), [true, true, false]);
        assert_eq!(at("section_label"), [true, true, false]);
        assert_eq!(at("section_energy"), [true, true, false]);
        assert_eq!(at("hits"), [true, true, true]);
        assert_eq!(at("drive"), [true, true, true]);
        assert!(find("clock").unwrap().meaningful_at(Level::C));
        assert!(!find("bars").unwrap().meaningful_at(Level::B));
    }

    #[test]
    fn the_bar_the_beat_and_the_percussive_bands_read_their_fields() {
        let music = MusicValues {
            bar: 12.25,
            beat: 49.5,
            low: 0.9,
            mid: 0.5,
            high: 0.1,
            ..Default::default()
        };
        assert_eq!(read("bar", &music), value(12.25));
        assert_eq!(read("beat", &music), value(49.5));
        assert_eq!(read("low", &music), value(0.9));
        assert_eq!(read("mid", &music), value(0.5));
        assert_eq!(read("high", &music), value(0.1));
        // Values, not events; the bar and the beat only mean something with a followed beat.
        assert!(
            ["bar", "beat", "low", "mid", "high"]
                .iter()
                .all(|n| !find(n).unwrap().event)
        );
        assert!(!find("bar").unwrap().meaningful_at(Level::B));
        assert!(find("high").unwrap().meaningful_at(Level::C));
    }

    #[test]
    fn the_section_is_the_index_the_chart_gives_and_means_something_where_it_does() {
        let music = MusicValues {
            section: 3.0,
            section_label: 1.0,
            ..Default::default()
        };
        assert_eq!(read("section", &music), value(3.0));
        assert_eq!(read("section_label", &music), value(1.0));
        let section = find("section").unwrap();
        assert!(!section.event);
        assert_eq!(section.levels, find("section_label").unwrap().levels);
        assert!(section.meaningful_at(Level::A) && section.meaningful_at(Level::B));
        assert!(!section.meaningful_at(Level::C));
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
