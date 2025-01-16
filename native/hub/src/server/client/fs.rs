use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};

use hub::messages::*;

use crate::connection::WSConnection;

#[derive(Clone, Debug)]
pub struct VirtualEntry {
    pub name: String,
    pub id: Option<i32>,
    pub is_directory: bool,
}

pub struct VirtualFS {
    pub current_path: PathBuf,
    pub root_dirs: Vec<String>,
    pub subdirs: HashMap<String, Vec<VirtualEntry>>,
    connection: Arc<WSConnection>,
}

impl VirtualFS {
    pub fn new(connection: Arc<WSConnection>) -> Self {
        let root_dirs = vec![
            "Artists".to_string(),
            "Playlists".to_string(),
            "Tracks".to_string(),
            "Albums".to_string(),
            "Mixes".to_string(),
        ];

        Self {
            current_path: PathBuf::from("/"),
            root_dirs,
            subdirs: HashMap::new(),
            connection,
        }
    }

    fn path_to_collection_type(&self, path: &Path) -> Option<CollectionType> {
        match path.components().nth(1)?.as_os_str().to_str()? {
            "Albums" => Some(CollectionType::Album),
            "Artists" => Some(CollectionType::Artist),
            "Playlists" => Some(CollectionType::Playlist),
            "Mixes" => Some(CollectionType::Mix),
            "Tracks" => Some(CollectionType::Track),
            _ => None,
        }
    }

    pub async fn list_current_dir(&self) -> Result<Vec<VirtualEntry>> {
        if self.current_path == Path::new("/") {
            return Ok(self
                .root_dirs
                .iter()
                .map(|name| VirtualEntry {
                    name: name.clone(),
                    id: None,
                    is_directory: true,
                })
                .collect());
        }

        let collection_type = self
            .path_to_collection_type(&self.current_path)
            .ok_or_else(|| anyhow!("Invalid path"))?;

        // If we're at the root of a collection type (e.g., /Artists)
        if self.current_path.components().count() == 2 {
            let request = FetchCollectionGroupSummaryRequest {
                collection_type: collection_type as i32,
            };

            let response: CollectionGroupSummaryResponse = self
                .connection
                .request("FetchCollectionGroupSummaryRequest", request)
                .await?;

            return Ok(response
                .groups
                .into_iter()
                .map(|group| VirtualEntry {
                    name: group.group_title,
                    id: None,
                    is_directory: true,
                })
                .collect());
        }

        // If we're in a group (e.g., /Artists/Rock)
        if self.current_path.components().count() == 3 {
            let group_title = self
                .current_path
                .components()
                .last()
                .unwrap()
                .as_os_str()
                .to_str()
                .unwrap();

            let request = FetchCollectionGroupsRequest {
                collection_type: collection_type as i32,
                bake_cover_arts: false,
                group_titles: vec![group_title.to_string()],
            };

            let response: FetchCollectionGroupsResponse = self
                .connection
                .request("FetchCollectionGroupsRequest", request)
                .await?;

            return Ok(response
                .groups
                .into_iter()
                .flat_map(|group| group.collections)
                .map(|collection| VirtualEntry {
                    name: collection.name,
                    id: Some(collection.id),
                    is_directory: false,
                })
                .collect());
        }

        Ok(Vec::new())
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_path
    }
}
