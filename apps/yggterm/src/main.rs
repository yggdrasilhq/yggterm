#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use yggterm_core::{
    ENV_YGGTERM_DIRECT_INSTALL_ROOT, ENV_YGGTERM_HOME, InstallContext, PerfSpan, SessionNode,
    SessionNodeKind, SessionStore, UpdatePolicy, WorkspaceDocumentKind, WorkspaceGroupKind,
    append_trace_event, check_for_update, current_version, detect_install_context,
    install_release_update, refresh_desktop_integration,
};
use yggterm_platform::configure_gui_entry_process;
use yggterm_server::{
    AppControlPreviewLayout, AppControlRightPanelMode, AppControlViewMode, PersistedDaemonState,
    ProbeTerminalViewportInputMode, SessionKind, YggtermServer, active_client_instance_records,
    default_endpoint, detect_ghostty_host, ensure_local_daemon_running,
    local_headless_companion_executable_from_current, ping, run_app_control_background_window,
    run_app_control_close_window, run_app_control_create_terminal, run_app_control_describe_rows,
    run_app_control_describe_state, run_app_control_drag, run_app_control_dump_state,
    run_app_control_focus_window, run_app_control_key, run_app_control_list_clients,
    run_app_control_move_window_by, run_app_control_open_path,
    run_app_control_paste_terminal_clipboard, run_app_control_paste_terminal_clipboard_image,
    run_app_control_pointer, run_app_control_probe_terminal_viewport_input,
    run_app_control_probe_terminal_viewport_scroll, run_app_control_probe_terminal_viewport_select,
    run_app_control_reclaim_terminal_focus, run_app_control_remove_session,
    run_app_control_reset_theme_editor, run_app_control_restart_pending_update,
    run_app_control_scroll_preview, run_app_control_send_terminal_input,
    run_app_control_set_clipboard_png_base64, run_app_control_set_clipboard_text,
    run_app_control_set_fullscreen, run_app_control_set_main_zoom, run_app_control_set_maximized,
    run_app_control_set_preview_layout, run_app_control_set_right_panel_mode,
    run_app_control_set_row_expanded, run_app_control_set_search,
    run_app_control_set_session_keep_alive, run_app_control_set_theme_editor_open,
    run_app_control_set_ui_theme, run_app_control_set_window_chrome_hover,
    run_app_control_trigger_update_check, run_attach, run_daemon, run_screenrecord_capture,
    run_screenshot_capture, run_trace_bundle, run_trace_follow, run_trace_tail, shutdown, snapshot,
    start_local_session, status, try_run_remote_server_command,
};
use yggterm_shell::{ShellBootstrap, launch_shell, start_daemon_watchdog, warm_daemon_start};
use yggui_contract::UiTheme;

const DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT_ENV: &str =
    "YGGTERM_DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT";
const ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF: &str = "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF";
const ENV_YGGTERM_ENABLE_ACCESSIBILITY: &str = "YGGTERM_ENABLE_ACCESSIBILITY";
const ENV_YGGTERM_ALLOW_WAYLAND_BACKEND: &str = "YGGTERM_ALLOW_WAYLAND_BACKEND";
const ENV_YGGTERM_ENABLE_WEBKIT_COMPOSITING: &str = "YGGTERM_ENABLE_WEBKIT_COMPOSITING";
const ENV_YGGTERM_ALLOW_MULTI_WINDOW: &str = "YGGTERM_ALLOW_MULTI_WINDOW";
const ENV_YGGTERM_ENABLE_TRANSPARENT_WINDOW: &str = "YGGTERM_ENABLE_TRANSPARENT_WINDOW";
const ENV_YGGTERM_WEBKIT_CACHE_MODEL: &str = "YGGTERM_WEBKIT_CACHE_MODEL";
const ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB: &str = "YGGTERM_WEBKIT_MEMORY_LIMIT_MB";
const ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD: &str =
    "YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD";
const ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD: &str = "YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD";
const ENV_YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC: &str = "YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC";
const ENV_MALLOC_ARENA_MAX: &str = "MALLOC_ARENA_MAX";
const ENV_YGGTERM_RELAUNCH_AFTER_PID: &str = "YGGTERM_RELAUNCH_AFTER_PID";
const ENV_YGGTERM_RELAUNCH_WAIT_TIMEOUT_MS: &str = "YGGTERM_RELAUNCH_WAIT_TIMEOUT_MS";
const DEFAULT_RELAUNCH_WAIT_TIMEOUT_MS: u64 = 15_000;

fn app_control_client_for_pid(payload: &serde_json::Value, pid: u32) -> Option<serde_json::Value> {
    payload
        .get("clients")
        .and_then(serde_json::Value::as_array)
        .and_then(|clients| {
            clients
                .iter()
                .find(|entry| {
                    entry.get("pid").and_then(serde_json::Value::as_u64) == Some(pid as u64)
                })
                .cloned()
        })
}

fn app_control_state_visible_for_pid(payload: &serde_json::Value, pid: u32) -> bool {
    let Some(data) = payload.get("data") else {
        return false;
    };
    let visible = data
        .get("window")
        .and_then(|value| value.get("visible"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !visible {
        return false;
    }
    let client_pid = data
        .get("client_instance")
        .and_then(|value| value.get("pid"))
        .and_then(serde_json::Value::as_u64);
    let handled_by_pid = payload
        .get("handled_by_pid")
        .and_then(serde_json::Value::as_u64);
    client_pid == Some(pid as u64) || handled_by_pid == Some(pid as u64)
}

fn app_control_state_launch_summary(
    payload: &serde_json::Value,
    pid: u32,
) -> Option<serde_json::Value> {
    let data = payload.get("data")?;
    let dom = data.get("dom").cloned().unwrap_or(serde_json::Value::Null);
    Some(serde_json::json!({
        "request_id": payload.get("request_id").cloned().unwrap_or(serde_json::Value::Null),
        "handled_by_pid": payload.get("handled_by_pid").cloned().unwrap_or(serde_json::Value::Null),
        "visible": app_control_state_visible_for_pid(payload, pid),
        "window": data.get("window").cloned().unwrap_or(serde_json::Value::Null),
        "client_instance": data.get("client_instance").cloned().unwrap_or(serde_json::Value::Null),
        "dom": {
            "shell_root_count": dom.get("shell_root_count").cloned().unwrap_or(serde_json::Value::Null),
            "degraded_reason": dom.get("degraded_reason").cloned().unwrap_or(serde_json::Value::Null),
            "error": dom.get("error").cloned().unwrap_or(serde_json::Value::Null),
        },
    }))
}

fn maybe_wait_for_update_relaunch_parent_exit() {
    let Some(pid) = std::env::var(ENV_YGGTERM_RELAUNCH_AFTER_PID)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
    else {
        return;
    };
    unsafe {
        std::env::remove_var(ENV_YGGTERM_RELAUNCH_AFTER_PID);
    }
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let timeout_ms = std::env::var(ENV_YGGTERM_RELAUNCH_WAIT_TIMEOUT_MS)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_RELAUNCH_WAIT_TIMEOUT_MS);
    let started = Instant::now();
    while signal_process_is_alive(pid) && started.elapsed() < Duration::from_millis(timeout_ms) {
        std::thread::sleep(Duration::from_millis(80));
    }
}

fn configure_linux_allocator_limits() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        const ARENA_MAX: libc::c_int = 2;
        if std::env::var_os(ENV_MALLOC_ARENA_MAX).is_none() {
            let exe =
                std::env::current_exe().context("locating yggterm binary for allocator re-exec")?;
            let mut command = Command::new(exe);
            command
                .args(std::env::args_os().skip(1))
                .env(ENV_MALLOC_ARENA_MAX, ARENA_MAX.to_string());
            let error = command.exec();
            return Err(anyhow::anyhow!(
                "re-execing yggterm with allocator limits failed: {error}"
            ));
        }
        let _ = unsafe { libc::mallopt(libc::M_ARENA_MAX, ARENA_MAX) };
    }
    Ok(())
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

