//! The host anomaly plane — detect, probe, surface.
//!
//! The [11.163] incident (the ownership ledger doubling itself to 111 GB
//! while every resume wrapper parsed it into a 150-290 GB-RSS process) was
//! invisible until the owner happened to look. This plane is the answer:
//! when a detector fires, the report ALWAYS lands in three places —
//!
//! 1. the trace (category `anomaly`), which `append_trace_event` dual-writes
//!    into ytrace, so Dash notebooks and any trace reader see the probe;
//! 2. `anomalies.jsonl` in the yggterm home — persistent, survives every
//!    process, capped;
//! 3. the daemon's status payload — the GUI already polls status on a tick,
//!    so unseen notices become toasts (Warning/Error tone) without any new
//!    push channel, and the GUI acks them by id.
//!
//! A report is DEDUPLICATED per kind inside [`ANOMALY_DEDUPE_MS`]: a runaway
//! detector must not write gigabytes of anomaly records about writing
//! gigabytes of records. The trace probe fires every time regardless — the
//! probe is the narrative log, the file is the notification queue.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use yggterm_core::append_trace_event;

pub const ANOMALIES_FILENAME: &str = "anomalies.jsonl";

/// Same-kind reports inside this window collapse into the standing unseen
/// record (its detail and first_seen stay the FIRST sighting's — the onset
/// is the fact a human needs).
pub const ANOMALY_DEDUPE_MS: u64 = 10 * 60 * 1000;

/// Unseen notices older than this no longer ride status: a two-day-old
/// toast about a problem that resolved itself is noise, and the trace
/// already keeps the full narrative forever.
pub const ANOMALY_STATUS_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// The file is a notification queue, not an archive — the trace is the
/// archive. Oldest SEEN records are dropped first when the cap is hit.
pub const ANOMALY_MAX_RECORDS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnomalyNotice {
    /// Stable within a host for one incident: `kind:first_seen_ms`. The GUI
    /// dedupes toasts on it; relays prefix the peer host.
    pub id: String,
    pub kind: String,
    /// `warning` or `error` — the tone a GUI toast carries.
    pub severity: String,
    pub title: String,
    pub detail: String,
    pub first_seen_ms: u64,
    pub seen: bool,
    /// Set when a daemon relayed this from a peer machine's status instead
    /// of detecting it locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relayed_from: Option<String>,
}

pub fn anomalies_path(home_dir: &Path) -> PathBuf {
    home_dir.join(ANOMALIES_FILENAME)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Load every record, oldest first. A missing file is an empty queue; a
/// corrupt line is skipped, never fatal — the queue must not take the
/// notifier down.
fn load_all(home_dir: &Path) -> Vec<AnomalyNotice> {
    let Ok(raw) = fs::read_to_string(anomalies_path(home_dir)) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| serde_json::from_str::<AnomalyNotice>(line).ok())
        .collect()
}

