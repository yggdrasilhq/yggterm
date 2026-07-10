use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use yggterm_core::{
    AppSettings, InstallContext, SessionCopyRegenerationFailure, SessionStore,
    best_effort_precis_from_context, best_effort_summary_from_context,
    best_effort_title_from_context, detect_install_context, looks_like_generated_fallback_title,
    looks_like_low_signal_generated_copy,
};
use yggterm_server::{
    AppControlRightPanelMode, AppControlViewMode, ProbeTerminalViewportInputMode,
    RemoteDeployState, RemoteMachineHealth, RemoteMachineSnapshot, RemoteScannedSession,
    ScreenshotPostProcess,
    SessionKind, SshConnectTarget, default_endpoint, detect_ghostty_host,
    ensure_local_daemon_running, fetch_remote_generation_context,
    persist_remote_generated_copy_with_options, ping, run_app_control_background_window,
    run_app_control_close_window, run_app_control_close_window_preserving_sessions,
    run_app_control_create_terminal, run_app_control_describe_rows, run_app_control_describe_state,
    run_app_control_desktop_identity, run_app_control_drag, run_app_control_dump_state,
    run_app_control_focus_window, run_app_control_key, run_app_control_list_clients,
    run_app_control_move_window_by, run_app_control_open_path,
    run_app_control_paste_terminal_clipboard, run_app_control_paste_terminal_clipboard_image,
    run_app_control_dom_eval, run_app_control_grid, run_app_control_pointer,
    run_app_control_probe_terminal_context_menu,
    run_app_control_probe_terminal_primary_selection_paste,
    run_app_control_probe_terminal_viewport_input, run_app_control_probe_terminal_viewport_scroll,
    run_app_control_probe_terminal_viewport_select, run_app_control_reclaim_terminal_focus,
    run_app_control_reconcile_terminal_from_daemon, run_app_control_redraw_terminal,
    run_app_control_remove_session,
    run_app_control_rename_session, run_app_control_restart_session,
    run_app_control_reset_theme_editor, run_app_control_resize_window,
    run_app_control_restart_pending_update, run_app_control_scroll_preview,
    run_app_control_scroll_right_panel, run_app_control_send_terminal_input,
    run_app_control_submit_terminal_prompt,
    run_app_control_set_clipboard_png_base64, run_app_control_set_clipboard_text,
    run_app_control_set_fullscreen, run_app_control_set_main_zoom, run_app_control_set_force_foreground,
    run_app_control_set_maximized,
    run_app_control_set_right_panel_mode, run_app_control_set_row_expanded,
    run_app_control_set_search, run_app_control_set_session_keep_alive,
    run_app_control_set_theme_editor_open, run_app_control_set_theme_editor_values,
    run_app_control_set_tree_selection, run_app_control_set_window_chrome_hover,
    run_app_control_show_start_page, run_app_control_start_action,
    run_app_control_trigger_update_check, run_attach, run_daemon, run_screenrecord_capture,
    run_screenshot_capture, run_screenshot_capture_with_post_process, run_trace_bundle,
    run_trace_follow, run_trace_tail, run_trace_transitions,
    scan_remote_machine_sessions_for_target, shutdown, snapshot, status, terminal_resize,
    terminal_restart,
    terminal_write, try_run_remote_server_command,
};

#[path = "../headless_monitor.rs"]
mod headless_monitor;

const ENV_YGGTERM_DIRECT_INSTALL_ROOT: &str = "YGGTERM_DIRECT_INSTALL_ROOT";
const ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF: &str = "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF";

fn app_control_close_preserve_flag(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--preserve-live-sessions" | "--preserve-sessions" | "--handoff" | "--restart-safe"
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinCliCommand {
    MainHelp,
    Version,
    ServerHelp,
    ServerAppHelp,
    ServerSessionsHelp,
    ServerSnapshot,
}

fn builtin_cli_command_is_pure(command: BuiltinCliCommand) -> bool {
    matches!(
        command,
        BuiltinCliCommand::MainHelp
            | BuiltinCliCommand::Version
            | BuiltinCliCommand::ServerHelp
            | BuiltinCliCommand::ServerAppHelp
            | BuiltinCliCommand::ServerSessionsHelp
    )
}

fn classify_builtin_cli_command(args: &[String]) -> Option<BuiltinCliCommand> {
    match args {
        [] => Some(BuiltinCliCommand::MainHelp),
        [arg] if matches!(arg.as_str(), "--help" | "-h" | "help") => {
            Some(BuiltinCliCommand::MainHelp)
        }
        [arg] if matches!(arg.as_str(), "--version" | "version") => {
            Some(BuiltinCliCommand::Version)
        }
        [command] if command == "server" => Some(BuiltinCliCommand::ServerHelp),
        [command, arg]
            if command == "server" && matches!(arg.as_str(), "--help" | "-h" | "help") =>
        {
            Some(BuiltinCliCommand::ServerHelp)
        }
        [server, app] if server == "server" && app == "app" => {
            Some(BuiltinCliCommand::ServerAppHelp)
        }
        [server, app, rest @ ..]
            if server == "server"
                && app == "app"
                && rest
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "help")) =>
        {
            Some(BuiltinCliCommand::ServerAppHelp)
        }
        [server, sessions]
            if server == "server" && matches!(sessions.as_str(), "sessions" | "session-copy") =>
        {
            Some(BuiltinCliCommand::ServerSessionsHelp)
        }
        [server, sessions, rest @ ..]
            if server == "server"
                && matches!(sessions.as_str(), "sessions" | "session-copy")
                && rest
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "help")) =>
        {
            Some(BuiltinCliCommand::ServerSessionsHelp)
        }
        [command, arg] if command == "server" && arg == "snapshot" => {
            Some(BuiltinCliCommand::ServerSnapshot)
        }
        _ => None,
    }
}

fn print_main_help() {
    println!(
        "usage:
  yggterm-headless
  yggterm-headless --help
  yggterm-headless --version
  yggterm-headless server <subcommand>

common server commands:
  yggterm-headless server daemon
  yggterm-headless server status
  yggterm-headless server snapshot
  yggterm-headless server monitor --scenario panic-report
  yggterm-headless server monitor --scenario latency-check --all
  yggterm-headless server app <subcommand>"
    );
}

fn print_server_help() {
    println!(
        "usage:
  yggterm-headless server daemon
  yggterm-headless server attach <session> [cwd]
  yggterm-headless server ping
  yggterm-headless server status
  yggterm-headless server snapshot
  yggterm-headless server shutdown
  yggterm-headless server terminal write <session> (--data <data>|--stdin)
  yggterm-headless server terminal restart <session> [--terminal-appearance <dark|light>] [--force-remote]
  yggterm-headless server sessions regenerate-copy [--budget <n>] [--force] [--reset-summary-history] [--skip-local] [--skip-remote] [--json]
  yggterm-headless server monitor --scenario <panic-report|server-list|latency-check|wait-session|hot-restart|managed-cli-refresh>
  yggterm-headless server perf-summary [--category <c>] [--since-ms <ms>] [--top <n>] [--json]
  yggterm-headless server trace <tail|follow|bundle|transitions>
  yggterm-headless server screenshot <target> [output]
  yggterm-headless server screenrecord <target> [output]
  yggterm-headless server app <subcommand>"
    );
}

fn print_server_app_help() {
    println!(
        "usage:
  yggterm-headless server app clients
  yggterm-headless server app desktop-identity
  yggterm-headless server app state [--pid <pid>]
  yggterm-headless server app rows [--pid <pid>]
  yggterm-headless server app screenshot [output] [--pid <pid>] [--region terminal|full] [--crop x,y,w,h] [--scale n] [--backend os]
  yggterm-headless server app open <session-path> [--view <terminal|preview>] [--pid <pid>]
  yggterm-headless server app resize-window --width <px> --height <px> [--pid <pid>]
  yggterm-headless server app maximize <on|off|toggle> [--pid <pid>]
  yggterm-headless server app force-foreground <on|off> [--pid <pid>]
  yggterm-headless server app session <remove|delete> <session-path> [--pid <pid>]
  yggterm-headless server app start-page [--pid <pid>]
  yggterm-headless server app update <check|restart>
  yggterm-headless server app terminal <new|send|focus|probe-type|probe-scroll|probe-select|probe-context-menu> ...
  yggterm-headless server app terminal send <session> (--data <data>|--stdin)"
    );
}