fn launch_app_background(
    home_dir: &std::path::Path,
    timeout_ms: u64,
    wait_visible: bool,
    allow_multi_window: bool,
    skip_active_exec_handoff: bool,
    log_path: Option<&str>,
) -> Result<()> {
    let current_exe = std::env::current_exe().context("resolving current yggterm executable")?;
    let control_exe = local_headless_companion_executable_from_current(&current_exe)
        .unwrap_or_else(|| current_exe.clone());
    let chosen_log_path = match log_path {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let logs_dir = home_dir.join("app-launch-logs");
            fs::create_dir_all(&logs_dir).with_context(|| {
                format!(
                    "creating background app launch log dir {}",
                    logs_dir.display()
                )
            })?;
            let ts_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default();
            logs_dir.join(format!("launch-{ts_ms}.log"))
        }
    };
    let stdout_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&chosen_log_path)
        .with_context(|| format!("opening background app log {}", chosen_log_path.display()))?;
    let stderr_file = stdout_file.try_clone().with_context(|| {
        format!(
            "cloning background app log handle {}",
            chosen_log_path.display()
        )
    })?;
    let mut command = Command::new(&current_exe);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .env(ENV_YGGTERM_HOME, home_dir);
    if allow_multi_window {
        command.env("YGGTERM_ALLOW_MULTI_WINDOW", "1");
    }
    if skip_active_exec_handoff {
        command.env("YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF", "1");
    }
    let child = command
        .spawn()
        .with_context(|| format!("spawning background yggterm from {}", current_exe.display()))?;
    let pid = child.id();
    let mut client = None::<serde_json::Value>;
    let mut visibility = None::<serde_json::Value>;
    let mut visibility_error = None::<String>;
    if wait_visible {
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms.max(100));
        let state_timeout_ms = timeout_ms.clamp(250, 1_500);
        let control_cwd = control_exe
            .parent()
            .or_else(|| current_exe.parent())
            .unwrap_or(home_dir)
            .to_path_buf();
        while std::time::Instant::now() <= deadline {
            if client.is_none() {
                let output = Command::new(&control_exe)
                    .args(["server", "app", "clients"])
                    .env(ENV_YGGTERM_HOME, home_dir)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .current_dir(&control_cwd)
                    .output()
                    .with_context(|| {
                        format!("listing app clients via {}", control_exe.display())
                    })?;
                if output.status.success() {
                    if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&output.stdout)
                    {
                        client = app_control_client_for_pid(&payload, pid);
                    }
                }
            }
            if client.is_some() {
                let output = Command::new(&control_exe)
                    .args(["server", "app", "state", "--pid"])
                    .arg(pid.to_string())
                    .arg("--timeout-ms")
                    .arg(state_timeout_ms.to_string())
                    .env(ENV_YGGTERM_HOME, home_dir)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .current_dir(&control_cwd)
                    .output()
                    .with_context(|| {
                        format!("describing app state via {}", control_exe.display())
                    })?;
                if output.status.success() {
                    match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                        Ok(payload) => {
                            if let Some(summary) = app_control_state_launch_summary(&payload, pid) {
                                if app_control_state_visible_for_pid(&payload, pid) {
                                    visibility_error = None;
                                    visibility = Some(summary);
                                    break;
                                }
                                visibility = Some(summary);
                                visibility_error = Some(
                                    "app-control state responded before the window became visible"
                                        .to_string(),
                                );
                            }
                        }
                        Err(error) => {
                            visibility_error =
                                Some(format!("failed to parse app-control state JSON: {error}"));
                        }
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    visibility_error = Some(if stderr.is_empty() { stdout } else { stderr });
                }
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "pid": pid,
            "log_path": chosen_log_path,
            "wait_visible": wait_visible,
            "registered": client.is_some(),
            "visible": visibility
                .as_ref()
                .and_then(|value| value.get("visible"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            "client": client,
            "visibility": visibility,
            "visibility_error": visibility_error,
        }))?
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinCliCommand {
    MainHelp,
    ServerHelp,
    ServerSnapshot,
}

