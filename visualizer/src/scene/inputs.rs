//! What a scene hears: `struct Music`, filled from the song's chart when it has one and from the
//! live beat tracker and spectrum otherwise, with the events since the last frame counted so
//! that a slow frame never loses a beat.
//!
//! A scene is only told of a beat when there is one to trust ([`crate::chart::trusted_rhythm`],
//! [`beat_trusted`]); for music without a beat it gets `hits` instead, the moments when
//! something audible happens ([`HitCounter`]). On top of those it fills in the signals scenes
//! are meant to read ([`Signals`]).

use std::time::Instant;

use crate::chart::ChartState;
use crate::rhythm::Rhythm;

use super::signals::{HitCounter, HitInput, SignalInput, Signals};

/// The live tracker's confidence from which its beats count; the whole-song analysis calls a
/// song pulsed from the same value, so the two agree on what a steady beat is.
pub const PULSE_CONFIDENCE: f64 = 0.3;
/// The longest wall time (seconds) one frame fades `hit` by, so a long pause empties it.
const MAX_WALL_DT: f64 = 1.0;
/// A 4-byte field of a uniform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Word {
    F(f32),
    U(u32),
}

/// Live levels from the spectrum, about 1 on average.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Levels {
    pub bass: f32,
    pub mids: f32,
    pub treble: f32,
    /// How fast it sounds now and over the song so far (Hz, 0 until measured).
    pub feel: f32,
    pub song_feel: f32,
}

/// The values of `struct Music`. What each one means is written in `docs/music.md` (the
/// reference, written in a later step); the fields after `hit` are the signals of
/// [`Signals`], which built-in scenes read instead of `bpm`, `feel` and `energy`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MusicValues {
    pub song_time: f32,
    pub playing: u32,
    pub bpm: f32,
    pub beat: f32,
    pub beat_confidence: f32,
    pub beats_per_bar: f32,
    pub bar: f32,
    pub meter_confidence: f32,
    pub beats: u32,
    pub bars: u32,
    pub sections: u32,
    pub section: f32,
    pub section_label: f32,
    pub section_repeat: f32,
    pub section_energy: f32,
    pub next_label: f32,
    pub next_energy: f32,
    pub section_time: f32,
    pub section_left: f32,
    pub section_confidence: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub harmonic: f32,
    pub phrase: f32,
    pub lead: f32,
    pub lead_onset: f32,
    pub loudness: f32,
    pub brightness: f32,
    pub percussive: f32,
    pub pulse: f32,
    pub bass: f32,
    pub mids: f32,
    pub treble: f32,
    pub energy: f32,
    pub has_chart: u32,
    pub feel: f32,
    pub song_feel: f32,
    /// How many hits (moments to move on) since the last frame, and an envelope that is the
    /// hit's strength when one happens and fades after.
    pub hits: u32,
    pub hit: f32,
    /// Tempo in bpm (never 0) and a beat clock in beats that always runs; beats per bar, how
    /// sure the beat is (0 to 1), and the slow and the faster energy of the music (0 to 1).
    pub tempo: f32,
    pub clock: f32,
    pub meter: f32,
    pub locked: f32,
    pub drive: f32,
    pub intensity: f32,
    /// Big moments since the last frame, the envelope of the last one, and the seconds to the
    /// next section (-1 without a chart).
    pub cues: u32,
    pub cue: f32,
    pub cue_in: f32,
    /// Melody notes since the last frame and their envelope.
    pub notes: u32,
    pub note: f32,
    /// The bass, mids and treble smoothed (quick to rise, slow to fall).
    pub band_low: f32,
    pub band_mid: f32,
    pub band_high: f32,
    /// Hard hits inside a section since the last frame and their envelope.
    pub impacts: u32,
    pub impact: f32,
    /// A phase in beats that moves on as much as the music does (MilkDrop's way of moving with
    /// `bass_att`), for motion that needs no beat.
    pub flow: f32,
    /// How high the section is in the song, 0 to 1: half its loudness, half its rank among the
    /// song's sections (see `scene::signals`).
    pub level: f32,
}

