//! When the visualizer moves on to another scene by itself, and how. A switch follows the music
//! instead of a timer: it waits for a place where the music changes (the start of a section that
//! differs from the one before), never switches too early or too late, and fades over a number of
//! beats that lands on that place. The decision is a pure function of the clock and the music's
//! values ([`MusicValues`], see `docs/music.md`), so it can be tested without a scene.

use crate::scene::MusicValues;
use crate::transition::{Durations, Transition};

/// The least time on a scene before the next automatic switch, in seconds of the visualizer's
/// clock (which stands still while paused).
pub const MIN_SECONDS: f64 = 30.0;
/// How long before [`MIN_SECONDS`] the next scene starts building, so that it is ready when the
/// first place to switch comes (a build can take seconds on some drivers).
pub const PREPARE_LEAD_SECONDS: f64 = 8.0;
/// After this long on a scene any section start is a place to switch, also one that repeats the
/// section before, and so is a cue that is not a section start.
pub const RELAX_SECONDS: f64 = 60.0;
/// After this long on a scene the switch happens at the next beat or hit, whatever the music does.
pub const MAX_SECONDS: f64 = 90.0;
/// How long past [`MAX_SECONDS`] the switch waits for a beat or a hit before it happens at once.
pub const MAX_GRACE_SECONDS: f64 = 2.0;
/// The change of a section's energy that makes it a different section for the switch, and that
/// makes a lift or a drop (the cue's own rule for a lift is the same, `scene::signals::CUE_LIFT`).
pub const ENERGY_CHANGE: f32 = 0.1;
/// The strength of a cue from which it is a section start (a cue of 0.3 or 0.2 comes from the
/// fallbacks of `cues`).
pub const SECTION_CUE: f32 = 0.5;
/// The strength of a cue that is a section start with an energy lift.
pub const LIFT_CUE: f32 = 0.8;
/// `locked` above this means the beat is followed, so that the tempo can be used for durations.
pub const FOLLOWED: f32 = 0.5;
/// Without a followed beat: how long a fade takes out, and in, in seconds.
pub const UNFOLLOWED_FADE_SECONDS: f64 = 0.4;
/// Without a followed beat: how long a dissolve takes, in seconds.
pub const UNFOLLOWED_DISSOLVE_SECONDS: f64 = 1.2;
/// A fade takes this many beats out, and the same in.
pub const FADE_BEATS: f64 = 1.0;
/// A dissolve takes this many beats.
pub const DISSOLVE_BEATS: f64 = 2.0;
/// A flash rises for this many beats and falls for [`FLASH_FALL_BEATS`].
pub const FLASH_RISE_BEATS: f64 = 1.0;
pub const FLASH_FALL_BEATS: f64 = 2.0;
/// Without a followed beat: how long a flash rises and falls, in seconds.
pub const UNFOLLOWED_FLASH_SECONDS: (f64, f64) = (0.4, 0.8);
/// A mosaic takes this many beats, or [`UNFOLLOWED_MOSAIC_SECONDS`] without a followed beat.
pub const MOSAIC_BEATS: f64 = 1.0;
pub const UNFOLLOWED_MOSAIC_SECONDS: f64 = 0.5;
/// A glitch takes a few frames, whatever the beat.
pub const GLITCH_SECONDS: f64 = 0.15;
/// A transition that dresses a cut is short, so that it stays on the beat: this fraction of what
/// the same transition takes for a switch.
pub const DRESSED_SHARE: f64 = 0.5;
/// A flash that dresses a cut rises for this many beats, and falls for twice as long.
pub const DRESSED_FLASH_RISE_BEATS: f64 = 0.25;

/// What the render loop does after a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// Nothing.
    Wait,
    /// Start building the next scene and hold it (then tell [`SceneSwitch::prepare_started`]).
    /// Repeated every frame until told.
    Prepare,
    /// Show the scene that was built, through this transition. The wait starts again.
    Switch {
        transition: Transition,
        durations: Durations,
    },
}

/// The kind of change a switch lands on, which picks its transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// A section with more energy than the one before.
    Lift,
    /// A section with less energy than the one before.
    Drop,
    /// Anything else, including a switch that is not at a section start.
    Plain,
}

/// A section as the switch tells them apart: what kind it is and how loud.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Section {
    label: f32,
    energy: f32,
}

impl Section {
    fn of(music: &MusicValues) -> Self {
        Self {
            label: music.section_label,
            energy: music.section_energy,
        }
    }

    fn next_of(music: &MusicValues) -> Self {
        Self {
            label: music.next_label,
            energy: music.next_energy,
        }
    }

    /// Whether the music changes going from `from` to this section. Without a section before
    /// (nothing seen yet) it does.
    fn differs_from(self, from: Option<Section>) -> bool {
        from.is_none_or(|from| {
            from.label != self.label || (from.energy - self.energy).abs() >= ENERGY_CHANGE
        })
    }

    /// The change going from `from` to this section.
    fn change_from(self, from: Option<Section>) -> Change {
        match from {
            Some(from) if self.energy - from.energy >= ENERGY_CHANGE => Change::Lift,
            Some(from) if from.energy - self.energy >= ENERGY_CHANGE => Change::Drop,
            _ => Change::Plain,
        }
    }
}

