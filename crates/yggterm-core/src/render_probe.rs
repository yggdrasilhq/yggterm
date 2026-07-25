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
//! # The GPU gauge
//!
//! CPU alone cannot tell "cheap because the GPU did the work" from "cheap because
//! nothing happened", and that ambiguity is exactly what let a host run with its GPU
//! switched off for months. So every sample also carries `drm-engine-*` time from
//! `/proc/<pid>/fdinfo/*` ([`parse_fdinfo_drm_engine_ns`]) — nonzero and rising means
//! the GPU really is rasterizing. `None` there means the counter was UNREADABLE and
//! is never rendered as a zero.
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

/// Where a [`ProcMemory`] reading came from. Carried in the value rather than implied
/// by the call site, because the two sources are not interchangeable: `smaps_rollup`
/// knows PSS and anonymous memory, `status` knows only RSS. A caller that cannot tell
/// which it got would silently read the fallback's zeroes as real numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcMemorySource {
    SmapsRollup,
    StatusVmRss,
}

/// How much memory one process is using.
///
/// This module is the ONE owner of that question. The GUI shell used to carry its own
/// `smaps_rollup` parser for the allocator-trim chore while this one parsed the same
/// file per pid — two encodings of one concept, free to drift apart on the next kernel
/// field rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcMemory {
    pub rss_kb: u64,
    /// PSS, or 0 when the reading came from the `status` fallback.
    pub pss_kb: u64,
    /// Anonymous (non-file-backed) memory, or 0 on the `status` fallback. This is the
    /// number the allocator-trim chore moves.
    pub anonymous_kb: u64,
    pub source: ProcMemorySource,
}

impl ProcMemory {
    /// The honest single number for "how much memory does this process cost".
    ///
    /// PSS where we have it: several WebKit processes map the same engine text, so
    /// summing RSS across a WebKit set double-counts it badly. RSS is the fallback,
    /// and it is only ever reached when PSS was genuinely unavailable.
    pub fn preferred_kb(&self) -> u64 {
        if self.pss_kb > 0 {
            self.pss_kb
        } else {
            self.rss_kb
        }
    }
}

/// Parse `/proc/<pid>/smaps_rollup` in one pass.
///
/// `None` when the text carries no `Rss:` at all (an empty or truncated rollup, which
/// the kernel produces for a process that is exiting) — the same guard the shell's
/// allocator-trim chore has always applied before deciding it is worth trimming.
pub fn parse_smaps_rollup(text: &str) -> Option<ProcMemory> {
    let mut memory = ProcMemory {
        rss_kb: 0,
        pss_kb: 0,
        anonymous_kb: 0,
        source: ProcMemorySource::SmapsRollup,
    };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(label) = parts.next() else {
            continue;
        };
        let value = parts
            .next()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or_default();
        match label {
            "Rss:" => memory.rss_kb = value,
            "Pss:" => memory.pss_kb = value,
            "Anonymous:" => memory.anonymous_kb = value,
            _ => {}
        }
    }
    (memory.rss_kb > 0).then_some(memory)
}

/// Parse `VmRSS:` (KiB) out of `/proc/<pid>/status`. Private on purpose: it is the
/// FALLBACK half of [`read_process_memory`] and has no business being anyone's answer
/// on its own, because it cannot see PSS.
fn parse_status_rss_kb(text: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        let value = rest.split_whitespace().next()?;
        return value.parse().ok();
    }
    None
}

/// Read one process's memory: `smaps_rollup` first, `status` VmRSS only if that failed.
///
/// The fallback is legal precisely because it lives inside the one owner and is
/// LABELLED in the value it returns, so it can never masquerade as a PSS reading.
/// `smaps_rollup` is unreadable on some hardened kernels and absent for kernel
/// threads, and one file per pid instead of two halves this probe's syscall cost.
pub fn read_process_memory(pid: i32) -> Option<ProcMemory> {
    if let Some(memory) = fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
        .ok()
        .and_then(|text| parse_smaps_rollup(&text))
    {
        return Some(memory);
    }
    let rss_kb = fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|text| parse_status_rss_kb(&text))?;
    Some(ProcMemory {
        rss_kb,
        pss_kb: 0,
        anonymous_kb: 0,
        source: ProcMemorySource::StatusVmRss,
    })
}

