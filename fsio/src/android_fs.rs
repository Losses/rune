use std::{
    fs,
    io::Read,
    os::unix::prelude::AsRawFd,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use jni::objects::{JObject, JString, JValue};
use jni::JNIEnv;
use ndk_saf::{AndroidFile, AndroidFileOps};
use rusqlite::{params, Connection};

use super::android_jni::{checked_result, with_document_cursor, with_document_env, Operation};
use super::{DirWalk, FileIo, FileIoError, FileStream, FsNode, OpenMode, StorageLocation};
use std::time as std_time;

fn with_saf<T>(
    operation: Operation,
    call: impl FnOnce(&mut JNIEnv) -> anyhow::Result<T>,
) -> Result<T, FileIoError> {
    let env = ndk_saf::get_env()
        .map_err(|_| FileIoError::DocumentRuntime("cannot attach the native thread".into()))?;
    with_document_env(env, operation, call)
}

fn tree_document(uri: &str) -> Result<AndroidFile, FileIoError> {
    with_saf(Operation::Item("read document metadata"), |_| {
        ndk_saf::from_tree_url(uri)
    })
}

fn open_document(uri: &str, mode: &str) -> Result<fs::File, FileIoError> {
    with_saf(Operation::Item("open document"), |_| {
        ndk_saf::open_content_url(uri, mode)
    })
}

const DOCUMENTS_CONTRACT: &str = "android/provider/DocumentsContract";
const DOCUMENT_URI: &str = "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;";
const DOCUMENT_COLUMNS: [&str; 4] = ["document_id", "_display_name", "_size", "mime_type"];

#[derive(Clone, Copy)]
enum DocumentQuery {
    Root,
    Children,
}

fn resolver_and_uri<'local>(
    env: &mut JNIEnv<'local>,
    url: &str,
) -> anyhow::Result<(JObject<'local>, JObject<'local>)> {
    let application = env
        .call_static_method(
            "android/app/ActivityThread",
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        )?
        .l()?;
    let resolver = env
        .call_method(
            &application,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        )?
        .l()?;
    let url = env.new_string(url)?;
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[(&url).into()],
        )?
        .l()?;
    Ok((resolver, uri))
}

fn cursor_string<'local>(
    env: &mut JNIEnv<'local>,
    cursor: &JObject,
    column: i32,
) -> jni::errors::Result<JString<'local>> {
    Ok(env
        .call_method(
            cursor,
            "getString",
            "(I)Ljava/lang/String;",
            &[JValue::Int(column)],
        )?
        .l()?
        .into())
}

fn uri_string(env: &mut JNIEnv, uri: &JObject, method: &str) -> anyhow::Result<String> {
    let value = JString::from(
        env.call_method(uri, method, "()Ljava/lang/String;", &[])?
            .l()?,
    );
    let value: String = env.get_string(&value)?.into();
    Ok(value)
}

fn read_cursor(
    env: &mut JNIEnv,
    cursor: &JObject,
    tree: &JObject,
) -> anyhow::Result<Vec<AndroidFile>> {
    let mut columns = [0; 4];
    for (index, name) in DOCUMENT_COLUMNS.iter().enumerate() {
        let column = env.new_string(name)?;
        columns[index] = env
            .call_method(
                cursor,
                "getColumnIndexOrThrow",
                "(Ljava/lang/String;)I",
                &[(&column).into()],
            )?
            .i()?;
    }
    let mut files = Vec::new();
    while env.call_method(cursor, "moveToNext", "()Z", &[])?.z()? {
        let file = env.with_local_frame(16, |env| -> anyhow::Result<AndroidFile> {
            let document_id = cursor_string(env, cursor, columns[0])?;
            let name = cursor_string(env, cursor, columns[1])?;
            let filename: String = env.get_string(&name)?.into();
            let size = env
                .call_method(cursor, "getLong", "(I)J", &[JValue::Int(columns[2])])?
                .j()?
                .max(0) as usize;
            let mime = cursor_string(env, cursor, columns[3])?;
            let is_dir = env.get_string(&mime)?.to_bytes() == b"vnd.android.document/directory";
            let uri = env
                .call_static_method(
                    DOCUMENTS_CONTRACT,
                    "buildDocumentUriUsingTree",
                    DOCUMENT_URI,
                    &[tree.into(), (&document_id).into()],
                )?
                .l()?;
            Ok(AndroidFile {
                filename,
                size,
                path: uri_string(env, &uri, "getPath")?,
                url: uri_string(env, &uri, "toString")?,
                is_dir,
            })
        })?;
        files.push(file);
    }
    files.sort_by(|left, right| left.filename.cmp(&right.filename));
    Ok(files)
}

