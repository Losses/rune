//! The signals a scene is meant to read instead of the raw findings: a tempo that is never 0, a
//! beat clock that always runs, how sure the beat is, how much is going on, and the moments that
//! matter (big cues, melody notes, hard impacts). They are made once per frame from the raw
//! values of [`super::inputs::music`], the beat tracker and the chart, so that every scene gets the
//! same answer to "is there a beat" and "is this a big moment" and none has to work it out from
//! `bpm`, `feel` and `energy` again.
//!
//! The moments only fire on a steady frame: the song time moved forward by a frame's worth and
//! the chart is the same as before. A seek, a stall, a new song and the chart arriving fire
//! nothing.

use crate::chart::{ChartState, LatestOnset, section_level};
use crate::rhythm::Rhythm;

use super::inputs::MusicValues;

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
/// The streams of the chart, as indices into [`ChartState::latest`], that make hits when there
/// is no beat: the lead (or this one when nothing leads) and the phrase starts.
const FALLBACK_STREAM: usize = 3;
const PHRASE_STREAM: usize = 4;

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

/// The time constant (seconds) of `tempo` gliding to a new target: when the beat becomes
/// trusted or is lost, when the chart arrives, when the trusted tempo steps, and while it
/// follows `feel`.
pub const TEMPO_GLIDE: f64 = 1.0;
/// A glide starts when the source of the tempo switches (the beat becoming trusted or lost,
/// the chart arriving or leaving) or its target steps by more than this many bpm in one frame,
/// as when the tracker finds twice the tempo; small changes are told as they come.
const TEMPO_STEP: f64 = 2.0;
/// A glide ends once `tempo` is within this many bpm of its target, or after
/// [`TEMPO_GLIDE_TIME`] seconds (less than 1% of the step is left then).
const TEMPO_EXACT: f64 = 0.5;
pub const TEMPO_GLIDE_TIME: f64 = 5.0;
/// The tempo (bpm) told when nothing is known of how fast the music is.
const TEMPO_DEFAULT: f64 = 90.0;
/// The tempos (bpm) that a `feel` of 3 Hz and of 7 Hz stand for when there is no beat to trust:
/// slow music is told as slow and fast music as fast, in between linearly.
const TEMPO_FEEL_SLOW: f64 = 70.0;
const TEMPO_FEEL_FAST: f64 = 110.0;
/// Without a beat to trust each hit moves `clock` on to the next whole beat, easing there with
/// this time constant (seconds), and it is put on the beat once it is this close.
const CLOCK_STEP: f64 = 0.08;
const CLOCK_ARRIVED: f64 = 0.002;
/// The most `flow` moves on in a beat's time: the bands are about 1 on average and up to 4 on a
/// burst.
const FLOW_MAX: f64 = 3.0;
/// The fewest and the most beats a bar of `meter` has, and the meter without a chart.
const METER_MIN: f64 = 2.0;
const METER_MAX: f64 = 7.0;
const METER_DEFAULT: f64 = 4.0;
/// How long without a beat (in beats of the tempo) `locked` holds before it falls.
pub const LOCK_BEATS: f64 = 2.5;
/// The time constant (seconds) of `locked` following its raw value.
const LOCK_SMOOTH: f64 = 0.3;
/// How many bars before the end of a section `drive` starts to ramp towards the next section's
/// energy, and how many seconds when there is no beat to count bars with.
pub const DRIVE_BARS: f64 = 4.0;
pub const DRIVE_SECONDS: f64 = 8.0;
/// The time constants (seconds) of `drive` with a chart (only to round the steps at the edges of
/// sections) and without (the loudness, slowly).
const DRIVE_SMOOTH_CHART: f64 = 0.3;
const DRIVE_SMOOTH_LIVE: f64 = 1.5;
/// The time constant (seconds) of `intensity`, and the live energy (about 1 on average) that
/// stands for 0 and the span up to 1 when there is no chart.
const INTENSITY_SMOOTH: f64 = 0.8;
/// Without a chart, the rank of the slow energy among what the song has had: a histogram of the
/// time spent at each level, forgetting with this time constant (seconds), so a new song takes
/// over within minutes.
const LEVEL_BINS: usize = 16;
const LEVEL_MEMORY: f64 = 180.0;
const INTENSITY_ENERGY_FLOOR: f64 = 0.6;
const INTENSITY_ENERGY_SPAN: f64 = 1.0;
/// A section whose energy is more than this above the one before is a lift: a cue of 1 rather
/// than 0.5.
pub const CUE_LIFT: f64 = 0.1;
/// Every this many bars since the last cue is a cue while the beat is locked, chart or not
/// (strength [`CUE_BAR_STRENGTH`]: a chart with one long section still gets them), and the time
/// (seconds) since the last cue after which the next hit is one when the beat is not locked
/// (strength [`CUE_IDLE_STRENGTH`]).
pub const CUE_BARS: u32 = 8;
pub const CUE_IDLE: f64 = 16.0;
const CUE_BAR_STRENGTH: f64 = 0.3;
const CUE_IDLE_STRENGTH: f64 = 0.2;
/// The `locked` from which the beat counts as steady enough to count bars for cues.
const CUE_LOCKED: f64 = 0.5;
/// The least time (seconds) between two cues: a later one is dropped.
pub const CUE_GAP: f64 = 4.0;
/// The time constant (seconds) of `cue` fading.
const CUE_DECAY: f64 = 1.0;
/// Without a chart, a melody note is `mids` rising this far above its own slow average; it
/// arms again once the rise is below half of it.
pub const NOTE_RISE: f64 = 0.35;
/// The time constant (seconds) of the slow average of `mids` the rise is measured against.
const NOTE_SLOW: f64 = 0.6;
/// The least time (seconds) between two notes.
pub const NOTE_GAP: f64 = 0.1;
/// The weakest note strength told from a rise, and the weakest onset of the chart's harmonic
/// stream that is a note (the others are ornaments).
const NOTE_MIN_STRENGTH: f64 = 0.4;
const CHART_NOTE_MIN: f64 = 0.3;
/// The stream of the chart (an index into [`ChartState::latest`]) whose onsets are notes: the
/// harmonic one.
const NOTE_STREAM: usize = 3;
/// The time constant (seconds) of `note` fading.
const NOTE_DECAY: f64 = 0.2;
/// The time constants (seconds) of the bands `band_low`, `band_mid` and `band_high` rising and
/// falling: quick to rise so that a hit shows, slow to fall so that it does not flicker.
const BAND_ATTACK: f64 = 0.03;
const BAND_RELEASE: f64 = 0.2;
/// An impact is the low onset rising above this level (1 is as strong as the recent strongest)
/// while `drive` is above [`IMPACT_DRIVE`], at least [`IMPACT_GAP`] seconds after the last.
pub const IMPACT_LEVEL: f64 = 0.95;
pub const IMPACT_DRIVE: f64 = 0.55;
pub const IMPACT_GAP: f64 = 6.0;
/// The time constant (seconds) of `impact` fading.
const IMPACT_DECAY: f64 = 0.35;

