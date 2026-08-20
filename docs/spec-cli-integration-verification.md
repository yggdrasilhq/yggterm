# Spec: CLI integration verification — the per-host verb + manual oracle pattern

**Status:** DRAFT 2026-08-16 · **Owner:** yggterm core/server
**Motivation:** Every CLI integration (startpage, titles, resume) has shipped a bug that was invisible to unit tests but trivial to see by comparing what the GUI *said* against what the stores *actually* contain on each host. The fix is always the same shape, so make the shape a spec instead of re-inventing it per bug.

This spec generalizes the `server startpage ls` fix (2026-08-16, durable 106 vs manual 106 proven) to all CLI-derived UI.

---

## 1. Principle

> **If the UI derives from stores, the daemon must be able to re-derive the same UI from stores on demand, and a second program written in a different language must be able to re-derive it from raw files.**

* One source of truth: `AGENT_CLIS` descriptors (`crates/yggterm-core/src/agent_cli.rs`) — `session_store_globs` + `read_store_entry` + `TitleAuthority`. No hand-written glob, path fragment, or title precedence elsewhere.
* One rank: the GUI's ranking function (for startpage: `modified_epoch_ms` desc; for titles: `effective_title` = store title else generated) must live in `yggterm-core` and be called by both the GUI and the verb. A verb that copies the logic is a second encoding waiting to drift.
* One oracle: a Python script that `ssh <host> find` + re-parse (JSONL head/tail, `sqlite3` for antigravity, `summary.json` for grok) — no Rust, no `yggterm` binary. If the two agree, the GUI is not lying.

---

## 2. The pattern — per-host verb + manual checker

```
+-------------------+     +-------------------+     +-------------------+
| GUI startpage/    |     | daemon verb       |     | python oracle     |
| titles rendering  |     | server <area> ls  |     | check-<area>.py   |
| yggterm-shell     | --> | yggterm-server    | --> | scripts/          |
| uses core rank    |     | calls same core   |     | ssh + find +     |
|                   |     | scan_all_*        |     | independent parse |
+-------------------+     +-------------------+     +-------------------+
         \                         |                         /
          \___________ compare ___________/
                      exit 0 = truthful, 2 = lie
```

**Verb contract** (`server <area> ls`):

* Runs on *each* host — Fleet is `local` + every `ssh_target` the daemon knows. `ssh <host> yggterm-headless server <area> ls --json`.
* Reuses `AGENT_CLIS` + `read_store_entry` (and `SessionTitleStore` for generated titles). No literal `".codex/sessions"` outside the descriptor.
* Output is JSON with `host`, `home`, `durable_count`, `live_count`, `rows[]` (each `session_id`, `cwd`, `title`/`effective_title`, `kind`, `modified_epoch_ms`, `storage_path`, `display_path`), `warnings` for `store_scan_gap` (opencode SQLite, kimi MD5) so a missing CLI is *declared*, not silent.
* Ordering is the GUI's ordering, without GUI-only gates that would hide a lying row (for startpage: live-first + recency; for titles: effective-title rank).
* Flags: `--json` (machine), `--limit N` (default 200), `--verbose` (human).

**Checker contract** (`scripts/check-<area>.py`):

* Discovers hosts via `yggterm-headless server daemons --json` + `~/.ssh/config` + `YGGTERM_CHECK_HOSTS`.
* For each host, runs `ssh <host> yggterm-headless server <area> ls --json` (Rust) and `ssh <host> find <literal_prefix> -name <pattern>` + `grep`/`sqlite3`/`cat` (Python). Python never imports Rust.
* Parses each file independently:
  * codex `rollout-*.jsonl` → `id`/`cwd` from JSONL, title = generated (None in file)
  * claude `*.jsonl` → filename is id, `grep '"cwd"'` + `grep -F '"custom-title"' | tail -1` else `'"ai-title"'` (custom precedence as `read_cc_session_title`)
  * pi/qwen first line `id`/`cwd`
  * antigravity `*.db` stem → `conversation_summaries.db` query
  * grok `summary.json` → `info.id`/`cwd`
* Diffs: `verb_ids - manual_ids` (extra), `manual_ids - verb_ids` (missing), `effective_title` mismatches where both non-empty, count drift. Exit 2 on any diff, 0 on `All hosts match`.

---

## 3. Instantiations

### 3.1 Startpage — `server startpage ls` ✅ shipped 2026-08-16

* Core: `crates/yggterm-core/src/startpage.rs` `scan_all_durable_sessions` + `order_for_startpage`
* Verb: `crates/yggterm-server/src/startpage_ls.rs` `run_server_startpage_ls`
* Checker: `scripts/check-startpage.py`
* Proven on `openclaw`: `durable 106 (codex 70 / claude_code 32 / antigravity 4) live 44` vs manual `106` — fixed literal-prefix bug (`.claude/projects/*` → `.claude/projects`) and antigravity central-DB lookup.

### 3.2 Titles — `server titles ls` (next)

Same shape, same files, title-specific rank:

* Core: `scan_all_durable_sessions` already returns `title`/`generated_title`/`effective_title` + `detail`. Title verb reuses it; no new scanner.
* Verb: `server titles ls --json` — rows sorted by `effective_title` presence (store/generation) + `modified_epoch_ms` (same as startpage), plus `live_session_paths` with daemon's current `title` for live rows.
* Checker: `scripts/check-titles.py` — per-host `find` as above, plus for each `session_id` query `~/.yggterm/session-titles.db` (`SELECT title FROM session_titles WHERE session_id=?`) to get generated copy, then compare `effective_title` = store title else generated. ⚠ **Corrected 2026-08-20:** this used to say *for Muse expect store `None`* — it is not. That CLI's `session-index.db` has a `title` column and it holds the **first prompt, verbatim, never updated**, so the oracle must expect a store value there and expect it to be a CLAMPED label (`AgentCliDescriptor::store_entry`, first sentence then a 72-char word-boundary cut) rather than the paragraph on disk. For antigravity expect store present.
* Fleet truth: titles are the row the GUI *paints*; the checker proves the GUI's `effective_title` vs raw stores.

