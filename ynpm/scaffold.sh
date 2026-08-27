#!/usr/bin/env bash
# ynpm scaffold — write the ynpm packaging into a yggdrasilhq repo checkout.
#
#   ynpm/scaffold.sh <repo-checkout> <bin-name> [more-bin-names...]
#
# Writes:
#   package.json                  the @ygghq/<repo> main package (shim + pins)
#   finalize.mjs                  the postinstall finalizer (copied template)
#   .github/workflows/ynpm-publish.yml   tag-triggered build + publish matrix
#
# The workflow publishes ONLY when NPM_TOKEN is present (repository secret),
# so the scaffolding lands dormant and activates with the org + token step.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd -- "$1" && pwd)"
name="$(basename -- "$repo")"
shift
bins=("$@")
[ ${#bins[@]} -eq 0 ] && { echo "usage: scaffold.sh <repo> <bin> [bin...]"; exit 1; }
bin="${bins[0]}"
version="$(grep -m1 '^version' "$repo/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')"

# ── package.json ─────────────────────────────────────────────────────────────
opts=$(printf ', "@ygghq/%s-%s": "%s"' "$name" \
  linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64 win32-arm64 \
  "$version" | sed 's/^, //')
{
printf '{\n  "name": "@ygghq/%s",\n  "version": "%s",\n' "$name" "$version"
printf '  "description": "%s — the yggdrasilhq build, delivered by ynpm",\n' "$name"
printf '  "bin": { "%s": "bin/%s" },\n' "$bin" "$bin"
printf '  "files": ["bin/", "finalize.mjs"],\n'
printf '  "optionalDependencies": { %s },\n' "$opts"
printf '  "scripts": { "postinstall": "node ./finalize.mjs" },\n'
printf '  "repository": { "type": "git", "url": "git+https://github.com/yggdrasilhq/%s.git" },\n' "$name"
printf '  "license": "GPL-3.0-or-later",\n  "publishConfig": { "access": "public", "provenance": true }\n}\n'
} > "$repo/package.json"

# ── finalize.mjs ─────────────────────────────────────────────────────────────
sed -e "s/__BIN__/$bin/g" -e "s/__PACKAGE__/@ygghq\/$name/g" \
    "$here/templates/finalize.mjs" > "$repo/finalize.mjs"

# ── publish workflow (generated in python: GH expressions vs shell vars) ────
mkdir -p "$repo/.github/workflows"
NAME="$name" VERSION="$version" BIN="${bins[*]}" REPO_ROOT="$repo" python3 "$(dirname -- "${BASH_SOURCE[0]}")/scaffold_gen.py"


echo "scaffolded $repo (@ygghq/$name v$version, bin: ${bins[*]})"
