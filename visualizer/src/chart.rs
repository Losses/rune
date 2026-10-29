//! Charts: what an analysis of the whole song found, looked up at the song time of each frame.
//!
//! A chart is what the chart analyzer (`chart_analyzer`) finds in one audio file (beats, bars,
//! sections, onset streams, the leading stream and loudness curves; every level is relative to
//! the song). The visualizer analyses the file that plays itself, in the background
//! ([`ChartFollower`]); until the chart is ready the live beat tracker drives the scenes. With a
//! chart the scenes know what is coming: the next section, the next downbeat.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use chart_analyzer::chart::{Beat, Curves};
use chart_analyzer::decode::Cancelled;
use fsio::FsIo;
use log::{info, warn};

use crate::rhythm::Rhythm;

/// The streams, in the order of [`ChartState::events`]; a lead of `i + 1` is stream `i`.
const STREAMS: [&str; 5] = ["low", "mid", "high", "harmonic", "phrase"];
/// How fast an onset fades in [`ChartState::events`], per stream (seconds).
const EVENT_DECAY: [f64; 5] = [0.12, 0.12, 0.08, 0.15, 0.6];

/// A song time that differs from the shown one by more than this is a seek: jump to it.
const SEEK_THRESHOLD: f64 = 0.3;
/// Share of the gap to the reported song time closed each frame, which smooths the 100 ms
/// steps of the player's reports.
const CLOCK_PULL: f64 = 0.1;
/// The loudness (0 to 1, against the song's own range) under which the chart's music counts as
/// silent, as the analyzer's streams take it.
const SILENT_LOUDNESS: f64 = 0.05;
/// How long (seconds) the loudness must have stayed under [`SILENT_LOUDNESS`] for the song to
/// count as silent: a short gap or a staccato rest keeps the beat.
const SILENT_SPAN: f64 = 1.0;

struct Section {
    start: f64,
    end: f64,
    /// 0 for the label "A", 1 for "B" and so on.
    label: f64,
    energy: f64,
    repeat: f64,
    confidence: f64,
    /// Where the section's energy ranks in the song, 0 to 1: the share of the song's time in
    /// quieter sections (half of that in sections as loud).
    rank: f64,
}

struct Lead {
    start: f64,
    end: f64,
    /// 0 for none, else 1 + the index of the stream in [`STREAMS`].
    stream: f64,
}

/// How much of a section's level is its rank in the song rather than its loudness: half, so a song
/// loud throughout has more high sections than a quiet one, and every song has some.
const LEVEL_RANK: f64 = 0.5;

/// How high a section is in the song, 0 to 1: half its energy, half its rank (`MusicValues::level`
/// is this for the section playing, and the director's tiers cut it by `director.cadence`).
pub fn section_level(energy: f64, rank: f64) -> f64 {
    ((1.0 - LEVEL_RANK) * energy + LEVEL_RANK * rank).clamp(0.0, 1.0)
}

/// One section of the song as the director learns it ahead of time (`ChartState::song`): where it
/// lies and how high it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SongSection {
    /// Song seconds.
    pub start: f32,
    pub end: f32,
    /// How high the section is in the song, as [`section_level`] makes it.
    pub level: f32,
}

/// The section of the song that is the last chance of something the song is promised (a shot or a
/// look that says `at_least_once`): the last section that `suits` it and that `fits` (a section
/// by its place, for the one that asks to say whether the change can be made there), or, where no
/// section does both, the song's highest section that fits (by level, the last of equals). None
/// where no section fits. Shots and looks share it, so the two promises read the song the same
/// way.
pub fn last_chance(
    song: &[SongSection],
    suits: impl Fn(&SongSection) -> bool,
    fits: impl Fn(usize) -> bool,
) -> Option<usize> {
    (0..song.len())
        .rev()
        .find(|&i| suits(&song[i]) && fits(i))
        .or_else(|| {
            (0..song.len())
                .filter(|&i| fits(i))
                .max_by(|&a, &b| song[a].level.total_cmp(&song[b].level).then(a.cmp(&b)))
        })
}

