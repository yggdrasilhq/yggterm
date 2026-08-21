#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

/// The NATIVE notification audio path (`server app audio`). Not the webview:
/// WebKitGTK's autoplay gate streams silent samples without a user gesture,
/// which an agent cannot produce.
mod build_identity;
mod supervisor;

use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
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
use yggterm_server::server_cli::{cli_server_endpoint, ensure_local_server_ready_for_cli};
use yggterm_server::{
    AppControlPreviewLayout, AppControlRightPanelMode, AppControlViewMode, ClientInstanceRecord,
    PersistedDaemonState, ProbeTerminalViewportInputMode, ScreenshotPostProcess, SessionKind,
    WorkspaceViewMode, YggtermServer, active_client_instance_records,
    control_endpoint_for_runtime_key, default_endpoint,
    detect_ghostty_host, ensure_local_daemon_running, focus_live_with_view,
    local_headless_companion_executable_from_current, open_remote_session_with_view,
    open_stored_session_with_view, ping, reorder_live_sessions_scoped,
    resolve_client_daemon_endpoint, row_order_ledger_report, run_app_control_background_window,
    run_app_control_close_window, run_app_control_close_window_preserving_sessions,
    run_app_control_create_terminal_with_tenancy, run_app_control_describe_rows,
    run_app_control_reorder_sessions,
    run_app_control_describe_state,
    run_app_control_desktop_identity, run_app_control_dom_eval, run_app_control_drag,
    app_control_focus_window_took_focus, run_app_control_dump_state,
    run_app_control_focus_window,
    run_app_control_grid, run_app_control_key, run_app_control_list_clients,
    run_app_control_memory_profile,
    run_app_control_move_window_by, run_app_control_launch_app, run_app_control_open_path,
    run_app_control_paste_terminal_clipboard, run_app_control_paste_terminal_clipboard_image,
    run_app_control_pointer, run_app_control_probe_terminal_context_menu,
    run_app_control_probe_terminal_primary_selection_paste,
    run_app_control_probe_terminal_viewport_input, run_app_control_probe_terminal_viewport_scroll,
    run_app_control_probe_terminal_viewport_select, run_app_control_read_terminal_buffer,
    run_app_control_reclaim_terminal_focus, run_app_control_redraw_terminal,
    run_app_control_remove_session, run_app_control_rename_session,
    run_app_control_reset_theme_editor, run_app_control_resize_window,
    run_app_control_restart_pending_update, run_app_control_restart_session,
    run_app_control_scroll_preview, run_app_control_scroll_right_panel,
    run_app_control_scroll_terminal_viewport, run_app_control_send_terminal_input,
    run_app_control_set_clipboard_png_base64, run_app_control_set_clipboard_text,
    run_app_control_set_force_foreground, run_app_control_set_fullscreen,
    run_app_control_set_main_zoom, run_app_control_set_maximized,
    run_app_control_app_pane_action, run_app_control_set_preview_layout,
    run_app_control_set_right_panel_mode,
    run_app_control_set_row_expanded, run_app_control_set_search,
    run_app_control_set_launch_flags, run_app_control_set_session_keep_alive,
    run_app_control_set_theme_editor_open,
    run_app_control_set_theme_editor_values, run_app_control_set_tree_selection,
    run_app_control_set_ui_theme, run_app_control_set_window_chrome_hover,
    run_app_control_start_action, run_app_control_check_terminal_input, run_app_control_submit_terminal_prompt,
    run_app_control_trigger_update_check, run_attach, run_daemon,
    run_screenrecord_capture, run_screenshot_capture, run_screenshot_capture_with_post_process,
    parse_trace_limit, parse_trace_poll_ms,
    run_trace_bundle, run_trace_follow, run_trace_tail, run_trace_transitions, shutdown, snapshot,
    start_local_session, status, terminal_history, terminal_resize, terminal_restart,
    terminal_retained_snapshot, terminal_snapshot, terminal_write, try_run_remote_server_command,
};
use yggterm_shell::{
    ShellBootstrap, launch_shell, start_daemon_watchdog, terminal_identity_appearance_for_settings,
    warm_daemon_start,
};
use yggui_contract::UiTheme;

const DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT_ENV: &str =
    "YGGTERM_DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT";
const ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF: &str = "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF";
const ENV_YGGTERM_ENABLE_ACCESSIBILITY: &str = "YGGTERM_ENABLE_ACCESSIBILITY";
const ENV_YGGTERM_ALLOW_WAYLAND_BACKEND: &str = "YGGTERM_ALLOW_WAYLAND_BACKEND";
const ENV_YGGTERM_FORCE_X11_BACKEND: &str = "YGGTERM_FORCE_X11_BACKEND";
const ENV_YGGTERM_ENABLE_XTERM_CANVAS: &str = "YGGTERM_ENABLE_XTERM_CANVAS";
const ENV_YGGTERM_ENABLE_WEBKIT_COMPOSITING: &str = "YGGTERM_ENABLE_WEBKIT_COMPOSITING";
/// Force the software rasterizer regardless of what the host reports. The FORCE half
/// of the allow/force pair (`ALLOW_WAYLAND_BACKEND` / `FORCE_X11_BACKEND` is the
/// precedent), and force beats allow: a host whose GPU is genuinely broken, or whose
/// probe is wrong, gets back to the old behaviour with one env var and no rebuild.
const ENV_YGGTERM_FORCE_SOFTWARE_GL: &str = "YGGTERM_FORCE_SOFTWARE_GL";
const ENV_YGGTERM_ALLOW_MULTI_WINDOW: &str = "YGGTERM_ALLOW_MULTI_WINDOW";
const ENV_YGGTERM_ENABLE_TRANSPARENT_WINDOW: &str = "YGGTERM_ENABLE_TRANSPARENT_WINDOW";
const ENV_YGGTERM_WEBKIT_CACHE_MODEL: &str = "YGGTERM_WEBKIT_CACHE_MODEL";
const ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB: &str = "YGGTERM_WEBKIT_MEMORY_LIMIT_MB";
/// Opt IN to bounding the whole yggterm process family with a private systemd
/// user scope. ⛔ **Default OFF, deliberately.** This is a launch-path change
/// whose failure mode is *the GUI does not start*, so it ships dark and is
/// turned on inside a window where somebody is watching.
const ENV_YGGTERM_MEMORY_SCOPE: &str = "YGGTERM_MEMORY_SCOPE";
/// Stamped on the re-executed child so it knows it is already inside the scope.
/// Without this the child would re-exec forever.
const ENV_YGGTERM_MEMORY_SCOPE_ACTIVE: &str = "YGGTERM_MEMORY_SCOPE_ACTIVE";
const ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD: &str =
    "YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD";
const ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD: &str = "YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD";
const ENV_YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC: &str = "YGGTERM_WEBKIT_MEMORY_POLL_INTERVAL_SEC";
const ENV_MALLOC_ARENA_MAX: &str = "MALLOC_ARENA_MAX";
const ENV_YGGTERM_RELAUNCH_AFTER_PID: &str = "YGGTERM_RELAUNCH_AFTER_PID";
const ENV_YGGTERM_RELAUNCH_WAIT_TIMEOUT_MS: &str = "YGGTERM_RELAUNCH_WAIT_TIMEOUT_MS";
const DEFAULT_RELAUNCH_WAIT_TIMEOUT_MS: u64 = 15_000;
/// App-control round-trip budget for an `automation …` verb.
///
/// Longer than the ordinary 15 s because the expensive call behind it is a
/// session CREATE, which on a remote machine includes an ssh handshake and a
/// managed-CLI check. A timeout here would be recorded as `spawn_failed` for a
/// session that then arrives anyway, leaving a row nothing in the store owns.
const AUTOMATION_APP_CONTROL_TIMEOUT_MS: u64 = 60_000;

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

