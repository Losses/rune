//! Tracks that only a platform's own SDK may play, such as DRM-protected tracks of a streaming
//! service. They stay in the player's queue, and while one is current a host on the platform
//! plays it and reports back. The player keeps the queue, the play order and its modes, and the
//! host holds only what the player gives it: the current track, and the tracks after it that the
//! same host plays too, so that it moves on without a gap, and plays on through them while the
//! player cannot run, as in an app that the system suspended.
//!
//! # The host contract
//!
//! Hosts may be written in Swift, Kotlin or JavaScript, and what follows is all that a host has
//! to know. Messages are JSON in camelCase. The names of the actions and of the `jukebox` events
//! come from Subsonic's `jukeboxControl`, and stay as they are on the wire; its `get` and
//! `shuffle` actions are not used.
//!
//! ## Start
//!
//! A host reports [`HostEvent::HostReady`] when it starts and again after every restart, before
//! it applies any batch. A host that starts holds nothing, so a `hostReady` from the host of the
//! current track makes the player hand that track over again. `authorized: false` makes its
//! tracks unavailable as access revoked, and `canPlay: false`, as from a platform with no player
//! for the host, as a library that cannot be reached: the track the host plays fails, and its
//! other tracks fail without being sent to it, until a later `hostReady` says that it can play.
//!
//! A host with a volume of its own, which `setGain` sets, says `ownVolume: true` there, and the
//! app's volume control stays the player's volume while the host plays. Without it, `setGain`
//! may do nothing, and that control moves the system volume instead. The player itself does not
//! read it.
//!
//! `showsInMediaControls: false` says that the platform player does not show itself in the
//! system's media controls, as on the lock screen, and takes no commands from them: the player
//! then shows the host's tracks there as it shows its own, and what the controls ask reaches the
//! host in batches. A host that leaves it out, or sends `true`, shows itself there, and the
//! player's own media controls give way while it plays. Either way, the platform player looks
//! after the audio of the host's tracks itself, such as the audio focus.
//!
//! ## Batches and the queue
//!
//! The player sends [`HostCommand`]s: batches of [`HostAction`]s under one revision, each higher
//! than the one before, which the host applies in order.
//!
//! The host keeps a queue of track ids, and one entry of it is current. Indexes count from 0 at
//! the latest `set`, and name the same entry until the next `set`: after a `set` the player only
//! appends an entry, or removes the last one before the host gets to it. Entries before the
//! current one stay in this queue, so that indexes hold, but a host should drop them from the
//! platform player's own queue: the system's controls then cannot take the platform player back
//! to a track the player has moved on from. A host that forgets played entries on its side still
//! takes and reports indexes as they were. A host never removes the entry it plays: it ignores a
//! `remove` aimed at it and keeps reporting, as when the player changed its next track just as the
//! host moved on to it.
//!
//! After the current track the host holds the run of tracks that it plays next in the player's
//! order, up to [`RUN_LIMIT`] of them: a host must take a `set` of the current track and that
//! many more, and `add`s up to that many after the current one.
//!
//! A hand-over is one batch: `set`, `skip` to index 0 at the place to play from, `setGain`, and
//! `start`, or `stop` to wait there paused. While the track plays, `remove` and `add` change the
//! tracks after it in place, from the tail. When the host's tracks stop being current, other than
//! by the host moving on, it gets `stop` and `clear`.
//!
//! ## Reports
//!
//! [`HostEvent::JukeboxStatus`] names the current entry by its index and by its id, and says
//! whether it plays and where it stands. A host reports one once it applied a batch, at least
//! every few seconds while it plays, and at once when it moves on by itself. Once told to play,
//! it has 10 s for its first report; after that, 15 s without a report while it plays counts as
//! a failure of its track.
//!
//! Between two statuses the player moves the track on by itself while it plays. `buffering: true`
//! says that the entry is told to play but does not move on yet, as while the platform player
//! loads its audio: it still counts as playing, and stays where the status puts it until a status
//! without it. A host that tells the two apart reports a status at once when either begins.
//!
//! At the end of an entry with an entry after it, the host makes that one current, plays it
//! without a gap, and reports a status with its index, one higher, and its id.
//! [`HostEvent::JukeboxEnded`] means that the last entry finished with nothing after it; a move
//! to the next entry is never reported as an end. A status whose entry is not the one the player
//! put at that index tells the player that the host lost its queue: at the current track's
//! index, the track is handed over again, and fails if that happens twice; at a higher index, the
//! current track counts as played through, and the track that comes next is handed over anew.
//!
//! A host that could not report for a while, as in an app that the system suspended while the
//! host played on, reports where it stands as soon as it runs again. It may then have moved on by
//! several entries: a status at a higher index counts the current track and those between as
//! played through, and `jukeboxEnded` names the entry that ended in `index`, so that an ended
//! entry above the current one counts it and every entry up to that one as played through. A stop
//! that the host did not see happen while it could not report is not a failure of its track.
//! Neither side counts such a time against the other's deadlines: a wait that ends more than
//! [`FROZEN_GAP`] later than it should means that the process did not run meanwhile.
//!
//! [`HostEvent::JukeboxItemFailed`] says that the entry at `index` cannot be played. The host
//! does not move on to an entry that it cannot play: when the entry before it ends, it reports
//! `jukeboxEnded`. The player takes a failed entry after the current one, and those after it, out
//! of the host's queue, hands it over anew once it is current, and marks it unavailable if it
//! fails then too.
//!
//! ## Revisions
//!
//! Every event carries the revision of the latest batch that the host applied.
//! - A status counts unless a later batch changed which entry is current, where it stands or
//!   whether it plays: `set`, `skip`, `start`, `stop` and `clear` make earlier statuses stale,
//!   while `add`, `remove` and `setGain` do not. A status about an entry the player no longer
//!   keeps at that index is told apart by its id.
//! - `jukeboxEnded` and `jukeboxItemFailed` about an entry count from the batch that gave the host
//!   that entry: the `set`, or the `add` that queued it. The failure of the next entry names its
//!   index, one past the current one.
//! - A host that gets a batch after its queue ended reports `jukeboxEnded` again, with that
//!   batch's revision.
//!
//! ## Next, Previous and seeking
//!
//! Next, while the host holds the track that Next goes to, is a `skip` to that entry at 0 and a
//! `start`. When the host had just moved on to it by itself, the track starts again from 0.
//! Previous, and any other choice of track, is a new hand-over. A seek is a `skip` within the
//! current entry.
//!
//! ## Unknown input
//!
//! Events of unknown types and extra fields are ignored, and a message that does not parse is
//! logged and dropped. A `jukeboxItemFailed` with a reason this version does not know still
//! counts: its track is unavailable as unreachable. `positionMs` may be an integer or a float.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::{debug, warn};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};

