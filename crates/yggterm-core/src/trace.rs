use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::trace_contract::{
    ForeignRecordFault, ForeignTraceRecord, MAX_FOREIGN_BATCH_RECORDS, validate_foreign_record,
};

use crate::retention::{
    DIAGNOSTIC_RETENTION_MAX_AGE_MS, JsonlRetention, now_epoch_ms, prune_jsonl_generations,
    rotate_jsonl_with_retention,
};

pub const EVENT_TRACE_FILENAME: &str = "event-trace.jsonl";
const EVENT_TRACE_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Rotated event-trace generations, hard-capped so a trace flood (a reveal
/// loop, a render storm) cannot eat the disk.
///
/// ⛔ **THE AGE RULE DOES NOT SET THE WINDOW — THE BYTE BUDGET DOES, AND IT
/// SHRINKS WITH THE DAEMON POPULATION.** The budget is per HOME; the write rate
/// is per DAEMON, and every daemon on the host writes into the same file, so the
/// retained window is roughly `cap / (per-daemon rate x N daemons)`. The old
/// comment here claimed "~3 days at ~80 MiB/day" and was measured wrong by more
/// than an order of magnitude on 2026-08-14: **83.1 MiB/HOUR (1.95 GiB/day)
/// across ~20 daemons on one host, retaining 247.9 MiB over 2.98 h.** The
/// window was therefore ~3 hours, not 72, and the 3-day `max_age_ms` had never
/// once been the binding rule.
///
/// ⚠ That is not a cosmetic error. Four session deaths that a release was being
/// judged on had already aged out of the trace before anyone came to look, so
/// the investigation could only report an absence — and an absence that is
/// structural says nothing about what happened. A diagnostic window shorter
/// than the time it takes to notice a problem is not a diagnostic.
///
/// 1 GiB buys ~12 h at the measured rate. It is still a budget, not a promise:
/// if the daemon population doubles, the window halves. ⭐ On a pool with
/// transparent compression this costs far less disk than it reserves — the same
/// 247.9 MiB of trace occupied 17 MiB on disk when measured — but the cap is
/// counted in logical bytes, so it binds either way.
const EVENT_TRACE_RETENTION: JsonlRetention = JsonlRetention {
    live_max_bytes: EVENT_TRACE_MAX_BYTES,
    generations_max_bytes: 1024 * 1024 * 1024,
    max_age_ms: DIAGNOSTIC_RETENTION_MAX_AGE_MS,
};

/// One line of the trace plane.
///
/// ⭐ **Every field added after `payload` is additive and skipped when absent**,
/// which is what lets a record written before the language-agnostic contract
/// existed still parse under it, and lets `ytop` read a stream containing both.
/// A native record therefore looks byte-for-byte as it always did; only records
/// that genuinely carry the new information pay for the fields.
///
/// The grammar these fields belong to lives in
/// [`crate::trace_contract`] and `docs/spec-trace-plane-contract.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTraceRecord {
    pub ts_ms: u128,
    pub pid: u32,
    pub component: String,
    pub category: String,
    pub name: String,
    #[serde(default)]
    pub payload: Value,
    /// Which runtime produced this. Absent means `rust` — see
    /// [`crate::trace_contract::TraceLayer`] for why the implicit default is
    /// the correct reading of every byte already on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// `point` / `span` / `window`. Absent means `point`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Which clock `duration_ms` is on. ⛔ A `duration_ms` without this is not
    /// a slightly worse number, it is an uninterpretable one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    /// The emitting layer's own monotonic counter, for ordering records that
    /// share a millisecond.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

impl EventTraceRecord {
    /// The layer a reader should attribute this record to. Absent is `rust`,
    /// stated once here so no consumer has to remember it.
    pub fn layer_or_default(&self) -> &str {
        self.layer.as_deref().unwrap_or("rust")
    }

    /// The kind a reader should attribute this record to. Absent is `point`
    /// unless the record carries a duration, in which case it is a span — the
    /// same inference the foreign validator makes, so both sides of the bridge
    /// answer this question identically.
    pub fn kind_or_default(&self) -> &str {
        match self.kind.as_deref() {
            Some(kind) => kind,
            None if self.duration_ms.is_some() => "span",
            None => "point",
        }
    }

    /// Whether `ts_ms` may be used for temporal correlation. False for a
    /// windowed aggregate, whose timestamp is bookkeeping — see
    /// `docs/observability.md` §4.3c for the analysis this prevents.
    pub fn timestamp_is_correlatable(&self) -> bool {
        self.kind_or_default() != "window"
    }
}

pub fn event_trace_path(home: &Path) -> PathBuf {
    home.join(EVENT_TRACE_FILENAME)
}

/// A cached, append-mode handle to one trace file plus an in-memory byte
/// counter. Keeping the handle open lets `append_trace_event` skip the
/// `create_dir_all` + `metadata` stat + `open` + `close` syscalls it used to
/// pay on every single call — under a reveal/forward-loop flood that per-call
/// cost was the dominant on-thread I/O. We still issue one `write` per event so
/// followers (`follow_trace_lines`) see records immediately.
struct TraceWriter {
    file: File,
    bytes_written: u64,
}

fn open_trace_writer(path: &Path) -> Option<TraceWriter> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let bytes_written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Some(TraceWriter {
        file,
        bytes_written,
    })
}

