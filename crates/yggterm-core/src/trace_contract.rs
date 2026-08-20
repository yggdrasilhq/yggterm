//! The language-agnostic trace contract.
//!
//! `trace.rs` owns the *transport* — where the bytes land, how the file
//! rotates. This module owns the *grammar*: the three tags that let a record
//! written by a non-Rust layer be read, ranked and correlated against a record
//! written by Rust, and the validation that a foreign emitter's submission
//! passes through before it is allowed onto the plane.
//!
//! The wire is `docs/spec-trace-plane-contract.md`. Everything here is the
//! executable half of that document; when the two disagree, they are both
//! wrong and the fix lands in the same commit.
//!
//! ## Why a foreign record is validated rather than trusted
//!
//! A foreign emitter runs in a sandbox that cannot know its own pid, cannot
//! read the app version, and — this is the one that matters — **cannot be
//! trusted to say which clock a duration is on**, because the sandbox it runs
//! in may not have the clock it is claiming. A record that arrives claiming
//! `clock: "cpu"` from a browser context is not a slightly-wrong record; it is
//! a number that will be divided by an interval it does not have and published
//! as a core fraction. The validator's job is to refuse that at the boundary,
//! once, rather than let every reader downstream discover it separately.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which runtime produced the record.
///
/// ⛔ This is **not** `component`, and collapsing the two is the mistake this
/// tag exists to prevent. `component` says which module inside the app the
/// event belongs to (`ui`, `daemon`, `session`); `layer` says which runtime
/// executed it. They are orthogonal, and it is precisely their product that
/// makes a vertical slice legible: `component=ui, layer=rust` is the Dioxus
/// component tree deciding to re-render, `component=ui, layer=xterm` is the
/// canvas that re-render is aimed at. Filter on `component` alone and the two
/// halves of one stall are indistinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceLayer {
    /// The native process: every `append_trace_event` call site in the
    /// yggterm crates. The implicit layer — a record with no `layer` field is
    /// `Rust`, which is what keeps every byte written before this contract
    /// existed readable under it.
    Rust,
    /// The reactive UI tree. Dioxus components are Rust functions, so this
    /// layer is *emitted from Rust* — the tag names where the work belongs in
    /// the architecture, not which compiler produced the instruction. A reader
    /// asking "what did the UI tree cost" wants these rows and not the daemon
    /// rows that happen to share `component=ui`.
    Dioxus,
    /// The terminal canvas: xterm.js inside the webview. Genuinely foreign —
    /// these records cross the bridge.
    Xterm,
    /// Reserved, and deliberately unused today. The co-browse surface will put
    /// a second live viewer on one session, and when it does, the question
    /// "which viewer was slow" needs a tag that already exists in the wire.
    /// Adding an enum variant later is free; adding it later *to bytes already
    /// on disk* is not, so the value is reserved now and the readers are
    /// written to expect it.
    Webkit,
}

impl TraceLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Dioxus => "dioxus",
            Self::Xterm => "xterm",
            Self::Webkit => "webkit",
        }
    }

    /// Parse a layer tag off the wire. Unknown tags are rejected rather than
    /// mapped to a default: a typo that silently becomes `rust` puts foreign
    /// rows into the native population and biases every aggregate computed
    /// over it, invisibly.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "rust" => Some(Self::Rust),
            "dioxus" => Some(Self::Dioxus),
            "xterm" => Some(Self::Xterm),
            "webkit" => Some(Self::Webkit),
            _ => None,
        }
    }

    /// Whether this layer runs inside the webview sandbox. The one place the
    /// clock rule below is decided from, so it cannot drift per call site.
    pub fn is_sandboxed(self) -> bool {
        matches!(self, Self::Xterm | Self::Webkit)
    }
}

/// What the record's timestamp *means*, which is not the same question as what
/// the record is called.
///
/// `docs/observability.md` §4.3c is the war story: `request/lock_wait_slow` and
/// `request/lock_wait_window` read as two views of one thing, and a correlation
/// run that matched them on the substring `lock_wait` compared point events
/// against bookkeeping ticks and produced a confident no-correlation that meant
/// nothing. The law it derived — establish point-vs-window before any temporal
/// analysis — was left to the reader, because *nothing in the record said so*.
/// This tag is that law made machine-readable, so the next reader cannot get it
/// wrong by matching on a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    /// `ts_ms` is the moment the thing happened. Safe to correlate against.
    Point,
    /// `ts_ms` is the moment the thing *finished*, and `duration_ms` says how
    /// far back it started. Correlate against the interval, not the instant.
    Span,
    /// `ts_ms` is the moment a **summary window closed**. The values inside are
    /// faithful; the timestamp is bookkeeping. ⛔ Never correlate on it.
    Window,
}

