#[swift_bridge::bridge]
pub mod apple_bridge {
    extern "Swift" {
        fn bundle_id() -> String;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(target_os = "macos")]
    fn test_bundle_id() {
        use crate::apple_bridge::apple_bridge::bundle_id;
        let id = bundle_id();
        println!("Bundle ID: {}", id);
        // assert!(!id.is_empty(), "Bundle ID should not be empty");
        // assert!(
        //     id.contains("."),
        //     "Bundle ID should contain at least one dot"
        // );
    }
}
