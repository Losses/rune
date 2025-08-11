
use super::{FileIo, FileIoError, FsNode};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rusqlite::{Connection, params};
use ndk_saf::{AndroidFile, AndroidFileOps, from_tree_url};
use tokio::task;

pub(crate) struct AndroidFsIo {
    db: Arc<Mutex<Connection>>,
    root_uri: String,
}

impl AndroidFsIo {
    pub(crate) async fn new(db_path: &Path, root_uri: &str) -> Result<Self, FileIoError> {
        let db = Connection::open(db_path).map_err(|e| FileIoError::Database(e.to_string()))?;
        db.execute(
            "CREATE TABLE IF NOT EXISTS fs_cache (
                path TEXT PRIMARY KEY,
                content_url TEXT NOT NULL,
                parent TEXT NOT NULL
            )",
            [],
        ).map_err(|e| FileIoError::Database(e.to_string()))?;

        let instance = Self {
            db: Arc::new(Mutex::new(db)),
            root_uri: root_uri.to_string(),
        };

        instance.refresh_cache().await?;

        Ok(instance)
    }

    pub async fn refresh_cache(&self) -> Result<(), FileIoError> {
        let root_file = from_tree_url(&self.root_uri).map_err(|e| FileIoError::Saf(e.to_string()))?;
        let db = self.db.clone();
        
        task::spawn_blocking(move || {
            let mut conn = db.lock().unwrap();
            let tx = conn.transaction().unwrap();
            tx.execute("DELETE FROM fs_cache", [])?;

            fn walk(file: AndroidFile, current_path: &Path, tx: &rusqlite::Transaction) -> Result<(), FileIoError> {
                let files = file.list_files().map_err(|e| FileIoError::Saf(e.to_string()))?;
                for f in files {
                    let new_path = current_path.join(&f.filename);
                    tx.execute(
                        "INSERT OR REPLACE INTO fs_cache (path, content_url, parent) VALUES (?1, ?2, ?3)",
                        params![new_path.to_str().unwrap(), f.url, current_path.to_str().unwrap()],
                    ).map_err(|e| FileIoError::Database(e.to_string()))?;

                    if f.is_dir {
                        walk(f, &new_path, tx)?;
                    }
                }
                Ok(())
            }

            walk(root_file, Path::new(""), &tx)?;
            tx.commit().map_err(|e| FileIoError::Database(e.to_string()))
        }).await.unwrap()
    }

    fn get_uri(&self, path: &Path) -> Result<String, FileIoError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT content_url FROM fs_cache WHERE path = ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let mut rows = stmt.query(params![path.to_str().unwrap()])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| FileIoError::Database(e.to_string()))? {
            row.get(0).map_err(|e| FileIoError::Database(e.to_string()))
        } else {
            Err(FileIoError::PathNotFound(path.to_string_lossy().to_string()))
        }
    }

    fn get_android_file(&self, path: &Path) -> Result<AndroidFile, FileIoError> {
        let uri = self.get_uri(path)?;
        from_tree_url(&uri).map_err(|e| FileIoError::Saf(e.to_string()))
    }
}

#[async_trait]
impl FileIo for AndroidFsIo {
    async fn open(&self, path: &Path, open_mode: &str) -> Result<std::fs::File, FileIoError> {
        let file = self.get_android_file(path)?;
        file.open(open_mode).map_err(|e| FileIoError::Saf(e.to_string()))
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        use std::io::Read;
        let mut file = self.open(path, "r").await?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError> {
        use std::io::Write;
        let mut file = self.open(path, "w").await?;
        file.write_all(contents)?;
        Ok(())
    }

    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        let parent_file = self.get_android_file(parent)?;
        let new_file = parent_file.create_directory(name).map_err(|e| FileIoError::Saf(e.to_string()))?;
        let new_path = parent.join(name);
        
        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO fs_cache (path, content_url, parent) VALUES (?1, ?2, ?3)",
            params![new_path.to_str().unwrap(), new_file.url, parent.to_str().unwrap()],
        ).map_err(|e| FileIoError::Database(e.to_string()))?;

        Ok(new_path)
    }

    async fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        let mut current_path = PathBuf::new();
        for component in path.components() {
            let component_str = component.as_os_str().to_str().unwrap();
            let next_path = current_path.join(component_str);
            if !self.exists(&next_path).await? {
                self.create_dir(&current_path, component_str).await?;
            }
            current_path = next_path;
        }
        Ok(())
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path FROM fs_cache WHERE parent = ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let mut rows = stmt.query(params![path.to_str().unwrap()])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        let mut nodes = Vec::new();
        while let Some(row) = rows.next().map_err(|e| FileIoError::Database(e.to_string()))? {
            let path_str: String = row.get(0)?;
            let path = PathBuf::from(path_str);
            let file = self.get_android_file(&path)?;
            nodes.push(FsNode {
                path,
                is_dir: file.is_dir,
                is_file: !file.is_dir,
                size: file.size as u64,
            });
        }
        Ok(nodes)
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError> {
        let file = self.get_android_file(path)?;
        file.remove_file().map_err(|e| FileIoError::Saf(e.to_string()))?;
        
        let conn = self.db.lock().unwrap();
        conn.execute("DELETE FROM fs_cache WHERE path = ?1", params![path.to_str().unwrap()])
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        let file = self.get_android_file(path)?;
        file.remove_file().map_err(|e| FileIoError::Saf(e.to_string()))?;

        let conn = self.db.lock().unwrap();
        conn.execute("DELETE FROM fs_cache WHERE path LIKE ?1", params![format!("{}%", path.to_str().unwrap())])
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    async fn walk_dir(&self, path: &Path, _follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path FROM fs_cache WHERE path LIKE ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let mut rows = stmt.query(params![format!("{}%", path.to_str().unwrap())])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        let mut nodes = Vec::new();
        while let Some(row) = rows.next().map_err(|e| FileIoError::Database(e.to_string()))? {
            let path_str: String = row.get(0)?;
            let path = PathBuf::from(path_str);
            let file = self.get_android_file(&path)?;
            nodes.push(FsNode {
                path,
                is_dir: file.is_dir,
                is_file: !file.is_dir,
                size: file.size as u64,
            });
        }
        Ok(nodes)
    }

    async fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        Ok(self.get_uri(path).is_ok())
    }

    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError> {
        let file = self.get_android_file(path)?;
        Ok(!file.is_dir)
    }

    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError> {
        let file = self.get_android_file(path)?;
        Ok(file.is_dir)
    }
}
