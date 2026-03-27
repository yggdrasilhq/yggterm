#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
BIN_PATH="${ROOT_DIR}/target/release/yggterm"
HEADLESS_BIN_PATH="${ROOT_DIR}/target/release/yggterm-headless"
TARGET_LABEL="${1:-linux-x86_64}"
RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.94.0}"
CARGO_CMD=(cargo "+${RUSTUP_TOOLCHAIN}")

mkdir -p "$DIST_DIR"

pushd "$ROOT_DIR" >/dev/null
"${CARGO_CMD[@]}" build --release -p yggterm --bin yggterm --bin yggterm-headless --no-default-features
popd >/dev/null

cp "$BIN_PATH" "$DIST_DIR/yggterm-${TARGET_LABEL}"
cp "$HEADLESS_BIN_PATH" "$DIST_DIR/yggterm-headless-${TARGET_LABEL}"
sha256sum "$DIST_DIR/yggterm-${TARGET_LABEL}" > "$DIST_DIR/yggterm-${TARGET_LABEL}.sha256"
sha256sum "$DIST_DIR/yggterm-headless-${TARGET_LABEL}" > "$DIST_DIR/yggterm-headless-${TARGET_LABEL}.sha256"

tar -C "$DIST_DIR" -czf "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz" \
  "yggterm-${TARGET_LABEL}" \
  "yggterm-${TARGET_LABEL}.sha256" \
  "yggterm-headless-${TARGET_LABEL}" \
  "yggterm-headless-${TARGET_LABEL}.sha256"

sha256sum "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz" > "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz.sha256"

echo "Release binary: $DIST_DIR/yggterm-${TARGET_LABEL}"
echo "Release headless binary: $DIST_DIR/yggterm-headless-${TARGET_LABEL}"
echo "Release archive: $DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz"
echo "Checksums generated in dist/."

"$ROOT_DIR/scripts/package-deb.sh"
