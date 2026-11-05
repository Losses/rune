//! Exchange URLs for endpoints. Optional `opt-` hints are ignored for versioned adapters.
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use thiserror::Error;
use url::{Url, form_urlencoded};
use uuid::Uuid;

/// Splits a trusted host entry (`host`, `host:port`, `ip`, `ip:port`, `[v6]:port`) into
/// its host and optional port. Returns `None` for an entry with port 0 or no host.
pub fn endpoint_host_port(value: &str) -> Option<(String, Option<u16>)> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return (addr.port() != 0).then(|| (addr.ip().to_string(), Some(addr.port())));
    }
    let host = value.trim_matches(|c| c == '[' || c == ']');
    if host.parse::<IpAddr>().is_ok() {
        return Some((host.to_owned(), None));
    }
    if let Some((name, port)) = value.rsplit_once(':')
        && !name.is_empty()
        && !name.contains(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return (port != 0).then(|| (name.to_owned(), Some(port)));
    }
    (!value.is_empty() && !value.contains(':')).then(|| (value.to_owned(), None))
}

/// Current version of the endpoint exchange format.
pub const ENDPOINT_URL_VERSION: u32 = 1;

/// A lowercase, unpadded base32 fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Validate and construct a fingerprint. Uppercase input, as QR codes prefer, is
    /// accepted and normalised to lowercase.
    pub fn new(value: impl Into<String>) -> Result<Self, EndpointUrlError> {
        let value = value.into().to_ascii_lowercase();
        if value.len() != 52
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        {
            return Err(EndpointUrlError::InvalidFingerprint);
        }
        Ok(Self(value))
    }
}
impl FromStr for Fingerprint {
    type Err = EndpointUrlError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}
impl Display for Fingerprint {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An object within a Rune endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointTarget {
    Library(Uuid),
    Session(String),
}
/// Transport used by a Subsonic endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpTransport {
    Http,
    Https,
}
/// An endpoint exchange address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointUrl {
    Rune {
        fingerprint: Fingerprint,
        addrs: Vec<SocketAddr>,
        target: Option<EndpointTarget>,
    },
    Subsonic {
        transport: HttpTransport,
        base: String,
        user: Option<String>,
    },
    OpenDal {
        service: String,
        location: String,
        params: BTreeMap<String, String>,
    },
    File {
        path: String,
    },
    Content {
        uri: String,
    },
}
/// Errors in an endpoint exchange address.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum EndpointUrlError {
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    #[error("missing version")]
    MissingVersion,
    #[error("unsupported version {found}; supported version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("unknown parameter: {0}")]
    UnknownParameter(String),
    #[error("invalid fingerprint")]
    InvalidFingerprint,
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("secret not allowed: {0}")]
    SecretNotAllowed(String),
    #[error("malformed URL: {0}")]
    Malformed(String),
}

fn secret(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "password"
            | "pass"
            | "p"
            | "token"
            | "t"
            | "s"
            | "secret"
            | "key"
            | "access_key"
            | "secret_key"
            | "apikey"
            | "api_key"
    )
}