/// Per-path cache of open trace handles. Keyed by path so a process that writes
/// to more than one home directory stays correct; in practice there is one
/// entry. The mutex serializes writers, so append-mode writes never interleave.
fn trace_writers() -> &'static Mutex<HashMap<PathBuf, TraceWriter>> {
    static WRITERS: OnceLock<Mutex<HashMap<PathBuf, TraceWriter>>> = OnceLock::new();
    WRITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

static YTRACE_TRACE_PROVIDER: OnceLock<ytrace::Provider> = OnceLock::new();
fn ytrace_trace_provider() -> &'static ytrace::Provider {
    YTRACE_TRACE_PROVIDER.get_or_init(|| {
        let p = ytrace::Provider::with_home(
            "yggterm",
            crate::current_version(),
            ytrace::compat::resolve_home("yggterm"),
        );
        // Trace events are the narrative log (session, daemon, gui) — always keep for Dash stories.
        p.register("trace/session", ytrace::Clock::Wall, ytrace::Sample::always());
        p.register("trace/daemon", ytrace::Clock::Wall, ytrace::Sample::always());
        p.register("trace/gui", ytrace::Clock::Wall, ytrace::Sample::always());
        p
    })
}

pub fn append_trace_event(
    home: &Path,
    component: impl Into<String>,
    category: impl Into<String>,
    name: impl Into<String>,
    payload: Value,
) {
    let component_s = component.into();
    let category_s = category.into();
    let name_s = name.into();
    // Attribution hint for the UI-block watchdog — see `ui_block::note_activity`.
    // The trace path sees far more of the GUI's work than the ytrace path does,
    // so it is the better witness to what ran just before a stall.
    crate::ui_block::note_activity(&format!("{category_s}/{name_s}"));
    let record = EventTraceRecord {
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        pid: std::process::id(),
        component: component_s.clone(),
        category: category_s.clone(),
        name: name_s.clone(),
        payload: payload.clone(),
        // The native path leaves every contract field absent on purpose. It is
        // the implicit `rust`/`point` case, and writing the defaults out would
        // add bytes to the highest-volume writer on the plane to say what the
        // reader already assumes.
        layer: None,
        kind: None,
        clock: None,
        duration_ms: None,
        seq: None,
    };
    let Ok(mut line) = serde_json::to_vec(&record) else {
        return;
    };
    line.push(b'\n');
    write_trace_line(home, &line);
    // Mirror to ytrace for Dash notebooks — Top stays trace-file-free in the book metaphor,
    // but the wire is dual-written so Dash can query via ytrace without reading the old file.
    ytrace_trace_provider().event(component_s, category_s, name_s, payload);
}

