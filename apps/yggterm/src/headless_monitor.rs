use anyhow::{Context, Result, bail};
use serde_json::json;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;
use yggterm_core::resolve_yggterm_home;
use yggterm_server::{
    HotRestartResult, ServerEndpoint, SessionKind, YGG_LOADING_NOTIFICATION_AFTER_MS,
    YggEventEnvelope, YggEventKind, YggProgress, YggRequestMeta, YggSurface, YggTarget,
    default_endpoint, ensure_local_daemon_running, hot_restart_detailed,
    local_headless_companion_executable_from_current, ping, prepare_update_restart,
    reachable_versioned_daemon_statuses, refresh_managed_cli, refresh_remote_machine,
    request_terminal_launch, retire_daemon, shutdown, snapshot, start_local_session_at, status,
    terminal_ensure, terminal_read, terminal_write,
};

fn remote_machine_details_json(snap: &yggterm_server::ServerUiSnapshot) -> serde_json::Value {
    serde_json::Value::Array(
        snap.remote_machines
            .iter()
            .map(|machine| {
                json!({
                    "machine_key": machine.machine_key,
                    "label": machine.label,
                    "health": machine.health,
                    "remote_deploy_state": machine.remote_deploy_state,
                    "session_count": machine.sessions.len(),
                })
            })
            .collect(),
    )
}

