//! THE per-profile browsing journal — `~/.yggterm/web-profiles/<p>/history.jsonl`.
//!
//! One `{ts_ms, url, title}` object per visit, one line each. The omnibox
//! suggestions, the single-page history viewer, `collection add-from-history`
//! and the browser import all answer questions about this one file, and a
//! second reader (or a second writer) is how they would come to disagree about
//! what was visited — so both halves live here, in the crate they all link,
//! rather than in the GUI that happens to write it.
//!
//! # The invariant: lines are in VISIT ORDER
//!
//! Live browsing keeps that for free, because time moves forward. Every reader
//! leans on it: [`parse_web_history`] walks the file BACKWARDS and treats the
//! last line as the most recent thing that happened, stopping at a cap.
//!
//! That invariant is why the browser import does not get its own writer.
//! Appending a decade of imported visits to the end of the file would leave
//! every date correct and still make the history page show nothing but pages
//! from 2016 — the reverse walk would spend its whole cap on them. So there are
//! two writers, and the difference between them is not performance:
//!
//! * [`append_web_visit`] — the ordinary path: one page load, one line, no read
//!   of the file at all. This runs on every navigation, so it stays O(1) in the
//!   size of a decade of history.
//! * [`merge_web_visits`] — the IMPORT path. Dedupes on `(url, ts_ms)`, and when
//!   the incoming visits are older than what is already there, merges them into
//!   position instead of appending. Existing lines are never reordered and never
//!   re-serialised: they are carried through verbatim, so a line this build
//!   cannot parse survives an import the same way it survives everything else.
//!
//! Only the import reads the whole file, and only the import can afford to: it
//! is a one-off the user asked for, not something that happens behind a click.
//! `merge_web_visits` still degrades to a plain append whenever the batch is not
//! older than the file, which is the common case for a re-import.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::web_profile::{web_profile_dir, web_profile_dir_in};

/// The history file's name inside a profile jar.
pub const WEB_HISTORY_FILE: &str = "history.jsonl";

/// A visited page — the record every reader and both writers speak.
///
/// `ts_ms` is Unix milliseconds, which is the ONE time base this file has and
/// the whole reason the browser importers convert rather than copy: Chromium
/// counts microseconds from 1601 and Firefox from 1970.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WebHistoryEntry {
    pub ts_ms: u64,
    pub url: String,
    pub title: String,
}