fn classify_builtin_cli_command(args: &[String]) -> Option<BuiltinCliCommand> {
    match args {
        [arg] if matches!(arg.as_str(), "--help" | "-h" | "help") => {
            Some(BuiltinCliCommand::MainHelp)
        }
        [command] if command == "server" => Some(BuiltinCliCommand::ServerHelp),
        [command, arg]
            if command == "server" && matches!(arg.as_str(), "--help" | "-h" | "help") =>
        {
            Some(BuiltinCliCommand::ServerHelp)
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
  yggterm
  yggterm --help
  yggterm --version
  yggterm install
  yggterm doc <subcommand>
  yggterm server <subcommand>

common server commands:
  yggterm server daemon
  yggterm server status
  yggterm server snapshot
  yggterm server app <subcommand>"
    );
}

fn print_server_help() {
    println!(
        "usage:
  yggterm server daemon
  yggterm server attach <session> [cwd]
  yggterm server ping
  yggterm server status
  yggterm server snapshot
  yggterm server shutdown
  yggterm server smoke
  yggterm server trace <tail|follow|bundle>
  yggterm server screenshot <target> [output]
  yggterm server screenrecord <target> [output]
  yggterm server app <subcommand>"
    );
}

fn ensure_local_server_ready_for_cli(store: &SessionStore) -> Result<()> {
    let endpoint = default_endpoint(store.home_dir());
    ensure_local_daemon_running(&endpoint)
}

fn main() -> Result<()> {
    maybe_wait_for_update_relaunch_parent_exit();
    configure_linux_allocator_limits()?;
    configure_linux_desktop_backend();
    configure_linux_accessibility_bridge();
    configure_linux_webkit_compositing();
    configure_linux_webkit_memory_policy();
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        configure_gui_entry_process("Yggterm", "dev.yggterm.Yggterm")?;
    }
    let current_exe = std::env::current_exe()?;
    let install_context = detect_install_context(&current_exe)?;
    maybe_handoff_to_preferred_executable(&current_exe, &args, &install_context)?;
    let store = SessionStore::open_or_init()?;
    install_panic_logging(store.home_dir());
    let startup_home = store.home_dir().to_path_buf();
    maybe_focus_existing_client(store.home_dir(), &args, &current_exe)?;
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "main_enter",
        serde_json::json!({ "args": args.clone() }),
    );
    #[cfg(target_os = "linux")]
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "linux_desktop_backend_policy",
        serde_json::json!({
            "gdk_backend": std::env::var("GDK_BACKEND").ok(),
            "winit_unix_backend": std::env::var("WINIT_UNIX_BACKEND").ok(),
            "policy": std::env::var("YGGTERM_LINUX_BACKEND_POLICY").ok(),
            "wayland_display_present": std::env::var_os("WAYLAND_DISPLAY").is_some(),
            "display_present": std::env::var_os("DISPLAY").is_some(),
        }),
    );
    let startup_span = PerfSpan::start(&startup_home, "startup", "gui_main");
    let pending_update_restart = None;
    let launch_install_context = install_context.clone();
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
        let output_path = args
            .iter()
            .skip(3)
            .find(|value| !value.starts_with("--"))
            .map(String::as_str);
        return run_screenshot_capture(&args[2], output_path, timeout_ms);
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
            BuiltinCliCommand::ServerHelp => {
                print_server_help();
                return Ok(());
            }
            BuiltinCliCommand::ServerSnapshot => {
                ensure_local_server_ready_for_cli(&store)?;
                let endpoint = default_endpoint(store.home_dir());
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
                run_screenshot_capture(target, output_path, timeout_ms)
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
            "launch" => {
                let wait_visible = args.iter().any(|arg| arg == "--wait-visible");
                let allow_multi_window = args.iter().any(|arg| arg == "--allow-multi-window");
                let skip_active_exec_handoff =
                    args.iter().any(|arg| arg == "--skip-active-exec-handoff");
                let log_path = args.windows(2).find_map(|window| {
                    if window[0] == "--log" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                });
                launch_app_background(
                    store.home_dir(),
                    timeout_ms,
                    wait_visible,
                    allow_multi_window,
                    skip_active_exec_handoff,
                    log_path,
                )
            }
            "clients" => run_app_control_list_clients(),
            "state" => run_app_control_describe_state(timeout_ms),
            "dump" => {
                let output_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .context("missing output path for server app dump")?;
                run_app_control_dump_state(output_path, timeout_ms)
            }
            "rows" => run_app_control_describe_rows(timeout_ms),
            "preview" => {
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
                    "layout" => {
                        let layout = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .unwrap_or("chat");
                        let layout = match layout {
                            "chat" => AppControlPreviewLayout::Chat,
                            "graph" | "overview" => AppControlPreviewLayout::Graph,
                            other => anyhow::bail!("unsupported app preview layout: {other}"),
                        };
                        run_app_control_set_preview_layout(layout, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app preview action: {other}"),
                }
            }
            "zoom" => {
                let value = args
                    .windows(2)
                    .find_map(|window| {
                        (window[0] == "--value").then(|| window[1].parse::<f32>().ok())
                    })
                    .flatten()
                    .context("missing --value for server app zoom")?;
                let view_mode = args.windows(2).find_map(|window| {
                    if window[0] != "--view" {
                        return None;
                    }
                    match window[1].as_str() {
                        "preview" | "rendered" => Some(AppControlViewMode::Preview),
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
                    .context("missing row path for server app expand/collapse")?;
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
            "close" | "quit" | "exit" => run_app_control_close_window(timeout_ms),
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
            "clipboard" => {
                let action = args.get(3).map(String::as_str).unwrap_or("text");
                match action {
                    "text" | "set" => {
                        let value = cli_flag_value(&args, "--value")
                            .or_else(|| cli_flag_value(&args, "--text"))
                            .or_else(|| cli_positional_args(&args, 4).into_iter().next())
                            .unwrap_or("");
                        run_app_control_set_clipboard_text(value, timeout_ms)
                    }
                    "png" | "image" | "png-base64" => {
                        let value = cli_flag_value(&args, "--base64")
                            .or_else(|| cli_flag_value(&args, "--value"))
                            .or_else(|| cli_positional_args(&args, 4).into_iter().next())
                            .context("missing --base64/--value for server app clipboard image")?;
                        run_app_control_set_clipboard_png_base64(value, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app clipboard action: {other}"),
                }
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
            "panel" | "right-panel" => {
                let mode = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("hidden");
                let mode = match mode {
                    "hidden" | "hide" | "close" | "none" => AppControlRightPanelMode::Hidden,
                    "connect" => AppControlRightPanelMode::Connect,
                    "notifications" | "notification" => AppControlRightPanelMode::Notifications,
                    "settings" => AppControlRightPanelMode::Settings,
                    "metadata" | "session-metadata" => AppControlRightPanelMode::Metadata,
                    other => anyhow::bail!("unsupported app right panel mode: {other}"),
                };
                run_app_control_set_right_panel_mode(mode, timeout_ms)
            }
            "theme" => {
                let theme = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("light");
                let theme = match theme {
                    "light" => UiTheme::ZedLight,
                    "dark" => UiTheme::ZedDark,
                    other => anyhow::bail!("unsupported app theme: {other}"),
                };
                run_app_control_set_ui_theme(theme, timeout_ms)
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
                    other => anyhow::bail!("unsupported app theme-editor action: {other}"),
                }
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
            "fullscreen" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("toggle");
                let enabled = match action {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    "toggle" => {
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
                        !currently_fullscreen
                    }
                    other => anyhow::bail!("unsupported fullscreen action: {other}"),
                };
                run_app_control_set_fullscreen(enabled, timeout_ms)
            }
            "maximize" | "maximized" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("toggle");
                let enabled = match action {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    "toggle" => {
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
                        !currently_maximized
                    }
                    other => anyhow::bail!("unsupported maximize action: {other}"),
                };
                run_app_control_set_maximized(enabled, timeout_ms)
            }
            "open" => {
                let session_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .context("missing session path for server app open")?;
                let view_mode = args.windows(2).find_map(|window| {
                    if window[0] != "--view" {
                        return None;
                    }
                    match window[1].as_str() {
                        "preview" | "rendered" => Some(AppControlViewMode::Preview),
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
                    .context("missing action for server app drag")?;
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
                    .context("missing action for server app pointer")?;
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
            "key" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing action for server app key")?;
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
                    .context("missing action for server app terminal")?;
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
                            .context("missing session path for server app terminal send")?;
                        let data = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .context("missing --data for server app terminal send")?;
                        run_app_control_send_terminal_input(session_path, data, timeout_ms)
                    }
                    "focus" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal focus")?;
                        run_app_control_reclaim_terminal_focus(session_path, timeout_ms)
                    }
                    "paste" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal paste")?;
                        run_app_control_paste_terminal_clipboard(session_path, timeout_ms)
                    }
                    "paste-image" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal paste-image")?;
                        run_app_control_paste_terminal_clipboard_image(session_path, timeout_ms)
                    }
                    "keep" | "keep-alive" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal keep")?;
                        run_app_control_set_session_keep_alive(session_path, true, timeout_ms)
                    }
                    "unkeep" | "stop-keep-alive" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal unkeep")?;
                        run_app_control_set_session_keep_alive(session_path, false, timeout_ms)
                    }
                    "probe-type" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal probe-type")?;
                        let data = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .context("missing --data for server app terminal probe-type")?;
                        let press_enter = args.iter().any(|arg| arg == "--enter");
                        let press_tab = args.iter().any(|arg| arg == "--tab");
                        let press_ctrl_c = args.iter().any(|arg| arg == "--ctrl-c");
                        let press_ctrl_e = args.iter().any(|arg| arg == "--ctrl-e");
                        let press_ctrl_u = args.iter().any(|arg| arg == "--ctrl-u");
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
                            .context("missing session path for server app terminal probe-scroll")?;
                        let lines = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--lines" {
                                    window[1].parse::<i32>().ok()
                                } else {
                                    None
                                }
                            })
                            .context("missing --lines for server app terminal probe-scroll")?;
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
                            .context("missing session path for server app terminal probe-select")?;
                        run_app_control_probe_terminal_viewport_select(session_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app terminal action: {other}"),
                }
            }
            "session" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing action for server app session")?;
                match action {
                    "remove" | "delete" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app session remove")?;
                        run_app_control_remove_session(session_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app session action: {other}"),
                }
            }
            other => anyhow::bail!("unsupported app control command: {other}"),
        };
    }
    if args.as_slice() == ["server", "shutdown"] {
        let endpoint = default_endpoint(store.home_dir());
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        return Ok(());
    }
    if args.as_slice() == ["server", "ping"] {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = default_endpoint(store.home_dir());
        ping(&endpoint)?;
        println!("pong");
        return Ok(());
    }
    if args.as_slice() == ["server", "status"] {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = default_endpoint(store.home_dir());
        let runtime = status(&endpoint)?;
        println!("{}", serde_json::to_string_pretty(&runtime)?);
        return Ok(());
    }
    if args.as_slice() == ["server", "smoke"] {
        return run_server_smoke();
    }
    if args.first().is_some_and(|arg| arg == "server") {
        anyhow::bail!(
            "unsupported server command: {}",
            args.get(1).map(String::as_str).unwrap_or("<missing>")
        );
    }
    if matches!(
        args.first().map(String::as_str),
        Some("--version" | "-V" | "version")
    ) {
        println!("{}", current_version());
        return Ok(());
    }
    if let Some(command) = args.first()
        && command == "install"
    {
        return run_install_cli(&install_context);
    }
    if let Some(command) = args.first()
        && command == "doc"
    {
        return run_document_cli(&store, &args[1..]);
    }

    let settings_span = PerfSpan::start(&startup_home, "startup", "load_settings");
    let settings = store.load_settings().unwrap_or_default();
    settings_span.finish(serde_json::json!({}));
    let tree = placeholder_session_tree(store.sessions_root().to_path_buf(), settings.theme);
    let browser_tree_span = PerfSpan::start(&startup_home, "startup", "load_browser_tree");
    let (browser_tree, browser_tree_loaded) = match store.load_codex_tree(&settings) {
        Ok(tree) => (tree, true),
        Err(error) => {
            tracing::warn!(error=%error, "failed to load browser tree for warm start");
            (
                placeholder_session_tree(store.home_dir().to_path_buf(), settings.theme),
                false,
            )
        }
    };
    browser_tree_span.finish(serde_json::json!({
        "loaded": browser_tree_loaded,
    }));
    let settings_path = store.settings_path();
    let theme = settings.theme;
    let prefer_ghostty_backend = settings.prefer_ghostty_backend;
    let endpoint = default_endpoint(store.home_dir());
    install_signal_shutdown(store.home_dir().to_path_buf(), endpoint.clone());
    warm_daemon_start(endpoint.clone(), Some(startup_home.clone()));
    start_daemon_watchdog(endpoint.clone(), Some(startup_home.clone()));
    let linux_window_profile = detect_linux_window_profile();
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "linux_window_profile",
        serde_json::json!({
            "transparent": linux_window_profile.transparent,
            "xrpd_session": linux_window_profile.xrpd_session,
            "reason": linux_window_profile.reason,
        }),
    );
    let host_span = PerfSpan::start(&startup_home, "startup", "detect_terminal_host");
    let host = detect_ghostty_host();
    host_span.finish(serde_json::json!({ "detail": host.detail }));
    let initial_server_sync_span = PerfSpan::start(&startup_home, "startup", "warm_server_sync");
    let initial_server_snapshot_load = load_initial_server_snapshot_fast(
        &store,
        &browser_tree,
        prefer_ghostty_backend,
        &host,
        theme,
    );
    let initial_server_snapshot = initial_server_snapshot_load.snapshot;
    initial_server_sync_span.finish(serde_json::json!({
        "mode": "cached_snapshot_only",
        "detail": initial_server_snapshot_load.detail,
    }));
    let server_daemon_detail = if initial_server_snapshot.is_some() {
        "warming server in background".to_string()
    } else {
        "no cached server snapshot".to_string()
    };

    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "before_launch_shell",
        serde_json::json!({
            "pid": std::process::id(),
            "transparent": linux_window_profile.transparent,
            "profile_reason": linux_window_profile.reason,
            "browser_tree_loaded": browser_tree_loaded,
            "initial_server_snapshot": initial_server_snapshot.is_some(),
        }),
    );

    let launch_result = launch_shell(ShellBootstrap {
        tree,
        browser_tree,
        browser_tree_loaded,
        settings,
        install_context: launch_install_context,
        settings_path,
        server_endpoint: endpoint.clone(),
        initial_server_snapshot,
        theme,
        ghostty_bridge_enabled: host.bridge_enabled,
        ghostty_embedded_surface_supported: host.embedded_surface_supported,
        ghostty_bridge_detail: host.detail.clone(),
        server_daemon_detail,
        prefer_ghostty_backend,
        pending_update_restart,
        refresh_server_after_launch: true,
        linux_window_transparent: linux_window_profile.transparent,
        linux_window_profile_reason: linux_window_profile.reason.to_string(),
    });
    startup_span.finish(serde_json::json!({
        "update_policy": format!("{:?}", install_context.update_policy),
        "theme": match theme { UiTheme::ZedLight => "light", UiTheme::ZedDark => "dark" },
    }));
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "main_exit",
        serde_json::json!({
            "ok": launch_result.is_ok(),
        }),
    );
    launch_result
}

