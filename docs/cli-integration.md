# CLI integration — what yggterm promises per agent, what it actually does, and what to fix next

**Status:** DRAFT 2026-08-16 · **Owner:** yggterm core/shell/server · **Campaign:** yggterm
**Steer for the next session:** `see the yggterm campaign and complete the docs/cli-integration.md work completely.`

This is the **one** place that answers "which CLI is first-class and where does it still lie?" The
answer is not a vibe ("Muse mostly works") but a matrix: for each of the 10 registered CLIs,
which of yggterm's promises (startpage, titles, cwd tree, live, resume, launch, cost) is
truthful and which is a second encoding waiting to drift. The harness that proves it is
`spec-cli-integration-verification.md` (verb + Python oracle); the procedure for *adding* a new
CLI is `spec-adding-an-agent-cli.md`. This file is the **BUGS** half — the current delta.

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

## 1. The matrix — what the user is promised vs what the code delivers

"Works" means **verified** by `server <area> ls --json` + `scripts/check-<area>.py` `0` on
`***`/`dev`/`***` **and** a faithful screenshot with the right glyph/colour/title.
"Shipped" means code landed but the verb/oracle still reports a lie. "Gap" means the descriptor
declares `store_scan_gap` (honest `unknown`, not `false`).

| CLI | slug | schemes | store | `TitleAuthority` | startpage `durable` | titles `effective_title` | cwdtree `icon` | live `kind`/`source` | resume `ready` | launch/resume cmd | `check-*` oracle |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **codex** | `codex` | `remote-session://` / `codex-runtime://` `local://` | `~/.codex/sessions/**/rollout-*.jsonl` (id inside file, name carries timestamp) | `Generated` (none in file) | **Works** ✅ `order_for_startpage` `modified_epoch` | **Works** ✅ `Muse` generated `M_` is `codex` `>_` `#0f766e` — generated title via `SessionTitleStore` | **Works** ✅ `>_` `#0f766e` `build_local_cwd_tree` | **Works** ✅ `codex` `codex-runtime://` `RemoteBootstrap` | **Works** ✅ `server resume ls` probes `daemon_owns_runtime+attach_ready_seen` (gates neutered 2026-08-16) | `codex resume <id>` measured | `check-startpage/titles` `0` on 2026-08-16 `durable 106 (70+32+4) live 44` |
| **claude-code** | `claude-code` | `remote-cc://` / `cc-runtime://` | `~/.claude/projects/*/*.jsonl` (filename is id, `custom-title` > `ai-title`) | `Store` (`custom` > `ai`) | **Works** ✅ | **Works** ✅ `*_ #c2410c` | **Works** ✅ `*_` | **Works** ✅ `claude_code` `cc-runtime://` | **Works** ✅ | `claude -r <id>` | `0` |
| **muse** | `muse` | `remote-muse://` / `muse-runtime://` | `~/.local/share/muse/sessions/**/session.jsonl` + `session-index.db` (`sessions.workspace_root→cwd, title, updated_at_us`, fallback `route_facts.cwd`) | `Generated` (store `None`, DB prompt) | **Shipped, lies** — durable `scan_all_durable_sessions` now correct (`M_ #86198f` via `SessionTitleStore` DB prompt, `d703` `see yggterm campaign meomry…`), verb `server startpage ls` returns `muse` rows, but GUI startpage on `yggterm` scope still shows `1319` `dev` `claude-code` only (screenshot) — scope filter `build_local_cwd_tree` + `order_for_startpage` still hard-codes `codex`/`claude` | **Shipped, lies** — `server titles ls` now correct (`M_` + `effective_title` = DB prompt) on `***` (`titles` 117 rows, `check-titles` verb 117 vs manual 117 minus `transcript` oracle bug), but `snapshot` `live` for `remote-muse://oc/d703` is `codex LiveLocal` not `muse LiveSsh` (`event-trace` `live_session_birth kind Codex` 5×). `server app rows` `depth1 session >_ Codex` (parent, wrong) + `depth4 muse M_` (child, correct) — second birth site `insert_live_session`/`open_or_focus_session` still hard-codes `Codex` for `remote-muse` (`synthesize_remote_scanned_*` is correct) | **Shipped, lies** — `server cwdtree ls` durable `M_` correct, but `build_local_cwd_tree` live icon fallback still `session >_ Codex` for `remote-muse` at `depth1` (same birth bug) | **Shipped, lies** — `remote-muse://oc/d703` `codex LiveLocal` not `muse LiveSsh`, `server connect remote-muse://oc/d703` → `Error: saved Codex session d703 is no longer available` (daemon `OpenStoredSession kind=Muse` is correct, but `remote_saved_agent_session_exists(Muse)` is not yet `true` for `muse` store, and `live_session_birth` re-creates as `Codex`) | `muse resume <uuid>` / `muse --yolo` measured 2026-08-08, but `working_screen_phrases &[]` unmeasured → Re-resume gate holds forever (plain shell fallback) until `server resume ls` probes replace it | `check-startpage/titles` `0` minus `transcript` (`brain/*/.system_generated/logs/transcript.jsonl` → `transcript` as `session_id` vs Rust `brain` UUID) + `***` `PATH` missing `~/.local/bin/yggterm-headless` |
| **antigravity** | `antigravity` | `remote-agy://` / `agy-runtime://` | `~/.gemini/antigravity-cli/conversations/*.db` (stem = id, `conversation_summaries` title, `transcript.jsonl` under `brain`) | `Store` (`conversation_summaries`) | **Shipped, lies** — `4` durable `A_ #1557b0` now correct, but same `yggterm` scope bug as `muse`; `transcript` oracle bug (`transcript` as `session_id`) makes `check-*` report `manual has 1 ids not in verb ['transcript']` even though counts `117` | **Shipped** — `38fe0c6f` `title null` correct for 0-byte `.db` (`New Antigravity Session` placeholder until first prompt), non-empty DB `conversation_summaries` title is honoured | **Shipped** | **Gap** — `agy-runtime://` not verified, `New Antigravity Session` placeholder until `conversation_summaries` appears | `agy` resume unmeasured | `check-*` `transcript` bug |
| **pi** | `pi` | `remote-pi://` / `pi-runtime://` | `~/.pi/agent/sessions/*/*.jsonl` (first line `id`/`cwd`) | `Store` | **Shipped, lies** — descriptor added 2026-08-08, but `working_screen_phrases &[]` unmeasured → Re-resume gate `Re-resume gate` forever, `launch --yolo` not measured, `check-*` not verified | **Shipped, lies** | **Shipped** | **Shipped, lies** — live `pi` via `live::` + `session_kind:"pi"` needs `LiveSsh` path, `local_managed_cli_tool_for` was `remote-cc`/`remote-session` only (now fixed for `muse`/`agy`/etc., but `pi` `LiveSsh` still `Shell` in `connect_session_kind_for_path` before fix) | `pi` resume unmeasured | — |
| **qwen** | `qwen` | `remote-qwen://` / `qwen-runtime://` | `~/.qwen/projects/*/chats/*.jsonl` (first line `id`/`cwd`, exclude `.runtime.`) | `Store` | Same as `pi` — `qwen` hides `--yolo`/`--approval-mode` from `--help` (both work, measured), `working` unmeasured | Same | Same | Same | `qwen` resume unmeasured | — |
| **opencode** | `opencode` | `remote-opencode://` / `opencode-runtime://` | `~/.local/share/opencode/opencode.db` single SQLite (`session` table `id/directory/title`) — **declared-unscannable** (`store_scan_gap` true, `every_agent_cli_declares_a_store` requires sentence) | `Store` | **Gap — by design** — `scan` is honest `unknown` (`true` not `false`): `server <area> ls` declares `store_scan_gap` warning, `check-*` cannot diff past sessions (only `live::` if any). A fix needs a scanner-shaped hook yielding MANY entries from ONE path + `rusqlite` WAL-safe read. | **Gap** | **Gap** | **Gap** | `opencode` resume unmeasured | `check-*` will never be `0` for `opencode` durable by design |
| **kimi** | `kimi` | `remote-kimi://` / `kimi-runtime://` | `~/.kimi/sessions/<md5(cwd)>/<id>/context.jsonl` — **declared-unscannable** (`md5(cwd)` bucket, `cwd` not recoverable from path, reverse map `~/.kimi/kimi.json work_dirs[]` needs `md5`, `sha2` only) | `Store` | **Gap — by design** — same `true` honest unknown; closing needs `md-5` + licence notice or indexing `kimi.json` directly. Deferred also because upstream says `kimi-cli` wound down for `MoonshotAI/kimi-code`. | **Gap** | **Gap** | **Gap** | `kimi` resume unmeasured | — |
| **grok-build** | `grok-build` | `remote-grok://` / `grok-runtime://` | `~/.grok/sessions/*/*/summary.json` (`info.id`/`cwd`) | `Store` | **Shipped, lies** — descriptor added, but `summary.json` `info.id`/`cwd` not verified via `check-*` (Python does `summary.json` but not tested), `store_scan_gap` false, `G_ #000000` not yet proven on `check-startpage` `0` | **Shipped, lies** | **Shipped** | **Gap** — `grok-runtime` not verified | `grok` resume unmeasured | — |
| **codex-litellm** | `codex-litellm` | `codex-litellm://` | `~/.codex-litellm/sessions/**/rollout-*.jsonl` (`.bak.` excluded) | `Generated` | **Shipped** — `TMPDIR` leak fixed (`cli-staging` + `sudo -n du` + floor), but titles/startpage not verified | **Shipped** | — | — | `codex-litellm` resume unmeasured | — |

