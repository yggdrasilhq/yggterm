use anyhow::{Context, Result, bail};
use serde_json::json;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;
use yggterm_core::resolve_yggterm_home;
use yggterm_server::{
    SessionKind, YGG_LOADING_NOTIFICATION_AFTER_MS, YggEventEnvelope, YggEventKind, YggProgress,
    YggRequestMeta, YggSurface, YggTarget, default_endpoint, ping, refresh_remote_machine,
    request_terminal_launch, shutdown, snapshot, start_local_session_at, status,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Startup,
    Ping,
    Status,
    Snapshot,
    RefreshRemote,
    DisconnectSafe,
    ReconnectCheck,
    GracefulShutdown,
}

impl Scenario {
    fn operation_name(self) -> &'static str {
        match self {
            Self::Startup => "startup_probe",
            Self::Ping => "ping",
            Self::Status => "status",
            Self::Snapshot => "snapshot",
            Self::RefreshRemote => "refresh_remote",
            Self::DisconnectSafe => "disconnect_safe",
            Self::ReconnectCheck => "reconnect_check",
            Self::GracefulShutdown => "graceful_shutdown",
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
}

fn main() -> Result<()> {
    let cfg = parse_args(env::args().skip(1).collect())?;
    let home_dir = resolve_yggterm_home()?;
    let endpoint = default_endpoint(&home_dir);

    for iteration in 0..cfg.iterations {
        run_scenario(&cfg, &endpoint, iteration)?;
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
                    "disconnect-safe" => Scenario::DisconnectSafe,
                    "reconnect-check" => Scenario::ReconnectCheck,
                    "graceful-shutdown" => Scenario::GracefulShutdown,
                    other => bail!("unknown scenario: {other}"),
                };
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
    })
}

fn run_scenario(
    cfg: &Config,
    endpoint: &yggterm_server::ServerEndpoint,
    iteration: usize,
) -> Result<()> {
    let request_id = format!(
        "yggterm-mock-cli-{}-{iteration}",
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

    let result: Result<serde_json::Value> = match cfg.scenario {
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
        Scenario::Snapshot => {
            let (snap, message) = snapshot(endpoint)?;
            Ok(json!({
                "message": message,
                "active_session_path": snap.active_session_path,
                "active_view_mode": snap.active_view_mode,
                "remote_machines": snap.remote_machines.len(),
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
        Scenario::GracefulShutdown => {
            let message = shutdown(endpoint)?;
            let ping_after = ping(endpoint).is_ok();
            Ok(json!({
                "message": message,
                "daemon_reachable_after": ping_after
            }))
        }
    };

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
