//! Carries messages between the player and the platform host on this device, which lives in the
//! app (Dart). Nothing here leaves the device.

use log::{debug, warn};
use playback::{
    hosted::{HostBridge, HostCommand, HostEvent},
    player::Playable,
};
use rinf::{DartSignal, RustSignal};
use tokio::sync::Mutex;

use crate::messages::{HostedPlayerCommand, HostedPlayerEvent};
use crate::node::NodeSlot;
use crate::service::reach::Reach;

/// Serialises batches of commands and hands them to `sink`, which must not block.
pub(crate) struct JsonHostBridge {
    sink: Box<dyn Fn(String) + Send + Sync>,
}

impl JsonHostBridge {
    pub(crate) fn new(sink: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            sink: Box::new(sink),
        }
    }

    /// Sends commands to the app on this device.
    pub(crate) fn to_app() -> Self {
        Self::new(|payload| HostedPlayerCommand { payload }.send_signal_to_dart())
    }
}

impl HostBridge for JsonHostBridge {
    fn send(&self, command: HostCommand) {
        match serde_json::to_string(&command) {
            Ok(payload) => (self.sink)(payload),
            Err(error) => warn!("Could not encode a host command: {error}"),
        }
    }
}

/// Hands what a host reported to the player. Bad JSON and types this version does not know are
/// logged and dropped.
async fn deliver_host_event(player: &Mutex<dyn Playable>, payload: &str) {
    if let Some(event) = HostEvent::parse(payload) {
        player.lock().await.deliver_host_event(event);
    }
}

/// What a platform host reports of this device's network. It is for the library of the service
/// on that host rather than the player, which has no use for it.
#[derive(serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ReachabilityEvent {
    NetworkState {
        host: String,
        online: bool,
        /// The ids of the tracks stored on this device, as the library names them. Only given
        /// while offline.
        #[serde(default)]
        downloaded: Vec<String>,
    },
}

/// The host and the reach a `networkState` event reports. None for any other payload.
fn reachability(payload: &str) -> Option<(String, Reach)> {
    let ReachabilityEvent::NetworkState {
        host,
        online,
        downloaded,
    } = serde_json::from_str(payload).ok()?;
    let reach = if online {
        Reach::default()
    } else {
        Reach::offline(downloaded.into_iter().collect())
    };
    Some((host, reach))
}

