# yggterm

Install the latest build directly from GitHub Releases:

Linux and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.ps1 | iex
```

Yggterm installs into a managed user-space location, wires up desktop integration, and keeps itself current on launch when it owns that install root.

Rerun the same one-liner any time to force a manual update of a direct install.

## What yggterm is

Yggdrasil Terminal (`yggterm`) is a remote-first terminal workspace built in Rust. It is meant for the way people actually work with terminals now: many long-lived sessions, many machines, restored context, Codex-heavy workflows, quick notes beside the terminal, and a sidebar that remembers how everything is related.

The product target is not “an editor with a terminal panel.” The terminal is the center. Everything else exists to help you find, restore, explain, automate, and organize terminal work without losing momentum.

That means:

- one desktop shell for sessions, notes, recipes, and metadata
- a daemon-owned PTY runtime so terminals survive view switches cleanly
- a virtual tree of sessions and documents, not a raw filesystem browser
- first-class support for Codex sessions, local shells, SSH sessions, and workspace notes
- fast startup from local metadata under `~/.yggterm`

## Why use it

Yggterm is trying to solve a specific problem: terminal work is usually scattered across shell history, tmux panes, half-remembered commands, AI transcripts, scratch files, and SSH tabs. That fragmentation kills flow.

Yggterm keeps those things nearby:

- the live terminal
- the recovered Codex session or other session preview
- notes and executable recipe documents
- session metadata and restore state

The left tree is not just a launcher. It is the workspace memory.

## Install and update model

The direct install path above is the mainline channel right now.

What it does:

- detects your OS and architecture
- downloads the latest matching GitHub Release artifact
- installs it into a managed user-space root
- creates desktop integration for direct installs
- refreshes integration when assets change
- self-updates on launch when a newer direct-release build is available
- shows a lightweight in-app update notification before restarting into the new version
- reruns of the install one-liner act as an explicit manual updater for direct installs

Package-managed installs behave differently on purpose:

- `.deb`, Homebrew, Winget, Scoop, Flatpak, and Snap installs are detected
- those installs switch to notify-only update mode
- Yggterm tells the user to update with the matching package manager instead of mutating the install behind its back

This split keeps the fast-moving direct channel frictionless while respecting native platform ownership when the app was installed by a package manager.

## Current product shape

Today the active stack is:

- Dioxus desktop shell for the app surface
- xterm.js embedded in the main viewport for terminal mode
- `yggterm-server` as the daemon-owned PTY/runtime layer
- SQLite-backed local metadata for workspace documents and generated titles

That means the current app already supports:

- embedded terminal mode in the main canvas
- rendered preview mode for stored sessions and documents
- daemon-owned live sessions that survive switching between items
- local shell sessions, SSH-backed sessions, and Codex-style agent sessions
- workspace notes stored in `~/.yggterm/workspace.db`
- recipe documents that can be edited in preview mode and executed in terminal mode
- generated session titles through a configured LiteLLM endpoint
- direct install with self-update and package-manager-aware notify-only mode

## The workspace model

`YGGTERM_HOME` defaults to `~/.yggterm`.

The long-term model is metadata-first. The tree should describe work, not just folders on disk. A path in Yggterm is allowed to mean:

- a Codex workspace
- a local shell area
- an SSH target or machine group
- a session-derived document cluster
- a restore grouping
- a future recipe or automation bucket

Documents are first-class workspace items, not a bolted-on notes tab. They live in `~/.yggterm/workspace.db` and appear in the same tree as sessions.

This is the direction:

- terminals stay alive in the daemon while you switch views
- preview and terminal are two lenses on the same workspace item
- notes live beside the sessions they explain
- recipes carry replay commands and source-session metadata
- local shells, Codex sessions, and SSH sessions share one runtime model
- fast local metadata keeps startup cheap even when the tree gets large

## Tree workflow

The sidebar is now an active workspace surface.

Examples of what you can do:

- right-click a folder or group and create a new virtual group there
- right-click a folder or group and create a new Codex session there
- right-click a folder or group and create a local shell in that context
- right-click a folder or group and create a nearby document
- right-click a folder or group and create a nearby recipe
- right-click a folder or group and move the selected document there
- right-click a stored session and create a note beside it
- right-click a stored session and create a recipe derived from it
- regenerate generated titles for a session when needed

The intent is simple: organizing the tree should naturally create the right place to work next.

Virtual groups are stored under `~/.yggterm/workspace.db`, so they load quickly and do not depend on walking a large on-disk workspace before the UI becomes useful.

The SSH connect rail is guided on purpose:

- type `user@ip`, `user@host`, or a shortcut from your `~/.ssh/config` such as `dev`
- optionally add a remote prefix if you want the session to start inside `machinectl`, `tmux`, or another remote wrapper
- reconnecting to the same SSH target focuses the existing live session instead of spawning a duplicate

## Documents and recipes

Yggterm documents already have a CLI path so notes can be created or updated outside the UI.

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

The path is virtual. It controls where the item appears in the Yggterm tree; it is not a requirement to mirror files on disk.

Inside the desktop shell:

- notes open in preview mode for editing
- recipes can store replay commands and source cwd metadata
- `Run Here` saves the recipe and reuses the current document-backed terminal view
- `Run In New Session` saves the recipe and starts a fresh daemon-owned shell session from the recipe cwd and replay commands

That is the beginning of a bigger idea: a workspace can hold the terminal, the explanation, and the repeatable command flow together.

## Daemon lifecycle

The desktop app talks to a long-lived `yggterm server daemon`.

That daemon owns:

- PTYs
- session restore state
- live-session lifecycle
- terminal attachment
- graceful shutdown behavior

This matters because terminals should not disappear just because the UI changed view or focused a different item.

Current lifecycle behavior:

- live PTYs remain available while you switch between preview and terminal
- the UI asks the daemon to shut down on exit
- Codex-flavored sessions receive `/quit`
- plain shells receive `exit`
- the PTY manager escalates only if the graceful stop path fails

For development, there is also a smoke command:

```bash
yggterm server smoke
```

That boots a temporary daemon home, starts a local shell session, and shuts it back down cleanly.

You can stop the daemon explicitly too:

```bash
yggterm server shutdown
```

## Build from source

Requirements:

- Rust `1.94.0`
- Node is not required for the release/install path
- no Ghostty checkout is required for the default embedded terminal path

Build:

```bash
cargo +1.94.0 build
```

Run:

```bash
cargo +1.94.0 run
```

## Release artifacts

Release packaging is generated from this repository and written to `dist/`.

Build the portable Linux release artifacts:

```bash
./scripts/package-release.sh linux-x86_64
```

Build only the Debian package:

```bash
./scripts/package-deb.sh
```

Current GitHub release matrix:

- `linux-x86_64`
- `linux-aarch64`
- `macos-x86_64`
- `macos-aarch64`
- `windows-x86_64`
- `windows-aarch64`
- Debian `.deb`

Each release artifact should also ship with a checksum.

## Repository layout

- `apps/yggterm`: CLI entrypoint and desktop launcher
- `crates/yggterm-core`: settings, workspace store, title generation, install detection, and browser state
- `crates/yggterm-server`: daemon, IPC, PTY runtime, and session orchestration
- `crates/yggterm-ui`: Dioxus shell and app interaction surface
- `crates/yggterm-platform`: platform detection helpers
- `crates/yggterm-ghostty-bridge`: optional legacy bridge code, not part of the default path
- `scripts/`: installers, packaging, and release helpers
- `debian/`: Debian package metadata

## License

- source code: `Apache-2.0`
- repository documentation (`*.md`): `CC BY-SA 4.0`

See `LICENSE`, `LICENSE-APACHE`, `LICENSE-CC-BY-SA-4.0`, and `NOTICE`.
