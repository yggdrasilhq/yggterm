# AGENTS.md

## Mission

Build **Yggdrasil Terminal**: a Rust-first, cross-platform terminal workspace that uses Ghostty terminal capabilities with a Zed-style UI and nested tree organization for terminal groups/sessions.

## Local repository relationships

- `../ghostty` contains Ghostty terminal code in Zig.
- `../zed` contains Zed editor UI code in Rust.
- This repo (`yggterm`) is the integration layer and product surface.

## Engineering constraints

- Primary implementation language: **Rust**.
- Ghostty interoperability may require **Zig** and `libghostty` integration work.
- Prefer explicit modular boundaries:
  - `core` (domain state and tree model)
  - `ui` (rendering and interaction)
  - `ghostty-bridge` (FFI boundary)
  - `platform` (OS-specific bindings)

## Product direction

- Tree-first terminal organization with vertical, multi-level nested folders.
- Fast startup and interactive responsiveness.
- Cross-platform support (Linux/macOS/Windows where feasible).
- Keep terminal semantics delegated to Ghostty where possible.

## Design philosophy

- Upstream-first integration: prefer using existing interfaces from `../zed` and `../ghostty` instead of reimplementing behavior.
- Minimize forks: keep Yggterm-specific code as adapter layers around upstream crates/APIs so upstream pulls stay low-friction.
- Reuse `workspace::Item` and `workspace::SerializableItem` as the primary viewport item model.
- Reuse `workspace::Panel`, `Pane`, and `PaneGroup` patterns for layout, split behavior, and focus routing.
- Reuse `project_panel` tree patterns, but map tree nodes to terminal sessions/groups rather than files.
- Replace editor-centric open flows with terminal-centric behavior: selecting a tree node should open/focus terminals, not text buffers.
- The central viewport should host Ghostty-backed terminal views in place of file editors.
- Session state is local-first and filesystem-backed.
- `YGGTERM_HOME` is `~/.yggterm`.
- Persist nested session folders and metadata under `~/.yggterm`, analogous to how `~/.codex` stores local app state.
- Treat `libghostty` C APIs as the terminal engine contract (app/surface lifecycle, input, render/tick hooks).
- Keep Rust-to-Ghostty interop thin and explicit via `ghostty-bridge`.
- Rust is the primary language for product code.
- Zig is required for Ghostty integration work; prefer stable Zig releases (including official stable tarballs) for reproducible builds.

## Agent workflow expectations

- Treat this as an integration-heavy systems project.
- When adding code, include clear ownership boundaries between Rust app logic and Ghostty FFI.
- Prefer incremental, testable changes.
- Document integration assumptions in `README.md` or module-level docs.
- Development and release workflow is server-first: builds happen in this server environment and release artifacts are pulled from `dist/` to a laptop for runtime testing.
- Always produce checksums for release artifacts and keep packaging repeatable via project scripts.
- Keep `debian/` metadata and packaging scripts current so each release can emit a usable `.deb` with accurate runtime dependencies.
- For every release build, always generate the `.deb` package (and checksum) in `dist/` so laptop-side GUI/runtime testing can be done outside the SSH server environment.
- For incremental development releases, always bump the patch version (e.g. `0.1.0` -> `0.1.1`) before packaging.

## Licensing

- Repository license: Apache License 2.0.
- Copyright owner: Avikalpa Kundu `<avi@gour.top>`.