fn remote_machine_health_counts_json(snap: &yggterm_server::ServerUiSnapshot) -> serde_json::Value {
    let healthy = snap
        .remote_machines
        .iter()
        .filter(|machine| matches!(machine.health, yggterm_server::RemoteMachineHealth::Healthy))
        .count();
    let cached = snap
        .remote_machines
        .iter()
        .filter(|machine| matches!(machine.health, yggterm_server::RemoteMachineHealth::Cached))
        .count();
    let offline = snap
        .remote_machines
        .iter()
        .filter(|machine| matches!(machine.health, yggterm_server::RemoteMachineHealth::Offline))
        .count();
    json!({
        "healthy": healthy,
        "cached": cached,
        "offline": offline,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Startup,
    Ping,
    Status,
    Snapshot,
    RefreshRemote,
    TerminalFootprint,
    DisconnectSafe,
    ReconnectCheck,
    GracefulShutdown,
    ServerList,
    HotRestart,
    WaitSession,
    LatencyCheck,
    PanicReport,
    ManagedCliRefresh,
}

impl Scenario {
    fn operation_name(self) -> &'static str {
        match self {
            Self::Startup => "startup_probe",
            Self::Ping => "ping",
            Self::Status => "status",
            Self::Snapshot => "snapshot",
            Self::RefreshRemote => "refresh_remote",
            Self::TerminalFootprint => "terminal_footprint",
            Self::DisconnectSafe => "disconnect_safe",
            Self::ReconnectCheck => "reconnect_check",
            Self::GracefulShutdown => "graceful_shutdown",
            Self::ServerList => "server_list",
            Self::HotRestart => "hot_restart",
            Self::WaitSession => "wait_session",
            Self::LatencyCheck => "latency_check",
            Self::PanicReport => "panic_report",
            Self::ManagedCliRefresh => "managed_cli_refresh",
        }
    }
}

#[derive(Debug)]
struct Config {
    scenario: Scenario,
    iterations: usize,
    jsonl_out: Option<PathBuf>,
    slow_notice_ms: u64,
    delay_ms: u64,
    progress_step_ms: u64,
    cwd: Option<String>,
    title_hint: Option<String>,
    expect_path: Option<String>,
    machine_key: Option<String>,
    session_count: usize,
    burst_bytes: usize,
    all_servers: bool,
    timeout_ms: u64,
    poll_ms: u64,
    interval_ms: u64,
    daemon_exe: Option<PathBuf>,
    expected_version: Option<String>,
    expected_build_id: Option<u64>,
    reason: Option<String>,
    background: bool,
    /// Force the hot-restart even when the daemon would otherwise defer
    /// for session_survival_required. Used by dev/agent deploys that share
    /// a version string. See [[bug-class-auto-hot-restart-version-gated]].
    force: bool,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let cfg = parse_args(args)?;
    let home_dir = resolve_yggterm_home()?;
    let endpoint = default_endpoint(&home_dir);

    for iteration in 0..cfg.iterations {
        run_scenario(&cfg, &endpoint, iteration)?;
        if iteration + 1 < cfg.iterations && cfg.interval_ms > 0 {
            sleep(Duration::from_millis(cfg.interval_ms));
        }
    }
    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<Config> {
    let mut scenario = Scenario::Startup;
    let mut iterations = 1usize;
    let mut jsonl_out = None::<PathBuf>;
    let mut slow_notice_ms = YGG_LOADING_NOTIFICATION_AFTER_MS;
    let mut delay_ms = 0_u64;
    let mut progress_step_ms = 500_u64;
    let mut cwd = None::<String>;
    let mut title_hint = None::<String>;
    let mut expect_path = None::<String>;
    let mut machine_key = None::<String>;
    let mut session_count = 23usize;
    let mut burst_bytes = 512 * 1024usize;
    let mut all_servers = false;
    let mut timeout_ms = 30_000_u64;
    let mut poll_ms = 500_u64;
    let mut interval_ms = 0_u64;
    let mut daemon_exe = None::<PathBuf>;
    let mut expected_version = None::<String>;
    let mut expected_build_id = None::<u64>;
    let mut reason = None::<String>;
    let mut background = false;
    let mut force = false;

    let mut ix = 0usize;
    while ix < args.len() {
        match args[ix].as_str() {
            "--scenario" => {
                ix += 1;
                let value = args.get(ix).context("missing value after --scenario")?;
                scenario = match value.as_str() {
                    "startup" => Scenario::Startup,
                    "ping" => Scenario::Ping,
                    "status" => Scenario::Status,
                    "snapshot" => Scenario::Snapshot,
                    "refresh-remote" => Scenario::RefreshRemote,
                    "terminal-footprint" => Scenario::TerminalFootprint,
                    "disconnect-safe" => Scenario::DisconnectSafe,
                    "reconnect-check" => Scenario::ReconnectCheck,
                    "graceful-shutdown" => Scenario::GracefulShutdown,
                    "server-list" | "status-all" => Scenario::ServerList,
                    "hot-restart" | "hot-update" => Scenario::HotRestart,
                    "wait-session" | "wait-loaded" => Scenario::WaitSession,
                    "latency-check" | "health-check" => Scenario::LatencyCheck,
                    "panic-report" | "incident-report" | "diagnose" => Scenario::PanicReport,
                    "managed-cli-refresh" | "codex-refresh" => Scenario::ManagedCliRefresh,
                    other => bail!("unknown scenario: {other}"),
                };
            }
            "--all" => {
                all_servers = true;
            }
            "--iterations" => {
                ix += 1;
                iterations = args
                    .get(ix)
                    .context("missing value after --iterations")?
                    .parse()
                    .context("invalid --iterations value")?;
            }
            "--jsonl-out" => {
                ix += 1;
                jsonl_out = Some(PathBuf::from(
                    args.get(ix).context("missing value after --jsonl-out")?,
                ));
            }
            "--slow-notice-ms" => {
                ix += 1;
                slow_notice_ms = args
                    .get(ix)
                    .context("missing value after --slow-notice-ms")?
                    .parse()
                    .context("invalid --slow-notice-ms value")?;
            }
            "--delay-ms" => {
                ix += 1;
                delay_ms = args
                    .get(ix)
                    .context("missing value after --delay-ms")?
                    .parse()
                    .context("invalid --delay-ms value")?;
            }
            "--progress-step-ms" => {
                ix += 1;
                progress_step_ms = args
                    .get(ix)
                    .context("missing value after --progress-step-ms")?
                    .parse()
                    .context("invalid --progress-step-ms value")?;
            }
            "--cwd" => {
                ix += 1;
                cwd = Some(
                    args.get(ix)
                        .context("missing value after --cwd")?
                        .to_string(),
                );
            }
            "--title-hint" => {
                ix += 1;
                title_hint = Some(
                    args.get(ix)
                        .context("missing value after --title-hint")?
                        .to_string(),
                );
            }
            "--expect-path" => {
                ix += 1;
                expect_path = Some(
                    args.get(ix)
                        .context("missing value after --expect-path")?
                        .to_string(),
                );
            }
            "--machine-key" => {
                ix += 1;
                machine_key = Some(
                    args.get(ix)
                        .context("missing value after --machine-key")?
                        .to_string(),
                );
            }
            "--session-count" => {
                ix += 1;
                session_count = args
                    .get(ix)
                    .context("missing value after --session-count")?
                    .parse()
                    .context("invalid --session-count value")?;
            }
            "--burst-bytes" => {
                ix += 1;
                burst_bytes = args
                    .get(ix)
                    .context("missing value after --burst-bytes")?
                    .parse()
                    .context("invalid --burst-bytes value")?;
            }
            "--timeout-ms" => {
                ix += 1;
                timeout_ms = args
                    .get(ix)
                    .context("missing value after --timeout-ms")?
                    .parse()
                    .context("invalid --timeout-ms value")?;
            }
            "--poll-ms" => {
                ix += 1;
                poll_ms = args
                    .get(ix)
                    .context("missing value after --poll-ms")?
                    .parse()
                    .context("invalid --poll-ms value")?;
            }
            "--interval-ms" => {
                ix += 1;
                interval_ms = args
                    .get(ix)
                    .context("missing value after --interval-ms")?
                    .parse()
                    .context("invalid --interval-ms value")?;
            }
            "--daemon-exe" => {
                ix += 1;
                daemon_exe = Some(PathBuf::from(
                    args.get(ix).context("missing value after --daemon-exe")?,
                ));
            }
            "--expected-version" => {
                ix += 1;
                expected_version = Some(
                    args.get(ix)
                        .context("missing value after --expected-version")?
                        .to_string(),
                );
            }
            "--expected-build-id" => {
                ix += 1;
                expected_build_id = Some(
                    args.get(ix)
                        .context("missing value after --expected-build-id")?
                        .parse()
                        .context("invalid --expected-build-id value")?,
                );
            }
            "--reason" => {
                ix += 1;
                reason = Some(
                    args.get(ix)
                        .context("missing value after --reason")?
                        .to_string(),
                );
            }
            "--background" => {
                background = true;
            }
            "--foreground" => {
                background = false;
            }
            "--force" => {
                // For `--scenario hot-restart`: bypass the same-version
                // refusal and the session-survival deferral so the swap
                // proceeds even when version_string matches the running
                // daemon (dev/agent deploy case). See
                // [[bug-class-auto-hot-restart-version-gated]].
                force = true;
            }
            other => bail!("unknown argument: {other}"),
        }
        ix += 1;
    }

    Ok(Config {
        scenario,
        iterations,
        jsonl_out,
        slow_notice_ms,
        delay_ms,
        progress_step_ms,
        cwd,
        title_hint,
        expect_path,
        machine_key,
        session_count,
        burst_bytes,
        all_servers,
        timeout_ms,
        poll_ms,
        interval_ms,
        daemon_exe,
        expected_version,
        expected_build_id,
        reason,
        background,
        force,
    })
}

fn endpoint_json(endpoint: &ServerEndpoint) -> serde_json::Value {
    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => json!({
            "transport": "unix",
            "path": path.display().to_string(),
        }),
        ServerEndpoint::Tcp { host, port } => json!({
            "transport": "tcp",
            "host": host,
            "port": port,
        }),
    }
}

fn server_status_json(
    endpoint: &ServerEndpoint,
    daemon_status: &yggterm_server::ServerRuntimeStatus,
) -> serde_json::Value {
    json!({
        "endpoint": endpoint_json(endpoint),
        "server_version": daemon_status.server_version,
        "server_build_id": daemon_status.server_build_id,
        "server_pid": daemon_status.server_pid,
        "host_kind": daemon_status.host_kind,
        "host_detail": daemon_status.host_detail,
        "owned_terminal_session_count": daemon_status.owned_terminal_session_count,
        "owned_terminal_session_keys": daemon_status.owned_terminal_session_keys,
        "terminal_session_count": daemon_status.terminal_session_count,
        "terminal_session_keys": daemon_status.terminal_session_keys,
        "restored_live_sessions": daemon_status.restored_live_sessions,
        "preserved_terminal_owner_count": daemon_status.preserved_terminal_owner_count,
        "preserved_terminal_owner_keys": daemon_status.preserved_terminal_owner_keys,
        "managed_session_count": daemon_status.managed_session_count,
        "restored_from_persisted_state": daemon_status.restored_from_persisted_state,
    })
}

fn daemon_status_has_live_terminal_runtime(
    daemon_status: &yggterm_server::ServerRuntimeStatus,
) -> bool {
    !daemon_status_owned_terminal_runtime_keys(daemon_status).is_empty()
}

fn daemon_status_owned_terminal_runtime_keys(
    daemon_status: &yggterm_server::ServerRuntimeStatus,
) -> Vec<String> {
    if !daemon_status.owned_terminal_session_keys.is_empty() {
        return daemon_status.owned_terminal_session_keys.clone();
    }
    if daemon_status.owned_terminal_session_count > 0
        && !daemon_status.terminal_session_keys.is_empty()
    {
        return daemon_status.terminal_session_keys.clone();
    }
    let has_preserved_owner = daemon_status.preserved_terminal_owner_count > 0
        || !daemon_status.preserved_terminal_owner_keys.is_empty();
    let has_legacy_terminal_keys =
        daemon_status.terminal_session_count > 0 || !daemon_status.terminal_session_keys.is_empty();
    if !has_preserved_owner && has_legacy_terminal_keys {
        return daemon_status.terminal_session_keys.clone();
    }
    Vec::new()
}

fn daemon_status_retire_guard_runtime_keys(
    daemon_status: &yggterm_server::ServerRuntimeStatus,
) -> Vec<String> {
    let mut keys = daemon_status_owned_terminal_runtime_keys(daemon_status);
    keys.extend(daemon_status.terminal_session_keys.iter().cloned());
    keys.extend(daemon_status.preserved_terminal_owner_keys.iter().cloned());
    keys.sort();
    keys.dedup();
    keys
}

fn daemon_status_owns_all_runtime_keys(
    daemon_status: &yggterm_server::ServerRuntimeStatus,
    runtime_keys: &[String],
) -> bool {
    if runtime_keys.is_empty() {
        return false;
    }
    let owned_keys = daemon_status_owned_terminal_runtime_keys(daemon_status)
        .into_iter()
        .collect::<HashSet<_>>();
    runtime_keys
        .iter()
        .all(|runtime_key| owned_keys.contains(runtime_key))
}

fn hot_restart_target_runtime_covered_by_expected_server(
    target_status: &yggterm_server::ServerRuntimeStatus,
    all_targets: &[(ServerEndpoint, yggterm_server::ServerRuntimeStatus)],
    cfg: &Config,
) -> Option<yggterm_server::ServerRuntimeStatus> {
    let runtime_keys = daemon_status_owned_terminal_runtime_keys(target_status);
    if runtime_keys.is_empty() {
        return None;
    }
    all_targets
        .iter()
        .map(|(_, status)| status)
        .filter(|status| status.server_pid != target_status.server_pid)
        .filter(|status| status_matches_expected(status, cfg))
        .filter(|status| daemon_status_owns_all_runtime_keys(status, &runtime_keys))
        .cloned()
        .max_by_key(|status| (status.server_build_id, status.server_pid))
}

fn hot_restart_target_retire_covered_by_expected_server(
    target_status: &yggterm_server::ServerRuntimeStatus,
    all_targets: &[(ServerEndpoint, yggterm_server::ServerRuntimeStatus)],
    cfg: &Config,
) -> Option<yggterm_server::ServerRuntimeStatus> {
    if target_status.owned_terminal_session_count > 0
        || !target_status.owned_terminal_session_keys.is_empty()
    {
        return hot_restart_target_runtime_covered_by_expected_server(
            target_status,
            all_targets,
            cfg,
        );
    }
    let guard_keys = daemon_status_retire_guard_runtime_keys(target_status);
    if guard_keys.is_empty() {
        return None;
    }
    all_targets
        .iter()
        .map(|(_, status)| status)
        .filter(|status| status.server_pid != target_status.server_pid)
        .filter(|status| status_matches_expected(status, cfg))
        .filter(|status| daemon_status_owns_all_runtime_keys(status, &guard_keys))
        .cloned()
        .max_by_key(|status| (status.server_build_id, status.server_pid))
}

fn retire_stale_daemon_without_session_shutdown(
    target: &ServerEndpoint,
    daemon_status: &yggterm_server::ServerRuntimeStatus,
    owner_status: &yggterm_server::ServerRuntimeStatus,
    reason: &'static str,
) -> serde_json::Value {
    let retire_result = retire_daemon(target, Some(reason))
        .map(|message| json!({"ok": true, "message": message}))
        .unwrap_or_else(|retire_error| json!({"ok": false, "error": retire_error.to_string()}));
    let process_retire = terminate_stale_daemon_process_if_needed(daemon_status.server_pid);
    json!({
        "endpoint": endpoint_json(target),
        "server": server_status_json(target, daemon_status),
        "native_hot_restart": false,
        "hot_update_handoff": false,
        "fallback_shutdown_skipped": false,
        "fallback_shutdown_used": false,
        "duplicate_runtime_owner_retired": true,
        "daemon_retired_without_session_shutdown": true,
        "covered_by_pid": owner_status.server_pid,
        "covered_by_version": owner_status.server_version,
        "covered_runtime_keys": daemon_status_retire_guard_runtime_keys(daemon_status),
        "retire": retire_result,
        "process_retire": process_retire,
        "update_priority": "duplicate_runtime_retire",
    })
}

#[cfg(target_os = "linux")]
fn linux_process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "linux")]
fn terminate_stale_daemon_process_if_needed(pid: u32) -> serde_json::Value {
    if pid == std::process::id() || pid == 0 {
        return json!({
            "attempted": false,
            "reason": "refusing_to_terminate_current_or_invalid_pid",
            "pid": pid,
        });
    }
    if !linux_process_exists(pid) {
        return json!({
            "attempted": false,
            "reason": "already_exited",
            "pid": pid,
        });
    }
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    for _ in 0..10 {
        sleep(Duration::from_millis(100));
        if !linux_process_exists(pid) {
            return json!({
                "attempted": true,
                "pid": pid,
                "signal": "SIGTERM",
                "exited": true,
            });
        }
    }
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    sleep(Duration::from_millis(120));
    json!({
        "attempted": true,
        "pid": pid,
        "signal": "SIGKILL",
        "exited": !linux_process_exists(pid),
    })
}

