# yggterm

Yggdrasil Terminal (`yggterm`) is a Rust-first terminal workspace that aims to combine Ghostty terminal semantics with a GPUI shell shaped like Zed.

The product target is not "an editor with terminals". It is a remote-first terminal application with a strong sidebar, persistent session metadata, and room for many long-lived shells across different machines.

## Product direction

- GPUI is the intended app shell and Zed is the primary visual and structural reference.
- Ghostty is the terminal engine contract.
- The left sidebar is a vertical tree of virtual folders and sessions.
- Sidebar nodes represent session metadata, not a direct mirror of the local filesystem.
- Session entries may point at Codex workflows, SSH targets, local shells, and other terminal contexts.
- Example paths should feel like `remote/prod/codex-session-tui`, `machines/pi/ghostty-admin`, or `local/design/zed-chrome-study`.
- Restoring all sessions, durable terminal metadata, and clipboard or screenshot paste into remote sessions are explicit quality-of-life goals.

## Current status

This repository is still scaffolding.

- Rust workspace structure is in place.
- Ghostty bridge packaging and runtime probing exist.
- A temporary desktop shell exists for fast iteration on layout and interaction.
- The current shell is useful for shape and workflow experiments, but it is not the final GPUI implementation yet.
- `yggterm` now opens the GPUI shell prototype directly.
- The old CLI subcommands and the `eframe` scaffold path have been removed.
- The GPUI shell should prefer direct reuse of Zed crates such as `ui`, `theme`, and `settings` over local visual reimplementation.
- Mock sidebars, tabs, docks, and bodies are acceptable only as placeholders inside Zed-derived chrome while Ghostty embedding is still pending.

When working in this repo, optimize for getting the application closer to "Zed chrome + Ghostty sessions + virtual session tree", not for deepening temporary scaffolding choices.

## Why Zed

Yggterm should inherit the parts of Zed that already work well for a dense workstation UI:

- title bar and window chrome proportions
- left-sidebar rhythm and hierarchy
- pane and tab vocabulary
- focus routing and panel behavior
- theme and settings behavior

The key change is that the center of the app is Ghostty sessions and session groups instead of editors and projects.

## Session model

`YGGTERM_HOME` defaults to `~/.yggterm`.

Today, the scaffold persists session state under `~/.yggterm/sessions`, but that storage layout is only a stepping stone. The long-term model is metadata-first: the sidebar tree should be able to describe terminal sessions that map to SSH hosts, Codex workspaces, Ghostty sessions, restore groups, and other non-file concepts.

References to keep in mind while iterating:

- local Zed checkout: `../zed`
- local Ghostty checkout: `../ghostty`
- Codex session UI reference: `~/gh/codex-session-tui`

## Usage

Launch the current desktop shell:

```bash
yggterm
```

## Build from source

Requirements:

- Rust stable
- Rust `1.94.0` is the current pinned toolchain for the local GPUI/Zed dependency stack.
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

Current upstream constraint:

- On Linux, `libghostty` links and the Yggterm server can launch Ghostty for terminal mode, but Ghostty's current embedded surface host only exposes macOS/iOS platform views. The in-viewport terminal path therefore still falls back to an external Ghostty process on Linux until upstream surface hosting expands.
- `crates/yggterm-gtk-glue` is the reserved Linux bypass layer for a GTK-hosted Ghostty path while upstream embedding remains unavailable there.

Build the workspace:

```bash
cargo +1.94.0 build
```

Run locally:

```bash
cargo +1.94.0 run
```

## Release artifacts

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

## Repository layout

- `apps/yggterm`: CLI entrypoint and current desktop scaffold
- `crates/yggterm-core`: session model and settings persistence
- `crates/yggterm-ui`: shared UI helpers
- `crates/yggterm-platform`: platform detection
- `crates/yggterm-ghostty-bridge`: Ghostty runtime bridge
- `scripts/`: packaging, installer, and toolchain helpers
- `debian/`: Debian package metadata

## License

- source code: `Apache-2.0`
- repository documentation (`*.md`): `CC BY-SA 4.0`

See `LICENSE`, `LICENSE-APACHE`, `LICENSE-CC-BY-SA-4.0`, and `NOTICE`.
