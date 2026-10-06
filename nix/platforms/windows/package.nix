# Rune Windows Application Build Derivation
# Supports both x86_64 (x64) and Windows on ARM (arm64).
#
# Mirrored from nix/packages/rune.nix, adapted for Windows native (PE32+ / MSVC):
# 1. Builds libhub as hub.dll and hub.dll.lib (using cargo + MSVC for target arch).
# 2. Applies rinf.patch to intercept CargoKit, wiring @output_lib@ to hub.dll.lib.
# 3. Builds the Flutter Windows Runner (cmake + ninja + cl.exe).
# 4. Assembles the standalone portable folder into $out.

{ msvc
, rust
, flutter
, tools
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  version = "2.0.1011";

  # Step 1: Rust core (hub.dll and hub.dll.lib)
  libhub = (derivation {
    name = "rune-libhub-windows-${arch}-${version}";
    system = system;
    builder = "builtin:unpack";
    srcs = [];

    VCINSTALLDIR = msvc.env.VCINSTALLDIR;
    WindowsSdkDir = msvc.env.WindowsSdkDir;
    INCLUDE = msvc.env.INCLUDE;
    LIB = msvc.env.LIB;
    PATH = "${msvc.env.PATH};${rust.binPath}";

    CARGO_BUILD_TARGET = rust.target;
  }) // {
    passthru = {
      dllName = "hub.dll";
      libName = "hub.dll.lib";
      libraryPath = "lib/hub.dll";
      importLibPath = "lib/hub.dll.lib";
      inherit arch;
    };
  };

  # Step 2: Main Rune Windows application bundle
  runeApplication = (derivation {
    name = "rune-windows-${arch}-${version}";
    system = system;
    builder = "builtin:unpack";
    srcs = [];

    VCINSTALLDIR = msvc.env.VCINSTALLDIR;
    WindowsSdkDir = msvc.env.WindowsSdkDir;
    INCLUDE = msvc.env.INCLUDE;
    LIB = msvc.env.LIB;
    PATH = builtins.concatStringsSep ";" [
      "${./shims}"
      msvc.env.PATH
      rust.binPath
      flutter.binPath
      (builtins.concatStringsSep ";" tools.binPaths)
    ];

    # Path to prebuilt hub.dll.lib for rinf.patch
    outputLib = "${libhub}/${libhub.passthru.importLibPath}";
    flutterBuildFlag = flutter.buildFlag;
  }) // {
    passthru = {
      inherit libhub version arch;
      mainProgram = "rune.exe";
    };
  };

in runeApplication
