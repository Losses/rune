use std::{collections::HashMap, fs::File, path::Path};

use anyhow::{Result, bail};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, Value};
use symphonia::core::probe::{Hint, ProbeResult};

use ::fsio::FsNode;

fn create_standard_tag_key_maps() -> (
    HashMap<StandardTagKey, &'static str>,
    HashMap<&'static str, StandardTagKey>,
) {
    let mut key_to_string = HashMap::new();
    let mut string_to_key = HashMap::new();

    let entries = vec![
        (StandardTagKey::AcoustidFingerprint, "acoustid_fingerprint"),
        (StandardTagKey::AcoustidId, "acoustid_id"),
        (StandardTagKey::Album, "album"),
        (StandardTagKey::AlbumArtist, "album_artist"),
        (StandardTagKey::Arranger, "arranger"),
        (StandardTagKey::Artist, "artist"),
        (StandardTagKey::Bpm, "bpm"),
        (StandardTagKey::Comment, "comment"),
        (StandardTagKey::Compilation, "compilation"),
        (StandardTagKey::Composer, "composer"),
        (StandardTagKey::Conductor, "conductor"),
        (StandardTagKey::ContentGroup, "content_group"),
        (StandardTagKey::Copyright, "copyright"),
        (StandardTagKey::Date, "date"),
        (StandardTagKey::Description, "description"),
        (StandardTagKey::DiscNumber, "disc_number"),
        (StandardTagKey::DiscSubtitle, "disc_subtitle"),
        (StandardTagKey::DiscTotal, "disc_total"),
        (StandardTagKey::EncodedBy, "encoded_by"),
        (StandardTagKey::Encoder, "encoder"),
        (StandardTagKey::EncoderSettings, "encoder_settings"),
        (StandardTagKey::EncodingDate, "encoding_date"),
        (StandardTagKey::Engineer, "engineer"),
        (StandardTagKey::Ensemble, "ensemble"),
        (StandardTagKey::Genre, "genre"),
        (StandardTagKey::IdentAsin, "ident_asin"),
        (StandardTagKey::IdentBarcode, "ident_barcode"),
        (StandardTagKey::IdentCatalogNumber, "ident_catalog_number"),
        (StandardTagKey::IdentEanUpn, "ident_ean_upn"),
        (StandardTagKey::IdentIsrc, "ident_isrc"),
        (StandardTagKey::IdentPn, "ident_pn"),
        (StandardTagKey::IdentPodcast, "ident_podcast"),
        (StandardTagKey::IdentUpc, "ident_upc"),
        (StandardTagKey::Label, "label"),
        (StandardTagKey::Language, "language"),
        (StandardTagKey::License, "license"),
        (StandardTagKey::Lyricist, "lyricist"),
        (StandardTagKey::Lyrics, "lyrics"),
        (StandardTagKey::MediaFormat, "media_format"),
        (StandardTagKey::MixDj, "mix_dj"),
        (StandardTagKey::MixEngineer, "mix_engineer"),
        (StandardTagKey::Mood, "mood"),
        (StandardTagKey::MovementName, "movement_name"),
        (StandardTagKey::MovementNumber, "movement_number"),
        (
            StandardTagKey::MusicBrainzAlbumArtistId,
            "musicbrainz_album_artist_id",
        ),
        (StandardTagKey::MusicBrainzAlbumId, "musicbrainz_album_id"),
        (StandardTagKey::MusicBrainzArtistId, "musicbrainz_artist_id"),
        (StandardTagKey::MusicBrainzDiscId, "musicbrainz_disc_id"),
        (StandardTagKey::MusicBrainzGenreId, "musicbrainz_genre_id"),
        (StandardTagKey::MusicBrainzLabelId, "musicbrainz_label_id"),
        (
            StandardTagKey::MusicBrainzOriginalAlbumId,
            "musicbrainz_original_album_id",
        ),
        (
            StandardTagKey::MusicBrainzOriginalArtistId,
            "musicbrainz_original_artist_id",
        ),
        (
            StandardTagKey::MusicBrainzRecordingId,
            "musicbrainz_recording_id",
        ),
        (
            StandardTagKey::MusicBrainzReleaseGroupId,
            "musicbrainz_release_group_id",
        ),
        (
            StandardTagKey::MusicBrainzReleaseStatus,
            "musicbrainz_release_status",
        ),
        (
            StandardTagKey::MusicBrainzReleaseTrackId,
            "musicbrainz_release_track_id",
        ),
        (
            StandardTagKey::MusicBrainzReleaseType,
            "musicbrainz_release_type",
        ),
        (StandardTagKey::MusicBrainzTrackId, "musicbrainz_track_id"),
        (StandardTagKey::MusicBrainzWorkId, "musicbrainz_work_id"),
        (StandardTagKey::Opus, "opus"),
        (StandardTagKey::OriginalAlbum, "original_album"),
        (StandardTagKey::OriginalArtist, "original_artist"),
        (StandardTagKey::OriginalDate, "original_date"),
        (StandardTagKey::OriginalFile, "original_file"),
        (StandardTagKey::OriginalWriter, "original_writer"),
        (StandardTagKey::Owner, "owner"),
        (StandardTagKey::Part, "part"),
        (StandardTagKey::PartTotal, "part_total"),
        (StandardTagKey::Performer, "performer"),
        (StandardTagKey::Podcast, "podcast"),
        (StandardTagKey::PodcastCategory, "podcast_category"),
        (StandardTagKey::PodcastDescription, "podcast_description"),
        (StandardTagKey::PodcastKeywords, "podcast_keywords"),
        (StandardTagKey::Producer, "producer"),
        (StandardTagKey::PurchaseDate, "purchase_date"),
        (StandardTagKey::Rating, "rating"),
        (StandardTagKey::ReleaseCountry, "release_country"),
        (StandardTagKey::ReleaseDate, "release_date"),
        (StandardTagKey::Remixer, "remixer"),
        (StandardTagKey::ReplayGainAlbumGain, "replaygain_album_gain"),
        (StandardTagKey::ReplayGainAlbumPeak, "replaygain_album_peak"),
        (StandardTagKey::ReplayGainTrackGain, "replaygain_track_gain"),
        (StandardTagKey::ReplayGainTrackPeak, "replaygain_track_peak"),
        (StandardTagKey::Script, "script"),
        (StandardTagKey::SortAlbum, "sort_album"),
        (StandardTagKey::SortAlbumArtist, "sort_album_artist"),
        (StandardTagKey::SortArtist, "sort_artist"),
        (StandardTagKey::SortComposer, "sort_composer"),
        (StandardTagKey::SortTrackTitle, "sort_track_title"),
        (StandardTagKey::TaggingDate, "tagging_date"),
        (StandardTagKey::TrackNumber, "track_number"),
        (StandardTagKey::TrackSubtitle, "track_subtitle"),
        (StandardTagKey::TrackTitle, "track_title"),
        (StandardTagKey::TrackTotal, "track_total"),
        (StandardTagKey::TvEpisode, "tv_episode"),
        (StandardTagKey::TvEpisodeTitle, "tv_episode_title"),
        (StandardTagKey::TvNetwork, "tv_network"),
        (StandardTagKey::TvSeason, "tv_season"),
        (StandardTagKey::TvShowTitle, "tv_show_title"),
        (StandardTagKey::Url, "url"),
        (StandardTagKey::UrlArtist, "url_artist"),
        (StandardTagKey::UrlCopyright, "url_copyright"),
        (StandardTagKey::UrlInternetRadio, "url_internet_radio"),
        (StandardTagKey::UrlLabel, "url_label"),
        (StandardTagKey::UrlOfficial, "url_official"),
        (StandardTagKey::UrlPayment, "url_payment"),
        (StandardTagKey::UrlPodcast, "url_podcast"),
        (StandardTagKey::UrlPurchase, "url_purchase"),
        (StandardTagKey::UrlSource, "url_source"),
        (StandardTagKey::Version, "version"),
        (StandardTagKey::Writer, "writer"),
    ];

    for (key, value) in entries {
        key_to_string.insert(key, value);
        string_to_key.insert(value, key);
    }

    (key_to_string, string_to_key)
}

