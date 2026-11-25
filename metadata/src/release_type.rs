//! Classification of a release independently of its edition and recording metadata.

use serde::{Deserialize, Serialize};

macro_rules! release_kind {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $value)] $variant),+
        }

        impl $name {
            /// The stable name used in storage and filters.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            /// Read a normalized type from tags or storage.
            pub fn from_label(value: &str) -> Option<Self> {
                match value.trim().to_ascii_lowercase().as_str() {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

release_kind! {
    /// The single primary type of a release group.
    ReleaseType {
        Album => "album",
        Ep => "ep",
        Single => "single",
        Broadcast => "broadcast",
        Other => "other",
    }
}

release_kind! {
    /// A classification that can coexist with other secondary classifications.
    ReleaseSecondaryType {
        Compilation => "compilation",
        Soundtrack => "soundtrack",
        Spokenword => "spokenword",
        Interview => "interview",
        Audiobook => "audiobook",
        AudioDrama => "audio drama",
        Live => "live",
        Remix => "remix",
        DjMix => "dj-mix",
        Mixtape => "mixtape/street",
        Demo => "demo",
        FieldRecording => "field recording",
    }
}
