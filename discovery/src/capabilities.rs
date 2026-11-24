use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Library browsing, media access, catalog sync and provider extensions.
    pub library_provider: Option<LibraryProviderCaps>,
    /// Playback sessions hosted by this endpoint.
    pub renderer: Option<RendererCaps>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryProviderCaps {
    /// Clients may browse metadata and issue provider queries.
    pub browse: bool,
    /// Clients may read media bytes as part of playback.
    pub stream: bool,
    /// Clients may retain media bytes for offline use.
    pub download: bool,
    /// Clients may replicate the provider's catalog.
    pub sync: bool,
    /// Optional provider features beyond the core operations.
    pub extensions: Vec<Extension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Extension {
    /// Realtime visualizer inputs for a remotely rendered session.
    VisualizerFeed,
    /// Per-track chart analysis data.
    Charts,
    /// Mix collection queries and data.
    Mixes,
    /// Recommendation queries and data.
    Recommendations,
    /// An extension added by a server version unknown to this client.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererCaps {
    /// Whether sessions may play any granted library or only this node's library.
    pub accepted_sources: AcceptedSources,
    /// Whether the renderer supports concurrent sessions or one node-wide session.
    pub sessions: SessionCardinality,
    /// Whether session status is pushed or must be polled.
    pub status_delivery: StatusDelivery,
    /// Commands accepted by the renderer.
    pub controls: Vec<Control>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AcceptedSources {
    /// Any library the renderer itself holds a standing read grant for.
    AnyGrantedLibrary,
    /// Only a library owned by the renderer.
    OwnLibraryOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionCardinality {
    /// The endpoint can host multiple independent sessions.
    Many,
    /// The endpoint has one session shared by all controllers.
    SingleGlobal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StatusDelivery {
    /// The renderer pushes status changes to attached controllers.
    Push,
    /// Controllers poll status at the given interval in milliseconds.
    Poll { interval_ms: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Control {
    /// Start or pause playback.
    PlayPause,
    /// Change the current playback position.
    Seek,
    /// Change output volume.
    Volume,
    /// Change output gain.
    Gain,
    /// Insert, remove or reorder queue items.
    QueueEdit,
    /// Load a collection or item onto the renderer.
    Load,
    /// A control added by a server version unknown to this client.
    #[serde(other)]
    Unknown,
}

impl Capabilities {
    /// Rune nodes browse, stream, download and sync their libraries. Mixes and
    /// recommendations are served to peers; charts and visualizer feeds are not.
    /// They render any library for which they hold a standing read grant, keep
    /// one node-wide session, and push status updates to attached controllers.
    /// Controllers can play or pause, seek, change volume, edit the queue, and
    /// load a cast of library items after the renderer verifies its standing grant.
    pub fn rune_native() -> Self {
        Self {
            library_provider: Some(LibraryProviderCaps {
                browse: true,
                stream: true,
                download: true,
                sync: true,
                extensions: vec![Extension::Mixes, Extension::Recommendations],
            }),
            renderer: Some(RendererCaps {
                accepted_sources: AcceptedSources::AnyGrantedLibrary,
                sessions: SessionCardinality::SingleGlobal,
                status_delivery: StatusDelivery::Push,
                controls: vec![
                    Control::PlayPause,
                    Control::Seek,
                    Control::Volume,
                    Control::QueueEdit,
                    Control::Load,
                ],
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AcceptedSources, Capabilities, Control, Extension, LibraryProviderCaps, RendererCaps,
        SessionCardinality, StatusDelivery,
    };
    use crate::utils::DeviceInfo;

    #[test]
    fn poll_interval_uses_camel_case_on_the_wire() {
        let json = serde_json::to_value(StatusDelivery::Poll { interval_ms: 500 }).unwrap();
        assert_eq!(json, serde_json::json!({ "poll": { "intervalMs": 500 } }));
    }

    #[test]
    fn full_capability_descriptor_round_trips_through_serde() {
        let capabilities = Capabilities {
            library_provider: Some(LibraryProviderCaps {
                browse: true,
                stream: true,
                download: true,
                sync: true,
                extensions: vec![Extension::Mixes, Extension::Recommendations],
            }),
            renderer: Some(RendererCaps {
                accepted_sources: AcceptedSources::AnyGrantedLibrary,
                sessions: SessionCardinality::SingleGlobal,
                status_delivery: StatusDelivery::Push,
                controls: vec![
                    Control::PlayPause,
                    Control::Seek,
                    Control::Volume,
                    Control::QueueEdit,
                    Control::Load,
                ],
            }),
        };

        let json = serde_json::to_string(&capabilities).unwrap();
        let expected = serde_json::json!({
            "libraryProvider": {
                "browse": true,
                "stream": true,
                "download": true,
                "sync": true,
                "extensions": ["mixes", "recommendations"]
            },
            "renderer": {
                "acceptedSources": "anyGrantedLibrary",
                "sessions": "singleGlobal",
                "statusDelivery": "push",
                "controls": ["playPause", "seek", "volume", "queueEdit", "load"]
            }
        });
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            expected
        );
        let decoded: Capabilities = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, capabilities);
    }

    #[test]
    fn unknown_extension_and_control_are_tolerated() {
        let json = r#"{
            "libraryProvider":{"browse":true,"stream":true,"download":true,"sync":true,"extensions":["futureExtension"]},
            "renderer":{"acceptedSources":"anyGrantedLibrary","sessions":"singleGlobal","statusDelivery":"push","controls":["futureControl"]}
        }"#;

        let decoded: Capabilities = serde_json::from_str(json).unwrap();

        assert_eq!(
            decoded.library_provider.unwrap().extensions,
            vec![Extension::Unknown]
        );
        assert_eq!(decoded.renderer.unwrap().controls, vec![Control::Unknown]);
    }

    #[test]
    fn old_device_info_without_capabilities_still_parses() {
        let json = r#"{
            "alias":"Rune","version":"1","deviceModel":null,"deviceType":"desktop",
            "fingerprint":"fingerprint","apiPort":7863,"protocol":"http"
        }"#;

        let decoded: DeviceInfo = serde_json::from_str(json).unwrap();

        assert!(decoded.capabilities.is_none());
        assert!(
            serde_json::to_value(decoded)
                .unwrap()
                .get("capabilities")
                .is_none()
        );
    }

    #[test]
    fn rune_native_reports_only_current_capabilities() {
        let capabilities = Capabilities::rune_native();

        let provider = capabilities.library_provider.unwrap();
        assert_eq!(
            provider.extensions,
            vec![Extension::Mixes, Extension::Recommendations]
        );
        let renderer = capabilities.renderer.unwrap();
        assert_eq!(
            renderer.accepted_sources,
            AcceptedSources::AnyGrantedLibrary
        );
        assert_eq!(renderer.sessions, SessionCardinality::SingleGlobal);
        assert_eq!(renderer.status_delivery, StatusDelivery::Push);
        assert!(renderer.controls.contains(&Control::Load));
    }
}