fn app_control_launch_terminal_surface_ready(data: &serde_json::Value) -> bool {
    if data
        .get("active_view_mode")
        .and_then(serde_json::Value::as_str)
        != Some("Terminal")
    {
        return true;
    }
    let Some(surface) = data.get("active_terminal_surface") else {
        return false;
    };
    if surface
        .get("problem")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|problem| !problem.trim().is_empty())
    {
        return false;
    }
    let active_session_path = data
        .get("active_session_path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let host = data
        .get("terminal_hosts")
        .and_then(serde_json::Value::as_array)
        .and_then(|hosts| {
            hosts
                .iter()
                .find(|host| {
                    !active_session_path.is_empty()
                        && host.get("session_path").and_then(serde_json::Value::as_str)
                            == Some(active_session_path)
                })
                .or_else(|| {
                    hosts.iter().find(|host| {
                        host.get("effective_input_focus")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                    })
                })
                .or_else(|| hosts.first())
        });
    let Some(host) = host else {
        return surface
            .get("rendered")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    };
    let xterm_present = host
        .get("xterm_present")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let viewport_present = host
        .get("viewport_present")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let screen_present = host
        .get("screen_present")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let rows_present = host
        .get("rows_present")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let canvas_count = host
        .get("canvas_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let child_count = host
        .get("child_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    xterm_present
        && viewport_present
        && child_count > 0
        && (screen_present || rows_present || canvas_count > 0)
}

fn app_control_state_settled_for_launch(payload: &serde_json::Value) -> bool {
    let Some(data) = payload.get("data") else {
        return false;
    };
    let initial_sync_done = data
        .get("shell")
        .and_then(|shell| shell.get("needs_initial_server_sync"))
        .and_then(serde_json::Value::as_bool)
        == Some(false);
    if !initial_sync_done {
        return false;
    }
    let contract_clean = data
        .get("session_view_contract_violations")
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty);
    if !contract_clean {
        return false;
    }
    let Some(runtime_truth) = data.get("runtime_truth") else {
        return true;
    };
    let daemon_runtime_count = runtime_truth
        .get("daemon_runtime_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if daemon_runtime_count == 0 {
        return true;
    }
    let active_runtime_present = runtime_truth
        .get("active_runtime_present")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let live_row_count = runtime_truth
        .get("live_row_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    active_runtime_present && live_row_count > 0 && app_control_launch_terminal_surface_ready(data)
}

fn app_control_state_launch_summary(
    payload: &serde_json::Value,
    pid: u32,
) -> Option<serde_json::Value> {
    let data = payload.get("data")?;
    let dom = data.get("dom").cloned().unwrap_or(serde_json::Value::Null);
    let shell = data
        .get("shell")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let runtime_truth = data
        .get("runtime_truth")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some(serde_json::json!({
        "request_id": payload.get("request_id").cloned().unwrap_or(serde_json::Value::Null),
        "handled_by_pid": payload.get("handled_by_pid").cloned().unwrap_or(serde_json::Value::Null),
        "visible": app_control_state_visible_for_pid(payload, pid),
        "settled": app_control_state_settled_for_launch(payload),
        "active_session_path": data.get("active_session_path").cloned().unwrap_or(serde_json::Value::Null),
        "active_view_mode": data.get("active_view_mode").cloned().unwrap_or(serde_json::Value::Null),
        "active_terminal_surface": data
            .get("active_terminal_surface")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "session_view_contract_violations": data
            .get("session_view_contract_violations")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "runtime_truth": {
            "daemon_runtime_count": runtime_truth
                .get("daemon_runtime_count")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "daemon_runtime_keys": runtime_truth
                .get("daemon_runtime_keys")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "active_runtime_key": runtime_truth
                .get("active_runtime_key")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "active_runtime_present": runtime_truth
                .get("active_runtime_present")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "live_row_count": runtime_truth
                .get("live_row_count")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "snapshot_live_session_count": runtime_truth
                .get("snapshot_live_session_count")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        },
        "window": data.get("window").cloned().unwrap_or(serde_json::Value::Null),
        "client_instance": data.get("client_instance").cloned().unwrap_or(serde_json::Value::Null),
        "shell": {
            "needs_initial_server_sync": shell
                .get("needs_initial_server_sync")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "server_busy": shell
                .get("server_busy")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        },
        "dom": {
            "shell_root_count": dom.get("shell_root_count").cloned().unwrap_or(serde_json::Value::Null),
            "degraded_reason": dom.get("degraded_reason").cloned().unwrap_or(serde_json::Value::Null),
            "error": dom.get("error").cloned().unwrap_or(serde_json::Value::Null),
        },
    }))
}

fn app_control_launch_state_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.clamp(250, 4_000)
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

/// Where the file-descriptor soft limit is raised to at startup.
///
/// NOT the hard limit, which on this host is 1,048,576. A very high
/// `RLIMIT_NOFILE` is not free: a program that closes descriptors in a loop up
/// to the limit before `exec` pays for every one of them, and yggterm spawns ssh
/// children constantly (one per remote session, one per egress tunnel, one per
/// remote command). 64 Ki is far past anything the GUI can reach — the measured
/// baseline is 51 descriptors with one webview realized, and a webview costs
/// single digits — while staying in the range where nothing behaves oddly.
const FILE_DESCRIPTOR_SOFT_LIMIT_TARGET: u64 = 65_536;

/// What to raise `RLIMIT_NOFILE`'s SOFT limit to, given the current pair.
/// `None` = leave it alone.
///
/// The default soft limit is 1024 while the hard limit is 1,048,576, which is
/// the shape every browser raises at startup: the soft limit is an inherited
/// default, not a policy anyone chose. yggterm needs it for the same reason a
/// browser does — each realized webview brings IPC sockets to its web and
/// network processes plus (under DMABuf) imported buffer descriptors into the UI
/// process, so the ceiling arrives exactly where many-tab use starts.
///
/// Never LOWERS the limit: a soft limit already above the target was set
/// deliberately by whoever launched us, and stepping on that would be the same
/// mistake in the other direction.
fn raised_file_descriptor_soft_limit(soft: u64, hard: u64) -> Option<u64> {
    let target = FILE_DESCRIPTOR_SOFT_LIMIT_TARGET.min(hard);
    (target > soft).then_some(target)
}

/// Raise this process's file-descriptor soft limit toward its hard limit.
///
/// Observable after the fact at `/proc/<pid>/limits` ("Max open files"), which
/// is where the 1024/1048576 pair was measured on the live GUI in the first
/// place. Failure is non-fatal and traced: an unraised limit costs a ceiling,
/// not correctness.
#[cfg(target_os = "linux")]
fn configure_file_descriptor_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        tracing::debug!("file-descriptor limit: getrlimit failed, leaving it alone");
        return;
    }
    let Some(target) = raised_file_descriptor_soft_limit(limit.rlim_cur, limit.rlim_max) else {
        return;
    };
    let raised = libc::rlimit {
        rlim_cur: target,
        rlim_max: limit.rlim_max,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0 {
        tracing::info!(
            from = limit.rlim_cur,
            to = target,
            hard = limit.rlim_max,
            "raised file-descriptor soft limit"
        );
    } else {
        tracing::warn!(
            from = limit.rlim_cur,
            wanted = target,
            "raising the file-descriptor soft limit failed"
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_file_descriptor_limit() {}

/// THE positional-argument rule, shared with the `yggterm-headless` binary and
/// with the `server app web` dispatcher that reads the same argv — one
/// implementation, so a `--flag value` pair cannot be skipped on one entry
/// point and read as a positional on another. See [`yggterm_core::cli_args`].
fn cli_positional_args(args: &[String], start: usize) -> Vec<&str> {
    yggterm_core::cli_positional_args(args, start)
}

/// Apply `--client-role <active|shadow>` / `--client-id <name>` to this
/// process's daemon-client identity (slice 4.3).
///
/// A shadow view client declares itself here so the daemon's slice-4.0 role gate
/// refuses it any ownership-claiming request. Absent flags leave the process
/// anonymous = `Active`, which is what the user's GUI is.
fn apply_client_identity_args(args: &[String]) -> Result<()> {
    let role = match cli_flag_value(args, "--client-role") {
        Some(value) => yggterm_server::parse_client_role(value)?,
        None => yggterm_server::ClientRole::Active,
    };
    let client_id = cli_flag_value(args, "--client-id")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    // Nothing declared: stay anonymous so the wire is byte-identical.
    if role == yggterm_server::ClientRole::Active && client_id.is_none() {
        return Ok(());
    }
    yggterm_server::set_client_identity(yggterm_server::ClientIdentity { role, client_id });
    Ok(())
}

/// THE argv flag rule, shared with the `yggterm-headless` binary and with the
/// server-side parsers that read the same argv — one implementation, so
/// `--flag=value` cannot be honoured on one entry point and silently discarded
/// on another. See [`yggterm_core::cli_args`].
fn cli_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    yggterm_core::cli_flag_value(args, flag)
}

/// Refuse a payload that is really a flag. Adapter over
/// [`yggterm_core::refuse_flag_shaped_payload`]; see there for why the guard
/// exists at all (the LIE-OF-SUCCESS shape, not a wrong answer).
#[cfg(test)]
fn refuse_flag_shaped_payload<'a>(value: &'a str, what: &str) -> Result<&'a str> {
    yggterm_core::refuse_flag_shaped_payload(value, what).map_err(|error| anyhow::anyhow!(error))
}

/// Parse the agent-oriented screenshot post-process flags:
///   --region <terminal|full>   crop to the active terminal viewport
///   --crop <x,y,w,h>           explicit pixel crop
///   --scale <factor>           nearest-neighbour upscale (e.g. 2 or 2.5)
/// Returns None when none are present (so the plain capture path is used).
/// `--backend os` forces an OS-compositor grab of the window so NATIVE child
/// widgets (web-surface webviews) appear in the frame — the default composite/
/// DOM backends are blind to them. Any other value (or absent) keeps the
/// default backend selection.
fn screenshot_backend_is_compositor(args: &[String]) -> bool {
    cli_flag_value(args, "--backend")
        .map(|value| value.eq_ignore_ascii_case("os"))
        .unwrap_or(false)
}

fn parse_screenshot_post_process(args: &[String]) -> Option<ScreenshotPostProcess> {
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
    let grid = parse_screenshot_grid(args);
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

/// `--grid` (12x8 default) / `--grid COLSxROWS`, plus `--grid-refine CELL` to
/// subdivide one cell into a labelled 3x3. Shares its body with the headless
/// binary via `yggterm_server::screenshot_grid_from_args`.
fn parse_screenshot_grid(args: &[String]) -> Option<yggterm_server::GridSpec> {
    yggterm_server::grid_overlay::screenshot_grid_from_args(args)
}

fn launch_app_background(
    home_dir: &std::path::Path,
    timeout_ms: u64,
    wait_visible: bool,
    wait_settled: bool,
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
    // ⛔ Deliberately NOT supervised (see `supervisor`). This launcher polls for
    // a client record whose pid is the pid it SPAWNED, and under a supervisor
    // that pid belongs to the shim while the record belongs to the child — so
    // `--wait-visible` would wait forever and report `registered: false`. The
    // desktop entry is the path that matters for the reported gap (the user's
    // window vanishing on a segfault); an agent that launched a GUI can launch
    // it again. Supervising here needs the poll to match the child first.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .env(ENV_YGGTERM_HOME, home_dir);
    #[cfg(target_os = "linux")]
    {
        let current_env = linux_current_environment_map();
        let desktop_overrides = linux_gui_entry_environment_overrides_from_desktop(
            &current_env,
            discover_linux_desktop_environment(),
        );
        command.envs(desktop_overrides);
    }
    if allow_multi_window {
        command.env("YGGTERM_ALLOW_MULTI_WINDOW", "1");
    }
    if skip_active_exec_handoff {
        command.env("YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF", "1");
    }
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command
        .spawn()
        .with_context(|| format!("spawning background yggterm from {}", current_exe.display()))?;
    let pid = child.id();
    let mut client = None::<serde_json::Value>;
    let mut visibility = None::<serde_json::Value>;
    let mut visibility_error = None::<String>;
    let should_wait_for_app = wait_visible || wait_settled;
    if should_wait_for_app {
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms.max(100));
        let state_timeout_ms = app_control_launch_state_timeout_ms(timeout_ms);
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
                                if app_control_state_visible_for_pid(&payload, pid)
                                    && (!wait_settled
                                        || app_control_state_settled_for_launch(&payload))
                                {
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
            "wait_settled": wait_settled,
            "registered": client.is_some(),
            "visible": visibility
                .as_ref()
                .and_then(|value| value.get("visible"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            "settled": visibility
                .as_ref()
                .and_then(|value| value.get("settled"))
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
    ServerAppHelp,
    ServerSessionsHelp,
    ServerSnapshot,
}

/// `server app` subcommands that print their OWN help.
///
/// The generic `server app … --help` interception below runs BEFORE the app
/// dispatcher, so without this exception it swallows the deeper help and the
/// subcommand's help printer becomes dead code the user can never reach —
/// which is exactly what happened to `server app audio --help`.
fn server_app_subcommand_owns_its_help(subcommand: &str) -> bool {
    matches!(subcommand, "audio")
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
        [server, app] if server == "server" && app == "app" => {
            Some(BuiltinCliCommand::ServerAppHelp)
        }
        [server, app, rest @ ..]
            if server == "server"
                && app == "app"
                && !rest
                    .first()
                    .is_some_and(|sub| server_app_subcommand_owns_its_help(sub))
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
  yggterm
  yggterm --help
  yggterm --version
  yggterm --build-commit
  yggterm install
  yggterm doc <subcommand>
  yggterm server <subcommand>

common server commands:
  yggterm server daemon
  yggterm server status
  yggterm server snapshot
  yggterm server connect <session-path> | --list
  yggterm server order [--json]
  yggterm server reorder <session-path>... | --stdin [--scope <scope>]
  yggterm server ledger [--scope <scope>]
  yggterm server app <subcommand>"
    );
}

fn print_server_help() {
    println!(
        "usage:
  yggterm server daemon
  yggterm server attach <session> [cwd] [--allow-plain-shell-fallback]
  yggterm server connect <session-path> [--view terminal|preview] [--top|--after <path>]
  yggterm server connect --list
  yggterm server order [--json]
  yggterm server reorder <session-path>... | --stdin [--scope <scope>]
  yggterm server ledger [--scope <scope>]
  yggterm server daemons [--json]
  yggterm server write-lock <acquire|release|status> [--holder <who>]
  yggterm server screen [<session-key>] [--state|--state-only] [--json]
    a row's screen as PLAIN DECODED TEXT on stdout — one line per visible row,
    no JSON envelope, nothing to unwrap. This is the only check that can tell a
    DELIVERED brief from one QUEUED behind a modal: every other instrument is
    downstream of the write, and a pty accepts bytes whether or not anything is
    consuming them. `--state` prefixes the row's state with its remedy and its
    prohibition; `--state-only` prints just the state token, for a spawn recipe
    to branch on. Read-only, never written to the trace.
  yggterm server gate-screen [<session-key>] [--tail <n>] [--json]
  yggterm server relay-boundary [--by <who>] [--wait-secs <n>] [--json]
  yggterm server wpe <verb> [--key value ...]
  yggterm server wpe agent <status|restart|stop>
  yggterm server ping
  yggterm server status
  yggterm server snapshot
  yggterm server shutdown
  yggterm server terminal write <session> (--data <data>|--stdin)
  yggterm server terminal screen <session> [--retained] [--raw] [--history]
  yggterm server terminal app-declares <session>
  yggterm server terminal resize <session> --cols <n> --rows <n>
  yggterm server terminal restart <session> [--terminal-appearance <dark|light>] [--force-remote]
  yggterm server sessions regenerate-copy [--budget <n>] [--force] [--reset-summary-history] [--json]
  yggterm server smoke
  yggterm server trace <tail|follow|bundle>
  yggterm server screenshot <target> [output] [--region terminal|full] [--crop x,y,w,h] [--scale n]
  yggterm server screenrecord <target> [output]
  yggterm server app <subcommand>"
    );
}


fn print_server_sessions_help() {
    println!(
        "usage:
  yggterm server sessions regenerate-copy [--budget <n>] [--force] [--reset-summary-history] [--json]

commands:
  regenerate-copy    Generate missing Codex session titles and summary timelines.

options:
  --budget <n>                Limit the number of sessions processed; 0 means no explicit limit.
  --force                     Regenerate existing generated copy.
  --reset-summary-history     Rebuild summary timeline history from scratch.
  --json                      Print a machine-readable report."
    );
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
    let report =
        store.regenerate_codex_session_copy(&settings, budget, force, reset_summary_history)?;
    if args.iter().any(|arg| arg == "--json") {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "scanned={} title_generated={} precis_generated={} summary_generated={} summary_history_reset={} skipped={} failed={}",
            report.scanned,
            report.title_generated,
            report.precis_generated,
            report.summary_generated,
            report.summary_history_reset,
            report.skipped,
            report.failed.len()
        );
        for failure in report.failed.iter().take(12) {
            println!(
                "failed {} {}: {}",
                failure.stage, failure.session_id, failure.error
            );
        }
    }
    Ok(())
}



fn main() -> Result<()> {
    // ⭐ BEFORE EVERYTHING, including the GL probe and the supervisor: resolve the
    // D-Bus session bus, because GLib autolaunches a PRIVATE one the moment
    // anything in this process touches GTK without an address to inherit, and
    // that bus plus its activated portal/secrets/a11y daemons then outlive us
    // forever. 4,574 MB across 243 such orphans on 43 buses was measured on the
    // live host (2026-07-30). Every child we spawn inherits this answer, which is
    // why it belongs here and not at the spawn sites — the sites that leaked were
    // exactly the ones nobody remembered.
    //
    // Must run before any thread exists (`set_var` is unsound afterwards) and
    // before GLib caches the address on first use.
    let _session_bus = yggterm_core::session_bus::adopt_or_refuse_session_bus();

    // Hand this build's identity to the library crates that answer questions
    // about a RUNNING process — the daemon's status and this window's client
    // record. `--build-commit` can only ever describe a file, and a deploy
    // replaces the file under a live process, so a process that does not say
    // what it is becomes unnameable the moment it matters.
    //
    // ⚠ AFTER the bus resolve, not before. It landed above it and turned the
    // `every_entry_point_refuses_autolaunch_before_it_can_happen` lock red:
    // GLib caches the D-Bus address on FIRST USE and `set_var` is unsound once
    // a thread exists, so "first statement in main()" is the whole guarantee.
    // Nothing here needs the identity declared first.
    yggterm_server::build_identity::declare_build_commit(build_identity::build_commit());

    let entry_args = std::env::args().skip(1).collect::<Vec<_>>();
    // FIRST, ahead of even the supervisor: this process may have been re-exec'd for
    // the sole purpose of dlopening libEGL and reporting what this host rasterizes
    // with. It owns no window, no store, no daemon connection and no threads — which
    // is the whole point, because a graphics driver that segfaults in here must cost
    // one line of stdout, not the user's window. Ordering matters: a probe must never
    // be able to nest inside a supervisor inside a probe.
    if yggterm_core::gl_probe::should_run_as_gl_probe(&entry_args) {
        std::process::exit(yggterm_core::gl_probe::run_gl_probe_child());
    }
    // Then, before the update-relaunch wait: in supervise mode this process owns no
    // window, no store and no daemon connection — it forks the real GUI and waits on
    // it, so that when the window dies from a SIGSEGV the user gets it back. See
    // `supervisor` for why the policy is on-abnormal and why the daemon could not do
    // this job.
    if supervisor::should_run_as_supervisor(&entry_args) {
        std::process::exit(supervisor::run_supervisor(&entry_args)?);
    }
    maybe_wait_for_update_relaunch_parent_exit();
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // Agent presence (cursor v1) — see the twin in the headless binary.
    yggterm_server::set_agent_identity(cli_flag_value(&args, "--agent"));
    // Daemon-client identity + role (slice 4.3). A DIFFERENT layer from
    // `--agent` above: that colours an agent's cursor, this decides whether this
    // process may take runtime ownership at all. Declared once here so every
    // outgoing daemon request carries it. An unparseable role is fatal rather
    // than a silent downgrade to Active (eng-review D7).
    apply_client_identity_args(&args)?;
    // Automations, on the GUI binary too. The generated unit invokes the
    // headless one, but an agent driving `yggterm` should not have to know
    // that a verb lives on the other binary — that is the mistake the whole
    // web verb plane made and had to be undone. ONE owner
    // (crates/yggterm-server/src/automation_cli.rs); do not inline a verb here.
    if args.first().is_some_and(|arg| arg == "automation") {
        return yggterm_server::run_automation_cli(&args, AUTOMATION_APP_CONTROL_TIMEOUT_MS);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "automation" {
        return yggterm_server::run_automation_cli(&args[1..], AUTOMATION_APP_CONTROL_TIMEOUT_MS);
    }
    // Collections — history organised into things worth keeping. Same shape as
    // automations above and ONE owner
    // (crates/yggterm-server/src/web_collection_cli.rs). Matched before the
    // daemon handshake because none of these verbs needs a daemon: a collection
    // is a Markdown file in the profile's own jar.
    // `snapshot` is matched at the TOP level only — `server snapshot` is the
    // daemon's own verb and must keep meaning that.
    if args.first().is_some_and(|arg| arg == "collection" || arg == "snapshot") {
        return yggterm_server::run_web_collection_cli(&args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "collection" {
        return yggterm_server::run_web_collection_cli(&args[1..]);
    }
    // Browser import — local file work, so it is matched before anything that
    // could need a daemon or a GUI. Same ONE-owner rule as the automation arm
    // above: route, never inline a verb.
    if args.first().is_some_and(|arg| arg == "web-import") {
        return yggterm_server::run_browser_import_cli(&args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "web-import" {
        return yggterm_server::run_browser_import_cli(&args[1..]);
    }
    #[cfg(target_os = "linux")]
    let memory_scope = if args.is_empty() {
        hydrate_linux_gui_entry_environment_from_desktop();
        // ⛔ ONLY the GUI, and only AFTER the desktop entry has been hydrated —
        // that is where the opt-in is set, so reading it earlier would miss it.
        // A CLI subcommand must never be re-executed into a scope: it would
        // bound a one-shot verb and leave a unit behind for every invocation.
        enter_memory_scope_if_requested()
    } else {
        MemoryScopeOutcome::NotAttempted
    };
    configure_linux_allocator_limits()?;
    // After the allocator re-exec above (which returns early), so the raise
    // lands on the process that actually runs the GUI.
    configure_file_descriptor_limit();
    configure_linux_desktop_backend();
    configure_linux_terminal_renderer_policy();
    configure_linux_accessibility_bridge();
    configure_linux_webkit_compositing();
    configure_linux_webkit_memory_policy();
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();

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
    if main_should_retire_superseded_clients_before_shell(&args) {
        maybe_retire_superseded_same_home_clients(store.home_dir(), &args, &current_exe)?;
    } else if args.is_empty() {
        append_trace_event(
            &startup_home,
            "gui",
            "startup",
            "main_superseded_retirement_deferred_to_shell_handoff",
            serde_json::json!({
                "reason": "shell captures outgoing active session before process retirement"
            }),
        );
    }
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "main_enter",
        serde_json::json!({ "args": args.clone() }),
    );
    // ⛔ WHETHER THIS GUI IS BOUNDED AT ALL, ON THE RECORD, EVERY LAUNCH.
    //
    // `/proc/<pid>/environ` cannot answer it and neither can the binary's
    // version: the cap is applied by a re-exec that can fail transiently, so two
    // processes running identical code differ in whether they are capped. The
    // only place that difference was previously visible was a cgroup file
    // somebody had to think to read.
    #[cfg(target_os = "linux")]
    append_trace_event(
        &startup_home,
        "gui",
        "startup",
        "linux_memory_scope",
        serde_json::json!({
            "outcome": memory_scope.label(),
            "bounded": memory_scope.bounded(),
            "inherited_unit": memory_scope.inherited_unit(),
            "fallback_reason": memory_scope.fallback_reason(),
        }),
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
            "xterm_canvas_renderer": std::env::var(ENV_YGGTERM_ENABLE_XTERM_CANVAS).ok(),
            "xterm_canvas_policy": std::env::var("YGGTERM_XTERM_CANVAS_POLICY").ok(),
            "wayland_display_present": std::env::var_os("WAYLAND_DISPLAY").is_some(),
            "display_present": std::env::var_os("DISPLAY").is_some(),
            // The GL decision and the three settings it owns. `configure_linux_webkit_compositing`
            // runs long before this trace exists, so it exports its reason and we read
            // it back — and the probe's own report is read from its OnceLock rather
            // than re-probed, so there is exactly one probe per process.
            "webkit_gl_policy": std::env::var(yggterm_core::gl_probe::ENV_YGGTERM_WEBKIT_GL_POLICY).ok(),
            "libgl_always_software": std::env::var("LIBGL_ALWAYS_SOFTWARE").ok(),
            "gallium_driver": std::env::var("GALLIUM_DRIVER").ok(),
            "webkit_disable_dmabuf_renderer": std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").ok(),
            "web_surface_under_glass": std::env::var("YGGTERM_WEB_SURFACE_UNDER_GLASS").ok(),
            "gl_probe_class": yggterm_core::gl_probe::gl_probe_report()
                .map(|report| report.class.as_str()),
            "gl_probe_driver": yggterm_core::gl_probe::gl_probe_report()
                .and_then(|report| report.driver.clone()),
            "gl_probe_renderer": yggterm_core::gl_probe::gl_probe_report()
                .and_then(|report| report.renderer.clone()),
            "gl_probe_reason": yggterm_core::gl_probe::gl_probe_report()
                .map(|report| report.reason.clone()),
            // What the probe COST, so a timeout budget that starts drifting is visible
            // rather than inferred.
            "gl_probe_elapsed_ms": yggterm_core::gl_probe::gl_probe_report()
                .map(|report| report.elapsed_ms),
        }),
    );
    let startup_span = PerfSpan::start(&startup_home, "startup", "gui_main");
    let pending_update_restart = None;
    let launch_install_context = install_context.clone();
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
            BuiltinCliCommand::ServerAppHelp => {
                yggterm_server::app_control_cli::print_server_app_help("yggterm");
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
    // ⛔ The forced command behind a phone's ssh key. See
    // `yggterm_server::daemon_bridge` for why a bare key is a supply-chain risk.
    if args.as_slice() == ["server", "daemon-bridge"] {
        return yggterm_server::daemon_bridge::run_daemon_bridge();
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "attach" {
        let (cwd, fallback) = yggterm_server::parse_attach_args(&args[3..]);
        return run_attach(&args[2], cwd.as_deref(), fallback);
    }
    // `yggterm server connect <session-path>|--list` — headless twin of clicking
    // a session row. Manually connect an existing-but-unconnected ("void")
    // session back into the live set + GUI: it sends the SAME daemon requests as
    // the GUI (FocusLive for a session the daemon already tracks, else
    // OpenRemoteSession for a scan-only remote), so the session becomes live and
    // its terminal is attached/resumed. Recovery tool for sessions stranded out
    // of Live Sessions (e.g. demoted by a restart). See [[project-purpose]].
    if args.len() >= 2 && args[0] == "server" && args[1] == "connect" {
        // ONE owner, both binaries — `yggterm_server::server_cli`. The ninth and
        // last `server` divergence; its verdict was accidental all along and only
        // its SIZE deferred it.
        return yggterm_server::server_cli::run_server_connect_cli(&store, &args);
    }
    // `yggterm server write-lock <acquire|hold|report|release> [--profile <name>]`
    // — drive the daemon-owned profile write-lock (slice 4.1/4.2) directly, to
    // prove Active-priority preemption and inspect who holds a jar. Identity
    // (Active|Shadow) is this process's --client-role/--client-id, already applied
    // above: `--client-role shadow --client-id s1 ... acquire` takes a PREEMPTIBLE
    // lock, and a later default (Active) `acquire` on the same profile PREEMPTS it
    // (`preempted_shadow`). The daemon reclaims a DEAD holder's lock, so a
    // short-lived `acquire` cannot be contended — use `hold`, which keeps the
    // process alive holding the lock (SIGTERM/Ctrl-C to release), to stand up a
    // live holder for a preemption test. See docs/agent-control-plane.md.
    if args.len() >= 2 && args[0] == "server" && args[1] == "write-lock" {
        // ONE owner, both binaries — `yggterm_server::server_cli`.
        return yggterm_server::server_cli::run_server_write_lock_cli(&store, &args);
    }
    // `yggterm server order [--json]` — dump the Live Sessions row order, one
    // path per line. Round-trips with `server reorder --stdin`, so an order can
    // always be captured before a disruptive operation and restored after:
    //   yggterm server order > order.txt
    //   yggterm server reorder --stdin < order.txt
    if args.len() >= 2 && args[0] == "server" && args[1] == "order" {
        // ONE owner, both binaries — `yggterm_server::server_cli`.
        return yggterm_server::server_cli::run_server_order_cli(&store, &args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "startpage" {
        return yggterm_server::server_cli::run_server_startpage_ls_cli(&store, &args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "titles" {
        return yggterm_server::server_cli::run_server_titles_ls_cli(&store, &args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "resume" {
        return yggterm_server::server_cli::run_server_resume_ls_cli(&store, &args);
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "cwdtree" {
        return yggterm_server::server_cli::run_server_cwdtree_ls_cli(&store, &args);
    }
    // `yggterm server ledger [--scope <scope>]` — dump the durable row-order
    // ledger (per-client-scope memory of row slots, including rows that are
    // not currently live). Read-only.
    // ⛔ THIS VERB ANSWERED ON THE HEADLESS BINARY ONLY, and the census it
    // reports is a host fact with no GUI in it — so `yggterm server daemons`
    // answering "unsupported server command: daemons" was an accident of which
    // file it was typed into, not a property of this binary. Both route to the
    // one owner now. ⚠ The `server` surface is still dispatched twice; the
    // triage of which of its other divergences are accidental and which are
    // real is filed in docs/pending-bugs.md.
    if args.first().is_some_and(|arg| arg == "server")
        && args.get(1).is_some_and(|arg| arg == "daemons")
    {
        return yggterm_server::run_server_daemons_census(
            store.home_dir(),
            args.iter().any(|arg| arg == "--json"),
        );
    }
    if args.len() >= 2 && args[0] == "server" && args[1] == "ledger" {
        // ONE owner, both binaries — `yggterm_server::server_cli`.
        return yggterm_server::server_cli::run_server_ledger_cli(&store, &args);
    }
    // `yggterm server reorder <path>... | --stdin [--scope <scope>]` — set the
    // Live Sessions row order. Paths are placed in the given order at the TOP;
    // any live row not listed keeps its relative position AFTER them (the
    // daemon appends the remainder), so a partial list is safe and never drops
    // a row. `--scope` also records the order into that client's row-order
    // ledger scope (multi-GUI arrangements).
    if args.len() >= 2 && args[0] == "server" && args[1] == "reorder" {
        // ONE owner, both binaries — `yggterm_server::server_cli`.
        return yggterm_server::server_cli::run_server_reorder_cli(&store, &args);
    }
    // ⛔ THE LAST THREE `server` DIVERGENCES, AND THEY WERE ACCIDENTAL TOO.
    // These three were read as deploy/relay machinery that belonged to the
    // headless CLI by design — the one real fork this surface was said to
    // have, and the reason it was not collapsed wholesale. Reading their
    // bodies refutes it: `wpe` opens with `ensure_local_server_ready_for_cli`
    // + `cli_server_endpoint` and talks to the daemon, which is the very test
    // that convicted the four before it; `gate-screen` is a read-only daemon
    // query; and `relay-boundary` reads and writes a file under the home dir
    // and touches no daemon at all, which is the `daemons` census's shape.
    // None of them needs a window, and none of them is unavailable to this
    // binary for any reason but which file it was typed into.
    // ⇒ A relay declaring its own hand-off is the case that stung: the binary
    // on an agent's PATH is this one.
    if args.len() >= 2 && args[0] == "server" && args[1] == "gate-screen" {
        return yggterm_server::server_cli::run_server_gate_screen_cli(&store, &args);
    }
    // ⛔ BOTH BINARIES, and pinned by a test. This dispatcher exists twice, and
    // a consolidation that touched only one copy has silently removed five
    // verbs before — the handlers and their tests stayed, so the verbs looked
    // present in the source and were absent from a built binary. Reachability
    // is the only property that notices.
    if args.len() >= 2 && args[0] == "server" && args[1] == "screen" {
        return yggterm_server::server_cli::run_server_screen_cli(&store, &args);
    }
    if args.first().is_some_and(|arg| arg == "server")
        && args.get(1).is_some_and(|arg| arg == "relay-boundary")
    {
        return yggterm_server::server_cli::run_server_relay_boundary_cli(&store, &args);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "wpe" {
        return yggterm_server::server_cli::run_server_wpe_cli(&store, &args[2..]);
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
    // Dump the daemon's vt100 screen for a session — the ground truth BEFORE
    // xterm.js renders it. Compare against the GUI's xterm buffer (app terminal
    // probe-scroll) to tell whether a blank/garbled viewport is a real session
    // problem or an xterm.js render/replay bug. `--retained` uses the full
    // scrollback snapshot; default is the current live screen.
    if args.len() >= 4
        && args[0] == "server"
        && args[1] == "terminal"
        && args[2] == "screen"
        && args.iter().any(|arg| arg == "--history")
    {
        // Diagnostic: dump the daemon's CLEAN scrolled-off vt100 scrollback rows for
        // a session (the history that CAN load into xterm scrollback). Read-only;
        // connect directly like the other terminal-screen reads.
        let endpoint = cli_server_endpoint(store.home_dir());
        let (rows, running) = terminal_history(&endpoint, &args[3])?;
        let nonblank = rows.iter().filter(|line| !line.trim().is_empty()).count();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "session_path": args[3],
                "running": running,
                "history_row_count": rows.len(),
                "nonblank_row_count": nonblank,
                "rows": rows,
            }))?
        );
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
    if args.len() >= 4 && args[0] == "server" && args[1] == "terminal" && args[2] == "screen" {
        // Read-only diagnostic: talk to whatever daemon currently holds the socket,
        // regardless of version. Do NOT call ensure_local_server_ready_for_cli — its
        // "is current" version gate would reject an older running daemon and try to
        // spawn a competing one (which fails while the socket is held), so a screen
        // dump must connect directly like `server status` / `server snapshot` do.
        let endpoint = cli_server_endpoint(store.home_dir());
        let retained = args.iter().any(|arg| arg == "--retained");
        let raw = args.iter().any(|arg| arg == "--raw");
        let (
            text,
            running,
            runtime_output_seen,
            post_resize_output_seen,
            last_resize_seq,
            _runtime_spawn_id,
        ) = if retained {
            terminal_retained_snapshot(&endpoint, &args[3])?
        } else {
            terminal_snapshot(&endpoint, &args[3])?
        };
        if raw {
            print!("{text}");
        } else {
            let nonblank = text.lines().filter(|line| !line.trim().is_empty()).count();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session_path": args[3],
                    "source": if retained { "retained_snapshot" } else { "live_screen" },
                    "running": running,
                    "runtime_output_seen": runtime_output_seen,
                    "post_resize_output_seen": post_resize_output_seen,
                    "last_resize_seq": last_resize_seq,
                    "line_count": text.lines().count(),
                    "nonblank_line_count": nonblank,
                    "char_count": text.chars().count(),
                    "text": text,
                }))?
            );
        }
        return Ok(());
    }
    // Resize a session's PTY (SIGWINCH). Forces a full-screen TUI repaint —
    // the safe recovery for a blank/garbled remote viewport where the daemon
    // holds the content but xterm.js seeded from a stale/empty snapshot and the
    // idle program won't re-emit on its own. Read-only-ish control op; skips the
    // is-current version gate so it works against an older running daemon.
    if args.len() >= 4 && args[0] == "server" && args[1] == "terminal" && args[2] == "resize" {
        // Address the daemon that OWNS this runtime key, not the one this
        // binary's version would spawn. This is the entry point the local
        // daemon's `forward_remote_pty_resize` reaches over ssh, and on a host
        // running version-coexisting daemons the owner is routinely an OLDER
        // daemon than the deployed binary. Resolving by version is why SIGWINCH
        // silently stopped reaching remote CC agents on `dev` while the same
        // tick resized `oc` fine — see `owning_daemon_endpoint_for_runtime_key`.
        let endpoint = control_endpoint_for_runtime_key(store.home_dir(), &args[3]);
        let cols = cli_flag_value(&args, "--cols")
            .and_then(|v| v.parse::<u16>().ok())
            .context("missing/invalid --cols for server terminal resize")?;
        let rows = cli_flag_value(&args, "--rows")
            .and_then(|v| v.parse::<u16>().ok())
            .context("missing/invalid --rows for server terminal resize")?;
        terminal_resize(&endpoint, &args[3], cols, rows)?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "resized": true,
                "session_path": args[3],
                "cols": cols,
                "rows": rows,
                "owner_endpoint": format!("{endpoint:?}"),
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
    if args.len() >= 3
        && args[0] == "server"
        && matches!(args[1].as_str(), "sessions" | "session-copy")
    {
        return run_sessions_regenerate_copy_cli(&store, &args);
    }
    if try_run_remote_server_command(&args)? {
        return Ok(());
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "tail" {
        return run_trace_tail(parse_trace_limit(&args, 200)?);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "follow" {
        let lines = parse_trace_limit(&args, 200)?;
        let poll_ms = parse_trace_poll_ms(&args, 500)?;
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
        // ONE owner for `--limit` across every `trace` verb. This one already
        // worked; sharing the parser is what stops the four drifting apart again.
        let limit = parse_trace_limit(&args, 200)?;
        return run_trace_transitions(session_filter.as_deref(), last_ms, limit);
    }
    if args.len() >= 3 && args[0] == "server" && args[1] == "trace" && args[2] == "bundle" {
        let lines = parse_trace_limit(&args, 200)?;
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
        if compositor || parse_screenshot_post_process(&args).is_some() {
            let post = parse_screenshot_post_process(&args).unwrap_or(ScreenshotPostProcess {
                region: None,
                crop: None,
                scale: 1.0,
                grid: None,
            });
            return run_screenshot_capture_with_post_process(
                &target,
                output_path,
                timeout_ms,
                post,
                compositor,
            );
        }
        return run_screenshot_capture(&target, output_path, timeout_ms);
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
        // ONE OWNER for the whole `server app` surface — see
        // `yggterm_server::app_control_cli`. The 1,308-line `match` that stood
        // here was a second copy of the headless binary's, and the pair had
        // drifted by six verbs. Do not inline a verb here.
        struct GuiHost;
        impl yggterm_server::app_control_cli::AppControlHost for GuiHost {
            fn binary_name(&self) -> &'static str {
                "yggterm"
            }
            // The one genuine fork: this binary IS the app, so it spawns the
            // window itself instead of asking a companion.
            fn launch_app(
                &self,
                args: &[String],
                home_dir: &std::path::Path,
                timeout_ms: u64,
            ) -> anyhow::Result<()> {
                let log_path = args.windows(2).find_map(|window| {
                    if window[0] == "--log" {
                        Some(window[1].as_str())
                    } else {
                        None
                    }
                });
                launch_app_background(
                    home_dir,
                    timeout_ms,
                    args.iter().any(|arg| arg == "--wait-visible"),
                    args.iter().any(|arg| arg == "--wait-settled"),
                    args.iter().any(|arg| arg == "--allow-multi-window"),
                    args.iter().any(|arg| arg == "--skip-active-exec-handoff"),
                    log_path,
                )
            }
        }
        return yggterm_server::app_control_cli::run_app_control_cli(
            &args,
            store.home_dir(),
            &GuiHost,
        );
    }
    if args.as_slice() == ["server", "shutdown"] {
        let endpoint = cli_server_endpoint(store.home_dir());
        if let Some(message) = shutdown(&endpoint)? {
            println!("{message}");
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
    // ⛔ SEPARATE FROM `--version` ON PURPOSE — see `build_identity`. The version
    // is a rendezvous key two clusters can spend on the same day; the commit is
    // the identity, and it is what tells a live probe whether the binary in
    // front of it carries the fix being probed for.
    if matches!(
        args.first().map(String::as_str),
        Some("--build-commit" | "build-commit")
    ) {
        println!("{}", build_identity::build_commit());
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
    let terminal_appearance = terminal_identity_appearance_for_settings(&settings).to_string();
    yggterm_server::sync_terminal_identity_appearance(&terminal_appearance);
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
    // Connect to the right daemon even when this GUI is NEWER than the running
    // daemon: the daemon only aliases sockets for versions <= its own, so a newer
    // GUI's own-version socket is absent. Falling back to the reachable older
    // daemon here keeps reopened terminal hosts from stranding on the stale client
    // snapshot (finding-gui-only-deploy-version-socket-mismatch).
    let resolved_daemon = resolve_client_daemon_endpoint(store.home_dir());
    if let Some((client_version, daemon_version)) = resolved_daemon.version_mismatch.as_ref() {
        append_trace_event(
            &startup_home,
            "gui",
            "startup",
            "daemon_version_mismatch",
            serde_json::json!({
                "client_version": client_version,
                "daemon_version": daemon_version,
                "connected_endpoint": format!("{:?}", resolved_daemon.endpoint),
                "detail": "GUI is newer than the running daemon; connected to the \
                           older daemon so sessions are not stranded. Deploy the \
                           matching daemon version to clear this.",
            }),
        );
    }
    let endpoint = resolved_daemon.endpoint;
    // Slice 4.3 fail-closed gate (eng-review D7, acceptance gate 15): a Shadow
    // view client refuses to attach to a daemon that does not advertise role
    // enforcement — such a daemon ignores the role and would treat this
    // read-only client as fully Active, exactly during the mixed-version window
    // when the guard matters most. No-op for the user's (Active) GUI.
    //
    // Deliberately placed BEFORE `warm_daemon_start`, so a refused shadow never
    // spawns or touches a daemon at all.
    yggterm_server::verify_shadow_client_can_attach(&endpoint)?;
    install_signal_shutdown(store.home_dir().to_path_buf(), endpoint.clone());
    warm_daemon_start(
        endpoint.clone(),
        Some(startup_home.clone()),
        Some(terminal_appearance.clone()),
    );
    start_daemon_watchdog(
        endpoint.clone(),
        Some(startup_home.clone()),
        Some(terminal_appearance.clone()),
    );
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
            "terminal_appearance": terminal_appearance,
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
    force_x11_backend: bool,
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
    backend: Option<&'static str>,
    reason: &'static str,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxTerminalRendererPolicyInput {
    explicit_canvas_env: bool,
    gdk_backend_x11: bool,
    wayland_display_present: bool,
    display_present: bool,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxTerminalRendererPolicy {
    set_canvas_env: Option<&'static str>,
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
            backend: None,
            reason: "wayland_backend_explicitly_allowed",
        };
    }
    if input.force_x11_backend {
        return LinuxDesktopBackendPolicy {
            force_x11_backend: input.display_present,
            set_gdk_backend: input.display_present && !input.gdk_backend_set,
            set_winit_backend: input.display_present && !input.winit_backend_set,
            backend: input.display_present.then_some("x11"),
            reason: if input.display_present {
                "x11_backend_explicitly_forced"
            } else {
                "x11_backend_force_without_display"
            },
        };
    }
    if input.gdk_backend_set {
        return LinuxDesktopBackendPolicy {
            force_x11_backend: false,
            set_gdk_backend: false,
            set_winit_backend: false,
            backend: None,
            reason: "gdk_backend_explicit",
        };
    }
    if !(input.kde_session && input.wayland_display_present && input.display_present) {
        return LinuxDesktopBackendPolicy {
            force_x11_backend: false,
            set_gdk_backend: false,
            set_winit_backend: false,
            backend: None,
            reason: "no_kde_wayland_x11_pair",
        };
    }
    LinuxDesktopBackendPolicy {
        force_x11_backend: false,
        set_gdk_backend: !input.gdk_backend_set,
        set_winit_backend: !input.winit_backend_set,
        backend: Some("wayland"),
        reason: "kde_wayland_native_default",
    }
}

#[cfg(target_os = "linux")]
fn linux_terminal_renderer_policy_from_input(
    input: LinuxTerminalRendererPolicyInput,
) -> LinuxTerminalRendererPolicy {
    if input.explicit_canvas_env {
        return LinuxTerminalRendererPolicy {
            set_canvas_env: None,
            reason: "xterm_canvas_explicit",
        };
    }
    if input.gdk_backend_x11 || (!input.wayland_display_present && input.display_present) {
        return LinuxTerminalRendererPolicy {
            set_canvas_env: Some("0"),
            reason: "xterm_canvas_disabled_for_x11",
        };
    }
    if input.wayland_display_present {
        // Wayland: WebGL (xterm.js 6's GPU renderer — the highest-performance tier).
        // WebGL only PRESENTS with WebKitGTK accelerated compositing enabled; the
        // earlier "WebGL is black" was compositing being DISABLED, not WebGL itself.
        // configure_linux_webkit_compositing now keeps compositing ON with a
        // software-GL safety net (verified on guihost: WebGL composites, no crash). xterm6
        // removed the 2D canvas renderer, so WebGL is the GPU tier here; DOM remains the
        // automatic fallback if the WebGL context is lost.
        return LinuxTerminalRendererPolicy {
            set_canvas_env: Some("1"),
            reason: "xterm_webgl_enabled_for_wayland",
        };
    }
    LinuxTerminalRendererPolicy {
        set_canvas_env: Some("0"),
        reason: "xterm_canvas_disabled_by_default",
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
const LINUX_GUI_ENTRY_ENV_KEYS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
    "KDE_FULL_SESSION",
    "DBUS_SESSION_BUS_ADDRESS",
];

#[cfg(target_os = "linux")]
const LINUX_GUI_ENTRY_ENV_SOURCE_KEY: &str = "YGGTERM_DESKTOP_ENV_HYDRATED_FROM";

#[cfg(target_os = "linux")]
fn linux_current_environment_map() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(key, value)| !key.trim().is_empty() && !value.trim().is_empty())
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_environ_bytes_to_map(environ: &[u8]) -> BTreeMap<String, String> {
    environ
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .filter_map(|entry| entry.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_desktop_env_score(command_name: &str, env: &BTreeMap<String, String>) -> Option<i32> {
    let has_display = env.contains_key("DISPLAY") || env.contains_key("WAYLAND_DISPLAY");
    let has_runtime = env.contains_key("XDG_RUNTIME_DIR");
    if !has_display || !has_runtime {
        return None;
    }
    let desktop_text = [
        env.get("XDG_CURRENT_DESKTOP"),
        env.get("XDG_SESSION_DESKTOP"),
        env.get("DESKTOP_SESSION"),
        env.get("KDE_FULL_SESSION"),
    ]
    .into_iter()
    .flatten()
    .map(|value| value.to_ascii_lowercase())
    .collect::<Vec<_>>()
    .join(" ");
    let kde = desktop_text.contains("kde")
        || desktop_text.contains("plasma")
        || matches!(
            env.get("KDE_FULL_SESSION").map(String::as_str),
            Some("true" | "1")
        );
    let command_score = match command_name {
        "plasmashell" => 100,
        "kwin_wayland" | "kwin_x11" => 95,
        "startplasma-wayland" | "startplasma-x11" => 90,
        "gnome-shell" => 70,
        "cinnamon" | "mate-session" | "xfce4-session" => 60,
        _ => 20,
    };
    let display_score = match (
        env.contains_key("WAYLAND_DISPLAY"),
        env.contains_key("DISPLAY"),
    ) {
        (true, true) => 25,
        (true, false) => 15,
        (false, true) => 10,
        (false, false) => 0,
    };
    let desktop_score = if kde { 40 } else { 0 };
    Some(command_score + display_score + desktop_score)
}

#[cfg(target_os = "linux")]
fn linux_choose_desktop_environment<I>(candidates: I) -> Option<(String, BTreeMap<String, String>)>
where
    I: IntoIterator<Item = (String, BTreeMap<String, String>)>,
{
    candidates
        .into_iter()
        .filter_map(|(command_name, env)| {
            linux_desktop_env_score(&command_name, &env).map(|score| (score, command_name, env))
        })
        .max_by_key(|(score, _, _)| *score)
        .map(|(_, command_name, env)| (command_name, env))
}

#[cfg(target_os = "linux")]
fn discover_linux_desktop_environment() -> Option<(String, BTreeMap<String, String>)> {
    let entries = fs::read_dir("/proc").ok()?;
    let candidates = entries.filter_map(|entry| {
        let entry = entry.ok()?;
        let file_name = entry.file_name();
        let pid = file_name.to_string_lossy();
        if !pid.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let proc_dir = entry.path();
        let command_name = fs::read_to_string(proc_dir.join("comm"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())?;
        let environ = fs::read(proc_dir.join("environ")).ok()?;
        let env = linux_environ_bytes_to_map(&environ);
        Some((command_name, env))
    });
    linux_choose_desktop_environment(candidates)
}

#[cfg(target_os = "linux")]
fn linux_gui_entry_environment_overrides_from_desktop(
    current_env: &BTreeMap<String, String>,
    desktop_env: Option<(String, BTreeMap<String, String>)>,
) -> BTreeMap<String, String> {
    let display_present =
        current_env.contains_key("DISPLAY") || current_env.contains_key("WAYLAND_DISPLAY");
    let desktop_identity_present = [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
        "KDE_FULL_SESSION",
    ]
    .iter()
    .any(|key| current_env.contains_key(*key));
    let runtime_present =
        current_env.contains_key("XDG_RUNTIME_DIR") && current_env.contains_key("XAUTHORITY");
    if display_present && desktop_identity_present && runtime_present {
        return BTreeMap::new();
    }
    let Some((source, env)) = desktop_env else {
        return BTreeMap::new();
    };
    let mut overrides = BTreeMap::new();
    for key in LINUX_GUI_ENTRY_ENV_KEYS {
        if display_present && matches!(*key, "DISPLAY" | "WAYLAND_DISPLAY") {
            continue;
        }
        if !current_env.contains_key(*key)
            && let Some(value) = env.get(*key)
            && !value.trim().is_empty()
        {
            overrides.insert((*key).to_string(), value.to_string());
        }
    }
    overrides.insert(LINUX_GUI_ENTRY_ENV_SOURCE_KEY.to_string(), source);
    overrides
}

#[cfg(target_os = "linux")]
fn hydrate_linux_gui_entry_environment_from_desktop() {
    let current_env = linux_current_environment_map();
    let overrides = linux_gui_entry_environment_overrides_from_desktop(
        &current_env,
        discover_linux_desktop_environment(),
    );
    for (key, value) in overrides {
        unsafe { std::env::set_var(key, value) };
    }
}

#[cfg(target_os = "linux")]
fn configure_linux_desktop_backend() {
    let policy = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
        allow_wayland_backend: linux_env_flag_truthy(ENV_YGGTERM_ALLOW_WAYLAND_BACKEND),
        force_x11_backend: linux_env_flag_truthy(ENV_YGGTERM_FORCE_X11_BACKEND),
        gdk_backend_set: std::env::var_os("GDK_BACKEND").is_some(),
        winit_backend_set: std::env::var_os("WINIT_UNIX_BACKEND").is_some(),
        kde_session: linux_session_env_looks_like_kde_plasma(),
        wayland_display_present: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        display_present: std::env::var_os("DISPLAY").is_some(),
    });
    let Some(backend) = policy.backend else {
        return;
    };
    if policy.set_gdk_backend {
        unsafe { std::env::set_var("GDK_BACKEND", backend) };
    }
    if policy.set_winit_backend {
        unsafe { std::env::set_var("WINIT_UNIX_BACKEND", backend) };
    }
    unsafe { std::env::set_var("YGGTERM_LINUX_BACKEND_POLICY", policy.reason) };
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_desktop_backend() {}

#[cfg(target_os = "linux")]
fn configure_linux_terminal_renderer_policy() {
    let policy = linux_terminal_renderer_policy_from_input(LinuxTerminalRendererPolicyInput {
        explicit_canvas_env: linux_canvas_env_is_user_explicit(
            std::env::var_os(ENV_YGGTERM_ENABLE_XTERM_CANVAS).is_some(),
            std::env::var("YGGTERM_XTERM_CANVAS_POLICY").ok().as_deref(),
        ),
        gdk_backend_x11: std::env::var("GDK_BACKEND")
            .ok()
            .is_some_and(|value| value.split(',').any(|part| part.trim() == "x11")),
        // Detect Wayland by the compositor SOCKET, not just $WAYLAND_DISPLAY: the
        // daemon-spawned GUI (`server app launch`) starts without display env vars (they
        // are recovered later), so an env-only check ran before WAYLAND_DISPLAY existed
        // and mis-picked DOM. The socket under XDG_RUNTIME_DIR is present regardless of
        // env timing — the reliable "a Wayland session is available" signal.
        wayland_display_present: linux_wayland_session_available(),
        display_present: std::env::var_os("DISPLAY").is_some()
            || std::path::Path::new("/tmp/.X11-unix/X0").exists(),
    });
    if let Some(value) = policy.set_canvas_env {
        unsafe { std::env::set_var(ENV_YGGTERM_ENABLE_XTERM_CANVAS, value) };
    }
    unsafe { std::env::set_var("YGGTERM_XTERM_CANVAS_POLICY", policy.reason) };
}

/// Whether YGGTERM_ENABLE_XTERM_CANVAS in this process's env is a USER
/// override (honor it) or an INHERITED launcher decision (recompute it).
/// The companion launch lane (`yggterm-headless server app launch` over ssh)
/// computes the policy in an environment with no display vars and exports
/// BOTH the canvas flag and YGGTERM_XTERM_CANVAS_POLICY; the windowed GUI it
/// spawns inherits the pair and — before this guard — mistook the flag for an
/// explicit override, locking every agent-launched GUI to the DOM renderer
/// (broken box-drawing / missing highlights) while desktop launches got
/// canvas. A canvas flag accompanied by a non-explicit policy marker is an
/// inherited decision; only a bare flag (user export) or one whose marker
/// says "xterm_canvas_explicit" (re-exec of an honored override) is explicit.
#[cfg(target_os = "linux")]
fn linux_canvas_env_is_user_explicit(canvas_env_present: bool, policy_env: Option<&str>) -> bool {
    canvas_env_present
        && policy_env
            .map(str::trim)
            .filter(|policy| !policy.is_empty())
            .is_none_or(|policy| policy == "xterm_canvas_explicit")
}

/// True when a Wayland session is available, detected by the compositor SOCKET under
/// $XDG_RUNTIME_DIR rather than $WAYLAND_DISPLAY alone. The daemon-spawned GUI
/// (`server app launch`) starts before its display env is recovered, so an env-only
/// check ran too early and mis-picked the DOM renderer; the socket is present
/// regardless of env timing. Honors an explicitly-named WAYLAND_DISPLAY socket and
/// falls back to the default `wayland-0` / any `wayland-*` socket.
#[cfg(target_os = "linux")]
fn linux_wayland_session_available() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return true;
    }
    let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return false;
    };
    let runtime = std::path::Path::new(&runtime_dir);
    if runtime.join("wayland-0").exists() {
        return true;
    }
    std::fs::read_dir(runtime)
        .map(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("wayland-") && !name.ends_with(".lock")
            })
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_terminal_renderer_policy() {}

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
    // ⛔ A WAYLAND CORNER IS A CAPABILITY, NOT A DESKTOP'S FEATURE.
    // Wayland has no Shape extension, so `shape_combine_region` — the whole X11
    // rounding path — is a no-op here. The ONLY way a corner rounds on Wayland
    // is an alpha surface the compositor composites, and EVERY Wayland
    // compositor composites alpha: it is wl_surface core, not an extension a
    // desktop may decline. So there is nothing to detect and no allow-list to
    // maintain.
    //
    // This branch used to read `kde_session && wayland_display_present`, which
    // made the corner a property of WHO the desktop claimed to be. That is why
    // the rounding kept cycling fixed→broken for the product's life:
    //   · every desktop not named here (GNOME, sway, Hyprland, wlroots, and
    //     yggterm's OWN shadow clients and sandboxes) shipped square corners —
    //     not because it could not round, but because it was not on the list;
    //   · KDE itself is recognised only by scraped environment
    //     (XDG_CURRENT_DESKTOP / KDE_FULL_SESSION), which a GUI launched before
    //     its desktop env is hydrated does not have — so the SAME machine
    //     rendered rounded or square depending on how that launch went, which
    //     is exactly the non-determinism the engineering contract forbids.
    // Keying on the capability removes both failures at once, and there is no
    // third desktop to add later.
    if input.wayland_display_present {
        return LinuxWindowProfile {
            transparent: true,
            xrpd_session: false,
            // KDE keeps its own reason string so existing traces stay
            // comparable across the change; the behaviour is now identical.
            reason: if input.kde_session {
                "kde_wayland_transparent_profile"
            } else {
                "wayland_transparent_profile"
            },
        };
    }
    LinuxWindowProfile {
        transparent: false,
        xrpd_session: false,
        reason: "linux_opaque_default",
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
        let display_present = std::env::var_os("DISPLAY").is_some();
        // ⛔ ENV ALONE ANSWERS THIS TOO EARLY. A GUI started by the daemon
        // (`server app launch`) runs before its display env is hydrated from the
        // desktop scope, so an env-only read saw NEITHER display and fell to the
        // opaque default — squaring the corners of a window whose compositor was
        // right there. The renderer policy already learned this and consults the
        // compositor SOCKET (`linux_wayland_session_available`); the window
        // profile never did, so the two disagreed about the same session.
        // The socket only RESCUES the no-display-env case: a genuine X11 session
        // has DISPLAY set, so a nested compositor's stray socket cannot capture
        // it away from the X11 arm above.
        let wayland_display_present = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || (!display_present && linux_wayland_session_available());
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

/// Whether under-glass web-surface stacking should be armed, from the two
/// environment knobs. **DEFAULT ON (user directive 2026-07-31): unset ⇒ armed
/// on a hardware-GL host.** Explicit opt-outs: `YGGTERM_WEB_SURFACE_UNDER_GLASS=0`
/// or the legacy force `YGGTERM_WEB_SURFACE_LEGACY_STACK=1`. Structural safety
/// stays runtime-side — the vendored host's self-probe demotes to legacy
/// stacking on engines/paths that cannot composite (SHM, webkit < 2.40),
/// so the default costs nothing where under-glass is impossible.
///
/// The user settled this after quitting the GUI and finding that a web surface
/// launched WITHOUT the flag does not sit flush in the viewport: *"I could not
/// understand why our software needs an extra flag to be correct."* That is the
/// right reading. Under-glass is not an experiment any more — it IS the
/// correct presentation path, and a product whose correct path is opt-in is a
/// product that is wrong by default.
fn under_glass_default_armed(
    under_glass_var: Option<&str>,
    legacy_stack_var: Option<&str>,
    software_gl: bool,
) -> bool {
    if legacy_stack_var == Some("1") {
        return false;
    }
    // SOFTWARE-GL HOSTS DEMOTE (2026-07-20, live-caught on the KDE host). Under
    // glass REQUIRES the DMABuf renderer (SHM cannot composite a transparent
    // webview), and the DMABuf path SIGSEGVs on a host with no working hardware
    // GL — the same Mesa/EGL crash the LIBGL_ALWAYS_SOFTWARE safety net below
    // exists for. Arming re-enabled exactly that path: the GUI crash-looped
    // (6 coredumps in a day, systemd relaunching each time = a blank viewport
    // and a dropped session every few minutes). A one-off sandbox verification
    // said "DMABuf composites fine over llvmpipe"; sustained production use
    // falsified it. So the arming decision — the ONE owner of the presentation
    // path — refuses to arm where DMABuf is unsafe, instead of forcing DMABuf on
    // and leaving the user to discover the crash. An explicit
    // YGGTERM_WEB_SURFACE_UNDER_GLASS=1 still wins (opt-in override for a host
    // whose software GL is known good); the default just stops being a trap.
    if software_gl && under_glass_var != Some("1") {
        return false;
    }
    // ⭐ UNDER-GLASS IS THE STANDARD PRESENTATION PATH (user directive
    // 2026-07-31). It is what makes a web surface sit FLUSH in the viewport:
    // the page composites at the back of the z-order with the chrome floating
    // above it, instead of a native child painting over the top of everything.
    // Without it the user sees the surface not fitting its frame — which is
    // exactly the report that settled this.
    //
    // ⚠ THE RISK THIS ACCEPTS, STATED HONESTLY. Under glass the shell webview
    // is TRANSPARENT by construction and the surfaces composite behind it, so
    // anything that stops the shell painting stops being "a blank window" and
    // starts being "whatever page happens to be back there, full screen". That
    // fired once in production (2026-07-26) while memory was exhausted — swap
    // 100% full, the GUI too starved to answer a state probe for 25 s, exactly
    // when the shell is least likely to paint. Under the legacy stack the same
    // starvation is invisible, because the shell is opaque.
    //
    // What changed since, and why the default flips anyway: the incident guard
    // is now pixel-proven (a second, never-revealed surface paints ZERO pixels;
    // visibility-truth keeps unrevealed pages unmapped), the arrangement has
    // been the user's daily driver on the live host since 2026-07-30 and they
    // have asked for it as the standard, and the swap exhaustion that supplied
    // the starvation has eased. The failure mode is a DEGRADED-PAINT problem,
    // not a stacking problem — the honest fix is to keep the shell painting,
    // not to keep the correct presentation path switched off.
    //
    // Both escape hatches survive and are the supported answer if it regresses:
    // `YGGTERM_WEB_SURFACE_UNDER_GLASS=0`, or `YGGTERM_WEB_SURFACE_LEGACY_STACK=1`
    // which beats everything. The software-GL demotion above still refuses to
    // arm where DMABuf would SIGSEGV, so this default cannot resurrect the
    // crash-loop.
    under_glass_var != Some("0")
}

/// What the arming decision implies for the WebKit presentation path. The
/// arming decision is the ONE owner of this choice, so an inherited SHM force
/// from an earlier unarmed run is cleared rather than left to fight it.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShmForce {
    /// Under-glass armed: the DMABuf renderer is required, drop any SHM force.
    Clear,
    /// Unarmed and nothing set it yet: apply the historical SHM workaround.
    Apply,
    /// Unarmed and already forced: leave the caller's value alone.
    Keep,
}

/// SHM presentation exists for ONE reason: it is the workaround for hosts whose
/// hardware EGL/DMABuf path crashes. So on a host we have established HAS working
/// hardware GL it is not a candidate at all, whatever arming decides — and that is
/// why `hardware_gl` sits alongside `armed` here rather than only feeding it.
///
/// Measured on the live host, and the reason this is one decision and not three:
/// hardware GL + SHM cost 15.82 s where software GL cost 15.33 s (i.e. hardware GL
/// with SHM buys nothing), and software GL + DMABuf cost 34.14 s — the WORST of the
/// four, llvmpipe emulating the compositor. Only hardware GL + DMABuf (6.85 s) wins.
/// Without this argument an explicit `YGGTERM_WEB_SURFACE_UNDER_GLASS=0` on a
/// probed-hardware host would land in exactly the no-win cell.
#[cfg(target_os = "linux")]
fn shm_force_for_arming(armed: bool, hardware_gl: bool, already_forced: bool) -> ShmForce {
    match (armed || hardware_gl, already_forced) {
        (true, _) => ShmForce::Clear,
        (false, false) => ShmForce::Apply,
        (false, true) => ShmForce::Keep,
    }
}

/// What the GL policy implies for the software-rasterizer variables — the twin of
/// [`ShmForce`], and for exactly the same reason.
///
/// ⚠ Caught by running the real binary 2026-07-25, not by reading it: this agent's own
/// shell inherits `LIBGL_ALWAYS_SOFTWARE=1` + `GALLIUM_DRIVER=llvmpipe` from the GUI
/// that spawned its terminal, and a hot-restarted GUI inherits the same pair from its
/// predecessor. `if !hardware_gl { set }` leaves those inherited values in place on a
/// host the probe just declared HARDWARE — so the startup trace read
/// `webkit_gl_policy: hardware_gl_probed` next to `libgl_always_software: "1"`, WebKit
/// stayed on llvmpipe, and the decision and the state disagreed silently. Declining to
/// set a variable is not the same as owning it.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoftwareGlForce {
    /// Software path: set the pair where nothing has set it.
    Apply,
    /// Hardware path: REMOVE the pair, including a value we inherited.
    Clear,
}

#[cfg(target_os = "linux")]
fn software_gl_force_for_policy(hardware_gl: bool) -> SoftwareGlForce {
    if hardware_gl {
        SoftwareGlForce::Clear
    } else {
        SoftwareGlForce::Apply
    }
}

/// What the GL decision does to ONE environment variable.
///
/// ⚠ This enum exists because the previous shape was untestable and the one bug found
/// by RUNNING the binary slipped through a green suite because of it: the decision
/// lived inside `configure_linux_webkit_compositing`, tangled with the `set_var` calls
/// that applied it, so no test could reach it. A reviewer restored the pre-fix shape
/// (`if !hardware_gl { set if unset }`, no `remove_var`) and 44/44 stayed green with
/// the bug fully back. The decision is DATA now, and the applier below has no
/// conditionals left to hide one in.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlEnvAction {
    /// Own it: write this value over whatever was inherited.
    Set(&'static str),
    /// Own it: REMOVE it, including a value we inherited. Declining to SET a variable
    /// is not the same as owning it — that difference IS the live-caught bug.
    Remove,
    /// Leave what is there: the inherited value already says what we want, or this
    /// variable is not ours to touch on this path.
    Keep,
}

/// The environment `configure_linux_webkit_compositing` INHERITED, as an input.
///
/// Presence-only for the three force variables, because that is exactly what the
/// applier used to test (`var_os(..).is_none()`); a non-UTF-8 value still counts as
/// set. The two under-glass knobs carry their text, because
/// [`under_glass_default_armed`] reads their values.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct LinuxWebkitGlEnvInherited<'a> {
    libgl_always_software_present: bool,
    gallium_driver_present: bool,
    webkit_disable_dmabuf_renderer_present: bool,
    web_surface_under_glass: Option<&'a str>,
    web_surface_legacy_stack: Option<&'a str>,
    /// `__EGL_VENDOR_LIBRARY_FILENAMES` already set: someone (the user, a
    /// wrapper) has an opinion about GLVND vendor selection and the guard
    /// defers to it.
    egl_vendor_library_filenames_present: bool,
}

/// Everything the GL decision does to the process environment, as a value.
///
/// One field per key in [`yggterm_core::gl_probe::WEBKIT_GL_ENVIRONMENT_KEYS`], so a
/// key cannot silently drop out of the decision — the applier iterates that list and a
/// test asserts the two agree.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxWebkitGlEnvPlan {
    webkit_gl_policy: GlEnvAction,
    web_surface_under_glass: GlEnvAction,
    libgl_always_software: GlEnvAction,
    gallium_driver: GlEnvAction,
    webkit_disable_dmabuf_renderer: GlEnvAction,
    egl_vendor_library_filenames: GlEnvAction,
}

#[cfg(target_os = "linux")]
fn gl_env_set_if_unset(present: bool, value: &'static str) -> GlEnvAction {
    if present {
        GlEnvAction::Keep
    } else {
        GlEnvAction::Set(value)
    }
}

/// Turn the GL policy plus the inherited environment into the exact set of
/// environment mutations to perform. THE decision; `configure_linux_webkit_compositing`
/// only applies it.
#[cfg(target_os = "linux")]
fn linux_webkit_gl_env_plan(
    policy: LinuxWebkitGlPolicy,
    compositing_disabled_env: bool,
    stray_nvidia_egl_vendor: bool,
    inherited: LinuxWebkitGlEnvInherited<'_>,
) -> LinuxWebkitGlEnvPlan {
    // Under-glass by DEFAULT: resolve the two env knobs into the ONE arming variable
    // every downstream reader keys on (the DMABuf gate below, the vendored
    // disable_dma_buf workaround, the vendored host's opt_in). Resolved AFTER the GL
    // policy: under glass needs DMABuf, and DMABuf is unsafe on a software-GL host, so
    // the GL decision is an INPUT to arming.
    let armed = under_glass_default_armed(
        inherited.web_surface_under_glass,
        inherited.web_surface_legacy_stack,
        !policy.hardware_gl,
    );
    let mut plan = LinuxWebkitGlEnvPlan {
        // This runs before tracing is initialized and before the store exists, so the
        // exported reason is the only way the decision is observable at all.
        webkit_gl_policy: GlEnvAction::Set(policy.reason),
        web_surface_under_glass: GlEnvAction::Set(if armed { "1" } else { "0" }),
        libgl_always_software: GlEnvAction::Keep,
        gallium_driver: GlEnvAction::Keep,
        webkit_disable_dmabuf_renderer: GlEnvAction::Keep,
        // A stray NVIDIA GLVND ICD on a device-less host gets libEGL_nvidia mapped
        // into this process and every web process, and it showed up in live WebKit
        // crash stacks right behind `eglMakeCurrent failed` (guihost, 2026-07-26). Pin
        // GLVND to the Mesa ICD — but only on the hardware path (the crash class
        // lives there, and `YGGTERM_FORCE_SOFTWARE_GL=1` must keep restoring the
        // old behaviour whole), and never over an explicit user filter.
        egl_vendor_library_filenames: if policy.hardware_gl
            && stray_nvidia_egl_vendor
            && !inherited.egl_vendor_library_filenames_present
        {
            GlEnvAction::Set(yggterm_core::gl_probe::MESA_EGL_VENDOR_JSON)
        } else {
            GlEnvAction::Keep
        },
    };
    // Escape hatch: if the user force-disabled compositing, respect it — WebGL becomes
    // unavailable and the renderer policy falls back to DOM. Deliberately AFTER the
    // two settings above: a short-circuited run must still be able to say what it
    // decided and why, or the one observable is missing exactly when someone is asking
    // why the GPU is off.
    if compositing_disabled_env {
        return plan;
    }
    match software_gl_force_for_policy(policy.hardware_gl) {
        SoftwareGlForce::Apply => {
            plan.libgl_always_software =
                gl_env_set_if_unset(inherited.libgl_always_software_present, "1");
            plan.gallium_driver = gl_env_set_if_unset(inherited.gallium_driver_present, "llvmpipe");
        }
        // ⚠ Live-caught 2026-07-25: this arm used to be "do nothing", and a GUI
        // relaunched by a running GUI inherits LIBGL_ALWAYS_SOFTWARE=1 from its
        // predecessor — so on a probed-HARDWARE host the trace read
        // `webkit_gl_policy: hardware_gl_probed` next to `libgl_always_software: "1"`
        // and WebKit stayed on llvmpipe.
        SoftwareGlForce::Clear => {
            plan.libgl_always_software = GlEnvAction::Remove;
            plan.gallium_driver = GlEnvAction::Remove;
        }
    }
    // Phase F under-glass REQUIRES the DMABUF renderer (F.0.1 root cause,
    // sandbox-proven): the SHM presentation path clears a transparent webview's
    // regions straight through every sibling widget beneath — the glass hole punches
    // through page webviews and backdrop to the window background, so the page can
    // never show. The DMABUF path composites in-widget with alpha and works,
    // INCLUDING over software GL (llvmpipe, the safety net above). So: armed ⇒ the
    // renderer MUST be at WebKit's default (DMABUF); unarmed ⇒ keep the historical SHM
    // workaround for the hosts whose hardware EGL/DMABUF path crashed.
    //
    // The arming decision is the SINGLE source of truth for the presentation path, so
    // armed CLEARS an inherited SHM force instead of leaving two answers to diverge.
    // Why this is not theoretical: an UNARMED run sets this var (here, and vendored
    // app.rs on Wayland+/dev/dri), and a GUI relaunched by a running GUI inherits that
    // process env — so the var outlived the run that wanted it and rode into an ARMED
    // launch, where the vendored host silently demoted under-glass to legacy stacking.
    // Live-caught on the KDE host 2026-07-20.
    plan.webkit_disable_dmabuf_renderer = match shm_force_for_arming(
        armed,
        policy.hardware_gl,
        inherited.webkit_disable_dmabuf_renderer_present,
    ) {
        ShmForce::Clear => GlEnvAction::Remove,
        ShmForce::Apply => GlEnvAction::Set("1"),
        ShmForce::Keep => GlEnvAction::Keep,
    };
    plan
}

/// The keys of a [`LinuxWebkitGlEnvPlan`], paired with the action decided for each.
/// The applier and the "which keys are the GL path" list meet here and nowhere else.
#[cfg(target_os = "linux")]
fn linux_webkit_gl_env_plan_entries(
    plan: &LinuxWebkitGlEnvPlan,
) -> [(&'static str, GlEnvAction); 6] {
    use yggterm_core::gl_probe as probe;
    [
        (probe::ENV_YGGTERM_WEBKIT_GL_POLICY, plan.webkit_gl_policy),
        (
            probe::ENV_YGGTERM_WEB_SURFACE_UNDER_GLASS,
            plan.web_surface_under_glass,
        ),
        (probe::ENV_LIBGL_ALWAYS_SOFTWARE, plan.libgl_always_software),
        (probe::ENV_GALLIUM_DRIVER, plan.gallium_driver),
        (
            probe::ENV_WEBKIT_DISABLE_DMABUF_RENDERER,
            plan.webkit_disable_dmabuf_renderer,
        ),
        (
            probe::ENV_EGL_VENDOR_LIBRARY_FILENAMES,
            plan.egl_vendor_library_filenames,
        ),
    ]
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxWebkitGlPolicyInput {
    /// `WEBKIT_DISABLE_COMPOSITING_MODE` is present: the user force-disabled
    /// compositing entirely, so there is no GPU path left to choose.
    compositing_disabled_env: bool,
    /// `YGGTERM_FORCE_SOFTWARE_GL` — the escape hatch for a host whose GPU is broken
    /// or whose probe is wrong. Force beats allow, as everywhere else here.
    force_software_gl: bool,
    /// `YGGTERM_ENABLE_WEBKIT_COMPOSITING` — the historical opt-in, now one input
    /// among several rather than the whole decision.
    enable_compositing: bool,
    /// What the host itself said. [`yggterm_core::gl_probe::GlClass::Unknown`] when
    /// nothing was asked or nothing conclusive came back.
    probe: yggterm_core::gl_probe::GlClass,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxWebkitGlPolicy {
    hardware_gl: bool,
    reason: &'static str,
}

/// The ONE place that decides whether WebKit gets the GPU on this host.
///
/// It used to be `std::env::var_os(ENV_YGGTERM_ENABLE_WEBKIT_COMPOSITING).is_some()`
/// — a hard-coded "no" with an opt-out nobody knew to set, justified by a premise
/// (`this iGPU exposes only llvmpipe`) that was measured false on the very host it
/// named. The premise is now an observation, and the two overrides are what a wrong
/// observation costs: one env var, no rebuild.
///
/// An INCONCLUSIVE probe stays on software. "We could not tell" must never be
/// promoted to "probably fine": that generalization, from one EACCES on one node, is
/// the entire bug this replaces.
#[cfg(target_os = "linux")]
fn linux_webkit_gl_policy_from_input(input: LinuxWebkitGlPolicyInput) -> LinuxWebkitGlPolicy {
    use yggterm_core::gl_probe::GlClass;
    if input.compositing_disabled_env {
        return LinuxWebkitGlPolicy {
            hardware_gl: false,
            reason: "webkit_compositing_disabled_by_env",
        };
    }
    if input.force_software_gl {
        return LinuxWebkitGlPolicy {
            hardware_gl: false,
            reason: "software_gl_forced",
        };
    }
    if input.enable_compositing {
        return LinuxWebkitGlPolicy {
            hardware_gl: true,
            reason: "hardware_gl_forced",
        };
    }
    match input.probe {
        GlClass::Hardware => LinuxWebkitGlPolicy {
            hardware_gl: true,
            reason: "hardware_gl_probed",
        },
        GlClass::Software => LinuxWebkitGlPolicy {
            hardware_gl: false,
            reason: "software_gl_probed",
        },
        GlClass::Unknown => LinuxWebkitGlPolicy {
            hardware_gl: false,
            reason: "software_gl_probe_inconclusive",
        },
    }
}

#[cfg(target_os = "linux")]
fn configure_linux_webkit_compositing() {
    // WebGL (xterm.js 6's GPU renderer — and therefore the TERMINAL's renderer) can
    // only present to screen with WebKitGTK accelerated compositing ENABLED. That has
    // never been in doubt. What was wrong was the next step: we kept compositing on
    // but forced the software-GL / SHM presentation
    //   LIBGL_ALWAYS_SOFTWARE=1 / GALLIUM_DRIVER=llvmpipe -> software GL
    //   WEBKIT_DISABLE_DMABUF_RENDERER=1                  -> SHM presentation
    // on the premise that this host's GPU compositing path crashed in Mesa/EGL. That
    // premise was a GBM probe taking EACCES on card0 while the compositor held DRM
    // master; every other EGL platform on the same machine reported the real GPU. The
    // bill was 4x to 22x the CPU for every frame, terminal frames included.
    //
    // So: ASK, and keep the three settings as ONE decision. The probe answers, the
    // policy turns that into hardware_gl, and hardware_gl feeds arming (under glass
    // needs DMABuf) which with hardware_gl feeds the SHM force. See
    // `linux_webkit_gl_policy_from_input` for the precedence and `shm_force_for_arming`
    // for why splitting them lands in a measured-worse cell.
    let compositing_disabled_env = std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_some();
    let force_software_gl = linux_env_flag_truthy(ENV_YGGTERM_FORCE_SOFTWARE_GL);
    let enable_compositing = linux_env_flag_truthy(ENV_YGGTERM_ENABLE_WEBKIT_COMPOSITING);
    // Only ask when nobody has already answered: an override makes the probe's cost
    // pure waste, and a host with no openable render node has nothing to ask.
    let probe = if compositing_disabled_env
        || force_software_gl
        || enable_compositing
        || !yggterm_core::gl_probe::render_node_present()
    {
        yggterm_core::gl_probe::GlClass::Unknown
    } else {
        std::env::current_exe()
            .ok()
            .map(|exe| yggterm_core::gl_probe::probe_via_child_once(&exe).class)
            .unwrap_or(yggterm_core::gl_probe::GlClass::Unknown)
    };
    let policy = linux_webkit_gl_policy_from_input(LinuxWebkitGlPolicyInput {
        compositing_disabled_env,
        force_software_gl,
        enable_compositing,
        probe,
    });
    // The decision, as DATA. Everything below this line is a mechanical apply — the
    // `if`s that used to live here (and hid the inherited-software-force bug from
    // every test) are in `linux_webkit_gl_env_plan`, where a test can reach them.
    let under_glass = std::env::var("YGGTERM_WEB_SURFACE_UNDER_GLASS").ok();
    let legacy_stack = std::env::var("YGGTERM_WEB_SURFACE_LEGACY_STACK").ok();
    let plan = linux_webkit_gl_env_plan(
        policy,
        compositing_disabled_env,
        yggterm_core::gl_probe::stray_nvidia_egl_vendor(),
        LinuxWebkitGlEnvInherited {
            libgl_always_software_present: std::env::var_os(
                yggterm_core::gl_probe::ENV_LIBGL_ALWAYS_SOFTWARE,
            )
            .is_some(),
            gallium_driver_present: std::env::var_os(yggterm_core::gl_probe::ENV_GALLIUM_DRIVER)
                .is_some(),
            webkit_disable_dmabuf_renderer_present: std::env::var_os(
                yggterm_core::gl_probe::ENV_WEBKIT_DISABLE_DMABUF_RENDERER,
            )
            .is_some(),
            web_surface_under_glass: under_glass.as_deref(),
            web_surface_legacy_stack: legacy_stack.as_deref(),
            egl_vendor_library_filenames_present: std::env::var_os(
                yggterm_core::gl_probe::ENV_EGL_VENDOR_LIBRARY_FILENAMES,
            )
            .is_some(),
        },
    );
    // HARDWARE VIDEO DECODE, from the sanctioned table so the value exists once.
    //
    // WebKitGTK decodes through GStreamer, which loads BOTH the VA (hardware)
    // and libav (software) decoders and chooses by rank. Measured on the live
    // host with a YouTube video playing: `libgstva.so` and `libgstlibav.so`
    // were both mapped into the video WebProcess while it burned 58-61% of one
    // core — software winning a pipeline that had hardware sitting right there,
    // which the user sees as judder rather than as heat.
    //
    // Set only when nobody has answered: an explicit rank is a deliberate act
    // (a bisect, a broken driver) and this is not the layer that overrules it.
    if std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_none()
        && let Some(rank) =
            yggterm_core::presentation_policy::sanctioned(
                yggterm_core::presentation_policy::PresentationTarget::LinuxWayland,
            )
            .iter()
            .find(|var| var.name == "GST_PLUGIN_FEATURE_RANK")
            .and_then(|var| var.value)
    {
        unsafe { std::env::set_var("GST_PLUGIN_FEATURE_RANK", rank) };
    }
    for (key, action) in linux_webkit_gl_env_plan_entries(&plan) {
        match action {
            GlEnvAction::Set(value) => unsafe { std::env::set_var(key, value) },
            GlEnvAction::Remove => unsafe { std::env::remove_var(key) },
            GlEnvAction::Keep => {}
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_webkit_compositing() {}

/// What ONE web process may hold before WebKit starts taking memory back.
///
/// ⚠ THIS IS AN AUDIO/VIDEO FIX as much as a memory one. The old numbers were a
/// 320 MB limit with conservative reclaim at 33% and strict at 50% — i.e. every
/// page over ~160 MB lived permanently in WebKit's most aggressive reclaim, and
/// the first things that reclaim drops are decoded media buffers. Measured on
/// the live host: YouTube tabs at 325 and 416 MB, so the user's video was ALWAYS
/// in strict pressure, and it came out as distorted audio (worse on Bluetooth,
/// where an underrun is audible immediately). 320 MB was sized when a web
/// surface was a small embedded viewer; ychrome is the user's browser now.
///
/// So: size the limit to the MACHINE, and leave real headroom below the
/// thresholds. The limit still bounds a runaway page — round 24 watched one
/// balloon to 3.9 GB — and it is no longer the only defence: per-tab reclaim
/// (2.12.18) bounds how many live pages exist at all.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WebkitMemoryPolicy {
    limit_mb: u32,
    conservative: f64,
    strict: f64,
}
/// A browser's share of one machine: an eighth of RAM per web process, never
/// below 768 MB (a media page needs room to decode) and never above 3 GB (past
/// that the limit stops being a bound at all). `None` (unreadable meminfo) takes
/// a conservative middle rather than the old cliff.
fn webkit_memory_policy(mem_total_kb: Option<u64>) -> WebkitMemoryPolicy {
    const MIN_MB: u64 = 768;
    const MAX_MB: u64 = 3072;
    let limit_mb = match mem_total_kb {
        Some(kb) if kb > 0 => ((kb / 1024) / 8).clamp(MIN_MB, MAX_MB),
        _ => 1024,
    } as u32;
    WebkitMemoryPolicy {
        limit_mb,
        // Three quarters, not a third. The thresholds are FRACTIONS OF THE
        // LIMIT, so on a modest host a low fraction re-creates the very bug this
        // policy exists to kill: at 0.5 an 8 GB machine starts reclaiming at
        // 512 MB, and a 416 MB video page is already crowding it. Reclaim should
        // begin when a page is genuinely outsized for its share, not when it is
        // merely playing a video.
        conservative: 0.75,
        strict: 0.90,
    }
}
/// What the kernel is asked to hold the whole yggterm family to.
///
/// ⛔ **THIS EXISTS BECAUSE WEBKIT'S OWN BOUND CANNOT WORK ON A HOST THAT
/// SWAPS.** `webkit_memory_policy` above is compared against RESIDENT memory
/// (upstream reads `/proc/self/statm`; the shipped library's only
/// `/proc/self/status` field is `VmRSS`), and the kernel is free to push
/// residency arbitrarily far below any threshold by swapping the cache out.
/// Measured on the desktop host: RSS flat in a 586–714 MB band while swap grew
/// 11× and the committed footprint went 649 → 1,362 MB, straight past a
/// conservative threshold of 1,416 MB *of RSS*, with no reclaim. The metric
/// subtracts the evidence of the thing it is measuring.
///
/// ⭐ A cgroup does not have that blind spot: `memory.current` and
/// `memory.swap.current` are the two halves of what the machine actually
/// committed, and bounding both bounds the footprint. **The kernel was already
/// measuring exactly what `VmRSS` cannot** — nothing here is invented, only
/// bounded.
///
/// ⛔ **`MemoryHigh`, NEVER `MemoryMax`.** `MemoryHigh` throttles and reclaims;
/// `MemoryMax` OOM-kills. A hard cap on a browser engine turns a memory spike
/// into a dead web surface, which trades a slow leak for a broken app.
///
/// **The derivation, so this is a rule and not a fitted constant.** WebKit's own
/// sanctioned residency for ONE web process is `MemTotal/8`. The family is that
/// process plus the GUI plus the network children, so the scope is allowed twice
/// that resident — the web process's full existing allowance, and as much again
/// for everything around it — and may swap at most another `MemTotal/8`. The
/// committed ceiling is therefore `3 × MemTotal/8`, derived from the limit the
/// app already applies to itself rather than fitted to one machine.
///
/// ⚠ **What this does and does not claim.** The observed growth is ~366 MB/h
/// with no plateau; this gives it a plateau. On a 15 GB host the ceiling lands
/// above today's ~4.0 GB committed, so it is not expected to reduce steady-state
/// usage — it bounds the unbounded, which is the actual defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryScopePolicy {
    high_mb: u64,
    swap_max_mb: u64,
}

fn memory_scope_policy(mem_total_kb: Option<u64>) -> MemoryScopePolicy {
    // The same floor the WebKit policy reserves for the smallest supported
    // machines, doubled for the family, so a small host is never throttled into
    // uselessness by its own bound.
    const MIN_HIGH_MB: u64 = 1536;
    let web_share_mb = match mem_total_kb {
        Some(kb) if kb > 0 => (kb / 1024) / 8,
        _ => 1024,
    };
    MemoryScopePolicy {
        high_mb: (web_share_mb * 2).max(MIN_HIGH_MB),
        swap_max_mb: web_share_mb.max(MIN_HIGH_MB / 2),
    }
}

/// The exact argv used to re-exec this process inside its own scope.
///
/// ⭐ Built as a pure function so the prohibition on `MemoryMax` and the shape
/// of the command are testable without launching anything.
fn memory_scope_command_args(
    policy: MemoryScopePolicy,
    unit: &str,
    exe: &std::path::Path,
    forwarded: &[String],
) -> Vec<String> {
    let mut args = vec![
        String::from("--user"),
        String::from("--scope"),
        String::from("--quiet"),
        // Do not leave a failed unit behind to collide with the next launch.
        String::from("--collect"),
        format!("--unit={unit}"),
        String::from("-p"),
        format!("MemoryHigh={}M", policy.high_mb),
        String::from("-p"),
        format!("MemorySwapMax={}M", policy.swap_max_mb),
        String::from("--"),
        exe.display().to_string(),
    ];
    args.extend(forwarded.iter().cloned());
    args
}

/// What became of the GUI's attempt to bound itself, so the startup trace can
/// say which of the three it was.
///
/// ⛔ **AN UNBOUNDED GUI USED TO BE INDISTINGUISHABLE FROM A BOUNDED ONE.** The
/// only report of a failure was an `eprintln!` to a stderr nobody reads, so the
/// cap could be lost on a relaunch and nothing anywhere said so. Measured on the
/// owner's laptop: the GUI sat in the plain login-session scope with
/// `memory.high = max` while `systemd-run --user --scope` worked perfectly when
/// tried by hand a few minutes later — a transient failure that left no trace of
/// having happened. This enum exists so the next one is on the record.
///
/// ⚠ **The loss is coupled to the crash rate, which is why it matters.** The GUI
/// SIGSEGVs in the GL compositing path several times a day; every relaunch is
/// another chance to come back unbounded, and the leak this caps is monotonic.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MemoryScopeOutcome {
    /// Inside the scope: the marker the re-exec stamps on its child is present.
    Entered,
    /// ⛔ INHERITED, NOT ARMED — and the difference is the whole point of this
    /// field. A GUI relaunched by a running GUI (`server app update restart`) is
    /// forked from a process that is already inside a scope, so it inherits BOTH
    /// the cgroup and the `…_SCOPE_ACTIVE` marker, takes the idempotent early
    /// return, and never runs `systemd-run` at all. That used to report
    /// `entered` / `bounded: true`, identically to a GUI that armed its own —
    /// which made the one field added to answer *"is this GUI capped"* unable to
    /// answer *"did this GUI cap itself"*.
    ///
    /// ⚠ The two halves of the inheritance travel by DIFFERENT mechanisms: the
    /// marker rides the environment, the bound rides the cgroup. They coincide
    /// for a plain fork and can come apart for anything that carries an
    /// environment further than it carries a process — at which point trusting
    /// the marker alone reports `bounded: true` for a GUI in the plain login
    /// scope. So this variant does not trust it: it carries what
    /// `/proc/self/cgroup` + `memory.high` actually say.
    Inherited { unit: String, bounded: bool },
    /// `YGGTERM_MEMORY_SCOPE=0` — a bound was declined, so its absence is not a
    /// fault.
    OptedOut,
    /// A CLI subcommand, which must never be re-executed into a scope.
    NotAttempted,
    /// Running unbounded, and this is why.
    Fallback(String),
}

#[cfg(target_os = "linux")]
impl MemoryScopeOutcome {
    fn label(&self) -> &'static str {
        match self {
            Self::Entered => "entered",
            Self::Inherited { .. } => "inherited",
            Self::OptedOut => "opted_out",
            Self::NotAttempted => "not_attempted",
            Self::Fallback(_) => "fallback",
        }
    }

    /// Whether this process is actually capped. An inherited scope answers from
    /// the cgroup rather than from the marker that put it on this path.
    fn bounded(&self) -> bool {
        match self {
            Self::Entered => true,
            Self::Inherited { bounded, .. } => *bounded,
            _ => false,
        }
    }

    /// The scope this process is in, when it did not create it. Named because
    /// the unit carries the pid of whoever DID create it, which is how "the
    /// scope name does not match my pid" reads as inheritance rather than as
    /// the mystery it was.
    fn inherited_unit(&self) -> Option<&str> {
        match self {
            Self::Inherited { unit, .. } => Some(unit.as_str()),
            _ => None,
        }
    }

    fn fallback_reason(&self) -> Option<&str> {
        match self {
            Self::Fallback(reason) => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// Read the cgroup this process is in and whether it carries a real memory
/// bound. ⛔ A `memory.high` of `max` is a cgroup with no bound, which is
/// exactly what an environment marker carried past its process would land in.
#[cfg(target_os = "linux")]
fn current_cgroup_memory_bound() -> (String, bool) {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.rsplit_once(':').map(|(_, path)| path.to_string()))
        })
        .unwrap_or_default();
    let unit = cgroup
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_string();
    let high = std::fs::read_to_string(format!("/sys/fs/cgroup{cgroup}/memory.high")).ok();
    (unit, cgroup_memory_high_is_a_bound(high.as_deref()))
}

/// Whether a `memory.high` reading is an actual ceiling.
///
/// ⛔ `max` is the kernel's word for NO limit, and it is what an unbounded
/// cgroup returns — so a reader that only checks "did the file exist and parse"
/// calls the login scope bounded. Unreadable is also not bounded: a missing file
/// means the question could not be answered, and BLIND IS NOT BOUNDED.
fn cgroup_memory_high_is_a_bound(reading: Option<&str>) -> bool {
    match reading.map(str::trim) {
        None | Some("") | Some("max") => false,
        Some(value) => value.parse::<u64>().is_ok_and(|bytes| bytes > 0),
    }
}

/// Ask systemd for a throwaway scope carrying the very properties the real one
/// will carry, and run `/bin/true` in it.
///
/// ⛔ **THIS IS WHAT MAKES THE FALL-THROUGH REAL RATHER THAN CLAIMED.** `exec`
/// returns only when the *exec itself* fails; if it succeeds and `systemd-run`
/// then refuses the scope, this process has already been replaced, `systemd-run`
/// exits with the error, and **the GUI never starts at all**. There is no line
/// after `exec` for that case to land on. So the refusal has to be discovered
/// while we are still a process that can carry on: a probe that fails costs one
/// short-lived subprocess, where the real attempt failing costs the app.
///
/// Spawned rather than exec'd, and `--collect` keeps a failed probe from leaving
/// a unit behind to collide with the launch that follows it.
#[cfg(target_os = "linux")]
fn memory_scope_preflight(policy: MemoryScopePolicy, unit: &str) -> Result<(), String> {
    let args = memory_scope_preflight_args(policy, unit);
    match Command::new("systemd-run").args(&args).output() {
        Err(error) => Err(format!("systemd-run could not be run ({error})")),
        Ok(probe) if probe.status.success() => Ok(()),
        Ok(probe) => {
            let stderr = String::from_utf8_lossy(&probe.stderr);
            let detail = stderr
                .lines()
                .filter(|line| !line.trim().is_empty())
                .next_back()
                .unwrap_or("no stderr")
                .trim()
                .to_string();
            Err(format!(
                "systemd-run refused a probe scope ({}): {detail}",
                probe.status
            ))
        }
    }
}

#[cfg(target_os = "linux")]
fn memory_scope_preflight_args(policy: MemoryScopePolicy, unit: &str) -> Vec<String> {
    vec![
        String::from("--user"),
        String::from("--scope"),
        String::from("--quiet"),
        String::from("--collect"),
        format!("--unit={unit}"),
        String::from("-p"),
        format!("MemoryHigh={}M", policy.high_mb),
        String::from("-p"),
        format!("MemorySwapMax={}M", policy.swap_max_mb),
        String::from("--"),
        String::from("/bin/true"),
    ]
}

/// Re-exec into a private systemd user scope, if asked to and if it can.
///
/// ⛔ **FALLING THROUGH IS THE POINT — BUT `exec` ALONE DOES NOT DELIVER IT.**
/// This comment used to claim that "every way this can go wrong lands on the
/// next line and the GUI starts exactly as it would have", and that is a
/// guarantee the code could not make: `exec` returns ONLY when the exec itself
/// fails, so a `systemd-run` that starts and *then* refuses the scope takes the
/// GUI down with it, with no next line to land on. The preflight above is what
/// closes that gap; the fall-through below covers the rest.
///
/// ⇒ Returns the outcome instead of swallowing it, because an unbounded GUI that
/// never says so is the failure that actually happened.
#[cfg(target_os = "linux")]
fn enter_memory_scope_if_requested() -> MemoryScopeOutcome {
    use std::os::unix::process::CommandExt;

    // ⛔ DEFAULT ON. This shipped opt-in and therefore bounded nothing.
    //
    // The leak it exists to cap is real, monotonic and measured on every single
    // web process: 8 of 8 sampled over 24 h started at ~260 MB and climbed
    // without plateau, one reaching **1,504 MB in 12.4 h**. Meanwhile the
    // owner's laptop sat at 11 GB of 15 GB swap and he reported the machine
    // burning. The GUI was in the plain login-session scope with
    // `memory.high = max`, `memory.swap.max = max` — no bound of any kind —
    // because this function returned on its first line.
    //
    // ⚠ A remedy that is switched off is not a remedy, and this one had already
    // been designed, derived, tested and documented. The only thing missing was
    // anyone getting it.
    //
    // Opting OUT is still one variable, and every failure path below already
    // falls through to a normal start, which is what makes defaulting this on
    // safe: no systemd, no user manager or a refused property costs nothing.
    if matches!(
        std::env::var(ENV_YGGTERM_MEMORY_SCOPE).ok().as_deref(),
        Some("0") | Some("false") | Some("no")
    ) {
        return MemoryScopeOutcome::OptedOut;
    }
    // Idempotent: the child we exec carries this, so it never re-enters.
    //
    // ⛔ BUT THE MARKER IS NOT PROOF OF A BOUND — it only proves that SOMETHING
    // upstream entered a scope. A GUI relaunched by a running GUI inherits the
    // marker and the cgroup together and lands here without running
    // `systemd-run` once; anything that carries the environment further than the
    // process would land here with no bound at all. So the answer comes from the
    // cgroup, and the outcome says which of the two happened.
    if std::env::var_os(ENV_YGGTERM_MEMORY_SCOPE_ACTIVE).is_some() {
        let (unit, bounded) = current_cgroup_memory_bound();
        if !bounded {
            eprintln!(
                "yggterm: the memory-scope marker is set but this process is in an unbounded \
                 cgroup ({unit}); running unbounded"
            );
        }
        return MemoryScopeOutcome::Inherited { unit, bounded };
    }
    let Ok(exe) = std::env::current_exe() else {
        return MemoryScopeOutcome::Fallback(String::from(
            "the running binary could not be located",
        ));
    };
    // ⛔ A DEPLOY REPLACES THIS BINARY WHILE IT IS RUNNING, and `current_exe`
    // then reports the old inode with " (deleted)" glued to the path. Handing
    // that to `systemd-run` asks it to launch something that no longer exists,
    // so the scope fails for a reason that has nothing to do with systemd. The
    // GUI that crashed on the owner's laptop was running exactly such a deleted
    // binary. Falling through here starts the GUI from the image it already has.
    if !exe.exists() {
        let reason = format!(
            "the running binary has been replaced on disk ({}), so no scope could re-exec it",
            exe.display()
        );
        eprintln!("yggterm: {reason}; starting unbounded instead");
        return MemoryScopeOutcome::Fallback(reason);
    }
    let policy = memory_scope_policy(read_mem_total_kb());
    let pid = std::process::id();
    if let Err(reason) = memory_scope_preflight(policy, &format!("yggterm-gui-preflight-{pid}")) {
        eprintln!("yggterm: {reason}; starting unbounded instead");
        return MemoryScopeOutcome::Fallback(reason);
    }
    let forwarded: Vec<String> = std::env::args().skip(1).collect();
    let unit = format!("yggterm-gui-{pid}");
    let args = memory_scope_command_args(policy, &unit, &exe, &forwarded);
    let error = Command::new("systemd-run")
        .args(&args)
        .env(ENV_YGGTERM_MEMORY_SCOPE_ACTIVE, "1")
        .exec();
    let reason = format!("exec of systemd-run failed after its probe succeeded ({error})");
    eprintln!(
        "yggterm: could not enter a private memory scope ({error}); starting unbounded instead"
    );
    MemoryScopeOutcome::Fallback(reason)
}

fn read_mem_total_kb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo.lines().find_map(|line| {
        let rest = line.strip_prefix("MemTotal:")?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

#[cfg(target_os = "linux")]
fn configure_linux_webkit_memory_policy() {
    if std::env::var_os(ENV_YGGTERM_WEBKIT_CACHE_MODEL).is_none() {
        // `document-viewer` is WebKit's own name for "disable the cache
        // completely" — correct when a web surface was an embedded viewer, wrong
        // now that ychrome is the user's browser: every navigation and every
        // reload refetched every byte, and nothing was ever served warm.
        // `web-browser` is the model WebKit sizes for a browser. The bound on it
        // is not this knob but the memory policy below (a hard limit plus the
        // conservative/strict/kill thresholds) and, since 2.12.18, per-tab
        // reclaim.
        //
        // ⛔ WHAT THAT POLICY ACTUALLY BOUNDS IS **RESIDENCY, NOT FOOTPRINT** —
        // this comment used to claim "so caching more does not mean growing
        // without end", and that guarantee is one the code cannot make.
        // WebKit evaluates the limit against RESIDENT memory only: upstream WTF
        // reads `/proc/self/statm`, which has no swap field, and the only
        // `/proc/self/status` field name in the shipped library is `VmRSS` —
        // there is no `VmSwap` in it anywhere. So on a host that swaps, the
        // kernel evicts the cold cache, RSS falls, and the threshold is never
        // reached while the committed footprint keeps climbing past it.
        // Measured: RSS flat in a 586-714 MB band while swap grew 11x and
        // committed went 649 -> 1,362 MB, against a conservative threshold of
        // 1,416 MB of RSS.
        //
        // ⛔ DO NOT "FIX" THIS BY LOWERING THE NUMBER. No constant can bound a
        // footprint through an RSS-valued comparison, because the kernel can
        // push RSS below any threshold by swapping. Making it trip against the
        // band above needs a limit near the 768 MB floor meant for the smallest
        // supported machines, which abolishes the rule that derives it.
        // The limit below is a sound derived rule for what it really is: an
        // eighth of RAM, resident. See `docs/pending-bugs.md` for the only two
        // honest options.
        // Override with YGGTERM_WEBKIT_CACHE_MODEL=document-viewer to get the
        // old cacheless behaviour back.
        unsafe { std::env::set_var(ENV_YGGTERM_WEBKIT_CACHE_MODEL, "web-browser") };
    }
    let policy = webkit_memory_policy(read_mem_total_kb());
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB).is_none() {
        unsafe {
            std::env::set_var(
                ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB,
                policy.limit_mb.to_string(),
            )
        };
    }
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD).is_none() {
        unsafe {
            std::env::set_var(
                ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD,
                format!("{:.2}", policy.conservative),
            )
        };
    }
    if std::env::var_os(ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD).is_none() {
        unsafe {
            std::env::set_var(
                ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD,
                format!("{:.2}", policy.strict),
            )
        };
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
    let mut server = YggtermServer::new(prefer_ghostty_backend, host.clone(), theme);
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
            let shutdown_daemon = status(&endpoint).ok().is_some_and(|runtime| {
                signal_shutdown_policy_allows_daemon_shutdown(
                    remaining_clients,
                    runtime.terminal_session_count,
                    runtime.owned_terminal_session_count,
                    runtime.preserved_terminal_owner_count,
                    true,
                )
            });
            if shutdown_daemon {
                let _ = shutdown(&endpoint);
            }
        }
        std::process::exit(130);
    });
}

fn signal_shutdown_policy_allows_daemon_shutdown(
    remaining_clients: usize,
    terminal_session_count: usize,
    owned_terminal_session_count: usize,
    preserved_terminal_owner_count: usize,
    status_reachable: bool,
) -> bool {
    status_reachable
        && remaining_clients == 0
        && terminal_session_count == 0
        && owned_terminal_session_count == 0
        && preserved_terminal_owner_count == 0
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

/// Which running client, if any, this fresh process should hand off to.
///
/// Pure, because the bug it now forbids shipped in the filter chain and nothing
/// could fail over it: the executable-only predicate matched a shadow, the
/// shadow was the newest record, and the newest record won.
fn select_focus_handoff_target(
    records: &[ClientInstanceRecord],
    current_exe: &std::path::Path,
) -> Option<u32> {
    records
        .iter()
        .filter(|record| record_matches_executable(record.executable_path.as_deref(), current_exe))
        // ⛔ AND IT MUST BE THE USER'S WINDOW, NOT MERELY THE SAME BINARY. A
        // shadow view runs the identical executable, so an executable-only
        // filter picks the newest shadow as "the GUI already running" — which
        // is the routing rule `ClientInstanceRecord::client_role` was added to
        // forbid, arriving at the one call site the July fix never reached.
        .filter(|record| record.is_active_gui())
        .max_by_key(|record| record.started_at_ms)
        .map(|record| record.pid)
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
    let Some(target_pid) = select_focus_handoff_target(&active_records, current_exe) else {
        return Ok(());
    };
    unsafe {
        std::env::set_var("YGGTERM_APP_CONTROL_PID", target_pid.to_string());
    }
    // ⛔ THE VERDICT COMES FROM THE RESPONSE, NOT FROM THE CALL. The old line
    // was `run_app_control_focus_window(3_000).is_ok()`, which is true whenever
    // the request completed a round trip — including when the reply's own text
    // is "app-control focus request did not produce native window focus". This
    // process then exited, having handed off to a window that never took focus.
    //
    // ⚠ An unreadable answer must not read as "focus succeeded": on error we
    // keep going and open a window. Two windows is a nuisance; zero windows is
    // a desktop with no terminal, which is what happened.
    let focused = app_control_focus_window_took_focus(3_000).unwrap_or(false);
    unsafe {
        std::env::remove_var("YGGTERM_APP_CONTROL_PID");
    }
    if focused {
        std::process::exit(0);
    }
    Ok(())
}

fn maybe_retire_superseded_same_home_clients(
    home_dir: &std::path::Path,
    args: &[String],
    current_exe: &std::path::Path,
) -> Result<()> {
    if !args.is_empty() || std::env::var_os(ENV_YGGTERM_ALLOW_MULTI_WINDOW).is_some() {
        return Ok(());
    }
    let endpoint = default_endpoint(home_dir);
    let active_records = active_client_instance_records(home_dir, &endpoint)?;
    let current_pid = std::process::id();
    let current_scope = current_signal_client_scope();
    for record in active_records {
        if !should_retire_superseded_client(&record, current_pid, current_exe, &current_scope) {
            continue;
        }
        append_trace_event(
            home_dir,
            "gui",
            "startup",
            "retire_superseded_client_begin",
            serde_json::json!({
                "pid": record.pid,
                "executable_path": record.executable_path.as_deref(),
                "display": record.display.as_deref(),
                "wayland_display": record.wayland_display.as_deref(),
                "xdg_session_id": record.xdg_session_id.as_deref(),
            }),
        );
        let close_ok = terminate_superseded_client_pid(record.pid);
        let exited = wait_for_process_exit(record.pid, Duration::from_millis(2_500));
        append_trace_event(
            home_dir,
            "gui",
            "startup",
            "retire_superseded_client_end",
            serde_json::json!({
                "pid": record.pid,
                "close_ok": close_ok,
                "strategy": superseded_client_retirement_strategy_label(),
                "exited": exited,
            }),
        );
    }
    Ok(())
}

fn main_should_retire_superseded_clients_before_shell(_args: &[String]) -> bool {
    // The shell owns superseded-client retirement because it can first query the
    // outgoing GUI over app-control and preserve the active terminal selection.
    false
}

/// Is this client running a binary that no longer exists on disk?
///
/// A deploy REPLACES the file rather than removing it, so the path still
/// resolves while the running process keeps the old inode — which is why a path
/// existence check cannot answer this and `/proc/<pid>/exe` must be read
/// directly. The kernel appends `(deleted)` to that link, and it is the one
/// unambiguous statement that a process can no longer be the build that is
/// installed.
fn client_process_runs_a_deleted_binary(pid: u32) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .map(|target| target.to_string_lossy().ends_with(" (deleted)"))
        .unwrap_or(false)
}

fn should_retire_superseded_client(
    record: &ClientInstanceRecord,
    current_pid: u32,
    current_exe: &std::path::Path,
    current_scope: &SignalClientScope,
) -> bool {
    if record.pid == current_pid {
        return false;
    }
    if !signal_client_record_scope_matches(record, current_scope) {
        return false;
    }
    // ⛔ A GUI on a DELETED binary is stale no matter what its recorded path
    // says, and this is the case that cost a user half a day.
    //
    // Retirement used to require the executable PATH to differ, which reads as
    // "a new version supersedes an old one". A deploy that overwrites the
    // binary in place leaves the path IDENTICAL while the running process keeps
    // the old inode, so the orphan matched `record_matches_executable`, was
    // read as the current build, and was kept. It then owned the user's window
    // for 12.4 hours across several restarts — each of which added another GUI
    // instead of replacing it — while painting a surface whose sidebars never
    // came back. Measured cost: 3.63 core-hours, 63% of all GUI CPU that day,
    // at an entirely NORMAL per-second rate. The waste was DURATION.
    //
    // ⚠ Deliberately narrower than "retire any older same-scope client": two
    // GUIs of the SAME LIVE build on one display may or may not be legitimate,
    // that question is open, and answering it is not this fix's job.
    if client_process_runs_a_deleted_binary(record.pid) {
        return true;
    }
    !record
        .executable_path
        .as_deref()
        .is_some_and(|path| record_matches_executable(Some(path), current_exe))
}

fn signal_client_record_scope_matches(
    record: &ClientInstanceRecord,
    current: &SignalClientScope,
) -> bool {
    let candidate = SignalClientScope {
        display: record.display.clone(),
        wayland_display: record.wayland_display.clone(),
        xdg_session_id: record.xdg_session_id.clone(),
        xdg_runtime_dir: record.xdg_runtime_dir.clone(),
        xauthority: record.xauthority.clone(),
    };
    signal_client_scope_matches(&candidate, current)
}

fn terminate_superseded_client_pid(pid: u32) -> bool {
    terminate_gui_client_process(pid);
    true
}

#[cfg(test)]
fn superseded_client_close_command() -> yggterm_server::AppControlCommand {
    yggterm_server::AppControlCommand::CloseWindowPreservingSessions {
        reason: Some("superseded-client-handoff".to_string()),
        // A client handoff is not a deploy: the superseding client is already
        // taking the window, so an agent lease must not block it.
        force: true,
    }
}

fn superseded_client_retirement_strategy_label() -> &'static str {
    "kill_process_only_no_client_cleanup"
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !signal_process_is_alive(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
fn superseded_client_termination_signal() -> i32 {
    libc::SIGKILL
}

#[cfg(unix)]
fn terminate_gui_client_process(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    unsafe {
        libc::kill(pid as i32, superseded_client_termination_signal());
    }
}

#[cfg(target_os = "windows")]
fn terminate_gui_client_process(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn terminate_gui_client_process(_pid: u32) {}

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
    if !should_handoff_to_preferred_executable(args) {
        return Ok(());
    }
    if std::env::var_os(ENV_YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF).is_some() {
        return Ok(());
    }
    let Some(preferred) = install_context.preferred_executable.as_ref() else {
        return Ok(());
    };
    // ⛔ Same guard as the headless binary's: a stale — or self-contradictory —
    // install record must not send a newer build down to an older one. The GUI
    // half of the 2026-08-07 live finding; see `handoff_target_is_usable` for
    // why the TARGET PATH and not the recorded version is what decides.
    if !yggterm_core::handoff_target_is_usable(
        env!("CARGO_PKG_VERSION"),
        &install_context.current_version,
        preferred,
    ) {
        return Ok(());
    }
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

fn should_handoff_to_preferred_executable(args: &[String]) -> bool {
    args.is_empty()
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[allow(dead_code)]
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
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(message.as_bytes());
        let _ = stderr.write_all(b"\n");
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
        ..Default::default()
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

    /// ⚠ THE AUDIO FIX. A media page must not live in permanent memory pressure:
    /// WebKit's reclaim drops decoded buffers first, which on the live host came
    /// out as distorted YouTube audio (325-416 MB tabs against a 320 MB limit
    /// whose strict threshold sat at 160 MB — every video always in the most
    /// aggressive reclaim). What is locked is the PROPERTY, not the numbers: a
    /// real media page has headroom, and a runaway page is still bounded.
    #[test]
    fn a_media_page_is_not_born_in_memory_pressure_and_a_runaway_is_still_bounded() {
        // A YouTube tab as measured on the live host, and the balloon round 24
        // watched.
        const MEDIA_PAGE_MB: f64 = 416.0;
        const RUNAWAY_MB: f64 = 3_900.0;
        // 8 GB and up. Below that a host genuinely cannot give one page 1.5x a
        // video's working set, and pretending otherwise would be the dishonest
        // half of this lock — what it asserts for a small machine is the floor.
        for total_gb in [8u64, 16, 32, 64] {
            let policy = super::webkit_memory_policy(Some(total_gb * 1024 * 1024));
            let limit = f64::from(policy.limit_mb);
            let conservative = limit * policy.conservative;
            assert!(
                conservative > MEDIA_PAGE_MB * 1.5,
                "on a {total_gb} GB host a {MEDIA_PAGE_MB} MB video page must sit \
                 well below the first reclaim threshold ({conservative} MB), or \
                 its audio is what pays"
            );
            assert!(
                limit < RUNAWAY_MB,
                "the limit must still bound a runaway page on a {total_gb} GB host"
            );
            assert!(
                policy.strict > policy.conservative && policy.strict < 1.0,
                "strict pressure comes after conservative, and before the limit"
            );
        }
        // The clamps: a small machine still gets room to decode, a huge one does
        // not get a limit so large it stops being a bound.
        assert_eq!(
            super::webkit_memory_policy(Some(2 * 1024 * 1024)).limit_mb,
            768,
            "even a 2 GB host gives a page room to decode media"
        );
        assert_eq!(
            super::webkit_memory_policy(Some(512 * 1024 * 1024)).limit_mb,
            3072,
            "a 512 GB host does not get an unbounded page"
        );
        // Unreadable meminfo takes a middle, never the old 320 MB cliff.
        let unknown = super::webkit_memory_policy(None);
        assert!(
            unknown.limit_mb >= 768,
            "an unknown machine still must not put a media page in permanent \
             pressure: {unknown:?}"
        );
    }

    /// ⛔ AN INHERITED MARKER IS NOT AN INHERITED BOUND.
    ///
    /// `server app update restart` relaunches the GUI from inside the running
    /// GUI, so the successor inherits `…_SCOPE_ACTIVE` and reports itself
    /// bounded without running `systemd-run`. Caught live on the desktop host
    /// 2026-08-14: the trace said `entered` / `bounded: true` while the process
    /// sat in `yggterm-gui-<the dead pid>.scope`. The bound was real THAT time
    /// because the cgroup came with the fork — but the marker travels by
    /// environment and the bound travels by cgroup, and the reading below is the
    /// only one of the two that can tell them apart.
    #[test]
    fn a_cgroup_is_bounded_only_when_memory_high_names_a_ceiling() {
        assert!(super::cgroup_memory_high_is_a_bound(Some("3959422976")));
        assert!(super::cgroup_memory_high_is_a_bound(Some("3959422976\n")));
        // The kernel's word for "no limit" — the login scope's own answer, and
        // the reading that must never be mistaken for a cap.
        assert!(!super::cgroup_memory_high_is_a_bound(Some("max")));
        assert!(!super::cgroup_memory_high_is_a_bound(Some("max\n")));
        // BLIND IS NOT BOUNDED: no cgroup file is an unanswered question.
        assert!(!super::cgroup_memory_high_is_a_bound(None));
        assert!(!super::cgroup_memory_high_is_a_bound(Some("")));
        // A zero ceiling is not a working bound either, and neither is garbage.
        assert!(!super::cgroup_memory_high_is_a_bound(Some("0")));
        assert!(!super::cgroup_memory_high_is_a_bound(Some("unlimited")));
    }

    /// ⛔ THE SCOPE THROTTLES, IT NEVER KILLS.
    ///
    /// `MemoryHigh` applies reclaim pressure; `MemoryMax` OOM-kills. On a
    /// browser engine a hard cap turns a memory spike into a dead web surface,
    /// which trades a slow leak for a broken app — so the string must never
    /// appear in the command at all.
    #[test]
    fn the_memory_scope_bounds_with_pressure_and_never_with_a_hard_cap() {
        let policy = super::memory_scope_policy(Some(16 * 1024 * 1024));
        let args = super::memory_scope_command_args(
            policy,
            "yggterm-gui-1234",
            std::path::Path::new("/usr/bin/yggterm"),
            &[String::from("--flag")],
        );
        let rendered = args.join(" ");
        assert!(
            !rendered.contains("MemoryMax"),
            "MemoryMax OOM-kills; a killed web process is a worse outcome than \
             the growth this bounds. Command was: {rendered}"
        );
        assert!(
            rendered.contains("MemoryHigh=") && rendered.contains("MemorySwapMax="),
            "both halves are required: MemoryHigh bounds what is RESIDENT and \
             MemorySwapMax bounds what is SWAPPED, and it is their sum that is \
             the committed footprint WebKit's own RSS-valued bound cannot see. \
             Command was: {rendered}"
        );
        assert!(
            rendered.contains("--user") && rendered.contains("--scope"),
            "a user scope, so it needs no privilege and dies with the session"
        );
        assert_eq!(
            args.last().map(String::as_str),
            Some("--flag"),
            "the GUI's own arguments must survive the re-exec"
        );
        assert!(
            args.iter().any(|arg| arg == "/usr/bin/yggterm"),
            "the re-exec must target this executable: {rendered}"
        );
    }

    /// The bound is DERIVED from the limit the app already applies to itself,
    /// not fitted to one machine — this lane has twice been burned by a constant
    /// tuned against a single host.
    #[test]
    fn the_scope_bound_is_derived_from_the_webkit_share_and_holds_on_every_host() {
        for total_gb in [8u64, 16, 32, 64] {
            let kb = total_gb * 1024 * 1024;
            let web_share = u64::from(super::webkit_memory_policy(Some(kb)).limit_mb);
            let scope = super::memory_scope_policy(Some(kb));
            assert!(
                scope.high_mb >= web_share,
                "on a {total_gb} GB host the family's resident bound ({} MB) must \
                 not be tighter than the single web process's own sanctioned \
                 share ({web_share} MB), or the scope throttles the app inside \
                 its own budget",
                scope.high_mb
            );
            assert!(
                scope.swap_max_mb > 0 && scope.swap_max_mb <= scope.high_mb,
                "swap must be bounded and must not exceed the resident bound: {scope:?}"
            );
        }
        // A small machine is not throttled into uselessness by its own bound.
        let small = super::memory_scope_policy(Some(2 * 1024 * 1024));
        assert!(
            small.high_mb >= 1536,
            "a 2 GB host still gets a floor it can run in: {small:?}"
        );
        // Unreadable meminfo must not produce a bound of zero, which would
        // throttle instantly.
        let unknown = super::memory_scope_policy(None);
        assert!(
            unknown.high_mb >= 1536 && unknown.swap_max_mb > 0,
            "an unknown machine takes a safe middle, never a bound of nothing: {unknown:?}"
        );
    }

    /// ⛔ DEFAULT **ON**, AND THE RE-EXEC MUST FALL THROUGH.
    ///
    /// A source guard, because both properties are about code that only runs on
    /// a real launch: the failure mode of this change is *the GUI does not
    /// start*, and neither an absent opt-in nor a failed `systemd-run` may ever
    /// produce it.
    ///
    /// ⚠ This heading said DEFAULT **OFF** while every assertion below required
    /// the opposite — left behind when the default was flipped. The body of a
    /// test is checked by the compiler; its prose is checked by nobody, so a
    /// stale heading on a passing test is a signpost pointing the wrong way at
    /// exactly the moment someone is trusting it.
    #[test]
    fn the_memory_scope_is_default_on_and_a_failure_to_enter_it_still_starts_the_gui() {
        // ⛔ SCAN THE PRODUCT HALF ONLY. `include_str!` pulls in this test too,
        // and the anchor string below appears in both. While the real signature
        // matched, `find` happened to hit the function first and the test looked
        // sound; the moment the signature gained a return type, the anchor
        // silently relocated to this test's OWN source and every assertion began
        // describing the test rather than the code. A source guard that can
        // match itself is not guarding anything.
        let source = include_str!("main.rs");
        let product = source
            .split("mod tests {")
            .next()
            .expect("main.rs has a product half above its tests");
        let start = product
            .find("fn enter_memory_scope_if_requested()")
            .expect("the scope entry point must exist");
        let body = &product[start..];
        let end = body[1..]
            .find("\nfn ")
            .map(|at| at + 1)
            .unwrap_or(body.len());
        let body = &body[..end];

        // ⛔ DEFAULT ON, opt-OUT only. This assertion used to require the
        // opposite — and it kept passing when the default was flipped, because
        // it only checked that the variable and a `return` both appeared
        // somewhere in the body. A test that passes under both behaviours
        // asserts neither; its NAME was the only thing that still claimed the
        // old contract, which is worse than having no test, because the name is
        // what a reader trusts.
        //
        // The bound this gates was measured off: the owner's laptop ran at
        // 11 GB of 15 GB swap with `memory.high = max`, while every sampled web
        // process climbed monotonically past 1.5 GB. A remedy that ships
        // switched off is not a remedy.
        assert!(
            body.contains("Some(\"0\") | Some(\"false\") | Some(\"no\")"),
            "the scope must be DEFAULT ON: the early return has to fire only on \
             an explicit opt-OUT, never on an unset variable"
        );
        assert!(
            !body.contains("Some(\"1\") | Some(\"true\") | Some(\"yes\")"),
            "an opt-IN gate would leave the leak unbounded for everyone who \
             never sets the variable, which is everyone"
        );
        assert!(
            body.contains("ENV_YGGTERM_MEMORY_SCOPE_ACTIVE"),
            "the re-executed child must be stamped, or it re-enters forever"
        );
        assert!(
            body.contains(".exec()") && body.contains("eprintln!"),
            "`exec` returns ONLY on failure, so there must be code AFTER it: \
             every way entering the scope can fail has to fall through and start \
             the GUI unbounded rather than not at all"
        );
        assert!(
            !body.contains("expect(") && !body.contains("unwrap()"),
            "nothing in the launch path may panic — a bad bound must cost memory, \
             never the app"
        );
        // ⛔ `exec` covers only the case where the exec ITSELF fails. A
        // `systemd-run` that starts and then refuses the scope has already
        // replaced this process, and takes the GUI down with it — there is no
        // line after `exec` for that to land on. The probe is what turns the
        // fall-through from a claim into a mechanism.
        assert!(
            body.contains("memory_scope_preflight("),
            "the scope must be probed with a throwaway unit BEFORE the real \
             `exec`, or a refused property stops the GUI starting at all"
        );
        assert!(
            body.contains("exe.exists()"),
            "a deploy replaces this binary while it runs and `current_exe` then \
             reports a path ending in ' (deleted)'; re-execing that can only fail"
        );
        // The failure that actually happened was not the cap being lost — it was
        // the cap being lost in silence, so that a bounded and an unbounded GUI
        // were indistinguishable from outside.
        assert!(
            body.contains("MemoryScopeOutcome::Fallback"),
            "every fall-through must RETURN its reason: an unbounded GUI that \
             does not say so is the defect this exists to prevent"
        );
    }

    /// ⛔ BOTH BINARIES MUST ROUTE `trace` THROUGH THE ONE PARSER.
    ///
    /// The `--limit` defect survived because the dispatch was **duplicated**:
    /// `yggterm` and `yggterm-headless` each carried their own copy, and
    /// `yggterm-headless` is the one agents actually type. Fixing the GUI binary
    /// alone would have left the bug exactly where it was being hit, and the
    /// suite would have gone green over it. ⇒ What is locked here is not the
    /// behaviour (the test below does that) but the **absence of a second
    /// implementation to drift from**.
    #[test]
    fn both_binaries_route_every_trace_verb_through_one_limit_parser() {
        for (binary, source) in [
            ("yggterm", include_str!("main.rs")),
            ("yggterm-headless", include_str!("bin/yggterm-headless.rs")),
        ] {
            let product = source
                .split("mod tests {")
                .next()
                .expect("a product half above the tests");
            assert!(
                product.contains("parse_trace_limit(&args, 200)"),
                "{binary} must ask the shared parser for its trace limit"
            );
            // The idiom that WAS the bug: a bare positional parse with a silent
            // fallback, which reads `--limit` as a line count and returns 200.
            assert!(
                !product.contains(".and_then(|value| value.parse::<usize>().ok())\n            .unwrap_or(200)"),
                "{binary} still parses a trace limit positionally with a silent \
                 default — that idiom IS the defect: it reads the literal \
                 \"--limit\" as a count, fails, and returns 200 for every request"
            );
        }
    }

    /// ⛔ `--limit N` MUST BE HONOURED, NOT MERELY ACCEPTED.
    ///
    /// The bug this locks was invisible precisely because nothing failed:
    /// `server trace tail --limit 500` parsed the literal `"--limit"` as a line
    /// count, failed, and silently fell back to 200. Asking for 50 and asking
    /// for 2000 both returned 200. So the assertion that matters is not "does it
    /// accept the flag" but **"does a different request produce a different
    /// number"**.
    #[test]
    fn the_trace_limit_flag_changes_the_answer_and_a_bad_one_is_refused() {
        let argv = |raw: &[&str]| raw.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // The exact invocation that used to be ignored, on all four verbs that
        // share the parser.
        for verb in ["tail", "follow", "bundle", "transitions"] {
            let args = argv(&["server", "trace", verb, "--limit", "500"]);
            assert_eq!(
                super::parse_trace_limit(&args, 200).ok(),
                Some(500),
                "`server trace {verb} --limit 500` must ask for 500, not the default"
            );
        }

        // Two different requests must not collapse to the same answer — the
        // property that a silent default breaks.
        let fifty = super::parse_trace_limit(&argv(&["server", "trace", "tail", "--limit", "50"]), 200);
        let many = super::parse_trace_limit(&argv(&["server", "trace", "tail", "--limit", "2000"]), 200);
        assert_ne!(
            fifty.ok(),
            many.ok(),
            "asking for 50 and asking for 2000 returned the same count, which is \
             the original defect exactly"
        );

        // The positional form still works, because it is what the verb took first.
        assert_eq!(
            super::parse_trace_limit(&argv(&["server", "trace", "tail", "750"]), 200).ok(),
            Some(750)
        );
        // An unrelated flag is not a malformed count.
        assert_eq!(
            super::parse_trace_limit(&argv(&["server", "trace", "bundle", "--screenshot"]), 200).ok(),
            Some(200)
        );
        // ⛔ Garbage must be REFUSED, never defaulted: a wrong count that looks
        // like an answer is worse than an error.
        assert!(
            super::parse_trace_limit(&argv(&["server", "trace", "tail", "--limit", "lots"]), 200)
                .is_err(),
            "an unparseable --limit must fail loudly rather than silently return the default"
        );
        assert!(
            super::parse_trace_limit(&argv(&["server", "trace", "tail", "--limit"]), 200).is_err(),
            "`--limit` with no value must fail rather than fall through"
        );

        // ⛔ And the poll interval must not swallow the limit's value:
        // `follow --limit 500` polled every 500 ms under the old positional read.
        assert_eq!(
            super::parse_trace_poll_ms(&argv(&["server", "trace", "follow", "--limit", "500"]), 500)
                .ok(),
            Some(500),
            "the default, NOT the limit's value read as a poll interval"
        );
        assert_eq!(
            super::parse_trace_poll_ms(&argv(&["server", "trace", "follow", "--limit", "50"]), 500)
                .ok(),
            Some(500),
            "with a limit of 50 the poll interval must stay 500ms, not become 50ms"
        );
        assert_eq!(
            super::parse_trace_poll_ms(&argv(&["server", "trace", "follow", "100", "250"]), 500)
                .ok(),
            Some(250),
            "the positional pair still works when neither argument is a flag"
        );
    }

    /// ⛔ THE PROBE MUST CARRY THE PROPERTIES IT IS PROBING.
    ///
    /// A probe that asks for a bare scope answers "can systemd make a scope",
    /// which is not the question. The question is whether it will accept THESE
    /// properties, so a refusal of `MemoryHigh`/`MemorySwapMax` — the one
    /// failure that would otherwise stop the GUI starting — has to be reachable
    /// by the probe.
    #[test]
    fn the_scope_probe_asks_for_the_same_bound_as_the_real_scope() {
        let policy = super::memory_scope_policy(Some(16 * 1024 * 1024));
        let probe = super::memory_scope_preflight_args(policy, "yggterm-gui-preflight-1");
        let real = super::memory_scope_command_args(
            policy,
            "yggterm-gui-1",
            std::path::Path::new("/usr/bin/yggterm"),
            &[],
        );

        for property in [
            format!("MemoryHigh={}M", policy.high_mb),
            format!("MemorySwapMax={}M", policy.swap_max_mb),
        ] {
            assert!(
                probe.contains(&property),
                "the probe must ask for {property}, or it cannot discover a \
                 refusal of it"
            );
            assert!(real.contains(&property), "the real scope must ask for {property}");
        }

        assert!(
            probe.contains(&String::from("/bin/true")),
            "the probe must run something that exits immediately — it is testing \
             the scope, not starting the app"
        );
        assert!(
            !probe.iter().any(|arg| arg.contains("yggterm")
                && arg.starts_with('/')),
            "the probe must never launch the GUI binary: a probe that starts a \
             second GUI is worse than no probe"
        );
        assert!(
            probe.contains(&String::from("--collect")),
            "a failed probe must not leave a unit behind to collide with the \
             launch that immediately follows it"
        );
        assert_ne!(
            probe.iter().find(|arg| arg.starts_with("--unit=")),
            real.iter().find(|arg| arg.starts_with("--unit=")),
            "the probe and the real scope must not share a unit name, or the \
             probe's own cleanup races the launch"
        );
    }

    /// A BROWSER CACHES. `document-viewer` is WebKit's own name for "disable the
    /// cache completely" — the default this process shipped, which meant every
    /// navigation and every reload in ychrome refetched every byte. The knob
    /// stays overridable; what is locked is that the DEFAULT is a caching model,
    /// and that the memory policy which bounds it is still set alongside.
    #[test]
    fn the_default_cache_model_is_a_browsers_and_the_memory_bound_still_applies() {
        let source = include_str!("main.rs");
        let product = source
            .split("mod tests {")
            .next()
            .expect("main.rs has a product half above its tests");
        let at = product
            .find("ENV_YGGTERM_WEBKIT_CACHE_MODEL, ")
            .expect("the cache-model default moved — move this lock with it");
        let decision = &product[at..at + 80];
        assert!(
            decision.contains("\"web-browser\""),
            "the default must be a caching model; document-viewer disables the \
             cache outright:\n{decision}"
        );
        // The bound: caching more is only safe because the memory policy is
        // still applied — and applied FROM THE DERIVATION, not hardcoded, so the
        // audio-headroom lock above actually governs what ships.
        let configure = product
            .split("fn configure_linux_webkit_memory_policy() {")
            .nth(1)
            .expect("the memory policy function moved — move this lock with it");
        let body = configure
            .split("\n}")
            .next()
            .expect("the function has a body");
        assert!(
            body.contains("let policy = webkit_memory_policy(read_mem_total_kb());"),
            "the policy must be DERIVED from the machine:\n{body}"
        );
        for (env_name, from) in [
            ("ENV_YGGTERM_WEBKIT_MEMORY_LIMIT_MB", "policy.limit_mb"),
            (
                "ENV_YGGTERM_WEBKIT_MEMORY_CONSERVATIVE_THRESHOLD",
                "policy.conservative",
            ),
            ("ENV_YGGTERM_WEBKIT_MEMORY_STRICT_THRESHOLD", "policy.strict"),
        ] {
            assert!(
                body.contains(env_name) && body.contains(from),
                "{env_name} must be set from {from}; a hardcoded number here \
                 escapes the headroom lock entirely"
            );
        }
    }
    #[cfg(unix)]
    use super::superseded_client_termination_signal;
    use super::{
        BuiltinCliCommand, FILE_DESCRIPTOR_SOFT_LIMIT_TARGET, LinuxWindowProfileInput,
        SignalClientScope, app_control_launch_state_timeout_ms,
        app_control_state_settled_for_launch, classify_builtin_cli_command,
        client_process_runs_a_deleted_binary, compatible_signal_client_count,
        linux_window_profile_from_input,
        main_should_retire_superseded_clients_before_shell, raised_file_descriptor_soft_limit,
        record_matches_executable, server_app_subcommand_owns_its_help,
        should_handoff_to_preferred_executable, should_retire_superseded_client,
        signal_client_instances_dir, signal_client_scope_matches,
        signal_parse_process_start_ticks_from_stat, signal_process_start_ticks,
        signal_shutdown_policy_allows_daemon_shutdown, superseded_client_close_command,
        superseded_client_retirement_strategy_label, under_glass_default_armed,
    };

    // §12 one-owner rule (rewritten for the §12.2 inversion, 2026-07-31): the
    // audit's walk lives in yggterm-shell (KEYTIP_INTERACTABLE_WALK_JS, the
    // same function the ALT overlay's derive pass runs) and this CLI only
    // ASKS. The predecessor test locked the CLI's own walk ordering — that
    // walk is gone, and this lock refuses its return: a `querySelectorAll` in
    // the keytips arm would be a second definition of "visible interactable",
    // which is exactly the drift the inversion killed. The new-model ordering
    // lock (declared credit before the per-element exempt test, no closest())
    // lives beside the walk in yggterm-shell.
    /// The ONE owner of the `server app` verb surface, read as source so the
    /// locks below can scan the arms themselves.
    const APP_CONTROL_CLI_SOURCE: &str =
        include_str!("../../../crates/yggterm-server/src/app_control_cli.rs");

    /// ⛔⛔ NEITHER BINARY MAY DISPATCH `server app` ITSELF.
    ///
    /// This is the structural form of every "both binaries route X" lock
    /// around it, and it exists because those locks were necessary at all:
    /// the whole dispatch was a `match` copied into both binaries, and the
    /// copies had drifted by six top-level verbs before anyone noticed —
    /// `audio` and `theme` reachable only from the GUI binary, `chrome`,
    /// `row-set`, `row-expanded` and `split` only from the headless one, which
    /// is the binary every agent skill says to drive.
    ///
    /// The tell of a local dispatcher is its own failure message, so that is
    /// what is banned: exactly one file in the tree may say
    /// `unsupported app control command`, and it is the owner.
    /// ⛔ `server daemons` IS A HOST FACT, SO BOTH BINARIES MUST ANSWER IT.
    ///
    /// It answered on the headless CLI only, and the census has no GUI in it —
    /// which binary a verb reaches was decided by which file it was typed
    /// into. Same accident as `server app audio`, on the `server` surface
    /// rather than the `server app` one.
    ///
    /// ⚠ This is a per-verb lock, deliberately, and NOT the structural ban
    /// that `server app` gets. The `server` surface is still dispatched in
    /// both binaries and its divergences are NOT uniformly accidental — some
    /// verbs really are headless-only. Banning a second dispatcher there would
    /// be wrong until each verb has been triaged; the triage is in
    /// `docs/pending-bugs.md`.
    /// ⛔ THE FOUR DAEMON-SOCKET VERBS ANSWER FROM BOTH BINARIES.
    ///
    /// `write-lock`, `order`, `ledger` and `reorder` answered on the GUI binary
    /// only, and every line of each talks to the daemon over the local socket —
    /// there is no window in any of them. This is a PER-VERB lock, like the
    /// census one below and unlike `server app`'s structural ban, because the
    /// `server` surface genuinely does contain headless-only verbs.
    #[test]
    fn both_binaries_answer_the_daemon_socket_verbs() {
        for (binary, source) in [
            ("yggterm", include_str!("main.rs")),
            ("yggterm-headless", include_str!("bin/yggterm-headless.rs")),
        ] {
            // ⭐ All nine, as of 2026-08-14. The last three were the FORK this
            // surface was said to have and reading their bodies refuted it;
            // `connect` followed once its cluster was measured rather than
            // counted. `server` has no measured fork left at all.
            for verb in [
                "write_lock",
                "order",
                "ledger",
                "reorder",
                "gate_screen",
                "relay_boundary",
                "wpe",
                "connect",
            ] {
                assert!(
                    source.contains(&format!("server_cli::run_server_{verb}_cli(")),
                    "{binary} must route `server {verb}` to its one owner"
                );
            }
            // ⛔ And the bodies must be GONE, not merely also-dispatched. A
            // binary that kept its inline copy beside the call would pass the
            // assertion above while the two copies drifted, which is the exact
            // failure this whole entry is about. `run_server_wpe` was the local
            // one; the other two were inline blocks whose distinctive strings
            // are checked instead.
            let product = source
                .split("mod tests {")
                .next()
                .expect("the binary has a product half above its tests");
            for needle in [
                "fn run_server_wpe(",
                "no sessions owned by this daemon match",
                "declare_relay_boundary(",
                "fn run_server_connect(",
                "enum ConnectPlacement",
            ] {
                assert!(
                    !product.contains(needle),
                    "{binary} kept its own copy of a moved `server` verb ({needle})"
                );
            }
            // ⛔ And the helpers those verbs need are shared too. Both binaries
            // carried byte-identical private copies; adding a third in the
            // shared crate while leaving those in place would have been worse
            // than the duplication it was fixing.
            //
            // ⚠ PRODUCT HALF ONLY. A ban that greps the whole file matches the
            // banned string in THIS assertion and fails the file that obeys it
            // — the same prose-vs-code trap the payload lock above records.
            let product = source
                .split("mod tests {")
                .next()
                .expect("the binary has a product half above its tests");
            assert!(
                !product.contains("fn cli_server_endpoint("),
                "{binary} grew its own `cli_server_endpoint` back"
            );
            assert!(
                !product.contains("fn ensure_local_server_ready_for_cli("),
                "{binary} grew its own `ensure_local_server_ready_for_cli` back"
            );
        }
    }

    #[test]
    fn both_binaries_answer_the_daemon_census() {
        for (binary, source) in [
            ("yggterm", include_str!("main.rs")),
            ("yggterm-headless", include_str!("bin/yggterm-headless.rs")),
        ] {
            assert!(
                source.contains("run_server_daemons_census("),
                "{binary} must route `server daemons` to its one owner — the \
                 census is a host fact and answering it on one binary only is \
                 how `yggterm server daemons` came to report the verb unknown"
            );
        }
    }

    #[test]
    fn neither_binary_dispatches_server_app_itself() {
        for (binary, source) in [
            ("yggterm", include_str!("main.rs")),
            ("yggterm-headless", include_str!("bin/yggterm-headless.rs")),
        ] {
            assert!(
                source.contains("app_control_cli::run_app_control_cli("),
                "{binary} must route the whole `server app` surface to its one \
                 owner (`yggterm_server::app_control_cli`)"
            );
            let product = source
                .split("mod tests {")
                .next()
                .expect("the binary has a product half above its tests");
            assert!(
                !product.contains("unsupported app control command"),
                "{binary} carries a `server app` dispatcher of its own. Two copies \
                 diverge on the first new verb and the failure is silent — the verb \
                 answers \"unsupported app control command\" from the other binary \
                 while --build-commit matches the deploy and the arm is visibly in \
                 the source. Add the verb to the owner instead."
            );
        }
        // The owner really does dispatch, so the ban above cannot pass by the
        // surface having quietly disappeared.
        assert!(
            APP_CONTROL_CLI_SOURCE.contains("unsupported app control command"),
            "the one owner no longer dispatches anything — this lock just went blind"
        );
    }

    #[test]
    fn the_keytips_cli_carries_no_walk_of_its_own() {
        // Scans the ONE owner. This used to read `main.rs`, and the comment
        // below records that the headless twin shipped without the arm — the
        // duplication that made a "both binaries" lock necessary is gone, so
        // the lock now watches the single dispatcher instead of two copies.
        let source = APP_CONTROL_CLI_SOURCE;
        let arm_start = source
            .find("\"keytips\" => {")
            .expect("the keytips CLI arm exists");
        // The arm ends where the next subcommand arm begins.
        // Bounded at THIS arm's own close brace, never at whichever arm
        // happens to follow it — the lesson the media lock below already
        // records, and the ordering did change when the dispatchers were
        // collapsed onto one owner.
        let rest = &source[arm_start..];
        let arm = &rest[..rest
            .find("\n        }")
            .map(|offset| offset + "\n        }".len())
            .expect("the keytips arm has no close brace")];
        assert!(
            !arm.contains("querySelectorAll"),
            "the keytips CLI must ask the GUI's one walk, never carry its own:\n{arm}"
        );
        for verb in [
            "run_app_control_keytips_audit",
            "run_app_control_keytips_overlay(true",
            "run_app_control_keytips_overlay(false",
        ] {
            assert!(
                arm.contains(verb),
                "the keytips arm must route `{verb}` through the app-control verbs:\n{arm}"
            );
        }

        // ⭐ THE "BOTH BINARIES, VERB-FOR-VERB" HALF THAT STOOD HERE IS GONE,
        // and its disappearance is the point. It existed because the headless
        // twin shipped WITHOUT this arm the first time, answering "unsupported
        // app control command" on a fresh daemon while every lock here was
        // green. A per-verb parity assertion is the right answer to two
        // dispatchers; it is the wrong answer to one. There is now a single
        // dispatcher, so parity is not checked verb-by-verb — it is structural,
        // and `neither_binary_dispatches_server_app_itself` is what holds it.
        // ⛔ Do not re-add a per-verb twin check here: a lock that scans a copy
        // is a lock that expects a copy to exist.
    }

    /// ★ THE CAPTURE-ANSWER PARITY LOCK. `server app media answer` is how an
    /// operator releases a page blocked on a camera prompt, and the binary
    /// agents are told to drive is `yggterm-headless` — a verb that exists on
    /// the GUI binary only would read to them as "this build cannot answer it",
    /// which is indistinguishable from the hang the verb exists to end.
    ///
    /// Also pinned: the CLI never builds the command itself. One owner
    /// (`yggterm_server::run_app_control_media_answer`), or the two binaries
    /// grow two spellings of "allow".
    #[test]
    fn both_binaries_route_the_capture_answer_to_its_one_owner() {
        for (binary, source) in [("the one server-app dispatcher", APP_CONTROL_CLI_SOURCE)] {
            let start = source.find("\"media\" => {").unwrap_or_else(|| {
                panic!(
                    "{binary} has no `server app media` arm — a page blocked on a \
                     camera prompt cannot be answered from this binary at all"
                )
            });
            // Bounded at THIS arm's own close brace, never at a byte count and
            // never at the next arm. A byte count can land mid-codepoint (both
            // files are full of `⛔`/`★`) and panic; "up to the next arm" swept
            // in the doc comment that introduces it, which names
            // `AppControlCommand::InvokeCommand` and failed the second
            // assertion below for a line that is not even code. `find` returns
            // a char boundary.
            let rest = &source[start..];
            let end = rest
                .find("\n            }")
                .map(|offset| offset + "\n            }".len())
                .unwrap_or_else(|| panic!("{binary}'s media arm has no close brace"));
            let arm = &rest[..end];
            assert!(
                arm.contains("crate::run_app_control_media_answer("),
                "{binary}'s media arm does not route the ONE owner; a dispatch of \
                 its own here is the split-dispatch trap"
            );
            assert!(
                !arm.contains("AppControlCommand::"),
                "{binary}'s media arm builds the app-control command itself — the \
                 wire shape has one owner and it is not a CLI arm"
            );
        }
    }

    /// The `server app web` arm line of one binary's `server app` dispatcher.
    ///
    /// A SCANNER, so it must never quietly match nothing: an absent arm is the
    /// exact bug being locked (`yggterm-headless server app web eval …` used to
    /// answer `unsupported app control command: web`), and a scanner that
    /// shrugged at it would pass green while the plane was missing.
    fn web_arm_line(binary: &str, source: &str) -> String {
        let start = source.find("\"web\" => ").unwrap_or_else(|| {
            panic!(
                "{binary} has no `server app web` arm at all — the whole verb plane \
                 (eval/read/await/do/fill/wait/ensure/frames/…) answers \
                 \"unsupported app control command: web\" on it"
            )
        });
        let rest = &source[start..];
        rest[..rest.find('\n').unwrap_or(rest.len())]
            .trim_end()
            .to_string()
    }

    /// ★ THE WEB-PLANE PARITY LOCK — the twin of the keytips one above, written
    /// because the same trap caught the same plane a second time and worse: the
    /// ENTIRE `server app web` verb set (eval, read, await, do, fill, wait,
    /// ensure, frames, batch, lease, totp, fill-vault, fill-card, …) lived in
    /// `main.rs` only. An agent following the docs types `yggterm-headless
    /// server app web eval …` — the headless binary is the one agents are told
    /// to drive — and got `unsupported app control command: web`, which reads
    /// as "this build does not have it".
    ///
    /// The fix is ONE owner (`yggterm_server::run_app_control_web_cli`), not a
    /// copy: 415 duplicated lines would diverge on the first new verb. So this
    /// lock does not check that both binaries route each verb — it checks that
    /// NEITHER binary knows what a verb is. The arm line must be the delegation
    /// EXACTLY; anything else (an inlined dispatch block, a partial copy, a
    /// second parser) fails here. (The arm text is deliberately not spelled out
    /// in prose anywhere in this file — the scanner takes the FIRST match, and
    /// a doc comment that quoted it could become that match.)
    ///
    /// The verb list is DERIVED from the owner (`web_action_names()`, which the
    /// owner's own drift lock pins to its dispatcher's match arms), so adding a
    /// verb over there cannot leave this lock stale: it is routed on both
    /// binaries by construction, and it must appear in the help both binaries
    /// render from the owner's block.
    #[test]
    fn both_binaries_route_the_web_plane_to_its_one_owner() {
        const DELEGATION: &str = "\"web\" => crate::run_app_control_web_cli(&args, timeout_ms),";

        // DERIVED, never hand-listed. A floor, because a verb list that went
        // empty would satisfy every loop below while proving nothing.
        let verbs = yggterm_server::web_action_names();
        assert!(
            verbs.len() >= 15,
            "the owner reports only {} web verbs — it went blind; fix the owner's \
             WEB_ACTIONS/drift lock rather than lowering this floor",
            verbs.len()
        );
        for (binary, source) in [("the one server-app dispatcher", APP_CONTROL_CLI_SOURCE)] {
            assert_eq!(
                web_arm_line(binary, source),
                DELEGATION,
                "{binary} must route `server app web` to the ONE owner verbatim. \
                 A dispatch of its own here is the split-dispatch trap: two copies \
                 diverge on the first new verb, which is how this plane came to \
                 exist on one binary only."
            );
            let _ = binary;
            assert!(
                source.contains("web_usage = crate::web_usage_block(binary)"),
                "{binary} must render the OWNER's usage block in `server app --help`, \
                 under its OWN name — an agent that reads --help and does not see a \
                 verb concludes the build lacks it, which is the misdiagnosis this \
                 whole plane already caused once"
            );
            // The usage this binary actually prints, verb by verb, against the
            // list the owner routes.
            let usage = yggterm_server::web_usage_block("yggterm");
            for verb in &verbs {
                assert!(
                    usage.contains(verb),
                    "`{verb}` is routed by the owner and named nowhere in the usage \
                     block {binary} prints"
                );
            }
        }
    }

    // ⭐ UNDER-GLASS IS THE STANDARD PRESENTATION PATH (user directive
    // 2026-07-31). This test asserted the opposite twice before, in both
    // directions, so read the whole history before flipping it a third time.
    //
    // Round 1 it was default-on for any host not on software GL — so the moment
    // the GL probe found a working GPU, Phase F armed in production for the
    // first time and the user's entire window became an agent's background
    // page. Round 2 it became strictly opt-in, which stopped that incident and
    // introduced a quieter one: a web surface launched without the flag does
    // not sit FLUSH in the viewport, and the user hit that after a restart —
    // *"I could not understand why our software needs an extra flag to be
    // correct."*
    //
    // Round 3 (here) restores the default, because the two rounds were arguing
    // about different things. Stacking was never the defect; DEGRADED PAINT
    // was. The incident needed a shell too starved to paint, and it is now
    // guarded structurally (unrevealed surfaces stay unmapped — pixel-proven:
    // a never-revealed second surface paints ZERO pixels) rather than by
    // keeping the correct path switched off. The software-GL demotion below is
    // the load-bearing safety and is UNCHANGED, so this default cannot
    // resurrect the DMABuf crash-loop.
    //
    // If you are here because under-glass regressed: the fix is an escape hatch
    // (`=0`, or `YGGTERM_WEB_SURFACE_LEGACY_STACK=1`) plus a root-cause on why
    // the shell stopped painting. Do NOT weaken this lock back to opt-in
    // without the user saying so — they asked for the flag to stop existing.
    #[test]
    fn under_glass_arms_by_default_on_hardware_gl() {
        // The correct presentation path does not wait to be asked for.
        assert!(
            under_glass_default_armed(None, None, false),
            "under-glass is the standard path on a hardware-GL host — a web \
             surface that needs an extra flag to sit flush is wrong by default"
        );
        assert!(under_glass_default_armed(Some("1"), None, false));
        // The explicit opt-out is the supported answer if it regresses.
        assert!(!under_glass_default_armed(Some("0"), None, false));
        // The legacy force is the escape hatch of last resort and beats both
        // the default and an explicit opt-in.
        assert!(!under_glass_default_armed(None, Some("1"), false));
        assert!(!under_glass_default_armed(Some("1"), Some("1"), false));
        // Legacy explicitly OFF is not an opt-out of the default.
        assert!(under_glass_default_armed(None, Some("0"), false));
    }

    // A software-GL host must NOT arm by default: under glass requires the
    // DMABuf renderer and DMABuf SIGSEGVs where there is no working hardware GL
    // — live-caught as a GUI crash-loop (blank viewport + dropped session every
    // few minutes, systemd relaunching each time). An explicit =1 still wins so
    // a host with known-good software GL can opt back in; the legacy force still
    // beats everything.
    #[test]
    fn under_glass_demotes_on_software_gl_hosts() {
        assert!(!under_glass_default_armed(None, None, true));
        assert!(!under_glass_default_armed(Some("0"), None, true));
        assert!(!under_glass_default_armed(None, Some("1"), true));
        // Explicit opt-in overrides the software-GL demotion...
        assert!(under_glass_default_armed(Some("1"), None, true));
        // ...but the legacy force still wins over that opt-in.
        assert!(!under_glass_default_armed(Some("1"), Some("1"), true));
    }

    // The arming decision OWNS the presentation path. A GUI relaunched by a
    // running GUI inherits its env, so an SHM force set during an unarmed run
    // used to ride into an armed launch and silently demote under-glass to
    // legacy stacking (live-caught on the KDE host 2026-07-20). Armed must
    // therefore CLEAR the force, not merely decline to set it.
    #[cfg(target_os = "linux")]
    #[test]
    fn arming_owns_the_shm_presentation_force() {
        // Software GL, so arming is the whole story — the historical matrix.
        assert_eq!(shm_force_for_arming(true, false, true), ShmForce::Clear);
        assert_eq!(shm_force_for_arming(true, false, false), ShmForce::Clear);
        assert_eq!(shm_force_for_arming(false, false, false), ShmForce::Apply);
        assert_eq!(shm_force_for_arming(false, false, true), ShmForce::Keep);
        // Hardware GL removes SHM from the table entirely: it exists ONLY as the
        // workaround for a broken hardware EGL/DMABuf path. Without this, an explicit
        // YGGTERM_WEB_SURFACE_UNDER_GLASS=0 on a probed-hardware host would produce
        // hardware GL + SHM, measured at 15.82 s against software GL's 15.33 s —
        // paying for the GPU and getting nothing.
        assert_eq!(shm_force_for_arming(false, true, false), ShmForce::Clear);
        assert_eq!(shm_force_for_arming(false, true, true), ShmForce::Clear);
    }

    /// The GL decision's precedence, spelled out. Force beats allow beats observation,
    /// and an inconclusive probe stays on software: "we could not tell" must never be
    /// promoted to "probably fine", because that promotion — from one EACCES on one
    /// DRM node — is the whole bug.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_gl_policy_prefers_a_force_then_an_opt_in_then_what_the_host_said() {
        use yggterm_core::gl_probe::GlClass;
        let policy = |compositing_disabled_env, force_software_gl, enable_compositing, probe| {
            linux_webkit_gl_policy_from_input(LinuxWebkitGlPolicyInput {
                compositing_disabled_env,
                force_software_gl,
                enable_compositing,
                probe,
            })
        };
        // Compositing force-disabled outranks everything: there is no GPU path left.
        assert_eq!(
            policy(true, false, true, GlClass::Hardware).reason,
            "webkit_compositing_disabled_by_env"
        );
        // FORCE beats ALLOW, even against a hardware probe.
        assert_eq!(
            policy(false, true, true, GlClass::Hardware).reason,
            "software_gl_forced"
        );
        assert!(!policy(false, true, false, GlClass::Hardware).hardware_gl);
        // The historical opt-in still wins over a probe that says software — a host
        // whose software GL is known good keeps its escape hatch.
        assert_eq!(
            policy(false, false, true, GlClass::Software).reason,
            "hardware_gl_forced"
        );
        // Otherwise the host decides.
        assert_eq!(
            policy(false, false, false, GlClass::Hardware).reason,
            "hardware_gl_probed"
        );
        assert_eq!(
            policy(false, false, false, GlClass::Software).reason,
            "software_gl_probed"
        );
        let inconclusive = policy(false, false, false, GlClass::Unknown);
        assert!(!inconclusive.hardware_gl);
        assert_eq!(inconclusive.reason, "software_gl_probe_inconclusive");
    }

    /// ⚠ THE MATRIX LOCK: GL, arming and the presentation path are ONE decision.
    ///
    /// Measured on the live host, same page and duration: hardware GL + DMABuf 6.85 s;
    /// software GL + SHM 15.33 s; **hardware GL + SHM 15.82 s** (no better than
    /// software); **software GL + DMABuf 34.14 s** — the worst of the four, llvmpipe
    /// emulating the compositor. So the only two legal cells are the diagonal, and the
    /// assertion below says exactly that over the full cross-product of every input.
    ///
    /// It fails if anyone clears the DMABuf force without flipping GL, and it fails if
    /// anyone turns on hardware GL while leaving SHM in place.
    ///
    /// The one legal departure from the diagonal is an EXPLICIT
    /// `YGGTERM_WEB_SURFACE_UNDER_GLASS=1` on a software host. That is a user asking
    /// for under-glass by name, and under glass without DMABuf is not slow, it is
    /// BROKEN — the glass punches straight through to the window background. A user
    /// who names it accepts the 34 s cell; nothing may wander into it by default.
    #[cfg(target_os = "linux")]
    #[test]
    fn gl_arming_and_presentation_are_one_decision_in_every_cell() {
        use yggterm_core::gl_probe::GlClass;
        let mut cells = 0;
        for probe in [GlClass::Hardware, GlClass::Software, GlClass::Unknown] {
            for compositing_disabled_env in [false, true] {
                for force_software_gl in [false, true] {
                    for enable_compositing in [false, true] {
                        for under_glass_var in [None, Some("0"), Some("1")] {
                            for already_forced in [false, true] {
                                // ⚠ THE AXIS THAT WAS MISSING. The old test computed
                                // `software_gl_force_for_policy(p.hardware_gl)` and
                                // asserted it equalled `Clear` iff `p.hardware_gl` —
                                // a restatement of a two-line function, and blind to
                                // the applier where the real bug lived. What decides
                                // the outcome is the INHERITED environment, so it is
                                // an input here.
                                for inherited_software_force in [false, true] {
                                    for stray_nvidia in [false, true] {
                                        for egl_filter_present in [false, true] {
                                            let policy = linux_webkit_gl_policy_from_input(
                                                LinuxWebkitGlPolicyInput {
                                                    compositing_disabled_env,
                                                    force_software_gl,
                                                    enable_compositing,
                                                    probe,
                                                },
                                            );
                                            let plan = linux_webkit_gl_env_plan(
                                                policy,
                                                compositing_disabled_env,
                                                stray_nvidia,
                                                LinuxWebkitGlEnvInherited {
                                                    libgl_always_software_present:
                                                        inherited_software_force,
                                                    gallium_driver_present:
                                                        inherited_software_force,
                                                    webkit_disable_dmabuf_renderer_present:
                                                        already_forced,
                                                    web_surface_under_glass: under_glass_var,
                                                    web_surface_legacy_stack: None,
                                                    egl_vendor_library_filenames_present:
                                                        egl_filter_present,
                                                },
                                            );
                                            let armed = under_glass_default_armed(
                                                under_glass_var,
                                                None,
                                                !policy.hardware_gl,
                                            );
                                            let explicit_under_glass = under_glass_var == Some("1");
                                            let context = format!(
                                                "hardware_gl={} probe={probe:?} \
                                         disabled={compositing_disabled_env} \
                                         force_sw={force_software_gl} \
                                         enable={enable_compositing} \
                                         glass={under_glass_var:?} forced={already_forced} \
                                         inherited_sw={inherited_software_force}",
                                                policy.hardware_gl
                                            );
                                            // The decision is always observable, escape hatch
                                            // or not — otherwise the one instrument is missing
                                            // exactly when someone asks why the GPU is off.
                                            assert_eq!(
                                                plan.webkit_gl_policy,
                                                GlEnvAction::Set(policy.reason),
                                                "the policy must publish itself ({context})"
                                            );
                                            assert_eq!(
                                                plan.web_surface_under_glass,
                                                GlEnvAction::Set(if armed { "1" } else { "0" }),
                                                "arming must publish itself ({context})"
                                            );
                                            // ⚠⚠ THE LOCK THE PREVIOUS ONE ONLY LOOKED LIKE.
                                            // Live-caught 2026-07-25: declining to SET the
                                            // software-GL pair is not the same as owning it. A
                                            // GUI relaunched by a running GUI inherits
                                            // LIBGL_ALWAYS_SOFTWARE=1 from its predecessor, so
                                            // on a probed-hardware host the policy said
                                            // hardware while WebKit stayed on llvmpipe. This
                                            // fails if `Clear` is ever downgraded to
                                            // "set if unset".
                                            let expected_software_pair = if compositing_disabled_env
                                            {
                                                // Compositing force-disabled: there is no GPU
                                                // path left to choose, so we touch neither.
                                                (GlEnvAction::Keep, GlEnvAction::Keep)
                                            } else if policy.hardware_gl {
                                                (GlEnvAction::Remove, GlEnvAction::Remove)
                                            } else if inherited_software_force {
                                                (GlEnvAction::Keep, GlEnvAction::Keep)
                                            } else {
                                                (
                                                    GlEnvAction::Set("1"),
                                                    GlEnvAction::Set("llvmpipe"),
                                                )
                                            };
                                            assert_eq!(
                                                (plan.libgl_always_software, plan.gallium_driver),
                                                expected_software_pair,
                                                "hardware GL must CLEAR an inherited software force, \
                                         not decline to set one ({context})"
                                            );
                                            let expected_shm = if compositing_disabled_env {
                                                GlEnvAction::Keep
                                            } else if policy.hardware_gl || explicit_under_glass {
                                                GlEnvAction::Remove
                                            } else if already_forced {
                                                GlEnvAction::Keep
                                            } else {
                                                GlEnvAction::Set("1")
                                            };
                                            assert_eq!(
                                                plan.webkit_disable_dmabuf_renderer, expected_shm,
                                                "DMABuf is legal only with hardware GL or an \
                                         explicit under-glass request ({context})"
                                            );
                                            // The GLVND vendor guard: pin to Mesa exactly when
                                            // the hardware path is on, the NVIDIA ICD is stray
                                            // (installed, no device), and nobody set their own
                                            // filter. Everywhere else: hands off — the escape
                                            // hatch must keep restoring the old behaviour
                                            // whole, and a user filter always wins.
                                            let expected_egl_filter = if policy.hardware_gl
                                                && stray_nvidia
                                                && !egl_filter_present
                                            {
                                                GlEnvAction::Set(
                                                    yggterm_core::gl_probe::MESA_EGL_VENDOR_JSON,
                                                )
                                            } else {
                                                GlEnvAction::Keep
                                            };
                                            assert_eq!(
                                                plan.egl_vendor_library_filenames,
                                                expected_egl_filter,
                                                "the stray-NVIDIA vendor guard fires only on the \
                                         hardware path with no user filter ({context} \
                                         stray={stray_nvidia} filter={egl_filter_present})"
                                            );
                                            if under_glass_var.is_none()
                                                && !compositing_disabled_env
                                            {
                                                // No user opinion at all. DMABuf still
                                                // follows hardware GL — that is the
                                                // measured half of the win and it must
                                                // not depend on Phase F.
                                                assert_eq!(
                                                    policy.hardware_gl,
                                                    plan.webkit_disable_dmabuf_renderer
                                                        == GlEnvAction::Remove
                                                );
                                                // ...and since 5b0280a arming FOLLOWS it.
                                                // This assertion used to read `!armed`
                                                // ("a working GPU is not consent"), which
                                                // was the contract before the user settled
                                                // the opposite: under-glass is the correct
                                                // presentation path, not an experiment, and
                                                // *"our software needs an extra flag to be
                                                // correct"* is the bug. The lock is rewritten
                                                // to the new model rather than deleted, and
                                                // it is STRICTER than what it replaces —
                                                // it pins both directions, so neither the
                                                // hardware default nor the software demotion
                                                // can regress silently. Software GL still
                                                // refuses: DMABuf cannot composite over SHM
                                                // and SIGSEGVs on a host with no working
                                                // hardware GL.
                                                assert_eq!(
                                                    armed,
                                                    policy.hardware_gl,
                                                    "with no user opinion, under-glass arms on \
                                             a hardware-GL host and demotes on software GL \
                                             ({context})"
                                                );
                                            }
                                            cells += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // A cross-product that silently collapsed to nothing would pass vacuously.
        assert_eq!(cells, 3 * 2 * 2 * 2 * 3 * 2 * 2 * 2 * 2);
    }

    /// The applier may not know a key the plan does not, and vice versa.
    ///
    /// `WEBKIT_GL_ENVIRONMENT_KEYS` is what every reader publishes as "the GL path";
    /// `linux_webkit_gl_env_plan_entries` is what the process actually writes. If those
    /// two lists drift, a window is on a GL path nobody can name.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_plan_writes_exactly_the_keys_the_gl_path_is_published_as() {
        let plan = linux_webkit_gl_env_plan(
            linux_webkit_gl_policy_from_input(LinuxWebkitGlPolicyInput {
                compositing_disabled_env: false,
                force_software_gl: false,
                enable_compositing: false,
                probe: yggterm_core::gl_probe::GlClass::Hardware,
            }),
            false,
            false,
            LinuxWebkitGlEnvInherited::default(),
        );
        let written: Vec<&str> = linux_webkit_gl_env_plan_entries(&plan)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        let mut published = yggterm_core::gl_probe::WEBKIT_GL_ENVIRONMENT_KEYS.to_vec();
        published.sort_unstable();
        let mut written_sorted = written.clone();
        written_sorted.sort_unstable();
        assert_eq!(
            written_sorted, published,
            "the keys the GL decision writes and the keys it is published under must \
             be the same set"
        );
        assert_eq!(
            written.len(),
            std::collections::BTreeSet::from_iter(written.iter()).len(),
            "no key may be written twice"
        );
    }
    #[cfg(target_os = "linux")]
    use super::{
        GlEnvAction, LINUX_GUI_ENTRY_ENV_SOURCE_KEY, LinuxWebkitGlEnvInherited,
        LinuxWebkitGlPolicyInput, ShmForce, linux_choose_desktop_environment,
        linux_environ_bytes_to_map, linux_gui_entry_environment_overrides_from_desktop,
        linux_webkit_gl_env_plan, linux_webkit_gl_env_plan_entries,
        linux_webkit_gl_policy_from_input, shm_force_for_arming,
    };
    #[cfg(target_os = "linux")]
    use std::collections::BTreeMap;
    use std::fs;
    use yggterm_server::{ClientInstanceRecord, ServerEndpoint};

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
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string(), "app".to_string()]),
            Some(BuiltinCliCommand::ServerAppHelp)
        );
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "app".to_string(),
                "launch".to_string(),
                "--help".to_string()
            ]),
            Some(BuiltinCliCommand::ServerAppHelp)
        );
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
    }

    /// A `server app` subcommand that prints its OWN help must not have that
    /// help swallowed by the generic interception, or its help printer is code
    /// the user can never reach. `audio` is the case that found this.
    #[test]
    fn a_server_app_subcommand_that_owns_its_help_is_not_intercepted() {
        for spelling in ["--help", "-h", "help"] {
            assert_eq!(
                classify_builtin_cli_command(&[
                    "server".to_string(),
                    "app".to_string(),
                    "audio".to_string(),
                    spelling.to_string(),
                ]),
                None,
                "`server app audio {spelling}` must fall through to the audio \
                 dispatcher, which owns the audio help",
            );
        }
        // Only the subcommands that actually have their own help are exempt —
        // everything else still gets the app-level help, as before.
        assert!(server_app_subcommand_owns_its_help("audio"));
        assert!(!server_app_subcommand_owns_its_help("screenshot"));
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "app".to_string(),
                "screenshot".to_string(),
                "--help".to_string(),
            ]),
            Some(BuiltinCliCommand::ServerAppHelp),
        );
        // Bare `server app` is still the app-level help, and `server app audio`
        // with no subcommand still falls through to the audio help.
        assert_eq!(
            classify_builtin_cli_command(&["server".to_string(), "app".to_string()]),
            Some(BuiltinCliCommand::ServerAppHelp),
        );
        assert_eq!(
            classify_builtin_cli_command(&[
                "server".to_string(),
                "app".to_string(),
                "audio".to_string(),
            ]),
            None,
        );
    }

    #[test]
    fn app_launch_wait_uses_dom_snapshot_sized_state_budget() {
        assert_eq!(app_control_launch_state_timeout_ms(100), 250);
        assert_eq!(app_control_launch_state_timeout_ms(1_500), 1_500);
        assert_eq!(app_control_launch_state_timeout_ms(8_000), 4_000);
    }

    #[test]
    fn app_launch_wait_rejects_blank_active_xterm_surface() {
        let payload = serde_json::json!({
            "data": {
                "shell": { "needs_initial_server_sync": false },
                "session_view_contract_violations": [],
                "active_session_path": "local://701cb151-58a8-4fe3-8194-451d8daa8192",
                "active_view_mode": "Terminal",
                "runtime_truth": {
                    "daemon_runtime_count": 1,
                    "active_runtime_present": true,
                    "live_row_count": 1
                },
                "active_terminal_surface": {
                    "rendered": false,
                    "problem": "active terminal host exists but xterm surface is empty"
                },
                "terminal_hosts": [{
                    "session_path": "local://701cb151-58a8-4fe3-8194-451d8daa8192",
                    "xterm_present": false,
                    "screen_present": false,
                    "viewport_present": false,
                    "rows_present": false,
                    "canvas_count": 0,
                    "child_count": 0
                }]
            }
        });
        assert!(!app_control_state_settled_for_launch(&payload));
    }

    #[test]
    fn app_launch_wait_accepts_mounted_active_xterm_surface() {
        let payload = serde_json::json!({
            "data": {
                "shell": { "needs_initial_server_sync": false },
                "session_view_contract_violations": [],
                "active_session_path": "local://701cb151-58a8-4fe3-8194-451d8daa8192",
                "active_view_mode": "Terminal",
                "runtime_truth": {
                    "daemon_runtime_count": 1,
                    "active_runtime_present": true,
                    "live_row_count": 1
                },
                "active_terminal_surface": {
                    "rendered": true,
                    "problem": null
                },
                "terminal_hosts": [{
                    "session_path": "local://701cb151-58a8-4fe3-8194-451d8daa8192",
                    "xterm_present": true,
                    "screen_present": true,
                    "viewport_present": true,
                    "rows_present": true,
                    "canvas_count": 0,
                    "child_count": 4
                }]
            }
        });
        assert!(app_control_state_settled_for_launch(&payload));
    }

    #[test]
    fn app_cli_help_exposes_settled_open_path_command() {
        let source = APP_CONTROL_CLI_SOURCE;
        assert!(source.contains("server app open <session-path>"));
        assert!(source.contains("\"open\" =>"));
        assert!(source.contains("run_app_control_open_path(session_path, view_mode, timeout_ms)"));
    }

    fn payload_argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    /// ⛔ THE LIE-OF-SUCCESS SHAPE. `server app dom-eval --client shadow
    /// '<script>'` read its script at `args[3]`, which is the token `--client`
    /// — so it evaluated the STRING "--client" in the GUI and printed a result
    /// for it, indistinguishable to the caller from their script having run.
    /// The targeting flags were never position-sensitive (they are scanned out
    /// of the whole argv), so it was the payload that had to catch up: both
    /// spellings are ONE command, same script and same target.
    #[test]
    fn a_server_app_payload_is_the_same_with_the_flags_on_either_side() {
        let script = "return document.title";
        let spellings = [
            payload_argv(&["server", "app", "dom-eval", "--client", "shadow", script]),
            payload_argv(&["server", "app", "dom-eval", script, "--client", "shadow"]),
            payload_argv(&[
                "server", "app", "dom-eval", "--pid", "4242", script, "--client", "shadow",
            ]),
            payload_argv(&[
                "server", "app", "dom-eval", "--client", "shadow", "--pid", "4242", script,
            ]),
        ];
        for args in &spellings {
            assert_eq!(
                yggterm_server::app_control_cli::app_control_payload_arg(args, 3, "script for server app dom-eval")
                    .expect("the script resolves wherever the flags sit"),
                script,
                "{args:?} must resolve the same script as its sibling spellings"
            );
            // The other half of "same command": the target the script rides
            // with. `app_control_client_flag`/`app_control_pid_flag` read the
            // whole argv under this same rule, so the two halves can no longer
            // disagree about which GUI worker a positional was typed for.
            assert_eq!(
                super::cli_flag_value(args, "--client"),
                Some("shadow"),
                "{args:?} names the same client whichever side the script sits"
            );
        }
        assert_eq!(super::cli_flag_value(&spellings[2], "--pid"), Some("4242"));
        assert_eq!(super::cli_flag_value(&spellings[3], "--pid"), Some("4242"));
        // The same reader, the same rule, for the other free-form payloads in
        // the family — `media answer` at index 4 and `command invoke`'s id.
        for (start, args, expected) in [
            (
                4,
                payload_argv(&["server", "app", "media", "answer", "--request", "7", "allow"]),
                "allow",
            ),
            (
                4,
                payload_argv(&["server", "app", "media", "answer", "allow", "--request", "7"]),
                "allow",
            ),
            (
                4,
                payload_argv(&["server", "app", "command", "invoke", "--pid", "42", "help.open"]),
                "help.open",
            ),
            (
                4,
                payload_argv(&["server", "app", "command", "invoke", "help.open", "--pid", "42"]),
                "help.open",
            ),
        ] {
            assert_eq!(
                yggterm_server::app_control_cli::app_control_payload_arg(&args, start, "value")
                    .expect("the payload resolves wherever the flags sit"),
                expected,
                "{args:?} must resolve the same payload as its sibling spelling"
            );
        }
    }

    /// Belt and braces: a value that looks like a flag is REFUSED, never acted
    /// on. A refusal names the problem; evaluating `--client` as a script is
    /// the defect this whole reader exists to close.
    #[test]
    fn a_server_app_payload_that_looks_like_a_flag_is_refused_not_acted_on() {
        let error =
            super::refuse_flag_shaped_payload("--client", "script for server app dom-eval")
                .expect_err("a flag is not a script");
        let message = format!("{error}");
        assert!(
            message.contains("--client") && message.contains("refusing"),
            "the refusal must name the token it refused: {message}"
        );
        // And through argv, with the script left out entirely: the arm must
        // still refuse rather than reach for whatever sits at the fixed index.
        let args = payload_argv(&["server", "app", "dom-eval", "--client", "shadow"]);
        let error = yggterm_server::app_control_cli::app_control_payload_arg(&args, 3, "script for server app dom-eval")
            .expect_err("a bare flag is not a script");
        let message = format!("{error}");
        assert!(
            message.contains("--client"),
            "the refusal must name the token it refused: {message}"
        );
        // A genuinely missing payload still says so plainly.
        let bare = payload_argv(&["server", "app", "dom-eval"]);
        assert!(
            format!(
                "{}",
                yggterm_server::app_control_cli::app_control_payload_arg(&bare, 3, "script for server app dom-eval")
                    .expect_err("no script at all")
            )
            .contains("missing script for server app dom-eval")
        );
    }

    /// The arms must ROUTE the shared reader. A lock on the helper alone stays
    /// green if a call site quietly goes back to `args.get(3)`, which is
    /// exactly how this bug shipped.
    #[test]
    fn every_free_form_app_payload_arm_reads_through_the_one_positional_reader() {
        let source = APP_CONTROL_CLI_SOURCE;
        let product = source
            .split("mod tests {")
            .next()
            .expect("main.rs has a product half above its tests");
        // ⚠ The close markers are INDENTATION-SENSITIVE and the arms moved one
        // level out when the two dispatchers were collapsed onto one owner. A
        // marker that no longer matches does not fail loudly — it runs on to
        // the NEXT arm's close and sweeps in code that is allowed to use a
        // fixed index (`start-action` reads `args.get(3)` legitimately), which
        // reads as this lock catching a bug it did not catch.
        for (marker, close, index) in [
            ("\"dom-eval\" => {", "\n        }", "3"),
            ("\"answer\" => {", "\n                }", "4"),
            ("\"invoke\" | \"run\" => {", "\n                }", "4"),
        ] {
            let rest = product
                .split(marker)
                .nth(1)
                .unwrap_or_else(|| panic!("the {marker} arm moved — move this lock with it"));
            let arm = &rest[..rest
                .find(close)
                .unwrap_or_else(|| panic!("the {marker} arm has no close brace"))];
            assert!(
                // Either spelling of the ONE reader — `cli_payload_arg`
                // directly, this crate's `app_control_payload_arg` adapter over
                // it, or the positional list `cli_positional_args` returns.
                // All three scan flags out of the argv first, which is the
                // property being locked; what is banned is a RAW index into
                // argv, asserted separately below.
                arm.contains("payload_arg(") || arm.contains("positional.get("),
                "{marker} does not read its payload through the one reader:\n{arm}"
            );
            // ⛔ SCAN CODE, NOT PROSE. These arms carry a comment that says
            // "NOT `args.get(3)`" precisely because the fixed-index read was
            // the bug — so a naive `contains` matches the warning against it
            // and fails the arm that heeded it. The media lock above records
            // the same trap from the other direction.
            let code: String = arm
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                !code.contains(&format!("args.get({index})")),
                "{marker} still reads a payload at a fixed index:\n{code}"
            );
        }
    }

    #[test]
    fn signal_shutdown_policy_preserves_daemon_with_terminal_runtimes() {
        assert!(!signal_shutdown_policy_allows_daemon_shutdown(
            0, 1, 1, 0, true
        ));
        assert!(!signal_shutdown_policy_allows_daemon_shutdown(
            0, 0, 0, 1, true
        ));
        assert!(!signal_shutdown_policy_allows_daemon_shutdown(
            0, 0, 0, 0, false
        ));
        assert!(!signal_shutdown_policy_allows_daemon_shutdown(
            1, 0, 0, 0, true
        ));
        assert!(signal_shutdown_policy_allows_daemon_shutdown(
            0, 0, 0, 0, true
        ));
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

    /// ⭐ THE CORNER CONTRACT, WAYLAND HALF. This test replaces
    /// `linux_wayland_window_profile_stays_opaque_by_default`, which asserted
    /// the opposite and so froze the defect in place: a non-KDE Wayland session
    /// was handed an opaque window, and an opaque window on Wayland can never
    /// round — there is no Shape extension to fall back to. Every Wayland
    /// compositor composites alpha, so the transparent profile is owed to all of
    /// them, not to an enumerated few.
    #[test]
    fn linux_wayland_window_profile_is_transparent_on_every_compositor() {
        for kde_session in [false, true] {
            let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
                transparent_opt_in: false,
                wayland_display_present: true,
                display_present: true,
                gdk_backend_x11: false,
                kde_session,
                xrpd_session: false,
            });
            assert!(
                profile.transparent,
                "wayland must round its corners regardless of desktop identity (kde={kde_session})"
            );
        }
    }

    /// The desktop's NAME may not change the outcome — only its reason string.
    /// This is the regression guard for the non-determinism itself: the same
    /// session, launched once with its desktop env hydrated and once without,
    /// must produce the same window.
    #[test]
    fn linux_wayland_window_profile_does_not_depend_on_desktop_identity() {
        let with_identity = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: true,
            display_present: true,
            gdk_backend_x11: false,
            kde_session: true,
            xrpd_session: false,
        });
        let without_identity = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: true,
            display_present: true,
            gdk_backend_x11: false,
            kde_session: false,
            xrpd_session: false,
        });
        assert_eq!(with_identity.transparent, without_identity.transparent);
        assert_eq!(with_identity.reason, "kde_wayland_transparent_profile");
        assert_eq!(without_identity.reason, "wayland_transparent_profile");
    }

    /// A session with no display environment at all is NOT a Wayland session as
    /// far as this pure function is concerned — the socket probe that rescues
    /// that case lives in `detect_linux_window_profile`, and feeds its answer in
    /// through `wayland_display_present`.
    #[test]
    fn linux_headless_window_profile_stays_opaque() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: false,
            display_present: false,
            gdk_backend_x11: false,
            kde_session: false,
            xrpd_session: false,
        });
        assert!(!profile.transparent);
        assert_eq!(profile.reason, "linux_opaque_default");
    }

    #[test]
    fn linux_kde_wayland_window_profile_uses_transparent_corners() {
        let profile = linux_window_profile_from_input(LinuxWindowProfileInput {
            transparent_opt_in: false,
            wayland_display_present: true,
            display_present: true,
            gdk_backend_x11: false,
            kde_session: true,
            xrpd_session: false,
        });
        assert!(profile.transparent);
        assert_eq!(profile.reason, "kde_wayland_transparent_profile");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_kde_wayland_defaults_to_native_backend_when_xwayland_exists() {
        use super::{LinuxDesktopBackendPolicyInput, linux_desktop_backend_policy_from_input};

        let policy = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
            allow_wayland_backend: false,
            force_x11_backend: false,
            gdk_backend_set: false,
            winit_backend_set: false,
            kde_session: true,
            wayland_display_present: true,
            display_present: true,
        });
        assert!(!policy.force_x11_backend);
        assert!(policy.set_gdk_backend);
        assert!(policy.set_winit_backend);
        assert_eq!(policy.backend, Some("wayland"));
        assert_eq!(policy.reason, "kde_wayland_native_default");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_kde_wayland_backend_policy_respects_forced_x11_fallback() {
        use super::{LinuxDesktopBackendPolicyInput, linux_desktop_backend_policy_from_input};

        let policy = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
            allow_wayland_backend: false,
            force_x11_backend: true,
            gdk_backend_set: false,
            winit_backend_set: false,
            kde_session: true,
            wayland_display_present: true,
            display_present: true,
        });
        assert!(policy.force_x11_backend);
        assert!(policy.set_gdk_backend);
        assert!(policy.set_winit_backend);
        assert_eq!(policy.backend, Some("x11"));
        assert_eq!(policy.reason, "x11_backend_explicitly_forced");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_kde_wayland_backend_policy_respects_explicit_env() {
        use super::{LinuxDesktopBackendPolicyInput, linux_desktop_backend_policy_from_input};

        let explicit_gdk =
            linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
                allow_wayland_backend: false,
                force_x11_backend: false,
                gdk_backend_set: true,
                winit_backend_set: false,
                kde_session: true,
                wayland_display_present: true,
                display_present: true,
            });
        assert!(!explicit_gdk.force_x11_backend);
        assert_eq!(explicit_gdk.backend, None);
        assert_eq!(explicit_gdk.reason, "gdk_backend_explicit");

        let opt_in = linux_desktop_backend_policy_from_input(LinuxDesktopBackendPolicyInput {
            allow_wayland_backend: true,
            force_x11_backend: false,
            gdk_backend_set: false,
            winit_backend_set: false,
            kde_session: true,
            wayland_display_present: true,
            display_present: true,
        });
        assert!(!opt_in.force_x11_backend);
        assert_eq!(opt_in.backend, None);
        assert_eq!(opt_in.reason, "wayland_backend_explicitly_allowed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_terminal_renderer_policy_keeps_canvas_opt_in() {
        use super::{LinuxTerminalRendererPolicyInput, linux_terminal_renderer_policy_from_input};

        // Wayland uses the WebGL GPU renderer (xterm 6's fastest tier). It presents
        // because configure_linux_webkit_compositing enables WebKitGTK compositing with
        // a software-GL safety net; the earlier "WebGL black" was compositing disabled.
        let wayland = linux_terminal_renderer_policy_from_input(LinuxTerminalRendererPolicyInput {
            explicit_canvas_env: false,
            gdk_backend_x11: false,
            wayland_display_present: true,
            display_present: true,
        });
        assert_eq!(wayland.set_canvas_env, Some("1"));
        assert_eq!(wayland.reason, "xterm_webgl_enabled_for_wayland");

        let x11 = linux_terminal_renderer_policy_from_input(LinuxTerminalRendererPolicyInput {
            explicit_canvas_env: false,
            gdk_backend_x11: true,
            wayland_display_present: true,
            display_present: true,
        });
        assert_eq!(x11.set_canvas_env, Some("0"));
        assert_eq!(x11.reason, "xterm_canvas_disabled_for_x11");

        let explicit =
            linux_terminal_renderer_policy_from_input(LinuxTerminalRendererPolicyInput {
                explicit_canvas_env: true,
                gdk_backend_x11: true,
                wayland_display_present: true,
                display_present: true,
            });
        assert_eq!(explicit.set_canvas_env, None);
        assert_eq!(explicit.reason, "xterm_canvas_explicit");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_canvas_env_explicit_only_for_user_overrides_not_inherited_policy() {
        use super::linux_canvas_env_is_user_explicit;

        // No canvas flag at all → not explicit.
        assert!(!linux_canvas_env_is_user_explicit(false, None));
        // Bare user export (no policy marker) → explicit, honor it.
        assert!(linux_canvas_env_is_user_explicit(true, None));
        // Re-exec of an honored override carries the explicit marker → still explicit.
        assert!(linux_canvas_env_is_user_explicit(
            true,
            Some("xterm_canvas_explicit")
        ));
        // Inherited launcher decisions (companion app-launch over ssh computed
        // the policy in a display-less env) → NOT explicit, recompute. This is
        // the rich-vs-broken renderer split: agent-launched GUIs were locked
        // to the DOM renderer by the inherited pair.
        assert!(!linux_canvas_env_is_user_explicit(
            true,
            Some("xterm_canvas_disabled_by_default")
        ));
        assert!(!linux_canvas_env_is_user_explicit(
            true,
            Some("xterm_canvas_disabled_for_x11")
        ));
        assert!(!linux_canvas_env_is_user_explicit(
            true,
            Some("xterm_webgl_enabled_for_wayland")
        ));
        // Empty marker behaves like no marker (bare user export).
        assert!(linux_canvas_env_is_user_explicit(true, Some("  ")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_desktop_env_parser_reads_null_separated_environment() {
        let env = linux_environ_bytes_to_map(
            b"DISPLAY=:1\0WAYLAND_DISPLAY=wayland-0\0XDG_RUNTIME_DIR=/run/user/1000\0",
        );
        assert_eq!(env.get("DISPLAY").map(String::as_str), Some(":1"));
        assert_eq!(
            env.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-0")
        );
        assert_eq!(
            env.get("XDG_RUNTIME_DIR").map(String::as_str),
            Some("/run/user/1000")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_desktop_env_picker_prefers_plasma_display_scope() {
        let mut ssh_env = BTreeMap::new();
        ssh_env.insert("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string());
        ssh_env.insert(
            "SSH_CONNECTION".to_string(),
            "192.0.2.1 1 192.0.2.2 2".to_string(),
        );

        let mut plasma_env = BTreeMap::new();
        plasma_env.insert("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string());
        plasma_env.insert("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string());
        plasma_env.insert("DISPLAY".to_string(), ":1".to_string());
        plasma_env.insert(
            "XAUTHORITY".to_string(),
            "/run/user/1000/xauth_example".to_string(),
        );
        plasma_env.insert("XDG_CURRENT_DESKTOP".to_string(), "KDE".to_string());
        plasma_env.insert("KDE_FULL_SESSION".to_string(), "true".to_string());

        let picked = linux_choose_desktop_environment(vec![
            ("sshd".to_string(), ssh_env),
            ("plasmashell".to_string(), plasma_env),
        ])
        .expect("plasma desktop environment selected");

        assert_eq!(picked.0, "plasmashell");
        assert_eq!(picked.1.get("DISPLAY").map(String::as_str), Some(":1"));
        assert_eq!(
            picked.1.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-0")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_gui_launch_env_hydrates_ssh_child_from_desktop_scope() {
        let mut current_env = BTreeMap::new();
        current_env.insert("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string());
        current_env.insert(
            "SSH_CONNECTION".to_string(),
            "192.0.2.1 1 192.0.2.2 2".to_string(),
        );

        let mut plasma_env = BTreeMap::new();
        plasma_env.insert("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string());
        plasma_env.insert("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string());
        plasma_env.insert("DISPLAY".to_string(), ":1".to_string());
        plasma_env.insert(
            "XAUTHORITY".to_string(),
            "/run/user/1000/xauth_example".to_string(),
        );
        plasma_env.insert("XDG_CURRENT_DESKTOP".to_string(), "KDE".to_string());
        plasma_env.insert("XDG_SESSION_DESKTOP".to_string(), "KDE".to_string());
        plasma_env.insert("DESKTOP_SESSION".to_string(), "plasma".to_string());
        plasma_env.insert("KDE_FULL_SESSION".to_string(), "true".to_string());
        plasma_env.insert(
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            "unix:path=/run/user/1000/bus".to_string(),
        );

        let overrides = linux_gui_entry_environment_overrides_from_desktop(
            &current_env,
            Some(("plasmashell".to_string(), plasma_env)),
        );

        assert_eq!(overrides.get("DISPLAY").map(String::as_str), Some(":1"));
        assert_eq!(
            overrides.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-0")
        );
        assert_eq!(
            overrides.get("XAUTHORITY").map(String::as_str),
            Some("/run/user/1000/xauth_example")
        );
        assert!(!overrides.contains_key("XDG_RUNTIME_DIR"));
        assert_eq!(
            overrides
                .get(LINUX_GUI_ENTRY_ENV_SOURCE_KEY)
                .map(String::as_str),
            Some("plasmashell")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_gui_launch_env_keeps_existing_display_scope() {
        let mut current_env = BTreeMap::new();
        current_env.insert("DISPLAY".to_string(), ":99".to_string());
        current_env.insert(
            "XDG_CURRENT_DESKTOP".to_string(),
            "test-desktop".to_string(),
        );
        current_env.insert("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string());

        let mut plasma_env = BTreeMap::new();
        plasma_env.insert("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string());
        plasma_env.insert("DISPLAY".to_string(), ":1".to_string());
        plasma_env.insert(
            "XAUTHORITY".to_string(),
            "/run/user/1000/xauth_example".to_string(),
        );

        let overrides = linux_gui_entry_environment_overrides_from_desktop(
            &current_env,
            Some(("plasmashell".to_string(), plasma_env)),
        );

        assert_eq!(overrides.get("DISPLAY"), None);
        assert_eq!(overrides.get("WAYLAND_DISPLAY"), None);
        assert_eq!(
            overrides.get("XAUTHORITY").map(String::as_str),
            Some("/run/user/1000/xauth_example")
        );
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

    #[test]
    fn preferred_executable_handoff_is_gui_entry_only() {
        assert!(should_handoff_to_preferred_executable(&[]));
        assert!(!should_handoff_to_preferred_executable(&[
            "--version".to_string()
        ]));
        assert!(!should_handoff_to_preferred_executable(&[
            "server".to_string(),
            "app".to_string(),
            "clients".to_string()
        ]));
        assert!(!should_handoff_to_preferred_executable(&[
            "server".to_string(),
            "app".to_string(),
            "launch".to_string(),
            "--wait-settled".to_string()
        ]));
    }

    #[test]
    fn superseded_client_retire_filter_targets_old_same_scope_gui() {
        let current = std::env::current_exe().expect("current exe");
        let scope = SignalClientScope {
            display: Some(":1".to_string()),
            wayland_display: Some("wayland-0".to_string()),
            xdg_session_id: None,
            xdg_runtime_dir: Some("/run/user/1000".to_string()),
            xauthority: Some("/run/user/1000/xauth".to_string()),
        };
        let old = ClientInstanceRecord {
            pid: 1234,
            started_at_ms: 1,
            client_id: None,
            linux_desktop_app_id: None,
            client_role: None,
            // Upstream's build-identity field, which these three fixtures were
            // not given when it landed — so `cargo test --workspace` could not
            // compile this bin at all. `None` is the honest fixture value: these
            // records describe a PRIOR client, and a record written before the
            // field existed carries nothing.
            build_commit: None,
            process_start_ticks: Some(77),
            executable_path: Some(
                "/home/user/.local/share/yggterm/direct/versions/2.1.49/yggterm".to_string(),
            ),
            display: Some(":1".to_string()),
            wayland_display: Some("wayland-0".to_string()),
            xdg_session_id: None,
            xdg_runtime_dir: Some("/run/user/1000".to_string()),
            xauthority: Some("/run/user/1000/xauth".to_string()),
            webkit_gl_environment: BTreeMap::new(),
        };
        assert!(should_retire_superseded_client(
            &old, 9999, &current, &scope
        ));
    }

    use crate::select_focus_handoff_target;

    fn handoff_record(pid: u32, started_at_ms: u128, role: Option<&str>, exe: &str) -> ClientInstanceRecord {
        ClientInstanceRecord {
            pid,
            started_at_ms,
            client_id: None,
            linux_desktop_app_id: None,
            client_role: role.map(str::to_string),
            build_commit: None,
            process_start_ticks: None,
            executable_path: Some(exe.to_string()),
            display: None,
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: None,
            webkit_gl_environment: BTreeMap::new(),
        }
    }

    #[test]
    fn a_shadow_is_never_the_window_a_fresh_gui_hands_off_to() {
        // ⛔⛔ THE REGRESSION THIS EXISTS FOR COST THE DESKTOP ITS GUI. The
        // selection filtered on executable path alone, and a shadow view runs
        // the identical executable — so the newest shadow was chosen as "the
        // window already running", the focus request went to something that is
        // not the user's window, and the fresh process exited. A deploy that
        // retires the incumbent first then leaves the desktop with nothing.
        let exe = std::path::Path::new("/opt/example-app/bin/example-gui");
        let records = vec![
            handoff_record(10, 100, Some("active"), "/opt/example-app/bin/example-gui"),
            // Newer, same binary, and NOT the user's window.
            handoff_record(11, 900, Some("shadow"), "/opt/example-app/bin/example-gui"),
        ];
        assert_eq!(
            select_focus_handoff_target(&records, exe),
            Some(10),
            "the active window wins even when a shadow is newer"
        );

        // And with ONLY a shadow registered there is no handoff target at all,
        // so the fresh process must go on to open its own window.
        let shadow_only = vec![handoff_record(11, 900, Some("shadow"), "/opt/example-app/bin/example-gui")];
        assert_eq!(select_focus_handoff_target(&shadow_only, exe), None);
    }

    #[test]
    fn a_legacy_record_without_a_role_is_still_a_handoff_target() {
        // Reading `None` as a shadow would break handoff for every client that
        // predates the role field — the opposite failure, equally real.
        let exe = std::path::Path::new("/opt/example-app/bin/example-gui");
        let records = vec![handoff_record(12, 100, None, "/opt/example-app/bin/example-gui")];
        assert_eq!(select_focus_handoff_target(&records, exe), Some(12));
    }

    #[test]
    fn a_focus_reply_that_reports_failure_is_not_a_handoff() {
        // ⛔ The other half of the same outage. The old call site read
        // `run_app_control_focus_window(..).is_ok()`, which is true whenever the
        // request completed a round trip — including for this reply, whose own
        // text says focus did not happen. The process exited on it.
        let refused = yggterm_server::AppControlResponse {
            request_id: "r-1".to_string(),
            handled_by_pid: 4242,
            completed_at_ms: 1,
            output_path: None,
            data: None,
            error: Some(
                "app-control focus request did not produce native window focus".to_string(),
            ),
        };
        assert!(!yggterm_server::app_control_response_took_focus(&refused));

        let accepted = yggterm_server::AppControlResponse {
            request_id: "r-2".to_string(),
            handled_by_pid: 4242,
            completed_at_ms: 1,
            output_path: None,
            data: None,
            error: None,
        };
        assert!(yggterm_server::app_control_response_took_focus(&accepted));
    }

    #[test]
    fn a_deleted_binary_is_detected_and_a_live_one_is_not() {
        // ⛔ BOTH CONTROLS, SAME RUN. A positive control alone cannot tell a
        // working detector from one that has collapsed to a constant `true`,
        // and this project has shipped exactly that mistake before.

        // Negative control: this test's own process is very much not deleted.
        assert!(
            !client_process_runs_a_deleted_binary(std::process::id()),
            "the running test binary must not read as deleted"
        );

        // Positive control: copy a real binary, run it, delete the copy. The
        // process survives its own file, which is precisely the orphan's state.
        let dir = std::env::temp_dir().join(format!("ygg-deleted-exe-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let copy = dir.join("sleeper");
        let Ok(_) = fs::copy("/bin/sleep", &copy) else {
            // No /bin/sleep (or an unwritable temp dir) means the positive
            // control could not be established. Say so rather than passing on
            // the negative control alone, which would prove nothing.
            eprintln!("skipping positive control: could not stage a copy of /bin/sleep");
            return;
        };
        let mut child = std::process::Command::new(&copy)
            .arg("30")
            .spawn()
            .expect("spawn the staged binary");
        // ⛔ `spawn` RETURNS BEFORE THE CHILD HAS EXEC'd, so unlinking the copy
        // on the next line races the kernel: until the exec lands,
        // `/proc/<pid>/exe` still points at THIS test binary, which is not
        // deleted, and the positive control reads false for a reason that has
        // nothing to do with the code under test.
        //
        // The signature is what gives it away and is worth recognising
        // elsewhere: **it passed five times alone and failed inside a full
        // parallel run**, because the loser is whichever process is slowest to
        // be scheduled, and nothing is slow when it is the only thing running.
        // A test that only fails under load is not flaky-and-harmless; it is
        // measuring the machine instead of the behaviour.
        let staged = fs::canonicalize(&copy).unwrap_or_else(|_| copy.clone());
        let exe_link = format!("/proc/{}/exe", child.id());
        let mut execed = false;
        for _ in 0..200 {
            if fs::read_link(&exe_link).is_ok_and(|target| target == staged) {
                execed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = fs::remove_file(&copy);
        let observed = client_process_runs_a_deleted_binary(child.id());
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            execed,
            "the staged binary never became the child's /proc/<pid>/exe, so the \
             positive control was never established — this is a fault in the \
             test's setup, not a verdict on the code"
        );
        assert!(
            observed,
            "a process whose binary was unlinked must read as deleted"
        );
    }

    #[test]
    fn superseded_client_retire_filter_keeps_current_or_other_scope_gui() {
        let current = std::env::current_exe().expect("current exe");
        let current_text = current.to_string_lossy().to_string();
        let scope = SignalClientScope {
            display: Some(":1".to_string()),
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: Some("/run/user/1000/xauth".to_string()),
        };
        let same_exe = ClientInstanceRecord {
            pid: 1234,
            started_at_ms: 1,
            client_id: None,
            linux_desktop_app_id: None,
            client_role: None,
            // Upstream's build-identity field, which these three fixtures were
            // not given when it landed — so `cargo test --workspace` could not
            // compile this bin at all. `None` is the honest fixture value: these
            // records describe a PRIOR client, and a record written before the
            // field existed carries nothing.
            build_commit: None,
            process_start_ticks: Some(77),
            executable_path: Some(current_text),
            display: Some(":1".to_string()),
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: Some("/run/user/1000/xauth".to_string()),
            webkit_gl_environment: BTreeMap::new(),
        };
        assert!(!should_retire_superseded_client(
            &same_exe, 9999, &current, &scope
        ));

        let other_display = ClientInstanceRecord {
            pid: 5678,
            started_at_ms: 1,
            client_id: None,
            linux_desktop_app_id: None,
            client_role: None,
            // Upstream's build-identity field, which these three fixtures were
            // not given when it landed — so `cargo test --workspace` could not
            // compile this bin at all. `None` is the honest fixture value: these
            // records describe a PRIOR client, and a record written before the
            // field existed carries nothing.
            build_commit: None,
            process_start_ticks: Some(88),
            executable_path: Some(
                "/home/user/.local/share/yggterm/direct/versions/2.1.49/yggterm".to_string(),
            ),
            display: Some(":99".to_string()),
            wayland_display: None,
            xdg_session_id: None,
            xdg_runtime_dir: None,
            xauthority: Some("/tmp/xvfb/Xauthority".to_string()),
            webkit_gl_environment: BTreeMap::new(),
        };
        assert!(!should_retire_superseded_client(
            &other_display,
            9999,
            &current,
            &scope
        ));
    }

    #[test]
    fn superseded_client_close_command_preserves_live_sessions() {
        assert!(matches!(
            superseded_client_close_command(),
            yggterm_server::AppControlCommand::CloseWindowPreservingSessions { .. }
        ));
    }

    #[test]
    fn superseded_client_retirement_is_process_only_not_app_control_close() {
        assert_eq!(
            superseded_client_retirement_strategy_label(),
            "kill_process_only_no_client_cleanup"
        );
        #[cfg(unix)]
        assert_eq!(superseded_client_termination_signal(), libc::SIGKILL);
    }

    #[test]
    fn gui_entry_defers_superseded_retirement_until_shell_active_handoff() {
        assert!(!main_should_retire_superseded_clients_before_shell(&[]));
        assert!(!main_should_retire_superseded_clients_before_shell(&[
            "server".to_string()
        ]));
    }

    /// The measured pair on the live GUI: soft 1024, hard 1,048,576. The soft
    /// limit is an inherited default nobody chose, and it is the wall many-tab
    /// use hits first after memory — each realized webview brings IPC sockets to
    /// its web and network processes plus imported buffer descriptors into the
    /// UI process.
    #[test]
    fn the_inherited_1024_descriptor_soft_limit_is_raised() {
        assert_eq!(
            raised_file_descriptor_soft_limit(1024, 1_048_576),
            Some(FILE_DESCRIPTOR_SOFT_LIMIT_TARGET),
            "the live host's own limits must produce a raise"
        );
        // Raised, but NOT to the hard limit: a very high RLIMIT_NOFILE makes
        // fork+exec expensive for anything that closes descriptors in a loop up
        // to it, and yggterm spawns ssh children constantly.
        assert!(FILE_DESCRIPTOR_SOFT_LIMIT_TARGET < 1_048_576);
    }

    /// It must never LOWER the limit, and never ask for more than the hard cap.
    /// A soft limit already above the target was set deliberately by whoever
    /// launched us; stepping on that is the same mistake in the other direction.
    #[test]
    fn the_descriptor_raise_never_lowers_and_never_exceeds_the_hard_cap() {
        assert_eq!(raised_file_descriptor_soft_limit(200_000, 1_048_576), None);
        assert_eq!(
            raised_file_descriptor_soft_limit(
                FILE_DESCRIPTOR_SOFT_LIMIT_TARGET,
                FILE_DESCRIPTOR_SOFT_LIMIT_TARGET
            ),
            None,
            "already at target is not a raise"
        );
        // A hard limit BELOW the target clamps to the hard limit — asking for
        // more would just fail the setrlimit and leave 1024 in place.
        assert_eq!(raised_file_descriptor_soft_limit(1024, 4096), Some(4096));
        assert_eq!(raised_file_descriptor_soft_limit(4096, 4096), None);
        // An unlimited hard cap still lands on the documented target.
        assert_eq!(
            raised_file_descriptor_soft_limit(1024, u64::MAX),
            Some(FILE_DESCRIPTOR_SOFT_LIMIT_TARGET)
        );
    }
}