impl MusicValues {
    /// Name and value of every field, in the order of `struct Music`. New fields go at the end.
    pub fn fields(&self) -> [(&'static str, Word); 58] {
        use Word::{F, U};
        [
            ("song_time", F(self.song_time)),
            ("playing", U(self.playing)),
            ("bpm", F(self.bpm)),
            ("beat", F(self.beat)),
            ("beat_confidence", F(self.beat_confidence)),
            ("beats_per_bar", F(self.beats_per_bar)),
            ("bar", F(self.bar)),
            ("meter_confidence", F(self.meter_confidence)),
            ("beats", U(self.beats)),
            ("bars", U(self.bars)),
            ("sections", U(self.sections)),
            ("section", F(self.section)),
            ("section_label", F(self.section_label)),
            ("section_repeat", F(self.section_repeat)),
            ("section_energy", F(self.section_energy)),
            ("next_label", F(self.next_label)),
            ("next_energy", F(self.next_energy)),
            ("section_time", F(self.section_time)),
            ("section_left", F(self.section_left)),
            ("section_confidence", F(self.section_confidence)),
            ("low", F(self.low)),
            ("mid", F(self.mid)),
            ("high", F(self.high)),
            ("harmonic", F(self.harmonic)),
            ("phrase", F(self.phrase)),
            ("lead", F(self.lead)),
            ("lead_onset", F(self.lead_onset)),
            ("loudness", F(self.loudness)),
            ("brightness", F(self.brightness)),
            ("percussive", F(self.percussive)),
            ("pulse", F(self.pulse)),
            ("bass", F(self.bass)),
            ("mids", F(self.mids)),
            ("treble", F(self.treble)),
            ("energy", F(self.energy)),
            ("has_chart", U(self.has_chart)),
            ("feel", F(self.feel)),
            ("song_feel", F(self.song_feel)),
            ("hits", U(self.hits)),
            ("hit", F(self.hit)),
            ("tempo", F(self.tempo)),
            ("clock", F(self.clock)),
            ("meter", F(self.meter)),
            ("locked", F(self.locked)),
            ("drive", F(self.drive)),
            ("intensity", F(self.intensity)),
            ("cues", U(self.cues)),
            ("cue", F(self.cue)),
            ("cue_in", F(self.cue_in)),
            ("notes", U(self.notes)),
            ("note", F(self.note)),
            ("band_low", F(self.band_low)),
            ("band_mid", F(self.band_mid)),
            ("band_high", F(self.band_high)),
            ("impacts", U(self.impacts)),
            ("impact", F(self.impact)),
            ("flow", F(self.flow)),
            ("level", F(self.level)),
        ]
    }

    /// The uniform's bytes, padded to a multiple of 16.
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes: Vec<u8> = self
            .fields()
            .iter()
            .flat_map(|(_, word)| match *word {
                Word::F(v) => v.to_le_bytes(),
                Word::U(v) => v.to_le_bytes(),
            })
            .collect();
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        bytes
    }
}

/// Counts how many whole beats, bars and sections went by between frames, and the hits. A jump
/// backwards (a seek, a new song) counts nothing and starts over from there; so does a jump of
/// more than [`EventCounter::MAX_JUMP`] (a seek forwards), which would otherwise fire a burst.
#[derive(Debug, Default)]
pub struct EventCounter {
    last: Option<(f64, f64, f64)>,
    /// Whether the last frame had a chart: the beat, bar and section of a chart and of the live
    /// tracker are unrelated numbers, so a change of source is a jump.
    chart: Option<bool>,
    /// The hits made of onsets, when there is no beat.
    hits: HitCounter,
    /// The signals made of the raw values.
    signals: Signals,
    /// When the last frame was, for fading `hit` by wall time (a paused song still fades it).
    wall: Option<Instant>,
    /// A test that draws a timeline as fast as the GPU goes: the wall seconds every frame after
    /// the first counts as (see [`EventCounter::with_wall_dt`]).
    #[cfg(test)]
    fixed_wall_dt: Option<f64>,
}