/// What [`Signals::tick`] looks at in one frame, besides the raw [`MusicValues`].
pub struct SignalInput<'a> {
    /// Whether the beat is to be trusted ([`super::inputs::beat_trusted`]).
    pub trusted: bool,
    pub chart: Option<&'a ChartState>,
    pub rhythm: &'a Rhythm,
    /// The song time, and the wall seconds since the last frame.
    pub song_time: f64,
    pub wall_dt: f64,
}

/// `x` moving towards `target` over `dt` seconds with time constant `tau`.
fn approach(x: f64, target: f64, dt: f64, tau: f64) -> f64 {
    x + (target - x) * (1.0 - (-dt / tau).exp())
}

/// The tempo (bpm) that a `feel` (Hz) stands for when no beat is to be trusted.
fn tempo_from_feel(feel: f64) -> f64 {
    if feel <= 0.0 {
        return TEMPO_DEFAULT;
    }
    let fast = ((feel - 3.0) / 4.0).clamp(0.0, 1.0);
    TEMPO_FEEL_SLOW + (TEMPO_FEEL_FAST - TEMPO_FEEL_SLOW) * fast
}

/// Makes the signals of [`MusicValues`] from the raw values. Tick it once per frame, after the
/// raw values and the hits are filled in.
#[derive(Debug)]
pub struct Signals {
    /// The song time of the last frame, and whether it had a chart.
    time: Option<f64>,
    chart: bool,
    /// Wall seconds since the first frame, the clock of every gap and idle time.
    now: f64,
    /// `tempo`, the target and source (trusted, chart) of the last frame, and the glide time
    /// left.
    tempo: f64,
    tempo_target: f64,
    tempo_source: (bool, bool),
    glide: f64,
    /// The beat clock, the whole beat it eases to after a hit while there is no beat to trust,
    /// and whether the last frame had a beat to trust.
    clock: f64,
    clock_target: f64,
    clock_trusted: bool,
    flow: f64,
    /// Wall seconds since the last frame with a beat, and `locked`.
    since_beat: f64,
    locked: f64,
    drive: f64,
    intensity: f64,
    /// Seconds spent at each slow energy, in [`LEVEL_BINS`] steps, forgetting slowly: what the
    /// level ranks the energy by without a chart.
    heard: [f64; LEVEL_BINS],
    /// The energy of the section as of the last frame, to compare the next one against.
    section_energy: f64,
    last_cue: f64,
    bars_since_cue: u32,
    cue: f64,
    /// Onsets of the chart's harmonic stream seen so far, the song time of the last chart note,
    /// the wall time of the last live note, the slow average of `mids`, whether a live note may
    /// fire and the envelope `note`.
    notes_seen: usize,
    last_chart_note: f64,
    last_live_note: f64,
    mids_slow: Option<f64>,
    note_armed: bool,
    note: f64,
    bands: [f64; 3],
    impact_above: bool,
    last_impact: f64,
    impact: f64,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            time: None,
            chart: false,
            now: 0.0,
            tempo: TEMPO_DEFAULT,
            tempo_target: TEMPO_DEFAULT,
            tempo_source: (false, false),
            glide: 0.0,
            clock: 0.0,
            clock_target: 0.0,
            clock_trusted: false,
            flow: 0.0,
            since_beat: f64::INFINITY,
            locked: 0.0,
            drive: 0.0,
            intensity: 0.0,
            heard: [0.0; LEVEL_BINS],
            section_energy: 0.0,
            last_cue: f64::NEG_INFINITY,
            bars_since_cue: 0,
            cue: 0.0,
            notes_seen: 0,
            last_chart_note: f64::NEG_INFINITY,
            last_live_note: f64::NEG_INFINITY,
            mids_slow: None,
            note_armed: true,
            note: 0.0,
            bands: [0.0; 3],
            impact_above: false,
            last_impact: f64::NEG_INFINITY,
            impact: 0.0,
        }
    }
}

impl Signals {
    /// Fills in the signals of `m`, whose raw values (and `hits`) are final.
    pub fn tick(&mut self, input: &SignalInput, m: &mut MusicValues) {
        let first = self.time.is_none();
        let has_chart = input.chart.is_some();
        let steady = self.time.is_some_and(|before| {
            (0.0..=HIT_MAX_FRAME).contains(&(input.song_time - before)) && self.chart == has_chart
        });
        self.time = Some(input.song_time);
        self.chart = has_chart;
        let dt = input.wall_dt.max(0.0);
        self.now += dt;
        if !steady {
            self.last_chart_note = f64::NEG_INFINITY;
        }

        let meter = match input.chart {
            Some(_) if m.beats_per_bar.is_finite() && m.beats_per_bar > 0.0 => {
                f64::from(m.beats_per_bar)
                    .round()
                    .clamp(METER_MIN, METER_MAX)
            }
            _ => METER_DEFAULT,
        };
        self.follow_tempo(input, m, first, dt);
        // A seek or the chart arriving or leaving changes what the beat counts from.
        self.run_clock(input, meter, m.hits, !steady, dt);
        self.follow_lock(input, m, dt);
        self.follow_energy(input, m, meter, first, dt);
        let cues = self.cues(input, m, steady, dt);
        let notes = self.notes(input, m, steady, dt);
        self.follow_bands(m, dt);
        let impacts = self.impacts(input, m, steady, dt);
        self.follow_flow(dt);
        self.section_energy = f64::from(m.section_energy);

        m.tempo = self.tempo as f32;
        m.clock = self.clock as f32;
        m.meter = meter as f32;
        m.locked = self.locked as f32;
        m.drive = self.drive as f32;
        m.intensity = self.intensity as f32;
        m.level = self.level(input, m, dt) as f32;
        m.cues = cues;
        m.cue = self.cue as f32;
        m.cue_in = if has_chart { m.section_left } else { -1.0 };
        m.notes = notes;
        m.note = self.note as f32;
        m.band_low = self.bands[0] as f32;
        m.band_mid = self.bands[1] as f32;
        m.band_high = self.bands[2] as f32;
        m.impacts = impacts;
        m.impact = self.impact as f32;
        m.flow = self.flow as f32;
    }

