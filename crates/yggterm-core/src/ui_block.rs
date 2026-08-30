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

/// A kernel witness about this process, read on the watchdog thread at the two
/// ends of a measured stall.
///
/// Measured 2026-08-29 on the GUI host: the interface went silent for 40.7 s — every tracing
/// thread, then the block watchdog measured a 37.3 s UI gap — and the trace
/// files alone cannot say WHY. The classes that freeze a whole process leave
/// different fingerprints in kernel counters: a bounded-cgroup reclaim wall
/// shows as minor-fault + `memory.events`/PSI jumps across the gap (the 6.7
/// family bound makes this a standing suspect); a swap storm shows as major
/// faults; a SIGSTOP or scheduler wedge shows as context switches that do not
/// move at all. One stall, three verdicts — but only if the counters were
/// captured at BOTH ends, because the cgroup they indict is destroyed by
/// systemd when the scope dies, taking `memory.events` with it.
///
/// Every field is `Option` on purpose: an absent counter must serialize as
/// null, never as zero — a zero would testify "no throttling" when the truth
/// is "no reading".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcWitness {
    pub min_flt: Option<u64>,
    pub maj_flt: Option<u64>,
    pub voluntary_ctxt: Option<u64>,
    pub nonvoluntary_ctxt: Option<u64>,
    pub cg_path: Option<String>,
    pub cg_high: Option<u64>,
    pub cg_max: Option<u64>,
    pub cg_oom: Option<u64>,
    pub psi_some_total_us: Option<u64>,
    pub psi_full_total_us: Option<u64>,
}

/// `/proc/self/stat` fault counters. `comm` may contain spaces and parens, so
/// parse after the LAST `)`; the remainder's fields start at `state` (field 3),
/// making min_flt the 8th token from there (field 10) and maj_flt the 10th
/// (field 12).
fn parse_stat_faults(stat: &str) -> Option<(u64, u64)> {
    let after_comm = stat.rsplit_once(')')?.1;
    let parts: Vec<&str> = after_comm.split_whitespace().collect();
    let min_flt = parts.get(7)?.parse().ok()?;
    let maj_flt = parts.get(9)?.parse().ok()?;
    Some((min_flt, maj_flt))
}

/// `voluntary_ctxt_switches` / `nonvoluntary_ctxt_switches` from
/// `/proc/self/status`.
fn parse_status_ctxt(status: &str) -> Option<(u64, u64)> {
    let mut vol = None;
    let mut nonvol = None;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("voluntary_ctxt_switches:") {
            vol = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("nonvoluntary_ctxt_switches:") {
            nonvol = rest.trim().parse().ok();
        }
    }
    Some((vol?, nonvol?))
}

/// The cgroup-v2 unified-hierarchy path from `/proc/self/cgroup` (the `0::`
/// line), or None when the kernel has no such line.
fn parse_cgroup_v2_path(cgroup: &str) -> Option<String> {
    let line = cgroup.lines().find(|l| l.starts_with("0::"))?;
    Some(line.trim_start_matches("0::").to_string())
}

/// (high, max, oom) event counters from a cgroup v2 `memory.events` body.
fn parse_memory_events(events: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    let get = |key: &str| {
        events.lines().find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k == key).then(|| v.trim().parse().ok())?
        })
    };
    (get("high"), get("max"), get("oom"))
}

/// (some-total, full-total) microseconds from a cgroup v2 `memory.pressure`
/// body. The `total=` counter is cumulative stall time — its delta across a
/// stall is the direct measure of reclaim pressure during that stall.
fn parse_pressure_totals(pressure: &str) -> (Option<u64>, Option<u64>) {
    let total = |kind: &str| {
        pressure.lines().find_map(|l| {
            let (k, rest) = l.split_once(' ')?;
            if k != kind {
                return None;
            }
            rest.split_whitespace().find_map(|f| {
                let (fk, fv) = f.split_once('=')?;
                (fk == "total").then(|| fv.parse().ok())?
            })
        })
    };
    (total("some"), total("full"))
}

