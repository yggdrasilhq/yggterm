# yggterm

Yggdrasil Terminal (`yggterm`) is a terminal workspace for people who operate many shells at once.
It is built around a persistent session tree in `~/.yggterm`, a desktop GUI, and an eventual Ghostty-backed terminal viewport with a Zed-inspired application shell.

## Status

`yggterm` is usable today as an early GUI session manager.

- It has a desktop app: `yggterm gui`
- It persists session folders under `~/.yggterm/sessions`
- It opens and manages live shell processes per selected session
- It includes a settings pane, theme switching, and a session tree filter
- It packages a Ghostty-enabled runtime path, but the center viewport is still PTY-backed UI rather than full embedded Ghostty rendering

## Install

Quick install from the latest GitHub release:

```bash
curl -fsSL https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.sh | sh
```

Installer behavior:

- On Debian-like systems with `dpkg` and `sudo`, it installs the latest `.deb`
- Otherwise it downloads the matching release tarball and installs `yggterm` into `~/.local/bin`
- Current automated asset detection supports `Linux x86_64`

Manual install from a release:

1. Download a release asset from GitHub Releases.
2. On Debian/Ubuntu/Raspberry Pi OS:

```bash
sudo dpkg -i yggterm_<version>_amd64.deb
```

3. Or install the standalone binary:

```bash
tar -xzf yggterm-linux-x86_64.tar.gz
chmod +x yggterm-linux-x86_64
mv yggterm-linux-x86_64 ~/.local/bin/yggterm
```

## Usage

Initialize local state:

```bash
yggterm init
```

Create nested sessions:

```bash
yggterm mk-session prod/api
yggterm mk-session prod/db
yggterm mk-session staging/web
```

Print the stored tree:

```bash
yggterm tree
```

Launch the desktop app:

```bash
yggterm gui
```

Inspect runtime/backend status:

```bash
yggterm doctor
```

Example `doctor` output for a packaged build:

```text
Host platform: Linux
YGGTERM_HOME: /home/user/.yggterm
Ghostty header discovered: not found
Ghostty bridge init status: enabled
```

Notes:

- `Ghostty header discovered` may be `not found` on installed machines; that is fine
- What matters for end users is whether the Ghostty bridge runtime is enabled

## GUI behavior

Current GUI features:

- left session tree with filter and quick session creation
- top chrome with hamburger menu actions
- persisted settings in `~/.yggterm/settings.json`
- light and dark Zed-inspired themes
- live shell processes in the main workspace
- multiple open terminal tabs with focus and close controls

Current limitation:

- the terminal viewport is not yet drawing embedded Ghostty surfaces
- Ghostty is packaged and linked for runtime, but the actual center-pane rendering integration is still in progress

## Build From Source

Requirements:

- Rust stable
- Zig stable
- adjacent checkouts of `../ghostty` and `../zed` for integration work

Install Zig:

```bash
./scripts/setup-zig.sh
```

Build Ghostty runtime artifacts:

```bash
./scripts/build-ghostty-lib.sh
```

Build release:

```bash
cargo build --release
```

Run locally:

```bash
./target/release/yggterm gui
```

## Release Artifacts

Release packaging is generated from this repository and written to `dist/`.

Build all public release artifacts:

```bash
./scripts/package-release.sh linux-x86_64
```

This produces:

- `yggterm-linux-x86_64`
- `yggterm-linux-x86_64.tar.gz`
- `yggterm_<version>-<revision>_amd64.deb`
- corresponding `.sha256` files

Build only the Debian package:

```bash
./scripts/package-deb.sh
```

Build the FFI bundle archive:

```bash
./scripts/package-release-ffi.sh linux-x86_64
```

## Repository Layout

- `apps/yggterm`: CLI entrypoint and desktop app
- `crates/yggterm-core`: session model and settings persistence
- `crates/yggterm-ui`: shared UI helpers
- `crates/yggterm-platform`: platform detection
- `crates/yggterm-ghostty-bridge`: Ghostty runtime bridge
- `crates/yggterm-zed-shell`: Zed-integration planning surface
- `scripts/`: packaging, installer, and toolchain helpers
- `debian/`: Debian package metadata

## License

Apache-2.0. See `LICENSE` and `NOTICE`.
