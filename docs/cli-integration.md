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
exits `0` on every fleet host **and** a 1920×1200 `os`/`xterm` faithful screenshot
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
every fleet host **and** a faithful screenshot with the right glyph/colour/title.
"Shipped" means code landed but the verb/oracle still reports a lie. "Gap" means the descriptor
declares `store_scan_gap` (honest `unknown`, not `false`). ⚠ Empty `session_store_globs`
does NOT mean gapped — see §Scanned, not gapped.

| CLI | slug | schemes | store | `TitleAuthority` | startpage `durable` | titles `effective_title` | cwdtree `icon` | live `kind`/`source` | resume `ready` | launch/resume cmd | `check-*` oracle | render notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **codex** | `codex` | `remote-session://` / `codex-runtime://` `local://` | `~/.codex/sessions/**/rollout-*.jsonl` (id inside file, name carries timestamp) | `Generated` (none in file) | **Works** ✅ `order_for_startpage` `modified_epoch` | **Works** ✅ `>_` `#0f766e` — generated title via `SessionTitleStore` | **Works** ✅ `>_` `#0f766e` `build_local_cwd_tree` | **Works** ✅ `codex` `codex-runtime://` `RemoteBootstrap` | **Works** ✅ `server resume ls` probes `daemon_owns_runtime+attach_ready_seen` (gates neutered 2026-08-16) | `codex resume <id>` measured | `check-startpage/titles/cwdtree` `0` |
| **claude-code** | `claude-code` | `remote-cc://` / `cc-runtime://` | `~/.claude/projects/*/*.jsonl` (filename is id, `custom-title` > `ai-title`) | `Store` (`custom` > `ai`) | **Works** ✅ | **Works** ✅ `*_ #c2410c` | **Works** ✅ `*_` | **Works** ✅ `claude_code` `cc-runtime://` | **Works** ✅ | `claude -r <id>` | `0` |
| **muse** | `muse` | `remote-muse://` / `muse-runtime://` | `~/.local/share/muse/sessions/**/session.jsonl` + `session-index.db` (`sessions.workspace_root→cwd, title, updated_at_us`, fallback `route_facts.cwd` → `heuristic_title_from_context`) | `Store` (tightened 2026-08-17, `session-index.db.title` authoritative, shorthash/weird filtered) | **Works** ✅ `scan_all_durable_sessions` (`M_ #86198f`, noise SKIPPED — not deleted, see Issue 8 — weird-title filtered, `is_agent_session` gated) | **Works** ✅ `server titles ls` (`M_` + `effective_title` Store-heuristic, shorthash `a8f6dbd1` filtered) | **Works** ✅ `M_ #86198f` | **Works** ✅ `remote-muse://` restored as `LiveSsh` with `kind: SessionKind::Muse` and `resume-muse` | **Works** ✅ `working_screen_phrases` wired | `muse resume <uuid>` / `muse --yolo` | `0` |
| **antigravity** | `antigravity` | `remote-agy://` / `agy-runtime://` | `~/.gemini/antigravity-cli/conversation_summaries.db` (the INDEX — 999 rows, of which 4 are sessions), `brain/*/.system_generated/logs/transcript_full.jsonl`, `history.jsonl` | `Store` (`conversation_summaries`) | **Works** ✅ `4` durable `A_ #1557b0` — the other 995 are batch conversations, see §The agy durable rule | **Works** ✅ `conversation_summaries` title + `history.jsonl` / prompt fallback | **Works** ✅ `A_ #1557b0` | **Works** ✅ `remote-agy://` restored as `LiveSsh` with `kind: SessionKind::Antigravity` and `resume-agy` | **Works** ✅ `working_screen_phrases` wired | `agy --conversation <id>` | `0` |
| **pi** | `pi` | `remote-pi://` / `pi-runtime://` | `~/.pi/agent/sessions/*/*.jsonl` (first line `id`/`cwd`) | `Store` | **Works** ✅ | **Works** ✅ `π_ #be185d` | **Works** ✅ `π_` | **Works** ✅ `remote-pi://` restored as `LiveSsh` with `SessionKind::Pi` | **Works** ✅ | `pi --session <id>` | `0` |
| **qwen** | `qwen` | `remote-qwen://` / `qwen-runtime://` | `~/.qwen/projects/*/chats/*.jsonl` (first line `id`/`cwd`, exclude `.runtime.`) | `Store` | **Works** ✅ | **Works** ✅ `Q_ #6d28d9` | **Works** ✅ `Q_` | **Works** ✅ `remote-qwen://` restored as `LiveSsh` with `SessionKind::QwenCode` | **Works** ✅ | `qwen --resume <id>` | `0` | ⚠ **render: UPSTREAM, not ours** — qwen paints a 3-row header, not the 6-row banner its ASCII art implies, and only ~102 columns, at every grid from 100x30 to 173x200 **in a bare PTY with no yggterm involved** (measured 2026-08-20, `scripts/cli-viewport-probe`). "qwen's motd is cut off at the top" is qwen's own rendering by the wrapper-vs-manual parity rule. Do not add a flag or a clamp for it. |
| **opencode** | `opencode` | `remote-opencode://` / `opencode-runtime://` | `~/.local/share/opencode/opencode.db` single SQLite (`session` table `id/directory/title`) — **scanned by `scan_opencode_sessions`**, no glob can express one DB file | `Store` | **Works** ✅ dedicated scanner (corrected 2026-08-20 — see §Scanned, not gapped) | **Works** ✅ | **Works** ✅ | **Works** ✅ `remote-opencode://` restored as `LiveSsh` | **Works** ✅ | `opencode --session <id>` | `0` |
| **kimi** | `kimi` | `remote-kimi://` / `kimi-runtime://` | `~/.kimi/sessions/<md5(cwd)>/<id>/context.jsonl` — **scanned by `scan_kimi_sessions`**, which reverses the md5 bucket via `kimi.json` `work_dirs[].path` | `Store` | **Works** ✅ dedicated scanner (corrected 2026-08-20 — see §Scanned, not gapped) | **Works** ✅ | **Works** ✅ | **Works** ✅ `remote-kimi://` restored as `LiveSsh` | **Works** ✅ | `kimi --resume <id>` | `0` |
| **grok-build** | `grok-build` | `remote-grok://` / `grok-runtime://` | `~/.grok/sessions/*/*/summary.json` (`info.id`/`cwd`) | `Store` | **Works** ✅ | **Works** ✅ `G_ #000000` | **Works** ✅ `G_` | **Works** ✅ `remote-grok://` restored as `LiveSsh` | **Works** ✅ `working_screen_phrases` wired | `grok --resume <id>` | `0` |
| **codex-litellm** | `codex-litellm` | `codex-litellm://` | `~/.codex-litellm/sessions/**/rollout-*.jsonl` (`.bak.` excluded) | `Generated` | **Works** ✅ | **Works** ✅ | **Works** ✅ | **Works** ✅ | **Works** ✅ | `codex-litellm resume <id>` | `0` |

