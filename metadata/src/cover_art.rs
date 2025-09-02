use std::path::Path;

use anyhow::{Context, Result};
use image::{GenericImageView, ImageBuffer, Pixel};
use lofty::file::TaggedFileExt;
use log::{error, info};
use palette_extract::{Color, get_palette_rgb};

use ::fsio::FsIo;
use symphonia::core::{
    formats::FormatOptions,
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};

use crate::crc::media_crc32;

pub struct CoverArt {
    pub crc: String,
    pub data: Vec<u8>,
    pub primary_color: i32,
}

fn decode_image(image_data: &[u8]) -> Result<Vec<u8>> {
    // Decode the image from binary data
    let img = image::load_from_memory(image_data)?;

    // Resize the image to 16x16 pixels
    let resized = img.resize_exact(16, 16, image::imageops::FilterType::Lanczos3);

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
    if let Some(cover_art) = extract_from_tagged_file(fsio, file_path) {
        return Some(cover_art);
    }

    if let Some(lib_path) = lib_path {
        info!("Falling back to external cover art");
        return fallback_to_external_cover(fsio, file_path.as_ref(), lib_path);
    }

    None
}

fn extract_from_tagged_file<P: AsRef<Path>>(fsio: &FsIo, file_path: &P) -> Option<CoverArt> {
    let file_path = match fsio.canonicalize_path(file_path.as_ref()) {
        Ok(x) => x,
        Err(_) => return None,
    };

    let tagged_file = lofty::read_from_path(file_path).ok()?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())?;

    let picture = tag.pictures().first()?;
    let cover_data = picture.data().to_vec();

    if cover_data.is_empty() {
        return None;
    }

    let rgb_sequence = decode_image(&cover_data).ok()?;

    // Calculate the CRC
    let crc = media_crc32(&rgb_sequence, 0, 0, rgb_sequence.len());
    let primary_color = get_palette_rgb(&rgb_sequence)[0];

    if crc == 0 {
        return None;
    }

    let crc_string = format!("{crc:08x}");

    Some(CoverArt {
        crc: crc_string,
        data: cover_data,
        primary_color: color_to_int(&primary_color),
    })
}

pub async fn extract_cover_art_from_stream(
    source: Box<dyn MediaSource>,
    mime_type: &str,
) -> Result<CoverArt> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    hint.with_extension(mime_type);

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .context("Failed to probe media source")?;

    let mut format = probed.format;

    if let Some(metadata) = format.metadata().current()
        && let Some(visual) = metadata.visuals().first()
    {
        let cover_data = visual.data.to_vec();
        if cover_data.is_empty() {
            return Err(anyhow::anyhow!("Empty cover art data in stream"));
        }

        let rgb_sequence = decode_image(&cover_data)?;

        // Calculate the CRC
        let crc = media_crc32(&rgb_sequence, 0, 0, rgb_sequence.len());
        if crc == 0 {
            return Err(anyhow::anyhow!("Invalid CRC for cover art"));
        }
        let primary_color = get_palette_rgb(&rgb_sequence)[0];
        let crc_string = format!("{crc:08x}");

        return Ok(CoverArt {
            crc: crc_string,
            data: cover_data,
            primary_color: color_to_int(&primary_color),
        });
    }

    Err(anyhow::anyhow!("No cover art found in stream"))
}

fn fallback_to_external_cover(fsio: &FsIo, lib_path: &Path, file_path: &Path) -> Option<CoverArt> {
    if !file_path.starts_with(lib_path) {
        error!("File path is not within the library path");
        return None;
    }

    let cover_names = [
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

    let mut current_dir = file_path.parent()?;

    while current_dir.starts_with(lib_path) {
        for cover_name in &cover_names {
            let cover_path = current_dir.join(cover_name);
            let exists = match fsio.exists(&cover_path) {
                Ok(x) => x,
                Err(_) => return None,
            };

            if exists && let Some(cover_art) = process_external_cover(fsio, &cover_path) {
                return Some(cover_art);
            }
        }
        current_dir = current_dir.parent()?;
    }

    None
}

fn process_external_cover(fsio: &FsIo, cover_path: &Path) -> Option<CoverArt> {
    let cover_data = fsio.read(cover_path).ok()?;

    if cover_data.is_empty() {
        return None;
    }

    let rgb_sequence = decode_image(&cover_data).ok()?;

    // Calculate the CRC
    let crc = media_crc32(&rgb_sequence, 0, 0, rgb_sequence.len());
    let primary_color = get_palette_rgb(&rgb_sequence)[0];

    if crc == 0 {
        return None;
    }

    let crc_string = format!("{crc:08x}");

    Some(CoverArt {
        crc: crc_string,
        data: cover_data,
        primary_color: color_to_int(&primary_color),
    })
}
