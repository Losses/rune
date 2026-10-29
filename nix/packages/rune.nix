{
  lib,
  stdenv,
  flutter,
  rustPlatform,
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
  version = "2.0.1015";

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
    macos_secure_bookmarks = "sha256-f6TDL1JfWc8zGE8H3+YLNmRW3/KJSgjQiwhk44bMp5g=";
    fluent_ui = "sha256-r40uN7mwr6MCNg41AKdk2Z9Zd4pUCxV0xwLXKgNauqo=";
    scrollable_positioned_list = "sha256-tkRlxnmyG5r5yZwEvj9KDfEf4OuSM0HEwPOYm7T7LnQ=";
    system_tray = "sha256-yx4Tc+HDx9LdVWJZKniQr6H3BsifPlNwVGxHj3jpvlE=";
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

    cargoHash = "sha256-UmaYXyfXMM/mU02qN96ulv22kR7KSfrnqsk4S8aJQZs=";

    nativeBuildInputs = [
      pkg-config
      # `opusic-sys` builds the Opus it vendors with CMake.
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
  '';

  meta = metaCommon // {
    mainProgram = "rune";
  };
}
