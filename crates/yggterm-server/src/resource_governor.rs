//! Resource governor — per-row CPU/memory sampling, incident filing, and
//! resource-aware actions (SSH detach, telemetry, ytrace complaints).
//!
//! Lives in the daemon, runs every 15 s. The diagnosis itself is pure and owned
//! by `ytrace::diagnosis` so Dash notebooks, `ytrace query`, and the daemon
//! agree byte-for-byte. This file only samples and acts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use yggterm_core::render_probe::{
    parse_proc_stat, read_process_memory, user_hz, cpu_ms_from_ticks, core_fraction,
};

/// Sampling interval — governor tick.
pub const GOVERNOR_INTERVAL_MS: u64 = 15_000;
/// Reattach delay after an SSH detach.
pub const SSH_REATTACH_DELAY_MS: u64 = 120_000;

fn is_ssh_row(key: &str) -> bool {
    key.starts_with("remote-") || key.starts_with("remote_")
}

fn is_governor_enabled() -> bool {
    std::env::var("YGGTERM_GOVERNOR")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(true)
}

fn is_ssh_detach_enabled() -> bool {
    std::env::var("YGGTERM_GOVERNOR_SSH_DETACH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn ytrace_home_for_app(app: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("YTRACE_HOME") {
        return PathBuf::from(dir).join(app);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("ytrace").join(app);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("ytrace")
            .join(app);
    }
    PathBuf::from("/tmp/ytrace").join(app)
}

fn ytrace_provider() -> ytrace::Provider {
    let home = ytrace_home_for_app("yggterm");
    let p = ytrace::Provider::with_home("yggterm", yggterm_core::current_version(), home);
    p.register("row_resource/hot", ytrace::Clock::Cpu, ytrace::Sample::always());
    p.register("row_resource/oom", ytrace::Clock::Cpu, ytrace::Sample::always());
    p.register("daemon/resource_governor", ytrace::Clock::Wall, ytrace::Sample::always());
    p
}

struct RowTick {
    last_ticks: u64,
    last_at: Instant,
    hot_since: Option<Instant>,
    last_incident_at: Option<Instant>,
    detached_until: Option<Instant>,
}

pub struct ResourceGovernor {
    ticks: HashMap<String, RowTick>,
    user_hz: u64,
    provider: ytrace::Provider,
    yggterm_home: PathBuf,
}

impl ResourceGovernor {
    pub fn new(yggterm_home: PathBuf) -> Self {
        Self {
            ticks: HashMap::new(),
            user_hz: user_hz(),
            provider: ytrace_provider(),
            yggterm_home,
        }
    }

    pub fn tick(
        &mut self,
        rows: &[(String, Option<u32>)],
        park_fn: &dyn Fn(&str) -> bool,
        unpark_fn: &dyn Fn(&str) -> bool,
    ) -> Vec<String> {
        if !is_governor_enabled() {
            return Vec::new();
        }
        let now = Instant::now();
        let mut acted = Vec::new();
        for (key, pid_opt) in rows {
            let Some(pid) = pid_opt else { continue };
            if let Some(tick) = self.ticks.get(key) {
                if let Some(until) = tick.detached_until {
                    if now < until {
                        continue;
                    } else {
                        if is_ssh_detach_enabled() {
                            let _ = unpark_fn(key);
                        }
                        if let Some(t) = self.ticks.get_mut(key) {
                            t.detached_until = None;
                            t.hot_since = None;
                        }
                    }
                }
            }
            let cpu_ticks = read_proc_ticks(*pid as i32);
            let mem_kb = read_process_memory(*pid as i32).map(|m| m.preferred_kb());
            let Some(ticks) = cpu_ticks else { continue };
            let entry = self.ticks.entry(key.clone()).or_insert(RowTick {
                last_ticks: ticks,
                last_at: now,
                hot_since: None,
                last_incident_at: None,
                detached_until: None,
            });
            let elapsed_ms = now.duration_since(entry.last_at).as_millis() as f64;
            if elapsed_ms < 100.0 {
                entry.last_ticks = ticks;
                entry.last_at = now;
                continue;
            }
            let delta = ticks.saturating_sub(entry.last_ticks);
            let cpu_ms = cpu_ms_from_ticks(delta, self.user_hz);
            let core = core_fraction(cpu_ms, elapsed_ms);
            entry.last_ticks = ticks;
            entry.last_at = now;

            let is_ssh = is_ssh_row(key);
            let hot = if is_ssh {
                core >= ytrace::diagnosis::SSH_ROW_CORE_THRESHOLD
            } else {
                core >= ytrace::diagnosis::LOCAL_ROW_CORE_THRESHOLD
                    || mem_kb.map(|kb| kb >= ytrace::diagnosis::LOCAL_ROW_MEM_KB_THRESHOLD).unwrap_or(false)
            };
            if hot {
                if entry.hot_since.is_none() {
                    entry.hot_since = Some(now);
                }
                let hot_secs = now.duration_since(entry.hot_since.unwrap()).as_secs();
                let sustained = if is_ssh {
                    hot_secs >= ytrace::diagnosis::SSH_ROW_SUSTAINED_SECS
                } else {
                    hot_secs >= ytrace::diagnosis::LOCAL_ROW_SUSTAINED_SECS
                };
                if sustained {
                    let should_emit = entry
                        .last_incident_at
                        .map(|last| now.duration_since(last).as_secs() >= 300)
                        .unwrap_or(true);
                    if should_emit {
                        let sample = ytrace::diagnosis::RowResourceSample {
                            row_id: key.clone(),
                            is_ssh,
                            core_fraction: core,
                            mem_kb,
                            duration_secs: hot_secs,
                        };
                        if let Some(incident) = ytrace::diagnosis::diagnose_row(&sample) {
                            let payload = ytrace::diagnosis::incident_payload(&incident);
                            self.provider.incident(
                                "daemon",
                                "row_resource",
                                if is_ssh { "ssh_hot" } else { "local_hot" },
                                payload.clone(),
                            );
                            log_telemetry_incident(&self.yggterm_home, key, &incident, core, mem_kb);
                            if is_ssh && is_ssh_detach_enabled() {
                                let parked = park_fn(key);
                                if parked {
                                    entry.detached_until = Some(now + Duration::from_millis(SSH_REATTACH_DELAY_MS));
                                    acted.push(key.clone());
                                }
                            } else {
                                acted.push(format!("incident:{}", key));
                            }
                            entry.last_incident_at = Some(now);
                        }
                    }
                }
            } else {
                entry.hot_since = None;
            }
        }
        let live_keys: std::collections::HashSet<_> = rows.iter().map(|(k, _)| k).collect();
        self.ticks.retain(|k, _| live_keys.contains(k));
        acted
    }
}

