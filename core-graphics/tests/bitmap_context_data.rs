#![cfg(target_os = "macos")]

extern crate core_foundation;
extern crate core_graphics;
extern crate foreign_types;

use core_foundation::base::{CFRelease, CFTypeRef};
use core_foundation::data::{CFDataRef, CFMutableDataRef};
use core_graphics::base::kCGImageAlphaPremultipliedLast;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::sys;
use std::os::raw::c_void;
use std::ptr;

const WIDTH: usize = 16;
const HEIGHT: usize = 8;
const ROW: usize = WIDTH * 4;
const MAGENTA: [u8; 4] = [255, 0, 255, 255];

fn bitmap_context() -> CGContext {
    CGContext::create_bitmap_context(
        WIDTH,
        HEIGHT,
        8,
        0,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedLast,
    )
}

fn fill_magenta(context: &CGContext) {
    context.set_rgb_fill_color(1.0, 0.0, 1.0, 1.0);
    context.fill_rect(CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(WIDTH as f64, HEIGHT as f64),
    ));
}

#[test]
fn a_bitmap_context_is_drawn_into_and_read() {
    let mut context = bitmap_context();
    fill_magenta(&context);
    // SAFETY: the context has no clone, and nothing draws while the slice is alive.
    let data = unsafe { context.data() }.expect("the bytes of a bitmap context");
    assert_eq!(data.len(), HEIGHT * ROW);
    assert!(data.chunks(4).all(|pixel| pixel == MAGENTA));

    // What is written into the bytes is the picture.
    data[..4].copy_from_slice(&[0, 255, 0, 255]);
    let image = context.create_image().expect("an image of the context");
    assert_eq!(image.data().bytes()[..8], [0, 255, 0, 255, 255, 0, 255, 255]);
}

#[test]
fn a_clone_draws_into_the_same_bytes() {
    let mut context = bitmap_context();
    let mut clone = context.clone();
    // SAFETY: neither slice outlives its statement, so the bytes are never borrowed twice.
    let (first, second) = unsafe {
        (
            context.data().expect("bytes").as_ptr(),
            clone.data().expect("bytes").as_ptr(),
        )
    };
    assert_eq!(first, second, "a clone has bytes of its own");

    fill_magenta(&clone);
    drop(clone);
    // SAFETY: the clone is gone, and nothing draws while the slice is alive.
    let data = unsafe { context.data() }.expect("bytes");
    assert_eq!(data[..4], MAGENTA);
}

#[test]
fn a_context_draws_into_storage_the_caller_keeps() {
    let mut storage = vec![0u8; HEIGHT * ROW];
    // SAFETY: `storage` holds `HEIGHT * ROW` bytes, outlives the context, and is not touched
    // while the context lives.
    let context = unsafe {
        CGContext::create_bitmap_context_with_data(
            storage.as_mut_ptr().cast(),
            WIDTH,
            HEIGHT,
            8,
            ROW,
            &CGColorSpace::create_device_rgb(),
            kCGImageAlphaPremultipliedLast,
        )
    };
    assert_eq!(context.bytes_per_row(), ROW);
    fill_magenta(&context);
    drop(context);
    assert!(storage.chunks(4).all(|pixel| pixel == MAGENTA));
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDataConsumerCreateWithCFData(data: CFMutableDataRef) -> *mut c_void;
    fn CGPDFContextCreate(
        consumer: *mut c_void,
        media_box: *const CGRect,
        auxiliary_info: *const c_void,
    ) -> *mut sys::CGContext;
    fn CGContextRelease(context: *mut sys::CGContext);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataCreateMutable(allocator: *const c_void, capacity: isize) -> CFMutableDataRef;
}

#[test]
fn a_context_that_is_no_bitmap_has_no_bytes() {
    let media_box = CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(16.0, 8.0));
    // SAFETY: the PDF context is made over a data consumer that writes into a mutable data
    // object; each object is released once, after the wrapper took a reference of its own.
    let mut context = unsafe {
        let data = CFDataCreateMutable(ptr::null(), 0);
        let consumer = CGDataConsumerCreateWithCFData(data);
        let pdf = CGPDFContextCreate(consumer, &media_box, ptr::null());
        assert!(!pdf.is_null());
        let context = CGContext::from_existing_context_ptr(pdf);
        CGContextRelease(pdf);
        CFRelease(consumer as CFTypeRef);
        CFRelease(data as CFDataRef as CFTypeRef);
        context
    };
    // SAFETY: the context has no clone and nothing else looks at it.
    assert!(unsafe { context.data() }.is_none());
}