#[cfg(not(target_os = "linux"))]
fn terminate_stale_daemon_process_if_needed(pid: u32) -> serde_json::Value {
    json!({
        "attempted": false,
        "pid": pid,
        "reason": "process_retire_fallback_linux_only",
    })
}

fn reachable_servers_json(home_dir: &Path) -> serde_json::Value {
    let servers = reachable_versioned_daemon_statuses(home_dir)
        .into_iter()
        .map(|(endpoint, daemon_status)| server_status_json(&endpoint, &daemon_status))
        .collect::<Vec<_>>();
    json!({
        "server_count": servers.len(),
        "servers": servers,
    })
}

fn resolve_hot_restart_daemon_exe(cfg: &Config) -> Result<PathBuf> {
    let daemon_exe = if let Some(path) = cfg.daemon_exe.clone() {
        path
    } else {
        let current_exe =
            std::env::current_exe().context("resolving yggterm-headless monitor path")?;
        local_headless_companion_executable_from_current(&current_exe).with_context(|| {
            format!(
                "missing yggterm-headless companion next to {}",
                current_exe.display()
            )
        })?
    };
    let metadata = std::fs::metadata(&daemon_exe)
        .with_context(|| format!("reading daemon executable {}", daemon_exe.display()))?;
    if !metadata.is_file() {
        bail!("daemon executable is not a file: {}", daemon_exe.display());
    }
    Ok(daemon_exe)
}

fn status_matches_expected(
    daemon_status: &yggterm_server::ServerRuntimeStatus,
    cfg: &Config,
) -> bool {
    let expected_version = cfg
        .expected_version
        .as_deref()
        .or_else(|| (cfg.scenario == Scenario::HotRestart).then_some(env!("CARGO_PKG_VERSION")));
    expected_version.is_none_or(|version| daemon_status.server_version == version)
        && cfg
            .expected_build_id
            .is_none_or(|build_id| daemon_status.server_build_id == build_id)
}

fn wait_for_daemon_status(endpoint: &ServerEndpoint, cfg: &Config) -> Result<serde_json::Value> {
    let started = Instant::now();
    loop {
        match status(endpoint) {
            Ok(daemon_status) if status_matches_expected(&daemon_status, cfg) => {
                return Ok(json!({
                    "ready": true,
                    "elapsed_ms": started.elapsed().as_millis() as u64,
                    "server": server_status_json(endpoint, &daemon_status),
                }));
            }
            Ok(daemon_status) if started.elapsed() >= Duration::from_millis(cfg.timeout_ms) => {
                bail!(
                    "daemon reachable but expected version/build did not match after {}ms: version={} build_id={}",
                    cfg.timeout_ms,
                    daemon_status.server_version,
                    daemon_status.server_build_id
                );
            }
            Err(error) if started.elapsed() >= Duration::from_millis(cfg.timeout_ms) => {
                bail!(
                    "daemon did not become reachable after {}ms: {}",
                    cfg.timeout_ms,
                    error
                );
            }
            _ => sleep(Duration::from_millis(cfg.poll_ms.max(1))),
        }
    }
}

fn session_present(
    expected_path: &str,
    snap: &yggterm_server::ServerUiSnapshot,
    daemon_status: &yggterm_server::ServerRuntimeStatus,
) -> (bool, bool, bool) {
    let active_matches = snap
        .active_session_path
        .as_deref()
        .is_some_and(|path| path == expected_path);
    let listed = snap
        .live_sessions
        .iter()
        .any(|session| session.session_path == expected_path);
    let terminal_keyed = daemon_status
        .terminal_session_keys
        .iter()
        .any(|path| path == expected_path);
    (active_matches, listed, terminal_keyed)
}

