#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
GHOSTTY_DIR="${GHOSTTY_DIR:-${ROOT_DIR}/../ghostty}"
ZIG_BIN="${ZIG_BIN:-${HOME}/.local/bin/zig}"
OUT_DIR="${ROOT_DIR}/.yggterm-state/ghostty-prefix"

if [[ ! -x "$ZIG_BIN" ]]; then
  echo "zig binary not found at $ZIG_BIN" >&2
  echo "run ./scripts/setup-zig.sh first" >&2
  exit 1
fi

if [[ ! -d "$GHOSTTY_DIR" ]]; then
  echo "ghostty repo not found at $GHOSTTY_DIR" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

pushd "$GHOSTTY_DIR" >/dev/null
"$ZIG_BIN" build \
  --release=fast \
  --prefix "$OUT_DIR" \
  -Dapp-runtime=none \
  -Demit-exe=false \
  install
popd >/dev/null

echo "Ghostty install prefix: $OUT_DIR"
echo "Header: $OUT_DIR/include/ghostty.h"
echo "Libs in: $OUT_DIR/lib"