/// Append one NATIVE record that carries contract tags.
///
/// `append_trace_event` is the hot, untagged path — it writes the implicit
/// `rust`/`point` case and pays no bytes to say so. This is its counterpart for
/// the native call sites that genuinely have more to declare: a Dioxus render
/// aggregate is on the `dioxus` layer and is a `window`, and both facts have to
/// be in the record or a reader has to know them by reputation.
///
/// ⛔ Unlike the foreign path there is no validation here, because there is
/// nothing to validate: the caller is in-process, its pid is this pid, and a
/// `TraceClock` it hands over is a clock it actually read.
#[allow(clippy::too_many_arguments)]
pub fn append_tagged_trace_event(
    home: &Path,
    layer: crate::trace_contract::TraceLayer,
    kind: crate::trace_contract::TraceKind,
    component: impl Into<String>,
    category: impl Into<String>,
    name: impl Into<String>,
    clock: Option<crate::trace_contract::TraceClock>,
    duration_ms: Option<f64>,
    payload: Value,
) {
    let component_s = component.into();
    let category_s = category.into();
    let name_s = name.into();
    crate::ui_block::note_activity(&format!("{category_s}/{name_s}"));
    let record = EventTraceRecord {
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        pid: std::process::id(),
        component: component_s.clone(),
        category: category_s.clone(),
        name: name_s.clone(),
        payload: payload.clone(),
        layer: Some(layer.as_str().to_string()),
        kind: Some(kind.as_str().to_string()),
        clock: clock.map(|clock| clock.as_str().to_string()),
        duration_ms,
        seq: None,
    };
    let Ok(mut line) = serde_json::to_vec(&record) else {
        return;
    };
    line.push(b'\n');
    write_trace_line(home, &line);
    ytrace_trace_provider().event(component_s, category_s, name_s, payload);
}

fn write_trace_line(home: &Path, line: &[u8]) {
    let path = event_trace_path(home);
    let mut writers = trace_writers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if !writers.contains_key(&path) {
        let _ = fs::create_dir_all(home);
        // First write through this process: sweep expired generations once so
        // the "at most 3 days" cap holds even across idle stretches where the
        // live file never reaches the rotation size.
        prune_jsonl_generations(&path, EVENT_TRACE_RETENTION, now_epoch_ms());
        match open_trace_writer(&path) {
            Some(writer) => {
                writers.insert(path.clone(), writer);
            }
            None => return,
        }
    }

    // Rotate off the in-memory counter so we never stat per call. This matches
    // the original "rotate when the existing file is already at the cap"
    // behavior; the new line then lands in a fresh file.
    if writers
        .get(&path)
        .is_some_and(|writer| writer.bytes_written >= EVENT_TRACE_MAX_BYTES)
    {
        // Close the handle before renaming the inode, otherwise we would keep
        // appending to the rotated-away file.
        writers.remove(&path);
        rotate_jsonl_with_retention(&path, EVENT_TRACE_RETENTION, now_epoch_ms());
        let _ = fs::create_dir_all(home);
        match open_trace_writer(&path) {
            Some(writer) => {
                writers.insert(path.clone(), writer);
            }
            None => return,
        }
    }

    if let Some(writer) = writers.get_mut(&path) {
        if writer.file.write_all(line).is_ok() {
            writer.bytes_written = writer.bytes_written.saturating_add(line.len() as u64);
        } else {
            // A failed write almost always means the handle is stale (file
            // removed/replaced underneath us); drop it so the next call reopens.
            writers.remove(&path);
        }
    }
}

/// What one foreign batch did, so the bridge can account for it in a single
/// record instead of one per outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForeignBatchOutcome {
    pub accepted: usize,
    /// Records refused at the boundary, keyed by fault. ⛔ A refusal must never
    /// be silent: a foreign layer that has started emitting garbage and a
    /// foreign layer that has gone quiet look identical from the reader's side,
    /// and only one of them is a bug in the emitter.
    pub rejected: Vec<(&'static str, usize)>,
    /// Records that landed but were altered on the way in.
    pub repaired: usize,
    /// Records the emitter itself dropped before the batch was sent, summed
    /// across the batch. This is the emitter's own back-pressure being honest
    /// about itself.
    pub emitter_dropped: u64,
    /// Records discarded because the batch exceeded
    /// [`MAX_FOREIGN_BATCH_RECORDS`].
    pub over_batch_cap: usize,
}