---

## 2. The 9-CLI Integration Protocol System

The protocol system enforces uniform, structured compliance across all 10 registered CLIs (`codex`, `claude-code`, `muse`, `antigravity`, `pi`, `qwen`, `opencode`, `kimi`, `grok-build`, `codex-litellm`). Every CLI is evaluated across nine core engineering pillars (seven original + 2026-08-17 noise classification — skip, never delete, see Issue 8 — and weird-title filtering):

### Issue Heading 1: Durable Store Discovery & Multi-Root Indexing
* **Rule:** Every agent CLI declares its exact store globs in `AGENT_CLIS` (`crates/yggterm-core/src/agent_cli.rs`). No hardcoded store directory paths may exist in product code outside the descriptor registry (enforced by `no_store_path_literal_outside_the_agent_cli_registry`).
* ⛔ **The scan's home argument is the AGENT STORE home, and `yggterm_core::startpage::agent_store_home` is its ONE resolver (2026-08-20).** `scan_all_durable_sessions` joins descriptor roots onto whatever path it is given, so handing it the yggterm home (`~/.yggterm`) walks `~/.yggterm/.codex/…` and returns **zero rows silently** — a 0 that reads as "no sessions", never as an error. That call shape cost the `ls` verbs once (0 rows, 2026-08-16, each verb then grew a private `dirs::home_dir()` fallback) and then the GUI's local cwd tree for three days (2026-08-17 unification → 2026-08-20): every local durable session without a live row left the sidebar and the start page while the verbs still counted it, and the only visible symptom was a four-row gap between the start page header and `startpage ls`. All five call sites now read the one resolver; `the_local_tree_scan_walks_the_agent_store_home_not_the_yggterm_home` locks the tree builder.
* ⭐ **The count contract:** "how many durable sessions are there" = the deduplicated union (by session id) of the local store scan + every machine scan + live agent rows. `durable_count` in `startpage ls` / `cwdtree ls` is that number, and the start page header shows the same universe — workspace documents (terminal recipes) are not RECENT WORK candidates (`a_workspace_document_is_not_a_start_page_candidate`). Per-machine sidebar counts are per-scan-scope INSTANCE counts and legitimately sum past the fleet-unique total (two machine keys can name one physical host); dedup is per-view, never cross-view, so the tree must NOT be deduplicated to force the sum to match.
* **Codex / Codex-LiteLLM:** Glob `~/.codex/sessions/**/rollout-*.jsonl` and `~/.codex-litellm/sessions/**/rollout-*.jsonl`. Parses timestamp from filename and UUID from content payload.
* **Claude Code:** Glob `~/.claude/projects/*/*.jsonl`. UUID is filename stem; cwd parsed from `cwd` / `relocatedCwd` fields. Excludes `agent-*` subagent logs.
* **Muse:** Glob `~/.local/share/muse/sessions/**/session.jsonl`. Reads session UUID from parent dir, workspace root & title from SQLite `~/.local/share/muse/session-index.db`, falling back to `route_facts.cwd`.
* **Antigravity:** Multi-root discovery across `~/.gemini/antigravity-cli/conversations/*.db`, `~/.gemini/antigravity-cli/brain/*/.system_generated/logs/transcript_full.jsonl`, and legacy `~/.antigravitycli/*.json`. Discovers additional sessions from `conversation_summaries.db`.
* **Pi / Qwen:** Globs `~/.pi/agent/sessions/*/*.jsonl` and `~/.qwen/projects/*/chats/*.jsonl`. Parses session ID and cwd from initial turn JSON.
* **Grok-Build:** Glob `~/.grok/sessions/*/*/summary.json`. Extracts `info.id` and `info.cwd`.
* **OpenCode / Kimi:** Scanned by `scan_opencode_sessions` / `scan_kimi_sessions`. Empty `session_store_globs`, no declared gap — see §Scanned, not gapped. ⛔ **Kimi was `TitleAuthority::Store` until 2026-08-21 and its store holds no title** — `scan_kimi_sessions` says so in its own comment and falls back to a generated or heuristic title, and a structural scan of a real store found no title, cwd or session id key anywhere. So the SCAN path already treated it as generating while the LIVE path honoured the declaration and refused to generate: one CLI, two answers, and the live answer was nobody. It is `Generated` now.

