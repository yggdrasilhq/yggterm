//! Host panic heartbeat — keep the client machine cool and quiet, or say why not.
//!
//! The complaint this answers: *the laptop goes angry even when almost
//! everything runs elsewhere.* That is an attribution question and it was
//! previously unanswerable, because nothing sampled the client host as a whole.
//! The row governor watches individual rows; the render probe watches our
//! process tree; neither of them ever asks "is this machine, overall, in
//! trouble, and are we the reason."
//!
//! This watcher does, once a minute, and files at most one incident naming the
//! single worst thing — the verdict itself is pure and lives in
//! `ytrace::diagnosis`, so Dash, the notebooks, the daemon and an LLM reading
//! `ytrace incidents` all agree byte for byte.
//!
//! ⛔ **An unreadable sensor stays unreadable.** Every sampled field is an
//! `Option` all the way through to the verdict. A thermometer that cannot be
//! read is not a cool machine, and a zero substituted for a missing measurement
//! is how a blind instrument becomes an all-clear.
//!
//! ⛔ **Detect, notify, do not actuate.** Throttling scans or parking rows in
//! response to heat is a policy decision with the owner's name on it, and a
//! detector that quietly starts changing behaviour is much harder to trust than
//! one that only ever reports. The actuator proposal is filed in
//! `docs/owner-attention.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

use yggterm_core::render_probe::{
    RenderRole, cpu_ms_from_ticks, parse_proc_stat, user_hz,
};

/// Sampling interval. Slower than the row governor on purpose: this is a
/// whole-machine verdict, and a whole-machine verdict that changes every few
/// seconds is noise.
pub const HOST_PANIC_INTERVAL_MS: u64 = 60_000;
/// Minimum gap between two notifications, however long the condition lasts. A
/// heartbeat that repeats every minute while a build runs is an alarm nobody
/// reads by the third one.
pub const NOTIFY_COOLDOWN_MS: u64 = 15 * 60_000;

fn is_enabled() -> bool {
    std::env::var("YGGTERM_HOST_PANIC")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(true)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

// ── sampling ────────────────────────────────────────────────────────────────

/// Hottest package-level sensor in degrees C, or `None` if nothing is readable.
///
/// `k10temp` and `coretemp` are the CPU package; `acpitz` is the board's own
/// idea of the same thing and is the only reading on some machines. Anything
/// else (drive, wifi, GPU) is deliberately excluded — a warm NVMe is not the
/// symptom being chased and would raise the floor for every host.
pub fn package_temp_c() -> Option<f64> {
    let mut hottest: Option<f64> = None;
    let entries = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
            continue;
        };
        if !matches!(name.trim(), "k10temp" | "coretemp" | "acpitz") {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let Some(stem) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !(stem.starts_with("temp") && stem.ends_with("_input")) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(milli) = text.trim().parse::<f64>() {
                    let c = milli / 1000.0;
                    // Guard against the sensor that reports an impossible value
                    // rather than failing to read — 0 C and 200 C are both a
                    // broken thermometer, not a machine state.
                    if (10.0..=125.0).contains(&c) {
                        hottest = Some(hottest.map_or(c, |h: f64| h.max(c)));
                    }
                }
            }
        }
    }
    hottest
}

/// Socket power in watts, and the fan reading if this machine has one.
///
/// ⛔ **Measured on the reference client, there is no fan tachometer.** The ACPI
/// fan (`PNP0C0B`) exposes `fan1_input` and it is a stub pinned at 0; the two
/// `type=Fan` thermal cooling devices are pinned at `cur_state=1` of `max_state=1`
/// and never move. Both look like fan telemetry and neither carries any
/// information, which is worse than having none — a series of zeroes graphs
/// beautifully and means nothing.
///
/// What IS live on that hardware is **socket power** (`amdgpu power1_average`,
/// the APU's package draw, covering CPU and GPU) alongside package temperature.
/// Sustained power is what a laptop's fan curve actually responds to, so power
/// plus temperature is the honest proxy — and it is reported as a proxy, never
/// relabelled as a fan speed.
pub fn power_and_fan() -> (Option<f64>, Option<u64>) {
    let mut watts = None;
    let mut rpm = None;
    let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
        return (None, None);
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
            continue;
        };
        match name.trim() {
            "amdgpu" | "intel-rapl" => {
                if let Ok(text) = std::fs::read_to_string(dir.join("power1_average")) {
                    if let Ok(micro) = text.trim().parse::<f64>() {
                        watts = Some(micro / 1_000_000.0);
                    }
                }
            }
            _ => {}
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("fan1_input")) {
            if let Ok(value) = text.trim().parse::<u64>() {
                // A stub reports 0 forever. Absent beats a confident zero: the
                // whole point of this field is to say whether the fan is on, and
                // "0 rpm" from a device that cannot count is not that answer.
                if value > 0 {
                    rpm = Some(value);
                }
            }
        }
    }
    (watts, rpm)
}

