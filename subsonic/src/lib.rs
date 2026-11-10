use std::fmt;

use discovery::endpoint::{EndpointUrl, HttpTransport};
use rand::RngCore;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use thiserror::Error;
use url::Url;

const CLIENT_VERSION: &str = "1.16.1";

/// Credentials supplied by the caller. Secrets are deliberately omitted from Debug.
#[derive(Clone)]
pub enum SubsonicCredentials {
    Password { user: String, password: String },
    ApiKey { key: String },
}

impl SubsonicCredentials {
    /// Return the account name when it is part of the selected authentication method.
    pub fn user(&self) -> Option<&str> {
        match self {
            Self::Password { user, .. } => Some(user),
            Self::ApiKey { .. } => None,
        }
    }
}

impl fmt::Debug for SubsonicCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password { user, .. } => formatter
                .debug_struct("Password")
                .field("user", user)
                .field("password", &"[REDACTED]")
                .finish(),
            Self::ApiKey { .. } => formatter
                .debug_struct("ApiKey")
                .field("key", &"[REDACTED]")
                .finish(),
        }
    }
}

/// Typed Subsonic failures. Transport errors drop their URL, which carries the
/// authentication query parameters, but keep the cause.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SubsonicError {
    #[error("Subsonic transport failed: {0}")]
    Transport(String),
    #[error("Subsonic returned HTTP status {0}")]
    HttpStatus(StatusCode),
    #[error("Subsonic request failed: {kind:?}: {message}")]
    Server {
        kind: SubsonicServerError,
        message: String,
    },
    #[error("malformed Subsonic response: {0}")]
    Malformed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsonicServerError {
    Generic,
    MissingParameter,
    ClientTooOld,
    ServerTooOld,
    WrongCredentials,
    TokenAuthenticationNotSupported,
    AuthenticationMechanismNotSupported,
    ConflictingAuthentication,
    InvalidApiKey,
    NotAuthorized,
    NotFound,
    Unknown(i64),
}

