#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
OUTPUT_DIR="${1:-$ROOT_DIR/dist}"
DAEMON_PATH="${OTD_DAEMON_PATH:-}"
TABLETFLOW_BINARY_PATH="${TABLETFLOW_BINARY_PATH:-$ROOT_DIR/target/release/tabletflow}"
VERSION="${TABLETFLOW_VERSION:-0.1.0}"
ARCH="${TABLETFLOW_ARCH:-$(uname -m)}"
PACKAGE_DIR="$OUTPUT_DIR/TabletFlow-${VERSION}-linux-${ARCH}"
ARCHIVE_PATH="$OUTPUT_DIR/TabletFlow-${VERSION}-linux-${ARCH}.tar.gz"

if [[ ! -f "$TABLETFLOW_BINARY_PATH" ]]; then
    echo "TABLETFLOW_BINARY_PATH must point to the TabletFlow executable" >&2
    exit 1
fi
if [[ -z "$DAEMON_PATH" || ! -f "$DAEMON_PATH" ]]; then
    echo "OTD_DAEMON_PATH must point to OpenTabletDriver.Daemon" >&2
    exit 1
fi

rm -rf "$PACKAGE_DIR" "$ARCHIVE_PATH"
mkdir -p "$PACKAGE_DIR"
cp "$TABLETFLOW_BINARY_PATH" "$PACKAGE_DIR/TabletFlow"
cp "$DAEMON_PATH" "$PACKAGE_DIR/OpenTabletDriver.Daemon"
if [[ -n "${OTD_LICENSE_PATH:-}" && -f "$OTD_LICENSE_PATH" ]]; then
    cp "$OTD_LICENSE_PATH" "$PACKAGE_DIR/OpenTabletDriver.LICENSE"
fi
chmod +x "$PACKAGE_DIR/TabletFlow" "$PACKAGE_DIR/OpenTabletDriver.Daemon"
tar -C "$OUTPUT_DIR" -czf "$ARCHIVE_PATH" "$(basename "$PACKAGE_DIR")"
rm -rf "$PACKAGE_DIR"

echo "$ARCHIVE_PATH"
