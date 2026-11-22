//! Thumbnails as the platform's own file browser draws them. Windows asks the
//! shell, Apple platforms ask Quick Look, Linux reads and fills the shared
//! freedesktop thumbnail cache. Images on every backend, including Android's
//! document trees, can also be decoded here from their bytes.

use std::io::Cursor;
use std::path::Path;

use anyhow::Result;
use image::{DynamicImage, ImageDecoder, ImageReader};

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

/// The thumbnail the platform draws for the file or folder at `path` on the
/// device's own file system, scaled to fit `size`. `Ok(None)` when it draws
/// none for this item, which then shows its type icon. Blocks while the
/// platform renders, so call it on a blocking thread.
pub fn native_thumbnail(path: &Path, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
    #[cfg(target_os = "windows")]
    return windows::thumbnail(path, size);
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    return apple::thumbnail(path, size);
    #[cfg(target_os = "linux")]
    return freedesktop::thumbnail(path, size);
    #[cfg(target_os = "android")]
    {
        let _ = (path, size);
        Ok(None)
    }
}

/// A thumbnail of an image file's bytes, turned upright by its EXIF
/// orientation and scaled to fit `size`. `None` when the bytes are not an
/// image this decoder reads.
pub fn decode_image_thumbnail(bytes: &[u8], size: ThumbnailSize) -> Option<Thumbnail> {
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Some(fit(image, size))
}

/// `image` scaled down to fit `size`, never up.
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

/// Straight alpha from premultiplied: platform bitmaps carry color already
/// multiplied by alpha. `alpha` is the byte offset of alpha within each
/// 4-byte pixel; the other three bytes are color.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "ios"))]
pub(crate) fn unpremultiply(pixels: &mut [u8], alpha: usize) {
    for pixel in pixels.chunks_exact_mut(4) {
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

#[cfg(test)]
mod tests {
    use image::{ImageFormat, Rgba, RgbaImage};

    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut image = RgbaImage::new(width, height);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decoded_thumbnails_fit_the_size_and_keep_the_aspect() {
        let thumbnail = decode_image_thumbnail(&png_bytes(400, 200), ThumbnailSize(100)).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (100, 50));
        assert_eq!(thumbnail.rgba.len(), 100 * 50 * 4);

        let small = decode_image_thumbnail(&png_bytes(20, 10), ThumbnailSize(100)).unwrap();
        assert_eq!((small.width, small.height), (20, 10));
        assert_eq!(&small.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn bytes_that_are_not_an_image_have_no_thumbnail() {
        assert_eq!(
            decode_image_thumbnail(b"fLaC not an image", ThumbnailSize(64)),
            None
        );
    }
}