use crate::player::UnavailableReason;

/// How many tracks after the current one a host holds at most.
pub const RUN_LIMIT: usize = 25;

/// A wait that ends this much later than it should means that the process did not run meanwhile,
/// as in an app that the system suspended. The player and the hosts take the same threshold.
pub const FROZEN_GAP: Duration = Duration::from_secs(5);

/// A track that a platform host plays: the host family, and the id that host understands.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostedTrack {
    pub host: String,
    pub key: String,
}

/// How the paths of hosted tracks begin.
const HOSTED_SCHEME: &str = "hosted://";

impl HostedTrack {
    /// The path the player reports for the track. It names no file.
    pub fn path(&self) -> PathBuf {
        PathBuf::from(format!("{HOSTED_SCHEME}{}/{}", self.host, self.key))
    }
}

/// Whether the player reports `path` for a hosted track, which has no file to read.
pub fn is_hosted_path(path: &Path) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .starts_with(HOSTED_SCHEME.as_bytes())
}

/// The host family of the hosted track that `path` names; none for any other path.
pub fn host_of_path(path: &Path) -> Option<&str> {
    let rest = path.to_str()?.strip_prefix(HOSTED_SCHEME)?;
    rest.split('/').next()
}

/// A batch of actions for one host, to be applied in order under one revision. Each batch has a
/// higher revision than the batch before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCommand {
    pub host: String,
    pub revision: u64,
    pub actions: Vec<HostAction>,
}