impl From<i64> for SubsonicServerError {
    fn from(code: i64) -> Self {
        match code {
            0 => Self::Generic,
            10 => Self::MissingParameter,
            20 => Self::ClientTooOld,
            30 => Self::ServerTooOld,
            40 => Self::WrongCredentials,
            41 => Self::TokenAuthenticationNotSupported,
            42 => Self::AuthenticationMechanismNotSupported,
            43 => Self::ConflictingAuthentication,
            44 => Self::InvalidApiKey,
            50 => Self::NotAuthorized,
            70 => Self::NotFound,
            code => Self::Unknown(code),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsonicRoles {
    pub stream_role: bool,
    pub download_role: bool,
    pub jukebox_role: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsonicProbe {
    pub api_version: String,
    pub open_subsonic: bool,
    pub server_type: Option<String>,
    pub server_version: Option<String>,
    pub extensions: Vec<(String, Vec<u32>)>,
    pub roles: SubsonicRoles,
    pub roles_known: bool,
}

/// A Subsonic client with caller-owned credentials and an HTTP client.
pub struct SubsonicClient {
    base_url: Url,
    credentials: SubsonicCredentials,
    http: Client,
}

impl SubsonicClient {
    pub fn new(base_url: Url, credentials: SubsonicCredentials, http: Client) -> Self {
        Self {
            base_url,
            credentials,
            http,
        }
    }

    pub fn from_endpoint(
        endpoint: &EndpointUrl,
        credentials: SubsonicCredentials,
        http: Client,
    ) -> Result<Self, SubsonicError> {
        let EndpointUrl::Subsonic {
            transport, base, ..
        } = endpoint
        else {
            return Err(SubsonicError::Malformed("endpoint is not Subsonic".into()));
        };
        let scheme = match transport {
            HttpTransport::Http => "http",
            HttpTransport::Https => "https",
        };
        let url = Url::parse(&format!("{scheme}://{base}"))
            .map_err(|error| SubsonicError::Malformed(error.to_string()))?;
        Ok(Self::new(url, credentials, http))
    }

    /// Probe in this order: public ping, public OpenSubsonic extensions, then the
    /// authenticated account lookup (tokenInfo followed by getUser for API keys).
    pub async fn probe(&self) -> Result<SubsonicProbe, SubsonicError> {
        let ping: PingResponse = self.call("ping", &[]).await?;
        let mut extensions = Vec::new();
        if ping.open_subsonic {
            let response: ExtensionsResponse = self.call("getOpenSubsonicExtensions", &[]).await?;
            extensions = response
                .extensions
                .unwrap_or_default()
                .into_iter()
                .map(|extension| (extension.name, extension.versions.unwrap_or_default()))
                .collect();
        }
        let username = match &self.credentials {
            SubsonicCredentials::Password { user, .. } => user.clone(),
            SubsonicCredentials::ApiKey { .. } => {
                let response: TokenInfoResponse = self.call("tokenInfo", &[]).await?;
                response
                    .token_info
                    .and_then(|info| info.username)
                    .ok_or_else(|| SubsonicError::Malformed("tokenInfo omitted username".into()))?
            }
        };
        let user_response: UserResponse = self
            .call("getUser", &[("username", username.as_str())])
            .await?;
        let roles = user_response
            .user
            .map(|user| SubsonicRoles {
                stream_role: user.stream_role.unwrap_or(false),
                download_role: user.download_role.unwrap_or(false),
                jukebox_role: user.jukebox_role.unwrap_or(false),
            })
            .ok_or_else(|| SubsonicError::Malformed("getUser omitted user".into()))?;
        Ok(SubsonicProbe {
            api_version: ping.version,
            open_subsonic: ping.open_subsonic,
            server_type: ping.server_type,
            server_version: ping.server_version,
            extensions,
            roles,
            roles_known: true,
        })
    }

    /// List ID3 artists.
    pub async fn artists(&self) -> Result<Vec<SubsonicArtist>, SubsonicError> {
        let response: ArtistsResponse = self.call("getArtists", &[]).await?;
        Ok(response
            .artists
            .and_then(|artists| artists.index)
            .map(OneOrMany::into_vec)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|index| index.artist.map(OneOrMany::into_vec).unwrap_or_default())
            .collect())
    }

    /// Read an artist and its albums.
    pub async fn artist(&self, id: &str) -> Result<SubsonicArtistDetail, SubsonicError> {
        let response: ArtistResponse = self.call("getArtist", &[("id", id)]).await?;
        response
            .artist
            .ok_or_else(|| SubsonicError::Malformed("getArtist omitted artist".into()))
    }

    /// Read an album and its songs.
    pub async fn album(&self, id: &str) -> Result<SubsonicAlbumDetail, SubsonicError> {
        let response: AlbumResponse = self.call("getAlbum", &[("id", id)]).await?;
        response
            .album
            .ok_or_else(|| SubsonicError::Malformed("getAlbum omitted album".into()))
    }

    /// Read a song.
    pub async fn song(&self, id: &str) -> Result<SubsonicSong, SubsonicError> {
        let response: SongResponse = self.call("getSong", &[("id", id)]).await?;
        response
            .song
            .ok_or_else(|| SubsonicError::Malformed("getSong omitted song".into()))
    }

    /// Search the ID3 library.
    pub async fn search(
        &self,
        query: &str,
        limits: SearchLimits,
    ) -> Result<SubsonicSearchResult, SubsonicError> {
        let artist_count = limits.artists.to_string();
        let album_count = limits.albums.to_string();
        let song_count = limits.songs.to_string();
        let response: SearchResponse = self
            .call(
                "search3",
                &[
                    ("query", query),
                    ("artistCount", &artist_count),
                    ("albumCount", &album_count),
                    ("songCount", &song_count),
                ],
            )
            .await?;
        Ok(response.search_result.unwrap_or_default())
    }

    /// List playlists visible to the authenticated user.
    pub async fn playlists(&self) -> Result<Vec<SubsonicPlaylist>, SubsonicError> {
        let response: PlaylistsResponse = self.call("getPlaylists", &[]).await?;
        Ok(response
            .playlists
            .and_then(|value| value.playlist)
            .map(OneOrMany::into_vec)
            .unwrap_or_default())
    }

    /// Read a playlist and its entries.
    pub async fn playlist(&self, id: &str) -> Result<SubsonicPlaylistDetail, SubsonicError> {
        let response: PlaylistResponse = self.call("getPlaylist", &[("id", id)]).await?;
        response
            .playlist
            .ok_or_else(|| SubsonicError::Malformed("getPlaylist omitted playlist".into()))
    }

    /// Build a stream URL with the same authentication parameters used by requests.
    pub fn stream_url(&self, song_id: &str, options: StreamOptions) -> Url {
        let mut url = self.endpoint_url("stream");
        let mut query = self.auth_query();
        query.push(("id", song_id.to_owned()));
        if let Some(format) = options.format {
            query.push(("format", format));
        }
        if let Some(bit_rate) = options.max_bit_rate {
            query.push(("maxBitRate", bit_rate.to_string()));
        }
        url.query_pairs_mut().extend_pairs(query);
        url
    }

    /// Build a cover-art URL with the same authentication parameters used by requests.
    pub fn cover_art_url(&self, id: &str, size: Option<u32>) -> Url {
        let mut url = self.endpoint_url("getCoverArt");
        let mut query = self.auth_query();
        query.push(("id", id.to_owned()));
        if let Some(size) = size {
            query.push(("size", size.to_string()));
        }
        url.query_pairs_mut().extend_pairs(query);
        url
    }

    fn endpoint_url(&self, endpoint: &str) -> Url {
        let mut url = self.base_url.clone();
        let path = format!(
            "{}/rest/{}.view",
            url.path().trim_end_matches('/'),
            endpoint
        );
        url.set_path(&path);
        url
    }

    fn auth_query(&self) -> Vec<(&'static str, String)> {
        let mut query = vec![
            ("c", "rune".to_owned()),
            ("v", CLIENT_VERSION.to_owned()),
            ("f", "json".to_owned()),
        ];
        match &self.credentials {
            SubsonicCredentials::Password { user, password } => {
                let salt = random_salt();
                let digest = md5::compute(format!("{password}{salt}"));
                query.push(("u", user.clone()));
                query.push(("t", format!("{digest:x}")));
                query.push(("s", salt));
            }
            SubsonicCredentials::ApiKey { key } => query.push(("apiKey", key.clone())),
        }
        query
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<T, SubsonicError> {
        let url = self.endpoint_url(endpoint);
        let mut query = self.auth_query();
        query.extend(
            params
                .iter()
                .map(|(key, value)| (*key, (*value).to_owned())),
        );
        let response = self
            .http
            .get(url)
            .query(&query)
            .send()
            .await
            .map_err(|error| SubsonicError::Transport(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(SubsonicError::HttpStatus(status));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| SubsonicError::Transport(error.without_url().to_string()))?;
        let envelope: RawEnvelope = serde_json::from_slice(&bytes)
            .map_err(|error| SubsonicError::Malformed(error.to_string()))?;
        if envelope.response.status == "failed" {
            let error = envelope
                .response
                .error
                .ok_or_else(|| SubsonicError::Malformed("failed response omitted error".into()))?;
            let mut message = error.message;
            if let SubsonicCredentials::Password { password, .. } = &self.credentials {
                message = message.replace(password, "[REDACTED]");
            }
            for (key, secret) in &query {
                if matches!(*key, "t" | "s" | "apiKey") {
                    message = message.replace(secret, "[REDACTED]");
                }
            }
            return Err(SubsonicError::Server {
                kind: error.code.into(),
                message,
            });
        }
        if envelope.response.status != "ok" {
            return Err(SubsonicError::Malformed("unknown response status".into()));
        }
        serde_json::from_value(serde_json::Value::Object(envelope.response.data))
            .map_err(|error| SubsonicError::Malformed(error.to_string()))
    }
}

fn random_salt() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicArtist {
    pub id: String,
    pub name: String,
    pub album_count: Option<u32>,
    pub cover_art: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsonicArtistDetail {
    pub id: String,
    pub name: String,
    pub album_count: Option<u32>,
    pub cover_art: Option<String>,
    pub albums: Vec<SubsonicAlbum>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicAlbum {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsonicAlbumDetail {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub songs: Vec<SubsonicSong>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicSong {
    pub id: String,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub track: Option<u32>,
    #[serde(rename = "discNumber")]
    pub disc: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub duration: Option<u32>,
    pub size: Option<u64>,
    pub suffix: Option<String>,
    pub content_type: Option<String>,
    pub cover_art: Option<String>,
    pub bit_rate: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchLimits {
    pub artists: u32,
    pub albums: u32,
    pub songs: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsonicSearchResult {
    pub artists: Vec<SubsonicArtist>,
    pub albums: Vec<SubsonicAlbum>,
    pub songs: Vec<SubsonicSong>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicPlaylist {
    pub id: String,
    pub name: String,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub cover_art: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsonicPlaylistDetail {
    pub id: String,
    pub name: String,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub cover_art: Option<String>,
    pub songs: Vec<SubsonicSong>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamOptions {
    pub format: Option<String>,
    pub max_bit_rate: Option<u32>,
}

#[derive(Deserialize)]
struct ArtistsResponse {
    #[serde(default)]
    artists: Option<ArtistsContainer>,
}
#[derive(Deserialize)]
struct ArtistsContainer {
    #[serde(default)]
    index: Option<OneOrMany<ArtistIndex>>,
}
#[derive(Deserialize)]
struct ArtistIndex {
    #[serde(default)]
    artist: Option<OneOrMany<SubsonicArtist>>,
}
#[derive(Deserialize)]
struct ArtistResponse {
    #[serde(default)]
    artist: Option<SubsonicArtistDetail>,
}
#[derive(Deserialize)]
struct AlbumResponse {
    #[serde(default)]
    album: Option<SubsonicAlbumDetail>,
}
#[derive(Deserialize)]
struct SongResponse {
    #[serde(default)]
    song: Option<SubsonicSong>,
}
#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default, rename = "searchResult3")]
    search_result: Option<SubsonicSearchResult>,
}
#[derive(Deserialize)]
struct PlaylistsResponse {
    #[serde(default)]
    playlists: Option<PlaylistList>,
}
#[derive(Deserialize)]
struct PlaylistList {
    #[serde(default)]
    playlist: Option<OneOrMany<SubsonicPlaylist>>,
}
#[derive(Deserialize)]
struct PlaylistResponse {
    #[serde(default)]
    playlist: Option<SubsonicPlaylistDetail>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            Self::Many(values) => values,
            Self::One(value) => vec![value],
        }
    }
}

impl<'de> Deserialize<'de> for SubsonicArtistDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawArtist {
            id: String,
            name: String,
            album_count: Option<u32>,
            cover_art: Option<String>,
            #[serde(rename = "album")]
            albums: Option<OneOrMany<SubsonicAlbum>>,
        }
        let raw = RawArtist::deserialize(deserializer)?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            album_count: raw.album_count,
            cover_art: raw.cover_art,
            albums: raw.albums.map(OneOrMany::into_vec).unwrap_or_default(),
        })
    }
}

impl<'de> Deserialize<'de> for SubsonicAlbumDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawAlbum {
            id: String,
            name: String,
            artist: Option<String>,
            artist_id: Option<String>,
            song_count: Option<u32>,
            duration: Option<u32>,
            year: Option<u32>,
            genre: Option<String>,
            cover_art: Option<String>,
            song: Option<OneOrMany<SubsonicSong>>,
        }
        let raw = RawAlbum::deserialize(deserializer)?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            artist: raw.artist,
            artist_id: raw.artist_id,
            song_count: raw.song_count,
            duration: raw.duration,
            year: raw.year,
            genre: raw.genre,
            cover_art: raw.cover_art,
            songs: raw.song.map(OneOrMany::into_vec).unwrap_or_default(),
        })
    }
}

impl<'de> Deserialize<'de> for SubsonicPlaylistDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawPlaylist {
            id: String,
            name: String,
            song_count: Option<u32>,
            duration: Option<u32>,
            owner: Option<String>,
            public: Option<bool>,
            cover_art: Option<String>,
            entry: Option<OneOrMany<SubsonicSong>>,
        }
        let raw = RawPlaylist::deserialize(deserializer)?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            song_count: raw.song_count,
            duration: raw.duration,
            owner: raw.owner,
            public: raw.public,
            cover_art: raw.cover_art,
            songs: raw.entry.map(OneOrMany::into_vec).unwrap_or_default(),
        })
    }
}