fn panic_report_targets(
    home_dir: &Path,
    endpoint: &ServerEndpoint,
) -> Vec<(ServerEndpoint, Option<yggterm_server::ServerRuntimeStatus>)> {
    let mut targets = reachable_versioned_daemon_statuses(home_dir)
        .into_iter()
        .map(|(endpoint, status)| (endpoint, Some(status)))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        targets.push((endpoint.clone(), None));
    }
    targets
}

fn run_scenario(
    cfg: &Config,
    endpoint: &yggterm_server::ServerEndpoint,
    iteration: usize,
) -> Result<()> {
    let request_id = format!(
        "yggterm-headless-monitor-{}-{iteration}",
        cfg.scenario.operation_name()
    );
    let meta = YggRequestMeta::interactive(
        request_id,
        cfg.scenario.operation_name(),
        YggSurface::App,
        YggTarget::App,
    );

    emit(
        cfg,
        YggEventEnvelope::new(meta.clone(), YggEventKind::Accepted)
            .with_message(format!("starting {}", cfg.scenario.operation_name())),
    )?;
    let start = Instant::now();
    emit(
        cfg,
        YggEventEnvelope::new(meta.clone(), YggEventKind::Loading)
            .with_message("waiting on daemon")
            .with_progress(YggProgress {
                step: "dispatch".to_string(),
                current: Some(0),
                total: Some(1),
                message: Some("sending request".to_string()),
            }),
    )?;

    maybe_emit_artificial_delay(cfg, &meta)?;

    let result: Result<serde_json::Value> = (|| -> Result<serde_json::Value> {
        match cfg.scenario {
            Scenario::Startup => {
                ping(endpoint)?;
                let daemon_status = status(endpoint)?;
                let (snap, _) = snapshot(endpoint)?;
                Ok(json!({
                    "server_version": daemon_status.server_version,
                    "server_build_id": daemon_status.server_build_id,
                    "restored_from_persisted_state": daemon_status.restored_from_persisted_state,
                    "restored_stored_sessions": daemon_status.restored_stored_sessions,
                    "restored_live_sessions": daemon_status.restored_live_sessions,
                    "restored_remote_machines": daemon_status.restored_remote_machines,
                    "active_session_path": snap.active_session_path,
                    "active_view_mode": snap.active_view_mode,
                    "remote_machines": snap.remote_machines.len(),
                    "remote_machine_details": remote_machine_details_json(&snap),
                    "remote_machine_health_counts": remote_machine_health_counts_json(&snap),
                    "ssh_targets": snap.ssh_targets.len(),
                    "live_sessions": snap.live_sessions.len(),
                }))
            }
            Scenario::Ping => {
                ping(endpoint)?;
                Ok(json!({"pong": true}))
            }
            Scenario::Status => {
                let daemon_status = status(endpoint)?;
                Ok(serde_json::to_value(daemon_status)?)
            }
            Scenario::ServerList => {
                let home_dir = resolve_yggterm_home()?;
                Ok(reachable_servers_json(&home_dir))
            }
            Scenario::Snapshot => {
                let (snap, message) = snapshot(endpoint)?;
                Ok(json!({
                    "message": message,
                    "active_session_path": snap.active_session_path,
                    "active_view_mode": snap.active_view_mode,
                    "remote_machines": snap.remote_machines.len(),
                    "remote_machine_details": remote_machine_details_json(&snap),
                    "remote_machine_health_counts": remote_machine_health_counts_json(&snap),
                    "ssh_targets": snap.ssh_targets.len(),
                    "live_sessions": snap.live_sessions.len(),
                }))
            }
            Scenario::RefreshRemote => {
                let machine_key = cfg
                    .machine_key
                    .as_deref()
                    .context("--machine-key is required for refresh-remote")?;
                ping(endpoint)?;
                let (snap, message) = refresh_remote_machine(endpoint, machine_key)?;
                let machine = snap
                    .remote_machines
                    .iter()
                    .find(|machine| machine.machine_key == machine_key);
                Ok(json!({
                    "message": message,
                    "machine_key": machine_key,
                    "health": machine.map(|machine| &machine.health),
                    "session_count": machine.map(|machine| machine.sessions.len()).unwrap_or_default(),
                }))
            }
            Scenario::TerminalFootprint => {
                ping(endpoint)?;
                let mut session_paths = Vec::new();
                for ix in 0..cfg.session_count {
                    let (snap, _) = start_local_session_at(
                        endpoint,
                        SessionKind::Shell,
                        cfg.cwd.as_deref(),
                        Some(&format!("mock footprint {}", ix + 1)),
                    )?;
                    let session_path = snap
                        .active_session_path
                        .clone()
                        .context("terminal-footprint scenario did not produce an active session")?;
                    let _ = request_terminal_launch(endpoint)?;
                    let _ = terminal_ensure(endpoint, &session_path)?;
                    let command = format!(
                        "head -c {bytes} /dev/zero | tr '\\\\000' x; printf '\\\\n'\n",
                        bytes = cfg.burst_bytes
                    );
                    let _ = terminal_write(endpoint, &session_path, &command)?;
                    std::thread::sleep(Duration::from_millis(80));
                    let _ = terminal_read(endpoint, &session_path, 0)?;
                    session_paths.push(session_path);
                }
                std::thread::sleep(Duration::from_millis(250));
                let daemon_status = status(endpoint)?;
                Ok(json!({
                    "session_paths": session_paths,
                    "requested_session_count": cfg.session_count,
                    "burst_bytes": cfg.burst_bytes,
                    "terminal_session_count": daemon_status.terminal_session_count,
                    "terminal_retained_chunks": daemon_status.terminal_retained_chunks,
                    "terminal_retained_bytes": daemon_status.terminal_retained_bytes,
                    "terminal_session_buffer_limit_bytes": daemon_status.terminal_session_buffer_limit_bytes,
                    "terminal_idle_buffer_limit_bytes": daemon_status.terminal_idle_buffer_limit_bytes,
                }))
            }
            Scenario::DisconnectSafe => {
                ping(endpoint)?;
                let (snap, _) = start_local_session_at(
                    endpoint,
                    SessionKind::Shell,
                    cfg.cwd.as_deref(),
                    cfg.title_hint.as_deref().or(Some("mock lifetime probe")),
                )?;
                let session_path = snap
                    .active_session_path
                    .clone()
                    .context("disconnect-safe scenario did not produce an active session")?;
                let _ = request_terminal_launch(endpoint);
                let (snap, _) = snapshot(endpoint)?;
                let retained_after_disconnect = snap
                    .active_session_path
                    .as_deref()
                    .is_some_and(|path| path == session_path)
                    || snap
                        .live_sessions
                        .iter()
                        .any(|session| session.session_path == session_path);
                Ok(json!({
                    "session_path": session_path,
                    "active_view_mode": snap.active_view_mode,
                    "live_sessions": snap.live_sessions.len(),
                    "retained_after_disconnect": retained_after_disconnect,
                    "note": "client may now exit without shutdown; rerun reconnect-check with --expect-path"
                }))
            }
            Scenario::ReconnectCheck => {
                let expected_path = cfg
                    .expect_path
                    .as_deref()
                    .context("--expect-path is required for reconnect-check")?;
                ping(endpoint)?;
                let daemon_status = status(endpoint)?;
                let (snap, message) = snapshot(endpoint)?;
                let active_matches = snap
                    .active_session_path
                    .as_deref()
                    .is_some_and(|path| path == expected_path);
                let listed = snap
                    .live_sessions
                    .iter()
                    .any(|session| session.session_path == expected_path);
                if !active_matches && !listed {
                    bail!("expected session path not found after reconnect: {expected_path}");
                }
                Ok(json!({
                    "message": message,
                    "expected_path": expected_path,
                    "active_matches": active_matches,
                    "listed": listed,
                    "active_session_path": snap.active_session_path,
                    "live_sessions": snap.live_sessions.len(),
                    "restored_from_persisted_state": daemon_status.restored_from_persisted_state,
                    "restored_stored_sessions": daemon_status.restored_stored_sessions,
                    "restored_live_sessions": daemon_status.restored_live_sessions,
                    "restored_remote_machines": daemon_status.restored_remote_machines
                }))
            }
            Scenario::WaitSession => {
                let expected_path = cfg
                    .expect_path
                    .as_deref()
                    .context("--expect-path is required for wait-session")?;
                let started = Instant::now();
                loop {
                    let daemon_status = status(endpoint)?;
                    let (snap, message) = snapshot(endpoint)?;
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    let (active_matches, listed, terminal_keyed) =
                        session_present(expected_path, &snap, &daemon_status);
                    if active_matches || listed || terminal_keyed {
                        break Ok(json!({
                            "message": message,
                            "expected_path": expected_path,
                            "active_matches": active_matches,
                            "listed": listed,
                            "terminal_keyed": terminal_keyed,
                            "elapsed_ms": elapsed_ms,
                            "active_session_path": snap.active_session_path,
                            "live_sessions": snap.live_sessions.len(),
                            "terminal_session_count": daemon_status.terminal_session_count,
                        }));
                    }
                    if elapsed_ms >= cfg.timeout_ms {
                        bail!(
                            "expected session path not found after {}ms: {}",
                            cfg.timeout_ms,
                            expected_path
                        );
                    }
                    emit(
                        cfg,
                        YggEventEnvelope::new(meta.clone(), YggEventKind::Progress)
                            .with_elapsed_ms(elapsed_ms)
                            .with_message("session not loaded yet")
                            .with_progress(YggProgress {
                                step: "poll_session".to_string(),
                                current: None,
                                total: None,
                                message: Some(format!("waiting for {expected_path}")),
                            }),
                    )?;
                    sleep(Duration::from_millis(cfg.poll_ms.max(1)));
                }
            }
            Scenario::LatencyCheck => {
                let home_dir = resolve_yggterm_home()?;
                let targets = if cfg.all_servers {
                    reachable_versioned_daemon_statuses(&home_dir)
                        .into_iter()
                        .map(|(endpoint, daemon_status)| (endpoint, Some(daemon_status)))
                        .collect::<Vec<_>>()
                } else {
                    vec![(endpoint.clone(), None)]
                };
                let mut checks = Vec::new();
                for (target, cached_status) in targets {
                    let ping_start = Instant::now();
                    let ping_error = ping(&target).err().map(|error| error.to_string());
                    let ping_ms = ping_start.elapsed().as_millis() as u64;

                    let status_start = Instant::now();
                    let status_result = status(&target);
                    let status_ms = status_start.elapsed().as_millis() as u64;
                    let daemon_status = status_result.ok().or(cached_status);

                    let snapshot_start = Instant::now();
                    let snapshot_result = snapshot(&target);
                    let snapshot_ms = snapshot_start.elapsed().as_millis() as u64;
                    let snapshot_error = snapshot_result.err().map(|error| error.to_string());
                    let slow = ping_ms >= cfg.slow_notice_ms
                        || status_ms >= cfg.slow_notice_ms
                        || snapshot_ms >= cfg.slow_notice_ms;
                    checks.push(json!({
                    "endpoint": endpoint_json(&target),
                    "server": daemon_status.as_ref().map(|daemon_status| server_status_json(&target, daemon_status)),
                    "ping_ms": ping_ms,
                    "ping_error": ping_error,
                    "status_ms": status_ms,
                    "snapshot_ms": snapshot_ms,
                    "snapshot_error": snapshot_error,
                    "slow": slow,
                }));
                }
                Ok(json!({
                    "server_count": checks.len(),
                    "slow_notice_ms": cfg.slow_notice_ms,
                    "checks": checks,
                }))
            }
            Scenario::PanicReport => {
                let home_dir = resolve_yggterm_home()?;
                let targets = panic_report_targets(&home_dir, endpoint);
                let mut checks = Vec::new();
                let mut recommended_next_steps = Vec::new();
                let expected_path = cfg.expect_path.as_deref();
                for (target, cached_status) in targets {
                    let ping_start = Instant::now();
                    let ping_error = ping(&target).err().map(|error| error.to_string());
                    let ping_ms = ping_start.elapsed().as_millis() as u64;

                    let status_start = Instant::now();
                    let status_result = status(&target);
                    let status_ms = status_start.elapsed().as_millis() as u64;
                    let status_error = status_result.as_ref().err().map(|error| error.to_string());
                    let daemon_status = status_result.ok().or(cached_status);

                    let snapshot_start = Instant::now();
                    let snapshot_result = snapshot(&target);
                    let snapshot_ms = snapshot_start.elapsed().as_millis() as u64;
                    let snapshot_error = snapshot_result
                        .as_ref()
                        .err()
                        .map(|error| error.to_string());
                    let snapshot_data = snapshot_result.ok().map(|(snap, message)| {
                    let expected_presence = expected_path
                        .zip(daemon_status.as_ref())
                        .map(|(expected_path, daemon_status)| {
                            let (active_matches, listed, terminal_keyed) =
                                session_present(expected_path, &snap, daemon_status);
                            if !active_matches && !listed && !terminal_keyed {
                                recommended_next_steps.push(format!(
                                    "expected session not present on {}: run wait-session or inspect restore path",
                                    serde_json::to_string(&endpoint_json(&target))
                                        .unwrap_or_else(|_| format!("{target:?}"))
                                ));
                            }
                            json!({
                                "expected_path": expected_path,
                                "active_matches": active_matches,
                                "listed": listed,
                                "terminal_keyed": terminal_keyed,
                            })
                        });
                    json!({
                        "message": message,
                        "active_session_path": snap.active_session_path,
                        "active_view_mode": snap.active_view_mode,
                        "live_sessions": snap.live_sessions.len(),
                        "remote_machines": snap.remote_machines.len(),
                        "remote_machine_health_counts": remote_machine_health_counts_json(&snap),
                        "expected_presence": expected_presence,
                    })
                });

                    let slow = ping_ms >= cfg.slow_notice_ms
                        || status_ms >= cfg.slow_notice_ms
                        || snapshot_ms >= cfg.slow_notice_ms;
                    if ping_error.is_some() || status_error.is_some() {
                        recommended_next_steps.push(
                        "daemon control plane is not reachable; check server-list, active sockets, and hot-restart if appropriate".to_string(),
                    );
                    }
                    if snapshot_error.is_some() {
                        recommended_next_steps.push(
                        "snapshot failed; inspect event trace and app-control state before assuming the GUI is at fault".to_string(),
                    );
                    }
                    if slow {
                        recommended_next_steps.push(format!(
                            "latency threshold exceeded on {}: ping={}ms status={}ms snapshot={}ms",
                            serde_json::to_string(&endpoint_json(&target))
                                .unwrap_or_else(|_| format!("{target:?}")),
                            ping_ms,
                            status_ms,
                            snapshot_ms
                        ));
                    }

                    checks.push(json!({
                    "endpoint": endpoint_json(&target),
                    "server": daemon_status.as_ref().map(|daemon_status| server_status_json(&target, daemon_status)),
                    "ping_ms": ping_ms,
                    "ping_error": ping_error,
                    "status_ms": status_ms,
                    "status_error": status_error,
                    "snapshot_ms": snapshot_ms,
                    "snapshot_error": snapshot_error,
                    "snapshot": snapshot_data,
                    "slow": slow,
                }));
                }
                recommended_next_steps.sort();
                recommended_next_steps.dedup();
                Ok(json!({
                    "home_dir": home_dir.display().to_string(),
                    "server_count": checks.len(),
                    "slow_notice_ms": cfg.slow_notice_ms,
                    "expected_path": expected_path,
                    "checks": checks,
                    "recommended_next_steps": recommended_next_steps,
                }))
            }
            Scenario::HotRestart => {
                let home_dir = resolve_yggterm_home()?;
                let daemon_exe = resolve_hot_restart_daemon_exe(cfg)?;
                let targets = if cfg.all_servers {
                    reachable_versioned_daemon_statuses(&home_dir)
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    vec![(endpoint.clone(), status(endpoint)?)]
                };
                let target_statuses = targets.clone();
                let mut fallback_used = false;
                let mut fallback_shutdown_skipped = false;
                let mut hot_update_handoff_used = false;
                let mut ready_skip_reason = None::<&'static str>;
                let mut target_results = Vec::new();
                for (target, daemon_status) in &targets {
                    if target == endpoint && status_matches_expected(daemon_status, cfg) {
                        target_results.push(json!({
                            "endpoint": endpoint_json(target),
                            "server": server_status_json(target, daemon_status),
                            "native_hot_restart": false,
                            "hot_update_handoff": false,
                            "already_ready": true,
                            "fallback_shutdown_skipped": false,
                            "message": "target daemon already matches expected version/build",
                        }));
                        continue;
                    }
                    if target != endpoint
                        && daemon_status_has_live_terminal_runtime(daemon_status)
                        && let Some(owner_status) =
                            hot_restart_target_runtime_covered_by_expected_server(
                                daemon_status,
                                &target_statuses,
                                cfg,
                            )
                    {
                        fallback_used = true;
                        target_results.push(retire_stale_daemon_without_session_shutdown(
                            target,
                            daemon_status,
                            &owner_status,
                            "duplicate runtime owner covered by expected daemon",
                        ));
                        continue;
                    }
                    if target != endpoint
                        && !daemon_status_has_live_terminal_runtime(daemon_status)
                        && let Some(owner_status) =
                            hot_restart_target_retire_covered_by_expected_server(
                                daemon_status,
                                &target_statuses,
                                cfg,
                            )
                    {
                        fallback_used = true;
                        target_results.push(retire_stale_daemon_without_session_shutdown(
                            target,
                            daemon_status,
                            &owner_status,
                            "stale daemon has no owned runtimes covered by expected daemon",
                        ));
                        continue;
                    }
                    let expected_version = cfg
                        .expected_version
                        .as_deref()
                        .or(Some(env!("CARGO_PKG_VERSION")));
                    let hot_result = hot_restart_detailed(
                        target,
                        &daemon_exe,
                        expected_version,
                        cfg.expected_build_id,
                        cfg.reason
                            .as_deref()
                            .or(Some("yggterm-headless monitor hot restart")),
                        cfg.force,
                    );
                    match hot_result {
                        Ok(HotRestartResult::Restarting { message }) => {
                            target_results.push(json!({
                                "endpoint": endpoint_json(target),
                                "server": server_status_json(target, daemon_status),
                                "native_hot_restart": true,
                                "hot_update_handoff": false,
                                "message": message,
                            }))
                        }
                        Ok(HotRestartResult::Handoff {
                            message,
                            owner_endpoint,
                            owner_server_version,
                            owner_server_pid,
                            target_server_version,
                            runtime_keys,
                        }) => {
                            hot_update_handoff_used = true;
                            fallback_shutdown_skipped = true;
                            ready_skip_reason = Some("hot_update_handoff_preserved_owner");
                            target_results.push(json!({
                                "endpoint": endpoint_json(target),
                                "server": server_status_json(target, daemon_status),
                                "native_hot_restart": false,
                                "hot_update_handoff": true,
                                "fallback_shutdown_skipped": true,
                                "fallback_shutdown_skip_reason": "handoff_preserved_owner",
                                "message": message,
                                "owner_endpoint": owner_endpoint,
                                "owner_server_version": owner_server_version,
                                "owner_server_pid": owner_server_pid,
                                "target_server_version": target_server_version,
                                "runtime_keys": runtime_keys,
                                "update_priority": "handoff_preserve_sessions",
                            }))
                        }
                        Err(error) => {
                            if daemon_status_has_live_terminal_runtime(daemon_status) {
                                if let Some(owner_status) =
                                    hot_restart_target_runtime_covered_by_expected_server(
                                        daemon_status,
                                        &target_statuses,
                                        cfg,
                                    )
                                {
                                    fallback_used = true;
                                    let mut result = retire_stale_daemon_without_session_shutdown(
                                        target,
                                        daemon_status,
                                        &owner_status,
                                        "duplicate runtime owner covered by expected daemon after hot restart error",
                                    );
                                    if let Some(object) = result.as_object_mut() {
                                        object.insert(
                                            "hot_restart_error".to_string(),
                                            json!(error.to_string()),
                                        );
                                    }
                                    target_results.push(result);
                                } else {
                                    fallback_shutdown_skipped = true;
                                    ready_skip_reason = Some("session_survival_required");
                                    target_results.push(json!({
                                        "endpoint": endpoint_json(target),
                                        "server": server_status_json(target, daemon_status),
                                        "native_hot_restart": false,
                                        "hot_restart_error": error.to_string(),
                                        "fallback_shutdown_skipped": true,
                                        "fallback_shutdown_skip_reason": "session_survival_required",
                                        "update_priority": "defer_update_preserve_sessions",
                                    }));
                                }
                            } else {
                                fallback_used = true;
                                let prepare = prepare_update_restart(target)
                                .map(|message| json!({"ok": true, "message": message}))
                                .unwrap_or_else(|prepare_error| {
                                    json!({"ok": false, "error": prepare_error.to_string()})
                                });
                                let shutdown_result = shutdown(target)
                                .map(|message| json!({"ok": true, "message": message}))
                                .unwrap_or_else(|shutdown_error| {
                                    json!({"ok": false, "error": shutdown_error.to_string()})
                                });
                                target_results.push(json!({
                                    "endpoint": endpoint_json(target),
                                    "server": server_status_json(target, daemon_status),
                                    "native_hot_restart": false,
                                    "hot_restart_error": error.to_string(),
                                    "fallback_shutdown_skipped": false,
                                    "prepare_update_restart": prepare,
                                    "shutdown": shutdown_result,
                                }));
                            }
                        }
                    }
                }
                if fallback_used || targets.is_empty() {
                    ensure_local_daemon_running(endpoint)?;
                }
                let ready = if fallback_shutdown_skipped {
                    let observed = status(endpoint)
                        .map(|daemon_status| server_status_json(endpoint, &daemon_status))
                        .ok();
                    json!({
                        "ready": false,
                        "deferred": true,
                        "reason": ready_skip_reason.unwrap_or("session_survival_required"),
                        "server": observed,
                    })
                } else {
                    wait_for_daemon_status(endpoint, cfg)?
                };
                Ok(json!({
                    "daemon_executable": daemon_exe.display().to_string(),
                    "target_count": targets.len(),
                    "fallback_used": fallback_used,
                    "fallback_shutdown_skipped": fallback_shutdown_skipped,
                    "hot_update_handoff_used": hot_update_handoff_used,
                    "targets": target_results,
                    "ready": ready,
                }))
            }
            Scenario::ManagedCliRefresh => {
                ensure_local_daemon_running(endpoint)?;
                let message =
                    refresh_managed_cli(endpoint, cfg.machine_key.as_deref(), cfg.background)?;
                let daemon_status = status(endpoint).ok();
                Ok(json!({
                    "message": message,
                    "machine_key": cfg.machine_key.clone(),
                    "background": cfg.background,
                    "server": daemon_status.as_ref().map(|daemon_status| server_status_json(endpoint, daemon_status)),
                }))
            }
            Scenario::GracefulShutdown => {
                let message = shutdown(endpoint)?;
                let ping_after = ping(endpoint).is_ok();
                Ok(json!({
                    "message": message,
                    "daemon_reachable_after": ping_after
                }))
            }
        }
    })();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    if elapsed_ms >= cfg.slow_notice_ms {
        emit(
            cfg,
            YggEventEnvelope::new(meta.clone(), YggEventKind::Progress)
                .with_elapsed_ms(elapsed_ms)
                .with_message(
                    "loading threshold exceeded; stale-or-local UI should stay interactive",
                )
                .with_progress(YggProgress {
                    step: "waiting".to_string(),
                    current: None,
                    total: None,
                    message: Some("show local loading state and notify the user".to_string()),
                }),
        )?;
    }

    match result {
        Ok(value) => emit(
            cfg,
            YggEventEnvelope::new(meta, YggEventKind::Result)
                .with_elapsed_ms(elapsed_ms)
                .with_data(value),
        ),
        Err(error) => emit(
            cfg,
            YggEventEnvelope::new(meta, YggEventKind::Error)
                .with_elapsed_ms(elapsed_ms)
                .with_message(error.to_string()),
        ),
    }
}