/// What a host is told to do. Indexes name entries of its queue, and hold until the next `Set`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostAction {
    /// The queue becomes these ids, and the first becomes current: the track to play, and the
    /// tracks after it that the same host plays too.
    Set { ids: Vec<String> },
    /// Appends exactly one id at the tail: a track after the last one the host holds. It starts
    /// nothing, even after the queue ended.
    Add { ids: Vec<String> },
    /// Removes the last entry, which comes after the current one, before it plays. A host never
    /// removes the entry it plays: it ignores a `remove` aimed at it.
    Remove { index: u32 },
    /// Empties the queue: the host holds nothing until the next `Set`.
    Clear,
    /// Makes the entry at `index` current at `offset_ms`, playing or paused as before. Within
    /// the current entry, this is a seek.
    Skip { index: u32, offset_ms: u64 },
    /// Plays the current entry.
    Start,
    /// Pauses the current entry where it stands.
    Stop,
    /// The volume, from 0 to 1.
    SetGain { gain: f32 },
}

/// What a host reports to the player. Every event but `HostReady` carries the revision of the
/// latest batch that the host applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostEvent {
    /// The host started, or started again, and holds nothing.
    HostReady {
        host: String,
        authorized: bool,
        can_play: bool,
        /// Whether the platform player shows itself in the system's media controls. A host that
        /// leaves it out does, and it is written only when it is false.
        #[serde(default = "shows_itself", skip_serializing_if = "is_true")]
        shows_in_media_controls: bool,
    },
    /// Which entry is current, by its index and its id, whether it plays and where it stands.
    /// `current_index` and `current_id` are none when the host holds no track, as after it
    /// restarted.
    JukeboxStatus {
        host: String,
        revision: u64,
        current_index: Option<u32>,
        #[serde(default)]
        current_id: Option<String>,
        playing: bool,
        #[serde(deserialize_with = "position_ms")]
        position_ms: u64,
        /// Whether the entry is told to play but does not move on yet. A host that leaves it out
        /// says that it moves on, and it is written only when it is true.
        #[serde(default, skip_serializing_if = "is_false")]
        buffering: bool,
    },
    /// The entry at `index` cannot be played.
    JukeboxItemFailed {
        host: String,
        revision: u64,
        index: u32,
        reason: HostFailure,
    },
    /// The last entry of the host's queue finished, and nothing follows it. `index` names that
    /// entry. Hosts that report every move as it happens may leave it out; one that moved on
    /// while it could not report must give it.
    JukeboxEnded {
        host: String,
        revision: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
    },
    /// An event of a type this version does not know.
    #[serde(other)]
    Unknown,
}

impl HostEvent {
    /// Reads an event from a host's JSON. Input that does not parse, and events of types this
    /// version does not know, are logged and dropped.
    pub fn parse(json: &str) -> Option<Self> {
        match serde_json::from_str(json) {
            Ok(Self::Unknown) => {
                debug!("Ignoring a host event of an unknown type: {json}");
                None
            }
            Ok(event) => Some(event),
            Err(error) => {
                warn!("Ignoring a host event that does not parse: {error}");
                None
            }
        }
    }
}

// What a `hostReady` without `showsInMediaControls` says.
fn shows_itself() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

// Hosts written in JavaScript may report a fractional position.
fn position_ms<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    struct Millis;

    impl Visitor<'_> for Millis {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a position in milliseconds")
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
            Ok(value)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<u64, E> {
            Ok(u64::try_from(value).unwrap_or(0))
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<u64, E> {
            // A cast saturates, and takes NaN to zero.
            Ok(value.max(0.0).round() as u64)
        }
    }

    deserializer.deserialize_any(Millis)
}

/// Why a host could not play a track. Not authorized makes the track unavailable as access
/// revoked, unplayable as gone from the catalog, and any other reason as unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostFailure {
    Unplayable,
    NotAuthorized,
    Network,
    /// A reason this version does not know.
    #[serde(other)]
    Unknown,
}

impl From<HostFailure> for UnavailableReason {
    fn from(failure: HostFailure) -> Self {
        match failure {
            HostFailure::Unplayable => Self::TrackGone,
            HostFailure::NotAuthorized => Self::AccessRevoked,
            HostFailure::Network | HostFailure::Unknown => Self::SourceUnreachable,
        }
    }
}

/// Carries commands to the platform hosts. Sending must not block: the player calls it from its
/// own loop, and the outcome comes back as [`HostEvent`]s.
pub trait HostBridge: Send + Sync {
    fn send(&self, command: HostCommand);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use std::fmt::Debug;