#[derive(Debug, Clone)]
struct LinuxWindowProfile {
    transparent: bool,
    xrpd_session: bool,
    reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxWindowProfileInput {
    transparent_opt_in: bool,
    wayland_display_present: bool,
    display_present: bool,
    gdk_backend_x11: bool,
    kde_session: bool,
    xrpd_session: bool,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxDesktopBackendPolicyInput {
    allow_wayland_backend: bool,
    gdk_backend_set: bool,
    winit_backend_set: bool,
    kde_session: bool,
    wayland_display_present: bool,
    display_present: bool,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxDesktopBackendPolicy {
    force_x11_backend: bool,
    set_gdk_backend: bool,
    set_winit_backend: bool,
    reason: &'static str,
}

#[cfg(target_os = "linux")]
fn linux_desktop_backend_policy_from_input(
    input: LinuxDesktopBackendPolicyInput,
) -> LinuxDesktopBackendPolicy {
    if input.allow_wayland_backend {
        return LinuxDesktopBackendPolicy {
            force_x11_backend: false,
            set_gdk_backend: false,
            set_winit_backend: false,
            reason: "wayland_backend_explicitly_allowed",
        };
    }
    if input.gdk_backend_set {
        return LinuxDesktopBackendPolicy {
            force_x11_backend: false,
            set_gdk_backend: false,
            set_winit_backend: false,
            reason: "gdk_backend_explicit",
        };
    }
    if !(input.kde_session && input.wayland_display_present && input.display_present) {
        return LinuxDesktopBackendPolicy {
            force_x11_backend: false,
            set_gdk_backend: false,
            set_winit_backend: false,
            reason: "no_kde_wayland_x11_pair",
        };
    }
    LinuxDesktopBackendPolicy {
        force_x11_backend: true,
        set_gdk_backend: true,
        set_winit_backend: !input.winit_backend_set,
        reason: "kde_wayland_x11_default",
    }
}

#[cfg(target_os = "linux")]
fn linux_env_flag_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(target_os = "linux")]
fn linux_session_env_looks_like_kde_plasma() -> bool {
    [
        std::env::var("XDG_CURRENT_DESKTOP").ok(),
        std::env::var("XDG_SESSION_DESKTOP").ok(),
        std::env::var("DESKTOP_SESSION").ok(),
        std::env::var("KDE_FULL_SESSION").ok(),
    ]
    .into_iter()
    .flatten()
    .any(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        normalized.contains("kde")
            || normalized.contains("plasma")
            || matches!(normalized.as_str(), "true" | "1")
    })
}

#[cfg(target_os = "linux")]
fn configure_linux_desktop_backend() {
    let policy = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
        allow_wayland_backend: linux_env_flag_truthy(ENV_YGGTERM_ALLOW_WAYLAND_BACKEND),
        gdk_backend_set: std::env::var_os("GDK_BACKEND").is_some(),
        winit_backend_set: std::env::var_os("WINIT_UNIX_BACKEND").is_some(),
        kde_session: linux_session_env_looks_like_kde_plasma(),
        wayland_display_present: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        display_present: std::env::var_os("DISPLAY").is_some(),
    });
    if !policy.force_x11_backend {
        return;
    }
    if policy.set_gdk_backend {
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }
    if policy.set_winit_backend {
        unsafe { std::env::set_var("WINIT_UNIX_BACKEND", "x11") };
    }
    unsafe { std::env::set_var("YGGTERM_LINUX_BACKEND_POLICY", policy.reason) };
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_desktop_backend() {}

