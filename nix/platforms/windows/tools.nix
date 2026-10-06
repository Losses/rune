# Essential Windows build tools (CMake, Ninja, rinf_cli)
#
# Provides standalone portable tools unpacked into store paths.

{ fetchurl ? (import <nix/fetchurl.nix>)
}:

let
  cmakePkg = derivation {
    name = "cmake-windows";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [
      (fetchurl {
        url = "https://github.com/Kitware/CMake/releases/download/v3.31.5/cmake-3.31.5-windows-x86_64.tar.zst";
        sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
      })
    ];
  };

  ninjaPkg = derivation {
    name = "ninja-windows";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [
      (fetchurl {
        url = "https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-win.tar.zst";
        sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
      })
    ];
  };

  rinfCliPkg = derivation {
    name = "rinf-cli-windows";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [
      (fetchurl {
        url = "https://github.com/cunarist/rinf/releases/download/v8.7.1/rinf_cli-x86_64-pc-windows-msvc.tar.zst";
        sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
      })
    ];
  };

in rec {
  cmake = cmakePkg;
  ninja = ninjaPkg;
  rinfCli = rinfCliPkg;

  binPaths = [
    "${cmake}/bin"
    "${ninja}"
    "${rinfCli}"
  ];

  setupHookPwsh = ''
    $env:PATH = "${builtins.concatStringsSep ";" binPaths};" + $env:PATH
  '';
}
