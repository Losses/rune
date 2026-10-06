# Essential Windows build tools (CMake, Ninja)
# Sourced directly from official release distributions matching Scoop Main bucket.

{ fetchurl ? (import <nix/fetchurl.nix>)
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  # Kitware CMake official binary (Scoop Main: bucket/cmake.json)
  cmakeInfo = if arch == "arm64" then {
    url = "https://github.com/Kitware/CMake/releases/download/v4.4.4/cmake-4.4.4-windows-arm64.zip";
    sha256 = "ed673bbc4eb7c1e59b0407fd52f1680fc0ae3229a46b9b7b3ea9133764f07f67";
  } else {
    url = "https://github.com/Kitware/CMake/releases/download/v4.4.4/cmake-4.4.4-windows-x86_64.zip";
    sha256 = "bace36e94b31c68ab6fa295f26dfa11219e0701cf7c94b0284a7d1cb13dac536";
  };

  # Ninja official release binary (Scoop Main: bucket/ninja.json)
  ninjaInfo = if arch == "arm64" then {
    url = "https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-winarm64.zip";
    sha256 = "e52f0bdef9dfb1003229dbd6508a508c4073fd017247002adc66e5e806cb0391";
  } else {
    url = "https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-win.zip";
    sha256 = "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65";
  };

  cmakeZip = fetchurl cmakeInfo;
  ninjaZip = fetchurl ninjaInfo;

  cmakePkg = derivation {
    name = "cmake-${arch}-4.4.4";
    system = system;
    builder = "cmd.exe";
    args = [ "/c" "mkdir %out% && tar.exe -xf %src% --strip-components=1 -C %out%" ];
    src = cmakeZip;
    PATH = "C:\\Windows\\System32";
  };

  ninjaPkg = derivation {
    name = "ninja-${arch}-1.13.2";
    system = system;
    builder = "cmd.exe";
    args = [ "/c" "mkdir %out% && tar.exe -xf %src% -C %out%" ];
    src = ninjaZip;
    PATH = "C:\\Windows\\System32";
  };

in rec {
  cmake = cmakePkg;
  ninja = ninjaPkg;

  binPaths = [
    "${cmake}/bin"
    "${ninja}"
  ];

  setupHookPwsh = ''
    $env:PATH = "${builtins.concatStringsSep ";" binPaths};" + $env:PATH
  '';
}