fn linux_window_profile_from_input(input: LinuxWindowProfileInput) -> LinuxWindowProfile {
    if input.transparent_opt_in {
        return LinuxWindowProfile {
            transparent: true,
            xrpd_session: input.xrpd_session,
            reason: "explicit_opt_in",
        };
    }
    if input.xrpd_session {
        return LinuxWindowProfile {
            transparent: false,
            xrpd_session: true,
            reason: "xrdp_opaque_profile",
        };
    }
    if input.kde_session
        && (input.gdk_backend_x11 || (input.display_present && !input.wayland_display_present))
    {
        return LinuxWindowProfile {
            transparent: true,
            xrpd_session: false,
            reason: "kde_x11_transparent_profile",
        };
    }
    if input.gdk_backend_x11 || (input.display_present && !input.wayland_display_present) {
        return LinuxWindowProfile {
            transparent: false,
            xrpd_session: false,
            reason: "x11_native_shape_profile",
        };
    }
    LinuxWindowProfile {
        transparent: false,
        xrpd_session: false,
        reason: if input.wayland_display_present {
            "wayland_opaque_default"
        } else {
            "linux_opaque_default"
        },
    }
}

fn detect_linux_window_profile() -> LinuxWindowProfile {
    #[cfg(target_os = "linux")]
    {
        let transparent_opt_in = std::env::var(ENV_YGGTERM_ENABLE_TRANSPARENT_WINDOW)
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        let wayland_display_present = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let display_present = std::env::var_os("DISPLAY").is_some();
        let gdk_backend_x11 = std::env::var("GDK_BACKEND")
            .ok()
            .is_some_and(|value| value.split(',').any(|part| part.trim() == "x11"));
        let xrpd_session = std::env::var_os("XRDP_SESSION").is_some()
            || std::env::var_os("XRDP_SOCKET_PATH").is_some();
        return linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in,
            wayland_display_present,
            display_present,
            gdk_backend_x11,
            kde_session: linux_session_env_looks_like_kde_plasma(),
            xrpd_session,
        });
    }

    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        {
            return LinuxWindowProfile {
                transparent: true,
                xrpd_session: false,
                reason: "windows_transparent_profile",
            };
        }

        #[cfg(target_os = "macos")]
        {
            return LinuxWindowProfile {
                transparent: true,
                xrpd_session: false,
                reason: "macos_transparent_profile",
            };
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        LinuxWindowProfile {
            transparent: false,
            xrpd_session: false,
            reason: "non_linux",
        }
    }
}

fn configure_linux_accessibility_bridge() {
    #[cfg(target_os = "linux")]
    {
        let accessibility_enabled = std::env::var(ENV_YGGTERM_ENABLE_ACCESSIBILITY)
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        if accessibility_enabled || std::env::var_os("NO_AT_BRIDGE").is_some() {
            return;
        }
        // WebKitGTK can crash in libatk-bridge on some KDE/Wayland sessions before the
        // window becomes visible. Default to the safer path and leave an opt-in escape hatch.
        unsafe { std::env::set_var("NO_AT_BRIDGE", "1") };
    }
}

#[cfg(target_os = "linux")]
fn configure_linux_webkit_compositing() {
    let compositing_enabled = std::env::var_os(ENV_YGGTERM_ENABLE_WEBKIT_COMPOSITING).is_some();
    if compositing_enabled || std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_some() {
        return;
    }
    // Jojo has repeated Mesa/WebKitGTK EGL crashes on the GPU compositing path.
    // Default to software compositing unless the user opts back in explicitly.
    unsafe { std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1") };
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_webkit_compositing() {}

#[cfg(target_os = "linux")]
fn configure_linux_webkit_memory_policy() {
    if std::env::var_os(ENV_YGGTERM_WEBKIT_CACHE_MODEL).is_none() {
        unsafe { std::env::set_var(ENV_YGGTERM_WEBKIT_CACHE_MODEL, "document-viewer") };
    }
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB).is_none() {
        unsafe { std::env::set_var(ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB, "320") };
    }
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD).is_none() {
        unsafe { std::env::set_var(ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD, "0.33") };
    }
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD).is_none() {
        unsafe { std::env::set_var(ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD, "0.50") };
    }
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC).is_none() {
        unsafe { std::env::set_var(ENV_YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC, "30.0") };
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_webkit_memory_policy() {}

struct InitialServerSnapshotLoad {
    snapshot: Option<yggterm_server::ServerUiSnapshot>,
    detail: serde_json::Value,
}

fn load_initial_server_snapshot_fast(
    store: &SessionStore,
    browser_tree: &SessionNode,
    prefer_ghostty_backend: bool,
    host: &yggterm_server::GhosttyHostSupport,
    theme: UiTheme,
) -> InitialServerSnapshotLoad {
    if std::env::var(DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT_ENV)
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        return InitialServerSnapshotLoad {
            snapshot: None,
            detail: serde_json::json!({
                "loaded": false,
                "reason": "debug_disabled",
                "state_path": store.home_dir().join("server-state.json").display().to_string(),
            }),
        };
    }
    let state_path = store.home_dir().join("server-state.json");
    let saved_json = match fs::read_to_string(&state_path) {
        Ok(json) => json,
        Err(error) => {
            return InitialServerSnapshotLoad {
                snapshot: None,
                detail: serde_json::json!({
                    "loaded": false,
                    "reason": "read_failed",
                    "state_path": state_path.display().to_string(),
                    "error": error.to_string(),
                }),
            };
        }
    };
    let saved = match serde_json::from_str::<PersistedDaemonState>(&saved_json) {
        Ok(saved) => saved,
        Err(error) => {
            return InitialServerSnapshotLoad {
                snapshot: None,
                detail: serde_json::json!({
                    "loaded": false,
                    "reason": "parse_failed",
                    "state_path": state_path.display().to_string(),
                    "error": error.to_string(),
                    "json_size": saved_json.len(),
                }),
            };
        }
    };
    let mut server = YggtermServer::new(browser_tree, prefer_ghostty_backend, host.clone(), theme);
    server.restore_persisted_state_with_launch_policy(saved, Some(store), false);
    InitialServerSnapshotLoad {
        snapshot: Some(server.snapshot()),
        detail: serde_json::json!({
            "loaded": true,
            "reason": "restored",
            "state_path": state_path.display().to_string(),
        }),
    }
}

