// Browsing an open Apple Music library through the requests the UI sends, against a fake
// Apple Music API.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use database::library_registry::{LibraryRecordId, ServiceKind};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::apple_music_binding::{AppleIdKind, AppleMusicBinding, CONTENT_CHANGED_DELAY, Tokens};
use crate::macros::{RequestSlots, capability_snapshot};
use crate::messages::{
    CallerQueue, CapabilityReason, CollectionType, ComplexQuery, ComplexQueryRequest,
    ExcludedItems, FetchAllPlaylistsRequest, FetchCollectionByIdsRequest,
    FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest, FetchMediaFileByIdsRequest,
    FetchMediaFilesRequest, FetchParsedMediaFileRequest, GetLyricByTrackIdRequest,
    GetLyricByTrackIdResponse, GetMediaFilesCountRequest, GetPrimaryColorByTrackIdRequest,
    LibraryContentChanged, LibraryHomeListMode, LibraryHomeQuery, LibraryTaskState,
    LibraryTaskStateChanged, MixQuery, MixQueryRequest, PlayingItemRef, RequestCapability,
    RequestFailureCause, ResolveMixQueryRequest, SearchCollectionSummaryRequest,
    SearchMediaFileSummaryRequest, SetLikedRequest,
};
use crate::node::NodeContext;
use crate::request_error::{ExpectedRequestError, classify, request_failure_cause};
use crate::rpc::{Slot, TargetRequest, UnsupportedReason, dispatch_gui_request};
use crate::server::http::websocket::withheld_library_event;
use crate::service::ServiceBinding;
use crate::service::covers::cover_cache_root;
use crate::service::folder::delete_service_library_folder;
use crate::service::lists::lists_folder;
use crate::service::reach::Reach;
use crate::service::registry::{Clock, ServiceSetup, SystemClock};
use crate::test_wait::wait_until;

const USER_TOKEN: &str = "browse-user-token";
const DEVELOPER_TOKEN: &str = "browse-developer-token";
const APPLE_OTHER_TOKEN: &str = "browse-other-user-token";

/// What the fake Apple API saw and how it should answer.
struct AppleFake {
    base: String,
    /// Every request, as path and query, with whether it carried Apple credentials.
    seen: Mutex<Vec<(String, bool)>>,
    /// Answers 401 to every request except cover downloads.
    reject_everything: AtomicBool,
    /// Answers 401 to the next request that is not the storefront read.
    reject_next_read: AtomicBool,
    /// Answers every read of this path with this status and body, though the sign-in is fine.
    respond_path: Mutex<Option<(String, u16, String)>>,
    /// Holds the answer to every request whose path starts with this.
    delay_path: Mutex<Option<(String, std::time::Duration)>>,
    /// Lists the library as empty.
    empty: AtomicBool,
    /// Names the second album as the one added last.
    night_added_last: AtomicBool,
    /// The Night Album and its songs have no cover.
    bare_night: AtomicBool,
    more_artists: Mutex<Vec<Value>>,
    /// How many songs the library holds besides the three.
    extra_songs: usize,
    /// Holds the answer to the first read whose path starts with this, once, until the test
    /// releases it.
    hold_once: Mutex<Option<(String, Arc<tokio::sync::Notify>)>>,
    /// Lists "Artist One" credited alone and with "Guest" as the library's artists, and finds
    /// the catalog's "Guest" besides.
    credited: bool,
    /// Lists no song of the library or its playlists, though its albums and artists hold some.
    unlisted_songs: bool,
    /// Lists a song credited "Solo Voice", which has no artist relationship.
    solo: bool,
    /// Lists the songs of `catalog_credited_songs` too.
    catalog_credits: bool,
    /// How many reads of catalog songs are being answered, and the most there were at once.
    catalog_songs_in_flight: AtomicUsize,
    most_catalog_songs_in_flight: AtomicUsize,
    /// Whether the library is that of another account: the same, but every library id has
    /// "b-" after its kind (see `other_account_id`).
    other_account: AtomicBool,
}

/// The id another account's library has for `id`, when `id` is a library id: "i.s1" is
/// "i.b-s1". Catalog ids have none.
fn other_account_id(id: &str) -> Option<String> {
    let (kind, rest) = id.split_once('.')?;
    (matches!(kind, "i" | "l" | "p" | "r") && !rest.is_empty() && !rest.starts_with("b-"))
        .then(|| format!("{kind}.b-{rest}"))
}

/// Gives every library id in `value` as another account's library has it.
fn as_other_account(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(id) = other_account_id(text) {
                *text = id;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(as_other_account),
        Value::Object(fields) => fields.values_mut().for_each(as_other_account),
        _ => {}
    }
}

/// Counts a read being answered for as long as it lives.
struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AppleFake {
    fn count(&self, prefix: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with(prefix))
            .count()
    }

    /// How many requests Apple's API was asked. The covers the binding downloads in the
    /// background come from the same server, and are not what a test counts.
    fn total(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| !target.starts_with("/art/"))
            .count()
    }

    fn artwork(&self, name: &str) -> Value {
        json!({
            "width": 1400,
            "height": 1400,
            "url": format!("{}/art/{}/{{w}}x{{h}}bb.jpg", self.base, name.replace(' ', "_")),
        })
    }

    fn song(
        &self,
        id: &str,
        title: &str,
        album: (&str, &str),
        artist: (&str, &str),
        number: u32,
    ) -> Value {
        let mut song = json!({
            "id": id,
            "type": "library-songs",
            "href": format!("/v1/me/library/songs/{id}"),
            "attributes": {
                "name": title,
                "albumName": album.1,
                "artistName": artist.1,
                "trackNumber": number,
                "discNumber": 1,
                "durationInMillis": 200_000,
                "playParams": {"id": id, "kind": "song", "isLibrary": true, "catalogId": format!("c{id}")},
                "artwork": self.artwork(album.1),
            },
            "relationships": {
                "albums": {"data": [{"id": album.0, "type": "library-albums", "attributes": {"name": album.1}}]},
                "artists": {"data": [{"id": artist.0, "type": "library-artists", "attributes": {"name": artist.1}}]},
            }
        });
        if album.1 == "Night Album" && self.bare_night.load(Ordering::SeqCst) {
            song["attributes"]
                .as_object_mut()
                .expect("attributes")
                .remove("artwork");
        }
        song
    }

    fn album(&self, id: &str, name: &str, artist: (&str, &str)) -> Value {
        let mut album = json!({
            "id": id,
            "type": "library-albums",
            "href": format!("/v1/me/library/albums/{id}"),
            "attributes": {
                "name": name,
                "artistName": artist.1,
                "trackCount": 2,
                "artwork": self.artwork(name),
                "playParams": {"id": id, "kind": "album", "isLibrary": true},
            },
            "relationships": {
                "artists": {"data": [{"id": artist.0, "type": "library-artists", "attributes": {"name": artist.1}}]},
            }
        });
        if name == "Night Album" && self.bare_night.load(Ordering::SeqCst) {
            album["attributes"]
                .as_object_mut()
                .expect("attributes")
                .remove("artwork");
        }
        album
    }

    fn artist(id: &str, name: &str) -> Value {
        json!({
            "id": id,
            "type": "library-artists",
            "href": format!("/v1/me/library/artists/{id}"),
            "attributes": {"name": name},
        })
    }

    /// A library artist as listed with `include=catalog`, with the catalog artists Apple names
    /// for it and their pictures.
    fn credited_artist(&self, id: &str, name: &str, catalog: &[(&str, &str)]) -> Value {
        let mut artist = Self::artist(id, name);
        let catalog: Vec<Value> = catalog
            .iter()
            .map(|(id, name)| {
                json!({"id": id, "type": "artists", "attributes": {
                    "name": name,
                    "artwork": self.artwork(&format!("{name} Photo")),
                }})
            })
            .collect();
        artist["relationships"] = json!({"catalog": {"data": catalog}});
        artist
    }

    /// The library's artists when "Artist One" is credited alone and with "Guest": Apple
    /// names one catalog artist for one credit of two, and both for the other.
    fn credited_artists(&self) -> Vec<Value> {
        vec![
            self.credited_artist("r.art1", "Artist One", &[("300", "Artist One")]),
            self.credited_artist("r.duo", "Artist One & Guest", &[("300", "Artist One")]),
            self.credited_artist(
                "r.feat",
                "Guest, Artist One",
                &[("302", "Guest"), ("300", "Artist One")],
            ),
        ]
    }

    /// The albums and songs of the credits of "Artist One" with "Guest": "Duet" is on both
    /// of their albums, and "Guest, Artist One" lists both albums.
    fn credited_albums(&self) -> [(Value, Vec<Value>); 2] {
        let duo = ("r.duo", "Artist One & Guest");
        let feat = ("r.feat", "Guest, Artist One");
        let duet = self.song("i.d1", "Duet", ("l.duo", "Duo Album"), duo, 1);
        [
            (self.album("l.duo", "Duo Album", duo), vec![duet.clone()]),
            (
                self.album("l.feat", "Guest Album", feat),
                vec![
                    self.song("i.g1", "Guest Spot", ("l.feat", "Guest Album"), feat, 1),
                    duet,
                ],
            ),
        ]
    }

    /// A catalog song the fake knows: "ci.s1" is the catalog copy of the library's "i.s1".
    fn catalog_song(&self, id: &str) -> Option<Value> {
        let (title, album, artist, number) = match id {
            "777" => ("Catalog Tune", "Catalog Album", "Catalog Artist", None),
            "ci.s1" => ("First Light", "Dawn Album", "Artist One", Some(1)),
            "ci.ab" => ("Together", "Shared Album", "Ann & Bob", Some(1)),
            "ci.ja" => ("Kana Song", "Shared Album", "Hiroyuki Sawano", Some(2)),
            "ci.en" => ("English Song", "Shared Album", "Hiroyuki Sawano", Some(3)),
            "ci.rev" => ("Reversed", "Shared Album", "Hiroyuki Sawano & Ann", Some(4)),
            _ => return None,
        };
        // Its catalog artists, as Apple names them with `include=artists`.
        let artists: &[(&str, &str)] = match id {
            "ci.ab" => &[("400", "Ann"), ("401", "Bob")],
            "ci.ja" | "ci.en" => &[("156", "Hiroyuki Sawano")],
            "ci.rev" => &[("156", "Hiroyuki Sawano"), ("400", "Ann")],
            "ci.s1" => &[("300", "Artist One")],
            _ => &[],
        };
        let artists: Vec<Value> = artists
            .iter()
            .map(|(id, name)| json!({"id": id, "type": "artists", "attributes": {"name": name}}))
            .collect();
        let mut attributes = json!({
            "name": title,
            "albumName": album,
            "artistName": artist,
            "durationInMillis": 100_000,
            "playParams": {"id": id, "kind": "song"},
            "artwork": self.artwork(album),
        });
        if let Some(number) = number {
            attributes["trackNumber"] = json!(number);
            attributes["discNumber"] = json!(1);
        }
        Some(json!({
            "id": id,
            "type": "songs",
            "attributes": attributes,
            "relationships": {"artists": {"data": artists}},
        }))
    }

    /// The library's songs whose credits name the same artists otherwise: "Ann & Bob", two
    /// catalog artists; "サワノヒロユキ", "Hiroyuki Sawano" and "Sawano Hiroyuki & Ann", the same
    /// catalog artist in another script and name order; and "サワノヒロユキ & Cara", a song without a
    /// catalog id.
    fn catalog_credited_songs(&self) -> Vec<Value> {
        let album = ("l.alb3", "Shared Album");
        let song = |id: &str, title: &str, credit: &str, number| {
            self.song(id, title, album, (&format!("r.{id}"), credit), number)
        };
        let mut uncatalogued = song("i.nc", "No Catalog", "サワノヒロユキ & Cara", 5);
        uncatalogued["attributes"]["playParams"]
            .as_object_mut()
            .expect("play parameters")
            .remove("catalogId");
        vec![
            song("i.ab", "Together", "Ann & Bob", 1),
            song("i.ja", "Kana Song", "サワノヒロユキ", 2),
            song("i.en", "English Song", "Hiroyuki Sawano", 3),
            song("i.rev", "Reversed", "Sawano Hiroyuki & Ann", 4),
            uncatalogued,
        ]
    }

    /// `songs` as Apple answers a request that does not include their artists.
    fn without_artists(songs: &mut Value) {
        for song in songs["data"].as_array_mut().into_iter().flatten() {
            if let Some(relationships) = song["relationships"].as_object_mut() {
                relationships.remove("artists");
            }
        }
    }

    fn catalog_songs(&self, ids: &[&str]) -> Value {
        json!({"data": ids.iter().filter_map(|id| self.catalog_song(id)).collect::<Vec<_>>()})
    }

    fn songs(&self) -> [Value; 3] {
        [
            self.song(
                "i.s1",
                "First Light",
                ("l.alb1", "Dawn Album"),
                ("r.art1", "Artist One"),
                1,
            ),
            self.song(
                "i.s2",
                "Second Wind",
                ("l.alb1", "Dawn Album"),
                ("r.art1", "Artist One"),
                2,
            ),
            self.song(
                "i.s3",
                "Third Road",
                ("l.alb2", "Night Album"),
                ("r.art2", "Artist Two"),
                1,
            ),
        ]
    }

    fn answer(&self, path: &str, query: &str) -> Option<Value> {
        if self.empty.load(Ordering::SeqCst)
            && matches!(
                path,
                "/v1/me/library/songs" | "/v1/me/library/albums" | "/v1/me/library/artists"
            )
        {
            return Some(json!({"data": []}));
        }
        let [s1, s2, s3] = self.songs();
        let albums = [
            self.album("l.alb1", "Dawn Album", ("r.art1", "Artist One")),
            self.album("l.alb2", "Night Album", ("r.art2", "Artist Two")),
        ];
        Some(match path {
            "/v1/me/storefront" => {
                json!({"data": [{"id": "it", "type": "storefronts", "attributes": {"name": "Italy"}}]})
            }
            // Italy answers in Italian, or in British English when asked.
            "/v1/storefronts/it" => json!({"data": [{
                "id": "it", "type": "storefronts",
                "attributes": {
                    "name": "Italy",
                    "defaultLanguageTag": "it",
                    "supportedLanguageTags": ["it", "en-GB"],
                },
            }]}),
            "/v1/me/library/songs" if self.unlisted_songs && !query.contains("ids=") => {
                json!({"data": []})
            }
            "/v1/me/library/songs" => {
                let mut songs = vec![s1, s2, s3];
                if self.credited {
                    // The songs of the credits "Artist One" shares, one of each album.
                    let [(_, duo), (_, feat)] = self.credited_albums();
                    songs.extend([duo[0].clone(), feat[0].clone()]);
                }
                if self.catalog_credits {
                    songs.extend(self.catalog_credited_songs());
                }
                if self.solo {
                    let mut solo = self.song(
                        "i.solo",
                        "Alone",
                        ("l.alb1", "Dawn Album"),
                        ("", "Solo Voice"),
                        7,
                    );
                    solo["relationships"]
                        .as_object_mut()
                        .expect("relationships")
                        .remove("artists");
                    songs.push(solo);
                }
                songs.extend((0..self.extra_songs).map(|n| {
                    self.song(
                        &format!("i.x{n:04}"),
                        &format!("Extra {n}"),
                        ("l.alb2", "Night Album"),
                        ("r.art2", "Artist Two"),
                        10,
                    )
                }));
                // Songs asked for by id are only those, and "i.s9" is one the list lacks.
                let asked = url::form_urlencoded::parse(query.as_bytes())
                    .find(|(name, _)| name == "ids")
                    .map(|(_, ids)| ids.split(',').map(str::to_owned).collect::<Vec<_>>());
                if let Some(asked) = asked {
                    songs.push(self.song(
                        "i.s9",
                        "Ninth Moon",
                        ("l.alb2", "Night Album"),
                        ("r.art2", "Artist Two"),
                        9,
                    ));
                    songs.retain(|song| asked.iter().any(|id| song["id"] == id.as_str()));
                }
                json!({"data": songs})
            }
            "/v1/catalog/it/songs" => {
                let ids = url::form_urlencoded::parse(query.as_bytes())
                    .find(|(name, _)| name == "ids")
                    .map(|(_, ids)| ids.into_owned())
                    .unwrap_or_default();
                let mut songs = self.catalog_songs(&ids.split(',').collect::<Vec<_>>());
                // Asked to include their artists, Apple names the artists of each song.
                let artists = url::form_urlencoded::parse(query.as_bytes())
                    .any(|(name, value)| name == "include" && value.contains("artists"));
                if !artists {
                    Self::without_artists(&mut songs);
                }
                songs
            }
            "/v1/me/library/albums" => json!({"data": albums}),
            "/v1/me/library/recently-added" if self.night_added_last.load(Ordering::SeqCst) => {
                json!({"data": [albums[1]]})
            }
            "/v1/me/library/recently-added" => json!({"data": []}),
            "/v1/me/library/albums/l.alb1" => json!({"data": [albums[0]]}),
            "/v1/me/library/albums/l.alb2" => json!({"data": [albums[1]]}),
            "/v1/me/library/albums/l.alb1/tracks" => json!({"data": [s1, s2]}),
            "/v1/me/library/albums/l.alb2/tracks" => json!({"data": [s3]}),
            "/v1/me/library/artists" if self.credited => json!({"data": self.credited_artists()}),
            "/v1/me/library/artists" => {
                let mut artists = vec![
                    Self::artist("r.art1", "Artist One"),
                    Self::artist("r.art2", "Artist Two"),
                ];
                artists.extend(self.more_artists.lock().unwrap().iter().cloned());
                json!({"data": artists})
            }
            "/v1/me/library/artists/r.duo" => {
                json!({"data": [Self::artist("r.duo", "Artist One & Guest")]})
            }
            "/v1/me/library/artists/r.feat" => {
                json!({"data": [Self::artist("r.feat", "Guest, Artist One")]})
            }
            "/v1/me/library/artists/r.duo/albums" => {
                let [(duo, _), _] = self.credited_albums();
                json!({"data": [duo]})
            }
            "/v1/me/library/artists/r.feat/albums" => {
                let [(duo, _), (feat, _)] = self.credited_albums();
                json!({"data": [feat, duo]})
            }
            credited
                if credited.starts_with("/v1/me/library/albums/l.duo")
                    || credited.starts_with("/v1/me/library/albums/l.feat") =>
            {
                let [duo, feat] = self.credited_albums();
                let (album, songs) = if credited.contains("l.duo") {
                    duo
                } else {
                    feat
                };
                if credited.ends_with("/tracks") {
                    json!({"data": songs})
                } else {
                    json!({"data": [album]})
                }
            }
            // Asked with `include=catalog`, Apple names the catalog artist of the library's.
            "/v1/me/library/artists/r.art1" => {
                let mut artist = Self::artist("r.art1", "Artist One");
                artist["relationships"] =
                    json!({"catalog": {"data": [{"id": "300", "type": "artists"}]}});
                json!({"data": [artist]})
            }
            "/v1/me/library/artists/r.art2" => {
                let mut artist = Self::artist("r.art2", "Artist Two");
                artist["relationships"] = json!({"catalog": {"data": []}});
                json!({"data": [artist]})
            }
            "/v1/me/library/artists/r.art1/albums" => json!({"data": [albums[0]]}),
            // The song list holds the songs of the library's playlists too; it has none.
            "/v1/me/library/playlists" => json!({"data": []}),
            _ => return None,
        })
    }
}

