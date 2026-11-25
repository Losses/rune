//! Rune's own playlists in a streaming service's library, kept on this device in the library's
//! records folder (see `device_records`). They hold tracks of the service by its own keys, and
//! nothing is written to the service.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use database::error::ExpectedError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::device_records::{DeviceStore, StoredRecord};

/// The file of a library's records folder that holds its playlists.
pub(crate) const PLAYLISTS_FILE: &str = "playlists.json";

/// One track of a playlist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredEntry {
    /// Names this entry for as long as it is in the playlist, however the others move.
    pub(crate) occurrence: String,
    /// The service's key of the track.
    pub(crate) track: String,
    /// What was known of the track when it was added.
    #[serde(flatten)]
    pub(crate) about: TrackAbout,
}

/// What was known of a track when it was added to a playlist, so that it can still be played or
/// named once the service no longer has it under its key. Each part may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TrackAbout {
    /// The catalog id of a song of the library, which plays it once its library copy is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) catalog_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) album: Option<String>,
}

/// One playlist, with its tracks in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredPlaylist {
    /// Names the playlist for good. The ids the UI knows it by are the binding's.
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: String,
    pub(crate) entries: Vec<StoredEntry>,
    /// The id of the service's own playlist it was imported from, so an import that was cut
    /// short never imports that playlist twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) imported_from: Option<String>,
}

impl StoredRecord for StoredPlaylist {
    const FIELD: &'static str = "playlists";
    const FORMAT: u32 = 1;
    const WHAT: &'static str = "Rune playlists";

    fn id(&self) -> &str {
        &self.id
    }
}

/// The playlists of one service library, in a store of their own.
pub(crate) struct RunePlaylists {
    store: DeviceStore<StoredPlaylist>,
}

fn playlist_not_found() -> anyhow::Error {
    ExpectedError::NotFound { what: "playlist" }.into()
}

fn occurrence_not_found() -> anyhow::Error {
    ExpectedError::NotFound {
        what: "playlist occurrence",
    }
    .into()
}

