#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
PREFIX_DIR="${ROOT_DIR}/.yggterm-state/ghostty-prefix"
LIB_DIR="${PREFIX_DIR}/lib"
TARGET_LABEL="${1:-linux-x86_64}"
PKG_DIR="${DIST_DIR}/yggterm-${TARGET_LABEL}-ghostty-ffi"
RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.94.0}"
CARGO_CMD=(cargo "+${RUSTUP_TOOLCHAIN}")

if [[ ! -d "$LIB_DIR" ]]; then
  "${ROOT_DIR}/scripts/build-ghostty-lib.sh"
fi

mkdir -p "$DIST_DIR"

checksum_file() {
  local file="$1"
  local out="$2"
  local dir
  local base
  dir="$(dirname "$file")"
  base="$(basename "$file")"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$dir" && sha256sum "$base") > "$out"
  else
    (cd "$dir" && shasum -a 256 "$base") > "$out"
  fi
}

pushd "$ROOT_DIR" >/dev/null
GHOSTTY_DIR="${ROOT_DIR}/../ghostty" \
GHOSTTY_LIB_DIR="$LIB_DIR" \
"${CARGO_CMD[@]}" build --release --features ghostty-ffi
popd >/dev/null

rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR/lib"
cp "$ROOT_DIR/target/release/yggterm" "$PKG_DIR/yggterm"
cp "$LIB_DIR/libghostty.so" "$PKG_DIR/lib/libghostty.so"
cp "$LIB_DIR/libghostty.a" "$PKG_DIR/lib/libghostty.a"

cat > "$PKG_DIR/run.sh" <<'RUNEOF'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
export LD_LIBRARY_PATH="$DIR/lib:${LD_LIBRARY_PATH:-}"
exec "$DIR/yggterm" "$@"
RUNEOF
chmod +x "$PKG_DIR/run.sh"

( cd "$DIST_DIR" && tar -czf "yggterm-${TARGET_LABEL}-ghostty-ffi.tar.gz" "yggterm-${TARGET_LABEL}-ghostty-ffi" )
checksum_file "$DIST_DIR/yggterm-${TARGET_LABEL}-ghostty-ffi.tar.gz" "$DIST_DIR/yggterm-${TARGET_LABEL}-ghostty-ffi.tar.gz.sha256"

echo "FFI release archive: $DIST_DIR/yggterm-${TARGET_LABEL}-ghostty-ffi.tar.gz"