fn maybe_emit_artificial_delay(cfg: &Config, meta: &YggRequestMeta) -> Result<()> {
    if cfg.delay_ms == 0 {
        return Ok(());
    }

    let total_steps = if cfg.progress_step_ms == 0 {
        1
    } else {
        cfg.delay_ms.div_ceil(cfg.progress_step_ms)
    };
    let mut elapsed = 0_u64;
    let step_ms = cfg.progress_step_ms.max(1);

    while elapsed < cfg.delay_ms {
        let sleep_ms = (cfg.delay_ms - elapsed).min(step_ms);
        sleep(Duration::from_millis(sleep_ms));
        elapsed += sleep_ms;

        emit(
            cfg,
            YggEventEnvelope::new(meta.clone(), YggEventKind::Progress)
                .with_elapsed_ms(elapsed)
                .with_message("artificial latency injection")
                .with_progress(YggProgress {
                    step: "delayed".to_string(),
                    current: Some(elapsed.div_ceil(step_ms)),
                    total: Some(total_steps),
                    message: Some(format!(
                        "simulating a slow server path for {}ms",
                        cfg.delay_ms
                    )),
                }),
        )?;
    }

    Ok(())
}

fn emit(cfg: &Config, event: YggEventEnvelope) -> Result<()> {
    let line = serde_json::to_string(&event)?;
    println!("{line}");
    if let Some(path) = &cfg.jsonl_out {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hot_restart_control_options() {
        let cfg = parse_args(vec![
            "--scenario".to_string(),
            "hot-restart".to_string(),
            "--all".to_string(),
            "--daemon-exe".to_string(),
            "/tmp/yggterm-headless".to_string(),
            "--expected-version".to_string(),
            "2.1.61".to_string(),
            "--expected-build-id".to_string(),
            "123".to_string(),
            "--reason".to_string(),
            "release".to_string(),
        ])
        .expect("parse hot restart args");

        assert_eq!(cfg.scenario, Scenario::HotRestart);
        assert!(cfg.all_servers);
        assert_eq!(
            cfg.daemon_exe.as_deref(),
            Some(Path::new("/tmp/yggterm-headless"))
        );
        assert_eq!(cfg.expected_version.as_deref(), Some("2.1.61"));
        assert_eq!(cfg.expected_build_id, Some(123));
        assert_eq!(cfg.reason.as_deref(), Some("release"));
    }

    #[test]
    fn hot_restart_monitor_detects_live_runtime_before_fallback_shutdown() {
        let live_status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.1.61",
                "server_build_id": 0,
                "server_pid": 44,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "restored_live_sessions": 1,
                "terminal_session_count": 1,
                "terminal_session_keys": ["local://kept"],
                "managed_session_count": 0,
            }))
            .expect("live status");
        assert!(daemon_status_has_live_terminal_runtime(&live_status));

        let empty_status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.1.61",
                "server_build_id": 0,
                "server_pid": 45,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "restored_live_sessions": 0,
                "terminal_session_count": 0,
                "terminal_session_keys": [],
                "managed_session_count": 0,
            }))
            .expect("empty status");
        assert!(!daemon_status_has_live_terminal_runtime(&empty_status));

        let preserved_only_status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.1.62",
                "server_build_id": 0,
                "server_pid": 46,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 0,
                "owned_terminal_session_keys": [],
                "terminal_session_count": 1,
                "terminal_session_keys": ["local://preserved"],
                "preserved_terminal_owner_count": 1,
                "preserved_terminal_owner_keys": ["local://preserved"],
                "managed_session_count": 0,
            }))
            .expect("preserved-only status");
        assert!(!daemon_status_has_live_terminal_runtime(
            &preserved_only_status
        ));
    }

    #[test]
    fn hot_restart_monitor_finds_duplicate_runtime_owner_covered_by_expected_server() {
        let stale_status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.2.58",
                "server_build_id": 58,
                "server_pid": 580,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 2,
                "owned_terminal_session_keys": ["remote-session://dev/one", "remote-session://dev/two"],
                "terminal_session_count": 2,
                "terminal_session_keys": ["remote-session://dev/one", "remote-session://dev/two"],
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
                "managed_session_count": 0,
            }))
            .expect("stale status");
        let current_status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.2.59",
                "server_build_id": 59,
                "server_pid": 590,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 2,
                "owned_terminal_session_keys": ["remote-session://dev/one", "remote-session://dev/two"],
                "terminal_session_count": 2,
                "terminal_session_keys": ["remote-session://dev/one", "remote-session://dev/two"],
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
                "managed_session_count": 0,
            }))
            .expect("current status");
        let partial_status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.2.59",
                "server_build_id": 60,
                "server_pid": 600,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 1,
                "owned_terminal_session_keys": ["remote-session://dev/one"],
                "terminal_session_count": 1,
                "terminal_session_keys": ["remote-session://dev/one"],
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
                "managed_session_count": 0,
            }))
            .expect("partial status");
        let cfg = parse_args(vec![
            "--scenario".to_string(),
            "hot-restart".to_string(),
            "--all".to_string(),
            "--expected-version".to_string(),
            "2.2.59".to_string(),
        ])
        .expect("parse cfg");
        let endpoint = yggterm_server::default_endpoint(Path::new("/tmp/yggterm-monitor-test"));
        let targets = vec![
            (endpoint.clone(), stale_status.clone()),
            (endpoint.clone(), partial_status),
            (endpoint, current_status),
        ];

        let owner =
            hot_restart_target_runtime_covered_by_expected_server(&stale_status, &targets, &cfg)
                .expect("duplicate owner coverage");

        assert_eq!(owner.server_pid, 590);
        assert_eq!(owner.server_version, "2.2.59");
    }

    #[test]
    fn hot_restart_monitor_retire_coverage_accepts_preserved_only_when_current_owns_key() {
        let stale_preserved_only: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.4.52",
                "server_build_id": 52,
                "server_pid": 520,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 0,
                "owned_terminal_session_keys": [],
                "terminal_session_count": 1,
                "terminal_session_keys": ["remote-session://dev/samplenotes"],
                "preserved_terminal_owner_count": 1,
                "preserved_terminal_owner_keys": ["remote-session://dev/samplenotes"],
                "managed_session_count": 0,
            }))
            .expect("stale preserved-only status");
        let current_owner: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.4.53",
                "server_build_id": 53,
                "server_pid": 530,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 1,
                "owned_terminal_session_keys": ["remote-session://dev/samplenotes"],
                "terminal_session_count": 1,
                "terminal_session_keys": ["remote-session://dev/samplenotes"],
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
                "managed_session_count": 0,
            }))
            .expect("current owner status");
        let cfg = parse_args(vec![
            "--scenario".to_string(),
            "hot-restart".to_string(),
            "--all".to_string(),
            "--expected-version".to_string(),
            "2.4.53".to_string(),
        ])
        .expect("parse cfg");
        let endpoint = yggterm_server::default_endpoint(Path::new("/tmp/yggterm-monitor-test"));
        let targets = vec![
            (endpoint.clone(), stale_preserved_only.clone()),
            (endpoint, current_owner),
        ];

        let owner = hot_restart_target_retire_covered_by_expected_server(
            &stale_preserved_only,
            &targets,
            &cfg,
        )
        .expect("preserved-only sidecar coverage");
        assert_eq!(owner.server_pid, 530);

        let uncovered_targets = vec![(
            yggterm_server::default_endpoint(Path::new("/tmp/yggterm-monitor-test")),
            stale_preserved_only.clone(),
        )];
        assert!(
            hot_restart_target_retire_covered_by_expected_server(
                &stale_preserved_only,
                &uncovered_targets,
                &cfg,
            )
            .is_none()
        );
    }

    #[test]
    fn hot_restart_monitor_retire_rejects_empty_runtime_coverage() {
        let stale_without_runtime_keys: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.6.12",
                "server_build_id": 612,
                "server_pid": 6120,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 0,
                "owned_terminal_session_keys": [],
                "terminal_session_count": 0,
                "terminal_session_keys": [],
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
                "managed_session_count": 17,
                "restored_live_sessions": 4,
            }))
            .expect("stale empty-key status");
        let current: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.6.13",
                "server_build_id": 613,
                "server_pid": 6130,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "owned_terminal_session_count": 1,
                "owned_terminal_session_keys": ["remote-session://dev/other"],
                "terminal_session_count": 1,
                "terminal_session_keys": ["remote-session://dev/other"],
                "preserved_terminal_owner_count": 0,
                "preserved_terminal_owner_keys": [],
                "managed_session_count": 17,
                "restored_live_sessions": 4,
            }))
            .expect("current status");
        let cfg = parse_args(vec![
            "--scenario".to_string(),
            "hot-restart".to_string(),
            "--all".to_string(),
            "--expected-version".to_string(),
            "2.6.13".to_string(),
        ])
        .expect("parse cfg");
        let endpoint = yggterm_server::default_endpoint(Path::new("/tmp/yggterm-monitor-test"));
        let targets = vec![
            (endpoint.clone(), stale_without_runtime_keys.clone()),
            (endpoint, current),
        ];

        assert!(
            hot_restart_target_retire_covered_by_expected_server(
                &stale_without_runtime_keys,
                &targets,
                &cfg,
            )
            .is_none(),
            "an empty covered_runtime_keys set is not proof that retiring a stale daemon is session-safe"
        );
    }

    #[test]
    fn server_status_json_reports_hot_update_handoff_owners() {
        let status: yggterm_server::ServerRuntimeStatus =
            serde_json::from_value(serde_json::json!({
                "server_version": "2.1.62",
                "server_build_id": 0,
                "server_pid": 62,
                "host_kind": "local",
                "host_detail": "test",
                "embedded_surface_supported": true,
                "bridge_enabled": true,
                "restored_live_sessions": 0,
                "terminal_session_count": 1,
                "terminal_session_keys": ["codex-runtime://kept"],
                "preserved_terminal_owner_count": 1,
                "preserved_terminal_owner_keys": ["codex-runtime://kept"],
                "managed_session_count": 0,
            }))
            .expect("handoff status");
        let endpoint = yggterm_server::default_endpoint(std::path::Path::new("/tmp/yggterm-test"));
        let json = server_status_json(&endpoint, &status);

        assert_eq!(
            json.get("preserved_terminal_owner_count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            json.get("preserved_terminal_owner_keys")
                .and_then(serde_json::Value::as_array)
                .and_then(|keys| keys.first())
                .and_then(serde_json::Value::as_str),
            Some("codex-runtime://kept")
        );
    }

    #[test]
    fn parse_wait_session_and_managed_cli_options() {
        let wait_cfg = parse_args(vec![
            "--scenario".to_string(),
            "wait-session".to_string(),
            "--expect-path".to_string(),
            "live::codex".to_string(),
            "--timeout-ms".to_string(),
            "30000".to_string(),
            "--poll-ms".to_string(),
            "250".to_string(),
        ])
        .expect("parse wait session args");
        assert_eq!(wait_cfg.scenario, Scenario::WaitSession);
        assert_eq!(wait_cfg.expect_path.as_deref(), Some("live::codex"));
        assert_eq!(wait_cfg.timeout_ms, 30_000);
        assert_eq!(wait_cfg.poll_ms, 250);

        let refresh_cfg = parse_args(vec![
            "--scenario".to_string(),
            "managed-cli-refresh".to_string(),
            "--background".to_string(),
            "--machine-key".to_string(),
            "jojo".to_string(),
        ])
        .expect("parse refresh args");
        assert_eq!(refresh_cfg.scenario, Scenario::ManagedCliRefresh);
        assert!(refresh_cfg.background);
        assert_eq!(refresh_cfg.machine_key.as_deref(), Some("jojo"));
    }

    #[test]
    fn parse_panic_report_monitoring_options() {
        let cfg = parse_args(vec![
            "--scenario".to_string(),
            "panic-report".to_string(),
            "--expect-path".to_string(),
            "live::hung-codex".to_string(),
            "--iterations".to_string(),
            "3".to_string(),
            "--interval-ms".to_string(),
            "1000".to_string(),
            "--slow-notice-ms".to_string(),
            "750".to_string(),
        ])
        .expect("parse panic report args");

        assert_eq!(cfg.scenario, Scenario::PanicReport);
        assert_eq!(cfg.expect_path.as_deref(), Some("live::hung-codex"));
        assert_eq!(cfg.iterations, 3);
        assert_eq!(cfg.interval_ms, 1000);
        assert_eq!(cfg.slow_notice_ms, 750);
    }
}
