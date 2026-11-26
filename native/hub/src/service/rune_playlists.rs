//! Rune's own playlists in a streaming service's library, kept on this device in the library's
//! records folder (see `device_records`). They hold tracks of the service by its own keys, and
//! nothing is written to the service. The service's own playlists come in once, as Rune's (see
//! `RunePlaylists::import`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use database::error::ExpectedError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::device_records::{DeviceStore, StoredRecord};

/// The file of a library's records folder that holds its playlists.
pub(crate) const PLAYLISTS_FILE: &str = "playlists.json";

/// The extension of the marker beside the playlists that says the service's playlists were
/// imported once, after which they never are again: `playlists.imported`.
const IMPORTED_EXTENSION: &str = "imported";

/// The extension of the note beside the playlists of the service's playlists an import brought
/// in, by their ids: `playlists.imports`. One the user removed before the import was finished is
/// not brought in again by the rest of it.
const IMPORTS_EXTENSION: &str = "imports";

/// The file beside the playlists that keeps some ids of the service account the library belongs
/// to, which tell whose the records are once the library's record is removed.
const ACCOUNT_FILE: &str = "account.json";

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
    /// See `IMPORTED_EXTENSION`.
    imported_marker: PathBuf,
    /// See `IMPORTS_EXTENSION`.
    imports: PathBuf,
    /// See `ACCOUNT_FILE`.
    account: PathBuf,
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
        Self::with_store(path, DeviceStore::new)
    }

    /// Playlists whose file is not read until `release` (see `DeviceStore::held`).
    pub(crate) fn held(path: PathBuf) -> Self {
        Self::with_store(path, DeviceStore::held)
    }

    fn with_store(path: PathBuf, store: fn(PathBuf) -> DeviceStore<StoredPlaylist>) -> Self {
        Self {
            imported_marker: path.with_extension(IMPORTED_EXTENSION),
            imports: path.with_extension(IMPORTS_EXTENSION),
            account: path.with_file_name(ACCOUNT_FILE),
            store: store(path),
        }
    }

    /// Reads the file from now on (see `DeviceStore::release`).
    pub(crate) async fn release(&self) {
        self.store.release().await;
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

    /// Keeps a playlist imported from the service's playlist `from`, with `tracks` and what was
    /// known of each in order, unless one imported from it is kept already or was brought in and
    /// removed since (see `IMPORTS_EXTENSION`). The playlist is written whole, so an import cut
    /// short keeps all of it or none. Answers whether it was made. The playlists of an import
    /// come in one after another.
    pub(crate) async fn import(
        &self,
        name: &str,
        group: &str,
        from: &str,
        tracks: Vec<(String, TrackAbout)>,
    ) -> Result<bool> {
        let playlist = StoredPlaylist {
            id: Uuid::new_v4().to_string(),
            name: name.to_owned(),
            group: group.to_owned(),
            entries: tracks
                .into_iter()
                .map(|(track, about)| StoredEntry {
                    occurrence: Uuid::new_v4().to_string(),
                    track,
                    about,
                })
                .collect(),
            imported_from: Some(from.to_owned()),
        };
        let mut noted = self.noted_imports().await;
        if noted.contains(from) {
            return Ok(false);
        }
        let made = self
            .store
            .change(move |playlists| {
                if playlists
                    .iter()
                    .any(|kept| kept.imported_from == playlist.imported_from)
                {
                    return Ok(false);
                }
                playlists.push(playlist);
                Ok(true)
            })
            .await?;
        if made {
            noted.insert(from.to_owned());
            let mut ids: Vec<String> = noted.into_iter().collect();
            ids.sort_unstable();
            // The playlist is kept either way; without the note, one removed before the import
            // is finished may come in again.
            if let Err(error) = self
                .store
                .write_beside(self.imports.clone(), serde_json::to_vec(&ids)?)
                .await
            {
                log::warn!("Could not note that a playlist was imported: {error:#}");
            }
        }
        Ok(made)
    }

    /// The service's playlists an import brought in: those a playlist kept was imported from,
    /// and those noted beside them, which the user may have removed since. A file that cannot be
    /// read fails with `UnreadableStore`, which names it.
    pub(crate) async fn imported_sources(&self) -> Result<HashSet<String>> {
        let mut sources = self.noted_imports().await;
        sources.extend(
            self.list()
                .await?
                .iter()
                .filter_map(|playlist| playlist.imported_from.clone()),
        );
        Ok(sources)
    }

    /// The service's playlists noted as brought in (see `IMPORTS_EXTENSION`); none when the
    /// note cannot be read.
    async fn noted_imports(&self) -> HashSet<String> {
        tokio::fs::read(&self.imports)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
            .map(|ids| ids.into_iter().collect())
            .unwrap_or_default()
    }

    /// Whether the service's own playlists were imported once, after which they never are
    /// again.
    pub(crate) async fn imported(&self) -> bool {
        tokio::fs::try_exists(&self.imported_marker)
            .await
            .unwrap_or(false)
    }

    /// Notes that the service's own playlists were imported, unless the store was closed or is
    /// held.
    pub(crate) async fn mark_imported(&self) -> Result<()> {
        self.store
            .write_beside(self.imported_marker.clone(), Vec::new())
            .await
    }

    /// What `ACCOUNT_FILE` holds, if it can be read.
    pub(crate) async fn account(&self) -> Option<Vec<u8>> {
        tokio::fs::read(&self.account).await.ok()
    }

    /// Writes `ACCOUNT_FILE`, unless the store was closed or is held.
    pub(crate) async fn write_account(&self, bytes: Vec<u8>) -> Result<()> {
        self.store.write_beside(self.account.clone(), bytes).await
    }

    /// Waits for a change being written, then refuses every later one: the library record is
    /// being removed.
    pub(crate) async fn close(&self) {
        self.store.close().await;
    }
}

