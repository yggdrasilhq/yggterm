# AGENTS.md

## Mission

Build **Yggdrasil Terminal**: a Rust-first, cross-platform, remote-first terminal workspace with a Dioxus desktop shell shaped like Zed, a daemon-owned PTY core, and an embedded xterm.js terminal surface.

### The product yggterm replaces

The user's pre-yggterm workflow: VSCode terminal panes → tmux inside them (for persistence) → ssh to N different machines → `codex resume` / `claude -r` on each. When VSCode dies, manually reattach to tmux, re-find sessions, re-orient across machines. **yggterm exists to make this workflow disappear.**

### Core value proposition (do not violate)

When the user clicks an agent session in the cwd tree, yggterm performs the equivalent of:

```
ssh <machine> "cd <cwd> && codex resume <UUID>"      # or: claude -r <UUID>
```

…and hands off the terminal. The user just types. **This handoff IS the product.** Anything that breaks rendering parity between this handoff and the equivalent manual command typed into a shell is a regression of the core promise.

### First-class vs second-class session kinds

| Class | Kinds | Persistence | Tree placement |
|---|---|---|---|
| First-class | Codex, Claude Code, future agent CLIs (per [[spec-cwd-tree-agent-cli-unified]]) | The agent CLI itself persists via its JSONL. yggterm's job is to faithfully invoke `<cli> resume <UUID>` so the conversation continues. | Organized by cwd in the tree |
| Second-class | Plain shell terminals (`Shell`, `SshShell`) | Survive GUI death IFF marked keep-alive. Otherwise die with the GUI. Persistence is provided by the yggterm-server (the tmux-like layer). | Listed in Live Sessions when keep-alive; transient otherwise |

### What yggterm does NOT do (don't propose these)

- **Does not parse codex/CC JSONL into the terminal viewport.** The terminal view delegates rendering to the CLI itself. Reading JSONL is the web view's job (separate surface, currently buggy, will improve).
- **Does not reinvent the agent CLI's rendering.** Codex's TUI is codex's choice. CC's render is CC's choice.
- **Does not add CLI flags beyond the minimum needed for handoff** (cwd, UUID, terminal-appearance env). If a fix adds a flag the manual case doesn't use, question it — the manual case PROVED the CLI works without it.

### The wrapper-vs-manual parity rule

> If `app open <agent-session>` renders differently from `ssh -t <machine> codex resume <UUID>` typed into a shell, that is a yggterm bug, NOT a CLI bug. The fix is in yggterm's wrapper / handoff / preservation path.

Diagnose by running the manual command in a clean shell FIRST and comparing rows/cursor/scrollback. If manual works, the wrapper is at fault — don't add flags that change the CLI's behavior; instead find what the wrapper does differently in env, stty, PTY setup, ownership, or preservation.

Full mission rationale: see `~/.claude/projects/-home-pi-gh-yggterm/memory/project-purpose.md` (link `[[project-purpose]]`).

## Local repository relationships