/// Sum every `drm-engine-*` counter (nanoseconds of GPU engine time) in one
/// `/proc/<pid>/fdinfo/<fd>` file. `None` when the file names no DRM engine at all,
/// which is how "this fd is not a GPU fd" stays distinct from "this GPU did no work".
///
/// Exact live format, captured from `krunner` on the GUI host 2026-07-25 — tab
/// separated, one engine per line, alongside `drm-driver:\tamdgpu` and a block of
/// `drm-memory-*` lines that must NOT be summed into the time:
///
/// ```text
/// drm-driver:     amdgpu
/// drm-engine-gfx: 56369242 ns
/// drm-engine-compute:     3434919 ns
/// ```
///
/// This is THE discriminator for the forced-software-GL bug: the GUI's
/// `WebKitWebProcess` holds no DRM fd at all while an ordinary desktop app on the same
/// machine shows tens of milliseconds of `drm-engine-gfx`. "The GPU works for
/// everything except us" is a fact this number states and a CPU number cannot.
pub fn parse_fdinfo_drm_engine_ns(text: &str) -> Option<u64> {
    let mut total: Option<u64> = None;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("drm-engine-") else {
            continue;
        };
        let Some((_engine, value)) = rest.split_once(':') else {
            continue;
        };
        let mut fields = value.split_whitespace();
        let Some(nanos) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        // The kernel writes the unit; anything else is a key we do not understand.
        if fields.next() != Some("ns") {
            continue;
        }
        total = Some(total.unwrap_or(0).saturating_add(nanos));
    }
    total
}

/// Total GPU engine nanoseconds across every fd this process holds.
///
/// `None` means WE COULD NOT LOOK (the fdinfo directory is unreadable — another user's
/// process, or a hardened `/proc`). `Some(0)` means we looked and this process is
/// doing no GPU work. Collapsing those two into a zero is the exact mistake that
/// produced the forced-software-GL bug: one EACCES read as "there is no GPU here".
pub fn drm_engine_ns_for_pid(pid: i32) -> Option<u64> {
    let entries = fs::read_dir(format!("/proc/{pid}/fdinfo")).ok()?;
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(nanos) = parse_fdinfo_drm_engine_ns(&text) {
            total = total.saturating_add(nanos);
        }
    }
    Some(total)
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
    /// Memory as one labelled reading, never three parallel Options that could come
    /// from different sources. `None` when `/proc` would not answer for this pid.
    pub memory: Option<ProcMemory>,
    /// GPU engine nanoseconds consumed since the previous sample, a delta exactly like
    /// `cpu_ms`. `None` when the counter was unreadable at either end of the interval —
    /// never a zero standing in for "we could not look".
    pub gpu_ns: Option<u64>,
}

impl RenderProcSample {
    pub fn core_fraction(&self) -> f64 {
        core_fraction(self.cpu_ms, self.interval_ms)
    }

    /// GPU engine milliseconds, the unit the tables and payloads report.
    pub fn gpu_ms(&self) -> Option<f64> {
        self.gpu_ns.map(|nanos| nanos as f64 / 1_000_000.0)
    }
}

/// A process observed in the tree, before deltas are applied. Exposed so the sampler
/// can be unit-tested against a synthetic tree instead of a live `/proc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderProcObservation {
    pub stat: ProcStat,
    pub memory: Option<ProcMemory>,
    /// CUMULATIVE GPU engine nanoseconds, as `/proc` reports them. The delta is taken
    /// by [`RenderProbe::observe`], the same place the CPU delta is taken.
    pub gpu_ns: Option<u64>,
}

