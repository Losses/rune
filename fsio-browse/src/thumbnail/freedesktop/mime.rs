//! File types as the shared MIME-info database names them, matched by file
//! name the way GLib matches them for thumbnails: literal names first, then
//! the longest extension. Content sniffing is left out; thumbnailers are
//! registered by the types names give.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::data_dirs;

/// One glob's type and weight, for picking among globs that match.
struct Glob {
    weight: u32,
    mime: String,
}

#[derive(Default)]
pub(super) struct MimeDatabase {
    /// Lowercased extensions of case-insensitive `*.ext` globs.
    extensions: HashMap<String, Glob>,
    /// Extensions of case-sensitive globs, as written.
    exact_extensions: HashMap<String, Glob>,
    /// Literal file names.
    names: HashMap<String, Glob>,
    /// Alias to canonical type.
    aliases: HashMap<String, String>,
}

impl MimeDatabase {
    /// The database from every XDG data folder, the more important ones
    /// winning ties.
    pub(super) fn load() -> Self {
        let mut database = Self::default();
        for dir in data_dirs() {
            if let Ok(text) = fs::read_to_string(dir.join("mime/globs2")) {
                database.add_globs(&text);
            }
            if let Ok(text) = fs::read_to_string(dir.join("mime/aliases")) {
                database.add_aliases(&text);
            }
        }
        database
    }

    /// Lines of `weight:type:glob[:flags]`.
    pub(super) fn add_globs(&mut self, text: &str) {
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.split(':');
            let (Some(weight), Some(mime), Some(pattern)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse() else {
                continue;
            };
            let case_sensitive = fields.next().is_some_and(|flags| flags.contains("cs"));
            let glob = Glob {
                weight,
                mime: mime.to_owned(),
            };
            let wildcard = |text: &str| text.contains(['*', '?', '[']);
            let (table, key) = match pattern.strip_prefix("*.") {
                Some(extension) if !wildcard(extension) && case_sensitive => {
                    (&mut self.exact_extensions, extension.to_owned())
                }
                Some(extension) if !wildcard(extension) => {
                    (&mut self.extensions, extension.to_lowercase())
                }
                None if !wildcard(pattern) => (&mut self.names, pattern.to_owned()),
                _ => continue,
            };
            if table.get(&key).is_none_or(|kept| kept.weight < glob.weight) {
                table.insert(key, glob);
            }
        }
    }

    /// Lines of `alias canonical`.
    pub(super) fn add_aliases(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((alias, canonical)) = line.split_once(' ') {
                self.aliases
                    .entry(alias.to_owned())
                    .or_insert_with(|| canonical.trim().to_owned());
            }
        }
    }

    /// The canonical name of `mime`, which may be an alias.
    pub(super) fn canonical(&self, mime: &str) -> String {
        self.aliases
            .get(mime)
            .cloned()
            .unwrap_or_else(|| mime.to_owned())
    }

    /// The type of a file called by `path`'s last component.
    pub(super) fn type_of(&self, path: &Path) -> Option<String> {
        let name = path.file_name()?.to_str()?;
        if let Some(glob) = self.names.get(name) {
            return Some(glob.mime.clone());
        }
        // Longest extension first, so archive.tar.gz is a compressed tarball
        // rather than a gzip file.
        for (index, _) in name.match_indices('.') {
            let extension = &name[index + 1..];
            if let Some(glob) = self
                .exact_extensions
                .get(extension)
                .or_else(|| self.extensions.get(&extension.to_lowercase()))
            {
                return Some(self.canonical(&glob.mime));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> MimeDatabase {
        let mut database = MimeDatabase::default();
        database.add_globs(
            "# comment\n\
             50:audio/flac:*.flac\n\
             50:audio/x-flac:*.flac\n\
             55:application/x-compressed-tar:*.tar.gz\n\
             50:application/gzip:*.gz\n\
             50:text/x-makefile:Makefile:cs\n\
             50:image/x-pcx:*.PCX:cs\n\
             50:text/x-csrc:*.[ch]\n",
        );
        database.add_aliases("audio/x-flac audio/flac\napplication/x-gzip application/gzip\n");
        database
    }

    #[test]
    fn types_come_from_names_and_the_longest_extension() {
        let database = database();
        let type_of = |name: &str| database.type_of(Path::new(name));
        assert_eq!(
            type_of("/music/Track 01.FLAC").as_deref(),
            Some("audio/flac")
        );
        assert_eq!(
            type_of("backup.tar.gz").as_deref(),
            Some("application/x-compressed-tar")
        );
        assert_eq!(type_of("notes.gz").as_deref(), Some("application/gzip"));
        assert_eq!(type_of("Makefile").as_deref(), Some("text/x-makefile"));
        assert_eq!(type_of("photo.PCX").as_deref(), Some("image/x-pcx"));
        assert_eq!(type_of("photo.pcx"), None);
        assert_eq!(type_of("main.c"), None);
        assert_eq!(type_of("README"), None);
    }

    #[test]
    fn aliases_resolve_to_canonical_types() {
        let database = database();
        assert_eq!(database.canonical("audio/x-flac"), "audio/flac");
        assert_eq!(database.canonical("audio/flac"), "audio/flac");
    }
}
