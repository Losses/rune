//! Canonical codecs and integrity checks for share objects.
use super::formats::*;
use super::keys::valid_prefix;
use crate::protocol::{SyncModel, Uuid, event_digest, next_anchor};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    error::Error as StdError,
    fmt::{Display, Error as FmtError, Formatter},
    io::{Read, Write},
    result::Result as StdResult,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    Parse(String),
    UnknownVersion { object: &'static str, version: u64 },
    Checksum,
    Digest,
    Compression(String),
    Invalid(String),
}

impl Display for FormatError {
    fn fmt(&self, f: &mut Formatter<'_>) -> StdResult<(), FmtError> {
        match self {
            Self::Parse(message) => write!(f, "share format parse error: {message}"),
            Self::UnknownVersion { object, version } => {
                write!(f, "unknown {object} format version {version}")
            }
            Self::Checksum => f.write_str("share object checksum mismatch"),
            Self::Digest => f.write_str("share object digest mismatch"),
            Self::Compression(message) => write!(f, "share object compression error: {message}"),
            Self::Invalid(message) => write!(f, "invalid share object: {message}"),
        }
    }
}

impl StdError for FormatError {}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, FormatError> {
    let value = serde_json::to_value(value).map_err(parse_error)?;
    let mut bytes = Vec::new();
    write_canonical(&value, &mut bytes)?;
    Ok(bytes)
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) -> Result<(), FormatError> {
    match value {
        Value::Object(map) => {
            out.push(b'{');
            let sorted: BTreeMap<_, _> = map.iter().collect();
            for (index, (key, item)) in sorted.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key).map_err(parse_error)?;
                out.push(b':');
                write_canonical(item, out)?;
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        _ => serde_json::to_writer(out, value).map_err(parse_error)?,
    }
    Ok(())
}

fn parse_error(error: impl Display) -> FormatError {
    FormatError::Parse(error.to_string())
}

fn verify_version<T: Serialize + DeserializeOwned>(
    bytes: &[u8],
    object: &'static str,
) -> Result<T, FormatError> {
    let value: Value = serde_json::from_slice(bytes).map_err(parse_error)?;
    let version = value
        .get("format")
        .and_then(Value::as_u64)
        .ok_or_else(|| FormatError::Invalid("missing format version".into()))?;
    if version != u64::from(FORMAT_VERSION) {
        return Err(FormatError::UnknownVersion { object, version });
    }
    let decoded = serde_json::from_value(value).map_err(parse_error)?;
    if canonical(&decoded)? != bytes {
        return Err(FormatError::Parse("noncanonical JSON".into()));
    }
    Ok(decoded)
}

pub fn encode_library(value: &LibraryFile) -> Result<Vec<u8>, FormatError> {
    check_format(value.format, "library")?;
    if value.library_id.is_nil() || value.model.is_empty() || value.schema.is_empty() {
        return Err(FormatError::Invalid("invalid library identity".into()));
    }
    canonical(value)
}
pub fn decode_library(bytes: &[u8]) -> Result<LibraryFile, FormatError> {
    let library: LibraryFile = verify_version(bytes, "library")?;
    encode_library(&library)?;
    Ok(library)
}

fn check_format(format: u32, object: &'static str) -> Result<(), FormatError> {
    if format == FORMAT_VERSION {
        Ok(())
    } else {
        Err(FormatError::UnknownVersion {
            object,
            version: u64::from(format),
        })
    }
}

pub fn encode_head(value: &Head) -> Result<Vec<u8>, FormatError> {
    check_format(value.format, "head")?;
    validate_head(value)?;
    let body = canonical(value)?;
    let checksum = blake3::hash(&body);
    let mut output = body;
    output.extend_from_slice(b"\n");
    output.extend_from_slice(checksum.to_hex().as_bytes());
    Ok(output)
}
pub fn decode_head(bytes: &[u8]) -> Result<Head, FormatError> {
    let split = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .ok_or(FormatError::Checksum)?;
    let (body, checksum_with_separator) = bytes.split_at(split);
    let checksum = &checksum_with_separator[1..];
    if checksum.len() != 64 || blake3::hash(body).to_hex().as_bytes() != checksum {
        return Err(FormatError::Checksum);
    }
    let head: Head = verify_version(body, "head")?;
    validate_head(&head)?;
    Ok(head)
}

fn validate_head(head: &Head) -> Result<(), FormatError> {
    if [
        head.library_id,
        head.stream,
        head.generation,
        head.local_replica,
        head.writer_session,
    ]
    .iter()
    .any(Uuid::is_nil)
        || head.floor.seq > head.head.seq
        || head.floor_base.covers_seq > head.head.seq
        || head.segments.len() > 35
        || head.segments.iter().any(|segment| {
            segment.first == 0
                || segment.last < segment.first
                || segment.last > head.head.seq
                || segment.level > 2
        })
        || head.floor_base.base_id.len() != 64
        || !valid_hash(&head.floor_base.base_id)
    {
        return Err(FormatError::Invalid("inconsistent head fields".into()));
    }
    valid_prefix(&head.floor_base.owner_prefix)
}