/// `(used_fraction, swap_used_gib)` from `/proc/meminfo`.
pub fn memory_pressure() -> (Option<f64>, Option<f64>) {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return (None, None);
    };
    let mut fields: HashMap<&str, u64> = HashMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if let Some(kb) = rest.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()) {
            fields.insert(key, kb);
        }
    }
    // MemAvailable, not MemFree: free memory on a healthy Linux box is near zero
    // by design, and a fraction computed from it reports permanent panic.
    let used = match (fields.get("MemTotal"), fields.get("MemAvailable")) {
        (Some(&total), Some(&avail)) if total > 0 => {
            Some((total.saturating_sub(avail)) as f64 / total as f64)
        }
        _ => None,
    };
    let swap = match (fields.get("SwapTotal"), fields.get("SwapFree")) {
        (Some(&total), Some(&free)) => {
            Some(total.saturating_sub(free) as f64 / (1024.0 * 1024.0))
        }
        _ => None,
    };
    (used, swap)
}

/// Bytes held under `$XDG_RUNTIME_DIR`.
///
/// That directory is a tmpfs, so this is resident memory, not disk. It is
/// sampled specifically because the one unbounded writer found in this system
/// lived there and nothing was looking.
pub fn runtime_tmpfs_bytes() -> Option<u64> {
    let dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    fn walk(path: &Path, depth: u32) -> u64 {
        if depth > 6 {
            return 0;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        let mut total = 0;
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() {
                total += meta.len();
            } else if meta.is_dir() {
                total += walk(&entry.path(), depth + 1);
            }
        }
        total
    }
    Some(walk(Path::new(&dir), 0))
}

/// Cores burned by OUR process tree — the GUI and its webview helpers.
///
/// This is the number that decides attribution. It is a sampled delta, never
/// `ps %CPU`, which is a lifetime average and reports a process that pinned a
/// core an hour ago as busy forever.
#[derive(Default)]
pub struct OurCpu {
    last: HashMap<i32, u64>,
    last_at: Option<Instant>,
    user_hz: u64,
}

impl OurCpu {
    pub fn new() -> Self {
        Self { user_hz: user_hz(), ..Default::default() }
    }

    pub fn sample(&mut self) -> Option<f64> {
        let now = Instant::now();
        let mut current: HashMap<i32, u64> = HashMap::new();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<i32>().ok())
            else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some(stat) = parse_proc_stat(&text) else { continue };
            if matches!(
                RenderRole::classify(&stat.comm),
                RenderRole::Gui | RenderRole::WebContent | RenderRole::WebNetwork | RenderRole::WebGpu
            ) {
                current.insert(pid, stat.cpu_ticks());
            }
        }
        let elapsed_ms = self.last_at.map(|t| now.duration_since(t).as_millis() as f64);
        let mut cores = None;
        if let Some(ms) = elapsed_ms {
            if ms > 0.0 {
                // Only pids present in BOTH samples contribute. A process that
                // appeared mid-interval has no baseline, and crediting its whole
                // lifetime counter to this interval invents load out of nothing.
                let delta: u64 = current
                    .iter()
                    .filter_map(|(pid, ticks)| {
                        self.last.get(pid).map(|prev| ticks.saturating_sub(*prev))
                    })
                    .sum();
                cores = Some(cpu_ms_from_ticks(delta, self.user_hz) / ms);
            }
        }
        self.last = current;
        self.last_at = Some(now);
        cores
    }
}

