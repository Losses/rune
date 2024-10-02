{ 
  lib
, jq
, stdenv
, fetchzip
, flutter324
, protobuf_26
, protoc-gen-prost
, makeDesktopItem
, moreutils
, cargo
, rustPlatform
, rustc
, alsa-lib
, lmdb
, targetFlutterPlatform ? "linux"
, buildDartApplication
, dart
}:

let
  mainPubspecLock = lib.importJSON ./pubspec.lock.json;
  cargokitPubspecLock = lib.importJSON ./pubspec.cargokit.lock.json;

  protoc-gen-dart = buildDartApplication rec {
    pname = "protoc-gen-dart";
    version = "21.1.2";

    src = fetchzip {
      url = "https://github.com/google/protobuf.dart/archive/refs/tags/protoc_plugin-v21.1.2.tar.gz";
      sha256 = "sha256-luptbRgOtOBapWmyIJ35GqOClpcmDuKSPu3QoDfp2FU=";
    };
    sourceRoot = "${src.name}/protoc_plugin";

    pubspecLock = lib.importJSON ./pubspec.protoc.lock.json;

    meta = with lib; {
      description = "Protobuf plugin for generating Dart code";
      mainProgram = "protoc-gen-dart";
      homepage = "https://pub.dev/packages/protoc_plugin";
      license = licenses.bsd3;
    };
  };
in
flutter324.buildFlutterApplication (rec {
  pname = "rune-${targetFlutterPlatform}";
  version = "1.20.0";

  src = fetchzip {
    url = "https://github.com/Losses/rune/archive/94b12535d185523a18d019c3ac3277ddd14998b5.tar.gz";
    sha256 = "sha256-i98DAH6SYI3ICzbItZDOgFWKzDXhrVxN+DBVn419fxc=";
  };

  gitHashes = {
    fluent_ui = "sha256-87wJgWP4DGsVOxc4PhCMDg+ro9faHKZXy2LQtFqbmso=";
  };

  customSourceBuilders = {
    rinf =
      { version, src, ... }:
      stdenv.mkDerivation {
        pname = "rinf";
        inherit version src;
        inherit (src) passthru;

        patches = [ ./rinf.patch ];

        installPhase = ''
          runHook preInstall
          mkdir -p "$out"
          cp -r * "$out"
          runHook postInstall
        '';
      };
  };

  # build_tool hack part 1: join dependencies with the main package
  pubspecLock = lib.recursiveUpdate cargokitPubspecLock mainPubspecLock;

  inherit targetFlutterPlatform;

  nativeBuildInputs = [
    jq
    moreutils # sponge
    protobuf_26
    protoc-gen-prost
    protoc-gen-dart
    cargo
    rustc
    rustPlatform.cargoSetupHook
    alsa-lib
    lmdb
  ]; 

  cargoDeps = rustPlatform.fetchCargoTarball {
    inherit src;
    name = "${pname}-${version}";
    hash = "sha256-GJiDGg26vQlWsG/X/S45CsqrBOYp2iHjVi/k693SpGE=";
  };

  desktopItem = makeDesktopItem {
    name = "Rune";
    exec = "player";
    icon = "rune";
    desktopName = "Rune";
    genericName = "Player your favorite music";
    categories = ["Audio"];
  };

  preBuild = ''
    echo =================================
    echo PATCHING CARGOKIT
    echo =================================
    # build_tool hack part 2: add build_tool as an actually resolvable package (the location is relative to the rinf package directory)
    jq '.packages += [.packages.[] | select(.name == "rinf") | .rootUri += "/cargokit/build_tool" | .name = "build_tool"]' .dart_tool/package_config.json | sponge .dart_tool/package_config.json
    echo =================================
    echo GENERATING PROTOBUF CODE
    echo =================================
    packageRun rinf message

  '';

  meta = with lib; {
    description = "Experience timeless melodies with a music player that blends classic design with modern technology.";
    homepage = "https://github.com/losses/rune";
    license = licenses.mpl20;
    mainProgram = "player";
    maintainers = with maintainers; [ losses ];
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ sourceTypes.fromSource ];
  };
})
