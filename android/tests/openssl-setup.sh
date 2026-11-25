#!/usr/bin/env bash
set -euo pipefail

android_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/build"
cp "$android_dir"/{build,setup-openssl}.sh "$scratch/build"

export ANDROID_NDK_PATH="$scratch/ndk"
unset NIX_NIX_DEV_SHELL
export REAL_MV
REAL_MV="$(command -v mv)"
for tool in bash dirname find mkdir mktemp rm uname; do
    ln -s "$(command -v "$tool")" "$scratch/bin/$tool"
done

cat > "$scratch/bin/download" <<'STUB'
#!/usr/bin/env bash
archive="${@: -1}"
echo "download ${archive##*/}" >> "$BUILD_TRACE"
[[ "$FAIL_STEP" != download || "$archive" != *armeabi-v7a* ]] || exit 1
: > "$archive"
STUB

cat > "$scratch/bin/tar" <<'STUB'
#!/usr/bin/env bash
archive="${2##*/}"
echo "extract $archive" >> "$BUILD_TRACE"
[[ "$FAIL_STEP" != extract || "$archive" != *armeabi-v7a* ]] || exit 1
archive_dir="$4/openssl_${archive#OpenSSL_}"
archive_dir="${archive_dir%.tar.gz}"
mkdir -p "$archive_dir/lib" "$archive_dir/include/openssl"
for file in lib/libssl.a lib/libcrypto.a include/openssl/ssl.h; do
    echo fixture > "$archive_dir/$file"
done
STUB

cat > "$scratch/bin/mv" <<'STUB'
#!/usr/bin/env bash
target="${@: -1}"
echo "install $target" >> "$BUILD_TRACE"
[[ "$FAIL_STEP" != install || "$target" != */openssl/armeabi-v7a* ]] || exit 1
exec "$REAL_MV" "$@"
STUB

cat > "$scratch/bin/flutter" <<'STUB'
#!/usr/bin/env bash
echo flutter >> "$BUILD_TRACE"
STUB
chmod +x "$scratch/bin/"{download,tar,mv,flutter}
ln -s "$scratch/bin/download" "$scratch/bin/curl"

export BUILD_TRACE="$scratch/trace"
run_build() {
    : > "$BUILD_TRACE"
    PATH="$scratch/bin" FAIL_STEP="$1" "$BASH" "$scratch/build/build.sh" > "$scratch/output" 2>&1
}

failures=0
for step in download extract install; do
    rm -rf "$scratch/build/openssl"
    if run_build "$step"; then
        echo "Expected the Android build to fail after an OpenSSL $step failure" >&2
        failures=1
    fi
    if ! grep -q "install .*/openssl/arm64-v8a" "$BUILD_TRACE" ||
       ! grep -q "^$step .*armeabi-v7a" "$BUILD_TRACE" ||
       grep -q '^flutter$' "$BUILD_TRACE"; then
        echo "Expected arm64 installation and armeabi-v7a $step failure before Flutter" >&2
        cat "$BUILD_TRACE" >&2
        failures=1
    fi
done
(( failures == 0 )) || exit 1

mkdir -p "$scratch/build/openssl/armeabi-v7a"
run_build none
if grep -q '^download .*arm64-v8a' "$BUILD_TRACE"; then
    echo "Retry downloaded an already installed architecture" >&2
    exit 1
fi
for arch in arm64-v8a armeabi-v7a x86 x86_64; do
    test -s "$scratch/build/openssl/$arch/lib/libssl.a"
done
grep -q '^flutter$' "$BUILD_TRACE"

rm "$scratch/bin/curl"
run_build none
test "$(cat "$BUILD_TRACE")" = flutter

rm -rf "$scratch/build/openssl/x86_64"
if run_build none; then
    echo "Expected setup to fail when an architecture is missing and curl is unavailable" >&2
    exit 1
fi
grep -q 'curl is required' "$scratch/output"
test ! -s "$BUILD_TRACE"
ln -s "$scratch/bin/download" "$scratch/bin/curl"
mkdir -p "$scratch/build/openssl/x86_64"
: > "$scratch/build/openssl/x86_64/.metadata"
: > "$scratch/build/openssl/x86_64/incomplete"
run_build none
if ! test -s "$scratch/build/openssl/x86_64/lib/libssl.a"; then
    echo "Expected a partial OpenSSL cache to be repaired" >&2
    exit 1
fi

test -z "$(find "$scratch/build/openssl" -name 'openssl_*.tar.gz' -o -name 'openssl_*' -o -name '.openssl-*')"
echo "OpenSSL preparation and Android build status checks passed"
