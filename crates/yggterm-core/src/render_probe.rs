//! The render-side cost probe: what the GUI and its WebKit children actually burn.
//!
//! # Why this exists
//!
//! The app profiling system ([`crate::perf`]) is rich for the Rust side (daemon
//! latency, copy scans, remote resolves) and **completely blind to the render side**.
//! Measured on jojo 2026-07-25: `perf-telemetry.jsonl` only ever emitted
//! `terminal_js`, `daemon_request`, `daemon`, `remote`, `attach`, `background`,
//! `cli`, `copy_generation`, `sidebar`. There was no render/WebKit category at all,
//! while the GUI process plus one `WebKitWebProcess` were together holding ~105% of
//! one core for two hours on a 14 GiB laptop. The thing making the fan spin was the
//! one thing the instrument could not see, so the optimization pass had no "before".
//!
//! # The trap this module exists to avoid
//!
//! `ps %CPU` is a **lifetime average**, not current load. A process that pegged a
//! core for two hours and then idled reads the same as one pegging it now, and a
//! busy GUI on a 16-core box reads a reassuring `load average: 0.79`. Every number
//! here is therefore a **delta between two samples**: ticks consumed since the last
//! observation, divided by the wall time that actually elapsed.
//!
//! # What it deliberately does NOT claim
//!
//! WebKitGTK runs **one web process per profile, serving every surface on it**. The
//! kernel can attribute CPU to a process, so a *per-surface* CPU number would be a
//! fabrication dressed up as telemetry. This module reports **per-process** cost with
//! a role label, and leaves the caller to record how many surfaces shared that
//! process as plain context. That asymmetry is the finding, not a gap: it is the
//! argument for profile partitioning as the actual lever.
//!
//! # Wire format
//!
//! Samples are emitted as ordinary perf events whose `duration_ms` is **CPU
//! milliseconds consumed during the interval**. That is a real duration, so
//! `summarize_perf_telemetry` aggregates it with no changes and
//! `perf-summary --category render` lands in the same table as `copy_scan` and
//! `resolve_yggterm_binary` — directly comparable, which is the whole point.
//! Memory gauges ride along in the payload, where the duration aggregator ignores
//! them.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Perf category for every event this module writes.
pub const RENDER_PERF_CATEGORY: &str = "render";

/// Fallback USER_HZ when `/proc/self/auxv` cannot be read. `/proc/<pid>/stat` reports
/// CPU in USER_HZ, which is 100 on every mainstream Linux/x86_64 configuration
/// regardless of `CONFIG_HZ`. We prefer the real value from auxv (below) and only
/// fall back here, so an exotic arch degrades to a scaling error rather than a panic.
pub const DEFAULT_USER_HZ: u64 = 100;

/// `AT_CLKTCK` — the auxiliary-vector key holding USER_HZ.
const AT_CLKTCK: u64 = 17;

/// Which part of the render pipeline a process is.
///
/// Classification is by `comm`, which is what `/proc/<pid>/stat` gives us and is
/// stable across WebKitGTK versions in a way command lines are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenderRole {
    /// The yggterm GUI process itself: Dioxus shell, the one webview painting chrome.
    Gui,
    /// A WebKit web-content process: page layout, script, paint. One per profile.
    WebContent,
    /// A WebKit network process.
    WebNetwork,
    /// A WebKit GPU/compositing process.
    WebGpu,
    /// A headless or nested compositor we started (the shadow-client / server-render lane).
    Compositor,
    /// In the GUI's process tree but none of the above.
    Other,
}

impl RenderRole {
    /// Classify by `comm`. Note `comm` is truncated to 15 bytes by the kernel, which
    /// is exactly why the match is on prefixes: the real strings on disk are
    /// `WebKitWebProces`, `WebKitNetworkPr`, `WebKitGPUProces`.
    pub fn classify(comm: &str) -> RenderRole {
        let comm = comm.trim();
        if comm.starts_with("WebKitWebProc") {
            return RenderRole::WebContent;
        }
        if comm.starts_with("WebKitNetwork") {
            return RenderRole::WebNetwork;
        }
        if comm.starts_with("WebKitGPU") {
            return RenderRole::WebGpu;
        }
        if matches!(
            comm,
            "sway" | "cage" | "Xvfb" | "labwc" | "weston" | "wayfire"
        ) {
            return RenderRole::Compositor;
        }
        if comm == "yggterm" || comm.starts_with("yggterm-gui") {
            return RenderRole::Gui;
        }
        RenderRole::Other
    }

