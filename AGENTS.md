# AGENTS.md

## Mission

Build **Yggdrasil Terminal**: a Rust-first, cross-platform, remote-first terminal workspace with a Dioxus desktop shell shaped like Zed and a Ghostty-backed terminal core.

## Local repository relationships

- `../ghostty` contains Ghostty terminal code in Zig.
- `../zed` is an optional visual/reference checkout for shell design study.
- This repo (`yggterm`) is the integration layer and product surface.

## Engineering constraints

- Primary implementation language: **Rust**.
- Use the repository-pinned Rust `1.94.0` toolchain for the local desktop stack.
- Ghostty interoperability may require **Zig** and `libghostty` integration work.
- Prefer explicit modular boundaries:
  - `core` (domain state and tree model)
  - `ui` (rendering and interaction)
  - `ghostty-bridge` (FFI boundary)
  - `platform` (OS-specific bindings)

## Product direction

- Dioxus desktop is the active application shell. Match the basic shape and chrome of Zed first, then replace editor-specific behaviors with terminal-specific ones.
- The primary navigation model is a vertical sidebar of virtual folders and sessions, not a filesystem browser.
- Tree nodes represent persisted session metadata for local shells, SSH targets, Codex sessions, and other terminal workflows.
- Example sidebar entries should feel like `remote/prod/codex-session-tui`, `machines/pi/ghostty-admin`, or other metadata-derived paths, not just on-disk folders.
- Fast startup and interactive responsiveness.
- Cross-platform support (Linux/macOS/Windows where feasible).
- Keep terminal semantics delegated to Ghostty where possible.
- Yggterm should feel remote-first: multiple machines, SSH-heavy workflows, and restoring many live terminal contexts is a core use case.

## Design philosophy

- Upstream-first integration: prefer using existing interfaces from `../ghostty` and proven layout patterns from `../zed` instead of reimplementing behavior blindly.
- Minimize forks: keep Yggterm-specific code as adapter layers around upstream crates/APIs so upstream pulls stay low-friction.
- Keep Yggterm-owned shell chrome and session UI in local crates so the desktop frontend stays maintainable and Apache-licensed.
- Reuse upstream Zed layout ideas and `codex-session-tui` browser behavior, but do not couple the active shell to GPUI crates again unless that tradeoff is revisited explicitly.
- Replace editor-centric open flows with terminal-centric behavior: selecting a tree node should open, restore, or focus Ghostty sessions rather than text buffers.
- The central viewport should host Ghostty-backed terminal views in place of file editors.
- Until Ghostty surfaces are embedded, keep mock terminal bodies and sidebar data inside the Yggterm-owned Dioxus shell rather than building throwaway alternate frontends.
- Session state is local-first under `~/.yggterm`, but the tree model is metadata-first rather than a direct filesystem mirror.
- Use `~/gh/codex-session-tui` and, when helpful, the local `../zed` checkout as reference material when refining shell shape, chrome, and interaction patterns.
- Use the running X11 session and screenshots of a live Zed window when validating visual changes to the scaffold.
- Treat `libghostty` C APIs as the terminal engine contract (app/surface lifecycle, input, render/tick hooks).
- Keep Rust-to-Ghostty interop thin and explicit via `ghostty-bridge`.
- On Linux today, `libghostty` links but the upstream embedded surface host remains macOS/iOS-only, so external Ghostty process fallback is still the expected terminal path until upstream support changes.
- Rust is the primary language for product code.
- Zig is required for Ghostty integration work; prefer stable Zig releases (including official stable tarballs) for reproducible builds.
- Quality-of-life features such as full session restore, clipboard and screenshot paste into remote sessions, and durable session metadata are first-class product goals.

## Agent workflow expectations

- Treat this as an integration-heavy systems project.
- When adding code, include clear ownership boundaries between Rust app logic and Ghostty FFI.
- Prefer incremental, testable changes.
- Document integration assumptions in `README.md` or module-level docs.
- The active shell is Dioxus-based. Keep steering it toward a polished Zed-shaped terminal workspace rather than rebuilding parallel frontend experiments.
- Development and release workflow is server-first: builds happen in this server environment and release artifacts are pulled from `dist/` to a laptop for runtime testing.
- Always produce checksums for release artifacts and keep packaging repeatable via project scripts.
- Keep `debian/` metadata and packaging scripts current so each release can emit a usable `.deb` with accurate runtime dependencies.
- For every release build, always generate the `.deb` package (and checksum) in `dist/` so laptop-side GUI/runtime testing can be done outside the SSH server environment.
- For incremental development releases, always bump the patch version (e.g. `0.1.0` -> `0.1.1`) before packaging.

## Licensing

- Repository license: Apache License 2.0.
- Markdown documentation license: CC BY-SA 4.0.
- Copyright owner: Avikalpa Kundu.
