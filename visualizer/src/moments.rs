//! When the effects play: the music's moments, the chance a moment gets an effect, and which
//! effect it is.
//!
//! - **Moments.** A section start that lifts the energy (a cue of 0.8 or more, which the burst of
//!   cuts leads to) is a strong moment. Another section start (a cue of 0.5 or more) and an impact
//!   are medium ones. Nothing else is a moment: no effect on a plain beat or cut.
//! - **Chance.** A moment plays an effect by the chance of its strength ([`Chances`]: the user's
//!   setting, or a scene lab's). The roll is a hash of where the moment is in the song (its
//!   section, and which impact of it), not dice: the same song plays its effects the same way each
//!   time. A medium moment does not play within [`GAP_BARS`] of the last effect.
//! - **Which.** It depends on what happens then. A scene switch, made when a new song starts, takes
//!   a transition of the scene coming (see `switching`); a hard cut of the director is dressed with
//!   a transition of the scene's list; with neither, a lens effect of the list plays over the
//!   picture. A cut to a shot the scene glides to is not dressed: it plays as a moment with no cut
//!   does, and a strong cut is never one of those, it is hard. A strong moment takes a strong
//!   effect if the scene lists one, a medium moment only a light one. Among those, the section's
//!   label picks (the same kind of section, the same effect).

use std::sync::Arc;

use crate::bind::{MomentBinds, RuntimeBindings};
use crate::scene::MusicValues;
use crate::switching::pick_by_label;
use crate::transition::Transition;

/// The fewest bars between two effects, unless the second is at a strong moment. The default of
/// `moments.gap` in `runtime.toml`, which the scheduler reads.
#[cfg(test)]
pub const GAP_BARS: f32 = 8.0;
/// The effects that are strong: they play only at a strong moment. The rest are light.
pub const STRONG: &[&str] = &["flash", "glitch", "mosaic", "cutin"];
/// The transitions that may dress a cut, strong first, then light.
const CUT_DRESSINGS: &[&str] = &["flash", "glitch", "mosaic", "dissolve", "fade"];
/// The lens effects, strong first, then light. A glitch and a mosaic that the scene lists in its
/// `lens` play over the picture at a strong moment with no cut.
const LENS: &[&str] = &["glitch", "mosaic", "flare", "leak"];

/// How often effects play, the user's setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EffectLevel {
    /// An effect at 40% of the strong moments and 15% of the medium ones (where only the light
    /// effects play): about two in a song.
    #[default]
    Default,
    /// 60% and 30%.
    More,
}

impl EffectLevel {
    /// The chance that a strong moment, and a medium one, gets an effect.
    pub fn chances(self) -> Chances {
        match self {
            Self::Default => Chances {
                strong: 0.4,
                medium: 0.15,
            },
            Self::More => Chances {
                strong: 0.6,
                medium: 0.3,
            },
        }
    }
}

/// The chance (0 to 1) that a moment of each strength gets an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chances {
    pub strong: f32,
    pub medium: f32,
}

impl Default for Chances {
    /// The default setting's.
    fn default() -> Self {
        EffectLevel::default().chances()
    }
}

impl Chances {
    /// The same chance for every moment, for a scene lab (1 plays an effect at every moment).
    pub fn every(chance: f32) -> Self {
        let chance = chance.clamp(0.0, 1.0);
        Self {
            strong: chance,
            medium: chance,
        }
    }
}

/// How strong a moment is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Medium,
    Strong,
}

/// A moment of the music that plays an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    pub strength: Strength,
    /// The label of the section it is in, which picks the effect.
    pub label: f32,
}

/// The moment of a frame by the default `moments.*` binds of `runtime.toml`, if it is one, and
/// the key its roll is made from (see [`MomentBinds::moment_of`], which a scheduler asks).
pub fn moment_of(music: &MusicValues, impact: u32) -> Option<(Strength, u32)> {
    RuntimeBindings::embedded().moments.moment_of(music, impact)
}

/// The key of a moment: its section, and 0 for the section's start or the number of an impact.
pub fn key(section: u32, which: u32) -> u32 {
    section.wrapping_mul(0x9e37_79b1) ^ which.wrapping_mul(0x85eb_ca6b)
}

/// Whether the moment of `key` plays at `chance` (0 to 1): a number from 0 to 1 hashed from the
/// key, under the chance.
pub fn rolls(key: u32, chance: f32) -> bool {
    let mut h = key ^ 0x2c1b_3c6d;
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    ((h >> 8) as f32 / (1u32 << 24) as f32) < chance
}

