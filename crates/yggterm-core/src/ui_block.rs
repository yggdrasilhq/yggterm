//! UI-thread block watchdog — the probe that survives what it measures.
//!
//! ⛔ **The watchdog cannot run on the thread it watches.** Every instrument
//! yggterm already had for a stalled GUI — the input chain, the render counter,
//! the mount events, every handler-side trace call — executes ON the UI thread.
//! When that thread blocks they do not fire, so a freeze bad enough that the
//! user kills the process by hand leaves an incident log that reads perfectly
//! clean. Zero incidents was never evidence of health; it was the shape of the
//! bug. Measuring a freeze from inside the freeze always returns zero.
//!
//! So the design inverts: the UI thread only ever **stamps** a timestamp, which
//! costs one relaxed atomic store, and a plain OS thread — owned by nothing the
//! GUI can block — decides whether that stamp has gone stale. If the UI thread
//! dies or blocks, the watcher keeps running and is the only thing that can
//! still speak.
//!
//! Attribution comes from `note_activity`, called on the trace emit paths, so a
//! recorded block can name what ran immediately before the gap opened. The
//! write uses `try_lock` and skips on contention: an attribution hint is worth
//! having, and never worth adding a lock to a hot path for.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Last moment the UI thread proved it was running, epoch ms.
static UI_STAMP_MS: AtomicU64 = AtomicU64::new(0);
/// Best-effort name of the last thing that ran, for attribution.
static LAST_ACTIVITY: Mutex<String> = Mutex::new(String::new());
static WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);

/// How often the UI thread stamps.
///
/// This is the instrument's resolution: a block shorter than one interval
/// cannot be distinguished from the ordinary gap between two stamps, so it must
/// stay comfortably under `UI_BLOCK_THRESHOLD_MS`. It also has to be cheap
/// enough to be uncontroversial on the UI thread — it is one relaxed atomic
/// store per tick.
pub const STAMP_INTERVAL_MS: u64 = 50;

/// How often the watcher looks. Must be well under the block threshold so a
/// block is noticed within roughly one poll of crossing it.
const POLL_MS: u64 = 50;
/// Rolling window for the blocks-per-minute density figure.
const DENSITY_WINDOW_MS: u64 = 60_000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Called from the UI thread to prove it is still running.
///
/// One relaxed store. This must stay cheap enough that nobody is ever tempted
/// to call it less often than the block threshold, because the resolution of
/// the whole instrument is the interval between two stamps.
pub fn stamp() {
    UI_STAMP_MS.store(now_ms(), Ordering::Relaxed);
}

/// Record what is about to run, so a block that follows can be attributed.
///
/// Never blocks: on lock contention the hint is skipped. A missing hint
/// degrades an incident to "unattributed", which is still an incident.
pub fn note_activity(label: &str) {
    if let Ok(mut slot) = LAST_ACTIVITY.try_lock() {
        slot.clear();
        slot.push_str(label);
    }
}

fn last_activity() -> Option<String> {
    LAST_ACTIVITY
        .try_lock()
        .ok()
        .and_then(|s| if s.is_empty() { None } else { Some(s.clone()) })
}

/// Start the watcher. Idempotent — a second call is a no-op.
///
/// `home` is the yggterm home the incident is logged against.
pub fn spawn_watchdog(home: PathBuf) {
    if WATCHDOG_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    stamp();
    std::thread::Builder::new()
        .name("yggterm-ui-block-watchdog".to_string())
        .spawn(move || watch_loop(home))
        .ok();
}

/// The detection state machine, kept pure so it can be driven by a test.
///
/// A block is only *measurable* once the UI thread comes back: while the thread
/// is away the gap is still growing, so filing at the moment the threshold is
/// crossed would report the threshold rather than the stall. The tracker
/// therefore remembers when the stamp went quiet and emits nothing until it
/// moves again.
#[derive(Debug, Default)]
pub struct BlockTracker {
    blocked_from: Option<u64>,
    last_seen_stamp: u64,
    recent: Vec<u64>,
}

impl BlockTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one observation of the heartbeat. Returns a sample when a completed
    /// block has just been measured.
    pub fn observe(&mut self, stamped_ms: u64, now_ms: u64, threshold_ms: u64) -> Option<UiBlockSampleData> {
        if stamped_ms == 0 {
            return None; // the UI thread has not started stamping yet
        }
        let gap_now = now_ms.saturating_sub(stamped_ms);
        if gap_now >= threshold_ms {
            if self.blocked_from.is_none() {
                self.blocked_from = Some(stamped_ms);
            }
            self.last_seen_stamp = stamped_ms;
            return None;
        }
        let blocked_from = self.blocked_from.take()?;
        self.last_seen_stamp = stamped_ms;
        let gap_ms = stamped_ms.saturating_sub(blocked_from);
        if gap_ms < threshold_ms {
            return None;
        }
        self.recent.push(now_ms);
        self.recent
            .retain(|t| now_ms.saturating_sub(*t) <= DENSITY_WINDOW_MS);
        let density = self.recent.len() as f64 * (60_000.0 / DENSITY_WINDOW_MS as f64);
        Some(UiBlockSampleData { gap_ms, blocks_per_min: density })
    }
}

/// What a completed block measured, before attribution is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct UiBlockSampleData {
    pub gap_ms: u64,
    pub blocks_per_min: f64,
}

