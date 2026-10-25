//! Compiles the scenes of `scenes/` into the program: one `include_bytes!` entry for every file
//! of every scene folder, in a sorted order so that the build does not depend on the file
//! system. Hidden files and folders (caches, `.gitignore`) and a scene's `tools/` are left out.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The files below `dir` as (path relative to the scene, absolute path), sorted.
fn files(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", dir.display()))
        .map(|entry| entry.expect("Failed to read a directory entry"))
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        // The scripts that made a scene's assets stay in the repository but not in the program.
        if name.starts_with('.') || (prefix.is_empty() && name == "tools") {
            continue;
        }
        let relative = format!("{prefix}{name}");
        let path = entry.path();
        if path.is_dir() {
            files(&path, &format!("{relative}/"), out);
        } else {
            out.push((relative, path));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=scenes");
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("scenes");
    let mut folders: Vec<_> = std::fs::read_dir(&root)
        .expect("Failed to read scenes/")
        .map(|entry| entry.expect("Failed to read a directory entry").path())
        .filter(|path| path.join("scene.toml").is_file())
        .collect();
    folders.sort();

    let mut code = String::from("pub static SCENES: &[(&str, Files)] = &[\n");
    for folder in &folders {
        let name = folder.file_name().unwrap().to_string_lossy();
        let mut list = Vec::new();
        files(folder, "", &mut list);
        writeln!(code, "    ({name:?}, &[").unwrap();
        for (relative, path) in list {
            writeln!(
                code,
                "        ({relative:?}, include_bytes!({:?})),",
                path.display().to_string()
            )
            .unwrap();
        }
        code.push_str("    ]),\n");
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("builtin_scenes.rs");
    std::fs::write(out, code).expect("Failed to write the table of scenes");
}
