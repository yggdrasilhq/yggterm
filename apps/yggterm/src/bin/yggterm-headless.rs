use anyhow::Result;
use yggterm_core::{SessionStore, detect_install_context, refresh_desktop_integration};
use yggterm_server::{
    AppControlViewMode, cleanup_legacy_daemons, default_endpoint, detect_ghostty_host, ping,
    run_app_control_describe_rows, run_app_control_describe_state, run_app_control_drag,
    run_app_control_focus_window, run_app_control_open_path, run_attach, run_daemon,
    run_remote_ensure_managed_cli, run_remote_generation_context, run_remote_preview,
    run_remote_protocol_version, run_remote_refresh_managed_cli, run_remote_resume_codex,
    run_remote_scan, run_remote_stage_clipboard_png, run_remote_upsert_generated_copy,
    run_screenshot_capture, run_trace_bundle, run_trace_follow, run_trace_tail, shutdown, status,
};

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
    if args.len() == 3 && args[0] == "server" && args[1] == "attach" {
        return run_attach(&args[2]);
    }
    if args.as_slice() == ["server", "remote", "stage-clipboard-png"] {
        return run_remote_stage_clipboard_png();
    }
    if args.as_slice() == ["server", "remote", "protocol-version"] {
        return run_remote_protocol_version();
    }
    if args.len() >= 4 && args[0] == "server" && args[1] == "remote" && args[2] == "resume-codex" {
        return run_remote_resume_codex(
            &args[3],
            args.get(4)
                .map(String::as_str)
                .filter(|value| !value.is_empty()),
        );
    }
    if args.len() >= 4
        && args[0] == "server"
        && args[1] == "remote"
        && args[2] == "refresh-managed-cli"
    {
        return run_remote_refresh_managed_cli(args[3] == "background");
    }
    if args.len() >= 4
        && args[0] == "server"
        && args[1] == "remote"
        && args[2] == "ensure-managed-cli"
    {
        let tool = match args[3].as_str() {
            "codex" => yggterm_server::ManagedCliTool::Codex,
            "codex-litellm" => yggterm_server::ManagedCliTool::CodexLiteLlm,
            other => anyhow::bail!("unknown managed cli tool: {other}"),
        };
        return run_remote_ensure_managed_cli(tool);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "remote" && args[2] == "scan" {
        return run_remote_scan(args.get(3).map(String::as_str));
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
                let output_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str);
                run_screenshot_capture("app", output_path, timeout_ms)
            }
            "state" => run_app_control_describe_state(timeout_ms),
            "rows" => run_app_control_describe_rows(timeout_ms),
            "focus" => run_app_control_focus_window(timeout_ms),
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
            other => anyhow::bail!("unsupported app control command: {other}"),
        };
    }
    if args.len() == 4 && args[0] == "server" && args[1] == "remote" && args[2] == "preview" {
        return run_remote_preview(&args[3]);
    }
    if args.len() == 4
        && args[0] == "server"
        && args[1] == "remote"
        && args[2] == "generation-context"
    {
        return run_remote_generation_context(&args[3]);
    }
    if args.len() == 4
        && args[0] == "server"
        && args[1] == "remote"
        && args[2] == "upsert-generated-copy"
    {
        return run_remote_upsert_generated_copy(&args[3]);
    }
    if args.as_slice() == ["server", "shutdown"] {
        let endpoint = default_endpoint(store.home_dir());
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
        }
        return Ok(());
    }
    if args.as_slice() == ["server", "ping"] {
        let endpoint = default_endpoint(store.home_dir());
        ping(&endpoint)?;
        println!("pong");
        return Ok(());
    }
    if args.as_slice() == ["server", "status"] {
        let endpoint = default_endpoint(store.home_dir());
        let runtime = status(&endpoint)?;
        println!("{}", serde_json::to_string_pretty(&runtime)?);
        return Ok(());
    }

    anyhow::bail!("yggterm-headless only supports server subcommands");
}
