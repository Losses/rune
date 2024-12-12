#!/bin/sh

DIR=$(pwd)
export PATH="$DIR:$PATH"

chmod +x "$DIR/rune"

exec "$DIR/rune --pro" "$@"