* Sweep verb: `server titles sweep [--dry-run] [--limit N] [--prune] [--kind <slug>] [--json]` — the ACT half of the same answer. It classifies every durable row with the SAME recognizer `ls` reports through (`looks_like_generated_fallback_title`), resolves the bad ones store-first then by generation, stops the moment the endpoint refuses (a report that kept going would blame the sessions for the endpoint), and with `--prune` forgets copy for sessions that exist nowhere — never younger than 7 days, never while the daemon is unreachable, because a live row's copy is keyed by its runtime id and would otherwise read as an orphan.

**Acceptance:** `check-titles.py` `0` on `openclaw`, `oc`, `dev` with no `verb has X ids not in manual` and no `title mismatch` where both present.

### 3.3 Resume readiness — replacing the Re-resume gate

The current gate (`crates/yggterm-shell/src/resume_gate.rs`, `shell.rs:REMOTE_RESUME_GATE_MAX_HOLD_MS 90s`, `NON_PROMPT_WAIT 30s`, `INPUT_GATE_STUCK_* 45s/60s`) is the old LLM's answer to *"when may the user type after a remote resume?"* It holds input on a text heuristic (`terminal_surface_has_prompt_ready_text`) and releases on wall-clock ceilings. On Muse it sticks: all sessions in `Re-resume gate` (plain shell fallback) because the prompt glyph (`›`/`❯`) was never measured for Muse, so `working_footer_hints` is empty and the gate never sees ready.

**The new answer is probes, not a timer.** Keep the verb+oracle pattern for the gate's decision:

* Probe verb: `server resume ls --json` — per-host, per-session `attach_ready_seen`, `runtime_output_seen`, `post_resize_output_seen`, `pty_cols/rows` vs client `cols/rows` (squish gauge), `was_ever_ready`, `daemon_owns_runtime`, `last_output_ms`. All from `server snapshot` + `server app state` + `terminal tenants` — no heuristic, just facts the daemon already publishes.
* Checker: `scripts/check-resume.py` — `ssh` each host, `cat` the live PTY's `terminal_lines` (daemon vt100) vs client `text_tail`, and `stat` the transcript's last `mtime`. If `terminal_lines` shows a prompt/composer and `pty` matches client, the row is ready — the gate should be open.
* Gate removal plan: delete `resume_gate.rs` ceilings, keep only the fast-path `terminal_live_host_connected` + `attach_ready_seen` fact. Input is disabled only while `attach_ready_seen==false` *and* `daemon_owns_runtime==false`. No `NON_PROMPT_WAIT`; no `90s` ceiling. A stuck row is then a *probe* failure (`daemon_owns_runtime false` → needs re-attach, not a timer) and `check-resume.py` says which probe failed. Leave the `RemoteResumeGateCeiling` type deleted, not deprecated — a second encoding of the same deadline is how it survived.

Profiling hook for later: `server perf-incidents` + `render_top` will record how long a resume *actually* takes per CLI, so the 60s `REMOTE_TERMINAL_RESUME_FAIL_MS` can be tuned per-CLI from data, not guessed.

---

## 4. Adding a new CLI — one place

Per `spec-adding-an-agent-cli` + this spec:

1. Add `AgentCliDescriptor` in `agent_cli.rs` (slug, globs, `read_store_entry`, `TitleAuthority`, `composer_marker`, `working_*_hints` measured from a live screen, not strings).
2. Nothing else: `scan_all_durable_sessions` picks it up, `server startpage ls` / `server titles ls` / `server resume ls` and their checkers pick it up, `build_local_cwd_tree` picks it up. A second `if kind==Muse` anywhere is a bug.

---

## 5. Workflow (binding)

No CLI-derived UI change (startpage, titles, resume, cwd tree) ships without:

1. `server <area> ls --json` green on the host, and
2. `scripts/check-<area>.py --verbose` `0` on that host, and
3. `cargo test -p yggterm-core -p yggterm-server --lib` + `cargo test -p yggterm-shell --lib -- start_page` green.

The verb+checker pair is the harness `docs/integration-testing.md` Phase A asks for, but below the full GUI — deterministic, no network, no timing luck.

---

## 6. Roadmap

* [x] `server startpage ls` + `check-startpage.py` (2026-08-16)
* [ ] `server titles ls` + `check-titles.py` (Muse + AGY titles — the immediate follow-on)
* [x] `server resume ls` fully wired 2026-08-16 — probe-based, no glyph heuristic (daemon_owns_runtime + attach_ready_seen + was_ever_ready + working/idle_secs/pty gauge). Gates neutered: `resume_gate.rs` ceilings 0, `retained_remote_surface_should_wait=>false`, `INPUT_GATE_STUCK_*` 1ms. Muse Re-resume stuck gone for ALL sessions.
* [ ] Delete `resume_gate.rs` type entirely (currently delete-not-deprecate with 0 constants) + ship `check-resume.py` oracle
* [ ] `build_local_cwd_tree` → `scan_all_durable_sessions` (remove 3 hardcoded scanners)
* [ ] CI: run both checkers on `openclaw`/`oc`/`dev` per push (read-only, no daemon restart)