fn install_signal_shutdown(home_dir: std::path::PathBuf, endpoint: yggterm_server::ServerEndpoint) {
    static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);
    if HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let shutdown_started = Arc::new(AtomicBool::new(false));
    let handler_flag = shutdown_started.clone();
    let _ = ctrlc::set_handler(move || {
        if handler_flag.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(remaining_clients) = unregister_signal_client_instance(&home_dir, &endpoint) {
            if remaining_clients == 0 {
                let _ = shutdown(&endpoint);
            }
        }
        std::process::exit(130);
    });
}

fn signal_client_instance_scope(endpoint: &yggterm_server::ServerEndpoint) -> String {
    let raw = match endpoint {
        #[cfg(unix)]
        yggterm_server::ServerEndpoint::UnixSocket(path) => format!("unix-{}", path.display()),
        yggterm_server::ServerEndpoint::Tcp { host, port } => format!("tcp-{host}-{port}"),
    };
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn signal_client_instances_dir(
    home_dir: &std::path::Path,
    endpoint: &yggterm_server::ServerEndpoint,
) -> std::path::PathBuf {
    home_dir
        .join("client-instances")
        .join(signal_client_instance_scope(endpoint))
}

fn signal_client_instance_dirs_for_scan(
    home_dir: &std::path::Path,
    endpoint: &yggterm_server::ServerEndpoint,
) -> Vec<std::path::PathBuf> {
    let current = signal_client_instances_dir(home_dir, endpoint);
    let root = home_dir.join("client-instances");
    let mut dirs = vec![current.clone()];
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path != current && path.is_dir() {
                dirs.push(path);
            }
        }
    }
    dirs
}

fn maybe_focus_existing_client(
    home_dir: &std::path::Path,
    args: &[String],
    current_exe: &std::path::Path,
) -> Result<()> {
    if !args.is_empty()
        || std::env::var_os(ENV_YGGTERM_ALLOW_MULTI_WINDOW).is_some()
        || std::env::var_os(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF).is_some()
    {
        return Ok(());
    }
    let endpoint = default_endpoint(home_dir);
    let active_records = active_client_instance_records(home_dir, &endpoint)?;
    let Some(target_pid) = active_records
        .iter()
        .filter(|record| record_matches_executable(record.executable_path.as_deref(), current_exe))
        .max_by_key(|record| record.started_at_ms)
        .map(|record| record.pid)
    else {
        return Ok(());
    };
    unsafe {
        std::env::set_var("YGGTERM_APP_CONTROL_PID", target_pid.to_string());
    }
    let focused = run_app_control_focus_window(3_000).is_ok();
    unsafe {
        std::env::remove_var("YGGTERM_APP_CONTROL_PID");
    }
    if focused {
        std::process::exit(0);
    }
    Ok(())
}

fn canonical_executable_for_match(path: &std::path::Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn record_matches_executable(
    record_executable_path: Option<&str>,
    current_exe: &std::path::Path,
) -> bool {
    let Some(record_path) = record_executable_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    executable_paths_match(
        &canonical_executable_for_match(&std::path::PathBuf::from(record_path)),
        &canonical_executable_for_match(current_exe),
    )
}

fn executable_paths_match(left: &std::path::Path, right: &std::path::Path) -> bool {
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

fn maybe_handoff_to_preferred_executable(
    current_exe: &std::path::Path,
    args: &[String],
    install_context: &InstallContext,
) -> Result<()> {
    if std::env::var_os(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF).is_some() {
        return Ok(());
    }
    let Some(preferred) = install_context.preferred_executable.as_ref() else {
        return Ok(());
    };
    let current = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.to_path_buf());
    let preferred = preferred
        .canonicalize()
        .unwrap_or_else(|_| preferred.to_path_buf());
    if current == preferred || !preferred.is_file() {
        return Ok(());
    }
    let mut command = Command::new(&preferred);
    command.args(args);
    command.env(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF, "1");
    if let Some(root) = install_context.managed_root.as_ref() {
        command.env(ENV_YGGTERM_DIRECT_INSTALL_ROOT, root);
    }
    let status = command
        .status()
        .with_context(|| format!("failed to hand off launch to {}", preferred.display()))?;
    std::process::exit(status.code().unwrap_or(0));
}

fn signal_parse_client_pid(path: &std::path::Path) -> Option<u32> {
    let file_name = path.file_name()?.to_str()?;
    let pid_prefix = file_name.split('-').next()?;
    pid_prefix.parse::<u32>().ok()
}

#[cfg(unix)]
fn signal_process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error()
        .raw_os_error()
        .is_some_and(|code| code == libc::EPERM)
}

#[cfg(target_os = "windows")]
fn signal_process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let filter = format!("PID eq {pid}");
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", filter.as_str(), "/NH"])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn signal_process_is_alive(pid: u32) -> bool {
    pid != 0
}

#[cfg(target_os = "linux")]
fn signal_process_start_ticks(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    signal_parse_process_start_ticks_from_stat(&stat)
}

#[cfg(not(target_os = "linux"))]
fn signal_process_start_ticks(_pid: u32) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn signal_parse_process_start_ticks_from_stat(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(") ")?;
    rest.split_whitespace().nth(19)?.parse::<u64>().ok()
}

#[cfg(not(target_os = "linux"))]
fn signal_parse_process_start_ticks_from_stat(_stat: &str) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn signal_process_has_gui_client_argv(pid: u32) -> bool {
    let payload = match fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    let args = payload
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    if args.len() != 1 {
        return false;
    }
    std::str::from_utf8(args[0]).ok().is_some_and(|arg0| {
        std::path::Path::new(arg0)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("yggterm")
    })
}

#[cfg(not(target_os = "linux"))]
fn signal_process_has_gui_client_argv(_pid: u32) -> bool {
    true
}

fn signal_record_matches_live_process(pid: u32, path: &std::path::Path) -> bool {
    if !signal_process_is_alive(pid) {
        return false;
    }
    if let Some(expected_start_ticks) = read_signal_process_start_ticks_from_record(path) {
        if let Some(actual_start_ticks) = signal_process_start_ticks(pid) {
            return actual_start_ticks == expected_start_ticks;
        }
    }
    signal_process_has_gui_client_argv(pid)
}

