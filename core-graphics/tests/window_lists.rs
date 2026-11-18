#![cfg(target_os = "macos")]

extern crate core_foundation;
extern crate core_graphics;

use core_foundation::{
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_graphics::window::*;
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};

#[test]
#[ignore = "requires Xcode and an interactive macOS desktop"]
fn window_lists_select_the_expected_windows() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/window_server.swift");
    let mut child = Command::new("xcrun")
        .arg("swift")
        .arg(fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("launch the AppKit fixture");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .expect("read the controlled window IDs");
    let ids: Vec<CGWindowID> = line
        .split_whitespace()
        .map(|id| id.parse().unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "AppKit fixture did not report its windows");
    let (back, front, desktop) = (ids[0], ids[1], ids[2]);

    let check = |contains: &dyn Fn(CGWindowListOption, CGWindowID, CGWindowID) -> bool| {
        assert!(contains(kCGWindowListOptionAll, kCGNullWindowID, desktop));
        assert!(contains(
            kCGWindowListOptionOnScreenOnly,
            kCGNullWindowID,
            front
        ));
        assert!(contains(
            kCGWindowListOptionOnScreenAboveWindow,
            back,
            front
        ));
        assert!(!contains(
            kCGWindowListOptionOnScreenAboveWindow,
            back,
            back
        ));
        assert!(contains(
            kCGWindowListOptionOnScreenBelowWindow,
            front,
            back
        ));
        assert!(contains(
            kCGWindowListOptionOnScreenAboveWindow | kCGWindowListOptionIncludingWindow,
            back,
            back,
        ));
        assert!(!contains(
            kCGWindowListOptionExcludeDesktopElements,
            kCGNullWindowID,
            desktop
        ));
        assert!(contains(
            kCGWindowListOptionExcludeDesktopElements,
            kCGNullWindowID,
            front
        ));
    };

    check(&contains_window_info);
    check(&|options, reference, window| {
        create_window_list(options, reference)
            .expect("window ID list unavailable")
            .get_all_values()
            .iter()
            .any(|&value| value as usize == window as usize)
    });

    drop(child.stdin.take());
    assert!(child.wait().expect("close AppKit fixture").success());
}

fn contains_window_info(
    options: CGWindowListOption,
    reference: CGWindowID,
    window: CGWindowID,
) -> bool {
    let list = copy_window_info(options, reference).expect("window info list unavailable");
    let key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    list.get_all_values().into_iter().any(|entry| {
        let entry = unsafe { CFType::wrap_under_get_rule(entry as _) };
        let entry = entry
            .downcast::<CFDictionary>()
            .expect("window info dictionary");
        let entry = unsafe {
            CFDictionary::<CFString, CFNumber>::wrap_under_get_rule(entry.as_concrete_TypeRef())
        };
        entry.find(&key).and_then(|number| number.to_i64()) == Some(i64::from(window))
    })
}