/// Blocks per minute reported by the GUI's ui/block watchdog, read from the bus.
///
/// ⛔⛔ THE THIRD ARGUMENT IS AN ABSOLUTE EPOCH CUTOFF, NOT A DURATION, AND THIS
/// PASSED A DURATION FOR MONTHS. `ytrace::query::summarize`'s parameter is
/// `since_ms` and its reader filters with `record.ts_ms < since`. Handing it a
/// window length (300_000) asks for "everything after 1970-01-01 00:05", which
/// every record satisfies — so NOTHING was filtered and this returned the
/// LIFETIME count of the retained trace divided by five minutes.
///
/// Both values are `u128`, both are honestly "milliseconds", and the call site
/// read correctly at a glance. Nothing in the type system or the name could
/// catch it; only the values could, and they were never checked against a
/// second source.
///
/// ⇒ Measured on the desktop host 2026-08-21, while the owner was reporting
///   input latency: this field read `1121.6`, then `97.0`, each REPEATED
///   IDENTICALLY across consecutive 60 s heartbeats, against a 6/min alarm
///   threshold. `ytrace health` gave a lifetime count of 487 for the same
///   probe, and 487 / 5 = 97.4. Three tells, any one of which is conclusive:
///   the value only ever rises, it is constant between adjacent samples (which
///   a rate cannot be), and it sat permanently above its own threshold — so the
///   panic incident fired every minute forever, ~200 error-severity incidents
///   of pure noise. **An alarm that is always on carries the same information
///   as one that is never on**, and the instrument built to make input blocks
///   visible was incapable of reporting them while appearing to report them
///   continuously.
///
/// ⚠ A rate that does not move between samples is not a rate. That check costs
/// nothing and is the one that would have caught this on day one.
fn ui_block_density(home: &Path, window_ms: u128) -> Option<f64> {
    let since = now_ms().saturating_sub(window_ms);
    let summaries = ytrace::query::summarize(home, Some("ui"), Some(since));
    let blocks = summaries.iter().find(|s| s.name == "block")?;
    Some(blocks.count as f64 / (window_ms as f64 / 60_000.0))
}

// ── the watcher ─────────────────────────────────────────────────────────────

pub struct HostPanicWatcher {
    home: PathBuf,
    ytrace_home: PathBuf,
    host: String,
    cpu: OurCpu,
    provider: ytrace::Provider,
    /// When the current above-threshold condition started.
    elevated_since: Option<Instant>,
    last_notify_ms: u128,
}

impl HostPanicWatcher {
    pub fn new(home: PathBuf, host: String) -> Self {
        let ytrace_home = ytrace::compat::resolve_home("yggterm");
        let provider = ytrace::Provider::with_home(
            "yggterm",
            yggterm_core::current_version(),
            ytrace_home.clone(),
        );
        provider.register("heartbeat/panic", ytrace::Clock::Wall, ytrace::Sample::always());
        Self {
            home,
            ytrace_home,
            host,
            cpu: OurCpu::new(),
            provider,
            elevated_since: None,
            last_notify_ms: 0,
        }
    }