impl RunePlaylists {
    /// The playlists kept in the file at `path`.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            store: DeviceStore::new(path),
        }
    }

    /// Every playlist, in the order they were made. A file that cannot be read fails with
    /// `UnreadableStore`, which names it.
    pub(crate) async fn list(&self) -> Result<Arc<Vec<StoredPlaylist>>> {
        self.store.list().await
    }

    /// The playlists to list. A file that cannot be read lists none, so the lists that show
    /// them still answer; it is logged once, until the file can be read again, and everything
    /// that uses one of the playlists still fails with it.
    pub(crate) async fn listed(&self) -> Result<Arc<Vec<StoredPlaylist>>> {
        self.store.listed().await
    }

    /// The playlist `id`.
    pub(crate) async fn playlist(&self, id: &str) -> Result<StoredPlaylist> {
        self.list()
            .await?
            .iter()
            .find(|playlist| playlist.id == id)
            .cloned()
            .ok_or_else(playlist_not_found)
    }

    /// Makes a playlist without tracks.
    pub(crate) async fn create(&self, name: &str, group: &str) -> Result<StoredPlaylist> {
        let playlist = StoredPlaylist {
            id: Uuid::new_v4().to_string(),
            name: name.to_owned(),
            group: group.to_owned(),
            entries: Vec::new(),
            imported_from: None,
        };
        let made = playlist.clone();
        self.store
            .change(move |playlists| {
                playlists.push(playlist);
                Ok(())
            })
            .await?;
        Ok(made)
    }

    /// Names and groups the playlist `id` anew.
    pub(crate) async fn update(&self, id: &str, name: &str, group: &str) -> Result<StoredPlaylist> {
        let (id, name, group) = (id.to_owned(), name.to_owned(), group.to_owned());
        self.store
            .change(move |playlists| {
                let playlist = find(playlists, &id)?;
                playlist.name = name;
                playlist.group = group;
                Ok(playlist.clone())
            })
            .await
    }

    /// Removes the playlist `id` with its tracks.
    pub(crate) async fn remove(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.store
            .change(move |playlists| {
                let index = playlists
                    .iter()
                    .position(|playlist| playlist.id == id)
                    .ok_or_else(playlist_not_found)?;
                playlists.remove(index);
                Ok(())
            })
            .await
    }

    /// Puts `track`, of which `about` is known, into the playlist `id` at `position`, or at the
    /// end when it is None or past the end. Answers the new entry's occurrence.
    pub(crate) async fn add(
        &self,
        id: &str,
        track: &str,
        about: TrackAbout,
        position: Option<usize>,
    ) -> Result<String> {
        let (id, track) = (id.to_owned(), track.to_owned());
        self.store
            .change(move |playlists| {
                let playlist = find(playlists, &id)?;
                let occurrence = Uuid::new_v4().to_string();
                let index = position.unwrap_or(usize::MAX).min(playlist.entries.len());
                playlist.entries.insert(
                    index,
                    StoredEntry {
                        occurrence: occurrence.clone(),
                        track,
                        about,
                    },
                );
                Ok(occurrence)
            })
            .await
    }

    /// Takes the entry `occurrence` out of the playlist `id`.
    pub(crate) async fn remove_entry(&self, id: &str, occurrence: &str) -> Result<()> {
        let (id, occurrence) = (id.to_owned(), occurrence.to_owned());
        self.store
            .change(move |playlists| {
                let playlist = find(playlists, &id)?;
                let index = entry_index(playlist, &occurrence)?;
                playlist.entries.remove(index);
                Ok(())
            })
            .await
    }

    /// Moves the entry `occurrence` of the playlist `id` so that it ends up at `position`, or
    /// last when that is past the end.
    pub(crate) async fn move_entry(
        &self,
        id: &str,
        occurrence: &str,
        position: usize,
    ) -> Result<()> {
        let (id, occurrence) = (id.to_owned(), occurrence.to_owned());
        self.store
            .change(move |playlists| {
                let playlist = find(playlists, &id)?;
                let index = entry_index(playlist, &occurrence)?;
                let entry = playlist.entries.remove(index);
                let position = position.min(playlist.entries.len());
                playlist.entries.insert(position, entry);
                Ok(())
            })
            .await
    }

    /// Waits for a change being written, then refuses every later one: the library record is
    /// being removed.
    pub(crate) async fn close(&self) {
        self.store.close().await;
    }
}

fn find<'a>(playlists: &'a mut [StoredPlaylist], id: &str) -> Result<&'a mut StoredPlaylist> {
    playlists
        .iter_mut()
        .find(|playlist| playlist.id == id)
        .ok_or_else(playlist_not_found)
}

