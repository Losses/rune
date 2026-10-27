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
//! - **Which.** It depends on what happens then. A scene switch takes a transition of the scene
//!   coming (see `switching`); a cut of the director is dressed with a transition of the scene's
//!   list; with neither, a lens effect of the list plays over the picture. A strong moment takes a
//!   strong effect if the scene lists one, a medium moment only a light one. Among those, the
//!   section's label picks (the same kind of section, the same effect).

use crate::scene::MusicValues;
use crate::switching::{FOLLOWED, LIFT_CUE, SECTION_CUE, pick_by_label};
use crate::transition::Transition;

/// The fewest bars between two effects, unless the second is at a strong moment.
pub const GAP_BARS: f32 = 8.0;
/// The effects that are strong: they play only at a strong moment. The rest are light.
pub const STRONG: &[&str] = &["flash", "glitch", "mosaic", "cutin"];
/// The transitions that may dress a cut, strong first, then light.
const CUT_DRESSINGS: &[&str] = &["flash", "glitch", "mosaic", "dissolve", "fade"];
/// The lens effects, strong first, then light (all of them light now).
const LENS: &[&str] = &["flare", "leak"];

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

/// The moment of a frame, if it is one, and the key its roll is made from.
pub fn moment_of(music: &MusicValues, impact: u32) -> Option<(Strength, u32)> {
    let section = music.section.max(0.0) as u32;
    if music.cues > 0 && music.cue >= LIFT_CUE {
        Some((Strength::Strong, key(section, 0)))
    } else if music.cues > 0 && music.cue >= SECTION_CUE {
        Some((Strength::Medium, key(section, 0)))
    } else if music.impacts > 0 {
        Some((Strength::Medium, key(section, impact)))
    } else {
        None
    }
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
    /// The song time (seconds) of the last effect that played, if any.
    last: Option<f32>,
    /// The section the impacts are counted in, and how many it had.
    section: f32,
    impacts: u32,
}

impl Scheduler {
    /// The moment of this frame, if it plays an effect at `chances`.
    pub fn tick(&mut self, music: &MusicValues, chances: Chances) -> Option<Played> {
        if music.section != self.section {
            self.section = music.section;
            self.impacts = 0;
        }
        if music.impacts > 0 {
            self.impacts += 1;
        }
        let (strength, key) = moment_of(music, self.impacts)?;
        let chance = match strength {
            Strength::Strong => chances.strong,
            Strength::Medium => chances.medium,
        };
        if !rolls(key, chance) {
            return None;
        }
        let bar = if music.locked > FOLLOWED && music.tempo > 0.0 {
            music.meter.max(1.0) * 60.0 / music.tempo
        } else {
            2.0
        };
        let soon = self
            .last
            .is_some_and(|last| (music.song_time - last).abs() < GAP_BARS * bar);
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

/// Of `names` (a list kept apart by kind, see `scene::manifest::Effects`), the ones in `order` that
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

/// A lens effect that may play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    Flare,
    Leak,
}

/// The lens effect that plays at a moment with no cut, from the scene's lens list.
pub fn lens_effect(names: &[String], played: Played) -> Option<Lens> {
    let options = eligible(names, LENS, played.strength);
    pick_by_label(&options, played.label).map(|name| match name {
        "leak" => Lens::Leak,
        _ => Lens::Flare,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            // No lens effect is strong: a strong moment takes a light one.
            let lens = lens_effect(&list, at(Strength::Strong, label)).unwrap();
            assert!(matches!(lens, Lens::Flare | Lens::Leak));
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
}