    /// `tempo`: the trusted tempo as it comes, the tempo `feel` stands for otherwise, and a
    /// glide to the new target when the source switches or the target steps.
    fn follow_tempo(&mut self, input: &SignalInput, m: &MusicValues, first: bool, dt: f64) {
        let source = (input.trusted, input.chart.is_some());
        let target = if input.trusted {
            f64::from(m.bpm)
        } else {
            tempo_from_feel(f64::from(m.feel))
        };
        if first {
            (self.tempo, self.glide) = (target, 0.0);
        } else {
            if source != self.tempo_source || (target - self.tempo_target).abs() > TEMPO_STEP {
                self.glide = TEMPO_GLIDE_TIME;
            }
            if self.glide > 0.0 {
                self.tempo = approach(self.tempo, target, dt, TEMPO_GLIDE);
                self.glide -= dt;
                if self.glide <= 0.0 || (target - self.tempo).abs() <= TEMPO_EXACT {
                    (self.tempo, self.glide) = (target, 0.0);
                }
            } else if input.trusted {
                self.tempo = target;
            } else {
                self.tempo = approach(self.tempo, target, dt, TEMPO_GLIDE);
            }
        }
        (self.tempo_target, self.tempo_source) = (target, source);
    }

    /// `clock`: with a beat to trust it runs at the tempo with the beat's phase, and when it
    /// gets one, or the beat counts from elsewhere after a seek or a chart (`realign`), it also
    /// takes its place in the bar. Without one it does not make up a tempo: each hit moves it on to
    /// the next whole beat and it rests between hits, so that what steps on it steps on what is
    /// heard. It never goes back.
    fn run_clock(&mut self, input: &SignalInput, meter: f64, hits: u32, realign: bool, dt: f64) {
        if input.trusted {
            let mut clock = self.clock + dt * self.tempo / 60.0;
            let beat = input.rhythm.beat;
            let phase = beat - beat.floor();
            // The whole beat nearest the clock that keeps the phase of `beat`.
            let mut whole = (clock - phase).round();
            if !self.clock_trusted || realign {
                let bar = meter as i64;
                let ahead = (beat.floor() as i64 - whole as i64).rem_euclid(bar);
                whole += ahead as f64;
            }
            clock = whole + phase;
            self.clock = clock.max(self.clock);
            self.clock_target = self.clock;
        } else {
            if hits > 0 {
                self.clock_target = self.clock_target.floor() + f64::from(hits);
            }
            self.clock = approach(self.clock, self.clock_target, dt, CLOCK_STEP);
            if self.clock_target - self.clock < CLOCK_ARRIVED {
                self.clock = self.clock_target;
            }
        }
        self.clock_trusted = input.trusted;
    }

    /// `flow`: moves on at the tempo, as much as the bands are loud against their average, so
    /// it slows in quiet passages and stops in silence.
    fn follow_flow(&mut self, dt: f64) {
        let activity = (self.bands.iter().sum::<f64>() / 3.0).clamp(0.0, FLOW_MAX);
        self.flow += dt * self.tempo / 60.0 * activity;
    }

    /// `locked`: the beat is trusted and was heard within the last [`LOCK_BEATS`] beats.
    fn follow_lock(&mut self, input: &SignalInput, m: &MusicValues, dt: f64) {
        if m.beats > 0 {
            self.since_beat = 0.0;
        } else {
            self.since_beat += dt;
        }
        let raw =
            input.trusted && m.bpm > 0.0 && self.since_beat < LOCK_BEATS * 60.0 / f64::from(m.bpm);
        self.locked = approach(self.locked, f64::from(u8::from(raw)), dt, LOCK_SMOOTH);
    }

    /// `drive` and `intensity`: the slow and the faster energy of the music.
    fn follow_energy(
        &mut self,
        input: &SignalInput,
        m: &MusicValues,
        meter: f64,
        first: bool,
        dt: f64,
    ) {
        let loudness = f64::from(m.loudness);
        let (drive, tau) = if input.chart.is_some() {
            let ramp = if input.trusted && m.bpm > 0.0 {
                DRIVE_BARS * meter * 60.0 / f64::from(m.bpm)
            } else {
                DRIVE_SECONDS
            };
            let r = (1.0 - f64::from(m.section_left) / ramp).clamp(0.0, 1.0);
            let (now, next) = (f64::from(m.section_energy), f64::from(m.next_energy));
            (now + (next - now) * r * r, DRIVE_SMOOTH_CHART)
        } else {
            (loudness, DRIVE_SMOOTH_LIVE)
        };
        let intensity = if input.chart.is_some() {
            loudness
        } else {
            ((f64::from(m.energy) - INTENSITY_ENERGY_FLOOR) / INTENSITY_ENERGY_SPAN).clamp(0.0, 1.0)
        };
        let drive = drive.clamp(0.0, 1.0);
        if first {
            (self.drive, self.intensity) = (drive, intensity);
        }
        self.drive = approach(self.drive, drive, dt, tau).clamp(0.0, 1.0);
        self.intensity = approach(self.intensity, intensity, dt, INTENSITY_SMOOTH).clamp(0.0, 1.0);
    }

    /// `level`: how high the section is in the song, half its loudness and half its rank. With a
    /// chart, the section's energy and its rank among the song's sections (both known ahead);
    /// without, the slow energy and its rank among what the song has had so far.
    fn level(&mut self, input: &SignalInput, m: &MusicValues, dt: f64) -> f64 {
        let (energy, rank) = match input.chart {
            Some(chart) => (f64::from(m.section_energy), chart.section_rank),
            None => {
                let forget = (-dt / LEVEL_MEMORY).exp();
                for seconds in &mut self.heard {
                    *seconds *= forget;
                }
                let bin = ((self.drive * LEVEL_BINS as f64) as usize).min(LEVEL_BINS - 1);
                self.heard[bin] += dt;
                let total: f64 = self.heard.iter().sum();
                let rank = if total > 0.0 {
                    let below: f64 = self.heard[..bin].iter().sum();
                    (below + 0.5 * self.heard[bin]) / total
                } else {
                    0.5
                };
                (self.drive, rank)
            }
        };
        section_level(energy, rank)
    }

