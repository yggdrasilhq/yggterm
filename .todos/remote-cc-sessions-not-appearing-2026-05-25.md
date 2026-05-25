# Remote CC Sessions Not Appearing + Button Color / Source-of-Truth Audit

**Branch:** main  
**Version:** 2.7.51  
**Date opened:** 2026-05-25  
**Status:** CC sessions now appear (commits `aee502b`, `f611677`). UI color bugs logged below — not yet fixed.

---

## Problem Statement

Start pages are inconsistent across machines. CC (Claude Code) sessions from remote machine `dev` never appeared in jojo's sidebar or start page. Additionally, the "Open in Codex" / "New Codex Session" button colors differ, and the local vs remote start page has inconsistent action buttons. Multiple hardcoded color sources, no single source of truth.

---

## Bug 1 — CC Sessions Silently Dropped by `storage_path` Durability Check  
**Status: FIXED (commit `aee502b`)**

### Root Cause
`dedupe_remote_scanned_sessions` (lib.rs:10454) calls `remote_scanned_session_is_durable` which returns `!session.storage_path.trim().is_empty()`. CC sessions were constructed with `storage_path: String::new()` (empty), so they were silently dropped — they never reached the daemon snapshot or the GUI.

### Fix
- Python script `REMOTE_CC_SCAN_SCRIPT` now emits `'path': str(jsonl_path)`
- `RemoteCcSummaryLine` struct gains `path: String` field
- Session constructor changed: `storage_path: summary.path` (was `String::new()`)

---

## Bug 2 — CC Scan stdin Pipe Never Closed → 20-Second Timeout  
**Status: FIXED (commit `f611677`)**

### Root Cause
`run_remote_python_lines` (lib.rs:10555) sent the script to stdin with `child.stdin.as_mut()`, which borrows without taking ownership. After `write_all`, the `ChildStdin` was never dropped, so the pipe remained open. `python3 -` reads until EOF — since EOF never came, it blocked forever. `wait_remote_command_with_timeout` fired after 20 seconds with "failed waiting for ssh python on dev".

