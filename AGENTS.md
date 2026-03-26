# AGENTS.md

## Mission

Build **Yggdrasil Terminal**: a Rust-first, cross-platform, remote-first terminal workspace with a Dioxus desktop shell shaped like Zed, a daemon-owned PTY core, and an embedded xterm.js terminal surface.

## Local repository relationships

- `../ghostty` contains legacy Ghostty integration code in Zig.
- `../zed` is an optional visual/reference checkout for shell design study.
- This repo (`yggterm`) is the integration layer and product surface.

## Engineering constraints

- Primary implementation language: **Rust**.
- Use the repository-pinned Rust `1.94.0` toolchain for the local desktop stack.
- Ghostty interoperability may still require **Zig** for legacy bridge work, but it is not required for the default terminal path.
- Prefer explicit modular boundaries:
  - `core` (domain state and tree model)
  - `ui` (rendering and interaction)
  - `ghostty-bridge` (legacy FFI boundary)
  - `platform` (OS-specific bindings)

## Product direction

- Dioxus desktop is the active application shell. Match the basic shape and chrome of Zed first, then replace editor-specific behaviors with terminal-specific ones.
- The primary navigation model is a vertical sidebar of virtual folders and sessions, not a filesystem browser.
- Tree nodes represent persisted metadata for sessions, generic terminals, papers, folders, separators, SSH targets, and other terminal workflows.
- Example sidebar entries should feel like `remote/prod/codex-session-tui`, `machines/pi/ghostty-admin`, or other metadata-derived paths, not just on-disk folders.
- Fast startup and interactive responsiveness.
- Cross-platform support (Linux/macOS/Windows where feasible).
- Keep terminal semantics owned by the Yggterm daemon plus xterm.js unless the Ghostty tradeoff is revisited explicitly.
- For SSH targets, prefer Yggterm-owned remote commands and metadata/clipboard flows over ad hoc shell-text or Python-side workarounds whenever the remote machine has a Yggterm binary available.
- Remote SSH flows should version-check and, when needed, bootstrap a matching `yggterm` binary into `~/.yggterm/bin/yggterm` on the target machine before depending on remote Yggterm commands.
- Yggterm should feel remote-first: multiple machines, SSH-heavy workflows, and restoring many live terminal contexts is a core use case.

## Design philosophy

- `DESIGN.md` at the repository root is the source of truth for UI language, interaction taste, visual polish, naming, and reusable styling preferences. Consult it before making UI wording or styling changes.
- When durable design preferences emerge during collaboration, update `DESIGN.md` instead of leaving them implicit in chat history.
- Upstream-first integration: prefer proven layout patterns from `../zed` and a thin adapter boundary around terminal/runtime dependencies instead of reimplementing behavior blindly.
- Minimize forks: keep Yggterm-specific code as adapter layers around upstream crates/APIs so upstream pulls stay low-friction.
- Keep Yggterm-owned shell chrome and session UI in local crates so the desktop frontend stays maintainable and Apache-licensed.
- Reuse upstream Zed layout ideas and `codex-session-tui` browser behavior, but do not couple the active shell to GPUI crates again unless that tradeoff is revisited explicitly.
- Replace editor-centric open flows with terminal-centric behavior: selecting a tree node should open, restore, or focus Yggterm PTY sessions rather than text buffers.
- The central viewport should host embedded xterm.js terminal views in place of file editors.
- Keep the active desktop shell centered on real server-owned terminals rather than reviving temporary mock terminal bodies.
- Treat `Session`, `Terminal`, `Paper`, `Folder`, and `Separator` as the active user-facing vocabulary unless explicitly revisited.
- Session state is local-first under `~/.yggterm`, but the tree model is metadata-first rather than a direct filesystem mirror.
- Use `~/gh/codex-session-tui` and, when helpful, the local `../zed` checkout as reference material when refining shell shape, chrome, and interaction patterns.
- Use the running X11 session and screenshots of a live Zed window when validating visual changes to the scaffold.
- Treat the yggterm daemon PTY API as the active terminal engine contract for the app.
- Keep any Rust-to-Ghostty interop thin and explicit via `ghostty-bridge`, but do not route the default terminal path through it.
- Rust is the primary language for product code.
- Zig is required for Ghostty integration work; prefer stable Zig releases (including official stable tarballs) for reproducible builds.
- Quality-of-life features such as full session restore, clipboard and screenshot paste into remote sessions, and durable session metadata are first-class product goals.

## Agent workflow expectations

- Treat this as an integration-heavy systems project.
- When adding code, include clear ownership boundaries between Rust app logic, PTY runtime, and any optional Ghostty FFI.
- Prefer incremental, testable changes.
- Document integration assumptions in `README.md` or module-level docs.
- Performance and UI snappiness are first-class requirements, not cleanup work.
- For any non-trivial shell/runtime change, capture local performance telemetry under `~/.yggterm/perf-telemetry.jsonl`, inspect it, and plot it before closing the task.
- Treat blocking startup work, render-path filesystem IO, synchronous IPC on the UI thread, and repeated full-tree rebuilds as bugs.
- When a slowdown or UI hitch is reported, instrument first, optimize second, and leave the telemetry hooks in place for future regressions.
- Treat `DESIGN.md` as reusable brand/design memory for this and future projects. Styling and naming rules should be captured there, not reinvented repeatedly.
- The active shell is Dioxus-based. Keep steering it toward a polished Zed-shaped terminal workspace rather than rebuilding parallel frontend experiments.
- Development and release workflow is server-first: builds happen in this server environment and release artifacts are pulled from `dist/` to a laptop for runtime testing.
- The primary install channel is a direct GitHub-release installer with self-update on launch for direct installs; package-managed installs must stay notify-only.
- Always produce checksums for release artifacts and keep packaging repeatable via project scripts.
- Keep `debian/` metadata and packaging scripts current so each release can emit a usable `.deb` with accurate runtime dependencies.
- For every release build, always generate the `.deb` package (and checksum) in `dist/` so laptop-side GUI/runtime testing can be done outside the SSH server environment.
- For incremental development releases, always bump the patch version (e.g. `0.1.0` -> `0.1.1`) before packaging.
- For GUI fixes, do not mark the issue as solved until it has been self-tested live on a different X11 display. Use `x11automation` when helpful for reliable interaction/click targeting. If only build/test validation was done, state that explicitly instead of claiming the GUI issue is fixed.

## Licensing

- Repository license: Apache License 2.0.
- Markdown documentation license: CC BY-SA 4.0.
- Copyright owner: Avikalpa Kundu.

## README And Release Notes Contract

- `README.md` should open with the direct-install one-liners and self-update behavior, then quickly explain the product shape: a remote-first, Zed-shaped terminal workspace.
- Keep screenshots and examples focused on install, session restore, remote workflows, and daemon-owned PTY behavior rather than vague UI aspirations.
- Changelog or release notes should explain install-path changes, runtime packaging, UI milestones, PTY/daemon behavior, and remote workflow gains that users will actually feel.
- Release pages should reuse curated notes rather than autogenerated summaries.
- Keep `README.md`, install scripts, `.deb` packaging, and release artifacts aligned.

## Release Notes Automation

- Keep the canonical changelog current with `## Unreleased` at the top while work is in flight.
- When cutting a release, move the user-visible notes into an exact version section before or during the tag workstream.
- Release automation should prefer the exact version section and fall back to `Unreleased` so curated notes still publish when the rename is late.
- Release pages should use curated changelog text rather than autogenerated notes.
