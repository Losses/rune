//! The camera director: decides when a scene's view cuts to another shot and to which, following
//! the music, and tells the scene through `struct Shot` (see `docs/music.md`, "Shots and the
//! director"). A scene declares its shots in `scene.toml`; the director is per scene (it needs the
//! scene's manifest and counts from the scene's start). It is a pure state machine of the music's
//! values ([`MusicValues`]) and the time step, so it can be tested without a scene or a GPU.

use super::inputs::MusicValues;
use super::manifest::{Camera, Manifest, Pace, Shot, ShotMove, ShotSize};
use crate::switching::{FOLLOWED, SECTION_CUE};

/// A section whose energy is below this is quiet.
pub const QUIET_BELOW: f32 = 0.35;
/// A section whose energy is above this is high.
pub const HIGH_ABOVE: f32 = 0.65;
// The cadence below is half of the first one, so that cuts are not too frequent.
/// With a followed beat: bars between two cuts in a quiet section.
pub const QUIET_BARS: u32 = 8;
/// With a followed beat: bars between two cuts in a section that is neither quiet nor high.
pub const NORMAL_BARS: u32 = 4;
/// With a followed beat: bars between two cuts in a high section.
pub const HIGH_BARS: u32 = 2;
/// Without a followed beat: seconds between two cuts in a quiet section.
pub const QUIET_SECONDS: f32 = 24.0;
/// Without a followed beat: seconds between two cuts in a section that is neither quiet nor high.
pub const NORMAL_SECONDS: f32 = 12.0;
/// Without a followed beat: seconds between two cuts in a high section.
pub const HIGH_SECONDS: f32 = 6.0;
/// Without a followed beat: how long past the time a cut is due it waits for a hit, then cuts at
/// once.
pub const HIT_GRACE_SECONDS: f32 = 2.0;
/// How many beats the burst of cuts before a section or a phrase takes: one cut on each.
pub const FILL_BEATS: u32 = 4;
/// How much more energy the next section needs for the burst before it (the cue's own rule for a
/// lift is the same, `scene::signals::CUE_LIFT`).
pub const FILL_LIFT: f32 = 0.1;
/// Without a followed beat, how long a burst a scene lab started waits between its cuts.
pub const FORCED_BURST_SECONDS: f32 = 0.25;
/// The length of a phrase in a high section, in bars from the section's start.
pub const PHRASE_BARS: u32 = 8;
/// Whether the end of every phrase of a high section gets a burst of cuts. Off: fewer cuts are
/// wanted, and the burst before an energy lift (as in the reference) stays. A scene of `pace =
/// "busy"` has them all the same.
pub const PHRASE_BURSTS: bool = false;
/// The share of the dutch angle a shot with seed 0 gets; the seed scales the rest.
pub const ROLL_LEAST: f32 = 0.5;
/// The farthest apart two sizes are when one of them is `top` and they differ: `top` is never
/// the nearest size to another one.
const TOP_DISTANCE: u32 = 10;

/// The paces a scene lab steps through, from the least often cutting to the most (after the
/// scene's own).
pub const PACE_CYCLE: [Pace; 4] = [Pace::Sections, Pace::Calm, Pace::Normal, Pace::Busy];

/// The sizes of the burst's cuts, one per beat. A scene without `top` gets `wide` for the last.
const FILL_SIZES: [ShotSize; FILL_BEATS as usize] = [
    ShotSize::Medium,
    ShotSize::Close,
    ShotSize::Detail,
    ShotSize::Top,
];
/// The sizes of a high section after it reached `close`, one per cut in turn.
const HIGH_SIZES: [ShotSize; 4] = [
    ShotSize::Detail,
    ShotSize::Medium,
    ShotSize::Close,
    ShotSize::Medium,
];
/// The sizes of an ordinary section after its establishing shot, one per cut in turn.
const NORMAL_SIZES: [ShotSize; 3] = [ShotSize::Medium, ShotSize::Close, ShotSize::Detail];

/// How lively the section is, from its energy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    Quiet,
    Normal,
    High,
}

impl Tier {
    /// The energy of the section: the chart's, else the music's slow energy.
    fn of(music: &MusicValues) -> Self {
        let energy = if music.has_chart != 0 {
            music.section_energy
        } else {
            music.drive
        };
        if energy < QUIET_BELOW {
            Self::Quiet
        } else if energy > HIGH_ABOVE {
            Self::High
        } else {
            Self::Normal
        }
    }

    /// The tier's name, for a readout.
    fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    fn bars(self) -> u32 {
        match self {
            Self::Quiet => QUIET_BARS,
            Self::Normal => NORMAL_BARS,
            Self::High => HIGH_BARS,
        }
    }

    fn seconds(self) -> f32 {
        match self {
            Self::Quiet => QUIET_SECONDS,
            Self::Normal => NORMAL_SECONDS,
            Self::High => HIGH_SECONDS,
        }
    }

    /// The size of the cut that is the `step`th since the section's start (step 0 is the
    /// establishing shot, which is always wide). A quiet section alternates wide and medium, an
    /// ordinary one steps closer, and a high one steps closer to close and then alternates.
    fn size(self, step: u32) -> ShotSize {
        if step == 0 {
            return ShotSize::Wide;
        }
        match self {
            Self::Quiet => {
                if step.is_multiple_of(2) {
                    ShotSize::Wide
                } else {
                    ShotSize::Medium
                }
            }
            Self::Normal => NORMAL_SIZES[(step as usize - 1) % NORMAL_SIZES.len()],
            Self::High => match step {
                1 => ShotSize::Medium,
                2 => ShotSize::Close,
                _ => HIGH_SIZES[(step as usize - 3) % HIGH_SIZES.len()],
            },
        }
    }
}

/// A cut the director made in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    /// The `[[shot]]` it cut to.
    pub index: u32,
    pub size: ShotSize,
    /// The shot's move, but always `Cut` inside a burst: a burst's cuts are hard.
    pub motion: ShotMove,
    /// 0 for a normal cut, else the position in a burst (1 to [`FILL_BEATS`]).
    pub fill: u32,
}

/// What the camera director tells the scene about the current shot: `struct Shot`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotState {
    /// Which `[[shot]]`, from 0.
    pub index: u32,
    /// How many cuts so far.
    pub cuts: u32,
    /// The size of the shot: 0 wide, 1 medium, 2 close, 3 detail, 4 top.
    pub size: u32,
    /// The dutch angle of this shot in radians, signed.
    pub roll: f32,
    /// 0 to 1, fixed for the shot.
    pub seed: f32,
    /// Seconds since the shot began.
    pub age: f32,
    /// The planned seconds of the shot, -1 when unknown.
    pub length: f32,
    /// 0 normally, else the position in a burst.
    pub fill: u32,
}

impl ShotState {
    /// The layout of `struct Shot`: eight words (32 bytes, a multiple of 16).
    pub fn bytes(&self) -> [u8; 32] {
        let words = [
            self.index.to_le_bytes(),
            self.cuts.to_le_bytes(),
            self.size.to_le_bytes(),
            self.roll.to_le_bytes(),
            self.seed.to_le_bytes(),
            self.age.to_le_bytes(),
            self.length.to_le_bytes(),
            self.fill.to_le_bytes(),
        ];
        let mut bytes = [0; 32];
        for (i, word) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(word);
        }
        bytes
    }
}

