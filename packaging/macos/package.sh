#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
OUTPUT_DIR="${1:-$ROOT_DIR/dist}"
DAEMON_PATH="${OTD_DAEMON_PATH:-}"
TABLETFLOW_BINARY_PATH="${TABLETFLOW_BINARY_PATH:-$ROOT_DIR/target/release/tabletflow}"
VERSION="${TABLETFLOW_VERSION:-0.1.0}"
ARCH="${TABLETFLOW_ARCH:-$(uname -m)}"

if [[ ! -f "$TABLETFLOW_BINARY_PATH" ]]; then
    echo "TABLETFLOW_BINARY_PATH must point to the TabletFlow executable" >&2
    exit 1
fi
if [[ -z "$DAEMON_PATH" || ! -f "$DAEMON_PATH" ]]; then
    echo "OTD_DAEMON_PATH must point to OpenTabletDriver.Daemon" >&2
    exit 1
fi

APP_DIR="$OUTPUT_DIR/TabletFlow.app"
STAGING_DIR="$OUTPUT_DIR/.dmg-staging"
DMG_PATH="$OUTPUT_DIR/TabletFlow-${VERSION}-macos-${ARCH}.dmg"

cleanup() {
    rm -rf "$STAGING_DIR"
}
trap cleanup EXIT

rm -rf "$APP_DIR" "$STAGING_DIR" "$DMG_PATH"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources" "$STAGING_DIR"

cp "$TABLETFLOW_BINARY_PATH" "$APP_DIR/Contents/MacOS/TabletFlow"
cp "$DAEMON_PATH" "$APP_DIR/Contents/MacOS/OpenTabletDriver.Daemon"
cp "$ROOT_DIR/packaging/macos/Info.plist" "$APP_DIR/Contents/Info.plist"
if [[ "$VERSION" != "0.1.0" ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP_DIR/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VERSION" "$APP_DIR/Contents/Info.plist"
fi
if [[ -n "${OTD_LICENSE_PATH:-}" && -f "$OTD_LICENSE_PATH" ]]; then
    cp "$OTD_LICENSE_PATH" "$APP_DIR/Contents/Resources/OpenTabletDriver.LICENSE"
fi
chmod +x "$APP_DIR/Contents/MacOS/TabletFlow" "$APP_DIR/Contents/MacOS/OpenTabletDriver.Daemon"

ln -s /Applications "$STAGING_DIR/Applications"
cp -R "$APP_DIR" "$STAGING_DIR/TabletFlow.app"

SOURCE_SIZE_KB="$(du -sk "$STAGING_DIR" | awk '{print $1}')"
IMAGE_SIZE_KB=$((SOURCE_SIZE_KB + 512 * 1024))

for attempt in 1 2 3; do
    rm -f "$DMG_PATH"
    echo "Creating macOS disk image (attempt $attempt/3)..." >&2
    if hdiutil create \
        -ov \
        -nospotlight \
        -noanyowners \
        -size "${IMAGE_SIZE_KB}k" \
        -fs APFS \
        -volname TabletFlow \
        -srcfolder "$STAGING_DIR" \
        -format UDZO \
        "$DMG_PATH"; then
        break
    fi

    if [[ "$attempt" -eq 3 ]]; then
        echo "Unable to create macOS disk image after 3 attempts." >&2
        exit 1
    fi
    sleep "$attempt"
done

echo "$DMG_PATH"
