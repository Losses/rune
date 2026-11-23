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

    // Use the NDK toolchain for Android even when the host is also compiling desktop artifacts.
    if os == "android" {
        let ndk = ["ANDROID_NDK_ROOT", "ANDROID_NDK_HOME", "ANDROID_NDK_PATH"]
            .into_iter()
            .find_map(env::var_os)
            .map(PathBuf::from)
            .expect("set ANDROID_NDK_ROOT to the Android NDK directory");
        let (abi, sysroot_arch) = match arch.as_str() {
            "arm" => ("armeabi-v7a", "arm-linux-androideabi"),
            "aarch64" => ("arm64-v8a", "aarch64-linux-android"),
            "x86" => ("x86", "i686-linux-android"),
            "x86_64" => ("x86_64", "x86_64-linux-android"),
            _ => panic!("unsupported Android architecture {arch}"),
        };
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
            .define("ANDROID_STL", "c++_static");
        let prebuilt = ndk.join("toolchains/llvm/prebuilt");
        let runtime = std::fs::read_dir(prebuilt)
            .expect("read NDK toolchains")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("sysroot/usr/lib").join(sysroot_arch))
            .find(|path| path.join("libc++_static.a").is_file())
            .expect("find the Android C++ runtime for the target");
        println!("cargo:rustc-link-search=native={}", runtime.display());
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
        "android" => {
            println!("cargo:rustc-link-lib=static=c++_static");
            println!("cargo:rustc-link-lib=static=c++abi");
        }
        "macos" | "ios" => println!("cargo:rustc-link-lib=c++"),
        _ if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") => (),
        _ => println!("cargo:rustc-link-lib=stdc++"),
    }
}