fn save_all(home_dir: &Path, records: &[AnomalyNotice]) {
    let mut body = String::new();
    for record in records {
        if let Ok(line) = serde_json::to_string(record) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    // Write-then-rename — the ledger's own pattern: a reader never sees a
    // torn queue.
    let path = anomalies_path(home_dir);
    let tmp = path.with_extension(format!("jsonl.tmp.{}", std::process::id()));
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// Report one anomaly. Returns the standing notice when a new record (or a
/// dedupe refresh) was written; `None` when the report collapsed into an
/// existing unseen record inside the dedupe window. The trace probe fires
/// either way.
pub fn report_anomaly(
    home_dir: &Path,
    kind: &str,
    severity: &str,
    title: &str,
    detail: Value,
) -> Option<AnomalyNotice> {
    let now = now_ms();
    let detail_str = shorten_detail(&detail.to_string());
    append_trace_event(
        home_dir,
        "server",
        "anomaly",
        kind.to_string(),
        serde_json::json!({
            "severity": severity,
            "title": title,
            "detail": detail,
        }),
    );
    let mut records = load_all(home_dir);
    if let Some(standing) = records.iter_mut().rev().find(|record| {
        record.kind == kind && !record.seen && now.saturating_sub(record.first_seen_ms) < ANOMALY_DEDUPE_MS
    }) {
        return None;
    }
    let notice = AnomalyNotice {
        id: format!("{kind}:{now}"),
        kind: kind.to_string(),
        severity: severity.to_string(),
        title: title.to_string(),
        detail: detail_str,
        first_seen_ms: now,
        seen: false,
        relayed_from: None,
    };
    records.push(notice.clone());
    // Cap: drop oldest seen first, then oldest unseen.
    while records.len() > ANOMALY_MAX_RECORDS {
        let drop_index = records
            .iter()
            .position(|r| r.seen)
            .unwrap_or(0);
        records.remove(drop_index);
    }
    save_all(home_dir, &records);
    Some(notice)
}

/// Unseen notices for the status wire: newest first, capped, inside the TTL.
pub fn recent_anomalies(home_dir: &Path, cap: usize) -> Vec<AnomalyNotice> {
    let now = now_ms();
    let mut notices: Vec<AnomalyNotice> = load_all(home_dir)
        .into_iter()
        .filter(|record| !record.seen && now.saturating_sub(record.first_seen_ms) <= ANOMALY_STATUS_TTL_MS)
        .collect();
    notices.reverse();
    notices.truncate(cap);
    notices
}

/// Ack notices by id (the GUI calls this after toasting). Returns how many
/// records flipped.
pub fn mark_anomalies_seen(home_dir: &Path, ids: &[String]) -> usize {
    if ids.is_empty() {
        return 0;
    }
    let mut records = load_all(home_dir);
    let mut flipped = 0;
    for record in records.iter_mut() {
        if !record.seen && ids.iter().any(|id| *id == record.id) {
            record.seen = true;
            flipped += 1;
        }
    }
    if flipped > 0 {
        save_all(home_dir, &records);
    }
    flipped
}

fn shorten_detail(detail: &str) -> String {
    if detail.len() <= 400 {
        detail.to_string()
    } else {
        let mut cut = 400;
        while cut > 0 && !detail.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &detail[..cut])
    }
}

/// Read this process's own RSS from /proc — Linux only, `None` elsewhere.
pub fn own_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
        line.split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
            .map(|kb| kb * 1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The thresholds the daemon watchdog enforces, as a pure predicate so the
/// hysteresis is testable: fire on a rising crossing of HIGH, re-arm only
/// under LOW.
pub const DAEMON_RSS_HIGH_BYTES: u64 = 1500 * 1024 * 1024;
pub const DAEMON_RSS_LOW_BYTES: u64 = 1000 * 1024 * 1024;
pub const LEDGER_GROWTH_WARN_BYTES: u64 = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zcode-host-anomaly-{}-{}-{tag}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_report_lands_in_the_file_and_dedupes_within_the_window() {
        let home = scratch_home("report");
        let first = report_anomaly(
            &home,
            "ledger_oversize",
            "error",
            "Ledger runaway",
            json!({"bytes": 5_000_000}),
        );
        assert!(first.is_some());
        // Same kind again inside the window collapses; a DIFFERENT kind lands.
        assert!(report_anomaly(&home, "ledger_oversize", "error", "Ledger runaway", json!({})).is_none());
        assert!(
            report_anomaly(&home, "daemon_rss_high", "error", "RSS", json!({})).is_some(),
            "a different kind must not be swallowed by the dedupe window"
        );
        assert_eq!(load_all(&home).len(), 2);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn status_carries_only_unseen_recent_and_ack_flips_them() {
        let home = scratch_home("status");
        report_anomaly(&home, "ledger_oversize", "error", "t", json!({}));
        report_anomaly(&home, "ledger_growth", "warning", "t", json!({}));
        let notices = recent_anomalies(&home, 20);
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].kind, "ledger_growth", "newest first");
        let ids: Vec<String> = notices.iter().map(|n| n.id.clone()).collect();
        assert_eq!(mark_anomalies_seen(&home, &ids), 2);
        assert!(recent_anomalies(&home, 20).is_empty(), "acked notices leave status");
        // An ack that names nothing flips nothing and costs no rewrite.
        assert_eq!(mark_anomalies_seen(&home, &["nope:1".to_string()]), 0);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_corrupt_queue_still_serves_the_healthy_lines() {
        let home = scratch_home("corrupt");
        let healthy = AnomalyNotice {
            id: format!("k:{}", now_ms()),
            kind: "k".to_string(),
            severity: "warning".to_string(),
            title: "t".to_string(),
            detail: String::new(),
            first_seen_ms: now_ms(),
            seen: false,
            relayed_from: None,
        };
        let line = serde_json::to_string(&healthy).unwrap();
        fs::write(anomalies_path(&home), format!("{{not json\n{line}\n")).unwrap();
        assert_eq!(recent_anomalies(&home, 20).len(), 1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_cap_drops_seen_records_before_unseen() {
        let home = scratch_home("cap");
        // Fill the queue to its cap with distinct kinds (distinct kinds never
        // dedupe), ack the OLDEST, then push one more: the acked record is
        // the one that must go, not a fresh unseen notice.
        let mut first_id = String::new();
        for i in 0..ANOMALY_MAX_RECORDS {
            let notice = report_anomaly(&home, &format!("kind_{i}"), "warning", "t", json!({}))
                .unwrap_or_else(|| panic!("report {i} collapsed"));
            if i == 0 {
                first_id = notice.id;
            }
        }
        assert_eq!(load_all(&home).len(), ANOMALY_MAX_RECORDS);
        assert_eq!(mark_anomalies_seen(&home, &[first_id]), 1);
        report_anomaly(&home, "kind_new", "warning", "t", json!({})).unwrap();
        let records = load_all(&home);
        assert_eq!(records.len(), ANOMALY_MAX_RECORDS, "the cap holds");
        assert!(
            records.iter().all(|r| r.kind != "kind_0"),
            "the seen record is the one that goes"
        );
        assert!(records.iter().any(|r| r.kind == "kind_new"), "the fresh notice survives");
        let _ = fs::remove_dir_all(&home);
    }
}
