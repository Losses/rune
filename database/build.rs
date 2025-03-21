use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let target = env::var("TARGET").expect("Cannot get TARGET");
    println!("database::crsqlite TARGET: {}", target);
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    println!("database::crsqlite PROFILE: {}", profile);
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    println!("database::crsqlite OUT_DIR: {:#?}", out_dir);
    let target_dir = out_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    println!("database::crsqlite target_dir: {:#?}", target_dir);

    let (source_path, lib_name) = match target.as_str() {
        // Android
        "aarch64-linux-android" => ("aarch64-linux-android/crsqlite.so", "crsqlite.so"),

        // macOS
        "aarch64-apple-darwin" => ("aarch64-apple-darwin/crsqlite.dylib", "crsqlite.dylib"),
        "x86_64-apple-darwin" => ("x86_64-apple-darwin/crsqlite.dylib", "crsqlite.dylib"),

        // Linux
        "aarch64-unknown-linux-gnu" => ("aarch64-unknown-linux-gnu/crsqlite.so", "crsqlite.so"),
        "x86_64-unknown-linux-gnu" => ("x86_64-unknown-linux-gnu/crsqlite.so", "crsqlite.so"),

        // Windows
        "i686-pc-windows-msvc" => ("i686-pc-windows-msvc/crsqlite.dll", "crsqlite.dll"),
        "x86_64-pc-windows-msvc" => ("x86_64-pc-windows-msvc/crsqlite.dll", "crsqlite.dll"),

        _ => panic!("Unsupported target: {}", target),
    };

    let src_path = Path::new("resources/crsqlite").join(source_path);

    println!("database::crsqlite src_path: {:#?}", src_path);

    fs::create_dir_all(target_dir).expect("Failed to create dest dir");

    fs::copy(&src_path, target_dir.join(lib_name))
        .unwrap_or_else(|_| panic!("Failed to copy {:?} to {:?}", src_path, target_dir));

    println!("cargo:rustc-env=CRSQLITE_LIB_PATH={}", target_dir.display());
}