/// Passes the app's host events to the local session's player, and what a host reports of the
/// network to the libraries of the service on it.
pub async fn receive_hosted_player_events(node: &'static NodeSlot) {
    let receiver = HostedPlayerEvent::get_dart_signal_receiver();
    while let Some(signal) = receiver.recv().await {
        if let Some((host, reach)) = reachability(&signal.message.payload) {
            match node.current().await {
                // A report from a host no service plays through is the player's to ignore.
                Ok(node) if !node.services.reach_changed(&host, reach) => {}
                Ok(_) => continue,
                Err(error) => {
                    debug!("Dropping a network report before the node is ready: {error:#}");
                    continue;
                }
            }
        }
        let session = match node.current().await.and_then(|node| node.session()) {
            Ok(session) => session,
            Err(error) => {
                debug!("Dropping a host event before the node is ready: {error:#}");
                continue;
            }
        };
        deliver_host_event(&session.player, &signal.message.payload).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex as StdMutex};

    use playback::hosted::{HostAction, HostFailure};

    fn parsed(payloads: &[&str]) -> Vec<HostEvent> {
        payloads
            .iter()
            .filter_map(|payload| HostEvent::parse(payload))
            .collect()
    }

    #[test]
    fn a_network_state_event_names_the_downloaded_tracks_only_while_offline() {
        assert_eq!(
            reachability(
                r#"{"type":"networkState","host":"platformHost","online":false,"downloaded":["a","b"]}"#
            ),
            Some((
                "platformHost".to_owned(),
                Reach::offline(HashSet::from(["a".to_owned(), "b".to_owned()]))
            ))
        );
        assert_eq!(
            reachability(r#"{"type":"networkState","host":"platformHost","online":false}"#),
            Some(("platformHost".to_owned(), Reach::offline(HashSet::new())))
        );
        assert_eq!(
            reachability(
                r#"{"type":"networkState","host":"platformHost","online":true,"downloaded":["a"]}"#
            ),
            Some(("platformHost".to_owned(), Reach::default()))
        );
    }

    #[test]
    fn other_events_are_not_network_states() {
        for payload in [
            r#"{"type":"hostReady","host":"platformHost","authorized":true,"canPlay":true}"#,
            r#"{"type":"networkState","host":"platformHost"}"#,
            "not json",
        ] {
            assert_eq!(reachability(payload), None, "{payload}");
        }
        // The player ignores a network state.
        assert_eq!(
            HostEvent::parse(r#"{"type":"networkState","host":"platformHost","online":false}"#),
            None
        );
    }

    #[test]
    fn batches_reach_the_sink_as_json() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let sink = seen.clone();
        let bridge = JsonHostBridge::new(move |payload| sink.lock().unwrap().push(payload));
        bridge.send(HostCommand {
            host: "host".into(),
            revision: 1,
            actions: vec![
                HostAction::Set {
                    ids: vec!["track-1".into(), "track-2".into()],
                },
                HostAction::Skip {
                    index: 0,
                    offset_ms: 1500,
                },
                HostAction::SetGain { gain: 0.5 },
                HostAction::Start,
            ],
        });
        bridge.send(HostCommand {
            host: "host".into(),
            revision: 2,
            actions: vec![
                HostAction::Remove { index: 1 },
                HostAction::Add {
                    ids: vec!["track-3".into()],
                },
            ],
        });
        bridge.send(HostCommand {
            host: "host".into(),
            revision: 3,
            actions: vec![HostAction::Stop, HostAction::Clear],
        });
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [
                r#"{"host":"host","revision":1,"actions":[{"action":"set","ids":["track-1","track-2"]},{"action":"skip","index":0,"offsetMs":1500},{"action":"setGain","gain":0.5},{"action":"start"}]}"#,
                r#"{"host":"host","revision":2,"actions":[{"action":"remove","index":1},{"action":"add","ids":["track-3"]}]}"#,
                r#"{"host":"host","revision":3,"actions":[{"action":"stop"},{"action":"clear"}]}"#,
            ]
        );
    }

    #[test]
    fn host_events_parse() {
        assert_eq!(
            parsed(&[
                r#"{"type":"hostReady","host":"host","authorized":false,"canPlay":false}"#,
                r#"{"type":"jukeboxStatus","host":"host","revision":4,"currentIndex":1,"currentId":"track-2","playing":true,"positionMs":2500.4}"#,
                r#"{"type":"jukeboxItemFailed","host":"host","revision":4,"index":1,"reason":"network"}"#,
                r#"{"type":"jukeboxEnded","host":"host","revision":5}"#,
            ]),
            [
                HostEvent::HostReady {
                    host: "host".into(),
                    authorized: false,
                    can_play: false,
                },
                HostEvent::JukeboxStatus {
                    host: "host".into(),
                    revision: 4,
                    current_index: Some(1),
                    current_id: Some("track-2".into()),
                    playing: true,
                    position_ms: 2500,
                },
                HostEvent::JukeboxItemFailed {
                    host: "host".into(),
                    revision: 4,
                    index: 1,
                    reason: HostFailure::Network,
                },
                HostEvent::JukeboxEnded {
                    host: "host".into(),
                    revision: 5,
                    index: None,
                },
            ]
        );
    }

    #[test]
    fn bad_and_unknown_events_are_dropped() {
        assert_eq!(
            parsed(&[
                "not json",
                "[1]",
                r#"{"type":"somethingNew","host":"host"}"#,
                r#"{"host":"host","revision":1}"#,
                r#"{"type":"jukeboxEnded","host":"host"}"#,
                r#"{"type":"jukeboxStatus","host":"host","revision":1,"currentIndex":0,"playing":"yes","positionMs":0}"#,
            ]),
            []
        );
    }
}
