//! Rune source-row declarations and pure portable value conversions.

use crate::actions::file::validate_digest;
use crate::entities::*;
use migration::catalog_identity::{self, ParentKind};
use sea_orm::{ColumnTrait, Condition, DbErr, Value, sea_query::Expr};
use serde::{Deserialize, Serialize};
use sync_orm::row_policy::{BinaryValue, FieldCodec, RowView};
use uuid::Uuid;

/// Maximum fingerprint size included in synchronization.
pub const MAX_PORTABLE_FINGERPRINT_BYTES: usize = 32768;

/// Metadata describing a blob; the sync engine does not transfer its bytes.
/// The digest is an opaque, nonempty string of at most 256 UTF-8 bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BlobRef {
    pub digest: String,
    pub size: u64,
}
/// Canonical decimal seconds, preserving the source precision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DurationValue(rust_decimal::Decimal);
impl TryFrom<String> for DurationValue {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let decimal: rust_decimal::Decimal = value
            .parse()
            .map_err(|error: rust_decimal::Error| error.to_string())?;
        if decimal.is_sign_negative() || decimal.normalize().to_string() != value {
            return Err("duration must be nonnegative canonical decimal seconds".into());
        }
        Ok(Self(decimal))
    }
}
impl From<DurationValue> for String {
    fn from(value: DurationValue) -> Self {
        value.0.normalize().to_string()
    }
}
impl FieldCodec for DurationValue {
    fn from_column(value: Value) -> Result<Self, DbErr> {
        let decimal = <rust_decimal::Decimal as sea_orm::sea_query::ValueType>::try_from(value)
            .map_err(|error| DbErr::Custom(error.to_string()))?;
        if decimal.is_sign_negative() {
            return Err(DbErr::Custom("negative duration".into()));
        }
        Ok(Self(decimal.normalize()))
    }
    fn to_column(&self) -> Value {
        self.0.into()
    }
    fn sample() -> Value {
        rust_decimal::Decimal::ZERO.into()
    }
}

/// Finite f32 values represented exactly; zero has a single representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScoreValue(pub u32);
impl FieldCodec for ScoreValue {
    fn from_column(value: Value) -> Result<Self, DbErr> {
        let score = <f32 as sea_orm::sea_query::ValueType>::try_from(value)
            .map_err(|error| DbErr::Custom(error.to_string()))?;
        let value = Self(if score == 0.0 { 0 } else { score.to_bits() });
        value.validate()?;
        Ok(value)
    }
    fn to_column(&self) -> Value {
        f32::from_bits(self.0).into()
    }
    fn sample() -> Value {
        0.0_f32.into()
    }
    fn validate(&self) -> Result<(), DbErr> {
        if !f32::from_bits(self.0).is_finite() || self.0 == (-0.0_f32).to_bits() {
            Err(DbErr::Custom(
                "similarity must be finite with canonical zero".into(),
            ))
        } else {
            Ok(())
        }
    }
}
fn validate_blob(hash: Option<&str>, size: Option<i64>) -> Result<(), DbErr> {
    match (hash, size) {
        (None, None) => Ok(()),
        (Some(hash), Some(size)) if size >= 0 => {
            validate_digest(hash).map_err(|error| DbErr::Custom(error.to_string()))
        }
        _ => Err(DbErr::Custom(
            "content digest and nonnegative length must be present together".into(),
        )),
    }
}
fn validate_provenance(
    algorithm: Option<&str>,
    configuration: Option<&str>,
    format: Option<i32>,
) -> Result<(), DbErr> {
    if [algorithm, configuration]
        .into_iter()
        .flatten()
        .any(|value| value.is_empty() || value.len() > 256)
        || format.is_some_and(|value| value < 0)
    {
        return Err(DbErr::Custom("invalid analysis provenance".into()));
    }
    Ok(())
}

