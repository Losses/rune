use super::gui_listener_tests::start;
use anyhow::Context;
use crate::server::utils::device::RunMode;
use crate::Session;
use crate::macros::{
    RequestAccess, RequestScope, RequestSlots, capability_snapshot, request_scope,
};
use crate::messages::PlaybackMode as WirePlaybackMode;
use crate::messages::{
    ExcludedItems, CallerQueue,
    CapabilityReason, LoadPlay, CloseLibraryRequest, CloseLibraryResponse, CreatePlaylistRequest,
    FetchAllPlaylistsRequest, FetchAllPlaylistsResponse, GetPlaylistByIdRequest,
    LibraryContentChanged, LibraryInitializeMode, ListLocalLibrariesRequest, MixQuery,
    OpenLibraryRequest, PlaybackState, PlaybackStatus, PlaylistOperateMode,
    RegisterLibraryFailureKind, RegisterLibraryOutcome, RemoteConnectionLost,
    RemoteLibraryReconnected, RemoteLibraryReconnecting, RemoteLossKind, RequestCapabilities,
    RequestCapability, RequestFailedResponse, LibraryRecordKindSummary, LibraryRecordSummary,
    OpenDalServiceKind,
    SetLikedRequest,
};
use crate::messages::{
    OpenDalLibraryService, RegisterOpenDalLibraryRequest, RegisterSubsonicLibraryRequest,
    ShareDataChoice, SubsonicOpenModeMessage,
};
use crate::node::{NodeContext, ReconnectConfig};
use crate::request_owner::{CurrentPlayer, OpenLibrary};
use crate::test_wait::{TEST_DEADLINE, wait_until};
use crate::utils::{
    files_to_playback_request, library_item_metadata, open_library_by_record_on_node,
    playback_implementation_for_item, record_library_item_played,
    register_local_library_on_node, register_opendal_library_on_node,
    register_remote_control_on_node, register_subsonic_library_on_node,
    register_subsonic_library_with_node, replace_subsonic_credential_on_node,
    report_item_played_through,
};
use axum::Router;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{
    StatusCode,
    header::{ACCEPT_RANGES, CONTENT_RANGE, RANGE},
};
use axum::response::IntoResponse;
use axum::routing::{any, get};
use database::{
    library_registry::{
        CatalogPlacement, LibraryRecordId, LibraryRegistry, ProtocolAdapter, SubsonicOpenMode,
    },
    sync::model::RuneModel,
};
use fsio::test_fixtures::WebDavFixture;
use fsio::{OpenDalConfig, OpenDalObjectStore};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc as std_mpsc,
    },
};
use anyhow::Error;
use axum::http as axum_http;
use discovery::protocol_version;
use futures::future;
use playback::player;
use playback::player::PausedPlayback;
use reqwest::StatusCode as ReqwestStatusCode;
use reqwest::header;
use std::collections;
use std::slice;
use tokio::task;
use tokio::time as tokio_time;
use visualizer::Chart;
use visualizer::Visualizer;
#[cfg(target_os = "linux")]
use std::{env, process::Command};
use sync::{
    protocol::SyncModel,
    share::{FORMAT_VERSION, LibraryFile, ObjectStore, encode_library},
};
use tempfile::TempDir;
use visualizer::VisualizerInputs;
use url::Url;
use uuid::Uuid;

use crate::{
    Signal,
    access_policy::{ConnectionLevel, DeviceGrants, SyncLevel},
    backends::remote::{RendererLinkUse, open_remote_library_for_fingerprint},
    handlers::playback::{
        playing_item_from_request, playing_item_requests, session_snapshot_from_status,
    },
    messages::{
        CastLibraryState, CastPreflightRequest, CastResolvedItemsRequest, CastTarget,
        GetLikedRequest, GetLyricByTrackIdRequest, GetMediaFilesCountRequest,
        GetPrimaryColorByTrackIdRequest, ListRenderersRequest,
        LoadResolvedItemsRequest, LyricContentLine, MediaFile, PlayingItemRef, PlaylistRevisionRequest,
        PlaylistUpdate, RendererAvailability, RendererCandidate, RequestErrorKind,
        RequestFailureCause, ResolveMixQueryRequest, ResolveMixQueryResponse,
        SessionSnapshotRequest, SwitchRequest, VolumeRequest,
    },
    request_error::{ExpectedRequestError, classify},
    request_owner::Implementation,
    rpc::{Slot, UnsupportedReason, dispatch_gui_request},
    server::api::fetch_device_info_at_port,
    session::store::{SessionState, StoredInterruptedItem},
    utils::{GlobalParams, service_binding_for_library},
};
use database::actions::{
    downloads::cleanup_unselected_downloads,
    file::{catalog_track, file_ids_by_uuid, file_uuids_by_id},
    logging::list_log,
    mixes::query_mix_media_files,
};
use database::credentials::{Credential, CredentialRef, Secret};
use database::entities::media_cover_art;
use database::entities::media_file_stats;
use database::entities::media_files;
use database::playing_item::MediaFileHandle;
use discovery::access_policy::PeerIdentity;
use migration::catalog_identity::artwork;
use playback::{
    PlaybackMode,
    player::{
        AudioPlaybackNote, AudioSourceMode, LibraryItem, MockPlayer, Playable, PlaybackSource,
        PlaybackState as PlayerPlaybackState, PlayedThrough, PlayerStatus, PlayingItem, QueueEntry,
        RemoteAudioError, ServiceItem, UnavailableReason,
    },
    strategies::AddMode,
};
use rust_decimal::prelude::ToPrimitive;
use scrobbling::manager::ScrobblingManager;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy as copy_io};
use tokio::net::TcpListener as TokioTcpListener;
#[cfg(target_os = "linux")]
use tokio::runtime::Builder;
use tokio::sync::{Mutex, Notify, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};


#[derive(Clone)]
struct SubsonicHubMock {
    calls: Arc<StdMutex<Vec<String>>>,
    stream_ranges: Arc<StdMutex<Vec<Option<String>>>>,
    reject_stream: Arc<AtomicBool>,
    supports_ranges: Arc<AtomicBool>,
    reject_credentials: bool,
    visualizer_covers: bool,
    starred: Arc<AtomicBool>,
}

async fn subsonic_hub_mock(
    reject_credentials: bool,
) -> (
    String,
    Arc<StdMutex<Vec<String>>>,
    Arc<StdMutex<Vec<Option<String>>>>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    JoinHandle<()>,
) {
    subsonic_hub_mock_with_covers(reject_credentials, false).await
}

async fn subsonic_hub_mock_with_covers(
    reject_credentials: bool,
    visualizer_covers: bool,
) -> (
    String,
    Arc<StdMutex<Vec<String>>>,
    Arc<StdMutex<Vec<Option<String>>>>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    JoinHandle<()>,
) {
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let stream_ranges = Arc::new(StdMutex::new(Vec::new()));
    let reject_stream = Arc::new(AtomicBool::new(false));
    let supports_ranges = Arc::new(AtomicBool::new(true));
    let state = SubsonicHubMock {
        calls: calls.clone(),
        stream_ranges: stream_ranges.clone(),
        reject_stream: reject_stream.clone(),
        supports_ranges: supports_ranges.clone(),
        reject_credentials,
        visualizer_covers,
        starred: Arc::new(AtomicBool::new(false)),
    };
    let app = Router::new()
        .route("/rest/{*action}", get(subsonic_hub_response))
        .with_state(state);
    let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        format!("http://{address}"),
        calls,
        stream_ranges,
        reject_stream,
        supports_ranges,
        task,
    )
}

/// A 2 by 2 bitmap of one color.
fn red_bitmap() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend(70_u32.to_le_bytes());
    bytes.extend([0; 4]);
    bytes.extend(54_u32.to_le_bytes());
    bytes.extend(40_u32.to_le_bytes());
    bytes.extend(2_i32.to_le_bytes());
    bytes.extend(2_i32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(24_u16.to_le_bytes());
    bytes.extend([0; 24]);
    for _ in 0..2 {
        // Two pixels in blue, green, red order, and the row padded to four bytes.
        bytes.extend([30, 30, 200, 30, 30, 200, 0, 0]);
    }
    bytes
}

async fn unreachable_http_endpoint() -> Result<(String, JoinHandle<()>)> {
    let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    Ok((format!("http://{address}"), task))
}

async fn subsonic_hub_response(
    AxumPath(action): AxumPath<String>,
    State(state): State<SubsonicHubMock>,
    Query(query): Query<HashMap<String, String>>,
    headers: axum_http::HeaderMap,
) -> impl IntoResponse {
    let action = action.strip_suffix(".view").unwrap_or(&action).to_owned();
    let call = match action.as_str() {
        // A now-playing report and a completed play differ only in their parameters.
        "scrobble" => format!(
            "scrobble id={} submission={}",
            query.get("id").map(String::as_str).unwrap_or_default(),
            query.get("submission").map(String::as_str).unwrap_or_default(),
        ),
        "star" | "unstar" => format!(
            "{action} id={}",
            query.get("id").map(String::as_str).unwrap_or_default()
        ),
        _ => action.clone(),
    };
    state.calls.lock().unwrap().push(call);
    if action == "stream" {
        let range = headers
            .get(RANGE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        state.stream_ranges.lock().unwrap().push(range.clone());
        if state.reject_stream.load(Ordering::Acquire) {
            return axum::Json(json!({ "subsonic-response": { "status": "failed", "error": { "code": 40, "message": format!("rejected t={} s={} p=test-secret", query.get("t").map(String::as_str).unwrap_or_default(), query.get("s").map(String::as_str).unwrap_or_default()) } } })).into_response();
        }
        let bytes = include_bytes!("../../../../assets/startup_0.ogg");
        if let Some(range) = range
            .as_deref()
            .and_then(|value| value.strip_prefix("bytes="))
        {
            let (start, end) = range.split_once('-').unwrap_or(("0", ""));
            let start = start.parse::<usize>().unwrap_or_default().min(bytes.len());
            let end = end
                .parse::<usize>()
                .unwrap_or(bytes.len().saturating_sub(1))
                .min(bytes.len().saturating_sub(1));
            let body = if start <= end {
                &bytes[start..=end]
            } else {
                &bytes[0..0]
            };
            return (
                StatusCode::PARTIAL_CONTENT,
                [
                    (ACCEPT_RANGES, "bytes"),
                    (
                        CONTENT_RANGE,
                        &format!("bytes {start}-{end}/{}", bytes.len()),
                    ),
                ],
                body.to_vec(),
            )
                .into_response();
        }
        return if state.supports_ranges.load(Ordering::Acquire) {
            (StatusCode::OK, [(ACCEPT_RANGES, "bytes")], bytes.to_vec()).into_response()
        } else {
            (StatusCode::OK, bytes.to_vec()).into_response()
        };
    }
    if action == "getCoverArt" {
        // One cover is a real image, whose color the hub can read.
        if query.get("id").map(String::as_str) == Some("cover-color") {
            return (StatusCode::OK, red_bitmap()).into_response();
        }
        return (StatusCode::OK, b"mock-cover-bytes".to_vec()).into_response();
    }
    let body = match action.as_str() {
        "ping" => {
            json!({ "subsonic-response": { "status": "ok", "version": "1.16.1", "openSubsonic": false } })
        }
        "getUser" if state.reject_credentials => {
            json!({ "subsonic-response": { "status": "failed", "error": { "code": 40, "message": format!("rejected t={} s={} p=test-secret", query.get("t").map(String::as_str).unwrap_or_default(), query.get("s").map(String::as_str).unwrap_or_default()) } } })
        }
        "getUser" => {
            json!({ "subsonic-response": { "status": "ok", "user": { "streamRole": true, "downloadRole": true } } })
        }
        "search3" => {
            let song_offset = query
                .get("songOffset")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();
            let songs = if song_offset == 0 {
                json!({ "id": "song-opaque", "title": "Song title", "artist": "Artist", "album": "Album", "artistId": "artist-opaque", "albumId": "album-opaque", "duration": 90, "track": 4 })
            } else {
                json!([])
            };
            json!({ "subsonic-response": { "status": "ok", "searchResult3": { "song": songs, "artist": { "id": "artist-opaque", "name": "Artist" }, "album": { "id": "album-opaque", "name": "Album", "artist": "Artist", "artistId": "artist-opaque" } } } })
        }
        "getArtists" => {
            json!({ "subsonic-response": { "status": "ok", "artists": { "index": [ { "name": "A", "artist": { "id": "artist-opaque", "name": "Artist", "albumCount": 1 } }, { "name": "B", "artist": { "id": "artist-beta", "name": "Beta" } } ] } } })
        }
        "getAlbumList2" => {
            let offset = query
                .get("offset")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();
            let albums: Vec<_> = if offset == 0 {
                (0..500)
                    .map(|index| {
                        let id = if index == 0 {
                            "album-opaque".to_owned()
                        } else {
                            format!("album-list-{index}")
                        };
                        let name = if index == 0 {
                            "Album".to_owned()
                        } else {
                            format!("Album {index:03}")
                        };
                        json!({ "id": id, "name": name, "artist": "Artist", "artistId": "artist-opaque", "coverArt": state.visualizer_covers.then(|| if index == 0 { "cover-playing".to_owned() } else { format!("cover-wall-{index}") }) })
                    })
                    .collect()
            } else {
                vec![
                    json!({ "id": "album-list-500", "name": "Album 500", "artist": "Artist", "artistId": "artist-opaque" }),
                ]
            };
            json!({ "subsonic-response": { "status": "ok", "albumList2": { "album": albums } } })
        }
        "getSong" if query.get("id").map(String::as_str) == Some("song-signed-out") => {
            // The server no longer takes the library's sign-in by the time this song is asked for.
            json!({ "subsonic-response": { "status": "failed", "error": { "code": 40, "message": "Wrong username or password" } } })
        }
        "getSong" => {
            // The album's later disc shares its cover; every other id is the searched song.
            let (id, title) = match query.get("id").map(String::as_str) {
                Some("song-later") => ("song-later", "Later disc"),
                Some("song-color") => ("song-color", "Colored"),
                _ => ("song-opaque", "Song title"),
            };
            let cover = if id == "song-color" {
                Some("cover-color".to_owned())
            } else {
                state.visualizer_covers.then(|| "cover-playing".to_owned())
            };
            let starred = state
                .starred
                .load(Ordering::SeqCst)
                .then_some("2026-01-01T00:00:00Z");
            json!({ "subsonic-response": { "status": "ok", "song": { "id": id, "title": title, "artist": "Artist", "album": "Album", "artistId": "artist-opaque", "albumId": "album-opaque", "coverArt": cover, "starred": starred, "duration": 90, "track": 4 } } })
        }
        "star" | "unstar" => {
            state.starred.store(action == "star", Ordering::SeqCst);
            json!({ "subsonic-response": { "status": "ok" } })
        }
        "getLyrics" => {
            json!({ "subsonic-response": { "status": "ok", "lyrics": { "artist": "Artist", "title": "Song title", "value": "[00:01.00]First line\n[00:02.00]Second line" } } })
        }
        "scrobble" => json!({ "subsonic-response": { "status": "ok" } }),
        "getArtist" => {
            json!({ "subsonic-response": { "status": "ok", "artist": { "id": "artist-opaque", "name": "Artist", "albumCount": 1, "album": { "id": "album-opaque", "name": "Album", "artist": "Artist", "artistId": "artist-opaque" } } } })
        }
        "getPlaylists" => {
            json!({ "subsonic-response": { "status": "ok", "playlists": { "playlist": { "id": "playlist-opaque", "name": "Road" } } } })
        }
        "getPlaylist" => {
            let entry = |id: &str, title: &str| json!({ "id": id, "title": title, "artist": "Artist", "album": "Album", "duration": 90 });
            json!({ "subsonic-response": { "status": "ok", "playlist": { "id": "playlist-opaque", "name": "Road", "entry": [ entry("song-a", "A"), entry("song-b", "B"), entry("song-a", "A") ] } } })
        }
        "getAlbum" => {
            json!({ "subsonic-response": { "status": "ok", "album": { "id": "album-opaque", "name": "Album", "artist": "Artist", "artistId": "artist-opaque", "song": [ { "id": "song-later", "title": "Later disc", "artist": "Artist", "album": "Album", "discNumber": 2, "track": 1, "duration": 90 }, { "id": "song-first", "title": "First disc", "artist": "Artist", "album": "Album", "discNumber": 1, "track": 8, "duration": 100 } ] } } })
        }
        _ => {
            json!({ "subsonic-response": { "status": "failed", "error": { "code": 70, "message": "unexpected action" } } })
        }
    };
    axum::Json(body).into_response()
}

#[derive(Default)]
struct ReloginServer {
    rotated: AtomicBool,
    hold_next_user: AtomicBool,
    user_received: Notify,
    release_user: Notify,
    /// Gives the account the jukebox role whatever its password.
    jukebox: AtomicBool,
    /// How many times the server gave the jukebox queue, which a poll reads once the status it
    /// read first was decoded.
    jukebox_reads: AtomicUsize,
}

async fn subsonic_relogin_mock() -> (String, Arc<ReloginServer>, JoinHandle<()>) {
    let state = Arc::new(ReloginServer::default());
    let app = Router::new()
        .route("/rest/{*action}", get(subsonic_relogin_response))
        .with_state(Arc::clone(&state));
    let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), state, task)
}

async fn register_relogin_record(
    node: &Arc<NodeContext>,
    endpoint: String,
) -> Result<(LibraryRecordId, CredentialRef)> {
    let record = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "old-password",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        node,
    )
    .await?
    .record
    .context("registered Subsonic record")?;
    let id = LibraryRecordId(Uuid::parse_str(&record.record_id)?);
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let ProtocolAdapter::SubsonicTranslate { credential, .. } = registry.record(id)?.protocol else {
        anyhow::bail!("expected a Subsonic record");
    };
    Ok((id, credential))
}

async fn subsonic_relogin_response(
    AxumPath(action): AxumPath<String>,
    State(state): State<Arc<ReloginServer>>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let action = action.strip_suffix(".view").unwrap_or(&action);
    let rotated = state.rotated.load(Ordering::SeqCst);
    let password = if rotated {
        "new-password"
    } else {
        "old-password"
    };
    let authorized = query.get("u").is_some_and(|user| user == "alice")
        && query
            .get("s")
            .zip(query.get("t"))
            .is_some_and(|(salt, token)| {
                *token == format!("{:x}", md5::compute(format!("{password}{salt}")))
            });
    let body = if !authorized {
        json!({"subsonic-response":{"status":"failed","error":{"code":40,"message":"Rejected"}}})
    } else if action == "getUser" {
        if state.hold_next_user.swap(false, Ordering::SeqCst) {
            state.user_received.notify_one();
            state.release_user.notified().await;
        }
        let jukebox = rotated || state.jukebox.load(Ordering::SeqCst);
        json!({"subsonic-response":{"status":"ok","user":{"streamRole":true,"jukeboxRole":jukebox}}})
    } else if action == "getPlaylists" {
        json!({"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"p","name":"Road"}]}}})
    } else if action == "jukeboxControl" {
        let jukebox = json!({"currentIndex":0,"playing":false,"gain":1.0,"position":0});
        if query.get("action").is_some_and(|action| action == "get") {
            state.jukebox_reads.fetch_add(1, Ordering::SeqCst);
            let mut playlist = jukebox;
            playlist["entry"] = json!([{"id":"track-1","title":"Song","duration":10}]);
            json!({"subsonic-response":{"status":"ok","jukeboxPlaylist":playlist}})
        } else {
            json!({"subsonic-response":{"status":"ok","jukeboxStatus":jukebox}})
        }
    } else if action == "getArtists" {
        json!({"subsonic-response":{"status":"ok","artists":{"index":[{"name":"A","artist":[{"id":"artist-1","name":"Artist One"}]}]}}})
    } else {
        json!({"subsonic-response":{"status":"ok","version":"1.16.1","openSubsonic":false}})
    };
    (StatusCode::OK, axum::Json(body))
}

struct MixPlaybackTestRequest {
    queries: Vec<MixQuery>,
    playback_mode: WirePlaybackMode,
    hint_position: i32,
    initial_playback_item: Option<PlayingItemRef>,
    instantly_play: bool,
    operate_mode: PlaylistOperateMode,
    fallback_playing_items: Vec<PlayingItemRef>,
    expected_revision: Option<u64>,
    cast_to: Option<CastTarget>,
}

struct MixPlaybackTestResponse {
    playing_items: Vec<PlayingItemRef>,
}

impl MixPlaybackTestRequest {
    fn extract_params(&self, params: &GlobalParams) -> GlobalParams {
        params.clone()
    }

    async fn handle(
        &self,
        params: GlobalParams,
        session: Option<Session>,
        _request: &Self,
    ) -> Result<Option<MixPlaybackTestResponse>> {
        let library = params.library.clone();
        let node = Arc::clone(&params.node);
        let excluded_items = if node.renderer_link()?.is_none() && self.cast_to.is_none() {
            dispatch_gui_request(&params, &SessionSnapshotRequest { call_id: None })
                .await?
                .context("current player snapshot")?
                .items
        } else {
            Vec::new()
        };
        let resolve_request = ResolveMixQueryRequest {
            call_id: None,
            queries: self.queries.clone(),
            hint_position: self.hint_position,
            initial_playback_item: self.initial_playback_item.clone(),
            instantly_play: self.instantly_play,
            fallback_playing_items: self.fallback_playing_items.clone(),
            excluded: ExcludedItems {
                items: excluded_items,
            },
        };
        let resolved = resolve_request
            .handle(resolve_request.extract_params(&params), session, &resolve_request)
            .await?
            .context("mix query resolution")?;
        let load = LoadResolvedItemsRequest {
            call_id: None,
            position_seconds: None,
            play: LoadPlay::Auto,
            items: resolved.playing_items.clone(),
            start_index: resolved.start_index,
            mode: self.operate_mode,
            expected_revision: self.expected_revision,
            playback_mode: self.instantly_play.then_some(self.playback_mode),
        };
        match &self.cast_to {
            Some(target) if !target.attach => {
                let cast = CastResolvedItemsRequest {
                    fingerprint: target.fingerprint.clone(),
                    load: load.clone(),
                    call_id: None,
                };
                cast.handle(node.clone(), None, &cast).await?;
            }
            Some(target) => {
                node.attach_renderer(&target.fingerprint).await?;
                node.renderer_link()?
                    .context("attached renderer")?
                    .call(&load)
                    .await?;
            }
            None => match node.renderer_link()? {
                Some(link) => {
                    link.call(&load).await?;
                }
                None => {
                    load.handle(
                        (Some(library.context("library is not open")?), node.clone()),
                        None,
                        &load,
                    )
                    .await?;
                }
            },
        }
        Ok(Some(MixPlaybackTestResponse {
            playing_items: resolved.playing_items,
        }))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_open_search_fetch_and_unsupported_requests() -> Result<()> {
    use crate::messages::{
        CollectionType, FetchAllPlaylistsRequest, FetchCollectionByIdsRequest,
        FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
        FetchMediaFileByIdsRequest, LoadResolvedItemsRequest, MixQueryRequest,
        OpenLibraryFailureKind, OpenLibraryRequest, SearchForRequest, SetDownloadRuleRequest,
    };
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRecordId, LibraryRegistry, ProtocolAdapter,
        SubsonicOpenMode,
    };
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let rune_row_id = session_audio(&fixture, "rune-in-subsonic-queue.ogg").await?;
    let rune_item = library_track_for_id(&fixture, rune_row_id).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, stream_ranges, reject_stream, supports_ranges, server) =
        subsonic_hub_mock(false).await;
    let credential = node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"test-secret".to_vec()),
        })
        .await?;
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Subsonic search".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint: endpoint.clone(),
            credential,
            mode: SubsonicOpenMode::PlayOnThisDevice,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(record.clone());
        Ok(())
    })
    .await?;
    let scrobbler = Arc::new(Mutex::new(ScrobblingManager::new(
        10,
        Duration::from_secs(5),
    )));
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(1),
            record_id: record.record_id.0.to_string(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await?;
    assert!(matches!(
        node.current_library(),
        Ok(OpenLibrary::Service(_))
    ));
    assert!(matches!(node.current_player()?, CurrentPlayer::Local));
    let capabilities = capability_snapshot(&node)?;
    assert_eq!(
        capabilities.search_for_request,
        RequestCapability::Available
    );
    assert_eq!(
        capabilities.fetch_collection_group_summary_request,
        RequestCapability::Available
    );
    assert_eq!(
        capabilities.fetch_collection_groups_request,
        RequestCapability::Available
    );
    assert_eq!(capabilities.mix_query_request, RequestCapability::Available);
    assert_eq!(
        capabilities.fetch_all_playlists_request,
        RequestCapability::Available
    );
    assert_eq!(capabilities.play_request, RequestCapability::Available);
    // The offline selection is kept in a library on this device, which a Subsonic one is not.
    assert_eq!(
        capabilities.set_download_rule_request,
        RequestCapability::Hidden
    );
    assert_eq!(
        capabilities.remove_download_rule_request,
        RequestCapability::Hidden
    );
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    assert_eq!(registry.last_open_record, Some(record.record_id));

    let search = dispatch_gui_request(
        &fixture.global_params,
        &SearchForRequest {
            call_id: Some(7),
            query_str: "Song".into(),
            fields: Vec::new(),
            n: 10,
        },
    )
    .await?
    .context("Subsonic search response")?;
    assert_eq!(search.tracks, vec![1]);
    assert_eq!(search.artists, vec![1]);
    assert_eq!(search.albums, vec![1]);
    let tracks = dispatch_gui_request(
        &fixture.global_params,
        &FetchMediaFileByIdsRequest {
            call_id: None,
            ids: search.tracks.clone(),
            bake_cover_arts: true,
        },
    )
    .await?
    .context("Subsonic track fetch")?;
    assert_eq!(tracks.media_files[0].title, "Song title");
    assert_eq!(tracks.media_files[0].artist, "Artist");
    assert_eq!(tracks.media_files[0].album, "Album");
    let artists = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: true,
            ids: search.artists.clone(),
        },
    )
    .await?
    .context("Subsonic artist fetch")?;
    assert_eq!(artists.result[0].name, "Artist");
    let albums = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Album,
            bake_cover_arts: true,
            ids: search.albums.clone(),
        },
    )
    .await?
    .context("Subsonic album fetch")?;
    assert_eq!(albums.result[0].name, "Album");
    let artist_groups = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        },
    )
    .await?
    .context("Subsonic artist summary")?;
    assert_eq!(artist_groups.groups[0].group_title, "A");
    assert_eq!(artist_groups.groups[0].count, 1);
    let artist_page = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: true,
            group_titles: vec!["A".into()],
        },
    )
    .await?
    .context("Subsonic artist page")?;
    assert_eq!(artist_page.groups[0].collections[0].name, "Artist");
    let album_groups = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Album,
        },
    )
    .await?
    .context("Subsonic album summary")?;
    assert_eq!(album_groups.groups[0].group_title, "A");
    assert_eq!(album_groups.groups[0].count, 501);
    let album_page = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Album,
            bake_cover_arts: false,
            group_titles: vec!["A".into()],
        },
    )
    .await?
    .context("Subsonic album page")?;
    assert_eq!(album_page.groups[0].collections.len(), 501);
    assert_eq!(album_page.groups[0].collections[0].name, "Album");
    assert_eq!(album_page.groups[0].collections[500].name, "Album 500");
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|action| action.as_str() == "getAlbumList2")
            .count(),
        2
    );
    let artist_tracks = dispatch_gui_request(
        &fixture.global_params,
        &MixQueryRequest {
            call_id: None,
            queries: vec![MixQuery {
                operator: "lib::artist".into(),
                parameter: search.artists[0].to_string(),
            }],
            cursor: 0,
            page_size: 30,
            bake_cover_arts: true,
            queue: CallerQueue::default(),
            random_seed: None,
        },
    )
    .await?
    .context("Subsonic artist tracks")?;
    assert_eq!(artist_tracks.files.len(), 2);
    assert_eq!(artist_tracks.files[0].title, "First disc");
    assert_eq!(artist_tracks.files[1].title, "Later disc");
    let album_tracks = dispatch_gui_request(
        &fixture.global_params,
        &MixQueryRequest {
            call_id: None,
            queries: vec![MixQuery {
                operator: "lib::album".into(),
                parameter: search.albums[0].to_string(),
            }],
            cursor: 0,
            page_size: 30,
            bake_cover_arts: true,
            queue: CallerQueue::default(),
            random_seed: None,
        },
    )
    .await?
    .context("Subsonic album tracks")?;
    assert_eq!(album_tracks.files[0].title, "First disc");
    let calls_before_cached = calls.lock().unwrap().len();
    let _cached = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: false,
            ids: vec![search.artists[0]],
        },
    )
    .await?
    .context("cached Subsonic artist")?;
    assert_eq!(calls.lock().unwrap().len(), calls_before_cached);
    assert!(calls.lock().unwrap().contains(&"getArtist".to_owned()));
    assert!(calls.lock().unwrap().contains(&"getAlbum".to_owned()));

    let calls_before_unsupported = calls.lock().unwrap().len();
    let unsupported_query = dispatch_gui_request(
        &fixture.global_params,
        &MixQueryRequest {
            call_id: None,
            queries: vec![MixQuery {
                operator: "filter::liked".into(),
                parameter: "true".into(),
            }],
            cursor: 0,
            page_size: 30,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        },
    )
    .await
    .err()
    .context("expected unsupported Subsonic mix query")?;
    assert!(matches!(
        unsupported_query.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported { .. })
    ));
    let unsupported_collection = dispatch_gui_request(
        &fixture.global_params,
        &FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Mix,
            bake_cover_arts: false,
            group_titles: vec!["A".into()],
        },
    )
    .await
    .err()
    .context("expected unsupported collection type")?;
    assert!(matches!(
        unsupported_collection.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported { .. })
    ));
    let unsupported_offline = dispatch_gui_request(
        &fixture.global_params,
        &SetDownloadRuleRequest {
            request_id: "rule".into(),
            queries: vec![MixQuery {
                operator: "lib::album".into(),
                parameter: search.albums[0].to_string(),
            }],
            mix_id: None,
            enabled: true,
            delete_unselected_files: false,
            call_id: None,
        },
    )
    .await
    .err()
    .context("expected unsupported offline selection")?;
    assert!(matches!(
        unsupported_offline.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported { .. })
    ));
    let playlists = dispatch_gui_request(
        &fixture.global_params,
        &FetchAllPlaylistsRequest { call_id: None },
    )
    .await?
    .context("Subsonic playlist list response")?;
    assert_eq!(playlists.playlists.len(), 1);
    assert_eq!(playlists.playlists[0].name, "Road");
    assert_eq!(calls.lock().unwrap().len(), calls_before_unsupported + 1);
    assert_eq!(
        calls.lock().unwrap().last().map(String::as_str),
        Some("getPlaylists")
    );
    let item = PlayingItem::LibraryTrack {
        library_id: record.record_id.0,
        track: "song-opaque".into(),
    };
    let load = LoadResolvedItemsRequest {
        items: vec![
            PlayingItemRef::LibraryTrack {
                library_id: record.record_id.0.to_string(),
                track: "song-opaque".into(),
            },
            rune_item.into(),
        ],
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: Some(WirePlaybackMode::Sequential),
        position_seconds: Some(0.0),
        play: LoadPlay::Play,
        call_id: None,
    };
    dispatch_gui_request(&fixture.global_params, &load).await?;
    let playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing
            && matches!(status.item, Some(PlayingItem::LibraryTrack { .. }))
    })
    .await?;
    assert!(
        matches!(
            playing.item.as_ref(),
            Some(PlayingItem::LibraryTrack { library_id: item_library, track })
                if *item_library == record.record_id.0 && track == "song-opaque"
        ),
        "unexpected Subsonic library identity: {:?}",
        playing.item
    );
    fixture.global_params.player.lock().await.seek(1.0);
    wait_player(&fixture, |status| {
        status.position.as_secs_f64() >= 0.8
            && status.item.as_ref().is_some_and(|playing| {
                matches!(playing, PlayingItem::LibraryTrack { track, .. } if track == "song-opaque")
            })
    })
    .await?;
    peer_step_with_timeout(
        "Subsonic server receives a stream request",
        TEST_DEADLINE,
        async {
            wait_until("the Subsonic stream request", || async {
                Ok((!stream_ranges.lock().unwrap().is_empty()).then_some(()))
            })
            .await
        },
    )
    .await?;
    fixture.global_params.player.lock().await.switch(1);
    wait_player(&fixture, |status| {
        status.item == Some(PlayingItem::from(rune_item)) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    fixture.global_params.player.lock().await.switch(0);
    wait_player(&fixture, |status| {
        status.item == Some(item.clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    let session_store = Path::new(&*node.config_path).join("sessions/local.json");
    peer_step_with_timeout(
        "player session persists the Subsonic queue identity",
        TEST_DEADLINE,
        async {
            wait_until("the persisted Subsonic queue identity", || async {
                Ok(fs::read_to_string(&session_store)
                    .is_ok_and(|contents| contents.contains("song-opaque"))
                    .then_some(()))
            })
            .await
        },
    )
    .await?;
    let stored = fs::read_to_string(session_store)?;
    assert!(stored.contains("song-opaque"));
    for secret_marker in ["t=", "s=", "p=", "test-secret"] {
        assert!(!stored.contains(secret_marker));
        assert!(!format!("{playing:?}").contains(secret_marker));
    }
    let dart_message = serde_json::to_string(
        &playing_item_requests(slice::from_ref(&item), &node.library_pool, None).await,
    )?;
    assert!(dart_message.contains("song-opaque"));
    for secret_marker in ["t=", "s=", "p=", "test-secret"] {
        assert!(!dart_message.contains(secret_marker));
    }
    let restored: SessionState = serde_json::from_str(&stored)?;
    assert_eq!(restored.queue_entries()[0].item, item);
    assert_eq!(
        restored.queue_entries()[0].item,
        PlayingItem::from(ServiceItem {
            library_id: record.record_id.0,
            track: "song-opaque".to_owned(),
        })
    );
    fixture.global_params.player.lock().await.stop();
    let binding = service_binding_for_library(&node, record.record_id.0).await?;
    let player::ServiceTarget::Stream(source) = binding.playback_source("song-opaque").await?
    else {
        anyhow::bail!("a Subsonic track must play from a stream");
    };
    let ranged = source.request.get(Some((12, Some(23)))).await?;
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    assert!(stream_ranges.lock().unwrap().iter().any(Option::is_some));
    wait_player(&fixture, |status| status.state == PlayerPlaybackState::Stopped).await?;
    supports_ranges.store(false, Ordering::Release);
    assert!(matches!(
        source.request.prepare().await?,
        AudioSourceMode::Buffered {
            bytes,
            note: AudioPlaybackNote::RangeUnsupportedBuffered,
        } if !bytes.is_empty()
    ));
    supports_ranges.store(true, Ordering::Release);
    reject_stream.store(true, Ordering::Release);
    let rejected = source.request.get(Some((12, Some(23)))).await.unwrap_err();
    assert!(matches!(
        rejected.downcast_ref::<RemoteAudioError>(),
        Some(RemoteAudioError::CredentialsRejected)
    ));
    for secret_marker in ["t=", "s=", "p=", "test-secret"] {
        assert!(!format!("{rejected:#}").contains(secret_marker));
    }
    let unavailable_load = dispatch_gui_request(&fixture.global_params, &load)
        .await
        .err()
        .context("a rejected source must fail the load request")?;
    assert!(unavailable_load.chain().any(|cause| {
        cause
            .downcast_ref::<ExpectedRequestError>()
            .is_some_and(|error| matches!(error, ExpectedRequestError::PlayerSourceUnavailable))
    }));
    wait_player(&fixture, |status| {
        status.unavailable_reasons.get(&item) == Some(&UnavailableReason::CredentialsRejected)
    })
    .await?;
    let calls_before_unknown = calls.lock().unwrap().len();
    let unknown = dispatch_gui_request(
        &fixture.global_params,
        &FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![999],
            bake_cover_arts: false,
        },
    )
    .await
    .err()
    .context("expected unknown mapped ID failure")?;
    assert!(matches!(
        unknown.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::InvalidInput { .. })
    ));
    assert_eq!(calls.lock().unwrap().len(), calls_before_unknown);

    let (bad_endpoint, bad_calls, _, _, _, bad_server) = subsonic_hub_mock(true).await;
    let bad_credential = node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"test-secret".to_vec()),
        })
        .await?;
    let bad_record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Rejected Subsonic".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint: bad_endpoint,
            credential: bad_credential,
            mode: SubsonicOpenMode::RemoteControl,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(bad_record.clone());
        Ok(())
    })
    .await?;
    let rejected = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(2),
            record_id: bad_record.record_id.0.to_string(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await;
    let rejected = match rejected {
        Ok(_) => anyhow::bail!("a rejected Subsonic account must not open"),
        Err(error) => error,
    };
    let detail = format!("{rejected:#}");
    assert!(!detail.contains("t=") && !detail.contains("p=") && !detail.contains("s="));
    assert_eq!(
        opened_failure_kind(Err(rejected))?,
        OpenLibraryFailureKind::RemoteNotAuthorized
    );
    assert_eq!(*bad_calls.lock().unwrap(), ["ping", "getUser"]);

    let (closed_endpoint, unreachable_server) = unreachable_http_endpoint().await?;
    let closed_credential = node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"test-secret".to_vec()),
        })
        .await?;
    let closed_record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Closed Subsonic".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint: closed_endpoint,
            credential: closed_credential,
            mode: SubsonicOpenMode::PlayOnThisDevice,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(closed_record.clone());
        Ok(())
    })
    .await?;
    let unreachable = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(3),
            record_id: closed_record.record_id.0.to_string(),
        },
        node.clone(),
        scrobbler,
    )
    .await;
    assert_eq!(opened_failure_kind(unreachable)?, OpenLibraryFailureKind::RemoteUnreachable);

    reject_stream.store(false, Ordering::Release);
    let fixture = reopen_fixture(&fixture).await?;
    let node = fixture.global_params.node.clone();
    wait_player(&fixture, |status| {
        status.playlist.iter().any(|entry| entry.item == item)
            && status
                .playlist
                .iter()
                .any(|entry| entry.item == PlayingItem::from(rune_item))
    })
    .await?;
    fixture.global_params.player.lock().await.play();
    let restored_playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing
            && matches!(status.item, Some(PlayingItem::LibraryTrack { .. }))
    })
    .await?;
    assert!(matches!(
        restored_playing.item,
        Some(PlayingItem::LibraryTrack { .. })
    ));

    unreachable_server.abort();
    server.abort();
    bad_server.abort();
    node.close_ui(&fixture.global_params.main_token).await;
    node.shutdown().await?;
    Ok(())
}

/// What a track list does when one of its tracks is clicked: resolve the list around the
/// track, leaving out the current queue, read the current player's queue revision and
/// replace the queue with the result.
async fn play_clicked_track(
    params: &GlobalParams,
    queries: Vec<MixQuery>,
    hint_position: i32,
    track: PlayingItemRef,
    queue: Vec<PlayingItemRef>,
) -> Result<ResolveMixQueryResponse> {
    let resolved = dispatch_gui_request(
        params,
        &ResolveMixQueryRequest {
            call_id: None,
            queries,
            hint_position,
            initial_playback_item: Some(track),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems { items: queue },
        },
    )
    .await?
    .context("resolved track list")?;
    let revision = dispatch_gui_request(params, &PlaylistRevisionRequest { call_id: None })
        .await?
        .context("current player queue revision")?
        .revision;
    dispatch_gui_request(
        params,
        &LoadResolvedItemsRequest {
            items: resolved.playing_items.clone(),
            start_index: resolved.start_index,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
            call_id: None,
        },
    )
    .await?;
    Ok(resolved)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_load_starts_the_selected_track_after_the_catalog_drops_an_earlier_one() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let ids = [
        session_audio(&fixture, "cleanup-first.ogg").await?,
        session_audio(&fixture, "cleanup-second.ogg").await?,
        session_audio(&fixture, "cleanup-third.ogg").await?,
    ];
    let items = futures::future::join_all(ids.iter().map(|id| library_track_for_id(&fixture, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let queries = ids
        .iter()
        .map(|id| MixQuery {
            operator: "lib::track".into(),
            parameter: id.to_string(),
        })
        .collect::<Vec<_>>();
    let resolved = dispatch_gui_request(
        &fixture.global_params,
        &ResolveMixQueryRequest {
            call_id: None,
            queries,
            hint_position: 1,
            initial_playback_item: Some(items[1].into()),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems { items: Vec::new() },
        },
    )
    .await?
    .context("resolved track list")?;
    assert_eq!(
        resolved.playing_items,
        items
            .iter()
            .map(|item| PlayingItemRef::from(*item))
            .collect::<Vec<_>>()
    );
    assert_eq!(resolved.start_index, Some(1));

    // The scanner removes the first track after the list was resolved, before the load
    // prepares it.
    media_files::Entity::delete_by_id(ids[0])
        .exec(&*fixture.global_params.test_library().main_db)
        .await?;
    let revision = dispatch_gui_request(
        &fixture.global_params,
        &PlaylistRevisionRequest { call_id: None },
    )
    .await?
    .context("player queue revision")?
    .revision;
    dispatch_gui_request(
        &fixture.global_params,
        &LoadResolvedItemsRequest {
            call_id: None,
            items: resolved.playing_items,
            start_index: resolved.start_index,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        },
    )
    .await?;
    let playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing && status.item.is_some()
    })
    .await?;
    assert_eq!(
        playing.item,
        Some(PlayingItem::from(items[1])),
        "the load started a different track after the catalog changed"
    );
    assert_eq!(
        playing.playlist,
        vec![
            PlayingItem::from(items[1]),
            PlayingItem::from(items[2])
        ]
    );
    fixture.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_load_maps_duplicate_occurrences_and_rejects_a_removed_selection() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let ids = [
        session_audio(&fixture, "duplicate-first.ogg").await?,
        session_audio(&fixture, "duplicate-second.ogg").await?,
        session_audio(&fixture, "duplicate-third.ogg").await?,
    ];
    let items = futures::future::join_all(ids.iter().map(|id| library_track_for_id(&fixture, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    // The scanner removes B, which the list names between two occurrences of A.
    media_files::Entity::delete_by_id(ids[1])
        .exec(&*fixture.global_params.test_library().main_db)
        .await?;
    let load = |items: Vec<PlayingItemRef>, start_index: u32| LoadResolvedItemsRequest {
        call_id: None,
        items,
        start_index: Some(start_index),
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: LoadPlay::Auto,
    };

    let duplicated = vec![
        PlayingItemRef::from(items[0]),
        PlayingItemRef::from(items[1]),
        PlayingItemRef::from(items[0]),
        PlayingItemRef::from(items[2]),
    ];
    dispatch_gui_request(&fixture.global_params, &load(duplicated, 2)).await?;
    let playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing && status.item.is_some()
    })
    .await?;
    assert_eq!(
        playing.item,
        Some(PlayingItem::from(items[0])),
        "the load started a different occurrence than the selected one"
    );
    assert_eq!(playing.index, Some(1));
    assert_eq!(
        playing.playlist,
        vec![
            PlayingItem::from(items[0]),
            PlayingItem::from(items[0]),
            PlayingItem::from(items[2])
        ]
    );

    let before = {
        fixture.global_params.player.lock().await.pause();
        wait_player(&fixture, |status| {
            status.state == PlayerPlaybackState::Paused
        })
        .await?
    };
    let error = match dispatch_gui_request(
        &fixture.global_params,
        &load(
            items
                .iter()
                .map(|item| PlayingItemRef::from(*item))
                .collect(),
            1,
        ),
    )
    .await
    {
        Ok(_) => panic!("a load whose selected track left the catalog succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "selected track"
        })
    ));
    let after = fixture.global_params.player.lock().await.get_status();
    assert_eq!(after.playlist, before.playlist);
    assert_eq!(after.item, before.item);
    assert_eq!(after.index, before.index);
    fixture.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clicked_subsonic_tracks_play_on_this_device() -> Result<()> {
    use crate::messages::{FetchMediaFilesRequest, MixQueryRequest, SearchForRequest};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let registered = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        &node,
    )
    .await?;
    let record = registered.record.context("Subsonic record")?;
    open_registered_library_record(node.clone(), record.record_id).await?;
    assert!(matches!(node.current_player()?, CurrentPlayer::Local));
    let track_ref = |file: &MediaFile| PlayingItemRef::LibraryTrack {
        library_id: file.library_id.clone(),
        track: file.track.clone(),
    };

    let tracks_page = dispatch_gui_request(
        &fixture.global_params,
        &FetchMediaFilesRequest {
            cursor: 0,
            page_size: 100,
            bake_cover_arts: false,
            call_id: None,
        },
    )
    .await?
    .context("Subsonic tracks page")?
    .media_files;
    let first_click = track_ref(&tracks_page[0]);
    let resolved = play_clicked_track(
        &fixture.global_params,
        vec![MixQuery {
            operator: "lib::all".into(),
            parameter: "true".into(),
        }],
        0,
        first_click.clone(),
        Vec::new(),
    )
    .await?;
    assert_eq!(resolved.playing_items, vec![first_click.clone()]);
    assert_eq!(resolved.start_index, Some(0));
    let first_track = playing_item_from_request(first_click.clone(), None).await?;
    wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing && status.item.as_ref() == Some(&first_track)
    })
    .await?;

    let album = dispatch_gui_request(
        &fixture.global_params,
        &SearchForRequest {
            call_id: None,
            query_str: "Album".into(),
            fields: Vec::new(),
            n: 10,
        },
    )
    .await?
    .context("Subsonic album search")?
    .albums[0];
    let album_query = vec![MixQuery {
        operator: "lib::album".into(),
        parameter: album.to_string(),
    }];
    let album_page = dispatch_gui_request(
        &fixture.global_params,
        &MixQueryRequest {
            call_id: None,
            queries: album_query.clone(),
            cursor: 0,
            page_size: 30,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        },
    )
    .await?
    .context("Subsonic album page")?
    .files;
    let album_tracks = album_page.iter().map(track_ref).collect::<Vec<_>>();
    let resolved = play_clicked_track(
        &fixture.global_params,
        album_query,
        1,
        album_tracks[1].clone(),
        vec![first_click],
    )
    .await?;
    assert_eq!(resolved.playing_items, album_tracks);
    assert_eq!(resolved.start_index, Some(1));
    let mut album_items = Vec::with_capacity(album_tracks.len());
    for track in album_tracks {
        album_items.push(playing_item_from_request(track, None).await?);
    }
    let playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing && status.item.as_ref() == Some(&album_items[1])
    })
    .await?;
    assert_eq!(
        playing
            .playlist
            .iter()
            .map(|entry| entry.item.clone())
            .collect::<Vec<_>>(),
        album_items
    );

    fixture.global_params.player.lock().await.stop();
    server.abort();
    node.close_ui(&fixture.global_params.main_token).await;
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_playlist_detail_lists_its_occurrences_in_order() -> Result<()> {
    use crate::messages::{FetchPlaylistOccurrencesRequest, FetchPlaylistOccurrencesResponse};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    let registered = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        &node,
    )
    .await?;
    let record = registered.record.context("Subsonic record")?;
    open_registered_library_record(node.clone(), record.record_id).await?;

    let playlist_id = dispatch_gui_request(
        &fixture.global_params,
        &FetchAllPlaylistsRequest { call_id: None },
    )
    .await?
    .context("Subsonic playlist list")?
    .playlists[0]
        .id;
    let page = |after| FetchPlaylistOccurrencesRequest {
        request_id: "playlist-detail".into(),
        playlist_id,
        after,
        page_size: 2,
        bake_cover_arts: true,
        call_id: None,
    };
    let listed = |page: &FetchPlaylistOccurrencesResponse| {
        page.entries
            .iter()
            .map(|entry| {
                let file = entry.file.as_ref().expect("a listed Subsonic entry has its track");
                (entry.position, file.track.clone(), file.title.clone())
            })
            .collect::<Vec<_>>()
    };
    let first = dispatch_gui_request(&fixture.global_params, &page(None))
        .await?
        .context("first playlist page")?;
    assert_eq!(first.request_id, "playlist-detail");
    assert_eq!(
        listed(&first),
        [(0, "song-a".into(), "A".into()), (1, "song-b".into(), "B".into())]
    );
    let second = dispatch_gui_request(&fixture.global_params, &page(first.next.clone()))
        .await?
        .context("second playlist page")?;
    assert_eq!(listed(&second), [(2, "song-a".into(), "A".into())]);
    assert!(second.next.is_none());
    assert_eq!(
        calls.lock().unwrap().last().map(String::as_str),
        Some("getPlaylist")
    );

    server.abort();
    node.close_ui(&fixture.global_params.main_token).await;
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clicked_subsonic_playlist_entries_play_on_this_device() -> Result<()> {
    use crate::messages::FetchPlaylistOccurrencesRequest;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let registered = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        &node,
    )
    .await?;
    let record = registered.record.context("Subsonic record")?;
    open_registered_library_record(node.clone(), record.record_id).await?;

    let playlist_id = dispatch_gui_request(
        &fixture.global_params,
        &FetchAllPlaylistsRequest { call_id: None },
    )
    .await?
    .context("Subsonic playlist list")?
    .playlists[0]
        .id;
    let entries = dispatch_gui_request(
        &fixture.global_params,
        &FetchPlaylistOccurrencesRequest {
            request_id: "playlist-detail".into(),
            playlist_id,
            after: None,
            page_size: 128,
            bake_cover_arts: false,
            call_id: None,
        },
    )
    .await?
    .context("Subsonic playlist detail")?
    .entries;
    let tracks = entries
        .iter()
        .map(|entry| {
            let file = entry
                .file
                .as_ref()
                .context("a listed Subsonic entry has its track")?;
            Ok(PlayingItemRef::LibraryTrack {
                library_id: file.library_id.clone(),
                track: file.track.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // The detail lists A, B, A, and its second A is clicked.
    let resolved = play_clicked_track(
        &fixture.global_params,
        vec![MixQuery {
            operator: "lib::playlist".into(),
            parameter: playlist_id.to_string(),
        }],
        2,
        tracks[2].clone(),
        Vec::new(),
    )
    .await?;
    assert_eq!(resolved.playing_items, tracks);
    assert_eq!(resolved.start_index, Some(2));
    let mut items = Vec::with_capacity(tracks.len());
    for track in tracks {
        items.push(playing_item_from_request(track, None).await?);
    }
    let playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing && status.index == Some(2)
    })
    .await?;
    assert_eq!(playing.item.as_ref(), Some(&items[2]));
    assert_eq!(
        playing
            .playlist
            .iter()
            .map(|entry| entry.item.clone())
            .collect::<Vec<_>>(),
        items
    );

    fixture.global_params.player.lock().await.stop();
    server.abort();
    node.close_ui(&fixture.global_params.main_token).await;
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_relogin_replaces_only_a_verified_secret() -> Result<()> {
    use crate::messages::{OpenLibraryRequest, ReplaceSubsonicCredentialRequest};
    use crate::request_error::request_failure_cause;
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, state, server) = subsonic_relogin_mock().await;
    let (record_id, credential) = register_relogin_record(&node, endpoint).await?;

    let replacement = |password: &str| ReplaceSubsonicCredentialRequest {
        record_id: record_id.0.to_string(),
        username: "alice".into(),
        password: password.into(),
        call_id: Some(1),
    };
    let ui = open_registered_library_record(node.clone(), record_id.0.to_string()).await?;
    let list = FetchAllPlaylistsRequest { call_id: None };
    let fetch = || dispatch_gui_request(&ui, &list);
    let before = fetch().await?.context("initial playlist list")?.playlists;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].name, "Road");
    state.rotated.store(true, Ordering::SeqCst);
    let denied = fetch()
        .await
        .err()
        .context("the old password should be rejected")?;
    assert_eq!(
        request_failure_cause(&denied),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: record_id.0.to_string(),
        })
    );
    let rejected_error =
        replace_subsonic_credential_on_node(&replacement("wrong-password"), &node)
            .await
            .expect_err("a rejected password must not replace the credential");
    assert_eq!(
        request_failure_cause(&rejected_error),
        Some(RequestFailureCause::LibraryRegistrationFailed {
            kind: RegisterLibraryFailureKind::SubsonicWrongCredentials
        }),
    );
    let old = node.credentials.get(credential).await?;
    assert_eq!(old.secret.as_bytes(), b"old-password");

    replace_subsonic_credential_on_node(&replacement("new-password"), &node).await?;
    let after = fetch()
        .await
        .context("the open library still sends the old credential")?
        .context("retried playlist list")?
        .playlists;
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].name, before[0].name);
    let updated = node.credentials.get(credential).await?;
    assert_eq!(updated.secret.as_bytes(), b"new-password");
    let stored = LibraryRegistry::load(Path::new(&*node.config_path))?;
    assert_eq!(stored.record(record_id)?.record_id, record_id);
    assert!(matches!(
        stored.record(record_id)?.protocol,
        ProtocolAdapter::SubsonicTranslate { credential: stored_ref, .. } if stored_ref == credential
    ));

    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(2),
            record_id: record_id.0.to_string(),
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await?;

    server.abort();
    node.close_ui(&fixture.global_params.main_token).await;
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_relogin_during_open_installs_the_verified_credential() -> Result<()> {
    use crate::messages::{PlayRequest, ReplaceSubsonicCredentialRequest};
    use crate::rpc::TargetRequest;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, state, server) = subsonic_relogin_mock().await;
    let (record_id, credential) = register_relogin_record(&node, endpoint).await?;

    state.hold_next_user.store(true, Ordering::SeqCst);
    let open = open_registered_library_record(node.clone(), record_id.0.to_string());
    let replace = async {
        state.user_received.notified().await;
        state.rotated.store(true, Ordering::SeqCst);
        replace_subsonic_credential_on_node(
            &ReplaceSubsonicCredentialRequest {
                record_id: record_id.0.to_string(),
                username: "alice".into(),
                password: "new-password".into(),
                call_id: None,
            },
            &node,
        )
        .await?;
        // The old getUser response cannot finish the open until re-login has succeeded.
        assert!(!node.service_library_open());
        assert_eq!(
            node.credentials.get(credential).await?.secret.as_bytes(),
            b"new-password"
        );
        state.release_user.notify_one();
        Ok(())
    };
    let (ui, ()) = timeout(TEST_DEADLINE, async { tokio::try_join!(open, replace) }).await??;
    let playlists = dispatch_gui_request(&ui, &FetchAllPlaylistsRequest { call_id: None })
        .await
        .context("the overlapping open installed the old credential")?
        .context("playlist response")?
        .playlists;
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].name, "Road");
    let binding = node
        .service_binding(record_id.0)?
        .context("open Subsonic binding")?;
    assert_eq!(
        PlayRequest::slot(&binding.slots()),
        Slot::Translate,
        "installed the old getUser response"
    );

    server.abort();
    node.close_ui(&ui.main_token).await;
    node.shutdown().await?;
    Ok(())
}

/// A closed Subsonic library signed in to another account drops the lists it kept for the old
/// one, and one signed in again to its own account keeps them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_subsonic_library_signed_in_to_another_account_drops_its_kept_lists() -> Result<()>
{
    use crate::messages::{
        CollectionType, FetchCollectionGroupSummaryRequest, ReplaceSubsonicCredentialRequest,
    };
    use crate::service::lists::lists_folder;
    use crate::subsonic_binding::SubsonicBinding;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let library_id = open_subsonic_library_here(&node, endpoint).await?;
    let ui = node.current_ui().await?;
    dispatch_gui_request(
        &ui,
        &FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        },
    )
    .await?
    .context("the artist summary")?;
    let kept = lists_folder(Path::new(&*node.config_path), library_id).join("artists.json");
    wait_until("the kept artists", || {
        let found = kept.is_file();
        async move { Ok(found.then_some(())) }
    })
    .await?;
    node.close_ui(&ui.main_token).await;

    let sign_in = |username: &str| ReplaceSubsonicCredentialRequest {
        record_id: library_id.to_string(),
        username: username.into(),
        password: "new-secret".into(),
        call_id: None,
    };
    replace_subsonic_credential_on_node(&sign_in("alice"), &node).await?;
    assert!(kept.is_file(), "a new password dropped the lists");
    replace_subsonic_credential_on_node(&sign_in("bob"), &node).await?;
    assert!(
        !kept.exists(),
        "another account kept the lists of the old one"
    );
    // Nothing of the old account is left in the folder of the lists: at most the probe the new
    // sign-in was verified with.
    let folder = lists_folder(Path::new(&*node.config_path), library_id);
    let left: Vec<String> = fs::read_dir(&folder)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        left.iter()
            .all(|name| name == "probe.json" || name.starts_with(".probe.json.")),
        "the folder of the lists holds {left:?}"
    );
    let config = Path::new(&*node.config_path);
    assert!(SubsonicBinding::kept_probe(config, library_id, "alice").is_none());

    server.abort();
    node.shutdown().await?;
    Ok(())
}

/// Waits until the library `library_id` keeps each of `kept` on disk.
async fn kept_on_disk(node: &NodeContext, library_id: Uuid, kept: &[&str]) -> Result<()> {
    use crate::service::lists::lists_folder;

    let folder = lists_folder(Path::new(&*node.config_path), library_id);
    for name in kept {
        let file = folder.join(format!("{name}.json"));
        wait_until("the kept lists on disk", || {
            let found = file.is_file();
            async move { Ok(found.then_some(())) }
        })
        .await?;
    }
    Ok(())
}

/// The groups of an artist summary, by title, with their counts.
fn summary_groups(response: crate::messages::CollectionGroupSummaryResponse) -> Vec<(String, i32)> {
    response
        .groups
        .into_iter()
        .map(|group| (group.group_title, group.count))
        .collect()
}

/// A Subsonic library opened before opens again at once from what it kept, also while its server
/// is down, and answers from its kept lists; what no kept list answers fails as it does while the
/// server is down.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subsonic_library_opened_before_opens_from_what_it_kept_while_its_server_is_down()
-> Result<()> {
    use crate::messages::{CollectionType, FetchCollectionGroupSummaryRequest};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let library_id = open_subsonic_library_here(&node, endpoint).await?;
    let summary = FetchCollectionGroupSummaryRequest {
        call_id: None,
        collection_type: CollectionType::Artist,
    };
    let ui = node.current_ui().await?;
    let first = summary_groups(
        dispatch_gui_request(&ui, &summary)
            .await?
            .context("the artist summary")?,
    );
    kept_on_disk(&node, library_id, &["artists", "probe"]).await?;
    node.close_ui(&ui.main_token).await;
    server.abort();
    let _ = server.await;

    timeout(TEST_DEADLINE, open_registered_here(&node, library_id))
        .await?
        .context("the library did not open from what it kept")?;
    let ui = node.current_ui().await?;
    let again = summary_groups(
        dispatch_gui_request(&ui, &summary)
            .await?
            .context("the kept artist summary")?,
    );
    assert_eq!(again, first);
    let playlists = dispatch_gui_request(&ui, &FetchAllPlaylistsRequest { call_id: None })
        .await
        .err()
        .context("the playlists answered without the server")?;
    assert_eq!(classify(&playlists), RequestErrorKind::Unavailable);

    node.close_ui(&ui.main_token).await;
    node.shutdown().await?;
    Ok(())
}

/// A Subsonic library whose password changed on the server opens again from what it kept,
/// answers from its kept lists, and asks for a new sign-in once the check of the server in the
/// background finds the old one rejected. The rejected sign-in clears nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subsonic_library_whose_sign_in_was_rejected_opens_with_its_lists_and_asks_for_another()
-> Result<()> {
    use crate::messages::{CollectionType, FetchCollectionGroupSummaryRequest};

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, state, server) = subsonic_relogin_mock().await;
    let (record_id, _) = register_relogin_record(&node, endpoint).await?;
    let summary = FetchCollectionGroupSummaryRequest {
        call_id: None,
        collection_type: CollectionType::Artist,
    };
    open_registered_here(&node, record_id.0).await?;
    let ui = node.current_ui().await?;
    let first = summary_groups(
        dispatch_gui_request(&ui, &summary)
            .await?
            .context("the artist summary")?,
    );
    assert_eq!(first, [("A".to_owned(), 1)]);
    kept_on_disk(&node, record_id.0, &["artists", "probe"]).await?;
    node.close_ui(&ui.main_token).await;

    // The password changes on the server, and the library opens again with the old one.
    state.rotated.store(true, Ordering::SeqCst);
    open_registered_here(&node, record_id.0).await?;
    let ui = node.current_ui().await?;
    let again = summary_groups(
        dispatch_gui_request(&ui, &summary)
            .await?
            .context("the kept artist summary")?,
    );
    assert_eq!(again, first);
    let asked = wait_until("a sign-in asked for", || async {
        Ok(signals
            .decoded::<RequestFailedResponse>("RequestFailedResponse")
            .into_iter()
            .find(|failure| failure.cause.is_some()))
    })
    .await?;
    assert_eq!(
        asked.cause,
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: record_id.0.to_string(),
        })
    );
    kept_on_disk(&node, record_id.0, &["artists", "probe"]).await?;

    server.abort();
    node.close_ui(&ui.main_token).await;
    node.shutdown().await?;
    Ok(())
}

/// A Subsonic library opened from what it kept takes the probe the check of its server in the
/// background gives in place of the kept one, and keeps it for the next open.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subsonic_library_opened_from_what_it_kept_takes_the_probe_its_server_gives() -> Result<()>
{
    use crate::messages::PlayRequest;
    use crate::rpc::TargetRequest;
    use crate::subsonic_binding::SubsonicBinding;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, state, server) = subsonic_relogin_mock().await;
    let (record_id, _) = register_relogin_record(&node, endpoint).await?;
    open_registered_here(&node, record_id.0).await?;
    let ui = node.current_ui().await?;
    let jukebox_role = || -> Result<bool> {
        let binding = node
            .service_binding(record_id.0)?
            .context("open Subsonic binding")?;
        Ok(PlayRequest::slot(&binding.slots()) == Slot::Translate)
    };
    assert!(!jukebox_role()?);
    kept_on_disk(&node, record_id.0, &["probe"]).await?;
    node.close_ui(&ui.main_token).await;

    // The server gives the account the jukebox role.
    state.jukebox.store(true, Ordering::SeqCst);
    open_registered_here(&node, record_id.0).await?;
    let ui = node.current_ui().await?;
    wait_until("the probe the server gives", || async {
        Ok(jukebox_role()?.then_some(()))
    })
    .await?;
    wait_until("the probe kept on disk", || async {
        let kept =
            SubsonicBinding::kept_probe(Path::new(&*node.config_path), record_id.0, "alice");
        Ok(kept
            .is_some_and(|probe| probe.roles.jukebox_role)
            .then_some(()))
    })
    .await?;

    server.abort();
    node.close_ui(&ui.main_token).await;
    node.shutdown().await?;
    Ok(())
}

/// A Subsonic library that plays on its server's jukebox, opened from a probe kept without the
/// jukebox role, follows the jukebox once the check of the server grants the role.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subsonic_jukebox_is_followed_once_the_server_check_grants_its_role() -> Result<()> {
    use database::library_registry::LibraryRecord;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, state, server) = subsonic_relogin_mock().await;
    // A record that plays on the jukebox, as one registered while the account had the role.
    let credential = node
        .credentials
        .put(Credential {
            username: "alice".into(),
            secret: Secret::new(b"old-password".to_vec()),
        })
        .await?;
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Jukebox".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint,
            credential,
            mode: SubsonicOpenMode::RemoteControl,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(record.clone());
        Ok(())
    })
    .await?;
    let record_id = record.record_id.0;
    open_registered_here(&node, record_id).await?;
    let ui = node.current_ui().await?;
    kept_on_disk(&node, record_id, &["probe"]).await?;
    node.close_ui(&ui.main_token).await;

    // The server grants the role, and the library opens from the probe it kept without it.
    state.jukebox.store(true, Ordering::SeqCst);
    let before = state.jukebox_reads.load(Ordering::SeqCst);
    open_registered_here(&node, record_id).await?;
    let ui = node.current_ui().await?;
    // Following the jukebox reads its status and queue again and again.
    wait_until("the jukebox followed", || async {
        Ok((state.jukebox_reads.load(Ordering::SeqCst) >= before + 2).then_some(()))
    })
    .await?;

    server.abort();
    node.close_ui(&ui.main_token).await;
    node.shutdown().await?;
    Ok(())
}

/// A closed Subsonic library signed in again keeps the probe that sign-in was verified with, so
/// it opens next time with the roles the server gives the account now.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subsonic_sign_in_keeps_the_probe_it_was_verified_with() -> Result<()> {
    use crate::messages::ReplaceSubsonicCredentialRequest;
    use crate::subsonic_binding::SubsonicBinding;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, state, server) = subsonic_relogin_mock().await;
    let (record_id, _) = register_relogin_record(&node, endpoint).await?;
    open_registered_here(&node, record_id.0).await?;
    let ui = node.current_ui().await?;
    kept_on_disk(&node, record_id.0, &["probe"]).await?;
    node.close_ui(&ui.main_token).await;
    let jukebox_kept = || {
        SubsonicBinding::kept_probe(Path::new(&*node.config_path), record_id.0, "alice")
            .is_some_and(|probe| probe.roles.jukebox_role)
    };
    assert!(!jukebox_kept());

    // The account gets another password, with which the server gives it the jukebox role.
    state.rotated.store(true, Ordering::SeqCst);
    replace_subsonic_credential_on_node(
        &ReplaceSubsonicCredentialRequest {
            record_id: record_id.0.to_string(),
            username: "alice".into(),
            password: "new-password".into(),
            call_id: None,
        },
        &node,
    )
    .await?;
    wait_until("the verified probe kept", || async {
        Ok(jukebox_kept().then_some(()))
    })
    .await?;

    server.abort();
    node.shutdown().await?;
    Ok(())
}

/// A service library's record is removed, and the removal answered, also when its folder cannot
/// be deleted.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_service_record_whose_folder_cannot_be_deleted_is_still_removed() -> Result<()> {
    use crate::messages::RemoveLibraryRecordRequest;
    use crate::service::folder::service_library_folder;
    use database::library_registry::LibraryRecord;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let credential = node
        .credentials
        .put(Credential {
            username: "alice".into(),
            secret: Secret::new(b"password".to_vec()),
        })
        .await?;
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Subsonic".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint: "http://127.0.0.1:1".into(),
            credential,
            mode: SubsonicOpenMode::PlayOnThisDevice,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(record.clone());
        Ok(())
    })
    .await?;
    // A cover the folder cannot let go of.
    let covers =
        service_library_folder(Path::new(&*node.config_path), record.record_id.0).join("covers");
    fs::create_dir_all(&covers)?;
    fs::write(covers.join("cover.img"), b"cover")?;
    fs::set_permissions(&covers, fs::Permissions::from_mode(0o500))?;

    let remove = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: record.record_id.0.to_string(),
    };
    let removed = remove.handle(node.clone(), None, &remove).await;
    fs::set_permissions(&covers, fs::Permissions::from_mode(0o700))?;
    removed?.context("the removal")?;
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    assert!(registry.record(record.record_id).is_err());

    node.shutdown().await?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn session_threads_stop_with_node() -> Result<()> {
    const CHILD: &str = "RUNE_SESSION_TEARDOWN_CASE";
    if let Ok(case) = env::var(CHILD) {
        // Retire harness blocking workers before counting the node's threads.
        return Builder::new_multi_thread()
            .enable_all()
            .thread_keep_alive(Duration::from_millis(100))
            .build()?
            .block_on(assert_node_threads_stop(&case));
    }
    let (_, module) = module_path!().split_once("::").context("test module")?;
    let test = format!("{module}::session_threads_stop_with_node");
    for case in [
        "node-drop",
        "node-shutdown",
        "fixture-drop",
        "fixture-shutdown",
    ] {
        let status = Command::new("timeout")
            .arg("90")
            .arg(env::current_exe()?)
            .args(["--exact", &test, "--nocapture"])
            .env(CHILD, case)
            .status()?;
        assert!(status.success(), "teardown case {case} failed: {status}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn assert_node_threads_stop(case: &str) -> Result<()> {
    // Warm up process-wide I/O services before measuring owned threads.
    let warm = tempfile::tempdir()?;
    let node = fixture(warm.path()).await?;
    node.global_params.node.shutdown().await?;
    drop(node);
    // The shutdown barrier above completes before checking that no runtime threads remain.
    sleep(Duration::from_millis(300)).await;
    let baseline = process_thread_count()?;
    let before = process_threads()?;
    let dir = tempfile::tempdir()?;
    let node = if case.starts_with("fixture") {
        let fixture = fixture(dir.path()).await?;
        let node = fixture.global_params.node.clone();
        drop(fixture);
        node
    } else {
        crate::test_node::disable_media_server(dir.path())?;
        let path = dir.path().to_str().context("node path")?;
        let node = NodeContext::new(
            path,
            path,
            Arc::new(Quiet),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        // The bare-node cases exercise listener teardown along with the player.
        let manager = node.manager()?;
        manager
            .clone()
            .start(
                ([127, 0, 0, 1], 0).into(),
                DiscoveryParams {
                    device_info: node.device_info().await?,
                },
                true,
            )
            .await?;
        assert!(manager.is_running());
        node
    };
    let session = node.session()?;
    let weak = Arc::downgrade(&node);
    if case.ends_with("shutdown") {
        node.shutdown().await?;
    }
    let held_player = if case.ends_with("drop") {
        Some(session.player.lock().await)
    } else {
        None
    };
    drop(node);
    wait_until("node and player threads to stop", || async {
        let threads = process_thread_count()?;
        Ok((weak.upgrade().is_none() && threads <= baseline).then_some(()))
    })
    .await
    .with_context(|| {
        format!(
            "{case}: node or player threads survived (baseline {baseline}, now {:?}, node owners {}, before {before:?}, after {:?})",
            process_thread_count(),
            weak.strong_count(),
            process_threads(),
        )
    })?;
    // A caller may retain the session/player after the node has gone.
    drop(held_player);
    drop(session);
    Ok(())
}

#[cfg(target_os = "linux")]
fn process_thread_count() -> Result<usize> {
    Ok(fs::read_dir("/proc/self/task")?.count())
}

#[cfg(target_os = "linux")]
fn process_threads() -> Result<Vec<(String, String)>> {
    fs::read_dir("/proc/self/task")?
        .map(|entry| {
            let path = entry?.path();
            Ok((
                path.display().to_string(),
                fs::read_to_string(path.join("comm"))?,
            ))
        })
        .collect()
}

#[derive(Default)]
struct RecordedSignals(StdMutex<Vec<(String, Vec<u8>)>>);
impl Broadcaster for RecordedSignals {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        self.0
            .lock()
            .unwrap()
            .push((message.name(), message.encode_to_vec().unwrap()));
    }
}

impl RecordedSignals {
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    /// Every signal named `name` recorded so far, decoded.
    fn decoded<T: DeserializeOwned>(&self, name: &str) -> Vec<T> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(signal, _)| signal == name)
            .filter_map(|(_, payload)| rinf::deserialize::<T>(payload).ok())
            .collect()
    }
}

fn recorded_handoff_order(
    signals: &RecordedSignals,
    source_session_id: &str,
    target_session_id: &str,
    merged_queue_len: usize,
) -> Result<(usize, usize)> {
    let recorded = signals.0.lock().unwrap();
    let source_paused_at = recorded
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| name == "PlaybackStatus")
        .filter_map(|(index, (_, payload))| {
            rinf::deserialize::<PlaybackStatus>(payload)
                .ok()
                .filter(|status| status.session_id == source_session_id && status.state == PlaybackState::Paused)
                .map(|_| index)
        })
        .next()
        .context("source pause status during handoff")?;
    let target_loaded_at = recorded
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| name == "PlaylistUpdate")
        .filter_map(|(index, (_, payload))| {
            rinf::deserialize::<PlaylistUpdate>(payload)
                .ok()
                .filter(|update| {
                    update.session_id == target_session_id && update.items.len() == merged_queue_len
                })
                .map(|_| index)
        })
        .next()
        .context("target load queue update during handoff")?;
    Ok((source_paused_at, target_loaded_at))
}

async fn session_audio(f: &Fixture, name: &str) -> Result<i32> {
    let library = f.global_params.test_library();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let target = Path::new(&library.media_root).join(name);
    fs::copy(source, &target)?;
    // Keep the scanner's async state out of the enclosing handoff test's frame.
    Box::pin(scan_audio_file(
        &library.fsio,
        &library.main_db,
        &f.global_params.node_id,
        Path::new(&library.media_root),
        &target,
        &library.library_sync.record_store(),
    ))
    .await?;
    Ok(media_files::Entity::find()
        .filter(Column::FileName.eq(name))
        .one(&*library.main_db)
        .await?
        .context("scanned session audio")?
        .id)
}

async fn repeat_one_single_item_queue() -> Result<(TempDir, Fixture)> {
    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let track = session_audio(&fixture, "repeat-one-next.ogg").await?;
    queue_session(&fixture, &[track]).await?;

    dispatch_gui_request(
        &fixture.global_params,
        &SetPlaybackModeRequest {
            call_id: None,
            mode: WirePlaybackMode::RepeatOne,
        },
    )
    .await?;
    dispatch_gui_request(&fixture.global_params, &PlayRequest { call_id: None }).await?;
    dispatch_gui_request(
        &fixture.global_params,
        &SeekRequest {
            call_id: None,
            position_seconds: 0.0,
        },
    )
    .await?;
    wait_player(&fixture, |status| {
        status.item.is_some()
            && status.playback_mode == PlaybackMode::RepeatOne
            && status.position.is_zero()
    })
    .await?;
    Ok((node_dir, fixture))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn next_in_repeat_one_single_item_queue_answers_at_zero_position() -> Result<()> {
    use crate::messages::NextRequest;

    let (_node_dir, fixture) = repeat_one_single_item_queue().await?;

    let started = Instant::now();
    let response = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&fixture.global_params, &NextRequest { call_id: None }),
    )
    .await;
    assert!(
        response.is_ok(),
        "NextRequest did not answer in {TEST_DEADLINE:?}; elapsed {:?}",
        started.elapsed()
    );
    response??;

    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn previous_in_repeat_one_single_item_queue_answers_at_zero_position() -> Result<()> {
    use crate::messages::PreviousRequest;

    let (_node_dir, fixture) = repeat_one_single_item_queue().await?;
    let started = Instant::now();
    let response = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&fixture.global_params, &PreviousRequest { call_id: None }),
    )
    .await;
    assert!(
        response.is_ok(),
        "PreviousRequest did not answer in {TEST_DEADLINE:?}; elapsed {:?}",
        started.elapsed()
    );
    response??;

    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn liked_requests_address_a_library_other_than_the_ui_library() -> Result<()> {
    use crate::messages::{GetLikedRequest, RegisterLocalLibraryRequest};

    let node_dir = tempfile::tempdir()?;
    let second_library_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let registered = register_local_library_on_node(
        &RegisterLocalLibraryRequest {
            call_id: None,
            path: second_library_dir.path().to_string_lossy().into_owned(),
            alias: "Second liked library".into(),
            mode: Some(LibraryInitializeMode::Portable),
        },
        &fixture.global_params.node,
    )
    .await?;
    let record = registered.record.context("second library record")?;
    let library_id = LibraryRegistry::load(node_dir.path())?
        .catalogs()
        .find(|library| library.alias == record.alias)
        .and_then(|library| library.library_id)
        .context("second library catalog id")?;
    let library = fixture
        .global_params
        .library_pool
        .acquire(library_id)
        .await?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let target = Path::new(&library.media_root).join("liked-in-second-library.ogg");
    fs::copy(source, &target)?;
    scan_audio_file(
        &library.fsio,
        &library.main_db,
        &fixture.global_params.node_id,
        Path::new(&library.media_root),
        &target,
        &library.library_sync.record_store(),
    )
    .await?;
    let file = media_files::Entity::find()
        .filter(Column::FileName.eq("liked-in-second-library.ogg"))
        .one(&*library.main_db)
        .await?
        .context("second library track")?;
    let track = file_uuids_by_id(&library.main_db, &[file.id])
        .await?
        .get(&file.id)
        .copied()
        .context("second library track UUID")?;
    let item = PlayingItemRef::LibraryTrack {
        library_id: library_id.to_string(),
        track: track.to_string(),
    };

    let initial = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            call_id: None,
            item: Some(item.clone()),
        },
    )
    .await?
    .context("initial liked response")?;
    assert!(!initial.liked);
    let updated = dispatch_gui_request(
        &fixture.global_params,
        &SetLikedRequest {
            call_id: None,
            item: Some(item.clone()),
            liked: true,
        },
    )
    .await?
    .context("set liked response")?;
    assert!(updated.liked);
    let final_state = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            call_id: None,
            item: Some(item.clone()),
        },
    )
    .await?
    .context("final liked response")?;
    assert!(final_state.liked);
    let unliked = dispatch_gui_request(
        &fixture.global_params,
        &SetLikedRequest {
            call_id: None,
            item: Some(item.clone()),
            liked: false,
        },
    )
    .await?
    .context("unset liked response")?;
    assert!(!unliked.liked);
    let final_state = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            call_id: None,
            item: Some(item),
        },
    )
    .await?
    .context("final unliked response")?;
    assert!(!final_state.liked);
    Ok(())
}

#[tokio::test]
async fn the_lyrics_of_a_track_of_another_local_library_are_read_from_that_library() -> Result<()> {
    use crate::messages::RegisterLocalLibraryRequest;

    let node_dir = tempfile::tempdir()?;
    let second_library_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let registered = register_local_library_on_node(
        &RegisterLocalLibraryRequest {
            call_id: None,
            path: second_library_dir.path().to_string_lossy().into_owned(),
            alias: "Second lyrics library".into(),
            mode: Some(LibraryInitializeMode::Portable),
        },
        &fixture.global_params.node,
    )
    .await?;
    let record = registered.record.context("second library record")?;
    let library_id = LibraryRegistry::load(node_dir.path())?
        .catalogs()
        .find(|library| library.alias == record.alias)
        .and_then(|library| library.library_id)
        .context("second library catalog id")?;
    let library = fixture
        .global_params
        .library_pool
        .acquire(library_id)
        .await?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let target = Path::new(&library.media_root).join("lyrics-in-second-library.ogg");
    fs::copy(source, &target)?;
    fs::write(
        target.with_extension("lrc"),
        "[00:01.00]From the second library\n",
    )?;
    scan_audio_file(
        &library.fsio,
        &library.main_db,
        &fixture.global_params.node_id,
        Path::new(&library.media_root),
        &target,
        &library.library_sync.record_store(),
    )
    .await?;
    let file = media_files::Entity::find()
        .filter(Column::FileName.eq("lyrics-in-second-library.ogg"))
        .one(&*library.main_db)
        .await?
        .context("second library track")?;
    let track = file_uuids_by_id(&library.main_db, &[file.id])
        .await?
        .get(&file.id)
        .copied()
        .context("second library track UUID")?;

    // The first library is open, and the track is the second one's.
    let lyrics = dispatch_gui_request(
        &fixture.global_params,
        &GetLyricByTrackIdRequest {
            call_id: None,
            item: Some(PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: track.to_string(),
            }),
        },
    )
    .await?
    .context("lyrics response")?;
    let contents = lyrics
        .lines
        .iter()
        .flat_map(|line| line.sections.iter().map(|section| section.content.clone()))
        .collect::<Vec<_>>();
    assert_eq!(contents, ["From the second library"]);
    Ok(())
}

#[tokio::test]
async fn setting_liked_on_a_read_only_replica_returns_a_typed_error() -> Result<()> {
    use crate::messages::SetLikedRequest;
    use database::library_registry::{
        CatalogPlacement, FsAdapter, LibraryAccess, LibraryConnectionAccess, LibraryRecord,
        LibraryRecordId, LibraryRegistry, ProtocolAdapter, RuneForwardMode,
    };

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let library_id = Uuid::new_v4();
    let owner_fingerprint = "read-only-owner";
    LibraryRegistry::update(node_dir.path(), |registry| {
        registry.libraries.push(LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "Read-only replica".into(),
            library_id: Some(library_id),
            protocol: ProtocolAdapter::RuneForward {
                owner_fingerprint: owner_fingerprint.into(),
                mode: RuneForwardMode::Replica {
                    access_level: Some(LibraryAccess::Read),
                    connection_access: Some(LibraryConnectionAccess::None),
                },
            },
            fs: vec![FsAdapter::RunePeer {
                fingerprint: owner_fingerprint.into(),
            }],
            catalog: CatalogPlacement::ConfigReplica,
            background_sync: false,
        });
        Ok(())
    })
    .await?;
    let item = PlayingItemRef::LibraryTrack {
        library_id: library_id.to_string(),
        track: Uuid::new_v4().to_string(),
    };
    let error = match dispatch_gui_request(
        &fixture.global_params,
        &SetLikedRequest {
            call_id: None,
            item: Some(item),
            liked: true,
        },
    )
    .await
    {
        Ok(_) => panic!("read-only replica must reject liked-state writes"),
        Err(error) => error,
    };
    assert!(error.chain().any(|cause| matches!(
        cause.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "write access to liked state"
        })
    )));
    Ok(())
}

#[tokio::test]
async fn setting_liked_on_an_item_without_liked_state_returns_a_typed_error() -> Result<()> {
    use crate::request_error::classify;

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let library_id = fixture.global_params.test_library().library_id.to_string();
    for (item, kind) in [
        (
            PlayingItemRef::LibraryTrack {
                library_id: library_id.clone(),
                track: Uuid::new_v4().to_string(),
            },
            RequestErrorKind::NotFound,
        ),
        (
            PlayingItemRef::LibraryTrack {
                library_id,
                track: "not-a-track-uuid".into(),
            },
            RequestErrorKind::InvalidInput,
        ),
        (
            PlayingItemRef::IndependentFile {
                path: "/music/independent.flac".into(),
            },
            RequestErrorKind::InvalidInput,
        ),
        (
            PlayingItemRef::IndependentFile {
                path: String::new(),
            },
            RequestErrorKind::InvalidInput,
        ),
    ] {
        let request = SetLikedRequest {
            call_id: None,
            item: Some(item.clone()),
            liked: true,
        };
        let error = match dispatch_gui_request(&fixture.global_params, &request).await {
            Ok(_) => anyhow::bail!("{item:?} has no liked state and must fail"),
            Err(error) => error,
        };
        assert_eq!(classify(&error), kind, "{item:?}: {error:#}");
    }
    Ok(())
}

#[tokio::test]
async fn liked_requests_without_an_item_are_refused() -> Result<()> {
    use crate::messages::GetLikedRequest;
    use crate::request_error::classify;

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let set = dispatch_gui_request(
        &fixture.global_params,
        &SetLikedRequest {
            call_id: None,
            item: None,
            liked: true,
        },
    )
    .await;
    let get = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            call_id: None,
            item: None,
        },
    )
    .await;
    for (name, result) in [
        ("SetLikedRequest", set.map(|response| response.is_some())),
        ("GetLikedRequest", get.map(|response| response.is_some())),
    ] {
        let error = match result {
            Ok(answered) => anyhow::bail!("{name} without an item answered {answered}"),
            Err(error) => error,
        };
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput, "{name}");
    }
    Ok(())
}

#[tokio::test]
async fn server_start_and_stop_failures_are_reported() -> Result<()> {
    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let node = &fixture.global_params.node;
    let manager = node.server_manager.get().cloned().context("server manager")?;

    // 192.0.2.1 is reserved for documentation (RFC 5737), so no interface of
    // this device has it and the server cannot listen there.
    let error = match Arc::clone(&manager)
        .start(
            ([192, 0, 2, 1], 0).into(),
            DiscoveryParams {
                device_info: node.device_info().await?,
            },
            true,
        )
        .await
    {
        Ok(()) => anyhow::bail!("a server cannot start on an address this device lacks"),
        Err(error) => error,
    };
    assert!(
        format!("{error:#}").contains("Failed to bind sync server"),
        "{error:#}"
    );
    assert!(!manager.is_running());
    assert!(!node.device_scanner.is_announcing().await);

    // The failed start left no server, so there is nothing to stop.
    let error = match manager.stop().await {
        Ok(()) => anyhow::bail!("stopping a server that is not running must fail"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("Server not running"), "{error:#}");
    Ok(())
}

/// The kind of a request failure, or an error naming the request that did not fail.
fn refused_kind<T>(name: &str, result: Result<Option<T>>) -> Result<RequestErrorKind> {
    match result {
        Ok(_) => anyhow::bail!("{name} must fail"),
        Err(error) => Ok(crate::request_error::classify(&error)),
    }
}

#[tokio::test]
async fn approving_or_blocking_an_unknown_device_is_not_found() -> Result<()> {
    use crate::messages::{ClientStatus, DevicePreset, UpdateClientStatusRequest};

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    for (status, preset) in [
        (ClientStatus::Blocked, None),
        (ClientStatus::Approved, Some(DevicePreset::Guest)),
    ] {
        let request = UpdateClientStatusRequest {
            call_id: None,
            fingerprint: "unknown-device".into(),
            status,
            preset,
        };
        let result = request
            .handle(request.extract_params(&fixture.global_params), None, &request)
            .await;
        assert_eq!(
            refused_kind("UpdateClientStatusRequest", result)?,
            RequestErrorKind::NotFound
        );
    }
    Ok(())
}

#[tokio::test]
async fn connecting_without_a_reachable_host_is_typed() -> Result<()> {
    use crate::messages::{ConnectRequest, FetchServerCertificateRequest};

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let params = &fixture.global_params;
    // Nothing listens on port 1 of this device.
    for (hosts, kind) in [
        (Vec::new(), RequestErrorKind::InvalidInput),
        (vec!["127.0.0.1:1".to_owned()], RequestErrorKind::Unavailable),
    ] {
        let request = ConnectRequest {
            call_id: None,
            hosts,
        };
        let result = request
            .handle(request.extract_params(params), None, &request)
            .await;
        assert_eq!(refused_kind("ConnectRequest", result)?, kind);
    }
    let request = FetchServerCertificateRequest {
        call_id: None,
        url: "https://127.0.0.1:1/ping".into(),
    };
    let result = request.handle((), None, &request).await;
    assert_eq!(
        refused_kind("FetchServerCertificateRequest", result)?,
        RequestErrorKind::Unavailable
    );
    Ok(())
}

#[tokio::test]
async fn remote_file_requests_that_cannot_be_served_are_typed() -> Result<()> {
    use crate::messages::FetchRemoteFileRequest;

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let digest = blake3::hash(b"cover").to_hex().to_string();
    let library_id = Uuid::new_v4();
    for (url, kind) in [
        ("not a url".to_owned(), RequestErrorKind::InvalidInput),
        (
            format!("https://127.0.0.1:7863/files/library/{digest}"),
            RequestErrorKind::InvalidInput,
        ),
        (
            format!("https://127.0.0.1:7863/?library_id={library_id}"),
            RequestErrorKind::InvalidInput,
        ),
        (
            format!("https://127.0.0.1:7863/files/library/cover.jpg?library_id={library_id}"),
            RequestErrorKind::InvalidInput,
        ),
        (
            format!("https://untrusted.invalid/files/library/{digest}?library_id={library_id}"),
            RequestErrorKind::PermissionDenied,
        ),
    ] {
        let request = FetchRemoteFileRequest {
            call_id: None,
            url: url.clone(),
        };
        let result = request
            .handle(request.extract_params(&fixture.global_params), None, &request)
            .await;
        assert_eq!(refused_kind(&url, result)?, kind, "{url}");
    }
    Ok(())
}

#[tokio::test]
async fn removing_trust_with_an_unreadable_trust_store_fails() -> Result<()> {
    use crate::messages::{RemoveTrustedClientRequest, RemoveTrustedServerRequest};

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let params = &fixture.global_params;
    fs::write(node_dir.path().join(".known-servers"), "not [ a trust store")?;
    let request = RemoveTrustedClientRequest {
        call_id: None,
        fingerprint: "some-device".into(),
    };
    let result = request
        .handle(request.extract_params(params), None, &request)
        .await;
    assert_eq!(
        refused_kind("RemoveTrustedClientRequest", result)?,
        RequestErrorKind::Internal
    );
    let request = RemoveTrustedServerRequest {
        call_id: None,
        fingerprint: "some-device".into(),
    };
    let result = request
        .handle(request.extract_params(params), None, &request)
        .await;
    assert_eq!(
        refused_kind("RemoveTrustedServerRequest", result)?,
        RequestErrorKind::Internal
    );
    Ok(())
}

/// The failure kind of a library record removal that must fail.
fn removal_failure_kind(
    result: Result<Option<crate::messages::RemoveLibraryRecordResponse>>,
) -> Result<crate::messages::RemoveLibraryRecordFailureKind> {
    let error = match result {
        Ok(_) => anyhow::bail!("the record removal must fail"),
        Err(error) => error,
    };
    match crate::request_error::request_failure_cause(&error) {
        Some(RequestFailureCause::LibraryRecordRemovalFailed { kind }) => Ok(kind),
        cause => anyhow::bail!("removal failed without its kind ({cause:?}): {error:#}"),
    }
}

/// The failure kind of a library that must not open.
fn opened_failure_kind(
    result: Result<crate::messages::OpenLibraryResponse>,
) -> Result<OpenLibraryFailureKind> {
    let error = match result {
        Ok(response) => anyhow::bail!("record {} must not open", response.record_id),
        Err(error) => error,
    };
    match crate::request_error::request_failure_cause(&error) {
        Some(RequestFailureCause::LibraryOpenFailed { kind }) => Ok(kind),
        cause => anyhow::bail!("opening failed without its kind ({cause:?}): {error:#}"),
    }
}

/// The failure kind of a library registration that must fail.
fn registration_failure_kind(
    result: Result<crate::messages::RegisterLibraryResponse>,
) -> Result<RegisterLibraryFailureKind> {
    let error = match result {
        Ok(response) => anyhow::bail!("registration must fail, answered {:?}", response.outcome),
        Err(error) => error,
    };
    match crate::request_error::request_failure_cause(&error) {
        Some(RequestFailureCause::LibraryRegistrationFailed { kind }) => Ok(kind),
        cause => anyhow::bail!("registration failed without its kind ({cause:?}): {error:#}"),
    }
}

async fn attach_test_cover(f: &Fixture, media_file_id: i32, bytes: &[u8]) -> Result<()> {
    let library = f.global_params.test_library();
    let hash = blake3::hash(bytes).to_hex().to_string();
    let cover_uuid = artwork(&hash).to_string();
    if media_cover_art::Entity::find()
        .filter(media_cover_art::Column::SourceUuid.eq(&cover_uuid))
        .one(&*library.main_db)
        .await?
        .is_none()
    {
        media_cover_art::Entity::insert(media_cover_art::ActiveModel {
            id: sea_orm::NotSet,
            file_hash: Set(String::new()),
            binary: Set(bytes.to_vec()),
            primary_color: Set(None),
            content_hash: Set(Some(hash)),
            content_size: Set(Some(i64::try_from(bytes.len())?)),
            local_available: Set(true),
            source_uuid: Set(cover_uuid.clone()),
        })
        .exec(&*library.main_db)
        .await?;
    }
    let mut file = media_files::Entity::find_by_id(media_file_id)
        .one(&*library.main_db)
        .await?
        .context("cover fixture track")?
        .into_active_model();
    file.cover_art_uuid = Set(Some(cover_uuid));
    file.update(&*library.main_db).await?;
    Ok(())
}

async fn library_track_for_id(f: &Fixture, id: i32) -> Result<LibraryItem> {
    let library = f.global_params.test_library();
    let file_uuid = file_uuids_by_id(&library.main_db, &[id])
        .await?
        .get(&id)
        .copied()
        .context("file uuid")?;
    Ok(LibraryItem {
        library_id: library.library_id,
        file_uuid,
    })
}

async fn wait_player(
    f: &Fixture,
    predicate: impl Fn(&PlayerStatus) -> bool,
) -> Result<PlayerStatus> {
    let predicate = &predicate;
    wait_until("the session to reach the expected state", || async move {
        let status = f.global_params.player.lock().await.get_status();
        Ok(predicate(&status).then_some(status))
    })
    .await
}

async fn queue_session(f: &Fixture, ids: &[i32]) -> Result<()> {
    let items = future::join_all(ids.iter().map(|id| library_track_for_id(f, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let player = f.global_params.player.lock().await;
    player.add_to_playlist_with_sources(
        items
            .into_iter()
            .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    drop(player);
    wait_player(f, |status| status.playlist.len() == ids.len()).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_link_forwards_controls_routes_status_and_keeps_local_queue() -> Result<()> {
    use crate::messages::{
        LoadRequest, MovePlaylistItemRequest, NextRequest, PauseRequest, PlayRequest,
        PreviousRequest, RemoveRequest, RendererAttachmentChanged, SeekRequest,
        SetPlaybackModeRequest, SwitchRequest,
    };

    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = super::fixture_with_broadcaster(
        first.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = super::fixture(second.path()).await?;
    let b_address = super::paired_full(&b, &a).await?;
    let b_info = super::info(&b).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_info.fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;

    let a_track = session_audio(&a, "local-renderer-link.ogg").await?;
    queue_session(&a, &[a_track]).await?;
    a.global_params.player.lock().await.play();
    wait_player(&a, |status| status.state == PlayerPlaybackState::Playing).await?;
    let b_tracks = [
        session_audio(&b, "remote-renderer-first.ogg").await?,
        session_audio(&b, "remote-renderer-second.ogg").await?,
    ];
    let b_items =
        future::join_all(b_tracks.iter().map(|id| library_track_for_id(&b, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let expected_first_uuid = b_items[1].file_uuid;
    b.global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                b_items
                    .into_iter()
                    .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&b, |status| status.playlist.len() == 2).await?;

    signals.0.lock().unwrap().clear();
    a.global_params
        .node
        .attach_renderer(&b_info.fingerprint)
        .await?;
    let attached = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|(name, payload)| {
            (name == "RendererAttachmentChanged")
                .then(|| rinf::deserialize::<RendererAttachmentChanged>(payload).ok())
                .flatten()
        })
        .context("attach signal")?;
    assert_eq!(
        attached
            .renderer
            .as_ref()
            .map(|renderer| renderer.fingerprint.as_str()),
        Some(b_info.fingerprint.as_str())
    );
    wait_player(&a, |status| status.state == PlayerPlaybackState::Paused).await?;
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let remote_session_id = b.global_params.node.session()?.session_id().await;
    wait_until("renderer queue refresh after attach", || async {
        let has_remote = signals.0.lock().unwrap().iter().any(|(name, payload)| {
            name == "PlaylistUpdate"
                && rinf::deserialize::<PlaylistUpdate>(payload)
                    .is_ok_and(|update| update.session_id == remote_session_id)
        });
        Ok(has_remote.then_some(()))
    })
    .await
    .context("renderer queue refresh did not reach local GUI after attach")?;
    let refreshed = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "PlaylistUpdate")
        .filter_map(|(_, payload)| rinf::deserialize::<PlaylistUpdate>(payload).ok())
        .find(|update| update.session_id == remote_session_id)
        .context("renderer PlaylistUpdate after attach")?;
    assert_eq!(
        refreshed.revision,
        b.global_params.node.session()?.queue_revision().await
    );
    assert_ne!(refreshed.session_id, local_session_id);
    let local_queue_len = a
        .global_params
        .player
        .lock()
        .await
        .get_status()
        .playlist
        .len();
    assert_eq!(local_queue_len, 1);

    let local_volume = a
        .global_params
        .node
        .session()?
        .playback_snapshot()
        .await
        .0
        .volume;
    let volume = dispatch_gui_request(
        &a.global_params,
        &VolumeRequest {
            call_id: None,
            volume: 0.63,
        },
    )
    .await?
    .context("renderer volume response")?;
    assert_eq!(volume.volume, 0.63);
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        0.63
    );
    assert_eq!(
        a.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        local_volume
    );

    dispatch_gui_request(&a.global_params, &PlayRequest { call_id: None }).await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .state,
        PlayerPlaybackState::Playing
    );
    assert_eq!(
        a.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .state,
        PlayerPlaybackState::Paused
    );
    dispatch_gui_request(&a.global_params, &PauseRequest { call_id: None }).await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .state,
        PlayerPlaybackState::Paused
    );

    dispatch_gui_request(
        &a.global_params,
        &SeekRequest {
            call_id: None,
            position_seconds: 0.2,
        },
    )
    .await?;
    assert!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .position
            >= Duration::from_millis(200)
    );
    dispatch_gui_request(
        &a.global_params,
        &SetPlaybackModeRequest {
            call_id: None,
            mode: WirePlaybackMode::RepeatAll,
        },
    )
    .await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .playback_mode,
        PlaybackMode::RepeatAll
    );
    dispatch_gui_request(
        &a.global_params,
        &LoadRequest {
            call_id: None,
            index: 1,
        },
    )
    .await?;
    let loaded = b.global_params.node.session()?.playback_snapshot().await.0;
    assert_eq!(loaded.index, Some(1));
    assert_eq!(loaded.state, PlayerPlaybackState::Paused);

    let revision = b.global_params.node.session()?.queue_revision().await;
    dispatch_gui_request(
        &a.global_params,
        &SwitchRequest {
            call_id: None,
            index: 0,
            expected_revision: Some(revision),
        },
    )
    .await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .index,
        Some(0)
    );
    dispatch_gui_request(&a.global_params, &NextRequest { call_id: None }).await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .index,
        Some(1)
    );
    dispatch_gui_request(&a.global_params, &PreviousRequest { call_id: None }).await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .index,
        Some(0)
    );

    let revision = b.global_params.node.session()?.queue_revision().await;
    dispatch_gui_request(
        &a.global_params,
        &MovePlaylistItemRequest {
            call_id: None,
            old_index: 0,
            new_index: 1,
            expected_revision: Some(revision),
        },
    )
    .await?;
    assert!(matches!(
        &b.global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist
            .first()
            .context("updated playlist is empty")?
            .item,
        PlayingItem::LibraryTrack { track, .. } if *track == expected_first_uuid.to_string()
    ));
    let revision = b.global_params.node.session()?.queue_revision().await;
    dispatch_gui_request(
        &a.global_params,
        &RemoveRequest {
            call_id: None,
            index: 1,
            expected_revision: Some(revision),
        },
    )
    .await?;
    assert_eq!(
        b.global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist
            .len(),
        1
    );

    wait_until("renderer queue update", || async {
        let has_remote = signals.0.lock().unwrap().iter().any(|(name, payload)| {
            name == "PlaylistUpdate"
                && rinf::deserialize::<PlaylistUpdate>(payload)
                    .is_ok_and(|update| update.session_id == remote_session_id)
        });
        Ok(has_remote.then_some(()))
    })
    .await
    .context("renderer queue update did not reach local GUI")?;
    let updates = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "PlaylistUpdate")
        .filter_map(|(_, payload)| rinf::deserialize::<PlaylistUpdate>(payload).ok())
        .collect::<Vec<_>>();
    assert!(
        updates
            .iter()
            .any(|update| update.session_id == remote_session_id)
    );
    assert!(
        !updates
            .iter()
            .any(|update| update.session_id == local_session_id)
    );

    a.global_params.node.detach_renderer().await?;
    assert!(matches!(a.global_params.node.current_player()?, CurrentPlayer::Local));
    assert_eq!(a.global_params.node.current_player()?.visualizer_inputs(), VisualizerInputs::Full);
    let detached = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|(name, payload)| {
            (name == "RendererAttachmentChanged")
                .then(|| rinf::deserialize::<RendererAttachmentChanged>(payload).ok())
                .flatten()
        })
        .context("detach signal")?;
    assert!(detached.renderer.is_none());
    assert_eq!(
        a.global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist
            .len(),
        1
    );
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_link_does_not_forward_its_controller_list() -> Result<()> {
    use crate::messages::{ControllersUpdated, ListControllersRequest};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let c_dir = tempfile::tempdir()?;
    let a_signals = Arc::new(RecordedSignals::default());
    let b_signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        a_signals.clone(),
    )
    .await?;
    let b = fixture_with_broadcaster(
        b_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        b_signals.clone(),
    )
    .await?;
    let c = fixture(c_dir.path()).await?;

    let b_address = super::paired_full(&b, &a).await?;
    let b_info = super::info(&b).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_info.fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;
    a.global_params
        .node
        .attach_renderer(&b_info.fingerprint)
        .await?;
    assert_eq!(
        a.global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(b_info.fingerprint.as_str())
    );
    a_signals.clear();

    super::pair(&b, &c).await?;
    let mut c_socket = super::connect_remote(
        "127.0.0.1",
        b_address.port(),
        &c.global_params.config_path,
        c.global_params.cert_validator.read().await.clone(),
        &c.global_params.node.sync_tokens,
    )
    .await?;
    wait_controller_count(&b.global_params.node, 2).await?;

    let b_list = ListControllersRequest { call_id: None };
    let b_controllers = b_list
        .handle(b.global_params.node.clone(), None, &b_list)
        .await?
        .context("renderer controller list")?
        .controllers;
    let a_fingerprint = super::info(&a).await?.fingerprint;
    let c_fingerprint = super::info(&c).await?.fingerprint;
    assert!(
        b_controllers
            .iter()
            .any(|entry| entry.fingerprint == a_fingerprint)
    );
    assert!(
        b_controllers
            .iter()
            .any(|entry| entry.fingerprint == c_fingerprint)
    );

    let b_updates = b_signals.decoded::<ControllersUpdated>("ControllersUpdated");
    assert!(b_updates.iter().any(|update| {
        update
            .controllers
            .iter()
            .any(|entry| entry.fingerprint == c_fingerprint)
    }));
    assert!(
        a_signals
            .decoded::<ControllersUpdated>("ControllersUpdated")
            .is_empty()
    );

    // Only updates sent after C leaves count: B's lists from before C joined lack C too.
    b_signals.clear();
    c_socket.close(None).await?;
    wait_controller_count(&b.global_params.node, 1).await?;
    wait_until("disconnected controller list", || async {
        let has_removed = b_signals
            .decoded::<ControllersUpdated>("ControllersUpdated")
            .iter()
            .any(|update| {
                update
                    .controllers
                    .iter()
                    .all(|entry| entry.fingerprint != c_fingerprint)
            });
        Ok(has_removed.then_some(()))
    })
    .await
    .context("renderer did not publish the disconnected controller list")?;
    assert!(
        a_signals
            .decoded::<ControllersUpdated>("ControllersUpdated")
            .is_empty()
    );

    drop(c_socket);
    for fixture in [&a, &b, &c] {
        fixture.global_params.node.shutdown().await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_attach_failures_are_typed() -> Result<()> {
    use crate::request_error::{ExpectedRequestError, classify};
    use database::error::ExpectedError;

    let dir = tempfile::tempdir()?;
    let a = super::fixture(dir.path()).await?;
    let unknown = a
        .global_params
        .node
        .attach_renderer("missing-renderer-fingerprint")
        .await
        .expect_err("unknown renderer should fail");
    assert!(unknown.chain().any(|cause| cause.is::<ExpectedError>()));
    assert_eq!(classify(&unknown), RequestErrorKind::NotFound);

    let fingerprint = "a".repeat(64);
    a.global_params
        .cert_validator
        .read()
        .await
        .add_trusted_domains(["127.0.0.1:1"], &fingerprint)
        .await?;
    let unreachable = timeout(
        TEST_DEADLINE,
        a.global_params.node.attach_renderer(&fingerprint),
    )
    .await
    .context("unreachable renderer attempt timed out")?
    .expect_err("unreachable renderer should fail");
    assert!(
        unreachable
            .chain()
            .any(|cause| cause.is::<ExpectedRequestError>())
    );
    assert_eq!(classify(&unreachable), RequestErrorKind::Unavailable);
    a.global_params.node.shutdown().await?;
    Ok(())
}

async fn attach_pair_with_signals(
    a: &Fixture,
    b: &Fixture,
    signals: &Arc<RecordedSignals>,
) -> Result<String> {
    let fingerprint = pair_renderer(a, b).await?;
    signals.0.lock().unwrap().clear();
    a.global_params.node.attach_renderer(&fingerprint).await?;
    Ok(fingerprint)
}

/// Pairs `b` so that `a` can attach it as its renderer, and returns `b`'s fingerprint.
async fn pair_renderer(a: &Fixture, b: &Fixture) -> Result<String> {
    let b_address = super::paired_full(b, a).await?;
    let b_info = super::info(b).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_info.fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;
    Ok(b_info.fingerprint)
}

/// Detaching a controller whose connection never ends still returns within the deadline and
/// leaves the controller unlisted.
#[tokio::test]
async fn detach_controller_returns_when_the_connection_never_ends() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let node = fixture(dir.path()).await?;
    tokio_time::pause();
    let session = node.global_params.node.session()?;
    let guard = session.attach_controller("fingerprint".into(), "alias".into());
    let id = Uuid::parse_str(&session.attached_controllers()[0].id)?;

    timeout(TEST_DEADLINE, session.detach_controller(id))
        .await
        .context("detaching a controller whose connection never ends did not return")??;

    assert!(guard.cancellation_token().is_cancelled());
    assert!(session.attached_controllers().is_empty());
    drop(guard);
    node.global_params.node.shutdown().await?;
    Ok(())
}

/// The reply to a detach request is sent only once the controller has left the list.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detach_controller_replies_after_the_controller_is_gone() -> Result<()> {
    use crate::messages::DetachControllerRequest;

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let renderer = b.global_params.node.session()?;
    wait_until("the controller to attach", || async {
        Ok((!renderer.attached_controllers().is_empty()).then_some(()))
    })
    .await?;
    let controller = renderer
        .attached_controllers()
        .into_iter()
        .next()
        .context("renderer did not register its controller")?;

    let reply = dispatch_gui_request(
        &b.global_params,
        &DetachControllerRequest {
            call_id: None,
            id: controller.id,
        },
    )
    .await?;
    assert!(
        renderer.attached_controllers().is_empty(),
        "the reply came before the controller was detached"
    );
    assert!(reply.is_some(), "the detach was not answered");
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detached_renderer_restores_local_session_signals() -> Result<()> {
    use crate::messages::{RendererAttachmentChanged, RendererLinkLost};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let b_alias = super::info(&b).await?.alias;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let local_track = session_audio(&a, "restore-after-detach.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    a.global_params.node.session()?.player.lock().await.pause();
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let controller = b
        .global_params
        .node
        .session()?
        .attached_controllers()
        .into_iter()
        .next()
        .context("renderer did not register its controller")?;
    b.global_params
        .node
        .session()?
        .detach_controller(Uuid::parse_str(&controller.id)?)
        .await?;

    wait_until("detached renderer signal sequence", || async {
        let (loss, attachment, local_queue) = {
            let records = signals.0.lock().unwrap();
            let loss = records.iter().position(|(name, bytes)| {
                name == "RendererLinkLost"
                    && rinf::deserialize::<RendererLinkLost>(bytes)
                        .is_ok_and(|event| event.alias == b_alias)
            });
            let attachment = records.iter().position(|(name, bytes)| {
                name == "RendererAttachmentChanged"
                    && rinf::deserialize::<RendererAttachmentChanged>(bytes)
                        .is_ok_and(|event| event.renderer.is_none())
            });
            let local_queue = records.iter().position(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes)
                        .is_ok_and(|event| event.session_id == local_session_id)
            });
            (loss, attachment, local_queue)
        };
        if let (Some(loss), Some(attachment), Some(queue)) = (loss, attachment, local_queue) {
            assert!(loss < attachment && attachment < queue);
            return Ok(Some(()));
        }
        Ok(None)
    })
    .await
    .context("detached renderer loss did not restore the local queue")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_renderer_trust_restores_local_session() -> Result<()> {
    use crate::messages::{RendererAttachmentChanged, RendererLinkLost};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = step("create renderer fixture", fixture(b_dir.path())).await?;
    let alias = step("read renderer identity", super::info(&b)).await?.alias;
    let fingerprint = step(
        "attach renderer",
        attach_pair_with_signals(&a, &b, &signals),
    )
    .await?;
    let local_track = step("create local track", session_audio(&a, "trust-revoked.ogg")).await?;
    step("queue local session", queue_session(&a, &[local_track])).await?;
    let local_session_id = step("read local session id", async {
        Ok::<_, anyhow::Error>(a.global_params.node.session()?.session_id().await)
    })
    .await?;
    signals.0.lock().unwrap().clear();

    step(
        "revoke renderer trust",
        a.global_params.node.revoke_outgoing_for(&fingerprint),
    )
    .await;

    step("observe restored local session", async {
        wait_until("restored local session after trust revocation", || async {
            let complete = {
                let records = signals.0.lock().unwrap();
                let lost = records.iter().any(|(name, bytes)| {
                    name == "RendererLinkLost"
                        && rinf::deserialize::<RendererLinkLost>(bytes).is_ok_and(|event| {
                            event.fingerprint == fingerprint && event.alias == alias
                        })
                });
                let detached = records.iter().any(|(name, bytes)| {
                    name == "RendererAttachmentChanged"
                        && rinf::deserialize::<RendererAttachmentChanged>(bytes)
                            .is_ok_and(|event| event.renderer.is_none())
                });
                let local_session = records.iter().any(|(name, bytes)| {
                    name == "PlaylistUpdate"
                        && rinf::deserialize::<PlaylistUpdate>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id)
                });
                lost && detached && local_session
            };
            Ok(complete.then_some(()))
        })
        .await
    })
    .await?;
    assert!(
        step("check renderer link", async {
            a.global_params.node.renderer_link()
        })
        .await?
        .is_none()
    );
    step("shutdown renderer fixture", b.global_params.node.shutdown()).await?;
    step(
        "shutdown controller fixture",
        a.global_params.node.shutdown(),
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopped_renderer_clears_link_and_reports_server_stopped() -> Result<()> {
    use crate::messages::{
        PlaybackStatus, RemoteConnectionLost, RendererAttachmentChanged, RendererLinkLost,
    };

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let b_alias = super::info(&b).await?.alias;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let local_track = session_audio(&a, "renderer-server-stopped.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    a.global_params.node.session()?.player.lock().await.pause();
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let local_session_id_text = local_session_id.to_string();
    b.global_params.node.shutdown().await?;
    wait_until("renderer shutdown signal sequence", || async {
        let observed = {
            let records = signals.0.lock().unwrap();
            let loss = records.iter().position(|(name, bytes)| {
                name == "RendererLinkLost"
                    && rinf::deserialize::<RendererLinkLost>(bytes)
                        .is_ok_and(|event| event.alias == b_alias && !event.fingerprint.is_empty())
            });
            let attachment = records.iter().position(|(name, bytes)| {
                name == "RendererAttachmentChanged"
                    && rinf::deserialize::<RendererAttachmentChanged>(bytes)
                        .is_ok_and(|event| event.renderer.is_none())
            });
            // The local player may tick a status as soon as the link is gone, before the
            // loss is announced; what matters is the re-emit after the detach.
            let after_attachment = attachment.map_or(records.len(), |index| index + 1);
            let status = records
                .iter()
                .skip(after_attachment)
                .position(|(name, bytes)| {
                    name == "PlaybackStatus"
                        && rinf::deserialize::<PlaybackStatus>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id_text)
                })
                .map(|index| index + after_attachment);
            let queue = records
                .iter()
                .skip(after_attachment)
                .position(|(name, bytes)| {
                    name == "PlaylistUpdate"
                        && rinf::deserialize::<PlaylistUpdate>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id)
                })
                .map(|index| index + after_attachment);
            let no_legacy_loss = !records.iter().any(|(name, bytes)| {
                name == "RemoteConnectionLost"
                    && rinf::deserialize::<RemoteConnectionLost>(bytes).is_ok()
            });
            (loss, attachment, status, queue, no_legacy_loss)
        };
        if let (Some(loss), Some(attachment), Some(status), Some(queue), no_legacy_loss) = observed
        {
            assert!(loss < attachment && attachment < status && status < queue);
            assert!(no_legacy_loss);
            return Ok(Some(()));
        }
        Ok(None)
    })
    .await
    .context("renderer shutdown was not reported")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    assert!(a.global_params.ui_library.lock().await.is_some());
    a.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_renderer_reader_cannot_clear_a_new_link() -> Result<()> {
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let fingerprint = attach_pair_with_signals(&a, &b, &signals).await?;
    let old = a
        .global_params
        .node
        .renderer_link()?
        .context("first link")?;
    a.global_params.node.detach_renderer().await?;
    a.global_params.node.attach_renderer(&fingerprint).await?;
    let current = a
        .global_params
        .node
        .renderer_link()?
        .context("second link")?;
    a.global_params.node.renderer_link_lost(old.id()).await;
    assert_eq!(
        a.global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.id()),
        Some(current.id())
    );
    a.global_params.node.detach_renderer().await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mix_query_forwarding_leaves_it_for_local_resolution() -> Result<()> {
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let id = session_audio(&b, "mix-query-renderer.ogg").await?;
    queue_session(&b, &[id]).await?;
    let before = b.global_params.player.lock().await.get_status().playlist;
    assert!(matches!(
        a.global_params
            .node
            .request_slot::<ResolveMixQueryRequest>()?,
        Slot::Native
    ));
    assert_eq!(
        b.global_params.player.lock().await.get_status().playlist,
        before
    );

    let read = GetMediaFilesCountRequest { call_id: None };
    assert!(matches!(
        a.global_params
            .node
            .request_slot::<GetMediaFilesCountRequest>()?,
        Slot::Native
    ));
    let response = dispatch_gui_request(&a.global_params, &read)
        .await?
        .context("local library count response")?;
    assert_eq!(response.count, 0);
    a.global_params.node.detach_renderer().await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attached_mix_query_resolves_for_controller_library_and_loads_resolved_items() -> Result<()>
{
    use crate::messages::{MixQuery, PlaylistOperateMode};
    use crate::request_error::{ExpectedRequestError, classify};

    let host_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let ids = [
        session_audio(&host, "mix-cast-first.ogg").await?,
        session_audio(&host, "mix-cast-second.ogg").await?,
        session_audio(&host, "mix-cast-local-queue.ogg").await?,
    ];
    let host_items =
        future::join_all(ids[..2].iter().map(|id| library_track_for_id(&host, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let (_, renderer_replica) = replicate_host_library(&host, &renderer).await?;
    let host_addr = host.get_address().await.context("host listener address")?;
    pair(&host, &controller).await?;
    pair(&controller, &host).await?;
    let (_, controller_replica) = replicate_host_library_at(&host, &controller, host_addr).await?;
    attach_pair_with_signals(&controller_replica, &renderer_replica, &signals).await?;

    queue_session(&controller_replica, &[ids[2]]).await?;
    let local_queue = controller_replica
        .global_params
        .player
        .lock()
        .await
        .get_playlist();
    let local_revision = controller_replica
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let controller_file_ids = file_ids_by_uuid(
        &controller_replica.global_params.test_library().main_db,
        &host_items
            .iter()
            .map(|item| item.file_uuid)
            .collect::<Vec<_>>(),
    )
    .await?;
    let renderer_file_ids = file_ids_by_uuid(
        &renderer_replica.global_params.test_library().main_db,
        &[host_items[0].file_uuid],
    )
    .await?;
    queue_session(
        &renderer_replica,
        &[renderer_file_ids[&host_items[0].file_uuid]],
    )
    .await?;
    let current_player_queue = controller_replica
        .global_params
        .node
        .renderer_link()?
        .context("attached renderer link")?
        .call(&SessionSnapshotRequest { call_id: None })
        .await?
        .items;
    assert_eq!(
        current_player_queue,
        vec![PlayingItemRef::LibraryTrack {
            library_id: controller_replica
                .global_params
                .test_library()
                .library_id
                .to_string(),
            track: host_items[0].file_uuid.to_string(),
        }]
    );
    let query: Vec<MixQuery> = host_items
        .iter()
        .map(|item| MixQuery {
            operator: "lib::track".into(),
            parameter: controller_file_ids[&item.file_uuid].to_string(),
        })
        .collect();
    let resolve_with_exclusion = |items| ResolveMixQueryRequest {
        call_id: None,
        queries: query.clone(),
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: true,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems { items },
    };
    let excluded = dispatch_gui_request(
        &controller_replica.global_params,
        &resolve_with_exclusion(current_player_queue.clone()),
    )
    .await?
    .context("resolved mix with current queue exclusion")?;
    assert_eq!(excluded.playing_items.len(), 1);
    let included = dispatch_gui_request(
        &controller_replica.global_params,
        &resolve_with_exclusion(Vec::new()),
    )
    .await?
    .context("resolved mix without current queue exclusion")?;
    assert_eq!(included.playing_items.len(), 2);

    queue_session(&host, &[ids[0]]).await?;
    let host_file_ids = file_ids_by_uuid(
        &host.global_params.test_library().main_db,
        &host_items
            .iter()
            .map(|item| item.file_uuid)
            .collect::<Vec<_>>(),
    )
    .await?;
    let host_queue =
        dispatch_gui_request(&host.global_params, &SessionSnapshotRequest { call_id: None })
            .await?
            .context("host player snapshot")?
            .items;
    let mut local_request = resolve_with_exclusion(host_queue);
    local_request.queries = host_items
        .iter()
        .map(|item| MixQuery {
            operator: "lib::track".into(),
            parameter: host_file_ids[&item.file_uuid].to_string(),
        })
        .collect();
    let local_excluded = dispatch_gui_request(&host.global_params, &local_request)
        .await?
        .context("resolved mix with local queue exclusion")?;
    assert_eq!(local_excluded.playing_items.len(), 1);

    let request = MixPlaybackTestRequest {
        queries: query.clone(),
        playback_mode: WirePlaybackMode::Sequential,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: true,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: Vec::new(),
        expected_revision: Some(0),
        cast_to: None,
    };
    let params = request.extract_params(&controller_replica.global_params);
    let response = request
        .handle(params, None, &request)
        .await?
        .context("cast mix query response")?;
    assert_eq!(response.playing_items.len(), 2);
    let expected_items = response
        .playing_items
        .iter()
        .map(|item| match item {
            PlayingItemRef::LibraryTrack { library_id, track } => Ok(PlayingItem::LibraryTrack {
                library_id: Uuid::parse_str(library_id)?,
                track: track.clone(),
            }),
            PlayingItemRef::IndependentFile { .. } => {
                anyhow::bail!("resolved cast item is independent")
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let renderer_status = wait_player(&renderer_replica, |status| {
        status.playlist == expected_items && status.item == expected_items.first().cloned()
    })
    .await?;
    renderer_replica.global_params.player.lock().await.pause();
    wait_player(&renderer_replica, |status| {
        status.state == PlayerPlaybackState::Paused
    })
    .await?;
    assert_eq!(renderer_status.playlist, expected_items);
    assert_eq!(
        controller_replica
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        local_queue
    );
    assert_eq!(
        controller_replica
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        local_revision
    );

    let renderer_revision = renderer_replica
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let mut stale = request;
    stale.expected_revision = Some(renderer_revision.saturating_sub(1));
    let params = stale.extract_params(&controller_replica.global_params);
    let error = match stale.handle(params, None, &stale).await {
        Err(error) => error,
        Ok(_) => anyhow::bail!("stale mix cast unexpectedly succeeded"),
    };
    assert!(
        error
            .chain()
            .any(|cause| cause.is::<ExpectedRequestError>())
    );
    assert_eq!(classify(&error), RequestErrorKind::Conflict);
    assert_eq!(
        renderer_replica
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        renderer_revision
    );
    assert_eq!(
        renderer_replica
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        renderer_status.playlist
    );

    controller_replica
        .global_params
        .node
        .renderer_link()?
        .context("attached renderer link")?
        .close();
    // The exclusion travels with the request, so a gone player cannot fail the resolution.
    let after_detach = dispatch_gui_request(
        &controller_replica.global_params,
        &resolve_with_exclusion(current_player_queue.clone()),
    )
    .await?
    .context("resolved mix after the renderer link closed")?;
    assert_eq!(after_detach.playing_items.len(), 1);

    renderer_replica.global_params.player.lock().await.stop();
    wait_player(&renderer_replica, |status| {
        status.state == PlayerPlaybackState::Stopped
    })
    .await?;
    controller_replica
        .global_params
        .node
        .detach_renderer()
        .await?;
    renderer_replica.global_params.node.shutdown().await?;
    controller_replica.global_params.node.shutdown().await?;
    host.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mix_exclusion_ignores_the_nodes_queue_and_rejects_an_unreadable_excluded_track() -> Result<()> {
    use crate::messages::{MixQuery, MixQueryRequest, PlayingItemRef};
    use crate::request_error::classify;

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let file_id = session_audio(&fixture, "mix-queue-failure.ogg").await?;
    let library_id = fixture.global_params.test_library().library_id;
    fixture
        .global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(
                PlayingItem::LibraryTrack {
                    library_id,
                    track: "not-a-uuid".into(),
                },
                PlaybackSource::LibraryStream,
            )],
            AddMode::AppendToEnd,
        );
    wait_player(&fixture, |status| status.playlist.len() == 1).await?;

    let request = |items| MixQueryRequest {
        call_id: None,
        queries: vec![MixQuery {
            operator: "lib::track".into(),
            parameter: file_id.to_string(),
        }],
        cursor: 0,
        page_size: 30,
        bake_cover_arts: false,
        queue: CallerQueue { items },
        random_seed: None,
    };
    // The node's own queue, readable or not, is not part of a library call.
    let files = dispatch_gui_request(&fixture.global_params, &request(Vec::new()))
        .await?
        .context("mix without exclusions")?
        .files;
    assert_eq!(files.len(), 1);
    // What the caller sends has to be readable.
    let error = match dispatch_gui_request(
        &fixture.global_params,
        &request(vec![PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: "not-a-uuid".into(),
        }]),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => anyhow::bail!("a mix must fail when an excluded track cannot be read"),
    };
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attached_mix_query_rejects_items_outside_renderer_libraries() -> Result<()> {
    use crate::messages::{PlayingItemRef, PlaylistOperateMode};

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let id = session_audio(&controller, "mix-cast-unheld.ogg").await?;
    attach_pair_with_signals(&controller, &renderer, &signals).await?;
    let before_queue = renderer.global_params.player.lock().await.get_playlist();
    let before_revision = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let independent_dir = tempfile::tempdir()?;
    let independent_path = independent_dir.path().join("outside.ogg");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        &independent_path,
    )?;
    let independent_request = MixPlaybackTestRequest {
        queries: Vec::new(),
        playback_mode: WirePlaybackMode::Sequential,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: vec![PlayingItemRef::IndependentFile {
            path: independent_path.to_string_lossy().into_owned(),
        }],
        expected_revision: None,
        cast_to: None,
    };
    let params = independent_request.extract_params(&controller.global_params);
    let error = match independent_request
        .handle(params, None, &independent_request)
        .await
    {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast of an independent file unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::Unavailable);
    assert!(format!("{error:#}").contains("casting files outside a library"));
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        before_queue
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        before_revision
    );

    let item = library_track_for_id(&controller, id).await?;
    let request = MixPlaybackTestRequest {
        queries: Vec::new(),
        playback_mode: WirePlaybackMode::Sequential,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: playing_item_requests(
            &[PlayingItem::from(item)],
            &controller.global_params.library_pool,
            controller.global_params.library_id,
        )
        .await,
        expected_revision: None,
        cast_to: None,
    };
    let params = request.extract_params(&controller.global_params);
    let error = match request.handle(params, None, &request).await {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast of an unheld library unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        before_queue
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        before_revision
    );
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chosen_renderer_cast_preserves_local_playback_and_can_switch_targets() -> Result<()> {
    use crate::backends::remote::RendererLinkUse;
    use crate::messages::{
        CastResolvedItemsRequest, CastTarget, LoadResolvedItemsRequest, MixQuery,
        PlaylistOperateMode, RemotePlaylistRevisionRequest, RendererAttachmentChanged,
    };
    use crate::rpc::dispatch_gui_request;

    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let other_renderer_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer_node = fixture(renderer_dir.path()).await?;
    let other_renderer = fixture(other_renderer_dir.path()).await?;
    let track_ids = [
        session_audio(&host, "target-cast-local-playing.ogg").await?,
        session_audio(&host, "target-cast-next.ogg").await?,
    ];
    let items = track_ids
        .iter()
        .map(|id| library_track_for_id(&host, *id))
        .collect::<Vec<_>>();
    let host_items = future::join_all(items)
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let (host_address, controller_replica) = replicate_host_library(&host, &controller).await?;
    super::pair(&host, &renderer_node).await?;
    super::pair(&renderer_node, &host).await?;
    let (_, renderer) = replicate_host_library_at(&host, &renderer_node, host_address).await?;
    let renderer_address = super::paired_full(&renderer_node, &controller).await?;
    let renderer_info = super::info(&renderer_node).await?;
    controller_replica
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;
    let other_address = super::paired_full(&other_renderer, &controller).await?;
    let other_info = super::info(&other_renderer).await?;
    controller_replica
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &other_info.fingerprint,
            vec![format!("127.0.0.1:{}", other_address.port())],
        )
        .await?;
    let local_file_ids = file_ids_by_uuid(
        &controller_replica.global_params.test_library().main_db,
        &host_items
            .iter()
            .map(|item| item.file_uuid)
            .collect::<Vec<_>>(),
    )
    .await?;
    let local_track_id = local_file_ids[&host_items[0].file_uuid];
    materialize_replica_file(
        &controller_replica,
        local_track_id,
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        "cast-local-playing.ogg",
    )
    .await?;
    queue_session(&controller_replica, &[local_track_id]).await?;
    controller_replica.global_params.player.lock().await.load(0);
    controller_replica.global_params.player.lock().await.play();
    let local_before = wait_player(&controller_replica, |status| {
        status.state == PlayerPlaybackState::Playing && status.item.is_some()
    })
    .await?;
    let make_query = |file_id: i32, cast_to: Option<CastTarget>| MixPlaybackTestRequest {
        queries: vec![MixQuery {
            operator: "lib::track".into(),
            parameter: file_id.to_string(),
        }],
        playback_mode: WirePlaybackMode::Sequential,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: true,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: Vec::new(),
        expected_revision: None,
        cast_to,
    };
    let cast = make_query(
        local_file_ids[&host_items[0].file_uuid],
        Some(CastTarget {
            fingerprint: renderer_info.fingerprint.clone(),
            attach: false,
        }),
    );
    let response = cast
        .handle(
            cast.extract_params(&controller_replica.global_params),
            None,
            &cast,
        )
        .await?
        .context("detached cast response")?;
    assert_eq!(response.playing_items.len(), 1);
    let renderer_status = wait_player(&renderer, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.item.is_some()
            && status.playlist.len() == 1
    })
    .await?;
    assert_eq!(renderer_status.playlist.len(), 1);
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Paused).await?;
    let local_after = controller_replica
        .global_params
        .player
        .lock()
        .await
        .get_status();
    assert_eq!(local_after.item, local_before.item);
    assert_eq!(local_after.state, PlayerPlaybackState::Playing);
    assert!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .is_none()
    );
    assert!(
        !signals
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "RendererAttachmentChanged")
    );
    // A one-off cast link must not show the renderer's session in this device's
    // UI: hold one open while the renderer's state changes.
    let renderer_session_id = renderer.global_params.node.session()?.session_id().await;
    let (cast_link, _) = controller_replica
        .global_params
        .node
        .connect_renderer(&renderer_info.fingerprint, RendererLinkUse::Cast)
        .await?;
    renderer.global_params.player.lock().await.play();
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Playing).await?;
    // The playing status is the barrier before checking that a detached cast emits no renderer status.
    tokio_time::sleep(Duration::from_secs(2)).await;
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Paused).await?;
    cast_link.close();
    drop(cast_link);
    assert!(
        !signals
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(name, payload)| match name.as_str() {
                "PlaylistUpdate" => rinf::deserialize::<PlaylistUpdate>(payload)
                    .ok()
                    .map(|update| update.session_id),
                "PlaybackStatus" => rinf::deserialize::<PlaybackStatus>(payload)
                    .ok()
                    .map(|status| status.session_id),
                _ => None,
            })
            .any(|session_id| session_id == renderer_session_id)
    );

    let attach_cast = make_query(
        local_file_ids[&host_items[1].file_uuid],
        Some(CastTarget {
            fingerprint: renderer_info.fingerprint.clone(),
            attach: true,
        }),
    );
    attach_cast
        .handle(
            attach_cast.extract_params(&controller_replica.global_params),
            None,
            &attach_cast,
        )
        .await?;
    let attached_cast_status = wait_player(&renderer, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.item == Some(PlayingItem::from(host_items[1]))
    })
    .await?;
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Paused).await?;
    assert_eq!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(renderer_info.fingerprint.as_str())
    );
    assert_eq!(
        attached_cast_status.item,
        Some(PlayingItem::from(host_items[1]))
    );

    let append = MixPlaybackTestRequest {
        queries: Vec::new(),
        playback_mode: WirePlaybackMode::Sequential,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: PlaylistOperateMode::PlayNext,
        fallback_playing_items: vec![host_items[1].into()],
        expected_revision: None,
        cast_to: Some(CastTarget {
            fingerprint: renderer_info.fingerprint.clone(),
            attach: false,
        }),
    };
    renderer.global_params.player.lock().await.play();
    let current = wait_player(&renderer, |status| status.state == PlayerPlaybackState::Playing).await?;
    append
        .handle(
            append.extract_params(&controller_replica.global_params),
            None,
            &append,
        )
        .await?;
    let after_next = wait_player(&renderer, |status| status.playlist.len() == 2).await?;
    assert_eq!(after_next.state, PlayerPlaybackState::Playing);
    assert_eq!(after_next.item, current.item);
    assert_eq!(after_next.playlist[1], PlayingItem::from(host_items[1]));
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Paused).await?;

    signals.0.lock().unwrap().clear();
    controller_replica
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert_eq!(
        signals
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "RendererAttachmentChanged")
            .count(),
        1
    );

    signals.0.lock().unwrap().clear();
    renderer.global_params.player.lock().await.play();
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Playing).await?;
    controller_replica
        .global_params
        .node
        .attach_renderer(&other_info.fingerprint)
        .await?;
    let changes = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "RendererAttachmentChanged")
        .filter_map(|(_, payload)| rinf::deserialize::<RendererAttachmentChanged>(payload).ok())
        .collect::<Vec<_>>();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0]
            .renderer
            .as_ref()
            .map(|renderer| renderer.fingerprint.as_str()),
        Some(other_info.fingerprint.as_str())
    );
    wait_controller_count(&renderer.global_params.node, 0).await?;
    wait_player(&renderer, |status| status.state == PlayerPlaybackState::Paused).await?;
    assert_eq!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(other_info.fingerprint.as_str())
    );

    let other_before = other_renderer
        .global_params
        .player
        .lock()
        .await
        .get_playlist();
    let revision_request = RemotePlaylistRevisionRequest {
        call_id: None,
        fingerprint: renderer_info.fingerprint.clone(),
    };
    let revision_response =
        dispatch_gui_request(&controller_replica.global_params, &revision_request)
            .await?
            .context("target renderer queue revision")?;
    assert_eq!(
        revision_response.revision,
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await
    );
    let stale_revision = revision_response
        .revision
        .checked_sub(1)
        .context("renderer queue revision must be nonzero")?;
    let stale_cast = CastResolvedItemsRequest {
        call_id: None,
        fingerprint: renderer_info.fingerprint.clone(),
        load: LoadResolvedItemsRequest {
            call_id: None,
            items: vec![host_items[0].into(), host_items[1].into()],
            start_index: Some(1),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(stale_revision),
            playback_mode: Some(WirePlaybackMode::Shuffle),
            position_seconds: Some(7.25),
            play: LoadPlay::Pause,
        },
    };
    let error = match dispatch_gui_request(&controller_replica.global_params, &stale_cast).await {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast with a stale load revision unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::Conflict);
    let targeted_load = CastResolvedItemsRequest {
        call_id: None,
        fingerprint: renderer_info.fingerprint.clone(),
        load: LoadResolvedItemsRequest {
            call_id: None,
            items: vec![host_items[0].into(), host_items[1].into()],
            start_index: Some(1),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision_response.revision),
            playback_mode: Some(WirePlaybackMode::Shuffle),
            position_seconds: Some(7.25),
            play: LoadPlay::Pause,
        },
    };
    dispatch_gui_request(&controller_replica.global_params, &targeted_load).await?;
    let forwarded_status = wait_player(&renderer, |status| {
        status.playlist
            == vec![
                PlayingItem::from(host_items[0]),
                PlayingItem::from(host_items[1]),
            ]
            && status.state == PlayerPlaybackState::Paused
            && status.index == Some(1)
            && status.playback_mode == PlaybackMode::Shuffle
            && (status.position.as_secs_f64() - 7.25).abs() < 0.1
    })
    .await?;
    assert_eq!(forwarded_status.state, PlayerPlaybackState::Paused);
    assert_eq!(forwarded_status.index, Some(1));
    assert_eq!(forwarded_status.playback_mode, PlaybackMode::Shuffle);
    assert!((forwarded_status.position.as_secs_f64() - 7.25).abs() < 0.1);
    assert_eq!(
        other_renderer
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        other_before
    );

    renderer.global_params.player.lock().await.stop();
    wait_player(&renderer, |status| status.index.is_none()).await?;
    let appended_index = renderer
        .global_params
        .player
        .lock()
        .await
        .get_playlist()
        .len();
    let play_next_when_idle = CastResolvedItemsRequest {
        call_id: None,
        fingerprint: renderer_info.fingerprint.clone(),
        load: LoadResolvedItemsRequest {
            call_id: None,
            items: vec![host_items[1].into()],
            start_index: None,
            mode: PlaylistOperateMode::PlayNext,
            expected_revision: None,
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        },
    };
    dispatch_gui_request(&controller_replica.global_params, &play_next_when_idle).await?;
    wait_player(&renderer, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.index == Some(appended_index)
            && status.item == Some(PlayingItem::from(host_items[1]))
    })
    .await?;

    let before_queue = other_renderer
        .global_params
        .player
        .lock()
        .await
        .get_playlist();
    let before_revision = other_renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let denied_cast = make_query(
        local_file_ids[&host_items[0].file_uuid],
        Some(CastTarget {
            fingerprint: other_info.fingerprint.clone(),
            attach: false,
        }),
    );
    let error = match denied_cast
        .handle(
            denied_cast.extract_params(&controller_replica.global_params),
            None,
            &denied_cast,
        )
        .await
    {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast to a renderer without the library unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
    assert_eq!(
        other_renderer
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        before_queue
    );
    assert_eq!(
        other_renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        before_revision
    );
    assert_eq!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(other_info.fingerprint.as_str())
    );

    controller_replica
        .global_params
        .node
        .detach_renderer()
        .await?;
    renderer.global_params.node.shutdown().await?;
    other_renderer.global_params.node.shutdown().await?;
    host.global_params.node.shutdown().await?;
    Ok(())
}

async fn playlist_update_with_titles(
    signals: &RecordedSignals,
    titles: &[&str],
) -> Result<PlaylistUpdate> {
    wait_until("playlist update with expected title order", || async {
        let update = signals
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|(name, bytes)| {
                (name == "PlaylistUpdate")
                    .then(|| rinf::deserialize::<PlaylistUpdate>(bytes).ok())
                    .flatten()
                    .filter(|update| {
                        update
                            .items
                            .iter()
                            .map(|item| item.title.as_str())
                            .collect::<Vec<_>>()
                            == titles
                    })
            });
        Ok(update)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f2_batches_playlist_metadata_per_library_in_queue_order() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    // The scanner resolves the titles from numbered filenames in each library.
    let a1 = session_audio(&a, "01 - A1.ogg").await?;
    let a2 = session_audio(&a, "02 - A2.ogg").await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    let b1 = session_audio(&b, "01 - B1.ogg").await?;

    let queue = vec![
        PlayingItem::from(library_track_for_id(&a, a1).await?),
        PlayingItem::from(library_track_for_id(&b, b1).await?),
        PlayingItem::from(library_track_for_id(&a, a2).await?),
    ];
    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            queue
                .iter()
                .cloned()
                .map(|item| (item, PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| status.playlist == queue).await?;

    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| {
        status.item == Some(queue[0].clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    a.global_params.player.lock().await.next();
    wait_player(&a, |status| {
        status.item == Some(queue[1].clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    a.global_params.player.lock().await.next();
    wait_player(&a, |status| {
        status.item == Some(queue[2].clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await?;

    let initial = playlist_update_with_titles(&signals, &["A1", "B1", "A2"]).await?;
    assert!(initial.items.iter().all(|item| item.unavailable.is_none()));

    let removed_id = a.global_params.library_id.context("library A")?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|library| library.catalog_library_id() != Some(removed_id));
        Ok(())
    })
    .await?;
    a.global_params.player.lock().await.clear_playlist();
    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            queue
                .iter()
                .cloned()
                .map(|item| (item, PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| status.playlist == queue).await?;

    let unavailable =
        playlist_update_with_titles(&signals, &["Unknown Title", "B1", "Unknown Title"]).await?;
    assert_eq!(
        unavailable
            .items
            .iter()
            .map(|item| item.unavailable.is_some())
            .collect::<Vec<_>>(),
        vec![true, false, true]
    );
    assert!(unavailable.items[0].artist == "Unknown Artist");
    assert!(unavailable.items[0].album == "Unknown Album");
    assert_eq!(unavailable.items[0].duration, 0.0);
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d1_survives_library_switch() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a =
        fixture_with_library_timeout(dir.path(), Uuid::new_v4(), Duration::from_millis(50)).await?;
    a.stop_worker().await?;
    let node = a.global_params.node.clone();
    let connection = Arc::downgrade(&a.global_params.test_library().main_db);
    let id = session_audio(&a, "first.ogg").await?;
    let next = session_audio(&a, "next.ogg").await?;
    queue_session(&a, &[id, next]).await?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |s| s.state == PlayerPlaybackState::Playing).await?;
    a.global_params.player.lock().await.seek(0.1);
    let before = wait_player(&a, |s| s.position >= Duration::from_millis(80)).await?;
    close_ui(&a.global_params).await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    assert!(Arc::ptr_eq(
        &a.global_params.player,
        &b.global_params.player
    ));
    let after = b.global_params.player.lock().await.get_status();
    assert_eq!(before.playlist, after.playlist);
    assert_eq!(before.item, after.item);
    assert_eq!(after.state, PlayerPlaybackState::Playing);
    assert!(after.position >= before.position);
    b.global_params.player.lock().await.pause();
    wait_player(&b, |s| s.state == PlayerPlaybackState::Paused).await?;
    drop(a);
    wait_until("closed UI library eviction", || async {
        Ok(connection.upgrade().is_none().then_some(()))
    })
    .await
    .context("closed UI library was not evicted from pool")?;
    b.global_params.player.lock().await.next();
    wait_player(&b, |s| {
        s.index == Some(1) && s.state == PlayerPlaybackState::Playing
    })
    .await?;
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d2_restart_restores_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "restart.ogg").await?;
    queue_session(&a, &[id, id]).await?;
    {
        let mut player = a.global_params.player.lock().await;
        player.set_playback_mode(PlaybackMode::RepeatAll);
        player.load(1);
    }
    wait_player(&a, |s| s.index == Some(1) && s.ready).await?;
    a.global_params.player.lock().await.seek(0.1);
    let before = wait_player(&a, |s| s.position >= Duration::from_millis(80)).await?;
    let b = reopen_fixture(&a).await?;
    let after = wait_player(&b, |s| {
        s.index == Some(1) && s.ready && s.position >= Duration::from_millis(80)
    })
    .await?;
    assert_eq!(before.playlist, after.playlist);
    assert_eq!(after.playback_mode, PlaybackMode::RepeatAll);
    assert!(after.position.abs_diff(before.position) < Duration::from_millis(50));
    b.global_params.node.shutdown().await
}

#[tokio::test]
async fn registered_open_helper_releases_its_ui_lease() -> Result<()> {
    let node_dir = tempfile::tempdir()?;
    let library_dir = tempfile::tempdir()?;
    let source = super::step("create lease fixture", super::fixture(node_dir.path())).await?;
    let registered = super::step(
        "register lease test library",
        register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                call_id: None,
                path: library_dir
                    .path()
                    .to_str()
                    .context("library path")?
                    .to_owned(),
                alias: "Lease test".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &source.global_params.node,
        ),
    )
    .await?;
    let record = registered.record.context("registered library")?;
    let opened = super::step(
        "open lease test library",
        super::open_registered_library_record(source.global_params.node.clone(), record.record_id),
    )
    .await?;
    let library_id = opened.library_id.context("opened library id")?;
    assert_eq!(
        super::step(
            "read UI leases after open",
            source
                .global_params
                .node
                .library_pool
                .ui_user_count(library_id),
        )
        .await,
        1
    );

    super::step("close lease test library", super::close_ui(&opened)).await?;
    assert_eq!(
        super::step(
            "read UI leases after close",
            source
                .global_params
                .node
                .library_pool
                .ui_user_count(library_id),
        )
        .await,
        0
    );
    super::step(
        "shutdown lease fixture",
        source.global_params.node.shutdown(),
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn playback_boundary_uses_stable_library_tracks_without_row_ids() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let f = fixture(dir.path()).await?;
    let id = session_audio(&f, "boundary.ogg").await?;
    let expected = library_track_for_id(&f, id).await?;
    let library = f.global_params.test_library();
    let parsed = playing_item_from_request(
        PlayingItemRef::LibraryTrack {
            library_id: library.library_id.to_string(),
            track: expected.file_uuid.to_string(),
        },
        Some(library),
    )
    .await?;
    assert_eq!(parsed, PlayingItem::from(expected));
    let current_library_id = library.library_id;
    let other = LibraryItem {
        library_id: Uuid::new_v4(),
        file_uuid: expected.file_uuid,
    };
    let outgoing = playing_item_requests(
        &[
            PlayingItem::from(expected),
            PlayingItem::from(other),
            PlayingItem::Online("https://example.invalid/audio".into(), None),
        ],
        &f.global_params.node.library_pool,
        Some(current_library_id),
    )
    .await;
    assert_eq!(outgoing[0], PlayingItemRef::from(expected));
    assert_eq!(outgoing[1], PlayingItemRef::from(other));
    assert_eq!(
        outgoing[2],
        PlayingItemRef::IndependentFile {
            path: String::new()
        }
    );

    let without_ui = playing_item_requests(
        &[PlayingItem::from(expected)],
        &f.global_params.node.library_pool,
        None,
    )
    .await;
    assert_eq!(without_ui[0], PlayingItemRef::from(expected));

    let other_request = PlayingItemRef::from(other);
    assert_eq!(
        playing_item_from_request(other_request, None).await?,
        PlayingItem::from(other),
    );
    let malformed = PlayingItemRef::LibraryTrack {
        library_id: "bad-library-id".into(),
        track: "not-interpreted-at-the-wire-boundary".into(),
    };
    let error = playing_item_from_request(malformed, None)
        .await
        .unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    f.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playlist_update_uses_library_track_after_switching_ui_library() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let a_id = session_audio(&a, "boundary-switch.ogg").await?;
    let a_item = library_track_for_id(&a, a_id).await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    assert_ne!(a_item.library_id, b.global_params.test_library().library_id);

    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(PlayingItem::from(a_item), PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| status.playlist == [PlayingItem::from(a_item)]).await?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| status.item == Some(PlayingItem::from(a_item))).await?;
    let update = wait_until("playlist update with stable library item", || async {
        let update = signals
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|(name, bytes)| {
                (name == "PlaylistUpdate")
                    .then(|| rinf::deserialize::<PlaylistUpdate>(bytes).ok())
                    .flatten()
                    .filter(|update| {
                        update.items.iter().any(|item| {
                            matches!(&item.item, PlayingItemRef::LibraryTrack { library_id, track }
                                        if library_id == &a_item.library_id.to_string()
                                            && track == &a_item.file_uuid.to_string())
                        })
                    })
            });
        Ok(update)
    })
    .await?;
    assert_eq!(update.items.len(), 1);
    assert_eq!(update.items[0].item, PlayingItemRef::from(a_item));
    let status = wait_until("playback status with stable library item", || async {
        let status = signals
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|(name, bytes)| {
                (name == "PlaybackStatus")
                    .then(|| rinf::deserialize::<PlaybackStatus>(bytes).ok())
                    .flatten()
                    .filter(|status| {
                        status
                            .item_request
                            .as_ref()
                            .is_some_and(|request| request == &PlayingItemRef::from(a_item))
                    })
            });
        Ok(status)
    })
    .await?;
    assert_eq!(status.item_request.unwrap(), PlayingItemRef::from(a_item));
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d4_queue_queries_and_download_rules_are_library_scoped() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let aid = session_audio(&a, "queued.ogg").await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    let bid = session_audio(&b, "other.ogg").await?;
    assert_eq!(aid, bid, "exercise colliding catalog integer IDs");
    queue_session(&a, &[aid]).await?;
    for (f, expected) in [(&a, vec![aid]), (&b, vec![])] {
        let library = f.global_params.test_library();
        let file = media_files::Entity::find()
            .one(&*library.main_db)
            .await?
            .context("download track")?;
        let managed = Path::new(&library.media_root).join("Rune Sync");
        fs::create_dir_all(&managed)?;
        let target = managed.join(&file.file_name);
        fs::copy(
            Path::new(&library.media_root).join(&file.file_name),
            &target,
        )?;
        media_files::Entity::update_many()
            .col_expr(Column::Directory, Expr::value("Rune Sync"))
            .filter(Column::Id.eq(file.id))
            .exec(&*library.main_db)
            .await?;
        let queue = f
            .global_params
            .node
            .session_queue_for_library(library.library_id)
            .await?;
        assert_eq!(queue, expected);
        let queries = vec![("lib::queue".into(), "true".into())];
        let rows = query_mix_media_files(
            &library.main_db,
            &library.recommend_db,
            queries.clone(),
            0,
            100,
            &queue,
        )
        .await?;
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), expected);
        set_download_rule(&library.main_db, queries, None, true).await?;
        // Cleanup uses the actual download-rule query path. The queued file must remain selected.
        let report = cleanup_unselected_downloads(
            &library.main_db,
            &library.recommend_db,
            &library.fsio,
            Path::new(&library.media_root),
            &library.library_sync.record_store(),
            &queue,
        )
        .await?;
        assert_eq!(report.delete_failures, 0);
        assert_eq!(report.removed, usize::from(expected.is_empty()));
        assert_eq!(target.exists(), !expected.is_empty());
    }
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f4_nodes_with_matching_sqlite_filenames_keep_queues_separate() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let aid = session_audio(&a, "queue-a.ogg").await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    let bid = session_audio(&b, "queue-b.ogg").await?;
    assert_eq!(aid, bid, "exercise colliding catalog integer IDs");

    let filename = |fixture: &Fixture| {
        fixture
            .global_params
            .test_library()
            .main_db
            .get_sqlite_connection_pool()
            .connect_options()
            .get_filename()
            .file_name()
            .map(ToOwned::to_owned)
    };
    assert_eq!(filename(&a), filename(&b));

    queue_session(&a, &[aid]).await?;
    for (fixture, expected) in [(&a, vec![aid]), (&b, vec![])] {
        let library = fixture.global_params.test_library();
        let queue = fixture
            .global_params
            .node
            .session_queue_for_library(library.library_id)
            .await?;
        assert_eq!(queue, expected);
        let rows = query_mix_media_files(
            &library.main_db,
            &library.recommend_db,
            vec![("lib::queue".into(), "true".into())],
            0,
            10,
            &queue,
        )
        .await?;
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), expected);
    }
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d5_stats_follow_item_library() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "stats.ogg").await?;
    queue_session(&a, &[id]).await?;
    close_ui(&a.global_params).await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    session_audio(&b, "collision.ogg").await?;
    let duration = media_files::Entity::find_by_id(id)
        .one(&*a.global_params.test_library().main_db)
        .await?
        .context("stats track")?
        .duration
        .to_f64()
        .context("stats track duration")?;
    b.global_params.player.lock().await.switch(0);
    wait_player(&b, |status| status.state == PlayerPlaybackState::Playing).await?;
    b.global_params
        .player
        .lock()
        .await
        .seek((duration - 0.1).max(0.0));
    b.global_params
        .player
        .lock()
        .await
        .status_after_pending()
        .await;
    wait_until("the played-through statistic", || async {
        let rows = media_file_stats::Entity::find()
            .all(&*a.global_params.test_library().main_db)
            .await?;
        Ok(rows.iter().any(|s| s.played_through > 0).then_some(()))
    })
    .await?;
    let wrong = media_file_stats::Entity::find()
        .all(&*b.global_params.test_library().main_db)
        .await?;
    assert!(wrong.iter().all(|s| s.played_through == 0));
    a.global_params.node.shutdown().await
}

/// Plays `completions` queued library tracks through to their end on the real player, with the
/// session's completion handler blocked on the shared scrobbler while `hold_handler` is set, and
/// returns how many times each completed track was recorded as played through.
async fn played_through_counts(completions: usize, hold_handler: bool) -> Result<Vec<i32>> {
    let dir = tempfile::tempdir()?;
    let f = fixture(dir.path()).await?;
    let mut ids = Vec::new();
    // One more track than completions, so the player is playing it after the last completion.
    for index in 0..=completions {
        ids.push(session_audio(&f, &format!("completion-{index}.ogg")).await?);
    }
    queue_session(&f, &ids).await?;
    let library = f.global_params.test_library();
    let mut tracks = Vec::new();
    for id in &ids[..completions] {
        tracks.push(library_track_for_id(&f, *id).await?.file_uuid.to_string());
    }
    let duration = media_files::Entity::find_by_id(ids[0])
        .one(&*library.main_db)
        .await?
        .context("completion track")?
        .duration
        .to_f64()
        .context("completion track duration")?;
    let counts = || async {
        let rows = media_file_stats::Entity::find().all(&*library.main_db).await?;
        Ok::<_, anyhow::Error>(
            tracks
                .iter()
                .map(|track| {
                    rows.iter()
                        .filter(|row| &row.track_uuid == track)
                        .map(|row| row.played_through)
                        .sum::<i32>()
                })
                .collect::<Vec<_>>(),
        )
    };
    let scrobbler = f.global_params.scrobbler.clone();
    let held = if hold_handler {
        Some(scrobbler.lock().await)
    } else {
        None
    };
    f.global_params.player.lock().await.switch(0);
    for index in 0..completions {
        wait_player(&f, |status| {
            status.index == Some(index) && status.state == PlayerPlaybackState::Playing
        })
        .await?;
        f.global_params
            .player
            .lock()
            .await
            .seek((duration - 0.1).max(0.0));
        if index == 0 && hold_handler {
            // The handler has taken the first completion and now waits for the scrobbler.
            wait_until("the first completion to be handled", || async {
                Ok((counts().await?[0] == 1).then_some(()))
            })
            .await?;
        }
    }
    wait_player(&f, |status| {
        status.index == Some(completions) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    f.global_params.player.lock().await.pause();
    wait_player(&f, |status| status.state == PlayerPlaybackState::Paused).await?;
    drop(held);
    // Completions are handled in order, so the last one being recorded means the earlier
    // ones were handled before it.
    wait_until("the last completion to be handled", || async {
        Ok((counts().await?[completions - 1] > 0).then_some(()))
    })
    .await?;
    let result = counts().await?;
    f.global_params.node.shutdown().await?;
    Ok(result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completions_queued_behind_a_held_handler_are_each_recorded_once() -> Result<()> {
    // The first completion holds the handler; seventeen more wait behind it.
    let counts = played_through_counts(18, true).await?;
    assert_eq!(counts, vec![1; 18]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completions_handled_as_they_arrive_are_each_recorded_once() -> Result<()> {
    let counts = played_through_counts(3, false).await?;
    assert_eq!(counts, vec![1; 3]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d6_removed_library_is_unavailable_without_crash() -> Result<()> {
    use crate::messages::PlaylistUnavailableReason;
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let aid = session_audio(&a, "removed.ogg").await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    let bid = session_audio(&b, "available.ogg").await?;
    queue_session(&a, &[aid]).await?;
    let good = PlayingItem::from(library_track_for_id(&b, bid).await?);
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(good.clone(), PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    wait_player(&b, |s| s.playlist.len() == 2).await?;
    let bad = a.global_params.player.lock().await.get_playlist()[0].clone();
    let descriptor = a.global_params.test_library().descriptor.clone();
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|lib| lib.catalog_library_id() != a.global_params.library_id);
        Ok(())
    })
    .await?;
    b.global_params.player.lock().await.switch(0);
    let status = wait_player(&b, |s| {
        s.item == Some(good.clone()) && s.state == PlayerPlaybackState::Playing
    })
    .await?;
    assert!(status.unavailable.contains(&bad));
    assert_eq!(
        status.unavailable_reasons.get(&bad),
        Some(&UnavailableReason::LibraryUnavailable)
    );
    assert_eq!(status.playlist.len(), 2);
    wait_until("unavailable playlist update", || async {
        let reported = signals.0.lock().unwrap().iter().any(|(name, bytes)| {
            name == "PlaylistUpdate"
                && rinf::deserialize::<PlaylistUpdate>(bytes).is_ok_and(|update| {
                    update.items.len() == 2
                        && update.items[0].unavailable
                            == Some(PlaylistUnavailableReason::LibraryUnavailable)
                        && update.items[1].unavailable.is_none()
                })
        });
        Ok(reported.then_some(()))
    })
    .await?;
    assert!(
        signals
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "CrashResponse")
    );
    LibraryRegistry::update(dir.path(), |registry| {
        registry.libraries.push(descriptor);
        Ok(())
    })
    .await?;
    b.global_params.player.lock().await.load(0);
    let recovered = wait_player(&b, |s| s.item == Some(bad.clone()) && s.ready).await?;
    assert!(!recovered.unavailable.contains(&bad));
    b.global_params.player.lock().await.play();
    let playing = wait_player(&b, |s| {
        s.item == Some(bad.clone()) && s.state == PlayerPlaybackState::Playing
    })
    .await?;
    assert!(!playing.unavailable.contains(&bad));
    wait_until("recovered playlist update", || async {
        let reported = signals.0.lock().unwrap().iter().any(|(name, bytes)| {
            name == "PlaylistUpdate"
                && rinf::deserialize::<PlaylistUpdate>(bytes).is_ok_and(|update| {
                    update.items.len() == 2 && update.items[0].unavailable.is_none()
                })
        });
        Ok(reported.then_some(()))
    })
    .await?;
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f1_repeat_all_wraps_after_failed_last_item() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let bad_id = session_audio(&a, "repeat-bad.ogg").await?;
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    let good_id = session_audio(&b, "repeat-good.ogg").await?;
    let good = PlayingItem::from(library_track_for_id(&b, good_id).await?);
    let bad = PlayingItem::from(library_track_for_id(&a, bad_id).await?);
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![
                (good.clone(), PlaybackSource::LibraryStream),
                (bad.clone(), PlaybackSource::LibraryStream),
            ],
            AddMode::AppendToEnd,
        );
    wait_player(&b, |s| s.playlist.len() == 2).await?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|lib| lib.catalog_library_id() != a.global_params.library_id);
        Ok(())
    })
    .await?;
    b.global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    b.global_params.player.lock().await.switch(1);
    wait_player(&b, |s| {
        s.item == Some(good.clone()) && s.index == Some(0) && s.state == PlayerPlaybackState::Playing
    })
    .await?;
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f1_all_failed_items_stop_without_hanging() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let ids = [
        session_audio(&a, "all-fail-a.ogg").await?,
        session_audio(&a, "all-fail-b.ogg").await?,
    ];
    let b = fixture_open_local_library(&a, bdir.path()).await?;
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            future::join_all(ids.iter().map(|id| library_track_for_id(&a, *id)))
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&b, |s| s.playlist.len() == ids.len()).await?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|lib| lib.catalog_library_id() != a.global_params.library_id);
        Ok(())
    })
    .await?;
    b.global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    b.global_params.player.lock().await.switch(0);
    let status = wait_until("all failed tracks to stop playback", || async {
        let status = b.global_params.player.lock().await.get_status();
        Ok(
            (status.unavailable.len() == ids.len() && status.state == PlayerPlaybackState::Stopped)
                .then_some(status),
        )
    })
    .await?;
    assert_eq!(status.unavailable.len(), ids.len());
    let settled = b.global_params.player.lock().await.get_status();
    assert_eq!(settled.item, None);
    assert_eq!(settled.index, None);
    // The stopped status is the barrier before checking that failed items do not restart.
    sleep(Duration::from_millis(300)).await;
    let after_quiet_period = b.global_params.player.lock().await.get_status();
    assert_eq!(after_quiet_period.state, PlayerPlaybackState::Stopped);
    assert_eq!(after_quiet_period.index, settled.index);
    assert_eq!(after_quiet_period.item, settled.item);
    assert_eq!(after_quiet_period.playlist, settled.playlist);
    assert_eq!(after_quiet_period.unavailable, settled.unavailable);
    assert_eq!(after_quiet_period.unavailable.len(), ids.len());
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleted_queued_uuid_is_unreadable_and_playback_advances() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let f = fixture(dir.path()).await?;
    let ids = [
        session_audio(&f, "delete-first.ogg").await?,
        session_audio(&f, "delete-middle.ogg").await?,
        session_audio(&f, "delete-last.ogg").await?,
    ];
    let queued = future::join_all(ids.iter().map(|id| library_track_for_id(&f, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    f.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            queued
                .iter()
                .copied()
                .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&f, |status| status.playlist.len() == 3).await?;
    media_files::Entity::delete_by_id(ids[1])
        .exec(&*f.global_params.test_library().main_db)
        .await?;
    f.global_params.player.lock().await.switch(0);
    wait_player(&f, |status| {
        status.item == Some(PlayingItem::from(queued[0])) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    f.global_params.player.lock().await.next();
    let advanced = wait_player(&f, |status| {
        status.item == Some(PlayingItem::from(queued[2]))
            && status.state == PlayerPlaybackState::Playing
            && status.unavailable.contains(&PlayingItem::from(queued[1]))
    })
    .await?;
    assert_eq!(
        advanced
            .unavailable_reasons
            .get(&PlayingItem::from(queued[1])),
        Some(&UnavailableReason::Unreadable)
    );
    f.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scenario_revoking_host_grant_stops_remote_playback() -> Result<()> {
    use crate::backends::remote::connect_remote;
    use crate::messages::{PlaylistUnavailableReason, PlaylistUpdate};
    use crate::server::{sync, sync_worker};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let c_dir = tempfile::tempdir()?;
    let b_signals = Arc::new(RecordedSignals::default());
    let a = fixture(a_dir.path()).await?;
    let b = fixture_with_broadcaster(
        b_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        b_signals.clone(),
    )
    .await?;
    let c = fixture(c_dir.path()).await?;
    let c_tracks = ["revoke-c1.ogg", "revoke-c2.ogg"];
    let mut c_ids = Vec::new();
    let mut c_track_uuids = Vec::new();
    for track in c_tracks {
        let source = if track == "revoke-c1.ogg" {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg")
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_1.ogg")
        };
        let target = c_dir.path().join(track);
        fs::copy(source, &target)?;
        scan_audio_file(
            &c.global_params.test_library().fsio,
            &c.global_params.test_library().main_db,
            &c.global_params.node_id,
            c_dir.path(),
            &target,
            &c.global_params.test_library().library_sync.record_store(),
        )
        .await?;
        let row = media_files::Entity::find()
            .filter(Column::FileName.eq(track))
            .one(&*c.global_params.test_library().main_db)
            .await?
            .context("host fixture track")?;
        c_ids.push(row.id);
        c_track_uuids.push(row.source_uuid);
    }
    let b_local_id = session_audio(&b, "revoke-b-local.ogg").await?;

    let (c_addr, b_replica) = peer_step_with_timeout(
        "replicate C library on B",
        TEST_DEADLINE,
        replicate_host_library(&c, &b),
    )
    .await?;
    let c_info = info(&c).await?;
    let b_info = info(&b).await?;
    let c_library_id = c.global_params.library_id.context("C library id")?;
    let b_c_ids = peer_step("resolve C replica track ids on B", async {
        let mut ids = Vec::new();
        for track_uuid in &c_track_uuids {
            ids.push(
                media_files::Entity::find()
                    .filter(Column::SourceUuid.eq(track_uuid))
                    .one(&*b_replica.global_params.test_library().main_db)
                    .await?
                    .context("replicated host track")?
                    .id,
            );
        }
        Ok(ids)
    })
    .await?;
    for id in &b_c_ids {
        let row = media_files::Entity::find_by_id(*id)
            .one(&*b_replica.global_params.test_library().main_db)
            .await?
            .context("replicated host row")?;
        assert!(!row.local_available, "C audio must remain remote only on B");
    }

    let a_addr = peer_step("pair A with B", paired_full(&b, &a)).await?;
    let mut controller = peer_step("connect A controller socket", async {
        Ok(connect_remote(
            "127.0.0.1",
            a_addr.port(),
            &a.global_params.config_path,
            a.global_params.cert_validator.read().await.clone(),
            &a.global_params.node.sync_tokens,
        )
        .await?)
    })
    .await?;
    let mut grant = c
        .global_params
        .node
        .access_policy
        .grants(&b_info.fingerprint)?;
    grant
        .connection
        .libraries
        .insert(c_library_id, ConnectionLevel::None);
    grant.sync.libraries.insert(c_library_id, SyncLevel::None);
    let c_blob = catalog_track(&c.global_params.test_library().main_db, c_ids[0])
        .await?
        .context("host audio blob")?;
    let (base, b_client) = peer_step("authenticate B audio client before playback", async {
        sync_worker::authenticated_client(
            &b.global_params.node,
            &b_info.fingerprint,
            &b.private_key,
            &c_info.fingerprint,
            "127.0.0.1",
            c_addr.port(),
        )
        .await
    })
    .await?;
    let audio_url = format!(
        "{}/sync/blobs/{}?track_id={}",
        sync::library_base(&base, c_library_id),
        c_blob.1.digest,
        c_blob.0
    );
    peer_step("verify B can request C audio before playback", async {
        let response = b_client
            .get(&audio_url)
            .header(header::RANGE, "bytes=0-31")
            .send()
            .await?;
        anyhow::ensure!(
            response.status() == ReqwestStatusCode::PARTIAL_CONTENT,
            "pre-revocation range request returned {}",
            response.status()
        );
        Ok(())
    })
    .await?;
    let queue = future::join_all(b_c_ids.iter().map(|id| library_track_for_id(&c, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
        .collect();
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(queue, AddMode::AppendToEnd);
    b.global_params.player.lock().await.switch(0);
    let first = PlayingItem::from(LibraryItem {
        library_id: c_library_id,
        file_uuid: library_track_for_id(&c, b_c_ids[0]).await?.file_uuid,
    });
    let second = PlayingItem::from(LibraryItem {
        library_id: c_library_id,
        file_uuid: library_track_for_id(&c, b_c_ids[1]).await?.file_uuid,
    });
    let playing_result = peer_step_with_timeout("B starts remote C track", TEST_DEADLINE, async {
        wait_until("remote C track to start", || async {
            let status = b.global_params.player.lock().await.get_status();
            Ok(
                (status.item == Some(first.clone()) && status.state == PlayerPlaybackState::Playing)
                    .then_some(status),
            )
        })
        .await
    })
    .await;
    let playing = match playing_result {
        Ok(playing) => playing,
        Err(error) => {
            let status = b.global_params.player.lock().await.get_status();
            let crashed = b_signals
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|(name, _)| name == "CrashResponse");
            return Err(error.context(format!(
                "latest B player status: {status:?}; crash response recorded: {crashed}"
            )));
        }
    };
    assert_eq!(playing.index, Some(0));
    assert!(
        playing
            .path
            .as_deref()
            .is_some_and(|path| path.as_os_str().is_empty())
    );
    let first_status_item = first.to_string();
    peer_step("revoke B read grant on C library", async {
        c.global_params
            .node
            .access_policy
            .set(&b_info.fingerprint, grant)?;
        Ok(())
    })
    .await?;
    peer_step_with_timeout("B stops after C stream revocation", TEST_DEADLINE, async {
        wait_until("player to settle after revocation", || async {
            let status = b.global_params.player.lock().await.get_status();
            Ok((status.state == PlayerPlaybackState::Stopped
                && status.item.is_none()
                && status.unavailable.contains(&first)
                && status.unavailable.contains(&second))
            .then_some(status))
        })
        .await
    })
    .await?;
    peer_step_with_timeout("A receives B playing status", TEST_DEADLINE, async {
        loop {
            let frame = controller
                .next()
                .await
                .context("controller socket closed")??;
            if let Some((name, payload, _)) = decode_message(&frame.into_data())
                && name == "PlaybackStatus"
            {
                let status: PlaybackStatus = rinf::deserialize(&payload)?;
                if status.state == PlaybackState::Playing
                    && status.item.as_deref() == Some(first_status_item.as_str())
                {
                    return Ok(());
                }
            }
        }
    })
    .await?;
    let controller_update = peer_step_with_timeout(
        "A receives stopped status or unavailable playlist update",
        TEST_DEADLINE,
        async {
            loop {
                let frame = controller
                    .next()
                    .await
                    .context("controller socket closed")??;
                if let Some((name, payload, _)) = decode_message(&frame.into_data()) {
                    if name == "PlaybackStatus" {
                        let status: PlaybackStatus = rinf::deserialize(&payload)?;
                        if status.state == PlaybackState::Stopped && status.item.is_none() {
                            return Ok(());
                        }
                    } else if name == "PlaylistUpdate" {
                        let update: PlaylistUpdate = rinf::deserialize(&payload)?;
                        if update
                            .items
                            .iter()
                            .filter(|item| {
                                item.unavailable == Some(PlaylistUnavailableReason::AccessRevoked)
                            })
                            .count()
                            == 2
                        {
                            return Ok(());
                        }
                    }
                }
            }
        },
    )
    .await;
    if let Err(error) = controller_update {
        let local_statuses: Vec<_> = b_signals
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "PlaybackStatus")
            .filter_map(|(_, payload)| rinf::deserialize::<PlaybackStatus>(payload).ok())
            .map(|status| (status.state, status.item, status.ready))
            .collect();
        return Err(error.context(format!(
            "B's locally recorded playback status signals: {local_statuses:?}"
        )));
    }
    peer_step("C denies a new ranged audio request from B", async {
        let response = b_client
            .get(&audio_url)
            .header(header::RANGE, "bytes=0-31")
            .send()
            .await?;
        anyhow::ensure!(
            response.status() == ReqwestStatusCode::FORBIDDEN
                || response.status() == ReqwestStatusCode::UNAUTHORIZED,
            "revoked range request returned {}",
            response.status()
        );
        Ok(())
    })
    .await?;
    assert!(
        b_signals
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "CrashResponse"),
        "expected access revocation must not crash the player"
    );

    // A playback that stops on its own must surface: the failure reaches the application log,
    // and since no pending request answer carries it, a typed request failure reaches the
    // listeners.
    peer_step_with_timeout(
        "B records the playback failure in its application log",
        TEST_DEADLINE,
        async {
            wait_until("application log entry for the revoked playback", || async {
                let logs = list_log(&b_replica.global_params.test_library().main_db, 0, 20).await?;
                Ok(logs
                    .iter()
                    .any(|entry| entry.domain == "playback")
                    .then_some(()))
            })
            .await
        },
    )
    .await?;
    peer_step("B broadcast the playback failure as a typed request failure", async {
        // The log row is written before the broadcast, so the row being present does not mean
        // the broadcast has landed yet: poll for it instead of reading once.
        let failure = wait_until("the typed playback failure broadcast", || async {
            let failure = b_signals
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, _)| name == "RequestFailedResponse")
                .filter_map(|(_, payload)| rinf::deserialize::<RequestFailedResponse>(payload).ok())
                .find(|failure| failure.request == "Playback");
            Ok(failure)
        })
        .await?;
        anyhow::ensure!(
            failure.kind == RequestErrorKind::PermissionDenied,
            "access revocation surfaced as {:?}, not permission denied",
            failure.kind
        );
        Ok(())
    })
    .await?;

    let local = PlayingItem::from(library_track_for_id(&b, b_local_id).await?);
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(local.clone(), PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    b.global_params.player.lock().await.switch(2);
    peer_step_with_timeout(
        "B player continues with local audio",
        TEST_DEADLINE,
        async {
            wait_until("local track to resume", || async {
                let status = b.global_params.player.lock().await.get_status();
                Ok(
                    (status.item == Some(local.clone()) && status.state == PlayerPlaybackState::Playing)
                        .then_some(()),
                )
            })
            .await
        },
    )
    .await?;
    peer_step_with_timeout(
        "A receives post-revocation playback update",
        TEST_DEADLINE,
        async {
            loop {
                let frame = tokio_time::timeout(TEST_DEADLINE, controller.next())
                    .await
                    .context("controller update timed out")?
                    .context("controller socket closed")??;
                if let Some((name, payload, _)) = decode_message(&frame.into_data())
                    && name == "PlaybackStatus"
                {
                    let status: PlaybackStatus = rinf::deserialize(&payload)?;
                    if status.item == Some(local.to_string()) {
                        return Ok(());
                    }
                }
            }
        },
    )
    .await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    c.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cast_preflight_and_load_check_renderer_library_grants_over_websocket() -> Result<()> {
    use crate::backends::remote::connect_remote;
    use crate::messages::{
        CastLibraryState, CastPreflightResponse, CastResolvedItemsResponse, HandoffBlockId,
        HandoffLoadRequest, LoadResolvedItemsRequest, PlaylistOperateMode, RemoteRequestFailed,
    };

    let host_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let host = fixture(host_dir.path()).await?;
    let renderer = fixture_with_broadcaster(
        renderer_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals,
    )
    .await?;
    let controller = fixture(controller_dir.path()).await?;

    let host_ids = [
        session_audio(&host, "cast-first.ogg").await?,
        session_audio(&host, "cast-missing.ogg").await?,
        session_audio(&host, "cast-third.ogg").await?,
    ];
    let host_items =
        future::join_all(host_ids.iter().map(|id| library_track_for_id(&host, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let (_, renderer_replica) = replicate_host_library(&host, &renderer).await?;
    let renderer_library = renderer_replica
        .global_params
        .node
        .library_pool
        .acquire_for_request(host.global_params.library_id.context("host library id")?)
        .await?;
    let target_file_ids =
        file_ids_by_uuid(&renderer_library.main_db, &[host_items[1].file_uuid]).await?;
    let target_file_id = target_file_ids
        .get(&host_items[1].file_uuid)
        .copied()
        .context("middle cast item missing from replica fixture")?;
    media_files::Entity::delete_by_id(target_file_id)
        .exec(renderer_library.main_db.as_ref())
        .await?;
    let renderer_info = info(&renderer).await?;
    let host_library_id = host.global_params.library_id.context("host library id")?;

    let controller_addr = paired_full(&renderer, &controller).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", controller_addr.port())],
        )
        .await?;
    let mut socket = connect_remote(
        "127.0.0.1",
        controller_addr.port(),
        &controller.global_params.config_path,
        controller.global_params.cert_validator.read().await.clone(),
        &controller.global_params.node.sync_tokens,
    )
    .await?;

    let preflight = CastPreflightRequest {
        call_id: None,
        library_ids: vec![host_library_id.to_string()],
    };
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&preflight)?,
    )
    .await?;
    assert_eq!(name, "CastPreflightResponse");
    let preflight: CastPreflightResponse = rinf::deserialize(&payload)?;
    assert_eq!(preflight.libraries[0].state, CastLibraryState::Ready);

    let cast_items = host_items
        .iter()
        .map(|item| (*item).into())
        .collect::<Vec<_>>();
    let cast_request = CastResolvedItemsRequest {
        call_id: None,
        fingerprint: renderer_info.fingerprint.clone(),
        load: LoadResolvedItemsRequest {
            call_id: None,
            items: cast_items.clone(),
            start_index: Some(1),
            mode: PlaylistOperateMode::Replace,
            expected_revision: None,
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        },
    };
    let response: CastResolvedItemsResponse =
        dispatch_gui_request(&controller.global_params, &cast_request)
            .await?
            .context("cast result")?;
    assert_eq!(response.result.left_out_items.len(), 1);
    assert_eq!(response.result.left_out_items, vec![host_items[1].into()]);
    wait_player(&renderer, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.item == Some(PlayingItem::from(host_items[2]))
            && status.playlist
                == vec![
                    PlayingItem::from(host_items[0]),
                    PlayingItem::from(host_items[2]),
                ]
    })
    .await?;
    renderer.stop_worker().await?;
    host.stop_worker().await?;
    let revision = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let queue_before_unavailable_cast = renderer
        .global_params
        .player
        .lock()
        .await
        .get_status()
        .playlist;
    let (name, payload, _) = controller_request(
        &mut socket,
        "LoadResolvedItemsRequest",
        rinf::serialize(&LoadResolvedItemsRequest {
            call_id: None,
            items: vec![host_items[1].into()],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::Unavailable
    );
    assert_eq!(
        renderer
            .global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist,
        queue_before_unavailable_cast
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );

    let handoff = HandoffLoadRequest {
        block: HandoffBlockId {
            uuid: Uuid::new_v4().to_string(),
        },
        load: LoadResolvedItemsRequest {
            call_id: None,
            items: vec![host_items[0].into(), host_items[1].into()],
            start_index: Some(0),
            mode: PlaylistOperateMode::PlayNext,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        },
        call_id: None,
    };
    let (name, payload, _) = controller_request(
        &mut socket,
        "HandoffLoadRequest",
        rinf::serialize(&handoff)?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::NotFound
    );
    assert_eq!(
        renderer
            .global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist,
        queue_before_unavailable_cast
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );

    let local_cast_id = session_audio(&renderer, "cast-local.ogg").await?;
    let local_cast_item = library_track_for_id(&renderer, local_cast_id).await?;
    let stale_load = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![local_cast_item.into()],
        expected_revision: Some(revision.saturating_sub(1)),
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        playback_mode: None,
        position_seconds: None,
        play: LoadPlay::Auto,
    };
    let queue_before_stale = renderer.global_params.player.lock().await.get_playlist();
    let (name, payload, _) = controller_request(
        &mut socket,
        "LoadResolvedItemsRequest",
        rinf::serialize(&stale_load)?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::Conflict
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_stale
    );

    let not_held = PlayingItemRef::LibraryTrack {
        library_id: controller
            .global_params
            .library_id
            .context("controller library id")?
            .to_string(),
        track: host_items[0].file_uuid.to_string(),
    };
    let not_held_library_id = match &not_held {
        PlayingItemRef::LibraryTrack { library_id, .. } => library_id.clone(),
        PlayingItemRef::IndependentFile { .. } => unreachable!(),
    };
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&CastPreflightRequest {
            call_id: None,
            library_ids: vec![not_held_library_id.clone()],
        })?,
    )
    .await?;
    let preflight: CastPreflightResponse = rinf::deserialize(&payload)?;
    assert_eq!(name, "CastPreflightResponse");
    assert_eq!(preflight.libraries[0].state, CastLibraryState::NotHeld);
    let queue_before_denial = renderer.global_params.player.lock().await.get_playlist();
    let (name, payload, _) = controller_request(
        &mut socket,
        "LoadResolvedItemsRequest",
        rinf::serialize(&LoadResolvedItemsRequest {
            call_id: None,
            items: vec![not_held.clone()],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    let failure = rinf::deserialize::<RemoteRequestFailed>(&payload)?;
    assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
    assert_eq!(
        failure.cause,
        Some(RequestFailureCause::CastNotPossible {
            library_id: not_held_library_id,
            state: CastLibraryState::NotHeld,
        })
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_denial
    );

    set_library_grant(
        &host,
        &renderer_info.fingerprint,
        host_library_id,
        SyncLevel::None,
    )?;
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&preflight_request(host_library_id))?,
    )
    .await?;
    assert_eq!(name, "CastPreflightResponse");
    assert_eq!(
        rinf::deserialize::<CastPreflightResponse>(&payload)?.libraries[0].state,
        CastLibraryState::AccessRevoked
    );
    let queue_before_revoked_load = renderer.global_params.player.lock().await.get_playlist();
    let (name, payload, _) = controller_request(
        &mut socket,
        "LoadResolvedItemsRequest",
        rinf::serialize(&LoadResolvedItemsRequest {
            call_id: None,
            items: cast_items.clone(),
            start_index: None,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::PermissionDenied
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_revoked_load
    );
    host.global_params.node.shutdown().await?;
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&preflight_request(host_library_id))?,
    )
    .await?;
    assert_eq!(name, "CastPreflightResponse");
    assert_eq!(
        rinf::deserialize::<CastPreflightResponse>(&payload)?.libraries[0].state,
        CastLibraryState::Unreachable
    );
    let (name, payload, _) = controller_request(
        &mut socket,
        "LoadResolvedItemsRequest",
        rinf::serialize(&LoadResolvedItemsRequest {
            call_id: None,
            items: cast_items,
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::Unavailable
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_denial
    );

    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&CastPreflightRequest {
            call_id: None,
            library_ids: vec!["malformed".into()],
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::InvalidInput
    );

    let (name, payload, _) = controller_request(
        &mut socket,
        "LoadResolvedItemsRequest",
        rinf::serialize(&LoadResolvedItemsRequest {
            call_id: None,
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: host_library_id.to_string(),
                track: "malformed".into(),
            }],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::InvalidInput
    );

    renderer.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[test]
fn cast_resolved_items_request_serializes_one_complete_load() -> Result<()> {
    use crate::messages::{CastResolvedItemsRequest, LoadResolvedItemsRequest, PlayingItemRef};

    let request = CastResolvedItemsRequest {
        fingerprint: "device-fingerprint".to_string(),
        load: LoadResolvedItemsRequest {
            call_id: None,
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: "library".to_string(),
                track: "track".to_string(),
            }],
            start_index: Some(2),
            mode: PlaylistOperateMode::PlayNext,
            expected_revision: Some(17),
            playback_mode: Some(WirePlaybackMode::Shuffle),
            position_seconds: Some(12.5),
            play: LoadPlay::Play,
        },
        call_id: Some(23),
    };
    let value = serde_json::to_value(request)?;
    assert_eq!(value["fingerprint"], "device-fingerprint");
    assert_eq!(value["call_id"], 23);
    assert_eq!(value["load"]["start_index"], 2);
    assert_eq!(value["load"]["mode"], "PlayNext");
    assert_eq!(value["load"]["expected_revision"], 17);
    assert_eq!(value["load"]["playback_mode"], "Shuffle");
    assert_eq!(value["load"]["position_seconds"], 12.5);
    assert_eq!(value["load"]["play"], "Play");
    assert_eq!(value["load"]["items"][0]["library_track"]["track"], "track");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_renderers_reports_access_availability_and_attachment() -> Result<()> {
    let host_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let no_control_dir = tempfile::tempdir()?;
    let offline_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let no_control = fixture(no_control_dir.path()).await?;
    let offline = fixture(offline_dir.path()).await?;
    let controller = fixture(controller_dir.path()).await?;

    replicate_host_library(&host, &renderer).await?;
    let renderer_address = paired_full(&renderer, &controller).await?;
    let no_control_address = paired_full(&no_control, &controller).await?;
    let offline_address = paired_full(&offline, &controller).await?;

    let controller_info = info(&controller).await?;
    let renderer_info = info(&renderer).await?;
    let no_control_info = info(&no_control).await?;
    let offline_info = info(&offline).await?;
    for (fingerprint, address) in [
        (&renderer_info.fingerprint, renderer_address),
        (&no_control_info.fingerprint, no_control_address),
        (&offline_info.fingerprint, offline_address),
    ] {
        controller
            .global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(
                fingerprint,
                vec![format!("127.0.0.1:{}", address.port())],
            )
            .await?;
    }
    let renderer_validator = controller.global_params.cert_validator.read().await.clone();
    wait_until("renderer server readiness", || async {
        let ready = fetch_device_info_at_port(
            "127.0.0.1",
            renderer_address.port(),
            Arc::new(Arc::new(renderer_validator.clone()).into_client_config()),
        )
        .await
        .is_ok();
        Ok(ready.then_some(()))
    })
    .await
    .context("renderer server did not become ready")?;
    let host_library_id = host.global_params.library_id.context("host library id")?;
    let controller_library_id = controller
        .global_params
        .library_id
        .context("controller library id")?;
    let mut no_control_grants = no_control
        .global_params
        .node
        .access_policy
        .grants(&controller_info.fingerprint)?;
    no_control_grants.control = false;
    no_control
        .global_params
        .node
        .access_policy
        .set(&controller_info.fingerprint, no_control_grants)?;
    offline.global_params.node.shutdown().await?;

    async fn list(
        controller: &Fixture,
        library_ids: Vec<String>,
    ) -> Result<Vec<RendererCandidate>> {
        let request = ListRenderersRequest {
            call_id: None,
            library_ids,
        };
        Ok(request
            .handle(
                request.extract_params(&controller.global_params),
                None,
                &request,
            )
            .await?
            .context("renderer list response")?
            .renderers)
    }

    let empty = list(&controller, Vec::new()).await?;
    fn candidate<'a>(
        renderers: &'a [RendererCandidate],
        fingerprint: &str,
    ) -> Result<&'a RendererCandidate> {
        renderers
            .iter()
            .find(|renderer| renderer.fingerprint == fingerprint)
            .with_context(|| format!("missing renderer candidate {fingerprint}"))
    }
    assert_eq!(
        candidate(&empty, &renderer_info.fingerprint)?.availability,
        RendererAvailability::Available
    );
    assert_eq!(
        candidate(&empty, &no_control_info.fingerprint)?.availability,
        RendererAvailability::NoControl
    );
    assert_eq!(
        candidate(&empty, &offline_info.fingerprint)?.availability,
        RendererAvailability::Offline
    );

    let host_library = list(&controller, vec![host_library_id.to_string()]).await?;
    assert_eq!(
        candidate(&host_library, &renderer_info.fingerprint)?.availability,
        RendererAvailability::Available
    );
    let own_library = list(&controller, vec![controller_library_id.to_string()]).await?;
    let blocked = candidate(&own_library, &renderer_info.fingerprint)?;
    assert_eq!(blocked.availability, RendererAvailability::CantPlay);
    let controller_library_id_text = controller_library_id.to_string();
    assert_eq!(
        blocked.blocked_library_id.as_deref(),
        Some(controller_library_id_text.as_str())
    );
    assert_eq!(blocked.library_state, Some(CastLibraryState::NotHeld));

    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    let attached = list(&controller, Vec::new()).await?;
    assert!(candidate(&attached, &renderer_info.fingerprint)?.attached);

    controller.global_params.node.detach_renderer().await?;
    host.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    no_control.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_link_request_round_trip_errors_and_cast_start_modes() -> Result<()> {
    use crate::backends::remote::RendererLink;
    use crate::messages::{
        CastLibraryState, LoadResolvedItemsRequest, PlaylistOperateMode, RequestErrorKind,
    };
    use crate::request_error::{ExpectedRequestError, classify};

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller_signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        controller_signals.clone(),
    )
    .await?;
    let renderer = fixture(renderer_dir.path()).await?;
    attach_pair_with_signals(&controller, &renderer, &controller_signals).await?;
    let link: Arc<RendererLink> = controller
        .global_params
        .node
        .renderer_link()?
        .context("attached renderer link")?;
    let library_id = renderer
        .global_params
        .library_id
        .context("renderer library id")?;

    let preflight = link.call(&preflight_request(library_id)).await?;
    assert_eq!(preflight.libraries[0].state, CastLibraryState::Ready);

    let ids = [
        session_audio(&renderer, "request-anchor.ogg").await?,
        session_audio(&renderer, "request-no-switch.ogg").await?,
        session_audio(&renderer, "request-append-play.ogg").await?,
        session_audio(&renderer, "request-play-next.ogg").await?,
        session_audio(&renderer, "request-handoff-last.ogg").await?,
    ];
    let items =
        future::join_all(ids.iter().map(|id| library_track_for_id(&renderer, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let anchor = items[0];
    renderer
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                vec![(PlayingItem::from(anchor), PlaybackSource::LibraryStream)],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&renderer, |status| status.playlist.len() == 1).await?;
    renderer.global_params.player.lock().await.switch(0);
    wait_player(&renderer, |status| {
        status.item == Some(PlayingItem::from(items[0]))
    })
    .await?;
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| {
        status.item == Some(PlayingItem::from(items[0])) && status.state == PlayerPlaybackState::Paused
    })
    .await?;

    let initial_revision = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let append_without_switch = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![items[1].into()],
        start_index: None,
        mode: PlaylistOperateMode::AppendToEnd,
        expected_revision: Some(initial_revision),
        playback_mode: None,
        position_seconds: None,
        play: LoadPlay::Auto,
    };
    link.call(&append_without_switch).await?;
    wait_player(&renderer, |status| status.playlist.len() == 2).await?;
    assert_eq!(
        renderer.global_params.player.lock().await.get_status().item,
        Some(PlayingItem::from(items[0]))
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        initial_revision + 1
    );

    for (mode, item, expected_state) in [
        (
            PlaylistOperateMode::AppendToEnd,
            items[2],
            PlayerPlaybackState::Playing,
        ),
        (
            PlaylistOperateMode::PlayNext,
            items[3],
            PlayerPlaybackState::Playing,
        ),
    ] {
        let request = LoadResolvedItemsRequest {
            call_id: None,
            items: vec![item.into()],
            start_index: Some(0),
            mode,
            expected_revision: Some(
                renderer
                    .global_params
                    .node
                    .session()?
                    .queue_revision()
                    .await,
            ),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        };
        link.call(&request).await?;
        wait_player(&renderer, |status| {
            status.item == Some(PlayingItem::from(item)) && status.state == expected_state
        })
        .await?;
        // Keep this test from decoding audio for the rest of its run; other tests in the
        // process share the machine and stream audio under 10 second deadlines.
        renderer.global_params.player.lock().await.pause();
    }

    // The player publishes the queue and the current item separately; wait for all four
    // items before taking the snapshot the rejected load must leave unchanged.
    let queue_before_invalid = wait_player(&renderer, |status| status.playlist.len() == 4)
        .await?
        .playlist;
    let revision_before_invalid = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let invalid = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: "malformed".into(),
        }],
        start_index: None,
        mode: PlaylistOperateMode::AppendToEnd,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: LoadPlay::Auto,
    };
    let error = match link.call(&invalid).await {
        Ok(_) => anyhow::bail!("malformed cast load unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Remote {
                kind: RequestErrorKind::InvalidInput,
                cause: None,
                ..
            })
        ),
        "unexpected typed call error: {error:#}"
    );
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);

    let not_held = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![PlayingItemRef::LibraryTrack {
            library_id: Uuid::new_v4().to_string(),
            track: Uuid::new_v4().to_string(),
        }],
        start_index: None,
        mode: PlaylistOperateMode::AppendToEnd,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: LoadPlay::Auto,
    };
    let error = match link.call(&not_held).await {
        Ok(_) => anyhow::bail!("cast to a library the renderer does not hold succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Remote {
            kind: RequestErrorKind::PermissionDenied,
            cause: Some(RequestFailureCause::CastNotPossible {
                state: CastLibraryState::NotHeld,
                ..
            }),
            ..
        })
    ));
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_invalid
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision_before_invalid
    );
    assert!(
        !controller_signals
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "RemoteRequestFailed")
    );

    let target_snapshot = link.call(&SessionSnapshotRequest { call_id: None }).await?;
    assert_eq!(target_snapshot.items.len(), 4);
    let target_before_handoff = renderer.global_params.player.lock().await.get_status();
    let changed_index = target_before_handoff.index.map_or(0, |index| {
        (index + 1) % target_before_handoff.playlist.len()
    });
    renderer
        .global_params
        .player
        .lock()
        .await
        .switch(changed_index);
    let target_after_switch =
        wait_player(&renderer, |status| status.index == Some(changed_index)).await?;
    let insertion = target_after_switch
        .index
        .map_or(target_after_switch.playlist.len(), |index| index + 1);
    let mut expected_merged = target_after_switch
        .playlist
        .iter()
        .map(|entry| entry.item.clone())
        .collect::<Vec<_>>();
    expected_merged.splice(
        insertion..insertion,
        [PlayingItem::from(items[2]), PlayingItem::from(items[3])],
    );
    let handoff = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![items[2].into(), items[3].into()],
        start_index: Some(1),
        mode: PlaylistOperateMode::PlayNext,
        expected_revision: Some(revision_before_invalid),
        playback_mode: Some(WirePlaybackMode::Shuffle),
        position_seconds: Some(1.25),
        play: LoadPlay::Play,
    };
    link.call(&handoff).await?;
    let merged = wait_player(&renderer, |status| {
        status.playlist == expected_merged
            && status.index == Some(insertion + 1)
            && status.item == Some(PlayingItem::from(items[3]))
            && status.state == PlayerPlaybackState::Playing
            && status.playback_mode == PlaybackMode::Shuffle
    })
    .await?;
    assert_eq!(merged.playlist, expected_merged);
    assert!(merged.position.abs_diff(Duration::from_millis(1250)) <= Duration::from_millis(1500));

    renderer.global_params.player.lock().await.pause();
    renderer.global_params.player.lock().await.clear_playlist();
    wait_player(&renderer, |status| status.playlist.is_empty()).await?;
    renderer
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                [items[0], items[1]]
                    .into_iter()
                    .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    let idle_queue = wait_player(&renderer, |status| {
        status.playlist.len() == 2 && status.index.is_none()
    })
    .await?
    .playlist;
    let idle_handoff = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![items[2].into(), items[3].into()],
        start_index: Some(1),
        mode: PlaylistOperateMode::PlayNext,
        expected_revision: None,
        playback_mode: None,
        position_seconds: Some(0.9),
        play: LoadPlay::Play,
    };
    link.call(&idle_handoff).await?;
    let idle_merged = wait_player(&renderer, |status| {
        status.playlist.len() == 4
            && status.index == Some(3)
            && status.item == Some(PlayingItem::from(items[3]))
            && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    assert_eq!(idle_merged.playlist[..2], idle_queue);
    assert_eq!(
        idle_merged.playlist[2..],
        [PlayingItem::from(items[2]), PlayingItem::from(items[3])]
    );

    renderer.global_params.player.lock().await.pause();
    renderer.global_params.player.lock().await.clear_playlist();
    wait_player(&renderer, |status| status.playlist.is_empty()).await?;
    let empty_handoff = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![
            items[0].into(),
            items[1].into(),
            items[2].into(),
            items[3].into(),
            items[4].into(),
        ],
        start_index: Some(2),
        mode: PlaylistOperateMode::PlayNext,
        expected_revision: None,
        playback_mode: None,
        position_seconds: Some(0.7),
        play: LoadPlay::Play,
    };
    link.call(&empty_handoff).await?;
    let empty_loaded = wait_player(&renderer, |status| {
        status.playlist.len() == 5
            && status.index == Some(2)
            && status.item == Some(PlayingItem::from(items[2]))
            && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    assert_eq!(
        empty_loaded.playlist,
        items
            .iter()
            .copied()
            .map(PlayingItem::from)
            .collect::<Vec<_>>()
    );

    renderer.global_params.node.shutdown().await?;
    wait_until("renderer link shutdown", || async {
        Ok(controller
            .global_params
            .node
            .renderer_link()?
            .is_none()
            .then_some(()))
    })
    .await
    .context("renderer link did not clear after renderer shutdown")?;
    let error = match link.call(&preflight_request(library_id)).await {
        Ok(_) => anyhow::bail!("request over a closed renderer link succeeded"),
        Err(error) => error,
    };
    assert_eq!(classify(&error), RequestErrorKind::Unavailable);
    assert!(controller.global_params.node.renderer_link()?.is_none());

    controller.global_params.node.shutdown().await?;
    Ok(())
}

fn preflight_request(library_id: Uuid) -> CastPreflightRequest {
    CastPreflightRequest {
        call_id: None,
        library_ids: vec![library_id.to_string()],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f3_persists_changes_without_rewriting_while_idle() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "persist-first.ogg").await?;
    queue_session(&a, &[id]).await?;
    let store = dir.path().join("sessions/local.json");
    wait_persisted_queue(&store, 1)
        .await
        .context("initial session queue was not persisted")?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| status.state == PlayerPlaybackState::Playing).await?;
    a.global_params.player.lock().await.pause();
    wait_player(&a, |status| status.state == PlayerPlaybackState::Paused).await?;
    let session = a.global_params.node.session()?;
    let persistence_state = Arc::new(StdMutex::new((
        session.persistence_write_count(),
        Instant::now(),
    )));
    wait_until("session persistence to become idle", || {
        let persistence_state = persistence_state.clone();
        let session = session.clone();
        async move {
            let current = session.persistence_write_count();
            let mut state = persistence_state.lock().unwrap();
            if current != state.0 {
                state.0 = current;
                state.1 = Instant::now();
            } else if state.1.elapsed() >= Duration::from_secs(1) {
                return Ok(Some(()));
            }
            Ok(None)
        }
    })
    .await?;
    let before = fs::read(&store)?;
    let writes_before = session.persistence_write_count();
    // The idle barrier above is positive; this interval checks that no later write occurs.
    sleep(Duration::from_secs(1)).await;
    assert_eq!(session.persistence_write_count(), writes_before);
    assert_eq!(fs::read(&store)?, before);

    let second = session_audio(&a, "persist-second.ogg").await?;
    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(
                PlayingItem::from(library_track_for_id(&a, second).await?),
                PlaybackSource::LibraryStream,
            )],
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| status.playlist.len() == 2).await?;
    wait_persisted_queue(&store, 2)
        .await
        .context("changed queue was not persisted within one second")?;
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2a_revision_conflicts_and_identity_persist() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let track = session_audio(&fixture, "revision-track.ogg").await?;
    let item = PlayingItem::from(library_track_for_id(&fixture, track).await?);
    let session = fixture.global_params.node.session()?;
    let session_id = session.session_id().await;
    session
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(item.clone(), PlaybackSource::LibraryStream)],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&fixture, |status| status.playlist == [item.clone()]).await?;
    assert_eq!(session.queue_revision().await, 1);

    fixture.global_params.player.lock().await.play();
    fixture.global_params.player.lock().await.pause();
    fixture.global_params.player.lock().await.seek(0.0);
    assert_eq!(session.queue_revision().await, 1);

    let stale = session
        .edit_queue(Some(0), |player| {
            player.remove_from_playlist(0);
            true
        })
        .await
        .unwrap_err();
    assert_eq!(classify(&stale), RequestErrorKind::Conflict);
    assert_eq!(
        fixture.global_params.player.lock().await.get_playlist(),
        [item]
    );

    session
        .edit_queue(None, |player| {
            player.remove_from_playlist(0);
            true
        })
        .await?;
    wait_player(&fixture, |status| status.playlist.is_empty()).await?;
    assert_eq!(session.queue_revision().await, 2);

    fixture.global_params.node.shutdown().await?;
    let reopened = NodeContext::new(
        &fixture.global_params.config_path,
        &fixture.global_params.config_path,
        Arc::new(Quiet),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    assert_eq!(reopened.session()?.session_id().await, session_id);
    reopened.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switch_request_rejects_stale_queue_revision_and_accepts_current_revision() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let first = session_audio(&fixture, "switch-revision-first.ogg").await?;
    let second = session_audio(&fixture, "switch-revision-second.ogg").await?;
    let items = [
        library_track_for_id(&fixture, first).await?,
        library_track_for_id(&fixture, second).await?,
    ];
    let session = fixture.global_params.node.session()?;
    session
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                items
                    .iter()
                    .copied()
                    .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&fixture, |status| status.playlist.len() == 2).await?;
    fixture.global_params.player.lock().await.switch(0);
    wait_player(&fixture, |status| status.index == Some(0)).await?;
    let before = fixture.global_params.player.lock().await.get_status().item;

    let stale = SwitchRequest {
        call_id: None,
        index: 1,
        expected_revision: Some(0),
    };
    let params = stale.extract_params(&fixture.global_params);
    let error = stale.handle(params, None, &stale).await.unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::Conflict);
    assert_eq!(
        fixture.global_params.player.lock().await.get_status().item,
        before
    );

    let current = SwitchRequest {
        call_id: None,
        index: 1,
        expected_revision: Some(1),
    };
    let params = current.extract_params(&fixture.global_params);
    current.handle(params, None, &current).await?;
    wait_player(&fixture, |status| status.index == Some(1)).await?;
    assert_eq!(
        fixture.global_params.player.lock().await.get_status().item,
        Some(PlayingItem::from(items[1])),
    );
    let invalid = SwitchRequest {
        call_id: None,
        index: 2,
        expected_revision: Some(1),
    };
    let params = invalid.extract_params(&fixture.global_params);
    let error = invalid.handle(params, None, &invalid).await.unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    assert_eq!(
        fixture.global_params.player.lock().await.get_status().item,
        Some(PlayingItem::from(items[1])),
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2a_two_controllers_observe_conflict_and_same_queue() -> Result<()> {
    use crate::messages::{
        MovePlaylistItemRequest, PlaylistUpdate, RemoteRequestFailed, RemoveRequest,
    };
    use futures::StreamExt;

    async fn wait_revision(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        mut updates: Vec<PlaylistUpdate>,
        revision: u64,
    ) -> Result<PlaylistUpdate> {
        timeout(TEST_DEADLINE, async {
            loop {
                if let Some(index) = updates
                    .iter()
                    .position(|update| update.revision == revision)
                {
                    return Ok(updates.swap_remove(index));
                }
                let frame = socket.next().await.context("controller socket closed")??;
                if let Some((name, payload, _)) = decode_message(&frame.into_data())
                    && name == "PlaylistUpdate"
                {
                    updates.push(rinf::deserialize(&payload)?);
                }
            }
        })
        .await?
    }

    let renderer_dir = tempfile::tempdir()?;
    let first_controller_dir = tempfile::tempdir()?;
    let second_controller_dir = tempfile::tempdir()?;
    let renderer = fixture(renderer_dir.path()).await?;
    let first_controller = fixture(first_controller_dir.path()).await?;
    let second_controller = fixture(second_controller_dir.path()).await?;
    let (mut first, mut second) =
        connect_two_controllers(&renderer, &first_controller, &second_controller).await?;

    let first_track = session_audio(&renderer, "conflict-first.ogg").await?;
    let second_track = session_audio(&renderer, "conflict-second.ogg").await?;
    queue_session(&renderer, &[first_track, second_track]).await?;

    let (first_name, _, first_updates) = controller_request(
        &mut first,
        "MovePlaylistItemRequest",
        rinf::serialize(&MovePlaylistItemRequest {
            call_id: None,
            old_index: 0,
            new_index: 1,
            expected_revision: Some(0),
        })?,
    )
    .await?;
    assert_eq!(first_name, "");
    let (second_name, failure_payload, second_updates) = controller_request(
        &mut second,
        "RemoveRequest",
        rinf::serialize(&RemoveRequest {
            call_id: None,
            index: 0,
            expected_revision: Some(0),
        })?,
    )
    .await?;
    assert_eq!(second_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&failure_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::Conflict);
    let first_final = wait_revision(&mut first, first_updates, 1).await?;
    let second_final = wait_revision(&mut second, second_updates, 1).await?;
    assert_eq!(first_final.items.len(), 2);
    assert_eq!(first_final.items.len(), second_final.items.len());
    assert_eq!(first_final.session_id, second_final.session_id);
    assert_eq!(
        first_final
            .items
            .iter()
            .map(|item| &item.title)
            .collect::<Vec<_>>(),
        second_final
            .items
            .iter()
            .map(|item| &item.title)
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        1
    );
    renderer.global_params.node.shutdown().await?;
    first_controller.global_params.node.shutdown().await?;
    second_controller.global_params.node.shutdown().await?;
    Ok(())
}

/// Pairs both controllers with `renderer` and opens a controller WebSocket from each.
async fn connect_two_controllers(
    renderer: &Fixture,
    first: &Fixture,
    second: &Fixture,
) -> Result<(ControllerSocket, ControllerSocket)> {
    let addr = paired_full(renderer, first).await?;
    pair(renderer, second).await?;
    let mut sockets = Vec::with_capacity(2);
    for controller in [first, second] {
        sockets.push(
            connect_remote(
                "127.0.0.1",
                addr.port(),
                &controller.global_params.config_path,
                controller.global_params.cert_validator.read().await.clone(),
                &controller.global_params.node.sync_tokens,
            )
            .await?,
        );
    }
    let second_socket = sockets.pop().context("second controller socket")?;
    let first_socket = sockets.pop().context("first controller socket")?;
    Ok((first_socket, second_socket))
}

type ControllerSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Send one addressed request over a controller socket and collect queue updates until its reply.
async fn controller_request(
    socket: &mut ControllerSocket,
    name: &str,
    payload: Vec<u8>,
) -> Result<(String, Vec<u8>, Vec<PlaylistUpdate>)> {
    use crate::backends::remote::encode_message;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let request_id = Uuid::new_v4();
    socket
        .send(Message::Binary(
            encode_message(name, &payload, Some(request_id)).into(),
        ))
        .await?;
    let mut updates = Vec::new();
    timeout(TEST_DEADLINE, async {
        loop {
            let frame = socket.next().await.context("controller socket closed")??;
            let Some((response_name, payload, response_id)) = decode_message(&frame.into_data())
            else {
                continue;
            };
            if response_name == "PlaylistUpdate" {
                updates.push(rinf::deserialize(&payload)?);
            }
            if response_id == request_id {
                return Ok::<_, anyhow::Error>((response_name, payload, updates));
            }
        }
    })
    .await?
}

/// Set one library's level in the grant `host` gives to `peer_fingerprint`.
fn set_library_grant(
    host: &Fixture,
    peer_fingerprint: &str,
    library_id: Uuid,
    level: SyncLevel,
) -> Result<()> {
    let policy = &host.global_params.node.access_policy;
    let mut grant = policy.grants(peer_fingerprint)?;
    grant.connection.libraries.insert(
        library_id,
        match level {
            SyncLevel::None => ConnectionLevel::None,
            SyncLevel::Pull => ConnectionLevel::View,
            SyncLevel::PullPush => ConnectionLevel::ViewEdit,
        },
    );
    grant.sync.libraries.insert(library_id, level);
    policy.set(peer_fingerprint, grant)?;
    Ok(())
}

/// Pair `peer` with `host`, grant it the host library, add the replica and sync its metadata.
async fn replicate_host_library(host: &Fixture, peer: &Fixture) -> Result<(SocketAddr, Fixture)> {
    let host_addr = paired_full(host, peer).await?;
    replicate_host_library_at(host, peer, host_addr).await
}

pub(super) async fn replicate_host_library_at(
    host: &Fixture,
    peer: &Fixture,
    host_addr: SocketAddr,
) -> Result<(SocketAddr, Fixture)> {
    use crate::server::sync_worker::share_worker_tests::TestPeerSession;
    use crate::server::sync_worker::PeerEndpoint;

    let host_info = info(host).await?;
    let peer_info = info(peer).await?;
    let trusted_host = format!("127.0.0.1:{}", host_addr.port());
    peer.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&host_info.fingerprint, vec![trusted_host.clone()])
        .await?;
    let host_library_id = host.global_params.library_id.context("host library id")?;
    set_library_grant(
        host,
        &peer_info.fingerprint,
        host_library_id,
        SyncLevel::PullPush,
    )?;
    let listed = request_peer_libraries(peer, &host_info.fingerprint).await?;
    let hosted = listed
        .libraries
        .iter()
        .find(|library| library.library_id == host_library_id.to_string())
        .with_context(|| format!("host library was not listed (libraries {:?})", listed.libraries))?;
    let replica_path = request_add_library(peer, &host_info.fingerprint, hosted).await?;
    let replica = fixture_open_record(peer, replica_path).await?;
    let endpoint = PeerEndpoint {
        fingerprint: host_info.fingerprint,
        trusted_host,
        host: "127.0.0.1".into(),
        port: host_addr.port(),
    };
    // One session for the whole replication: an incomplete pass resumes its work on the
    // next pass instead of abandoning it, as the background worker does.
    let session = TestPeerSession::default();
    timeout(TEST_DEADLINE, async {
        loop {
            match session
                .pass(
                    replica.global_params.clone(),
                    &replica.private_key,
                    endpoint.clone(),
                    true,
                )
                .await?
            {
                PeerPass::Complete => return Ok::<_, anyhow::Error>(()),
                PeerPass::Incomplete => task::yield_now().await,
                other => anyhow::bail!("unexpected replica metadata pass: {other:?}"),
            }
        }
    })
    .await
    .context("replica metadata sync timed out")??;
    Ok((host_addr, replica))
}

/// Give a replica a local copy of one of its files, so it plays without streaming.
async fn materialize_replica_file(
    replica: &Fixture,
    file_id: i32,
    source: &Path,
    file_name: &str,
) -> Result<()> {
    let library = replica.global_params.test_library();
    fs::copy(source, Path::new(&library.media_root).join(file_name))?;
    media_files::Entity::update(media_files::ActiveModel {
        id: Set(file_id),
        directory: Set(String::new()),
        file_name: Set(file_name.into()),
        local_available: Set(true),
        ..Default::default()
    })
    .exec(&*library.main_db)
    .await?;
    Ok(())
}

async fn attached_handoff_source_recorded(
    hold_source_library: bool,
    source_signals: Option<Arc<RecordedSignals>>,
) -> Result<(Vec<TempDir>, Fixture, Fixture, String)> {
    attached_handoff_source_with_player(hold_source_library, source_signals, None).await
}

/// `source_player` makes the source's player of the real one; none keeps the real player.
async fn attached_handoff_source_with_player(
    hold_source_library: bool,
    source_signals: Option<Arc<RecordedSignals>>,
    source_player: Option<crate::session::PlayerWrap>,
) -> Result<(Vec<TempDir>, Fixture, Fixture, String)> {
    let controller_dir = tempfile::tempdir()?;
    let source_dir = tempfile::tempdir()?;
    let controller = fixture(controller_dir.path()).await?;
    let broadcaster: Arc<dyn Broadcaster> = match source_signals {
        Some(signals) => signals,
        None => Arc::new(Quiet),
    };
    let source = fixture_with_player(
        source_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        broadcaster,
        ReconnectConfig::default(),
        RunMode::App,
        source_player.unwrap_or_else(crate::session::PlayerSession::real_player),
    )
    .await?;
    let source_id = session_audio(&source, "handoff-retry-source.ogg").await?;
    queue_session(&source, &[source_id]).await?;
    // The fixture track lasts a few seconds; repeat it so a slow run never sees it end.
    for node in [&controller, &source] {
        node.global_params
            .player
            .lock()
            .await
            .set_playback_mode(PlaybackMode::RepeatAll);
    }
    source.global_params.player.lock().await.load(0);
    source.global_params.player.lock().await.play();
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    let source_address = if hold_source_library {
        replicate_host_library(&source, &controller).await?.0
    } else {
        super::paired_full(&source, &controller).await?
    };
    let source_info = super::info(&source).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &source_info.fingerprint,
            vec![format!("127.0.0.1:{}", source_address.port())],
        )
        .await?;
    controller
        .global_params
        .node
        .attach_renderer(&source_info.fingerprint)
        .await?;
    Ok((
        vec![controller_dir, source_dir],
        controller,
        source,
        source_info.fingerprint,
    ))
}

async fn hand_off(controller: &Fixture, target: Option<String>) -> Result<()> {
    use crate::messages::HandoffRequest;

    let handoff = HandoffRequest {
        call_id: None,
        target,
    };
    handoff
        .handle(
            handoff.extract_params(&controller.global_params),
            None,
            &handoff,
        )
        .await?;
    Ok(())
}

mod faulty_player {
    use std::future;
    use std::result;
    use super::*;
    use futures::future::BoxFuture;
    use playback::player::{
        ClearHandoffBlocksResult, HandoffBlockId, HandoffLoadResult, InterruptedItem,
        PausedPlayback, PlaybackSourceResolver, Player, PlayerCommandError, PlaylistEdit,
        PlaylistStatus, TakeBackBlockResult,
    };
    use playback::pcm_tap::PcmTap;
    use playback::{InternalLog, PlayerCommand, SimpleReceiver};
    use tokio::sync::oneshot::{self, Receiver};

    /// The real player, except that a pause is answered with a failure: a device whose player
    /// cannot pause, so that handing its session off fails at the pause and nowhere else.
    pub(super) struct PauseFails(pub(super) Player);

    impl Playable for PauseFails {
        fn command_with_completion(
            &mut self,
            command: PlayerCommand,
        ) -> BoxFuture<
            'static,
            result::Result<(), PlayerCommandError>,
        > {
            if matches!(command, PlayerCommand::Pause) {
                return Box::pin(future::ready(Err(
                    PlayerCommandError::CannotApply,
                )));
            }
            self.0.command_with_completion(command)
        }
        fn command_completion(
            &mut self,
        ) -> BoxFuture<
            'static,
            result::Result<PlayerStatus, PlayerCommandError>,
        > {
            self.0.command_completion()
        }
        fn load(&self, index: usize) {
            self.0.load(index)
        }
        fn load_at_position(&self, index: usize, position: Duration, play: bool) {
            self.0.load_at_position(index, position, play)
        }
        fn play(&self) {
            self.0.play()
        }
        fn pause(&self) {
            self.0.pause()
        }
        fn stop(&self) {
            self.0.stop()
        }
        fn next(&self) {
            self.0.next()
        }
        fn previous(&self) {
            self.0.previous()
        }
        fn switch(&self, index: usize) {
            self.0.switch(index)
        }
        fn seek(&self, position_seconds: f64) {
            self.0.seek(position_seconds)
        }
        fn add_to_playlist(&self, tracks: Vec<(PlayingItem, PathBuf)>, mode: AddMode) {
            self.0.add_to_playlist(tracks, mode)
        }
        fn add_to_playlist_with_sources(
            &self,
            tracks: Vec<(PlayingItem, PlaybackSource)>,
            mode: AddMode,
        ) {
            self.0.add_to_playlist_with_sources(tracks, mode)
        }
        fn edit_playlist(&mut self, edit: PlaylistEdit) {
            self.0.edit_playlist(edit)
        }
        fn handoff_load(
            &self,
            tracks: Vec<(PlayingItem, PlaybackSource)>,
            block: HandoffBlockId,
            start_index: Option<usize>,
            position: Duration,
            play: bool,
            playback_mode: Option<PlaybackMode>,
        ) -> Receiver<HandoffLoadResult> {
            self.0
                .handoff_load(tracks, block, start_index, position, play, playback_mode)
        }
        fn take_back_block(
            &self,
            block: HandoffBlockId,
            interrupted: InterruptedItem,
            resume: bool,
        ) -> Receiver<Option<TakeBackBlockResult>> {
            self.0.take_back_block(block, interrupted, resume)
        }
        fn clear_handoff_blocks(
            &self,
        ) -> Receiver<ClearHandoffBlocksResult> {
            self.0.clear_handoff_blocks()
        }
        fn pause_reporting(&self) -> Receiver<PausedPlayback> {
            // This pause fails as well: its answer is dropped unsent.
            oneshot::channel().1
        }
        fn restore(
            &self,
            tracks: Vec<(PlayingItem, PlaybackSource)>,
            index: Option<usize>,
            position: Duration,
            mode: PlaybackMode,
        ) {
            self.0.restore(tracks, index, position, mode)
        }
        fn restore_tagged(
            &self,
            entries: Vec<QueueEntry>,
            index: Option<usize>,
            position: Duration,
            mode: PlaybackMode,
            revision: u64,
        ) {
            self.0.restore_tagged(entries, index, position, mode, revision)
        }
        fn set_source_resolver(&self, resolver: Arc<dyn PlaybackSourceResolver>) {
            self.0.set_source_resolver(resolver)
        }
        fn remove_from_playlist(&self, index: usize) {
            self.0.remove_from_playlist(index)
        }
        fn clear_playlist(&self) {
            self.0.clear_playlist()
        }
        fn move_playlist_item(&self, old_index: usize, new_index: usize) {
            self.0.move_playlist_item(old_index, new_index)
        }
        fn set_playback_mode(&mut self, mode: PlaybackMode) {
            self.0.set_playback_mode(mode)
        }
        fn set_volume(&mut self, volume: f32) {
            self.0.set_volume(volume)
        }
        fn set_realtime_fft_enabled(&mut self, enabled: bool) {
            self.0.set_realtime_fft_enabled(enabled)
        }
        fn pcm_tap(&self) -> Arc<PcmTap> {
            self.0.pcm_tap()
        }
        fn set_adaptive_switching_enabled(&mut self, enabled: bool) {
            self.0.set_adaptive_switching_enabled(enabled)
        }
        fn set_mix_with_others(&mut self, enabled: bool) {
            self.0.set_mix_with_others(enabled)
        }
        fn terminate(&self) {
            self.0.terminate()
        }
        fn get_status(&self) -> PlayerStatus {
            self.0.get_status()
        }
        fn status_after_pending(&self) -> BoxFuture<'static, PlayerStatus> {
            self.0.status_after_pending()
        }
        fn get_playlist(&self) -> Vec<PlayingItem> {
            self.0.get_playlist()
        }
        fn subscribe_status(&self) -> SimpleReceiver<PlayerStatus> {
            self.0.subscribe_status()
        }
        fn subscribe_played_through(&self) -> SimpleReceiver<PlayedThrough> {
            self.0.subscribe_played_through()
        }
        fn subscribe_playlist(&self) -> SimpleReceiver<PlaylistStatus> {
            self.0.subscribe_playlist()
        }
        fn subscribe_realtime_fft(&self) -> SimpleReceiver<Vec<f32>> {
            self.0.subscribe_realtime_fft()
        }
        fn subscribe_crash(&self) -> SimpleReceiver<String> {
            self.0.subscribe_crash()
        }
        fn subscribe_log(&self) -> SimpleReceiver<InternalLog> {
            self.0.subscribe_log()
        }
    }
}
use faulty_player::PauseFails;

/// A handoff source whose player cannot pause.
async fn attached_handoff_source_that_cannot_pause()
-> Result<(Vec<TempDir>, Fixture, Fixture, String)> {
    attached_handoff_source_with_player(
        true,
        None,
        Some(Box::new(|player| Arc::new(Mutex::new(PauseFails(player))))),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_pause_failure_leaves_destination_unchanged() -> Result<()> {
    let (_dirs, controller, source, _) = attached_handoff_source_that_cannot_pause().await?;
    let error = hand_off(&controller, None).await.unwrap_err();
    // The handoff stops at the pause that failed, not at a later check.
    assert_eq!(
        crate::request_error::classify(&error),
        RequestErrorKind::InvalidInput,
        "{error:#}"
    );
    assert_eq!(
        controller
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .unwrap()
            .fingerprint(),
        super::info(&source).await?.fingerprint
    );
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    controller.global_params.node.shutdown().await?;
    source.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_dispatch_appends_to_idle_renderer_queue_and_starts_source_cursor() -> Result<()> {
    use crate::messages::HandoffRequest;

    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let source = fixture(source_dir.path()).await?;
    let target = fixture_with_broadcaster(
        target_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let ids = [
        session_audio(&source, "handoff-merge-1.ogg").await?,
        session_audio(&source, "handoff-merge-2.ogg").await?,
        session_audio(&source, "handoff-merge-3.ogg").await?,
        session_audio(&source, "handoff-merge-4.ogg").await?,
        session_audio(&source, "handoff-merge-5.ogg").await?,
    ];
    let items = future::join_all(ids.iter().map(|id| library_track_for_id(&source, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let (_, target_replica) = replicate_host_library(&source, &target).await?;
    let target_address = paired_full(&target, &source).await?;
    let target_fingerprint = info(&target).await?.fingerprint;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;

    let source_queue = items
        .iter()
        .copied()
        .map(PlayingItem::from)
        .collect::<Vec<_>>();
    source
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                source_queue
                    .iter()
                    .cloned()
                    .map(|item| (item, PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&source, |status| status.playlist.len() == 5).await?;
    source
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::Shuffle);
    source.global_params.player.lock().await.switch(2);
    wait_player(&source, |status| status.index == Some(2)).await?;
    source.global_params.player.lock().await.play();
    let source_before = wait_player(&source, |status| {
        status.state == PlayerPlaybackState::Playing && status.index == Some(2)
    })
    .await?;

    let target_queue = items[..3]
        .iter()
        .copied()
        .map(PlayingItem::from)
        .collect::<Vec<_>>();
    let expected_target_queue = target_queue.clone();
    target_replica
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                target_queue
                    .into_iter()
                    .map(|item| (item, PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&target, |status| {
        status.playlist.len() == 3 && status.index.is_none()
    })
    .await?;

    let handoff = HandoffRequest {
        call_id: None,
        target: Some(target_fingerprint.clone()),
    };
    // Once the renderer is attached, the source GUI correctly suppresses local
    // session events. Sample the real source at the target's load boundary:
    // a source that is still playing here must not pass the ordering check.
    let source_session = source.global_params.node.session()?;
    let order_signals: Arc<dyn Broadcaster> = signals.clone();
    let target_node = Arc::downgrade(&target.global_params.node);
    let load = "HandoffLoadRequest";
    target
        .global_params
        .node
        .websocket_service
        .override_handler(load, move |payload, session, library_id| {
            let source_session = source_session.clone();
            let order_signals = order_signals.clone();
            let target_node = target_node.clone();
            async move {
                source_session
                    .reemit_to(&order_signals)
                    .await
                    .expect("sample source session");
                let node = target_node.upgrade().expect("target node");
                dispatch_remote(load, &payload, session, library_id, &node)
                    .await
                    .expect("real handoff load handler")
            }
        })
        .await;
    signals.clear();
    dispatch_gui_request(&source.global_params, &handoff).await?;
    let source_session_id = source
        .global_params
        .node
        .session()?
        .session_id()
        .await
        .to_string();
    let target_session_id = target
        .global_params
        .node
        .session()?
        .session_id()
        .await
        .to_string();
    let mut expected_queue = expected_target_queue.clone();
    expected_queue.extend(source_queue.iter().cloned());
    let (source_paused_at, target_loaded_at) =
        wait_until("the source pause and the target queue update", || async {
            Ok(recorded_handoff_order(
                &signals,
                &source_session_id,
                &target_session_id,
                expected_queue.len(),
            )
            .ok())
        })
        .await?;
    assert!(
        source_paused_at < target_loaded_at,
        "source paused at {source_paused_at}, target loaded at {target_loaded_at}"
    );

    let merged = wait_player(&target, |status| {
        status.playlist == expected_queue
            && status.index == Some(3 + 2)
            && status.item == source_before.item
            && status.state == PlayerPlaybackState::Playing
            && status.playback_mode == PlaybackMode::Shuffle
    })
    .await?;
    let source_paused =
        wait_player(&source, |status| status.state == PlayerPlaybackState::Paused).await?;
    assert!(merged.position.abs_diff(source_paused.position) <= Duration::from_millis(1500));
    assert_eq!(source_paused.playlist, source_queue);
    assert_eq!(source_paused.item, source_before.item);

    let back = HandoffRequest {
        call_id: None,
        target: None,
    };
    dispatch_gui_request(&source.global_params, &back).await?;
    let returned_source = wait_player(&source, |status| {
        status.state == PlayerPlaybackState::Playing && status.playlist == source_queue
    })
    .await?;
    assert_eq!(returned_source.item, source_before.item);
    let idle_target = wait_player(&target, |status| {
        status.playlist == expected_target_queue
            && status.index.is_none()
            && status.state == PlayerPlaybackState::Stopped
    })
    .await?;
    assert_eq!(idle_target.state, PlayerPlaybackState::Stopped);

    target.global_params.player.lock().await.clear_playlist();
    wait_player(&target, |status| status.playlist.is_empty()).await?;
    let second_handoff = HandoffRequest {
        call_id: None,
        target: Some(target_fingerprint),
    };
    dispatch_gui_request(&source.global_params, &second_handoff).await?;
    wait_player(&target, |status| {
        status.playlist == source_queue
            && status.index == Some(2)
            && status.item == source_before.item
            && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    dispatch_gui_request(&source.global_params, &back).await?;
    let empty_target = wait_player(&target, |status| {
        status.playlist.is_empty() && status.state == PlayerPlaybackState::Stopped
    })
    .await?;
    assert_eq!(empty_target.state, PlayerPlaybackState::Stopped);

    source.global_params.node.shutdown().await?;
    target.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn failed_pause_does_not_block_a_later_handoff() -> Result<()> {
    let (_dirs, controller, source, source_fingerprint) =
        attached_handoff_source_that_cannot_pause().await?;
    let error = hand_off(&controller, None).await.unwrap_err();
    assert_eq!(
        crate::request_error::classify(&error),
        RequestErrorKind::InvalidInput,
        "{error:#}"
    );
    hand_off(&controller, Some(source_fingerprint)).await?;
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    controller.global_params.node.shutdown().await?;
    source.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn failed_handoff_load_restores_source_playback_state() -> Result<()> {
    let signals = Arc::new(RecordedSignals::default());
    let (_dirs, controller, source, _) =
        attached_handoff_source_recorded(false, Some(signals.clone())).await?;
    signals.clear();
    let target_before = controller.global_params.player.lock().await.get_status();
    assert!(hand_off(&controller, None).await.is_err());
    let target_after = controller.global_params.player.lock().await.get_status();
    assert_eq!(target_after.playlist, target_before.playlist);
    assert_eq!(target_after.index, target_before.index);
    assert_eq!(target_after.item, target_before.item);
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    let failed_handoff_states = wait_until("source pause and resume statuses", || async {
        let states = signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .into_iter()
            .map(|status| status.state)
            .collect::<Vec<_>>();
        let has_pause_then_resume = states.iter().enumerate().any(|(index, state)| {
            *state == PlaybackState::Paused && states[index + 1..].contains(&PlaybackState::Playing)
        });
        Ok(has_pause_then_resume.then_some(states))
    })
    .await?;
    let paused_at = failed_handoff_states
        .iter()
        .position(|state| *state == PlaybackState::Paused)
        .context("source pause before failed target load")?;
    let resumed_at = failed_handoff_states
        .iter()
        .skip(paused_at + 1)
        .position(|state| *state == PlaybackState::Playing)
        .map(|index| index + paused_at + 1)
        .context("source resume after failed target load")?;
    assert!(
        paused_at < resumed_at,
        "source statuses after failed load: {failed_handoff_states:?}"
    );
    source.global_params.player.lock().await.pause();
    wait_player(&source, |status| status.state == PlayerPlaybackState::Paused).await?;
    signals.clear();
    assert!(hand_off(&controller, None).await.is_err());
    assert_eq!(
        source.global_params.player.lock().await.get_status().state,
        PlayerPlaybackState::Paused
    );
    assert!(
        signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .iter()
            .all(|status| status.state != PlaybackState::Playing)
    );
    controller.global_params.node.shutdown().await?;
    source.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_unplayable_and_decode_failures_roll_back_and_resume_source() -> Result<()> {
    use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants, SyncLevel};
    use crate::messages::HandoffRequest;
    use crate::request_error::ExpectedRequestError;
    use tokio::sync::Notify;
    let host_dir = tempfile::tempdir()?;
    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let source = fixture(source_dir.path()).await?;
    let target = fixture(target_dir.path()).await?;
    let host_id = session_audio(&host, "handoff-source-corrupt-on-target.ogg").await?;
    let (host_address, source_replica) = replicate_host_library(&host, &source)
        .await
        .context("replicate source library")?;
    pair(&host, &target).await?;
    pair(&target, &host).await?;
    let host_library_id = host.global_params.library_id.context("host library id")?;
    let target_fingerprint = info(&target).await?.fingerprint;
    host.global_params.node.access_policy.set(
        &target_fingerprint,
        DeviceGrants {
            control: true,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(host_library_id, ConnectionLevel::ViewEdit)]
                    .into_iter()
                    .collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: [(host_library_id, SyncLevel::PullPush)]
                    .into_iter()
                    .collect(),
            },
            administer: false,
        },
    )?;
    let (_, target_replica) = replicate_host_library_at(&host, &target, host_address)
        .await
        .context("replicate target library")?;
    let source_file = Path::new(&source_replica.global_params.test_library().media_root)
        .join("handoff-source-corrupt-on-target.ogg");
    fs::copy(
        Path::new(&host.global_params.test_library().media_root)
            .join("handoff-source-corrupt-on-target.ogg"),
        &source_file,
    )?;
    let host_item = library_track_for_id(&host, host_id).await?;
    let source_item = player::LibraryItem {
        library_id: target_replica
            .global_params
            .library_id
            .context("target replica library id")?,
        file_uuid: host_item.file_uuid,
    };
    source
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    PlayingItem::from(source_item),
                    PlaybackSource::Local(source_file.clone()),
                )],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    source
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    source.global_params.player.lock().await.load(0);
    source.global_params.player.lock().await.play();
    let source_before =
        wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    let expected_item = source_before.item.clone().context("source item")?;

    let source_fingerprint = info(&source).await?.fingerprint;
    let host_fingerprint = info(&host).await?.fingerprint;
    target
        .global_params
        .node
        .access_policy
        .set(&source_fingerprint, DeviceGrants::my_device())?;
    target
        .global_params
        .node
        .access_policy
        .set(&host_fingerprint, DeviceGrants::my_device())?;
    let target_address = paired_full(&target, &source)
        .await
        .context("pair target renderer")?;
    host.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;

    let target_id = session_audio(&target, "handoff-target-before-corrupt.ogg").await?;
    let target_during_handoff_id =
        session_audio(&target, "handoff-target-edited-during-handoff.ogg").await?;
    let target_during_handoff_item = library_track_for_id(&target, target_during_handoff_id).await?;
    queue_session(&target, &[target_id]).await?;
    target
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    target.global_params.player.lock().await.load(0);
    target.global_params.player.lock().await.play();
    let target_before =
        wait_player(&target, |status| status.state == PlayerPlaybackState::Playing).await?;
    let target_track_id = file_ids_by_uuid(
        &target_replica.global_params.test_library().main_db,
        &[host_item.file_uuid],
    )
    .await?
    .get(&host_item.file_uuid)
    .copied()
    .context("target replica track id")?;
    let target_file = Path::new(&target_replica.global_params.test_library().media_root)
        .join("handoff-source-corrupt-on-target.ogg");
    if target_file.exists() {
        fs::remove_file(&target_file)?;
    }
    media_files::Entity::update(media_files::ActiveModel {
        id: Set(target_track_id),
        local_available: Set(false),
        content_hash: Set(None),
        content_size: Set(None),
        ..Default::default()
    })
    .exec(&*target_replica.global_params.test_library().main_db)
    .await?;

    let handoff = HandoffRequest {
        call_id: None,
        target: Some(target_fingerprint.clone()),
    };
    // The source plays on until a handoff pauses it, so where it resumes is compared with where
    // it stood right before the handoff was asked for.
    let mut expected_position = source.global_params.player.lock().await.get_status().position;
    let unplayable_result = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&source.global_params, &handoff),
    )
    .await
    .context("handoff with no playable target item did not fail promptly")?
    .context("dispatch handoff with no playable target item");
    assert!(
        unplayable_result
            .as_ref()
            .is_err_and(|error| error.chain().any(|cause| {
                cause
                    .downcast_ref::<ExpectedRequestError>()
                    .is_some_and(|expected| {
                        matches!(
                            expected,
                            ExpectedRequestError::Unavailable { .. }
                                | ExpectedRequestError::Remote {
                                    kind: RequestErrorKind::Unavailable
                                        | RequestErrorKind::NotFound,
                                    ..
                                }
                        )
                    })
            })),
        "handoff with no playable target item should return a typed failure: {unplayable_result:?}"
    );
    let target_after_unplayable = wait_player(&target, |status| {
        status.playlist == target_before.playlist && status.item == target_before.item
    })
    .await?;
    assert_eq!(target_after_unplayable.playlist, target_before.playlist);
    assert_eq!(target_after_unplayable.index, target_before.index);
    assert_eq!(target_after_unplayable.item, target_before.item);
    let source_after_unplayable = wait_player(&source, |status| {
        status.item == Some(expected_item.clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    assert!(
        source_after_unplayable.position.abs_diff(expected_position) <= Duration::from_millis(1500)
    );

    let host_file = Path::new(&host.global_params.test_library().media_root)
        .join("handoff-source-corrupt-on-target.ogg");
    fs::write(&host_file, b"not an audio file")?;
    materialize_replica_file(
        &target_replica,
        target_track_id,
        &host_file,
        "handoff-source-corrupt-on-target.ogg",
    )
    .await?;
    fs::write(&target_file, b"not an audio file")?;
    let controller_link = host
        .global_params
        .node
        .connect_renderer(&target_fingerprint, RendererLinkUse::Control)
        .await?
        .0;
    // Let the real decode failure initiate rollback, then hold that request at the
    // test's WebSocket boundary until the other controller's edit has committed.
    let rollback_entered = Arc::new(Notify::new());
    let release_rollback = Arc::new(Notify::new());
    let target_node = Arc::downgrade(&target.global_params.node);
    let entered = rollback_entered.clone();
    let release = release_rollback.clone();
    let take_back = "TakeBackHandoffBlockRequest";
    target
        .global_params
        .node
        .websocket_service
        .override_handler(take_back, move |payload, session, library_id| {
            let target_node = target_node.clone();
            let entered = entered.clone();
            let release = release.clone();
            async move {
                entered.notify_one();
                release.notified().await;
                let node = target_node.upgrade().expect("target node");
                dispatch_remote(take_back, &payload, session, library_id, &node)
                    .await
                    .expect("real handoff rollback handler")
            }
        })
        .await;
    let source_params = source.global_params.clone();
    let handoff_request = HandoffRequest {
        call_id: None,
        target: handoff.target.clone(),
    };
    expected_position = source.global_params.player.lock().await.get_status().position;
    let handoff_task = tokio::spawn(async move {
        dispatch_gui_request(&source_params, &handoff_request).await
    });
    let concurrent_edit = async {
        timeout(TEST_DEADLINE, rollback_entered.notified())
            .await
            .context("decode failure should request a handoff rollback")?;
        let before_rollback = target.global_params.player.lock().await.get_status();
        assert!(before_rollback.playlist.iter().any(|entry| {
            entry.item == PlayingItem::LibraryTrack {
                library_id: target_replica.global_params.library_id.unwrap(),
                track: host_item.file_uuid.to_string(),
            }
        }));
        controller_link
            .call(&LoadResolvedItemsRequest {
                call_id: None,
                items: vec![PlayingItemRef::LibraryTrack {
                    library_id: target_during_handoff_item.library_id.to_string(),
                    track: target_during_handoff_item.file_uuid.to_string(),
                }],
                start_index: None,
                mode: PlaylistOperateMode::AppendToEnd,
                expected_revision: None,
                playback_mode: None,
                position_seconds: None,
                play: LoadPlay::Auto,
            })
            .await?;
        wait_player(&target, |status| {
            status
                .playlist
                .iter()
                .any(|entry| entry.item == PlayingItem::from(target_during_handoff_item))
        })
        .await
        .context("concurrent target queue edit before rollback")
    }
    .await;
    release_rollback.notify_one();
    concurrent_edit?;
    let result = handoff_task
        .await
        .context("join corrupt-track handoff")?
        .context("dispatch corrupt-track handoff");
    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.chain().any(|cause| {
                cause
                    .downcast_ref::<ExpectedRequestError>()
                    .is_some_and(|expected| {
                        matches!(
                            expected,
                            ExpectedRequestError::Unavailable { .. }
                                | ExpectedRequestError::Remote {
                                    kind: RequestErrorKind::Unavailable
                                        | RequestErrorKind::NotFound,
                                    ..
                                }
                        )
                    })
            })),
        "decode failure should be returned as a typed unavailable error: {result:?}"
    );
    let target_after = wait_player(&target, |status| {
        status.item == target_before.item
            && status.playlist.iter().any(|entry| {
                entry.item == PlayingItem::from(target_during_handoff_item)
            })
            && !status.playlist.iter().any(|entry| {
                entry.item == PlayingItem::LibraryTrack {
                    library_id: target_replica.global_params.library_id.unwrap(),
                    track: host_item.file_uuid.to_string(),
                }
            })
    })
    .await
    .context("rollback should remove the handoff block and retain the concurrent queue edit")?;
    assert_eq!(target_after.index, target_before.index);
    assert_eq!(target_after.item, target_before.item);
    // The rollback undoes the handoff completely on the target: the playback mode it replaced
    // is back and the interrupted track goes on playing, as it did before the interruption.
    assert_eq!(
        target_after.state,
        PlayerPlaybackState::Playing,
        "the interrupted track resumes playing after the rollback: {target_after:?}"
    );
    assert_eq!(
        target_after.playback_mode,
        PlaybackMode::RepeatAll,
        "the rollback restores the playback mode the handoff replaced: {target_after:?}"
    );
    let source_after =
        wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    assert_eq!(source_after.item, Some(expected_item));
    assert!(source_after.position.abs_diff(expected_position) <= Duration::from_millis(1500));
    source.global_params.node.shutdown().await?;
    target.global_params.node.shutdown().await?;
    host.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_is_not_confirmed_by_this_devices_own_playing_status() -> Result<()> {
    use crate::backends::remote::decode_message;
    use crate::messages::HandoffRequest;
    use tokio::sync::broadcast::error::RecvError;

    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;
    let source = fixture(source_dir.path()).await?;
    let target = fixture(target_dir.path()).await?;
    let id = session_audio(&source, "handoff-own-status.ogg").await?;
    let item = library_track_for_id(&source, id).await?;
    let (_, target_replica) = replicate_host_library(&source, &target).await?;
    let target_address = paired_full(&target, &source).await?;
    let target_fingerprint = info(&target).await?.fingerprint;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;
    // The target has the track, but its copy does not decode, so it never starts it.
    let target_track_id = file_ids_by_uuid(
        &target_replica.global_params.test_library().main_db,
        &[item.file_uuid],
    )
    .await?
    .get(&item.file_uuid)
    .copied()
    .context("target replica track id")?;
    let corrupt = target_dir.path().join("handoff-own-status-corrupt.ogg");
    fs::write(&corrupt, b"not an audio file")?;
    materialize_replica_file(
        &target_replica,
        target_track_id,
        &corrupt,
        "handoff-own-status.ogg",
    )
    .await?;

    queue_session(&source, &[id]).await?;
    source
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    source.global_params.player.lock().await.load(0);
    source.global_params.player.lock().await.play();
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;

    // The target's session reports nothing while its UI library is held, as when it lags behind.
    let target_ui = target.global_params.node.current_ui().await?;
    let target_reports = target_ui.ui_library.lock().await;
    let source_params = source.global_params.clone();
    let handoff_request = HandoffRequest {
        call_id: None,
        target: Some(target_fingerprint),
    };
    let handoff = tokio::spawn(async move {
        dispatch_gui_request(&source_params, &handoff_request).await
    });
    let handed_off = PlayingItem::from(item);
    wait_player(&target, |status| {
        status.playlist.iter().any(|entry| entry.item == handed_off)
    })
    .await?;

    // Before the target answers, this device reports the handed-off track playing at the
    // handed-off place, as its own late statuses do under load.
    let handed_off_ref = PlayingItemRef::LibraryTrack {
        library_id: item.library_id.to_string(),
        track: item.file_uuid.to_string(),
    };
    let mut source_reports = source
        .global_params
        .node
        .websocket_service
        .broadcast_tx
        .subscribe();
    source.global_params.player.lock().await.play();
    timeout(TEST_DEADLINE, async {
        loop {
            let frame = match source_reports.recv().await {
                Ok(frame) => frame,
                Err(RecvError::Lagged(_)) => continue,
                Err(error) => return Err(Error::new(error)),
            };
            let Some((name, payload, _)) = decode_message(&frame) else {
                continue;
            };
            if name != "PlaybackStatus" {
                continue;
            }
            let status = rinf::deserialize::<PlaybackStatus>(&payload)?;
            if status.state == PlaybackState::Playing
                && status.item_request.as_ref() == Some(&handed_off_ref)
            {
                return Ok(());
            }
        }
    })
    .await
    .context("this device did not report the handed-off track playing")??;
    drop(target_reports);

    let result = timeout(TEST_DEADLINE, handoff)
        .await
        .context("handoff did not finish")?
        .context("join handoff")?;
    assert!(
        result.as_ref().is_err_and(|error| error.chain().any(|cause| matches!(
            cause.downcast_ref::<ExpectedRequestError>(),
            Some(
                ExpectedRequestError::Unavailable { .. }
                    | ExpectedRequestError::Remote {
                        kind: RequestErrorKind::Unavailable,
                        ..
                    }
            )
        ))),
        "a handoff the target never started should fail as unavailable: {result:?}"
    );
    wait_player(&target, |status| {
        !status.playlist.iter().any(|entry| entry.item == handed_off)
    })
    .await
    .context("the failed handoff should take its block back from the target")?;
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    source.global_params.node.shutdown().await?;
    target.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_of_a_paused_session_leaves_the_target_paused_at_its_position() -> Result<()> {
    use crate::messages::HandoffRequest;

    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;
    let source = fixture(source_dir.path()).await?;
    let target = fixture(target_dir.path()).await?;
    let ids = [
        session_audio(&source, "handoff-paused-1.ogg").await?,
        session_audio(&source, "handoff-paused-2.ogg").await?,
        session_audio(&source, "handoff-paused-3.ogg").await?,
        session_audio(&source, "handoff-paused-target-1.ogg").await?,
        session_audio(&source, "handoff-paused-target-2.ogg").await?,
    ];
    let items = future::join_all(ids.iter().map(|id| library_track_for_id(&source, *id)))
        .await
        .into_iter()
        .map(|item| item.map(PlayingItem::from))
        .collect::<Result<Vec<_>>>()?;
    let (_, target_replica) = replicate_host_library(&source, &target).await?;
    let target_address = paired_full(&target, &source).await?;
    let target_fingerprint = info(&target).await?.fingerprint;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;

    // The target plays its own queue.
    let target_queue = items[3..].to_vec();
    let target_tracks = target_queue.clone();
    target_replica
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                target_tracks
                    .into_iter()
                    .map(|item| (item, PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    target
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    target.global_params.player.lock().await.load(0);
    target.global_params.player.lock().await.play();
    wait_player(&target, |status| {
        status.state == PlayerPlaybackState::Playing && status.item == Some(target_queue[0].clone())
    })
    .await?;

    // This device has played its second track past the tolerance of the place check and is
    // paused there.
    let source_queue = items[..3].to_vec();
    let source_tracks = source_queue.clone();
    source
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                source_tracks
                    .into_iter()
                    .map(|item| (item, PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&source, |status| status.playlist.len() == 3).await?;
    source
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    source.global_params.player.lock().await.switch(1);
    source.global_params.player.lock().await.play();
    wait_player(&source, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.index == Some(1)
            && status.position >= Duration::from_secs(2)
    })
    .await?;
    source.global_params.player.lock().await.pause();
    let source_paused =
        wait_player(&source, |status| status.state == PlayerPlaybackState::Paused).await?;

    let handoff = HandoffRequest {
        call_id: None,
        target: Some(target_fingerprint),
    };
    let result = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&source.global_params, &handoff),
    )
    .await
    .context("paused handoff did not finish")?;
    assert!(
        result.is_ok(),
        "a paused session should hand off to a renderer that loads it: {result:?}"
    );

    // The handed-off block follows the target's current track, and its track waits paused at
    // the handed-off place.
    let expected_queue = vec![
        target_queue[0].clone(),
        source_queue[0].clone(),
        source_queue[1].clone(),
        source_queue[2].clone(),
        target_queue[1].clone(),
    ];
    let target_after = wait_player(&target, |status| {
        status.playlist.iter().map(|entry| entry.item.clone()).collect::<Vec<_>>()
            == expected_queue
            && status.index == Some(2)
            && status.item == source_paused.item
            && status.state == PlayerPlaybackState::Paused
    })
    .await?;
    assert!(target_after.position.abs_diff(source_paused.position) <= Duration::from_millis(1500));
    let source_after = source.global_params.player.lock().await.get_status();
    assert_eq!(source_after.state, PlayerPlaybackState::Paused);
    assert_eq!(source_after.item, source_paused.item);
    source.global_params.node.shutdown().await?;
    target.global_params.node.shutdown().await?;
    Ok(())
}

/// A source that hosts one track and a renderer it can hand off to, whose copy of the track
/// does not decode: a handoff there fails once the target has the block.
async fn handoff_source_and_undecodable_target(
    name: &str,
) -> Result<(Vec<TempDir>, Fixture, Fixture, Fixture, String, i32)> {
    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;
    let source = fixture(source_dir.path()).await?;
    let target = fixture(target_dir.path()).await?;
    let file_name = format!("{name}.ogg");
    let id = session_audio(&source, &file_name).await?;
    let item = library_track_for_id(&source, id).await?;
    let (_, target_replica) = replicate_host_library(&source, &target).await?;
    let target_address = paired_full(&target, &source).await?;
    let target_fingerprint = info(&target).await?.fingerprint;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;
    let target_track_id = file_ids_by_uuid(
        &target_replica.global_params.test_library().main_db,
        &[item.file_uuid],
    )
    .await?
    .get(&item.file_uuid)
    .copied()
    .context("target replica track id")?;
    let corrupt = target_dir.path().join(format!("{name}-corrupt.ogg"));
    fs::write(&corrupt, b"not an audio file")?;
    materialize_replica_file(&target_replica, target_track_id, &corrupt, &file_name).await?;
    queue_session(&source, &[id]).await?;
    source
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    Ok((
        vec![source_dir, target_dir],
        source,
        target,
        target_replica,
        target_fingerprint,
        id,
    ))
}

/// Asks for a handoff right after Play was pressed on the source, and expects the handoff to
/// fail at the target and the source to play on.
async fn hand_off_right_after_play_and_expect_resume(
    source: &Fixture,
    target_fingerprint: String,
) -> Result<()> {
    use crate::messages::HandoffRequest;

    source.global_params.player.lock().await.play();
    let handoff = HandoffRequest {
        call_id: None,
        target: Some(target_fingerprint),
    };
    let result = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&source.global_params, &handoff),
    )
    .await
    .context("handoff to an undecodable target did not finish")?;
    assert!(
        result.as_ref().is_err_and(|error| error.chain().any(|cause| matches!(
            cause.downcast_ref::<ExpectedRequestError>(),
            Some(
                ExpectedRequestError::Unavailable { .. }
                    | ExpectedRequestError::Remote {
                        kind: RequestErrorKind::Unavailable,
                        ..
                    }
            )
        ))),
        "a handoff to an undecodable target should fail as unavailable: {result:?}"
    );
    wait_player(source, |status| status.state == PlayerPlaybackState::Playing)
        .await
        .context("the source should play on after the failed handoff")?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn failed_handoff_resumes_a_paused_source_whose_play_was_just_pressed() -> Result<()> {
    let (_dirs, source, target, _target_replica, target_fingerprint, _) =
        handoff_source_and_undecodable_target("handoff-play-pressed").await?;
    source.global_params.player.lock().await.load(0);
    source.global_params.player.lock().await.play();
    wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    source.global_params.player.lock().await.pause();
    wait_player(&source, |status| status.state == PlayerPlaybackState::Paused).await?;

    hand_off_right_after_play_and_expect_resume(&source, target_fingerprint).await?;
    source.global_params.node.shutdown().await?;
    target.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn failed_handoff_resumes_a_source_whose_play_reopens_its_track() -> Result<()> {
    let (_dirs, source, target, _target_replica, target_fingerprint, id) =
        handoff_source_and_undecodable_target("handoff-play-reopens").await?;
    // The track comes up paused without an output, as a restored session does, so Play loads it
    // again before it plays.
    source
        .global_params
        .player
        .lock()
        .await
        .load_at_position(0, Duration::from_secs(1), false);
    let item = PlayingItem::from(library_track_for_id(&source, id).await?);
    wait_player(&source, |status| {
        status.state == PlayerPlaybackState::Paused
            && status.ready
            && status.item == Some(item.clone())
    })
    .await?;

    hand_off_right_after_play_and_expect_resume(&source, target_fingerprint).await?;
    source.global_params.node.shutdown().await?;
    target.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_moves_queue_and_position_and_preserves_source_sessions() -> Result<()> {
    use crate::messages::{
        HandoffBlockId, HandoffRequest, MovePlaylistItemRequest, RemoveRequest,
        SetPlaybackModeRequest, TakeBackHandoffBlockRequest,
    };
    use crate::request_error::request_failure_cause;
    use crate::utils::ParamsExtractor;

    let h_dir = tempfile::tempdir()?;
    let g_dir = tempfile::tempdir()?;
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let c_dir = tempfile::tempdir()?;
    let h = fixture(h_dir.path()).await?;
    let g = fixture(g_dir.path()).await?;
    let a_signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        a_signals.clone(),
    )
    .await?;
    let b_signals = Arc::new(RecordedSignals::default());
    let b = fixture_with_broadcaster(
        b_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        b_signals.clone(),
    )
    .await?;
    let c = fixture(c_dir.path()).await?;

    let h_ids = [
        session_audio(&h, "handoff-h1.ogg").await?,
        session_audio(&h, "handoff-h2.ogg").await?,
        session_audio(&h, "handoff-h3.ogg").await?,
        session_audio(&h, "handoff-h4.ogg").await?,
        session_audio(&h, "handoff-h5.ogg").await?,
        session_audio(&h, "handoff-h6.ogg").await?,
    ];
    let g_id = session_audio(&g, "handoff-g.ogg").await?;
    let (h_address, _a_h_replica) = replicate_host_library(&h, &a).await?;
    let (g_address, a_g_replica) = replicate_host_library(&g, &a).await?;
    pair(&h, &b).await?;
    pair(&b, &h).await?;
    pair(&g, &b).await?;
    pair(&b, &g).await?;
    let (_, _b_h) = replicate_host_library_at(&h, &b, h_address).await?;
    let (_, _b_g) = replicate_host_library_at(&g, &b, g_address).await?;
    pair(&h, &c).await?;
    pair(&c, &h).await?;
    let (_, _c_h) = replicate_host_library_at(&h, &c, h_address).await?;
    let b_fingerprint = info(&b).await?.fingerprint;
    let b_address = paired_full(&b, &a).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;
    let c_fingerprint = info(&c).await?.fingerprint;
    let c_address = paired_full(&c, &a).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &c_fingerprint,
            vec![format!("127.0.0.1:{}", c_address.port())],
        )
        .await?;

    let h_items = future::join_all(h_ids.iter().map(|id| library_track_for_id(&h, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let g_item = library_track_for_id(&g, g_id).await?;
    let queue = vec![
        h_items[0], g_item, h_items[1], h_items[2], h_items[3], h_items[4],
    ];
    let source_queue_len = queue.len();
    let g_source = g.global_params.test_library();
    let a_g_library = a_g_replica.global_params.test_library();
    let g_local_id = file_ids_by_uuid(&a_g_library.main_db, &[g_item.file_uuid])
        .await?
        .get(&g_item.file_uuid)
        .copied()
        .context("replica G file id")?;
    let controller = &a_g_replica;
    let shuffle = SetPlaybackModeRequest {
        call_id: None,
        mode: WirePlaybackMode::Shuffle,
    };
    shuffle
        .handle(
            shuffle.extract_params(&controller.global_params),
            None,
            &shuffle,
        )
        .await?;
    let g_row = media_files::Entity::find_by_id(g_id)
        .one(&*g_source.main_db)
        .await?
        .context("source G row")?;
    materialize_replica_file(
        controller,
        g_local_id,
        &Path::new(&g_source.media_root)
            .join(&g_row.directory)
            .join(&g_row.file_name),
        "handoff-materialized.ogg",
    )
    .await?;
    controller
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                queue
                    .into_iter()
                    .map(|item| (PlayingItem::from(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(controller, |status| {
        status.playlist.len() == source_queue_len
    })
    .await?;
    controller.global_params.player.lock().await.switch(1);
    wait_player(controller, |status| {
        status.index.is_some_and(|index| index > 0)
    })
    .await?;
    controller.global_params.player.lock().await.play();
    wait_player(controller, |status| status.state == PlayerPlaybackState::Playing).await?;
    let target_ids = [
        session_audio(&b, "handoff-target-a.ogg").await?,
        session_audio(&b, "handoff-target-b.ogg").await?,
        session_audio(&b, "handoff-target-c.ogg").await?,
        session_audio(&b, "handoff-target-d.ogg").await?,
        session_audio(&b, "handoff-target-e.ogg").await?,
    ];
    let target_items =
        future::join_all(target_ids.iter().map(|id| library_track_for_id(&b, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let target_queue = target_items
        .iter()
        .copied()
        .map(PlayingItem::from)
        .collect::<Vec<_>>();
    b.global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                target_queue
                    .iter()
                    .cloned()
                    .map(|item| (item, PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&b, |status| status.playlist == target_queue).await?;
    b.global_params.player.lock().await.switch(2);
    wait_player(&b, |status| status.index == Some(2)).await?;
    b.global_params.player.lock().await.play();
    wait_player(&b, |status| status.state == PlayerPlaybackState::Playing).await?;
    b.global_params.player.lock().await.seek(1.1);
    let source_before = wait_player(controller, |status| {
        status.state == PlayerPlaybackState::Playing && status.position >= Duration::from_millis(1900)
    })
    .await?;
    let target_before = wait_player(&b, |status| status.position >= Duration::from_secs(1)).await?;
    let sampled_at = Instant::now();
    let source_queue = source_before
        .playlist
        .iter()
        .map(|entry| entry.item.clone())
        .collect::<Vec<_>>();
    let source_index = source_before.index.context("shuffle source index")?;

    let handoff = HandoffRequest {
        call_id: None,
        target: Some(b_fingerprint.clone()),
    };
    a_signals.0.lock().unwrap().clear();
    dispatch_gui_request(&controller.global_params, &handoff).await?;
    let handoff_elapsed = sampled_at.elapsed();
    let b_status = wait_player(&b, |status| {
        status.index == Some(3 + source_index)
            && status.item == source_before.item
            && status.state == PlayerPlaybackState::Playing
            && status.playlist.len() == target_queue.len() + source_queue.len()
            && status.playlist[..3] == target_queue[..3]
            && status.playlist[3 + source_queue.len()..] == target_queue[3..]
    })
    .await?;
    let remote_session_id = b.global_params.node.session()?.session_id().await;
    // B keeps playing between the first sample and the handoff load. Check
    // the saved interruption against that interval, then check the actual
    // takeback against the saved cursor rather than the earlier sample.
    let block = b_status.playlist[3 + source_index]
        .block
        .context("inserted block")?;
    let store = b_dir.path().join("sessions/local.json");
    let interrupted = wait_until("persisted handoff block", || async {
        Ok(SessionState::read(&store)?.handoff_block(block))
    })
    .await?;
    let StoredInterruptedItem::Item { item, position, .. } = interrupted.interrupted else {
        anyhow::bail!("playing target was not captured");
    };
    assert_eq!(Some(PlayingItem::from(item)), target_before.item);
    let tolerance = Duration::from_millis(1500);
    assert!(
        position >= Duration::from_secs(1)
            && position + tolerance >= target_before.position
            && position <= target_before.position + handoff_elapsed + tolerance,
        "interruption {position:?}, sample {:?}, elapsed {handoff_elapsed:?}",
        target_before.position
    );
    wait_until("renderer queue refresh after handoff", || async {
        let has_remote = a_signals.0.lock().unwrap().iter().any(|(name, payload)| {
            name == "PlaylistUpdate"
                && rinf::deserialize::<PlaylistUpdate>(payload)
                    .is_ok_and(|update| update.session_id == remote_session_id)
        });
        Ok(has_remote.then_some(()))
    })
    .await
    .context("renderer queue refresh did not reach local GUI after handoff")?;
    let refreshed = a_signals
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "PlaylistUpdate")
        .filter_map(|(_, payload)| rinf::deserialize::<PlaylistUpdate>(payload).ok())
        .find(|update| update.session_id == remote_session_id)
        .context("renderer PlaylistUpdate after handoff")?;
    assert_eq!(
        refreshed.revision,
        b.global_params.node.session()?.queue_revision().await
    );
    let mut combined_queue = target_queue[..3].to_vec();
    combined_queue.extend(source_queue.clone());
    combined_queue.extend(target_queue[3..].iter().cloned());
    assert_eq!(b_status.playlist, combined_queue);
    let a_paused = wait_player(controller, |status| {
        status.state == PlayerPlaybackState::Paused && status.playlist == source_queue
    })
    .await?;
    assert!(b_status.position.abs_diff(a_paused.position) <= Duration::from_millis(1500));
    assert_eq!(a_paused.playlist, source_queue);
    assert_eq!(a_paused.index, Some(source_index));
    assert_eq!(
        controller
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(b_fingerprint.as_str())
    );

    let unknown_block = TakeBackHandoffBlockRequest {
        block: HandoffBlockId {
            uuid: Uuid::new_v4().to_string(),
        },
        resume: false,
        call_id: None,
    };
    let b_link = controller
        .global_params
        .node
        .renderer_link()?
        .context("renderer link for unknown take-back")?;
    let target_before_unknown = b.global_params.player.lock().await.get_status();
    let source_before_unknown = controller.global_params.player.lock().await.get_status();
    let unknown_error = match b_link.call(&unknown_block).await {
        Ok(_) => anyhow::anyhow!("unknown handoff block was accepted"),
        Err(error) => error,
    };
    assert_eq!(classify(&unknown_error), RequestErrorKind::Unavailable);
    let target_after_unknown = b.global_params.player.lock().await.get_status();
    let source_after_unknown = controller.global_params.player.lock().await.get_status();
    assert_eq!(
        target_after_unknown.playlist,
        target_before_unknown.playlist
    );
    assert_eq!(target_after_unknown.index, target_before_unknown.index);
    assert_eq!(target_after_unknown.item, target_before_unknown.item);
    assert_eq!(target_after_unknown.state, target_before_unknown.state);
    assert_eq!(
        source_after_unknown.playlist,
        source_before_unknown.playlist
    );
    assert_eq!(source_after_unknown.index, source_before_unknown.index);
    assert_eq!(source_after_unknown.item, source_before_unknown.item);
    assert_eq!(source_after_unknown.state, source_before_unknown.state);

    let inserted_item = PlayingItem::from(h_items[5]);
    let mut expected_block = source_queue.clone();
    expected_block.insert(source_index + 1, inserted_item.clone());
    b_link
        .call(&LoadResolvedItemsRequest {
            call_id: None,
            items: vec![h_items[5].into()],
            start_index: Some(0),
            mode: PlaylistOperateMode::PlayNext,
            expected_revision: None,
            playback_mode: None,
            position_seconds: Some(0.0),
            play: LoadPlay::Play,
        })
        .await?;
    let after_insert = wait_player(&b, |status| {
        status
            .playlist
            .iter()
            .any(|entry| entry.item == inserted_item)
    })
    .await?;
    let remove_index = after_insert
        .playlist
        .iter()
        .position(|entry| entry.item == PlayingItem::from(h_items[4]) && entry.block.is_some())
        .context("handoff block item five")?;
    b_link
        .call(&RemoveRequest {
            call_id: None,
            index: remove_index as u32,
            expected_revision: None,
        })
        .await?;
    expected_block.retain(|item| item != &PlayingItem::from(h_items[4]));
    let after_remove = wait_player(&b, |status| {
        !status
            .playlist
            .iter()
            .any(|entry| entry.item == PlayingItem::from(h_items[4]))
    })
    .await?;
    let tagged_indices = after_remove
        .playlist
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.block.is_some().then_some(index))
        .collect::<Vec<_>>();
    b_link
        .call(&MovePlaylistItemRequest {
            call_id: None,
            old_index: tagged_indices[0] as u32,
            new_index: tagged_indices[3] as u32,
            expected_revision: None,
        })
        .await?;
    let moved = expected_block.remove(0);
    expected_block.insert(3, moved);

    let outside_id = session_audio(&b, "handoff-target-outside.ogg").await?;
    let outside_item = PlayingItem::from(library_track_for_id(&b, outside_id).await?);
    let outside_item_ref = match &outside_item {
        PlayingItem::LibraryTrack { library_id, track } => PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: track.clone(),
        },
        _ => unreachable!(),
    };
    b_link
        .call(&LoadResolvedItemsRequest {
            call_id: None,
            items: vec![outside_item_ref],
            start_index: None,
            mode: PlaylistOperateMode::AppendToEnd,
            expected_revision: None,
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
        })
        .await?;
    let mut expected_target_queue = target_queue.clone();
    expected_target_queue.push(outside_item.clone());
    wait_player(&b, |status| {
        status
            .playlist
            .last()
            .is_some_and(|entry| entry.item == outside_item)
    })
    .await?;

    b.global_params.player.lock().await.pause();
    wait_player(&b, |status| status.state == PlayerPlaybackState::Paused).await?;
    b.global_params.player.lock().await.play();
    wait_player(&b, |status| status.state == PlayerPlaybackState::Playing).await?;
    let block_start = b
        .global_params
        .player
        .lock()
        .await
        .get_status()
        .playlist
        .iter()
        .position(|entry| entry.block.is_some())
        .context("handoff block start")?;
    b.global_params.player.lock().await.switch(block_start);
    wait_player(&b, |status| {
        status.index == Some(block_start) && status.state == PlayerPlaybackState::Playing
    })
    .await?;
    let b_before_return = b.global_params.player.lock().await.get_status();
    let block_items = b_before_return
        .playlist
        .iter()
        .filter_map(|entry| entry.block.is_some().then_some(entry.item.clone()))
        .collect::<Vec<_>>();
    let current_index = b_before_return.index.context("handoff block index")?;
    let block_index = b_before_return.playlist[..current_index]
        .iter()
        .filter(|entry| entry.block.is_some())
        .count();
    assert_eq!(block_items, expected_block);
    b_signals.clear();
    let back = HandoffRequest {
        call_id: None,
        target: None,
    };
    dispatch_gui_request(&controller.global_params, &back).await?;
    let a_returned = wait_player(controller, |status| {
        status.index == Some(block_index)
            && status.item == b_before_return.item
            && status.state == PlayerPlaybackState::Playing
            && status.playlist == block_items
            && status.position.abs_diff(b_before_return.position) <= Duration::from_millis(1500)
    })
    .await?;
    assert_eq!(a_returned.playlist, block_items);
    let b_session_id = b
        .global_params
        .node
        .session()?
        .session_id()
        .await
        .to_string();
    let b_paused_source_position = wait_until("paused renderer source position", || async {
        let position = b_signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .into_iter()
            .find(|status| {
                status.state == PlaybackState::Paused
                    && status.index == Some(current_index as i32)
                    && status.session_id == b_session_id
            })
            .map(|status| Duration::from_secs_f64(status.progress_seconds.max(0.0) as f64));
        Ok(position)
    })
    .await?;
    assert!(a_returned.position.abs_diff(b_paused_source_position) <= Duration::from_millis(1500));
    assert!(controller.global_params.node.renderer_link()?.is_none());
    let b_paused = wait_player(&b, |status| {
        status.state == PlayerPlaybackState::Paused && status.playlist == expected_target_queue
    })
    .await?;
    assert_eq!(b_paused.item, target_before.item);
    assert!(
        b_paused.position.abs_diff(position) <= tolerance,
        "restored {:?}, interrupted at {position:?}",
        b_paused.position
    );
    controller.global_params.player.lock().await.pause();
    wait_player(controller, |status| status.state == PlayerPlaybackState::Paused).await?;

    controller.global_params.player.lock().await.play();
    let a_before_refused =
        wait_player(controller, |status| status.state == PlayerPlaybackState::Playing).await?;
    let c_before = c.global_params.player.lock().await.get_playlist();
    let refused = HandoffRequest {
        call_id: None,
        target: Some(c_fingerprint),
    };
    let error = dispatch_gui_request(&controller.global_params, &refused)
        .await
        .unwrap_err();
    let g_library_id = g
        .global_params
        .library_id
        .context("G library id")?
        .to_string();
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::CastNotPossible {
            library_id: g_library_id,
            state: CastLibraryState::NotHeld,
        })
    );
    assert!(controller.global_params.node.renderer_link()?.is_none());
    wait_player(controller, |status| status.state == PlayerPlaybackState::Playing).await?;
    assert_eq!(
        controller
            .global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist,
        a_before_refused.playlist
    );
    assert_eq!(c.global_params.player.lock().await.get_playlist(), c_before);
    controller.global_params.player.lock().await.pause();
    wait_player(controller, |status| status.state == PlayerPlaybackState::Paused).await?;

    controller
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    PlayingItem::IndependentFile("outside-library.ogg".into()),
                    PlaybackSource::Local(PathBuf::from("unused-handoff-test-path")),
                )],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    let independent_before = wait_player(controller, |status| {
        status.state == PlayerPlaybackState::Paused && status.playlist.len() == source_queue.len() + 1
    })
    .await?;
    let b_before_independent = b.global_params.player.lock().await.get_status();
    let refused_independent = HandoffRequest {
        call_id: None,
        target: Some(b_fingerprint.clone()),
    };
    let error = refused_independent
        .handle(
            refused_independent.extract_params(&controller.global_params),
            None,
            &refused_independent,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "handing off files outside a library"
        })
    ));
    let independent_after = controller.global_params.player.lock().await.get_status();
    assert_eq!(independent_after.state, PlayerPlaybackState::Paused);
    assert_eq!(independent_after.playlist, independent_before.playlist);
    assert_eq!(independent_after.index, independent_before.index);
    assert!(controller.global_params.node.renderer_link()?.is_none());
    let b_after_independent = b.global_params.player.lock().await.get_status();
    assert_eq!(b_after_independent.playlist, b_before_independent.playlist);
    assert_eq!(b_after_independent.index, b_before_independent.index);
    assert_eq!(b_after_independent.position, b_before_independent.position);

    controller
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.remove_from_playlist(source_queue.len());
            true
        })
        .await?;
    controller.global_params.player.lock().await.switch(0);
    wait_player(controller, |status| status.state == PlayerPlaybackState::Playing).await?;
    let second_handoff = HandoffRequest {
        call_id: None,
        target: Some(b_fingerprint.clone()),
    };
    dispatch_gui_request(&controller.global_params, &second_handoff).await?;
    let after_second_handoff = wait_player(&b, |status| {
        status.playlist.len() > target_queue.len() && status.playlist[..3] == target_queue[..3]
    })
    .await?;
    let target_d_index = after_second_handoff
        .playlist
        .iter()
        .position(|item| item == &target_queue[3])
        .context("target D after the inserted block")?;
    b.global_params.player.lock().await.switch(target_d_index);
    wait_player(&b, |status| status.item == Some(target_queue[3].clone())).await?;
    let second_back = HandoffRequest {
        call_id: None,
        target: None,
    };
    dispatch_gui_request(&controller.global_params, &second_back).await?;
    let returned_stopped = wait_player(controller, |status| {
        status.state == PlayerPlaybackState::Stopped && status.playlist == expected_block
    })
    .await?;
    assert!(returned_stopped.index.is_none());
    let target_kept_playing = wait_player(&b, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.item == Some(target_queue[3].clone())
            && status.playlist == expected_target_queue
    })
    .await?;
    assert_eq!(target_kept_playing.item, Some(target_queue[3].clone()));

    controller.global_params.node.detach_renderer().await?;
    for fixture in [controller, &b, &c, &h, &g] {
        fixture.global_params.node.shutdown().await?;
    }
    Ok(())
}

#[test]
fn session_snapshot_rejects_independent_files() {
    let mut status = MockPlayer.get_status();
    status
        .playlist
        .push(PlayingItem::IndependentFile("outside-library.ogg".into()).into());
    let error = match session_snapshot_from_status(status, 0, &HashSet::new()) {
        Ok(_) => panic!("independent file unexpectedly appeared in a handoff snapshot"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "handing off files outside a library"
        })
    ));
}

#[test]
fn session_snapshot_rejects_subsonic_tracks_without_mutating_the_queue() {
    let subsonic_track = ServiceItem {
        library_id: Uuid::new_v4(),
        track: "remote-track".to_owned(),
    };
    let item = PlayingItem::from(subsonic_track.clone());
    let mut status = MockPlayer.get_status();
    status.playlist.push(QueueEntry::untagged(item.clone()));
    let original = status.playlist.clone();
    let error = match session_snapshot_from_status(
        status.clone(),
        0,
        &HashSet::from([subsonic_track.library_id]),
    ) {
        Ok(_) => panic!("Subsonic track unexpectedly appeared in a handoff snapshot"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "handing off files outside a library"
        })
    ));
    assert_eq!(status.playlist, original);
    let encoded = serde_json::to_string(&PlayingItemRef::from(subsonic_track)).unwrap();
    for secret_marker in ["t=", "s=", "p=", "password"] {
        assert!(!encoded.contains(secret_marker));
    }
}

#[tokio::test]
async fn playback_snapshot_waits_for_accepted_queue_changes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let fixture = super::step("create snapshot fixture", fixture(temp.path())).await?;
    let track = super::step(
        "scan snapshot track",
        session_audio(&fixture, "snapshot-barrier.ogg"),
    )
    .await?;
    let item = super::step(
        "resolve snapshot track",
        library_track_for_id(&fixture, track),
    )
    .await?;
    let session = fixture.global_params.node.session()?;

    super::step(
        "accept queue edit",
        session.edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                vec![(PlayingItem::from(item), PlaybackSource::LibraryStream)],
                AddMode::AppendToEnd,
            );
            true
        }),
    )
    .await?;

    let (status, revision) = timeout(TEST_DEADLINE, session.playback_snapshot())
        .await
        .context("snapshot barrier did not return")?;
    assert_eq!(status.playlist, [PlayingItem::from(item)]);
    assert_eq!(revision, 1);
    super::step(
        "shutdown snapshot fixture",
        fixture.global_params.node.shutdown(),
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn playback_snapshot_on_idle_player_returns_promptly() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let fixture = fixture(temp.path()).await?;
    let session = fixture.global_params.node.session()?;

    let (status, revision) = timeout(TEST_DEADLINE, session.playback_snapshot())
        .await
        .context("idle snapshot barrier did not return")?;
    assert!(status.playlist.is_empty());
    assert_eq!(revision, 0);
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

async fn wait_controller_count(node: &NodeContext, count: usize) -> Result<()> {
    wait_until("the expected attached controller count", || async {
        Ok((node.session()?.attached_controllers().len() == count).then_some(()))
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2b_attached_controllers_list_detach_and_disconnect() -> Result<()> {
    use crate::backends::remote::encode_message;
    use crate::messages::{
        DetachControllerRequest, ListControllersRequest, MovePlaylistItemRequest,
    };
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let renderer_dir = tempfile::tempdir()?;
    let first_controller_dir = tempfile::tempdir()?;
    let second_controller_dir = tempfile::tempdir()?;
    let renderer = fixture(renderer_dir.path()).await?;
    let first_controller = fixture(first_controller_dir.path()).await?;
    let second_controller = fixture(second_controller_dir.path()).await?;
    let (mut first, mut second) =
        connect_two_controllers(&renderer, &first_controller, &second_controller).await?;
    let renderer_node = &renderer.global_params.node;
    wait_controller_count(renderer_node, 2).await?;
    let list_request = ListControllersRequest { call_id: None };
    let controllers = list_request
        .handle(renderer_node.clone(), None, &list_request)
        .await?
        .context("owner controller list response")?
        .controllers;
    let first_fingerprint = info(&first_controller).await?.fingerprint;
    let second_fingerprint = info(&second_controller).await?.fingerprint;
    let first_entry = controllers
        .iter()
        .find(|controller| controller.fingerprint == first_fingerprint)
        .context("first controller missing from owner list")?;
    assert!(!first_entry.alias.is_empty());
    assert!(first_entry.connected_at > 0);
    assert!(
        controllers
            .iter()
            .any(|controller| controller.fingerprint == second_fingerprint)
    );

    let unknown_detach = DetachControllerRequest {
        call_id: None,
        id: Uuid::new_v4().to_string(),
    };
    let error = unknown_detach
        .handle(renderer_node.clone(), None, &unknown_detach)
        .await
        .unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::NotFound);
    let detach = DetachControllerRequest {
        call_id: None,
        id: first_entry.id.clone(),
    };
    detach.handle(renderer_node.clone(), None, &detach).await?;
    let frame = timeout(TEST_DEADLINE, first.next())
        .await?
        .context("detached socket closed without a frame")??;
    assert!(
        matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 4409 && frame.reason == "detached")
    );
    wait_controller_count(renderer_node, 1).await?;

    let first_track = session_audio(&renderer, "attached-controller-first.ogg").await?;
    let second_track = session_audio(&renderer, "attached-controller-second.ogg").await?;
    queue_session(&renderer, &[first_track, second_track]).await?;
    let request_id = Uuid::new_v4();
    second
        .send(Message::Binary(
            encode_message(
                "MovePlaylistItemRequest",
                &rinf::serialize(&MovePlaylistItemRequest {
                    call_id: None,
                    old_index: 0,
                    new_index: 1,
                    expected_revision: Some(0),
                })?,
                Some(request_id),
            )
            .into(),
        ))
        .await?;
    timeout(TEST_DEADLINE, async {
        loop {
            let frame = second
                .next()
                .await
                .context("second controller disconnected")??;
            if let Some((name, payload, _)) = decode_message(&frame.into_data())
                && name == "PlaylistUpdate"
            {
                let update: PlaylistUpdate = rinf::deserialize(&payload)?;
                if update.revision == 1 {
                    assert_eq!(update.items.len(), 2);
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
    })
    .await??;
    assert_eq!(renderer_node.session()?.attached_controllers().len(), 1);

    drop(second);
    wait_controller_count(renderer_node, 0).await?;
    renderer.global_params.node.shutdown().await?;
    first_controller.global_params.node.shutdown().await?;
    second_controller.global_params.node.shutdown().await?;
    Ok(())
}

/// Waits for the session file at `store` to hold a queue of `len` items.
async fn wait_persisted_queue(store: &Path, len: usize) -> Result<()> {
    wait_until("the persisted queue", || async {
        let contents = fs::read(store).unwrap_or_default();
        Ok(serde_json::from_slice::<serde_json::Value>(&contents)
            .is_ok_and(|state| {
                state["queue"]
                    .as_array()
                    .is_some_and(|queue| queue.len() == len)
            })
            .then_some(()))
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_m3u8_and_independent_lyrics_cannot_read_outside_library() -> Result<()> {
    use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants, SyncLevel};
    use crate::backends::remote::connect_remote;
    use crate::messages::{
        CreateM3u8PlaylistRequest, CreateM3u8PlaylistResponse, RemoteRequestFailed,
        RemoveItemFromPlaylistRequest,
    };
    use sea_orm::ConnectionTrait;

    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let controller = fixture(controller_dir.path()).await?;
    let library_id = host.global_params.library_id.context("host library id")?;
    let track_id = session_audio(&host, "remote-import.ogg").await?;
    let media_root = Path::new(&host.global_params.test_library().media_root);
    let inside_playlist = media_root.join("inside.m3u8");
    fs::write(&inside_playlist, "remote-import.ogg\n")?;

    let addr = paired_full(&host, &controller).await?;
    let fingerprint = info(&controller).await?.fingerprint;
    host.global_params.node.access_policy.set(
        &fingerprint,
        DeviceGrants {
            control: true,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, ConnectionLevel::ViewEdit)]
                    .into_iter()
                    .collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, SyncLevel::PullPush)].into_iter().collect(),
            },
            administer: false,
        },
    )?;
    let mut socket = connect_remote(
        "127.0.0.1",
        addr.port(),
        &controller.global_params.config_path,
        controller.global_params.cert_validator.read().await.clone(),
        &controller.global_params.node.sync_tokens,
    )
    .await?;

    let outside_dir = tempfile::tempdir()?;
    let secret = "REMOTE_PATH_SECRET_MARKER";
    let outside_playlist = outside_dir.path().join("secret.m3u8");
    fs::write(&outside_playlist, format!("{secret}\n"))?;
    let outside_request = CreateM3u8PlaylistRequest {
        call_id: None,
        name: "outside remote".into(),
        group: "security".into(),
        path: outside_playlist.to_string_lossy().into_owned(),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "CreateM3u8PlaylistRequest",
        &rinf::serialize(&outside_request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&response_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::InvalidInput);
    assert!(!failure.detail.contains(secret));
    assert!(!String::from_utf8_lossy(&response_payload).contains(secret));

    host.global_params
        .test_library()
        .main_db
        .execute_unprepared(
            "CREATE TRIGGER fail_playlist_insert BEFORE INSERT ON playlists \
             BEGIN SELECT RAISE(ABORT, 'injected playlist failure'); END",
        )
        .await?;
    let failed_create_request = CreateM3u8PlaylistRequest {
        call_id: None,
        name: "failed remote".into(),
        group: "security".into(),
        path: inside_playlist.to_string_lossy().into_owned(),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "CreateM3u8PlaylistRequest",
        &rinf::serialize(&failed_create_request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&response_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::Internal);
    host.global_params
        .test_library()
        .main_db
        .execute_unprepared("DROP TRIGGER fail_playlist_insert")
        .await?;

    let missing_playlist_request = RemoveItemFromPlaylistRequest {
        call_id: None,
        playlist_id: i32::MAX,
        occurrence_id: Uuid::new_v4().to_string(),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "RemoveItemFromPlaylistRequest",
        &rinf::serialize(&missing_playlist_request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&response_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::NotFound);

    let inside_request = CreateM3u8PlaylistRequest {
        call_id: None,
        name: "inside remote".into(),
        group: "security".into(),
        path: inside_playlist.to_string_lossy().into_owned(),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "CreateM3u8PlaylistRequest",
        &rinf::serialize(&inside_request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "CreateM3u8PlaylistResponse");
    let imported: CreateM3u8PlaylistResponse = rinf::deserialize(&response_payload)?;
    assert_eq!(imported.imported_count, Some(1));
    assert!(imported.playlist.is_some());

    let local_request = CreateM3u8PlaylistRequest {
        call_id: None,
        name: "outside local".into(),
        group: "security".into(),
        path: outside_playlist.to_string_lossy().into_owned(),
    };
    let local_response = local_request
        .handle(
            (
                Some(host.global_params.test_library().clone()),
                Arc::new(Quiet),
            ),
            None,
            &local_request,
        )
        .await?
        .context("local playlist import returned no response")?;
    assert_eq!(local_response.imported_count, Some(0));

    assert!(track_id > 0);
    socket.close(None).await?;
    host.manager.stop().await?;
    host.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m3u8_import_reports_ambiguous_paths_and_keeps_occurrence_order() -> Result<()> {
    use database::entities::{media_file_playlists, playlists};
    use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let library = fixture.global_params.test_library().clone();
    let media_root = Path::new(&library.media_root);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");

    for directory in ["disc-a", "disc-b"] {
        let directory = media_root.join(directory);
        fs::create_dir_all(&directory)?;
        let target = directory.join("ambiguous.ogg");
        fs::copy(&source, &target)?;
        scan_audio_file(
            &library.fsio,
            &library.main_db,
            &fixture.global_params.node_id,
            media_root,
            &target,
            &library.library_sync.record_store(),
        )
        .await?;
    }
    let unique_path = media_root.join("unique.ogg");
    fs::copy(&source, &unique_path)?;
    scan_audio_file(
        &library.fsio,
        &library.main_db,
        &fixture.global_params.node_id,
        media_root,
        &unique_path,
        &library.library_sync.record_store(),
    )
    .await?;

    let ambiguous_files = media_files::Entity::find()
        .filter(media_files::Column::FileName.eq("ambiguous.ogg"))
        .order_by_asc(media_files::Column::Id)
        .all(&*library.main_db)
        .await?;
    assert_eq!(ambiguous_files.len(), 2);
    let selected = ambiguous_files.last().context("ambiguous files")?;
    let unique = media_files::Entity::find()
        .filter(media_files::Column::FileName.eq("unique.ogg"))
        .one(&*library.main_db)
        .await?
        .context("unique file")?;

    let ambiguity_only_path = node_dir.path().join("ambiguity-only.m3u8");
    fs::write(
        &ambiguity_only_path,
        "unique.ogg\nambiguous.ogg\nunique.ogg\nambiguous.ogg\n",
    )?;
    let ambiguity_only_request = CreateM3u8PlaylistRequest {
        call_id: None,
        name: "ambiguity only".into(),
        group: "tests".into(),
        path: ambiguity_only_path.to_string_lossy().into_owned(),
    };
    let ambiguity_only = ambiguity_only_request
        .handle(
            (Some(library.clone()), Arc::new(Quiet)),
            None,
            &ambiguity_only_request,
        )
        .await?
        .context("ambiguity-only import response")?;
    // The response is the API boundary; asserting its serialized shape keeps
    // this test failing when the diagnostic is dropped from the response.
    let ambiguity_only_payload = serde_json::to_value(&ambiguity_only)?;
    assert_eq!(ambiguity_only_payload["imported_count"], json!(4));
    assert_eq!(ambiguity_only_payload["not_found_paths"], json!([]));
    assert_eq!(
        ambiguity_only_payload["ambiguous_paths"],
        json!(["ambiguous.ogg", "ambiguous.ogg"])
    );

    let playlist = playlists::Entity::find_by_id(
        ambiguity_only
            .playlist
            .as_ref()
            .context("imported playlist")?
            .id,
    )
    .one(&*library.main_db)
    .await?
    .context("stored playlist")?;
    let occurrences = media_file_playlists::Entity::find()
        .filter(media_file_playlists::Column::PlaylistUuid.eq(&playlist.source_uuid))
        .order_by_asc(media_file_playlists::Column::Position)
        .all(&*library.main_db)
        .await?;
    let track_uuids = occurrences
        .iter()
        .map(|occurrence| occurrence.track_uuid.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        track_uuids,
        vec![
            unique.source_uuid.as_str(),
            selected.source_uuid.as_str(),
            unique.source_uuid.as_str(),
            selected.source_uuid.as_str(),
        ]
    );

    let mixed_path = node_dir.path().join("mixed.m3u8");
    fs::write(&mixed_path, "ambiguous.ogg\nmissing.ogg\n")?;
    let mixed_request = CreateM3u8PlaylistRequest {
        call_id: None,
        name: "mixed".into(),
        group: "tests".into(),
        path: mixed_path.to_string_lossy().into_owned(),
    };
    let mixed = mixed_request
        .handle((Some(library.clone()), Arc::new(Quiet)), None, &mixed_request)
        .await?
        .context("mixed import response")?;
    let mixed_payload = serde_json::to_value(&mixed)?;
    assert_eq!(mixed_payload["imported_count"], json!(1));
    assert_eq!(mixed_payload["not_found_paths"], json!(["missing.ogg"]));
    assert_eq!(mixed_payload["ambiguous_paths"], json!(["ambiguous.ogg"]));

    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_independent_file_lyrics_are_refused() -> Result<()> {
    use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants, SyncLevel};
    use crate::backends::remote::connect_remote;
    use crate::messages::{GetLyricByTrackIdRequest, RemoteRequestFailed};

    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let controller = fixture(controller_dir.path()).await?;
    let library_id = host.global_params.library_id.context("host library id")?;
    let addr = paired_full(&host, &controller).await?;
    let fingerprint = info(&controller).await?.fingerprint;
    host.global_params.node.access_policy.set(
        &fingerprint,
        DeviceGrants {
            control: true,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, ConnectionLevel::View)].into_iter().collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, SyncLevel::Pull)].into_iter().collect(),
            },
            administer: false,
        },
    )?;
    let mut socket = connect_remote(
        "127.0.0.1",
        addr.port(),
        &controller.global_params.config_path,
        controller.global_params.cert_validator.read().await.clone(),
        &controller.global_params.node.sync_tokens,
    )
    .await?;

    let outside_dir = tempfile::tempdir()?;
    let secret = "REMOTE_LYRIC_SECRET_MARKER";
    let outside_audio = outside_dir.path().join("private-audio.ogg");
    fs::write(&outside_audio, secret)?;
    let request = GetLyricByTrackIdRequest {
        call_id: None,
        item: Some(PlayingItemRef::IndependentFile {
            path: outside_audio.to_string_lossy().into_owned(),
        }),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "GetLyricByTrackIdRequest",
        &rinf::serialize(&request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&response_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::InvalidInput);
    assert!(!failure.detail.contains(secret));
    assert!(!String::from_utf8_lossy(&response_payload).contains(secret));

    socket.close(None).await?;
    host.manager.stop().await?;
    host.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn typed_renderer_call_rejects_wrong_reply_name() -> Result<()> {
    use crate::backends::remote::{
        RemoteStream, RendererLink, RendererLinkUse, decode_message, encode_message,
    };
    use crate::messages::{LoadResolvedItemsRequest, PlaylistOperateMode};
    use futures::SinkExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let fake_renderer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = accept_async(stream).await?;
        let message = socket
            .next()
            .await
            .context("typed request was not received")??;
        let Message::Binary(payload) = message else {
            anyhow::bail!("expected a binary request frame")
        };
        let (_, _, request_id) = decode_message(&payload).context("invalid request frame")?;
        socket
            .send(Message::Binary(
                encode_message("SessionSnapshotResponse", &[], Some(request_id)).into(),
            ))
            .await?;
        // Stay open until the link closes: dropping the socket now makes Windows reset the
        // connection, which can discard the reply before the link reads it.
        while let Some(Ok(_)) = socket.next().await {}
        Ok::<_, anyhow::Error>(())
    });
    let (stream, _) = connect_async(format!("ws://{address}")).await?;
    let stream: RemoteStream = stream;
    let link = RendererLink::new(
        stream,
        fixture.global_params.node.clone(),
        "fake-renderer".to_owned(),
        "Fake renderer".to_owned(),
        protocol_version::PROTOCOL_VERSION,
        RendererLinkUse::Cast,
    )
    .await;
    let request = LoadResolvedItemsRequest {
        call_id: None,
        items: Vec::new(),
        start_index: None,
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: LoadPlay::Auto,
    };
    let error = match timeout(TEST_DEADLINE, link.call(&request))
        .await
        .context("typed renderer call timed out")?
    {
        Ok(_) => anyhow::bail!("typed call accepted a response with the wrong name"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::UnexpectedResponse { expected, actual })
                if expected == "LoadResolvedItemsResponse" && actual == "SessionSnapshotResponse"
        ),
        "unexpected typed call error: {error:#}"
    );
    link.close();
    drop(link);
    fake_renderer.await??;
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn rpc_target_local_session_changes_local_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let local = fixture(dir.path()).await?;
    let request = VolumeRequest {
        call_id: None,
        volume: 0.37,
    };
    assert!(local.global_params.node.renderer_link()?.is_none());
    let response = dispatch_gui_request(&local.global_params, &request)
        .await?
        .context("native volume response")?;
    assert_eq!(response.volume, 0.37);
    wait_player(&local, |status| status.volume == 0.37).await?;
    local.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn rpc_target_session_and_library_catalogue_matches_registered_peer_handlers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let peer = fixture(dir.path()).await?;
    peer.global_params
        .node
        .close_ui(&peer.global_params.main_token)
        .await;
    for (name, slot) in RequestSlots::rune().scoped_slots() {
        // Session-only requests authorize without a UI; catalogue requests still need one.
        // An untrusted identity must never execute commands, including zero-argument ones.
        let session = Session {
            public_key: "untrusted key".into(),
            fingerprint: "untrusted peer".into(),
            base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
        };
        let dispatched =
            dispatch_remote(name, &[255], Some(session), None, &peer.global_params.node).await;
        match slot {
            Slot::Forward => {
                let (_, result) =
                    dispatched.with_context(|| format!("{name} has no peer handler"))?;
                let error = result.unwrap_err();
                if request_scope(name) == Some(RequestScope::Session)
                    && request_scope(name) == Some(RequestScope::Session)
                {
                    assert_eq!(
                        classify(&error),
                        RequestErrorKind::PermissionDenied,
                        "{name}"
                    );
                } else {
                    assert_eq!(classify(&error), RequestErrorKind::Unavailable, "{name}");
                }
            }
            Slot::Unsupported { reason } => {
                assert_eq!(reason, UnsupportedReason::OwnerOnly);
                assert!(
                    dispatched.is_none(),
                    "{name} no longer needs its blacklist entry"
                );
            }
            _ => panic!("Rune scoped slot {name} is neither forwarded nor blacklisted"),
        }
    }
    peer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn rpc_target_blacklisted_request_returns_typed_failure_without_sending() -> Result<()> {
    // Test that a request unsupported by the real implementation fails with the expected
    // typed error. We use a Subsonic library whose account has no jukebox role, so player
    // requests like PauseRequest are unsupported with AccountRoleMissing.
    let dir = tempfile::tempdir()?;
    let fixture = super::step("create subsonic fixture", fixture(dir.path())).await?;
    let (endpoint, _, _, _, _, server) = super::step(
        "start subsonic mock",
        subsonic_hub_mock(false),
    )
    .await;
    let subsonic_library_response = super::step(
        "register subsonic library",
        register_subsonic_library_on_node(
            &RegisterSubsonicLibraryRequest {
                endpoint,
                username: "demo".to_owned(),
                password: "demo".to_owned(),
                alias: "Subsonic library".to_owned(),
                mode: Some(SubsonicOpenModeMessage::PlayOnThisDevice),
                call_id: None,
            },
            &fixture.global_params.node,
        ),
    )
    .await
    .context("register subsonic library")?;
    let library_id = subsonic_library_response
        .record
        .context("subsonic library registration should return a record")?
        .record_id
        .parse()
        .context("record_id should be a valid UUID")?;
    let service_binding = super::step(
        "get subsonic service binding",
        service_binding_for_library(&fixture.global_params.node, library_id),
    )
    .await
    .context("get subsonic service binding")?;
    let subsonic_implementation = Implementation::Service(Arc::clone(&service_binding));

    // Subsonic without jukebox role marks player requests as AccountRoleMissing
    assert_eq!(
        RequestCapabilities::from_implementation(&subsonic_implementation).pause_request,
        RequestCapability::Disabled {
            reason: CapabilityReason::AccountRoleMissing,
        }
    );

    // PauseRequest is unsupported and fails with typed error
    let error = super::step(
        "reject unsupported PauseRequest",
        subsonic_implementation.run(&fixture.global_params, &PauseRequest { call_id: None }),
    )
    .await
    .err()
    .context("PauseRequest unexpectedly succeeded")?;
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            request: "PauseRequest",
            reason: UnsupportedReason::AccountRoleMissing,
        })
    ));
    assert_eq!(classify(&error), RequestErrorKind::Unavailable);

    // Verify that dispatch_gui_request still answers for a successful request after the unsupported error
    let result = dispatch_gui_request(&fixture.global_params, &ListLocalLibrariesRequest { call_id: None })
        .await;
    assert!(result.is_ok());

    server.abort();
    super::step(
        "shutdown fixture",
        fixture.global_params.node.shutdown(),
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_mix_calls_use_the_queue_the_caller_sent() -> Result<()> {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = super::fixture_with_reconnect_config(
        first.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
        reconnect_config(Duration::from_millis(400)),
    )
    .await?;
    let b = fixture(second.path()).await?;
    b.global_params
        .node
        .set_open_local_catalog(
            None,
            b.global_params.library_id.context("library node id")?,
        )
        .await?;
    let address = super::paired_full(&b, &a).await?;
    let fingerprint = super::info(&b).await?.fingerprint;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;
    let remote =
        super::open_remote_control_record(a.global_params.node.clone(), &fingerprint).await?;

    // B owns the library and its own player plays `playing`. The caller, which browses B's
    // library through the remote handle, controls another player whose queue holds `queued`.
    let playing = session_audio(&b, "rr-exclusion-playing.ogg").await?;
    let queued = session_audio(&b, "rr-exclusion-queued.ogg").await?;
    queue_session(&b, &[playing]).await?;
    let queued_ref: PlayingItemRef = library_track_for_id(&b, queued).await?.into();
    let playing_ref: PlayingItemRef = library_track_for_id(&b, playing).await?.into();
    let query = [playing, queued]
        .iter()
        .map(|id| MixQuery {
            operator: "lib::track".into(),
            parameter: id.to_string(),
        })
        .collect::<Vec<_>>();
    let resolve = |items| ResolveMixQueryRequest {
        call_id: None,
        queries: query.clone(),
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: true,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems { items },
    };

    let resolved = dispatch_gui_request(&remote, &resolve(vec![queued_ref.clone()]))
        .await?
        .context("resolved mix excluding the caller's queue")?;
    assert_eq!(resolved.playing_items, vec![playing_ref.clone()]);
    let nothing_excluded = dispatch_gui_request(&remote, &resolve(Vec::new()))
        .await?
        .context("resolved mix excluding nothing")?;
    assert_eq!(nothing_excluded.playing_items.len(), 2);

    // `lib::queue` selects the caller's queue, which is not B's own player queue.
    let PlayingItemRef::LibraryTrack { track: playing_track, .. } = &playing_ref else {
        anyhow::bail!("expected a library track");
    };
    let PlayingItemRef::LibraryTrack { track: queued_track, .. } = &queued_ref else {
        anyhow::bail!("expected a library track");
    };
    let paged = |items| MixQueryRequest {
        call_id: None,
        queries: vec![
            MixQuery {
                operator: "lib::track".into(),
                parameter: playing.to_string(),
            },
            MixQuery {
                operator: "lib::queue".into(),
                parameter: "true".into(),
            },
        ],
        cursor: 0,
        page_size: 30,
        bake_cover_arts: false,
        queue: CallerQueue { items },
        random_seed: None,
    };
    let with_queue = dispatch_gui_request(&remote, &paged(vec![queued_ref.clone()]))
        .await?
        .context("mix page with the caller's queue")?;
    let mut tracks = with_queue
        .files
        .iter()
        .map(|file| file.track.clone())
        .collect::<Vec<_>>();
    tracks.sort();
    let mut both = vec![playing_track.clone(), queued_track.clone()];
    both.sort();
    assert_eq!(tracks, both);
    let without_queue = dispatch_gui_request(&remote, &paged(Vec::new()))
        .await?
        .context("mix page with an empty caller queue")?;
    assert_eq!(
        without_queue
            .files
            .iter()
            .map(|file| file.track.clone())
            .collect::<Vec<_>>(),
        vec![playing_track.clone()]
    );

    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_library_routes_gui_edits_events_and_lifecycle_between_real_nodes() -> Result<()> {
    use crate::backends::remote::update_visualizer_feed;
    use crate::messages::{
        AnalysisOutcome, LibraryTaskState, LibraryTaskStateChanged,
        PlaylistItem, PlayingItemRef, ScanTaskPhase,
        RealtimeFFT, VisualizerFeedChart, VisualizerFeedCovers,
    };
    use crate::utils::broadcastable::REMOTE_EVENT_DECLARATIONS;

    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = super::fixture_with_reconnect_config(
        first.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
        reconnect_config(Duration::from_millis(400)),
    )
    .await?;
    let b = fixture(second.path()).await?;
    b.global_params.node.set_open_local_catalog(
        None,
        b.global_params.library_id.context("renderer library id")?,
    ).await?;
    let address = super::paired_full(&b, &a).await?;
    let fingerprint = super::info(&b).await?.fingerprint;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;
    async fn create(params: &GlobalParams, name: &str) -> Result<()> {
        dispatch_gui_request(
            params,
            &CreatePlaylistRequest {
                call_id: None,
                name: name.into(),
                group: "RR".into(),
            },
        )
        .await?;
        Ok(())
    }
    async fn names(params: &GlobalParams) -> Result<Vec<String>> {
        Ok(
            dispatch_gui_request(params, &FetchAllPlaylistsRequest { call_id: None })
                .await?
                .context("playlists")?
                .playlists
                .into_iter()
                .map(|playlist| playlist.name)
                .collect(),
        )
    }
    create(&a.global_params, "only on A").await?;
    create(&b.global_params, "only on B").await?;
    let a_track = session_audio(&a, "rr-local.ogg").await?;
    queue_session(&a, &[a_track]).await?;
    let local_queue = a.global_params.player.lock().await.get_playlist();
    let b_track = session_audio(&b, "rr-remote.ogg").await?;
    let next_track = session_audio(&b, "rr-remote-next.ogg").await?;
    let wall_track = session_audio(&b, "rr-wall.ogg").await?;
    let cover_bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo_cover.webp"))?;
    attach_test_cover(&b, b_track, &cover_bytes).await?;
    attach_test_cover(&b, next_track, &cover_bytes).await?;
    attach_test_cover(&b, wall_track, &cover_bytes).await?;
    let remote_session_id = b.global_params.node.session()?.session_id().await;
    let remote =
        super::open_remote_control_record(a.global_params.node.clone(), &fingerprint).await?;
    assert!(Arc::ptr_eq(&remote.player, &a.global_params.player));
    assert!(Arc::ptr_eq(&remote.scrobbler, &a.global_params.scrobbler));
    assert!(remote.library.is_none());

    signals.clear();
    b.global_params.broadcaster.broadcast(&PlaybackStatus {
        session_id: "declared-session".into(),
        state: PlaybackState::Paused,
        progress_seconds: 0.0,
        progress_percentage: 0.0,
        artist: None,
        album: None,
        title: None,
        duration: 0.0,
        index: None,
        item: None,
        item_request: None,
        playback_mode: WirePlaybackMode::Sequential,
        ready: true,
        cover_art_path: None,
        lib_path: String::new(),
    });
    b.global_params.broadcaster.broadcast(&PlaylistUpdate {
        items: Vec::<PlaylistItem>::new(),
        session_id: "declared-session".into(),
        revision: 1,
    });
    b.global_params
        .broadcaster
        .broadcast(&RealtimeFFT { value: vec![0.25] });
    b.global_params
        .broadcaster
        .broadcast(&LibraryContentChanged {
            path: "declared-library".into(),
        });
    b.global_params.broadcaster.broadcast(&LibraryTaskStateChanged {
        state: LibraryTaskState::AnalysisRunning {
            record_id: "declared-library".into(),
            run_id: "declared-run".into(),
            progress: 1,
            total: 2,
        },
    });
    b.global_params.broadcaster.broadcast(&LibraryTaskStateChanged {
        state: LibraryTaskState::AnalysisEnded {
            record_id: "declared-library".into(),
            run_id: "declared-run".into(),
            progress: 2,
            total: 2,
            outcome: AnalysisOutcome::Finished,
        },
    });
    b.global_params
        .broadcaster
        .broadcast(&capability_snapshot(&b.global_params.node)?);
    wait_until("declared remote events", || async {
        let decoded_all = {
            let records = signals.0.lock().unwrap();
            REMOTE_EVENT_DECLARATIONS
                .iter()
                .filter(|declaration| {
                    !matches!(
                        declaration.name,
                        "VisualizerFeedChart" | "VisualizerFeedCovers"
                    )
                })
                .all(|declaration| {
                    let expected = if declaration.name == "LibraryTaskStateChanged" {
                        2
                    } else {
                        1
                    };
                    records
                        .iter()
                        .filter(|(name, _)| name == declaration.name)
                        .count()
                        >= expected
                        && records.iter().any(|(name, payload)| {
                            name == declaration.name && (declaration.decode)(payload).is_ok()
                        })
                })
        };
        Ok(decoded_all.then_some(()))
    })
    .await
    .context("declared events were not decoded and re-broadcast")?;
    let task_states = signals.decoded::<LibraryTaskStateChanged>("LibraryTaskStateChanged");
    assert!(matches!(task_states.as_slice(), [
        LibraryTaskStateChanged { state: LibraryTaskState::AnalysisRunning { run_id, progress: 1, .. } },
        LibraryTaskStateChanged { state: LibraryTaskState::AnalysisEnded { run_id: ended_run, outcome: AnalysisOutcome::Finished, .. } },
    ] if run_id == "declared-run" && ended_run == run_id));
    assert!(
        signals
            .decoded::<PlaylistUpdate>("PlaylistUpdate")
            .iter()
            .any(|update| update.session_id == "declared-session")
    );
    let capabilities = capability_snapshot(&a.global_params.node)?;
    assert_eq!(
        capabilities.fetch_all_playlists_request,
        RequestCapability::Available
    );
    assert_eq!(
        capabilities.close_library_request,
        RequestCapability::Available
    );
    assert_eq!(
        a.global_params
            .node
            .request_slot::<FetchAllPlaylistsRequest>()?,
        Slot::Forward
    );
    assert_eq!(
        a.global_params.node.request_slot::<CloseLibraryRequest>()?,
        Slot::Native
    );
    let mut browse = start::<FetchAllPlaylistsRequest>(&remote);
    browse.send(FetchAllPlaylistsRequest { call_id: None });
    let response: FetchAllPlaylistsResponse = browse.receive("FetchAllPlaylistsResponse").await;
    assert_eq!(
        response
            .playlists
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["only on B"]
    );
    browse.stop().await;
    let mut missing = start::<GetPlaylistByIdRequest>(&remote);
    missing.send(GetPlaylistByIdRequest {
        call_id: None,
        playlist_id: -1,
    });
    let failure: RequestFailedResponse = missing.receive("RequestFailedResponse").await;
    assert_eq!(failure.kind, RequestErrorKind::NotFound);
    missing.stop().await;

    let local_volume = a
        .global_params
        .node
        .session()?
        .playback_snapshot()
        .await
        .0
        .volume;
    dispatch_gui_request(
        &remote,
        &VolumeRequest {
            call_id: None,
            volume: 0.61,
        },
    )
    .await?;
    wait_player(&b, |status| status.volume == 0.61).await?;
    assert_eq!(
        a.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        local_volume
    );
    let local = dispatch_gui_request(&remote, &ListLocalLibrariesRequest { call_id: None })
        .await?
        .context("local libraries")?;
    assert!(local.libraries.iter().any(|library| {
        Some(library.library_id.as_str())
            == a.global_params
                .library_id
                .map(|id| id.to_string())
                .as_deref()
    }));
    assert!(!local.libraries.iter().any(|library| {
        Some(library.library_id.as_str())
            == b.global_params
                .library_id
                .map(|id| id.to_string())
                .as_deref()
    }));

    signals.0.lock().unwrap().clear();
    create(&remote, "edited through A").await?;
    assert!(
        names(&b.global_params)
            .await?
            .contains(&"edited through A".into())
    );
    // An explicitly addressed request still uses A's library, even while its GUI is remote.
    let (_, bytes) = dispatch_remote(
        "FetchAllPlaylistsRequest",
        &rinf::serialize(&FetchAllPlaylistsRequest { call_id: None })?,
        None,
        a.global_params.library_id,
        &a.global_params.node,
    )
    .await
    .context("addressed request")?;
    let own: FetchAllPlaylistsResponse = rinf::deserialize(&bytes?)?;
    assert_eq!(
        own.playlists
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["only on A"]
    );
    let resolved: ResolveMixQueryResponse = dispatch_gui_request(
        &remote,
        &ResolveMixQueryRequest {
            call_id: None,
            queries: vec![MixQuery {
                operator: "lib::track".into(),
                parameter: b_track.to_string(),
            }],
            hint_position: -1,
            initial_playback_item: None,
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
        },
    )
    .await?
    .context("resolved playback items")?;
    dispatch_gui_request(
        &remote,
        &LoadResolvedItemsRequest {
            position_seconds: None,
            play: LoadPlay::Auto,

            call_id: None,
            items: resolved.playing_items,
            start_index: resolved.start_index,
            mode: PlaylistOperateMode::Replace,
            expected_revision: None,
            playback_mode: Some(WirePlaybackMode::Sequential),
        },
    )
    .await?;
    wait_player(&b, |status| status.state == PlayerPlaybackState::Playing).await?;
    let next_playing_item = PlayingItem::from(library_track_for_id(&b, next_track).await?);
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(next_playing_item, PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    wait_player(&b, |status| status.playlist.len() == 2).await?;
    let feed_item: PlayingItemRef = library_track_for_id(&b, b_track).await?.into();
    wait_until("renderer chart and cover bytes", || async {
        let chart = signals
            .decoded::<VisualizerFeedChart>("VisualizerFeedChart")
            .into_iter()
            .find(|chart| {
                chart.item == feed_item && Chart::decode(&chart.chart).is_ok()
            });
        let covers = signals
            .decoded::<VisualizerFeedCovers>("VisualizerFeedCovers")
            .into_iter()
            .find(|covers| covers.item == feed_item);
        let ready = if let (Some(_), Some(covers)) = (chart, covers)
            && covers.playing.as_deref() == Some(cover_bytes.as_slice())
            && !covers.random.is_empty()
        {
            true
        } else {
            false
        };
        Ok(ready.then_some(()))
    })
    .await
    .context("renderer chart and cover bytes did not reach the controller")?;
    let first_chart = signals
        .decoded::<VisualizerFeedChart>("VisualizerFeedChart")
        .into_iter()
        .find(|chart| chart.item == feed_item && Chart::decode(&chart.chart).is_ok())
        .context("first track chart was not forwarded")?;
    let first_covers = signals
        .decoded::<VisualizerFeedCovers>("VisualizerFeedCovers")
        .into_iter()
        .find(|covers| {
            covers.item == feed_item && covers.playing.as_deref() == Some(cover_bytes.as_slice())
        })
        .context("first track covers were not forwarded")?;
    let first_status = signals
        .decoded::<PlaybackStatus>("PlaybackStatus")
        .into_iter()
        .rev()
        .find(|status| status.item_request.as_ref() == Some(&feed_item))
        .context("renderer playback identity was not forwarded")?;
    // A's own visualizer follows the renderer's feed; which charts and covers it keeps is the
    // visualizer crate's to test.
    let a_visualizer = a.global_params.node.visualizer();
    update_visualizer_feed(a_visualizer, &first_status);
    update_visualizer_feed(a_visualizer, &first_chart);
    update_visualizer_feed(a_visualizer, &first_covers);
    assert!(a_visualizer.is_feed_playback());
    b.global_params.player.lock().await.load(1);
    let next_item: PlayingItemRef = library_track_for_id(&b, next_track).await?.into();
    wait_until("track chart and covers", || async {
        let chart_arrived = signals
            .decoded::<VisualizerFeedChart>("VisualizerFeedChart")
            .iter()
            .any(|chart| {
                chart.item == next_item && Chart::decode(&chart.chart).is_ok()
            });
        let covers_arrived = signals
            .decoded::<VisualizerFeedCovers>("VisualizerFeedCovers")
            .iter()
            .any(|covers| {
                covers.item == next_item
                    && covers.playing.as_deref() == Some(cover_bytes.as_slice())
            });
        Ok((chart_arrived && covers_arrived).then_some(()))
    })
    .await
    .context("track change did not replace the chart and covers")?;
    let next_status = signals
        .decoded::<PlaybackStatus>("PlaybackStatus")
        .into_iter()
        .rev()
        .find(|status| status.item_request.as_ref() == Some(&next_item))
        .context("renderer playback identity was not forwarded after track change")?;
    let next_chart = signals
        .decoded::<VisualizerFeedChart>("VisualizerFeedChart")
        .into_iter()
        .find(|chart| chart.item == next_item)
        .context("next track chart was not forwarded")?;
    let next_covers = signals
        .decoded::<VisualizerFeedCovers>("VisualizerFeedCovers")
        .into_iter()
        .find(|covers| covers.item == next_item)
        .context("next track covers were not forwarded")?;
    update_visualizer_feed(a_visualizer, &next_status);
    update_visualizer_feed(a_visualizer, &next_chart);
    update_visualizer_feed(a_visualizer, &next_covers);
    assert!(a_visualizer.is_feed_playback());
    assert!(
        REMOTE_EVENT_DECLARATIONS
            .iter()
            .filter(|declaration| {
                matches!(
                    declaration.name,
                    "VisualizerFeedChart" | "VisualizerFeedCovers"
                )
            })
            .all(
                |declaration| signals.0.lock().unwrap().iter().any(|(name, payload)| {
                    name == declaration.name && (declaration.decode)(payload).is_ok()
                })
            )
    );
    assert_eq!(
        a.global_params.player.lock().await.get_playlist(),
        local_queue
    );
    b.global_params.broadcaster.broadcast(&LibraryTaskStateChanged {
        state: LibraryTaskState::ScanRunning {
            record_id: "B".into(),
            run_id: "rr-scan".into(),
            progress: 3,
            total: 10,
            phase: ScanTaskPhase::IndexFiles,
        },
    });
    wait_until("RR status, content and scan events", || async {
        let ready = {
            let events = signals.0.lock().unwrap();
            let status = events.iter().any(|(name, body)| {
                name == "PlaybackStatus"
                    && rinf::deserialize::<PlaybackStatus>(body)
                        .is_ok_and(|s| s.session_id == remote_session_id)
            });
            let content = events.iter().any(|(name, body)| {
                name == "LibraryContentChanged"
                    && rinf::deserialize::<LibraryContentChanged>(body)
                        .is_ok_and(|s| s.path == b.global_params.test_library().media_root)
            });
            let scan = events.iter().any(|(name, body)| {
                name == "LibraryTaskStateChanged"
                    && rinf::deserialize::<LibraryTaskStateChanged>(body).is_ok_and(|event| {
                        matches!(event.state, LibraryTaskState::ScanRunning { ref run_id, .. } if run_id == "rr-scan")
                    })
            });
            status && content && scan
        };
        Ok(ready.then_some(()))
    })
    .await
    .context("RR pushed status/content/scan events missing")?;
    a.global_params.node.detach_renderer().await?;
    // Back on its own player, A's visualizer follows the local session again.
    assert!(!a_visualizer.is_feed_playback());

    let mut close = start::<CloseLibraryRequest>(&remote);
    close.send(CloseLibraryRequest { call_id: None });
    let _: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
    close.stop().await;
    assert!(a.global_params.node.renderer_link()?.is_none());
    assert!(matches!(
        a.global_params.node.current_library()?,
        OpenLibrary::Closed | OpenLibrary::Local
    ));
    assert!(a.global_params.node.current_ui().await.is_err());
    let local = Arc::new(
        a.global_params
            .node
            .params_for_library(a.global_params.test_library().clone())?,
    );
    a.global_params.node.attach_ui(&local).await;
    assert_eq!(names(&local).await?, ["only on A"]);
    dispatch_gui_request(
        &local,
        &VolumeRequest {
            call_id: None,
            volume: 0.27,
        },
    )
    .await?;
    wait_player(&a, |status| status.volume == 0.27).await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        0.61
    );

    // Switching directly to a local library also leaves RR.
    let remote =
        super::open_remote_control_record(a.global_params.node.clone(), &fingerprint).await?;
    let replacement = Arc::new(
        a.global_params
            .node
            .params_for_library(a.global_params.test_library().clone())?,
    );
    a.global_params.node.attach_ui(&replacement).await;
    assert!(remote.main_token.is_cancelled());
    assert!(a.global_params.node.renderer_link()?.is_none());
    assert_eq!(names(&replacement).await?, ["only on A"]);

    let remote =
        super::open_remote_control_record(a.global_params.node.clone(), &fingerprint).await?;
    signals.0.lock().unwrap().clear();
    b.global_params.node.manager()?.stop().await?;
    wait_until("RR connection-lost notice", || async {
        let found = signals.0.lock().unwrap().iter().any(|(name, body)| {
            name == "RemoteConnectionLost"
                && rinf::deserialize::<RemoteConnectionLost>(body)
                    .is_ok_and(|loss| loss.kind == RemoteLossKind::ServerStopped)
        });
        Ok(found.then_some(()))
    })
    .await
    .context("RR connection-lost notice missing")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    let failure = dispatch_gui_request(&remote, &FetchAllPlaylistsRequest { call_id: None })
        .await
        .err()
        .context("lost link silently used local library")?;
    assert_eq!(classify(&failure), RequestErrorKind::Unavailable);
    // This is the existing Dart loss handler's final step: close and await its response.
    let mut close = start::<CloseLibraryRequest>(&remote);
    close.send(CloseLibraryRequest { call_id: None });
    let _: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
    close.stop().await;
    assert!(matches!(
        a.global_params.node.current_library()?,
        OpenLibrary::Closed | OpenLibrary::Local
    ));
    assert!(a.global_params.node.current_ui().await.is_err());
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[derive(Clone, Copy)]
struct FakeSubsonicRoles {
    stream: bool,
    jukebox: bool,
    wrong_password: bool,
}

struct FakeSubsonicServer {
    endpoint: String,
    task: task::JoinHandle<()>,
}

async fn fake_subsonic_server(roles: FakeSubsonicRoles) -> Result<FakeSubsonicServer> {
    let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = Router::new()
        .route("/rest/{*endpoint}", get(fake_subsonic_response))
        .with_state(roles);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(FakeSubsonicServer {
        endpoint: format!("http://{address}"),
        task,
    })
}

async fn fake_subsonic_response(
    State(roles): State<FakeSubsonicRoles>,
    AxumPath(endpoint): AxumPath<String>,
    Query(_query): Query<collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let body = match endpoint.trim_end_matches(".view") {
        "ping" => r#"{"subsonic-response":{"status":"ok","version":"1.16.1","openSubsonic":false}}"#.to_owned(),
        "getUser" if roles.wrong_password => r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Rejected"}}}"#.to_owned(),
        "getUser" => format!(
            r#"{{"subsonic-response":{{"status":"ok","user":{{"username":"alice","streamRole":{},"downloadRole":false,"jukeboxRole":{}}}}}}}"#,
            roles.stream, roles.jukebox
        ),
        "jukeboxControl" if roles.jukebox => r#"{"subsonic-response":{"status":"ok","jukeboxPlaylist":{"entry":{"id":"jukebox-track","title":"Queued"},"currentIndex":0,"playing":true,"gain":1.0,"position":7},"jukeboxStatus":{"currentIndex":0,"playing":true,"gain":1.0,"position":7}}}"#.to_owned(),
        "getPlaylists" => r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":{"id":"road","name":"Road"}}}}"#.to_owned(),
        _ => r#"{"subsonic-response":{"status":"ok"}}"#.to_owned(),
    };
    (StatusCode::OK, body)
}

fn subsonic_registration_request(
    endpoint: String,
    password: &str,
    mode: Option<SubsonicOpenModeMessage>,
) -> RegisterSubsonicLibraryRequest {
    RegisterSubsonicLibraryRequest {
        call_id: None,
        endpoint,
        username: "alice".into(),
        password: password.into(),
        alias: "Remote music".into(),
        mode,
    }
}

fn opendal_registration_request(endpoint: String) -> RegisterOpenDalLibraryRequest {
    RegisterOpenDalLibraryRequest {
        call_id: None,
        service: OpenDalLibraryService::WebDav {
            endpoint,
            root: "/dav/library/".into(),
            username: "alice".into(),
            password: "wrong-password".into(),
        },
        alias: "Remote music".into(),
        share_data_choice: ShareDataChoice::ThisDeviceOnly,
    }
}

#[tokio::test]
async fn opendal_registration_reports_auth_rejection_without_storing_anything() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;
    use database::library_registry::FsAdapter;

    let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let app = Router::new().route("/{*path}", any(|| async { StatusCode::UNAUTHORIZED }));
        let _ = axum::serve(listener, app).await;
    });
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let response = timeout(
        TEST_DEADLINE,
        register_opendal_library_on_node(
            &opendal_registration_request(format!("http://{address}")),
            &node,
        ),
    )
    .await
    .context("WebDAV authentication probe hung")?;
    assert_eq!(registration_failure_kind(response)?, RegisterLibraryFailureKind::OpenDalAuthenticationRejected);
    let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
    assert!(registry.libraries.iter().all(|record| {
        !record
            .fs
            .iter()
            .any(|adapter| matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. }))
    }));
    assert!(
        !PathBuf::from(&*node.config_path)
            .join("credentials.enc")
            .exists()
    );
    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn opendal_registration_reports_unreachable_endpoint() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;
    use database::library_registry::FsAdapter;

    let (endpoint, unreachable_server) = unreachable_http_endpoint().await?;
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let response = timeout(
        TEST_DEADLINE,
        register_opendal_library_on_node(&opendal_registration_request(endpoint), &node),
    )
    .await
    .context("unreachable WebDAV probe hung")?;
    assert_eq!(registration_failure_kind(response)?, RegisterLibraryFailureKind::OpenDalUnreachable);
    let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
    assert!(registry.libraries.iter().all(|record| {
        !record
            .fs
            .iter()
            .any(|adapter| matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. }))
    }));
    assert!(
        !PathBuf::from(&*node.config_path)
            .join("credentials.enc")
            .exists()
    );
    unreachable_server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn opendal_registration_reports_non_folder_root() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;
    use database::library_registry::FsAdapter;
    use fsio::test_fixtures::WebDavFixture;

    let fixture_server = WebDavFixture::start(Vec::new()).await?;
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let response = timeout(
        TEST_DEADLINE,
        register_opendal_library_on_node(
            &RegisterOpenDalLibraryRequest {
                call_id: None,
                service: OpenDalLibraryService::WebDav {
                    endpoint: fixture_server.endpoint.clone(),
                    root: "/missing/".into(),
                    username: "user".into(),
                    password: "pass".into(),
                },
                alias: "Missing folder".into(),
                share_data_choice: ShareDataChoice::ThisDeviceOnly,
            },
            &node,
        ),
    )
    .await
    .context("non-folder WebDAV probe hung")?;
    assert_eq!(registration_failure_kind(response)?, RegisterLibraryFailureKind::OpenDalNotFolder);
    let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
    assert!(registry.libraries.iter().all(|record| {
        !record
            .fs
            .iter()
            .any(|adapter| matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. }))
    }));
    assert!(
        !PathBuf::from(&*node.config_path)
            .join("credentials.enc")
            .exists()
    );
    node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn opendal_share_registration_uses_choice_and_existing_share_identity() -> Result<()> {
    let server = WebDavFixture::start(vec![
        ("keep/track.ogg".into(), Vec::new()),
        ("local/track.ogg".into(), Vec::new()),
        ("existing/track.ogg".into(), Vec::new()),
        ("conflict/track.ogg".into(), Vec::new()),
    ])
    .await?;
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let mut kept_library_id = None;

    for (root, choice, in_share) in [
        ("/dav/library/keep/", ShareDataChoice::KeepInShare, true),
        (
            "/dav/library/local/",
            ShareDataChoice::ThisDeviceOnly,
            false,
        ),
    ] {
        let response = register_opendal_library_on_node(
            &RegisterOpenDalLibraryRequest {
                call_id: None,
                service: OpenDalLibraryService::WebDav {
                    endpoint: server.endpoint.clone(),
                    root: root.into(),
                    username: "user".into(),
                    password: "pass".into(),
                },
                alias: "Share test".into(),
                share_data_choice: choice,
            },
            &node,
        )
        .await?;
        anyhow::ensure!(response.outcome == RegisterLibraryOutcome::Registered);
        let record_id = response
            .record
            .context("registered share record")?
            .record_id;
        let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
        let record = registry
            .libraries
            .iter()
            .find(|item| item.record_id.0.to_string() == record_id)
            .context("stored share record")?;
        if in_share {
            kept_library_id = record.library_id;
        }
        anyhow::ensure!(
            matches!(&record.catalog, CatalogPlacement::InShare(_)) == in_share,
            "catalog placement did not match the selected choice"
        );
    }

    let existing_id = Uuid::new_v4();
    let existing_root = "/dav/library/existing/";
    let store = OpenDalObjectStore::new(OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: existing_root.into(),
        username: "user".into(),
        password: "pass".into(),
    })
    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let descriptor = encode_library(&LibraryFile {
        format: FORMAT_VERSION,
        library_id: existing_id,
        model: RuneModel::model_id().into(),
        schema: RuneModel::schema_id().into(),
    })?;
    store
        .put("library.json", &descriptor)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let response = register_opendal_library_on_node(
        &RegisterOpenDalLibraryRequest {
            call_id: None,
            service: OpenDalLibraryService::WebDav {
                endpoint: server.endpoint.clone(),
                root: existing_root.into(),
                username: "user".into(),
                password: "pass".into(),
            },
            alias: "Existing share".into(),
            share_data_choice: ShareDataChoice::ThisDeviceOnly,
        },
        &node,
    )
    .await?;
    anyhow::ensure!(response.outcome == RegisterLibraryOutcome::Registered);
    let record_id = response.record.context("existing share record")?.record_id;
    let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
    let record = registry
        .libraries
        .iter()
        .find(|item| item.record_id.0.to_string() == record_id)
        .context("stored existing share record")?;
    anyhow::ensure!(record.library_id == Some(existing_id));
    anyhow::ensure!(matches!(&record.catalog, CatalogPlacement::InShare(_)));

    let conflicting_id = kept_library_id.context("share library id")?;
    let conflict_root = "/dav/library/conflict/";
    let conflict_store = OpenDalObjectStore::new(OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: conflict_root.into(),
        username: "user".into(),
        password: "pass".into(),
    })
    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let conflicting_descriptor = encode_library(&LibraryFile {
        format: FORMAT_VERSION,
        library_id: conflicting_id,
        model: RuneModel::model_id().into(),
        schema: RuneModel::schema_id().into(),
    })?;
    conflict_store
        .put("library.json", &conflicting_descriptor)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let conflict = register_opendal_library_on_node(
        &RegisterOpenDalLibraryRequest {
            call_id: None,
            service: OpenDalLibraryService::WebDav {
                endpoint: server.endpoint.clone(),
                root: conflict_root.into(),
                username: "user".into(),
                password: "pass".into(),
            },
            alias: "Conflicting share".into(),
            share_data_choice: ShareDataChoice::KeepInShare,
        },
        &node,
    )
    .await;
    assert_eq!(
        registration_failure_kind(conflict)?,
        RegisterLibraryFailureKind::OpenDalShareChanged
    );

    node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn opendal_webdav_registration_open_and_scan_are_read_only_on_current_thread() -> Result<()> {
    assert_opendal_library_scan(OpenDalServiceKind::WebDav).await
}

#[tokio::test]
async fn keep_in_share_rejects_a_read_only_share_before_registration() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;
    use database::library_registry::FsAdapter;
    use fsio::test_fixtures::WebDavFixture;

    let server = WebDavFixture::start_read_only(vec![("track.ogg".into(), Vec::new())]).await?;
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let response = register_opendal_library_on_node(
        &RegisterOpenDalLibraryRequest {
            call_id: None,
            service: OpenDalLibraryService::WebDav {
                endpoint: server.endpoint.clone(),
                root: "/dav/library/".into(),
                username: "user".into(),
                password: "pass".into(),
            },
            alias: "Read-only share".into(),
            share_data_choice: ShareDataChoice::KeepInShare,
        },
        &node,
    )
    .await;

    assert_eq!(registration_failure_kind(response)?, RegisterLibraryFailureKind::OpenDalReadOnly);
    let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
    assert!(registry.libraries.iter().all(|record| {
        !record
            .fs
            .iter()
            .any(|adapter| matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. }))
    }));
    assert!(
        !PathBuf::from(&*node.config_path)
            .join("credentials.enc")
            .exists()
    );
    node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn opendal_s3_registration_open_and_scan_are_read_only_on_current_thread() -> Result<()> {
    assert_opendal_library_scan(OpenDalServiceKind::S3).await
}

async fn assert_opendal_library_scan(service_kind: OpenDalServiceKind) -> Result<()> {
    use database::{
        actions::metadata::{
            MissingHostFilePolicy, ScanOptions, get_metadata_summary_by_files, scan_audio_library,
        },
        entities::media_files,
    };
    use fsio::test_fixtures::{S3Fixture, WebDavFixture};
    use sea_orm::EntityTrait;
    use tokio::runtime::Handle;
    use tokio::task::spawn_blocking;

    let _ = env_logger::builder().is_test(true).try_init();

    timeout(TEST_DEADLINE, async {
        let files = vec![
            (
                "startup_0.ogg".into(),
                include_bytes!("../../../../assets/startup_0.ogg").to_vec(),
            ),
            (
                "startup_1.ogg".into(),
                include_bytes!("../../../../assets/startup_1.ogg").to_vec(),
            ),
        ];
        let (endpoint, webdav, s3_fixture) = match service_kind {
            OpenDalServiceKind::S3 => {
                let fixture = S3Fixture::start_read_only(files).await?;
                (fixture.endpoint.clone(), None, Some(fixture))
            }
            OpenDalServiceKind::WebDav => {
                let fixture = WebDavFixture::start_read_only(files).await?;
                (fixture.endpoint.clone(), Some(fixture), None)
            }
        };
        let dir = tempfile::tempdir()?;
        let fixture = super::fixture(dir.path()).await?;
        let node = fixture.global_params.node.clone();
        let service = match service_kind {
            OpenDalServiceKind::S3 => OpenDalLibraryService::S3 {
                endpoint,
                bucket: "bucket".into(),
                region: "us-east-1".into(),
                root: "/library/".into(),
                access_key_id: "access".into(),
                secret_access_key: "secret".into(),
            },
            OpenDalServiceKind::WebDav => OpenDalLibraryService::WebDav {
                endpoint,
                root: "/dav/library/".into(),
                username: "user".into(),
                password: "pass".into(),
            },
        };
        let request = RegisterOpenDalLibraryRequest {
            call_id: None,
            service,
            alias: "WebDAV test".into(),
            share_data_choice: ShareDataChoice::ThisDeviceOnly,
        };
        let response = register_opendal_library_on_node(&request, &node).await?;
        anyhow::ensure!(
            response.outcome == RegisterLibraryOutcome::Registered,
            "OpenDAL registration failed: {:?}",
            response.outcome
        );
        let record_id = response.record.context("WebDAV record")?.record_id;
        let duplicate = register_opendal_library_on_node(&request, &node).await?;
        anyhow::ensure!(
            duplicate.outcome == RegisterLibraryOutcome::Registered,
            "duplicate OpenDAL registration failed: {:?}",
            duplicate.outcome
        );
        anyhow::ensure!(
            duplicate
                .record
                .context("duplicate WebDAV record")?
                .record_id
                == record_id,
            "duplicate OpenDAL registration created a second record"
        );
        let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
        let record = registry
            .libraries
            .iter()
            .find(|record| record.record_id.0.to_string() == record_id)
            .context("stored record")?;
        let summary = LibraryRecordSummary::from(record);
        anyhow::ensure!(
            matches!(
                summary.kind,
                LibraryRecordKindSummary::OpenDal { service, .. } if service == service_kind
            ),
            "registered OpenDAL record had the wrong service summary: {:?}",
            summary.kind
        );
        let library_id = record.library_id.context("catalog library id")?;
        let library = node.library_pool.acquire_ui(library_id).await?;
        let storage_key = match &record.catalog {
            CatalogPlacement::Redirected(key) => *key,
            placement => anyhow::bail!("OpenDAL catalog was not redirected: {placement:?}"),
        };
        let redirected_db_dir = PathBuf::from(&*node.config_path).join(storage_key.to_string());
        anyhow::ensure!(
            redirected_db_dir.join(".0.db").exists(),
            "redirected catalog database was not created in the config directory"
        );
        let root = PathBuf::from(&library.media_root);
        let fsio = library.fsio.clone();
        let main_db = library.main_db.clone();
        let node_id = node.node_id.to_string();
        let store = library.library_sync.record_store();
        let handle = Handle::current();
        let scanned = spawn_blocking(move || {
            handle.block_on(scan_audio_library(
                &fsio,
                &main_db,
                &node_id,
                &root,
                ScanOptions {
                    cleanup: true,
                    force: true,
                    missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                    cancel_token: None,
                },
                |_| {},
                &store,
            ))
        })
        .await??;
        anyhow::ensure!(scanned == 2, "expected two scanned tracks, got {scanned}");
        let tracks = media_files::Entity::find().all(&*library.main_db).await?;
        let mut remote_copy = tracks[1].clone().into_active_model();
        remote_copy.local_available = Set(false);
        remote_copy.content_hash = Set(Some("a".repeat(64)));
        remote_copy.content_size = Set(Some(1024));
        remote_copy.update(&*library.main_db).await?;
        let mut summaries =
            get_metadata_summary_by_files(&*library.main_db, tracks.clone()).await?;
        summaries.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        anyhow::ensure!(summaries.len() == 2, "expected two tracks");
        anyhow::ensure!(
            summaries[0].directory.is_empty()
                && summaries[0].file_name == "startup_0.ogg"
                && summaries[0].title == "startup_0.ogg",
            "first relative track metadata was incorrect"
        );
        anyhow::ensure!(
            summaries[1].directory.is_empty()
                && summaries[1].file_name == "startup_1.ogg"
                && summaries[1].title == "startup_1.ogg",
            "second relative track metadata was incorrect"
        );
        let queued = files_to_playback_request(
            &library.fsio,
            &library.main_db,
            &library_id.to_string(),
            &library.media_root,
            &[
                MediaFileHandle::from(tracks[0].clone()),
                MediaFileHandle::from(tracks[1].clone()),
            ],
        )
        .await?;
        anyhow::ensure!(
            queued.len() == 2,
            "mixed OpenDAL/replica tracks were omitted from the queue"
        );
        anyhow::ensure!(
            matches!(queued[0].1, PlaybackSource::Local(_)),
            "OpenDAL track did not use the device library source"
        );
        anyhow::ensure!(
            matches!(queued[1].1, PlaybackSource::LibraryStream),
            "replica track did not keep its LibraryStream source beside OpenDAL audio"
        );
        let load = LoadResolvedItemsRequest {
            call_id: None,
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: tracks[0].source_uuid.clone(),
            }],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: None,
            playback_mode: Some(WirePlaybackMode::Sequential),
            position_seconds: Some(0.0),
            play: LoadPlay::Play,
        };
        dispatch_gui_request(&fixture.global_params, &load).await?;
        let played = wait_player(&fixture, |status| {
            status.state == PlayerPlaybackState::Playing
                && matches!(status.item, Some(PlayingItem::LibraryTrack { library_id: id, .. }) if id == library_id)
        })
        .await?;
        anyhow::ensure!(played.item.is_some(), "OpenDAL track did not reach the player");
        node.library_pool.unregister_ui(library_id).await;
        node.shutdown().await?;
        if let Some(webdav) = webdav {
            anyhow::ensure!(
                webdav.write_requests().is_empty(),
                "read-only WebDAV fixture received a write request"
            );
        }
        if let Some(s3) = s3_fixture {
            anyhow::ensure!(
                s3.write_requests().is_empty(),
                "read-only S3 fixture received a write request"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("OpenDAL WebDAV registration/open/scan timed out")??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_registration_resolves_mode_stores_secret_and_deduplicates() -> Result<()> {
    use crate::messages::RegisterLibraryOutcome;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;
    let base =
        subsonic_registration_request(server.endpoint.clone(), "sensitive-test-password", None);
    let needs_mode = register_subsonic_library_on_node(&base, &node).await?;
    assert_eq!(
        needs_mode.outcome,
        RegisterLibraryOutcome::NeedsSubsonicMode {
            allowed: vec![
                SubsonicOpenModeMessage::PlayOnThisDevice,
                SubsonicOpenModeMessage::RemoteControl
            ]
        }
    );
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "sensitive-test-password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    assert_eq!(registration.outcome, RegisterLibraryOutcome::Registered);
    let summary = registration.record.context("registered Subsonic record")?;
    assert_eq!(summary.alias, "Remote music");
    let ui = open_registered_library_record(node.clone(), summary.record_id.clone()).await?;
    assert!(matches!(
        node.current_player()?,
        CurrentPlayer::ServicePlayer(_)
    ));
    assert_eq!(node.current_player()?.visualizer_inputs(), VisualizerInputs::None);
    let snapshot = dispatch_gui_request(
        &fixture.global_params,
        &SessionSnapshotRequest { call_id: None },
    )
    .await?
    .context("Subsonic jukebox session snapshot")?;
    assert_eq!(
        snapshot.items,
        vec![PlayingItemRef::LibraryTrack {
            library_id: summary.record_id.clone(),
            track: "jukebox-track".into(),
        }]
    );
    assert_eq!(snapshot.index, Some(0));
    assert_eq!(snapshot.position_seconds, 7.0);
    assert!(snapshot.playing);
    let registry = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?;
    let stored = registry
        .libraries
        .iter()
        .find(|record| record.record_id.0.to_string() == summary.record_id)
        .context("record persisted")?;
    let ProtocolAdapter::SubsonicTranslate {
        endpoint,
        credential,
        mode,
    } = &stored.protocol
    else {
        anyhow::bail!("record is not Subsonic")
    };
    assert_eq!(endpoint, &Url::parse(&server.endpoint)?.to_string());
    assert_eq!(*mode, SubsonicOpenMode::RemoteControl);
    let restored = node.credentials.get(*credential).await?;
    assert_eq!(restored.username, "alice");
    assert_eq!(restored.secret.as_bytes(), b"sensitive-test-password");
    let registry_bytes = fs::read(PathBuf::from(&*node.config_path).join("libraries.json"))?;
    assert!(!String::from_utf8_lossy(&registry_bytes).contains("sensitive-test-password"));
    let credentials_path = PathBuf::from(&*node.config_path).join("credentials.enc");
    let before_duplicate = fs::read(&credentials_path)?;
    let duplicate = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "sensitive-test-password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    assert_eq!(duplicate.outcome, RegisterLibraryOutcome::Registered);
    assert_eq!(
        duplicate.record.context("duplicate record")?.record_id,
        summary.record_id
    );
    assert_eq!(fs::read(credentials_path)?, before_duplicate);
    assert_eq!(
        registry
            .libraries
            .iter()
            .filter(|record| matches!(record.protocol, ProtocolAdapter::SubsonicTranslate { .. }))
            .count(),
        1
    );
    node.close_ui(&ui.main_token).await;
    assert!(matches!(node.current_player()?, CurrentPlayer::Local));
    assert_eq!(node.current_player()?.visualizer_inputs(), VisualizerInputs::Full);
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

/// A Subsonic library opened before any other library, as when it is restored at
/// startup, answers GUI requests through its own UI until that UI closes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_library_opened_at_startup_answers_gui_requests_until_closed() -> Result<()> {
    use crate::messages::SessionSnapshotResponse;

    for mode in [
        SubsonicOpenModeMessage::RemoteControl,
        SubsonicOpenModeMessage::PlayOnThisDevice,
    ] {
        let dir = tempfile::tempdir()?;
        fs::create_dir_all(dir.path().join(".rune"))?;
        SyncSettings {
            media_server: false,
            ..SyncSettings::default()
        }
        .save(dir.path())?;
        let config = dir.path().to_str().context("config path")?;
        let node = NodeContext::new(
            config,
            config,
            Arc::new(Quiet),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        let server = fake_subsonic_server(FakeSubsonicRoles {
            stream: true,
            jukebox: true,
            wrong_password: false,
        })
        .await?;
        let registration = register_subsonic_library_on_node(
            &subsonic_registration_request(server.endpoint.clone(), "password", Some(mode)),
            &node,
        )
        .await?;
        let record = registration.record.context("registered Subsonic record")?;
        let ui = open_registered_library_record(node.clone(), record.record_id.clone())
            .await
            .with_context(|| format!("{mode:?} Subsonic library opened without a UI"))?;

        let mut playlists = start::<FetchAllPlaylistsRequest>(&ui);
        playlists.send(FetchAllPlaylistsRequest { call_id: Some(1) });
        let listed: FetchAllPlaylistsResponse =
            playlists.receive("FetchAllPlaylistsResponse").await;
        playlists.stop().await;
        assert_eq!(listed.call_id, Some(1));
        assert_eq!(
            listed
                .playlists
                .iter()
                .map(|playlist| playlist.name.as_str())
                .collect::<Vec<_>>(),
            ["Road"]
        );

        let mut snapshot = start::<SessionSnapshotRequest>(&ui);
        snapshot.send(SessionSnapshotRequest { call_id: Some(2) });
        let session: SessionSnapshotResponse = snapshot.receive("SessionSnapshotResponse").await;
        snapshot.stop().await;
        assert_eq!(session.call_id, Some(2));
        // The jukebox answers for its own queue; on this device the local session does.
        let jukebox_queue = vec![PlayingItemRef::LibraryTrack {
            library_id: record.record_id.clone(),
            track: "jukebox-track".into(),
        }];
        match mode {
            SubsonicOpenModeMessage::RemoteControl => assert_eq!(session.items, jukebox_queue),
            SubsonicOpenModeMessage::PlayOnThisDevice => assert!(session.items.is_empty()),
        }

        let mut close = start::<CloseLibraryRequest>(&ui);
        close.send(CloseLibraryRequest { call_id: Some(3) });
        let _: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
        close.stop().await;
        assert!(
            ui.main_token.is_cancelled(),
            "{mode:?} Subsonic GUI listeners kept running after close"
        );
        assert!(!node.service_library_open());
        assert!(node.current_ui().await.is_err());
        assert!(matches!(node.current_player()?, CurrentPlayer::Local));

        server.task.abort();
        node.shutdown().await?;
    }
    Ok(())
}

/// Opening a Subsonic library takes the UI over from the library that was open, and
/// opening that library again takes it back, so only the open library's listeners run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_to_and_from_a_subsonic_library_retires_the_previous_ui() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let local_record = LibraryRegistry::load(Path::new(&*node.config_path))?
        .catalog(fixture.global_params.library_id.context("local library")?)?
        .record_id;
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic record")?;

    let subsonic = open_registered_library_record(node.clone(), record.record_id).await?;
    assert!(
        fixture.global_params.main_token.is_cancelled(),
        "the local library's GUI listeners kept running after Subsonic opened"
    );
    assert!(fixture.global_params.ui_library.lock().await.is_none());

    let local = open_registered_library_record(node.clone(), local_record.0.to_string()).await?;
    assert!(
        subsonic.main_token.is_cancelled(),
        "the Subsonic GUI listeners kept running after the local library opened"
    );
    assert!(!node.service_library_open());
    assert!(matches!(node.current_player()?, CurrentPlayer::Local));
    assert!(!local.main_token.is_cancelled());

    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_session_signals_are_hidden_during_subsonic_remote_control() -> Result<()> {
    use crate::messages::PlayRequest;

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let local_track = session_audio(&fixture, "local-hidden-by-jukebox.ogg").await?;
    queue_session(&fixture, &[local_track]).await?;
    dispatch_gui_request(&fixture.global_params, &PlayRequest { call_id: None }).await?;
    wait_player(&fixture, |status| status.state == PlayerPlaybackState::Playing).await?;
    let local_session_id = node.session()?.session_id().await;
    signals.clear();

    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "session-filter-test-password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic record")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id,
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(10, Duration::from_secs(5)))),
    )
    .await?;
    let subsonic_ui = node.current_ui().await?;
    anyhow::ensure!(
        matches!(node.current_player()?, CurrentPlayer::ServicePlayer(_)),
        "Subsonic remote-control player did not become current"
    );
    // The jukebox poller broadcasts on its first poll and afterwards only on a change, so
    // nothing recorded is dropped: local signals are checked only after this point.
    let current_since = signals.0.lock().unwrap().len();
    let jukebox_session_id = match node.current_player()? {
        CurrentPlayer::ServicePlayer(player) => player.library_id().to_string(),
        _ => anyhow::bail!("Subsonic remote-control player did not become current"),
    };
    let local_status = fixture.global_params.player.lock().await.status_after_pending();
    anyhow::ensure!(
        local_status.await.state == PlayerPlaybackState::Paused,
        "local audio was not paused when the jukebox became current"
    );

    wait_until("the jukebox status and queue to reach the UI", || async {
        let records = signals.0.lock().unwrap();
        let status = records.iter().any(|(name, bytes)| {
            name == "PlaybackStatus"
                && rinf::deserialize::<PlaybackStatus>(bytes)
                    .is_ok_and(|status| status.session_id == jukebox_session_id)
        });
        let playlist = records.iter().any(|(name, bytes)| {
            name == "PlaylistUpdate"
                && rinf::deserialize::<PlaylistUpdate>(bytes)
                    .is_ok_and(|update| update.session_id == jukebox_session_id)
        });
        Ok((status && playlist).then_some(()))
    })
    .await?;
    node.session()?.reemit_to(&node.broadcaster).await?;
    node.broadcaster
        .broadcast(&crate::messages::RealtimeFFT { value: vec![0.25] });
    let (local_status_hidden, local_playlist_hidden, local_fft_hidden) = {
        let records = signals.0.lock().unwrap();
        let records = &records[current_since..];
        (
            !records.iter().any(|(name, bytes)| {
                name == "PlaybackStatus"
                    && rinf::deserialize::<PlaybackStatus>(bytes)
                        .is_ok_and(|status| status.session_id == local_session_id)
            }),
            !records.iter().any(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes)
                        .is_ok_and(|update| update.session_id == local_session_id)
            }),
            !records.iter().any(|(name, _)| name == "RealtimeFFT"),
        )
    };
    anyhow::ensure!(
        local_status_hidden,
        "local PlaybackStatus reached the UI while Subsonic was current"
    );
    anyhow::ensure!(
        local_playlist_hidden,
        "local PlaylistUpdate reached the UI while Subsonic was current"
    );
    anyhow::ensure!(
        local_fft_hidden,
        "local RealtimeFFT reached the UI while Subsonic was current"
    );

    signals.clear();
    node.close_ui(&subsonic_ui.main_token).await;
    let (local_status_restored, local_playlist_restored) = {
        let restored = signals.0.lock().unwrap();
        (
            restored.iter().any(|(name, bytes)| {
                name == "PlaybackStatus"
                    && rinf::deserialize::<PlaybackStatus>(bytes)
                        .is_ok_and(|status| status.session_id == local_session_id)
            }),
            restored.iter().any(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes)
                        .is_ok_and(|update| update.session_id == local_session_id)
            }),
        )
    };
    anyhow::ensure!(
        local_status_restored,
        "closing the Subsonic library did not restore local status"
    );
    anyhow::ensure!(
        local_playlist_restored,
        "closing the Subsonic library did not restore local queue"
    );
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opening_local_library_restores_local_session_signals() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let local_track = session_audio(&fixture, "local-restored-from-jukebox.ogg").await?;
    queue_session(&fixture, &[local_track]).await?;

    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "session-restore-test-password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic record")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id,
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(10, Duration::from_secs(5)))),
    )
    .await?;

    signals.clear();
    node.set_open_local_catalog(
        None,
        fixture.global_params.library_id.context("local library")?,
    )
    .await?;
    let local_session_id = node.session()?.session_id().await;
    let (local_status_restored, local_playlist_restored) = {
        let records = signals.0.lock().unwrap();
        (
            records.iter().any(|(name, bytes)| {
                name == "PlaybackStatus"
                    && rinf::deserialize::<PlaybackStatus>(bytes)
                        .is_ok_and(|status| status.session_id == local_session_id)
            }),
            records.iter().any(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes)
                        .is_ok_and(|update| update.session_id == local_session_id)
            }),
        )
    };
    anyhow::ensure!(
        local_status_restored,
        "opening the local library did not restore the local player status"
    );
    anyhow::ensure!(
        local_playlist_restored,
        "opening the local library did not restore the local player queue"
    );

    node.close_ui(&fixture.global_params.main_token).await;
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_remote_control_pauses_and_restores_local_session_position() -> Result<()> {
    use crate::messages::PlayRequest;

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let local_track = session_audio(&fixture, "local-paused-by-jukebox.ogg").await?;
    queue_session(&fixture, &[local_track]).await?;
    dispatch_gui_request(&fixture.global_params, &PlayRequest { call_id: None }).await?;
    let playing = wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.position >= Duration::from_millis(500)
    })
    .await?;
    let before_switch_position = playing.position;
    let local_session_id = node.session()?.session_id().await;

    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "session-pause-test-password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic record")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id,
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(10, Duration::from_secs(5)))),
    )
    .await?;

    let paused = node.session()?.playback_snapshot().await.0;
    assert_eq!(paused.state, PlayerPlaybackState::Paused);
    assert!(paused.position >= before_switch_position);
    let paused_position = paused.position;

    signals.clear();
    node.set_open_local_catalog(
        None,
        fixture.global_params.library_id.context("local library")?,
    )
    .await?;
    let local_status = signals
        .decoded::<PlaybackStatus>("PlaybackStatus")
        .into_iter()
        .find(|status| status.session_id == local_session_id)
        .context("restored local playback status")?;
    assert_eq!(local_status.state, PlaybackState::Paused);
    let restored_position = Duration::from_secs_f64(f64::from(local_status.progress_seconds));
    assert!(
        restored_position.abs_diff(paused_position) <= Duration::from_millis(100),
        "restored position {restored_position:?} differs from paused position {paused_position:?}"
    );

    node.close_ui(&fixture.global_params.main_token).await;
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

/// Opens a Subsonic library that plays on its server's jukebox, which makes the jukebox the
/// current player.
async fn open_subsonic_jukebox(node: Arc<NodeContext>, endpoint: String) -> Result<()> {
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "player-change-test-password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic record")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id,
        },
        node,
        Arc::new(Mutex::new(ScrobblingManager::new(10, Duration::from_secs(5)))),
    )
    .await?;
    Ok(())
}

async fn wait_for_jukebox_player(node: &NodeContext) -> Result<()> {
    wait_until("the jukebox to become the current player", || async {
        Ok(
            matches!(node.current_player_snapshot()?, CurrentPlayer::ServicePlayer(_))
                .then_some(()),
        )
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_local_load_still_pending_when_a_renderer_becomes_current_never_plays() -> Result<()> {
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let a = fixture(a_dir.path()).await?;
    let b = fixture(b_dir.path()).await?;
    let node = a.global_params.node.clone();
    let track = library_track_for_id(&a, session_audio(&a, "pending-load-before-renderer.ogg").await?)
        .await?;
    let renderer = pair_renderer(&a, &b).await?;

    // Holding the local player keeps the load from reaching it until the renderer is current.
    // Attaching a renderer keeps the open library, so nothing but the player change can stop
    // the load.
    let held_player = a.global_params.player.lock().await;
    let params = a.global_params.clone();
    let mut load = Box::pin(async move {
        dispatch_gui_request(
            &params,
            &LoadResolvedItemsRequest {
                items: vec![PlayingItemRef::LibraryTrack {
                    library_id: track.library_id.to_string(),
                    track: track.file_uuid.to_string(),
                }],
                start_index: Some(0),
                mode: PlaylistOperateMode::Replace,
                expected_revision: None,
                playback_mode: None,
                position_seconds: None,
                play: LoadPlay::Play,
                call_id: None,
            },
        )
        .await
    });
    assert!(
        futures::poll!(&mut load).is_pending(),
        "the load waits for the local player"
    );
    let load = tokio::spawn(load);
    let attach_node = node.clone();
    let attach = tokio::spawn(async move { attach_node.attach_renderer(&renderer).await });
    wait_until("the renderer to become the current player", || async {
        Ok(matches!(node.current_player_snapshot()?, CurrentPlayer::RunePeer { .. }).then_some(()))
    })
    .await?;
    drop(held_player);
    attach.await??;

    let error = load
        .await?
        .err()
        .context("a load sent to the local player before the switch must not complete")?;
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::PlayerChanged)
    ));
    assert_eq!(classify(&error), RequestErrorKind::Conflict);
    let paused = a.global_params.player.lock().await.pause_reporting();
    assert_eq!(paused.await?, PausedPlayback::NotPlaying);

    node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_play_queued_just_before_the_jukebox_becomes_current_is_paused() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let track = session_audio(&fixture, "queued-play-before-switch.ogg").await?;
    queue_session(&fixture, &[track]).await?;
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;

    let held_player = fixture.global_params.player.lock().await;
    let switch = tokio::spawn(open_subsonic_jukebox(node.clone(), server.endpoint.clone()));
    wait_for_jukebox_player(&node).await?;
    // The Play reaches the local player just before the switch does, so the player's status
    // does not show it yet when the switch looks.
    held_player.play();
    drop(held_player);
    switch.await??;

    let paused = fixture.global_params.player.lock().await.pause_reporting();
    assert_eq!(paused.await?, PausedPlayback::NotPlaying);

    node.close_ui(&fixture.global_params.main_token).await;
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_departure_whose_request_is_cancelled_still_pauses_the_local_player() -> Result<()> {
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let a = fixture(a_dir.path()).await?;
    let b = fixture(b_dir.path()).await?;
    let node = a.global_params.node.clone();
    let track = session_audio(&a, "play-before-cancelled-departure.ogg").await?;
    queue_session(&a, &[track]).await?;
    let renderer = pair_renderer(&a, &b).await?;

    // The Play is queued before the player changes, and the player stays held until the
    // attachment has been cancelled, so only a pause sent after the cancellation can stop it.
    let held_player = a.global_params.player.lock().await;
    held_player.play();
    // A local request still running keeps the departure waiting while the attachment that
    // started it is cancelled.
    let tenure = node.local_player_tenure()?;
    let running = tenure.enter().await?;
    let attach_node = node.clone();
    let attach = tokio::spawn(async move { attach_node.attach_renderer(&renderer).await });
    timeout(TEST_DEADLINE, tenure.ended())
        .await
        .context("attaching the renderer did not end the local player's tenure")?;
    attach.abort();
    let cancelled = attach
        .await
        .expect_err("the attachment was cancelled while the departure waited");
    assert!(cancelled.is_cancelled());

    drop(running);
    drop(held_player);
    // The departure installs a new tenure only once its pause is queued, so the pause below
    // follows the departure's.
    wait_until("the departure to install a new tenure", || async {
        Ok((!Arc::ptr_eq(&node.local_player_tenure()?, &tenure)).then_some(()))
    })
    .await?;
    let paused = a.global_params.player.lock().await.pause_reporting();
    assert_eq!(paused.await?, PausedPlayback::NotPlaying);

    node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_during_a_departure_is_refused_until_the_departure_has_paused() -> Result<()> {
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let a = fixture(a_dir.path()).await?;
    let b = fixture(b_dir.path()).await?;
    let node = a.global_params.node.clone();
    let renderer = pair_renderer(&a, &b).await?;

    // A local request still running keeps the departure from pausing.
    let tenure = node.local_player_tenure()?;
    let running = tenure.enter().await?;
    let attach_node = node.clone();
    let attach = tokio::spawn(async move { attach_node.attach_renderer(&renderer).await });
    timeout(TEST_DEADLINE, tenure.ended())
        .await
        .context("attaching the renderer did not end the local player's tenure")?;

    // A request arriving now, for instance after a quick return to the local player, finds the
    // ended tenure. The tenure lets no request in while its end waits for the running one, so
    // the entry is answered only after that request has left, and is then refused.
    let arriving = node.local_player_tenure()?;
    let entry = arriving.enter();
    drop(running);
    let error = timeout(TEST_DEADLINE, entry)
        .await
        .context("a request arriving during the departure was neither admitted nor refused")?
        .expect_err("a request arriving during the departure must be refused");
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::PlayerChanged)
    ));
    attach.await??;

    // The departure has queued its pause, so a new tenure admits requests.
    let fresh = node.local_player_tenure()?;
    drop(fresh.enter().await?);

    node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_library_pauses_and_restores_local_session_position() -> Result<()> {
    use crate::messages::PlayRequest;

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    b.global_params
        .node
        .set_open_local_catalog(
            None,
            b.global_params.library_id.context("renderer library id")?,
        )
        .await?;
    let address = super::paired_full(&b, &a).await?;
    let fingerprint = super::info(&b).await?.fingerprint;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &fingerprint,
            vec![format!("127.0.0.1:{}", address.port())],
        )
        .await?;

    let local_track = session_audio(&a, "local-paused-by-rr.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    dispatch_gui_request(&a.global_params, &PlayRequest { call_id: None }).await?;
    let playing = wait_player(&a, |status| {
        status.state == PlayerPlaybackState::Playing && status.position >= Duration::from_millis(500)
    })
    .await?;
    let before_switch_position = playing.position;

    let remote = super::open_remote_control_record(a.global_params.node.clone(), &fingerprint)
        .await?;
    let paused = a
        .global_params
        .node
        .session()?
        .playback_snapshot()
        .await
        .0;
    assert_eq!(paused.state, PlayerPlaybackState::Paused);
    assert!(paused.position >= before_switch_position);
    let paused_position = paused.position;

    signals.clear();
    a.global_params
        .node
        .close_ui(&remote.main_token)
        .await;
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let local_status = signals
        .decoded::<PlaybackStatus>("PlaybackStatus")
        .into_iter()
        .find(|status| status.session_id == local_session_id)
        .context("restored local playback status after @RR close")?;
    assert_eq!(local_status.state, PlaybackState::Paused);
    let restored_position = Duration::from_secs_f64(f64::from(local_status.progress_seconds));
    assert!(
        restored_position.abs_diff(paused_position) <= Duration::from_millis(100),
        "restored position {restored_position:?} differs from paused position {paused_position:?}"
    );

    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

/// Plays the Subsonic track `track` on this device through the UI's load request.
async fn play_subsonic_track_here(
    fixture: &Fixture,
    library_id: Uuid,
    track: &str,
) -> Result<PlayingItemRef> {
    let item = PlayingItemRef::LibraryTrack {
        library_id: library_id.to_string(),
        track: track.into(),
    };
    let load = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![item.clone()],
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: Some(WirePlaybackMode::Sequential),
        position_seconds: Some(0.0),
        play: LoadPlay::Play,
    };
    dispatch_gui_request(&fixture.global_params, &load).await?;
    Ok(item)
}

/// The playback statuses this device's session reported for `item`, in order.
fn statuses_for(
    signals: &RecordedSignals,
    session_id: &str,
    item: &PlayingItemRef,
) -> Vec<PlaybackStatus> {
    signals
        .decoded::<PlaybackStatus>("PlaybackStatus")
        .into_iter()
        .filter(|status| status.session_id == session_id && status.item_request.as_ref() == Some(item))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_tracks_played_here_report_their_title_and_cover() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock_with_covers(false, true).await;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic library")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id.clone(),
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await?;
    let library_id = match node.current_library()? {
        OpenLibrary::Service(binding) => binding.library_id(),
        _ => anyhow::bail!("registered Subsonic library was not opened"),
    };
    let session_id = node.session()?.session_id().await.to_string();

    let first = play_subsonic_track_here(&fixture, library_id, "song-opaque").await?;
    let playing = wait_until("the first track playing", || async {
        Ok(statuses_for(&signals, &session_id, &first)
            .into_iter()
            .find(|status| status.state == PlaybackState::Playing))
    })
    .await?;
    assert_eq!(playing.title.as_deref(), Some("Song title"));
    assert_eq!(playing.artist.as_deref(), Some("Artist"));
    // The cover is not cached when the first track starts, so it is shown once it arrives.
    let cold = wait_until("the first track's status with its cover", || async {
        Ok(statuses_for(&signals, &session_id, &first)
            .into_iter()
            .find(|status| status.cover_art_path.is_some()))
    })
    .await?;
    assert_eq!(cold.title.as_deref(), Some("Song title"));
    let cover = cold
        .cover_art_path
        .clone()
        .context("first track's cover path")?;
    assert_eq!(fs::read(&cover)?, b"mock-cover-bytes");
    wait_until("the now-playing report of the first track", || async {
        Ok(calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call == "scrobble id=song-opaque submission=false")
            .then_some(()))
    })
    .await?;

    // The next track has the same cover, now cached, so it shows from its first status.
    let second = play_subsonic_track_here(&fixture, library_id, "song-later").await?;
    let warm = wait_until("the second track's first status", || async {
        Ok(statuses_for(&signals, &session_id, &second)
            .into_iter()
            .next())
    })
    .await?;
    assert_eq!(warm.title.as_deref(), Some("Later disc"));
    assert_eq!(warm.cover_art_path.as_deref(), Some(cover.as_str()));

    server.abort();
    node.shutdown().await?;
    Ok(())
}

/// Records every signal, and stops a session's event loop in the broadcast of its next status
/// until released, as a loop that falls behind its player would be.
#[derive(Default)]
struct HeldStatusSignals {
    signals: RecordedSignals,
    hold: StdMutex<Option<StatusHold>>,
}

struct StatusHold {
    session_id: String,
    held: oneshot::Sender<Duration>,
    release: std_mpsc::Receiver<()>,
}

impl HeldStatusSignals {
    /// Holds the event loop of session `session_id` in the broadcast of its next status. Returns
    /// where that status put the track, and what releases the loop.
    async fn hold_next_status(&self, session_id: &str) -> Result<(Duration, std_mpsc::Sender<()>)> {
        let (held, loop_held) = oneshot::channel();
        let (release, released) = std_mpsc::channel();
        *self.hold.lock().unwrap() = Some(StatusHold {
            session_id: session_id.to_owned(),
            held,
            release: released,
        });
        let held_at = timeout(TEST_DEADLINE, loop_held)
            .await
            .context("the event loop sent no status to hold")??;
        Ok((held_at, release))
    }

    /// The statuses of session `session_id` recorded so far, as state and position.
    fn statuses(&self, session_id: &str) -> Vec<(PlaybackState, Duration)> {
        self.signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .into_iter()
            .filter(|status| status.session_id == session_id)
            .map(|status| {
                let position = Duration::from_secs_f64(f64::from(status.progress_seconds));
                (status.state, position)
            })
            .collect()
    }
}

impl Broadcaster for HeldStatusSignals {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        self.signals.broadcast(message);
        let Some(status) = message.as_any().downcast_ref::<PlaybackStatus>() else {
            return;
        };
        let hold = self
            .hold
            .lock()
            .unwrap()
            .take_if(|hold| hold.session_id == status.session_id);
        if let Some(hold) = hold {
            let _ = hold
                .held
                .send(Duration::from_secs_f64(f64::from(status.progress_seconds)));
            // Blocking here hands this worker's other tasks on, which only the multi-thread
            // runtime does.
            let _ = tokio::task::block_in_place(|| hold.release.recv());
        }
    }
}

/// Two paired nodes, the second a host the first can open as an @RR library. The first, which
/// sends its UI to `signals`, plays a track of its own.
async fn local_playing_beside_rr_host(
    signals: Arc<HeldStatusSignals>,
) -> Result<(Vec<TempDir>, Fixture, Fixture, String)> {
    use crate::messages::PlayRequest;

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals,
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    b.global_params
        .node
        .set_open_local_catalog(
            None,
            b.global_params.library_id.context("renderer library id")?,
        )
        .await?;
    let address = super::paired_full(&b, &a).await?;
    let fingerprint = super::info(&b).await?.fingerprint;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;

    let local_track = session_audio(&a, "local-held-behind-rr.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    dispatch_gui_request(&a.global_params, &PlayRequest { call_id: None }).await?;
    wait_player(&a, |status| status.state == PlayerPlaybackState::Playing).await?;
    Ok((vec![a_dir, b_dir], a, b, fingerprint))
}

/// How many statuses of session `session_id` playing past `after` its event loop handled before
/// its first paused one. The session sends its controllers what its loop handled, whether the UI
/// was shown it or not.
async fn playing_handled_before_the_pause(
    sent: &mut broadcast::Receiver<Vec<u8>>,
    session_id: &str,
    after: Duration,
) -> Result<usize> {
    use crate::backends::remote::decode_message;
    use tokio::sync::broadcast::error::RecvError;

    timeout(TEST_DEADLINE, async {
        let mut playing = 0;
        loop {
            let frame = match sent.recv().await {
                Ok(frame) => frame,
                Err(RecvError::Lagged(_)) => continue,
                Err(error) => return Err(Error::new(error)),
            };
            let Some(status) = decode_message(&frame)
                .filter(|(name, _, _)| name == "PlaybackStatus")
                .and_then(|(_, payload, _)| rinf::deserialize::<PlaybackStatus>(&payload).ok())
                .filter(|status| status.session_id == session_id)
            else {
                continue;
            };
            let position = Duration::from_secs_f64(f64::from(status.progress_seconds));
            match status.state {
                PlaybackState::Playing if position > after => playing += 1,
                PlaybackState::Paused => return Ok(playing),
                _ => {}
            }
        }
    })
    .await
    .context("the session's event loop sent no paused status")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_library_close_shows_no_local_status_from_before_the_pause() -> Result<()> {
    let signals = Arc::new(HeldStatusSignals::default());
    let (_dirs, a, b, fingerprint) = local_playing_beside_rr_host(signals.clone()).await?;
    let node = a.global_params.node.clone();
    let local_session_id = node.session()?.session_id().await;
    // What the session sends controllers shows what its event loop handled, shown in the UI
    // or not.
    let mut sent = node.websocket_service.broadcast_tx.subscribe();

    // The session's event loop falls behind its player: it stops in the broadcast of one status
    // while the player reports once more that it plays, and then pauses. The player's status
    // queue holds 16 reports and clears when full, and a paused player reports nothing new, so
    // the queue keeps these reports however long the rest takes.
    let (held_at, release) = signals.hold_next_status(&local_session_id).await?;
    let held_since = Instant::now();
    wait_player(&a, |status| status.position >= held_at + Duration::from_millis(50)).await?;
    a.global_params.player.lock().await.pause();
    wait_player(&a, |status| status.state == PlayerPlaybackState::Paused).await?;
    let played_held = held_since.elapsed();

    let remote = super::open_remote_control_record(node.clone(), &fingerprint).await?;
    let paused = node.session()?.playback_snapshot().await.0;
    assert_eq!(paused.state, PlayerPlaybackState::Paused);
    let paused_position = paused.position;

    signals.signals.clear();
    let closing = tokio::spawn({
        let node = node.clone();
        let token = remote.main_token.clone();
        async move { node.close_ui(&token).await }
    });
    wait_until("this device's player to be current again", || async {
        Ok(matches!(node.current_player_snapshot()?, CurrentPlayer::Local).then_some(()))
    })
    .await?;
    release.send(())?;
    closing.await?;

    let stale = playing_handled_before_the_pause(&mut sent, &local_session_id, held_at).await?;
    assert!(
        stale > 0,
        "inconclusive: the status queue overflowed while the player played for {played_held:?} \
         behind the loop"
    );

    let shown = signals.statuses(&local_session_id);
    assert!(!shown.is_empty(), "the local session was not shown again");
    for (state, position) in &shown {
        assert!(
            *state == PlaybackState::Paused
                && position.abs_diff(paused_position) <= Duration::from_millis(100),
            "the local session showed more than its pause at {paused_position:?}: {shown:?}"
        );
    }

    node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_library_close_lets_a_renderer_attached_meanwhile_stay_current() -> Result<()> {
    let signals = Arc::new(HeldStatusSignals::default());
    let (_dirs, a, b, fingerprint) = local_playing_beside_rr_host(signals.clone()).await?;
    let node = a.global_params.node.clone();
    let session = node.session()?;
    let local_session_id = session.session_id().await;
    let (_, release) = signals.hold_next_status(&local_session_id).await?;
    let remote = super::open_remote_control_record(node.clone(), &fingerprint).await?;

    // The close makes this device's player current again, and its session waits for the held
    // loop to be shown again.
    let closing = tokio::spawn({
        let node = node.clone();
        let token = remote.main_token.clone();
        async move { node.close_ui(&token).await }
    });
    wait_until("this device's player to be current again", || async {
        Ok(matches!(node.current_player_snapshot()?, CurrentPlayer::Local).then_some(()))
    })
    .await?;
    // Meanwhile this device's player plays, as a handoff back to it starts the track before
    // the player is current, and the host is attached as a renderer.
    a.global_params.player.lock().await.play();
    wait_player(&a, |status| status.state == PlayerPlaybackState::Playing).await?;
    node.attach_renderer(&fingerprint).await?;
    assert!(
        !closing.is_finished(),
        "inconclusive: the close stopped waiting for the session's event loop"
    );
    signals.signals.clear();
    release.send(())?;
    closing.await?;

    // The renderer attached last is the current player, as stored and as the node sees it.
    for player in [node.current_player_snapshot()?, node.current_player()?] {
        assert!(
            matches!(
                player,
                CurrentPlayer::RunePeer { link: Some(link), .. }
                    if link.fingerprint() == fingerprint
            ),
            "the attached renderer is not the current player"
        );
    }
    // This device's player, left for the renderer, is paused, and its session stays out of the
    // UI.
    assert_eq!(
        session.playback_snapshot().await.0.state,
        PlayerPlaybackState::Paused
    );
    let shown = signals.statuses(&local_session_id);
    assert!(
        shown.is_empty(),
        "the local session was shown beside the renderer: {shown:?}"
    );

    node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_library_close_shows_the_local_session_when_its_caller_stops_waiting() -> Result<()> {
    let signals = Arc::new(HeldStatusSignals::default());
    let (_dirs, a, b, fingerprint) = local_playing_beside_rr_host(signals.clone()).await?;
    let node = a.global_params.node.clone();
    let local_session_id = node.session()?.session_id().await;
    let (_, release) = signals.hold_next_status(&local_session_id).await?;
    let remote = super::open_remote_control_record(node.clone(), &fingerprint).await?;

    // The close waits for the held loop to show the session again, and its caller stops waiting
    // meanwhile. The close asks for that in the same step as it makes this device's player
    // current, so its caller stops waiting only once it has asked.
    signals.signals.clear();
    let closing = tokio::spawn({
        let node = node.clone();
        let token = remote.main_token.clone();
        async move { node.close_ui(&token).await }
    });
    wait_until("this device's player to be current again", || async {
        Ok(matches!(node.current_player_snapshot()?, CurrentPlayer::Local).then_some(()))
    })
    .await?;
    closing.abort();
    assert!(
        closing.await.is_err_and(|error| error.is_cancelled()),
        "the close finished before its caller stopped waiting"
    );
    release.send(())?;

    // The loop shows the session again all the same, and only once it has handled the statuses
    // from before the pause that it held.
    wait_until("the local session to be shown again", || async {
        Ok(signals
            .statuses(&local_session_id)
            .iter()
            .any(|(state, _)| *state == PlaybackState::Paused)
            .then_some(()))
    })
    .await?;
    let shown = signals.statuses(&local_session_id);
    assert!(
        shown.iter().all(|(state, _)| *state == PlaybackState::Paused),
        "the local session showed statuses from before the pause: {shown:?}"
    );

    node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_player_current_again_hides_statuses_its_player_reports_late() -> Result<()> {
    use crate::messages::PlayRequest;

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(HeldStatusSignals::default());
    // The status the real player caches. Its thread that reports statuses takes this lock for
    // every report.
    let cached = Arc::new(OnceLock::new());
    let f = fixture_with_player(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
        ReconnectConfig::default(),
        RunMode::App,
        Box::new({
            let cached = Arc::clone(&cached);
            move |player: player::Player| -> Arc<Mutex<dyn Playable>> {
                let _ = cached.set(Arc::clone(&player.current_status));
                Arc::new(Mutex::new(player))
            }
        }),
    )
    .await?;
    let track = session_audio(&f, "local-reports-late.ogg").await?;
    queue_session(&f, &[track]).await?;
    dispatch_gui_request(&f.global_params, &PlayRequest { call_id: None }).await?;
    wait_player(&f, |status| status.state == PlayerPlaybackState::Playing).await?;
    let node = f.global_params.node.clone();
    let session_id = node.session()?.session_id().await;
    let mut sent = node.websocket_service.broadcast_tx.subscribe();

    // The player's reports fall behind: its thread that sends them waits for the cached status
    // while the player plays on. Nothing here reads that status until it is let go.
    let cached = Arc::clone(cached.get().context("the player's cached status")?);
    let (blocked, reports_blocked) = oneshot::channel();
    let (unblock, unblocked) = std_mpsc::channel::<()>();
    let blocking = thread::spawn(move || {
        let cached = cached.lock().unwrap();
        let _ = blocked.send(cached.position);
        let _ = unblocked.recv();
    });
    let blocked_at = timeout(TEST_DEADLINE, reports_blocked).await??;
    sleep(Duration::from_millis(250)).await;
    // Another player takes over, and this device's player pauses, which it also reports late.
    let away = CurrentPlayer::RunePeer {
        link: None,
        reconnecting: None,
    };
    node.transition_current_player(CurrentPlayer::Local, away.clone()).await?;

    // This device's player is current again before it has sent what it reported before the
    // pause.
    signals.signals.clear();
    let returning = tokio::spawn({
        let node = node.clone();
        async move { node.transition_current_player(away, CurrentPlayer::Local).await }
    });
    wait_until("this device's player to be current again", || async {
        Ok(matches!(node.current_player_snapshot()?, CurrentPlayer::Local).then_some(()))
    })
    .await?;
    unblock.send(())?;
    blocking
        .join()
        .map_err(|_| anyhow::anyhow!("the thread holding the cached status panicked"))?;
    returning.await??;

    let late = playing_handled_before_the_pause(
        &mut sent,
        &session_id,
        blocked_at + Duration::from_millis(50),
    )
    .await?;
    assert!(late > 0, "inconclusive: the player sent no report late from before the pause");
    let shown = signals.statuses(&session_id);
    assert!(!shown.is_empty(), "the local session was not shown again");
    assert!(
        shown.iter().all(|(state, _)| *state == PlaybackState::Paused),
        "the local session showed statuses from before the pause: {shown:?}"
    );

    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_session_dropping_a_show_request_leaves_the_next_session_visible() -> Result<()> {
    use crate::messages::PlayRequest;
    use crate::session::PlayerSession;
    use std::pin::pin;

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(HeldStatusSignals::default());
    let f = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let track = session_audio(&f, "dropped-show-request.ogg").await?;
    queue_session(&f, &[track]).await?;
    dispatch_gui_request(&f.global_params, &PlayRequest { call_id: None }).await?;
    wait_player(&f, |status| status.state == PlayerPlaybackState::Playing).await?;
    let node = f.global_params.node.clone();
    let session = node.session()?;
    let session_id = session.session_id().await;

    // This device's session falls behind its player, so its event loop cannot take a request to
    // show it again. Another player takes over, and this device's player is current again.
    let (_, release) = signals.hold_next_status(&session_id).await?;
    let away = CurrentPlayer::RunePeer {
        link: None,
        reconnecting: None,
    };
    node.transition_current_player(CurrentPlayer::Local, away.clone()).await?;
    signals.signals.clear();
    // The return's first poll switches the player and queues its request to show the session,
    // with nothing to wait for in between. The held loop keeps the request queued, so the return
    // is left waiting for the answer.
    let mut returning = pin!(node.transition_current_player(away, CurrentPlayer::Local));
    assert!(
        futures::poll!(&mut returning).is_pending(),
        "the return finished without waiting for the held event loop"
    );

    // The session is cancelled with the request still queued. The first step of its shutdown
    // cancels it, and the rest waits for the held loop, which then stops without taking the
    // request.
    let mut shutdown = pin!(session.shutdown());
    assert!(
        futures::poll!(&mut shutdown).is_pending(),
        "the session shut down without waiting for its held event loop"
    );
    release.send(())?;
    shutdown.await?;
    returning.await?;
    let shown = signals.statuses(&session_id);
    assert!(
        shown.is_empty(),
        "inconclusive: the session took the request before it was cancelled: {shown:?}"
    );

    // The next session the node opens plays, and its statuses reach the UI.
    let next = PlayerSession::new(&node, "next", false, PlayerSession::real_player()).await?;
    let next_id = next.session_id().await;
    let item = library_track_for_id(&f, track).await?;
    {
        let player = next.player.lock().await;
        player.add_to_playlist_with_sources(
            vec![(PlayingItem::from(item), PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
        player.load(0);
        player.play();
    }
    wait_until("the next session's statuses to reach the UI", || async {
        Ok(signals
            .statuses(&next_id)
            .iter()
            .any(|(state, _)| *state == PlaybackState::Playing)
            .then_some(()))
    })
    .await?;

    next.shutdown().await?;
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_visualizer_covers_use_the_bounded_server_cover_cache() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();

    let local_id = session_audio(&fixture, "visualizer-local-cover.ogg").await?;
    attach_test_cover(&fixture, local_id, b"local-visualizer-cover").await?;
    let local_item: PlayingItem = library_track_for_id(&fixture, local_id).await?.into();
    let (local_playing, local_wall) = Implementation::Local
        .visualizer_covers(&local_item, &node)
        .await?;
    assert!(
        local_playing.is_some(),
        "local playing cover was not read from the DB"
    );
    assert!(
        !local_wall.is_empty(),
        "local cover wall did not use the DB selection"
    );

    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock_with_covers(false, true).await;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        &node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic library")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id.clone(),
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await?;
    let library_id = match node.current_library()? {
        OpenLibrary::Service(binding) => binding.library_id(),
        _ => anyhow::bail!("registered Subsonic library was not opened"),
    };
    let implementation = playback_implementation_for_item(&node, library_id).await?;
    let item = PlayingItem::LibraryTrack {
        library_id,
        track: "song-opaque".into(),
    };
    let load = LoadResolvedItemsRequest {
        call_id: None,
        items: vec![PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: "song-opaque".into(),
        }],
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: Some(WirePlaybackMode::Sequential),
        position_seconds: Some(0.0),
        play: LoadPlay::Play,
    };
    dispatch_gui_request(&fixture.global_params, &load).await?;
    wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing && status.item.as_ref() == Some(&item)
    })
    .await?;

    let (playing, wall) = implementation.visualizer_covers(&item, &node).await?;
    assert!(playing.is_some(), "playing cover was not returned");
    assert_eq!(
        wall.len(),
        62,
        "wall selection included the playing cover or exceeded its bound"
    );
    let cover_requests = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|action| action.as_str() == "getCoverArt")
        .count();
    assert_eq!(
        cover_requests, 63,
        "covers were not fetched once per cache key"
    );

    let (cached_playing, cached_wall) = implementation.visualizer_covers(&item, &node).await?;
    assert_eq!(cached_playing, playing);
    assert_eq!(cached_wall, wall);
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|action| action.as_str() == "getCoverArt")
            .count(),
        cover_requests,
        "cached covers were fetched again"
    );

    server.abort();
    node.shutdown().await?;
    Ok(())
}

/// Registers a Subsonic library at `endpoint` that plays on this device, and opens it through
/// its record as the UI does. Returns the library id its tracks carry.
async fn open_subsonic_library_here(node: &Arc<NodeContext>, endpoint: String) -> Result<Uuid> {
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        node,
    )
    .await?;
    let record = registration.record.context("registered Subsonic library")?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record.record_id.clone(),
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await?;
    Ok(Uuid::parse_str(&record.record_id)?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_service_library_translates_requests_through_its_binding() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    let library_id = open_subsonic_library_here(&node, endpoint).await?;
    assert!(matches!(
        node.current_library()?,
        OpenLibrary::Service(binding) if binding.library_id() == library_id
    ));
    assert!(matches!(node.current_player()?, CurrentPlayer::Local));

    let ui = node.current_ui().await?;
    let playlists = dispatch_gui_request(&ui, &FetchAllPlaylistsRequest { call_id: None })
        .await?
        .context("the library's playlists")?
        .playlists;
    assert_eq!(
        playlists
            .iter()
            .map(|playlist| playlist.name.as_str())
            .collect::<Vec<_>>(),
        ["Road"]
    );
    assert!(calls.lock().unwrap().iter().any(|call| call == "getPlaylists"));

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn service_track_playing_here_reports_its_metadata() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    let library_id = open_subsonic_library_here(&node, endpoint).await?;
    let session_id = node.session()?.session_id().await.to_string();

    let item = play_subsonic_track_here(&fixture, library_id, "song-opaque").await?;
    let playing = wait_until("the track playing", || async {
        Ok(statuses_for(&signals, &session_id, &item)
            .into_iter()
            .find(|status| status.state == PlaybackState::Playing))
    })
    .await?;
    assert_eq!(playing.title.as_deref(), Some("Song title"));
    assert_eq!(playing.artist.as_deref(), Some("Artist"));

    // The track is short, so it plays to its end: the server hears that it started, and once
    // that it was played through.
    let submitted = "scrobble id=song-opaque submission=true";
    wait_until("the server to hear the track was played through", || async {
        Ok(calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call == submitted)
            .then_some(()))
    })
    .await?;
    let calls = calls.lock().unwrap().clone();
    let started = calls
        .iter()
        .position(|call| call == "scrobble id=song-opaque submission=false")
        .context("the start report")?;
    let played = calls
        .iter()
        .position(|call| call == submitted)
        .context("the play-through report")?;
    assert!(started < played);
    assert_eq!(calls.iter().filter(|call| *call == submitted).count(), 1);

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_service_library_is_not_opened_for_item_metadata() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    let record = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        &node,
    )
    .await?
    .record
    .context("registered Subsonic library")?;
    let item = PlayingItem::LibraryTrack {
        library_id: Uuid::parse_str(&record.record_id)?,
        track: "song-opaque".into(),
    };
    calls.lock().unwrap().clear();

    // Opening the library would sign in to its server. Only an open library answers about its
    // items, so the server hears nothing.
    assert!(
        library_item_metadata(&node, &item, true)
            .await
            .is_err()
    );
    assert!(
        report_item_played_through(&node, &item)
            .await
            .is_err()
    );
    assert!(calls.lock().unwrap().is_empty());

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn log_requests_answer_while_a_service_library_is_open() -> Result<()> {
    use crate::messages::{ClearLogRequest, ListLogRequest, RemoveLogRequest};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    open_subsonic_library_here(&node, endpoint).await?;

    // The library keeps no log on this device, so the log page shows nothing and the
    // requests that change the log find nothing to change.
    let ui = node.current_ui().await?;
    let log = dispatch_gui_request(
        &ui,
        &ListLogRequest {
            cursor: 0,
            page_size: 10,
            call_id: None,
        },
    )
    .await?
    .context("the log page")?;
    assert!(log.result.is_empty());
    let removed = dispatch_gui_request(&ui, &RemoveLogRequest { id: 1, call_id: None })
        .await?
        .context("the removed entry")?;
    assert_eq!(removed.id, 1);
    assert!(
        dispatch_gui_request(&ui, &ClearLogRequest { call_id: None })
            .await?
            .is_some()
    );

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn visualizer_starts_while_a_service_library_is_open() -> Result<()> {
    use crate::messages::{SetVisualizerLogRequest, StartVisualizerRequest};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    open_subsonic_library_here(&node, endpoint).await?;
    let ui = node.current_ui().await?;

    assert!(
        dispatch_gui_request(&ui, &SetVisualizerLogRequest { enabled: false, call_id: None })
            .await?
            .is_some()
    );
    // The test holds the player's audio, so the visualizer finds it in use and reports that
    // instead of starting its renderer, which needs a GPU. The start gets that far only when the
    // open library does not stop it first.
    let tap = ui.player.lock().await.pcm_tap();
    let audio = tap.take_consumer().context("the player's audio")?;
    let started = dispatch_gui_request(
        &ui,
        &StartVisualizerRequest {
            width: 16,
            height: 16,
            fps: 1,
            call_id: None,
        },
    )
    .await;
    tap.restore_consumer(audio);
    assert!(started?.is_some());
    let node_visualizer = Arc::clone(node.node_visualizer().visualizer());
    let reported = node_visualizer.status().last_error.unwrap_or_default();
    assert!(reported.contains("PCM tap"), "unexpected report: {reported}");

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_capabilities_hide_roaming_and_the_playlist_file_but_not_play_on() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    open_subsonic_library_here(&node, endpoint).await?;

    // A Subsonic library has no analysis and no files of its own, so the UI hides what needs
    // them. Another device can still play it.
    let capabilities = capability_snapshot(&node)?;
    assert_ne!(
        capabilities.get_analyze_count_request,
        RequestCapability::Available
    );
    assert_ne!(
        capabilities.fetch_directory_tree_request,
        RequestCapability::Available
    );
    assert_eq!(
        capabilities.cast_resolved_items_request,
        RequestCapability::Available
    );

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_subsonic_library_answers_its_home() -> Result<()> {
    use crate::messages::{
        ComplexQuery, ComplexQueryRequest, LibraryHomeListMode, LibraryHomeQuery,
    };

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    open_subsonic_library_here(&node, endpoint).await?;

    // The library home asks for its summaries because the library offers them.
    assert_eq!(
        capability_snapshot(&node)?.complex_query_request,
        RequestCapability::Available
    );
    let group = |title: &str, query| ComplexQuery {
        title: title.into(),
        query,
    };
    let mode = LibraryHomeListMode::Forward;
    let ui = node.current_ui().await?;
    let home = dispatch_gui_request(
        &ui,
        &ComplexQueryRequest {
            queries: vec![
                group("Artists", LibraryHomeQuery::Artists(mode)),
                group("Playlists", LibraryHomeQuery::Playlists(mode)),
                group("Tracks", LibraryHomeQuery::Tracks(mode)),
            ],
            call_id: None,
        },
    )
    .await?
    .context("the library home")?;
    let names = |index: usize| {
        home.result[index]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(0), ["Artist", "Beta"]);
    assert_eq!(names(1), ["Road"]);
    assert_eq!(names(2), ["Song title"]);
    assert!(calls.lock().unwrap().iter().any(|call| call == "getPlaylists"));

    server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn played_service_track_leaves_local_statistics_alone() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let local_id = session_audio(&fixture, "service-played.ogg").await?;
    let local_item = PlayingItem::from(library_track_for_id(&fixture, local_id).await?);
    let library = fixture.global_params.test_library();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let library_id = open_subsonic_library_here(&node, endpoint).await?;
    let played_through = || async {
        Ok::<_, Error>(
            media_file_stats::Entity::find()
                .all(&*library.main_db)
                .await?
                .iter()
                .map(|stats| stats.played_through)
                .sum::<i32>(),
        )
    };

    // A played track of the service is recorded by nothing on this device.
    record_library_item_played(
        &node,
        &PlayingItem::LibraryTrack {
            library_id,
            track: "song-opaque".into(),
        },
    )
    .await?;
    assert_eq!(played_through().await?, 0);

    // The service's implementation leaves local statistics alone even for a track this device
    // holds, which only the local one counts.
    let binding = node
        .service_binding(library_id)?
        .context("the open service binding")?;
    Implementation::Service(binding)
        .record_played(&local_item, library)
        .await?;
    assert_eq!(played_through().await?, 0);
    Implementation::Local
        .record_played(&local_item, library)
        .await?;
    assert_eq!(played_through().await?, 1);

    server.abort();
    node.shutdown().await?;
    Ok(())
}

/// Registers the Subsonic server at `endpoint` as a library, without opening it.
async fn register_subsonic_here(node: &Arc<NodeContext>, endpoint: String) -> Result<Uuid> {
    let record = register_subsonic_library_on_node(
        &subsonic_registration_request(
            endpoint,
            "test-secret",
            Some(SubsonicOpenModeMessage::PlayOnThisDevice),
        ),
        node,
    )
    .await?
    .record
    .context("registered Subsonic library")?;
    Ok(Uuid::parse_str(&record.record_id)?)
}

/// Opens the registered library `record_id` in the UI.
async fn open_registered_here(node: &Arc<NodeContext>, record_id: Uuid) -> Result<()> {
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id: record_id.to_string(),
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await?;
    Ok(())
}

fn service_item(library_id: Uuid, track: &str) -> PlayingItemRef {
    PlayingItemRef::LibraryTrack {
        library_id: library_id.to_string(),
        track: track.into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn liked_state_goes_to_the_library_that_owns_the_item() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let local_id = session_audio(&fixture, "liked-local.ogg").await?;
    let local_track = PlayingItemRef::from(library_track_for_id(&fixture, local_id).await?);
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = register_subsonic_here(&node, endpoint).await?;
    let service_track = service_item(record_id, "song-opaque");
    let stars = || {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.starts_with("star") || call.starts_with("unstar"))
            .cloned()
            .collect::<Vec<_>>()
    };

    // The local library is open, and the service's track is liked in its own library.
    let liked = dispatch_gui_request(
        &fixture.global_params,
        &SetLikedRequest {
            item: Some(service_track.clone()),
            liked: true,
            call_id: None,
        },
    )
    .await?
    .context("service liked")?;
    assert!(liked.liked);
    assert_eq!(stars(), ["star id=song-opaque"]);
    let state = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            item: Some(service_track.clone()),
            call_id: None,
        },
    )
    .await?
    .context("service liked state")?;
    assert!(state.liked);

    // A local track stays with its local library.
    dispatch_gui_request(
        &fixture.global_params,
        &SetLikedRequest {
            item: Some(local_track.clone()),
            liked: true,
            call_id: None,
        },
    )
    .await?
    .context("local liked")?;
    let state = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            item: Some(local_track.clone()),
            call_id: None,
        },
    )
    .await?
    .context("local liked state")?;
    assert!(state.liked);
    assert_eq!(stars(), ["star id=song-opaque"]);

    // While the service library is open, the local track is still the local library's, and
    // the service's track still the service's.
    open_registered_here(&node, record_id).await?;
    let ui = node.current_ui().await?;
    dispatch_gui_request(
        &ui,
        &SetLikedRequest {
            item: Some(local_track.clone()),
            liked: false,
            call_id: None,
        },
    )
    .await?
    .context("local unliked")?;
    let state = dispatch_gui_request(
        &ui,
        &GetLikedRequest {
            item: Some(local_track),
            call_id: None,
        },
    )
    .await?
    .context("local unliked state")?;
    assert!(!state.liked);
    assert_eq!(stars(), ["star id=song-opaque"]);
    dispatch_gui_request(
        &ui,
        &SetLikedRequest {
            item: Some(service_track),
            liked: false,
            call_id: None,
        },
    )
    .await?
    .context("service unliked")?;
    assert_eq!(stars(), ["star id=song-opaque", "unstar id=song-opaque"]);

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lyrics_of_a_service_track_come_from_its_library_while_a_local_library_is_open()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let local_id = session_audio(&fixture, "lyrics-local.ogg").await?;
    let local_track = PlayingItemRef::from(library_track_for_id(&fixture, local_id).await?);
    let (endpoint, calls, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = register_subsonic_here(&node, endpoint).await?;
    let lyric_requests = || {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| *call == "getLyrics")
            .count()
    };
    let lyrics_of = |params: Arc<GlobalParams>, item: PlayingItemRef| async move {
        dispatch_gui_request(
            &params,
            &GetLyricByTrackIdRequest {
                item: Some(item),
                call_id: None,
            },
        )
        .await?
        .context("lyrics")
    };
    let contents = |lines: &[LyricContentLine]| {
        lines
            .iter()
            .flat_map(|line| line.sections.iter().map(|section| section.content.clone()))
            .collect::<Vec<_>>()
    };

    // The local library is open, and the service's track gets the lyrics its server has.
    let lyrics = lyrics_of(
        fixture.global_params.clone(),
        service_item(record_id, "song-opaque"),
    )
    .await?;
    assert_eq!(contents(&lyrics.lines), ["First line", "Second line"]);
    assert_eq!(lyric_requests(), 1);

    // A local track's lyrics are read from its file, and no server is asked.
    let local = lyrics_of(fixture.global_params.clone(), local_track.clone()).await?;
    assert!(local.lines.is_empty());
    assert_eq!(lyric_requests(), 1);

    // The same holds once the service library is the one open.
    open_registered_here(&node, record_id).await?;
    let ui = node.current_ui().await?;
    let lyrics = lyrics_of(ui.clone(), service_item(record_id, "song-opaque")).await?;
    assert_eq!(contents(&lyrics.lines), ["First line", "Second line"]);
    assert_eq!(lyric_requests(), 2);
    let local = lyrics_of(ui, local_track).await?;
    assert!(local.lines.is_empty());
    assert_eq!(lyric_requests(), 2);

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lyrics_of_the_open_service_librarys_track_need_no_library_records() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = register_subsonic_here(&node, endpoint).await?;
    open_registered_here(&node, record_id).await?;
    let ui = node.current_ui().await?;

    // The library records cannot be read for now, but the open library's binding knows its own
    // items without them.
    let records = Path::new(&*node.config_path).join("libraries.json");
    let kept = fs::read(&records)?;
    fs::write(&records, b"unreadable")?;
    let lyrics = dispatch_gui_request(
        &ui,
        &GetLyricByTrackIdRequest {
            item: Some(service_item(record_id, "song-opaque")),
            call_id: None,
        },
    )
    .await;
    fs::write(&records, kept)?;
    assert!(!lyrics?.context("lyrics")?.lines.is_empty());

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_color_of_a_service_track_comes_from_its_cover_while_a_local_library_is_open()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = register_subsonic_here(&node, endpoint).await?;
    let color_of = |params: Arc<GlobalParams>, track: &'static str| async move {
        dispatch_gui_request(
            &params,
            &GetPrimaryColorByTrackIdRequest {
                item: Some(service_item(record_id, track)),
                call_id: None,
            },
        )
        .await?
        .context("primary color")
        .map(|response| response.primary_color)
    };
    let expected = metadata::cover_art::get_primary_color(&red_bitmap());
    assert!(expected.is_some());

    assert_eq!(
        color_of(fixture.global_params.clone(), "song-color").await?,
        expected
    );
    // A track without a cover has no color.
    assert_eq!(
        color_of(fixture.global_params.clone(), "song-opaque").await?,
        None
    );

    open_registered_here(&node, record_id).await?;
    let ui = node.current_ui().await?;
    assert_eq!(color_of(ui, "song-color").await?, expected);

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_about_an_item_of_a_library_without_its_credential_asks_for_a_sign_in()
-> Result<()> {
    use crate::request_error::request_failure_cause;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = register_subsonic_here(&node, endpoint).await?;
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let ProtocolAdapter::SubsonicTranslate { credential, .. } =
        registry.record(LibraryRecordId(record_id))?.protocol.clone()
    else {
        anyhow::bail!("expected a Subsonic record");
    };
    node.credentials.remove(credential).await?;

    let error = dispatch_gui_request(
        &fixture.global_params,
        &GetLikedRequest {
            item: Some(service_item(record_id, "song-opaque")),
            call_id: None,
        },
    )
    .await
    .err()
    .context("a like asked of a library whose credential is gone")?;
    assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: record_id.to_string(),
        })
    );

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_color_whose_cover_the_service_refuses_for_its_sign_in_asks_for_a_sign_in() -> Result<()>
{
    use crate::request_error::request_failure_cause;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = register_subsonic_here(&node, endpoint).await?;

    // The library opens, but the server rejects its sign-in when the cover's song is asked for.
    let error = dispatch_gui_request(
        &fixture.global_params,
        &GetPrimaryColorByTrackIdRequest {
            item: Some(service_item(record_id, "song-signed-out")),
            call_id: None,
        },
    )
    .await
    .err()
    .context("a color asked of a library whose sign-in the server rejects")?;
    assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: record_id.to_string(),
        })
    );

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_what_a_service_track_supports_before_the_status_that_names_it() -> Result<()>
{
    use crate::messages::{PlayingItemCapabilities, RequestCapability};

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let library_id = open_subsonic_library_here(&node, endpoint).await?;
    let session_id = node.session()?.session_id().await.to_string();
    let item = play_subsonic_track_here(&fixture, library_id, "song-opaque").await?;
    wait_until("the track named by a status", || async {
        Ok((!statuses_for(&signals, &session_id, &item).is_empty()).then_some(()))
    })
    .await?;

    let recorded = signals.0.lock().unwrap().clone();
    let position = |name: &str, matches: &dyn Fn(&[u8]) -> bool| {
        recorded
            .iter()
            .position(|(signal, payload)| signal == name && matches(payload))
    };
    let declared = position("PlayingItemCapabilities", &|payload| {
        rinf::deserialize::<PlayingItemCapabilities>(payload)
            .is_ok_and(|capabilities| capabilities.item.as_ref() == Some(&item))
    })
    .context("the declaration for the track")?;
    let named = position("PlaybackStatus", &|payload| {
        rinf::deserialize::<PlaybackStatus>(payload)
            .is_ok_and(|status| status.item_request.as_ref() == Some(&item))
    })
    .context("the status naming the track")?;
    assert!(declared < named);
    let capabilities = recorded[declared].clone();
    let capabilities = rinf::deserialize::<PlayingItemCapabilities>(&capabilities.1)?;
    // What the Subsonic library declares for its items: all of them are answered.
    assert_eq!(
        [
            capabilities.set_liked_request,
            capabilities.get_liked_request,
            capabilities.get_lyric_by_track_id_request,
            capabilities.get_primary_color_by_track_id_request,
        ],
        [RequestCapability::Available; 4]
    );

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_that_a_service_track_answers_nothing_once_its_record_is_removed()
-> Result<()> {
    use crate::messages::{PlayingItemCapabilities, RemoveLibraryRecordRequest, RequestCapability};

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let local_id = Uuid::new_v4();
    let fixture = fixture_with_broadcaster(
        dir.path(),
        local_id,
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let (endpoint, _, _, _, _, server) = subsonic_hub_mock(false).await;
    let record_id = open_subsonic_library_here(&node, endpoint).await?;
    let session_id = node.session()?.session_id().await.to_string();
    let item = play_subsonic_track_here(&fixture, record_id, "song-opaque").await?;
    let likes = || {
        signals
            .decoded::<PlayingItemCapabilities>("PlayingItemCapabilities")
            .into_iter()
            .rfind(|capabilities| capabilities.item.as_ref() == Some(&item))
            .map(|capabilities| capabilities.set_liked_request)
    };
    wait_until("the track's likes answered", || async {
        Ok((likes() == Some(RequestCapability::Available)).then_some(()))
    })
    .await?;

    // The local library opens again, and the service's record goes while its track plays: the UI
    // hears that nothing about the track can be answered.
    let told = statuses_for(&signals, &session_id, &item).len();
    let local = fixture_open_replica(&fixture, local_id).await?;
    // The session takes in the opened library between two of the track's statuses.
    wait_until("two more statuses", || async {
        Ok((statuses_for(&signals, &session_id, &item).len() >= told + 2).then_some(()))
    })
    .await?;
    dispatch_gui_request(
        &local.global_params,
        &RemoveLibraryRecordRequest {
            record_id: record_id.to_string(),
            call_id: None,
        },
    )
    .await?;
    wait_until("the track's likes gone", || async {
        Ok((likes() == Some(RequestCapability::Hidden)).then_some(()))
    })
    .await?;

    server.abort();
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_registration_uses_single_role_and_rejects_no_roles() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;

    assert_single_role_subsonic_registration(
        true,
        false,
        SubsonicOpenModeMessage::PlayOnThisDevice,
    )
    .await?;
    let remote_mode = SubsonicOpenModeMessage::RemoteControl;
    assert_single_role_subsonic_registration(false, true, remote_mode).await?;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: false,
        jukebox: false,
        wrong_password: false,
    })
    .await?;
    let response = register_subsonic_library_on_node(
        &subsonic_registration_request(server.endpoint.clone(), "password", None),
        &node,
    )
    .await;
    assert_eq!(registration_failure_kind(response)?, RegisterLibraryFailureKind::SubsonicRoleMissing);
    assert!(
        !PathBuf::from(&*node.config_path)
            .join("credentials.enc")
            .exists()
    );
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

async fn assert_single_role_subsonic_registration(
    stream: bool,
    jukebox: bool,
    expected: SubsonicOpenModeMessage,
) -> Result<()> {
    use crate::messages::RegisterLibraryOutcome;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream,
        jukebox,
        wrong_password: false,
    })
    .await?;
    let response = register_subsonic_library_on_node(
        &subsonic_registration_request(server.endpoint.clone(), "password", None),
        &node,
    )
    .await?;
    assert_eq!(
        response.outcome,
        RegisterLibraryOutcome::NeedsSubsonicMode {
            allowed: vec![expected]
        }
    );
    assert!(
        LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?
            .libraries
            .iter()
            .all(|record| !matches!(&record.protocol, ProtocolAdapter::SubsonicTranslate { .. }))
    );
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_registration_wrong_password_does_not_store_secret() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: false,
        wrong_password: true,
    })
    .await?;
    let password = "wrong-password-must-not-leak";
    let response = register_subsonic_library_on_node(
        &subsonic_registration_request(server.endpoint.clone(), password, None),
        &node,
    )
    .await;
    let error = match response {
        Ok(response) => anyhow::bail!("a wrong password registered: {:?}", response.outcome),
        Err(error) => error,
    };
    assert!(!format!("{error:#}").contains(password));
    assert_eq!(
        registration_failure_kind(Err(error))?,
        RegisterLibraryFailureKind::SubsonicWrongCredentials
    );
    assert!(
        !PathBuf::from(&*node.config_path)
            .join("credentials.enc")
            .exists()
    );
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_registration_unreachable_is_bounded() -> Result<()> {
    use crate::messages::RegisterLibraryFailureKind;

    let (endpoint, unreachable_server) = unreachable_http_endpoint().await?;
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let response = timeout(
        TEST_DEADLINE,
        register_subsonic_library_on_node(
            &subsonic_registration_request(endpoint, "password", None),
            &node,
        ),
    )
    .await
    .context("unreachable registration hung")?;
    assert_eq!(registration_failure_kind(response)?, RegisterLibraryFailureKind::SubsonicUnreachable);
    unreachable_server.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subsonic_registration_receiver_dispatches_to_node_handler() -> Result<()> {
    use crate::messages::RegisterLibraryOutcome;
    use rinf::{DartSignalPack, signal_channel};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = fixture.global_params.node.clone();
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: false,
        wrong_password: false,
    })
    .await?;
    let (sender, receiver) = signal_channel();
    sender.send(DartSignalPack {
        message: subsonic_registration_request(server.endpoint.clone(), "password", None),
        binary: Vec::new(),
    });
    let signal = timeout(TEST_DEADLINE, receiver.recv())
        .await
        .context("registration receiver did not receive the request")?
        .context("registration signal channel closed")?;
    let response = register_subsonic_library_with_node(&signal.message, &node).await?;
    assert_eq!(
        response.outcome,
        RegisterLibraryOutcome::NeedsSubsonicMode {
            allowed: vec![SubsonicOpenModeMessage::PlayOnThisDevice]
        }
    );
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_whose_folder_was_moved_is_not_opened_as_an_empty_one() -> Result<()> {
    use crate::library_pool::MediaFolderMissing;
    use crate::messages::{
        LibraryInitializeMode, OpenLibraryFailureKind, OpenLibraryRequest, RegisterLibraryOutcome,
        RegisterLocalLibraryRequest,
    };
    use database::library_registry::{LibraryRecordId, LibraryRegistry};
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        Arc::new(RecordedSignals::default()),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let folder = dir.path().join("Music");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("track.ogg"), "audio")?;
    let registered = register_local_library_on_node(
        &RegisterLocalLibraryRequest {
            call_id: None,
            path: folder.to_string_lossy().into_owned(),
            alias: "Music".into(),
            mode: Some(LibraryInitializeMode::Portable),
        },
        &node,
    )
    .await?;
    assert_eq!(registered.outcome, RegisterLibraryOutcome::Registered);
    let record_id = registered.record.context("registered record")?.record_id;
    let library_id = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?
        .record(LibraryRecordId(Uuid::parse_str(&record_id)?))?
        .library_id
        .context("local library id")?;
    let aside = dir.path().join("Music.offline");
    fs::rename(&folder, &aside)?;

    // Background work reaches the library before the user opens it.
    let background = node.library_pool.acquire_background(library_id).await;
    let error = background
        .err()
        .context("background work opened a library without its folder")?;
    assert!(
        error.downcast_ref::<MediaFolderMissing>().is_some(),
        "{error:#}"
    );
    let opened = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(1),
            record_id,
        },
        node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await;
    assert_eq!(
        opened_failure_kind(opened)?,
        OpenLibraryFailureKind::MediaFolderMissing
    );

    assert!(!folder.exists());
    assert_eq!(fs::read(aside.join("track.ogg"))?, b"audio");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_records_register_open_and_preserve_previous_ui_on_failure() -> Result<()> {
    use crate::messages::{
        InvalidRecordReasonSummary, LibraryInitializeMode, LibraryRecordKindSummary,
        LibraryRecordsChanged, ListLibraryRecordsRequest, OpenLibraryFailureKind,
        OpenLibraryRequest, RegisterLibraryFailureKind, RegisterLibraryOutcome,
        RegisterLocalLibraryRequest, RegisterRemoteControlRequest, RemoveLibraryRecordFailureKind,
        RemoveLibraryRecordRequest,
    };
    use database::library_registry::{
        CatalogPlacement, FsAdapter, LibraryRecord, LibraryRecordId, LibraryRegistry,
        ProtocolAdapter, RuneForwardMode, SubsonicOpenMode,
    };
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;

    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = super::fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    signals.clear();
    let selected = dir.path().join("selected");
    fs::create_dir_all(&selected)?;
    let request = RegisterLocalLibraryRequest {
        call_id: None,
        path: selected.to_string_lossy().into_owned(),
        alias: "Selected".into(),
        mode: None,
    };
    assert_eq!(
        register_local_library_on_node(&request, &node)
            .await?
            .outcome,
        RegisterLibraryOutcome::NeedsInitializeMode
    );
    let registered = register_local_library_on_node(
        &RegisterLocalLibraryRequest {
            call_id: None,
            mode: Some(LibraryInitializeMode::Portable),
            ..request
        },
        &node,
    )
    .await?;
    assert_eq!(registered.outcome, RegisterLibraryOutcome::Registered);
    let record_id = registered.record.context("registered record")?.record_id;
    assert!(
        signals
            .decoded::<LibraryRecordsChanged>("LibraryRecordsChanged")
            .iter()
            .any(|changed| changed
                .records
                .iter()
                .any(|record| record.record_id == record_id))
    );
    let scrobbler = Arc::new(Mutex::new(ScrobblingManager::new(
        10,
        Duration::from_secs(5),
    )));
    let opened = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(4),
            record_id: record_id.clone(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await?;
    let first_generation = opened.generation;
    assert!(first_generation > 0);
    let first_snapshot = capability_snapshot(&node)?;
    assert_eq!(
        first_snapshot.record_id.as_deref(),
        Some(record_id.as_str())
    );
    assert_eq!(first_snapshot.generation, first_generation);
    assert_eq!(
        first_snapshot.set_download_rule_request,
        RequestCapability::Available
    );
    let current = node.current_ui().await?;
    dispatch_gui_request(&current, &FetchAllPlaylistsRequest { call_id: None }).await?;

    let second_path = dir.path().join("selected-second");
    fs::create_dir_all(&second_path)?;
    let second_registration = register_local_library_on_node(
        &RegisterLocalLibraryRequest {
            call_id: None,
            path: second_path.to_string_lossy().into_owned(),
            alias: "Selected second".into(),
            mode: Some(LibraryInitializeMode::Portable),
        },
        &node,
    )
    .await?;
    assert_eq!(
        second_registration.outcome,
        RegisterLibraryOutcome::Registered
    );
    let second_record_id = second_registration
        .record
        .context("second registered record")?
        .record_id;
    let second_opened = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(9),
            record_id: second_record_id.clone(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await?;
    assert!(second_opened.generation > first_generation);
    let second_snapshot = capability_snapshot(&node)?;
    assert_eq!(
        second_snapshot.record_id.as_deref(),
        Some(second_record_id.as_str())
    );
    assert_eq!(second_snapshot.generation, second_opened.generation);
    let invalid_id = LibraryRecordId(Uuid::new_v4());
    let mut invalid_record = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?
        .record(LibraryRecordId(Uuid::parse_str(&second_record_id)?))?
        .clone();
    invalid_record.record_id = invalid_id;
    invalid_record.library_id = Some(Uuid::new_v4());
    invalid_record.fs.clear();
    let registry_path = PathBuf::from(&*node.config_path).join("libraries.json");
    let mut registry_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path)?)?;
    registry_json["libraries"]
        .as_array_mut()
        .context("registry libraries")?
        .push(serde_json::to_value(&invalid_record)?);
    fs::write(&registry_path, serde_json::to_vec_pretty(&registry_json)?)?;
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(30),
            record_id: record_id.clone(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await?;
    let invalid_open = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(31),
            record_id: invalid_id.0.to_string(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await;
    assert_eq!(opened_failure_kind(invalid_open)?, OpenLibraryFailureKind::InvalidRecord);
    assert!(
        LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?
            .invalid
            .iter()
            .any(|(id, _)| *id == invalid_id)
    );
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(32),
            record_id: second_record_id.clone(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await?;
    let current = node.current_ui().await?;

    let unknown = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(5),
            record_id: Uuid::new_v4().to_string(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await;
    assert_eq!(opened_failure_kind(unknown)?, OpenLibraryFailureKind::RecordNotFound);

    let unreachable = format!("unreachable-{}", Uuid::new_v4());
    let missing_replica = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Missing replica".into(),
        library_id: Some(Uuid::new_v4()),
        protocol: ProtocolAdapter::RuneForward {
            owner_fingerprint: unreachable.clone(),
            mode: RuneForwardMode::Replica {
                access_level: None,
                connection_access: None,
            },
        },
        fs: vec![FsAdapter::RunePeer {
            fingerprint: unreachable.clone(),
        }],
        catalog: CatalogPlacement::ConfigReplica,
        background_sync: true,
    };
    LibraryRegistry::update(PathBuf::from(&*node.config_path).as_path(), |registry| {
        registry.libraries.push(missing_replica.clone());
        Ok(())
    })
    .await?;
    let missing = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(8),
            record_id: missing_replica.record_id.0.to_string(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await;
    assert_eq!(opened_failure_kind(missing)?, OpenLibraryFailureKind::MediaFolderMissing);

    let subsonic_credential = node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"test-secret".to_vec()),
        })
        .await?;
    let subsonic = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Unsupported".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint: "https://example.invalid".into(),
            credential: subsonic_credential,
            mode: SubsonicOpenMode::PlayOnThisDevice,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    LibraryRegistry::update(PathBuf::from(&*node.config_path).as_path(), |registry| {
        registry.libraries.push(subsonic.clone());
        Ok(())
    })
    .await?;
    let unreachable_subsonic = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(6),
            record_id: subsonic.record_id.0.to_string(),
        },
        node.clone(),
        scrobbler.clone(),
    )
    .await;
    assert_eq!(opened_failure_kind(unreachable_subsonic)?, OpenLibraryFailureKind::RemoteUnreachable);

    node.cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&unreachable, vec!["127.0.0.1:1".into()])
        .await?;
    let remote = register_remote_control_on_node(
        &RegisterRemoteControlRequest {
            call_id: None,
            fingerprint: unreachable.clone(),
            alias: "Away".into(),
        },
        &node,
    )
    .await?;
    assert_eq!(remote.outcome, RegisterLibraryOutcome::Registered);
    let remote_id = remote.record.context("remote record")?.record_id;
    let listed = ListLibraryRecordsRequest { call_id: None }
        .handle(
            node.clone(),
            None,
            &ListLibraryRecordsRequest { call_id: None },
        )
        .await?
        .context("list library records")?;
    let invalid_summary = listed
        .invalid_records
        .iter()
        .find(|record| record.record_id == invalid_id.0.to_string())
        .context("invalid library record summary")?;
    assert_eq!(invalid_summary.alias, invalid_record.alias);
    assert_eq!(
        invalid_summary.reason,
        InvalidRecordReasonSummary::NativeInvalid
    );
    assert_eq!(
        listed.last_open_record.as_deref(),
        Some(second_record_id.as_str())
    );
    let local_summary = listed
        .records
        .iter()
        .find(|record| record.record_id == record_id)
        .context("local record summary")?;
    assert_eq!(local_summary.alias, "Selected");
    assert_eq!(local_summary.background_sync, Some(true));
    assert!(matches!(
        &local_summary.kind,
        LibraryRecordKindSummary::Local { path, .. } if path == &selected.to_string_lossy()
    ));
    let replica_summary = listed
        .records
        .iter()
        .find(|record| record.record_id == missing_replica.record_id.0.to_string())
        .context("replica record summary")?;
    assert!(matches!(
        &replica_summary.kind,
        LibraryRecordKindSummary::Replica { owner_fingerprint, owner_alias: Some(owner_alias), .. }
            if owner_fingerprint == &unreachable && owner_alias == "Away"
    ));
    assert!(listed.records.iter().any(|record| {
        record.record_id == remote_id
            && matches!(
                &record.kind,
                LibraryRecordKindSummary::RemoteControl { fingerprint, .. }
                    if fingerprint == &unreachable
            )
    }));
    assert!(listed.records.iter().any(|record| {
        record.record_id == subsonic.record_id.0.to_string()
            && matches!(
                &record.kind,
                LibraryRecordKindSummary::Subsonic { endpoint, account }
                    if endpoint == "https://example.invalid" && account == "account"
            )
    }));
    assert_eq!(
        listed.open.as_ref().map(|open| open.record_id.as_str()),
        Some(second_record_id.as_str())
    );
    let failure = open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(7),
            record_id: remote_id.clone(),
        },
        node.clone(),
        scrobbler,
    )
    .await;
    opened_failure_kind(failure)?;
    let after_failure = node.current_ui().await?;
    assert!(Arc::ptr_eq(&current, &after_failure));
    dispatch_gui_request(&after_failure, &FetchAllPlaylistsRequest { call_id: None }).await?;

    let open_removal = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: second_record_id.clone(),
    }
    .handle(
        node.clone(),
        None,
        &RemoveLibraryRecordRequest {
            call_id: None,
            record_id: second_record_id.clone(),
        },
    )
    .await;
    assert_eq!(
        removal_failure_kind(open_removal)?,
        RemoveLibraryRecordFailureKind::RecordIsOpen
    );
    let unknown_removal = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: Uuid::new_v4().to_string(),
    };
    assert_eq!(
        removal_failure_kind(
            unknown_removal
                .handle(node.clone(), None, &unknown_removal)
                .await
        )?,
        RemoveLibraryRecordFailureKind::RecordNotFound
    );
    let remove_remote = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: remote_id.clone(),
    };
    signals.clear();
    remove_remote
        .handle(node.clone(), None, &remove_remote)
        .await?
        .context("remote record removal")?;
    let subsonic_cache = PathBuf::from(&*node.config_path)
        .join("subsonic-covers")
        .join(subsonic.record_id.0.to_string());
    fs::create_dir_all(&subsonic_cache)?;
    fs::write(subsonic_cache.join("cover.img"), b"cover")?;
    let remove_subsonic = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: subsonic.record_id.0.to_string(),
    };
    remove_subsonic
        .handle(node.clone(), None, &remove_subsonic)
        .await?
        .context("Subsonic record removal")?;
    assert!(!subsonic_cache.exists());
    let remove_invalid = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: invalid_id.0.to_string(),
    };
    remove_invalid
        .handle(node.clone(), None, &remove_invalid)
        .await?
        .context("invalid record removal")?;
    assert!(
        signals
            .decoded::<LibraryRecordsChanged>("LibraryRecordsChanged")
            .iter()
            .any(|changed| changed
                .records
                .iter()
                .all(|record| record.record_id != remote_id))
    );

    let before = LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?
        .libraries
        .len();
    let untrusted = register_remote_control_on_node(
        &RegisterRemoteControlRequest {
            call_id: None,
            fingerprint: "untrusted-fingerprint".into(),
            alias: "No".into(),
        },
        &node,
    )
    .await;
    assert_eq!(registration_failure_kind(untrusted)?, RegisterLibraryFailureKind::Untrusted);
    assert_eq!(
        LibraryRegistry::load(PathBuf::from(&*node.config_path).as_path())?
            .libraries
            .len(),
        before
    );

    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registered_remote_control_opens_and_routes_library_and_session_requests() -> Result<()> {
    use crate::messages::{
        OpenLibraryRequest, RegisterLibraryOutcome, RegisterRemoteControlRequest,
    };
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let a = fixture(a_dir.path()).await?;
    let b = fixture(b_dir.path()).await?;
    b.global_params.node.set_open_local_catalog(
        None,
        b.global_params.library_id.context("renderer library id")?,
    ).await?;
    let address = super::paired_full(&b, &a).await?;
    let fingerprint = super::info(&b).await?.fingerprint;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;
    let registered = register_remote_control_on_node(
        &RegisterRemoteControlRequest {
            call_id: None,
            fingerprint,
            alias: "B".into(),
        },
        &a.global_params.node,
    )
    .await?;
    assert_eq!(registered.outcome, RegisterLibraryOutcome::Registered);
    let record_id = registered.record.context("remote record")?.record_id;
    let scrobbler = Arc::new(Mutex::new(ScrobblingManager::new(
        10,
        Duration::from_secs(5),
    )));
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: Some(22),
            record_id,
        },
        a.global_params.node.clone(),
        scrobbler,
    )
    .await?;
    let params = a.global_params.node.current_ui().await?;
    assert_eq!(params.library_id, b.global_params.library_id);
    let other_id = super::held_owned_library(&b, &b_dir.path().join("other-library")).await?;
    super::scan_held_track(&b, other_id, "other-library-track.ogg").await?;
    dispatch_gui_request(&params, &FetchAllPlaylistsRequest { call_id: None }).await?;
    dispatch_gui_request(
        &params,
        &VolumeRequest {
            call_id: None,
            volume: 0.37,
        },
    )
    .await?;
    wait_player(&b, |status| status.volume == 0.37).await?;

    let registry = LibraryRegistry::load(Path::new(&*b.global_params.config_path))?;
    let record_id = registry.catalog(other_id)?.record_id.0.to_string();
    open_library_by_record_on_node(
        &OpenLibraryRequest {
            call_id: None,
            record_id,
        },
        b.global_params.node.clone(),
        Arc::new(Mutex::new(ScrobblingManager::new(
            10,
            Duration::from_secs(5),
        ))),
    )
    .await?;
    let link = a
        .global_params
        .node
        .renderer_link()?
        .context("remote renderer link")?;
    wait_until("remote link library selection", || async {
        Ok((link.active_library_id() == Some(other_id)).then_some(()))
    })
    .await
    .context("renderer library-open event did not update the remote link")?;
    let count = dispatch_gui_request(&params, &GetMediaFilesCountRequest { call_id: None })
        .await?
        .context("remote media count response")?;
    assert_eq!(count.count, 1);
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

struct TestTcpProxy {
    listen_addr: SocketAddr,
    connection_count: Arc<AtomicUsize>,
    request_bytes: Arc<AtomicUsize>,
    server: JoinHandle<()>,
}

impl TestTcpProxy {
    async fn new(target_addr: SocketAddr) -> Result<Self> {
        let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
        let listen_addr = listener.local_addr()?;
        let connection_count = Arc::new(AtomicUsize::new(0));
        let request_bytes = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn({
            let connection_count = connection_count.clone();
            let request_bytes = request_bytes.clone();
            async move {
                while let Ok((client, _)) = listener.accept().await {
                    let _ = client.set_nodelay(true);
                    connection_count.fetch_add(1, Ordering::SeqCst);
                    let request_bytes = request_bytes.clone();
                    tokio::spawn(async move {
                        let Ok(server) = TcpStream::connect(target_addr).await else {
                            return;
                        };
                        let _ = server.set_nodelay(true);
                        let (mut client_reader, mut client_writer) = client.into_split();
                        let (mut server_reader, mut server_writer) = server.into_split();
                        let client_to_server = async move {
                            let mut buffer = [0_u8; 8192];
                            while let Ok(read) = client_reader.read(&mut buffer).await {
                                if read == 0 {
                                    break;
                                }
                                request_bytes.fetch_add(read, Ordering::SeqCst);
                                if server_writer.write_all(&buffer[..read]).await.is_err() {
                                    break;
                                }
                            }
                            let _ = server_writer.shutdown().await;
                        };
                        let server_to_client = async move {
                            let _ = copy_io(&mut server_reader, &mut client_writer).await;
                            let _ = client_writer.shutdown().await;
                        };
                        tokio::join!(client_to_server, server_to_client);
                    });
                }
            }
        });
        Ok(Self {
            listen_addr,
            connection_count,
            request_bytes,
            server,
        })
    }

    fn reset(&self) {
        self.connection_count.store(0, Ordering::SeqCst);
        self.request_bytes.store(0, Ordering::SeqCst);
    }

    fn connection_count(&self) -> usize {
        self.connection_count.load(Ordering::SeqCst)
    }

    fn request_bytes(&self) -> usize {
        self.request_bytes.load(Ordering::SeqCst)
    }
}

impl Drop for TestTcpProxy {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_library_request_without_renderer_library_is_typed_and_not_sent() -> Result<()> {
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller = fixture(controller_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    renderer.global_params.node.set_open_local_catalog(
        None,
        renderer
            .global_params
            .library_id
            .context("renderer library id")?,
    )
    .await?;
    let address = super::paired_full(&renderer, &controller).await?;
    controller.stop_worker().await?;
    renderer.stop_worker().await?;
    let fingerprint = super::info(&renderer).await?.fingerprint;
    let proxy = TestTcpProxy::new(address).await?;
    let proxy_host = format!("127.0.0.1:{}", proxy.listen_addr.port());
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![proxy_host.clone()])
        .await?;
    let (params, _) = open_remote_library_for_fingerprint(
        controller.global_params.node.clone(),
        &fingerprint,
        vec![proxy_host],
        None,
    )
    .await?;
    let link = controller
        .global_params
        .node
        .renderer_link()?
        .context("remote renderer link")?;
    assert_eq!(link.active_library_id(), renderer.global_params.library_id);

    let renderer_ui = renderer.global_params.node.current_ui().await?;
    super::close_ui(&renderer_ui).await?;
    wait_until("remote link library close", || async {
        Ok(link.active_library_id().is_none().then_some(()))
    })
    .await
    .context("renderer close event did not clear the remote link library")?;

    let before_probe = proxy.request_bytes();
    dispatch_gui_request(&params, &SessionSnapshotRequest { call_id: None })
        .await?
        .context("barrier snapshot before closed-library request")?;
    let barrier_bytes = proxy.request_bytes() - before_probe;
    assert!(barrier_bytes > 0);

    let before_request = proxy.request_bytes();
    let error =
        match dispatch_gui_request(&params, &GetMediaFilesCountRequest { call_id: None }).await {
            Ok(_) => anyhow::bail!("library request without renderer library must fail"),
            Err(error) => error,
        };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable { what: "library id" })
    ));
    // Barrier on the same link: once its reply arrives, any bytes the controller sent
    // earlier have already passed through the proxy.
    dispatch_gui_request(&params, &SessionSnapshotRequest { call_id: None })
        .await?
        .context("barrier snapshot after closed-library request")?;
    assert_eq!(proxy.request_bytes() - before_request, barrier_bytes);

    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

/// Opening a remote library must publish the remote player's queue from the
/// open itself. The renderer answers the refresh only after it has re-emitted
/// its session, so the initial queue event can reach the reader before the
/// open task has installed the link; only an already installed link may
/// publish it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opening_a_remote_library_publishes_the_remote_queue() -> Result<()> {
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer = super::fixture(renderer_dir.path()).await?;
    let first = session_audio(&renderer, "remote-open-first.ogg").await?;
    let second = session_audio(&renderer, "remote-open-second.ogg").await?;
    queue_session(&renderer, &[first, second]).await?;
    let local_track = session_audio(&controller, "remote-open-local.ogg").await?;
    queue_session(&controller, &[local_track]).await?;
    let address = super::paired_full(&renderer, &controller).await?;
    controller.stop_worker().await?;
    renderer.stop_worker().await?;
    let fingerprint = super::info(&renderer).await?.fingerprint;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &fingerprint,
            vec![format!("127.0.0.1:{}", address.port())],
        )
        .await?;
    let expected: Vec<PlayingItemRef> = renderer
        .global_params
        .player
        .lock()
        .await
        .get_playlist()
        .iter()
        .filter_map(|item| match item {
            PlayingItem::LibraryTrack { library_id, track } => Some(PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: track.to_string(),
            }),
            _ => None,
        })
        .collect();
    assert_eq!(expected.len(), 2);
    let remote_session_id = renderer.global_params.node.session()?.session_id().await;
    signals.clear();

    open_remote_library_for_fingerprint(
        controller.global_params.node.clone(),
        &fingerprint,
        vec![format!("127.0.0.1:{}", address.port())],
        None,
    )
    .await?;

    // The queue is settled and paused, so nothing emits another full queue
    // update after this open: the initial one must have been published, not
    // discarded before the link was installed.
    let update = {
        let remote_session_id = remote_session_id.clone();
        let expected = expected.clone();
        wait_until("the remote queue update from opening the library", move || {
            let remote_session_id = remote_session_id.clone();
            let expected = expected.clone();
            let signals = signals.clone();
            async move {
                Ok(signals
                    .decoded::<PlaylistUpdate>("PlaylistUpdate")
                    .into_iter()
                    .find(|update| {
                        update.session_id == remote_session_id
                            && update.items.iter().map(|item| item.item.clone()).eq(expected.iter().cloned())
                    }))
            }
        })
        .await?
    };
    assert_eq!(update.items.len(), 2);
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

/// An open that fails before the link is installed installs nothing and keeps
/// the local player current.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_remote_open_installs_nothing_and_keeps_the_local_player() -> Result<()> {
    use crate::backends::remote::RemoteConnectError;

    let controller_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let local_track = session_audio(&controller, "failed-remote-open-local.ogg").await?;
    queue_session(&controller, &[local_track]).await?;
    signals.clear();

    let error = open_remote_library_for_fingerprint(
        controller.global_params.node.clone(),
        "untrusted-fingerprint",
        vec!["127.0.0.1:9".into()],
        None,
    )
    .await
    .err()
    .context("opening an untrusted endpoint must fail")?;
    assert!(matches!(
        error.downcast_ref::<RemoteConnectError>(),
        Some(RemoteConnectError::UntrustedEndpoint)
    ));
    assert!(controller.global_params.node.renderer_link()?.is_none());
    assert!(matches!(
        controller.global_params.node.current_player()?,
        CurrentPlayer::Local
    ));
    assert!(
        signals.decoded::<PlaylistUpdate>("PlaylistUpdate").is_empty(),
        "a failed open must not publish queue events"
    );
    controller.global_params.node.shutdown().await?;
    Ok(())
}

fn reconnect_config(deadline: Duration) -> ReconnectConfig {
    ReconnectConfig {
        delays: [Duration::from_millis(50); 4],
        interval: Duration::from_millis(80),
        deadline,
    }
}

struct ReconnectFixture {
    a: Fixture,
    b: Fixture,
    remote: Arc<GlobalParams>,
    signals: Arc<RecordedSignals>,
    address: SocketAddr,
    fingerprint: String,
    proxy: TestTcpProxy,
    _directories: [TempDir; 2],
}

enum ReconnectFixtureQueue {
    Empty,
    WithTrack,
}

#[derive(Clone, Copy, Debug)]
enum ReconnectFixtureExit {
    Close,
    Switch,
    Shutdown,
}

impl ReconnectFixture {
    async fn new(deadline: Duration) -> Result<Self> {
        Self::with_queue(deadline, ReconnectFixtureQueue::WithTrack).await
    }

    async fn with_queue(deadline: Duration, queue: ReconnectFixtureQueue) -> Result<Self> {
        let directories = [tempfile::tempdir()?, tempfile::tempdir()?];
        let signals = Arc::new(RecordedSignals::default());
        let a = step(
            "create controller fixture",
            super::fixture_with_reconnect_config(
                directories[0].path(),
                Uuid::new_v4(),
                Duration::from_secs(300),
                signals.clone(),
                reconnect_config(deadline),
            ),
        )
        .await?;
        let b = step("create renderer fixture", fixture(directories[1].path())).await?;
        let address = step("pair reconnect fixtures", super::paired_full(&b, &a)).await?;
        step("stop background sync workers", async {
            a.stop_worker().await?;
            b.stop_worker().await
        })
        .await?;
        let fingerprint = step("read renderer identity", super::info(&b))
            .await?
            .fingerprint;
        let proxy = step("start renderer proxy", TestTcpProxy::new(address)).await?;
        step("store reconnect endpoint", async {
            a.global_params
                .cert_validator
                .read()
                .await
                .replace_hosts_for_fingerprint(
                    &fingerprint,
                    vec![format!("127.0.0.1:{}", proxy.listen_addr.port())],
                )
                .await?;
            Ok::<_, anyhow::Error>(())
        })
        .await?;
        match queue {
            ReconnectFixtureQueue::Empty => {}
            ReconnectFixtureQueue::WithTrack => {
                let track =
                    step("create reconnect track", session_audio(&b, "reconnect.ogg")).await?;
                step("queue reconnect track", queue_session(&b, &[track])).await?;
            }
        }
        let remote = step(
            "open remote library",
            super::open_remote_control_record(a.global_params.node.clone(), &fingerprint),
        )
        .await?;
        signals.clear();
        proxy.reset();
        Ok(Self {
            a,
            b,
            remote,
            signals,
            address,
            fingerprint,
            proxy,
            _directories: directories,
        })
    }

    async fn wait_signal<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        wait_until(name, || async {
            Ok(self.signals.decoded(name).into_iter().next())
        })
        .await
    }

    async fn disconnect(&self) -> Result<()> {
        self.b.manager.stop().await?;
        let event: RemoteLibraryReconnecting =
            self.wait_signal("RemoteLibraryReconnecting").await?;
        assert_eq!(event.fingerprint, self.fingerprint);
        assert_eq!(event.alias, super::info(&self.b).await?.alias);
        assert!(!self.remote.main_token.is_cancelled());
        assert!(matches!(
            self.a.global_params.node.current_library()?,
            OpenLibrary::RunePeer { .. }
        ));
        assert!(
            self.signals
                .decoded::<RemoteConnectionLost>("RemoteConnectionLost")
                .is_empty()
        );
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        self.a.global_params.node.shutdown().await?;
        self.b.global_params.node.shutdown().await?;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_restores_real_node_requests() -> Result<()> {
    let f = step(
        "create reconnect fixture",
        Box::pin(ReconnectFixture::new(Duration::from_secs(10))),
    )
    .await?;
    f.disconnect().await?;
    let mut browse = start::<FetchAllPlaylistsRequest>(&f.remote);
    browse.send(FetchAllPlaylistsRequest { call_id: None });
    let failure: RequestFailedResponse =
        timeout(TEST_DEADLINE, browse.receive("RequestFailedResponse")).await?;
    assert_eq!(failure.kind, RequestErrorKind::Unavailable);
    assert_eq!(
        failure.cause,
        Some(RequestFailureCause::Reconnecting {
            alias: super::info(&f.b).await?.alias
        })
    );
    browse.stop().await;
    let failure = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(
            &f.remote,
            &VolumeRequest {
                call_id: None,
                volume: 0.7,
            },
        ),
    )
    .await?
    .err()
    .context("session request must fail")?;
    assert!(matches!(
        failure.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Reconnecting { .. })
    ));
    dispatch_gui_request(&f.remote, &ListLocalLibrariesRequest { call_id: None })
        .await?
        .context("node requests stay local")?;
    f.signals.clear();
    f.a.global_params
        .node
        .session()?
        .player
        .lock()
        .await
        .set_volume(0.2);
    // Applying the volume is the barrier before checking that the remote view emits no status.
    sleep(Duration::from_millis(50)).await;
    assert!(
        f.signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .is_empty(),
        "local session replaced the remote view"
    );
    f.b.manager
        .clone()
        .start(
            f.address,
            DiscoveryParams {
                device_info: super::info(&f.b).await?,
            },
            true,
        )
        .await?;
    let event: RemoteLibraryReconnected = f.wait_signal("RemoteLibraryReconnected").await?;
    assert_eq!(event.fingerprint, f.fingerprint);
    assert_eq!(event.alias, super::info(&f.b).await?.alias);
    let session_id = f.b.global_params.node.session()?.session_id().await;
    let status: PlaybackStatus = f.wait_signal("PlaybackStatus").await?;
    let queue: PlaylistUpdate = f.wait_signal("PlaylistUpdate").await?;
    assert_eq!(status.session_id, session_id);
    assert_eq!(queue.session_id, session_id);
    assert_eq!(queue.items.len(), 1);
    dispatch_gui_request(&f.remote, &FetchAllPlaylistsRequest { call_id: None })
        .await?
        .context("remote request")?;
    dispatch_gui_request(
        &f.remote,
        &VolumeRequest {
            call_id: None,
            volume: 0.7,
        },
    )
    .await?;
    wait_player(&f.b, |status| status.volume == 0.7).await?;
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_deadline_uses_existing_loss_and_close() -> Result<()> {
    let f = ReconnectFixture::new(Duration::from_millis(400)).await?;
    let started = Instant::now();
    f.disconnect().await?;
    let loss: RemoteConnectionLost = f.wait_signal("RemoteConnectionLost").await?;
    assert_eq!(loss.kind, RemoteLossKind::ServerStopped);
    assert!(started.elapsed() >= Duration::from_millis(400));
    let attempts = f.proxy.connection_count();
    assert!(attempts > 0);
    // The deadline-loss event is the barrier before checking that retries stopped.
    sleep(Duration::from_millis(200)).await;
    assert_eq!(f.proxy.connection_count(), attempts);
    let mut close = start::<CloseLibraryRequest>(&f.remote);
    close.send(CloseLibraryRequest { call_id: None });
    let _: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
    close.stop().await;
    assert!(matches!(
        f.a.global_params.node.current_library()?,
        OpenLibrary::Closed | OpenLibrary::Local
    ));
    assert!(f.a.global_params.node.current_ui().await.is_err());
    assert!(f.remote.main_token.is_cancelled());
    assert!(
        f.signals
            .decoded::<RemoteLibraryReconnected>("RemoteLibraryReconnected")
            .is_empty()
    );
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_stops_after_incompatible_peer_with_permanent_loss() -> Result<()> {
    use discovery::protocol_version::{PROTOCOL_VERSION, ProtocolVersion};

    let f = step(
        "create reconnect fixture",
        Box::pin(ReconnectFixture::new(Duration::from_secs(10))),
    )
    .await?;
    f.disconnect().await?;
    let mut device_info = super::info(&f.b).await?;
    // One major above ours, whatever ours is, so a protocol bump never breaks this test.
    device_info.protocol_version = Some(ProtocolVersion {
        major: PROTOCOL_VERSION.major + 1,
        ..PROTOCOL_VERSION
    });
    f.b.manager
        .clone()
        .start(f.address, DiscoveryParams { device_info }, true)
        .await?;
    let loss: RemoteConnectionLost = f.wait_signal("RemoteConnectionLost").await?;
    assert_eq!(loss.kind, RemoteLossKind::ProtocolIncompatible);
    let attempts = f.proxy.connection_count();
    // The permanent-loss event is the barrier before checking that no reconnect starts.
    sleep(Duration::from_millis(250)).await;
    assert_eq!(f.proxy.connection_count(), attempts);
    // One reconnect attempt probes the host over TLS and then opens `/ws`, where 426 stops it
    // before fetching device info.
    assert_eq!(attempts, 2);
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_leaving_stops_attempts() -> Result<()> {
    let mut f = step(
        "create reconnect fixture",
        Box::pin(ReconnectFixture::with_queue(
            Duration::from_secs(2),
            ReconnectFixtureQueue::Empty,
        )),
    )
    .await?;
    for exit in [
        ReconnectFixtureExit::Close,
        ReconnectFixtureExit::Switch,
        ReconnectFixtureExit::Shutdown,
    ] {
        // The fixture is reused across exits and the attempt count never resets, so each
        // exit waits for an attempt made after its own disconnect.
        let attempts_before = f.proxy.connection_count();
        step("disconnect renderer", f.disconnect()).await?;
        step("wait for first reconnect attempt", async {
            wait_until("first reconnect attempt", || async {
                Ok((f.proxy.connection_count() > attempts_before).then_some(()))
            })
            .await
        })
        .await?;
        let replacement = step("prepare replacement library", async {
            Ok::<_, anyhow::Error>(Arc::new(
                f.a.global_params
                    .node
                    .params_for_library(f.a.global_params.test_library().clone())?,
            ))
        })
        .await?;
        step("leave remote library", async {
            match exit {
                ReconnectFixtureExit::Close => {
                    f.a.global_params.node.close_ui(&f.remote.main_token).await
                }
                ReconnectFixtureExit::Switch => {
                    f.a.global_params.node.attach_ui(&replacement).await
                }
                ReconnectFixtureExit::Shutdown => f.a.global_params.node.shutdown().await?,
            }
            Ok::<_, anyhow::Error>(())
        })
        .await?;
        let attempts = f.proxy.connection_count();
        step(
            "restart renderer",
            f.b.manager.clone().start(
                f.address,
                DiscoveryParams {
                    device_info: super::info(&f.b).await?,
                },
                true,
            ),
        )
        .await?;
        // Renderer startup is the barrier before checking that cancellation prevents a reconnect.
        step(
            "wait for cancellation to settle",
            sleep(Duration::from_millis(350)),
        )
        .await;
        step("verify reconnect stopped", async {
            assert_eq!(
                f.proxy.connection_count(),
                attempts,
                "attempt after {exit:?}"
            );
            assert!(f.a.global_params.node.renderer_link()?.is_none());
            assert!(
                f.signals
                    .decoded::<RemoteLibraryReconnected>("RemoteLibraryReconnected")
                    .is_empty()
            );
            assert!(
                f.signals
                    .decoded::<RemoteConnectionLost>("RemoteConnectionLost")
                    .is_empty()
            );
            Ok::<_, anyhow::Error>(())
        })
        .await?;
        match exit {
            ReconnectFixtureExit::Shutdown => {
                step("shutdown renderer fixture", f.b.global_params.node.shutdown()).await?;
            }
            ReconnectFixtureExit::Close | ReconnectFixtureExit::Switch => {
                f.remote = step(
                    "reopen remote library",
                    super::open_remote_control_record(
                        f.a.global_params.node.clone(),
                        &f.fingerprint,
                    ),
                )
                .await?;
                f.signals.clear();
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_capability_snapshot_marks_every_request_available() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;

    assert_eq!(
        capability_snapshot(&fixture.global_params.node)?,
        RequestCapabilities::all_available()
    );
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        1
    );
    fixture.global_params.node.publish_capabilities()?;
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        1
    );
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_library_answers_the_artist_page_with_derived_sections_and_tracks() -> Result<()> {
    use crate::messages::{
        ArtistSectionKind, ArtistSectionOpens, FetchArtistPageRequest,
        FetchArtistSectionAlbumsRequest, MixQueryRequest,
    };
    use database::actions::collection::{CollectionQuery, CollectionQueryListMode};
    use database::entities::{portable_albums, portable_artists};
    use database::test_fixtures::fixtures;

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let library = fixture.global_params.test_library();
    let db = &*library.main_db;
    // Two long songs of a release the tags call a single, and one of a release they flag as a
    // compilation.
    let root = Path::new(&library.media_root);
    let single = [("ALBUM", "Single"), ("RELEASETYPE", "single")];
    let collected = [("ALBUM", "Collected"), ("COMPILATION", "1"), ("DATE", "2021-04-01")];
    for (path, number, tags) in [
        ("Single/1.flac", "1", &single[..]),
        ("Single/2.flac", "2", &single[..]),
        ("Collected/1.flac", "1", &collected[..]),
    ] {
        let mut all = vec![
            ("ARTIST", "Page artist"),
            ("TITLE", path),
            ("TRACKNUMBER", number),
        ];
        all.extend_from_slice(tags);
        fixtures::write_tagged_flac(&root.join(path), 1200, &all)?;
    }
    Box::pin(fixtures::scan_library(
        &library.fsio,
        db,
        &fixture.global_params.node_id,
        root,
        &library.library_sync.record_store(),
    ))
    .await?;
    let artist = portable_artists::Entity::find()
        .filter(portable_artists::Column::Name.eq("Page artist"))
        .one(db)
        .await?
        .context("the scanned artist")?;

    let page = dispatch_gui_request(
        &fixture.global_params,
        &FetchArtistPageRequest {
            call_id: None,
            artist_id: artist.id,
            language: String::new(),
        },
    )
    .await?
    .context("artist page")?;
    assert_eq!(page.artist_id, artist.id);
    let kinds: Vec<_> = page.sections.iter().map(|section| section.kind).collect();
    assert_eq!(
        kinds,
        vec![ArtistSectionKind::Singles, ArtistSectionKind::Compilations]
    );
    let singles = &page.sections[0];
    assert_eq!(singles.count, 1);
    assert_eq!(page.sections[1].count, 1);
    // The local library gives no names of its own, singles play as songs and compilations
    // list their albums.
    assert!(page.sections.iter().all(|section| section.title.is_empty()));
    assert!(page.sections.iter().all(|section| section.album.is_none()));
    assert_eq!(singles.opens, ArtistSectionOpens::Songs);
    assert_eq!(page.sections[1].opens, ArtistSectionOpens::Albums);
    let albums = portable_albums::Model::list(db, 10, CollectionQueryListMode::Name).await?;
    let album_of = |name: &str| {
        albums
            .iter()
            .find(|album| album.name == name)
            .map(|album| album.id)
            .context("album")
    };
    let section_albums = |section: usize, cursor: i32, page_size: i32| {
        let queries = page.sections[section].queries.clone();
        let params = &fixture.global_params;
        async move {
            anyhow::Ok(
                dispatch_gui_request(
                    params,
                    &FetchArtistSectionAlbumsRequest {
                        call_id: None,
                        queries,
                        cursor,
                        page_size,
                    },
                )
                .await?
                .context("section albums")?
                .groups
                .into_iter()
                .map(|group| (group.group_title, group.album_ids))
                .collect::<Vec<_>>(),
            )
        }
    };
    let ids = |groups: Vec<(String, Vec<i32>)>| -> Vec<i32> {
        groups.into_iter().flat_map(|(_, ids)| ids).collect()
    };
    assert_eq!(ids(section_albums(0, 0, 10).await?), vec![album_of("Single")?]);
    // The albums are in groups by the year of their release dates.
    assert_eq!(
        section_albums(1, 0, 10).await?,
        vec![("2021".to_owned(), vec![album_of("Collected")?])]
    );
    assert!(section_albums(1, 1, 10).await?.is_empty());
    assert_eq!(
        singles
            .queries
            .iter()
            .map(|query| (query.operator.as_str(), query.parameter.as_str()))
            .collect::<Vec<_>>(),
        vec![(
            "lib::artist_section",
            format!("{}:singles", artist.id).as_str()
        )]
    );
    assert_eq!(page.tracks.len(), 1);
    assert_eq!(page.tracks[0].operator, "lib::artist_tracks");
    assert_eq!(page.tracks[0].parameter, artist.id.to_string());

    // What the page names opens through the paged mix query.
    let opened = dispatch_gui_request(
        &fixture.global_params,
        &MixQueryRequest {
            call_id: None,
            queries: singles.queries.clone(),
            cursor: 0,
            page_size: 10,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        },
    )
    .await?
    .context("section songs")?;
    assert_eq!(opened.files.len(), 2);

    let missing = dispatch_gui_request(
        &fixture.global_params,
        &FetchArtistPageRequest {
            call_id: None,
            artist_id: artist.id + 100,
            language: String::new(),
        },
    )
    .await;
    assert!(missing.is_err());
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn controller_capabilities_follow_catalogue_access_grants() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let node = &fixture.global_params.node;
    let library_id = fixture
        .global_params
        .library_id
        .context("fixture library identity")?;
    let peer = PeerIdentity {
        fingerprint: "guest-controller".into(),
        public_key: "guest-key".into(),
    };

    node.access_policy
        .set(&peer.fingerprint, DeviceGrants::guest())?;
    let mut guest = node.capabilities_for_peer(capability_snapshot(node)?, &peer, Some(library_id));
    let mut guest_connection_edit_count = 0;
    let mut guest_control_count = 0;
    let mut guest_node_local_count = 0;
    guest.for_each_capability_mut(|access, capability| match access {
        RequestAccess::ConnectionEdit => {
            guest_connection_edit_count += 1;
            assert_eq!(
                *capability,
                RequestCapability::Disabled {
                    reason: CapabilityReason::NotAllowed,
                }
            );
        }
        RequestAccess::Control => {
            guest_control_count += 1;
            assert_eq!(*capability, RequestCapability::Available);
        }
        RequestAccess::NodeLocal => {
            guest_node_local_count += 1;
            assert_eq!(
                *capability,
                RequestCapability::Disabled {
                    reason: CapabilityReason::OwnerOnly,
                }
            );
        }
        _ => {}
    });
    assert!(guest_connection_edit_count > 0);
    assert!(guest_control_count > 0);
    assert!(guest_node_local_count > 0);

    node.access_policy
        .set(&peer.fingerprint, DeviceGrants::my_device())?;
    let mut my_device =
        node.capabilities_for_peer(capability_snapshot(node)?, &peer, Some(library_id));
    let mut my_device_count = 0;
    my_device.for_each_capability_mut(|access, capability| {
        if matches!(
            access,
            RequestAccess::ConnectionEdit | RequestAccess::Control
        ) {
            my_device_count += 1;
            assert_eq!(*capability, RequestCapability::Available);
        }
    });
    assert!(my_device_count > 0);
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_scopes_resolve_to_the_current_implementations() -> Result<()> {
    use crate::messages::{
        FetchAllPlaylistsRequest, HandoffRequest, OpenLibraryRequest,
        PauseRequest,
    };
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;

    fn kind(implementation: &Implementation) -> &'static str {
        match implementation {
            Implementation::Local => "Local",
            Implementation::LocalBeside(_) => "LocalBeside",
            Implementation::RunePeer {
                reconnecting: Some(_),
                ..
            } => "Reconnecting",
            Implementation::RunePeer { .. } => "RunePeer",
            Implementation::Service(_) => "Subsonic",
        }
    }

    macro_rules! assert_scopes {
        ($node:expr, $library:literal, $player:literal, $label:expr) => {{
            let library = $node.implementation_for_scope::<FetchAllPlaylistsRequest>()?;
            let player = $node.implementation_for_scope::<PauseRequest>()?;
            let local = $node.implementation_for_scope::<HandoffRequest>()?;
            // This device answers node requests, beside what a Subsonic library declares.
            let node = if $library == "Subsonic" {
                "LocalBeside"
            } else {
                "Local"
            };
            assert_eq!(kind(&library), $library, "{} library", $label);
            assert_eq!(kind(&player), $player, "{} player", $label);
            assert_eq!(kind(&local), node, "{} node", $label);
        }};
    }

    let local_dir = tempfile::tempdir()?;
    let local = fixture(local_dir.path()).await?;
    assert_scopes!(local.global_params.node, "Local", "Local", "local library");

    let host_dir = tempfile::tempdir()?;
    let peer_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let peer = fixture(peer_dir.path()).await?;
    let (_, replica) = replicate_host_library(&host, &peer).await?;
    assert_scopes!(replica.global_params.node, "Local", "Local", "replica");

    let reconnecting = ReconnectFixture::new(Duration::from_secs(10)).await?;
    assert_scopes!(
        reconnecting.a.global_params.node,
        "RunePeer",
        "RunePeer",
        "@RR remote library"
    );
    reconnecting.disconnect().await?;
    assert_scopes!(
        reconnecting.a.global_params.node,
        "Reconnecting",
        "Reconnecting",
        "@RR reconnecting"
    );

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller = fixture(controller_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let renderer_address = paired_full(&renderer, &controller).await?;
    let renderer_info = info(&renderer).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;
    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert_scopes!(
        controller.global_params.node,
        "Local",
        "RunePeer",
        "attached renderer"
    );

    for jukebox in [false, true] {
        let directory = tempfile::tempdir()?;
        let subsonic_node = fixture(directory.path()).await?;
        let server = fake_subsonic_server(FakeSubsonicRoles {
            stream: true,
            jukebox,
            wrong_password: false,
        })
        .await?;
        let mode = if jukebox {
            SubsonicOpenModeMessage::RemoteControl
        } else {
            SubsonicOpenModeMessage::PlayOnThisDevice
        };
        let registered = register_subsonic_library_on_node(
            &subsonic_registration_request(server.endpoint.clone(), "password", Some(mode)),
            &subsonic_node.global_params.node,
        )
        .await?;
        assert_eq!(registered.outcome, RegisterLibraryOutcome::Registered);
        let record = registered.record.context("Subsonic record")?;
        open_library_by_record_on_node(
            &OpenLibraryRequest {
                call_id: Some(1),
                record_id: record.record_id,
            },
            subsonic_node.global_params.node.clone(),
            Arc::new(Mutex::new(ScrobblingManager::new(
                10,
                Duration::from_secs(5),
            ))),
        )
        .await?;
        if jukebox {
            assert_scopes!(
                subsonic_node.global_params.node,
                "Subsonic",
                "Subsonic",
                "Subsonic jukebox"
            );
        } else {
            assert_scopes!(
                subsonic_node.global_params.node,
                "Subsonic",
                "Local",
                "Subsonic on-device playback"
            );
        }
        server.task.abort();
        subsonic_node.global_params.node.shutdown().await?;
    }

    local.global_params.node.shutdown().await?;
    host.global_params.node.shutdown().await?;
    replica.global_params.node.shutdown().await?;
    reconnecting.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attached_renderer_selects_rune_slots_and_suppresses_equal_capabilities() -> Result<()> {
    use crate::messages::{HandoffRequest, PauseRequest};

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer = super::fixture(renderer_dir.path()).await?;
    let renderer_address = super::paired_full(&renderer, &controller).await?;
    let renderer_info = super::info(&renderer).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;

    signals.clear();
    assert_eq!(
        controller
            .global_params
            .node
            .request_slot::<PauseRequest>()?,
        Slot::Native
    );
    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert_eq!(
        controller
            .global_params
            .node
            .request_slot::<PauseRequest>()?,
        Slot::Forward
    );
    assert_eq!(
        controller
            .global_params
            .node
            .request_slot::<HandoffRequest>()?,
        Slot::Native
    );
    let capabilities = capability_snapshot(&controller.global_params.node)?;
    assert_eq!(capabilities.pause_request, RequestCapability::Available);
    assert_eq!(capabilities.handoff_request, RequestCapability::Available);
    let snapshots = signals.decoded::<RequestCapabilities>("RequestCapabilities");
    assert!(
        snapshots.is_empty(),
        "equal capability snapshots are suppressed"
    );
    controller.global_params.node.publish_capabilities()?;
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        0,
        "a no-op publication must be suppressed"
    );

    controller.global_params.node.detach_renderer().await?;
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        0,
        "detach restores the same capability snapshot and is suppressed"
    );
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

/// A controller with a renderer attached, and the renderer.
async fn attached_controller_and_renderer(
    controller_dir: &Path,
    renderer_dir: &Path,
    signals: Arc<RecordedSignals>,
) -> Result<(Fixture, Fixture)> {
    let controller = fixture_with_broadcaster(
        controller_dir,
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals,
    )
    .await?;
    let renderer = super::fixture(renderer_dir).await?;
    let renderer_address = super::paired_full(&renderer, &controller).await?;
    let renderer_info = super::info(&renderer).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;
    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    Ok((controller, renderer))
}

/// This device's own screen keeps its capability snapshot while a Rune player
/// is the current player, even though that player owns the session events.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn own_ui_receives_capabilities_while_a_rune_player_is_current() -> Result<()> {
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let (controller, renderer) =
        attached_controller_and_renderer(controller_dir.path(), renderer_dir.path(), signals.clone())
            .await?;
    let node = &controller.global_params.node;
    assert!(matches!(
        node.current_player()?,
        CurrentPlayer::RunePeer { .. }
    ));

    let library_id = Uuid::new_v4();
    signals.clear();
    node.set_open_local_catalog(None, library_id).await?;
    wait_until("this device's snapshot for the opened library", || async {
        Ok(signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .iter()
            .any(|snapshot| snapshot.library_id.as_deref() == Some(&library_id.to_string()))
            .then_some(()))
    })
    .await?;
    node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

/// A controller's screen shows the snapshot its remote player publishes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_receives_the_remote_player_capabilities() -> Result<()> {
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let (controller, renderer) =
        attached_controller_and_renderer(controller_dir.path(), renderer_dir.path(), signals.clone())
            .await?;

    let library_id = Uuid::new_v4();
    signals.clear();
    renderer
        .global_params
        .node
        .set_open_local_catalog(None, library_id)
        .await?;
    wait_until("the remote player's snapshot on the controller", || async {
        Ok(signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .iter()
            .any(|snapshot| snapshot.library_id.as_deref() == Some(&library_id.to_string()))
            .then_some(()))
    })
    .await?;
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

/// This device's own screen keeps its capability snapshot while a Subsonic
/// jukebox is the current player.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn own_ui_receives_capabilities_while_a_subsonic_jukebox_is_current() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let server = fake_subsonic_server(FakeSubsonicRoles {
        stream: true,
        jukebox: true,
        wrong_password: false,
    })
    .await?;
    let registration = register_subsonic_library_on_node(
        &subsonic_registration_request(
            server.endpoint.clone(),
            "password",
            Some(SubsonicOpenModeMessage::RemoteControl),
        ),
        &node,
    )
    .await?;
    let summary = registration.record.context("registered Subsonic record")?;
    signals.clear();
    let _ui = open_registered_library_record(node.clone(), summary.record_id.clone()).await?;
    assert!(matches!(
        node.current_player()?,
        CurrentPlayer::ServicePlayer(_)
    ));
    wait_until("this device's snapshot for the Subsonic library", || async {
        Ok(signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .iter()
            .any(|snapshot| snapshot.record_id.as_deref() == Some(summary.record_id.as_str()))
            .then_some(()))
    })
    .await?;
    server.task.abort();
    node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_binding_prefers_remote_peer_over_attached_renderer() -> Result<()> {
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller = fixture(controller_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let renderer_address = paired_full(&renderer, &controller).await?;
    let renderer_info = info(&renderer).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;

    let node = &controller.global_params.node;
    assert!(matches!(node.current_player()?, CurrentPlayer::Local));
    assert_eq!(node.current_player()?.visualizer_inputs(), VisualizerInputs::Full);
    node.attach_renderer(&renderer_info.fingerprint).await?;
    assert_eq!(node.current_player()?.visualizer_inputs(), VisualizerInputs::Full);
    let attached = match node.current_player()? {
        CurrentPlayer::RunePeer {
            link: Some(renderer), ..
        } => renderer,
        _ => anyhow::bail!("expected attached renderer binding"),
    };
    node.attach_remote_ui(&controller.global_params, renderer_info.fingerprint, None)
        .await?;
    assert_eq!(node.current_player()?.visualizer_inputs(), VisualizerInputs::Full);
    match node.current_player()? {
        CurrentPlayer::RunePeer {
            link: Some(renderer), ..
        } => assert_eq!(renderer.id(), attached.id()),
        _ => anyhow::bail!("remote peer must take precedence over attached renderer"),
    }
    node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_capability_snapshot_uses_rune_library_and_local_close_slots() -> Result<()> {
    let local_dir = tempfile::tempdir()?;
    let remote_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let local = fixture_with_broadcaster(
        local_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals,
    )
    .await?;
    let remote = super::fixture(remote_dir.path()).await?;
    let address = super::paired_full(&remote, &local).await?;
    let remote_info = super::info(&remote).await?;
    local
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &remote_info.fingerprint,
            vec![format!("127.0.0.1:{}", address.port())],
        )
        .await?;
    let remote_params = super::open_remote_control_record(
        local.global_params.node.clone(),
        &remote_info.fingerprint,
    )
    .await?;

    let snapshot = capability_snapshot(&local.global_params.node)?;
    assert_eq!(
        snapshot.fetch_all_playlists_request,
        RequestCapability::Available
    );
    assert_eq!(snapshot.close_library_request, RequestCapability::Available);
    assert_eq!(
        local
            .global_params
            .node
            .request_slot::<FetchAllPlaylistsRequest>()?,
        Slot::Forward
    );
    assert_eq!(
        local
            .global_params
            .node
            .request_slot::<CloseLibraryRequest>()?,
        Slot::Native
    );

    local
        .global_params
        .node
        .close_ui(&remote_params.main_token)
        .await;
    local.global_params.node.shutdown().await?;
    remote.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn node_owned_calls_stay_local_with_remote_library_and_attached_renderer() -> Result<()> {
    use crate::TypedRequest;
    use crate::macros::{BodyLibraries, RequestAccess, RequestSlots};
    use crate::messages::{
        AttachRendererRequest, CloseLibraryRequest, DetachControllerRequest, DetachRendererRequest,
        GetSyncSettingsRequest, HandoffRequest, ListControllersRequest, ListRenderersRequest,
        RemoveDownloadRuleRequest, SetDownloadRuleRequest, SetSyncSettingsRequest,
        TestLibraryInitializedRequest,
    };
    use crate::rpc::TargetRequest;

    struct NodeLocalSessionProbe;

    impl TypedRequest for NodeLocalSessionProbe {
        type Response = ();

        fn wire_name() -> &'static str {
            "NodeLocalSessionProbe"
        }

        fn response_name() -> &'static str {
            "NodeLocalSessionProbeResponse"
        }
    }

    impl TargetRequest for NodeLocalSessionProbe {
        const SCOPE: RequestScope = RequestScope::Session;
        const ACCESS: RequestAccess = RequestAccess::NodeLocal;
        const BODY_LIBRARIES: BodyLibraries = BodyLibraries::NoLibrary;
        const INTRODUCED_MINOR: u32 = 0;

        fn slot(slots: &RequestSlots) -> Slot {
            slots.pause_request
        }
    }

    macro_rules! assert_node_target {
        ($node:expr, $request:ty) => {
            assert_eq!(
                $node.request_slot::<$request>()?,
                Slot::Native,
                stringify!($request)
            );
        };
    }

    let local_dir = tempfile::tempdir()?;
    let remote_dir = tempfile::tempdir()?;
    let local = fixture(local_dir.path()).await?;
    let remote = super::fixture(remote_dir.path()).await?;
    let address = super::paired_full(&remote, &local).await?;
    let remote_info = super::info(&remote).await?;
    local
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &remote_info.fingerprint,
            vec![format!("127.0.0.1:{}", address.port())],
        )
        .await?;
    let remote_params = super::open_remote_control_record(
        local.global_params.node.clone(),
        &remote_info.fingerprint,
    )
    .await?;
    let remote_node = &local.global_params.node;
    assert_node_target!(remote_node, AttachRendererRequest);
    assert_node_target!(remote_node, DetachRendererRequest);
    assert_node_target!(remote_node, ListRenderersRequest);
    assert_node_target!(remote_node, HandoffRequest);
    assert_node_target!(remote_node, ListControllersRequest);
    assert_node_target!(remote_node, DetachControllerRequest);
    assert_node_target!(remote_node, GetSyncSettingsRequest);
    assert_node_target!(remote_node, SetSyncSettingsRequest);
    assert_node_target!(remote_node, SetDownloadRuleRequest);
    assert_node_target!(remote_node, RemoveDownloadRuleRequest);
    assert_node_target!(remote_node, TestLibraryInitializedRequest);
    assert_node_target!(remote_node, CloseLibraryRequest);
    local
        .global_params
        .node
        .close_ui(&remote_params.main_token)
        .await;
    local.global_params.node.shutdown().await?;
    remote.global_params.node.shutdown().await?;

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller = fixture(controller_dir.path()).await?;
    let renderer = super::fixture(renderer_dir.path()).await?;
    let renderer_address = super::paired_full(&renderer, &controller).await?;
    let renderer_info = super::info(&renderer).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;
    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    let attached_node = &controller.global_params.node;
    assert_eq!(
        attached_node.request_slot::<NodeLocalSessionProbe>()?,
        Slot::Forward,
        "scope selects the attached player even when authorization is NodeLocal"
    );
    assert_node_target!(attached_node, AttachRendererRequest);
    assert_node_target!(attached_node, DetachRendererRequest);
    assert_node_target!(attached_node, ListRenderersRequest);
    assert_node_target!(attached_node, HandoffRequest);
    assert_node_target!(attached_node, ListControllersRequest);
    assert_node_target!(attached_node, DetachControllerRequest);
    assert_node_target!(attached_node, GetSyncSettingsRequest);
    assert_node_target!(attached_node, SetSyncSettingsRequest);
    assert_node_target!(attached_node, SetDownloadRuleRequest);
    assert_node_target!(attached_node, RemoveDownloadRuleRequest);
    assert_node_target!(attached_node, TestLibraryInitializedRequest);
    assert_node_target!(attached_node, CloseLibraryRequest);
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capability_snapshot_agrees_with_unsupported_dispatch_by_scope() -> Result<()> {
    use crate::messages::{ListLocalLibrariesRequest, PauseRequest, ScanAudioLibraryRequest};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let params = &fixture.global_params;

    // Node scope: ListLocalLibrariesRequest is supported by Local implementation
    assert_eq!(
        RequestCapabilities::from_implementation(&Implementation::Local).list_local_libraries_request,
        RequestCapability::Available
    );
    let node_response = super::step(
        "node scope supported request",
        Implementation::Local.run(params, &ListLocalLibrariesRequest { call_id: None }),
    )
    .await
    .context("node request should succeed")?;
    assert!(node_response.is_some());
    let result = dispatch_gui_request(params, &ListLocalLibrariesRequest { call_id: None })
        .await;
    assert!(result.is_ok());

    // Session scope: PauseRequest is unsupported by Subsonic without jukebox role
    let (endpoint, _, _, _, _, server) = super::step(
        "start subsonic mock",
        subsonic_hub_mock(false),
    )
    .await;
    let subsonic_library_response = super::step(
        "register subsonic library",
        register_subsonic_library_on_node(
            &RegisterSubsonicLibraryRequest {
                endpoint,
                username: "demo".to_owned(),
                password: "demo".to_owned(),
                alias: "Subsonic library".to_owned(),
                mode: Some(SubsonicOpenModeMessage::PlayOnThisDevice),
                call_id: None,
            },
            &fixture.global_params.node,
        ),
    )
    .await
    .context("register subsonic library")?;
    let library_id = subsonic_library_response
        .record
        .context("subsonic library registration should return a record")?
        .record_id
        .parse()
        .context("record_id should be a valid UUID")?;
    let service_binding = super::step(
        "get subsonic service binding",
        service_binding_for_library(&fixture.global_params.node, library_id),
    )
    .await
    .context("get subsonic service binding")?;
    let subsonic_implementation = Implementation::Service(Arc::clone(&service_binding));

    // Subsonic without jukebox role: PauseRequest is AccountRoleMissing
    let capabilities = RequestCapabilities::from_implementation(&subsonic_implementation);
    assert_eq!(
        capabilities.pause_request,
        RequestCapability::Disabled {
            reason: CapabilityReason::AccountRoleMissing,
        }
    );
    let session_error = super::step(
        "session scope unsupported request",
        subsonic_implementation.run(params, &PauseRequest { call_id: None }),
    )
    .await
    .err()
    .context("PauseRequest should fail on Subsonic without jukebox")?;
    assert!(matches!(
        session_error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            request: "PauseRequest",
            reason: UnsupportedReason::AccountRoleMissing,
        })
    ));
    // Verify dispatch_gui_request still answers for a successful request after the failure
    let result = dispatch_gui_request(params, &ListLocalLibrariesRequest { call_id: None })
        .await;
    assert!(result.is_ok());

    // Library scope: ScanAudioLibraryRequest is a library request unsupported by Subsonic
    // Subsonic does not translate library requests it does not handle, leaving them NotImplemented
    assert_eq!(
        capabilities.scan_audio_library_request,
        RequestCapability::Hidden
    );
    let library_error = super::step(
        "library scope unsupported request",
        subsonic_implementation.run(params, &ScanAudioLibraryRequest { force: false, call_id: None }),
    )
    .await
    .err()
    .context("ScanAudioLibraryRequest should fail on Subsonic")?;
    assert!(matches!(
        library_error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            request: "ScanAudioLibraryRequest",
            reason: UnsupportedReason::NotImplemented,
        })
    ));
    // Verify dispatch_gui_request still answers for a successful request after the failure
    let result = dispatch_gui_request(params, &ListLocalLibrariesRequest { call_id: None })
        .await;
    assert!(result.is_ok());

    server.abort();
    super::step(
        "shutdown fixture",
        fixture.global_params.node.shutdown(),
    )
    .await?;
    Ok(())
}

#[test]
fn a_playback_mode_the_wire_does_not_know_is_refused() {
    assert!(serde_json::from_str::<WirePlaybackMode>("\"Bogus\"").is_err());
    assert!(serde_json::from_str::<SetPlaybackModeRequest>(r#"{"mode":99}"#).is_err());
    assert!(serde_json::from_str::<SetPlaybackModeRequest>("{}").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_load_keeps_the_playback_mode_unless_it_names_one_and_follows_play() -> Result<()> {
    let node_dir = tempfile::tempdir()?;
    let fixture = fixture(node_dir.path()).await?;
    let first = session_audio(&fixture, "load-mode-a.ogg").await?;
    let second = session_audio(&fixture, "load-mode-b.ogg").await?;
    let items: Vec<_> = vec![
        library_track_for_id(&fixture, first).await?.into(),
        library_track_for_id(&fixture, second).await?.into(),
    ];
    dispatch_gui_request(
        &fixture.global_params,
        &SetPlaybackModeRequest {
            call_id: None,
            mode: WirePlaybackMode::RepeatAll,
        },
    )
    .await?;
    wait_player(&fixture, |status| {
        status.playback_mode == PlaybackMode::RepeatAll
    })
    .await?;
    let load = |playback_mode, position_seconds, play| LoadResolvedItemsRequest {
        call_id: None,
        items: items.clone(),
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode,
        position_seconds,
        play,
    };

    // No mode named: the player's own mode stays. Pause: the track is loaded but not started.
    dispatch_gui_request(&fixture.global_params, &load(None, None, LoadPlay::Pause)).await?;
    wait_player(&fixture, |status| {
        status.playlist.len() == 2
            && status.state == PlayerPlaybackState::Paused
            && status.playback_mode == PlaybackMode::RepeatAll
    })
    .await?;

    // Play: started, and the mode is still the player's.
    dispatch_gui_request(&fixture.global_params, &load(None, None, LoadPlay::Play)).await?;
    wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing
            && status.playback_mode == PlaybackMode::RepeatAll
    })
    .await?;

    // Pause a playing player through a load, naming a mode: the mode changes, the track pauses.
    dispatch_gui_request(
        &fixture.global_params,
        &load(Some(WirePlaybackMode::Shuffle), None, LoadPlay::Pause),
    )
    .await?;
    wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Paused && status.playback_mode == PlaybackMode::Shuffle
    })
    .await?;

    // Auto with a position and no mode stays paused, and without a position it plays.
    dispatch_gui_request(
        &fixture.global_params,
        &load(None, Some(1.0), LoadPlay::Auto),
    )
    .await?;
    wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Paused && status.playback_mode == PlaybackMode::Shuffle
    })
    .await?;
    dispatch_gui_request(&fixture.global_params, &load(None, None, LoadPlay::Auto)).await?;
    wait_player(&fixture, |status| {
        status.state == PlayerPlaybackState::Playing
    })
    .await?;

    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn created_mixes_keep_the_playback_mode_they_were_given() -> Result<()> {
    use crate::messages::{CreateMixRequest, FetchAllMixesRequest, UpdateMixRequest};

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let params = &fixture.global_params;
    let queries = || {
        vec![MixQuery {
            operator: "lib::all".into(),
            parameter: "true".into(),
        }]
    };
    let mut created = Vec::new();
    for (name, mode) in [
        ("default mode", None),
        ("repeat all", Some(WirePlaybackMode::RepeatAll)),
    ] {
        let response = dispatch_gui_request(
            params,
            &CreateMixRequest {
                name: name.into(),
                group: "modes".into(),
                mode,
                queries: queries(),
                call_id: None,
            },
        )
        .await?
        .context("created mix")?;
        assert_eq!(response.mix.mode, mode);
        created.push((response.mix.id, mode));
    }
    let updated = dispatch_gui_request(
        params,
        &UpdateMixRequest {
            mix_id: created[1].0,
            name: "repeat all".into(),
            group: "modes".into(),
            mode: None,
            queries: queries(),
            call_id: None,
        },
    )
    .await?
    .context("updated mix")?;
    assert_eq!(updated.mix.mode, None);
    created[1].1 = None;
    let all = dispatch_gui_request(params, &FetchAllMixesRequest { call_id: None })
        .await?
        .context("all mixes")?;
    for (id, mode) in created {
        let mix = all.mixes.iter().find(|mix| mix.id == id).context("mix")?;
        assert_eq!(mix.mode, mode);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mix_without_a_stored_mode_keeps_the_players_mode_instead_of_failing_requests()
-> Result<()> {
    use crate::messages::FetchAllMixesRequest;
    use database::entities::mixes;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let library = fixture.global_params.test_library();
    let row = mixes::Entity::find()
        .one(library.main_db.as_ref())
        .await?
        .context("a built-in mix")?;
    let id = row.id;
    let mut unset: mixes::ActiveModel = row.into();
    unset.mode = Set(None);
    unset.update(library.main_db.as_ref()).await?;
    let all = dispatch_gui_request(
        &fixture.global_params,
        &FetchAllMixesRequest { call_id: None },
    )
    .await?
    .context("all mixes")?;
    let mix = all.mixes.iter().find(|mix| mix.id == id).context("mix")?;
    assert_eq!(mix.mode, None);
    Ok(())
}

/// What a request that did not do its work answers: a typed failure, never a response carrying an
/// error text.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_settings_and_scene_requests_answer_typed_failures() -> Result<()> {
    use crate::messages::{
        ImportVisualizerSceneRequest, RemoveVisualizerSceneRequest, SetDownloadRuleRequest,
    };
    use crate::request_error::classify;

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let params = &fixture.global_params;

    let rule = |mix_id, queries: Vec<MixQuery>| SetDownloadRuleRequest {
        request_id: "rule".into(),
        queries,
        mix_id,
        enabled: true,
        delete_unselected_files: false,
        call_id: None,
    };
    for (request, kind) in [
        (rule(None, Vec::new()), RequestErrorKind::InvalidInput),
        (rule(Some(9999), Vec::new()), RequestErrorKind::NotFound),
    ] {
        let error = request
            .handle(request.extract_params(params), None, &request)
            .await
            .err()
        .expect("an offline rule that selects nothing or a missing mix");
        assert_eq!(classify(&error), kind, "{error:#}");
    }

    let scenes = tempfile::tempdir()?;
    let import = ImportVisualizerSceneRequest {
        file: scenes.path().join("missing.rune-scene").to_string_lossy().into(),
        dir: scenes.path().to_string_lossy().into(),
        call_id: None,
    };
    let error = import
        .handle(import.extract_params(params), None, &import)
        .await
        .err()
        .expect("a file that is not there");
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput, "{error:#}");
    let remove = RemoveVisualizerSceneRequest {
        path: scenes.path().join("not-a-scene").to_string_lossy().into(),
        dir: scenes.path().to_string_lossy().into(),
        call_id: None,
    };
    remove
        .handle(remove.extract_params(params), None, &remove)
        .await
        .err()
        .expect("a scene that is not there");

    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_scrobble_login_answers_its_typed_failure() -> Result<()> {
    use crate::messages::{
        AuthenticateSingleServiceRequest, LastFmApiCredentialField, LoginRequestItem,
        ScrobbleApiCredentials, ScrobbleAuthenticationFailure, ScrobbleSecretInput,
    };
    use crate::request_error::{classify, request_failure_cause};

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let request = AuthenticateSingleServiceRequest {
        request: Some(LoginRequestItem {
            service_id: "LastFm".into(),
            username: "listener".into(),
            secret_input: ScrobbleSecretInput::Password {
                password: "password".into(),
                api_credentials: ScrobbleApiCredentials::LastFm {
                    api_key: String::new(),
                    api_secret: String::new(),
                },
                replaces: None,
            },
        }),
        call_id: None,
    };
    let error = request
        .handle(
            request.extract_params(&fixture.global_params),
            None,
            &request,
        )
        .await
        .err()
        .expect("a Last.fm login without its API credentials");
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput, "{error:#}");
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ScrobbleAuthentication {
            failure: ScrobbleAuthenticationFailure::LastFmApiCredentialsMissing {
                field: LastFmApiCredentialField::ApiKey,
            },
        })
    );
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

/// Plays every track of `library_id` through the platform host `"host"`, as a streaming service
/// that declares a host for its tracks does.
struct HostedTracks {
    library_id: Uuid,
}

impl player::PlaybackSourceResolver for HostedTracks {
    fn resolve_track(
        &self,
        library_id: String,
        track: String,
    ) -> futures::future::BoxFuture<'static, Result<player::LibraryTrackKey>> {
        let hosted = library_id == self.library_id.to_string();
        Box::pin(async move {
            anyhow::ensure!(hosted, "library {library_id} has no hosted tracks");
            Ok(player::LibraryTrackKey::Service(track))
        })
    }

    fn resolve(
        &self,
        _: i32,
    ) -> futures::future::BoxFuture<'static, Result<player::RemoteAudioSource>> {
        Box::pin(async { anyhow::bail!("hosted tracks have no stream") })
    }

    fn resolve_service(
        &self,
        item: ServiceItem,
    ) -> futures::future::BoxFuture<'static, Result<player::ServiceTarget>> {
        Box::pin(async move {
            Ok(player::ServiceTarget::Hosted(
                playback::hosted::HostedTrack {
                    host: "host".to_owned(),
                    key: item.track,
                },
            ))
        })
    }
}

/// A platform host that records the batches the player sends it.
#[derive(Default)]
struct RecordingHost(StdMutex<Vec<playback::hosted::HostCommand>>);

impl playback::hosted::HostBridge for RecordingHost {
    fn send(&self, command: playback::hosted::HostCommand) {
        self.0.lock().unwrap().push(command);
    }
}

impl RecordingHost {
    /// Waits for a batch that `wanted` picks, and returns its revision.
    async fn sent(&self, wanted: impl Fn(&[playback::hosted::HostAction]) -> bool) -> Result<u64> {
        let wanted = &wanted;
        wait_until("the host to get the expected batch", || async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|command| command.host == "host" && wanted(&command.actions))
                .map(|command| command.revision))
        })
        .await
    }
}

/// A node whose own player hands the tracks of a streaming library to a recording host. The
/// session plays what the node's UI asks for, and the node tells the UI what it shows.
async fn hosted_fixture(
    dir: &Path,
) -> Result<(Fixture, Arc<RecordedSignals>, Arc<RecordingHost>, Uuid)> {
    let signals = Arc::new(RecordedSignals::default());
    let f = fixture_with_broadcaster(
        dir,
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let host = Arc::new(RecordingHost::default());
    let library_id = Uuid::new_v4();
    let player = f.global_params.player.lock().await;
    player.set_source_resolver(Arc::new(HostedTracks { library_id }));
    player.set_host_bridge(host.clone());
    drop(player);
    Ok((f, signals, host, library_id))
}

/// Waits until the latest signal named `name` is one that `wanted` picks.
async fn latest_signal<T: DeserializeOwned>(
    signals: &RecordedSignals,
    name: &str,
    wanted: impl Fn(&T) -> bool,
) -> Result<()> {
    let wanted = &wanted;
    wait_until("the UI to be told the expected state", || async move {
        Ok(signals
            .decoded::<T>(name)
            .last()
            .is_some_and(wanted)
            .then_some(()))
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hosted_track_plays_on_its_host_and_the_next_file_plays_here() -> Result<()> {
    use crate::messages::{
        PauseRequest, PlayRequest, PlayerHostChanged, SeekRequest, VisualizerInputsChanged,
    };
    use playback::hosted::{HostAction, HostEvent};

    let dir = tempfile::tempdir()?;
    let (f, signals, host, library_id) = hosted_fixture(dir.path()).await?;
    let file = dir.path().join("after-the-host.ogg");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        &file,
    )?;
    let hosted = PlayingItem::LibraryTrack {
        library_id,
        track: "1440".to_owned(),
    };
    let after = PlayingItem::IndependentFile(file.to_string_lossy().into_owned());
    f.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![
                (hosted.clone(), PlaybackSource::LibraryStream),
                (after.clone(), PlaybackSource::Local(file.clone())),
            ],
            AddMode::AppendToEnd,
        );
    wait_player(&f, |status| status.playlist.len() == 2).await?;

    // The host gets the track to play, and the UI's volume control and visualizer follow it.
    dispatch_gui_request(
        &f.global_params,
        &SwitchRequest {
            index: 0,
            expected_revision: None,
            call_id: None,
        },
    )
    .await?;
    let handed = host
        .sent(|actions| {
            matches!(actions.first(), Some(HostAction::Set { ids }) if ids == &["1440"])
                && actions.last() == Some(&HostAction::Start)
        })
        .await?;
    wait_player(&f, |status| {
        status.item.as_ref() == Some(&hosted)
            && status.state == PlayerPlaybackState::Playing
            && status.hosted_by.as_deref() == Some("host")
    })
    .await?;
    latest_signal(
        &signals,
        "PlayerHostChanged",
        |changed: &PlayerHostChanged| changed.hosted,
    )
    .await?;
    latest_signal(
        &signals,
        "VisualizerInputsChanged",
        |changed: &VisualizerInputsChanged| {
            changed.inputs == crate::messages::VisualizerInputs::None
        },
    )
    .await?;

    // What the host reports is what the UI shows.
    let report = format!(
        r#"{{"type":"jukeboxStatus","host":"host","revision":{handed},"currentIndex":0,"currentId":"1440","playing":true,"positionMs":30000}}"#
    );
    f.global_params
        .player
        .lock()
        .await
        .deliver_host_event(HostEvent::parse(&report).context("a host status")?);
    latest_signal(&signals, "PlaybackStatus", |status: &PlaybackStatus| {
        status.state == PlaybackState::Playing && status.progress_seconds >= 30.0
    })
    .await?;

    // Pause, seek and play go to the host.
    dispatch_gui_request(&f.global_params, &PauseRequest { call_id: None }).await?;
    host.sent(|actions| actions == [HostAction::Stop]).await?;
    wait_player(&f, |status| status.state == PlayerPlaybackState::Paused).await?;
    dispatch_gui_request(
        &f.global_params,
        &SeekRequest {
            position_seconds: 42.5,
            call_id: None,
        },
    )
    .await?;
    host.sent(|actions| {
        actions
            == [HostAction::Skip {
                index: 0,
                offset_ms: 42_500,
            }]
    })
    .await?;
    dispatch_gui_request(&f.global_params, &PlayRequest { call_id: None }).await?;
    host.sent(|actions| actions == [HostAction::Start]).await?;

    // The host's track ends, and the file after it plays here: the host lets go, and the volume
    // control and the visualizer come back to this device's player.
    let ended = format!(r#"{{"type":"jukeboxEnded","host":"host","revision":{handed}}}"#);
    f.global_params
        .player
        .lock()
        .await
        .deliver_host_event(HostEvent::parse(&ended).context("a host end")?);
    wait_player(&f, |status| {
        status.item.as_ref() == Some(&after)
            && status.index == Some(1)
            && status.state == PlayerPlaybackState::Playing
            && status.hosted_by.is_none()
    })
    .await?;
    host.sent(|actions| actions == [HostAction::Stop, HostAction::Clear])
        .await?;
    latest_signal(
        &signals,
        "PlayerHostChanged",
        |changed: &PlayerHostChanged| !changed.hosted,
    )
    .await?;
    latest_signal(
        &signals,
        "VisualizerInputsChanged",
        |changed: &VisualizerInputsChanged| {
            changed.inputs == crate::messages::VisualizerInputs::Full
        },
    )
    .await?;

    f.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_volume_control_leaves_the_host_while_another_player_is_current() -> Result<()> {
    use crate::messages::PlayerHostChanged;
    use playback::hosted::HostAction;

    let dir = tempfile::tempdir()?;
    let (f, signals, host, library_id) = hosted_fixture(dir.path()).await?;
    let hosted = PlayingItem::LibraryTrack {
        library_id,
        track: "1440".to_owned(),
    };
    f.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(hosted, PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    wait_player(&f, |status| status.playlist.len() == 1).await?;
    f.global_params.player.lock().await.switch(0);
    host.sent(|actions| actions.last() == Some(&HostAction::Start))
        .await?;
    latest_signal(
        &signals,
        "PlayerHostChanged",
        |changed: &PlayerHostChanged| changed.hosted,
    )
    .await?;

    // Another player takes over: this device's player pauses its host, and the volume control
    // is no longer the system's, whatever this device's player holds.
    let node = f.global_params.node.clone();
    let away = CurrentPlayer::RunePeer {
        link: None,
        reconnecting: None,
    };
    node.transition_current_player(CurrentPlayer::Local, away.clone())
        .await?;
    latest_signal(
        &signals,
        "PlayerHostChanged",
        |changed: &PlayerHostChanged| !changed.hosted,
    )
    .await?;
    host.sent(|actions| actions == [HostAction::Stop]).await?;

    // Back on this device, the paused hosted track goes to its host on the next play, so the
    // volume control is the system's again.
    node.transition_current_player(away, CurrentPlayer::Local)
        .await?;
    latest_signal(
        &signals,
        "PlayerHostChanged",
        |changed: &PlayerHostChanged| changed.hosted,
    )
    .await?;

    node.shutdown().await?;
    Ok(())
}

// Test for issue #424: A stale queue can remove the wrong track even with the current revision.
// A tracking broadcaster keeps the items every published playlist revision described and fails
// any update whose items do not match its revision: the revision a snapshot goes out with must
// be the revision of the queue state the snapshot shows.
struct QueueRevisionAudit {
    recorded: RecordedSignals,
    // The item ids each revision was first published with.
    revisions: StdMutex<HashMap<u64, Vec<String>>>,
    violations: StdMutex<Vec<String>>,
}

impl QueueRevisionAudit {
    fn new() -> Self {
        Self {
            recorded: RecordedSignals::default(),
            revisions: StdMutex::new(HashMap::new()),
            violations: StdMutex::new(Vec::new()),
        }
    }

    fn violations(&self) -> Vec<String> {
        self.violations.lock().unwrap().clone()
    }
}

impl Broadcaster for QueueRevisionAudit {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        self.recorded.broadcast(message);
        if message.name() != "PlaylistUpdate" {
            return;
        }
        let Ok(update) =
            rinf::deserialize::<PlaylistUpdate>(&message.encode_to_vec().unwrap())
        else {
            return;
        };
        let items = update
            .items
            .iter()
            .map(|entry| match &entry.item {
                PlayingItemRef::LibraryTrack { track, .. } => track.clone(),
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>();
        let mut revisions = self.revisions.lock().unwrap();
        match revisions.get(&update.revision) {
            Some(known) if *known != items => {
                self.violations.lock().unwrap().push(format!(
                    "revision {} was published as {known:?} and again as {items:?}",
                    update.revision
                ));
            }
            None => {
                revisions.insert(update.revision, items);
            }
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn stale_queue_snapshot_does_not_remove_wrong_track() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let audit = Arc::new(QueueRevisionAudit::new());
    let fixture = fixture_with_broadcaster(dir.path(), Uuid::new_v4(), Duration::from_secs(300), audit.clone()).await?;

    let ids = [
        session_audio(&fixture, "audit-rev-a.ogg").await?,
        session_audio(&fixture, "audit-rev-b.ogg").await?,
        session_audio(&fixture, "audit-rev-c.ogg").await?,
    ];
    let items = future::join_all(ids.iter().map(|id| library_track_for_id(&fixture, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

    // The queue must be populated through the session, as the UI loads it: a queue edit is what
    // moves the revision, and edits outside it never produce one.
    let revision = dispatch_gui_request(
        &fixture.global_params,
        &PlaylistRevisionRequest { call_id: None },
    )
    .await?
    .context("initial queue revision")?
    .revision;
    assert_eq!(revision, 0);
    dispatch_gui_request(
        &fixture.global_params,
        &LoadResolvedItemsRequest {
            call_id: None,
            items: items.iter().map(|item| PlayingItemRef::from(*item)).collect(),
            start_index: None,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(0),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Pause,
        },
    )
    .await?;
    wait_player(&fixture, |status| status.playlist.len() == 3).await?;

    let uuid_of = async |id: i32| -> Result<String> {
        Ok(file_uuids_by_id(&fixture.global_params.test_library().main_db, &[id])
            .await?
            .get(&id)
            .copied()
            .context("file uuid")?
            .to_string())
    };
    let ua = uuid_of(ids[0]).await?;
    let ub = uuid_of(ids[1]).await?;
    let uc = uuid_of(ids[2]).await?;

    // The loaded queue is published under revision 1.
    wait_until("initial playlist delivery", || async {
        Ok(audit
            .recorded
            .decoded::<PlaylistUpdate>("PlaylistUpdate")
            .iter()
            .any(|p| p.items.len() == 3 && p.revision == 1)
            .then_some(()))
    })
    .await?;

    // Remove A: the queue becomes [B, C] under revision 2.
    dispatch_gui_request(
        &fixture.global_params,
        &RemoveRequest { call_id: Some(6010), index: 0, expected_revision: Some(1) },
    )
    .await?;
    // The debounced publication is the edit settling: it and the queue it shows arrive
    // together, where waiting on the player's status races its lock against the report.
    wait_until("the removal to settle", || async {
        Ok(audit
            .revisions
            .lock()
            .unwrap()
            .get(&2)
            .is_some_and(|items| items.len() == 2)
            .then_some(()))
    })
    .await?;

    // A track becomes unavailable between the two edits. Its report is a playlist snapshot of
    // its own, and it must not go out under the revision of the edit that follows it.
    let media_root = fixture.global_params.test_library().media_root.clone();
    fs::remove_file(Path::new(&media_root).join("audit-rev-c.ogg"))?;
    fixture.global_params.player.lock().await.switch(1);
    // Move B to the end immediately, so the edit and the in-flight unavailable report reach
    // the session together: the queue becomes [C, B] under revision 3, and the unavailable
    // snapshot must not go out under it.
    dispatch_gui_request(
        &fixture.global_params,
        &MovePlaylistItemRequest { call_id: Some(6011), old_index: 0, new_index: 1, expected_revision: Some(2) },
    )
    .await?;
    wait_until("unavailable report", || async {
        Ok(audit
            .recorded
            .decoded::<PlaylistUpdate>("PlaylistUpdate")
            .iter()
            .any(|p| {
                p.items
                    .iter()
                    .any(|entry| entry.unavailable.is_some())
            })
            .then_some(()))
    })
    .await?;
    wait_until("the move to settle", || async {
        let revisions = audit.revisions.lock().unwrap();
        Ok(revisions
            .get(&3)
            .is_some_and(|items| items.first().map(String::as_str) == Some(uc.as_str()))
            .then_some(()))
    })
    .await?;

    // Every published snapshot pairs its items with the revision of the queue state they are:
    // the empty queue with 0, [A, B, C] with 1, [B, C] with 2, [C, B] with 3, whichever of them
    // debouncing publishes.
    assert!(audit.revisions.lock().unwrap().contains_key(&3));
    assert!(
        audit.violations().is_empty(),
        "snapshots went out under revisions they do not belong to: {:?}",
        audit.violations()
    );
    {
        let revisions = audit.revisions.lock().unwrap();
        if let Some(items) = revisions.get(&0) {
            assert!(items.is_empty());
        }
        if let Some(items) = revisions.get(&1) {
            assert_eq!(*items, vec![ua.clone(), ub.clone(), uc.clone()]);
        }
        if let Some(items) = revisions.get(&2) {
            assert_eq!(*items, vec![ub.clone(), uc.clone()]);
        }
        if let Some(items) = revisions.get(&3) {
            assert_eq!(*items, vec![uc.clone(), ub.clone()]);
        }
    }

    // An edit against the revision of the queue before the move is stale now.
    let result = dispatch_gui_request(
        &fixture.global_params,
        &RemoveRequest { call_id: Some(6012), index: 0, expected_revision: Some(2) },
    )
    .await;
    assert!(result.is_err(), "Expected error when removing with stale revision");
    let error = result.unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::Conflict);

    // The queue is untouched: revision 3's [C, B] did not remove C.
    let revisions = audit.revisions.lock().unwrap().clone();
    assert_eq!(
        revisions.get(&3).map(Vec::as_slice),
        Some([uc.clone(), ub.clone()].as_slice())
    );

    fixture.global_params.node.shutdown().await?;
    Ok(())
}

/// A restart must restore the queue under the revision it persisted with: the restore is no
/// edit and bumps nothing, but its snapshots pair with the revision edits continue from, or
/// the first Unavailable report after it goes out under a revision the queue never had.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_restores_the_queue_under_its_persisted_revision() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let id = session_audio(&fixture, "restart-revision.ogg").await?;
    let item = library_track_for_id(&fixture, id).await?;

    // One edit through the session: the queue and the persisted revision are 1.
    dispatch_gui_request(
        &fixture.global_params,
        &LoadResolvedItemsRequest {
            call_id: None,
            items: vec![PlayingItemRef::from(item)],
            start_index: None,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(0),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Pause,
        },
    )
    .await?;
    wait_player(&fixture, |status| status.playlist.len() == 1).await?;
    let config_path = fixture.global_params.config_path.to_string();
    fixture.global_params.node.shutdown().await?;

    // The node comes back with its signals recorded, so the restored playlist's revision can
    // be read against the session's.
    let signals = Arc::new(RecordedSignals::default());
    let node = NodeContext::new(
        &config_path,
        &config_path,
        signals.clone(),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    let session_revision = node.session()?.queue_revision().await;
    assert_eq!(session_revision, 1, "the persisted revision survived the restart");

    let restored = wait_until("the restored playlist publication", || async {
        Ok(signals
            .decoded::<PlaylistUpdate>("PlaylistUpdate")
            .into_iter()
            .find(|update| update.items.len() == 1))
    })
    .await?;
    assert_eq!(
        restored.revision, session_revision,
        "the restored queue went out under revision {} while the session continues from {}",
        restored.revision, session_revision
    );
    node.shutdown().await?;
    Ok(())
}
