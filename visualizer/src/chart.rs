//! Charts: what an analysis of the whole song found, looked up at the song time of each frame.
//!
//! A chart is the JSON that the chart analyzer writes for one audio file (beats, bars, sections,
//! onset streams, the leading stream and loudness curves; every level is relative to the song).
//! Charts live in one folder, named after a hash of the file's path ([`chart_key`]). With a
//! chart the presets know what is coming: the next section, the next downbeat.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use log::{info, warn};
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

/// The streams, in the order of [`ChartState::events`]; a lead of `i + 1` is stream `i`.
const STREAMS: [&str; 5] = ["low", "mid", "high", "harmonic", "phrase"];
/// How fast an onset fades in [`ChartState::events`], per stream (seconds).
const EVENT_DECAY: [f64; 5] = [0.12, 0.12, 0.08, 0.15, 0.6];

/// A song time that differs from the shown one by more than this is a seek: jump to it.
const SEEK_THRESHOLD: f64 = 0.3;
/// Share of the gap to the reported song time closed each frame, which smooths the 100 ms
/// steps of the player's reports.
const CLOCK_PULL: f64 = 0.1;

/// The chart folder: `RUNE_CHART_DIR`, else `rune/charts` in the user's cache folder.
pub fn chart_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RUNE_CHART_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::BaseDirs::new().map(|dirs| dirs.cache_dir().join("rune").join("charts"))
}

/// File name of the chart of the audio file at `path`: FNV-1a (64 bit) of the path in NFC,
/// so that the chart analyzer and the player agree however the file system spells it.
pub fn chart_key(path: &Path) -> String {
    let text: String = path.to_string_lossy().nfc().collect();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}.json")
}

#[derive(Deserialize)]
struct RawChart {
    beats: Vec<RawBeat>,
    meter: RawMeter,
    sections: Vec<RawSection>,
    streams: std::collections::HashMap<String, RawStream>,
    lead: Vec<RawLead>,
    curves: RawCurves,
}

#[derive(Deserialize)]
struct RawBeat {
    t: f64,
    bar: i64,
    pos: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct RawMeter {
    beats_per_bar: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct RawSection {
    start: f64,
    end: f64,
    label: String,
    energy: f64,
    repeat: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct RawStream {
    events: Vec<(f64, f64)>,
}

#[derive(Deserialize)]
struct RawLead {
    start: f64,
    end: f64,
    stream: Option<String>,
}

#[derive(Deserialize)]
struct RawCurves {
    hop: f64,
    loudness: Vec<f64>,
    brightness: Vec<f64>,
    percussive: Vec<f64>,
    beat_conf: Vec<f64>,
}

struct Section {
    start: f64,
    end: f64,
    /// 0 for the label "A", 1 for "B" and so on.
    label: f64,
    energy: f64,
    repeat: f64,
    confidence: f64,
}

struct Lead {
    start: f64,
    end: f64,
    /// 0 for none, else 1 + the index of the stream in [`STREAMS`].
    stream: f64,
}

pub struct Chart {
    beats: Vec<RawBeat>,
    beats_per_bar: f64,
    meter_confidence: f64,
    sections: Vec<Section>,
    /// `(time, strength)` per stream, in the order of [`STREAMS`].
    streams: [Vec<(f64, f64)>; 5],
    lead: Vec<Lead>,
    curves: RawCurves,
}

/// The chart at one moment, for the presets.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChartState {
    /// Tempo over the beats around now; 0 without beats.
    pub bpm: f64,
    /// Beats since the first one, with the phase as the fraction. Grows linearly between beats
    /// and is extrapolated before the first and after the last.
    pub beat: f64,
    pub beat_confidence: f64,
    /// Bars since the first, with the phase within the bar as the fraction.
    pub bar: f64,
    pub beats_per_bar: f64,
    pub meter_confidence: f64,
    /// Index of the section, its label (0 for "A") and how many earlier sections had the label.
    pub section: f64,
    pub section_label: f64,
    pub section_repeat: f64,
    pub section_energy: f64,
    /// Label and energy of the next section (those of this one in the last section).
    pub next_label: f64,
    pub next_energy: f64,
    /// Seconds since the section started and until it ends.
    pub section_time: f64,
    pub section_left: f64,
    pub section_confidence: f64,
    /// Onset envelopes of the streams (low, mid, high, harmonic, phrase): the latest onset's
    /// strength, fading.
    pub events: [f64; 5],
    /// The leading stream (0 none, 1 low, 2 mid, 3 high, 4 harmonic) and its envelope.
    pub lead: f64,
    pub lead_event: f64,
    pub loudness: f64,
    pub brightness: f64,
    pub percussive: f64,
    /// How clear the pulse is around now.
    pub pulse: f64,
}

impl Chart {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("Failed to parse {}", path.display()))
    }