impl TraceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Span => "span",
            Self::Window => "window",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "point" => Some(Self::Point),
            "span" => Some(Self::Span),
            "window" => Some(Self::Window),
            _ => None,
        }
    }

    /// Whether a reader may treat `ts_ms` as the moment the measured thing
    /// occurred. False for `Window`, and that is the whole point of the tag.
    pub fn timestamp_is_correlatable(self) -> bool {
        !matches!(self, Self::Window)
    }
}

/// Which clock a `duration_ms` is on. Mirrors `ytrace::Clock` deliberately —
/// the trace plane and the ytrace bus must not grow two spellings of one
/// concept — but is declared here so a foreign record can be validated without
/// the bridge depending on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceClock {
    /// Elapsed time between two monotonic reads. A latency.
    Wall,
    /// CPU milliseconds consumed during a sampling interval. ⛔ Not a latency,
    /// and meaningless without the interval it covers — see
    /// `docs/observability.md` §3.1.
    Cpu,
}

impl TraceClock {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wall => "wall",
            Self::Cpu => "cpu",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "wall" => Some(Self::Wall),
            "cpu" => Some(Self::Cpu),
            _ => None,
        }
    }
}

/// The largest payload a single foreign record may carry, serialized.
///
/// The bridge hands these to a synchronous append on the UI thread. A foreign
/// emitter that stuffs a whole terminal buffer into a payload turns one probe
/// into a multi-kilobyte write on the thread being measured — the emitter
/// becoming the perturbation, which is the failure this whole channel is built
/// to avoid. Oversized payloads are replaced by a marker, never dropped: a
/// record that says "my payload was too big" is diagnostic; a missing record is
/// an absence, and an absence that is structural says nothing.
pub const MAX_FOREIGN_PAYLOAD_BYTES: usize = 8 * 1024;

/// The largest number of records the bridge will accept in one batch. The JS
/// emitter's ring is smaller than this; the cap exists so a malformed or
/// hostile submission cannot hold the trace lock for an unbounded time.
pub const MAX_FOREIGN_BATCH_RECORDS: usize = 256;

/// A record submitted by a non-Rust layer, exactly as it arrives.
///
/// Note what is **absent** and cannot be supplied by the sender: `pid`, `app`,
/// `app_version`. A sandboxed emitter has no truthful access to any of them, so
/// the fields are stamped by the receiver rather than carried on the wire. An
/// emitter that could set its own pid could also set someone else's.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForeignTraceRecord {
    /// Epoch millis **at the moment of emission**, not of arrival.
    ///
    /// ⛔ `u64`, NOT `u128`, and that is a wire constraint rather than a taste.
    /// The record crosses the webview bridge as a `serde_json::Value`, whose
    /// number type is `u64`/`i64`/`f64` — a `u128` field fails to deserialize
    /// with `u128 is not supported`, and the bridge answers by discarding the
    /// WHOLE batch. It reached the trace as `js_event_ignored` and looked
    /// exactly like a layer that had nothing to say. u64 millis run to the year
    /// 584 million, so nothing is lost; the record widens to the trace plane's
    /// `u128` at the boundary, where it is no longer JSON.
    ///
    /// ⛔ This field is the reason the batch is worth building and the reason
    /// it is dangerous. The emitter buffers and flushes off the hot path, so
    /// arrival can trail emission by the whole flush interval. Stamping on
    /// arrival would shift every foreign row later by an amount that varies
    /// with how busy the UI thread was — i.e. it would be *most* wrong exactly
    /// during the stalls the plane exists to explain, and the resulting
    /// timeline would show the probe firing after the fault it was measuring.
    pub ts_ms: u64,
    pub layer: String,
    pub component: String,
    pub category: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub clock: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    /// Per-emitter monotonic counter. Two records that share a millisecond are
    /// not orderable by `ts_ms`, and a corrupted repaint is precisely a
    /// question about what interleaved inside one millisecond, so the emitter
    /// numbers its own output and the numbering crosses the bridge.
    #[serde(default)]
    pub seq: Option<u64>,
    /// How many records the emitter dropped since its last accepted record.
    /// Carried per record rather than reported separately so that a drop can
    /// never be the thing that gets dropped.
    #[serde(default)]
    pub dropped: Option<u64>,
    #[serde(default)]
    pub payload: Value,
}

