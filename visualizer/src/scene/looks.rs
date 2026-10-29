//! The looks of a scene: decides when its look changes and to which, following the music, and
//! tells the scene through `struct Look` (see `docs/music.md`, "Looks"). A scene declares its
//! looks (large visual changes: the light, the palette) in `scene.toml` and draws them; the
//! runtime decides the rest by `look.change` of `runtime.toml`. Like the director it is per scene
//! and a pure state machine of the music's values ([`MusicValues`]), the time step and the frame's
//! seed, so it can be tested without a scene or a GPU.

use std::sync::Arc;

use super::director::{label_hash, weighted};
use super::inputs::MusicValues;
use super::manifest::{Look, Manifest};
use crate::bind::{DirectorBinds, LookBinds, LookCue};
use crate::chart::{SongSection, last_chance};

/// The `k` the label's hash is taken with for the look. The director takes the small numbers (and
/// `k + 7`) for its shots, so a section's look is not tied to its shot.
const PICK_K: u32 = 100;

/// What the scene is told about its look: `struct Look`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LookState {
    /// The look before this one, as the order of the scene's `[[look]]`s from 0. (`from` is a
    /// reserved word of WGSL, which the scene reads this as.)
    pub prev: u32,
    /// The look now.
    pub to: u32,
    /// 0 to 1 through the fade from `prev` to `to`, already eased.
    pub mix: f32,
    /// How many changes so far: a new one starts a fade.
    pub changes: u32,
}

impl LookState {
    /// The layout of `struct Look`: four words (16 bytes).
    pub fn bytes(&self) -> [u8; 16] {
        let words = [
            self.prev.to_le_bytes(),
            self.to.to_le_bytes(),
            self.mix.to_le_bytes(),
            self.changes.to_le_bytes(),
        ];
        let mut bytes = [0; 16];
        for (i, word) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(word);
        }
        bytes
    }
}

/// What a readout can show of the looks (see [`Looks::view`]).
#[derive(Clone, Debug, PartialEq)]
pub struct LooksView {
    pub state: LookState,
    /// The `name` of each look in order (empty if it has none).
    pub names: Vec<String>,
    /// Whether the look is held (see [`Looks::hold`]), so that the runtime does not change it.
    pub held: bool,
}

/// A number from 0 up to but not including 1, made from a seed.
fn unit(seed: u32) -> f32 {
    (seed >> 8) as f32 / (1u32 << 24) as f32
}

/// The order the scene steps through its looks in without section labels: a shuffle of them made
/// from the seed alone, so that the same seed steps the same way.
fn order_from(seed: u32, count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    let mut x = seed | 1;
    for i in (1..count).rev() {
        // xorshift32.
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        order.swap(i, x as usize % (i + 1));
    }
    order
}

/// The ease of the fade: slow at both ends.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Decides the look changes of one scene from the music.
pub struct Looks {
    /// The `name` of each `[[look]]`, in order.
    names: Vec<String>,
    /// What each look says of when it is picked: whether the scene may open in it, and its weight.
    picks: Vec<(bool, f32)>,
    state: LookState,
    /// Whether the first look has been picked yet: it is, with the first tick, by that frame's
    /// seed.
    started: bool,
    /// The order of the looks without section labels, made from the seed at the start.
    order: Vec<usize>,
    /// Seconds since the last change, or since the start.
    since_change: f32,
    /// How long the fade now under way lasts, and how far it is, in seconds.
    fade_length: f32,
    fade_age: f32,
    /// The look is held: no change happens until [`Looks::release`] (for a scene lab).
    held: bool,
    /// The look the scene opens in if it is held before its first frame.
    held_first: Option<usize>,
    binds: Arc<LookBinds>,
    /// The section's tier (quiet, normal, high), as the director has it: what a look `suits`.
    director: Arc<DirectorBinds>,
    /// Whether each look suits a quiet, a normal and a high section.
    suits: Vec<[bool; 3]>,
    /// Whether each look says `at_least_once`.
    promises: Vec<bool>,
    /// The sections of the song with their levels, shared with the director (see
    /// [`Self::set_song`]); none without a chart, where nothing is promised.
    song: Option<Arc<[SongSection]>>,
    /// For each look: the section of the song whose start is its last chance, if it says
    /// `at_least_once` and the song has a section it can be changed to at.
    promised: Vec<Option<usize>>,
    /// For each look: whether it has been on in this song. Starts again when the song time goes
    /// back, the look then on counting.
    shown: Vec<bool>,
    /// The song time of the last frame, which going back tells a new song (or a seek) by.
    last_song_time: f32,
}

