//! When the visualizer moves on to another scene by itself, and how. A scene stays for as long as
//! the song it came on with plays, and the next one comes when another file starts: it is built in
//! the background while the old scene keeps drawing, then shown as soon as it is built, through
//! black if the new song is silent at that moment (the gap between two tracks), else on the next
//! beat or hit through a strong transition. A song is a playing file, so a pause, a seek or the
//! same file starting again is no change, and a song of any length keeps its scene. The decision is
//! a pure function of the clock, the playing file and the music's values ([`MusicValues`], see
//! `docs/music.md`), so it can be tested without a scene.

use std::path::{Path, PathBuf};

use crate::bind::audible;
use crate::scene::MusicValues;
use crate::transition::{
    CUTIN_BAND_SECONDS, CUTIN_MIN_HOLD_SECONDS, CutInStyle, Durations, Transition,
};

/// A new song is silent when `audible` (see `docs/music.md`) is below this: the gap between two
/// tracks, or the fade-out of the last one.
pub const SILENT_AUDIBLE: f32 = 0.2;
/// How long the switch waits for a beat or a hit once the next scene is built, in seconds of the
/// visualizer's clock. Without one in that time (a long note, no beat heard) it happens at once.
pub const GIVE_UP_SECONDS: f64 = 2.0;
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
/// A cut-in holds its band for this many beats once it has opened.
pub const CUTIN_HOLD_BEATS: f64 = 1.0;
/// Without a followed beat: how long a cut-in holds its band, in seconds.
pub const UNFOLLOWED_CUTIN_HOLD_SECONDS: f64 = 0.6;
/// How long the band of a cut-in tears away, in seconds: ten frames at 60 fps (the cracks held
/// for three, then the strips flying apart on twos).
pub const CUTIN_TEAR_SECONDS: f64 = 10.0 / 60.0;
/// A flash rises for this many beats and falls for [`FLASH_FALL_BEATS`].
pub const FLASH_RISE_BEATS: f64 = 1.0;
pub const FLASH_FALL_BEATS: f64 = 2.0;
/// Without a followed beat: how long a flash rises and falls, in seconds.
pub const UNFOLLOWED_FLASH_SECONDS: (f64, f64) = (0.4, 0.8);
/// A mosaic takes this many beats, or [`UNFOLLOWED_MOSAIC_SECONDS`] without a followed beat.
pub const MOSAIC_BEATS: f64 = 1.0;
pub const UNFOLLOWED_MOSAIC_SECONDS: f64 = 0.5;
/// A glitch takes this many beats, or [`UNFOLLOWED_GLITCH_SECONDS`] without a followed beat: long
/// enough for its stages to hold a few frames each.
pub const GLITCH_BEATS: f64 = 1.0;
pub const UNFOLLOWED_GLITCH_SECONDS: f64 = 0.5;
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

/// Decides when the next automatic switch happens: when the playing file is another one than the
/// scene on screen came on with. One value lives as long as the render loop.
#[derive(Default)]
pub struct SceneSwitch {
    /// The file the scene on screen came on with, once it is known ([`Self::known`]); none if
    /// nothing was playing then.
    song: Option<PathBuf>,
    /// Whether [`Self::song`] was read since the last [`Self::restart`]: the first frame of a scene
    /// takes the file that plays as the one the scene came on with.
    known: bool,
    /// Another file began to play: the next scene is wanted.
    wanted: bool,
    /// Whether [`Decision::Prepare`] was answered since the last restart.
    prepare_started: bool,
    /// When the next scene was first seen built, on the render loop's clock.
    ready_since: Option<f64>,
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
    pub cutin: bool,
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
            cutin: false,
        }
    }
}

impl Allowed {
    /// The strong transitions of these, in the order a lift picks among them. A new strong one
    /// is one more line here and in [`Allowed::of`]. `label` is the label of the section the
    /// switch lands on, for a cut-in, which dresses itself by the kind of section.
    fn strong(self, label: f32, has_chart: bool) -> Vec<Transition> {
        [
            (self.flash, Transition::Flash),
            (self.glitch, Transition::Glitch),
            (self.mosaic, Transition::Mosaic),
            (
                self.cutin,
                Transition::CutIn(CutInStyle::of_section(label, has_chart)),
            ),
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
            cutin: has("cutin"),
        }
    }
}

