use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use sea_orm::{DatabaseConnection, EntityTrait, QuerySelect};

use crate::entities::media_files;

#[derive(Debug)]
pub struct DirectoryTreeMap {
    pub name: String,
    pub path: String,
    pub children: HashMap<String, DirectoryTreeMap>,
}

#[derive(Debug)]
pub struct DirectoryTree {
    pub name: String,
    pub path: String,
    pub children: Vec<DirectoryTree>,
}

impl DirectoryTreeMap {
    fn new(name: &str, path: &str) -> Self {
        DirectoryTreeMap {
            name: name.to_string(),
            path: path.to_string(),
            children: HashMap::new(),
        }
    }

    fn add_path(&mut self, path_str: &str) {
        let path = Path::new(&path_str);
        if let Some((first, rest)) = path
            .iter()
            .map(|s| s.to_str().unwrap())
            .collect::<Vec<_>>()
            .split_first()
        {
            let child_path = format!("{}/{}", self.path, first);
            let child = self
                .children
                .entry(first.to_string())
                .or_insert_with(|| DirectoryTreeMap::new(first, &child_path));
            if !rest.is_empty() {
                child.add_path(&rest.join("/"));
            }
        }
    }

    fn into(self) -> DirectoryTree {
        let children = self
            .children
            .into_values()
            .map(|node| node.into())
            .collect();
        DirectoryTree {
            name: self.name,
            path: self.path,
            children,
        }
    }
}

pub fn build_tree(paths: Vec<String>) -> DirectoryTree {
    let mut root = DirectoryTreeMap::new("/", "");
    for path in paths {
        root.add_path(&path);
    }
    root.into()
}

pub async fn get_directory_tree(db: &DatabaseConnection) -> Result<DirectoryTree> {
    {
        // Query distinct media directories
        let unique_directories: Vec<String> = media_files::Entity::find()
            .select_only()
            .column(media_files::Column::Directory)
            .distinct()
            .into_tuple::<String>()
            .all(db)
            .await?;

        Ok(build_tree(unique_directories))
    }
}
