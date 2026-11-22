#!/bin/sh
# Packages the app and both native watchers into an IPK in build/.
# Run scripts/build-watcher.sh first.
set -eu
cd "$(dirname "$0")/.."

for ARCH in armv7 aarch64; do
	[ -f "build/bin/arc-restore-watcher-$ARCH" ] ||
		{ echo "Missing build/bin/arc-restore-watcher-$ARCH; run scripts/build-watcher.sh" >&2; exit 1; }
done
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT INT TERM
cp -R app/. "$STAGE/"
mkdir -p "$STAGE/bin"
cp build/bin/arc-restore-watcher-armv7 build/bin/arc-restore-watcher-aarch64 "$STAGE/bin/"
chmod 755 "$STAGE/bin/"* "$STAGE/scripts/"*.sh
npx --yes --package @webos-tools/cli@3.2.6 ares-package --no-minify -o build "$STAGE"
ls -l build/*.ipk