/// What a readout can show of a director: its state and the names of the shot and the section's
/// tier (see `Director::view`).
#[derive(Clone, Debug, PartialEq)]
pub struct DirectorView {
    pub state: ShotState,
    /// The `name` of the shot on, as the scene declares it (empty if it has none).
    pub shot_name: String,
    /// "quiet", "normal" or "high": how lively the music's section is, which sets how often the
    /// director cuts.
    pub tier: &'static str,
    /// Whether the shot is held (see `Director::hold_shot`), so that the director does not cut.
    pub held: bool,
    /// The scene's shots in order: the name (empty if it has none) and the size.
    pub shots: Vec<(String, ShotSize)>,
    /// The pace in effect and the scene's own, and whether a lab overrides it.
    pub pace: Pace,
    pub own_pace: Pace,
    pub pace_overridden: bool,
}

/// Why a burst cut is wanted now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Burst {
    /// Before a section that lifts the energy.
    Lift,
    /// Before the end of a phrase in a high section.
    Phrase,
}

/// A burst a scene lab started by hand (see [`Director::start_burst`]), under way.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ForcedBurst {
    /// The position (1 to [`FILL_BEATS`]) of the next cut.
    next: u32,
    /// Without a followed beat: seconds until the next cut.
    wait: f32,
}

/// The number `rv_label_hash` in `stdlib.wgsl` gives: 0 to 1, the same for the same label and `k`.
fn label_hash(label: f32, k: u32) -> f32 {
    let multiplier = [0.618_033_9, 0.754_877_7, 0.569_840_3][(k % 3) as usize];
    ((label + 1.0) * multiplier + (k / 3) as f32 * 0.314_159_3).fract()
}

/// How far apart two sizes are, for finding the nearest size a scene has.
fn distance(a: ShotSize, b: ShotSize) -> u32 {
    if a == b {
        0
    } else if a == ShotSize::Top || b == ShotSize::Top {
        TOP_DISTANCE
    } else {
        (a as u32).abs_diff(b as u32)
    }
}

/// Decides the camera cuts of one scene from the music.
pub struct Director {
    camera: Camera,
    /// The scene's shots, in the order of its `[[shot]]`s.
    shots: Vec<(ShotSize, ShotMove)>,
    /// The `name` of each shot, in the same order.
    names: Vec<String>,
    /// Whether the scene has a `close` or a `detail` shot, which the bursts need.
    has_near: bool,
    state: ShotState,
    /// The next tilted shot leans to the right (positive roll).
    tilt_right: bool,
    /// Cuts since the section started, counting its establishing shot.
    in_section: u32,
    /// Bars since the section started, counted from its start.
    bars_in_section: u32,
    /// Bars and seconds since the last cut.
    bars_since_cut: u32,
    seconds_since_cut: f32,
    /// Which beat of the bar the last beat was (0 on the downbeat); none before the first
    /// downbeat.
    beat_in_bar: Option<u32>,
    /// The position of the last cut in a burst, 0 when the last cut was not in one.
    last_fill: u32,
    /// After a burst before a phrase's end: the bar count (from the section's start) at which
    /// the next phrase starts.
    phrase_start: Option<u32>,
    /// Whether the end of a phrase gets a burst (from [`PHRASE_BURSTS`], or a busy pace).
    phrase_bursts: bool,
    /// A pace a scene lab asked for in place of the scene's own (see [`Self::cycle_pace`]).
    pace_override: Option<Pace>,
    /// The shot is held: no cut happens until [`Director::release`] (for a scene lab).
    held: bool,
    /// A burst a scene lab asked for, while its cuts are still to come.
    forced_burst: Option<ForcedBurst>,
}

impl Director {
    /// The director of a scene with shots; none for a scene without (it works as it did before
    /// there were shots).
    pub fn for_manifest(manifest: &Manifest) -> Option<Self> {
        (!manifest.shots.is_empty()).then(|| Self::new(manifest.camera, &manifest.shots))
    }

    /// A director over `shots`, which must not be empty. It starts on a wide shot (or the
    /// nearest to one), before any cut.
    pub fn new(camera: Camera, shots: &[Shot]) -> Self {
        assert!(!shots.is_empty(), "a director needs a shot");
        let names = shots.iter().map(|s| s.name.clone()).collect();
        let shots: Vec<(ShotSize, ShotMove)> = shots.iter().map(|s| (s.size, s.motion)).collect();
        let has_near = shots
            .iter()
            .any(|(size, _)| matches!(size, ShotSize::Close | ShotSize::Detail));
        let mut director = Self {
            camera,
            shots,
            names,
            has_near,
            state: ShotState {
                index: 0,
                cuts: 0,
                size: 0,
                roll: 0.0,
                seed: 0.0,
                age: 0.0,
                length: -1.0,
                fill: 0,
            },
            tilt_right: false,
            in_section: 0,
            bars_in_section: 0,
            bars_since_cut: 0,
            seconds_since_cut: 0.0,
            beat_in_bar: None,
            last_fill: 0,
            phrase_start: None,
            phrase_bursts: PHRASE_BURSTS || camera.pace == Pace::Busy,
            pace_override: None,
            held: false,
            forced_burst: None,
        };
        let index = director.pick(ShotSize::Wide, 0.0, 0, None);
        director.state.index = index as u32;
        director.state.size = director.shots[index].0 as u32;
        director.state.seed = label_hash(0.0, 0);
        director
    }

    /// The pace in effect: the lab's override, else the scene's own.
    pub fn pace(&self) -> Pace {
        self.pace_override.unwrap_or(self.camera.pace)
    }

    /// The pace override, to give a rebuilt director (see [`Self::set_pace_override`]).
    pub fn pace_override(&self) -> Option<Pace> {
        self.pace_override
    }

    /// Use `pace` in place of the scene's own, or with none the scene's own again. The cadence
    /// counts on from the last cut, so a shorter pace may cut at once.
    pub fn set_pace_override(&mut self, pace: Option<Pace>) {
        self.pace_override = pace;
        self.phrase_bursts = PHRASE_BURSTS || self.pace() == Pace::Busy;
    }

    /// Step to the next pace for a scene lab: the scene's own, then [`PACE_CYCLE`], then the
    /// scene's own again.
    pub fn cycle_pace(&mut self) {
        let next = match self.pace_override {
            None => PACE_CYCLE.first().copied(),
            Some(pace) => PACE_CYCLE
                .iter()
                .position(|p| *p == pace)
                .and_then(|i| PACE_CYCLE.get(i + 1))
                .copied(),
        };
        self.set_pace_override(next);
    }

    /// The scene's shots as a readout lists them: the name (empty if it has none) and the size.
    pub fn shot_list(&self) -> Vec<(String, ShotSize)> {
        self.names
            .iter()
            .zip(&self.shots)
            .map(|(name, (size, _))| (name.clone(), *size))
            .collect()
    }

    /// What the scene is told now.
    pub fn state(&self) -> &ShotState {
        &self.state
    }

