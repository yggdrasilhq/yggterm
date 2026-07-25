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
pub const PERF_TELEMETRY_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Rotated perf-telemetry generations: at most 3 days, 128 MiB total. The
/// floor+sampling policy below already shrinks the stream ~10x, so this budget
/// comfortably holds the full window.
const PERF_TELEMETRY_RETENTION: crate::retention::JsonlRetention =
    crate::retention::JsonlRetention {
        live_max_bytes: PERF_TELEMETRY_MAX_BYTES,
        generations_max_bytes: 128 * 1024 * 1024,
        max_age_ms: crate::retention::DIAGNOSTIC_RETENTION_MAX_AGE_MS,
    };

pub fn perf_telemetry_path(home: &Path) -> PathBuf {
    home.join(PERF_TELEMETRY_FILENAME)
}

/// Intelligent telemetry retention: a handful of spans fire thousands of times an
/// hour at ~0ms (a GUI->daemon `status` poll was ~70% of guihost's perf log, with
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
            | ("daemon_request", "working_flags")
    )
}

/// Which clock a span's `duration_ms` is measured on.
///
/// Almost every span in this system is WALL time between two `Instant`s. The render
/// probe's are not: its `duration_ms` is CPU milliseconds a process consumed during an
/// interval, deliberately, so the existing aggregator could report it unchanged. That
/// works for aggregation and breaks for any rule that assumes elapsed time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfTimeBase {
    Wall,
    Cpu,
}

impl PerfTimeBase {
    pub fn as_str(&self) -> &'static str {
        match self {
            PerfTimeBase::Wall => "wall",
            PerfTimeBase::Cpu => "cpu",
        }
    }
}

/// THE owner of "which clock is this span on", pure and keyed on the CATEGORY — the
/// same shape as [`perf_span_is_high_frequency_noise`], and for the same reason: a fact
/// about a span kind belongs in one predicate, not stamped onto every event where two
/// copies could disagree. Keying on the category also means it applies retroactively to
/// the events already on disk, which a wire field could never do.
pub fn perf_span_time_base(category: &str) -> PerfTimeBase {
    if category == crate::render_probe::RENDER_PERF_CATEGORY {
        PerfTimeBase::Cpu
    } else {
        PerfTimeBase::Wall
    }
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
    crate::retention::append_retained_jsonl_record(&path, PERF_TELEMETRY_RETENTION, &event);
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
    /// Which clock the durations below are on, as a WIRE field, so a human (or the
    /// `--json` consumer) reading a `render` row does not mistake CPU milliseconds for
    /// elapsed time.
    ///
    /// ⚠ It is a projection of `category`, resolved once at construction. It is NOT the
    /// owner and no rule may read it: [`perf_span_time_base`]'s own note says a fact
    /// about a span kind belongs in one predicate, "not stamped onto every event where
    /// two copies could disagree", and this field was exactly such a stamp — read by
    /// `detect_perf_incident` while the owner sat one field away. Rules call
    /// [`PerfSpanSummary::time_base`] instead, which re-derives from the category.
    pub time_base: PerfTimeBase,
    pub count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub total_ms: f64,
}