impl Looks {
    /// The looks of a scene that declares some; none for a scene without (it works as it did
    /// before there were looks).
    pub fn for_manifest(manifest: &Manifest) -> Option<Self> {
        (!manifest.looks.is_empty()).then(|| Self::new(&manifest.looks))
    }

    /// The looks of `looks`, which must not be empty.
    pub fn new(looks: &[Look]) -> Self {
        assert!(!looks.is_empty(), "a scene with looks needs a look");
        Self {
            names: looks.iter().map(|look| look.name.clone()).collect(),
            picks: looks.iter().map(|look| (look.opens, look.weight)).collect(),
            state: LookState {
                mix: 1.0,
                ..LookState::default()
            },
            started: false,
            order: Vec::new(),
            since_change: 0.0,
            fade_length: 0.0,
            fade_age: 0.0,
            held: false,
            held_first: None,
            binds: Arc::default(),
            director: Arc::default(),
            suits: looks
                .iter()
                .map(|look| [look.suits(0), look.suits(1), look.suits(2)])
                .collect(),
            promises: looks.iter().map(|look| look.at_least_once).collect(),
            song: None,
            promised: vec![None; looks.len()],
            shown: vec![false; looks.len()],
            last_song_time: 0.0,
        }
    }

    /// Decide by these bindings from now on (a scene lab replaced `runtime.toml`): the looks'
    /// own, and the director's, whose tiers say which looks suit a section.
    pub fn set_binds(&mut self, binds: Arc<LookBinds>, director: Arc<DirectorBinds>) {
        self.binds = binds;
        self.director = director;
        self.plan_promises();
    }

    /// Tell the looks the sections of the song with their levels (`ChartState::song`, the list
    /// the director has), or none without a chart. A look that says `at_least_once` is promised a
    /// change at the start of the last section that suits it (see [`Self::promise`]). Only a
    /// different list makes the looks plan again, and one that replaces another forgets what was
    /// on.
    pub fn set_song(&mut self, song: Option<&Arc<[SongSection]>>) {
        match (&self.song, song) {
            (Some(on), Some(new)) if Arc::ptr_eq(on, new) => return,
            (None, None) => return,
            (Some(_), Some(_)) => self.forget_shown(),
            _ => {}
        }
        self.song = song.cloned();
        self.plan_promises();
    }

    /// Start the song's account again: only the look on counts as shown.
    fn forget_shown(&mut self) {
        self.shown.fill(false);
        if self.started {
            self.shown[self.state.to as usize] = true;
        }
    }

    /// Decide again at which section start each look that says `at_least_once` is promised.
    fn plan_promises(&mut self) {
        self.promised = (0..self.picks.len())
            .map(|i| self.promises[i].then(|| self.promise(i)).flatten())
            .collect();
    }

    /// The section whose start is the last chance of look `index`: the last that suits it by its
    /// `tiers` (the level against `director.cadence`'s thresholds), or where none does the song's
    /// highest (the last of equals), as for a shot. Only a section start where the look's gap
    /// allows a change counts: the first section, or one that starts at least `look.change`'s
    /// gap after the one before it (a look changes only at section starts, so the change before
    /// is then far enough). None where no section does, or the song is not known.
    fn promise(&self, index: usize) -> Option<usize> {
        let song = self.song.as_ref()?;
        let gap = self.binds.gap(LookCue::Section);
        let fits = |i: usize| i == 0 || song[i].start - song[i - 1].start >= gap;
        let suits =
            |section: &SongSection| self.suits[index][self.director.tier_at(section.level).min(2)];
        last_chance(song, suits, fits)
    }