*No other CLI-derived UI change ships without `server <area> ls --json` + `scripts/check-<area>.py --verbose` `0` on `***`/`oc`/`dev`/`***` + `cargo test -p yggterm-core -p yggterm-server --lib` (`spec-cli-integration-verification.md:5`).*

---

## 2. Harness — one rank, one scan, one oracle

> **If the UI derives from stores, the daemon must be able to re-derive the same UI from stores on demand, and a second program in a different language must be able to re-derive it from raw files.** — `spec-cli-integration-verification.md:1`

* **One source of truth:** `AGENT_CLIS` (`crates/yggterm-core/src/agent_cli.rs`) — `session_store_globs` + `read_store_entry` + `TitleAuthority` + `icon_glyph`/`brand_color`/`remote_row_scheme`/`runtime_key_scheme`/`store_scan_gap`. No literal `".codex/sessions"` outside the descriptor.
* **One rank:** `scan_all_durable_sessions` + `order_for_startpage` (`modified_epoch_ms` desc) + `effective_title` (store `TitleAuthority::Store` `custom>ai` else `Generated` via `SessionTitleStore`) live in `yggterm-core` and are called by GUI (`yggterm-shell` `build_local_cwd_tree`, `startpage.rs`) **and** verb (`yggterm-server` `startpage_ls.rs`/`titles.rs`). A verb that copies the logic is a second encoding.
* **One oracle:** `scripts/check-startpage.py` / `check-titles.py` / `check-cwdtree.py` — `ssh <host> find <literal_prefix> -name <pattern>` + `grep`/`sqlite3`/`cat` (Python never imports Rust), `YGGTERM_CHECK_HOSTS` + `~/.ssh/config` + `server daemons --json` discovery, `find` uses `~/.local/bin/yggterm-headless` (non-interactive `PATH` on `***` is `/usr/local/bin:/usr/bin:/bin`, misses `~/.local/bin`). Diffs `verb_ids - manual_ids`, `manual_ids - verb_ids`, `effective_title` mismatches. Exit `2` = lie, `0` = truthful.