impl FromStr for EndpointUrl {
    type Err = EndpointUrlError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (scheme, rest) = input
            .split_once("://")
            .ok_or_else(|| EndpointUrlError::Malformed(input.into()))?;
        let scheme = scheme.to_ascii_lowercase();
        let service = scheme.strip_prefix("opendal+");
        if !matches!(
            scheme.as_str(),
            "rune" | "subsonic+http" | "subsonic+https" | "file" | "content"
        ) && !service.is_some_and(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        }) {
            return Err(EndpointUrlError::UnsupportedScheme(scheme));
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority
            .rsplit_once('@')
            .is_some_and(|(userinfo, _)| userinfo.contains(':'))
        {
            return Err(EndpointUrlError::SecretNotAllowed("password".into()));
        }
        let parsed = Url::parse(input).map_err(|e| EndpointUrlError::Malformed(e.to_string()))?;
        if parsed.password().is_some() {
            return Err(EndpointUrlError::SecretNotAllowed("password".into()));
        }
        let query_source = if scheme == "file" || scheme == "content" {
            rest.split('#').next().unwrap_or("")
        } else {
            rest
        };
        let (body, query) = query_source.split_once('?').unwrap_or((query_source, ""));
        if scheme != "file" && scheme != "content" && (body.contains('#') || query.contains('#')) {
            return Err(EndpointUrlError::Malformed("fragment not allowed".into()));
        }
        let query = query.split('#').next().unwrap_or("");
        let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        for (key, _) in &pairs {
            if secret(key) {
                return Err(EndpointUrlError::SecretNotAllowed(key.clone()));
            }
        }
        if scheme == "file" {
            return Ok(Self::File { path: input.into() });
        }
        if scheme == "content" {
            return Ok(Self::Content { uri: input.into() });
        }
        let (authority, path) = body
            .split_once('/')
            .map_or((body, ""), |(a, _)| (a, &body[a.len()..]));
        if authority.is_empty() {
            return Err(EndpointUrlError::Malformed("missing authority".into()));
        }
        let mut version = None;
        let mut addrs = Vec::new();
        let mut user = None;
        let mut params = BTreeMap::new();
        for (key, value) in pairs {
            if key.starts_with("opt-") {
                continue;
            }
            if key == "v" {
                if version.is_some() {
                    return Err(EndpointUrlError::Malformed("duplicate version".into()));
                }
                version = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| EndpointUrlError::Malformed("invalid version".into()))?,
                );
            } else if scheme == "rune" && key == "addr" {
                addrs.push(
                    value
                        .parse()
                        .map_err(|_| EndpointUrlError::InvalidAddress(value))?,
                );
            } else if scheme.starts_with("subsonic+") && key == "user" {
                if user.replace(value).is_some() {
                    return Err(EndpointUrlError::Malformed("duplicate user".into()));
                }
            } else if service.is_some() {
                if params.insert(key, value).is_some() {
                    return Err(EndpointUrlError::Malformed("duplicate parameter".into()));
                }
            } else {
                return Err(EndpointUrlError::UnknownParameter(key));
            }
        }
        let version = version.ok_or(EndpointUrlError::MissingVersion)?;
        if version > ENDPOINT_URL_VERSION {
            return Err(EndpointUrlError::UnsupportedVersion {
                found: version,
                supported: ENDPOINT_URL_VERSION,
            });
        }
        if scheme == "rune" {
            if authority.contains(':') {
                return Err(EndpointUrlError::Malformed("port in Rune authority".into()));
            }
            let fingerprint = Fingerprint::new(authority)?;
            let target = if path.is_empty() {
                None
            } else if let Some(id) = path.strip_prefix("/libraries/") {
                Some(EndpointTarget::Library(
                    id.parse()
                        .map_err(|_| EndpointUrlError::InvalidPath(path.into()))?,
                ))
            } else if let Some(id) = path.strip_prefix("/sessions/") {
                if id.is_empty() || id.contains('/') {
                    return Err(EndpointUrlError::InvalidPath(path.into()));
                }
                let decoded = form_urlencoded::parse(format!("x={id}").as_bytes())
                    .next()
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default();
                Some(EndpointTarget::Session(decoded))
            } else {
                return Err(EndpointUrlError::InvalidPath(path.into()));
            };
            Ok(Self::Rune {
                fingerprint,
                addrs,
                target,
            })
        } else if let Some(service) = service {
            Ok(Self::OpenDal {
                service: service.into(),
                location: body.into(),
                params,
            })
        } else {
            let transport = if scheme == "subsonic+http" {
                HttpTransport::Http
            } else {
                HttpTransport::Https
            };
            Ok(Self::Subsonic {
                transport,
                base: body.into(),
                user,
            })
        }
    }
}

impl Display for EndpointUrl {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair("v", &ENDPOINT_URL_VERSION.to_string());
        match self {
            Self::File { path } => return f.write_str(path),
            Self::Content { uri } => return f.write_str(uri),
            Self::Rune {
                fingerprint,
                addrs,
                target,
            } => {
                write!(f, "rune://{fingerprint}")?;
                match target {
                    Some(EndpointTarget::Library(id)) => write!(f, "/libraries/{id}")?,
                    Some(EndpointTarget::Session(id)) => {
                        let encoded =
                            form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
                        write!(f, "/sessions/{encoded}")?;
                    }
                    None => {}
                }
                for addr in addrs {
                    query.append_pair("addr", &addr.to_string());
                }
            }
            Self::Subsonic {
                transport,
                base,
                user,
            } => {
                let transport = match transport {
                    HttpTransport::Http => "http",
                    HttpTransport::Https => "https",
                };
                write!(f, "subsonic+{transport}://{base}")?;
                if let Some(user) = user {
                    query.append_pair("user", user);
                }
            }
            Self::OpenDal {
                service,
                location,
                params,
            } => {
                write!(f, "opendal+{service}://{location}")?;
                for (key, value) in params {
                    query.append_pair(key, value);
                }
            }
        }
        write!(f, "?{}", query.finish())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "abcdefghijklmnopqrstuvwxyz234567abcdefghijklmnopqrst";