/// Follows the moments of one scene's music and says which play.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// What a moment is and how far apart they are (`moments.*` of `runtime.toml`).
    binds: Arc<MomentBinds>,
    /// The song time (seconds) of the last effect that played, if any.
    last: Option<f32>,
    /// The section the impacts are counted in, and how many it had.
    section: f32,
    impacts: u32,
}

impl Scheduler {
    /// Decide by these binds from now on (a scene lab replaced them).
    pub fn set_binds(&mut self, binds: Arc<MomentBinds>) {
        self.binds = binds;
    }

    /// The moment of this frame, if it plays an effect at `chances`.
    pub fn tick(&mut self, music: &MusicValues, chances: Chances) -> Option<Played> {
        if music.section != self.section {
            self.section = music.section;
            self.impacts = 0;
        }
        if self.binds.counts_other(music) {
            self.impacts += 1;
        }
        let (strength, key) = self.binds.moment_of(music, self.impacts)?;
        let chance = match strength {
            Strength::Strong => chances.strong,
            Strength::Medium => chances.medium,
        };
        if !rolls(key, chance) {
            return None;
        }
        let soon = self.last.is_some_and(|last| {
            self.binds
                .gap_seconds(music)
                .is_some_and(|gap| (music.song_time - last).abs() < gap)
        });
        if strength == Strength::Medium && soon {
            return None;
        }
        self.last = Some(music.song_time);
        Some(Played {
            strength,
            label: music.section_label,
        })
    }
}

/// Of `names` (one of the lists of `scene::manifest::Effects`), the ones in `order` that
/// a moment of `strength` may play: strong and light ones at a strong moment (the strong ones if
/// there are any), light ones only at a medium one.
fn eligible<'a>(names: &[String], order: &[&'a str], strength: Strength) -> Vec<&'a str> {
    let listed: Vec<&str> = order
        .iter()
        .copied()
        .filter(|name| names.iter().any(|n| n == name))
        .collect();
    let (strong, light): (Vec<&str>, Vec<&str>) =
        listed.into_iter().partition(|name| STRONG.contains(name));
    match strength {
        Strength::Strong if !strong.is_empty() => strong,
        _ => light,
    }
}

/// The transition that dresses a cut at a moment that plays, from the scene's cut list.
pub fn cut_dressing(names: &[String], played: Played) -> Option<Transition> {
    let options = eligible(names, CUT_DRESSINGS, played.strength);
    pick_by_label(&options, played.label).and_then(Transition::from_name)
}

/// What a cut at a moment that plays gets: its dressing from the scene's list, or none for a cut
/// the scene glides to (`glides`), which is not dressed and leaves the moment to a lens effect.
pub fn dressing_of_cut(names: &[String], played: Played, glides: bool) -> Option<Transition> {
    if glides {
        None
    } else {
        cut_dressing(names, played)
    }
}

/// The lens effect that plays at a moment over a cut the scene glides to: a light one of the list
/// only (a flare, a leak), never a glitch or a mosaic over a smooth move, which the scene's list
/// keeps for its hard cuts. The section's label picks among them as at any moment.
pub fn lens_over_glide(names: &[String], played: Played) -> Option<Lens> {
    lens_effect(
        names,
        Played {
            strength: Strength::Medium,
            ..played
        },
    )
}

/// A lens effect that may play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    Flare,
    Leak,
    /// A glitch over the live picture (see `crate::transition::Overlay`).
    Glitch,
    /// A mosaic over the live picture.
    Mosaic,
}

