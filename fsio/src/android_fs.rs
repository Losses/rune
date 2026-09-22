use std::{
    fs,
    os::unix::prelude::AsRawFd,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ndk_saf::{from_tree_url, open_content_url, AndroidFile, AndroidFileOps};
use rusqlite::{params, Connection};

use super::{FileIo, FileIoError, FileStream, FsNode};

pub(crate) struct AndroidFsIo {
    db: Arc<Mutex<Connection>>,
    root_uri: String,
}

impl AndroidFsIo {
    pub(crate) fn new(db_path: &Path, root_uri: &str) -> Result<Self, FileIoError> {
        let root_file = from_tree_url(root_uri).map_err(|e| FileIoError::Saf(e.to_string()))?;

        let db_file = Self::find_file_by_path(root_file, db_path, true)
            .map_err(|e| FileIoError::Saf(e.to_string()))?;

        let std_file = db_file
            .open("rw")
            .map_err(|e| FileIoError::Saf(e.to_string()))?;
        let fd = std_file.as_raw_fd();
        let path = PathBuf::from(format!("/proc/self/fd/{}", fd));

        let db = Connection::open(path).map_err(|e| FileIoError::Database(e.to_string()))?;

        // Schema v2 caches entry metadata (filename/is_dir/size) so listings
        // don't need a provider IPC per entry. Old caches lack these columns;
        // the cache is disposable, so drop and rebuild.
        let has_v2_schema = {
            let mut stmt = db
                .prepare("PRAGMA table_info(fs_cache)")
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let has_is_dir = columns.filter_map(|c| c.ok()).any(|name| name == "is_dir");
            has_is_dir
        };
        if !has_v2_schema {
            db.execute("DROP TABLE IF EXISTS fs_cache", [])
                .map_err(|e| FileIoError::Database(e.to_string()))?;
        }
        db.execute(
            "CREATE TABLE IF NOT EXISTS fs_cache (
                path TEXT PRIMARY KEY,
                content_url TEXT NOT NULL,
                parent TEXT NOT NULL,
                filename TEXT NOT NULL,
                is_dir INTEGER NOT NULL,
                size INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;

        let instance = Self {
            db: Arc::new(Mutex::new(db)),
            root_uri: root_uri.to_string(),
        };

        let cache_empty = {
            let conn = instance.db.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM fs_cache", [], |r| r.get::<_, i64>(0))
                .map(|count| count == 0)
                .unwrap_or(true)
        };
        if cache_empty {
            instance.refresh_cache()?;
        } else {
            log::info!("fs_cache is populated, skipping full refresh on startup");
        }

        Ok(instance)
    }

    fn find_file_by_path(
        start_file: AndroidFile,
        path: &Path,
        create_if_not_exist: bool,
    ) -> Result<AndroidFile, String> {
        let mut current_file = start_file;
        for component in path.components() {
            let component_name = match component {
                Component::Normal(name) => name.to_str().ok_or("Invalid path component")?,
                _ => continue,
            };

            let files = current_file.list_files().map_err(|e| e.to_string())?;
            let found_file = files.into_iter().find(|f| f.filename == component_name);

            current_file = match found_file {
                Some(file) => file,
                None => {
                    if create_if_not_exist {
                        if let Some(_ext) = Path::new(component_name).extension() {
                            current_file
                                .create_file("application/octet-stream", component_name)
                                .map_err(|e| e.to_string())?
                        } else {
                            current_file
                                .create_directory(component_name)
                                .map_err(|e| e.to_string())?
                        }
                    } else {
                        return Err(format!("File not found: {}", component_name));
                    }
                }
            };
        }
        Ok(current_file)
    }

    pub fn refresh_cache(&self) -> Result<(), FileIoError> {
        let root_file =
            from_tree_url(&self.root_uri).map_err(|e| FileIoError::Saf(e.to_string()))?;
        let db = self.db.clone();

        let mut conn = db.lock().unwrap();
        let tx = conn
            .transaction()
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        tx.execute("DELETE FROM fs_cache", [])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        fn walk(
            file: AndroidFile,
            current_path: &Path,
            tx: &rusqlite::Transaction,
        ) -> Result<(), FileIoError> {
            let files = file
                .list_files()
                .map_err(|e| FileIoError::Saf(e.to_string()))?;
            for f in files {
                let new_path = current_path.join(&f.filename);
                tx.execute(
                    "INSERT OR REPLACE INTO fs_cache (path, content_url, parent, filename, is_dir, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        new_path.to_str().unwrap(),
                        f.url,
                        current_path.to_str().unwrap(),
                        f.filename,
                        f.is_dir,
                        f.size as i64,
                    ],
                )
                .map_err(|e| FileIoError::Database(e.to_string()))?;

                if f.is_dir {
                    walk(f, &new_path, tx)?;
                }
            }
            Ok(())
        }

        match walk(root_file, Path::new(""), &tx) {
            Ok(_) => tx
                .commit()
                .map_err(|e| FileIoError::Database(e.to_string())),
            Err(e) => {
                tx.rollback()
                    .map_err(|e| FileIoError::Database(e.to_string()))?;
                Err(e)
            }
        }
    }

    fn get_uri(&self, path: &Path) -> Result<String, FileIoError> {
        let cached = {
            let conn = self.db.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT content_url FROM fs_cache WHERE path = ?1")
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let mut rows = stmt
                .query(params![path.to_str().unwrap()])
                .map_err(|e| FileIoError::Database(e.to_string()))?;

            match rows
                .next()
                .map_err(|e| FileIoError::Database(e.to_string()))?
            {
                Some(row) => Some(
                    row.get::<_, String>(0)
                        .map_err(|e| FileIoError::Database(e.to_string()))?,
                ),
                None => None,
            }
        };

        if let Some(url) = cached {
            return Ok(url);
        }

        // Cache miss: resolve live by walking the SAF tree (the same way other
        // platforms hit the real filesystem), then cache just this entry.
        let root_file =
            from_tree_url(&self.root_uri).map_err(|e| FileIoError::Saf(e.to_string()))?;
        let file = Self::find_file_by_path(root_file, path, false)
            .map_err(|_| FileIoError::PathNotFound(path.to_string_lossy().to_string()))?;

        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO fs_cache (path, content_url, parent, filename, is_dir, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                path.to_str().unwrap(),
                file.url,
                parent.to_str().unwrap(),
                file.filename,
                file.is_dir,
                file.size as i64,
            ],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;

        Ok(file.url)
    }

    fn evict_cache(&self, path: &Path) {
        let conn = self.db.lock().unwrap();
        if let Err(e) = conn.execute(
            "DELETE FROM fs_cache WHERE path = ?1",
            params![path.to_str().unwrap()],
        ) {
            log::warn!("Failed to evict stale cache entry for {path:?}: {e}");
        }
    }

    /// Read cached metadata for `path`; the read-through `get_uri` guarantees
    /// the row exists (resolving live and caching it on miss).
    fn get_cached_meta(&self, path: &Path) -> Result<(String, bool, u64), FileIoError> {
        self.get_uri(path)?;
        let conn = self.db.lock().unwrap();
        conn.query_row(
            "SELECT filename, is_dir, size FROM fs_cache WHERE path = ?1",
            params![path.to_str().unwrap()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, i64>(2)? as u64,
                ))
            },
        )
        .map_err(|e| FileIoError::Database(e.to_string()))
    }

    /// Synchronous open shared by `open` and `open_async`. Building a fresh
    /// tokio runtime and block_on-ing here would panic when the caller is
    /// already on a runtime thread; everything below is synchronous JNI.
    fn open_sync(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        // Skip DocumentFile construction entirely: opening only needs the
        // content URI, so this is a single provider IPC.
        let creates = open_mode.contains('w') || open_mode.contains('a') || open_mode.contains('t');
        let uri = match self.get_uri(path) {
            Ok(uri) => uri,
            Err(FileIoError::PathNotFound(_)) if creates => self.create_file_for_write(path)?,
            Err(e) => return Err(e),
        };
        match open_content_url(&uri, open_mode) {
            Ok(file) => Ok(Box::new(file)),
            Err(first_error) => {
                // The cached URI may be stale (the file was replaced outside the
                // app); evict it and re-resolve live once before giving up.
                self.evict_cache(path);
                let uri = self.get_uri(path)?;
                let file = open_content_url(&uri, open_mode).map_err(|second_error| {
                    FileIoError::Saf(format!(
                        "{second_error} (initial attempt with cached URI: {first_error})"
                    ))
                })?;
                Ok(Box::new(file))
            }
        }
    }

    /// SAF can only create files through the parent directory's DocumentFile;
    /// mirror std semantics where write-style open modes create missing files.
    fn create_file_for_write(&self, path: &Path) -> Result<String, FileIoError> {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if !parent.as_os_str().is_empty() {
            self.create_dir_all(parent)?;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(FileIoError::InvalidPath)?;

        let parent_file = self.get_android_file(parent)?;
        let new_file = parent_file
            .create_file("application/octet-stream", name)
            .map_err(|e| FileIoError::Saf(e.to_string()))?;

        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO fs_cache (path, content_url, parent, filename, is_dir, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                path.to_str().unwrap(),
                new_file.url,
                parent.to_str().unwrap(),
                new_file.filename,
                new_file.is_dir,
                new_file.size as i64,
            ],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;

        Ok(new_file.url)
    }

    fn create_dir_sync(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        let parent_file = self.get_android_file(parent)?;
        let new_file = parent_file
            .create_directory(name)
            .map_err(|e| FileIoError::Saf(e.to_string()))?;
        let new_path = parent.join(name);

        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO fs_cache (path, content_url, parent, filename, is_dir, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                new_path.to_str().unwrap(),
                new_file.url,
                parent.to_str().unwrap(),
                new_file.filename,
                new_file.is_dir,
                new_file.size as i64,
            ],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;

        Ok(new_path)
    }

    fn get_android_file(&self, path: &Path) -> Result<AndroidFile, FileIoError> {
        let uri = self.get_uri(path)?;
        from_tree_url(&uri).map_err(|e| FileIoError::Saf(e.to_string()))
    }
}

