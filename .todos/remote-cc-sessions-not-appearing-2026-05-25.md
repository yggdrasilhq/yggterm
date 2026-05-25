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

- [ ] **Bug 5**: Unify button colors — create `session_kind_primary_color(kind, palette)` function, use it for both top-bar and card buttons. Fix "Open in Codex" from `#6366f1` to `palette.accent`. Decide on CC color: amber `#d97706` or `palette.accent`. Apply consistently everywhere.
- [ ] **Bug 6**: Fix `load_remote_machine_sessions_from_mirror` to preserve session path type (add `session_path` column to mirror DB). Without this, CC sessions appear correctly only until the daemon restarts; after restart they're loaded with wrong paths.
- [ ] **Bug 7**: Fix 7 duplicate sidebar rows (live + stored).
- [x] **gstack skill**: Installed on pi, dev, and jojo. See below.
- [ ] **Verify after daemon restart**: Restart the daemon fresh (without triggering a scan first) and confirm CC sessions still appear correctly (this tests Bug 6's impact).

---

## Key Code Locations for Next Agent

| Concept | File | Line |
|---------|------|------|
| `remote_scanned_session_is_durable` | `crates/yggterm-server/src/lib.rs` | 1359 |
| `dedupe_remote_scanned_sessions` | `crates/yggterm-server/src/lib.rs` | 10454 |
| `run_remote_python_lines` (stdin fix) | `crates/yggterm-server/src/lib.rs` | 10555 |
| `load_remote_machine_sessions_from_mirror` (Bug 6) | `crates/yggterm-server/src/lib.rs` | 10367 |
| `mirror_remote_machine_sessions` | `crates/yggterm-server/src/lib.rs` | 10323 |
| `REMOTE_CC_SCAN_SCRIPT` | `crates/yggterm-server/src/lib.rs` | 11582 |
| `scan_remote_machine_sessions` | `crates/yggterm-server/src/lib.rs` | 11698 |
| Top-bar button styles (Bug 5) | `crates/yggterm-shell/src/shell.rs` | 63437–63453 |
| Card `open_button_style` (Bug 5) | `crates/yggterm-shell/src/shell.rs` | 63635–63652 |
| `remote_scanned_session_is_start_page_durable` | `crates/yggterm-shell/src/shell.rs` | 63192 |
