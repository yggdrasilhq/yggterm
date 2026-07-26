#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

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
use yggterm_server::{
    AppControlPreviewLayout, AppControlRightPanelMode, AppControlViewMode, ClientInstanceRecord,
    PersistedDaemonState, ProbeTerminalViewportInputMode, ScreenshotPostProcess, SessionKind,
    WorkspaceViewMode, YggtermServer, active_client_instance_records, default_endpoint,
    detect_ghostty_host, ensure_local_daemon_running, focus_live_with_view,
    local_headless_companion_executable_from_current, open_remote_session_with_view,
    open_stored_session_with_view, ping, reorder_live_sessions_scoped,
    resolve_client_daemon_endpoint, row_order_ledger_report, run_app_control_background_window,
    run_app_control_close_window, run_app_control_close_window_preserving_sessions,
    run_app_control_create_terminal, run_app_control_describe_rows, run_app_control_describe_state,
    run_app_control_desktop_identity, run_app_control_dom_eval, run_app_control_drag,
    run_app_control_dump_state, run_app_control_ensure_web_surface, run_app_control_focus_window,
    run_app_control_grid, run_app_control_key, run_app_control_list_clients,
    run_app_control_move_window_by, run_app_control_open_path,
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
    run_app_control_set_preview_layout, run_app_control_set_right_panel_mode,
    run_app_control_set_row_expanded, run_app_control_set_search,
    run_app_control_set_session_keep_alive, run_app_control_set_theme_editor_open,
    run_app_control_set_theme_editor_values, run_app_control_set_tree_selection,
    run_app_control_set_ui_theme, run_app_control_set_window_chrome_hover,
    run_app_control_start_action, run_app_control_submit_terminal_prompt,
    run_app_control_trigger_update_check, run_app_control_web_surface_await,
    run_app_control_web_surface_batch, run_app_control_web_surface_capture_element,
    run_app_control_web_surface_close, run_app_control_web_surface_cookies,
    run_app_control_web_surface_devtools, run_app_control_web_surface_do,
    run_app_control_web_surface_eval, run_app_control_web_surface_fill,
    run_app_control_web_surface_fill_vault, run_app_control_web_surface_frames,
    run_app_control_web_surface_lease, run_app_control_web_surface_read,
    run_app_control_web_surface_reload, run_app_control_web_surface_screenshot,
    run_app_control_web_surface_totp, run_app_control_web_surface_wait, run_attach, run_daemon,
    run_screenrecord_capture, run_screenshot_capture, run_screenshot_capture_with_post_process,
    run_trace_bundle, run_trace_follow, run_trace_tail, run_trace_transitions, shutdown, snapshot,
    start_local_session, status, terminal_history, terminal_resize, terminal_restart,
    terminal_retained_snapshot, terminal_snapshot, terminal_write, try_run_remote_server_command,
};
use yggterm_shell::{
    ShellBootstrap, launch_shell, start_daemon_watchdog, terminal_identity_appearance_for_settings,
    warm_daemon_start,
};
use yggui_contract::UiTheme;

/// The no-orphan-affordance audit (spec §12, `docs/alt-keytips.md`). Walks the
/// visible DOM interactables and reports every one carrying neither a
/// `data-keytip-node` marker (declared) nor `data-keytip-exempt`. The KeyTip
/// tree is built in Rust; this only reads the DOM to MEASURE coverage, which is
/// what makes "wire the whole UI" a number that must be zero. Runs through the
/// existing dom-eval channel (`return`s its report).
const KEYTIPS_AUDIT_JS: &str = r#"
    const SEL = 'button, [role=button], a[href], input, select, [data-sidebar-row-path], [data-app-verb]';
    function vis(el){
        if (!el.getClientRects().length) return false;
        const r = el.getBoundingClientRect();
        if (r.width < 2 || r.height < 2) return false;      // 1px shortcut proxies are not affordances
        const cs = window.getComputedStyle(el);
        if (!cs) return true;
        if (cs.visibility === 'hidden' || cs.display === 'none') return false;
        if (parseFloat(cs.opacity || '1') === 0) return false;
        if (cs.pointerEvents === 'none') return false;      // cannot be clicked -> not an affordance
        return true;
    }
    // §12.1: report REACHABLE / EXCUSED / ORPHAN as three numbers, not one.
    // The old audit counted `data-keytip-exempt` as satisfied and returned a
    // single orphan_count, so exempting a SUBTREE silently retired every control
    // inside it. Measured 2026-07-22: it read 0 while 6 of 71 affordances were
    // reachable (25 hidden under settings-panel alone). Subtree suppression is
    // now a VIOLATION class of its own — exemption is per-element or it is a
    // hiding place.
    function describe(el){
        const label = (el.getAttribute('aria-label') || el.textContent || el.getAttribute('data-app-verb') || el.id || '')
            .replace(/\s+/g, ' ').trim().slice(0, 60);
        return { tag: el.tagName.toLowerCase(), id: el.id || '', cls: (el.className && el.className.baseVal !== undefined ? el.className.baseVal : String(el.className || '')).slice(0, 60), label: label };
    }
    const all = Array.prototype.slice.call(document.querySelectorAll(SEL));
    const visible = all.filter(vis);
    const orphans = [];
    const subtreeSuppressed = [];
    const excusedReasons = {};
    const suppressedBySubtree = {};
    let reachable = 0, excused = 0;
    visible.forEach(function(el){
        if (el.hasAttribute('data-keytip-node')) { reachable++; return; }
        // A control whose badge marker is a CHILD span -- the shipped pattern for
        // every wired button, so the badge paints over the control -- is reachable
        // no matter what encloses it. This MUST precede the exempt-ancestor test:
        // otherwise wiring a control that still sits inside a subtree-exempt panel
        // reports it as a violation, and the metric cannot show its own fix.
        // Measured 2026-07-22: the already-wired settings/theme.light|dark buttons
        // were both counted as `suppressed_by: settings-panel`.
        if (el.querySelector('[data-keytip-node]')) { reachable++; return; }
        if (el.hasAttribute('data-keytip-exempt')) {
            excused++;
            const why = el.getAttribute('data-keytip-exempt') || '(no reason)';
            excusedReasons[why] = (excusedReasons[why] || 0) + 1;
            return;
        }
        const exemptAncestor = el.closest('[data-keytip-exempt]');
        if (exemptAncestor) {
            const why = exemptAncestor.getAttribute('data-keytip-exempt') || '(no reason)';
            suppressedBySubtree[why] = (suppressedBySubtree[why] || 0) + 1;
            const row = describe(el); row.suppressed_by = why;
            subtreeSuppressed.push(row);
            return;
        }
        // The declared-descendant case is handled above; what is left here is a
        // container whose only marked descendant is exempt.
        if (el.querySelector('[data-keytip-exempt]')) { reachable++; return; }
        orphans.push(describe(el));
    });
    const violations = orphans.length + subtreeSuppressed.length;
    return {
        visible_interactables: visible.length,
        reachable: reachable,
        excused: excused,
        excused_reasons: excusedReasons,
        subtree_suppressed_count: subtreeSuppressed.length,
        suppressed_by_subtree: suppressedBySubtree,
        subtree_suppressed: subtreeSuppressed.slice(0, 300),
        orphan_count: orphans.length,
        orphans: orphans.slice(0, 300),
        // The number that must go to zero. Subtree suppression counts against
        // it, so the old escape hatch can no longer flatter the score.
        violations: violations,
        reachable_pct: visible.length ? Math.round((reachable / visible.length) * 100) : 100
    };
"#;
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

/// Parse the element-addressing flags shared by every `do` verb into ONE
/// [`WebElementRef`] (C4). One parser, so `click`, `type`, `key` and `fill`
/// can never disagree about how an element is named.
///
/// Precedence is fixed and documented rather than "most specific wins":
/// `--selector` (CSS) > `--role`+`--label` > text. A fixed order is what keeps
/// a script's meaning stable when someone adds a stray flag.
///
/// `text_addresses` is true only for verbs that carry NO text payload
/// (`click`), where `--text "Proceed to Pay"` is the natural spelling. For
/// `type`/`fill`, `--text` is the value being typed, so text addressing spells
/// itself `--target-text` — which is also accepted everywhere, so a script can
/// always use the unambiguous form.
fn parse_web_element_ref(
    args: &[String],
    text_addresses: bool,
) -> anyhow::Result<Option<yggterm_server::WebElementRef>> {
    use yggterm_server::WebElementRef;
    let nth = cli_flag_value(args, "--nth")
        .map(|raw| raw.parse::<usize>().context("--nth needs a number"))
        .transpose()?;
    if let Some(selector) = cli_flag_value(args, "--selector") {
        return Ok(Some(WebElementRef::Css(selector.to_string())));
    }
    if let Some(role) = cli_flag_value(args, "--role") {
        let label = cli_flag_value(args, "--label")
            .context("--role needs --label (the element's accessible name)")?;
        return Ok(Some(WebElementRef::Role {
            role: role.to_string(),
            label: label.to_string(),
            nth,
        }));
    }
    let text = cli_flag_value(args, "--target-text").or_else(|| {
        if text_addresses {
            cli_flag_value(args, "--text")
        } else {
            None
        }
    });
    if let Some(text) = text {
        return Ok(Some(WebElementRef::Text {
            text: text.to_string(),
            exact: args.iter().any(|arg| arg == "--exact"),
            tag: cli_flag_value(args, "--tag").map(str::to_string),
            nth,
        }));
    }
    Ok(None)
}