#[async_trait]
impl FileIo for AndroidFsIo {
    fn name(&self) -> &'static str {
        "Android"
    }

    fn refresh_cache(&self) -> Result<(), FileIoError> {
        AndroidFsIo::refresh_cache(self)
    }

    fn open(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        self.open_sync(path, open_mode)
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: &str,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        self.open_sync(path, open_mode)
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        use std::io::Read;
        let mut file = self.open(path, "r")?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileIoError> {
        let content = self.read(path)?;
        String::from_utf8(content).map_err(|_| FileIoError::InvalidPath)
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError> {
        use std::io::Write;
        let mut file = self.open_async(path, "w").await?;
        file.write_all(contents)?;
        Ok(())
    }

    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), FileIoError> {
        self.write(path, contents.as_bytes()).await
    }

    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        self.create_dir_sync(parent, name)
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        if self.exists(path)? {
            return Ok(());
        }

        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if !parent.as_os_str().is_empty() && !self.exists(parent)? {
            self.create_dir_all(parent)?;
        }

        let name = path.file_name().unwrap().to_str().unwrap();
        self.create_dir_sync(parent, name)?;

        Ok(())
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path, filename, is_dir, size FROM fs_cache WHERE parent = ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![path.to_str().unwrap()], |row| {
                let path_str: String = row.get(0)?;
                let is_dir: bool = row.get(2)?;
                Ok(FsNode {
                    filename: row.get(1)?,
                    raw_path: path_str.clone(),
                    path: PathBuf::from(path_str),
                    is_dir,
                    is_file: !is_dir,
                    size: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        let mut nodes = Vec::new();
        for node in rows {
            nodes.push(node.map_err(|e| FileIoError::Database(e.to_string()))?);
        }
        Ok(nodes)
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError> {
        let file = self.get_android_file(path)?;
        file.remove_file()
            .map_err(|e| FileIoError::Saf(e.to_string()))?;

        let conn = self.db.lock().unwrap();
        conn.execute(
            "DELETE FROM fs_cache WHERE path = ?1",
            params![path.to_str().unwrap()],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        let file = self.get_android_file(path)?;
        file.remove_file()
            .map_err(|e| FileIoError::Saf(e.to_string()))?;

        let conn = self.db.lock().unwrap();
        let path_str = path.to_str().unwrap();
        conn.execute(
            "DELETE FROM fs_cache WHERE path = ?1 OR path LIKE ?2",
            params![path_str, format!("{path_str}/%")],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    fn walk_dir(&self, path: &Path, _follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        let path_str = path.to_str().unwrap();
        let conn = self.db.lock().unwrap();

        let mut nodes = Vec::new();
        let mut collect = |row: &rusqlite::Row| -> rusqlite::Result<FsNode> {
            let path_str: String = row.get(0)?;
            let is_dir: bool = row.get(2)?;
            Ok(FsNode {
                filename: row.get(1)?,
                raw_path: path_str.clone(),
                path: PathBuf::from(path_str),
                is_dir,
                is_file: !is_dir,
                size: row.get::<_, i64>(3)? as u64,
            })
        };

        if path_str.is_empty() {
            let mut stmt = conn
                .prepare("SELECT path, filename, is_dir, size FROM fs_cache")
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let rows = stmt
                .query_map([], &mut collect)
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            for node in rows {
                nodes.push(node.map_err(|e| FileIoError::Database(e.to_string()))?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT path, filename, is_dir, size FROM fs_cache WHERE path = ?1 OR path LIKE ?2",
                )
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let rows = stmt
                .query_map(params![path_str, format!("{path_str}/%")], &mut collect)
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            for node in rows {
                nodes.push(node.map_err(|e| FileIoError::Database(e.to_string()))?);
            }
        }
        Ok(nodes)
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        Ok(self.get_uri(path).is_ok())
    }

    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError> {
        let (_, is_dir, _) = self.get_cached_meta(path)?;
        Ok(!is_dir)
    }

    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError> {
        let (_, is_dir, _) = self.get_cached_meta(path)?;
        Ok(is_dir)
    }

    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        let uri = self.get_uri(path)?;
        let std_file = open_content_url(&uri, "r").map_err(|e| FileIoError::Saf(e.to_string()))?;
        let fd = std_file.as_raw_fd();
        let proc_path = format!("/proc/self/fd/{}", fd);
        let real_path = fs::read_link(proc_path).map_err(FileIoError::Io)?;
        Ok(real_path)
    }

    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError> {
        if path.contains(':') {
            let std_file =
                open_content_url(path, "r").map_err(|e| FileIoError::Saf(e.to_string()))?;
            let fd = std_file.as_raw_fd();
            let proc_path = format!("/proc/self/fd/{}", fd);
            let real_path = fs::read_link(proc_path).map_err(FileIoError::Io)?;
            Ok(real_path)
        } else {
            self.canonicalize_path(Path::new(path))
        }
    }

    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError> {
        let (filename, is_dir, size) = self.get_cached_meta(path)?;
        let real_path = self.canonicalize_path(path)?;
        Ok(FsNode {
            filename,
            raw_path: real_path.to_str().unwrap_or_default().to_string(),
            path: real_path,
            is_dir,
            is_file: !is_dir,
            size,
        })
    }

    fn modified_time(&self, path: &Path) -> Result<u64, FileIoError> {
        // fstat on the SAF fd goes through FUSE getattr and carries real mtime
        let uri = self.get_uri(path)?;
        let file = open_content_url(&uri, "r").map_err(|e| FileIoError::Saf(e.to_string()))?;
        let modified = file
            .metadata()?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(modified.as_secs())
    }

    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        if path.contains(':') {
            let file = from_tree_url(path).map_err(|e| FileIoError::Saf(e.to_string()))?;
            let canon_path = self.canonicalize_path_str(path)?;
            Ok(FsNode {
                filename: file.filename,
                raw_path: canon_path.to_str().unwrap_or_default().to_string(),
                path: canon_path,
                is_dir: file.is_dir,
                is_file: !file.is_dir,
                size: file.size as u64,
            })
        } else {
            self.canonicalize(Path::new(path))
        }
    }

    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError> {
        if !self.exists(path)? {
            if let Some(parent) = path.parent() {
                self.ensure_directory(parent).await?;
            }
            self.write(path, &[]).await?;
        }
        self.canonicalize(path)
    }

    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError> {
        if !self.exists(path)? {
            self.create_dir_all(path)?;
        }
        self.canonicalize(path)
    }
}
