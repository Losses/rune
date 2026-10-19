//! Textures for presets (`sampler_<name>`, `rand##`): from the built-in pack or from a folder.
//! Names, extensions and the random pool follow lactea-host's `DirTextures` (projectM's
//! `TextureManager`): lowercase base names without a folder or a known image extension, files in
//! path order, and one random-pool entry per file, sorted.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use anyhow::Result;
use lactea::{Image, Media, TextureProvider};
use lactea_host::{DirTextures, TEXTURE_EXTENSIONS, decode_image};

use crate::pack::Zip;

/// Decoded pixels kept for presets that ask again (a preset change re-requests its textures).
/// Textures are decoded on first use, never the whole pack up front.
const CACHE_BYTES: usize = 64 << 20;

/// Which textures the engine gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureSource {
    /// `textures/` of the built-in pack.
    Builtin,
    /// A preset folder: its `textures` subfolder if it has one, else the folder itself.
    Folder(std::path::PathBuf),
}

/// Same as lactea-host's private `normalize`.
fn normalize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    for ext in TEXTURE_EXTENSIONS {
        if let Some(stem) = lower.strip_suffix(&format!(".{ext}")) {
            return stem.to_string();
        }
    }
    lower
}

fn extension_is_texture(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| TEXTURE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn stem(name: &str) -> &str {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

enum Backend {
    Dir(DirTextures),
    Zip {
        zip: Zip,
        /// (lowercase base name, path in the zip) in path order.
        files: Vec<(String, String)>,
        /// Lowercase base names, one per file, sorted.
        pool: Vec<String>,
    },
}

pub struct PackTextures {
    backend: Backend,
    /// Decoded plain textures by name (`None` = not found / undecodable), oldest first out.
    cache: HashMap<String, Option<Image>>,
    order: VecDeque<String>,
    cached_bytes: usize,
}

impl PackTextures {
    pub fn new(source: &TextureSource) -> Result<Self> {
        let backend = match source {
            TextureSource::Builtin => Self::zip_backend(Zip::builtin()?),
            TextureSource::Folder(dir) => Self::dir_backend(dir),
        };
        Ok(Self::with_backend(backend))
    }

    #[cfg(test)]
    fn from_zip(zip: Zip) -> Self {
        Self::with_backend(Self::zip_backend(zip))
    }

    fn with_backend(backend: Backend) -> Self {
        Self {
            backend,
            cache: HashMap::new(),
            order: VecDeque::new(),
            cached_bytes: 0,
        }
    }

    fn dir_backend(dir: &Path) -> Backend {
        let textures = dir.join("textures");
        let root = if textures.is_dir() {
            textures
        } else {
            dir.to_path_buf()
        };
        Backend::Dir(DirTextures::scan(&root))
    }

    fn zip_backend(zip: Zip) -> Backend {
        let mut names: Vec<String> = zip
            .file_names()
            .into_iter()
            .filter(|n| n.starts_with("textures/") && extension_is_texture(n))
            .collect();
        names.sort();
        let files: Vec<(String, String)> = names
            .into_iter()
            .map(|n| (stem(&n).to_ascii_lowercase(), n))
            .collect();
        let mut pool: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
        pool.sort();
        Backend::Zip { zip, files, pool }
    }

    fn decode(&mut self, key: &str) -> Option<Image> {
        match &mut self.backend {
            Backend::Dir(_) => unreachable!("directories decode through DirTextures"),
            Backend::Zip { zip, files, .. } => files
                .iter()
                .filter(|(n, _)| n == key)
                .find_map(|(_, path)| decode_image(&zip.read(path).ok()?).ok()),
        }
    }

    fn remember(&mut self, key: String, image: Option<Image>) {
        let size = image.as_ref().map_or(0, |i| i.rgba8.len());
        while self.cached_bytes + size > CACHE_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(Some(old)) = self.cache.remove(&oldest) {
                self.cached_bytes -= old.rgba8.len();
            }
        }
        self.cached_bytes += size;
        self.order.push_back(key.clone());
        self.cache.insert(key, image);
    }
}

impl TextureProvider for PackTextures {
    fn load(&mut self, name: &str) -> Option<Image> {
        let key = normalize(name);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let image = match &mut self.backend {
            Backend::Dir(dir) => dir.load(name),
            Backend::Zip { .. } => self.decode(&key),
        };
        self.remember(key, image.clone());
        image
    }

    fn random_pool(&self) -> Vec<String> {
        match &self.backend {
            Backend::Dir(dir) => dir.random_pool(),
            Backend::Zip { pool, .. } => pool.clone(),
        }
    }

    // The MD3 lookups (GIFs, sprites) only exist for folders; the built-in pack has none.
    fn load_md3_texture(&mut self, name: &str) -> Option<Media> {
        match &mut self.backend {
            Backend::Dir(dir) => dir.load_md3_texture(name),
            Backend::Zip { .. } => self.load(name).map(Media::Still),
        }
    }

    fn load_sprite(&mut self, path: &str) -> Option<Media> {
        match &mut self.backend {
            Backend::Dir(dir) => dir.load_sprite(path),
            Backend::Zip { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::test_zip;

    /// A 32-bit BMP (one of the plain texture formats), `width * height` pixels of `rgb`.
    fn bmp(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let pixels = width * height * 4;
        let mut out = Vec::new();
        out.extend(b"BM");
        out.extend((54 + pixels).to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(54u32.to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(pixels.to_le_bytes());
        out.extend([0u8; 16]);
        for _ in 0..width * height {
            out.extend([rgb[2], rgb[1], rgb[0], 255]);
        }
        out
    }

    #[test]
    fn zip_lookup_ignores_case_folder_and_extension() {
        let bytes = test_zip(&[
            ("textures/Clouds2.BMP", &bmp(4, 2, [1, 2, 3])),
            ("textures/sub/Noise.bmp", &bmp(2, 2, [9, 9, 9])),
            ("textures/notes.txt", b"x"),
            ("presets/clouds2.bmp", &bmp(1, 1, [0, 0, 0])),
        ]);
        let mut textures = PackTextures::from_zip(Zip::new(bytes).unwrap());
        for name in ["clouds2", "CLOUDS2", "Clouds2.bmp", "x/clouds2.jpg"] {
            let image = textures.load(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!((image.width, image.height), (4, 2), "{name}");
            assert_eq!(&image.rgba8[..3], &[1, 2, 3]);
        }
        assert_eq!(textures.load("noise").unwrap().width, 2);
        assert!(textures.load("notes").is_none());
        assert!(textures.load("nope").is_none());
        // Only `textures/` counts, one entry per file, sorted.
        assert_eq!(textures.random_pool(), ["clouds2", "noise"]);
    }

    #[test]
    fn cache_is_bounded() {
        let bytes = test_zip(&[("textures/a.bmp", &bmp(2, 2, [1, 1, 1]))]);
        let mut textures = PackTextures::from_zip(Zip::new(bytes).unwrap());
        for i in 0..1000 {
            textures.remember(
                format!("k{i}"),
                Some(Image {
                    width: 1024,
                    height: 1024,
                    rgba8: vec![0; 1024 * 1024 * 4],
                }),
            );
        }
        assert!(textures.cached_bytes <= CACHE_BYTES);
        assert!(textures.cache.len() <= 16);
        assert!(textures.load("a").is_some());
    }

    #[test]
    fn folder_uses_its_textures_subfolder_else_itself() {
        let root = std::env::temp_dir().join(format!("rune-textures-{}", std::process::id()));
        let with = root.join("with");
        let without = root.join("without");
        std::fs::create_dir_all(with.join("textures")).unwrap();
        std::fs::create_dir_all(&without).unwrap();
        std::fs::write(with.join("textures/inner.bmp"), bmp(3, 1, [1, 1, 1])).unwrap();
        std::fs::write(with.join("outer.bmp"), bmp(1, 1, [1, 1, 1])).unwrap();
        std::fs::write(without.join("flat.bmp"), bmp(5, 1, [1, 1, 1])).unwrap();

        let mut a = PackTextures::new(&TextureSource::Folder(with)).unwrap();
        assert_eq!(a.load("inner").unwrap().width, 3);
        assert!(a.load("outer").is_none());
        assert_eq!(a.random_pool(), ["inner"]);
        let mut b = PackTextures::new(&TextureSource::Folder(without)).unwrap();
        assert_eq!(b.load("flat").unwrap().width, 5);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(builtin_presets)]
    fn every_builtin_texture_decodes() {
        let mut textures = PackTextures::new(&TextureSource::Builtin).unwrap();
        let pool = textures.random_pool();
        assert!(pool.len() > 50, "{}", pool.len());
        for name in pool {
            assert!(textures.load(&name).is_some(), "{name}");
        }
    }
}
