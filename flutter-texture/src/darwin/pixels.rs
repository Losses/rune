//! Frames that come as pixels, copied into IOSurface-backed pixel buffers of a pool.

use std::ptr::NonNull;

use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferPool,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey,
    kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};

use crate::{PixelFormat, Pixels, Result, TextureError};

/// `kCVPixelFormatType_32BGRA` buffers of one size.
pub(super) struct Pool {
    pool: CFRetained<CVPixelBufferPool>,
    width: u32,
    height: u32,
}

impl Pool {
    fn new(width: u32, height: u32) -> Result<Self> {
        let number = |value: i32| CFNumber::new_i32(value);
        // SAFETY: CoreVideo's constant keys are initialized once the framework is loaded.
        let keys = unsafe {
            [
                kCVPixelBufferPixelFormatTypeKey,
                kCVPixelBufferWidthKey,
                kCVPixelBufferHeightKey,
                kCVPixelBufferIOSurfacePropertiesKey,
                kCVPixelBufferMetalCompatibilityKey,
            ]
        };
        let attributes = CFDictionary::<CFString, CFType>::from_slices(
            &keys,
            &[
                &number(kCVPixelFormatType_32BGRA as i32),
                &number(width as i32),
                &number(height as i32),
                // IOSurface-backed: Flutter wraps the buffer in a Metal texture.
                &CFDictionary::<CFString, CFType>::empty(),
                CFBoolean::new(true),
            ],
        );
        let mut pool: *mut CVPixelBufferPool = std::ptr::null_mut();
        // SAFETY: a valid dictionary and an out pointer to a local.
        let status = unsafe {
            CVPixelBufferPool::create(
                None,
                None,
                Some(attributes.as_opaque()),
                NonNull::from(&mut pool),
            )
        };
        if status != kCVReturnSuccess {
            return Err(frame_error(format!(
                "no pixel buffer pool of {width}x{height} (CVReturn {status})"
            )));
        }
        let pool =
            NonNull::new(pool).ok_or_else(|| frame_error("the pixel buffer pool is null"))?;
        Ok(Self {
            // SAFETY: `create` returns the pool with a +1 reference, which this takes over.
            pool: unsafe { CFRetained::from_raw(pool) },
            width,
            height,
        })
    }

    /// A new buffer from the pool in `slot` (made again if the size changed) with `pixels` in it.
    pub(super) fn copy(
        slot: &mut Option<Self>,
        pixels: &Pixels,
    ) -> Result<CFRetained<CVPixelBuffer>> {
        let (width, height) = (pixels.width(), pixels.height());
        let source = pixels
            .bytes()
            .ok_or_else(|| frame_error(format!("unusable pixels of {width}x{height}")))?;
        if slot
            .as_ref()
            .is_none_or(|pool| (pool.width, pool.height) != (width, height))
        {
            *slot = None;
            *slot = Some(Self::new(width, height)?);
        }
        let pool = slot.as_ref().expect("made above");

        let mut raw: *mut CVPixelBuffer = std::ptr::null_mut();
        // SAFETY: a valid pool and an out pointer to a local.
        let status = unsafe {
            CVPixelBufferPool::create_pixel_buffer(None, &pool.pool, NonNull::from(&mut raw))
        };
        if status != kCVReturnSuccess {
            return Err(frame_error(format!("no pixel buffer (CVReturn {status})")));
        }
        let raw = NonNull::new(raw).ok_or_else(|| frame_error("the pixel buffer is null"))?;
        // SAFETY: the pool returns the buffer with a +1 reference, which this takes over.
        let buffer = unsafe { CFRetained::from_raw(raw) };

        // SAFETY: a lock for writing on a valid buffer, released below.
        if unsafe { CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()) }
            != kCVReturnSuccess
        {
            return Err(frame_error("the pixel buffer cannot be locked"));
        }
        let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
        let stride = CVPixelBufferGetBytesPerRow(&buffer);
        let copied = if base.is_null() || stride < width as usize * 4 {
            false
        } else {
            let len = stride * (height as usize - 1) + width as usize * 4;
            // SAFETY: the buffer is locked and has `height` rows of `stride` bytes, the first
            // `width * 4` of each being pixels.
            let target = unsafe { std::slice::from_raw_parts_mut(base, len) };
            copy_rows(source, pixels.stride(), target, stride, pixels);
            true
        };
        // SAFETY: matches the lock above.
        unsafe { CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()) };
        if !copied {
            return Err(frame_error("the pixel buffer has no writable pixels"));
        }
        Ok(buffer)
    }
}

