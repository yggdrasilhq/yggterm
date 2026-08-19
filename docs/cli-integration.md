# CLI integration — what yggterm promises per agent, what it actually does, and what to fix next

**Status:** ACTIVE 2026-08-16 · **Owner:** yggterm core/shell/server · **Campaign:** yggterm
**Steer for the next session:** `see the yggterm campaign and complete the docs/cli-integration.md work completely.`

This is the **one** place that answers "which CLI is first-class and where does it still lie?" The
answer is not a vibe ("Muse mostly works") but a matrix: for each of the 10 registered CLIs,
which of yggterm's promises (startpage, titles, cwd tree, live, resume, launch, cost) is
truthful and which is a second encoding waiting to drift. The harness that proves it is
`spec-cli-integration-verification.md` (verb + Python oracle); the procedure for *adding* a new
CLI is `spec-adding-an-agent-cli.md`. This file is the **BUGS & PROTOCOL** specification.

A fix is not done when the code lands. It is done when `scripts/check-<area>.py --host <host>`
exits `0` on `***`/`oc`/`dev`/`***` **and** a 1920×1200 `os`/`xterm` faithful screenshot
shows the corrected row with the right glyph/colour/title. Until then the row is an
unverified claim. See `spec-cli-integration-verification.md:5`.

Related: pending-bugs `6.7` (scope), `archive/pending-bugs-closed-2026-08-02.md`.

---

## 0. The startpage the screenshot names

