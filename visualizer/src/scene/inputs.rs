//! What a scene hears: `struct Music`, filled from the song's chart when it has one and from the
//! live beat tracker and spectrum otherwise, with the events since the last frame counted so
//! that a slow frame never loses a beat.
//!
//! A scene is only told of a beat when there is one to trust ([`crate::chart::trusted_rhythm`],
//! [`beat_trusted`]); for music without a beat it gets `hits` instead, the moments when
//! something audible happens ([`HitCounter`]).

use std::time::Instant;

use crate::chart::{ChartState, LatestOnset};
use crate::rhythm::Rhythm;

/// The live tracker's confidence from which its beats count; the whole-song analysis calls a
/// song pulsed from the same value, so the two agree on what a steady beat is.
pub const PULSE_CONFIDENCE: f64 = 0.3;
/// Time constant (seconds) of `hit` fading after a hit: a swell a scene can ride that is gone
/// within about half a second, slower than a kick's own envelope so that it reads on a screen.
pub const HIT_DECAY: f64 = 0.2;
/// The least time (seconds) between two hits made from onsets: twice that is 4 per second, the
/// most a scene should be asked to move on, whatever the music does.
pub const HIT_GAP: f64 = 0.25;
/// Onsets of the chart weaker than this (strength 0 to about 1, relative to the song) are not
/// hits: ornaments and noise, not moments.
pub const CHART_HIT_MIN: f64 = 0.3;
/// A live onset (kick, snare or hat, 1 on a hit as strong as the recent strongest) must rise
/// above this to be a hit.
pub const LIVE_HIT_MIN: f64 = 0.5;
/// A song time that moves by more than this between frames (seconds) is a seek or a stall, not
/// a frame: no hit comes of it.
pub const HIT_MAX_FRAME: f64 = 2.0;
/// The longest wall time (seconds) one frame fades `hit` by, so a long pause empties it.
const MAX_WALL_DT: f64 = 1.0;
/// The streams of the chart, as indices into [`ChartState::latest`], that make hits when there
/// is no beat: the lead (or this one when nothing leads) and the phrase starts.
const FALLBACK_STREAM: usize = 3;
const PHRASE_STREAM: usize = 4;

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

/// The values of `struct Music`; see the format draft for what each one means.
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
}

impl MusicValues {
    /// Name and value of every field, in the order of `struct Music`. New fields go at the end.
    pub fn fields(&self) -> [(&'static str, Word); 40] {
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
    /// When the last frame was, for fading `hit` by wall time (a paused song still fades it).
    wall: Option<Instant>,
}

impl EventCounter {
    const MAX_JUMP: f64 = 8.0;

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
        self.wall = Some(now);
        dt.min(MAX_WALL_DT)
    }
}

/// Whether the beats of `rhythm` (as [`trusted_rhythm`] left it) are to be told to scenes: those
/// of a chart always are, those of the live tracker only from [`PULSE_CONFIDENCE`].
pub fn beat_trusted(rhythm: &Rhythm, has_chart: bool) -> bool {
    rhythm.bpm > 0.0 && (has_chart || rhythm.confidence >= PULSE_CONFIDENCE)
}

/// What [`HitCounter::tick`] looks at in one frame.
pub struct HitInput<'a> {
    /// The beats of this frame when they are to be trusted (0 otherwise), and whether they are.
    pub beats: u32,
    pub trusted: bool,
    pub chart: Option<&'a ChartState>,
    pub rhythm: &'a Rhythm,
    /// The song time, and the wall seconds since the last frame.
    pub song_time: f64,
    pub wall_dt: f64,
}

/// Makes the moments to move on from whatever the music has. Tick it once per frame.
///
/// In order of what there is: every trusted beat (strength 1); else, with a chart, its onsets of
/// the leading stream (the harmonic one when none leads) and phrase starts; else the live
/// tracker's onsets. Onsets weaker than a threshold are left out and two are never closer than
/// [`HIT_GAP`]. A seek or a stall, and the chart arriving, fire nothing.
#[derive(Debug)]
pub struct HitCounter {
    /// The song time of the last frame, if any.
    time: Option<f64>,
    /// Whether the last frame had a chart.
    chart: bool,
    /// Onsets seen of each chart stream so far.
    seen: [usize; 5],
    /// Whether the live onset was above [`LIVE_HIT_MIN`] in the last frame.
    above: bool,
    /// The song time of the last hit made from an onset.
    last_hit: f64,
    /// The envelope `hit`.
    envelope: f64,
}

