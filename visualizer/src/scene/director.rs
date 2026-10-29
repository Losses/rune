//! The camera director: decides when a scene's view cuts to another shot and to which, following
//! the music, and tells the scene through `struct Shot` (see `docs/music.md`, "Shots and the
//! director"). A scene declares its shots (and the shots of its burst) in `scene.toml`; the
//! director is per scene (it needs the scene's manifest and counts from the scene's start). It is a pure state machine of the music's
//! values ([`MusicValues`]) and the time step, so it can be tested without a scene or a GPU.

use std::sync::Arc;

use super::inputs::MusicValues;
use super::manifest::{
    BurstShot, Camera, DutchIn, MAX_BURSTS, Manifest, Pace, Shot, ShotMove, ShotSize,
};
use crate::bind::{DirectorBinds, MomentBinds, Said};
use crate::chart::{SongSection, last_chance};
use crate::moments::{key, rolls};
use crate::switching::FOLLOWED;

// The numbers below are the defaults of `director.*` in `runtime.toml`, which the director reads
// (a test pins that they agree); a scene lab can replace them live.
/// A section whose energy is below this is quiet.
#[cfg(test)]
pub const QUIET_BELOW: f32 = 0.35;
/// A section whose energy is above this is high.
#[cfg(test)]
pub const HIGH_ABOVE: f32 = 0.71;
// The cadence is in seconds, whatever the tempo: counted in bars, a fast song cut every few
// seconds, far too often.
/// Seconds between two cuts in a quiet section.
#[cfg(test)]
pub const QUIET_SECONDS: f32 = 24.0;
/// Seconds between two cuts in a section that is neither quiet nor high.
#[cfg(test)]
pub const NORMAL_SECONDS: f32 = 16.0;
/// Seconds between two cuts in a high section.
#[cfg(test)]
pub const HIGH_SECONDS: f32 = 10.0;
/// The shortest time between any two cuts that are not inside one burst.
#[cfg(test)]
pub const GAP_SECONDS: f32 = 6.0;
/// With a known next section start, how close to it a cadence cut waits for it.
#[cfg(test)]
pub const LEAD_SECONDS: f32 = 4.0;
/// How much a calm pace stretches the seconds between cuts, and a busy one shortens them.
pub const PACE_FACTOR: f32 = 1.5;
/// Without a followed beat: how long past the time a cut is due it waits for a hit, then cuts at
/// once.
#[cfg(test)]
pub const HIT_GRACE_SECONDS: f32 = 2.0;
/// How much more energy the next section needs for the burst before it (the cue's own rule for a
/// lift is the same, `scene::signals::CUE_LIFT`).
#[cfg(test)]
pub const FILL_LIFT: f32 = 0.1;
/// Without a followed beat, how long a burst a scene lab started waits between its cuts.
pub const FORCED_BURST_SECONDS: f32 = 0.25;
/// The length of a phrase in a high section, in bars from the section's start.
pub const PHRASE_BARS: u32 = 8;
/// Whether the end of every phrase of a high section gets a burst of cuts. Off: fewer cuts are
/// wanted, and the burst before an energy lift (as in the reference) stays. A scene of `pace =
/// "busy"` has them all the same.
pub const PHRASE_BURSTS: bool = false;
/// What the roll of a section's burst is mixed with (the `which` of [`key`]), so that it is not
/// the roll of the moment at the same section start, which would make the sections that burst the
/// ones that play an effect.
const BURST_ROLL: u32 = 0xb057;
/// The share of the dutch angle a shot with seed 0 gets; the seed scales the rest.
pub const ROLL_LEAST: f32 = 0.5;
/// The paces a scene lab steps through, from the least often cutting to the most (after the
/// scene's own).
pub const PACE_CYCLE: [Pace; 4] = [Pace::Sections, Pace::Calm, Pace::Normal, Pace::Busy];

/// How lively the section is, from its energy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    Quiet,
    Normal,
    High,
}

impl Tier {
    /// The tier of the section by `director.cadence`: its energy (the chart's, else the music's
    /// slow energy) against the thresholds.
    fn of(binds: &DirectorBinds, music: &MusicValues) -> Self {
        match binds.tier(music) {
            0 => Self::Quiet,
            2 => Self::High,
            _ => Self::Normal,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Quiet => 0,
            Self::Normal => 1,
            Self::High => 2,
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

    fn seconds(self, binds: &DirectorBinds) -> f32 {
        binds.seconds(self.index())
    }
}

/// A cut the director made in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    /// The `[[shot]]` it cut to.
    pub index: u32,
    pub size: ShotSize,
    /// The shot's move, but always `Cut` for a hard cut: inside a burst, and on a strong cut (the
    /// cut a burst leads to, the start of a high section), whatever the shot's `move` says, so a
    /// gliding scene jumps there. A `Glide` is a move the scene makes itself and the transition
    /// layer does not dress.
    pub motion: ShotMove,
    /// 0 for a normal cut, else the position in a burst (1 to [`MAX_BURSTS`]).
    pub fill: u32,
    /// Whether the cut lands on a strong transition of the scene's list, whatever the moment's
    /// roll: the cut a burst leads to (the first after its last: a section's or a phrase's start,
    /// or the opening after a lab's burst), and the start of a high section, which a climax is
    /// always entered on.
    pub strong: bool,
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
    /// 1 when the shot was entered by a hard cut (a cut to a `move = "cut"` shot, any cut of a
    /// burst, a strong cut) and the scene is to jump to it; 0 when it is to glide to it. The
    /// scene's first shot is 1: there is nothing to glide from.
    pub hard: u32,
}

impl ShotState {
    /// The layout of `struct Shot`: nine words and three of padding (48 bytes, a multiple of 16).
    pub fn bytes(&self) -> [u8; 48] {
        let words = [
            self.index.to_le_bytes(),
            self.cuts.to_le_bytes(),
            self.size.to_le_bytes(),
            self.roll.to_le_bytes(),
            self.seed.to_le_bytes(),
            self.age.to_le_bytes(),
            self.length.to_le_bytes(),
            self.fill.to_le_bytes(),
            self.hard.to_le_bytes(),
        ];
        let mut bytes = [0; 48];
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
    /// The scene's shots in order, its `[[shot]]`s and then its `[[burst]]`s: the name (empty if
    /// it has none), the size, and the place in the burst (1 to 4; 0 for a `[[shot]]`).
    pub shots: Vec<(String, ShotSize, u32)>,
    /// The pace in effect and the scene's own, and whether a lab overrides it.
    pub pace: Pace,
    pub own_pace: Pace,
    pub pace_overridden: bool,
    /// The chance `director.burst` gives a section start that lifts its burst, whether the lift
    /// that comes next will be bursting (the lab's choice, else the roll of the coming section
    /// or the song's last lift if none has burst), and the lab's choice.
    pub burst_chance: f32,
    pub burst_next: bool,
    pub bursts: Option<bool>,
    /// For each shot of `shots`: whether the song still owes it a cut (it says `at_least_once`,
    /// has not been on in this song, and the section playing is not past its last chance).
    pub owed: Vec<bool>,
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
    /// The position (1 to the number of declared burst shots) of the next cut.
    next: u32,
    /// Without a followed beat: seconds until the next cut.
    wait: f32,
}

/// The number `rv_label_hash` in `stdlib.wgsl` gives: 0 to 1, the same for the same label and `k`.
pub(super) fn label_hash(label: f32, k: u32) -> f32 {
    let multiplier = [0.618_033_9, 0.754_877_7, 0.569_840_3][(k % 3) as usize];
    ((label + 1.0) * multiplier + (k / 3) as f32 * 0.314_159_3).fract()
}

/// What a cut is for, which decides which shots it may go to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// A section starts: any shot that opens a section.
    Opening,
    /// The cadence (or a lab) cuts within a section: a shot of another size, as the tier wants.
    Next(Tier),
}

/// The index of the `r`th (0 to 1) of `chances` by their weight, where each is picked in
/// proportion to its chance and one whose chance is 0 never is. All of them 0 picks evenly. It is
/// a pure function of `r`, so that a hash of the section picks the same each time.
pub(super) fn weighted(chances: &[(usize, f32)], r: f32) -> usize {
    let total: f32 = chances.iter().map(|(_, chance)| chance).sum();
    if total <= 0.0 {
        let at = (r * chances.len() as f32) as usize;
        return chances[at.min(chances.len() - 1)].0;
    }
    let target = r * total;
    let mut sum = 0.0;
    for (index, chance) in chances {
        sum += chance;
        if sum > target && *chance > 0.0 {
            return *index;
        }
    }
    // Only a rounding error in the sum gets here: the last shot that has a chance.
    chances
        .iter()
        .rev()
        .find(|(_, chance)| *chance > 0.0)
        .map_or(chances[0].0, |(index, _)| *index)
}

/// What a `[[shot]]` declares of when the director picks it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pickable {
    /// It may open a section.
    opens: bool,
    /// How often it is picked against the others.
    weight: f32,
    /// Whether it suits a quiet, a normal and a high section.
    suits: [bool; 3],
    /// The longest it may last, in seconds, if the scene says.
    longest: Option<f32>,
    /// Whether it tilts whatever `[camera] dutch_in` says, if the scene says.
    dutch: Option<bool>,
    /// Whether the song is promised a cut to it (`[[shot]] at_least_once`).
    at_least_once: bool,
}

/// Decides the camera cuts of one scene from the music.
pub struct Director {
    camera: Camera,
    /// The scene's shots in the order of its `[[shot]]`s, then the shots of its `[[burst]]`s in
    /// the order of the burst (always hard cuts).
    shots: Vec<(ShotSize, ShotMove)>,
    /// The `name` of each shot, in the same order.
    names: Vec<String>,
    /// How many of `shots` are `[[shot]]`s: the ones ordinary cuts and section starts pick from.
    /// The rest are the burst's, which only the burst cuts to.
    ordinary: usize,
    /// What each `[[shot]]` says of when it is picked, in the same order (only the ordinary ones).
    picks: Vec<Pickable>,
    state: ShotState,
    /// The next tilted shot leans to the right (positive roll).
    tilt_right: bool,
    /// Cuts since the section started, counting its establishing shot.
    in_section: u32,
    /// The shot this section's own sequence cut to last (none before its first): what the next
    /// pick differs from, so that a section's shots follow from its label alone and every chorus
    /// cuts the same way, whatever was on before it.
    seq_prev: Option<usize>,
    /// The section's tier as of the last frame: which shots suit it.
    tier_now: Tier,
    /// Bars since the section started, counted from its start.
    bars_in_section: u32,
    /// Seconds since the last cut.
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
    /// What a scene lab decides of the bursts before a lift in place of the chance: every lift
    /// bursts (`Some(true)`), none does (`Some(false)`), or the runtime's chance (none).
    burst_override: Option<bool>,
    /// Whether a burst before a lift has begun in this song. A scene that declares a burst bursts
    /// at least once in a song it is on for the end of: if none has begun by the song's last lift,
    /// that lift bursts whatever the roll.
    burst_taken: bool,
    /// The biggest rise of energy among the song's section boundaries after the coming section
    /// start (see [`Self::set_song_ahead`]); none without a chart, where there is no burst.
    later_rise: Option<f32>,
    /// The sections of the song with their levels, as the chart tells them ahead of time (see
    /// [`Self::set_song`]); none without a chart, where nothing is promised.
    song: Option<Arc<[SongSection]>>,
    /// For each `[[shot]]`: the section of the song it is promised in, if it says
    /// `at_least_once` and the song has a section it can be cut to in (see [`Self::promise`]).
    /// Made when the song or the bindings change, so a frame only compares an index.
    promised: Vec<Option<usize>>,
    /// For each `[[shot]]`: whether it has been on in this song. Like `burst_taken` it starts
    /// again when the song time goes back.
    shown: Vec<bool>,
    /// The song time of the last frame, which going back tells a new song (or a seek) by.
    last_song_time: f32,
    /// The runtime's decisions the director reads (`director.*` and what a section start is),
    /// from `runtime.toml`.
    binds: Arc<DirectorBinds>,
    moments: Arc<MomentBinds>,
}

impl Director {
    /// The director of a scene with shots; none for a scene without (it works as it did before
    /// there were shots).
    pub fn for_manifest(manifest: &Manifest) -> Option<Self> {
        (!manifest.shots.is_empty())
            .then(|| Self::new(manifest.camera, &manifest.shots).with_bursts(&manifest.bursts))
    }

    /// A director over `shots`, which must not be empty. It starts on a wide shot (or the
    /// nearest to one), before any cut.
    pub fn new(camera: Camera, shots: &[Shot]) -> Self {
        assert!(!shots.is_empty(), "a director needs a shot");
        let names = shots.iter().map(|s| s.name.clone()).collect();
        let ordinary = shots.len();
        let picks = shots
            .iter()
            .map(|s| Pickable {
                opens: s.opens(),
                weight: s.weight,
                longest: s.longest,
                dutch: s.dutch,
                at_least_once: s.at_least_once,
                suits: [0, 1, 2].map(|tier| super::manifest::suits(&s.tiers, tier)),
            })
            .collect();
        let shots: Vec<(ShotSize, ShotMove)> = shots.iter().map(|s| (s.size, s.motion)).collect();
        let mut director = Self {
            camera,
            shots,
            names,
            ordinary,
            picks,
            state: ShotState {
                index: 0,
                cuts: 0,
                size: 0,
                roll: 0.0,
                seed: 0.0,
                age: 0.0,
                length: -1.0,
                fill: 0,
                hard: 1,
            },
            tilt_right: false,
            in_section: 0,
            seq_prev: None,
            tier_now: Tier::Normal,
            bars_in_section: 0,
            seconds_since_cut: 0.0,
            beat_in_bar: None,
            last_fill: 0,
            phrase_start: None,
            phrase_bursts: PHRASE_BURSTS || camera.pace == Pace::Busy,
            pace_override: None,
            held: false,
            forced_burst: None,
            burst_override: None,
            burst_taken: false,
            later_rise: None,
            song: None,
            promised: vec![None; ordinary],
            shown: vec![false; ordinary],
            last_song_time: 0.0,
            binds: Arc::default(),
            moments: Arc::default(),
        };
        let index = director.pick(Why::Opening, 0.0, 0, None);
        director.state.index = index as u32;
        director.state.size = director.shots[index].0 as u32;
        director.state.seed = label_hash(0.0, 0);
        director
    }

