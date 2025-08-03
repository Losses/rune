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
        SwiftLinker::new("10.13")
            .with_ios("12")
            .with_package("apple-bridge-library", "./apple-bridge-library/")
            .link();
    }

    Ok(())
}
