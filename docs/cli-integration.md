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
exits `0` on `openclaw`/`oc`/`dev`/`jojo` **and** a 1920×1200 `os`/`xterm` faithful screenshot
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
`openclaw`/`dev`/`jojo` **and** a faithful screenshot with the right glyph/colour/title.
"Shipped" means code landed but the verb/oracle still reports a lie. "Gap" means the descriptor
declares `store_scan_gap` (honest `unknown`, not `false`).

| CLI | slug | schemes | store | `TitleAuthority` | startpage `durable` | titles `effective_title` | cwdtree `icon` | live `kind`/`source` | resume `ready` | launch/resume cmd | `check-*` oracle |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **codex** | `codex` | `remote-session://` / `codex-runtime://` `local://` | `~/.codex/sessions/**/rollout-*.jsonl` (id inside file, name carries timestamp) | `Generated` (none in file) | **Works** ✅ `order_for_startpage` `modified_epoch` | **Works** ✅ `>_` `#0f766e` — generated title via `SessionTitleStore` | **Works** ✅ `>_` `#0f766e` `build_local_cwd_tree` | **Works** ✅ `codex` `codex-runtime://` `RemoteBootstrap` | **Works** ✅ `server resume ls` probes `daemon_owns_runtime+attach_ready_seen` (gates neutered 2026-08-16) | `codex resume <id>` measured | `check-startpage/titles/cwdtree` `0` |
| **claude-code** | `claude-code` | `remote-cc://` / `cc-runtime://` | `~/.claude/projects/*/*.jsonl` (filename is id, `custom-title` > `ai-title`) | `Store` (`custom` > `ai`) | **Works** ✅ | **Works** ✅ `*_ #c2410c` | **Works** ✅ `*_` | **Works** ✅ `claude_code` `cc-runtime://` | **Works** ✅ | `claude -r <id>` | `0` |
| **muse** | `muse` | `remote-muse://` / `muse-runtime://` | `~/.local/share/muse/sessions/**/session.jsonl` + `session-index.db` (`sessions.workspace_root→cwd, title, updated_at_us`, fallback `route_facts.cwd`) | `Generated` (store `None`, DB prompt) | **Works** ✅ `scan_all_durable_sessions` (`M_ #86198f` via `SessionTitleStore` DB prompt) | **Works** ✅ `server titles ls` (`M_` + `effective_title` = DB prompt) | **Works** ✅ `M_ #86198f` | **Works** ✅ `remote-muse://` restored as `LiveSsh` with `kind: SessionKind::Muse` and `resume-muse` | **Works** ✅ `working_screen_phrases` wired | `muse resume <uuid>` / `muse --yolo` | `0` |
| **antigravity** | `antigravity` | `remote-agy://` / `agy-runtime://` | `~/.gemini/antigravity-cli/conversations/*.db`, `brain/*/.system_generated/logs/transcript.jsonl`, `history.jsonl` | `Store` (`conversation_summaries`) | **Works** ✅ `4` durable `A_ #1557b0` | **Works** ✅ `conversation_summaries` title + `history.jsonl` / prompt fallback | **Works** ✅ `A_ #1557b0` | **Works** ✅ `remote-agy://` restored as `LiveSsh` with `kind: SessionKind::Antigravity` and `resume-agy` | **Works** ✅ `working_screen_phrases` wired | `agy --conversation <id>` | `0` |
| **pi** | `pi` | `remote-pi://` / `pi-runtime://` | `~/.pi/agent/sessions/*/*.jsonl` (first line `id`/`cwd`) | `Store` | **Works** ✅ | **Works** ✅ `π_ #be185d` | **Works** ✅ `π_` | **Works** ✅ `remote-pi://` restored as `LiveSsh` with `SessionKind::Pi` | **Works** ✅ | `pi --session <id>` | `0` |
| **qwen** | `qwen` | `remote-qwen://` / `qwen-runtime://` | `~/.qwen/projects/*/chats/*.jsonl` (first line `id`/`cwd`, exclude `.runtime.`) | `Store` | **Works** ✅ | **Works** ✅ `Q_ #6d28d9` | **Works** ✅ `Q_` | **Works** ✅ `remote-qwen://` restored as `LiveSsh` with `SessionKind::QwenCode` | **Works** ✅ | `qwen --resume <id>` | `0` |
| **opencode** | `opencode` | `remote-opencode://` / `opencode-runtime://` | `~/.local/share/opencode/opencode.db` single SQLite (`session` table `id/directory/title`) — **declared-unscannable** (`store_scan_gap` true) | `Store` | **Gap — by design** — `scan` is honest `unknown` (`true` not `false`): `server <area> ls` declares `store_scan_gap` warning. | **Gap** | **Gap** | **Works** ✅ `remote-opencode://` restored as `LiveSsh` | **Works** ✅ | `opencode --session <id>` | `Gap` by design |
| **kimi** | `kimi` | `remote-kimi://` / `kimi-runtime://` | `~/.kimi/sessions/<md5(cwd)>/<id>/context.jsonl` — **declared-unscannable** (`md5(cwd)` bucket, `cwd` not recoverable from path) | `Store` | **Gap — by design** — same `true` honest unknown; closing needs `md-5` or indexing `kimi.json` directly. | **Gap** | **Gap** | **Works** ✅ `remote-kimi://` restored as `LiveSsh` | **Works** ✅ | `kimi --resume <id>` | `Gap` by design |
| **grok-build** | `grok-build` | `remote-grok://` / `grok-runtime://` | `~/.grok/sessions/*/*/summary.json` (`info.id`/`cwd`) | `Store` | **Works** ✅ | **Works** ✅ `G_ #000000` | **Works** ✅ `G_` | **Works** ✅ `remote-grok://` restored as `LiveSsh` | **Works** ✅ `working_screen_phrases` wired | `grok --resume <id>` | `0` |
| **codex-litellm** | `codex-litellm` | `codex-litellm://` | `~/.codex-litellm/sessions/**/rollout-*.jsonl` (`.bak.` excluded) | `Generated` | **Works** ✅ | **Works** ✅ | **Works** ✅ | **Works** ✅ | **Works** ✅ | `codex-litellm resume <id>` | `0` |

---

## 2. The 9-CLI Integration Protocol System

The protocol system enforces uniform, structured compliance across all 10 registered CLIs (`codex`, `claude-code`, `muse`, `antigravity`, `pi`, `qwen`, `opencode`, `kimi`, `grok-build`, `codex-litellm`). Every CLI is evaluated across seven core engineering pillars:

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
* **Muse:** `TitleAuthority::Generated`. Reads custom prompt/title from SQLite `session-index.db`, falling back to `SessionTitleStore`.
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

## 3. Inventory — which spec/doc now lives where

* `spec-cli-integration-verification.md` — the **harness** (verb + oracle pattern, `AGENT_CLIS` SSOT, adding a CLI is one descriptor).
* `spec-adding-an-agent-cli.md` — the **procedure** for a new CLI (10 recon questions, descriptor fields, rolling-upgrade hazard).
* **This file** — the **BUGS & 9-CLI PROTOCOL** matrix (what is promised vs what is delivered for each of the 10 CLIs).
* `pending-bugs.md:CLI` — pointer to this file (open) plus the `6.7` tmpfs/swap leak.


