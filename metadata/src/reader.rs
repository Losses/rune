//! Read native tags and project them onto the existing library keys.

use std::path::Path;

use anyhow::Result;
use fsio::{FsIo, FsNode};

use crate::{
    model::RawTagMap,
    taglib::{self, ReadFields},
};

/// Project tag values onto the field names used by the portable track payload.
pub fn legacy_tags(tags: &RawTagMap, blacklist: Option<Vec<&str>>) -> Vec<(String, String)> {
    let blacklist =
        blacklist.unwrap_or_else(|| vec!["encoded_by", "encoder", "comment", "description"]);
    tags.iter()
        .flat_map(|(key, values)| {
            let key = match key.as_str() {
                "TITLE" => "track_title".to_owned(),
                "ALBUMARTIST" => "album_artist".to_owned(),
                "TRACKNUMBER" => "track_number".to_owned(),
                "DISCNUMBER" => "disc_number".to_owned(),
                "TRACKTOTAL" | "TOTALTRACKS" => "track_total".to_owned(),
                "DISCTOTAL" | "TOTALDISCS" => "disc_total".to_owned(),

                // Preserve key names consumed by existing search and presentation code.
                "DISCSUBTITLE" | "TXXX:DISCSUBTITLE" | "TSST" => "disc_subtitle".to_owned(),
                "SUBTITLE" => "track_subtitle".to_owned(),
                "ORIGINALDATE" => "original_date".to_owned(),
                "RELEASEDATE" => "release_date".to_owned(),
                "MEDIA" => "media_format".to_owned(),
                "ENCODEDBY" => "encoded_by".to_owned(),
                "ENCODING" => "encoder".to_owned(),
                "ENCODERSETTINGS" => "encoder_settings".to_owned(),

                // Maintain the legacy spellings for common IDs, sort fields and gain tags.
                "ISRC" => "ident_isrc".to_owned(),
                "BARCODE" => "ident_barcode".to_owned(),
                "CATALOGNUMBER" => "ident_catalog_number".to_owned(),
                "ALBUMSORT" => "sort_album".to_owned(),
                "ARTISTSORT" => "sort_artist".to_owned(),
                "TITLESORT" => "sort_track_title".to_owned(),
                "ORIGINALALBUM" => "original_album".to_owned(),
                key => key.to_lowercase().replace(' ', "_"),
            };
            let include = !blacklist.contains(&key.as_str());
            values
                .iter()
                .filter(move |_| include)
                .map(move |value| (key.clone(), value.clone()))
        })
        .collect()
}

/// Read multi-valued tags through the audio file's filesystem backend.
pub fn get_metadata(
    fsio: &FsIo,
    node: &FsNode,
    blacklist: Option<Vec<&str>>,
) -> Result<Vec<(String, String)>> {
    let tags = taglib::read_file(fsio, &node.path, ReadFields::Tags)?.tags;
    Ok(legacy_tags(&tags, blacklist))
}

/// Read embedded lyrics through the same TagLib reader used by the library scanner.
pub fn get_lyrics(fsio: &FsIo, path: &Path) -> Result<Option<String>> {
    let mut tags = taglib::read_file(fsio, path, ReadFields::Tags)?.tags;
    Ok([
        "SYLT",
        "SYNCEDLYRICS",
        "SYNCED LYRICS",
        "LYRICS",
        "UNSYNCED LYRICS",
    ]
    .into_iter()
    .find_map(|key| {
        tags.remove(key)?
            .into_iter()
            .find(|value| !value.trim().is_empty())
    }))
}
