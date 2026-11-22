//! The freedesktop.org thumbnail specification: every file manager on Linux
//! keeps thumbnails in `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of URI>.png`,
//! tagged with the file's URI and modification time. A cached thumbnail is
//! used while it is current; an image without one is decoded here and added
//! to the cache, as the specification asks of any program that makes one.

use std::env;
use std::fs;
use std::io::{BufWriter, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use image::ImageFormat;

use super::{Thumbnail, ThumbnailSize, decode_image_thumbnail, fit};

/// The cache's size folders with the edge their thumbnails fit in.
const BUCKETS: [(&str, u32); 4] = [
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

/// Images larger than this are left to the type icon: decoding them would
/// hold up the folder for a long time.
const LARGEST_DECODED_IMAGE: u64 = 64 * 1024 * 1024;

/// Extensions of the image formats the decoder reads.
const IMAGE_EXTENSIONS: [&str; 13] = [
    "png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "ico", "tif", "tiff", "tga", "qoi", "avif",
];

pub(super) fn thumbnail(path: &Path, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
    let Some(cache) = cache_dir() else {
        return Ok(None);
    };
    let path = fs::canonicalize(path).context("could not resolve the file")?;
    if path.starts_with(&cache) {
        // The specification forbids thumbnails of thumbnails.
        return Ok(None);
    }
    let metadata = fs::metadata(&path)?;
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|age| age.as_secs())
        .unwrap_or_default();
    let uri = file_uri(&path);
    let name = format!("{:x}.png", md5::compute(uri.as_bytes()));

    for (bucket, _) in buckets_for(size) {
        let file = cache.join(bucket).join(&name);
        if let Some(thumbnail) = read_current(&file, &uri, mtime, size) {
            return Ok(Some(thumbnail));
        }
    }

    if !metadata.is_file() || metadata.len() > LARGEST_DECODED_IMAGE || !is_image(&path) {
        return Ok(None);
    }
    let (bucket, edge) = bucket_for(size);
    let Some(full) = decode_image_thumbnail(&fs::read(&path)?, ThumbnailSize(edge)) else {
        return Ok(None);
    };
    if let Err(error) = store(&cache.join(bucket), &name, &uri, mtime, &full) {
        log::debug!("Could not cache the thumbnail of {path:?}: {error:#}");
    }
    Ok(Some(shrink(full, size)))
}

fn cache_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("thumbnails"))
}

/// The bucket a thumbnail of `size` is made in: the smallest that holds it.
fn bucket_for(size: ThumbnailSize) -> (&'static str, u32) {
    let ThumbnailSize(edge) = size;
    BUCKETS
        .into_iter()
        .find(|(_, bucket)| *bucket >= edge)
        .unwrap_or(BUCKETS[BUCKETS.len() - 1])
}

