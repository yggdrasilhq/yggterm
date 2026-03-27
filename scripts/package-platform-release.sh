#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
TARGET_LABEL="${1:?usage: package-platform-release.sh <label> [target-triple]}"
TARGET_TRIPLE="${2:-}"
RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.94.0}"
CARGO_CMD=(cargo "+${RUSTUP_TOOLCHAIN}")

mkdir -p "$DIST_DIR"

BIN_NAME="yggterm"
HEADLESS_BIN_NAME="yggterm-headless"
case "$TARGET_LABEL" in
  windows-*)
    BIN_NAME="yggterm.exe"
    HEADLESS_BIN_NAME="yggterm-headless.exe"
    ;;
esac

BUILD_CMD=("${CARGO_CMD[@]}" build --release -p yggterm --bin yggterm --bin yggterm-headless --no-default-features)
BIN_PATH="${ROOT_DIR}/target/release/${BIN_NAME}"
HEADLESS_BIN_PATH="${ROOT_DIR}/target/release/${HEADLESS_BIN_NAME}"
if [[ -n "$TARGET_TRIPLE" ]]; then
  BUILD_CMD+=(--target "$TARGET_TRIPLE")
  BIN_PATH="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/${BIN_NAME}"
  HEADLESS_BIN_PATH="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/${HEADLESS_BIN_NAME}"
fi

pushd "$ROOT_DIR" >/dev/null
"${BUILD_CMD[@]}"
popd >/dev/null

OUT_BASENAME="yggterm-${TARGET_LABEL}"
case "$BIN_NAME" in
  *.exe)
    OUT_BASENAME="${OUT_BASENAME}.exe"
    ;;
esac

cp "$BIN_PATH" "${DIST_DIR}/${OUT_BASENAME}"
HEADLESS_OUT_BASENAME="yggterm-headless-${TARGET_LABEL}"
case "$HEADLESS_BIN_NAME" in
  *.exe)
    HEADLESS_OUT_BASENAME="${HEADLESS_OUT_BASENAME}.exe"
    ;;
esac
cp "$HEADLESS_BIN_PATH" "${DIST_DIR}/${HEADLESS_OUT_BASENAME}"

checksum_file() {
  local file="$1"
  local out="$2"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" > "$out"
  else
    shasum -a 256 "$file" > "$out"
  fi
}

checksum_file "${DIST_DIR}/${OUT_BASENAME}" "${DIST_DIR}/${OUT_BASENAME}.sha256"
checksum_file "${DIST_DIR}/${HEADLESS_OUT_BASENAME}" "${DIST_DIR}/${HEADLESS_OUT_BASENAME}.sha256"

tar -C "$DIST_DIR" -czf "${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz" \
  "${OUT_BASENAME}" \
  "${OUT_BASENAME}.sha256" \
  "${HEADLESS_OUT_BASENAME}" \
  "${HEADLESS_OUT_BASENAME}.sha256"

checksum_file "${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz" "${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz.sha256"

echo "Release binary: ${DIST_DIR}/${OUT_BASENAME}"
echo "Release headless binary: ${DIST_DIR}/${HEADLESS_OUT_BASENAME}"
echo "Release archive: ${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz"
