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

/// Where a `ShellState` write came from.
///
/// ⛔ **The key allocates nothing, and that is what lets this be always on.**
/// The predecessor histogram keyed on `String` and formatted a `file:line` per
/// write, which was expensive enough that it had to sit behind an env flag or a
/// storm-armed window — and the consequence is on the record: twenty-one render
/// storms were detected and every one of them was unattributed, because the
/// attribution was off at the moment it was needed. A probe that is only armed
/// during the event it is meant to explain is armed by someone who already knew.
///
/// `Location::caller()` yields `&'static str` for the file, and a labelled
/// context is already `&'static str`, so one tuple covers both: `line == 0`
/// means the label is a human-written context rather than a source position.
/// Formatting happens once, at report time.
pub(crate) type WriteSite = (&'static str, u32);

fn write_site_label(site: &WriteSite) -> String {
    if site.1 == 0 {
        site.0.to_string()
    } else {
        let file = site
            .0
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(site.0);
        format!("{file}:{}", site.1)
    }
}

/// What a write site did to the render loop, over one window.
#[derive(Debug, Default, Clone)]
struct CauseAccumulator {
    /// How many times this site wrote.
    writes: u64,
    /// How many ROOT RENDERS this site wrote before.
    ///
    /// ⭐ The pair is the whole instrument. One site writing 500 times before a
    /// single render is churn with no render cost; one site writing once before
    /// each of 500 renders is the storm. A total alone cannot tell those apart,
    /// and the blink-storm question is exactly which one is happening.
    renders_preceded: u64,
}

#[derive(Debug, Default, Clone)]
struct ComponentAccumulator {
    renders: u64,
    total_ms: f64,
    max_ms: f64,
}

struct AttributionState {
    started_at: Instant,
    buckets: HashMap<&'static str, ComponentAccumulator>,
    /// Writes since the last root render — the causes of the render about to
    /// happen. Drained by `begin_root_render`.
    pending_writes: HashMap<WriteSite, u64>,
    /// Cause aggregate for the window.
    causes: HashMap<WriteSite, CauseAccumulator>,
    /// Cumulative per-site totals, for the storm autopsy's own reporting. This
    /// is the ONE place write sites are tallied; nothing keeps a second copy.
    cumulative: HashMap<WriteSite, u64>,
    root_renders: u64,
    /// Root renders that had **no state write at all** in front of them.
    ///
    /// ⭐ This is amplification, measured rather than inferred. A render nothing
    /// wrote for is a forced wake or a second pass over one change — the "one
    /// driver event, about two full renders" half of the blink storm — and it
    /// is the number a coalescing fix has to move.
    renders_unattributed: u64,
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
            pending_writes: HashMap::new(),
            causes: HashMap::new(),
            cumulative: HashMap::new(),
            root_renders: 0,
            renders_unattributed: 0,
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

/// Record one `ShellState` write. Called unconditionally from both write paths.
///
/// ⛔ Unconditionally is the point — see [`WriteSite`]. The cost is two hash
/// lookups on a path that already writes a signal, against a probe that is
/// finally armed when the storm happens rather than after someone notices one.
pub(crate) fn note_state_write(site: WriteSite) {
    let mut state = lock_attribution();
    *state.pending_writes.entry(site).or_insert(0) += 1;
    *state.cumulative.entry(site).or_insert(0) += 1;
}

/// Close the causal gap between the last root render and this one.
///
/// Writes land BETWEEN renders and cause the next one, so the set drained here
/// is this render's cause set. Called from the top of the root component.
pub(crate) fn begin_root_render() {
    let mut state = lock_attribution();
    state.root_renders += 1;
    if state.pending_writes.is_empty() {
        state.renders_unattributed += 1;
        return;
    }
    let pending = std::mem::take(&mut state.pending_writes);
    for (site, writes) in pending {
        let cause = state.causes.entry(site).or_default();
        cause.writes += writes;
        // One increment per RENDER, however many times the site wrote before
        // it. That is what separates a chatty site from a re-rendering one.
        cause.renders_preceded += 1;
    }
}

/// Drop the cumulative per-site tally. The storm autopsy calls this when it
/// arms, so its window starts from zero.
pub(crate) fn clear_state_write_totals() {
    lock_attribution().cumulative.clear();
}

/// The cumulative per-site tally, formatted and ranked hottest first. `limit`
/// of `None` returns every site.
pub(crate) fn state_write_totals(limit: Option<usize>) -> Vec<(String, u64)> {
    let state = lock_attribution();
    let mut entries: Vec<(&WriteSite, &u64)> = state.cumulative.iter().collect();
    entries.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    entries
        .into_iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|(site, count)| (write_site_label(site), *count))
        .collect()
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
    let causes = std::mem::take(&mut state.causes);
    let root_renders = std::mem::replace(&mut state.root_renders, 0);
    let renders_unattributed = std::mem::replace(&mut state.renders_unattributed, 0);
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

    let mut causes = causes
        .into_iter()
        .map(|(site, cause)| {
            json!({
                "site": write_site_label(&site),
                "writes": cause.writes,
                "renders_preceded": cause.renders_preceded,
            })
        })
        .collect::<Vec<_>>();
    // Ranked by renders CAUSED, not by writes. A site that writes constantly
    // and re-renders nothing is not the problem being looked for, and ranking
    // by write count puts it on top of the one that is.
    causes.sort_by(|left, right| {
        let left_renders = left["renders_preceded"].as_u64().unwrap_or(0);
        let right_renders = right["renders_preceded"].as_u64().unwrap_or(0);
        right_renders
            .cmp(&left_renders)
            .then_with(|| right["writes"].as_u64().unwrap_or(0).cmp(&left["writes"].as_u64().unwrap_or(0)))
    });
    causes.truncate(24);

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
            // The denominator. A component whose `renders` equals this
            // re-rendered on EVERY root pass — i.e. nothing memoized it — which
            // is the "which component invalidates" question answered directly.
            "root_renders": root_renders,
            // ⭐ Renders with no state write in front of them: amplification,
            // measured. A coalescing fix has to move this number.
            "renders_unattributed": renders_unattributed,
            "components": components,
            // ⭐ Who wrote the signal. `renders_preceded` is the causal count;
            // `writes` is the churn count. A site with high writes and low
            // renders_preceded is chatty and harmless; the reverse is the storm.
            "causes": causes,
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
        reset_for_test();

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

    fn reset_for_test() {
        let mut state = lock_attribution();
        state.buckets.clear();
        state.pending_writes.clear();
        state.causes.clear();
        state.cumulative.clear();
        state.root_renders = 0;
        state.renders_unattributed = 0;
        state.started_at = Instant::now();
    }

    #[test]
    fn a_chatty_site_and_a_re_rendering_site_are_told_apart() {
        // ⛔ THE DISCRIMINATOR THE BLINK-STORM QUESTION TURNS ON. Both sites
        // below write ten times. One writes them all before a single render and
        // costs one render; the other writes once before each of ten renders
        // and costs ten. A write TOTAL reports them identically — and ranking by
        // that total puts the harmless one on top of the expensive one.
        let _serial = serialized();
        reset_for_test();

        for _ in 0..10 {
            note_state_write(("chatty.rs", 1));
        }
        begin_root_render();

        for _ in 0..10 {
            note_state_write(("rerenderer.rs", 2));
            begin_root_render();
        }

        let state = lock_attribution();
        let chatty = state.causes.get(&("chatty.rs", 1)).expect("chatty tracked");
        let rerenderer = state
            .causes
            .get(&("rerenderer.rs", 2))
            .expect("rerenderer tracked");

        assert_eq!(chatty.writes, 10);
        assert_eq!(chatty.renders_preceded, 1, "ten writes, one render");
        assert_eq!(rerenderer.writes, 10);
        assert_eq!(rerenderer.renders_preceded, 10, "ten writes, ten renders");
        assert_eq!(state.root_renders, 11);
    }

    #[test]
    fn a_render_with_no_write_before_it_is_counted_as_amplification() {
        // ⭐ The second half of the blink storm, measured rather than inferred:
        // one driver event producing about two full renders. A render nothing
        // wrote for is a forced wake or a second pass over one change, and it is
        // the number a coalescing fix has to move.
        let _serial = serialized();
        reset_for_test();

        note_state_write(("driver.rs", 7));
        begin_root_render(); // attributed to driver.rs
        begin_root_render(); // nothing wrote — amplification
        begin_root_render(); // nothing wrote — amplification

        let state = lock_attribution();
        assert_eq!(state.root_renders, 3);
        assert_eq!(state.renders_unattributed, 2);
        assert_eq!(
            state.causes.get(&("driver.rs", 7)).map(|c| c.renders_preceded),
            Some(1),
            "a site is credited with the render it caused, not the ones that followed"
        );
    }

    #[test]
    fn a_write_site_label_names_a_source_line_or_a_context_but_never_both() {
        // Line 0 is the marker that a label is a human-written context. If that
        // convention ever slips, a context called `foo` and a file called `foo`
        // merge into one bucket and the histogram silently lies.
        let _serial = serialized();
        assert_eq!(
            write_site_label(&("crates/yggterm-shell/src/shell/state.rs", 4242)),
            "state.rs:4242"
        );
        assert_eq!(write_site_label(&("terminal_attach_bridge_closed", 0)),
            "terminal_attach_bridge_closed");
    }

    #[test]
    fn totals_rank_hottest_first_and_the_limit_binds() {
        let _serial = serialized();
        reset_for_test();
        for _ in 0..5 {
            note_state_write(("hot.rs", 1));
        }
        note_state_write(("cold.rs", 2));

        let all = state_write_totals(None);
        assert_eq!(all.first().map(|(site, count)| (site.as_str(), *count)), Some(("hot.rs:1", 5)));
        assert_eq!(all.len(), 2);
        assert_eq!(state_write_totals(Some(1)).len(), 1);

        clear_state_write_totals();
        assert!(state_write_totals(None).is_empty());
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