impl SceneSwitch {
    /// The scene changed (any load, manual or automatic): the file playing now is the one it came
    /// on with, and what was wanted is forgotten.
    pub fn restart(&mut self) {
        self.known = false;
        self.wanted = false;
        self.prepare_started = false;
        self.ready_since = None;
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

    /// Call once per frame with the file that plays (none: nothing, or a stream that has no path)
    /// and the music of the frame drawn last. `ready` is whether the next scene is built and can
    /// be shown at once. While `paused` (the scene is locked, or a menu is open) the scene counts
    /// as having come on with the file that plays, so a song that begins meanwhile is no change.
    pub fn tick(
        &mut self,
        now: f64,
        song: Option<&Path>,
        music: &MusicValues,
        paused: bool,
        ready: bool,
    ) -> Decision {
        if paused {
            self.restart();
            return Decision::Wait;
        }
        if !self.known {
            self.song = song.map(Path::to_path_buf);
            self.known = true;
            return Decision::Wait;
        }
        if !self.wanted {
            match (&self.song, song) {
                (_, None) => {}
                // The first song after nothing played: the scene was not on with any.
                (None, Some(path)) => self.song = Some(path.to_path_buf()),
                (Some(known), Some(path)) => self.wanted = known != path,
            }
            if !self.wanted {
                return Decision::Wait;
            }
        }
        if !self.prepare_started {
            return Decision::Prepare;
        }
        if !ready {
            self.ready_since = None;
            return Decision::Wait;
        }
        let since = *self.ready_since.get_or_insert(now);
        let silent = audible(music) < SILENT_AUDIBLE;
        if !silent && music.hits == 0 && now - since < GIVE_UP_SECONDS {
            return Decision::Wait;
        }
        self.restart();
        let (transition, durations) = choose(silent, music.section_label, music, self.allowed);
        Decision::Switch {
            transition,
            durations,
        }
    }
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

/// The transition for a switch, among those `allowed`. This is the one place that picks it: a
/// silent new song fades through black, whatever the scene allows. Otherwise the switch takes a
/// strong transition if the scene allows any, chosen by the label of the section playing, so that
/// the same kind of section gets the same one; a scene that allows none fades, else dissolves, else
/// cuts, and fades when it names none the runtime draws.
fn choose(
    silent: bool,
    label: f32,
    music: &MusicValues,
    allowed: Allowed,
) -> (Transition, Durations) {
    let beat = beat_seconds(music);
    if silent {
        return (Transition::Fade, durations_of(Transition::Fade, beat));
    }
    let strong = allowed.strong(label, music.has_chart != 0);
    if let Some(kind) = pick_by_label(&strong, label) {
        return (kind, durations_of(kind, beat));
    }
    let takes = |kind| match kind {
        Transition::Cut => allowed.cut,
        Transition::Fade => allowed.fade,
        Transition::Dissolve => allowed.dissolve,
        Transition::Flash => allowed.flash,
        Transition::Glitch => allowed.glitch,
        Transition::Mosaic => allowed.mosaic,
        Transition::CutIn(_) => allowed.cutin,
    };
    let kind = [Transition::Fade, Transition::Dissolve, Transition::Cut]
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
            into: beat.map_or(UNFOLLOWED_GLITCH_SECONDS, |beat| beat * GLITCH_BEATS),
        },
        Transition::Mosaic => Durations {
            out: 0.0,
            into: beat.map_or(UNFOLLOWED_MOSAIC_SECONDS, |beat| beat * MOSAIC_BEATS),
        },
        // The band opens and holds for `out` (a beat of hold if the beat is followed) over the old
        // scene, and tears for `into`. The new scene goes in with the tear, so a switch that
        // knows the section start starts `out` before it.
        Transition::CutIn(_) => {
            let hold = beat
                .map_or(UNFOLLOWED_CUTIN_HOLD_SECONDS, |beat| {
                    beat * CUTIN_HOLD_BEATS
                })
                .max(CUTIN_MIN_HOLD_SECONDS);
            Durations {
                out: CUTIN_BAND_SECONDS + hold,
                into: CUTIN_TEAR_SECONDS,
            }
        }
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
        _ => Durations {
            out: full.out * DRESSED_SHARE,
            into: full.into * DRESSED_SHARE,
        },
    }
}

