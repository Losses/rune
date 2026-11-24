#!/bin/bash
# Run isolated validation commands only after storage preflight succeeds.
set -euo pipefail
if [[ $# -eq 0 ]]; then
  echo 'Usage: macos_scale_run.sh COMMAND [ARG ...]' >&2
  exit 2
fi
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$script_directory/macos_scale_environment.sh" || exit 1
exec "$@"