    /// One tick. Returns the incident filed, if any.
    pub fn tick(&mut self) -> Option<ytrace::diagnosis::Incident> {
        if !is_enabled() {
            return None;
        }
        let (mem_used_fraction, swap_used_gib) = memory_pressure();
        let (socket_watts, fan_rpm) = power_and_fan();
        let mut sample = ytrace::diagnosis::HostPanicSample {
            host: self.host.clone(),
            package_temp_c: package_temp_c(),
            mem_used_fraction,
            swap_used_gib,
            our_cores: self.cpu.sample(),
            runtime_tmpfs_bytes: runtime_tmpfs_bytes(),
            ui_blocks_per_min: ui_block_density(&self.ytrace_home, 5 * 60_000),
            sustained_secs: 0,
            subject_row: None,
        };

        // A condition must HOLD to be a panic. Track how long anything has been
        // elevated by asking the pure verdict with the sustain requirement
        // already met — if it would fire, the clock is running.
        let probe = ytrace::diagnosis::HostPanicSample {
            sustained_secs: ytrace::diagnosis::HOST_PANIC_SUSTAINED_SECS,
            ..sample.clone()
        };
        if ytrace::diagnosis::diagnose_host_panic(&probe).is_some() {
            let since = *self.elevated_since.get_or_insert_with(Instant::now);
            sample.sustained_secs = since.elapsed().as_secs();
        } else {
            self.elevated_since = None;
            return None;
        }

        let incident = ytrace::diagnosis::diagnose_host_panic(&sample)?;
        let payload = ytrace::diagnosis::incident_payload(&incident);
        self.provider
            .incident("daemon", "heartbeat", "panic", payload.clone());
        yggterm_core::append_trace_event(
            &self.home,
            "daemon",
            "heartbeat",
            "panic",
            json!({
                "host": self.host,
                "incident_id": incident.id,
                "severity": incident.severity.as_str(),
                "diagnosis": incident.diagnosis,
                // Proxies for "is the fan about to spin", carried alongside the
                // verdict so a reader never has to go and re-sample the host.
                "socket_watts": socket_watts,
                "fan_rpm": fan_rpm,
            }),
        );
        Some(incident)
    }

    /// Whether a notification is due, given the cooldown.
    pub fn should_notify(&mut self, severity: ytrace::diagnosis::Severity) -> bool {
        if severity != ytrace::diagnosis::Severity::Error {
            return false; // a warn is for the log and the notebooks, not a toast
        }
        let now = now_ms();
        if now.saturating_sub(self.last_notify_ms) < NOTIFY_COOLDOWN_MS as u128 {
            return false;
        }
        self.last_notify_ms = now;
        true
    }
}

pub fn spawn_host_panic_watcher(home: PathBuf, host: String, notify: Arc<dyn Fn(&ytrace::diagnosis::Incident) + Send + Sync>) {
    std::thread::Builder::new()
        .name("yggterm-host-panic".to_string())
        .spawn(move || {
            let watcher = Mutex::new(HostPanicWatcher::new(home, host));
            loop {
                std::thread::sleep(Duration::from_millis(HOST_PANIC_INTERVAL_MS));
                let mut guard = match watcher.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                if let Some(incident) = guard.tick() {
                    if guard.should_notify(incident.severity) {
                        drop(guard);
                        notify(&incident);
                    }
                }
            }
        })
        .ok();
}


/// This machine's name, as the owner would recognise it.
pub fn local_host_label() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "this host".to_string())
}

/// Reach the owner through the documented door.
///
/// This shells out to `server app notify` rather than constructing an
/// app-control request in-process, deliberately: the CLI is the ONE owner of
/// what a notification means — tone vocabulary, job upsert, `--session` making
/// the card clickable through to a row. A second in-process encoding of that
/// would be free to drift from it, and this fires at most once every fifteen
/// minutes, so the cost of a process spawn is not worth a second copy.
///
/// ⭐ `--session` is what makes this an ADDRESS rather than an announcement: the
/// card lands pointing at the row that caused the trouble, so the next action is
/// one click away instead of a search.
/// Returns the pid of the notification process, so a caller — or a lock — can
/// name the thing that was launched instead of counting the population it
/// landed in. `None` means no process was started.
pub fn notify_owner(home: &Path, incident: &ytrace::diagnosis::Incident) -> Option<u32> {
    let binary = installed_binary(home)?;
    let mut cmd = std::process::Command::new(binary);
    cmd.args(["server", "app", "notify"])
        .arg(format!("Client host: {}", short_reason(&incident.id)))
        .arg(&incident.diagnosis)
        .args(["--tone", "warning"])
        // One job key, so a condition that lasts an hour updates a single card
        // instead of stacking fifteen identical toasts.
        .args(["--job", "host-panic"]);
    if let Some(row) = incident.subject.as_deref() {
        cmd.args(["--session", row]);
    }
    // ⛔ REAPED, NOT JUST LAUNCHED. This was `let _ = cmd.spawn()`, and a
    // dropped `Child` is never waited on — so every fifteen-minute notification
    // left a permanent zombie in the daemon's process table. Measured
    // 2026-08-21: 79 of them under a daemon that had been up 19.9 hours, the
    // oldest exactly as old as the daemon. See `yggterm_platform::child_reaper`.
    yggterm_platform::child_reaper::spawn_and_reap(
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
    )
    .ok()
}