async fn apple_fake(
    State(fake): State<Arc<AppleFake>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let authed = headers.contains_key("authorization") || headers.contains_key("music-user-token");
    let target = uri
        .path_and_query()
        .map(|target| target.as_str().to_owned())
        .unwrap_or_default();
    fake.seen.lock().unwrap().push((target, authed));
    let path = uri.path();
    let _in_flight = path.starts_with("/v1/catalog/it/songs").then(|| {
        let now = fake.catalog_songs_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        fake.most_catalog_songs_in_flight
            .fetch_max(now, Ordering::SeqCst);
        InFlight(&fake.catalog_songs_in_flight)
    });
    let hold = {
        let mut hold = fake.hold_once.lock().unwrap();
        if method == Method::GET
            && hold
                .as_ref()
                .is_some_and(|(prefix, _)| path.starts_with(prefix.as_str()))
        {
            hold.take()
        } else {
            None
        }
    };
    if let Some((_, release)) = hold {
        release.notified().await;
    }
    let delay = fake
        .delay_path
        .lock()
        .unwrap()
        .clone()
        .filter(|(prefix, _)| path.starts_with(prefix.as_str()));
    if let Some((_, delay)) = delay {
        tokio::time::sleep(delay).await;
    }
    if path.starts_with("/art/") {
        let size: usize = if path.contains("/Huge/") {
            8 * 1024 * 1024 + 1
        } else {
            11
        };
        let body = if size == 11 {
            b"cover-bytes".to_vec()
        } else {
            vec![7; size]
        };
        return (StatusCode::OK, body).into_response();
    }
    let answered = fake
        .respond_path
        .lock()
        .unwrap()
        .clone()
        .filter(|(answered, _, _)| answered == path);
    if let Some((_, status, body)) = answered {
        return (StatusCode::from_u16(status).unwrap(), body).into_response();
    }
    let rejected = fake.reject_everything.load(Ordering::SeqCst)
        || (path != "/v1/me/storefront" && fake.reject_next_read.swap(false, Ordering::SeqCst));
    if rejected {
        return (
            StatusCode::UNAUTHORIZED,
            r#"{"errors":[{"status":"401","code":"UNAUTHORIZED"}]}"#,
        )
            .into_response();
    }
    // Nothing writes to the Apple account.
    if method != Method::GET {
        return (StatusCode::METHOD_NOT_ALLOWED, "{}").into_response();
    }
    // Another account's library answers with its own ids, and knows only those.
    let other = fake.other_account.load(Ordering::SeqCst);
    let own_id = |text: &str| {
        if other {
            text.replace(".b-", ".")
        } else {
            text.to_owned()
        }
    };
    let query = uri.query().unwrap_or_default();
    let asks_own_ids = other && path.starts_with("/v1/me/") && {
        let ids = url::form_urlencoded::parse(query.as_bytes())
            .filter(|(name, _)| name == "ids")
            .flat_map(|(_, ids)| ids.split(',').map(str::to_owned).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        ids.iter().any(|id| other_account_id(id).is_some())
    };
    let names_own_id = other && path.split('/').any(|part| other_account_id(part).is_some());
    if asks_own_ids || names_own_id {
        return (StatusCode::NOT_FOUND, "{}").into_response();
    }
    match fake.answer(&own_id(path), &own_id(query)) {
        Some(mut body) => {
            if other && path.starts_with("/v1/me/") {
                as_other_account(&mut body);
            }
            (StatusCode::OK, body.to_string()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "{}").into_response(),
    }
}

/// A clock a test moves on by hand, so what the libraries keep ages without waiting.
struct TestClock {
    start: Instant,
    passed: Mutex<std::time::Duration>,
}

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            passed: Mutex::default(),
        })
    }

    /// Moves the clock on by `by`.
    fn advance(&self, by: std::time::Duration) {
        *self.passed.lock().unwrap() += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        Instant::now().max(self.start) + *self.passed.lock().unwrap()
    }
}

/// How a node's services reach the fake at `base` and tell the time by `clock`.
fn fake_services(base: &str, clock: &Arc<TestClock>) -> ServiceSetup {
    ServiceSetup {
        apple_music_host: Some(base.to_owned()),
        clock: Arc::clone(clock) as Arc<dyn Clock>,
    }
}

/// A node on the configuration in `config` whose services reach the fake at `base`, as one
/// started again on it.
async fn node_on(
    config: &str,
    broadcaster: Arc<dyn crate::utils::Broadcaster>,
    base: &str,
) -> Result<Arc<NodeContext>> {
    NodeContext::with_reconnect_config(
        config,
        config,
        broadcaster,
        false,
        RunMode::App,
        ReconnectConfig::default(),
        crate::session::PlayerSession::real_player(),
        Arc::new(::visualizer::Visualizer::new()),
        fake_services(base, &TestClock::new()),
    )
    .await
}

/// An Apple Music library signed in against the fake and open in the UI.
struct OpenApple {
    fixture: Fixture,
    node: Arc<NodeContext>,
    binding: Arc<AppleMusicBinding>,
    fake: Arc<AppleFake>,
    record: LibraryRecordId,
    server: tokio::task::JoinHandle<()>,
    /// The clock what the library keeps ages by.
    clock: Arc<TestClock>,
    /// The UI params the open made. The node keeps them weakly, and in the app the GUI request
    /// listeners keep them alive. In a test run another test's listeners take the shared signal
    /// channels over and end these, so the test holds the params itself.
    ui: Arc<crate::utils::GlobalParams>,
}

async fn open_apple(dir: &Path) -> Result<OpenApple> {
    open_apple_with(dir, AppleOptions::default()).await
}

/// Like `open_apple`, for a library with nothing in it when `empty` is set.
async fn open_apple_listing(dir: &Path, empty: bool) -> Result<OpenApple> {
    open_apple_with(
        dir,
        AppleOptions {
            empty,
            ..AppleOptions::default()
        },
    )
    .await
}

/// How the fake Apple library is set up before it is opened.
#[derive(Default)]
struct AppleOptions {
    empty: bool,
    extra_songs: usize,
    /// What the node's broadcaster is told, if the test looks.
    heard: Option<Arc<Heard>>,
    /// Whether the Night Album and its songs come without a cover.
    bare_night: bool,
    /// Whether "Artist One" is credited with "Guest" too.
    credited: bool,
    /// Whether the song list is empty while the albums and artists still hold songs.
    unlisted_songs: bool,
    /// Whether a song credited "Solo Voice" has no artist relationship.
    solo: bool,
    /// Whether the songs of `catalog_credited_songs` are listed too.
    catalog_credits: bool,
}

async fn open_apple_with(dir: &Path, options: AppleOptions) -> Result<OpenApple> {
    use crate::messages::{RegisterServiceLibraryRequest, ServiceSignIn, StreamingService};
    use crate::utils::register_service_library_on_node;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let clock = TestClock::new();
    let broadcaster: Arc<dyn crate::utils::Broadcaster> = match &options.heard {
        Some(heard) => heard.clone(),
        None => Arc::new(Quiet),
    };
    let fixture =
        super::fixture_with_services(dir, broadcaster, fake_services(&base, &clock)).await?;
    let node = fixture.global_params.node.clone();
    let fake = Arc::new(AppleFake {
        base: base.clone(),
        seen: Mutex::default(),
        reject_everything: AtomicBool::new(false),
        reject_next_read: AtomicBool::new(false),
        respond_path: Mutex::default(),
        delay_path: Mutex::default(),
        empty: AtomicBool::new(options.empty),
        night_added_last: AtomicBool::new(false),
        bare_night: AtomicBool::new(options.bare_night),
        more_artists: Mutex::default(),
        extra_songs: options.extra_songs,
        hold_once: Mutex::default(),
        credited: options.credited,
        unlisted_songs: options.unlisted_songs,
        solo: options.solo,
        catalog_credits: options.catalog_credits,
        catalog_songs_in_flight: AtomicUsize::new(0),
        most_catalog_songs_in_flight: AtomicUsize::new(0),
        other_account: AtomicBool::new(false),
    });
    let app = Router::new().fallback(apple_fake).with_state(fake.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let registered = register_service_library_on_node(
        &RegisterServiceLibraryRequest {
            service: StreamingService::AppleMusic,
            alias: "Apple Music".into(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        &node,
    )
    .await?;
    let record = LibraryRecordId(
        registered
            .record
            .context("registered record")?
            .record_id
            .parse()?,
    );
    let binding = node
        .services
        .apple_music_binding(record)
        .context("signed-in binding")?;
    node.open_service_catalog(Some(record), binding.clone())
        .await?;
    let ui = node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    wait_until("the song list to warm up", || async {
        Ok(binding.known_songs().map(|_| ()))
    })
    .await?;
    Ok(OpenApple {
        fixture,
        node,
        binding,
        fake,
        record,
        server,
        clock,
        ui,
    })
}

impl OpenApple {
    async fn run<R>(&self, request: &R) -> Result<<R as crate::TypedRequest>::Response>
    where
        R: crate::rpc::TargetRequest
            + crate::Signal<
                Response = <R as crate::TypedRequest>::Response,
                Params = <R as crate::utils::ParamsExtractor>::Params,
            > + crate::utils::ParamsExtractor
            + rinf::DartSignal
            + serde::Serialize,
        <R as crate::TypedRequest>::Response: serde::de::DeserializeOwned,
    {
        dispatch_gui_request(&self.fixture.global_params, request)
            .await?
            .context("the request answered nothing")
    }

    async fn finish(self) -> Result<()> {
        self.server.abort();
        self.node.shutdown().await
    }
}

fn unsupported(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported { .. })
    )
}

fn mix(operator: &str, parameter: impl ToString) -> MixQueryRequest {
    MixQueryRequest {
        call_id: None,
        queries: vec![MixQuery {
            operator: operator.into(),
            parameter: parameter.to_string(),
        }],
        cursor: 0,
        page_size: 30,
        bake_cover_arts: false,
        queue: CallerQueue::default(),
        random_seed: None,
    }
}

#[test]
fn apple_music_slots_translate_browsing_and_leave_the_rest_unsupported() {
    let slots = RequestSlots::apple_music();
    for slot in [
        FetchMediaFilesRequest::slot(&slots),
        FetchMediaFileByIdsRequest::slot(&slots),
        FetchParsedMediaFileRequest::slot(&slots),
        SearchMediaFileSummaryRequest::slot(&slots),
        GetMediaFilesCountRequest::slot(&slots),
        FetchCollectionGroupSummaryRequest::slot(&slots),
        FetchCollectionGroupsRequest::slot(&slots),
        FetchCollectionByIdsRequest::slot(&slots),
        SearchCollectionSummaryRequest::slot(&slots),
        MixQueryRequest::slot(&slots),
        ResolveMixQueryRequest::slot(&slots),
        ComplexQueryRequest::slot(&slots),
    ] {
        assert_eq!(slot, Slot::Translate);
    }
    // What is asked about one item is decided by the library of the item, which for an Apple
    // Music item is only its color (see `RequestSlots::apple_music_items`).
    for slot in [
        GetPrimaryColorByTrackIdRequest::slot(&slots),
        GetLyricByTrackIdRequest::slot(&slots),
        SetLikedRequest::slot(&slots),
        crate::messages::GetLikedRequest::slot(&slots),
    ] {
        assert_eq!(slot, Slot::Native);
    }
    let items = RequestSlots::apple_music_items();
    assert_eq!(GetPrimaryColorByTrackIdRequest::slot(&items), Slot::Native);
    let hidden = Slot::Unsupported {
        reason: UnsupportedReason::NotImplemented,
    };
    for slot in [
        GetLyricByTrackIdRequest::slot(&items),
        SetLikedRequest::slot(&items),
        crate::messages::GetLikedRequest::slot(&items),
    ] {
        assert_eq!(slot, hidden);
    }
    // What the library does not hold, its playlists, mixes and search, it does not answer.
    for slot in [
        crate::messages::ScanAudioLibraryRequest::slot(&slots),
        FetchAllPlaylistsRequest::slot(&slots),
        crate::messages::GetPlaylistByIdRequest::slot(&slots),
        crate::messages::FetchPlaylistOccurrencesRequest::slot(&slots),
        crate::messages::CreatePlaylistRequest::slot(&slots),
        crate::messages::AddItemToPlaylistRequest::slot(&slots),
        crate::messages::FetchAllMixesRequest::slot(&slots),
        crate::messages::CreateMixRequest::slot(&slots),
        crate::messages::AddItemToMixRequest::slot(&slots),
        crate::messages::SearchForRequest::slot(&slots),
    ] {
        assert_eq!(slot, hidden);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_capabilities_show_browsing_and_hide_the_rest() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let capabilities = capability_snapshot(&apple.node)?;
    for capability in [
        capabilities.fetch_media_files_request,
        capabilities.fetch_collection_group_summary_request,
        capabilities.mix_query_request,
        capabilities.complex_query_request,
        capabilities.resolve_mix_query_request,
        capabilities.get_primary_color_by_track_id_request,
        capabilities.get_lyric_by_track_id_request,
        capabilities.set_liked_request,
    ] {
        assert_eq!(capability, RequestCapability::Available);
    }
    for capability in [
        capabilities.scan_audio_library_request,
        capabilities.get_analyze_count_request,
        capabilities.fetch_directory_tree_request,
        capabilities.search_for_request,
        capabilities.fetch_all_playlists_request,
        capabilities.fetch_all_mixes_request,
    ] {
        assert_eq!(capability, RequestCapability::Hidden);
    }
    // The library stays on this device, so no other device can be cast to. The snapshot a peer
    // gets does not say so: it is a fact about this device's own UI.
    assert_ne!(
        capabilities.cast_resolved_items_request,
        RequestCapability::Disabled {
            reason: CapabilityReason::DeviceOnly,
        }
    );
    assert_eq!(
        apple.node.own_capabilities()?.cast_resolved_items_request,
        RequestCapability::Disabled {
            reason: CapabilityReason::DeviceOnly,
        }
    );
    apple.finish().await
}

/// What the Tracks page and the summaries say of whether each song plays, in order.
async fn song_availability(apple: &OpenApple) -> Result<Vec<(String, AudioAvailability)>> {
    let page = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 0,
            page_size: 50,
            bake_cover_arts: false,
        })
        .await?;
    let summaries = apple
        .run(&SearchMediaFileSummaryRequest {
            call_id: None,
            n: 50,
        })
        .await?;
    let listed: Vec<_> = page
        .media_files
        .into_iter()
        .map(|file| (file.track, file.audio_availability))
        .collect();
    assert_eq!(
        summaries
            .result
            .iter()
            .map(|summary| summary.audio_availability)
            .collect::<Vec<_>>(),
        listed
            .iter()
            .map(|(_, playable)| *playable)
            .collect::<Vec<_>>()
    );
    Ok(listed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_only_the_songs_stored_on_this_device_play_and_the_lists_reload() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let all = |availability| -> Vec<(String, AudioAvailability)> {
        ["i.s1", "i.s2", "i.s3"]
            .into_iter()
            .map(|track| (track.to_owned(), availability))
            .collect()
    };
    // Online, and before the host says anything, every song plays from Apple.
    assert_eq!(
        song_availability(&apple).await?,
        all(AudioAvailability::RemoteOnly)
    );
    let announced = told_settled(&heard).await;

    // Offline, the downloaded song still plays and the others do not. A library id the
    // library lacks changes nothing for its songs.
    let downloaded = Reach::offline(HashSet::from(["i.s2".to_owned(), "i.other".to_owned()]));
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        downloaded.clone(),
    );
    assert_eq!(
        song_availability(&apple).await?,
        [
            ("i.s1".to_owned(), AudioAvailability::Unavailable),
            ("i.s2".to_owned(), AudioAvailability::RemoteOnly),
            ("i.s3".to_owned(), AudioAvailability::Unavailable),
        ]
    );
    assert_eq!(told_settled(&heard).await, announced + 1);
    // The same report again tells nobody.
    apple
        .node
        .services
        .reach_changed(crate::apple_music_binding::APPLE_MUSIC_HOST, downloaded);
    assert_eq!(told_settled(&heard).await, announced + 1);
    // A catalog song is never stored on this device.
    let catalog = "catalog:777";
    assert_eq!(
        apple.binding.audio_availability(catalog),
        AudioAvailability::Unavailable
    );
    assert_eq!(
        apple.binding.audio_availability("i.s2"),
        AudioAvailability::RemoteOnly
    );

    // A download that finishes meanwhile is a change.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::from(["i.s1".to_owned()])),
    );
    assert_eq!(told_settled(&heard).await, announced + 2);

    // Online again, everything plays, and the lists reload.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::default(),
    );
    assert_eq!(
        song_availability(&apple).await?,
        all(AudioAvailability::RemoteOnly)
    );
    assert_eq!(told_settled(&heard).await, announced + 3);
    assert_eq!(
        apple.binding.audio_availability(catalog),
        AudioAvailability::RemoteOnly
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binding_made_while_offline_starts_offline() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, _) = open_apple_heard(dir.path()).await?;
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::new()),
    );
    assert_eq!(
        apple.binding.audio_availability("i.s1"),
        AudioAvailability::Unavailable
    );
    // Signing in again keeps what the host reported.
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
    assert_eq!(
        apple.binding.audio_availability("i.s1"),
        AudioAvailability::Unavailable
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_songs_are_listed_fetched_by_id_and_counted() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let record_id = apple.record.0.to_string();

    let page = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 1,
            page_size: 2,
            bake_cover_arts: false,
        })
        .await?;
    let titles: Vec<_> = page
        .media_files
        .iter()
        .map(|file| file.title.as_str())
        .collect();
    assert_eq!(titles, ["Second Wind", "Third Road"]);
    assert!(
        page.media_files
            .iter()
            .all(|file| file.library_id == record_id)
    );
    assert_eq!(page.media_files[0].track, "i.s2");
    assert_eq!(page.media_files[0].album, "Dawn Album");
    assert_eq!(page.media_files[0].track_number, Some(2));
    assert!(page.media_files[0].cover_art_id.is_some());

    let count = apple
        .run(&GetMediaFilesCountRequest { call_id: None })
        .await?;
    assert_eq!(count.count, 3);
    let summaries = apple
        .run(&SearchMediaFileSummaryRequest {
            call_id: None,
            n: 2,
        })
        .await?;
    assert_eq!(
        summaries
            .result
            .iter()
            .map(|song| song.name.as_str())
            .collect::<Vec<_>>(),
        ["First Light", "Second Wind"]
    );
    // The song list was read once for all of this.
    assert_eq!(apple.fake.count("/v1/me/library/songs"), 1);

    // Ids of songs in the list, then one the list lacks and one only the catalog has.
    let by_ids = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![page.media_files[1].id, page.media_files[0].id],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(by_ids.media_files[0].title, "Third Road");
    assert_eq!(by_ids.media_files[1].title, "Second Wind");
    assert_eq!(apple.fake.count("/v1/me/library/songs"), 1);

    let library_miss = apple.binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    let catalog_miss = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    let fetched = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![library_miss, catalog_miss],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(fetched.media_files[0].title, "Ninth Moon");
    assert_eq!(fetched.media_files[0].track, "i.s9");
    assert_eq!(fetched.media_files[1].title, "Catalog Tune");
    assert_eq!(fetched.media_files[1].track, "catalog:777");
    assert_eq!(apple.fake.count("/v1/catalog/it/songs"), 1);
    // Asking again is answered from what was fetched.
    let before = apple.fake.total();
    apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![library_miss, catalog_miss],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(apple.fake.total(), before);

    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: page.media_files[0].id,
        })
        .await?;
    assert_eq!(parsed.file.title, "Second Wind");
    assert_eq!(parsed.album.name, "Dawn Album");
    assert_eq!(parsed.artists[0].name, "Artist One");
    assert_ne!(parsed.album.id, 0);

    // An id this binding never gave is an input error.
    let unknown = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![9999],
            bake_cover_arts: false,
        })
        .await
        .err()
        .context("unknown id")?;
    assert_eq!(
        classify(&unknown),
        crate::messages::RequestErrorKind::InvalidInput
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_collections_are_listed_grouped_and_fetched() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;

    for (collection_type, groups, names) in [
        (
            CollectionType::Album,
            vec![("D", 1), ("N", 1)],
            vec!["Dawn Album"],
        ),
        (
            CollectionType::Artist,
            vec![("A", 2)],
            vec!["Artist One", "Artist Two"],
        ),
    ] {
        let summary = apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type,
            })
            .await?;
        let found: Vec<_> = summary
            .groups
            .iter()
            .map(|group| (group.group_title.as_str(), group.count))
            .collect();
        assert_eq!(found, groups, "{collection_type:?}");
        let title = groups[0].0.to_owned();
        let page = apple
            .run(&FetchCollectionGroupsRequest {
                call_id: None,
                collection_type,
                bake_cover_arts: false,
                group_titles: vec![title.clone()],
            })
            .await?;
        assert_eq!(page.groups[0].group_title, title);
        let listed: Vec<_> = page.groups[0]
            .collections
            .iter()
            .map(|collection| collection.name.as_str())
            .collect();
        assert_eq!(listed, names, "{collection_type:?}");
        let ids: Vec<i32> = page.groups[0].collections.iter().map(|c| c.id).collect();
        let by_ids = apple
            .run(&FetchCollectionByIdsRequest {
                call_id: None,
                collection_type,
                bake_cover_arts: false,
                ids: ids.clone(),
            })
            .await?;
        assert_eq!(
            by_ids.result.iter().map(|c| c.id).collect::<Vec<_>>(),
            ids,
            "{collection_type:?}"
        );
        assert!(
            by_ids
                .result
                .iter()
                .all(|c| c.collection_type == collection_type && c.readonly)
        );
    }

    let searched = apple
        .run(&SearchCollectionSummaryRequest {
            call_id: None,
            collection_type: Some(CollectionType::Album),
            bake_cover_arts: Some(false),
            n: 1,
        })
        .await?;
    assert_eq!(searched.result.len(), 1);

    // Genres and folders are refused when asked, not hidden by a panic, and so are the
    // playlists and mixes, which the library does not list.
    for refused in [
        CollectionType::Genre,
        CollectionType::Directory,
        CollectionType::Playlist,
        CollectionType::Mix,
    ] {
        let error = apple
            .run(&FetchCollectionGroupsRequest {
                call_id: None,
                collection_type: refused,
                bake_cover_arts: false,
                group_titles: vec!["A".into()],
            })
            .await
            .err()
            .context("refused collection type")?;
        assert!(unsupported(&error), "{refused:?}");
    }
    // Ids are numbered per kind, so an id nothing was given for is an input error.
    let unknown = apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Album,
            bake_cover_arts: false,
            ids: vec![4242],
        })
        .await
        .err()
        .context("unknown collection id")?;
    assert_eq!(
        classify(&unknown),
        crate::messages::RequestErrorKind::InvalidInput
    );
    apple.finish().await
}

