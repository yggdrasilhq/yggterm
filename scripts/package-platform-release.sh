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

APP_VERSION="$(
  awk '
    $0 == "[workspace.package]" { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${ROOT_DIR}/Cargo.toml"
)"
if [[ -z "$APP_VERSION" ]]; then
  APP_VERSION="0.0.0"
fi

HOST_TRIPLE="$(rustc "+${RUSTUP_TOOLCHAIN}" -vV | awk '/^host:/ { print $2 }')"
BUILD_CMD=("${CARGO_CMD[@]}")
if [[ -n "$TARGET_TRIPLE" && "$TARGET_TRIPLE" != "$HOST_TRIPLE" && "$TARGET_TRIPLE" == *-pc-windows-msvc ]] && command -v cargo-xwin >/dev/null 2>&1; then
  BUILD_CMD+=("xwin" "build")
elif [[ -n "$TARGET_TRIPLE" && "$TARGET_TRIPLE" != "$HOST_TRIPLE" ]] && command -v cargo-zigbuild >/dev/null 2>&1; then
  BUILD_CMD+=("zigbuild")
else
  BUILD_CMD+=("build")
fi
BUILD_CMD+=(--release -p yggterm --bin yggterm --bin yggterm-headless --bin yggterm-mock-cli --no-default-features)
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

python_zip() {
  local archive_path="$1"
  shift
  local python_cmd=""
  if command -v python3 >/dev/null 2>&1; then
    python_cmd="python3"
  elif command -v python >/dev/null 2>&1; then
    python_cmd="python"
  else
    echo "python is required to create zip archives" >&2
    exit 1
  fi
  "$python_cmd" - "$archive_path" "$@" <<'PY'
import pathlib
import sys
import zipfile

archive = pathlib.Path(sys.argv[1])
entries = [pathlib.Path(value) for value in sys.argv[2:]]
archive.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as handle:
    for entry in entries:
        if entry.is_dir():
            for child in sorted(entry.rglob("*")):
                if child.is_dir():
                    continue
                handle.write(child, child.relative_to(entry.parent))
        else:
            handle.write(entry, entry.name)
PY
}

build_macos_release_bundle() {
  local gui_binary_path="$1"
  local headless_binary_path="$2"
  local mock_cli_binary_path="$3"
  local app_path="${DIST_DIR}/Yggterm.app"
  local contents_path="${app_path}/Contents"
  local macos_path="${contents_path}/MacOS"
  local resources_path="${contents_path}/Resources"
  local icon_png="${ROOT_DIR}/assets/brand/yggterm-icon-512.png"
  local icon_file="yggterm.png"
  local iconset_path="${DIST_DIR}/yggterm.iconset"
  local app_zip_path="${DIST_DIR}/yggterm-${TARGET_LABEL}.app.zip"

  rm -rf "$app_path" "$iconset_path"
  mkdir -p "$macos_path" "$resources_path"
  cp "$gui_binary_path" "${macos_path}/Yggterm"
  chmod 0755 "${macos_path}/Yggterm" || true
  if [[ -f "$headless_binary_path" ]]; then
    cp "$headless_binary_path" "${macos_path}/yggterm-headless"
    chmod 0755 "${macos_path}/yggterm-headless" || true
  fi
  if [[ -f "$mock_cli_binary_path" ]]; then
    cp "$mock_cli_binary_path" "${macos_path}/yggterm-mock-cli"
    chmod 0755 "${macos_path}/yggterm-mock-cli" || true
  fi

  if [[ -f "$icon_png" ]]; then
    cp "$icon_png" "${resources_path}/yggterm.png"
    if command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
      mkdir -p "$iconset_path"
      while IFS=: read -r size name; do
        [[ -n "$size" ]] || continue
        sips -z "$size" "$size" "$icon_png" --out "${iconset_path}/${name}" >/dev/null
      done <<'SIZES'
16:icon_16x16.png
32:icon_16x16@2x.png
32:icon_32x32.png
64:icon_32x32@2x.png
128:icon_128x128.png
256:icon_128x128@2x.png
256:icon_256x256.png
512:icon_256x256@2x.png
512:icon_512x512.png
1024:icon_512x512@2x.png
SIZES
      if iconutil -c icns "$iconset_path" -o "${resources_path}/yggterm.icns" >/dev/null 2>&1; then
        icon_file="yggterm.icns"
      fi
      rm -rf "$iconset_path"
    fi
  fi

  cat > "${contents_path}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Yggterm</string>
  <key>CFBundleDisplayName</key>
  <string>Yggterm</string>
  <key>CFBundleIdentifier</key>
  <string>dev.yggdrasilhq.yggterm</string>
  <key>CFBundleExecutable</key>
  <string>Yggterm</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${APP_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${APP_VERSION}</string>
  <key>CFBundleIconFile</key>
  <string>${icon_file}</string>
</dict>
</plist>
PLIST

  if command -v ditto >/dev/null 2>&1; then
    rm -f "$app_zip_path"
    ditto -c -k --keepParent "$app_path" "$app_zip_path"
  else
    python_zip "$app_zip_path" "$app_path"
  fi
  checksum_file "$app_zip_path" "${app_zip_path}.sha256"
  echo "Release app bundle: ${app_zip_path}"
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

if [[ "$TARGET_LABEL" == macos-* ]]; then
  build_macos_release_bundle \
    "${DIST_DIR}/${OUT_BASENAME}" \
    "${DIST_DIR}/${HEADLESS_OUT_BASENAME}" \
    "${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}"
fi

if [[ "$TARGET_LABEL" == windows-* ]]; then
  WINDOWS_ZIP_PATH="${DIST_DIR}/yggterm-${TARGET_LABEL}.zip"
  rm -f "$WINDOWS_ZIP_PATH"
  python_zip \
    "$WINDOWS_ZIP_PATH" \
    "${DIST_DIR}/${OUT_BASENAME}" \
    "${DIST_DIR}/${HEADLESS_OUT_BASENAME}" \
    "${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}" \
    "${DIST_DIR}/${WEBVIEW2_OUT_BASENAME}"
  checksum_file "$WINDOWS_ZIP_PATH" "${WINDOWS_ZIP_PATH}.sha256"
  echo "Release zip: ${WINDOWS_ZIP_PATH}"
fi

echo "Release binary: ${DIST_DIR}/${OUT_BASENAME}"
echo "Release headless binary: ${DIST_DIR}/${HEADLESS_OUT_BASENAME}"
echo "Release mock cli: ${DIST_DIR}/${MOCK_CLI_OUT_BASENAME}"
if [[ -n "$WEBVIEW2_OUT_BASENAME" ]]; then
  echo "Release WebView2 loader: ${DIST_DIR}/${WEBVIEW2_OUT_BASENAME}"
fi
echo "Release archive: ${DIST_DIR}/yggterm-${TARGET_LABEL}.tar.gz"