    #[test]
    fn endpoint_round_trips() {
        let cases = [
            format!(
                "rune://{FP}/libraries/550e8400-e29b-41d4-a716-446655440000?v=1&addr=127.0.0.1:42&addr=%5B%3A%3A1%5D%3A43"
            ),
            format!("rune://{FP}/sessions/abc?v=1"),
            "subsonic+https://music.example.com/navidrome?v=1&user=alice".into(),
            "subsonic+http://music.example.com?v=1".into(),
            "opendal+s3://bucket/prefix?v=1&region=west".into(),
            "file:///music".into(),
            "content://media/music".into(),
        ];
        let library: EndpointUrl = cases[0].parse().unwrap();
        assert_eq!(
            library,
            EndpointUrl::Rune {
                fingerprint: Fingerprint::new(FP).unwrap(),
                addrs: vec!["127.0.0.1:42".parse().unwrap(), "[::1]:43".parse().unwrap()],
                target: Some(EndpointTarget::Library(
                    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
                )),
            }
        );
        let session: EndpointUrl = cases[1].parse().unwrap();
        assert!(
            matches!(session, EndpointUrl::Rune { target: Some(EndpointTarget::Session(id)), .. } if id == "abc")
        );
        let storage: EndpointUrl = cases[4].parse().unwrap();
        assert!(
            matches!(storage, EndpointUrl::OpenDal { service, location, params } if service == "s3" && location == "bucket/prefix" && params.get("region").is_some_and(|v| v == "west"))
        );
        for case in cases {
            let parsed: EndpointUrl = case.parse().unwrap();
            assert_eq!(parsed.to_string().parse::<EndpointUrl>().unwrap(), parsed);
        }
    }

    #[test]
    fn endpoint_validation() {
        let cases = [
            (
                "other://host?v=1".to_string(),
                EndpointUrlError::UnsupportedScheme("other".into()),
            ),
            (format!("rune://{FP}"), EndpointUrlError::MissingVersion),
            (
                format!("rune://{FP}?v=2"),
                EndpointUrlError::UnsupportedVersion {
                    found: 2,
                    supported: 1,
                },
            ),
            (
                format!("rune://{FP}?v=1&surprise=1"),
                EndpointUrlError::UnknownParameter("surprise".into()),
            ),
            (
                "rune://bad?v=1".into(),
                EndpointUrlError::InvalidFingerprint,
            ),
            (
                format!("rune://{FP}?v=1&addr=bad"),
                EndpointUrlError::InvalidAddress("bad".into()),
            ),
            (
                format!("rune://{FP}/other?v=1"),
                EndpointUrlError::InvalidPath("/other".into()),
            ),
            (
                format!("rune://{FP}?v=no"),
                EndpointUrlError::Malformed("invalid version".into()),
            ),
            (
                format!("rune://{FP}:42?v=1"),
                EndpointUrlError::Malformed("port in Rune authority".into()),
            ),
            (
                format!("rune://{FP}?v=1&PaSsWoRd=x"),
                EndpointUrlError::SecretNotAllowed("PaSsWoRd".into()),
            ),
            (
                "subsonic+http://host?v=1&token=x".into(),
                EndpointUrlError::SecretNotAllowed("token".into()),
            ),
            (
                "opendal+s3://bucket?v=1&secret_key=x".into(),
                EndpointUrlError::SecretNotAllowed("secret_key".into()),
            ),
            (
                "file:///music?key=x".into(),
                EndpointUrlError::SecretNotAllowed("key".into()),
            ),
            (
                "content://media/music?api_key=x".into(),
                EndpointUrlError::SecretNotAllowed("api_key".into()),
            ),
            (
                "file://user:pass@host/path".into(),
                EndpointUrlError::SecretNotAllowed("password".into()),
            ),
            (
                "subsonic+https://user:pass@host?v=1".into(),
                EndpointUrlError::SecretNotAllowed("password".into()),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(input.parse::<EndpointUrl>(), Err(expected), "{input}");
        }
        for input in [
            format!("RUNE://{FP}?v=1&opt-hint=yes"),
            format!("rune://{}?v=1&opt-hint=yes", FP.to_ascii_uppercase()),
            "SUBSONIC+HTTP://host?v=1&opt-hint=yes".into(),
            "OPENDAL+S3://bucket?v=1&opt-hint=yes".into(),
        ] {
            let parsed: EndpointUrl = input.parse().unwrap();
            assert!(!parsed.to_string().contains("opt-hint"));
            assert_eq!(parsed.to_string().parse::<EndpointUrl>().unwrap(), parsed);
        }
    }
}
