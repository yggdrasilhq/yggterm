//! The per-profile browsing history file, read in exactly ONE place.
//!
//! `~/.yggterm/web-profiles/<profile>/history.jsonl` is append-only, one
//! `{ts_ms, url, title}` object per visit, written by the GUI's page observer.
//! The omnibox suggestions, the single-page history viewer and
//! `collection add-from-history` all answer questions about the same file, and
//! a second reader is how they would come to disagree about what was visited —
//! so the reader lives here, in the crate all three link, rather than in the
//! GUI that happens to write it.
//!
//! Nothing here writes: the append path stays with the observer that owns the
//! visit.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::web_profile::{web_profile_dir, web_profile_dir_in};

/// The history file's name inside a profile jar.
pub const WEB_HISTORY_FILE: &str = "history.jsonl";

/// A visited page, as every reader of the file needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebHistoryEntry {
    pub ts_ms: u64,
    pub url: String,
    pub title: String,
}

/// A profile's history file under `root`. `None` for the ephemeral profile —
/// temp browsing leaves no disk trace, which is a property of the profile, not
/// of the caller asking.
pub fn web_history_path_in(root: impl AsRef<Path>, profile: &str) -> Option<PathBuf> {
    Some(web_profile_dir_in(root, profile)?.join(WEB_HISTORY_FILE))
}

/// A profile's history file on this host.
pub fn web_history_path(profile: &str) -> Option<PathBuf> {
    Some(web_profile_dir(profile)?.join(WEB_HISTORY_FILE))
}

/// Parse a history file body newest-first, deduped by URL (keeping the most
/// recent visit), capped at `limit`.
///
/// Pure, so the ordering and the dedupe can be pinned without a jar on disk.
/// A line that is not JSON, or carries no `url`, is skipped rather than
/// failing the read: the file is append-only and a torn last line is a normal
/// state, not a corrupt history.
pub fn parse_web_history(raw: &str, limit: usize) -> Vec<WebHistoryEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for line in raw.lines().rev() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(url) = value.get("url").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(url.to_string()) {
            continue;
        }
        out.push(WebHistoryEntry {
            ts_ms: value.get("ts_ms").and_then(Value::as_u64).unwrap_or(0),
            url: url.to_string(),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Read a profile's history newest-first, deduped by URL, capped. A missing or
/// unreadable file is an EMPTY history, never an error: a profile that has not
/// browsed yet is the common case.
pub fn web_history_entries(profile: &str, limit: usize) -> Vec<WebHistoryEntry> {
    let Some(path) = web_history_path(profile) else {
        return Vec::new();
    };
    read_web_history_file(&path, limit)
}

/// The same read against an explicit path, for a scratch jar in a test.
pub fn read_web_history_file(path: &Path, limit: usize) -> Vec<WebHistoryEntry> {
    match std::fs::read_to_string(path) {
        Ok(raw) => parse_web_history(&raw, limit),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = r#"{"ts_ms":1000,"url":"https://a.example/","title":"A"}
{"ts_ms":2000,"url":"https://b.example/","title":"B"}
{"ts_ms":3000,"url":"https://a.example/","title":"A again"}
"#;

    #[test]
    fn history_reads_newest_first_and_keeps_the_most_recent_visit_of_a_url() {
        let entries = parse_web_history(RAW, 100);
        assert_eq!(entries.len(), 2, "the repeat visit is one row, not two");
        assert_eq!(entries[0].url, "https://a.example/");
        assert_eq!(entries[0].title, "A again", "the NEWEST visit wins");
        assert_eq!(entries[0].ts_ms, 3000);
        assert_eq!(entries[1].url, "https://b.example/");
    }

    #[test]
    fn a_torn_or_foreign_line_is_skipped_rather_than_failing_the_whole_read() {
        let raw = format!("{RAW}not json at all\n{{\"ts_ms\":4000}}\n");
        let entries = parse_web_history(&raw, 100);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url, "https://a.example/");
    }

    #[test]
    fn the_cap_bounds_the_read() {
        let entries = parse_web_history(RAW, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://a.example/");
    }

    #[test]
    fn the_ephemeral_profile_has_no_history_path_at_all() {
        assert_eq!(
            web_history_path_in("/tmp/scratch", crate::web_profile::WEB_PROFILE_TEMP),
            None,
            "temp browsing must leave no disk trace"
        );
        assert_eq!(
            web_history_path_in("/tmp/scratch", "work"),
            Some(PathBuf::from("/tmp/scratch/work/history.jsonl"))
        );
    }
}
