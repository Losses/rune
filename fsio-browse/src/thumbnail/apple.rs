//! Quick Look draws the thumbnails Finder and the Files app show, for every
//! type a Quick Look extension handles.

use std::ffi::c_void;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes,
    QLThumbnailGenerator, QLThumbnailRepresentation,
};

use super::{Thumbnail, ThumbnailSize, fit_pixels, unpremultiply};

/// How long one thumbnail may take. Quick Look answers within a moment for
/// ordinary files; a slow extension must not hold up the folder.
const DEADLINE: Duration = Duration::from_secs(10);

pub(super) fn thumbnail(path: &Path, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
    let Some(path_text) = path.to_str() else {
        return Ok(None);
    };
    let ThumbnailSize(edge) = size;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path_text));
    let (sender, receiver) = mpsc::sync_channel(1);
    // SAFETY: the request and the handler outlive the call; Quick Look calls
    // the handler once, on a queue of its own.
    unsafe {
        let request =
            QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
                QLThumbnailGenerationRequest::alloc(),
                &url,
                CGSize::new(f64::from(edge), f64::from(edge)),
                1.0,
                QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
            );
        let handler = RcBlock::new(
            move |representation: *mut QLThumbnailRepresentation, _error: *mut NSError| {
                let pixels = representation
                    .as_ref()
                    .and_then(|representation| image_pixels(&representation.CGImage()));
                let _ = sender.try_send(pixels);
            },
        );
        QLThumbnailGenerator::sharedGenerator()
            .generateBestRepresentationForRequest_completionHandler(&request, &handler);
    }
    let Ok(Some((width, height, rgba))) = receiver.recv_timeout(DEADLINE) else {
        return Ok(None);
    };
    Ok(fit_pixels(width, height, rgba, size))
}

/// The image's pixels as straight RGBA, top row first.
fn image_pixels(image: &CGImage) -> Option<(u32, u32, Vec<u8>)> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixels = vec![0u8; width * height * 4];
    let space = CGColorSpace::new_device_rgb()?;
    // SAFETY: the buffer holds `height` rows of `width * 4` bytes and lives
    // until the context is released at the end of this function.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast::<c_void>(),
            width,
            height,
            8,
            width * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&context), rect, Some(image));
    drop(context);
    unpremultiply(&mut pixels, 3);
    Some((
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        pixels,
    ))
}
