use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Process-global gate for the app profiling system. Default ON to preserve the
/// pre-toggle always-on behavior; the daemon and GUI both push
/// `AppSettings.perf_profiling_enabled` here on startup and whenever settings change
/// (`set_perf_profiling_enabled`). When off, `append_perf_event` / `PerfSpan::finish`
/// are no-ops, so an instrumented hot path costs only an `Instant::now()` plus an
/// early-returning call — cheap enough to leave the spans compiled in permanently.
static PERF_PROFILING_ENABLED: AtomicBool = AtomicBool::new(true);

/// Update the process-global profiling gate (called from settings load / change).
pub fn set_perf_profiling_enabled(enabled: bool) {
    PERF_PROFILING_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether the app profiling system is currently recording. Callers that build an
/// expensive payload before recording should check this first to skip the work.
pub fn perf_profiling_enabled() -> bool {
    PERF_PROFILING_ENABLED.load(Ordering::Relaxed)
}

pub const PERF_TELEMETRY_FILENAME: &str = "perf-telemetry.jsonl";
pub const PERF_TELEMETRY_ROTATED_FILENAME: &str = "perf-telemetry.previous.jsonl";
pub const PERF_TELEMETRY_MAX_BYTES: u64 = 16 * 1024 * 1024;

pub fn perf_telemetry_path(home: &Path) -> PathBuf {
    home.join(PERF_TELEMETRY_FILENAME)
}

/// Intelligent telemetry retention: a handful of spans fire thousands of times an
/// hour at ~0ms (a GUI->daemon `status` poll was ~70% of jojo's perf log, with
/// per-keystroke `terminal_read`/`terminal_write` and `ping` close behind). At 16 MiB
/// the log then rotates the genuinely diagnostic spans (`copy_scan`, the chores) out
/// within a few hours. Rather than 7x-ing the cap (7x disk + 7x slower `perf-summary`
/// scans, mostly of noise), we KEEP every slow outlier of a noisy span (a `status`
/// poll that took 40ms IS worth seeing) and 1:50-SAMPLE the rest so the rate stays
/// visible (count x50) at ~2% of the volume — shrinking the log ~10x so the same cap
/// holds a day+ of what matters. Everything else is always recorded.
const NOISY_SPAN_RECORD_FLOOR_MS: f64 = 8.0;
const NOISY_SPAN_SAMPLE_RATE: u64 = 50;
static NOISY_SPAN_SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The high-frequency, low-diagnostic-value spans subject to floor+sampling. Pure so
/// the policy is unit-testable and obvious at a glance.
pub fn perf_span_is_high_frequency_noise(category: &str, name: &str) -> bool {
    matches!(
        (category, name),
        ("daemon_request", "status")
            | ("daemon_request", "ping")
            | ("daemon_request", "terminal_read")
            | ("daemon_request", "terminal_write")
            | ("daemon_request", "terminal_snapshot")
    )
}

/// Whether a finished span should be written to the telemetry log. Noisy spans are
/// kept only when SLOW (>= floor) or on the 1:50 sample; everything else always.
fn perf_span_should_record(category: &str, name: &str, duration_ms: f64) -> bool {
    if !perf_span_is_high_frequency_noise(category, name) {
        return true;
    }
    if duration_ms >= NOISY_SPAN_RECORD_FLOOR_MS {
        return true;
    }
    NOISY_SPAN_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed) % NOISY_SPAN_SAMPLE_RATE == 0
}

pub fn append_bounded_jsonl_record(
    path: &Path,
    rotated_filename: &str,
    max_bytes: u64,
    record: &Value,
) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = create_dir_all(parent);
    let Ok(mut line) = serde_json::to_vec(record) else {
        return;
    };
    line.push(b'\n');
    rotate_jsonl_if_needed(path, rotated_filename, max_bytes, line.len() as u64);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(&line);
    }
}

pub fn append_perf_event(home: &Path, category: &str, name: &str, payload: Value) {
    if !perf_profiling_enabled() {
        return;
    }
    let _ = create_dir_all(home);
    let path = perf_telemetry_path(home);
    let event = json!({
        "ts_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        "category": category,
        "name": name,
        "payload": payload,
    });
    append_bounded_jsonl_record(
        &path,
        PERF_TELEMETRY_ROTATED_FILENAME,
        PERF_TELEMETRY_MAX_BYTES,
        &event,
    );
}