/// The file in the records folder `folder` that keeps some ids of the service account the
/// library belongs to (see `ACCOUNT_FILE`).
pub(crate) fn account_file(folder: &Path) -> PathBuf {
    folder.join(ACCOUNT_FILE)
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
    async fn an_import_keeps_each_playlist_once_beside_its_marker_and_the_account() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, playlists) = store(dir.path());
        let mine = playlists.create("Mine", "Mood").await?;
        let songs = vec![
            ("i.a".to_owned(), TrackAbout::default()),
            ("i.b".to_owned(), TrackAbout::default()),
        ];
        assert!(
            playlists
                .import("Road", "Imported", "p.1", songs.clone())
                .await?
        );
        // An import cut short and made again keeps the playlist once.
        assert!(!playlists.import("Road", "Imported", "p.1", songs).await?);
        let kept = playlists.list().await?;
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], mine);
        assert_eq!(
            (kept[1].imported_from.as_deref(), tracks(&kept[1])),
            (Some("p.1"), vec!["i.a", "i.b"])
        );
        assert!(!playlists.imported().await);
        playlists.mark_imported().await?;
        playlists.write_account(b"ids".to_vec()).await?;
        let folder = path.parent().unwrap();
        assert!(folder.join("playlists.imported").exists());
        assert_eq!(account_file(folder), folder.join("account.json"));
        let again = RunePlaylists::new(path.clone());
        assert!(again.imported().await);
        assert_eq!(again.account().await.as_deref(), Some(&b"ids"[..]));
        // Held, the playlists list none and nothing is written beside them until they are
        // released.
        let held = RunePlaylists::held(path);
        assert!(held.list().await?.is_empty());
        assert!(held.write_account(b"other".to_vec()).await.is_err());
        held.release().await;
        assert_eq!(held.list().await?.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn a_playlist_removed_before_its_import_is_finished_is_not_imported_again() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let (path, playlists) = store(dir.path());
        assert!(
            playlists
                .import("Road", "Imported", "p.1", Vec::new())
                .await?
        );
        let road = playlists.list().await?[0].id.clone();
        playlists.remove(&road).await?;
        // The import, cut short, runs again: also in a store made afresh, as at the next sign-in.
        for playlists in [&playlists, &RunePlaylists::new(path.clone())] {
            assert!(playlists.imported_sources().await?.contains("p.1"));
            assert!(
                !playlists
                    .import("Road", "Imported", "p.1", Vec::new())
                    .await?
            );
        }
        assert!(
            playlists
                .import("Jazz", "Imported", "p.2", Vec::new())
                .await?
        );
        let sources = playlists.imported_sources().await?;
        assert_eq!(sources, HashSet::from(["p.1".to_owned(), "p.2".to_owned()]));
        assert_eq!(playlists.list().await?.len(), 1);
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
