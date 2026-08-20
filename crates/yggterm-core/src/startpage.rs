//! Startpage ground-truth helpers — shared by shell rendering and `server startpage ls`.
//!
//! The startpage's `RECENT WORK` list used to be GUI-only (`yggterm-shell/src/shell.rs`
//! `start_page_recent_rows*`). A per-host `server startpage ls` that re-derives the same
//! list from the stores is the cheap lie detector the fleet needs: the GUI can be
//! asked what it showed, the daemon what it thinks the page should be, and a
//! Python oracle can walk the raw jsonls independently.

use std::collections::HashSet;
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
        if let Some(home) = dirs::home_dir() {
            let db_path = home.join(".local/share/muse/session-index.db");
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
                                        return true;
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

/// Walk every registered agent CLI's store globs under `home` and return
/// one entry per readable session file, using the descriptor's own
/// `read_store_entry` so title/cwd semantics stay single-sourced.
pub fn scan_all_durable_sessions(home: &Path) -> Vec<StartpageDurableRow> {
    let mut out = Vec::new();
    let mut seen_paths = HashSet::<PathBuf>::new();
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
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, directory, title, time_updated, time_created FROM session",
    ) else {
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
        // time_updated/created are seconds since epoch (INTEGER NOT NULL in schema)
        let epoch_ms = if time_updated > 0 {
            (time_updated as u128) * 1000
        } else if time_created > 0 {
            (time_created as u128) * 1000
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
        out.append(&mut walked);
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
            if let Some(entry) = (descriptor.read_store_entry)(&path) {
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
}