    /// Stable perf-event `name` for this role. Aggregating by name across pids answers
    /// "how much CPU did web content burn", which is the number the pass reports.
    pub fn as_str(&self) -> &'static str {
        match self {
            RenderRole::Gui => "gui",
            RenderRole::WebContent => "web_content",
            RenderRole::WebNetwork => "web_network",
            RenderRole::WebGpu => "web_gpu",
            RenderRole::Compositor => "compositor",
            RenderRole::Other => "other",
        }
    }
}

/// The fields of `/proc/<pid>/stat` this probe needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStat {
    pub pid: i32,
    pub comm: String,
    pub ppid: i32,
    /// Field 14, user-mode ticks.
    pub utime_ticks: u64,
    /// Field 15, kernel-mode ticks.
    pub stime_ticks: u64,
}

impl ProcStat {
    pub fn cpu_ticks(&self) -> u64 {
        self.utime_ticks.saturating_add(self.stime_ticks)
    }
}

/// Parse `/proc/<pid>/stat`.
///
/// The one subtlety that breaks naive `split_whitespace` parsers: field 2 is the
/// executable name **in parentheses, and it may contain spaces and parentheses**
/// (`(Web Content (1))` is legal). The kernel guarantees the comm is the text between
/// the FIRST `(` and the LAST `)`, so we split there and index the rest positionally.
pub fn parse_proc_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let pid: i32 = text.get(..open)?.trim().parse().ok()?;
    let comm = text.get(open + 1..close)?.to_string();
    // After the closing paren, field 3 is `state`, so `utime` (field 14) is index 11
    // and `stime` (field 15) is index 12 of this remainder.
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let ppid: i32 = rest.get(1)?.parse().ok()?;
    let utime_ticks: u64 = rest.get(11)?.parse().ok()?;
    let stime_ticks: u64 = rest.get(12)?.parse().ok()?;
    Some(ProcStat {
        pid,
        comm,
        ppid,
        utime_ticks,
        stime_ticks,
    })
}

/// Parse `Pss:` (KiB) out of `/proc/<pid>/smaps_rollup`.
///
/// PSS, not RSS, is the honest memory number for WebKit: several processes map the
/// same engine text, so summing RSS across a WebKit set double-counts it badly.
pub fn parse_smaps_rollup_pss_kb(text: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("Pss:") else {
            continue;
        };
        let value = rest.split_whitespace().next()?;
        return value.parse().ok();
    }
    None
}

/// Parse `VmRSS:` (KiB) out of `/proc/<pid>/status`.
pub fn parse_status_rss_kb(text: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        let value = rest.split_whitespace().next()?;
        return value.parse().ok();
    }
    None
}

/// Extract `AT_CLKTCK` from a raw 64-bit `auxv` blob (pairs of little-endian u64
/// `(key, value)`, terminated by a zero key).
///
/// This is how we get the true USER_HZ without taking a `libc` dependency into
/// `yggterm-core`.
pub fn parse_auxv_clk_tck(bytes: &[u8]) -> Option<u64> {
    for pair in bytes.chunks_exact(16) {
        let key = u64::from_le_bytes(pair[..8].try_into().ok()?);
        let value = u64::from_le_bytes(pair[8..16].try_into().ok()?);
        if key == 0 {
            break;
        }
        if key == AT_CLKTCK {
            return Some(value);
        }
    }
    None
}

/// USER_HZ for this process, from `/proc/self/auxv`, falling back to
/// [`DEFAULT_USER_HZ`].
pub fn user_hz() -> u64 {
    fs::read("/proc/self/auxv")
        .ok()
        .and_then(|bytes| parse_auxv_clk_tck(&bytes))
        .filter(|hz| *hz > 0)
        .unwrap_or(DEFAULT_USER_HZ)
}

/// Convert a tick delta into CPU milliseconds. Pure so the arithmetic is testable.
pub fn cpu_ms_from_ticks(delta_ticks: u64, user_hz: u64) -> f64 {
    let hz = if user_hz == 0 {
        DEFAULT_USER_HZ
    } else {
        user_hz
    };
    (delta_ticks as f64) * 1000.0 / (hz as f64)
}

/// Fraction of ONE core used over the interval, as a convenience for readers.
/// `1.0` means one core fully pegged; on a 16-core box that is `load average` 1.
pub fn core_fraction(cpu_ms: f64, interval_ms: f64) -> f64 {
    if interval_ms <= 0.0 {
        return 0.0;
    }
    cpu_ms / interval_ms
}

/// One process's render cost over the interval since the previous sample.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderProcSample {
    pub pid: i32,
    pub ppid: i32,
    pub role: RenderRole,
    pub comm: String,
    /// CPU milliseconds consumed since the previous sample. This is what becomes
    /// `duration_ms`.
    pub cpu_ms: f64,
    /// Wall milliseconds the delta was measured over.
    pub interval_ms: f64,
    pub rss_kb: Option<u64>,
    pub pss_kb: Option<u64>,
}