    /// What a readout shows: the state, the shot's name, the section's tier (from `music`) and
    /// whether the shot is held.
    pub fn view(&self, music: &MusicValues) -> DirectorView {
        DirectorView {
            state: self.state,
            shot_name: self.names[self.state.index as usize].clone(),
            tier: Tier::of(music).name(),
            held: self.held,
            shots: self.shot_list(),
            pace: self.pace(),
            own_pace: self.camera.pace,
            pace_overridden: self.pace_override.is_some(),
        }
    }

    /// Cut now, as if a cut were due: to the size the section's tier gives for the next cut, by
    /// the same rules as the cadence, and tell the scene. It is what a scene lab asks for; the
    /// cadence counts from this cut on. A held shot is cut away from (and stays held).
    pub fn force_cut(&mut self, music: &MusicValues) -> Cut {
        let tier = Tier::of(music);
        self.cut(tier.size(self.in_section), music, tier, 0)
    }

    /// Start a burst of cuts now, as the one before a lift is (see [`FILL_SIZES`]): a cut on
    /// each of the next [`FILL_BEATS`] beats, or without a followed beat one every
    /// [`FORCED_BURST_SECONDS`], the first at once. It is what a scene lab asks for, to see a
    /// burst without waiting for a lift; the cadence is quiet until it is over, and a section
    /// start still cuts as it does. A held shot defers it until the shot is let go of.
    pub fn start_burst(&mut self) {
        self.forced_burst = Some(ForcedBurst { next: 1, wait: 0.0 });
    }

    /// Cut to shot `index` (of the scene's `[[shot]]`s) and hold it: the director cuts no more
    /// until [`Self::release_shot`]. The shot's size and roll follow the rules of any cut. Returns
    /// the cut, or none for a shot the scene does not have (nothing changes then).
    pub fn hold_shot(&mut self, index: u32, music: &MusicValues) -> Option<Cut> {
        let index = index as usize;
        if index >= self.shots.len() {
            return None;
        }
        let cut = self.enter(index, music, Tier::of(music), 0);
        self.held = true;
        Some(cut)
    }

    /// Let go of a held shot: the director cuts by its cadence again, counted from now so that it
    /// does not cut at once.
    pub fn release_shot(&mut self) {
        if self.held {
            self.held = false;
            self.bars_since_cut = 0;
            self.seconds_since_cut = 0.0;
        }
    }

    /// Advance by one frame of `dt` seconds with the music of the frame, and return the cut that
    /// happened in it, if any.
    pub fn tick(&mut self, dt: f32, music: &MusicValues) -> Option<Cut> {
        self.state.age += dt;
        self.seconds_since_cut += dt;
        self.bars_since_cut += music.bars;
        self.bars_in_section += music.bars;
        if music.bars > 0 {
            self.beat_in_bar = Some(0);
        } else if music.beats > 0 {
            self.beat_in_bar = self.beat_in_bar.map(|beat| beat + music.beats);
        }
        let tier = Tier::of(music);
        let followed = music.locked > FOLLOWED;
        if self.held {
            return None;
        }

        // A section starts: establish it, and count the progression again.
        if music.cues > 0 && music.cue >= SECTION_CUE {
            self.in_section = 0;
            self.bars_in_section = 0;
            self.phrase_start = None;
            return Some(self.cut(ShotSize::Wide, music, tier, 0));
        }
        // A phrase that was announced by a burst starts: the same.
        if let Some(start) = self.phrase_start
            && music.bars > 0
            && self.bars_in_section >= start
        {
            self.in_section = 0;
            self.phrase_start = None;
            return Some(self.cut(ShotSize::Wide, music, tier, 0));
        }
        // A burst a scene lab started.
        if let Some(burst) = &mut self.forced_burst {
            let due = if followed && music.tempo > 0.0 {
                music.beats > 0
            } else {
                burst.wait -= dt;
                burst.wait <= 0.0
            };
            if !due {
                return None;
            }
            let fill = burst.next;
            burst.next += 1;
            burst.wait = FORCED_BURST_SECONDS;
            if fill >= FILL_BEATS {
                self.forced_burst = None;
            }
            return Some(self.cut(FILL_SIZES[fill as usize - 1], music, tier, fill));
        }
        // The burst before a section or a phrase ends.
        if let Some((kind, fill)) = self.burst(music, tier, followed)
            && fill > self.last_fill
        {
            if kind == Burst::Phrase {
                self.phrase_start = Some(self.bars_in_section + 1);
            }
            let size = FILL_SIZES[fill as usize - 1];
            return Some(self.cut(size, music, tier, fill));
        }
        // The cadence of the section.
        let due = if followed {
            self.bars(tier)
                .is_some_and(|bars| music.bars > 0 && self.bars_since_cut >= bars)
        } else {
            self.seconds(tier, music).is_some_and(|wait| {
                self.seconds_since_cut >= wait
                    && (music.hits > 0 || self.seconds_since_cut >= wait + HIT_GRACE_SECONDS)
            })
        };
        due.then(|| self.cut(tier.size(self.in_section), music, tier, 0))
    }

    /// Bars between two cuts with a followed beat: the section's at the scene's pace, but at
    /// least the scene's `hold`. None for a scene that cuts only at a section's start.
    fn bars(&self, tier: Tier) -> Option<u32> {
        let bars = match self.pace() {
            Pace::Sections => return None,
            Pace::Calm => tier.bars() * 2,
            Pace::Normal => tier.bars(),
            Pace::Busy => (tier.bars() / 2).max(1),
        };
        Some(bars.max(self.camera.hold))
    }

    /// Seconds between two cuts without a followed beat: the section's at the scene's pace, but
    /// at least the scene's `hold` in bars at the music's tempo (4 beats to a bar when the meter
    /// is unknown). None for a scene that cuts only at a section's start.
    fn seconds(&self, tier: Tier, music: &MusicValues) -> Option<f32> {
        let seconds = match self.pace() {
            Pace::Sections => return None,
            Pace::Calm => tier.seconds() * 2.0,
            Pace::Normal => tier.seconds(),
            Pace::Busy => tier.seconds() / 2.0,
        };
        let meter = if music.meter > 0.0 { music.meter } else { 4.0 };
        let held = if music.tempo > 0.0 {
            self.camera.hold as f32 * meter * 60.0 / music.tempo
        } else {
            0.0
        };
        Some(seconds.max(held))
    }

    /// Whether this frame is a beat in the last [`FILL_BEATS`] before a place worth a burst, and
    /// which one of the burst it is (1 for the farthest). Only with the song's analysis, a
    /// followed beat, `fill` declared and a near shot to cut to.
    fn burst(&self, music: &MusicValues, tier: Tier, followed: bool) -> Option<(Burst, u32)> {
        if !self.camera.fill
            || !self.has_near
            || music.has_chart == 0
            || !followed
            || music.beats == 0
            || music.tempo <= 0.0
        {
            return None;
        }
        let position = |left: f32| {
            let left = left.round();
            (1.0..=FILL_BEATS as f32)
                .contains(&left)
                .then(|| FILL_BEATS - left as u32 + 1)
        };
        if music.cue_in >= 0.0
            && music.next_energy - music.section_energy >= FILL_LIFT
            && let Some(fill) = position(music.cue_in * music.tempo / 60.0)
        {
            return Some((Burst::Lift, fill));
        }
        if self.phrase_bursts
            && tier == Tier::High
            && self.bars_in_section % PHRASE_BARS == PHRASE_BARS - 1
            && let Some(beat) = self.beat_in_bar
            && let Some(fill) = position(music.meter.round() - beat as f32)
        {
            return Some((Burst::Phrase, fill));
        }
        None
    }