impl ForeignBatchOutcome {
    fn count_rejection(&mut self, fault: &ForeignRecordFault) {
        let key = fault.as_str();
        if let Some(entry) = self.rejected.iter_mut().find(|(name, _)| *name == key) {
            entry.1 += 1;
        } else {
            self.rejected.push((key, 1));
        }
    }

    pub fn rejected_total(&self) -> usize {
        self.rejected.iter().map(|(_, count)| *count).sum()
    }
}

/// Append a batch of foreign (non-Rust) trace records in one pass.
///
/// ⛔⛔ **THE BATCH IS THE POINT, AND IT IS NOT AN OPTIMISATION.** The single
/// -record path does a lock acquire, a rotation check and a `write_all` per
/// call, on whichever thread called it. For the native crates that thread is
/// usually a worker; for a record arriving from the webview it is the **UI
/// event thread** — the exact thread whose stalls the foreign layers were
/// instrumented to explain. Hundreds of those back-to-back is not a
/// hypothetical: it is `finding-ui-freeze-js-debug-trace-flood`, where a reveal
/// burst turned the diagnostic channel into a seconds-long freeze, and the
/// standing mitigation is a throttle that sheds the events you most wanted.
///
/// ⇒ An instrument that perturbs the thing it measures does not produce a noisy
/// reading; it produces a reading **of itself**. One lock, one write, N lines
/// is what makes the channel affordable enough not to need the throttle that
/// was hiding the evidence.
pub fn append_foreign_trace_batch(
    home: &Path,
    records: Vec<ForeignTraceRecord>,
) -> ForeignBatchOutcome {
    let mut outcome = ForeignBatchOutcome::default();
    if records.is_empty() {
        return outcome;
    }

    let mut records = records;
    if records.len() > MAX_FOREIGN_BATCH_RECORDS {
        // Keep the OLDEST, because the batch is ordered and the tail of an
        // oversized batch is the part a following batch will carry anyway,
        // whereas the head is the only copy of the earliest evidence.
        outcome.over_batch_cap = records.len() - MAX_FOREIGN_BATCH_RECORDS;
        records.truncate(MAX_FOREIGN_BATCH_RECORDS);
    }

    // Attribution hint for the UI-block watchdog, once for the batch rather
    // than once per record: the watchdog wants to know what ran before a stall,
    // and N identical notes for one drain is noise, not attribution.
    crate::ui_block::note_activity("trace/foreign_batch");

    let mut buffer: Vec<u8> = Vec::with_capacity(records.len() * 256);
    let mut mirrored: Vec<(String, String, String, Value)> = Vec::with_capacity(records.len());

    for raw in records {
        let emitter_dropped = raw.dropped.unwrap_or(0);
        let validated = match validate_foreign_record(raw) {
            Ok(validated) => validated,
            Err(fault) => {
                outcome.count_rejection(&fault);
                // A refused record still contributes its drop count: the
                // emitter really did drop those, and refusing the carrier is
                // no reason to also lose the number it was carrying.
                outcome.emitter_dropped = outcome.emitter_dropped.saturating_add(emitter_dropped);
                continue;
            }
        };
        outcome.emitter_dropped = outcome
            .emitter_dropped
            .saturating_add(validated.dropped.unwrap_or(0));
        if !validated.repairs.is_empty() {
            outcome.repaired += 1;
        }

        let mut payload = validated.payload.clone();
        // Fold the emitter's own drop count into the payload so it survives
        // even for a reader who never looks at the batch accounting record.
        if let Some(dropped) = validated.dropped.filter(|dropped| *dropped > 0)
            && let Some(map) = payload.as_object_mut()
        {
            map.insert("ygg_emitter_dropped".into(), json!(dropped));
        }
        if !validated.repairs.is_empty()
            && let Some(map) = payload.as_object_mut()
        {
            map.insert(
                "ygg_repairs".into(),
                json!(
                    validated
                        .repairs
                        .iter()
                        .map(|fault| fault.as_str())
                        .collect::<Vec<_>>()
                ),
            );
        }

        let record = EventTraceRecord {
            ts_ms: u128::from(validated.ts_ms),
            // ⛔ Stamped by the RECEIVER, never carried on the wire: a
            // sandboxed emitter has no truthful access to a pid, and one that
            // could set its own could set another process's.
            pid: std::process::id(),
            component: validated.component.clone(),
            category: validated.category.clone(),
            name: validated.name.clone(),
            payload: payload.clone(),
            layer: Some(validated.layer.as_str().to_string()),
            kind: Some(validated.kind.as_str().to_string()),
            clock: validated.clock.map(|clock| clock.as_str().to_string()),
            duration_ms: validated.duration_ms,
            seq: validated.seq,
        };
        let Ok(mut line) = serde_json::to_vec(&record) else {
            continue;
        };
        line.push(b'\n');
        buffer.extend_from_slice(&line);
        outcome.accepted += 1;
        mirrored.push((
            validated.component,
            validated.category,
            validated.name,
            payload,
        ));
    }

    if !buffer.is_empty() {
        // ⭐ ONE call, so ONE lock acquisition and ONE `write_all` for the whole
        // drain. Rotation is still checked, once, against the accumulated
        // counter — the same rule the single-record path uses.
        write_trace_line(home, &buffer);
    }

    for (component, category, name, payload) in mirrored {
        ytrace_trace_provider().event(component, category, name, payload);
    }

    outcome
}