### Issue Heading 2: Titling Authority & Prompt Extraction
* **Rule:** Titling authority is governed by `TitleAuthority` in `AgentCliDescriptor`: `Store` (CLI transcript/DB holds authoritative user/AI titles) vs `Generated` (`SessionTitleStore` synthesizes or records titles).
* **Codex / Codex-LiteLLM:** `TitleAuthority::Generated`. Titles synthesized from first turn prompt or retrieved from `~/.yggterm/session-titles.db`.
* **Claude Code:** `TitleAuthority::Store`. Latest `custom-title` wins, followed by `ai-title`, followed by first human prompt.
* **Muse:** `TitleAuthority::Store` (tightened 2026-08-17). Reads authoritative `title` and `workspace_root` from SQLite `session-index.db` (`sessions` table) — the same source `muse resume` lists — falling back to `route_facts.cwd` and transcript `heuristic_title_from_context` only when the stored title is empty or matches `looks_like_generated_fallback_title` / `looks_like_low_signal_generated_copy` (e.g. 8-hex shorthash `a8f6dbd1`, `Yggterm Shell`, `Remote Muse <hash>`). This mirrors Claude Code's `TitleAuthority::Store` contract: the CLI's store is authoritative, `SessionTitleStore` is only the fallback for untitled sessions.
* **Antigravity:** `TitleAuthority::Store`. Reads `title` (user rename) or `preview` (auto-summary) from SQLite `conversation_summaries.db`. If empty, extracts user prompt from `<USER_REQUEST>` in `transcript_full.jsonl` or `history.jsonl` `display`. ⛔ **The ORDER above is precedence, not likelihood — measured 2026-08-21, the first two sources are usually EMPTY.** Of the eight most recently touched conversations on a real store, **zero** had a row in `conversation_summaries.db` and six had no `history.jsonl` entry, while **all eight** carried a usable prompt in their own `transcript_full.jsonl`. The index is where an OLD title lives and where an owner's rename lands; the transcript is where a NEW conversation's title actually is. A reader wired to the index alone answers `no_title_in_store` for exactly the rows a person is looking at — which is why the first remote probe, though correctly registry-driven, still titled nothing (A/B on that store: 2 of 8 answered before adding the transcript arm, 8 of 8 after).
* **Qwen:** `TitleAuthority::Store`. The title is a `custom_title` record the CLI RE-APPENDS near EOF, so the last one wins and only the tail is read (`read_qwen_custom_title_tail`). ⭐ **Wired for live rows 2026-08-21, local and remote** — it had a measured store entry parser and `read_live_store_title: None` at the same time, which claimed unmeasured about a store two functions already decoded. Combined with `Store` refusing generated copy, a live Qwen row was titled by nothing. ⚠ Its chat file is **not** contractually named for the session, so a lookup matches the file stem first and falls back to the first record's `sessionId`.
* **Pi / Grok-Build:** ⚠ **`TitleAuthority::Generated`, not `Store`** — corrected 2026-08-21, this line named all three as Store and only Qwen is. They are titled by generation like Codex, and nothing reads a title out of their stores.

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

### Issue Heading 8: Noise and Empty Session CLASSIFICATION (the scan never deletes)
⛔⛔ **CORRECTED 2026-08-20 — THIS SECTION DESCRIBED CODE THAT NEVER SHIPPED, AND IT WAS BELIEVED.**
It specified a delete: `remove_file` plus `SessionTitleStore::delete` from both the startpage walk
and `build_local_cwd_tree`, a 20-byte size rule, an `extract_tail_context` rule, and a 60-second
write guard. **None of that exists.** `is_noise_session_file` tests exactly two things — a
zero-length file, and a muse session whose `session-index.db` row has `prompt_count = 0` with a
placeholder title — it has exactly two callers, and **both `continue`**. There is no delete path
in the scan and there never was.

⚠ This is why the correction is written at this length rather than quietly edited: the claim was
carried out of this file into a lane brief as an established fact about our own code, under the
heading *"treat deletion as the highest-stakes code in your lane"*. A doc that describes an
implementation which does not exist is worse than one that says nothing, because it is
authoritative-looking and nobody re-reads the callers.

* **What the scan actually does:** it CLASSIFIES. Noise is skipped in memory, never removed from
  disk. `scanning_never_removes_a_session_file` pins that, and it also pins the case that matters:
  a muse session the index calls noise still has real bytes behind it (measured: four such
  sessions carrying ~12 KB of lifecycle records each, a clean `session_end`, ~12 minutes of
  uptime). Skipping them is right; reading the index row as *"the file is empty"* is not.
* **What may delete, and how:** [`spec-sweep-policy.md`](spec-sweep-policy.md) is the one owner of
  that question. Its §9.2a carries the owner's 2026-08-20 ruling: quarantine first — a `.noise/`
  sidecar beside the store (⛔ never a temp directory; `/tmp` is a tmpfs on at least one fleet
  host, so "quarantine" there is deletion that also spends RAM), swept after 7 days, a ytrace
  incident per action carrying path and reason, and direct `rm` only once quarantine has earned
  trust.
* **Implementation:** `crates/yggterm-core/src/startpage.rs::is_noise_session_file` — a predicate,
  and only a predicate.

### Issue Heading 10: Per-CLI Rendering Quirks, Workarounds & Viewport Invariants

⚠ **WHERE THIS CODE ACTUALLY LIVES — corrected 2026-08-20.** This heading used to
say the quirks are isolated in `crates/yggterm-server/src/managed_cli/{cli}.rs`.
They are not, and following that pointer costs a session: every per-CLI file in
that directory is a **five-line placeholder** (`README.md` there says so — the
split is a rename plus stubs, and extraction is still pending). The render
behaviour is in the shared paths: `managed_cli/mod.rs` (launch/identity),
`yggterm-server/src/terminal.rs` (PTY spawn/restart/resize + the vt100 screen
model), `yggterm-server/src/daemon.rs` (the attach-time grid resync), and
`yggterm-shell/src/shell/viewport.rs` (client seeding, geometry gates, replay).

⛔ **AND THE FIRST RULE IS THAT THERE IS NO PER-CLI GEOMETRY.**
`agent_arm_shell_matrix.rs` states the invariant: **every axis must be a property
of WHERE THE PTY LIVES, never of WHICH CLI is talking.** A quirk entry below
describes what a CLI *does*; it is not a licence to branch the geometry on which
CLI it is. The one place that did is written up in the next paragraph, because it
caused the fault it was meant to fix.

* **Rule:** Each CLI has unique TUI rendering patterns and terminal control behaviors. Yggterm absorbs them in the shared PTY/viewport paths named above, aiming at zero rendering artifacts across attach, switch, or resize.