    /// This director with the burst through `bursts`, one cut to each in order, as the scene
    /// declares them (at most [`MAX_BURSTS`]; the manifest refuses more). A scene without any has
    /// no burst. The shots go after the scene's `[[shot]]`s in `struct Shot`'s `index`.
    pub fn with_bursts(mut self, bursts: &[BurstShot]) -> Self {
        assert!(
            bursts.len() <= MAX_BURSTS,
            "at most {MAX_BURSTS} burst shots"
        );
        self.shots.truncate(self.ordinary);
        self.names.truncate(self.ordinary);
        for burst in bursts {
            self.shots.push((burst.size, ShotMove::Cut));
            self.names.push(burst.name.clone());
        }
        self
    }

    /// How many shots the burst cuts to: the scene's `[[burst]]`s.
    fn bursts(&self) -> u32 {
        (self.shots.len() - self.ordinary) as u32
    }

    /// Decide by these bindings from now on (a scene lab replaced `runtime.toml`).
    pub fn set_binds(&mut self, binds: Arc<DirectorBinds>, moments: Arc<MomentBinds>) {
        self.binds = binds;
        self.moments = moments;
        self.plan_promises();
    }

    /// The pace in effect: the lab's override, else the scene's own.
    pub fn pace(&self) -> Pace {
        self.pace_override.unwrap_or(self.camera.pace)
    }

    /// What a scene lab decides of the bursts before a lift, to give a rebuilt director (see
    /// [`Self::set_burst_override`]).
    pub fn burst_override(&self) -> Option<bool> {
        self.burst_override
    }

    /// Make every burst before a lift happen (`Some(true)`) or none (`Some(false)`), in place of
    /// the runtime's chance and the burst the song's last lift is promised; none leaves it to the runtime.
    /// A burst already under way goes on, and the lab's own (see [`Self::start_burst`]) always
    /// bursts.
    pub fn set_burst_override(&mut self, bursts: Option<bool>) {
        self.burst_override = bursts;
    }

    /// Tell the director what the song's chart says of the sections after the coming one: the
    /// biggest rise of energy among their boundaries (`ChartState::later_rise`), or none without a
    /// chart. A lift that no later boundary matches is the song's last, which is what the promised
    /// burst of a song waits for.
    pub fn set_song_ahead(&mut self, later_rise: Option<f32>) {
        self.later_rise = later_rise;
    }

    /// Tell the director the sections of the song with their levels (`ChartState::song`), or none
    /// without a chart. A shot that says `at_least_once` is promised a cut in the last section
    /// that suits it (see [`Self::promise`]), which the director can only tell by knowing every
    /// later section's level ahead of time. The scene hands it over each frame, but only a
    /// different song (another list, not another frame of the same) makes the director plan
    /// again, and one that replaces another forgets what was on.
    pub fn set_song(&mut self, song: Option<&Arc<[SongSection]>>) {
        match (&self.song, song) {
            (Some(on), Some(new)) if Arc::ptr_eq(on, new) => return,
            (None, None) => return,
            (Some(_), Some(_)) => self.shown.fill(false),
            _ => {}
        }
        self.song = song.cloned();
        self.plan_promises();
    }

    /// Decide again in which section each shot that says `at_least_once` is promised.
    fn plan_promises(&mut self) {
        self.promised = (0..self.ordinary)
            .map(|i| {
                self.picks[i]
                    .at_least_once
                    .then(|| self.promise(i))
                    .flatten()
            })
            .collect();
    }

    /// The section of the song in which shot `index` is cut to if it has not been on by then: the
    /// last section that suits it by its `tiers` (the section's level against `director.cadence`'s
    /// thresholds), or, where no section suits it, the song's highest (the last of equals). A
    /// section counts only if a cut can be made in it after its opening: it must last the gap
    /// from the opening, the room `waits_for_section` keeps before the next section start (the
    /// larger of `lead` and the gap) and the cut's grace to land. If the last section that suits
    /// is too short, the one before it that suits is the last chance; if none is long enough,
    /// the highest section of the song that is, by level, the last of equals first. None where
    /// no section is long enough, or the song is not known.
    fn promise(&self, index: usize) -> Option<usize> {
        let song = self.song.as_ref()?;
        let room = self.min_gap() + self.binds.lead().max(self.min_gap()) + self.binds.grace;
        let fits = |i: usize| song[i].end - song[i].start >= room;
        let suits =
            |section: &SongSection| self.picks[index].suits[self.binds.tier_at(section.level)];
        last_chance(song, suits, fits)
    }

    /// The shot the song is still owed in the section playing, if any: one that says
    /// `at_least_once`, has not been on in this song and has this section as its last chance.
    /// The first of them if several.
    fn owed(&self, music: &MusicValues) -> Option<usize> {
        let section = music.section.max(0.0) as usize;
        (0..self.ordinary).find(|&i| self.promised[i] == Some(section) && !self.shown[i])
    }

    /// Whether the song still owes shot `index` a cut, in this section or a later one.
    fn is_owed(&self, index: usize, music: &MusicValues) -> bool {
        let section = music.section.max(0.0) as usize;
        !self.shown.get(index).copied().unwrap_or(true)
            && self
                .promised
                .get(index)
                .copied()
                .flatten()
                .is_some_and(|last| last >= section)
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

    /// The scene's shots as a readout lists them, its `[[shot]]`s and then its `[[burst]]`s (which
    /// a lab may hold too, to frame them): the name (empty if it has none), the size and the place
    /// in the burst (0 for a `[[shot]]`).
    pub fn shot_list(&self) -> Vec<(String, ShotSize, u32)> {
        self.names
            .iter()
            .zip(&self.shots)
            .enumerate()
            .map(|(index, (name, (size, _)))| (name.clone(), *size, self.burst_place(index)))
            .collect()
    }

    /// The place of shot `index` in the burst, from 1; 0 for one of the scene's `[[shot]]`s.
    fn burst_place(&self, index: usize) -> u32 {
        (index + 1).saturating_sub(self.ordinary) as u32
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
            tier: Tier::of(&self.binds, music).name(),
            held: self.held,
            shots: self.shot_list(),
            pace: self.pace(),
            own_pace: self.camera.pace,
            pace_overridden: self.pace_override.is_some(),
            burst_chance: self.binds.burst_chance(),
            burst_next: self.bursts() > 0
                && self.binds.ahead(music).is_some_and(|ahead| ahead.lifts)
                && self.will_burst(music),
            bursts: self.burst_override,
            owed: (0..self.shots.len())
                .map(|index| self.is_owed(index, music))
                .collect(),
        }
    }

    /// Cut now, as if a cut were due: to the size the section's tier gives for the next cut, by
    /// the same rules as the cadence, and tell the scene. It is what a scene lab asks for; the
    /// cadence counts from this cut on. A held shot is cut away from (and stays held).
    pub fn force_cut(&mut self, music: &MusicValues) -> Cut {
        let tier = Tier::of(&self.binds, music);
        self.tier_now = tier;
        self.cut(self.why(tier), music, tier, 0)
    }

    /// Start a burst of cuts now, through the shots the scene declares as `[[burst]]`, as the one
    /// before a lift is: a cut on each of the next beats, one per shot, or without a followed
    /// beat one every [`FORCED_BURST_SECONDS`], the first at once. It is what a scene lab asks
    /// for, to see a burst without waiting for a lift; the cadence is quiet until it is over, and
    /// a section start still cuts as it does. A scene that declares none does nothing. A held
    /// shot defers it until the shot is let go of.
    pub fn start_burst(&mut self) {
        if self.bursts() > 0 {
            self.forced_burst = Some(ForcedBurst { next: 1, wait: 0.0 });
        }
    }

    /// Cut to shot `index` (of the scene's `[[shot]]`s, then its `[[burst]]`s) and hold it: the
    /// director cuts no more until [`Self::release_shot`]. The shot's size and roll follow the
    /// rules of any cut, and a burst's shot is told as its place in the burst, as when the burst
    /// cuts to it. Returns the cut, or none for a shot the scene does not have (nothing changes
    /// then).
    pub fn hold_shot(&mut self, index: u32, music: &MusicValues) -> Option<Cut> {
        let index = index as usize;
        if index >= self.shots.len() {
            return None;
        }
        let fill = self.burst_place(index);
        let cut = self.enter(index, music, Tier::of(&self.binds, music), fill);
        self.held = true;
        Some(cut)
    }

    /// Let go of a held shot: the director cuts by its cadence again, counted from now so that it
    /// does not cut at once.
    pub fn release_shot(&mut self) {
        if self.held {
            self.held = false;
            self.seconds_since_cut = 0.0;
        }
    }

    /// Advance by one frame of `dt` seconds with the music of the frame, and return the cut that
    /// happened in it, if any.
    pub fn tick(&mut self, dt: f32, music: &MusicValues) -> Option<Cut> {
        self.state.age += dt;
        self.seconds_since_cut += dt;
        self.bars_in_section += music.bars;
        if music.bars > 0 {
            self.beat_in_bar = Some(0);
        } else if music.beats > 0 {
            self.beat_in_bar = self.beat_in_bar.map(|beat| beat + music.beats);
        }
        let tier = Tier::of(&self.binds, music);
        self.tier_now = tier;
        let followed = music.locked > FOLLOWED;
        // Time going back is a new song (or a seek): its burst is still to come.
        if music.song_time < self.last_song_time - 1.0 {
            self.burst_taken = false;
            self.shown.fill(false);
        }
        self.last_song_time = music.song_time;
        if self.held {
            return None;
        }

        // A section starts: establish it, and count the progression again. A start within the gap
        // of the last cut does not cut (the shot goes on), unless it follows a burst (the burst
        // and the start after it are one group, which the gap does not hold apart) or it is the
        // start of a high section: a climax is always entered on a cut, on a strong transition.
        if self.moments.section_start(music) {
            self.begin_section(music.section_label);
            self.bars_in_section = 0;
            self.phrase_start = None;
            let climax = tier == Tier::High;
            return (self.gap_is_open() || self.last_fill > 0 || climax).then(|| {
                let mut cut = self.cut(Why::Opening, music, tier, 0);
                if climax {
                    self.harden(&mut cut);
                }
                cut
            });
        }
        // A phrase that was announced by a burst starts: the same, and in the same group.
        if let Some(start) = self.phrase_start
            && music.bars > 0
            && self.bars_in_section >= start
        {
            self.begin_section(music.section_label);
            self.phrase_start = None;
            return Some(self.cut(Why::Opening, music, tier, 0));
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
            if fill > self.bursts() {
                // The beat after its last cut opens a section, as the section start a burst leads
                // to would: left alone, the lab's burst stayed on its last shot until the cadence.
                self.forced_burst = None;
                self.begin_section(music.section_label);
                return Some(self.cut(Why::Opening, music, tier, 0));
            }
            return Some(self.burst_cut(fill, music, tier));
        }
        // The burst before a section or a phrase ends. It goes through its shots one on each
        // beat, from the first: a burst that cannot start (the last cut is within the gap) is
        // skipped whole, not begun in the middle.
        if let Some((kind, fill)) = self.burst(music, tier)
            && fill == self.last_fill + 1
            && (fill > 1 || self.gap_is_open())
        {
            if kind == Burst::Phrase {
                self.phrase_start = Some(self.bars_in_section + 1);
            } else if fill == 1 {
                self.burst_taken = true;
            }
            return Some(self.burst_cut(fill, music, tier));
        }
        // The cadence of the section: its seconds since the last cut, landing on the event of
        // `director.cut`, and not close to a section start or the burst before it, which cut
        // instead.
        let wait = self.shot_wait(self.state.index as usize, tier);
        // A shot the song still owes, in the section that is its last chance, is cut to at the
        // gap from the opening if the cadence would not cut again before the section ends.
        let hurried = self.hurries(wait, music);
        let wait = match (wait, hurried) {
            (Some(wait), true) => wait.min(self.min_gap()),
            (None, true) => self.min_gap(),
            (wait, false) => wait?,
        };
        if self.seconds_since_cut < wait || (!hurried && self.waits_for_section(music, followed)) {
            return None;
        }
        let landed = match self.binds.cut(music) {
            Said::Source(cut) => cut.count > 0,
            Said::Fallback(..) | Said::Value(_) => {
                self.binds.cut(music).count() > 0
                    || self.seconds_since_cut >= wait + self.binds.grace
            }
            Said::Off => false,
        };
        landed.then(|| self.cut(self.why(tier), music, tier, 0))
    }

    /// The shortest time between two cuts that are not inside one burst: `director.cadence`'s
    /// `gap`, or the scene's `hold` if that is longer.
    fn min_gap(&self) -> f32 {
        self.binds.gap().max(self.camera.hold)
    }

    /// Whether a cut may come now: the scene has not cut yet, or the last cut is the gap ago.
    fn gap_is_open(&self) -> bool {
        self.state.cuts == 0 || self.seconds_since_cut >= self.min_gap()
    }

    /// Seconds between two cadence cuts: the section's at the scene's pace (`calm` by
    /// [`PACE_FACTOR`] more, `busy` by it less), but at least the gap. None for a scene that cuts
    /// only at a section's start.
    fn cadence_seconds(&self, tier: Tier) -> Option<f32> {
        let seconds = tier.seconds(&self.binds);
        let seconds = match self.pace() {
            Pace::Sections => return None,
            Pace::Calm => seconds * PACE_FACTOR,
            Pace::Normal => seconds,
            Pace::Busy => seconds / PACE_FACTOR,
        };
        Some(seconds.max(self.min_gap()))
    }

    /// How long shot `index` waits for the next cut by the cadence: the section's seconds at the
    /// scene's pace, or the shot's own `longest` if that is shorter, but at least the gap. None
    /// where nothing but a section's start cuts it.
    fn shot_wait(&self, index: usize, tier: Tier) -> Option<f32> {
        let longest = self
            .picks
            .get(index)
            .and_then(|pick| pick.longest)
            .map(|s| s.max(self.min_gap()));
        match (self.cadence_seconds(tier), longest) {
            (Some(cadence), Some(longest)) => Some(cadence.min(longest)),
            (cadence, longest) => cadence.or(longest),
        }
    }

    /// Whether a shot the song owes is to be cut to sooner than the cadence would cut, `wait`
    /// seconds after the last cut: a shot is owed in this section, its opening is on (the opening
    /// keeps its own rule), and the cadence's next cut would come too late, that is within the
    /// room `waits_for_section` keeps clear before the next section start (or never, at a pace
    /// that cuts only at section starts). Otherwise the cadence's own next cut is the promised
    /// one (see [`Self::cut`]).
    fn hurries(&self, wait: Option<f32>, music: &MusicValues) -> bool {
        if self.in_section == 0 || self.owed(music).is_none() {
            return false;
        }
        let Some(wait) = wait else {
            return true;
        };
        let left = music.cue_in;
        let due = (wait - self.seconds_since_cut).max(0.0);
        let keeps_clear = self.binds.lead().max(self.min_gap());
        left >= 0.0 && due + self.binds.grace + keeps_clear > left
    }

