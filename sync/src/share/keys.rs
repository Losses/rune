//! Object key construction and strict parsing.
use super::verification::{FormatError, valid_hash};
use std::{fmt, str::FromStr};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DevicePrefix {
    pub stream: Uuid,
    pub generation: Uuid,
}

impl FromStr for DevicePrefix {
    type Err = FormatError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (stream, generation) = value
            .split_once('.')
            .ok_or_else(|| FormatError::Invalid("invalid device prefix".into()))?;
        let stream = Uuid::parse_str(stream)
            .map_err(|_| FormatError::Invalid("invalid device stream id".into()))?;
        let generation = Uuid::parse_str(generation)
            .map_err(|_| FormatError::Invalid("invalid device generation id".into()))?;
        if stream.is_nil() || generation.is_nil() {
            return Err(FormatError::Invalid("invalid device prefix".into()));
        }
        let parsed = Self { stream, generation };
        if parsed.to_string() != value {
            return Err(FormatError::Invalid("invalid device prefix".into()));
        }
        Ok(parsed)
    }
}

impl fmt::Display for DevicePrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.stream, self.generation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectKey {
    Library,
    Head {
        prefix: DevicePrefix,
        slot: u8,
    },
    Segment {
        prefix: DevicePrefix,
        first: u64,
        last: u64,
        hash16: String,
    },
    Base {
        prefix: DevicePrefix,
        base_id: String,
    },
    Chunk {
        prefix: DevicePrefix,
        digest: String,
    },
    Other(String),
}

pub fn library_key() -> String {
    ".rune/sync/library.json".into()
}
pub fn head_key(prefix: &str, slot: u8) -> Result<String, FormatError> {
    valid_prefix(prefix)?;
    if slot > 1 {
        return Err(FormatError::Invalid("head slot must be 0 or 1".into()));
    }
    Ok(format!("devices/{prefix}/head.{slot}.json"))
}
pub fn segment_key(
    prefix: &str,
    first: u64,
    last: u64,
    hash16: &str,
) -> Result<String, FormatError> {
    valid_prefix(prefix)?;
    if first == 0
        || last < first
        || hash16.len() != 16
        || !hash16
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(FormatError::Invalid(
            "invalid segment key components".into(),
        ));
    }
    Ok(format!(
        "devices/{prefix}/seg/{first:020}-{last:020}-{hash16}.gz"
    ))
}
pub fn base_key(prefix: &str, base_id: &str) -> Result<String, FormatError> {
    valid_prefix(prefix)?;
    if !valid_hash(base_id) {
        return Err(FormatError::Invalid("invalid base id".into()));
    }
    Ok(format!("devices/{prefix}/base/{base_id}.json"))
}
pub fn chunk_key(prefix: &str, digest: &str) -> Result<String, FormatError> {
    valid_prefix(prefix)?;
    if !valid_hash(digest) {
        return Err(FormatError::Invalid("invalid chunk digest".into()));
    }
    Ok(format!("devices/{prefix}/obj/{digest}.gz"))
}
pub(super) fn valid_prefix(prefix: &str) -> Result<(), FormatError> {
    let parsed = DevicePrefix::from_str(prefix)?;
    if parsed.to_string() != prefix {
        return Err(FormatError::Invalid("invalid device prefix".into()));
    }
    Ok(())
}
pub fn parse_key(key: &str) -> Result<ObjectKey, FormatError> {
    if key == library_key() {
        return Ok(ObjectKey::Library);
    }
    if let Some((prefix_text, tail)) = key
        .strip_prefix("devices/")
        .and_then(|value| value.split_once('/'))
    {
        let prefix = DevicePrefix::from_str(prefix_text)?;
        if let Some(slot) = tail
            .strip_prefix("head.")
            .and_then(|v| v.strip_suffix(".json"))
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|v| *v <= 1)
        {
            return Ok(ObjectKey::Head { prefix, slot });
        }
        if let Some(name) = tail
            .strip_prefix("seg/")
            .and_then(|v| v.strip_suffix(".gz"))
        {
            let parts: Vec<_> = name.split('-').collect();
            if parts.len() == 3
                && let (Ok(first), Ok(last)) = (parts[0].parse(), parts[1].parse())
            {
                let key = ObjectKey::Segment {
                    prefix: prefix.clone(),
                    first,
                    last,
                    hash16: parts[2].into(),
                };
                if segment_key(&prefix.to_string(), first, last, parts[2]).as_deref()
                    == Ok(key_string(&key).as_str())
                {
                    return Ok(key);
                }
            }
        }
        if let Some(id) = tail
            .strip_prefix("base/")
            .and_then(|v| v.strip_suffix(".json"))
            && valid_hash(id)
        {
            return Ok(ObjectKey::Base {
                prefix,
                base_id: id.into(),
            });
        }
        if let Some(digest) = tail
            .strip_prefix("obj/")
            .and_then(|v| v.strip_suffix(".gz"))
            && valid_hash(digest)
        {
            return Ok(ObjectKey::Chunk {
                prefix,
                digest: digest.into(),
            });
        }
    }
    Err(FormatError::Invalid("unrecognized share object key".into()))
}
fn key_string(key: &ObjectKey) -> String {
    match key {
        ObjectKey::Segment {
            prefix,
            first,
            last,
            hash16,
        } => format!("devices/{prefix}/seg/{first:020}-{last:020}-{hash16}.gz"),
        _ => String::new(),
    }
}