fn print_server_sessions_help() {
    println!(
        "usage:
  yggterm-headless server sessions regenerate-copy [--budget <n>] [--force] [--reset-summary-history] [--skip-local] [--skip-remote] [--json]

commands:
  regenerate-copy    Generate Codex session titles and summary timelines for local and app-discovered remote machines.

options:
  --budget <n>                Limit the number of sessions processed; 0 means no explicit limit.
  --force                     Regenerate existing generated copy.
  --reset-summary-history     Rebuild summary timeline history from scratch.
  --skip-local                Skip local ~/.codex history and refresh only app-discovered remote machines.
  --skip-remote               Only regenerate local Codex session copy.
  --json                      Print a machine-readable report."
    );
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlEnvelope<T> {
    data: T,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlStateData {
    remote: AppControlRemoteState,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlRemoteState {
    #[serde(default)]
    machines: Vec<AppControlRemoteMachine>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppControlRemoteMachine {
    machine_key: String,
    label: String,
    ssh_target: String,
}

#[derive(Debug, Clone, Serialize, Default)]
struct RemoteSessionCopyRegenerationReport {
    machine_key: String,
    ssh_target: String,
    scanned: usize,
    title_generated: usize,
    precis_generated: usize,
    summary_generated: usize,
    summary_history_reset: usize,
    skipped: usize,
    failed: Vec<SessionCopyRegenerationFailure>,
}

#[derive(Debug, Clone, Serialize)]
struct CombinedSessionCopyRegenerationReport {
    local: yggterm_core::SessionCopyRegenerationReport,
    remote: Vec<RemoteSessionCopyRegenerationReport>,
}

fn monitor_scenario_alias(command: &str) -> Option<&'static str> {
    match command {
        "diagnose" | "panic-report" | "incident-report" => Some("panic-report"),
        "server-list" | "status-all" => Some("server-list"),
        "hot-restart" | "hot-update" => Some("hot-restart"),
        "wait-session" | "wait-loaded" => Some("wait-session"),
        "latency-check" | "health-check" => Some("latency-check"),
        "managed-cli-refresh" | "codex-refresh" => Some("managed-cli-refresh"),
        _ => None,
    }
}

fn normalize_monitor_args(args: &[String]) -> Option<Vec<String>> {
    match args {
        [first, rest @ ..] if first == "monitor" => Some(rest.to_vec()),
        [first, rest @ ..] if first == "--scenario" => {
            let mut monitor_args = vec![first.clone()];
            monitor_args.extend(rest.iter().cloned());
            Some(monitor_args)
        }
        [server, monitor, rest @ ..] if server == "server" && monitor == "monitor" => {
            Some(rest.to_vec())
        }
        [server, command, rest @ ..] if server == "server" => {
            monitor_scenario_alias(command).map(|scenario| {
                let mut monitor_args = vec!["--scenario".to_string(), scenario.to_string()];
                monitor_args.extend(rest.iter().cloned());
                monitor_args
            })
        }
        [command, rest @ ..] => monitor_scenario_alias(command).map(|scenario| {
            let mut monitor_args = vec!["--scenario".to_string(), scenario.to_string()];
            monitor_args.extend(rest.iter().cloned());
            monitor_args
        }),
        [] => None,
    }
}

fn cli_positional_args(args: &[String], start: usize) -> Vec<&str> {
    let mut positional = Vec::new();
    let mut index = start;
    while index < args.len() {
        let value = args[index].as_str();
        if value.starts_with("--") {
            if index + 1 < args.len() && !args[index + 1].starts_with("--") {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        positional.push(value);
        index += 1;
    }
    positional
}

fn cli_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let inline_prefix = format!("{flag}=");
    for (index, value) in args.iter().enumerate() {
        if value == flag {
            return args
                .get(index + 1)
                .map(String::as_str)
                .filter(|next| !next.starts_with("--"));
        }
        if let Some(inline) = value.strip_prefix(&inline_prefix) {
            return Some(inline);
        }
    }
    None
}

/// Parse the screenshot post-process flags (`--region <name>`, `--crop x,y,w,h`,
/// `--scale N`) into a ScreenshotPostProcess. Mirrors the GUI binary's parser so the
/// headless CLI (what agents drive) gets the SAME crop/zoom/upscale pipeline — the
/// "1920px-frame-is-illegible → crop + upscale the region of interest" affordance.
/// Returns None when no post-process flags are present (capture written verbatim).
/// `--backend os` forces an OS-compositor grab of the window so NATIVE child
/// widgets (web-surface webviews) appear in the frame — the default composite/
/// DOM backends are blind to them. Any other value (or absent) keeps the
/// default backend selection. Mirrors the GUI binary's parser.
fn screenshot_backend_is_compositor(args: &[String]) -> bool {
    cli_flag_value(args, "--backend")
        .map(|value| value.eq_ignore_ascii_case("os"))
        .unwrap_or(false)
}

fn screenshot_post_process_from_args(args: &[String]) -> Option<ScreenshotPostProcess> {
    let region = cli_flag_value(args, "--region").map(str::to_string);
    let crop = cli_flag_value(args, "--crop").and_then(|raw| {
        let parts: Vec<u32> = raw
            .split(',')
            .filter_map(|piece| piece.trim().parse::<u32>().ok())
            .collect();
        if parts.len() == 4 {
            Some((parts[0], parts[1], parts[2], parts[3]))
        } else {
            None
        }
    });
    let scale = cli_flag_value(args, "--scale").and_then(|raw| raw.parse::<f32>().ok());
    if region.is_none() && crop.is_none() && scale.is_none() {
        return None;
    }
    Some(ScreenshotPostProcess {
        region,
        crop,
        scale: scale.unwrap_or(1.0),
    })
}

/// The daemon this CLI invocation talks to. See the twin in `main.rs` — never
/// `default_endpoint` (our own version's socket), or a headless binary newer
/// than the running daemon spawns a rival that cold-restores `server-state.json`
/// and resurrects closed sessions.
fn cli_server_endpoint(home_dir: &std::path::Path) -> yggterm_server::ServerEndpoint {
    yggterm_server::resolve_client_daemon_endpoint(home_dir).endpoint
}

fn ensure_local_server_ready_for_cli(store: &SessionStore) -> Result<()> {
    let resolved = yggterm_server::resolve_client_daemon_endpoint(store.home_dir());
    if resolved.version_mismatch.is_some() {
        // A daemon of another version is live and owns this home's sessions.
        // It is the source of truth; attach to it rather than spawning a peer.
        return Ok(());
    }
    ensure_local_daemon_running(&resolved.endpoint)
}

/// `server update-daemons [--force]` — bring every reachable local daemon onto
/// this binary's version while PRESERVING their live terminal runtimes.
///
/// Each daemon is asked to hot-restart ITSELF (`ServerRequest::HotRestart`): it
/// spawns the new-version successor, keeps its PTY fds, and lingers as the
/// preserved owner while progressive migration drains its sessions one at a
/// time, as each goes idle. Nothing is re-resumed; no in-flight turn is cut.
///
/// It never sends `ServerRequest::Shutdown`. On a daemon older than 2.9.66 that
/// runs `shutdown_all`, which WRITES `/exit\r` into every live PTY — appending
/// to whatever the user has typed and submitting it.
/// (`yggterm_server::shutdown` now refuses to do that too, but the shortest path
/// to "no slash exit" is not to ask.)
/// See [[finding-never-type-into-a-live-prompt]].
///
/// `--force` bypasses the daemon's same-version target check, for a dev/agent
/// deploy that must land. It does NOT bypass the idle gate, which now guards
/// only the destructive cold-shutdown fallback — the handoff itself is
/// ungated. See [[finding-hot-update-never-converges-idle-gate]].
fn run_update_all_daemons(store: &SessionStore, force: bool) -> Result<()> {
    let current_version = yggterm_server::SERVER_PROTOCOL_VERSION;
    let daemon_executable = std::env::current_exe().context("locating current executable")?;
    let mut results = Vec::new();

    for (endpoint, status) in yggterm_server::reachable_versioned_daemon_statuses(store.home_dir())
    {
        if status.server_version == current_version {
            results.push(serde_json::json!({
                "pid": status.server_pid,
                "version": status.server_version,
                "action": "skipped_already_current",
            }));
            continue;
        }
        let outcome = yggterm_server::hot_restart(
            &endpoint,
            &daemon_executable,
            Some(current_version),
            None,
            Some(if force { "forced_update_all" } else { "update_all" }),
        );
        results.push(match outcome {
            Ok(message) => serde_json::json!({
                "pid": status.server_pid,
                "version": status.server_version,
                "target_version": current_version,
                "owned_terminal_session_count": status.owned_terminal_session_count,
                "action": "handoff_requested",
                "message": message,
            }),
            Err(error) => serde_json::json!({
                "pid": status.server_pid,
                "version": status.server_version,
                "target_version": current_version,
                "action": "failed",
                "error": error.to_string(),
            }),
        });
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "current_version": current_version,
            "forced": force,
            "daemons": results,
        }))?
    );
    Ok(())
}

fn discover_remote_machines_from_app_state() -> Result<Vec<RemoteMachineSnapshot>> {
    let binary = std::env::current_exe()
        .context("locating current executable")?
        .with_file_name(if cfg!(target_os = "windows") {
            "yggterm.exe"
        } else {
            "yggterm"
        });
    if !binary.exists() {
        return Ok(Vec::new());
    }
    let output = Command::new(binary)
        .args(["server", "app", "state", "--timeout-ms", "5000"])
        .output()
        .context("running app-control state for remote machine discovery")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let envelope: AppControlEnvelope<AppControlStateData> =
        serde_json::from_slice(&output.stdout).context("parsing app-control state")?;
    Ok(envelope
        .data
        .remote
        .machines
        .into_iter()
        .map(|machine| RemoteMachineSnapshot {
            machine_key: machine.machine_key,
            label: machine.label,
            ssh_target: machine.ssh_target,
            prefix: None,
            remote_binary_expr: None,
            remote_deploy_state: RemoteDeployState::Ready,
            health: RemoteMachineHealth::Healthy,
            sessions: Vec::new(),
        })
        .collect())
}

fn dedupe_remote_machines(machines: Vec<RemoteMachineSnapshot>) -> Vec<RemoteMachineSnapshot> {
    let mut seen = std::collections::BTreeSet::<(String, String)>::new();
    let mut deduped = Vec::new();
    for machine in machines {
        let key = (machine.machine_key.clone(), machine.ssh_target.clone());
        if seen.insert(key) {
            deduped.push(machine);
        }
    }
    deduped
}

fn merge_context_fragments(primary: &str, secondary: &str) -> String {
    let primary = primary.trim();
    let secondary = secondary.trim();
    match (primary.is_empty(), secondary.is_empty()) {
        (true, true) => String::new(),
        (false, true) => primary.to_string(),
        (true, false) => secondary.to_string(),
        (false, false) => {
            let primary_lower = primary.to_ascii_lowercase();
            let secondary_lower = secondary.to_ascii_lowercase();
            if primary_lower.contains(&secondary_lower) {
                primary.to_string()
            } else if secondary_lower.contains(&primary_lower) {
                secondary.to_string()
            } else {
                format!("{primary}\n{secondary}")
            }
        }
    }
}

fn cached_copy_hint_is_usable(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| !looks_like_low_signal_generated_copy(value))
}