fn song_ref(apple: &OpenApple, track: &str) -> PlayingItemRef {
    PlayingItemRef::LibraryTrack {
        library_id: apple.record.0.to_string(),
        track: track.into(),
    }
}

fn resolve(
    apple: &OpenApple,
    album: i32,
    initial: &str,
    excluded: &[&str],
) -> ResolveMixQueryRequest {
    ResolveMixQueryRequest {
        queries: vec![MixQuery {
            operator: "lib::album".into(),
            parameter: album.to_string(),
        }],
        hint_position: 0,
        initial_playback_item: Some(song_ref(apple, initial)),
        instantly_play: true,
        fallback_playing_items: vec![song_ref(apple, "i.s3")],
        excluded: ExcludedItems {
            items: excluded.iter().map(|id| song_ref(apple, id)).collect(),
        },
        call_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_resolves_a_mix_query_in_order_without_the_queue() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let tracks = |response: &crate::messages::ResolveMixQueryResponse| {
        response
            .playing_items
            .iter()
            .map(|item| match item {
                PlayingItemRef::LibraryTrack { track, .. } => track.clone(),
                PlayingItemRef::IndependentFile { path } => path.clone(),
            })
            .collect::<Vec<_>>()
    };
    let all = apple.run(&resolve(&apple, album, "i.s2", &[])).await?;
    assert_eq!(tracks(&all), ["i.s1", "i.s2"]);
    assert_eq!(all.start_index, Some(1));
    // The caller's own queue is left out, and the start index follows.
    let without = apple
        .run(&resolve(&apple, album, "i.s2", &["i.s1"]))
        .await?;
    assert_eq!(tracks(&without), ["i.s2"]);
    assert_eq!(without.start_index, Some(0));
    // A track the query does not hold starts the fallback items at the hint.
    let mut elsewhere = resolve(&apple, album, "i.s9", &[]);
    elsewhere.hint_position = 1;
    let missing = apple.run(&elsewhere).await?;
    assert_eq!(tracks(&missing), ["i.s3"]);
    assert_eq!(missing.start_index, Some(1));
    // Without a wish to play there is no index.
    let mut quiet = resolve(&apple, album, "i.s2", &[]);
    quiet.instantly_play = false;
    let quiet = apple.run(&quiet).await?;
    assert_eq!(
        (tracks(&quiet), quiet.start_index),
        (vec!["i.s1".into(), "i.s2".into()], None)
    );
    // The song list answered all of this, with no read of the album.
    assert_eq!(apple.fake.count("/v1/me/library/albums/l.alb1"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_home_lists_its_groups_and_leaves_the_rest_empty() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let group = |title: &str, query| ComplexQuery {
        title: title.into(),
        query,
    };
    let mode = LibraryHomeListMode::Name;
    let response = apple
        .run(&ComplexQueryRequest {
            queries: vec![
                group("Artists", LibraryHomeQuery::Artists(mode)),
                group("Albums", LibraryHomeQuery::Albums(mode)),
                group("Playlists", LibraryHomeQuery::Playlists(mode)),
                group("Tracks", LibraryHomeQuery::Tracks(mode)),
                group("Liked", LibraryHomeQuery::Liked),
                group("Mixes", LibraryHomeQuery::Mixes(mode)),
                group("Genres", LibraryHomeQuery::Genres(mode)),
                group("Most", LibraryHomeQuery::Most),
            ],
            call_id: None,
        })
        .await?;
    let names = |index: usize| {
        response.result[index]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        response
            .result
            .iter()
            .map(|group| group.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Artists",
            "Albums",
            "Playlists",
            "Tracks",
            "Liked",
            "Mixes",
            "Genres",
            "Most"
        ]
    );
    assert_eq!(names(0), ["Artist One", "Artist Two"]);
    assert_eq!(names(1), ["Dawn Album", "Night Album"]);
    // The library lists no playlists, mixes or likes.
    assert!(names(2).is_empty());
    assert_eq!(names(3), ["First Light", "Second Wind", "Third Road"]);
    for empty in 4..8 {
        assert!(response.result[empty].entries.is_empty());
    }
    // An album opens by the query that lists its tracks.
    let album = &response.result[1].entries[0];
    assert_eq!(album.queries[0].operator, "lib::album");
    assert_eq!(album.queries[0].parameter, album.id.to_string());
    assert!(album.readonly);
    // A track plays the listed tracks from itself.
    let track = &response.result[3].entries[1];
    let ids: Vec<_> = track
        .queries
        .iter()
        .map(|query| query.parameter.clone())
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], track.id.to_string());
    // Those queries resolve to the tracks, in that order.
    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: track.queries.clone(),
            hint_position: 0,
            initial_playback_item: None,
            instantly_play: false,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(resolved.playing_items[0], song_ref(&apple, "i.s2"));
    assert_eq!(resolved.playing_items.len(), 3);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_takes_the_queries_the_tracks_and_album_pages_send() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let resolve_with = |queries: Vec<MixQuery>, initial: &str| ResolveMixQueryRequest {
        queries,
        hint_position: 0,
        initial_playback_item: Some(song_ref(&apple, initial)),
        instantly_play: true,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems::default(),
        call_id: None,
    };
    let keys = |response: &crate::messages::ResolveMixQueryResponse| {
        response
            .playing_items
            .iter()
            .map(|item| match item {
                PlayingItemRef::LibraryTrack { track, .. } => track.clone(),
                PlayingItemRef::IndependentFile { path } => path.clone(),
            })
            .collect::<Vec<_>>()
    };
    let query = |operator: &str, parameter: &str| MixQuery {
        operator: operator.into(),
        parameter: parameter.into(),
    };
    // The tracks page: every song, in the order of the song list.
    let all = vec![query("lib::all", "true")];
    let resolved = apple.run(&resolve_with(all.clone(), "i.s3")).await?;
    assert_eq!(keys(&resolved), ["i.s1", "i.s2", "i.s3"]);
    assert_eq!(resolved.start_index, Some(2));
    let page = apple
        .run(&MixQueryRequest {
            call_id: None,
            queries: all,
            cursor: 1,
            page_size: 5,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        })
        .await?;
    assert_eq!(
        page.files
            .iter()
            .map(|file| file.track.as_str())
            .collect::<Vec<_>>(),
        ["i.s2", "i.s3"]
    );
    // An album's page: its tracks in order, with the track-number sort it already has.
    let detail = vec![
        query("lib::album", &album.to_string()),
        query("sort::track_number", "true"),
    ];
    let resolved = apple.run(&resolve_with(detail.clone(), "i.s2")).await?;
    assert_eq!(keys(&resolved), ["i.s1", "i.s2"]);
    let page = apple
        .run(&MixQueryRequest {
            call_id: None,
            queries: detail,
            cursor: 0,
            page_size: 5,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        })
        .await?;
    assert_eq!(
        page.files
            .iter()
            .map(|file| file.track.as_str())
            .collect::<Vec<_>>(),
        ["i.s1", "i.s2"]
    );
    // A sort on anything but an album stays unsupported, and names the request.
    let refused = apple
        .run(&resolve_with(
            vec![
                query("lib::artist", "1"),
                query("sort::track_number", "true"),
            ],
            "i.s1",
        ))
        .await
        .err()
        .context("unsupported query")?;
    assert!(unsupported(&refused));
    assert!(format!("{refused:#}").contains("ResolveMixQueryRequest"));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_answers_a_random_query_with_covers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            bare_night: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let query = |count: &str, page: (i32, i32)| MixQueryRequest {
        call_id: None,
        queries: vec![
            MixQuery {
                operator: "lib::random".into(),
                parameter: count.into(),
            },
            MixQuery {
                operator: "filter::with_cover_art".into(),
                parameter: "true".into(),
            },
        ],
        cursor: page.0,
        page_size: page.1,
        bake_cover_arts: false,
        queue: CallerQueue::default(),
        random_seed: None,
    };
    let tracks = |response: &crate::messages::MixQueryResponse| {
        response
            .files
            .iter()
            .map(|file| file.track.clone())
            .collect::<Vec<_>>()
    };
    // Only the two songs with a cover are drawn, each once, for any count.
    let drawn = apple.run(&query("30", (0, 30))).await?;
    let mut sorted = tracks(&drawn);
    sorted.sort();
    assert_eq!(sorted, ["i.s1", "i.s2"]);
    // A smaller count is a sample of that size.
    assert_eq!(tracks(&apple.run(&query("1", (0, 30))).await?).len(), 1);
    // The page cuts the draw.
    assert_eq!(tracks(&apple.run(&query("30", (0, 1))).await?).len(), 1);
    assert!(tracks(&apple.run(&query("30", (2, 5))).await?).is_empty());
    // Without the filter the song without a cover is drawn too.
    let mut plain = mix("lib::random", 30);
    plain.page_size = 30;
    assert_eq!(tracks(&apple.run(&plain).await?).len(), 3);
    // The filter goes with the whole song list too.
    let mut album = mix("lib::all", "true");
    album.queries.push(MixQuery {
        operator: "filter::with_cover_art".into(),
        parameter: "true".into(),
    });
    assert_eq!(tracks(&apple.run(&album).await?), ["i.s1", "i.s2"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pages_of_a_seeded_random_draw_are_pages_of_one_draw() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            extra_songs: 30,
            ..AppleOptions::default()
        },
    )
    .await?;
    let page = |seed: Option<u64>, cursor: i32, page_size: i32| {
        let mut request = mix("lib::random", 20);
        request.random_seed = seed;
        request.cursor = cursor;
        request.page_size = page_size;
        let apple = &apple;
        async move {
            Ok::<_, anyhow::Error>(
                apple
                    .run(&request)
                    .await?
                    .files
                    .into_iter()
                    .map(|file| file.track)
                    .collect::<Vec<_>>(),
            )
        }
    };
    // Two pages of a seeded draw are the two halves of the draw in one page.
    let whole = page(Some(7), 0, 20).await?;
    assert_eq!(whole.len(), 20);
    let mut halves = page(Some(7), 0, 10).await?;
    halves.extend(page(Some(7), 10, 10).await?);
    assert_eq!(halves, whole);
    // Another seed draws another list, as does a draw without a seed, almost surely.
    assert_ne!(page(Some(8), 0, 20).await?, whole);
    assert_ne!(page(None, 0, 20).await?, whole);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_random_query_of_an_empty_library_is_empty() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_listing(dir.path(), true).await?;
    let mut request = mix("lib::random", 30);
    request.queries.push(MixQuery {
        operator: "filter::with_cover_art".into(),
        parameter: "true".into(),
    });
    assert!(apple.run(&request).await?.files.is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cover_filtered_collection_is_cut_from_the_song_list_for_every_page() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let page = |cursor| {
        let mut request = mix("lib::album", album);
        request.queries.push(MixQuery {
            operator: "filter::with_cover_art".into(),
            parameter: "true".into(),
        });
        request.cursor = cursor;
        request.page_size = 1;
        request
    };
    let first = apple.run(&page(0)).await?;
    assert_eq!(first.files.len(), 1);
    // A later page cuts the same songs, and the album is never read.
    let second = apple.run(&page(1)).await?;
    assert_eq!(second.files.len(), 1);
    assert_ne!(first.files[0].track, second.files[0].track);
    assert_eq!(apple.fake.count("/v1/me/library/albums/l.alb1"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_home_shows_the_newest_first_in_reverse() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // Apple lists the album it added last first.
    apple.fake.night_added_last.store(true, Ordering::SeqCst);
    let group = |title: &str, query| ComplexQuery {
        title: title.into(),
        query,
    };
    let names = |response: &crate::messages::ComplexQueryResponse, index: usize| {
        response.result[index]
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };
    let newest = apple
        .run(&ComplexQueryRequest {
            queries: vec![
                group(
                    "Albums",
                    LibraryHomeQuery::Albums(LibraryHomeListMode::Reverse),
                ),
                group(
                    "Tracks",
                    LibraryHomeQuery::Tracks(LibraryHomeListMode::Reverse),
                ),
            ],
            call_id: None,
        })
        .await?;
    assert_eq!(names(&newest, 0), ["Night Album", "Dawn Album"]);
    assert_eq!(
        names(&newest, 1),
        ["Third Road", "Second Wind", "First Light"]
    );
    let oldest = apple
        .run(&ComplexQueryRequest {
            queries: vec![
                group(
                    "Albums",
                    LibraryHomeQuery::Albums(LibraryHomeListMode::Forward),
                ),
                group(
                    "Tracks",
                    LibraryHomeQuery::Tracks(LibraryHomeListMode::Forward),
                ),
            ],
            call_id: None,
        })
        .await?;
    assert_eq!(names(&oldest, 0), ["Dawn Album", "Night Album"]);
    assert_eq!(
        names(&oldest, 1),
        ["First Light", "Second Wind", "Third Road"]
    );
    // A random group shows the same entries each time the home is read.
    let random = |_: ()| ComplexQueryRequest {
        queries: vec![group(
            "Tracks",
            LibraryHomeQuery::Tracks(LibraryHomeListMode::Random),
        )],
        call_id: None,
    };
    let first = apple.run(&random(())).await?;
    let second = apple.run(&random(())).await?;
    assert_eq!(names(&first, 0), names(&second, 0));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_apple_library_has_an_empty_home() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_listing(dir.path(), true).await?;
    let mode = LibraryHomeListMode::Reverse;
    let titles = ["Artists", "Albums", "Playlists", "Tracks", "Liked"];
    let queries = [
        LibraryHomeQuery::Artists(mode),
        LibraryHomeQuery::Albums(mode),
        LibraryHomeQuery::Playlists(mode),
        LibraryHomeQuery::Tracks(mode),
        LibraryHomeQuery::Liked,
    ];
    let response = apple
        .run(&ComplexQueryRequest {
            queries: titles
                .iter()
                .zip(queries)
                .map(|(title, query)| ComplexQuery {
                    title: (*title).into(),
                    query,
                })
                .collect(),
            call_id: None,
        })
        .await?;
    assert_eq!(response.result.len(), titles.len());
    assert!(response.result.iter().all(|group| group.entries.is_empty()));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rejected_sign_in_fails_requests_with_its_record_and_keeps_the_binding() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;

    // 401 on the request and on the storefront read that follows it: the sign-in is over. A
    // song the library does not list is read from Apple, as nothing kept it yet.
    apple.fake.reject_everything.store(true, Ordering::SeqCst);
    let unlisted = apple.binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    let error = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: unlisted,
        })
        .await
        .err()
        .context("rejected sign-in")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: apple.record.0.to_string(),
        })
    );
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::PermissionDenied
    );
    // The binding is not signed out: the app signs in again and goes on.
    assert!(apple.binding.is_signed_in());
    apple.fake.reject_everything.store(false, Ordering::SeqCst);
    assert_eq!(
        apple
            .run(&GetMediaFilesCountRequest { call_id: None })
            .await?
            .count,
        3
    );

    // A 401 on one read while the storefront read works is answered by the library's own
    // second try, so the request answers normally.
    apple.fake.reject_next_read.store(true, Ordering::SeqCst);
    let before = apple.fake.count("/v1/me/library/artists");
    // Artists are read afresh, unlike the songs kept by now.
    let artists = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?;
    assert_eq!(artists.groups[0].count, 2);
    // The library's own second try after the 401: this pins that retry.
    assert_eq!(apple.fake.count("/v1/me/library/artists"), before + 2);

    // A read Apple refuses again while the storefront read still works is not a lost
    // sign-in: the request fails as an ordinary error and asks for no new sign-in.
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/catalog/it/songs".into(),
        401,
        r#"{"errors":[{"status":"401","code":"UNAUTHORIZED"}]}"#.into(),
    ));
    let error = catalog_song(&apple)
        .await
        .err()
        .context("refused request")?;
    assert_eq!(request_failure_cause(&error), None);
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable
    );
    assert!(apple.binding.is_signed_in());
    *apple.fake.respond_path.lock().unwrap() = None;
    assert_eq!(catalog_song(&apple).await?.file.title, "Catalog Tune");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn covers_download_once_without_credentials_and_back_the_visualizer() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    apple
        .run(&GetMediaFilesCountRequest { call_id: None })
        .await?;

    let (summary, _) = apple.binding.item_metadata("i.s1", true).await?;
    assert_eq!(summary.title, "First Light");
    // The cover downloads in the background; asking again finds it once it is there.
    let path = wait_until("the cover is cached", || async {
        Ok(apple.binding.item_metadata("i.s1", true).await?.1)
    })
    .await?;
    assert_eq!(std::fs::read(&path)?, b"cover-bytes");
    assert_eq!(apple.fake.count("/art/Dawn_Album/600x600bb.jpg"), 1);
    assert!(
        apple
            .fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with("/art/"))
            .all(|(_, authed)| !authed),
        "a cover download carried Apple credentials"
    );

    let (playing, wall) = apple.binding.visualizer_covers("i.s1").await?;
    assert_eq!(playing.as_deref(), Some(path.as_str()));
    // The other album's cover joins the wall.
    assert_eq!(wall.len(), 1);
    assert_ne!(wall[0], path);
    // The playing cover came from the cache.
    assert_eq!(apple.fake.count("/art/Dawn_Album"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parsed_media_files_share_collection_ids_and_take_catalog_songs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let ids_of = |collection_type| {
        let title = if collection_type == CollectionType::Album {
            "D"
        } else {
            "A"
        };
        let apple = &apple;
        async move {
            Ok::<_, anyhow::Error>(
                apple
                    .run(&FetchCollectionGroupsRequest {
                        call_id: None,
                        collection_type,
                        bake_cover_arts: false,
                        group_titles: vec![title.into()],
                    })
                    .await?
                    .groups
                    .remove(0)
                    .collections
                    .into_iter()
                    .map(|collection| (collection.name, collection.id))
                    .collect::<std::collections::HashMap<_, _>>(),
            )
        }
    };
    let albums = ids_of(CollectionType::Album).await?;
    let artists = ids_of(CollectionType::Artist).await?;
    let first = apple.binding.intern(AppleIdKind::LibrarySong, "i.s1")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: first,
        })
        .await?;
    assert_eq!(parsed.album.id, albums["Dawn Album"]);
    assert_eq!(parsed.artists[0].id, artists["Artist One"]);

    let catalog = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: catalog,
        })
        .await?;
    assert_eq!(parsed.file.track, "catalog:777");
    assert_eq!(parsed.file.title, "Catalog Tune");
    assert_eq!(parsed.album.name, "Catalog Album");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paging_past_the_end_is_empty_and_a_negative_position_is_an_input_error() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let past = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 100,
            page_size: 10,
            bake_cover_arts: false,
        })
        .await?;
    assert!(past.media_files.is_empty());
    let mut page = mix(
        "lib::album",
        apple.binding.intern(AppleIdKind::Album, "l.alb1")?,
    );
    page.cursor = 100;
    assert!(apple.run(&page).await?.files.is_empty());

    let invalid = crate::messages::RequestErrorKind::InvalidInput;
    let negative = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: -1,
            page_size: 10,
            bake_cover_arts: false,
        })
        .await
        .err()
        .context("negative cursor")?;
    assert_eq!(classify(&negative), invalid);
    page.cursor = -1;
    let negative = apple
        .run(&page)
        .await
        .err()
        .context("negative mix cursor")?;
    assert_eq!(classify(&negative), invalid);
    page.cursor = 0;
    page.page_size = -1;
    let negative = apple.run(&page).await.err().context("negative page size")?;
    assert_eq!(classify(&negative), invalid);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_library_has_no_songs_and_no_groups() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_listing(dir.path(), true).await?;
    assert_eq!(
        apple
            .run(&GetMediaFilesCountRequest { call_id: None })
            .await?
            .count,
        0
    );
    for collection_type in [CollectionType::Album, CollectionType::Artist] {
        let summary = apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type,
            })
            .await?;
        assert!(summary.groups.is_empty(), "{collection_type:?}");
    }
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_apple_rejects_carries_its_status_and_code_but_no_token() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/catalog/it/songs".into(),
        400,
        r#"{"errors":[{"status":"400","code":"PARAMETER_ERROR"}]}"#.into(),
    ));
    // A song of the catalog is read from the catalog.
    let error = catalog_song(&apple)
        .await
        .err()
        .context("rejected request")?;
    let text = format!("{error:#}");
    assert!(
        text.contains("status 400") && text.contains("PARAMETER_ERROR"),
        "{text}"
    );
    assert!(!text.contains(USER_TOKEN) && !text.contains(DEVELOPER_TOKEN));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cached_summary_never_starts_the_read_the_song_list_needs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // A binding nothing has read yet knows no song, and does not go and ask.
    let setup = fake_services(&apple.fake.base, &TestClock::new());
    let unread = AppleMusicBinding::new(
        uuid::Uuid::new_v4(),
        |tokens| setup.apple_music_client(tokens),
        dir.path(),
        Arc::new(Quiet),
        Arc::new(SystemClock),
    );
    unread.sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).context("tokens")?);
    let before = apple.fake.total();
    assert!(unread.cached_summary("i.s1").is_none());
    assert_eq!(apple.fake.total(), before);

    // The opened library has read its song list, and answers from it.
    let summary = apple
        .binding
        .cached_summary("i.s2")
        .context("listed song")?;
    assert_eq!(
        (
            summary.title.as_str(),
            summary.album.as_str(),
            summary.artist.as_str()
        ),
        ("Second Wind", "Dawn Album", "Artist One")
    );
    assert_eq!(summary.track_number, 2);
    assert!((summary.duration - 200.0).abs() < f64::EPSILON);
    assert!(apple.binding.cached_summary("i.unlisted").is_none());
    assert!(apple.binding.cached_summary("not a key").is_none());
    assert_eq!(apple.fake.total(), before);
    apple.finish().await
}

