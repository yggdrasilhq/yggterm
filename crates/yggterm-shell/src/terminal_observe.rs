use serde_json::{Value, json};
use yggterm_server::WorkspaceViewMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalOpenAttemptState {
    Pending,
    Recovering,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalOpenAttempt {
    pub(crate) attempt_id: String,
    pub(crate) session_path: String,
    pub(crate) request_id: String,
    pub(crate) open_request_id: u64,
    pub(crate) source: String,
    pub(crate) started_at_ms: u64,
    pub(crate) state: TerminalOpenAttemptState,
    pub(crate) observations: u64,
    pub(crate) ready_at_ms: Option<u64>,
    pub(crate) latched_failure_at_ms: Option<u64>,
    pub(crate) latched_failure_reason: Option<String>,
    pub(crate) last_observed_ready: bool,
    pub(crate) last_observed_reason: Option<String>,
    pub(crate) last_surface_problem: Option<String>,
    pub(crate) last_overlay_visible: bool,
    pub(crate) last_overlay_kind: Option<String>,
    pub(crate) last_overlay_text: Option<String>,
}

pub(crate) fn describe_viewport_snapshot(snapshot: &Value, dom: &Value) -> Value {
    let active_session_path = snapshot
        .get("active_session_path")
        .and_then(Value::as_str)
        .map(str::to_string);
    let active_view_mode = snapshot
        .get("active_view_mode")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let active_title = snapshot
        .get("active_title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let active_summary = snapshot
        .get("active_summary")
        .and_then(Value::as_str)
        .map(str::to_string);
    let titlebar_title_text = dom
        .get("titlebar_title_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let titlebar_summary_text = dom
        .get("titlebar_summary_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let titlebar_button_tooltip = dom
        .get("titlebar_button_tooltip")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let titlebar_menu_open = dom
        .get("titlebar_menu_open")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let terminal_attach_in_flight = snapshot
        .get("shell")
        .and_then(|shell| shell.get("terminal_attach_in_flight"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let notifications = snapshot
        .get("shell")
        .and_then(|shell| shell.get("notifications"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let notification_count = notifications.len();
    let preview_text_sample = dom
        .get("preview_text_sample")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let preview_viewport_rect = dom
        .get("preview_viewport_rect")
        .cloned()
        .unwrap_or(Value::Null);
    let preview_visible_block_ids = dom
        .get("preview_visible_block_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let preview_font_family = dom
        .get("preview_font_family")
        .and_then(Value::as_str)
        .map(str::to_string);
    let preview_visible_entries = dom
        .get("preview_visible_entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let preview_rendered_sections = dom
        .get("preview_rendered_sections")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let preview_fallback_context_visible = dom
        .get("preview_fallback_context_visible")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let preview_fallback_context_text = dom
        .get("preview_fallback_context_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let preview_timestamp_labels = dom
        .get("preview_timestamp_labels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let preview_window = dom.get("preview_window").cloned().unwrap_or(Value::Null);
    let shell_text_sample = dom
        .get("shell_text_sample")
        .and_then(Value::as_str)
        .unwrap_or("");
    let document_editor_count = dom
        .get("document_editor_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let document_body_sample = dom
        .get("document_body_sample")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let terminal_hosts = dom
        .get("terminal_hosts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let active_terminal_hosts = active_session_path
        .as_deref()
        .map(|path| {
            terminal_hosts
                .iter()
                .filter(|host| {
                    host.get("session_path")
                        .and_then(Value::as_str)
                        .is_some_and(|session_path| session_path == path)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let dom_terminal_resume_overlay =
        dom.get("terminal_resume_overlay")
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "visible": false,
                    "text_sample": "",
                    "excerpt": "",
                    "kind": "",
                    "phase": "hidden",
                    "effective_failed": false
                })
            });
    let terminal_resume_overlay = if dom_terminal_resume_overlay
        .get("visible")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        dom_terminal_resume_overlay
    } else {
        active_terminal_hosts
            .iter()
            .find(|host| {
                host.get("resume_overlay_visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .map(|host| {
                json!({
                    "visible": true,
                    "text_sample": host.get("resume_overlay_text").and_then(Value::as_str).unwrap_or(""),
                    "excerpt": host.get("resume_overlay_excerpt").and_then(Value::as_str).unwrap_or(""),
                    "kind": host.get("resume_overlay_kind").and_then(Value::as_str).unwrap_or(""),
                    "phase": host.get("resume_overlay_phase").and_then(Value::as_str).unwrap_or(""),
                    "effective_failed": host.get("resume_overlay_effective_failed").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .unwrap_or_else(|| {
                json!({
                    "visible": false,
                    "text_sample": "",
                    "excerpt": "",
                    "kind": "",
                    "phase": "hidden",
                    "effective_failed": false
                })
            })
    };
    let terminal_resume_overlay_phase = terminal_resume_overlay
        .get("phase")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            terminal_resume_overlay
                .get("kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("hidden")
                .to_string()
        });
    let terminal_resume_overlay_effective_failed = terminal_resume_overlay
        .get("effective_failed")
        .and_then(Value::as_bool)
        .unwrap_or(terminal_resume_overlay_phase == "failure");
    let terminal_resume_overlay = json!({
        "visible": terminal_resume_overlay.get("visible").and_then(Value::as_bool).unwrap_or(false),
        "text_sample": terminal_resume_overlay.get("text_sample").and_then(Value::as_str).unwrap_or(""),
        "excerpt": terminal_resume_overlay.get("excerpt").and_then(Value::as_str).unwrap_or(""),
        "kind": terminal_resume_overlay.get("kind").and_then(Value::as_str).unwrap_or(""),
        "phase": terminal_resume_overlay_phase,
        "effective_failed": terminal_resume_overlay_effective_failed,
    });
    let preview_visible_block_count = dom
        .get("preview_visible_block_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let preview_placeholder = preview_text_looks_like_loading_placeholder(&preview_text_sample);
    let overlay_context_visible = false;
    let active_terminal_surface =
        summarize_terminal_surface_for_app_control(&active_terminal_hosts, overlay_context_visible);
    let terminal_rendered = active_terminal_surface
        .get("rendered")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let terminal_surface_problem = active_terminal_surface
        .get("problem")
        .and_then(Value::as_str)
        .map(str::to_string);
    let terminal_attach_pending = active_session_path.as_deref().is_some_and(|path| {
        terminal_attach_in_flight.iter().any(|candidate| {
            candidate
                .as_str()
                .is_some_and(|candidate_path| candidate_path == path)
        })
    });
    let preview_loading = snapshot
        .get("active_surface_requests")
        .and_then(Value::as_array)
        .is_some_and(|requests| {
            requests.iter().any(|request| {
                request.get("surface").and_then(Value::as_str) == Some("Preview")
                    && match (
                        request
                            .get("target")
                            .and_then(|target| target.get("kind"))
                            .and_then(Value::as_str),
                        request
                            .get("target")
                            .and_then(|target| target.get("session_path"))
                            .and_then(Value::as_str),
                        active_session_path.as_deref(),
                    ) {
                        (Some("active_session"), _, Some(_)) => true,
                        (
                            Some("session" | "terminal" | "preview"),
                            Some(session_path),
                            Some(active_path),
                        ) => session_path == active_path,
                        _ => false,
                    }
            })
        })
        || shell_text_sample.contains("Refreshing preview…")
        || shell_text_sample.contains("Refreshing preview...");
    let (ready, interactive, settled_kind, reason) = if active_session_path.is_none() {
        (
            false,
            false,
            None::<String>,
            Some("no active session selected".to_string()),
        )
    } else if active_view_mode == "Rendered" {
        let preview_scroll_count = dom
            .get("preview_scroll_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let preview_has_content = preview_scroll_count > 0
            && ((preview_visible_block_count > 0 || !preview_rendered_sections.is_empty())
                || (!preview_text_sample.is_empty() && !preview_placeholder));
        if preview_loading && !preview_has_content {
            (
                false,
                false,
                None::<String>,
                Some("preview still loading".to_string()),
            )
        } else if preview_placeholder {
            (
                false,
                false,
                None::<String>,
                Some("preview placeholder is still visible".to_string()),
            )
        } else if preview_scroll_count == 0 && document_editor_count == 0 {
            (
                false,
                false,
                None::<String>,
                Some("preview surface not mounted".to_string()),
            )
        } else if preview_scroll_count > 0
            && preview_text_sample.is_empty()
            && preview_visible_block_count == 0
        {
            (
                false,
                false,
                None::<String>,
                Some("preview surface mounted but content is empty".to_string()),
            )
        } else if document_editor_count > 0 && document_body_sample.is_empty() {
            (
                false,
                false,
                None::<String>,
                Some("document editor mounted but body is empty".to_string()),
            )
        } else {
            (true, true, Some("preview".to_string()), None)
        }
    } else if active_view_mode == "Terminal" {
        let overlay_phase = terminal_resume_overlay
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("");
        if active_terminal_hosts.is_empty() {
            (
                false,
                false,
                None::<String>,
                Some("active terminal host is missing".to_string()),
            )
        } else if terminal_resume_overlay
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if overlay_phase == "failure" {
                (
                    false,
                    false,
                    None::<String>,
                    Some("terminal resume failure overlay is still visible".to_string()),
                )
            } else {
                (
                    false,
                    false,
                    Some("recovering".to_string()),
                    Some("terminal resume overlay is still visible".to_string()),
                )
            }
        } else if let Some(problem) = terminal_surface_problem.clone() {
            (false, false, None::<String>, Some(problem))
        } else if terminal_rendered {
            (true, true, Some("interactive".to_string()), None)
        } else if terminal_attach_pending {
            (
                false,
                false,
                None::<String>,
                Some("active terminal host exists but attach is still in flight".to_string()),
            )
        } else if terminal_hosts.is_empty() {
            (
                false,
                false,
                None::<String>,
                Some("terminal host is missing".to_string()),
            )
        } else {
            (
                false,
                false,
                None::<String>,
                Some("active terminal host exists but xterm surface is empty".to_string()),
            )
        }
    } else {
        (
            false,
            false,
            None::<String>,
            Some(format!("unsupported active view mode: {active_view_mode}")),
        )
    };
    json!({
        "active_session_path": active_session_path,
        "active_view_mode": active_view_mode,
        "active_title": active_title,
        "active_summary": active_summary,
        "titlebar": {
            "title_text": titlebar_title_text,
            "summary_text": titlebar_summary_text,
            "button_tooltip": titlebar_button_tooltip,
            "menu_open": titlebar_menu_open,
        },
        "notification_count": notification_count,
        "notifications": notifications,
        "preview": {
            "text_sample": preview_text_sample,
            "placeholder": preview_placeholder,
            "viewport_rect": preview_viewport_rect,
            "visible_block_count": preview_visible_block_count,
            "visible_block_ids": preview_visible_block_ids,
            "visible_entries": preview_visible_entries,
            "rendered_sections": preview_rendered_sections,
            "fallback_context_visible": preview_fallback_context_visible,
            "fallback_context_text": preview_fallback_context_text,
            "font_family": preview_font_family,
            "timestamp_labels": preview_timestamp_labels,
            "window": preview_window,
        },
        "document_editor_count": document_editor_count,
        "document_body_sample": document_body_sample,
        "terminal_host_count": terminal_hosts.len(),
        "terminal_hosts": terminal_hosts,
        "active_terminal_host_count": active_terminal_hosts.len(),
        "active_terminal_hosts": active_terminal_hosts,
        "active_terminal_surface": active_terminal_surface,
        "terminal_resume_overlay": terminal_resume_overlay,
        "ready": ready,
        "interactive": interactive,
        "terminal_settled_kind": settled_kind,
        "reason": reason,
    })
}

pub(crate) fn preview_text_looks_like_loading_placeholder(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    normalized.contains("refreshing preview")
        || normalized.contains("loading remote session")
        || normalized.contains("loading remote preview")
        || normalized.contains("loading remote paper")
        || normalized.contains("loading remote recipe")
        || normalized.contains("fetching rendered transcript")
        || normalized.contains("preparing the remote preview surface")
        || normalized.contains("waiting for transcript hydration")
        || normalized.contains("preview unavailable")
        || normalized.contains("could not load this remote")
}

pub(crate) fn terminal_open_attempt_state_label(state: &TerminalOpenAttemptState) -> &'static str {
    match state {
        TerminalOpenAttemptState::Pending => "pending",
        TerminalOpenAttemptState::Recovering => "recovering",
        TerminalOpenAttemptState::Ready => "ready",
        TerminalOpenAttemptState::Failed => "failed",
    }
}

pub(crate) fn describe_terminal_open_attempt(attempt: &TerminalOpenAttempt) -> Value {
    json!({
        "attempt_id": attempt.attempt_id,
        "session_path": attempt.session_path,
        "request_id": attempt.request_id,
        "open_request_id": attempt.open_request_id,
        "source": attempt.source,
        "started_at_ms": attempt.started_at_ms,
        "state": terminal_open_attempt_state_label(&attempt.state),
        "observations": attempt.observations,
        "ready_at_ms": attempt.ready_at_ms,
        "latched_failure_at_ms": attempt.latched_failure_at_ms,
        "latched_failure_reason": attempt.latched_failure_reason,
        "last_observed_ready": attempt.last_observed_ready,
        "last_observed_reason": attempt.last_observed_reason,
        "last_surface_problem": attempt.last_surface_problem,
        "last_overlay_visible": attempt.last_overlay_visible,
        "last_overlay_kind": attempt.last_overlay_kind,
        "last_overlay_text": attempt.last_overlay_text,
    })
}

pub(crate) fn terminal_open_attempt_failure_reason_from_viewport(
    viewport: &Value,
) -> Option<String> {
    if viewport.get("interactive").and_then(Value::as_bool) == Some(true)
        && viewport
            .get("terminal_settled_kind")
            .and_then(Value::as_str)
            == Some("interactive")
    {
        return None;
    }
    if viewport
        .get("terminal_settled_kind")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "overlay_context" || value == "recovering")
    {
        return None;
    }
    if let Some(problem) = viewport
        .get("active_terminal_surface")
        .and_then(|value| value.get("problem"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if terminal_open_attempt_problem_is_fatal(problem) {
            return Some(problem.to_string());
        }
    }
    let overlay = viewport
        .get("terminal_resume_overlay")
        .and_then(Value::as_object)?;
    let visible = overlay
        .get("visible")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !visible {
        return None;
    }
    let phase = overlay
        .get("phase")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            overlay
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
        })
        .to_ascii_lowercase();
    let effective_failed = overlay
        .get("effective_failed")
        .and_then(Value::as_bool)
        .unwrap_or(phase == "failure");
    if phase != "failure" || !effective_failed {
        return None;
    }
    let kind = overlay
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let text = overlay
        .get("text_sample")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let normalized = text.to_ascii_lowercase();
    if kind == "failure"
        || normalized.contains("remote host is unavailable")
        || normalized.contains("still not interactive")
        || normalized.contains("has not become interactive")
        || normalized.contains("needs attention")
    {
        return Some(if text.is_empty() {
            "terminal resume failure overlay is still visible".to_string()
        } else {
            text.to_string()
        });
    }
    None
}

fn terminal_open_attempt_problem_is_fatal(problem: &str) -> bool {
    let normalized = problem.trim().to_ascii_lowercase();
    normalized.contains("transport/error output")
        || normalized.contains("session is unavailable")
        || normalized.contains("saved remote session is unavailable")
        || normalized.contains("remote host is unavailable")
}

pub(crate) fn terminal_bootstrap_should_wait_for_mount_epoch_sync(
    active_view_mode: WorkspaceViewMode,
    active_session_path: Option<&str>,
    session_path: &str,
    mount_epoch: u64,
    latest_open_request_id: u64,
) -> bool {
    active_view_mode == WorkspaceViewMode::Terminal
        && active_session_path == Some(session_path)
        && mount_epoch == 0
        && latest_open_request_id == 0
}

pub(crate) fn summarize_terminal_surface_for_app_control(
    hosts: &[Value],
    overlay_context_visible: bool,
) -> Value {
    let geometry_problem = hosts
        .iter()
        .find_map(terminal_host_geometry_problem_for_app_control)
        .map(str::to_string);
    let rendered = hosts
        .iter()
        .any(terminal_host_has_rendered_surface_for_app_control);
    let live_problem = hosts
        .iter()
        .find_map(terminal_host_problem_for_app_control)
        .map(str::to_string);
    let problem = if overlay_context_visible {
        None::<String>
    } else {
        geometry_problem.clone().or(live_problem.clone())
    };
    json!({
        "rendered": rendered,
        "problem": problem,
        "geometry_problem": geometry_problem,
        "live_problem": live_problem,
        "overlay_context_visible": overlay_context_visible,
    })
}

fn terminal_host_has_rendered_surface_for_app_control(host: &Value) -> bool {
    let child_count = host.get("child_count").and_then(Value::as_u64).unwrap_or(0);
    let xterm_present = host
        .get("xterm_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let screen_present = host
        .get("screen_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let viewport_present = host
        .get("viewport_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rows_present = host
        .get("rows_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let canvas_count = host
        .get("canvas_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let text_sample = host
        .get("text_sample")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    (child_count > 0 || xterm_present || screen_present || viewport_present || rows_present)
        && (canvas_count > 0 || !text_sample.is_empty())
}

fn terminal_host_problem_for_app_control(host: &Value) -> Option<&'static str> {
    if let Some(problem) = terminal_host_geometry_problem_for_app_control(host) {
        return Some(problem);
    }
    let text_sample = host
        .get("text_sample")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if text_sample.is_empty() {
        return None;
    }
    if terminal_chunk_is_transport_error(text_sample) {
        return Some("active terminal host is showing transport/error output");
    }
    if terminal_chunk_is_loading_placeholder(text_sample) {
        return Some("active terminal host is still showing resume placeholder content");
    }
    if terminal_chunk_is_generic_codex_idle(text_sample) {
        return Some("active terminal host is still showing generic Codex idle chrome");
    }
    if terminal_chunk_has_generic_codex_idle_footer(text_sample)
        && !terminal_chunk_has_meaningful_output(text_sample)
    {
        return Some("active terminal host is still showing generic Codex idle footer");
    }
    if terminal_chunk_has_prompt_output(text_sample) {
        return Some("active terminal host is only showing a plain shell prompt");
    }
    if terminal_chunk_is_transcript_browser(text_sample) {
        return Some("active terminal host is still showing the transcript browser");
    }
    if terminal_chunk_is_saved_transcript_prefill(text_sample) {
        return Some("active terminal host is still showing saved transcript prefill");
    }
    if terminal_chunk_is_launcher_boilerplate(text_sample) {
        return Some("active terminal host is still showing launcher boilerplate");
    }
    if terminal_chunk_is_low_signal_terminal_noise(text_sample) {
        return Some("active terminal host is still showing low-signal terminal noise");
    }
    None
}

fn terminal_host_geometry_problem_for_app_control(host: &Value) -> Option<&'static str> {
    let host_left = host
        .get("host_rect")
        .and_then(|value| value.get("left"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let host_top = host
        .get("host_rect")
        .and_then(|value| value.get("top"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let host_width = host
        .get("host_content_width")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 1.0)
        .or_else(|| {
            host.get("host_rect")
                .and_then(|value| value.get("width"))
                .and_then(Value::as_f64)
        })
        .unwrap_or(0.0);
    let host_outer_width = host
        .get("host_rect")
        .and_then(|value| value.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let screen_width = host
        .get("screen_rect")
        .and_then(|value| value.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let viewport_width = host
        .get("viewport_rect")
        .and_then(|value| value.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let host_height = host
        .get("host_content_height")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 1.0)
        .or_else(|| {
            host.get("host_rect")
                .and_then(|value| value.get("height"))
                .and_then(Value::as_f64)
        })
        .unwrap_or(0.0);
    let host_outer_height = host
        .get("host_rect")
        .and_then(|value| value.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let screen_height = host
        .get("screen_rect")
        .and_then(|value| value.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let viewport_height = host
        .get("viewport_rect")
        .and_then(|value| value.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let helpers_width = host
        .get("helpers_rect")
        .and_then(|value| value.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let helpers_height = host
        .get("helpers_rect")
        .and_then(|value| value.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let helper_textarea_width = host
        .get("helper_textarea_rect")
        .and_then(|value| value.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let helper_textarea_height = host
        .get("helper_textarea_rect")
        .and_then(|value| value.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let helper_textarea_left = host
        .get("helper_textarea_rect")
        .and_then(|value| value.get("left"))
        .and_then(Value::as_f64)
        .unwrap_or(host_left);
    let helper_textarea_top = host
        .get("helper_textarea_rect")
        .and_then(|value| value.get("top"))
        .and_then(Value::as_f64)
        .unwrap_or(host_top);
    let width_delta = (host_width - screen_width).abs();
    let compensated_screen_width_gap = host_width >= 240.0
        && screen_width >= 200.0
        && helpers_width >= 200.0
        && viewport_width >= 200.0
        && width_delta > 12.0
        && width_delta <= 18.0
        && (screen_width - helpers_width).abs() <= 4.0
        && (host_width - viewport_width).abs() <= 4.0;
    if host_width >= 240.0 && screen_width >= 200.0 && width_delta > 12.0
        && !compensated_screen_width_gap
    {
        return Some("active terminal host geometry does not match the xterm screen width");
    }
    if host_outer_width >= 240.0 && host_width >= 200.0 && (host_outer_width - host_width) > 28.0 {
        return Some(
            "active terminal host outer width is much wider than its usable xterm content",
        );
    }
    if screen_width >= 200.0
        && viewport_width >= 200.0
        && (screen_width - viewport_width).abs() > 12.0
        && !compensated_screen_width_gap
    {
        return Some("active terminal host geometry does not match the xterm viewport width");
    }
    if host_height >= 140.0 && screen_height >= 120.0 && (host_height - screen_height).abs() > 12.0
    {
        return Some("active terminal host geometry does not match the xterm screen height");
    }
    if host_outer_height >= 160.0
        && host_height >= 120.0
        && (host_outer_height - host_height) > 28.0
    {
        return Some(
            "active terminal host outer height is much taller than its usable xterm content",
        );
    }
    if screen_height >= 120.0
        && viewport_height >= 120.0
        && (screen_height - viewport_height).abs() > 12.0
    {
        return Some("active terminal host geometry does not match the xterm viewport height");
    }
    if helpers_width >= 200.0
        && host_width >= 200.0
        && (helpers_width - host_width).abs() > 12.0
        && !compensated_screen_width_gap
    {
        return Some("active terminal host helper layer is wider than the visible host");
    }
    if helpers_height >= 120.0
        && host_height >= 120.0
        && (helpers_height - host_height).abs() > 12.0
    {
        return Some("active terminal host helper layer is taller than the visible host");
    }
    if helper_textarea_width > 8.0
        || helper_textarea_height > 8.0
        || (helper_textarea_left - host_left).abs() > 32.0
        || (helper_textarea_top - host_top).abs() > 32.0
    {
        return Some("active terminal host helper textarea drifted outside the visible host");
    }
    None
}

pub(crate) fn terminal_chunk_has_meaningful_output(data: &str) -> bool {
    if terminal_chunk_is_generic_codex_idle(data)
        || terminal_chunk_is_transcript_browser(data)
        || terminal_chunk_is_loading_placeholder(data)
    {
        return false;
    }
    let stripped = strip_terminal_control_sequences(data);
    let normalized_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if normalized_lines.is_empty() {
        return false;
    }
    let prompt_like = normalized_lines.len() <= 2
        && normalized_lines.iter().all(|line| line.len() <= 48)
        && normalized_lines.iter().any(|line| {
            line.ends_with('$') || line.ends_with('#') || line.ends_with('>') || line.ends_with('%')
        });
    if prompt_like {
        return false;
    }
    let printable = stripped
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .count();
    let newline_count = stripped
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\r')
        .count();
    printable >= 80 || newline_count >= 6 || normalized_lines.len() >= 4
}

pub(crate) fn terminal_chunk_has_prompt_output(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !normalized_lines.is_empty()
        && normalized_lines.len() <= 2
        && normalized_lines.iter().all(|line| line.len() <= 48)
        && normalized_lines.iter().any(|line| {
            line.ends_with('$') || line.ends_with('#') || line.ends_with('>') || line.ends_with('%')
        })
}

pub(crate) fn terminal_chunk_is_generic_codex_idle(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if normalized_lines.is_empty() {
        return false;
    }
    let has_codex_header =
        stripped.contains("OpenAI Codex") && stripped.contains("/model to change");
    if !has_codex_header {
        return false;
    }
    let printable = stripped
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .count();
    if printable > 420 || normalized_lines.len() > 12 {
        return false;
    }
    let transcript_like_lines = normalized_lines
        .iter()
        .filter(|line| {
            let line = line.trim();
            let semantic =
                line.trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '));
            let lower = semantic.to_ascii_lowercase();
            let border_only = semantic.is_empty();
            !lower.starts_with("tip:")
                && !lower.contains("model:")
                && !lower.contains("directory:")
                && !lower.contains("openai codex")
                && !lower.starts_with('›')
                && !lower.contains("% left")
                && !border_only
        })
        .count();
    transcript_like_lines <= 3
}

pub(crate) fn terminal_chunk_has_generic_codex_idle_footer(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.len() > 5 {
        return false;
    }
    let normalized = lines.join("\n").to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    let mentions_generic_prompt = lines.iter().any(|line| {
        let semantic = line
            .trim()
            .trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '));
        let lower = semantic.to_ascii_lowercase();
        lower.starts_with('›')
            && (lower.contains("implement {feature}")
                || lower.contains("explain this codebase")
                || lower.contains("find and fix a bug")
                || lower.contains("resume a previous session"))
    });
    let mentions_model_footer = (normalized.contains("gpt-5")
        || normalized.contains("gpt-4")
        || normalized.contains("claude"))
        && normalized.contains("% left");
    mentions_generic_prompt && mentions_model_footer
}

pub(crate) fn terminal_chunk_is_transcript_browser(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.contains("resume a previous session")
        && (normalized.contains("sort updated at")
            || normalized.contains("sort: updated at")
            || normalized.contains("conversation"))
    {
        return true;
    }
    if !normalized.contains("transcript") && !normalized.contains("t r a n s c r i p t") {
        return false;
    }
    normalized.contains("q to quit")
        || normalized.contains("to scroll")
        || normalized.contains("pgup/pgdn")
        || normalized.contains("pgup pgdn")
        || normalized.contains("home/end to jump")
        || normalized.contains("home end to jump")
        || normalized.contains("esc to edit prev")
        || normalized.contains("edit prev")
}

pub(crate) fn terminal_chunk_is_loading_placeholder(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (normalized.contains("resuming live codex session")
        || normalized.contains("resuming remote codex session"))
        && (normalized.contains("waiting for the remote terminal to paint")
            || normalized.contains("still connecting to remote terminal"))
}

pub(crate) fn terminal_chunk_has_visible_output(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    stripped
        .chars()
        .any(|ch| !ch.is_control() && !ch.is_whitespace())
}

pub(crate) fn terminal_chunk_is_saved_transcript_prefill(data: &str) -> bool {
    let normalized = strip_terminal_control_sequences(data)
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.starts_with("saved transcript")
        && normalized.contains("saved transcript ·")
        && normalized.contains("typing takes over the live terminal")
}

pub(crate) fn terminal_chunk_is_launcher_boilerplate(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.contains("open live terminal ")
        || normalized.contains("launch command prepared:")
        || normalized.contains("daemon pty:")
        || normalized.contains("queue remote yggterm resume ")
        || stripped.contains("__YGGTERM_REQUESTED=")
        || stripped.contains("__YGGTERM_CWD_OK=")
}

pub(crate) fn terminal_chunk_is_low_signal_terminal_noise(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped.trim();
    if normalized.is_empty() {
        return false;
    }
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() > 96 || !compact.contains("^[") {
        return false;
    }
    compact.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '^' | '[' | ']' | ';' | '?' | ':' | '-' | '_' | '<' | '>' | '=' | '/' | '\\' | '.'
            )
    })
}

pub(crate) fn terminal_chunk_is_transport_error(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }
    let head = lines.join(" ");
    if [
        "error: reading /tmp/yggterm-screen",
        "mux_client_request_session",
        "session open refused by peer",
        "controlsocket",
        "exec: export: not found",
        "terminal session not found",
        "[screen is terminating]",
        "saved codex session",
        "cannot be restored as a live terminal",
    ]
    .iter()
    .any(|fragment| head.contains(fragment))
    {
        return true;
    }
    lines.iter().any(|line| {
        let line = line.trim();
        (line.starts_with("shared connection to ")
            && (line.contains(" closed") || line.contains("refused") || line.contains("timed out")))
            || (line.starts_with("connection to ")
                && (line.contains(" closed")
                    || line.contains("refused")
                    || line.contains("timed out")))
            || ((line.starts_with("ssh:")
                || line.starts_with("error:")
                || line.starts_with("fatal:")
                || line.starts_with("rsync:"))
                && (line.contains("permission denied")
                    || line.contains("connection refused")
                    || line.contains("no route to host")
                    || line.contains("connection timed out")
                    || line.contains("broken pipe")))
    })
}

pub(crate) fn terminal_tail_excerpt(data: &str, max_chars: usize) -> String {
    let chars = data.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return data.to_string();
    }
    chars[chars.len().saturating_sub(max_chars)..]
        .iter()
        .collect()
}

pub(crate) fn strip_terminal_control_sequences(data: &str) -> String {
    let chars = data.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(data.len());
    let mut ix = 0usize;
    while ix < chars.len() {
        if chars[ix] == '\u{1b}' {
            ix += 1;
            if ix >= chars.len() {
                break;
            }
            match chars[ix] {
                '[' => {
                    ix += 1;
                    while ix < chars.len() {
                        let ch = chars[ix];
                        ix += 1;
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                ']' => {
                    ix += 1;
                    while ix < chars.len() {
                        let ch = chars[ix];
                        ix += 1;
                        if ch == '\u{7}' {
                            break;
                        }
                        if ch == '\u{1b}' && ix < chars.len() && chars[ix] == '\\' {
                            ix += 1;
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        out.push(chars[ix]);
        ix += 1;
    }
    out
}
