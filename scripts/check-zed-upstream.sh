#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ZED_DIR="${ZED_DIR:-${ROOT_DIR}/../zed}"

if [[ ! -d "$ZED_DIR/crates" ]]; then
  echo "zed repo not found at $ZED_DIR" >&2
  exit 1
fi

required=(
  "crates/workspace/src/item.rs"
  "crates/workspace/src/workspace.rs"
  "crates/project_panel/src/project_panel.rs"
  "crates/terminal_view/src/terminal_view.rs"
  "crates/terminal_view/src/terminal_panel.rs"
)

missing=0
for rel in "${required[@]}"; do
  if [[ ! -f "$ZED_DIR/$rel" ]]; then
    echo "missing: $rel"
    missing=1
  else
    echo "ok: $rel"
  fi
done

if [[ "$missing" -ne 0 ]]; then
  exit 2
fi

echo "Zed upstream interface files are present."