/// Writes a Subsonic record that points at `endpoint`.
async fn write_subsonic_record(
    node: &Arc<NodeContext>,
    endpoint: String,
) -> Result<LibraryRecordId> {
    use database::credentials::{Credential, Secret};
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter, SubsonicOpenMode,
    };

    let credential = node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"unused".to_vec()),
        })
        .await?;
    let record = LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Closed service".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint,
            credential,
            mode: SubsonicOpenMode::PlayOnThisDevice,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let record_id = record.record_id;
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(record);
        Ok(())
    })
    .await?;
    Ok(record_id)
}

/// A library that answers lyrics and likes itself, standing for a Subsonic one: like a
/// Subsonic library's, its answers are only for its own items. `slots` are how it runs requests
/// as the open library.
struct LyricService {
    library_id: uuid::Uuid,
    slots: RequestSlots,
    liked: Mutex<std::collections::HashSet<String>>,
}

impl LyricService {
    fn with_slots(library_id: uuid::Uuid, slots: RequestSlots) -> Self {
        Self {
            library_id,
            slots,
            liked: Mutex::default(),
        }
    }

    /// As the stand-in was before it took a Subsonic library's slots: lyrics and colors are
    /// left to the library of each item named.
    fn routing_by_item(library_id: uuid::Uuid) -> Self {
        let mut slots = RequestSlots::unimplemented();
        slots.get_lyric_by_track_id_request = Slot::Native;
        slots.get_primary_color_by_track_id_request = Slot::Native;
        Self::with_slots(library_id, slots)
    }

    /// The track `item` names, refused as a Subsonic library refuses an item of another one.
    fn own_track(&self, item: &Option<PlayingItemRef>) -> Result<String> {
        match item {
            Some(PlayingItemRef::LibraryTrack { library_id, track })
                if *library_id == self.library_id.to_string() =>
            {
                Ok(track.clone())
            }
            _ => Err(ExpectedRequestError::invalid_input("Item belongs to another library").into()),
        }
    }
}

impl crate::macros::RequestTranslator for LyricService {
    fn translate_get_lyric_by_track_id_request<'a>(
        &'a self,
        request: &'a GetLyricByTrackIdRequest,
    ) -> crate::rpc::RequestTranslationFuture<'a, GetLyricByTrackIdResponse> {
        Box::pin(async move {
            self.own_track(&request.item)?;
            Ok(Some(GetLyricByTrackIdResponse {
                item: request.item.clone().context("lyric item")?,
                lines: vec![LyricContentLine {
                    start_time: 0,
                    end_time: 1,
                    sections: Vec::new(),
                }],
                call_id: None,
            }))
        })
    }

    fn translate_set_liked_request<'a>(
        &'a self,
        request: &'a SetLikedRequest,
    ) -> crate::rpc::RequestTranslationFuture<'a, crate::messages::SetLikedResponse> {
        Box::pin(async move {
            let track = self.own_track(&request.item)?;
            let mut liked = self.liked.lock().unwrap();
            if request.liked {
                liked.insert(track);
            } else {
                liked.remove(&track);
            }
            Ok(Some(crate::messages::SetLikedResponse {
                item: request.item.clone().context("liked item")?,
                liked: request.liked,
                call_id: None,
            }))
        })
    }

    fn translate_get_liked_request<'a>(
        &'a self,
        request: &'a crate::messages::GetLikedRequest,
    ) -> crate::rpc::RequestTranslationFuture<'a, crate::messages::GetLikedResponse> {
        Box::pin(async move {
            let track = self.own_track(&request.item)?;
            Ok(Some(crate::messages::GetLikedResponse {
                item: request.item.clone().context("liked item")?,
                liked: self.liked.lock().unwrap().contains(&track),
                call_id: None,
            }))
        })
    }
}

impl ServiceBinding for LyricService {
    fn library_id(&self) -> uuid::Uuid {
        self.library_id
    }

    fn slots(&self) -> RequestSlots {
        self.slots
    }

    fn service(&self) -> ServiceKind {
        ServiceKind::Subsonic
    }

    fn item_metadata<'a>(
        &'a self,
        _track: &'a str,
        _with_cover: bool,
    ) -> crate::service::ServiceFuture<
        'a,
        (
            database::playing_item::PlayingItemMetadataSummary,
            Option<String>,
        ),
    > {
        Box::pin(async { anyhow::bail!("no metadata") })
    }

    fn visualizer_covers<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, (Option<String>, Vec<String>)> {
        Box::pin(async { anyhow::bail!("no covers") })
    }

    fn playback_source<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, playback::player::ServiceTarget> {
        Box::pin(async { anyhow::bail!("no playback") })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lyrics_and_colors_go_to_the_library_each_item_belongs_to() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local_library = apple
        .fixture
        .global_params
        .library_id
        .context("fixture library id")?;
    // Another streaming library is the one open, so the Apple item is a foreign one.
    let other_record = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let other = other_record.0;
    apple
        .node
        .open_service_catalog(
            Some(other_record),
            Arc::new(LyricService::routing_by_item(other)),
        )
        .await?;
    let item = |library_id: uuid::Uuid, track: &str| PlayingItemRef::LibraryTrack {
        library_id: library_id.to_string(),
        track: track.into(),
    };
    let apple_item = item(apple.record.0, "i.s1");
    let other_item = item(other, "t1");
    let local_item = item(local_library, &uuid::Uuid::new_v4().to_string());
    // The UI's own params of an open streaming library hold no local library.
    let ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    assert!(ui.library.is_none());
    let requests_before = apple.fake.total();

    for params in [&ui, &apple.fixture.global_params] {
        let lyric = |item: &PlayingItemRef| {
            let request = GetLyricByTrackIdRequest {
                call_id: None,
                item: Some(item.clone()),
            };
            async move { dispatch_gui_request(params, &request).await }
        };
        // An Apple item's library declares no lyrics for its items, which it refuses.
        let refused = lyric(&apple_item).await.err().context("Apple lyrics")?;
        assert!(unsupported(&refused), "{refused:#}");
        // Another streaming library's item gets that library's own answer.
        let answered = lyric(&other_item)
            .await?
            .context("other library's lyrics")?;
        assert_eq!(answered.lines.len(), 1);
        // A local item reaches its own library, which has nothing for it.
        let local = lyric(&local_item).await?.context("local lyrics")?;
        assert!(local.lines.is_empty());

        let color = |item: &PlayingItemRef| {
            let request = GetPrimaryColorByTrackIdRequest {
                call_id: None,
                item: Some(item.clone()),
            };
            async move { dispatch_gui_request(params, &request).await }
        };
        // Without a local library open, a misrouted Apple item has no library to read from
        // and fails; routed to its binding it answers no color.
        let apple_color = color(&apple_item).await?.context("Apple color")?;
        assert_eq!(apple_color.primary_color, None);
        assert!(color(&local_item).await?.is_some());
    }
    // With no item the request still needs an open local library, as it always did.
    let none = GetLyricByTrackIdRequest {
        call_id: None,
        item: None,
    };
    assert!(dispatch_gui_request(&ui, &none).await.is_err());
    assert!(
        dispatch_gui_request(&apple.fixture.global_params, &none)
            .await?
            .is_none()
    );
    assert_eq!(apple.fake.total(), requests_before);
    apple.finish().await
}