- `../ghostty` contains legacy Ghostty integration code in Zig.
- `../zed` is an optional visual/reference checkout for shell design study.
- This repo (`yggterm`) is the integration layer and product surface.
- `~/gh/paper` and `~/gh/cellulose` are the intended local checkouts for
  standalone Paper and Cellulose apps. They should live under
  `github.com/avikalpa`, remain Apache-2.0 licensed, and expose clean
  integration boundaries that Yggterm can embed without absorbing the whole app.

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
- Focusing a sidebar folder must open that folder's scoped Startpage and clear the active terminal host without closing live runtimes; expansion/collapse must use explicit disclosure controls or keyboard arrows.
- Fast startup and interactive responsiveness.
- Cross-platform support (Linux/macOS/Windows where feasible).
- Keep terminal semantics owned by the Yggterm daemon plus xterm.js unless the Ghostty tradeoff is revisited explicitly.
- Keep the minimal session promise front and center: a Yggterm session should feel like a durable, snappy automation of `ssh <machine>; cd <cwd>; codex resume <uuid>` or the equivalent shell task. Metadata, sidebar placement, hot update, screenshots, and observability support that terminal routine; they must not become alternate renderers, alternate input targets, or alternate sources of session truth.
- For terminal rendering bugs, fix the daemon PTY/xterm.js path first. Do not hide prompt-background, cursor, resize redraw, animation, typed-echo, or scrollback defects behind shell-owned decorative layers just to satisfy screenshots. xterm.js-native renderer APIs such as decorations are part of the terminal surface; Yggterm-owned cursor/prompt/visibility aids must be explicit, narrow, observable, and secondary to PTY/xterm truth.
- Keep terminal implementation notes and local xterm.js reproduction fixtures in `docs/xterm.md`; consult it before terminal rendering, resize, or PTY identity changes.
- For SSH targets, prefer Yggterm-owned remote commands and metadata/clipboard flows over ad hoc shell-text or Python-side workarounds whenever the remote machine has a Yggterm binary available.
- Remote SSH flows should version-check and, when needed, bootstrap a matching `yggterm` binary into `~/.yggterm/bin/yggterm` on the target machine before depending on remote Yggterm commands.
- Yggterm should feel remote-first: multiple machines, SSH-heavy workflows, and restoring many live terminal contexts is a core use case.
- Treat richer surfaces such as `paper`, `cellulose`, and Excalidraw integration as app-grade surfaces that can live in separate repos while still embedding into Yggterm.

## Terminal multiplexer positioning (vs tmux / screen / abduco)

Yggterm's daemon-owned PTY model IS a terminal multiplexer. Product positioning: **Yggterm must match tmux's baseline capabilities AND exceed them in modern affordances.** This is first-class customer value, not a nice-to-have. A user choosing yggterm should never have to fall back to tmux for "I need my history when I reconnect" or similar baseline expectations.

### Decentralized host-resident daemon architecture (the core model — do not violate)

This is the architecture that makes yggterm a real tmux replacement, and it is the lens for every persistence/remote decision.

**Nomenclature (use these terms precisely):**
- **yggterm** — the GUI client (the desktop app the user sees).
- **yggterm-headless** — the headless *client*, made for agents (the app-control surface agents drive). It is a CLIENT, not the session-holder.
- **yggterm server** — the daemon(s) that *hold* sessions alive (codex, Claude Code, plain shell, other TUIs). This is the tmux-equivalent. (The `yggterm-headless` binary can launch a server via `server daemon`, but the session-holding *role* is "yggterm server".)

The model:
- **The yggterm server (daemon) runs ON EVERY host** — the local machine AND each SSH host. The server that owns a session's PTY runs *on the machine where that session lives*, and holds it alive there, exactly as a tmux server would. A remote session is held alive by the remote host's yggterm server, not by anything on the GUI machine.
- **SSH is the transport AND the auth layer.** A client reaches a remote host's yggterm server over SSH; having SSH access to the host *is* the authorization to see and attach its sessions. There is no separate yggterm auth.
- **Metadata is decentralized — stored on each machine.** Each host's `~/.yggterm` holds its own sessions, retained scrollback, and session metadata. There is no central server; the cwd tree is a union the client composes by talking to each host's yggterm server.
- **Many clients, one server.** Any client (GUI or headless) with SSH access to a host can attach the same live servers/sessions concurrently. Sessions are owned by the host's yggterm server, not by a client.
- **Therefore tmux must NOT be a dependency.** The host-resident yggterm server IS the multiplexer that keeps shell terminals alive across client disconnect and SSH death — that is precisely tmux's job, done by yggterm with full xterm.js scrollback and yggterm's own UX. Any current use of tmux/screen to keep a shell alive (see `docs/terminal-backends.md`) is a **stopgap** for the remote-shell-PTY path not yet being fully wired through the host yggterm server, not a design choice. The target end state: shell terminals are owned by the host's yggterm server directly — no tmux process, no tmux status bar, no tmux-owned scrollback. The objection "the local server can't keep a remote shell alive across an SSH drop" is NOT a justification for tmux — it is the reason the server must run on the *remote* host.