    pub fn parse(json: &[u8]) -> Result<Self> {
        let raw: RawChart = serde_json::from_slice(json)?;
        let streams = STREAMS.map(|name| {
            raw.streams
                .get(name)
                .map(|s| s.events.clone())
                .unwrap_or_default()
        });
        let sections = raw
            .sections
            .into_iter()
            .map(|s| Section {
                start: s.start,
                end: s.end,
                label: s
                    .label
                    .bytes()
                    .next()
                    .map_or(0.0, |b| f64::from(b.saturating_sub(b'A'))),
                energy: s.energy,
                repeat: f64::from(s.repeat),
                confidence: s.confidence,
            })
            .collect();
        let lead = raw
            .lead
            .into_iter()
            .map(|l| Lead {
                start: l.start,
                end: l.end,
                stream: l
                    .stream
                    .and_then(|name| STREAMS.iter().position(|s| *s == name))
                    .map_or(0.0, |i| (i + 1) as f64),
            })
            .collect();
        Ok(Self {
            beats: raw.beats,
            beats_per_bar: f64::from(raw.meter.beats_per_bar.max(1)),
            meter_confidence: raw.meter.confidence,
            sections,
            streams,
            lead,
            curves: raw.curves,
        })
    }

    /// The chart at song time `t` (seconds).
    pub fn at(&self, t: f64) -> ChartState {
        let mut state = ChartState {
            beats_per_bar: self.beats_per_bar,
            meter_confidence: self.meter_confidence,
            ..ChartState::default()
        };
        self.beats_at(t, &mut state);
        self.section_at(t, &mut state);
        for (i, events) in self.streams.iter().enumerate() {
            state.events[i] = envelope(events, t, EVENT_DECAY[i]);
        }
        if let Some(lead) = span_at(&self.lead, t, |l| (l.start, l.end)) {
            state.lead = lead.stream;
            if lead.stream > 0.0 {
                state.lead_event = state.events[lead.stream as usize - 1];
            }
        }
        let c = &self.curves;
        state.loudness = curve(&c.loudness, c.hop, t);
        state.brightness = curve(&c.brightness, c.hop, t);
        state.percussive = curve(&c.percussive, c.hop, t);
        state.pulse = curve(&c.beat_conf, c.hop, t);
        state
    }

    fn beats_at(&self, t: f64, state: &mut ChartState) {
        let beats = &self.beats;
        if beats.len() < 2 {
            return;
        }
        // The beat at or before `t`, or the first one; the next one bounds the phase.
        let i = beats
            .partition_point(|b| b.t <= t)
            .saturating_sub(1)
            .min(beats.len() - 2);
        let (a, b) = (&beats[i], &beats[i + 1]);
        let phase = (t - a.t) / (b.t - a.t).max(1e-3);
        state.beat = i as f64 + phase;
        // Tempo over up to 8 beats around now.
        let (lo, hi) = (i.saturating_sub(4), (i + 4).min(beats.len() - 1));
        state.bpm = 60.0 * (hi - lo) as f64 / (beats[hi].t - beats[lo].t).max(1e-3);
        state.beat_confidence = a.confidence;
        state.bar = a.bar as f64 + (f64::from(a.pos) + phase) / self.beats_per_bar;
    }

    fn section_at(&self, t: f64, state: &mut ChartState) {
        let sections = &self.sections;
        if sections.is_empty() {
            return;
        }
        let j = sections.partition_point(|s| s.start <= t).saturating_sub(1);
        let s = &sections[j];
        state.section = j as f64;
        state.section_label = s.label;
        state.section_repeat = s.repeat;
        state.section_energy = s.energy;
        let next = sections.get(j + 1).unwrap_or(s);
        state.next_label = next.label;
        state.next_energy = next.energy;
        state.section_time = (t - s.start).max(0.0);
        state.section_left = (s.end - t).max(0.0);
        state.section_confidence = s.confidence;
    }
}

/// The span that contains `t`, if any (spans are sorted and do not overlap).
fn span_at<T>(spans: &[T], t: f64, range: impl Fn(&T) -> (f64, f64)) -> Option<&T> {
    let i = spans.partition_point(|s| range(s).0 <= t).checked_sub(1)?;
    let span = &spans[i];
    (t < range(span).1).then_some(span)
}

/// The strongest of the recent onsets, each fading by `decay` seconds.
fn envelope(events: &[(f64, f64)], t: f64, decay: f64) -> f64 {
    let end = events.partition_point(|e| e.0 <= t);
    events[..end]
        .iter()
        .rev()
        .take_while(|e| t - e.0 < 6.0 * decay)
        .map(|&(at, strength)| strength * (-(t - at) / decay).exp())
        .fold(0.0, f64::max)
}