/// The transition and its durations for a switch somebody asks for now (a scene lab), as an
/// automatic one of that kind would take with this music: a fade as for a plain change, a dissolve
/// as for a drop, a cut at once, a strong one as it is.
pub fn requested(transition: Transition, music: &MusicValues) -> (Transition, Durations) {
    (transition, durations_of(transition, beat_seconds(music)))
}

/// What a lab's override of the scene switches makes of the choice: its transition, with the
/// durations its kind takes, in place of whatever was chosen; with none the choice stays.
pub fn replaced(
    by: Option<Transition>,
    chosen: (Transition, Durations),
    music: &MusicValues,
) -> (Transition, Durations) {
    by.map_or(chosen, |kind| requested(kind, music))
}

/// What a lab's override of the camera cuts makes of the dressing `chosen` for a cut (none for a
/// plain one): its transition, as short as a dressed cut is, in place of whatever was chosen, and
/// a cut for a plain one; with none the choice stays.
pub fn cut_dressing(
    by: Option<Transition>,
    chosen: Option<(Transition, Durations)>,
    music: &MusicValues,
) -> Option<(Transition, Durations)> {
    match by {
        None => chosen,
        Some(Transition::Cut) => None,
        Some(kind) => Some((kind, dressed_durations(kind, music))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An override replaces the choice of a switch or a cut, and none brings the choice back.
    #[test]
    fn a_lab_override_replaces_the_choice_and_none_restores_it() {
        let music = MusicValues {
            tempo: 120.0,
            locked: 1.0,
            has_chart: 1,
            ..MusicValues::default()
        };
        let fade = requested(Transition::Fade, &music);
        let switched = |by| replaced(by, fade, &music);
        assert_eq!(switched(None), fade);
        assert_eq!(
            switched(Some(Transition::Mosaic)),
            requested(Transition::Mosaic, &music)
        );
        assert_eq!(
            switched(Some(Transition::Cut)),
            requested(Transition::Cut, &music)
        );

        let chosen = Some((
            Transition::Flash,
            dressed_durations(Transition::Flash, &music),
        ));
        let cut = |by, chosen| cut_dressing(by, chosen, &music);
        assert_eq!(cut(None, chosen), chosen);
        assert_eq!(cut(None, None), None);
        // Plain removes a dressing the scene would give, and any other dresses a plain cut.
        assert_eq!(cut(Some(Transition::Cut), chosen), None);
        let glitch = Some((
            Transition::Glitch,
            dressed_durations(Transition::Glitch, &music),
        ));
        assert_eq!(cut(Some(Transition::Glitch), chosen), glitch);
        assert_eq!(cut(Some(Transition::Glitch), None), glitch);
    }

    #[test]
    fn a_requested_switch_takes_the_durations_of_an_automatic_one() {
        // At 120 bpm, followed: a fade takes a beat out and in, a dissolve two beats.
        let music = song(0.0);
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

    #[test]
    fn a_picked_option_is_the_labels_and_an_empty_list_has_none() {
        assert_eq!(pick_by_label::<Transition>(&[], 1.0), None);
        let one = [Transition::Mosaic];
        assert_eq!(pick_by_label(&one, 5.0), Some(Transition::Mosaic));
        let two = [Transition::Flash, Transition::Glitch];
        assert_eq!(pick_by_label(&two, 3.0), pick_by_label(&two, 3.0));
    }

    #[test]
    fn the_strong_transitions_take_a_beat() {
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
                into: 0.6
            }
        );
        assert_eq!(durations_of(Transition::Glitch, None).into, 0.5);
        // Without a followed beat: seconds.
        assert_eq!(
            durations_of(Transition::Flash, None),
            Durations {
                out: 0.4,
                into: 0.8
            }
        );
        assert_eq!(durations_of(Transition::Mosaic, None).into, 0.5);
    }

    #[test]
    fn a_requested_strong_switch_is_that_transition_with_its_durations() {
        let music = MusicValues {
            tempo: 100.0,
            ..song(1.0)
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
    fn a_dressed_cut_is_short() {
        let music = MusicValues {
            tempo: 120.0,
            ..song(1.0)
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
        // A glitch too: half a beat, its stages a few frames each still.
        assert_eq!(dressed_durations(Transition::Glitch, &music).into, 0.25);
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

    #[test]
    fn a_requested_cutin_takes_the_durations_of_an_automatic_one() {
        let (kind, durations) = requested(Transition::CutIn(CutInStyle::Echo), &song(0.0));
        assert_eq!(kind, Transition::CutIn(CutInStyle::Echo));
        assert_eq!(durations.out, CUTIN_BAND_SECONDS + 0.5);
        assert_eq!(durations.into, CUTIN_TEAR_SECONDS);
        assert!(!durations.is_instant(kind));
    }

    /// A song with a chart, playing at a usual loudness: tempo 120 (a beat is 0.5 s), the beat
    /// followed, in the section with this label.
    fn song(label: f32) -> MusicValues {
        MusicValues {
            has_chart: 1,
            tempo: 120.0,
            locked: 1.0,
            loudness: 0.5,
            section_label: label,
            ..MusicValues::default()
        }
    }

    /// The same without a chart: no beat, no sections.
    fn no_chart() -> MusicValues {
        MusicValues {
            tempo: 120.0,
            loudness: 0.5,
            ..MusicValues::default()
        }
    }

    /// A frame of `music` with a beat.
    fn beat(music: MusicValues) -> MusicValues {
        MusicValues {
            hits: 1,
            beats: 1,
            ..music
        }
    }

    /// A frame of `music` in the gap between two songs.
    fn silence(music: MusicValues) -> MusicValues {
        MusicValues {
            loudness: 0.0,
            ..music
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    const A: &str = "/music/a.flac";
    const B: &str = "/music/b.flac";

    fn tick(switch: &mut SceneSwitch, now: f64, path: &str, music: &MusicValues) -> Decision {
        switch.tick(now, Some(Path::new(path)), music, false, true)
    }

    /// A scene that came on with the song A, which still plays at 1 s.
    fn on_song_a() -> SceneSwitch {
        let mut switch = SceneSwitch::default();
        assert_eq!(tick(&mut switch, 0.0, A, &song(0.0)), Decision::Wait);
        switch.allow(Allowed::of(&names(&["fade", "flash", "glitch", "mosaic"])));
        switch
    }

    /// B begins at 10 s and its scene is built at once (the build is told as it is asked for).
    fn song_b_begins(switch: &mut SceneSwitch, music: &MusicValues) {
        assert_eq!(tick(switch, 10.0, B, music), Decision::Prepare);
        assert_eq!(tick(switch, 10.1, B, music), Decision::Prepare);
        switch.prepare_started();
    }

    fn fade_through_black(seconds: f64) -> Decision {
        Decision::Switch {
            transition: Transition::Fade,
            durations: Durations {
                out: seconds,
                into: seconds,
            },
        }
    }

    fn is_strong(decision: Decision) -> bool {
        matches!(
            decision,
            Decision::Switch {
                transition: Transition::Flash
                    | Transition::Glitch
                    | Transition::Mosaic
                    | Transition::CutIn(_),
                ..
            }
        )
    }

    #[test]
    fn nothing_switches_within_one_song_however_long_it_is() {
        let mut switch = on_song_a();
        // Ten minutes of sections, strong cues, beats and a silence in the middle of the song.
        for step in 0..1200 {
            let now = f64::from(step) * 0.5;
            let music = MusicValues {
                section: (now / 20.0).floor() as f32,
                section_label: ((now / 20.0).floor() % 3.0) as f32,
                section_energy: ((now / 20.0).floor() % 2.0) as f32,
                cue: 1.0,
                cues: u32::from(step % 40 == 0),
                ..beat(song(0.0))
            };
            let music = if (300.0..304.0).contains(&now) {
                silence(music)
            } else {
                music
            };
            assert_eq!(
                tick(&mut switch, now, A, &music),
                Decision::Wait,
                "at {now}"
            );
        }
    }

    #[test]
    fn a_new_song_builds_the_next_scene_and_switches_at_its_first_beat() {
        let mut switch = on_song_a();
        let music = song(2.0);
        song_b_begins(&mut switch, &music);
        // The build is under way, and the old scene goes on.
        assert_eq!(
            switch.tick(11.0, Some(Path::new(B)), &beat(music), false, false),
            Decision::Wait
        );
        // Built: it waits for a beat, then takes a strong transition of the scene.
        assert_eq!(tick(&mut switch, 12.0, B, &music), Decision::Wait);
        assert!(is_strong(tick(&mut switch, 12.3, B, &beat(music))));
        // Switched: this song's scene is the new one, and B going on is no change.
        for step in 0..100 {
            let now = 13.0 + f64::from(step);
            assert_eq!(tick(&mut switch, now, B, &beat(music)), Decision::Wait);
        }
    }

    #[test]
    fn without_a_beat_the_switch_takes_the_first_hit_or_gives_up_after_two_seconds() {
        let mut switch = on_song_a();
        song_b_begins(&mut switch, &no_chart());
        assert_eq!(tick(&mut switch, 20.0, B, &no_chart()), Decision::Wait);
        assert_eq!(tick(&mut switch, 21.9, B, &no_chart()), Decision::Wait);
        let late = tick(&mut switch, 22.0, B, &no_chart());
        assert!(is_strong(late), "{late:?}");
        // Another time a hit comes before that.
        let mut switch = on_song_a();
        song_b_begins(&mut switch, &no_chart());
        assert_eq!(tick(&mut switch, 20.0, B, &no_chart()), Decision::Wait);
        assert!(is_strong(tick(&mut switch, 20.5, B, &beat(no_chart()))));
    }

    #[test]
    fn a_new_song_that_starts_in_silence_fades_through_black_at_once() {
        let mut switch = on_song_a();
        let quiet = silence(no_chart());
        song_b_begins(&mut switch, &quiet);
        // The fixed fade, as the beat is not followed.
        assert_eq!(
            tick(&mut switch, 10.5, B, &quiet),
            fade_through_black(UNFOLLOWED_FADE_SECONDS)
        );
        // With the beat followed (a fade-out of a chart's song): a beat out and in.
        let mut switch = on_song_a();
        let quiet = silence(song(0.0));
        song_b_begins(&mut switch, &quiet);
        assert_eq!(tick(&mut switch, 10.5, B, &quiet), fade_through_black(0.5));
    }

    #[test]
    fn a_scene_that_is_not_built_yet_waits_on_the_old_one() {
        let mut switch = on_song_a();
        let music = song(1.0);
        song_b_begins(&mut switch, &silence(music));
        for step in 0..50 {
            let now = 11.0 + f64::from(step);
            let decision = switch.tick(now, Some(Path::new(B)), &silence(music), false, false);
            assert_eq!(decision, Decision::Wait);
        }
        assert_eq!(
            tick(&mut switch, 62.0, B, &silence(music)),
            fade_through_black(0.5)
        );
    }

    #[test]
    fn pausing_seeking_and_the_same_file_again_are_no_change() {
        let mut switch = on_song_a();
        let at = |song_time: f32| MusicValues {
            song_time,
            ..song(0.0)
        };
        // A seek is a jump of the song's time, a pause is a standing one.
        assert_eq!(tick(&mut switch, 5.0, A, &at(150.0)), Decision::Wait);
        assert_eq!(tick(&mut switch, 6.0, A, &at(3.0)), Decision::Wait);
        assert_eq!(tick(&mut switch, 7.0, A, &at(3.0)), Decision::Wait);
        // No file for a moment (between two reports) and the same one again (repeat one).
        let none = switch.tick(8.0, None, &song(0.0), false, true);
        assert_eq!(none, Decision::Wait);
        assert_eq!(tick(&mut switch, 9.0, A, &at(0.0)), Decision::Wait);
        assert_eq!(tick(&mut switch, 9.5, A, &beat(at(0.0))), Decision::Wait);
    }

    #[test]
    fn a_locked_scene_stays_across_songs_and_the_next_change_after_it_switches() {
        let mut switch = on_song_a();
        // Locked (the loop passes `paused`): B begins and then C, with nothing happening.
        let music = song(0.0);
        assert_eq!(
            switch.tick(10.0, Some(Path::new(B)), &music, true, false),
            Decision::Wait
        );
        assert_eq!(
            switch.tick(500.0, Some(Path::new(B)), &music, true, false),
            Decision::Wait
        );
        // Unlocked while B plays: B is what the scene is on with, so nothing yet.
        for step in 0..20 {
            let now = 501.0 + f64::from(step);
            assert_eq!(tick(&mut switch, now, B, &beat(music)), Decision::Wait);
        }
        // Then the next song changes it.
        assert_eq!(tick(&mut switch, 600.0, A, &music), Decision::Prepare);
    }

    #[test]
    fn a_scene_loaded_by_hand_takes_the_song_that_plays_and_drops_what_was_wanted() {
        let mut switch = on_song_a();
        song_b_begins(&mut switch, &song(0.0));
        switch.restart();
        for step in 0..10 {
            let now = 11.0 + f64::from(step);
            assert_eq!(tick(&mut switch, now, B, &beat(song(0.0))), Decision::Wait);
        }
    }

    #[test]
    fn nothing_switches_without_a_file() {
        let mut switch = SceneSwitch::default();
        switch.allow(Allowed::of(&names(&["flash"])));
        for step in 0..1000 {
            let now = f64::from(step);
            let decision = switch.tick(now, None, &beat(song(0.0)), false, true);
            assert_eq!(decision, Decision::Wait);
        }
        // The first song after that is what the scene is on with, not a change; the next is.
        assert_eq!(tick(&mut switch, 1000.0, A, &song(0.0)), Decision::Wait);
        assert_eq!(tick(&mut switch, 1001.0, A, &song(0.0)), Decision::Wait);
        assert_eq!(tick(&mut switch, 1002.0, B, &song(0.0)), Decision::Prepare);
    }

    #[test]
    fn the_scene_coming_picks_the_strong_transition_by_the_section_label() {
        let strong = Allowed::of(&names(&["fade", "flash", "glitch", "mosaic"]));
        let music = song(1.0);
        let kind_at = |allowed, label| choose(false, label, &music, allowed).0;
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
        // A scene without any fades, else dissolves, else cuts, and fades if it names none.
        assert_eq!(kind_at(Allowed::default(), 2.0), Transition::Fade);
        let only = |name: &str| Allowed::of(&names(&[name]));
        assert_eq!(kind_at(only("dissolve"), 2.0), Transition::Dissolve);
        assert_eq!(kind_at(only("cut"), 2.0), Transition::Cut);
        assert_eq!(kind_at(only("nothing"), 2.0), Transition::Fade);
        // In silence it fades through black whatever the scene takes.
        assert_eq!(choose(true, 2.0, &music, strong).0, Transition::Fade);
    }

    /// Allowed with only the cut-in as a strong transition, and the plain ones as ever.
    fn with_cutin() -> Allowed {
        Allowed {
            cutin: true,
            ..Allowed::default()
        }
    }

    #[test]
    fn a_switch_cuts_in_when_the_scene_takes_it() {
        // A beat of hold at 120 bpm, in the style of the section's label.
        let style = CutInStyle::of_section(2.0, true);
        assert_eq!(
            choose(false, 2.0, &song(2.0), with_cutin()),
            (
                Transition::CutIn(style),
                Durations {
                    out: CUTIN_BAND_SECONDS + 0.5,
                    into: CUTIN_TEAR_SECONDS,
                }
            )
        );
        // The beat not followed: the fixed hold.
        let music = MusicValues {
            locked: 0.0,
            ..song(2.0)
        };
        assert_eq!(
            choose(false, 2.0, &music, with_cutin()).1.out,
            CUTIN_BAND_SECONDS + UNFOLLOWED_CUTIN_HOLD_SECONDS
        );
    }
}