impl EventCounter {
    const MAX_JUMP: f64 = 8.0;

    /// A counter that starts from nothing and counts every frame after the first as `dt` wall
    /// seconds, however long it took to draw. The music's signals (the tempo's glide, the clock,
    /// the drive, the envelopes) move on by wall time, so a timeline drawn as fast as the GPU
    /// goes, and a slow first frame (a shader compiled cold), would otherwise move them by
    /// something different on every run.
    #[cfg(test)]
    pub(crate) fn with_wall_dt(dt: f64) -> Self {
        Self {
            fixed_wall_dt: Some(dt),
            ..Self::default()
        }
    }

    /// The crossings since the last call, for the beat, bar and section counters now.
    pub fn tick(&mut self, beat: f64, bar: f64, section: f64) -> (u32, u32, u32) {
        let crossed = |before: f64, now: f64| {
            let n = now.floor() - before.floor();
            if (1.0..=Self::MAX_JUMP).contains(&n) {
                n as u32
            } else {
                0
            }
        };
        let counts = match self.last {
            Some((b, r, s)) => (crossed(b, beat), crossed(r, bar), crossed(s, section)),
            None => (0, 0, 0),
        };
        self.last = Some((beat, bar, section));
        counts
    }

    /// Tells whether this frame has a chart; when that changes, the counting starts over.
    fn follow(&mut self, chart: bool) {
        if self.chart.is_some_and(|before| before != chart) {
            self.last = None;
        }
        self.chart = Some(chart);
    }

    /// Wall seconds since the last frame (0 on the first), at most [`MAX_WALL_DT`].
    fn wall_dt(&mut self) -> f64 {
        let now = Instant::now();
        let dt = self.wall.map_or(0.0, |before| {
            now.saturating_duration_since(before).as_secs_f64()
        });
        #[cfg(test)]
        let dt = match self.fixed_wall_dt {
            Some(fixed) if self.wall.is_some() => fixed,
            _ => dt,
        };
        self.wall = Some(now);
        dt.min(MAX_WALL_DT)
    }
}

/// Whether the beats of `rhythm` (as [`trusted_rhythm`] left it) are to be told to scenes: those
/// of a chart always are, those of the live tracker only from [`PULSE_CONFIDENCE`].
pub fn beat_trusted(rhythm: &Rhythm, has_chart: bool) -> bool {
    rhythm.bpm > 0.0 && (has_chart || rhythm.confidence >= PULSE_CONFIDENCE)
}