    /// Cut to a shot of about this size, other than the one on, and tell the scene.
    fn cut(&mut self, want: ShotSize, music: &MusicValues, tier: Tier, fill: u32) -> Cut {
        let label = music.section_label;
        let current = self.state.index as usize;
        let index = self.pick(want, label, self.in_section, Some(current));
        self.enter(index, music, tier, fill)
    }

    /// Cut to shot `index`: count the cut, give the shot its roll and seed, and say how long it
    /// is planned to last.
    fn enter(&mut self, index: usize, music: &MusicValues, tier: Tier, fill: u32) -> Cut {
        let label = music.section_label;
        let (size, motion) = self.shots[index];
        self.state.cuts = self.state.cuts.wrapping_add(1);
        self.in_section += 1;
        let seed = label_hash(label, self.state.cuts.wrapping_add(7));
        let roll = if matches!(size, ShotSize::Wide | ShotSize::Top) {
            0.0
        } else {
            self.tilt_right = !self.tilt_right;
            let side = if self.tilt_right { 1.0 } else { -1.0 };
            side * self.camera.dutch * (ROLL_LEAST + (1.0 - ROLL_LEAST) * seed)
        };
        let beat = 60.0 / music.tempo.max(1.0);
        self.state = ShotState {
            index: index as u32,
            cuts: self.state.cuts,
            size: size as u32,
            roll,
            seed,
            age: 0.0,
            length: match (music.locked > FOLLOWED, fill) {
                (true, 0) => self
                    .bars(tier)
                    .map_or(-1.0, |bars| (bars as f32 * music.meter * beat).max(0.0)),
                (true, _) => beat,
                (false, _) => self.seconds(tier, music).unwrap_or(-1.0),
            },
            fill,
        };
        self.bars_since_cut = 0;
        self.seconds_since_cut = 0.0;
        self.last_fill = fill;
        Cut {
            index: index as u32,
            size,
            motion: if fill > 0 { ShotMove::Cut } else { motion },
            fill,
        }
    }

