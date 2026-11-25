#!/bin/sh
set -eu

DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export PATH="$DIR:$PATH"
export LD_LIBRARY_PATH="$DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
if [ -d "$DIR/share" ]; then
  export XDG_DATA_DIRS="$DIR/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
fi

chmod +x "$DIR/rune" 2>/dev/null || true

exec "$DIR/rune" "$@"