fn entry_index(playlist: &StoredPlaylist, occurrence: &str) -> Result<usize> {
    playlist
        .entries
        .iter()
        .position(|entry| entry.occurrence == occurrence)
        .ok_or_else(occurrence_not_found)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::request_error::ExpectedRequestError;
    use crate::service::device_records::device_records_folder;

    fn store(dir: &Path) -> (PathBuf, RunePlaylists) {
        let path = device_records_folder(dir, Uuid::new_v4()).join(PLAYLISTS_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        (path.clone(), RunePlaylists::new(path))
    }

    fn tracks(playlist: &StoredPlaylist) -> Vec<&str> {
        playlist
            .entries
            .iter()
            .map(|entry| entry.track.as_str())
            .collect()
    }

    #[tokio::test]
    async fn playlists_are_made_filled_reordered_and_read_back_by_another_store() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, playlists) = store(dir.path());
        let made = playlists.create("Night", "Favorite").await?;
        let first = playlists
            .add(&made.id, "i.a", TrackAbout::default(), None)
            .await?;
        playlists
            .add(&made.id, "i.b", TrackAbout::default(), Some(0))
            .await?;
        playlists
            .add(&made.id, "catalog:1", TrackAbout::default(), Some(1))
            .await?;
        playlists
            .add(&made.id, "i.c", TrackAbout::default(), Some(99))
            .await?;
        assert_eq!(
            tracks(&playlists.playlist(&made.id).await?),
            ["i.b", "catalog:1", "i.a", "i.c"]
        );
        playlists.move_entry(&made.id, &first, 0).await?;
        playlists.move_entry(&made.id, &first, 99).await?;
        assert_eq!(
            tracks(&playlists.playlist(&made.id).await?),
            ["i.b", "catalog:1", "i.c", "i.a"]
        );
        playlists.remove_entry(&made.id, &first).await?;
        playlists.update(&made.id, "Late Night", "Mood").await?;

        let again = RunePlaylists::new(path);
        let read = again.playlist(&made.id).await?;
        assert_eq!(
            (read.name.as_str(), read.group.as_str()),
            ("Late Night", "Mood")
        );
        assert_eq!(tracks(&read), ["i.b", "catalog:1", "i.c"]);
        assert_eq!(read, playlists.playlist(&made.id).await?);

        again.remove(&made.id).await?;
        assert!(again.list().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn what_was_known_of_a_track_is_kept_and_may_be_missing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, playlists) = store(dir.path());
        let made = playlists.create("Night", "Favorite").await?;
        let about = TrackAbout {
            catalog_id: Some("777".into()),
            title: Some("Tune".into()),
            artist: Some("Singer".into()),
            album: None,
        };
        playlists.add(&made.id, "i.a", about.clone(), None).await?;
        playlists
            .add(&made.id, "i.b", TrackAbout::default(), None)
            .await?;
        let written = std::fs::read_to_string(&path)?;
        // Parts that are not known are left out of the file.
        assert!(written.contains("\"catalog_id\": \"777\"") && !written.contains("album"));
        assert!(!written.contains("imported_from"));
        let read = RunePlaylists::new(path.clone()).playlist(&made.id).await?;
        assert_eq!(read.entries[0].about, about);
        assert_eq!(read.entries[1].about, TrackAbout::default());

        // An entry that names its track only is read too.
        std::fs::write(
            &path,
            br#"{"format": 1, "playlists": [{"id": "a", "name": "A", "group": "G",
                "entries": [{"occurrence": "o", "track": "i.a"}]}]}"#,
        )?;
        let bare = RunePlaylists::new(path).playlist("a").await?;
        assert_eq!(bare.entries[0].about, TrackAbout::default());
        assert_eq!(bare.imported_from, None);
        Ok(())
    }

    #[tokio::test]
    async fn unknown_playlists_and_entries_are_not_found() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (_, playlists) = store(dir.path());
        let made = playlists.create("Night", "Favorite").await?;
        let not_found = |error: anyhow::Error| {
            matches!(
                error.downcast_ref::<ExpectedError>(),
                Some(ExpectedError::NotFound { .. })
            )
        };
        assert!(not_found(playlists.playlist("missing").await.unwrap_err()));
        assert!(not_found(
            playlists
                .add("missing", "i.a", TrackAbout::default(), None)
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            playlists
                .remove_entry(&made.id, "missing")
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            playlists
                .move_entry(&made.id, "missing", 0)
                .await
                .unwrap_err()
        ));
        assert!(not_found(playlists.remove("missing").await.unwrap_err()));
        assert!(not_found(
            playlists.update("missing", "A", "G").await.unwrap_err()
        ));
        Ok(())
    }

    #[tokio::test]
    async fn closed_playlists_refuse_changes_and_keep_their_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, playlists) = store(dir.path());
        playlists.create("Night", "Favorite").await?;
        playlists.close().await;
        let refused = playlists.create("Later", "Favorite").await.unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ));
        assert_eq!(RunePlaylists::new(path).list().await?.len(), 1);
        Ok(())
    }
}