/// A Subsonic library's slots, as a real binding gets them from a server that has no jukebox.
fn subsonic_slots() -> RequestSlots {
    RequestSlots::subsonic(&subsonic::SubsonicProbe {
        api_version: "1.16.1".into(),
        open_subsonic: false,
        server_type: None,
        server_version: None,
        extensions: Vec::new(),
        roles: subsonic::SubsonicRoles::default(),
        roles_known: true,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_subsonic_library_leaves_the_lyrics_of_other_libraries_items_to_them() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "beside-subsonic.ogg").await?;
    let record = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    apple
        .node
        .open_service_catalog(
            Some(record),
            Arc::new(LyricService::with_slots(record.0, subsonic_slots())),
        )
        .await?;
    let ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    let lyric = |item: PlayingItemRef| {
        let request = GetLyricByTrackIdRequest {
            call_id: None,
            item: Some(item),
        };
        let ui = &ui;
        async move { dispatch_gui_request(ui, &request).await }
    };

    // A local track's lyrics come from its own library, which has none for it.
    let local_lyrics = lyric(local).await?.context("local lyrics")?;
    assert!(local_lyrics.lines.is_empty());
    // An Apple Music track's library declares no lyrics, whichever library is open.
    let refused = lyric(song_ref(&apple, "i.s1"))
        .await
        .err()
        .context("Apple lyrics")?;
    assert!(unsupported(&refused), "{refused:#}");
    // The Subsonic library still answers for its own tracks.
    let own = lyric(PlayingItemRef::LibraryTrack {
        library_id: record.0.to_string(),
        track: "s1".into(),
    })
    .await?
    .context("Subsonic lyrics")?;
    assert_eq!(own.lines.len(), 1);
    apple.finish().await
}

/// Asks each request routed by item about `item` through `params`, and checks it fails as
/// unsupported exactly where `declared` hides it. Any other failure must be one the UI takes
/// as such, as a library out of reach, never an internal one.
async fn asking_matches(
    params: &crate::utils::GlobalParams,
    item: &PlayingItemRef,
    declared: &crate::messages::PlayingItemCapabilities,
) -> Result<()> {
    use crate::messages::{GetLikedRequest, RequestErrorKind};

    let named = Some(item.clone());
    let asked = [
        (
            "SetLikedRequest",
            declared.set_liked_request,
            dispatch_gui_request(
                params,
                &SetLikedRequest {
                    item: named.clone(),
                    liked: true,
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
        (
            "GetLikedRequest",
            declared.get_liked_request,
            dispatch_gui_request(
                params,
                &GetLikedRequest {
                    item: named.clone(),
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
        (
            "GetLyricByTrackIdRequest",
            declared.get_lyric_by_track_id_request,
            dispatch_gui_request(
                params,
                &GetLyricByTrackIdRequest {
                    item: named.clone(),
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
        (
            "GetPrimaryColorByTrackIdRequest",
            declared.get_primary_color_by_track_id_request,
            dispatch_gui_request(
                params,
                &GetPrimaryColorByTrackIdRequest {
                    item: named,
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
    ];
    for (request, capability, error) in asked {
        let refused = error.as_ref().is_some_and(unsupported);
        assert_eq!(
            capability == RequestCapability::Hidden,
            refused,
            "{request} for {item:?}: {capability:?}, {error:?}"
        );
        if let Some(error) = &error {
            assert_ne!(
                classify(error),
                RequestErrorKind::Internal,
                "{request} for {item:?}: {error:#}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_an_item_supports_is_what_asking_about_it_does() -> Result<()> {
    use crate::macros::item_capability_snapshot;
    use crate::request_owner::Implementation;
    use crate::service::item_slots;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "asked-about.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let subsonic_record = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let subsonic: Arc<dyn ServiceBinding> = Arc::new(LyricService::with_slots(
        subsonic_record.0,
        subsonic_slots(),
    ));
    let subsonic_track = PlayingItemRef::LibraryTrack {
        library_id: subsonic_record.0.to_string(),
        track: "s1".into(),
    };
    let apple_binding: Arc<dyn ServiceBinding> = apple.binding.clone();

    // Each binding is of the service its record names, so what a signed-in binding declares
    // for its items is what the record declares.
    let registry = LibraryRegistry::load(Path::new(&*apple.node.config_path))?;
    for (record, binding) in [(apple.record, &apple_binding), (subsonic_record, &subsonic)] {
        let protocol = &registry
            .record(record)
            .map_err(|_| anyhow::anyhow!("the record of {record:?}"))?
            .protocol;
        let service = protocol.service().context("a service record")?;
        assert_eq!(binding.service(), service);
        assert_eq!(
            item_capability_snapshot(None, &Implementation::Service(binding.clone()).item_slots()),
            item_capability_snapshot(None, &item_slots(service))
        );
    }
    let declared = |item: &PlayingItemRef, implementation: Implementation| {
        item_capability_snapshot(Some(item.clone()), &implementation.item_slots())
    };
    let items = [
        declared(&song, Implementation::Service(apple_binding.clone())),
        declared(&local, Implementation::Local),
        declared(&subsonic_track, Implementation::Service(subsonic.clone())),
    ];
    let ask_all = |params: &crate::utils::GlobalParams| {
        let items = items.clone();
        let params = params.clone();
        async move {
            for told in &items {
                let item = told.item.as_ref().context("a declared item")?;
                asking_matches(&params, item, told).await?;
            }
            anyhow::Ok(())
        }
    };

    // With the Apple Music library open, asked through it and through the local library.
    let apple_ui = Arc::clone(&apple.ui);
    ask_all(&apple_ui).await?;
    ask_all(&apple.fixture.global_params).await?;

    // With the Subsonic library open.
    apple
        .node
        .open_service_catalog(Some(subsonic_record), subsonic.clone())
        .await?;
    let subsonic_ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the Subsonic library")?;
    ask_all(&subsonic_ui).await?;

    // With the local library open.
    open_own_library(&apple).await?;
    ask_all(&apple.fixture.global_params).await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_events_of_a_device_only_library_are_withheld_from_other_devices() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let policy = &apple.node.access_policy;
    let task = |record_id: String| {
        rinf::serialize(&LibraryTaskStateChanged {
            state: LibraryTaskState::AnalysisRunning {
                record_id,
                run_id: "run".into(),
                progress: 1,
                total: 2,
            },
        })
    };
    let local_record = uuid::Uuid::new_v4().to_string();
    // The Apple library is open and kept on this device; its own task event stays here.
    assert!(withheld_library_event(
        policy,
        "LibraryTaskStateChanged",
        &task(apple.record.0.to_string())?
    ));
    // A local library's event goes on telling other devices while Apple is open.
    assert!(!withheld_library_event(
        policy,
        "LibraryTaskStateChanged",
        &task(local_record)?
    ));
    // Other events name no library and are not decided here, and a task event nobody can
    // read is not let through.
    let content = rinf::serialize(&LibraryContentChanged {
        path: String::new(),
    })?;
    assert!(!withheld_library_event(
        policy,
        "LibraryContentChanged",
        &content
    ));
    assert!(!withheld_library_event(policy, "PlaybackStatus", &[]));
    assert!(withheld_library_event(
        policy,
        "LibraryTaskStateChanged",
        &[1, 2, 3]
    ));
    apple.finish().await
}

/// What a device's broadcaster was told, to see what reached it over the network.
#[derive(Default)]
struct Heard(Mutex<Vec<(String, Vec<u8>)>>);

impl crate::utils::Broadcaster for Heard {
    fn broadcast(&self, message: &dyn crate::utils::RinfRustSignal) {
        self.0
            .lock()
            .unwrap()
            .push((message.name(), message.encode_to_vec().unwrap()));
    }
}

impl Heard {
    fn task_records(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "LibraryTaskStateChanged")
            .filter_map(|(_, payload)| rinf::deserialize::<LibraryTaskStateChanged>(payload).ok())
            .map(|changed| changed.state.record_id().to_owned())
            .collect()
    }

    fn count(&self, name: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(heard, _)| heard == name)
            .count()
    }

    fn heard(&self, name: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .iter()
            .any(|(heard, _)| heard == name)
    }

    /// Every message named `name` that decodes as `T`, with its place among all heard.
    fn placed<T: serde::de::DeserializeOwned>(&self, name: &str) -> Vec<(usize, T)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, (heard, _))| heard == name)
            .filter_map(|(place, (_, payload))| {
                rinf::deserialize::<T>(payload)
                    .ok()
                    .map(|message| (place, message))
            })
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_outgoing_loop_keeps_a_device_only_librarys_task_events_off_the_wire() -> Result<()> {
    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = open_apple(host_dir.path()).await?;
    let heard = Arc::new(Heard::default());
    let controller = super::fixture_with_broadcaster(
        controller_dir.path(),
        uuid::Uuid::new_v4(),
        std::time::Duration::from_secs(300),
        heard.clone(),
    )
    .await?;
    // The host's own library is open while the devices pair, as in a real session.
    let host_library = host
        .fixture
        .global_params
        .library_id
        .context("host library id")?;
    host.node.set_open_local_catalog(None, host_library).await?;
    let address = super::paired_full(&host.fixture, &controller).await?;
    let fingerprint = super::info(&host.fixture).await?.fingerprint;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;
    super::open_remote_control_record(controller.global_params.node.clone(), &fingerprint).await?;
    host.node
        .open_service_catalog(Some(host.record), host.binding.clone())
        .await?;

    let task = |record_id: String| LibraryTaskStateChanged {
        state: LibraryTaskState::AnalysisRunning {
            record_id,
            run_id: "run".into(),
            progress: 1,
            total: 2,
        },
    };
    let local_record = uuid::Uuid::new_v4().to_string();
    let broadcaster = &host.fixture.global_params.broadcaster;
    broadcaster.broadcast(&task(host.record.0.to_string()));
    broadcaster.broadcast(&task(local_record.clone()));
    broadcaster.broadcast(&LibraryContentChanged {
        path: String::new(),
    });
    // Events reach the controller in the order they were sent, so once the last has arrived
    // the first would have.
    wait_until("the content change to arrive", || async {
        Ok(heard.heard("LibraryContentChanged").then_some(()))
    })
    .await?;
    assert_eq!(heard.task_records(), vec![local_record]);
    host.server.abort();
    host.node.shutdown().await?;
    controller.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_the_record_stops_covers_that_are_still_downloading() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.delay_path.lock().unwrap() =
        Some(("/art/".into(), std::time::Duration::from_secs(2)));
    apple
        .binding
        .cover_path(Some(&format!(
            "{}/art/Dawn_Album/600x600bb.jpg",
            apple.fake.base
        )))
        .await;
    wait_until("the cover request to arrive", || async {
        Ok((apple.fake.count("/art/") > 0).then_some(()))
    })
    .await?;
    let config = std::path::PathBuf::from(&*apple.node.config_path);
    let folder = cover_cache_root(&config, apple.record.0);
    apple.node.services.retire(apple.record).await;
    delete_service_library_folder(&config, apple.record.0).await?;
    // Apple's answer comes after the removal; it must not bring the folder back.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(
        !folder.exists(),
        "a cover made its folder again after the removal"
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_begun_before_a_sign_out_is_not_kept() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.delay_path.lock().unwrap() = Some((
        "/v1/me/library/artists".into(),
        std::time::Duration::from_millis(600),
    ));
    let read = apple.binding.artists();
    let switch = async {
        wait_until("the artists request to arrive", || async {
            Ok((apple.fake.count("/v1/me/library/artists") > 0).then_some(()))
        })
        .await?;
        apple.binding.sign_out();
        apple
            .binding
            .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
        Ok::<_, anyhow::Error>(())
    };
    let (read, switch) = tokio::join!(read, switch);
    // The read still answers whoever waited for it.
    read?;
    switch?;
    // What that read returned was not kept, so the next one asks Apple again.
    *apple.fake.delay_path.lock().unwrap() = None;
    let before = apple.fake.count("/v1/me/library/artists");
    apple.binding.artists().await?;
    assert_eq!(apple.fake.count("/v1/me/library/artists"), before + 1);
    // Read under the new account, it is kept.
    apple.binding.artists().await?;
    assert_eq!(apple.fake.count("/v1/me/library/artists"), before + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_old_list_answers_at_once_and_the_ui_hears_once_apple_changed_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    // The artists are read when the artist list is opened; "Late Night" is not there yet.
    let listed = async || -> Result<i32> {
        Ok(apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type: CollectionType::Artist,
            })
            .await?
            .groups
            .iter()
            .map(|group| group.count)
            .sum())
    };
    assert_eq!(listed().await?, 2);

    // Apple lists another artist, the kept list grows old, and Apple's next answer is held.
    apple
        .fake
        .more_artists
        .lock()
        .unwrap()
        .push(AppleFake::artist("r.art3", "Late Night"));
    apple
        .clock
        .advance(crate::apple_music_binding::SHORT_LIST_AGE);
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() =
        Some(("/v1/me/library/artists".into(), release.clone()));
    let reads = apple.fake.count("/v1/me/library/artists");
    let told = told_settled(&heard).await;
    // The page answers at once with the kept list, and Apple is asked again behind it, once.
    assert_eq!(listed().await?, 2);
    fake_asked(&apple, "/v1/me/library/artists", reads + 1).await?;
    assert_eq!(listed().await?, 2);
    assert_eq!(heard.count(CONTENT_CHANGED), told);

    // Apple's answer differs from what the page showed: the UI is told, and the list grows.
    release.notify_one();
    wait_until("the UI to hear the artists changed", || async {
        Ok((heard.count(CONTENT_CHANGED) > told).then_some(()))
    })
    .await?;
    assert_eq!(listed().await?, 3);
    assert_eq!(apple.fake.count("/v1/me/library/artists"), reads + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binding_made_afresh_answers_the_artist_list_from_disk_before_its_sign_in() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artists = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?;
    assert_eq!(
        artists.groups.iter().map(|group| group.count).sum::<i32>(),
        2
    );
    let config = std::path::PathBuf::from(&*apple.node.config_path);
    let lists = lists_folder(&config, apple.record.0);
    wait_until("the artists and songs to be kept on disk", || async {
        let kept = lists.join("artists.json").exists() && lists.join("songs.json").exists();
        Ok(kept.then_some(()))
    })
    .await?;

    // As after a restart, before the platform signs the library in again.
    let setup = fake_services(&apple.fake.base, &TestClock::new());
    let fresh = AppleMusicBinding::new(
        apple.record.0,
        |tokens| setup.apple_music_client(tokens),
        &config,
        Arc::new(Heard::default()),
        Arc::new(SystemClock),
    );
    assert!(!fresh.is_signed_in());
    let asked = apple.fake.total();
    let named: Vec<String> = fresh
        .named_artists()
        .await?
        .iter()
        .map(|artist| artist.name.clone())
        .collect();
    assert_eq!(named, ["Artist One", "Artist Two"]);
    assert_eq!(fresh.songs().await?.len(), 3);
    assert_eq!(apple.fake.total(), asked);

    // Signing out deletes them, and nothing answers until a sign-in.
    fresh.sign_out();
    assert!(!lists.exists());
    let error = fresh.artists().await.err().context("a signed-out read")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: apple.record.0.to_string(),
        })
    );
    apple.finish().await
}

/// An Apple Music library as the next start of the app finds it, before the platform signs it
/// in: the node starts again on the configuration a signed-in run left, with the library's
/// lists kept on disk.
struct Restarted {
    node: Arc<NodeContext>,
    heard: Arc<Heard>,
    fake: Arc<AppleFake>,
    record: LibraryRecordId,
    server: tokio::task::JoinHandle<()>,
    /// The run before, whose files the new node takes over.
    _before: Fixture,
}

/// Has `apple` keep the songs, the albums and the artists on disk, as the pages that show them
/// read them, and starts the node again on its configuration.
async fn restart_apple(apple: OpenApple) -> Result<Restarted> {
    for collection_type in [CollectionType::Album, CollectionType::Artist] {
        apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type,
            })
            .await?;
    }
    let lists = lists_folder(Path::new(&*apple.node.config_path), apple.record.0);
    wait_until(
        "the songs, albums and artists to be kept on disk",
        || async {
            let kept = ["songs", "albums", "artists"]
                .iter()
                .all(|name| lists.join(format!("{name}.json")).exists());
            Ok(kept.then_some(()))
        },
    )
    .await?;
    apple.node.shutdown().await?;
    let heard = Arc::new(Heard::default());
    let config = apple.fixture.global_params.config_path.clone();
    let node = node_on(&config, heard.clone(), &apple.fake.base).await?;
    let OpenApple {
        fixture,
        fake,
        record,
        server,
        ..
    } = apple;
    Ok(Restarted {
        node,
        heard,
        fake,
        record,
        server,
        _before: fixture,
    })
}

impl Restarted {
    /// Opens the library as the UI does, before any sign-in, and answers the UI's params and
    /// the binding the open made.
    async fn open(&self) -> Result<(Arc<crate::utils::GlobalParams>, Arc<AppleMusicBinding>)> {
        assert!(
            self.node
                .services
                .apple_music_binding(self.record)
                .is_none()
        );
        let ui =
            open_registered_library_record(self.node.clone(), self.record.0.to_string()).await?;
        let binding = self
            .node
            .services
            .apple_music_binding(self.record)
            .context("the binding the open made")?;
        Ok((ui, binding))
    }

    /// Signs the library in as the platform does.
    async fn sign_in(&self, user_token: &str) -> Result<()> {
        use crate::messages::{ServiceSignIn, SignInServiceLibraryRequest};
        use crate::utils::sign_in_service_library_on_node;

        sign_in_service_library_on_node(
            &SignInServiceLibraryRequest {
                record_id: self.record.0.to_string(),
                sign_in: ServiceSignIn::Platform {
                    user_token: user_token.into(),
                    developer_token: Some(DEVELOPER_TOKEN.into()),
                },
                call_id: None,
            },
            &self.node,
        )
        .await
    }

    async fn finish(self) -> Result<()> {
        self.server.abort();
        self.node.shutdown().await
    }
}

/// The songs the Tracks page lists, as id, track and whether each plays.
async fn listed_songs(
    ui: &crate::utils::GlobalParams,
) -> Result<Vec<(i32, String, AudioAvailability)>> {
    let page = dispatch_gui_request(
        ui,
        &FetchMediaFilesRequest {
            call_id: None,
            cursor: 0,
            page_size: 50,
            bake_cover_arts: false,
        },
    )
    .await?
    .context("the song page")?;
    Ok(page
        .media_files
        .into_iter()
        .map(|file| (file.id, file.track, file.audio_availability))
        .collect())
}

/// How many entries each group of the collection type has, summed.
async fn collection_count(
    ui: &crate::utils::GlobalParams,
    collection_type: CollectionType,
) -> Result<i32> {
    Ok(dispatch_gui_request(
        ui,
        &FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type,
        },
    )
    .await?
    .context("the group summary")?
    .groups
    .iter()
    .map(|group| group.count)
    .sum())
}

/// The home of the albums, artists and tracks, as entry names per group.
async fn home_names(ui: &crate::utils::GlobalParams) -> Result<Vec<Vec<String>>> {
    let mode = LibraryHomeListMode::Name;
    let queries = [
        LibraryHomeQuery::Albums(mode),
        LibraryHomeQuery::Artists(mode),
        LibraryHomeQuery::Tracks(mode),
    ];
    let response = dispatch_gui_request(
        ui,
        &ComplexQueryRequest {
            queries: queries
                .into_iter()
                .map(|query| ComplexQuery {
                    title: format!("{query:?}"),
                    query,
                })
                .collect(),
            call_id: None,
        },
    )
    .await?
    .context("the home")?;
    Ok(response
        .result
        .iter()
        .map(|group| {
            group
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
        .collect())
}

/// A read that needs Apple: a song the kept song list does not hold.
async fn unlisted_song(
    ui: &crate::utils::GlobalParams,
    binding: &AppleMusicBinding,
) -> Result<crate::messages::MediaFile> {
    let id = binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    Ok(
        dispatch_gui_request(ui, &FetchParsedMediaFileRequest { call_id: None, id })
            .await?
            .context("the unlisted song")?
            .file,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_library_opens_offline_from_what_it_kept_and_waits_for_its_sign_in() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let home_signed_in = home_names(&apple.fixture.global_params).await?;
    let restarted = restart_apple(apple).await?;
    // The device starts offline, with one song downloaded.
    restarted.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::from(["i.s2".to_owned()])),
    );
    let asked = restarted.fake.total();
    let (ui, binding) = restarted.open().await?;
    assert!(!binding.is_signed_in());

    // The pages answer from the lists kept on disk, the songs that cannot play offline greyed
    // out, and the browsing requests stay available.
    let songs = listed_songs(&ui).await?;
    assert_eq!(
        songs
            .iter()
            .map(|(_, track, playable)| (track.as_str(), *playable))
            .collect::<Vec<_>>(),
        [
            ("i.s1", AudioAvailability::Unavailable),
            ("i.s2", AudioAvailability::RemoteOnly),
            ("i.s3", AudioAvailability::Unavailable),
        ]
    );
    assert_eq!(
        dispatch_gui_request(&ui, &GetMediaFilesCountRequest { call_id: None })
            .await?
            .context("the song count")?
            .count,
        3
    );
    assert_eq!(collection_count(&ui, CollectionType::Album).await?, 2);
    assert_eq!(collection_count(&ui, CollectionType::Artist).await?, 2);
    // The home shows what it showed signed in.
    assert_eq!(home_names(&ui).await?, home_signed_in);
    assert_eq!(home_signed_in[0], ["Dawn Album", "Night Album"]);
    let capabilities = capability_snapshot(&restarted.node)?;
    for capability in [
        capabilities.fetch_media_files_request,
        capabilities.complex_query_request,
        capabilities.fetch_collection_group_summary_request,
    ] {
        assert_eq!(capability, RequestCapability::Available);
    }

    // A song plays on the host by its key, and is named from the kept list.
    match binding.playback_source("i.s2").await? {
        playback::player::ServiceTarget::Hosted(hosted) => assert_eq!(hosted.key, "i.s2"),
        _ => anyhow::bail!("an Apple Music song plays on the host"),
    }
    let (summary, _) = binding.item_metadata("i.s2", false).await?;
    assert_eq!(summary.title, "Second Wind");
    assert_eq!(restarted.fake.total(), asked);

    // A read that needs Apple fails as every request does offline.
    let error = unlisted_song(&ui, &binding)
        .await
        .err()
        .context("a read offline")?;
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable,
        "{error:#}"
    );
    assert_eq!(request_failure_cause(&error), None);
    // Online, it waits for the sign-in, which the UI takes up quietly.
    restarted.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::default(),
    );
    let error = unlisted_song(&ui, &binding)
        .await
        .err()
        .context("a read online before the sign-in")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: restarted.record.0.to_string(),
        })
    );
    assert!(
        listed_songs(&ui)
            .await?
            .iter()
            .all(|(_, _, playable)| *playable == AudioAvailability::RemoteOnly)
    );
    assert_eq!(home_names(&ui).await?, home_signed_in);
    assert_eq!(restarted.fake.total(), asked);
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_in_after_an_open_from_what_was_kept_refreshes_the_same_binding_and_ids()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let restarted = restart_apple(open_apple(dir.path()).await?).await?;
    let (ui, binding) = restarted.open().await?;
    let songs = listed_songs(&ui).await?;
    assert_eq!(collection_count(&ui, CollectionType::Artist).await?, 2);
    // The home shows what was kept.
    let home = home_names(&ui).await?;
    assert_eq!(home[1], ["Artist One", "Artist Two"]);
    assert_eq!(home[2].len(), 3);

    // Apple lists another artist by now, which the sign-in reads behind the kept list.
    restarted
        .fake
        .more_artists
        .lock()
        .unwrap()
        .push(AppleFake::artist("r.art3", "Late Night"));
    let (told, reads) = (
        told_settled(&restarted.heard).await,
        restarted.fake.count("/v1/me/library/songs"),
    );
    restarted.sign_in(USER_TOKEN).await?;
    let signed_in = restarted
        .node
        .services
        .apple_music_binding(restarted.record)
        .context("the signed-in binding")?;
    assert!(Arc::ptr_eq(&signed_in, &binding));
    assert!(binding.is_signed_in());
    wait_until("the UI to hear what Apple lists now", || async {
        Ok((restarted.heard.count(CONTENT_CHANGED) > told).then_some(()))
    })
    .await?;
    wait_until("the artist Apple added to be listed", || async {
        Ok((collection_count(&ui, CollectionType::Artist).await? == 3).then_some(()))
    })
    .await?;
    // The kept song list served before the sign-in is read again, once, and the songs keep
    // the ids the pages hold.
    fake_asked_at_least(&restarted, "/v1/me/library/songs", reads + 1).await?;
    assert_eq!(listed_songs(&ui).await?, songs);
    restarted.finish().await
}

/// Waits until Apple was asked for `prefix` at least `times` times.
async fn fake_asked_at_least(restarted: &Restarted, prefix: &str, times: usize) -> Result<()> {
    wait_until("Apple to be asked again", || async {
        Ok((restarted.fake.count(prefix) >= times).then_some(()))
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_account_signing_in_after_an_open_from_what_was_kept_replaces_the_old_lists()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let restarted = restart_apple(open_apple(dir.path()).await?).await?;
    let lists = lists_folder(Path::new(&*restarted.node.config_path), restarted.record.0);
    let (ui, binding) = restarted.open().await?;
    let tracks = |songs: Vec<(i32, String, AudioAvailability)>| -> Vec<String> {
        songs.into_iter().map(|(_, track, _)| track).collect()
    };
    assert_eq!(tracks(listed_songs(&ui).await?), ["i.s1", "i.s2", "i.s3"]);
    restarted.sign_in(USER_TOKEN).await?;

    // Other tokens may be another account's, which Apple does not say: the lists kept are read
    // again under them, and what the new account has replaces them, on disk too.
    restarted.fake.other_account.store(true, Ordering::SeqCst);
    restarted.sign_in(APPLE_OTHER_TOKEN).await?;
    wait_until("the pages to list what the new account has", || async {
        let listed = tracks(listed_songs(&ui).await?);
        Ok((listed == ["i.b-s1", "i.b-s2", "i.b-s3"]).then_some(()))
    })
    .await?;
    wait_until("the kept songs to be the new account's", || async {
        let kept = std::fs::read_to_string(lists.join("songs.json")).unwrap_or_default();
        Ok((kept.contains("i.b-s1") && !kept.contains("\"i.s1\"")).then_some(()))
    })
    .await?;
    assert!(Arc::ptr_eq(
        &restarted
            .node
            .services
            .apple_music_binding(restarted.record)
            .context("the binding")?,
        &binding
    ));
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_record_that_kept_nothing_opens_empty_and_its_reads_are_typed() -> Result<()> {
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter,
    };

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // An Apple Music record this device never signed in to, so nothing of it is kept.
    let unsigned = LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Never signed in".into(),
        library_id: None,
        protocol: ProtocolAdapter::AppleMusic,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let unsigned_id = unsigned.record_id;
    LibraryRegistry::update(Path::new(&*apple.node.config_path), |registry| {
        registry.libraries.push(unsigned);
        Ok(())
    })
    .await?;
    let asked = apple.fake.total();
    let ui = open_registered_library_record(apple.node.clone(), unsigned_id.0.to_string()).await?;
    let binding = apple
        .node
        .services
        .apple_music_binding(unsigned_id)
        .context("the binding the open made")?;
    assert!(!binding.is_signed_in());
    assert_eq!(ui.library_id, Some(unsigned_id.0));

    let waits_for_sign_in = |error: anyhow::Error| {
        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::ServiceSignInRequired {
                record_id: unsigned_id.0.to_string(),
            }),
            "{error:#}"
        );
        assert_eq!(
            classify(&error),
            crate::messages::RequestErrorKind::PermissionDenied
        );
    };
    waits_for_sign_in(listed_songs(&ui).await.err().context("the songs")?);
    waits_for_sign_in(
        dispatch_gui_request(&ui, &GetMediaFilesCountRequest { call_id: None })
            .await
            .err()
            .context("the song count")?,
    );
    // The home has nothing to show, and says what it waits for.
    waits_for_sign_in(home_names(&ui).await.err().context("the home")?);

    // Offline, the same reads fail as every request does offline.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::new()),
    );
    let error = listed_songs(&ui).await.err().context("the songs offline")?;
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable,
        "{error:#}"
    );
    assert_eq!(apple.fake.total(), asked);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unreachable_library_gives_no_color_and_an_unsigned_apple_item_waits_for_it()