#### ⛔ REMOVED 2026-08-20: the narrow-TUI PTY clamp (`is_narrow_tui_session`)

*The fault it produced:* "many CLIs have their TUI not covering the entire
viewport and looks broken (Grok Build, OpenCode)".

`terminal.rs` shrank eight named CLIs (`grok`, `opencode`, `qwen`, `kimi`,
`muse`, `agy`/`antigravity`, `pi`) to **120x40** at PTY spawn *and* at restart, on
the premise that they "render at a fixed width (e.g. 100 cols) and leave large
dead margins on a 173-col viewport", so a smaller PTY would make them "fill the
available area and reduce `blank_rows_below_cursor`".

**Measured against the daemon's own vt100** (`scripts/cli-viewport-probe`, which
feeds a real PTY to the same `vt100` crate `terminal.rs` parses with — see the
tell below for why that matters), given a 173x63 PTY:

| CLI | max column painted | verdict |
|---|---|---|
| `grok` | 171 / 173 (98.8%) | fills whatever grid it is given |
| `opencode` | 172 / 173 (99.4%) | fills whatever grid it is given |
| `pi` | 173 / 173 (100%) | fills whatever grid it is given |
| `qwen` | 102 / 173 | genuinely narrow — **and paints the same 102 columns at 120**, so the clamp did not help it either |

⇒ **The premise was false for the CLIs it damaged and irrelevant for the one it
described.** It also could not do what it claimed: the *viewport* is xterm's
grid, which shrinking the PTY does not change — so the clamp only shrank the
app's world and left the remainder of the screen dead. That is the reported
symptom, produced by the workaround.

⭐ **Why it read as plausible, and the general lesson.** The dead margin that
motivated it is real — but its cause is **stale PTY geometry** (a PTY left at a
default or preserved size while the client renders wider), the same class as the
codex squish. Someone saw a CLI painting ~120 columns inside a 173-column
viewport, concluded *this CLI renders narrow*, and made the PTY officially 120 —
**cementing the symptom and giving it a justification**. The tell was in the
comment: it justified itself with `blank_rows_below_cursor`, a telemetry number,
never with a pixel.

⛔ **The half that made it permanent, and the reason it was usually seen.** The
attach path resizes the PTY to the client's grid immediately after
`ensure_session_with_size` (the D1 `reattach_grid_resync`, `daemon.rs`), so a
clamp applied *there* self-healed on the next attach and looked survivable.
**Nothing resyncs behind a RESTART** (`daemon.rs` `restart_session_with_size`
call sites), and the client emits a `Resize` only when its OWN grid changes —
which a daemon-side restart does not do. So a restarted row kept 120x40 for the
rest of its life. **A daemon hot-update restarts every live row**, which is why
the affected CLIs were nearly always found in the broken state.

⚠ **A latent second route, now gone with it.** The predicate also matched a bare
CLI binary name ANYWHERE in the launch command, by `file_name()` compare — so an
ordinary path token whose basename happened to be `pi`, `muse` or `grok` pulled
in rows that were never agent rows, **plain shells included**. On the current
fleet's launch-command shape that fired on 1 of 54 live rows and that one was a
genuine match, so it was latent rather than active — but it is exactly the
"axis that reads the CLI" the matrix invariant forbids.

**Locked by:** `terminal::tests::pty_is_created_at_the_requested_grid_for_every_cli`
(walks the whole former list, both match routes) and
`terminal::tests::a_restart_keeps_the_client_grid_for_a_formerly_clamped_cli`.
Both were confirmed to FAIL against a re-introduced clamp before being trusted.

⚠ **THE PROBE IS PART OF THE FINDING — a hand-rolled vt100 lied first.** The
first measurement used a quick hand-written parser and reported qwen's banner as
"cut off from the top". It ignored alt-screen (`?1049h`) and scroll regions, and
counted a cell as blank when its TEXT was blank — but **gradient banners are
routinely drawn as SPACES with a background colour**, which such a test scores as
unpainted. `scripts/cli-viewport-probe` exists so a coverage number is measured
by the daemon's own eyes, and it reports `bg_only_cells` so that failure mode is
visible rather than silent.
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

