//! Startpage ground-truth helpers — shared by shell rendering and `server startpage ls`.
//!
//! The startpage's `RECENT WORK` list used to be GUI-only (`yggterm-shell/src/shell.rs`
//! `start_page_recent_rows*`). A per-host `server startpage ls` that re-derives the same
//! list from the stores is the cheap lie detector the fleet needs: the GUI can be
//! asked what it showed, the daemon what it thinks the page should be, and a
//! Python oracle can walk the raw jsonls independently.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::agent_cli::{AGENT_CLIS, AgentStoreEntry};
use crate::SessionKind;

/// Whether a session file is noise (empty or zero-prompt placeholder) and should be skipped during scan.
/// Note: Startpage scanning is strictly READ-ONLY. Noise files are skipped in memory and NEVER deleted from disk.
pub fn is_noise_session_file(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() == 0 { return true }
    let path_str = path.display().to_string();
    // Muse: a session with no prompts is noise.
    if path_str.contains("muse/sessions") {
        if let Some(data_root) = crate::agent_cli::muse_data_root_from_session_path(path) {
            let db_path = data_root.join("session-index.db");
            if db_path.exists() {
                if let Ok(conn) = rusqlite::Connection::open_with_flags(
                    &db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_URI
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                ) {
                    if let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
                        if let Ok(mut stmt) = conn.prepare(
                            "SELECT prompt_count, title FROM sessions WHERE session_id=?1",
                        ) {
                            if let Ok(mut rows) = stmt.query(rusqlite::params![parent]) {
                                if let Ok(Some(row)) = rows.next() {
                                    let prompt_count: i64 = row.get(0).unwrap_or(1);
                                    let title: String = row.get(1).unwrap_or_default();
                                    let title_lower = title.trim().to_ascii_lowercase();
                                    if prompt_count == 0
                                        && (title_lower == "new session"
                                            || title_lower.is_empty()
                                            || title_lower == "new muse code session")
                                    {
                                        // The index counter is an observer, not
                                        // transcript truth. Muse has left it at
                                        // zero for a multi-megabyte session whose
                                        // first accepted intent was record 688.
                                        return !crate::agent_cli::muse_session_contains_accepted_user_intent(path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

/// One session as the startpage sees it — durable, from a store file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StartpageDurableRow {
    pub session_id: String,
    pub cwd: String,
    /// Title the CLI itself wrote into its store (when TitleAuthority::Store).
    pub title: Option<String>,
    /// Title yggterm generated via LLM / heuristic when the CLI stores none.
    /// `None` means no generated copy exists yet (or the CLI is store-authoritative).
    pub generated_title: Option<String>,
    /// What the startpage actually shows — store title wins when present,
    /// otherwise generated, otherwise None (falls back to short id).
    pub effective_title: Option<String>,
    pub detail: Option<String>,
    pub kind: SessionKind,
    pub modified_epoch_ms: u128,
    pub storage_path: String,
    pub display_path: String,
}

/// Whether this CLI is scanned by a hand-written scanner instead of the generic
/// glob walk.
///
/// ⛔ SINGLE OWNER of that question. `scan_all_durable_sessions` dispatches on
/// it and the `server <area> ls` warnings consult it, so a CLI cannot be scanned
/// and simultaneously reported as invisible. It could, until 2026-08-20:
/// OpenCode and Kimi have empty `session_store_globs` (their stores are one
/// SQLite DB and an md5-bucketed tree, neither of which a glob can express), so
/// all three `ls` verbs printed "has no store globs and no declared gap —
/// sessions will be invisible" on every run while `scan_opencode_sessions` and
/// `scan_kimi_sessions` were reading them perfectly well.
pub fn kind_has_dedicated_scanner(kind: crate::SessionKind) -> bool {
    matches!(
        kind,
        crate::SessionKind::OpenCode | crate::SessionKind::Kimi | crate::SessionKind::Antigravity
    )
}

/// The home the AGENT CLI STORES live under (`~/.codex`, `~/.claude`, …).
///
/// ⛔ SINGLE OWNER of that question, and it is NOT the yggterm home
/// (`~/.yggterm`). The distinction has now cost two surfaces independently:
/// `server startpage ls` walked the yggterm home and reported 0 rows before
/// growing its own resolution, and the local cwd tree passed the yggterm home
/// to [`scan_all_durable_sessions`] after the 2026-08-17 unification — so
/// every local durable session without a live row vanished from the sidebar
/// and the start page while the `ls` verbs, resolving correctly, still counted
/// it. One resolver, called by every scanner entry point, so a caller cannot
/// pick the wrong home again.
///
/// The yggterm home is the fallback only for the environments where no user
/// home resolves at all (containers with no `$HOME`), where it degrades to the
/// old behaviour instead of panicking.
pub fn agent_store_home(yggterm_home: &Path) -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| yggterm_home.to_path_buf())
}

/// What a memoised row was derived from: the transcript's own `(mtime_ns, len)`.
///
/// Both halves, not just mtime: an agent CLI appends to its JSONL many times a
/// second, and a filesystem whose mtime granularity is coarser than the append
/// rate would otherwise hand back a row built from a shorter file.
///
/// ⚠ NANOseconds, deliberately. At millisecond resolution a title written and
/// re-read inside the same millisecond — which is exactly what the titles
/// sweep does to verify its own writes — reads as "unchanged", and the sweep
/// would report that its write had not landed.
type DurableFileStamp = (u128, u64);

/// Per-process memo for the durable-session scan.
///
/// ⭐ **The scan is a POLL, and almost nothing it reads has changed.** The GUI
/// re-runs it every 8 s and the daemon chore every 12 s over a corpus of
/// hundreds of JSONL transcripts totalling gigabytes; measured 2026-08-21 on
/// a GUI host at p50 4.6 s per run (13.9 s on the larger corpus), with
/// duty cycles of 87-109% of a 60 s window — the dominant steady CPU burn on
/// the machine, alongside recorded input-block incidents. Per file per run
/// the old path
/// paid a fresh SQLite connection (schema batch included), a multi-megabyte
/// tail read, and a `serde_json` parse of every line in that tail — up to
/// three times over.
///
/// None of that work can produce a different answer for a file that has not
/// changed, so it is done once and remembered.
///
/// ⛔ **A row is NOT a pure function of its transcript.** `load_generated_title`
/// reads the title store, which the generation chore writes behind the scan's
/// back — so a memo keyed on the transcript alone would pin a freshly
/// generated title out of sight forever, which is the instrument-freezing
/// failure this project pays for most often. The store's own stamp is
/// therefore the memo's GENERATION: when it moves, every row is rebuilt. One
/// `stat` per scan buys that, against one SQLite open per row.
#[derive(Default)]
struct DurableScanMemo {
    generation: Option<DurableFileStamp>,
    rows: HashMap<PathBuf, (DurableFileStamp, StartpageDurableRow)>,
    hits: usize,
    misses: usize,
}

static DURABLE_SCAN_MEMO: std::sync::OnceLock<std::sync::Mutex<DurableScanMemo>> =
    std::sync::OnceLock::new();

fn durable_scan_memo() -> &'static std::sync::Mutex<DurableScanMemo> {
    DURABLE_SCAN_MEMO.get_or_init(|| std::sync::Mutex::new(DurableScanMemo::default()))
}

/// `(mtime_ms, len)` for a path, or `None` when it cannot be stat'ed.
///
/// A file that cannot be stat'ed is never memoised: an unknown stamp must read
/// as "rebuild", never as "unchanged".
fn durable_file_stamp(path: &Path) -> Option<DurableFileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((mtime, meta.len()))
}

/// The generation stamp: the title store's own `(mtime, len)`, folded with its
/// write-ahead log when one exists.
///
/// ⚠ The `-wal` half matters — under WAL journalling a committed write lands in
/// the sidecar and the main file's mtime can sit still until a checkpoint, so
/// reading only the `.db` would declare the store unchanged after a title was
/// written. Absent files fold in as zeroes, which is a stable stamp of their
/// own ("there is no store").
fn durable_scan_generation(home: &Path) -> DurableFileStamp {
    // ⛔ EVERY store the row builder might read, not just the likely one.
    // `load_generated_title` resolves through `dirs::home_dir()` and falls back
    // from `~/.yggterm` to `~`, while this scan is handed a `home` that a
    // caller may have resolved differently. A generation that watched one path
    // while the rows were built from another would go stale silently — the
    // exact shape of failure the memo is supposed to be safe against — so all
    // of them fold in, at three `stat`s per scan.
    let mut candidates = vec![
        home.join(".yggterm").join(crate::titles::TITLE_DB_FILENAME),
        home.join(crate::titles::TITLE_DB_FILENAME),
    ];
    if let Some(user_home) = dirs::home_dir() {
        candidates.push(user_home.join(".yggterm").join(crate::titles::TITLE_DB_FILENAME));
        candidates.push(user_home.join(crate::titles::TITLE_DB_FILENAME));
    }
    candidates.sort();
    candidates.dedup();
    let mut mtime: u128 = 0;
    let mut len: u64 = 0;
    for db in candidates {
        // ⚠ The `-wal` sidecar matters: under WAL journalling a committed write
        // lands there and the main file's mtime can sit still until a
        // checkpoint, so reading only the `.db` would call the store unchanged
        // just after a title was written.
        let wal = PathBuf::from(format!("{}-wal", db.display()));
        for path in [db, wal] {
            let (path_mtime, path_len) = durable_file_stamp(&path).unwrap_or((0, 0));
            mtime = mtime.rotate_left(1) ^ path_mtime;
            len = len.rotate_left(1) ^ path_len;
        }
    }
    (mtime, len)
}

/// Drop every memoised row, unconditionally.
///
/// ⛔ The in-process net for a title write. The `(mtime, len)` generation stamp
/// is the CROSS-process net — it is how a GUI notices that the daemon wrote a
/// title — but a stamp can only ever be as fine as the clock behind it, and a
/// process that writes a title and immediately rescans to check its own work
/// must not be told the corpus is unchanged. Writers call this directly, so
/// that path does not depend on a timestamp at all.
pub(crate) fn invalidate_durable_scan_memo() {
    let mut memo = durable_scan_memo().lock().unwrap_or_else(|e| e.into_inner());
    memo.rows.clear();
    memo.generation = None;
}

/// Memo counters for the run that just finished, and a reset for the next one.
/// Reported on the scan's perf span so the memo cannot quietly stop working:
/// a hit rate that collapses is a fact about the corpus, and it must be
/// readable without attaching a debugger.
pub fn take_durable_scan_memo_counts() -> (usize, usize, usize) {
    let mut memo = durable_scan_memo().lock().unwrap_or_else(|e| e.into_inner());
    let counts = (memo.hits, memo.misses, memo.rows.len());
    memo.hits = 0;
    memo.misses = 0;
    counts
}

/// Walk every registered agent CLI's store globs under `home` and return
/// one entry per readable session file, using the descriptor's own
/// `read_store_entry` so title/cwd semantics stay single-sourced.
pub fn scan_all_durable_sessions(home: &Path) -> Vec<StartpageDurableRow> {
    let mut out = Vec::new();
    let mut seen_paths = HashSet::<PathBuf>::new();
    // Generation check FIRST, before a single file is read: a title written
    // since the last scan invalidates every memoised row at once, because the
    // store is consulted for every row and a per-row test would cost the very
    // SQLite open the memo exists to avoid.
    {
        let generation = durable_scan_generation(home);
        let mut memo = durable_scan_memo().lock().unwrap_or_else(|e| e.into_inner());
        if memo.generation != Some(generation) {
            memo.generation = Some(generation);
            memo.rows.clear();
        }
    }
    for descriptor in AGENT_CLIS {
        debug_assert!(
            !descriptor.session_store_globs.is_empty()
                || kind_has_dedicated_scanner(descriptor.kind)
                || descriptor.store_scan_gap.is_some(),
            "a CLI with no globs must have a dedicated scanner or a declared gap",
        );
        if descriptor.kind == crate::SessionKind::OpenCode {
            scan_opencode_sessions(home, &mut out);
            continue;
        }
        if descriptor.kind == crate::SessionKind::Kimi {
            scan_kimi_sessions(home, &mut out, &mut seen_paths);
            continue;
        }
        if descriptor.kind == crate::SessionKind::Antigravity {
            scan_antigravity_sessions(home, &mut out, &mut seen_paths);
            continue;
        }
        if descriptor.session_store_globs.is_empty() {
            continue;
        }
        for root in descriptor.store_roots_absolute(home) {
            // `store_roots_absolute` returns the directory above the glob's literal
            // prefix (e.g. `~/.codex/sessions` for codex, `~/.local/share/muse/sessions`
            // for Muse). Walk from that directory even when the glob contains `**`
            // — the recursive walk plus `store_path_is_session_file` does the glob
            // matching, so a shallow literal prefix would otherwise miss deep
            // `YYYY/MM/DD/<uuid>/session.jsonl` layouts.
            let walk_root = root;
            if !walk_root.exists() {
                continue;
            }
            // Also walk the XDG fallback for Muse when `XDG_DATA_HOME` is set
            // but the store still lives under `~/.local/share` — the descriptor's
            // `store_home_env_override` is None for Muse, so we check both.
            walk_and_collect(descriptor, &walk_root, &mut out, &mut seen_paths);
            // For Muse, also check the XDG_DATA_HOME location if it differs
            if descriptor.kind == crate::SessionKind::Muse {
                if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
                    let xdg_root = PathBuf::from(xdg).join("muse/sessions");
                    if xdg_root != walk_root && xdg_root.exists() && !seen_paths.contains(&xdg_root) {
                        walk_and_collect(descriptor, &xdg_root, &mut out, &mut seen_paths);
                    }
                }
            }
        }
    }
    // Add Codex-family legacy roots that live under `~/.codex/sessions` with
    // date subdirectories — already covered by AGENT_CLIS walk, but keep as
    // fallback if walk missed due to permission.

    // Forget what this walk did not meet. A memo that only ever grows would
    // hold rows for deleted transcripts for the life of the process, and would
    // report a row count that no longer describes the corpus.
    {
        let mut memo = durable_scan_memo().lock().unwrap_or_else(|e| e.into_inner());
        memo.rows.retain(|path, _| seen_paths.contains(path));
    }
    out
}

fn scan_opencode_sessions(home: &Path, out: &mut Vec<StartpageDurableRow>) {
    let db_path = home.join(".local/share/opencode/opencode.db");
    if !db_path.exists() {
        return;
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    // ⛔ The v2 preview writes NEW sessions to `session_v2` and stops writing
    // the v1-era `session` table (measured 2026-08-29: `session` held 3 stale
    // rows while the service served 11 — every yggterm reader of `session`
    // was blind to 8 of them, and the membership probe answered "absent" for
    // a REAL `ses_…` id). Read the v2 table first and fall back to the v1
    // table only when the service has never migrated (an older install).
    //
    // v2 timestamp columns are MILLISECONDS since epoch (verified against the
    // service's own `/api/session` output); the v1 table's are seconds.
    //
    // `parent_id` marks CHILD sessions (opencode's sub-agent primitive), not
    // peer conversations — the durable projection is about resumable peer
    // sessions, so children are filtered at the reader, never deleted.
    let has_v2 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'session_v2'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    let (query, times_are_ms) = if has_v2 {
        (
            "SELECT id, directory, title, time_updated, time_created FROM session_v2 \
             WHERE parent_id IS NULL OR parent_id = ''",
            true,
        )
    } else {
        (
            "SELECT id, directory, title, time_updated, time_created FROM session",
            false,
        )
    };
    let Ok(mut stmt) = conn.prepare(query) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let directory: String = row.get(1)?;
        let title: String = row.get(2)?;
        let time_updated: i64 = row.get(3)?;
        let time_created: i64 = row.get(4)?;
        Ok((id, directory, title, time_updated, time_created))
    }) else {
        return;
    };
    for row in rows.flatten() {
        let (session_id, directory, raw_title, time_updated, time_created) = row;
        if session_id.trim().is_empty() {
            continue;
        }
        let cwd = if directory.trim().is_empty() {
            home.display().to_string()
        } else {
            directory.clone()
        };
        let epoch_ms = if time_updated > 0 {
            if times_are_ms {
                time_updated as u128
            } else {
                (time_updated as u128) * 1000
            }
        } else if time_created > 0 {
            if times_are_ms {
                time_created as u128
            } else {
                (time_created as u128) * 1000
            }
        } else {
            0
        };
        let title = raw_title.trim();
        let filtered_title = if title.is_empty()
            || crate::looks_like_generated_fallback_title(title)
            || crate::looks_like_low_signal_generated_copy(title)
        {
            None
        } else {
            Some(title.to_string())
        };
        let descriptor = crate::agent_cli::agent_cli_descriptor(crate::SessionKind::OpenCode);
        let is_store_auth = descriptor.map(|d| d.title_is_store_authoritative()).unwrap_or(false);
        let generated_title = if is_store_auth && filtered_title.is_some() {
            None
        } else {
            StartpageDurableRow::load_generated_title(&session_id)
        };
        let filtered_gen = generated_title.clone().filter(|s| {
            !crate::looks_like_generated_fallback_title(s)
                && !crate::looks_like_low_signal_generated_copy(s)
        });
        let effective_title = if is_store_auth {
            filtered_title.clone().or(filtered_gen.clone())
        } else {
            filtered_gen.clone().or(filtered_title.clone())
        };
        let display_path = descriptor
            .and_then(|d| d.remote_row_scheme)
            .map(|s| format!("{}{}", s, session_id))
            .unwrap_or_else(|| db_path.display().to_string());
        out.push(StartpageDurableRow {
            session_id: session_id.clone(),
            cwd: cwd.clone(),
            title: filtered_title,
            generated_title: filtered_gen,
            effective_title,
            detail: None,
            kind: crate::SessionKind::OpenCode,
            modified_epoch_ms: epoch_ms,
            storage_path: db_path.display().to_string(),
            display_path,
        });
    }
}

fn scan_kimi_sessions(home: &Path, out: &mut Vec<StartpageDurableRow>, seen: &mut HashSet<PathBuf>) {
    // Kimi buckets: ~/.kimi/sessions/<md5(cwd)>/<session-id>/context.jsonl
    // Reverse map lives in ~/.kimi/kimi.json work_dirs[].path
    let kimi_json_path = home.join(".kimi/kimi.json");
    let sessions_root = home.join(".kimi/sessions");
    if !sessions_root.exists() {
        return;
    }
    // Build md5 -> cwd map from kimi.json
    let mut md5_to_cwd: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(content) = std::fs::read_to_string(&kimi_json_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(work_dirs) = v.get("work_dirs").and_then(|w| w.as_array()) {
                for wd in work_dirs {
                    if let Some(p) = wd.get("path").and_then(|s| s.as_str()) {
                        let md5_hex = format!("{:x}", md5::compute(p.as_bytes()));
                        md5_to_cwd.insert(md5_hex, p.to_string());
                    }
                }
            }
        }
    }
    // Also walk all bucket dirs and fallback to unknown if not in map (handles md5 buckets not in work_dirs)
    let Ok(buckets) = std::fs::read_dir(&sessions_root) else {
        return;
    };
    for bucket_entry in buckets.flatten() {
        let bucket_path = bucket_entry.path();
        if !bucket_path.is_dir() {
            continue;
        }
        let bucket_name = bucket_entry.file_name().to_string_lossy().to_string();
        let cwd = md5_to_cwd.get(&bucket_name).cloned().unwrap_or_else(|| {
            // Fallback: unknown cwd, use home; but still surface the session so divergence is visible
            // rather than silently dropping it. The cwd tree will hang it at home.
            home.display().to_string()
        });
        let Ok(sessions) = std::fs::read_dir(&bucket_path) else {
            continue;
        };
        for sess_entry in sessions.flatten() {
            let sess_path = sess_entry.path();
            if !sess_path.is_dir() {
                continue;
            }
            let session_id = sess_entry.file_name().to_string_lossy().to_string();
            if session_id.trim().is_empty() {
                continue;
            }
            let context_path = sess_path.join("context.jsonl");
            if !context_path.exists() {
                continue;
            }
            if !seen.insert(context_path.clone()) {
                continue;
            }
            // Noise check: skip empty placeholder
            if is_noise_session_file(&context_path) {
                continue;
            }
            let modified_epoch_ms = std::fs::metadata(&context_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            // Title: kimi stores no title in context.jsonl system prompt; use generated or heuristic fallback
            let generated = StartpageDurableRow::load_generated_title(&session_id);
            let filtered_gen = generated.clone().filter(|s| {
                !crate::looks_like_generated_fallback_title(s)
                    && !crate::looks_like_low_signal_generated_copy(s)
            });
            // Heuristic from tail context if available
            let heuristic = crate::titles::extract_tail_context(&context_path)
                .ok()
                .and_then(|ctx| crate::titles::heuristic_title_from_context(&ctx))
                .filter(|s| {
                    !crate::looks_like_generated_fallback_title(s)
                        && !crate::looks_like_low_signal_generated_copy(s)
                });
            let effective_title = filtered_gen.clone().or(heuristic.clone());
            let title = filtered_gen.clone();
            let descriptor = crate::agent_cli::agent_cli_descriptor(crate::SessionKind::Kimi);
            let display_path = descriptor
                .and_then(|d| d.remote_row_scheme)
                .map(|s| format!("{}{}", s, session_id))
                .unwrap_or_else(|| context_path.display().to_string());
            out.push(StartpageDurableRow {
                session_id: session_id.clone(),
                cwd: cwd.clone(),
                title,
                generated_title: filtered_gen,
                effective_title,
                detail: None,
                kind: crate::SessionKind::Kimi,
                modified_epoch_ms,
                storage_path: context_path.display().to_string(),
                display_path,
            });
        }
    }
}

/// Parse Antigravity's per-row `last_modified_time` into epoch milliseconds.
///
/// The column is a Go `datetime` rendered with a SPACE between date and time
/// (`2026-08-12 08:41:02.868930001+00:00`), which is ISO-8601 but not RFC-3339,
/// so it needs the separator swapped before `time` will accept it. Rows that
/// were never written carry `0001-01-01 00:00:00+00:00`; those clamp to 0
/// rather than becoming a large negative epoch.
pub fn parse_antigravity_last_modified_ms(raw: &str) -> Option<u128> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Only the FIRST space is the date/time separator; an offset never contains one.
    let rfc3339 = match trimmed.find(' ') {
        Some(ix) => format!("{}T{}", &trimmed[..ix], &trimmed[ix + 1..]),
        None => trimmed.to_string(),
    };
    let parsed = time::OffsetDateTime::parse(
        &rfc3339,
        &time::format_description::well_known::Rfc3339,
    )
    .ok()?;
    let nanos = parsed.unix_timestamp_nanos();
    if nanos <= 0 {
        return Some(0);
    }
    Some((nanos / 1_000_000) as u128)
}

/// Whether one `conversation_summaries` row is a resumable agy CLI session.
///
/// ⛔ Do NOT reach for the columns that look built for this — MEASURED
/// 2026-08-20 on a 999-row store, `source`, `status`, `agent_name`,
/// `nesting_depth`, `parent_conversation_id`, `battle_id`, `not_fully_idle`
/// and `last_user_input_step_index` were uniformly empty/default, and `killed`
/// was 0 for every single row, which makes the scan's `WHERE killed=0` filter
/// a no-op rather than the guard it reads as. The only columns carrying signal
/// are `step_count` and `workspace_uris`.
///
/// The three classes that store held:
///   * 499 rows — a real repo root PLUS an ephemeral `/tmp` scratch dir, all
///     from one batch burst (`step_count` 6, previews like "Transcribe Video
///     File Content"). A tool invocation, not a session someone resumes.
///   * 494 rows — `/tmp`-only workspaces. These are what became the "/tmp
///     forest" of one-session cwd-tree groups.
///   * 6 rows — real roots only. 4 of them have content; 2 are empty shells.
///
/// So: a conversation is durable when it has at least one workspace root, none
/// of its roots is ephemeral scratch, and it actually holds steps.
pub fn antigravity_row_is_durable(workspace_uris: &str, step_count: i64) -> bool {
    if step_count <= 0 {
        return false;
    }
    let roots = crate::antigravity_workspace_paths(workspace_uris);
    if roots.is_empty() {
        return false;
    }
    !roots.iter().any(|root| crate::path_is_ephemeral_scratch(root))
}

/// The conversation ids the summaries DB says are resumable sessions.
///
/// Returns `None` when there is no readable DB, which is the signal to leave
/// the file walk ungated rather than silently show nothing.
fn antigravity_durable_ids(db_path: &Path) -> Option<HashSet<String>> {
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT conversation_id, workspace_uris, step_count FROM conversation_summaries")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let uris: String = row.get(1).unwrap_or_default();
            let steps: i64 = row.get(2).unwrap_or_default();
            Ok((id, uris, steps))
        })
        .ok()?;
    Some(
        rows.flatten()
            .filter(|(id, uris, steps)| !id.trim().is_empty() && antigravity_row_is_durable(uris, *steps))
            .map(|(id, _, _)| id)
            .collect(),
    )
}

