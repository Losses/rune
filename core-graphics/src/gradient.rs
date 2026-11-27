// Copyright 2013 The Servo Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(non_upper_case_globals)]

use base::CGFloat;
use color::CGColor;
use color_space::CGColorSpace;

use core_foundation::array::{ CFArray, CFArrayRef };
use core_foundation::base::{CFRelease, CFRetain, TCFType};
use foreign_types::ForeignType;

use libc::size_t;
use std::ptr;

bitflags! {
    #[repr(C)]
    pub struct CGGradientDrawingOptions: u32 {
        const CGGradientDrawsBeforeStartLocation = (1 << 0);
        const CGGradientDrawsAfterEndLocation = (1 << 1);
    }
}

foreign_type! {
    #[doc(hidden)]
    type CType = ::sys::CGGradient;
    fn drop = |p| CFRelease(p as *mut _);
    fn clone = |p| CFRetain(p as *const _) as *mut _;
    pub struct CGGradient;
    pub struct CGGradientRef;
}

impl CGGradient {
    /// A gradient of `count` stops. `components` holds for each stop the components of a color of
    /// `color_space` and then its alpha. `locations` holds the place of each stop, from 0 to 1;
    /// when it is empty the stops are spread evenly.
    ///
    /// # Panics
    ///
    /// If `components` or a `locations` that is not empty is shorter than `count` stops need, or
    /// if Core Graphics makes no gradient, as it does of no stops.
    pub fn create_with_color_components(color_space: &CGColorSpace, components: &[CGFloat], locations: &[CGFloat], count: usize) -> CGGradient {
        let per_stop = color_space.number_of_components() + 1;
        let needed = count.checked_mul(per_stop).expect("the components of that many stops cannot be counted");
        assert!(components.len() >= needed, "{} stops need {} components, got {}", count, needed, components.len());
        let locations = locations_of(locations, count);
        unsafe {
            let result = CGGradientCreateWithColorComponents(color_space.as_ptr(), components.as_ptr(), locations, count);
            assert!(!result.is_null());
            Self::from_ptr(result)
        }
    }

    /// A gradient with one stop for each of `colors`. `locations` holds the place of each stop,
    /// from 0 to 1; when it is empty the stops are spread evenly.
    ///
    /// # Panics
    ///
    /// If a `locations` that is not empty is shorter than `colors`, or if Core Graphics makes no
    /// gradient, as it does of no colors.
    pub fn create_with_colors(color_space: &CGColorSpace, colors: &CFArray<CGColor>, locations: &[CGFloat]) -> CGGradient {
        let locations = locations_of(locations, colors.len() as usize);
        unsafe {
            let result = CGGradientCreateWithColors(color_space.as_ptr(), colors.as_concrete_TypeRef(), locations);
            assert!(!result.is_null());
            Self::from_ptr(result)
        }
    }
}

/// What Core Graphics is given for the places of `stops` stops: it reads one of `locations` for
/// each stop, and spreads the stops evenly when it is given none.
fn locations_of(locations: &[CGFloat], stops: usize) -> *const CGFloat {
    if locations.is_empty() {
        return ptr::null();
    }
    assert!(locations.len() >= stops, "{} stops need {} locations, got {}", stops, stops, locations.len());
    locations.as_ptr()
}

#[link(name = "CoreGraphics", kind = "framework")]
extern {
    fn CGGradientCreateWithColorComponents(color_space: ::sys::CGColorSpaceRef, components: *const CGFloat, locations: *const CGFloat, count: size_t) -> ::sys::CGGradientRef;
    fn CGGradientCreateWithColors(color_space: ::sys::CGColorSpaceRef, colors: CFArrayRef, locations: *const CGFloat) -> ::sys::CGGradientRef;
}

