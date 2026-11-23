//! A Subsonic server on the loopback interface that answers from Apple Music.
//!
//! Apple Music audio is DRM protected and only Apple's own player can play it. The hub reaches it
//! the way it reaches any Subsonic server in jukebox mode: browsing through the Subsonic REST
//! endpoints and playback through `jukeboxControl`, with Apple's player as the jukebox.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::extract::{Path, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use url::form_urlencoded;
use uuid::Uuid;

use crate::apple_bridge::music::{self, AppleMusicError};

const USERNAME: &str = "apple-music";
const API_VERSION: &str = "1.16.1";

/// Items asked for per request when a Subsonic endpoint returns a whole list.
const LIST_PAGE: usize = 200;
/// Apple Music libraries hold at most 100,000 songs, so no list is longer. The cap keeps a
/// source that ignores the offset from being asked for pages forever.
const MAX_LISTED: usize = 100_000;

const CODE_GENERIC: i64 = 0;
const CODE_MISSING_PARAMETER: i64 = 10;
const CODE_WRONG_CREDENTIALS: i64 = 40;
const CODE_NOT_FOUND: i64 = 70;

// Changing the library needs a user token this OS build does not issue to the app.
/// Cover ids name the song or album whose artwork they are.
const SONG_COVER: &str = "song-";
const ALBUM_COVER: &str = "album-";
const PLAYLISTS_READ_ONLY: &str = "Apple Music playlists cannot be changed from Rune yet";
const FAVORITES_READ_ONLY: &str = "Apple Music favorites cannot be changed from Rune yet";
const NO_STREAMING: &str = "Apple Music audio can only be played by Apple's player";

/// Where the gateway gets Apple Music data and sends player commands.
#[async_trait]
pub trait AppleMusicSource: Send + Sync + 'static {
    /// Sends one bridge request, an object with an `op` and its arguments, and returns its answer.
    async fn request(&self, request: Value) -> Result<Value>;
}

/// The Apple Music library and player of this Mac, reached through MusicKit.
pub struct MusicKitSource;

#[async_trait]
impl AppleMusicSource for MusicKitSource {
    async fn request(&self, request: Value) -> Result<Value> {
        music::request::<Value>(request).await
    }
}

/// The gateway of this process, started on first use. One is enough: it serves the one Apple
/// Music library of this device, and its address must stay the same while that library is open.
pub async fn shared() -> Result<&'static AppleMusicGateway> {
    static GATEWAY: tokio::sync::OnceCell<AppleMusicGateway> = tokio::sync::OnceCell::const_new();
    GATEWAY
        .get_or_try_init(|| AppleMusicGateway::start(Arc::new(MusicKitSource)))
        .await
}

/// A running loopback Subsonic server for Apple Music. It serves until dropped.
pub struct AppleMusicGateway {
    address: SocketAddr,
    password: String,
    shutdown: CancellationToken,
}

impl AppleMusicGateway {
    /// Starts serving `source` on a free loopback port with a new random password.
    pub async fn start(source: Arc<dyn AppleMusicSource>) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .context("failed to open the Apple Music gateway port")?;
        let address = listener
            .local_addr()
            .context("failed to read the Apple Music gateway address")?;
        let password = random_password();
        let gateway = Arc::new(Gateway {
            source,
            password: password.clone(),
            jukebox: Mutex::new(HashMap::new()),
            links: Mutex::new(None),
        });
        let app = Router::new()
            .route("/rest/{endpoint}", get(handle))
            .with_state(gateway);
        let shutdown = CancellationToken::new();
        let stopped = shutdown.clone().cancelled_owned();
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(stopped)
                .await
            {
                log::warn!("Apple Music gateway stopped: {error}");
            }
        });
        Ok(Self {
            address,
            password,
            shutdown,
        })
    }

    /// The base URL Subsonic clients connect to, such as `http://127.0.0.1:49152`.
    pub fn endpoint(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The only account the gateway accepts.
    pub fn username(&self) -> &str {
        USERNAME
    }

    /// The account's password, new for every gateway.
    pub fn password(&self) -> &str {
        &self.password
    }
}