**Baseline parity (REQUIRED for credibility against tmux):**
- Session survives client disconnect — GUI close/restart never kills a running session. ✓ shipped
- Reattach from any client — any GUI sees the live daemon. ✓ shipped
- Window/pane resize handshake — daemon reflows cell grid on client resize. ✓ shipped
- **Real terminal history across restart** — daemon maintains a per-session headless VT parser with a scrollback ring (default 10k+ rows), so GUI restart restores full terminal history regardless of whether the inner app is a TUI (Codex, vim, htop) or plain shell. This matches tmux's `history-limit` semantics. ❌ NOT YET SHIPPED — currently the daemon retains only a small raw-byte ring (~146 KB) which collapses to ~one screen for TUI sessions. Implementing this is a parity gate, not an enhancement.
- Per-session `history-limit` config — tunable scrollback retention exposed in settings.

**Where yggterm must EXCEED tmux (these are why a customer picks yggterm):**
- Native xterm.js render fidelity: 24-bit color, glyph anti-aliasing, hyperlinks, image protocols (Sixel, Kitty), web-grade font rendering. tmux is constrained to whatever terminal emulator the user happens to be in.
- Cross-machine session graph: one GUI sees local + remote daemons as a unified session tree. tmux requires manual ssh nesting and per-machine state.
- Cursor + scroll position preserved across GUI restart — not just history, also the user's reading position. ✓ shipped (commit 5a6e19f).
- Agent-CLI awareness: Codex / Claude Code transcript stitching, generated summaries, session titles, kind-driven icons. tmux is application-agnostic by design and loses this.
- First-class observability: app-control state, screenshots, probes for every visible region. tmux exposes only `capture-pane`.
- Persistent session metadata: cwd, machine, working folder, agent kind, generated title — carried across restart and visible in the cwd tree. tmux drops this.
- Unified cwd tree across agent CLIs ([[spec-cwd-tree-agent-cli-unified]]) — sessions from any CLI (Codex, Claude Code, future ones) live in one tree organized by cwd, with kind-driven icon/label/style. tmux has no concept of this.

When choosing whether to ship a feature, ask: "does this advance baseline parity or the exceed-tmux line?" If neither, it's noise.

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
- `docs/architecture-audit-2026-05-16.md` is required reading before terminal,
  session, hot-update, theme, telemetry, app-control, or release-gate changes.
  Its authority table is the standing source-of-truth map.
- Before fixing any regression, name the authoritative source of truth and the
  observers involved. Daemon PTY/runtime truth, xterm render truth, session
  identity, metadata, app-control, telemetry, screenshots, and smoke tests are
  not interchangeable.
- **Spec interpretation rule: every spec MUST enumerate what it does NOT cover.**
  When you cite a spec to justify code, (1) quote the exact spec language,
  (2) state the literal claim, (3) state at least one *adjacent* claim that
  the spec does NOT make. A spec that reads "X stays in A and B. Also, C is
  ordered by recency" is TWO separate claims (X∈{A,B}; C has a sort rule);
  it is NOT "X stays in {A, B, C}". When in doubt, treat the spec as
  *narrower* than your reading; expansions require user confirmation, not
  agent inference. Memories that paraphrase a spec MUST link to the original
  spec text (AGENTS.md or user message) and MUST include an "Out of scope"
  section listing the related claims they deliberately do not make. Any code
  comment of the form `// Per [[spec-X]]: do Y` must survive the test
  "would the user agree Y is in [[spec-X]]'s scope, not Y-adjacent?"
- Never promote an observer into product truth. App-control, telemetry,
  screenshots, logs, generated summaries, and smoke results can prove or
  disprove behavior, but they must not drive terminal rendering, input routing,
  saved-session identity, daemon ownership, or theme state.