/// Use the provider's cursor directly. ndk-saf's list_files maps a null
/// cursor to an empty folder and does not expose partial results.
fn query_documents(url: &str, target: DocumentQuery) -> Result<Vec<AndroidFile>, FileIoError> {
    with_saf(Operation::List, |env| {
        let (resolver, tree) = resolver_and_uri(env, url)?;
        let (get_id, build_uri, operation) = match target {
            DocumentQuery::Root => (
                "getTreeDocumentId",
                "buildDocumentUriUsingTree",
                Operation::RootQuery,
            ),
            DocumentQuery::Children => (
                "getDocumentId",
                "buildChildDocumentsUriUsingTree",
                Operation::List,
            ),
        };
        let id = env
            .call_static_method(
                DOCUMENTS_CONTRACT,
                get_id,
                "(Landroid/net/Uri;)Ljava/lang/String;",
                &[(&tree).into()],
            )?
            .l()?;
        let uri = env
            .call_static_method(
                DOCUMENTS_CONTRACT,
                build_uri,
                DOCUMENT_URI,
                &[(&tree).into(), (&id).into()],
            )?
            .l()?;
        let projection = env.new_object_array(4, "java/lang/String", JObject::null())?;
        for (index, name) in DOCUMENT_COLUMNS.iter().enumerate() {
            let column = env.new_string(name)?;
            env.set_object_array_element(&projection, index as i32, column)?;
        }
        let null = JObject::null();
        let query = env.call_method(&resolver, "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[(&uri).into(), (&projection).into(), (&null).into(), (&null).into(), (&null).into()]).and_then(|v| v.l());
        let cursor = checked_result(env, operation, query.map_err(Into::into))?;
        Ok(with_document_cursor(
            env,
            &cursor,
            |env, cursor| {
                Ok(env
                    .call_method(cursor, "getExtras", "()Landroid/os/Bundle;", &[])?
                    .l()?)
            },
            |env, cursor| read_cursor(env, cursor, &tree),
        )?)
    })
}

fn root_document(url: &str) -> Result<AndroidFile, FileIoError> {
    let mut rows = query_documents(url, DocumentQuery::Root)?;
    match rows.len() {
        0 => Err(FileIoError::DocumentRootUnavailable(
            "root document is missing".into(),
        )),
        1 if rows[0].is_dir => Ok(rows.remove(0)),
        _ => Err(FileIoError::DocumentListingIncomplete(
            "expected one root directory".into(),
        )),
    }
}

/// A failed child query may be the first access after revocation. Only a
/// separate root query can turn that failure into a request to reselect it.
fn check_root_after_listing_error(root: &str, error: FileIoError) -> FileIoError {
    if matches!(error, FileIoError::DocumentListingIncomplete(_)) {
        if let Err(root_error) = root_document(root) {
            return root_error;
        }
    }
    error
}

/// Reads a UTF-8 document for which Android has granted access to this app.
pub fn read_android_document(uri: &str) -> Result<String, FileIoError> {
    let mut file = open_document(uri, "r")?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

pub(crate) struct AndroidFsIo {
    db: Arc<Mutex<Connection>>,
    root_uri: String,
}

impl AndroidFsIo {
    pub(crate) fn new(db_path: &Path, root_uri: &str) -> Result<Self, FileIoError> {
        let root_file = root_document(root_uri)?;

        let db_file = Self::find_file_by_path(root_uri, root_file, db_path, true)?;

        let std_file = with_saf(Operation::Item("open cache database"), |_| {
            db_file.open("rw")
        })?;
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
            let mut columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| FileIoError::Database(e.to_string()))?;
            columns.any(|column| column.is_ok_and(|name| name == "is_dir"))
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
        root: &str,
        start_file: AndroidFile,
        path: &Path,
        create_if_not_exist: bool,
    ) -> Result<AndroidFile, FileIoError> {
        let mut current_file = start_file;
        for component in path.components() {
            let component_name = match component {
                Component::Normal(name) => name.to_str().ok_or(FileIoError::InvalidPath)?,
                _ => continue,
            };

            let files = query_documents(&current_file.url, DocumentQuery::Children)
                .map_err(|error| check_root_after_listing_error(root, error))?;
            let found_file = files.into_iter().find(|f| f.filename == component_name);

            current_file = match found_file {
                Some(file) => file,
                None => {
                    if create_if_not_exist {
                        if let Some(_ext) = Path::new(component_name).extension() {
                            with_saf(Operation::Item("create document"), |_| {
                                current_file.create_file("application/octet-stream", component_name)
                            })?
                        } else {
                            with_saf(Operation::Item("create directory"), |_| {
                                current_file.create_directory(component_name)
                            })?
                        }
                    } else {
                        return Err(FileIoError::PathNotFound(path.display().to_string()));
                    }
                }
            };
        }
        Ok(current_file)
    }

    pub fn refresh_cache(&self) -> Result<(), FileIoError> {
        let root_file = root_document(&self.root_uri)?;
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
            let files = query_documents(&file.url, DocumentQuery::Children)?;
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
                Err(check_root_after_listing_error(&self.root_uri, e))
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

        self.resolve_uri(path)
    }

    fn resolve_uri(&self, path: &Path) -> Result<String, FileIoError> {
        let root_file = root_document(&self.root_uri)?;
        let file = Self::find_file_by_path(&self.root_uri, root_file, path, false)?;

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

    fn evict_cache(&self, path: &Path, stale_uri: &str) -> Result<(), FileIoError> {
        let conn = self.db.lock().unwrap();
        conn.execute(
            "DELETE FROM fs_cache WHERE path = ?1 AND content_url = ?2",
            params![path.to_str().ok_or(FileIoError::InvalidPath)?, stale_uri],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
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
    fn open_sync(&self, path: &Path, open_mode: OpenMode) -> Result<fs::File, FileIoError> {
        let uri = match self.get_uri(path) {
            Ok(uri) => uri,
            Err(FileIoError::PathNotFound(_)) if open_mode.creates() => {
                self.create_file_for_write(path)?
            }
            Err(e) => return Err(e),
        };
        match open_document(&uri, open_mode.as_str()) {
            Err(FileIoError::Saf(_)) => {
                self.evict_cache(path, &uri)?;
                // Bypass the cache for this one retry. Only a complete live
                // enumeration of the parent can establish that the file is gone.
                let uri = self.resolve_uri(path)?;
                open_document(&uri, open_mode.as_str())
            }
            result => result,
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
        let new_file = with_saf(Operation::Item("create document"), |_| {
            parent_file.create_file("application/octet-stream", name)
        })?;

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
        let new_file = with_saf(Operation::Item("create directory"), |_| {
            parent_file.create_directory(name)
        })?;
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
        tree_document(&uri)
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

    fn storage(&self) -> StorageLocation {
        StorageLocation::Local
    }

    fn open(&self, path: &Path, open_mode: OpenMode) -> Result<Box<dyn FileStream>, FileIoError> {
        Ok(Box::new(self.open_sync(path, open_mode)?))
    }

    fn content_uri(&self, path: &Path) -> Result<Option<String>, FileIoError> {
        self.get_uri(path).map(Some)
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: OpenMode,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        Ok(Box::new(self.open_sync(path, open_mode)?))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        let mut file = self.open(path, OpenMode::Read)?;
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
        let mut file = self.open_async(path, OpenMode::Write).await?;
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
        with_saf(Operation::Item("remove document"), |_| file.remove_file())?;

        let conn = self.db.lock().unwrap();
        conn.execute(
            "DELETE FROM fs_cache WHERE path = ?1",
            params![path.to_str().unwrap()],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    async fn rename_file(&self, from: &Path, to: &Path) -> Result<(), FileIoError> {
        if from.parent() != to.parent() || from == to {
            return Err(FileIoError::InvalidPath);
        }
        let file = self.get_android_file(from)?;
        if file.is_dir || self.exists(to)? {
            return Err(FileIoError::InvalidPath);
        }
        let name = to
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(FileIoError::InvalidPath)?;
        let uri = with_saf(Operation::Item("rename document"), |env| {
            let (resolver, uri) = resolver_and_uri(env, &file.url)?;
            let name = env.new_string(name)?;
            let renamed = env.call_static_method(
                "android/provider/DocumentsContract", "renameDocument",
                "(Landroid/content/ContentResolver;Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&resolver), JValue::Object(&uri), JValue::Object(&name)],
            )?.l()?;
            anyhow::ensure!(!renamed.is_null(), "document provider refused rename");
            uri_string(env, &renamed, "toString")
        })?;
        let mut conn = self.db.lock().unwrap();
        let tx = conn
            .transaction()
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        tx.execute(
            "DELETE FROM fs_cache WHERE path=?1",
            params![from.to_str().ok_or(FileIoError::InvalidPath)?],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        tx.execute(
            "INSERT OR REPLACE INTO fs_cache(path,content_url,parent,filename,is_dir,size) VALUES(?1,?2,?3,?4,0,?5)",
            params![to.to_str().ok_or(FileIoError::InvalidPath)?, uri, to.parent().and_then(|p| p.to_str()).ok_or(FileIoError::InvalidPath)?, name, file.size as i64],
        ).map_err(|e| FileIoError::Database(e.to_string()))?;
        tx.commit()
            .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        let file = self.get_android_file(path)?;
        with_saf(Operation::Item("remove document"), |_| file.remove_file())?;

        let conn = self.db.lock().unwrap();
        let path_str = path.to_str().unwrap();
        conn.execute(
            "DELETE FROM fs_cache WHERE path = ?1 OR path LIKE ?2",
            params![path_str, format!("{path_str}/%")],
        )
        .map_err(|e| FileIoError::Database(e.to_string()))?;
        Ok(())
    }

    fn walk_dir(&self, path: &Path, _follow_links: bool) -> Result<DirWalk, FileIoError> {
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
        // The cache stores names as text, so every entry has a node.
        Ok(DirWalk {
            nodes,
            skipped: Vec::new(),
        })
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        self.entry_exists(path)
    }

    fn entry_exists(&self, path: &Path) -> Result<bool, FileIoError> {
        match self.get_uri(path) {
            Ok(_) => Ok(true),
            Err(FileIoError::PathNotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
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
        let std_file = self.open_sync(path, OpenMode::Read)?;
        let fd = std_file.as_raw_fd();
        let proc_path = format!("/proc/self/fd/{}", fd);
        let real_path = fs::read_link(proc_path).map_err(FileIoError::Io)?;
        Ok(real_path)
    }

    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError> {
        if path.contains(':') {
            let std_file = open_document(path, "r")?;
            let fd = std_file.as_raw_fd();
            let proc_path = format!("/proc/self/fd/{}", fd);
            let real_path = fs::read_link(proc_path).map_err(FileIoError::Io)?;
            Ok(real_path)
        } else {
            self.canonicalize_path(Path::new(path))
        }
    }

    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError> {
        let real_path = self.canonicalize_path(path)?;
        let (filename, is_dir, size) = self.get_cached_meta(path)?;
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
        let file = self.open_sync(path, OpenMode::Read)?;
        let modified = file
            .metadata()?
            .modified()?
            .duration_since(std_time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(modified.as_secs())
    }

    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        if path.contains(':') {
            let file = tree_document(path)?;
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