lazy_static::lazy_static! {
    static ref STANDARD_TAG_KEY_TO_STRING: HashMap<StandardTagKey, &'static str> = {
        let (map, _) = create_standard_tag_key_maps();
        map
    };
    static ref STRING_TO_STANDARD_TAG_KEY: HashMap<&'static str, StandardTagKey> = {
        let (_, map) = create_standard_tag_key_maps();
        map
    };
}

pub fn standard_tag_key_to_string(key: StandardTagKey) -> String {
    STANDARD_TAG_KEY_TO_STRING
        .get(&key)
        .unwrap_or(&"")
        .to_string()
}

pub fn string_to_standard_tag_key(s: &str) -> Option<StandardTagKey> {
    STRING_TO_STANDARD_TAG_KEY.get(s).cloned()
}

pub fn push_tags(
    revision: &MetadataRevision,
    metadata_list: &mut Vec<(String, String)>,
    field_blacklist: &[&str],
) {
    for tag in revision.tags() {
        let std_key = match tag.std_key {
            Some(standard_key) => standard_tag_key_to_string(standard_key),
            None => String::from(""),
        };

        if field_blacklist.contains(&std_key.as_str()) {
            continue;
        }

        let value: String = match &tag.value {
            Value::String(val) => val.clone(),
            Value::UnsignedInt(val) => val.to_string(),
            Value::SignedInt(val) => val.to_string(),
            Value::Float(val) => val.to_string(),
            Value::Boolean(val) => val.to_string(),
            Value::Binary(_) => String::from("[Binary data]"),
            Value::Flag => String::from("[Flag]"),
        };
        metadata_list.push((std_key, value));
    }
}

