#!/bin/sh
# Builds the Linux release archive from an existing release build:
#
#   cargo build --release
#   installer/linux/package.sh 0.3.0      → target/installer/Paneltarium-0.3.0-linux-x86_64.tar.gz
#
# The archive holds the three programs, the menu entries, the icon and
# install.sh (see there). Used by .github/workflows/release.yml.
set -eu

VERSION=${1:?usage: package.sh VERSION}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ARCH=$(uname -m)
NAME="Paneltarium-$VERSION-linux-$ARCH"
OUT="$ROOT/target/installer"
STAGE="$OUT/$NAME"
APP_ID=io.github.jordan_dev_stuff.Paneltarium

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/icons"
for p in paneltarium paneltarium-app paneltarium-manager; do
    install -m 755 "$ROOT/target/release/$p" "$STAGE/bin/$p"
done
install -m 644 "$ROOT/installer/linux/$APP_ID.desktop" "$ROOT/installer/linux/$APP_ID.Manager.desktop" \
    "$STAGE/share/applications/"
install -m 644 "$ROOT/assets/paneltarium-256.png" "$STAGE/share/icons/$APP_ID.png"
install -m 755 "$ROOT/installer/linux/install.sh" "$STAGE/install.sh"
install -m 644 "$ROOT/README.md" "$ROOT/LICENSE" "$STAGE/"

tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
echo "$OUT/$NAME.tar.gz"
