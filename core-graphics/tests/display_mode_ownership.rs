#![cfg(target_os = "macos")]

extern crate core_foundation;
extern crate core_graphics;
extern crate foreign_types;

use core_foundation::array::{CFArray, CFArrayGetCount, CFArrayGetValueAtIndex};
use core_foundation::base::{CFGetRetainCount, CFIndex, CFType, TCFType};
use core_graphics::display::{
    CGDirectDisplayID, CGDisplayCopyAllDisplayModes, CGDisplayMode, CGMainDisplayID,
};
use foreign_types::ForeignType;
use std::{mem, ptr};

/// The modes of the main display, in an array this test owns. The system hands out the same mode
/// objects on every enumeration, so while this array lives a mode's retain count can be read
/// whatever the wrappers under test do with their references.
fn native_modes() -> (CGDirectDisplayID, CFArray<CFType>) {
    let display = unsafe { CGMainDisplayID() };
    let array = unsafe { CGDisplayCopyAllDisplayModes(display, ptr::null()) };
    assert!(!array.is_null(), "this test needs a display with modes");
    let array = unsafe { CFArray::wrap_under_create_rule(array) };
    assert!(!array.is_empty(), "this test needs a display with modes");
    (display, array)
}

fn retain_counts(array: &CFArray<CFType>) -> Vec<CFIndex> {
    let array = array.as_concrete_TypeRef();
    (0..unsafe { CFArrayGetCount(array) })
        .map(|index| unsafe { CFGetRetainCount(CFArrayGetValueAtIndex(array, index)) })
        .collect()
}

fn raised_by(counts: &[CFIndex], references: CFIndex) -> Vec<CFIndex> {
    counts.iter().map(|count| count + references).collect()
}

#[test]
fn each_enumerated_mode_owns_one_reference_of_its_own() {
    let (display, native) = native_modes();
    let before = retain_counts(&native);

    let modes = CGDisplayMode::all_display_modes(display, ptr::null())
        .expect("the display that listed its modes natively lists them here");
    // The array the enumeration copied is released by now: what the wrappers hold is theirs.
    let held = retain_counts(&native);
    if held != raised_by(&before, 1) {
        // Dropping wrappers that own nothing would release the references of someone else.
        mem::forget(modes);
        panic!(
            "the returned modes hold no reference of their own: retain counts {:?} before the \
             enumeration and {:?} after it",
            before, held
        );
    }

    let native_pointers: Vec<_> = (0..native.len())
        .map(|index| unsafe { CFArrayGetValueAtIndex(native.as_concrete_TypeRef(), index) })
        .collect();
    let returned_pointers: Vec<_> = modes.iter().map(|mode| mode.as_ptr().cast_const().cast()).collect();
    assert_eq!(returned_pointers, native_pointers);
    for mode in &modes {
        assert!(mode.width() > 0 && mode.height() > 0);
    }

    let clones = modes.clone();
    assert_eq!(retain_counts(&native), raised_by(&before, 2));
    drop(modes);
    assert_eq!(retain_counts(&native), raised_by(&before, 1));
    for mode in &clones {
        assert!(mode.width() > 0 && mode.height() > 0);
    }
    drop(clones);
    assert_eq!(retain_counts(&native), before);
}

#[test]
fn a_display_without_modes_lists_none() {
    // No display has this ID: IDs are small numbers the window server hands out.
    assert!(CGDisplayMode::all_display_modes(0xdead_beef, ptr::null()).is_none());
}