/// Escape a literal so it matches itself as a regex.
///
/// `--until url:contains:<s>` is SUGAR, not a second predicate: it compiles to
/// the same `UrlMatches` the regex form does, so there is exactly one url rule
/// in the GUI. Escaping happens here, where the user's literal is, rather than
/// by teaching the matcher a second mode.
fn regex_escape_literal(literal: &str) -> String {
    let mut escaped = String::with_capacity(literal.len() * 2);
    for c in literal.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Parse `--frame <index|path|url-substring>` into a [`WebFrameRef`].
///
/// Three spellings, one meaning: a bare number is `window.frames[n]`, a
/// dotted/comma path (`0.2`) is a descent — the form `web frames` reports, so
/// its output feeds straight back in — and anything else is a url substring.
fn parse_web_frame_ref(args: &[String]) -> anyhow::Result<Option<yggterm_server::WebFrameRef>> {
    use yggterm_server::WebFrameRef;
    let Some(raw) = cli_flag_value(args, "--frame") else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("--frame needs an index, a path like 0.2, or a url substring");
    }
    if let Ok(index) = raw.parse::<usize>() {
        return Ok(Some(WebFrameRef::Index(index)));
    }
    let separators: &[char] = &['.', ','];
    if raw.contains(separators)
        && raw
            .split(separators)
            .all(|part| part.trim().parse::<usize>().is_ok())
    {
        let path = raw
            .split(separators)
            .map(|part| part.trim().parse::<usize>().unwrap_or(0))
            .collect();
        return Ok(Some(WebFrameRef::Path(path)));
    }
    Ok(Some(WebFrameRef::UrlContains(raw.to_string())))
}

/// Split one batch-script line into argv tokens, honouring `'`/`"` quoting and
/// backslash escapes.
///
/// A batch line IS a `do` invocation, so it has to tokenize the way a shell
/// would or `--text "Proceed to Pay"` would arrive as three arguments. Kept
/// deliberately small: quotes and backslashes, no expansion, no globbing, no
/// variables — a batch script is a list of verbs, not a shell.
fn tokenize_argv_line(line: &str) -> anyhow::Result<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let escaped = chars.next().context("trailing backslash in batch line")?;
                current.push(escaped);
                has_token = true;
            }
            '\'' | '"' if quote.is_none() => {
                quote = Some(c);
                has_token = true;
            }
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if quote.is_some() {
        anyhow::bail!("unterminated quote in batch line");
    }
    if has_token {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Parse a batch script — one `do`-style invocation per line — into actions.
///
/// Blank lines and `#` comments are skipped. Every line goes through the SAME
/// `parse_web_surface_do_action` a CLI verb goes through, so a batched action
/// and a typed one can never mean different things; that shared parser is the
/// single source of truth for what a `do` verb IS.
fn parse_web_do_batch_script(
    script: &str,
) -> anyhow::Result<Vec<yggterm_server::WebSurfaceDoAction>> {
    let mut actions = Vec::new();
    for (index, line) in script.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let tokens =
            tokenize_argv_line(trimmed).with_context(|| format!("batch line {}", index + 1))?;
        // `parse_web_surface_do_action` reads the verb at args[4] — the shape a
        // real `server app web do …` invocation has — so the line is spliced
        // onto that prefix rather than parsed by a second, parallel reader.
        let mut argv: Vec<String> = ["server", "app", "web", "do"]
            .iter()
            .map(|part| (*part).to_string())
            .collect();
        argv.extend(tokens);
        actions.push(
            parse_web_surface_do_action(&argv)
                .with_context(|| format!("batch line {}: {trimmed}", index + 1))?,
        );
    }
    if actions.is_empty() {
        anyhow::bail!("batch script contained no actions");
    }
    Ok(actions)
}