impl PerfSpanSummary {
    /// Which clock this span is on, DERIVED from its category — the one owner.
    /// Deliberately not `self.time_base`: a summary that arrived with a stale or
    /// hand-built copy would otherwise be classified by the copy.
    pub fn time_base(&self) -> PerfTimeBase {
        perf_span_time_base(&self.category)
    }
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
    for path in crate::retention::jsonl_read_paths(&perf_telemetry_path(home)) {
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
            let time_base = perf_span_time_base(&category);
            PerfSpanSummary {
                category,
                name,
                time_base,
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

/// One incident kind, aggregated across the log — what `server perf-incidents`
/// reports.
///
/// The records have been written since the feature landed, but nothing could
/// READ them: the CLI answered `unsupported server command`, so 183 durable
/// snapshots of "the app went hot" sat on guihost unopened until someone parsed
/// them by hand (2026-07-25). That hand-parse is what named the top driver
/// (`remote/resolve_yggterm_binary`, 65 of 183), which is the whole reason this
/// reader exists: an instrument nobody can read is not an instrument.
#[derive(Debug, Clone, Serialize)]
pub struct PerfIncidentSummary {
    /// The trigger's first word — `span_busy`, `span_stall`,
    /// `copy_generation_busy`.
    pub trigger_kind: String,
    /// The span the trigger named, `category/name`, or empty when the trigger
    /// does not name one.
    pub span: String,
    pub count: usize,
    pub first_ts_ms: u64,
    pub last_ts_ms: u64,
    /// Worst `total_ms` this kind reached in any one incident window.
    pub worst_total_ms: f64,
}

/// Every incident record, oldest-first, optionally since a timestamp. Raw
/// `Value`s: the record carries a caller-supplied `extra` whose shape is the
/// caller's business, so parsing it here would be a second encoding of it.
pub fn read_perf_incidents(home: &Path, since_ms: Option<u64>) -> Vec<Value> {
    let mut records = Vec::new();
    for path in crate::retention::jsonl_read_paths(&home.join(PERF_INCIDENT_FILENAME)) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(since) = since_ms
                && record.get("ts_ms").and_then(Value::as_u64).unwrap_or(0) < since
            {
                continue;
            }
            records.push(record);
        }
    }
    records.sort_by_key(|record| record.get("ts_ms").and_then(Value::as_u64).unwrap_or(0));
    records
}

/// Group incidents by what triggered them, most frequent first.
///
/// Ranked by COUNT rather than by duration on purpose: incidents measure "the
/// app stalled", and the thing worth fixing first is the one that keeps
/// happening. Reading the 183 records this way is what turned a pile of
/// snapshots into one name.
pub fn summarize_perf_incidents(home: &Path, since_ms: Option<u64>) -> Vec<PerfIncidentSummary> {
    let mut grouped: BTreeMap<(String, String), PerfIncidentSummary> = BTreeMap::new();
    for record in read_perf_incidents(home, since_ms) {
        let ts_ms = record.get("ts_ms").and_then(Value::as_u64).unwrap_or(0);
        let trigger = record
            .get("trigger")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut parts = trigger.split_whitespace();
        let trigger_kind = parts.next().unwrap_or("").to_string();
        // `span_busy remote/resolve_yggterm_binary total_ms=…` — the middle
        // token is the span. `copy_generation_busy total_ms=…` names none.
        let span = parts
            .next()
            .filter(|token| !token.contains('='))
            .unwrap_or("")
            .to_string();
        let total_ms = trigger
            .split_whitespace()
            .find_map(|token| token.strip_prefix("total_ms="))
            .or_else(|| {
                trigger
                    .split_whitespace()
                    .find_map(|token| token.strip_prefix("max_ms="))
            })
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let entry = grouped
            .entry((trigger_kind.clone(), span.clone()))
            .or_insert_with(|| PerfIncidentSummary {
                trigger_kind,
                span,
                count: 0,
                first_ts_ms: ts_ms,
                last_ts_ms: ts_ms,
                worst_total_ms: 0.0,
            });
        entry.count += 1;
        entry.first_ts_ms = entry.first_ts_ms.min(ts_ms);
        entry.last_ts_ms = entry.last_ts_ms.max(ts_ms);
        entry.worst_total_ms = entry.worst_total_ms.max(total_ms);
    }
    let mut summaries: Vec<PerfIncidentSummary> = grouped.into_values().collect();
    summaries.sort_by(|a, b| {
        b.count.cmp(&a.count).then_with(|| {
            b.worst_total_ms
                .partial_cmp(&a.worst_total_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    summaries
}

pub const PERF_INCIDENT_FILENAME: &str = "perf-incidents.jsonl";
pub const PERF_INCIDENT_ROTATED_FILENAME: &str = "perf-incidents.previous.jsonl";
// Incidents are tiny (one record) and rare, so a generous cap keeps WEEKS of them —
// the whole point is to still have the snapshot when the user reports a fan flare
// from hours/days ago.
pub const PERF_INCIDENT_MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const PERF_INCIDENT_DEBOUNCE_MS: u64 = 5 * 60 * 1000;
const PERF_INCIDENT_STALL_MS: f64 = 30_000.0;
/// The bar for a CPU-time span. Cores, not milliseconds: a `render` span reporting 30
/// CPU-seconds over a 60 s window is HALF A CORE, i.e. an ordinarily busy GUI, while
/// the wall rules would have called it a 30-second stall. Past ~1.2 cores a single
/// render role really is eating the machine and deserves the snapshot.
const PERF_INCIDENT_CPU_CORES: f64 = 1.2;

/// Decide whether a recent perf-summary window looks like a LOAD INCIDENT worth a
/// durable snapshot — the random "guihost fan gets angry" moments you can't predict.
/// Triggers (each a short reason string):
///  - `copy_generation_busy`: title/summary generation ate > half the window (the
///    title-regen loop, the measured guihost fan driver).
///  - `span_busy`: a single span monopolized > 60% of the window.
///  - `span_stall`: a span's worst case blew past the stall ceiling.
///  - `span_cpu_hot`: a CPU-time span held more than [`PERF_INCIDENT_CPU_CORES`].
/// Returns `None` when the window is calm. Pure, so the policy is unit-tested.
///
/// ⚠ **The first three rules are WALL-CLOCK rules and only wall spans are eligible.**
/// Before that was enforced, the render probe's CPU-millisecond spans were judged by
/// them, and 35 of the 222 incidents on the live host were that misreading: a
/// `span_stall render/web_content` means only that one process used 30 CPU-seconds,
/// which on a 16-core box is unremarkable. It was not merely noise — incidents are
/// debounced five minutes and this function returns the FIRST match, so a render span
/// could take the slot a genuine stall would have used and mask it. Hence also the
/// ordering: the CPU rule is tried LAST, so a real wall-clock stall always wins.
pub fn detect_perf_incident(summary: &[PerfSpanSummary], window_ms: u64) -> Option<String> {
    let window = window_ms.max(1) as f64;
    let generation_total: f64 = summary
        .iter()
        .filter(|span| span.category == "copy_generation")
        .map(|span| span.total_ms)
        .sum();
    if generation_total > window * 0.5 {
        return Some(format!(
            "copy_generation_busy total_ms={generation_total:.0}"
        ));
    }
    let wall = || {
        summary
            .iter()
            .filter(|span| span.time_base() == PerfTimeBase::Wall)
    };
    if let Some(span) = wall().find(|span| span.total_ms > window * 0.6) {
        return Some(format!(
            "span_busy {}/{} total_ms={:.0}",
            span.category, span.name, span.total_ms
        ));
    }
    if let Some(span) = wall().find(|span| span.max_ms >= PERF_INCIDENT_STALL_MS) {
        return Some(format!(
            "span_stall {}/{} max_ms={:.0}",
            span.category, span.name, span.max_ms
        ));
    }
    if let Some((span, cores)) = summary
        .iter()
        .filter(|span| span.time_base() == PerfTimeBase::Cpu)
        .map(|span| (span, span.total_ms / window))
        .find(|(_, cores)| *cores >= PERF_INCIDENT_CPU_CORES)
    {
        // `total_ms=` stays in the string so `summarize_perf_incidents` ranks this
        // trigger with the others rather than reading it as zero.
        return Some(format!(
            "span_cpu_hot {}/{} cores={cores:.2} total_ms={:.0}",
            span.category, span.name, span.total_ms
        ));
    }
    None
}

/// If the last `window_ms` of perf telemetry looks like an incident (and none was
/// recorded within the debounce), append a compact snapshot — the trigger + the top
/// spans by total time + caller `extra` context — to `perf-incidents.jsonl`. Returns
/// the timestamp to store as the new `last_incident_ms` (unchanged when nothing was
/// recorded). The durable record is the catch for the random fan-angry: it's still
/// there when the user reports it after the fact. No-op when profiling is off.
pub fn record_perf_incident_if_hot(
    home: &Path,
    window_ms: u64,
    now_ms: u64,
    last_incident_ms: u64,
    extra: Value,
) -> u64 {
    if !perf_profiling_enabled() {
        return last_incident_ms;
    }
    if now_ms.saturating_sub(last_incident_ms) < PERF_INCIDENT_DEBOUNCE_MS {
        return last_incident_ms;
    }
    let since = now_ms.saturating_sub(window_ms);
    let summary = summarize_perf_telemetry(home, Some(since), None);
    let Some(trigger) = detect_perf_incident(&summary, window_ms) else {
        return last_incident_ms;
    };
    let top_spans: Vec<Value> = summary
        .iter()
        .take(8)
        .map(|span| {
            json!({
                "category": span.category,
                "name": span.name,
                "count": span.count,
                "total_ms": span.total_ms,
                "p99_ms": span.p99_ms,
                "max_ms": span.max_ms,
            })
        })
        .collect();
    let record = json!({
        "ts_ms": now_ms,
        "window_ms": window_ms,
        "trigger": trigger,
        "top_spans": top_spans,
        "extra": extra,
    });
    append_bounded_jsonl_record(
        &home.join(PERF_INCIDENT_FILENAME),
        PERF_INCIDENT_ROTATED_FILENAME,
        PERF_INCIDENT_MAX_BYTES,
        &record,
    );
    now_ms
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

    /// ⚠ THE COPY MAY NOT DECIDE ANYTHING. `PerfSpanSummary.time_base` is a wire
    /// projection of `category`; the owner is `perf_span_time_base`. This builds a
    /// summary whose stamped copy LIES and asserts the incident rules ignore it —
    /// which they can only do by re-deriving from the category.
    ///
    /// Without this, a hand-built or stale summary would be classified by its stamp:
    /// a `render` row stamped `Wall` would trip the stall rule at 30 s of CPU time
    /// (half a core), and a `startup` row stamped `Cpu` would let a genuine 45 s stall
    /// through as 0.75 cores.
    #[test]
    fn the_incident_rules_read_the_category_not_the_stamped_copy() {
        let window = 60_000u64;
        let lying =
            |category: &str, name: &str, total_ms: f64, max_ms: f64, stamp| PerfSpanSummary {
                time_base: stamp,
                ..span(category, name, total_ms, max_ms)
            };
        // A render span stamped Wall: half a core, not a stall.
        let render_stamped_wall = vec![lying(
            "render",
            "gui",
            30_000.0,
            30_000.0,
            PerfTimeBase::Wall,
        )];
        assert_eq!(
            super::detect_perf_incident(&render_stamped_wall, window),
            None
        );
        // A startup stall stamped Cpu is still a stall.
        let stall_stamped_cpu = vec![lying(
            "startup",
            "initial_server_sync",
            100.0,
            45_000.0,
            PerfTimeBase::Cpu,
        )];
        assert!(
            super::detect_perf_incident(&stall_stamped_cpu, window)
                .unwrap()
                .starts_with("span_stall")
        );
        // And the derived accessor is the owner's answer, whatever the stamp says.
        assert_eq!(render_stamped_wall[0].time_base(), PerfTimeBase::Cpu);
        assert_eq!(stall_stamped_cpu[0].time_base(), PerfTimeBase::Wall);
    }

    /// Builds a summary the way `summarize_perf_telemetry` would: the time base is
    /// RESOLVED from the category, never chosen by the test, so these cases exercise
    /// the same classification the live path takes.
    fn span(category: &str, name: &str, total_ms: f64, max_ms: f64) -> PerfSpanSummary {
        PerfSpanSummary {
            category: category.into(),
            name: name.into(),
            time_base: perf_span_time_base(category),
            count: 1,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            max_ms,
            mean_ms: 0.0,
            total_ms,
        }
    }

    #[test]
    fn perf_incident_detects_title_loop_and_stalls_but_not_calm() {
        let window = 60_000u64;
        // Calm 60s window — nothing fires.
        let calm = vec![span("background", "copy_scan", 4_000.0, 300.0)];
        assert!(super::detect_perf_incident(&calm, window).is_none());
        // Title-regen loop: > half the window spent generating → incident.
        let title_loop = vec![span("copy_generation", "title", 40_000.0, 6_000.0)];
        assert_eq!(
            super::detect_perf_incident(&title_loop, window).as_deref(),
            Some("copy_generation_busy total_ms=40000")
        );
        // A single span monopolizing the window.
        let busy = vec![span("daemon", "runtime_load", 45_000.0, 300.0)];
        assert!(
            super::detect_perf_incident(&busy, window)
                .unwrap()
                .starts_with("span_busy")
        );
        // A stall (worst case past the ceiling) even if total is small.
        let stall = vec![span("startup", "initial_server_sync", 35_000.0, 284_000.0)];
        // total 35k > 36k? no (0.6*60k=36k) → falls through to stall on max_ms.
        assert!(
            super::detect_perf_incident(&stall, window)
                .unwrap()
                .starts_with("span_stall")
        );
    }

    /// ⚠ THE lock for the incident log's biggest source of noise. `render` spans carry
    /// CPU milliseconds in `duration_ms` (deliberately — it is what lets the existing
    /// aggregator report them unchanged), and the busy/stall rules are about WALL
    /// time. Judging one by the other produced 35 of 222 incidents on the live host,
    /// every one meaningless: `span_stall render/web_content max_ms=70850` says only
    /// that a process used 70 CPU-seconds, which over a 60 s window is ~1.2 cores of
    /// ordinary work, not a stall.
    ///
    /// Each assertion below FIRES against the pre-fix code: the same numbers used to
    /// return `span_busy` / `span_stall`.
    #[test]
    fn a_cpu_time_span_is_never_judged_by_the_wall_clock_rules() {
        let window = 60_000u64;
        // 30 CPU-seconds in a 60 s window is HALF A CORE. The wall rule called this a
        // stall (max_ms >= 30_000); in cores it is not remotely an incident.
        let half_a_core = vec![span("render", "gui", 30_000.0, 30_000.0)];
        assert_eq!(super::detect_perf_incident(&half_a_core, window), None);
        // The worst render span actually recorded on the live host: 70.85 CPU-seconds
        // in a 60 s window. The wall rule called it `span_busy`; it is 1.18 cores.
        let live_worst = vec![span("render", "web_content", 70_850.0, 70_850.0)];
        assert_eq!(super::detect_perf_incident(&live_worst, window), None);
        // Past the bar, a CPU span IS an incident — with a trigger that names the
        // unit, so nobody has to guess what 90000 means.
        let genuinely_hot = vec![span("render", "web_content", 90_000.0, 90_000.0)];
        assert_eq!(
            super::detect_perf_incident(&genuinely_hot, window).as_deref(),
            Some("span_cpu_hot render/web_content cores=1.50 total_ms=90000")
        );
        // Wall spans are untouched: the SAME numbers still trip the old rules.
        let wall_busy = vec![span("daemon", "runtime_load", 70_850.0, 70_850.0)];
        assert!(
            super::detect_perf_incident(&wall_busy, window)
                .unwrap()
                .starts_with("span_busy")
        );
        let wall_stall = vec![span("startup", "initial_server_sync", 100.0, 30_000.0)];
        assert!(
            super::detect_perf_incident(&wall_stall, window)
                .unwrap()
                .starts_with("span_stall")
        );
        // And a genuine stall is never MASKED by a hot render span sharing the window:
        // incidents debounce for five minutes, so whichever trigger fires consumes the
        // slot. The wall rules run first, so the real one wins.
        let both = vec![
            span("render", "web_content", 120_000.0, 120_000.0),
            span("startup", "initial_server_sync", 100.0, 45_000.0),
        ];
        assert!(
            super::detect_perf_incident(&both, window)
                .unwrap()
                .starts_with("span_stall")
        );
    }

    /// The time base is a property of the SPAN KIND, resolved in one place, and it
    /// reaches the read surface so `perf-summary --json` says which clock a row is on.
    #[test]
    fn the_time_base_is_owned_by_the_category() {
        assert_eq!(perf_span_time_base("render"), PerfTimeBase::Cpu);
        assert_eq!(perf_span_time_base("daemon_request"), PerfTimeBase::Wall);
        assert_eq!(perf_span_time_base("copy_generation"), PerfTimeBase::Wall);
        assert_eq!(perf_span_time_base(""), PerfTimeBase::Wall);

        let dir = temp_test_dir("time_base");
        set_perf_profiling_enabled(true);
        append_perf_event(
            &dir,
            "render",
            "web_content",
            json!({ "duration_ms": 250.0 }),
        );
        append_perf_event(&dir, "daemon", "persist", json!({ "duration_ms": 250.0 }));
        let summary = summarize_perf_telemetry(&dir, None, None);
        let render = summary.iter().find(|s| s.category == "render").unwrap();
        let daemon = summary.iter().find(|s| s.category == "daemon").unwrap();
        assert_eq!(render.time_base, PerfTimeBase::Cpu);
        assert_eq!(daemon.time_base, PerfTimeBase::Wall);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn high_frequency_noise_spans_keep_outliers_and_useful_spans() {
        // Noisy spans: fast ones are floor/sampled, SLOW ones (an outlier worth seeing)
        // are always kept.
        assert!(super::perf_span_is_high_frequency_noise(
            "daemon_request",
            "status"
        ));
        assert!(super::perf_span_is_high_frequency_noise(
            "daemon_request",
            "terminal_read"
        ));
        assert!(super::perf_span_should_record(
            "daemon_request",
            "status",
            40.0
        )); // slow → keep
        // Useful spans are ALWAYS recorded regardless of duration.
        assert!(!super::perf_span_is_high_frequency_noise(
            "background",
            "copy_scan"
        ));
        assert!(super::perf_span_should_record(
            "background",
            "copy_scan",
            0.0
        ));
        assert!(super::perf_span_should_record(
            "copy_generation",
            "title",
            0.0
        ));
        assert!(super::perf_span_should_record(
            "daemon",
            "snapshot_response",
            0.0
        ));
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

    /// The reader that turns a pile of incident snapshots into one NAME. Ranked
    /// by count, because an incident measures "the app stalled" and the driver
    /// worth fixing is the one that keeps happening — which is exactly how
    /// `remote/resolve_yggterm_binary` (65 of 183) was found on the live host.
    #[test]
    fn perf_incidents_group_by_trigger_and_rank_by_count() {
        let dir = temp_test_dir("incidents");
        let home = dir.clone();
        let write = |ts_ms: u64, trigger: &str| {
            append_bounded_jsonl_record(
                &home.join(PERF_INCIDENT_FILENAME),
                PERF_INCIDENT_ROTATED_FILENAME,
                PERF_INCIDENT_MAX_BYTES,
                &json!({ "ts_ms": ts_ms, "window_ms": 60_000, "trigger": trigger }),
            );
        };
        write(
            1_000,
            "span_busy remote/resolve_yggterm_binary total_ms=40000",
        );
        write(
            2_000,
            "span_busy remote/resolve_yggterm_binary total_ms=52000",
        );
        write(3_000, "span_busy daemon/persist total_ms=39000");
        write(4_000, "copy_generation_busy total_ms=31000");

        let summary = summarize_perf_incidents(&home, None);
        assert_eq!(summary[0].trigger_kind, "span_busy");
        assert_eq!(summary[0].span, "remote/resolve_yggterm_binary");
        assert_eq!(summary[0].count, 2);
        assert_eq!(summary[0].worst_total_ms, 52000.0);
        assert_eq!(summary[0].first_ts_ms, 1_000);
        assert_eq!(summary[0].last_ts_ms, 2_000);
        // A trigger that names no span still groups, with an empty span.
        let generation = summary
            .iter()
            .find(|entry| entry.trigger_kind == "copy_generation_busy")
            .expect("copy_generation_busy present");
        assert_eq!(generation.span, "");
        assert_eq!(generation.worst_total_ms, 31000.0);
        // `--since-ms` drops everything older.
        assert_eq!(summarize_perf_incidents(&home, Some(3_000)).len(), 2);
        assert_eq!(read_perf_incidents(&home, Some(4_000)).len(), 1);

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