/// All four kernel witnesses at once: `/proc/self/stat`, `/proc/self/status`,
/// and the process's own cgroup v2 `memory.events` + `memory.pressure` joined
/// off the `0::` line under the standard unified-hierarchy mount.
fn proc_witness(
    stat: &str,
    status: &str,
    cgroup: &str,
    mem_events: Option<&str>,
    pressure: Option<&str>,
) -> ProcWitness {
    let (min_flt, maj_flt) = parse_stat_faults(stat).unzip();
    let (voluntary_ctxt, nonvoluntary_ctxt) = parse_status_ctxt(status).unzip();
    let cg_path = parse_cgroup_v2_path(cgroup);
    let (cg_high, cg_max, cg_oom) = match mem_events.map(parse_memory_events) {
        Some((high, max, oom)) => (high, max, oom),
        None => (None, None, None),
    };
    let (psi_some_total_us, psi_full_total_us) = pressure.map(parse_pressure_totals).unzip();
    let (psi_some_total_us, psi_full_total_us) =
        (psi_some_total_us.flatten(), psi_full_total_us.flatten());
    ProcWitness {
        min_flt,
        maj_flt,
        voluntary_ctxt,
        nonvoluntary_ctxt,
        cg_path,
        cg_high,
        cg_max,
        cg_oom,
        psi_some_total_us,
        psi_full_total_us,
    }
}

impl ProcWitness {
    /// Read the witness for this process. `None` only when even
    /// `/proc/self/stat` is unreadable — a platform without /proc, say — in
    /// which case the incident files with `witness: null` and nothing else
    /// degrades.
    fn read() -> Option<ProcWitness> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
        let rel = parse_cgroup_v2_path(&cgroup).unwrap_or_default();
        let cg_root = std::path::Path::new("/sys/fs/cgroup");
        let cg_dir = if rel.is_empty() {
            cg_root.to_path_buf()
        } else {
            cg_root.join(rel.trim_start_matches('/'))
        };
        let mem_events = std::fs::read_to_string(cg_dir.join("memory.events")).ok();
        let pressure = std::fs::read_to_string(cg_dir.join("memory.pressure")).ok();
        Some(proc_witness(
            &stat,
            &status,
            &cgroup,
            mem_events.as_deref(),
            pressure.as_deref(),
        ))
    }

    /// Per-field saturating delta post-minus-pre. A field is present only when
    /// BOTH ends read it — a one-sided reading is a guess about the stall, and
    /// the witness does not guess.
    fn delta_json(pre: &ProcWitness, post: &ProcWitness) -> serde_json::Value {
        let d = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => serde_json::json!(b.saturating_sub(a)),
            _ => serde_json::Value::Null,
        };
        serde_json::json!({
            "min_flt": d(pre.min_flt, post.min_flt),
            "maj_flt": d(pre.maj_flt, post.maj_flt),
            "voluntary_ctxt": d(pre.voluntary_ctxt, post.voluntary_ctxt),
            "nonvoluntary_ctxt": d(pre.nonvoluntary_ctxt, post.nonvoluntary_ctxt),
            "cg_high": d(pre.cg_high, post.cg_high),
            "cg_max": d(pre.cg_max, post.cg_max),
            "cg_oom": d(pre.cg_oom, post.cg_oom),
            "psi_some_total_us": d(pre.psi_some_total_us, post.psi_some_total_us),
            "psi_full_total_us": d(pre.psi_full_total_us, post.psi_full_total_us),
        })
    }
}

/// The `witness` payload for an incident: pre (first poll inside the stall),
/// post (at recovery), and the delta across the stall. `null` when the platform
/// has no witness at all — the incident is still filed.
fn witness_json(pre: Option<&ProcWitness>, post: Option<&ProcWitness>) -> serde_json::Value {
    match (pre, post) {
        (None, None) => serde_json::Value::Null,
        (pre, post) => serde_json::json!({
            "pre": pre.map(witness_fields).unwrap_or(serde_json::Value::Null),
            "post": post.map(witness_fields).unwrap_or(serde_json::Value::Null),
            "delta": match (pre, post) {
                (Some(pre), Some(post)) => ProcWitness::delta_json(pre, post),
                _ => serde_json::Value::Null,
            },
        }),
    }
}

