//! Textures for presets (`sampler_<name>`, `rand##`) from the preset folder. Names, extensions and
//! the random pool follow lactea-host's `DirTextures` (projectM's `TextureManager`): lowercase base
//! names without a folder or a known image extension, files in path order, and one random-pool
//! entry per file, sorted.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use lactea::{Image, Media, TextureProvider};
use lactea_host::{DirTextures, TEXTURE_EXTENSIONS};

/// Decoded pixels kept for presets that ask again (a preset change re-requests its textures).
/// Textures are decoded on first use, never the whole folder up front.
const CACHE_BYTES: usize = 64 << 20;

/// The folder the textures of the preset folder `dir` are read from: its `textures` subfolder if
/// it has one, else `dir` itself if it holds images, else a `textures` folder next to it (presets
/// in a subfolder, textures beside it, as in Winamp's layout), else `dir`.
fn texture_root(dir: &Path) -> PathBuf {
    let inner = dir.join("textures");
    if inner.is_dir() {
        return inner;
    }
    if !DirTextures::scan(dir).is_empty() {
        return dir.to_path_buf();
    }
    match dir.parent().map(|parent| parent.join("textures")) {
        Some(sibling) if sibling.is_dir() => sibling,
        _ => dir.to_path_buf(),
    }
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

pub struct FolderTextures {
    dir: DirTextures,
    /// Decoded plain textures by name (`None` = not found / undecodable), oldest first out.
    cache: HashMap<String, Option<Image>>,
    order: VecDeque<String>,
    cached_bytes: usize,
}

impl FolderTextures {
    /// The textures of the preset folder `dir`, none without one.
    pub fn new(dir: Option<&Path>) -> Self {
        let dir = match dir {
            Some(dir) => DirTextures::scan(&texture_root(dir)),
            None => DirTextures::new(&[]),
        };
        Self {
            dir,
            cache: HashMap::new(),
            order: VecDeque::new(),
            cached_bytes: 0,
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

impl TextureProvider for FolderTextures {
    fn load(&mut self, name: &str) -> Option<Image> {
        let key = normalize(name);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let image = self.dir.load(name);
        self.remember(key, image.clone());
        image
    }

    fn random_pool(&self) -> Vec<String> {
        self.dir.random_pool()
    }

    fn load_md3_texture(&mut self, name: &str) -> Option<Media> {
        self.dir.load_md3_texture(name)
    }

    fn load_sprite(&mut self, path: &str) -> Option<Media> {
        self.dir.load_sprite(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cache_is_bounded() {
        let mut textures = FolderTextures::new(None);
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
        assert!(textures.load("a").is_none());
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

        let mut a = FolderTextures::new(Some(&with));
        assert_eq!(a.load("inner").unwrap().width, 3);
        assert!(a.load("outer").is_none());
        assert_eq!(a.random_pool(), ["inner"]);
        let mut b = FolderTextures::new(Some(&without));
        assert_eq!(b.load("flat").unwrap().width, 5);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_falls_back_to_a_textures_folder_next_to_it() {
        let root =
            std::env::temp_dir().join(format!("rune-textures-beside-{}", std::process::id()));
        let presets = root.join("presets");
        let own = root.join("own");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(root.join("textures")).unwrap();
        std::fs::write(presets.join("a.milk"), "").unwrap();
        std::fs::write(root.join("textures/beside.bmp"), bmp(4, 1, [1, 1, 1])).unwrap();
        std::fs::write(own.join("mine.bmp"), bmp(2, 1, [1, 1, 1])).unwrap();

        // No images in the folder: the `textures` folder next to it is used.
        let mut a = FolderTextures::new(Some(&presets));
        assert_eq!(a.load("beside").unwrap().width, 4);
        assert_eq!(a.random_pool(), ["beside"]);
        // Images in the folder itself win over the one next to it.
        let mut b = FolderTextures::new(Some(&own));
        assert_eq!(b.load("mine").unwrap().width, 2);
        assert!(b.load("beside").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
