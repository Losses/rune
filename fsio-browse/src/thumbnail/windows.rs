//! The shell's image factory draws the same thumbnails Explorer shows,
//! including the previews of a folder's contents.

use std::mem;
use std::path::Path;

use anyhow::Result;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
};
use windows::core::HSTRING;

use super::{Thumbnail, ThumbnailSize, fit_pixels, unpremultiply};
use crate::windows_com::Apartment;

pub(super) fn thumbnail(path: &Path, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
    let _apartment = Apartment::enter();
    let ThumbnailSize(edge) = size;
    let edge = i32::try_from(edge).unwrap_or(i32::MAX);
    // SAFETY: the path is a valid HSTRING for the duration of the call.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)? };
    // Thumbnails only: a file the shell has no preview for keeps Rune's own
    // type icon, as everywhere else in the dialog.
    // SAFETY: the factory is a live shell item.
    let bitmap = match unsafe {
        factory.GetImage(
            SIZE { cx: edge, cy: edge },
            SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
        )
    } {
        Ok(bitmap) => bitmap,
        Err(_) => return Ok(None),
    };
    let pixels = bitmap_pixels(bitmap);
    // SAFETY: the bitmap belongs to this function once GetImage returns it.
    unsafe {
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
    }
    let Some((width, height, rgba)) = pixels else {
        return Ok(None);
    };
    Ok(fit_pixels(width, height, rgba, size))
}

/// The bitmap's pixels as straight RGBA, top row first.
fn bitmap_pixels(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    let mut header = BITMAP::default();
    // SAFETY: `header` is a BITMAP of the size passed.
    let read = unsafe {
        GetObjectW(
            HGDIOBJ::from(bitmap),
            mem::size_of::<BITMAP>() as i32,
            Some(&mut header as *mut BITMAP as *mut _),
        )
    };
    if read == 0 || header.bmWidth <= 0 || header.bmHeight == 0 {
        return None;
    }
    let width = header.bmWidth;
    let height = header.bmHeight.abs();
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows top first.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: the buffer holds `height` rows of `width` 32-bit pixels, the
    // format `info` asks for; the memory DC is deleted below.
    let copied = unsafe {
        let dc = CreateCompatibleDC(None);
        let copied = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        copied
    };
    if copied == 0 {
        return None;
    }
    // Many thumbnail handlers leave alpha at zero for opaque pictures.
    if pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
        pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|pixel| pixel[3] = 255);
    } else {
        unpremultiply(&mut pixels, 3);
    }
    // BGRA to RGBA.
    pixels
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .for_each(|pixel| pixel.swap(0, 2));
    Some((width as u32, height as u32, pixels))
}
