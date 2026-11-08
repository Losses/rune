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

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<T, SubsonicError> {
        let mut url = self.base_url.clone();
        let path = format!(
            "{}/rest/{}.view",
            url.path().trim_end_matches('/'),
            endpoint
        );
        url.set_path(&path);
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
            return Err(SubsonicError::Server {
                kind: error.code.into(),
                message: error.message,
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