    /// Whether a cadence cut now would crowd the cuts that come next, so that it waits: the next
    /// section start is known and less than `lead` seconds away, or less than the gap (a cut then
    /// would hold the section start back), or a burst is declared and the section after it lifts
    /// the energy, and the burst's first cut is less than the gap away.
    fn waits_for_section(&self, music: &MusicValues, followed: bool) -> bool {
        let Some(ahead) = self.binds.ahead(music) else {
            return false;
        };
        if ahead.cue_in < self.binds.lead().max(self.min_gap()) {
            return true;
        }
        // A burst that will not be taken holds nothing back.
        if self.bursts() == 0
            || !ahead.lifts
            || !followed
            || music.tempo <= 0.0
            || !self.will_burst(music)
        {
            return false;
        }
        let burst = self.bursts() as f32 * 60.0 / music.tempo;
        ahead.cue_in - burst < self.min_gap()
    }

    /// Whether the burst before the next section start, if that lifts the energy, will be taken.
    /// The lab's choice decides if it made one. Else the roll does: a number from 0 to 1 hashed
    /// from the coming section (its index in the song, which tells the occurrences of a label
    /// apart, and its label), so that the same section of the same song rolls the same on every
    /// play and sections differ, under `director.burst`'s `chance`. Failing the roll, the song's
    /// last lift bursts if none has begun in this song (a scene that declares a burst bursts at
    /// least once in a song, the guarantee waiting at the end so that the early lifts are left to
    /// chance). The last lift is the one that no later section boundary rises as much as
    /// `director.burst`'s `lift`. A burst that has begun goes on to its last cut.
    fn will_burst(&self, music: &MusicValues) -> bool {
        if self.last_fill > 0 {
            return true;
        }
        if let Some(bursts) = self.burst_override {
            return bursts;
        }
        let coming = music.section.max(0.0) as u32 + 1;
        let roll = key(coming, BURST_ROLL) ^ music.next_label.to_bits().wrapping_mul(0x27d4_eb2f);
        rolls(roll, self.binds.burst_chance())
            || (!self.burst_taken && self.later_rise.is_some_and(|rise| rise < self.binds.lift))
    }

