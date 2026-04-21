use anyhow::Result;
use yggterm_core::{SessionStore, detect_install_context, refresh_desktop_integration};
use yggterm_server::{
    AppControlRightPanelMode, AppControlViewMode, cleanup_legacy_daemons, default_endpoint,
    detect_ghostty_host, ensure_local_daemon_running, ping, run_app_control_create_terminal,
    run_app_control_describe_rows, run_app_control_describe_state, run_app_control_drag,
    run_app_control_dump_state, run_app_control_focus_window, run_app_control_open_path,
    run_app_control_paste_terminal_clipboard_image, run_app_control_remove_session,
    run_app_control_scroll_preview, run_app_control_send_terminal_input,
    run_app_control_set_fullscreen, run_app_control_set_main_zoom,
    run_app_control_set_right_panel_mode, run_app_control_set_row_expanded,
    run_app_control_set_search, run_attach, run_daemon, run_screenrecord_capture,
    run_screenshot_capture, run_trace_bundle, run_trace_follow, run_trace_tail, shutdown, snapshot,
    status, try_run_remote_server_command,
};

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
  yggterm-headless
  yggterm-headless --help
  yggterm-headless server <subcommand>

common server commands:
  yggterm-headless server daemon
  yggterm-headless server status
  yggterm-headless server snapshot
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
  yggterm-headless server trace <tail|follow|bundle>
  yggterm-headless server screenshot <target> [output]
  yggterm-headless server screenrecord <target> [output]
  yggterm-headless server app <subcommand>"
    );
}

fn ensure_local_server_ready_for_cli(store: &SessionStore) -> Result<()> {
    let endpoint = default_endpoint(store.home_dir());
    ensure_local_daemon_running(&endpoint)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let current_exe = std::env::current_exe()?;
    let install_context = detect_install_context(&current_exe)?;
    let store = SessionStore::open_or_init()?;

    if args.as_slice() == ["server", "daemon"] {
        let _ = refresh_desktop_integration(&install_context);
        let endpoint = default_endpoint(store.home_dir());
        let _ = cleanup_legacy_daemons(&endpoint, &current_exe);
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
                let output_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--") && *value != target)
                    .map(String::as_str);
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
                let output_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str);
                run_screenrecord_capture("app", output_path, timeout_ms, duration_secs)
            }
            "state" => run_app_control_describe_state(timeout_ms),
            "dump" => {
                let output_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing output path for server app dump"))?;
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
                    .ok_or_else(|| anyhow::anyhow!("missing --value for server app zoom"))?;
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
                let row_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!("missing row path for server app expand/collapse")
                    })?;
                run_app_control_set_row_expanded(row_path, args[2] == "expand", timeout_ms)
            }
            "focus" => run_app_control_focus_window(timeout_ms),
            "search" => {
                let action = args.get(3).map(String::as_str).unwrap_or("set");
                match action {
                    "set" => {
                        let query = args
                            .iter()
                            .skip(4)
                            .find(|value| !value.starts_with("--"))
                            .map(String::as_str)
                            .ok_or_else(|| {
                                anyhow::anyhow!("missing query for server app search set")
                            })?;
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
                let mode = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str)
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
            "fullscreen" => {
                let action = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str)
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
            "open" => {
                let session_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing session path for server app open"))?;
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
                    .ok_or_else(|| anyhow::anyhow!("missing action for server app drag"))?;
                let row_path = args
                    .iter()
                    .skip(4)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str);
                let placement = args.windows(2).find_map(|window| {
                    if window[0] == "--placement" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                });
                run_app_control_drag(action, row_path, placement, timeout_ms)
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
                        run_app_control_create_terminal(machine_key, cwd, title_hint, timeout_ms)
                    }
                    "send" => {
                        let session_path = args
                            .iter()
                            .skip(4)
                            .find(|value| !value.starts_with("--"))
                            .map(String::as_str)
                            .ok_or_else(|| {
                                anyhow::anyhow!("missing session path for server app terminal send")
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
                                anyhow::anyhow!("missing --data for server app terminal send")
                            })?;
                        run_app_control_send_terminal_input(session_path, data, timeout_ms)
                    }
                    "paste-image" => {
                        let session_path = args
                            .iter()
                            .skip(4)
                            .find(|value| !value.starts_with("--"))
                            .map(String::as_str)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app terminal paste-image"
                                )
                            })?;
                        run_app_control_paste_terminal_clipboard_image(session_path, timeout_ms)
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
                        let session_path = args
                            .iter()
                            .skip(4)
                            .find(|value| !value.starts_with("--"))
                            .map(String::as_str)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "missing session path for server app session remove"
                                )
                            })?;
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
    if args.first().is_some_and(|arg| arg == "server") {
        anyhow::bail!(
            "unsupported server command: {}",
            args.get(1).map(String::as_str).unwrap_or("<missing>")
        );
    }

    anyhow::bail!("yggterm-headless only supports server subcommands");
}
