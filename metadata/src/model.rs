//! Resolved metadata for releases, discs, and audio.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::release_type::{ReleaseSecondaryType, ReleaseType};

pub type RawTagMap = BTreeMap<String, Vec<String>>;
pub type Credits = Vec<Credit>;
pub type ExternalIdentifiers = BTreeMap<String, Vec<String>>;
pub type AudioFingerprints = BTreeMap<String, String>;
pub type MetadataProvenance = BTreeMap<String, Vec<FieldSource>>;

/// A credited name and any confirmed association with an artist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credit {
    pub credited_as: String,
    pub role: String,
    pub artist_id: Option<Uuid>,
    pub instrument: Option<String>,
    pub source: FieldSource,
    /// Named writer for user and external credits. The scanner owns the unnamed tag source.
    pub source_provider: Option<String>,
}

/// Shared descriptive fields and provenance.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetadataBase {
    pub id: Uuid,
    pub credits: Option<Credits>,
    pub genres: Option<Vec<String>>,
    pub release_date: Option<String>,
    pub original_release_date: Option<String>,
    pub version: Option<String>,
    pub sort_title: Option<String>,

    // Additional metadata for this entity.
    pub grouping: Option<String>,
    pub moods: Option<Vec<String>>,
    pub description: Option<String>,
    pub comment: Option<String>,
    pub copyright: Option<String>,
    pub license: Option<String>,
    pub languages: Option<Vec<String>>,
    pub script: Option<String>,

    // Additional metadata for this entity.
    pub work: Option<String>,
    pub part: Option<String>,
    pub section: Option<String>,
    pub opus: Option<String>,
    pub work_id: Option<String>,
    pub user_tags: Option<Vec<String>>,
    pub identifiers: Option<ExternalIdentifiers>,
    pub links: Option<Vec<MetadataLink>>,

    // Additional metadata for this entity.
    pub recording_date: Option<String>,
    pub recording_start_date: Option<String>,
    pub recording_end_date: Option<String>,
    pub recording_location: Option<String>,
    pub is_live: Option<bool>,
    pub is_bootleg: Option<bool>,
    pub artwork: Option<Vec<Artwork>>,
    pub duration_ms: Option<u64>,

    // Additional metadata for this entity.
    pub import_date: Option<String>,
    pub provenance: Option<MetadataProvenance>,
}

/// One logical release across all of its discs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumMetadata {
    #[serde(flatten)]
    pub base: MetadataBase,

    // Fields owned by this entity.
    pub title: String,
    /// The available evidence matches more than one edition in this directory.
    #[serde(default)]
    pub needs_review: bool,
    pub source: AlbumSource,
    pub extra_resources: Option<Vec<AlbumExtraResource>>,
    pub barcode: Option<String>,
    pub catalog_number: Option<String>,
    pub labels: Option<Vec<String>>,
    pub release_country: Option<String>,
    pub release_type: Option<ReleaseType>,
    /// Each value records an explicit or derived assertion. An omitted type is unknown.
    #[serde(default)]
    pub release_secondary_types: BTreeMap<ReleaseSecondaryType, bool>,
    pub release_status: Option<String>,

    // Additional metadata for this entity.
    pub disc_count: Option<u32>,
    pub total_tracks: Option<u32>,
    pub declared_track_count: Option<u32>,
    pub replaygain_album_gain_db: Option<f64>,

    // Additional metadata for this entity.
    pub replaygain_album_peak: Option<f64>,
    pub replaygain_reference_lufs: Option<f64>,
}

/// A persisted disc belonging to a release.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscMetadata {
    #[serde(flatten)]
    pub base: MetadataBase,

    // Fields owned by this entity.
    pub title: Option<String>,
    pub album_id: Uuid,
    pub disc_number: u32,
    pub media_format: Option<String>,
    pub directory_path: Option<String>,
    pub track_count: Option<u32>,
    pub declared_track_count: Option<u32>,
}

/// One indexed audio file or logical track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioMetadata {
    #[serde(flatten)]
    pub base: MetadataBase,

    // Fields owned by this entity.
    pub title: String,
    pub subtitle: Option<String>,
    pub bpm: Option<f64>,
    pub movement_number: Option<u32>,
    pub movement_total: Option<u32>,
    pub file_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub file_modified_at: Option<String>,
    pub disc_id: Option<Uuid>,
    pub recording_id: Option<Uuid>,

    // Additional metadata for this entity.
    pub album_id: Option<Uuid>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub disc_total: Option<u32>,
    pub format: Option<String>,
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,

    // Additional metadata for this entity.
    pub bits_per_sample: Option<u32>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub bitrate: Option<u64>,
    pub frame_count: Option<u64>,
    pub fingerprints: Option<AudioFingerprints>,
    pub encoding: Option<AudioEncodingInfo>,
    pub replaygain_track_gain_db: Option<f64>,

    // Additional metadata for this entity.
    pub replaygain_track_peak: Option<f64>,
    pub replaygain_reference_lufs: Option<f64>,
    pub lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
    pub original_album_title: Option<String>,
    pub isrc: Option<String>,
    pub content_type: Option<AudioContentType>,
}