/// Why a foreign record was refused or altered at the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignRecordFault {
    UnknownLayer,
    /// A native layer tag arrived over the foreign bridge. Only the bridge can
    /// produce foreign records, so a record claiming `layer: "rust"` is either
    /// a bug or an attempt to launder foreign data into the native population.
    LayerNotForeign,
    UnknownKind,
    UnknownClock,
    /// A sandboxed layer claimed the CPU clock. There is no per-thread CPU
    /// clock in a webview content process: `performance.now()` is monotonic
    /// wall time and `Date.now()` is epoch wall time, and neither one knows
    /// what fraction of the interval the thread was scheduled for. A cpu claim
    /// from there cannot be true.
    CpuClockFromSandbox,
    /// `duration_ms` present without a `clock` to interpret it, or negative,
    /// or not finite. §3.1's whole complaint, arriving as data.
    UnusableDuration,
    EmptyProbeName,
    PayloadTooLarge,
}

impl ForeignRecordFault {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownLayer => "unknown_layer",
            Self::LayerNotForeign => "layer_not_foreign",
            Self::UnknownKind => "unknown_kind",
            Self::UnknownClock => "unknown_clock",
            Self::CpuClockFromSandbox => "cpu_clock_from_sandbox",
            Self::UnusableDuration => "unusable_duration",
            Self::EmptyProbeName => "empty_probe_name",
            Self::PayloadTooLarge => "payload_too_large",
        }
    }
}

/// A foreign record that has passed the boundary, with every field the plane
/// requires either validated or known-absent.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedForeignRecord {
    pub ts_ms: u64,
    pub layer: TraceLayer,
    pub component: String,
    pub category: String,
    pub name: String,
    pub kind: TraceKind,
    pub clock: Option<TraceClock>,
    pub duration_ms: Option<f64>,
    pub seq: Option<u64>,
    pub dropped: Option<u64>,
    pub payload: Value,
    /// Faults that were *repaired* rather than fatal — an oversized payload
    /// replaced by a marker, say. The record still lands; the repair is
    /// visible in it.
    pub repairs: Vec<ForeignRecordFault>,
}