fn title_case_path_segment(segment: &str) -> Option<String> {
    let words = segment
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut title = first.to_ascii_uppercase().to_string();
            title.push_str(&chars.as_str().to_ascii_lowercase());
            title
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    (!words.is_empty()).then(|| words.join(" "))
}

fn cwd_title_fallback(cwd: &str) -> Option<String> {
    let meaningful_segment = cwd.split('/').rev().map(str::trim).find(|segment| {
        !segment.is_empty()
            && !matches!(
                segment.to_ascii_lowercase().as_str(),
                "." | "home" | "users" | "user" | "pi" | "gh" | "git" | "src" | "tmp"
            )
    })?;
    let label = title_case_path_segment(meaningful_segment)?;
    let candidate = format!("{label} Workspace");
    (!looks_like_generated_fallback_title(&candidate)).then_some(candidate)
}

fn remote_session_title_fallback(scanned: &RemoteScannedSession, context: &str) -> Option<String> {
    best_effort_title_from_context(context).or_else(|| cwd_title_fallback(&scanned.cwd))
}

fn regenerate_remote_machine_copy(
    store: &SessionStore,
    settings: &AppSettings,
    machine: RemoteMachineSnapshot,
    budget: usize,
    force: bool,
    reset_summary_history: bool,
) -> RemoteSessionCopyRegenerationReport {
    let target = SshConnectTarget {
        label: machine.label.clone(),
        kind: SessionKind::SshShell,
        ssh_target: machine.ssh_target.clone(),
        prefix: machine.prefix.clone(),
        cwd: None,
    };
    let mut report = RemoteSessionCopyRegenerationReport {
        machine_key: machine.machine_key.clone(),
        ssh_target: machine.ssh_target.clone(),
        ..RemoteSessionCopyRegenerationReport::default()
    };
    let sessions = match scan_remote_machine_sessions_for_target(&target) {
        Ok(mut sessions) => {
            sessions.sort_by(|left, right| {
                right
                    .modified_epoch
                    .cmp(&left.modified_epoch)
                    .then_with(|| left.session_id.cmp(&right.session_id))
            });
            if budget > 0 && sessions.len() > budget {
                sessions.truncate(budget);
            }
            sessions
        }
        Err(error) => {
            report.failed.push(SessionCopyRegenerationFailure {
                session_id: String::new(),
                path: machine.ssh_target.clone(),
                stage: "remote_scan".to_string(),
                error: error.to_string(),
            });
            return report;
        }
    };

    for scanned in sessions {
        report.scanned += 1;
        let context = match fetch_remote_generation_context(&target, &scanned.storage_path) {
            Ok(fetched) => merge_context_fragments(&fetched, &scanned.recent_context),
            Err(error) => {
                report.failed.push(SessionCopyRegenerationFailure {
                    session_id: scanned.session_id.clone(),
                    path: scanned.storage_path.clone(),
                    stage: "remote_context".to_string(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        let mut touched = false;
        let should_generate_title = force
            || scanned.title_hint.trim().is_empty()
            || looks_like_generated_fallback_title(&scanned.title_hint);
        let title = if should_generate_title {
            match store.generate_title_for_context(
                settings,
                &scanned.session_id,
                &scanned.cwd,
                &context,
                force,
            ) {
                Ok(Some(value)) => {
                    report.title_generated += 1;
                    touched = true;
                    Some(value)
                }
                Ok(None) => {
                    let fallback = remote_session_title_fallback(&scanned, &context);
                    if fallback.is_some() {
                        report.title_generated += 1;
                        touched = true;
                    }
                    fallback
                }
                Err(error) => {
                    let fallback = remote_session_title_fallback(&scanned, &context);
                    if fallback.is_some() {
                        report.title_generated += 1;
                        touched = true;
                    } else {
                        report.failed.push(SessionCopyRegenerationFailure {
                            session_id: scanned.session_id.clone(),
                            path: scanned.storage_path.clone(),
                            stage: "remote_title".to_string(),
                            error: error.to_string(),
                        });
                    }
                    fallback
                }
            }
        } else {
            Some(scanned.title_hint.clone())
        };
        let should_generate_precis =
            force || !cached_copy_hint_is_usable(scanned.cached_precis.as_deref());
        let precis = if should_generate_precis {
            match store.generate_precis_for_context(
                settings,
                &scanned.session_id,
                &scanned.cwd,
                &context,
                force,
            ) {
                Ok(Some(value)) => {
                    report.precis_generated += 1;
                    touched = true;
                    Some(value)
                }
                Ok(None) => {
                    let fallback = best_effort_precis_from_context(&context);
                    if fallback.is_some() {
                        report.precis_generated += 1;
                        touched = true;
                    }
                    fallback
                }
                Err(error) => {
                    report.failed.push(SessionCopyRegenerationFailure {
                        session_id: scanned.session_id.clone(),
                        path: scanned.storage_path.clone(),
                        stage: "remote_precis".to_string(),
                        error: error.to_string(),
                    });
                    let fallback = best_effort_precis_from_context(&context);
                    if fallback.is_some() {
                        report.precis_generated += 1;
                        touched = true;
                    }
                    fallback
                }
            }
        } else {
            scanned.cached_precis.clone()
        };
        if reset_summary_history {
            report.summary_history_reset += 1;
            touched = true;
        }
        let should_generate_summary = force
            || reset_summary_history
            || !cached_copy_hint_is_usable(scanned.cached_summary.as_deref());
        let summary = if should_generate_summary {
            match store.generate_summary_for_context(
                settings,
                &scanned.session_id,
                &scanned.cwd,
                &context,
                force || reset_summary_history,
            ) {
                Ok(Some(value)) => {
                    report.summary_generated += 1;
                    touched = true;
                    Some(value)
                }
                Ok(None) => {
                    let fallback = best_effort_summary_from_context(&context);
                    if fallback.is_some() {
                        report.summary_generated += 1;
                        touched = true;
                    }
                    fallback
                }
                Err(error) => {
                    report.failed.push(SessionCopyRegenerationFailure {
                        session_id: scanned.session_id.clone(),
                        path: scanned.storage_path.clone(),
                        stage: "remote_summary".to_string(),
                        error: error.to_string(),
                    });
                    let fallback = best_effort_summary_from_context(&context);
                    if fallback.is_some() {
                        report.summary_generated += 1;
                        touched = true;
                    }
                    fallback
                }
            }
        } else {
            scanned.cached_summary.clone()
        };
        if let Err(error) = persist_remote_generated_copy_with_options(
            &machine,
            &scanned.session_id,
            &scanned.cwd,
            title.as_deref(),
            precis.as_deref(),
            summary.as_deref(),
            &settings.interface_llm_model,
            reset_summary_history,
        ) {
            report.failed.push(SessionCopyRegenerationFailure {
                session_id: scanned.session_id.clone(),
                path: scanned.storage_path.clone(),
                stage: "remote_persist".to_string(),
                error: error.to_string(),
            });
            continue;
        }
        if !touched {
            report.skipped += 1;
        }
    }

    report
}

fn run_sessions_regenerate_copy_cli(store: &SessionStore, args: &[String]) -> Result<()> {
    let action = args
        .get(2)
        .map(String::as_str)
        .context("missing server sessions action")?;
    if !matches!(
        action,
        "regenerate-copy" | "regenerate" | "copy" | "refresh-copy"
    ) {
        anyhow::bail!("unsupported server sessions action: {action}");
    }
    let settings = store.load_settings()?;
    let budget = cli_flag_value(args, "--budget")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let force = args.iter().any(|arg| arg == "--force");
    let reset_summary_history = args
        .iter()
        .any(|arg| arg == "--reset-summary-history" || arg == "--reset-history");
    let skip_local = args.iter().any(|arg| arg == "--skip-local");
    let skip_remote = args.iter().any(|arg| arg == "--skip-remote");
    let local_report = if skip_local {
        yggterm_core::SessionCopyRegenerationReport::default()
    } else {
        store.regenerate_codex_session_copy(&settings, budget, force, reset_summary_history)?
    };
    let remote_reports = if skip_remote {
        Vec::new()
    } else {
        dedupe_remote_machines(discover_remote_machines_from_app_state().unwrap_or_default())
            .into_iter()
            .map(|machine| {
                regenerate_remote_machine_copy(
                    store,
                    &settings,
                    machine,
                    budget,
                    force,
                    reset_summary_history,
                )
            })
            .collect::<Vec<_>>()
    };
    if args.iter().any(|arg| arg == "--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&CombinedSessionCopyRegenerationReport {
                local: local_report,
                remote: remote_reports,
            })?
        );
    } else {
        println!(
            "scanned={} title_generated={} precis_generated={} summary_generated={} summary_history_reset={} skipped={} failed={}",
            local_report.scanned,
            local_report.title_generated,
            local_report.precis_generated,
            local_report.summary_generated,
            local_report.summary_history_reset,
            local_report.skipped,
            local_report.failed.len()
        );
        for failure in local_report.failed.iter().take(12) {
            println!(
                "failed {} {}: {}",
                failure.stage, failure.session_id, failure.error
            );
        }
        for remote in &remote_reports {
            println!(
                "remote machine={} scanned={} title_generated={} precis_generated={} summary_generated={} summary_history_reset={} skipped={} failed={}",
                remote.ssh_target,
                remote.scanned,
                remote.title_generated,
                remote.precis_generated,
                remote.summary_generated,
                remote.summary_history_reset,
                remote.skipped,
                remote.failed.len()
            );
            for failure in remote.failed.iter().take(12) {
                println!(
                    "failed remote {} {} {}: {}",
                    remote.ssh_target, failure.stage, failure.session_id, failure.error
                );
            }
        }
    }
    Ok(())
}

fn paths_same_executable(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        return left
            .to_string_lossy()
            .replace('/', "\\")
            .eq_ignore_ascii_case(&right.to_string_lossy().replace('/', "\\"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

fn preferred_headless_executable(install_context: &InstallContext) -> Option<std::path::PathBuf> {
    let preferred_gui = install_context.preferred_executable.as_ref()?;
    let binary_name = if cfg!(target_os = "windows") {
        "yggterm-headless.exe"
    } else {
        "yggterm-headless"
    };
    Some(
        preferred_gui
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(binary_name),
    )
}

fn gui_companion_executable_from_headless(current_exe: &Path) -> Option<std::path::PathBuf> {
    let file_name = current_exe.file_name()?.to_string_lossy();
    let gui_name = if cfg!(target_os = "windows") {
        file_name.replace("yggterm-headless", "yggterm")
    } else {
        file_name.replace("yggterm-headless", "yggterm")
    };
    if gui_name == file_name.as_ref() {
        return None;
    }
    Some(
        current_exe
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(gui_name),
    )
}

fn preferred_gui_executable_from_headless(
    current_exe: &Path,
    install_context: &InstallContext,
) -> Option<std::path::PathBuf> {
    install_context
        .preferred_executable
        .clone()
        .filter(|path| path.is_file())
        .or_else(|| {
            gui_companion_executable_from_headless(current_exe).filter(|path| path.is_file())
        })
}

fn run_app_launch_via_gui_companion(
    current_exe: &Path,
    args: &[String],
    install_context: &InstallContext,
) -> Result<()> {
    let Some(gui_exe) = preferred_gui_executable_from_headless(current_exe, install_context) else {
        anyhow::bail!(
            "server app launch requires a yggterm GUI companion next to {} or in install-state",
            current_exe.display()
        );
    };
    let mut command = Command::new(&gui_exe);
    command.args(args);
    if let Some(root) = install_context.managed_root.as_ref() {
        command.env(ENV_YGGTERM_DIRECT_INSTALL_ROOT, root);
    }
    let status = command
        .status()
        .with_context(|| format!("launching app via GUI companion {}", gui_exe.display()))?;
    if !status.success() {
        anyhow::bail!(
            "server app launch via {} exited with status {}",
            gui_exe.display(),
            status
        );
    }
    Ok(())
}

fn maybe_handoff_to_preferred_headless_executable(
    current_exe: &Path,
    args: &[String],
    install_context: &InstallContext,
) -> Result<()> {
    if std::env::var_os(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF).is_some() {
        return Ok(());
    }
    if classify_builtin_cli_command(args).is_some_and(builtin_cli_command_is_pure) {
        return Ok(());
    }
    // `perf-summary` reads the LOCAL perf-telemetry.jsonl in-process and never talks to
    // the daemon, so it must run in THIS (newest) binary. Handing it off to a stale
    // active-executable (e.g. a dev deploy that overwrote ~/.local/bin but not
    // install-state) would hit a binary that predates the command and fail.
    if matches!(args.first().map(String::as_str), Some("server"))
        && matches!(args.get(1).map(String::as_str), Some("perf-summary"))
    {
        return Ok(());
    }
    let Some(preferred) = preferred_headless_executable(install_context) else {
        return Ok(());
    };
    let current = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.to_path_buf());
    let preferred = preferred
        .canonicalize()
        .unwrap_or_else(|_| preferred.to_path_buf());
    if paths_same_executable(&current, &preferred) || !preferred.is_file() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new(&preferred);
        command.args(args);
        command.env(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF, "1");
        if let Some(root) = install_context.managed_root.as_ref() {
            command.env(ENV_YGGTERM_DIRECT_INSTALL_ROOT, root);
        }
        let error = command.exec();
        return Err(error).with_context(|| {
            format!("failed to exec headless command as {}", preferred.display())
        });
    }

    #[cfg(not(unix))]
    {
        let mut command = Command::new(&preferred);
        command.args(args);
        command.env(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF, "1");
        if let Some(root) = install_context.managed_root.as_ref() {
            command.env(ENV_YGGTERM_DIRECT_INSTALL_ROOT, root);
        }
        let status = command.status().with_context(|| {
            format!(
                "failed to hand off headless command to {}",
                preferred.display()
            )
        })?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let current_exe = std::env::current_exe()?;
    let install_context = detect_install_context(&current_exe)?;
    maybe_handoff_to_preferred_headless_executable(&current_exe, &args, &install_context)?;
    let store = SessionStore::open_or_init()?;

    if let Some(command) = classify_builtin_cli_command(&args) {
        match command {
            BuiltinCliCommand::MainHelp => {
                print_main_help();
                return Ok(());
            }
            BuiltinCliCommand::Version => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            BuiltinCliCommand::ServerHelp => {
                print_server_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerAppHelp => {
                print_server_app_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerSessionsHelp => {
                print_server_sessions_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerSnapshot => {
                ensure_local_server_ready_for_cli(&store)?;
                let endpoint = cli_server_endpoint(store.home_dir());
                let (snapshot, _) = snapshot(&endpoint)?;
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
                return Ok(());
            }
        }
    }

    if args.as_slice() == ["server", "daemon"] {
        let endpoint = default_endpoint(store.home_dir());
        let host = detect_ghostty_host();
        return run_daemon(&endpoint, host);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "attach" {
        return run_attach(
            &args[2],
            args.get(3)
                .map(String::as_str)
                .filter(|value| !value.is_empty()),
        );
    }
    if args.len() >= 5 && args[0] == "server" && args[1] == "terminal" && args[2] == "write" {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let data = if args.iter().any(|arg| arg == "--stdin") {
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .context("reading terminal write stdin")?;
            value
        } else {
            cli_flag_value(&args, "--data")
                .context("missing --data or --stdin for server terminal write")?
                .to_string()
        };
        terminal_write(&endpoint, &args[3], &data)?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "accepted": true,
                "session_path": args[3],
                "bytes": data.len(),
            }))?
        );
        return Ok(());
    }
    if args.len() >= 4 && args[0] == "server" && args[1] == "terminal" && args[2] == "restart" {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let terminal_appearance = cli_flag_value(&args, "--terminal-appearance");
        let force_remote = args.iter().any(|arg| arg == "--force-remote");
        let (snapshot, message) =
            terminal_restart(&endpoint, &args[3], terminal_appearance, force_remote)?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "accepted": true,
                "session_path": args[3],
                "force_remote": force_remote,
                "message": message,
                "active_session_path": snapshot.active_session_path,
            }))?
        );
        return Ok(());
    }
    if args.len() >= 4 && args[0] == "server" && args[1] == "terminal" && args[2] == "resize" {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let cols = cli_flag_value(&args, "--cols")
            .and_then(|v| v.parse::<u16>().ok())
            .context("missing/invalid --cols for server terminal resize")?;
        let rows = cli_flag_value(&args, "--rows")
            .and_then(|v| v.parse::<u16>().ok())
            .context("missing/invalid --rows for server terminal resize")?;
        // Resizing the LOCAL daemon PTY sends a SIGWINCH down the ssh channel to the
        // remote agent CLI — the way to confirm/recover a "squish" where the remote
        // codex is rendering at a stale smaller grid than the client (re-resume after a
        // daemon restart). Idle codex repaints on the next frame; pass a transient
        // off-size then the real size with `--nudge` to force a fresh SIGWINCH when the
        // daemon PTY already matches. See finding-codex-squish-post-restart-pty-size.
        let nudge = args.iter().any(|a| a == "--nudge");
        if nudge {
            let _ = terminal_resize(&endpoint, &args[3], cols.saturating_sub(1).max(1), rows.saturating_sub(1).max(1));
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let message = terminal_resize(&endpoint, &args[3], cols, rows)?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "accepted": true,
                "session_path": args[3],
                "cols": cols,
                "rows": rows,
                "nudged": nudge,
                "message": message,
            }))?
        );
        return Ok(());
    }
    // `server sessions reorder <order.json>` — set the Live-region row order from
    // an explicit list of session paths. Written for incident recovery: the DAEMON
    // owns row order (the GUI's row-order ledger only mirrors it), the GUI's only
    // way to set it is a mouse drag, and a hand-organized order is real user work
    // that a bad restart can scramble. A lost order is recoverable from
    // `event-trace.jsonl` → the last `live_session_reorder_persisted` payload.
    if args.len() >= 4 && args[0] == "server" && args[1] == "sessions" && args[2] == "reorder" {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let order_path = &args[3];
        let raw = std::fs::read_to_string(order_path)
            .with_context(|| format!("reading order file {order_path}"))?;
        let ordered_paths: Vec<String> = serde_json::from_str(&raw)
            .with_context(|| format!("{order_path} must be a JSON array of session paths"))?;
        if ordered_paths.is_empty() {
            anyhow::bail!("{order_path} is empty; refusing to clear the row order");
        }
        let (snapshot, message) =
            yggterm_server::reorder_live_sessions(&endpoint, &ordered_paths)?;
        // The daemon keeps only the rows it actually has, so report what the order
        // BECAME rather than echoing the request back as if it succeeded.
        let applied: Vec<&str> = snapshot
            .live_sessions
            .iter()
            .map(|session| session.session_path.as_str())
            .collect();
        let requested: Vec<&str> = ordered_paths.iter().map(String::as_str).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "requested": requested.len(),
                "live_rows": applied.len(),
                "matches_request": applied == requested,
                "applied_order": applied,
                "message": message,
            }))?
        );
        return Ok(());
    }
    if args.len() >= 3
        && args[0] == "server"
        && matches!(args[1].as_str(), "sessions" | "session-copy")
    {
        return run_sessions_regenerate_copy_cli(&store, &args);
    }
    if let Some(monitor_args) = normalize_monitor_args(&args) {
        return headless_monitor::run(monitor_args);
    }
    if try_run_remote_server_command(&args)? {
        return Ok(());
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "tail" {
        let lines = args
            .get(3)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(200);
        return run_trace_tail(lines);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "follow" {
        let lines = args
            .get(3)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(200);
        let poll_ms = args
            .get(4)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(500);
        return run_trace_follow(lines, poll_ms);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "transitions" {
        let session_filter = args
            .windows(2)
            .find_map(|window| (window[0] == "--session").then(|| window[1].clone()));
        let last_ms = args
            .windows(2)
            .find_map(|window| {
                (window[0] == "--last-ms").then(|| window[1].parse::<u64>().ok())?
            })
            .unwrap_or(180_000);
        let limit = args
            .windows(2)
            .find_map(|window| {
                (window[0] == "--limit").then(|| window[1].parse::<usize>().ok())?
            })
            .unwrap_or(200);
        return run_trace_transitions(session_filter.as_deref(), last_ms, limit);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "bundle" {
        let lines = args
            .get(3)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(200);
        let include_screenshot = args.iter().any(|value| value == "--screenshot");
        return run_trace_bundle(lines, include_screenshot);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "screenshot" {
        let timeout_ms = args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--timeout-ms" {
                    window[1].parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(15_000);
        let target = args[2].clone();
        let output_path = cli_positional_args(&args, 3)
            .into_iter()
            .find(|value| *value != target);
        let compositor = screenshot_backend_is_compositor(&args);
        return match (screenshot_post_process_from_args(&args), compositor) {
            (None, false) => run_screenshot_capture(&target, output_path, timeout_ms),
            (post, compositor) => run_screenshot_capture_with_post_process(
                &target,
                output_path,
                timeout_ms,
                post.unwrap_or(ScreenshotPostProcess {
                    region: None,
                    crop: None,
                    scale: 1.0,
                }),
                compositor,
            ),
        };
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "screenrecord" {
        let duration_secs = args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--duration-sec" {
                    window[1].parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(10);
        let timeout_ms = args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--timeout-ms" {
                    window[1].parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(duration_secs.saturating_mul(1_000) + 15_000);
        let output_path = args
            .iter()
            .skip(3)
            .find(|value| !value.starts_with("--"))
            .map(String::as_str);
        return run_screenrecord_capture(&args[2], output_path, timeout_ms, duration_secs);
    }
    if let Some(command) = classify_builtin_cli_command(&args) {
        match command {
            BuiltinCliCommand::MainHelp => {
                print_main_help();
                return Ok(());
            }
            BuiltinCliCommand::Version => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            BuiltinCliCommand::ServerHelp => {
                print_server_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerAppHelp => {
                print_server_app_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerSessionsHelp => {
                print_server_sessions_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerSnapshot => {
                ensure_local_server_ready_for_cli(&store)?;
                let endpoint = cli_server_endpoint(store.home_dir());
                let (snapshot, _) = snapshot(&endpoint)?;
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
                return Ok(());
            }
        }
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "app" {
        let preferred_pid = args.windows(2).find_map(|window| {
            if window[0] == "--pid" {
                window[1].parse::<u32>().ok()
            } else {
                None
            }
        });
        if let Some(preferred_pid) = preferred_pid {
            unsafe {
                std::env::set_var("YGGTERM_APP_CONTROL_PID", preferred_pid.to_string());
            }
        } else {
            unsafe {
                std::env::remove_var("YGGTERM_APP_CONTROL_PID");
            }
        }
        let timeout_ms = args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--timeout-ms" {
                    window[1].parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(15_000);
        return match args[2].as_str() {
            "--help" | "-h" | "help" => {
                print_server_app_help();
                Ok(())
            }
            "screenshot" => {
                let target = args
                    .windows(2)
                    .find_map(|window| {
                        if window[0] == "--target" {
                            Some(window[1].as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("app");
                let output_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .find(|value| *value != target);
                let compositor = screenshot_backend_is_compositor(&args);
                match (screenshot_post_process_from_args(&args), compositor) {
                    (None, false) => run_screenshot_capture(target, output_path, timeout_ms),
                    (post, compositor) => run_screenshot_capture_with_post_process(
                        target,
                        output_path,
                        timeout_ms,
                        post.unwrap_or(ScreenshotPostProcess {
                            region: None,
                            crop: None,
                            scale: 1.0,
                        }),
                        compositor,
                    ),
                }
            }
            "screenrecord" => {
                let duration_secs = args
                    .windows(2)
                    .find_map(|window| {
                        if window[0] == "--duration-sec" {
                            window[1].parse::<u64>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(10);
                let output_path = cli_positional_args(&args, 3).into_iter().next();
                run_screenrecord_capture("app", output_path, timeout_ms, duration_secs)
            }
            "launch" => run_app_launch_via_gui_companion(&current_exe, &args, &install_context),
            "clients" => run_app_control_list_clients(),
            "desktop-identity" => run_app_control_desktop_identity(),
            "state" => run_app_control_describe_state(timeout_ms),
            "dump" => {
                let output_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing output path for server app dump"))?;
                run_app_control_dump_state(output_path, timeout_ms)
            }
            "rows" => run_app_control_describe_rows(timeout_ms),
            "preview" | "web-view" | "webview" => {
                let action = args.get(3).map(String::as_str).unwrap_or("scroll");
                match action {
                    "scroll" => {
                        let top_px = args.windows(2).find_map(|window| {
                            if window[0] == "--top" {
                                window[1].parse::<f64>().ok()
                            } else {
                                None
                            }
                        });
                        let ratio = args.windows(2).find_map(|window| {
                            if window[0] == "--ratio" {
                                window[1].parse::<f64>().ok()
                            } else {
                                None
                            }
                        });
                        run_app_control_scroll_preview(top_px, ratio, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app web view action: {other}"),
                }
            }
            "zoom" => {
                let value = args
                    .windows(2)
                    .find_map(|window| {
                        (window[0] == "--value").then(|| window[1].parse::<f32>().ok())
                    })
                    .flatten()
                    .ok_or_else(|| anyhow::anyhow!("missing --value for server app zoom"))?;
                let view_mode = args.windows(2).find_map(|window| {
                    if window[0] != "--view" {
                        return None;
                    }
                    match window[1].as_str() {
                        "preview" | "rendered" | "web-view" | "webview" => {
                            Some(AppControlViewMode::Preview)
                        }
                        "terminal" => Some(AppControlViewMode::Terminal),
                        _ => None,
                    }
                });
                run_app_control_set_main_zoom(value, view_mode, timeout_ms)
            }
            "expand" | "collapse" => {
                let row_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        anyhow::anyhow!("missing row path for server app expand/collapse")
                    })?;
                run_app_control_set_row_expanded(row_path, args[2] == "expand", timeout_ms)
            }
            "focus" => run_app_control_focus_window(timeout_ms),
            "background" | "minimize" => run_app_control_background_window(timeout_ms),
            "move-window" | "move-by" | "nudge" => {
                let delta_x = args.windows(2).find_map(|window| {
                    if window[0] == "--delta-x" || window[0] == "--dx" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let delta_y = args.windows(2).find_map(|window| {
                    if window[0] == "--delta-y" || window[0] == "--dy" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                run_app_control_move_window_by(
                    delta_x.context("missing --delta-x/--dx for server app move-window")?,
                    delta_y.context("missing --delta-y/--dy for server app move-window")?,
                    timeout_ms,
                )
            }
            "resize-window" | "set-window-size" | "size" => {
                let width = args.windows(2).find_map(|window| {
                    if window[0] == "--width" || window[0] == "--w" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let height = args.windows(2).find_map(|window| {
                    if window[0] == "--height" || window[0] == "--h" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                run_app_control_resize_window(
                    width.context("missing --width/--w for server app resize-window")?,
                    height.context("missing --height/--h for server app resize-window")?,
                    timeout_ms,
                )
            }
            "close" | "quit" | "exit" => {
                if app_control_close_preserve_flag(&args) {
                    run_app_control_close_window_preserving_sessions(
                        timeout_ms,
                        Some("manual-preserve-close".to_string()),
                    )
                } else {
                    run_app_control_close_window(timeout_ms)
                }
            }
            "chrome-hover" | "titlebar-hover" => {
                let active = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .map(|value| match value {
                        "on" | "true" | "1" | "hover" | "enter" => Some(true),
                        "off" | "false" | "0" | "leave" => Some(false),
                        _ => None,
                    })
                    .flatten()
                    .context("missing or invalid hover state for server app chrome-hover")?;
                run_app_control_set_window_chrome_hover(active, timeout_ms)
            }
            "search" => {
                let action = args.get(3).map(String::as_str).unwrap_or("set");
                match action {
                    "set" => {
                        let query = cli_flag_value(&args, "--query")
                            .or_else(|| cli_flag_value(&args, "--value"))
                            .or_else(|| cli_positional_args(&args, 4).into_iter().next())
                            .unwrap_or("");
                        let focused = args.windows(2).find_map(|window| {
                            if window[0] != "--focus" {
                                return None;
                            }
                            match window[1].as_str() {
                                "on" | "true" | "1" => Some(true),
                                "off" | "false" | "0" => Some(false),
                                _ => None,
                            }
                        });
                        run_app_control_set_search(query, focused, timeout_ms)
                    }
                    "clear" => run_app_control_set_search("", Some(false), timeout_ms),
                    other => anyhow::bail!("unsupported app search action: {other}"),
                }
            }
            "clipboard" => {
                let action = args.get(3).map(String::as_str).unwrap_or("text");
                match action {
                    "text" | "set" => {
                        let value = cli_flag_value(&args, "--value")
                            .or_else(|| cli_flag_value(&args, "--text"))
                            .or_else(|| {
                                args.iter()
                                    .skip(4)
                                    .find(|value| !value.starts_with("--"))
                                    .map(String::as_str)
                            })
                            .unwrap_or("");
                        run_app_control_set_clipboard_text(value, timeout_ms)
                    }
                    "png" | "image" | "png-base64" => {
                        let value = cli_flag_value(&args, "--base64")
                            .or_else(|| cli_flag_value(&args, "--value"))
                            .or_else(|| {
                                args.iter()
                                    .skip(4)
                                    .find(|value| !value.starts_with("--"))
                                    .map(String::as_str)
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing --base64/--value for server app clipboard image"
                                )
                            })?;
                        run_app_control_set_clipboard_png_base64(value, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app clipboard action: {other}"),
                }
            }
            "panel" | "right-panel" => {
                let mode = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("hidden");
                if mode == "scroll" {
                    let top_px = args.windows(2).find_map(|window| {
                        if window[0] == "--top" {
                            window[1].parse::<f64>().ok()
                        } else {
                            None
                        }
                    });
                    let ratio = args.windows(2).find_map(|window| {
                        if window[0] == "--ratio" {
                            window[1].parse::<f64>().ok()
                        } else {
                            None
                        }
                    });
                    return run_app_control_scroll_right_panel(top_px, ratio, timeout_ms);
                }
                let mode = match mode {
                    "hidden" | "hide" | "close" | "none" => AppControlRightPanelMode::Hidden,
                    "connect" => AppControlRightPanelMode::Connect,
                    "notifications" | "notification" => AppControlRightPanelMode::Notifications,
                    "settings" => AppControlRightPanelMode::Settings,
                    "metadata" | "session-metadata" => AppControlRightPanelMode::Metadata,
                    // `pane:<id>` opens a pane the ACTIVE APP contributed over
                    // OSC 7717 (e.g. `pane:vault`). yggterm does not know the
                    // ids; the app declares them.
                    pane if pane.starts_with("pane:") => AppControlRightPanelMode::AppPane {
                        id: pane.trim_start_matches("pane:").to_string(),
                    },
                    other => anyhow::bail!(
                        "unsupported app right panel mode: {other} \
                         (try hidden|connect|notifications|settings|metadata|pane:<id>)"
                    ),
                };
                run_app_control_set_right_panel_mode(mode, timeout_ms)
            }
            "update" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("check");
                match action {
                    "check" | "trigger" => run_app_control_trigger_update_check(timeout_ms),
                    "restart" => run_app_control_restart_pending_update(timeout_ms),
                    other => anyhow::bail!("unsupported app update action: {other}"),
                }
            }
            "theme-editor" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("open");
                match action {
                    "open" | "show" | "on" | "true" | "1" => {
                        run_app_control_set_theme_editor_open(true, timeout_ms)
                    }
                    "close" | "hide" | "off" | "false" | "0" => {
                        run_app_control_set_theme_editor_open(false, timeout_ms)
                    }
                    "reset" | "defaults" => run_app_control_reset_theme_editor(timeout_ms),
                    "set" | "values" => {
                        let brightness = cli_flag_value(&args, "--brightness")
                            .map(str::parse::<f32>)
                            .transpose()
                            .context("invalid --brightness for server app theme-editor set")?;
                        let alpha = cli_flag_value(&args, "--alpha")
                            .map(str::parse::<f32>)
                            .transpose()
                            .context("invalid --alpha for server app theme-editor set")?;
                        let grain = cli_flag_value(&args, "--grain")
                            .map(str::parse::<f32>)
                            .transpose()
                            .context("invalid --grain for server app theme-editor set")?;
                        run_app_control_set_theme_editor_values(
                            brightness, alpha, grain, timeout_ms,
                        )
                    }
                    other => anyhow::bail!("unsupported app theme-editor action: {other}"),
                }
            }
            "fullscreen" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("toggle");
                let current_state = yggterm_server::request_app_control(
                    store.home_dir(),
                    yggterm_server::AppControlCommand::DescribeState,
                    timeout_ms,
                )?;
                let currently_fullscreen = current_state
                    .data
                    .as_ref()
                    .and_then(|data| data.get("shell"))
                    .and_then(|shell| shell.get("fullscreen"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let enabled = match action {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    "toggle" => !currently_fullscreen,
                    other => anyhow::bail!("unsupported fullscreen action: {other}"),
                };
                run_app_control_set_fullscreen(enabled, timeout_ms)
            }
            "maximize" | "maximized" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("toggle");
                let current_state = yggterm_server::request_app_control(
                    store.home_dir(),
                    yggterm_server::AppControlCommand::DescribeState,
                    timeout_ms,
                )?;
                let currently_maximized = current_state
                    .data
                    .as_ref()
                    .and_then(|data| data.get("window"))
                    .and_then(|window| window.get("maximized"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let enabled = match action {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    "toggle" => !currently_maximized,
                    other => anyhow::bail!("unsupported maximize action: {other}"),
                };
                run_app_control_set_maximized(enabled, timeout_ms)
            }
            "force-foreground" | "force-fg" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("on");
                let enabled = match action {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    other => anyhow::bail!("unsupported force-foreground action: {other}"),
                };
                run_app_control_set_force_foreground(enabled, timeout_ms)
            }
            "open" => {
                let session_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing session path for server app open"))?;
                let view_mode = args.windows(2).find_map(|window| {
                    if window[0] != "--view" {
                        return None;
                    }
                    match window[1].as_str() {
                        "preview" | "rendered" | "web-view" | "webview" => {
                            Some(AppControlViewMode::Preview)
                        }
                        "terminal" => Some(AppControlViewMode::Terminal),
                        _ => None,
                    }
                });
                run_app_control_open_path(session_path, view_mode, timeout_ms)
            }
            "drag" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app drag"))?;
                let row_path = cli_positional_args(&args, 4).into_iter().next();
                let placement = args.windows(2).find_map(|window| {
                    if window[0] == "--placement" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                });
                run_app_control_drag(action, row_path, placement, timeout_ms)
            }
            "pointer" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app pointer"))?;
                let x = args.windows(2).find_map(|window| {
                    if window[0] == "--x" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let y = args.windows(2).find_map(|window| {
                    if window[0] == "--y" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let start_x = args.windows(2).find_map(|window| {
                    if window[0] == "--start-x" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let start_y = args.windows(2).find_map(|window| {
                    if window[0] == "--start-y" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let end_x = args.windows(2).find_map(|window| {
                    if window[0] == "--end-x" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let end_y = args.windows(2).find_map(|window| {
                    if window[0] == "--end-y" {
                        window[1].parse::<f64>().ok()
                    } else {
                        None
                    }
                });
                let button = args.windows(2).find_map(|window| {
                    if window[0] == "--button" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                });
                let count = args.windows(2).find_map(|window| {
                    if window[0] == "--count" {
                        window[1].parse::<u8>().ok()
                    } else {
                        None
                    }
                });
                let steps = args.windows(2).find_map(|window| {
                    if window[0] == "--steps" {
                        window[1].parse::<u16>().ok()
                    } else {
                        None
                    }
                });
                let step_delay_ms = args.windows(2).find_map(|window| {
                    if window[0] == "--step-delay-ms" {
                        window[1].parse::<u64>().ok()
                    } else {
                        None
                    }
                });
                run_app_control_pointer(
                    action,
                    x,
                    y,
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    button,
                    count,
                    steps,
                    step_delay_ms,
                    timeout_ms,
                )
            }
            "grid" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing action for server app grid")?;
                let cell = cli_positional_args(&args, 4).into_iter().next();
                let cols = cli_flag_value(&args, "--cols").and_then(|v| v.parse::<u32>().ok());
                let rows = cli_flag_value(&args, "--rows").and_then(|v| v.parse::<u32>().ok());
                let region = cli_flag_value(&args, "--region");
                let target = cli_flag_value(&args, "--target");
                let ttl_secs =
                    cli_flag_value(&args, "--ttl-secs").and_then(|v| v.parse::<u64>().ok());
                let button = cli_flag_value(&args, "--button");
                let count = cli_flag_value(&args, "--count").and_then(|v| v.parse::<u8>().ok());
                let refine = args.iter().any(|arg| arg == "--refine");
                let keep = args.iter().any(|arg| arg == "--keep");
                run_app_control_grid(
                    action, cell, cols, rows, region, target, ttl_secs, button, count, refine,
                    keep, timeout_ms,
                )
            }
            "dom-eval" => {
                let script = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing script for server app dom-eval")?;
                run_app_control_dom_eval(script, timeout_ms)
            }
            "start-action" | "start" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app start-action"))?;
                run_app_control_start_action(action, timeout_ms)
            }
            "start-page" | "show-start-page" | "home" => {
                run_app_control_show_start_page(timeout_ms)
            }
            "tree" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app tree"))?;
                match action {
                    "select" | "selection" => {
                        let paths = cli_positional_args(&args, 4)
                            .into_iter()
                            .map(ToOwned::to_owned)
                            .collect::<Vec<_>>();
                        let anchor_path = cli_flag_value(&args, "--anchor").map(ToOwned::to_owned);
                        run_app_control_set_tree_selection(paths, anchor_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app tree action: {other}"),
                }
            }
            "key" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app key"))?;
                let positional = cli_positional_args(&args, 4);
                let positional_owned = positional
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect::<Vec<_>>();
                let text = args.windows(2).find_map(|window| {
                    if window[0] == "--text" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                });
                let keys = if action == "press" {
                    positional_owned.clone()
                } else {
                    Vec::new()
                };
                run_app_control_key(
                    action,
                    &keys,
                    text.or_else(|| positional.first().copied()),
                    timeout_ms,
                )
            }
            "terminal" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app terminal"))?;
                match action {
                    "new" => {
                        let machine_key = args.windows(2).find_map(|window| {
                            if window[0] == "--machine-key" {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        });
                        let cwd = args.windows(2).find_map(|window| {
                            if window[0] == "--cwd" {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        });
                        let title_hint = args.windows(2).find_map(|window| {
                            if window[0] == "--title" {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        });
                        let kind = args.windows(2).find_map(|window| {
                            if window[0] == "--kind" {
                                Some(window[1].as_str())
                            } else {
                                None
                            }
                        });
                        run_app_control_create_terminal(
                            machine_key,
                            cwd,
                            title_hint,
                            kind,
                            timeout_ms,
                        )
                    }
                    "send" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!("missing session path for server app terminal send")
                            })?;
                        let data = if args.iter().any(|arg| arg == "--stdin") {
                            let mut value = String::new();
                            std::io::stdin()
                                .read_to_string(&mut value)
                                .context("reading app terminal send stdin")?;
                            value
                        } else {
                            args.windows(2)
                                .find_map(|window| {
                                    if window[0] == "--data" {
                                        Some(window[1].as_str())
                                    } else {
                                        None
                                    }
                                })
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "missing --data or --stdin for server app terminal send"
                                    )
                                })?
                                .to_string()
                        };
                        run_app_control_send_terminal_input(session_path, &data, timeout_ms)
                    }
                    "submit" => {
                        // Readiness-gated prompt insertion: waits for the session to
                        // reach an idle interactive prompt, then sends; refuses if it
                        // never becomes ready. `--ready-timeout-ms` bounds the wait.
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal submit"
                                )
                            })?;
                        let data = if args.iter().any(|arg| arg == "--stdin") {
                            let mut value = String::new();
                            std::io::stdin()
                                .read_to_string(&mut value)
                                .context("reading app terminal submit stdin")?;
                            value
                        } else {
                            args.windows(2)
                                .find_map(|window| {
                                    if window[0] == "--data" {
                                        Some(window[1].as_str())
                                    } else {
                                        None
                                    }
                                })
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "missing --data or --stdin for server app terminal submit"
                                    )
                                })?
                                .to_string()
                        };
                        let ready_timeout_ms = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--ready-timeout-ms" {
                                    window[1].parse::<u64>().ok()
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(30_000);
                        run_app_control_submit_terminal_prompt(
                            session_path,
                            &data,
                            ready_timeout_ms,
                            timeout_ms,
                        )
                    }
                    "focus" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal focus"
                                )
                            })?;
                        run_app_control_reclaim_terminal_focus(session_path, timeout_ms)
                    }
                    "redraw" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal redraw"
                                )
                            })?;
                        run_app_control_redraw_terminal(session_path, timeout_ms)
                    }
                    "reconcile" | "reconcile-from-daemon" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal reconcile"
                                )
                            })?;
                        run_app_control_reconcile_terminal_from_daemon(session_path, timeout_ms)
                    }
                    "paste" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal paste"
                                )
                            })?;
                        run_app_control_paste_terminal_clipboard(session_path, timeout_ms)
                    }
                    "paste-image" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal paste-image"
                                )
                            })?;
                        run_app_control_paste_terminal_clipboard_image(session_path, timeout_ms)
                    }
                    "keep" | "keep-alive" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!("missing session path for server app terminal keep")
                            })?;
                        run_app_control_set_session_keep_alive(session_path, true, timeout_ms)
                    }
                    "unkeep" | "stop-keep-alive" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal unkeep"
                                )
                            })?;
                        run_app_control_set_session_keep_alive(session_path, false, timeout_ms)
                    }
                    "probe-type" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal probe-type"
                                )
                            })?;
                        let data = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!("missing --data for server app terminal probe-type")
                            })?;
                        let press_enter = args.iter().any(|arg| arg == "--enter");
                        let press_tab = args.iter().any(|arg| arg == "--tab");
                        let press_ctrl_c = args.iter().any(|arg| arg == "--ctrl-c");
                        let press_ctrl_e = args.iter().any(|arg| arg == "--ctrl-e");
                        let press_ctrl_u = args.iter().any(|arg| arg == "--ctrl-u");
                        let per_char = args.iter().any(|arg| arg == "--per-char");
                        let mode = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] != "--mode" {
                                    return None;
                                }
                                match window[1].as_str() {
                                    "auto" => Some(ProbeTerminalViewportInputMode::Auto),
                                    "keyboard" => Some(ProbeTerminalViewportInputMode::Keyboard),
                                    "xterm" => Some(ProbeTerminalViewportInputMode::Xterm),
                                    _ => None,
                                }
                            })
                            .unwrap_or(ProbeTerminalViewportInputMode::Auto);
                        run_app_control_probe_terminal_viewport_input(
                            session_path,
                            data,
                            mode,
                            per_char,
                            press_enter,
                            press_tab,
                            press_ctrl_c,
                            press_ctrl_e,
                            press_ctrl_u,
                            timeout_ms,
                        )
                    }
                    "probe-scroll" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal probe-scroll"
                                )
                            })?;
                        let lines = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--lines" {
                                    window[1].parse::<i32>().ok()
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing --lines for server app terminal probe-scroll"
                                )
                            })?;
                        run_app_control_probe_terminal_viewport_scroll(
                            session_path,
                            lines,
                            timeout_ms,
                        )
                    }
                    "probe-select" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal probe-select"
                                )
                            })?;
                        run_app_control_probe_terminal_viewport_select(session_path, timeout_ms)
                    }
                    "probe-primary-paste" | "probe-primary-selection-paste" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal probe-primary-paste"
                                )
                            })?;
                        let data = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing --data for server app terminal probe-primary-paste"
                                )
                            })?;
                        run_app_control_probe_terminal_primary_selection_paste(
                            session_path,
                            data,
                            timeout_ms,
                        )
                    }
                    "probe-context-menu" | "probe-right-click-menu" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal probe-context-menu"
                                )
                            })?;
                        run_app_control_probe_terminal_context_menu(session_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app terminal action: {other}"),
                }
            }
            "session" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app session"))?;
                match action {
                    "remove" | "delete" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app session remove"
                                )
                            })?;
                        run_app_control_remove_session(session_path, timeout_ms)
                    }
                    "rename" => {
                        let positionals = cli_positional_args(&args, 4);
                        let session_path = positionals.first().copied().ok_or_else(|| {
                            anyhow::anyhow!("missing session path for server app session rename")
                        })?;
                        let title = positionals.get(1).copied().ok_or_else(|| {
                            anyhow::anyhow!("missing title for server app session rename")
                        })?;
                        run_app_control_rename_session(session_path, title, timeout_ms)
                    }
                    "restart" => {
                        let session_path = cli_positional_args(&args, 4).into_iter().next().ok_or_else(|| {
                            anyhow::anyhow!("missing session path for server app session restart")
                        })?;
                        run_app_control_restart_session(session_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app session action: {other}"),
                }
            }
            other => anyhow::bail!("unsupported app control command: {other}"),
        };
    }
    if args.as_slice() == ["server", "shutdown"] {
        let endpoint = cli_server_endpoint(store.home_dir());
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("server")
        && args.get(1).map(String::as_str) == Some("update-daemons")
    {
        return run_update_all_daemons(&store, args.iter().any(|arg| arg == "--force"));
    }
    if args.as_slice() == ["server", "retire-stale-daemons"] {
        // Per [[bug-class-old-daemon-never-retires]]: yggterm-headless processes
        // from older deploys keep running because they own preserved sessions
        // (which blocks idle shutdown) and never check for newer binaries on
        // disk. This CLI scans every server-*.sock in YGGTERM_HOME and sends
        // RetireDaemon to each one whose version differs from the current
        // SERVER_PROTOCOL_VERSION.
        let report = yggterm_server::retire_stale_daemons(
            store.home_dir(),
            yggterm_server::SERVER_PROTOCOL_VERSION,
        )?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("server")
        && args.get(1).map(String::as_str) == Some("perf-summary")
    {
        // App profiling system read side: aggregate perf-telemetry.jsonl into per-span
        // p50/p95/p99/max/total, ranked by total wall-clock. The switch path is the
        // `attach`/`daemon_request` categories. Honors --category/--since-ms/--top/--json.
        let category = cli_flag_value(&args, "--category");
        let since_ms = cli_flag_value(&args, "--since-ms").and_then(|value| value.parse::<u64>().ok());
        let top = cli_flag_value(&args, "--top")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(40);
        let summaries =
            yggterm_core::summarize_perf_telemetry(store.home_dir(), since_ms, category);
        if args.iter().any(|arg| arg == "--json") {
            println!("{}", serde_json::to_string_pretty(&summaries)?);
        } else if summaries.is_empty() {
            println!(
                "(no perf-telemetry data yet — enable Performance Profiling in settings; log: {})",
                yggterm_core::perf_telemetry_path(store.home_dir()).display()
            );
        } else {
            println!(
                "{:<24} {:<30} {:>6} {:>8} {:>8} {:>8} {:>8} {:>10}",
                "category", "name", "count", "p50ms", "p95ms", "p99ms", "maxms", "totalms"
            );
            for summary in summaries.iter().take(top) {
                println!(
                    "{:<24} {:<30} {:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>10.1}",
                    summary.category,
                    summary.name,
                    summary.count,
                    summary.p50_ms,
                    summary.p95_ms,
                    summary.p99_ms,
                    summary.max_ms,
                    summary.total_ms
                );
            }
        }
        return Ok(());
    }
    if args.as_slice() == ["server", "ping"] {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        ping(&endpoint)?;
        println!("pong");
        return Ok(());
    }
    if args.as_slice() == ["server", "status"] {
        let endpoint = cli_server_endpoint(store.home_dir());
        match status(&endpoint) {
            Ok(runtime) => println!("{}", serde_json::to_string_pretty(&runtime)?),
            Err(error) => println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "running": false,
                    "error": error.to_string(),
                }))?
            ),
        }
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "server") {
        anyhow::bail!(
            "unsupported server command: {}",
            args.get(1).map(String::as_str).unwrap_or("<missing>")
        );
    }

    anyhow::bail!("this yggterm build only supports server subcommands");
}