    /// The cues of this frame, and `cue` fading or set. The first source that applies decides: a
    /// section of the chart; else, while the beat is locked, every [`CUE_BARS`] bars since the
    /// last cue; else the first hit after [`CUE_IDLE`] seconds without one. Neither of the last
    /// two comes within [`CUE_GAP`] before a section start the chart knows is coming: the gap
    /// would drop that section start for it.
    fn cues(&mut self, input: &SignalInput, m: &MusicValues, steady: bool, dt: f64) -> u32 {
        self.cue *= (-dt / CUE_DECAY).exp();
        let section = (input.chart.is_some() && m.sections > 0).then(|| {
            let lift = f64::from(m.section_energy) > self.section_energy + CUE_LIFT;
            if lift { 1.0 } else { 0.5 }
        });
        let section_soon =
            input.chart.is_some() && m.section_left > 0.0 && f64::from(m.section_left) < CUE_GAP;
        let strength = if section.is_some() {
            // A section start begins the count of bars again.
            self.bars_since_cue = 0;
            section
        } else if self.locked > CUE_LOCKED {
            self.bars_since_cue += m.bars;
            (self.bars_since_cue >= CUE_BARS).then_some(CUE_BAR_STRENGTH)
        } else {
            self.bars_since_cue = 0;
            (m.hits > 0 && self.now - self.last_cue >= CUE_IDLE).then_some(CUE_IDLE_STRENGTH)
        };
        match strength {
            Some(_) if section.is_none() && section_soon => 0,
            Some(strength) if steady && self.now - self.last_cue >= CUE_GAP => {
                self.last_cue = self.now;
                self.bars_since_cue = 0;
                self.cue = strength;
                1
            }
            _ => 0,
        }
    }

    /// The notes of this frame, and `note` fading or set: new onsets of the chart's harmonic
    /// stream, or a rise of `mids` above its slow average without a chart.
    fn notes(&mut self, input: &SignalInput, m: &MusicValues, steady: bool, dt: f64) -> u32 {
        self.note *= (-dt / NOTE_DECAY).exp();
        let mut found = None;
        if let Some(chart) = input.chart {
            let latest: LatestOnset = chart.latest[NOTE_STREAM];
            if steady
                && latest.count > self.notes_seen
                && latest.strength >= CHART_NOTE_MIN
                && latest.time - self.last_chart_note >= NOTE_GAP
            {
                self.last_chart_note = latest.time;
                found = Some(latest.strength);
            }
            self.notes_seen = latest.count;
        } else {
            self.notes_seen = 0;
            let mids = f64::from(m.mids);
            let slow = self.mids_slow.unwrap_or(mids);
            let rise = mids - slow;
            if rise < NOTE_RISE * 0.5 {
                self.note_armed = true;
            } else if self.note_armed && rise > NOTE_RISE && steady {
                self.note_armed = false;
                if self.now - self.last_live_note >= NOTE_GAP {
                    self.last_live_note = self.now;
                    found = Some((rise / (2.0 * NOTE_RISE)).clamp(NOTE_MIN_STRENGTH, 1.0));
                }
            }
            self.mids_slow = Some(approach(slow, mids, dt, NOTE_SLOW));
        }
        match found {
            Some(strength) => {
                self.note = strength;
                1
            }
            None => 0,
        }
    }

    /// `band_low`, `band_mid` and `band_high`: the live bands with a quick attack and a slow
    /// release.
    fn follow_bands(&mut self, m: &MusicValues, dt: f64) {
        let levels = [m.bass, m.mids, m.treble];
        for (band, level) in self.bands.iter_mut().zip(levels) {
            let level = f64::from(level);
            let tau = if level > *band {
                BAND_ATTACK
            } else {
                BAND_RELEASE
            };
            *band = approach(*band, level, dt, tau);
        }
    }

    /// The impacts of this frame, and `impact` fading or set.
    fn impacts(&mut self, input: &SignalInput, m: &MusicValues, steady: bool, dt: f64) -> u32 {
        self.impact *= (-dt / IMPACT_DECAY).exp();
        let onset = if input.chart.is_some() {
            f64::from(m.low)
        } else {
            input.rhythm.kick
        };
        let rising = onset > IMPACT_LEVEL && !self.impact_above;
        self.impact_above = onset > IMPACT_LEVEL;
        if steady
            && rising
            && self.drive > IMPACT_DRIVE
            && self.now - self.last_impact >= IMPACT_GAP
        {
            self.last_impact = self.now;
            self.impact = onset.clamp(0.0, 1.0);
            return 1;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::inputs::{EventCounter, Levels, beat_trusted, music};

    /// The wall time of one frame in these tests.
    const DT: f64 = 0.05;

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

    /// Feeds signals frames of [`DT`] seconds, the song time moving with the wall time.
    #[derive(Default)]
    struct Rig {
        signals: Signals,
        time: f64,
        first: bool,
    }

    impl Rig {
        /// One frame: `raw` has the values the signals read, `rhythm` the tracker's.
        fn step(
            &mut self,
            raw: MusicValues,
            chart: Option<&ChartState>,
            rhythm: &Rhythm,
        ) -> MusicValues {
            if self.first {
                self.time += DT;
            }
            self.first = true;
            self.jump(raw, chart, rhythm, self.time)
        }

        /// One frame at an arbitrary song time, as after a seek.
        fn jump(
            &mut self,
            mut raw: MusicValues,
            chart: Option<&ChartState>,
            rhythm: &Rhythm,
            time: f64,
        ) -> MusicValues {
            self.time = time;
            let trusted = beat_trusted(rhythm, chart.is_some());
            if !trusted {
                raw.bpm = 0.0;
            }
            raw.beat = rhythm.beat as f32;
            self.signals.tick(
                &SignalInput {
                    trusted,
                    chart,
                    rhythm,
                    song_time: time,
                    wall_dt: DT,
                },
                &mut raw,
            );
            raw
        }

        /// `seconds` of the same frame; the last one's values.
        fn hold(
            &mut self,
            raw: MusicValues,
            chart: Option<&ChartState>,
            rhythm: &Rhythm,
            seconds: f64,
        ) -> MusicValues {
            let mut m = raw;
            for _ in 0..(seconds / DT).round() as usize {
                m = self.step(raw, chart, rhythm);
            }
            m
        }
    }

    /// A tracker that is sure of `bpm` and has counted to `beat`.
    fn beating(bpm: f64, beat: f64) -> Rhythm {
        Rhythm {
            bpm,
            beat,
            confidence: 0.9,
            ..Default::default()
        }
    }

    /// Raw values of a frame with the trusted tempo `bpm`.
    fn raw_bpm(bpm: f32) -> MusicValues {
        MusicValues {
            bpm,
            ..Default::default()
        }
    }

    /// A chart with sections, 4 beats to the bar and a trusted tempo.
    fn sectioned() -> ChartState {
        ChartState {
            bpm: 120.0,
            beats_per_bar: 4.0,
            ..Default::default()
        }
    }

    #[test]
    fn the_tempo_is_never_zero_and_is_the_trusted_bpm() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        assert_eq!(rig.step(MusicValues::default(), None, &nothing).tempo, 90.0);
        // A beat to trust: after the glide it is the tracker's tempo exactly, as it moves.
        let mut beat = 0.0;
        let mut m = MusicValues::default();
        for i in 0..120 {
            beat += DT * 2.0;
            m = rig.step(raw_bpm(120.0 + i as f32 * 0.1), None, &beating(120.0, beat));
            assert!(m.tempo > 0.0);
        }
        assert_eq!(m.tempo, 131.9);
    }

    #[test]
    fn without_a_beat_the_tempo_comes_from_how_fast_the_music_feels() {
        assert_eq!(tempo_from_feel(0.0), 90.0);
        assert_eq!(tempo_from_feel(-1.0), 90.0);
        assert_eq!(tempo_from_feel(3.0), 70.0);
        assert_eq!(tempo_from_feel(1.0), 70.0);
        assert_eq!(tempo_from_feel(5.0), 90.0);
        assert_eq!(tempo_from_feel(7.0), 110.0);
        assert_eq!(tempo_from_feel(12.0), 110.0);
        let mut rig = Rig::default();
        let raw = MusicValues {
            feel: 7.0,
            ..Default::default()
        };
        assert_eq!(rig.step(raw, None, &Rhythm::default()).tempo, 110.0);
    }

    #[test]
    fn the_tempo_glides_to_a_new_target_and_then_follows_exactly() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let felt = MusicValues {
            feel: 3.0,
            ..Default::default()
        };
        rig.hold(felt, None, &nothing, 1.0);
        // The beat becomes trusted: no jump to 150, a glide towards it.
        let m = rig.step(raw_bpm(150.0), None, &beating(150.0, 10.0));
        assert!(m.tempo > 70.0 && m.tempo < 75.0, "{}", m.tempo);
        let m = rig.hold(raw_bpm(150.0), None, &beating(150.0, 10.0), 1.0);
        assert!(m.tempo > 100.0 && m.tempo < 140.0, "{}", m.tempo);
        // Once within half a bpm the glide ends and it is exact.
        let m = rig.hold(raw_bpm(150.0), None, &beating(150.0, 10.0), 4.5);
        assert_eq!(m.tempo, 150.0);
        // A small change is told as it comes, a step glides.
        let m = rig.step(raw_bpm(150.4), None, &beating(150.4, 10.0));
        assert_eq!(m.tempo, 150.4);
        let m = rig.step(raw_bpm(100.0), None, &beating(100.0, 10.0));
        assert!(m.tempo > 145.0, "{}", m.tempo);
        // And back: the beat is lost and the tempo glides to what `feel` says.
        let m = rig.step(felt, None, &nothing);
        assert!(m.tempo > 140.0, "{}", m.tempo);
        let m = rig.hold(felt, None, &nothing, 8.0);
        assert!((m.tempo - 70.0).abs() < 0.1, "{}", m.tempo);
    }

