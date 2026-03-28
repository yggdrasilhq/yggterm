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

Direct installs also ship `yggterm-mock-cli`, a small diagnostic CLI for probing daemon state, startup behavior, and native integration problems from the same installed runtime.

Yggterm also ships an always-on event trace so sluggish sessions can be debugged after the fact or while the app is still running:

```bash
yggterm-headless server trace tail 200
yggterm-headless server trace follow 200 500
yggterm-headless server trace bundle 200 --screenshot > yggterm-trace.json
```

The trace lives at `~/.yggterm/event-trace.jsonl` and is designed to stay on during dogfooding, with lightweight rotation once it gets large.

## What yggterm is

Yggdrasil Terminal (`yggterm`) is a remote-first terminal workspace built in Rust.

The short version is:

**Zen Browser for AI terminal work, with server-owned session persistence.**

The product target is not “an editor with a terminal panel.” The terminal is the center. Everything else exists to help you find, restore, explain, automate, and organize terminal work without losing momentum.

This project comes from a very specific pain. Modern terminal work is not one shell doing one thing. It is many long-lived sessions, many machines, many half-finished threads of thought, AI transcripts, SSH reconnects, and a constant risk that a laptop sleep or crash will erase the mental desktop that told you what was happening. Yggterm is trying to turn that mess into a workspace you can trust.

That means:

- one desktop shell for sessions, terminals, papers, folders, separators, and metadata
- a daemon-owned PTY runtime so terminals survive view switches cleanly
- a virtual tree of work surfaces, not a raw filesystem browser
- first-class support for agent sessions, generic terminals, SSH sessions, papers, and virtual folders
- fast startup from local metadata under `~/.yggterm`

## Why use it

Yggterm is trying to solve a specific problem: terminal work is usually scattered across shell history, tmux panes, GNU Screen sessions, half-remembered commands, AI transcripts, scratch files, and SSH tabs. That fragmentation kills flow.

The worst part is not just losing a process. It is losing the map in your head:

- what is running
- on which machine
- for which project
- why that session exists
- what the last meaningful conclusion was

That is the job.

Yggterm is for people doing real multi-machine work, especially AI-heavy terminal work, who are tired of rebuilding that map by hand every time life interrupts them.

Yggterm keeps those things nearby:

- the live terminal
- the recovered Codex session or other session preview
- papers and future structured canvases
- session metadata and restore state

The left tree is not just a launcher. It is the workspace memory.

The bigger bet is that terminal work deserves the same level of UX ambition browsers got when tabs became too important to manage casually. If Zen Browser and Arc helped people survive the chaos of 120 tabs, Yggterm should do the same for 120 terminal contexts.

If that resonates, read the product thesis in [PRODUCT_THESIS.md](/home/pi/gh/yggterm/PRODUCT_THESIS.md).

## Core nouns

Yggterm is converging on a small set of user-facing concepts:

- `Session`: an agent-oriented context. Today that usually means Codex, with `Codex` and `Codex LiteLLM` treated as two modes of the same session model.
- `Terminal`: a generic daemon-owned shell/process context. This is where plain shells, SSH terminals, and future low-friction automation live.
- `Paper`: a canvas surface for thinking, planning, and organizing work near the terminal. Over time it can grow beyond plain text into richer surfaces like kanban, calendar, and spreadsheet-like modes.
- `Folder`: a virtual organizational node in the tree, often tied to a cwd or project context.
- `Separator`: a visual divider for compartmentalization in the tree.

This vocabulary matters. Yggterm is not trying to become “documents plus recipes plus tabs.” It is trying to become a calm workspace where sessions, terminals, papers, and folders stay near each other and stay understandable.

## Install and update model

The direct install path above is the mainline channel right now.

What it does:

- detects your OS and architecture
- downloads the latest matching GitHub Release artifact
- installs it into a managed user-space root
- creates desktop integration for direct installs
- refreshes integration when assets change
- self-updates on launch when a newer direct-release build is available
- installs updates in the background and prompts you to restart when you choose
- reruns of the install one-liner act as an explicit manual updater for direct installs

Package-managed installs behave differently on purpose:

- `.deb`, Homebrew, Winget, Scoop, Flatpak, and Snap installs are detected
- those installs switch to notify-only update mode
- Yggterm tells the user to update with the matching package manager instead of mutating the install behind its back

This split keeps the fast-moving direct channel frictionless while respecting native platform ownership when the app was installed by a package manager.

On Linux, Yggterm now defaults to `NO_AT_BRIDGE=1` unless you explicitly opt back in with `YGGTERM_ENABLE_ACCESSIBILITY=1`. That workaround exists because some KDE/Wayland + WebKitGTK setups crash in `libatk-bridge` before the app window becomes visible.

## Current product shape

Today the active stack is:

- Dioxus desktop shell for the app surface
- xterm.js embedded in the main viewport for terminal mode
- `yggterm-server` as the daemon-owned PTY/runtime layer
- SQLite-backed local metadata for papers, folders, separators, and generated labels