impl RenderProcSample {
    pub fn core_fraction(&self) -> f64 {
        core_fraction(self.cpu_ms, self.interval_ms)
    }
}

/// A process observed in the tree, before deltas are applied. Exposed so the sampler
/// can be unit-tested against a synthetic tree instead of a live `/proc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderProcObservation {
    pub stat: ProcStat,
    pub rss_kb: Option<u64>,
    pub pss_kb: Option<u64>,
}

/// Holds the previous observation so every reported number is a delta.
///
/// The first `sample()` after construction reports nothing: with no prior
/// observation the only available number would be the lifetime average, which is
/// precisely the lie this module exists to avoid.
#[derive(Debug, Default)]
pub struct RenderProbe {
    last_ticks: BTreeMap<i32, u64>,
    last_at_ms: Option<u64>,
    user_hz: Option<u64>,
}

impl RenderProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override USER_HZ (tests; exotic arches).
    pub fn with_user_hz(mut self, hz: u64) -> Self {
        self.user_hz = Some(hz);
        self
    }

    fn hz(&self) -> u64 {
        self.user_hz.unwrap_or_else(user_hz)
    }

    /// Turn a set of observations into deltas against the previous call.
    ///
    /// `now_ms` is monotonic wall time supplied by the caller so this stays pure and
    /// testable. Processes that vanished are forgotten; processes that appeared are
    /// recorded and reported only from their *second* observation onward.
    pub fn observe(
        &mut self,
        observations: &[RenderProcObservation],
        now_ms: u64,
    ) -> Vec<RenderProcSample> {
        let hz = self.hz();
        let interval_ms = self
            .last_at_ms
            .map(|last| now_ms.saturating_sub(last) as f64)
            .unwrap_or(0.0);
        let mut samples = Vec::new();
        let mut next_ticks = BTreeMap::new();
        for observation in observations {
            let pid = observation.stat.pid;
            let ticks = observation.stat.cpu_ticks();
            next_ticks.insert(pid, ticks);
            let Some(previous) = self.last_ticks.get(&pid).copied() else {
                continue;
            };
            if interval_ms <= 0.0 {
                continue;
            }
            // A tick counter that went BACKWARDS means pid reuse, not negative work.
            let delta = ticks.saturating_sub(previous);
            samples.push(RenderProcSample {
                pid,
                ppid: observation.stat.ppid,
                role: RenderRole::classify(&observation.stat.comm),
                comm: observation.stat.comm.clone(),
                cpu_ms: cpu_ms_from_ticks(delta, hz),
                interval_ms,
                rss_kb: observation.rss_kb,
                pss_kb: observation.pss_kb,
            });
        }
        self.last_ticks = next_ticks;
        self.last_at_ms = Some(now_ms);
        samples
    }
}

/// Read `/proc` and collect `root_pid` plus every descendant.
///
/// Only the GUI's own tree is walked, so the probe never accounts for unrelated
/// processes on a shared host (dev and oc are LXC containers on one kernel: a
/// whole-system walk would attribute a neighbour's load to us).
pub fn observe_process_tree(root_pid: i32) -> Vec<RenderProcObservation> {
    let mut by_ppid: BTreeMap<i32, Vec<ProcStat>> = BTreeMap::new();
    let mut roots: Vec<ProcStat> = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(pid) = name.parse::<i32>() else {
            continue;
        };
        let Ok(text) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(stat) = parse_proc_stat(&text) else {
            continue;
        };
        if stat.pid == root_pid {
            roots.push(stat);
        } else {
            by_ppid.entry(stat.ppid).or_default().push(stat);
        }
    }
    let mut observations = Vec::new();
    let mut queue = roots;
    while let Some(stat) = queue.pop() {
        if let Some(children) = by_ppid.remove(&stat.pid) {
            queue.extend(children);
        }
        let pid = stat.pid;
        observations.push(RenderProcObservation {
            rss_kb: fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|text| parse_status_rss_kb(&text)),
            pss_kb: fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
                .ok()
                .and_then(|text| parse_smaps_rollup_pss_kb(&text)),
            stat,
        });
    }
    observations
}

/// One role's cost over the interval, summed across the processes filling that role.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRoleRollup {
    pub role: RenderRole,
    pub cpu_ms: f64,
    /// PSS where available, else RSS, summed across the role's processes.
    pub mem_kb: u64,
    pub procs: usize,
    pub interval_ms: f64,
    /// The single busiest process in this role, and its share. This is what exposed
    /// "one web process holds all the CPU while its siblings sit at zero" — a fact
    /// invisible in a role total alone.
    pub hot_pid: i32,
    pub hot_cpu_ms: f64,
}