fn scan_antigravity_sessions(
    home: &Path,
    out: &mut Vec<StartpageDurableRow>,
    seen: &mut HashSet<PathBuf>,
) {
    let descriptor = crate::agent_cli::agent_cli_descriptor(crate::SessionKind::Antigravity);
    let db_path = home.join(".gemini/antigravity-cli/conversation_summaries.db");
    // The DB is the INDEX of conversations; a brain transcript is only storage.
    // So the durable verdict is taken once, from the DB, and gates BOTH the file
    // walk and the DB rows. Without this the two paths disagree: a batch
    // conversation filtered out of the DB half would walk straight back in
    // through its transcript file.
    let durable_ids = antigravity_durable_ids(&db_path);
    if let Some(desc) = descriptor {
        let mut walked = Vec::new();
        for root in desc.store_roots_absolute(home) {
            if root.exists() {
                walk_and_collect(desc, &root, &mut walked, seen);
            }
        }
        if let Some(ids) = durable_ids.as_ref() {
            walked.retain(|row| ids.contains(&row.session_id));
        }
        // ⛔ One conversation can have MORE THAN ONE file — its per-conversation
        // DB, a brain transcript, and a legacy `.antigravitycli/<id>.json` all
        // resolve to the same id. Deduping by first path made filesystem walk
        // order title authority: the DB won before the title-bearing transcript.
        // Merge by id and fill absent title/detail fields; an existing store
        // title keeps precedence.
        let mut merged = Vec::<StartpageDurableRow>::new();
        let mut merged_indices = HashMap::<String, usize>::new();
        for candidate in walked {
            if let Some(index) = merged_indices.get(&candidate.session_id).copied() {
                let current = &mut merged[index];
                current.modified_epoch_ms = current.modified_epoch_ms.max(candidate.modified_epoch_ms);
                if current.effective_title.is_none() && candidate.effective_title.is_some() {
                    current.title = candidate.title;
                    current.generated_title = candidate.generated_title;
                    current.effective_title = candidate.effective_title;
                    current.storage_path = candidate.storage_path;
                }
                if current.detail.is_none() {
                    current.detail = candidate.detail;
                }
            } else {
                merged_indices.insert(candidate.session_id.clone(), merged.len());
                merged.push(candidate);
            }
        }
        out.append(&mut merged);
    }

    if !db_path.exists() {
        return;
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    // ⛔ `WHERE killed=0` used to stand here as the guard. It filters nothing:
    // every row in a measured 999-row store had killed=0. The real filter is
    // `antigravity_row_is_durable` below.
    let Ok(mut stmt) = conn.prepare(
        "SELECT conversation_id, title, preview, workspace_uris, last_modified_time, step_count \
         FROM conversation_summaries WHERE killed=0",
    ) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let title: String = row.get(1).unwrap_or_default();
        let preview: String = row.get(2).unwrap_or_default();
        let uris: String = row.get(3).unwrap_or_default();
        let raw_mod: String = row.get(4).unwrap_or_default();
        let steps: i64 = row.get(5).unwrap_or_default();
        Ok((id, title, preview, uris, raw_mod, steps))
    }) else {
        return;
    };
    let mut existing_ids: HashSet<String> = out.iter().map(|r| r.session_id.clone()).collect();
    // ⛔ The DB FILE's mtime used to be stamped on every row, giving all of them
    // one shared fake recency that moved whenever agy touched the store. Each
    // row carries its own `last_modified_time`; use it, and fall back to the
    // file mtime only when a row's own timestamp will not parse.
    let db_file_epoch_ms = std::fs::metadata(&db_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);

    for row in rows.flatten() {
        let (session_id, raw_title, raw_preview, uris, raw_mod, step_count) = row;
        if session_id.trim().is_empty() || existing_ids.contains(&session_id) {
            continue;
        }
        if !antigravity_row_is_durable(&uris, step_count) {
            continue;
        }
        let epoch_ms =
            parse_antigravity_last_modified_ms(&raw_mod).unwrap_or(db_file_epoch_ms);
        existing_ids.insert(session_id.clone());
        let cwd = crate::parse_antigravity_workspace_uris(&uris)
            .unwrap_or_else(|| home.display().to_string());
        let title_cand = if !raw_title.trim().is_empty() {
            crate::agent_cli::clean_agy_prompt_first_line(&raw_title)
        } else if !raw_preview.trim().is_empty() {
            crate::agent_cli::clean_agy_prompt_first_line(&raw_preview)
        } else {
            crate::read_antigravity_session_title(home, &session_id).ok().flatten()
        };
        let filtered_title = title_cand.filter(|t| {
            !crate::looks_like_generated_fallback_title(t)
                && !crate::looks_like_low_signal_generated_copy(t)
        });
        let generated_title = StartpageDurableRow::load_generated_title(&session_id).filter(|s| {
            !crate::looks_like_generated_fallback_title(s)
                && !crate::looks_like_low_signal_generated_copy(s)
        });
        let effective_title = filtered_title
            .clone()
            .or_else(|| generated_title.clone())
            .or_else(|| {
                crate::read_antigravity_session_title(home, &session_id)
                    .ok()
                    .flatten()
                    .filter(|t| {
                        !crate::looks_like_generated_fallback_title(t)
                            && !crate::looks_like_low_signal_generated_copy(t)
                    })
            });
        let display_path = format!("agy-runtime://{session_id}");
        out.push(StartpageDurableRow {
            session_id: session_id.clone(),
            cwd,
            title: filtered_title,
            generated_title,
            effective_title,
            detail: None,
            kind: crate::SessionKind::Antigravity,
            modified_epoch_ms: epoch_ms,
            storage_path: db_path.display().to_string(),
            display_path,
        });
    }
}

