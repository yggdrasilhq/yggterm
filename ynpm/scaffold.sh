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
# Optional PKG_NAME override: the PACKAGE name when it differs from
# the repo basename (ztlkasten ships as @ygghq/kasten per the spec).
pkgname="${PKG_NAME:-$name}"
[ ${#bins[@]} -eq 0 ] && { echo "usage: scaffold.sh <repo> <bin> [bin...]"; exit 1; }
bin="${bins[0]}"
version="$(grep -m1 '^version' "$repo/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')"

# ── package.json ─────────────────────────────────────────────────────────────
opts=$(printf ', "@ygghq/%s-%s": "%s"' "$pkgname" \
  linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64 win32-arm64 \
  "$version" | sed 's/^, //')
{
printf '{\n  "name": "@ygghq/%s",\n  "version": "%s",\n' "$pkgname" "$version"
printf '  "description": "%s — the yggdrasilhq build, delivered by ynpm",\n' "$pkgname"
printf '  "bin": { "%s": "bin/%s" },\n' "$bin" "$bin"
printf '  "files": ["bin/", "finalize.mjs"],\n'
printf '  "optionalDependencies": { %s },\n' "$opts"
printf '  "scripts": { "postinstall": "node ./finalize.mjs" },\n'
printf '  "repository": { "type": "git", "url": "git+https://github.com/yggdrasilhq/%s.git" },\n' "$pkgname"
printf '  "license": "GPL-3.0-or-later",\n  "publishConfig": { "access": "public", "provenance": true }\n}\n'
} > "$repo/package.json"

# ── finalize.mjs ─────────────────────────────────────────────────────────────
# The finalize template derives everything at runtime from package.json
# (bin key, package name, platform) — no placeholder substitution needed.
cp "$here/templates/finalize.mjs" "$repo/finalize.mjs"

# ── publish workflow (generated in python: GH expressions vs shell vars) ────
mkdir -p "$repo/.github/workflows"
NAME="$pkgname" VERSION="$version" BIN="${bins[*]}" REPO_ROOT="$repo" python3 "$(dirname -- "${BASH_SOURCE[0]}")/scaffold_gen.py"


echo "scaffolded $repo (@ygghq/$pkgname v$version, bin: ${bins[*]})"