- Do not patch a symptom by adding a second source of truth. Banned shortcuts
  include shell-owned terminal text overlays, prompt/cursor repair layers,
  PTY-byte coalescing or trimming, post-hoc transport cleanup as a normal path,
  runtime-key identity substitution, alpha/blur/grain behavior in stable theme
  code, and stale-daemon mutation outside the hot-update protocol.
- **Session display = dual presence.** An active session appears in BOTH the
  "Live Sessions" group AND its cwd folder group. "Single source of truth"
  applies to the session OBJECT (one `ManagedSessionView` per logical session),
  not to the display location. Never silently filter a live session out of
  the cwd tree just because it also appears in Live Sessions; if you find
  dedup code that does this, that is a SPEC VIOLATION. Acceptable dedup is
  per-view (one row per logical session within the same tree), not cross-view.
  **Out of scope: the start page.** The start page is a *launching pad* for
  sessions to open/resume; live sessions are already running and are
  accessible from the Live Sessions sidebar group — they are NOT a third
  presence target. The `start_page_recent_rows` candidate loop must not push
  `snapshot.live_sessions`. The `live_projection_paths` filter that strips
  browser_row duplicates of live sessions from start page recents is the
  correct behavior, not a bug. The start-page-ordered-by-recency rule is
  about sort order on the durable recents list, NOT about content membership.
- **SessionKind drives display, not path prefix.** Icon, glyph, label color,
  button styling, and other display dispatch MUST consult `SessionKind`
  (carried on `BrowserRow.session_kind` when the row was built from a
  `ManagedSessionView`). Branches like `if path.starts_with("local://")
  { "terminal" } else { "session" }` are SSOT violations — `local://` covers
  both shells and Codex sessions, so the path alone cannot answer the
  question. Path prefix is acceptable ONLY as a fallback for rows
  synthesized from file paths where kind is genuinely unknown.
- **Local and remote session display paths share code.** Do not introduce a
  separate display path for local sessions vs remote sessions. Cosmetic
  divergence (icon, label, button style) between a local Codex session and a
  remote Codex session is itself a bug. When fixing session display, fix the
  kind-driven code, not a locality-driven branch.
- **Local cwd tree is agent-CLI-agnostic.** Per
  [[spec-cwd-tree-agent-cli-unified]] every saved agent-CLI session
  (Codex, Claude Code, future) flows through ONE pipeline:
  `yggterm_core::scan_local_<cli>_sessions()` returns
  `LocalAgentSessionSummary` records → `build_local_cwd_tree` groups them
  by cwd → `SessionNode { session_kind, detail, ... }` → flattener carries
  `session_kind` into `BrowserRow.session_kind`. NO post-hoc injection
  passes. Adding a new agent CLI is ONE scanner + one call site in
  `build_local_cwd_tree` + display-dispatch updates for the new
  `SessionKind` variant — never a parallel `inject_<cli>_rows()` path.
  The prior `inject_file_backed_cc_session_rows` bypassed expand/collapse
  state, causing the orphaned-row bug reported 2026-05-26.
