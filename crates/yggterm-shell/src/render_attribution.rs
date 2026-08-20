//! Per-component render attribution for the Dioxus layer.
//!
//! The root already reports a render RATE (`ui/perf/app_render_rate`), which
//! answers "is the tree re-rendering too often" and nothing else. When the
//! answer is yes, the next question — *which part of the tree* — has had no
//! instrument, and the storm autopsy that does exist attributes a render to the
//! `ShellState` field that changed rather than to the component that spent the
//! time. Those are different questions: a single field mutation can re-render a
//! cheap component or an expensive one, and only the second is worth acting on.
//!
//! ## ⛔ Why this aggregates instead of emitting per render
//!
//! Per-render trace emission at ~50/s is not a heavier version of this; it is
//! `finding-ui-freeze-js-debug-trace-flood`, where a synchronous append per UI
//! event froze the app for seconds. The storm autopsy in `state.rs` was built
//! bounded for exactly that reason and says so at its own arm site. An
//! instrument that stalls the render loop it is timing does not report a slow
//! render — it *causes* one, and then reports that.
//!
//! ⇒ Renders accumulate in memory; one `window` record per component leaves the
//! process per interval. The cost of measuring is then independent of the
//! render rate, which is the only property that makes it safe to leave armed
//! during a storm — which is precisely when it has something to say.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::json;
use yggterm_core::{TraceClock, TraceKind, TraceLayer, append_tagged_trace_event};

/// How long a component's renders accumulate before the bucket is reported.
///
/// Sized against the root's own 60 s rate report: short enough that a storm
/// lasting a few seconds still produces its own row rather than being averaged
/// into a quiet minute, long enough that the reporting itself is rare.
const RENDER_WINDOW_MS: u128 = 2_000;

#[derive(Debug, Default, Clone)]
struct ComponentAccumulator {
    renders: u64,
    total_ms: f64,
    max_ms: f64,
}

struct AttributionState {
    started_at: Instant,
    buckets: HashMap<&'static str, ComponentAccumulator>,
}

/// ⛔ Recover from poisoning rather than bailing out, matching the trace
/// writer's own discipline. A panic anywhere in the process while this lock is
/// held would otherwise disable render attribution **for the remaining life of
/// the GUI, silently** — and the reader would see a probe that is registered,
/// emits nothing, and looks exactly like a healthy component that never
/// rendered. The state behind the lock is a counter bucket: the worst a
/// poisoned view can cost is one skewed window, against an instrument that
/// stays dead until restart.
fn lock_attribution() -> std::sync::MutexGuard<'static, AttributionState> {
    attribution()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn attribution() -> &'static Mutex<AttributionState> {
    static STATE: OnceLock<Mutex<AttributionState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(AttributionState {
            started_at: Instant::now(),
            buckets: HashMap::new(),
        })
    })
}

/// An RAII guard whose lifetime is one component render.
///
/// ⚠ It measures the component function's own execution, which is the time
/// Dioxus spends building that subtree's `Element` — NOT the time the webview
/// spends applying the resulting mutations. Those are two different costs on
/// two sides of a serialization boundary, and conflating them is how a UI stall
/// gets blamed on the wrong half of the app. The `layer` tag exists so a reader
/// can hold them apart: this is `dioxus`, and what the canvas does with the
/// result is `xterm`.
pub(crate) struct ComponentRenderSpan {
    name: &'static str,
    started_at: Instant,
}

impl ComponentRenderSpan {
    pub(crate) fn start(name: &'static str) -> Self {
        Self {
            name,
            started_at: Instant::now(),
        }
    }
}

impl Drop for ComponentRenderSpan {
    fn drop(&mut self) {
        let elapsed_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
        let mut state = lock_attribution();
        let bucket = state.buckets.entry(self.name).or_default();
        bucket.renders += 1;
        bucket.total_ms += elapsed_ms;
        if elapsed_ms > bucket.max_ms {
            bucket.max_ms = elapsed_ms;
        }
    }
}

