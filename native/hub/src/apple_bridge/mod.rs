#[cfg(target_os = "macos")]
use std::ffi::{CStr, c_char};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn rune_bundle_id() -> *mut c_char;
    fn rune_free_bundle_id(pointer: *mut c_char);
}

#[cfg(target_os = "macos")]
pub fn get_bundle_id() -> String {
    // SAFETY: Swift returns an owned, non-null, NUL-terminated UTF-8 allocation.
    // Copy it before releasing it through the same Swift allocator.
    unsafe {
        let pointer = rune_bundle_id();
        let bundle_id = CStr::from_ptr(pointer).to_string_lossy().into_owned();
        rune_free_bundle_id(pointer);
        bundle_id
    }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_id() {
        let bundle_id = get_bundle_id();

        println!("Bundle ID: {}", { bundle_id });
    }
}
