//! What a scene hears: `struct Music`, filled from the song's chart when it has one and from the
//! live beat tracker and spectrum otherwise, with the events since the last frame counted so
//! that a slow frame never loses a beat.

use crate::chart::ChartState;
use crate::rhythm::Rhythm;

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
}

impl MusicValues {
    /// Name and value of every field, in the order of `struct Music`. New fields go at the end.
    pub fn fields(&self) -> [(&'static str, Word); 36] {
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

/// Counts how many whole beats, bars and sections went by between frames. A jump backwards (a
/// seek, a new song) counts nothing and starts over from there; so does a jump of more than
/// [`EventCounter::MAX_JUMP`] (a seek forwards), which would otherwise fire a burst.
#[derive(Debug, Default)]
pub struct EventCounter {
    last: Option<(f64, f64, f64)>,
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
}

/// `struct Music` for this frame. `song` is the song time and whether it plays.
pub fn music(
    rhythm: &Rhythm,
    chart: Option<&ChartState>,
    levels: Levels,
    song: (f64, bool),
    events: &mut EventCounter,
) -> MusicValues {
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
    m
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ..Default::default()
        };
        let bytes = m.bytes();
        assert_eq!(bytes.len(), 144);
        assert_eq!(&bytes[0..4], &2.0f32.to_le_bytes());
        assert_eq!(&bytes[140..144], &1u32.to_le_bytes());
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
}