    /// The look the song still owes at the start of the section playing, if any: one that says
    /// `at_least_once`, has not been on in this song and has this section as its last chance.
    fn owed(&self, music: &MusicValues) -> Option<usize> {
        let section = music.section.max(0.0) as usize;
        (0..self.picks.len()).find(|&i| self.promised[i] == Some(section) && !self.shown[i])
    }

    /// Whether the song still owes look `index` a change, at this section start or a later one.
    #[cfg(test)]
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

    /// The looks that suit a section of `tier`, or all of them if none does.
    fn suiting(&self, tier: usize) -> Vec<usize> {
        let fit: Vec<usize> = (0..self.picks.len())
            .filter(|&i| self.suits[i][tier.min(2)])
            .collect();
        if fit.is_empty() {
            (0..self.picks.len()).collect()
        } else {
            fit
        }
    }

    /// What the scene is told now.
    pub fn state(&self) -> &LookState {
        &self.state
    }

    /// What a readout shows: the state, the names and whether the look is held.
    pub fn view(&self) -> LooksView {
        LooksView {
            state: self.state,
            names: self.names.clone(),
            held: self.held,
        }
    }

    /// Take over what is under way in `old`, the looks of the scene this one replaces when its
    /// files were saved: the look on, the fade and a hold stay, if the scene still has as many
    /// looks.
    pub fn inherit(&mut self, old: &Looks) {
        if old.names.len() != self.names.len() {
            return;
        }
        self.state = old.state;
        self.started = old.started;
        self.order = old.order.clone();
        self.since_change = old.since_change;
        self.fade_length = old.fade_length;
        self.fade_age = old.fade_age;
        self.held = old.held;
        self.held_first = old.held_first;
        self.shown = old.shown.clone();
        self.last_song_time = old.last_song_time;
    }

    /// The first look, picked among the looks that open by `seed`, each by its weight.
    fn open(&self, seed: u32) -> usize {
        let chances: Vec<(usize, f32)> = self
            .picks
            .iter()
            .enumerate()
            .filter(|(_, (opens, _))| *opens)
            .map(|(i, (_, weight))| (i, *weight))
            .collect();
        // The manifest refuses a scene where none opens; a look is always found.
        let chances = if chances.is_empty() {
            vec![(0, 1.0)]
        } else {
            chances
        };
        weighted(&chances, unit(seed))
    }

    /// The look a section of `tier` picks among those that suit it: by the weights, with the
    /// section's label hashed to the roll, so that a section of the same kind picks the same.
    fn pick(&self, label: f32, tier: usize) -> usize {
        let chances: Vec<(usize, f32)> = self
            .suiting(tier)
            .into_iter()
            .map(|i| (i, self.picks[i].1))
            .collect();
        weighted(&chances, label_hash(label, PICK_K))
    }

    /// The look after the one on in the order made from the seed, of those that suit `tier`.
    fn next(&self, tier: usize) -> usize {
        let fit = self.suiting(tier);
        let at = self
            .order
            .iter()
            .position(|&i| i == self.state.to as usize)
            .unwrap_or(0);
        (1..=self.order.len())
            .map(|k| self.order[(at + k) % self.order.len()])
            .find(|i| fit.contains(i))
            .unwrap_or(self.order[(at + 1) % self.order.len()])
    }

    /// Start a fade to look `to` of `seconds`.
    fn change(&mut self, to: usize, seconds: f32) {
        self.state.prev = self.state.to;
        self.state.to = to as u32;
        self.shown[to] = true;
        self.state.changes = self.state.changes.wrapping_add(1);
        self.state.mix = if seconds > 0.0 { 0.0 } else { 1.0 };
        self.fade_length = seconds;
        self.fade_age = 0.0;
        self.since_change = 0.0;
    }

