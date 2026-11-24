use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use anyhow::Result;

use crate::request_error::ExpectedRequestError;

/// Gives a service's opaque catalog ids the integer ids Rune's messages carry, counted
/// separately for each kind of id. The ids last as long as the binding that holds the map.
pub(crate) struct RemoteIdMap<K> {
    forward: HashMap<K, HashMap<String, i32>>,
    reverse: HashMap<K, HashMap<i32, String>>,
    next: HashMap<K, i32>,
}

impl<K> Default for RemoteIdMap<K> {
    fn default() -> Self {
        Self {
            forward: HashMap::new(),
            reverse: HashMap::new(),
            next: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + Hash + Debug> RemoteIdMap<K> {
    pub(crate) fn intern(&mut self, kind: K, remote_id: &str) -> Result<i32> {
        if let Some(id) = self.forward.get(&kind).and_then(|ids| ids.get(remote_id)) {
            return Ok(*id);
        }
        let next = self.next.entry(kind).or_default();
        *next = next
            .checked_add(1)
            .ok_or_else(|| ExpectedRequestError::invalid_input("Catalog ID map is full"))?;
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

    /// The id given to `remote_id`, if there is one.
    pub(crate) fn lookup(&self, kind: K, remote_id: &str) -> Option<i32> {
        self.forward.get(&kind)?.get(remote_id).copied()
    }

    pub(crate) fn resolve(&self, kind: K, id: i32) -> Result<&str> {
        self.reverse
            .get(&kind)
            .and_then(|ids| ids.get(&id))
            .map(String::as_str)
            .ok_or_else(|| {
                ExpectedRequestError::invalid_input(format!("Unknown catalog ID: {kind:?} {id}"))
                    .into()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteIdMap;
    use crate::request_error::ExpectedRequestError;

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    enum Kind {
        Artist,
        Album,
        Track,
    }

    #[test]
    fn ids_are_stable_per_kind_and_unknown_ids_are_typed() {
        let mut ids = RemoteIdMap::default();
        let artist = ids.intern(Kind::Artist, "artist-opaque").unwrap();
        assert_eq!(artist, 1);
        assert_eq!(ids.intern(Kind::Artist, "artist-opaque").unwrap(), artist);
        assert_eq!(ids.intern(Kind::Album, "album-opaque").unwrap(), 1);
        assert_eq!(ids.resolve(Kind::Artist, artist).unwrap(), "artist-opaque");
        assert_eq!(ids.lookup(Kind::Artist, "artist-opaque"), Some(artist));
        assert_eq!(ids.lookup(Kind::Track, "artist-opaque"), None);
        let error = ids.resolve(Kind::Track, 999).unwrap_err();
        assert!(error.downcast_ref::<ExpectedRequestError>().is_some());
    }

    #[test]
    fn an_unknown_id_names_its_kind_and_value() {
        let ids = RemoteIdMap::<Kind>::default();
        let error = ids.resolve(Kind::Artist, 0).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        assert!(error.to_string().contains("Unknown catalog ID: Artist 0"));
    }
}