    #[test]
    fn the_meter_is_the_charts_bars_rounded_and_four_otherwise() {
        let meter = |chart: Option<f64>| {
            let mut rig = Rig::default();
            let chart = chart.map(|beats_per_bar| ChartState {
                beats_per_bar,
                ..Default::default()
            });
            let raw = MusicValues {
                beats_per_bar: chart.as_ref().map_or(4.0, |c| c.beats_per_bar as f32),
                ..Default::default()
            };
            rig.step(raw, chart.as_ref(), &Rhythm::default()).meter
        };
        assert_eq!(meter(None), 4.0);
        assert_eq!(meter(Some(3.0)), 3.0);
        assert_eq!(meter(Some(3.4)), 3.0);
        assert_eq!(meter(Some(1.0)), 2.0);
        assert_eq!(meter(Some(12.0)), 7.0);
        assert_eq!(meter(Some(0.0)), 4.0);
        assert_eq!(meter(Some(f64::NAN)), 4.0);
    }

    #[test]
    fn the_clock_never_goes_backwards() {
        let mut rig = Rig::default();
        let mut before = 0.0;
        let mut beat = 5.3;
        for i in 0..400 {
            // Trusted and untrusted by turns, the tracker's beat jumping about, and seeks.
            let trusted = (i / 37) % 2 == 0;
            beat += if i % 53 == 0 { -7.4 } else { DT * 2.0 };
            let rhythm = if trusted {
                beating(120.0, beat.max(0.0))
            } else {
                Rhythm::default()
            };
            let time = if i % 71 == 0 {
                300.0 - f64::from(i)
            } else {
                rig.time + DT
            };
            let m = rig.jump(raw_bpm(120.0), None, &rhythm, time);
            assert!(m.clock >= before, "frame {i}: {} < {before}", m.clock);
            before = m.clock;
        }
        assert!(before > 0.0);
    }

