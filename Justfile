lint:
  cargo fmt -- --check
  cargo clippy -- -D warnings
  flutter analyze .
  dart analyze .

macos-ci-all: macos-ci-clean macos-ci-install
  ./scripts/macos_2_build.sh
  ./scripts/macos_3_prepare_before_sign.sh
  ./scripts/macos_4_replace_dylib.sh
  ./scripts/macos_5_codesign_and_notarize.sh
  ./scripts/macos_6_create_dmg.sh

macos-ci-all-appstore: macos-ci-clean macos-ci-install
  ./scripts/macos_2_appstore_build.sh
  ./scripts/macos_3_prepare_before_sign.sh
  ./scripts/macos_4_replace_dylib.sh
  ./scripts/macos_5_codesign_and_submit_to_appstore.sh

macos-ci-clean:
  ./scripts/macos_7_clean.sh

macos-ci-install:
  ./scripts/macos_1_ci_install.sh

macos-install:
  ./scripts/macos_1_install.sh

macos-run:
  ./scripts/macos_2_run.sh

macos-run-all: macos-install macos-run

macos-build:
  ./scripts/macos_2_build.sh

macos-build-all: macos-install macos-build
