#!/usr/bin/env python3
# ynpm scaffold generator — scaffold.sh sets NAME/VERSION/BIN/REPO_ROOT.
# Writes: package.json (@ygghq main package), finalize.mjs (postinstall),
# .github/workflows/ynpm-publish.yml (tag-triggered build + publish matrix).
import pathlib
import json
import os

name = os.environ["NAME"]
version = os.environ["VERSION"]
bins = [b for b in os.environ["BIN"].split() if b]
bin_ = bins[0]
repo_root = os.environ["REPO_ROOT"]
platforms = ["linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64",
             "win32-x64", "win32-arm64"]

package = {
    "name": f"@ygghq/{name}",
    "version": version,
    "description": f"{name} - the yggdrasilhq build, delivered by ynpm",
    "bin": {b: f"bin/{b}" for b in bins},
    "files": ["bin/", "finalize.mjs"],
    "optionalDependencies": {
        f"@ygghq/{name}-{platform}": version for platform in platforms
    },
    "scripts": {"postinstall": "node ./finalize.mjs"},
    "repository": {
        "type": "git",
        "url": f"git+https://github.com/yggdrasilhq/{name}.git",
    },
    "license": "GPL-3.0-or-later",
    "publishConfig": {"access": "public", "provenance": True},
}
with open(os.path.join(repo_root, "package.json"), "w") as f:
    f.write(json.dumps(package, indent=2) + "\n")

# The finalize is the CANONICAL template (ESM, self-deriving) — the same
# file scaffold.sh copies, so the two generators cannot drift again.
finalize = (pathlib.Path(__file__).parent / "templates" / "finalize.mjs").read_text()
with open(os.path.join(repo_root, "finalize.mjs"), "w") as f:
    f.write(finalize)

workflow = """name: ynpm publish

on:
  push:
    tags: ["v*"]
  workflow_dispatch:

permissions:
  contents: read
  id-token: write

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - {target: linux-x64, os: ubuntu-24.04, triple: ""}
          - {target: linux-arm64, os: ubuntu-24.04-arm, triple: aarch64-unknown-linux-gnu}
          - {target: darwin-x64, os: macos-15, triple: ""}
          - {target: darwin-arm64, os: macos-14, triple: ""}
          - {target: win32-x64, os: windows-2025, triple: ""}
          - {target: win32-arm64, os: windows-2025, triple: aarch64-pc-windows-msvc}
    runs-on: ${{{{ matrix.os }}}}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{{{ matrix.triple }}}}
      - name: Build release
        shell: bash
        run: |
          if [ -n "${{{{ matrix.triple }}}}" ]; then
            cargo build --release --target ${{{{ matrix.triple }}}}
          else
            cargo build --release
          fi
      - name: Pack platform package
        shell: bash
        env:
          TARGET: ${{{{ matrix.target }}}}
          TRIPLE: ${{{{ matrix.triple }}}}
        run: |
          set -e
          BUILT="target/release"; [ -n "$TRIPLE" ] && BUILT="target/$TRIPLE/release"
          EXE="{bin_}"; case "$TARGET" in win32-*) EXE="{bin_}.exe";; esac
          case "$TARGET" in
            linux-x64) PKG_OS=linux; PKG_CPU=x64 ;;
            linux-arm64) PKG_OS=linux; PKG_CPU=arm64 ;;
            darwin-x64) PKG_OS=darwin; PKG_CPU=x64 ;;
            darwin-arm64) PKG_OS=darwin; PKG_CPU=arm64 ;;
            win32-x64) PKG_OS=win32; PKG_CPU=x64 ;;
            win32-arm64) PKG_OS=win32; PKG_CPU=arm64 ;;
          esac
          mkdir -p "pack/@ygghq/{name}-$TARGET/bin"
          cp "$BUILT/$EXE" "pack/@ygghq/{name}-$TARGET/bin/{bin_}"
          printf '{"name":"@ygghq/{name}-%s","version":"{version}","bin":{"{bin_}":"bin/{bin_}"},"os":["$PKG_OS"],"cpu":["$PKG_CPU"],"license":"GPL-3.0-or-later","repository":{"type":"git","url":"git+https://github.com/yggdrasilhq/{name}.git"}}\\n' "$TARGET" > "pack/@ygghq/{name}-$TARGET/package.json"
      - name: Publish platform package
        shell: bash
        env:
          NODE_AUTH_TOKEN: ${{{{ secrets.NPM_TOKEN }}}}
        run: |
          echo '//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}' > ~/.npmrc
          cd "pack/@ygghq/{name}-${{{{ matrix.target }}}}"
          # IDEMPOTENCE WITHOUT LYING: only the genuine already-published
          # conflict is a skip; every other publish error fails the step with
          # its real output (a broken package.json must not report as a clean
          # skip — that exact mask shipped once and went green publishing
          # nothing).
          if ! npm publish --access public --provenance 2>pub_err.txt; then
            grep -qE "EPUBLISHCONFLICT|previously published|cannot publish over" pub_err.txt || { cat pub_err.txt >&2; exit 1; }
            echo "version already published — skipping (genuine conflict)"
          fi
          rm -f pub_err.txt

  publish-main:
    needs: build
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          registry-url: https://registry.npmjs.org
      - name: Publish main package
        env:
          NODE_AUTH_TOKEN: ${{{{ secrets.NPM_TOKEN }}}}
        run: npm publish --access public --provenance
"""
# The workflow's GH expressions and the {name}/{version}/{bin_} substitutions:
# the triple-brace forms above are GH literal `${{ }}` after ONE .format() pass
# on the python-level placeholders; do it explicitly instead of trusting
# str.format's brace rules.
workflow = (workflow
            .replace("${{{{", "${{")
            .replace("}}}}", "}}")
            .replace("{name}", name)
            .replace("{version}", version)
            .replace("{bin_}", bin_))
os.makedirs(os.path.join(repo_root, ".github", "workflows"), exist_ok=True)
with open(os.path.join(repo_root, ".github", "workflows", "ynpm-publish.yml"), "w") as f:
    f.write(workflow)
print(f"scaffolded @ygghq/{name} v{version} (bin: {bin_})")
