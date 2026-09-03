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
* ⭐ **The recency contract (2026-09-02, owner falsifier: "the most recent sessions in dev should be some opencode sessions from ~/gh/yggterm"):** RECENT WORK's own header says "most recently used first", so recency is the whole ranking law — `scope > recency > started_at` (`order_candidates_for_startpage`). ⛔ `is_live` is NOT a tier, and no surface may stamp a row's `modified_epoch_ms` with the scan time: the two together let a row idle for days claim "used one second ago" at every scan tick and permanently bury the durable rows whose store mtimes held the truth (62 rows measured sharing one scan millisecond). A live row's truthful recency is the daemon's own `last_activity_epoch_ms` (PTY idle clock, on the snapshot's live rows), else its store epoch; unknown is 0 and ranks honestly last. The gauge is `scripts/check-fs-truth.py` — an independent Python store walk that fails on any claimed recency newer than the store/daemon fact, on scan-stamp collapse, and on ordering inversions.
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
* **Restore invariant (2026-08-24):** `restored_live_row_key` must return the same normalized per-CLI scheme that `restore_live_session` inserts. Rebuilding a generic parsed remote key through the historical `remote_scanned_session_path` constructor silently turns every scheme into `remote-session://` during the pre-restore identity, tombstone, and active-path passes. `restore_live_session_preserves_every_registered_remote_agent_scheme` iterates `AGENT_CLIS` and locks both sides of that relation.

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

**What Claude does right (copy this):** `Slug cc` (historical `remote-cc://` / `cc-runtime://`), `binary "claude"`, `resume_selector_token "--resume"`, `store_globs &[".claude/projects/*/*.jsonl"]` (filename IS id, no DB), `TitleAuthority::Store` (`custom-title > ai-title`), `re_roots_with_cwd:false`, `id_assigned_at_birth:true` — yggterm hands CC the row uuid at birth and CC names the transcript after it, so row→store mapping is an identity function. **That is the property the self-minting CLIs lack, and the whole of Fault 3.**

**⛔ CORRECTED 2026-08-22 — Codex titles/resume were not fine.** Two live
remote Codex rows on the live GUI host still carried version-4 birth UUIDs; neither id
existed among the remote machine's 798 scanned durable sessions. The remote
identity verb successfully found three running Codex identities, but exact cwd
matching found zero for both rows: the rows were launched in a worktree while
Codex's transcripts retained another checkout cwd. After twelve polls the join
silently stopped. The host-resident daemon already held the exact relation:
each owned Codex runtime had a version-7 real `Codex Session`, a `Storage`
path, and the original version-4 id preserved as `UUID`. The remote identity
wire now exports that owner-reported `birth_session_id`; the GUI-host poll joins
on it first and uses cwd only as a compatibility fallback. `cli/identity_poll`
states discovery, exact-alias candidates, cwd candidates, rebinds and exhausted
rows, while `cli/projection` catches the resulting birth placeholder at the
last GUI stage. This is the remote twin of Fault 3, and it also prevents a cold
restore from resuming the phantom version-4 id with `--require-existing`.

The first live projection read exposed an observer defect too: eleven active
agent paths were each reported twice with `presence:"live_rail"` (seven Claude
Code, two Codex, two Antigravity). The product rows were the required rail + cwd
dual presence; `server app rows` had guessed presence from a path set, so the
cwd occurrence inherited the rail label and alternated authoritative/inferred
trace edges for one key. Presence now comes from the concrete row index inside
the Live Sessions region. Per-view uniqueness is audited by
`(full_path,presence)`, never by session identity alone.

After that witness correction, the live all-registry sweep reported zero icon
mismatches and named the remaining title debt without exposing title text: six
Codex short hashes, one Muse short hash, and one low-signal Claude Code row.
Antigravity and Grok Build were clean in the projected sample. Codex LiteLLM,
Kimi, OpenCode, Pi, and Qwen Code had zero projected rows, which is stated as
absence rather than misreported as success; those CLIs still need an active
birth/launch/restore specimen before their integration can be graded.

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

*Why codex.* `codex` (`codex` binary, `remote-session://` historical `remote-codex://` + `codex-runtime://`, `codex resume <id>`) and `codex-litellm` (`codex-litellm` binary, local-only `codex-litellm://`, `id_assigned_at_birth` same) are *workable* per matrix (`TitleAuthority::Generated` via `SessionTitleStore` heuristic/litellm, store `~/.codex/sessions/**/rollout-*.jsonl` id inside file, `re_roots_with_cwd:true` for `codex`, `false` for `litellm`). Their measured faults are: (1) **Remote identity/title/resume drift** — the GUI-host wrapper is born with a version-4 UUID while the target-host daemon discovers Codex's version-7 transcript id. Joining those rows by transcript cwd fails for worktrees, leaving the birth title and a cold-restore command aimed at a nonexistent id. The target daemon's `UUID` → `Codex Session` pair is authoritative; `local-codex-identities` now carries it as `birth_session_id`, the poll matches it before cwd, and `cli/identity_poll` + `cli/projection` make every failed edge visible. (2) **Geometry squish** — daemon re-creates PTY at default `120×36` after hot-update re-resume, `last_sent_terminal_resize_*` is stale-equal to live grid, so no `Resize` fires and `codex` renders squished; fix is `viewport.rs:9837` re-resume squish repair (`last_sent_* = 0` + `spawn_terminal_startup_resize_repair`) now emits `ytrace cli/codex_geometry` (`stale_cols/rows`, `live_cols/rows`, `kind: codex_squish_repair`) for Dash. (3) **Differential CUF spaces** — `CUF` cursor-forward skips leave stale `bg` artifacts; mitigated by full screen replay on reveal (same `muse`/`codex` path, `ytrace` `terminal_mount` already). (4) **Middle desync on rapid switch** — `codex` status bar truncation if `SIGWINCH` not nudged; `terminal_write_should_frame_budget` / `terminal_write_bridge` already gates, now `ytrace` `cli/codex_geometry` covers it. `claude` gold has no identity indirection and its Ink engine re-anchors absolute `CUP` on switch.

### Issue Heading 16: Self-minting identity must be fleet-wide, and title copy must follow it

**Measured failure (2026-08-24):** Codex, Muse, and Antigravity all launch before
their CLI-owned durable id exists. The target daemon learned the real id from
the process marker, but the `local-codex-identities` compatibility wire exported
the owning daemon's birth-id alias only for Codex. The GUI daemon also selected
only Codex rows for polling. Muse and Antigravity therefore kept a wrapper birth
UUID as store lookup id, while their durable scanner produced a second row under
the real id. The symptoms were raw-path or generic live titles, short-hash
durable titles, and rows that disappeared after a daemon transition.

**Rule:** `AgentCliDescriptor::id_assigned_at_birth == false` plus a measured
identity source is the policy; no self-minting CLI gets a private identity
lifecycle. The owning daemon overlays the real id in its snapshot, exports the
exact `(kind, birth_id, real_id)` relation, and refreshes it on the bounded
background chore because a marker may appear after the launch lifecycle pass.
The GUI host polls each machine once, joins by `(kind, birth_id)`, persists the
logical id, queries the store using that logical id, and targets generated copy
at the existing birth-path live wrapper. Cwd fallback remains Codex-only for
rolling compatibility; Muse and Antigravity must never be paired by a default
cwd their marker cannot prove. A same-id store artifact is not proof that the
birth wrapper is correctly bound: the bounded `/proc` marker check remains
periodic and idempotent, because both Muse and Antigravity can create a phantom
birth-id artifact before exposing their real CLI-owned id.

Muse's `session-index.db` is title/cwd metadata, not transcript-existence truth.
In particular, measured sessions retained `prompt_count = 0` and `New session`
after real accepted user intents appeared hundreds of lifecycle records into a
multi-megabyte JSONL. A zero counter may suppress only a transcript that itself
contains no `runtime.user_intent.accepted`; title extraction searches beyond the
startup prelude. The DB consulted by a scan is derived from that transcript's
store root, never the scanning process's HOME.

**Title coexistence:** `TitleAuthority::Store` still wins. Claude Code continues
to read `custom-title` then `ai-title` from its JSONL and is never overwritten by
Interface-LLM generation. Generated-authority CLIs get built-in Interface-LLM
rescue by default; the historical background-copy environment variable is an
explicit off switch, not the feature gate.

**Observability:** `cli/local_identity_bind` records only path, kind, and id
origin. Remote `cli/identity_poll` is emitted per CLI kind and separates exact
alias candidates from the Codex compatibility cwd candidates. Title probes
record title quality and presence, never title text.

**Not covered:** this does not change a CLI's launch or resume syntax, infer an
id for a CLI without a measured marker/transcript source, parse transcripts into
the terminal viewport, or alter Claude Code's title mechanism. Rendering quirks
and daemon PTY handoff safety remain their existing per-CLI and daemon contracts.

### Issue Heading 17: A blocked restart must not detach the whole CLI fleet

**Measured failure (2026-08-24):** a Codex child entered Linux uninterruptible
sleep while the daemon handled `terminal_restart`. Restart held the global
runtime lock, sent SIGKILL, and then waited for exit without a deadline. The
request held the lock for 119.85 seconds; during that interval Codex, Muse, and
Antigravity attach/ensure requests repeatedly hit their client deadlines and
their rows appeared detached. A separate unbounded `codex --version` child also
stopped the shared background identity/title chore.

**Rule:** restart teardown may spend the existing graceful signal window, but
after SIGKILL it gets only the bounded force-exit deadline. If the kernel still
reports the child alive, the old runtime is put back in its seat and restart is
refused; yggterm must not spawn a second writer. Managed-CLI version probes are
metadata and have a two-second ceiling. A timed-out child is killed and handed
to a detached reaper so waiting on a D-state process cannot stall the chore.

The same preservation rule applies at daemon bind. A predecessor may leave the
successor's versioned request name as a compatibility symlink back to itself.
The successor may unlink that symlink and bind its own name only when the
preserved-owner registry both contains runtimes and explicitly names the
successor's version. Otherwise an answering alias remains an owner and is never
taken. `cli/attachment` records the authorized alias release.

`server update-daemons --force` must also bypass the same-version short circuit.
A semver identifies a release, not a particular dirty/rebuilt inode; deployment
of a fixed daemon under the current semver is exactly why the force flag exists.

**Observability:** `cli/attachment_sweep` reports running, preserved,
exited-runtime, missing-runtime, unbound-presence, and not-expected counts for
every `AGENT_CLIS` descriptor, including Claude as the control. A projected
remote row is not expected on the GUI-host daemon; a legacy/birth `local://`
row with no owner is an unbound presence anomaly, not proof that this process
dropped a PTY. `cli/attachment` records a bounded restart that left the live
runtime seated. `cli/version_probe` records binary name, outcome, elapsed time,
and ceiling. `cli/runtime_conflict` records the Codex active-writer refusal
without copying terminal text or launch commands into ytrace.

**Not covered:** these safeguards do not terminate an externally launched CLI,
take over a CLI's own single-writer lock, declare an observer authoritative, or
change Claude Code's store-owned title and resume behavior.

### Issue Heading 18: Runtime-key aliases and rowless PTYs are one handoff invariant

**Measured failure (2026-08-25):** a version handoff transferred all eleven PTY
descriptors successfully, but the successor appeared to preserve only seven.
The other four were still alive and readable. Muse and Antigravity rows were
keyed `local://<birth>` while their terminal seats were keyed by the descriptor's
`muse-runtime://<birth>` / `agy-runtime://<birth>` schemes, and the resolver
tested only the Codex-shaped alias. Two additional PTYs had no managed row at
all after earlier failed resolver/restart cycles. The result was an interactive
process with no addressable sidebar presence, plus duplicate resumes when a row
opened the empty spelling and launched a second writer.

**Rule:** a live agent row represents both its row key and every runtime alias
derived from its `AgentCliDescriptor`, using the birth id before the rebound CLI
id. The terminal manager is authoritative about which spelling is actually
seated. Read, write, resize, close, identity refresh, preservation, and
attachment audit must all use that corrected resolver. No CLI gets a private
alias branch.

PTY handoff and row persistence are independent channels. Immediately after a
successor adopts a descriptor, it checks whether any managed row represents the
runtime key. If not, it reconstructs an agent row from the registered runtime
scheme or an exact CLI executable token in the preserved launch command, the
live process marker when available, and the preserved cwd. This is presence
recovery only: it never launches or restarts the adopted process. Plain shells
are not promoted. The temporary `untitled session` label stays under normal
store/Interface-LLM title authority.

**Observability:** `cli/orphan_runtime_row_recovered` is content-free and records
kind, runtime scheme, and identity origin. The lifecycle trace also records the
runtime and recovered row keys for handoff forensics. A handoff is green only
when every owned agent runtime is represented after adoption, not merely when
every descriptor crossed the socket.

**Not covered:** this does not merge two already-running PTYs that point at the
same CLI conversation, kill either writer, infer a plain `local://` shell as an
agent without an exact executable token, or change Claude Code's store-owned
title mechanism.

### Issue Heading 19: Remote title readers and handoff classifiers must preserve CLI identity

**Measured failure (2026-08-25):** two durable Muse sessions had accepted user
intents and usable local titles, but their remote wrappers stayed named as raw
cwd paths. The ssh-side Python reader stopped after 65 lifecycle records while
the local Rust reader scanned until the first accepted intent. In the same
handoff, an Antigravity PTY was reconstructed as Pi because the launch-command
classifier split `/home/user/pi/...` into the registered executable word `pi` before
it reached the actual `agy` command. Exact launch-word parsing removed that
path bug but did not repair the live row: preserved launch metadata could still
describe an older wrapper while the adopted PTY's root process was already
`agy`.

**Rule:** a remote store reader answers the same title question as its local
reader. Muse scans until its first accepted/materialized user intent; startup
record count is not an identity or title boundary. Agent inference from a
preserved launch command recognizes exact command words in command order and
does not tokenize path components into executable names. For an adopted generic
`local://` runtime, the live process tree's exact `argv[0]` basename outranks
preserved launch text; the descriptor-owned runtime scheme still outranks both.

If an older daemon already synthesized the wrong kind, the successor may
reclassify only a non-explicit generic placeholder whose runtime marker has the
same birth id. The row is re-keyed to the correct descriptor scheme and its
stale CLI metadata label is removed. The repair does not depend on a transient
update-restore marker, because the corrupted row can persist after that marker
is gone. Owner-titled and already-usable-title rows are never rewritten.

**Observability:** `cli/orphan_runtime_row_recovered` carries `row_repair` as
`recovered` or `reclassified`, plus kind, runtime scheme, kind origin
(`runtime_scheme`, `live_process`, or `launch_word`), and id origin. Title pickup
remains content-free and reports only the kind, match, and quality.

**Not covered:** this does not infer an agent from a path-only shell command or
from arbitrary process arguments, rewrite owner-set titles, merge multiple live
writers, or alter Claude Code's store-owned title behavior.

### Issue Heading 20: A remote-store miss is not title confirmation

**Measured failure (2026-08-25):** a remote Muse wrapper was first queried with
its yggterm birth UUID, before Muse exposed the real session UUID. The store
correctly returned no title for that birth UUID, but the daemon inserted the
`(wrapper path, UUID)` lookup into its confirmed set anyway. After identity
rebind, the remote probe could return the title for the real UUID, yet the idle
row was already classified `skipped_title_settled` and kept its raw cwd path.

**Rule:** only a positive remote-store lookup is confirmation. `no_title_in_store`
leaves the row eligible for a later bounded idle retry; the existing chore
backoff limits the ssh cost. Confirmation remains keyed by both wrapper path and
logical session id, so a birth-id miss can never suppress the first real-id
lookup. Store agreement with the current row is positive and may settle it.

**Observability:** the negative `cli/title` outcome carries
`retry:"unconfirmed_until_store_title"`; a later positive lookup emits the
normal content-free `picked_up` or `skipped_title_settled` outcome. The trace
never contains the title text.

**Not covered:** this does not invent a title when a CLI has no store signal,
overwrite an owner-set title, change Claude Code's title authority, or make a
store miss an error.

### Issue Heading 21: Title writers must resolve runtime-key aliases too

**Measured failure (2026-08-25):** the local Antigravity reader repeatedly
found a usable store title and emitted `picked_up`, while every background tick
reported `updates:1, applied:0`. The managed row was stored under its PTY birth
key (`local://<birth>`) but exposed the descriptor-owned row path
(`agy-runtime://<birth>`). Read, screen, and ownership paths resolved the alias;
`set_session_title_hint` indexed the sessions map by the exposed spelling and
silently found nothing.

**Rule:** derived, passive, and explicit title writers, owner-title checks, and
summary writers resolve the same row aliases as terminal/session readers before
mutating state. The map key remains the PTY seat and the row path remains CLI
identity; neither is rewritten merely to make a title land.

**Observability:** a proposed title that does not land emits content-free
`cli/title_apply_refused` with `row_resolved` and `owner_titled`. The existing
`background_copy/tick` `updates` versus `applied` counts remain the aggregate
oracle. Repeated `updates>0, applied=0` is a writer defect, not progress.

**Not covered:** alias resolution does not merge duplicate rows, infer a CLI
kind, re-key a PTY, override an explicit title, or change title precedence.

### Issue Heading 22: Classify Muse after condensing its raw prompt

**Measured failure (2026-08-25):** after the retry and real-id fixes, ytrace
proved a remote Muse row was queried repeatedly under its correct UUID but
still returned `no_title_in_store`. The ssh probe actually returned two
candidates. Both began with polite prompt copy (`Please ...`), and the remote
chooser rejected the raw sentence as a low-signal finished title before calling
the condenser. The local durable reader condensed first and produced a usable
title from the same transcript.

**Rule:** Muse store candidates are title input, not necessarily titles.
Condense each raw candidate first, then apply fallback/low-signal validation to
the condensed result. Local and remote readers therefore answer the same
question in the same order.

**Observability:** remote `no_title_in_store` includes `candidate_count` and
`probe_line_count`, never candidate text. Zero candidates means store absence;
a positive count means the shared chooser rejected what the probe returned.

**Not covered:** this does not accept raw prompt prose as the rendered title,
weaken title-quality checks for other CLIs, or override Muse's store authority.

### Issue Heading 23: Startpage observers must never start or hand off a daemon

**Measured failure (2026-08-25):** `server startpage ls` took 80.9 seconds on
the GUI host while `cwdtree ls` completed inside the 45-second oracle budget.
The durable scan was shared. The extra time came from three nested shell
commands used as GUI witnesses; `server snapshot` alone spent about 25 seconds
trying to make a daemon reachable. A read-only Startpage diagnostic could
therefore enter daemon startup/handoff machinery and perturb the PTY owner it
was supposed only to observe.

**Rule:** Startpage reads the resolved daemon endpoint in-process, as CwdTree
and Titles do, and asks the already-running GUI directly through bounded
read-only app-control requests. An absent/busy GUI falls back to store truth in
at most one second. It never searches `PATH`, starts a child yggterm binary, or
calls daemon readiness/startup from the faithful observer path.

**Observability:** `cli/startpage_observers/faithful_read` records only browser
row count, daemon-snapshot and app-state availability, elapsed milliseconds,
and the app-control ceiling. It carries no row paths, ids, cwd, or titles.

**Not covered:** this does not make GUI state authoritative, change Startpage
ordering/scoping, increase the independent oracle timeout, or hide a slow
durable-store scanner.

### Issue Heading 24: Durable rows need conversation signal, not merely a file and UUID

**Measured failure (2026-08-25):** after every live non-Claude row was attached
and titled, the GUI still exposed nine dormant CwdTree rows as eight-character
hashes: startup-only Codex rollouts, a lifecycle-only Muse transcript, a Muse
transcript whose first accepted envelope was low-signal but whose next envelope
held the task, an Antigravity conversation whose per-conversation DB was walked
before its title-bearing brain transcript, and one real Codex conversation for
which the configured generator returned an unshowable title.

**Rule:** a startup-only transcript is scan noise and is skipped in memory,
never removed. For an otherwise title-less Codex row, the shared transcript
reader must find generation context; for Muse, the transcript must contain an
accepted user intent. Muse title extraction examines every accepted envelope,
message content item, and refill block until one condenses to a usable title.
Antigravity projections with the same conversation id are merged: an existing
store title keeps precedence, while a title-bearing transcript fills an absent
title instead of losing to filesystem walk order.

**Observability:** `cli/projection` remains the content-free rendered oracle:
`short_hash`, `raw_path`, and generic-title counts must all be zero. The three
independent Startpage, Titles, and CwdTree oracles still prove store parity; a
noise row disappearing from all three is expected only when its source contains
no conversation signal.

**Not covered:** this does not delete or quarantine CLI store files, infer a
title from startup instructions, replace an owner/store title, alter Claude
Code authority, or hide a transcript that contains accepted user work merely
because title generation is temporarily unavailable.

### Issue Heading 25: Fleet CwdTree consumes the core durable projection over SSH

**Measured failure (2026-08-26):** on the same current binary, `server titles
ls` omitted startup-only Codex and Muse files while `server cwdtree ls` still
returned them. The latter appended a daemon remote-machine snapshot populated
by one hand-written Python scanner per CLI. Those transport scanners were a
second implementation of store identity, title rejection, and conversation
durability; Muse stopped at the wrong envelope, Pi emitted headers without
dialogue, and Codex kept metadata-only rollouts. Restarting the GUI could not
repair the disagreement because it faithfully reloaded the wrong remote
projection.

**Rule:** current peers call `server remote durable-sessions`, which serializes
`scan_all_durable_sessions(agent_store_home(...))` rows without re-deriving
their identity, cwd, title, kind, or durable verdict. The historical per-CLI
SSH scripts remain only as a rolling-upgrade compatibility fallback when an
older peer does not recognize the verb. Claude Code still keeps its native
store-title authority; it merely crosses the same transport as every other
core row.

**Observability:** every applied remote refresh emits content-free
`server/remote_machine/durable_projection_source` with `source: core_ssot` or
`legacy_compat`, machine key, and row count. The remote producer emits
`remote/durable_scan/complete` with aggregate counts by kind. No session id,
cwd, path, title, or transcript text is recorded by either probe.

**Not covered:** this does not remove rolling-upgrade compatibility, make an
observer authoritative over a CLI store, change resume commands or PTY
ownership, copy transcripts between machines, or allow an empty/partial parse
to replace a previously healthy remote snapshot.

**Checklist for any new CLI (add to `spec-adding-an-agent-cli.md` steps 1–9):** 1) `SessionKind` variant, 2) `AGENT_CLIS` descriptor (+ `TitleAuthority`, `store_globs`, `id_assigned_at_birth`, `resume_selector_token`, `re_roots_with_cwd`), 3) `SESSION_PATH_SCHEMES` (`remote-<slug>://` + `<slug>-runtime://`), 4) `cargo check` exhaustive matches, 5) catch-alls `rg SessionKind::(Codex|ClaudeCode)`, 6) `agent_arm_matrix` two arms (Local `local://` + Remote `remote-<slug>://`), 7) surfaces (icon/menu/KeyTips free), 8) provisioning `install`/`update`, 9) **title lifecycle** — the birth name is automatic (`New {machine} {display_name}`, from `new_session_birth_title`; nothing per-CLI to add), then either `heuristic`/`litellm` via `SessionTitleStore` for a `Generated` CLI + its fallback list, or `read_live_store_title` for a `Store` one — ⛔ a `Store` CLI without that hook can never be titled at all, 10) **resume id** (if `id_assigned_at_birth:false`, implement store→row mapping), 11) `spec-cli-integration-verification.md` oracles (`check-startpage.py`/`check-titles.py`/`check-cwdtree.py` must `0` on every fleet host + faithful 1920×1200 screenshot).