/// Holds the previous observation so every reported number is a delta.
///
/// The first `sample()` after construction reports nothing: with no prior
/// observation the only available number would be the lifetime average, which is
/// precisely the lie this module exists to avoid.
#[derive(Debug, Default)]
pub struct RenderProbe {
    last_ticks: BTreeMap<i32, u64>,
    last_gpu_ns: BTreeMap<i32, u64>,
    last_at_ms: Option<u64>,
    user_hz: Option<u64>,
    /// The probe's OWN clock, lazily started on first use so `Default` stays trivial.
    started: Option<std::time::Instant>,
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
    /// **The probe owns its clock, and it is `Instant` — monotonic.** The caller does
    /// not get to supply one, because a caller that supplied a WALL clock (as the
    /// GUI's sampling loop did) breaks the denominator in a way nothing reports: an
    /// NTP step or a suspend/resume inflates `interval_ms` by the gap while the CPU
    /// tick counters do not advance, so `core_fraction` reads artificially LOW right
    /// after the laptop wakes — on the one host whose fan is the complaint. Two
    /// answers to "how much time passed" is one too many.
    ///
    /// Processes that vanished are forgotten; processes that appeared are recorded and
    /// reported only from their *second* observation onward.
    pub fn observe(&mut self, observations: &[RenderProcObservation]) -> Vec<RenderProcSample> {
        let now_ms = self
            .started
            .get_or_insert_with(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64;
        self.observe_at(observations, now_ms)
    }

    /// The delta arithmetic, with the clock injected. **Deliberately private**: it is
    /// the testable core, and keeping it in-module is what makes it impossible for an
    /// outside caller to hand this probe a clock that can jump.
    fn observe_at(
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
        let mut next_gpu_ns = BTreeMap::new();
        for observation in observations {
            let pid = observation.stat.pid;
            let ticks = observation.stat.cpu_ticks();
            next_ticks.insert(pid, ticks);
            if let Some(gpu_ns) = observation.gpu_ns {
                next_gpu_ns.insert(pid, gpu_ns);
            }
            let Some(previous) = self.last_ticks.get(&pid).copied() else {
                continue;
            };
            if interval_ms <= 0.0 {
                continue;
            }
            // A tick counter that went BACKWARDS means pid reuse, not negative work.
            let delta = ticks.saturating_sub(previous);
            // Same rule for the GPU counter, plus one more: a delta needs BOTH ends.
            // An fd closed mid-interval drops the total, and a pid whose fdinfo became
            // unreadable has no reading at all — both must read as "no number", never
            // as a zero that would look like an idle GPU.
            let gpu_ns = match (self.last_gpu_ns.get(&pid).copied(), observation.gpu_ns) {
                (Some(previous), Some(current)) => Some(current.saturating_sub(previous)),
                _ => None,
            };
            samples.push(RenderProcSample {
                pid,
                ppid: observation.stat.ppid,
                role: RenderRole::classify(&observation.stat.comm),
                comm: observation.stat.comm.clone(),
                cpu_ms: cpu_ms_from_ticks(delta, hz),
                interval_ms,
                memory: observation.memory,
                gpu_ns,
            });
        }
        self.last_ticks = next_ticks;
        self.last_gpu_ns = next_gpu_ns;
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
            memory: read_process_memory(pid),
            gpu_ns: drm_engine_ns_for_pid(pid),
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
    /// GPU engine nanoseconds summed across the role's processes, or `None` when NO
    /// process in the role had a readable counter. The distinction survives the rollup
    /// on purpose: "this role did no GPU work" and "we could not see the GPU" are
    /// different findings, and only one of them means the GPU is switched off.
    pub gpu_ns: Option<u64>,
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

    /// GPU engine milliseconds, the unit the tables and payloads report.
    pub fn gpu_ms(&self) -> Option<f64> {
        self.gpu_ns.map(|nanos| nanos as f64 / 1_000_000.0)
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
                gpu_ns: None,
                procs: 0,
                interval_ms: sample.interval_ms,
                hot_pid: sample.pid,
                hot_cpu_ms: f64::MIN,
            });
        entry.cpu_ms += sample.cpu_ms;
        entry.mem_kb += sample
            .memory
            .map(|memory| memory.preferred_kb())
            .unwrap_or(0);
        if let Some(gpu_ns) = sample.gpu_ns {
            entry.gpu_ns = Some(entry.gpu_ns.unwrap_or(0).saturating_add(gpu_ns));
        }
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
        // Absent, not zero, when the counter was unreadable — see RenderRoleRollup.
        if let Some(gpu_ms) = rollup.gpu_ms() {
            payload["gpu_ms"] = json!(gpu_ms);
        }
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
        if let Some(memory) = sample.memory {
            payload["rss_kb"] = json!(memory.rss_kb);
            payload["pss_kb"] = json!(memory.pss_kb);
            payload["anonymous_kb"] = json!(memory.anonymous_kb);
        }
        if let Some(gpu_ms) = sample.gpu_ms() {
            payload["gpu_ms"] = json!(gpu_ms);
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

    /// Moved here from the GUI shell, which carried its own copy of this parser for
    /// the allocator-trim chore while this module parsed the same file per pid.
    #[test]
    fn parses_rss_pss_and_anonymous_in_one_pass() {
        let memory = parse_smaps_rollup(
            "Rss:                123456 kB\nPss:                 98765 kB\nAnonymous:           54321 kB\n",
        )
        .expect("a rollup with an Rss line parses");
        assert_eq!(memory.rss_kb, 123456);
        assert_eq!(memory.pss_kb, 98765);
        assert_eq!(memory.anonymous_kb, 54321);
        assert_eq!(memory.source, ProcMemorySource::SmapsRollup);
        assert_eq!(memory.preferred_kb(), 98765, "PSS wins where we have it");
        // No Rss line at all is a truncated rollup (a process on its way out), which
        // the allocator-trim chore has always refused to act on.
        assert_eq!(parse_smaps_rollup("Pss: 4 kB\n"), None);
        assert_eq!(parse_smaps_rollup(""), None);
    }

    /// The fallback must be LABELLED, never silently passed off as a PSS reading: a
    /// caller that could not tell the difference would read its zeroes as real.
    #[test]
    fn the_status_fallback_is_labelled_and_prefers_rss() {
        let status = "Name:\tyggterm\nVmPeak:\t 900 kB\nVmRSS:\t  543284 kB\nThreads:\t42\n";
        assert_eq!(parse_status_rss_kb(status), Some(543284));
        assert_eq!(parse_status_rss_kb("Name:\tx\n"), None);
        let fallback = ProcMemory {
            rss_kb: 543_284,
            pss_kb: 0,
            anonymous_kb: 0,
            source: ProcMemorySource::StatusVmRss,
        };
        assert_eq!(fallback.preferred_kb(), 543_284);
    }

    /// One owner, wired to the right pid. This fails outright if the collapse
    /// mis-builds the `/proc` path — the sort of thing a pure parser test cannot see.
    #[cfg(target_os = "linux")]
    #[test]
    fn reads_this_processs_own_memory_from_proc() {
        let memory = read_process_memory(std::process::id() as i32)
            .expect("this process must be able to read its own memory");
        assert!(memory.rss_kb > 0);
        assert_eq!(memory.source, ProcMemorySource::SmapsRollup);
        // A pid that cannot exist has no memory, and that is None rather than zeroes.
        assert_eq!(read_process_memory(-1), None);
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
            memory: Some(ProcMemory {
                rss_kb: 1000,
                pss_kb: 600,
                anonymous_kb: 400,
                source: ProcMemorySource::SmapsRollup,
            }),
            gpu_ns: None,
        }
    }

    fn observation_with_gpu(
        pid: i32,
        comm: &str,
        ppid: i32,
        ticks: u64,
        gpu_ns: u64,
    ) -> RenderProcObservation {
        RenderProcObservation {
            gpu_ns: Some(gpu_ns),
            ..observation(pid, comm, ppid, ticks)
        }
    }

    /// The verbatim live fixture from the GUI host (krunner, `/proc/<pid>/fdinfo/15`,
    /// 2026-07-25) — the frame of reference for "the GPU works for everything except
    /// us". Both engines sum; the `drm-memory-*` block must not leak into the time.
    const KRUNNER_FDINFO: &str = "pos:\t0\nflags:\t02100002\nmnt_id:\t26\nino:\t1128\n\
drm-driver:\tamdgpu\ndrm-client-id:\t141\ndrm-memory-vram:\t8192 KiB\n\
drm-memory-gtt:\t3564 KiB\ndrm-engine-gfx:\t56369242 ns\n\
drm-engine-compute:\t3434919 ns\n";

    #[test]
    fn sums_every_drm_engine_and_ignores_the_memory_block() {
        assert_eq!(
            parse_fdinfo_drm_engine_ns(KRUNNER_FDINFO),
            Some(56_369_242 + 3_434_919)
        );
        // An fd that is not a GPU fd names no engine at all.
        assert_eq!(parse_fdinfo_drm_engine_ns("pos:\t0\nflags:\t02\n"), None);
        // ...including one that carries only the memory gauges.
        assert_eq!(
            parse_fdinfo_drm_engine_ns("drm-driver:\tamdgpu\ndrm-memory-vram:\t8192 KiB\n"),
            None
        );
        // A counter in some other unit is a key we do not understand, not nanoseconds.
        assert_eq!(
            parse_fdinfo_drm_engine_ns("drm-engine-gfx:\t500 us\n"),
            None
        );
    }

    /// The GPU gauge must be a DELTA like everything else here. A lifetime total would
    /// make a long-lived process look busy forever — the same `ps %CPU` lie this
    /// module exists to avoid, wearing a different hat.
    #[test]
    fn gpu_time_is_a_delta_and_a_missing_reading_is_not_a_zero() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe_at(
            &[observation_with_gpu(10, "WebKitWebProces", 1, 0, 1_000_000)],
            1_000,
        );
        let samples = probe.observe_at(
            &[observation_with_gpu(
                10,
                "WebKitWebProces",
                1,
                10,
                4_000_000,
            )],
            2_000,
        );
        assert_eq!(samples[0].gpu_ns, Some(3_000_000));
        assert_eq!(samples[0].gpu_ms(), Some(3.0));
        // A pid whose fdinfo became unreadable reports NO number, never zero: "we
        // could not look" and "the GPU was idle" are different findings.
        let samples = probe.observe_at(&[observation(10, "WebKitWebProces", 1, 20)], 3_000);
        assert_eq!(samples[0].gpu_ns, None);
        assert_eq!(samples[0].gpu_ms(), None);
        // ...and a counter that went backwards (an fd closed mid-interval) reads as
        // zero work rather than underflowing into an astronomical delta.
        probe.observe_at(
            &[observation_with_gpu(
                10,
                "WebKitWebProces",
                1,
                30,
                9_000_000,
            )],
            4_000,
        );
        let samples = probe.observe_at(
            &[observation_with_gpu(10, "WebKitWebProces", 1, 40, 12)],
            5_000,
        );
        assert_eq!(samples[0].gpu_ns, Some(0));
    }