pub fn encode_segment<M: SyncModel>(segment: &Segment<M>) -> Result<Vec<u8>, FormatError> {
    check_format(segment.format, "segment")?;
    let mut anchor = segment.prev_anchor;
    let mut expected_seq: Option<u64> = None;
    for link in &segment.links {
        if let Some(previous) = expected_seq
            && previous.checked_add(1) != Some(link.seq)
        {
            return Err(FormatError::Invalid(
                "non-contiguous segment sequence".into(),
            ));
        }
        let digest = event_digest(&link.event).map_err(parse_error)?;
        anchor =
            next_anchor::<M>(anchor, link.seq, &link.collection, digest).map_err(parse_error)?;
        expected_seq = Some(link.seq);
    }
    if anchor != segment.last_anchor {
        return Err(FormatError::Digest);
    }
    let body = canonical(segment)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&body).map_err(parse_error)?;
    encoder.finish().map_err(parse_error)
}
pub fn decode_segment<M: SyncModel>(bytes: &[u8]) -> Result<Segment<M>, FormatError> {
    let mut decoder = GzDecoder::new(bytes);
    let mut body = Vec::new();
    decoder
        .read_to_end(&mut body)
        .map_err(|error| FormatError::Compression(error.to_string()))?;
    let segment: Segment<M> = verify_version(&body, "segment")?;
    // Re-encoding validates every link and rejects noncanonical gzip payloads.
    encode_segment(&segment)?;
    Ok(segment)
}

pub fn encode_manifest<M: SyncModel>(
    value: &BaseManifest<M>,
) -> Result<(String, Vec<u8>), FormatError> {
    check_format(value.format, "manifest")?;
    validate_manifest(value)?;
    let bytes = canonical(value)?;
    Ok((hex(blake3::hash(&bytes).as_bytes()), bytes))
}
pub fn decode_manifest<M: SyncModel>(
    bytes: &[u8],
    expected_id: &str,
) -> Result<BaseManifest<M>, FormatError> {
    let manifest: BaseManifest<M> = verify_version(bytes, "manifest")?;
    validate_manifest(&manifest)?;
    if hex(blake3::hash(bytes).as_bytes()) != expected_id {
        return Err(FormatError::Digest);
    }
    Ok(manifest)
}

fn validate_manifest<M: SyncModel>(manifest: &BaseManifest<M>) -> Result<(), FormatError> {
    for collection in manifest.collections.values() {
        if collection.bucket_count as usize != collection.buckets.len()
            || collection.root.candidate_count > 0 && collection.buckets.is_empty()
            || collection
                .buckets
                .iter()
                .any(|bucket| !valid_hash(&bucket.chunk))
            || collection
                .context_fragments
                .iter()
                .any(|name| !valid_hash(name))
        {
            return Err(FormatError::Invalid(
                "inconsistent manifest summaries".into(),
            ));
        }
    }
    Ok(())
}

pub fn encode_chunk(candidates: Vec<u8>) -> Result<(String, Vec<u8>), FormatError> {
    let supplied = candidates;
    let candidates: Value = serde_json::from_slice(&supplied).map_err(parse_error)?;
    let mut canonical_candidates = Vec::new();
    write_canonical(&candidates, &mut canonical_candidates)?;
    if !candidates.is_array() || canonical_candidates != supplied {
        return Err(FormatError::Invalid(
            "chunk candidates must be an array".into(),
        ));
    }
    let bytes = canonical(&ChunkWire {
        format: FORMAT_VERSION,
        candidates,
    })?;
    let name = hex(blake3::hash(&bytes).as_bytes());
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&bytes).map_err(parse_error)?;
    Ok((name, encoder.finish().map_err(parse_error)?))
}
pub fn decode_chunk(name: &str, gzip: &[u8]) -> Result<Chunk, FormatError> {
    if !valid_hash(name) {
        return Err(FormatError::Invalid("invalid chunk name".into()));
    }
    let mut decoder = GzDecoder::new(gzip);
    let mut canonical_bytes = Vec::new();
    decoder
        .read_to_end(&mut canonical_bytes)
        .map_err(|error| FormatError::Compression(error.to_string()))?;
    if hex(blake3::hash(&canonical_bytes).as_bytes()) != name {
        return Err(FormatError::Digest);
    }
    let wire: ChunkWire = verify_version(&canonical_bytes, "chunk")?;
    let mut recoded = Vec::new();
    write_canonical(&wire.candidates, &mut recoded)?;
    if !wire.candidates.is_array() || recoded.is_empty() {
        return Err(FormatError::Parse("noncanonical chunk JSON".into()));
    }
    Ok(Chunk {
        format: wire.format,
        name: name.to_owned(),
        canonical_bytes,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkWire {
    format: u32,
    candidates: Value,
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
