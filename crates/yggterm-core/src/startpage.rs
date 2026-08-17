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
    // Muse sessions use a different JSON schema; extract_tail_context is Codex/Claude-centric
    // and returns empty for them. For Muse, a large file with payload_type is not noise.
    if path_str.contains("muse/sessions") && meta.len() > 5000 {
        // Quick heuristic: Muse files contain payload_type and are not noise if large.
        // We avoid reading the whole 50M file: just check tail context fallback.
        if let Ok(ctx) = crate::titles::extract_tail_context(path) {
            if ctx.trim().len() >= 20 {
                return false;
            }
        }
        // If tail context is empty, Muse file is still not noise if it has session markers.
        // Check a small sample from the file head instead of full read.
        if let Ok(file) = std::fs::File::open(path) {
            use std::io::{BufRead, BufReader};
            let mut reader = BufReader::new(file);
            let mut buf = String::new();
            for _ in 0..5 {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 { break; }
                buf.push_str(&line);
                if buf.len() > 2000 { break; }
            }
            if buf.contains("payload_type") && buf.contains("session_id") {
                return false;
            }
        }
        if meta.len() > 10000 {
            return false;
        }
    }
    if let Ok(ctx) = crate::titles::extract_tail_context(path) {
        if ctx.trim().len() >= 20 {
            return false;
        }
    } else {
        if let Ok(content) = std::fs::read_to_string(path) {
            if content.to_lowercase().contains("agent") || content.len() > 200 {
                return false;
            }
        }
    }
    // Large files are not noise even if tail context is thin
    if meta.len() > 5000 {
        return false;
    }
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
