//! The freedesktop.org thumbnail specification. Every file manager on Linux
//! keeps thumbnails in `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of URI>.png`,
//! tagged with the file's URI and modification time. A current cached
//! thumbnail is used as it is. For a file without one, the session's
//! thumbnail service draws it where one runs; elsewhere the registered
//! thumbnailer programs do, confined (see [`sandbox`]), as GNOME runs them.
//! Their result goes into the shared cache, and a failure is recorded there
//! so the file is not tried again until it changes.

mod mime;
mod programs;
mod sandbox;
mod service;

use std::env;
use std::fs;
use std::io::{BufWriter, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result};
use image::ImageFormat;

use self::mime::MimeDatabase;
use self::programs::Thumbnailers;
use self::sandbox::Sandbox;
use self::service::ThumbnailService;
use super::{Thumbnail, ThumbnailSize, fit};

/// The cache's size folders with the edge their thumbnails fit in.
const BUCKETS: [(&str, u32); 4] = [
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

/// Where failures are recorded, as `fail/<program>` asks.
const FAILURES: &str = "fail/rune";

/// How long to wait for the thumbnail service to fill the cache.
const SERVICE_DEADLINE: Duration = Duration::from_secs(10);

/// What draws thumbnails the cache does not have yet.
enum Generator {
    Service(ThumbnailService, MimeDatabase),
    Programs(Thumbnailers, Sandbox, MimeDatabase),
    /// No service, no sandbox or no registered thumbnailer: only the cache.
    Nothing,
}

pub(super) struct Freedesktop {
    cache: Option<PathBuf>,
    generator: OnceLock<Generator>,
}

impl Freedesktop {
    pub(super) fn new() -> Self {
        Self {
            cache: cache_dir(),
            generator: OnceLock::new(),
        }
    }

    #[cfg(test)]
    fn with(cache: PathBuf, generator: Generator) -> Self {
        Self {
            cache: Some(cache),
            generator: OnceLock::from(generator),
        }
    }

    /// The generator this desktop offers, found on first use.
    fn generator(&self) -> &Generator {
        self.generator.get_or_init(|| {
            let mimes = MimeDatabase::load();
            if let Some(service) = ThumbnailService::connect() {
                return Generator::Service(service, mimes);
            }
            let Some(sandbox) = Sandbox::detect() else {
                log::info!("No sandbox for thumbnailers; using cached thumbnails only");
                return Generator::Nothing;
            };
            let thumbnailers = Thumbnailers::load(&data_dirs(), &mimes);
            if thumbnailers.is_empty() {
                return Generator::Nothing;
            }
            Generator::Programs(thumbnailers, sandbox, mimes)
        })
    }

    pub(super) fn thumbnail(&self, path: &Path, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
        let Some(cache) = &self.cache else {
            return Ok(None);
        };
        let path = fs::canonicalize(path).context("could not resolve the file")?;
        if path.starts_with(cache) {
            // The specification forbids thumbnails of thumbnails.
            return Ok(None);
        }
        let metadata = fs::metadata(&path)?;
        let item = Item {
            mtime: metadata
                .modified()?
                .duration_since(UNIX_EPOCH)
                .map(|age| age.as_secs())
                .unwrap_or_default(),
            uri: file_uri(&path),
            path,
        };
        let name = item.cache_name();
        for (bucket, _) in buckets_for(size) {
            if let Some(thumbnail) = read_current(&cache.join(bucket).join(&name), &item, size) {
                return Ok(Some(thumbnail));
            }
        }
        if !metadata.is_file()
            || read_current(&cache.join(FAILURES).join(&name), &item, size).is_some()
        {
            return Ok(None);
        }
        let (bucket, edge) = bucket_for(size);
        match self.generator() {
            Generator::Nothing => Ok(None),
            Generator::Service(service, mimes) => {
                let Some(mime) = mimes
                    .type_of(&item.path)
                    .filter(|mime| service.handles(mime))
                else {
                    return Ok(None);
                };
                let order = BUCKETS.map(|(name, _)| name);
                let Some(flavor) = service.flavor(bucket, &order) else {
                    return Ok(None);
                };
                if !service.queue(&item.uri, &mime, &flavor) {
                    return Ok(None);
                }
                let file = cache.join(&flavor).join(&name);
                let started = Instant::now();
                while started.elapsed() < SERVICE_DEADLINE {
                    if let Some(thumbnail) = read_current(&file, &item, size) {
                        return Ok(Some(thumbnail));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Ok(None)
            }
            Generator::Programs(thumbnailers, sandbox, mimes) => {
                let Some(mime) = mimes
                    .type_of(&item.path)
                    .filter(|mime| thumbnailers.handles(mime))
                else {
                    return Ok(None);
                };
                let drawn = thumbnailers
                    .draw(sandbox, &mime, &item.path, &item.uri, edge)
                    .and_then(|png| {
                        image::load_from_memory_with_format(&png, ImageFormat::Png).ok()
                    });
                let Some(drawn) = drawn else {
                    let failure = Thumbnail {
                        width: 1,
                        height: 1,
                        rgba: vec![0; 4],
                    };
                    if let Err(error) = store(&cache.join(FAILURES), &name, &item, &failure) {
                        log::debug!("Could not record a failed thumbnail: {error:#}");
                    }
                    return Ok(None);
                };
                let full = fit(drawn, ThumbnailSize(edge));
                if let Err(error) = store(&cache.join(bucket), &name, &item, &full) {
                    log::debug!(
                        "Could not cache the thumbnail of {:?}: {error:#}",
                        item.path
                    );
                }
                Ok(Some(shrink(full, size)))
            }
        }
    }
}

/// A file as the cache knows it.
struct Item {
    path: PathBuf,
    uri: String,
    mtime: u64,
}

impl Item {
    fn cache_name(&self) -> String {
        format!("{:x}.png", md5::compute(self.uri.as_bytes()))
    }
}

/// The XDG data folders, most important first.
fn data_dirs() -> Vec<PathBuf> {
    let home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let system = env::var_os("XDG_DATA_DIRS")
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(env::split_paths(&system).filter(|path| path.is_absolute()))
        .collect()
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

/// The file's URI as GLib writes it, which is what the cache is keyed by:
/// unreserved characters and `!$&'()*+,;=:@/` stay, every other byte is
/// escaped.
fn file_uri(path: &Path) -> String {
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
/// item.
fn read_current(file: &Path, item: &Item, size: ThumbnailSize) -> Option<Thumbnail> {
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
    if text("Thumb::URI").as_deref() != Some(item.uri.as_str()) {
        return None;
    }
    // tumbler and newer GNOME write fractional seconds; the whole second is
    // what the cache's writers agree on.
    let stored = text("Thumb::MTime")?;
    let seconds = stored.trim().split('.').next()?.parse::<u64>().ok()?;
    if seconds != item.mtime {
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
fn store(folder: &Path, name: &str, item: &Item, thumbnail: &Thumbnail) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    let temporary = folder.join(format!(".{name}.rune-{}", process::id()));
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
        encoder.add_text_chunk("Thumb::URI".into(), item.uri.clone())?;
        encoder.add_text_chunk("Thumb::MTime".into(), item.mtime.to_string())?;
        encoder.add_text_chunk("Software".into(), "Rune".into())?;
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&thumbnail.rgba)?;
        writer.finish()?;
        Ok(())
    })();
    if let Err(error) = written.and_then(|()| Ok(fs::rename(&temporary, folder.join(name))?)) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;

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

    fn png_of(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba(color),
        ))
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
        bytes
    }

    fn item_for(path: &Path) -> Item {
        let path = fs::canonicalize(path).unwrap();
        Item {
            mtime: fs::metadata(&path)
                .unwrap()
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            uri: file_uri(&path),
            path,
        }
    }

    /// A registered thumbnailer for `audio/flac` that copies a prepared PNG
    /// to its output, run in whatever sandbox this machine offers.
    #[test]
    fn registered_programs_fill_the_cache_and_failures_are_remembered() {
        let Some(sandbox) = Sandbox::detect() else {
            eprintln!("no sandbox here; skipped");
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        fs::create_dir_all(data.join("mime")).unwrap();
        fs::create_dir_all(data.join("thumbnailers")).unwrap();
        fs::write(data.join("mime/globs2"), "50:audio/flac:*.flac\n").unwrap();
        // The drawn picture travels as the input's own bytes: the sandbox
        // shows the program its input and nothing else of this folder.
        let script = root.path().join("thumb");
        fs::write(&script, "#!/bin/sh\ncp \"$1\" \"$2\"\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            data.join("thumbnailers/test.thumbnailer"),
            format!(
                "[Thumbnailer Entry]\nTryExec={0}\nExec={0} %i %o %s\nMimeType=audio/flac;\n",
                script.display()
            ),
        )
        .unwrap();
        let mut mimes = MimeDatabase::default();
        mimes.add_globs("50:audio/flac:*.flac\n");
        let thumbnailers = Thumbnailers::load(&[data], &mimes);
        let cache = root.path().join("cache");
        let freedesktop = Freedesktop::with(
            cache.clone(),
            Generator::Programs(thumbnailers, sandbox, mimes),
        );

        let song = root.path().join("song.flac");
        fs::write(&song, png_of(400, 400, [200, 10, 10, 255])).unwrap();
        let drawn = freedesktop
            .thumbnail(&song, ThumbnailSize(200))
            .unwrap()
            .unwrap();
        assert_eq!((drawn.width, drawn.height), (200, 200));
        assert_eq!(&drawn.rgba[..4], &[200, 10, 10, 255]);
        let item = item_for(&song);
        let cached = image::open(cache.join("large").join(item.cache_name())).unwrap();
        assert_eq!((cached.width(), cached.height()), (256, 256));

        // A file the program cannot draw is recorded, and not tried again
        // until it changes.
        let broken = root.path().join("broken.flac");
        fs::write(&broken, b"not a picture").unwrap();
        assert_eq!(
            freedesktop.thumbnail(&broken, ThumbnailSize(128)).unwrap(),
            None
        );
        let failed = item_for(&broken);
        assert!(cache.join(FAILURES).join(failed.cache_name()).is_file());
        let failed_at = fs::metadata(&broken).unwrap().modified().unwrap();
        fs::write(&broken, png_of(64, 64, [1, 2, 3, 255])).unwrap();
        let file = fs::File::options().write(true).open(&broken).unwrap();
        file.set_modified(failed_at).unwrap();
        assert_eq!(
            freedesktop.thumbnail(&broken, ThumbnailSize(128)).unwrap(),
            None
        );
        file.set_modified(failed_at + Duration::from_secs(10))
            .unwrap();
        let retried = freedesktop
            .thumbnail(&broken, ThumbnailSize(128))
            .unwrap()
            .unwrap();
        assert_eq!(&retried.rgba[..4], &[1, 2, 3, 255]);

        // Types nobody registered keep their icon without running anything.
        let notes = root.path().join("notes.txt");
        fs::write(&notes, b"text").unwrap();
        assert_eq!(
            freedesktop.thumbnail(&notes, ThumbnailSize(128)).unwrap(),
            None
        );
    }

    #[test]
    fn only_current_cache_entries_are_used() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        let freedesktop = Freedesktop::with(cache.clone(), Generator::Nothing);
        let song = root.path().join("song.flac");
        fs::write(&song, b"flac").unwrap();
        let item = item_for(&song);
        let cover = Thumbnail {
            width: 128,
            height: 128,
            rgba: [0, 255, 0, 255].repeat(128 * 128),
        };

        store(&cache.join("normal"), &item.cache_name(), &item, &cover).unwrap();
        let cached = freedesktop
            .thumbnail(&song, ThumbnailSize(128))
            .unwrap()
            .unwrap();
        assert_eq!(&cached.rgba[..4], &[0, 255, 0, 255]);

        let stale = Item {
            mtime: item.mtime + 1,
            ..item_for(&song)
        };
        store(&cache.join("normal"), &item.cache_name(), &stale, &cover).unwrap();
        assert_eq!(
            freedesktop.thumbnail(&song, ThumbnailSize(128)).unwrap(),
            None
        );
    }

    /// tumbler writes the modification time with microseconds.
    #[test]
    fn fractional_modification_times_are_current() {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song.flac");
        fs::write(&song, b"flac").unwrap();
        let item = item_for(&song);
        let file = root.path().join("thumb.png");
        let mut encoder = png::Encoder::new(fs::File::create(&file).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Thumb::URI".into(), item.uri.clone())
            .unwrap();
        encoder
            .add_text_chunk("Thumb::MTime".into(), format!("{}.383310", item.mtime))
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3, 255]).unwrap();
        writer.finish().unwrap();

        let read = read_current(&file, &item, ThumbnailSize(64)).unwrap();
        assert_eq!(read.rgba, [1, 2, 3, 255]);
        let later = Item {
            mtime: item.mtime + 1,
            ..item_for(&song)
        };
        assert_eq!(read_current(&file, &later, ThumbnailSize(64)), None);
    }
}
