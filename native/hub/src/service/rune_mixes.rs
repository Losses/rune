//! Rune's own mixes in a streaming service's library, kept on this device in the library's
//! records folder (see `device_records`), as Rune's playlists are. A mix keeps the items its
//! queries name by the service's keys of them, never by the ids the UI knows them by: those are
//! given afresh each time the app runs.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use database::error::ExpectedError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::messages::PlaybackMode;
use crate::service::device_records::{DeviceStore, StoredRecord};

/// The file of a library's records folder that holds its mixes.
pub(crate) const MIXES_FILE: &str = "mixes.json";

/// One query of a mix, as a mix query is sent, except that an item it names is named by the
/// service's key of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredQuery {
    pub(crate) operator: String,
    pub(crate) parameter: String,
}

/// One mix, with its queries in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredMix {
    /// Names the mix for good. The ids the UI knows it by are the binding's.
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: String,
    /// The mode the mix plays in; none keeps the player's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mode: Option<PlaybackMode>,
    pub(crate) queries: Vec<StoredQuery>,
}

impl StoredRecord for StoredMix {
    const FIELD: &'static str = "mixes";
    const FORMAT: u32 = 1;
    const WHAT: &'static str = "Rune mixes";

    fn id(&self) -> &str {
        &self.id
    }
}

/// The mixes of one service library, in a store of their own.
pub(crate) struct RuneMixes {
    store: DeviceStore<StoredMix>,
}

fn mix_not_found() -> anyhow::Error {
    ExpectedError::NotFound { what: "mix" }.into()
}

fn find<'a>(mixes: &'a mut [StoredMix], id: &str) -> Result<&'a mut StoredMix> {
    mixes
        .iter_mut()
        .find(|mix| mix.id == id)
        .ok_or_else(mix_not_found)
}

impl RuneMixes {
    /// The mixes kept in the file at `path`.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            store: DeviceStore::new(path),
        }
    }

    /// Every mix, in the order they were made. A file that cannot be read fails with
    /// `UnreadableStore`, which names it.
    pub(crate) async fn list(&self) -> Result<Arc<Vec<StoredMix>>> {
        self.store.list().await
    }

    /// The mixes to list beside the service's own. A file that cannot be read lists none, so
    /// the service's mixes still show; everything that uses one of Rune's still fails with it.
    pub(crate) async fn listed(&self) -> Result<Arc<Vec<StoredMix>>> {
        self.store.listed().await
    }

    /// The mix `id`.
    pub(crate) async fn mix(&self, id: &str) -> Result<StoredMix> {
        self.list()
            .await?
            .iter()
            .find(|mix| mix.id == id)
            .cloned()
            .ok_or_else(mix_not_found)
    }

    /// Makes a mix of `queries`.
    pub(crate) async fn create(
        &self,
        name: &str,
        group: &str,
        mode: Option<PlaybackMode>,
        queries: Vec<StoredQuery>,
    ) -> Result<StoredMix> {
        let mix = StoredMix {
            id: Uuid::new_v4().to_string(),
            name: name.to_owned(),
            group: group.to_owned(),
            mode,
            queries,
        };
        let made = mix.clone();
        self.store
            .change(move |mixes| {
                mixes.push(mix);
                Ok(())
            })
            .await?;
        Ok(made)
    }

    /// Names, groups and fills the mix `id` anew.
    pub(crate) async fn update(
        &self,
        id: &str,
        name: &str,
        group: &str,
        mode: Option<PlaybackMode>,
        queries: Vec<StoredQuery>,
    ) -> Result<StoredMix> {
        let (id, name, group) = (id.to_owned(), name.to_owned(), group.to_owned());
        self.store
            .change(move |mixes| {
                let mix = find(mixes, &id)?;
                mix.name = name;
                mix.group = group;
                mix.mode = mode;
                mix.queries = queries;
                Ok(mix.clone())
            })
            .await
    }

    /// Adds `query` to the end of the mix `id`.
    pub(crate) async fn add_query(&self, id: &str, query: StoredQuery) -> Result<()> {
        let id = id.to_owned();
        self.store
            .change(move |mixes| {
                find(mixes, &id)?.queries.push(query);
                Ok(())
            })
            .await
    }

    /// Removes the mix `id`.
    pub(crate) async fn remove(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.store
            .change(move |mixes| {
                let index = mixes
                    .iter()
                    .position(|mix| mix.id == id)
                    .ok_or_else(mix_not_found)?;
                mixes.remove(index);
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::request_error::ExpectedRequestError;
    use crate::service::device_records::device_records_folder;

    fn store(dir: &Path) -> (PathBuf, RuneMixes) {
        let path = device_records_folder(dir, Uuid::new_v4()).join(MIXES_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        (path.clone(), RuneMixes::new(path))
    }

    fn query(operator: &str, parameter: &str) -> StoredQuery {
        StoredQuery {
            operator: operator.to_owned(),
            parameter: parameter.to_owned(),
        }
    }

    #[tokio::test]
    async fn mixes_are_written_by_their_keys_and_read_back_by_another_store() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, mixes) = store(dir.path());
        // Nothing is written until something changes.
        assert!(mixes.list().await?.is_empty());
        assert!(!path.exists());

        let made = mixes
            .create(
                "Night",
                "Favorite",
                Some(PlaybackMode::Shuffle),
                vec![query("lib::artist", "name:ann")],
            )
            .await?;
        mixes
            .add_query(&made.id, query("lib::track", "catalog:1"))
            .await?;
        let kept = mixes.create("Day", "Favorite", None, Vec::new()).await?;
        mixes
            .update(
                &kept.id,
                "Late Day",
                "Mood",
                None,
                vec![query("lib::album", "l.a")],
            )
            .await?;

        let written = std::fs::read_to_string(&path)?;
        assert!(written.contains("\"format\": 1") && written.contains("\"mixes\""));
        // A mix that keeps the player's mode says nothing of it.
        assert_eq!(written.matches("\"mode\"").count(), 1);

        let again = RuneMixes::new(path.clone());
        let read = again.mix(&made.id).await?;
        assert_eq!(read.mode, Some(PlaybackMode::Shuffle));
        assert_eq!(
            read.queries,
            [
                query("lib::artist", "name:ann"),
                query("lib::track", "catalog:1")
            ]
        );
        let read = again.mix(&kept.id).await?;
        assert_eq!(
            (read.name.as_str(), read.group.as_str()),
            ("Late Day", "Mood")
        );
        assert_eq!(*again.list().await?, *mixes.list().await?);

        again.remove(&made.id).await?;
        assert_eq!(again.list().await?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn unknown_mixes_are_not_found_and_a_closed_store_refuses_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (_, mixes) = store(dir.path());
        let not_found = |error: anyhow::Error| {
            matches!(
                error.downcast_ref::<ExpectedError>(),
                Some(ExpectedError::NotFound { .. })
            )
        };
        assert!(not_found(mixes.mix("missing").await.unwrap_err()));
        assert!(not_found(
            mixes
                .add_query("missing", query("lib::all", "true"))
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            mixes
                .update("missing", "A", "G", None, Vec::new())
                .await
                .unwrap_err()
        ));
        assert!(not_found(mixes.remove("missing").await.unwrap_err()));
        mixes.close().await;
        let refused = mixes
            .create("Night", "Favorite", None, Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ));
        Ok(())
    }
}
