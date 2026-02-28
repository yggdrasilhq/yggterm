#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
BIN_PATH="${ROOT_DIR}/target/release/yggterm"
TARGET_LABEL="${1:-linux-x86_64}"

mkdir -p "$DIST_DIR"

pushd "$ROOT_DIR" >/dev/null
cargo build --release
popd >/dev/null

cp "$BIN_PATH" "$DIST_DIR/yggterm-${TARGET_LABEL}"
sha256sum "$DIST_DIR/yggterm-${TARGET_LABEL}" > "$DIST_DIR/yggterm-${TARGET_LABEL}.sha256"

tar -C "$DIST_DIR" -czf "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz" \
  "yggterm-${TARGET_LABEL}" \
  "yggterm-${TARGET_LABEL}.sha256"

sha256sum "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz" > "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz.sha256"

echo "Release binary: $DIST_DIR/yggterm-${TARGET_LABEL}"
echo "Release archive: $DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz"
echo "Checksums generated in dist/."

"$ROOT_DIR/scripts/package-deb.sh"