Instantiations:

* `server startpage ls` ✅ 2026-08-16 (`durable 106 live 44` vs manual `106`, literal-prefix `.../projects/*` → `.../projects` + antigravity central-DB fix).
* `server titles ls` — same core, plus `SessionTitleStore` (`~/.yggterm/session-titles.db` `SELECT title FROM session_titles WHERE session_id=?`) for `Generated` (`muse` `None` + generated when turns exist; `antigravity` store present). Fleet truth: the row the GUI *paints*.
* `server resume ls` — probe-based (`daemon_owns_runtime` + `attach_ready_seen` + `was_ever_ready` + `working/idle_secs` + `pty` gauge), gates neutered 2026-08-16 (`resume_gate.rs` ceilings `0`, `retained_remote_surface_should_wait=>false`, `INPUT_GATE_STUCK_*` `1ms`), `check-resume.py` oracle planned.

Adding a new CLI is one `AgentCliDescriptor` — `spec-adding-an-agent-cli.md:2` — then `scan_all_durable_sessions` picks it up; a second `if kind==Muse` anywhere is a bug.

---

## 3. What to do — the next three commits, in order

**Commit 1 — folder scope + live birth `Muse` (the screenshot bug):**
1. `crates/yggterm-server/src/lib.rs` `insert_live_session`/`open_or_focus_session`/`remote_saved_agent_session_exists` + `crates/yggterm-shell/src/shell.rs` `build_local_cwd_tree`/`session_kind_for_row` — make `remote-muse`/`agy`/`grok`/`kimi`/`qwen`/`pi` `live` births use `agent_scheme::session_kind_for_path` / `parse_remote_agent_session_path_with_kind` (not hard-coded `Codex`), and selected-folder `startpage` filter `order_for_startpage` + `build_local_cwd_tree` to honour the selected folder (e.g. `myproject 16` not global `1319`). Remove the `session >_ Codex` fallback at `depth1` for `remote-muse` (second birth site) — `synthesize_remote_scanned_*` is already correct.
2. `scripts/check-*.py` — fix `transcript` oracle (`brain` UUID not `transcript`) and `***` `PATH` (`~/.local/bin/yggterm-headless` fallback), then `check-titles/cwdtree/startpage --host *** --host ***` `0` with `muse`/`agy` rows present.
3. `muse --yolo resume` `7a319776`/`798c5bd3` on `oc` if not running (`ps aux | grep muse`), `server app session remove` the three stale `codex LiveLocal` `remote-muse` rows on `***`, `server connect` each → `muse LiveSsh M_ #86198f` at `depth1`, `server snapshot` `muse` + `server app rows` `M_` + `os`/`xterm` 1920×1200 `capture_faithful true` + 4-frame `perf-summary`/`render-top`.

