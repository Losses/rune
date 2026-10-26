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
}

impl SceneSwitch {
    /// The scene changed (any load, manual or automatic): count again from the next frame.
    pub fn restart(&mut self) {
        self.since = None;
        self.prepare_started = false;
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
        let change = self
            .at_cue(music, elapsed)
            .or_else(|| self.coming(music, elapsed))
            .or_else(|| forced(music, elapsed));
        match change {
            Some(change) => {
                self.restart();
                let (transition, durations) = choose(change, music);
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
        let (_, durations) = choose(change, music);
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

/// The transition for a change. This is the one place that picks it: a lift will get a strong
/// transition here once there are some, for now it fades like any other change.
fn choose(change: Change, music: &MusicValues) -> (Transition, Durations) {
    let beat = beat_seconds(music);
    match change {
        Change::Drop => (
            Transition::Dissolve,
            Durations {
                out: 0.0,
                into: beat.map_or(UNFOLLOWED_DISSOLVE_SECONDS, |beat| beat * DISSOLVE_BEATS),
            },
        ),
        Change::Lift | Change::Plain => {
            let seconds = beat.map_or(UNFOLLOWED_FADE_SECONDS, |beat| beat * FADE_BEATS);
            (
                Transition::Fade,
                Durations {
                    out: seconds,
                    into: seconds,
                },
            )
        }
    }
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
}
