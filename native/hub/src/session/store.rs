use std::{
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use playback::{
    PlaybackMode,
    player::{OnlineInLibraryFile, PlaybackSource, PlayerStatus, PlayingItem},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredItem {
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
impl From<PlayingItem> for StoredItem {
    fn from(item: PlayingItem) -> Self {
        match item {
            PlayingItem::InLibrary(id, library_id) => Self::InLibrary { id, library_id },
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
            StoredItem::InLibrary { id, library_id } => Self::InLibrary(id, library_id),
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
pub(super) struct SessionState {
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
            version: 1,
            migrated: false,
            queue: Vec::new(),
            index: None,
            position: Duration::ZERO,
            mode: 0,
        }
    }
}
impl SessionState {
    pub fn read(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let state: Self = serde_json::from_slice(&bytes).context("Read player session")?;
                ensure!(state.version == 1, "Unsupported player session version");
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