-> Result<()> {
    use database::credentials::{Credential, Secret};
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter, SubsonicOpenMode,
    };
    use std::sync::atomic::AtomicUsize;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // A Subsonic server that would be probed if a binding were made for its library.
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let server = Router::new().fallback(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::INTERNAL_SERVER_ERROR }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let subsonic_server = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let credential = apple
        .node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"unused".to_vec()),
        })
        .await?;
    let record = |protocol| LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Closed service".into(),
        library_id: None,
        protocol,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let subsonic = record(ProtocolAdapter::SubsonicTranslate {
        endpoint,
        credential,
        mode: SubsonicOpenMode::PlayOnThisDevice,
    });
    // An Apple library this device is not signed in to.
    let unsigned = record(ProtocolAdapter::AppleMusic);
    let (subsonic_id, unsigned_id) = (subsonic.record_id, unsigned.record_id);
    LibraryRegistry::update(Path::new(&*apple.node.config_path), |registry| {
        registry.libraries.push(subsonic);
        registry.libraries.push(unsigned);
        Ok(())
    })
    .await?;
    let ui = Arc::clone(&apple.ui);
    let item = |record: LibraryRecordId| PlayingItemRef::LibraryTrack {
        library_id: record.0.to_string(),
        track: "t".into(),
    };

    // The server of a closed Subsonic library answers with a failure, which gives no color.
    let color = dispatch_gui_request(
        &ui,
        &GetPrimaryColorByTrackIdRequest {
            call_id: None,
            item: Some(item(subsonic_id)),
        },
    )
    .await?
    .context("color")?;
    assert_eq!(color.primary_color, None);
    // Reaching the library for its cover is the only connection made.
    let connections = hits.load(Ordering::SeqCst);
    assert!(connections > 0);
    // The color of an Apple item whose library is not signed in here fails as a typed failure,
    // without a lookup elsewhere.
    let color = dispatch_gui_request(
        &ui,
        &GetPrimaryColorByTrackIdRequest {
            call_id: None,
            item: Some(item(unsigned_id)),
        },
    )
    .await
    .err()
    .context("the color of an unsigned Apple item")?;
    assert_eq!(
        request_failure_cause(&color),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: unsigned_id.0.to_string(),
        }),
        "{color:#}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), connections);
    subsonic_server.abort();
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_like_of_an_item_whose_library_cannot_answer_fails_as_a_typed_failure() -> Result<()> {
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter,
    };

    use crate::messages::{GetLikedRequest, RequestErrorKind};

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // An Apple library this device is not signed in to yet, as at startup before the
    // platform's silent sign-in, and a Subsonic library whose server cannot be reached.
    let unsigned = LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Not signed in".into(),
        library_id: None,
        protocol: ProtocolAdapter::AppleMusic,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let unsigned_id = unsigned.record_id;
    LibraryRegistry::update(Path::new(&*apple.node.config_path), |registry| {
        registry.libraries.push(unsigned);
        Ok(())
    })
    .await?;
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let item = |record: LibraryRecordId| {
        Some(PlayingItemRef::LibraryTrack {
            library_id: record.0.to_string(),
            track: "t1".into(),
        })
    };
    let ui = Arc::clone(&apple.ui);

    for params in [&ui, &apple.fixture.global_params] {
        let likes = |record| {
            let item = item(record);
            async move {
                [
                    dispatch_gui_request(
                        params,
                        &GetLikedRequest {
                            item: item.clone(),
                            call_id: None,
                        },
                    )
                    .await
                    .err(),
                    dispatch_gui_request(
                        params,
                        &SetLikedRequest {
                            item,
                            liked: true,
                            call_id: None,
                        },
                    )
                    .await
                    .err(),
                ]
            }
        };
        // The Apple item's library has no binding to ask before its sign-in, which the UI takes
        // up as such.
        for failure in likes(unsigned_id).await {
            let failure = failure.context("a like before the sign-in")?;
            assert_eq!(
                classify(&failure),
                RequestErrorKind::PermissionDenied,
                "{failure:#}"
            );
            assert_eq!(
                request_failure_cause(&failure),
                Some(RequestFailureCause::ServiceSignInRequired {
                    record_id: unsigned_id.0.to_string(),
                })
            );
        }
        // The Subsonic item's library is out of reach for now.
        for failure in likes(subsonic).await {
            let failure = failure.context("a like with the server out of reach")?;
            assert_eq!(
                classify(&failure),
                RequestErrorKind::Unavailable,
                "{failure:#}"
            );
        }
    }
    apple.finish().await
}

/// Keeps what the player hands to the platform hosts.
#[derive(Default)]
struct RecordedHost(Mutex<Vec<playback::hosted::HostCommand>>);

impl playback::hosted::HostBridge for RecordedHost {
    fn send(&self, command: playback::hosted::HostCommand) {
        self.0.lock().unwrap().push(command);
    }
}

impl RecordedHost {
    fn sent(&self) -> Vec<playback::hosted::HostCommand> {
        self.0.lock().unwrap().clone()
    }
}

/// A track of the fixture's own library, scanned from the bundled startup sound.
async fn local_track(fixture: &Fixture, name: &str) -> Result<PlayingItemRef> {
    use database::actions::file::file_uuids_by_id;

    let library = fixture.global_params.test_library();
    let target = Path::new(&library.media_root).join(name);
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        &target,
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
    let id = media_files::Entity::find()
        .filter(Column::FileName.eq(name))
        .one(&*library.main_db)
        .await?
        .context("the scanned track")?
        .id;
    let uuid = *file_uuids_by_id(&library.main_db, &[id])
        .await?
        .get(&id)
        .context("the scanned track's uuid")?;
    Ok(PlayingItemRef::LibraryTrack {
        library_id: library.library_id.to_string(),
        track: uuid.to_string(),
    })
}

/// A load of `items` in place of the queue, from the first at `position` seconds.
fn load_items(
    items: Vec<PlayingItemRef>,
    play: crate::messages::LoadPlay,
    position: f64,
) -> crate::messages::LoadResolvedItemsRequest {
    crate::messages::LoadResolvedItemsRequest {
        items,
        start_index: Some(0),
        mode: crate::messages::PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: Some(crate::messages::PlaybackMode::Sequential),
        position_seconds: Some(position),
        play,
        call_id: None,
    }
}

/// Waits for the player to show what `check` asks for, and returns its status then.
async fn player_reaches(
    player: &Arc<tokio::sync::Mutex<dyn playback::player::Playable>>,
    check: impl Fn(&playback::player::PlayerStatus) -> bool,
) -> Result<playback::player::PlayerStatus> {
    let check = &check;
    wait_until("the player to reach the state", || async move {
        let status = player.lock().await.get_status();
        Ok(check(&status).then_some(status))
    })
    .await
}

/// Opens the fixture's own library in the UI in place of the Apple Music one.
async fn open_own_library(apple: &OpenApple) -> Result<()> {
    let registry = LibraryRegistry::load(Path::new(&*apple.node.config_path))?;
    let record = registry
        .libraries
        .iter()
        .find(|record| matches!(record.protocol, ProtocolAdapter::Native))
        .context("the fixture's own library")?;
    super::open_registered_library_record(apple.node.clone(), record.record_id.0.to_string())
        .await?;
    Ok(())
}

