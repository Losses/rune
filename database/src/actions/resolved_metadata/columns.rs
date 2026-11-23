use anyhow::Result;
use metadata::model::{
    AlbumMetadata, AudioContentType, AudioEncodingInfo, AudioMetadata, DiscMetadata, MetadataBase,
    ReleaseType,
};
use sea_orm::{QueryResult, Value};

use super::Winners;

// SQLite stores integers in signed 64-bit columns. Check conversions at both boundaries.
trait Column: Sized {
    fn bind(value: &Option<Self>) -> Result<Value>;
    fn read(row: &QueryResult, field: &str) -> Result<Option<Self>>;
}

macro_rules! native_columns {
    ($($kind:ty),+ $(,)?) => {
        $(impl Column for $kind {
            fn bind(value: &Option<Self>) -> Result<Value> { Ok(value.clone().into()) }
            fn read(row: &QueryResult, field: &str) -> Result<Option<Self>> {
                Ok(row.try_get("", field)?)
            }
        })+
    };
}
native_columns!(String, f64, bool);

macro_rules! unsigned_columns {
    ($($kind:ty),+ $(,)?) => {
        $(impl Column for $kind {
            fn bind(value: &Option<Self>) -> Result<Value> {
                Ok(value.map(i64::try_from).transpose()?.into())
            }
            fn read(row: &QueryResult, field: &str) -> Result<Option<Self>> {
                Ok(row.try_get::<Option<i64>>("", field)?.map(Self::try_from).transpose()?)
            }
        })+
    };
}
unsigned_columns!(u32, u64);

impl Column for AudioContentType {
    fn bind(value: &Option<Self>) -> Result<Value> {
        let value = value.map(|value| match value {
            Self::Music => "music",
            Self::Podcast => "podcast",
            Self::Audiobook => "audiobook",
            Self::Other => "other",
        });
        Ok(value.into())
    }

    fn read(row: &QueryResult, field: &str) -> Result<Option<Self>> {
        Ok(match row.try_get::<Option<String>>("", field)?.as_deref() {
            Some("music") => Some(Self::Music),
            Some("podcast") => Some(Self::Podcast),
            Some("audiobook") => Some(Self::Audiobook),
            Some("other") => Some(Self::Other),
            Some(value) => anyhow::bail!("invalid audio content type {value}"),
            None => None,
        })
    }
}

impl Column for ReleaseType {
    fn bind(value: &Option<Self>) -> Result<Value> {
        Ok(value.map(Self::as_str).into())
    }

    fn read(row: &QueryResult, field: &str) -> Result<Option<Self>> {
        row.try_get::<Option<String>>("", field)?
            .map(|value| {
                Self::from_label(&value)
                    .ok_or_else(|| anyhow::anyhow!("invalid release type {value}"))
            })
            .transpose()
    }
}

// Keep the column names and their bind/read order defined together.
macro_rules! columns {
    ($model:ty, $names:ident, $bind:ident, $fill:ident, $($field:ident),+ $(,)?) => {
        pub(super) const $names: &[&str] = &[$(stringify!($field)),+];

        pub(super) fn $bind(model: &$model, values: &mut Vec<Value>) -> Result<()> {
            $(values.push(Column::bind(&model.$field)?);)+
            Ok(())
        }

        pub(super) fn $fill(
            model: &mut $model, row: &QueryResult, file_id: i32, winners: &mut Winners,
        ) -> Result<()> {
            $(if model.$field.is_none() {
                model.$field = Column::read(row, stringify!($field))?;
                if model.$field.is_some() {
                    winners.insert(stringify!($field), file_id);
                }
            })+
            Ok(())
        }
    };
}

// Descriptive metadata is available across all three scopes.
columns!(
    MetadataBase,
    COMMON,
    bind_common,
    fill_common,
    release_date,
    original_release_date,
    version,
    sort_title,
    grouping,
    description,
    comment,
    copyright,
    license,
    script,
    work,
    part,
    section,
    opus,
    work_id,
    recording_date,
    recording_start_date,
    recording_end_date,
    recording_location,
    is_live,
    is_bootleg,
);

// Release and disc fields are selected in track order when resolving evidence.
columns!(
    AlbumMetadata,
    ALBUM,
    bind_album,
    fill_album,
    barcode,
    catalog_number,
    release_country,
    release_type,
    release_status,
    disc_count,
    declared_track_count,
    replaygain_album_gain_db,
    replaygain_album_peak,
    replaygain_reference_lufs,
);
columns!(
    DiscMetadata,
    DISC,
    bind_disc,
    fill_disc,
    media_format,
    directory_path,
    declared_track_count,
);

// Audio details can be read independently of either parent.
columns!(
    AudioMetadata,
    AUDIO,
    bind_audio,
    fill_audio,
    track_number,
    track_total,
    disc_number,
    disc_total,
    subtitle,
    bpm,
    movement_number,
    movement_total,
    file_path,
    file_size_bytes,
    file_modified_at,
    format,
    codec,
    sample_rate,
    bits_per_sample,
    channels,
    channel_layout,
    bitrate,
    frame_count,
    replaygain_track_gain_db,
    replaygain_track_peak,
    replaygain_reference_lufs,
    lyrics,
    synced_lyrics,
    original_album_title,
    isrc,
    content_type,
);
columns!(
    AudioEncodingInfo,
    ENCODING,
    bind_encoding,
    fill_encoding,
    encoder,
    encoder_settings,
    encoding_date,
    tagging_date,
);

pub(super) fn bind_duration(duration: Option<u64>) -> Result<Value> {
    Column::bind(&duration)
}

pub(super) fn duration(row: &QueryResult) -> Result<Option<u64>> {
    Column::read(row, "duration_ms")
}
