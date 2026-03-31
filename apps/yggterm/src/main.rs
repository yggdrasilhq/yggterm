use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use yggterm_core::{
    ENV_YGGTERM_DIRECT_INSTALL_ROOT, ENV_YGGTERM_HOME, InstallContext, PerfSpan, SessionNode,
    SessionNodeKind, SessionStore, UiTheme, UpdatePolicy, WorkspaceDocumentKind,
    WorkspaceGroupKind, append_trace_event, check_for_update, current_version,
    detect_install_context, install_release_update, refresh_desktop_integration,
};
use yggterm_server::{
    AppControlViewMode, PersistedDaemonState, SessionKind, YggtermServer, cleanup_legacy_daemons,
    default_endpoint, detect_ghostty_host, ping, run_app_control_create_terminal,
    run_app_control_describe_rows, run_app_control_describe_state, run_app_control_drag,
    run_app_control_dump_state, run_app_control_focus_window, run_app_control_open_path,
    run_app_control_remove_session, run_app_control_scroll_preview,
    run_app_control_send_terminal_input, run_app_control_set_fullscreen,
    run_app_control_set_row_expanded, run_attach, run_daemon, run_remote_generation_context,
    run_remote_preview, run_remote_protocol_version, run_remote_resume_codex, run_remote_scan,
    run_remote_stage_clipboard_png, run_remote_terminate_codex, run_remote_upsert_generated_copy,
    run_screenrecord_capture, run_screenshot_capture, run_trace_bundle, run_trace_follow,
    run_trace_tail, shutdown, start_local_session, status,
};

const DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT_ENV: &str =
    "YGGTERM_DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT";
const ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF: &str = "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF";
const ENV_YGGTERM_ENABLE_ACCESSIBILITY: &str = "YGGTERM_ENABLE_ACCESSIBILITY";
const ENV_YGGTERM_ENABLE_WEBKIT_COMPOSITING: &str = "YGGTERM_ENABLE_WEBKIT_COMPOSITING";
const ENV_YGGTERM_ALLOW_MULTI_WINDOW: &str = "YGGTERM_ALLOW_MULTI_WINDOW";
const ENV_YGGTERM_ENABLE_TRANSPARENT_WINDOW: &str = "YGGTERM_ENABLE_TRANSPARENT_WINDOW";