/// The lens effect that plays at a moment with no cut, from the scene's lens list (its
/// `[effects] lens`).
pub fn lens_effect(names: &[String], played: Played) -> Option<Lens> {
    let options = eligible(names, LENS, played.strength);
    pick_by_label(&options, played.label).map(|name| match name {
        "glitch" => Lens::Glitch,
        "mosaic" => Lens::Mosaic,
        "leak" => Lens::Leak,
        _ => Lens::Flare,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_the_scene_glides_to_is_not_dressed_and_a_hard_one_is() {
        let list: Vec<String> = ["glitch", "fade"].map(String::from).to_vec();
        let played = Played {
            strength: Strength::Strong,
            label: 1.0,
        };
        assert_eq!(dressing_of_cut(&list, played, true), None);
        assert_eq!(
            dressing_of_cut(&list, played, false),
            cut_dressing(&list, played)
        );
        assert!(dressing_of_cut(&list, played, false).is_some());
    }

    /// Over a glide a strong moment plays only a light lens effect of the list, never the glitch
    /// or the mosaic it would play with no cut, and nothing if the list has no light one.
    #[test]
    fn a_glide_gets_only_a_light_lens_effect() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let list = names(&["glitch", "mosaic", "flare"]);
        for label in 0..8 {
            let played = Played {
                strength: Strength::Strong,
                label: label as f32,
            };
            assert!(matches!(
                lens_effect(&list, played),
                Some(Lens::Glitch | Lens::Mosaic)
            ));
            assert_eq!(lens_over_glide(&list, played), Some(Lens::Flare));
        }
        let strong_only = names(&["glitch"]);
        let played = Played {
            strength: Strength::Strong,
            label: 0.0,
        };
        assert_eq!(lens_over_glide(&strong_only, played), None);
    }

    #[test]
    fn the_settings_are_the_users_numbers() {
        let default = EffectLevel::default().chances();
        assert_eq!((default.strong, default.medium), (0.4, 0.15));
        let more = EffectLevel::More.chances();
        assert_eq!((more.strong, more.medium), (0.6, 0.3));
        assert_eq!(Chances::every(2.0).medium, 1.0);
    }

    fn music(section: f32, cue: f32, impacts: u32, time: f32) -> MusicValues {
        MusicValues {
            has_chart: 1,
            tempo: 120.0,
            meter: 4.0,
            locked: 1.0,
            section,
            section_label: section,
            cues: u32::from(cue > 0.0),
            cue,
            impacts,
            song_time: time,
            ..MusicValues::default()
        }
    }

    #[test]
    fn moments_are_section_starts_and_impacts_and_nothing_else() {
        assert_eq!(
            moment_of(&music(1.0, 1.0, 0, 0.0), 0).unwrap().0,
            Strength::Strong
        );
        assert_eq!(
            moment_of(&music(1.0, 0.5, 0, 0.0), 0).unwrap().0,
            Strength::Medium
        );
        assert_eq!(
            moment_of(&music(1.0, 0.0, 1, 0.0), 1).unwrap().0,
            Strength::Medium
        );
        // The 8-bar cue and plain frames are no moments.
        assert_eq!(moment_of(&music(1.0, 0.3, 0, 0.0), 0), None);
        assert_eq!(moment_of(&music(1.0, 0.0, 0, 0.0), 0), None);
    }

    #[test]
    fn the_roll_is_the_songs_and_follows_the_chance() {
        // The same moment rolls the same way every time.
        for k in 0..50 {
            assert_eq!(rolls(key(k, 0), 0.4), rolls(key(k, 0), 0.4));
            assert!(rolls(key(k, 0), 1.0) && !rolls(key(k, 0), 0.0));
        }
        // Over many moments about the chance plays.
        let played = (0..2000).filter(|&k| rolls(key(k, 3), 0.4)).count();
        assert!((700..900).contains(&played), "{played}");
    }

    #[test]
    fn a_medium_moment_waits_for_the_gap_and_a_strong_one_does_not() {
        let every = Chances::every(1.0);
        let mut scheduler = Scheduler::default();
        assert!(scheduler.tick(&music(1.0, 0.5, 0, 10.0), every).is_some());
        // 8 bars at 120 bpm are 16 s: an impact 6 s later is too soon, a lift is not.
        assert!(scheduler.tick(&music(1.0, 0.0, 1, 16.0), every).is_none());
        assert!(scheduler.tick(&music(2.0, 1.0, 0, 17.0), every).is_some());
        assert!(scheduler.tick(&music(3.0, 0.5, 0, 34.0), every).is_some());
        // Nothing plays at a chance of 0.
        let mut scheduler = Scheduler::default();
        assert!(
            scheduler
                .tick(&music(1.0, 1.0, 0, 0.0), Chances::every(0.0))
                .is_none()
        );
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_strong_moment_takes_a_strong_effect_and_a_medium_one_a_light_one() {
        let list = names(&["fade", "dissolve", "flash", "glitch", "flare", "leak"]);
        let at = |strength, label| Played { strength, label };
        for label in 0..8 {
            let label = label as f32;
            let cut = cut_dressing(&list, at(Strength::Strong, label)).unwrap();
            assert!(
                matches!(cut, Transition::Flash | Transition::Glitch),
                "{cut:?}"
            );
            let cut = cut_dressing(&list, at(Strength::Medium, label)).unwrap();
            assert!(
                matches!(cut, Transition::Fade | Transition::Dissolve),
                "{cut:?}"
            );
            // The strong lens effect of the list: the glitch over the picture.
            let lens = lens_effect(&list, at(Strength::Strong, label)).unwrap();
            assert_eq!(lens, Lens::Glitch);
            let lens = lens_effect(&list, at(Strength::Medium, label)).unwrap();
            assert!(matches!(lens, Lens::Flare | Lens::Leak));
        }
        // The same label, the same effect.
        assert_eq!(
            cut_dressing(&list, at(Strength::Strong, 3.0)),
            cut_dressing(&list, at(Strength::Strong, 3.0))
        );
        // A strong moment in a scene of light effects only takes a light one; a medium moment in
        // a scene of strong ones only takes none.
        let light = names(&["dissolve", "leak"]);
        assert_eq!(
            cut_dressing(&light, at(Strength::Strong, 1.0)),
            Some(Transition::Dissolve)
        );
        assert_eq!(
            lens_effect(&light, at(Strength::Strong, 1.0)),
            Some(Lens::Leak)
        );
        let strong = names(&["glitch", "mosaic"]);
        assert_eq!(cut_dressing(&strong, at(Strength::Medium, 1.0)), None);
        assert_eq!(lens_effect(&strong, at(Strength::Medium, 1.0)), None);
        assert_eq!(lens_effect(&[], at(Strength::Strong, 1.0)), None);
    }

    /// A glitch or a mosaic that the scene lists plays over the picture at a strong moment with
    /// no cut, and never at a medium one: they are strong.
    #[test]
    fn a_listed_glitch_or_mosaic_is_a_strong_lens_effect_too() {
        let at = |strength, label| Played { strength, label };
        let list = names(&["fade", "glitch", "mosaic", "flare"]);
        let picked: Vec<Lens> = (0..16)
            .filter_map(|label| lens_effect(&list, at(Strength::Strong, label as f32)))
            .collect();
        assert!(picked.contains(&Lens::Glitch) && picked.contains(&Lens::Mosaic));
        assert!(
            picked
                .iter()
                .all(|l| matches!(l, Lens::Glitch | Lens::Mosaic))
        );
        for label in 0..16 {
            let medium = lens_effect(&list, at(Strength::Medium, label as f32));
            assert_eq!(medium, Some(Lens::Flare));
        }
        assert!(STRONG.contains(&"glitch") && STRONG.contains(&"mosaic"));
    }

    #[test]
    fn the_runtime_file_has_todays_numbers() {
        let runtime = RuntimeBindings::embedded();
        let m = &runtime.moments;
        assert_eq!(m.strong.len(), 1);
        assert_eq!(m.strong[0].strength, 0.8);
        assert_eq!(m.medium[0].strength, 0.5);
        assert_eq!(m.medium[0].pick.source().name, "cues");
        assert_eq!(m.medium[1].pick.source().name, "impacts");
        assert_eq!(m.medium[1].strength, 0.0);
        assert_eq!(m.gap_every, GAP_BARS);
        // Without a followed beat a bar counts as 2 s.
        assert_eq!(m.gap.fallback_value(), Some(2.0));
    }

    #[test]
    fn an_impact_that_the_file_calls_strong_plays_a_strong_moment() {
        let impact = music(1.0, 0.0, 1, 0.0);
        let play = |runtime: &RuntimeBindings| {
            let mut scheduler = Scheduler::default();
            scheduler.set_binds(runtime.moments.clone());
            scheduler
                .tick(&impact, Chances::every(1.0))
                .map(|p| p.strength)
        };
        assert_eq!(play(&RuntimeBindings::embedded()), Some(Strength::Medium));
        let custom = crate::bind::tests::embedded_with(
            "target = \"moments.strong\"\nsource = \"cues\"\nhow = \"trigger\"\nwhen = { strength = 0.8 }",
            "target = \"moments.strong\"\nsource = \"impacts\"\nhow = \"trigger\"\nwhen = { strength = 0.0 }",
        );
        assert_eq!(play(&custom), Some(Strength::Strong));
        // And a section start of 0.8 is then a medium one at most (it is a cue, not an impact).
        let lift = music(1.0, 1.0, 0, 0.0);
        let mut scheduler = Scheduler::default();
        scheduler.set_binds(custom.moments.clone());
        assert_eq!(
            scheduler
                .tick(&lift, Chances::every(1.0))
                .map(|p| p.strength),
            Some(Strength::Medium)
        );
    }

    #[test]
    fn the_gap_between_effects_follows_the_file() {
        let at = |time| music(1.0, 0.0, 1, time);
        let second = |runtime: &RuntimeBindings| {
            let mut scheduler = Scheduler::default();
            scheduler.set_binds(runtime.moments.clone());
            assert!(scheduler.tick(&at(0.0), Chances::every(1.0)).is_some());
            // A bar is 2 s at 120 bpm: 10 s is within 8 bars, and past 1.
            scheduler.tick(&at(10.0), Chances::every(1.0)).is_some()
        };
        assert!(!second(&RuntimeBindings::embedded()));
        let custom =
            crate::bind::tests::embedded_with("when = { every = 8 }", "when = { every = 1 }");
        assert!(second(&custom));
    }
}
