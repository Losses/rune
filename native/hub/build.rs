use anyhow::Result;
use std::env;
use std::path::Path;
use std::process::Command;
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
    emit_git_commit();
    emit_app_version()?;

    let target = std::env::var("TARGET").unwrap();

    if target.contains("darwin") {
        #[cfg(target_os = "macos")]
        build_apple_bridge()?;
    }

    Ok(())
}

/// Hands the hub the version the app declares in its manifest, so devices announce the version
/// users see rather than this crate's own.
fn emit_app_version() -> Result<()> {
    let manifest = Path::new(&env::var("CARGO_MANIFEST_DIR")?).join("../../pubspec.yaml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest)?;
    let version = text
        .lines()
        .find_map(|line| line.strip_prefix("version:"))
        .map(|value| value.trim().split('+').next().unwrap_or_default().trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("pubspec.yaml has no version"))?;
    println!("cargo:rustc-env=RUNE_APP_VERSION={version}");
    Ok(())
}

/// Hands the About page the commit the build is made from, in the variables vergen's git feature
/// sets. Without git, or outside a checkout (a source archive, a nix build), they stay unset.
fn emit_git_commit() {
    let git = |args: &[&str]| -> Option<String> {
        let output = Command::new("git").args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
    };
    // Run again when HEAD moves: a commit, a checkout, the branch packed into packed-refs or a
    // reftable update. Watched before HEAD is read, so a repository's first commit is picked up too.
    for name in ["HEAD", "packed-refs", "reftable/tables.list"] {
        let path = git(&["rev-parse", "--path-format=absolute", "--git-path", name]);
        if let Some(path) = path.filter(|path| Path::new(path).exists()) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    // A branch kept only in packed-refs has no file of its own until a commit writes one, so
    // watch the nearest folder that exists, up to refs/heads: Cargo also reruns for a file created
    // inside it.
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", &branch])
        && let Some(path) = Path::new(&path)
            .ancestors()
            .take(branch.split('/').count() - 1)
            .find(|path| path.exists())
    {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    // One read, so the hash and the date come from the same commit.
    let Some(commit) = git(&["log", "-1", "--format=%H %cI"]) else {
        return;
    };
    let Some((sha, timestamp)) = commit.split_once(' ') else {
        return;
    };
    println!("cargo:rustc-env=VERGEN_GIT_SHA={sha}");
    println!("cargo:rustc-env=VERGEN_GIT_COMMIT_TIMESTAMP={timestamp}");
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
