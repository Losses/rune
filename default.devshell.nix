{ pkgs, masterPkgs, androidPkgs, androidSdk, rust-bin,
  prebuiltOpenSSL
}:

let
  pinnedJDK = pkgs.jdk17;

  # Pinned toolchains. Bump these two lines to upgrade Rust / Flutter;
  # `nix flake update` alone will NOT move them.
  rustVersion = "1.98.1";
  flutterPkg = pkgs.flutter347;

  rustToolchain = rust-bin.stable.${rustVersion}.default.override {
    extensions = [ "rust-src" "rustfmt" "clippy" "rust-analyzer" ];
    targets = [ "armv7-linux-androideabi" "aarch64-linux-android" "i686-linux-android" "x86_64-linux-android" ];
  };

  # cargokit (rinf's build layer) refuses to use a plain cargo: it always
  # locates `rustup` (hard-coding "$HOME/.cargo/bin" ahead of PATH) and
  # builds via `rustup run stable cargo build`, which silently picks up
  # whatever stale toolchain lives in ~/.rustup instead of the Nix one.
  # This shim answers cargokit's queries and delegates the real build to
  # the Nix-provided toolchain above.
  rustupShim = pkgs.writeShellScriptBin "rustup" ''
    case "$1" in
      toolchain)
        case "$2" in
          list) echo "stable-x86_64-unknown-linux-gnu (default)" ;;
          install) echo "rustup-shim: toolchain is managed by Nix, skipping install of '$3'" ;;
          *) echo "rustup-shim: unsupported 'toolchain $2'" >&2; exit 1 ;;
        esac
        ;;
      target)
        case "$2" in
          list)
            printf '%s\n' \
              x86_64-unknown-linux-gnu \
              aarch64-linux-android \
              armv7-linux-androideabi \
              i686-linux-android \
              x86_64-linux-android
            ;;
          add) echo "rustup-shim: target is managed by Nix, skipping add of '$4'" ;;
          *) echo "rustup-shim: unsupported 'target $2'" >&2; exit 1 ;;
        esac
        ;;
      component)
        case "$2" in
          add) : ;;
          *) echo "rustup-shim: unsupported 'component $2'" >&2; exit 1 ;;
        esac
        ;;
      run)
        shift 2
        exec "$@"
        ;;
      --version)
        echo "rustup 1.28.2 (nix-shim, rustc ${rustVersion})"
        ;;
      *)
        echo "rustup-shim: unsupported command '$1'" >&2
        exit 1
        ;;
    esac
  '';

  # NDK setup
  ndkVersion = "27.1.12297006";
  ndkRoot = "${androidSdk}/share/android-sdk/ndk/${ndkVersion}";
  toolchainPath = "${ndkRoot}/toolchains/llvm/prebuilt/linux-x86_64";
  sysrootPath = "${toolchainPath}/sysroot";
  toolchainBinPath = "${toolchainPath}/bin";
  cmakeToolchainFile = "${ndkRoot}/build/cmake/android.toolchain.cmake";

  # Android Build Tools setup for aapt2
  buildToolsVersion = "34.0.0";
  aapt2Path = "${androidSdk}/share/android-sdk/build-tools/${buildToolsVersion}/aapt2";

