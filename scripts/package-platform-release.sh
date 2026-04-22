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
MOCK_CLI_BIN_NAME="yggterm-mock-cli"
case "$TARGET_LABEL" in
  windows-*)
    BIN_NAME="yggterm.exe"
    HEADLESS_BIN_NAME="yggterm-headless.exe"
    MOCK_CLI_BIN_NAME="yggterm-mock-cli.exe"
    ;;
esac

BUILD_CMD=("${CARGO_CMD[@]}" build --release -p yggterm --bin yggterm --bin yggterm-headless --bin yggterm-mock-cli --no-default-features)
BIN_PATH="${ROOT_DIR}/target/release/${BIN_NAME}"
HEADLESS_BIN_PATH="${ROOT_DIR}/target/release/${HEADLESS_BIN_NAME}"
MOCK_CLI_BIN_PATH="${ROOT_DIR}/target/release/${MOCK_CLI_BIN_NAME}"
WEBVIEW2_LOADER_PATH=""
if [[ -n "$TARGET_TRIPLE" ]]; then
  BUILD_CMD+=(--target "$TARGET_TRIPLE")
  BIN_PATH="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/${BIN_NAME}"
  HEADLESS_BIN_PATH="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/${HEADLESS_BIN_NAME}"
  MOCK_CLI_BIN_PATH="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/${MOCK_CLI_BIN_NAME}"
fi

find_webview2_loader() {
  local candidate=""
  if [[ -n "$TARGET_TRIPLE" ]]; then
    candidate="$(compgen -G "${ROOT_DIR}/target/${TARGET_TRIPLE}/release/build/webview2-com-sys*/out/x64/WebView2Loader.dll" | head -n 1 || true)"
    if [[ -z "$candidate" ]]; then
      candidate="$(compgen -G "${ROOT_DIR}/target/${TARGET_TRIPLE}/debug/build/webview2-com-sys*/out/x64/WebView2Loader.dll" | head -n 1 || true)"
    fi
  fi
  if [[ -z "$candidate" ]]; then
    candidate="$(compgen -G "${HOME}/.cargo/registry/src/*/webview2-com-sys-*/x64/WebView2Loader.dll" | head -n 1 || true)"
  fi
  printf '%s' "$candidate"
}

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
MOCK_CLI_OUT_BASENAME="yggterm-mock-cli-${TARGET_LABEL}"
case "$MOCK_CLI_BIN_NAME" in
  *.exe)
    MOCK_CLI_OUT_BASENAME="${MOCK_CLI_OUT_BASENAME}.exe"
    ;;
esac
cp "$MOCK_CLI_BIN_PATH" "${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}"

WEBVIEW2_OUT_BASENAME=""
if [[ "$TARGET_LABEL" == windows-* ]]; then
  WEBVIEW2_LOADER_PATH="$(find_webview2_loader)"
  if [[ -z "$WEBVIEW2_LOADER_PATH" || ! -f "$WEBVIEW2_LOADER_PATH" ]]; then
    echo "failed to locate WebView2Loader.dll for ${TARGET_LABEL}" >&2
    exit 1
  fi
  WEBVIEW2_OUT_BASENAME="WebView2Loader-${TARGET_LABEL}.dll"
  cp "$WEBVIEW2_LOADER_PATH" "${DIST_DIR}/${WEBVIEW2_OUT_BASENAME}"
fi

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
checksum_file "${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}" "${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}.sha256"
if [[ -n "$WEBVIEW2_OUT_BASENAME" ]]; then
  checksum_file "${DIST_DIR}/${WEBVIEW2_OUT_BASENAME}" "${DIST_DIR}/${WEBVIEW2_OUT_BASENAME}.sha256"
fi

TAR_CONTENTS=(
  "${OUT_BASENAME}"
  "${OUT_BASENAME}.sha256"
  "${HEADLESS_OUT_BASENAME}"
  "${HEADLESS_OUT_BASENAME}.sha256"
  "${MOCK_CLI_OUT_BASENAME}"
  "${MOCK_CLI_OUT_BASENAME}.sha256"
)
if [[ -n "$WEBVIEW2_OUT_BASENAME" ]]; then
  TAR_CONTENTS+=(
    "${WEBVIEW2_OUT_BASENAME}"
    "${WEBVIEW2_OUT_BASENAME}.sha256"
  )
fi

tar -C "$DIST_DIR" -czf "${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz" "${TAR_CONTENTS[@]}"

checksum_file "${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz" "${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz.sha256"

echo "Release binary: ${DIST_DIR}/${OUT_BASENAME}"
echo "Release headless binary: ${DIST_DIR}/${HEADLESS_OUT_BASENAME}"
echo "Release mock cli: ${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}"
if [[ -n "$WEBVIEW2_OUT_BASENAME" ]]; then
  echo "Release WebView2 loader: ${DIST_DIR}/${WEBVIEW2_OUT_BASENAME}"
fi
echo "Release archive: ${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz"
