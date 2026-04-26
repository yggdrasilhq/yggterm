use anyhow::{Context, Result};
use yggterm_core::{SessionStore, detect_install_context};
use yggterm_server::{
    AppControlRightPanelMode, AppControlViewMode, cleanup_legacy_daemons, default_endpoint,
    detect_ghostty_host, ensure_local_daemon_running, ping, run_app_control_background_window,
    run_app_control_close_window, run_app_control_create_terminal, run_app_control_describe_rows,
    run_app_control_describe_state, run_app_control_drag, run_app_control_dump_state,
    run_app_control_focus_window, run_app_control_list_clients, run_app_control_move_window_by,
    run_app_control_open_path, run_app_control_paste_terminal_clipboard,
    run_app_control_paste_terminal_clipboard_image, run_app_control_remove_session,
    run_app_control_scroll_preview, run_app_control_send_terminal_input,
    run_app_control_set_clipboard_png_base64, run_app_control_set_clipboard_text,
    run_app_control_set_fullscreen, run_app_control_set_main_zoom,
    run_app_control_set_right_panel_mode, run_app_control_set_row_expanded,
    run_app_control_set_search, run_app_control_set_session_keep_alive,
    run_app_control_set_window_chrome_hover, run_attach, run_daemon, run_screenrecord_capture,
    run_screenshot_capture, run_trace_bundle, run_trace_follow, run_trace_tail, shutdown, snapshot,
    status, try_run_remote_server_command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinCliCommand {
    MainHelp,
    Version,
    ServerHelp,
    ServerSnapshot,
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
    let _install_context = detect_install_context(&current_exe)?;
    let store = SessionStore::open_or_init()?;

    if args.as_slice() == ["server", "daemon"] {
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
            BuiltinCliCommand::Version => {
                println!("{}", env!("CARGO_PKG_VERSION"));
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
            "clients" => run_app_control_list_clients(),
            "state" => run_app_control_describe_state(timeout_ms),
            "dump" => {
                let output_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
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

    anyhow::bail!("this yggterm build only supports server subcommands");
}

#[cfg(test)]
mod tests {
    use super::cli_positional_args;

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
}
