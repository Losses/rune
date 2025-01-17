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

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub entries: Vec<VirtualEntry>,
    pub collection_type: CollectionType,
}

pub struct VirtualFS {
    pub current_path: PathBuf,
    pub root_dirs: Vec<String>,
    pub cache: HashMap<PathBuf, CacheEntry>,
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
            cache: HashMap::new(),
            connection,
        }
    }

    fn cache_entries(
        &mut self,
        path: PathBuf,
        entries: Vec<VirtualEntry>,
        collection_type: CollectionType,
    ) {
        self.cache.insert(
            path,
            CacheEntry {
                entries,
                collection_type,
            },
        );
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

    async fn fetch_collection_group_summary(
        &self,
        collection_type: CollectionType,
    ) -> Result<CollectionGroupSummaryResponse> {
        let request = FetchCollectionGroupSummaryRequest {
            collection_type: collection_type as i32,
        };
        self.connection
            .request("FetchCollectionGroupSummaryRequest", request)
            .await
    }

    async fn fetch_collection_groups(
        &self,
        collection_type: CollectionType,
        group_titles: Vec<String>,
    ) -> Result<FetchCollectionGroupsResponse> {
        let request = FetchCollectionGroupsRequest {
            collection_type: collection_type as i32,
            bake_cover_arts: false,
            group_titles,
        };
        self.connection
            .request("FetchCollectionGroupsRequest", request)
            .await
    }

    async fn fetch_mix_queries_by_mix_id(&self, mix_id: i32) -> Result<Vec<MixQuery>> {
        let request = FetchMixQueriesRequest { mix_id };
        let response: FetchMixQueriesResponse = self
            .connection
            .request("FetchMixQueriesRequest", request)
            .await?;
        Ok(response.result)
    }

    fn build_collection_query(
        &self,
        collection_type: CollectionType,
        id: i32,
    ) -> Result<Vec<(String, String)>> {
        if collection_type == CollectionType::Mix {
            return Err(anyhow!("Not Allow"));
        }
        let operator = match collection_type {
            CollectionType::Album => "lib::album",
            CollectionType::Artist => "lib::artist",
            CollectionType::Playlist => "lib::playlist",
            CollectionType::Track => "lib::track",
            _ => return Err(anyhow!("Invalid collection type")),
        };
        Ok(vec![(operator.to_string(), id.to_string())])
    }

    async fn build_query(
        &self,
        collection_type: CollectionType,
        id: i32,
    ) -> Result<Vec<(String, String)>> {
        if collection_type == CollectionType::Mix {
            let queries = self.fetch_mix_queries_by_mix_id(id).await?;
            Ok(queries
                .into_iter()
                .map(|q| (q.operator, q.parameter))
                .collect())
        } else {
            self.build_collection_query(collection_type, id)
        }
    }

    pub async fn list_current_dir(&mut self) -> Result<Vec<VirtualEntry>> {
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

        let entries = if self.current_path == Path::new("/") {
            // Root directory
            let entries = self
                .root_dirs
                .iter()
                .map(|name| VirtualEntry {
                    name: name.clone(),
                    id: None,
                    is_directory: true,
                })
                .collect::<Vec<_>>();

            Ok(entries)
        } else {
            match self.current_path.components().count() {
                // If we're at the root of a collection type (e.g., /Artists)
                2 => {
                    let response = self.fetch_collection_group_summary(collection_type).await?;

                    Ok(response
                        .groups
                        .into_iter()
                        .map(|group| VirtualEntry {
                            name: group.group_title,
                            id: None,
                            is_directory: true,
                        })
                        .collect::<Vec<_>>())
                }
                // If we're in a group (e.g., /Artists/:Group)
                3 => {
                    let group_title = self
                        .current_path
                        .components()
                        .last()
                        .unwrap()
                        .as_os_str()
                        .to_str()
                        .unwrap()
                        .to_string();
                    let response = self
                        .fetch_collection_groups(collection_type, vec![group_title])
                        .await?;

                    Ok(response
                        .groups
                        .into_iter()
                        .flat_map(|group| group.collections)
                        .map(|collection| VirtualEntry {
                            name: collection.name,
                            id: Some(collection.id),
                            is_directory: true,
                        })
                        .collect::<Vec<_>>())
                }
                4 => {
                    let parent_path = self.current_path.parent().unwrap().to_path_buf();
                    let collection_name = self.current_path.file_name().unwrap().to_str().unwrap();

                    let collection_id = if let Some(parent_cache) = self.cache.get(&parent_path) {
                        parent_cache
                            .entries
                            .iter()
                            .find(|e| e.name == collection_name)
                            .and_then(|e| e.id)
                            .ok_or_else(|| anyhow!("Collection not found in cache"))?
                    } else {
                        return Err(anyhow!("Parent directory not cached"));
                    };

                    let queries = self.build_query(collection_type, collection_id).await?;
                    let request = MixQueryRequest {
                        queries: queries
                            .into_iter()
                            .map(|(operator, parameter)| MixQuery {
                                operator,
                                parameter,
                            })
                            .collect(),
                        cursor: 0,
                        page_size: 100,
                        bake_cover_arts: false,
                    };
                    let mix_response: MixQueryResponse =
                        self.connection.request("MixQueryRequest", request).await?;

                    Ok(mix_response
                        .files
                        .into_iter()
                        .map(|file| VirtualEntry {
                            name: file.title,
                            id: Some(file.id),
                            is_directory: false,
                        })
                        .collect::<Vec<_>>())
                }
                _ => Ok(Vec::new()),
            }
        };

        if let Some(collection_type) = self.path_to_collection_type(&self.current_path) {
            if let Ok(ref entries) = entries {
                self.cache_entries(self.current_path.clone(), entries.clone(), collection_type);
            }
        }

        entries
    }

    pub async fn verify_group_exists(
        &self,
        collection_type: CollectionType,
        group_name: &str,
    ) -> Result<bool> {
        let response = self.fetch_collection_group_summary(collection_type).await?;
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
        let response = self
            .fetch_collection_groups(collection_type, vec![group_name.to_string()])
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