sync_orm::owned_list_mapping! { pub struct MetadataTag, MetadataMap for media_metadata::Entity {
    max_items: 128,
    owners: [media_metadata::Column::FileId], constants: {},
    key: id: Uuid => media_metadata::Column::HlcUuid,
    fields: {
        position: i32 => media_metadata::Column::Position,
        key: String => media_metadata::Column::MetaKey,
        value: String => media_metadata::Column::MetaValue,
    },
    defaults: {},
    order: position => media_metadata::Column::Position,
    limits: { position => (0, 127), key => (0, 256), value => (0, 16384) },
} }
sync_orm::owned_list_mapping! { pub struct MixClause, MixClauseMap for mix_queries::Entity {
    max_items: 128,
    owners: [mix_queries::Column::MixUuid], constants: {},
    key: id: Uuid => mix_queries::Column::HlcUuid,
    fields: {
        position: i32 => mix_queries::Column::Position,
        operator: String => mix_queries::Column::Operator,
        parameter: String => mix_queries::Column::Parameter,
        target_uuid: Option<Uuid> => mix_queries::Column::TargetUuid,
        group: i32 => mix_queries::Column::Group,
    },
    defaults: { mix_queries::Column::LocalDirectory => None::<String> },
    order: position => mix_queries::Column::Position,
    reference_targets: ["tracks", "albums", "artists", "genres", "playlists"],
    reference_values: |value: &MixClause| {
        mix_target_collection(&value.operator).zip(value.target_uuid).into_iter().collect::<Vec<_>>()
    },
    limits: { position => (0, 127), operator => (1, 128), parameter => (0, 256) },
    validate: validate_mix_clause,
} }
fn validate_mix_clause(value: &MixClause) -> Result<(), DbErr> {
    let valid = match value.operator.as_str() {
        "lib::track" | "lib::album" | "lib::artist" | "lib::genre" | "lib::playlist" => {
            value.target_uuid.is_some() && value.parameter.is_empty()
        }
        "lib::directory.deep" | "lib::directory.shallow" => {
            value.target_uuid.is_none() && value.parameter.is_empty()
        }
        "lib::all"
        | "lib::queue"
        | "sort::track_number"
        | "sort::last_modified"
        | "sort::duration"
        | "sort::playedthrough"
        | "sort::skipped"
        | "filter::liked"
        | "filter::analyzed"
        | "filter::with_cover_art" => {
            value.target_uuid.is_none() && matches!(value.parameter.as_str(), "true" | "false")
        }
        "lib::random" | "pipe::recommend" => {
            value.target_uuid.is_none()
                && value
                    .parameter
                    .parse::<i32>()
                    .is_ok_and(|number| number.to_string() == value.parameter)
        }
        "pipe::limit" => {
            value.target_uuid.is_none()
                && value
                    .parameter
                    .parse::<u64>()
                    .is_ok_and(|number| number.to_string() == value.parameter)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(DbErr::Custom("invalid mix clause".into()))
    }
}
pub fn mix_target_collection(operator: &str) -> Option<&'static str> {
    match operator {
        "lib::track" => Some("tracks"),
        "lib::album" => Some("albums"),
        "lib::artist" => Some("artists"),
        "lib::genre" => Some("genres"),
        "lib::playlist" => Some("playlists"),
        _ => None,
    }
}
sync_orm::derived_effect_mapping! { pub struct SearchDocument, SearchMap for search_index::Entity {
    max_items: 2,
    owners: [search_index::Column::Key, search_index::Column::EntryType], constants: {},
    fields: { text: Option<String> => search_index::Column::Doc },
    defaults: { search_index::Column::Id => "" },
} }
fn search_documents(name: Option<&str>) -> Result<Vec<SearchDocument>, DbErr> {
    Ok(name
        .into_iter()
        .flat_map(|name| [name.to_owned(), deunicode::deunicode(name)])
        .map(|text| SearchDocument { text: Some(text) })
        .collect())
}
sync_orm::row_policies! {
    pub RuneModel,
    Collection,
    Payload {
        model: "rune.library",
        schema: "2",
        collections: {
            Albums => {
                tag: "albums",
                entity: albums::Entity,
                value: Album,
                mapping: AlbumMap,
                policy: {
                    uuid: albums::Column::HlcUuid,
                    fields: {
                        name: String => albums::Column::Name,
                        group: String => albums::Column::Group
                    },
                    defaults: {},
                    effects: {
                        search => {
                            entity: search_index::Entity,
                            mapping: SearchMap,
                            owner: |row: &RowView<albums::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![Some(row.read::<i32>(albums::Column::Id)?.to_string()).into(), Some("album".to_owned()).into()])
                            },
                            derive: |_: &RowView<albums::Entity>, value: Option<&Album>| search_documents(value.map(|value| value.name.as_str())),
                        }
                    },
                    limits: {
                        name => (0, 16384),
                        group => (0, 16384)
                    },
                    identity: |value: &Album| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::parent(ParentKind::Album, &value.name))
                    },
                }
            },
            Artists => {
                tag: "artists",
                entity: artists::Entity,
                value: Artist,
                mapping: ArtistMap,
                policy: {
                    uuid: artists::Column::HlcUuid,
                    fields: {
                        name: String => artists::Column::Name,
                        group: String => artists::Column::Group
                    },
                    defaults: {},
                    effects: {
                        search => {
                            entity: search_index::Entity,
                            mapping: SearchMap,
                            owner: |row: &RowView<artists::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![Some(row.read::<i32>(artists::Column::Id)?.to_string()).into(), Some("artist".to_owned()).into()])
                            },
                            derive: |_: &RowView<artists::Entity>, value: Option<&Artist>| search_documents(value.map(|value| value.name.as_str())),
                        }
                    },
                    limits: {
                        name => (0, 16384),
                        group => (0, 16384)
                    },
                    identity: |value: &Artist| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::parent(ParentKind::Artist, &value.name))
                    },
                }
            },
            Genres => {
                tag: "genres",
                entity: genres::Entity,
                value: Genre,
                mapping: GenreMap,
                policy: {
                    uuid: genres::Column::HlcUuid,
                    fields: {
                        name: String => genres::Column::Name,
                        group: String => genres::Column::Group
                    },
                    defaults: {},
                    effects: {
                        search => {
                            entity: search_index::Entity,
                            mapping: SearchMap,
                            owner: |row: &RowView<genres::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![Some(row.read::<i32>(genres::Column::Id)?.to_string()).into(), Some("genre".to_owned()).into()])
                            },
                            derive: |_: &RowView<genres::Entity>, value: Option<&Genre>| search_documents(value.map(|value| value.name.as_str())),
                        }
                    },
                    limits: {
                        name => (0, 16384),
                        group => (0, 16384)
                    },
                    identity: |value: &Genre| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::parent(ParentKind::Genre, &value.name))
                    },
                }
            },
            Playlists => {
                tag: "playlists",
                entity: playlists::Entity,
                value: Playlist,
                mapping: PlaylistMap,
                policy: {
                    uuid: playlists::Column::HlcUuid,
                    fields: {
                        name: String => playlists::Column::Name,
                        group: String => playlists::Column::Group
                    },
                    defaults: {},
                    effects: {
                        search => {
                            entity: search_index::Entity,
                            mapping: SearchMap,
                            owner: |row: &RowView<playlists::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![Some(row.read::<i32>(playlists::Column::Id)?.to_string()).into(), Some("playlist".to_owned()).into()])
                            },
                            derive: |_: &RowView<playlists::Entity>, value: Option<&Playlist>| search_documents(value.map(|value| value.name.as_str())),
                        }
                    },
                    limits: {
                        name => (0, 16384),
                        group => (0, 16384)
                    },
                }
            },
            Tracks => {
                tag: "tracks",
                entity: media_files::Entity,
                value: Track,
                mapping: TrackMap,
                policy: {
                    uuid: media_files::Column::HlcUuid,
                    fields: {
                        extension: String => media_files::Column::Extension,
                        content_hash: Option<String> => media_files::Column::ContentHash,
                        content_size: Option<i64> => media_files::Column::ContentSize,
                        duration: DurationValue => media_files::Column::Duration,
                        sample_rate: i32 => media_files::Column::SampleRate,
                        cover_art_id: Option<Uuid> => media_files::Column::CoverArtUuid
                    },
                    defaults: {
                        media_files::Column::FileName => "",
                        media_files::Column::Directory => "",
                        media_files::Column::FileHash => "",
                        media_files::Column::LastModified => "",
                        media_files::Column::LocalAvailable => false,
                        media_files::Column::CoverArtCheckedAt => None::<String>,
                    },
                    local_reads: [media_files::Column::LocalAvailable, media_files::Column::FileName, media_files::Column::ChangeToken],
                    lists: {
                        metadata: MetadataTag => {
                            entity: media_metadata::Entity,
                            mapping: MetadataMap,
                            owner: |row: &RowView<media_files::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![row.read::<i32>(media_files::Column::Id)?.into()])
                            }
                        }
                    },
                    effects: {
                        search => {
                            entity: search_index::Entity,
                            mapping: SearchMap,
                            owner: |row: &RowView<media_files::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![Some(row.read::<i32>(media_files::Column::Id)?.to_string()).into(), Some("track".to_owned()).into()])
                            },
                            derive: |row: &RowView<media_files::Entity>, value: Option<&Track>| {
                                let Some(value) = value else { return search_documents(None); };
                                let filename = row.read::<String>(media_files::Column::FileName)?;
                                search_documents(value.metadata.iter().find(|tag| tag.key == "track_title").map(|tag| tag.value.as_str()).or_else(|| (!filename.is_empty()).then_some(filename.as_str())))
                            },
                        }
                    },
                    local_updates: |before: Option<&RowView<media_files::Entity>>,
                    value: &Track| -> Result<Vec<(media_files::Column, Value)>, DbErr> {
                        let available = if let Some(before) = before {
                            before.read::<bool>(media_files::Column::LocalAvailable)? && (value.content_hash.is_none() || (before.read::<Option<String>>(media_files::Column::ContentHash)? == value.content_hash && before.read::<Option<i64>>(media_files::Column::ContentSize)? == value.content_size))
                        } else {
                            false
                        };
                        Ok(vec![(media_files::Column::LocalAvailable, available.into())])
                    },
                    references: {
                        cover_art_id => "artworks"
                    },
                    limits: {
                        extension => (0, 32),
                        content_hash => (64, 64),
                        content_size => (0, i64::MAX as i128),
                        sample_rate => (0, i32::MAX as i128)
                    },
                    validate: |_: Uuid,
                    value: Option<&Track>| -> Result<(), DbErr> {
                        if let Some(value) = value {
                            validate_blob(value.content_hash.as_deref(), value.content_size)?;
                        } Ok(())
                    },
                }
            },
            Artworks => {
                tag: "artworks",
                entity: media_cover_art::Entity,
                value: Artwork,
                mapping: ArtworkMap,
                policy: {
                    uuid: media_cover_art::Column::HlcUuid,
                    fields: {
                        digest: String => media_cover_art::Column::ContentHash,
                        size: i64 => media_cover_art::Column::ContentSize,
                        primary_color: Option<i32> => media_cover_art::Column::PrimaryColor
                    },
                    defaults: {
                        media_cover_art::Column::FileHash => "",
                        media_cover_art::Column::Binary => Vec::<u8>::new(),
                        media_cover_art::Column::LocalAvailable => false
                    },
                    // The library migration leaves a cover over the size limit without a digest and
                    // marks it unavailable, so it has nothing to sync.
                    source_filter: Condition::all().add(media_cover_art::Column::HlcUuid.ne(Uuid::nil().to_string())).add(media_cover_art::Column::HlcUuid.ne("")).add(media_cover_art::Column::ContentHash.is_not_null()),
                    limits: {
                        digest => (64, 64),
                        size => (1, 8 * 1024 * 1024)
                    },
                    identity: |value: &Artwork| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::artwork(&value.digest))
                    },
                    validate: |_: Uuid,
                    value: Option<&Artwork>| -> Result<(), DbErr> {
                        if let Some(value) = value {
                            validate_blob(Some(&value.digest), Some(value.size))?;
                        } Ok(())
                    },
                }
            },
            FileArtists => {
                tag: "file_artists",
                entity: media_file_artists::Entity,
                value: FileArtist,
                mapping: FileArtistMap,
                policy: {
                    uuid: media_file_artists::Column::HlcUuid,
                    fields: {
                        file_id: Uuid => media_file_artists::Column::TrackUuid,
                        artist_id: Uuid => media_file_artists::Column::ArtistUuid
                    },
                    defaults: {},
                    references: {
                        file_id => "tracks",
                        artist_id => "artists"
                    },
                    identity: |value: &FileArtist| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::membership(value.file_id, ParentKind::Artist, value.artist_id))
                    },
                }
            },
            FileGenres => {
                tag: "file_genres",
                entity: media_file_genres::Entity,
                value: FileGenre,
                mapping: FileGenreMap,
                policy: {
                    uuid: media_file_genres::Column::HlcUuid,
                    fields: {
                        file_id: Uuid => media_file_genres::Column::TrackUuid,
                        genre_id: Uuid => media_file_genres::Column::GenreUuid
                    },
                    defaults: {},
                    references: {
                        file_id => "tracks",
                        genre_id => "genres"
                    },
                    identity: |value: &FileGenre| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::membership(value.file_id, ParentKind::Genre, value.genre_id))
                    },
                }
            },
            TrackAlbums => {
                tag: "track_albums",
                entity: media_file_albums::Entity,
                value: TrackAlbum,
                mapping: TrackAlbumMap,
                policy: {
                    uuid: media_file_albums::Column::HlcUuid,
                    fields: {
                        track_id: Uuid => media_file_albums::Column::TrackUuid,
                        album_id: Uuid => media_file_albums::Column::AlbumUuid,
                        track_number: Option<i32> => media_file_albums::Column::TrackNumber
                    },
                    defaults: {},
                    references: {
                        track_id => "tracks",
                        album_id => "albums"
                    },
                    limits: {
                        track_number => (0, i32::MAX as i128)
                    },
                    identity: |value: &TrackAlbum| -> Result<Uuid, DbErr> {
                        Ok(value.track_id)
                    },
                }
            },
            Likes => {
                tag: "likes",
                entity: media_file_likes::Entity,
                value: Liked,
                mapping: LikedMap,
                policy: {
                    uuid: media_file_likes::Column::HlcUuid,
                    fields: {
                        track_id: Uuid => media_file_likes::Column::TrackUuid,
                        liked: bool => media_file_likes::Column::Liked,
                        updated_at: String => media_file_likes::Column::UpdatedAt
                    },
                    defaults: {},
                    references: {
                        track_id => "tracks"
                    },
                    limits: {
                        updated_at => (0, 64)
                    },
                    identity: |value: &Liked| -> Result<Uuid, DbErr> {
                        Ok(value.track_id)
                    },
                }
            },
            PlaylistEntries => {
                tag: "playlist_entries",
                entity: media_file_playlists::Entity,
                value: PlaylistEntry,
                mapping: PlaylistEntryMap,
                policy: {
                    uuid: media_file_playlists::Column::HlcUuid,
                    fields: {
                        playlist_id: Uuid => media_file_playlists::Column::PlaylistUuid,
                        track_id: Uuid => media_file_playlists::Column::TrackUuid,
                        position: i32 => media_file_playlists::Column::Position
                    },
                    defaults: {},
                    references: {
                        playlist_id => "playlists",
                        track_id => "tracks"
                    },
                }
            },
            PlaybackStatistics => {
                tag: "playback_statistics",
                entity: media_file_stats::Entity,
                value: PlaybackStatistics,
                mapping: PlaybackStatisticsMap,
                policy: {
                    uuid: media_file_stats::Column::HlcUuid,
                    fields: {
                        track_id: Uuid => media_file_stats::Column::TrackUuid,
                        device_id: Uuid => media_file_stats::Column::DeviceId,
                        skipped: i32 => media_file_stats::Column::Skipped,
                        played_through: i32 => media_file_stats::Column::PlayedThrough,
                        updated_at: String => media_file_stats::Column::UpdatedAt
                    },
                    defaults: {},
                    references: {
                        track_id => "tracks"
                    },
                    limits: {
                        skipped => (0, i32::MAX as i128),
                        played_through => (0, i32::MAX as i128),
                        updated_at => (0, 64)
                    },
                    owner: {
                        field: device_id,
                        allow_removal: false
                    },
                    identity: |value: &PlaybackStatistics| -> Result<Uuid, DbErr> {
                        Ok(Uuid::new_v5(&value.track_id, value.device_id.as_bytes()))
                    },
                }
            },
            Mixes => {
                tag: "mixes",
                entity: mixes::Entity,
                value: Mix,
                mapping: MixMap,
                policy: {
                    uuid: mixes::Column::HlcUuid,
                    fields: {
                        name: String => mixes::Column::Name,
                        group: String => mixes::Column::Group,
                        mode: Option<i32> => mixes::Column::Mode,
                        scriptlet_mode: bool => mixes::Column::ScriptletMode,
                        locked: bool => mixes::Column::Locked
                    },
                    defaults: {},
                    lists: {
                        queries: MixClause => {
                            entity: mix_queries::Entity,
                            mapping: MixClauseMap,
                            owner: |row: &RowView<mixes::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![row.read::<String>(mixes::Column::HlcUuid)?.into()])
                            }
                        }
                    },
                    effects: {
                        search => {
                            entity: search_index::Entity,
                            mapping: SearchMap,
                            owner: |row: &RowView<mixes::Entity>| -> Result<Vec<Value>, DbErr> {
                                Ok(vec![Some(row.read::<i32>(mixes::Column::Id)?.to_string()).into(), Some("mix".to_owned()).into()])
                            },
                            derive: |_: &RowView<mixes::Entity>, value: Option<&Mix>| search_documents(value.map(|value| value.name.as_str())),
                        }
                    },
                    limits: {
                        name => (0, 16384),
                        group => (0, 16384)
                    },
                }
            },
            Fingerprints => {
                tag: "fingerprints",
                entity: media_file_fingerprint::Entity,
                value: Fingerprint,
                mapping: FingerprintMap,
                policy: {
                    uuid: media_file_fingerprint::Column::HlcUuid,
                    fields: {
                        file_id: Uuid => media_file_fingerprint::Column::TrackUuid,
                        bytes: BinaryValue => media_file_fingerprint::Column::Fingerprint,
                        is_duplicated: i32 => media_file_fingerprint::Column::IsDuplicated,
                        source_hash: Option<String> => media_file_fingerprint::Column::SourceHash,
                        source_size: Option<i64> => media_file_fingerprint::Column::SourceSize,
                        algorithm: Option<String> => media_file_fingerprint::Column::Algorithm,
                        configuration: Option<String> => media_file_fingerprint::Column::Configuration,
                        format_version: Option<i32> => media_file_fingerprint::Column::FormatVersion
                    },
                    defaults: {},
                    source_filter: Condition::all().add(Expr::cust_with_values("length(fingerprint) <= ?", [MAX_PORTABLE_FINGERPRINT_BYTES as i64])),
                    references: {
                        file_id => "tracks"
                    },
                    limits: {
                        bytes => (0, MAX_PORTABLE_FINGERPRINT_BYTES as i128),
                        source_hash => (64, 64),
                        source_size => (0, i64::MAX as i128),
                        algorithm => (1, 256),
                        configuration => (1, 256),
                        format_version => (0, i32::MAX as i128)
                    },
                    identity: |value: &Fingerprint| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::fingerprint(value.file_id, value.algorithm.as_deref(), value.configuration.as_deref(), value.format_version))
                    },
                    validate: |_: Uuid,
                    value: Option<&Fingerprint>| -> Result<(), DbErr> {
                        if let Some(value) = value {
                            validate_blob(value.source_hash.as_deref(), value.source_size)?;
                            validate_provenance(value.algorithm.as_deref(), value.configuration.as_deref(), value.format_version)?;
                        } Ok(())
                    },
                }
            },
            Similarities => {
                tag: "similarities",
                entity: media_file_similarity::Entity,
                value: Similarity,
                mapping: SimilarityMap,
                policy: {
                    uuid: media_file_similarity::Column::HlcUuid,
                    fields: {
                        first_file_id: Uuid => media_file_similarity::Column::FirstTrackUuid,
                        second_file_id: Uuid => media_file_similarity::Column::SecondTrackUuid,
                        first_source_hash: Option<String> => media_file_similarity::Column::FirstSourceHash,
                        first_source_size: Option<i64> => media_file_similarity::Column::FirstSourceSize,
                        second_source_hash: Option<String> => media_file_similarity::Column::SecondSourceHash,
                        second_source_size: Option<i64> => media_file_similarity::Column::SecondSourceSize,
                        algorithm: Option<String> => media_file_similarity::Column::Algorithm,
                        configuration: Option<String> => media_file_similarity::Column::Configuration,
                        format_version: Option<i32> => media_file_similarity::Column::FormatVersion,
                        score: ScoreValue => media_file_similarity::Column::Similarity
                    },
                    defaults: {},
                    references: {
                        first_file_id => "tracks",
                        second_file_id => "tracks"
                    },
                    limits: {
                        first_source_hash => (64, 64),
                        first_source_size => (0, i64::MAX as i128),
                        second_source_hash => (64, 64),
                        second_source_size => (0, i64::MAX as i128),
                        algorithm => (1, 256),
                        configuration => (1, 256),
                        format_version => (0, i32::MAX as i128)
                    },
                    identity: |value: &Similarity| -> Result<Uuid, DbErr> {
                        Ok(catalog_identity::similarity(value.first_file_id, value.second_file_id, value.algorithm.as_deref(), value.configuration.as_deref(), value.format_version))
                    },
                    validate: |_: Uuid,
                    value: Option<&Similarity>| -> Result<(), DbErr> {
                        if let Some(value) = value {
                            validate_blob(value.first_source_hash.as_deref(), value.first_source_size)?;
                            validate_blob(value.second_source_hash.as_deref(), value.second_source_size)?;
                            validate_provenance(value.algorithm.as_deref(), value.configuration.as_deref(), value.format_version)?;
                        } Ok(())
                    },
                }
            }
        }
    }
}