### Issue Heading 26: OpenCode2 owns a row system of its own — the tab bar must mirror into ours

**Owner directive 2026-08-28.** Every other registered CLI satisfies one law this
file has been built on since Issue 3: **one PTY = one process = one session = one
row.** OpenCode2 (the v2 preview, `@opencode-ai/cli`, bin `opencode2` — §11
register) breaks it, and it is the only CLI that does:

* the row's PTY hosts the TUI **client**; a shared background **service** owns
  the sessions;
* the TUI renders **N open session tabs** on the current cwd — a tab bar with
  `+ New session`, per-tab close (×), and `session.tab.next/previous/close/
  reopen/select.N` keybinds;
* so one yggterm row is **1 : N** opencode sessions. OpenCode has its own row
  system, and ours has never heard of it.

**Why this must integrate, and why it blocks the fleet.** The yggterm row is the
addressing primitive for everything the fleet does — booter, monitor, notify
cards, `terminal submit`, cross-row messaging, the context gauge, and the
orchestrator/relay/sub-session workflows. With N sessions hidden behind one row:

* a PTY write (submit, send, a boot, a nudge) lands in **whichever tab the human
  has focused** — a message addressed to one session can enter a stranger's
  composer, which is the wrong-row wake hazard without even needing a wrong row;