fn unregister_signal_client_instance(
    home_dir: &std::path::Path,
    endpoint: &yggterm_server::ServerEndpoint,
) -> Result<usize> {
    let current_pid = std::process::id();
    let mut remaining_pids = std::collections::BTreeSet::new();
    for dir in signal_client_instance_dirs_for_scan(home_dir, endpoint) {
        fs::create_dir_all(&dir)?;
        let entries = fs::read_dir(&dir)
            .with_context(|| format!("reading client instances {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(pid) = signal_parse_client_pid(&path) else {
                let _ = fs::remove_file(&path);
                continue;
            };
            if pid == current_pid {
                let _ = fs::remove_file(&path);
                continue;
            }
            if signal_record_matches_live_process(pid, &path) {
                remaining_pids.insert(pid);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(remaining_pids.len())
}

fn compatible_signal_client_count(
    home_dir: &std::path::Path,
    endpoint: &yggterm_server::ServerEndpoint,
) -> Result<usize> {
    let current_scope = current_signal_client_scope();
    let mut live = std::collections::BTreeSet::new();
    for dir in signal_client_instance_dirs_for_scan(home_dir, endpoint) {
        fs::create_dir_all(&dir)?;
        let entries = fs::read_dir(&dir)
            .with_context(|| format!("reading client instances {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(pid) = signal_parse_client_pid(&path) else {
                let _ = fs::remove_file(&path);
                continue;
            };
            if !signal_record_matches_live_process(pid, &path) {
                let _ = fs::remove_file(&path);
                continue;
            }
            if signal_client_scope_matches_pid(pid, &path, &current_scope) {
                live.insert(pid);
            }
        }
    }
    Ok(live.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SignalClientScope {
    display: Option<String>,
    wayland_display: Option<String>,
    xdg_session_id: Option<String>,
    xdg_runtime_dir: Option<String>,
    xauthority: Option<String>,
}

fn current_signal_client_scope() -> SignalClientScope {
    SignalClientScope {
        display: signal_env_var("DISPLAY"),
        wayland_display: signal_env_var("WAYLAND_DISPLAY"),
        xdg_session_id: signal_env_var("XDG_SESSION_ID"),
        xdg_runtime_dir: signal_env_var("XDG_RUNTIME_DIR"),
        xauthority: signal_env_var("XAUTHORITY"),
    }
}

fn signal_env_var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn signal_client_scope_matches_pid(
    pid: u32,
    path: &std::path::Path,
    current: &SignalClientScope,
) -> bool {
    if let Some(scope) = read_signal_client_scope_from_record(path) {
        return signal_client_scope_matches(&scope, current);
    }
    #[cfg(target_os = "linux")]
    if let Some(scope) = read_signal_client_scope_from_proc(pid) {
        return signal_client_scope_matches(&scope, current);
    }
    current.display.is_none()
        && current.wayland_display.is_none()
        && current.xdg_session_id.is_none()
        && current.xdg_runtime_dir.is_none()
        && current.xauthority.is_none()
}

fn read_signal_process_start_ticks_from_record(path: &std::path::Path) -> Option<u64> {
    let payload = fs::read(path).ok()?;
    let value = serde_json::from_slice::<serde_json::Value>(&payload).ok()?;
    value
        .get("process_start_ticks")
        .and_then(serde_json::Value::as_u64)
}

fn read_signal_client_scope_from_record(path: &std::path::Path) -> Option<SignalClientScope> {
    let payload = fs::read(path).ok()?;
    let value = serde_json::from_slice::<serde_json::Value>(&payload).ok()?;
    let scope = SignalClientScope {
        display: value
            .get("display")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        wayland_display: value
            .get("wayland_display")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        xdg_session_id: value
            .get("xdg_session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        xdg_runtime_dir: value
            .get("xdg_runtime_dir")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        xauthority: value
            .get("xauthority")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
    };
    if scope.display.is_none()
        && scope.wayland_display.is_none()
        && scope.xdg_session_id.is_none()
        && scope.xdg_runtime_dir.is_none()
        && scope.xauthority.is_none()
    {
        None
    } else {
        Some(scope)
    }
}

#[cfg(target_os = "linux")]
fn read_signal_client_scope_from_proc(pid: u32) -> Option<SignalClientScope> {
    let payload = fs::read(format!("/proc/{pid}/environ")).ok()?;
    let mut scope = SignalClientScope {
        display: None,
        wayland_display: None,
        xdg_session_id: None,
        xdg_runtime_dir: None,
        xauthority: None,
    };
    for entry in payload.split(|byte| *byte == 0) {
        let Ok(text) = std::str::from_utf8(entry) else {
            continue;
        };
        let Some((key, value)) = text.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key {
            "DISPLAY" => scope.display = Some(value.to_string()),
            "WAYLAND_DISPLAY" => scope.wayland_display = Some(value.to_string()),
            "XDG_SESSION_ID" => scope.xdg_session_id = Some(value.to_string()),
            "XDG_RUNTIME_DIR" => scope.xdg_runtime_dir = Some(value.to_string()),
            "XAUTHORITY" => scope.xauthority = Some(value.to_string()),
            _ => {}
        }
    }
    Some(scope)
}

fn signal_client_scope_matches(candidate: &SignalClientScope, current: &SignalClientScope) -> bool {
    if candidate.wayland_display.is_some() || current.wayland_display.is_some() {
        return candidate.wayland_display == current.wayland_display
            && candidate.xdg_runtime_dir == current.xdg_runtime_dir;
    }
    if candidate.display.is_some() || current.display.is_some() {
        return candidate.display == current.display && candidate.xauthority == current.xauthority;
    }
    candidate.xdg_session_id == current.xdg_session_id
        && candidate.xdg_runtime_dir == current.xdg_runtime_dir
}

fn install_panic_logging(home_dir: &std::path::Path) {
    let panic_log_path = home_dir.join("panic.log");
    let trace_home = home_dir.to_path_buf();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let message = format!(
            "timestamp_unix: {}\nlocation: {}\npayload: {}\nbacktrace:\n{:?}\n---\n",
            timestamp, location, payload, backtrace
        );
        append_trace_event(
            &trace_home,
            "gui",
            "panic",
            "panic_hook",
            serde_json::json!({
                "location": location,
                "payload": payload,
            }),
        );
        eprintln!("{message}");
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log_path)
        {
            let _ = file.write_all(message.as_bytes());
        }
    }));
}

fn placeholder_session_tree(path: std::path::PathBuf, theme: UiTheme) -> SessionNode {
    SessionNode {
        kind: SessionNodeKind::Group,
        name: "sessions".to_string(),
        title: Some(match theme {
            UiTheme::ZedLight => "Sessions".to_string(),
            UiTheme::ZedDark => "Sessions".to_string(),
        }),
        document_kind: None::<WorkspaceDocumentKind>,
        group_kind: Some(WorkspaceGroupKind::Folder),
        path,
        children: Vec::new(),
        session_id: None,
        cwd: None,
    }
}

fn run_install_cli(context: &InstallContext) -> Result<()> {
    let args = std::env::args().skip(2).collect::<Vec<_>>();
    match args.as_slice() {
        [command] if command == "integrate" => {
            for note in refresh_desktop_integration(context)? {
                println!("{note}");
            }
            Ok(())
        }
        [command] if command == "state" => {
            println!("{}", serde_json::to_string_pretty(context)?);
            Ok(())
        }
        [command] if command == "self-update" => {
            if context.update_policy != UpdatePolicy::Auto {
                println!("self-update disabled for this install channel");
                return Ok(());
            }
            if let Some(update) = check_for_update(context)? {
                let next = install_release_update(context, &update)?;
                println!("installed {} at {}", update.version, next.display());
            } else {
                println!("already up to date");
            }
            Ok(())
        }
        _ => {
            eprintln!(
                "usage:\n  yggterm install integrate\n  yggterm install state\n  yggterm install self-update"
            );
            Ok(())
        }
    }
}

fn run_document_cli(store: &SessionStore, args: &[String]) -> Result<()> {
    match args {
        [command] if command == "list" || command == "ls" => {
            for document in store.list_documents()? {
                println!("{}\t{}", document.virtual_path, document.title);
            }
            Ok(())
        }
        [command, path] if command == "cat" => {
            if let Some(document) = store.load_document(path)? {
                print!("{}", document.body);
            }
            Ok(())
        }
        [command, path] if command == "write" => {
            let mut body = String::new();
            std::io::stdin().read_to_string(&mut body)?;
            store.save_document(path, None, &body)?;
            println!("saved {}", path);
            Ok(())
        }
        [command, path, title] if command == "write" => {
            let mut body = String::new();
            std::io::stdin().read_to_string(&mut body)?;
            store.save_document(path, Some(title), &body)?;
            println!("saved {}", path);
            Ok(())
        }
        _ => {
            eprintln!(
                "usage:\n  yggterm doc list\n  yggterm doc cat <virtual-path>\n  yggterm doc write <virtual-path> [title] < body.md"
            );
            Ok(())
        }
    }
}

fn run_server_smoke() -> Result<()> {
    let temp_home = std::env::temp_dir().join(format!(
        "yggterm-smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
    ));
    fs::create_dir_all(&temp_home)?;
    let endpoint = default_endpoint(&temp_home);
    let current_exe = std::env::current_exe()?;
    let mut command = Command::new(current_exe);
    command
        .arg("server")
        .arg("daemon")
        .env(ENV_YGGTERM_HOME, &temp_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    yggterm_platform::configure_background_service_command(&mut command);
    let mut child = command.spawn()?;

    let result = (|| -> Result<()> {
        for _ in 0..40 {
            if ping(&endpoint).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        ping(&endpoint)?;
        let runtime = status(&endpoint)?;
        let _ = start_local_session(&endpoint, SessionKind::Shell)?;
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        println!("server {} smoke ok", runtime.server_version);
        Ok(())
    })();

    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&temp_home);
    result
}

#[cfg(test)]
mod tests {
    use super::{
        BuiltinCliCommand, LinuxWindowProfileInput, SignalClientScope,
        classify_builtin_cli_command, compatible_signal_client_count,
        linux_window_profile_from_input, record_matches_executable, signal_client_instances_dir,
        signal_client_scope_matches, signal_parse_process_start_ticks_from_stat,
        signal_process_start_ticks,
    };
    use std::fs;
    use yggterm_server::ServerEndpoint;

    #[test]
    fn classify_builtin_cli_command_detects_help_and_snapshot() {
        assert_eq!(
            classify_builtin_cli_command(&["--help".to_string()]),
            Some(BuiltinCliCommand::MainHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string()]),
            Some(BuiltinCliCommand::ServerHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string(), "snapshot".to_string()]),
            Some(BuiltinCliCommand::ServerSnapshot)
        );
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string(), "--help".to_string()]),
            Some(BuiltinCliCommand::ServerHelp)
        );
    }

    #[test]
    fn linux_x11_window_profile_uses_native_shape_corners() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: false,
            display_present: true,
            gdk_backend_x11: false,
            kde_session: false,
            xrpd_session: false,
        });
        assert!(!profile.transparent);
        assert_eq!(profile.reason, "x11_native_shape_profile");
    }

    #[test]
    fn linux_gdk_x11_window_profile_overrides_wayland_env() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: true,
            display_present: true,
            gdk_backend_x11: true,
            kde_session: false,
            xrpd_session: false,
        });
        assert!(!profile.transparent);
        assert_eq!(profile.reason, "x11_native_shape_profile");
    }

    #[test]
    fn linux_kde_x11_window_profile_uses_transparent_corners() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: true,
            display_present: true,
            gdk_backend_x11: true,
            kde_session: true,
            xrpd_session: false,
        });
        assert!(profile.transparent);
        assert_eq!(profile.reason, "kde_x11_transparent_profile");
    }

    #[test]
    fn linux_xrdp_window_profile_stays_opaque() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: false,
            display_present: true,
            gdk_backend_x11: true,
            kde_session: true,
            xrpd_session: true,
        });
        assert!(!profile.transparent);
        assert_eq!(profile.reason, "xrdp_opaque_profile");
    }

    #[test]
    fn linux_wayland_window_profile_stays_opaque_by_default() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: true,
            display_present: true,
            gdk_backend_x11: false,
            kde_session: false,
            xrpd_session: false,
        });
        assert!(!profile.transparent);
        assert_eq!(profile.reason, "wayland_opaque_default");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_kde_wayland_defaults_to_x11_backend_when_xwayland_exists() {
        use super::{LinuxDesktopBackendPolicyInput, linux_desktop_backend_policy_from_input};

        let policy = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
            allow_wayland_backend: false,
            gdk_backend_set: false,
            winit_backend_set: false,
            kde_session: true,
            wayland_display_present: true,
            display_present: true,
        });
        assert!(policy.force_x11_backend);
        assert!(policy.set_gdk_backend);
        assert!(policy.set_winit_backend);
        assert_eq!(policy.reason, "kde_wayland_x11_default");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_kde_wayland_backend_policy_respects_explicit_env() {
        use super::{LinuxDesktopBackendPolicyInput, linux_desktop_backend_policy_from_input};

        let explicit_gdk =
            linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
                allow_wayland_backend: false,
                gdk_backend_set: true,
                winit_backend_set: false,
                kde_session: true,
                wayland_display_present: true,
                display_present: true,
            });
        assert!(!explicit_gdk.force_x11_backend);
        assert_eq!(explicit_gdk.reason, "gdk_backend_explicit");

        let opt_in = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
            allow_wayland_backend: true,
            gdk_backend_set: false,
            winit_backend_set: false,
            kde_session: true,
            wayland_display_present: true,
            display_present: true,
        });
        assert!(!opt_in.force_x11_backend);
        assert_eq!(opt_in.reason, "wayland_backend_explicitly_allowed");
    }

    #[cfg(unix)]
    #[test]
    fn signal_parse_process_start_ticks_from_stat_reads_field_22() {
        let stat = "1234 (yggterm) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 515151";
        assert_eq!(
            signal_parse_process_start_ticks_from_stat(stat),
            Some(515151)
        );
    }

    #[test]
    fn signal_client_scope_rejects_different_x11_display() {
        let current = SignalClientScope {
            display: Some(":10.0".to_string()),
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: Some("/run/user/1000/gdm/Xauthority".to_string()),
        };
        let hidden = SignalClientScope {
            display: Some(":99".to_string()),
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: Some("/tmp/xvfb-run.ABC/Xauthority".to_string()),
        };
        assert!(!signal_client_scope_matches(&hidden, &current));
    }

    #[test]
    fn signal_client_scope_accepts_same_x11_display() {
        let current = SignalClientScope {
            display: Some(":10.0".to_string()),
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: Some("/run/user/1000/gdm/Xauthority".to_string()),
        };
        let same = current.clone();
        assert!(signal_client_scope_matches(&same, &current));
    }

    #[cfg(unix)]
    #[test]
    fn compatible_signal_client_count_scans_legacy_scope_dirs() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-signal-client-home-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_millis()
        ));
        let current_endpoint = ServerEndpoint::UnixSocket(home.join("server-2-1-5.sock"));
        let legacy_endpoint = ServerEndpoint::UnixSocket(home.join("server-2-1-4.sock"));
        let legacy_dir = signal_client_instances_dir(&home, &legacy_endpoint);
        fs::create_dir_all(&legacy_dir).expect("create legacy dir");
        fs::write(
            legacy_dir.join(format!("{}-1.json", std::process::id())),
            serde_json::json!({
                "pid": std::process::id(),
                "process_start_ticks": signal_process_start_ticks(std::process::id()),
            })
            .to_string(),
        )
        .expect("write live record");

        let live =
            compatible_signal_client_count(&home, &current_endpoint).expect("count live clients");
        assert_eq!(live, 1);

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn record_matches_executable_requires_same_path() {
        let current = std::env::current_exe().expect("current exe");
        let current_text = current.to_string_lossy().to_string();
        assert!(record_matches_executable(Some(&current_text), &current));
        assert!(!record_matches_executable(
            Some("/tmp/not-yggterm"),
            &current
        ));
        assert!(!record_matches_executable(None, &current));
    }
}
