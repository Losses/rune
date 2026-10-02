use anyhow::Result;
use std::env;
#[cfg(target_os = "macos")]
use swift_rs::SwiftLinker;
use vergen::{BuildBuilder, Emitter, RustcBuilder};

fn main() -> Result<()> {
    let target_os = env::var("CARGO_CFG_TARGET_OS");
    if let Ok("android") = target_os.as_ref().map(|x| &**x) {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=c++_shared");
    }

    let build = BuildBuilder::all_build()?;
    let rustc = RustcBuilder::all_rustc()?;

    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&rustc)?
        .emit()?;

    let target = std::env::var("TARGET").unwrap();

    if target.contains("darwin") {
        #[cfg(target_os = "macos")]
        build_apple_bridge()?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn build_apple_bridge() -> Result<()> {
    use anyhow::{Context, ensure};
    use std::{path::PathBuf, process::Command};

    let rust_arch = env::var("CARGO_CFG_TARGET_ARCH")?;
    let arch = match rust_arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => anyhow::bail!("unsupported macOS architecture: {rust_arch}"),
    };
    let target = format!("{arch}-apple-macosx12.0");
    let output_dir = PathBuf::from(env::var("OUT_DIR")?);
    let archive = output_dir.join("libapple-bridge-library.a");
    let source = "apple-bridge-library/src/lib.swift";

    let sdk = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .context("failed to locate the macOS SDK")?;
    ensure!(sdk.status.success(), "xcrun could not locate the macOS SDK");
    let sdk_path = String::from_utf8(sdk.stdout)?;

    // swift-rs passes the host --arch to SwiftPM on macOS. Xcode 27
    // overrides its -Xswiftc target, producing arm64 even for an Intel build.
    // Compile this dependency-free bridge directly with Cargo's target instead.
    let mut compiler = Command::new("xcrun");
    compiler.args([
        "--sdk",
        "macosx",
        "swiftc",
        "-parse-as-library",
        "-emit-library",
        "-static",
        "-swift-version",
        "6",
        "-module-name",
        "apple_bridge_library",
        "-target",
        &target,
        "-sdk",
        sdk_path.trim(),
    ]);
    compiler.arg(if env::var("OPT_LEVEL").as_deref() == Ok("0") {
        "-Onone"
    } else {
        "-O"
    });
    if env::var("DEBUG").as_deref() == Ok("true") {
        compiler.arg("-g");
    }
    compiler.arg(source).arg("-o").arg(&archive);
    ensure!(
        compiler.status()?.success(),
        "Swift bridge build failed for {target}"
    );

    // Keep the runtime and compiler support library search paths supplied by swift-rs.
    SwiftLinker::new("12.0").link();
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rustc-link-search=native={}", output_dir.display());
    println!("cargo:rustc-link-lib=static=apple-bridge-library");
    Ok(())
}
