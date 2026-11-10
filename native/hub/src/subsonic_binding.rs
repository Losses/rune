use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use subsonic::{SubsonicAlbum, SubsonicArtist, SubsonicClient, SubsonicSong};

use crate::request_error::ExpectedRequestError;

/// Opaque catalog IDs are scoped to the lifetime of the open Subsonic binding.
#[derive(Default)]
pub(crate) struct SubsonicIdMap {
    forward: HashMap<SubsonicIdKind, HashMap<String, i32>>,
    reverse: HashMap<SubsonicIdKind, HashMap<i32, String>>,
    next: HashMap<SubsonicIdKind, i32>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum SubsonicIdKind {
    Artist,
    Album,
    Track,
}

impl SubsonicIdMap {
    pub(crate) fn intern(&mut self, kind: SubsonicIdKind, remote_id: &str) -> Result<i32> {
        if let Some(id) = self.forward.get(&kind).and_then(|ids| ids.get(remote_id)) {
            return Ok(*id);
        }
        let next = self.next.entry(kind).or_default();
        *next = next
            .checked_add(1)
            .ok_or_else(|| ExpectedRequestError::invalid_input("Subsonic ID map is full"))?;
        let id = *next;
        self.forward
            .entry(kind)
            .or_default()
            .insert(remote_id.to_owned(), id);
        self.reverse
            .entry(kind)
            .or_default()
            .insert(id, remote_id.to_owned());
        Ok(id)
    }

    pub(crate) fn resolve(&self, kind: SubsonicIdKind, id: i32) -> Result<&str> {
        self.reverse
            .get(&kind)
            .and_then(|ids| ids.get(&id))
            .map(String::as_str)
            .ok_or_else(|| {
                ExpectedRequestError::invalid_input("Unknown Subsonic catalog ID").into()
            })
    }
}

pub(crate) struct SubsonicBinding {
    pub(crate) client: SubsonicClient,
    pub(crate) probe: subsonic::SubsonicProbe,
    pub(crate) catalog: Mutex<SubsonicCatalog>,
}

#[derive(Default)]
pub(crate) struct SubsonicCatalog {
    pub(crate) ids: SubsonicIdMap,
    pub(crate) artists: HashMap<i32, SubsonicArtist>,
    pub(crate) albums: HashMap<i32, SubsonicAlbum>,
    pub(crate) songs: HashMap<i32, SubsonicSong>,
    pub(crate) artists_loaded: bool,
    pub(crate) albums_loaded: bool,
}

impl SubsonicBinding {
    pub(crate) fn new(client: SubsonicClient, probe: subsonic::SubsonicProbe) -> Self {
        Self {
            client,
            probe,
            catalog: Mutex::new(SubsonicCatalog::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SubsonicIdKind, SubsonicIdMap};
    use crate::request_error::ExpectedRequestError;

    #[test]
    fn ids_are_stable_per_kind_and_unknown_ids_are_typed() {
        let mut ids = SubsonicIdMap::default();
        let artist = ids.intern(SubsonicIdKind::Artist, "artist-opaque").unwrap();
        assert_eq!(artist, 1);
        assert_eq!(
            ids.intern(SubsonicIdKind::Artist, "artist-opaque").unwrap(),
            artist
        );
        assert_eq!(
            ids.intern(SubsonicIdKind::Album, "album-opaque").unwrap(),
            1
        );
        assert_eq!(
            ids.resolve(SubsonicIdKind::Artist, artist).unwrap(),
            "artist-opaque"
        );
        let error = ids.resolve(SubsonicIdKind::Track, 999).unwrap_err();
        assert!(error.downcast_ref::<ExpectedRequestError>().is_some());
    }
}
