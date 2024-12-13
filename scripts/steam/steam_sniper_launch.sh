#!/bin/sh

DIR=$(pwd)
export PATH="$DIR:$PATH"
export LD_LIBRARY_PATH="$DIR/lib:$LD_LIBRARY_PATH"

chmod +x "$DIR/rune"

exec "$DIR/rune" "$@"