fn walk_and_collect(
    descriptor: &crate::agent_cli::AgentCliDescriptor,
    dir: &Path,
    out: &mut Vec<StartpageDurableRow>,
    seen: &mut HashSet<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            walk_and_collect(descriptor, &path, out, seen);
        } else if ft.is_file() {
            let path_str = path.display().to_string();
            if !descriptor.store_path_is_session_file(&path_str) {
                continue;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            if is_noise_session_file(&path) {
                continue;
            }
            // ⭐ The memo, checked before any read. A transcript whose
            // `(mtime, len)` is unchanged since the last scan cannot produce a
            // different row, and rebuilding it is what made this walk the
            // machine's dominant steady burn. An unstampable file falls
            // through to the full path rather than being remembered wrong.
            let stamp = durable_file_stamp(&path);
            if let Some(stamp) = stamp {
                let mut memo = durable_scan_memo().lock().unwrap_or_else(|e| e.into_inner());
                if let Some((cached_stamp, row)) = memo.rows.get(&path)
                    && *cached_stamp == stamp
                {
                    let row = row.clone();
                    memo.hits += 1;
                    drop(memo);
                    out.push(row);
                    continue;
                }
                memo.misses += 1;
            }
            if let Some(entry) = descriptor.store_entry(&path) {
                let mut row = StartpageDurableRow::from_entry(entry, descriptor.kind, &path);
                // Weird-title filtering on sight (heading 9): if title/detail looks generated, try heuristic.
                if let Some(ref tt) = row.title {
                    if crate::looks_like_generated_fallback_title(tt) || crate::looks_like_low_signal_generated_copy(tt) {
                        if let Ok(ctx) = crate::titles::extract_tail_context(&path) {
                            if let Some(heu) = crate::titles::heuristic_title_from_context(&ctx) {
                                if !crate::looks_like_generated_fallback_title(&heu) && !crate::looks_like_low_signal_generated_copy(&heu) {
                                    row.title = Some(heu.clone());
                                    row.effective_title = Some(heu.clone());
                                } else {
                                    row.title = None;
                                    row.effective_title = row.generated_title.clone();
                                }
                            } else {
                                row.title = None;
                                row.effective_title = row.generated_title.clone();
                            }
                        } else {
                            row.title = None;
                            row.effective_title = row.generated_title.clone();
                        }
                    }
                }
                if let Some(ref d) = row.detail {
                    if crate::looks_like_low_signal_generated_copy(d) {
                        row.detail = None;
                    } else if crate::looks_like_generated_fallback_title(d) {
                        row.detail = None;
                    }
                }
                if let Some(ref et) = row.effective_title {
                    if crate::looks_like_generated_fallback_title(et) || crate::looks_like_low_signal_generated_copy(et) {
                        if let Ok(ctx) = crate::titles::extract_tail_context(&path) {
                            if let Some(heu) = crate::titles::heuristic_title_from_context(&ctx) {
                                if !crate::looks_like_generated_fallback_title(&heu) && !crate::looks_like_low_signal_generated_copy(&heu) {
                                    row.effective_title = Some(heu);
                                } else {
                                    row.effective_title = row.generated_title.clone();
                                }
                            } else {
                                row.effective_title = row.generated_title.clone();
                            }
                        } else {
                            row.effective_title = row.generated_title.clone();
                        }
                    }
                }
                // A transcript containing only startup metadata is not a
                // durable conversation. It used to survive because it had
                // bytes and a UUID, then both shared surfaces rendered that
                // UUID as an eight-character title. Ask the CLI's measured
                // reader only for otherwise title-less rows, so normal scans
                // remain one read per file.
                let contextless_noise = row.effective_title.is_none()
                    && row.detail.is_none()
                    && match descriptor.kind {
                        crate::SessionKind::Codex
                        | crate::SessionKind::CodexLiteLlm
                        | crate::SessionKind::Pi => {
                            crate::titles::extract_tail_context(&path)
                                .map(|context| context.trim().is_empty())
                                .unwrap_or(false)
                        }
                        crate::SessionKind::Muse => {
                            !crate::agent_cli::muse_session_contains_accepted_user_intent(&path)
                        }
                        _ => false,
                    };
                if contextless_noise {
                    continue;
                }
                if let Some(stamp) = stamp {
                    let mut memo = durable_scan_memo().lock().unwrap_or_else(|e| e.into_inner());
                    memo.rows.insert(path.clone(), (stamp, row.clone()));
                }
                out.push(row);
            }
        }
    }
}

