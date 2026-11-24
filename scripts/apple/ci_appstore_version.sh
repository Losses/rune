#!/usr/bin/env bash
# Prints the App Store version (X.Y.Z) of this build. A tag build takes it from
# its tag (REF_NAME). A rehearsal from a branch takes the newest v* tag, which is
# the version the next tag build would most likely upload.

set -euo pipefail

if [[ "${REF_NAME:-}" =~ ^v([0-9]+\.[0-9]+\.[0-9]+) ]]; then
  echo "${BASH_REMATCH[1]}"
  exit 0
fi

version="$(git ls-remote --tags --refs origin 'refs/tags/v*' |
  sed -nE 's#.*refs/tags/v([0-9]+\.[0-9]+\.[0-9]+).*#\1#p' |
  sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)"
[[ -n "$version" ]] || { echo "No v* tag to take the App Store version from." >&2; exit 1; }
echo "$version"
