use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use colored::Colorize;

use hub::messages::*;

use crate::api::{
    build_query, fetch_collection_group_summary, fetch_collection_groups, path_to_collection_type,
    send_mix_query_request,
};
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
    pub connection: Arc<WSConnection>,
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

    async fn find_entry_by_id_and_type(
        &self,
        id: i32,
        collection_type: CollectionType,
    ) -> Result<Option<(PathBuf, VirtualEntry)>> {
        let root_dir = collection_type.as_str();

        let root_path = PathBuf::from("/").join(root_dir);

        if collection_type == CollectionType::Track {
            let query = vec![("lib::directory.deep".to_string(), "/".to_string())];

            let mix_response = send_mix_query_request(query, &self.connection).await?;

            for file in mix_response.files {
                if file.id == id {
                    return Ok(Some((
                        root_path.join(&file.title),
                        VirtualEntry {
                            name: file.title,
                            id: Some(file.id),
                            is_directory: false,
                        },
                    )));
                }
            }

            return Ok(None);
        }

        // Fetch the group summary
        let summary = fetch_collection_group_summary(collection_type, &self.connection).await?;

        // Iterate through each group
        for group in summary.groups {
            let group_path = root_path.join(&group.group_title);

            // Fetch the collections within the group
            let collections =
                fetch_collection_groups(collection_type, vec![group.group_title], &self.connection)
                    .await?;

            // Search for the matching id within the collections
            for group in collections.groups {
                for collection in group.collections {
                    if collection.id == id {
                        return Ok(Some((
                            group_path.join(&collection.name),
                            VirtualEntry {
                                name: collection.name,
                                id: Some(collection.id),
                                is_directory: true,
                            },
                        )));
                    }
                }
            }
        }
        Ok(None)
    }

    fn get_collection_type_from_current_path(&self) -> Option<CollectionType> {
        if self.current_path == PathBuf::from("/") {
            None
        } else {
            path_to_collection_type(&self.current_path)
        }
    }

    pub async fn resolve_path_with_ids(&mut self, path: &str) -> Result<PathBuf> {
        let mut current = self.current_path.clone();

        // Get the collection type from the current path
        let mut collection_type = self.get_collection_type_from_current_path();

        for component in Path::new(path).components() {
            let component_str = component
                .as_os_str()
                .to_str()
                .ok_or_else(|| anyhow!("Invalid path component"))?;

            if component_str == "." {
                continue;
            } else if component_str == ".." {
                if current != PathBuf::from("/") {
                    current.pop();
                    // Update collection type after moving up
                    collection_type = path_to_collection_type(&current);
                }
            } else if component_str == "/" {
                current = PathBuf::from("/");
                collection_type = None;
            } else {
                // Check if we're already at depth 4 (which would make the next component depth 5)
                let current_depth = current.components().count();

                // Attempt to parse the component as an ID
                if let Ok(id) = component_str.parse::<i32>() {
                    if current_depth == 4 {
                        // At depth 4, we're dealing with a file - just append the name directly
                        let parent_path = current.clone();

                        // If cache doesn't exist, build it
                        if !self.cache.contains_key(&parent_path) {
                            // We need to build the cache for the parent directory
                            let queries = self.path_to_query(&parent_path).await?;
                            let mix_response =
                                send_mix_query_request(queries, &self.connection).await?;

                            let entries: Vec<VirtualEntry> = mix_response
                                .files
                                .into_iter()
                                .map(|file| VirtualEntry {
                                    name: file.title,
                                    id: Some(file.id),
                                    is_directory: false,
                                })
                                .collect();

                            // Cache the entries
                            let collection_type = path_to_collection_type(&parent_path)
                                .ok_or_else(|| {
                                    anyhow!("Invalid collection type for path: {:?}", parent_path)
                                })?;

                            self.cache_entries(parent_path.clone(), entries, collection_type);
                        }

                        // Now try to find the file in the cache
                        if let Some(parent_cache) = self.cache.get(&parent_path)
                            && let Some(file_entry) =
                                parent_cache.entries.iter().find(|e| e.id == Some(id))
                        {
                            current = current.join(&file_entry.name);
                            continue;
                        }

                        return Err(anyhow!("File ID {} not found in parent directory", id));
                    }

                    let ctype = if let Some(ct) = collection_type {
                        ct
                    } else {
                        // If we're at root or collection type is unknown,
                        // try to determine from the first directory component
                        let root_dir = current
                            .components()
                            .nth(1)
                            .and_then(|c| c.as_os_str().to_str())
                            .ok_or_else(|| anyhow!("Cannot determine collection type"))?;

                        match root_dir {
                            "Albums" => CollectionType::Album,
                            "Artists" => CollectionType::Artist,
                            "Playlists" => CollectionType::Playlist,
                            "Mixes" => CollectionType::Mix,
                            "Tracks" => CollectionType::Track,
                            "Genres" => CollectionType::Genre,
                            _ => {
                                return Err(anyhow!("Invalid collection type: {}", root_dir));
                            }
                        }
                    };

                    if let Some((path, _)) = self.find_entry_by_id_and_type(id, ctype).await? {
                        // Important: Don't use the full path returned by find_entry_by_id_and_type
                        // Instead, preserve the current path's structure and only use the final component
                        let name = path
                            .file_name()
                            .ok_or_else(|| anyhow!("Invalid path structure"))?;
                        current = current.join(name);
                    } else {
                        return Err(anyhow!(
                            "ID {} not found in {} collection",
                            id,
                            ctype.as_str()
                        ));
                    }
                } else {
                    current = current.join(component_str);
                    // Update collection type after joining new component
                    collection_type = path_to_collection_type(&current);
                }
            }
        }

        Ok(current)
    }

    pub async fn path_to_query(&self, path: &Path) -> Result<Vec<(String, String)>> {
        let depth = path.components().count();
        log::debug!("path_to_query: path={:?}, depth={}", path, depth);

        match depth {
            2 => Ok(vec![("lib::directory.deep".to_string(), "/".to_string())]),
            3 => {
                // Prevent query generation for paths under /Tracks as they shouldn't exist
                if path.starts_with("/Tracks") {
                    // For /Tracks paths, we need to find the file in the cache and create a track query
                    let file_name = path
                        .file_name()
                        .ok_or_else(|| anyhow!("Invalid path: no file name"))?
                        .to_str()
                        .ok_or_else(|| anyhow!("Invalid file name encoding"))?;

                    // Since tracks are cached at the /Tracks directory level, we'll look there
                    let tracks_path = PathBuf::from("/Tracks");

                    if let Some(cache_entry) = self.cache.get(&tracks_path) {
                        // Find the track in the cache
                        if let Some(track) =
                            cache_entry.entries.iter().find(|e| e.name == file_name)
                            && let Some(file_id) = track.id
                        {
                            return Ok(vec![("lib::track".to_string(), file_id.to_string())]);
                        }

                        return Err(anyhow!("Track not found in cache"));
                    }

                    return Err(anyhow!("Tracks directory not cached"));
                }

                println!(
                    "{}",
                    "Unable to parse a collection group, fallback to the whole library".yellow()
                );
                Ok(vec![("lib::directory.deep".to_string(), "/".to_string())])
            }
            4 => {
                let collection_type =
                    path_to_collection_type(path).ok_or_else(|| {
                        anyhow!("Invalid path: {:?}", path)
                    })?;

                let parent_path = path.parent().unwrap().to_path_buf();
                let collection_name = path.file_name().unwrap().to_str().unwrap();

                let collection_id = if let Some(parent_cache) = self.cache.get(&parent_path) {
                    parent_cache
                        .entries
                        .iter()
                        .find(|e| e.name == collection_name)
                        .and_then(|e| e.id)
                        .ok_or_else(|| {
                            anyhow!("Collection not found in cache")
                        })?
                } else {
                    return Err(anyhow!("Parent directory not cached"));
                };

                build_query(collection_type, collection_id, &self.connection).await
            }
            5 => {
                // For files directly under /Tracks or at depth 5 in other paths
                let file_name = path
                    .file_name()
                    .ok_or_else(|| anyhow!("Invalid path: no file name"))?
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid file name encoding"))?;

                // Get the parent directory's cache to find the file ID
                let parent_path = path.parent().unwrap().to_path_buf();
                if let Some(parent_cache) = self.cache.get(&parent_path) {
                    // Find the file entry in the cache
                    if let Some(file_entry) =
                        parent_cache.entries.iter().find(|e| e.name == file_name)
                    {
                        // Get the file ID and construct the track query
                        if let Some(file_id) = file_entry.id {
                            return Ok(vec![("lib::track".to_string(), file_id.to_string())]);
                        }
                    }
                    return Err(anyhow!("File not found in cache"));
                }
                Err(anyhow!("Parent directory not cached"))
            }
            _ => Ok(vec![("lib::directory.deep".to_string(), "/".to_string())]),
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

        let collection_type =
            path_to_collection_type(&self.current_path).ok_or_else(|| anyhow!("Invalid path"))?;

        let entries = if self.current_path.components().count() == 2
            && self.current_path.ends_with("Tracks")
        {
            // Special handling for /Tracks directory - list files directly
            let query = vec![("lib::directory.deep".to_string(), "/".to_string())];
            let mix_response = send_mix_query_request(query, &self.connection).await?;

            Ok(mix_response
                .files
                .into_iter()
                .map(|file| VirtualEntry {
                    name: file.title,
                    id: Some(file.id),
                    is_directory: false,
                })
                .collect::<Vec<_>>())
        } else {
            match self.current_path.components().count() {
                // If we're at the root of a collection type (e.g., /Artists)
                2 => {
                    // Skip group listing for Tracks
                    if collection_type == CollectionType::Track {
                        Ok(Vec::new())
                    } else {
                        let response =
                            fetch_collection_group_summary(collection_type, &self.connection)
                                .await?;

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
                }
                // If we're in a group (e.g., /Artists/:Group)
                3 => {
                    let group_title = self
                        .current_path
                        .components()
                        .next_back()
                        .unwrap()
                        .as_os_str()
                        .to_str()
                        .unwrap()
                        .to_string();
                    let response = fetch_collection_groups(
                        collection_type,
                        vec![group_title],
                        &self.connection,
                    )
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
                    let queries = self.path_to_query(&self.current_path).await?;
                    let mix_response = send_mix_query_request(queries, &self.connection).await?;

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

        if let Some(collection_type) = path_to_collection_type(&self.current_path)
            && let Ok(ref entries) = entries
        {
            self.cache_entries(self.current_path.clone(), entries.clone(), collection_type);
        }

        entries
    }

    pub async fn verify_group_exists(
        &self,
        collection_type: CollectionType,
        group_name: &str,
    ) -> Result<bool> {
        let response = fetch_collection_group_summary(collection_type, &self.connection).await?;
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
        let response = fetch_collection_groups(
            collection_type,
            vec![group_name.to_string()],
            &self.connection,
        )
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
                    .next_back()
                    .unwrap()
                    .as_os_str()
                    .to_string_lossy()
                    .to_string(),
            )),
            // Second level directories (groups) must exist in the server
            3 => {
                // Prevent navigation into subdirectories under /Tracks
                if new_path.starts_with("/Tracks") {
                    return Ok(false);
                }

                let collection_type = path_to_collection_type(new_path)
                    .ok_or_else(|| {
                        anyhow!("Invalid collection type for path: {:?}", new_path)
                    })?;
                let group_name = new_path
                    .components()
                    .next_back()
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid group name"))?;
                self.verify_group_exists(collection_type, group_name).await
            }
            // Third level (individual collections) must exist in the server
            4 => {
                let collection_type = path_to_collection_type(new_path)
                    .ok_or_else(|| {
                        anyhow!("Invalid collection type for path: {:?}", new_path)
                    })?;
                let group_name = new_path
                    .components()
                    .nth(2)
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid group name"))?;
                let collection_name = new_path
                    .components()
                    .next_back()
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

trait AsStr {
    fn as_str(&self) -> &'static str;
}

impl AsStr for CollectionType {
    fn as_str(&self) -> &'static str {
        match self {
            CollectionType::Album => "Album",
            CollectionType::Artist => "Artist",
            CollectionType::Playlist => "Playlist",
            CollectionType::Mix => "Mix",
            CollectionType::Track => "Track",
            CollectionType::Genre => "Genre",
            CollectionType::Directory => "Directory",
        }
    }
}