### Why This Was Hard to Find
Running the scan manually (from the user's shell) always succeeded because interactive SSH has ControlMaster multiplexing. The daemon opens fresh SSH connections with `BatchMode=yes`. With a 25MB active session file and no stdin EOF signal, the Python process never exited.

### Fix
Changed `child.stdin.as_mut()` → `child.stdin.take()`. The `ChildStdin` is immediately dropped after `write_all`, sending EOF to the remote python3 process. Applied to both `run_remote_python_lines` and `run_remote_python`.

---

## Bug 3 — CC Scan Script Drops Sessions with Empty `cwd`  
**Status: FIXED (commit `f611677`)**

### Root Cause
`REMOTE_CC_SCAN_SCRIPT` had `if not resolved_id or not cwd: return None`. All CC JSONL files on `dev` lack a top-level `cwd` field in the format the script expected, so every session returned `cwd = ""`, and all were dropped.

### Fix
Removed `not cwd` from the guard. The guard is now `if not resolved_id: return None`. The `cwd` field can be empty (the session is still valid; it just has an unknown working directory). Return value uses `cwd or ''` to guarantee a string.

---

## Bug 4 — CC Scan Script Reads Entire File → Hangs on Large Active Sessions  
**Status: FIXED (commit `f611677`)**

### Root Cause
The Python script read every line of every JSONL file with no size limit. The active Claude Code session `254c9ef2` was 25MB and growing (being written to during the scan). Reading 25MB line-by-line over SSH was slow enough to combine with Bug 2 to reliably hit the timeout.

### Fix
Added `MAX_BYTES_PER_FILE = 512 * 1024`. The scan loop now tracks bytes read and breaks after 512 KB. Session metadata (session_id, cwd, title) all appear in the first few KB; 512 KB is generous.

---

## Bug 5 — "Open in Codex" Card Button Uses Different Color Than "New Codex Session" Button  
**Status: NOT FIXED — logged for next agent**

### Location
`crates/yggterm-shell/src/shell.rs`

| Button | Style Source | Color |
|--------|-------------|-------|
| **"New Codex Session"** (start page top bar) | `primary_button_style` → `palette.accent` | theme-aware blue |
| **"New Claude Code Session"** (start page top bar) | `claude_code_button_style` → hardcoded `#d97706` | amber (hardcoded) |
| **"Open in Codex"** (session card) | `open_button_style` → hardcoded `#6366f1` | indigo (hardcoded) |
| **"Open in Claude"** (session card) | `open_button_style` → hardcoded `#d97706` | amber (hardcoded) |

**Problems:**
- "New Codex Session" = `palette.accent` (blue). "Open in Codex" = `#6366f1` (indigo). **Different colors for the same action on the same session kind.**
- "New Claude Code Session" = hardcoded `#d97706`. "Open in Claude" = also hardcoded `#d97706`. These match each other but are fully disconnected from the palette.
- Three separate style definitions for two logical colors (Codex color, CC color). Any future color change must be made in 3+ places.
- Card buttons use `min-height:28px; padding:0 10px`. Top bar buttons use `min-height:34px; padding:0 13px`. Different sizes for equivalent primary actions.

### Single Source of Truth Design (what it should be)
DESIGN.md section "Primary buttons" says: blue background is acceptable for the main affirmative action. A consistent rule:

```
Codex sessions → palette.accent (theme-aware blue)
CC sessions    → #d97706 (amber) OR palette.accent (pick one, enforce everywhere)
```

The card `open_button_style` at line 63635 should use the same color variable as the top-bar action buttons. One function — `session_kind_primary_color(kind, palette)` — should be the single source of truth, called by both the card buttons and the top-bar buttons.

### Code Locations
- Top-bar buttons: shell.rs lines 63437–63453 (`quick_button_style`, `claude_code_button_style`, `primary_button_style`)
- Card buttons: shell.rs lines 63627–63652 (`open_button_label`, `open_button_style`)
- Context menu "New Codex Session" / "New Claude Code Session": shell.rs lines 64510–64548 (uses `context_menu_action_style`, no color difference)

---

## Bug 6 — `load_remote_machine_sessions_from_mirror` Reconstructs CC Sessions with Wrong Path  
**Status: NOT FIXED — logged for next agent**

### Location
`crates/yggterm-server/src/lib.rs` line 10382

```rust
session_path: remote_scanned_session_path(machine_key, &session_id),
```

This always produces `remote-session://{machine_key}/{session_id}`. But CC sessions need `remote-cc://{machine_key}/{session_id}`.

### Impact
When CC sessions are saved to the mirror DB (which happens after a successful scan), loading them back via `load_remote_machine_sessions_from_mirror` gives them the wrong path type. This means:
- After the daemon restarts and loads from mirror, CC sessions appear in `machine.sessions` with `remote-session://` paths instead of `remote-cc://` paths
- `is_claude_code_session_path` returns false for them
- They're treated as Codex sessions in the sidebar row builder

### What Needs to Be Stored
The `session_path` (which encodes the type) must be stored in the mirror DB alongside `session_id`, so it can be reconstructed correctly. Currently the DB schema has no `session_path` column — it reconstructs the path from just `session_id` using `remote_scanned_session_path`, which assumes Codex format.

**Fix**: Add `session_path` column to `remote_session_metadata` table, store it during `mirror_remote_machine_sessions`, and use it during `load_remote_machine_sessions_from_mirror`. Fall back to `remote_scanned_session_path` if the column is empty (for migration).

---

## Bug 7 — 7 Duplicate Sidebar Rows (live + stored)  
**Status: NOT FIXED — logged for next agent**

7 sessions appear twice in browser rows — once as a live session and once as a stored session. Root cause is likely in `push_remote_machine_rows` or `merged_sidebar_rows_uncached` where live projections are not properly deduplicating stored scanned sessions.

---

## Mishaps / Investigation Detours

### Wrong binary on jojo (initial deployment)
The binary at `/home/pi/.local/share/yggterm/direct/versions/2.7.51/yggterm-headless` is what the daemon actually runs — NOT `~/.local/bin/yggterm-headless`. Discovered this when SCP to `~/.local/bin/` didn't change daemon behavior. The GUI launcher copies from `.local/bin/` to the versioned path on first launch; after that, the versioned copy is what gets executed. Deployment requires: kill GUI + daemon → SCP → copy to versioned path → restart.

### `session_count: 95` in trace but CC sessions in snapshot
The `session_count` trace event in `apply_remote_machine_refresh_scan` correctly reflects the final session array length. Before Bug 2 was found, the count being 95 (not 115) was confusing — it confirmed CC sessions never made it to the snapshot, not that they were being deduped at display time.

### SSH ControlMaster difference
When running the CC scan manually from the user's shell (`ssh jojo 'ssh dev python3 -'`), it completed in ~0.2 seconds. The daemon (a subprocess with different process state) doesn't benefit from any ControlMaster socket, so each SSH connection is a full handshake. This masked the stdin bug in manual testing.

---

## What Was Verified Live (Screenshots)

- After `f611677` was deployed and a `refresh-remote dev` was triggered: `session_count: 115` in trace (95 Codex + 20 CC)
- `server app rows` shows `CC rows: 20, Codex/dev rows: 55` — CC sessions in sidebar ✓
- Screenshot of start page shows CC session cards with "Open in Claude" buttons appearing in the recent work list ✓

---

## What Still Needs to Be Done

All major bugs fixed. No remaining open items.

- [x] **Bug 5**: Unified button colors via `session_kind_primary_bg(kind, accent)` function in shell.rs. "Open in Codex" card now uses `palette.accent`. Fixed commit `3f9aecd`.
- [x] **Bug 6**: Added `session_path` column to `remote_session_metadata` mirror DB. `mirror_remote_machine_sessions` stores full path. `load_remote_machine_sessions_from_mirror` reads it back with fallback. Verified: 20 CC rows survive daemon restart with `remote-cc://` paths. Fixed commit `3f9aecd`.
- [x] **Bug 7**: Fixed duplicate rows (7 → 0). Two fixes: (1) `inject_cc_sessions_into_stored_rows` skips sessions already in stored_rows by session_id; (2) `display_promoted_sessions` excludes sessions already in machine scanned set. Total rows: 127 → 112. Fixed commit `3f9aecd`.
- [x] **gstack skill**: Installed on pi, dev, and jojo. See below.
- [x] **Verify after daemon restart**: Killed daemon, waited for GUI to restart it. CC rows: 20 before, 20 after. All `remote-cc://` paths correct. Bug 6 verified.

---

## Bug 5 — Button Color Fix Details (commit `3f9aecd`)
**Status: FIXED**

Added `fn session_kind_primary_bg<'a>(kind: SessionKind, accent: &'a str) -> &'a str` near `row_session_kind` (~line 14558 of shell.rs). Returns `#d97706` for CC, `accent` for Codex/CodexLiteLlm. Both top-bar buttons and card `open_button_style` now call this function. The hardcoded `#6366f1` (indigo) for card Codex buttons is gone — they now use `palette.accent` (same blue as top-bar "New Codex Session" button).

---

## Bug 6 — Mirror DB Path Type Fix Details (commit `3f9aecd`)
**Status: FIXED**

1. `open_remote_metadata_mirror_store` (lib.rs:10291): Added `session_path TEXT NOT NULL DEFAULT ''` to schema. Runs `ALTER TABLE ... ADD COLUMN session_path` after CREATE TABLE, swallowing "duplicate column name" for existing databases.
2. `mirror_remote_machine_sessions` (lib.rs:10323): Stores `session.session_path` as param `?15` in INSERT.
3. `load_remote_machine_sessions_from_mirror` (lib.rs:10367): Reads `session_path` at index 12. Uses it directly if non-empty, falls back to `remote_scanned_session_path()` otherwise (migration path for older DB rows).

---

## Bug 7 — Duplicate Row Fix Details (commit `3f9aecd`)
**Status: FIXED**

Two separate root causes:

**Fix A** — Local CC session doubled with file-backed row (`inject_cc_sessions_into_stored_rows`, shell.rs ~line 19804):
Added `stored_session_ids: HashSet<String>` from existing stored rows. Skips live CC sessions whose `session.id` is already in `stored_session_ids`.

**Fix B** — Remote live Codex sessions doubled in Live Sessions + machine group (`merged_sidebar_rows_uncached`, shell.rs ~line 19431):
After resort, build `machine_scanned_paths: HashSet<String>` of all session paths in any machine's `scanned_sessions`. Filter `display_promoted_sessions` to exclude sessions in `machine_scanned_paths`. This prevents sessions that are both live AND in the dev scan from appearing in both the "Live Sessions" group and the dev machine group.

Result: Row count 127 → 112, duplicates 7 → 0. Verified live.

---

## gstack Installation (commit `3f9aecd`)
**Status: DONE**

[gstack](https://github.com/garrytan/gstack) by Garry Tan — Claude Code skill framework with 25+ slash commands.

- **pi**: `git clone ssh://dev/home/pi/gh/gstack ~/.claude/skills/gstack && cd ~/.claude/skills/gstack && ./setup`
- **dev**: `git clone ~/gh/gstack ~/.claude/skills/gstack && ./setup` (bun installed first)
- **jojo**: `git clone https://github.com/garrytan/gstack.git ~/.claude/skills/gstack && ./setup` (bun installed first)

Available skills: `/review`, `/ship`, `/qa`, `/investigate`, `/plan-eng-review`, `/plan-ceo-review`, `/retro`, `/office-hours`, `/autoplan`, and 16+ more. See `CLAUDE.md` for the full list.

---

## Key Code Locations

| Concept | File | Approx Line |
|---------|------|-------------|
| `session_kind_primary_bg` (Bug 5 SSOT) | `crates/yggterm-shell/src/shell.rs` | 14558 |
| Top-bar button styles | `crates/yggterm-shell/src/shell.rs` | 63443–63453 |
| Card `open_button_style` | `crates/yggterm-shell/src/shell.rs` | 63635 |
| `inject_cc_sessions_into_stored_rows` (Bug 7a) | `crates/yggterm-shell/src/shell.rs` | 19804 |
| `display_promoted_sessions` filter (Bug 7b) | `crates/yggterm-shell/src/shell.rs` | 19442 |
| `open_remote_metadata_mirror_store` (Bug 6 schema) | `crates/yggterm-server/src/lib.rs` | 10291 |
| `mirror_remote_machine_sessions` (Bug 6 store) | `crates/yggterm-server/src/lib.rs` | 10323 |
| `load_remote_machine_sessions_from_mirror` (Bug 6 load) | `crates/yggterm-server/src/lib.rs` | 10367 |
| `remote_scanned_session_is_durable` | `crates/yggterm-server/src/lib.rs` | 1359 |
| `REMOTE_CC_SCAN_SCRIPT` | `crates/yggterm-server/src/lib.rs` | 11582 |