* per-session liveness, context budget and title are invisible; the footer read
  answers for the focused tab only;
* a tab spawned from `+ New session` appears nowhere in Live Sessions; a tab
  closed with × leaves its row untouched.

**The contract (owner-specified, verbatim intent):**

1. **Every open opencode2 tab ↔ one yggterm Live Sessions row.** Each session
   becomes addressable, ownable and claimable like any agent row.
2. **Tab spawn → row spawn immediately below the TUI's last tab row.** A new
   session seats itself as a contiguous block directly under its opencode
   anchor, in tab order — exactly where the human sees it appear in the tab bar.
3. **Tab switch → row switch.** Moving between tabs moves yggterm's active row
   with it, both directions: focusing a row in yggterm focuses that tab in the
   TUI.
4. **Tab close → row despawn.** ⚠ The TUI has `session.tab.reopen` — a closed
   tab is recoverable upstream, so the row removal is a hide/retire, **never** a
   durable tombstone of the session.
5. **TUI row close** takes the anchor and its tab rows with it.

**Mechanism sketch — the v2 service API already exposes every primitive:**
`GET /api/session/active` (the open tabs), `GET /api/event` (SSE lifecycle
stream: spawn/close/focus), `POST /api/session/{id}/prompt` (per-session
delivery with steer/queue inbox semantics — the same contract as §4's "a busy
row queues your message"), `POST /api/session/{id}/rename` (title sync),
`DELETE /api/session/{id}` (explicit delete only), `GET /api/session/{id}/context`
(per-session context gauge feed). Drive it through `opencode2 api` — the same
discovery and auth flow the TUI uses — rather than hand-rolled socket discovery.
Seat tab rows as sub-seats under the anchor row (`N.x.y`), per the standing
row-hygiene scheme.

**Two identity notes.** (a) The tab rows are a **projection** of the opencode
service's session list, marked as such — the same relationship a durable scan
row has to its store. The service is authoritative for its sessions; yggterm
rows stay yggterm's truth for presence; the mirror must not become a second
row source of truth. (b) opencode2 also has **child sessions** (`session.child.*`
/ `session.parent`, and fork) — a sub-agent primitive, not a tab. Tabs are peer
sessions on one cwd; children are nested work. The row mirror is for tabs; child
sessions are the natural hook for the later sub-session orchestrator workflows
and must not be conflated with them.

**Until this ships (standing rule):** opencode2 rows are **excluded from fleet
orchestration** — an opencode session works like a normal session (no relay,
no booter claim, no monitor subscription), because the primitive those tools
address does not exist per-session yet. No verb may assume a PTY write reaches
a specific opencode session; per-session addressing goes through the service
API or does not happen.

**Not covered:** this does not change resume selectors or JSONL delegation, does
not parse opencode transcripts into the viewport, does not promote the opencode
service to an observer of yggterm row state, and does not ask yggterm to render
the tab bar itself — the TUI keeps its chrome; we mirror presence, not pixels.

### Issue Heading 27: opencode2 self-mints `ses_…` ids — a store probe false-deathed a live row (gate fixed; rebind owed)

**Owner-caught live 2026-08-28**, minutes after Issue 26 was written. After a
routine daemon hot-restart, switching to a live remote opencode2 row never
restored it: the viewport kept stale pixels, the "Resuming Remote Terminal"
toast never cleared, and three switch attempts changed nothing. The agent
inside the row kept working the whole time — the owner routed around his own
row through a plain shell row.

**Root cause (two halves).** (1) *Identity:* the v2 preview ignores yggterm's
`--session <row-uuid>` and mints its own `ses_…` ids in its SQLite store — the
row uuid is never stored, so every store-keyed half answers about a phantom.
(2) *Gate order:* the restore path probed the store BEFORE attempting the
resume; the probe answered "absent", the row was marked
`Saved Session: missing`, and a sticky mark refused every later launch. The
resume wrapper's own live-runtime arm — which bridges the held PTY and never
needs the transcript — would have succeeded; it was never allowed to run. The
sweep's own trace field said `live_runtime_truth_beats_transcript_metadata`
while the code let transcript truth kill the live row: the policy and the
branch disagreed, and the trace was the only honest one.

**Landed (`19b53996`):** a daemon-held live runtime row now outranks the store
probe wherever `--require-existing` is enforced probe-first
(`remote_require_existing_refusal_allowed` + the sweep skip/retract), and a
held row re-attaches through the resume command — never the resume picker,
whose keystrokes would drive a running TUI. Regression lock:
`a_live_runtime_row_cannot_be_refused_on_a_store_absence`.

**Tab mirror LANDED (`df977647d`):** a daemon chore (5 s, service IO outside
the daemon lock) keeps one yggterm row per OPEN opencode2 tab — keyed
`opencode-runtime://<ses_id>`, seeded through ensure with the launch line
`opencode2 --session <ses_id>` (no PTY until opened), seated directly below
the opencode TUI anchor row, titled from the service's session title.
Switching tabs in the TUI moves the active row (focus-follow on per-session
`time.viewed`, gated to fire only while the opencode context is already
active, so it never yanks the viewport); a closed tab retires its row —
unless the user opened it, in which case it is a window and closes only when
the user closes it. The mirror touches only rows it created (`Source:
opencode-tab-mirror`). Multi-client justification measured: a second
`opencode2 --session` on a session another TUI had open painted the same
conversation with no conflict — the service owns state, windows are clients.

**TAB MIRROR LANDED + LIVE (`df977647d` core, `f84ffd0f` silent insert,
`d41b6c1f` title sync, `d2f445f48` adoption, `5fc4b712` unlimited adoption;
live-verified 2026-08-29 21:54):** a 5 s daemon chore keeps one row per open
opencode2 tab — keyed `opencode-runtime://<ses_id>`, born Queued with the
launch line `opencode2 --session <ses_id>` (no PTY until opened), seated
under the anchor, titled from the service and re-synced every tick.
Focus-follow (active row tracks `time.viewed`, gated to the opencode context)
verified firing live; adoption is UNLIMITED because daemon takeovers restore
rows without metadata (owned fell 4→1 across one — the mirror self-heals in
one tick instead of trickling). The mount path for a freshly clicked tab row
is the last unverified edge: the wrapper resume-by-id is proven from a shell,
the row-mount context needs one live click to confirm.

**Still open:** the identity rebind (see `pending-bugs.md` [CLI] entry) —
`id_assigned_at_birth: false` for opencode2 plus store→row mapping through the
opencode service API. Cold resume of a v2 session cannot work until then, and
the Issue 26 tab mirror needs the same `ses_…` ids as its CLI-side handle.

**Stopgap landed 2026-08-29 (`62ec8286`):** for self-minting kinds an absent
probe now opens the CLI's OWN resume picker instead of refusing (both the
wrapper and the ensure lane; the sweep never marks these kinds missing), so a
cold opencode row boots into opencode2's own TUI — which re-lists the recent
sessions in its tab bar — instead of painting a dead-end error. Live-verified
the same hour on the row this incident was filed from.

