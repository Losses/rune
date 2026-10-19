//! Opt-in built-in preset pack: when `RUNE_BUILTIN_PRESETS` names a zip file, it is embedded
//! into the library (see `BUILTIN_PRESETS.md`). Without it nothing is embedded.

use std::path::{Path, PathBuf};

const ENV: &str = "RUNE_BUILTIN_PRESETS";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(builtin_presets)");
    println!("cargo:rerun-if-env-changed={ENV}");
    println!("cargo:rerun-if-changed=build.rs");

    let Some(path) = std::env::var_os(ENV).map(PathBuf::from) else {
        return;
    };
    if path.as_os_str().is_empty() {
        return;
    }
    if !Path::new(&path).is_file() {
        println!(
            "cargo:warning={ENV}={} is not a file; building without the built-in presets",
            path.display()
        );
        return;
    }
    println!("cargo:rerun-if-changed={}", path.display());
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("builtin.zip");
    std::fs::copy(&path, &out).expect("failed to copy the built-in preset pack");
    println!("cargo:rustc-cfg=builtin_presets");
}
