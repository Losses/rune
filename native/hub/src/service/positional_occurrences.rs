//! The entries of a playlist that a service names only by their order. An entry is named by its
//! position together with a fingerprint of the order it was listed in, so a page or an edit
//! that names an entry of an older order is refused instead of reaching another entry.

use std::collections::HashMap;
use std::fmt::Display;
use std::ops::Range;

use anyhow::Result;
use database::actions::playlists::OCCURRENCE_PAGE_SIZES;

use crate::messages::{
    FetchPlaylistOccurrencesRequest, FetchPlaylistOccurrencesResponse, MediaFile,
    PlaylistOccurrence, PlaylistOccurrenceCursor,
};
use crate::request_error::ExpectedRequestError;

/// The length of a playlist fingerprint, in lowercase hexadecimal digits.
const PLAYLIST_FINGERPRINT_LEN: usize = 16;

/// Identifies one order of a playlist's entries, named by the keys of their tracks.
pub(crate) fn playlist_fingerprint<'a>(tracks: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = blake3::Hasher::new();
    for track in tracks {
        hasher.update(&(track.len() as u64).to_le_bytes());
        hasher.update(track.as_bytes());
    }
    hasher.finalize().to_hex()[..PLAYLIST_FINGERPRINT_LEN].to_owned()
}

fn is_playlist_fingerprint(value: &str) -> bool {
    value.len() == PLAYLIST_FINGERPRINT_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Names a playlist entry: its position together with the fingerprint of the order it was
/// listed in.
pub(crate) fn occurrence_id(position: impl Display, fingerprint: &str) -> String {
    format!("{position}@{fingerprint}")
}

/// The position and listing fingerprint an occurrence ID names.
pub(crate) fn parse_occurrence_id(value: &str) -> Result<(usize, &str)> {
    value
        .split_once('@')
        .filter(|(_, fingerprint)| is_playlist_fingerprint(fingerprint))
        .and_then(|(position, fingerprint)| Some((position.parse().ok()?, fingerprint)))
        .ok_or_else(|| ExpectedRequestError::invalid_input("Invalid playlist occurrence").into())
}

/// The position of a listed occurrence in `tracks`, the playlist as it is now. A position names
/// the entry it was listed for only while the playlist keeps the order it was listed in, so an
/// occurrence listed before any change to the playlist is rejected.
pub(crate) fn occurrence_position(
    (position, fingerprint): (usize, &str),
    tracks: &[String],
) -> Result<usize> {
    if fingerprint != playlist_fingerprint(tracks.iter().map(String::as_str)) {
        return Err(ExpectedRequestError::PlaylistChanged.into());
    }
    if position >= tracks.len() {
        return Err(ExpectedRequestError::invalid_input("Unknown playlist occurrence").into());
    }
    Ok(position)
}

/// The occurrence a listing page continues after, checked with the page size before the
/// playlist is read. A cursor names the last entry of the previous page, so a cursor whose
/// occurrence names another position did not come from a listing.
pub(crate) fn listed_after(
    request: &FetchPlaylistOccurrencesRequest,
) -> Result<Option<(usize, &str)>> {
    if !OCCURRENCE_PAGE_SIZES.contains(&request.page_size) {
        return Err(ExpectedRequestError::invalid_input(format!(
            "Playlist page size must be {}..={}",
            OCCURRENCE_PAGE_SIZES.start(),
            OCCURRENCE_PAGE_SIZES.end()
        ))
        .into());
    }
    let Some(cursor) = &request.after else {
        return Ok(None);
    };
    match parse_occurrence_id(&cursor.occurrence_id) {
        Ok((position, fingerprint))
            if i32::try_from(position).is_ok_and(|id| id == cursor.position) =>
        {
            Ok(Some((position, fingerprint)))
        }
        _ => Err(ExpectedRequestError::invalid_input("Invalid playlist occurrence cursor").into()),
    }
}

/// Which entries of a playlist one listing page holds.
pub(crate) struct OccurrencePage {
    /// The positions of the page's entries.
    pub(crate) range: Range<usize>,
    fingerprint: String,
    more: bool,
}

/// The page that follows `after` in the playlist as it is now, whose entries play `tracks` in
/// order. A page continues only the order the previous page was listed in.
pub(crate) fn occurrence_page<'a>(
    after: Option<(usize, &str)>,
    tracks: impl ExactSizeIterator<Item = &'a str>,
    page_size: u32,
) -> Result<OccurrencePage> {
    let len = tracks.len();
    let fingerprint = playlist_fingerprint(tracks);
    let start = match after {
        None => 0,
        Some((position, listed)) if listed == fingerprint => position + 1,
        Some(_) => return Err(ExpectedRequestError::PlaylistChanged.into()),
    }
    .min(len);
    let end = len.min(start.saturating_add(usize::try_from(page_size)?));
    Ok(OccurrencePage {
        range: start..end,
        fingerprint,
        more: end < len,
    })
}

