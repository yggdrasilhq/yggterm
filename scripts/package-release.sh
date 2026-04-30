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

maybe_refresh_release_codex_cli() {
  if [[ "${YGGTERM_RELEASE_CODEX_REFRESH:-1}" == "0" ]]; then
    echo "Skipping managed Codex CLI refresh because YGGTERM_RELEASE_CODEX_REFRESH=0."
    return
  fi
  if [[ ! -x "$HEADLESS_BIN_PATH" ]]; then
    echo "warning: cannot refresh managed Codex CLI; missing executable $HEADLESS_BIN_PATH" >&2
    return
  fi
  local report_path="${DIST_DIR}/managed-codex-refresh-${TARGET_LABEL}.jsonl"
  rm -f "$report_path"
  echo "Queueing managed Codex CLI refresh/check..."
  if "$HEADLESS_BIN_PATH" server monitor --scenario managed-cli-refresh --background --jsonl-out "$report_path"; then
    if grep -q '"kind":"error"' "$report_path"; then
      echo "warning: managed Codex CLI refresh/check reported an error; continuing release packaging: $report_path" >&2
    else
      echo "Managed Codex CLI refresh report: $report_path"
    fi
  else
    echo "warning: managed Codex CLI refresh/check failed; continuing release packaging" >&2
  fi
}

pushd "$ROOT_DIR" >/dev/null
"${CARGO_CMD[@]}" build --release -p yggterm --bin yggterm --bin yggterm-headless --no-default-features
popd >/dev/null
maybe_refresh_release_codex_cli

cp "$BIN_PATH" "$DIST_DIR/yggterm-${TARGET_LABEL}"
cp "$HEADLESS_BIN_PATH" "$DIST_DIR/yggterm-headless-${TARGET_LABEL}"
checksum_file "$DIST_DIR/yggterm-${TARGET_LABEL}" "$DIST_DIR/yggterm-${TARGET_LABEL}.sha256"
checksum_file "$DIST_DIR/yggterm-headless-${TARGET_LABEL}" "$DIST_DIR/yggterm-headless-${TARGET_LABEL}.sha256"

tar -C "$DIST_DIR" -czf "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz" \
  "yggterm-${TARGET_LABEL}" \
  "yggterm-${TARGET_LABEL}.sha256" \
  "yggterm-headless-${TARGET_LABEL}" \
  "yggterm-headless-${TARGET_LABEL}.sha256"

checksum_file "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz" "$DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz.sha256"

echo "Release binary: $DIST_DIR/yggterm-${TARGET_LABEL}"
echo "Release headless binary: $DIST_DIR/yggterm-headless-${TARGET_LABEL}"
echo "Release archive: $DIST_DIR/yggterm-${TARGET_LABEL}.tar.gz"
echo "Checksums generated in dist/."

"$ROOT_DIR/scripts/package-deb.sh"