impl WebHistoryEntry {
    pub fn new(ts_ms: u64, url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            ts_ms,
            url: url.into(),
            title: title.into(),
        }
    }

    /// The line as it is written. One `json!` shape for the GUI's page observer
    /// and for an import, so an imported line and a browsed line are
    /// indistinguishable in the file.
    pub fn to_jsonl_line(&self) -> String {
        json!({"ts_ms": self.ts_ms, "url": self.url, "title": self.title}).to_string()
    }

    /// Parse one line. `None` for anything that is not a visit record — the
    /// writers keep such lines verbatim rather than dropping them.
    pub fn from_jsonl_line(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let url = value.get("url")?.as_str()?;
        Some(Self {
            ts_ms: value.get("ts_ms").and_then(Value::as_u64).unwrap_or(0),
            url: url.to_string(),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// Whether a URL is a PAGE — THE rule for what gets journalled at all, and for
/// what may come back as a restored tab.
///
/// `chrome://`, `about:`, `file://`, `data:` and `javascript:` are not browsing
/// history: an internal `data:` page (the history viewer itself) must never be
/// journalled as a visit, and a browser's own database is full of rows that
/// would fill the user's timeline with links that go nowhere.
pub fn web_history_url_is_page(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
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
        let Some(entry) = WebHistoryEntry::from_jsonl_line(line) else {
            continue;
        };
        if !seen.insert(entry.url.clone()) {
            continue;
        }
        out.push(entry);
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

/// EVERY visit in the journal, in file order, undeduped and uncapped — the
/// question an import asks, and the only one that needs the whole file.
///
/// Distinct from [`read_web_history_file`], which answers the viewer's question
/// (newest first, one row per URL, capped). Two questions, one file, no second
/// parser: both build [`WebHistoryEntry`] the same way.
pub fn read_web_visits(path: &Path) -> Vec<WebHistoryEntry> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(WebHistoryEntry::from_jsonl_line)
        .collect()
}

/// What a write did. Every number here is a fact the import report quotes, and
/// `appended == 0 && !rewrote` is what idempotence looks like from outside.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct HistoryWriteReport {
    /// Visits actually written.
    pub appended: usize,
    /// Visits already in the file at the same `(url, ts_ms)`.
    pub duplicates: usize,
    /// Visits refused because their URL is not a page.
    pub skipped_not_page: usize,
    /// True when the file had to be rewritten to keep visit order — i.e. the
    /// incoming visits were older than what was already there.
    pub rewrote: bool,
}

/// Journal ONE visit — the live path, called on every navigation.
///
/// Deliberately does not read the file: dedupe is the import's problem, and a
/// page load must not pay for the size of the history behind it. Returns
/// whether a line was written (a non-page is silently not journalled).
pub fn append_web_visit(path: &Path, visit: &WebHistoryEntry) -> std::io::Result<bool> {
    if !web_history_url_is_page(&visit.url) {
        return Ok(false);
    }
    append_lines(path, std::slice::from_ref(visit))?;
    Ok(true)
}

/// Append already-planned lines. Private: every public writer decides what
/// belongs in the file before reaching here.
fn append_lines(path: &Path, visits: &[WebHistoryEntry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for visit in visits {
        writeln!(file, "{}", visit.to_jsonl_line())?;
    }
    Ok(())
}

/// Write visits into the journal IN ORDER — the import path.
///
/// Pure append when every incoming visit is at least as new as the file's
/// newest line (the common case for re-importing a profile that only grew).
/// Otherwise the file is rebuilt with the new lines merged into position:
/// existing lines are carried through byte-for-byte in their original order,
/// including ones this build cannot parse, and the replacement is swapped in
/// with a rename.
///
/// ⚠ The concurrent-append race is real and is narrowed, not eliminated: the
/// GUI's page observer opens the file per visit, so a page loaded between the
/// read and the rename would land on the old inode. The rewrite therefore
/// re-reads the source just before the swap and carries any bytes that
/// appeared. A visit written inside the microseconds after that re-read is
/// still lost; the honest fix is a single writer, which nothing owns today.
pub fn merge_web_visits(
    path: &Path,
    visits: &[WebHistoryEntry],
) -> std::io::Result<HistoryWriteReport> {
    let (fresh, mut report) = plan_write(path, visits);
    if fresh.is_empty() {
        return Ok(report);
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let newest_existing = existing
        .lines()
        .filter_map(WebHistoryEntry::from_jsonl_line)
        .map(|visit| visit.ts_ms)
        .max();
    // Appending keeps visit order whenever the batch is not older than the file.
    if newest_existing.is_none_or(|newest| fresh[0].ts_ms >= newest) {
        append_lines(path, &fresh)?;
        report.appended = fresh.len();
        return Ok(report);
    }

    let mut merged = String::with_capacity(existing.len() + fresh.len() * 128);
    let mut pending = fresh.iter().peekable();
    // An unparseable line inherits the timestamp of the line before it, so it
    // keeps its place next to the record it belongs to instead of migrating to
    // the top of the file.
    let mut carried_ts = 0u64;
    for line in existing.lines() {
        if line.trim().is_empty() {
            merged.push('\n');
            continue;
        }
        if let Some(visit) = WebHistoryEntry::from_jsonl_line(line) {
            carried_ts = visit.ts_ms;
        }
        while pending.peek().is_some_and(|next| next.ts_ms <= carried_ts) {
            let next = pending.next().expect("peeked");
            merged.push_str(&next.to_jsonl_line());
            merged.push('\n');
        }
        merged.push_str(line);
        merged.push('\n');
    }
    for remaining in pending {
        merged.push_str(&remaining.to_jsonl_line());
        merged.push('\n');
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.import-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| WEB_HISTORY_FILE.to_string()),
        std::process::id()
    ));
    std::fs::write(&temp, merged.as_bytes())?;
    // Carry anything the browser wrote while we were merging.
    let after = std::fs::read_to_string(path).unwrap_or_default();
    if after.len() > existing.len() && after.starts_with(&existing) {
        let mut file = std::fs::OpenOptions::new().append(true).open(&temp)?;
        file.write_all(after[existing.len()..].as_bytes())?;
    }
    std::fs::rename(&temp, path)?;
    report.appended = fresh.len();
    report.rewrote = true;
    Ok(report)
}

/// The import's planner: filter to pages, drop what the file already holds,
/// drop repeats within the batch, and sort ascending.
///
/// This is the O(file) step, and it is why only [`merge_web_visits`] calls it.
fn plan_write(
    path: &Path,
    visits: &[WebHistoryEntry],
) -> (Vec<WebHistoryEntry>, HistoryWriteReport) {
    let mut report = HistoryWriteReport::default();
    let mut seen: HashSet<(String, u64)> = read_web_visits(path)
        .into_iter()
        .map(|visit| (visit.url, visit.ts_ms))
        .collect();
    let mut fresh = Vec::new();
    for visit in visits {
        if !web_history_url_is_page(&visit.url) {
            report.skipped_not_page += 1;
            continue;
        }
        if !seen.insert((visit.url.clone(), visit.ts_ms)) {
            report.duplicates += 1;
            continue;
        }
        fresh.push(visit.clone());
    }
    fresh.sort();
    (fresh, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = r#"{"ts_ms":1000,"url":"https://a.example/","title":"A"}
{"ts_ms":2000,"url":"https://b.example/","title":"B"}
{"ts_ms":3000,"url":"https://a.example/","title":"A again"}
"#;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yggterm-web-history-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }
        fn journal(&self) -> PathBuf {
            self.0.join(WEB_HISTORY_FILE)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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

    /// The viewer's read and the import's read are two questions about one
    /// file: newest-first-and-deduped versus every visit in order. Both must
    /// come out of the same parse.
    #[test]
    fn the_import_reads_every_visit_while_the_viewer_reads_one_row_per_url() {
        let scratch = Scratch::new("two-questions");
        let journal = scratch.journal();
        std::fs::write(&journal, RAW).expect("seed");
        assert_eq!(read_web_visits(&journal).len(), 3, "three visits happened");
        assert_eq!(
            read_web_history_file(&journal, 100).len(),
            2,
            "two distinct pages were visited"
        );
    }

    #[test]
    fn a_line_written_here_is_the_same_shape_the_browser_writes() {
        let visit = WebHistoryEntry::new(1_622_505_600_000, "https://example.org/a", "A");
        let line = visit.to_jsonl_line();
        let parsed: Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(parsed["ts_ms"].as_u64(), Some(1_622_505_600_000));
        assert_eq!(parsed["url"].as_str(), Some("https://example.org/a"));
        assert_eq!(parsed["title"].as_str(), Some("A"));
        assert_eq!(WebHistoryEntry::from_jsonl_line(&line), Some(visit));
    }

    #[test]
    fn only_pages_are_journalled() {
        for page in ["http://example.org", "https://example.org/x?y=1"] {
            assert!(web_history_url_is_page(page));
        }
        for other in [
            "chrome://settings",
            "about:blank",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "",
        ] {
            assert!(!web_history_url_is_page(other), "{other} is not a page");
        }
    }

    /// The live writer is O(1) in the size of the history: it appends and does
    /// not read. This is the assertion that fails if dedupe ever creeps into
    /// the page-load path — a decade-deep journal would then be re-read on
    /// every navigation.
    #[test]
    fn the_live_writer_appends_without_reading_the_file() {
        let scratch = Scratch::new("live-writer");
        let journal = scratch.journal();
        assert!(
            append_web_visit(
                &journal,
                &WebHistoryEntry::new(1_000, "https://example.org/a", "A")
            )
            .expect("write")
        );
        assert!(
            !append_web_visit(
                &journal,
                &WebHistoryEntry::new(2_000, "chrome://settings", "S")
            )
            .expect("write"),
            "a non-page is still refused"
        );
        assert_eq!(read_web_visits(&journal).len(), 1);
    }

    /// ⚠ THE ORDER LOCK. Every reader walks this file BACKWARDS and treats the
    /// last line as the most recent visit, with a cap. An import that appended
    /// a decade of old visits would push the user's actual browsing off the end
    /// of that walk — every date correct, the page useless. This is the
    /// assertion that fails if the merge ever becomes a plain append.
    #[test]
    fn older_visits_merge_into_position_instead_of_landing_at_the_end() {
        let scratch = Scratch::new("order");
        let journal = scratch.journal();
        // Seeded the way live browsing writes it: one visit at a time.
        for visit in [
            WebHistoryEntry::new(3_000, "https://example.org/recent-1", "R1"),
            WebHistoryEntry::new(4_000, "https://example.org/recent-2", "R2"),
        ] {
            assert!(append_web_visit(&journal, &visit).expect("seed"));
        }

        let report = merge_web_visits(
            &journal,
            &[
                WebHistoryEntry::new(1_000, "https://example.org/old-1", "O1"),
                WebHistoryEntry::new(3_500, "https://example.org/middle", "M"),
            ],
        )
        .expect("merge");
        assert_eq!(report.appended, 2);
        assert!(report.rewrote, "an older batch cannot be a plain append");

        let order: Vec<u64> = read_web_visits(&journal)
            .iter()
            .map(|visit| visit.ts_ms)
            .collect();
        assert_eq!(
            order,
            vec![1_000, 3_000, 3_500, 4_000],
            "the file must stay in visit order"
        );
        // The reader's rule: the LAST line is the newest thing that happened.
        assert_eq!(
            read_web_visits(&journal).last().map(|v| v.url.clone()),
            Some("https://example.org/recent-2".to_string())
        );
        assert_eq!(
            parse_web_history(&std::fs::read_to_string(&journal).expect("read"), 1)[0].url,
            "https://example.org/recent-2",
            "and the viewer still opens on what the user did most recently"
        );
    }

    #[test]
    fn a_batch_newer_than_the_file_stays_a_pure_append() {
        let scratch = Scratch::new("append-fast-path");
        let journal = scratch.journal();
        append_web_visit(
            &journal,
            &WebHistoryEntry::new(1_000, "https://example.org/a", "A"),
        )
        .expect("seed");
        let report = merge_web_visits(
            &journal,
            &[WebHistoryEntry::new(2_000, "https://example.org/b", "B")],
        )
        .expect("merge");
        assert_eq!(report.appended, 1);
        assert!(
            !report.rewrote,
            "a batch that is newer must not rewrite the file"
        );
    }

    /// ⚠ THE IDEMPOTENCE LOCK, at the journal's own level. The same visits
    /// written twice must leave the file byte-identical — not "deduped on
    /// read", not "nearly the same", identical.
    #[test]
    fn writing_the_same_visits_twice_changes_nothing() {
        let scratch = Scratch::new("idempotent");
        let journal = scratch.journal();
        let batch = [
            WebHistoryEntry::new(1_000, "https://example.org/a", "A"),
            WebHistoryEntry::new(2_000, "https://example.org/b", "B"),
            // Same URL, different instant — a genuinely different visit.
            WebHistoryEntry::new(3_000, "https://example.org/a", "A again"),
        ];
        let first = merge_web_visits(&journal, &batch).expect("first");
        assert_eq!(first.appended, 3);
        let after_first = std::fs::read_to_string(&journal).expect("read");

        let second = merge_web_visits(&journal, &batch).expect("second");
        assert_eq!(second.appended, 0);
        assert_eq!(second.duplicates, 3);
        assert!(!second.rewrote, "a no-op import must not touch the file");
        assert_eq!(
            std::fs::read_to_string(&journal).expect("read"),
            after_first,
            "a re-import must leave the journal byte-identical"
        );
    }

    #[test]
    fn a_repeat_inside_one_batch_is_written_once() {
        let scratch = Scratch::new("batch-dupe");
        let journal = scratch.journal();
        let report = merge_web_visits(
            &journal,
            &[
                WebHistoryEntry::new(1_000, "https://example.org/a", "A"),
                WebHistoryEntry::new(1_000, "https://example.org/a", "A"),
            ],
        )
        .expect("merge");
        assert_eq!(report.appended, 1);
        assert_eq!(report.duplicates, 1);
        assert_eq!(read_web_visits(&journal).len(), 1);
    }

    #[test]
    fn non_pages_never_reach_the_journal() {
        let scratch = Scratch::new("not-page");
        let journal = scratch.journal();
        let report = merge_web_visits(
            &journal,
            &[
                WebHistoryEntry::new(1_000, "chrome://settings", "Settings"),
                WebHistoryEntry::new(2_000, "https://example.org/a", "A"),
            ],
        )
        .expect("merge");
        assert_eq!(report.appended, 1);
        assert_eq!(report.skipped_not_page, 1);
        assert_eq!(read_web_visits(&journal).len(), 1);
    }

    /// A rewrite must not be an excuse to tidy the user's file: a line this
    /// build cannot parse stays, in its place.
    #[test]
    fn a_line_we_cannot_parse_survives_a_merge() {
        let scratch = Scratch::new("unparseable");
        let journal = scratch.journal();
        std::fs::write(
            &journal,
            "{\"ts_ms\":2000,\"url\":\"https://example.org/b\",\"title\":\"B\"}\n\
             not json at all, but it is the user's line\n",
        )
        .expect("seed");
        merge_web_visits(
            &journal,
            &[WebHistoryEntry::new(1_000, "https://example.org/a", "A")],
        )
        .expect("merge");
        let raw = std::fs::read_to_string(&journal).expect("read");
        assert!(
            raw.contains("not json at all, but it is the user's line"),
            "an unparseable line must survive: {raw}"
        );
        let lines: Vec<&str> = raw.lines().collect();
        assert!(
            lines[0].contains("/a") && lines[1].contains("/b"),
            "the merge must land the older visit first: {raw}"
        );
    }
}
