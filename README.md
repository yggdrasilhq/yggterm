# yggterm

Yggdrasil Terminal (`yggterm`) is a Rust-first terminal workspace that combines a Dioxus desktop shell shaped like Zed with a server-owned PTY runtime and an embedded xterm.js terminal surface.

The product target is not "an editor with terminals". It is a remote-first terminal application with a strong sidebar, persistent session metadata, and room for many long-lived shells across different machines.

What that means in practice is simple: a project space in Yggterm should be able to hold the live terminal, the recovered Codex transcript, and nearby notes for the same problem. The sidebar is not just a launcher. It is the memory of the workspace.

## Product direction

- Dioxus desktop is the active app shell and Zed remains the primary visual and structural reference.
- xterm.js is the active embedded terminal surface.
- Yggterm's daemon owns PTYs, restore state, and session attachment.
- The left sidebar is a vertical tree of virtual folders and sessions.
- Sidebar nodes represent session metadata, not a direct mirror of the local filesystem.
- Session entries may point at Codex workflows, SSH targets, local shells, documents, and other terminal contexts.
- Example paths should feel like `remote/prod/codex-session-tui`, `machines/pi/build-box`, or `local/design/zed-chrome-study`.
- Restoring all sessions, durable terminal metadata, and clipboard or screenshot paste into remote sessions are explicit quality-of-life goals.
- The Yggterm daemon is intended to stay authoritative underneath the UI so terminals can survive view switches and session hopping cleanly.

## Current status

This repository is still scaffolding.

- Rust workspace structure is in place.
- xterm.js is embedded directly inside the main viewport for terminal mode.
- A Dioxus desktop shell exists for fast iteration on layout and interaction.
- The current shell lives in `crates/yggterm-ui` and is the active product surface.
- Session orchestration now has a dedicated crate boundary in `crates/yggterm-server`.
- `yggterm server daemon` now owns session restore/runtime state and persists it under `~/.yggterm/server-state.json`.
- `yggterm server attach <uuid>` now creates reusable attach metadata under `~/.yggterm/runtime/attach/<uuid>/session.json` and falls back to `tmux` or the user shell.
- Workspace documents are now stored under `~/.yggterm/workspace.db` and can be loaded into the same browser tree as Codex sessions.
- `yggterm` now opens the Dioxus shell directly.
- The old CLI subcommands and the `eframe` scaffold path have been removed.
- The shell chrome is now owned locally in `yggterm-ui`, while the adjacent Zed checkout remains the visual reference stack.
- Ghostty bridge code still exists as optional legacy integration work, but it is no longer part of the default release path.

When working in this repo, optimize for getting the application closer to "Zed-shaped chrome + server-owned sessions + virtual session tree", not for deepening temporary scaffolding choices.

## Why Zed

Yggterm should inherit the parts of Zed that already work well for a dense workstation UI:

- title bar and window chrome proportions
- left-sidebar rhythm and hierarchy
- pane vocabulary
- focus routing and panel behavior
- theme behavior

The key change is that the center of the app is terminal sessions and session groups instead of editors and projects.

## Session model

`YGGTERM_HOME` defaults to `~/.yggterm`.

Today, the scaffold persists session state under `~/.yggterm/sessions`, but that storage layout is only a stepping stone. The long-term model is metadata-first: the sidebar tree should be able to describe terminal sessions that map to SSH hosts, Codex workspaces, restore groups, and other non-file concepts.

That same model now applies to documents. Notes are not treated as an afterthought bolted onto a filesystem panel. They are first-class workspace items stored in `~/.yggterm/workspace.db`, so they can sit right beside the sessions they explain.

This is the direction:

- terminal sessions stay alive in the daemon while you switch views
- preview mode and terminal mode are two lenses on the same underlying workspace
- documents live near the sessions and commands they belong to
- fast local metadata stores keep startup cheap even when the tree gets large

References to keep in mind while iterating:

- local Zed checkout: `../zed`
- Codex session UI reference: `~/gh/codex-session-tui`