pub struct Chart {
    beats: Vec<Beat>,
    beats_per_bar: f64,
    meter_confidence: f64,
    sections: Vec<Section>,
    /// The sections as the director learns them ahead (`ChartState::song`), made once.
    song: Arc<[SongSection]>,
    /// `(time, strength)` per stream, in the order of [`STREAMS`].
    streams: [Vec<(f64, f64)>; 5],
    lead: Vec<Lead>,
    curves: Curves,
    song_feel: f64,
}

/// The latest onset of a stream at one moment: how many there have been and the last one.
///
/// Comparing the counts of two frames tells exactly how many onsets lay between them, which the
/// fading envelope in [`ChartState::events`] cannot (two quick onsets blur into one).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LatestOnset {
    /// Onsets at or before now; 0 means none yet (then `time` and `strength` are 0 too).
    pub count: usize,
    /// Song time (seconds) and strength (0 to about 1) of the latest one.
    pub time: f64,
    pub strength: f64,
}

/// The chart at one moment, for the scenes.
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// Where this section's energy ranks in the song, 0 to 1 (the share of the song's time in
    /// quieter sections): how high it is against the rest of the song.
    pub section_rank: f64,
    /// Label and energy of the next section (those of this one in the last section).
    pub next_label: f64,
    pub next_energy: f64,
    /// The biggest rise of energy from one section to the next among the boundaries after the next
    /// section start (0 where there is none): a section start that lifts by less than this is not
    /// the song's last lift (see `Director::set_song_ahead`).
    pub later_rise: f64,
    /// Seconds since the section started and until it ends.
    pub section_time: f64,
    pub section_left: f64,
    pub section_confidence: f64,
    /// Every section of the song with its level, shared by all frames of the chart (cloning it is
    /// a counter), for the director to tell which later section suits a shot (see
    /// `Director::set_song`). None where the chart has no sections.
    pub song: Option<Arc<[SongSection]>>,
    /// Onset envelopes of the streams (low, mid, high, harmonic, phrase): the latest onset's
    /// strength, fading.
    pub events: [f64; 5],
    /// The latest onset of each stream, in the order of [`ChartState::events`].
    pub latest: [LatestOnset; 5],
    /// The leading stream (0 none, 1 low, 2 mid, 3 high, 4 harmonic) and its envelope.
    pub lead: f64,
    pub lead_event: f64,
    pub loudness: f64,
    pub brightness: f64,
    pub percussive: f64,
    /// How clear the pulse is around now.
    pub pulse: f64,
    /// How fast it sounds now and over the whole song: the centroid of the modulation spectrum
    /// of the spectrum's change, about 3 Hz for a ballad to 7 for drum and bass; 0 if the chart
    /// predates it.
    pub feel: f64,
    pub song_feel: f64,
    /// Nothing has been heard for [`SILENT_SPAN`]: a song's silent end or a gap of a second or more. The chart's
    /// beats go on through it at the last tempo, but there is no beat there to follow, so
    /// scenes are told of none (see [`crate::scene::inputs::music`]).
    pub silent: bool,
}