    #[test]
    fn without_a_beat_the_clock_steps_on_hits_and_rests_between_them() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let felt = MusicValues {
            feel: 5.0,
            ..Default::default()
        };
        // No hits: it makes up no tempo.
        let m = rig.hold(felt, None, &nothing, 3.0);
        assert_eq!(m.clock, 0.0);
        // A hit: it eases on to the next whole beat and rests there.
        let hit = MusicValues { hits: 1, ..felt };
        let m = rig.step(hit, None, &nothing);
        assert!(m.clock > 0.0 && m.clock < 1.0, "{}", m.clock);
        let m = rig.hold(felt, None, &nothing, 1.0);
        assert_eq!(m.clock, 1.0);
        // Two hits in one frame are two steps.
        rig.step(MusicValues { hits: 2, ..felt }, None, &nothing);
        let m = rig.hold(felt, None, &nothing, 1.0);
        assert_eq!(m.clock, 3.0);
    }

    #[test]
    fn the_clock_steps_from_where_the_beat_left_it() {
        let mut rig = Rig::default();
        let m = rig.step(raw_bpm(120.0), None, &beating(120.0, 5.3));
        assert!((m.clock.fract() - 0.3).abs() < 1e-3, "{}", m.clock);
        // The beat is lost mid-beat: the clock waits there, and a hit takes it to the next one.
        let nothing = Rhythm::default();
        let held = rig.hold(MusicValues::default(), None, &nothing, 1.0);
        assert!(
            (held.clock - m.clock).abs() < 0.05,
            "{} {}",
            held.clock,
            m.clock
        );
        rig.step(
            MusicValues {
                hits: 1,
                ..Default::default()
            },
            None,
            &nothing,
        );
        let m = rig.hold(MusicValues::default(), None, &nothing, 1.0);
        assert_eq!(m.clock, held.clock.floor() + 1.0);
    }

    #[test]
    fn flow_moves_with_how_much_is_going_on() {
        let flow = |level: f32| {
            let mut rig = Rig::default();
            let raw = MusicValues {
                bass: level,
                mids: level,
                treble: level,
                feel: 5.0,
                ..Default::default()
            };
            rig.hold(raw, None, &Rhythm::default(), 10.0).flow
        };
        // At the average level it moves at the tempo (90 bpm here), louder faster, and in
        // silence not at all.
        let average = flow(1.0);
        assert!((average - 15.0).abs() < 0.3, "{average}");
        assert!((flow(2.0) - 2.0 * average).abs() < 0.5);
        assert_eq!(flow(0.0), 0.0);
        assert!((flow(9.0) - 3.0 * average).abs() < 0.5);
    }

    #[test]
    fn the_clock_has_the_phase_of_the_beat_and_a_downbeat_at_a_multiple_of_the_meter() {
        let mut rig = Rig::default();
        // Free-running for a while, then a beat to trust, 37.3 beats counted so far.
        let felt = MusicValues {
            feel: 5.0,
            ..Default::default()
        };
        let free = rig.hold(felt, None, &Rhythm::default(), 3.2);
        let m = rig.step(raw_bpm(120.0), None, &beating(120.0, 37.3));
        assert!(m.clock >= free.clock);
        assert_eq!(m.clock.floor() as i64 % 4, 37 % 4);
        assert!((m.clock.fract() - 0.3).abs() < 1e-3, "{}", m.clock);
        // It does not move to the bar again, and keeps the phase as the beat goes on.
        let mut beat = 37.3;
        for _ in 0..100 {
            beat += 0.0877;
            let m = rig.step(raw_bpm(120.0), None, &beating(120.0, beat));
            assert_eq!(m.clock.floor() as i64 % 4, beat.floor() as i64 % 4);
            let apart = (m.clock.fract() - beat.fract() as f32).abs();
            assert!(apart.min(1.0 - apart) < 1e-3, "{apart}");
        }
    }

    #[test]
    fn the_downbeat_alignment_uses_the_charts_meter() {
        let mut rig = Rig::default();
        let chart = ChartState {
            beats_per_bar: 3.0,
            ..sectioned()
        };
        let raw = MusicValues {
            bpm: 120.0,
            beats_per_bar: 3.0,
            ..Default::default()
        };
        let m = rig.step(raw, Some(&chart), &beating(120.0, 11.5));
        assert_eq!(m.meter, 3.0);
        assert_eq!(m.clock.floor() as i64 % 3, 11 % 3);
    }

    #[test]
    fn a_seek_or_the_chart_arriving_moves_the_clock_to_the_bar_again() {
        let mut rig = Rig::default();
        let mut beat = 8.0;
        for _ in 0..40 {
            beat += DT * 2.0;
            rig.step(raw_bpm(120.0), None, &beating(120.0, beat));
        }
        // The chart arrives, counting its beats from somewhere else: 101.5, the second beat of
        // its bar of 4.
        let chart = sectioned();
        let raw = MusicValues {
            beats_per_bar: 4.0,
            ..raw_bpm(120.0)
        };
        let before = rig.signals.clock;
        let m = rig.step(raw, Some(&chart), &beating(120.0, 101.5));
        assert!(f64::from(m.clock) >= before);
        assert_eq!(m.clock.floor() as i64 % 4, 101 % 4);
        // A seek: the beat now counts 202.25.
        let m = rig.jump(raw, Some(&chart), &beating(120.0, 202.25), 100.0);
        assert_eq!(m.clock.floor() as i64 % 4, 202 % 4);
        assert!((m.clock.fract() - 0.25).abs() < 1e-3, "{}", m.clock);
    }

    #[test]
    fn locked_falls_to_zero_after_two_and_a_half_beats_without_one() {
        let mut rig = Rig::default();
        let rhythm = beating(120.0, 4.0);
        // A beat every 0.5 s for four seconds.
        let mut m = MusicValues::default();
        for i in 0..80 {
            let raw = MusicValues {
                beats: u32::from(i % 10 == 0),
                ..raw_bpm(120.0)
            };
            m = rig.step(raw, None, &rhythm);
        }
        assert!(m.locked > 0.95, "{}", m.locked);
        // 2.5 beats at 120 bpm are 1.25 s: still locked 1 s after the last beat, gone well after.
        let m = rig.hold(raw_bpm(120.0), None, &rhythm, 0.5);
        assert!(m.locked > 0.9, "{}", m.locked);
        let m = rig.hold(raw_bpm(120.0), None, &rhythm, 3.0);
        assert!(m.locked < 0.02, "{}", m.locked);
        // And an untrusted tracker is never locked.
        let mut rig = Rig::default();
        let m = rig.hold(
            MusicValues {
                beats: 1,
                ..Default::default()
            },
            None,
            &Rhythm::default(),
            2.0,
        );
        assert_eq!(m.locked, 0.0);
    }

    #[test]
    fn drive_ramps_towards_the_next_sections_energy_at_the_end_of_a_section() {
        let chart = sectioned();
        let section = |left: f32, bpm: f32| MusicValues {
            section_energy: 0.2,
            next_energy: 0.9,
            section_left: left,
            bpm,
            beats_per_bar: 4.0,
            ..Default::default()
        };
        let mut rig = Rig::default();
        let beating = beating(60.0, 1.0);
        // Far from the end: the section's own energy.
        let m = rig.hold(section(60.0, 60.0), Some(&chart), &beating, 3.0);
        assert!((m.drive - 0.2).abs() < 1e-3, "{}", m.drive);
        // 4 bars of 4 beats at 60 bpm are 16 s: 4 s left is 3/4 of the way, squared.
        let m = rig.hold(section(4.0, 60.0), Some(&chart), &beating, 3.0);
        assert!((m.drive - (0.2 + 0.7 * 0.5625)).abs() < 1e-3, "{}", m.drive);
        let m = rig.hold(section(0.0, 60.0), Some(&chart), &beating, 3.0);
        assert!((m.drive - 0.9).abs() < 1e-3, "{}", m.drive);
        // With no beat to trust the ramp is 8 s: 4 s left is halfway, squared.
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let m = rig.hold(section(4.0, 0.0), Some(&chart), &nothing, 3.0);
        assert!((m.drive - (0.2 + 0.7 * 0.25)).abs() < 1e-3, "{}", m.drive);
    }

    #[test]
    fn without_a_chart_drive_follows_the_loudness_slowly_and_intensity_the_energy() {
        let mut rig = Rig::default();
        let raw = MusicValues {
            loudness: 0.6,
            energy: 1.4,
            ..Default::default()
        };
        let nothing = Rhythm::default();
        let m = rig.step(raw, None, &nothing);
        assert!((m.drive - 0.6).abs() < 1e-6);
        let loud = MusicValues {
            loudness: 1.0,
            ..raw
        };
        // The step from 0.6 to 1 is not made in a frame, but is in ten seconds.
        let m = rig.step(loud, None, &nothing);
        assert!(m.drive > 0.6 && m.drive < 0.62, "{}", m.drive);
        let m = rig.hold(loud, None, &nothing, 10.0);
        assert!(m.drive > 0.99, "{}", m.drive);
        assert!((m.intensity - 0.8).abs() < 1e-3, "{}", m.intensity);
        // With a chart, intensity is its loudness.
        let mut rig = Rig::default();
        let chart = sectioned();
        let m = rig.hold(
            MusicValues {
                loudness: 0.3,
                energy: 1.9,
                ..Default::default()
            },
            Some(&chart),
            &nothing,
            5.0,
        );
        assert!((m.intensity - 0.3).abs() < 1e-3, "{}", m.intensity);
    }

    /// A frame at a section line, with the sections' energy now `energy`.
    fn section_line(energy: f32) -> MusicValues {
        MusicValues {
            sections: 1,
            section_energy: energy,
            beats_per_bar: 4.0,
            ..Default::default()
        }
    }

    #[test]
    fn a_section_that_lifts_the_energy_is_a_full_cue_and_a_flat_one_half() {
        let chart = sectioned();
        let nothing = Rhythm::default();
        let level = |energy| MusicValues {
            section_energy: energy,
            ..Default::default()
        };
        let mut rig = Rig::default();
        rig.hold(level(0.3), Some(&chart), &nothing, 1.0);
        let m = rig.step(section_line(0.6), Some(&chart), &nothing);
        assert_eq!((m.cues, m.cue), (1, 1.0));
        // The envelope fades with a time constant of a second.
        let m = rig.hold(level(0.6), Some(&chart), &nothing, 1.0);
        assert!((m.cue - (-1.0f32).exp()).abs() < 0.01, "{}", m.cue);
        // Two cues are not closer than 4 s: this flat section comes a second after, and is dropped.
        let m = rig.step(section_line(0.62), Some(&chart), &nothing);
        assert_eq!(m.cues, 0);
        rig.hold(level(0.62), Some(&chart), &nothing, 4.0);
        let m = rig.step(section_line(0.64), Some(&chart), &nothing);
        assert_eq!((m.cues, m.cue), (1, 0.5));
        assert_eq!(m.cue_in, 0.0);
    }

    #[test]
    fn without_a_chart_every_eighth_bar_of_a_steady_beat_is_a_small_cue() {
        let mut rig = Rig::default();
        let rhythm = beating(120.0, 4.0);
        for i in 0..60 {
            let raw = MusicValues {
                beats: u32::from(i % 10 == 0),
                ..raw_bpm(120.0)
            };
            let m = rig.step(raw, None, &rhythm);
            assert_eq!((m.cues, m.cue_in), (0, -1.0));
        }
        let bar = MusicValues {
            bars: 1,
            beats: 1,
            ..raw_bpm(120.0)
        };
        // A bar line every half second or so, 16 of them.
        let mut cues = Vec::new();
        let mut cue = 0.0;
        for _ in 0..16 {
            let m = rig.step(bar, None, &rhythm);
            cues.push(m.cues);
            if m.cues > 0 {
                cue = m.cue;
            }
            rig.hold(raw_bpm(120.0), None, &rhythm, 0.5);
        }
        // The eighth and the sixteenth bar, 4.4 s apart.
        assert_eq!(cues.iter().sum::<u32>(), 2);
        assert_eq!((cues[7], cues[15]), (1, 1));
        assert_eq!(cue, 0.3);
    }

    #[test]
    fn a_chart_with_one_long_section_still_cues_every_eighth_bar_and_a_section_resets_the_count() {
        let chart = sectioned();
        let rhythm = beating(120.0, 4.0);
        let mut rig = Rig::default();
        let beat = MusicValues {
            beats: 1,
            ..raw_bpm(120.0)
        };
        for _ in 0..40 {
            rig.step(beat, Some(&chart), &rhythm);
            rig.hold(raw_bpm(120.0), Some(&chart), &rhythm, 0.45);
        }
        let bar = MusicValues { bars: 1, ..beat };
        let mut cues = Vec::new();
        for _ in 0..8 {
            cues.push(rig.step(bar, Some(&chart), &rhythm).cues);
            rig.hold(raw_bpm(120.0), Some(&chart), &rhythm, 0.5);
        }
        assert_eq!(cues, [0, 0, 0, 0, 0, 0, 0, 1]);
        // Five bars, a section start (a cue of its own, 4 s on), then seven more bars: none yet,
        // the eighth since the section is a cue.
        for _ in 0..5 {
            rig.step(bar, Some(&chart), &rhythm);
            rig.hold(raw_bpm(120.0), Some(&chart), &rhythm, 0.9);
        }
        let line = MusicValues {
            sections: 1,
            section_energy: 0.5,
            ..bar
        };
        assert_eq!(rig.step(line, Some(&chart), &rhythm).cues, 1);
        let mut cues = Vec::new();
        for _ in 0..8 {
            rig.hold(raw_bpm(120.0), Some(&chart), &rhythm, 0.5);
            cues.push(rig.step(bar, Some(&chart), &rhythm).cues);
        }
        assert_eq!(cues, [0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn without_a_beat_or_a_chart_a_hit_after_a_long_quiet_is_a_faint_cue() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let hit = MusicValues {
            hits: 1,
            ..Default::default()
        };
        rig.step(MusicValues::default(), None, &nothing);
        let m = rig.step(hit, None, &nothing);
        assert_eq!((m.cues, m.cue), (1, 0.2));
        // Hits go on, and none is a cue for 16 s.
        for _ in 0..300 {
            assert_eq!(rig.step(hit, None, &nothing).cues, 0);
        }
        rig.hold(MusicValues::default(), None, &nothing, 2.0);
        assert_eq!(rig.step(hit, None, &nothing).cues, 1);
    }

    #[test]
    fn a_seek_or_the_chart_arriving_is_no_cue() {
        let chart = sectioned();
        let nothing = Rhythm::default();
        let mut rig = Rig::default();
        rig.step(MusicValues::default(), None, &nothing);
        // The chart arrives on a frame that also counts a section.
        let m = rig.step(section_line(0.9), Some(&chart), &nothing);
        assert_eq!(m.cues, 0);
        // The very first frame of all.
        let mut rig = Rig::default();
        assert_eq!(rig.step(section_line(0.9), Some(&chart), &nothing).cues, 0);
        // A seek.
        rig.hold(MusicValues::default(), Some(&chart), &nothing, 1.0);
        let m = rig.jump(section_line(0.9), Some(&chart), &nothing, 120.0);
        assert_eq!(m.cues, 0);
    }

    fn with_harmonic(count: usize, time: f64, strength: f64) -> ChartState {
        let mut chart = sectioned();
        chart.latest[3] = LatestOnset {
            count,
            time,
            strength,
        };
        chart
    }

    #[test]
    fn the_charts_harmonic_onsets_are_notes() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let raw = MusicValues::default();
        rig.step(raw, Some(&with_harmonic(0, 0.0, 0.0)), &nothing);
        let t = rig.time;
        let m = rig.step(raw, Some(&with_harmonic(1, t, 0.7)), &nothing);
        assert_eq!(m.notes, 1);
        assert!((m.note - 0.7).abs() < 1e-6);
        // The same onset is not new on the next frame, and the envelope fades.
        let m = rig.step(raw, Some(&with_harmonic(1, t, 0.7)), &nothing);
        assert_eq!(m.notes, 0);
        assert!(m.note < 0.7);
        // A weak one is an ornament; one 0.05 s after the last is too close.
        let t = rig.time;
        assert_eq!(
            rig.step(raw, Some(&with_harmonic(2, t, 0.1)), &nothing)
                .notes,
            0
        );
        assert_eq!(
            rig.step(raw, Some(&with_harmonic(3, t - 0.05, 0.9)), &nothing)
                .notes,
            0
        );
        let t = rig.time;
        assert_eq!(
            rig.step(raw, Some(&with_harmonic(4, t, 0.9)), &nothing)
                .notes,
            1
        );
        // A seek over onsets and the next frame's onset: only the latter.
        let m = rig.jump(raw, Some(&with_harmonic(9, 80.0, 0.9)), &nothing, 80.0);
        assert_eq!(m.notes, 0);
        let m = rig.step(raw, Some(&with_harmonic(10, 80.2, 0.9)), &nothing);
        assert_eq!(m.notes, 1);
    }

    #[test]
    fn without_a_chart_a_rise_of_the_mids_is_a_note_that_needs_to_arm_again() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let mids = |mids| MusicValues {
            mids,
            ..Default::default()
        };
        rig.hold(mids(1.0), None, &nothing, 2.0);
        let m = rig.step(mids(1.6), None, &nothing);
        assert_eq!(m.notes, 1);
        // Strength is the rise over twice the threshold.
        assert!((m.note - 0.6 / 0.7).abs() < 0.05, "{}", m.note);
        // Held up it is no new note, though the average has not caught up for a while.
        for _ in 0..10 {
            assert_eq!(rig.step(mids(1.6), None, &nothing).notes, 0);
        }
        // Down (arming again) and up: a second note.
        rig.hold(mids(1.0), None, &nothing, 3.0);
        assert_eq!(rig.step(mids(1.5), None, &nothing).notes, 1);
        // A small rise is none.
        rig.hold(mids(1.0), None, &nothing, 3.0);
        assert_eq!(rig.step(mids(1.2), None, &nothing).notes, 0);
    }

    #[test]
    fn the_bands_rise_fast_and_fall_slowly() {
        let mut rig = Rig::default();
        let nothing = Rhythm::default();
        let loud = MusicValues {
            bass: 1.0,
            ..Default::default()
        };
        rig.step(MusicValues::default(), None, &nothing);
        let m = rig.hold(loud, None, &nothing, 0.1);
        assert!(m.band_low > 0.9, "{}", m.band_low);
        assert_eq!((m.band_mid, m.band_high), (0.0, 0.0));
        let m = rig.hold(MusicValues::default(), None, &nothing, 0.1);
        assert!(m.band_low > 0.5 && m.band_low < 0.65, "{}", m.band_low);
    }

    #[test]
    fn a_hard_hit_in_a_loud_section_is_an_impact_and_two_are_six_seconds_apart() {
        let chart = sectioned();
        let nothing = Rhythm::default();
        let section = |low| MusicValues {
            section_energy: 0.8,
            next_energy: 0.8,
            section_left: 100.0,
            low,
            ..Default::default()
        };
        let mut rig = Rig::default();
        rig.hold(section(0.0), Some(&chart), &nothing, 1.0);
        let m = rig.step(section(1.0), Some(&chart), &nothing);
        assert_eq!((m.impacts, m.impact), (1, 1.0));
        // Held above the level, and a second hit 2 s later: not yet.
        assert_eq!(rig.step(section(1.0), Some(&chart), &nothing).impacts, 0);
        rig.hold(section(0.0), Some(&chart), &nothing, 2.0);
        assert_eq!(rig.step(section(1.0), Some(&chart), &nothing).impacts, 0);
        rig.hold(section(0.0), Some(&chart), &nothing, 6.0);
        assert_eq!(rig.step(section(0.97), Some(&chart), &nothing).impacts, 1);
        // Below the level: not a hit.
        rig.hold(section(0.0), Some(&chart), &nothing, 7.0);
        assert_eq!(rig.step(section(0.9), Some(&chart), &nothing).impacts, 0);
    }

    #[test]
    fn a_quiet_section_has_no_impacts_and_without_a_chart_the_kick_makes_them() {
        let chart = sectioned();
        let nothing = Rhythm::default();
        let quiet = |low| MusicValues {
            section_energy: 0.3,
            next_energy: 0.3,
            section_left: 100.0,
            low,
            ..Default::default()
        };
        let mut rig = Rig::default();
        rig.hold(quiet(0.0), Some(&chart), &nothing, 1.0);
        assert_eq!(rig.step(quiet(1.0), Some(&chart), &nothing).impacts, 0);
        // Live: loud (the drive follows the loudness) and a kick.
        let mut rig = Rig::default();
        let loud = MusicValues {
            loudness: 0.9,
            ..Default::default()
        };
        let kick = |kick| Rhythm {
            kick,
            ..Default::default()
        };
        rig.hold(loud, None, &kick(0.0), 1.0);
        let m = rig.step(loud, None, &kick(1.0));
        assert_eq!(m.impacts, 1);
        // A seek is not one.
        let mut rig = Rig::default();
        rig.hold(loud, None, &kick(0.0), 1.0);
        assert_eq!(rig.jump(loud, None, &kick(1.0), 500.0).impacts, 0);
    }

    #[test]
    fn music_fills_the_signals_in() {
        let mut events = EventCounter::default();
        let m = music(
            &Rhythm::default(),
            None,
            Levels::default(),
            (1.0, true),
            &mut events,
        );
        assert_eq!((m.tempo, m.meter, m.cue_in), (90.0, 4.0, -1.0));
        let rhythm = beating(128.0, 9.5);
        let m = music(&rhythm, None, Levels::default(), (1.1, true), &mut events);
        assert!(m.tempo >= 90.0 && m.tempo < 128.0);
        assert!((m.clock.fract() - 0.5).abs() < 1e-3);
    }
}