fn rotate_jsonl_if_needed(path: &Path, rotated_filename: &str, max_bytes: u64, incoming_len: u64) {
    let rotated_path = path.with_file_name(rotated_filename);
    if fs::metadata(&rotated_path)
        .map(|metadata| metadata.len() > max_bytes)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&rotated_path);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() > max_bytes {
        let _ = fs::remove_file(path);
        return;
    }
    if metadata.len().saturating_add(incoming_len) <= max_bytes {
        return;
    }
    let _ = fs::remove_file(&rotated_path);
    let _ = fs::rename(path, rotated_path);
}

pub struct PerfSpan {
    home: PathBuf,
    category: String,
    name: String,
    started_at: Instant,
}

impl PerfSpan {
    pub fn start(
        home: impl Into<PathBuf>,
        category: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            home: home.into(),
            category: category.into(),
            name: name.into(),
            started_at: Instant::now(),
        }
    }

    pub fn finish(self, payload: Value) {
        let duration_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
        if !perf_span_should_record(&self.category, &self.name, duration_ms) {
            return;
        }
        append_perf_event(
            &self.home,
            &self.category,
            &self.name,
            json!({
                "duration_ms": duration_ms,
                "meta": payload,
            }),
        );
    }
}

/// RAII profiling span: records its duration when dropped. Built for hot paths laced
/// with `?` early returns, where an explicit `PerfSpan::finish` would be skipped on the
/// error branch. Creating one is nearly free when profiling is off (a single atomic
/// load — the inner span and its `PathBuf` are only allocated when recording is on), so
/// these can stay compiled into the hot paths permanently. Attach payload context with
/// [`PerfGuard::annotate`] before the guard drops.
pub struct PerfGuard {
    span: Option<PerfSpan>,
    payload: Value,
}

impl PerfGuard {
    pub fn new(
        home: impl Into<PathBuf>,
        category: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        let span = perf_profiling_enabled().then(|| PerfSpan::start(home, category, name));
        Self {
            span,
            payload: Value::Null,
        }
    }

    /// Replace the payload recorded when the guard drops (e.g. the resolved session
    /// path, byte counts, or a sub-phase outcome). No-op when profiling is off.
    pub fn annotate(&mut self, payload: Value) {
        if self.span.is_some() {
            self.payload = payload;
        }
    }
}

impl Drop for PerfGuard {
    fn drop(&mut self) {
        if let Some(span) = self.span.take() {
            span.finish(std::mem::replace(&mut self.payload, Value::Null));
        }
    }
}