impl Chart {
    /// The chart of an analysis, as the scenes look it up.
    pub fn from_analysis(chart: chart_analyzer::Chart) -> Self {
        let streams = chart.streams;
        let streams = [
            streams.low,
            streams.mid,
            streams.high,
            streams.harmonic,
            streams.phrase,
        ]
        .map(|s| {
            s.events
                .into_iter()
                .map(|[t, strength]| (t, strength))
                .collect()
        });
        let sections = chart
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
                rank: 0.0,
            })
            .collect();
        let sections = ranked(sections);
        let song = sections
            .iter()
            .map(|s| SongSection {
                start: s.start as f32,
                end: s.end as f32,
                level: section_level(s.energy, s.rank) as f32,
            })
            .collect();
        let lead = chart
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
        Self {
            beats: chart.beats,
            beats_per_bar: f64::from(chart.meter.beats_per_bar.max(1)),
            meter_confidence: chart.meter.confidence,
            sections,
            song,
            streams,
            lead,
            curves: chart.curves,
            song_feel: chart.tempo.feel,
        }
    }

    /// The sections of the song, for a readout.
    pub fn sections(&self) -> Vec<crate::InspectSection> {
        self.sections
            .iter()
            .map(|s| crate::InspectSection {
                start: s.start,
                end: s.end,
                label: crate::inspect::label_name(s.label),
                energy: s.energy as f32,
            })
            .collect()
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
            state.latest[i] = latest_onset(events, t);
        }
        if let Some(lead) = span_at(&self.lead, t, |l| (l.start, l.end)) {
            state.lead = lead.stream;
            if lead.stream > 0.0 {
                state.lead_event = state.events[lead.stream as usize - 1];
            }
        }
        let c = &self.curves;
        state.loudness = curve(&c.loudness, c.hop, t);
        state.silent = silent(&c.loudness, c.hop, t);
        state.brightness = curve(&c.brightness, c.hop, t);
        state.percussive = curve(&c.percussive, c.hop, t);
        state.pulse = curve(&c.beat_conf, c.hop, t);
        if !c.feel.is_empty() {
            state.feel = curve(&c.feel, c.hop, t);
            state.song_feel = self.song_feel;
        }
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
        state.bar = a.bar as f64 + (a.pos as f64 + phase) / self.beats_per_bar;
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
        state.section_rank = s.rank;
        let next = sections.get(j + 1).unwrap_or(s);
        state.next_label = next.label;
        state.next_energy = next.energy;
        state.later_rise = sections[(j + 1).min(sections.len())..]
            .windows(2)
            .map(|pair| pair[1].energy - pair[0].energy)
            .fold(0.0, f64::max);
        state.section_time = (t - s.start).max(0.0);
        state.section_left = (s.end - t).max(0.0);
        state.section_confidence = s.confidence;
        state.song = Some(Arc::clone(&self.song));
    }
}

/// The sections with their ranks (see `Section::rank`), by time: a section is as high as the
/// share of the song's time spent in quieter sections, plus half that in sections as loud.
fn ranked(mut sections: Vec<Section>) -> Vec<Section> {
    let total: f64 = sections.iter().map(|s| (s.end - s.start).max(0.0)).sum();
    if total <= 0.0 {
        return sections;
    }
    let spans: Vec<(f64, f64)> = sections
        .iter()
        .map(|s| (s.energy, (s.end - s.start).max(0.0)))
        .collect();
    for s in &mut sections {
        let below: f64 = spans
            .iter()
            .filter(|(e, _)| *e < s.energy)
            .map(|(_, d)| d)
            .sum();
        let same: f64 = spans
            .iter()
            .filter(|(e, _)| *e == s.energy)
            .map(|(_, d)| d)
            .sum();
        s.rank = (below + 0.5 * same) / total;
    }
    sections
}

/// The rhythm the scenes get: the beats come from the song's chart and from nothing else.
///
/// A chart with beats gives the tempo, beat and confidence. A chart without beats (the whole
/// song was heard and has no steady pulse) is the verdict: no tempo, no beat, no confidence.
/// Without a chart (being made, failed, or an online track) there is no beat either: the live
/// tracker's guesses are often off by half or double, and scenes move on what is heard instead
/// (its onsets as hits, its levels, how fast the music feels). Its levels and onsets always
/// stay.
pub fn trusted_rhythm(live: Rhythm, chart: Option<&ChartState>) -> Rhythm {
    let Some(chart) = chart else {
        return Rhythm {
            bpm: 0.0,
            beat: 0.0,
            confidence: 0.0,
            ..live
        };
    };
    if chart.bpm > 0.0 {
        Rhythm {
            bpm: chart.bpm,
            beat: chart.beat,
            confidence: chart.beat_confidence,
            ..live
        }
    } else {
        Rhythm {
            bpm: 0.0,
            beat: 0.0,
            confidence: 0.0,
            ..live
        }
    }
}

/// The span that contains `t`, if any (spans are sorted and do not overlap).
fn span_at<T>(spans: &[T], t: f64, range: impl Fn(&T) -> (f64, f64)) -> Option<&T> {
    let i = spans.partition_point(|s| range(s).0 <= t).checked_sub(1)?;
    let span = &spans[i];
    (t < range(span).1).then_some(span)
}