/// Decides when the next automatic switch happens, from the music. One value lives as long as the
/// render loop; it remembers the sections it saw across switches, since the music goes on.
#[derive(Default)]
pub struct SceneSwitch {
    /// When the scene on screen was shown, on the render loop's clock; set by the first frame
    /// after a [`Self::restart`].
    since: Option<f64>,
    /// Whether [`Decision::Prepare`] was answered since the last restart.
    prepare_started: bool,
    /// The section playing and the one before it, as seen in the music's values.
    current: Option<Section>,
    before: Option<Section>,
    /// The number of the current section, to see a section that repeats the last.
    index: f32,
    /// The transitions the scene held for the switch takes (see [`Self::allow`]).
    allowed: Allowed,
}

/// The transitions a switch may take: those of the scene coming that the runtime draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed {
    pub cut: bool,
    pub fade: bool,
    pub dissolve: bool,
    pub flash: bool,
    pub glitch: bool,
    pub mosaic: bool,
}

impl Default for Allowed {
    /// A scene that says nothing: a fade or a dissolve.
    fn default() -> Self {
        Self {
            cut: false,
            fade: true,
            dissolve: true,
            flash: false,
            glitch: false,
            mosaic: false,
        }
    }
}

impl Allowed {
    /// The strong transitions of these, in the order a lift picks among them. A new strong one
    /// is one more line here and in [`Allowed::of`].
    fn strong(self) -> Vec<Transition> {
        [
            (self.flash, Transition::Flash),
            (self.glitch, Transition::Glitch),
            (self.mosaic, Transition::Mosaic),
        ]
        .into_iter()
        .filter_map(|(allowed, kind)| allowed.then_some(kind))
        .collect()
    }

    /// Of the transitions in `names` (a scene's `effects.switch`), those the runtime draws.
    pub fn of(names: &[String]) -> Self {
        let has = |name: &str| names.iter().any(|n| n == name);
        Self {
            cut: has("cut"),
            fade: has("fade"),
            dissolve: has("dissolve"),
            flash: has("flash"),
            glitch: has("glitch"),
            mosaic: has("mosaic"),
        }
    }
}

impl SceneSwitch {
    /// The scene changed (any load, manual or automatic): count again from the next frame.
    pub fn restart(&mut self) {
        self.since = None;
        self.prepare_started = false;
    }

    /// Seconds the scene on screen has been counted, on the render loop's clock at `now`, once
    /// the first frame of it was seen.
    pub fn seconds(&self, now: f64) -> Option<f64> {
        self.since.map(|since| now - since)
    }

    /// The scene built for the switch takes these transitions; until another is built, the
    /// switch picks among them.
    pub fn allow(&mut self, allowed: Allowed) {
        self.allowed = allowed;
    }

    /// The build [`Decision::Prepare`] asked for is running.
    pub fn prepare_started(&mut self) {
        self.prepare_started = true;
    }

    /// Call once per frame with the music of the frame drawn last. `ready` is whether the next
    /// scene is built and can be shown at once; a switch only happens then, so that a scene that
    /// is late waits for the next place instead of switching off the music. While `paused` the
    /// wait keeps starting over.
    pub fn tick(&mut self, now: f64, music: &MusicValues, paused: bool, ready: bool) -> Decision {
        self.observe(music);
        if paused {
            self.restart();
            return Decision::Wait;
        }
        let since = *self.since.get_or_insert(now);
        let elapsed = now - since;
        if elapsed < MIN_SECONDS - PREPARE_LEAD_SECONDS {
            return Decision::Wait;
        }
        if !self.prepare_started {
            return Decision::Prepare;
        }
        if !ready {
            return Decision::Wait;
        }
        // The section the change lands on, by its label, picks among the strong transitions.
        let change = self
            .at_cue(music, elapsed)
            .map(|change| (change, music.section_label))
            .or_else(|| {
                self.coming(music, elapsed)
                    .map(|change| (change, music.next_label))
            })
            .or_else(|| forced(music, elapsed).map(|change| (change, music.section_label)));
        match change {
            Some((change, label)) => {
                self.restart();
                let (transition, durations) = choose(change, label, music, self.allowed);
                Decision::Switch {
                    transition,
                    durations,
                }
            }
            None => Decision::Wait,
        }
    }

    /// Remember the sections: when the music's section is another one than the last frame's, it
    /// is the current one and the last is the one before.
    fn observe(&mut self, music: &MusicValues) {
        // Without a chart there are no sections (and the first frame of a new scene is the
        // default values).
        if music.has_chart == 0 {
            return;
        }
        let section = Section::of(music);
        match self.current {
            Some(current) if current == section && self.index == music.section => {}
            Some(current) => {
                self.before = Some(current);
                self.current = Some(section);
                self.index = music.section;
            }
            None => {
                self.current = Some(section);
                self.index = music.section;
            }
        }
    }