That means the current app already supports:

- embedded terminal mode in the main canvas
- rendered preview mode for stored sessions and documents
- daemon-owned live sessions that survive switching between items
- local shell sessions, SSH-backed sessions, and Codex-style agent sessions
- an in-terminal Codex/Codex LiteLLM mode switch with server-side guardrails
- lightweight papers stored in `~/.yggterm/workspace.db`
- executable terminal recipes as an intermediate step toward richer terminal automation
- generated session titles through a configured LiteLLM endpoint
- direct install with self-update and package-manager-aware notify-only mode
- SSH-side Yggterm commands for remote session scanning, generated-copy persistence, and clipboard image staging

## Remote command surface

For SSH machines, Yggterm now prefers a Yggterm-owned remote command surface over ad hoc remote helper scripts.

That means the desktop app can ask a remote host to:

- report its Yggterm remote protocol version
- scan Codex sessions
- persist generated `title` / `precis` / `summary`
- stage clipboard images into that machine's `~/.yggterm/clipboard/`

When the SSH target does not already have a compatible `yggterm` binary on `PATH`, Yggterm bootstraps a matching binary into:

```text
~/.yggterm/bin/yggterm
```

and then reuses that binary for later remote commands on the same host. This is the current foundation for a fuller VS Code-style remote server model.

## The workspace model

`YGGTERM_HOME` defaults to `~/.yggterm`.

The long-term model is metadata-first. The tree should describe work, not just folders on disk. A path in Yggterm is allowed to mean:

- a Session
- a Terminal
- a Paper
- a Folder
- a Separator
- an SSH target or machine context
- a future automation surface

Papers are first-class workspace items, not a bolted-on notes tab. They live in `~/.yggterm/workspace.db` and appear in the same tree as sessions and terminals.

This is the direction:

- terminals stay alive in the daemon while you switch views
- preview and terminal are two lenses on the same workspace item
- papers live beside the sessions and terminals they explain
- generic terminals should gradually absorb low-friction automation behavior instead of making users “manage scripts”
- local shells, agent sessions, and SSH terminals share one runtime model
- fast local metadata keeps startup cheap even when the tree gets large

## Tree workflow

The sidebar is now an active workspace surface.

Examples of what you can do:

- use `+Session`, `+Terminal`, and `+Paper` as the primary quick-create actions
- right-click a folder and create a new session there
- right-click a folder and create a new terminal there
- right-click a folder and create a nearby paper
- right-click a folder and add another folder for compartmentalization
- right-click a folder and add a separator for visual grouping
- right-click a stored session and create a paper beside it
- right-click a stored session and create an executable terminal recipe derived from it
- regenerate generated titles for a session when needed

The intent is simple: organizing the tree should naturally create the right place to work next.

Virtual folders and separators are stored under `~/.yggterm/workspace.db`, so they load quickly and do not depend on walking a large on-disk workspace before the UI becomes useful.

The SSH connect rail is guided on purpose:

- type `user@ip`, `user@host`, or a shortcut from your `~/.ssh/config` such as `dev`
- optionally add a remote prefix if you want the session to start inside `machinectl`, `tmux`, or another remote wrapper
- reconnecting to the same SSH target focuses the existing live session instead of spawning a duplicate

Agent mode switching is handled inside the terminal header, not in global settings:

- a live Codex session can be switched into Codex LiteLLM mode in place
- Yggterm asks the daemon to stop and relaunch that same session cleanly
- if the terminal still looks active, the switch is refused and the user gets a notification instead of a corrupted session

## Papers and automation

Yggterm already has a CLI path for lightweight paper content, so notes can be created or updated outside the UI while the richer paper surface keeps evolving.

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

- papers open in preview mode for editing
- the long-term direction for paper is a richer canvas surface, not just a note blob
- paper-oriented tools should eventually live in a ribbon-like strip beneath the titlebar, closer to Office-style task organization than a markdown toolbar
- the current executable “recipe” layer is an intermediate step toward a more natural terminal automation model
- `Run Here` saves the current automation content and reuses the current document-backed terminal view
- `Run In New Session` saves it and starts a fresh daemon-owned shell session from the saved cwd and commands

That is the beginning of a bigger idea: a workspace can hold the terminal, the explanation, the planning surface, and the repeatable command flow together.

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
- `crates/yggui`: Dioxus shell and app interaction surface
- `crates/yggterm-platform`: platform detection helpers
- `crates/yggterm-ghostty-bridge`: optional legacy bridge code, not part of the default path
- `scripts/`: installers, packaging, and release helpers
- `debian/`: Debian package metadata

## License

- source code: `Apache-2.0`
- repository documentation (`*.md`): `CC BY-SA 4.0`

See `LICENSE`, `LICENSE-APACHE`, `LICENSE-CC-BY-SA-4.0`, and `NOTICE`.
