use anyhow::Result;
use std::env;
use vergen_git2::{BuildBuilder, Emitter, Git2Builder, RustcBuilder};

fn main() -> Result<()> {
    let target_os = env::var("CARGO_CFG_TARGET_OS");
    if let Ok("android") = target_os.as_ref().map(|x| &**x) {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=c++_shared");
    }

    let git2 = Git2Builder::all_git()?;
    let build = BuildBuilder::all_build()?;
    let rustc = RustcBuilder::all_rustc()?;

    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&git2)?
        .add_instructions(&rustc)?
        .emit()?;

    #[cfg(target_os = "macos")]
    apple_bridge::build_apple_bridge_library();

    Ok(())
}

#[cfg(target_os = "macos")]
pub mod apple_bridge {
    use std::path::PathBuf;
    use std::process::Command;

    pub fn build_apple_bridge_library() {
        println!("cargo:rerun-if-changed=apple-bridge-library/");

        // 1. Use `swift-bridge-build` to generate Swift/C FFI glue.
        //    You can also use the `swift-bridge` CLI.
        let bridge_files = vec!["src/apple_bridge/mod.rs"];
        swift_bridge_build::parse_bridges(bridge_files)
            .write_all_concatenated(swift_bridge_out_dir(), "rust-calls-swift");

        // 2. Compile Swift library
        compile_swift();

        // 3. Link to Swift library
        println!("cargo:rustc-link-lib=static=apple-bridge-library");
        println!(
            "cargo:rustc-link-search={}",
            swift_library_static_lib_dir().to_str().unwrap()
        );
        println!("cargo:rustc-link-search=native=apple-bridge-library/.build/debug");
        println!("cargo:rustc-link-search=native=apple-bridge-library/.build/release");

        // Without this we will get warnings about not being able to find dynamic libraries, and then
        // we won't be able to compile since the Swift static libraries depend on them:
        // For example:
        // ld: warning: Could not find or use auto-linked library 'swiftCompatibility51'
        // ld: warning: Could not find or use auto-linked library 'swiftCompatibility50'
        // ld: warning: Could not find or use auto-linked library 'swiftCompatibilityDynamicReplacements'
        // ld: warning: Could not find or use auto-linked library 'swiftCompatibilityConcurrency'
        let xcode_path = if let Ok(output) = std::process::Command::new("xcode-select")
            .arg("--print-path")
            .output()
        {
            String::from_utf8(output.stdout.as_slice().into())
                .unwrap()
                .trim()
                .to_string()
        } else {
            "/Applications/Xcode.app/Contents/Developer".to_string()
        };
        println!(
            "cargo:rustc-link-search={}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx/",
            &xcode_path
        );
        println!("cargo:rustc-link-search=/usr/lib/swift");
    }

    fn compile_swift() {
        let debug_or_release = if is_release_build() {
            "release"
        } else {
            "debug"
        };

        let swift_package_dir = manifest_dir().join("apple-bridge-library");
        let build_script = swift_package_dir.join("build.sh");

        let status = Command::new("sh")
            .arg(build_script)
            .arg(debug_or_release)
            .current_dir(&swift_package_dir)
            .status()
            .expect("Failed to execute build.sh");

        if !status.success() {
            panic!("build.sh failed");
        }
    }

    fn swift_bridge_out_dir() -> PathBuf {
        generated_code_dir()
    }

    fn manifest_dir() -> PathBuf {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        PathBuf::from(manifest_dir)
    }

    fn is_release_build() -> bool {
        std::env::var("PROFILE").unwrap() == "release"
    }

    fn swift_source_dir() -> PathBuf {
        manifest_dir().join("apple-bridge-library/Sources/apple-bridge-library")
    }

    fn generated_code_dir() -> PathBuf {
        swift_source_dir().join("generated")
    }

    fn swift_library_static_lib_dir() -> PathBuf {
        manifest_dir().join("apple-bridge-library/lib")
    }
}