/// Buckets to look in, best first: those large enough from smallest up,
/// then the smaller ones from largest down.
fn buckets_for(size: ThumbnailSize) -> Vec<(&'static str, u32)> {
    let ThumbnailSize(edge) = size;
    let mut order: Vec<_> = BUCKETS
        .iter()
        .copied()
        .filter(|(_, b)| *b >= edge)
        .collect();
    order.extend(BUCKETS.iter().rev().copied().filter(|(_, b)| *b < edge));
    order
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// The file's URI as GLib writes it, which is what the cache is keyed by:
/// unreserved characters and `!$&'()*+,;=:@/` stay, every other byte is
/// escaped.
pub(super) fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        let kept = byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(byte);
        if kept {
            uri.push(char::from(*byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The thumbnail cached in `file` if it was made from this version of the
/// file at `uri`.
fn read_current(file: &Path, uri: &str, mtime: u64, size: ThumbnailSize) -> Option<Thumbnail> {
    let bytes = fs::read(file).ok()?;
    let decoder = png::Decoder::new(bytes.as_slice());
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    let text = |key: &str| {
        info.uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == key)
            .map(|chunk| chunk.text.clone())
            .or_else(|| {
                info.utf8_text
                    .iter()
                    .find(|chunk| chunk.keyword == key)
                    .and_then(|chunk| chunk.get_text().ok())
            })
    };
    if text("Thumb::URI").as_deref() != Some(uri) {
        return None;
    }
    if text("Thumb::MTime")?.trim().parse::<u64>().ok()? != mtime {
        return None;
    }
    let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?;
    Some(fit(image, size))
}

fn shrink(thumbnail: Thumbnail, size: ThumbnailSize) -> Thumbnail {
    let ThumbnailSize(edge) = size;
    if thumbnail.width <= edge && thumbnail.height <= edge {
        return thumbnail;
    }
    let Some(image) =
        image::RgbaImage::from_raw(thumbnail.width, thumbnail.height, thumbnail.rgba.clone())
    else {
        return thumbnail;
    };
    fit(image::DynamicImage::ImageRgba8(image), size)
}

/// Write a thumbnail into the cache the way the specification asks: tagged
/// with its source, readable only by the account, and moved into place in
/// one step so no reader sees half a file.
fn store(bucket: &Path, name: &str, uri: &str, mtime: u64, thumbnail: &Thumbnail) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(bucket)?;
    let temporary = bucket.join(format!(".{name}.rune-{}", process::id()));
    let file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let written = (|| -> Result<()> {
        let mut encoder =
            png::Encoder::new(BufWriter::new(file), thumbnail.width, thumbnail.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.add_text_chunk("Thumb::URI".into(), uri.into())?;
        encoder.add_text_chunk("Thumb::MTime".into(), mtime.to_string())?;
        encoder.add_text_chunk("Software".into(), "Rune".into())?;
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&thumbnail.rgba)?;
        writer.finish()?;
        Ok(())
    })();
    if let Err(error) = written.and_then(|()| Ok(fs::rename(&temporary, bucket.join(name))?)) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, Rgba, RgbaImage};

    use super::*;

    #[test]
    fn uris_escape_like_glib() {
        assert_eq!(
            file_uri(Path::new("/home/rune/My Music/a#b?.flac")),
            "file:///home/rune/My%20Music/a%23b%3F.flac"
        );
        assert_eq!(
            file_uri(Path::new("/srv/音乐/(live) [2019].png")),
            "file:///srv/%E9%9F%B3%E4%B9%90/(live)%20%5B2019%5D.png"
        );
    }

    #[test]
    fn buckets_prefer_the_smallest_that_holds_the_size() {
        let names = |size| {
            buckets_for(ThumbnailSize(size))
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(96), ["normal", "large", "x-large", "xx-large"]);
        assert_eq!(names(256), ["large", "x-large", "xx-large", "normal"]);
        assert_eq!(names(2048), ["xx-large", "x-large", "large", "normal"]);
        assert_eq!(bucket_for(ThumbnailSize(200)), ("large", 256));
        assert_eq!(bucket_for(ThumbnailSize(4096)), ("xx-large", 1024));
    }

    /// The cache directory comes from the environment, so these steps run in
    /// one test with their own cache.
    #[test]
    fn images_are_cached_once_and_reused_while_current() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join("cache");
        // SAFETY: no other test in this crate reads XDG_CACHE_HOME.
        unsafe { env::set_var("XDG_CACHE_HOME", &cache) };

        let picture = home.path().join("cover.png");
        let image = RgbaImage::from_pixel(600, 300, Rgba([0, 0, 255, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        fs::write(&picture, &bytes).unwrap();

        let made = thumbnail(&picture, ThumbnailSize(200)).unwrap().unwrap();
        assert_eq!((made.width, made.height), (200, 100));

        let uri = file_uri(&fs::canonicalize(&picture).unwrap());
        let cached = cache
            .join("thumbnails/large")
            .join(format!("{:x}.png", md5::compute(uri.as_bytes())));
        let stored = image::open(&cached).unwrap();
        assert_eq!((stored.width(), stored.height()), (256, 128));

        // A marked cached copy is what the next request returns.
        let marked = RgbaImage::from_pixel(256, 128, Rgba([0, 255, 0, 255]));
        let mtime = fs::metadata(&picture)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let marked_thumbnail = Thumbnail {
            width: 256,
            height: 128,
            rgba: marked.into_raw(),
        };
        let bucket = cached.parent().unwrap();
        let name = cached.file_name().unwrap().to_str().unwrap();
        store(bucket, name, &uri, mtime, &marked_thumbnail).unwrap();
        let reused = thumbnail(&picture, ThumbnailSize(256)).unwrap().unwrap();
        assert_eq!(&reused.rgba[..4], &[0, 255, 0, 255]);

        // A copy made from another version of the file is not used.
        store(bucket, name, &uri, mtime + 1, &marked_thumbnail).unwrap();
        let remade = thumbnail(&picture, ThumbnailSize(256)).unwrap().unwrap();
        assert_eq!(&remade.rgba[..4], &[0, 0, 255, 255]);

        let text = home.path().join("notes.txt");
        fs::write(&text, b"not an image").unwrap();
        assert_eq!(thumbnail(&text, ThumbnailSize(128)).unwrap(), None);
    }
}
