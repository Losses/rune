use swift_rs::{swift, SRString};

#[cfg(target_os = "macos")]
swift!(fn bundle_id() -> SRString);

#[cfg(target_os = "macos")]
pub fn get_bundle_id() -> String {
    unsafe { bundle_id().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_id() {
        let bundle_id = get_bundle_id();

        println!("Bundle ID: {}", bundle_id);
    }
}