fn short_reason(incident_id: &str) -> &str {
    match incident_id {
        "host_panic_tmpfs" => "memory held in tmpfs",
        "host_panic_memory" => "memory pressure",
        "host_panic_our_cpu" => "our GUI is burning CPU",
        "host_panic_ui_thrash" => "the UI is thrashing",
        "host_panic_thermal" => "running hot",
        _ => "under load",
    }
}

fn installed_binary(home: &Path) -> Option<PathBuf> {
    let candidate = home.join("bin").join("yggterm-headless");
    if candidate.exists() {
        return Some(candidate);
    }
    // Fall back to PATH resolution rather than guessing another absolute path:
    // a wrong absolute path fails silently and a missing notification is
    // indistinguishable from a quiet machine.
    Some(PathBuf::from("yggterm-headless"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn memory_pressure_uses_available_not_free() {
        // MemFree on a healthy Linux box is near zero by design, so a fraction
        // computed from it would report permanent panic. This guards the choice.
        let (used, swap) = memory_pressure();
        if let Some(fraction) = used {
            assert!(
                (0.0..=1.0).contains(&fraction),
                "used fraction out of range: {fraction}"
            );
            assert!(
                fraction < 0.999,
                "a fraction this high on a working host means MemFree crept back in"
            );
        }
        if let Some(gib) = swap {
            assert!(gib >= 0.0);
        }
    }

    /// ⛔ THE LOCK ON THE REAL CALL SITE, NOT JUST THE PRIMITIVE.
    ///
    /// `notify_owner` used to end in `let _ = cmd.spawn()`, and a dropped
    /// `Child` is never waited on — so every fifteen-minute owner notification
    /// left a zombie in the daemon's table for the daemon's whole life
    /// (measured 2026-08-21: 79 of them under one 19.9-hour daemon). The
    /// primitive is unit-tested in `yggterm_platform::child_reaper`; this asserts
    /// that THIS function actually routes through it.
    ///
    /// ⚠ The fake home is load-bearing for SAFETY, not just isolation:
    /// `installed_binary` falls back to `yggterm-headless` on PATH, and on a
    /// developer host that would fire a REAL notification at the owner. Placing
    /// an executable at `<home>/bin/yggterm-headless` takes the first branch.
    #[cfg(target_os = "linux")]
    #[test]
    fn notify_owner_reaps_the_notification_child() {
        /// The state of one pid as a child of THIS process: `Some('Z')` for a
        /// zombie we are still the parent of, `Some(other)` while it lives,
        /// `None` once it is gone or was never ours.
        fn state_of_our_child(pid: u32) -> Option<char> {
            let me = std::process::id();
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // `pid (comm) state ppid ...`, and comm may contain spaces and
            // parentheses, so split after the LAST ')'.
            let (_, rest) = stat.rsplit_once(')')?;
            let mut fields = rest.split_whitespace();
            let state = fields.next()?.chars().next()?;
            let ppid: u32 = fields.next()?.parse().ok()?;
            (ppid == me).then_some(state)
        }

        let home = std::env::temp_dir().join(format!("ygg-reaplock-{}", std::process::id()));
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).expect("create the bin dir");
        let stub = bin.join("yggterm-headless");
        std::fs::write(&stub, "#!/bin/sh\nexit 0\n").expect("write the stub");
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("make the stub executable");

        let incident = ytrace::diagnosis::Incident {
            id: "host_panic_memory".to_string(),
            kind: ytrace::diagnosis::IncidentKind::Resource,
            severity: ytrace::diagnosis::Severity::Warn,
            diagnosis: "a synthetic incident for the reaping lock".to_string(),
            remedy: "none — this incident is not real".to_string(),
            observed: serde_json::json!(0),
            threshold: serde_json::json!(0),
            subject: None,
            suggested_queries: Vec::new(),
        };
        let pid = notify_owner(&home, &incident);
        let _ = std::fs::remove_dir_all(&home);

        // ⛔ CONTROL FIRST. Without this the whole test passes when nothing was
        // launched at all, which is the shape a reaping lock is most likely to
        // rot into.
        let pid = pid.expect("notify_owner must report the pid it launched");

        // ⛔ ASK ABOUT THIS PID, NEVER COUNT THE POPULATION. The first version
        // of this lock counted zombie children of the whole test process
        // against a baseline snapshot — which passes alone and FAILS in the
        // suite, because 1,200 sibling tests spawn and reap children of the
        // same process throughout the window. A lock that goes red on its
        // neighbours' work teaches people to re-run it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            match state_of_our_child(pid) {
                Some('Z') => std::thread::sleep(std::time::Duration::from_millis(20)),
                // Gone: waited on and cleared, which is the whole claim.
                None => return,
                // Still running; the reaper thread is parked on it.
                Some(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        panic!(
            "notify_owner left pid {pid} as a zombie child for 15s — the Child was \
             dropped instead of waited on",
        );
    }

    #[test]
    fn a_broken_thermometer_is_none_not_zero() {
        // Can't force a bad sensor here, but the contract is checkable: whatever
        // comes back is either absent or physically plausible. A 0.0 would mean
        // an unreadable sensor had been laundered into a cool reading.
        if let Some(c) = package_temp_c() {
            assert!((10.0..=125.0).contains(&c), "implausible temperature: {c}");
        }
    }

    #[test]
    fn our_cpu_reports_nothing_on_the_first_sample() {
        // A delta needs two points. Reporting on the first sample would credit
        // each process's entire lifetime to one interval.
        let mut cpu = OurCpu::new();
        assert!(cpu.sample().is_none(), "the first sample has no baseline");
        let second = cpu.sample();
        if let Some(cores) = second {
            assert!(cores >= 0.0 && cores < 1024.0, "implausible core count: {cores}");
        }
    }

    #[test]
    fn the_notification_names_a_row_when_one_is_known() {
        // A notification is an ADDRESS. An incident that knows which row caused
        // the trouble must carry it through, or the card is an announcement the
        // reader then has to go and act on by hand.
        let s = ytrace::diagnosis::HostPanicSample {
            host: "test-host".to_string(),
            ui_blocks_per_min: Some(ytrace::diagnosis::UI_BLOCK_DENSITY_PER_MIN + 2.0),
            sustained_secs: ytrace::diagnosis::HOST_PANIC_SUSTAINED_SECS,
            subject_row: Some("example-row://host/id".to_string()),
            ..Default::default()
        };
        let inc = ytrace::diagnosis::diagnose_host_panic(&s).expect("thrash");
        assert_eq!(inc.subject.as_deref(), Some("example-row://host/id"));
        assert_eq!(short_reason(&inc.id), "the UI is thrashing");
    }

    #[test]
    fn every_panic_id_has_a_human_reason() {
        for id in [
            "host_panic_tmpfs", "host_panic_memory", "host_panic_our_cpu",
            "host_panic_ui_thrash", "host_panic_thermal",
        ] {
            assert_ne!(short_reason(id), "under load", "unmapped incident id: {id}");
        }
    }

    #[test]
    fn a_stub_fan_reads_as_absent_not_as_zero() {
        // On hardware with no tachometer the ACPI fan reports 0 forever. A
        // series of confident zeroes is worse than no series: it graphs
        // beautifully and answers nothing.
        let (watts, rpm) = power_and_fan();
        if let Some(r) = rpm {
            assert!(r > 0, "a zero rpm must never be reported as a reading");
        }
        if let Some(w) = watts {
            assert!((0.0..=500.0).contains(&w), "implausible socket power: {w} W");
        }
    }

    #[test]
    fn a_warning_never_raises_a_toast() {
        let mut w = HostPanicWatcher::new(std::env::temp_dir(), "test-host".to_string());
        assert!(
            !w.should_notify(ytrace::diagnosis::Severity::Warn),
            "a warn belongs in the log and the notebooks, not on the owner's screen"
        );
        assert!(w.should_notify(ytrace::diagnosis::Severity::Error), "first error notifies");
        assert!(
            !w.should_notify(ytrace::diagnosis::Severity::Error),
            "and the cooldown suppresses the second — an alarm that repeats is one nobody reads"
        );
    }
}
