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
use crate::bind::{LookBinds, LookCue};

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
        }
    }

    /// Decide by these bindings from now on (a scene lab replaced `runtime.toml`).
    pub fn set_binds(&mut self, binds: Arc<LookBinds>) {
        self.binds = binds;
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

    /// The look a section picks: by the weights, with the section's label hashed to the roll, so
    /// that a section of the same kind picks the same.
    fn pick(&self, label: f32) -> usize {
        let chances: Vec<(usize, f32)> = self
            .picks
            .iter()
            .enumerate()
            .map(|(i, (_, weight))| (i, *weight))
            .collect();
        weighted(&chances, label_hash(label, PICK_K))
    }

    /// The look after the one on in the order made from the seed.
    fn next(&self) -> usize {
        let at = self
            .order
            .iter()
            .position(|&i| i == self.state.to as usize)
            .unwrap_or(0);
        self.order[(at + 1) % self.order.len()]
    }

    /// Start a fade to look `to` of `seconds`.
    fn change(&mut self, to: usize, seconds: f32) {
        self.state.prev = self.state.to;
        self.state.to = to as u32;
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
        if self.held {
            return false;
        }
        let Some(cue) = self.binds.cue(music) else {
            return false;
        };
        if self.since_change < self.binds.gap(cue) {
            return false;
        }
        let to = match cue {
            LookCue::Section => self.pick(music.section_label),
            LookCue::Unlabelled => self.next(),
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
        }
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
            assert_eq!(to, four().pick(label) as u32);
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
            .find(|l| looks.pick(*l) as u32 != looks.state().to)
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
                .find(|l| looks.pick(*l) as u32 != looks.state().to)
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
                .find(|l| looks.pick(*l) as u32 != looks.state().to)
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
            .find(|l| looks.pick(*l) != 3)
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
}