    /// The shot to cut to for a size: the nearest size the scene has among the shots other than
    /// `not` (all of them if the scene has just one), and among those the one the section's label
    /// and `k` hash to, so that a section of the same kind picks the same. A scene without `top`
    /// gets a wide shot in its place.
    fn pick(&self, want: ShotSize, label: f32, k: u32, not: Option<usize>) -> usize {
        let want = if want == ShotSize::Top && self.shots.iter().all(|(s, _)| *s != ShotSize::Top) {
            ShotSize::Wide
        } else {
            want
        };
        let allowed = |i: &usize| not != Some(*i) || self.shots.len() == 1;
        let nearest = (0..self.shots.len())
            .filter(allowed)
            .map(|i| distance(self.shots[i].0, want))
            .min()
            .unwrap_or(0);
        let tied: Vec<usize> = (0..self.shots.len())
            .filter(allowed)
            .filter(|&i| distance(self.shots[i].0, want) == nearest)
            .collect();
        let at = (label_hash(label, k) * tied.len() as f32) as usize;
        tied[at.min(tied.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(size: ShotSize, motion: ShotMove) -> Shot {
        Shot {
            name: String::new(),
            size,
            motion,
        }
    }

    /// One shot of each size, in the order wide, medium, close, detail, top, and a second wide
    /// one (a section can then start wide after another one ended wide).
    fn each_size() -> Vec<Shot> {
        use ShotSize::*;
        [Wide, Medium, Close, Detail, Top, Wide]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec()
    }

    fn held(hold: u32) -> Camera {
        Camera {
            hold,
            ..camera(0.0, false)
        }
    }

    fn camera(dutch: f32, fill: bool) -> Camera {
        Camera {
            dutch,
            fill,
            hold: 0,
            pace: Pace::Normal,
        }
    }

    /// A song at 120 bpm (a beat is 0.5 s, a bar 2 s) in 4/4 with a chart, the beat followed, in
    /// a section of this energy that nothing is coming after.
    fn song(energy: f32) -> MusicValues {
        MusicValues {
            has_chart: 1,
            tempo: 120.0,
            meter: 4.0,
            locked: 1.0,
            drive: energy,
            section_energy: energy,
            next_energy: energy,
            cue_in: -1.0,
            ..MusicValues::default()
        }
    }

    /// The frame that starts a section with this label.
    fn start(label: f32, base: MusicValues) -> MusicValues {
        MusicValues {
            section_label: label,
            sections: 1,
            cues: 1,
            cue: 1.0,
            ..base
        }
    }

    /// Feed one frame per beat of `base`, `count` beats from beat `from` (a beat that is a
    /// multiple of 4 is a downbeat), with `with` changing each beat's frame. Returns each cut
    /// and the beat it happened on.
    fn beats(
        director: &mut Director,
        from: u32,
        count: u32,
        base: &MusicValues,
        with: impl Fn(u32, MusicValues) -> MusicValues,
    ) -> Vec<(u32, Cut)> {
        (from..from + count)
            .filter_map(|beat| {
                let frame = MusicValues {
                    beats: 1,
                    bars: u32::from(beat % 4 == 0),
                    ..*base
                };
                let frame = with(beat, frame);
                director.tick(0.5, &frame).map(|cut| (beat, cut))
            })
            .collect()
    }

    fn plain(_: u32, m: MusicValues) -> MusicValues {
        m
    }

    #[test]
    fn a_scene_without_shots_has_no_director() {
        let manifest = Manifest::parse(
            "format = 1\nname = \"T\"\nlicense = \"MPL-2.0\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n",
        )
        .unwrap();
        assert!(Director::for_manifest(&manifest).is_none());
        let with = Manifest::parse(
            "format = 1\nname = \"T\"\nlicense = \"MPL-2.0\"\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[[shot]]\nsize = \"wide\"\n",
        )
        .unwrap();
        assert!(Director::for_manifest(&with).is_some());
    }

    #[test]
    fn the_state_has_the_layout_of_struct_shot() {
        let state = ShotState {
            index: 1,
            cuts: 2,
            size: 3,
            roll: 0.5,
            seed: 0.25,
            age: 1.5,
            length: -1.0,
            fill: 4,
        };
        let bytes = state.bytes();
        assert_eq!(bytes.len() as u64, crate::scene::wgsl::SHOT_SIZE);
        let word = |i: usize| <[u8; 4]>::try_from(&bytes[i * 4..i * 4 + 4]).unwrap();
        assert_eq!(u32::from_le_bytes(word(1)), 2);
        assert_eq!(f32::from_le_bytes(word(3)), 0.5);
        assert_eq!(f32::from_le_bytes(word(6)), -1.0);
        assert_eq!(u32::from_le_bytes(word(7)), 4);
    }

    #[test]
    fn it_starts_on_a_wide_shot_before_any_cut() {
        let director = Director::new(camera(0.3, false), &each_size());
        let state = director.state();
        assert_eq!((state.size, state.cuts), (0, 0));
        assert_eq!((state.roll, state.length, state.fill), (0.0, -1.0, 0));
    }

    /// The number of cuts in 16 bars after the first cut of a section of this energy.
    fn cuts_in_16_bars(energy: f32) -> usize {
        paced_cuts_in_16_bars(Pace::Normal, energy)
    }

    fn paced_cuts_in_16_bars(pace: Pace, energy: f32) -> usize {
        let camera = Camera {
            pace,
            ..camera(0.0, false)
        };
        let mut director = Director::new(camera, &each_size());
        let base = song(energy);
        // Beat 1 starts the section (not on a downbeat); the bars count from beat 4.
        beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
        beats(&mut director, 2, 64, &base, plain).len()
    }

    #[test]
    fn the_scene_pace_scales_the_bars_between_cuts() {
        use Pace::*;
        let counts = |energy| [Calm, Normal, Busy].map(|pace| paced_cuts_in_16_bars(pace, energy));
        assert_eq!(counts(0.2), [1, 2, 4]);
        assert_eq!(counts(0.5), [2, 4, 8]);
        assert_eq!(counts(0.8), [4, 8, 16]);
        // Only at a section's start: none within it, whatever the energy.
        assert_eq!(paced_cuts_in_16_bars(Sections, 0.9), 0);
    }

    #[test]
    fn a_scene_of_sections_cuts_at_the_next_section_and_without_a_beat_never_on_time() {
        let camera = Camera {
            pace: Pace::Sections,
            ..camera(0.0, false)
        };
        let mut director = Director::new(camera, &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(director.state().length, -1.0);
        assert!(beats(&mut director, 1, 40, &base, plain).is_empty());
        let next = beats(&mut director, 41, 1, &base, |_, m| start(1.0, m));
        assert_eq!(next.len(), 1);
        let unfollowed = MusicValues {
            locked: 0.0,
            hits: 1,
            ..base
        };
        for _ in 0..600 {
            assert!(director.tick(0.1, &unfollowed).is_none());
        }
    }

    #[test]
    fn a_busy_scene_bursts_at_the_end_of_a_phrase() {
        let busy = Camera {
            pace: Pace::Busy,
            ..camera(0.3, true)
        };
        assert!(Director::new(busy, &each_size()).phrase_bursts);
        assert!(!Director::new(camera(0.3, true), &each_size()).phrase_bursts);
    }

    #[test]
    fn cuts_follow_the_bars_of_the_section_energy() {
        assert_eq!(cuts_in_16_bars(0.2), 2);
        assert_eq!(cuts_in_16_bars(0.5), 4);
        assert_eq!(cuts_in_16_bars(0.8), 8);
    }

    #[test]
    fn cuts_land_on_downbeats() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.8);
        beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
        let cuts = beats(&mut director, 2, 40, &base, plain);
        assert!(cuts.len() >= 5);
        assert!(cuts.iter().all(|(beat, _)| beat % 4 == 0));
    }

    /// Frames of 0.1 s with a hit every `hit_every` frames (0 for none), without a beat, for
    /// `seconds`; the seconds each cut happened at.
    fn unfollowed(energy: f32, hit_every: u32, seconds: f32) -> Vec<f32> {
        unfollowed_held(0, energy, hit_every, seconds)
    }

    fn unfollowed_held(hold: u32, energy: f32, hit_every: u32, seconds: f32) -> Vec<f32> {
        let mut director = Director::new(held(hold), &each_size());
        let base = MusicValues {
            locked: 0.0,
            has_chart: 0,
            ..song(energy)
        };
        let mut cuts = Vec::new();
        for frame in 1..=(seconds * 10.0) as u32 {
            let hits = u32::from(hit_every > 0 && frame % hit_every == 0);
            let music = MusicValues { hits, ..base };
            if director.tick(0.1, &music).is_some() {
                cuts.push(frame as f32 / 10.0);
            }
        }
        cuts
    }

    #[test]
    fn without_a_beat_cuts_wait_for_a_hit() {
        // A hit every 0.7 s: quiet 24 s lands on the first hit after (24.5 s), and so on.
        let quiet = unfollowed(0.2, 7, 26.0);
        assert_eq!(quiet.len(), 1, "{quiet:?}");
        assert!((quiet[0] - 24.5).abs() < 0.11, "{quiet:?}");
        let normal = unfollowed(0.5, 7, 13.0);
        assert!((normal[0] - 12.6).abs() < 0.11, "{normal:?}");
        let high = unfollowed(0.8, 7, 13.0);
        assert!((high[0] - 6.3).abs() < 0.11, "{high:?}");
        assert_eq!(high.len(), 2, "{high:?}");
    }

    #[test]
    fn without_a_beat_or_a_hit_a_cut_waits_two_seconds_more() {
        let cuts = unfollowed(0.8, 0, 20.0);
        assert!((cuts[0] - 8.0).abs() < 0.11, "{cuts:?}");
        assert!((cuts[1] - cuts[0] - 8.0).abs() < 0.11, "{cuts:?}");
        let cuts = unfollowed(0.2, 0, 30.0);
        assert!((cuts[0] - 26.0).abs() < 0.11, "{cuts:?}");
    }

    #[test]
    fn without_a_chart_the_drive_gives_the_energy() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = MusicValues {
            has_chart: 0,
            section_energy: 0.0,
            drive: 0.9,
            ..song(0.9)
        };
        // High by `drive` although the section energy is 0: a cut every 2 bars.
        let cuts = beats(&mut director, 1, 16, &base, plain);
        assert_eq!(cuts.len(), 2);
    }

    #[test]
    fn a_section_starts_wide_and_steps_closer() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.5);
        let first = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(first[0].1.size, ShotSize::Wide);
        let sizes: Vec<ShotSize> = beats(&mut director, 1, 96, &base, plain)
            .into_iter()
            .map(|(_, cut)| cut.size)
            .collect();
        use ShotSize::*;
        assert_eq!(sizes, [Medium, Close, Detail, Medium, Close, Detail]);
    }

    #[test]
    fn a_new_section_starts_the_progression_again() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let before = beats(&mut director, 1, 32, &base, plain);
        assert_eq!(before.len(), 2);
        assert_eq!(before[1].1.size, ShotSize::Close);
        let again = beats(&mut director, 33, 1, &base, |_, m| start(1.0, m));
        assert_eq!(again[0].1.size, ShotSize::Wide);
        let next = beats(&mut director, 34, 15, &base, plain);
        assert_eq!(next[0].1.size, ShotSize::Medium);
    }

    #[test]
    fn a_weak_cue_is_not_a_section_start() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.2);
        let cuts = beats(&mut director, 1, 12, &base, |_, m| MusicValues {
            cues: 1,
            cue: 0.3,
            ..m
        });
        assert!(cuts.is_empty());
    }

    #[test]
    fn a_quiet_section_alternates_wide_and_medium() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let sizes: Vec<ShotSize> = beats(&mut director, 1, 100, &base, plain)
            .into_iter()
            .map(|(_, cut)| cut.size)
            .collect();
        use ShotSize::*;
        assert_eq!(sizes, [Medium, Wide, Medium].repeat(2)[..sizes.len()]);
        assert!(sizes.len() >= 3);
    }

    #[test]
    fn a_high_section_steps_closer_and_then_alternates() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.9);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let sizes: Vec<ShotSize> = beats(&mut director, 1, 60, &base, plain)
            .into_iter()
            .map(|(_, cut)| cut.size)
            .collect();
        use ShotSize::*;
        assert_eq!(
            sizes[..7],
            [Medium, Close, Detail, Medium, Close, Medium, Detail]
        );
        assert!(sizes.iter().all(|s| !matches!(s, Wide | Top)));
    }

    #[test]
    fn a_missing_size_is_the_nearest_one() {
        use ShotSize::*;
        // Only wide and detail: medium goes to wide, close to detail.
        let shots = [Wide, Detail]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        let director = Director::new(camera(0.0, false), &shots);
        assert_eq!(director.pick(Medium, 0.0, 0, None), 0);
        assert_eq!(director.pick(Close, 0.0, 0, None), 1);
        // Top is never the nearest to another size, and is wide where missing.
        let with_top = [Top, Wide].map(|size| shot(size, ShotMove::Cut)).to_vec();
        let director = Director::new(camera(0.0, false), &with_top);
        assert_eq!(director.pick(Detail, 0.0, 0, None), 1);
        assert_eq!(director.pick(Top, 0.0, 0, None), 0);
        let director = Director::new(camera(0.0, false), &shots);
        assert_eq!(director.pick(Top, 0.0, 0, None), 0);
    }

    #[test]
    fn a_shot_is_never_twice_in_a_row() {
        // Three medium shots and one wide: every size wanted has several or one choice.
        use ShotSize::*;
        let shots = [Wide, Medium, Medium, Close, Close, Detail]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        for energy in [0.2, 0.5, 0.9] {
            let mut director = Director::new(camera(0.3, true), &shots);
            let base = song(energy);
            let mut last = director.state().index;
            let mut cuts = 0;
            for section in 0..6 {
                let mut all = beats(&mut director, section * 40, 1, &base, |_, m| {
                    start(section as f32 % 3.0, m)
                });
                all.extend(beats(&mut director, section * 40 + 1, 39, &base, plain));
                for (_, cut) in all {
                    assert_ne!(cut.index, last, "energy {energy}");
                    last = cut.index;
                    cuts += 1;
                }
            }
            assert!(cuts >= 12);
        }
    }

    #[test]
    fn a_scene_with_one_shot_changes_the_seed() {
        let shots = [shot(ShotSize::Medium, ShotMove::Glide)];
        let mut director = Director::new(camera(0.4, false), &shots);
        let base = song(0.9);
        let cuts = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(cuts[0].1.index, 0);
        assert_eq!(cuts[0].1.motion, ShotMove::Glide);
        let mut seeds = vec![director.state().seed];
        for beat in 1..33u32 {
            let frame = MusicValues {
                beats: 1,
                bars: u32::from(beat % 4 == 0),
                ..base
            };
            if let Some(cut) = director.tick(0.5, &frame) {
                assert_eq!(cut.index, 0, "beat {beat}");
                seeds.push(director.state().seed);
            }
        }
        seeds.dedup();
        assert!(seeds.len() >= 3);
        assert!(seeds.iter().all(|s| (0.0..1.0).contains(s)));
    }

    #[test]
    fn the_same_kind_of_section_picks_the_same_shots() {
        use ShotSize::*;
        let shots = [
            Wide, Wide, Medium, Medium, Medium, Close, Close, Detail, Detail,
        ]
        .map(|size| shot(size, ShotMove::Cut))
        .to_vec();
        let mut director = Director::new(camera(0.0, false), &shots);
        let mut section = |label: f32, from: u32| -> Vec<u32> {
            let base = MusicValues {
                section_label: label,
                ..song(0.9)
            };
            let mut all = beats(&mut director, from, 1, &base, |_, m| start(label, m));
            all.extend(beats(&mut director, from + 1, 47, &base, plain));
            all.iter().map(|(_, cut)| cut.index).collect()
        };
        let first = section(2.0, 0);
        let other = section(3.0, 48);
        let again = section(2.0, 96);
        // The first starts with a shot that was on before it, so compare from the second cut.
        assert_eq!(first[1..], again[1..]);
        assert_ne!(first[1..], other[1..]);
    }

    #[test]
    fn wide_and_top_never_tilt_and_the_others_alternate() {
        let mut director = Director::new(camera(0.4, true), &each_size());
        let base = song(0.9);
        let mut rolls = Vec::new();
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        for beat in 1..200u32 {
            let frame = MusicValues {
                beats: 1,
                bars: u32::from(beat % 4 == 0),
                ..base
            };
            if let Some(cut) = director.tick(0.5, &frame) {
                let state = director.state();
                match cut.size {
                    ShotSize::Wide | ShotSize::Top => assert_eq!(state.roll, 0.0),
                    _ => {
                        assert!(state.roll.abs() >= 0.4 * ROLL_LEAST);
                        assert!(state.roll.abs() <= 0.4);
                        rolls.push(state.roll);
                    }
                }
            }
        }
        assert!(rolls.len() > 10);
        assert!(rolls.windows(2).all(|w| w[0] * w[1] < 0.0), "{rolls:?}");
    }

    #[test]
    fn a_scene_that_never_tilts_has_no_roll() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.9);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        for beat in 1..60u32 {
            let frame = MusicValues {
                beats: 1,
                bars: u32::from(beat % 4 == 0),
                ..base
            };
            director.tick(0.5, &frame);
            assert_eq!(director.state().roll, 0.0);
        }
    }

    #[test]
    fn a_shot_knows_its_age_and_planned_length() {
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        // Four bars of 2 s at 120 bpm.
        assert_eq!(director.state().length, 8.0);
        assert_eq!(director.state().age, 0.0);
        beats(&mut director, 1, 2, &base, plain);
        assert_eq!(director.state().age, 1.0);
        assert_eq!(director.state().cuts, 1);
        let unfollowed = MusicValues {
            locked: 0.0,
            ..base
        };
        director.tick(0.1, &start(1.0, unfollowed));
        assert_eq!(director.state().length, NORMAL_SECONDS);
    }

    /// The beats before a section at beat 24 that lifts the energy from 0.5 to 0.9, with the
    /// time to it known, from beat 16 on.
    fn lift_run(director: &mut Director) -> Vec<(u32, Cut)> {
        // A quiet section, so that no cut of its own cadence falls before the burst.
        let base = song(0.2);
        let mut all = beats(director, 0, 1, &base, |_, m| start(0.0, m));
        let coming = MusicValues {
            next_energy: 0.9,
            ..base
        };
        all.extend(beats(director, 1, 23, &coming, |beat, m| MusicValues {
            cue_in: (24 - beat) as f32 * 0.5,
            ..m
        }));
        // The section starts, as a cue.
        let high = song(0.9);
        all.extend(beats(director, 24, 1, &high, |_, m| start(1.0, m)));
        all
    }

    #[test]
    fn a_burst_of_cuts_comes_before_a_lift() {
        let mut director = Director::new(camera(0.3, true), &each_size());
        let all = lift_run(&mut director);
        // The burst: one cut on each of beats 20 to 23.
        let burst: Vec<&(u32, Cut)> = all.iter().filter(|(_, cut)| cut.fill > 0).collect();
        assert_eq!(burst.len(), 4, "{all:?}");
        use ShotSize::*;
        assert_eq!(
            burst.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            [20, 21, 22, 23]
        );
        assert_eq!(
            burst.iter().map(|(_, c)| c.fill).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert_eq!(
            burst.iter().map(|(_, c)| c.size).collect::<Vec<_>>(),
            [Medium, Close, Detail, Top]
        );
        // And the section start is a wide shot, with the progression started again.
        let (beat, last) = all.last().unwrap();
        assert_eq!((*beat, last.size, last.fill), (24, Wide, 0));
        // Two bars of 2 s: the section is a high one.
        assert_eq!(director.state().length, 4.0);
    }

    #[test]
    fn a_burst_started_by_hand_cuts_on_the_next_four_beats() {
        let mut director = Director::new(camera(0.3, false), &each_size());
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        director.start_burst();
        let all = beats(&mut director, 1, 8, &base, plain);
        use ShotSize::*;
        assert_eq!(
            all.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            [1, 2, 3, 4],
            "{all:?}"
        );
        assert_eq!(
            all.iter().map(|(_, c)| c.fill).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert_eq!(
            all.iter().map(|(_, c)| c.size).collect::<Vec<_>>(),
            [Medium, Close, Detail, Top]
        );
        assert!(all.iter().all(|(_, c)| c.motion == ShotMove::Cut));
        // Then it is over: no more burst cuts.
        assert_eq!(director.forced_burst, None);
        assert!(
            beats(&mut director, 9, 4, &base, plain)
                .iter()
                .all(|(_, c)| c.fill == 0)
        );
    }

    #[test]
    fn a_burst_started_by_hand_without_a_followed_beat_cuts_every_quarter_second() {
        let mut director = Director::new(camera(0.3, false), &each_size());
        let base = MusicValues {
            locked: 0.0,
            ..song(0.2)
        };
        director.start_burst();
        let mut cuts = Vec::new();
        for frame in 0..100 {
            if let Some(cut) = director.tick(0.0625, &base) {
                cuts.push((frame, cut.fill));
            }
        }
        // At once, then every 0.25 s (4 frames of 0.0625 s).
        assert_eq!(cuts, [(0, 1), (4, 2), (8, 3), (12, 4)]);
    }

    #[test]
    fn a_burst_tilts_alternately() {
        let mut director = Director::new(camera(0.3, true), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let coming = MusicValues {
            next_energy: 0.9,
            ..base
        };
        let mut rolls = Vec::new();
        for beat in 1..24u32 {
            let frame = MusicValues {
                beats: 1,
                bars: u32::from(beat % 4 == 0),
                cue_in: (24 - beat) as f32 * 0.5,
                ..coming
            };
            if let Some(cut) = director.tick(0.5, &frame)
                && cut.fill > 0
            {
                rolls.push(director.state().roll);
            }
        }
        // Medium, close, detail tilt (alternately), top does not.
        assert_eq!(rolls.len(), 4);
        assert!(rolls[0] * rolls[1] < 0.0 && rolls[1] * rolls[2] < 0.0);
        assert_eq!(rolls[3], 0.0);
    }

    #[test]
    fn a_burst_cuts_hard_to_a_glide_shot() {
        let shots: Vec<Shot> = each_size()
            .into_iter()
            .map(|s| shot(s.size, ShotMove::Glide))
            .collect();
        let mut director = Director::new(camera(0.3, true), &shots);
        let all = lift_run(&mut director);
        assert!(all.iter().any(|(_, c)| c.fill > 0));
        for (_, cut) in &all {
            let want = if cut.fill > 0 {
                ShotMove::Cut
            } else {
                ShotMove::Glide
            };
            assert_eq!(cut.motion, want, "{all:?}");
        }
    }

    #[test]
    fn a_burst_ends_on_wide_without_a_top_shot() {
        use ShotSize::*;
        // Two wide shots, as scenes declare, so that the section start at beat 0 stays wide.
        let shots = [Wide, Medium, Close, Detail, Wide]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        let mut director = Director::new(camera(0.3, true), &shots);
        let all = lift_run(&mut director);
        let burst: Vec<ShotSize> = all
            .iter()
            .filter(|(_, c)| c.fill > 0)
            .map(|(_, c)| c.size)
            .collect();
        assert_eq!(burst, [Medium, Close, Detail, Wide]);
    }

    #[test]
    fn no_burst_without_the_conditions() {
        use ShotSize::*;
        let count = |director: &mut Director| {
            lift_run(director)
                .iter()
                .filter(|(_, c)| c.fill > 0)
                .count()
        };
        // The scene does not allow it.
        assert_eq!(
            count(&mut Director::new(camera(0.3, false), &each_size())),
            0
        );
        // The scene has nothing near.
        let far = [Wide, Medium, Top]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        assert_eq!(count(&mut Director::new(camera(0.3, true), &far)), 0);
        // The energy does not rise enough.
        let mut director = Director::new(camera(0.3, true), &each_size());
        let base = song(0.5);
        let weak = MusicValues {
            next_energy: 0.55,
            ..base
        };
        let all = beats(&mut director, 1, 23, &weak, |beat, m| MusicValues {
            cue_in: (24 - beat) as f32 * 0.5,
            ..m
        });
        assert!(all.iter().all(|(_, c)| c.fill == 0));
        // The beat is not followed, or the song has no analysis.
        for (locked, has_chart) in [(0.2, 1), (1.0, 0)] {
            let mut director = Director::new(camera(0.3, true), &each_size());
            let coming = MusicValues {
                locked,
                has_chart,
                next_energy: 0.9,
                ..song(0.5)
            };
            let all = beats(&mut director, 1, 23, &coming, |beat, m| MusicValues {
                cue_in: (24 - beat) as f32 * 0.5,
                ..m
            });
            assert!(all.iter().all(|(_, c)| c.fill == 0), "{locked} {has_chart}");
        }
        // The time to the next section is unknown.
        let mut director = Director::new(camera(0.3, true), &each_size());
        let unknown = MusicValues {
            next_energy: 0.9,
            ..song(0.5)
        };
        let all = beats(&mut director, 1, 23, &unknown, plain);
        assert!(all.iter().all(|(_, c)| c.fill == 0));
    }

    #[test]
    fn a_burst_ends_a_phrase_of_a_high_section_and_the_next_phrase_starts_wide() {
        let mut director = Director::new(camera(0.3, true), &each_size());
        director.phrase_bursts = true;
        let base = song(0.9);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        // The section's 8 bars are beats 1 to 32 (a downbeat each 4), a second phrase follows.
        all.extend(beats(&mut director, 1, 40, &base, plain));
        let burst: Vec<&(u32, Cut)> = all.iter().filter(|(_, c)| c.fill > 0).collect();
        // The phrase's last bar is the bar that starts at beat 28 (the 7th downbeat is beat 28).
        assert_eq!(
            burst.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            [28, 29, 30, 31],
            "{all:?}"
        );
        assert_eq!(
            burst.iter().map(|(_, c)| c.fill).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        // The phrase starts on the downbeat of beat 32 with a wide shot.
        let at_32 = all.iter().find(|(b, _)| *b == 32).unwrap();
        assert_eq!((at_32.1.size, at_32.1.fill), (ShotSize::Wide, 0));
        // And the progression goes on from there: medium next, two bars after.
        let at_40 = all.iter().find(|(b, _)| *b == 40).unwrap();
        assert_eq!(at_40.1.size, ShotSize::Medium);
    }

    #[test]
    fn no_phrase_burst_by_default() {
        let mut director = Director::new(camera(0.3, true), &each_size());
        let base = song(0.9);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 80, &base, plain));
        assert!(all.len() > 10);
        assert!(all.iter().all(|(_, c)| c.fill == 0), "{all:?}");
    }

    #[test]
    fn hold_is_the_shortest_shot() {
        // A high section cuts every 2 bars; a hold of 4 makes it every 4 (beats 16 and 32).
        let mut director = Director::new(held(4), &each_size());
        let base = song(0.9);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let cuts = beats(&mut director, 1, 40, &base, plain);
        assert_eq!(cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(), [16, 32]);
        assert_eq!(director.state().length, 8.0);
        // A hold shorter than the section's cadence changes nothing.
        let mut director = Director::new(held(1), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let cuts = beats(&mut director, 1, 40, &base, plain);
        assert_eq!(cuts.len(), 5);
    }

    #[test]
    fn without_a_beat_the_wait_grows_with_hold() {
        // 4 bars of 2 s are 8 s, over the high section's 6 s; with no hit, 2 s more.
        let cuts = unfollowed_held(4, 0.8, 0, 25.0);
        assert!((cuts[0] - 10.0).abs() < 0.11, "{cuts:?}");
        assert!((cuts[1] - cuts[0] - 10.0).abs() < 0.11, "{cuts:?}");
        // A hold under the section's wait changes nothing.
        let cuts = unfollowed_held(2, 0.8, 0, 12.0);
        assert!((cuts[0] - 8.0).abs() < 0.11, "{cuts:?}");
    }

    #[test]
    fn no_phrase_burst_outside_a_high_section() {
        let mut director = Director::new(camera(0.3, true), &each_size());
        let base = song(0.5);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 80, &base, plain));
        assert!(all.iter().all(|(_, c)| c.fill == 0));
    }

    #[test]
    fn a_forced_cut_follows_the_size_rules_and_is_reported() {
        use ShotSize::*;
        let mut director = Director::new(camera(0.0, false), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(director.state().cuts, 1);
        // An ordinary section steps closer: medium, close, detail.
        let sizes: Vec<ShotSize> = (0..3).map(|_| director.force_cut(&base).size).collect();
        assert_eq!(sizes, [Medium, Close, Detail]);
        let state = *director.state();
        assert_eq!(state.cuts, 4);
        assert_eq!(state.size, Detail as u32);
        assert_eq!(state.index, 3);
        assert_eq!(state.age, 0.0);
        assert_eq!(state.fill, 0);
        // The cadence counts from the forced cut: four bars of 2 s later, not before.
        let cuts = beats(&mut director, 1, 20, &base, plain);
        assert_eq!(cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(), [16]);
        // The tier shows in the readout.
        assert_eq!(director.view(&base).tier, "normal");
        assert_eq!(director.view(&song(0.9)).tier, "high");
        assert_eq!(director.view(&song(0.1)).tier, "quiet");
    }

    #[test]
    fn a_held_shot_is_not_cut_away_from_until_released() {
        let mut director = Director::new(camera(0.3, true), &each_size());
        let base = song(0.9);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        // A shot the scene does not have changes nothing.
        let before = *director.state();
        assert!(director.hold_shot(6, &base).is_none());
        assert_eq!(*director.state(), before);
        let cut = director.hold_shot(2, &base).expect("shot 2 exists");
        assert_eq!(cut.index, 2);
        assert_eq!(director.state().index, 2);
        assert_eq!(director.state().size, ShotSize::Close as u32);
        assert_eq!(director.state().cuts, 2);
        assert!(director.view(&base).held);
        // A high section cuts every 2 bars and a section start cuts at once; none happens.
        let mut all = beats(&mut director, 1, 40, &base, plain);
        all.extend(beats(&mut director, 41, 1, &base, |_, m| start(1.0, m)));
        assert!(all.is_empty(), "{all:?}");
        assert_eq!(director.state().index, 2);
        assert_eq!(director.state().cuts, 2);
        // The shot ages meanwhile.
        assert!(director.state().age > 20.0);
        // Released, it waits a whole cadence (2 bars, 4 s) before it cuts.
        director.release_shot();
        assert!(!director.view(&base).held);
        let cuts = beats(&mut director, 42, 20, &base, plain);
        assert_eq!(cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(), [48, 56]);
    }

    #[test]
    fn a_lab_pace_overrides_the_cadence_and_its_own_restores_it() {
        let camera = Camera {
            pace: Pace::Calm,
            ..camera(0.0, false)
        };
        let cuts = |pace: Option<Pace>| {
            let mut director = Director::new(camera, &each_size());
            director.set_pace_override(pace);
            let base = song(0.5);
            beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
            beats(&mut director, 2, 64, &base, plain).len()
        };
        // The scene's own, calm: half of the 4 cuts a normal one makes in 16 bars.
        assert_eq!(cuts(None), 2);
        assert_eq!(cuts(Some(Pace::Busy)), 8);
        assert_eq!(cuts(Some(Pace::Sections)), 0);
        assert_eq!(cuts(None), 2);

        let mut director = Director::new(camera, &each_size());
        assert_eq!(
            (director.pace(), director.pace_override().is_some()),
            (Pace::Calm, false)
        );
        let mut seen = Vec::new();
        for _ in 0..PACE_CYCLE.len() {
            director.cycle_pace();
            seen.push(director.pace());
            assert!(director.pace_override().is_some());
        }
        assert_eq!(seen, PACE_CYCLE);
        // Past the last it is the scene's own again, which the view tells.
        director.cycle_pace();
        let view = director.view(&song(0.5));
        assert_eq!(
            (view.pace, view.own_pace, view.pace_overridden),
            (Pace::Calm, Pace::Calm, false)
        );
        assert_eq!(director.pace_override(), None);
    }

    #[test]
    fn the_view_lists_the_shots_in_order() {
        let director = Director::new(camera(0.0, false), &each_size());
        let list = director.view(&song(0.5)).shots;
        assert_eq!(list.len(), 6);
        assert_eq!(
            (list[0].1, list[4].1, list[5].1),
            (ShotSize::Wide, ShotSize::Top, ShotSize::Wide)
        );
    }
}
