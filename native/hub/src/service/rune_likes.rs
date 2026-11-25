//! The songs liked in a streaming service's library, kept on this device in the library's records
//! folder (see `device_records`), as Rune's playlists and mixes are. Liking and unliking in Rune
//! change this store only: nothing Rune does writes to the service's account. What the service
//! has liked comes in once (see `RuneLikes::imported`).
//!
//! A song is named by the service's key of it, never by the id the UI knows it by: those are
//! given afresh each time the app runs. A like belongs to the song, not to one copy of it: a song
//! the service has as a library copy and as a catalog song has two keys, and a like is kept under
//! the first key given, while either key finds it, as an entry made before the other key was
//! known does. The store keeps every song it was told about, liked or not: an unliked song stays
//! as a note that the user decided about it, so an import never likes it again.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::service::device_records::{DeviceStore, StoredRecord};

/// The file of a library's records folder that holds its liked songs.
pub(crate) const LIKES_FILE: &str = "likes.json";

/// The extension of the marker beside the likes that says the service's likes were imported
/// once, after which they never are again: `likes.imported`.
const IMPORTED_EXTENSION: &str = "imported";

/// One song the user decided about, liked or unliked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredLike {
    /// The service's key of the song.
    pub(crate) id: String,
    /// Whether the song is liked now; a song that is not was unliked, and stays noted.
    pub(crate) liked: bool,
}

impl StoredRecord for StoredLike {
    const FIELD: &'static str = "songs";
    const FORMAT: u32 = 1;
    const WHAT: &'static str = "Rune likes";

    fn id(&self) -> &str {
        &self.id
    }
}

/// The liked songs of one service library, in a store of their own. The songs are kept in the
/// order they were liked in, the one liked last at the end.
pub(crate) struct RuneLikes {
    store: DeviceStore<StoredLike>,
    /// See `IMPORTED_EXTENSION`.
    imported_marker: PathBuf,
}