    /// Whether this frame is a cue that is a place to switch, and what kind of change it is.
    fn at_cue(&self, music: &MusicValues, elapsed: f64) -> Option<Change> {
        if music.cues == 0 || elapsed < MIN_SECONDS {
            return None;
        }
        if music.has_chart == 0 {
            // No analysis: the fallback cues are all there is.
            return Some(Change::Plain);
        }
        if music.cue < SECTION_CUE {
            // The 8 bars of a followed beat and the 16 s without a beat.
            return (elapsed >= RELAX_SECONDS).then_some(Change::Plain);
        }
        let section = Section::of(music);
        if !section.differs_from(self.before) && elapsed < RELAX_SECONDS {
            return None;
        }
        Some(if music.cue >= LIFT_CUE {
            Change::Lift
        } else {
            section.change_from(self.before)
        })
    }

    /// Whether the next section start is known, is a place to switch, and is so near that a
    /// fade must start now to land on it. A dissolve starts at the cue, so it is not early.
    fn coming(&self, music: &MusicValues, elapsed: f64) -> Option<Change> {
        if music.has_chart == 0 || music.cue_in < 0.0 {
            return None;
        }
        let current = Section::of(music);
        let next = Section::next_of(music);
        let change = next.change_from(Some(current));
        if change == Change::Drop {
            return None;
        }
        let at = elapsed + f64::from(music.cue_in);
        let place = at >= MIN_SECONDS && (next.differs_from(Some(current)) || at >= RELAX_SECONDS);
        let (_, durations) = choose(change, music.next_label, music, self.allowed);
        (place && f64::from(music.cue_in) <= durations.out).then_some(change)
    }
}

/// The switch that happens at the next beat or hit once the scene had its [`MAX_SECONDS`], or at
/// once when none comes within [`MAX_GRACE_SECONDS`].
fn forced(music: &MusicValues, elapsed: f64) -> Option<Change> {
    let moment = music.beats > 0 || music.hits > 0 || elapsed >= MAX_SECONDS + MAX_GRACE_SECONDS;
    (elapsed >= MAX_SECONDS && moment).then_some(Change::Plain)
}

/// How long a beat is, if the beat is followed.
fn beat_seconds(music: &MusicValues) -> Option<f64> {
    (music.locked > FOLLOWED && music.tempo > 0.0).then(|| 60.0 / f64::from(music.tempo))
}

/// The option a section's label picks among `options`: the same label gets the same one, so a
/// kind of section keeps its transition (and a name added to `options` is all it takes to use
/// it). None if there are no options.
pub fn pick_by_label<T: Copy>(options: &[T], label: f32) -> Option<T> {
    let mut hashed = (label.max(0.0) as u32)
        .wrapping_add(1)
        .wrapping_mul(0x9e37_79b1);
    hashed = (hashed ^ (hashed >> 15)).wrapping_mul(0x85eb_ca6b);
    hashed ^= hashed >> 13;
    options.get(hashed as usize % options.len().max(1)).copied()
}

/// The whole of the automatic selection of a strong transition, which name at which moment: at
/// a moment of `cue` [`LIFT_CUE`] or more (a section start that lifts the energy), one of
/// `options` by the label of the section; at any other, none. A scene switch (see [`choose`])
/// and a dressed camera cut (see [`dress`]) both ask here, so this is the one place to replace
/// when the moments come from a shared scheduler.
fn auto_pick(cue: f32, options: &[Transition], label: f32) -> Option<Transition> {
    if cue < LIFT_CUE {
        return None;
    }
    pick_by_label(options, label)
}

/// The transition for a change, among those `allowed`. This is the one place that picks it: a
/// lift takes a strong transition if the scene allows any, chosen by the label of the section it
/// lands on; a plain change fades, a drop dissolves. A scene that takes neither of those gets the
/// other one, then a cut, and a fade when it names none the runtime draws.
fn choose(
    change: Change,
    label: f32,
    music: &MusicValues,
    allowed: Allowed,
) -> (Transition, Durations) {
    let beat = beat_seconds(music);
    let cue = if change == Change::Lift { 1.0 } else { 0.0 };
    if let Some(kind) = auto_pick(cue, &allowed.strong(), label) {
        return (kind, durations_of(kind, beat));
    }
    let want = match change {
        Change::Drop => Transition::Dissolve,
        _ => Transition::Fade,
    };
    let other = match want {
        Transition::Dissolve => Transition::Fade,
        _ => Transition::Dissolve,
    };
    let takes = |kind| match kind {
        Transition::Cut => allowed.cut,
        Transition::Fade => allowed.fade,
        Transition::Dissolve => allowed.dissolve,
        Transition::Flash => allowed.flash,
        Transition::Glitch => allowed.glitch,
        Transition::Mosaic => allowed.mosaic,
    };
    let kind = [want, other, Transition::Cut]
        .into_iter()
        .find(|&kind| takes(kind))
        .unwrap_or(Transition::Fade);
    (kind, durations_of(kind, beat))
}