pub struct EventTraceSpan {
    home: PathBuf,
    component: String,
    category: String,
    name: String,
    started_at: Instant,
}

impl EventTraceSpan {
    pub fn start(
        home: impl Into<PathBuf>,
        component: impl Into<String>,
        category: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            home: home.into(),
            component: component.into(),
            category: category.into(),
            name: name.into(),
            started_at: Instant::now(),
        }
    }

    pub fn finish(self, payload: Value) {
        append_trace_event(
            &self.home,
            self.component,
            self.category,
            self.name,
            json!({
                "duration_ms": self.started_at.elapsed().as_secs_f64() * 1000.0,
                "meta": payload,
            }),
        );
    }
}

pub fn read_trace_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines().map_while(Result::ok).collect::<Vec<_>>();
    if lines.len() > max_lines {
        let keep_from = lines.len().saturating_sub(max_lines);
        lines.drain(0..keep_from);
    }
    lines
}

pub fn follow_trace_lines(path: &Path, initial_lines: usize, poll_ms: u64) -> ! {
    let mut emitted = read_trace_tail(path, initial_lines);
    for line in emitted.drain(..) {
        println!("{line}");
    }
    let mut last_len = fs::metadata(path)
        .map(|meta| meta.len())
        .unwrap_or_default();
    loop {
        sleep(Duration::from_millis(poll_ms.max(100)));
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let current_len = metadata.len();
        if current_len < last_len {
            last_len = 0;
        }
        if current_len == last_len {
            continue;
        }
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let _ = reader.seek_relative(last_len as i64);
        let mut line = String::new();
        while reader
            .read_line(&mut line)
            .ok()
            .is_some_and(|bytes| bytes > 0)
        {
            print!("{line}");
            line.clear();
        }
        let _ = std::io::stdout().flush();
        last_len = current_len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_home(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("ygg-trace-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn cached_writer_appends_every_event() {
        let home = unique_home("append");
        for i in 0..5 {
            append_trace_event(&home, "test", "unit", "ev", json!({ "i": i }));
        }
        // A separate reader (mimicking the follower process) sees all records,
        // proving each event is flushed to disk, not held in an in-memory buffer.
        let lines = read_trace_tail(&event_trace_path(&home), 100);
        assert_eq!(lines.len(), 5, "expected 5 trace lines, got {lines:?}");
        for (i, line) in lines.iter().enumerate() {
            let rec: EventTraceRecord = serde_json::from_str(line).expect("valid jsonl");
            assert_eq!(rec.name, "ev");
            assert_eq!(rec.payload["i"], json!(i));
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn cached_writer_reuses_one_handle() {
        let home = unique_home("reuse");
        let path = event_trace_path(&home);
        append_trace_event(&home, "test", "unit", "first", json!({}));
        // After the first write the handle is cached; a second write must not
        // create a new entry, and the byte counter must reflect both lines.
        append_trace_event(&home, "test", "unit", "second", json!({}));
        let on_disk = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let counted = {
            let writers = trace_writers().lock().unwrap();
            writers.get(&path).map(|w| w.bytes_written).unwrap_or(0)
        };
        assert_eq!(
            counted, on_disk,
            "in-memory byte counter must track the real file size"
        );
        let _ = fs::remove_dir_all(&home);
    }

    fn foreign(category: &str, name: &str) -> crate::trace_contract::ForeignTraceRecord {
        crate::trace_contract::ForeignTraceRecord {
            ts_ms: 1_723_900_000_123,
            layer: "xterm".into(),
            component: "ui".into(),
            category: category.into(),
            name: name.into(),
            kind: None,
            clock: None,
            duration_ms: None,
            seq: None,
            dropped: None,
            payload: json!({}),
        }
    }

    #[test]
    fn a_record_written_before_the_contract_still_parses_under_it() {
        // ⛔ The regression this guards is invisible: adding required fields to
        // the record would make every byte written before this commit fail to
        // deserialize, and the failure mode is a reader reporting an ABSENCE
        // over a window where the events plainly happened. Absence is the one
        // answer a diagnostic must never give wrongly.
        let legacy = r#"{"ts_ms":1,"pid":2,"component":"ui","category":"c","name":"n","payload":{}}"#;
        let record: EventTraceRecord = serde_json::from_str(legacy).expect("legacy line parses");
        assert_eq!(record.layer, None);
        assert_eq!(record.layer_or_default(), "rust");
        assert_eq!(record.kind_or_default(), "point");
        assert!(record.timestamp_is_correlatable());
    }

    #[test]
    fn a_native_record_gains_no_bytes_from_the_contract_fields() {
        // The native path is the highest-volume writer on the plane, and the
        // retention window is set by the BYTE budget. Serializing five absent
        // fields onto every line would shorten the diagnostic window for
        // nothing — the reader already assumes exactly these defaults.
        let record = EventTraceRecord {
            ts_ms: 1,
            pid: 2,
            component: "ui".into(),
            category: "c".into(),
            name: "n".into(),
            payload: json!({}),
            layer: None,
            kind: None,
            clock: None,
            duration_ms: None,
            seq: None,
        };
        let line = serde_json::to_string(&record).unwrap();
        for absent in ["layer", "kind", "clock", "duration_ms", "seq"] {
            assert!(!line.contains(absent), "{absent} must not be serialized: {line}");
        }
    }

    #[test]
    fn a_foreign_batch_lands_every_record_and_tags_the_layer() {
        let home = unique_home("foreign-batch");
        let mut records = Vec::new();
        for i in 0..4u64 {
            let mut record = foreign("xterm_write", "flush");
            record.seq = Some(i);
            record.clock = Some("wall".into());
            record.duration_ms = Some(1.5 + i as f64);
            records.push(record);
        }
        let outcome = append_foreign_trace_batch(&home, records);
        assert_eq!(outcome.accepted, 4);
        assert_eq!(outcome.rejected_total(), 0);

        let lines = read_trace_tail(&event_trace_path(&home), 100);
        assert_eq!(lines.len(), 4, "one line per record: {lines:?}");
        for (i, line) in lines.iter().enumerate() {
            let record: EventTraceRecord = serde_json::from_str(line).expect("valid jsonl");
            assert_eq!(record.layer_or_default(), "xterm");
            assert_eq!(record.kind_or_default(), "span");
            assert_eq!(record.clock.as_deref(), Some("wall"));
            assert_eq!(record.seq, Some(i as u64));
            // ⛔ Stamped by the receiver: a sandboxed emitter has no truthful
            // pid, so the wire never carries one.
            assert_eq!(record.pid, std::process::id());
        }
        trace_writers().lock().unwrap().remove(&event_trace_path(&home));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_refused_record_is_counted_and_never_silently_swallowed() {
        // A foreign layer emitting garbage and a foreign layer that has gone
        // quiet look identical from the reader's side. Only one is a bug in
        // the emitter, so the refusal has to be a number someone can read.
        let home = unique_home("foreign-refuse");
        let mut bad_clock = foreign("xterm_write", "flush");
        bad_clock.clock = Some("cpu".into());
        bad_clock.duration_ms = Some(2.0);
        let mut bad_layer = foreign("xterm_write", "flush");
        bad_layer.layer = "rust".into();
        let good = foreign("xterm_write", "enqueue");

        let outcome = append_foreign_trace_batch(&home, vec![bad_clock, bad_layer, good]);
        assert_eq!(outcome.accepted, 1);
        assert_eq!(outcome.rejected_total(), 2);
        assert!(outcome.rejected.contains(&("cpu_clock_from_sandbox", 1)));
        assert!(outcome.rejected.contains(&("layer_not_foreign", 1)));

        trace_writers().lock().unwrap().remove(&event_trace_path(&home));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_emitters_own_drop_count_survives_the_record_that_carried_it_being_refused() {
        // The drop count rides on records precisely so a drop cannot be the
        // thing that gets dropped. Refusing the carrier for an unrelated fault
        // must not also lose the number it was carrying.
        let home = unique_home("foreign-dropcount");
        let mut refused = foreign("xterm_write", "flush");
        refused.dropped = Some(17);
        refused.clock = Some("cpu".into());
        refused.duration_ms = Some(2.0);
        let outcome = append_foreign_trace_batch(&home, vec![refused]);
        assert_eq!(outcome.accepted, 0);
        assert_eq!(outcome.emitter_dropped, 17);

        trace_writers().lock().unwrap().remove(&event_trace_path(&home));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_oversized_batch_keeps_the_oldest_records_and_reports_the_rest() {
        // The tail of an oversized batch is what a following flush would carry
        // anyway; the head is the only copy of the earliest evidence.
        let home = unique_home("foreign-cap");
        let over = crate::trace_contract::MAX_FOREIGN_BATCH_RECORDS + 5;
        let records = (0..over as u64)
            .map(|i| {
                let mut record = foreign("xterm_write", "enqueue");
                record.seq = Some(i);
                record
            })
            .collect::<Vec<_>>();
        let outcome = append_foreign_trace_batch(&home, records);
        assert_eq!(outcome.over_batch_cap, 5);
        assert_eq!(
            outcome.accepted,
            crate::trace_contract::MAX_FOREIGN_BATCH_RECORDS
        );
        let lines = read_trace_tail(&event_trace_path(&home), 4);
        let first: EventTraceRecord = serde_json::from_str(&lines[0]).unwrap();
        assert!(
            first.seq.unwrap() < over as u64 - 5,
            "the retained window must start at the head, not the tail"
        );

        trace_writers().lock().unwrap().remove(&event_trace_path(&home));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn cached_writer_rotates_at_cap() {
        // Use a dedicated home so the global cache entry is isolated.
        let home = unique_home("rotate");
        let path = event_trace_path(&home);

        // Pre-seed the live file just past the cap, then force the cache to
        // adopt it by writing once (open picks up the existing size).
        let _ = fs::create_dir_all(&home);
        {
            let big = vec![b'x'; (EVENT_TRACE_MAX_BYTES + 16) as usize];
            fs::write(&path, &big).unwrap();
        }
        // Drop any stale cached handle from a prior run of this path.
        trace_writers().lock().unwrap().remove(&path);

        append_trace_event(&home, "test", "unit", "after-cap", json!({ "k": 1 }));

        let has_generation = fs::read_dir(&home).unwrap().flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("event-trace.g") && name.ends_with(".jsonl")
        });
        assert!(
            has_generation,
            "a timestamped generation should exist after rotation"
        );
        // The fresh live file holds only the post-rotation record.
        let lines = read_trace_tail(&path, 100);
        assert_eq!(lines.len(), 1, "fresh file should hold one record");
        let rec: EventTraceRecord = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(rec.name, "after-cap");

        trace_writers().lock().unwrap().remove(&path);
        let _ = fs::remove_dir_all(&home);
    }
}
