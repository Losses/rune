#![cfg(target_os = "macos")]

extern crate core_graphics;

use core_graphics::{display, window};

#[test]
fn window_list_options_match_the_sdk_and_display_module() {
    let window_options = [
        window::kCGWindowListOptionAll,
        window::kCGWindowListOptionOnScreenOnly,
        window::kCGWindowListOptionOnScreenAboveWindow,
        window::kCGWindowListOptionOnScreenBelowWindow,
        window::kCGWindowListOptionIncludingWindow,
        window::kCGWindowListOptionExcludeDesktopElements,
    ];

    // CoreGraphics/CGWindow.h
    assert_eq!(window_options, [0, 1 << 0, 1 << 1, 1 << 2, 1 << 3, 1 << 4]);
    assert_eq!(
        window_options,
        [
            display::kCGWindowListOptionAll,
            display::kCGWindowListOptionOnScreenOnly,
            display::kCGWindowListOptionOnScreenAboveWindow,
            display::kCGWindowListOptionOnScreenBelowWindow,
            display::kCGWindowListOptionIncludingWindow,
            display::kCGWindowListExcludeDesktopElements,
        ]
    );
}

#[test]
fn window_sharing_values_match_the_sdk() {
    // CoreGraphics/CGWindow.h
    assert_eq!(window::kCGWindowSharingNone, 0);
    assert_eq!(window::kCGWindowSharingReadOnly, 1);
    assert_eq!(window::kCGWindowSharingReadWrite, 2);
}