/// Validate one foreign submission.
///
/// Returns `Err` only for faults that make the record unreadable or a lie.
/// Everything survivable is repaired in place and recorded in `repairs`, on the
/// principle that a diagnostic stream loses more to silent absences than to
/// slightly degraded rows.
pub fn validate_foreign_record(
    raw: ForeignTraceRecord,
) -> Result<ValidatedForeignRecord, ForeignRecordFault> {
    let layer = TraceLayer::parse(&raw.layer).ok_or(ForeignRecordFault::UnknownLayer)?;
    if !layer.is_sandboxed() {
        return Err(ForeignRecordFault::LayerNotForeign);
    }
    if raw.category.trim().is_empty() || raw.name.trim().is_empty() {
        return Err(ForeignRecordFault::EmptyProbeName);
    }

    let clock = match raw.clock.as_deref() {
        None => None,
        Some(raw_clock) => {
            let parsed = TraceClock::parse(raw_clock).ok_or(ForeignRecordFault::UnknownClock)?;
            if parsed == TraceClock::Cpu {
                return Err(ForeignRecordFault::CpuClockFromSandbox);
            }
            Some(parsed)
        }
    };

    let duration_ms = match raw.duration_ms {
        None => None,
        Some(value) => {
            if !value.is_finite() || value < 0.0 || clock.is_none() {
                return Err(ForeignRecordFault::UnusableDuration);
            }
            Some(value)
        }
    };

    // The kind defaults from the shape rather than from a constant: a record
    // carrying a duration IS a span whether or not the emitter said so, and
    // inferring it here means one emitter forgetting the tag cannot put a span
    // into the point population where §4.3c's correlation trap lives.
    let kind = match raw.kind.as_deref() {
        Some(raw_kind) => TraceKind::parse(raw_kind).ok_or(ForeignRecordFault::UnknownKind)?,
        None if duration_ms.is_some() => TraceKind::Span,
        None => TraceKind::Point,
    };

    let mut repairs = Vec::new();
    let mut payload = raw.payload;
    let payload_bytes = serde_json::to_vec(&payload).map(|v| v.len()).unwrap_or(0);
    if payload_bytes > MAX_FOREIGN_PAYLOAD_BYTES {
        payload = serde_json::json!({
            "ygg_payload_dropped": true,
            "ygg_payload_bytes": payload_bytes,
        });
        repairs.push(ForeignRecordFault::PayloadTooLarge);
    }

    Ok(ValidatedForeignRecord {
        ts_ms: raw.ts_ms,
        layer,
        component: raw.component,
        category: raw.category,
        name: raw.name,
        kind,
        clock,
        duration_ms,
        seq: raw.seq,
        dropped: raw.dropped,
        payload,
        repairs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(layer: &str, category: &str, name: &str) -> ForeignTraceRecord {
        ForeignTraceRecord {
            ts_ms: 1_723_900_000_123,
            layer: layer.to_string(),
            component: "ui".to_string(),
            category: category.to_string(),
            name: name.to_string(),
            kind: None,
            clock: None,
            duration_ms: None,
            seq: None,
            dropped: None,
            payload: json!({}),
        }
    }

    #[test]
    fn a_record_decodes_from_the_wire_shape_the_bridge_actually_delivers() {
        // ⛔⛔ EVERY OTHER TEST HERE BUILDS THE STRUCT IN RUST AND THEREFORE
        // SKIPS THE WIRE. That gap shipped a real defect: `ts_ms` was `u128`,
        // which a `serde_json::Value` cannot represent, so the bridge failed
        // the whole batch with "u128 is not supported" and dropped it. On the
        // trace it appeared as `js_event_ignored` — indistinguishable from a
        // foreign layer that simply had nothing to say, which is the failure
        // mode this contract spends most of its rules trying to prevent.
        //
        // ⇒ This test decodes from a `Value`, the way the bridge does. A
        // contract test that constructs its own input is testing the
        // constructor.
        let wire = json!({
            "ts_ms": 1_787_234_359_561u64,
            "layer": "xterm",
            "component": "ui",
            "category": "xterm_write",
            "name": "flush",
            "kind": "span",
            "clock": "wall",
            "duration_ms": 1.4,
            "seq": 8817,
            "payload": { "host_id": "terminal-a", "pending_chars": 0 },
        });
        let raw: ForeignTraceRecord =
            serde_json::from_value(wire).expect("the bridge's own wire shape must decode");
        let validated = validate_foreign_record(raw).expect("and then validate");
        assert_eq!(validated.ts_ms, 1_787_234_359_561);
        assert_eq!(validated.layer, TraceLayer::Xterm);
        assert_eq!(validated.kind, TraceKind::Span);
        assert_eq!(validated.seq, Some(8817));

        // And the minimum an emitter may send: no kind, no clock, no payload.
        let minimal: ForeignTraceRecord = serde_json::from_value(json!({
            "ts_ms": 1_787_234_359_561u64,
            "layer": "xterm",
            "component": "ui",
            "category": "xterm_screen",
            "name": "reset",
        }))
        .expect("a minimal record must decode");
        assert_eq!(
            validate_foreign_record(minimal).unwrap().kind,
            TraceKind::Point
        );
    }

    #[test]
    fn a_sandboxed_layer_may_not_claim_the_cpu_clock() {
        // There is no per-thread CPU clock in a webview content process, so a
        // record claiming one is not imprecise — it is a number that would be
        // divided by an interval it does not have and published as a core
        // fraction. Refuse it at the boundary, once.
        let mut record = raw("xterm", "xterm_write", "flush");
        record.clock = Some("cpu".into());
        record.duration_ms = Some(4.0);
        assert_eq!(
            validate_foreign_record(record),
            Err(ForeignRecordFault::CpuClockFromSandbox)
        );
    }

    #[test]
    fn a_duration_without_a_clock_is_refused_not_defaulted() {
        // Defaulting to `wall` would be the friendly thing and the wrong one:
        // an emitter that forgot the clock is an emitter whose units are
        // unknown, and a guessed unit is indistinguishable from a measured one
        // once it is on disk.
        let mut record = raw("xterm", "xterm_write", "flush");
        record.duration_ms = Some(4.0);
        assert_eq!(
            validate_foreign_record(record),
            Err(ForeignRecordFault::UnusableDuration)
        );
    }

    #[test]
    fn a_native_layer_tag_cannot_arrive_over_the_foreign_bridge() {
        // Only the bridge produces foreign records. A submission claiming
        // `rust` would launder webview-sourced rows into the native population
        // and quietly bias every aggregate computed over it.
        assert_eq!(
            validate_foreign_record(raw("rust", "render", "gui")),
            Err(ForeignRecordFault::LayerNotForeign)
        );
        assert_eq!(
            validate_foreign_record(raw("dioxus", "render", "gui")),
            Err(ForeignRecordFault::LayerNotForeign)
        );
    }

    #[test]
    fn an_unknown_layer_is_refused_rather_than_mapped_to_a_default() {
        assert_eq!(
            validate_foreign_record(raw("xtrem", "xterm_write", "flush")),
            Err(ForeignRecordFault::UnknownLayer)
        );
    }

    #[test]
    fn kind_is_inferred_from_shape_when_the_emitter_omits_it() {
        // A record carrying a duration IS a span whether or not it said so.
        // Inferring here means one forgetful emitter cannot put spans into the
        // point population, which is where the §4.3c correlation trap lives.
        let mut span = raw("xterm", "xterm_write", "flush");
        span.clock = Some("wall".into());
        span.duration_ms = Some(4.0);
        assert_eq!(validate_foreign_record(span).unwrap().kind, TraceKind::Span);

        let point = raw("xterm", "xterm_write", "enqueue");
        assert_eq!(validate_foreign_record(point).unwrap().kind, TraceKind::Point);
    }

    #[test]
    fn a_window_records_timestamp_is_not_correlatable_and_says_so() {
        let mut window = raw("xterm", "xterm_render", "frame_window");
        window.kind = Some("window".into());
        let validated = validate_foreign_record(window).unwrap();
        assert_eq!(validated.kind, TraceKind::Window);
        assert!(!validated.kind.timestamp_is_correlatable());
        assert!(TraceKind::Point.timestamp_is_correlatable());
        assert!(TraceKind::Span.timestamp_is_correlatable());
    }

    #[test]
    fn an_oversized_payload_is_replaced_by_a_marker_not_dropped() {
        // A record that says "my payload was too big" is diagnostic. A missing
        // record is an absence, and a structural absence says nothing at all.
        let mut record = raw("xterm", "xterm_write", "flush");
        record.payload = json!({ "buffer": "x".repeat(MAX_FOREIGN_PAYLOAD_BYTES + 1) });
        let validated = validate_foreign_record(record).unwrap();
        assert_eq!(validated.repairs, vec![ForeignRecordFault::PayloadTooLarge]);
        assert_eq!(validated.payload["ygg_payload_dropped"], json!(true));
        assert!(
            validated.payload["ygg_payload_bytes"].as_u64().unwrap()
                > MAX_FOREIGN_PAYLOAD_BYTES as u64
        );
    }

    #[test]
    fn an_empty_probe_name_is_refused() {
        assert_eq!(
            validate_foreign_record(raw("xterm", "", "flush")),
            Err(ForeignRecordFault::EmptyProbeName)
        );
        assert_eq!(
            validate_foreign_record(raw("xterm", "xterm_write", "   ")),
            Err(ForeignRecordFault::EmptyProbeName)
        );
    }

    #[test]
    fn a_negative_or_infinite_duration_never_reaches_the_plane() {
        for bad in [-1.0_f64, f64::INFINITY, f64::NAN] {
            let mut record = raw("xterm", "xterm_write", "flush");
            record.clock = Some("wall".into());
            record.duration_ms = Some(bad);
            assert_eq!(
                validate_foreign_record(record),
                Err(ForeignRecordFault::UnusableDuration),
                "duration {bad} must be refused"
            );
        }
    }

    #[test]
    fn the_reserved_webkit_layer_already_parses() {
        // Reserved for the co-browse surface's second viewer. Adding a variant
        // later is free; teaching a reader to expect it in bytes already
        // written is not, so it is accepted from the first release.
        assert_eq!(TraceLayer::parse("webkit"), Some(TraceLayer::Webkit));
        assert!(TraceLayer::Webkit.is_sandboxed());
        assert!(!TraceLayer::Rust.is_sandboxed());
        assert!(!TraceLayer::Dioxus.is_sandboxed());
    }
}