/// A curve sampled every `hop` seconds, linearly between samples.
fn curve(values: &[f64], hop: f64, t: f64) -> f64 {
    if values.is_empty() || hop <= 0.0 {
        return 0.0;
    }
    let x = (t / hop).max(0.0);
    let i = (x as usize).min(values.len() - 1);
    let j = (i + 1).min(values.len() - 1);
    let f = (x - i as f64).clamp(0.0, 1.0);
    values[i] * (1.0 - f) + values[j] * f
}

/// Where the player is, as it last said.
#[derive(Debug, Clone, Default)]
pub struct Playback {
    pub path: Option<PathBuf>,
    pub position: Duration,
    pub playing: bool,
    /// When the position was reported.
    pub at: Option<Instant>,
}

/// The song time shown, following the player's reports smoothly.
#[derive(Default)]
pub struct SongClock {
    shown: Option<f64>,
    last: Option<Instant>,
}

impl SongClock {
    /// The song time at `now`. Within [`SEEK_THRESHOLD`] of the reported time it runs at the
    /// wall clock's pace and closes [`CLOCK_PULL`] of the gap per frame; beyond it (a seek, a
    /// new track) it jumps.
    pub fn tick(&mut self, playback: &Playback, now: Instant) -> f64 {
        let Some(at) = playback.at else {
            return 0.0;
        };
        let mut reported = playback.position.as_secs_f64();
        if playback.playing {
            reported += now.saturating_duration_since(at).as_secs_f64();
        }
        let shown = match (self.shown, self.last) {
            (Some(shown), Some(last)) if (reported - shown).abs() <= SEEK_THRESHOLD => {
                let run = if playback.playing {
                    now.saturating_duration_since(last).as_secs_f64()
                } else {
                    0.0
                };
                let ahead = shown + run;
                ahead + (reported - ahead) * CLOCK_PULL
            }
            _ => reported,
        };
        self.shown = Some(shown);
        self.last = Some(now);
        shown
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The chart of the file that plays, loaded off the render thread, and the song time.
pub struct ChartFollower {
    dir: Option<PathBuf>,
    path: Option<PathBuf>,
    chart: Option<Chart>,
    loading: Option<Receiver<Option<Chart>>>,
    clock: SongClock,
    /// The song time of the last tick.
    time: f64,
}

impl ChartFollower {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            path: None,
            chart: None,
            loading: None,
            clock: SongClock::default(),
            time: 0.0,
        }
    }

