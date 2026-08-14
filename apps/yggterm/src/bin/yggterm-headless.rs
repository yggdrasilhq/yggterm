use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// App-control round-trip budget for an `automation …` verb. The twin of the
/// GUI binary's constant, and the same reasoning: the expensive call behind it
/// is a session CREATE, which on a remote machine includes an ssh handshake and
/// a managed-CLI check. Timing out would record `spawn_failed` for a session
/// that then arrives anyway, leaving a row nothing in the store owns.
const AUTOMATION_APP_CONTROL_TIMEOUT_MS: u64 = 60_000;
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
    ScreenshotPostProcess, SessionKind, SshConnectTarget, control_endpoint_for_runtime_key,
    default_endpoint, detect_ghostty_host,
    ensure_local_daemon_running, fetch_remote_generation_context,
    persist_remote_generated_copy_with_options, ping, run_app_control_background_window,
    run_app_control_close_window, run_app_control_close_window_preserving_sessions,
    run_app_control_create_split_group, run_app_control_create_terminal_with_tenancy,
    run_app_control_describe_rows, run_app_control_describe_state,
    run_app_control_reorder_sessions,
    run_app_control_desktop_identity, run_app_control_dom_eval, run_app_control_drag,
    run_app_control_dump_state, run_app_control_focus_split_pane, run_app_control_focus_window,
    run_app_control_grid, run_app_control_invoke_command, run_app_control_key,
    run_app_control_list_clients, run_app_control_list_commands, run_app_control_memory_profile,
    run_app_control_move_window_by,
    run_app_control_launch_app, run_app_control_open_path, run_app_control_paste_terminal_clipboard,
    run_app_control_paste_terminal_clipboard_image, run_app_control_pointer,
    run_app_control_probe_chrome_input, run_app_control_probe_terminal_context_menu,
    run_app_control_probe_terminal_primary_selection_paste,
    run_app_control_probe_terminal_viewport_input, run_app_control_probe_terminal_viewport_scroll,
    run_app_control_probe_terminal_viewport_select, run_app_control_reclaim_terminal_focus,
    run_app_control_reconcile_terminal_from_daemon, run_app_control_redraw_terminal,
    run_app_control_remove_session, run_app_control_rename_session,
    run_app_control_reset_theme_editor, run_app_control_resize_window,
    run_app_control_restart_pending_update, run_app_control_restart_session,
    run_app_control_scroll_preview, run_app_control_scroll_right_panel,
    run_app_control_send_terminal_input, run_app_control_set_clipboard_png_base64,
    run_app_control_set_clipboard_text, run_app_control_set_force_foreground,
    run_app_control_set_fullscreen, run_app_control_set_main_zoom, run_app_control_set_maximized,
    run_app_control_app_pane_action, run_app_control_set_right_panel_mode,
    run_app_control_arrange_row_set, run_app_control_set_row_expanded,
    run_app_control_set_search, run_app_control_set_session_keep_alive,
    run_app_control_set_launch_flags, run_app_control_set_split_group_ratio,
    run_app_control_set_theme_editor_open,
    run_app_control_set_theme_editor_values, run_app_control_set_tree_selection,
    run_app_control_set_window_chrome_hover, run_app_control_show_start_page,
    run_app_control_split_web_tab, run_app_control_start_action,
    run_app_control_check_terminal_input, run_app_control_submit_terminal_prompt, run_app_control_trigger_update_check,
    run_app_control_ungroup_split_group, run_attach, run_daemon, run_screenrecord_capture,
    run_screenshot_capture, run_screenshot_capture_with_post_process, run_trace_bundle,
    run_trace_follow, run_trace_tail, run_trace_transitions,
    scan_remote_machine_sessions_for_target, shutdown, snapshot, status, terminal_resize,
    terminal_restart, terminal_write, try_run_remote_server_command,
};

#[path = "../build_identity.rs"]
mod build_identity;
#[path = "../headless_monitor.rs"]
mod headless_monitor;

const ENV_YGGTERM_DIRECT_INSTALL_ROOT: &str = "YGGTERM_DIRECT_INSTALL_ROOT";
const ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF: &str = "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinCliCommand {
    MainHelp,
    Version,
    /// ⛔ NOT A SYNONYM FOR `Version` — see `build_identity`. Two clusters can
    /// spend the same version number in the same minute; only the commit says
    /// which source is in front of you.
    BuildCommit,
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
            | BuiltinCliCommand::BuildCommit
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
        [arg] if matches!(arg.as_str(), "--build-commit" | "build-commit") => {
            Some(BuiltinCliCommand::BuildCommit)
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
  yggterm-headless --build-commit
  yggterm-headless server <subcommand>

common server commands:
  yggterm-headless server daemon
  yggterm-headless server status
  yggterm-headless server daemons [--json]
  yggterm-headless server snapshot
  yggterm-headless server monitor --scenario panic-report
  yggterm-headless server monitor --scenario latency-check --all
  yggterm-headless server app <subcommand>
  yggterm-headless collection <list|show|new|add|add-from-history|move|rename|tag|note|promote|open|export|prune>
  yggterm-headless snapshot now [--profile <p>] (--url <u> [--title <t>])...
    `collection --help` prints the whole plane"
    );
}

