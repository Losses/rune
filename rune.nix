{
  lib,
  stdenv,
  flutter,
  rustPlatform,
  alsa-lib,
  fetchFromGitHub,
  replaceVars,
  pkg-config,
  openssl,
  dbus,
  libnotify,
  libayatana-appindicator,
  targetFlutterPlatform ? "linux",
}:

let
  version = "2.0.1008";

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

  pubspecLock = lib.importJSON ./pubspec.lock.json;

  gitHashes = {
    macos_secure_bookmarks = "sha256-qC3Ytxkg5bGh6rns0Z/hG3uLYf0Qyw6y6Hq+da1Je0I=";
    fluent_ui = "sha256-r40uN7mwr6MCNg41AKdk2Z9Zd4pUCxV0xwLXKgNauqo=";
    scrollable_positioned_list = "sha256-tkRlxnmyG5r5yZwEvj9KDfEf4OuSM0HEwPOYm7T7LnQ=";
    system_tray = "sha256-1XMVu1uHy4ZgPKDqfQ7VTDVJvDxky5+/BbocGz8rKYs=";
  };

  src = flutter.buildFlutterApplication {
    inherit version pubspecLock gitHashes;
    pname = "source";

    src = fetchFromGitHub {
      owner = "Losses";
      repo = "rune";
      tag = "v${version}";
      hash = "sha256-vYbi6vguKPI7UoCIKjlGHTQi+OoVjPbIOLNyoW2DOv0=";
    };

    nativeBuildInputs = [
      rustPlatform.cargoSetupHook
      pkg-config
    ];

    buildPhase = ''
      runHook preBuild

      export CARGO_HOME=$(mktemp -d)
      cargo install rinf_cli
      export PATH="$CARGO_HOME/bin:$PATH"
      rinf gen

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      cp -r . $out
      mkdir $debug

      runHook postInstall
    '';

    meta = metaCommon;
  };

  libhub = rustPlatform.buildRustPackage {
    inherit version src;
    pname = "libhub";

    useFetchCargoVendor = true;

    cargoHash = "sha256-+7zUUUpXYKmeCVA+XZLMR1Z41ZIIfHvPTelCCY/UOfI=";

    nativeBuildInputs = [
      pkg-config
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
    pubspecLock
    gitHashes
    ;
  pname = "rune-${targetFlutterPlatform}";

  nativeBuildInputs = [
    rustPlatform.cargoSetupHook
    pkg-config
  ];

  buildInputs = [
    libnotify
    libayatana-appindicator
  ];

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