*Why this exemplar.* `muse` (`muse` binary, `remote-muse://` / `muse-runtime://`, `muse resume <uuid>`) shipped with the 2026-08-08 six-CLI intake but is the only one of the three archetypes that still misbehaves live: every fresh Muse row lands `"Muse Code Stays Attached Daemon"` (`server snapshot` `remote-muse://dev/134…` measured 2026-08-19), two Muse rows share one `7-char` shorthash (`43936dd`), switching away from that row orphans the PTY and the next `open` creates a **brand new** Muse session. `claude-code` (`claude --resume`) is the *perfect* reference (store is one file per session `~/.claude/projects/*/*.jsonl`, `TitleAuthority::Store` with `custom-title > ai-title > first prompt`, `id_assigned_at_birth:**true**` — CC is BORN with yggterm's uuid via `--session-id`, and the transcript is then named after it, so row→store mapping needs no indirection at all; resume is a flag). `codex` (`codex resume <id>`) is *workable* — titles via `Generated` + `SessionTitleStore`, store inside file, but hiccups on geometry squish / differential spaces / middle desync (Issue 10). `muse` is *bad* — all four pillars below are wrong.

**Fault 1 — placeholder not recognised as fallback.** `looks_like_generated_fallback_title()` (`crates/yggterm-core/src/titles.rs:2837`) now lists `"Local Shell Stay Alive Daemon"`, `"new muse code session"`, `"yggterm muse"`, **`"muse code stays attached daemon"` / `"muse stays attached daemon"` + `"untitled session"`** (added 2026-08-19). Previously a fresh Muse row was created with daemon-derived context `muse … stays attached daemon` → `heuristic_title_from_context()` word-list → that 4-word string → stored as *real* title. Subsequent `title_was_generated || looks_like_generated_fallback_title()` guard (`daemon.rs:11682`, `lib.rs:2741`) never fired, so the row was never retitled. Titles must never be shorthash (`43936dd` bare 7-char hash, `remote <hash>`, `Q…` ) or generic (`"Muse Code Stays Attached Daemon"` etc.) — they are now filtered and trigger CLI-store → interface LLM (`request_litellm_title` via LiteLLM) → `"untitled session"` fallback with `ytrace` `title/resolve_attempt` + `title/untitled_session` incident (re-tried next tick because `"untitled session"` is itself a fallback).

**Fault 2 — lifecycle missing: `New Muse Code Session` → generated title after first prompt, then `"untitled session"` fallback with `ytrace` re-resolve.** ⚠ **Corrected 2026-08-20 — this used to read "`muse` writes **no** title into its store", and that is wrong.** `~/.local/share/muse/session-index.db` has a `title` column and muse fills it with the **first prompt, verbatim, never updated** (measured: two rows carrying whole paragraphs, one of them 900+ characters, `prompt_count 19` and still wearing prompt #1). So the store value is a first-prompt field wearing a title's name, which is worse than an empty one: it is authoritative-looking and it is not a title. `AgentCliDescriptor::store_entry` now clamps any store title to a row label (first sentence, then a word-boundary cut at 72 chars) and keeps the full text as the row's `detail`; the clamp is deterministic and model-free, because the case it exists for is exactly the case where the interface LLM is unreachable. The contract is: new row → `"New Muse Code Session"` (explicit `set_session_title_explicit` at `terminal new` in `crates/yggterm-server/src/terminal.rs` / `lib.rs:2501`), then *after* the first user prompt + assistant turn appears in `.local/share/muse/sessions/**/session.jsonl`, the background title chore (`daemon.rs` title poll, same throttle as `claude` — `LIVE_SUMMARY_REFRESH_HORIZON` 30 min, `SessionTitleStore` `request_litellm_title` / `heuristic_title_from_context`) replaces it via `set_session_title_hint()` (passive, respects explicit). If LLM/heuristic fails, title becomes `"untitled session"` (never shorthash/generic) and `ytrace` `title/untitled_session` incident is emitted; next tick retries because `"untitled session"` is itself a fallback (`ytrace` `title/resolve_attempt` every tick until a real title lands). Currently Muse is born with the daemon-derived fallback and never enters the `title_was_generated` path. Fix: at `terminal new --kind muse` set explicit `"New Muse Code Session"`; add `read_muse_session_title()` (mirror `read_cc_session_title()`) reading `session-index.db` + `session.jsonl` tail, and include it in the `stored_missing` check.

**Fault 3 — RESOLVED 2026-08-20. Root cause was not the resume builder: no CLI outside codex/Claude Code had a runtime-identity rebind at all.** `muse` is `id_assigned_at_birth:false` (it mints its own id via RPC), so a row is born carrying yggterm's row uuid and the CLI's store has never held it. The §7.5 rebind that would replace it existed only as `live_codex_session_keys_for_runtime_identity` / `live_claude_code_session_keys_for_runtime_identity` — both hardcoded to one `SessionKind` — so every later self-minting CLI (`muse`, `antigravity`, `opencode`) kept the phantom **forever**. Measured live: a running muse row carried `local://1344182a…` while the muse process it owned was in session `a0339481…`, and that row uuid appears nowhere in muse's store.

*What the phantom does, measured 2026-08-20 against the real binaries — the two owner symptoms are one bug wearing two faces:*
* `muse resume <phantom>` → `retained session not found: session … has no saved log`, **exit 1**. The PTY dies at open, so the row never persists ⇒ *"no session persists in Live session"*.
* `agy --conversation <phantom>` → `warning: conversation "…" not found` and then a **brand new conversation**, **exit 0**. yggterm sees a clean launch ⇒ *"a new session under the same title pops up"*. This one is silent data loss.

*Fix, in three parts, all descriptor-driven so a tenth CLI is a table row and not a branch:*
1. `store_membership_index` — a KEYED "does the store hold this id" lookup. A CLI without one answers *unknowable*, and unknowable must behave as *yes*: re-birthing because a store could not be READ would destroy live sessions. Only `Some(false)` re-routes a resume, and only for LOCAL rows.
2. `live_session_marker` — how a RUNNING process names its own session, so the row can be bound to the real id. Two measured shapes: `EnclosingDirectory { ".session.lock" }` for muse (⚠ its process holds `cron.db` open for SEVERAL session dirs at once and the lock for only the one it is running — "some open file under the store" is a coin flip), and `FileStem { "presence", "lock" }` for agy. **Binding is refused when two sessions are named**: a wrong id points a row at another row's session, which is worse than no id.
3. On a vouched miss the row is **re-birthed** (no id for a self-minting CLI) and `agent_resume_miss` is traced *before* the CLI runs — the PTY scanner already catches both refusal lines as `session_not_found`, but by then `agy` has created the replacement.

**Fault 4 — RESTATED 2026-08-20; the dedup half was a MISREADING of the spec.** The observation is real: `server app rows` shows the same muse path at depth 1 (`live_rail`) and depth 4 (`row`), switching away tears one, and the next `open` spawns a new PTY. But **the two rows are not a duplicate to be removed.** `AGENTS.md` §*Session Display = Dual Presence*: an active session appears in BOTH Live Sessions and its cwd folder, and *"dedup is per-view, never cross-view"* — the shared `full_path` is the SESSION's identity, and SSOT applies to the session object, not to where it is displayed. ⛔ So `build_local_cwd_tree` cross-view dedup is not the fix; shipping it would be a spec violation, and a `session_id`-keyed skip can only ever express a cross-view rule. Per-view dedup must key on something view-local.

What is left of Fault 4 once the spec is respected is a **PTY-lifecycle** question, not a row-count one — and its "next `open` spawns a NEW session" half was Fault 3 all along: the reopen resumed a phantom id, so the CLI made a new session and the old PTY had nothing to reattach to. Re-measure the tear-on-switch behaviour against the Fault-3 fix before treating any of it as a separate defect.

**What Claude does right (copy this):** `Slug cc` (historical `remote-cc://` / `cc-runtime://`), `binary "claude"`, `resume_selector_token "--resume"`, `store_globs &[".claude/projects/*/*.jsonl"]` (filename IS id, no DB), `TitleAuthority::Store` (`custom-title > ai-title`), `re_roots_with_cwd:false`, `id_assigned_at_birth:true` — yggterm hands CC the row uuid at birth and CC names the transcript after it, so row→store mapping is an identity function. **That is the property the self-minting CLIs lack, and the whole of Fault 3.** Codex hiccups are only viewport (CUF `/x1b[?25l` latching, SIGWINCH nudge) — titles/resume are fine.

### Issue Heading 13: Input latency — keystroke → PTY register → PTY render (flush out latency bugs)

*Contract.* Every keystroke must be traceable end-to-end: `shell` `input/keystroke` (client has the bytes) → `daemon` `input/pty` (PTY `terminals.write(runtime_key, data)` accepted) → `shell` `input/render` (`terminal_write_bridge.stage_or_immediate` staged for xterm). Each hop emits `ytrace` `input/*` (`Wall always`, `session_path`, `data_len`, `is_remote`) so Dash `dash-common-bugs` p4 can compute `pty - keystroke` and `render - pty` p50/p95 per session (like `render/storm` vs `daemon_request/status` 4.65µs/row). A stuck input gate (`remote_resume_input_ready` false, the session-only branch) or a lost PTY write (`terminal_write_error` → `recover_terminal_write_lost_runtime`) shows as `keystroke` without `pty`/`render` — the latency tail, not a screenshot, is the falsifier.

*Probes wired.* `crates/yggterm-core/src/perf.rs: input/keystroke|pty|render` registered `always`; `crates/yggterm-shell/src/shell/viewport.rs:Ok(TerminalJsEvent::Input)` emits `input/keystroke`, `crates/yggterm-server/src/daemon.rs:write_local_terminal_with_lost_runtime_recovery` emits `input/pty`, `crates/yggterm-shell/src/shell/viewport.rs:terminal_write_bridge.stage_or_immediate` emits `input/render`. Use `ytrace tail --category input --since 5m --json | jq 'group_by(.name)'` to flush out bugs where `keystroke` count ≫ `pty` or `render` lags >50 ms.

#### ⛔ THE GUI EVENT LOOP STOPPED READING INPUT WHILE IT WAITED ON THE DAEMON (fixed 2026-08-20)

*The fault it produced: "many CLI sessions have a sudden input freeze".*

`tokio::select!` runs the chosen branch's body to completion before polling any
branch again. The terminal event loop in `yggterm-shell/src/shell/viewport.rs`
has three branches — the JS bridge result, **the JS events (keystrokes)**, and
**the read poll** — and the read poll awaited `terminal_read_async` inline.
While that await was outstanding the keystroke branch was not polled at all, so
the user's typing sat in the JS event channel until the daemon answered.
`TerminalRead` carries the 10 s default client IO timeout, so one slow read could
hold input for up to ten seconds. **Echo was waiting on the read poll, not on its
own PTY write** — and so were resize, clipboard, focus, bell and close.

⛔ **AND THE PROBE FOR THIS SYMPTOM COULD NOT SEE IT.** `input/keystroke` (Issue
13) is emitted at the top of the keystroke handler, i.e. **downstream of the
block**. A real multi-second stall therefore records nothing at all. ⇒ **A zero
reading from `input/*` during a reported freeze is not evidence that the user did
not type**; it is consistent with the loop never having been polled. This is a
blind instrument by construction, not by misconfiguration — the deployed GUI does
carry the probe strings.

**The fix.** The read runs on its own task and its result arrives on a channel,
so typing is serviced while the daemon answers. One read stays outstanding at a
time (depth-1 channel plus an in-flight latch), so cursor advance stays
sequential. The hoist admits an interleaving that could not happen before — six
paths in the JS-event branch reset `cursor` to 0 to force a re-attach, and
applying a stale read's `next_cursor` afterwards would skip past everything the
re-attach meant to replay — so a read issued at a different cursor than the
current one is discarded and re-read (`terminal_read_discarded_stale_cursor`).

**New instrument: `input/loop_block`.** Emitted by whichever branch held the loop
past ~120 ms, with the branch name and the hold, so a stall names its cause
instead of vanishing. Drop-based, because the branch bodies exit from many
places and the unusual slow path is exactly the one an explicit timing call at
the bottom would miss.

**A/B, measured in the under-glass sandbox** — same daemon build, same 6 s
`SIGSTOP` of the daemon, only the GUI differs:

| GUI arm | `input/loop_block` during the stall |
|---|---|
| read awaited inline (before) | **1 event — `branch:read_poll, held_ms:5964`** |
| read dispatched off-loop (after) | **0 events** |

The fixed session came back healthy across the stall (live prompt, intact bottom,
grid unchanged) and recorded zero stale-cursor discards.

⚠ **WHAT THIS DOES NOT FIX, AND DO NOT READ A QUIET LOOP AS A QUIET DAEMON.** The
daemon serves every request under ONE runtime lock, and `TerminalRead` can proxy
synchronously to a preserved-owner daemon while holding it. That is the amplifier
underneath, it is a separate OPEN queue entry, and it is what decides how long the
round trip itself takes once the loop is free.

**Locked by:** `the_daemon_read_is_dispatched_off_the_select_loop` — a
source-level check, because a behavioural test cannot reach this loop and a
re-inlined `.await` would pass every functional assertion while quietly restoring
the stall.

### Issue Heading 14: Agy exemplar — Antigravity faults vs Claude gold (like Muse)

*Why agy.* `antigravity` (`agy` binary, `remote-agy://` / `agy-runtime://`, `agy --conversation <id>`) stores in SQLite `~/.gemini/antigravity-cli/conversations/*.db` + `brain/*/.system_generated/logs/transcript_full.jsonl` + `history.jsonl`, `TitleAuthority::Store` (`conversation_summaries.title` > `preview`). Like `muse`, it writes no title for empty sessions — fresh rows landed `A_ #1557b0` shorthash or generic `antigravity` until the `Muse` fix. Unlike `claude`, its store is a DB, not one file per session, so `read_antigravity_session_title()` must open the DB (not a JSONL tail) and `id` is `conversation_id` (not filename). Faults: (1) shorthash/generic not filtered → now via `titles.rs` bare_hash + `generic_runtime_title` (same `Muse` fix, `ytrace title/*`), (2) ⛔ **CORRECTED 2026-08-21 — the agy title pickup was reading the WRONG HOME and had never once succeeded.** `collect_live_antigravity_title_syncs` resolved its home with `resolve_yggterm_home()` and looked under `~/.yggterm/.gemini/antigravity-cli/…`, which does not exist; measured on the GUI host, **96 consecutive `no_title_in_store` events in 91 minutes for one row** whose title `history.jsonl` had held the whole time. A wrong home is not an error — it is an empty directory, and an empty directory answers "no title" indistinguishably from the truth. The per-CLI arm is gone: ONE registry-driven chore, `daemon.rs:collect_live_store_title_syncs`, calls the descriptor's `read_live_store_title` under `startpage::agent_store_home`, and emits `cli/store_title_miss` (carrying the id it asked about, so a failed identity REBIND is distinguishable from an empty store), (3) resume uses `agy-runtime://` + `agy --conversation` (like `muse` `resume` subcommand, not flag) — the internal id must come from the CLI, never the row UUID, or switch orphans the PTY (same `muse` kick). ⭐ **Where agy's live id actually is, measured 2026-08-20:** at LAUNCH the process holds only the shared index, so a fresh row has nothing to bind to — *the conversation does not exist yet*. Once a turn has happened it holds `presence/<conversation-id>.lock`, and the id is that file's **stem**, not a directory name. `agy -p … --output-format json` also returns `conversation_id` directly, which is the cheap way to confirm the mapping by hand, **(4) `server connect` wiring — `remote-agy://oc/<id>` opened as Codex (`yggterm: saved Codex session <id> is no longer available` measured 2026-08-19 `oc → gh/yggterm` `2cc9f225…` via `server connect remote-agy://oc/…` and cwdtree click): `crates/yggterm-server/src/server_cli.rs:488 connect_session_kind_for_path()` hand-list missed `remote-agy://` in the live `4083ede2` daemon, falling through to `Codex` default, and `crates/yggterm-server/src/lib.rs:11658 remote_saved_agent_session_exists()` checked **local** `~/.gemini/...` (`dirs::home_dir()` on `dev`) for a `remote-agy://oc/<id>` instead of `ssh oc` remote DB — so a DB-only `conversation_id` (`2cc9f225…` no file, only `conversation_summaries.db` row) was seen as missing. Fix: `server_cli.rs:475 parse_remote_scanned_connect_path()` only matched `remote-session://` (Codex) by design, but `server_cli.rs:488` now registry-derived via `yggterm_core::agent_scheme::remote_agent_row_schemes()` (so every `remote-<slug>://` maps to its `SessionKind`), and `lib.rs` dispatches the remote existence check per kind, with `open_stored_session` then launching `agy --conversation <id>` via `managed_cli` `agy-runtime://`. ⛔ **CORRECTED 2026-08-20: `conversation_summaries.db` is NOT the authority and must never be asked as a `no`.** Measured: a conversation the CLI creates gets `brain/<id>/` and `conversations/<id>.db` immediately and is **still absent from `conversation_summaries.db`** afterwards — yet `agy --conversation <id>` resumes it without complaint. A membership check against that table therefore reports live, resumable conversations as missing, and anything acting on that answer (a `--require-existing` gate, or the Fault-3 resume guard) would re-birth over them. The per-conversation artefacts are the authority and they are a **path check, not a query** — the file NAME is the id, exactly as Claude Code's is. The summaries table survives only as an additional *yes*. Verified via `server snapshot` `remote_machines[oc].scanned_sessions` `remote-agy://oc/… cwd ~/gh/yggterm` under `oc/__remote_folder__/gh/yggterm` as `A_ #1557b0` and `server connect remote-agy://oc/2cc9f225…` → live `agy-runtime://` + `pty`**. `claude` gold remains the reference: one file per session, flag `--resume`, filename IS id.

### Issue Heading 15: Codex / codex-litellm exemplar — wiring hiccups vs Claude gold (like Muse)

*Why codex.* `codex` (`codex` binary, `remote-session://` historical `remote-codex://` + `codex-runtime://`, `codex resume <id>`) and `codex-litellm` (`codex-litellm` binary, local-only `codex-litellm://`, `id_assigned_at_birth` same) are *workable* per matrix (`TitleAuthority::Generated` via `SessionTitleStore` heuristic/litellm, store `~/.codex/sessions/**/rollout-*.jsonl` id inside file, `re_roots_with_cwd:true` for `codex`, `false` for `litellm`). Their faults are viewport, not title/resume: (1) **Geometry squish** — daemon re-creates PTY at default `120×36` after hot-update re-resume, `last_sent_terminal_resize_*` is stale-equal to live grid, so no `Resize` fires and `codex` renders squished; fix is `viewport.rs:9837` re-resume squish repair (`last_sent_* = 0` + `spawn_terminal_startup_resize_repair`) now emits `ytrace cli/codex_geometry` (`stale_cols/rows`, `live_cols/rows`, `kind: codex_squish_repair`) for Dash. (2) **Differential CUF spaces** — `CUF` cursor-forward skips leave stale `bg` artifacts; mitigated by full screen replay on reveal (same `muse`/`codex` path, `ytrace` `terminal_mount` already). (3) **Middle desync on rapid switch** — `codex` status bar truncation if `SIGWINCH` not nudged; `terminal_write_should_frame_budget` / `terminal_write_bridge` already gates, now `ytrace` `cli/codex_geometry` covers it. `claude` gold has no squish because its Ink engine re-anchors absolute `CUP` on switch; `codex` needs the explicit resize repair — that is the hiccup to copy for any TUI that expects full width.

**Checklist for any new CLI (add to `spec-adding-an-agent-cli.md` steps 1–9):** 1) `SessionKind` variant, 2) `AGENT_CLIS` descriptor (+ `TitleAuthority`, `store_globs`, `id_assigned_at_birth`, `resume_selector_token`, `re_roots_with_cwd`), 3) `SESSION_PATH_SCHEMES` (`remote-<slug>://` + `<slug>-runtime://`), 4) `cargo check` exhaustive matches, 5) catch-alls `rg SessionKind::(Codex|ClaudeCode)`, 6) `agent_arm_matrix` two arms (Local `local://` + Remote `remote-<slug>://`), 7) surfaces (icon/menu/KeyTips free), 8) provisioning `install`/`update`, 9) **title lifecycle** — the birth name is automatic (`New {machine} {display_name}`, from `new_session_birth_title`; nothing per-CLI to add), then either `heuristic`/`litellm` via `SessionTitleStore` for a `Generated` CLI + its fallback list, or `read_live_store_title` for a `Store` one — ⛔ a `Store` CLI without that hook can never be titled at all, 10) **resume id** (if `id_assigned_at_birth:false`, implement store→row mapping), 11) `spec-cli-integration-verification.md` oracles (`check-startpage.py`/`check-titles.py`/`check-cwdtree.py` must `0` on every fleet host + faithful 1920×1200 screenshot).