**Commit 2 — `precis` full drop:**
`crates/yggterm-server/src/lib.rs:3084` `RemoteScannedSession.cached_precis` field + `cached_precis TEXT` column + `persist_remote_generated_copy` 7→6 args already done in `yggterm-refresh-copy.rs`, but `cached_precis` column migration + `store_scan_gap` for `opencode`/`kimi` + `SessionTitleStore` `precis_*` methods (`titles.rs:117` deprecated stubs `None`) still present (83 hits `cached_precis`). Delete column (SQLite `ALTER TABLE ... DROP COLUMN` is `12.0`, so recreate), field, and stubs; `cargo test` + `check-*` still `0`.

**Commit 3 — `check-*` CI + `grok`/`qwen`/`pi` *measured* `working_screen_phrases`:**
Run the three checkers on `***`/`oc`/`dev`/`***` per push (read-only, no daemon restart), and fill `working_screen_phrases`/`working_footer_hints`/`composer_marker` for `pi`/`qwen`/`grok`/`kimi` from live screens (not `--help` strings) so `server resume ls` no longer reports `Re-resume gate` for those CLIs.

No CLI-derived UI change ships without `5` above.

---

## 4. Inventory — which spec/doc now lives where

* `spec-cli-integration-verification.md` — the **harness** (verb + oracle pattern, `AGENT_CLIS` SSOT, adding a CLI is one descriptor).
* `spec-adding-an-agent-cli.md` — the **procedure** for a new CLI (10 recon questions, descriptor fields, rolling-upgrade hazard).
* **This file** — the **BUGS** matrix (what is promised vs what is delivered for each of the 10 CLIs, with falsifiers and next commits).
* `pending-bugs.md:CLI` — pointer to this file (open) plus the `6.7` tmpfs/swap leak (open, 78 MB × 51, 2.85 GB RAM, `install_npm_batch` + `ygg-resource-panic.sh` + `ygg-zed-upgrade.service.d` shipped, sweep bounds it).

Steer next session with: `see the yggterm campaign and complete the docs/cli-integration.md work completely.`

