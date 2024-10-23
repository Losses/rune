# Download OpenSSL prebuilt library
OPENSSL_VERSION="3.4.0"
GITHUB_REPO="217heidai/openssl_for_android"
BASE_URL="https://github.com/$GITHUB_REPO/releases/download/$OPENSSL_VERSION"
ARCHITECTURES=("arm64-v8a" "armeabi-v7a" "x86" "x86_64")

# Create base directory if not exist
BASE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )/openssl"
mkdir -p "$BASE_DIR"

check_directory() {
    local dir=$1
    
    # If OpenSSL directory does not exist, we'll need to download
    if [ ! -d "$dir" ]; then
        return 0
    fi
    
    # Check if OpenSSL directory is empty
    if [ -n "$(find "$dir" -mindepth 1 -not -name ".*" -print -quit)" ]; then
        return 1    # not empty
    fi
    
    return 0    # empty
}

download_and_extract() {
    local arch=$1
    local target_dir="$BASE_DIR/$arch"
    local filename="OpenSSL_${OPENSSL_VERSION}_${arch}.tar.gz"
    local download_url="$BASE_URL/$filename"
    local temp_dir="/tmp/openssl_temp_$$"
    
    echo "Processing $arch architecture..."
    
    # Create arch-specific directory
    mkdir -p "$temp_dir"
    mkdir -p "$target_dir"
    
    echo "Downloading $download_url..."
    if curl -L "$download_url" -o "/tmp/$filename"; then
        echo "Download successful for $arch"
        
        # Extract to target directory
        echo "Extracting to $target_dir..."
        tar -xzf "/tmp/$filename" -C "$temp_dir"
        mv "$temp_dir"/openssl_"${OPENSSL_VERSION}_${arch}"/* "$target_dir/"
        
        rm "/tmp/$filename"
        rm -rf "$temp_dir"
        
        echo "Successfully processed $arch"
    else
        echo "Error: Failed to download $arch"
        return 1
    fi
}

download_openssl() {
    mkdir -p "$BASE_DIR"
    
    
    # Check if all architectures are present and not empty
    for arch in "${ARCHITECTURES[@]}"; do
        if check_directory "$BASE_DIR/$arch"; then
            # If at least one directory is empty, we need to download
            echo "OpenSSL directory is empty or incomplete. Starting download..."
            break
        else
            # If last architecture is checked and all are present, we can skip download
            if [ "$arch" = "${ARCHITECTURES[-1]}" ]; then
                echo "OpenSSL already exists in $BASE_DIR"
                echo "If you want to re-download, please remove or empty the $BASE_DIR directory first."
                return 0
            fi
            continue
        fi
    done
    
    echo "Starting OpenSSL download and setup for version $OPENSSL_VERSION"
    
    # Check if curl is installed
    if ! command -v curl &> /dev/null; then
        echo "Error: curl is required but not installed. Please install curl first."
        return 1
    fi
    
    # Process each architecture
    for arch in "${ARCHITECTURES[@]}"; do
        if ! download_and_extract "$arch"; then
            echo "Warning: Failed to process $arch"
        fi
    done
    
    echo "Setup complete!"
    echo "OpenSSL libraries are installed in $BASE_DIR"
    return 0
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    download_openssl
fi
