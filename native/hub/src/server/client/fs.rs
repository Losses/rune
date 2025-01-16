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
                    is_directory: true,
                })
                .collect());
        }

        Ok(Vec::new())
    }

    pub async fn verify_group_exists(
        &self,
        collection_type: CollectionType,
        group_name: &str,
    ) -> Result<bool> {
        let request = FetchCollectionGroupSummaryRequest {
            collection_type: collection_type as i32,
        };

        let response: CollectionGroupSummaryResponse = self
            .connection
            .request("FetchCollectionGroupSummaryRequest", request)
            .await?;

        Ok(response
            .groups
            .iter()
            .any(|group| group.group_title == group_name))
    }

    pub async fn verify_collection_exists(
        &self,
        collection_type: CollectionType,
        group_name: &str,
        collection_name: &str,
    ) -> Result<bool> {
        let request = FetchCollectionGroupsRequest {
            collection_type: collection_type as i32,
            bake_cover_arts: false,
            group_titles: vec![group_name.to_string()],
        };

        let response: FetchCollectionGroupsResponse = self
            .connection
            .request("FetchCollectionGroupsRequest", request)
            .await?;

        Ok(response
            .groups
            .iter()
            .flat_map(|group| &group.collections)
            .any(|collection| collection.name == collection_name))
    }

    pub async fn validate_path(&self, new_path: &Path) -> Result<bool> {
        match new_path.components().count() {
            // Root path is always valid
            1 => Ok(true),
            // First level directories must be in root_dirs
            2 => Ok(self.root_dirs.contains(
                &new_path
                    .components()
                    .last()
                    .unwrap()
                    .as_os_str()
                    .to_string_lossy()
                    .to_string(),
            )),
            // Second level directories (groups) must exist in the server
            3 => {
                let collection_type = self
                    .path_to_collection_type(new_path)
                    .ok_or_else(|| anyhow!("Invalid collection type"))?;
                let group_name = new_path
                    .components()
                    .last()
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid group name"))?;
                self.verify_group_exists(collection_type, group_name).await
            }
            // Third level (individual collections) must exist in the server
            4 => {
                let collection_type = self
                    .path_to_collection_type(new_path)
                    .ok_or_else(|| anyhow!("Invalid collection type"))?;
                let group_name = new_path
                    .components()
                    .nth(2)
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid group name"))?;
                let collection_name = new_path
                    .components()
                    .last()
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid collection name"))?;
                self.verify_collection_exists(collection_type, group_name, collection_name)
                    .await
            }
            _ => Ok(false),
        }
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_path
    }
}