/// `struct Music` for this frame. `song` is the song time and whether it plays.
pub fn music(
    rhythm: &Rhythm,
    chart: Option<&ChartState>,
    levels: Levels,
    song: (f64, bool),
    events: &mut EventCounter,
) -> MusicValues {
    events.follow(chart.is_some());
    let mut m = MusicValues {
        song_time: song.0 as f32,
        playing: u32::from(song.1),
        bpm: rhythm.bpm as f32,
        beat: rhythm.beat as f32,
        beat_confidence: rhythm.confidence as f32,
        bass: levels.bass,
        mids: levels.mids,
        treble: levels.treble,
        energy: rhythm.energy as f32,
        lead: -1.0,
        feel: levels.feel,
        song_feel: levels.song_feel,
        ..Default::default()
    };
    let (bar, section) = match chart {
        Some(c) => {
            m.has_chart = 1;
            m.beats_per_bar = c.beats_per_bar as f32;
            m.bar = c.bar as f32;
            m.meter_confidence = c.meter_confidence as f32;
            m.section = c.section as f32;
            m.section_label = c.section_label as f32;
            m.section_repeat = c.section_repeat as f32;
            m.section_energy = c.section_energy as f32;
            m.next_label = c.next_label as f32;
            m.next_energy = c.next_energy as f32;
            m.section_time = c.section_time as f32;
            m.section_left = c.section_left as f32;
            m.section_confidence = c.section_confidence as f32;
            m.low = c.events[0] as f32;
            m.mid = c.events[1] as f32;
            m.high = c.events[2] as f32;
            m.harmonic = c.events[3] as f32;
            m.phrase = c.events[4] as f32;
            // The chart counts streams from 1 with 0 for none.
            m.lead = c.lead as f32 - 1.0;
            m.lead_onset = c.lead_event as f32;
            m.loudness = c.loudness as f32;
            m.brightness = c.brightness as f32;
            m.percussive = c.percussive as f32;
            m.pulse = c.pulse as f32;
            // The chart heard the whole song; older charts lack it, and the live measure stays.
            if c.feel > 0.0 {
                m.feel = c.feel as f32;
                m.song_feel = c.song_feel as f32;
            }
            (c.bar, c.section)
        }
        None => {
            // Without a chart: bars of four from the tracker's beats, one long section, and
            // levels from what plays now.
            m.beats_per_bar = 4.0;
            m.bar = (rhythm.beat / 4.0) as f32;
            m.low = rhythm.kick as f32;
            m.mid = rhythm.snare as f32;
            m.high = rhythm.hat as f32;
            m.loudness = (rhythm.energy as f32 / 2.0).clamp(0.0, 1.0);
            m.section_energy = m.loudness;
            m.next_energy = m.loudness;
            m.pulse = rhythm.confidence as f32;
            (rhythm.beat / 4.0, 0.0)
        }
    };
    let beat = if rhythm.bpm > 0.0 { rhythm.beat } else { 0.0 };
    (m.beats, m.bars, m.sections) = events.tick(beat, bar, section);
    // Beats and bars only count where there is a beat to trust; the counters ran on regardless,
    // so that a beat found later does not fire what went by. Nor is an untrusted tempo shown:
    // a scene reads `bpm > 0` as "there is a beat" and takes its speeds from `feel` otherwise.
    // A chart's beats run on through a silent end or gap, but nothing is heard there to follow.
    let trusted = beat_trusted(rhythm, chart.is_some()) && !chart.is_some_and(|c| c.silent);
    if !trusted {
        (m.beats, m.bars, m.bpm) = (0, 0, 0.0);
    }
    // The wall time is taken once: asking again would give the time since this very call.
    let wall_dt = events.wall_dt();
    let input = HitInput {
        beats: m.beats,
        trusted,
        chart,
        rhythm,
        song_time: song.0,
        wall_dt,
    };
    (m.hits, m.hit) = events.hits.tick(&input);
    // The signals read the finished raw values, so they come last.
    let input = SignalInput {
        trusted,
        chart,
        rhythm,
        song_time: song.0,
        wall_dt,
    };
    events.signals.tick(&input, &mut m);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::trusted_rhythm;

    #[test]
    fn a_slow_frame_still_counts_every_beat() {
        let mut events = EventCounter::default();
        assert_eq!(events.tick(10.9, 2.7, 0.0), (0, 0, 0));
        assert_eq!(events.tick(11.0, 2.75, 0.0), (1, 0, 0));
        assert_eq!(events.tick(11.5, 2.9, 0.0), (0, 0, 0));
        // Three beats and a bar line inside one frame.
        assert_eq!(events.tick(14.1, 3.5, 1.0), (3, 1, 1));
    }

    #[test]
    fn seeking_fires_nothing() {
        let mut events = EventCounter::default();
        events.tick(40.2, 10.0, 3.0);
        assert_eq!(events.tick(3.5, 0.9, 0.0), (0, 0, 0));
        assert_eq!(events.tick(4.0, 1.0, 0.0), (1, 1, 0));
        // A seek forwards lands far ahead: no burst of beats.
        assert_eq!(events.tick(120.0, 30.0, 9.0), (0, 0, 0));
    }

    #[test]
    fn music_fields_pack_in_order() {
        let m = MusicValues {
            song_time: 2.0,
            has_chart: 1,
            hits: 3,
            hit: 0.5,
            tempo: 120.0,
            cues: 2,
            impact: 0.25,
            flow: 7.5,
            ..Default::default()
        };
        let bytes = m.bytes();
        // 58 words padded to a multiple of 16 bytes; the newest are last and the old ones have
        // not moved.
        assert_eq!(m.fields().len(), 58);
        assert_eq!(bytes.len(), 240);
        assert_eq!(&bytes[0..4], &2.0f32.to_le_bytes());
        assert_eq!(&bytes[140..144], &1u32.to_le_bytes());
        assert_eq!(&bytes[152..156], &3u32.to_le_bytes());
        assert_eq!(&bytes[156..160], &0.5f32.to_le_bytes());
        assert_eq!(&bytes[160..164], &120.0f32.to_le_bytes());
        assert_eq!(&bytes[184..188], &2u32.to_le_bytes());
        assert_eq!(&bytes[220..224], &0.25f32.to_le_bytes());
        assert_eq!(&bytes[224..228], &7.5f32.to_le_bytes());
        assert_eq!(&bytes[228..240], &[0; 12]);
        let names: Vec<_> = m.fields().iter().map(|(name, _)| *name).collect();
        assert_eq!(&names[39..42], ["hit", "tempo", "clock"]);
        assert_eq!(names[55], "impact");
        assert_eq!(names[56], "flow");
    }

    #[test]
    fn without_a_chart_the_tracker_fills_in() {
        let rhythm = Rhythm {
            bpm: 120.0,
            beat: 9.25,
            confidence: 0.8,
            kick: 0.5,
            energy: 1.2,
            ..Default::default()
        };
        let mut events = EventCounter::default();
        let m = music(&rhythm, None, Levels::default(), (3.0, true), &mut events);
        assert_eq!(m.has_chart, 0);
        assert_eq!(m.lead, -1.0);
        assert_eq!(m.low, 0.5);
        assert!((m.bar - 2.3125).abs() < 1e-6);
        assert!((m.loudness - 0.6).abs() < 1e-6);
    }

    #[test]
    fn the_live_tracker_is_trusted_only_when_sure_and_a_chart_with_beats_always() {
        let live = |bpm, confidence| Rhythm {
            bpm,
            confidence,
            ..Default::default()
        };
        assert!(!beat_trusted(&live(120.0, 0.05), false));
        assert!(!beat_trusted(&live(120.0, 0.29), false));
        assert!(beat_trusted(&live(120.0, 0.3), false));
        assert!(!beat_trusted(&live(0.0, 0.9), false));
        // The chart's own beats are trusted at any confidence; none, none.
        assert!(beat_trusted(&live(120.0, 0.1), true));
        assert!(!beat_trusted(&live(0.0, 0.0), true));
    }

    #[test]
    fn an_unsure_tracker_tells_no_beats_and_a_sure_one_does() {
        let rhythm = |confidence, beat| Rhythm {
            bpm: 120.0,
            beat,
            confidence,
            ..Default::default()
        };
        let mut events = EventCounter::default();
        music(
            &rhythm(0.05, 9.9),
            None,
            Levels::default(),
            (1.0, true),
            &mut events,
        );
        let m = music(
            &rhythm(0.05, 10.1),
            None,
            Levels::default(),
            (1.1, true),
            &mut events,
        );
        assert_eq!((m.beats, m.bars), (0, 0));
        // Its tempo is not shown either: scenes take their speeds from `feel` then.
        assert_eq!(m.bpm, 0.0);
        // Sure now: the next beat counts, and no beat is owed for the ones before.
        let m = music(
            &rhythm(0.9, 10.9),
            None,
            Levels::default(),
            (1.5, true),
            &mut events,
        );
        assert_eq!(m.beats, 0);
        let m = music(
            &rhythm(0.9, 11.1),
            None,
            Levels::default(),
            (1.6, true),
            &mut events,
        );
        assert_eq!(m.beats, 1);
        assert_eq!(m.bpm, 120.0);
        assert_eq!((m.hits, m.hit), (1, 1.0));
    }

    #[test]
    fn a_silent_stretch_of_a_chart_tells_no_beat_and_none_piled_up_after() {
        let live = Rhythm::default();
        let at = |beat: f64, silent| ChartState {
            bpm: 120.0,
            beat,
            beat_confidence: 0.0,
            bar: beat / 4.0,
            silent,
            ..Default::default()
        };
        let mut events = EventCounter::default();
        let mut step = |chart: ChartState| {
            let rhythm = trusted_rhythm(live, Some(&chart));
            let m = music(
                &rhythm,
                Some(&chart),
                Levels::default(),
                (chart.beat / 2.0, true),
                &mut events,
            );
            (m.beats, m.bars, m.bpm, m.hits)
        };
        step(at(3.9, false));
        assert_eq!(step(at(4.0, false)), (1, 1, 120.0, 1));
        // The chart's grid goes on through the silence: nothing is told.
        for beat in [5.0, 6.0, 7.0, 8.0] {
            assert_eq!(step(at(beat, true)), (0, 0, 0.0, 0));
        }
        // The sound comes back: the next beat counts, the ones that went by do not.
        assert_eq!(step(at(8.5, false)), (0, 0, 120.0, 0));
        assert_eq!(step(at(9.0, false)), (1, 0, 120.0, 1));
    }

    #[test]
    fn a_chart_without_beats_tells_none_though_the_tracker_is_sure() {
        let live = Rhythm {
            bpm: 120.0,
            beat: 40.0,
            confidence: 0.9,
            kick: 0.2,
            ..Default::default()
        };
        let chart = ChartState {
            lead: 1.0,
            ..Default::default()
        };
        let rhythm = trusted_rhythm(live, Some(&chart));
        assert_eq!(
            (rhythm.bpm, rhythm.beat, rhythm.confidence),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(rhythm.kick, 0.2);
        let mut events = EventCounter::default();
        for i in 0..5 {
            let m = music(
                &rhythm,
                Some(&chart),
                Levels::default(),
                (f64::from(i), true),
                &mut events,
            );
            assert_eq!(
                (m.beats, m.bars, m.bpm, m.beat_confidence),
                (0, 0, 0.0, 0.0)
            );
        }
    }

    #[test]
    fn the_chart_taking_over_fires_nothing_and_a_chart_with_beats_replaces_the_tracker() {
        let live = Rhythm {
            bpm: 90.0,
            beat: 20.5,
            confidence: 0.9,
            ..Default::default()
        };
        let mut events = EventCounter::default();
        music(&live, None, Levels::default(), (10.0, true), &mut events);
        let m = music(&live, None, Levels::default(), (10.1, true), &mut events);
        assert_eq!(m.beats, 0);
        // The chart arrives with its beat count a few ahead of the tracker's, as a number
        // unrelated to it: no burst.
        let chart = ChartState {
            bpm: 128.0,
            beat: 24.2,
            beat_confidence: 0.5,
            bar: 6.0,
            ..Default::default()
        };
        let rhythm = trusted_rhythm(live, Some(&chart));
        assert_eq!(
            (rhythm.bpm, rhythm.beat, rhythm.confidence),
            (128.0, 24.2, 0.5)
        );
        let m = music(
            &rhythm,
            Some(&chart),
            Levels::default(),
            (10.2, true),
            &mut events,
        );
        assert_eq!((m.beats, m.hits), (0, 0));
        let chart = ChartState {
            beat: 25.1,
            bar: 6.2,
            ..chart
        };
        let rhythm = trusted_rhythm(live, Some(&chart));
        let m = music(
            &rhythm,
            Some(&chart),
            Levels::default(),
            (10.3, true),
            &mut events,
        );
        assert_eq!((m.beats, m.hits), (1, 1));
    }
}
