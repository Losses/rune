#!/bin/sh

DIR=$(pwd)
export PATH="$DIR/xdg-user-dir:$PATH"

chmod +x "$DIR/rune"

exec "$DIR/rune" "$@"
