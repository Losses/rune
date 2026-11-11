#!/usr/bin/env bash
# Decodes a base64 signing secret into a file and checks it before it is used,
# so a bad secret fails with its name instead of a bare base64 or import error.
#
# Usage: ci_decode_secret.sh SECRET_NAME OUTPUT_PATH [PASSWORD_NAME]
#   SECRET_NAME    environment variable holding the base64 text
#   OUTPUT_PATH    file to write; its extension says what is checked:
#                    .p12                                 certificate with its private key, not expired
#                    .provisionprofile, .mobileprovision  provisioning profile, not expired
#                    .p8                                  App Store Connect API key
#   PASSWORD_NAME  environment variable holding the .p12 password
#
# Secret values are read from the environment by name and never printed.

set -euo pipefail

name="$1"
out="$2"
password_name="${3:-}"

fail() {
  echo "::error title=$name::$1"
  exit 1
}

value="${!name:-}"
[[ -n "$value" ]] || fail "$name is empty or not set."
printf '%s' "$value" | base64 --decode > "$out" 2>/dev/null || fail "$name is not valid base64."
[[ -s "$out" ]] || fail "$name decodes to nothing."

# OpenSSL 3 opens a .p12 with older ciphers only with -legacy; LibreSSL has no
# -legacy and opens both.
pkcs12() {
  openssl pkcs12 -in "$out" -passin "env:$password_name" "$@" 2>/dev/null ||
    openssl pkcs12 -legacy -in "$out" -passin "env:$password_name" "$@" 2>/dev/null
}

case "$out" in
  *.p12)
    [[ -n "$password_name" ]] || fail "No password variable was given for $name."
    cert="$(pkcs12 -nokeys -clcerts)" ||
      fail "$name cannot be opened with $password_name: the password is wrong or the file is not a .p12."
    # grep -c reads to the end: grep -q would close the pipe early under pipefail.
    keys="$(pkcs12 -nocerts -nodes | grep -c 'PRIVATE KEY' || true)"
    [[ "$keys" -gt 0 ]] || fail "$name has no private key. Export the certificate together with its key."
    end="$(openssl x509 -noout -enddate <<<"$cert" | cut -d= -f2)"
    openssl x509 -noout -checkend 0 <<<"$cert" > /dev/null ||
      fail "$name holds a certificate that expired on $end."
    echo "$name: certificate valid until $end"
    ;;
  *.provisionprofile | *.mobileprovision)
    plist="$(security cms -D -i "$out" 2>/dev/null)" || fail "$name is not a provisioning profile."
    profile="$(plutil -extract Name raw -o - - <<<"$plist")" || fail "$name is a profile without a name."
    expiry="$(plutil -extract ExpirationDate raw -o - - <<<"$plist")" ||
      fail "$name is a profile without an expiration date."
    [[ "$expiry" > "$(date -u +%Y-%m-%dT%H:%M:%SZ)" ]] ||
      fail "$name holds the profile \"$profile\", which expired on $expiry."
    echo "$name: profile \"$profile\" valid until $expiry"
    ;;
  *.p8)
    grep -q 'BEGIN PRIVATE KEY' "$out" || fail "$name is not an App Store Connect API key (.p8)."
    echo "$name: API key decoded"
    ;;
esac