/// The onsets at or before `t` counted, and the last of them.
fn latest_onset(events: &[(f64, f64)], t: f64) -> LatestOnset {
    let count = events.partition_point(|e| e.0 <= t);
    match count.checked_sub(1) {
        Some(i) => LatestOnset {
            count,
            time: events[i].0,
            strength: events[i].1,
        },
        None => LatestOnset::default(),
    }
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

/// Whether the loudness curve stayed under [`SILENT_LOUDNESS`] over the last [`SILENT_SPAN`]
/// before `t`; never without a curve, as in an older chart.
fn silent(values: &[f64], hop: f64, t: f64) -> bool {
    if values.is_empty() || hop <= 0.0 {
        return false;
    }
    let last = values.len() - 1;
    let from = (((t - SILENT_SPAN) / hop).max(0.0) as usize).min(last);
    let to = ((t / hop).max(0.0).ceil() as usize).min(last);
    values[from..=to].iter().all(|&v| v < SILENT_LOUDNESS)
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

/// The chart of the file that plays, made off the render thread, and the song time.
///
/// On a new file it starts analysing it in a thread of its own and drops the analysis of the
/// file before, which stops at its next decoded packet; a burst of track changes thus leaves
/// one analysis running for long. Without a chart the caller falls back on the live tracker.
pub struct ChartFollower {
    /// Where files are opened from; without it nothing is analysed.
    fsio: Option<Arc<FsIo>>,
    path: Option<PathBuf>,
    chart: Option<Chart>,
    loading: Option<Receiver<Option<Chart>>>,
    /// Stops the running analysis, if any.
    cancel: Option<Arc<AtomicBool>>,
    clock: SongClock,
    /// The song time of the last tick.
    time: f64,
}

impl ChartFollower {
    pub fn new(fsio: Option<Arc<FsIo>>) -> Self {
        Self {
            fsio,
            path: None,
            chart: None,
            loading: None,
            cancel: None,
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
            self.cancel_analysis();
            if let (Some(fsio), Some(path)) = (&self.fsio, &self.path)
                && !path.as_os_str().is_empty()
            {
                self.loading = Some(self.analyse(Arc::clone(fsio), path.clone()));
            }
        }
        let mut arrived = false;
        if let Some(receiver) = &self.loading {
            match receiver.try_recv() {
                Ok(chart) => {
                    arrived = chart.is_some();
                    self.chart = chart;
                    self.loading = None;
                }
                Err(TryRecvError::Disconnected) => self.loading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let t = self.clock.tick(playback, now);
        self.time = t;
        if arrived && let Some(path) = &self.path {
            // Until now the live tracker had the beats.
            info!("The chart of {} took over at {t:.1} s", path.display());
        }
        self.chart.as_ref().map(|chart| chart.at(t))
    }

    /// The sections of the chart, once there is one (else none).
    pub fn sections(&self) -> Vec<crate::InspectSection> {
        self.chart.as_ref().map(Chart::sections).unwrap_or_default()
    }

    /// The song time of the last [`Self::tick`], with or without a chart.
    pub fn song_time(&self) -> f64 {
        self.time
    }

    /// Stops the running analysis and forgets it.
    fn cancel_analysis(&mut self) {
        self.loading = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Starts analysing `path`; the chart (none if it failed) comes over the receiver.
    fn analyse(&mut self, fsio: Arc<FsIo>, path: PathBuf) -> Receiver<Option<Chart>> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(Arc::clone(&cancel));
        let (sender, receiver) = mpsc::channel();
        let job = move || {
            let start = Instant::now();
            let chart = match chart_analyzer::analyze_file(&fsio, &path, &cancel) {
                Ok(chart) => {
                    info!("Analysed {} in {:.2?}", path.display(), start.elapsed());
                    Some(Chart::from_analysis(chart))
                }
                Err(e) if e.downcast_ref::<Cancelled>().is_some() => {
                    info!("Analysis of {} cancelled", path.display());
                    None
                }
                Err(e) => {
                    warn!("Failed to analyse {}: {e:#}", path.display());
                    None
                }
            };
            let _ = sender.send(chart);
        };
        if let Err(e) = thread::Builder::new()
            .name("chart analyzer".into())
            .spawn(job)
        {
            warn!("Failed to start the chart analyzer: {e}");
        }
        receiver
    }
}

impl Drop for ChartFollower {
    /// The visualizer stops: so does the analysis.
    fn drop(&mut self) {
        info!("The visualizer stopped following the song");
        self.cancel_analysis();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const CHART: &str = r#"{
        "version": 1, "title": "t", "audio": "/a.wav", "duration": 20,
        "analysis_seconds": 1, "analyzer": "test",
        "tempo": {"bpm": 120, "confidence": 0.9, "feel": 5.0},
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
                   "beat_conf": [0], "feel": [4, 6]}
    }"#;

    fn chart() -> Chart {
        Chart::from_analysis(serde_json::from_str(CHART).unwrap())
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
    fn a_chart_without_beats_silences_the_trackers_beat() {
        let live = Rhythm {
            bpm: 120.0,
            beat: 30.0,
            confidence: 0.9,
            kick: 0.4,
            ..Default::default()
        };
        // Without a chart there is no beat, whatever the tracker thinks, but its levels and
        // onsets stay.
        let alone = trusted_rhythm(live, None);
        assert_eq!((alone.bpm, alone.beat, alone.confidence), (0.0, 0.0, 0.0));
        assert_eq!(alone.kick, live.kick);
        let with = ChartState {
            bpm: 100.0,
            beat: 5.5,
            beat_confidence: 0.7,
            ..Default::default()
        };
        let r = trusted_rhythm(live, Some(&with));
        assert_eq!(
            (r.bpm, r.beat, r.confidence, r.kick),
            (100.0, 5.5, 0.7, 0.4)
        );
        let r = trusted_rhythm(live, Some(&ChartState::default()));
        assert_eq!((r.bpm, r.beat, r.confidence, r.kick), (0.0, 0.0, 0.0, 0.4));
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
    fn the_biggest_rise_after_the_coming_section_start_is_known() {
        // Energies 0.3, 0.9, 0.5, 0.8: the boundaries rise by 0.6, -0.4 and 0.3.
        let text = CHART.replace(
            r#"{"start": 8, "end": 20, "label": "B", "energy": 0.9, "repeat": 0, "confidence": 0.6}"#,
            r#"{"start": 8, "end": 12, "label": "B", "energy": 0.9, "repeat": 0, "confidence": 0.6},
            {"start": 12, "end": 16, "label": "C", "energy": 0.5, "repeat": 0, "confidence": 0.6},
            {"start": 16, "end": 20, "label": "D", "energy": 0.8, "repeat": 0, "confidence": 0.6}"#,
        );
        let chart = Chart::from_analysis(serde_json::from_str(&text).unwrap());
        // Before the first start: the later boundaries are 0.9 to 0.5 and 0.5 to 0.8.
        assert!((chart.at(6.0).later_rise - 0.3).abs() < 1e-9);
        // Before the second: only 0.5 to 0.8 is left; before the third and in the last, none.
        assert!((chart.at(10.0).later_rise - 0.3).abs() < 1e-9);
        assert_eq!(chart.at(14.0).later_rise, 0.0);
        assert_eq!(chart.at(18.0).later_rise, 0.0);
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
    fn the_latest_onset_is_counted_exactly() {
        let at = |t| chart().at(t).latest[0];
        assert_eq!(at(0.9), LatestOnset::default());
        assert_eq!(
            at(1.0),
            LatestOnset {
                count: 1,
                time: 1.0,
                strength: 0.8
            }
        );
        assert_eq!(at(1.99).count, 1);
        assert_eq!(
            at(2.5),
            LatestOnset {
                count: 2,
                time: 2.0,
                strength: 0.5
            }
        );
    }

    #[test]
    fn curves_are_interpolated() {
        assert!((chart().at(0.25).loudness - 0.5).abs() < 1e-9);
        assert!((chart().at(0.25).feel - 5.0).abs() < 1e-9);
        assert_eq!(chart().at(0.25).song_feel, 5.0);
        assert!((chart().at(0.75).loudness - 0.75).abs() < 1e-9);
        assert_eq!(chart().at(99.0).loudness, 0.5);
    }

    #[test]
    fn a_second_without_sound_is_silent() {
        let curve = [0.8, 0.3, 0.04, 0.0, 0.0, 0.01, 0.0, 0.9];
        let at = |t| silent(&curve, 0.25, t);
        // The sound fades out at 0.5 s: silent once the last second holds nothing but silence.
        assert!(!at(0.5));
        assert!(!at(1.4));
        assert!(at(1.5));
        // The sound comes back at 1.75 s, and counts from the sample before it.
        assert!(!at(1.6));
        assert!(!at(9.0));
        assert!(!silent(&[], 0.25, 1.0));
        // Before the song's first sound there is no beat to follow either.
        assert!(chart().at(0.0).silent);
        assert!(!chart().at(0.5).silent);
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

    /// Clicks with a low thump under each, at 120 beats per minute, as a mono 16-bit WAV of
    /// `seconds` in a folder of its own.
    fn write_clicks(name: &str, seconds: usize) -> PathBuf {
        const RATE: u32 = 22050;
        let dir =
            std::env::temp_dir().join(format!("visualizer_chart_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut samples = vec![0i16; seconds * RATE as usize];
        for start in (0..samples.len()).step_by(RATE as usize / 2) {
            for i in 0..2000.min(samples.len() - start) {
                let t = i as f64 / f64::from(RATE);
                let thump = (2.0 * std::f64::consts::PI * 60.0 * t).sin() * (-t * 30.0).exp();
                let click = if i < 40 { 0.5 } else { 0.0 };
                samples[start + i] = ((thump * 0.8 + click) * 30000.0) as i16;
            }
        }
        let data = samples.len() as u32 * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 1, 0]);
        bytes.extend_from_slice(&RATE.to_le_bytes());
        bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
        bytes.extend_from_slice(&[2, 0, 16, 0]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let path = dir.join("clicks.wav");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn playing(path: &Path, position: f64) -> Playback {
        Playback {
            path: Some(path.to_path_buf()),
            position: Duration::from_secs_f64(position),
            playing: false,
            at: Some(Instant::now()),
        }
    }

    /// Ticks until a chart arrives (at most 30 s).
    fn wait_for_chart(follower: &mut ChartFollower, playback: &Playback) -> Option<ChartState> {
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            if let Some(state) = follower.tick(playback, Instant::now()) {
                return Some(state);
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn the_follower_analyses_the_playing_file() {
        let path = write_clicks("follows", 20);
        let mut follower = ChartFollower::new(Some(Arc::new(FsIo::new())));
        let playback = playing(&path, 10.0);
        // The live tracker has the say until the analysis is done.
        assert!(follower.tick(&playback, Instant::now()).is_none());
        let state = wait_for_chart(&mut follower, &playback).expect("no chart in time");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
        // What the analyzer gives for it: the clicks' own 120 beats per minute.
        assert!((state.bpm - 120.0).abs() < 1.0, "{}", state.bpm);
        assert!(state.beat > 5.0 && state.beats_per_bar >= 1.0);
    }

    #[test]
    fn no_file_system_or_no_path_means_no_analysis() {
        let path = write_clicks("none", 5);
        let mut follower = ChartFollower::new(None);
        assert!(
            follower
                .tick(&playing(&path, 1.0), Instant::now())
                .is_none()
        );
        assert!(follower.loading.is_none());
        let mut follower = ChartFollower::new(Some(Arc::new(FsIo::new())));
        let online = Playback {
            path: Some(PathBuf::new()),
            ..playing(&path, 1.0)
        };
        assert!(follower.tick(&online, Instant::now()).is_none());
        assert!(follower.loading.is_none() && follower.cancel.is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_new_path_or_a_drop_cancels_the_analysis() {
        // Ten minutes take several seconds to analyse.
        let long = write_clicks("long", 600);
        let short = write_clicks("short", 5);
        let mut follower = ChartFollower::new(Some(Arc::new(FsIo::new())));
        follower.tick(&playing(&long, 0.0), Instant::now());
        let first = follower.cancel.clone().expect("analysing");
        assert!(!first.load(Ordering::Relaxed));
        follower.tick(&playing(&short, 0.0), Instant::now());
        assert!(first.load(Ordering::Relaxed));
        let second = follower.cancel.clone().expect("analysing");
        assert!(!second.load(Ordering::Relaxed));
        drop(follower);
        assert!(second.load(Ordering::Relaxed));
        for path in [long, short] {
            std::fs::remove_dir_all(path.parent().unwrap()).ok();
        }
    }
}
