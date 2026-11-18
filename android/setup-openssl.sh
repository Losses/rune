# Download OpenSSL prebuilt library
OPENSSL_VERSION="3.4.0"
GITHUB_REPO="217heidai/openssl_for_android"
BASE_URL="https://github.com/$GITHUB_REPO/releases/download/$OPENSSL_VERSION"
ARCHITECTURES=("arm64-v8a" "armeabi-v7a" "x86" "x86_64")

BASE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )/openssl"

has_openssl() {
    local dir=$1
    [ -s "$dir/lib/libssl.a" ] &&
        [ -s "$dir/lib/libcrypto.a" ] &&
        [ -s "$dir/include/openssl/ssl.h" ]
}

download_and_extract() (
    local arch=$1
    local target_dir="$BASE_DIR/$arch"
    local filename="OpenSSL_${OPENSSL_VERSION}_${arch}.tar.gz"
    local temp_dir
    temp_dir="$(mktemp -d "$BASE_DIR/.openssl-$arch.XXXXXX")" || return 1
    trap 'rm -rf "$temp_dir"' EXIT

    echo "Downloading $BASE_URL/$filename..."
    if ! curl -fL --connect-timeout 15 --max-time 300 "$BASE_URL/$filename" -o "$temp_dir/$filename"; then
        echo "Error: Failed to download OpenSSL for $arch" >&2
        return 1
    fi

    if ! tar -xzf "$temp_dir/$filename" -C "$temp_dir"; then
        echo "Error: Failed to extract OpenSSL for $arch" >&2
        return 1
    fi

    local extracted_dir="$temp_dir/openssl_${OPENSSL_VERSION}_${arch}"
    if ! has_openssl "$extracted_dir"; then
        echo "Error: Incomplete OpenSSL archive for $arch" >&2
        return 1
    fi

    if has_openssl "$target_dir"; then
        return 0
    fi
    if ! rm -rf "$target_dir" || ! mv "$extracted_dir" "$target_dir"; then
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