/// Strong identifiers for a recording independently of a file or edition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingIdentifiers {
    pub musicbrainz_id: Option<Uuid>,
    pub isrc: Option<String>,
}

/// A provider's copy of an identified recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingSource {
    pub provider: String,
    pub resource_id: String,
}

/// Shared recording evidence and the audio files or providers that can supply it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingMetadata {
    pub id: Uuid,
    pub identifiers: RecordingIdentifiers,
    pub credits: Credits,
    pub audio_ids: Vec<Uuid>,
    pub remote_sources: Vec<RecordingSource>,
    #[serde(default)]
    pub versions: Vec<AudioVariant>,
}

/// Codec and quality information for either local files or provider copies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioQuality {
    pub codec: Option<String>,
    pub is_lossless: Option<bool>,
    pub sample_rate: Option<u32>,
    pub bits_per_sample: Option<u32>,
    pub bitrate: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioVariantSource {
    File {
        audio_id: Uuid,
    },
    Provider {
        provider: String,
        resource_id: String,
    },
}

/// A playable asset, optionally restricted to a time range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioVariant {
    pub source: AudioVariantSource,
    pub available: bool,
    pub quality: AudioQuality,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

/// A logical track on an indexed edition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseTrackMetadata {
    pub id: Uuid,
    pub disc_id: Uuid,
    #[serde(default)]
    pub recording_id: Uuid,
    pub track_number: Option<u32>,
    pub title: String,
    pub variants: Vec<AudioVariant>,
    #[serde(default)]
    pub provenance: Option<MetadataProvenance>,
}

/// A segment supplied by a disc image's track layout, such as a CUE index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSegment {
    pub track_number: u32,
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}
/// Field Source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSource {
    pub kind: MetadataSourceKind,
    pub key: Option<String>,
    pub path: Option<String>,
    pub pattern: Option<String>,
    pub entity_id: Option<Uuid>,
    pub confidence: Option<f64>,
}

/// Artwork Info.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtworkInfo {
    #[serde(rename = "type")]
    pub kind: ArtworkType,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Metadata Link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataLink {
    pub relation: MetadataLinkRelation,
    pub url: String,
    pub title: Option<String>,
}

/// Audio Encoding Info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioEncodingInfo {
    pub encoder: Option<String>,
    pub encoder_settings: Option<String>,
    pub encoding_date: Option<String>,
    pub tagging_date: Option<String>,
}

/// Album Extra Resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumExtraResource {
    #[serde(rename = "type")]
    pub kind: AlbumExtraResourceType,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtworkType {
    Front,
    Back,
    Disc,
    Booklet,
    Artist,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSourceKind {
    AudioTag,
    AudioStream,
    Filename,
    Directory,
    Sidecar,
    EmbeddedArtwork,
    Aggregate,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataLinkRelation {
    Official,
    Artist,
    Label,
    Copyright,
    Source,
    Purchase,
    Payment,
    Podcast,
    Radio,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlbumExtraResourceType {
    Scans,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioContentType {
    Music,
    Podcast,
    Audiobook,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlbumSource {
    Local {
        directory_path: String,
    },
    Remote {
        provider: String,
        resource_id: String,
        url: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artwork {
    #[serde(flatten)]
    pub info: ArtworkInfo,
    #[serde(flatten)]
    pub source: ArtworkSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ArtworkSource {
    File {
        path: String,
    },
    Embedded {
        audio_path: String,
        picture_index: u32,
    },
    Url {
        url: String,
    },
    Library {
        file_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MusicMetadata {
    Album(Box<AlbumMetadata>),
    Disc(Box<DiscMetadata>),
    Audio(Box<AudioMetadata>),
}

impl DiscMetadata {
    /// Prefer artwork on this disc, with display-only fallback to its album.
    pub fn preferred_cover<'a>(&'a self, album: Option<&'a AlbumMetadata>) -> Option<&'a Artwork> {
        fn preferred(art: &[Artwork]) -> Option<&Artwork> {
            art.iter()
                .find(|entry| entry.info.kind == ArtworkType::Front)
                .or_else(|| art.first())
        }

        self.base
            .artwork
            .as_deref()
            .and_then(preferred)
            .or_else(|| {
                let album = album.filter(|album| album.base.id == self.album_id)?;
                preferred(album.base.artwork.as_deref()?)
            })
    }
}
