use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=native");
    for variable in [
        "ANDROID_NDK_ROOT",
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_PATH",
        "ANDROID_PLATFORM",
        "SDKROOT",
        "MACOSX_DEPLOYMENT_TARGET",
        "IPHONEOS_DEPLOYMENT_TARGET",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo supplies the target OS");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo supplies the target architecture");
    let target = env::var("TARGET").expect("Cargo supplies the target triple");
    let mut native = cmake::Config::new("native");
    native
        .profile("Release")
        .define("CMAKE_INSTALL_LIBDIR", "lib");

    // Package builders can provide pinned sources for an offline CMake build.
    for variable in [
        "FETCHCONTENT_SOURCE_DIR_UTFCPP",
        "FETCHCONTENT_SOURCE_DIR_TAGLIB",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
        native.define(variable, env::var_os(variable).unwrap_or_default());
    }

    // Use the NDK toolchain for Android even when the host is also compiling desktop artifacts.
    if os == "android" {
        let ndk = ["ANDROID_NDK_ROOT", "ANDROID_NDK_HOME", "ANDROID_NDK_PATH"]
            .into_iter()
            .find_map(env::var_os)
            .map(PathBuf::from)
            .expect("set ANDROID_NDK_ROOT to the Android NDK directory");
        let abi = match arch.as_str() {
            "arm" => "armeabi-v7a",
            "aarch64" => "arm64-v8a",
            "x86" => "x86",
            "x86_64" => "x86_64",
            _ => panic!("unsupported Android architecture {arch}"),
        };

        // Use the shared C++ runtime packaged by the app and the NDK's linker defaults.
        native
            .define(
                "CMAKE_TOOLCHAIN_FILE",
                ndk.join("build/cmake/android.toolchain.cmake"),
            )
            .define("ANDROID_ABI", abi)
            .define(
                "ANDROID_PLATFORM",
                env::var("ANDROID_PLATFORM").unwrap_or_else(|_| "android-21".into()),
            )
            .define("ANDROID_STL", "c++_shared");
    } else if matches!(os.as_str(), "ios" | "macos") {
        // Specify the Apple SDK independently of the host's active Xcode target.
        let (sdk, deployment, minimum) = match os.as_str() {
            "ios" if arch == "x86_64" || target.ends_with("-sim") => {
                ("iphonesimulator", "IPHONEOS_DEPLOYMENT_TARGET", "15.0")
            }
            "ios" => ("iphoneos", "IPHONEOS_DEPLOYMENT_TARGET", "15.0"),
            _ => ("macosx", "MACOSX_DEPLOYMENT_TARGET", "12.0"),
        };
        native
            .define(
                "CMAKE_SYSTEM_NAME",
                if os == "ios" { "iOS" } else { "Darwin" },
            )
            .define("CMAKE_OSX_SYSROOT", sdk)
            .define(
                "CMAKE_OSX_ARCHITECTURES",
                if arch == "aarch64" { "arm64" } else { "x86_64" },
            )
            .define(
                "CMAKE_OSX_DEPLOYMENT_TARGET",
                env::var(deployment).unwrap_or_else(|_| minimum.into()),
            );
    }

    // Link TagLib and the bridge into the Rust library for every target.
    let installed = native.build();
    println!(
        "cargo:rustc-link-search=native={}",
        installed.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=rune_taglib");
    println!("cargo:rustc-link-lib=static=tag");
    match os.as_str() {
        "android" => println!("cargo:rustc-link-lib=dylib=c++_shared"),
        "macos" | "ios" => println!("cargo:rustc-link-lib=c++"),
        _ if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") => (),
        _ => println!("cargo:rustc-link-lib=stdc++"),
    }
}
