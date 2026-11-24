use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: Option<PreRelease>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreRelease {
    Alpha(u32),
    Beta(u32),
    Rc(u32),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("invalid protocol version: {0}")]
pub struct ParseProtocolVersionError(pub String);

/// Request and response header carrying a node's protocol version.
pub const PROTOCOL_VERSION_HEADER: &str = "x-rune-protocol-version";
/// Response header naming the answering device, for "needs update" messages.
pub const DEVICE_ALIAS_HEADER: &str = "x-rune-device-alias";

pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion {
    major: 2,
    minor: 0,
    patch: 0,
    pre: None,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    Compatible { peer_minor: u32 },
    PeerOlder,
    PeerNewer,
    PreReleaseMismatch,
}

pub fn compatibility(local: ProtocolVersion, peer: ProtocolVersion) -> Compatibility {
    if local.major > peer.major {
        return Compatibility::PeerOlder;
    }
    if local.major < peer.major {
        return Compatibility::PeerNewer;
    }
    if (local.pre.is_some() || peer.pre.is_some()) && local != peer {
        return Compatibility::PreReleaseMismatch;
    }
    Compatibility::Compatible {
        peer_minor: peer.minor,
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = self.pre {
            match pre {
                PreRelease::Alpha(n) => write!(f, "-alpha.{n}"),
                PreRelease::Beta(n) => write!(f, "-beta.{n}"),
                PreRelease::Rc(n) => write!(f, "-rc.{n}"),
            }?;
        }
        Ok(())
    }
}

impl FromStr for ProtocolVersion {
    type Err = ParseProtocolVersionError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (core, pre) = match value.split_once('-') {
            Some((core, suffix)) => (core, Some(suffix)),
            None => (value, None),
        };
        let mut parts = core.split('.');
        let number = |part: Option<&str>| -> Result<u32, Self::Err> {
            let part = part.ok_or_else(|| ParseProtocolVersionError(value.into()))?;
            if part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(ParseProtocolVersionError(value.into()));
            }
            part.parse()
                .map_err(|_| ParseProtocolVersionError(value.into()))
        };
        let major = number(parts.next())?;
        let minor = number(parts.next())?;
        let patch = number(parts.next())?;
        if parts.next().is_some() {
            return Err(ParseProtocolVersionError(value.into()));
        }
        let pre = match pre {
            None => None,
            Some(suffix) => {
                let (kind, n) = suffix
                    .split_once('.')
                    .ok_or_else(|| ParseProtocolVersionError(value.into()))?;
                let n = number(Some(n))?;
                Some(match kind {
                    "alpha" => PreRelease::Alpha(n),
                    "beta" => PreRelease::Beta(n),
                    "rc" => PreRelease::Rc(n),
                    _ => return Err(ParseProtocolVersionError(value.into())),
                })
            }
        };
        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl ProtocolVersion {
    pub fn parse(value: &str) -> Result<Self, ParseProtocolVersionError> {
        value.parse()
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{Compatibility, PreRelease, ProtocolVersion, compatibility};

    #[test]
    fn parses_and_displays_strict_semver_forms() {
        for (wire, expected) in [
            (
                "1.0.0",
                ProtocolVersion {
                    major: 1,
                    minor: 0,
                    patch: 0,
                    pre: None,
                },
            ),
            (
                "2.4.7-alpha.1",
                ProtocolVersion {
                    major: 2,
                    minor: 4,
                    patch: 7,
                    pre: Some(PreRelease::Alpha(1)),
                },
            ),
            (
                "2.4.7-beta.2",
                ProtocolVersion {
                    major: 2,
                    minor: 4,
                    patch: 7,
                    pre: Some(PreRelease::Beta(2)),
                },
            ),
            (
                "2.4.7-rc.3",
                ProtocolVersion {
                    major: 2,
                    minor: 4,
                    patch: 7,
                    pre: Some(PreRelease::Rc(3)),
                },
            ),
        ] {
            let parsed = ProtocolVersion::parse(wire).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), wire);
        }
        for invalid in [
            "",
            "1",
            "1.0",
            "01.0.0",
            "1.0.0-rc",
            "1.0.0-alpha.01",
            "1.0.0-dev.1",
            "1.0.0+build",
            "1.0.0.1",
        ] {
            assert!(ProtocolVersion::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn checks_major_minor_and_prerelease_rules() {
        let v = |major, minor, patch, pre| ProtocolVersion {
            major,
            minor,
            patch,
            pre,
        };
        assert_eq!(
            compatibility(v(2, 0, 0, None), v(1, 9, 9, None)),
            Compatibility::PeerOlder
        );
        assert_eq!(
            compatibility(v(1, 0, 0, None), v(2, 0, 0, None)),
            Compatibility::PeerNewer
        );
        assert_eq!(
            compatibility(v(1, 0, 0, None), v(1, 4, 2, None)),
            Compatibility::Compatible { peer_minor: 4 }
        );
        assert_eq!(
            compatibility(
                v(1, 0, 0, Some(PreRelease::Alpha(1))),
                v(1, 0, 0, Some(PreRelease::Alpha(1)))
            ),
            Compatibility::Compatible { peer_minor: 0 }
        );
        assert_eq!(
            compatibility(v(1, 0, 0, None), v(1, 0, 0, Some(PreRelease::Alpha(1)))),
            Compatibility::PreReleaseMismatch
        );
        assert_eq!(
            compatibility(
                v(1, 0, 0, Some(PreRelease::Alpha(1))),
                v(1, 0, 0, Some(PreRelease::Beta(1)))
            ),
            Compatibility::PreReleaseMismatch
        );
    }
}
