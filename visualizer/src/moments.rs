//! How often the effects play at the music's moments. The moments themselves (a section start that
//! lifts the energy is a strong one; another section start, an impact, the start and the end of a
//! song are medium ones) and the choice of effect come from the shared scheduler; this is the
//! chance that a moment gets an effect at all, which the user sets (two settings, and in a scene
//! lab any chance).

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
}