**Store blindness root-caused + fixed (`36d0744e`):** the v2 preview writes
new sessions to a **`session_v2`** table and stops writing the v1-era
`session` table (3 stale rows vs 11 served) — every yggterm reader of
`session` was blind, and the saved-session probe answered "absent" for REAL
`ses_…` ids because opencode's glob roots are empty (one SQLite file is not a
directory). `scan_opencode_sessions` now prefers `session_v2` (ms timestamps,
child/sub-agent sessions filtered), `opencode_store_index_holds_session` is
the descriptor's membership index, and the remote probe consults the declared
index before the (empty) glob walk.

**Service plane substrate landed (`d3e74119`):** `opencode_service` in core —
discovery from the CLI's own registration file
(`~/.local/state/opencode/service.json`, fallback spellings; carries url +
password), HTTP **Basic** auth (`opencode` : password — `packages/opencode/
src/server/auth.ts` in the opencode repo; raw unauthenticated GETs 401), the
active-tabs join (`GET /api/session/active` = open tabs; joined with
`GET /api/session` for title/directory/times, sorted by viewed recency), and
per-session `view`/`prompt` verbs. The mirror facts the next lane needs: the
TUI publishes tab switches as **`tui.session.select`** events on
`GET /api/event` (SSE), `time.viewed` per session is the poll-side focus
signal, and the TUI's own per-cwd tab registry is
`~/.local/state/opencode/beta/tui/tabs.json`. Cold resume of a real id is
`opencode2 --session <ses_id>`.

