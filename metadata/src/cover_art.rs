use image::imageops::FilterType;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{GenericImageView, ImageBuffer, Pixel};
use palette_extract::{Color, get_palette_rgb};

use ::fsio::FsIo;

use crate::crc::media_crc32;
use crate::taglib::{self, ReadFields};

#[derive(Clone)]
pub struct CoverArt {
    pub crc: String,
    pub data: Vec<u8>,
    pub primary_color: i32,
}

fn decode_image(image_data: &[u8]) -> Result<Vec<u8>> {
    // Decode the image from binary data
    let img = image::load_from_memory(image_data)?;

    // Resize the image to 16x16 pixels
    let resized = img.resize_exact(16, 16, FilterType::Lanczos3);

    // Create a new RGB image buffer
    let mut rgb_image = ImageBuffer::new(16, 16);

    // Copy the resized image into the RGB buffer
    for (x, y, pixel) in resized.pixels() {
        let rgb = pixel.to_rgb();
        rgb_image.put_pixel(x, y, rgb);
    }

    // Convert the RGB image into a flat RGB sequence
    let rgb_sequence: Vec<u8> = rgb_image.into_raw();

    Ok(rgb_sequence)
}

pub fn get_primary_color(x: &[u8]) -> Option<i32> {
    if x.is_empty() {
        return None;
    }
    let decoded_image = decode_image(x);
    match decoded_image {
        Ok(x) => {
            let primary_color = get_palette_rgb(&x)[0];
            Some(color_to_int(&primary_color))
        }
        Err(_) => None,
    }
}

pub fn color_to_int(color: &Color) -> i32 {
    let alpha: i32 = 0xFF;
    let r: i32 = (color.r as i32) & 0xFF;
    let g: i32 = (color.g as i32) & 0xFF;
    let b: i32 = (color.b as i32) & 0xFF;

    (alpha << 24) | (r << 16) | (g << 8) | b
}

pub fn extract_cover_art_binary<P: AsRef<Path>>(
    fsio: &FsIo,
    lib_path: Option<&Path>,
    file_path: &P,
) -> Option<CoverArt> {
    extract_cover_art_binary_checked(fsio, lib_path, file_path)
        .ok()
        .flatten()
}

/// Inspect embedded and external artwork, keeping inspection errors distinct from absence.
pub fn extract_cover_art_binary_checked<P: AsRef<Path>>(
    fsio: &FsIo,
    lib_path: Option<&Path>,
    file_path: &P,
) -> Result<Option<CoverArt>> {
    let snapshot = taglib::read_file(fsio, file_path.as_ref(), ReadFields::Pictures)?;
    if let Some(picture) = snapshot.pictures.into_iter().next() {
        return cover_from_bytes(picture.data).map(Some);
    }
    match lib_path {
        Some(root) => fallback_to_external_cover(fsio, file_path.as_ref(), root),
        None => Ok(None),
    }
}

pub async fn extract_cover_art_from_stream(
    source: impl std::io::Read + std::io::Seek + Send + 'static,
    hint: &str,
) -> Result<CoverArt> {
    let hint = hint.to_owned();
    let snapshot =
        tokio::task::spawn_blocking(move || taglib::read(source, &hint, ReadFields::Pictures))
            .await
            .context("TagLib stream reader failed")??;
    let picture = snapshot
        .pictures
        .into_iter()
        .next()
        .context("No cover art found in stream")?;
    cover_from_bytes(picture.data)
}

/// Validate an image and derive the cover metadata from its encoded bytes.
pub fn cover_from_bytes(data: Vec<u8>) -> Result<CoverArt> {
    anyhow::ensure!(!data.is_empty(), "empty artwork");
    let rgb = decode_image(&data)?;
    let crc = media_crc32(&rgb, 0, 0, rgb.len());
    anyhow::ensure!(crc != 0, "invalid artwork CRC");
    Ok(CoverArt {
        crc: format!("{crc:08x}"),
        data,
        primary_color: color_to_int(&get_palette_rgb(&rgb)[0]),
    })
}

const EXTERNAL_COVER_NAMES: &[&str] = &[
    "cover.png",
    "cover.jpg",
    "cover.jpeg",
    "folder.png",
    "folder.jpg",
    "folder.jpeg",
    "Cover.png",
    "Cover.jpg",
    "Cover.jpeg",
    "Folder.png",
    "Folder.jpg",
    "Folder.jpeg",
    "front.png",
    "front.jpg",
    "front.jpeg",
    "Front.png",
    "Front.jpg",
    "Front.jpeg",
    "albumart.png",
    "albumart.jpg",
    "albumart.jpeg",
    "AlbumArt.png",
    "AlbumArt.jpg",
    "AlbumArt.jpeg",
    "Albumart.png",
    "Albumart.jpg",
    "Albumart.jpeg",
    "coverart.png",
    "coverart.jpg",
    "coverart.jpeg",
    "CoverArt.png",
    "CoverArt.jpg",
    "CoverArt.jpeg",
    "Coverart.png",
    "Coverart.jpg",
    "Coverart.jpeg",
];

pub(crate) fn resembles_external_cover(name: &str) -> bool {
    EXTERNAL_COVER_NAMES
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
}

/// Use the backend's case rules and the same order as fallback extraction.
pub fn find_external_cover_in_directory(fsio: &FsIo, directory: &Path) -> Result<Option<PathBuf>> {
    for name in EXTERNAL_COVER_NAMES {
        let path = directory.join(name);
        if fsio.exists(&path)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn fallback_to_external_cover(
    fsio: &FsIo,
    file_path: &Path,
    lib_path: &Path,
) -> Result<Option<CoverArt>> {
    anyhow::ensure!(
        file_path.starts_with(lib_path),
        "file path is not within library"
    );

    let mut current_dir = file_path.parent().context("file has no parent directory")?;

    while current_dir.starts_with(lib_path) {
        if let Some(path) = find_external_cover_in_directory(fsio, current_dir)? {
            return Ok(Some(read_external_cover(fsio, &path)?));
        }
        let Some(parent) = current_dir.parent() else {
            break;
        };
        current_dir = parent;
    }
    Ok(None)
}

fn read_external_cover(fsio: &FsIo, cover_path: &Path) -> Result<CoverArt> {
    cover_from_bytes(fsio.read(cover_path)?)
}

#[cfg(test)]
mod inspection_tests {
    use super::*;
    use std::env;
    use std::process as std_process;
    #[test]
    fn missing_or_unreadable_audio_is_an_error_not_confirmed_absence() {
        let fs = FsIo::new();
        let missing = env::temp_dir().join(format!("rune-missing-art-{}", std_process::id()));
        assert!(extract_cover_art_binary_checked(&fs, missing.parent(), &missing).is_err());
    }
}