impl StartpageDurableRow {
    fn from_entry(entry: AgentStoreEntry, kind: SessionKind, path: &Path) -> Self {
        let storage_path = path.display().to_string();
        let display_path = match kind {
            SessionKind::Codex => format!("codex://{}", entry.session_id),
            SessionKind::ClaudeCode => format!("claude-code://{}", entry.session_id),
            _ => {
                if let Some(desc) = crate::agent_cli::agent_cli_descriptor(kind) {
                    if let Some(scheme) = desc.remote_row_scheme {
                        format!("{}{}", scheme, entry.session_id)
                    } else {
                        storage_path.clone()
                    }
                } else {
                    storage_path.clone()
                }
            }
        };
        // Resolve generated copy based on title authority.
        // Store-authoritative CLIs (Claude, Qwen, Kimi) use the store title when present;
        // Generated CLIs (Muse, Codex, Antigravity...) prefer the generated copy.
        let raw_title = entry.title.clone();
        let descriptor = crate::agent_cli::agent_cli_descriptor(kind);
        let is_store_authoritative = descriptor.map(|d| d.title_is_store_authoritative()).unwrap_or(false);
        let generated_title = if is_store_authoritative {
            if raw_title.is_none() {
                Self::load_generated_title(&entry.session_id)
            } else {
                None
            }
        } else {
            Self::load_generated_title(&entry.session_id)
        };
        // Filter weird titles (heading 9) before choosing effective.
        let filtered_raw = raw_title.clone().filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s));
        let filtered_generated = generated_title.clone().filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s));
        let effective_title = if is_store_authoritative {
            filtered_raw.clone().or_else(|| filtered_generated.clone()).or_else(|| {
                // Fallback heuristic when store title is weird/empty.
                crate::titles::extract_tail_context(path).ok().and_then(|ctx| crate::titles::heuristic_title_from_context(&ctx)).filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
            })
        } else {
            filtered_generated.clone().or_else(|| filtered_raw.clone()).or_else(|| {
                crate::titles::extract_tail_context(path).ok().and_then(|ctx| crate::titles::heuristic_title_from_context(&ctx)).filter(|s| !crate::looks_like_generated_fallback_title(s) && !crate::looks_like_low_signal_generated_copy(s))
            })
        };
        let detail = entry.detail.filter(|d| !crate::looks_like_low_signal_generated_copy(d) && !crate::looks_like_generated_fallback_title(d));
        Self {
            session_id: entry.session_id,
            cwd: entry.cwd,
            title: filtered_raw,
            generated_title: filtered_generated,
            effective_title,
            detail,
            kind,
            modified_epoch_ms: entry.modified_epoch_ms,
            storage_path,
            display_path,
        }
    }

    fn load_generated_title(session_id: &str) -> Option<String> {
        let home = dirs::home_dir()?;
        let store = crate::SessionTitleStore::open(&home.join(".yggterm")).ok()
            .or_else(|| crate::SessionTitleStore::open(&home).ok())?;
        // Try YGGTERM_HOME first, then plain HOME.
        store.get_title(session_id).ok().flatten()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                // Fallback: check titles.db directly via resolver
                crate::SessionTitleStore::open(&home).ok()?
                    .get_title(session_id).ok().flatten()
            })
    }
}