/// How long a switch of `kind` takes, with `beat` seconds to a beat if the beat is followed.
fn durations_of(kind: Transition, beat: Option<f64>) -> Durations {
    match kind {
        Transition::Cut => Durations {
            out: 0.0,
            into: 0.0,
        },
        Transition::Dissolve => Durations {
            out: 0.0,
            into: beat.map_or(UNFOLLOWED_DISSOLVE_SECONDS, |beat| beat * DISSOLVE_BEATS),
        },
        Transition::Fade => {
            let seconds = beat.map_or(UNFOLLOWED_FADE_SECONDS, |beat| beat * FADE_BEATS);
            Durations {
                out: seconds,
                into: seconds,
            }
        }
        Transition::Flash => match beat {
            Some(beat) => Durations {
                out: beat * FLASH_RISE_BEATS,
                into: beat * FLASH_FALL_BEATS,
            },
            None => Durations {
                out: UNFOLLOWED_FLASH_SECONDS.0,
                into: UNFOLLOWED_FLASH_SECONDS.1,
            },
        },
        Transition::Glitch => Durations {
            out: 0.0,
            into: GLITCH_SECONDS,
        },
        Transition::Mosaic => Durations {
            out: 0.0,
            into: beat.map_or(UNFOLLOWED_MOSAIC_SECONDS, |beat| beat * MOSAIC_BEATS),
        },
    }
}

/// How long `kind` takes when it dresses a camera cut: short, so that it stays on the beat.
pub fn dressed_durations(kind: Transition, music: &MusicValues) -> Durations {
    let beat = beat_seconds(music);
    let full = durations_of(kind, beat);
    match kind {
        Transition::Flash => {
            let rise = beat.map_or(UNFOLLOWED_FLASH_SECONDS.0 * DRESSED_SHARE, |beat| {
                beat * DRESSED_FLASH_RISE_BEATS
            });
            Durations {
                out: rise,
                into: rise * 2.0,
            }
        }
        // A few frames already.
        Transition::Glitch => full,
        _ => Durations {
            out: full.out * DRESSED_SHARE,
            into: full.into * DRESSED_SHARE,
        },
    }
}

/// The transition that dresses a camera cut of the director with `cue` in a scene whose
/// `effects.cut` is `names`, if there is one: only a cue of [`LIFT_CUE`] or more (a section start
/// that lifts the energy, or the last cut of a burst before it) is dressed, with the one of the
/// scene's names that the label of the section picks. A scene that names none, or only ones the
/// runtime does not draw, keeps plain cuts.
pub fn dress(
    names: &[String],
    cue: f32,
    label: f32,
    music: &MusicValues,
) -> Option<(Transition, Durations)> {
    let options: Vec<Transition> = names
        .iter()
        .filter_map(|name| Transition::from_name(name))
        .filter(|&kind| kind != Transition::Cut)
        .collect();
    let kind = auto_pick(cue, &options, label)?;
    Some((kind, dressed_durations(kind, music)))
}

