// Copyright 2017 The Servo Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

pub use sys::CGPathRef as SysCGPathRef;

use core_foundation::base::{CFRelease, CFRetain, CFTypeID};
use foreign_types::ForeignType;
use geometry::{CGAffineTransform, CGPoint, CGRect};
use libc::c_void;
use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::slice;

foreign_type! {
    #[doc(hidden)]
    type CType = ::sys::CGPath;
    fn drop = |p| CFRelease(p as *mut _);
    fn clone = |p| CFRetain(p as *const _) as *mut _;
    pub struct CGPath;
    pub struct CGPathRef;
}

impl CGPath {
    pub fn from_rect(rect: CGRect, transform: Option<&CGAffineTransform>) -> CGPath {
        unsafe {
            let transform = match transform {
                None => ptr::null(),
                Some(transform) => transform as *const CGAffineTransform,
            };
            CGPath(CGPathCreateWithRect(rect, transform))
        }
    }

    pub fn type_id() -> CFTypeID {
        unsafe {
            CGPathGetTypeID()
        }
    }

    /// Calls `closure` once for each element in the path.
    ///
    /// Core Graphics passes a `const CGPathElement *` to the applier and does not define an
    /// ownership or retain operation for that pointer. This wrapper treats each element as a
    /// callback-scoped borrow. Copy element data that must outlive the callback.
    ///
    /// A closure must have exclusive mutable access:
    ///
    /// ```compile_fail
    /// extern crate core_graphics;
    ///
    /// use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    /// use core_graphics::path::{CGPath, CGPathElementRef};
    ///
    /// let path = CGPath::from_rect(
    ///     CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(10.0, 10.0)),
    ///     None,
    /// );
    /// let closure = |_element: CGPathElementRef<'_>| {};
    /// path.apply(&closure);
    /// ```
    ///
    /// Element references cannot be retained after the callback:
    ///
    /// ```compile_fail
    /// extern crate core_graphics;
    ///
    /// use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    /// use core_graphics::path::{CGPath, CGPathElementRef};
    ///
    /// let path = Box::leak(Box::new(CGPath::from_rect(
    ///     CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(10.0, 10.0)),
    ///     None,
    /// )));
    /// let retained = Box::leak(Box::new(None));
    /// let closure = Box::leak(Box::new(move |element: CGPathElementRef<'static>| {
    ///     *retained = Some(element);
    /// }));
    /// path.apply(closure);
    /// ```
    pub fn apply<F>(&self, closure: &mut F) where F: for<'a> FnMut(CGPathElementRef<'a>) {
        unsafe {
            CGPathApply(self.as_ptr(), closure as *mut F as *mut c_void, do_apply::<F>);
        }

        unsafe extern "C" fn do_apply<F>(info: *mut c_void, element: *const CGPathElement)
                                             where F: for<'a> FnMut(CGPathElementRef<'a>) {
            let closure = &mut *(info as *mut F);
            closure(CGPathElementRef::new(element))
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CGPathElementType {
    MoveToPoint = 0,
    AddLineToPoint = 1,
    AddQuadCurveToPoint = 2,
    AddCurveToPoint = 3,
    CloseSubpath = 4,
}

pub struct CGPathElementRef<'a> {
    element: *const CGPathElement,
    phantom: PhantomData<&'a CGPathElement>,
}

impl<'a> CGPathElementRef<'a> {
    fn new<'b>(element: *const CGPathElement) -> CGPathElementRef<'b> {
        CGPathElementRef {
            element: element,
            phantom: PhantomData,
        }
    }
}

impl<'a> Deref for CGPathElementRef<'a> {
    type Target = CGPathElement;
    fn deref(&self) -> &CGPathElement {
        unsafe {
            &*self.element
        }
    }
}

#[repr(C)]
pub struct CGPathElement {
    pub element_type: CGPathElementType,
    points: *mut CGPoint,
}

impl Debug for CGPathElement {
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "{:?}: {:?}", self.element_type, self.points())
    }
}

impl CGPathElement {
    pub fn points(&self) -> &[CGPoint] {
        unsafe {
            match self.element_type {
                CGPathElementType::CloseSubpath => &[],
                CGPathElementType::MoveToPoint | CGPathElementType::AddLineToPoint => {
                    slice::from_raw_parts(self.points, 1)
                }
                CGPathElementType::AddQuadCurveToPoint => slice::from_raw_parts(self.points, 2),
                CGPathElementType::AddCurveToPoint => slice::from_raw_parts(self.points, 3),
            }
        }
    }
}

#[test]
fn test_apply_visits_rect_with_stateful_callback() {
    use geometry::CGSize;

    let path = CGPath::from_rect(
        CGRect::new(&CGPoint::new(1.0, 2.0), &CGSize::new(3.0, 4.0)),
        None,
    );
    let mut visits = 0;
    let mut element_types = Vec::new();
    let mut point_lists = Vec::new();
    let mut closure = |element: CGPathElementRef<'_>| {
        visits += 1;
        element_types.push(element.element_type);
        point_lists.push(
            element.points()
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
        );
    };

    path.apply(&mut closure);

    assert_eq!(visits, 5);
    assert_eq!(element_types, vec![
        CGPathElementType::MoveToPoint,
        CGPathElementType::AddLineToPoint,
        CGPathElementType::AddLineToPoint,
        CGPathElementType::AddLineToPoint,
        CGPathElementType::CloseSubpath,
    ]);
    assert_eq!(point_lists, vec![
        vec![(1.0, 2.0)],
        vec![(4.0, 2.0)],
        vec![(4.0, 6.0)],
        vec![(1.0, 6.0)],
        vec![],
    ]);
}

type CGPathApplierFunction = unsafe extern "C" fn(info: *mut c_void,
                                                  element: *const CGPathElement);

#[link(name = "CoreGraphics", kind = "framework")]
extern {
    fn CGPathCreateWithRect(rect: CGRect, transform: *const CGAffineTransform) -> ::sys::CGPathRef;
    fn CGPathApply(path: ::sys::CGPathRef, info: *mut c_void, function: CGPathApplierFunction);
    fn CGPathGetTypeID() -> CFTypeID;
}