    fn assert_json<T>(value: T, json: &str)
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        assert_eq!(serde_json::to_string(&value).unwrap(), json);
        assert_eq!(serde_json::from_str::<T>(json).unwrap(), value);
    }

    #[test]
    fn a_batch_carries_jukebox_actions_in_camel_case() {
        assert_json(
            HostCommand {
                host: "host".to_owned(),
                revision: 12,
                actions: vec![
                    HostAction::Set {
                        ids: vec!["track-1".to_owned()],
                    },
                    HostAction::Skip {
                        index: 0,
                        offset_ms: 0,
                    },
                    HostAction::SetGain { gain: 0.6 },
                    HostAction::Start,
                ],
            },
            r#"{"host":"host","revision":12,"actions":[{"action":"set","ids":["track-1"]},{"action":"skip","index":0,"offsetMs":0},{"action":"setGain","gain":0.6},{"action":"start"}]}"#,
        );
        let cases = [
            (
                HostAction::Add {
                    ids: vec!["1440".to_owned(), "1441".to_owned()],
                },
                r#"{"action":"add","ids":["1440","1441"]}"#,
            ),
            (
                HostAction::Remove { index: 2 },
                r#"{"action":"remove","index":2}"#,
            ),
            (HostAction::Clear, r#"{"action":"clear"}"#),
            (
                HostAction::Skip {
                    index: 0,
                    offset_ms: 1500,
                },
                r#"{"action":"skip","index":0,"offsetMs":1500}"#,
            ),
            (HostAction::Stop, r#"{"action":"stop"}"#),
        ];
        for (action, json) in cases {
            assert_json(action, json);
        }
    }

    #[test]
    fn events_are_tagged_by_type_in_camel_case() {
        assert_json(
            HostEvent::HostReady {
                host: "host".to_owned(),
                authorized: true,
                can_play: false,
                shows_in_media_controls: true,
            },
            r#"{"type":"hostReady","host":"host","authorized":true,"canPlay":false}"#,
        );
        assert_json(
            HostEvent::JukeboxStatus {
                host: "host".to_owned(),
                revision: 3,
                current_index: Some(0),
                current_id: Some("track-1".to_owned()),
                playing: true,
                position_ms: 1500,
                buffering: false,
            },
            r#"{"type":"jukeboxStatus","host":"host","revision":3,"currentIndex":0,"currentId":"track-1","playing":true,"positionMs":1500}"#,
        );
        assert_json(
            HostEvent::JukeboxStatus {
                host: "host".to_owned(),
                revision: 3,
                current_index: None,
                current_id: None,
                playing: false,
                position_ms: 0,
                buffering: false,
            },
            r#"{"type":"jukeboxStatus","host":"host","revision":3,"currentIndex":null,"currentId":null,"playing":false,"positionMs":0}"#,
        );
        assert_json(
            HostEvent::JukeboxStatus {
                host: "host".to_owned(),
                revision: 3,
                current_index: Some(0),
                current_id: Some("track-1".to_owned()),
                playing: true,
                position_ms: 0,
                buffering: true,
            },
            r#"{"type":"jukeboxStatus","host":"host","revision":3,"currentIndex":0,"currentId":"track-1","playing":true,"positionMs":0,"buffering":true}"#,
        );
        for (reason, json) in [
            (HostFailure::Unplayable, "unplayable"),
            (HostFailure::NotAuthorized, "notAuthorized"),
            (HostFailure::Network, "network"),
        ] {
            assert_json(
                HostEvent::JukeboxItemFailed {
                    host: "host".to_owned(),
                    revision: 3,
                    index: 0,
                    reason,
                },
                &format!(
                    r#"{{"type":"jukeboxItemFailed","host":"host","revision":3,"index":0,"reason":"{json}"}}"#
                ),
            );
        }
        assert_json(
            HostEvent::JukeboxEnded {
                host: "host".to_owned(),
                revision: 3,
                index: None,
            },
            r#"{"type":"jukeboxEnded","host":"host","revision":3}"#,
        );
        assert_json(
            HostEvent::JukeboxEnded {
                host: "host".to_owned(),
                revision: 3,
                index: Some(4),
            },
            r#"{"type":"jukeboxEnded","host":"host","revision":3,"index":4}"#,
        );
    }

    #[test]
    fn a_host_says_only_when_its_platform_player_does_not_show_itself_in_the_media_controls() {
        let ready = |shows_in_media_controls| HostEvent::HostReady {
            host: "host".to_owned(),
            authorized: true,
            can_play: true,
            shows_in_media_controls,
        };
        assert_json(
            ready(false),
            r#"{"type":"hostReady","host":"host","authorized":true,"canPlay":true,"showsInMediaControls":false}"#,
        );
        assert_json(
            ready(true),
            r#"{"type":"hostReady","host":"host","authorized":true,"canPlay":true}"#,
        );
        assert_eq!(
            HostEvent::parse(
                r#"{"type":"hostReady","host":"host","authorized":true,"canPlay":true,"showsInMediaControls":true}"#
            ),
            Some(ready(true))
        );
    }

    #[test]
    fn a_status_without_a_current_entry_has_none() {
        let event: HostEvent = serde_json::from_str(
            r#"{"type":"jukeboxStatus","host":"host","revision":1,"playing":false,"positionMs":0}"#,
        )
        .unwrap();
        assert!(matches!(
            event,
            HostEvent::JukeboxStatus {
                current_index: None,
                current_id: None,
                ..
            }
        ));
    }

    #[test]
    fn a_fractional_position_is_rounded_to_milliseconds() {
        assert_eq!(
            HostEvent::parse(
                r#"{"type":"jukeboxStatus","host":"host","revision":1,"currentIndex":0,"currentId":"track-1","playing":true,"positionMs":81250.5}"#
            ),
            Some(HostEvent::JukeboxStatus {
                host: "host".to_owned(),
                revision: 1,
                current_index: Some(0),
                current_id: Some("track-1".to_owned()),
                playing: true,
                position_ms: 81251,
                buffering: false,
            })
        );
        assert!(matches!(
            HostEvent::parse(
                r#"{"type":"jukeboxStatus","host":"host","revision":1,"currentIndex":0,"playing":true,"positionMs":-3}"#
            ),
            Some(HostEvent::JukeboxStatus { position_ms: 0, .. })
        ));
    }

    #[test]
    fn unknown_types_and_reasons_do_not_break_parsing() {
        assert_eq!(
            serde_json::from_str::<HostEvent>(r#"{"type":"somethingNew","extra":1}"#).unwrap(),
            HostEvent::Unknown
        );
        assert_eq!(HostEvent::parse(r#"{"type":"somethingNew"}"#), None);
        assert_eq!(HostEvent::parse("not json"), None);
        assert_eq!(
            HostEvent::parse(
                r#"{"type":"jukeboxItemFailed","host":"host","revision":3,"index":0,"reason":"regionLocked"}"#
            ),
            Some(HostEvent::JukeboxItemFailed {
                host: "host".to_owned(),
                revision: 3,
                index: 0,
                reason: HostFailure::Unknown,
            })
        );
    }

    #[test]
    fn hosted_paths_are_told_from_files() {
        let track = HostedTrack {
            host: "host".to_owned(),
            key: "1440".to_owned(),
        };
        assert!(is_hosted_path(&track.path()));
        assert!(!is_hosted_path(Path::new("/music/hosted.flac")));
        assert!(!is_hosted_path(Path::new("")));
    }

    #[test]
    fn a_hosted_path_names_its_host() {
        let track = HostedTrack {
            host: "appleMusic".to_owned(),
            key: "i.s1".to_owned(),
        };
        assert_eq!(host_of_path(&track.path()), Some("appleMusic"));
        assert_eq!(host_of_path(Path::new("/music/hosted.flac")), None);
        assert_eq!(host_of_path(Path::new("")), None);
    }

    #[test]
    fn host_failures_map_to_queue_reasons() {
        assert_eq!(
            UnavailableReason::from(HostFailure::Unplayable),
            UnavailableReason::TrackGone
        );
        assert_eq!(
            UnavailableReason::from(HostFailure::NotAuthorized),
            UnavailableReason::AccessRevoked
        );
        assert_eq!(
            UnavailableReason::from(HostFailure::Network),
            UnavailableReason::SourceUnreachable
        );
        assert_eq!(
            UnavailableReason::from(HostFailure::Unknown),
            UnavailableReason::SourceUnreachable
        );
    }
}