/// Aggregated timing for one `(category, name)` profiling span, the unit
/// `server perf-summary` reports. Durations are milliseconds.
#[derive(Debug, Clone, Serialize)]
pub struct PerfSpanSummary {
    pub category: String,
    pub name: String,
    pub count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub total_ms: f64,
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    // Nearest-rank on the already-sorted slice.
    let rank = ((pct / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

/// Aggregate `perf-telemetry.jsonl` (plus its rotated sibling) into per-span stats,
/// sorted by total time descending (the spans where the app spends the most wall-clock).
/// `since_ms`: only include events with `ts_ms >= since_ms`. `category_filter`: only
/// that category. This is the read side of the app profiling system — it answers "where
/// is time going?" without re-deriving anything from the raw log by hand.
pub fn summarize_perf_telemetry(
    home: &Path,
    since_ms: Option<u64>,
    category_filter: Option<&str>,
) -> Vec<PerfSpanSummary> {
    let mut durations: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    let primary = perf_telemetry_path(home);
    let rotated = primary.with_file_name(PERF_TELEMETRY_ROTATED_FILENAME);
    for path in [rotated, primary] {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(since) = since_ms
                && event.get("ts_ms").and_then(Value::as_u64).unwrap_or(0) < since
            {
                continue;
            }
            let category = event.get("category").and_then(Value::as_str).unwrap_or("");
            if let Some(filter) = category_filter
                && category != filter
            {
                continue;
            }
            let name = event.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(duration) = event
                .get("payload")
                .and_then(|payload| payload.get("duration_ms"))
                .and_then(Value::as_f64)
            else {
                continue;
            };
            durations
                .entry((category.to_string(), name.to_string()))
                .or_default()
                .push(duration);
        }
    }
    let mut summaries: Vec<PerfSpanSummary> = durations
        .into_iter()
        .map(|((category, name), mut values)| {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let count = values.len();
            let total_ms: f64 = values.iter().sum();
            PerfSpanSummary {
                category,
                name,
                count,
                p50_ms: percentile(&values, 50.0),
                p95_ms: percentile(&values, 95.0),
                p99_ms: percentile(&values, 99.0),
                max_ms: values.last().copied().unwrap_or(0.0),
                mean_ms: if count == 0 {
                    0.0
                } else {
                    total_ms / count as f64
                },
                total_ms,
            }
        })
        .collect();
    summaries.sort_by(|a, b| {
        b.total_ms
            .partial_cmp(&a.total_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yggterm-perf-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn high_frequency_noise_spans_keep_outliers_and_useful_spans() {
        // Noisy spans: fast ones are floor/sampled, SLOW ones (an outlier worth seeing)
        // are always kept.
        assert!(super::perf_span_is_high_frequency_noise("daemon_request", "status"));
        assert!(super::perf_span_is_high_frequency_noise("daemon_request", "terminal_read"));
        assert!(super::perf_span_should_record("daemon_request", "status", 40.0)); // slow → keep
        // Useful spans are ALWAYS recorded regardless of duration.
        assert!(!super::perf_span_is_high_frequency_noise("background", "copy_scan"));
        assert!(super::perf_span_should_record("background", "copy_scan", 0.0));
        assert!(super::perf_span_should_record("copy_generation", "title", 0.0));
        assert!(super::perf_span_should_record("daemon", "snapshot_response", 0.0));
    }

    #[test]
    fn append_bounded_jsonl_record_rotates_when_file_would_overflow() {
        let dir = temp_test_dir("rotate");
        let path = dir.join("test.jsonl");
        let first = json!({ "message": "a".repeat(90) });
        let second = json!({ "message": "b".repeat(90) });

        append_bounded_jsonl_record(&path, "test.previous.jsonl", 120, &first);
        append_bounded_jsonl_record(&path, "test.previous.jsonl", 120, &second);

        let rotated = dir.join("test.previous.jsonl");
        let current_text = fs::read_to_string(&path).expect("read current file");
        let rotated_text = fs::read_to_string(&rotated).expect("read rotated file");

        assert!(current_text.contains(&"b".repeat(20)));
        assert!(rotated_text.contains(&"a".repeat(20)));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn summarize_perf_telemetry_groups_and_ranks_by_total() {
        let dir = temp_test_dir("summary");
        set_perf_profiling_enabled(true);
        let home = dir.clone();
        // attach span: 3 samples (10, 20, 30) -> total 60; persist: 1 sample (100).
        for d in [10.0_f64, 20.0, 30.0] {
            append_perf_event(&home, "attach", "managed_cli", json!({ "duration_ms": d }));
        }
        append_perf_event(&home, "daemon", "persist", json!({ "duration_ms": 100.0 }));

        let summary = summarize_perf_telemetry(&home, None, None);
        // persist (total 100) outranks attach (total 60).
        assert_eq!(summary[0].name, "persist");
        assert_eq!(summary[0].count, 1);
        assert_eq!(summary[0].max_ms, 100.0);
        let attach = summary.iter().find(|s| s.name == "managed_cli").unwrap();
        assert_eq!(attach.count, 3);
        assert_eq!(attach.total_ms, 60.0);
        assert_eq!(attach.max_ms, 30.0);
        assert_eq!(attach.mean_ms, 20.0);
        // category filter narrows the result set.
        let only_attach = summarize_perf_telemetry(&home, None, Some("attach"));
        assert_eq!(only_attach.len(), 1);
        assert_eq!(only_attach[0].name, "managed_cli");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn append_bounded_jsonl_record_discards_pathological_oversized_file() {
        let dir = temp_test_dir("oversized");
        let path = dir.join("test.jsonl");
        fs::write(&path, "x".repeat(512)).expect("seed oversized file");

        append_bounded_jsonl_record(&path, "test.previous.jsonl", 120, &json!({ "ok": true }));

        let current_text = fs::read_to_string(&path).expect("read current file");
        let rotated = dir.join("test.previous.jsonl");
        assert!(current_text.contains("\"ok\":true"));
        assert!(!rotated.exists());

        let _ = fs::remove_dir_all(dir);
    }
}
