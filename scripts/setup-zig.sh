#!/usr/bin/env bash
set -euo pipefail

ARCH_KEY="x86_64-linux"
INSTALL_BASE="${HOME}/.local/zig"
INDEX_JSON="$(curl -fsSL https://ziglang.org/download/index.json)"
LATEST_STABLE="$(printf '%s' "$INDEX_JSON" | jq -r 'keys[]' | rg '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n1)"
URL="$(printf '%s' "$INDEX_JSON" | jq -r --arg v "$LATEST_STABLE" --arg a "$ARCH_KEY" '.[$v][$a].tarball')"
SHA="$(printf '%s' "$INDEX_JSON" | jq -r --arg v "$LATEST_STABLE" --arg a "$ARCH_KEY" '.[$v][$a].shasum')"
ARCHIVE="/tmp/zig-${ARCH_KEY}-${LATEST_STABLE}.tar.xz"
DEST_DIR="${INSTALL_BASE}/zig-${ARCH_KEY}-${LATEST_STABLE}"

mkdir -p "$INSTALL_BASE"
curl -fL "$URL" -o "$ARCHIVE"
printf '%s  %s\n' "$SHA" "$ARCHIVE" | sha256sum -c -
rm -rf "$DEST_DIR"
tar -xJf "$ARCHIVE" -C "$INSTALL_BASE"
ln -sfn "$DEST_DIR" "$INSTALL_BASE/current"
mkdir -p "${HOME}/.local/bin"
ln -sfn "$INSTALL_BASE/current/zig" "${HOME}/.local/bin/zig"

printf 'Installed Zig %s at %s\n' "$LATEST_STABLE" "$INSTALL_BASE/current/zig"
printf 'Ensure ~/.local/bin is on PATH\n'
"$INSTALL_BASE/current/zig" version