- If code and docs disagree, stop and reconcile the interface doc before
  implementing. The canonical docs are `docs/protocol.md` for runtime/hot-update
  behavior, `docs/xterm.md` for terminal rendering and PTY bytes,
  `docs/xterm-bugs.md` for the structured xterm.js bug registry (every
  workaround site MUST have an `// XTERM-BUG: <id>` anchor and a matching
  registry entry — see that file's template),
  `docs/sessions.md` for saved-session identity and copy, `docs/theme.md` for
  stable shell chrome, and `docs/telemetry.md` for observer-only telemetry.
- For every reported regression, update the harness, smoke test, unit test, or CI gate to fail on the exact defect class before applying the runtime fix. Do not accept a fix based only on manual observation when a deterministic regression can be captured.
- For terminal resize regressions, the harness must assert a visible prompt-prefix pixel sample after settle on resize so partial redraw and prompt-clipping artifacts are caught even when x/y geometry metrics still pass.
- Document integration assumptions in `README.md` or module-level docs.
- Treat stale binary execution as destructive. Do not execute archived/versioned GUI binaries, old `dist/` artifacts, backup copies, or direct-install store entries just to inspect their version. In particular, never run globbed commands like `~/.local/share/yggterm/direct/versions/*/yggterm --version`; old GUI binaries may ignore CLI-only intent, launch against the live `~/.yggterm` state, and overwrite session metadata.
- Prove versions from canonical metadata before installing, publishing, or replacing a running app: `Cargo.toml`/lockfile, changelog section, git commit/tag, release asset checksum, `install-state.json`, and the active launcher/headless path. If an executable must be probed, use the active launcher on 2.1.52+ or the exact active `yggterm-headless` sibling from `install-state.json`; otherwise isolate it with a temporary `HOME`/`YGGTERM_HOME` and no access to user state.
- Never "fix" a release or runtime issue by installing, launching, or copying an older artifact unless the user explicitly requests rollback. If rollback is requested, snapshot user state first, state the exact target version/date, and keep the old artifact isolated from normal self-update paths.
- Before touching a live install or remote GUI session, snapshot the relevant state files (`~/.yggterm/server-state*.json`, `session-titles.db`, `event-trace.jsonl`, install metadata) and confirm the currently running GUI/daemon executable paths. Treat mismatched GUI, daemon, launcher, and install-state versions as an incident until reconciled.
- Treat `yggterm-headless server monitor` as the first-line panic-management tool for live terminal incidents. When a session is hung, missing after restore, slow to load, input-lagged, or visually blank on a live desktop host, run a read-only incident pass before changing code: `mkdir -p ~/.tmp/yggterm && yggterm-headless server monitor --scenario panic-report --expect-path <session-path> --jsonl-out ~/.tmp/yggterm/yggterm-incident.jsonl`, then `server-list`, `latency-check --all`, `wait-session`, or `hot-restart --all` as the evidence indicates.
- For repeated or intermittent failures, monitor with `yggterm-headless server monitor --scenario panic-report --iterations <n> --interval-ms <ms> --jsonl-out ~/.tmp/yggterm/<name>.jsonl` so latency/session truth is captured independently of the GUI render loop.
- Use `yggterm-headless server monitor` evidence to split incidents cleanly: daemon/version/reachability issues belong to server lifecycle and hot-restart paths; missing sessions belong to restore/session graph logic; slow status/snapshot belongs to daemon blocking work; healthy daemon state with bad pixels or input belongs to app-control screenshot/probe investigation.
- For daemon-owned Codex live sessions, treat the Codex transcript JSONL discovered from the PTY process tree as the saved-session identity. A `codex-runtime://...` terminal key may be synthetic and must remain the terminal I/O key, but sidebar search, remote scans, resume deduplication, and user-facing saved-session identity should use the real Codex session id from the open transcript when available.
- For KDE duplicate-icons, pinned launcher regressions, or update-handoff identity bugs, run `yggterm-headless server app desktop-identity` before changing code. The report should capture canonical desktop file fields, KDE pinned launchers, live client app ids, and the handoff environment flags.
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
- Treat a user's active work desktop as a real machine, not a disposable lab. Prefer rigorous local/debug-build proof in this server environment and isolated `YGGTERM_HOME`/second-display harnesses before driving a live user profile. When a live-desktop bug requires live proof, prefer Yggterm-owned app-control paths (`server app state`, `terminal send`, `probe-type --mode xterm`, `background`, `close`) over desktop-wide keyboard/pointer automation; KDE permission prompts or input leaking into other apps are automation failures to fix, not normal proof. Keep live runs short, target explicit PIDs, background or close automation-owned windows afterward, and remove temporary live sessions created by the test unless the user asks to keep them. For user-visible GUI/runtime regressions on a live desktop, do not hand control back until the target install has been updated, the relevant harness has been updated when needed to expose missing proof, and the harness proof has been run and recorded.
- Treat the current `codex-litellm` upstream update banner as expected noise unless the user explicitly asks to update `~/gh/codex-litellm`. Do not chase that banner as a Yggterm regression or use it as failure evidence in latency, startup, or render investigations.
- For terminal input/rendering regressions, the proof bar is stricter: run a second-X11 keyboard smoke against the real viewport, type `/status` plus `Enter`, capture screenshot + state, and do not mark the issue fixed unless the terminal stays interactive and readable afterward.
- For `/status`-class terminal typing regressions, prefer the live keyboard smoke over synthetic core-trigger proof. The accepted bar is: the real viewport shows `/status` in the prompt area, the Codex status panel renders, the cursor remains visible at the next prompt, and the terminal stays interactive with no retry/disconnect toast.
- For terminal UI/UX fixes, do not rely on state alone. On a second X11 display, require the matching trio: `server app state`, `server app screenshot`, and the relevant viewport probe (`probe-type`, `probe-scroll`, or `probe-select`). For light-theme readability/cursor bugs, reject the fix unless the screenshot itself shows readable text and a visible cursor, and the state/probe agree.
- For terminal/UI visibility fixes, require a probe that matches the defect class before closing the issue. Examples: type/enter for overwrite bugs, scroll probes for viewport bugs, and selection/contrast probes for “text only visible when selected” regressions.
- When touching terminal UX, resume behavior, or app-control truth, update and run the local checklist at `.todos/terminal-ux-smoketests.md` before handover. Keep the checklist untracked, but keep this reference in sync.
- Screenshot review must be explicit, not impressionistic. When validating a GUI screenshot, classify each region separately: viewport content, floating toasts/overlays, title/session chip, sidebar affordances, and cursor/input position. Do not infer "no toast", "no arrows", or similar UI absence from memory, nearby state, or prior runs; confirm it from the exact image being discussed and cross-check with app-control state when possible.

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

## Experimental Branch And Release Protocol

- `main` contains release-ready work only. Stable end-user releases are cut from
  `main`; experimental feature work must not land there until the feature is
  intentionally promoted.
- Experimental feature branches use the `experimental/<feature>` namespace and
  live in sibling worktrees named `~/gh/yggterm--<feature>`, for example
  `~/gh/yggterm--paper-integration`.
- Daily experiment work starts with `git fetch origin --prune` and a rebase of
  the experiment branch onto `origin/main`. Do not merge `main` into an
  experiment branch. Use `scripts/rebase_experimental_worktrees.sh` for the
  local multi-worktree pass; it must skip dirty worktrees and stop on conflicts.
- The active experimental worktrees are `experimental/alpha-blur`,
  `experimental/paper-integration`, `experimental/openwebui-integration`,
  `experimental/excalidraw-obsidian-integration`, and
  `experimental/cellulose-integration`.
- Experimental releases must use `yggterm-` prefixed binaries and package names,
  such as `yggterm-alpha-blur`, `yggterm-paper`, `yggterm-openwebui`,
  `yggterm-excalidraw-obsidian`, and `yggterm-cellulose`. Headless siblings
  should follow the same channel identity, for example
  `yggterm-paper-headless`.
- Experimental release channels must not overwrite the stable `yggterm`
  launcher, desktop identity, direct-install metadata, or stable update channel.
  Use isolated state homes by default, such as
  `~/.yggterm-experimental/<channel>`, unless the task is explicitly testing a
  migration against the stable home after snapshotting it.
- Experimental CI/release work may default to Linux x64 plus `.deb` and
  checksums. Run the full cross-platform release matrix when the experiment
  touches platform-specific shell, installer, compositor, or runtime behavior.
- The working protocol and experiment scopes live in
  `docs/experimental-worktrees.md` and `docs/experiments/`.
- Experimental branches may intentionally carry branch-specific `AGENTS.md`
  changes, helper docs, scripts, or local operator notes for that feature. Treat
  those as experiment-local by default. When promoting or merging an experiment,
  explicitly decide which of those files should be dropped, which should remain
  branch-only, and which should be reconciled into the stable docs on `main`.

## Shared YggUI Platform Direction

- Treat `yggterm` as the first proving ground for a reusable `yggui` platform covering app-control, observability, automation, proof bundles, and demo composition.
- When a feature feels generic across future YggdrasilHQ desktop apps, prefer a structure that can later move into `yggui`, `yggui-platform`, `yggui-observe`, `yggui-automation`, or `yggui-demo` rather than hard-wiring it to `yggterm`.
- Keep app-specific semantics in `yggterm`, but design schemas, manifests, trace formats, and macro concepts so future apps like `yggtopo` or `cellulose` can reuse them.
- Prefer clean embedding boundaries so standalone repos like `paper` or `cellulose` remain independently valuable products rather than permanently trapped as private subfeatures.

## Demo And Changelog Evidence

- Significant user-visible work should be representable as a proof bundle: manifest, screenshots, optional recording, trace/state evidence, and a short narrative summary.
- Use `docs/demos/` for the durable system design, `artifacts/demos/` for bundle layout, and `.agents/skills/` for operator workflow.
- Prefer release notes that cite proof bundles and screenshots over vague summaries.
- When adding new app-control or automation powers, update the demo/changelog docs and skill in the same change rather than letting the workflow drift out of sync.
- When touching observability, app-control, proof capture, or terminal-resume verification, always update the relevant skill file in the same change so the debugging workflow stays current.
- Terminal geometry classification in app-control is part of observability. If `active_terminal_surface.geometry_problem` changes semantics, update the relevant skill files in the same commit.
- GUI singleton and focus behavior are part of observability too. If client-instance registration, app-control window metadata, or display/session matching changes, update the relevant skill files in the same commit.
- Terminal text visibility and selection diagnostics are also part of observability. If `terminal_hosts[].low_contrast_span_*`, selection probes, or smoke coverage change, update the relevant skill files in the same commit.
- Terminal input gating is part of observability too. If `terminal_hosts[].host_stdin_enabled` or the startup recovery input/focus contract changes, update the relevant skill files in the same commit.
- Visual changelog assets should be deterministic, reusable, and cleanly composited, with restrained motion and annotation rather than flashy effects.
- Once a UI/UX bug is fixed to the desired behavior, add or extend a concrete smoke test for it instead of relying on visual judgment alone. For terminal UI work, prefer app-control assertions against the mounted xterm row styles, theme state, input state, and screenshots on a second X11 display before calling it fixed.
- Smoke tests should target the exact defect class that slipped. If the bug was a half-mounted terminal, invisible cursor, low-contrast rows, or broken overwrite path, add an assertion for that concrete failure mode instead of a generic “looks loaded” check.
- For xterm embed fixes, the default regression bar is `scripts/smoke_xterm_embed_faults.py` plus a second-X11 screenshot. Do not call terminal display/input/cursor issues fixed unless that suite passes for the relevant session kind and the screenshot agrees.
- For any user-visible UI behavior change, prefer the real probe/app-control path over ad hoc visual inspection. Add or extend a deterministic end-to-end smoke that matches the defect class, run it on a second X11 display, and only then call the behavior fixed.
- For any reported regression, update the harness, tests, and CI first so the exact defect class fails deterministically before applying the product fix. Do not rely on a manual screenshot or state-only check when the missed behavior can be captured by app-control, a smoke probe, or a focused unit test.
- Sidebar folder-focus regressions must be proved through app-control by selecting/opening a folder row, requiring `start_page_visible=true`, no active session path, no active terminal input target, and selected-row truth still pointing at the folder.
- For retained terminal bugs, never assume `terminal_hosts[0]` is the active surface. Proof and smoke code must select the active host by active session path plus focus/activity, or use an explicit active-host field from app-control.