fn witness_fields(w: &ProcWitness) -> serde_json::Value {
    serde_json::json!({
        "min_flt": w.min_flt,
        "maj_flt": w.maj_flt,
        "voluntary_ctxt": w.voluntary_ctxt,
        "nonvoluntary_ctxt": w.nonvoluntary_ctxt,
        "cg_path": w.cg_path,
        "cg_high": w.cg_high,
        "cg_max": w.cg_max,
        "cg_oom": w.cg_oom,
        "psi_some_total_us": w.psi_some_total_us,
        "psi_full_total_us": w.psi_full_total_us,
    })
}

fn watch_loop(home: PathBuf) {
    let threshold = ytrace::diagnosis::UI_BLOCK_THRESHOLD_MS;
    let mut tracker = BlockTracker::new();
    // Witness bookkeeping: captured on the FIRST poll inside a stall, so the
    // delta measures the stall window and not the recovery. The read runs on
    // this thread — the UI thread cannot delay or block it, which is the whole
    // reason the watchdog exists.
    let mut pre_witness: Option<ProcWitness> = None;
    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let stamped = UI_STAMP_MS.load(Ordering::Relaxed);
        let now = now_ms();
        let ui_stalled = stamped != 0 && now.saturating_sub(stamped) >= threshold;
        if ui_stalled && pre_witness.is_none() {
            pre_witness = ProcWitness::read();
        }
        let Some(measured) = tracker.observe(stamped, now, threshold) else {
            continue;
        };
        let post_witness = ProcWitness::read();
        let witness = witness_json(pre_witness.as_ref(), post_witness.as_ref());
        pre_witness = None;
        let sample = ytrace::diagnosis::UiBlockSample {
            gap_ms: measured.gap_ms,
            last_activity: last_activity(),
            blocks_per_min: Some(measured.blocks_per_min),
        };
        if let Some(incident) = ytrace::diagnosis::diagnose_ui_block(&sample) {
            file_incident(&home, &incident, measured.gap_ms, measured.blocks_per_min, witness);
        }
    }
}