## Usage

Launch the current desktop shell:

```bash
yggterm
```

## Build from source

Requirements:

- Rust stable
- Rust `1.94.0` is the current pinned toolchain for the local desktop dependency stack.
- no Ghostty checkout is required for the active embedded terminal path
- optional local checkout of `../zed` as a design/reference repo while refining shell shape

Current runtime model:

- The active terminal path is `yggterm-server` PTYs plus an embedded xterm.js surface inside the Dioxus viewport.
- Preview mode stays rendered in-process.
- Ghostty crates remain in the repo only as optional legacy integration work and are no longer required for the default app build.

Build the workspace:

```bash
cargo +1.94.0 build
```

Run locally:

```bash
cargo +1.94.0 run
```

Run via npm:

```bash
npx -y yggterm
```

The npm launcher currently supports:

- Linux `x86_64`
- macOS `x86_64`
- macOS `aarch64`
- Windows `x86_64`

## Documents from the CLI

Yggterm documents already have a simple CLI path so notes can be created or updated without opening another editor surface first.

List documents:

```bash
yggterm doc list
```

Write a document from stdin:

```bash
printf 'check deploy order\ncapture rollback notes\n' | \
  yggterm doc write /home/pi/gh/yggterm/notes/release-plan "Release Plan"
```

Read a document back:

```bash
yggterm doc cat /home/pi/gh/yggterm/notes/release-plan
```

The path is virtual. It describes where the note should appear in the sidebar tree, not where a markdown file needs to exist on disk.

## Daemon lifecycle

The desktop app talks to a long-lived `yggterm server daemon`. That daemon owns the PTYs and session restore state so terminals do not disappear just because the UI switched to preview mode or focused a different item.

You can stop the daemon explicitly:

```bash
yggterm server shutdown
```

The app also sends that shutdown request on exit so active sessions can be asked to stop gracefully instead of being dropped blindly.

## Release artifacts

Release packaging is generated from this repository and written to `dist/`.

Build the portable Linux release artifacts:

```bash
./scripts/package-release.sh linux-x86_64
```

This produces:

- `yggterm-linux-x86_64`
- `yggterm-linux-x86_64.tar.gz`
- `yggterm_<version>-<revision>_amd64.deb`
- corresponding `.sha256` files

Cross-platform release assets are produced by GitHub Actions on tag pushes:

- `linux-x86_64`
- `macos-x86_64`
- `macos-aarch64`
- `windows-x86_64`
- Debian `.deb` package for Linux

Build only the Debian package:

```bash
./scripts/package-deb.sh
```

## npm publishing

The npm launcher package lives under [`npm/`](./npm) and is published by GitHub Actions from:

- [npm-publish.yml](/home/pi/gh/yggterm/.github/workflows/npm-publish.yml)

That workflow publishes on version tag pushes and can also be run manually. It is designed for npm trusted publishing rather than a long-lived `NPM_TOKEN`.

For npm trusted publisher setup on npmjs.com, use:

- Organization or user: `yggdrasilhq`
- Repository: `yggterm`
- Workflow filename: `npm-publish.yml`

Only the filename is entered on npmjs.com, not the full path.

## Repository layout

- `apps/yggterm`: CLI entrypoint and desktop launcher
- `crates/yggterm-core`: session model, workspace documents, and settings persistence
- `crates/yggterm-server`: session orchestration, daemon/IPC state, PTY runtime, and terminal attachment
- `crates/yggterm-ui`: Dioxus desktop shell, titlebar, statusbar, and view rendering
- `crates/yggterm-platform`: platform detection
- `crates/yggterm-ghostty-bridge`: optional legacy Ghostty runtime bridge
- `scripts/`: packaging, installer, and toolchain helpers
- `debian/`: Debian package metadata

## License

- source code: `Apache-2.0`
- repository documentation (`*.md`): `CC BY-SA 4.0`

See `LICENSE`, `LICENSE-APACHE`, `LICENSE-CC-BY-SA-4.0`, and `NOTICE`.