fn main() -> Result<()> {
    configure_linux_accessibility_bridge();
    configure_linux_webkit_compositing();
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let current_exe = std::env::current_exe()?;
    let install_context = detect_install_context(&current_exe)?;
    maybe_handoff_to_preferred_executable(&current_exe, &args, &install_context)?;
    let store = SessionStore::open_or_init()?;
    install_panic_logging(store.home_dir());
    let startup_home = store.home_dir().to_path_buf();
    maybe_focus_existing_client(store.home_dir(), &args)?;
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "main_enter",
        serde_json::json!({ "args": args.clone() }),
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
                    other => anyhow::bail!("unsupported app preview action: {other}"),
                }
            }
            "expand" | "collapse" => {
                let row_path = args
                    .iter()
                    .skip(3)
                    .find(|value| !value.starts_with("--"))
                    .map(String::as_str)
                    .context("missing row path for server app expand/collapse")?;
                run_app_control_set_row_expanded(row_path, args[2] == "expand", timeout_ms)
            }
            "focus" => run_app_control_focus_window(timeout_ms),
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
                        run_app_control_create_terminal(machine_key, cwd, title_hint, timeout_ms)
                    }
                    "send" => {
                        let session_path = args
                            .iter()
                            .skip(4)
                            .find(|value| !value.starts_with("--"))
                            .map(String::as_str)
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
                        let session_path = args
                            .iter()
                            .skip(4)
                            .find(|value| !value.starts_with("--"))
                            .map(String::as_str)
                            .context("missing session path for server app session remove")?;
                        run_app_control_remove_session(session_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app session action: {other}"),
                }
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
    if args.len() == 4 && args[0] == "server" && args[1] == "remote" && args[2] == "terminate-codex"
    {
        return run_remote_terminate_codex(&args[3]);
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
    if args.as_slice() == ["server", "smoke"] {
        return run_server_smoke();
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
    let cleanup_span = PerfSpan::start(&startup_home, "startup", "cleanup_legacy_daemons");
    let _ = cleanup_legacy_daemons(&endpoint, &current_exe);
    cleanup_span.finish(serde_json::json!({}));
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
    let initial_server_snapshot = load_initial_server_snapshot_fast(
        &store,
        &browser_tree,
        prefer_ghostty_backend,
        &host,
        theme,
    );
    initial_server_sync_span.finish(serde_json::json!({
        "loaded": initial_server_snapshot.is_some(),
        "mode": "cached_snapshot_only",
    }));
    let server_daemon_detail = if initial_server_snapshot.is_some() {
        "warming server in background".to_string()
    } else {
        "no cached server snapshot".to_string()
    };

    let launch_result = yggui::launch_shell(yggui::ShellBootstrap {
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
        let xrpd_session = std::env::var_os("XRDP_SESSION").is_some()
            || std::env::var_os("XRDP_SOCKET_PATH").is_some();
        if transparent_opt_in {
            return LinuxWindowProfile {
                transparent: true,
                xrpd_session,
                reason: "explicit_opt_in",
            };
        }
        if xrpd_session {
            return LinuxWindowProfile {
                transparent: false,
                xrpd_session: true,
                reason: "xrdp_safe_mode",
            };
        }
        return LinuxWindowProfile {
            transparent: true,
            xrpd_session: false,
            reason: "default_linux_profile",
        };
    }

    #[cfg(not(target_os = "linux"))]
    {
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

fn load_initial_server_snapshot_fast(
    store: &SessionStore,
    browser_tree: &SessionNode,
    prefer_ghostty_backend: bool,
    host: &yggterm_server::GhosttyHostSupport,
    theme: UiTheme,
) -> Option<yggterm_server::ServerUiSnapshot> {
    if std::env::var(DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT_ENV)
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        return None;
    }
    let state_path = store.home_dir().join("server-state.json");
    let saved = fs::read_to_string(&state_path)
        .ok()
        .and_then(|json| serde_json::from_str::<PersistedDaemonState>(&json).ok())?;
    let mut server = YggtermServer::new(browser_tree, prefer_ghostty_backend, host.clone(), theme);
    server.restore_persisted_state(saved, Some(store));
    Some(server.snapshot())
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

fn maybe_focus_existing_client(home_dir: &std::path::Path, args: &[String]) -> Result<()> {
    if !args.is_empty()
        || std::env::var_os(ENV_YGGTERM_ALLOW_MULTI_WINDOW).is_some()
        || std::env::var_os(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF).is_some()
    {
        return Ok(());
    }
    let endpoint = default_endpoint(home_dir);
    if live_signal_client_count(home_dir, &endpoint)? == 0 {
        return Ok(());
    }
    if run_app_control_focus_window(3_000).is_ok() {
        std::process::exit(0);
    }
    Ok(())
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

#[cfg(not(unix))]
fn signal_process_is_alive(pid: u32) -> bool {
    pid != 0
}

fn unregister_signal_client_instance(
    home_dir: &std::path::Path,
    endpoint: &yggterm_server::ServerEndpoint,
) -> Result<usize> {
    let dir = signal_client_instances_dir(home_dir, endpoint);
    fs::create_dir_all(&dir)?;
    let current_pid = std::process::id();
    let entries = fs::read_dir(&dir)
        .with_context(|| format!("reading client instances {}", dir.display()))?;
    let mut remaining = 0_usize;
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
        if signal_process_is_alive(pid) {
            remaining += 1;
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(remaining)
}

fn live_signal_client_count(
    home_dir: &std::path::Path,
    endpoint: &yggterm_server::ServerEndpoint,
) -> Result<usize> {
    let dir = signal_client_instances_dir(home_dir, endpoint);
    fs::create_dir_all(&dir)?;
    let entries = fs::read_dir(&dir)
        .with_context(|| format!("reading client instances {}", dir.display()))?;
    let mut live = 0_usize;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(pid) = signal_parse_client_pid(&path) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        if signal_process_is_alive(pid) {
            live += 1;
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(live)
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
    let mut child = Command::new(current_exe)
        .arg("server")
        .arg("daemon")
        .env(ENV_YGGTERM_HOME, &temp_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

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