in
pkgs.mkShell {
  name = "Rune Development Shell";

  buildInputs = with pkgs; [
    rustupShim
    rustToolchain
    yq
    openssl
    pkg-config
    flutterPkg
    android-studio
    pinnedJDK
    clang
    cmake
    pcre2
    ninja
    unzip
    curl
    wayland
    eza
    fd
    gtk3
    libpulseaudio
    pulseaudioFull
    fontconfig
    mesa
    libxkbcommon
    pkgs.libx11
    libGL
    alsa-lib.dev
    wayland.dev
    zstd.dev
    lmdb.dev
    sqlite.dev
    util-linux.dev
    libsysprof-capture
    libayatana-appindicator
    libnotify.dev
    libselinux.dev
  ];

  env = {
    JAVA_HOME = "${pinnedJDK}";
    ANDROID_HOME = "${androidSdk}/share/android-sdk";
    RUST_BACKTRACE = 1;
    ANDROID_NDK_PATH = ndkRoot;
    NIX_NIX_DEV_SHELL = "true";
    NIX_ANDROID_NDK_ROOT = ndkRoot;
    NIX_CFLAGS = "-I${sysrootPath}/usr/include";
    NIX_CXXFLAGS = "-I${sysrootPath}/usr/include/c++/v1";
    NIX_BINDGEN_EXTRA_CLANG_ARGS = "--sysroot=${sysrootPath}";
    NIX_RUSTFLAGS = "-Clink-arg=--sysroot=${sysrootPath}";
    NIX_CMAKE_TOOLCHAIN_FILE = cmakeToolchainFile;
    NIX_TOOLCHAIN_BIN_PATH = toolchainBinPath;
    NIX_ANDROID_SDK = androidSdk;
    NIX_PINNED_JDK = pinnedJDK;
    NIX_GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${aapt2Path}";
  };

  shellHook = ''
    alias ls=eza
    alias find=fd
    flutter config --jdk-dir "${pinnedJDK}"
    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath (with pkgs; [ wayland fontconfig libxkbcommon libx11 libGL ])}:$LD_LIBRARY_PATH
    # Keep user-installed cargo binaries (rinf, protoc-gen-prost, ...) reachable,
    # but AFTER the Nix toolchain so they can never shadow it.
    export PATH="$PATH:$HOME/.cargo/bin:$HOME/.pub-cache/bin"

    # mkShell accumulates -isystem flags from every (transitive) input with
    # heavy duplication; past a certain size gcc fails to spawn cc1/collect2
    # with "posix_spawn: Argument list too long". Dedupe the flag lists
    # pairwise (flags come as "-isystem <path>" pairs, order preserved).
    dedupe_flag_pairs() {
      local var="$1"
      local val="''${!var}"
      [ -n "$val" ] || return 0
      local new
      new=$(printf '%s\n' $val | paste -d' ' - - | awk '!seen[$0]++' | paste -sd' ')
      export "$var=$new"
    }
    dedupe_flag_pairs NIX_CFLAGS_COMPILE
    dedupe_flag_pairs NIX_CFLAGS_COMPILE_FOR_TARGET
    dedupe_flag_pairs NIX_LDFLAGS
    dedupe_flag_pairs NIX_LDFLAGS_FOR_TARGET

    if [ -e "$HOME/.cargo/bin/rustup" ]; then
      echo "WARNING: $HOME/.cargo/bin/rustup exists. cargokit checks that path BEFORE PATH,"
      echo "so it would bypass the Nix toolchain. Remove it to keep builds reproducible."
    fi

    setup_android_env() {
      echo "Setting up environment for Android cross-compilation..."
      export _NATIVE_PATH=$PATH
      export ANDROID_NDK_ROOT="${ndkRoot}"
      export ANDROID_NDK_PATH="${ndkRoot}"
      export CMAKE_TOOLCHAIN_FILE="${cmakeToolchainFile}"
      export CFLAGS="-I${sysrootPath}/usr/include"
      export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=${sysrootPath}"
      export RUSTFLAGS="-Clink-arg=--sysroot=${sysrootPath}"
      export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${aapt2Path}"
      export PKG_CONFIG_PATH="${toolchainBinPath}"
      export PKG_CONFIG_SYSROOT_DIR="${sysrootPath}"
      export _JAVA_OPTIONS="-Dorg.gradle.projectcachedir=$(mktemp -d)"

      # Point to the correct subdirectories within the fetched archive
      # This now perfectly mirrors the logic from your build.sh script.
      export ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR="${prebuiltOpenSSL}/armeabi-v7a"
      export AARCH64_LINUX_ANDROID_OPENSSL_DIR="${prebuiltOpenSSL}/arm64-v8a"
      export I686_LINUX_ANDROID_OPENSSL_DIR="${prebuiltOpenSSL}/x86"
      export X86_64_LINUX_ANDROID_OPENSSL_DIR="${prebuiltOpenSSL}/x86_64"

      export PATH="${toolchainBinPath}:${androidSdk}/share/android-sdk/platform-tools:${androidSdk}/share/android-sdk/tools:${androidSdk}/share/android-sdk/tools/bin:$PATH"
      echo "Android environment is ready."
    }

    teardown_android_env() {
      echo "Restoring native build environment..."
      if [ -n "$_NATIVE_PATH" ]; then export PATH=$_NATIVE_PATH && unset _NATIVE_PATH; fi
      unset ANDROID_NDK_ROOT ANDROID_NDK_PATH CMAKE_TOOLCHAIN_FILE CFLAGS BINDGEN_EXTRA_CLANG_ARGS RUSTFLAGS GRADLE_OPTS PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR
      unset ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR AARCH64_LINUX_ANDROID_OPENSSL_DIR I686_LINUX_ANDROID_OPENSSL_DIR X86_64_LINUX_ANDROID_OPENSSL_DIR
      echo "Native environment restored."
    }
    export -f setup_android_env teardown_android_env

    echo "--------------------------------------------------------"
    echo "Nix shell is ready for native (x86) development."
    echo "To build for Android, run: setup_android_env"
    echo "To return to native mode, run: teardown_android_env"
    echo "--------------------------------------------------------"
  '';
}
