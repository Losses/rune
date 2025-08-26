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
        db.execute(
            "CREATE TABLE IF NOT EXISTS fs_cache (
                path TEXT PRIMARY KEY,
                content_url TEXT NOT NULL,
                parent TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;

        let instance = Self {
            db: Arc::new(Mutex::new(db)),
            root_uri: root_uri.to_string(),
        };

        instance.refresh_cache()?;

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
                    "INSERT OR REPLACE INTO fs_cache (path, content_url, parent) VALUES (?1, ?2, ?3)",
                    params![
                        new_path.to_str().unwrap(),
                        f.url,
                        current_path.to_str().unwrap()
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
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT content_url FROM fs_cache WHERE path = ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let mut rows = stmt
            .query(params![path.to_str().unwrap()])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| FileIoError::Database(e.to_string()))?
        {
            row.get(0).map_err(|e| FileIoError::Database(e.to_string()))
        } else {
            Err(FileIoError::PathNotFound(
                path.to_string_lossy().to_string(),
            ))
        }
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

    fn open(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        // block on the async open
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(self.open_async(path, open_mode))
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: &str,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        let file = self.get_android_file(path)?;
        let android_file = file
            .open(open_mode)
            .map_err(|e| FileIoError::Saf(e.to_string()))?;
        Ok(Box::new(android_file))
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
        let parent_file = self.get_android_file(parent)?;
        let new_file = parent_file
            .create_directory(name)
            .map_err(|e| FileIoError::Saf(e.to_string()))?;
        let new_path = parent.join(name);

        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO fs_cache (path, content_url, parent) VALUES (?1, ?2, ?3)",
            params![
                new_path.to_str().unwrap(),
                new_file.url,
                parent.to_str().unwrap()
            ],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;

        Ok(new_path)
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
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(self.create_dir(parent, name))?;

        Ok(())
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path FROM fs_cache WHERE parent = ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let mut rows = stmt
            .query(params![path.to_str().unwrap()])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        let mut nodes = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| FileIoError::Database(e.to_string()))?
        {
            let path_str: String = row
                .get(0)
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let path = PathBuf::from(path_str);
            let file = self.get_android_file(&path)?;
            nodes.push(FsNode {
                filename: file.filename,
                raw_path: path.to_str().unwrap_or_default().to_string(),
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
        conn.execute(
            "DELETE FROM fs_cache WHERE path LIKE ?1",
            params![format!("{}%", path.to_str().unwrap())],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    fn walk_dir(&self, path: &Path, _follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path FROM fs_cache WHERE path LIKE ?1")
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        let mut rows = stmt
            .query(params![format!("{}%", path.to_str().unwrap())])
            .map_err(|e| FileIoError::Database(e.to_string()))?;

        let mut nodes = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| FileIoError::Database(e.to_string()))?
        {
            let path_str: String = row
                .get(0)
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            let path = PathBuf::from(path_str);
            let file = self.get_android_file(&path)?;
            nodes.push(FsNode {
                filename: file.filename,
                raw_path: path.to_str().unwrap_or_default().to_string(),
                path,
                is_dir: file.is_dir,
                is_file: !file.is_dir,
                size: file.size as u64,
            });
        }
        Ok(nodes)
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
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

    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        let file = self.get_android_file(path)?;
        let std_file = file
            .open("r")
            .map_err(|e| FileIoError::Saf(e.to_string()))?;
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
        let file = self.get_android_file(path)?;
        let path = self.canonicalize_path(path)?;
        Ok(FsNode {
            filename: file.filename,
            raw_path: path.to_str().unwrap_or_default().to_string(),
            path,
            is_dir: file.is_dir,
            is_file: !file.is_dir,
            size: file.size as u64,
        })
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
