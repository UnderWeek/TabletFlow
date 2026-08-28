#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OTD_ROOT="${OTD_ROOT:-$ROOT_DIR/OpenTabletDriver}"
RID="${1:-osx-arm64}"
OUTPUT_DIR="${2:-$ROOT_DIR/target/otd/$RID}"
DOTNET_COMMAND="${DOTNET_COMMAND:-dotnet}"

PROJECT="$OTD_ROOT/OpenTabletDriver.Daemon/OpenTabletDriver.Daemon.csproj"
if [[ ! -f "$PROJECT" ]]; then
    echo "OpenTabletDriver daemon project was not found: $PROJECT" >&2
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
"$DOTNET_COMMAND" publish "$PROJECT" \
    --configuration Release \
    --runtime "$RID" \
    --self-contained true \
    -p:PublishSingleFile=true \
    -p:IncludeNativeLibrariesForSelfExtract=true \
    -p:DebugType=None \
    --output "$OUTPUT_DIR"

echo "$OUTPUT_DIR/OpenTabletDriver.Daemon"
