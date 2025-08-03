#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

install_if_missing() {
    if command -v "$1" >/dev/null 2>&1; then
        echo "$2 is already installed"
    else
        echo "$2 is not installed. Installing with Homebrew..."
        brew install $3
    fi
}

install_if_missing flutter "Flutter" "--cask flutter"
install_if_missing rustup "rustup" "rustup"

brew install lmdb create-dmg protobuf CocoaPods
brew link cocoapods

if [ -f "$HOME/.cargo/env" ] && command -v rustc >/dev/null 2>&1; then
    echo "Skip rustup-init due to already installed"
else
    rustup-init -y
fi

. "$HOME/.cargo/env"
rustup default stable
rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin

flutter pub global activate protoc_plugin
export PATH="$PATH":"$HOME/.pub-cache/bin"

cargo install rinf_cli

echo "DEBUG: ----------------------------"

which cargo
which rustc
which rustup
rustup target list --installed