## 3. Inventory — which spec/doc now lives where

* `spec-cli-integration-verification.md` — the **harness** (verb + oracle pattern, `AGENT_CLIS` SSOT, adding a CLI is one descriptor).
* `spec-adding-an-agent-cli.md` — the **procedure** for a new CLI (10 recon questions, descriptor fields, rolling-upgrade hazard).
* **This file** — the **BUGS & 9-CLI PROTOCOL** matrix (what is promised vs what is delivered for each of the 10 CLIs).
* `pending-bugs.md:CLI` — pointer to this file (open) plus the `6.7` tmpfs/swap leak.




---

## Scanned, not gapped — what empty `session_store_globs` actually means

**Corrected 2026-08-20.** This file recorded OpenCode and Kimi as
*declared-unscannable*, `store_scan_gap: true`, "Gap by design". None of that was
true of the shipped code, and had not been for some time:

* every descriptor in `AGENT_CLIS` carries `store_scan_gap: None`;
* `scan_all_durable_sessions` dispatches OpenCode to `scan_opencode_sessions`
  (one SQLite `session` table) and Kimi to `scan_kimi_sessions` (md5 buckets
  reversed through `kimi.json` `work_dirs[].path`), and both return rows.

The reason it read as a gap is that all three `ls` verbs printed

