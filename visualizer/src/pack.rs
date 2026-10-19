//! The optional built-in preset pack: a zip with `presets/<name>.milk` and `textures/<name>.jpg`
//! that is embedded at build time only when `RUNE_BUILTIN_PRESETS` names one (`build.rs`,
//! `BUILTIN_PRESETS.md`).

use std::borrow::Cow;
use std::io::{Cursor, Read};

use anyhow::{Context, Result};
use zip::ZipArchive;

#[cfg(builtin_presets)]
static BUILTIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/builtin.zip"));

/// The embedded pack, if this build has one.
pub fn builtin_pack() -> Option<&'static [u8]> {
    #[cfg(builtin_presets)]
    {
        Some(BUILTIN)
    }
    #[cfg(not(builtin_presets))]
    {
        None
    }
}

/// Display name of the built-in pack.
pub const BUILTIN_NAME: &str = "MilkDrop Original";

/// Read access to a zip held in memory. Opening one only parses the central directory, so each
/// thread that needs files opens its own.
pub struct Zip {
    archive: ZipArchive<Cursor<Cow<'static, [u8]>>>,
}

impl Zip {
    pub fn new(bytes: impl Into<Cow<'static, [u8]>>) -> Result<Self> {
        let archive =
            ZipArchive::new(Cursor::new(bytes.into())).context("Failed to open the preset pack")?;
        Ok(Self { archive })
    }

    pub fn builtin() -> Result<Self> {
        let bytes = builtin_pack().context("This build has no built-in presets")?;
        Self::new(bytes)
    }

    /// The names of all files (not directories), in archive order.
    pub fn file_names(&self) -> Vec<String> {
        self.archive
            .file_names()
            .filter_map(Result::ok)
            .filter(|name| !name.ends_with('/'))
            .map(Cow::into_owned)
            .collect()
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut file = self
            .archive
            .by_name(name)
            .with_context(|| format!("{name} is not in the preset pack"))?;
        let mut bytes = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
        file.read_to_end(&mut bytes)
            .with_context(|| format!("Failed to unpack {name}"))?;
        Ok(bytes)
    }
}

/// A zip built in memory, for tests.
#[cfg(test)]
pub fn test_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(builtin_presets))]
    fn no_builtin_pack_without_the_env_var() {
        assert!(builtin_pack().is_none());
        assert!(Zip::builtin().is_err());
    }

    #[test]
    #[cfg(builtin_presets)]
    fn builtin_pack_is_a_readable_zip() {
        let mut zip = Zip::builtin().unwrap();
        let names = zip.file_names();
        assert!(names.iter().any(|n| n.starts_with("presets/")));
        assert!(names.iter().any(|n| n.starts_with("textures/")));
        let sources = zip.read("SOURCES.txt").unwrap();
        assert!(!sources.is_empty());
    }

    #[test]
    fn reads_files() {
        let bytes = test_zip(&[("a/b.txt", b"hello")]);
        let mut zip = Zip::new(bytes).unwrap();
        assert_eq!(zip.file_names(), ["a/b.txt"]);
        assert_eq!(zip.read("a/b.txt").unwrap(), b"hello");
        assert!(zip.read("missing").is_err());
    }
}
