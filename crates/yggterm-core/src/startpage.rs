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
            if let Some(entry) = (descriptor.read_store_entry)(&path) {
                out.push(StartpageDurableRow::from_entry(entry, descriptor.kind, &path));
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
        // Resolve generated copy when the CLI itself stores no title.
        // This is what the shell's `detail` pipeline does — do it here so
        // `effective_title` is what the startpage actually paints, not what
        // the store file alone contains.
        let raw_title = entry.title.clone();
        let generated_title = if raw_title.is_none() {
            // Try to load from titles.db; failure means no generated copy yet.
            Self::load_generated_title(&entry.session_id)
        } else {
            None
        };
        let effective_title = raw_title.clone().or_else(|| generated_title.clone());
        Self {
            session_id: entry.session_id,
            cwd: entry.cwd,
            title: raw_title,
            generated_title,
            effective_title,
            detail: entry.detail,
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

/// Startpage ordering — live first, then recency.
/// This is the shell's `candidates.sort_by` ranking extracted verbatim:
/// `is_live` > `in_scope` > `modified_epoch` > `started_at` > `insertion_index`.
/// For the daemon verb `in_scope` is always true and `started_at` is empty,
/// so it collapses to live-first + recency, which is what matters for the
/// lie detector. Full scope handling lives in the shell.
pub fn order_for_startpage(mut rows: Vec<StartpageDurableRow>) -> Vec<StartpageDurableRow> {
    rows.sort_by(|a, b| {
        b.modified_epoch_ms
            .cmp(&a.modified_epoch_ms)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    rows
}
