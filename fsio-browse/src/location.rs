use std::env;
use std::path::{Component, Path, PathBuf};

use fsio::FsIo;

use crate::listing::BrowseError;

/// The storage a [`Location`] lives in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Root {
    /// The device's own file system, addressed by absolute paths.
    FileSystem,
    /// A folder the user granted through Android's document picker, addressed
    /// by paths relative to it.
    DocumentTree { uri: String },
}

/// A folder or file a file dialog can show.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub root: Root,
    pub path: PathBuf,
}

impl Location {
    pub fn file_system(path: impl Into<PathBuf>) -> Self {
        Self {
            root: Root::FileSystem,
            path: path.into(),
        }
    }
}

impl Root {
    /// The backend that reads this root.
    pub fn open(&self) -> Result<FsIo, BrowseError> {
        match self {
            Root::FileSystem => file_system(),
            Root::DocumentTree { uri } => document_tree(uri),
        }
    }
}

#[cfg(not(target_os = "android"))]
fn file_system() -> Result<FsIo, BrowseError> {
    Ok(FsIo::new())
}

/// A path that is not a content URI gives the standard file system.
#[cfg(target_os = "android")]
fn file_system() -> Result<FsIo, BrowseError> {
    FsIo::new(Path::new(ANDROID_CACHE_DB), "").map_err(BrowseError::from)
}

/// Where Rune keeps its cache of a granted tree, the same file a library in
/// that tree uses.
#[cfg(target_os = "android")]
const ANDROID_CACHE_DB: &str = ".rune/.android-fs.db";

#[cfg(target_os = "android")]
fn document_tree(uri: &str) -> Result<FsIo, BrowseError> {
    FsIo::new(Path::new(ANDROID_CACHE_DB), uri).map_err(BrowseError::from)
}

#[cfg(not(target_os = "android"))]
fn document_tree(_uri: &str) -> Result<FsIo, BrowseError> {
    Err(BrowseError::Unavailable(
        "document trees exist only on Android".into(),
    ))
}

/// The account's home folder, which `~` stands for in typed paths.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

/// An app container has no home a person would type `~` for.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn home_dir() -> Option<PathBuf> {
    None
}

/// The path a person typed into an address bar or file name box, resolved the
/// way the platform's file browser resolves it: surrounding quotes and spaces
/// are dropped, `file://` URIs become paths, `~` stands for `home`, Windows
/// `%VARIABLE%`s are expanded, and a relative path is taken from `current`.
/// The result is normalized with [`normalize_lexically`].
pub fn resolve_typed_path(typed: &str, current: &Path, home: Option<&Path>) -> PathBuf {
    let typed = typed.trim();
    let typed = typed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(typed);
    let typed = expand_variables(typed);
    let path = file_uri_path(&typed).unwrap_or_else(|| PathBuf::from(&typed));
    let path = match (home, path.strip_prefix("~")) {
        (Some(home), Ok(rest)) => home.join(rest),
        _ => path,
    };
    let path = if path.is_absolute() || has_windows_prefix(&path) {
        path
    } else {
        current.join(path)
    };
    normalize_lexically(&path)
}

/// `path` with `.` dropped and `..` taking away the component before it,
/// without asking the file system, so a link in the path keeps its name as
/// the platform's file browser keeps it. `..` never climbs above the root.
pub fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push("..");
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Whether `path` starts with a drive or UNC prefix, which makes it absolute
/// on Windows even without a root.
fn has_windows_prefix(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::Prefix(_)))
}

/// `typed` with each `%NAME%` that names an environment variable replaced by
/// its value. Only Windows file browsers do this.
fn expand_variables(typed: &str) -> String {
    if !cfg!(windows) {
        return typed.to_owned();
    }
    let mut expanded = String::new();
    let mut rest = typed;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            break;
        };
        let name = &after[..end];
        expanded.push_str(&rest[..start]);
        match env::var(name) {
            Ok(value) if !name.is_empty() => expanded.push_str(&value),
            _ => {
                expanded.push('%');
                expanded.push_str(name);
                expanded.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);
    expanded
}

/// The path a local `file://` URI names, with its escapes decoded.
fn file_uri_path(typed: &str) -> Option<PathBuf> {
    let rest = typed.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest)?;
    if cfg!(windows) {
        // file:///C:/Music names C:/Music.
        let trimmed = decoded.strip_prefix('/').unwrap_or(&decoded);
        return Some(PathBuf::from(trimmed.replace('/', "\\")));
    }
    Some(PathBuf::from(decoded))
}

/// `text` with every `%XX` escape decoded, or `None` when an escape is broken
/// or the bytes are not UTF-8.
pub(crate) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing_drops_dots_and_climbs_parents() {
        assert_eq!(
            normalize_lexically(Path::new("/music/./albums/../singles/")),
            PathBuf::from("/music/singles")
        );
        assert_eq!(normalize_lexically(Path::new("/../..")), PathBuf::from("/"));
        assert_eq!(
            normalize_lexically(Path::new("a/../../b")),
            PathBuf::from("../b")
        );
    }

    #[cfg(unix)]
    #[test]
    fn typed_paths_resolve_like_a_file_browser() {
        let current = Path::new("/music/albums");
        let home = Some(Path::new("/home/rune"));
        let resolve = |typed| resolve_typed_path(typed, current, home);

        assert_eq!(resolve("  /srv/media  "), PathBuf::from("/srv/media"));
        assert_eq!(resolve("\"/srv/my media\""), PathBuf::from("/srv/my media"));
        assert_eq!(resolve("~"), PathBuf::from("/home/rune"));
        assert_eq!(resolve("~/Music"), PathBuf::from("/home/rune/Music"));
        assert_eq!(
            resolve("live/2019"),
            PathBuf::from("/music/albums/live/2019")
        );
        assert_eq!(resolve(".."), PathBuf::from("/music"));
        assert_eq!(
            resolve("file:///home/rune/My%20Music"),
            PathBuf::from("/home/rune/My Music")
        );
        assert_eq!(
            resolve("file://localhost/srv/%E9%9F%B3%E4%B9%90"),
            PathBuf::from("/srv/音乐")
        );
        // Without a home, a leading ~ is an ordinary name.
        assert_eq!(
            resolve_typed_path("~", current, None),
            PathBuf::from("/music/albums/~")
        );
    }

    #[test]
    fn broken_escapes_are_not_decoded() {
        assert_eq!(percent_decode("a%2"), None);
        assert_eq!(percent_decode("a%zz"), None);
        assert_eq!(percent_decode("%ff"), None);
        assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
    }
}