    /// The rollup keeps the same distinction: a role where nobody could be read has no
    /// number, and the payload therefore carries no `gpu_ms` key at all.
    #[test]
    fn role_rollup_sums_gpu_time_and_keeps_unreadable_distinct_from_idle() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe_at(
            &[
                observation_with_gpu(11, "WebKitWebProces", 10, 0, 0),
                observation_with_gpu(12, "WebKitWebProces", 10, 0, 500_000),
                observation(13, "yggterm", 1, 0),
            ],
            1_000,
        );
        let samples = probe.observe_at(
            &[
                observation_with_gpu(11, "WebKitWebProces", 10, 5, 2_000_000),
                observation_with_gpu(12, "WebKitWebProces", 10, 5, 1_500_000),
                observation(13, "yggterm", 1, 5),
            ],
            2_000,
        );
        let rolled = roll_up_roles(&samples);
        let web = rolled
            .iter()
            .find(|rollup| rollup.role == RenderRole::WebContent)
            .unwrap();
        assert_eq!(web.gpu_ns, Some(2_000_000 + 1_000_000));
        assert_eq!(web.gpu_ms(), Some(3.0));
        let gui = rolled
            .iter()
            .find(|rollup| rollup.role == RenderRole::Gui)
            .unwrap();
        assert_eq!(gui.gpu_ns, None, "unreadable must not roll up as zero");
    }

    /// THE anti-regression test for this whole module: the first observation must
    /// report nothing. If it ever reports, it is reporting a lifetime average, which
    /// is the `ps %CPU` trap that made a pegged GUI look idle.
    #[test]
    fn first_observation_reports_nothing() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        let samples = probe.observe_at(&[observation(10, "yggterm", 1, 100_000)], 1_000);
        assert!(
            samples.is_empty(),
            "first sample must not report a lifetime average"
        );
    }

    #[test]
    fn second_observation_reports_the_delta_only() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe_at(&[observation(10, "yggterm", 1, 100_000)], 1_000);
        // 50 ticks (500 CPU ms) burned over a 1000 ms interval = half a core.
        let samples = probe.observe_at(&[observation(10, "yggterm", 1, 100_050)], 2_000);
        assert_eq!(samples.len(), 1);
        let sample = &samples[0];
        assert_eq!(sample.cpu_ms, 500.0);
        assert_eq!(sample.interval_ms, 1000.0);
        assert_eq!(sample.core_fraction(), 0.5);
        assert_eq!(sample.role, RenderRole::Gui);
    }

    /// The probe times ITSELF, off a monotonic `Instant`. The GUI's sampling loop used
    /// to hand it `current_millis()` — a `SystemTime` wall clock — so an NTP step or a
    /// suspend/resume inflated `interval_ms` by the gap while the CPU tick counters
    /// stood still, and `core_fraction` read artificially low right after every wake.
    /// The caller cannot make that mistake any more, because it no longer passes a
    /// clock at all.
    #[test]
    fn the_probe_times_itself_over_a_monotonic_interval() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe(&[observation(10, "yggterm", 1, 0)]);
        std::thread::sleep(std::time::Duration::from_millis(30));
        let samples = probe.observe(&[observation(10, "yggterm", 1, 3)]);
        assert_eq!(samples.len(), 1);
        assert!(
            samples[0].interval_ms >= 20.0 && samples[0].interval_ms <= 5_000.0,
            "interval must be the real elapsed time, got {}",
            samples[0].interval_ms
        );
    }

    /// A clock that steps BACKWARDS must yield no sample rather than a nonsense rate.
    /// Pinning the saturating degradation as intended behaviour: a signed subtraction
    /// here would turn one bad clock reading into a negative interval and an infinite
    /// core fraction.
    #[test]
    fn a_clock_that_steps_backwards_reports_no_sample() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe_at(&[observation(10, "yggterm", 1, 0)], 5_000);
        let samples = probe.observe_at(&[observation(10, "yggterm", 1, 50)], 1_000);
        assert!(samples.is_empty());
    }

    /// PID reuse resets the counter downward. That must read as zero work, never as a
    /// huge negative that underflows into an astronomical delta.
    #[test]
    fn tick_counter_going_backwards_reads_as_zero() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe_at(&[observation(10, "yggterm", 1, 5_000)], 1_000);
        let samples = probe.observe_at(&[observation(10, "yggterm", 1, 12)], 2_000);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].cpu_ms, 0.0);
    }

    #[test]
    fn vanished_process_is_forgotten_and_new_one_waits_a_turn() {
        let mut probe = RenderProbe::new().with_user_hz(100);
        probe.observe_at(&[observation(10, "yggterm", 1, 100)], 1_000);
        // pid 10 gone, pid 11 new: nothing reportable this turn.
        let samples = probe.observe_at(&[observation(11, "WebKitWebProces", 10, 900)], 2_000);
        assert!(samples.is_empty());
        // Now pid 11 has a baseline and reports its own delta.
        let samples = probe.observe_at(&[observation(11, "WebKitWebProces", 10, 910)], 3_000);
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
        probe.observe_at(&first, 1_000);
        let second = [
            observation(10, "yggterm", 1, 70),
            observation(11, "WebKitWebProces", 10, 30),
            observation(12, "WebKitWebProces", 10, 5),
        ];
        let samples = probe.observe_at(&second, 2_000);
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
            gpu_ns: None,
            procs: 1,
            interval_ms: 60_000.0,
            hot_pid: 99,
            hot_cpu_ms: 0.0,
        };
        let busy = RenderRoleRollup {
            role: RenderRole::WebContent,
            cpu_ms: 250.0,
            mem_kb: 4096,
            gpu_ns: Some(7_000_000),
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
        // The GPU gauge rides in the payload: with hardware GL this is nonzero and
        // with the software rasterizer it is not, which is the ONE field that
        // distinguishes "cheap because the GPU did it" from "cheap because nothing
        // happened".
        assert!(log.contains("\"gpu_ms\":7.0"));
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
        probe.observe_at(&[observation(10, "WebKitWebProces", 1, 0)], 1_000);
        let samples = probe.observe_at(&[observation(10, "WebKitWebProces", 1, 25)], 2_000);
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
