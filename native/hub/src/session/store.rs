use std::{
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::library_pool::LibraryPool;
use anyhow::{Context, Result, ensure};
use database::actions::file::file_uuids_by_id;
use playback::{
    PlaybackMode,
    player::{LibraryItem, OnlineInLibraryFile, PlaybackSource, PlayerStatus, PlayingItem},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredItem {
    InLibrary {
        library_id: Uuid,
        file_uuid: Uuid,
    },
    IndependentFile {
        path: String,
    },
    Online {
        url: String,
        library: Option<(String, String, i32)>,
    },
    Unknown,
}
impl From<PlayingItem> for StoredItem {
    fn from(item: PlayingItem) -> Self {
        match item {
            PlayingItem::InLibrary(item) => Self::InLibrary {
                library_id: item.library_id,
                file_uuid: item.file_uuid,
            },
            PlayingItem::IndependentFile(path) => Self::IndependentFile { path },
            PlayingItem::Online(url, library) => Self::Online {
                url,
                library: library.map(|f| (f.host, f.fingerprint, f.id)),
            },
            PlayingItem::Unknown => Self::Unknown,
        }
    }
}
impl From<StoredItem> for PlayingItem {
    fn from(item: StoredItem) -> Self {
        match item {
            StoredItem::InLibrary {
                library_id,
                file_uuid,
            } => Self::InLibrary(LibraryItem {
                library_id,
                file_uuid,
            }),
            StoredItem::IndependentFile { path } => Self::IndependentFile(path),
            StoredItem::Online { url, library } => Self::Online(
                url,
                library.map(|(host, fingerprint, id)| OnlineInLibraryFile {
                    host,
                    fingerprint,
                    id,
                }),
            ),
            StoredItem::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionState {
    version: u32,
    pub migrated: bool,
    queue: Vec<StoredItem>,
    pub index: Option<usize>,
    pub position: Duration,
    pub mode: u32,
}
impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: 2,
            migrated: false,
            queue: Vec::new(),
            index: None,
            position: Duration::ZERO,
            mode: 0,
        }
    }
}
impl SessionState {
    pub async fn read(path: &Path, pool: &Arc<LibraryPool>) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&bytes).context("Read player session")?;
                let version = value
                    .get("version")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                if version == 2 {
                    return serde_json::from_value(value).context("Read player session v2");
                }
                ensure!(version == 1, "Unsupported player session version");
                #[derive(Deserialize)]
                #[serde(tag = "type", rename_all = "snake_case")]
                enum LegacyItem {
                    InLibrary {
                        id: i32,
                        library_id: String,
                    },
                    IndependentFile {
                        path: String,
                    },
                    Online {
                        url: String,
                        library: Option<(String, String, i32)>,
                    },
                    Unknown,
                }
                #[derive(Deserialize)]
                struct LegacyState {
                    migrated: bool,
                    queue: Vec<LegacyItem>,
                    index: Option<usize>,
                    position: Duration,
                    mode: u32,
                }
                let legacy: LegacyState =
                    serde_json::from_value(value.take()).context("Read player session v1")?;
                let old_index = legacy.index;
                let mut dropped_before_index = 0;
                let mut current_dropped = false;
                let mut queue = Vec::new();
                for (old_position, item) in legacy.queue.into_iter().enumerate() {
                    let converted = match item {
                        LegacyItem::InLibrary { id, library_id } => {
                            let result = async {
                                let library = pool.acquire(library_id.parse()?).await?;
                                let rows =
                                    file_uuids_by_id(library.main_db.as_ref(), &[id]).await?;
                                Ok::<_, anyhow::Error>(rows.get(&id).copied().map(|file_uuid| {
                                    StoredItem::InLibrary {
                                        library_id: library.descriptor.library_id,
                                        file_uuid,
                                    }
                                }))
                            }
                            .await;
                            match result {
                                Ok(Some(item)) => Some(item),
                                Ok(None) => {
                                    log::warn!(
                                        "Dropped player session item: library {library_id} has no file row {id}"
                                    );
                                    None
                                }
                                Err(error) => {
                                    log::warn!(
                                        "Dropped player session item {id} in library {library_id}: {error:#}"
                                    );
                                    None
                                }
                            }
                        }
                        LegacyItem::IndependentFile { path } => {
                            Some(StoredItem::IndependentFile { path })
                        }
                        LegacyItem::Online { url, library } => {
                            Some(StoredItem::Online { url, library })
                        }
                        LegacyItem::Unknown => Some(StoredItem::Unknown),
                    };
                    if let Some(item) = converted {
                        queue.push(item);
                    } else if old_index.is_some_and(|index| old_position < index) {
                        dropped_before_index += 1;
                    } else if old_index == Some(old_position) {
                        current_dropped = true;
                    }
                }
                let index = old_index
                    .filter(|_| !current_dropped)
                    .and_then(|index| index.checked_sub(dropped_before_index))
                    .filter(|index| *index < queue.len());
                let state = Self {
                    version: 2,
                    migrated: legacy.migrated,
                    queue,
                    index,
                    position: legacy.position,
                    mode: legacy.mode,
                };
                state.write(path)?;
                Ok(state)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).context("Read player session store"),
        }
    }
    pub fn update(&mut self, status: &PlayerStatus) {
        self.update_cursor(status);
        self.update_queue(&status.playlist);
    }
    pub fn update_queue(&mut self, items: &[PlayingItem]) {
        self.queue = items.iter().cloned().map(Into::into).collect();
        self.version = 2;
        if !self.queue.is_empty() {
            self.migrated = true;
        }
    }
    pub fn update_cursor(&mut self, status: &PlayerStatus) {
        self.index = status.index;
        self.position = status.position;
        self.mode = status.playback_mode.into();
    }
    pub fn tracks(&self) -> Vec<(PlayingItem, PlaybackSource)> {
        self.queue
            .iter()
            .cloned()
            .map(|item| {
                let item: PlayingItem = item.into();
                let source = match &item {
                    PlayingItem::IndependentFile(path) => {
                        PlaybackSource::Local(PathBuf::from(path))
                    }
                    _ => PlaybackSource::LibraryStream,
                };
                (item, source)
            })
            .collect()
    }
    pub fn playback_mode(&self) -> PlaybackMode {
        self.mode.into()
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("Session store has no directory")?;
        fs::create_dir_all(dir).context("Create session directory")?;
        let temporary = path.with_extension("json.tmp");
        let mut file = File::create(&temporary).context("Create temporary session store")?;
        file.write_all(&serde_json::to_vec(self)?)
            .context("Write session store")?;
        file.sync_all().context("Sync session store")?;
        fs::rename(&temporary, path).context("Publish session store")?;
        #[cfg(unix)]
        File::open(dir)?
            .sync_all()
            .context("Sync session directory")?;
        Ok(())
    }
}
