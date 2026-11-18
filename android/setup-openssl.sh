# Download OpenSSL prebuilt library
OPENSSL_VERSION="3.4.0"
GITHUB_REPO="217heidai/openssl_for_android"
BASE_URL="https://github.com/$GITHUB_REPO/releases/download/$OPENSSL_VERSION"
ARCHITECTURES=("arm64-v8a" "armeabi-v7a" "x86" "x86_64")

BASE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )/openssl"

has_openssl() {
    local dir=$1
    [ -d "$dir" ] && [ -n "$(find "$dir" -mindepth 1 -maxdepth 1 -not -name ".*" -print -quit)" ]
}

download_and_extract() (
    local arch=$1
    local target_dir="$BASE_DIR/$arch"
    local filename="OpenSSL_${OPENSSL_VERSION}_${arch}.tar.gz"
    local temp_dir
    temp_dir="$(mktemp -d "$BASE_DIR/.openssl-$arch.XXXXXX")" || return 1
    trap 'rm -rf "$temp_dir"' EXIT

    echo "Downloading $BASE_URL/$filename..."
    if ! curl -fL "$BASE_URL/$filename" -o "$temp_dir/$filename"; then
        echo "Error: Failed to download OpenSSL for $arch" >&2
        return 1
    fi

    if ! tar -xzf "$temp_dir/$filename" -C "$temp_dir"; then
        echo "Error: Failed to extract OpenSSL for $arch" >&2
        return 1
    fi

    # Install the extracted directory in one move so a failed attempt leaves no partial cache.
    local extracted_dir="$temp_dir/openssl_${OPENSSL_VERSION}_${arch}"
    if [ -d "$target_dir" ] && ! rmdir "$target_dir"; then
        echo "Error: Could not replace the OpenSSL directory for $arch" >&2
        return 1
    fi
    if ! has_openssl "$extracted_dir" || ! mv "$extracted_dir" "$target_dir"; then
        echo "Error: Failed to install OpenSSL for $arch" >&2
        return 1
    fi
    echo "Successfully processed $arch"
)

download_openssl() {
    mkdir -p "$BASE_DIR" || return 1

    local arch
    for arch in "${ARCHITECTURES[@]}"; do
        if has_openssl "$BASE_DIR/$arch"; then
            continue
        fi
        if ! command -v curl &> /dev/null; then
            echo "Error: curl is required but not installed. Please install curl first." >&2
            return 1
        fi
        download_and_extract "$arch" || return 1
    done

    echo "OpenSSL libraries are installed in $BASE_DIR"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    download_openssl
fi