/// The response for `page`, whose tracks, in wire form and in order, are `files`.
pub(crate) fn occurrences_response(
    request: &FetchPlaylistOccurrencesRequest,
    page: OccurrencePage,
    files: Vec<MediaFile>,
    cover_art_map: HashMap<i32, String>,
) -> Result<FetchPlaylistOccurrencesResponse> {
    let entries = page
        .range
        .zip(files)
        .map(|(position, file)| {
            let position = i32::try_from(position)
                .map_err(|_| ExpectedRequestError::invalid_input("Playlist is too large"))?;
            Ok(PlaylistOccurrence {
                occurrence_id: occurrence_id(position, &page.fingerprint),
                position,
                track_uuid: file.track.clone(),
                file: Some(file),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let next = page
        .more
        .then(|| entries.last())
        .flatten()
        .map(|entry| PlaylistOccurrenceCursor {
            position: entry.position,
            occurrence_id: entry.occurrence_id.clone(),
        });
    Ok(FetchPlaylistOccurrencesResponse {
        request_id: request.request_id.clone(),
        entries,
        next,
        cover_art_map,
        call_id: request.call_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AudioAvailability, AudioOrigin, RequestErrorKind};
    use crate::request_error::classify;

    fn listing(
        page_size: u32,
        after: Option<PlaylistOccurrenceCursor>,
    ) -> FetchPlaylistOccurrencesRequest {
        FetchPlaylistOccurrencesRequest {
            request_id: "listing".into(),
            playlist_id: 1,
            after,
            page_size,
            bake_cover_arts: false,
            call_id: Some(4),
        }
    }

    fn file(track: &str) -> MediaFile {
        MediaFile {
            id: 0,
            library_id: "library".into(),
            track: track.into(),
            path: String::new(),
            artist: String::new(),
            album: String::new(),
            title: track.into(),
            duration: 0.0,
            cover_art_id: None,
            track_number: None,
            audio_availability: AudioAvailability::RemoteOnly,
            audio_origin: AudioOrigin::Remote,
        }
    }

    /// Lists the page of `request` in a playlist of `tracks`.
    fn page_of(
        tracks: &[&str],
        request: &FetchPlaylistOccurrencesRequest,
    ) -> Result<FetchPlaylistOccurrencesResponse> {
        let after = listed_after(request)?;
        let page = occurrence_page(after, tracks.iter().copied(), request.page_size)?;
        let files = tracks[page.range.clone()]
            .iter()
            .map(|track| file(track))
            .collect();
        occurrences_response(request, page, files, HashMap::new())
    }

    fn entries(response: &FetchPlaylistOccurrencesResponse) -> Vec<(i32, &str)> {
        response
            .entries
            .iter()
            .map(|entry| (entry.position, entry.track_uuid.as_str()))
            .collect()
    }

    #[test]
    fn pages_follow_each_other_to_the_end_and_name_the_order_they_list() {
        let tracks = ["a", "b", "a", "c", "d"];
        let first = page_of(&tracks, &listing(2, None)).unwrap();
        assert_eq!(entries(&first), [(0, "a"), (1, "b")]);
        assert_eq!(
            (first.request_id.as_str(), first.call_id),
            ("listing", Some(4))
        );
        let fingerprint = playlist_fingerprint(tracks);
        assert_eq!(
            first.entries[1].occurrence_id,
            occurrence_id(1, &fingerprint)
        );
        let second = page_of(&tracks, &listing(2, first.next.clone())).unwrap();
        assert_eq!(entries(&second), [(2, "a"), (3, "c")]);
        let last = page_of(&tracks, &listing(2, second.next.clone())).unwrap();
        assert_eq!(entries(&last), [(4, "d")]);
        assert!(last.next.is_none());
        // A page that ends exactly at the end has no next one.
        assert!(
            page_of(&tracks[..4], &listing(4, None))
                .unwrap()
                .next
                .is_none()
        );
        assert!(page_of(&[], &listing(4, None)).unwrap().entries.is_empty());
    }

    #[test]
    fn a_page_of_an_older_order_is_a_conflict() {
        let first = page_of(&["a", "b", "c"], &listing(1, None)).unwrap();
        let error = page_of(&["b", "c"], &listing(1, first.next)).err().unwrap();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::PlaylistChanged)
        ));
        assert_eq!(classify(&error), RequestErrorKind::Conflict);
    }

    #[test]
    fn page_sizes_and_cursors_a_listing_cannot_name_are_input_errors() {
        let fingerprint = playlist_fingerprint(["a", "b"]);
        let cursor = |position, occurrence_id: String| PlaylistOccurrenceCursor {
            position,
            occurrence_id,
        };
        for request in [
            listing(0, None),
            listing(129, None),
            listing(2, Some(cursor(0, occurrence_id(1, &fingerprint)))),
            listing(2, Some(cursor(-1, "-1".into()))),
            listing(2, Some(cursor(0, "0".into()))),
            listing(2, Some(cursor(0, "0@".into()))),
            listing(
                2,
                Some(cursor(0, format!("0@{}", fingerprint.to_uppercase()))),
            ),
            listing(2, Some(cursor(0, format!("0@{fingerprint}0")))),
        ] {
            let error = listed_after(&request).err().unwrap();
            let after = request
                .after
                .as_ref()
                .map(|after| after.occurrence_id.as_str());
            assert_eq!(
                classify(&error),
                RequestErrorKind::InvalidInput,
                "{after:?}"
            );
        }
        assert!(listed_after(&listing(128, None)).is_ok());
    }

    #[test]
    fn an_edit_names_its_entry_only_in_the_order_it_was_listed_in() {
        let tracks = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
        let fingerprint = playlist_fingerprint(tracks.iter().map(String::as_str));
        let listed = occurrence_id(2, &fingerprint);
        let listed = parse_occurrence_id(&listed).unwrap();
        assert_eq!(occurrence_position(listed, &tracks).unwrap(), 2);
        // Removing the first entry moves the listed one to another position.
        let error = occurrence_position(listed, &tracks[1..]).err().unwrap();
        assert_eq!(classify(&error), RequestErrorKind::Conflict);
        let past_the_end = occurrence_id(3, &fingerprint);
        let past_the_end = parse_occurrence_id(&past_the_end).unwrap();
        let error = occurrence_position(past_the_end, &tracks).err().unwrap();
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
        for malformed in ["2", "first@0123456789abcdef", "2@0123456789ABCDEF"] {
            let error = parse_occurrence_id(malformed).err().unwrap();
            assert_eq!(
                classify(&error),
                RequestErrorKind::InvalidInput,
                "{malformed}"
            );
        }
    }
}