    /// Whether this frame is a beat in the last beats before a place worth a burst, one for each
    /// shot the scene declares as `[[burst]]`, and which one of the burst it is (1 for the
    /// farthest). Only with the song's analysis, a followed beat and a burst declared.
    fn burst(&self, music: &MusicValues, tier: Tier) -> Option<(Burst, u32)> {
        let count = self.bursts();
        if count == 0 {
            return None;
        }
        // The burst is on where `director.burst` has a time to the next section and its `on`
        // source has a beat to cut on: with the chart and a followed beat.
        let ahead = self.binds.burst(music)?;
        let position = |left: f32| {
            let left = left.round();
            (1.0..=count as f32)
                .contains(&left)
                .then(|| count - left as u32 + 1)
        };
        if ahead.cue_in >= 0.0
            && ahead.lifts
            && self.will_burst(music)
            && let Some(fill) = position(ahead.cue_in * music.tempo / 60.0)
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

    /// Cut to the burst's `fill`th shot (from 1), a hard cut, and tell the scene.
    fn burst_cut(&mut self, fill: u32, music: &MusicValues, tier: Tier) -> Cut {
        self.enter(self.ordinary + fill as usize - 1, music, tier, fill)
    }

    /// What an ordinary cut is for: it opens the section if none has been cut to since the section
    /// started (a section start within the gap did not cut, so the next cut establishes the
    /// section), else it goes to another size.
    fn why(&self, tier: Tier) -> Why {
        if self.in_section == 0 {
            Why::Opening
        } else {
            Why::Next(tier)
        }
    }

    /// Cut to a `[[shot]]` that suits `why`, other than the one on, and tell the scene.
    ///
    /// The pick follows the section's own sequence, so that the same kind of section cuts the same
    /// way each time: a section opens with the shot its label picks, and every later cut differs
    /// from the sequence's last shot. Only an opening that would land on the shot on screen picks
    /// again away from it (a cut there would only jump), and one of the size of a burst's last
    /// shot (after a burst that ends from above, a view from above would only seem to move in).
    fn cut(&mut self, why: Why, music: &MusicValues, tier: Tier, fill: u32) -> Cut {
        let label = music.section_label;
        let current = self.state.index as usize;
        let index = match why {
            Why::Opening => {
                let index = self.pick(why, label, self.in_section, None);
                let after_burst = current >= self.ordinary;
                if index == current || (after_burst && self.shots[index].0 == self.shots[current].0)
                {
                    self.pick(why, label, self.in_section, Some(current))
                } else {
                    index
                }
            }
            // The shot the song owes goes first, in the section that is its last chance.
            Why::Next(_) => self.owed(music).unwrap_or_else(|| {
                let prev = self.seq_prev.unwrap_or(current);
                self.pick(why, label, self.in_section, Some(prev))
            }),
        };
        self.seq_prev = Some(index);
        self.enter(index, music, tier, fill)
    }

    /// A section starts: its sequence of shots and its leaning from the dutch angle begin again,
    /// from its label.
    fn begin_section(&mut self, label: f32) {
        self.in_section = 0;
        self.seq_prev = None;
        self.tilt_right = label_hash(label, 3) < 0.5;
    }

    /// Cut to shot `index`: count the cut, give the shot its roll and seed, and say how long it
    /// is planned to last.
    fn enter(&mut self, index: usize, music: &MusicValues, tier: Tier, fill: u32) -> Cut {
        let label = music.section_label;
        let (size, motion) = self.shots[index];
        self.state.cuts = self.state.cuts.wrapping_add(1);
        self.in_section += 1;
        if let Some(shown) = self.shown.get_mut(index) {
            *shown = true;
        }
        // From the section's label and the cut's place in it, so a chorus is seen the same way
        // each time.
        let seed = label_hash(label, self.in_section.wrapping_add(7));
        // A shot may say for itself; a burst's shots and the others follow `dutch_in`.
        let tilts = self
            .picks
            .get(index)
            .and_then(|pick| pick.dutch)
            .unwrap_or(fill > 0 || self.camera.dutch_in == DutchIn::All);
        let level = matches!(size, ShotSize::Wide | ShotSize::Top) || !tilts;
        let roll = if level {
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
            length: if fill > 0 && music.locked > FOLLOWED {
                beat
            } else {
                self.shot_wait(index, tier).unwrap_or(-1.0)
            },
            fill,
            hard: 0,
        };
        self.seconds_since_cut = 0.0;
        let strong = fill == 0 && self.last_fill > 0;
        self.last_fill = fill;
        let mut cut = Cut {
            strong,
            index: index as u32,
            size,
            motion,
            fill,
        };
        if fill > 0 || strong {
            self.harden(&mut cut);
        } else {
            self.state.hard = u32::from(motion == ShotMove::Cut);
        }
        cut
    }

    /// Make `cut` a strong, hard one: it lands on a strong transition and the scene jumps to the
    /// shot, whatever the shot's `move` (a burst's cuts are hard too, see [`Self::enter`]).
    fn harden(&mut self, cut: &mut Cut) {
        cut.strong = cut.strong || cut.fill == 0;
        cut.motion = ShotMove::Cut;
        self.state.hard = 1;
    }

    /// The shot to cut to, among the `[[shot]]`s (the burst's are never among them), other than
    /// `not` (all of them if the scene has just one), picked by weight with the section's label
    /// and `k` hashed to the roll, so that a section of the same kind picks the same.
    ///
    /// A section start picks among the shots that open a section, by `weight`; with none that
    /// does, the one nearest to wide. Any other cut picks among the shots of another size than
    /// the one on (`top` differs from every size; if every shot is of the one size, any other
    /// shot), each by its `weight` times what `director.sizes` wants of its size in the tier. If
    /// that leaves no chance at all, by `weight` alone.
    fn pick(&self, why: Why, label: f32, k: u32, not: Option<usize>) -> usize {
        let shots = &self.shots[..self.ordinary];
        let candidates: Vec<usize> = (0..shots.len())
            .filter(|i| not != Some(*i) || shots.len() == 1)
            .collect();
        // Only the shots that suit the section's energy, if any does.
        let tier = match why {
            Why::Next(tier) => tier,
            Why::Opening => self.tier_now,
        };
        let suiting: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&i| self.picks[i].suits[tier.index()])
            .collect();
        let candidates = if suiting.is_empty() {
            candidates
        } else {
            suiting
        };
        let weight = |i: usize| self.picks[i].weight;
        // The size of the shot on, which may be one of the burst's (after a burst): its size is in
        // the whole list.
        let on = not.map(|i| self.shots[i].0);
        let chances: Vec<(usize, f32)> = match why {
            Why::Opening => {
                let opening: Vec<usize> =
                    (0..shots.len()).filter(|&i| self.picks[i].opens).collect();
                // The shot on is left out (a cut to it would only jump), unless it is the scene's
                // only shot; with no other that opens, the nearest to wide opens instead.
                let others: Vec<usize> = opening
                    .iter()
                    .copied()
                    .filter(|i| candidates.contains(i))
                    .collect();
                // Of another size than the shot on where one opens (after a burst that ends from
                // above, a view from above again would only seem to move in).
                let fresh: Vec<usize> = others
                    .iter()
                    .copied()
                    .filter(|&i| on != Some(shots[i].0))
                    .collect();
                let others = if fresh.is_empty() { others } else { fresh };
                if others.is_empty() {
                    let nearest = candidates.iter().map(|&i| shots[i].0 as u32).min();
                    let nearest = candidates
                        .iter()
                        .copied()
                        .filter(|&i| Some(shots[i].0 as u32) == nearest);
                    nearest.map(|i| (i, weight(i))).collect()
                } else {
                    others.into_iter().map(|i| (i, weight(i))).collect()
                }
            }
            Why::Next(tier) => {
                let differing: Vec<usize> = candidates
                    .iter()
                    .copied()
                    .filter(|&i| on != Some(shots[i].0))
                    .collect();
                let candidates = if differing.is_empty() {
                    candidates
                } else {
                    differing
                };
                let wanted: Vec<(usize, f32)> = candidates
                    .iter()
                    .map(|&i| {
                        let wish = self.binds.size_wish(tier.index(), shots[i].0 as usize);
                        (i, weight(i) * wish)
                    })
                    .collect();
                if wanted.iter().any(|(_, chance)| *chance > 0.0) {
                    wanted
                } else {
                    candidates.into_iter().map(|i| (i, weight(i))).collect()
                }
            }
        };
        weighted(&chances, label_hash(label, k))
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
            opens: None,
            weight: 1.0,
            longest: None,
            tiers: Vec::new(),
            dutch: None,
            at_least_once: false,
        }
    }

    /// A cutting shot of this size that says when it is picked.
    fn picked(size: ShotSize, opens: Option<bool>, weight: f32) -> Shot {
        Shot {
            opens,
            weight,
            ..shot(size, ShotMove::Cut)
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

    fn held(hold: f32) -> Camera {
        Camera {
            hold,
            ..camera(0.0)
        }
    }

    fn camera(dutch: f32) -> Camera {
        Camera {
            dutch,
            hold: 0.0,
            pace: Pace::Normal,
            dutch_in: DutchIn::All,
        }
    }

    /// The burst's shots of these sizes, in order.
    fn burst_of(sizes: &[ShotSize]) -> Vec<BurstShot> {
        sizes
            .iter()
            .map(|&size| BurstShot {
                name: String::new(),
                size,
            })
            .collect()
    }

    /// The burst most tests declare: four shots, each nearer than the one before but the last.
    fn four() -> Vec<BurstShot> {
        use ShotSize::*;
        burst_of(&[Medium, Close, Detail, Top])
    }

    /// A director over [`each_size`] (six shots, so the burst's are indices 6 and on) that
    /// declares [`four`], whose every lift bursts (`chance = 1`), so that the tests of what a
    /// burst does do not hang on how a section rolls.
    fn bursting(camera: Camera) -> Director {
        let mut director = Director::new(camera, &each_size()).with_bursts(&four());
        with_chance(&mut director, Some(1.0));
        director
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
            level: energy,
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
            hard: 1,
        };
        let bytes = state.bytes();
        assert_eq!(bytes.len() as u64, crate::scene::wgsl::SHOT_SIZE);
        let word = |i: usize| <[u8; 4]>::try_from(&bytes[i * 4..i * 4 + 4]).unwrap();
        assert_eq!(u32::from_le_bytes(word(1)), 2);
        assert_eq!(f32::from_le_bytes(word(3)), 0.5);
        assert_eq!(f32::from_le_bytes(word(6)), -1.0);
        assert_eq!(u32::from_le_bytes(word(7)), 4);
        assert_eq!(u32::from_le_bytes(word(8)), 1);
        assert!(bytes[36..].iter().all(|b| *b == 0));
    }

    fn glides() -> Vec<Shot> {
        each_size()
            .into_iter()
            .map(|s| shot(s.size, ShotMove::Glide))
            .collect()
    }

    #[test]
    fn an_ordinary_cut_to_a_glide_shot_glides_and_a_cut_shot_jumps() {
        let base = song(0.5);
        let mut director = Director::new(camera(0.0), &glides());
        assert_eq!(
            director.state().hard,
            1,
            "the first shot has nothing to glide from"
        );
        let cuts = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(cuts.len(), 1);
        assert_eq!(
            (cuts[0].1.motion, cuts[0].1.strong, director.state().hard),
            (ShotMove::Glide, false, 0)
        );
        let mut director = Director::new(camera(0.0), &each_size());
        let cuts = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(
            (cuts[0].1.motion, director.state().hard),
            (ShotMove::Cut, 1)
        );
    }

    #[test]
    fn a_strong_cut_and_a_burst_are_hard_even_in_a_gliding_scene() {
        let mut director = Director::new(camera(0.3), &glides()).with_bursts(&four());
        with_chance(&mut director, Some(1.0));
        let mut states = Vec::new();
        let base = song(0.2);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        states.push(director.state().hard);
        let coming = MusicValues {
            next_energy: 0.9,
            ..base
        };
        for beat in 1..24 {
            let cuts = beats(&mut director, beat, 1, &coming, |b, m| MusicValues {
                cue_in: (24 - b) as f32 * 0.5,
                ..m
            });
            if !cuts.is_empty() {
                states.push(director.state().hard);
            }
            all.extend(cuts);
        }
        let high = song(0.9);
        let cuts = beats(&mut director, 24, 1, &high, |_, m| start(1.0, m));
        states.push(director.state().hard);
        all.extend(cuts);
        // The opening glides; the four burst cuts and the cut they lead to jump.
        assert_eq!(all.len(), 6, "{all:?}");
        assert_eq!(all[0].1.motion, ShotMove::Glide);
        assert!(all[1..].iter().all(|(_, c)| c.motion == ShotMove::Cut));
        assert!(all.last().unwrap().1.strong);
        assert_eq!(states, [0, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn the_start_of_a_high_section_is_a_hard_cut_to_a_glide_shot_without_a_burst() {
        let base = song(0.9);
        let mut director = Director::new(camera(0.0), &glides());
        let cuts = beats(&mut director, 0, 1, &base, |_, m| start(1.0, m));
        let (_, cut) = cuts[0];
        assert_eq!((cut.motion, cut.strong, cut.fill), (ShotMove::Cut, true, 0));
        assert_eq!(director.state().hard, 1);
    }

    #[test]
    fn a_lab_burst_and_the_opening_after_it_are_hard_in_a_gliding_scene() {
        let mut director = Director::new(camera(0.3), &glides()).with_bursts(&four());
        director.start_burst();
        let all = beats(&mut director, 0, 6, &song(0.2), plain);
        assert!(all.len() >= 5, "{all:?}");
        assert!(all[..5].iter().all(|(_, c)| c.motion == ShotMove::Cut));
        assert!(all[4].1.strong);
    }

    #[test]
    fn a_shot_held_by_hand_reports_how_it_was_entered() {
        let mut director = Director::new(camera(0.0), &glides()).with_bursts(&four());
        let music = song(0.5);
        let cut = director.hold_shot(2, &music).unwrap();
        assert_eq!((cut.motion, director.state().hard), (ShotMove::Glide, 0));
        let cut = director.hold_shot(7, &music).unwrap();
        assert_eq!((cut.motion, director.state().hard), (ShotMove::Cut, 1));
    }

    #[test]
    fn it_starts_on_a_wide_shot_before_any_cut() {
        let director = Director::new(camera(0.3), &each_size());
        let state = director.state();
        assert_eq!((state.size, state.cuts), (0, 0));
        assert_eq!((state.roll, state.length, state.fill), (0.0, -1.0, 0));
    }

    /// The beats the cadence cut on in the `count` beats after a section started at beat 1 (a
    /// beat is 0.5 s at 120 bpm), at this pace and energy.
    fn paced_cut_beats(pace: Pace, energy: f32, count: u32) -> Vec<u32> {
        let camera = Camera {
            pace,
            ..camera(0.0)
        };
        let mut director = Director::new(camera, &each_size());
        let base = song(energy);
        beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
        beats(&mut director, 2, count, &base, plain)
            .iter()
            .map(|(beat, _)| *beat)
            .collect()
    }

    #[test]
    fn the_scene_pace_scales_the_seconds_between_cuts() {
        use Pace::*;
        // A normal section: 16 s, calm is 1.5 times that (24 s) and busy a third less (10.7 s).
        let first = |pace| paced_cut_beats(pace, 0.5, 128)[0];
        assert_eq!([first(Calm), first(Normal), first(Busy)], [52, 36, 24]);
        let all = |pace| paced_cut_beats(pace, 0.5, 128);
        assert_eq!(all(Calm), [52, 100]);
        assert_eq!(all(Normal), [36, 68, 100]);
        assert_eq!(all(Busy), [24, 48, 72, 96, 120]);
        // Only at a section's start: none within it, whatever the energy.
        assert!(paced_cut_beats(Sections, 0.9, 128).is_empty());
    }

    #[test]
    fn calm_is_one_and_a_half_times_normal() {
        let director = |pace| {
            let camera = Camera {
                pace,
                ..camera(0.0)
            };
            Director::new(camera, &each_size())
        };
        for tier in [Tier::Quiet, Tier::Normal, Tier::High] {
            let normal = director(Pace::Normal).cadence_seconds(tier).unwrap();
            let calm = director(Pace::Calm).cadence_seconds(tier).unwrap();
            assert_eq!(calm, normal * 1.5, "{}", tier.name());
        }
        assert_eq!(director(Pace::Sections).cadence_seconds(Tier::Normal), None);
    }

    #[test]
    fn the_cadence_counts_seconds_not_bars() {
        // A slow and a fast song cut after the same 16 s of a normal section: it did after 4
        // bars, which are 16 s at 60 bpm and 4 s at 240.
        let first_cut = |tempo: f32| {
            let beat = 60.0 / tempo;
            let mut director = Director::new(camera(0.0), &each_size());
            let base = MusicValues { tempo, ..song(0.5) };
            let first = MusicValues {
                beats: 1,
                bars: 1,
                ..base
            };
            director.tick(beat, &start(0.0, first));
            (1..400u32)
                .find(|beat_number| {
                    let frame = MusicValues {
                        beats: 1,
                        bars: u32::from(beat_number % 4 == 0),
                        ..base
                    };
                    director.tick(beat, &frame).is_some()
                })
                .map(|beat_number| beat_number as f32 * beat)
        };
        assert_eq!(first_cut(60.0), Some(16.0));
        assert_eq!(first_cut(240.0), Some(16.0));
    }

    #[test]
    fn a_scene_of_sections_cuts_at_the_next_section_and_without_a_beat_never_on_time() {
        let camera = Camera {
            pace: Pace::Sections,
            ..camera(0.0)
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
            ..camera(0.3)
        };
        assert!(Director::new(busy, &each_size()).phrase_bursts);
        assert!(!Director::new(camera(0.3), &each_size()).phrase_bursts);
    }

    #[test]
    fn cuts_follow_the_seconds_of_the_section_energy() {
        // The first downbeat once 24, 16 and 10 s have passed since the section started.
        let first = |energy| paced_cut_beats(Pace::Normal, energy, 80)[0];
        assert_eq!([first(0.2), first(0.5), first(0.8)], [52, 36, 24]);
    }

    #[test]
    fn cuts_land_on_downbeats() {
        let mut director = Director::new(camera(0.0), &each_size());
        let base = song(0.8);
        beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
        let cuts = beats(&mut director, 2, 120, &base, plain);
        assert!(cuts.len() >= 5);
        assert!(cuts.iter().all(|(beat, _)| beat % 4 == 0));
    }

    /// Frames of 0.1 s with a hit every `hit_every` frames (0 for none), without a beat, for
    /// `seconds`; the seconds each cut happened at.
    fn unfollowed(energy: f32, hit_every: u32, seconds: f32) -> Vec<f32> {
        unfollowed_held(0.0, energy, hit_every, seconds)
    }

    fn unfollowed_held(hold: f32, energy: f32, hit_every: u32, seconds: f32) -> Vec<f32> {
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
        let normal = unfollowed(0.5, 7, 17.0);
        assert!((normal[0] - 16.1).abs() < 0.11, "{normal:?}");
        let high = unfollowed(0.8, 7, 22.0);
        assert!((high[0] - 10.5).abs() < 0.11, "{high:?}");
        assert_eq!(high.len(), 2, "{high:?}");
    }

    #[test]
    fn without_a_beat_or_a_hit_a_cut_waits_two_seconds_more() {
        let cuts = unfollowed(0.8, 0, 30.0);
        assert!((cuts[0] - 12.0).abs() < 0.11, "{cuts:?}");
        assert!((cuts[1] - cuts[0] - 12.0).abs() < 0.11, "{cuts:?}");
        let cuts = unfollowed(0.2, 0, 30.0);
        assert!((cuts[0] - 26.0).abs() < 0.11, "{cuts:?}");
    }

    #[test]
    fn without_a_chart_the_drive_gives_the_energy() {
        let mut director = Director::new(camera(0.0), &each_size());
        let base = MusicValues {
            has_chart: 0,
            section_energy: 0.0,
            drive: 0.9,
            ..song(0.9)
        };
        // High by `drive` although the section energy is 0 (quiet, 24 s): the first cut is on
        // the first downbeat after 10 s.
        let cuts = beats(&mut director, 1, 60, &base, plain);
        assert_eq!(cuts.first().map(|(beat, _)| *beat), Some(20), "{cuts:?}");
    }

    #[test]
    fn a_section_starts_on_a_shot_that_opens_and_each_cut_is_another_size() {
        let mut director = Director::new(camera(0.0), &each_size());
        let base = song(0.5);
        let first = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(first[0].1.size, ShotSize::Wide);
        let sizes: Vec<ShotSize> = beats(&mut director, 1, 192, &base, plain)
            .into_iter()
            .map(|(_, cut)| cut.size)
            .collect();
        assert!(sizes.len() >= 5);
        let mut last = ShotSize::Wide;
        for size in sizes {
            assert_ne!(size, last);
            last = size;
        }
    }
    #[test]
    fn a_new_section_opens_again_after_cuts_within_the_last() {
        let mut director = Director::new(camera(0.0), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        // Cadence cuts at beats 32 and 64, then a section starts 13 s after the last.
        let before = beats(&mut director, 1, 89, &base, plain);
        assert_eq!(before.len(), 2);
        let again = beats(&mut director, 90, 1, &base, |_, m| start(1.0, m));
        assert_eq!(again[0].1.size, ShotSize::Wide);
        assert_eq!(director.in_section, 1);
    }
    #[test]
    fn a_weak_cue_is_not_a_section_start() {
        let mut director = Director::new(camera(0.0), &each_size());
        let base = song(0.2);
        let cuts = beats(&mut director, 1, 12, &base, |_, m| MusicValues {
            cues: 1,
            cue: 0.3,
            ..m
        });
        assert!(cuts.is_empty());
    }

    /// How many of the `labels` (0, 1, 2, ...) pick each size of `each_size()`'s shots as the cut
    /// after the top shot (index 4), in a tier, at the cut number `k`, with the runtime's defaults.
    fn size_counts(tier: Tier, labels: u32) -> [u32; 5] {
        let director = Director::new(camera(0.0), &each_size());
        let mut counts = [0; 5];
        for label in 0..labels {
            let index = director.pick(Why::Next(tier), label as f32, 1, Some(4));
            counts[director.shots[index].0 as usize] += 1;
        }
        counts
    }

    #[test]
    fn a_high_section_picks_close_and_detail_more_often_and_a_quiet_one_wide_and_medium() {
        let quiet = size_counts(Tier::Quiet, 600);
        let high = size_counts(Tier::High, 600);
        let near = |c: [u32; 5]| c[2] + c[3];
        let far = |c: [u32; 5]| c[0] + c[1];
        assert!(far(quiet) > 2 * near(quiet), "{quiet:?}");
        assert!(near(high) > far(high), "{high:?}");
        assert!(near(high) > 3 * near(quiet), "{quiet:?} {high:?}");
        // Detail has no wish in a quiet section: never.
        assert_eq!(quiet[3], 0, "{quiet:?}");
        // Top is wanted in every tier: after a wide shot it is picked in each.
        let director = Director::new(camera(0.0), &each_size());
        for tier in [Tier::Quiet, Tier::Normal, Tier::High] {
            assert!((0..300).any(|label| {
                let to = director.pick(Why::Next(tier), label as f32, 1, Some(0));
                director.shots[to].0 == ShotSize::Top
            }));
        }
    }

    #[test]
    fn a_section_start_only_picks_shots_that_open_and_a_top_can() {
        use ShotSize::*;
        let shots = [
            picked(Wide, None, 1.0),
            picked(Medium, None, 1.0),
            picked(Top, Some(true), 1.0),
            picked(Wide, Some(false), 1.0),
            picked(Close, None, 1.0),
        ];
        let director = Director::new(camera(0.0), &shots);
        let mut seen = [0; 5];
        for label in 0..200 {
            seen[director.pick(Why::Opening, label as f32, 0, Some(1))] += 1;
        }
        // Only the wide shot that opens and the top that says so.
        assert_eq!(seen[1] + seen[3] + seen[4], 0, "{seen:?}");
        assert!(seen[0] > 0 && seen[2] > 0, "{seen:?}");
        // The shot on is not picked again: the other that opens is, or with none the nearest to
        // wide (a cut to the shot on would only jump).
        for label in 0..50 {
            assert_eq!(director.pick(Why::Opening, label as f32, 0, Some(0)), 2);
        }
        let one = [picked(Wide, None, 1.0), picked(Close, None, 1.0)];
        let director = Director::new(camera(0.0), &one);
        assert_eq!(director.pick(Why::Opening, 0.3, 0, Some(0)), 1);
    }

    #[test]
    fn a_section_start_without_a_shot_that_opens_takes_the_nearest_to_wide() {
        use ShotSize::*;
        let shots = [Top, Close, Medium, Detail].map(|size| shot(size, ShotMove::Cut));
        let director = Director::new(camera(0.0), &shots);
        assert_eq!(director.state().index, 2);
        for label in 0..20 {
            assert_eq!(director.pick(Why::Opening, label as f32, 0, Some(0)), 2);
        }
        // Not the shot on: the next nearest.
        assert_eq!(director.pick(Why::Opening, 0.5, 0, Some(2)), 1);
        let only_top = [shot(Top, ShotMove::Cut), shot(Top, ShotMove::Cut)];
        let director = Director::new(camera(0.0), &only_top);
        assert_eq!(director.pick(Why::Opening, 0.5, 0, Some(0)), 1);
    }

    #[test]
    fn a_cut_in_a_section_never_keeps_the_size_while_another_size_exists() {
        use ShotSize::*;
        let shots = [Wide, Wide, Medium, Medium, Close, Detail, Top, Top]
            .map(|size| shot(size, ShotMove::Cut));
        let director = Director::new(camera(0.0), &shots);
        for tier in [Tier::Quiet, Tier::Normal, Tier::High] {
            for current in 0..shots.len() {
                for label in 0..40 {
                    for k in 1..4 {
                        let to =
                            director.pick(Why::Next(tier), label as f32 * 0.37, k, Some(current));
                        assert_ne!(shots[to].size, shots[current].size, "{tier:?} {current}");
                    }
                }
            }
        }
        // With one size only, any other shot; with one shot, the same.
        let same = [shot(Medium, ShotMove::Cut), shot(Medium, ShotMove::Cut)];
        let director = Director::new(camera(0.0), &same);
        assert_eq!(director.pick(Why::Next(Tier::Normal), 0.1, 1, Some(0)), 1);
        let alone = [shot(Medium, ShotMove::Cut)];
        let director = Director::new(camera(0.0), &alone);
        assert_eq!(director.pick(Why::Next(Tier::Normal), 0.1, 1, Some(0)), 0);
    }

    #[test]
    fn a_shot_of_weight_0_is_never_picked_while_another_has_a_chance() {
        use ShotSize::*;
        let shots = [
            picked(Wide, None, 1.0),
            picked(Wide, None, 0.0),
            picked(Medium, None, 0.0),
            picked(Close, None, 1.0),
            picked(Detail, None, 2.0),
        ];
        let director = Director::new(camera(0.0), &shots);
        for tier in [Tier::Quiet, Tier::Normal, Tier::High] {
            for label in 0..200 {
                let label = label as f32 * 0.173;
                for current in [0, 3, 4] {
                    let to = director.pick(Why::Next(tier), label, 1, Some(current));
                    assert!(to != 1 && to != 2, "{tier:?} {current} {label}");
                }
                assert_ne!(director.pick(Why::Opening, label, 0, Some(3)), 1);
            }
        }
        // A weight is a share: the detail shot (2) against the close one (1) in a high section.
        let (mut close, mut detail) = (0, 0);
        for label in 0..600 {
            match director.pick(Why::Next(Tier::High), label as f32, 1, Some(0)) {
                3 => close += 1,
                4 => detail += 1,
                other => panic!("{other}"),
            }
        }
        // Wishes 3 and 2 times weights 1 and 2: 3 against 4.
        assert!(detail > close, "{close} {detail}");
    }

    #[test]
    fn when_no_candidate_is_wanted_the_weights_alone_decide_and_then_evenly() {
        use ShotSize::*;
        // Detail is not wanted in a quiet section (0): the only candidate still gets picked.
        let shots = [picked(Wide, None, 1.0), picked(Detail, None, 1.0)];
        let director = Director::new(camera(0.0), &shots);
        assert_eq!(director.pick(Why::Next(Tier::Quiet), 0.4, 1, Some(0)), 1);
        // Two of them, one of weight 0: the other, by weight alone.
        let shots = [
            picked(Wide, None, 1.0),
            picked(Detail, None, 0.0),
            picked(Detail, None, 3.0),
        ];
        let director = Director::new(camera(0.0), &shots);
        for label in 0..50 {
            assert_eq!(
                director.pick(Why::Next(Tier::Quiet), label as f32 * 0.31, 1, Some(0)),
                2
            );
        }
        // Weights all 0 among the candidates: evenly.
        let shots = [
            picked(Wide, None, 1.0),
            picked(Detail, None, 0.0),
            picked(Detail, None, 0.0),
        ];
        let director = Director::new(camera(0.0), &shots);
        let picks: Vec<usize> = (0..100)
            .map(|label| director.pick(Why::Next(Tier::Quiet), label as f32 * 0.07, 1, Some(0)))
            .collect();
        assert!(picks.contains(&1) && picks.contains(&2));
    }

    #[test]
    fn the_same_label_and_cut_number_pick_the_same_shot() {
        let a = Director::new(camera(0.0), &each_size());
        let b = Director::new(camera(0.0), &each_size());
        for label in 0..30 {
            for k in 0..6 {
                let (label, current) = (label as f32, (k as usize + 2) % 6);
                assert_eq!(
                    a.pick(Why::Next(Tier::High), label, k, Some(current)),
                    b.pick(Why::Next(Tier::High), label, k, Some(current))
                );
                assert_eq!(
                    a.pick(Why::Opening, label, k, Some(current)),
                    b.pick(Why::Opening, label, k, Some(current))
                );
            }
        }
        // And the choice varies with the label.
        let picks: std::collections::HashSet<usize> = (0..30)
            .map(|label| a.pick(Why::Next(Tier::Normal), label as f32, 1, Some(4)))
            .collect();
        assert!(picks.len() >= 3);
    }

    #[test]
    fn the_weighted_pick_follows_the_shares() {
        let chances = [(7, 1.0), (8, 0.0), (9, 3.0)];
        assert_eq!(weighted(&chances, 0.0), 7);
        assert_eq!(weighted(&chances, 0.24), 7);
        assert_eq!(weighted(&chances, 0.26), 9);
        assert_eq!(weighted(&chances, 0.999_999), 9);
        assert_eq!(weighted(&[(4, 0.0), (5, 0.0)], 0.0), 4);
        assert_eq!(weighted(&[(4, 0.0), (5, 0.0)], 0.99), 5);
        assert_eq!(weighted(&[(4, 0.0), (5, 2.0)], 0.0), 5);
    }

    #[test]
    fn a_shot_is_never_twice_in_a_row() {
        // Two shots that open (a section start never repeats the one on) and pairs of the others.
        use ShotSize::*;
        let shots = [Wide, Wide, Medium, Medium, Close, Close, Detail]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        for energy in [0.2, 0.5, 0.9] {
            let mut director = Director::new(camera(0.3), &shots);
            let base = song(energy);
            let mut last = director.state().index;
            let mut cuts = 0;
            // Sections 60 s apart, so that each starts well past the gap after the cadence's cuts.
            for section in 0..6 {
                let mut all = beats(&mut director, section * 120, 1, &base, |_, m| {
                    start(section as f32 % 3.0, m)
                });
                all.extend(beats(&mut director, section * 120 + 1, 119, &base, plain));
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
        let mut director = Director::new(camera(0.4), &shots);
        let base = song(0.9);
        let cuts = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(cuts[0].1.index, 0);
        // The start of a high section is a hard cut, even to a glide shot.
        assert_eq!(cuts[0].1.motion, ShotMove::Cut);
        let mut seeds = vec![director.state().seed];
        for beat in 1..130u32 {
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
    fn a_song_picks_the_same_shots_each_time_it_plays_and_each_kind_of_section_its_own() {
        use ShotSize::*;
        let shots = [
            Wide, Wide, Medium, Medium, Close, Close, Detail, Detail, Top,
        ]
        .map(|size| shot(size, ShotMove::Cut))
        .to_vec();
        // A play of sections with these labels, 60 s apart (well past the gap after the
        // cadence's cuts): the shots it cuts to, section by section.
        let play = |labels: &[f32]| -> Vec<Vec<u32>> {
            let mut director = Director::new(camera(0.0), &shots);
            let mut from = 0;
            labels
                .iter()
                .map(|&label| {
                    let base = MusicValues {
                        section_label: label,
                        ..song(0.9)
                    };
                    let mut all = beats(&mut director, from, 1, &base, |_, m| start(label, m));
                    all.extend(beats(&mut director, from + 1, 119, &base, plain));
                    from += 120;
                    all.iter().map(|(_, cut)| cut.index).collect()
                })
                .collect()
        };
        let song = play(&[2.0, 3.0, 2.0]);
        assert!(song[0].len() >= 5);
        assert_eq!(song, play(&[2.0, 3.0, 2.0]));
        // The chorus comes back cut the same way, whatever came between. Here the sections end on
        // the wide shot the chorus opens with, so the second chorus opens on the other wide one
        // (a cut to the shot on would only jump); the rest of its sequence is the same.
        assert_eq!(song[0][1..], song[2][1..]);
        assert_ne!(song[2][0], song[1][song[1].len() - 1]);
        assert_eq!(play(&[2.0, 5.0, 2.0])[2][1..], song[0][1..]);
        assert_ne!(play(&[2.0])[0], play(&[3.0])[0]);
    }

    #[test]
    fn a_scene_that_tilts_in_bursts_only_keeps_its_other_cuts_level() {
        let mut director = bursting(Camera {
            dutch_in: DutchIn::Bursts,
            ..camera(0.4)
        });
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let mut level = 0;
        for (_, cut) in beats(&mut director, 1, 200, &base, plain) {
            assert_eq!(cut.fill, 0);
            assert_eq!(director.state().roll, 0.0, "{cut:?}");
            level += 1;
        }
        assert!(level > 3);
        // A burst's near cuts tilt.
        director.start_burst();
        let mut tilted = 0;
        for beat in 201..204 {
            for (_, cut) in beats(&mut director, beat, 1, &base, plain) {
                if cut.fill > 0
                    && matches!(
                        cut.size,
                        ShotSize::Medium | ShotSize::Close | ShotSize::Detail
                    )
                {
                    assert!(director.state().roll.abs() > 0.0, "{cut:?}");
                    tilted += 1;
                }
            }
        }
        assert!(tilted >= 2);
    }

    #[test]
    fn a_shot_may_say_for_itself_whether_it_tilts() {
        use ShotSize::*;
        let shots = |dutch| {
            vec![
                picked(Wide, Some(true), 1.0),
                Shot {
                    dutch: Some(dutch),
                    ..picked(Close, None, 1.0)
                },
                picked(Medium, None, 1.0),
            ]
        };
        // Each beat's cuts: the size cut to and the roll it got.
        let rolls = |dutch_in, dutch| {
            let camera = Camera {
                dutch_in,
                ..camera(0.4)
            };
            let mut director = Director::new(camera, &shots(dutch));
            let base = song(0.2);
            beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
            let mut all = Vec::new();
            for beat in 1..400 {
                for (_, cut) in beats(&mut director, beat, 1, &base, plain) {
                    all.push((cut.size, director.state().roll));
                }
            }
            all
        };
        // In a scene that tilts only its bursts, the shot that says so tilts and the others not.
        let all = rolls(DutchIn::Bursts, true);
        assert!(all.iter().any(|&(size, _)| size == Close), "{all:?}");
        assert!(all.iter().any(|&(size, _)| size == Medium), "{all:?}");
        for &(size, roll) in &all {
            assert_eq!(roll != 0.0, size == Close, "{all:?}");
        }
        // In one that tilts every cut, the shot that says not stays level.
        let all = rolls(DutchIn::All, false);
        for &(size, roll) in &all {
            assert_eq!(roll != 0.0, size == Medium, "{all:?}");
        }
    }

    #[test]
    fn wide_and_top_never_tilt_and_the_others_alternate() {
        let mut director = Director::new(camera(0.4), &each_size());
        let base = song(0.9);
        let mut rolls = Vec::new();
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        for beat in 1..400u32 {
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
        let mut director = Director::new(camera(0.0), &each_size());
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
        let mut director = Director::new(camera(0.0), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        // A normal section's 16 s.
        assert_eq!(director.state().length, 16.0);
        assert_eq!(director.state().age, 0.0);
        beats(&mut director, 1, 2, &base, plain);
        assert_eq!(director.state().age, 1.0);
        assert_eq!(director.state().cuts, 1);
        let unfollowed = MusicValues {
            locked: 0.0,
            ..base
        };
        director.force_cut(&unfollowed);
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
        let mut director = bursting(camera(0.3));
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
        // The burst's shots come after the scene's six.
        assert_eq!(
            burst.iter().map(|(_, c)| c.index).collect::<Vec<_>>(),
            [6, 7, 8, 9]
        );
        // And the section start is a wide shot, with the progression started again.
        let (beat, last) = all.last().unwrap();
        assert_eq!((*beat, last.size, last.fill), (24, Wide, 0));
        // Ten seconds: the section is a high one.
        assert_eq!(director.state().length, 10.0);
    }

    #[test]
    fn a_burst_is_only_its_declared_shots_in_order() {
        use ShotSize::*;
        // The scene has no shot of the burst's sizes in the order it goes through them, and a
        // wide, medium and close shot of its own.
        let shots = [Wide, Medium, Close]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        let declared = burst_of(&[Detail, Top, Medium]);
        let mut director = Director::new(camera(0.3), &shots).with_bursts(&declared);
        let all = lift_run(&mut director);
        let burst: Vec<&(u32, Cut)> = all.iter().filter(|(_, c)| c.fill > 0).collect();
        // One cut on each of the last three beats, to the shots after the scene's own.
        assert_eq!(
            burst.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            [21, 22, 23]
        );
        assert_eq!(
            burst
                .iter()
                .map(|(_, c)| (c.index, c.size, c.fill))
                .collect::<Vec<_>>(),
            [(3, Detail, 1), (4, Top, 2), (5, Medium, 3)]
        );
        assert!(burst.iter().all(|(_, c)| c.motion == ShotMove::Cut));
    }

    #[test]
    fn a_burst_cuts_once_on_each_declared_shot_from_one_to_four() {
        use ShotSize::*;
        for count in 1..=4usize {
            let declared = burst_of(&[Medium, Close, Detail, Top][..count]);
            let mut director = Director::new(camera(0.3), &each_size()).with_bursts(&declared);
            let all = lift_run(&mut director);
            let burst: Vec<&(u32, Cut)> = all.iter().filter(|(_, c)| c.fill > 0).collect();
            let beats: Vec<u32> = (24 - count as u32..24).collect();
            assert_eq!(burst.iter().map(|(b, _)| *b).collect::<Vec<_>>(), beats);
            assert_eq!(
                burst.iter().map(|(_, c)| c.fill).collect::<Vec<_>>(),
                (1..=count as u32).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn ordinary_cuts_never_pick_a_burst_shot_nor_a_lab_hold_one() {
        use ShotSize::*;
        // The burst's shots are the only close and detail ones: nothing ordinary may reach them.
        let shots = [Wide, Medium, Top]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        let mut director =
            Director::new(camera(0.3), &shots).with_bursts(&burst_of(&[Close, Detail]));
        let base = song(0.9);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 400, &base, plain));
        all.extend(beats(&mut director, 401, 1, &base, |_, m| start(1.0, m)));
        all.extend(beats(&mut director, 402, 100, &base, plain));
        assert!(all.len() > 15);
        for (beat, cut) in &all {
            assert!(cut.index < 3, "beat {beat}: {cut:?}");
            assert!(!matches!(cut.size, Close | Detail), "beat {beat}: {cut:?}");
        }
        for _ in 0..20 {
            assert!(director.force_cut(&base).index < 3);
        }
        // A lab lists the burst's shots after the scene's and may hold one, told as its place in
        // the burst, to frame it.
        assert_eq!(director.shot_list().len(), 5);
        assert_eq!(director.shot_list()[3].2, 1);
        assert_eq!(director.shot_list()[2].2, 0);
        assert_eq!(
            director.hold_shot(4, &base).map(|c| (c.index, c.fill)),
            Some((4, 2))
        );
        assert!(director.hold_shot(5, &base).is_none());
        assert!(director.hold_shot(2, &base).is_some_and(|c| c.fill == 0));
    }

    #[test]
    fn no_burst_is_declared_no_burst_ever() {
        // A scene with no `[[burst]]`: not before a lift, not at a phrase's end, not by hand.
        let mut director = Director::new(camera(0.3), &each_size());
        let all = lift_run(&mut director);
        assert!(all.iter().all(|(_, c)| c.fill == 0), "{all:?}");
        let busy = Camera {
            pace: Pace::Busy,
            ..camera(0.3)
        };
        let mut director = Director::new(busy, &each_size());
        let base = song(0.9);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 100, &base, plain));
        director.start_burst();
        all.extend(beats(&mut director, 101, 12, &base, plain));
        assert!(all.len() > 5);
        assert!(all.iter().all(|(_, c)| c.fill == 0), "{all:?}");
        assert_eq!(director.forced_burst, None);
    }

    #[test]
    fn a_burst_started_by_hand_cuts_on_the_next_four_beats_and_then_opens_a_section() {
        let mut director = bursting(camera(0.3));
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        director.start_burst();
        let all = beats(&mut director, 1, 8, &base, plain);
        use ShotSize::*;
        assert_eq!(
            all.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5],
            "{all:?}"
        );
        assert_eq!(
            all.iter().map(|(_, c)| c.fill).collect::<Vec<_>>(),
            [1, 2, 3, 4, 0]
        );
        // The beat after the last opens a section, as the section start after a burst would, and
        // it is the cut that closes the burst (the scene lands it on a strong transition).
        assert_eq!(
            all.iter().map(|(_, c)| c.size).collect::<Vec<_>>(),
            [Medium, Close, Detail, Top, Wide]
        );
        assert_eq!(
            all.iter().map(|(_, c)| c.strong).collect::<Vec<_>>(),
            [false, false, false, false, true]
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
    fn the_cut_after_a_burst_started_by_hand_picks_one_of_the_scenes_shots() {
        let mut director = bursting(camera(0.3));
        let ordinary = director.ordinary;
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        director.start_burst();
        let all = beats(&mut director, 1, 80, &base, plain);
        let after: Vec<_> = all.iter().filter(|(_, c)| c.fill == 0).collect();
        assert!(!after.is_empty(), "{all:?}");
        assert!(after.iter().all(|(_, c)| (c.index as usize) < ordinary));
    }

    #[test]
    fn a_section_start_opens_with_another_size_than_the_shot_on_where_it_can() {
        use ShotSize::*;
        let shots = [
            picked(Wide, None, 1.0),
            picked(Top, Some(true), 1.0),
            picked(Medium, None, 1.0),
        ];
        let director = Director::new(camera(0.0), &shots).with_bursts(&[BurstShot {
            name: String::new(),
            size: Top,
        }]);
        // On the burst's view from above, every label opens with the wide shot, not the top.
        for label in 0..50 {
            assert_eq!(director.pick(Why::Opening, label as f32, 0, Some(3)), 0);
        }
    }

    #[test]
    fn a_section_start_on_the_only_shot_that_opens_cuts_to_another() {
        use ShotSize::*;
        let shots: Vec<Shot> = [Wide, Medium, Close]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        let mut director = Director::new(camera(0.0), &shots);
        let base = song(0.5);
        // The director starts on the wide shot, the only one that opens.
        assert_eq!(director.state.index, 0);
        let cut = beats(&mut director, 0, 1, &base, |_, m| start(1.0, m));
        assert_eq!(cut.len(), 1);
        assert_ne!(cut[0].1.index, 0);
    }

    #[test]
    fn a_burst_started_by_hand_without_a_followed_beat_cuts_every_quarter_second() {
        let mut director = bursting(camera(0.3));
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
        // At once, then every 0.25 s (4 frames of 0.0625 s), and the opening after it.
        assert_eq!(cuts, [(0, 1), (4, 2), (8, 3), (12, 4), (16, 0)]);
    }

    #[test]
    fn a_burst_tilts_alternately() {
        let mut director = bursting(camera(0.3));
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
        let mut director = Director::new(camera(0.3), &glides()).with_bursts(&four());
        let all = lift_run(&mut director);
        assert!(all.iter().any(|(_, c)| c.fill > 0));
        for (_, cut) in &all {
            let want = if cut.fill > 0 || cut.strong {
                ShotMove::Cut
            } else {
                ShotMove::Glide
            };
            assert_eq!(cut.motion, want, "{all:?}");
        }
    }

    #[test]
    fn no_burst_without_the_conditions() {
        let count = |director: &mut Director| {
            lift_run(director)
                .iter()
                .filter(|(_, c)| c.fill > 0)
                .count()
        };
        // The scene declares none.
        assert_eq!(count(&mut Director::new(camera(0.3), &each_size())), 0);
        assert_eq!(count(&mut bursting(camera(0.3))), 4);
        // The energy does not rise enough.
        let mut director = bursting(camera(0.3));
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
            let mut director = bursting(camera(0.3));
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
        let mut director = bursting(camera(0.3));
        let unknown = MusicValues {
            next_energy: 0.9,
            ..song(0.5)
        };
        let all = beats(&mut director, 1, 23, &unknown, plain);
        assert!(all.iter().all(|(_, c)| c.fill == 0));
    }

    #[test]
    fn a_burst_ends_a_phrase_of_a_high_section_and_the_next_phrase_starts_wide() {
        // Only at a section's start, so that no cadence cut falls among the phrase's.
        let sections = Camera {
            pace: Pace::Sections,
            ..camera(0.3)
        };
        let mut director = bursting(sections);
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
        // The same declared shots as the burst before a lift.
        assert_eq!(
            burst.iter().map(|(_, c)| c.index).collect::<Vec<_>>(),
            [6, 7, 8, 9]
        );
        // The phrase starts on the downbeat of beat 32 with a wide shot.
        let at_32 = all.iter().find(|(b, _)| *b == 32).unwrap();
        assert_eq!((at_32.1.size, at_32.1.fill), (ShotSize::Wide, 0));
    }

    #[test]
    fn no_phrase_burst_by_default() {
        let mut director = bursting(camera(0.3));
        let base = song(0.9);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 200, &base, plain));
        assert!(all.len() > 10);
        assert!(all.iter().all(|(_, c)| c.fill == 0), "{all:?}");
    }

    #[test]
    fn hold_is_the_shortest_shot() {
        // A high section cuts every 10 s; a hold of 16 s makes it every 16 (beats 32 and 64).
        let mut director = Director::new(held(16.0), &each_size());
        let base = song(0.9);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let cuts = beats(&mut director, 1, 70, &base, plain);
        assert_eq!(cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(), [32, 64]);
        assert_eq!(director.state().length, 16.0);
        // A hold shorter than the section's cadence changes nothing.
        let mut director = Director::new(held(4.0), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let cuts = beats(&mut director, 1, 70, &base, plain);
        assert_eq!(
            cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            [20, 40, 60]
        );
    }

    #[test]
    fn without_a_beat_the_wait_grows_with_hold() {
        // A hold of 12 s is over the high section's 10 s; with no hit, 2 s more.
        let cuts = unfollowed_held(12.0, 0.8, 0, 30.0);
        assert!((cuts[0] - 14.0).abs() < 0.11, "{cuts:?}");
        assert!((cuts[1] - cuts[0] - 14.0).abs() < 0.11, "{cuts:?}");
        // A hold under the section's wait changes nothing.
        let cuts = unfollowed_held(4.0, 0.8, 0, 14.0);
        assert!((cuts[0] - 12.0).abs() < 0.11, "{cuts:?}");
    }

    #[test]
    fn no_phrase_burst_outside_a_high_section() {
        let mut director = Director::new(camera(0.3), &each_size());
        let base = song(0.5);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 80, &base, plain));
        assert!(all.iter().all(|(_, c)| c.fill == 0));
    }

    #[test]
    fn a_forced_cut_follows_the_pick_rules_and_is_reported() {
        let mut director = Director::new(camera(0.0), &each_size());
        let base = song(0.5);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert_eq!(director.state().cuts, 1);
        // Each is another size than the one before, and one of the scene's shots.
        let mut last = director.state().size;
        let mut cuts = Vec::new();
        for _ in 0..3 {
            let cut = director.force_cut(&base);
            assert_ne!(cut.size as u32, last);
            assert!(cut.index < 6);
            last = cut.size as u32;
            cuts.push(cut);
        }
        let state = *director.state();
        assert_eq!(state.cuts, 4);
        assert_eq!(state.size, last);
        assert_eq!(state.index, cuts[2].index);
        assert_eq!(state.age, 0.0);
        assert_eq!(state.fill, 0);
        // The cadence counts from the forced cut: 16 s later, not before.
        let cuts = beats(&mut director, 1, 40, &base, plain);
        assert_eq!(cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(), [32]);
        // The tier shows in the readout.
        assert_eq!(director.view(&base).tier, "normal");
        assert_eq!(director.view(&song(0.9)).tier, "high");
        assert_eq!(director.view(&song(0.1)).tier, "quiet");
    }

    #[test]
    fn a_held_shot_is_not_cut_away_from_until_released() {
        let mut director = Director::new(camera(0.3), &each_size());
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
        // A high section cuts every 10 s and a section start cuts at once; none happens.
        let mut all = beats(&mut director, 1, 40, &base, plain);
        all.extend(beats(&mut director, 41, 1, &base, |_, m| start(1.0, m)));
        assert!(all.is_empty(), "{all:?}");
        assert_eq!(director.state().index, 2);
        assert_eq!(director.state().cuts, 2);
        // The shot ages meanwhile.
        assert!(director.state().age > 20.0);
        // Released, it waits a whole cadence (10 s) before it cuts.
        director.release_shot();
        assert!(!director.view(&base).held);
        let cuts = beats(&mut director, 42, 40, &base, plain);
        assert_eq!(cuts.iter().map(|(b, _)| *b).collect::<Vec<_>>(), [64]);
    }

    #[test]
    fn a_lab_pace_overrides_the_cadence_and_its_own_restores_it() {
        let camera = Camera {
            pace: Pace::Calm,
            ..camera(0.0)
        };
        let cuts = |pace: Option<Pace>| {
            let mut director = Director::new(camera, &each_size());
            director.set_pace_override(pace);
            let base = song(0.5);
            beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
            beats(&mut director, 2, 128, &base, plain).len()
        };
        // The scene's own, calm: 2 cuts in 64 s, where a normal one makes 3 and a busy one 5.
        assert_eq!(cuts(None), 2);
        assert_eq!(cuts(Some(Pace::Normal)), 3);
        assert_eq!(cuts(Some(Pace::Busy)), 5);
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
        let director = bursting(camera(0.0));
        let list = director.view(&song(0.5)).shots;
        // The scene's six, then the burst's four, which a lab may hold too.
        assert_eq!(list.len(), 10);
        assert_eq!(
            (list[0].1, list[4].1, list[5].1),
            (ShotSize::Wide, ShotSize::Top, ShotSize::Wide)
        );
        assert_eq!(
            list.iter().map(|shot| shot.2).collect::<Vec<_>>(),
            [0, 0, 0, 0, 0, 0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn the_runtime_file_has_todays_numbers() {
        let runtime = crate::bind::RuntimeBindings::embedded();
        let d = &runtime.director;
        assert_eq!((d.below, d.above), (QUIET_BELOW, HIGH_ABOVE));
        assert_eq!(d.seconds, [QUIET_SECONDS, NORMAL_SECONDS, HIGH_SECONDS]);
        assert_eq!((d.gap, d.lead), (GAP_SECONDS, LEAD_SECONDS));
        assert_eq!(d.grace, HIT_GRACE_SECONDS);
        assert_eq!(d.lift, FILL_LIFT);
        assert_eq!(d.sizes[1], [2.0, 3.0, 2.0, 1.0, 1.0]);
        assert_eq!(d.cadence.source().name, "section_level");
        assert_eq!(d.cadence.fallback_source().map(|s| s.name), None);
        assert_eq!(d.cut.source().name, "bars");
        assert_eq!(d.cut.fallback_source().map(|s| s.name), Some("hits"));
        assert_eq!(d.burst.source().name, "cue_in");
        assert_eq!(d.on.source().name, "beats");
    }

    /// The beat of the first cut in the 100 beats after a section started at beat 1, of a song
    /// whose chart says quiet (0.2) and whose `drive` says high (0.8), by `runtime`.
    fn first_cut_by(runtime: &crate::bind::RuntimeBindings) -> Option<u32> {
        let mut director = Director::new(camera(0.0), &each_size());
        director.set_binds(runtime.director.clone(), runtime.moments.clone());
        let base = MusicValues {
            drive: 0.8,
            ..song(0.2)
        };
        beats(&mut director, 1, 1, &base, |_, m| start(0.0, m));
        beats(&mut director, 2, 100, &base, plain)
            .first()
            .map(|(beat, _)| *beat)
    }

    #[test]
    fn the_cadence_follows_the_source_the_file_names() {
        let embedded = crate::bind::RuntimeBindings::embedded();
        // The section's level, 0.2: a quiet section, a cut after 24 s.
        assert_eq!(first_cut_by(&embedded), Some(52));
        // The drive, 0.8: a high one, after 10 s.
        let when = "when = { below = 0.35, above = 0.71, seconds = [24.0, 16.0, 10.0], gap = 6.0, lead = 4.0 }";
        let custom = crate::bind::tests::embedded_with(
            &format!("source = \"section_level\"\nhow = \"follow\"\n{when}"),
            &format!("source = \"drive\"\nhow = \"follow\"\n{when}"),
        );
        assert_eq!(first_cut_by(&custom), Some(24));
    }

    #[test]
    fn a_cut_lands_on_the_event_the_file_names() {
        // The quiet section's 24 s are due at beat 49: on the beats a cut lands there, on the
        // bars at the next downbeat.
        let embedded = crate::bind::RuntimeBindings::embedded();
        assert_eq!(first_cut_by(&embedded), Some(52));
        let custom = crate::bind::tests::embedded_with(
            "target = \"director.cut\"\nsource = \"bars\"",
            "target = \"director.cut\"\nsource = \"beats\"",
        );
        assert_eq!(first_cut_by(&custom), Some(49));
    }

    #[test]
    fn the_gap_holds_a_cadence_cut_back() {
        // A quiet section whose cadence is 3 s: held back to the gap's 6 s (the downbeat of beat
        // 16), or with no gap cutting at 3 s (beat 8).
        let with_gap = |gap: &str| {
            crate::bind::tests::embedded_with(
                "seconds = [24.0, 16.0, 10.0], gap = 6.0",
                &format!("seconds = [3.0, 3.0, 3.0], gap = {gap}"),
            )
        };
        assert_eq!(first_cut_by(&with_gap("6.0")), Some(16));
        assert_eq!(first_cut_by(&with_gap("0.0")), Some(8));
    }

    #[test]
    fn a_section_start_within_the_gap_does_not_cut() {
        let base = song(0.5);
        let mut director = Director::new(camera(0.0), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        // A section starts 3 s after the cut: the shot goes on.
        assert!(beats(&mut director, 1, 5, &base, plain).is_empty());
        assert!(beats(&mut director, 6, 1, &base, |_, m| start(1.0, m)).is_empty());
        assert_eq!(director.state().cuts, 1);
        // Its progression still starts again: the next cadence cut establishes the section.
        let cuts = beats(&mut director, 7, 40, &base, plain);
        assert_eq!(cuts[0].1.size, ShotSize::Wide, "{cuts:?}");

        // 7 s after a cut it is past the gap: it cuts.
        let mut director = Director::new(camera(0.0), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert!(beats(&mut director, 1, 13, &base, plain).is_empty());
        let cut = beats(&mut director, 14, 1, &base, |_, m| start(1.0, m));
        assert_eq!(cut.len(), 1);

        // A scene's `hold` of 8 s raises the gap for it: 7 s is not enough, 8.5 s is.
        let mut director = Director::new(held(8.0), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        assert!(beats(&mut director, 1, 13, &base, plain).is_empty());
        assert!(beats(&mut director, 14, 1, &base, |_, m| start(1.0, m)).is_empty());
        assert!(beats(&mut director, 15, 1, &base, plain).is_empty());
        assert!(beats(&mut director, 16, 1, &base, plain).is_empty());
        assert_eq!(
            beats(&mut director, 17, 1, &base, |_, m| start(2.0, m)).len(),
            1
        );
    }

    #[test]
    fn a_cadence_cut_waits_for_a_section_start_it_is_less_than_the_lead_before() {
        let base = song(0.5);
        // A normal section's 16 s are due at beat 32, a downbeat. A section starts at beat 36.
        let run = |known: bool| {
            let mut director = Director::new(camera(0.0), &each_size());
            beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
            let mut all = beats(&mut director, 1, 35, &base, |beat, m| MusicValues {
                cue_in: if known {
                    (36 - beat) as f32 * 0.5
                } else {
                    -1.0
                },
                ..m
            });
            all.extend(beats(&mut director, 36, 1, &base, |_, m| start(1.0, m)));
            all.iter().map(|(beat, _)| *beat).collect::<Vec<_>>()
        };
        // Known ahead, 2 s away, less than the lead's 4: the section start cuts instead.
        assert_eq!(run(true), [36]);
        // Unknown: the cadence cuts at 32 and the section start 2 s later is within the gap.
        assert_eq!(run(false), [32]);
    }

    #[test]
    fn a_cadence_cut_waits_for_a_section_start_it_is_more_than_the_lead_but_less_than_the_gap_before()
     {
        let base = song(0.5);
        // The cadence's 16 s are due at beat 32; the section starts 5 s later, at beat 42: past
        // the lead's 4 s but within the gap's 6, so a cut at 32 would hold the section start back.
        let mut director = Director::new(camera(0.0), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let mut all = beats(&mut director, 1, 41, &base, |beat, m| MusicValues {
            cue_in: (42 - beat) as f32 * 0.5,
            ..m
        });
        all.extend(beats(&mut director, 42, 1, &base, |_, m| start(1.0, m)));
        let cut_at: Vec<_> = all.iter().map(|(beat, _)| *beat).collect();
        assert_eq!(cut_at, [42]);
    }

    #[test]
    fn a_high_section_never_cuts_to_a_shot_that_does_not_suit_it() {
        use ShotSize::*;
        let not_high = |size| Shot {
            tiers: vec!["quiet".into(), "normal".into()],
            ..picked(size, None, 1.0)
        };
        // Two far views not for the climax, and the near ones.
        let shots = vec![
            not_high(Wide),
            Shot {
                opens: Some(true),
                ..not_high(Top)
            },
            picked(Medium, Some(true), 1.0),
            picked(Close, None, 1.0),
            picked(Detail, None, 1.0),
        ];
        let mut director = Director::new(camera(0.0), &shots);
        let base = song(0.9);
        let mut all = beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 400, &base, plain));
        assert!(all.len() > 10);
        assert!(all.iter().all(|(_, cut)| cut.index >= 2), "{all:?}");
        // A quiet section may use them.
        let mut director = Director::new(camera(0.0), &shots);
        let quiet = song(0.1);
        let mut all = beats(&mut director, 0, 1, &quiet, |_, m| start(0.0, m));
        all.extend(beats(&mut director, 1, 400, &quiet, plain));
        assert!(all.iter().any(|(_, cut)| cut.index < 2), "{all:?}");
    }

    #[test]
    fn a_shot_with_a_longest_is_cut_away_from_after_it() {
        use ShotSize::*;
        let run = |longest: Option<f32>| {
            let shots = vec![
                Shot {
                    longest,
                    ..picked(Wide, Some(true), 1.0)
                },
                picked(Medium, None, 1.0),
            ];
            let mut director = Director::new(camera(0.0), &shots);
            assert_eq!(director.state().index, 0);
            beats(&mut director, 1, 80, &song(0.5), plain)
                .iter()
                .map(|(beat, _)| *beat)
                .collect::<Vec<_>>()
        };
        // A normal section waits 16 s (32 beats); a wide shot of 8 s is cut away from at 8 s,
        // and the medium one after it, with none, waits its 16 s.
        assert_eq!(run(None)[0], 32);
        let cuts = run(Some(8.0));
        assert_eq!(cuts[0], 16);
        assert_eq!(cuts[1] - cuts[0], 32);
        // Never under the gap.
        assert_eq!(run(Some(1.0))[0], 12);
    }

    #[test]
    fn a_cadence_cut_makes_way_for_the_burst_before_a_lift() {
        let coming = MusicValues {
            next_energy: 0.9,
            ..song(0.5)
        };
        // A lifting section starts at beat 46; the cadence's 16 s are due at beat 32, when the
        // burst's first cut (beat 42) is 5 s away, less than the gap.
        let run = |mut director: Director| {
            let mut all = beats(&mut director, 0, 1, &coming, |_, m| start(0.0, m));
            all.extend(beats(&mut director, 1, 45, &coming, |beat, m| {
                MusicValues {
                    cue_in: (46 - beat) as f32 * 0.5,
                    ..m
                }
            }));
            all.extend(beats(&mut director, 46, 1, &song(0.9), |_, m| {
                start(1.0, m)
            }));
            all.iter()
                .map(|(beat, cut)| (*beat, cut.fill))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            run(bursting(camera(0.0))),
            [(0, 0), (42, 1), (43, 2), (44, 3), (45, 4), (46, 0)]
        );
        // With no burst declared only the lead applies, and the cadence cuts at 32.
        assert_eq!(
            run(Director::new(camera(0.0), &each_size())),
            [(0, 0), (32, 0), (46, 0)]
        );
    }

    #[test]
    fn a_burst_that_would_come_within_the_gap_of_a_cut_is_left_out() {
        // The section starts at beat 0 and a lift comes at beat 8: the burst's first cut would
        // be 2 s after the cut, so none of it is, nor is the section start 4 s after the cut.
        let mut director = bursting(camera(0.0));
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        let coming = MusicValues {
            next_energy: 0.9,
            ..base
        };
        let mut all = beats(&mut director, 1, 7, &coming, |beat, m| MusicValues {
            cue_in: (8 - beat) as f32 * 0.5,
            ..m
        });
        all.extend(beats(&mut director, 8, 1, &song(0.9), |_, m| start(1.0, m)));
        // Only the section start cuts, as a high section's start does within the gap, and no
        // burst cut came before it.
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!((all[0].1.fill, all[0].1.strong), (0, true));
    }

    /// The embedded runtime with `director.burst`'s chance replaced (none: left out of the file).
    fn with_chance(director: &mut Director, chance: Option<f32>) {
        let to = chance.map_or(String::new(), |c| format!(", chance = {c:?}"));
        let runtime = crate::bind::tests::embedded_with(", chance = 0.5", &to);
        director.set_binds(runtime.director.clone(), runtime.moments.clone());
    }

    /// The frame `will_burst` is asked on: in section `section`, the next with label `label`.
    fn coming(section: f32, label: f32) -> MusicValues {
        MusicValues {
            section,
            next_label: label,
            ..song(0.2)
        }
    }

    #[test]
    fn the_chance_of_a_burst_is_0_never_1_always_and_1_where_left_out() {
        for (chance, expected) in [(Some(0.0), false), (Some(1.0), true), (None, true)] {
            let mut director = bursting(camera(0.3));
            with_chance(&mut director, chance);
            for section in 0..40 {
                let music = coming(section as f32, (section % 3) as f32);
                assert_eq!(
                    director.will_burst(&music),
                    expected,
                    "{chance:?} {section}"
                );
            }
        }
        // The file's own chance is half.
        assert_eq!(DirectorBinds::default().burst_chance(), 0.5);
    }

    #[test]
    fn the_roll_is_the_sections_and_not_the_same_for_all() {
        let mut director = bursting(camera(0.3));
        with_chance(&mut director, Some(0.5));
        let rolled = |director: &Director| -> Vec<bool> {
            (0..40)
                .map(|section| director.will_burst(&coming(section as f32, (section % 3) as f32)))
                .collect()
        };
        let first = rolled(&director);
        // Every play of the song rolls the same, whatever the director has been through.
        let mut other = bursting(camera(0.3));
        with_chance(&mut other, Some(0.5));
        beats(&mut other, 0, 30, &song(0.5), plain);
        assert_eq!(rolled(&other), first);
        // Some sections burst and some do not, near the chance of half.
        let bursts = first.iter().filter(|b| **b).count();
        assert!((8..=32).contains(&bursts), "{first:?}");
        // The same label in another place of the song rolls on its own.
        let same_label: Vec<bool> = (0..40)
            .map(|section| director.will_burst(&coming(section as f32, 1.0)))
            .collect();
        assert!(same_label.contains(&true) && same_label.contains(&false));
    }

    #[test]
    fn the_last_lift_of_a_song_bursts_if_none_has_whatever_the_roll() {
        let bursts = |all: &[(u32, Cut)]| all.iter().filter(|(_, c)| c.fill > 0).count();
        // A chance of 0, and the chart says a later boundary lifts too: nothing is forced early,
        // and neither is it where the chart is not known.
        for later in [Some(0.5), None] {
            let mut director = bursting(camera(0.3));
            with_chance(&mut director, Some(0.0));
            director.set_song_ahead(later);
            let all = lift_run(&mut director);
            assert_eq!(bursts(&all), 0, "{later:?} {all:?}");
        }
        // No later boundary lifts by `lift` (0.1): this is the last, and it bursts.
        for later in [0.0, 0.05] {
            let mut director = bursting(camera(0.3));
            with_chance(&mut director, Some(0.0));
            director.set_song_ahead(Some(later));
            let all = lift_run(&mut director);
            assert_eq!(bursts(&all), 4, "{later} {all:?}");
            assert!(director.burst_taken);
        }
        // Not if a burst has begun in the song already, and not under the lab's never.
        let music = coming(1.0, 2.0);
        let mut director = bursting(camera(0.3));
        with_chance(&mut director, Some(0.0));
        director.set_song_ahead(Some(0.0));
        assert!(director.will_burst(&music));
        director.burst_taken = true;
        assert!(!director.will_burst(&music));
        director.burst_taken = false;
        director.set_burst_override(Some(false));
        assert!(!director.will_burst(&music));
    }

    #[test]
    fn a_song_that_starts_over_is_promised_its_burst_again() {
        let mut director = bursting(camera(0.3));
        director.burst_taken = true;
        let frame = MusicValues {
            song_time: 100.0,
            ..song(0.5)
        };
        director.tick(0.016, &frame);
        assert!(director.burst_taken);
        // The song time going back is a new song or a seek.
        director.tick(
            0.016,
            &MusicValues {
                song_time: 0.0,
                ..frame
            },
        );
        assert!(!director.burst_taken);
    }

    #[test]
    fn a_burst_that_is_not_taken_holds_no_cadence_cut_back() {
        // 7 s to a lifting section: the burst's 2 s would start within the 6 s gap of a cadence
        // cut, so a burst that will be taken makes the cadence wait; one that will not, no.
        let music = MusicValues {
            cue_in: 7.0,
            next_energy: 0.9,
            ..coming(0.0, 1.0)
        };
        let mut director = bursting(camera(0.3));
        with_chance(&mut director, Some(1.0));
        assert!(director.waits_for_section(&music, true));
        with_chance(&mut director, Some(0.0));
        assert!(!director.waits_for_section(&music, true));
        // The lab can decide either way too.
        director.set_burst_override(Some(true));
        assert!(director.waits_for_section(&music, true));
    }

    #[test]
    fn a_high_section_start_within_the_gap_cuts_on_a_strong_transition_a_normal_one_does_not() {
        let at = |energy: f32| {
            let base = song(energy);
            let mut director = Director::new(camera(0.0), &each_size());
            beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
            // 3 s after the cut.
            assert!(beats(&mut director, 1, 5, &base, plain).is_empty());
            beats(&mut director, 6, 1, &base, |_, m| start(1.0, m))
        };
        let high = at(0.9);
        assert_eq!(high.len(), 1, "{high:?}");
        assert!(high[0].1.strong && high[0].1.fill == 0);
        assert!(at(0.5).is_empty());
        assert!(at(0.2).is_empty());
        // Past the gap a normal start cuts, but not on a strong transition by this rule.
        let base = song(0.5);
        let mut director = Director::new(camera(0.0), &each_size());
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        beats(&mut director, 1, 13, &base, plain);
        let cut = beats(&mut director, 14, 1, &base, |_, m| start(1.0, m));
        assert!(!cut[0].1.strong);
    }

    #[test]
    fn the_labs_choice_wins_over_the_chance_and_a_burst_by_hand_always_bursts() {
        let bursts = |all: &[(u32, Cut)]| all.iter().filter(|(_, c)| c.fill > 0).count();
        // Never, though the chance is 1 and no burst has been taken.
        let mut director = bursting(camera(0.3));
        with_chance(&mut director, Some(1.0));
        director.set_burst_override(Some(false));
        let all = lift_run(&mut director);
        assert_eq!(bursts(&all), 0, "{all:?}");
        // The climax is still entered on a cut, on a strong transition.
        assert!(all.last().unwrap().1.strong);
        // Always, though the chance is 0 and the song's burst is spent.
        let mut director = bursting(camera(0.3));
        with_chance(&mut director, Some(0.0));
        director.set_burst_override(Some(true));
        assert_eq!(bursts(&lift_run(&mut director)), 4);
        // The lab's own burst (B) bursts under never.
        let mut director = bursting(camera(0.3));
        director.set_burst_override(Some(false));
        let base = song(0.2);
        beats(&mut director, 0, 1, &base, |_, m| start(0.0, m));
        director.start_burst();
        let all = beats(&mut director, 1, 6, &base, plain);
        assert_eq!(bursts(&all), 4, "{all:?}");
    }

    #[test]
    fn the_burst_reads_the_lift_the_file_asks_for() {
        let run = |runtime: &crate::bind::RuntimeBindings| {
            let mut director = bursting(camera(0.3));
            director.set_binds(runtime.director.clone(), runtime.moments.clone());
            lift_run(&mut director).len()
        };
        let embedded = crate::bind::RuntimeBindings::embedded();
        // The lift of the song is 0.7, and a burst of 4 cuts comes before it.
        assert!(run(&embedded) >= 5);
        let custom = crate::bind::tests::embedded_with("lift = 0.1,", "lift = 0.9,");
        assert_eq!(run(&custom), 2);
    }

    /// The sections of a song from `(seconds, level)` each, one after the other from 0.
    fn sections_of(spec: &[(f32, f32)]) -> Arc<[SongSection]> {
        let mut at = 0.0;
        spec.iter()
            .map(|&(length, level)| {
                let section = SongSection {
                    start: at,
                    end: at + length,
                    level,
                };
                at += length;
                section
            })
            .collect()
    }

    /// Two wide shots (the opening may pick either), a medium and a close one that says
    /// `at_least_once`, suits `tiers` and is never picked by chance (weight 0).
    fn promised_shots(tiers: &[&str]) -> Vec<Shot> {
        use ShotSize::*;
        let mut shots = [Wide, Wide, Medium, Close]
            .map(|size| shot(size, ShotMove::Cut))
            .to_vec();
        shots[3].weight = 0.0;
        shots[3].at_least_once = true;
        shots[3].tiers = tiers.iter().map(|t| t.to_string()).collect();
        shots
    }

    /// Plays a whole song through, a frame per beat at 120 bpm (the beat followed, a section's
    /// first frame its start, the seconds left in it known). Returns each cut with the section
    /// it was in and the seconds into that section.
    fn play(director: &mut Director, chart: &Arc<[SongSection]>) -> Vec<(usize, f32, Cut)> {
        play_with(director, chart, true)
    }

    /// [`play`], with the song's sections told to the director or not.
    fn play_with(
        director: &mut Director,
        chart: &Arc<[SongSection]>,
        tell: bool,
    ) -> Vec<(usize, f32, Cut)> {
        director.set_song(tell.then_some(chart));
        let mut cuts = Vec::new();
        let mut beat = 0u32;
        for (i, section) in chart.iter().enumerate() {
            let beats = ((section.end - section.start) / 0.5) as u32;
            for b in 0..beats {
                let into = b as f32 * 0.5;
                let frame = MusicValues {
                    section: i as f32,
                    section_energy: section.level,
                    next_energy: section.level,
                    cue_in: section.end - section.start - into,
                    song_time: section.start + into,
                    beats: 1,
                    bars: u32::from(beat.is_multiple_of(4)),
                    ..song(section.level)
                };
                let frame = if b == 0 {
                    start(i as f32, frame)
                } else {
                    frame
                };
                beat += 1;
                if let Some(cut) = director.tick(0.5, &frame) {
                    cuts.push((i, into, cut));
                }
            }
        }
        cuts
    }

    /// The cuts to the promised shot (index 3 of [`promised_shots`]).
    fn to_promised(cuts: &[(usize, f32, Cut)]) -> Vec<(usize, f32)> {
        cuts.iter()
            .filter(|(_, _, cut)| cut.index == 3)
            .map(|&(section, into, _)| (section, into))
            .collect()
    }

    #[test]
    fn a_shot_that_chance_never_picks_is_cut_to_in_the_last_section_that_suits_it() {
        // Quiet, high, normal, high, normal: the last high section is the fourth.
        let chart = sections_of(&[
            (40.0, 0.2),
            (40.0, 0.9),
            (40.0, 0.5),
            (40.0, 0.9),
            (40.0, 0.5),
        ]);
        let mut director = Director::new(camera(0.0), &promised_shots(&["high"]));
        let cuts = play(&mut director, &chart);
        let promised = to_promised(&cuts);
        // Once, in the fourth section, and after its opening: its first cut is the opening.
        assert_eq!(promised.len(), 1, "{cuts:?}");
        assert_eq!(promised[0].0, 3);
        assert!(promised[0].1 >= 6.0, "{promised:?}");
        let first = cuts.iter().find(|(section, ..)| *section == 3).unwrap();
        assert_ne!(first.2.index, 3, "{cuts:?}");
        // Nothing is owed afterwards, in the lab's readout either.
        assert!(!director.is_owed(3, &MusicValues::default()));
    }

    #[test]
    fn a_shot_already_on_is_not_cut_to_again_for_the_promise() {
        // Two shots, a wide and a close one that says it: a high section's cuts alternate
        // between them, so the shot is on long before the last section. The song plays exactly
        // as it does with no promise.
        let shots = |promise: bool| {
            let mut shots = vec![
                shot(ShotSize::Wide, ShotMove::Cut),
                shot(ShotSize::Close, ShotMove::Cut),
            ];
            shots[1].at_least_once = promise;
            shots[1].tiers = vec!["high".to_string()];
            shots
        };
        let chart = sections_of(&[(40.0, 0.9), (40.0, 0.5), (40.0, 0.9)]);
        let with = play(&mut Director::new(camera(0.0), &shots(true)), &chart);
        let without = play(&mut Director::new(camera(0.0), &shots(false)), &chart);
        assert_eq!(with, without);
        assert!(with.iter().any(|(_, _, cut)| cut.index == 1));
    }

    #[test]
    fn a_song_with_no_section_of_its_tier_still_shows_the_shot_in_its_highest() {
        // The shot suits only quiet sections, and this song has none. The highest is the third,
        // and the last of two equals wins.
        let chart = sections_of(&[
            (40.0, 0.5),
            (40.0, 0.6),
            (40.0, 0.65),
            (40.0, 0.65),
            (40.0, 0.55),
        ]);
        let mut director = Director::new(camera(0.0), &promised_shots(&["quiet"]));
        let cuts = play(&mut director, &chart);
        let promised = to_promised(&cuts);
        assert_eq!(promised.len(), 1, "{cuts:?}");
        assert_eq!(promised[0].0, 3);
    }

    #[test]
    fn a_section_too_short_for_a_cut_after_the_gap_is_not_the_last_chance() {
        let mut director = Director::new(camera(0.0), &promised_shots(&["high"]));
        // The last high section lasts 10 s: the gap, the room before the next section and the
        // cut's grace need 14. The high section before it is the last chance.
        let chart = sections_of(&[(40.0, 0.9), (40.0, 0.5), (10.0, 0.9), (40.0, 0.5)]);
        director.set_song(Some(&chart));
        assert_eq!(director.promised[3], Some(0));
        // None suits and the highest is short: the next highest that fits.
        let mut director = Director::new(camera(0.0), &promised_shots(&["quiet"]));
        let chart = sections_of(&[(40.0, 0.5), (40.0, 0.6), (10.0, 0.65), (40.0, 0.55)]);
        director.set_song(Some(&chart));
        assert_eq!(director.promised[3], Some(1));
        // A song with no section that long promises nothing.
        let chart = sections_of(&[(10.0, 0.5), (12.0, 0.9)]);
        director.set_song(Some(&chart));
        assert_eq!(director.promised[3], None);
        // Without a chart nothing is promised either.
        director.set_song(None);
        assert_eq!(director.promised[3], None);
    }

    #[test]
    fn the_cut_comes_after_the_gap_if_the_cadence_would_not_cut_again_in_the_section() {
        // A calm pace cuts a high section every 15 s, and the last high section lasts 20: the
        // cadence's cut would come within the room kept before the next section start.
        let chart = sections_of(&[(40.0, 0.5), (20.0, 0.9), (40.0, 0.5)]);
        let mut calm = camera(0.0);
        calm.pace = Pace::Calm;
        let mut director = Director::new(calm, &promised_shots(&["high"]));
        let cuts = play(&mut director, &chart);
        let promised = to_promised(&cuts);
        assert_eq!(promised.len(), 1, "{cuts:?}");
        assert_eq!(promised[0].0, 1);
        assert!(
            (6.0..=8.5).contains(&promised[0].1),
            "the gap from the opening: {promised:?}"
        );
        // A scene that cuts only at section starts has the same promise.
        let mut sections = camera(0.0);
        sections.pace = Pace::Sections;
        let mut director = Director::new(sections, &promised_shots(&["high"]));
        let promised = to_promised(&play(&mut director, &chart));
        assert_eq!(promised.len(), 1);
        assert_eq!(promised[0].0, 1);
    }

    #[test]
    fn a_shot_without_the_flag_plays_as_before_and_a_song_not_known_promises_nothing() {
        let chart = sections_of(&[(40.0, 0.2), (40.0, 0.9), (40.0, 0.5)]);
        let mut plain = promised_shots(&["high"]);
        plain[3].at_least_once = false;
        plain[3].weight = 1.0;
        let known = play(&mut Director::new(camera(0.0), &plain), &chart);
        let mut director = Director::new(camera(0.0), &plain);
        let unknown = play_with(&mut director, &chart, false);
        assert_eq!(known, unknown);
        // And a flagged shot in a song whose chart is not known is never owed.
        let mut director = Director::new(camera(0.0), &promised_shots(&["high"]));
        assert!(director.promised.iter().all(Option::is_none));
        assert!(!director.is_owed(3, &song(0.9)));
        director.set_song(None);
        assert!(director.owed(&song(0.9)).is_none());
    }

    #[test]
    fn the_song_going_back_owes_the_shot_again() {
        let chart = sections_of(&[(40.0, 0.5), (40.0, 0.9), (40.0, 0.5)]);
        let mut director = Director::new(camera(0.0), &promised_shots(&["high"]));
        play(&mut director, &chart);
        assert!(director.shown[3]);
        let at = |section: f32, time: f32| MusicValues {
            section,
            song_time: time,
            ..song(0.5)
        };
        assert!(!director.is_owed(3, &at(2.0, 100.0)));
        // A seek back to the first section: a new account, the shot owed with its chance ahead.
        director.tick(0.016, &at(0.0, 3.0));
        assert!(!director.shown[3]);
        assert!(director.is_owed(3, &at(0.0, 3.0)));
        // Past its last chance it is no longer owed, and the director does not cut to it late.
        assert!(!director.is_owed(3, &at(2.0, 85.0)));
        assert!(director.owed(&at(2.0, 85.0)).is_none());
        // Another song forgets what was on too.
        play(&mut director, &chart);
        assert!(director.shown[3]);
        director.set_song(Some(&sections_of(&[(40.0, 0.5), (40.0, 0.9)])));
        assert!(!director.shown[3]);
    }

    /// Play `chart` (an analysis as JSON) from its start to `until` seconds at 60 frames a
    /// second through the runtime's own path (the chart's state, the trusted rhythm, the music's
    /// raw values and signals), as the app does, and return each cut with its song time.
    fn play_chart(director: &mut Director, chart: &str, until: f64) -> Vec<(f64, Cut)> {
        use crate::scene::inputs::{EventCounter, Levels, music};
        let chart = crate::chart::Chart::from_analysis(serde_json::from_str(chart).unwrap());
        let dt = 1.0 / 60.0;
        let mut events = EventCounter::with_wall_dt(dt);
        let mut cuts = Vec::new();
        for frame in 0..(until / dt) as u32 {
            let t = f64::from(frame) * dt;
            let state = chart.at(t);
            let rhythm = crate::chart::trusted_rhythm(Default::default(), Some(&state));
            let m = music(
                &rhythm,
                Some(&state),
                Levels::default(),
                (t, true),
                &mut events,
            );
            director.set_song_ahead(Some(state.later_rise as f32));
            director.set_song(state.song.as_ref());
            if let Some(cut) = director.tick(dt as f32, &m) {
                cuts.push((t, cut));
            }
        }
        cuts
    }

    /// A chart at 120 bpm in 4/4 (a beat every 0.5 s from 0, a bar every 2 s) with these
    /// sections (start, end, energy), labelled from A, heard throughout.
    fn chart_json(sections: &[(f64, f64, f64)]) -> String {
        let end = sections.last().map_or(0.0, |s| s.1);
        let beats: Vec<String> = (0..(end * 2.0) as u32)
            .map(|i| {
                format!(
                    r#"{{"t": {}, "bar": {}, "pos": {}, "strength": 1, "confidence": 0.9}}"#,
                    f64::from(i) * 0.5,
                    i / 4,
                    i % 4
                )
            })
            .collect();
        let sections: Vec<String> = sections
            .iter()
            .enumerate()
            .map(|(i, (start, end, energy))| {
                format!(
                    r#"{{"start": {start}, "end": {end}, "label": "{}", "energy": {energy}, "repeat": 0, "confidence": 1}}"#,
                    char::from(b'A' + i as u8)
                )
            })
            .collect();
        format!(
            r#"{{"version": 1, "title": "t", "audio": "/a.wav", "duration": {end},
            "analysis_seconds": 1, "analyzer": "test",
            "tempo": {{"bpm": 120, "confidence": 0.9, "feel": 5.0}},
            "beats": [{}],
            "meter": {{"beats_per_bar": 4, "confidence": 0.9}},
            "sections": [{}],
            "streams": {{"low": {{"name": "x", "events": []}}, "mid": {{"name": "x", "events": []}},
                "high": {{"name": "x", "events": []}}, "harmonic": {{"name": "x", "events": []}},
                "phrase": {{"name": "x", "events": []}}}},
            "lead": [],
            "curves": {{"hop": 0.5, "loudness": [0.5], "brightness": [0], "percussive": [0],
                "beat_conf": [0.9], "feel": [5]}}}}"#,
            beats.join(","),
            sections.join(",")
        )
    }

    #[test]
    fn a_burst_leads_to_its_cut_when_an_eighth_bar_cue_comes_just_before_the_lift() {
        // A quiet first section of nine bars, then a high one. With the beat followed, a cue of
        // 0.3 comes every eight bars since the last cue, chart or not: here at 16 s, two seconds
        // before the section starts at 18 s, where the burst runs (beats at 16 to 17.5 s).
        let chart = chart_json(&[(0.0, 18.0, 0.2), (18.0, 40.0, 0.9)]);
        let mut director = bursting(camera(0.3));
        let cuts = play_chart(&mut director, &chart, 30.0);
        let burst: Vec<(f64, u32)> = cuts
            .iter()
            .filter(|(_, c)| c.fill > 0)
            .map(|(t, c)| ((t * 2.0).round() / 2.0, c.fill))
            .collect();
        assert_eq!(
            burst,
            [(16.0, 1), (16.5, 2), (17.0, 3), (17.5, 4)],
            "{cuts:?}"
        );
        // The cut after the burst's last is the section start it leads to, a strong cut to one
        // of the scene's shots, not a cadence cut seconds later.
        let last = cuts.iter().position(|(_, c)| c.fill == 4).unwrap();
        let (at, after) = cuts.get(last + 1).expect("a cut after the burst");
        assert!((18.0..18.05).contains(at), "{cuts:?}");
        assert_eq!((after.fill, after.strong), (0, true));
        assert!((after.index as usize) < director.ordinary);
    }
}