/// Startpage ordering — MUST stay single-sourced with `yggterm-shell/src/shell/startpage.rs`.
///
/// Shell ranking is `is_live` > `in_scope` > `modified_epoch` > `started_at` > `insertion_index`.
/// The verb `in_scope` is always true and `started_at` is empty, so it collapses to
/// live-first + recency — but the verb MUST go through this same fn so the lie detector
/// cannot drift. The faithful verb therefore calls `order_for_startpage_with_live_scope`
/// with the live/scope it learned from `app state` / `snapshot`; the simple recency
/// fallback below is only for headless oracles that have no GUI state.
pub fn order_for_startpage(mut rows: Vec<StartpageDurableRow>) -> Vec<StartpageDurableRow> {
    rows.sort_by(|a, b| {
        b.modified_epoch_ms
            .cmp(&a.modified_epoch_ms)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    rows
}

/// Faithful ordering — the exact `candidates.sort_by` the shell uses.
///
/// `rows` are `(row, is_live, in_scope, modified_epoch, started_at, insertion_index)`.
/// Kept here so `server startpage ls` and the shell cannot drift.
pub fn order_candidates_for_startpage(
    mut candidates: Vec<(StartpageDurableRow, bool, bool, i64, String, usize)>,
) -> Vec<StartpageDurableRow> {
    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| right.3.cmp(&left.3))
            .then_with(|| right.4.cmp(&left.4))
            .then_with(|| left.5.cmp(&right.5))
    });
    candidates.into_iter().map(|(row, ..)| row).collect()
}

