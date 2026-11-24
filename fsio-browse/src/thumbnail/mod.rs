//! Thumbnails as the platform draws them for its own file browser: the
//! Windows shell, Quick Look on Apple platforms, the freedesktop thumbnail
//! cache and its thumbnailers on Linux, and the document provider on
//! Android. An item the platform draws none for keeps its type icon.

use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::Arc;

use anyhow::Result;
use fsio::FsIo;
use image::DynamicImage;
use tokio::task;

use crate::location::{Location, Root};

#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(target_os = "linux")]
mod freedesktop;
#[cfg(target_os = "windows")]
mod windows;

/// The pixels of a thumbnail: rows of straight (not premultiplied) RGBA8,
/// top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The edge, in pixels, of the square a thumbnail is scaled to fit in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailSize(pub u32);

/// Draws thumbnails the platform's way. What it learns about the platform
/// (on Linux: the type database, the registered thumbnailers, the sandbox and
/// the thumbnail service) it keeps for its own lifetime, so keep one for as
/// long as thumbnails are asked for. Clones share that knowledge.
#[derive(Clone)]
pub struct Thumbnailer {
    #[cfg(target_os = "linux")]
    freedesktop: Arc<freedesktop::Freedesktop>,
}

impl Default for Thumbnailer {
    fn default() -> Self {
        Self::new()
    }
}

impl Thumbnailer {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            freedesktop: Arc::new(freedesktop::Freedesktop::new()),
        }
    }

    /// The thumbnail the platform draws for the item at `location`, scaled
    /// to fit `size`. Items in an Android document tree are drawn by their
    /// document provider, which `fs` names them to.
    pub async fn item(
        &self,
        fs: &FsIo,
        location: &Location,
        size: ThumbnailSize,
    ) -> Result<Option<Thumbnail>> {
        match &location.root {
            Root::FileSystem => {
                let thumbnailer = self.clone();
                let path = location.path.clone();
                task::spawn_blocking(move || thumbnailer.file(&path, size)).await?
            }
            Root::DocumentTree { .. } => {
                let Some(uri) = fs.content_uri(&location.path)? else {
                    return Ok(None);
                };
                task::spawn_blocking(move || document_thumbnail(&uri, size)).await?
            }
        }
    }

    /// The thumbnail the platform draws for the file or folder at `path` on
    /// the device's own file system, scaled to fit `size`. `Ok(None)` when
    /// it draws none for this item. Blocks while the platform renders, so
    /// call it on a blocking thread.
    pub fn file(&self, path: &Path, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
        #[cfg(target_os = "windows")]
        return windows::thumbnail(path, size);
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        return apple::thumbnail(path, size);
        #[cfg(target_os = "linux")]
        return self.freedesktop.thumbnail(path, size);
        // Android apps reach shared files through document providers, which
        // draw thumbnails for document URIs. A path gives none.
        #[cfg(target_os = "android")]
        {
            let _ = (path, size);
            Ok(None)
        }
    }
}

/// The thumbnail a document provider draws for the document at `uri`.
fn document_thumbnail(uri: &str, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
    #[cfg(target_os = "android")]
    return android::thumbnail(uri, size);
    #[cfg(not(target_os = "android"))]
    {
        let _ = (uri, size);
        Ok(None)
    }
}

/// `image` scaled down to fit `size`, never up. Platforms may draw a larger
/// thumbnail than asked when that is what they have at hand.
pub(crate) fn fit(image: DynamicImage, size: ThumbnailSize) -> Thumbnail {
    let ThumbnailSize(edge) = size;
    let image = if image.width() > edge || image.height() > edge {
        image.thumbnail(edge, edge)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Thumbnail {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

/// [`fit`] for pixels the platform handed over as straight RGBA rows.
#[cfg(not(target_os = "linux"))]
pub(crate) fn fit_pixels(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    size: ThumbnailSize,
) -> Option<Thumbnail> {
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    Some(fit(DynamicImage::ImageRgba8(image), size))
}

/// Straight alpha from premultiplied: platform bitmaps carry color already
/// multiplied by alpha. `alpha` is the byte offset of alpha within each
/// 4-byte pixel; the other three bytes are color.
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
pub(crate) fn unpremultiply(pixels: &mut [u8], alpha: usize) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let a = u32::from(pixel[alpha]);
        if a == 0 || a == 255 {
            continue;
        }
        for (index, value) in pixel.iter_mut().enumerate() {
            if index != alpha {
                *value = ((u32::from(*value) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn larger_platform_thumbnails_shrink_to_fit_and_smaller_ones_stay() {
        let wide = fit_pixels(400, 200, vec![255; 400 * 200 * 4], ThumbnailSize(100)).unwrap();
        assert_eq!((wide.width, wide.height), (100, 50));
        assert_eq!(wide.rgba.len(), 100 * 50 * 4);

        let small = fit_pixels(20, 10, vec![7; 20 * 10 * 4], ThumbnailSize(100)).unwrap();
        assert_eq!((small.width, small.height), (20, 10));
        assert_eq!(&small.rgba[..4], &[7, 7, 7, 7]);

        assert_eq!(fit_pixels(10, 10, vec![0; 3], ThumbnailSize(100)), None);
    }

    #[test]
    fn premultiplied_color_is_restored() {
        let mut pixels = vec![64, 32, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0];
        unpremultiply(&mut pixels, 3);
        assert_eq!(pixels, [128, 64, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0]);
    }
}