    /// Change to look `index` now, with the usual fade, and hold it: the runtime changes the look
    /// no more until [`Self::release`] (for a scene lab). Before the scene's first frame the
    /// scene opens in it. Whether the scene has the look.
    pub fn hold(&mut self, index: u32, music: &MusicValues) -> bool {
        let index = index as usize;
        if index >= self.names.len() {
            return false;
        }
        self.held = true;
        if !self.started {
            self.held_first = Some(index);
        } else if index != self.state.to as usize {
            self.change(index, self.binds.fade_length(music));
        }
        true
    }

    /// Let go of a held look: the runtime changes it by its rules again, counted from now so that
    /// it does not change at once.
    pub fn release(&mut self) {
        if self.held {
            self.held = false;
            self.held_first = None;
            self.since_change = 0.0;
        }
    }

    /// Advance by one frame of `dt` seconds with the music and the seed of the frame; returns
    /// whether the look changed in it. The first tick picks the scene's first look from the seed.
    pub fn tick(&mut self, dt: f32, music: &MusicValues, seed: u32) -> bool {
        if !self.started {
            self.started = true;
            self.order = order_from(seed, self.names.len());
            let first = self.held_first.unwrap_or_else(|| self.open(seed));
            self.state.prev = first as u32;
            self.state.to = first as u32;
            self.shown[first] = true;
            return false;
        }
        if self.state.mix < 1.0 {
            self.fade_age += dt;
            self.state.mix = if self.fade_length > 0.0 {
                ease(self.fade_age / self.fade_length)
            } else {
                1.0
            };
        }
        self.since_change += dt;
        // Time going back is a new song (or a seek): its promise is still to keep.
        if music.song_time < self.last_song_time - 1.0 {
            self.forget_shown();
        }
        self.last_song_time = music.song_time;
        if self.held {
            return false;
        }
        let Some(cue) = self.binds.cue(music) else {
            return false;
        };
        if self.since_change < self.binds.gap(cue) {
            return false;
        }
        let tier = self.director.tier(music);
        let to = match cue {
            // The look the song owes goes first, at the section start that is its last chance.
            LookCue::Section => self
                .owed(music)
                .unwrap_or_else(|| self.pick(music.section_label, tier)),
            LookCue::Unlabelled => self.next(tier),
        };
        // The look stays tied to the kind of section: the one on is not changed to again.
        if to == self.state.to as usize {
            return false;
        }
        self.change(to, self.binds.fade_length(music));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(name: &str, opens: bool, weight: f32) -> Look {
        Look {
            name: name.to_string(),
            tags: Vec::new(),
            weight,
            opens,
            tiers: Vec::new(),
            at_least_once: false,
        }
    }

    /// A grey look for quiet and normal sections and one in the theme colour for high ones pick
    /// by the section's energy: every quiet or normal section the grey, every high one the colour.
    #[test]
    fn a_section_picks_among_the_looks_that_suit_its_energy() {
        let grey = Look {
            tiers: vec!["quiet".into(), "normal".into()],
            ..look("grey", true, 1.0)
        };
        let colour = Look {
            tiers: vec!["high".into()],
            ..look("colour", false, 1.0)
        };
        let looks = Looks::new(&[grey, colour]);
        for label in 0..20 {
            assert_eq!(looks.pick(label as f32, 0), 0);
            assert_eq!(looks.pick(label as f32, 1), 0);
            assert_eq!(looks.pick(label as f32, 2), 1);
        }
        // None suits: all may be picked.
        let only_high = Looks::new(&[
            Look {
                tiers: vec!["high".into()],
                ..look("a", true, 1.0)
            },
            Look {
                tiers: vec!["high".into()],
                ..look("b", true, 1.0)
            },
        ]);
        assert_eq!(only_high.suiting(0), [0, 1]);
    }

    /// Four looks, the second of which may not open.
    fn four() -> Looks {
        Looks::new(&[
            look("a", true, 1.0),
            look("b", false, 1.0),
            look("c", true, 1.0),
            look("d", true, 1.0),
        ])
    }

    /// The music of a charted frame where a section with `label` starts (a cue of `strength`).
    fn section(label: f32, strength: f32) -> MusicValues {
        MusicValues {
            has_chart: 1,
            cues: 1,
            cue: strength,
            section_label: label,
            ..MusicValues::default()
        }
    }

    /// The music of a frame with no chart where a cue of `strength` comes.
    fn unlabelled(strength: f32) -> MusicValues {
        MusicValues {
            cues: 1,
            cue: strength,
            ..MusicValues::default()
        }
    }

    /// Looks that have started, and where this much time has passed.
    fn started(looks: &mut Looks, seconds: f32) {
        looks.tick(0.0, &MusicValues::default(), 7);
        looks.tick(seconds, &MusicValues::default(), 7);
    }

    #[test]
    fn the_defaults_are_the_files() {
        let binds = LookBinds::default();
        assert_eq!((binds.strength, binds.gap), (0.5, 8.0));
        assert_eq!(
            (binds.strength_unlabelled, binds.gap_unlabelled),
            (0.0, 30.0)
        );
        assert_eq!((binds.fade, binds.fade_seconds), (1.0, 0.5));
    }

    #[test]
    fn the_first_look_is_one_that_opens_and_follows_the_seed() {
        let mut seen = [false; 4];
        for seed in 0..200u32 {
            let mut looks = four();
            assert!(!looks.tick(0.0, &MusicValues::default(), seed.wrapping_mul(0x9e37_79b9)));
            let state = *looks.state();
            assert_eq!(
                (state.prev, state.to, state.mix, state.changes),
                (state.to, state.to, 1.0, 0)
            );
            seen[state.to as usize] = true;
            // The same seed opens the same.
            let mut again = four();
            again.tick(0.0, &MusicValues::default(), seed.wrapping_mul(0x9e37_79b9));
            assert_eq!(again.state(), looks.state());
        }
        assert_eq!(
            seen,
            [true, false, true, true],
            "never the look that does not open"
        );
    }

    #[test]
    fn a_section_start_changes_to_the_look_its_label_picks() {
        let after = |label: f32, seed: u32| {
            let mut looks = four();
            looks.tick(0.0, &MusicValues::default(), seed);
            looks.tick(10.0, &MusicValues::default(), seed);
            looks.tick(0.0, &section(label, 1.0), seed);
            looks.state().to
        };
        let mut seen = [false; 4];
        for label in 0..8 {
            let label = label as f32;
            // Whatever the scene opened in, the label's look is on, and the same each time.
            let to = after(label, 7);
            assert_eq!(to, after(label, 99), "label {label}");
            assert_eq!(to, four().pick(label, 1) as u32);
            seen[to as usize] = true;
        }
        assert!(seen.iter().filter(|s| **s).count() >= 3, "{seen:?}");
        // A cue weaker than `strength` is not a section start.
        let mut weak = four();
        started(&mut weak, 10.0);
        let before = *weak.state();
        assert!(!weak.tick(0.0, &section(1.0, 0.4), 7));
        assert_eq!(*weak.state(), before);
    }

    #[test]
    fn the_look_on_is_kept() {
        let mut looks = four();
        started(&mut looks, 10.0);
        // Find a label whose look differs from the one on, change to it, and let the gap pass.
        let label = (0..8)
            .map(|l| l as f32)
            .find(|l| looks.pick(*l, 1) as u32 != looks.state().to)
            .unwrap();
        assert!(looks.tick(0.0, &section(label, 1.0), 7));
        let changes = looks.state().changes;
        looks.tick(20.0, &MusicValues::default(), 7);
        // The same kind of section again: it is the look on, so nothing changes (and the gap is
        // not started over).
        assert!(!looks.tick(0.0, &section(label, 1.0), 7));
        assert_eq!(looks.state().changes, changes);
    }

    #[test]
    fn two_changes_are_a_gap_apart_and_the_scenes_start_counts() {
        let mut looks = four();
        looks.tick(0.0, &MusicValues::default(), 7);
        let other = |looks: &Looks| {
            (0..8)
                .map(|l| l as f32)
                .find(|l| looks.pick(*l, 1) as u32 != looks.state().to)
                .unwrap()
        };
        // Within 8 s of the start no section changes the look.
        looks.tick(7.0, &MusicValues::default(), 7);
        let label = other(&looks);
        assert!(!looks.tick(0.0, &section(label, 1.0), 7));
        looks.tick(1.0, &MusicValues::default(), 7);
        assert!(looks.tick(0.0, &section(label, 1.0), 7));
        // And 8 s after a change, not before.
        looks.tick(7.9, &MusicValues::default(), 7);
        let label = other(&looks);
        assert!(!looks.tick(0.0, &section(label, 1.0), 7));
        looks.tick(0.2, &MusicValues::default(), 7);
        assert!(looks.tick(0.0, &section(label, 1.0), 7));
    }

    #[test]
    fn without_labels_the_looks_step_through_in_the_seeds_order() {
        let mut looks = four();
        started(&mut looks, 40.0);
        let mut seen = vec![looks.state().to as usize];
        for i in 0..4 {
            // Weaker cues change it, but not within 30 s of the last change.
            if i > 0 {
                looks.tick(29.0, &MusicValues::default(), 7);
                assert!(!looks.tick(0.0, &unlabelled(0.2), 7));
            }
            looks.tick(1.0, &MusicValues::default(), 7);
            assert!(looks.tick(0.0, &unlabelled(0.2), 7));
            seen.push(looks.state().to as usize);
        }
        // Every look comes, in the order, which comes round after the last.
        let mut once = seen[..4].to_vec();
        once.sort_unstable();
        assert_eq!(once, [0, 1, 2, 3]);
        assert_eq!(seen[4], seen[0]);
        // The order is the seed's: the same seed, the same steps.
        let mut again = four();
        started(&mut again, 40.0);
        again.tick(0.0, &unlabelled(0.2), 7);
        assert_eq!(again.state().to as usize, seen[1]);
        // Another seed may step another way.
        assert!((0..32u32).any(|seed| order_from(seed, 4) != order_from(7, 4)));
    }

    #[test]
    fn the_fade_is_in_beats_when_the_beat_is_followed_else_in_seconds() {
        let followed = |mut m: MusicValues| {
            m.locked = 1.0;
            m.tempo = 60.0;
            m
        };
        let label_for = |looks: &Looks| {
            (0..8)
                .map(|l| l as f32)
                .find(|l| looks.pick(*l, 1) as u32 != looks.state().to)
                .unwrap()
        };
        // One beat at 60 bpm is a second: half-way (eased) at half a second.
        let mut looks = four();
        started(&mut looks, 10.0);
        let label = label_for(&looks);
        assert!(looks.tick(0.0, &followed(section(label, 1.0)), 7));
        assert_eq!(looks.state().mix, 0.0);
        looks.tick(0.5, &followed(MusicValues::default()), 7);
        assert!((looks.state().mix - 0.5).abs() < 1e-5);
        looks.tick(0.5, &followed(MusicValues::default()), 7);
        assert_eq!(looks.state().mix, 1.0);
        // Without a followed beat the fade is half a second whatever the tempo.
        let mut looks = four();
        started(&mut looks, 10.0);
        let label = label_for(&looks);
        let music = MusicValues {
            tempo: 60.0,
            ..section(label, 1.0)
        };
        assert!(looks.tick(0.0, &music, 7));
        looks.tick(0.25, &MusicValues::default(), 7);
        assert!((looks.state().mix - 0.5).abs() < 1e-5);
        looks.tick(0.25, &MusicValues::default(), 7);
        assert_eq!(looks.state().mix, 1.0);
        // The ease is slow at both ends.
        assert!(ease(0.1) < 0.1 && ease(0.9) > 0.9);
    }

    #[test]
    fn a_held_look_is_cut_to_and_kept_until_it_is_released() {
        let mut looks = four();
        // Held before the first frame, the scene opens in it, even one that may not open.
        assert!(looks.hold(1, &MusicValues::default()));
        looks.tick(0.0, &MusicValues::default(), 7);
        assert_eq!(looks.state().to, 1);
        assert!(!looks.hold(4, &MusicValues::default()), "no such look");
        // Held to another: a change with the usual fade, and no more changes by the music.
        started(&mut looks, 40.0);
        assert!(looks.hold(3, &MusicValues::default()));
        let state = *looks.state();
        assert_eq!(
            (state.prev, state.to, state.mix, state.changes),
            (1, 3, 0.0, 1)
        );
        looks.tick(0.5, &MusicValues::default(), 7);
        assert_eq!(looks.state().mix, 1.0);
        for label in 0..8 {
            looks.tick(20.0, &MusicValues::default(), 7);
            assert!(!looks.tick(0.0, &section(label as f32, 1.0), 7));
        }
        assert!(!looks.tick(0.0, &unlabelled(1.0), 7));
        assert_eq!(looks.state().to, 3);
        assert!(looks.view().held);
        // Released, the runtime counts from now: no change at once, then it changes again.
        looks.release();
        assert!(!looks.view().held);
        let label = (0..8)
            .map(|l| l as f32)
            .find(|l| looks.pick(*l, 1) != 3)
            .unwrap();
        assert!(!looks.tick(0.0, &section(label, 1.0), 7));
        looks.tick(8.0, &MusicValues::default(), 7);
        assert!(looks.tick(0.0, &section(label, 1.0), 7));
    }

    #[test]
    fn the_state_has_the_layout_of_struct_look() {
        let state = LookState {
            prev: 1,
            to: 2,
            mix: 0.5,
            changes: 3,
        };
        let bytes = state.bytes();
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), 0.5);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 3);
    }

    #[test]
    fn a_reload_keeps_what_is_under_way() {
        let mut old = four();
        started(&mut old, 10.0);
        old.hold(3, &MusicValues::default());
        let mut new = four();
        new.inherit(&old);
        assert_eq!(new.state(), old.state());
        assert!(new.view().held);
        // A scene with another number of looks starts over.
        let mut other = Looks::new(&[look("a", true, 1.0), look("b", true, 1.0)]);
        other.inherit(&old);
        assert!(!other.view().held && other.state().changes == 0);
    }

    /// The sections of a song from `(seconds, level)` each, one after the other from 0.
    fn song_of(spec: &[(f32, f32)]) -> Arc<[SongSection]> {
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

    /// A night look for every kind of section and a day look for `tiers` that says
    /// `at_least_once` and is never picked by chance (weight 0), and does not open.
    fn night_and_day(tiers: &[&str]) -> Looks {
        let night = look("night", true, 1.0);
        let day = Look {
            tiers: tiers.iter().map(|t| t.to_string()).collect(),
            at_least_once: true,
            ..look("day", false, 0.0)
        };
        Looks::new(&[night, day])
    }

    /// Plays a song through, a section start after another with its whole length between, and
    /// returns the look on in each section. The song's sections are told or not.
    fn play(looks: &mut Looks, song: &Arc<[SongSection]>, tell: bool) -> Vec<u32> {
        looks.set_song(tell.then_some(song));
        looks.tick(0.0, &MusicValues::default(), 7);
        song.iter()
            .enumerate()
            .map(|(i, section)| {
                let frame = MusicValues {
                    section: i as f32,
                    level: section.level,
                    section_energy: section.level,
                    song_time: section.start,
                    ..section_start(i as f32)
                };
                looks.tick(section.end - section.start, &frame, 7);
                looks.state().to
            })
            .collect()
    }

    /// A charted frame where the section labelled `label` starts.
    fn section_start(label: f32) -> MusicValues {
        section(label, 1.0)
    }

    #[test]
    fn a_look_that_chance_never_picks_is_changed_to_at_the_last_section_that_suits_it() {
        // Quiet, high, normal, high, normal: the last high section is the fourth.
        let song = song_of(&[
            (40.0, 0.2),
            (40.0, 0.9),
            (40.0, 0.5),
            (40.0, 0.9),
            (40.0, 0.5),
        ]);
        let mut looks = night_and_day(&["high"]);
        let on = play(&mut looks, &song, true);
        // The day (1) comes at the fourth section's start only; the look the section after it
        // picks by its kind is the night again.
        assert_eq!(on, [0, 0, 0, 1, 0]);
        assert!(!looks.is_owed(1, &MusicValues::default()));
        // Not told of the song, nothing is promised: only chance, which never picks it.
        let mut unknown = night_and_day(&["high"]);
        assert_eq!(play(&mut unknown, &song, false), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_song_with_no_section_of_its_tiers_shows_the_look_at_its_highest() {
        let song = song_of(&[
            (40.0, 0.5),
            (40.0, 0.6),
            (40.0, 0.65),
            (40.0, 0.65),
            (40.0, 0.55),
        ]);
        let mut looks = night_and_day(&["quiet"]);
        assert_eq!(play(&mut looks, &song, true), [0, 0, 0, 1, 0]);
    }

    #[test]
    fn a_look_already_on_is_not_changed_to_again_for_the_promise() {
        // The day is picked by chance in every high section (the night does not suit them), so it
        // has been on long before the last one: the song plays as it does without the promise.
        let song = song_of(&[(40.0, 0.9), (40.0, 0.5), (40.0, 0.9), (40.0, 0.5)]);
        let looks = |promise: bool| {
            let night = Look {
                tiers: vec!["quiet".into(), "normal".into()],
                ..look("night", true, 1.0)
            };
            let day = Look {
                tiers: vec!["high".into()],
                at_least_once: promise,
                ..look("day", false, 1.0)
            };
            Looks::new(&[night, day])
        };
        let with = play(&mut looks(true), &song, true);
        assert_eq!(with, play(&mut looks(false), &song, true));
        assert_eq!(with, [1, 0, 1, 0]);
    }

    #[test]
    fn a_section_start_too_soon_after_the_one_before_is_not_the_last_chance() {
        // The last high section starts 4 s after the one before it, within the look's gap of 8:
        // the change would not be allowed. The high section before it is the last chance.
        let song = song_of(&[(40.0, 0.5), (4.0, 0.9), (40.0, 0.9), (40.0, 0.5)]);
        let mut looks = night_and_day(&["high"]);
        looks.set_song(Some(&song));
        assert_eq!(looks.promised[1], Some(1));
        // Nothing fits: no section starts far enough after another, but the first.
        let song = song_of(&[(4.0, 0.5), (4.0, 0.9)]);
        looks.set_song(Some(&song));
        assert_eq!(looks.promised[1], Some(0));
        let alone = song_of(&[(40.0, 0.9)]);
        looks.set_song(Some(&alone));
        assert_eq!(looks.promised[1], Some(0));
        looks.set_song(None);
        assert_eq!(looks.promised[1], None);
    }

    #[test]
    fn the_song_going_back_owes_the_look_again() {
        let song = song_of(&[(40.0, 0.5), (40.0, 0.9), (40.0, 0.5)]);
        let mut looks = night_and_day(&["high"]);
        assert_eq!(play(&mut looks, &song, true), [0, 1, 0]);
        assert!(looks.shown[1]);
        let at = |section: f32, time: f32| MusicValues {
            section,
            song_time: time,
            ..MusicValues::default()
        };
        assert!(!looks.is_owed(1, &at(0.0, 3.0)));
        // A seek back: a new account, in which only the look on counts.
        looks.tick(0.016, &at(0.0, 3.0), 7);
        assert!(!looks.shown[1] && looks.shown[0]);
        assert!(looks.is_owed(1, &at(0.0, 3.0)));
        // Past its last chance it is not owed any more, and a late section does not change to it.
        assert!(!looks.is_owed(1, &at(2.0, 85.0)));
        assert!(looks.owed(&at(2.0, 85.0)).is_none());
        // Another song forgets what was on too.
        play(&mut looks, &song, true);
        assert!(looks.shown[1]);
        looks.set_song(Some(&song_of(&[(40.0, 0.5), (40.0, 0.9)])));
        assert!(!looks.shown[1]);
    }
}