fn print_server_help() {
    // The verb list is READ from the client module rather than spelled here:
    // the agent owns the vocabulary, and a hand-copied list in help text is a
    // second encoding that drifts silently the first time a verb is added.
    let wpe_verbs = yggterm_server::wpe_agent::KNOWN_VERBS.join(", ");
    println!(
        "usage:
  yggterm-headless server daemon
  yggterm-headless server attach <session> [cwd] [--allow-plain-shell-fallback]
  yggterm-headless server ping
  yggterm-headless server status
  yggterm-headless server daemons [--json]
  yggterm-headless server relay-boundary [--by <who>] [--wait-secs <n>] [--json]
  yggterm-headless server gate-screen [<session-key>] [--tail <n>] [--json]
    what the hot-restart idle gate is CLASSIFYING FROM, per owned session — the
    live in-daemon screen plus the blocker it produced. This is not
    `server snapshot`'s terminal_lines, which is usually a stored summary line
    rather than screen text. Read-only, on demand, never written to the trace.
  yggterm-headless server <status|snapshot> --endpoint <socket-path|version|pid>
  yggterm-headless server snapshot
  yggterm-headless server shutdown
  yggterm-headless server terminal write <session> (--data <data>|--stdin) [--refuse-if-draft]
  yggterm-headless server terminal restart <session> [--terminal-appearance <dark|light>] [--force-remote]
  yggterm-headless server terminal tenants [<session>]
  yggterm-headless server wpe <verb> [--key value ...]
    verbs: {wpe_verbs}
    (the agent owns this list; an unknown verb is refused by the agent itself)
  yggterm-headless server wpe agent <status|restart|stop>
  yggterm-headless server sessions regenerate-copy [--budget <n>] [--force] [--reset-summary-history] [--skip-local] [--skip-remote] [--json]
  yggterm-headless server monitor --scenario <panic-report|server-list|latency-check|wait-session|hot-restart|managed-cli-refresh>
  yggterm-headless server perf-summary [--category <c>] [--since-ms <ms>] [--top <n>] [--json]
  yggterm-headless server perf-incidents [--since-ms <ms>] [--top <n>] [--list] [--json]
  yggterm-headless server render-top [--pid <pid>] [--client <name>] [--interval-ms <ms>] [--top <n>] [--json]
  yggterm-headless server trace <tail|follow|bundle|transitions>
  yggterm-headless server screenshot <target> [output]
  yggterm-headless server screenrecord <target> [output]
  yggterm-headless server app <subcommand>"
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

/// THE positional-argument rule, shared with the `yggterm` binary and with the
/// `server app web` dispatcher that reads the same argv — one implementation,
/// so a `--flag value` pair cannot be skipped on one entry point and read as a
/// positional on another. See [`yggterm_core::cli_args`].
fn cli_positional_args(args: &[String], start: usize) -> Vec<&str> {
    yggterm_core::cli_positional_args(args, start)
}

/// THE argv flag rule, shared with the `yggterm` binary and with the
/// server-side parsers that read the same argv — one implementation, so
/// `--flag=value` cannot be honoured on one entry point and silently discarded
/// on another. See [`yggterm_core::cli_args`].
fn cli_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    yggterm_core::cli_flag_value(args, flag)
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
    // `--grid [COLSxROWS]` / `--grid-refine CELL`: the agent-only click grid,
    // composited into the RETURNED IMAGE only — the live page never sees it.
    let grid = yggterm_server::grid_overlay::screenshot_grid_from_args(args);
    if region.is_none() && crop.is_none() && scale.is_none() && grid.is_none() {
        return None;
    }
    Some(ScreenshotPostProcess {
        region,
        crop,
        scale: scale.unwrap_or(1.0),
        grid,
    })
}

/// The daemon this CLI invocation talks to. See the twin in `main.rs` — never
/// `default_endpoint` (our own version's socket), or a headless binary newer
/// than the running daemon spawns a rival that cold-restores `server-state.json`
/// and resurrects closed sessions.
/// `server wpe <verb> [--key value ...]` and `server wpe agent <action>`.
///
/// A thin proxy on purpose. Everything after the verb is passed to the agent as
/// it was typed (numbers coerced only for the keys the protocol declares
/// numeric), and the answer is printed verbatim. The CLI is not a second place
/// where the verb vocabulary lives.
///
/// **The exit code is part of the contract**: a typed failure prints its JSON
/// and exits non-zero, so an agent scripting this can branch on `$?` rather
/// than re-parsing the outcome it just received.
fn run_server_wpe(store: &SessionStore, args: &[String]) -> Result<()> {
    use yggterm_server::wpe_agent::{WpeOutcome, params_from_flags};

    // The plane lives INSIDE the daemon (it owns the agent process), so unlike
    // the read-only diagnostics this needs a daemon to exist.
    ensure_local_server_ready_for_cli(store)?;
    let endpoint = cli_server_endpoint(store.home_dir());

    if args[0] == "agent" {
        let action = args
            .get(1)
            .map(String::as_str)
            .context("usage: server wpe agent <status|restart|stop>")?;
        return match yggterm_server::wpe_agent_control(&endpoint, action)? {
            Ok(report) => {
                println!("{}", serde_json::to_string_pretty(&report)?);
                Ok(())
            }
            Err(outcome) => {
                print_wpe_failure("agent", &outcome)?;
                std::process::exit(1);
            }
        };
    }

    let verb = args[0].as_str();
    let params = params_from_flags(&args[1..]).map_err(|message| anyhow::anyhow!(message))?;
    match yggterm_server::wpe_verb(&endpoint, verb, params)? {
        WpeOutcome::Answer { response } => {
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        outcome => {
            print_wpe_failure(verb, &outcome)?;
            std::process::exit(1);
        }
    }
}

/// One printer for every failure arm, so the shape a script parses does not
/// depend on which way the plane failed.
fn print_wpe_failure(verb: &str, outcome: &yggterm_server::wpe_agent::WpeOutcome) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": false,
            "verb": verb,
            "summary": outcome.summary(),
            "failure": outcome,
        }))?
    );
    Ok(())
}

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
            Some(if force {
                "forced_update_all"
            } else {
                "update_all"
            }),
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
            apps: Vec::new(),
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

    // Routing only: the persist path never reads the session list, and hoisting
    // it out of the loop keeps it from copying the machine per session.
    let machine_ref = machine.routing_ref();
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
            &machine_ref,
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