> `OpenCode has no store globs and no declared gap — sessions will be invisible`

on **every single run**, because they asked only whether a descriptor has globs.
A store that is one DB file, or a tree bucketed by a hash of the cwd, cannot be
expressed as a glob — that is why these two have a hand-written scanner instead,
not evidence that they are unscanned.

⇒ The question has one owner now:
`yggterm_core::startpage::kind_has_dedicated_scanner`. The scanner dispatch, the
three warnings and `every_agent_cli_declares_a_store` all ask it, so a CLI cannot
be scanned and advertised as invisible at the same time. A CLI with no globs must
have a dedicated scanner **or** a declared gap; `a_scanned_cli_is_never_reported_invisible`
fails the build otherwise.

⚠ The general shape, worth carrying: **a warning that fires on every run stops
being read.** Both of these had been printing for long enough that a whole doc
section was written around them as though they described reality.

## The agy durable rule — why 999 conversations are 4 sessions

`conversation_summaries.db` is an INDEX of everything the CLI has ever done, not
a list of sessions to resume. Measured 2026-08-20 on a 999-row store:

| class | rows | shape |
|---|---|---|
| batch, mixed workspace | 499 | a real repo root **plus** an ephemeral scratch dir; one burst, `step_count` 6 |
| batch, scratch-only workspace | 494 | scratch roots only — these became the "/tmp forest" of one-session cwd-tree groups |
| real sessions | 6 | real roots only; 4 hold steps, 2 are empty shells |