**⚠ Deploy note (2026-08-29):** the fix reached disk but the daemon's
hot-restart was BLOCKED by 17 working sessions (correct — it protects live
work). `server status` shows `running_build_id` < `on_disk_build_id` until a
restart lands it; verify against the RUNNING ids, never the deploy output.

**Not covered:** does not change the wrapper verbs' contract (they already
took the live arm first), does not parse or write the v2 store schema, does
not alter v1-line opencode.

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

---

## Issue Heading 28: agy (Antigravity) restart births a fresh conversation; startpage under-shows live agy sessions (2026-08-30)

Owner symptoms: *row title and metadata not integrated with agy; on restart the
agy session dies and either a new agy session is spawned or a buggy-titled one
spawns which says the original session is still owned by a PID and continuing
would corrupt the transcript; startpage and folders do not correctly reflect
agy sessions.*

### Root causes, measured 2026-08-30 on the live store

1. **Restart → fresh conversation (the rebind gap).** agy mints its
   conversation id POST-first-turn (`id_assigned_at_birth: false`): the birth
   command carries NO id, so the runtime-identity rebind's string-replace
   (`repoint_stored_launch_command_session_id`) was a no-op, no `Storage` stamp
   ever existed for the row, and restore kept the id-less birth command — the
   first cold attach after every restart spawned a NEW conversation under the
   old row's title while the real transcript was orphaned. Fixed twice over:
   the rebind now rebuilds the command through the vouching builder
   (`apply_agent_runtime_session_id_to_live_session`), and restore gained the
   self-minting kinds' arm (rebound id ⇒ resume command, phantom ⇒ traced
   re-birth).
2. **The "owned by a PID / transcript corrupt" warning = our gate was
   codex-shaped.** `agent_resume_args_match_session` routed every kind but
   Claude Code through the codex matcher, so an `agy --conversation <id>`
   holder was invisible to the second-resume gate (`ensure_remote_runtime_agent_session`,
   `wait_for_external_agent_resume_to_clear`); the second spawn reached agy and
   agy printed its own corruption warning. The gate is now descriptor-driven
   (`descriptor_resume_args_match_session`): binary + `resume_selector_token()`
   + id, the same SSOT the composer reads — agy AND muse holders are now seen,
   classified (external vs stranded-yggterm-owned) and refused with the
   PID-naming message.
3. **Startpage/cwdtree hid every LIVE agy conversation.** The scan's retain
   gate used `conversation_summaries` as the sole durability witness, but agy
   writes the index row LATE — measured this day, both of the day's live
   conversations had NO row while their brain transcripts held turns. The gate
   now follows the membership probe's law: the DB verdict stands for ids it
   KNOWS (the 2026-07-14 batch burst stays hidden); for an id it has never
   heard of, the artifact decides — a transcript (not a birth-minted
   `conversations/<id>.db`) is evidence. The remote ssh mirror
   (`REMOTE_AGY_SCAN_SCRIPT`) changed in lockstep.
