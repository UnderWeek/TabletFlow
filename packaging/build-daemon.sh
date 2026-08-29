#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OTD_ROOT="${OTD_ROOT:-$ROOT_DIR/OpenTabletDriver}"
RID="${1:-osx-arm64}"
OUTPUT_DIR="${2:-$ROOT_DIR/target/otd/$RID}"
DOTNET_COMMAND="${DOTNET_COMMAND:-dotnet}"

if [[ "$OUTPUT_DIR" != /* ]]; then
    OUTPUT_DIR="$ROOT_DIR/$OUTPUT_DIR"
fi

PROJECT="$OTD_ROOT/OpenTabletDriver.Daemon/OpenTabletDriver.Daemon.csproj"
if [[ ! -f "$PROJECT" ]]; then
    echo "OpenTabletDriver daemon project was not found: $PROJECT" >&2
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
PUBLISH_OPTIONS=(
    --configuration Release
    --runtime "$RID"
    --self-contained true
    -p:PublishSingleFile=true
    -p:UseAppHost=true
    -p:IncludeNativeLibrariesForSelfExtract=true
    -p:DebugType=None
    --output "$OUTPUT_DIR"
)
if [[ "$RID" == osx-* ]]; then
    PUBLISH_OPTIONS+=( -p:_EnableMacOSCodeSign=false )
fi

"$DOTNET_COMMAND" publish "$PROJECT" "${PUBLISH_OPTIONS[@]}" >&2

if [[ "$RID" == win-* ]]; then
    DAEMON_PATH="$OUTPUT_DIR/OpenTabletDriver.Daemon.exe"
else
    DAEMON_PATH="$OUTPUT_DIR/OpenTabletDriver.Daemon"
fi

if [[ ! -f "$DAEMON_PATH" ]]; then
    DAEMON_PATH="$(find "$OUTPUT_DIR" -type f \( \
        -name 'OpenTabletDriver.Daemon' -o \
        -name 'OpenTabletDriver.Daemon.exe' \
    \) -print -quit 2>/dev/null || true)"
fi

if [[ -z "$DAEMON_PATH" || ! -f "$DAEMON_PATH" ]]; then
    echo "OpenTabletDriver daemon binary was not produced in $OUTPUT_DIR" >&2
    find "$OUTPUT_DIR" -type f -print >&2 2>/dev/null || true
    exit 1
fi

echo "$DAEMON_PATH"
