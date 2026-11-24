//! How a switch to another scene and a cut of the director's camera look: the transitions and
//! how long they take with the music (see `docs/music.md`). The visualizer switches scenes only
//! when asked (the user's choice, a scene lab); it does not move on by itself, also not when the
//! song changes.

use crate::scene::MusicValues;
use crate::transition::{CUTIN_BAND_SECONDS, CUTIN_MIN_HOLD_SECONDS, Durations, Transition};

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

/// The transition and its durations for a switch somebody asks for now (a scene lab), with this
/// music: a fade or a dissolve over a beat or two, a cut at once, a strong one as it is.
pub fn requested(transition: Transition, music: &MusicValues) -> (Transition, Durations) {
    (transition, durations_of(transition, beat_seconds(music)))
}

/// What a lab's override of the scene switches makes of the transition asked for: its own, with
/// the durations its kind takes; with none the one asked for stays.
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
    use crate::transition::CutInStyle;

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
    fn a_requested_switch_takes_the_durations_of_its_kind() {
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
    fn a_requested_cutin_holds_a_beat_or_the_fixed_time() {
        let (kind, durations) = requested(Transition::CutIn(CutInStyle::Echo), &song(0.0));
        assert_eq!(kind, Transition::CutIn(CutInStyle::Echo));
        assert_eq!(durations.out, CUTIN_BAND_SECONDS + 0.5);
        assert_eq!(durations.into, CUTIN_TEAR_SECONDS);
        assert!(!durations.is_instant(kind));
        // The beat not followed: the fixed hold.
        let music = MusicValues {
            locked: 0.0,
            ..song(0.0)
        };
        let (_, durations) = requested(Transition::CutIn(CutInStyle::Echo), &music);
        assert_eq!(
            durations.out,
            CUTIN_BAND_SECONDS + UNFOLLOWED_CUTIN_HOLD_SECONDS
        );
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
}