impl Default for HitCounter {
    fn default() -> Self {
        Self {
            time: None,
            chart: false,
            seen: [0; 5],
            above: false,
            last_hit: f64::NEG_INFINITY,
            envelope: 0.0,
        }
    }
}

impl HitCounter {
    /// The hits since the last frame and the envelope now.
    pub fn tick(&mut self, input: &HitInput) -> (u32, f32) {
        let has_chart = input.chart.is_some();
        let steady = self.time.is_some_and(|before| {
            (0.0..=HIT_MAX_FRAME).contains(&(input.song_time - before)) && self.chart == has_chart
        });
        self.time = Some(input.song_time);
        self.chart = has_chart;
        if !steady {
            self.last_hit = f64::NEG_INFINITY;
        }
        self.envelope *= (-input.wall_dt / HIT_DECAY).exp();

        let (hits, strength) = if input.trusted {
            self.seen = input.chart.map_or([0; 5], |c| c.latest.map(|l| l.count));
            self.above = false;
            (input.beats, 1.0)
        } else if let Some(chart) = input.chart {
            self.above = false;
            self.chart_onsets(chart, steady)
        } else {
            self.seen = [0; 5];
            self.live_onset(input.rhythm, input.song_time, steady)
        };
        if hits > 0 {
            self.envelope = strength.clamp(0.0, 1.0);
        }
        (hits, self.envelope as f32)
    }

    /// The number and the greatest strength of the hits made of the new onsets of the streams
    /// that count now.
    fn chart_onsets(&mut self, chart: &ChartState, steady: bool) -> (u32, f64) {
        // The chart counts streams from 1 with 0 for none.
        let lead = match chart.lead as usize {
            0 => FALLBACK_STREAM,
            stream => stream - 1,
        };
        let mut fresh: Vec<LatestOnset> = [lead, PHRASE_STREAM]
            .into_iter()
            .filter(|&i| steady && chart.latest[i].count > self.seen[i])
            .map(|i| chart.latest[i])
            .collect();
        self.seen = chart.latest.map(|l| l.count);
        fresh.sort_by(|a, b| a.time.total_cmp(&b.time));
        let (mut hits, mut strength) = (0, 0.0f64);
        for onset in fresh {
            if onset.strength >= CHART_HIT_MIN && onset.time - self.last_hit >= HIT_GAP {
                self.last_hit = onset.time;
                hits += 1;
                strength = strength.max(onset.strength);
            }
        }
        (hits, strength)
    }

