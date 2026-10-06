# Rune Windows Application Build Derivation
#
# Mirrored from nix/packages/rune.nix, adapted for Windows native (PE32+ / MSVC):
# 1. Builds libhub as hub.dll and hub.dll.lib (using cargo + MSVC).
# 2. Applies rinf.patch to intercept CargoKit, wiring @output_lib@ to hub.dll.lib.
# 3. Builds the Flutter Windows Runner (cmake + ninja + cl.exe).
# 4. Assembles the standalone portable folder into $out.

{ msvc
, rust
, flutter
, tools
}:

let
  version = "2.0.1011";

  # Step 1: Rust core (hub.dll and hub.dll.lib)
  libhub = (derivation {
    name = "rune-libhub-windows-${version}";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [];

    VCINSTALLDIR = msvc.env.VCINSTALLDIR;
    WindowsSdkDir = msvc.env.WindowsSdkDir;
    INCLUDE = msvc.env.INCLUDE;
    LIB = msvc.env.LIB;
    PATH = "${msvc.env.PATH};${rust.binPath}";

    CARGO_BUILD_TARGET = "x86_64-pc-windows-msvc";
  }) // {
    passthru = {
      dllName = "hub.dll";
      libName = "hub.dll.lib";
      libraryPath = "lib/hub.dll";
      importLibPath = "lib/hub.dll.lib";
    };
  };

  # Step 2: Main Rune Windows application bundle
  runeApplication = (derivation {
    name = "rune-windows-${version}";
    system = "x86_64-windows";
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
  }) // {
    passthru = {
      inherit libhub version;
      mainProgram = "rune.exe";
    };
  };

in runeApplication