fn probe_audio_file<P: AsRef<Path>>(file_path: P) -> Result<ProbeResult> {
    if !Path::new(file_path.as_ref()).exists() {
        bail!("File not found");
    }

    // Create a probe hint using the file's extension.
    let mut hint = Hint::new();
    let file_path_str = file_path.as_ref().to_string_lossy();

    // Open the media source.
    let src = File::open(&file_path)?;

    // Create the media source stream.
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let ext = file_path_str.split('.').next_back().unwrap_or_default();
    hint.with_extension(ext);

    // Use the default options for metadata and format readers.
    let fmt_opts: FormatOptions = Default::default();
    let meta_opts: MetadataOptions = Default::default();

    // Probe the media source.
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| anyhow::anyhow!("Failed to probe file: {e}"))?;

    Ok(probed)
}

pub fn get_lyrics<P: AsRef<Path>>(file_path: P) -> Result<Option<String>> {
    let mut probed = probe_audio_file(file_path)?;
    let mut format = probed.format;

    let format_metadata = format.metadata();
    let metadata_rev = if let Some(current) = format_metadata.current() {
        Some(current.clone())
    } else if let Some(metadata) = probed.metadata.get() {
        metadata.current().cloned()
    } else {
        None
    };

    if let Some(revision) = metadata_rev {
        for tag in revision.tags() {
            // Check if this is the standard lyrics tag
            if tag.std_key == Some(StandardTagKey::Lyrics) {
                // Convert value to string based on type
                let value = match &tag.value {
                    Value::String(s) => s.clone(),
                    Value::UnsignedInt(n) => n.to_string(),
                    Value::SignedInt(n) => n.to_string(),
                    Value::Float(n) => n.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => continue, // Skip binary data and other non-text types
                };
                return Ok(Some(value));
            }
        }
    }

    Ok(None)
}

pub fn get_metadata(
    fs_node: &FsNode,
    field_blacklist: Option<Vec<&str>>,
) -> Result<Vec<(String, String)>> {
    let mut probed = probe_audio_file(fs_node.path.clone())?;
    let mut format = probed.format;
    let mut metadata_list = Vec::new();

    let blacklist =
        field_blacklist.unwrap_or(vec!["encoded_by", "encoder", "comment", "description"]);

    if let Some(metadata_rev) = format.metadata().current() {
        push_tags(metadata_rev, &mut metadata_list, &blacklist);
    } else if let Some(metadata_rev) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        push_tags(metadata_rev, &mut metadata_list, &blacklist);
    }

    Ok(metadata_list)
}
