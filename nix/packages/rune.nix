{
  lib,
  stdenv,
  flutter,
  rustPlatform,
  fetchFromGitHub,
  alsa-lib,
  replaceVars,
  pkg-config,
  cmake,
  openssl,
  dbus,
  gsettings-desktop-schemas,
  dconf,
  libnotify,
  libayatana-appindicator,
  vulkan-loader,
  rinf-cli,
  targetFlutterPlatform ? "linux",
}:

let
  version = "2.0.1016";

  metaCommon = {
    description = "Experience timeless melodies with a music player that blends classic design with modern technology";
    homepage = "https://github.com/Losses/rune";
    license = with lib.licenses; [
      mpl20
      mit
      asl20
    ];
    maintainers = with lib.maintainers; [ losses ];
  };

  # The lock files of the checkout, read as they are (`autoPubspecLock` turns
  # `pubspec.lock` into the JSON the Dart builder wants while evaluating).
  pubspecLockFile = ../../pubspec.lock;

  gitHashes = {
    bitsdojo_window_linux = "sha256-irPQE+PP2mfTNW5PMSVr6al2iE6kBu3ujCXiiQL71m0=";
    fluent_ui = "sha256-jSYl046/hLmrLLK+pIbRqAm/WJLpM999yJ5N0c/ucWs=";
    macos_secure_bookmarks = "sha256-f6TDL1JfWc8zGE8H3+YLNmRW3/KJSgjQiwhk44bMp5g=";
    scrollable_positioned_list = "sha256-tkRlxnmyG5r5yZwEvj9KDfEf4OuSM0HEwPOYm7T7LnQ=";
    system_tray = "sha256-yx4Tc+HDx9LdVWJZKniQr6H3BsifPlNwVGxHj3jpvlE=";
  };

  # Match the revisions declared in metadata/native/CMakeLists.txt.
  utfcppSource = fetchFromGitHub {
    owner = "Losses";
    repo = "utfcpp";
    rev = "161c216be03e671dbc0ee8cace084d3d82b18932";
    hash = "sha256-CeJ2+d7kgN+H1XKLsoqvwIgr1BYtMxKGAK61bb+eY7s=";
  };

  taglibSource = fetchFromGitHub {
    owner = "Losses";
    repo = "taglib";
    rev = "7333be146c0a412d345f7e73554e383ebb8d6fc3";
    hash = "sha256-TEawukG0PrcXjD6sDzTlFJ8rwPiS6sgQqZPrEwSzfzU=";
  };

  src = flutter.buildFlutterApplication {
    inherit version gitHashes;
    autoPubspecLock = pubspecLockFile;
    pname = "source";

    # The checkout itself: its own lock files are used above, and a released
    # tarball need not agree with them.
    src = ../..;

    nativeBuildInputs = [
      pkg-config
      rinf-cli
    ];

    # Only the sources are wanted here; the app is built below. `rinf gen`
    # writes the Dart and Rust bindings, which the repository does not carry.
    buildPhase = ''
      runHook preBuild

      rinf gen

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      # The pub phases point `pubspec.lock` into /build; the app build below
      # links its own.
      rm -f pubspec.lock
      cp -r . $out
      mkdir $debug

      runHook postInstall
    '';

    meta = metaCommon;
  };

  libhub = rustPlatform.buildRustPackage {
    inherit version src;
    pname = "libhub";

    cargoHash = "sha256-j3S1qXXl79vBDk3s8Iiq1hvaOxNtKDG5q0AGQbRFaAQ=";

    env = {
      FETCHCONTENT_SOURCE_DIR_UTFCPP = "${utfcppSource}";
      FETCHCONTENT_SOURCE_DIR_TAGLIB = "${taglibSource}";
    };

    nativeBuildInputs = [
      pkg-config
      # TagLib and the Opus bundled by `opusic-sys` are built with CMake.
      cmake
    ];

    buildInputs = [
      openssl
      alsa-lib
      dbus
    ];

    doCheck = false; # test failed

    passthru.libraryPath = "lib/libhub.so";

    meta = metaCommon // {
      mainProgram = "rune-cli";
    };
  };
in
flutter.buildFlutterApplication {
  inherit
    version
    src
    gitHashes
    ;
  autoPubspecLock = pubspecLockFile;
  pname = "rune-${targetFlutterPlatform}";

  # No Rust is built here: the `rinf` source builder links the prebuilt
  # `libhub` (see `customSourceBuilders`), so `cargoSetupHook` would only try
  # to unpack cargo dependencies that do not exist.
  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    libnotify
    libayatana-appindicator
    gsettings-desktop-schemas
    dconf
  ];

  # wgpu, which draws the visualizer, loads the Vulkan loader at run time, so
  # it is not an ELF dependency and no RUNPATH would find it.
  extraWrapProgramArgs = lib.optionalString stdenv.hostPlatform.isLinux ''
    --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ vulkan-loader ]}
  '';

  customSourceBuilders = {
    rinf =
      { version, src, ... }:
      stdenv.mkDerivation {
        pname = "rinf";
        inherit version src;
        inherit (src) passthru;

        patches = [
          (replaceVars ./rinf.patch {
            output_lib = "${libhub}/${libhub.passthru.libraryPath}";
          })
        ];

        installPhase = ''
          runHook preInstall

          cp -r . $out

          runHook postInstall
        '';
      };
  };

  postInstall = ''
    mkdir -p $out/share
    cp -r --no-preserve=mode $src/assets/icons $out/share/icons
    ln -s $out/share/icons/Papirus $out/share/icons/hicolor
    ln -s $out/share/icons/Papirus $out/share/icons/Papirus-Dark
    ln -s $out/share/icons/Papirus $out/share/icons/Papirus-Light
    ln -s $out/share/icons/breeze/apps/1024/rune.png $out/share/icons/rune.png
    install -Dm0644 assets/source/linux/rune.desktop $out/share/applications/rune.desktop
    install -Dm0644 assets/source/linux/rune-scene.xml $out/share/mime/packages/rune-scene.xml
  '';

  meta = metaCommon // {
    mainProgram = "rune";
  };
}