/// Copy the rows of `pixels` from `source` to `target` as BGRA.
fn copy_rows(
    source: &[u8],
    source_stride: usize,
    target: &mut [u8],
    target_stride: usize,
    pixels: &Pixels,
) {
    let row = pixels.width() as usize * 4;
    for y in 0..pixels.height() as usize {
        let from = &source[y * source_stride..][..row];
        let to = &mut target[y * target_stride..][..row];
        match pixels.format() {
            PixelFormat::Bgra8 => to.copy_from_slice(from),
            PixelFormat::Rgba8 => {
                let (to, from) = (to.as_chunks_mut::<4>().0, from.as_chunks::<4>().0);
                for (to, from) in to.iter_mut().zip(from) {
                    *to = [from[2], from[1], from[0], from[3]];
                }
            }
        }
    }
}

fn frame_error(message: impl Into<String>) -> TextureError {
    TextureError::Frame(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_copied_as_bgra() {
        // Two rows of two pixels, with padding.
        let source = [
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let mut target = [0xff; 24];
        // SAFETY: `source` holds both rows and outlives `pixels`.
        let rgba = unsafe { Pixels::new(source.as_ptr(), 2, 2, 10, PixelFormat::Rgba8) };
        copy_rows(&source, 10, &mut target, 12, &rgba);
        assert_eq!(
            target,
            [
                3, 2, 1, 4, 7, 6, 5, 8, 0xff, 0xff, 0xff, 0xff, //
                11, 10, 9, 12, 15, 14, 13, 16, 0xff, 0xff, 0xff, 0xff,
            ]
        );

        // SAFETY: as above.
        let bgra = unsafe { Pixels::new(source.as_ptr(), 2, 2, 10, PixelFormat::Bgra8) };
        copy_rows(&source, 10, &mut target, 12, &bgra);
        assert_eq!(&target[..8], &source[..8]);
        assert_eq!(&target[12..20], &source[10..18]);
    }

    #[test]
    fn pool_follows_the_size() {
        let pixels = [0x80u8; 4 * 4 * 2];
        let mut slot = None;
        // SAFETY: `pixels` holds the rows and outlives the calls.
        let small = unsafe { Pixels::new(pixels.as_ptr(), 4, 2, 16, PixelFormat::Bgra8) };
        let buffer = Pool::copy(&mut slot, &small).unwrap();
        assert_eq!(objc2_core_video::CVPixelBufferGetWidth(&buffer), 4);
        assert_eq!(slot.as_ref().map(|pool| pool.width), Some(4));

        // SAFETY: as above.
        let smaller = unsafe { Pixels::new(pixels.as_ptr(), 2, 2, 16, PixelFormat::Bgra8) };
        let buffer = Pool::copy(&mut slot, &smaller).unwrap();
        assert_eq!(objc2_core_video::CVPixelBufferGetWidth(&buffer), 2);
        assert_eq!(slot.as_ref().map(|pool| pool.width), Some(2));

        // SAFETY: a null pointer is never read.
        let empty = unsafe { Pixels::new(std::ptr::null(), 2, 2, 16, PixelFormat::Bgra8) };
        assert!(Pool::copy(&mut slot, &empty).is_err());
    }
}