    /// A hit when the strongest live onset rises above [`LIVE_HIT_MIN`].
    fn live_onset(&mut self, rhythm: &Rhythm, song_time: f64, steady: bool) -> (u32, f64) {
        let onset = rhythm.kick.max(rhythm.snare).max(rhythm.hat);
        let rising = onset >= LIVE_HIT_MIN && !self.above;
        self.above = onset >= LIVE_HIT_MIN;
        if steady && rising && song_time - self.last_hit >= HIT_GAP {
            self.last_hit = song_time;
            return (1, onset);
        }
        (0, 0.0)
    }
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
    let trusted = beat_trusted(rhythm, chart.is_some());
    if !trusted {
        (m.beats, m.bars, m.bpm) = (0, 0, 0.0);
    }
    let input = HitInput {
        beats: m.beats,
        trusted,
        chart,
        rhythm,
        song_time: song.0,
        wall_dt: events.wall_dt(),
    };
    (m.hits, m.hit) = events.hits.tick(&input);
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
            ..Default::default()
        };
        let bytes = m.bytes();
        // 40 words, a multiple of 16 bytes already; the newest two are last.
        assert_eq!(bytes.len(), 160);
        assert_eq!(&bytes[0..4], &2.0f32.to_le_bytes());
        assert_eq!(&bytes[140..144], &1u32.to_le_bytes());
        assert_eq!(&bytes[152..156], &3u32.to_le_bytes());
        assert_eq!(&bytes[156..160], &0.5f32.to_le_bytes());
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

    /// A chart that has no beats and whose lead (stream 1, low) has had `count` onsets, the
    /// last at `time` with `strength`.
    fn beatless(count: usize, time: f64, strength: f64) -> ChartState {
        let mut chart = ChartState {
            lead: 1.0,
            ..Default::default()
        };
        chart.latest[0] = LatestOnset {
            count,
            time,
            strength,
        };
        chart
    }

    /// One frame of the counter at song time `t`, 16 ms of wall time after the last.
    fn frame(
        hits: &mut HitCounter,
        chart: Option<&ChartState>,
        rhythm: &Rhythm,
        t: f64,
    ) -> (u32, f32) {
        let trusted = beat_trusted(rhythm, chart.is_some());
        hits.tick(&HitInput {
            beats: 0,
            trusted,
            chart,
            rhythm,
            song_time: t,
            wall_dt: 0.016,
        })
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
    fn a_chart_without_beats_tells_none_though_the_tracker_is_sure() {
        let live = Rhythm {
            bpm: 120.0,
            beat: 40.0,
            confidence: 0.9,
            kick: 0.2,
            ..Default::default()
        };
        let chart = beatless(0, 0.0, 0.0);
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

    #[test]
    fn hits_fade_by_wall_time() {
        let mut hits = HitCounter::default();
        let rhythm = Rhythm::default();
        let input = |beats, wall_dt| HitInput {
            beats,
            trusted: true,
            chart: None,
            rhythm: &rhythm,
            song_time: 1.0,
            wall_dt,
        };
        assert_eq!(hits.tick(&input(1, 0.0)), (1, 1.0));
        let (n, hit) = hits.tick(&input(0, HIT_DECAY));
        assert_eq!(n, 0);
        assert!((f64::from(hit) - (-1.0f64).exp()).abs() < 1e-6);
    }

    #[test]
    fn several_beats_in_a_slow_frame_are_several_hits() {
        let mut hits = HitCounter::default();
        let rhythm = Rhythm::default();
        let (n, hit) = hits.tick(&HitInput {
            beats: 3,
            trusted: true,
            chart: None,
            rhythm: &rhythm,
            song_time: 5.0,
            wall_dt: 0.5,
        });
        assert_eq!((n, hit), (3, 1.0));
    }

    #[test]
    fn a_chart_without_beats_hits_on_the_leads_onsets() {
        let rhythm = Rhythm::default();
        let mut hits = HitCounter::default();
        assert_eq!(
            frame(&mut hits, Some(&beatless(0, 0.0, 0.0)), &rhythm, 1.0),
            (0, 0.0)
        );
        // A strong onset: one hit with its strength.
        let (n, hit) = frame(&mut hits, Some(&beatless(1, 1.02, 0.8)), &rhythm, 1.03);
        assert_eq!(n, 1);
        assert!((hit - 0.8).abs() < 0.01);
        // The same onset on the next frame is not new.
        assert_eq!(
            frame(&mut hits, Some(&beatless(1, 1.02, 0.8)), &rhythm, 1.05).0,
            0
        );
        // A weak one is left out; a strong one 0.1 s after the first is too close; one 0.3 s
        // after it counts.
        assert_eq!(
            frame(&mut hits, Some(&beatless(2, 1.1, 0.1)), &rhythm, 1.11).0,
            0
        );
        assert_eq!(
            frame(&mut hits, Some(&beatless(3, 1.12, 0.9)), &rhythm, 1.13).0,
            0
        );
        assert_eq!(
            frame(&mut hits, Some(&beatless(4, 1.4, 0.9)), &rhythm, 1.41).0,
            1
        );
    }

    #[test]
    fn without_a_lead_the_harmonic_stream_and_phrase_starts_hit() {
        let rhythm = Rhythm::default();
        let mut hits = HitCounter::default();
        let mut chart = ChartState::default();
        frame(&mut hits, Some(&chart), &rhythm, 1.0);
        // The low stream is not the lead: nothing.
        chart.latest[0] = LatestOnset {
            count: 1,
            time: 1.02,
            strength: 1.0,
        };
        assert_eq!(frame(&mut hits, Some(&chart), &rhythm, 1.03).0, 0);
        chart.latest[3] = LatestOnset {
            count: 1,
            time: 1.04,
            strength: 0.6,
        };
        assert_eq!(frame(&mut hits, Some(&chart), &rhythm, 1.05).0, 1);
        chart.latest[4] = LatestOnset {
            count: 1,
            time: 2.0,
            strength: 0.7,
        };
        assert_eq!(frame(&mut hits, Some(&chart), &rhythm, 2.01).0, 1);
    }

    #[test]
    fn a_slow_frame_with_two_onsets_apart_counts_each_stream() {
        let rhythm = Rhythm::default();
        let mut hits = HitCounter::default();
        let mut chart = beatless(0, 0.0, 0.0);
        frame(&mut hits, Some(&chart), &rhythm, 1.0);
        chart.latest[0] = LatestOnset {
            count: 2,
            time: 1.4,
            strength: 0.7,
        };
        chart.latest[4] = LatestOnset {
            count: 1,
            time: 1.1,
            strength: 0.9,
        };
        let (n, hit) = frame(&mut hits, Some(&chart), &rhythm, 1.5);
        assert_eq!(n, 2);
        assert!((hit - 0.9).abs() < 0.01);
    }

    #[test]
    fn seeking_or_the_chart_arriving_fires_no_onset() {
        let rhythm = Rhythm::default();
        let mut hits = HitCounter::default();
        frame(&mut hits, Some(&beatless(1, 1.0, 0.8)), &rhythm, 1.1);
        // A seek forwards over three onsets, then one backwards.
        assert_eq!(
            frame(&mut hits, Some(&beatless(4, 40.0, 0.8)), &rhythm, 40.1).0,
            0
        );
        assert_eq!(
            frame(&mut hits, Some(&beatless(2, 5.0, 0.8)), &rhythm, 5.1).0,
            0
        );
        // And the next onset after it counts again.
        assert_eq!(
            frame(&mut hits, Some(&beatless(3, 5.3, 0.8)), &rhythm, 5.35).0,
            1
        );
        // The chart arriving after the live tracker had the song.
        let mut hits = HitCounter::default();
        frame(&mut hits, None, &rhythm, 3.0);
        assert_eq!(
            frame(&mut hits, Some(&beatless(5, 3.0, 0.8)), &rhythm, 3.02).0,
            0
        );
    }

    #[test]
    fn an_unsure_tracker_hits_on_a_rising_onset_not_a_held_one() {
        let mut hits = HitCounter::default();
        let rhythm = |kick| Rhythm {
            kick,
            ..Default::default()
        };
        assert_eq!(frame(&mut hits, None, &rhythm(0.0), 1.0).0, 0);
        assert_eq!(frame(&mut hits, None, &rhythm(0.3), 1.02).0, 0);
        let (n, hit) = frame(&mut hits, None, &rhythm(0.9), 1.04);
        assert_eq!(n, 1);
        assert!((hit - 0.9).abs() < 0.01);
        // Still above, then fading, then a new one within the gap, then after it.
        assert_eq!(frame(&mut hits, None, &rhythm(0.8), 1.06).0, 0);
        assert_eq!(frame(&mut hits, None, &rhythm(0.1), 1.1).0, 0);
        assert_eq!(frame(&mut hits, None, &rhythm(0.9), 1.2).0, 0);
        assert_eq!(frame(&mut hits, None, &rhythm(0.1), 1.25).0, 0);
        assert_eq!(frame(&mut hits, None, &rhythm(0.9), 1.4).0, 1);
        // A seek is not a hit.
        frame(&mut hits, None, &rhythm(0.1), 1.45);
        assert_eq!(frame(&mut hits, None, &rhythm(0.9), 90.0).0, 0);
    }
}