/// The transition and its durations for a switch somebody asks for now (a scene lab), as an
/// automatic one of that kind would take with this music: a fade as for a plain change, a dissolve
/// as for a drop, a cut at once, a strong one as it is.
pub fn requested(transition: Transition, music: &MusicValues) -> (Transition, Durations) {
    (transition, durations_of(transition, beat_seconds(music)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A song with a chart: tempo 120 (a beat is 0.5 s), the beat followed, in the section with
    /// this label and energy, no cue.
    fn song(label: f32, energy: f32) -> MusicValues {
        MusicValues {
            has_chart: 1,
            tempo: 120.0,
            locked: 1.0,
            section_label: label,
            section_energy: energy,
            next_label: label,
            next_energy: energy,
            cue_in: -1.0,
            ..MusicValues::default()
        }
    }

    /// The same without a chart: no beat, no sections.
    fn no_chart() -> MusicValues {
        MusicValues {
            tempo: 120.0,
            cue_in: -1.0,
            ..MusicValues::default()
        }
    }

    /// A frame of `base` that starts section number `index` with this label and energy, as a cue
    /// of `cue`.
    fn start(base: MusicValues, index: f32, label: f32, energy: f32, cue: f32) -> MusicValues {
        MusicValues {
            section: index,
            section_label: label,
            section_energy: energy,
            sections: 1,
            cues: 1,
            cue,
            ..base
        }
    }

    /// Feed `music` once per second from `from` to `to` and return the first decision that is
    /// not a wait or a prepare, with its time.
    fn run(
        switch: &mut SceneSwitch,
        from: f64,
        to: f64,
        music: &MusicValues,
    ) -> Option<(f64, Decision)> {
        let mut t = from;
        while t < to {
            match switch.tick(t, music, false, true) {
                Decision::Wait => {}
                Decision::Prepare => switch.prepare_started(),
                decision => return Some((t, decision)),
            }
            t += 1.0;
        }
        None
    }

    /// A switch that saw section 0 (label 0, energy 0.5) from 0 s and section 1 (label 1) start
    /// at 5 s, so that it knows a section before the current one.
    fn seasoned() -> SceneSwitch {
        let mut switch = SceneSwitch::default();
        assert_eq!(
            switch.tick(0.0, &song(0.0, 0.5), false, true),
            Decision::Wait
        );
        switch.prepare_started();
        switch.tick(5.0, &song(1.0, 0.5), false, true);
        switch
    }

    /// A switch that has no chart and started at 0 s.
    fn fresh() -> SceneSwitch {
        let mut switch = SceneSwitch::default();
        assert_eq!(switch.tick(0.0, &no_chart(), false, true), Decision::Wait);
        switch.prepare_started();
        switch
    }

    fn fade(seconds: f64) -> Decision {
        Decision::Switch {
            transition: Transition::Fade,
            durations: Durations {
                out: seconds,
                into: seconds,
            },
        }
    }

    #[test]
    fn nothing_happens_before_the_minimum() {
        let mut switch = seasoned();
        // The wait counts from the first frame (0 s). A section start with a new label at 29 s is
        // too early, and at 30 s it is a switch.
        let early = start(song(0.0, 0.5), 2.0, 0.0, 0.5, 0.5);
        assert_eq!(switch.tick(29.0, &early, false, true), Decision::Wait);
        let change = start(song(0.0, 0.5), 3.0, 2.0, 0.5, 0.5);
        assert_eq!(switch.tick(30.0, &change, false, true), fade(0.5));
    }

    #[test]
    fn the_next_scene_is_prepared_before_it_can_be_used() {
        let mut switch = SceneSwitch::default();
        let music = song(0.0, 0.5);
        assert_eq!(switch.tick(100.0, &music, false, false), Decision::Wait);
        assert_eq!(switch.tick(121.9, &music, false, false), Decision::Wait);
        // MIN - lead = 22 s after the first frame.
        assert_eq!(switch.tick(122.0, &music, false, false), Decision::Prepare);
        // It is asked again until the build started, and then not any more.
        assert_eq!(switch.tick(122.1, &music, false, false), Decision::Prepare);
        switch.prepare_started();
        assert_eq!(switch.tick(122.2, &music, false, false), Decision::Wait);
    }

    #[test]
    fn a_section_with_another_label_switches() {
        let mut switch = seasoned();
        let cue = start(song(0.0, 0.5), 2.0, 2.0, 0.5, 0.5);
        assert_eq!(switch.tick(40.0, &cue, false, true), fade(0.5));
    }

    #[test]
    fn a_section_with_another_energy_switches() {
        let mut switch = seasoned();
        let cue = start(song(1.0, 0.5), 2.0, 1.0, 0.65, 0.5);
        assert_eq!(switch.tick(40.0, &cue, false, true), fade(0.5));
    }

    #[test]
    fn the_same_section_again_waits_until_the_music_relaxes() {
        let mut switch = seasoned();
        // Section 2 repeats section 1's label and energy.
        let again = start(song(1.0, 0.5), 2.0, 1.0, 0.5, 0.5);
        assert_eq!(switch.tick(40.0, &again, false, true), Decision::Wait);
        // After RELAX any section start is enough.
        let again = start(song(1.0, 0.5), 3.0, 1.0, 0.5, 0.5);
        assert_eq!(switch.tick(RELAX_SECONDS, &again, false, true), fade(0.5));
    }

    #[test]
    fn relax_accepts_the_cue_of_8_bars_but_before_it_waits() {
        let mut switch = seasoned();
        let bars = MusicValues {
            cues: 1,
            cue: 0.3,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(40.0, &bars, false, true), Decision::Wait);
        assert_eq!(switch.tick(RELAX_SECONDS, &bars, false, true), fade(0.5));
    }

    #[test]
    fn without_a_chart_any_cue_after_the_minimum_switches() {
        let mut switch = fresh();
        let cue = MusicValues {
            cues: 1,
            cue: 0.2,
            ..no_chart()
        };
        assert_eq!(switch.tick(29.0, &cue, false, true), Decision::Wait);
        // Without a followed beat a fade is 0.4 s out and in.
        assert_eq!(switch.tick(31.0, &cue, false, true), fade(0.4));
    }

    #[test]
    fn the_longest_wait_ends_at_the_next_beat_or_hit() {
        let quiet = song(1.0, 0.5);
        let mut switch = seasoned();
        assert_eq!(
            switch.tick(MAX_SECONDS - 0.1, &quiet, false, true),
            Decision::Wait
        );
        let beat = MusicValues { beats: 1, ..quiet };
        assert_eq!(switch.tick(MAX_SECONDS, &beat, false, true), fade(0.5));

        let mut switch = seasoned();
        let hit = MusicValues { hits: 1, ..quiet };
        assert_eq!(switch.tick(MAX_SECONDS + 0.5, &hit, false, true), fade(0.5));

        // Neither comes: it happens at once after the grace.
        let mut switch = seasoned();
        assert_eq!(
            switch.tick(MAX_SECONDS + 1.9, &quiet, false, true),
            Decision::Wait
        );
        assert_eq!(
            switch.tick(MAX_SECONDS + MAX_GRACE_SECONDS, &quiet, false, true),
            fade(0.5)
        );
    }

    #[test]
    fn a_known_section_start_starts_the_fade_early_to_land_on_it() {
        let mut switch = seasoned();
        // The next section is another one, 0.5 s away (the fade out is 0.5 s): start now.
        let coming = MusicValues {
            next_label: 2.0,
            cue_in: 0.5,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(39.0, &coming, false, true), fade(0.5));
    }

    #[test]
    fn a_fade_that_is_not_yet_due_waits_for_its_start() {
        let mut switch = seasoned();
        let far = MusicValues {
            next_label: 2.0,
            cue_in: 2.0,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(39.0, &far, false, true), Decision::Wait);
        let near = MusicValues { cue_in: 0.5, ..far };
        assert_eq!(switch.tick(40.5, &near, false, true), fade(0.5));
    }

    #[test]
    fn an_early_fade_counts_its_landing_for_the_minimum() {
        let mut switch = seasoned();
        // At 29.8 s the section starts in 0.4 s: it lands at 30.2 s, after the minimum.
        let coming = MusicValues {
            next_label: 2.0,
            cue_in: 0.4,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(29.8, &coming, false, true), fade(0.5));
        // At 28 s it would land at 28.4 s, too early.
        let mut switch = seasoned();
        assert_eq!(switch.tick(28.0, &coming, false, true), Decision::Wait);
    }

    #[test]
    fn a_known_section_that_repeats_the_last_is_not_a_place() {
        let mut switch = seasoned();
        let same = MusicValues {
            cue_in: 0.4,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(40.0, &same, false, true), Decision::Wait);
    }

    #[test]
    fn a_drop_dissolves_over_two_beats_at_the_cue() {
        let mut switch = seasoned();
        // Known in advance: nothing starts early for a dissolve.
        let coming = MusicValues {
            next_label: 2.0,
            next_energy: 0.2,
            cue_in: 0.3,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(40.0, &coming, false, true), Decision::Wait);
        let cue = start(song(1.0, 0.5), 2.0, 2.0, 0.2, 0.5);
        assert_eq!(
            switch.tick(40.3, &cue, false, true),
            Decision::Switch {
                transition: Transition::Dissolve,
                durations: Durations {
                    out: 0.0,
                    into: 1.0
                },
            }
        );
    }

    #[test]
    fn the_scene_coming_picks_among_the_transitions_it_takes() {
        let only = |cut, fade, dissolve| Allowed {
            cut,
            fade,
            dissolve,
            ..Allowed::default()
        };
        let drop = start(song(1.0, 0.5), 2.0, 2.0, 0.2, 0.5);
        // A drop dissolves; a scene that takes only fades fades at it.
        let mut switch = seasoned();
        switch.allow(only(false, true, false));
        assert_eq!(switch.tick(40.0, &drop, false, true), fade(0.5));
        // A plain change fades; a scene that takes only dissolves dissolves at it.
        let mut switch = seasoned();
        switch.allow(only(false, false, true));
        let plain = start(song(1.0, 0.5), 2.0, 3.0, 0.5, 0.5);
        assert!(matches!(
            switch.tick(40.0, &plain, false, true),
            Decision::Switch {
                transition: Transition::Dissolve,
                ..
            }
        ));
        // Only a cut: a cut, and nothing starts early for it.
        let mut switch = seasoned();
        switch.allow(only(true, false, false));
        let coming = MusicValues {
            next_label: 3.0,
            next_energy: 0.5,
            cue_in: 0.3,
            ..song(1.0, 0.5)
        };
        assert_eq!(switch.tick(39.7, &coming, false, true), Decision::Wait);
        let Decision::Switch {
            transition,
            durations,
        } = switch.tick(40.0, &plain, false, true)
        else {
            panic!("no switch");
        };
        assert!(transition == Transition::Cut && durations.is_instant(transition));
        // None the runtime draws (a scene of later effects only): a fade.
        let mut switch = seasoned();
        switch.allow(Allowed::of(&["cutin".to_string()]));
        assert_eq!(switch.tick(40.0, &drop, false, true), fade(0.5));
    }

    #[test]
    fn allowed_transitions_are_read_from_the_names() {
        let names =
            |list: &[&str]| Allowed::of(&list.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(
            names(&["fade", "cutin", "cut"]),
            Allowed {
                cut: true,
                fade: true,
                dissolve: false,
                ..Allowed::default()
            }
        );
        assert_eq!(names(&["fade", "dissolve"]), Allowed::default());
    }

    #[test]
    fn a_dissolve_without_a_followed_beat_takes_1_2_seconds() {
        let mut switch = seasoned();
        let cue = MusicValues {
            locked: 0.0,
            ..start(song(1.0, 0.5), 2.0, 2.0, 0.2, 0.5)
        };
        assert_eq!(
            switch.tick(40.0, &cue, false, true),
            Decision::Switch {
                transition: Transition::Dissolve,
                durations: Durations {
                    out: 0.0,
                    into: 1.2
                },
            }
        );
    }

    #[test]
    fn a_lift_fades_for_a_beat_out_and_in() {
        let mut switch = seasoned();
        let cue = MusicValues {
            tempo: 100.0,
            ..start(song(1.0, 0.5), 2.0, 2.0, 0.9, 1.0)
        };
        assert_eq!(switch.tick(40.0, &cue, false, true), fade(0.6));
    }

    #[test]
    fn a_scene_that_is_not_built_waits_for_the_next_place() {
        let mut switch = seasoned();
        let cue = start(song(0.0, 0.5), 2.0, 2.0, 0.5, 0.5);
        assert_eq!(switch.tick(40.0, &cue, false, false), Decision::Wait);
        // The next frame is no cue any more: ready now does not switch off the music.
        assert_eq!(
            switch.tick(40.02, &song(2.0, 0.5), false, true),
            Decision::Wait
        );
        let later = start(song(2.0, 0.5), 3.0, 3.0, 0.5, 0.5);
        assert_eq!(switch.tick(50.0, &later, false, true), fade(0.5));
    }

    #[test]
    fn a_switch_starts_the_wait_again() {
        let mut switch = seasoned();
        let cue = start(song(0.0, 0.5), 2.0, 2.0, 0.5, 0.5);
        assert_eq!(switch.tick(40.0, &cue, false, true), fade(0.5));
        // The next frame counts from there: prepare in 22 s, and no switch before 30 s.
        let next = start(song(2.0, 0.5), 3.0, 3.0, 0.5, 0.5);
        assert_eq!(switch.tick(40.5, &next, false, true), Decision::Wait);
        assert_eq!(
            switch.tick(62.5, &song(3.0, 0.5), false, true),
            Decision::Prepare
        );
        switch.prepare_started();
        assert_eq!(switch.tick(65.0, &next, false, true), Decision::Wait);
    }

    #[test]
    fn pausing_starts_the_wait_again() {
        let mut switch = seasoned();
        let music = song(1.0, 0.5);
        assert_eq!(switch.tick(100.0, &music, true, true), Decision::Wait);
        assert_eq!(switch.tick(500.0, &music, true, true), Decision::Wait);
        // Unpausing counts from the next frame, not from the old start: no prepare yet.
        assert_eq!(switch.tick(501.0, &music, false, true), Decision::Wait);
        assert_eq!(switch.tick(510.0, &music, false, true), Decision::Wait);
        assert_eq!(switch.tick(523.0, &music, false, true), Decision::Prepare);
    }

    #[test]
    fn a_manual_load_starts_the_wait_again() {
        let mut switch = seasoned();
        switch.tick(25.0, &song(1.0, 0.5), false, true);
        switch.restart();
        // Prepare is asked for again, 22 s after the next frame.
        assert_eq!(
            switch.tick(26.0, &song(1.0, 0.5), false, true),
            Decision::Wait
        );
        assert_eq!(
            switch.tick(48.0, &song(1.0, 0.5), false, true),
            Decision::Prepare
        );
    }

    #[test]
    fn run_helper_finds_a_forced_switch_in_a_quiet_song() {
        let mut switch = SceneSwitch::default();
        let (at, decision) = run(&mut switch, 0.0, 200.0, &song(0.0, 0.5)).unwrap();
        assert!((92.0..=93.0).contains(&at), "{at}");
        assert_eq!(decision, fade(0.5));
    }

    #[test]
    fn a_requested_switch_takes_the_durations_of_an_automatic_one() {
        // At 120 bpm, followed: a fade takes a beat out and in, a dissolve two beats.
        let music = song(0.0, 0.5);
        let (kind, durations) = requested(Transition::Fade, &music);
        assert_eq!(kind, Transition::Fade);
        assert_eq!((durations.out, durations.into), (0.5, 0.5));
        let (kind, durations) = requested(Transition::Dissolve, &music);
        assert_eq!(kind, Transition::Dissolve);
        assert_eq!(durations.into, 1.0);
        // A cut shows nothing.
        let (kind, durations) = requested(Transition::Cut, &music);
        assert!(durations.is_instant(kind));
        // Without a followed beat the fixed times.
        let (_, durations) = requested(Transition::Fade, &no_chart());
        assert_eq!(durations.out, UNFOLLOWED_FADE_SECONDS);
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    /// A lift to the section with this label, 100 bpm.
    fn lift(label: f32) -> MusicValues {
        MusicValues {
            tempo: 100.0,
            ..start(song(1.0, 0.5), 2.0, label, 0.9, 1.0)
        }
    }

    #[test]
    fn a_lift_takes_a_strong_transition_the_scene_allows_by_its_section_label() {
        let strong = Allowed::of(&names(&["fade", "flash", "glitch", "mosaic"]));
        let kind_at = |allowed, label| {
            let mut switch = seasoned();
            switch.allow(allowed);
            match switch.tick(40.0, &lift(label), false, true) {
                Decision::Switch { transition, .. } => transition,
                other => panic!("{other:?}"),
            }
        };
        // The same kind of section, the same transition; and the labels are not all alike.
        let kinds: Vec<Transition> = (0..8).map(|label| kind_at(strong, label as f32)).collect();
        assert_eq!(
            kinds,
            (0..8)
                .map(|label| kind_at(strong, label as f32))
                .collect::<Vec<_>>()
        );
        for kind in [Transition::Flash, Transition::Glitch, Transition::Mosaic] {
            assert!(kinds.contains(&kind), "{kinds:?}");
        }
        // A scene that allows one strong transition gets that one.
        let flash_only = Allowed::of(&names(&["fade", "flash"]));
        assert!((0..8).all(|label| kind_at(flash_only, label as f32) == Transition::Flash));
        // A scene without any fades at a lift, as before.
        assert!((0..8).all(|label| kind_at(Allowed::default(), label as f32) == Transition::Fade));
    }

    #[test]
    fn a_plain_change_fades_and_a_drop_dissolves_whatever_the_scene_allows() {
        let strong = Allowed::of(&names(&["fade", "dissolve", "flash", "glitch", "mosaic"]));
        let mut switch = seasoned();
        switch.allow(strong);
        let drop = start(song(1.0, 0.5), 2.0, 2.0, 0.2, 0.5);
        assert!(matches!(
            switch.tick(40.0, &drop, false, true),
            Decision::Switch {
                transition: Transition::Dissolve,
                ..
            }
        ));
        let mut switch = seasoned();
        switch.allow(strong);
        let plain = start(song(1.0, 0.5), 2.0, 3.0, 0.5, 0.5);
        assert!(matches!(
            switch.tick(40.0, &plain, false, true),
            Decision::Switch {
                transition: Transition::Fade,
                ..
            }
        ));
    }

    #[test]
    fn a_picked_option_is_the_labels_and_an_empty_list_has_none() {
        assert_eq!(pick_by_label::<Transition>(&[], 1.0), None);
        let one = [Transition::Mosaic];
        assert_eq!(pick_by_label(&one, 5.0), Some(Transition::Mosaic));
        let two = [Transition::Flash, Transition::Glitch];
        assert_eq!(pick_by_label(&two, 3.0), pick_by_label(&two, 3.0));
    }

    #[test]
    fn the_strong_transitions_take_a_beat_or_a_few_frames() {
        let beat = |kind| durations_of(kind, Some(0.6));
        assert_eq!(
            beat(Transition::Flash),
            Durations {
                out: 0.6,
                into: 1.2
            }
        );
        assert_eq!(
            beat(Transition::Mosaic),
            Durations {
                out: 0.0,
                into: 0.6
            }
        );
        assert_eq!(
            beat(Transition::Glitch),
            Durations {
                out: 0.0,
                into: GLITCH_SECONDS
            }
        );
        // Without a followed beat: seconds.
        assert_eq!(
            durations_of(Transition::Flash, None),
            Durations {
                out: 0.4,
                into: 0.8
            }
        );
        assert_eq!(durations_of(Transition::Mosaic, None).into, 0.5);
        // A flash for a lift starts early, to peak on the section's start.
        let mut switch = seasoned();
        switch.allow(Allowed::of(&names(&["flash"])));
        let coming = MusicValues {
            next_label: 3.0,
            next_energy: 0.9,
            cue_in: 0.5,
            tempo: 120.0,
            ..song(1.0, 0.5)
        };
        assert!(matches!(
            switch.tick(39.5, &coming, false, true),
            Decision::Switch {
                transition: Transition::Flash,
                durations: Durations { out, into },
            } if out == 0.5 && into == 1.0
        ));
    }

    #[test]
    fn a_requested_strong_switch_is_that_transition_with_its_durations() {
        let music = MusicValues {
            tempo: 100.0,
            ..song(1.0, 0.5)
        };
        assert_eq!(
            requested(Transition::Flash, &music),
            (
                Transition::Flash,
                Durations {
                    out: 0.6,
                    into: 1.2
                }
            )
        );
        assert_eq!(requested(Transition::Glitch, &music).0, Transition::Glitch);
    }

    #[test]
    fn a_cut_is_dressed_only_with_a_lifting_cue_and_a_name_in_the_scenes_list() {
        let music = MusicValues {
            tempo: 100.0,
            ..song(1.0, 0.5)
        };
        let list = names(&["flash", "glitch", "mosaic"]);
        // A weak cue (the beat or a cadence, a section start without a lift): plain.
        assert_eq!(dress(&list, 0.5, 1.0, &music), None);
        assert_eq!(dress(&list, 0.79, 1.0, &music), None);
        // A scene with the default empty list, or names nothing draws: plain.
        assert_eq!(dress(&[], 1.0, 1.0, &music), None);
        assert_eq!(
            dress(&names(&["cutin", "sparkle", "cut"]), 1.0, 1.0, &music),
            None
        );
        // Else one of the names, by the label: the same for the same label.
        let picked: Vec<_> = (0..8)
            .map(|label| dress(&list, 0.8, label as f32, &music).unwrap().0)
            .collect();
        for kind in [Transition::Flash, Transition::Glitch, Transition::Mosaic] {
            assert!(picked.contains(&kind), "{picked:?}");
        }
        assert_eq!(
            dress(&list, 1.0, 3.0, &music),
            dress(&list, 0.9, 3.0, &music)
        );
        // Fade and dissolve can dress a cut too.
        assert_eq!(
            dress(&names(&["dissolve"]), 1.0, 0.0, &music).map(|dressing| dressing.0),
            Some(Transition::Dissolve)
        );
    }

    #[test]
    fn a_dressed_cut_is_short() {
        let music = MusicValues {
            tempo: 120.0,
            ..song(1.0, 0.5)
        };
        // A beat is 0.5 s: the flash rises for a quarter of it and falls for twice as long.
        assert_eq!(
            dressed_durations(Transition::Flash, &music),
            Durations {
                out: 0.125,
                into: 0.25
            }
        );
        assert_eq!(dressed_durations(Transition::Mosaic, &music).into, 0.25);
        assert_eq!(
            dressed_durations(Transition::Glitch, &music).into,
            GLITCH_SECONDS
        );
        assert_eq!(
            dressed_durations(Transition::Fade, &music),
            Durations {
                out: 0.25,
                into: 0.25
            }
        );
        for kind in [
            Transition::Fade,
            Transition::Dissolve,
            Transition::Flash,
            Transition::Glitch,
            Transition::Mosaic,
        ] {
            let d = dressed_durations(kind, &music);
            assert!(d.out + d.into <= 0.5, "{kind:?} {d:?}");
        }
    }
}
