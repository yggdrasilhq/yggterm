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

/// Whether a session file is noise (empty or no agent turn) and should be deleted on sight.
/// Guard: files younger than 60s are kept to avoid deleting a CLI mid-write.
/// Mirrors the spec heading 8 (2026-08-17) and is called from both startpage and cwd tree.
pub fn is_noise_session_file(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() < 20 { return true }
    let path_str = path.display().to_string();
    // Muse: a session with no prompts is noise even if the bootstrap is >7k.
    // Query session-index.db for prompt_count; a zero-prompt session that is
    // older than the 60s guard is the exact "New session" placeholder the
    // detector used to keep via the >5000 bypass.
    if path_str.contains("muse/sessions") {
        // Muse's JSONL is NOT codex-shaped, so the generic extract_tail_context
        // below would misclassify every real Muse session as noise (empty context
        // → large-file bypass removed). Decide SOLELY from the DB when the DB
        // has an entry — that is the source `muse resume` lists from.
        let mut muse_db_decided = false;
        let mut muse_is_noise: bool = false;
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
                                    muse_db_decided = true;
                                    let prompt_count: i64 = row.get(0).unwrap_or(1);
                                    let title: String = row.get(1).unwrap_or_default();
                                    let title_lower = title.trim().to_ascii_lowercase();
                                    if prompt_count == 0
                                        && (title_lower == "new session"
                                            || title_lower.is_empty()
                                            || title_lower == "new muse code session")
                                    {
                                        muse_is_noise = true;
                                    } else if prompt_count <= 1 && title_lower.len() <= 8 {
                                        let lower = title_lower.as_str();
                                        if matches!(lower, "hi" | "hello" | "hey" | "/" | "/status" | "/help" | "/context" | "/clear" | "test") {
                                            muse_is_noise = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if muse_db_decided {
            return muse_is_noise;
        }
        // No DB entry (old file or DB absent) — fall through to generic check.
        // Do not early-return false on size alone; the generic tail will decide.
    }
    // Antigravity: its JSONL is payload_type USER_INPUT or conversation_summaries.db,
    // not Codex-shaped. Using the generic Codex tail would mark every real AGY
    // session as noise. Prefer transcript presence when the DB is absent.
    if path_str.contains("antigravity") || path_str.contains(".gemini") {
        if let Ok(content) = std::fs::read_to_string(path) {
            // AGY transcript with a USER_INPUT and a non-whitespace prompt is not noise.
            if content.contains("USER_INPUT") {
                // Find a prompt line with substantive text (>10 chars after trimming tags)
                for line in content.lines().take(32) {
                    if line.contains("USER_INPUT") {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(c) = value.get("content").and_then(|v| v.as_str()) {
                                let prompt = if let Some(s) = c.find("<USER_REQUEST>") {
                                    let after = &c[s + "<USER_REQUEST>".len()..];
                                    after.split("</USER_REQUEST>").next().unwrap_or(after)
                                } else {
                                    c
                                };
                                if prompt.trim().len() >= 10 {
                                    return false;
                                }
                            }
                        }
                    }
                }
                // Has USER_INPUT but no extractable prompt — still not noise (has turn)
                return false;
            }
            if content.len() > 500 && content.contains("conversation_id") {
                return false;
            }
        }
        // For AGY .db files, stat-based size already handled; fall through to false
        // only if file is truly empty. A .db with tables is not noise.
        if path.extension().and_then(|e| e.to_str()) == Some("db") {
            return false;
        }
    }
    // Generic tail-context check: if we can extract substantive context (>=20 chars
    // and not low-signal), this is not noise. Otherwise it is.
    if let Ok(ctx) = crate::titles::extract_tail_context(path) {
        let trimmed = ctx.trim();
        if trimmed.len() >= 20 && !crate::looks_like_low_signal_generated_copy(trimmed) {
            // Additionally, a context that is only a single "hi" or slash command
            // is still noise — treat ultra-short prompt after stripping header.
            let lower = trimmed.to_ascii_lowercase();
            // If the only USER line is a single word hi/slash, the context will be just that.
            if lower == "hi" || lower == "hello" || lower == "hey" || lower.starts_with("user: hi") || lower.contains("user: /status") {
                return true;
            }
            return false;
        }
        // ctx exists but is empty/short/low-signal => noise candidate, continue to final verdict
    } else if let Ok(content) = std::fs::read_to_string(path) {
        // Fallback for unreadable tail: if file contains agent marker and is substantial, keep.
        // But a small file (<500) with no substantive marker is noise.
        if content.len() > 500 && content.to_lowercase().contains("agent") {
            return false;
        }
    }
    // No size-based keep-alive: a 7k muse bootstrap with 12 lines and no agent turn
    // must be deleted. Large-file bypass was the bug.
    true
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

/// Walk every registered agent CLI's store globs under `home` and return
/// one entry per readable session file, using the descriptor's own
/// `read_store_entry` so title/cwd semantics stay single-sourced.
pub fn scan_all_durable_sessions(home: &Path) -> Vec<StartpageDurableRow> {
    let mut out = Vec::new();
    let mut seen_paths = HashSet::<PathBuf>::new();
    for descriptor in AGENT_CLIS {
        if descriptor.session_store_globs.is_empty() {
            continue;
        }
        for root in descriptor.store_roots_absolute(home) {
            // `store_roots_absolute` returns the directory *above* the glob's
            // literal prefix (e.g. `~/.codex`), but walk from the parent that
            // actually exists — `collect` will filter by `store_path_is_session_file`.
            let walk_root = root;
            if !walk_root.exists() {
                continue;
            }
            walk_and_collect(descriptor, &walk_root, &mut out, &mut seen_paths);
        }
    }
    // Add Codex-family legacy roots that live under `~/.codex/sessions` with
    // date subdirectories — already covered by AGENT_CLIS walk, but keep as
    // fallback if walk missed due to permission.
    out
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
            // Noise deletion (heading 8) — mtime guard 60s, then DELETE file + title store entry.
            if is_noise_session_file(&path) {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(elapsed) = mtime.elapsed() {
                            if elapsed.as_secs() < 60 {
                                // Too young — keep for now, skip push but don't delete.
                                continue;
                            }
                        }
                    }
                }
                let _ = std::fs::remove_file(&path);
                // Best-effort: remove any generated title for this id.
                if let Some(entry) = (descriptor.read_store_entry)(&path) {
                    if let Some(home) = dirs::home_dir() {
                        if let Ok(store) = crate::SessionTitleStore::open(&home.join(".yggterm")) {
                            let _ = store.delete_title(&entry.session_id);
                        }
                        if let Ok(store) = crate::SessionTitleStore::open(&home) {
                            let _ = store.delete_title(&entry.session_id);
                        }
                    }
                } else if let Some(home) = dirs::home_dir() {
                    // Fallback: derive id from filename/parent for Muse etc.
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(store) = crate::SessionTitleStore::open(&home.join(".yggterm")) {
                            let _ = store.delete_title(stem);
                        }
                    }
                    if let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
                        if let Ok(store) = crate::SessionTitleStore::open(&home.join(".yggterm")) {
                            let _ = store.delete_title(parent);
                        }
                    }
                }
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
            filtered_generated.clone().or_else(|| filtered_raw.clone())
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
