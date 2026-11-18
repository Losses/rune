#!/bin/bash
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
for tool in dirname find mkdir mktemp rm rmdir uname; do
    ln -s "$(command -v "$tool")" "$scratch/bin/$tool"
done

cat > "$scratch/bin/download" <<'STUB'
#!/bin/bash
archive="${@: -1}"
echo "download ${archive##*/}" >> "$BUILD_TRACE"
[[ "$FAIL_STEP" != download || "$archive" != *armeabi-v7a* ]] || exit 1
: > "$archive"
STUB

cat > "$scratch/bin/tar" <<'STUB'
#!/bin/bash
archive="${2##*/}"
echo "extract $archive" >> "$BUILD_TRACE"
[[ "$FAIL_STEP" != extract || "$archive" != *armeabi-v7a* ]] || exit 1
archive_dir="$4/openssl_${archive#OpenSSL_}"
archive_dir="${archive_dir%.tar.gz}"
mkdir -p "$archive_dir"
: > "$archive_dir/libssl.a"
STUB

cat > "$scratch/bin/mv" <<'STUB'
#!/bin/bash
target="${@: -1}"
echo "install $target" >> "$BUILD_TRACE"
[[ "$FAIL_STEP" != install || "$target" != */openssl/armeabi-v7a* ]] || exit 1
exec "$REAL_MV" "$@"
STUB

cat > "$scratch/bin/flutter" <<'STUB'
#!/bin/bash
echo flutter >> "$BUILD_TRACE"
STUB
chmod +x "$scratch/bin/"{download,tar,mv,flutter}
ln -s "$scratch/bin/download" "$scratch/bin/curl"

export BUILD_TRACE="$scratch/trace"
run_build() {
    : > "$BUILD_TRACE"
    PATH="$scratch/bin" FAIL_STEP="$1" /bin/bash "$scratch/build/build.sh" > "$scratch/output" 2>&1
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
    test -f "$scratch/build/openssl/$arch/libssl.a"
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
test -z "$(find "$scratch/build/openssl" -name '.openssl-*' -print -quit)"
echo "OpenSSL preparation and Android build status checks passed"