/// Report and reset any window that has run its course.
///
/// Called from the root render rather than from a timer: the thing being
/// measured is the render loop, so the render loop is the one clock guaranteed
/// to be ticking whenever there is anything to report. ⚠ The corollary, stated
/// because a reader will otherwise assume the interval: **when rendering stops,
/// the last window stays open** and is reported by the next render, whenever
/// that comes. `window_ms` is therefore measured and emitted, never assumed —
/// a consumer that divides by the constant computes a rate that is wrong by
/// exactly the length of the idle stretch.
pub(crate) fn flush_component_render_windows(trace_home: &Path) {
    let mut state = lock_attribution();
    let window_ms = state.started_at.elapsed().as_millis();
    if window_ms < RENDER_WINDOW_MS || state.buckets.is_empty() {
        return;
    }
    let buckets = std::mem::take(&mut state.buckets);
    state.started_at = Instant::now();
    drop(state);

    let mut components = buckets
        .into_iter()
        .map(|(name, bucket)| {
            json!({
                "component": name,
                "renders": bucket.renders,
                "total_ms": (bucket.total_ms * 100.0).round() / 100.0,
                "max_ms": (bucket.max_ms * 100.0).round() / 100.0,
                "mean_ms": if bucket.renders > 0 {
                    ((bucket.total_ms / bucket.renders as f64) * 100.0).round() / 100.0
                } else {
                    0.0
                },
            })
        })
        .collect::<Vec<_>>();
    // Hottest first, so the row that matters is the row a reader sees without
    // scanning. Sorted on TOTAL rather than max: a component rendering 400
    // times for 0.4 ms each is the storm, and ranking by the worst single
    // render would bury it under one unlucky outlier elsewhere.
    components.sort_by(|left, right| {
        let left_total = left["total_ms"].as_f64().unwrap_or(0.0);
        let right_total = right["total_ms"].as_f64().unwrap_or(0.0);
        right_total
            .partial_cmp(&left_total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    append_tagged_trace_event(
        trace_home,
        TraceLayer::Dioxus,
        // ⛔ A window, and tagged as one. Its `ts_ms` is when the bucket
        // CLOSED, which is bookkeeping — correlating an incident against it
        // would be comparing the incident to a reporting tick.
        TraceKind::Window,
        "ui",
        "dioxus_render",
        "component_window",
        // No clock on the record itself: the durations live per component
        // inside the payload, and a single `duration_ms` on a record that
        // summarises several components would have to mean their sum, which is
        // not a latency of anything.
        None::<TraceClock>,
        None,
        json!({
            "window_ms": window_ms as u64,
            "components": components,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The accumulator is process-global by design — it summarises the render
    /// loop, of which there is one — so two tests mutating it in parallel are
    /// testing each other. Serializing them here is not a workaround for a
    /// flake; it is the only honest way to assert on a global.
    fn serialized() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn a_span_accumulates_into_its_components_bucket() {
        let _serial = serialized();
        lock_attribution().buckets.clear();

        for _ in 0..3 {
            let _span = ComponentRenderSpan::start("TestComponent");
        }
        let state = lock_attribution();
        let bucket = state.buckets.get("TestComponent").expect("bucket exists");
        assert_eq!(bucket.renders, 3);
        assert!(bucket.max_ms >= 0.0);
        assert!(bucket.total_ms >= bucket.max_ms);
    }

    #[test]
    fn a_window_that_has_not_elapsed_reports_nothing_and_keeps_its_bucket() {
        // The guard that keeps the cost of measuring independent of the render
        // rate. If this ever short-circuits, a storm emits per render and the
        // instrument becomes the stall.
        let _serial = serialized();
        let mut state = lock_attribution();
        state.buckets.clear();
        state.started_at = Instant::now();
        state
            .buckets
            .insert("HeldComponent", ComponentAccumulator::default());
        drop(state);

        flush_component_render_windows(Path::new("/nonexistent-on-purpose"));

        assert!(
            lock_attribution().buckets.contains_key("HeldComponent"),
            "an unelapsed window must not be drained"
        );
    }

    #[test]
    fn a_poisoned_lock_does_not_retire_the_instrument() {
        // ⛔ The failure this guards is invisible in exactly the way that
        // matters: a dead attribution probe and a component that genuinely
        // never rendered produce the same empty result, so the instrument would
        // report "quiet" for the rest of the session and nobody would know to
        // doubt it.
        let _serial = serialized();
        let _ = std::panic::catch_unwind(|| {
            let _held = lock_attribution();
            panic!("poison the attribution lock");
        });
        lock_attribution().buckets.clear();
        // Dropped explicitly: the span accumulates on Drop, so a binding still
        // alive at the assertion has recorded nothing yet.
        drop(ComponentRenderSpan::start("AfterPoison"));
        assert_eq!(
            lock_attribution()
                .buckets
                .get("AfterPoison")
                .map(|bucket| bucket.renders),
            Some(1),
            "attribution must survive a panic that poisoned its lock"
        );
    }
}