fn watch_loop(home: PathBuf) {
    let threshold = ytrace::diagnosis::UI_BLOCK_THRESHOLD_MS;
    let mut tracker = BlockTracker::new();
    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let stamped = UI_STAMP_MS.load(Ordering::Relaxed);
        let Some(measured) = tracker.observe(stamped, now_ms(), threshold) else {
            continue;
        };
        let sample = ytrace::diagnosis::UiBlockSample {
            gap_ms: measured.gap_ms,
            last_activity: last_activity(),
            blocks_per_min: Some(measured.blocks_per_min),
        };
        if let Some(incident) = ytrace::diagnosis::diagnose_ui_block(&sample) {
            file_incident(&home, &incident, measured.gap_ms, measured.blocks_per_min);
        }
    }
}

fn file_incident(
    home: &std::path::Path,
    incident: &ytrace::diagnosis::Incident,
    gap_ms: u64,
    density: f64,
) {
    let payload = ytrace::diagnosis::incident_payload(incident);
    // The bus, so `ytrace incidents` and the notebooks see it.
    crate::perf::ytrace_provider().incident("ui", "ui", "block", payload.clone());
    // And a queryable span-shaped record, so `ytrace query --category ui
    // --name block` can rank blocks by duration like any other probe.
    crate::perf::ytrace_provider().emit_span(
        "ui",
        "ui".to_string(),
        "block".to_string(),
        ytrace::Clock::Wall,
        gap_ms as f64,
        serde_json::json!({
            "gap_ms": gap_ms,
            "blocks_per_min": density,
            "last_activity": incident.subject,
            "incident_id": incident.id,
        }),
    );
    // And the durable terminal-telemetry log, which is where an incident that
    // outlives the process is expected to be found.
    let event = crate::TerminalTelemetryEvent::new(
        "ui_block_watchdog",
        "ui",
        incident.id.clone(),
        payload,
    )
    .severity(incident.severity.as_str().to_string())
    .reason(Some(incident.diagnosis.clone()));
    let _ = crate::append_terminal_telemetry_event(home, &event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_activity_hint_still_produces_an_incident() {
        // Attribution is a nicety; the incident is the point. A freeze that
        // cannot name its cause must still be recorded, or the zero-incident
        // failure returns by a different route.
        let sample = ytrace::diagnosis::UiBlockSample {
            gap_ms: 900,
            last_activity: None,
            blocks_per_min: Some(1.0),
        };
        let incident = ytrace::diagnosis::diagnose_ui_block(&sample)
            .expect("a 900 ms stall is a block whether or not it can be attributed");
        assert_eq!(incident.id, "ui_block");
    }

    #[test]
    fn the_activity_hint_round_trips() {
        note_activity("copy_generation/title");
        assert_eq!(last_activity().as_deref(), Some("copy_generation/title"));
        note_activity("sidebar/merge_rows");
        assert_eq!(last_activity().as_deref(), Some("sidebar/merge_rows"));
    }

    #[test]
    fn a_stamp_is_observable_from_another_thread() {
        // The whole design rests on this: the watcher runs somewhere the UI
        // thread cannot stop it, and reads the stamp across that boundary.
        stamp();
        let seen = std::thread::spawn(|| UI_STAMP_MS.load(Ordering::Relaxed))
            .join()
            .expect("watcher thread");
        assert!(seen > 0, "the stamp must be visible off the stamping thread");
    }

    const T: u64 = 200; // threshold used by these tests

    #[test]
    fn an_induced_block_is_measured_when_the_ui_thread_returns() {
        // THE FALSIFIER, in miniature: the UI thread stamps at t=1000, blocks
        // for 500 ms, and stamps again at t=1500. The tracker must report a
        // 500 ms block — not 200 (the threshold) and not nothing.
        let mut tracker = BlockTracker::new();
        assert_eq!(tracker.observe(1000, 1000, T), None, "healthy tick");
        // Polls during the block: the gap is still growing, nothing to file yet.
        assert_eq!(tracker.observe(1000, 1250, T), None, "still blocked");
        assert_eq!(tracker.observe(1000, 1400, T), None, "still blocked");
        let measured = tracker
            .observe(1500, 1500, T)
            .expect("the block is measurable once the thread answers again");
        assert_eq!(measured.gap_ms, 500, "the REAL stall, not the threshold");
    }

    #[test]
    fn ordinary_scheduling_jitter_is_not_a_block() {
        let mut tracker = BlockTracker::new();
        for step in 0..20u64 {
            let t = 1000 + step * 60; // 60 ms apart, above the 50 ms stamp interval
            assert_eq!(tracker.observe(t, t + 10, T), None, "jitter must not file");
        }
    }

    #[test]
    fn a_block_is_filed_once_not_on_every_poll_while_it_lasts() {
        let mut tracker = BlockTracker::new();
        tracker.observe(1000, 1000, T);
        let mut filed = 0;
        for now in [1300u64, 1600, 1900, 2200] {
            if tracker.observe(1000, now, T).is_some() {
                filed += 1;
            }
        }
        assert_eq!(filed, 0, "a block in progress must not file repeatedly");
        assert!(tracker.observe(2300, 2300, T).is_some(), "and files exactly once on recovery");
    }

    #[test]
    fn density_counts_blocks_inside_the_window() {
        let mut tracker = BlockTracker::new();
        let mut last = 0.0;
        for i in 0..3u64 {
            let base = 1000 + i * 1000;
            tracker.observe(base, base, T);
            tracker.observe(base, base + 400, T);
            let m = tracker.observe(base + 500, base + 500, T).expect("block");
            last = m.blocks_per_min;
        }
        assert_eq!(last, 3.0, "three blocks inside the one-minute window");
    }

    #[test]
    fn the_watchdog_starts_only_once() {
        let home = std::env::temp_dir().join("yggterm-ui-block-idempotence");
        spawn_watchdog(home.clone());
        spawn_watchdog(home); // must not panic or start a second thread
        assert!(WATCHDOG_STARTED.load(Ordering::SeqCst));
    }
}