    /// The chart at the song time of `now`, if the playing file has one (and it has loaded).
    pub fn tick(&mut self, playback: &Playback, now: Instant) -> Option<ChartState> {
        if playback.path != self.path {
            self.path = playback.path.clone();
            self.chart = None;
            self.clock.reset();
            self.loading = None;
            if let (Some(dir), Some(path)) = (&self.dir, &self.path) {
                let file = dir.join(chart_key(path));
                let path = path.clone();
                let (sender, receiver) = mpsc::channel();
                thread::spawn(move || {
                    let chart = if !file.exists() {
                        info!("No chart for {} ({})", path.display(), file.display());
                        None
                    } else {
                        Chart::load(&file)
                            .inspect(|_| info!("Chart for {}", path.display()))
                            .inspect_err(|e| warn!("{e:#}"))
                            .ok()
                    };
                    let _ = sender.send(chart);
                });
                self.loading = Some(receiver);
            }
        }
        if let Some(receiver) = &self.loading {
            match receiver.try_recv() {
                Ok(chart) => {
                    self.chart = chart;
                    self.loading = None;
                }
                Err(TryRecvError::Disconnected) => self.loading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let t = self.clock.tick(playback, now);
        self.time = t;
        self.chart.as_ref().map(|chart| chart.at(t))
    }

    /// The song time of the last [`Self::tick`], with or without a chart.
    pub fn song_time(&self) -> f64 {
        self.time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = r#"{
        "version": 1, "title": "t", "audio": "/a.wav", "duration": 20,
        "tempo": {"bpm": 120, "confidence": 0.9},
        "beats": [
            {"t": 1.0, "bar": 0, "pos": 0, "strength": 1, "confidence": 0.8},
            {"t": 1.5, "bar": 0, "pos": 1, "strength": 1, "confidence": 0.8},
            {"t": 2.0, "bar": 0, "pos": 2, "strength": 1, "confidence": 0.8},
            {"t": 2.5, "bar": 0, "pos": 3, "strength": 1, "confidence": 0.8},
            {"t": 3.0, "bar": 1, "pos": 0, "strength": 1, "confidence": 0.6}
        ],
        "meter": {"beats_per_bar": 4, "confidence": 0.7},
        "sections": [
            {"start": 0, "end": 8, "label": "A", "energy": 0.3, "repeat": 0, "confidence": 1},
            {"start": 8, "end": 20, "label": "B", "energy": 0.9, "repeat": 0, "confidence": 0.6}
        ],
        "streams": {
            "low": {"name": "x", "events": [[1.0, 0.8], [2.0, 0.5]]},
            "mid": {"name": "x", "events": []},
            "high": {"name": "x", "events": []},
            "harmonic": {"name": "x", "events": [[9.0, 1.0]]},
            "phrase": {"name": "x", "events": []}
        },
        "lead": [{"start": 0, "end": 8, "stream": "low"}, {"start": 8, "end": 20, "stream": null}],
        "curves": {"hop": 0.5, "loudness": [0, 1, 0.5], "brightness": [0], "percussive": [0],
                   "beat_conf": [0]}
    }"#;

    fn chart() -> Chart {
        Chart::parse(CHART.as_bytes()).unwrap()
    }

    #[test]
    fn beats_and_bars_follow_the_chart() {
        let s = chart().at(1.75);
        assert!((s.beat - 1.5).abs() < 1e-9);
        assert!((s.bar - 1.5 / 4.0).abs() < 1e-9);
        assert!((s.bpm - 120.0).abs() < 1e-9);
        assert_eq!(s.beat_confidence, 0.8);
        // The downbeat of bar 1 is a whole bar.
        assert!((chart().at(3.0).bar - 1.0).abs() < 1e-9);
        // Before the first beat the grid is extrapolated.
        assert!((chart().at(0.5).beat + 1.0).abs() < 1e-9);
    }

    #[test]
    fn sections_tell_what_is_coming() {
        let s = chart().at(6.5);
        assert_eq!(
            (s.section, s.section_label, s.section_energy),
            (0.0, 0.0, 0.3)
        );
        assert_eq!((s.next_label, s.next_energy), (1.0, 0.9));
        assert!((s.section_left - 1.5).abs() < 1e-9 && (s.section_time - 6.5).abs() < 1e-9);
        let s = chart().at(10.0);
        assert_eq!((s.section, s.section_label, s.next_energy), (1.0, 1.0, 0.9));
    }

    #[test]
    fn onsets_fade_and_the_lead_has_one() {
        let at = |t| chart().at(t);
        assert!((at(1.0).events[0] - 0.8).abs() < 1e-9);
        assert!((at(1.12).events[0] - 0.8 * (-1.0f64).exp()).abs() < 1e-9);
        assert_eq!(at(0.9).events[0], 0.0);
        assert_eq!(at(1.0).lead, 1.0);
        assert_eq!(at(1.0).lead_event, at(1.0).events[0]);
        // No lead in the second section, though the harmonic stream has an onset there.
        assert_eq!((at(9.0).lead, at(9.0).lead_event), (0.0, 0.0));
        assert!((at(9.0).events[3] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn curves_are_interpolated() {
        assert!((chart().at(0.25).loudness - 0.5).abs() < 1e-9);
        assert!((chart().at(0.75).loudness - 0.75).abs() < 1e-9);
        assert_eq!(chart().at(99.0).loudness, 0.5);
    }

    #[test]
    fn the_key_matches_the_analyzer() {
        // From the Python prototype's chart_key for the same path, in NFC and in NFD.
        let nfc = "/music/Café/Été/09 Façade.flac";
        let nfd: String = nfc.nfd().collect();
        assert_eq!(chart_key(Path::new(nfc)), "f7ca7b5cb168a3be.json");
        assert_eq!(chart_key(Path::new(&nfd)), "f7ca7b5cb168a3be.json");
    }

    #[test]
    fn the_song_clock_smooths_reports_and_jumps_on_seeks() {
        let start = Instant::now();
        let mut clock = SongClock::default();
        let mut playback = Playback {
            path: None,
            position: Duration::from_secs(10),
            playing: true,
            at: Some(start),
        };
        assert_eq!(clock.tick(&playback, start), 10.0);
        // A report 40 ms late moves the clock only part of the way.
        let frame = start + Duration::from_millis(16);
        playback.position = Duration::from_millis(9_976);
        let t = clock.tick(&playback, frame);
        assert!(t > 9.976 + 0.016 - 1e-9 && t < 10.016, "{t}");
        // A seek jumps.
        playback.position = Duration::from_secs(60);
        playback.at = Some(frame);
        let t = clock.tick(&playback, frame + Duration::from_millis(16));
        assert!((t - 60.016).abs() < 1e-9, "{t}");
        // Paused, the clock holds.
        playback.playing = false;
        playback.position = Duration::from_secs(61);
        let t1 = clock.tick(&playback, frame + Duration::from_millis(40));
        let t2 = clock.tick(&playback, frame + Duration::from_millis(400));
        assert_eq!(t1, t2);
    }
}