impl RenderRoleRollup {
    pub fn core_fraction(&self) -> f64 {
        core_fraction(self.cpu_ms, self.interval_ms)
    }
}

/// Roll per-process samples up by role.
pub fn roll_up_roles(samples: &[RenderProcSample]) -> Vec<RenderRoleRollup> {
    let mut by_role: BTreeMap<RenderRole, RenderRoleRollup> = BTreeMap::new();
    for sample in samples {
        let entry = by_role
            .entry(sample.role)
            .or_insert_with(|| RenderRoleRollup {
                role: sample.role,
                cpu_ms: 0.0,
                mem_kb: 0,
                procs: 0,
                interval_ms: sample.interval_ms,
                hot_pid: sample.pid,
                hot_cpu_ms: f64::MIN,
            });
        entry.cpu_ms += sample.cpu_ms;
        entry.mem_kb += sample.pss_kb.or(sample.rss_kb).unwrap_or(0);
        entry.procs += 1;
        if sample.cpu_ms > entry.hot_cpu_ms {
            entry.hot_cpu_ms = sample.cpu_ms;
            entry.hot_pid = sample.pid;
        }
    }
    by_role.into_values().collect()
}

/// Write ONE perf event per role, the shape the continuous in-app sampler uses.
///
/// Per-role rather than per-process on purpose. A background sampler runs forever, and
/// the telemetry log is a shared, size-capped resource: emitting every process every
/// tick would crowd out the daemon spans that share the log. Role totals lose no CPU
/// accuracy (the sum is the sum) and `hot_pid` preserves the one per-process fact that
/// actually drove a finding. Use [`emit_render_perf_events`] when full per-process
/// detail is wanted for a one-shot read.
///
/// Roles with no CPU and no memory are skipped entirely, so a quiet tree is nearly free.
pub fn emit_render_role_events(home: &Path, rollups: &[RenderRoleRollup], context: &Value) {
    if !crate::perf::perf_profiling_enabled() {
        return;
    }
    for rollup in rollups {
        if rollup.cpu_ms <= 0.0 && rollup.mem_kb == 0 {
            continue;
        }
        let mut payload = json!({
            "duration_ms": rollup.cpu_ms,
            "core_fraction": rollup.core_fraction(),
            "interval_ms": rollup.interval_ms,
            "role": rollup.role.as_str(),
            "procs": rollup.procs,
            "mem_kb": rollup.mem_kb,
            "hot_pid": rollup.hot_pid,
            "hot_cpu_ms": rollup.hot_cpu_ms.max(0.0),
        });
        if let Some(extra) = context.as_object()
            && let Some(object) = payload.as_object_mut()
        {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        crate::perf::append_perf_event(home, RENDER_PERF_CATEGORY, rollup.role.as_str(), payload);
    }
}

/// Write one perf event per sampled process under the `render` category.
///
/// `context` is merged into every payload: the caller passes what the kernel cannot
/// know (how many web surfaces are live, how many are soft-stashed, whether the
/// window is visible). Keeping that as caller-supplied context rather than a derived
/// per-surface CPU number is the honesty boundary described in the module docs.
pub fn emit_render_perf_events(home: &Path, samples: &[RenderProcSample], context: &Value) {
    if !crate::perf::perf_profiling_enabled() {
        return;
    }
    for sample in samples {
        let mut payload = json!({
            "duration_ms": sample.cpu_ms,
            "core_fraction": sample.core_fraction(),
            "interval_ms": sample.interval_ms,
            "pid": sample.pid,
            "ppid": sample.ppid,
            "comm": sample.comm,
            "role": sample.role.as_str(),
        });
        if let Some(rss_kb) = sample.rss_kb {
            payload["rss_kb"] = json!(rss_kb);
        }
        if let Some(pss_kb) = sample.pss_kb {
            payload["pss_kb"] = json!(pss_kb);
        }
        if let Some(extra) = context.as_object() {
            if let Some(object) = payload.as_object_mut() {
                for (key, value) in extra {
                    object.insert(key.clone(), value.clone());
                }
            }
        }
        crate::perf::append_perf_event(home, RENDER_PERF_CATEGORY, sample.role.as_str(), payload);
    }
}

/// One `render-top` read: the whole tree's cost over one interval, rolled up by
/// role plus the busiest processes.
///
/// Serializable because `server render-top --json` is the machine-readable
/// half of the same read — one report type, so the table and the JSON can
/// never disagree about a number.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderTopReport {
    pub root_pid: i32,
    pub interval_ms: f64,
    pub user_hz: u64,
    pub process_count: usize,
    pub roles: Vec<RenderRoleRollupReport>,
    pub top_processes: Vec<RenderProcSampleReport>,
    pub total_cpu_ms: f64,
    pub total_core_fraction: f64,
    pub total_mem_kb: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderRoleRollupReport {
    pub role: &'static str,
    pub cpu_ms: f64,
    pub core_fraction: f64,
    pub mem_kb: u64,
    pub procs: usize,
    pub hot_pid: i32,
    pub hot_cpu_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderProcSampleReport {
    pub pid: i32,
    pub ppid: i32,
    pub comm: String,
    pub role: &'static str,
    pub cpu_ms: f64,
    pub core_fraction: f64,
    pub mem_kb: u64,
}

impl RenderTopReport {
    /// Pure: no `/proc`, no sleep. Everything the command prints is decided
    /// here, so the ranking and the totals are testable against a synthetic
    /// tree.
    pub fn from_samples(
        root_pid: i32,
        interval_ms: f64,
        user_hz: u64,
        process_count: usize,
        samples: &[RenderProcSample],
        top: usize,
    ) -> Self {
        let roles: Vec<RenderRoleRollupReport> = roll_up_roles(samples)
            .into_iter()
            .map(|rollup| RenderRoleRollupReport {
                role: rollup.role.as_str(),
                cpu_ms: rollup.cpu_ms,
                core_fraction: rollup.core_fraction(),
                mem_kb: rollup.mem_kb,
                procs: rollup.procs,
                hot_pid: rollup.hot_pid,
                hot_cpu_ms: rollup.hot_cpu_ms,
            })
            .collect();
        let mut ranked: Vec<&RenderProcSample> = samples.iter().collect();
        // Descending by cpu_ms, then by pid so equal costs order the same way
        // on every run — a "top processes" list that reshuffles between reads
        // is unreadable as a before/after.
        ranked.sort_by(|left, right| {
            right
                .cpu_ms
                .partial_cmp(&left.cpu_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.pid.cmp(&right.pid))
        });
        let top_processes = ranked
            .into_iter()
            .take(top)
            .map(|sample| RenderProcSampleReport {
                pid: sample.pid,
                ppid: sample.ppid,
                comm: sample.comm.clone(),
                role: sample.role.as_str(),
                cpu_ms: sample.cpu_ms,
                core_fraction: sample.core_fraction(),
                mem_kb: sample.pss_kb.or(sample.rss_kb).unwrap_or(0),
            })
            .collect();
        Self {
            root_pid,
            interval_ms,
            user_hz,
            process_count,
            total_cpu_ms: roles.iter().map(|role| role.cpu_ms).sum(),
            total_core_fraction: roles.iter().map(|role| role.core_fraction).sum(),
            total_mem_kb: roles.iter().map(|role| role.mem_kb).sum(),
            roles,
            top_processes,
        }
    }
}

/// Observe a process tree, wait, observe again, and report the delta.
///
/// Deliberately does NOT emit perf events. The GUI's continuous tick owns the
/// `render` category; a CLI-triggered write into it would make that series
/// depend on how often an agent ran the command, and `perf-summary --category
/// render` would silently be mixing two samplers with different intervals.
pub fn render_top_sample(root_pid: i32, interval_ms: u64, top: usize) -> Option<RenderTopReport> {
    let mut probe = RenderProbe::new();
    let started = std::time::Instant::now();
    let first = observe_process_tree(root_pid);
    if first.is_empty() {
        return None;
    }
    probe.observe(&first, started.elapsed().as_millis() as u64);
    std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    let second = observe_process_tree(root_pid);
    let samples = probe.observe(&second, started.elapsed().as_millis() as u64);
    let measured_interval_ms = samples
        .first()
        .map(|sample| sample.interval_ms)
        .unwrap_or(interval_ms as f64);
    Some(RenderTopReport::from_samples(
        root_pid,
        measured_interval_ms,
        user_hz(),
        second.len(),
        &samples,
        top,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `/proc/<pid>/stat` shape, trimmed after the fields we read.
    fn stat_line(pid: i32, comm: &str, ppid: i32, utime: u64, stime: u64) -> String {
        let mut fields = vec![
            ppid.to_string(),
            "1000".into(),
            "1000".into(),
            "0".into(),
            "-1".into(),
            "4194304".into(),
            "9999".into(),
            "0".into(),
            "12".into(),
            "0".into(),
            utime.to_string(),
            stime.to_string(),
            "5".into(),
            "6".into(),
        ];
        fields.extend((0..10).map(|_| "0".to_string()));
        format!("{pid} ({comm}) S {}", fields.join(" "))
    }

    fn proc_sample(pid: i32, comm: &str, cpu_ms: f64, mem_kb: u64) -> RenderProcSample {
        RenderProcSample {
            pid,
            ppid: 1,
            role: RenderRole::classify(comm),
            comm: comm.to_string(),
            cpu_ms,
            interval_ms: 5_000.0,
            rss_kb: Some(mem_kb),
            pss_kb: None,
        }
    }

    /// The whole report is decided by `from_samples`, so it can be asserted
    /// against a synthetic tree — no `/proc`, no sleep, no live GUI.
    #[test]
    fn render_top_report_rolls_up_roles_and_ranks_processes_by_cpu() {
        let samples = vec![
            proc_sample(10, "yggterm", 220.0, 300_000),
            proc_sample(11, "WebKitWebProces", 1_360.0, 700_000),
            proc_sample(12, "WebKitWebProces", 4.0, 120_000),
            proc_sample(13, "WebKitNetworkPr", 8.0, 40_000),
        ];

        let report = RenderTopReport::from_samples(9, 5_000.0, 100, 4, &samples, 2);

        let web = report
            .roles
            .iter()
            .find(|role| role.role == "web_content")
            .expect("web_content rollup");
        assert_eq!(web.procs, 2, "both web processes roll into one role");
        assert_eq!(web.cpu_ms, 1_364.0);
        assert_eq!(
            web.hot_pid, 11,
            "one web process holding all the CPU is the fact a role total hides"
        );
        assert!((web.core_fraction - 0.2728).abs() < 1e-6);

        assert_eq!(report.top_processes.len(), 2, "--top is respected");
        assert_eq!(report.top_processes[0].pid, 11);
        assert_eq!(report.top_processes[1].pid, 10);
        assert_eq!(report.top_processes[0].role, "web_content");

        assert_eq!(report.total_cpu_ms, 1_592.0);
        assert_eq!(report.total_mem_kb, 1_160_000);
        assert!((report.total_core_fraction - 0.3184).abs() < 1e-6);
    }

    /// Two processes with identical cost must not reshuffle between reads —
    /// a "top processes" list that reorders on its own is unreadable as a
    /// before/after.
    #[test]
    fn render_top_ranking_is_stable_for_equal_cost_processes() {
        let samples = vec![
            proc_sample(30, "WebKitWebProces", 100.0, 1),
            proc_sample(20, "WebKitWebProces", 100.0, 1),
        ];
        let report = RenderTopReport::from_samples(1, 5_000.0, 100, 2, &samples, 10);
        assert_eq!(
            report
                .top_processes
                .iter()
                .map(|sample| sample.pid)
                .collect::<Vec<_>>(),
            vec![20, 30]
        );
    }

    #[test]
    fn parses_proc_stat_fields() {
        let stat = parse_proc_stat(&stat_line(4242, "WebKitWebProces", 4200, 1500, 300)).unwrap();
        assert_eq!(stat.pid, 4242);
        assert_eq!(stat.comm, "WebKitWebProces");
        assert_eq!(stat.ppid, 4200);
        assert_eq!(stat.utime_ticks, 1500);
        assert_eq!(stat.stime_ticks, 300);
        assert_eq!(stat.cpu_ticks(), 1800);
    }

    /// The parser bug this guards against: a comm containing spaces AND parentheses
    /// derails positional `split_whitespace`, silently yielding another field's value
    /// as CPU time.
    #[test]
    fn parses_comm_containing_spaces_and_parens() {
        let stat = parse_proc_stat(&stat_line(7, "Web Content (1)", 3, 40, 2)).unwrap();
        assert_eq!(stat.comm, "Web Content (1)");
        assert_eq!(stat.pid, 7);
        assert_eq!(stat.ppid, 3);
        assert_eq!(stat.cpu_ticks(), 42);
    }

    #[test]
    fn rejects_malformed_stat() {
        assert!(parse_proc_stat("").is_none());
        assert!(parse_proc_stat("123 no-parens S 1 2 3").is_none());
        assert!(parse_proc_stat("123 (short) S 1").is_none());
    }

    #[test]
    fn classifies_webkit_roles_from_truncated_comm() {
        assert_eq!(
            RenderRole::classify("WebKitWebProces"),
            RenderRole::WebContent
        );
        assert_eq!(
            RenderRole::classify("WebKitNetworkPr"),
            RenderRole::WebNetwork
        );
        assert_eq!(RenderRole::classify("WebKitGPUProces"), RenderRole::WebGpu);
        assert_eq!(RenderRole::classify("yggterm"), RenderRole::Gui);
        assert_eq!(RenderRole::classify("sway"), RenderRole::Compositor);
        assert_eq!(RenderRole::classify("Xvfb"), RenderRole::Compositor);
        assert_eq!(RenderRole::classify("bash"), RenderRole::Other);
    }

    #[test]
    fn parses_memory_gauges() {
        let rollup =
            "Rss:              123456 kB\nPss:               65432 kB\nShared_Clean: 1 kB\n";
        assert_eq!(parse_smaps_rollup_pss_kb(rollup), Some(65432));
        let status = "Name:\tyggterm\nVmPeak:\t 900 kB\nVmRSS:\t  543284 kB\nThreads:\t42\n";
        assert_eq!(parse_status_rss_kb(status), Some(543284));
        assert_eq!(parse_smaps_rollup_pss_kb("no pss here"), None);
        assert_eq!(parse_status_rss_kb("Name:\tx\n"), None);
    }

    #[test]
    fn parses_clk_tck_from_auxv() {
        let mut bytes = Vec::new();
        // AT_PAGESZ = 6 -> 4096, then AT_CLKTCK = 17 -> 100, then the null terminator.
        for (key, value) in [(6u64, 4096u64), (AT_CLKTCK, 100), (0, 0)] {
            bytes.extend_from_slice(&key.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(parse_auxv_clk_tck(&bytes), Some(100));
        assert_eq!(parse_auxv_clk_tck(&[]), None);
    }

    /// A key AFTER the null terminator must not be read: the kernel ends the vector
    /// there and anything beyond is not ours to interpret.
    #[test]
    fn auxv_stops_at_null_terminator() {
        let mut bytes = Vec::new();
        for (key, value) in [(0u64, 0u64), (AT_CLKTCK, 1024)] {
            bytes.extend_from_slice(&key.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(parse_auxv_clk_tck(&bytes), None);
    }

    #[test]
    fn converts_ticks_to_cpu_ms_and_core_fraction() {
        // 100 ticks at 100 Hz is one full second of CPU.
        assert_eq!(cpu_ms_from_ticks(100, 100), 1000.0);
        assert_eq!(cpu_ms_from_ticks(50, 100), 500.0);
        // A zero HZ must degrade, not divide by zero.
        assert_eq!(cpu_ms_from_ticks(100, 0), 1000.0);
        // 1000 CPU ms over a 1000 ms wall interval is exactly one core.
        assert_eq!(core_fraction(1000.0, 1000.0), 1.0);
        assert_eq!(core_fraction(350.0, 1000.0), 0.35);
        assert_eq!(core_fraction(10.0, 0.0), 0.0);
    }

    fn observation(pid: i32, comm: &str, ppid: i32, ticks: u64) -> RenderProcObservation {
        RenderProcObservation {
            stat: parse_proc_stat(&stat_line(pid, comm, ppid, ticks, 0)).unwrap(),
            rss_kb: Some(1000),
            pss_kb: Some(600),
        }
    }

    /// THE anti-regression test for this whole module: the first observation must
    /// report nothing. If it ever reports, it is reporting a lifetime average, which
    /// is the `ps %CPU` trap that made a pegged GUI look idle.
    #[test]
    fn first_observation_reports_nothing() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        let samples = probe.observe(&[observation(10, "yggterm", 1, 100_000)], 1_000);
        assert!(
            samples.is_empty(),
            "first sample must not report a lifetime average"
        );
    }

    #[test]
    fn second_observation_reports_the_delta_only() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe(&[observation(10, "yggterm", 1, 100_000)], 1_000);
        // 50 ticks (500 CPU ms) burned over a 1000 ms interval = half a core.
        let samples = probe.observe(&[observation(10, "yggterm", 1, 100_050)], 2_000);
        assert_eq!(samples.len(), 1);
        let sample = &samples[0];
        assert_eq!(sample.cpu_ms, 500.0);
        assert_eq!(sample.interval_ms, 1000.0);
        assert_eq!(sample.core_fraction(), 0.5);
        assert_eq!(sample.role, RenderRole::Gui);
    }

    /// PID reuse resets the counter downward. That must read as zero work, never as a
    /// huge negative that underflows into an astronomical delta.
    #[test]
    fn tick_counter_going_backwards_reads_as_zero() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe(&[observation(10, "yggterm", 1, 5_000)], 1_000);
        let samples = probe.observe(&[observation(10, "yggterm", 1, 12)], 2_000);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].cpu_ms, 0.0);
    }

    #[test]
    fn vanished_process_is_forgotten_and_new_one_waits_a_turn() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe(&[observation(10, "yggterm", 1, 100)], 1_000);
        // pid 10 gone, pid 11 new: nothing reportable this turn.
        let samples = probe.observe(&[observation(11, "WebKitWebProces", 10, 900)], 2_000);
        assert!(samples.is_empty());
        // Now pid 11 has a baseline and reports its own delta.
        let samples = probe.observe(&[observation(11, "WebKitWebProces", 10, 910)], 3_000);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].role, RenderRole::WebContent);
        assert_eq!(samples[0].cpu_ms, 100.0);
    }

    #[test]
    fn rolls_up_by_role_and_prefers_pss_over_rss() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        let first = [
            observation(10, "yggterm", 1, 0),
            observation(11, "WebKitWebProces", 10, 0),
            observation(12, "WebKitWebProces", 10, 0),
        ];
        probe.observe(&first, 1_000);
        let second = [
            observation(10, "yggterm", 1, 70),
            observation(11, "WebKitWebProces", 10, 30),
            observation(12, "WebKitWebProces", 10, 5),
        ];
        let samples = probe.observe(&second, 2_000);
        let rolled = roll_up_roles(&samples);
        assert_eq!(rolled.len(), 2);
        let web = rolled
            .iter()
            .find(|rollup| rollup.role == RenderRole::WebContent)
            .unwrap();
        assert_eq!(
            web.cpu_ms, 350.0,
            "two web processes: 30+5 ticks = 350 CPU ms"
        );
        assert_eq!(
            web.mem_kb, 1200,
            "PSS preferred over RSS, summed across the pair"
        );
        assert_eq!(web.procs, 2);
        // The asymmetry is the finding, so the hot process must be identified.
        assert_eq!(web.hot_pid, 11);
        assert_eq!(web.hot_cpu_ms, 300.0);
        let gui = rolled
            .iter()
            .find(|rollup| rollup.role == RenderRole::Gui)
            .unwrap();
        assert_eq!(gui.cpu_ms, 700.0);
        assert_eq!(gui.core_fraction(), 0.7);
    }

    /// The continuous sampler runs forever into a size-capped shared log, so a quiet
    /// tree must cost almost nothing: roles with neither CPU nor memory are dropped.
    #[test]
    fn role_events_skip_roles_with_no_cpu_and_no_memory() {
        let home =
            std::env::temp_dir().join(format!("yggterm-render-role-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        crate::perf::set_perf_profiling_enabled(true);

        let idle = RenderRoleRollup {
            role: RenderRole::WebGpu,
            cpu_ms: 0.0,
            mem_kb: 0,
            procs: 1,
            interval_ms: 60_000.0,
            hot_pid: 99,
            hot_cpu_ms: 0.0,
        };
        let busy = RenderRoleRollup {
            role: RenderRole::WebContent,
            cpu_ms: 250.0,
            mem_kb: 4096,
            procs: 3,
            interval_ms: 60_000.0,
            hot_pid: 42,
            hot_cpu_ms: 250.0,
        };
        emit_render_role_events(&home, &[idle, busy], &json!({ "web_surfaces": 2 }));

        let summary = crate::perf::summarize_perf_telemetry(&home, None, Some("render"));
        assert_eq!(summary.len(), 1, "the idle role must not be written at all");
        assert_eq!(summary[0].name, "web_content");
        assert_eq!(summary[0].total_ms, 250.0);
        let log = fs::read_to_string(crate::perf::perf_telemetry_path(&home)).unwrap();
        assert!(log.contains("\"hot_pid\":42"));
        assert!(log.contains("\"procs\":3"));
        assert!(log.contains("\"web_surfaces\":2"));
        assert!(!log.contains("web_gpu"));
        let _ = fs::remove_dir_all(&home);
    }

    /// The emitted event must be shaped so `summarize_perf_telemetry` aggregates it
    /// with no aggregator changes: `duration_ms` present, in CPU milliseconds.
    #[test]
    fn emits_events_the_existing_aggregator_can_read() {
        let home =
            std::env::temp_dir().join(format!("yggterm-render-probe-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        crate::perf::set_perf_profiling_enabled(true);

        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe(&[observation(10, "WebKitWebProces", 1, 0)], 1_000);
        let samples = probe.observe(&[observation(10, "WebKitWebProces", 1, 25)], 2_000);
        emit_render_perf_events(&home, &samples, &json!({ "web_surface_live_count": 3 }));

        let summary = crate::perf::summarize_perf_telemetry(&home, None, Some("render"));
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].category, "render");
        assert_eq!(summary[0].name, "web_content");
        assert_eq!(summary[0].count, 1);
        assert_eq!(summary[0].total_ms, 250.0);

        // The caller-supplied context must survive into the payload, since that is
        // how many-surfaces-per-process is recorded without faking per-surface CPU.
        let log = fs::read_to_string(crate::perf::perf_telemetry_path(&home)).unwrap();
        assert!(log.contains("\"web_surface_live_count\":3"));
        assert!(log.contains("\"role\":\"web_content\""));
        let _ = fs::remove_dir_all(&home);
    }
}