/// The library item `item` names.
fn library_item(item: &PlayingItemRef) -> Result<playback::player::PlayingItem> {
    let PlayingItemRef::LibraryTrack { library_id, track } = item else {
        anyhow::bail!("not a library item: {item:?}");
    };
    Ok(playback::player::PlayingItem::LibraryTrack {
        library_id: library_id.parse()?,
        track: track.clone(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_tracks_play_on_the_host_beside_local_ones_and_stay_on_this_device() -> Result<()> {
    use ::visualizer::VisualizerInputs;
    use playback::hosted::{HostAction, HostEvent};
    use playback::player::PlaybackState;

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::{
        CastLibraryState, CastResolvedItemsRequest, LoadPlay, SessionSnapshotRequest,
    };

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "beside-apple.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    let inputs = || apple.node.visualizer().status().inputs;
    assert_eq!(inputs(), VisualizerInputs::Full);

    apple
        .run(&load_items(
            vec![song.clone(), local.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;
    // The host gets the Apple track by its key, from the start; the local track after it is
    // not the host's to play.
    let sent = host.sent();
    let [hand_over] = sent.as_slice() else {
        anyhow::bail!("one hand-over, not {sent:?}");
    };
    assert_eq!(hand_over.host, APPLE_MUSIC_HOST);
    assert!(
        matches!(
            hand_over.actions.as_slice(),
            [
                HostAction::Set { ids },
                HostAction::Skip { index: 0, offset_ms: 0 },
                HostAction::SetGain { .. },
                HostAction::Start,
            ] if ids == &["i.s1".to_owned()]
        ),
        "{hand_over:?}"
    );
    let apple_item = library_item(&song)?;
    player_reaches(&player, |status| {
        status.item.as_ref() == Some(&apple_item)
            && status.hosted_by.as_deref() == Some(APPLE_MUSIC_HOST)
            && status.state == PlaybackState::Playing
    })
    .await?;
    wait_until("the visualizer to lose its inputs", || async {
        Ok((inputs() == VisualizerInputs::None).then_some(()))
    })
    .await?;

    // The queue holds an Apple track, so it cannot be handed off or cast elsewhere.
    let snapshot = apple
        .run(&SessionSnapshotRequest { call_id: None })
        .await
        .err()
        .context("a snapshot of a queue with an Apple track")?;
    assert!(
        matches!(
            snapshot.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ),
        "{snapshot:#}"
    );
    let cast = |items: Vec<PlayingItemRef>| CastResolvedItemsRequest {
        fingerprint: "not-a-paired-renderer".into(),
        load: load_items(items, LoadPlay::Play, 0.0),
        call_id: None,
    };
    let refused = apple
        .run(&cast(vec![local.clone(), song.clone()]))
        .await
        .err()
        .context("a cast of an Apple track")?;
    assert_eq!(
        request_failure_cause(&refused),
        Some(RequestFailureCause::CastNotPossible {
            library_id: apple.record.0.to_string(),
            state: CastLibraryState::NotHeld,
        })
    );
    // A local track and a Subsonic one, whose library other devices may reach, are not refused
    // for that: the cast only fails to find the renderer.
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let subsonic_track = PlayingItemRef::LibraryTrack {
        library_id: subsonic.0.to_string(),
        track: "s1".into(),
    };
    for items in [vec![local.clone()], vec![subsonic_track]] {
        let unknown = apple
            .run(&cast(items))
            .await
            .err()
            .context("a cast to a renderer that is not paired")?;
        assert!(
            matches!(
                unknown.downcast_ref::<database::error::ExpectedError>(),
                Some(database::error::ExpectedError::NotFound { what: "renderer" })
            ),
            "{unknown:#}"
        );
    }

    // The host plays the track to its end, and the local track decodes here.
    let revision = hand_over.revision;
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxStatus {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            current_index: Some(0),
            current_id: Some("i.s1".into()),
            playing: true,
            position_ms: 1_000,
        });
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxEnded {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            index: None,
        });
    let local_item = library_item(&local)?;
    let playing = player_reaches(&player, |status| {
        status.item.as_ref() == Some(&local_item)
            && status.hosted_by.is_none()
            && status.state == PlaybackState::Playing
    })
    .await?;
    assert!(
        playing
            .path
            .as_deref()
            .is_some_and(|path| path.ends_with("beside-apple.ogg")),
        "{:?}",
        playing.path
    );
    let sent = host.sent();
    assert!(
        matches!(
            sent.last().map(|command| command.actions.as_slice()),
            Some([HostAction::Stop, HostAction::Clear])
        ),
        "{sent:?}"
    );
    wait_until("the visualizer to get its inputs back", || async {
        Ok((inputs() == VisualizerInputs::Full).then_some(()))
    })
    .await?;
    player.lock().await.stop();
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_player_bar_names_the_track_the_host_moves_on_to_by_itself() -> Result<()> {
    use playback::hosted::{HostAction, HostEvent};
    use playback::player::PlaybackState;

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::{LoadPlay, PlaybackStatus};

    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let first = song_ref(&apple, "i.s1");
    let second = song_ref(&apple, "i.s2");
    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    let titles = || {
        heard
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "PlaybackStatus")
            .filter_map(|(_, payload)| rinf::deserialize::<PlaybackStatus>(payload).ok())
            .map(|status| (status.title, status.duration))
            .collect::<Vec<_>>()
    };

    apple
        .run(&load_items(
            vec![first.clone(), second.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;
    let first_item = library_item(&first)?;
    player_reaches(&player, |status| {
        status.item.as_ref() == Some(&first_item) && status.state == PlaybackState::Playing
    })
    .await?;
    wait_until("the player bar to name the first track", || async {
        Ok(titles()
            .last()
            .is_some_and(|(title, _)| title.as_deref() == Some("First Light"))
            .then_some(()))
    })
    .await?;

    // The host holds the second track behind the first, and moves on to it by itself.
    let revision = wait_until("the host to hold the second track", || async {
        Ok(host.sent().iter().rev().find_map(|command| {
            command
                .actions
                .iter()
                .any(|action| match action {
                    HostAction::Set { ids } | HostAction::Add { ids } => {
                        ids.iter().any(|id| id == "i.s2")
                    }
                    _ => false,
                })
                .then_some(command.revision)
        }))
    })
    .await?;
    let latest = host
        .sent()
        .last()
        .map_or(revision, |command| command.revision);
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxStatus {
            host: APPLE_MUSIC_HOST.into(),
            revision: latest,
            current_index: Some(1),
            current_id: Some("i.s2".into()),
            playing: true,
            position_ms: 1_000,
        });
    let second_item = library_item(&second)?;
    player_reaches(&player, |status| {
        status.item.as_ref() == Some(&second_item) && status.index == Some(1)
    })
    .await?;
    wait_until("the player bar to name the second track", || async {
        Ok(titles()
            .last()
            .is_some_and(|(title, _)| title.as_deref() == Some("Second Wind"))
            .then_some(()))
    })
    .await?;
    player.lock().await.stop();
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_hosted_track_gives_way_to_a_local_one_and_the_visualizer_inputs_follow()
-> Result<()> {
    use ::visualizer::VisualizerInputs;
    use playback::hosted::{HostEvent, HostFailure};
    use playback::player::{PlaybackState, UnavailableReason};

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::LoadPlay;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "after-failure.ogg").await?;
    let inputs = || apple.node.visualizer().status().inputs;

    // A report without a track, as between two hosted tracks, leaves the inputs as they are.
    let file =
        Path::new(&apple.fixture.global_params.test_library().media_root).join("after-failure.ogg");
    for (path, expected) in [
        (
            Some(Path::new("hosted://appleMusic/i.s1")),
            VisualizerInputs::None,
        ),
        (None, VisualizerInputs::None),
        (
            Some(Path::new("hosted://appleMusic/i.s2")),
            VisualizerInputs::None,
        ),
        (Some(file.as_path()), VisualizerInputs::Full),
        (None, VisualizerInputs::Full),
    ] {
        apple.node.note_local_track(path, true)?;
        assert_eq!(inputs(), expected, "after {path:?}");
    }

    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    let song = song_ref(&apple, "i.s1");
    apple
        .run(&load_items(
            vec![song.clone(), local.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;
    let revision = host.sent().first().context("the hand-over")?.revision;
    wait_until("the visualizer to lose its inputs", || async {
        Ok((inputs() == VisualizerInputs::None).then_some(()))
    })
    .await?;

    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxItemFailed {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            index: 0,
            reason: HostFailure::Unplayable,
        });
    let local_item = library_item(&local)?;
    let playing = player_reaches(&player, |status| {
        status.item.as_ref() == Some(&local_item) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(
        playing.unavailable_reasons.get(&library_item(&song)?),
        Some(&UnavailableReason::TrackGone)
    );
    wait_until("the visualizer to get its inputs back", || async {
        Ok((inputs() == VisualizerInputs::Full).then_some(()))
    })
    .await?;
    player.lock().await.stop();
    apple.finish().await
}

/// What the UI was told `item` supports, last before the first status that names it.
async fn told_before_its_status(
    heard: &Heard,
    item: &PlayingItemRef,
) -> Result<crate::messages::PlayingItemCapabilities> {
    use crate::messages::{PlaybackStatus, PlayingItemCapabilities};

    let status = wait_until("a status that names the item", || async {
        Ok(heard
            .placed::<PlaybackStatus>("PlaybackStatus")
            .into_iter()
            .find(|(_, status)| status.item_request.as_ref() == Some(item))
            .map(|(place, _)| place))
    })
    .await?;
    heard
        .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
        .into_iter()
        .rev()
        .find(|(place, told)| *place < status && told.item.as_ref() == Some(item))
        .map(|(_, told)| told)
        .context("what the item supports, before its status")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_what_the_playing_item_supports_before_the_status_that_names_it() -> Result<()>
{
    use playback::hosted::HostEvent;

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::LoadPlay;

    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    let local = local_track(&apple.fixture, "after-apple.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    apple
        .run(&load_items(
            vec![song.clone(), local.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;

    // Apple Music answers the color of its track, and keeps no likes or lyrics for it.
    let told = told_before_its_status(&heard, &song).await?;
    assert_eq!(
        told.get_primary_color_by_track_id_request,
        RequestCapability::Available,
        "{told:?}"
    );
    for capability in [
        told.set_liked_request,
        told.get_lyric_by_track_id_request,
        told.get_liked_request,
    ] {
        assert_eq!(capability, RequestCapability::Hidden, "{told:?}");
    }

    // The host plays its track to the end, and the local track after it has everything.
    let revision = host.sent().first().context("the hand-over")?.revision;
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxEnded {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            index: None,
        });
    let told = told_before_its_status(&heard, &local).await?;
    for capability in [
        told.set_liked_request,
        told.get_liked_request,
        told.get_lyric_by_track_id_request,
        told.get_primary_color_by_track_id_request,
    ] {
        assert_eq!(capability, RequestCapability::Available, "{told:?}");
    }
    player.lock().await.stop();
    apple.finish().await
}

/// What the session declares for a file played by its path (`item_capabilities::file_slots`)
/// is what asking about it does: it has no liked state, and its lyrics and color are read
/// through the local library open in the UI, without which nothing about it is answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_played_by_its_path_is_answered_only_through_a_local_library() -> Result<()> {
    use crate::messages::{GetLikedRequest, RequestErrorKind};

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let file = Some(PlayingItemRef::IndependentFile {
        path: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/startup_0.ogg")
            .to_string_lossy()
            .into_owned(),
    });
    let asked = |params: Arc<crate::utils::GlobalParams>| {
        let file = file.clone();
        async move {
            [
                dispatch_gui_request(
                    &params,
                    &SetLikedRequest {
                        item: file.clone(),
                        liked: true,
                        call_id: None,
                    },
                )
                .await
                .map(|liked| liked.map(|liked| liked.liked)),
                dispatch_gui_request(
                    &params,
                    &GetLikedRequest {
                        item: file.clone(),
                        call_id: None,
                    },
                )
                .await
                .map(|liked| liked.map(|liked| liked.liked)),
                dispatch_gui_request(
                    &params,
                    &GetLyricByTrackIdRequest {
                        item: file.clone(),
                        call_id: None,
                    },
                )
                .await
                .map(|_| None),
                dispatch_gui_request(
                    &params,
                    &GetPrimaryColorByTrackIdRequest {
                        item: file,
                        call_id: None,
                    },
                )
                .await
                .map(|_| None),
            ]
        }
    };

    // Through the open Apple Music library, which has no local library, nothing is answered.
    let ui = Arc::clone(&apple.ui);
    for answer in asked(ui).await {
        let failure = answer.err().context("a request about the file")?;
        assert_ne!(
            classify(&failure),
            RequestErrorKind::Internal,
            "{failure:#}"
        );
    }
    // Through a local library, the file has no like to set or to show, and its lyrics and color
    // are read.
    let [set, get, lyrics, color] = asked(apple.fixture.global_params.clone()).await;
    let refused = set.err().context("a like of the file")?;
    assert_eq!(
        classify(&refused),
        RequestErrorKind::InvalidInput,
        "{refused:#}"
    );
    assert_eq!(get?, Some(false));
    lyrics?;
    color?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_file_follows_the_library_the_ui_opens_and_closes() -> Result<()> {
    use crate::messages::PlayingItemCapabilities;

    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let file = PlayingItemRef::IndependentFile {
        path: path.to_string_lossy().into_owned(),
    };
    // The file is queued by its path, as a file opened from outside a library is, and stays
    // paused: the player reports nothing more.
    apple
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    playback::player::PlayingItem::IndependentFile(path.to_string_lossy().into()),
                    playback::player::PlaybackSource::Local(path.clone()),
                )],
                playback::strategies::AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    apple
        .run(&crate::messages::LoadRequest {
            index: 0,
            call_id: None,
        })
        .await?;
    let each = |told: &PlayingItemCapabilities| {
        [
            told.set_liked_request,
            told.get_liked_request,
            told.get_lyric_by_track_id_request,
            told.get_primary_color_by_track_id_request,
        ]
    };
    use RequestCapability::{Available, Hidden};
    // With a streaming library open there is no local library to read it through.
    let told = told_before_its_status(&heard, &file).await?;
    assert_eq!(each(&told), [Hidden; 4], "{told:?}");
    let last_told = || {
        heard
            .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
            .into_iter()
            .map(|(_, told)| told)
            .rfind(|told| told.item.as_ref() == Some(&file))
    };

    // The UI opens the local library: the file's lyrics and color can be read through it.
    let registry = LibraryRegistry::load(Path::new(&*apple.node.config_path))?;
    let record = registry
        .libraries
        .iter()
        .find(|record| matches!(record.protocol, ProtocolAdapter::Native))
        .context("the fixture's own library")?;
    let ui =
        super::open_registered_library_record(apple.node.clone(), record.record_id.0.to_string())
            .await?;
    wait_until("the UI to hear the file can be read", || async {
        Ok(last_told().filter(|told| each(told) == [Hidden, Hidden, Available, Available]))
    })
    .await?;

    // The UI closes it, and nothing about the file can be read again.
    apple.node.close_ui(&ui.main_token).await;
    wait_until("the UI to hear the file cannot be read", || async {
        Ok(last_told().filter(|told| each(told) == [Hidden; 4]))
    })
    .await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_an_apple_item_supports_nothing_once_its_library_goes() -> Result<()> {
    use crate::messages::{LoadPlay, PlayingItemCapabilities, RemoveLibraryRecordRequest};

    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    let song = song_ref(&apple, "i.s1");
    apple
        .run(&load_items(vec![song.clone()], LoadPlay::Pause, 0.0))
        .await?;
    let told = told_before_its_status(&heard, &song).await?;
    assert_eq!(
        told.get_primary_color_by_track_id_request,
        RequestCapability::Available
    );
    let last_told = || {
        heard
            .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
            .into_iter()
            .map(|(_, told)| told)
            .rfind(|told| told.item.as_ref() == Some(&song))
    };

    // Another library is opened, and the Apple Music library is removed with its sign-in: no
    // sign-in brings it back, and nothing about the track can be answered.
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    apple
        .node
        .open_service_catalog(
            Some(subsonic),
            Arc::new(LyricService::with_slots(subsonic.0, subsonic_slots())),
        )
        .await?;
    let removal = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: apple.record.0.to_string(),
    };
    <RemoveLibraryRecordRequest as crate::Signal>::handle(
        &removal,
        apple.node.clone(),
        None,
        &removal,
    )
    .await?;
    let told = wait_until("the UI to hear the track has nothing to answer", || async {
        Ok(last_told()
            .filter(|told| told.get_primary_color_by_track_id_request == RequestCapability::Hidden))
    })
    .await?;
    for capability in [
        told.set_liked_request,
        told.get_liked_request,
        told.get_lyric_by_track_id_request,
    ] {
        assert_eq!(capability, RequestCapability::Hidden, "{told:?}");
    }
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_each_change_of_the_visualizer_inputs_once() -> Result<()> {
    use crate::messages::{VisualizerInputs, VisualizerInputsChanged};

    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    let told = || {
        heard
            .placed::<VisualizerInputsChanged>("VisualizerInputsChanged")
            .into_iter()
            .map(|(_, changed)| changed.inputs)
            .collect::<Vec<_>>()
    };
    let before = told();
    // Whatever was sent while the library opened left this device's player feeding everything.
    assert_eq!(before.last(), Some(&VisualizerInputs::Full));

    let file = Path::new(&apple.fixture.global_params.test_library().media_root).join("a-file.ogg");
    for path in [
        Some(Path::new("hosted://appleMusic/i.s1")),
        None,
        Some(Path::new("hosted://appleMusic/i.s2")),
        Some(file.as_path()),
        None,
        Some(file.as_path()),
    ] {
        apple.node.note_local_track(path, path.is_some())?;
    }
    // The UI heard of the hosted track and of the file after it, and of nothing in between.
    assert_eq!(
        told()[before.len()..],
        [VisualizerInputs::None, VisualizerInputs::Full]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restored_apple_tracks_keep_their_place_before_the_sign_in_and_are_named_after_it()
-> Result<()> {
    use crate::messages::{
        LoadPlay, PlaybackStatus, PlaylistUpdate, ServiceSignIn, SignInServiceLibraryRequest,
    };
    use crate::utils::sign_in_service_library_on_node;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "after-restore.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let apple_item = library_item(&song)?;
    let at_its_place = |status: &playback::player::PlayerStatus| {
        status.item.as_ref() == Some(&apple_item)
            && status.index == Some(0)
            && status.position == Duration::from_secs(30)
            && status.ready
    };
    apple
        .run(&load_items(
            vec![song.clone(), song_ref(&apple, "i.s2"), local],
            LoadPlay::Pause,
            30.0,
        ))
        .await?;
    player_reaches(&apple.fixture.global_params.player, at_its_place).await?;

    // The app quits, and starts again with the hub not signed in to Apple Music.
    apple.node.shutdown().await?;
    let heard = Arc::new(Heard::default());
    let config = apple.fixture.global_params.config_path.clone();
    let node = node_on(&config, heard.clone(), &apple.fake.base).await?;
    assert!(node.services.apple_music_binding(apple.record).is_none());
    let player = node.session()?.player.clone();
    player_reaches(&player, at_its_place).await?;
    let settled = player.lock().await.status_after_pending().await;
    assert!(at_its_place(&settled), "{settled:?}");
    assert!(settled.unavailable_reasons.is_empty(), "{settled:?}");

    let decoded = |name: &str| -> Vec<Vec<u8>> {
        heard
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(heard, _)| heard == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    };
    let named = || {
        decoded("PlaybackStatus")
            .iter()
            .filter_map(|payload| rinf::deserialize::<PlaybackStatus>(payload).ok())
            .any(|status| status.title.as_deref() == Some("First Light"))
    };
    let sign_in = || {
        let request = SignInServiceLibraryRequest {
            record_id: apple.record.0.to_string(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        };
        let node = node.clone();
        async move { sign_in_service_library_on_node(&request, &node).await }
    };

    let lookups = || {
        apple
            .fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with("/v1/me/library/songs?ids="))
            .map(|(target, _)| target.clone())
            .collect::<Vec<_>>()
    };

    // A first sign-in finds Apple unable to name the songs: the binding is there, but the
    // tracks stay unnamed.
    *apple.fake.respond_path.lock().unwrap() =
        Some(("/v1/me/library/songs".into(), 404, "{}".into()));
    let updates = decoded("PlaylistUpdate").len();
    sign_in().await?;
    // This sign-in asks Apple for the queue's songs together and for the playing song on its
    // own, for the player bar. Both are waited for, so neither counts as a read of the next
    // sign-in.
    wait_until("the first sign-in's reads", || async {
        let lookups = lookups();
        let asked = |ids: &str| {
            lookups
                .iter()
                .any(|target| target.starts_with(&format!("/v1/me/library/songs?ids={ids}&")))
        };
        Ok((asked("i.s1,i.s2") && asked("i.s1")).then_some(()))
    })
    .await?;
    wait_until("the queue to be sent after the first sign-in", || async {
        Ok((decoded("PlaylistUpdate").len() > updates).then_some(()))
    })
    .await?;
    assert!(!named());
    *apple.fake.respond_path.lock().unwrap() = None;

    // Once the app signs the hub in again, the player bar and the queue name the tracks,
    // which are read together.
    let before = lookups().len();
    sign_in().await?;
    wait_until("the player bar to name the track", || async {
        Ok(named().then_some(()))
    })
    .await?;
    wait_until("the queue to name the tracks", || async {
        Ok(decoded("PlaylistUpdate")
            .iter()
            .filter_map(|payload| rinf::deserialize::<PlaylistUpdate>(payload).ok())
            .any(|update| {
                let titles: Vec<&str> = update.items.iter().map(|row| row.title.as_str()).collect();
                titles.starts_with(&["First Light", "Second Wind"])
                    && update.items.iter().all(|row| row.unavailable.is_none())
            })
            .then_some(()))
    })
    .await?;
    let lookups = lookups();
    assert_eq!(lookups.len(), before + 1, "{lookups:?}");
    assert!(
        lookups[before].starts_with("/v1/me/library/songs?ids=i.s1,i.s2"),
        "{lookups:?}"
    );
    assert!(at_its_place(&player.lock().await.get_status()));
    apple.server.abort();
    node.shutdown().await
}

/// How many bytes this device sends the renderer behind `proxy` for what `sends` sends, measured
/// up to the answer of `barrier`, a request known to reach the renderer on the same link: once it
/// is answered, every byte sent before it has passed the proxy. The bytes of `barrier` itself are
/// not counted.
async fn bytes_sent<S, B>(
    proxy: &super::player_session_tests::TestTcpProxy,
    sends: S,
    barrier: impl AsyncFn() -> Result<B>,
) -> Result<usize>
where
    S: Future,
{
    let before = proxy.request_bytes();
    barrier().await?;
    let barrier_bytes = proxy.request_bytes() - before;
    let before = proxy.request_bytes();
    sends.await;
    barrier().await?;
    Ok(proxy.request_bytes() - before - barrier_bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_items_stay_on_this_device_while_a_renderer_is_attached() -> Result<()> {
    use super::player_session_tests::TestTcpProxy;
    use crate::messages::{
        CastLibraryState, ListRenderersRequest, LoadPlay, RendererAvailability, RequestErrorKind,
        SessionSnapshotRequest,
    };
    use crate::request_owner::CurrentPlayer;
    use crate::server::api::fetch_device_info_at_port;

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let apple = open_apple(controller_dir.path()).await?;
    let renderer = super::fixture(renderer_dir.path()).await?;
    let renderer_address = super::paired_full(&renderer, &apple.fixture).await?;
    // Only the renderer's link passes the proxy, through which the test sees what this device
    // sends the renderer.
    apple.fixture.stop_worker().await?;
    renderer.stop_worker().await?;
    let proxy = TestTcpProxy::new(renderer_address).await?;
    let renderer_info = super::info(&renderer).await?;
    let validator = &apple.fixture.global_params.cert_validator;
    validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", proxy.listen_addr.port())],
        )
        .await?;
    let client_validator = validator.read().await.clone();
    wait_until("the renderer to answer", || async {
        let ready = fetch_device_info_at_port(
            "127.0.0.1",
            renderer_address.port(),
            Arc::new(Arc::new(client_validator.clone()).into_client_config()),
        )
        .await
        .is_ok();
        Ok(ready.then_some(()))
    })
    .await?;
    let apple_library = apple.record.0.to_string();

    // The renderer is not asked about the Apple library: it is answered here, ahead of a
    // library the renderer would be asked about.
    let local_library = apple
        .fixture
        .global_params
        .library_id
        .context("the local library")?
        .to_string();
    let renderers = apple
        .run(&ListRenderersRequest {
            call_id: None,
            library_ids: vec![local_library, apple_library.clone()],
        })
        .await?
        .renderers;
    let candidate = renderers
        .iter()
        .find(|candidate| candidate.fingerprint == renderer_info.fingerprint)
        .context("the renderer among the candidates")?;
    assert_eq!(candidate.availability, RendererAvailability::CantPlay);
    assert_eq!(
        candidate.blocked_library_id.as_deref(),
        Some(apple_library.as_str())
    );
    assert_eq!(candidate.library_state, Some(CastLibraryState::NotHeld));

    // An open service library keeps the player on this device, so another library is opened
    // first. With the renderer as the player, a load of an Apple track, as from the queue,
    // never leaves this device.
    open_own_library(&apple).await?;
    apple
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert!(matches!(
        apple.node.current_player()?,
        CurrentPlayer::RunePeer { link: Some(_), .. }
    ));
    let refused = apple
        .run(&load_items(
            vec![song_ref(&apple, "i.s1")],
            LoadPlay::Play,
            0.0,
        ))
        .await
        .err()
        .context("a load of an Apple track on the renderer")?;
    assert_eq!(
        request_failure_cause(&refused),
        Some(RequestFailureCause::CastNotPossible {
            library_id: apple_library,
            state: CastLibraryState::NotHeld,
        })
    );
    // Any other request that would name the library on the renderer, as a mix whose fallback
    // items are Apple tracks, is refused as unavailable, not as a cast.
    let remote = crate::request_owner::Implementation::RunePeer {
        link: apple.node.renderer_link()?,
        reconnecting: None,
    };
    let mix = ResolveMixQueryRequest {
        queries: Vec::new(),
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        fallback_playing_items: vec![song_ref(&apple, "i.s1")],
        excluded: ExcludedItems::default(),
        call_id: None,
    };
    let unavailable = remote
        .run(&apple.fixture.global_params, &mix)
        .await
        .err()
        .context("a mix naming an Apple track asked of the renderer")?;
    assert!(
        matches!(
            unavailable.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable {
                what: "library kept on this device"
            })
        ),
        "{unavailable:#}"
    );
    // The color of an Apple track names the library too, but it is answered here, from the
    // covers the library keeps, which hold none for it yet.
    let color = GetPrimaryColorByTrackIdRequest {
        item: Some(song_ref(&apple, "i.s1")),
        call_id: None,
    };
    let to_renderer = async || {
        remote
            .run(
                &apple.fixture.global_params,
                &SessionSnapshotRequest { call_id: None },
            )
            .await
    };
    let mut answered = None;
    let sent = bytes_sent(
        &proxy,
        async {
            answered = Some(remote.run(&apple.fixture.global_params, &color).await);
        },
        to_renderer,
    )
    .await?;
    let answered = answered
        .context("the color asked")??
        .context("the color of an Apple track asked of the renderer")?;
    assert_eq!(answered.primary_color, None);
    assert_eq!(
        sent, 0,
        "the color of an Apple track was asked of the renderer"
    );
    // While the remote library reconnects, or has no link, an Apple track is still answered
    // here, and a request that is not routed by item still waits for the remote library.
    for (link, reconnecting, expected) in [
        (None, Some("Renderer".to_owned()), "reconnecting"),
        (None, None, "unavailable"),
    ] {
        let away = crate::request_owner::Implementation::RunePeer { link, reconnecting };
        let answered = away
            .run(&apple.fixture.global_params, &color)
            .await?
            .context("the color of an Apple track while the remote library is away")?;
        assert_eq!(answered.primary_color, None);
        let waiting = away
            .run(&apple.fixture.global_params, &mix)
            .await
            .err()
            .context("a mix while the remote library is away")?;
        let kind = match waiting.downcast_ref::<ExpectedRequestError>() {
            Some(ExpectedRequestError::Reconnecting { .. }) => "reconnecting",
            Some(ExpectedRequestError::Unavailable { .. }) => "unavailable",
            _ => "other",
        };
        assert_eq!(kind, expected, "{waiting:#}");
    }
    // A Subsonic track goes to the renderer, which refuses it for its own reasons.
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let subsonic_track = PlayingItemRef::LibraryTrack {
        library_id: subsonic.0.to_string(),
        track: "s1".into(),
    };
    let forwarded = apple
        .run(&load_items(
            vec![subsonic_track.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await
        .err()
        .context("a load of a Subsonic track on the renderer")?;
    assert_eq!(
        classify(&forwarded),
        RequestErrorKind::InvalidInput,
        "{forwarded:#}"
    );
    assert!(
        matches!(
            forwarded.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Remote { .. })
        ),
        "{forwarded:#}"
    );

    apple.node.detach_renderer().await?;

    // With the renderer's library open here, browsing it lists the caller's queue, from which
    // the Apple track is left out before it goes; the rest goes, and the page still answers.
    let renderer_library = renderer
        .global_params
        .library_id
        .context("the renderer's library")?;
    renderer
        .global_params
        .node
        .set_open_local_catalog(None, renderer_library)
        .await?;
    let remote =
        super::open_remote_control_record(apple.node.clone(), &renderer_info.fingerprint).await?;
    let queue_query = vec![MixQuery {
        operator: "lib::queue".into(),
        parameter: "true".into(),
    }];
    let page = |items: Vec<PlayingItemRef>| crate::messages::MixQueryRequest {
        queries: queue_query.clone(),
        cursor: 0,
        page_size: 30,
        bake_cover_arts: false,
        queue: CallerQueue { items },
        random_seed: None,
        call_id: None,
    };
    let mix = |items: Vec<PlayingItemRef>| ResolveMixQueryRequest {
        queries: queue_query.clone(),
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems { items },
        call_id: None,
    };
    let to_renderer =
        async || dispatch_gui_request(&remote, &SessionSnapshotRequest { call_id: None }).await;
    let with_apple = vec![song_ref(&apple, "i.s1"), subsonic_track.clone()];
    let without_apple = vec![subsonic_track.clone()];
    // What goes for a request that lists the Apple track is what goes for one that lists only the
    // rest, and both answer.
    let mut sizes = Vec::new();
    for items in [with_apple.clone(), without_apple.clone()] {
        let mut answered = None;
        let sent = bytes_sent(
            &proxy,
            async {
                answered = Some(dispatch_gui_request(&remote, &page(items)).await);
            },
            to_renderer,
        )
        .await?;
        answered
            .context("the page asked")??
            .context("a page of the renderer's library")?;
        sizes.push(sent);
    }
    assert!(sizes[1] > 0, "the page did not reach the renderer");
    assert_eq!(sizes[0], sizes[1], "the Apple track went with the page");
    let mut sizes = Vec::new();
    for items in [with_apple.clone(), without_apple] {
        let mut answered = None;
        let sent = bytes_sent(
            &proxy,
            async {
                answered = Some(dispatch_gui_request(&remote, &mix(items)).await);
            },
            to_renderer,
        )
        .await?;
        answered
            .context("the mix asked")??
            .context("a mix of the renderer's library")?;
        sizes.push(sent);
    }
    assert!(sizes[1] > 0, "the mix did not reach the renderer");
    assert_eq!(sizes[0], sizes[1], "the Apple track went with the mix");

    // With the renderer's library open here, what an Apple track's library declares for its
    // items is decided here, and never sent to the renderer: it keeps no likes or lyrics, which
    // are refused, and there is no cover file to take a color from.
    let mut answers = Vec::new();
    let sent = bytes_sent(
        &proxy,
        async {
            answers.push(
                dispatch_gui_request(
                    &remote,
                    &SetLikedRequest {
                        item: Some(song_ref(&apple, "i.s1")),
                        liked: true,
                        call_id: None,
                    },
                )
                .await
                .map(drop),
            );
            answers.push(
                dispatch_gui_request(
                    &remote,
                    &GetLyricByTrackIdRequest {
                        item: Some(song_ref(&apple, "i.s1")),
                        call_id: None,
                    },
                )
                .await
                .map(drop),
            );
        },
        to_renderer,
    )
    .await?;
    for (answer, what) in answers.into_iter().zip(["a like", "the lyrics"]) {
        let refused = answer.err().with_context(|| {
            format!("{what} of an Apple track with the renderer's library open")
        })?;
        assert!(unsupported(&refused), "{what}: {refused:#}");
    }
    let mut color = None;
    let sent = sent
        + bytes_sent(
            &proxy,
            async {
                color = Some(
                    dispatch_gui_request(
                        &remote,
                        &GetPrimaryColorByTrackIdRequest {
                            item: Some(song_ref(&apple, "i.s1")),
                            call_id: None,
                        },
                    )
                    .await,
                );
            },
            to_renderer,
        )
        .await?;
    let color = color
        .context("the color asked")??
        .context("the color of an Apple track with the renderer's library open")?;
    assert_eq!(color.primary_color, None);
    assert_eq!(
        sent, 0,
        "a request about an Apple track reached the renderer"
    );
    // A like of a track of a library the renderer may hold goes to the renderer, which answers
    // for itself.
    let mut peer_answer = None;
    let sent = bytes_sent(
        &proxy,
        async {
            for liked in [true, false] {
                let _ = dispatch_gui_request(
                    &remote,
                    &SetLikedRequest {
                        item: Some(subsonic_track.clone()),
                        liked,
                        call_id: None,
                    },
                )
                .await;
            }
            peer_answer = Some(
                dispatch_gui_request(
                    &remote,
                    &crate::messages::GetLikedRequest {
                        item: Some(subsonic_track.clone()),
                        call_id: None,
                    },
                )
                .await,
            );
        },
        to_renderer,
    )
    .await?;
    assert!(
        sent > 0,
        "the likes of a Subsonic track did not reach the renderer"
    );
    if let Some(Err(error)) = peer_answer {
        assert!(
            matches!(
                error.downcast_ref::<ExpectedRequestError>(),
                Some(ExpectedRequestError::Remote { .. })
            ),
            "{error:#}"
        );
    }

    renderer.global_params.node.shutdown().await?;
    apple.finish().await
}

/// An Apple library whose node broadcaster is heard.
async fn open_apple_heard(dir: &Path) -> Result<(OpenApple, Arc<Heard>)> {
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir,
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    Ok((apple, heard))
}

const CONTENT_CHANGED: &str = "LibraryContentChanged";

async fn open_big_apple(dir: &Path, extra_songs: usize) -> Result<OpenApple> {
    open_apple_with(
        dir,
        AppleOptions {
            extra_songs,
            ..AppleOptions::default()
        },
    )
    .await
}

/// Waits until the fake has been asked for something whose path and query start with `prefix`.
async fn fake_asked(apple: &OpenApple, prefix: &'static str, times: usize) -> Result<()> {
    wait_until("the fake Apple API to be asked", || async {
        Ok((apple.fake.count(prefix) >= times).then_some(()))
    })
    .await
}

async fn collection_names(
    apple: &OpenApple,
    collection_type: CollectionType,
    ids: &[i32],
) -> Result<Vec<String>> {
    Ok(apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type,
            bake_cover_arts: false,
            ids: ids.to_vec(),
        })
        .await?
        .result
        .into_iter()
        .map(|collection| collection.name)
        .collect())
}

/// The names and ids of the artists listed in the groups `titles`.
async fn listed_artists(apple: &OpenApple, titles: &[&str]) -> Result<Vec<(String, i32)>> {
    Ok(apple
        .run(&FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: false,
            group_titles: titles.iter().map(|title| (*title).to_owned()).collect(),
        })
        .await?
        .groups
        .into_iter()
        .flat_map(|group| group.collections)
        .map(|collection| (collection.name, collection.id))
        .collect())
}

/// The titles of the songs of an artist's page.
async fn artist_songs(apple: &OpenApple, artist: i32) -> Result<Vec<String>> {
    Ok(apple
        .run(&mix("lib::artist", artist))
        .await?
        .files
        .into_iter()
        .map(|file| file.title)
        .collect())
}

async fn library_album_songs(apple: &OpenApple, remote: &str) -> Result<Vec<String>> {
    let album = apple.binding.intern(AppleIdKind::Album, remote)?;
    Ok(apple
        .run(&mix("lib::album", album))
        .await?
        .files
        .into_iter()
        .map(|file| file.title)
        .collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_album_page_is_answered_from_the_song_list() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    assert_eq!(
        library_album_songs(&apple, "l.alb1").await?,
        ["First Light", "Second Wind"]
    );
    assert_eq!(apple.fake.count("/v1/me/library/albums/l.alb1"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_album_page_is_read_from_apple_when_the_song_list_lacks_its_songs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            unlisted_songs: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    assert_eq!(
        library_album_songs(&apple, "l.alb1").await?,
        ["First Light", "Second Wind"]
    );
    assert!(apple.fake.count("/v1/me/library/albums/l.alb1") > 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn artists_credited_together_are_listed_one_by_one_with_pictures_of_their_own() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            credited: true,
            ..AppleOptions::default()
        },
    )
    .await?;

    let summary = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?;
    let groups: Vec<_> = summary
        .groups
        .iter()
        .map(|group| (group.group_title.as_str(), group.count))
        .collect();
    // "Artist Two" is no library artist here, only credited by a song.
    assert_eq!(groups, [("A", 2), ("G", 1)]);
    let listed = listed_artists(&apple, &["A", "G"]).await?;
    let names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Artist One", "Artist Two", "Guest"]);
    let (one, guest) = (listed[0].1, listed[2].1);
    // Each shows the catalog artist of its name, though Apple names "Artist One" alone for
    // one of the credits of both.
    let cover = |id| {
        apple
            .binding
            .collection_info(AppleIdKind::Artist, id)
            .and_then(|info| info.cover)
    };
    let photo = |name: &str| {
        Some(format!(
            "{}/art/{name}_Photo/600x600bb.jpg",
            apple.fake.base
        ))
    };
    assert_eq!(cover(one), photo("Artist_One"));
    assert_eq!(cover(guest), photo("Guest"));
    assert_eq!(
        collection_names(&apple, CollectionType::Artist, &[one, guest]).await?,
        ["Artist One", "Guest"]
    );

    // An artist's page has the songs Apple's artists name them for, each once, by album name
    // as no album has a year, and it is answered from the song list. Apple names only
    // "Artist One" for the credit "Artist One & Guest", so that song is not Guest's.
    let reads = apple.fake.total();
    assert_eq!(
        artist_songs(&apple, one).await?,
        ["First Light", "Second Wind", "Duet", "Guest Spot"]
    );
    assert_eq!(artist_songs(&apple, guest).await?, ["Guest Spot"]);
    assert_eq!(apple.fake.total(), reads);

    // A library artist the UI was given before still opens, as that credit alone.
    let duo = apple.binding.intern(AppleIdKind::Artist, "r.duo")?;
    assert_eq!(
        collection_names(&apple, CollectionType::Artist, &[duo]).await?,
        ["Artist One & Guest"]
    );
    assert_eq!(artist_songs(&apple, duo).await?, ["Duet"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_only_a_song_credits_is_listed_linked_and_opens_from_the_song_list() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            solo: true,
            ..AppleOptions::default()
        },
    )
    .await?;

    let listed = listed_artists(&apple, &["A", "S"]).await?;
    let names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Artist One", "Artist Two", "Solo Voice"]);
    let solo = listed[2].1;

    // The song's details link to that entry, never to id 0.
    let song = apple.binding.intern(AppleIdKind::LibrarySong, "i.solo")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: song,
        })
        .await?;
    assert_eq!(parsed.artists[0].name, "Solo Voice");
    assert_eq!(parsed.artists[0].id, solo);
    assert_ne!(solo, 0);

    // The entry's page lists the song, from the song list.
    let reads = apple.fake.total();
    assert_eq!(artist_songs(&apple, solo).await?, ["Alone"]);
    assert_eq!(apple.fake.total(), reads);
    apple.finish().await
}

fn catalog_credited() -> AppleOptions {
    AppleOptions {
        catalog_credits: true,
        ..AppleOptions::default()
    }
}

/// Every artist of the Artists page, by name, with its id.
async fn all_artists(apple: &OpenApple) -> Result<Vec<(String, i32)>> {
    let titles: Vec<String> = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?
        .groups
        .into_iter()
        .map(|group| group.group_title)
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    let mut listed = listed_artists(apple, &titles).await?;
    listed.sort();
    Ok(listed)
}

fn names(listed: &[(String, i32)]) -> Vec<&str> {
    listed.iter().map(|(name, _)| name.as_str()).collect()
}

fn artist_named(listed: &[(String, i32)], name: &str) -> Result<i32> {
    listed
        .iter()
        .find(|(listed, _)| listed == name)
        .map(|(_, id)| *id)
        .with_context(|| format!("the artist {name}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn artists_are_told_apart_by_their_catalog_artists_whatever_their_credits_spell() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), catalog_credited()).await?;
    let listed = all_artists(&apple).await?;
    assert_eq!(
        names(&listed),
        [
            "Ann",
            "Artist One",
            "Artist Two",
            "Bob",
            "Hiroyuki Sawano",
            "サワノヒロユキ & Cara"
        ]
    );
    let remote = |name: &str| -> Result<String> {
        apple
            .binding
            .resolve(AppleIdKind::Artist, artist_named(&listed, name)?)
    };
    assert_eq!(remote("Ann")?, "artist:400");
    assert_eq!(remote("Hiroyuki Sawano")?, "artist:156");
    assert_eq!(remote("Artist One")?, "artist:300");
    // Apple names no catalog artist for these, so their credits tell them, whole.
    assert_eq!(
        remote("サワノヒロユキ & Cara")?,
        "name:サワノヒロユキ & cara"
    );
    assert_eq!(remote("Artist Two")?, "name:artist two");

    // "Ann & Bob" is listed under both its catalog artists.
    let ann = artist_named(&listed, "Ann")?;
    assert_eq!(artist_songs(&apple, ann).await?, ["Together", "Reversed"]);
    let bob = artist_named(&listed, "Bob")?;
    assert_eq!(artist_songs(&apple, bob).await?, ["Together"]);
    // "サワノヒロユキ" is the catalog's "Hiroyuki Sawano", named so. The song without a catalog id
    // stays under its whole credit: its text is not split to find "Hiroyuki Sawano" in it.
    let sawano = artist_named(&listed, "Hiroyuki Sawano")?;
    assert_eq!(
        artist_songs(&apple, sawano).await?,
        ["Kana Song", "English Song", "Reversed"]
    );
    let whole = artist_named(&listed, "サワノヒロユキ & Cara")?;
    assert_eq!(artist_songs(&apple, whole).await?, ["No Catalog"]);
    // A song's details link to its catalog artists, by the ids the list gives them.
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: apple.binding.intern(AppleIdKind::LibrarySong, "i.rev")?,
        })
        .await?;
    assert_eq!(
        parsed
            .artists
            .iter()
            .map(|artist| (artist.id, artist.name.as_str()))
            .collect::<Vec<_>>(),
        [(sawano, "Hiroyuki Sawano"), (ann, "Ann")]
    );
    // The catalog was asked once for every song with a catalog id, and not again.
    let asked = apple.fake.count("/v1/catalog/it/songs?");
    all_artists(&apple).await?;
    artist_songs(&apple, sawano).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), asked);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_catalog_artists_of_a_large_library_are_read_several_batches_at_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_big_apple(dir.path(), 1000).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 0);
    *apple.fake.delay_path.lock().unwrap() = Some((
        "/v1/catalog/it/songs".into(),
        std::time::Duration::from_millis(300),
    ));
    let listed = all_artists(&apple).await?;
    assert_eq!(names(&listed), ["Artist One", "Artist Two"]);
    // The songs were asked about in several batches, more than one at a time, and every answer
    // was kept.
    let batches = apple.fake.count("/v1/catalog/it/songs?");
    assert!(batches >= 4, "{batches} batches");
    assert!(
        apple
            .fake
            .most_catalog_songs_in_flight
            .load(Ordering::SeqCst)
            > 1
    );
    let kept = apple
        .binding
        .kept_song_artists()
        .await
        .context("the kept catalog artists")?;
    let songs = apple.binding.known_songs().context("the songs")?;
    assert!(kept.missing(songs.songs()).is_empty());
    // So a listing again asks about none.
    all_artists(&apple).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), batches);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_artist_list_waits_a_while_for_the_catalog_then_follows_it() -> Result<()> {
    use crate::apple_music_binding::FIRST_SONG_ARTISTS_WAIT;

    let heard = Arc::new(Heard::default());
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..catalog_credited()
        },
    )
    .await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 0);
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some(("/v1/catalog/it/songs".into(), release.clone()));
    let by_credits = [
        "Ann & Bob",
        "Artist One",
        "Artist Two",
        "Hiroyuki Sawano",
        "Sawano Hiroyuki & Ann",
        "サワノヒロユキ",
        "サワノヒロユキ & Cara",
    ];
    // Apple does not answer in time, so the first list tells its artists apart by their
    // credits once it waited its while.
    let started = std::time::Instant::now();
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    let waited = started.elapsed();
    assert!(
        waited >= FIRST_SONG_ARTISTS_WAIT && waited < FIRST_SONG_ARTISTS_WAIT * 3,
        "waited {waited:?}"
    );
    // Later lists do not wait again.
    let started = std::time::Instant::now();
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    assert!(started.elapsed() < FIRST_SONG_ARTISTS_WAIT);
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 1);

    // Once Apple answers, the read is kept, the UI is told, and the list follows.
    let told = heard.count(CONTENT_CHANGED);
    release.notify_one();
    wait_until("the artists to be told apart by the catalog", || async {
        let listed = all_artists(&apple).await?;
        Ok((names(&listed)
            == [
                "Ann",
                "Artist One",
                "Artist Two",
                "Bob",
                "Hiroyuki Sawano",
                "サワノヒロユキ & Cara",
            ])
        .then_some(()))
    })
    .await?;
    told_after(&heard, told).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_catalog_read_names_artists_by_their_whole_credits_and_is_tried_again()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), catalog_credited()).await?;
    *apple.fake.respond_path.lock().unwrap() =
        Some(("/v1/catalog/it/songs".into(), 500, "{}".into()));
    // The list still answers, every credit an artist of its own, whole.
    let by_credits = [
        "Ann & Bob",
        "Artist One",
        "Artist Two",
        "Hiroyuki Sawano",
        "Sawano Hiroyuki & Ann",
        "サワノヒロユキ",
        "サワノヒロユキ & Cara",
    ];
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    // Once Apple answers again, a later listing asks it behind the list, and the list follows.
    *apple.fake.respond_path.lock().unwrap() = None;
    apple
        .clock
        .advance(crate::apple_music_binding::SHORT_LIST_AGE);
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    wait_until("the artists to be told apart by the catalog", || async {
        let listed = all_artists(&apple).await?;
        Ok((names(&listed)
            == [
                "Ann",
                "Artist One",
                "Artist Two",
                "Bob",
                "Hiroyuki Sawano",
                "サワノヒロユキ & Cara",
            ])
        .then_some(()))
    })
    .await?;
    apple.finish().await
}

/// The song of the catalog the fake has by id 777, as its menu reads it: from the catalog, the
/// first time.
async fn catalog_song(apple: &OpenApple) -> Result<crate::messages::FetchParsedMediaFileResponse> {
    let id = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    apple
        .run(&FetchParsedMediaFileRequest { call_id: None, id })
        .await
}

/// What the UI is told about the sign-in of the Apple Music record, in order.
fn announced_sign_ins(heard: &Heard, record: LibraryRecordId) -> Vec<Option<bool>> {
    heard
        .placed::<crate::messages::LibraryRecordsChanged>("LibraryRecordsChanged")
        .into_iter()
        .filter_map(|(_, changed)| {
            changed
                .records
                .into_iter()
                .find(|summary| summary.record_id == record.0.to_string())
                .map(|summary| summary.signed_in)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_is_told_when_an_apple_library_signs_out_and_in_again() -> Result<()> {
    use crate::messages::{
        ListLibraryRecordsRequest, ServiceSignIn, SignInServiceLibraryRequest,
        SignOutServiceLibraryRequest,
    };
    use crate::utils::{sign_in_service_library_on_node, sign_out_service_library_on_node};

    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let node = &apple.node;
    let list = || async {
        ListLibraryRecordsRequest { call_id: None }
            .handle(
                node.clone(),
                None,
                &ListLibraryRecordsRequest { call_id: None },
            )
            .await?
            .context("list library records")
    };
    let sign_state = |listed: &crate::messages::ListLibraryRecordsResponse| {
        listed
            .records
            .iter()
            .find(|summary| summary.record_id == apple.record.0.to_string())
            .map(|summary| summary.signed_in)
    };

    // Signed in by the registration.
    let listed = list().await?;
    assert_eq!(sign_state(&listed), Some(Some(true)));
    // Records with no sign-in of their own say nothing.
    assert!(
        listed
            .records
            .iter()
            .filter(|summary| summary.record_id != apple.record.0.to_string())
            .all(|summary| summary.signed_in.is_none())
    );

    let sign_out = SignOutServiceLibraryRequest {
        record_id: apple.record.0.to_string(),
        call_id: None,
    };
    sign_out_service_library_on_node(&sign_out, node).await?;
    assert!(!apple.binding.is_signed_in());
    assert_eq!(sign_state(&list().await?), Some(Some(false)));
    assert_eq!(
        announced_sign_ins(&heard, apple.record).last(),
        Some(&Some(false))
    );
    // Signing out again is no failure.
    sign_out_service_library_on_node(&sign_out, node).await?;

    sign_in_service_library_on_node(
        &SignInServiceLibraryRequest {
            record_id: apple.record.0.to_string(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        node,
    )
    .await?;
    assert_eq!(sign_state(&list().await?), Some(Some(true)));
    assert_eq!(
        announced_sign_ins(&heard, apple.record).last(),
        Some(&Some(true))
    );
    apple.finish().await
}

// Telling the UI ----------------------------------------------------------------------------

/// Waits for the UI to be told that the library changed since it was told `told` times.
async fn told_after(heard: &Heard, told: usize) -> Result<()> {
    wait_until("the UI to be told the library changed", || async {
        Ok((heard.count(CONTENT_CHANGED) > told).then_some(()))
    })
    .await
}

/// Waits until the UI has not been told anything for longer than the binding waits before it
/// tells, and answers how often it was told.
async fn told_settled(heard: &Heard) -> usize {
    loop {
        let told = heard.count(CONTENT_CHANGED);
        tokio::time::sleep(CONTENT_CHANGED_DELAY * 2).await;
        if heard.count(CONTENT_CHANGED) == told {
            return told;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gone_song_is_left_out_of_the_songs_fetched_by_id() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let first = apple.binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    let gone = apple
        .binding
        .intern(AppleIdKind::LibrarySong, "catalog:404404")?;
    let last = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    let fetched = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![first, gone, last],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(
        fetched
            .media_files
            .iter()
            .map(|file| file.track.as_str())
            .collect::<Vec<_>>(),
        ["i.s9", "catalog:777"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_the_library_no_longer_lists_is_skipped_by_the_collections_fetched_by_id()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // Ids of before the list told its artists apart, which name artists by their spelling.
    let gone = apple
        .binding
        .intern(AppleIdKind::Artist, "name:nobody of this name")?;
    let held = apple
        .binding
        .intern(AppleIdKind::Artist, "name:artist one")?;
    let fetched = apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: false,
            ids: vec![gone, held],
        })
        .await?;
    assert_eq!(
        fetched
            .result
            .iter()
            .map(|artist| (artist.id, artist.name.as_str()))
            .collect::<Vec<_>>(),
        [(held, "Artist One")]
    );
    apple.finish().await
}