/// Which GUI `server app launch` should start.
///
/// ⛔ **The install state's recorded executable is preferred ONLY when it is not
/// a downgrade** — the same rule as the exec handoff, and it belongs here for
/// the same reason. On the live host `preferred_executable` was
/// `~/.yggterm/versions/2.11.0/yggterm`, and it EXISTS, so an agent relaunching
/// the GUI through app control would have started a 2.11.0 window against a
/// 3.0.44 daemon — the loudest possible version skew, arrived at silently.
/// Found 2026-08-07 while reaching for this verb to prove another fix.
///
/// When the record is a downgrade we fall through to the companion sitting
/// beside THIS binary, which is by construction the build the caller deployed.
fn preferred_gui_executable_from_headless(
    current_exe: &Path,
    install_context: &InstallContext,
) -> Option<std::path::PathBuf> {
    install_context
        .preferred_executable
        .clone()
        .filter(|path| path.is_file())
        .filter(|recorded| {
            yggterm_core::handoff_target_is_usable(
                env!("CARGO_PKG_VERSION"),
                &install_context.current_version,
                recorded,
            )
        })
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
    // An agent-launched GUI must take the SAME GL path a desktop-launched one takes.
    // This process inherits its parent's environment and hands it straight to the GUI,
    // so a stale v3 launcher still on disk — or an operator's shell — could smuggle in
    // WEBKIT_DISABLE_COMPOSITING_MODE and make `server app launch` land on software GL
    // while the desktop entry probes its way to hardware. That is the same class as
    // the inherited-canvas-flag bug (`linux_canvas_env_is_user_explicit`), where an
    // agent-launched GUI was locked to the DOM renderer for months.
    command.env_remove("WEBKIT_DISABLE_COMPOSITING_MODE");
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

/// Commands that read LOCAL state in-process and never talk to the daemon, so
/// they must run in THIS (newest) binary.
///
/// Handing one off to a stale active-executable — a dev deploy that overwrote
/// `~/.local/bin` but not `install-state.json` — runs a binary that predates
/// the command, which is precisely how `perf-incidents` answered "unsupported
/// server command" on the very host whose incidents you were reading. It was a
/// bare `matches!` inline in the handoff with no test, so `render-top` would
/// have re-learned the lesson the same way.
fn command_reads_local_state_in_process(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("server"))
        && matches!(
            args.get(1).map(String::as_str),
            Some("perf-summary") | Some("perf-incidents") | Some("render-top")
        )
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
    if command_reads_local_state_in_process(args) {
        return Ok(());
    }
    let Some(preferred) = preferred_headless_executable(install_context) else {
        return Ok(());
    };
    // ⛔ Never hand a NEWER binary down to an older install. The live host ran
    // 3.0.43 with a record naming 2.11.0, so every verb was exec'd sixteen
    // minors backwards and the flags that postdate 2.11.0 were silently dropped
    // — and the record then bumped its VERSION without moving its PATH, which
    // defeats a version-only check. `handoff_target_is_usable` reads the target
    // path too, because a path cannot be bumped without a move.
    if !yggterm_core::handoff_target_is_usable(
        env!("CARGO_PKG_VERSION"),
        &install_context.current_version,
        &preferred,
    ) {
        return Ok(());
    }
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
    // ⭐ FIRST, before the logger and before any thread: see
    // `yggterm_core::session_bus`. This binary spawns daemons, shadow clients and
    // web surfaces, and a GLib autolaunch in any of them leaks a session bus plus
    // its activated helper daemons permanently — 4,574 MB of them were measured on
    // the live host. Children inherit whatever we resolve here.
    let _session_bus = yggterm_core::session_bus::adopt_or_refuse_session_bus();

    // This process becomes a DAEMON that outlives the file it was loaded from,
    // so it publishes the source it was built from while it still can. See
    // `yggterm_server::build_identity` for why nothing outside can recover it.
    //
    // ⚠ AFTER the bus resolve, not before — same reason as the GUI binary's:
    // "first statement in main()" is the whole guarantee the lock enforces, and
    // nothing here needs the identity declared first.
    yggterm_server::build_identity::declare_build_commit(build_identity::build_commit());

    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // Agent presence (cursor v1): stamp every app-control request this
    // invocation sends with who is driving, so the window can show the user an
    // `agent-N` pointer. One resolve for the whole process — an invocation is
    // one agent — instead of an --agent parameter on every verb.
    yggterm_server::set_agent_identity(cli_flag_value(&args, "--agent"));
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
            BuiltinCliCommand::BuildCommit => {
                println!("{}", build_identity::build_commit());
                return Ok(());
            }
            BuiltinCliCommand::ServerHelp => {
                print_server_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerAppHelp => {
                yggterm_server::app_control_cli::print_server_app_help("yggterm-headless");
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
    // ⛔ The forced command behind a phone's ssh key. This is the binary the
    // phone actually invokes (`~/.yggterm/bin/yggterm-headless`), so the arm has
    // to exist HERE — the twin in apps/yggterm/src/main.rs serves the GUI binary.
    // See `yggterm_server::daemon_bridge` for why a bare key is a supply-chain risk.
    if args.as_slice() == ["server", "daemon-bridge"] {
        return yggterm_server::daemon_bridge::run_daemon_bridge();
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "attach" {
        let (cwd, fallback) = yggterm_server::parse_attach_args(&args[3..]);
        return run_attach(&args[2], cwd.as_deref(), fallback);
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
        // ⛔ `"accepted": true` used to be a LITERAL here — printed whatever the
        // daemon replied, so the field could never report anything but success.
        // It is now the daemon's own answer, which is what makes
        // `--refuse-if-draft` observable at all: a guard whose refusal prints as
        // `accepted:true` is not a guard.
        // [[finding-a-constant-anomaly-is-a-measurement-bug]]
        let refuse_if_draft = args.iter().any(|arg| arg == "--refuse-if-draft");
        let message =
            yggterm_server::terminal_write_guarded(&endpoint, &args[3], &data, refuse_if_draft)?;
        let refused = yggterm_server::terminal_write_was_refused_for_draft(message.as_deref());
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "accepted": !refused,
                "refused_for_draft": refused,
                "session_path": args[3],
                "bytes": data.len(),
                "message": message,
            }))?
        );
        return Ok(());
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "gate-screen" {
        // §3's audit instrument. Read-only, on demand, and connected directly
        // like the other read-only diagnostics — a verb that spawned a daemon
        // in order to ask what a daemon is looking at would answer about a
        // process that did not exist when the question was asked.
        //
        // ⛔ NOT WRITTEN ANYWHERE. The screens go to this stdout and nowhere
        // else — see `HotRestartGateScreen`. A caller harvesting a corpus owns
        // where it lands and how long it lives.
        let endpoint = cli_server_endpoint(store.home_dir());
        let path = args.get(2).filter(|arg| !arg.starts_with("--"));
        let tail_lines = cli_flag_value(&args, "--tail").and_then(|value| value.parse().ok());
        let sessions = yggterm_server::hot_restart_gate_screens(
            &endpoint,
            path.map(String::as_str),
            tail_lines,
        )?;
        if args.iter().any(|arg| arg == "--json") {
            println!("{}", serde_json::to_string_pretty(&sessions)?);
            return Ok(());
        }
        if sessions.is_empty() {
            println!("no sessions owned by this daemon match");
            return Ok(());
        }
        for session in &sessions {
            let verdict = match session.blocker.as_ref() {
                Some(blocker) => format!(
                    "{kind}{permanent}, idle {idle}",
                    kind = blocker.kind,
                    permanent = if blocker.permanent { " (permanent)" } else { "" },
                    idle = blocker
                        .idle_ms
                        .map(|ms| format!("{}s", ms / 1000))
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
                None => "not blocking".to_string(),
            };
            println!(
                "== {key}\n   gate verdict: {verdict}\n   screen_text_shows_agent_working: {working}\n   screen: {screen}",
                key = session.session_key,
                working = session.shows_agent_working,
                screen = if session.screen_available {
                    "readable"
                } else {
                    "UNREADABLE — the gate is classifying this one blind"
                },
            );
            for line in session.screen_tail.iter().flatten() {
                println!("   | {line}");
            }
        }
        return Ok(());
    }
    if args.len() >= 4 && args[0] == "server" && args[1] == "terminal" && args[2] == "app-declares"
    {
        // Read-only: what the daemon retained off this session's OSC 7717
        // channel (the app's latest web-surface / sidebar payload). This is
        // what `web ensure` rebuilds a never-revealed or reaped surface from,
        // so it is also the first thing to look at when a rebuild answers
        // "no declared web surface". Connect directly like the other read-only
        // diagnostics — no version gate, no daemon spawn.
        let endpoint = cli_server_endpoint(store.home_dir());
        let (records, running) = yggterm_server::terminal_app_declares(&endpoint, &args[3])?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "session_path": args[3],
                "running": running,
                "declare_count": records.len(),
                "declares": records,
            }))?
        );
        return Ok(());
    }
    // Automations — scheduled agent-CLI sessions. THIS is what a generated
    // systemd timer's ExecStart invokes, so it is deliberately matched before
    // anything that could need a daemon handshake: a timer firing at midnight
    // must reach the executor without first negotiating a version.
    // Accepted BOTH as `automation …` (what the unit writes) and
    // `server automation …` (what fits the rest of this CLI's shape).
    if args.first().is_some_and(|arg| arg == "automation") {
        return yggterm_server::run_automation_cli(&args, AUTOMATION_APP_CONTROL_TIMEOUT_MS);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "automation" {
        return yggterm_server::run_automation_cli(&args[1..], AUTOMATION_APP_CONTROL_TIMEOUT_MS);
    }
    // Collections — history organised into things worth keeping. ONE owner
    // (crates/yggterm-server/src/web_collection_cli.rs), both binaries, exactly
    // as `automation` above. No daemon handshake: a collection is a Markdown
    // file in the profile's own jar, and `collection list` must answer on a
    // machine with no GUI and no daemon at all.
    // `snapshot` is matched at the TOP level only — `server snapshot` is the
    // daemon screen dump and must keep meaning that.
    if args.first().is_some_and(|arg| arg == "collection" || arg == "snapshot") {
        return yggterm_server::run_web_collection_cli(&args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "collection" {
        return yggterm_server::run_web_collection_cli(&args[1..]);
    }
    // Browser import (history + bookmarks out of Chromium/Firefox profiles).
    // Matched here for the same reason automations are: it is local file work
    // and must not have to negotiate a daemon version to run.
    if args.first().is_some_and(|arg| arg == "web-import") {
        return yggterm_server::run_browser_import_cli(&args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "web-import" {
        return yggterm_server::run_browser_import_cli(&args[1..]);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "terminal" && args[2] == "sanity" {
        // THE TABLE, in the words a person uses about it: what is on it, what
        // is squatting, and what may go. Read-only unless --apply is passed —
        // a sweep that acts by default is a sweep nobody can safely run once to
        // see what it would do.
        let endpoint = cli_server_endpoint(store.home_dir());
        let (rows, degraded) = yggterm_server::terminal_tenants(&endpoint, None)?;
        let apply = args.iter().any(|arg| arg == "--apply");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as u64)
            .unwrap_or(0);
        let records = yggterm_server::load_sweep_records(store.home_dir());
        // `degraded` is a REASON, not a flag — its presence is the degradation.
        let is_degraded = degraded.is_some();
        let (decisions, next_records) =
            yggterm_server::row_sanity::plan_sweep(&rows, &records, now_ms, is_degraded);
        yggterm_server::print_row_sanity_report(&rows, &decisions, is_degraded, apply);
        if apply {
            // ⛔ SECOND LINE OF DEFENCE. The classifier refuses a row whose work
            // runs on another host, but that fix is in the DAEMON and an older
            // one answers for its own rows without the field. A single layer
            // between --apply and a live agent session is not enough: on
            // 2026-08-06 this system offered to close a cogs delegate that
            // was five hours into its task.
            let unvouched = yggterm_server::row_sanity::unvouched_rows(&rows, &decisions);
            if !unvouched.is_empty() {
                eprintln!(
                    "\nREFUSING --apply: {} row(s) would be acted on that this daemon \
                     cannot vouch for as local work.",
                    unvouched.len()
                );
                for path in unvouched.iter().take(10) {
                    eprintln!("  {path}");
                }
                eprintln!(
                    "A row whose agent runs on another host looks empty from here — that is \
                     an ssh bridge, not an idle plate. Run the sweep ON that host."
                );
                anyhow::bail!("row sanity refused to act on rows it cannot vouch for");
            }
            yggterm_server::save_sweep_records(store.home_dir(), &next_records);
        }
        return Ok(());
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "terminal" && args[2] == "tenants" {
        // Per-row tenant accounting (docs/pending-bugs.md, the immortal tenant
        // class). Read-only and ON DEMAND — nothing polls, so asking costs one
        // /proc reading and idling costs nothing. Connect directly like the
        // other read-only diagnostics: no version gate, no daemon spawn.
        let endpoint = cli_server_endpoint(store.home_dir());
        let session_path = args.get(3).map(String::as_str).filter(|value| {
            !value.starts_with("--")
        });
        let (rows, degraded) = yggterm_server::terminal_tenants(&endpoint, session_path)?;
        let measured = rows
            .iter()
            .filter(|row| row.unavailable_reason.is_none())
            .count();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "session_path": session_path,
                "row_count": rows.len(),
                "measured_rows": measured,
                "unmeasured_rows": rows.len() - measured,
                "degraded": degraded,
                "rows": rows,
            }))?
        );
        return Ok(());
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "wpe" {
        return run_server_wpe(&store, &args[2..]);
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
        // No `ensure_local_server_ready_for_cli` here, deliberately, and the
        // sibling verb in `yggterm` never had one: SPAWNING a daemon cannot help
        // a resize. Either some daemon already holds this PTY — in which case we
        // want THAT one, not a fresh peer — or no PTY exists and the honest
        // answer is a failure. The gate also actively broke the op: it insists
        // the reachable daemon be the CURRENT build, so a binary built from a
        // different tree than the running daemon died with "local yggterm daemon
        // did not become reachable" before ever looking for the owner.
        // Address the daemon that OWNS this runtime key, not the one this
        // binary's version would spawn. On a host running version-coexisting
        // daemons those are different processes, and the owner is routinely the
        // OLDER one (the constitution keeps it alive while its sessions work).
        // Resolving by version answered `terminal session not found` for a live
        // remote CC session on `dev` while the identical call succeeded on `oc`
        // purely because there the deployed binary's daemon happened to be the
        // owner — see `owning_daemon_endpoint_for_runtime_key`.
        let endpoint = control_endpoint_for_runtime_key(store.home_dir(), &args[3]);
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
            let _ = terminal_resize(
                &endpoint,
                &args[3],
                cols.saturating_sub(1).max(1),
                rows.saturating_sub(1).max(1),
            );
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
                "owner_endpoint": format!("{endpoint:?}"),
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
        let (snapshot, message) = yggterm_server::reorder_live_sessions(&endpoint, &ordered_paths)?;
        // The daemon keeps only the rows it actually has, so report what the order
        // BECAME rather than echoing the request back as if it succeeded — and
        // take `applied`/`skipped` from the DAEMON's own answer rather than
        // re-deriving them here, so this surface and `yggterm server reorder`
        // cannot disagree about what happened.
        let resulting_order: Vec<&str> = snapshot
            .live_sessions
            .iter()
            .map(|session| session.session_path.as_str())
            .collect();
        let requested: Vec<&str> = ordered_paths.iter().map(String::as_str).collect();
        let update = message
            .as_deref()
            .and_then(yggterm_server::LiveSessionOrderUpdate::from_message);
        let mut report = serde_json::json!({
            "requested": requested.len(),
            "live_rows": resulting_order.len(),
            "matches_request": resulting_order == requested,
            "applied_order": resulting_order,
        });
        match &update {
            Some(update) => {
                report["applied"] = serde_json::json!(update.applied);
                report["skipped"] = serde_json::json!(update.skipped);
                report["message"] = serde_json::json!(update.summary());
            }
            // An older daemon cannot say what it applied; say so instead of
            // guessing.
            None => {
                report["applied_unreported_by_daemon"] = serde_json::json!(true);
                report["message"] = serde_json::json!(message);
            }
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        if let Some(update) = update
            && !update.skipped.is_empty()
        {
            anyhow::bail!("{}", update.summary());
        }
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
            .find_map(|window| (window[0] == "--last-ms").then(|| window[1].parse::<u64>().ok())?)
            .unwrap_or(180_000);
        let limit = args
            .windows(2)
            .find_map(|window| (window[0] == "--limit").then(|| window[1].parse::<usize>().ok())?)
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
                    grid: None,
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
            BuiltinCliCommand::BuildCommit => {
                println!("{}", build_identity::build_commit());
                return Ok(());
            }
            BuiltinCliCommand::ServerHelp => {
                print_server_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerAppHelp => {
                yggterm_server::app_control_cli::print_server_app_help("yggterm-headless");
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
        // ONE OWNER for the whole `server app` surface — see
        // `yggterm_server::app_control_cli`. This used to be a 1,536-line
        // `match` that the GUI binary carried its own copy of, and the two had
        // already drifted by six verbs. Do not inline a verb here.
        struct HeadlessHost<'a> {
            current_exe: &'a std::path::Path,
            install_context: &'a InstallContext,
        }
        impl yggterm_server::app_control_cli::AppControlHost for HeadlessHost<'_> {
            fn binary_name(&self) -> &'static str {
                "yggterm-headless"
            }
            // The one genuine fork: no GUI here, so the launch is asked of a
            // companion rather than spawned in-process.
            fn launch_app(
                &self,
                args: &[String],
                _home_dir: &std::path::Path,
                _timeout_ms: u64,
            ) -> anyhow::Result<()> {
                run_app_launch_via_gui_companion(self.current_exe, args, self.install_context)
            }
        }
        let host = HeadlessHost {
            current_exe: &current_exe,
            install_context: &install_context,
        };
        return yggterm_server::app_control_cli::run_app_control_cli(
            &args,
            store.home_dir(),
            &host,
        );
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
        let since_ms =
            cli_flag_value(&args, "--since-ms").and_then(|value| value.parse::<u64>().ok());
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
            // The `clock` column is not decoration: a `render` row's milliseconds are
            // CPU time consumed, not elapsed time, so reading its totalms as wall
            // duration overstates it by however many cores were busy.
            // The `pids` column answers "which process burned this" — three
            // daemons and a GUI append to ONE perf-telemetry.jsonl per home,
            // and until 2026-07-26 the records could not say. Empty means the
            // rows predate the pid stamp.
            println!(
                "{:<24} {:<30} {:>5} {:>6} {:>8} {:>8} {:>8} {:>8} {:>10}  {}",
                "category",
                "name",
                "clock",
                "count",
                "p50ms",
                "p95ms",
                "p99ms",
                "maxms",
                "totalms",
                "pids"
            );
            for summary in summaries.iter().take(top) {
                println!(
                    "{:<24} {:<30} {:>5} {:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>10.1}  {}",
                    summary.category,
                    summary.name,
                    summary.time_base().as_str(),
                    summary.count,
                    summary.p50_ms,
                    summary.p95_ms,
                    summary.p99_ms,
                    summary.max_ms,
                    summary.total_ms,
                    summary
                        .pids
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<String>>()
                        .join(",")
                );
            }
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("server")
        && args.get(1).map(String::as_str) == Some("perf-incidents")
    {
        // The read side of the DURABLE half of profiling. `perf-summary`
        // aggregates the rolling telemetry; this reads the snapshots the daemon
        // kept when the app actually went hot — the ones still there days later
        // when the user reports "the fan flared this morning". The writer has
        // been live all along and only the reader was missing, which is how 183
        // records sat unread on the live host. Ranked by COUNT: the driver worth
        // fixing is the one that keeps happening.
        let since_ms =
            cli_flag_value(&args, "--since-ms").and_then(|value| value.parse::<u64>().ok());
        let top = cli_flag_value(&args, "--top")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(20);
        let list = args.iter().any(|arg| arg == "--list");
        let json = args.iter().any(|arg| arg == "--json");
        if list || json {
            let records = yggterm_core::read_perf_incidents(store.home_dir(), since_ms);
            let tail = records
                .iter()
                .rev()
                .take(top)
                .cloned()
                .collect::<Vec<serde_json::Value>>();
            println!("{}", serde_json::to_string_pretty(&tail)?);
            return Ok(());
        }
        let summaries = yggterm_core::summarize_perf_incidents(store.home_dir(), since_ms);
        if summaries.is_empty() {
            println!(
                "(no perf incidents recorded — they are written only while Performance \
                 Profiling is on and a window actually went hot; log: {})",
                store
                    .home_dir()
                    .join(yggterm_core::PERF_INCIDENT_FILENAME)
                    .display()
            );
            return Ok(());
        }
        let total: usize = summaries.iter().map(|summary| summary.count).sum();
        println!("{total} incidents recorded");
        println!(
            "{:<22} {:<40} {:>6} {:>12} {:>20}",
            "trigger", "span", "count", "worst_ms", "last"
        );
        for summary in summaries.iter().take(top) {
            println!(
                "{:<22} {:<40} {:>6} {:>12.0} {:>20}",
                summary.trigger_kind,
                if summary.span.is_empty() {
                    "-"
                } else {
                    summary.span.as_str()
                },
                summary.count,
                summary.worst_total_ms,
                summary.last_ts_ms
            );
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("server")
        && args.get(1).map(String::as_str) == Some("render-top")
    {
        // The read side of the render probe. Every number is a DELTA over the
        // interval, which is the whole point: `ps %CPU` is a lifetime average,
        // and reading it as current load is what produced the phantom "105% of
        // a core" the optimization pass started from.
        //
        // Reads /proc only — no daemon round-trip, hence no
        // `ensure_local_server_ready_for_cli`.
        let interval_ms = cli_flag_value(&args, "--interval-ms")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(5_000);
        let top = cli_flag_value(&args, "--top")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10);
        let json = args.iter().any(|arg| arg == "--json");
        // `--pid` here names any process-tree root, deliberately unlike
        // `server app --pid` where it must name a REGISTERED client. Only the
        // untargeted default goes through the client registry.
        let requested_pid =
            cli_flag_value(&args, "--pid").and_then(|value| value.parse::<u32>().ok());
        let requested_client = cli_flag_value(&args, "--client");
        let root_pid = match requested_pid {
            Some(pid) => Some(pid),
            None => {
                yggterm_server::choose_registered_gui_pid(store.home_dir(), None, requested_client)?
            }
        };
        let Some(root_pid) = root_pid else {
            bail!(
                "no registered yggterm GUI to measure — pass --pid <pid> to name a \
                 process tree, or --client <name> to pick one"
            );
        };
        let Some(report) =
            yggterm_core::render_probe::render_top_sample(root_pid as i32, interval_ms, top)
        else {
            bail!("no such process tree: {root_pid}");
        };
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        println!(
            "render-top: root={} processes={} interval={:.0}ms user_hz={}",
            report.root_pid, report.process_count, report.interval_ms, report.user_hz
        );
        // `gpu_ms` is the whole point of reading this on the GUI host: a role
        // burning CPU with NO GPU time is rasterizing in software, which is the
        // defect §1a exists for. A dash means the counters were unreadable —
        // never a zero, because "we could not look" and "it did no work" are
        // the two answers that must not be confused here.
        println!(
            "{:<14} {:>6} {:>10} {:>8} {:>10} {:>12} {:>10}",
            "role", "procs", "cpu_ms", "cores", "gpu_ms", "mem_mb", "hot_pid"
        );
        for role in &report.roles {
            println!(
                "{:<14} {:>6} {:>10.1} {:>8.3} {:>10} {:>12.1} {:>10}",
                role.role,
                role.procs,
                role.cpu_ms,
                role.core_fraction,
                role.gpu_ms
                    .map(|ms| format!("{ms:.1}"))
                    .unwrap_or_else(|| "-".to_string()),
                role.mem_kb as f64 / 1024.0,
                role.hot_pid
            );
        }
        println!(
            "{:<14} {:>6} {:>10.1} {:>8.3} {:>10} {:>12.1} {:>10}",
            "TOTAL",
            report.process_count,
            report.total_cpu_ms,
            report.total_core_fraction,
            report
                .total_gpu_ms
                .map(|ms| format!("{ms:.1}"))
                .unwrap_or_else(|| "-".to_string()),
            report.total_mem_kb as f64 / 1024.0,
            ""
        );
        println!("\ntop processes by cpu_ms:");
        for sample in &report.top_processes {
            println!(
                "  pid={:<8} {:<16} {:<12} cpu_ms={:>9.1} cores={:>6.3} mem_mb={:>8.1}",
                sample.pid,
                sample.comm,
                sample.role,
                sample.cpu_ms,
                sample.core_fraction,
                sample.mem_kb as f64 / 1024.0
            );
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
    if args.first().is_some_and(|arg| arg == "server")
        && args.get(1).is_some_and(|arg| arg == "daemons")
    {
        // ONE owner, both binaries — see `daemon::run_server_daemons_census`.
        // The census is a host fact with no GUI in it, and it used to answer
        // here only.
        return yggterm_server::run_server_daemons_census(
            store.home_dir(),
            args.iter().any(|arg| arg == "--json"),
        );
    }
    if args.first().is_some_and(|arg| arg == "server")
        && args.get(1).is_some_and(|arg| arg == "relay-boundary")
    {
        // §2 of docs/spec-hot-restart-relay-gate.md — *"a relay hand-off is a
        // genuine, declared, zero-cost quiet point … the gate stops being a
        // search and becomes an appointment."*
        //
        // ⛔ It does NOT spawn a daemon (no `ensure_local_server_ready_for_cli`)
        // and it does not talk to one. The queue is a HOST fact in a file, and
        // making the verb reach a daemon would mean choosing which of the
        // several a stale host is running — the exact question §4 moved out of
        // any one daemon's status. A drainer picks the boundary up on its next
        // 20 s poll.
        let json = args.iter().any(|arg| arg == "--json");
        let declared_by = args
            .iter()
            .position(|arg| arg == "--by")
            .and_then(|index| args.get(index + 1))
            .cloned()
            .unwrap_or_else(|| "relay_boundary".to_string());
        let wait_secs = args
            .iter()
            .position(|arg| arg == "--wait-secs")
            .and_then(|index| args.get(index + 1))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        let outcome = yggterm_server::hot_restart_queue::declare_relay_boundary(
            store.home_dir(),
            now_ms,
            &declared_by,
        );
        let (owed, target_version, waiting_ms) = match &outcome {
            yggterm_server::hot_restart_queue::RelayBoundaryOutcome::Declared {
                target_version,
                waiting_ms,
            } => (true, Some(target_version.clone()), Some(*waiting_ms)),
            yggterm_server::hot_restart_queue::RelayBoundaryOutcome::NothingOwed => {
                (false, None, None)
            }
        };
        // ⚠ The drainer polls every 20 s, so a wait shorter than that can only
        // ever time out — say so rather than reporting a converged host as
        // still-owing. Waiting is opt-in because the common case is a converged
        // host with nothing to wait for.
        let mut converged = !owed;
        if owed && wait_secs > 0 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if yggterm_server::hot_restart_queue::load(store.home_dir()).is_none() {
                    converged = true;
                    break;
                }
            }
        }
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "declared_by": declared_by,
                    "swap_owed": owed,
                    "target_version": target_version,
                    "waiting_ms": waiting_ms,
                    "waited_for_secs": wait_secs,
                    "converged": converged,
                }))?
            );
        } else if let Some(target_version) = target_version {
            let waiting_min = waiting_ms.unwrap_or(0) / 60_000;
            if converged {
                println!(
                    "relay boundary declared by {declared_by}; swap to {target_version} converged"
                );
            } else {
                println!(
                    "relay boundary declared by {declared_by}; swap to {target_version} \
                     (owed {waiting_min}m) is due at the next drainer poll"
                );
            }
        } else {
            println!("relay boundary declared by {declared_by}; no swap is owed on this host");
        }
        return Ok(());
    }
    // `--endpoint <path|version|pid>` aims a READ-ONLY verb at one of the
    // daemons the census names. Read-only on purpose: seeing all 28 and being
    // able to ask any of them is the gap; being able to MUTATE an arbitrary one
    // by hand is a footgun, and `server update-daemons` already owns the
    // sanctioned way to act on the whole set.
    if let Some(index) = args.iter().position(|arg| arg == "--endpoint") {
        let selector = args.get(index + 1).cloned().unwrap_or_default();
        let rest = args
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != index && *position != index + 1)
            .map(|(_, arg)| arg.clone())
            .collect::<Vec<_>>();
        if !matches!(
            rest.as_slice(),
            [command, verb] if command == "server" && (verb == "status" || verb == "snapshot")
        ) {
            bail!(
                "--endpoint applies to the read-only verbs only (server status, server snapshot); \
                 got: {}",
                rest.join(" ")
            );
        }
        let (endpoint, kind) =
            yggterm_server::resolve_daemon_endpoint_selector(store.home_dir(), &selector)?;
        let runtime = status(&endpoint)?;
        let mut value = serde_json::to_value(&runtime)?;
        if let Some(object) = value.as_object_mut() {
            // Say WHICH daemon answered and how it was chosen. A reply that does
            // not name its own subject is how "I asked the stale one" becomes
            // indistinguishable from "I asked mine".
            object.insert(
                "answered_for".to_string(),
                serde_json::json!({
                    "selector": selector,
                    "selector_kind": format!("{kind:?}"),
                    "server_pid": runtime.server_pid,
                    "server_version": runtime.server_version,
                }),
            );
        }
        if rest.last().is_some_and(|verb| verb == "snapshot") {
            println!("{}", serde_json::to_string_pretty(&snapshot(&endpoint)?)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        return Ok(());
    }
    if args.as_slice() == ["server", "status"] {
        let endpoint = cli_server_endpoint(store.home_dir());
        match status(&endpoint) {
            Ok(runtime) => {
                // A CLI answers about the daemon matching its OWN version, which
                // is not necessarily the one the user's window is attached to.
                // Say so in the answer rather than leaving it to be discovered.
                let peers: Vec<yggterm_server::PeerDaemonSummary> =
                    yggterm_server::reachable_versioned_daemon_statuses(store.home_dir())
                        .into_iter()
                        .map(|(_endpoint, peer)| yggterm_server::PeerDaemonSummary {
                            pid: peer.server_pid,
                            version: peer.server_version.clone(),
                            owned_terminal_session_count: peer.owned_terminal_session_count,
                        })
                        .collect();
                let warning = yggterm_server::stale_daemon_answer_warning(
                    runtime.server_pid,
                    runtime.owned_terminal_session_count,
                    &peers,
                );
                let mut value = serde_json::to_value(&runtime)?;
                if let Some(object) = value.as_object_mut() {
                    if let Some(warning) = warning {
                        object.insert(
                            "stale_daemon_warning".to_string(),
                            serde_json::Value::String(warning),
                        );
                    }
                    if peers.len() > 1 {
                        object.insert("peer_daemons".to_string(), serde_json::to_value(&peers)?);
                    }
                }
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
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
        classify_builtin_cli_command, cli_positional_args, command_reads_local_state_in_process,
        gui_companion_executable_from_headless, normalize_monitor_args,
        preferred_headless_executable, remote_session_title_fallback,
    };
    use std::path::PathBuf;
    use yggterm_core::{InstallChannel, InstallContext, UpdatePolicy};
    use yggterm_server::RemoteScannedSession;

    /// The carve-out that keeps a local-log reader in THIS binary. It was an
    /// inline `matches!` with no test, and without `render-top` in it the new
    /// command would exec a stale installed binary and answer "unsupported
    /// server command" on the very host it was measuring — exactly what
    /// `perf-incidents` shipped to fix.
    #[test]
    fn local_state_readers_never_hand_off_to_the_installed_binary() {
        let argv = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        for command in ["perf-summary", "perf-incidents", "render-top"] {
            assert!(
                command_reads_local_state_in_process(&argv(&["server", command])),
                "`server {command}` reads local state in-process and must not hand off"
            );
        }
        // Anything that goes through the daemon SHOULD hand off to the
        // preferred executable — the carve-out is an exception list, not a
        // blanket opt-out.
        assert!(!command_reads_local_state_in_process(&argv(&[
            "server", "snapshot"
        ])));
        assert!(!command_reads_local_state_in_process(&argv(&[
            "server", "status"
        ])));
        assert!(!command_reads_local_state_in_process(&argv(&[
            "render-top"
        ])));
    }

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

    /// The ONE owner of the `server app` verb surface. These locks used to
    /// scan this binary's own copy of the dispatch; there is one copy now, so
    /// they scan it. `neither_binary_dispatches_server_app_itself`
    /// (`apps/yggterm/src/main.rs`) is what keeps a second one from appearing.
    const APP_CONTROL_CLI_SOURCE: &str =
        include_str!("../../../../crates/yggterm-server/src/app_control_cli.rs");

    #[test]
    fn headless_app_control_exposes_theme_editor_actions() {
        let source = APP_CONTROL_CLI_SOURCE;
        assert!(source.contains("\"theme-editor\" =>"));
        assert!(source.contains("run_app_control_set_theme_editor_open"));
        assert!(source.contains("run_app_control_reset_theme_editor"));
        assert!(source.contains("run_app_control_set_theme_editor_values"));
    }

    #[test]
    fn headless_app_control_exposes_maximize_command() {
        let source = APP_CONTROL_CLI_SOURCE;
        assert!(source.contains("server app maximize <on|off|toggle>"));
        assert!(source.contains("\"maximize\" | \"maximized\" =>"));
        assert!(source.contains("run_app_control_set_maximized(enabled, timeout_ms)"));
    }

    #[test]
    fn headless_app_control_exposes_settled_open_path_command() {
        let source = APP_CONTROL_CLI_SOURCE;
        assert!(source.contains("server app open <session-path>"));
        assert!(source.contains("\"open\" =>"));
        assert!(source.contains("run_app_control_open_path(session_path, view_mode, timeout_ms)"));
    }

    #[test]
    fn headless_app_control_routes_launch_through_gui_companion() {
        let source = APP_CONTROL_CLI_SOURCE;
        // Launching is the ONE genuinely per-binary cell, so it is the one
        // thing this binary still answers itself — through the owner's
        // `AppControlHost` hook rather than through a dispatcher of its own.
        assert!(source.contains("host.launch_app(&args, home_dir, timeout_ms)"));
        let this_binary = include_str!("yggterm-headless.rs");
        assert!(this_binary.contains("run_app_launch_via_gui_companion(self.current_exe"));
        assert!(this_binary.contains("server app launch requires a yggterm GUI companion"));
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
            cwd: "/home/user/gh/yggterm".to_string(),
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
            storage_path: "/home/user/.codex/sessions/session.jsonl".to_string(),
            // A scanned row whose title the CLI never wrote: this fixture is
            // exactly the case the cwd fallback exists for.
            title_is_explicit: false,
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

    // ⛔ `server app launch` must not start an OLDER GUI just because a stale
    // install record names one. On the live host the recorded executable was a
    // real, present `2.11.0/yggterm` while 3.0.44 was deployed, so this verb
    // would have put a 2.11.0 window in front of a 3.0.44 daemon.
    #[test]
    fn app_launch_refuses_a_downgrade_gui_and_falls_back_to_its_own_companion() {
        let stale = InstallContext {
            channel: InstallChannel::Direct,
            update_policy: UpdatePolicy::Auto,
            repo: "test/repo".to_string(),
            asset_label: "linux-x86_64".to_string(),
            // What the record CLAIMS is active — older than this build.
            current_version: "2.11.0".to_string(),
            executable_path: PathBuf::from("/home/u/.yggterm/bin/yggterm-headless"),
            preferred_executable: Some(PathBuf::from("/home/u/.yggterm/versions/2.11.0/yggterm")),
            managed_root: Some(PathBuf::from("/home/u/.yggterm")),
            manager_hint: Some("Direct install".to_string()),
        };
        // Both candidates are absent on disk here, so the function can only
        // answer None — but the point is WHICH branch it took: with the guard
        // removed it would return the 2.11.0 path whenever that file exists.
        let recorded = stale.preferred_executable.clone().expect("a recorded path");
        assert!(
            !yggterm_core::handoff_target_is_usable(
                env!("CARGO_PKG_VERSION"),
                &stale.current_version,
                &recorded,
            ),
            "this build must consider a 2.11.0 record a downgrade, or the guard \
             above is inert and the verb starts the old GUI"
        );

        // ⛔ AND the shape that defeated the first guard: the record's VERSION
        // bumped to ours while its PATH stayed on 2.11.0. Live on guihost within an
        // hour of shipping the version-only check.
        let lying = InstallContext {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            ..stale.clone()
        };
        assert!(
            !yggterm_core::handoff_target_is_usable(
                env!("CARGO_PKG_VERSION"),
                &lying.current_version,
                &recorded,
            ),
            "a record whose version contradicts its own path must be refused, \
             not reconciled — there is no way to know which half is true"
        );

        // The case the preference exists for is untouched: a consistent record
        // AHEAD of this build still wins.
        assert!(yggterm_core::handoff_target_is_usable(
            env!("CARGO_PKG_VERSION"),
            "999.0.0",
            std::path::Path::new("/home/u/.yggterm/versions/999.0.0/yggterm"),
        ));
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

    /// `server app terminal new` is parsed TWICE — once here and once in the
    /// GUI binary — so a flag added to one and not the other is a silent
    /// divergence: the same command means different things depending on which
    /// executable the agent reached for. Both parses are read from source
    /// here, because there is no shared parser to assert against.
    #[test]
    fn both_binaries_parse_the_same_terminal_new_flags() {
        // ⭐ THE TWO PARSERS ARE ONE. This asserted that each binary parsed
        // every flag, because each binary had its own `terminal new` parsing
        // and "the same command would mean different things per executable"
        // was a live risk. The dispatch has one owner now, so the flags are
        // parsed once and the risk is structural rather than checked — what
        // is left to lock is that the ONE parser still reads them all.
        for flag in ["--machine-key", "--cwd", "--title", "--purpose", "--kind"] {
            let needle = format!("if window[0] == \"{flag}\"");
            assert!(
                APP_CONTROL_CLI_SOURCE.contains(&needle),
                "the one `server app` dispatcher no longer parses `{flag}` for \
                 terminal new"
            );
        }
        // The purpose has to REACH the verb, not just be parsed and dropped.
        for source in [APP_CONTROL_CLI_SOURCE] {
            let call = source
                .find("run_app_control_create_terminal_with_tenancy(")
                .map(|start| &source[start..start + 220])
                .expect("the create verb is still called");
            assert!(
                call.contains("purpose,"),
                "a parsed `--purpose` that never reaches the verb names nothing:\n{call}"
            );
        }
    }

    /// Both usage blocks must teach the two things an agent gets wrong without
    /// them: that a nameless create names itself, and that a removal answers
    /// with a verdict rather than an assertion.
    #[test]
    fn both_usage_blocks_teach_the_naming_flag_and_the_verified_removal() {
        // ONE usage text now, rendered per binary by its owner — so this reads
        // the owner rather than two copies that could teach different things.
        for source in [APP_CONTROL_CLI_SOURCE] {
            assert!(
                source.contains("[--purpose <what-for>]"),
                "the usage block never mentions --purpose, so no agent will pass it"
            );
            assert!(
                source.contains("verified:false"),
                "the usage block never mentions that a removal can refuse"
            );
        }
    }
}