pub async fn encode_source_on<C: sea_orm::ConnectionTrait>(
    db: &C,
    key: &ObjectKey,
) -> anyhow::Result<Option<Payload>> {
    Ok(RuneModel::encode_on(db, key).await?)
}
pub async fn author_source_on(
    tx: &sea_orm::DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    key: ObjectKey,
) -> anyhow::Result<()> {
    let value = encode_source_on(tx, &key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source row missing during authoring"))?;
    if store
        .get_on(tx, &key)
        .await?
        .is_none_or(|record| record.value.as_ref() != Some(&value))
    {
        store
            .author_on(tx, key.clone(), Some(value.clone()))
            .await?;
        RuneModel::project_on(tx, &key, Some(&value)).await?;
    }
    Ok(())
}
pub fn blob_reference(hash: Option<&str>, size: Option<i64>) -> Result<Option<BlobRef>, DbErr> {
    validate_blob(hash, size)?;
    Ok(hash.zip(size).map(|(hash, size)| BlobRef {
        digest: hash.to_owned(),
        size: size as u64,
    }))
}
impl Track {
    pub fn blob(&self) -> Result<Option<BlobRef>, DbErr> {
        blob_reference(self.content_hash.as_deref(), self.content_size)
    }
    pub fn title(&self) -> &str {
        self.metadata
            .iter()
            .find(|tag| tag.key == "track_title")
            .map_or("", |tag| tag.value.as_str())
    }
}
impl Artwork {
    pub fn body(&self) -> Result<BlobRef, DbErr> {
        blob_reference(Some(&self.digest), Some(self.size))?
            .ok_or_else(|| DbErr::Custom("missing artwork content identity".into()))
    }
}
pub type ObjectKey = sync::ObjectKey<RuneModel>;
pub type Record = sync::Record<RuneModel>;
pub type Scope = sync::Scope<RuneModel>;
pub type PageRequest = sync::PageRequest<RuneModel>;
pub type ChangePage = sync::ChangePage<RuneModel>;
pub type MemorySyncStore = sync::MemorySyncStore<RuneModel>;
pub type StorePeer = sync::StorePeer<RuneModel>;
pub trait RuneObjectKey {
    fn liked(track_id: Uuid) -> Self;
    fn track_album(track_id: Uuid) -> Self;
}
impl RuneObjectKey for ObjectKey {
    fn liked(track_id: Uuid) -> Self {
        Self {
            collection: Collection::Likes,
            id: track_id,
        }
    }
    fn track_album(track_id: Uuid) -> Self {
        Self {
            collection: Collection::TrackAlbums,
            id: track_id,
        }
    }
}