impl<'de> Deserialize<'de> for SubsonicSearchResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawSearchResult {
            #[serde(default)]
            artist: Option<OneOrMany<SubsonicArtist>>,
            #[serde(default)]
            album: Option<OneOrMany<SubsonicAlbum>>,
            #[serde(default)]
            song: Option<OneOrMany<SubsonicSong>>,
        }
        let raw = RawSearchResult::deserialize(deserializer)?;
        Ok(Self {
            artists: raw.artist.map(OneOrMany::into_vec).unwrap_or_default(),
            albums: raw.album.map(OneOrMany::into_vec).unwrap_or_default(),
            songs: raw.song.map(OneOrMany::into_vec).unwrap_or_default(),
        })
    }
}

#[derive(Deserialize)]
struct RawEnvelope {
    #[serde(rename = "subsonic-response")]
    response: RawResponse,
}
#[derive(Deserialize)]
struct RawResponse {
    status: String,
    #[serde(default)]
    error: Option<RawError>,
    #[serde(flatten)]
    data: serde_json::Map<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct RawError {
    code: i64,
    message: String,
}
#[derive(Deserialize)]
struct PingResponse {
    version: String,
    #[serde(default, rename = "openSubsonic")]
    open_subsonic: bool,
    #[serde(default, rename = "type")]
    server_type: Option<String>,
    #[serde(default, rename = "serverVersion")]
    server_version: Option<String>,
}
#[derive(Deserialize)]
struct ExtensionsResponse {
    #[serde(default, rename = "openSubsonicExtensions")]
    extensions: Option<Vec<RawExtension>>,
}
#[derive(Deserialize)]
struct RawExtension {
    name: String,
    #[serde(default)]
    versions: Option<Vec<u32>>,
}
#[derive(Deserialize)]
struct TokenInfoResponse {
    #[serde(default)]
    #[serde(rename = "tokenInfo")]
    token_info: Option<RawTokenInfo>,
}
#[derive(Deserialize)]
struct RawTokenInfo {
    #[serde(default)]
    username: Option<String>,
}
#[derive(Deserialize)]
struct UserResponse {
    #[serde(default)]
    user: Option<RawUser>,
}
#[derive(Deserialize)]
struct RawUser {
    #[serde(default, rename = "streamRole")]
    stream_role: Option<bool>,
    #[serde(default, rename = "downloadRole")]
    download_role: Option<bool>,
    #[serde(default, rename = "jukeboxRole")]
    jukebox_role: Option<bool>,
}