impl RuneLikes {
    /// The likes kept in the file at `path`.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self::with_store(path, DeviceStore::new)
    }

    /// Likes whose file is not read until `release` (see `DeviceStore::held`).
    pub(crate) fn held(path: PathBuf) -> Self {
        Self::with_store(path, DeviceStore::held)
    }

    fn with_store(path: PathBuf, store: fn(PathBuf) -> DeviceStore<StoredLike>) -> Self {
        Self {
            imported_marker: path.with_extension(IMPORTED_EXTENSION),
            store: store(path),
        }
    }

    /// Reads the file from now on (see `DeviceStore::release`).
    pub(crate) async fn release(&self) {
        self.store.release().await;
    }

    /// Whether the song is liked, which `keys` name: the keys it is known by, the one a like is
    /// kept under first (see the module docs). The entry made last among them decides, so a song
    /// unliked under one key is not liked by an older like under the other. A file that cannot be
    /// read fails with `UnreadableStore`, which names it.
    pub(crate) async fn is_liked<K: AsRef<str>>(&self, keys: &[K]) -> Result<bool> {
        Ok(self
            .store
            .list()
            .await?
            .iter()
            .rfind(|like| keys.iter().any(|key| key.as_ref() == like.id))
            .is_some_and(|like| like.liked))
    }

    /// Every song the store holds, liked or not, in order. A file that cannot be read lists none
    /// (see `DeviceStore::listed`).
    pub(crate) async fn listed(&self) -> Result<Arc<Vec<StoredLike>>> {
        self.store.listed().await
    }

    /// The liked songs, the one liked last first, at most `limit`. A file that cannot be read
    /// lists none, so the library home still shows (see `DeviceStore::listed`).
    pub(crate) async fn newest(&self, limit: usize) -> Result<Vec<String>> {
        Ok(self
            .listed()
            .await?
            .iter()
            .rev()
            .filter(|like| like.liked)
            .take(limit)
            .map(|like| like.id.clone())
            .collect())
    }

    /// Likes or unlikes the song which `keys` name (see `is_liked`). The like is kept under the
    /// first key; an entry the song has under another key is changed with it, so the keys never
    /// disagree afterwards. A song liked again is the one liked last; one that is liked already
    /// stays where it is.
    pub(crate) async fn set_liked<K: AsRef<str>>(&self, keys: &[K], liked: bool) -> Result<()> {
        let keys: Vec<String> = keys.iter().map(|key| key.as_ref().to_owned()).collect();
        self.store
            .change(move |likes| {
                for (at, key) in keys.iter().enumerate() {
                    match likes.iter().position(|like| like.id == *key) {
                        Some(index) if likes[index].liked == liked => {}
                        Some(index) if liked => {
                            likes.remove(index);
                            likes.push(StoredLike {
                                id: key.clone(),
                                liked,
                            });
                        }
                        Some(index) => likes[index].liked = false,
                        None if at == 0 => likes.push(StoredLike {
                            id: key.clone(),
                            liked,
                        }),
                        None => {}
                    }
                }
                Ok(())
            })
            .await
    }

    /// Likes the songs `songs`, given oldest first, that the store never heard of, as older than
    /// everything it knows: a song liked or unliked here meanwhile keeps what the user made of it.
    /// Each song is given by the keys it is known by (see `is_liked`) and is kept under the first;
    /// it is never heard of when none of its keys is known. Answers how many were added.
    pub(crate) async fn import(&self, songs: Vec<Vec<String>>) -> Result<usize> {
        self.store
            .change(move |likes| {
                let mut known: HashSet<String> = likes.iter().map(|like| like.id.clone()).collect();
                let mut added = Vec::new();
                for keys in songs {
                    let Some(first) = keys.first().cloned() else {
                        continue;
                    };
                    // Every key is noted, not only up to the first one known.
                    let fresh: Vec<bool> =
                        keys.iter().map(|key| known.insert(key.clone())).collect();
                    if fresh.iter().all(|fresh| *fresh) {
                        added.push(StoredLike {
                            id: first,
                            liked: true,
                        });
                    }
                }
                let count = added.len();
                likes.splice(0..0, added);
                Ok(count)
            })
            .await
    }

    /// Whether the service's likes were imported once, after which they never are again.
    pub(crate) async fn imported(&self) -> bool {
        tokio::fs::try_exists(&self.imported_marker)
            .await
            .unwrap_or(false)
    }

    /// Notes that the service's likes were imported, unless the store was closed or is held.
    pub(crate) async fn mark_imported(&self) -> Result<()> {
        self.store
            .write_beside(self.imported_marker.clone(), Vec::new())
            .await
    }

    /// Waits for a change being written, then refuses every later one: the library record is
    /// being removed.
    pub(crate) async fn close(&self) {
        self.store.close().await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use uuid::Uuid;

    use super::*;
    use crate::service::device_records::{device_records_folder, read_records};

    fn store(dir: &Path) -> (PathBuf, RuneLikes) {
        let folder = device_records_folder(dir, Uuid::new_v4());
        std::fs::create_dir_all(&folder).unwrap();
        (folder.clone(), RuneLikes::new(folder.join(LIKES_FILE)))
    }

    fn songs(keys: &[&str]) -> Vec<Vec<String>> {
        keys.iter().map(|key| vec![(*key).to_owned()]).collect()
    }

    /// The songs the file holds, liked or not, in order.
    fn written(folder: &Path) -> Vec<(String, bool)> {
        read_records::<StoredLike>(&folder.join(LIKES_FILE))
            .into_iter()
            .map(|like| (like.id, like.liked))
            .collect()
    }

    fn noted(list: &[(&str, bool)]) -> Vec<(String, bool)> {
        list.iter()
            .map(|(id, liked)| ((*id).to_owned(), *liked))
            .collect()
    }

    #[tokio::test]
    async fn the_newest_like_comes_first_and_an_unliked_song_is_noted() {
        let dir = tempfile::tempdir().unwrap();
        let (folder, likes) = store(dir.path());
        likes.set_liked(&["a"], true).await.unwrap();
        likes.set_liked(&["b"], true).await.unwrap();
        likes.set_liked(&["c"], true).await.unwrap();
        likes.set_liked(&["b"], false).await.unwrap();
        assert!(!likes.is_liked(&["b"]).await.unwrap());
        assert_eq!(likes.newest(10).await.unwrap(), ["c", "a"]);
        assert_eq!(likes.newest(1).await.unwrap(), ["c"]);
        // Liked again, it is the newest; one liked already keeps its place.
        likes.set_liked(&["b"], true).await.unwrap();
        likes.set_liked(&["c"], true).await.unwrap();
        assert_eq!(likes.newest(10).await.unwrap(), ["b", "c", "a"]);
        // Unliking a song never heard of notes it too.
        likes.set_liked(&["z"], false).await.unwrap();
        assert_eq!(written(&folder).len(), 4);
    }

    #[tokio::test]
    async fn an_import_adds_only_songs_never_heard_of_as_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let (folder, likes) = store(dir.path());
        likes.set_liked(&["kept"], true).await.unwrap();
        likes.set_liked(&["gone"], false).await.unwrap();
        let added = likes
            .import(songs(&["x", "kept", "gone", "y"]))
            .await
            .unwrap();
        assert_eq!(added, 2);
        assert_eq!(
            written(&folder),
            noted(&[("x", true), ("y", true), ("kept", true), ("gone", false)])
        );
        assert_eq!(likes.newest(10).await.unwrap(), ["kept", "y", "x"]);
        // Again, it adds nothing.
        assert_eq!(likes.import(songs(&["x", "gone"])).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn the_likes_and_the_marker_survive_a_new_store_and_wait_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let (folder, likes) = store(dir.path());
        likes.set_liked(&["a"], true).await.unwrap();
        assert!(!likes.imported().await);
        likes.mark_imported().await.unwrap();
        assert!(folder.join("likes.imported").exists());
        let again = RuneLikes::new(folder.join(LIKES_FILE));
        assert!(again.is_liked(&["a"]).await.unwrap());
        assert!(again.imported().await);
        // Held, the likes list none and take no change until they are released.
        let held = RuneLikes::held(folder.join(LIKES_FILE));
        assert!(!held.is_liked(&["a"]).await.unwrap());
        assert!(held.set_liked(&["b"], true).await.is_err());
        held.release().await;
        assert!(held.is_liked(&["a"]).await.unwrap());
        likes.close().await;
        assert!(likes.set_liked(&["b"], true).await.is_err());
    }

    #[tokio::test]
    async fn a_like_under_either_key_of_a_song_counts_for_both() {
        let dir = tempfile::tempdir().unwrap();
        let (folder, likes) = store(dir.path());
        let both = ["i.1", "catalog:1"];
        // An entry made before the catalog key was known.
        likes.set_liked(&["i.1"], true).await.unwrap();
        assert!(likes.is_liked(&both).await.unwrap());
        assert!(likes.is_liked(&["i.1"]).await.unwrap());
        // Unliking by both keys changes the entry it has, and makes none under the second.
        likes.set_liked(&both, false).await.unwrap();
        assert!(!likes.is_liked(&both).await.unwrap());
        assert_eq!(written(&folder), noted(&[("i.1", false)]));
        likes.set_liked(&both, true).await.unwrap();
        assert!(likes.is_liked(&both).await.unwrap());
        assert_eq!(likes.newest(10).await.unwrap(), ["i.1"]);
        // A song with one key is not touched by another's.
        assert!(!likes.is_liked(&["i.2"]).await.unwrap());
    }

    #[tokio::test]
    async fn the_latest_entry_of_a_song_decides_and_an_import_respects_every_key() {
        let dir = tempfile::tempdir().unwrap();
        let (_, likes) = store(dir.path());
        // Unliked under the catalog key only, then the library key is imported.
        likes.set_liked(&["catalog:1"], false).await.unwrap();
        let added = likes
            .import(vec![
                vec!["i.1".into(), "catalog:1".into()],
                vec!["i.2".into(), "catalog:2".into()],
                vec!["i.3".into()],
            ])
            .await
            .unwrap();
        assert_eq!(added, 2);
        assert!(!likes.is_liked(&["i.1", "catalog:1"]).await.unwrap());
        assert!(likes.is_liked(&["i.2", "catalog:2"]).await.unwrap());
        assert_eq!(likes.newest(10).await.unwrap(), ["i.3", "i.2"]);
        // Imported under the library key, then unliked by the catalog key alone: it stays
        // unliked.
        likes.set_liked(&["i.3"], true).await.unwrap();
        likes.set_liked(&["catalog:3"], false).await.unwrap();
        assert!(!likes.is_liked(&["i.3", "catalog:3"]).await.unwrap());
    }
}