impl Drop for AppleMusicGateway {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

fn random_password() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Gateway {
    source: Arc<dyn AppleMusicSource>,
    password: String,
    /// Songs of the player's queue already read, by id, so polling the jukebox does not read
    /// them again. The lock also runs player commands one at a time: each reads the player's
    /// state and then changes it.
    jukebox: Mutex<HashMap<String, Child>>,
    /// The ids of the library's albums and artists by name, read once. MusicKit names a song's
    /// album and artist but gives their ids only for one more request per song.
    links: Mutex<Option<Arc<Links>>>,
}

#[derive(Default)]
struct Links {
    albums: HashMap<(String, String), String>,
    album_titles: HashMap<String, String>,
    artists: HashMap<String, String>,
}

async fn handle(
    State(gateway): State<Arc<Gateway>>,
    Path(endpoint): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let params = Params::parse(query.as_deref().unwrap_or_default());
    if !gateway.authorized(&params) {
        return failed(Failure {
            code: CODE_WRONG_CREDENTIALS,
            message: "Wrong username or password".into(),
        });
    }
    let endpoint = endpoint.strip_suffix(".view").unwrap_or(&endpoint);
    if endpoint == "getCoverArt" {
        return match gateway.cover(&params).await {
            Some(jpeg) => ([(header::CONTENT_TYPE, "image/jpeg")], jpeg).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    let started = std::time::Instant::now();
    let answer = gateway.answer(endpoint, &params).await;
    log::debug!(
        "Apple Music gateway {endpoint} took {} ms",
        started.elapsed().as_millis()
    );
    match answer {
        Ok(body) => envelope("ok", body),
        Err(failure) => {
            log::warn!(
                "Apple Music gateway refused {endpoint}: {} (code {})",
                failure.message,
                failure.code
            );
            failed(failure)
        }
    }
}

fn envelope(status: &str, body: Value) -> Response {
    let mut response = Map::new();
    response.insert("status".into(), status.into());
    response.insert("version".into(), API_VERSION.into());
    if let Value::Object(fields) = body {
        response.extend(fields);
    }
    Json(json!({ "subsonic-response": response })).into_response()
}

fn failed(failure: Failure) -> Response {
    envelope(
        "failed",
        json!({ "error": { "code": failure.code, "message": failure.message } }),
    )
}

/// A Subsonic `failed` answer.
#[derive(Debug)]
struct Failure {
    code: i64,
    message: String,
}

impl Failure {
    fn generic(message: impl Into<String>) -> Self {
        Self {
            code: CODE_GENERIC,
            message: message.into(),
        }
    }

    fn refused(message: &str) -> Self {
        Self {
            code: CODE_GENERIC,
            message: message.into(),
        }
    }

    fn missing(key: &str) -> Self {
        Self {
            code: CODE_MISSING_PARAMETER,
            message: format!("Required parameter is missing: {key}"),
        }
    }

    /// Codes 40 and 50 both read as a credential problem to the hub, which then asks the user
    /// to sign in to the server again. A MusicKit permission problem is not one, so it is
    /// reported as a plain failure with its reason.
    fn from_source(error: anyhow::Error) -> Self {
        match error
            .chain()
            .find_map(|cause| cause.downcast_ref::<AppleMusicError>())
        {
            Some(cause @ AppleMusicError::NotAuthorized(_)) => Self {
                code: CODE_GENERIC,
                message: cause.to_string(),
            },
            Some(cause @ AppleMusicError::NotFound(_)) => Self {
                code: CODE_NOT_FOUND,
                message: cause.to_string(),
            },
            _ => Self::generic(format!("{error:#}")),
        }
    }
}

/// The query parameters of one request with repeats kept: jukebox `set` and `add` repeat `id`.
struct Params(Vec<(String, String)>);

impl Params {
    fn parse(query: &str) -> Self {
        Self(
            form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect(),
        )
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn all(&self, key: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    fn required(&self, key: &str) -> Result<&str, Failure> {
        self.get(key).ok_or_else(|| Failure::missing(key))
    }

    fn number<T: FromStr>(&self, key: &str, default: T) -> Result<T, Failure> {
        self.get(key)
            .map_or(Ok(default), |text| parse_number(key, text))
    }

    fn required_number<T: FromStr>(&self, key: &str) -> Result<T, Failure> {
        parse_number(key, self.required(key)?)
    }
}

fn parse_number<T: FromStr>(key: &str, text: &str) -> Result<T, Failure> {
    text.parse()
        .map_err(|_| Failure::generic(format!("Invalid parameter {key}: {text}")))
}

fn decode_hex(text: &str) -> Option<String> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..text.len())
        .step_by(2)
        .map(|start| {
            text.get(start..start + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// A page of results inside a longer list, from a `...Count` and a `...Offset` parameter.
struct Window {
    count: usize,
    offset: usize,
}

impl Window {
    fn from_params(params: &Params, count: &str, offset: &str) -> Result<Self, Failure> {
        Ok(Self {
            count: params.number(count, 20)?,
            offset: params.number(offset, 0)?,
        })
    }

    fn end(&self) -> usize {
        self.offset.saturating_add(self.count)
    }

    fn take<T>(&self, items: Vec<T>) -> impl Iterator<Item = T> {
        items.into_iter().skip(self.offset).take(self.count)
    }
}

/// The search text without the quotes and the trailing `*` wildcard Rune's search box adds.
fn search_term(query: &str) -> &str {
    query.trim().trim_matches('"').trim_end_matches('*').trim()
}

/// The Subsonic artist index an artist is listed under: its upper-case initial, or `#`.
fn index_name(artist: &str) -> String {
    match artist.trim_start().chars().next() {
        Some(first) if first.is_alphabetic() => first.to_uppercase().collect(),
        _ => "#".to_owned(),
    }
}

fn whole_seconds(seconds: f64, round: fn(f64) -> f64) -> Option<u32> {
    (seconds.is_finite() && seconds >= 0.0).then(|| round(seconds).min(f64::from(u32::MAX)) as u32)
}

fn count(items: usize) -> Option<u32> {
    u32::try_from(items).ok()
}

impl Gateway {
    /// The simulator's rules: the user must match and either the password (plain or `enc:`
    /// hex) or the salted MD5 token must.
    fn authorized(&self, params: &Params) -> bool {
        if params.get("u") != Some(USERNAME) {
            return false;
        }
        if let Some(password) = params.get("p") {
            return match password.strip_prefix("enc:") {
                Some(hex) => decode_hex(hex).is_some_and(|password| password == self.password),
                None => password == self.password,
            };
        }
        match (params.get("t"), params.get("s")) {
            (Some(token), Some(salt)) => {
                let expected = md5::compute(format!("{}{salt}", self.password));
                token.eq_ignore_ascii_case(&format!("{expected:x}"))
            }
            _ => false,
        }
    }

    async fn call<T: DeserializeOwned>(&self, request: Value) -> Result<T, Failure> {
        let reply = self
            .source
            .request(request)
            .await
            .map_err(Failure::from_source)?;
        serde_json::from_value(reply)
            .map_err(|error| Failure::generic(format!("Unexpected Apple Music reply: {error}")))
    }

    /// The album and artist ids by name. A failed read is not kept, so the next song tries
    /// again; until then songs go without the ids.
    async fn links(&self) -> Arc<Links> {
        let mut links = self.links.lock().await;
        if let Some(links) = links.as_ref() {
            return Arc::clone(links);
        }
        let albums: Result<Vec<SourceAlbum>, Failure> = self.everything("albums").await;
        let artists: Result<Vec<SourceArtist>, Failure> = self.everything("artists").await;
        let (Ok(albums), Ok(artists)) = (albums, artists) else {
            return Arc::new(Links::default());
        };
        let mut read = Links::default();
        for album in albums {
            read.album_titles
                .insert(album.title.clone(), album.id.clone());
            read.albums
                .insert((album.title, album.artist.unwrap_or_default()), album.id);
        }
        for artist in artists {
            read.artists.insert(artist.name, artist.id);
        }
        let read = Arc::new(read);
        *links = Some(Arc::clone(&read));
        read
    }

    /// A song as the client lists it, with the ids of its album and artist found by name.
    async fn child(&self, song: SourceSong) -> Child {
        let mut child = Child::from(song);
        if child.album_id.is_some() && child.artist_id.is_some() {
            return child;
        }
        let links = self.links().await;
        let artist = child.artist.clone().unwrap_or_default();
        if child.album_id.is_none()
            && let Some(album) = &child.album
        {
            child.album_id = links
                .albums
                .get(&(album.clone(), artist.clone()))
                .or_else(|| links.album_titles.get(album))
                .cloned();
        }
        if child.artist_id.is_none() {
            child.artist_id = links.artists.get(&artist).cloned();
        }
        child
    }

    async fn children(&self, songs: impl IntoIterator<Item = SourceSong>) -> Vec<Child> {
        let mut children = Vec::new();
        for song in songs {
            children.push(self.child(song).await);
        }
        children
    }

    /// The artwork a cover id names, or `None` when there is none to show.
    async fn cover(&self, params: &Params) -> Option<Vec<u8>> {
        let id = params.required("id").ok()?;
        let (kind, id) = id
            .strip_prefix(ALBUM_COVER)
            .map(|id| ("album", id))
            .or_else(|| id.strip_prefix(SONG_COVER).map(|id| ("song", id)))?;
        let size = params
            .get("size")
            .and_then(|size| size.parse::<u32>().ok())
            .unwrap_or(300);
        let artwork: Value = self
            .call(json!({ "op": "artwork", "kind": kind, "id": id, "size": size }))
            .await
            .ok()?;
        BASE64.decode(artwork["jpeg"].as_str()?).ok()
    }

    async fn page<T: DeserializeOwned>(
        &self,
        op: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<T>, Failure> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.call(json!({ "op": op, "offset": offset, "limit": limit }))
            .await
    }

    async fn everything<T: DeserializeOwned>(&self, op: &str) -> Result<Vec<T>, Failure> {
        let mut items = Vec::new();
        loop {
            let page: Vec<T> = self.page(op, items.len(), LIST_PAGE).await?;
            let last = page.len() < LIST_PAGE;
            items.extend(page);
            if last || items.len() >= MAX_LISTED {
                return Ok(items);
            }
        }
    }

    async fn answer(&self, endpoint: &str, params: &Params) -> Result<Value, Failure> {
        match endpoint {
            "ping" => Ok(json!({})),
            // Playback is jukebox only: Apple's player is the only one that can play the audio.
            "getUser" => Ok(json!({
                "user": {
                    "username": USERNAME,
                    "streamRole": false,
                    "downloadRole": false,
                    "jukeboxRole": true,
                }
            })),
            // The Tracks page takes its number of rows from this count.
            "getScanStatus" => {
                let total: Value = self.call(json!({ "op": "songCount" })).await?;
                Ok(json!({ "scanStatus": { "scanning": false, "count": total["count"] } }))
            }
            "search3" => self.search(params).await,
            "getArtists" => self.artists().await,
            "getArtist" => {
                let detail: SourceArtistDetail = self
                    .call(json!({ "op": "artist", "id": params.required("id")? }))
                    .await?;
                let artist_id = detail.artist.id.clone();
                let albums: Vec<AlbumEntry> = detail
                    .albums
                    .into_iter()
                    .map(|album| AlbumEntry::from(album).by_artist(&artist_id))
                    .collect();
                let mut artist = ArtistEntry::from(detail.artist);
                artist.album_count = count(albums.len());
                artist.album = Some(albums);
                Ok(json!({ "artist": artist }))
            }
            // Every list type is answered in name order; MusicKit has no play statistics to
            // order by.
            "getAlbumList2" => {
                let albums: Vec<SourceAlbum> = self
                    .page(
                        "albums",
                        params.number("offset", 0)?,
                        params.number("size", 10)?,
                    )
                    .await?;
                let album: Vec<AlbumEntry> = albums.into_iter().map(AlbumEntry::from).collect();
                Ok(json!({ "albumList2": { "album": album } }))
            }
            "getAlbum" => {
                let detail: SourceAlbumDetail = self
                    .call(json!({ "op": "album", "id": params.required("id")? }))
                    .await?;
                let mut album = AlbumEntry::from(detail.album);
                album.song = Some(self.children(detail.songs).await);
                Ok(json!({ "album": album }))
            }
            "getSong" => {
                let song: SourceSong = self
                    .call(json!({ "op": "song", "id": params.required("id")? }))
                    .await?;
                Ok(json!({ "song": self.child(song).await }))
            }
            "getPlaylists" => {
                let playlists: Vec<SourcePlaylist> = self.everything("playlists").await?;
                let playlist: Vec<PlaylistEntry> =
                    playlists.into_iter().map(PlaylistEntry::from).collect();
                Ok(json!({ "playlists": { "playlist": playlist } }))
            }
            "getPlaylist" => {
                let detail: SourcePlaylistDetail = self
                    .call(json!({ "op": "playlist", "id": params.required("id")? }))
                    .await?;
                let entry = self.children(detail.songs).await;
                let mut playlist = PlaylistEntry::from(detail.playlist);
                playlist.song_count = count(entry.len());
                playlist.entry = Some(entry);
                Ok(json!({ "playlist": playlist }))
            }
            "createPlaylist" | "updatePlaylist" | "deletePlaylist" => {
                Err(Failure::refused(PLAYLISTS_READ_ONLY))
            }
            // The cover wall asks for these. A run of songs from a random place in the library
            // is random enough for it and takes one request.
            "getRandomSongs" => {
                let size = params
                    .get("size")
                    .and_then(|size| size.parse::<usize>().ok())
                    .unwrap_or(10)
                    .clamp(1, 500);
                let total: Value = self.call(json!({ "op": "songCount" })).await?;
                let total = total["count"].as_u64().map_or(0, |count| count as usize);
                let span = total.saturating_sub(size) + 1;
                let offset = (Uuid::new_v4().as_u128() % span as u128) as usize;
                let songs: Vec<SourceSong> = self.page("songs", offset, size).await?;
                let song = self.children(songs).await;
                Ok(json!({ "randomSongs": { "song": song } }))
            }
            "getStarred2" => Ok(json!({ "starred2": {} })),
            "star" | "unstar" => Err(Failure::refused(FAVORITES_READ_ONLY)),
            "scrobble" => Ok(json!({})),
            "getLyrics" => Ok(json!({ "lyrics": {} })),
            "stream" | "download" => Err(Failure::refused(NO_STREAMING)),
            "jukeboxControl" => self.jukebox(params).await,
            other => Err(Failure::generic(format!("Unsupported endpoint {other}"))),
        }
    }

    async fn search(&self, params: &Params) -> Result<Value, Failure> {
        let songs = Window::from_params(params, "songCount", "songOffset")?;
        let albums = Window::from_params(params, "albumCount", "albumOffset")?;
        let artists = Window::from_params(params, "artistCount", "artistOffset")?;
        let term = search_term(params.get("query").unwrap_or_default());
        if term.is_empty() {
            let song: Vec<SourceSong> = self.page("songs", songs.offset, songs.count).await?;
            let mut result = json!({ "song": self.children(song).await });
            if albums.count > 0 {
                let album: Vec<SourceAlbum> =
                    self.page("albums", albums.offset, albums.count).await?;
                result["album"] =
                    json!(album.into_iter().map(AlbumEntry::from).collect::<Vec<_>>());
            }
            if artists.count > 0 {
                let artist: Vec<SourceArtist> =
                    self.page("artists", artists.offset, artists.count).await?;
                result["artist"] = json!(
                    artist
                        .into_iter()
                        .map(ArtistEntry::from)
                        .collect::<Vec<_>>()
                );
            }
            return Ok(json!({ "searchResult3": result }));
        }
        // MusicKit's library search has a limit but no offset, so enough results are asked for
        // to cover the furthest page and the pages are cut out here.
        let limit = songs.end().max(albums.end()).max(artists.end());
        let found = if limit == 0 {
            SourceSearch::default()
        } else {
            self.call(json!({ "op": "search", "term": term, "limit": limit }))
                .await?
        };
        let song = self.children(songs.take(found.songs)).await;
        let album: Vec<AlbumEntry> = albums.take(found.albums).map(AlbumEntry::from).collect();
        let artist: Vec<ArtistEntry> = artists.take(found.artists).map(ArtistEntry::from).collect();
        Ok(json!({ "searchResult3": { "song": song, "album": album, "artist": artist } }))
    }

    async fn artists(&self) -> Result<Value, Failure> {
        let artists: Vec<SourceArtist> = self.everything("artists").await?;
        let mut positions = HashMap::new();
        let mut indexes: Vec<(String, Vec<ArtistEntry>)> = Vec::new();
        for artist in artists {
            let name = index_name(&artist.name);
            let position = *positions.entry(name.clone()).or_insert_with(|| {
                indexes.push((name, Vec::new()));
                indexes.len() - 1
            });
            indexes[position].1.push(ArtistEntry::from(artist));
        }
        let index: Vec<Value> = indexes
            .into_iter()
            .map(|(name, artist)| json!({ "name": name, "artist": artist }))
            .collect();
        Ok(json!({ "artists": { "index": index } }))
    }

    async fn jukebox(&self, params: &Params) -> Result<Value, Failure> {
        let mut queue_songs = self.jukebox.lock().await;
        let action = params.required("action")?;
        let request = match action {
            "get" | "status" => json!({ "op": "playerGet" }),
            "set" => json!({ "op": "playerSet", "ids": params.all("id") }),
            "add" => json!({ "op": "playerAdd", "ids": params.all("id") }),
            "clear" => json!({ "op": "playerClear" }),
            "remove" => json!({
                "op": "playerRemove",
                "index": params.required_number::<u32>("index")?,
            }),
            "skip" => json!({
                "op": "playerSkip",
                "index": params.required_number::<u32>("index")?,
                "offset": params.number::<u32>("offset", 0)?,
            }),
            "start" => json!({ "op": "playerStart" }),
            "stop" => json!({ "op": "playerStop" }),
            // Apple's player has no volume control for apps; the gain stays at full.
            "setGain" => json!({ "op": "playerGet" }),
            "playbackMode" => json!({ "op": "playerMode", "mode": params.required("mode")? }),
            "shuffle" => {
                return Err(Failure::generic(
                    "Shuffling is not supported by Apple Music playback",
                ));
            }
            other => return Err(Failure::generic(format!("Unsupported action {other}"))),
        };
        let player: PlayerState = self.call(request).await?;
        if action != "get" {
            return Ok(json!({ "jukeboxStatus": JukeboxState::from(&player) }));
        }
        let queued: HashSet<&String> = player.ids.iter().collect();
        queue_songs.retain(|id, _| queued.contains(id));
        // A queue can be a whole library, so the songs not yet read are asked for together.
        let unread: Vec<&String> = player
            .ids
            .iter()
            .filter(|id| !queue_songs.contains_key(*id))
            .collect();
        if !unread.is_empty() {
            let songs: Vec<SourceSong> = self
                .call(json!({ "op": "songsById", "ids": unread }))
                .await?;
            for song in songs {
                let song = self.child(song).await;
                queue_songs.insert(song.id.clone(), song);
            }
        }
        let entry: Vec<Child> = player
            .ids
            .iter()
            .filter_map(|id| queue_songs.get(id).cloned())
            .collect();
        let mut playlist = JukeboxState::from(&player);
        playlist.entry = Some(entry);
        Ok(json!({ "jukeboxPlaylist": playlist }))
    }
}

/// A song as the bridge describes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceSong {
    id: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    duration: Option<f64>,
    track: Option<u32>,
    disc: Option<u32>,
    album_id: Option<String>,
    artist_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceAlbum {
    id: String,
    title: String,
    artist: Option<String>,
    artist_id: Option<String>,
    song_count: Option<u32>,
}

#[derive(Deserialize)]
struct SourceAlbumDetail {
    #[serde(flatten)]
    album: SourceAlbum,
    songs: Vec<SourceSong>,
}

#[derive(Deserialize)]
struct SourceArtist {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct SourceArtistDetail {
    #[serde(flatten)]
    artist: SourceArtist,
    albums: Vec<SourceAlbum>,
}

#[derive(Deserialize)]
struct SourcePlaylist {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct SourcePlaylistDetail {
    #[serde(flatten)]
    playlist: SourcePlaylist,
    songs: Vec<SourceSong>,
}

#[derive(Default, Deserialize)]
struct SourceSearch {
    songs: Vec<SourceSong>,
    albums: Vec<SourceAlbum>,
    artists: Vec<SourceArtist>,
}

/// What every player op answers with.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerState {
    playing: bool,
    position: f64,
    current_index: Option<u32>,
    ids: Vec<String>,
}

/// A Subsonic `child`: one song.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Child {
    id: String,
    title: String,
    is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disc_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist_id: Option<String>,
    cover_art: String,
}

impl From<SourceSong> for Child {
    fn from(song: SourceSong) -> Self {
        Self {
            cover_art: format!("{SONG_COVER}{}", song.id),
            id: song.id,
            title: song.title,
            is_dir: false,
            artist: song.artist.filter(|name| !name.is_empty()),
            album: song.album.filter(|name| !name.is_empty()),
            duration: song
                .duration
                .and_then(|seconds| whole_seconds(seconds, f64::round)),
            track: song.track,
            disc_number: song.disc,
            album_id: song.album_id,
            artist_id: song.artist_id,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumEntry {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    song: Option<Vec<Child>>,
    cover_art: String,
}

impl AlbumEntry {
    /// Names the artist an album was listed under when MusicKit did not name one.
    fn by_artist(mut self, artist_id: &str) -> Self {
        self.artist_id.get_or_insert_with(|| artist_id.to_owned());
        self
    }
}

impl From<SourceAlbum> for AlbumEntry {
    fn from(album: SourceAlbum) -> Self {
        Self {
            cover_art: format!("{ALBUM_COVER}{}", album.id),
            id: album.id,
            name: album.title,
            artist: album.artist.filter(|name| !name.is_empty()),
            artist_id: album.artist_id,
            song_count: album.song_count,
            song: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtistEntry {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    album_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    album: Option<Vec<AlbumEntry>>,
}

impl From<SourceArtist> for ArtistEntry {
    fn from(artist: SourceArtist) -> Self {
        Self {
            id: artist.id,
            name: artist.name,
            album_count: None,
            album: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistEntry {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<Vec<Child>>,
}

impl From<SourcePlaylist> for PlaylistEntry {
    fn from(playlist: SourcePlaylist) -> Self {
        Self {
            id: playlist.id,
            name: playlist.name,
            song_count: None,
            entry: None,
        }
    }
}

/// A Subsonic `jukeboxStatus`, or with `entry` a `jukeboxPlaylist`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JukeboxState {
    #[serde(skip_serializing_if = "Option::is_none")]
    current_index: Option<u32>,
    playing: bool,
    gain: f32,
    position: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<Vec<Child>>,
}

impl From<&PlayerState> for JukeboxState {
    fn from(player: &PlayerState) -> Self {
        Self {
            current_index: player.current_index,
            playing: player.playing,
            gain: 1.0,
            position: whole_seconds(player.position, f64::floor).unwrap_or_default(),
            entry: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use anyhow::{anyhow, bail};
    use reqwest::Client;
    use subsonic::{
        SearchLimits, SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicServerError,
    };
    use tokio::net::TcpStream;
    use url::Url;

    use super::*;
    use crate::test_wait::wait_until;

    const SONGS: [(&str, &str, u32); 3] =
        [("s1", "Alpha", 1), ("s2", "Beta", 2), ("s3", "Gamma", 3)];
    const PLAYLIST: [&str; 2] = ["s3", "s1"];

    #[derive(Default)]
    struct FakePlayer {
        ids: Vec<String>,
        current: Option<usize>,
        playing: bool,
        position: f64,
    }

    impl FakePlayer {
        fn state(&self) -> Value {
            let mut state =
                json!({ "playing": self.playing, "position": self.position, "ids": self.ids });
            if let Some(current) = self.current {
                state["currentIndex"] = json!(current);
            }
            state
        }
    }

    /// One album by one artist with three songs and a playlist, and a player that behaves like
    /// the bridge's jukebox.
    #[derive(Default)]
    struct FakeSource {
        requests: StdMutex<Vec<Value>>,
        player: StdMutex<FakePlayer>,
        denied: AtomicBool,
    }

    impl FakeSource {
        fn ops(&self, op: &str) -> Vec<Value> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request["op"] == op)
                .cloned()
                .collect()
        }

        fn song(id: &str) -> Option<Value> {
            SONGS
                .iter()
                .find(|(song, _, _)| *song == id)
                .map(|(id, title, track)| {
                    json!({
                        "id": id,
                        "title": title,
                        "artist": "Artist One",
                        "album": "Album One",
                        "duration": 61.6,
                        "playable": true,
                        "track": track,
                        "disc": 1,
                    })
                })
        }

        fn songs() -> Vec<Value> {
            SONGS
                .iter()
                .filter_map(|(id, _, _)| Self::song(id))
                .collect()
        }

        fn album() -> Value {
            json!({ "id": "al1", "title": "Album One", "artist": "Artist One", "songCount": 3 })
        }

        fn artist() -> Value {
            json!({ "id": "ar1", "name": "Artist One" })
        }

        fn playlist() -> Value {
            json!({ "id": "pl1", "name": "Mix" })
        }

        fn page(items: Vec<Value>, request: &Value) -> Value {
            let offset = request["offset"].as_u64().unwrap() as usize;
            let limit = request["limit"].as_u64().unwrap() as usize;
            Value::Array(items.into_iter().skip(offset).take(limit).collect())
        }

        fn found(item: Option<Value>, request: &Value) -> Result<Value> {
            item.ok_or_else(|| AppleMusicError::NotFound(format!("no {}", request["id"])).into())
        }

        fn library(&self, request: &Value) -> Result<Value> {
            let id = request["id"].as_str().unwrap_or_default();
            match request["op"].as_str().unwrap_or_default() {
                "songCount" => Ok(json!({ "count": Self::songs().len() })),
                "songs" => Ok(Self::page(Self::songs(), request)),
                "albums" => Ok(Self::page(vec![Self::album()], request)),
                "artists" => Ok(Self::page(vec![Self::artist()], request)),
                "playlists" => Ok(Self::page(vec![Self::playlist()], request)),
                "songsById" => Ok(Value::Array(
                    request["ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|id| Self::song(id.as_str().unwrap_or_default()))
                        .collect(),
                )),
                "song" => Self::found(Self::song(id), request),
                "album" => Self::found(
                    (id == "al1").then(|| {
                        let mut album = Self::album();
                        album["songs"] = Self::songs()
                            .into_iter()
                            .map(|mut song| {
                                song["albumId"] = json!("al1");
                                song
                            })
                            .collect();
                        album
                    }),
                    request,
                ),
                "artist" => Self::found(
                    (id == "ar1").then(|| {
                        let mut artist = Self::artist();
                        artist["albums"] = json!([Self::album()]);
                        artist
                    }),
                    request,
                ),
                "playlist" => Self::found(
                    (id == "pl1").then(|| {
                        let mut playlist = Self::playlist();
                        playlist["songs"] =
                            PLAYLIST.iter().filter_map(|id| Self::song(id)).collect();
                        playlist
                    }),
                    request,
                ),
                "search" => {
                    let term = request["term"].as_str().unwrap().to_lowercase();
                    let limit = request["limit"].as_u64().unwrap() as usize;
                    let matches = |text: &str| text.to_lowercase().contains(&term);
                    let songs: Vec<Value> = Self::songs()
                        .into_iter()
                        .filter(|song| matches(song["title"].as_str().unwrap()))
                        .take(limit)
                        .collect();
                    let albums = if matches("Album One") {
                        vec![Self::album()]
                    } else {
                        vec![]
                    };
                    let artists = if matches("Artist One") {
                        vec![Self::artist()]
                    } else {
                        vec![]
                    };
                    Ok(json!({ "songs": songs, "albums": albums, "artists": artists }))
                }
                op => self.play(op, request),
            }
        }

        fn play(&self, op: &str, request: &Value) -> Result<Value> {
            let mut player = self.player.lock().unwrap();
            let ids = || -> Vec<String> {
                request["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|id| id.as_str().filter(|id| Self::song(id).is_some()))
                    .map(str::to_owned)
                    .collect()
            };
            match op {
                "playerSet" => {
                    let ids = ids();
                    *player = FakePlayer {
                        current: (!ids.is_empty()).then_some(0),
                        ids,
                        ..FakePlayer::default()
                    };
                }
                "playerAdd" if player.ids.is_empty() => {
                    let ids = ids();
                    player.current = (!ids.is_empty()).then_some(0);
                    player.ids = ids;
                }
                "playerAdd" => player.ids.extend(ids()),
                "playerClear" => *player = FakePlayer::default(),
                "playerRemove" => {
                    let index = request["index"].as_u64().unwrap() as usize;
                    if index < player.ids.len() {
                        player.ids.remove(index);
                        player.current = match player.current {
                            _ if player.ids.is_empty() => None,
                            Some(current) if current > index => Some(current - 1),
                            Some(current) if current == player.ids.len() => Some(0),
                            current => current,
                        };
                    }
                }
                "playerSkip" => {
                    let index = request["index"].as_u64().unwrap() as usize;
                    if index >= player.ids.len() {
                        bail!(AppleMusicError::Service(format!("no queue entry {index}")));
                    }
                    player.current = Some(index);
                    player.position = request["offset"].as_f64().unwrap();
                }
                "playerStart" => player.playing = !player.ids.is_empty(),
                "playerStop" => player.playing = false,
                "playerGet" | "playerMode" => {}
                op => return Err(anyhow!("unknown op {op}")),
            }
            Ok(player.state())
        }
    }

    #[async_trait]
    impl AppleMusicSource for FakeSource {
        async fn request(&self, request: Value) -> Result<Value> {
            self.requests.lock().unwrap().push(request.clone());
            if self.denied.load(Ordering::SeqCst) {
                bail!(AppleMusicError::NotAuthorized(
                    "Apple Music access is not allowed".into()
                ));
            }
            self.library(&request)
        }
    }

    async fn start() -> (AppleMusicGateway, Arc<FakeSource>) {
        let source = Arc::new(FakeSource::default());
        let gateway = AppleMusicGateway::start(source.clone()).await.unwrap();
        (gateway, source)
    }

    /// The client the hub's Subsonic binding uses, signed in to `gateway`.
    fn client_for(gateway: &AppleMusicGateway, user: &str, password: &str) -> SubsonicClient {
        SubsonicClient::new(
            Url::parse(&gateway.endpoint()).unwrap(),
            SubsonicCredentials::Password {
                user: user.into(),
                password: password.into(),
            },
            Client::new(),
        )
    }

    fn client(gateway: &AppleMusicGateway) -> SubsonicClient {
        client_for(gateway, gateway.username(), gateway.password())
    }

    fn server_error(error: SubsonicError) -> (SubsonicServerError, String) {
        match error {
            SubsonicError::Server { kind, message } => (kind, message),
            other => panic!("expected a Subsonic server error, got {other:?}"),
        }
    }

    async fn ping_with_password(gateway: &AppleMusicGateway, password: &str) -> Value {
        let url = format!(
            "{}/rest/ping.view?u={}&p={password}&c=test&v=1.16.1&f=json",
            gateway.endpoint(),
            gateway.username()
        );
        Client::new()
            .get(url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn wrong_credentials_are_rejected_and_the_right_password_pings() {
        let (gateway, _) = start().await;

        let wrong_password = client_for(&gateway, gateway.username(), "not-it");
        let (kind, _) = server_error(wrong_password.probe().await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::WrongCredentials);
        let wrong_user = client_for(&gateway, "alice", gateway.password());
        let (kind, _) = server_error(wrong_user.probe().await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::WrongCredentials);

        let probe = client(&gateway).probe().await.unwrap();
        assert_eq!(probe.api_version, API_VERSION);

        let plain = ping_with_password(&gateway, gateway.password()).await;
        assert_eq!(plain["subsonic-response"]["status"], "ok");
        let hex: String = gateway
            .password()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let encoded = ping_with_password(&gateway, &format!("enc:{hex}")).await;
        assert_eq!(encoded["subsonic-response"]["status"], "ok");
        let wrong = ping_with_password(&gateway, "not-it").await;
        assert_eq!(wrong["subsonic-response"]["status"], "failed");
        assert_eq!(wrong["subsonic-response"]["error"]["code"], 40);
    }

    #[tokio::test]
    async fn every_gateway_has_its_own_password() {
        let (first, _) = start().await;
        let (second, _) = start().await;
        assert_ne!(first.password(), second.password());
        assert_eq!(first.password().len(), 32);
    }

    #[tokio::test]
    async fn the_probe_sees_jukebox_playback_only() {
        let (gateway, _) = start().await;
        let probe = client(&gateway).probe().await.unwrap();
        assert!(probe.roles_known);
        assert!(probe.roles.jukebox_role);
        assert!(!probe.roles.stream_role);
        assert!(!probe.roles.download_role);
        assert!(!probe.open_subsonic);
    }

    #[tokio::test]
    async fn the_library_lists_in_pages_and_details_keep_their_order() {
        let (gateway, source) = start().await;
        let client = client(&gateway);
        let songs_only = SearchLimits {
            songs: 2,
            ..SearchLimits::default()
        };

        let first = client
            .search_with_song_offset("", songs_only, 0)
            .await
            .unwrap();
        let ids: Vec<&str> = first.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
        assert!(first.albums.is_empty() && first.artists.is_empty());
        let song = &first.songs[0];
        assert_eq!(song.title, "Alpha");
        assert_eq!(song.artist.as_deref(), Some("Artist One"));
        assert_eq!(song.album.as_deref(), Some("Album One"));
        assert_eq!(song.duration, Some(62));
        assert_eq!((song.track, song.disc), (Some(1), Some(1)));
        let rest = client
            .search_with_song_offset("", songs_only, 2)
            .await
            .unwrap();
        let ids: Vec<&str> = rest.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["s3"]);
        assert!(source.ops("search").is_empty());

        let collections = client
            .search(
                "",
                SearchLimits {
                    artists: 5,
                    albums: 5,
                    songs: 0,
                },
            )
            .await
            .unwrap();
        assert!(collections.songs.is_empty());
        assert_eq!(collections.albums[0].id, "al1");
        assert_eq!(collections.artists[0].id, "ar1");

        let albums = client.album_list(500, 0).await.unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Album One");
        assert_eq!(albums[0].song_count, Some(3));
        assert!(client.album_list(500, 1).await.unwrap().is_empty());

        let artists = client.artists().await.unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Artist One");
        let artist = client.artist("ar1").await.unwrap();
        assert_eq!(artist.albums.len(), 1);
        assert_eq!(artist.albums[0].artist_id.as_deref(), Some("ar1"));

        let album = client.album("al1").await.unwrap();
        let titles: Vec<&str> = album.songs.iter().map(|song| song.title.as_str()).collect();
        assert_eq!(titles, ["Alpha", "Beta", "Gamma"]);
        assert!(
            album
                .songs
                .iter()
                .all(|song| song.album_id.as_deref() == Some("al1"))
        );

        let playlists = client.playlists().await.unwrap();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].name, "Mix");
        let playlist = client.playlist("pl1").await.unwrap();
        let ids: Vec<&str> = playlist.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, PLAYLIST);
        assert_eq!(playlist.song_count, Some(2));

        let song = client.song("s2").await.unwrap();
        assert_eq!(song.title, "Beta");
        // MusicKit named only the album and artist; their ids were found by those names.
        assert_eq!(song.album_id.as_deref(), Some("al1"));
        assert_eq!(song.artist_id.as_deref(), Some("ar1"));
        let (kind, _) = server_error(client.song("gone").await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::NotFound);
        assert!(!client.scan_status().await.unwrap().scanning);
        assert!(client.cover_art("al1", None).await.is_err());
    }

    #[test]
    fn artists_are_indexed_by_initial() {
        assert_eq!(index_name("artist one"), "A");
        assert_eq!(index_name("Émile"), "É");
        assert_eq!(index_name("2Pac"), "#");
        assert_eq!(index_name(""), "#");
    }

    #[tokio::test]
    async fn a_text_search_asks_musickit_search() {
        let (gateway, source) = start().await;
        let found = client(&gateway)
            .search(
                "gam*",
                SearchLimits {
                    artists: 10,
                    albums: 10,
                    songs: 10,
                },
            )
            .await
            .unwrap();
        let ids: Vec<&str> = found.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["s3"]);
        assert!(found.albums.is_empty() && found.artists.is_empty());
        let searches = source.ops("search");
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0]["term"], "gam");
        assert_eq!(searches[0]["limit"], 10);
        assert!(source.ops("songs").is_empty());
    }

    #[tokio::test]
    async fn the_jukebox_follows_set_start_skip_remove_and_clear() {
        let (gateway, source) = start().await;
        let client = client(&gateway);

        client.jukebox_set(&["s1", "s2", "s3"]).await.unwrap();
        let jukebox = client.jukebox_get().await.unwrap();
        let ids: Vec<&str> = jukebox
            .playlist
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect();
        assert_eq!(ids, ["s1", "s2", "s3"]);
        assert!(!jukebox.status.playing);
        assert_eq!(jukebox.status.current_index, Some(0));
        assert_eq!(jukebox.status.gain, 1.0);
        assert_eq!(jukebox.playlist.songs[1].title, "Beta");

        client.jukebox_playback_mode("repeatAll").await.unwrap();
        assert_eq!(source.ops("playerMode").len(), 1);
        client.jukebox_start().await.unwrap();
        assert!(client.jukebox_get().await.unwrap().status.playing);
        // The queue's songs were read together, once; polling reuses them.
        assert_eq!(source.ops("songsById").len(), 1);

        client.jukebox_skip(2, 5).await.unwrap();
        let status = client.jukebox_status().await.unwrap();
        assert_eq!(status.current_index, Some(2));
        assert_eq!(status.position_secs, Some(5));
        assert!(status.playing);

        client.jukebox_remove(0).await.unwrap();
        let jukebox = client.jukebox_get().await.unwrap();
        let ids: Vec<&str> = jukebox
            .playlist
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect();
        assert_eq!(ids, ["s2", "s3"]);
        assert_eq!(jukebox.status.current_index, Some(1));

        client.jukebox_add(&["s1"]).await.unwrap();
        assert_eq!(client.jukebox_get().await.unwrap().playlist.songs.len(), 3);
        client.jukebox_set_gain(0.5).await.unwrap();
        client.jukebox_stop().await.unwrap();
        assert!(!client.jukebox_status().await.unwrap().playing);

        client.jukebox_clear().await.unwrap();
        let jukebox = client.jukebox_get().await.unwrap();
        assert!(jukebox.playlist.songs.is_empty());
        assert_eq!(jukebox.status.current_index, None);
        assert!(!jukebox.status.playing);

        // Shuffling fails without making the client think the jukebox is missing.
        let (kind, _) = server_error(client.jukebox_shuffle().await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::Generic);
        let (kind, _) = server_error(client.jukebox_skip(9, 0).await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::Generic);
    }

    #[tokio::test]
    async fn playlist_and_favorite_changes_are_refused_without_a_credential_error() {
        let (gateway, source) = start().await;
        let client = client(&gateway);
        let refusals = [
            client.create_playlist("New", &["s1"]).await.unwrap_err(),
            client
                .replace_playlist_songs("pl1", &["s1"])
                .await
                .unwrap_err(),
            client
                .update_playlist("pl1", Some("Renamed"), &[], &[0])
                .await
                .unwrap_err(),
            client.delete_playlist("pl1").await.unwrap_err(),
        ];
        for refusal in refusals {
            let (kind, message) = server_error(refusal);
            assert_eq!(kind, SubsonicServerError::Generic);
            assert_eq!(message, PLAYLISTS_READ_ONLY);
        }
        let (kind, _) = server_error(client.star(&["s1"]).await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::Generic);
        assert!(source.requests.lock().unwrap().is_empty());
        assert_eq!(client.random_songs(2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn musickit_permission_errors_do_not_read_as_a_wrong_password() {
        let (gateway, source) = start().await;
        source.denied.store(true, Ordering::SeqCst);
        let client = client(&gateway);

        let (kind, message) = server_error(client.album("al1").await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::Generic);
        assert!(message.contains("not allowed"));
        let (kind, _) = server_error(client.jukebox_get().await.unwrap_err());
        assert_eq!(kind, SubsonicServerError::Generic);
        // The account itself still signs in.
        assert!(client.probe().await.is_ok());
    }

    #[tokio::test]
    async fn dropping_the_gateway_stops_serving() {
        let (gateway, _) = start().await;
        let address = gateway.address;
        client(&gateway).probe().await.unwrap();
        drop(gateway);
        wait_until("the gateway port to close", || async move {
            Ok(TcpStream::connect(address).await.is_err().then_some(()))
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_whole_library_can_be_queued() {
        let (gateway, _source) = start().await;
        let client = client(&gateway);
        let ids: Vec<String> = (0..1289)
            .map(|index| format!("i.dlvq2J9Uz3Xg{index:04}"))
            .collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let result = client.jukebox_set(&refs).await;
        assert!(result.is_ok(), "{result:?}");
    }
}