#[cfg(test)]
mod tests {
    use super::{
        BuiltinCliCommand, builtin_cli_command_is_pure, cached_copy_hint_is_usable,
        classify_builtin_cli_command, cli_positional_args, gui_companion_executable_from_headless,
        normalize_monitor_args, preferred_headless_executable, remote_session_title_fallback,
    };
    use std::path::PathBuf;
    use yggterm_core::{InstallChannel, InstallContext, UpdatePolicy};
    use yggterm_server::RemoteScannedSession;

    #[test]
    fn classify_builtin_cli_command_detects_server_app_help_without_mutating() {
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string(), "app".to_string()]),
            Some(BuiltinCliCommand::ServerAppHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "app".to_string(),
                "screenshot".to_string(),
                "--help".to_string()
            ]),
            Some(BuiltinCliCommand::ServerAppHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "app".to_string(),
                "terminal".to_string(),
                "probe-scroll".to_string(),
                "-h".to_string()
            ]),
            Some(BuiltinCliCommand::ServerAppHelp)
        );
    }

    #[test]
    fn headless_app_control_exposes_theme_editor_actions() {
        let source = include_str!("yggterm-headless.rs");
        assert!(source.contains("\"theme-editor\" =>"));
        assert!(source.contains("run_app_control_set_theme_editor_open"));
        assert!(source.contains("run_app_control_reset_theme_editor"));
        assert!(source.contains("run_app_control_set_theme_editor_values"));
    }

    #[test]
    fn headless_app_control_exposes_maximize_command() {
        let source = include_str!("yggterm-headless.rs");
        assert!(source.contains("server app maximize <on|off|toggle>"));
        assert!(source.contains("\"maximize\" | \"maximized\" =>"));
        assert!(source.contains("run_app_control_set_maximized(enabled, timeout_ms)"));
    }

    #[test]
    fn headless_app_control_exposes_settled_open_path_command() {
        let source = include_str!("yggterm-headless.rs");
        assert!(source.contains("server app open <session-path>"));
        assert!(source.contains("\"open\" =>"));
        assert!(source.contains("run_app_control_open_path(session_path, view_mode, timeout_ms)"));
    }

    #[test]
    fn headless_app_control_routes_launch_through_gui_companion() {
        let source = include_str!("yggterm-headless.rs");
        assert!(source.contains("\"launch\" => run_app_launch_via_gui_companion"));
        assert!(source.contains("server app launch requires a yggterm GUI companion"));
        assert_eq!(
            gui_companion_executable_from_headless(&PathBuf::from("/opt/yggterm-headless")),
            Some(PathBuf::from("/opt/yggterm"))
        );
        assert_eq!(
            gui_companion_executable_from_headless(&PathBuf::from(
                "/opt/yggterm-headless-linux-x86_64"
            )),
            Some(PathBuf::from("/opt/yggterm-linux-x86_64"))
        );
    }

    #[test]
    fn classify_builtin_cli_command_detects_server_sessions_help_without_mutating() {
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string(), "sessions".to_string()]),
            Some(BuiltinCliCommand::ServerSessionsHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "sessions".to_string(),
                "regenerate-copy".to_string(),
                "--help".to_string()
            ]),
            Some(BuiltinCliCommand::ServerSessionsHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "session-copy".to_string(),
                "regenerate-copy".to_string(),
                "-h".to_string()
            ]),
            Some(BuiltinCliCommand::ServerSessionsHelp)
        );
    }

    #[test]
    fn cached_copy_hint_is_usable_rejects_empty_and_low_signal_copy() {
        assert!(!cached_copy_hint_is_usable(None));
        assert!(!cached_copy_hint_is_usable(Some("  ")));
        assert!(!cached_copy_hint_is_usable(Some("s craft:.")));
        assert!(cached_copy_hint_is_usable(Some(
            "The session repaired remote terminal restore behavior and verified the live app-control probes."
        )));
    }

    #[test]
    fn remote_title_fallback_uses_cwd_when_context_is_empty() {
        let scanned = RemoteScannedSession {
            session_path: "remote-session://dev/019dfc5a".to_string(),
            session_id: "019dfc5a-f5ca-7793-a44f-ee7f423aed38".to_string(),
            cwd: "/home/pi/gh/yggterm".to_string(),
            started_at: "2026-05-13T00:00:00Z".to_string(),
            modified_epoch: 0,
            event_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            title_hint: "019dfc5a".to_string(),
            recent_context: String::new(),
            cached_precis: None,
            cached_summary: None,
            live_runtime: false,
            storage_path: "/home/pi/.codex/sessions/session.jsonl".to_string(),
        };
        assert_eq!(
            remote_session_title_fallback(&scanned, "").as_deref(),
            Some("Yggterm Workspace")
        );
    }

    #[test]
    fn cli_positional_args_skips_flag_values() {
        let args = vec![
            "server".to_string(),
            "app".to_string(),
            "screenshot".to_string(),
            "--pid".to_string(),
            "7064".to_string(),
            "C:\\Users\\Admin\\window.png".to_string(),
            "--timeout-ms".to_string(),
            "20000".to_string(),
        ];
        assert_eq!(
            cli_positional_args(&args, 3),
            vec!["C:\\Users\\Admin\\window.png"]
        );
    }

    #[test]
    fn preferred_headless_executable_uses_active_gui_sibling() {
        let context = InstallContext {
            channel: InstallChannel::Direct,
            update_policy: UpdatePolicy::Auto,
            repo: "test/repo".to_string(),
            asset_label: "linux-x86_64".to_string(),
            current_version: "2.1.52".to_string(),
            executable_path: PathBuf::from("/direct/versions/2.1.50/yggterm-headless"),
            preferred_executable: Some(PathBuf::from("/direct/versions/2.1.52/yggterm")),
            managed_root: Some(PathBuf::from("/direct")),
            manager_hint: Some("Direct install".to_string()),
        };
        let preferred = preferred_headless_executable(&context).expect("preferred headless");
        let expected_name = if cfg!(target_os = "windows") {
            "yggterm-headless.exe"
        } else {
            "yggterm-headless"
        };
        assert_eq!(
            preferred,
            PathBuf::from("/direct/versions/2.1.52").join(expected_name)
        );
    }

    #[test]
    fn builtin_version_command_is_pure_and_must_not_handoff() {
        let command = classify_builtin_cli_command(&["--version".to_string()])
            .expect("version should be builtin");
        assert_eq!(command, BuiltinCliCommand::Version);
        assert!(builtin_cli_command_is_pure(command));

        let snapshot =
            classify_builtin_cli_command(&["server".to_string(), "snapshot".to_string()])
                .expect("snapshot should be builtin");
        assert_eq!(snapshot, BuiltinCliCommand::ServerSnapshot);
        assert!(!builtin_cli_command_is_pure(snapshot));
    }

    #[test]
    fn normalize_monitor_args_accepts_server_monitor_and_incident_aliases() {
        assert_eq!(
            normalize_monitor_args(&[
                "server".to_string(),
                "monitor".to_string(),
                "--scenario".to_string(),
                "panic-report".to_string(),
                "--jsonl-out".to_string(),
                "/tmp/incident.jsonl".to_string(),
            ]),
            Some(vec![
                "--scenario".to_string(),
                "panic-report".to_string(),
                "--jsonl-out".to_string(),
                "/tmp/incident.jsonl".to_string(),
            ])
        );
        assert_eq!(
            normalize_monitor_args(&[
                "server".to_string(),
                "latency-check".to_string(),
                "--all".to_string(),
            ]),
            Some(vec![
                "--scenario".to_string(),
                "latency-check".to_string(),
                "--all".to_string(),
            ])
        );
        assert_eq!(
            normalize_monitor_args(&[
                "panic-report".to_string(),
                "--iterations".to_string(),
                "3".to_string()
            ]),
            Some(vec![
                "--scenario".to_string(),
                "panic-report".to_string(),
                "--iterations".to_string(),
                "3".to_string(),
            ])
        );
    }
}