#[cfg(test)]
mod scan_truth_tests {
    use super::*;

    fn summaries_db(dir: &Path, rows: &[(&str, &str, i64)]) -> PathBuf {
        let agy = dir.join(".gemini/antigravity-cli");
        std::fs::create_dir_all(&agy).unwrap();
        let db = agy.join("conversation_summaries.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE conversation_summaries (conversation_id text, title text NOT NULL DEFAULT '', \
             preview text NOT NULL DEFAULT '', step_count integer NOT NULL DEFAULT 0, \
             last_modified_time datetime NOT NULL, workspace_uris text NOT NULL, \
             killed numeric NOT NULL DEFAULT 0, PRIMARY KEY (conversation_id))",
            [],
        )
        .unwrap();
        for (id, uris, steps) in rows {
            conn.execute(
                "INSERT INTO conversation_summaries \
                 (conversation_id, title, preview, step_count, last_modified_time, workspace_uris, killed) \
                 VALUES (?1, '', 'p', ?2, '2026-08-12 08:41:02.868930001+00:00', ?3, 0)",
                rusqlite::params![id, steps, uris],
            )
            .unwrap();
        }
        db
    }

    // The three shapes a real 999-row store held. A batch conversation is told
    // from a session by its workspace, never by `killed` — every row was
    // killed=0, so the `WHERE killed=0` filter decided nothing.
    #[test]
    fn a_batch_conversation_is_not_a_durable_session() {
        // /tmp-only: the "/tmp forest" of one-session cwd-tree groups.
        assert!(!antigravity_row_is_durable(
            r#"["file:///tmp/claude-999/scratchpad/vn_abc"]"#,
            6
        ));
        // A real root with an ephemeral scratch dir beside it — the batch signature.
        assert!(!antigravity_row_is_durable(
            r#"["file:///home/user/proj/sample","file:///tmp/tmpq1w2e3"]"#,
            6
        ));
        // Started but never stepped: an empty shell, not a session.
        assert!(!antigravity_row_is_durable(r#"["file:///home/user/proj/sample"]"#, 0));
        // No workspace at all.
        assert!(!antigravity_row_is_durable("[]", 12));
        assert!(!antigravity_row_is_durable("not json", 12));
        // A real interactive session survives all of it.
        assert!(antigravity_row_is_durable(r#"["file:///home/user/proj/sample"]"#, 14));
    }

    // ⚠ Two batch conversations in the measured store still had LIVE /tmp dirs,
    // so "does the directory still exist" kept them. The rule is on the PATH,
    // which also keeps the scan deterministic as /tmp is reaped.
    #[test]
    fn the_scratch_rule_reads_the_path_not_the_filesystem() {
        let live = std::env::temp_dir().join("ygg_scan_truth_live_probe");
        std::fs::create_dir_all(&live).unwrap();
        assert!(crate::path_is_ephemeral_scratch(live.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&live);
        assert!(crate::path_is_ephemeral_scratch("/tmp"));
        assert!(!crate::path_is_ephemeral_scratch("/tmpfoo/proj"));
        assert!(!crate::path_is_ephemeral_scratch("/home/user/tmp/proj"));
    }

    // Every row used to be stamped with the DB FILE's mtime — one shared fake
    // recency for the whole store, which moved every time agy touched it.
    #[test]
    fn each_row_keeps_its_own_recency() {
        let a = parse_antigravity_last_modified_ms("2026-08-12 08:41:02.868930001+00:00").unwrap();
        let b = parse_antigravity_last_modified_ms("2026-08-16 16:19:03.388933001+00:00").unwrap();
        assert!(a < b, "distinct timestamps must stay distinct: {a} vs {b}");
        assert_eq!(a, 1786524062868);
        // A never-written row clamps to 0 rather than a huge negative epoch.
        assert_eq!(parse_antigravity_last_modified_ms("0001-01-01 00:00:00+00:00"), Some(0));
        assert_eq!(parse_antigravity_last_modified_ms(""), None);
        assert_eq!(parse_antigravity_last_modified_ms("nonsense"), None);
    }

    #[test]
    fn the_scan_keeps_only_real_agy_sessions_and_dates_them_apart() {
        let tmp = std::env::temp_dir().join(format!("ygg_scan_truth_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        summaries_db(
            &tmp,
            &[
                ("keep-me", r#"["file:///home/user/proj/sample"]"#, 14),
                ("batch-tmp-only", r#"["file:///tmp/tmpaaaa"]"#, 6),
                ("batch-mixed", r#"["file:///home/user/proj/sample","file:///tmp/tmpbbbb"]"#, 6),
                ("empty-shell", r#"["file:///home/user/proj/sample"]"#, 0),
            ],
        );
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        scan_antigravity_sessions(&tmp, &mut out, &mut seen);
        let ids: Vec<&str> = out.iter().map(|r| r.session_id.as_str()).collect();
        assert_eq!(ids, ["keep-me"], "only the real session survives, got {ids:?}");
        assert_eq!(out[0].modified_epoch_ms, 1786524062868, "row recency, not DB file mtime");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // One conversation, more than one file on disk — the shape that put the same
    // session in one cwd-tree group twice.
    #[test]
    fn a_conversation_stored_in_two_files_is_still_one_session() {
        let tmp = std::env::temp_dir().join(format!("ygg_scan_dup_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let id = "3f2a71c4-9b0e-4d6a-8c15-a7e30bd94f62";
        let db = summaries_db(&tmp, &[(id, r#"["file:///home/user/proj/sample"]"#, 9)]);
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute(
            "UPDATE conversation_summaries SET preview='' WHERE conversation_id=?1",
            rusqlite::params![id],
        )
        .unwrap();
        drop(conn);
        // A brain transcript AND a legacy json, both naming the same conversation.
        let brain = tmp
            .join(".gemini/antigravity-cli/brain")
            .join(id)
            .join(".system_generated/logs");
        std::fs::create_dir_all(&brain).unwrap();
        std::fs::write(
            brain.join("transcript_full.jsonl"),
            b"{\"type\":\"USER_INPUT\",\"content\":\"Trace persistent Antigravity row identity\"}\n",
        )
        .unwrap();
        let legacy = tmp.join(".antigravitycli");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(format!("{id}.json")), b"{}").unwrap();

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        scan_antigravity_sessions(&tmp, &mut out, &mut seen);
        let hits = out.iter().filter(|r| r.session_id == id).count();
        assert_eq!(hits, 1, "one conversation is one row, got {hits}");
        let row = out.iter().find(|row| row.session_id == id).unwrap();
        assert_eq!(
            row.effective_title.as_deref(),
            Some("Trace persistent Antigravity row identity"),
            "the transcript title must enrich the earlier per-conversation DB projection"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn codex_startup_only_transcript_is_not_a_durable_row() {
        let home = dirs::home_dir()
            .unwrap()
            .join(".yggterm/scratchpad")
            .join(format!("codex-startup-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let sessions = home.join(".codex/sessions/2026/08/25");
        std::fs::create_dir_all(&sessions).unwrap();
        let id = "019d5af1-65ea-7fb2-90c1-0123456789ab";
        let transcript = sessions.join(format!("rollout-{id}.jsonl"));
        std::fs::write(
            &transcript,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/home/user/proj\"}}}}\n\
                 {{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\"}}}}\n"
            ),
        )
        .unwrap();

        let rows = scan_all_durable_sessions(&home);
        assert!(
            rows.iter().all(|row| row.session_id != id),
            "startup metadata alone must not surface as an eight-character title"
        );
        assert!(transcript.exists(), "classification must never delete the source");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn pi_startup_only_transcript_is_not_a_durable_row() {
        let home = dirs::home_dir()
            .unwrap()
            .join(".yggterm/scratchpad")
            .join(format!("pi-startup-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let sessions = home.join(".pi/agent/sessions/--home-user-proj");
        std::fs::create_dir_all(&sessions).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let transcript = sessions.join(format!("{id}.jsonl"));
        std::fs::write(
            &transcript,
            format!(
                "{{\"type\":\"session\",\"id\":\"{id}\",\"cwd\":\"/home/user/proj\"}}\n"
            ),
        )
        .unwrap();

        let rows = scan_all_durable_sessions(&home);
        assert!(
            rows.iter().all(|row| row.session_id != id),
            "a Pi header without dialogue must not surface as an id-derived title"
        );
        assert!(transcript.exists(), "classification must never delete the source");
        let _ = std::fs::remove_dir_all(&home);
    }

    // ⛔ THE HIGHEST-STAKES PROPERTY IN THIS FILE. Scanning classifies; it must
    // never remove. An agent once mass-deleted real transcripts while "clearing
    // noise", and the muse index is exactly the sort of signal that would drive
    // it: four sessions on this host carry prompt_count=0 with 12 KB of real
    // lifecycle records on disk. Skipping them is right; deleting them is not.
    #[test]
    fn scanning_never_removes_a_session_file() {
        let tmp = std::env::temp_dir().join(format!("ygg_scan_ro_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let sessions = tmp.join(".codex/sessions/2026/08/20");
        std::fs::create_dir_all(&sessions).unwrap();
        let empty = sessions.join("rollout-empty.jsonl");
        std::fs::write(&empty, b"").unwrap();
        let real = sessions.join("rollout-real.jsonl");
        std::fs::write(&real, b"{\"payload\":{\"cwd\":\"/home/user/proj\"}}\n").unwrap();
        // A muse session the index calls noise, with a non-empty file behind it.
        let muse = tmp.join(".local/share/muse/sessions/2026/08/20/aaaa");
        std::fs::create_dir_all(&muse).unwrap();
        let muse_file = muse.join("session.jsonl");
        std::fs::write(&muse_file, b"{\"schema_version\":1}\n").unwrap();

        assert!(is_noise_session_file(&empty), "an empty file is noise");
        let _ = scan_all_durable_sessions(&tmp);

        for path in [&empty, &real, &muse_file] {
            assert!(path.exists(), "scanning deleted {}", path.display());
        }
        assert_eq!(std::fs::read(&muse_file).unwrap(), b"{\"schema_version\":1}\n");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The Muse index is eventually-consistent metadata, not the authority for
    /// whether a transcript contains work.  A live 9.9 MB session was omitted
    /// from both startpage and cwdtree because this counter remained zero.
    #[test]
    fn muse_transcript_intent_overrides_a_stale_zero_prompt_index() {
        let home = dirs::home_dir()
            .unwrap()
            .join(".yggterm/scratchpad")
            .join(format!("muse-scan-stale-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let muse_root = home.join(".local/share/muse");
        let session_id = "9f1c2e3a-4b5d-46e7-8f90-1a2b3c4d5e6f";
        let session_dir = muse_root
            .join("sessions/2026/08/24")
            .join(session_id);
        std::fs::create_dir_all(&session_dir).unwrap();
        let transcript = session_dir.join("session.jsonl");
        let mut records = (0..80)
            .map(|sequence| {
                format!(
                    "{{\"sequence\":{sequence},\"payload_type\":\"runtime.lifecycle\"}}\n"
                )
            })
            .collect::<String>();
        records.push_str(
            r#"{"payload_type":"runtime.user_intent.accepted","payload":{"model_messages":[{"content":[{"text":"Repair durable Muse discovery"}]}]}}
"#,
        );
        std::fs::write(&transcript, records).unwrap();

        let conn = rusqlite::Connection::open(muse_root.join("session-index.db")).unwrap();
        conn.execute(
            "CREATE TABLE sessions (session_id TEXT PRIMARY KEY, workspace_root TEXT, title TEXT, updated_at_us INTEGER, prompt_count INTEGER);",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions VALUES (?1, ?2, 'New session', ?3, 0);",
            rusqlite::params![session_id, "/home/user/proj", 1_787_586_190_925_000i64],
        )
        .unwrap();
        drop(conn);

        assert!(
            !is_noise_session_file(&transcript),
            "accepted transcript intent defeats the stale zero counter"
        );
        let rows = scan_all_durable_sessions(&home);
        let row = rows
            .iter()
            .find(|row| row.session_id == session_id)
            .expect("the durable Muse session must reach both shared surfaces");
        assert_eq!(row.cwd, "/home/user/proj");
        assert_eq!(
            row.effective_title.as_deref(),
            Some("Repair Durable Muse Discovery")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    // A CLI cannot be scanned and simultaneously warned about as invisible.
    #[test]
    fn a_scanned_cli_is_never_reported_invisible() {
        for desc in crate::agent_cli::AGENT_CLIS {
            let invisible = desc.session_store_globs.is_empty()
                && desc.store_scan_gap.is_none()
                && !kind_has_dedicated_scanner(desc.kind);
            assert!(
                !invisible,
                "{:?} has no globs, no dedicated scanner and no declared gap",
                desc.kind
            );
        }
        assert!(kind_has_dedicated_scanner(crate::SessionKind::OpenCode));
        assert!(kind_has_dedicated_scanner(crate::SessionKind::Kimi));
        assert!(!kind_has_dedicated_scanner(crate::SessionKind::Codex));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ygg-scan-memo-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An append must move the stamp. This is the whole safety of the memo: a
    /// transcript that grew has a row that changed, and a stamp that could not
    /// tell would serve the shorter file's row forever.
    #[test]
    fn durable_file_stamp_moves_when_a_transcript_grows() {
        let dir = scratch("stamp");
        let file = dir.join("session.jsonl");
        std::fs::write(&file, b"{\"one\":1}\n").unwrap();
        let before = super::durable_file_stamp(&file).expect("a written file stamps");

        std::fs::write(&file, b"{\"one\":1}\n{\"two\":2}\n").unwrap();
        let after = super::durable_file_stamp(&file).expect("a grown file stamps");
        assert_ne!(before, after, "an append must be visible in the stamp");
        assert_ne!(before.1, after.1, "and the length half alone already says so");

        assert!(
            super::durable_file_stamp(&dir.join("absent.jsonl")).is_none(),
            "a file that cannot be stat'ed has no stamp — an unknown stamp must \
             read as REBUILD, never as unchanged"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The generation is what stops the memo pinning a stale TITLE, which is
    /// the failure a transcript-only key would have.
    #[test]
    fn durable_scan_generation_moves_when_a_title_is_written() {
        let home = scratch("generation");
        std::fs::create_dir_all(home.join(".yggterm")).unwrap();
        let before = super::durable_scan_generation(&home);

        let store = crate::SessionTitleStore::open(&home.join(".yggterm")).unwrap();
        store
            .put_manual_title("session-under-test", "/tmp", "a new name")
            .unwrap();
        let after = super::durable_scan_generation(&home);
        assert_ne!(
            before, after,
            "a title written behind the scan's back must invalidate every \
             memoised row — the row builder consults this store"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The in-process net, which does not depend on a clock at all: the titles
    /// sweep writes a title and rescans to verify its own work, and those two
    /// steps can be closer together than any filesystem timestamp resolves.
    #[test]
    fn opencode_scan_reads_session_v2_and_skips_child_sessions() {
        // Regression lock, 2026-08-29: the v2 preview writes new sessions to
        // `session_v2` and stops writing the v1-era `session` table, so the
        // v1-only reader served 3 of 11 sessions and the cwd tree lost the
        // rest. The reader must prefer `session_v2`, read its MILLISECOND
        // timestamps as ms, and project peer sessions only (`parent_id` marks
        // opencode's child/sub-agent primitive, not a resumable peer).
        let home = scratch("opencode-v2-scan");
        let oc = home.join(".local/share/opencode");
        std::fs::create_dir_all(&oc).unwrap();
        let conn = rusqlite::Connection::open(oc.join("opencode.db")).unwrap();
        conn.execute(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, title TEXT, \
             time_updated INTEGER, time_created INTEGER);",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, title, time_updated, time_created) VALUES \
             ('ses_v1legacy0000000000000001', '/home/user/proj', 'legacy', 1700000000, \
             1699999999);",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, project_id TEXT, parent_id TEXT, \
             directory TEXT, title TEXT, time_updated INTEGER, time_created INTEGER);",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2 (id, project_id, parent_id, directory, title, time_updated, \
             time_created) VALUES \
             ('ses_tab0000000000000000001', 'p1', NULL, '/home/user/proj', \
             'peer session', 1787984001574, 1787937064362), \
             ('ses_child00000000000000001', 'p1', 'ses_tab0000000000000000001', \
             '/home/user/proj', 'child run', 1787984002574, 1787937065362);",
            [],
        )
        .unwrap();

        let mut out = Vec::new();
        scan_opencode_sessions(&home, &mut out);
        let ids: Vec<&str> = out.iter().map(|r| r.session_id.as_str()).collect();
        assert!(
            ids.contains(&"ses_tab0000000000000000001"),
            "the v2 peer session must be projected, got {ids:?}"
        );
        assert!(
            !ids.contains(&"ses_child00000000000000001"),
            "child (sub-agent) sessions are not durable peers"
        );
        assert!(
            !ids.contains(&"ses_v1legacy0000000000000001"),
            "when session_v2 exists it is the authority; the stale v1 table must not \
             resurrect rows the service migrated"
        );
        // v2 timestamps are MILLISECONDS: recency must not be inflated x1000.
        let peer = out
            .iter()
            .find(|r| r.session_id == "ses_tab0000000000000000001")
            .unwrap();
        assert_eq!(peer.modified_epoch_ms, 1787984001574);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_title_write_drops_the_memo_in_this_process() {
        let home = scratch("invalidate");
        std::fs::create_dir_all(home.join(".yggterm")).unwrap();
        {
            let mut memo = super::durable_scan_memo()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            memo.generation = Some((1, 1));
            memo.rows.insert(
                PathBuf::from("/nonexistent/session.jsonl"),
                (
                    (1, 1),
                    StartpageDurableRow {
                        session_id: "aaaa".to_string(),
                        cwd: "/tmp".to_string(),
                        title: None,
                        generated_title: None,
                        effective_title: None,
                        detail: None,
                        kind: crate::SessionKind::Codex,
                        modified_epoch_ms: 0,
                        storage_path: "/nonexistent/session.jsonl".to_string(),
                        display_path: "codex://aaaa".to_string(),
                    },
                ),
            );
        }

        let store = crate::SessionTitleStore::open(&home.join(".yggterm")).unwrap();
        store
            .put_manual_title("aaaa", "/tmp", "renamed by the sweep")
            .unwrap();

        let memo = super::durable_scan_memo()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            memo.rows.is_empty() && memo.generation.is_none(),
            "a title write must drop the memo outright, so the next scan cannot \
             report that the write did not land"
        );
        drop(memo);
        let _ = std::fs::remove_dir_all(&home);
    }
}