/// Parse a `server app web do <verb> …` invocation into a typed
/// `WebSurfaceDoAction` (agent control plane `do` verb, slice 2b). Coordinates
/// are document-space CSS pixels; the GUI resolves selectors + maps to widget
/// px. `args[4]` is the verb (`click`/`move`/`scroll`/`type`/`key`).
fn parse_web_surface_do_action(
    args: &[String],
) -> anyhow::Result<yggterm_server::WebSurfaceDoAction> {
    use yggterm_server::{AppControlPointerButton, WebSurfaceDoAction};
    let verb = args
        .get(4)
        .map(String::as_str)
        .context("missing verb for server app web do (click|move|scroll|type|fill|key)")?;
    let button = match cli_flag_value(args, "--button") {
        Some("middle" | "auxiliary" | "2") => AppControlPointerButton::Middle,
        Some("secondary" | "right" | "3") => AppControlPointerButton::Secondary,
        _ => AppControlPointerButton::Primary,
    };
    let req_f64 = |flag: &str| -> anyhow::Result<f64> {
        cli_flag_value(args, flag)
            .with_context(|| format!("missing {flag} for server app web do {verb}"))?
            .parse::<f64>()
            .with_context(|| format!("invalid number for {flag}"))
    };
    let opt_f64 = |flag: &str| cli_flag_value(args, flag).and_then(|v| v.parse::<f64>().ok());
    let action = match verb {
        "click" | "tap" => {
            // Addressed click (CSS / text / role+label) when any addressing flag
            // is present; blind coordinates only when none is. The addressed
            // path resolves in the page immediately before injection, which is
            // both safer (it hit-tests) and stale-proof.
            if let Some(target) = parse_web_element_ref(args, true)? {
                WebSurfaceDoAction::ClickSelector {
                    selector: target,
                    button,
                }
            } else {
                WebSurfaceDoAction::Click {
                    x: req_f64("--x")?,
                    y: req_f64("--y")?,
                    button,
                }
            }
        }
        "move" | "hover" => WebSurfaceDoAction::Move {
            x: req_f64("--x")?,
            y: req_f64("--y")?,
        },
        "scroll" => {
            let dx = opt_f64("--dx").unwrap_or(0.0);
            let dy = opt_f64("--dy").unwrap_or(0.0);
            if dx == 0.0 && dy == 0.0 {
                anyhow::bail!("server app web do scroll needs --dx and/or --dy");
            }
            WebSurfaceDoAction::Scroll {
                x: opt_f64("--x"),
                y: opt_f64("--y"),
                dx,
                dy,
            }
        }
        "type" => WebSurfaceDoAction::Type {
            text: cli_flag_value(args, "--text")
                .context("missing --text for server app web do type")?
                .to_string(),
            selector: parse_web_element_ref(args, false)?,
        },
        // `fill` REPLACES a field's contents; `type` appends. Use it whenever the
        // field may already hold something — see the merge failure documented on
        // `WebSurfaceDoAction::Fill`. `--selector-set` is the comma-separated
        // box list of a segmented input (a 6-box OTP), which must be cleared
        // box-by-box or the old characters survive inside the widget.
        "fill" | "set" => WebSurfaceDoAction::Fill {
            text: cli_flag_value(args, "--text")
                .context("missing --text for server app web do fill")?
                .to_string(),
            selector: parse_web_element_ref(args, false)?,
            selectors: cli_flag_value(args, "--selector-set")
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|css| yggterm_server::WebElementRef::Css(css.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "key" | "press" => WebSurfaceDoAction::Key {
            key: cli_flag_value(args, "--key")
                .context("missing --key for server app web do key")?
                .to_string(),
            mods: cli_flag_value(args, "--mods")
                .map(|raw| raw.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            selector: parse_web_element_ref(args, false)?,
        },
        other => {
            anyhow::bail!("unsupported web do verb: {other} (click|move|scroll|type|fill|key)")
        }
    };
    Ok(action)
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
  yggterm server attach <session> [cwd]
  yggterm server connect <session-path> [--view terminal|preview] [--top|--after <path>]
  yggterm server connect --list
  yggterm server order [--json]
  yggterm server reorder <session-path>... | --stdin [--scope <scope>]
  yggterm server ledger [--scope <scope>]
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

/// EVERY `server app web` action, with its usage.
///
/// This exists because a hand-maintained usage string is exactly the thing that
/// drifts: `ensure`, `fill` and `totp` were implemented and undocumented, and a
/// stale usage block is what produced a "not deployed" misdiagnosis in the
/// field on 2026-07-22 (docs/agent-control-plane.md). An agent reading `--help`
/// and concluding a verb does not exist is a failure mode of the DOCS.
///
/// One list, rendered into the usage text, and a test that fails when the
/// dispatcher's own match arms disagree with it. An alias carries an empty
/// usage string — it is named in its primary's line.
const WEB_ACTIONS: &[(&str, &str)] = &[
    (
        "eval",
        "  yggterm server app web eval (<script>|--script <js>|--stdin) [--frame <f>] [--session <path>]\n",
    ),
    (
        "read",
        "  yggterm server app web read [--as snapshot|forms|tables|readable|links|text|html] [--frame <f>] [--session <path>]\n    read with NO --frame searches EVERY reachable frame and returns\n    frames:[ {{frame:{{path,url}},result}} ] — the top document is frame []\n",
    ),
    (
        "await",
        "  yggterm server app web await (<script>|--script <file>|--stdin) [--await-timeout <ms>] [--session <path>]\n    the script is the BODY of an async function; `return` its value.\n    `eval` cannot return a Promise — this is the one verb that can.\n",
    ),
    (
        "frames",
        "  yggterm server app web frames [--session <path>]\n    --frame <f> is an index (2), a path (0.2), or a url substring (billdesk)\n",
    ),
    (
        "do",
        "  yggterm server app web do <click|move|scroll|type|fill|key> <target> [--text …|--key …|--mods …] [--generation <n>] [--new-batch] [--session <path>]\n    target (resolved in the page at click time, precedence in this order):\n      --selector <css> | --role <r> --label <s> [--nth <n>]\n      | --target-text <s> [--exact] [--tag <css>] [--nth <n>]   (on `click`, --text is an alias)\n      | --selector-set <css,css,…>   (segmented inputs: one box per character)\n      | --x <n> --y <n>              (blind coordinates; prefer an addressed target)\n",
    ),
    (
        "fill-vault",
        "  yggterm server app web fill-vault --item <name> [--field password|username|totp|notes] [--user <u>] <target> [--session <path>]\n",
    ),
    (
        "fill-card",
        "  yggterm server app web fill-card --item <name> [--field number|expiry|code|holder] <target> [--session <path>]\n",
    ),
    (
        "fill",
        "  yggterm server app web fill [--entry <name>] [--user <u>] [--session <path>]\n    auto-match the page host against the vault and fill the login form.\n    For ONE named field into ONE addressed element, use fill-vault.\n",
    ),
    (
        "totp",
        "  yggterm server app web totp [--entry <name>] [--user <u>] [--session <path>]  (alias: code)\n    put the entry's current TOTP code into the page's one-time-code field\n",
    ),
    (
        "batch",
        "  yggterm server app web batch (--script <file>|--stdin) [--stop-on-error] [--generation <n>] [--session <path>]\n    one `do` invocation per line; # comments and blank lines skipped\n",
    ),
    (
        "wait",
        "  yggterm server app web wait --until <cond> [--visible] [--wait-timeout <ms>] [--session <path>]\n    cond: load:committed | load:finished | idle:<ms> | settled:<ms>\n        | selector:<css> | js:<expr> | url:matches:<regex> | url:contains:<substring>\n    url:* and settled:* are read from the ENGINE, so they survive a navigation\n    that makes every page-side predicate unavailable\n",
    ),
    (
        "ensure",
        "  yggterm server app web ensure --session <path> [--ttl <secs>]\n    LIVENESS-based: probes the page with a real round trip, rebuilds a corpse,\n    and reports generation_before/generation_after + healed so a caller can tell\n    a new page from the same one. Refusals name WHICH fact failed (no_declare,\n    declare_stale, declare_url_scheme_refused, daemon_declare_unavailable, ...).\n",
    ),
    (
        "reload",
        "  yggterm server app web reload --session <path>\n",
    ),
    ("close", "  yggterm server app web close --session <path>\n"),
    (
        "lease",
        "  yggterm server app web lease --ttl <secs> [--session <path>]\n",
    ),
    (
        "screenshot",
        "  yggterm server app web screenshot [output.png] [--session <path>]\n",
    ),
    (
        "cookies",
        "  yggterm server app web cookies (--import <jar>|--export <jar>) [--session <path>]\n    Netscape format, both ways (what `curl -c`/`-b` writes and reads).\n    WARNING: the jar is per-PROFILE; an unqualified surface is `default`, the\n    user's own browsing jar. Use an `agent-<n>` profile surface. Export covers\n    every ROOT-PATH cookie per domain — path-scoped cookies are not visible to\n    the engine API and are reported as export_scope=root_path_per_domain.\n",
    ),
    (
        "capture-element",
        "  yggterm server app web capture-element <target> [out.png] [--split <n>] [--session <path>]\n    in-page canvas rasterize of one <img>/<canvas>/<video>; works on an UNMAPPED surface\n",
    ),
    (
        "devtools",
        "  yggterm server app web devtools [--close] [--session <path>]\n",
    ),
    ("code", ""),
    ("capture", ""),
];

/// Render the `server app web` usage block from [`WEB_ACTIONS`].
fn web_usage_block() -> String {
    WEB_ACTIONS.iter().map(|(_, usage)| *usage).collect()
}

fn print_server_app_help() {
    println!(
        "usage:
  yggterm server app clients
  yggterm server app desktop-identity
  yggterm server app launch [--wait-visible] [--wait-settled] [--allow-multi-window]
  yggterm server app state [--pid <pid>]
  yggterm server app rows [--pid <pid>]
  yggterm server app update <check|restart> [--force]
    restart REFUSES while an agent web-surface lease is live (agent_lease_active);
    pre-flight with `server app state | jq .agent_leases`
  yggterm server app screenshot [output] [--pid <pid>] [--region terminal|full] [--crop x,y,w,h] [--scale n] [--backend os]
  yggterm server app open <session-path> [--view <terminal|preview>] [--pid <pid>]
  yggterm server app session <remove|delete> <session-path> [--pid <pid>]
  yggterm server app session rename <session-path> <title> [--pid <pid>]
  yggterm server app start-page [--pid <pid>]
  yggterm server app terminal <new|send|focus|scroll|probe-type|probe-scroll|probe-select|probe-context-menu> ...
  yggterm server app terminal scroll <session> --to <top|bottom|±N>
  yggterm server app terminal read-buffer <session> [--mode screen|full|cells]
  yggterm server app terminal send <session> (--data <data>|--stdin)
  yggterm server app keytips audit
  yggterm server app command <list|invoke <id>>
{web_usage}
targeting (any app verb): [--pid <pid>] or [--client <name>] picks which GUI
  worker handles the verb; --client names a client by its --client-id (a shadow
  view client, slice 4.3) — see `server app clients`. --pid wins if both given;
  with one GUI and no target it routes there automatically.",
        web_usage = web_usage_block()
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

/// The daemon this CLI invocation talks to.
///
/// NEVER `default_endpoint` — that is OUR OWN version's socket. A CLI binary
/// newer than the running daemon finds nothing there, and the old code then let
/// `ensure_local_daemon_running` spawn a RIVAL daemon at our version. The rival
/// cold-restores `server-state.json`, which resurrects sessions the user closed
/// under the live daemon, silently drops keep-alive on every session whose
/// terminal runtime the rival does not own, and re-seeds the row order. The GUI
/// already avoids this via `resolve_client_daemon_endpoint`; CLI verbs must use
/// the same resolver or a single `yggterm-headless server ...` call from a
/// freshly built binary forks the world.
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

/// The trailing session identifier of a session path (`.../<uuid>` → `<uuid>`),
/// used to match a requested path against the daemon's canonical key regardless
/// of scheme/prefix normalization.
fn connect_path_session_uuid(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Parse a remote-SCANNED Codex path (`remote-session://<machine>/<uuid>`) into
/// `(machine_key, id)`. Deliberately does NOT match `remote-cc://`: mirroring the
/// GUI's `open_session_row`, only a scanned Codex row goes through
/// OpenRemoteSession; a Claude Code row is opened as a stored session (its path
/// is not a remote-scanned path, and OpenRemoteSession would look it up as a
/// Codex transcript and fail with "saved Codex session is no longer available").
fn parse_remote_scanned_connect_path(path: &str) -> Option<(String, String)> {
    let rest = path
        .trim_start_matches('/')
        .strip_prefix("remote-session://")?;
    let (machine, id) = rest.split_once('/')?;
    if machine.is_empty() || id.is_empty() {
        return None;
    }
    Some((machine.to_string(), id.to_string()))
}

/// Session kind for a path we are opening as a stored session — the CLI twin of
/// the GUI's `session_kind_for_row`.
fn connect_session_kind_for_path(path: &str) -> SessionKind {
    if path.starts_with("remote-cc://") || path.contains("/.claude/projects/") {
        SessionKind::ClaudeCode
    } else {
        SessionKind::Codex
    }
}

/// The scanned `(cwd, title)` for a session id, looked up from the daemon's
/// remote scans. The resume needs the right cwd (`claude -r` / `codex resume`
/// run inside the session's directory), so pass it through like the GUI does.
fn connect_scanned_metadata(
    snapshot: &yggterm_server::ServerUiSnapshot,
    path: &str,
) -> (Option<String>, Option<String>) {
    let want = connect_path_session_uuid(path);
    snapshot
        .remote_machines
        .iter()
        .flat_map(|machine| machine.sessions.iter())
        .find(|scanned| {
            scanned.session_id == want || connect_path_session_uuid(&scanned.session_path) == want
        })
        .map(|scanned| {
            let cwd = (!scanned.cwd.trim().is_empty()).then(|| scanned.cwd.clone());
            let title = (!scanned.title_hint.trim().is_empty()).then(|| scanned.title_hint.clone());
            (cwd, title)
        })
        .unwrap_or((None, None))
}

fn connect_session_is_active(snapshot: &yggterm_server::ServerUiSnapshot, path: &str) -> bool {
    let want = connect_path_session_uuid(path);
    snapshot
        .active_session_path
        .as_deref()
        .is_some_and(|active| active == path || connect_path_session_uuid(active) == want)
}

fn connect_session_key_is_known(snapshot: &yggterm_server::ServerUiSnapshot, path: &str) -> bool {
    let want = connect_path_session_uuid(path);
    connect_session_is_active(snapshot, path)
        || snapshot.live_sessions.iter().any(|session| {
            session.session_path == path || connect_path_session_uuid(&session.session_path) == want
        })
}

/// Where a freshly connected row lands in the Live Sessions order.
enum ConnectPlacement {
    /// Preserve the existing order; put the connected row last. Default: a
    /// connect must never rewrite an ordering the user arranged.
    End,
    /// Preserve the existing order; put the connected row directly after `anchor`.
    After(String),
    /// Daemon-native behavior: the row is prepended to the top.
    Top,
}

/// Restore `before` as the Live Sessions order, with `connected` placed per
/// `placement`. The daemon appends any live row we omit, so this can never drop
/// a row; rows in `before` that are no longer live simply resolve to nothing.
fn connect_desired_order(
    before: &[String],
    connected: &str,
    placement: &ConnectPlacement,
) -> Vec<String> {
    let want = connect_path_session_uuid(connected);
    let same =
        |candidate: &str| candidate == connected || connect_path_session_uuid(candidate) == want;
    // If the row was already live, leave it exactly where the user had it.
    if before.iter().any(|path| same(path)) {
        return before.to_vec();
    }
    let mut order = Vec::with_capacity(before.len() + 1);
    match placement {
        ConnectPlacement::Top => {
            order.push(connected.to_string());
            order.extend(before.iter().cloned());
        }
        ConnectPlacement::End => {
            order.extend(before.iter().cloned());
            order.push(connected.to_string());
        }
        ConnectPlacement::After(anchor) => {
            let anchor_uuid = connect_path_session_uuid(anchor);
            let mut placed = false;
            for path in before {
                order.push(path.clone());
                if !placed && (path == anchor || connect_path_session_uuid(path) == anchor_uuid) {
                    order.push(connected.to_string());
                    placed = true;
                }
            }
            if !placed {
                order.push(connected.to_string());
            }
        }
    }
    order
}

/// `yggterm server connect <session-path>`: connect an existing session into the
/// live set + GUI. Reuses the same daemon requests the GUI issues on a click.
fn run_server_connect(
    endpoint: &yggterm_server::ServerEndpoint,
    path: &str,
    view: WorkspaceViewMode,
    placement: ConnectPlacement,
) -> Result<()> {
    // Capture the row order BEFORE anything opens/focuses — both paths prepend a
    // newly-live row, so this is the only chance to know where the user's rows sat.
    let before_order: Vec<String> = snapshot(endpoint)?
        .0
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    // FocusLive reveals + resumes any session the daemon already tracks — even a
    // row the runtime-truth filter is currently suppressing, since launching its
    // runtime un-hides it — and is kind-agnostic (it uses the row the daemon
    // holds). FocusLive is a silent no-op on an unknown key, so a session that is
    // only in the remote scan falls through to the open path below.
    let (mut snapshot, mut message) = focus_live_with_view(endpoint, path, Some(view))?;
    if !connect_session_is_active(&snapshot, path) {
        // Mirror the GUI's `open_session_row` exactly (one source of truth): a
        // scanned CODEX row (remote-session://) goes through OpenRemoteSession;
        // everything else — notably a Claude Code row (remote-cc://), whose path
        // is not a remote-scanned path — is opened as a stored session carrying
        // its kind, id, cwd and title.
        let (cwd, title) = connect_scanned_metadata(&snapshot, path);
        let (opened, opened_message) =
            if let Some((machine_key, session_id)) = parse_remote_scanned_connect_path(path) {
                open_remote_session_with_view(
                    endpoint,
                    &machine_key,
                    &session_id,
                    cwd.as_deref(),
                    title.as_deref(),
                    Some(view),
                )?
            } else {
                open_stored_session_with_view(
                    endpoint,
                    connect_session_kind_for_path(path),
                    path,
                    Some(connect_path_session_uuid(path)),
                    cwd.as_deref(),
                    title.as_deref(),
                    Some(view),
                )?
            };
        snapshot = opened;
        message = opened_message;
    }
    let connected =
        connect_session_is_active(&snapshot, path) || connect_session_key_is_known(&snapshot, path);
    // Put the user's rows back where they were. The daemon prepended the new row;
    // unless the caller asked for --top, restore `before_order` and place the
    // connected row per `placement`. Reorder never drops a row (unlisted live
    // rows are appended), so this is safe even if the scan added rows meanwhile.
    let mut placed_at = "top";
    if connected && !matches!(placement, ConnectPlacement::Top) && !before_order.is_empty() {
        let desired = connect_desired_order(&before_order, path, &placement);
        let (reordered, _) = reorder_live_sessions_scoped(endpoint, &desired, None)?;
        snapshot = reordered;
        placed_at = match placement {
            ConnectPlacement::After(_) => "after_anchor",
            _ => "end",
        };
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "connected": connected,
            "requested_path": path,
            "active_session_path": snapshot.active_session_path,
            "view": match view {
                WorkspaceViewMode::Terminal => "terminal",
                WorkspaceViewMode::Rendered => "preview",
            },
            "row_placement": placed_at,
            "order_preserved": placed_at != "top",
            "live_session_count": snapshot.live_sessions.len(),
            "message": message,
        }))?
    );
    if !connected {
        anyhow::bail!(
            "could not connect {path}: not tracked as a live session and not found in remote scans (run `yggterm server connect --list` to see connectable sessions)"
        );
    }
    Ok(())
}

/// `yggterm server reorder <path>...`: set the Live Sessions row order. The
/// daemon places the listed rows first and appends every unlisted live row after
/// them (see `replace_live_session_order`), so a partial list only promotes the
/// rows you name and can never drop one. Dormant rows (no runtime) reorder like
/// any other row.
///
/// The report is the DAEMON's account, not an echo of the request: `applied` and
/// `skipped` come off the wire. The old output printed `requested: N` and the
/// caller's own list, which read as success even when the daemon had silently
/// dropped every row (field guide §4.5).
fn run_server_reorder(
    endpoint: &yggterm_server::ServerEndpoint,
    ordered_paths: &[String],
    client_scope: Option<&str>,
) -> Result<()> {
    let (before, _) = snapshot(endpoint)?;
    let before_order: Vec<String> = before
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    let (after, message) = reorder_live_sessions_scoped(endpoint, ordered_paths, client_scope)?;
    let after_order: Vec<String> = after
        .live_sessions
        .iter()
        .map(|session| session.session_path.clone())
        .collect();
    let update = message
        .as_deref()
        .and_then(yggterm_server::LiveSessionOrderUpdate::from_message);
    let mut report = serde_json::json!({
        "requested": ordered_paths,
        "live_session_count": after_order.len(),
        "changed": before_order != after_order,
        "order": after_order,
    });
    match &update {
        Some(update) => {
            report["applied"] = serde_json::json!(update.applied);
            report["skipped"] = serde_json::json!(update.skipped);
            report["message"] = serde_json::json!(update.summary());
        }
        // An older daemon cannot say what it applied. Report the gap rather
        // than inventing an `applied` list out of the request.
        None => {
            report["applied"] = serde_json::Value::Null;
            report["skipped"] = serde_json::Value::Null;
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
    Ok(())
}

/// `yggterm server connect --list`: enumerate sessions that EXIST (remote scans)
/// but are NOT currently in the live set — the connectable "void", newest first.
fn run_server_connect_list(endpoint: &yggterm_server::ServerEndpoint) -> Result<()> {
    let (snapshot, _) = snapshot(endpoint)?;
    let live_uuids: Vec<&str> = snapshot
        .live_sessions
        .iter()
        .map(|session| connect_path_session_uuid(&session.session_path))
        .collect();
    let mut connectable: Vec<&yggterm_server::RemoteScannedSession> = snapshot
        .remote_machines
        .iter()
        .flat_map(|machine| machine.sessions.iter())
        .filter(|scanned| !live_uuids.contains(&connect_path_session_uuid(&scanned.session_path)))
        .collect();
    // Newest first, so the sessions the user was most recently working with are
    // at the top of what can be a large scan (a busy host has hundreds).
    connectable.sort_by(|a, b| b.modified_epoch.cmp(&a.modified_epoch));
    let items: Vec<serde_json::Value> = connectable
        .iter()
        .map(|scanned| {
            serde_json::json!({
                "path": scanned.session_path,
                "title": scanned.title_hint,
                "cwd": scanned.cwd,
                "modified_epoch": scanned.modified_epoch,
                "live_runtime": scanned.live_runtime,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "connectable_count": items.len(),
            "live_session_count": snapshot.live_sessions.len(),
            "connectable": items,
        }))?
    );
    Ok(())
}

fn main() -> Result<()> {
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
    #[cfg(target_os = "linux")]
    if args.is_empty() {
        hydrate_linux_gui_entry_environment_from_desktop();
    }
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
    // `yggterm server connect <session-path>|--list` — headless twin of clicking
    // a session row. Manually connect an existing-but-unconnected ("void")
    // session back into the live set + GUI: it sends the SAME daemon requests as
    // the GUI (FocusLive for a session the daemon already tracks, else
    // OpenRemoteSession for a scan-only remote), so the session becomes live and
    // its terminal is attached/resumed. Recovery tool for sessions stranded out
    // of Live Sessions (e.g. demoted by a restart). See [[project-purpose]].
    if args.len() >= 2 && args[0] == "server" && args[1] == "connect" {
        let rest = &args[2..];
        let listing = rest.iter().any(|arg| arg == "--list" || arg == "-l");
        // Validate the invocation BEFORE touching the daemon: `ensure_local_...`
        // would otherwise spawn a daemon just to print a usage error.
        let path = if listing {
            None
        } else {
            Some(
                rest.iter()
                    .find(|arg| !arg.starts_with('-'))
                    .context("usage: yggterm server connect <session-path> | --list")?
                    .clone(),
            )
        };
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let Some(path) = path else {
            return run_server_connect_list(&endpoint);
        };
        let view = match cli_flag_value(&args, "--view") {
            Some("preview") | Some("rendered") => WorkspaceViewMode::Rendered,
            _ => WorkspaceViewMode::Terminal,
        };
        // Row placement. The daemon's open/focus path PREPENDS a newly-live row,
        // which silently rewrites the user's Live Sessions ordering on every
        // connect (live-caught: a 15-session batch buried a 28-row list). Default
        // to preserving the existing order and placing the row LAST; `--top`
        // restores the old prepend, `--after <path>` places it under an anchor.
        let placement = if args.iter().any(|arg| arg == "--top") {
            ConnectPlacement::Top
        } else if let Some(anchor) = cli_flag_value(&args, "--after") {
            ConnectPlacement::After(anchor.to_string())
        } else {
            ConnectPlacement::End
        };
        return run_server_connect(&endpoint, &path, view, placement);
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
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let verb = args.get(2).map(String::as_str).unwrap_or("");
        let profile = cli_flag_value(&args, "--profile");
        let pid = std::process::id();
        match verb {
            "acquire" | "hold" => {
                let status = yggterm_server::acquire_profile_write_lock(&endpoint, profile, pid)?;
                println!("{}", serde_json::to_string_pretty(&status)?);
                if verb == "hold" {
                    if !status.writable {
                        // Did not get the lock (a peer holds it): nothing to hold.
                        std::process::exit(1);
                    }
                    // Flush before parking: stdout is block-buffered when piped, so
                    // without this the JSON above never reaches a redirected log.
                    use std::io::Write as _;
                    let _ = std::io::stdout().flush();
                    eprintln!(
                        "holding profile write-lock {:?} as pid {} — SIGTERM/Ctrl-C to release",
                        status.profile, pid
                    );
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                }
                return Ok(());
            }
            "report" => {
                let status = yggterm_server::profile_write_lock_report(&endpoint)?;
                println!("{}", serde_json::to_string_pretty(&status)?);
                return Ok(());
            }
            "release" => {
                let status = yggterm_server::release_profile_write_lock(&endpoint, profile, pid)?;
                println!("{}", serde_json::to_string_pretty(&status)?);
                return Ok(());
            }
            other => anyhow::bail!(
                "usage: yggterm server write-lock <acquire|hold|report|release> \
                 [--profile <name>] (got {other:?})"
            ),
        }
    }
    // `yggterm server order [--json]` — dump the Live Sessions row order, one
    // path per line. Round-trips with `server reorder --stdin`, so an order can
    // always be captured before a disruptive operation and restored after:
    //   yggterm server order > order.txt
    //   yggterm server reorder --stdin < order.txt
    if args.len() >= 2 && args[0] == "server" && args[1] == "order" {
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let (snapshot, _) = snapshot(&endpoint)?;
        let order: Vec<String> = snapshot
            .live_sessions
            .iter()
            .map(|session| session.session_path.clone())
            .collect();
        if args.iter().any(|arg| arg == "--json") {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "live_session_count": order.len(),
                    "order": order,
                }))?
            );
        } else {
            for path in &order {
                println!("{path}");
            }
        }
        return Ok(());
    }
    // `yggterm server ledger [--scope <scope>]` — dump the durable row-order
    // ledger (per-client-scope memory of row slots, including rows that are
    // not currently live). Read-only.
    if args.len() >= 2 && args[0] == "server" && args[1] == "ledger" {
        let scope = args
            .iter()
            .position(|arg| arg == "--scope")
            .and_then(|ix| args.get(ix + 1))
            .map(String::as_str);
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        let report = row_order_ledger_report(&endpoint, scope)?;
        match serde_json::from_str::<serde_json::Value>(&report) {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
            Err(_) => println!("{report}"),
        }
        return Ok(());
    }
    // `yggterm server reorder <path>... | --stdin [--scope <scope>]` — set the
    // Live Sessions row order. Paths are placed in the given order at the TOP;
    // any live row not listed keeps its relative position AFTER them (the
    // daemon appends the remainder), so a partial list is safe and never drops
    // a row. `--scope` also records the order into that client's row-order
    // ledger scope (multi-GUI arrangements).
    if args.len() >= 2 && args[0] == "server" && args[1] == "reorder" {
        let scope = args
            .iter()
            .position(|arg| arg == "--scope")
            .and_then(|ix| args.get(ix + 1))
            .cloned();
        let scope_value_ix = args
            .iter()
            .position(|arg| arg == "--scope")
            .map(|ix| ix + 1);
        let ordered: Vec<String> = if args.iter().any(|arg| arg == "--stdin") {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading reorder stdin")?;
            buf.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        } else {
            args[2..]
                .iter()
                .enumerate()
                .filter(|(ix, arg)| !arg.starts_with('-') && Some(ix + 2) != scope_value_ix)
                .map(|(_, arg)| arg.clone())
                .collect()
        };
        if ordered.is_empty() {
            anyhow::bail!(
                "usage: yggterm server reorder <session-path>... | --stdin [--scope <scope>]"
            );
        }
        ensure_local_server_ready_for_cli(&store)?;
        let endpoint = cli_server_endpoint(store.home_dir());
        return run_server_reorder(&endpoint, &ordered, scope.as_deref());
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
        let endpoint = cli_server_endpoint(store.home_dir());
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
        // `--client <name>` (slice 4.3): route this verb to the GUI worker whose
        // daemon-client identity (`--client-id`) matches — how a probe names a
        // shadow view client. Resolved to a pid by `choose_app_control_pid`;
        // `--pid` still wins if both are given.
        let preferred_client = args.windows(2).find_map(|window| {
            if window[0] == "--client" && !window[1].starts_with("--") {
                Some(window[1].clone())
            } else {
                None
            }
        });
        if let Some(preferred_client) = preferred_client {
            unsafe {
                std::env::set_var("YGGTERM_APP_CONTROL_CLIENT", preferred_client);
            }
        } else {
            unsafe {
                std::env::remove_var("YGGTERM_APP_CONTROL_CLIENT");
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
                if compositor || parse_screenshot_post_process(&args).is_some() {
                    let post =
                        parse_screenshot_post_process(&args).unwrap_or(ScreenshotPostProcess {
                            region: None,
                            crop: None,
                            scale: 1.0,
                            grid: None,
                        });
                    run_screenshot_capture_with_post_process(
                        target,
                        output_path,
                        timeout_ms,
                        post,
                        compositor,
                    )
                } else {
                    run_screenshot_capture(target, output_path, timeout_ms)
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
            "launch" => {
                let wait_visible = args.iter().any(|arg| arg == "--wait-visible");
                let wait_settled = args.iter().any(|arg| arg == "--wait-settled");
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
                    wait_settled,
                    allow_multi_window,
                    skip_active_exec_handoff,
                    log_path,
                )
            }
            "clients" => run_app_control_list_clients(),
            "desktop-identity" => run_app_control_desktop_identity(),
            "state" => run_app_control_describe_state(timeout_ms),
            "dump" => {
                let output_path = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .context("missing output path for server app dump")?;
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
                    "layout" => {
                        let layout = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .unwrap_or("chat");
                        let layout = match layout {
                            "chat" => AppControlPreviewLayout::Chat,
                            "graph" | "overview" => AppControlPreviewLayout::Graph,
                            other => anyhow::bail!("unsupported app web view layout: {other}"),
                        };
                        run_app_control_set_preview_layout(layout, timeout_ms)
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
                    .context("missing --value for server app zoom")?;
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
                        args.iter().any(|arg| arg == "--force"),
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
            "update" => {
                let action = cli_positional_args(&args, 3)
                    .into_iter()
                    .next()
                    .unwrap_or("check");
                match action {
                    "check" | "trigger" => run_app_control_trigger_update_check(timeout_ms),
                    // N5: refuses while an agent holds a live web-surface
                    // lease — a deploy that lands mid-flow kills the flow.
                    // `--force` says you mean it.
                    "restart" => run_app_control_restart_pending_update(
                        args.iter().any(|arg| arg == "--force"),
                        timeout_ms,
                    ),
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
                    .context("missing session path for server app open")?;
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
            "keytips" => {
                let action = args.get(3).map(String::as_str).unwrap_or("audit");
                match action {
                    // The no-orphan-affordance audit (spec §12): walk the visible
                    // DOM interactables and report every one carrying neither
                    // `data-keytip-node` nor `data-keytip-exempt`. The definition of
                    // done for the ALT+ layer is `orphan_count: 0`.
                    "audit" => run_app_control_dom_eval(KEYTIPS_AUDIT_JS, timeout_ms),
                    other => anyhow::bail!("unsupported app keytips action: {other}"),
                }
            }
            // The `command invoke <id>` probe that `execute_shell_command`'s doc
            // comment has always promised. Both handlers were already plumbed
            // through to the shell (`AppControlCommand::InvokeCommand`); only the
            // CLI arm was missing, so the one dispatch every ALT+ KeyTip runs
            // through had no probe and keyboard-path fixes could not be exercised
            // live. Routing here means a probe drives the SAME terminus as the
            // chord, rather than a lookalike.
            "command" | "commands" => {
                let action = args.get(3).map(String::as_str).unwrap_or("list");
                match action {
                    "list" => yggterm_server::run_app_control_list_commands(timeout_ms),
                    "invoke" => {
                        let id = args
                            .get(4)
                            .map(String::as_str)
                            .context("missing command id for server app command invoke")?;
                        yggterm_server::run_app_control_invoke_command(id.to_string(), timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app command action: {other}"),
                }
            }
            "start-action" | "start" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing action for server app start-action")?;
                run_app_control_start_action(action, timeout_ms)
            }
            "start-page" | "show-start-page" | "home" => {
                yggterm_server::run_app_control_show_start_page(timeout_ms)
            }
            "tree" => {
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing action for server app tree")?;
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
                        let activate = !args.iter().any(|arg| arg == "--no-activate");
                        run_app_control_create_terminal(
                            machine_key,
                            cwd,
                            title_hint,
                            kind,
                            activate,
                            timeout_ms,
                        )
                    }
                    "send" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal send")?;
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
                                .context("missing --data or --stdin for server app terminal send")?
                                .to_string()
                        };
                        run_app_control_send_terminal_input(session_path, &data, timeout_ms)
                    }
                    "submit" => {
                        // Readiness-gated prompt insertion (waits for an idle prompt
                        // before sending; refuses if never ready).
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
                            .context("missing session path for server app terminal focus")?;
                        run_app_control_reclaim_terminal_focus(session_path, timeout_ms)
                    }
                    "redraw" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal redraw")?;
                        run_app_control_redraw_terminal(session_path, timeout_ms)
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
                    "scroll" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal scroll")?;
                        let to = cli_flag_value(&args, "--to").context(
                            "missing --to (top|bottom|±N lines) for server app terminal scroll",
                        )?;
                        run_app_control_scroll_terminal_viewport(session_path, to, timeout_ms)
                    }
                    "read-buffer" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal read-buffer")?;
                        let mode = cli_flag_value(&args, "--mode").unwrap_or("screen");
                        run_app_control_read_terminal_buffer(session_path, mode, timeout_ms)
                    }
                    "probe-select" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app terminal probe-select")?;
                        run_app_control_probe_terminal_viewport_select(session_path, timeout_ms)
                    }
                    "probe-primary-paste" | "probe-primary-selection-paste" => {
                        let session_path =
                            cli_positional_args(&args, 4).into_iter().next().context(
                                "missing session path for server app terminal probe-primary-paste",
                            )?;
                        let data = args
                            .windows(2)
                            .find_map(|window| {
                                if window[0] == "--data" {
                                    Some(window[1].as_str())
                                } else {
                                    None
                                }
                            })
                            .context(
                                "missing --data for server app terminal probe-primary-paste",
                            )?;
                        run_app_control_probe_terminal_primary_selection_paste(
                            session_path,
                            data,
                            timeout_ms,
                        )
                    }
                    "probe-context-menu" | "probe-right-click-menu" => {
                        let session_path =
                            cli_positional_args(&args, 4).into_iter().next().context(
                                "missing session path for server app terminal probe-context-menu",
                            )?;
                        run_app_control_probe_terminal_context_menu(session_path, timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app terminal action: {other}"),
                }
            }
            "web" => {
                // Web-surface (ychrome) automation: the agent is a first-class
                // user of pages. `--session <path>` targets a session's active
                // surface tab; omitted = the active session.
                let action = args
                    .get(3)
                    .map(String::as_str)
                    .context("missing action for server app web")?;
                let session_path = cli_flag_value(&args, "--session");
                match action {
                    "eval" => {
                        let script = if args.iter().any(|arg| arg == "--stdin") {
                            let mut value = String::new();
                            std::io::stdin()
                                .read_to_string(&mut value)
                                .context("reading app web eval stdin")?;
                            value
                        } else {
                            cli_flag_value(&args, "--script")
                                .map(str::to_string)
                                .or_else(|| {
                                    cli_positional_args(&args, 4)
                                        .into_iter()
                                        .next()
                                        .map(str::to_string)
                                })
                                .context("missing script (positional, --script or --stdin) for server app web eval")?
                        };
                        run_app_control_web_surface_eval(
                            session_path,
                            &script,
                            parse_web_frame_ref(&args)?,
                            timeout_ms,
                        )
                    }
                    "screenshot" => {
                        let output = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .unwrap_or("web-surface.png");
                        run_app_control_web_surface_screenshot(session_path, output, timeout_ms)
                    }
                    "await" => {
                        // The ONE async bridge:
                        //   web await (--script <file>|--stdin) [--await-timeout <ms>]
                        // The script is the BODY of an async function; `return`
                        // its value. `eval` cannot return a Promise, and this
                        // is the verb that means nobody has to hand-roll a
                        // stash-and-poll around that fact again.
                        let script = if args.iter().any(|arg| arg == "--stdin") {
                            let mut value = String::new();
                            std::io::stdin()
                                .read_to_string(&mut value)
                                .context("reading app web await stdin")?;
                            value
                        } else if let Some(path) = cli_flag_value(&args, "--script") {
                            // A FILE by default, unlike `eval`'s `--script`,
                            // because an async body is rarely a one-liner. A
                            // path that does not exist is read as the script
                            // itself rather than failing silently.
                            std::fs::read_to_string(path).unwrap_or_else(|_| path.to_string())
                        } else {
                            cli_positional_args(&args, 4)
                                .into_iter()
                                .next()
                                .map(str::to_string)
                                .context("missing script (positional, --script <file> or --stdin) for server app web await")?
                        };
                        let await_timeout_ms = cli_flag_value(&args, "--await-timeout")
                            .map(|raw| raw.parse::<u64>().context("--await-timeout needs ms"))
                            .transpose()?
                            .unwrap_or(15_000);
                        run_app_control_web_surface_await(session_path, &script, await_timeout_ms)
                    }
                    "frames" => {
                        // What frames this page has, and how much is IN each:
                        //   web frames [--session <path>]
                        // A top-document `read` returning [] next to a frame
                        // reporting 107 elements is a legible answer; the []
                        // alone was not.
                        run_app_control_web_surface_frames(session_path, timeout_ms)
                    }
                    "cookies" => {
                        // Move the surface's cookie jar to or from a Netscape
                        // file — the format `curl -c`/`-b` speaks:
                        //   web cookies --import <jar> | --export <jar>
                        // This is what makes a flow SPLITTABLE: script the
                        // mechanical parts on curl, hand the session to a
                        // surface for the one interactive step, hand it back.
                        //
                        // ⚠ The jar is per-PROFILE and an unqualified surface
                        // is `default` — the user's own browsing jar. Drive
                        // agent work on a `--profile agent-<n>` surface before
                        // importing. The response reports which profile was
                        // written; check it.
                        use yggterm_server::WebCookieDirection;
                        let (direction, jar) = match (
                            cli_flag_value(&args, "--import"),
                            cli_flag_value(&args, "--export"),
                        ) {
                            (Some(jar), None) => (WebCookieDirection::Import, jar),
                            (None, Some(jar)) => (WebCookieDirection::Export, jar),
                            (Some(_), Some(_)) => anyhow::bail!(
                                "web cookies takes --import <jar> OR --export <jar>, not both"
                            ),
                            (None, None) => anyhow::bail!(
                                "web cookies needs --import <jar> or --export <jar> (Netscape format)"
                            ),
                        };
                        run_app_control_web_surface_cookies(
                            session_path,
                            direction,
                            jar,
                            timeout_ms,
                        )
                    }
                    "capture-element" | "capture" => {
                        // Rasterize ONE addressed element to a PNG, in the page:
                        //   web capture-element <target> [out.png] [--split <n>]
                        // Compositor-independent, so it works on an unmapped
                        // surface — <img>/<canvas>/<video> only, and every other
                        // element gets a named refusal rather than a blank file.
                        let target = parse_web_element_ref(&args, true)?.context(
                            "capture-element needs a target: --selector <css>, --role <r> --label <s>, \
                             or --text <s>",
                        )?;
                        let output = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .unwrap_or("web-element.png");
                        let split = cli_flag_value(&args, "--split")
                            .map(|raw| raw.parse::<usize>().context("--split needs a number"))
                            .transpose()?;
                        run_app_control_web_surface_capture_element(
                            session_path,
                            target,
                            output,
                            split,
                            timeout_ms,
                        )
                    }
                    "devtools" => {
                        let open = !args.iter().any(|arg| arg == "--close");
                        run_app_control_web_surface_devtools(session_path, open, timeout_ms)
                    }
                    "fill" => {
                        let entry = cli_flag_value(&args, "--entry");
                        let user = cli_flag_value(&args, "--user");
                        run_app_control_web_surface_fill(session_path, entry, user, timeout_ms)
                    }
                    "fill-vault" | "fill-card" => {
                        // Type ONE named vault field into ONE addressed
                        // element, with real keys:
                        //   web fill-vault --item <name> --field password
                        //                  (--selector <css>|--role …|--target-text …)
                        //   web fill-card  --item <name> --field number …
                        // The secret never reaches this process: the CLI names
                        // the item and the field, the GUI reads and types it,
                        // and the answer is a length plus a boolean.
                        let source = if action == "fill-card" {
                            yggterm_server::VaultFieldSource::Card
                        } else {
                            yggterm_server::VaultFieldSource::Login
                        };
                        let target = parse_web_element_ref(&args, false)?.context(
                            "fill-vault needs a target: --selector <css>, --role <r> --label <s>, \
                             or --target-text <s>",
                        )?;
                        let item = cli_flag_value(&args, "--item")
                            .context("missing --item (the vault entry NAME) for web fill-vault")?;
                        let field =
                            cli_flag_value(&args, "--field").unwrap_or(if action == "fill-card" {
                                "number"
                            } else {
                                "password"
                            });
                        let user = cli_flag_value(&args, "--user");
                        let generation = cli_flag_value(&args, "--generation")
                            .map(|raw| raw.parse::<u64>().context("--generation needs a number"))
                            .transpose()?;
                        run_app_control_web_surface_fill_vault(
                            session_path,
                            target,
                            item,
                            field,
                            user,
                            source,
                            generation,
                            timeout_ms,
                        )
                    }
                    "totp" | "code" => {
                        let entry = cli_flag_value(&args, "--entry");
                        let user = cli_flag_value(&args, "--user");
                        run_app_control_web_surface_totp(session_path, entry, user, timeout_ms)
                    }
                    "do" => {
                        // Trusted action injection (agent control plane slice 2b):
                        //   web do click   --selector <css> | --x <n> --y <n> [--button …]
                        //   web do move    --x <n> --y <n>
                        //   web do scroll  [--x --y] --dx <n> --dy <n>
                        //   web do type    --text "…" [--selector <css>]
                        //   web do key     --key Enter [--mods ctrl,shift]
                        // `--generation <n>` pins the surface incarnation the
                        // verb was issued against: if the webview has been
                        // destroyed and rebuilt since (reload, profile/proxy
                        // change, hold expiry), the verb fails closed with
                        // `stale_handle` rather than acting on a page the
                        // caller never observed (F3). Every response reports
                        // the current `generation` to pin the next call with.
                        let action = parse_web_surface_do_action(&args)?;
                        let generation = cli_flag_value(&args, "--generation")
                            .map(|raw| raw.parse::<u64>().context("--generation needs a number"))
                            .transpose()?;
                        // `--new-batch` is the documented recovery from a
                        // `preempted` refusal: the agent asserts it re-observed
                        // the page, and the surface's batch lane is reopened.
                        let new_batch = args.iter().any(|arg| arg == "--new-batch");
                        run_app_control_web_surface_do(
                            session_path,
                            action,
                            generation,
                            new_batch,
                            timeout_ms,
                        )
                    }
                    "batch" => {
                        // One explicitly-opened agent batch, N verbs, one gate:
                        //   web batch --script <file> [--stop-on-error]
                        //             [--generation <n>] [--session <path>]
                        // Each line is a `do` invocation; the human still wins
                        // mid-batch (the GUI re-reads seat input between
                        // actions and aborts the remainder).
                        let script = if args.iter().any(|arg| arg == "--stdin") {
                            let mut value = String::new();
                            std::io::stdin()
                                .read_to_string(&mut value)
                                .context("reading app web batch stdin")?;
                            value
                        } else {
                            let path = cli_flag_value(&args, "--script").context(
                                "missing --script <file> (or --stdin) for server app web batch",
                            )?;
                            std::fs::read_to_string(path)
                                .with_context(|| format!("reading batch script {path}"))?
                        };
                        let actions = parse_web_do_batch_script(&script)?;
                        let generation = cli_flag_value(&args, "--generation")
                            .map(|raw| raw.parse::<u64>().context("--generation needs a number"))
                            .transpose()?;
                        let stop_on_error = args.iter().any(|arg| arg == "--stop-on-error");
                        run_app_control_web_surface_batch(
                            session_path,
                            actions,
                            generation,
                            stop_on_error,
                        )
                    }
                    "ensure" => {
                        // Headless surface-create: materialize a BACKGROUNDED
                        // session's declared web surfaces into the soft stash
                        // (never revealed) so agent verbs can drive them:
                        //   web ensure --session <path> [--ttl <secs>]
                        let ttl_secs = cli_flag_value(&args, "--ttl")
                            .map(|raw| raw.parse::<u64>().context("--ttl needs a number"))
                            .transpose()?;
                        let session = session_path
                            .context("web ensure needs --session <path> (a backgrounded surface has no active default)")?;
                        run_app_control_ensure_web_surface(session, ttl_secs, timeout_ms)
                    }
                    "reload" | "close" => {
                        // Recover a surface without destroying its session:
                        //   web reload --session <path>   (new incarnation)
                        //   web close  --session <path>
                        // Both report generation_before; compare it against a
                        // following `web ensure`'s generation_after to tell a
                        // HEALED surface from the same corpse.
                        let session =
                            session_path.context("web reload/close needs --session <path>")?;
                        if action == "reload" {
                            run_app_control_web_surface_reload(session, timeout_ms)
                        } else {
                            run_app_control_web_surface_close(session, timeout_ms)
                        }
                    }
                    "lease" => {
                        // Claim the surface so the background reaper leaves it
                        // alone while unattended work runs:
                        //   web lease --ttl <secs>   (0 releases the claim)
                        // The lease only ever EXTENDS the background hold — it
                        // cannot cut one short, so leasing can never destroy a
                        // surface the user is about to return to.
                        let ttl_secs = cli_flag_value(&args, "--ttl")
                            .context("missing --ttl for server app web lease")?
                            .parse::<u64>()
                            .context("--ttl needs a number of seconds")?;
                        run_app_control_web_surface_lease(session_path, ttl_secs, timeout_ms)
                    }
                    "read" => {
                        // Structured read-only observation (agent control plane
                        // slice 2b, rung 1):
                        //   web read [--as snapshot|forms|tables|readable|links|text|html]
                        let mode = match cli_flag_value(&args, "--as").unwrap_or("snapshot") {
                            "snapshot" | "interactable" | "tree" => {
                                yggterm_server::WebSurfaceReadAs::Snapshot
                            }
                            "forms" | "form" => yggterm_server::WebSurfaceReadAs::Forms,
                            "tables" | "table" => yggterm_server::WebSurfaceReadAs::Tables,
                            "readable" | "article" => yggterm_server::WebSurfaceReadAs::Readable,
                            "links" | "link" => yggterm_server::WebSurfaceReadAs::Links,
                            "text" => yggterm_server::WebSurfaceReadAs::Text,
                            "html" => yggterm_server::WebSurfaceReadAs::Html,
                            other => anyhow::bail!(
                                "unknown --as for web read: {other} (snapshot|forms|tables|readable|links|text|html)"
                            ),
                        };
                        run_app_control_web_surface_read(
                            session_path,
                            mode,
                            parse_web_frame_ref(&args)?,
                            timeout_ms,
                        )
                    }
                    "wait" => {
                        // Event-driven synchronization (agent control plane slice
                        // 2b, rung 2) — no more screenshot-poll loops:
                        //   web wait --until load:finished|load:committed|idle:<ms>
                        //                    |selector:<css> [--visible] |js:<expr>
                        //            [--wait-timeout <ms>]
                        use yggterm_server::WebSurfaceWaitUntil;
                        let raw = cli_flag_value(&args, "--until")
                            .context("missing --until for server app web wait")?;
                        let visible = args.iter().any(|arg| arg == "--visible");
                        let until = match raw.split_once(':') {
                            Some(("load", "committed")) => WebSurfaceWaitUntil::LoadCommitted,
                            Some(("load", "finished")) => WebSurfaceWaitUntil::LoadFinished,
                            Some(("idle", ms)) => WebSurfaceWaitUntil::Idle {
                                ms: ms.parse().context("--until idle:<ms> needs a number")?,
                            },
                            Some(("selector", css)) => WebSurfaceWaitUntil::Selector {
                                css: css.to_string(),
                                visible,
                            },
                            Some(("js", expr)) => WebSurfaceWaitUntil::Js {
                                expr: expr.to_string(),
                            },
                            // `url:matches:<re>` and its sugar `url:contains:<s>`
                            // compile to ONE predicate: the sugar is escaped
                            // into a regex here rather than becoming a second
                            // matching rule in the GUI.
                            Some(("url", rest)) => match rest.split_once(':') {
                                Some(("matches", pattern)) => WebSurfaceWaitUntil::UrlMatches {
                                    pattern: pattern.to_string(),
                                },
                                Some(("contains", needle)) => WebSurfaceWaitUntil::UrlMatches {
                                    pattern: regex_escape_literal(needle),
                                },
                                _ => anyhow::bail!(
                                    "bad --until url:… ({rest}) — use url:matches:<regex> or url:contains:<substring>"
                                ),
                            },
                            Some(("settled", ms)) => WebSurfaceWaitUntil::Settled {
                                ms: ms.parse().context("--until settled:<ms> needs a number")?,
                            },
                            _ => match raw {
                                "committed" => WebSurfaceWaitUntil::LoadCommitted,
                                "finished" | "load" | "loaded" => WebSurfaceWaitUntil::LoadFinished,
                                other => anyhow::bail!(
                                    "bad --until: {other} (load:committed|load:finished|idle:<ms>|settled:<ms>|selector:<css>|js:<expr>|url:matches:<re>|url:contains:<s>)"
                                ),
                            },
                        };
                        let wait_timeout_ms = cli_flag_value(&args, "--wait-timeout")
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(10_000);
                        run_app_control_web_surface_wait(session_path, until, wait_timeout_ms)
                    }
                    other => anyhow::bail!("unsupported app web action: {other}"),
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
                    "rename" => {
                        let positionals = cli_positional_args(&args, 4);
                        let session_path = positionals
                            .first()
                            .copied()
                            .context("missing session path for server app session rename")?;
                        let title = positionals
                            .get(1)
                            .copied()
                            .context("missing title for server app session rename")?;
                        run_app_control_rename_session(session_path, title, timeout_ms)
                    }
                    "restart" => {
                        let session_path = cli_positional_args(&args, 4)
                            .into_iter()
                            .next()
                            .context("missing session path for server app session restart")?;
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
    if input.kde_session && input.wayland_display_present {
        return LinuxWindowProfile {
            transparent: true,
            xrpd_session: false,
            reason: "kde_wayland_transparent_profile",
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

/// Whether under-glass web-surface stacking should be armed, from the two
/// environment knobs. DEFAULT ON (F.1 tail, 2026-07-19): unset ⇒ armed.
/// Explicit opt-outs: `YGGTERM_WEB_SURFACE_UNDER_GLASS=0` or the legacy
/// force `YGGTERM_WEB_SURFACE_LEGACY_STACK=1`. Structural safety stays
/// runtime-side — the vendored host's self-probe demotes to legacy
/// stacking on engines/paths that cannot composite (SHM, webkit < 2.40),
/// so the default costs nothing where under-glass is impossible.
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
    // ⛔ HARDWARE GL DOES NOT IMPLY ARMING PHASE F. It used to: the default was
    // `true` for any host that was not on software GL, so probing a working GPU
    // silently turned under-glass on for the first time in production — and the
    // user's whole window became an agent's background page.
    //
    // Under glass the shell webview is TRANSPARENT by construction and the
    // surfaces composite behind it, so anything that stops the shell painting
    // stops being "a blank window" and starts being "whatever page happens to
    // be back there, full screen". On the live host that happened while memory
    // was exhausted (swap 100% full, the GUI too starved to answer a state
    // probe for 25 s) — exactly when the shell is least likely to paint. Under
    // the legacy stack the same starvation is invisible, because the shell is
    // opaque.
    //
    // The two decisions were bundled for a reason that no longer holds. Under
    // glass genuinely REQUIRES DMABuf (the refusal above), but the reverse was
    // never true, and `shm_force_for_arming` now refuses SHM on a probed
    // hardware host whatever arming decides. So hardware GL keeps DMABuf and
    // keeps the measured GL benefit with under-glass OFF; Phase F is a separate
    // feature and now needs its own explicit opt-in, which is what the user
    // asked for when they said hardware GL should come back without this.
    under_glass_var == Some("1")
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

fn app_control_close_preserve_flag(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--preserve-live-sessions" | "--preserve-sessions" | "--handoff" | "--restart-safe"
        )
    })
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
mod web_usage_tests {
    use super::*;

    /// Every action name the `server app web` dispatcher accepts, read from the
    /// dispatcher's own match arms.
    ///
    /// This is a SCANNER, and a scanner that silently matches nothing passes
    /// green while proving nothing — the exact failure a brace-counting lock in
    /// this repo shipped with. So it returns the arms AND the test asserts a
    /// coverage floor below.
    fn dispatcher_web_actions() -> Vec<String> {
        let source = include_str!("main.rs");
        let start = source
            .find("            \"web\" => {")
            .expect("the web dispatcher block moved; fix this scanner, do not delete it");
        let end = source[start..]
            .find("other => anyhow::bail!(\"unsupported app web action")
            .expect("the web dispatcher's catch-all moved")
            + start;
        let mut actions = Vec::new();
        // The OUTER match's arms sit at exactly this indentation. Matching on
        // indentation is what keeps the INNER matches out of the answer — the
        // `--as` modes and `--until` forms are options of a verb, not verbs,
        // and counting them would make this lock unsatisfiable and then
        // deleted.
        const ARM_INDENT: &str = "                    \"";
        for line in source[start..end].lines() {
            if !line.starts_with(ARM_INDENT) {
                continue;
            }
            let trimmed = line.trim();
            // An arm looks like:  "eval" => {   or   "totp" | "code" => {
            let Some(head) = trimmed.split("=>").next() else {
                continue;
            };
            if !trimmed.contains("=>") || !head.trim_start().starts_with('"') {
                continue;
            }
            for part in head.split('|') {
                let part = part.trim();
                if let Some(name) = part
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                {
                    actions.push(name.to_string());
                }
            }
        }
        actions
    }

    /// THE DRIFT LOCK. `ensure`, `fill` and `totp` were implemented and
    /// undocumented, and a stale usage block is what produced a "not deployed"
    /// misdiagnosis in the field — an agent read `--help`, did not see the
    /// verb, and concluded the build lacked it.
    ///
    /// Fails today against the pre-D1 usage string, which is the point.
    #[test]
    fn every_web_action_appears_in_the_usage_string() {
        let dispatcher = dispatcher_web_actions();
        // COVERAGE FLOOR: a scanner that finds nothing must fail, not pass.
        assert!(
            dispatcher.len() >= 15,
            "the arm scanner found only {} actions — it went blind; fix it rather than \
             lowering this floor",
            dispatcher.len()
        );
        assert!(
            dispatcher.contains(&"eval".to_string()),
            "sanity: {dispatcher:?}"
        );

        let documented: std::collections::BTreeSet<&str> =
            WEB_ACTIONS.iter().map(|(name, _)| *name).collect();
        let implemented: std::collections::BTreeSet<String> = dispatcher.iter().cloned().collect();

        let undocumented: Vec<&String> = implemented
            .iter()
            .filter(|name| !documented.contains(name.as_str()))
            .collect();
        assert!(
            undocumented.is_empty(),
            "these web actions are implemented and undocumented: {undocumented:?} — add them to \
             WEB_ACTIONS. An agent that reads --help and does not see a verb concludes the build \
             lacks it."
        );

        let phantom: Vec<&&str> = documented
            .iter()
            .filter(|name| !implemented.contains(&(**name).to_string()))
            .collect();
        assert!(
            phantom.is_empty(),
            "these web actions are documented and NOT implemented: {phantom:?} — worse than an \
             omission, because it sends a caller after a verb that will never answer."
        );
    }

    /// The rendered block must actually contain a line per non-alias action —
    /// a `WEB_ACTIONS` entry with an empty usage string would satisfy the set
    /// comparison above while printing nothing.
    #[test]
    fn the_rendered_usage_names_every_non_alias_action() {
        let rendered = web_usage_block();
        for (name, usage) in WEB_ACTIONS {
            if usage.is_empty() {
                // An alias: it must still be findable in its primary's line.
                assert!(
                    rendered.contains(name),
                    "alias {name} is documented nowhere"
                );
                continue;
            }
            assert!(
                rendered.contains(&format!("server app web {name} ")),
                "{name} has a usage entry that does not name it"
            );
        }
        // And the whole block is non-trivial.
        assert!(rendered.lines().count() > 25, "the usage block collapsed");
    }
}

#[cfg(test)]
mod tests {
    use super::KEYTIPS_AUDIT_JS;
    #[cfg(unix)]
    use super::superseded_client_termination_signal;
    use super::{
        BuiltinCliCommand, FILE_DESCRIPTOR_SOFT_LIMIT_TARGET, LinuxWindowProfileInput,
        SignalClientScope, app_control_launch_state_timeout_ms,
        app_control_state_settled_for_launch, classify_builtin_cli_command,
        compatible_signal_client_count, linux_window_profile_from_input,
        main_should_retire_superseded_clients_before_shell, raised_file_descriptor_soft_limit,
        record_matches_executable, should_handoff_to_preferred_executable,
        should_retire_superseded_client, signal_client_instances_dir, signal_client_scope_matches,
        signal_parse_process_start_ticks_from_stat, signal_process_start_ticks,
        signal_shutdown_policy_allows_daemon_shutdown, superseded_client_close_command,
        superseded_client_retirement_strategy_label, under_glass_default_armed,
    };

    // §12.1: a control whose badge marker is a child span is REACHABLE, whatever
    // encloses it. If the exempt-ancestor test ran first, wiring a control inside
    // a panel that still carries a subtree exemption would report it as a
    // violation — the metric could not show its own fix. Measured 2026-07-22: the
    // already-wired settings/theme.light|dark buttons were both counted as
    // `suppressed_by: settings-panel`. The ORDER is the invariant, so assert the
    // order rather than the mere presence of the checks.
    #[test]
    fn audit_counts_a_declared_descendant_before_testing_for_an_exempt_ancestor() {
        let declared_descendant = KEYTIPS_AUDIT_JS
            .find("if (el.querySelector('[data-keytip-node]')) { reachable++; return; }")
            .expect("audit must credit a control that contains a declared marker");
        let exempt_ancestor = KEYTIPS_AUDIT_JS
            .find("el.closest('[data-keytip-exempt]')")
            .expect("audit must still detect subtree suppression");
        assert!(
            declared_descendant < exempt_ancestor,
            "the declared-descendant credit must precede the exempt-ancestor test, \
             else a wired control inside an exempt subtree reads as a violation"
        );
    }

    // ⛔ UNDER-GLASS IS OPT-IN, and this test used to assert the opposite.
    //
    // It was default-on for any host not on software GL, so the moment the GL
    // probe found a working GPU, Phase F armed in production for the first time
    // — and the user's entire window became an agent's background page. Under
    // glass the shell is transparent by construction, so a shell that fails to
    // paint (memory was exhausted: swap 100% full, the GUI too starved to
    // answer a state probe for 25 s) stops showing a blank window and starts
    // showing whatever surface is behind it, full screen.
    //
    // Arming now requires saying so. The GL benefit does not depend on it:
    // `shm_force_for_arming` refuses SHM on a probed hardware host whatever
    // arming decides, so hardware GL keeps DMABuf either way.
    #[test]
    fn under_glass_arms_only_when_asked() {
        // Hardware GL is NOT consent: an unset var must not arm Phase F.
        assert!(
            !under_glass_default_armed(None, None, false),
            "hardware GL must not silently arm under-glass — that is how a \
             background page took over the whole window"
        );
        assert!(under_glass_default_armed(Some("1"), None, false));
        assert!(!under_glass_default_armed(Some("0"), None, false));
        // The legacy force is the escape hatch of last resort and beats an
        // explicit opt-in.
        assert!(!under_glass_default_armed(None, Some("1"), false));
        assert!(!under_glass_default_armed(Some("1"), Some("1"), false));
        // Legacy explicitly off is still not an opt-IN.
        assert!(!under_glass_default_armed(None, Some("0"), false));
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
                                                // ...but arming does NOT follow it. A
                                                // working GPU is not consent to turn on
                                                // under-glass compositing; that pairing
                                                // is what put an agent's page over the
                                                // user's whole window.
                                                assert!(
                                                    !armed,
                                                    "under-glass must stay unarmed without an \
                                             explicit request ({context})"
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
        let source = include_str!("main.rs");
        assert!(source.contains("server app open <session-path>"));
        assert!(source.contains("\"open\" =>"));
        assert!(source.contains("run_app_control_open_path(session_path, view_mode, timeout_ms)"));
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