fn read_proc_ticks(pid: i32) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let stat = parse_proc_stat(&text)?;
    Some(stat.cpu_ticks())
}

fn log_telemetry_incident(
    home: &Path,
    row_id: &str,
    incident: &ytrace::diagnosis::Incident,
    core: f64,
    mem_kb: Option<u64>,
) {
    let event = yggterm_core::TerminalTelemetryEvent::new(
        "resource_governor",
        "row_resource",
        incident.id.clone(),
        json!({
            "diagnosis": incident.diagnosis,
            "remedy": incident.remedy,
            "severity": incident.severity.as_str(),
            "kind": incident.kind.as_str(),
            "row_id": row_id,
            "core_fraction": core,
            "mem_kb": mem_kb,
            "observed": incident.observed,
            "threshold": incident.threshold,
            "suggested_queries": incident.suggested_queries,
            "subject": incident.subject,
        }),
    )
    .severity(incident.severity.as_str().to_string())
    .session_path(row_id.to_string())
    .reason(Some(incident.diagnosis.clone()));
    let _ = yggterm_core::append_terminal_telemetry_event(home, &event);
    yggterm_core::append_trace_event(
        home,
        "daemon",
        "resource_governor",
        incident.id.clone(),
        json!({
            "row_id": row_id,
            "diagnosis": incident.diagnosis,
            "severity": incident.severity.as_str(),
        }),
    );
}

pub fn spawn_row_resource_governor(
    yggterm_home: PathBuf,
    runtime: Arc<Mutex<crate::daemon::DaemonRuntime>>,
) {
    std::thread::Builder::new()
        .name("yggterm-row-governor".to_string())
        .spawn(move || {
            let mut gov = ResourceGovernor::new(yggterm_home);
            loop {
                std::thread::sleep(Duration::from_millis(GOVERNOR_INTERVAL_MS));
                let rows: Vec<(String, Option<u32>)> = {
                    let rt = match runtime.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    rt.governor_row_snapshot()
                };
                let park = |key: &str| -> bool {
                    let mut rt = match runtime.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    rt.governor_park_reader(key)
                };
                let unpark = |_key: &str| -> bool { true };
                let _ = gov.tick(&rows, &park, &unpark);
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_ssh_vs_local_by_key() {
        assert!(is_ssh_row("remote-cc://dev/abc"));
        assert!(is_ssh_row("remote-session://host/abc"));
        assert!(is_ssh_row("remote-pi://x/y"));
        assert!(!is_ssh_row("local://abc"));
        assert!(!is_ssh_row("cc-runtime://abc"));
    }
    #[test]
    fn governor_emits_incident_for_hot_ssh_row_over_sustained_window() {
        let s = ytrace::diagnosis::RowResourceSample {
            row_id: "remote-cc://dev/42".into(),
            is_ssh: true,
            core_fraction: 0.9,
            mem_kb: None,
            duration_secs: 60,
        };
        let inc = ytrace::diagnosis::diagnose_row(&s).expect("hot ssh should diagnose");
        assert_eq!(inc.id, "ssh_row_hot");
        assert_eq!(inc.severity, ytrace::diagnosis::Severity::Warn);
    }
    #[test]
    fn governor_respects_env_disable() {
        std::env::set_var("YGGTERM_GOVERNOR", "0");
        let mut gov = ResourceGovernor::new(std::path::PathBuf::from("/tmp"));
        let rows = vec![("local://x".to_string(), Some(1u32))];
        let out = gov.tick(&rows, &|_| false, &|_| false);
        assert!(out.is_empty(), "disabled governor should not act");
        std::env::remove_var("YGGTERM_GOVERNOR");
    }
}