⛔ **Do not reach for the columns that look built for this.** `source`, `status`,
`agent_name`, `nesting_depth`, `parent_conversation_id`, `battle_id`,
`not_fully_idle` and `last_user_input_step_index` were uniformly empty or default
across all 999 rows, and **`killed` was 0 for every row** — so the scan's
`WHERE killed=0`, which reads like a guard, filters nothing. Only `step_count`
and `workspace_uris` carry signal.

⇒ `antigravity_row_is_durable`: a workspace exists, none of its roots is
ephemeral scratch, and `step_count > 0`. 999 → 4, matching the `4` this file had
already recorded from an independent count.

⚠ **The rule reads the PATH, not the filesystem.** "Does the workspace dir still
exist" is the tempting version and it measured worse — two batch conversations
still had live scratch dirs and survived it — besides making the scan
non-deterministic as temp dirs are reaped.

⚠ **The DB is authoritative for BOTH halves.** The durable verdict gates the file
walk as well, or a filtered batch conversation returns through its brain
transcript. That was not hypothetical: the descriptor's glob said
`transcript.jsonl` while all 497 files on disk are `transcript_full.jsonl`, so
the file half of the agy scan had been dead while reading as fully wired.

Per-row recency: each row's own `last_modified_time` is parsed
(`parse_antigravity_last_modified_ms`). It is ISO-8601 with a **space**
separator, so it needs converting before RFC-3339 will accept it, and
never-written rows carry `0001-01-01` and clamp to 0. Every row used to be
stamped with the DB **file's** mtime — one shared fake recency for the whole
store, which moved every time the CLI touched it.
