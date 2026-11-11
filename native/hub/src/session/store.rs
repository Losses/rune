use std::{
    collections::HashSet,
    fs::{self, File},
    io::{ErrorKind, Write},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result};
use playback::{
    PlaybackMode,
    player::{
        HandoffBlockId, InterruptedItem, OnlineInLibraryFile, PlayerStatus, PlayingItem, QueueEntry,
    },
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StoredItem {
    LibraryTrack {
        library_id: Uuid,
        track: String,
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
            PlayingItem::LibraryTrack { library_id, track } => {
                Self::LibraryTrack { library_id, track }
            }
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
            StoredItem::LibraryTrack { library_id, track } => {
                Self::LibraryTrack { library_id, track }
            }
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
struct StoredQueueEntry {
    item: StoredItem,
    block: Option<Uuid>,
}

impl From<QueueEntry> for StoredQueueEntry {
    fn from(entry: QueueEntry) -> Self {
        Self {
            item: entry.item.into(),
            block: entry.block.map(|block| block.0),
        }
    }
}

impl From<&StoredQueueEntry> for QueueEntry {
    fn from(entry: &StoredQueueEntry) -> Self {
        Self {
            item: entry.item.clone().into(),
            block: entry.block.map(HandoffBlockId),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionState {
    version: u32,
    pub session_id: Uuid,
    pub queue_revision: u64,
    pub migrated: bool,
    queue: Vec<StoredQueueEntry>,
    pub handoff_blocks: Vec<HandoffBlock>,
    pub index: Option<usize>,
    pub position: Duration,
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HandoffBlock {
    pub id: Uuid,
    pub interrupted: StoredInterruptedItem,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StoredInterruptedItem {
    Nothing,
    Item {
        index_hint: usize,
        item: StoredItem,
        position: Duration,
    },
}

impl From<InterruptedItem> for StoredInterruptedItem {
    fn from(value: InterruptedItem) -> Self {
        match value {
            InterruptedItem::Nothing => Self::Nothing,
            InterruptedItem::Item {
                index_hint,
                item,
                position,
            } => Self::Item {
                index_hint,
                item: item.into(),
                position,
            },
        }
    }
}

impl From<StoredInterruptedItem> for InterruptedItem {
    fn from(value: StoredInterruptedItem) -> Self {
        match value {
            StoredInterruptedItem::Nothing => Self::Nothing,
            StoredInterruptedItem::Item {
                index_hint,
                item,
                position,
            } => Self::Item {
                index_hint,
                item: item.into(),
                position,
            },
        }
    }
}
impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: 5,
            session_id: Uuid::new_v4(),
            queue_revision: 0,
            migrated: false,
            queue: Vec::new(),
            handoff_blocks: Vec::new(),
            index: None,
            position: Duration::ZERO,
            mode: 0,
        }
    }
}
impl SessionState {
    pub fn add_handoff_block(&mut self, id: HandoffBlockId, interrupted: InterruptedItem) {
        self.handoff_blocks.push(HandoffBlock {
            id: id.0,
            interrupted: interrupted.into(),
        });
    }

    pub fn handoff_block(&self, id: HandoffBlockId) -> Option<HandoffBlock> {
        self.handoff_blocks
            .iter()
            .find(|block| block.id == id.0)
            .cloned()
    }

    pub fn remove_handoff_block(&mut self, id: HandoffBlockId) {
        self.handoff_blocks.retain(|block| block.id != id.0);
    }

    pub fn read(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).context("Read player session")?;
                let version = value
                    .get("version")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                if version == 5 {
                    return serde_json::from_value(value).context("Read player session v5");
                }
                log::info!(
                    "Player session store has unsupported version {version}; starting empty"
                );
                let state = Self::default();
                state.write(path)?;
                Ok(state)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let state = Self::default();
                state.write(path)?;
                Ok(state)
            }
            Err(error) => Err(error).context("Read player session store"),
        }
    }
    pub fn update(&mut self, status: &PlayerStatus) {
        self.update_cursor(status);
        self.update_queue(&status.playlist);
    }
    pub fn update_queue(&mut self, entries: &[QueueEntry]) {
        self.queue = entries.iter().cloned().map(Into::into).collect();
        let live_blocks = entries
            .iter()
            .filter_map(|entry| entry.block)
            .map(|id| id.0)
            .collect::<HashSet<_>>();
        self.handoff_blocks
            .retain(|block| live_blocks.contains(&block.id));
        self.version = 5;
        if !self.queue.is_empty() {
            self.migrated = true;
        }
    }
    pub fn update_cursor(&mut self, status: &PlayerStatus) {
        self.index = status.index;
        self.position = status.position;
        self.mode = status.playback_mode.into();
    }
    pub fn queue_entries(&self) -> Vec<QueueEntry> {
        self.queue.iter().map(QueueEntry::from).collect()
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

#[cfg(test)]
mod tests {
    use super::{HandoffBlock, SessionState, StoredInterruptedItem, StoredItem, Uuid};
    use playback::player::{
        HandoffBlockId, InterruptedItem, PlayingItem, QueueEntry, SubsonicItem,
    };

    #[test]
    fn subsonic_queue_item_roundtrips_as_secret_free_identity() {
        let item = PlayingItem::from(SubsonicItem {
            library_id: Uuid::new_v4(),
            track_id: "remote-track-42".to_owned(),
        });
        let stored = StoredItem::from(item.clone());
        let encoded = serde_json::to_string(&stored).unwrap();
        assert_eq!(
            PlayingItem::from(serde_json::from_str::<StoredItem>(&encoded).unwrap()),
            item
        );
        for marker in ["t=", "s=", "p=", "password"] {
            assert!(!encoded.contains(marker));
        }
    }

    #[test]
    fn unsupported_session_version_starts_with_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let unsupported = serde_json::json!({
            "version": 4,
            "session_id": Uuid::new_v4(),
            "queue_revision": 9,
            "migrated": true,
            "queue": [{"type": "unknown"}],
            "index": 0,
            "position": 1,
            "mode": 0,
        });
        std::fs::write(&path, serde_json::to_vec(&unsupported).unwrap()).unwrap();

        let state = SessionState::read(&path).unwrap();

        assert!(state.queue_entries().is_empty());
        assert_eq!(state.queue_revision, 0);
        assert!(!state.migrated);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["version"], 5);
    }

    #[test]
    fn handoff_record_is_dropped_when_its_last_queue_tag_leaves() {
        let id = HandoffBlockId(Uuid::new_v4());
        let mut state = SessionState::default();
        state.handoff_blocks.push(HandoffBlock {
            id: id.0,
            interrupted: StoredInterruptedItem::Nothing,
        });
        let item = PlayingItem::from(SubsonicItem {
            library_id: Uuid::new_v4(),
            track_id: "block-track".to_owned(),
        });
        let entry = QueueEntry {
            item: item.clone(),
            block: Some(id),
        };
        state.update_queue(std::slice::from_ref(&entry));
        assert_eq!(state.handoff_blocks.len(), 1);
        state.update_queue(&[QueueEntry { item, block: None }]);
        assert!(state.handoff_blocks.is_empty());
    }

    #[test]
    fn tagged_queue_and_interruption_record_roundtrip_together() {
        let id = HandoffBlockId(Uuid::new_v4());
        let item = PlayingItem::from(SubsonicItem {
            library_id: Uuid::new_v4(),
            track_id: "tagged-track".to_owned(),
        });
        let mut state = SessionState::default();
        state.update_queue(&[QueueEntry {
            item: item.clone(),
            block: Some(id),
        }]);
        state.add_handoff_block(id, InterruptedItem::Nothing);
        let encoded = serde_json::to_vec(&state).unwrap();
        let restored: SessionState = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored, state);
        assert_eq!(restored.queue_entries()[0].block, Some(id));
        assert!(restored.handoff_block(id).is_some());
    }
}