fn file_incident(
    home: &std::path::Path,
    incident: &ytrace::diagnosis::Incident,
    gap_ms: u64,
    density: f64,
    witness: serde_json::Value,
) {
    let payload = ytrace::diagnosis::incident_payload(incident);
    // The bus, so `ytrace incidents` and the notebooks see it.
    crate::perf::ytrace_provider().incident("ui", "ui", "block", payload.clone());
    // And a queryable span-shaped record, so `ytrace query --category ui
    // --name block` can rank blocks by duration like any other probe. The
    // kernel witness rides here: the span record is the one analysts query,
    // and the witness is what turns "the GUI froze 37 s" into a verdict
    // (reclaim wall / swap storm / stop wedge) instead of a mystery.
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
            "witness": witness,
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

    // ── the kernel witness ──────────────────────────────────────────────
    // 2026-08-29, the GUI host: a 40.7 s whole-GUI stall left no verdict — the frozen
    // cgroup died with the scope. These parsers are the next stall's witness;
    // each one is tested against a fixture shaped like the real file.

    #[test]
    fn stat_faults_parse_after_the_last_paren() {
        // Field layout after comm: state ppid pgrp session tty tpgid flags
        // minflt cminflt majflt — so minflt is the 8th token, majflt the 10th.
        let stat = "977466 (yggterm) R 1 977452 977452 0 34816 10 4210999 12345 678 0 0 0";
        assert_eq!(parse_stat_faults(stat), Some((4210999, 678)));
        // `comm` may contain spaces AND parens; rsplit must see through both.
        let stat = "123 (WebKit Net S) S 1 1 1 0 -1 4194560 77 7 88 8 99 9";
        assert_eq!(parse_stat_faults(stat), Some((77, 88)));
        assert_eq!(parse_stat_faults("garbage"), None);
    }

    #[test]
    fn status_ctxt_reads_both_switch_counters() {
        let status = "Uid:\t1000\t1000\t1000\t1000\n\
                      voluntary_ctxt_switches:\t42022\n\
                      nonvoluntary_ctxt_switches:\t8117\n\
                      Threads:\t42\n";
        assert_eq!(parse_status_ctxt(status), Some((42022, 8117)));
        assert_eq!(parse_status_ctxt("voluntary_ctxt_switches:\t5\n"), None,
            "a one-sided reading must not masquerade as a pair");
    }

    #[test]
    fn cgroup_v2_path_comes_from_the_zero_line() {
        let cgroup = "12:pids:/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope\n\
                      0::/user.slice/user-1000.slice/user@1000.service/app.slice/yggterm-gui-977452.scope/gui";
        assert_eq!(
            parse_cgroup_v2_path(cgroup).as_deref(),
            Some("/user.slice/user-1000.slice/user@1000.service/app.slice/yggterm-gui-977452.scope/gui")
        );
        assert_eq!(parse_cgroup_v2_path("0::/\n").as_deref(), Some("/"));
        assert_eq!(parse_cgroup_v2_path("12:pids:/x\n"), None);
    }

    #[test]
    fn memory_events_reads_high_max_oom_but_not_kill_variants() {
        let events = "low 0\nhigh 12\nmax 3\noom 0\noom_kill 0\noom_group_kill 0\nsock_throttled 2\n";
        assert_eq!(parse_memory_events(events), (Some(12), Some(3), Some(0)));
        // `oom` must not be satisfied by `oom_kill`: an oom that never fired
        // while oom_kill fired is exactly the distinction the verdict needs.
        assert_ne!(parse_memory_events("oom_kill 4\n").2, Some(4));
    }

    #[test]
    fn pressure_totals_read_some_and_full() {
        let pressure = "some avg10=0.00 avg60=0.00 avg300=0.00 total=987654\n\
                        full avg10=0.00 avg60=0.00 avg300=0.00 total=12345\n";
        assert_eq!(parse_pressure_totals(pressure), (Some(987654), Some(12345)));
        assert_eq!(parse_pressure_totals("some avg10=0.00 avg60=0.00\n"), (None, None),
            "a missing total= is a missing reading, not a zero");
    }

    #[test]
    fn the_witness_degrades_per_field_never_to_zero() {
        let w = proc_witness(
            "1 (x) R 0 0 0 0 0 4194560 500 0 7 0 0 0",
            "voluntary_ctxt_switches:\t10\nnonvoluntary_ctxt_switches:\t2\n",
            "0::/a/b",
            Some("high 9\nmax 0\noom 0\n"),
            None, // pressure unreadable
        );
        assert_eq!(w.min_flt, Some(500));
        assert_eq!(w.maj_flt, Some(7));
        assert_eq!(w.cg_high, Some(9));
        assert_eq!(w.psi_some_total_us, None, "unreadable pressure stays absent");
        let fields = witness_fields(&w);
        assert_eq!(fields["psi_some_total_us"], serde_json::Value::Null,
            "an absent counter serializes null, never zero");
    }

    #[test]
    fn the_delta_needs_both_ends_and_saturates() {
        let pre = ProcWitness { min_flt: Some(100), maj_flt: Some(50), ..Default::default() };
        let post = ProcWitness { min_flt: Some(150), maj_flt: Some(10), ..Default::default() };
        let d = ProcWitness::delta_json(&pre, &post);
        assert_eq!(d["min_flt"], serde_json::json!(50));
        assert_eq!(d["maj_flt"], serde_json::json!(0), "counter resets saturate, never underflow");
        let one_sided = ProcWitness::delta_json(&pre, &ProcWitness::default());
        assert_eq!(one_sided["min_flt"], serde_json::Value::Null,
            "a one-sided reading is a guess, and the witness does not guess");
    }

    #[test]
    fn witness_json_is_null_when_nothing_was_read() {
        assert_eq!(witness_json(None, None), serde_json::Value::Null);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_reading_has_fault_counters() {
        let w = ProcWitness::read().expect("linux has /proc/self/stat");
        assert!(w.min_flt.is_some(), "the running test process has faulted pages");
    }
}