4. **The live-title reader's third fallback was doubly dead** (old
   `transcript.jsonl` spelling against every root — 552 of ~607 brain dirs hold
   only `transcript_full.jsonl`), so rows with an empty summaries title AND no
   history line fell to the LLM rescue as if the store had nothing — e.g. a
   live row wearing `Remote Antigravity 514c4d23` while its transcript held the
   real first prompt. The reader now tries `transcript_full.jsonl` first per
   root. (Kept: the any-scratch durability rule — re-measured, the 499
   mixed-root rows are ALL the old batch burst, "Transcribe Video File
   Content" ×84; the rule is correct.)
5. **WAL blind-read hardening.** `antigravity_durable_ids` swallows per-row
   errors (`rows.flatten()`) and had no busy timeout: a store read that failed
   halfway answered with the rows it managed to read — a truncated index
   reading as truth. Now: 400 ms busy timeout, any row error fails OPEN
   (`None`), and the scan probe records `db_readable:false`.

### The probe battery (cli-integration plane)

Wired and registered (registered == wired — dead names mislead):
`cli/scan` (per-agy-scan counts: `db_present/db_readable/db_rows/db_durable/
walked/retained/rows/home_cwd`), `cli/scan_total` (duration + per-kind row
counts — the first record to read when startpage/cwdtree disagree with the
store), `cli/resume_decision` (per resume: slug, vouch `vouched|absent|
unanswerable`, action `resume|rebirth`), plus the pre-existing `cli/codex_geometry`.
Battery for a live agy pass:

```sh
ytrace attach --app yggterm 'cli/scan -> @count keep slug,db_readable,rows,retained'
ytrace attach --app yggterm 'cli/resume_decision -> @count by payload.action,payload.vouch'
ytrace attach --app yggterm 'cli where payload.slug == "agy" -> @count by payload.name'
```

### ⚠ FILED, NOT FIXED: the restart round-trip has a later actor that recomposes from the key

At the state layer (`a_rebound_agy_row_restores_as_the_resume_its_conversation_names`,
2026-08-30): rebind rebuilds the resume ✓, persist carries the rebound id ✓,
`restored_local_runtime_id` resolves it ✓ — but a later actor in the same
round-trip recomposes the row from its KEY: the persist-side repair pass inside
`persisted_state_for_update_restart` re-restores a key-id row, and the
post-restore launch of the active row recomposes a fresh birth command
(`agy '<preset flags>'`, no selector) over the rebuilt resume, resetting
`session.id` to the key uuid. `local_runtime_id_from_key(key)` then vouches
`Some(false)` (the birth uuid is agy's phantom) ⇒ re-birth ⇒ fresh spawn —
the user-visible restart bug, one actor deep. Fix next: the launch/ensure path
must not reset a self-minting row's id to its key's uuid when the two differ
(rebound), and must re-derive through the vouching builder — with
`cli/resume_decision` recording every rebirth so the fix is provable.

---

## Issue Heading 29: title integration for every CLI — the heuristic polluter, and live-title readers for the six uncovered CLIs (2026-08-30)

Owner: *only Claude Code's title integration is sort of bullet proof; Codex
and Muse also fail.*

### Measured map (the messageboard post, ACK-6ad55132b5, carries the same table)

| CLI | Own store title? | yggterm reads it |
|---|---|---|
| Claude Code | yes (rollout summaries) | local reader + remote probe — bullet proof |
| Qwen Code | yes | local reader + remote probe |
| agy / Antigravity | yes (`conversation_summaries.db`) | local reader + remote probe |
| OpenCode2 | **yes** (`session_v2.title`, real self-titles) | scanner only — descriptor said `Generated` (drift) |
| Grok Build | yes (`summary.json` `generated_title`, often empty) | scan only |
| Codex / Codex-LiteLLM | **no** — owner spec 2026-06-06: yggterm OWNS these | scan only (first prompt / LLM) |
| Muse | first prompt only (`session-index.db.title`; `New session` when empty) | local reader + remote probe |
| Pi | no (transcript jsonl only) | scan only |
| Kimi | none found (`~/.kimi/kimi.json` is a work_dirs map) | dedicated scanner only |

### The polluter (all CLIs): the heuristic arm cached yggterm's own banner

`generate_for_context`/`generate_for_session` fall back to
`heuristic_title_from_context` when LLM settings are not ready (or when the
LLM answer is low-signal) — and the heuristic arm **stored and returned its
answer unfiltered**. The context is often the terminal's own screen, so
yggterm's attach phrasing ("Muse Code Stays Attached Daemon") and CLI UI lines
became cached titles in `~/.yggterm/session-titles.db` (source='heuristic'),
shadowing real store titles on every read. 77 heuristic rows; 31 poisoned;
purged 2026-08-30 (manual/LLM rows untouched). Fixed at both heuristic sites:
the same `title_is_low_signal_for_cwd` verdict the LLM arm applies now gates
store-and-return. Also added to the shared fallback recognizer: opencode2's
never-prompted shape `New session - <ISO>`.

### Live-title readers wired (the 12-second chore now serves every live row)

`read_live_store_title` existed for only four CLIs; codex, codex-litellm, pi,
opencode, kimi and grok live rows kept their birth names for a whole session
(`SkippedNoReader`). Wired, each reusing its CLI's own truth:

* **Codex / Codex-LiteLLM** — cached yggterm title → the rollout's FIRST REAL
  USER PROMPT (`codex_first_real_user_prompt`): the rollout's first `user`
  item is the AGENTS.md/instructions wrapper (measured), which the reader
  skips before cleaning. One remote probe script serves both (per-CLI store
  globs resolve at runtime).
* **OpenCode2** — `session_v2.title` by id (v1 table as tail), placeholder
  shape refused. New `RemoteStoreLocators::HomeRelative` hands the shared db
  path to the remote probe (no store globs to resolve).
* **Pi** — the session jsonl's own store entry (header id == file-name uuid).
* **Grok Build** — the session directory's `summary.json` (generated_title →
  session_summary).
* **Kimi** — ⛔ declared gap: no per-session title store found
  (`~/.kimi/kimi.json` is a work_dirs map); rows stay on the LLM-rescue path.

Coverage locks extended the contract the same commit: a new local reader with
a remote arm demands a remote probe (codex/pi/grok/opencode probes added);
codex-litellm correctly carries NO probe — it has no remote arm.

### ⚠ FILED (next unit): the LLM title rescue is codex-gated

`YggtermCore::generate_title_for_session_path` (server/src/lib.rs caller at
daemon.rs:13754) refuses any transcript that fails `is_codex_session_file`,
and resolves identity via `read_codex_session_identity` — so the rescue (the
designed last resort for `TitleAuthority::Generated` kinds) never fires for
Muse/Pi/Grok/OpenCode/Kimi rows whose own store carries no usable title
(measured: muse zero-content sessions hold `New session`/`hi`, correctly
filtered, and then nothing rescues them). Fix shape: a
`generate_title_for_transcript(session_id, cwd, path, force)` that skips the
codex identity parse (the caller — the chore candidate — already holds both),
gates on ANY descriptor's `store_path_is_session_file`, and requires the
candidate to carry a real transcript path (muse live rows may need the
`Storage` stamp their reader can already locate via
`find_muse_session_jsonl_in`).

### Kimi re-homed: the installed CLI writes `~/.kimi-code`, not `~/.kimi` (2026-08-30)

Measured against the INSTALLED kimi (0.27.0, `~/.kimi-code/bin/kimi`): every
kimi path in this descriptor pointed at the PREVIOUS kimi's home — the
installed CLI never touches `~/.kimi`, which is why kimi rows had no store
title at all. The live store is
`~/.kimi-code/sessions/wd_<slug>_<hash>/session_<uuid>/` with `state.json`
(`title` = first prompt, `isCustomTitle` on rename, `workDir`, timestamps)
and `agents/main/wire.jsonl`. Verified end-to-end against the owner's
litellm endpoint: `kimi -m chatgpt/gpt-5.6-luna -p …` creates the session,
`state.json` carries the title, and yggterm's scan/reader/probe all read
that file. Kimi left the dedicated-scanner set (the store is
glob-expressible now); the old `scan_kimi_sessions` (dead home) is gone.
Resume flag stays `--resume` (the CLI's own resume hint prints `-r`).

### ⛔ The second-spawn gate now covers the LOCAL door (the agy fork prompt)

`local_agent_cli_launch_refusal_for_path` — the funnel every local agent
launch passes — previously checked only that the binary exists. A resume
whose conversation is already held by a live process (hot restart keeping
the CLI's children alive; twin rows naming one conversation) reached the
CLI, which printed its own fork-or-corrupt warning naming the ghost holder.
The funnel now runs the descriptor-driven holder scan
(`external_agent_resume_processes_for_session`) and refuses BY PID
(`external_active_refused_local_spawn` trace event,
policy session_survival_before_yggterm_attach).

### ⛔ The blank-reveal repaint law: an identical-geometry resize signals nobody (2026-09-02)

A fullscreen TUI repaints on input, a REAL winsize change, or its own timers —
never on a client merely attaching. After a GUI restart the mount's startup
resize asked for the grid the PTY already held, the kernel (correctly) did not
signal the child, and the idle TUI slept while the fresh client showed its
empty surface for the whole reveal (measured: 39.5 s, pending-bugs [11.39]).
The resize wire request now carries `repaint` (serde-default false): at
identical geometry the daemon bounces the PTY one row and restores it — two
real SIGWINCHes — and the TUI's CURRENT frame arrives as ordinary bytes. The
startup repair and the post-attach redraw nudge send it; shadows never do.
Witness: `resize_repaint_nudge` + mock-tui `--scenario winch-repaint`.

### ⛔ The SSOT session-title law: one name per row, rail and store (2026-09-02)

Owner law: an agent row's title is the session's real title or, before that
exists, the birth title `New {machine} {CLI}` — the SAME string the metadata
rail shows. The cwd is not an agent's name (`humanized_terminal_title` no
longer composes `{directory} {CLI}` for agent kinds; shells keep it), the
daemon's remote-resume fallback stamps the birth title instead of
`Remote {CLI} {shorthash}` (lib.rs, both sites), the low-signal title
detector derives the `yggterm {CLI}` placeholder family from the registry
(both hyphenated and spaced spellings — the hand list is how "yggterm
opencode" leaked), and the rail resolves a low-signal stored title through
the same humanized fallback the row label ends at (pending-bugs [11.41]).

## Issue Heading 30: the metadata plane speaks each CLI's dynamicity language (2026-09-02)

Owner directive: *"We need to make metadata integration with each CLI perfect
and then address the row title issues of each CLI. Most CLIs can switch
session midway with internal mechanisms. Yggterm should detect that and our
metadata system should understand their dynamicity language. Specially
opencode is very dynamic."*

### Measured defects behind this issue (all live 2026-09-02)

1. **The metadata pane's Title field dropped CLI titles and showed lies.** The
   pane printed `session.title` raw: a stored CLI session with no carried
   title rendered NO Title line, and rows restored from persistence kept the
   forbidden `Remote {CLI} {shorthash}` shape for days because every title
   chore tick recorded `no_title_in_store` and stopped — the detector KNEW
   the title was machine copy, but nothing ever INSERTED a replacement.
2. **The pane's Session id line showed the row uuid as a session id.** For a
   uuid-keyed OpenCode anchor row that uuid is a seat, not a session id; the
   label was a hand match covering two CLIs.
3. **A Dynamic CLI's retitle never landed.** OpenCode rewrites session titles
   for their whole life (auto-title after the first prompt, human rename in
   the TUI, fork names); the title chore's settle-skip (idle + well-formed
   title ⇒ never polled again) froze last week's name on the row forever.
4. **The anchor was "first row that qualified", not the live TUI.** With four
   uuid rows and one real TUI the anchor-as-header title landed on an
   arbitrary dead row.
5. **Nothing surfaced which session a TUI is rendering right now** — the
   anchor's whole point, invisible to the metadata pane.

### The contract, as landed

* **Title mutability is a per-CLI registry fact**
  (`agent_cli::title_mutability`): `Dynamic` for OpenCode (the one measured
  retiler), `Static` for every CLI not yet measured as one — extend the match
  with the measurement, never a guess. A Dynamic CLI's non-owner-set title is
  NEVER "settled": the chore polls every tick, and the store-agrees check
  keeps a quiet tick write-free.
* **A store-silent row wearing a detector-caught fallback gets the birth
  title INSERTED** (`CliTitleOutcome::InsertedBirthTitle`), per the ACT VII
  lesson: a detector that filters the lie is half a fix. A row wearing a
  real-looking title with a silent store is left alone.
* **The pane's session-id line resolves in registry order**: the CLI store id
  (`session_metadata_label`) → the tab mirror's id → the row uuid — labelled
  by the registry (`OpenCode Session`, `Codex Session`, …), never a hand
  match.
* **Dynamicity is metadata**: the mirror stamps `Viewing Tab Session Id` on
  the anchor every tick (the session the human is LOOKING at, from the
  service's focus stream) and clears it when the service goes quiet — a stale
  "Viewing" claim would be a lie about the present. The pane shows it as
  "Viewing session" (and a tab row's own id as "Mirrored session").
* **The anchor is the LIVE TUI**: selection prefers a row with a pid or a
  running phase; the first-qualified order survives only as the fallback for
  a set with no live TUI.

### What this issue does NOT cover

* The identity language (which session id a row is bound to mid-flight) —
  that is the existing rebind machinery (CC `/clear`, codex resume, the
  runtime-id poll) and the tab mirror's focus-follow; this issue only
  SURFACES the dynamic state, it does not add a new switch detector.
* The phantom-resume defect: uuid-keyed OpenCode rows restored via
  `resume-opencode <row-uuid>` boot TUIs on session ids no store ever held
  (measured live 2026-09-02: three such TUIs running). The birth-title
  insertion stops the lie on the row, not the ether resume itself; that
  repair belongs to the restore path and is filed in pending-bugs.
* Restore metadata naming the wrong CLI's verb (`resume-codex` on an OpenCode
  row) is fixed on the fs-truth lane (bug B1) and merges separately.

## Issue Heading 31: live CLI probes — expected vs actual, on the trace plane and in the metadata pane (2026-09-03)

Owner directive: *"why don't we add probe points called cli/common,
cli/opencode, etc. in the pathways so you can dynamically 'see' in live
yggterm GUI as you try to launch new session what is expected and what is
actually happening?"* — for metadata integration, dynamic row updates, the
working indicator, and daemon switching, CLI by CLI.

### The gap that motivated it (measured 2026-09-03)

Four live `opencode-runtime://<uuid>` rows on dev, all titled `New dev
OpenCode`, all carrying their birth uuid as the OpenCode Session — while
their TUIs rendered real `ses_…` sessions (one verified: the TUI showed the
`sessions`-switched campaign session, the row still named the uuid). The
mirror-tick rebind (`lane/dev/mirror-tick-rebind`, in the running build,
ticks running, focus visible on the trace) emitted **zero** rebind events —
and no instrument could say why: not which anchor was picked among five live
TUIs, not what viewing each tick computed, not which rail of the rebind
refused. The failure was unobservable from every surface. Probes are the fix
for the unobservability; they are not the fix for the rebind.

### The contract

Four ytrace probes, all under the cli-plane laws (one category `cli` except
the daemon one; skips are outcomes; edge-triggered or change-gated, never
per-tick spam; shapes and counts, never user content — cwd, flags, prompts,
screen text and argv strings stay out):

* **`cli/mirror_tick`** — one event per INTERESTING mirror tick (spawned /
  retired / focus present / identity diverged), plus a five-minute heartbeat
  on quiet ticks so a synced mirror is distinguishable from a dead one (the
  plane's own law — ~288 small events a day, stated here, not discovered
  later). Payload:
  `anchor` (row path or null), `candidates` (count of live-anchor-eligible
  rows — with N live TUIs this is where the single-anchor model shows),
  `viewing` (viewed session id or null), `bound` (anchor's bound id or null),
  `decision`: `in_sync` | `diverged` | `rebound` | `rebind_failed` |
  `no_anchor` | `anchor_not_live` | `no_viewing`, `active_tabs`. The
  `diverged` outcome is the event this probe exists for: bound ≠ viewing and
  no rebind happened, with the anchor, the candidate count and both ids on
  the event instead of in a debugger.
* **`cli/launch_contract`** — one event per composed launch/resume that
  DEGRADES from the descriptor-declared shape (the ses_ guard's fresh-launch
  degrade, the store-vouch absent-arm rebirth, the service-vouched override).
  Payload: `slug`, `declared_selector`, `action`, `selector`, `carries_id`,
  `reason`: `as_declared` is never emitted (the `cli/launch` shape event
  already covers the faithful path) — `ses_guard_degrade` |
  `store_absent_rebirth` | `service_vouched_resume`. Expected vs actual, per
  launch, at the moment of composition.
* **`cli/working_edge`** — the daemon's working verdict TRANSITIONS per row
  (`working` | `idle` with the two sub-signals `screen_signal` and
  `recency_signal` as bools), so the blinking dot's flicker is attributable:
  a dot that blinks on recency alone reads differently from one the CLI's own
  footer drives. Edge-triggered (enter/exit vs the last pass set); quiet rows
  cost nothing.
* **`cli/osc_witness`** — generic OSC classes per PTY reader, first-sight
  per reader epoch (added with the implementation, `lane/dev/osc-witness`):
  classes only, never parameters — titles carry cwds, hyperlinks carry URLs.
  Answers the owner's "what is the CLI emitting for OSC?" with one filter.
* **`daemon/idle_gate_eval`** — the swap-deferral DECISION mirrored from the
  existing `daemon_cold_shutdown_deferred_idle_gate` trace event (same
  change-or-heartbeat discipline) plus the same-version HotRestart-request
  defer (per-request, bounded by restart requests). Payload: `blocker_count`,
  blocker classes, head blocker — which session pins the swap, without
  guessing.

### GUI surface: the Live Diagnostic section (read-only)

The metadata pane gains a `Live Diagnostic` group, rendered ONLY for agent
rows that have something dynamic to say (a Viewing stamp present, or bound
identity diverged from it) — quiet rows show nothing new. Entries are
composed from snapshot fields the GUI already holds (no new wire fields —
the JS forwarder drops those silently, measured):

* `Identity`: `in sync` | `DIVERGED — row aims at <bound>, TUI shows
  <viewing>` (labels from the registry, never hand matches).
* `Working`: the existing Status wording; the sub-signal breakdown lives on
  `cli/working_edge`, referenced by name, not duplicated.

⛔ The pane is a witness, never a driver: nothing in it feeds back into
daemon decisions.

### What this issue does NOT cover

* The rebind failure itself (Defect A: zero rebinds with live code+ticks+
  focus) — these probes are the instrument that names it; the repair is a
  separate unit, decided from probe evidence, not from this spec.
* The single-anchor model vs N live TUIs (Defect B): `candidates` on
  `cli/mirror_tick` measures it every tick, but per-TUI identity binding is
  its own design unit with its own spec.
* Sub-signal detail below bools (which footer needle matched) — follow-up if
  the bools prove insufficient; the needle strings stay out of the trace
  until a spec explicitly admits them.

## Issue Heading 32: the phantom-resume vouch — an anchor's resume must land in the session it was viewing (2026-09-03)

### The defect, measured live

`ytrace tail --category cli`, dev, 2026-09-03 23:34–23:45: **five
`cli/launch_contract` breaches** (`breach: ses_guard_degrade`,
`declared_selector: --session`, `selector: ""`, `carries_id: false`), each
followed seconds later by fresh-TUI chrome on `cli/osc_witness` — a fresh
opencode2 window where the owner's conversation had been. The clusters ride
daemon swaps (23:43:43 start → four breaches 23:44–23:45), and the mirror
named the standing wound between them: `cli/mirror_tick` `diverged` on every
tick (anchor bound to the row uuid, `viewing` riding a real `ses_…` id).

The causal chain, all measured:

1. An OpenCode ANCHOR row is keyed by its birth uuid
   (`opencode-runtime://<uuid>`); opencode2's store has never held that id
   (the descriptor's `id_assigned_at_birth: false`).
2. Restore and every remount compose the anchor's resume through
   `ensure_remote_runtime_agent_session`, which passes the row's id — the
   uuid — to the persistent-resume composer.
3. The ses_ guard (Issue 27's fix) correctly refuses the phantom and
   degrades to a fresh launch. Correct refusal, wrong final answer: the
   owner's viewed session is abandoned for an empty window, and because the
   row never learns the `ses_` id, the loop repeats at every restart.

### The repair: vouch to the row's own focus truth

`ensure_remote_runtime_agent_session` now consults the anchor's
`Viewing Tab Session Id` metadata (the mirror's per-tick focus stamp,
Issue 30) when the requested id is not `ses_`-shaped. If the stamp names a
service-shaped session, the resume is composed with THAT id, and the
function's existing tail re-points the row — id, launch command, Restore
line, registry — in the same step it already ran. The key does not move:
the mirror keeps keying its tab rows by the service id and never adopts
uuid keys, so anchor and tab rows stay distinct.

* Probe: the vouch emits `cli/launch_contract` with breach
  `service_vouched_resume` — the third rail the probe reserved at Issue 31,
  silent until now.
* No stamp (fresh anchor, mirror never saw a focus) → today's degrade
  stands, byte-identical behaviour. A stale stamp degrades gracefully:
  resuming the last-viewed session is still the owner's conversation,
  never an empty window.

### What this unit does NOT cover

* The cold-restore arm where the anchor row was NOT persisted (no row, no
  stamp, no vouch) — the store-side "newest ses_ for cwd" candidate stays
  OPEN; it is heuristic under N-windows-one-cwd and needs its own measured
  unit.
* Per-TUI identity binding for OTHER self-minting CLIs — muse/agy bind via
  the `live_session_marker` /proc walk; OpenCode's descriptor carries no
  marker (its truth lives in one shared sqlite db), which is exactly why
  the vouch rides the mirror's stamp instead.
* Defect B (single-anchor vs N TUIs) beyond what the vouch already heals:
  every anchor now resumes ITS OWN last-viewed session; the mirror's
  one-anchor scope is unchanged.

### Regression locks

* `an_anchor_resume_vouches_to_the_session_the_mirror_saw_it_viewing` —
  stamp present: the composed resume names the vouched `ses_` id, the
  phantom uuid survives nowhere in the launch, the Restore line names
  `resume-opencode <ses_…>`.
* `an_anchor_without_a_viewing_stamp_still_degrades_instead_of_resuming_a_phantom`
  — stamp absent: no `--session` on the composed launch, no phantom id
  carried.