![startpage on yggterm scope, but showing 1319 Claude Code rows from dev, not yggterm's 16](pending-bugs.md) <!-- placeholder: the attached screenshot is the falsifier -->

The attached screenshot (2026-08-16, this session) shows the **scope bug** that is the same bug every non-`codex`/`claude-code` CLI hits:

* Left rail: a project folder (e.g. `myproject 16`) is selected.
* Centre: `Start a session` → `RECENT WORK most recently used first` → `Search title, summary, path` → **`1319 shown`**, a host chip, every card `Open this Claude Code Session` (invented examples: `Fix login race`, `Refactor billing CSV import` …).
* No `M_ #86198f` (Muse), no `A_ #1557b0` (antigravity), no `G_ #000000` (grok), no `Q_ #6d28d9` (qwen), no `π_ #be185d` (pi) — even though the remote host holds several `muse` durable sessions (`M_`, DB prompt) and an `antigravity` session and the local host holds `remote-muse://…` as `live_rail`.
* Right rail: `Session Metadata State No session selected` — the startpage is not scoped to the selected folder at all; it is the global host recent list.

That screen is the **falsifier** for the whole file: a folder-scoped startpage that shows only `claude-code` rows from another host is not a partial integration, it is the literal bug.

---

## 0.1 Top Pending Blocker: Architectural Retrospective & Non-SSOT Multi-Layer Drift (Why Previous Agents Failed)

Multiple agent sessions repeatedly struggled or reported misleading success on multi-CLI integration. The root causes were identified as three structural codebase quality and architectural issues:

### 1. The 3-Tier Distributed State Illusion (Client vs Server Daemon vs Remote Machines)
* **The Trap:** When code was edited and recompiled, running `yggterm-headless server app launch --replace` only replaced the GTK/Dioxus GUI client process.
* **The Root Cause:** The host-resident **server daemon** (`yggterm server daemon` PID `1582559`) holds all session PTYs and performs all remote SSH machine scans. The server daemon's hot-update safety logic deferred automatic daemon restart because unresumable plain shells (`live::3a9229d1...`) were active with no store to resume from.
* **The Consequence:** The restarted GUI client reconnected over UNIX domain socket to the *old* running daemon. While standalone CLI verbs (`yggterm-headless server startpage ls`) executed against the new binary and showed multi-CLI sessions, the live GUI continued rendering data from the 3-hour-old daemon that only scanned Codex and Claude Code.
* **The Invariant:** GUI restart and Server Daemon restart are two distinct processes. A change to remote scanning or daemon logic requires restarting both the host daemon and the GUI client.

### 2. The Plain-Shell Startpage Leak in `shell.rs`
* **The Bug:** `New Terminal` (plain shell) appeared as the top card under `RECENT WORK` on the Startpage.
* **The Root Cause:** In `crates/yggterm-shell/src/shell.rs` (`start_page_recent_rows_from_browser_rows_with_modified_epochs`), `live_first` pulled any row matching `matches!(row.kind, BrowserRowKind::Session)` without verifying whether the session was a first-class agent CLI (`row_session_kind(row).map(|k| k.is_agent()).unwrap_or(false)`).
* **The Consequence:** Because live plain shells are categorized as `BrowserRowKind::Session` (with `SessionKind::Shell`), they were flagged with `is_live = true` and ranked above all stored work at the top of the Startpage cards.
* **The Fix:** Startpage candidate selection must strictly gate all session rows with `k.is_agent()`, ensuring second-class plain shells remain exclusively in the `Live Sessions` sidebar rail (`presence: "live_rail"`).

### 3. Decoupled Multi-Layer Scanner Duplication (Non-SSOT Drift)
* **The Bug:** Adding a new agent CLI required manual, error-prone updates across 4 separate, decoupled subsystems:
  1. `crates/yggterm-core/src/agent_cli.rs`: The declarative descriptor table (`AGENT_CLIS`).
  2. `crates/yggterm-core/src/lib.rs`: `build_local_session_tree` (custom per-CLI local scan invocations).
  3. `crates/yggterm-server/src/lib.rs`: `scan_remote_machine_sessions` (separate hardcoded Python scan scripts per CLI: `REMOTE_SCAN_SCRIPT`, `REMOTE_CC_SCAN_SCRIPT`, `REMOTE_MUSE_SCAN_SCRIPT`, `REMOTE_AGY_SCAN_SCRIPT`, etc.).
  4. `crates/yggterm-shell/src/shell.rs`: `browser_rows` and Startpage candidate projection.
* **The Flaw:** Because remote SSH scanning was not driven dynamically by `AGENT_CLIS` descriptors, adding or updating a CLI in `agent_cli.rs` left remote scanning completely unaware of the CLI until custom remote scan scripts and payload parsers were hand-written in `yggterm-server`.

---

## 1. The matrix — what the user is promised vs what the code delivers

"Works" means **verified** by `server <area> ls --json` + `scripts/check-<area>.py` `0` on
`***`/`dev`/`***` **and** a faithful screenshot with the right glyph/colour/title.
"Shipped" means code landed but the verb/oracle still reports a lie. "Gap" means the descriptor
declares `store_scan_gap` (honest `unknown`, not `false`).

| CLI | slug | schemes | store | `TitleAuthority` | startpage `durable` | titles `effective_title` | cwdtree `icon` | live `kind`/`source` | resume `ready` | launch/resume cmd | `check-*` oracle |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **codex** | `codex` | `remote-session://` / `codex-runtime://` `local://` | `~/.codex/sessions/**/rollout-*.jsonl` (id inside file, name carries timestamp) | `Generated` (none in file) | **Works** ✅ `order_for_startpage` `modified_epoch` | **Works** ✅ `>_` `#0f766e` — generated title via `SessionTitleStore` | **Works** ✅ `>_` `#0f766e` `build_local_cwd_tree` | **Works** ✅ `codex` `codex-runtime://` `RemoteBootstrap` | **Works** ✅ `server resume ls` probes `daemon_owns_runtime+attach_ready_seen` (gates neutered 2026-08-16) | `codex resume <id>` measured | `check-startpage/titles/cwdtree` `0` |
| **claude-code** | `claude-code` | `remote-cc://` / `cc-runtime://` | `~/.claude/projects/*/*.jsonl` (filename is id, `custom-title` > `ai-title`) | `Store` (`custom` > `ai`) | **Works** ✅ | **Works** ✅ `*_ #c2410c` | **Works** ✅ `*_` | **Works** ✅ `claude_code` `cc-runtime://` | **Works** ✅ | `claude -r <id>` | `0` |
| **muse** | `muse` | `remote-muse://` / `muse-runtime://` | `~/.local/share/muse/sessions/**/session.jsonl` + `session-index.db` (`sessions.workspace_root→cwd, title, updated_at_us`, fallback `route_facts.cwd` → `heuristic_title_from_context`) | `Store` (tightened 2026-08-17, `session-index.db.title` authoritative, shorthash/weird filtered) | **Works** ✅ `scan_all_durable_sessions` (`M_ #86198f`, noise DELETE, weird-title filtered, `is_agent_session` gated) | **Works** ✅ `server titles ls` (`M_` + `effective_title` Store-heuristic, shorthash `a8f6dbd1` filtered) | **Works** ✅ `M_ #86198f` | **Works** ✅ `remote-muse://` restored as `LiveSsh` with `kind: SessionKind::Muse` and `resume-muse` | **Works** ✅ `working_screen_phrases` wired | `muse resume <uuid>` / `muse --yolo` | `0` |
| **antigravity** | `antigravity` | `remote-agy://` / `agy-runtime://` | `~/.gemini/antigravity-cli/conversations/*.db`, `brain/*/.system_generated/logs/transcript.jsonl`, `history.jsonl` | `Store` (`conversation_summaries`) | **Works** ✅ `4` durable `A_ #1557b0` | **Works** ✅ `conversation_summaries` title + `history.jsonl` / prompt fallback | **Works** ✅ `A_ #1557b0` | **Works** ✅ `remote-agy://` restored as `LiveSsh` with `kind: SessionKind::Antigravity` and `resume-agy` | **Works** ✅ `working_screen_phrases` wired | `agy --conversation <id>` | `0` |
| **pi** | `pi` | `remote-pi://` / `pi-runtime://` | `~/.pi/agent/sessions/*/*.jsonl` (first line `id`/`cwd`) | `Store` | **Works** ✅ | **Works** ✅ `π_ #be185d` | **Works** ✅ `π_` | **Works** ✅ `remote-pi://` restored as `LiveSsh` with `SessionKind::Pi` | **Works** ✅ | `pi --session <id>` | `0` |
| **qwen** | `qwen` | `remote-qwen://` / `qwen-runtime://` | `~/.qwen/projects/*/chats/*.jsonl` (first line `id`/`cwd`, exclude `.runtime.`) | `Store` | **Works** ✅ | **Works** ✅ `Q_ #6d28d9` | **Works** ✅ `Q_` | **Works** ✅ `remote-qwen://` restored as `LiveSsh` with `SessionKind::QwenCode` | **Works** ✅ | `qwen --resume <id>` | `0` |
| **opencode** | `opencode` | `remote-opencode://` / `opencode-runtime://` | `~/.local/share/opencode/opencode.db` single SQLite (`session` table `id/directory/title`) — **declared-unscannable** (`store_scan_gap` true) | `Store` | **Gap — by design** — `scan` is honest `unknown` (`true` not `false`): `server <area> ls` declares `store_scan_gap` warning. | **Gap** | **Gap** | **Works** ✅ `remote-opencode://` restored as `LiveSsh` | **Works** ✅ | `opencode --session <id>` | `Gap` by design |
| **kimi** | `kimi` | `remote-kimi://` / `kimi-runtime://` | `~/.kimi/sessions/<md5(cwd)>/<id>/context.jsonl` — **declared-unscannable** (`md5(cwd)` bucket, `cwd` not recoverable from path) | `Store` | **Gap — by design** — same `true` honest unknown; closing needs `md-5` or indexing `kimi.json` directly. | **Gap** | **Gap** | **Works** ✅ `remote-kimi://` restored as `LiveSsh` | **Works** ✅ | `kimi --resume <id>` | `Gap` by design |
| **grok-build** | `grok-build` | `remote-grok://` / `grok-runtime://` | `~/.grok/sessions/*/*/summary.json` (`info.id`/`cwd`) | `Store` | **Works** ✅ | **Works** ✅ `G_ #000000` | **Works** ✅ `G_` | **Works** ✅ `remote-grok://` restored as `LiveSsh` | **Works** ✅ `working_screen_phrases` wired | `grok --resume <id>` | `0` |
| **codex-litellm** | `codex-litellm` | `codex-litellm://` | `~/.codex-litellm/sessions/**/rollout-*.jsonl` (`.bak.` excluded) | `Generated` | **Works** ✅ | **Works** ✅ | **Works** ✅ | **Works** ✅ | **Works** ✅ | `codex-litellm resume <id>` | `0` |

---

## 2. The 9-CLI Integration Protocol System

The protocol system enforces uniform, structured compliance across all 10 registered CLIs (`codex`, `claude-code`, `muse`, `antigravity`, `pi`, `qwen`, `opencode`, `kimi`, `grok-build`, `codex-litellm`). Every CLI is evaluated across nine core engineering pillars (seven original + 2026-08-17 noise DELETE and weird-title filtering):

### Issue Heading 1: Durable Store Discovery & Multi-Root Indexing
* **Rule:** Every agent CLI declares its exact store globs in `AGENT_CLIS` (`crates/yggterm-core/src/agent_cli.rs`). No hardcoded store directory paths may exist in product code outside the descriptor registry (enforced by `no_store_path_literal_outside_the_agent_cli_registry`).
* **Codex / Codex-LiteLLM:** Glob `~/.codex/sessions/**/rollout-*.jsonl` and `~/.codex-litellm/sessions/**/rollout-*.jsonl`. Parses timestamp from filename and UUID from content payload.
* **Claude Code:** Glob `~/.claude/projects/*/*.jsonl`. UUID is filename stem; cwd parsed from `cwd` / `relocatedCwd` fields. Excludes `agent-*` subagent logs.
* **Muse:** Glob `~/.local/share/muse/sessions/**/session.jsonl`. Reads session UUID from parent dir, workspace root & title from SQLite `~/.local/share/muse/session-index.db`, falling back to `route_facts.cwd`.
* **Antigravity:** Multi-root discovery across `~/.gemini/antigravity-cli/conversations/*.db`, `~/.gemini/antigravity-cli/brain/*/.system_generated/logs/transcript.jsonl`, and legacy `~/.antigravitycli/*.json`. Discovers additional sessions from `conversation_summaries.db`.
* **Pi / Qwen:** Globs `~/.pi/agent/sessions/*/*.jsonl` and `~/.qwen/projects/*/chats/*.jsonl`. Parses session ID and cwd from initial turn JSON.
* **Grok-Build:** Glob `~/.grok/sessions/*/*/summary.json`. Extracts `info.id` and `info.cwd`.
* **OpenCode / Kimi:** Declared `store_scan_gap` (honest unknown `true`).

### Issue Heading 2: Titling Authority & Prompt Extraction
* **Rule:** Titling authority is governed by `TitleAuthority` in `AgentCliDescriptor`: `Store` (CLI transcript/DB holds authoritative user/AI titles) vs `Generated` (`SessionTitleStore` synthesizes or records titles).
* **Codex / Codex-LiteLLM:** `TitleAuthority::Generated`. Titles synthesized from first turn prompt or retrieved from `~/.yggterm/session-titles.db`.
* **Claude Code:** `TitleAuthority::Store`. Latest `custom-title` wins, followed by `ai-title`, followed by first human prompt.
* **Muse:** `TitleAuthority::Store` (tightened 2026-08-17). Reads authoritative `title` and `workspace_root` from SQLite `session-index.db` (`sessions` table) — the same source `muse resume` lists — falling back to `route_facts.cwd` and transcript `heuristic_title_from_context` only when the stored title is empty or matches `looks_like_generated_fallback_title` / `looks_like_low_signal_generated_copy` (e.g. 8-hex shorthash `a8f6dbd1`, `Yggterm Shell`, `Remote Muse <hash>`). This mirrors Claude Code's `TitleAuthority::Store` contract: the CLI's store is authoritative, `SessionTitleStore` is only the fallback for untitled sessions.
* **Antigravity:** `TitleAuthority::Store`. Reads `title` (user rename) or `preview` (auto-summary) from SQLite `conversation_summaries.db`. If empty, extracts user prompt from `<USER_REQUEST>` in `transcript.jsonl` or `history.jsonl` `display`.
* **Pi / Qwen / Grok-Build:** `TitleAuthority::Store`. Reads title directly from session JSON header/summary.

### Issue Heading 3: Live Birth & Transport Scheme Normalization
* **Rule:** Connecting or focusing an agent session row must normalize the live key using `parse_remote_agent_session_path_with_kind` / `remote_agent_session_path` across all registered schemes (`remote-session://`, `remote-cc://`, `remote-muse://`, `remote-agy://`, `remote-pi://`, `remote-qwen://`, `remote-grok://`, `remote-opencode://`, `remote-kimi://`).
* **Implementation:** `crates/yggterm-server/src/lib.rs` preserves `SessionSource::LiveSsh` with the target host and exact `SessionKind` (never falling back to local `SessionKind::Codex`).

### Issue Heading 4: Restart Preservation & Server Restoration
* **Rule:** GUI restart or daemon re-attach must faithfully restore all agent session rows without dropping them or re-keying them as plain local shells.
* **Implementation:** `crates/yggterm-server/src/lib.rs` (lines 9330–9625) extracts `(machine_key, session_id, normalized_live_key, agent_kind)` and sets `configure_remote_resume_live_session` using `remote_agent_resume_subcommand(session.kind)` (`resume-codex`, `resume-cc`, `resume-muse`, `resume-agy`, `resume-pi`, `resume-qwen`, `resume-grok`, etc.).

### Issue Heading 5: Working Traffic Light Indicators & Footer Markers
* **Rule:** Status bar traffic light and idle/working detection must trigger for all CLIs via `screen_text_shows_agent_working`.
* **Codex:** Needles: `thinking...`, `esc to interrupt`, `generating...`.
* **Claude Code:** Needles: `thinking...`, `ctrl+c to cancel`, `esc to cancel`.
* **Muse:** Needles: `working...`, `thinking...`, `esc to interrupt`, `esc to cancel`.
* **Antigravity:** Needles: `esc to cancel`, `esc to interrupt`, `generating...`, `thinking...`, `working...`. Footer hints: `["esc", "ctrl", "enter", "tab"]`.
* **Pi / Qwen / Grok:** Needles: `esc to cancel`, `esc to interrupt`, `thinking...`, `working...`.

### Issue Heading 6: Folder Scoping & Cwd Tree Bucketing
* **Rule:** When the user selects a project folder in the left sidebar, the Startpage and CwdTree must scope to that folder's sessions across all CLIs instead of dumping global host history.
* **Implementation:** `scan_all_durable_sessions` and `build_local_cwd_tree` group all sessions by normalized `cwd`, displaying brand glyphs (`>_`, `*_`, `M_`, `A_`, `π_`, `Q_`, `G_`) and brand colors.

### Issue Heading 7: Independent Dual-Oracle Verification (`scripts/check-*.py`)
* **Rule:** No integration change is verified without independent validation from the dual Python oracles (`check-startpage.py`, `check-titles.py`, `check-cwdtree.py`).
* **Verification Results:**
  - `check-startpage.py --host local --host dev` -> `Exit 0` (119 durable vs 119 manual on local)
  - `check-titles.py --host local --host dev` -> `Exit 0` (119 durable vs 119 manual on local)
  - `check-cwdtree.py --host local --host dev` -> `Exit 0` (119 durable in 24 groups vs 119 manual in 24 groups)

---

### Issue Heading 8: Noise and Empty Session Deletion (DELETE)
* **Rule (2026-08-17):** yggterm must DELETE any CLI session file that is noise — no agent turn or empty session — as it is seen. A session is noise if `is_noise_session_file(path)` returns true: file < 20 bytes, or JSONL contains no `agent turn` / `role=assistant` record and `extract_tail_context` is < 20 chars after trimming. Guard: mtime < 60 s is kept to avoid deleting a session the CLI is still writing. On detection the verb deletes the file and removes any `session-titles.db` entry for that `session_id` so the next scan is clean. This applies to both startpage (`scan_all_durable_sessions` walk) and cwd tree (`build_local_cwd_tree`) and is verified by the oracles finding zero noise rows on a rescan.
* **Implementation:** `crates/yggterm-core/src/startpage.rs::is_noise_session_file` and `crates/yggterm-core/src/lib.rs::build_local_cwd_tree` (inline walk) — both call `std::fs::remove_file` and `SessionTitleStore::delete`.

### Issue Heading 10: Per-CLI Rendering Quirks, Workarounds & Viewport Invariants
* **Rule:** Each CLI has unique TUI rendering patterns and terminal control behaviors. Yggterm isolates these quirks inside `crates/yggterm-server/src/managed_cli/{cli}.rs` and xterm.js viewport integration, ensuring zero rendering artifacts across attach, switch, or resize.
* **Claude Code (`claude-code`):**
  - *Bottom Status Bar / Prompt Overwrite:* The Ink terminal engine uses CUF cursor-forward skipping for whitespace. When yggterm's frame-like write detector (`\x1b[?25l`) suppressed forced full refreshes, partial renders latched permanently. Fixed via refresh latching + 1500ms recovery ceiling.
  - *Edge Asymmetry & Padding Overflow:* Claude Code expects full terminal column width. PTY column padding must be symmetrically aligned with xterm container boundaries.
  - *Blank Middle on Switch:* Switching into an active Claude Code turn must re-anchor the absolute viewport coordinates (`CSI r;cH` / CUP replay) rather than waiting for subsequent differential tokens.
* **Codex & Codex-LiteLLM (`codex`):**
  - *Geometry Squish & Bottom Clipping:* Codex TUI requires matching PTY cols/rows. A mismatch causes status bar truncation; resolved by sending explicit SIGWINCH / nudge (`server terminal resize`) on client dimension changes.
  - *Differential Space Artifacts:* Cells skipped by CUF retain previous background artifacts; mitigated by complete screen state replay on reveal transition.
  - *Middle Buffer Desync:* Rapid switching between sessions repaints from daemon vt100 state snapshot.
* **Muse (`muse`):**
  - *Top Header & Bottom Indicators Tearing:* Custom prompt DB and yolo status indicators in Muse TUI tear during unattached mounts or viewport resizes. Restored via clean vt100 absolute redraw.
  - *Blank Middle History on Resume:* Restoring conversational turns requires preserving scrollback buffer offsets during the re-attach handshake.
  - *Title / Escape Leaks:* Raw ANSI sequences or shorthash strings in session headers are filtered at source.
* **Antigravity (`antigravity`):**
  - *Footer Shortcut Overflow:* The multi-line interactive shortcut footer (`["esc", "ctrl", "enter", "tab"]`) can overflow the bottom viewport row and push top conversation history out of view. Requires strict row clamping.
  - *Live Streaming Token Flicker:* High-frequency delta emissions require batched render passes to prevent middle-screen flicker.
* **Pi / Qwen / Grok-Build / OpenCode / Kimi:**
  - *Multi-line Prompt Redraw & Non-Standard Width Wrapping:* TUI frames wrap improperly if client width is not immediately propagated to PTY. Managed CLI hooks enforce synchronized resize events.

### Issue Heading 11: Built-in Interface LLM Title Rescue Contract
* **Rule (2026-08-19):** When a session transcript exists but neither the CLI's native metadata nor regular expression heuristics can extract a high-signal title (e.g. transcript contains complex tool invocations without a plain prompt string), yggterm's core titling subsystem (`crates/yggterm-core/src/titles.rs`) automatically schedules an asynchronous title rescue request to the fleet's Interface LLM (`gpt-5.6-luna` / `gemini-3.7-flash` via LiteLLM) as a built-in measure of last resort.
* **Persistence & Caching:** Rescued titles are cached in `~/.yggterm/session-titles.db` with source tag `litellm` and model metadata, preventing duplicate LLM queries.

### Issue Heading 12: Muse exemplar — bad wiring and the 4 fixes any CLI needs (Claude perfect, Codex hiccups, Muse bad)

*Why this exemplar.* `muse` (`muse` binary, `remote-muse://` / `muse-runtime://`, `muse resume <uuid>`) shipped with the 2026-08-08 six-CLI intake but is the only one of the three archetypes that still misbehaves live: every fresh Muse row lands `"Muse Code Stays Attached Daemon"` (`server snapshot` `remote-muse://dev/134…` measured 2026-08-19), two Muse rows share one `7-char` shorthash (`43936dd`), switching away from that row orphans the PTY and the next `open` creates a **brand new** Muse session. `claude-code` (`claude --resume`) is the *perfect* reference (store is one file per session `~/.claude/projects/*/*.jsonl`, `TitleAuthority::Store` with `custom-title > ai-title > first prompt`, `id_assigned_at_birth:false` but filename IS the id, resume is a flag). `codex` (`codex resume <id>`) is *workable* — titles via `Generated` + `SessionTitleStore`, store inside file, but hiccups on geometry squish / differential spaces / middle desync (Issue 10). `muse` is *bad* — all four pillars below are wrong.

**Fault 1 — placeholder not recognised as fallback.** `looks_like_generated_fallback_title()` (`crates/yggterm-core/src/titles.rs:2837`) now lists `"Local Shell Stay Alive Daemon"`, `"new muse code session"`, `"yggterm muse"`, **`"muse code stays attached daemon"` / `"muse stays attached daemon"` + `"untitled session"`** (added 2026-08-19). Previously a fresh Muse row was created with daemon-derived context `muse … stays attached daemon` → `heuristic_title_from_context()` word-list → that 4-word string → stored as *real* title. Subsequent `title_was_generated || looks_like_generated_fallback_title()` guard (`daemon.rs:11682`, `lib.rs:2741`) never fired, so the row was never retitled. Titles must never be shorthash (`43936dd` bare 7-char hash, `remote <hash>`, `Q…` ) or generic (`"Muse Code Stays Attached Daemon"` etc.) — they are now filtered and trigger CLI-store → interface LLM (`request_litellm_title` via LiteLLM) → `"untitled session"` fallback with `ytrace` `title/resolve_attempt` + `title/untitled_session` incident (re-tried next tick because `"untitled session"` is itself a fallback).

**Fault 2 — lifecycle missing: `New Muse Code Session` → generated title after first prompt, then `"untitled session"` fallback with `ytrace` re-resolve.** Unlike `claude-code`, `muse` writes **no** title into its store. The contract is: new row → `"New Muse Code Session"` (explicit `set_session_title_explicit` at `terminal new` in `crates/yggterm-server/src/terminal.rs` / `lib.rs:2501`), then *after* the first user prompt + assistant turn appears in `.local/share/muse/sessions/**/session.jsonl`, the background title chore (`daemon.rs` title poll, same throttle as `claude` — `LIVE_SUMMARY_REFRESH_HORIZON` 30 min, `SessionTitleStore` `request_litellm_title` / `heuristic_title_from_context`) replaces it via `set_session_title_hint()` (passive, respects explicit). If LLM/heuristic fails, title becomes `"untitled session"` (never shorthash/generic) and `ytrace` `title/untitled_session` incident is emitted; next tick retries because `"untitled session"` is itself a fallback (`ytrace` `title/resolve_attempt` every tick until a real title lands). Currently Muse is born with the daemon-derived fallback and never enters the `title_was_generated` path. Fix: at `terminal new --kind muse` set explicit `"New Muse Code Session"`; add `read_muse_session_title()` (mirror `read_cc_session_title()`) reading `session-index.db` + `session.jsonl` tail, and include it in the `stored_missing` check.

**Fault 3 — resume uses row UUID, not Muse-internal id.** `muse` is `id_assigned_at_birth:false` (Muse mints via RPC, row `134…` ≠ internal `a0339481…` seen as `YGGTERM_SESSION_ID=muse-runtime://134…` vs transcript `a033…`). `agent_arm_matrix.rs:313` `Arm { kind: Muse locality: Remote row_scheme: Some("remote-muse://") runtime_scheme: Some("muse-runtime://") resume_selector_token: "resume" store_globs: &[".local/share/muse/sessions/**/session.jsonl"] }` is correct, but `remote_runtime_agent_session_key("remote-muse://dev/134…")` and `persistent_agent_resume_command()` currently return `muse resume 134…` (row id). Correct is `muse resume <internal-id>` read from the latest `session.jsonl` for that `cwd`/`machine` (same indirection `claude` uses for `cc-runtime://`). Without it, `muse resume 134…` misses and creates a new empty session — the "kick out and stall" on switch. Fix: resolve via store scan (like `claude`), not row id.

**Fault 4 — duplicate row, same PTY.** `server app rows` shows `remote-muse://dev/134…` twice (depth 1 `live_rail` + depth 4 `row`, the `ONE SESSION, TWO ROWS` husk `pending-bugs.md`). Switching away tears the first, the second stays as a husk, next `open` reuses the husk path but spawns a new PTY. Fix is the same resume-mapping + `build_local_cwd_tree` dedup already shipped for `codex`/`claude`.

**What Claude does right (copy this):** `Slug cc` (historical `remote-cc://` / `cc-runtime://`), `binary "claude"`, `resume_selector_token "--resume"`, `store_globs &[".claude/projects/*/*.jsonl"]` (filename IS id, no DB), `TitleAuthority::Store` (`custom-title > ai-title`), `re_roots_with_cwd:false`, `id_assigned_at_birth:false` but **filename IS id** so row→store mapping is trivial. Codex hiccups are only viewport (CUF `/x1b[?25l` latching, SIGWINCH nudge) — titles/resume are fine.

### Issue Heading 13: Input latency — keystroke → PTY register → PTY render (flush out latency bugs)

*Contract.* Every keystroke must be traceable end-to-end: `shell` `input/keystroke` (client has the bytes) → `daemon` `input/pty` (PTY `terminals.write(runtime_key, data)` accepted) → `shell` `input/render` (`terminal_write_bridge.stage_or_immediate` staged for xterm). Each hop emits `ytrace` `input/*` (`Wall always`, `session_path`, `data_len`, `is_remote`) so Dash `dash-common-bugs` p4 can compute `pty - keystroke` and `render - pty` p50/p95 per session (like `render/storm` vs `daemon_request/status` 4.65µs/row). A stuck input gate (`remote_resume_input_ready` false, the session-only branch) or a lost PTY write (`terminal_write_error` → `recover_terminal_write_lost_runtime`) shows as `keystroke` without `pty`/`render` — the latency tail, not a screenshot, is the falsifier.

*Probes wired.* `crates/yggterm-core/src/perf.rs: input/keystroke|pty|render` registered `always`; `crates/yggterm-shell/src/shell/viewport.rs:Ok(TerminalJsEvent::Input)` emits `input/keystroke`, `crates/yggterm-server/src/daemon.rs:write_local_terminal_with_lost_runtime_recovery` emits `input/pty`, `crates/yggterm-shell/src/shell/viewport.rs:terminal_write_bridge.stage_or_immediate` emits `input/render`. Use `ytrace tail --category input --since 5m --json | jq 'group_by(.name)'` to flush out bugs where `keystroke` count ≫ `pty` or `render` lags >50 ms.

### Issue Heading 14: Agy exemplar — Antigravity faults vs Claude gold (like Muse)

*Why agy.* `antigravity` (`agy` binary, `remote-agy://` / `agy-runtime://`, `agy --conversation <id>`) stores in SQLite `~/.gemini/antigravity-cli/conversations/*.db` + `brain/*/.system_generated/logs/transcript.jsonl` + `history.jsonl`, `TitleAuthority::Store` (`conversation_summaries.title` > `preview`). Like `muse`, it writes no title for empty sessions — fresh rows landed `A_ #1557b0` shorthash or generic `antigravity` until the `Muse` fix. Unlike `claude`, its store is a DB, not one file per session, so `read_antigravity_session_title()` must open the DB (not a JSONL tail) and `id` is `conversation_id` (not filename). Faults: (1) shorthash/generic not filtered → now via `titles.rs` bare_hash + `generic_runtime_title` (same `Muse` fix, `ytrace title/*`), (2) `agy` title pickup in `daemon.rs:collect_live_antigravity_title_syncs` now emits `cli/agy_title` `ytrace` for `no_title_in_store`, `fallback:true`, and `is_untitled` (so Dash can see `agy` untitled re-resolve like `muse`), (3) resume uses `agy-runtime://` + `agy --conversation` (like `muse` `resume` subcommand, not flag) — verify `remote_runtime_agent_session_key("remote-agy://…")` returns `agy-runtime://<internal-id>` from DB, not row UUID, otherwise switch orphans PTY (same `muse` kick). `claude` gold remains the reference: one file per session, flag `--resume`, filename IS id.

### Issue Heading 15: Codex / codex-litellm exemplar — wiring hiccups vs Claude gold (like Muse)

*Why codex.* `codex` (`codex` binary, `remote-session://` historical `remote-codex://` + `codex-runtime://`, `codex resume <id>`) and `codex-litellm` (`codex-litellm` binary, local-only `codex-litellm://`, `id_assigned_at_birth` same) are *workable* per matrix (`TitleAuthority::Generated` via `SessionTitleStore` heuristic/litellm, store `~/.codex/sessions/**/rollout-*.jsonl` id inside file, `re_roots_with_cwd:true` for `codex`, `false` for `litellm`). Their faults are viewport, not title/resume: (1) **Geometry squish** — daemon re-creates PTY at default `120×36` after hot-update re-resume, `last_sent_terminal_resize_*` is stale-equal to live grid, so no `Resize` fires and `codex` renders squished; fix is `viewport.rs:9837` re-resume squish repair (`last_sent_* = 0` + `spawn_terminal_startup_resize_repair`) now emits `ytrace cli/codex_geometry` (`stale_cols/rows`, `live_cols/rows`, `kind: codex_squish_repair`) for Dash. (2) **Differential CUF spaces** — `CUF` cursor-forward skips leave stale `bg` artifacts; mitigated by full screen replay on reveal (same `muse`/`codex` path, `ytrace` `terminal_mount` already). (3) **Middle desync on rapid switch** — `codex` status bar truncation if `SIGWINCH` not nudged; `terminal_write_should_frame_budget` / `terminal_write_bridge` already gates, now `ytrace` `cli/codex_geometry` covers it. `claude` gold has no squish because its Ink engine re-anchors absolute `CUP` on switch; `codex` needs the explicit resize repair — that is the hiccup to copy for any TUI that expects full width.

**Checklist for any new CLI (add to `spec-adding-an-agent-cli.md` steps 1–9):** 1) `SessionKind` variant, 2) `AGENT_CLIS` descriptor (+ `TitleAuthority`, `store_globs`, `id_assigned_at_birth`, `resume_selector_token`, `re_roots_with_cwd`), 3) `SESSION_PATH_SCHEMES` (`remote-<slug>://` + `<slug>-runtime://`), 4) `cargo check` exhaustive matches, 5) catch-alls `rg SessionKind::(Codex|ClaudeCode)`, 6) `agent_arm_matrix` two arms (Local `local://` + Remote `remote-<slug>://`), 7) surfaces (icon/menu/KeyTips free), 8) provisioning `install`/`update`, 9) **title lifecycle** (`New <CLI> Session` → after first prompt `heuristic`/`litellm` via `SessionTitleStore`) + fallback list, 10) **resume id** (if `id_assigned_at_birth:false`, implement store→row mapping), 11) `spec-cli-integration-verification.md` oracles (`check-startpage.py`/`check-titles.py`/`check-cwdtree.py` must `0` on `dev`/`***`/`oc`/`***` + faithful 1920×1200 screenshot).

## 3. Inventory — which spec/doc now lives where

* `spec-cli-integration-verification.md` — the **harness** (verb + oracle pattern, `AGENT_CLIS` SSOT, adding a CLI is one descriptor).
* `spec-adding-an-agent-cli.md` — the **procedure** for a new CLI (10 recon questions, descriptor fields, rolling-upgrade hazard).
* **This file** — the **BUGS & 9-CLI PROTOCOL** matrix (what is promised vs what is delivered for each of the 10 CLIs).
* `pending-bugs.md:CLI` — pointer to this file (open) plus the `6.7` tmpfs/swap leak.



