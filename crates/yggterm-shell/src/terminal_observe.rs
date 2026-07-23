use serde_json::{Value, json};
use yggterm_server::WorkspaceViewMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalOpenAttemptState {
    Pending,
    Recovering,
    Ready,
    Failed,
}

/// Snapshot of system memory/swap pressure, sampled when a terminal reveal
/// begins. The reveal-starvation finding
/// ([[finding-xterm6-cold-reveal-render-starvation]]) established swap thrash as
/// the dominant amplifier of slow cold reveals, so every reveal records the swap
/// state at its start. That makes "the terminal took forever to come up"
/// self-diagnosing — a slow reveal recorded alongside `swap_used_mb: 9400` tells
/// the user to free RAM rather than chase a phantom yggterm render bug.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MemoryPressureSnapshot {
    pub(crate) swap_used_kb: u64,
    pub(crate) swap_total_kb: u64,
    pub(crate) mem_available_kb: u64,
    pub(crate) mem_total_kb: u64,
}

impl MemoryPressureSnapshot {
    pub(crate) fn swap_used_mb(&self) -> u64 {
        self.swap_used_kb / 1024
    }
    pub(crate) fn swap_total_mb(&self) -> u64 {
        self.swap_total_kb / 1024
    }
    pub(crate) fn mem_available_mb(&self) -> u64 {
        self.mem_available_kb / 1024
    }
    pub(crate) fn mem_total_mb(&self) -> u64 {
        self.mem_total_kb / 1024
    }
    /// True when enough swap is in use that a cold mount is likely to fault
    /// against it. >512 MB is the rough floor where the finding saw cold-reveal
    /// mounts start thrashing; below that, swap is incidental.
    pub(crate) fn swap_pressured(&self) -> bool {
        self.swap_used_kb > 512 * 1024
    }
    /// Whether the snapshot carries any usable reading at all (false on
    /// platforms without `/proc/meminfo` or when the read failed).
    pub(crate) fn is_present(&self) -> bool {
        self.mem_total_kb > 0
    }
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "present": self.is_present(),
            "swap_used_mb": self.swap_used_mb(),
            "swap_total_mb": self.swap_total_mb(),
            "mem_available_mb": self.mem_available_mb(),
            "mem_total_mb": self.mem_total_mb(),
            "swap_pressured": self.swap_pressured(),
        })
    }
}

/// Parse kernel `/proc/meminfo` text into a memory-pressure snapshot. Pure (no
/// I/O) so it is unit-testable with fixture text. Unknown / missing keys leave
/// their fields at zero; `swap_used = SwapTotal - SwapFree`.
pub(crate) fn parse_meminfo(text: &str) -> MemoryPressureSnapshot {
    let mut mem_total_kb = 0_u64;
    let mut mem_available_kb = 0_u64;
    let mut swap_total_kb = 0_u64;
    let mut swap_free_kb = 0_u64;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(value_kb) = rest
            .split_whitespace()
            .next()
            .and_then(|token| token.parse::<u64>().ok())
        else {
            continue;
        };
        match key.trim() {
            "MemTotal" => mem_total_kb = value_kb,
            "MemAvailable" => mem_available_kb = value_kb,
            "SwapTotal" => swap_total_kb = value_kb,
            "SwapFree" => swap_free_kb = value_kb,
            _ => {}
        }
    }
    MemoryPressureSnapshot {
        swap_used_kb: swap_total_kb.saturating_sub(swap_free_kb),
        swap_total_kb,
        mem_available_kb,
        mem_total_kb,
    }
}

/// Read live system memory pressure from `/proc/meminfo`. Returns the default
/// (all zeros, `is_present() == false`) on any platform without the file or on
/// read failure — callers treat an absent snapshot as "unknown", never as
/// "no pressure".
pub(crate) fn read_memory_pressure_snapshot() -> MemoryPressureSnapshot {
    std::fs::read_to_string("/proc/meminfo")
        .map(|text| parse_meminfo(&text))
        .unwrap_or_default()
}

/// One finished terminal reveal (ready or failed), retained in a small ring so
/// the GUI can show a "reveal log" and the agent can review reveal timing +
/// swap pressure WITHOUT polling app-state — which itself starves the reveal it
/// is trying to measure (see [[finding-xterm6-cold-reveal-render-starvation]]).
#[derive(Debug, Clone)]
pub(crate) struct RevealLogEntry {
    pub(crate) session_path: String,
    pub(crate) label: String,
    pub(crate) kind: String,
    pub(crate) source: String,
    pub(crate) tier: String,
    pub(crate) started_at_ms: u64,
    pub(crate) finished_at_ms: u64,
    pub(crate) surface_mounted_at_ms: Option<u64>,
    pub(crate) first_output_at_ms: Option<u64>,
    /// "ready" | "failed"
    pub(crate) outcome: String,
    pub(crate) failure_reason: Option<String>,
    pub(crate) memory_pressure: MemoryPressureSnapshot,
}

impl RevealLogEntry {
    /// Total wall-clock the reveal took, start to terminal state.
    pub(crate) fn total_ms(&self) -> u64 {
        self.finished_at_ms.saturating_sub(self.started_at_ms)
    }
    fn relative_ms(&self, at_ms: Option<u64>) -> Option<u64> {
        at_ms.map(|value| value.saturating_sub(self.started_at_ms))
    }
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "session_path": self.session_path,
            "label": self.label,
            "kind": self.kind,
            "source": self.source,
            "tier": self.tier,
            "started_at_ms": self.started_at_ms,
            "finished_at_ms": self.finished_at_ms,
            "total_ms": self.total_ms(),
            "surface_mounted_ms": self.relative_ms(self.surface_mounted_at_ms),
            "first_output_ms": self.relative_ms(self.first_output_at_ms),
            "outcome": self.outcome,
            "failure_reason": self.failure_reason,
            "memory_pressure": self.memory_pressure.to_json(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalOpenAttempt {
    pub(crate) attempt_id: String,
    pub(crate) session_path: String,
    pub(crate) request_id: String,
    pub(crate) open_request_id: u64,
    pub(crate) source: String,
    pub(crate) started_at_ms: u64,
    /// System memory/swap pressure sampled at reveal start — the amplifier the
    /// reveal-starvation finding tracks. Carried into the reveal log so a slow
    /// reveal can be attributed to swap thrash vs an actual render stall.
    pub(crate) memory_pressure_at_start: MemoryPressureSnapshot,
    /// True when the daemon did NOT already own this session's runtime at reveal
    /// start — i.e. a COLD mount (re-resume + fresh PTY), the slow path the
    /// reveal-starvation finding is about. Hot reveals reuse a retained host.
    pub(crate) cold_at_start: bool,
    pub(crate) state: TerminalOpenAttemptState,
    pub(crate) observations: u64,
    pub(crate) rearm_count: u32,
    pub(crate) ready_at_ms: Option<u64>,
    pub(crate) surface_mounted_at_ms: Option<u64>,
    pub(crate) first_output_at_ms: Option<u64>,
    pub(crate) first_protocol_only_output_at_ms: Option<u64>,
    pub(crate) first_meaningful_output_at_ms: Option<u64>,
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
    let dom_error = dom.get("error").and_then(Value::as_str).map(str::to_string);
    let dom_degraded_reason = dom
        .get("degraded_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut active_terminal_hosts = active_session_path
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
    active_terminal_hosts.sort_by(|left, right| {
        terminal_host_active_rank_for_app_control(left)
            .cmp(&terminal_host_active_rank_for_app_control(right))
            .then_with(|| {
                terminal_host_sort_key_for_app_control(right)
                    .cmp(&terminal_host_sort_key_for_app_control(left))
            })
    });
    let active_terminal_identity_problem = active_session_path.as_deref().and_then(|path| {
        terminal_hosts.iter().find_map(|host| {
            let host_session_path = host
                .get("session_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if host_session_path.is_empty() || host_session_path == path {
                return None;
            }
            terminal_host_claims_foreground_input_for_app_control(host).then(|| {
                format!(
                    "active terminal host identity mismatch: selected {path} but focused host belongs to {host_session_path}"
                )
            })
        })
    });
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
    let terminal_input_enabled = active_terminal_surface
        .get("foreground_input_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let clean_daemon_pty_surface_visible =
        active_terminal_hosts_have_clean_daemon_pty_output(&active_terminal_hosts);
    let clean_retained_prompt_surface_visible =
        active_terminal_hosts_have_clean_retained_prompt_output(&active_terminal_hosts);
    let active_resume_notification_visible = active_session_path
        .as_deref()
        .is_some_and(|path| active_terminal_resume_notification_visible(&notifications, path));
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
        if let Some(problem) = active_terminal_identity_problem.clone() {
            (false, false, Some("problem".to_string()), Some(problem))
        } else if active_terminal_hosts.is_empty() {
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
            (false, false, Some("problem".to_string()), Some(problem))
        } else if active_resume_notification_visible
            && !terminal_input_enabled
            && !clean_daemon_pty_surface_visible
            && !clean_retained_prompt_surface_visible
        {
            (
                false,
                false,
                Some("recovering".to_string()),
                Some("terminal resume notification is still visible".to_string()),
            )
        } else if terminal_rendered && terminal_input_enabled {
            (true, true, Some("interactive".to_string()), None)
        } else if terminal_rendered && !terminal_input_enabled {
            (
                true,
                false,
                Some("visible".to_string()),
                Some("terminal rendered but focus is outside the terminal".to_string()),
            )
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
        "dom_error": dom_error,
        "dom_degraded_reason": dom_degraded_reason,
        "terminal_host_count": terminal_hosts.len(),
        "terminal_hosts": terminal_hosts,
        "active_terminal_identity_problem": active_terminal_identity_problem,
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

fn active_terminal_resume_notification_visible(
    notifications: &[Value],
    session_path: &str,
) -> bool {
    notifications.iter().any(|notification| {
        notification
            .get("job_key")
            .and_then(Value::as_str)
            .and_then(|job_key| job_key.strip_prefix("terminal-resume:"))
            .is_some_and(|candidate| candidate == session_path)
    })
}

fn active_terminal_hosts_have_clean_daemon_pty_output(hosts: &[Value]) -> bool {
    hosts.iter().any(terminal_host_has_clean_daemon_pty_output)
}

fn active_terminal_hosts_have_clean_retained_prompt_output(hosts: &[Value]) -> bool {
    hosts
        .iter()
        .any(terminal_host_has_clean_retained_prompt_output)
}

fn terminal_host_has_clean_retained_prompt_output(host: &Value) -> bool {
    let content_source = host
        .get("terminal_content_source")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if content_source != "daemon_retained_history_screen_snapshot"
        || !host
            .get("retained_replay_prompt_follow_ready")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || !terminal_host_has_rendered_surface_for_app_control(host)
        || terminal_host_problem_for_app_control(host).is_some()
    {
        return false;
    }
    let visible_text = [
        "cursor_line_text",
        "text_tail",
        "text_sample",
        "buffer_text_sample",
        "cursor_row_text",
    ]
    .into_iter()
    .filter_map(|key| host.get(key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    !visible_text.is_empty()
        && !terminal_chunk_is_transport_error(&visible_text)
        && !terminal_chunk_is_loading_placeholder(&visible_text)
        && !terminal_chunk_is_transcript_browser(&visible_text)
        && !terminal_chunk_is_saved_transcript_prefill(&visible_text)
        && !terminal_chunk_is_low_signal_terminal_noise(&visible_text)
        && (terminal_chunk_has_current_codex_input_row(&visible_text)
            || terminal_chunk_has_codex_prompt_output(&visible_text)
            || terminal_chunk_is_codex_prompt_surface(&visible_text)
            || terminal_chunk_has_prompt_output(&visible_text))
}

fn terminal_host_has_clean_daemon_pty_output(host: &Value) -> bool {
    let content_source = host
        .get("terminal_content_source")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let last_raw_payload_length = host
        .get("last_raw_payload_length")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let retained_prompt_follow_ready = host
        .get("retained_replay_prompt_follow_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pty_source_ready = content_source == "daemon_pty"
        || (matches!(
            content_source,
            "active_recovery_pty_snapshot" | "daemon_terminal_read" | "daemon_screen_snapshot"
        ) && retained_prompt_follow_ready);
    if !pty_source_ready || !terminal_host_has_rendered_surface_for_app_control(host) {
        return false;
    }
    if terminal_host_problem_for_app_control(host).is_some() {
        return false;
    }
    let visible_text = [
        "text_tail",
        "text_sample",
        "buffer_text_sample",
        "cursor_line_text",
        "cursor_row_text",
    ]
    .into_iter()
    .filter_map(|key| host.get(key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    !visible_text.is_empty()
        && !terminal_chunk_is_transport_error(&visible_text)
        && !terminal_chunk_is_loading_placeholder(&visible_text)
        && !terminal_chunk_is_transcript_browser(&visible_text)
        && !terminal_chunk_is_saved_transcript_prefill(&visible_text)
        && !terminal_chunk_is_low_signal_terminal_noise(&visible_text)
        && (content_source != "daemon_pty"
            || (last_raw_payload_length > 0 && terminal_chunk_has_meaningful_output(&visible_text)))
        && (terminal_chunk_has_prompt_output(&visible_text)
            || terminal_chunk_has_codex_prompt_output(&visible_text)
            || terminal_chunk_is_codex_prompt_surface(&visible_text)
            || content_source == "daemon_pty")
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
        "rearm_count": attempt.rearm_count,
        "ready_at_ms": attempt.ready_at_ms,
        "surface_mounted_at_ms": attempt.surface_mounted_at_ms,
        "first_output_at_ms": attempt.first_output_at_ms,
        "first_protocol_only_output_at_ms": attempt.first_protocol_only_output_at_ms,
        "first_meaningful_output_at_ms": attempt.first_meaningful_output_at_ms,
        "request_to_surface_mounted_ms": attempt
            .surface_mounted_at_ms
            .map(|value| value.saturating_sub(attempt.started_at_ms)),
        "request_to_first_output_ms": attempt
            .first_output_at_ms
            .map(|value| value.saturating_sub(attempt.started_at_ms)),
        "surface_mounted_to_first_output_ms": attempt.first_output_at_ms.and_then(|first_output| {
            attempt
                .surface_mounted_at_ms
                .map(|mounted| first_output.saturating_sub(mounted))
        }),
        "request_to_first_protocol_only_output_ms": attempt
            .first_protocol_only_output_at_ms
            .map(|value| value.saturating_sub(attempt.started_at_ms)),
        "request_to_first_meaningful_output_ms": attempt
            .first_meaningful_output_at_ms
            .map(|value| value.saturating_sub(attempt.started_at_ms)),
        "surface_mounted_to_first_meaningful_output_ms": attempt
            .first_meaningful_output_at_ms
            .and_then(|first_meaningful| {
                attempt
                    .surface_mounted_at_ms
                    .map(|mounted| first_meaningful.saturating_sub(mounted))
            }),
        "first_output_to_ready_ms": attempt.ready_at_ms.and_then(|ready| {
            attempt
                .first_output_at_ms
                .map(|first_output| ready.saturating_sub(first_output))
        }),
        "first_meaningful_output_to_ready_ms": attempt.ready_at_ms.and_then(|ready| {
            attempt
                .first_meaningful_output_at_ms
                .map(|first_meaningful| ready.saturating_sub(first_meaningful))
        }),
        "request_to_ready_ms": attempt
            .ready_at_ms
            .map(|value| value.saturating_sub(attempt.started_at_ms)),
        "surface_mounted_to_ready_ms": attempt.ready_at_ms.and_then(|ready| {
            attempt
                .surface_mounted_at_ms
                .map(|mounted| ready.saturating_sub(mounted))
        }),
        "request_to_failure_ms": attempt
            .latched_failure_at_ms
            .map(|value| value.saturating_sub(attempt.started_at_ms)),
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

pub(crate) fn terminal_bootstrap_activation_epoch(
    active_view_mode: WorkspaceViewMode,
    active_session_path: Option<&str>,
    session_path: &str,
    latest_open_request_id: u64,
) -> u64 {
    if active_view_mode == WorkspaceViewMode::Terminal && active_session_path == Some(session_path)
    {
        latest_open_request_id
    } else {
        0
    }
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
    let performance_problem = hosts
        .iter()
        .find_map(terminal_host_performance_problem_for_app_control)
        .map(str::to_string);
    let render_health = terminal_render_health_for_app_control(hosts);
    let active_host = hosts
        .iter()
        .min_by_key(|host| terminal_host_active_rank_for_app_control(host));
    let raw_input_enabled = hosts.iter().any(|host| {
        host.get("host_stdin_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let helper_textarea_focused = hosts.iter().any(|host| {
        host.get("helper_textarea_focused")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let raw_effective_input_focus = hosts.iter().any(|host| {
        host.get("effective_input_focus")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                host.get("host_stdin_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && host
                        .get("helper_textarea_focused")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    && host
                        .get("host_has_active_element")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
    });
    let terminal_input_hot = hosts.iter().any(|host| {
        host.get("terminal_input_hot")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let render_health_problem = render_health
        .get("healthy")
        .and_then(Value::as_bool)
        .filter(|healthy| !healthy)
        .and_then(|_| render_health.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let problem = if overlay_context_visible {
        None::<String>
    } else {
        geometry_problem
            .clone()
            .or(render_health_problem.clone())
            .or(live_problem.clone())
    };
    let foreground_input_ready = raw_input_enabled && raw_effective_input_focus && problem.is_none();
    let effective_input_focus = raw_effective_input_focus && problem.is_none();
    json!({
        "rendered": rendered,
        "problem": problem,
        "geometry_problem": geometry_problem,
        "live_problem": live_problem,
        "performance_problem": performance_problem,
        "render_health": render_health,
        "overlay_context_visible": overlay_context_visible,
        "foreground_input_ready": foreground_input_ready,
        "raw_input_enabled": raw_input_enabled,
        "effective_input_focus": effective_input_focus,
        "raw_effective_input_focus": raw_effective_input_focus,
        "helper_textarea_focused": helper_textarea_focused,
        "terminal_input_hot": terminal_input_hot,
        "host_id": active_host.and_then(|host| host.get("host_id")).cloned().unwrap_or(Value::Null),
        "session_path": active_host.and_then(|host| host.get("session_path")).cloned().unwrap_or(Value::Null),
        "content_source": active_host.and_then(|host| host.get("terminal_content_source")).cloned().unwrap_or(Value::Null),
        "retained_replay_source": active_host.and_then(|host| host.get("retained_replay_source")).cloned().unwrap_or(Value::Null),
        "source_mismatch_reason": active_host.and_then(|host| host.get("terminal_source_mismatch_reason")).cloned().unwrap_or(Value::Null),
        "prompt_band": terminal_prompt_band_for_app_control(active_host),
        "timing": terminal_timing_for_app_control(active_host),
    })
}

fn terminal_host_active_rank_for_app_control(host: &Value) -> u8 {
    if terminal_host_has_effective_focus_for_app_control(host) {
        return 0;
    }
    if host
        .get("host_stdin_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return 1;
    }
    if terminal_host_has_rendered_surface_for_app_control(host) {
        return 2;
    }
    3
}

fn terminal_host_claims_foreground_input_for_app_control(host: &Value) -> bool {
    let host_stdin_enabled = host
        .get("host_stdin_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let raw_input_enabled = host
        .get("raw_input_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if host_stdin_enabled || raw_input_enabled {
        return true;
    }
    terminal_host_has_effective_focus_for_app_control(host)
}

fn terminal_host_has_effective_focus_for_app_control(host: &Value) -> bool {
    let helper_focused = host
        .get("helper_textarea_focused")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || host
            .get("host_has_active_element")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if !helper_focused {
        return false;
    }
    let document_focused = host
        .get("document_focused")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let active_session_host = host
        .get("is_active_session_host")
        .and_then(Value::as_bool)
        .or_else(|| host.get("active").and_then(Value::as_bool))
        .unwrap_or(false);
    document_focused && active_session_host
}

fn terminal_host_sort_key_for_app_control(host: &Value) -> String {
    [
        host.get("last_render_event_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .to_string(),
        host.get("last_write_flush_started_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .to_string(),
        host.get("host_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ]
    .join(":")
}

fn terminal_observe_max_blank_rows_below_live_cursor(rows: u64) -> u64 {
    if rows >= 36 {
        3
    } else if rows >= 20 {
        2
    } else {
        1
    }
}

fn terminal_observe_prompt_layout_is_acceptable(rows: u64, blank_rows_below_cursor: u64) -> bool {
    rows == 0 || blank_rows_below_cursor <= terminal_observe_max_blank_rows_below_live_cursor(rows)
}

fn terminal_observe_codex_prompt_tail_has_real_scrollback(
    cursor_line_text: &str,
    visible_text: &str,
) -> bool {
    if !terminal_chunk_has_codex_prompt_output(cursor_line_text)
        || !terminal_chunk_has_codex_prompt_output(visible_text)
        || terminal_chunk_is_transport_error(visible_text)
        || terminal_chunk_is_loading_placeholder(visible_text)
        || terminal_chunk_is_transcript_browser(visible_text)
        || terminal_chunk_is_saved_transcript_prefill(visible_text)
        || terminal_chunk_is_low_signal_terminal_noise(visible_text)
    {
        return false;
    }
    let stripped = strip_terminal_control_sequences(visible_text);
    let non_empty_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let Some(prompt_index) = non_empty_lines.iter().rposition(|line| {
        let semantic =
            line.trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '));
        semantic.starts_with('›')
    }) else {
        return false;
    };
    prompt_index >= 4 && non_empty_lines.len().saturating_sub(prompt_index) <= 4
}

fn terminal_prompt_band_for_app_control(host: Option<&Value>) -> Value {
    let Some(host) = host else {
        return Value::Null;
    };
    json!({
        "cursor_line_text": host.get("cursor_line_text").or_else(|| host.get("cursor_row_text")).cloned().unwrap_or(Value::Null),
        "cursor_row_rect": host.get("cursor_row_rect").cloned().unwrap_or(Value::Null),
        "cursor_sample_rect": host.get("cursor_sample_rect").cloned().unwrap_or(Value::Null),
        "cursor_expected_rect": host.get("cursor_expected_rect").cloned().unwrap_or(Value::Null),
        "input_line_overlay_rect": host.get("input_line_overlay_rect").cloned().unwrap_or(Value::Null),
        "input_line_overlay_background": host.get("input_line_overlay_background").cloned().unwrap_or(Value::Null),
        "input_line_overlay_box_shadow": host.get("input_line_overlay_box_shadow").cloned().unwrap_or(Value::Null),
        "software_canvas_input_line_overlay_present": host.get("software_canvas_input_line_overlay_present").cloned().unwrap_or(Value::Bool(false)),
        "software_canvas_input_line_overlay_visible": host.get("software_canvas_input_line_overlay_visible").cloned().unwrap_or(Value::Bool(false)),
        "software_canvas_cursor_overlay_present": host.get("software_canvas_cursor_overlay_present").cloned().unwrap_or(Value::Bool(false)),
        "software_canvas_cursor_overlay_visible": host.get("software_canvas_cursor_overlay_visible").cloned().unwrap_or(Value::Bool(false)),
        "xterm_input_line_decoration_present": host.get("xterm_input_line_decoration_present").cloned().unwrap_or(Value::Bool(false)),
        "xterm_input_line_decoration_visible": host.get("xterm_input_line_decoration_visible").cloned().unwrap_or(Value::Bool(false)),
        "xterm_input_line_decoration_line": host.get("xterm_input_line_decoration_line").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_width": host.get("xterm_input_line_decoration_width").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_background": host.get("xterm_input_line_decoration_background").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_error": host.get("xterm_input_line_decoration_error").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_disposed": host.get("xterm_input_line_decoration_disposed").cloned().unwrap_or(Value::Bool(false)),
        "xterm_input_line_decoration_marker_line": host.get("xterm_input_line_decoration_marker_line").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_element_present": host.get("xterm_input_line_decoration_element_present").cloned().unwrap_or(Value::Bool(false)),
        "xterm_input_line_decoration_element_visible": host.get("xterm_input_line_decoration_element_visible").cloned().unwrap_or(Value::Bool(false)),
        "xterm_input_line_decoration_element_background": host.get("xterm_input_line_decoration_element_background").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_element_display": host.get("xterm_input_line_decoration_element_display").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_element_rect": host.get("xterm_input_line_decoration_element_rect").cloned().unwrap_or(Value::Null),
        "xterm_input_line_decoration_render_count": host.get("xterm_input_line_decoration_render_count").cloned().unwrap_or(Value::Number(0.into())),
        "last_raw_payload_sample": host.get("last_raw_payload_sample").cloned().unwrap_or(Value::Null),
        "cursor_bottom_overflow_px": host.get("cursor_bottom_overflow_px").cloned().unwrap_or(Value::Null),
        "fit_overflow_px": host.get("fit_overflow_px").cloned().unwrap_or(Value::Null),
        "fit_required_height_px": host.get("fit_required_height_px").cloned().unwrap_or(Value::Null),
        "fit_available_height_px": host.get("fit_available_height_px").cloned().unwrap_or(Value::Null),
        "blank_rows_below_cursor": host.get("blank_rows_below_cursor").cloned().unwrap_or(Value::Null),
        "base_y": host.get("base_y").cloned().unwrap_or(Value::Null),
        "viewport_y": host.get("viewport_y").cloned().unwrap_or(Value::Null),
        "dom_paint_hit_test_problem": host.get("dom_paint_hit_test_problem").cloned().unwrap_or(Value::Null),
        "dom_paint_hit_test": host.get("dom_paint_hit_test").cloned().unwrap_or(Value::Null),
        "retained_replay_expected": host.get("retained_replay_expected").cloned().unwrap_or(Value::Bool(false)),
        "retained_replay_source": host.get("retained_replay_source").cloned().unwrap_or(Value::Null),
        "retained_replay_prompt_follow_ready": host.get("retained_replay_prompt_follow_ready").cloned().unwrap_or(Value::Null),
        "retained_replay_unsafe_skip_prompt_ready": host.get("retained_replay_unsafe_skip_prompt_ready").cloned().unwrap_or(Value::Bool(false)),
        "retained_replay_rejected_visible_text": host.get("retained_replay_rejected_visible_text").cloned().unwrap_or(Value::Null),
        "retained_replay_recovered_from_snapshot": host.get("retained_replay_recovered_from_snapshot").cloned().unwrap_or(Value::Bool(false)),
        "retained_replay_superseded_by_daemon_pty": host.get("retained_replay_superseded_by_daemon_pty").cloned().unwrap_or(Value::Bool(false)),
        "retained_replay_snapshot_age_ms": host.get("retained_replay_snapshot_age_ms").cloned().unwrap_or(Value::Null),
        "retained_replay_snapshot_error": host.get("retained_replay_snapshot_error").cloned().unwrap_or(Value::Null),
        "last_retained_replay_follow_debug": host.get("last_retained_replay_follow_debug").cloned().unwrap_or(Value::Null),
        "scrollback_expected": host.get("scrollback_expected").cloned().unwrap_or(Value::Bool(false)),
        "scrollback_locked": host.get("scrollback_locked").cloned().unwrap_or(Value::Bool(false)),
        "scrollback_intent": host.get("scrollback_intent").cloned().unwrap_or(Value::String("PromptFollow".to_string())),
        "last_viewport_force_debug": host.get("last_viewport_force_debug").cloned().unwrap_or(Value::Null),
        "last_viewport_force_reason": host.get("last_viewport_force_reason").cloned().unwrap_or(Value::Null),
        "last_viewport_force_at_ms": host.get("last_viewport_force_at_ms").cloned().unwrap_or(Value::Null),
        "low_power_tui_overlay_active": host.get("low_power_tui_overlay_active").cloned().unwrap_or(Value::Bool(false)),
        "low_power_tui_overlay_present": host.get("low_power_tui_overlay_present").cloned().unwrap_or(Value::Bool(false)),
    })
}

fn terminal_timing_for_app_control(host: Option<&Value>) -> Value {
    let Some(host) = host else {
        return Value::Null;
    };
    json!({
        "last_data_event_at_ms": host.get("last_data_event_at_ms").cloned().unwrap_or(Value::Null),
        "pending_input_bytes": host.get("pending_input_bytes").cloned().unwrap_or(Value::Null),
        "pending_input_flush_scheduled": host.get("pending_input_flush_scheduled").cloned().unwrap_or(Value::Null),
        "input_batch_flush_count": host.get("input_batch_flush_count").cloned().unwrap_or(Value::Null),
        "last_input_batch_length": host.get("last_input_batch_length").cloned().unwrap_or(Value::Null),
        "last_input_batch_flush_reason": host.get("last_input_batch_flush_reason").cloned().unwrap_or(Value::Null),
        "last_input_batch_at_ms": host.get("last_input_batch_at_ms").cloned().unwrap_or(Value::Null),
        "last_pending_input_reason": host.get("last_pending_input_reason").cloned().unwrap_or(Value::Null),
        "protocol_data_event_count": host.get("protocol_data_event_count").cloned().unwrap_or(Value::Null),
        "suppressed_terminal_protocol_response_count": host.get("suppressed_terminal_protocol_response_count").cloned().unwrap_or(Value::Null),
        "last_suppressed_terminal_protocol_response": host.get("last_suppressed_terminal_protocol_response").cloned().unwrap_or(Value::Null),
        "last_suppressed_terminal_protocol_response_at_ms": host.get("last_suppressed_terminal_protocol_response_at_ms").cloned().unwrap_or(Value::Null),
        "ignored_data_event_count": host.get("ignored_data_event_count").cloned().unwrap_or(Value::Null),
        "last_write_queued_at_ms": host.get("last_write_queued_at_ms").cloned().unwrap_or(Value::Null),
        "last_write_flush_started_at_ms": host.get("last_write_flush_started_at_ms").cloned().unwrap_or(Value::Null),
        "last_write_callback_at_ms": host.get("last_write_callback_at_ms").cloned().unwrap_or(Value::Null),
        "last_render_event_at_ms": host.get("last_render_event_at_ms").cloned().unwrap_or(Value::Null),
        "last_render_health_checked_at_ms": host.get("last_render_health_checked_at_ms").cloned().unwrap_or(Value::Null),
        "last_activation_repaint_at_ms": host.get("last_activation_repaint_at_ms").cloned().unwrap_or(Value::Null),
        "last_activation_repaint_reason": host.get("last_activation_repaint_reason").cloned().unwrap_or(Value::Null),
        "last_manual_redraw_at_ms": host.get("last_manual_redraw_at_ms").cloned().unwrap_or(Value::Null),
        "last_manual_redraw_started_at_ms": host.get("last_manual_redraw_started_at_ms").cloned().unwrap_or(Value::Null),
        "last_manual_redraw_settled_at_ms": host.get("last_manual_redraw_settled_at_ms").cloned().unwrap_or(Value::Null),
        "last_manual_redraw_duration_ms": host.get("last_manual_redraw_duration_ms").cloned().unwrap_or(Value::Null),
        "last_manual_redraw_effect": host.get("last_manual_redraw_effect").cloned().unwrap_or(Value::Null),
        "terminal_write_frame_ms": host.get("terminal_write_frame_ms").cloned().unwrap_or(Value::Null),
        "terminal_active_write_frame_ms": host.get("terminal_active_write_frame_ms").cloned().unwrap_or(Value::Null),
        "terminal_active_animation_write_frame_ms": host.get("terminal_active_animation_write_frame_ms").cloned().unwrap_or(Value::Null),
        "terminal_active_animation_sustained_write_frame_ms": host.get("terminal_active_animation_sustained_write_frame_ms").cloned().unwrap_or(Value::Null),
        "effective_terminal_write_frame_ms": host.get("effective_terminal_write_frame_ms").cloned().unwrap_or(Value::Null),
        "active_write_frame_budget": host.get("active_write_frame_budget").cloned().unwrap_or(Value::Null),
        "recent_frame_like_write_hot": host.get("recent_frame_like_write_hot").cloned().unwrap_or(Value::Null),
        "recent_inline_status_animation_hot": host.get("recent_inline_status_animation_hot").cloned().unwrap_or(Value::Null),
        "recent_inline_status_animation_started_at_ms": host.get("recent_inline_status_animation_started_at_ms").cloned().unwrap_or(Value::Null),
        "last_raw_payload_length": host.get("last_raw_payload_length").cloned().unwrap_or(Value::Null),
        "last_coalesced_payload_length": host.get("last_coalesced_payload_length").cloned().unwrap_or(Value::Null),
        "last_raw_payload_line_count": host.get("last_raw_payload_line_count").cloned().unwrap_or(Value::Null),
        "write_command_count": host.get("write_command_count").cloned().unwrap_or(Value::Null),
        "write_bridge_flush_count": host.get("write_bridge_flush_count").cloned().unwrap_or(Value::Null),
        "render_event_count": host.get("render_event_count").cloned().unwrap_or(Value::Null),
        "forced_refresh_count": host.get("forced_refresh_count").cloned().unwrap_or(Value::Null),
        "forced_refresh_skipped_count": host.get("forced_refresh_skipped_count").cloned().unwrap_or(Value::Null),
        "activation_repaint_count": host.get("activation_repaint_count").cloned().unwrap_or(Value::Null),
        "manual_redraw_count": host.get("manual_redraw_count").cloned().unwrap_or(Value::Null),
    })
}

fn terminal_render_health_for_app_control(hosts: &[Value]) -> Value {
    for host in hosts {
        if let Some(health) = terminal_host_render_health_for_app_control(host) {
            return health;
        }
    }
    json!({
        "healthy": true,
        "status": "unknown",
        "reason": Value::Null,
        "recovery_scheduled": false,
    })
}

fn terminal_host_render_health_for_app_control(host: &Value) -> Option<Value> {
    let status = host
        .get("render_health_status")
        .and_then(Value::as_str)
        .unwrap_or("");
    let reason = host
        .get("render_health_reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    let recovery_scheduled = host
        .get("pending_render_health_recovery")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let recovery_count = host
        .get("render_health_recovery_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if status == "unhealthy" {
        return Some(json!({
            "healthy": false,
            "status": status,
            "reason": if reason.is_empty() { "render_health_unhealthy" } else { reason },
            "recovery_scheduled": recovery_scheduled,
            "recovery_count": recovery_count,
        }));
    }

    let has_buffer_text = terminal_host_has_buffer_text_for_app_control(host);
    let canvas_count = host
        .get("canvas_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if terminal_host_dom_renderer_missing_text_layer_with_buffer_text(host) {
        return Some(json!({
            "healthy": false,
            "status": "unhealthy",
            "reason": "dom_renderer_missing_text_layer_with_buffer_text",
            "recovery_scheduled": recovery_scheduled,
            "recovery_count": recovery_count,
        }));
    }
    let (sampled_pixels, nontransparent_pixels, alpha_sum) = terminal_host_canvas_ink_totals(host);
    if has_buffer_text
        && canvas_count > 0
        && sampled_pixels > 0
        && (nontransparent_pixels == 0 || alpha_sum <= 12)
    {
        return Some(json!({
            "healthy": false,
            "status": "unhealthy",
            "reason": "canvas_blank_with_buffer_text",
            "recovery_scheduled": recovery_scheduled,
            "recovery_count": recovery_count,
            "sampled_pixels": sampled_pixels,
            "nontransparent_pixels": nontransparent_pixels,
            "alpha_sum": alpha_sum,
        }));
    }
    // NOTE: the DOM-style low-contrast check (terminal_host_canvas_low_contrast_style)
    // is intentionally NOT applied here. It reads DOM-derived foreground/background
    // colors, but when the canvas renderer owns the glyphs the `.xterm-rows` are
    // unstyled (computed `color` resolves to black) — so on Wayland (canvas
    // renderer) every healthy session falsely reported
    // `canvas_low_contrast_foreground_with_buffer_text` and got stuck `ready:false`.
    // Genuine canvas paint failures are still caught by `canvas_blank_with_buffer_text`
    // (real canvas-ink sampling) above. See docs/xterm-bugs.md#xterm-pipeline-latency.
    if terminal_host_dom_rows_transparent_with_buffer_text(host) {
        return Some(json!({
            "healthy": false,
            "status": "unhealthy",
            "reason": "dom_rows_transparent_with_buffer_text",
            "recovery_scheduled": recovery_scheduled,
            "recovery_count": recovery_count,
            "sampled_pixels": sampled_pixels,
            "nontransparent_pixels": nontransparent_pixels,
            "alpha_sum": alpha_sum,
        }));
    }
    if status == "healthy" {
        return Some(json!({
            "healthy": true,
            "status": status,
            "reason": Value::Null,
            "recovery_scheduled": recovery_scheduled,
            "recovery_count": recovery_count,
            "sampled_pixels": sampled_pixels,
            "nontransparent_pixels": nontransparent_pixels,
            "alpha_sum": alpha_sum,
        }));
    }
    None
}

fn terminal_host_has_buffer_text_for_app_control(host: &Value) -> bool {
    [
        host.get("text_sample")
            .and_then(Value::as_str)
            .unwrap_or(""),
        host.get("text_tail").and_then(Value::as_str).unwrap_or(""),
        host.get("buffer_text_sample")
            .and_then(Value::as_str)
            .unwrap_or(""),
        host.get("cursor_line_text")
            .or_else(|| host.get("cursor_row_text"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    ]
    .join("\n")
    .chars()
    .any(|ch| ch.is_ascii_alphanumeric())
}

fn terminal_host_dom_renderer_missing_text_layer_with_buffer_text(host: &Value) -> bool {
    if !terminal_host_has_buffer_text_for_app_control(host) {
        return false;
    }
    let xterm_present = host
        .get("xterm_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let screen_present = host
        .get("screen_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !xterm_present || !screen_present {
        return false;
    }
    let rows_present = host
        .get("rows_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if rows_present {
        return false;
    }
    let canvas_count = host
        .get("canvas_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let visible_canvas_layer_count = host
        .get("visible_canvas_layer_count")
        .and_then(Value::as_u64)
        .unwrap_or(canvas_count);
    if canvas_count > 0 || visible_canvas_layer_count > 0 {
        return false;
    }
    let renderer_mode = host
        .get("xterm_renderer_mode")
        .and_then(Value::as_str)
        .unwrap_or("");
    renderer_mode.is_empty() || renderer_mode == "dom"
}

fn terminal_host_dom_rows_transparent_with_buffer_text(host: &Value) -> bool {
    if !terminal_host_has_buffer_text_for_app_control(host) {
        return false;
    }
    if !host
        .get("rows_present")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    let canvas_count = host
        .get("canvas_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let renderer_mode = host
        .get("xterm_renderer_mode")
        .and_then(Value::as_str)
        .unwrap_or("");
    if canvas_count > 0 && renderer_mode != "dom" {
        return false;
    }
    let direct_fields = [
        "rows_text_fill_color",
        "rows_sample_text_fill_color",
        "cursor_row_text_fill_color",
    ];
    if direct_fields.iter().any(|field| {
        host.get(*field)
            .and_then(Value::as_str)
            .is_some_and(terminal_css_color_is_transparent)
    }) {
        return true;
    }
    for field in [
        "visible_row_samples_head",
        "visible_row_samples_tail",
        "cursor_row_span_samples",
    ] {
        let Some(samples) = host.get(field).and_then(Value::as_array) else {
            continue;
        };
        if samples.iter().any(|sample| {
            sample
                .get("text_fill_color")
                .and_then(Value::as_str)
                .is_some_and(terminal_css_color_is_transparent)
        }) {
            return true;
        }
    }
    false
}

fn terminal_css_color_is_transparent(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase().replace(' ', "");
    matches!(
        normalized.as_str(),
        "transparent" | "rgba(0,0,0,0)" | "rgba(0,0,0,0.0)" | "#0000"
    )
}

fn terminal_host_canvas_ink_totals(host: &Value) -> (u64, u64, u64) {
    let mut sampled_pixels = 0;
    let mut nontransparent_pixels = 0;
    let mut alpha_sum = 0;
    if let Some(layers) = host.get("canvas_layers").and_then(Value::as_array) {
        for layer in layers {
            let Some(sample) = layer.get("ink_sample") else {
                continue;
            };
            sampled_pixels += sample
                .get("sampled_pixels")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            nontransparent_pixels += sample
                .get("nontransparent_pixels")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            alpha_sum += sample.get("alpha_sum").and_then(Value::as_u64).unwrap_or(0);
        }
    }
    if let Some(sample) = host.get("render_health_ink_sample") {
        sampled_pixels += sample
            .get("sampled_pixels")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        nontransparent_pixels += sample
            .get("nontransparent_pixels")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        alpha_sum += sample.get("alpha_sum").and_then(Value::as_u64).unwrap_or(0);
    }
    (sampled_pixels, nontransparent_pixels, alpha_sum)
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

// XTERM-BUG: blank-viewport-client-snapshot-poison
// "viewport beyond scrollback base" is a TRANSIENT artifact for a cursor-addressed
// codex surface during a reveal/reseed: codex owns its scrollback so base_y stays
// near the top, and a brief reseed can leave viewport_y momentarily past base_y.
// Escalating that as a host fault makes the clean-output checks fail and drives a
// recovery/remount that interrupts a working session (the live "blink → restart"
// symptom). True only when the visible text is a codex surface AND base_y is small
// (no real scrollback). A genuinely scrolled real-scrollback session (base_y in the
// hundreds/thousands) returns false here and still escalates.
fn terminal_host_viewport_beyond_base_is_transient_codex_reseed(host: &Value) -> bool {
    let base_y = host.get("base_y").and_then(Value::as_f64).unwrap_or(0.0);
    let rows = host
        .get("rows")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 1.0)
        .unwrap_or(64.0);
    if base_y > rows {
        return false;
    }
    let visible_text = [
        "cursor_line_text",
        "text_tail",
        "text_sample",
        "buffer_text_sample",
        "cursor_row_text",
    ]
    .into_iter()
    .filter_map(|key| host.get(key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    !visible_text.is_empty()
        && (terminal_chunk_has_current_codex_input_row(&visible_text)
            || terminal_chunk_has_codex_prompt_output(&visible_text)
            || terminal_chunk_is_codex_prompt_surface(&visible_text)
            || terminal_chunk_is_codex_working_surface(&visible_text))
}
fn terminal_host_problem_for_app_control(host: &Value) -> Option<&'static str> {
    if let Some(problem) = terminal_host_geometry_problem_for_app_control(host) {
        // Suppress ONLY the transient codex-reseed "viewport beyond base" from the
        // escalation path (it stays observable via the geometry_problem field). All
        // other geometry problems, and this one for non-codex / real-scrollback
        // sessions, still escalate.
        if !(problem == "active terminal viewport is beyond the xterm scrollback base"
            && terminal_host_viewport_beyond_base_is_transient_codex_reseed(host))
        {
            return Some(problem);
        }
    }
    let text_sample = host
        .get("text_sample")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let text_tail = host
        .get("text_tail")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let buffer_text_sample = host
        .get("buffer_text_sample")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let session_path = host
        .get("session_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let cursor_line_text = host
        .get("cursor_line_text")
        .or_else(|| host.get("cursor_row_text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let host_stdin_enabled = host
        .get("host_stdin_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let raw_input_enabled = host
        .get("raw_input_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(host_stdin_enabled);
    let helper_textarea_focused = host
        .get("helper_textarea_focused")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let host_has_active_element = host
        .get("host_has_active_element")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let document_focused = host
        .get("document_focused")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let xterm_present = host
        .get("xterm_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let screen_present = host
        .get("screen_present")
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
    let render_event_count = host
        .get("render_event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cursor_node_count = host
        .get("cursor_node_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let data_event_count = host
        .get("data_event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let protocol_data_event_count = host
        .get("protocol_data_event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let user_data_event_count = data_event_count.saturating_sub(protocol_data_event_count);
    let pending_input_bytes = host
        .get("pending_input_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input_batch_flush_count = host
        .get("input_batch_flush_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_input_batch_length = host
        .get("last_input_batch_length")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_pending_input_reason = host
        .get("last_pending_input_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let last_input_batch_at_ms = host
        .get("last_input_batch_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let blank_rows_below_cursor = host
        .get("blank_rows_below_cursor")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let rows = host.get("rows").and_then(Value::as_u64).unwrap_or(0);
    let cols = host.get("cols").and_then(Value::as_u64).unwrap_or(0);
    let cursor_y = host.get("cursor_y").and_then(Value::as_u64).unwrap_or(rows);
    let base_y = host.get("base_y").and_then(Value::as_u64).unwrap_or(0);
    let scrollback_expected = host
        .get("scrollback_expected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let wheel_event_count = host
        .get("wheel_event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_raw_payload_line_count = host
        .get("last_raw_payload_line_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_raw_payload_length = host
        .get("last_raw_payload_length")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_raw_payload_sample = host
        .get("last_raw_payload_sample")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let last_data_event_at_ms = host
        .get("last_data_event_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_write_queued_at_ms = host
        .get("last_write_queued_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_write_flush_started_at_ms = host
        .get("last_write_flush_started_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_write_callback_at_ms = host
        .get("last_write_callback_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let write_command_count = host
        .get("write_command_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let terminal_content_source = host
        .get("terminal_content_source")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let retained_replay_source = host
        .get("retained_replay_source")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let retained_replay_prompt_follow_ready = host
        .get("retained_replay_prompt_follow_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let retained_replay_unsafe_skip_prompt_ready = host
        .get("retained_replay_unsafe_skip_prompt_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let xterm_buffer_kind = host
        .get("xterm_buffer_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let xterm_cursor_hidden = host
        .get("xterm_cursor_hidden")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mounted_entry_host_connected = host
        .get("mounted_entry_host_connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cursor_sample_visible = host
        .get("cursor_sample_rect")
        .and_then(Value::as_object)
        .is_some()
        || cursor_node_count > 0;
    let host_opacity = host
        .get("host_opacity")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let host_visibility = host
        .get("host_visibility")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let host_transparent = host_opacity
        .parse::<f64>()
        .is_ok_and(|opacity| opacity <= 0.001)
        || host_visibility.eq_ignore_ascii_case("hidden")
        || host_visibility.eq_ignore_ascii_case("collapse");
    let cursor_geometry_visible = !xterm_cursor_hidden
        && rows > 0
        && cursor_y < rows
        && host
            .get("cursor_expected_rect")
            .and_then(Value::as_object)
            .is_some();
    let mut visible_text_parts = Vec::new();
    for part in [text_sample, text_tail, buffer_text_sample, cursor_line_text] {
        if !part.is_empty() && !visible_text_parts.contains(&part) {
            visible_text_parts.push(part);
        }
    }
    let visible_text_samples = [text_sample, text_tail, buffer_text_sample, cursor_line_text];
    let sample_has_prompt_output = visible_text_samples
        .iter()
        .any(|sample| terminal_chunk_has_prompt_output(sample));
    let sample_has_codex_prompt_output = visible_text_samples
        .iter()
        .any(|sample| terminal_chunk_has_codex_prompt_output(sample));
    let sample_has_current_codex_input_row = visible_text_samples
        .iter()
        .any(|sample| terminal_chunk_has_current_codex_input_row(sample));
    let sample_has_meaningful_output = visible_text_samples
        .iter()
        .any(|sample| terminal_chunk_has_meaningful_output(sample));
    let sample_is_codex_prompt_surface = visible_text_samples
        .iter()
        .any(|sample| terminal_chunk_is_codex_prompt_surface(sample));
    // A codex turn in flight ("esc to interrupt" working footer): the surface is
    // HEALTHY-BUSY, not faulted, even though it has no `›` input row and may
    // transiently clear. Used below to suppress the "no current input row" /
    // "stale scrollback but no current prompt" / "empty surface" fault reasons so
    // a working codex is never remounted mid-turn (which would destroy scrollback).
    let codex_working_surface = visible_text_samples
        .iter()
        .any(|sample| terminal_chunk_is_codex_working_surface(sample));
    let visible_text = visible_text_parts.join("\n");
    let visible_text = visible_text.as_str();
    if host_transparent
        && (xterm_present
            || screen_present
            || rows_present
            || canvas_count > 0
            || render_event_count > 0
            || !visible_text.is_empty())
    {
        return Some("active terminal host is transparent while mounted");
    }
    if terminal_host_dom_rows_transparent_with_buffer_text(host) {
        return Some("dom_rows_transparent_with_buffer_text");
    }
    let dom_paint_hit_test_problem = host
        .get("dom_paint_hit_test_problem")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !visible_text.is_empty()
        && !dom_paint_hit_test_problem.is_empty()
        && !terminal_host_dom_paint_problem_is_titlebar_clip(host)
    {
        return Some("active terminal DOM rows are present but not paint-visible");
    }
    // COULDN'T-OBSERVE GUARD — do NOT invent a problem from the absence of a
    // readable client text snapshot.
    //
    // A buffer read taken while this host is NOT the foreground input owner is
    // low-confidence: the canvas can be painting live content the user sees and
    // uses, while `term.buffer.active` reads back empty or as a single sparse
    // row (the snapshot is captured on blur via the "focus_released" path).
    // Classifying that sparse read as a definite problem ("active terminal host
    // is only showing a plain shell prompt", "...has no current input row",
    // "...accepted input without a following daemon stream echo", ...) is a
    // false positive — and it also drives spurious fault-recovery on a healthy
    // session. When the surface is demonstrably rendered AND the daemon is
    // actively feeding it bytes, but this host does not hold input focus AND the
    // readable client text is too sparse to classify, report no problem rather
    // than diagnosing from non-evidence. Genuine geometry / transparency / paint
    // faults are already handled above this guard and still surface.
    // "Reliable read" = this host owns input, OR the window/document is focused
    // (the user is looking at it and its buffer is painted live + current). A read
    // taken with NONE of these is low-confidence — captured on blur, it reads back
    // empty/sparse/placeholder even while the canvas paints live.
    let host_holds_input_focus = raw_input_enabled
        || helper_textarea_focused
        || host_has_active_element
        || document_focused;
    // The decisive "this is a live, healthy surface I just can't read cleanly"
    // signal is a non-empty `last_raw_payload_sample`: the daemon delivered a
    // real paint frame for THIS surface very recently. A genuinely stuck/stale
    // surface (codex never reached a prompt, retained prose, gated tail) shows
    // buffered/retained text with NO current live paint frame — those keep being
    // flagged. Combined with "this host is not the foreground input owner" (the
    // text snapshot was captured on blur and reads back empty/sparse/placeholder
    // even while the canvas paints live), this is exactly the false-positive
    // illusion (2026-06-03): the instrument, not the session, was broken.
    let surface_has_live_daemon_paint =
        (canvas_count > 0 || render_event_count > 0) && !last_raw_payload_sample.is_empty();
    // A transport/error string is unambiguous and worth surfacing regardless of
    // focus; everything else (which prompt/state the surface is "supposed" to be
    // in) is an interactive-state judgment that needs a reliable foreground read.
    let any_transport_error = terminal_chunk_is_transport_error(visible_text)
        || terminal_chunk_is_transport_error(last_raw_payload_sample)
        || visible_text_samples
            .iter()
            .any(|sample| terminal_chunk_is_transport_error(sample));
    // CONFIDENCE-GATED abstain. The content-finding samples (text_tail /
    // buffer_text_sample / cursor_line_text, built from readTerminalBufferSample's
    // trailing+leading fallback) are RELIABLE even when this host is unfocused —
    // confirmed live 2026-06-04: an unfocused codex surface (focus_released, all
    // focus flags false) still read text_tail=4044 chars + cursor_line_text="› Use
    // /skills…". So "unfocused" alone is NOT "couldn't observe"; only an unfocused
    // host whose readable content is ALSO too sparse to classify is genuinely
    // unobservable. Abstaining only in that case restores full detection for the
    // common unfocused-but-readable case (retires the broad focus-only guard) while
    // still suppressing the real false-positive illusion (empty/sparse blur read).
    let readable_content_is_sparse =
        visible_text.chars().filter(|c| !c.is_whitespace()).count() < 8;
    if !host_holds_input_focus
        && surface_has_live_daemon_paint
        && readable_content_is_sparse
        && !any_transport_error
    {
        return None;
    }
    let codex_interactive_setup_prompt =
        terminal_chunk_is_codex_interactive_setup_prompt(visible_text);
    let codex_prompt_surface =
        terminal_chunk_is_codex_prompt_surface(visible_text) || sample_is_codex_prompt_surface;
    let codex_interrupted_input_surface =
        terminal_chunk_is_codex_interrupted_input_surface(visible_text);
    let remote_codex_prompt_has_real_output = data_event_count > 0
        || last_data_event_at_ms > 0
        || last_raw_payload_length > 0
        || write_command_count > 0;
    let live_remote_codex_prompt_only_surface = session_path.starts_with("remote-session://")
        && !codex_prompt_surface
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && mounted_entry_host_connected
        && remote_codex_prompt_has_real_output
        && (host_stdin_enabled || helper_textarea_focused || cursor_sample_visible)
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text);
    let live_remote_codex_retained_prompt_surface = session_path.starts_with("remote-session://")
        && !codex_prompt_surface
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && mounted_entry_host_connected
        && remote_codex_prompt_has_real_output
        && terminal_observe_prompt_layout_is_acceptable(rows, blank_rows_below_cursor)
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text);
    let retained_replay_from_pty = matches!(
        terminal_content_source,
        "active_recovery_pty_snapshot"
            | "daemon_pty"
            | "daemon_terminal_read"
            | "daemon_retained_history_screen_snapshot"
    ) || matches!(
        retained_replay_source,
        "active_recovery_pty_snapshot"
            | "daemon_retained_snapshot"
            | "daemon_retained_history_screen_snapshot"
            | "daemon_terminal_read"
            | "daemon_screen_snapshot"
    );
    let live_remote_daemon_pty_current_output_surface = session_path
        .starts_with("remote-session://")
        && terminal_content_source == "daemon_pty"
        && !host_stdin_enabled
        && mounted_entry_host_connected
        && remote_codex_prompt_has_real_output
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text)
        && !terminal_chunk_is_saved_transcript_prefill(visible_text)
        && !terminal_chunk_is_low_signal_terminal_noise(visible_text);
    let last_raw_payload_is_control_only = last_raw_payload_length > 0
        && last_raw_payload_line_count == 0
        && terminal_chunk_printable_signal_count(last_raw_payload_sample) < 3;
    let live_remote_current_frame_has_prompt_signal =
        !cursor_line_text.is_empty() || !last_raw_payload_is_control_only;
    let live_remote_retained_pty_prompt_follow_surface = session_path
        .starts_with("remote-session://")
        && retained_replay_prompt_follow_ready
        && retained_replay_from_pty
        && mounted_entry_host_connected
        && remote_codex_prompt_has_real_output
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && (terminal_chunk_has_prompt_output(visible_text)
            || terminal_chunk_has_codex_prompt_output(visible_text)
            || sample_has_prompt_output
            || sample_has_codex_prompt_output)
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text);
    let live_remote_daemon_pty_scrollback_prompt_surface = session_path
        .starts_with("remote-session://")
        && retained_replay_from_pty
        && mounted_entry_host_connected
        && remote_codex_prompt_has_real_output
        && (base_y > 0 || last_raw_payload_line_count > rows.saturating_add(4))
        && (terminal_observe_prompt_layout_is_acceptable(
            rows,
            blank_rows_below_cursor.saturating_sub(2),
        ) || terminal_observe_codex_prompt_tail_has_real_scrollback(
            cursor_line_text,
            visible_text,
        ))
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && (cursor_sample_visible || cursor_geometry_visible)
        && live_remote_current_frame_has_prompt_signal
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text);
    let live_remote_codex_prompt_surface = session_path.starts_with("remote-session://")
        && (codex_prompt_surface
            || live_remote_codex_prompt_only_surface
            || live_remote_codex_retained_prompt_surface
            || live_remote_retained_pty_prompt_follow_surface
            || live_remote_daemon_pty_scrollback_prompt_surface)
        && mounted_entry_host_connected
        && (xterm_present || screen_present || rows_present || canvas_count > 0);
    let live_remote_codex_interrupted_input_surface = session_path.starts_with("remote-session://")
        && codex_interrupted_input_surface
        && mounted_entry_host_connected
        && host_stdin_enabled
        && (helper_textarea_focused || cursor_sample_visible)
        && (xterm_present || screen_present || rows_present || canvas_count > 0);
    let local_codex_welcome_with_active_cursor_surface = session_path.starts_with("local://")
        && terminal_chunk_is_generic_codex_idle(visible_text)
        && mounted_entry_host_connected
        && host_stdin_enabled
        && (helper_textarea_focused || host_has_active_element || cursor_sample_visible)
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && (last_raw_payload_length > 0 || write_command_count > 0);
    let transcript_browser_surface = terminal_chunk_is_transcript_browser(visible_text);
    let last_stream_output_after_input_ms = last_write_queued_at_ms
        .max(last_write_flush_started_at_ms)
        .max(last_write_callback_at_ms);
    let input_batch_fields_present = host.get("pending_input_bytes").is_some()
        || host.get("input_batch_flush_count").is_some()
        || host.get("last_input_batch_at_ms").is_some();
    let input_delivered_to_daemon_bridge = if input_batch_fields_present {
        input_batch_flush_count > 0 || last_input_batch_at_ms > 0
    } else {
        last_stream_output_after_input_ms > 0
    };
    let accepted_input_without_following_stream_echo = session_path
        .starts_with("remote-session://")
        && host_stdin_enabled
        && helper_textarea_focused
        && mounted_entry_host_connected
        && user_data_event_count > 0
        && pending_input_bytes == 0
        && input_delivered_to_daemon_bridge
        && last_stream_output_after_input_ms > 0
        && last_data_event_at_ms > last_stream_output_after_input_ms.saturating_add(250)
        && last_data_event_at_ms > 0
        && !terminal_observe_prompt_layout_is_acceptable(rows, blank_rows_below_cursor)
        && (terminal_chunk_has_codex_prompt_output(visible_text)
            || sample_has_codex_prompt_output
            || terminal_chunk_is_codex_prompt_surface(visible_text)
            || codex_interrupted_input_surface)
        && (xterm_present || screen_present || rows_present || canvas_count > 0);
    let remote_codex_sparse_prompt_missing_welcome = session_path.starts_with("remote-session://")
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && !codex_prompt_surface
        && !codex_interactive_setup_prompt
        && rows >= 24
        && blank_rows_below_cursor > rows / 2
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text);
    let visible_text_prompt = terminal_chunk_has_prompt_output(visible_text)
        || terminal_chunk_has_codex_prompt_output(visible_text)
        || sample_has_prompt_output
        || sample_has_codex_prompt_output;
    let prompt_visible = terminal_chunk_has_prompt_output(text_sample)
        || terminal_chunk_has_codex_prompt_output(text_sample)
        || visible_text_prompt
        || codex_interactive_setup_prompt
        || codex_prompt_surface
        || (!cursor_line_text.is_empty()
            && (terminal_chunk_has_prompt_output(cursor_line_text)
                || terminal_chunk_has_codex_prompt_output(cursor_line_text)));
    let local_prompt_surface = session_path.starts_with("local://") && prompt_visible;
    // Focus-INDEPENDENT prompt-ready signal. A *current* codex input row (a "›"
    // prompt line followed by the model/status footer — see
    // terminal_chunk_has_current_codex_input_row) is proof codex is sitting at its
    // live prompt, whether or not THIS host currently holds input focus. The
    // content read is reliable even when unfocused (the confidence-gated abstain
    // below relies on the same fact), so prompt-readiness must NOT be withheld just
    // because the focus / cursor-geometry signals are absent. Without this, a
    // healthy but unfocused codex prompt fails every focus-gated branch of
    // prompt_ready_surface and is misread as "only showing a plain shell prompt"
    // (the 2026-06-04 false positive). Gated to a mounted, rendered remote surface.
    let current_codex_input_row_present = session_path.starts_with("remote-session://")
        && (terminal_chunk_has_current_codex_input_row(cursor_line_text)
            || terminal_chunk_has_current_codex_input_row(visible_text)
            || sample_has_current_codex_input_row)
        && mounted_entry_host_connected
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_is_transport_error(visible_text);
    let prompt_ready_surface = live_remote_codex_interrupted_input_surface
        || local_codex_welcome_with_active_cursor_surface
        || current_codex_input_row_present
        || (prompt_visible
            && (host_stdin_enabled
                || helper_textarea_focused
                || cursor_sample_visible
                || local_prompt_surface
                || live_remote_codex_prompt_surface));
    let current_prompt_cursor_visible = !cursor_line_text.is_empty()
        && (terminal_chunk_has_prompt_output(cursor_line_text)
            || terminal_chunk_has_codex_prompt_output(cursor_line_text)
            || terminal_chunk_is_codex_interrupted_input_surface(cursor_line_text))
        && (cursor_sample_visible || cursor_geometry_visible);
    let prompt_ready_current_screen_without_scrollback = session_path
        .starts_with("remote-session://")
        && prompt_ready_surface
        && current_prompt_cursor_visible
        && base_y == 0
        && xterm_buffer_kind != "alternate"
        && (xterm_present || screen_present || rows_present || canvas_count > 0);
    let prompt_ready_retained_unsafe_skip = false
        && retained_replay_unsafe_skip_prompt_ready
        && retained_replay_prompt_follow_ready
        && prompt_ready_surface
        && !cursor_line_text.is_empty()
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text);
    let transcript_browser_ready_surface = false;
    let remote_prompt_input_gated_after_user_input = session_path.starts_with("remote-session://")
        && !host_stdin_enabled
        && !raw_input_enabled
        && (document_focused || helper_textarea_focused || host_has_active_element)
        && mounted_entry_host_connected
        && user_data_event_count > 0
        && input_batch_flush_count > 0
        && last_input_batch_length > 0
        && last_pending_input_reason == "queue"
        && (terminal_chunk_has_current_codex_input_row(cursor_line_text)
            || terminal_chunk_has_current_codex_input_row(visible_text))
        && (xterm_present || screen_present || rows_present || canvas_count > 0);
    if !cursor_line_text.is_empty() && terminal_chunk_is_transport_error(cursor_line_text) {
        if terminal_chunk_is_codex_session_not_on_remote(cursor_line_text) {
            return Some(
                "active terminal host shows: saved Codex session no longer on remote machine",
            );
        }
        return Some("active terminal host is showing transport/error output");
    }
    let blank_but_mounted_surface = text_sample.is_empty()
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && (render_event_count > 0
            || data_event_count > 0
            || !cursor_line_text.is_empty()
            || blank_rows_below_cursor > 0
            || !xterm_buffer_kind.is_empty()
            || host_stdin_enabled
            || helper_textarea_focused
            || xterm_cursor_hidden);
    if visible_text.is_empty() {
        if prompt_ready_surface {
            return None;
        }
        if !terminal_host_has_rendered_surface_for_app_control(host) {
            return Some("active terminal host exists but xterm surface is empty");
        }
        if blank_but_mounted_surface {
            return Some("active terminal host exists but xterm surface is empty");
        }
        return None;
    }
    if terminal_chunk_is_transport_error(visible_text) {
        if terminal_chunk_is_codex_session_not_on_remote(visible_text) {
            return Some(
                "active terminal host shows: saved Codex session no longer on remote machine",
            );
        }
        return Some("active terminal host is showing transport/error output");
    }
    if terminal_chunk_is_loading_placeholder(visible_text) {
        return Some("active terminal host is still showing resume placeholder content");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && terminal_content_source == "daemon_retained_history_screen_snapshot"
        && !prompt_ready_surface
    {
        return Some("active remote terminal is input-enabled on retained history replay");
    }
    if remote_prompt_input_gated_after_user_input {
        return Some("active remote terminal prompt is input-gated after user input");
    }
    if session_path.starts_with("remote-session://")
        && terminal_chunk_is_codex_resume_instruction(visible_text)
    {
        return Some("active remote Codex runtime has exited and is showing a resume instruction");
    }
    if terminal_chunk_is_local_codex_scaffold(visible_text) {
        return Some("active terminal host is still showing local Codex scaffold content");
    }
    if terminal_chunk_is_generic_codex_idle(visible_text) && !prompt_ready_surface {
        return Some("active terminal host is still showing generic Codex idle chrome");
    }
    if terminal_chunk_has_generic_codex_idle_footer(visible_text)
        && !terminal_chunk_has_meaningful_output(visible_text)
        && !prompt_ready_surface
    {
        return Some("active terminal host is still showing generic Codex idle footer");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && retained_replay_from_pty
        && (base_y > 0
            || scrollback_expected
            || terminal_chunk_has_codex_prompt_output(visible_text)
            || sample_has_codex_prompt_output)
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && cursor_line_text.is_empty()
        && last_raw_payload_is_control_only
        && !(terminal_chunk_has_current_codex_input_row(visible_text)
            || sample_has_current_codex_input_row)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text)
        && !codex_working_surface
    {
        return Some(
            "active remote Codex prompt surface has stale scrollback but no current prompt",
        );
    }
    if session_path.starts_with("remote-session://")
        && retained_replay_from_pty
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && (terminal_chunk_has_codex_prompt_output(visible_text) || sample_has_codex_prompt_output)
        && cursor_line_text.is_empty()
        && !(terminal_chunk_has_current_codex_input_row(visible_text)
            || sample_has_current_codex_input_row)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text)
        && !codex_working_surface
    {
        return Some("active remote Codex prompt surface has no current input row");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && mounted_entry_host_connected
        && retained_replay_from_pty
        && xterm_buffer_kind != "alternate"
        && cursor_line_text.is_empty()
        && (terminal_chunk_has_meaningful_output(visible_text) || sample_has_meaningful_output)
        && (visible_text.contains('›')
            || terminal_chunk_has_codex_prompt_output(visible_text)
            || sample_has_codex_prompt_output
            || terminal_chunk_is_codex_prompt_surface(visible_text)
            || terminal_chunk_has_generic_codex_idle_footer(visible_text))
        && !(terminal_chunk_has_current_codex_input_row(visible_text)
            || sample_has_current_codex_input_row)
        && !codex_interactive_setup_prompt
        && !codex_interrupted_input_surface
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text)
        && !codex_working_surface
    {
        return Some("active remote Codex prompt surface has no current input row");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && mounted_entry_host_connected
        && retained_replay_from_pty
        && cursor_line_text.is_empty()
        && rows >= 8
        && last_raw_payload_line_count > rows.saturating_add(4)
        && blank_rows_below_cursor > terminal_observe_max_blank_rows_below_live_cursor(rows)
        && !(terminal_chunk_has_current_codex_input_row(visible_text)
            || sample_has_current_codex_input_row)
        && !terminal_chunk_is_transport_error(visible_text)
        && !terminal_chunk_is_loading_placeholder(visible_text)
        && !terminal_chunk_is_transcript_browser(visible_text)
        && !terminal_chunk_is_generic_codex_idle(visible_text)
        && !codex_working_surface
    {
        return Some("active remote Codex prompt surface has no current input row");
    }
    if (terminal_chunk_has_prompt_output(visible_text)
        || terminal_chunk_has_codex_prompt_output(visible_text)
        || sample_has_prompt_output
        || sample_has_codex_prompt_output)
        && !prompt_ready_surface
    {
        if !session_path.starts_with("remote-session://")
            && !(terminal_chunk_has_codex_prompt_output(visible_text)
                || sample_has_codex_prompt_output)
        {
            return None;
        }
        return Some("active terminal host is only showing a plain shell prompt");
    }
    if session_path.starts_with("remote-session://")
        && remote_codex_sparse_prompt_missing_welcome
        && !accepted_input_without_following_stream_echo
    {
        return Some("active remote Codex prompt surface is missing the welcome frame");
    }
    if transcript_browser_surface {
        return Some("active terminal host is still showing the transcript browser");
    }
    if accepted_input_without_following_stream_echo {
        return Some(
            "active remote terminal accepted input without a following daemon stream echo",
        );
    }
    // XTERM-BUG: resume-gate-too-restrictive
    // See docs/xterm-bugs.md#resume-gate-too-restrictive
    //
    // The "no prompt-ready surface" gate must NOT fire when the visible
    // surface is showing meaningful PTY output. A session in the middle of
    // a long-running command (pytest run, Codex agent reply, etc.) won't
    // have a prompt visible, but it's a healthy live session — gating it
    // costs 60–160s of spurious recovery loops before the user can use it.
    //
    // The remaining conditions still catch the real fault case: input
    // enabled, daemon PTY connected, surface non-empty, NOT in a prompt
    // pattern, NOT in a recognized non-prompt activity (transport error,
    // loading, transcript browser, idle chrome), AND no meaningful PTY
    // output to justify the prompt-less view.
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && mounted_entry_host_connected
        && !prompt_ready_surface
        && !cursor_line_text.is_empty()
        && !terminal_chunk_has_prompt_output(cursor_line_text)
        && !terminal_chunk_has_codex_prompt_output(cursor_line_text)
        && !terminal_chunk_is_codex_interrupted_input_surface(cursor_line_text)
        && !terminal_chunk_is_transport_error(cursor_line_text)
        && !terminal_chunk_is_loading_placeholder(cursor_line_text)
        && !terminal_chunk_is_transcript_browser(cursor_line_text)
        && !terminal_chunk_is_generic_codex_idle(cursor_line_text)
        && (terminal_content_source == "daemon_pty" || retained_replay_from_pty)
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && !terminal_chunk_has_meaningful_output(visible_text)
        && !sample_has_meaningful_output
    {
        return Some("active remote terminal is input-enabled without a prompt-ready surface");
    }
    if terminal_chunk_is_saved_transcript_prefill(visible_text) {
        return Some("active terminal host is still showing saved transcript prefill");
    }
    if terminal_chunk_is_launcher_boilerplate(visible_text) {
        return Some("active terminal host is still showing launcher boilerplate");
    }
    if terminal_chunk_is_low_signal_terminal_noise(visible_text) {
        return Some("active terminal host is still showing low-signal terminal noise");
    }
    if session_path.starts_with("remote-session://")
        && (terminal_content_source.contains("server_prompt")
            || retained_replay_source.contains("server_prompt"))
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
    {
        return Some("active remote terminal is showing non-PTY server snapshot content");
    }
    if session_path.starts_with("remote-session://")
        && !host_stdin_enabled
        && mounted_entry_host_connected
        && (xterm_present || screen_present || rows_present || canvas_count > 0)
        && (last_raw_payload_length > 0 || write_command_count > 0)
        && !prompt_ready_surface
        && !transcript_browser_ready_surface
        && !live_remote_daemon_pty_current_output_surface
    {
        return Some(
            "active remote terminal is showing stale retained text before prompt-ready surface",
        );
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && scrollback_expected
        && !prompt_ready_retained_unsafe_skip
        && (!prompt_ready_surface || retained_replay_unsafe_skip_prompt_ready)
        && rows >= 8
        && cols >= 20
        && base_y == 0
        && xterm_buffer_kind != "alternate"
    {
        return Some("active remote terminal lost expected scrollback after retained replay");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && mounted_entry_host_connected
        && retained_replay_from_pty
        && wheel_event_count > 0
        && !prompt_ready_retained_unsafe_skip
        && !terminal_observe_prompt_layout_is_acceptable(rows, blank_rows_below_cursor)
        && rows >= 8
        && cols >= 20
        && base_y == 0
        && xterm_buffer_kind != "alternate"
        && !prompt_ready_current_screen_without_scrollback
        && terminal_observe_codex_prompt_tail_has_real_scrollback(cursor_line_text, visible_text)
    {
        return Some("active remote terminal received scroll input but has no xterm scrollback");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && last_raw_payload_line_count > rows.saturating_add(4)
        && !prompt_ready_retained_unsafe_skip
        && (!prompt_ready_surface || retained_replay_unsafe_skip_prompt_ready)
        && rows >= 8
        && cols >= 20
        && base_y == 0
        && xterm_buffer_kind != "alternate"
    {
        return Some("active remote terminal accepted multi-row replay without scrollback");
    }
    if session_path.starts_with("remote-session://")
        && host_stdin_enabled
        && (helper_textarea_focused || host_has_active_element)
        && !prompt_ready_surface
        && !transcript_browser_ready_surface
        && !live_remote_daemon_pty_current_output_surface
        // XTERM-BUG: resume-gate-too-restrictive
        // See docs/xterm-bugs.md#resume-gate-too-restrictive
        // Same exemption as the earlier site: a session with meaningful
        // PTY output is a healthy live session even without a prompt visible.
        && !terminal_chunk_has_meaningful_output(visible_text)
        && !sample_has_meaningful_output
    {
        return Some("active remote terminal is input-enabled without a prompt-ready surface");
    }
    None
}

fn terminal_host_dom_paint_problem_is_titlebar_clip(host: &Value) -> bool {
    let Some(paint) = host.get("dom_paint_hit_test").and_then(Value::as_object) else {
        return false;
    };
    if paint
        .get("row_sample_covered_by_shell_chrome")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    let cursor_row_visible = paint
        .get("cursor_row_top_within_rows")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || paint
            .get("cursor_sample_top_within_rows")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if !cursor_row_visible {
        return false;
    }
    let Some(stack) = paint.get("row_sample_stack").and_then(Value::as_array) else {
        return false;
    };
    for node in stack {
        if node
            .get("within_host")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || node
                .get("within_rows")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            break;
        }
        if paint_stack_node_is_yggterm_chrome(node) {
            return true;
        }
    }
    false
}

fn paint_stack_node_is_yggterm_chrome(node: &Value) -> bool {
    let class_name = node.get("class_name").and_then(Value::as_str).unwrap_or("");
    let text = node.get("text").and_then(Value::as_str).unwrap_or("");
    let z_index = node.get("z_index").and_then(Value::as_str).unwrap_or("");
    class_name.contains("yggterm-titlebar")
        || class_name.contains("yggterm-chrome")
        || text.contains("Web ViewTerminal")
        || text.contains("Search live sessions")
        || (z_index == "211" && text.contains('▾'))
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
    if host_left < -1.0 || host_top < -1.0 {
        return Some("active terminal host is mounted offscreen");
    }
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
    let helper_textarea_present = host
        .get("helper_textarea_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let host_stdin_enabled = host
        .get("host_stdin_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let helper_textarea_opacity = host
        .get("helper_textarea_opacity")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let helper_textarea_background = host
        .get("helper_textarea_background")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let helper_textarea_outline_style = host
        .get("helper_textarea_outline_style")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let helper_textarea_outline_color = host
        .get("helper_textarea_outline_color")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let helper_textarea_box_shadow = host
        .get("helper_textarea_box_shadow")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let helper_textarea_clip_path = host
        .get("helper_textarea_clip_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let helper_textarea_clip = host
        .get("helper_textarea_clip")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let helper_textarea_pointer_events = host
        .get("helper_textarea_pointer_events")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let scrollback_locked = host
        .get("scrollback_locked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let scrollback_intent = host
        .get("scrollback_intent")
        .and_then(Value::as_str)
        .unwrap_or("PromptFollow");
    let user_scrollback_locked = scrollback_locked && scrollback_intent == "UserScrollback";
    let base_y = host
        .get("base_y")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 0.0);
    let viewport_y = host
        .get("viewport_y")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 0.0);
    let public_viewport_y = host
        .get("public_viewport_y")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 0.0);
    let visual_viewport_y = host
        .get("visual_viewport_y")
        .and_then(Value::as_f64)
        .filter(|value| *value >= 0.0);
    let viewport_beyond_xterm_base = base_y.is_some_and(|base_y| {
        [viewport_y, public_viewport_y, visual_viewport_y]
            .into_iter()
            .flatten()
            .any(|viewport_y| viewport_y > base_y + 1.0)
    });
    let scrollback_lock_is_stale_at_bottom = scrollback_locked
        && !user_scrollback_locked
        && base_y.is_some_and(|base_y| {
            [viewport_y, public_viewport_y, visual_viewport_y]
                .into_iter()
                .flatten()
                .any(|viewport_y| viewport_y + 0.5 >= base_y)
        });
    let fit_overflow_px = host
        .get("fit_overflow_px")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let prompt_follow_force_matched_bottom = {
        let debug = host.get("last_viewport_force_debug");
        let matched = debug
            .and_then(|value| value.get("matched_target"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let target = debug
            .and_then(|value| value.get("target_viewport_y"))
            .and_then(Value::as_f64);
        let after = debug
            .and_then(|value| {
                value
                    .get("after_effective_viewport_y")
                    .or_else(|| value.get("after_viewport_y"))
            })
            .and_then(Value::as_f64);
        matched
            && fit_overflow_px <= 0.05
            && base_y.is_some_and(|base_y| target.is_some_and(|target| target + 0.5 >= base_y))
            && target
                .zip(after)
                .is_some_and(|(target, after)| (target - after).abs() <= 1.0)
            && base_y.is_some_and(|base_y| {
                [viewport_y, public_viewport_y, visual_viewport_y]
                    .into_iter()
                    .flatten()
                    .any(|viewport_y| viewport_y + 0.5 >= base_y)
            })
    };
    let cursor_expected_top = host
        .get("cursor_expected_rect")
        .and_then(|value| value.get("top"))
        .and_then(Value::as_f64);
    let cursor_expected_height = host
        .get("cursor_expected_rect")
        .and_then(|value| value.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let host_bottom = host
        .get("host_rect")
        .and_then(|value| value.get("bottom"))
        .and_then(Value::as_f64)
        .unwrap_or(host_top + host_outer_height);
    let width_delta = (host_width - screen_width).abs();
    let viewport_spans_host = host_width >= 240.0
        && viewport_width >= 200.0
        && (host_width - viewport_width).abs() <= 4.0;
    let compensated_screen_width_gap = host_width >= 240.0
        && screen_width >= 200.0
        && viewport_width >= 200.0
        && width_delta > 12.0
        && width_delta <= 28.0
        && viewport_spans_host
        && (helpers_width < 1.0 || (screen_width - helpers_width).abs() <= 4.0);
    if host_width >= 240.0
        && screen_width >= 200.0
        && width_delta > 12.0
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
    if viewport_beyond_xterm_base {
        return Some("active terminal viewport is beyond the xterm scrollback base");
    }
    if scrollback_locked && !user_scrollback_locked && !scrollback_lock_is_stale_at_bottom {
        return Some("active terminal prompt-follow viewport is stuck in scrollback");
    }
    if !user_scrollback_locked
        && cursor_expected_height >= 8.0
        && let Some(cursor_top) = cursor_expected_top
    {
        let cursor_bottom = cursor_top + cursor_expected_height;
        if host_bottom >= 120.0
            && cursor_bottom > host_bottom + 0.5
            && !prompt_follow_force_matched_bottom
        {
            return Some("active terminal cursor row is clipped below the visible host");
        }
    }
    if !user_scrollback_locked && fit_overflow_px > 0.05 {
        return Some("active terminal xterm rows exceed the visible host height");
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
    if helper_textarea_present && host_stdin_enabled {
        let tiny_helper = helper_textarea_width >= 1.0
            && helper_textarea_width <= 2.0
            && helper_textarea_height >= 1.0
            && helper_textarea_height <= 2.0;
        let visually_null = helper_textarea_opacity == "0"
            && helper_textarea_outline_style == "none"
            && helper_textarea_pointer_events == "none"
            && helper_textarea_box_shadow == "none"
            && (helper_textarea_background == "transparent"
                || helper_textarea_background == "rgba(0, 0, 0, 0)")
            && (helper_textarea_outline_color.is_empty()
                || helper_textarea_outline_color == "transparent"
                || helper_textarea_outline_color == "rgba(0, 0, 0, 0)")
            && (helper_textarea_clip_path.contains("inset(50%)")
                || helper_textarea_clip.contains("rect(0px, 0px, 0px, 0px)")
                || helper_textarea_clip.contains("rect(0, 0, 0, 0)"));
        let hidden_offscreen = helper_textarea_left <= (host_left - 1000.0)
            && (helper_textarea_top - host_top).abs() <= 32.0;
        if !visually_null {
            return Some(
                "active terminal host helper textarea is visibly mounted instead of visually hidden",
            );
        }
        if !tiny_helper || !hidden_offscreen {
            return Some(
                "active terminal host helper textarea drifted outside the expected hidden contract",
            );
        }
    }
    None
}

fn terminal_host_performance_problem_for_app_control(host: &Value) -> Option<&'static str> {
    let host_stdin_enabled = host
        .get("host_stdin_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let terminal_has_effective_input_focus = host
        .get("effective_input_focus")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            let helper_textarea_focused = host
                .get("helper_textarea_focused")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let host_has_active_element = host
                .get("host_has_active_element")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let document_focused = host
                .get("document_focused")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            host_stdin_enabled
                && (helper_textarea_focused || host_has_active_element || document_focused)
        });
    let active_visible_terminal = host_stdin_enabled
        && terminal_has_effective_input_focus
        && host
            .get("viewport_present")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && host
            .get("screen_present")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let write_budget_fields_present = host.get("active_write_frame_budget").is_some()
        || host.get("effective_terminal_write_frame_ms").is_some()
        || host.get("terminal_active_write_frame_ms").is_some();
    let write_budget_should_be_fast = host
        .get("terminal_input_hot")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || host
            .get("recent_frame_like_write_hot")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || host
            .get("recent_inline_status_animation_hot")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let inline_animation_hot = host
        .get("recent_inline_status_animation_hot")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let terminal_input_hot = host
        .get("terminal_input_hot")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if active_visible_terminal && write_budget_fields_present && write_budget_should_be_fast {
        let active_write_frame_budget = host
            .get("active_write_frame_budget")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let effective_frame_ms = host
            .get("effective_terminal_write_frame_ms")
            .and_then(Value::as_f64);
        if !active_write_frame_budget {
            return Some("active visible terminal is using the background write budget");
        }
        let max_expected_frame_ms = if !terminal_input_hot && inline_animation_hot {
            550.0
        } else {
            220.0
        };
        if effective_frame_ms.is_none_or(|value| value > max_expected_frame_ms) {
            return Some("active visible terminal write budget is too slow");
        }
    }
    None
}

pub(crate) fn terminal_chunk_has_meaningful_output(data: &str) -> bool {
    if terminal_chunk_is_generic_codex_idle(data)
        || terminal_chunk_is_transcript_browser(data)
        || terminal_chunk_is_loading_placeholder(data)
        || terminal_chunk_is_local_codex_scaffold(data)
        || terminal_chunk_is_codex_resume_instruction(data)
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

pub(crate) fn terminal_chunk_is_codex_resume_instruction(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.contains("to continue this session, run codex resume")
        || normalized.contains("run codex resume ")
}

pub(crate) fn terminal_chunk_is_local_codex_scaffold(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }
    let mut saw_scaffold = false;
    for line in lines {
        let normalized = line.to_ascii_lowercase();
        let is_scaffold = normalized == "local codex terminal."
            || normalized.starts_with("> this codex session stays attached to the daemon")
            || normalized.contains("this codex session stays attached to the daemon")
            || normalized.contains("codex is launched locally and will receive /quit")
            || normalized.starts_with("open live terminal ")
            || normalized.starts_with("launch command prepared:");
        if is_scaffold {
            saw_scaffold = true;
            continue;
        }
        return false;
    }
    saw_scaffold
}

pub(crate) fn terminal_chunk_is_codex_interactive_setup_prompt(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return false;
    }
    let codex_context = normalized.contains("openai codex")
        || normalized.contains("welcome to codex")
        || normalized.contains("codex can read and edit files")
        || normalized.contains("codex can edit files outside this workspace")
        || normalized.contains("update model permissions");
    let complete_authentication_menu = (normalized.contains("sign in with chatgpt")
        || normalized.contains("sign in with device code")
        || normalized.contains("provide your own api key"))
        && (normalized.contains("usage-based billing")
            || normalized.contains("paid plan")
            || normalized.contains("press enter to continue"));
    let truncated_authentication_menu = normalized.contains("sign in with device code")
        && normalized.contains("provide your own api key")
        && normalized.contains("press enter to continue")
        && (normalized.contains("usage included with plus")
            || normalized.contains("sign in from another device")
            || normalized.contains("pay for what you use"));
    let authentication_menu = complete_authentication_menu || truncated_authentication_menu;
    let permissions_menu = normalized.contains("update model permissions")
        || (normalized.contains("default (current)")
            && normalized.contains("auto-review")
            && normalized.contains("full access"))
        || (normalized.contains("full access")
            && normalized.contains("exercise caution when using"));
    let explicit_input = normalized.contains("press enter to confirm")
        || normalized.contains("enter to confirm")
        || normalized.contains("press enter to continue")
        || normalized.contains("esc to go back");
    (codex_context || truncated_authentication_menu)
        && explicit_input
        && (permissions_menu || authentication_menu)
}

fn terminal_chunk_is_codex_interrupted_input_surface(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let mut tail_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(6)
        .collect::<Vec<_>>();
    tail_lines.reverse();
    let tail = tail_lines.join(" ").to_ascii_lowercase();
    tail.contains("conversation interrupted - tell the model what to do differently")
}

/// A codex surface ACTIVELY WORKING: its footer shows the interruptible-working
/// indicator ("esc to interrupt", as in "• Working (3s · esc to interrupt)" or
/// "• Booting MCP server… (0s · esc to interrupt)"). While codex is working it
/// legitimately replaces its `›` composer/input row with the working line and may
/// transiently clear/repaint — that is a HEALTHY busy state, NOT a faulted
/// surface. Surface-health must NOT report a "no current input row" / "stale
/// scrollback but no current prompt" / "empty surface" problem for a working
/// codex, because doing so trips retained-fault-recovery into a needless REMOUNT
/// that reseeds xterm from a one-screen snapshot and DESTROYS the session's
/// scrollback mid-turn (the user-visible "yggterm destroys the scrollback buffer";
/// see finding-codex-scroll-lock-no-client-scrollback).
pub(crate) fn terminal_chunk_is_codex_working_surface(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lower = stripped.to_ascii_lowercase();
    // "esc to interrupt" is codex's canonical interruptible-working footer token.
    // It is present ONLY while a turn is in flight (Working / Booting / tool calls);
    // an idle composer never shows it. Matching the token anywhere in the visible
    // text is sufficient and robust to the surrounding "(Ns · …)" formatting.
    lower.contains("esc to interrupt")
}

pub(crate) fn terminal_chunk_is_codex_prompt_surface(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized = stripped
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty()
        || terminal_chunk_is_transport_error(&stripped)
        || terminal_chunk_is_loading_placeholder(&stripped)
        || terminal_chunk_is_transcript_browser(&stripped)
        || terminal_chunk_has_generic_codex_idle_footer(&stripped)
    {
        return false;
    }
    let has_codex_header =
        stripped.contains("OpenAI Codex") && stripped.contains("/model to change");
    has_codex_header
        && terminal_chunk_has_codex_prompt_output(&stripped)
        && (normalized.contains("model:")
            || normalized.contains("directory:")
            || normalized.contains("gpt-")
            || normalized.contains("claude"))
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

/// Recognize a Claude Code prompt/ready surface.
///
/// The remote retained-replay readiness gate was historically Codex-only
/// (`terminal_chunk_is_codex_prompt_surface`, `…_has_codex_prompt_output`,
/// `…_is_codex_interactive_setup_prompt`), so a resumed Claude Code session's
/// retained snapshot was judged "non-prompt / not replayable" and the viewport
/// blanked on mount/remount. Claude Code is a first-class `SessionKind` but its
/// prompt shape was never taught to this layer. See
/// docs/xterm-bugs.md#remote-cc-replay-codex-only and memory
/// finding-remote-cc-retained-replay-codex-only.
///
/// Markers are kept Claude-specific (low false-positive against shell/Codex
/// surfaces): the idle input-box footer "? for shortcuts", or a selection
/// prompt that pairs Claude's `❯` caret with a permission affordance
/// ("Do you want", "Yes, allow", "Tab to", "esc to", or an explicit
/// yes/no option set).
pub(crate) fn terminal_chunk_is_claude_prompt_surface(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let trimmed = stripped.trim();
    if trimmed.is_empty()
        || terminal_chunk_is_transport_error(&stripped)
        || terminal_chunk_is_loading_placeholder(&stripped)
        || terminal_chunk_is_transcript_browser(&stripped)
    {
        return false;
    }
    let normalized = stripped.to_ascii_lowercase();
    let idle_footer = normalized.contains("? for shortcuts");
    let has_selection_caret = stripped.contains('❯');
    let permission_prompt = has_selection_caret
        && (normalized.contains("do you want")
            || normalized.contains("yes, allow")
            || normalized.contains("tab to ")
            || normalized.contains("esc to ")
            || (normalized.contains("yes") && normalized.contains("no")));
    idle_footer || permission_prompt
}

pub(crate) fn terminal_chunk_has_current_codex_input_row(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '))
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let Some(prompt_index) = lines.iter().rposition(|line| line.starts_with('›')) else {
        return false;
    };
    if lines.len().saturating_sub(prompt_index) > 5 {
        return terminal_chunk_has_wrapped_codex_input_region(&lines, prompt_index);
    }
    terminal_chunk_has_wrapped_codex_input_region(&lines, prompt_index)
        || lines[prompt_index + 1..].iter().all(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("gpt-")
                || lower.contains("claude")
                || lower.starts_with("tab to ")
                || lower.starts_with("ctrl")
                || lower.starts_with("esc")
        })
}

pub(crate) fn terminal_chunk_has_codex_prompt_output(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let normalized_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if normalized_lines.is_empty() {
        return false;
    }
    let tail = normalized_lines
        .iter()
        .rev()
        .take(4)
        .copied()
        .collect::<Vec<_>>();
    if tail.iter().all(|line| line.chars().count() <= 160)
        && tail.iter().any(|line| {
            let semantic =
                line.trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '));
            semantic.starts_with('›')
        })
    {
        return true;
    }
    if let Some(prompt_index) = normalized_lines.iter().rposition(|line| {
        let semantic =
            line.trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '));
        semantic.starts_with('›')
    }) {
        if terminal_chunk_has_wrapped_codex_input_region(&normalized_lines, prompt_index) {
            return true;
        }
    }
    if terminal_chunk_is_transport_error(&stripped)
        || terminal_chunk_is_loading_placeholder(&stripped)
        || terminal_chunk_is_transcript_browser(&stripped)
        || terminal_chunk_is_saved_transcript_prefill(&stripped)
    {
        return false;
    }
    let compact = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(prompt_ix) = compact.rfind('›') else {
        return false;
    };
    let prompt_suffix = compact[prompt_ix..]
        .trim_matches(|ch: char| matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | ' '));
    prompt_suffix.starts_with('›')
        && prompt_suffix.chars().count() <= 260
        && (prompt_suffix.contains("gpt-")
            || prompt_suffix.contains("claude")
            || prompt_suffix.contains("~/")
            || prompt_suffix.chars().count() <= 140)
}

fn terminal_chunk_has_wrapped_codex_input_region(lines: &[&str], prompt_index: usize) -> bool {
    let suffix = &lines[prompt_index + 1..];
    if suffix.is_empty() {
        return true;
    }
    if suffix.len() > 10 {
        return false;
    }
    if suffix
        .iter()
        .any(|line| terminal_line_is_obvious_codex_output_after_prompt(line))
    {
        return false;
    }
    let footer_index = suffix
        .iter()
        .rposition(|line| terminal_line_is_codex_footer_or_hint(line));
    if let Some(index) = footer_index {
        return suffix[index..]
            .iter()
            .all(|line| terminal_line_is_codex_footer_or_hint(line));
    }
    suffix
        .iter()
        .all(|line| terminal_line_is_codex_footer_or_hint(line))
}

fn terminal_line_is_codex_footer_or_hint(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.contains("gpt-")
        || lower.contains("claude")
        || lower.contains("% left")
        || lower.starts_with("tab to ")
        || lower.starts_with("ctrl")
        || lower.starts_with("esc")
        || lower.starts_with("press enter to ")
        || lower.starts_with("enter to ")
}

fn terminal_line_is_obvious_codex_output_after_prompt(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with('•')
        || lower.starts_with('■')
        || lower.starts_with("error:")
        || lower.starts_with("warning:")
        || lower.starts_with("assistant:")
        || lower.starts_with("user:")
        || lower.starts_with("worked for ")
        || lower.starts_with("model changed to ")
        || lower.starts_with("permissions updated ")
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
                || lower.contains("resume a previous session")
                || lower.contains("write tests for")
                || lower.contains("@filename")
                || lower.contains("review my changes")
                || lower.contains("summarize recent commits")
                || lower.contains("create a pr"))
    });
    let mentions_model_footer = (normalized.contains("gpt-5")
        || normalized.contains("gpt-4")
        || normalized.contains("claude"))
        && normalized.contains("% left");
    mentions_generic_prompt && mentions_model_footer
}

pub(crate) fn terminal_chunk_is_transcript_browser(data: &str) -> bool {
    if terminal_chunk_is_codex_role_transcript(data) {
        return true;
    }
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

pub(crate) fn terminal_chunk_is_codex_role_transcript(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let mut non_empty = 0usize;
    let mut role_markers = 0usize;
    for line in stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        non_empty += 1;
        let lower = line.to_ascii_lowercase();
        if lower == "user:"
            || lower == "assistant:"
            || lower.starts_with("user: ")
            || lower.starts_with("assistant: ")
        {
            role_markers += 1;
        }
    }
    role_markers >= 2 && non_empty >= 4
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
    if terminal_chunk_is_codex_role_transcript(data) {
        return true;
    }
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

/// Per [[spec-xterm-gating-ux]] / dead-session UX: distinguishes the
/// "saved <agent> session UUID is no longer available on this machine"
/// failure (from `crates/yggterm-server/src/lib.rs` wrapper) from generic
/// transport errors. This is recoverable only by Remove-from-sidebar, not by
/// a retry — so the toast text should differ. Matches the agent-agnostic
/// invariant tail so BOTH the Codex ("saved Codex session …") and Claude Code
/// ("saved Claude Code session …") wrappers classify the same — the recovery
/// (Delete row) is identical, so the failure is one concept, not two.
pub(crate) fn terminal_chunk_is_codex_session_not_on_remote(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let lower: String = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    lower.contains("no longer available on this machine")
        && lower.contains("cannot be restored as a live terminal")
}
pub(crate) fn terminal_chunk_is_transport_error(data: &str) -> bool {
    let stripped = strip_terminal_control_sequences(data);
    let all_lines = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if all_lines.is_empty() {
        return false;
    }
    if all_lines
        .iter()
        .any(|line| terminal_line_is_internal_transport_error(line))
    {
        return true;
    }
    let all_text = all_lines.join(" ");
    let compact_all_text = all_text.split_whitespace().collect::<String>();
    if (all_text.contains("error: connecting to ")
        && all_text.contains("server-")
        && all_text.contains(".sock"))
        || (compact_all_text.contains("error:connectingto")
            && compact_all_text.contains("server-")
            && compact_all_text.contains(".sock"))
    {
        return true;
    }
    let lines = all_lines.iter().take(4).cloned().collect::<Vec<_>>();
    let head = lines.join(" ");
    let compact_head = head.split_whitespace().collect::<String>();
    if head.contains("error: connecting to ") && head.contains("server-") && head.contains(".sock")
        || (compact_head.contains("error:connectingto")
            && compact_head.contains("server-")
            && compact_head.contains(".sock"))
    {
        return true;
    }
    if [
        "[yggterm] terminal reader stopped",
        "error: reading /tmp/yggterm-screen",
        "mux_client_request_session",
        "session open refused by peer",
        "controlsocket",
        "exec: export: not found",
        "exec: __yggterm_initial_tty_size",
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
            && (line.contains(" closed")
                || line.contains("refused")
                || line.contains("timed out")
                || looks_like_incomplete_shared_connection_notice(line)))
            || (line.starts_with("connection to ")
                && (line.contains(" closed")
                    || line.contains("refused")
                    || line.contains("timed out")))
            || line == "permission denied"
            || line == "no route to host"
            || line == "broken pipe"
            || line == "connection reset by peer"
            || ((line.starts_with("ssh:")
                || line.starts_with("scp:")
                || line.starts_with("sftp:")
                || line.starts_with("error:")
                || line.starts_with("fatal:")
                || line.starts_with("rsync:"))
                && (line.contains("permission denied")
                    || (line.contains("connecting to ")
                        && line.contains("server-")
                        && line.contains(".sock"))
                    || line.contains("connection refused")
                    || line.contains("no route to host")
                    || line.contains("connection timed out")
                    || line.contains("broken pipe")))
    })
}

fn looks_like_incomplete_shared_connection_notice(line: &str) -> bool {
    line.starts_with("shared connection to ")
        && !line.contains(" closed")
        && !line.contains(" refused")
        && !line.contains(" timed out")
}

pub(crate) fn terminal_line_is_internal_transport_error(line: &str) -> bool {
    let stripped = strip_terminal_control_sequences(line);
    let normalized = stripped
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, '›' | '>' | ' ' | '\t'))
        .trim()
        .to_ascii_lowercase();
    normalized.starts_with("error: terminal session not found: local://")
        || normalized.starts_with("terminal session not found: local://")
        || normalized.starts_with("error: terminal session not found: remote-session://")
        || normalized.starts_with("terminal session not found: remote-session://")
        || normalized.starts_with("error: terminal session not found: codex-runtime://")
        || normalized.starts_with("terminal session not found: codex-runtime://")
        || normalized.starts_with("error: hot update failed before bridging stale remote runtime")
        || normalized.starts_with("hot update failed before bridging stale remote runtime")
        || normalized.contains("hot update failed before bridging stale remote runtime")
        || normalized.starts_with("warn ignoring stale yggterm daemon for current app version")
        || normalized.contains("warn ignoring stale yggterm daemon for current app version")
        || normalized == "reading daemon response"
        || normalized.contains("error: terminal session not found: local://")
        || normalized.contains("error: terminal session not found: remote-session://")
        || normalized.contains("error: terminal session not found: codex-runtime://")
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
    let normalized_data = if data.contains("\\x1b")
        || data.contains("\\u001b")
        || data.contains("\\u{1b}")
        || data.contains("\\x07")
        || data.contains("\\u0007")
    {
        Some(
            data.replace("\\x1b", "\u{1b}")
                .replace("\\u001b", "\u{1b}")
                .replace("\\u{1b}", "\u{1b}")
                .replace("\\x07", "\u{7}")
                .replace("\\u0007", "\u{7}"),
        )
    } else {
        None
    };
    let data = normalized_data.as_deref().unwrap_or(data);
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

fn terminal_chunk_printable_signal_count(data: &str) -> usize {
    strip_terminal_control_sequences(data)
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .take(3)
        .count()
}

#[cfg(test)]
mod tests {
    // ── Scheme-registry predicate lock (harness spec §2.3/§8 phase 0) ───────
    // The SSOT twin of shell.rs's transport-error index — same expected
    // scheme set, same burn-down table in yggterm-core::agent_scheme.
    #[test]
    fn scheme_registry_lock_terminal_line_is_internal_transport_error() {
        use yggterm_core::agent_scheme::{self, SchemeRole};
        let name = "terminal_line_is_internal_transport_error";
        // Any scheme that can key a terminal session can appear in a daemon
        // "terminal session not found: <key>" error line.
        let in_scope = |s: &agent_scheme::SchemeDescriptor| {
            !s.legacy
                && s.agent
                && matches!(
                    s.role,
                    SchemeRole::RowIdentity
                        | SchemeRole::RuntimeKey
                        | SchemeRole::RowAndRuntimeKey
                )
        };
        for scheme in agent_scheme::SESSION_PATH_SCHEMES.iter().filter(|s| in_scope(s)) {
            let line = format!("Error: terminal session not found: {}", scheme.example);
            let covered = super::terminal_line_is_internal_transport_error(&line);
            let hole = agent_scheme::predicate_hole_allowed(name, scheme.prefix);
            assert!(
                covered || hole,
                "{name} does not excise `{line}` and no hole is recorded — fix it or record it"
            );
            assert!(
                !(covered && hole),
                "STALE HOLE: {name}×{} — delete the KNOWN_PREDICATE_HOLES row",
                scheme.prefix
            );
        }
        for hole in agent_scheme::predicate_holes_for(name) {
            let scheme = agent_scheme::scheme_for_prefix(hole.scheme)
                .expect("hole names a registered scheme");
            assert!(in_scope(scheme), "{name}'s hole row {} out of scope", hole.scheme);
        }
    }

    use super::{
        MemoryPressureSnapshot, RevealLogEntry, TerminalOpenAttempt, TerminalOpenAttemptState,
        WorkspaceViewMode, parse_meminfo,
        describe_terminal_open_attempt, describe_viewport_snapshot,
        summarize_terminal_surface_for_app_control, terminal_bootstrap_activation_epoch,
        terminal_chunk_has_codex_prompt_output, terminal_chunk_has_current_codex_input_row,
        terminal_chunk_has_meaningful_output, terminal_chunk_is_claude_prompt_surface,
        terminal_chunk_is_codex_interactive_setup_prompt,
        terminal_chunk_is_codex_prompt_surface, terminal_chunk_is_codex_session_not_on_remote,
        terminal_chunk_is_codex_working_surface,
        terminal_chunk_is_local_codex_scaffold,
        terminal_chunk_is_transport_error, terminal_host_geometry_problem_for_app_control,
        terminal_host_problem_for_app_control,
        terminal_observe_codex_prompt_tail_has_real_scrollback,
        terminal_observe_prompt_layout_is_acceptable, terminal_timing_for_app_control,
    };
    use serde_json::{Value, json};

    #[test]
    fn saved_agent_session_gone_classifies_both_codex_and_claude_code() {
        // Regression: a Claude Code row whose transcript is gone from the remote
        // must classify as "not on remote" (Delete-to-recover), not as a generic
        // transport error. The wrapper names the correct agent; the detector keys
        // on the agent-agnostic invariant tail so both wrappers land here.
        let codex = "yggterm: saved Codex session 35669b3d-b6c6-4c9f-ad18-bed2e49bb074 \
             is no longer available on this machine, so this row cannot be restored as a live terminal.";
        let claude = "yggterm: saved Claude Code session 35669b3d-b6c6-4c9f-ad18-bed2e49bb074 \
             is no longer available on this machine, so this row cannot be restored as a live terminal.";
        assert!(terminal_chunk_is_codex_session_not_on_remote(codex));
        assert!(terminal_chunk_is_codex_session_not_on_remote(claude));
        // A generic transport error must NOT be misread as a dead-transcript row.
        assert!(!terminal_chunk_is_codex_session_not_on_remote(
            "shared connection to dev closed."
        ));
    }

    #[test]
    fn parse_meminfo_computes_swap_used_and_available() {
        let text = "\
MemTotal:       16384000 kB
MemFree:          512000 kB
MemAvailable:    4096000 kB
Buffers:          100000 kB
SwapTotal:       8388608 kB
SwapFree:        1048576 kB
";
        let snapshot = parse_meminfo(text);
        assert_eq!(snapshot.mem_total_kb, 16_384_000);
        assert_eq!(snapshot.mem_available_kb, 4_096_000);
        assert_eq!(snapshot.swap_total_kb, 8_388_608);
        // used = total - free = 8388608 - 1048576 = 7340032 kB
        assert_eq!(snapshot.swap_used_kb, 7_340_032);
        assert_eq!(snapshot.swap_total_mb(), 8_192);
        assert_eq!(snapshot.swap_used_mb(), 7_168);
        assert_eq!(snapshot.mem_available_mb(), 4_000);
        assert!(snapshot.is_present());
        // 7 GB of swap in use is well past the 512 MB pressure floor.
        assert!(snapshot.swap_pressured());
    }

    #[test]
    fn parse_meminfo_handles_absent_swap_and_garbage() {
        // No swap configured, plus a malformed line that must be skipped.
        let text = "MemTotal: 8000000 kB\nMemAvailable: 6000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\nGarbageLineWithoutColon\nBogus: notanumber kB\n";
        let snapshot = parse_meminfo(text);
        assert_eq!(snapshot.swap_used_kb, 0);
        assert_eq!(snapshot.swap_total_kb, 0);
        assert_eq!(snapshot.mem_available_kb, 6_000_000);
        assert!(snapshot.is_present());
        assert!(!snapshot.swap_pressured());
    }

    #[test]
    fn memory_pressure_snapshot_default_is_absent() {
        let snapshot = MemoryPressureSnapshot::default();
        assert!(!snapshot.is_present());
        assert!(!snapshot.swap_pressured());
        assert_eq!(snapshot.to_json()["present"], json!(false));
    }

    #[test]
    fn reveal_log_entry_serializes_timing_and_pressure() {
        let entry = RevealLogEntry {
            session_path: "local://abc".to_string(),
            label: "codex • repo".to_string(),
            kind: "Codex".to_string(),
            source: "open_row".to_string(),
            tier: "cold".to_string(),
            started_at_ms: 10_000,
            finished_at_ms: 13_400,
            surface_mounted_at_ms: Some(10_900),
            first_output_at_ms: Some(11_250),
            outcome: "ready".to_string(),
            failure_reason: None,
            memory_pressure: MemoryPressureSnapshot {
                swap_used_kb: 9_400 * 1024,
                swap_total_kb: 16_000 * 1024,
                mem_available_kb: 3_500 * 1024,
                mem_total_kb: 16_000 * 1024,
            },
        };
        assert_eq!(entry.total_ms(), 3_400);
        let value = entry.to_json();
        assert_eq!(value["total_ms"], json!(3_400));
        assert_eq!(value["surface_mounted_ms"], json!(900));
        assert_eq!(value["first_output_ms"], json!(1_250));
        assert_eq!(value["tier"], json!("cold"));
        assert_eq!(value["outcome"], json!("ready"));
        assert_eq!(value["memory_pressure"]["swap_used_mb"], json!(9_400));
        assert_eq!(value["memory_pressure"]["swap_pressured"], json!(true));
    }

    #[test]
    fn terminal_open_attempt_description_splits_resume_timing_phases() {
        let attempt = TerminalOpenAttempt {
            attempt_id: "attempt-1".to_string(),
            session_path: "remote-session://dev/session".to_string(),
            request_id: "request-1".to_string(),
            open_request_id: 7,
            source: "open_row".to_string(),
            started_at_ms: 1_000,
            memory_pressure_at_start: MemoryPressureSnapshot::default(),
            cold_at_start: false,
            state: TerminalOpenAttemptState::Ready,
            observations: 3,
            rearm_count: 1,
            ready_at_ms: Some(1_900),
            surface_mounted_at_ms: Some(1_100),
            first_output_at_ms: Some(1_250),
            first_protocol_only_output_at_ms: Some(1_200),
            first_meaningful_output_at_ms: Some(1_600),
            latched_failure_at_ms: None,
            latched_failure_reason: None,
            last_observed_ready: true,
            last_observed_reason: Some("ready".to_string()),
            last_surface_problem: None,
            last_overlay_visible: false,
            last_overlay_kind: None,
            last_overlay_text: None,
        };

        let summary = describe_terminal_open_attempt(&attempt);
        assert_eq!(summary["request_to_surface_mounted_ms"], 100);
        assert_eq!(summary["request_to_first_protocol_only_output_ms"], 200);
        assert_eq!(summary["surface_mounted_to_first_output_ms"], 150);
        assert_eq!(summary["request_to_first_meaningful_output_ms"], 600);
        assert_eq!(
            summary["surface_mounted_to_first_meaningful_output_ms"],
            500
        );
        assert_eq!(summary["first_output_to_ready_ms"], 650);
        assert_eq!(summary["first_meaningful_output_to_ready_ms"], 300);
        assert_eq!(summary["request_to_ready_ms"], 900);
    }

    #[test]
    fn terminal_host_problem_accepts_prompt_ready_codex_footer_surface() {
        let host = json!({
            "text_sample": "› Explain this codebase

  gpt-5.4 high fast · 100% left · ~/git",
            "cursor_line_text": "› Explain this codebase",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn active_terminal_surface_reports_unmounted_xterm_as_empty_surface() {
        let host = json!({
            "session_path": "live::practice-remount",
            "host_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "text_sample": "",
            "text_tail": "",
            "buffer_text_sample": "",
            "cursor_line_text": "",
            "xterm_present": false,
            "screen_present": false,
            "viewport_present": false,
            "rows_present": false,
            "canvas_count": 0,
            "child_count": 0,
            "mounted_entry_host_connected": false,
            "render_event_count": 0,
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
        });

        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host exists but xterm surface is empty")
        );
        let surface = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            surface.get("problem").and_then(Value::as_str),
            Some("active terminal host exists but xterm surface is empty")
        );
        assert_eq!(
            surface.get("rendered").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn terminal_host_problem_rejects_dom_rows_that_fail_hit_test_paint() {
        let host = json!({
            "session_path": "remote-session://practice/paint-hidden",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0},
            "text_sample": "meaningful terminal output\n› prompt",
            "text_tail": "meaningful terminal output\n› prompt",
            "buffer_text_sample": "meaningful terminal output\n› prompt",
            "cursor_line_text": "› prompt",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "child_count": 2,
            "mounted_entry_host_connected": true,
            "render_event_count": 3,
            "data_event_count": 1,
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "dom_paint_hit_test_problem": "xterm row sample is not topmost at its visible text point",
        });

        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal DOM rows are present but not paint-visible")
        );
        let surface = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            surface.get("problem").and_then(Value::as_str),
            Some("active terminal DOM rows are present but not paint-visible")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_titlebar_clipped_top_row_when_cursor_row_is_visible() {
        let host = json!({
            "session_path": "remote-session://oc/titlebar-clip",
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0},
            "host_content_width": 1343.0,
            "host_content_height": 1144.0,
            "screen_rect": {"width": 1343.0, "height": 1144.0},
            "viewport_rect": {"width": 1343.0, "height": 1144.0},
            "helpers_rect": {"width": 1343.0, "height": 1144.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0},
            "text_sample": "- Rust\n- Systems\n\n› Implement {feature}",
            "text_tail": "- Rust\n- Systems\n\n› Implement {feature}",
            "buffer_text_sample": "- Rust\n- Systems\n\n› Implement {feature}",
            "cursor_line_text": "› Implement {feature}",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "child_count": 2,
            "mounted_entry_host_connected": true,
            "render_event_count": 3,
            "data_event_count": 1,
            "host_stdin_enabled": true,
            "effective_input_focus": true,
            "helper_textarea_focused": true,
            "dom_paint_hit_test_problem": "xterm row sample is not topmost at its visible text point",
            "dom_paint_hit_test": {
                "row_sample_covered_by_shell_chrome": false,
                "row_sample_top_within_rows": false,
                "cursor_row_top_within_rows": true,
                "cursor_sample_top_within_rows": true,
                "row_sample_stack": [
                    {
                        "tag": "span",
                        "class_name": "",
                        "text": "gravatar update",
                        "z_index": "auto",
                        "within_host": false,
                        "within_rows": false
                    },
                    {
                        "tag": "button",
                        "class_name": "",
                        "text": "gravatar update▾",
                        "z_index": "211",
                        "within_host": false,
                        "within_rows": false
                    },
                    {
                        "tag": "div",
                        "class_name": "yggterm-titlebar-session-shell",
                        "text": "gravatar update▾",
                        "z_index": "auto",
                        "within_host": false,
                        "within_rows": false
                    }
                ]
            }
        });

        assert_eq!(terminal_host_problem_for_app_control(&host), None);
        let surface = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(surface.get("problem").and_then(Value::as_str), None);
        assert_eq!(
            surface.get("foreground_input_ready").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn terminal_chunk_marks_hot_update_bridge_failure_as_transport_error() {
        let data = "Error: hot update failed before bridging stale remote runtime codex-runtime://019dfde8 from 2.2.49 pid 1629132\r\n\r\nCaused by:\r\n    reading daemon response\r\n    Resource temporarily unavailable (os error 11)\r\n";

        assert!(terminal_chunk_is_transport_error(data));
        assert_eq!(
            terminal_host_problem_for_app_control(&json!({
                "session_path": "remote-session://dev/019dfde8",
                "text_sample": data,
                "xterm_present": true,
                "screen_present": true,
                "rows_present": true,
                "canvas_count": 1,
                "terminal_content_source": "daemon_pty"
            })),
            Some("active terminal host is showing transport/error output")
        );
    }

    #[test]
    fn terminal_chunk_marks_stale_daemon_warning_as_transport_error() {
        let data = "\x1b[33m WARN\x1b[0m ignoring stale yggterm daemon for current app version \x1b[3mstage\x1b[0m=\"initial_status\" \x1b[3mcurrent_version\x1b[0m=\"2.2.52\" \x1b[3mstale_version\x1b[0m=\"2.2.51\"\r\n";

        assert!(terminal_chunk_is_transport_error(data));
        assert_eq!(
            terminal_host_problem_for_app_control(&json!({
                "session_path": "remote-session://dev/019dbdcf",
                "cursor_line_text": data,
                "xterm_present": true,
                "screen_present": true,
                "rows_present": true,
                "canvas_count": 1,
                "terminal_content_source": "daemon_pty"
            })),
            Some("active terminal host is showing transport/error output")
        );
    }

    #[test]
    fn terminal_chunk_marks_stale_yggterm_socket_connect_error_as_transport_error() {
        let data = "╭─────────────────────────────────────────────╮\n\
                                               │ >_ OpenAI Codex (v0.130.0)                  │\n\
                               │ model:     gpt-5.5 xhigh   /model to change │\n\
                                                                              │ directory: ~/git/samplenotes         \n\
               ╰─────────────────────────────────────────────╯\n\
                                                              Error: connecting to /home/pi/.yggterm/server-2-\n\
1-10.sock\n\n\
Caused by:\n\
    No such file or directory (os error 2)";

        assert!(terminal_chunk_is_transport_error(data));
        assert_eq!(
            terminal_host_problem_for_app_control(&json!({
                "session_path": "remote-session://dev/019dbdcf",
                "text_sample": data,
                "text_tail": data,
                "buffer_text_sample": data,
                "xterm_present": true,
                "screen_present": true,
                "rows_present": false,
                "canvas_count": 4,
                "terminal_content_source": "daemon_pty",
                "mounted_entry_host_connected": true,
                "last_raw_payload_length": 117,
                "write_command_count": 2,
                "rows": 50,
                "blank_rows_below_cursor": 35,
                "xterm_buffer_kind": "normal"
            })),
            Some("active terminal host is showing transport/error output")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_prompt_ready_generic_codex_idle_surface() {
        let host = json!({
            "text_sample": "pi@dev:/home/pi$ codex\n╭────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.120.0)                 │\n│                                            │\n│ model:     gpt-5.4 high   /model to change │\n│ directory: /home/pi                        │\n╰────────────────────────────────────────────╯\n\n  Tip: New Use /fast to enable our fastest inference at 2X plan usage.\n\n\n› Implement {feature}\n\n  gpt-5.4 high · /home/pi",
            "cursor_line_text": "› Implement {feature}",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_remote_codex_title_card_without_prompt() {
        let host = json!({
            "session_path": "remote-session://dev/title-card-only",
            "text_sample": "╭─────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.128.0)                  │\n│                                             │\n│ model:     gpt-5.5 xhigh   /model to change │\n│ directory: ~/gh/yggterm                     │",
            "text_tail": "╭─────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.128.0)                  │\n│                                             │\n│ model:     gpt-5.5 xhigh   /model to change │\n│ directory: ~/gh/yggterm                     │",
            "cursor_line_text": "│ directory: ~/gh/yggterm                     │",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 4,
            "render_event_count": 8,
            "write_command_count": 1,
            "last_raw_payload_line_count": 4,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is still showing generic Codex idle chrome")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_local_codex_blank_prompt_with_active_cursor() {
        let text = "╭─────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.128.0)                  │\n│                                             │\n│ model:     gpt-5.5 xhigh   /model to change │\n│ directory: ~                                │\n╰─────────────────────────────────────────────╯";
        let host = json!({
            "session_path": "local://blank-prompt-codex",
            "text_sample": text,
            "text_tail": text,
            "buffer_text_sample": text,
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "xterm_present": true,
            "screen_present": true,
            "canvas_count": 4,
            "mounted_entry_host_connected": true,
            "last_raw_payload_length": 181,
            "write_command_count": 9,
            "rows": 48,
            "blank_rows_below_cursor": 37,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 296.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_codex_permission_menu_surface() {
        let host = json!({
            "session_path": "remote-session://dev/codex-permissions",
            "text_sample": "╭─────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.128.0)                  │\n│                                             │\n│ model:     gpt-5.5 xhigh   /model to change │\n│ directory: ~/gh/yggterm                     │",
            "text_tail": "╭─────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.128.0)                  │\n│                                             │\n│ model:     gpt-5.5 xhigh   /model to change │\n│ directory: ~/gh/yggterm                     │\n╰─────────────────────────────────────────────╯\n\n  Tip: You can run any shell command from Codex using ! (e.g. !ls)\n\n\n  Update Model Permissions\n\n› 1. Default (current)  Codex can read and edit files in the current workspace, and run commands. Approval\n                        is required to access the internet or edit other files.\n  2. Auto-review        Same workspace-write permissions as Default, but eligible `on-request` approvals\n                        are routed through the auto-reviewer subagent.\n  3. Full Access        Codex can edit files outside this workspace and access the internet without asking\n                        for approval. Exercise caution when using.\n\n  Press enter to confirm or esc to go back",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 22,
            "data_event_count": 5,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert!(terminal_chunk_is_codex_interactive_setup_prompt(
            host.get("text_tail").and_then(Value::as_str).unwrap()
        ));
        assert!(terminal_chunk_is_codex_interactive_setup_prompt(
            "uto-reviewer subagent.\n  3. Full Access        Codex can edit files outside this workspace and access the internet without asking\n                        for approval. Exercise caution when using.\n\n  Press enter to confirm or esc to go back"
        ));
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn codex_onboarding_menu_is_interactive_setup_surface() {
        let onboarding = "Welcome to Codex, OpenAI's command-line coding agent\n\n\
  Sign in with ChatGPT to use Codex as part of your paid plan\n\
or connect an API key for usage-based billing\n\n\
› 1. Sign in with ChatGPT\n\
  2. Sign in with Device Code\n\
  3. Provide your own API key\n\n\
Press enter to continue";

        assert!(terminal_chunk_is_codex_interactive_setup_prompt(onboarding));
    }

    #[test]
    fn codex_onboarding_tail_after_logo_truncation_is_interactive_setup_surface() {
        let truncated_tail = "\
tGPT
     Usage included with Plus, Pro, Business, and Enterprise plans

  2. Sign in with Device Code
     Sign in from another device with a one-time code

  3. Provide your own API key
     Pay for what you use

  Press enter to continue";

        assert!(terminal_chunk_is_codex_interactive_setup_prompt(
            truncated_tail
        ));
    }

    #[test]
    fn terminal_host_problem_accepts_basic_snapshot_codex_prompt_row_text() {
        let host = json!({
            "text_sample": "pi@jojo:~$ codex\n╭───────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.124.0)                │\n│                                           │\n│ model:     gpt-5.4 low   /model to change │\n│ directory: ~                              │\n╰───────────────────────────────────────────╯\n\n› Summarize recent commits\n\n  gpt-5.4 low · ~",
            "cursor_row_text": "› Summarize recent commits",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_sample_rect": {"left": 325.0, "top": 256.0, "width": 8.0, "height": 18.0},
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_abstains_on_sparse_read_of_unfocused_rendered_daemon_fed_surface() {
        // Regression: the live false-positive illusion (2026-06-03). A healthy,
        // actively-used remote Codex session that is NOT the foreground input
        // owner read back with an empty/sparse client text buffer (the snapshot
        // was captured on blur, "focus_released"), while the canvas was painting
        // live and the daemon was still feeding bytes. The detector classified
        // that sparse read as "active terminal host is only showing a plain
        // shell prompt" — a false positive on a session the user was scrolling
        // and typing in. With the couldn't-observe guard it must abstain (None).
        //
        // Codex composer paint frame as raw daemon bytes (cursor-addressed redraw
        // with the composer-box background) — proof the daemon is feeding a live
        // surface even though the client text read came back empty.
        let raw_payload = "\\x1b[?2026h\\x1b[25;2H\\x1b[0m\\x1b[49m\\x1b[K\\x1b[26;2H\\x1b[0m\\x1b[48;2;64;67;75m\\x1b[K\\x1b[?2026l";
        let host = json!({
            "session_path": "remote-session://dev/unfocused-codex",
            "text_sample": "",
            "text_tail": "",
            "buffer_text_sample": "",
            "cursor_line_text": "",
            "host_stdin_enabled": false,
            "raw_input_enabled": false,
            "helper_textarea_focused": false,
            "host_has_active_element": false,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 4,
            "render_event_count": 124,
            "data_event_count": 29,
            "write_command_count": 30,
            "last_raw_payload_sample": raw_payload,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 35,
            "base_y": 1940,
            "rows": 36,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0},
            "host_content_width": 1343.0,
            "host_content_height": 1144.0,
            "screen_rect": {"width": 1343.0, "height": 1144.0},
            "viewport_rect": {"width": 1343.0, "height": 1144.0},
            "helpers_rect": {"width": 1343.0, "height": 1144.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);

        // Discriminator: the SAME sparse read, but with the window focused
        // (document_focused = the user is looking at it, so the read is reliable
        // and a live paint frame doesn't excuse an unreadable surface), must NOT
        // abstain — a genuinely empty focused surface is still a problem.
        let mut focused = host.clone();
        focused["document_focused"] = json!(true);
        assert!(
            terminal_host_problem_for_app_control(&focused).is_some(),
            "a focused window's empty surface must still be diagnosable"
        );
    }

    #[test]
    fn terminal_host_problem_judges_unfocused_surface_when_content_is_readable() {
        // #1 root-cause fix (2026-06-04): the couldn't-observe abstain is now
        // CONFIDENCE-gated, not focus-gated. The content-finding samples
        // (text_tail / cursor_line_text, built from readTerminalBufferSample's
        // trailing+leading fallback) are reliable even when the host is unfocused
        // — confirmed live: an unfocused codex surface (all focus flags false,
        // focus_released) still read text_tail=4044 chars + cursor_line_text="› Use
        // /skills…". So an unfocused host with READABLE content must NOT be
        // abstained-on; the detector must judge it (and here, recognize a healthy
        // codex prompt -> None). This is what retires the broad focus-only guard:
        // detection is restored for the common unfocused-but-readable case.
        let tail = "  Worktree clean. Ready.\n\n› Use /skills to list available skills\n\n  gpt-5.5 medium · ~/git/samplescripts";
        let host = json!({
            "session_path": "remote-session://dev/unfocused-readable-codex",
            "text_sample": tail,
            "text_tail": tail,
            "buffer_text_sample": tail,
            "cursor_line_text": "› Use /skills to list available skills",
            // unfocused — every focus signal false, snapshot captured on blur
            "host_stdin_enabled": false,
            "raw_input_enabled": false,
            "helper_textarea_focused": false,
            "host_has_active_element": false,
            "document_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "render_event_count": 28,
            "data_event_count": 29,
            "write_command_count": 30,
            "last_raw_payload_sample": "\\x1b[?2026h\\x1b[58;2H\\x1b[0m\\x1b[49m\\x1b[K\\x1b[?2026l",
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 4,
            "base_y": 969,
            "cursor_y": 58,
            "rows": 63,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0},
            "host_content_width": 1343.0,
            "host_content_height": 1144.0,
            "screen_rect": {"width": 1343.0, "height": 1144.0},
            "viewport_rect": {"width": 1343.0, "height": 1144.0},
            "helpers_rect": {"width": 1343.0, "height": 1144.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert!(
            terminal_chunk_has_current_codex_input_row("› Use /skills to list available skills"),
            "fixture sanity: the cursor line is a recognized codex input row"
        );
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            None,
            "an unfocused host with a readable, healthy codex prompt must be judged \
             (not abstained-on) and reported healthy — detection restored"
        );
    }

    #[test]
    fn terminal_host_problem_accepts_live_remote_codex_prompt_before_input_gate() {
        let text_tail = "╭──────────────────────────────────────────────╮\n│ >_ OpenAI Codex (v0.128.0)                   │\n│                                              │\n│ model:     gpt-5.5 medium   /model to change │\n│ directory: ~/gh/yggterm                      │\n╰──────────────────────────────────────────────╯\n\n  Tip: New Use /fast to enable our fastest inference with increased plan usage.\n\n\n› Explain this codebase\n\n  gpt-5.5 medium · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://dev/new-codex",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Explain this codebase",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 42,
            "data_event_count": 0,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 39,
            "rows": 63,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert!(terminal_chunk_is_codex_prompt_surface(text_tail));
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_collapsed_remote_codex_prompt_surface() {
        let text_tail = "• Booting MCP server: codex_apps (0s • esc to interrupt)\n\n\n› Summarize recent commits\n\n  gpt-5.5 medium · ~";
        let host = json!({
            "session_path": "remote-session://dev/collapsed-codex",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Summarize recent commits",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 154,
            "data_event_count": 0,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 35,
            "rows": 48,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface is missing the welcome frame")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_remote_codex_prompt_only_surface_with_current_focus() {
        let text_tail = "⚠ Heads up, you have less than 25% of your weekly limit left. Run /status for a breakdown.\n\n\n› Write tests for @filename\n\n  gpt-5.5 xhigh · ~";
        let host = json!({
            "session_path": "remote-session://dev/prompt-only-codex",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Write tests for @filename",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 980,
            "data_event_count": 7,
            "last_data_event_at_ms": 1777800290482_u64,
            "last_raw_payload_length": 140,
            "write_command_count": 7,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 4,
            "rows": 48,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            None,
            "a focused, input-enabled current prompt is valid terminal truth; sparse prompt tests cover broken welcome-frame restores"
        );
    }

    #[test]
    fn terminal_host_problem_rejects_jojo_sparse_prompt_after_update() {
        let text_tail = "› say hi\n\n\n•\n\n\n\n\n\n\n\n\n \n› Use /skills to list available skills\n \n  gpt-5.5 xhigh · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://dev/019dfc5a-f5ca-7793-a44f-ee7f423aed38",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Use /skills to list available skills",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 30,
            "data_event_count": 2,
            "last_raw_payload_length": 491,
            "write_command_count": 9,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "cursor_sample_rect": {"left": 24.0, "top": 120.0, "width": 8.0, "height": 18.0},
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 46,
            "rows": 63,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 1336.0, "height": 1134.0},
            "host_content_width": 1336.0,
            "host_content_height": 1134.0,
            "screen_rect": {"width": 1336.0, "height": 1134.0},
            "viewport_rect": {"width": 1336.0, "height": 1134.0},
            "helpers_rect": {"width": 1336.0, "height": 1134.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 296.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface is missing the welcome frame")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_retained_remote_codex_prompt_with_real_scrollback_when_unfocused()
     {
        let text_tail = "Fixes Applied\n\n- Updated app_control.rs to capture resource baselines.\n- Added smoke coverage for remote startup.\n\nVerification\n\n- cargo test passed.\n- jojo screenshot showed a readable terminal.\n\n› Use /skills to list available skills\n\n  gpt-5.5 xhigh · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://dev/retained-ready",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Use /skills to list available skills",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 33,
            "data_event_count": 0,
            "last_raw_payload_length": 193514,
            "write_command_count": 8,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "daemon_retained_snapshot",
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 2,
            "rows": 50,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 8.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_daemon_pty_codex_scrollback_prompt_before_input_gate() {
        let text_tail = "\
- Active chart: SAMPLENOTES_BENCH_0099
  - Log rows: 742
  - Physical PDFs in output dir: 2510
  - Newly generated in this rerun: 6
  - Final chart-open failures: 0

Best thing to improve in the meantime:

1. Harden PL9 automation observability: classify failures by root cause.
2. Start SAMPLENOTES-side sync/comparison on already-ready PL9 fixtures.
3. Use the growing benchmark corpus to attack Varshaphala parity next.

› Improve documentation in @filename

  gpt-5.5 xhigh · ~/git/samplenotes";
        let host = json!({
            "session_path": "remote-session://dev/hot-update-real-pty",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Improve documentation in @filename",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 6,
            "data_event_count": 0,
            "last_raw_payload_length": 95_153,
            "last_raw_payload_line_count": 1_009,
            "write_command_count": 1,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "cursor_x": 2,
            "cursor_y": 33,
            "cursor_expected_rect": {"left": 293.0, "top": 602.0, "width": 8.0, "height": 18.0},
            "mounted_entry_host_connected": true,
            "base_y": 1000,
            "blank_rows_below_cursor": 16,
            "rows": 50,
            "cols": 110,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "host_content_width": 883.0,
            "host_content_height": 904.0,
            "screen_rect": {"width": 883.0, "height": 904.0},
            "viewport_rect": {"width": 883.0, "height": 904.0},
            "helpers_rect": {"width": 883.0, "height": 904.0},
            "helper_textarea_rect": {"left": -9723.0, "top": 8.0, "width": 1.0, "height": 1.0}
        });
        assert!(!terminal_observe_prompt_layout_is_acceptable(50, 14));
        assert!(terminal_observe_codex_prompt_tail_has_real_scrollback(
            "› Improve documentation in @filename",
            text_tail,
        ));
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_hot_update_retained_pty_prompt_follow_surface_with_current_input_row()
     {
        let text_tail = "Status as of 2026-05-08 11:53 IST:\n\n- PL9 batch is still alive.\n- Current action: generating PH2.\n\nProgress:\n\n- Charts reached: up to 0307.\n\n› /status\n\n  gpt-5.5 xhigh · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://dev/hot-update-kept",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 86,
            "data_event_count": 0,
            "last_raw_payload_length": 185570,
            "write_command_count": 1,
            "terminal_content_source": "active_recovery_pty_snapshot",
            "retained_replay_source": "active_recovery_pty_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 5,
            "rows": 50,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 8.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_daemon_pty_scrollback_prompt_with_blank_current_frame() {
        let text_tail = "Status as of 2026-05-08 11:53 IST:\n\n- PL9 batch is still alive.\n- Current action: generating PH2.\n\nProgress:\n\n- Charts reached: up to 0307.\n\n› 2\n\n• I’m not sure what 2 refers to. If you mean status again, say so and I’ll check the live batch state.";
        let host = json!({
            "session_path": "remote-session://dev/hot-update-daemon-pty",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "",
            "cursor_y": 44,
            "cursor_expected_rect": {"left": 1005.0, "top": 800.0, "width": 8.0, "height": 18.0},
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 63,
            "data_event_count": 0,
            "last_raw_payload_length": 178,
            "last_raw_payload_line_count": 0,
            "last_raw_payload_sample": "\u{1b}[?2026h\u{1b}[46;2H\u{1b}[0m\u{1b}[49m\u{1b}[K\u{1b}[47;2H\u{1b}[0m\u{1b}[48;2;57;57;57m\u{1b}[K\u{1b}[48;22H\u{1b}[0m\u{1b}[48;2;57;57;57m\u{1b}[K\u{1b}[49;2H\u{1b}[0m\u{1b}[48;2;57;57;57m\u{1b}[K\u{1b}[50;30H\u{1b}[0m\u{1b}[49m\u{1b}[K\u{1b}[39m\u{1b}[49m\u{1b}[0m\u{1b}[0 q\u{1b}[?25h\u{1b}[48;3H\u{1b}[?2026l",
            "write_command_count": 1,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "retained_replay_prompt_follow_ready": false,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 5,
            "rows": 50,
            "base_y": 1000,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 8.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface has stale scrollback but no current prompt")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_retained_recovery_tail_without_current_input_row() {
        let text_tail = "Status as of 2026-05-08 11:53 IST:\n\n- PL9 batch is still alive: PID 3573158.\n- Current chart: SAMPLENOTES_BENCH_0307.\n- Current action: generating PH2.\n\nProgress:\n\n- Charts reached: up to 0307.\n- Physical PL9 PDFs present: 1253.\n\n› 2\n\n• I’m not sure what 2 refers to. If you mean status again, say so and I’ll check the live batch\nstate.";
        let host = json!({
            "session_path": "remote-session://dev/hot-update-retained-recovery",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "",
            "cursor_y": 44,
            "cursor_expected_rect": {"left": 1005.0, "top": 800.0, "width": 8.0, "height": 18.0},
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 79,
            "data_event_count": 0,
            "last_raw_payload_length": 185570,
            "last_raw_payload_line_count": 2009,
            "write_command_count": 1,
            "terminal_content_source": "active_recovery_pty_snapshot",
            "retained_replay_source": "active_recovery_pty_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 5,
            "rows": 50,
            "base_y": 1000,
            "scrollback_expected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 8.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface has no current input row")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_working_codex_without_input_row() {
        // Regression lock for the mid-turn remount that destroyed scrollback
        // (finding-codex-scroll-lock-no-client-scrollback). Identical surface to
        // the "no current input row" fault above, EXCEPT codex is actively WORKING
        // (its footer shows the "esc to interrupt" indicator). A working codex
        // legitimately has no `›` input row; it must NOT be reported as a faulted
        // surface, or retained-fault-recovery remounts it mid-turn and reseeds
        // xterm from a one-screen snapshot, destroying the session's scrollback.
        let text_tail = "Status as of 2026-05-08 11:53 IST:\n\n- PL9 batch is still alive: PID 3573158.\n- Current chart: SAMPLENOTES_BENCH_0307.\n- Current action: generating PH2.\n\nProgress:\n\n- Charts reached: up to 0307.\n- Physical PL9 PDFs present: 1253.\n\n• Working (3s · esc to interrupt)";
        let host = json!({
            "session_path": "remote-session://dev/working-codex-no-input-row",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "",
            "cursor_y": 44,
            "cursor_expected_rect": {"left": 1005.0, "top": 800.0, "width": 8.0, "height": 18.0},
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 79,
            "data_event_count": 0,
            "last_raw_payload_length": 185570,
            "last_raw_payload_line_count": 2009,
            "write_command_count": 1,
            "terminal_content_source": "active_recovery_pty_snapshot",
            "retained_replay_source": "active_recovery_pty_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 5,
            "rows": 50,
            "base_y": 1000,
            "scrollback_expected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 8.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            None,
            "a working codex (esc to interrupt) must not be reported as a faulted \
             surface — that remount destroys scrollback mid-turn"
        );
    }

    #[test]
    fn terminal_chunk_is_codex_working_surface_detects_interrupt_footer() {
        assert!(terminal_chunk_is_codex_working_surface(
            "• Working (3s · esc to interrupt)"
        ));
        assert!(terminal_chunk_is_codex_working_surface(
            "• Booting MCP server: codex_apps (0s • esc to interrupt)"
        ));
        // Idle composer / interrupted state must NOT read as working.
        assert!(!terminal_chunk_is_codex_working_surface(
            "› What is the status now?\n\n  gpt-5.5 medium · ~/git"
        ));
        assert!(!terminal_chunk_is_codex_working_surface(
            "■ Conversation interrupted - tell the model what to do differently."
        ));
    }

    #[test]
    fn terminal_host_problem_rejects_gated_daemon_pty_tail_without_current_input_row() {
        let text_tail = "─────────────────────────────────────────────────────────────────────────────────────────────────────────────\n\n• Status as of 2026-05-08 11:53 IST:\n\n- PL9 batch is still alive: PID 3573158.\n- Current chart: SAMPLENOTES_BENCH_0307.\n\n› 2\n\n• I’m not sure what 2 refers to. If you mean status again, say so and I’ll check the live PL9 batch\nstate.";
        let host = json!({
            "session_path": "remote-session://dev/hot-update-gated-daemon-pty",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 63,
            "data_event_count": 0,
            "last_raw_payload_length": 185570,
            "last_raw_payload_line_count": 2009,
            "write_command_count": 1,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "retained_replay_prompt_follow_ready": false,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 5,
            "rows": 50,
            "base_y": 1000,
            "scrollback_expected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface has no current input row")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_remote_prompt_gated_after_user_input() {
        let text_tail = "Current checkpoint:\n\n- Active chart: SAMPLENOTES_BENCH_0099\n- Log rows: 742\n- Physical PDFs in output dir: 2510\n\n› What is the status now?\n\n• Working (0s • esc to interrupt)";
        let host = json!({
            "session_path": "remote-session://dev/samplenotes-paint-regression",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Write tests for @filename",
            "host_stdin_enabled": false,
            "raw_input_enabled": false,
            "helper_textarea_focused": false,
            "document_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 105,
            "data_event_count": 50,
            "protocol_data_event_count": 0,
            "pending_input_bytes": 0,
            "input_batch_flush_count": 50,
            "last_input_batch_length": 1,
            "last_input_batch_at_ms": 1_778_947_705_565_u64,
            "last_pending_input_reason": "queue",
            "last_data_event_at_ms": 1_778_947_705_565_u64,
            "last_write_callback_at_ms": 1_778_947_705_767_u64,
            "last_write_flush_started_at_ms": 1_778_947_739_673_u64,
            "last_write_queued_at_ms": 1_778_947_739_673_u64,
            "last_raw_payload_length": 370,
            "last_raw_payload_line_count": 2,
            "last_raw_payload_sample": "\u{1b}[?2026h\u{1b}[1;45r\u{1b}[45;1H\r\n\u{1b}[39;49m\u{1b}[K\u{1b}[39m\u{1b}[49m\u{1b}[0m\r\n\u{1b}[39;49m\u{1b}[K\u{1b}[2m──────────────────────────────────────────────────────────────────────────────────────────────────────────────\u{1b}[39m\u{1b}[49m\u{1b}[0m\u{1b}[r\u{1b}[48;3H\u{1b}[46;2H\u{1b}[0m\u{1b}[49m\u{1b}[K\u{1b}[47;2H\u{1b}[0m\u{1b}[48;2;57;57;57m\u{1b}[K\u{1b}[48;28H\u{1b}[0m\u{1b}[48;2;57;57;57m\u{1b}[K\u{1b}[49;2H\u{1b}[0m\u{1b}[48;2;57;57;57m\u{1b}[K\u{1b}[50;30H\u{1b}[0m\u{1b}[49m\u{1b}[K\u{1b}[39m\u{1b}[49m\u{1b}[0m\u{1b}[0 q\u{1b}[?25h\u{1b}[48;3H\u{1b}[?2026l",
            "write_command_count": 67,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "retained_replay_prompt_follow_ready": false,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 2,
            "rows": 50,
            "cols": 110,
            "base_y": 958,
            "viewport_y": 958,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "host_content_width": 883.0,
            "host_content_height": 904.0,
            "screen_rect": {"width": 883.0, "height": 904.0},
            "viewport_rect": {"width": 883.0, "height": 904.0},
            "helpers_rect": {"width": 883.0, "height": 904.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal prompt is input-gated after user input")
        );
    }

    #[test]
    fn terminal_host_problem_ignores_input_gate_when_snapshot_unfocused() {
        let text_tail = "Current checkpoint:\n\n› Implement {feature}";
        let host = json!({
            "session_path": "remote-session://practice/unfocused-app-control-snapshot",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Implement {feature}",
            "host_stdin_enabled": false,
            "raw_input_enabled": false,
            "helper_textarea_focused": false,
            "host_has_active_element": false,
            "document_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "render_event_count": 12,
            "data_event_count": 8,
            "protocol_data_event_count": 0,
            "pending_input_bytes": 0,
            "input_batch_flush_count": 8,
            "last_input_batch_length": 1,
            "last_pending_input_reason": "queue",
            "last_raw_payload_length": 120,
            "last_raw_payload_line_count": 0,
            "write_command_count": 8,
            "terminal_content_source": "daemon_pty",
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "cursor_sample_rect": {"left": 24.0, "top": 120.0, "width": 8.0, "height": 18.0},
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 2,
            "rows": 50,
            "base_y": 1,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0}
        });

        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_codex_resume_instruction_as_interactive() {
        let text_tail = "⚠ Heads up, you have less than 5% of your weekly limit left. Run /status for a breakdown.\nToken usage: total=1,064,406 input=1,023,193 (+ 11,361,664 cached) output=41,213 (reasoning 10,977)\nTo continue this session, run codex resume 019d0000-0000-7000-8000-000000000001";
        let host = json!({
            "session_path": "remote-session://dev/exited-codex",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": " this session, run codex resume 019d0000-0000-7000-8000-000000000001",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 102,
            "data_event_count": 13,
            "last_raw_payload_length": 194,
            "last_raw_payload_line_count": 1,
            "write_command_count": 6,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "daemon_retained_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 2,
            "rows": 50,
            "base_y": 0,
            "scrollback_expected": false,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 880.0, "height": 900.0},
            "host_content_width": 880.0,
            "host_content_height": 900.0,
            "screen_rect": {"width": 880.0, "height": 900.0},
            "viewport_rect": {"width": 880.0, "height": 900.0},
            "helpers_rect": {"width": 880.0, "height": 900.0}
        });

        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex runtime has exited and is showing a resume instruction")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_accepted_remote_input_without_stream_echo() {
        let text_tail = "⚠ Heads up, you have less than 25% of your weekly limit left. Run /status for a breakdown.\n\n\n› Find and fix a bug in @filename\n\n  gpt-5.5 xhigh · ~";
        let host = json!({
            "session_path": "remote-session://dev/no-echo-after-input",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Find and fix a bug in @filename",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 249,
            "data_event_count": 2,
            "last_data_event_at_ms": 1777876167708_u64,
            "last_write_queued_at_ms": 1777875854493_u64,
            "last_write_flush_started_at_ms": 1777875854749_u64,
            "last_write_callback_at_ms": 1777875854749_u64,
            "last_raw_payload_length": 140,
            "write_command_count": 12,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 43,
            "rows": 48,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });

        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal accepted input without a following daemon stream echo")
        );
    }

    #[test]
    fn terminal_host_problem_keeps_current_prompt_input_enabled_without_later_echo() {
        let text_tail = "Release notes updated.\n\n› Copy pasting the code from email or typing it does not auto switch the input boxes like other prompts\n\n  gpt-5.5 xhigh · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://practice/current-prompt-no-later-echo",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Copy pasting the code from email or typing it does not auto switch the input boxes like other prompts",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "render_event_count": 121,
            "data_event_count": 135,
            "pending_input_bytes": 0,
            "input_batch_flush_count": 135,
            "last_input_batch_length": 1,
            "last_input_batch_flush_reason": "timer",
            "last_input_batch_at_ms": 1_779_113_609_839_u64,
            "last_data_event_at_ms": 1_779_113_609_831_u64,
            "last_write_queued_at_ms": 1_779_113_609_457_u64,
            "last_write_flush_started_at_ms": 1_779_113_609_457_u64,
            "last_write_callback_at_ms": 1_779_113_609_458_u64,
            "last_raw_payload_length": 207,
            "last_raw_payload_line_count": 0,
            "write_command_count": 104,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "daemon_terminal_read",
            "retained_replay_prompt_follow_ready": true,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 2,
            "rows": 63,
            "base_y": 347,
            "scrollback_expected": true,
            "cursor_sample_rect": {"left": 1092.0, "top": 1088.0, "width": 8.0, "height": 18.0},
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1134.0},
            "host_content_width": 1336.0,
            "host_content_height": 1134.0,
            "screen_rect": {"width": 1336.0, "height": 1134.0},
            "viewport_rect": {"width": 1336.0, "height": 1134.0},
            "helpers_rect": {"width": 1336.0, "height": 1134.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 1088.0, "width": 1.0, "height": 1.0}
        });

        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_waits_for_pending_input_batch_before_stream_echo_alarm() {
        let text_tail = "⚠ Heads up, you have less than 25% of your weekly limit left. Run /status for a breakdown.\n\n\n› Find and fix a bug in @filename\n\n  gpt-5.5 xhigh · ~";
        let host = json!({
            "session_path": "remote-session://dev/pending-batched-input",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Find and fix a bug in @filename",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 249,
            "data_event_count": 2,
            "pending_input_bytes": 18,
            "pending_input_flush_scheduled": true,
            "input_batch_flush_count": 0,
            "last_input_batch_length": 0,
            "last_input_batch_flush_reason": "",
            "last_input_batch_at_ms": 0,
            "last_pending_input_reason": "timer",
            "last_data_event_at_ms": 1777876167708_u64,
            "last_write_queued_at_ms": 1777875854493_u64,
            "last_write_flush_started_at_ms": 1777875854749_u64,
            "last_write_callback_at_ms": 1777875854749_u64,
            "last_raw_payload_length": 140,
            "write_command_count": 12,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 4,
            "rows": 48,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });

        assert_eq!(terminal_host_problem_for_app_control(&host), None);
        let timing = terminal_timing_for_app_control(Some(&host));
        assert_eq!(
            timing.get("pending_input_bytes").and_then(Value::as_u64),
            Some(18)
        );
        assert_eq!(
            timing
                .get("pending_input_flush_scheduled")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            timing
                .get("last_pending_input_reason")
                .and_then(Value::as_str),
            Some("timer")
        );
    }

    #[test]
    fn terminal_host_problem_ignores_remote_focus_protocol_without_stream_echo() {
        let text_tail = "\
╭─────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.128.0)                  │
│                                             │
│ model:     gpt-5.5 xhigh   /model to change │
│ directory: ~/gh/yggterm                     │
╰─────────────────────────────────────────────╯

  Tip: Use /feedback to send logs to the maintainers when something looks off.


› Run /review on my current changes

  gpt-5.5 xhigh · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://dev/focus-protocol-only",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "› Run /review on my current changes",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 13,
            "data_event_count": 1,
            "protocol_data_event_count": 1,
            "last_data_event_at_ms": 1778152763642_u64,
            "last_write_queued_at_ms": 1778152744451_u64,
            "last_write_flush_started_at_ms": 1778152744451_u64,
            "last_write_callback_at_ms": 1778152744188_u64,
            "last_raw_payload_length": 139,
            "write_command_count": 7,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 37,
            "rows": 48,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });

        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_chunk_is_claude_prompt_surface_recognizes_claude_surfaces() {
        // Permission/selection prompt (the live remote-cc blank-viewport repro).
        assert!(terminal_chunk_is_claude_prompt_surface(
            "\
●

 ❯ 1. Yes
   2. Yes, allow reading from src/ during this session
   3. No

               · Tab to amend"
        ));
        // Idle input box footer.
        assert!(terminal_chunk_is_claude_prompt_surface(
            "\
╭───────────────────────────────────────────╮
│ > Try \"edit <filepath>\"                     │
╰───────────────────────────────────────────╯
  ? for shortcuts"
        ));
        // Must NOT match a plain shell surface or empty/loading content.
        assert!(!terminal_chunk_is_claude_prompt_surface("user@host:~/gh/yggterm$ "));
        assert!(!terminal_chunk_is_claude_prompt_surface(""));
        // A bare caret without any Claude affordance is not enough.
        assert!(!terminal_chunk_is_claude_prompt_surface("❯ "));
    }

    #[test]
    fn terminal_chunk_has_codex_prompt_output_accepts_codex_prompt_line() {
        assert!(terminal_chunk_has_codex_prompt_output(
            "› Explain this codebase"
        ));
        assert!(terminal_chunk_has_codex_prompt_output(
            "› Write tests for @filename"
        ));
        assert!(terminal_chunk_has_codex_prompt_output(
            "\
╭────────────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.125.0)                             │
│                                                        │
│ model:     gpt-5.3-codex-spark high   /model to change │
│ directory: ~                                           │
╰────────────────────────────────────────────────────────╯


› Summarize recent commits

  gpt-5.3-codex-spark high · ~"
        ));
        assert!(terminal_chunk_has_codex_prompt_output(
            "\
╰─────────────────────────────────────────────╯\x1b[8;1H  Tip: New Use /fast to enable our fastest inference with increased plan usage.\x1b[10;1H⚠ Heads up, you have less than 10% of your weekly limit left. Run /status for a breakdown.\x1b[13;1H›\x1b[CSummarize recent commits\x1b[15;1H  gpt-5.5 xhigh · ~/gh/yggterm\x1b[13;3H\x1b>\x1b[?1l\x1b[?2004h"
        ));
        assert!(!terminal_chunk_has_codex_prompt_output(
            "\
╭────────────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.125.0)                             │
│                                                        │
│ model:     gpt-5.3-codex-spark high   /model to change │
│ directory: ~                                           │
╰────────────────────────────────────────────────────────╯"
        ));
        assert!(!terminal_chunk_has_codex_prompt_output("$ echo hi"));
    }

    #[test]
    fn terminal_chunk_has_current_codex_input_row_rejects_old_prompt_before_output() {
        assert!(terminal_chunk_has_current_codex_input_row(
            "Status rows\n\n› /status\n\n  gpt-5.5 xhigh · ~/gh/yggterm"
        ));
        assert!(terminal_chunk_has_current_codex_input_row(
            "Status rows\n\n›"
        ));
        assert!(!terminal_chunk_has_current_codex_input_row(
            "Status rows\n\n› 2\n\n• I’m not sure what 2 refers to.\nstate."
        ));
    }

    #[test]
    fn terminal_chunk_accepts_wrapped_codex_prompt_input_region() {
        let prompt = "\
╭─────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.130.0)                  │
│ model:     gpt-5.5 low   /model to change   │
│ directory: ~                                │
╰─────────────────────────────────────────────╯

› I want you to ssh manin and setup PaddleOCR in ssh practice in
  passthoguth for usage in practice container. Then I want you to setup
  PaddleOCR in ssh practice in ~/gh/paddleocr and validate it.

  gpt-5.5 low · ~";
        assert!(terminal_chunk_has_current_codex_input_row(prompt));
        assert!(terminal_chunk_has_codex_prompt_output(prompt));
        assert_eq!(
            terminal_host_problem_for_app_control(&json!({
                "session_path": "remote-session://dev/wrapped-prompt",
                "text_sample": prompt,
                "text_tail": prompt,
                "cursor_line_text": "  PaddleOCR in ssh practice in ~/gh/paddleocr and validate it.",
                "host_stdin_enabled": true,
                "helper_textarea_focused": true,
                "mounted_entry_host_connected": true,
                "xterm_present": true,
                "screen_present": true,
                "rows_present": true,
                "canvas_count": 1,
                "terminal_content_source": "daemon_pty",
                "last_raw_payload_length": 64,
                "write_command_count": 3,
                "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
                "host_content_width": 840.0,
                "host_content_height": 830.0,
                "screen_rect": {"width": 840.0, "height": 830.0},
                "viewport_rect": {"width": 840.0, "height": 830.0},
                "helpers_rect": {"width": 840.0, "height": 830.0},
                "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
            })),
            None
        );
    }

    #[test]
    fn terminal_host_problem_accepts_wrapped_codex_prompt_from_tail_sample() {
        let header = "\
╭─────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.130.0)                  │
│                                             │
│ model:     gpt-5.5 xhigh   /model to change │
│ directory: ~/git/samplers                │";
        let tail = "\
╭─────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.130.0)                  │
│                                             │
│ model:     gpt-5.5 xhigh   /model to change │
│ directory: ~/git/samplers                │
╰─────────────────────────────────────────────╯

  Tip: GPT-5.5 is now available in Codex.

• Permissions updated to Full Access

› 1. Always sign the Android builds before publishing any debug artifact.
  Register the developer account later and keep only development builds in CI.

  2. Replace password login with Google, Apple, and email magic-link login.
  Send a short alphanumeric code and collect it in six single-character boxes.
  Uppercase the alphabetic code characters even when the user types lowercase.

  gpt-5.5 xhigh · ~/git/samplers";
        let cursor_line =
            "  Uppercase the alphabetic code characters even when the user types lowercase.";
        assert!(
            !terminal_chunk_has_codex_prompt_output(
                &[header, tail, header, cursor_line].join("\n")
            ),
            "overlapping app-control samples are not a single ordered terminal transcript"
        );
        assert!(terminal_chunk_has_codex_prompt_output(tail));
        assert_eq!(
            terminal_host_problem_for_app_control(&json!({
                "session_path": "remote-session://practice/wrapped-overlap",
                "text_sample": header,
                "text_tail": tail,
                "buffer_text_sample": header,
                "cursor_line_text": cursor_line,
                "host_stdin_enabled": true,
                "helper_textarea_focused": true,
                "host_has_active_element": true,
                "mounted_entry_host_connected": true,
                "xterm_present": true,
                "screen_present": true,
                "rows_present": true,
                "canvas_count": 1,
                "terminal_content_source": "daemon_pty",
                "rows": 50,
                "cols": 110,
                "cursor_y": 23,
                "blank_rows_below_cursor": 26,
                "base_y": 1,
                "last_raw_payload_length": 460,
                "write_command_count": 538,
                "data_event_count": 359,
                "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
                "host_content_width": 840.0,
                "host_content_height": 830.0,
                "screen_rect": {"width": 840.0, "height": 830.0},
                "viewport_rect": {"width": 840.0, "height": 830.0},
                "helpers_rect": {"width": 840.0, "height": 830.0},
                "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
            })),
            None
        );
    }

    #[test]
    fn terminal_host_problem_accepts_visible_prompt_surface_without_terminal_focus() {
        let host = json!({
            "text_sample": "› Explain this codebase

  gpt-5.4 high fast · 100% left · ~/git",
            "cursor_line_text": "› Explain this codebase",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "cursor_node_count": 0,
            "cursor_sample_rect": {"left": 302.0, "top": 214.0, "width": 8.0, "height": 17.0},
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_fresh_local_shell_prompt_before_input_is_enabled() {
        let host = json!({
            "session_path": "local://fresh-shell",
            "text_sample": "pi@jojo:~$",
            "cursor_line_text": "pi@jojo:~$",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "mounted_entry_host_connected": true,
            "render_event_count": 12,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn app_control_terminal_surface_flags_blank_canvas_with_buffer_text() {
        let host = json!({
            "session_path": "local://codex-session",
            "text_tail": "OpenAI Codex ready",
            "buffer_text_sample": "OpenAI Codex\n› /status",
            "cursor_line_text": "› /status",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 2,
            "canvas_layers": [
                {
                    "ink_sample": {
                        "sampled_pixels": 24,
                        "nontransparent_pixels": 0,
                        "alpha_sum": 0
                    }
                }
            ]
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("healthy"))
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("reason"))
                .and_then(Value::as_str),
            Some("canvas_blank_with_buffer_text")
        );
        assert_eq!(
            summary.get("problem").and_then(Value::as_str),
            Some("canvas_blank_with_buffer_text")
        );
    }

    #[test]
    fn app_control_terminal_surface_flags_dom_renderer_missing_rows_with_buffer_text() {
        let host = json!({
            "session_path": "local://codex-session",
            "text_tail": "OpenAI Codex ready\n› Use /skills to list available skills",
            "buffer_text_sample": "OpenAI Codex\n› Use /skills to list available skills",
            "cursor_line_text": "› Use /skills to list available skills",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 0,
            "visible_canvas_layer_count": 0,
            "xterm_renderer_mode": "dom",
            "render_health_status": "healthy"
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("healthy"))
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("reason"))
                .and_then(Value::as_str),
            Some("dom_renderer_missing_text_layer_with_buffer_text")
        );
        assert_eq!(
            summary.get("problem").and_then(Value::as_str),
            Some("dom_renderer_missing_text_layer_with_buffer_text")
        );
    }

    #[test]
    fn app_control_terminal_surface_treats_prompt_follow_visual_scroll_mismatch_as_stale_at_bottom()
    {
        let host = json!({
            "session_path": "local://codex-session",
            "text_tail": "OpenAI Codex ready\n› /status",
            "buffer_text_sample": "OpenAI Codex\n› /status",
            "cursor_line_text": "› /status",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "xterm_renderer_mode": "dom",
            "scrollback_locked": true,
            "scrollback_intent": "PromptFollow",
            "base_y": 70.0,
            "viewport_y": 0.0,
            "public_viewport_y": 70.0,
            "visual_viewport_y": 0.0,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });

        assert_eq!(terminal_host_geometry_problem_for_app_control(&host), None);
    }

    #[test]
    fn app_control_terminal_surface_flags_viewport_beyond_xterm_base() {
        let host = json!({
            "session_path": "remote-session://dev/live",
            "text_tail": "OpenAI Codex ready\n› Run /review",
            "buffer_text_sample": "OpenAI Codex\n› Run /review",
            "cursor_line_text": "› Run /review",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "xterm_renderer_mode": "dom",
            "scrollback_locked": false,
            "scrollback_intent": "PromptFollow",
            "base_y": 1000.0,
            "viewport_y": 1286.0,
            "public_viewport_y": 1000.0,
            "visual_viewport_y": 1286.0,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0, "bottom": 1152.0},
            "host_content_width": 1343.0,
            "host_content_height": 1144.0,
            "screen_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "viewport_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0},
            "cursor_expected_rect": {"left": 293.0, "top": 1124.0, "width": 8.0, "height": 18.0},
            "fit_required_height_px": 1134.0,
            "fit_available_height_px": 1142.0,
            "fit_overflow_px": 0.0,
        });

        assert_eq!(
            terminal_host_geometry_problem_for_app_control(&host),
            Some("active terminal viewport is beyond the xterm scrollback base")
        );
    }

    // XTERM-BUG: blank-viewport-client-snapshot-poison — a transient "viewport
    // beyond scrollback base" on a cursor-addressed codex reseed must stay
    // OBSERVABLE (geometry fn) but must NOT escalate (host-problem consumer),
    // so it can't drive a recovery/remount that restarts a working session.
    #[test]
    fn host_problem_suppresses_transient_viewport_beyond_base_for_codex_reseed() {
        // A healthy codex surface (prompt box + input row + model footer) — the
        // body of terminal_host_problem_for_app_control accepts this as clean.
        let codex_tail = "\
╭─────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.130.0)                  │
╰─────────────────────────────────────────────╯

› Run /review on my current changes

  gpt-5.5 medium · ~/gh/yggterm";
        let cursor_line = "› Run /review on my current changes";
        // Same surface, but the viewport is transiently beyond a LOW base_y
        // (codex owns its scrollback) — the reseed artifact.
        let codex_reseed = json!({
            "session_path": "codex-runtime://live",
            "text_sample": codex_tail,
            "text_tail": codex_tail,
            "buffer_text_sample": codex_tail,
            "cursor_line_text": cursor_line,
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 1,
            "terminal_content_source": "daemon_pty",
            "rows": 63.0,
            "base_y": 8.0,
            "viewport_y": 40.0,
            "public_viewport_y": 40.0,
            "visual_viewport_y": 40.0,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        // Still OBSERVED by the pure geometry detector…
        assert_eq!(
            terminal_host_geometry_problem_for_app_control(&codex_reseed),
            Some("active terminal viewport is beyond the xterm scrollback base")
        );
        // …but NOT escalated by the host-problem consumer (no recovery/restart).
        assert_eq!(terminal_host_problem_for_app_control(&codex_reseed), None);

        // A real-scrollback session (base_y far above rows) with the same
        // beyond-base reading is a genuine fault and STILL escalates.
        let mut real_scrollback = codex_reseed.clone();
        real_scrollback["base_y"] = json!(1000.0);
        real_scrollback["viewport_y"] = json!(1286.0);
        real_scrollback["public_viewport_y"] = json!(1000.0);
        real_scrollback["visual_viewport_y"] = json!(1286.0);
        assert_eq!(
            terminal_host_problem_for_app_control(&real_scrollback),
            Some("active terminal viewport is beyond the xterm scrollback base")
        );
    }

    #[test]
    fn app_control_terminal_surface_ignores_stale_cursor_clip_after_matched_prompt_follow_force() {
        let host = json!({
            "session_path": "remote-session://samplenotes-webapp/live",
            "text_tail": "OpenAI Codex ready\n› Improve documentation in @filename",
            "buffer_text_sample": "OpenAI Codex\n› Improve documentation in @filename",
            "cursor_line_text": "› Improve documentation in @filename",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "xterm_renderer_mode": "dom",
            "scrollback_locked": false,
            "scrollback_intent": "PromptFollow",
            "base_y": 882.0,
            "viewport_y": 257.0,
            "public_viewport_y": 257.0,
            "visual_viewport_y": 882.0,
            "host_rect": {"left": 277.0, "top": 257.0, "width": 883.0, "height": 900.0, "bottom": 1157.0},
            "host_content_width": 883.0,
            "host_content_height": 900.0,
            "screen_rect": {"left": 277.0, "top": 257.0, "width": 883.0, "height": 900.0},
            "viewport_rect": {"left": 277.0, "top": 257.0, "width": 883.0, "height": 900.0},
            "cursor_expected_rect": {"left": 293.0, "top": 12374.0, "width": 8.0, "height": 18.0},
            "fit_required_height_px": 900.0,
            "fit_available_height_px": 900.0,
            "fit_overflow_px": 0.0,
            "last_viewport_force_debug": {
                "matched_target": true,
                "target_viewport_y": 882.0,
                "after_effective_viewport_y": 882.0,
                "after_viewport_y": 882.0
            }
        });

        assert_eq!(terminal_host_geometry_problem_for_app_control(&host), None);
    }

    #[test]
    fn app_control_terminal_surface_rejects_stale_prompt_follow_match_after_viewport_drift() {
        let host = json!({
            "session_path": "remote-session://dev/live",
            "text_tail": "OpenAI Codex ready\n› Before the story",
            "buffer_text_sample": "OpenAI Codex\n› Before the story",
            "cursor_line_text": "› Before the story",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "xterm_renderer_mode": "dom",
            "scrollback_locked": false,
            "scrollback_intent": "PromptFollow",
            "base_y": 1000.0,
            "viewport_y": 778.0,
            "public_viewport_y": 778.0,
            "visual_viewport_y": 778.0,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0, "bottom": 1152.0},
            "host_content_width": 1343.0,
            "host_content_height": 1144.0,
            "screen_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "viewport_rect": {"left": 277.0, "top": 8.0, "width": 1343.0, "height": 1144.0},
            "cursor_expected_rect": {"left": 470.0, "top": 5120.0, "width": 8.0, "height": 18.0},
            "fit_required_height_px": 1134.0,
            "fit_available_height_px": 1142.0,
            "fit_overflow_px": 0.0,
            "last_viewport_force_debug": {
                "matched_target": true,
                "target_viewport_y": 1000.0,
                "after_effective_viewport_y": 1000.0,
                "after_viewport_y": 1000.0
            }
        });

        assert_eq!(
            terminal_host_geometry_problem_for_app_control(&host),
            Some("active terminal cursor row is clipped below the visible host")
        );
    }

    #[test]
    fn app_control_terminal_surface_does_not_flag_canvas_low_contrast_from_dom_color() {
        // With the canvas renderer the DOM `.xterm-rows` are unstyled, so the
        // computed `foreground_color` resolves to black even though the canvas
        // paints readable glyphs. The old DOM-style low-contrast heuristic
        // falsely flagged every healthy Wayland (canvas) session here and stuck
        // it at `ready:false`. That check is removed; genuine canvas paint
        // failures are caught by the canvas-ink (`canvas_blank...`) check.
        let host = json!({
            "session_path": "remote-session://practice/codex",
            "text_tail": "OpenAI Codex ready",
            "buffer_text_sample": "OpenAI Codex\n› Find and fix a bug in @filename",
            "cursor_line_text": "› Find and fix a bug in @filename",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "foreground_color": "rgb(0, 0, 0)",
            "background_color": "rgb(38, 42, 51)",
            "effective_background_color": "rgb(38, 42, 51)"
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_ne!(
            summary
                .get("render_health")
                .and_then(|health| health.get("reason"))
                .and_then(Value::as_str),
            Some("canvas_low_contrast_foreground_with_buffer_text"),
            "DOM-color low-contrast must not flag a canvas-rendered surface"
        );
        assert_ne!(
            summary.get("problem").and_then(Value::as_str),
            Some("canvas_low_contrast_foreground_with_buffer_text")
        );
    }

    #[test]
    fn app_control_terminal_surface_trusts_canvas_with_healthy_ink_over_dom_contrast() {
        // Same low DOM-style contrast as the flagged case, but the canvas
        // layers report healthy ink — the glyphs are visibly painted on the
        // canvas, so the DOM-style contrast heuristic must NOT fire (this is
        // the Wayland canvas-renderer false positive that blocked enabling it).
        let host = json!({
            "session_path": "remote-session://practice/codex",
            "text_tail": "OpenAI Codex ready",
            "buffer_text_sample": "OpenAI Codex\n› Find and fix a bug in @filename",
            "cursor_line_text": "› Find and fix a bug in @filename",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "foreground_color": "rgb(0, 0, 0)",
            "background_color": "rgb(38, 42, 51)",
            "effective_background_color": "rgb(38, 42, 51)",
            "canvas_layers": [
                {"ink_sample": {"sampled_pixels": 4096, "nontransparent_pixels": 1200, "alpha_sum": 250000}}
            ]
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("healthy"))
                .and_then(Value::as_bool),
            Some(true),
            "canvas with healthy ink must not be flagged low-contrast from DOM style"
        );
    }

    #[test]
    fn app_control_terminal_surface_flags_transparent_dom_rows_with_buffer_text() {
        let host = json!({
            "session_path": "remote-session://dev/codex",
            "text_tail": "OpenAI Codex ready",
            "buffer_text_sample": "OpenAI Codex\n› /status",
            "cursor_line_text": "› /status",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "xterm_renderer_mode": "dom",
            "rows_text_fill_color": "rgb(251, 251, 253)",
            "rows_sample_text_fill_color": "rgba(0, 0, 0, 0)",
            "cursor_row_text_fill_color": "rgb(251, 251, 253)",
            "visible_row_samples_tail": [
                {
                    "text": "› /status",
                    "text_fill_color": "rgba(0, 0, 0, 0)"
                }
            ]
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("healthy"))
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            summary
                .get("render_health")
                .and_then(|health| health.get("reason"))
                .and_then(Value::as_str),
            Some("dom_rows_transparent_with_buffer_text")
        );
        assert_eq!(
            summary.get("problem").and_then(Value::as_str),
            Some("dom_rows_transparent_with_buffer_text")
        );
    }

    #[test]
    fn app_control_terminal_surface_exposes_active_prompt_and_timing_truth() {
        let host = json!({
            "host_id": "host-active",
            "session_path": "remote-session://dev/live",
            "text_tail": "OpenAI Codex\n› Improve docs",
            "cursor_line_text": "› Improve docs",
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "canvas_count": 2,
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "terminal_input_hot": true,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "terminal_source_mismatch_reason": "",
            "cursor_expected_rect": {"left": 12.0, "top": 24.0, "width": 8.0, "height": 18.0},
            "cursor_bottom_overflow_px": 0.0,
            "fit_overflow_px": 0.0,
            "last_write_flush_started_at_ms": 1200,
            "last_render_event_at_ms": 1234,
            "last_manual_redraw_started_at_ms": 1300,
            "last_manual_redraw_settled_at_ms": 1324,
            "last_manual_redraw_duration_ms": 24,
            "last_manual_redraw_effect": "rendered",
            "write_bridge_flush_count": 7,
            "render_event_count": 9,
            "manual_redraw_count": 2
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            summary.get("foreground_input_ready").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary.get("content_source").and_then(Value::as_str),
            Some("daemon_pty")
        );
        assert_eq!(
            summary
                .get("prompt_band")
                .and_then(|prompt| prompt.get("cursor_line_text"))
                .and_then(Value::as_str),
            Some("› Improve docs")
        );
        assert_eq!(
            summary
                .get("timing")
                .and_then(|timing| timing.get("last_manual_redraw_effect"))
                .and_then(Value::as_str),
            Some("rendered")
        );
        assert_eq!(
            summary
                .get("timing")
                .and_then(|timing| timing.get("write_bridge_flush_count"))
                .and_then(Value::as_u64),
            Some(7)
        );
    }

    #[test]
    fn app_control_terminal_surface_gates_input_when_surface_has_problem() {
        let host = json!({
            "host_id": "host-blank",
            "session_path": "local://blank-codex",
            "text_sample": "",
            "text_tail": "",
            "buffer_text_sample": "",
            "cursor_line_text": "",
            "xterm_present": true,
            "screen_present": true,
            "canvas_count": 2,
            "render_event_count": 1,
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "terminal_content_source": "empty"
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            summary.get("problem").and_then(Value::as_str),
            Some("active terminal host exists but xterm surface is empty")
        );
        assert_eq!(
            summary.get("raw_input_enabled").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary.get("foreground_input_ready").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn app_control_terminal_surface_accepts_unfocused_busy_daemon_pty_output() {
        let host = json!({
            "host_id": "remote-busy",
            "session_path": "remote-session://dev/busy-codex",
            "text_sample": "The official full validation coverage is stale from 2026-05-06 20:40 IST.\n\n─ Worked for 1m 42s ─",
            "text_tail": "The official full validation coverage is stale from 2026-05-06 20:40 IST.\n\n─ Worked for 1m 42s ─",
            "buffer_text_sample": "The official full validation coverage is stale from 2026-05-06 20:40 IST.\n\n─ Worked for 1m 42s ─",
            "cursor_line_text": "",
            "xterm_present": true,
            "screen_present": true,
            "canvas_count": 4,
            "render_event_count": 69,
            "data_event_count": 1,
            "last_raw_payload_length": 7648,
            "last_raw_payload_line_count": 6,
            "write_command_count": 1,
            "terminal_content_source": "daemon_pty",
            "mounted_entry_host_connected": true,
            "host_stdin_enabled": true,
            "helper_textarea_focused": false,
            "host_has_active_element": false,
            "xterm_buffer_kind": "normal",
            "rows": 50,
            "cols": 110,
            "base_y": 6,
            "cursor_y": 47,
            "blank_rows_below_cursor": 2,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0}
        });
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(summary.get("problem"), Some(&Value::Null));
        assert_eq!(
            summary.get("raw_input_enabled").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary.get("foreground_input_ready").and_then(Value::as_bool),
            Some(false),
            "raw protocol bridge may stay open, but user input is not ready without prompt focus"
        );
    }

    #[test]
    fn terminal_host_problem_keeps_remote_prompt_only_surface_recovering_until_ready() {
        let host = json!({
            "session_path": "remote-session://dev/fresh-shell",
            "text_sample": "pi@dev:~$",
            "cursor_line_text": "pi@dev:~$",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "mounted_entry_host_connected": true,
            "render_event_count": 12,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is only showing a plain shell prompt")
        );
    }

    #[test]
    fn terminal_host_problem_allows_live_ssh_plain_prompt_when_unfocused() {
        let host = json!({
            "session_path": "live::practice-shell",
            "text_sample": "pi@practice:~$",
            "cursor_line_text": "pi@practice:~$ ",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "host_has_active_element": false,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "mounted_entry_host_connected": true,
            "render_event_count": 12,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(summary.get("problem"), Some(&Value::Null));
        assert_eq!(summary.get("live_problem"), Some(&Value::Null));
    }

    #[test]
    fn terminal_host_problem_allows_unfocused_visible_terminal_background_budget() {
        let host = json!({
            "session_path": "live::practice-shell",
            "text_sample": "normal terminal output",
            "cursor_line_text": "pi@practice:~$ ",
            "host_stdin_enabled": true,
            "effective_input_focus": false,
            "helper_textarea_focused": false,
            "host_has_active_element": false,
            "document_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "mounted_entry_host_connected": true,
            "render_event_count": 12,
            "data_event_count": 1,
            "active_write_frame_budget": false,
            "effective_terminal_write_frame_ms": 4000.0,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(summary.get("problem"), Some(&Value::Null));
        assert_eq!(summary.get("geometry_problem"), Some(&Value::Null));
    }

    #[test]
    fn terminal_host_problem_allows_idle_focused_visible_terminal_background_budget() {
        let host = json!({
            "session_path": "live::practice-shell",
            "text_sample": "normal terminal output",
            "cursor_line_text": "pi@practice:~$ ",
            "host_stdin_enabled": true,
            "effective_input_focus": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "document_focused": false,
            "terminal_input_hot": false,
            "recent_frame_like_write_hot": false,
            "recent_inline_status_animation_hot": false,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "mounted_entry_host_connected": true,
            "render_event_count": 12,
            "data_event_count": 1,
            "active_write_frame_budget": false,
            "effective_terminal_write_frame_ms": 4000.0,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
        let summary = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(summary.get("problem"), Some(&Value::Null));
        assert_eq!(summary.get("geometry_problem"), Some(&Value::Null));
        assert_eq!(
            summary.get("foreground_input_ready").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn terminal_host_problem_accepts_input_enabled_remote_tail_with_meaningful_output() {
        // XTERM-BUG: resume-gate-too-restrictive (commit 332072e). Sessions with
        // meaningful PTY output in `text_sample` are valid running sessions
        // even when input is enabled and no prompt is visible — the gate must
        // not reject them as "not ready". This used to assert Some(reason).
        let host = json!({
            "session_path": "remote-session://dev/stale-codex",
            "text_sample": "The final 2.1.59 artifacts are built. Before replacing the live jojo install, I’m taking one more runtime/install snapshot.",
            "text_tail": "The final 2.1.59 artifacts are built. Before replacing the live jojo install, I’m taking one more runtime/install snapshot.",
            "cursor_line_text": "stale scan processes.",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_connected_daemon_pty_output_without_prompt() {
        let host = json!({
            "session_path": "remote-session://dev/current-output",
            "text_sample": "Status as of 2026-05-13 11:26 IST:\n\nThe batch is no longer running and the validation coverage is stale.",
            "text_tail": "The official full validation coverage is stale from 2026-05-06 20:40 IST, still showing 48/1000 ready charts. So v3 readiness remains blocked until we restart or repair the batch.",
            "cursor_line_text": "",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 5,
            "last_raw_payload_length": 512,
            "last_raw_payload_line_count": 8,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "daemon_retained_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "mounted_entry_host_connected": true,
            "xterm_buffer_kind": "normal",
            "rows": 50,
            "cols": 110,
            "base_y": 963,
            "viewport_y": 963,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_input_enabled_daemon_pty_output_without_prompt() {
        // XTERM-BUG: resume-gate-too-restrictive (commit 332072e). An empty
        // cursor_line_text with meaningful PTY output in `text_sample` is a
        // legitimate "mid-output" running session; the gate must not reject it.
        // This used to assert Some(reason).
        let host = json!({
            "session_path": "remote-session://dev/current-output",
            "text_sample": "Status as of 2026-05-13 11:26 IST:\n\nThe batch is no longer running and the validation coverage is stale.",
            "text_tail": "The official full validation coverage is stale from 2026-05-06 20:40 IST, still showing 48/1000 ready charts. So v3 readiness remains blocked until we restart or repair the batch.",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 5,
            "last_raw_payload_length": 512,
            "last_raw_payload_line_count": 8,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "daemon_retained_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "mounted_entry_host_connected": true,
            "xterm_buffer_kind": "normal",
            "rows": 50,
            "cols": 110,
            "base_y": 963,
            "viewport_y": 963,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_daemon_pty_retained_scrollback_with_non_prompt_cursor() {
        // XTERM-BUG: resume-gate-too-restrictive (commit 332072e). Retained
        // daemon-PTY scrollback (93k chars / 1000 rows) with a non-prompt
        // cursor line is a real running session; the gate must not reject
        // it. This used to assert Some(reason).
        let host = json!({
            "session_path": "remote-session://dev/stale-codex",
            "text_sample": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n  - Physical PDFs in output dir: 2510",
            "text_tail": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n  - Physical PDFs in output dir: 2510\n\nThe current actionable improvement is not astrology logic yet.",
            "cursor_line_text": "  - Physical PDFs in output",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 11,
            "write_command_count": 3,
            "last_raw_payload_length": 93335,
            "last_raw_payload_line_count": 1000,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "daemon_screen_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "mounted_entry_host_connected": true,
            "scrollback_expected": true,
            "xterm_buffer_kind": "normal",
            "rows": 50,
            "cols": 110,
            "base_y": 1000,
            "viewport_y": 1000,
            "blank_rows_below_cursor": 16,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_large_daemon_pty_scrollback_with_blank_input_row() {
        let host = json!({
            "session_path": "remote-session://dev/stale-codex",
            "text_sample": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n  - Physical PDFs in output dir: 2510",
            "text_tail": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n  - Physical PDFs in output dir: 2510\n\nThe immediate engineering priority is still the harness: no fixture corpus, no rigorous parity measurement.",
            "buffer_text_sample": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n  - Physical PDFs in output dir: 2510",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 7,
            "write_command_count": 1,
            "last_raw_payload_length": 94404,
            "last_raw_payload_line_count": 1008,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "mounted_entry_host_connected": true,
            "scrollback_expected": false,
            "xterm_buffer_kind": "normal",
            "rows": 50,
            "cols": 110,
            "base_y": 1000,
            "viewport_y": 1000,
            "blank_rows_below_cursor": 16,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface has no current input row")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_daemon_pty_old_prompt_tail_with_enabled_input() {
        let host = json!({
            "session_path": "remote-session://dev/stale-codex",
            "text_sample": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n  - Physical PDFs in output dir: 2510",
            "text_tail": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742\n\nThe immediate engineering priority is still the harness: no fixture corpus, no rigorous parity measurement.\n \n \n› I’ll use the SAMPLENOTES v3 research workflow here and check the live PL9 batch before recommending parallel work. I’m going to distinguish runner health from v3 parity work so we do not confuse fixture generation progress with solved logic.git/samplenotes\n\nThe batch is still alive and the repaired open path is working, but the main friction has shifted: VP2+ generation is failing on some charts after the chart opens correctly.",
            "buffer_text_sample": "- Active chart: SAMPLENOTES_BENCH_0099\n  - Log rows: 742",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 8,
            "write_command_count": 2,
            "last_raw_payload_length": 1098,
            "last_raw_payload_line_count": 7,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "mounted_entry_host_connected": true,
            "scrollback_expected": false,
            "xterm_buffer_kind": "normal",
            "rows": 50,
            "cols": 110,
            "base_y": 1000,
            "viewport_y": 1000,
            "blank_rows_below_cursor": 1,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote Codex prompt surface has no current input row")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_input_enabled_interrupted_codex_input_surface() {
        let host = json!({
            "session_path": "remote-session://dev/interrupted-codex",
            "mounted_entry_host_connected": true,
            "text_sample": "• I found a concrete rename root cause.\n\n■ Conversation interrupted - tell the model what to do differently. Something went wrong? Hit `/feedback` to",
            "text_tail": "• I found a concrete rename root cause.\n\n■ Conversation interrupted - tell the model what to do differently. Something went wrong? Hit `/feedback` to",
            "cursor_line_text": "■ Conversation interrupted - tell the model what to do differently. Something went wrong? Hit `/feedback` to",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_input_disabled_stale_retained_remote_prose() {
        let stale_tail = "The commit and signed tag are pushed. I’m creating the GitHub release directly with the Linux installer archive, companion binaries, `.deb`, and checksums so the curl installer can resolve `v2.1.44` immediately; the tag workflow can still add any matrix artifacts afterward.";
        let host = json!({
            "session_path": "remote-session://dev/stale-codex",
            "text_sample": stale_tail,
            "text_tail": stale_tail,
            "buffer_text_sample": stale_tail,
            "cursor_line_text": "workflow can still add any matrix artifacts afterward.",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 0,
            "last_raw_payload_length": 886,
            "write_command_count": 1,
            "mounted_entry_host_connected": true,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some(
                "active remote terminal is showing stale retained text before prompt-ready surface"
            )
        );
    }

    #[test]
    fn terminal_host_problem_rejects_server_prompt_snapshot_as_terminal_source() {
        let stale_tail = "Killing the stale helper exposed another deterministic bug instead of hiding it: the 2.1.150 helper relaunched\nand the daemon now has two runtime keys for the same id.\n\n› Improve documentation in @filename\n\n  gpt-5.5 xhigh · ~/gh/yggterm";
        let host = json!({
            "session_path": "remote-session://dev/stale-codex",
            "text_sample": stale_tail,
            "text_tail": stale_tail,
            "buffer_text_sample": stale_tail,
            "cursor_line_text": "› Improve documentation in @filename",
            "terminal_content_source": "server_prompt_snapshot",
            "retained_replay_source": "server_prompt_snapshot",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 0,
            "last_raw_payload_length": 886,
            "write_command_count": 1,
            "mounted_entry_host_connected": true,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal is showing non-PTY server snapshot content")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_input_enabled_codex_status_surface_with_meaningful_output() {
        // XTERM-BUG: resume-gate-too-restrictive (commit 332072e). A Codex
        // /status banner is a meaningful PTY surface for a live session even
        // though no prompt char is on the cursor line. The gate must not
        // reject it. This used to assert Some(reason).
        let status = "\
>_ OpenAI Codex (v0.128.0)

Model:                       gpt-5.5
Directory:                   ~/gh/yggterm
Session:                     019dbdc7-7e63-7211-a7f8-51eb4d6e80b2

Context window:              22% left (205K used / 258K)
5h limit:                    55% left
Weekly limit:                97% left
";
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": status,
            "text_tail": status,
            "cursor_line_text": "Weekly limit:                97% left",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 3,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_status_prompt_in_text_tail_when_cursor_line_is_empty() {
        let tail = "\
• I found a concrete rename root cause.

/status

>_ OpenAI Codex (v0.128.0)

Model:                       gpt-5.5
Directory:                   ~/gh/yggterm
Session:                     019dbdc7-7e63-7211-a7f8-51eb4d6e80b2

Context window:              22% left (205K used / 258K)
5h limit:                    94% left
Weekly limit:                21% left

› Explain this codebase
";
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "• I found a concrete rename root cause.",
            "text_tail": tail,
            "buffer_text_sample": "• I found a concrete rename root cause.",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 20,
            "data_event_count": 0,
            "base_y": 541,
            "rows": 50,
            "blank_rows_below_cursor": 0,
            "scrollback_expected": true,
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_defers_scrollback_loss_on_collapsed_retained_grid() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "›",
            "text_tail": "›",
            "cursor_line_text": "›",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 20,
            "data_event_count": 5,
            "base_y": 0,
            "rows": 1,
            "cols": 2,
            "scrollback_expected": true,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 16.0, "height": 18.0},
            "host_content_width": 16.0,
            "host_content_height": 18.0,
            "screen_rect": {"width": 16.0, "height": 18.0},
            "viewport_rect": {"width": 16.0, "height": 18.0},
            "helpers_rect": {"width": 16.0, "height": 18.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_allows_prompt_ready_collapsed_scrollback_restore() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "› Write tests for @filename",
            "text_tail": "› Write tests for @filename",
            "cursor_line_text": "› Write tests for @filename",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 20,
            "data_event_count": 5,
            "base_y": 0,
            "rows": 50,
            "cols": 110,
            "blank_rows_below_cursor": 0,
            "scrollback_expected": true,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_flags_collapsed_scrollback_without_prompt_ready_surface() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "Previous retained output without a current prompt row",
            "text_tail": "Previous retained output without a current prompt row",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 20,
            "data_event_count": 5,
            "base_y": 0,
            "rows": 50,
            "cols": 110,
            "blank_rows_below_cursor": 0,
            "scrollback_expected": true,
            "xterm_buffer_kind": "normal",
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal lost expected scrollback after retained replay")
        );
    }

    #[test]
    fn terminal_host_problem_allows_prompt_ready_scroll_noop_without_scrollback() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "• I generated the SAMPLENOTES evidence pack.",
            "text_tail": "• I generated the SAMPLENOTES evidence pack.\n\nBest timing:\n- 23 Nov 2026 - 10 Jan 2027\n- 5 Aug 2027 - 4 Oct 2028\n\n› Improve documentation in @filename\n\ngpt-5.5 medium · ~/git/samplenotes",
            "buffer_text_sample": "• I generated the SAMPLENOTES evidence pack.",
            "cursor_line_text": "› Improve documentation in @filename",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 29,
            "data_event_count": 3,
            "base_y": 0,
            "rows": 63,
            "cols": 105,
            "blank_rows_below_cursor": 29,
            "scrollback_expected": false,
            "wheel_event_count": 436,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "last_raw_payload_length": 178,
            "last_raw_payload_line_count": 0,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "host_content_width": 883.0,
            "host_content_height": 904.0,
            "screen_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "viewport_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "helpers_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_flags_scroll_wheel_when_broken_prompt_has_no_scrollback() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "• I generated the SAMPLENOTES evidence pack.",
            "text_tail": "• I generated the SAMPLENOTES evidence pack.\n\nBest timing:\n- 23 Nov 2026 - 10 Jan 2027\n- 5 Aug 2027 - 4 Oct 2028\n\n› Improve documentation in @filename\n\ngpt-5.5 medium · ~/git/samplenotes",
            "buffer_text_sample": "• I generated the SAMPLENOTES evidence pack.",
            "cursor_line_text": "› Improve documentation in @filename",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 29,
            "data_event_count": 3,
            "base_y": 0,
            "rows": 50,
            "cols": 105,
            "blank_rows_below_cursor": 12,
            "scrollback_expected": false,
            "wheel_event_count": 436,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "",
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "last_raw_payload_length": 178,
            "last_raw_payload_line_count": 0,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "host_content_width": 883.0,
            "host_content_height": 904.0,
            "screen_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "viewport_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "helpers_rect": {"left": 277.0, "top": 8.0, "width": 883.0, "height": 904.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal received scroll input but has no xterm scrollback")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_empty_cursor_sample_when_current_input_row_visible() {
        let text_tail = "\
- Rust
- Self-hosting
- CLI tools
- AI tooling
- Systems

• It is fine sentimentally, but not strategically.

  A good compromise is:
  retire it with respect, not because it was bad, but because it has completed its term.

⚠ This session was recorded with model `gpt-5.4` but is resuming with `gpt-5.5`.

› Implement {feature}

  gpt-5.5 medium · ~/data/smbfs/dada/obsidian/codex";
        let host = json!({
            "session_path": "remote-session://oc/79ffb29d",
            "text_sample": text_tail,
            "text_tail": text_tail,
            "buffer_text_sample": text_tail,
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "render_event_count": 20,
            "data_event_count": 0,
            "last_raw_payload_length": 178,
            "last_raw_payload_line_count": 0,
            "last_raw_payload_sample": "\u{1b}[?2026h\u{1b}[48;3H\u{1b}[?2026l",
            "write_command_count": 1,
            "terminal_content_source": "daemon_pty",
            "retained_replay_source": "xterm_session_snapshot",
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "blank_rows_below_cursor": 0,
            "rows": 63,
            "cols": 159,
            "base_y": 955,
            "scrollback_expected": true,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "host_content_width": 1336.0,
            "host_content_height": 1144.0,
            "screen_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "viewport_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "helpers_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_prompt_ready_unsafe_retained_replay_skip() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "• Previous output\n\n› Run /review on my current changes\n \n  gpt-5.5 medium · ~/git/samplenotes",
            "text_tail": "• Previous output\n\n› Run /review on my current changes\n \n  gpt-5.5 medium · ~/git/samplenotes",
            "cursor_line_text": "› Run /review on my current changes",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 218,
            "data_event_count": 5,
            "base_y": 0,
            "rows": 50,
            "cols": 110,
            "blank_rows_below_cursor": 2,
            "scrollback_expected": true,
            "retained_replay_source": "daemon_retained_snapshot",
            "retained_replay_prompt_follow_ready": true,
            "retained_replay_unsafe_skip_prompt_ready": true,
            "last_raw_payload_line_count": 1000,
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal lost expected scrollback after retained replay")
        );
    }

    #[test]
    fn terminal_host_problem_allows_prompt_ready_retained_history_replay() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "Previous output\n\n› e s\n \n  gpt-5.5 xhigh · ~/git/samplenotes",
            "text_tail": "Previous output\n\n› e s\n \n  gpt-5.5 xhigh · ~/git/samplenotes",
            "cursor_line_text": "› e s",
            "host_stdin_enabled": true,
            "effective_input_focus": true,
            "helper_textarea_focused": true,
            "host_has_active_element": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 9,
            "data_event_count": 0,
            "base_y": 836,
            "rows": 50,
            "cols": 110,
            "blank_rows_below_cursor": 2,
            "scrollback_expected": false,
            "terminal_content_source": "daemon_retained_history_screen_snapshot",
            "last_raw_payload_line_count": 818,
            "last_raw_payload_length": 48885,
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
        let surface = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(surface.get("problem").and_then(Value::as_str), None);
        assert_eq!(
            surface.get("foreground_input_ready").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn terminal_host_problem_rejects_input_enabled_stale_retained_history_replay() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "Previous output\nCodex was working here",
            "text_tail": "Previous output\nCodex was working here",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "render_event_count": 9,
            "data_event_count": 0,
            "base_y": 12,
            "rows": 50,
            "cols": 110,
            "blank_rows_below_cursor": 12,
            "scrollback_expected": false,
            "terminal_content_source": "daemon_retained_history_screen_snapshot",
            "last_raw_payload_line_count": 12,
            "last_raw_payload_length": 1024,
            "xterm_buffer_kind": "normal",
            "mounted_entry_host_connected": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active remote terminal is input-enabled on retained history replay")
        );
        let surface = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            surface.get("problem").and_then(Value::as_str),
            Some("active remote terminal is input-enabled on retained history replay")
        );
        assert_eq!(
            surface.get("foreground_input_ready").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            surface.get("raw_input_enabled").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn terminal_host_problem_rejects_transparent_mounted_active_host() {
        let host = json!({
            "session_path": "remote-session://oc/live-codex",
            "text_sample": "Current PTY output\n\n› Summarize recent commits",
            "text_tail": "Current PTY output\n\n› Summarize recent commits",
            "cursor_line_text": "› Summarize recent commits",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "cursor_node_count": 1,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 0,
            "render_event_count": 8,
            "data_event_count": 1,
            "base_y": 955,
            "rows": 63,
            "cols": 159,
            "blank_rows_below_cursor": 4,
            "terminal_content_source": "daemon_pty",
            "host_opacity": "0",
            "host_visibility": "visible",
            "mounted_entry_host_connected": true,
            "host_rect": {"left": 277.0, "top": 8.0, "width": 1336.0, "height": 1144.0},
            "host_content_width": 1336.0,
            "host_content_height": 1144.0,
            "screen_rect": {"width": 1336.0, "height": 1144.0},
            "viewport_rect": {"width": 1336.0, "height": 1144.0},
            "helpers_rect": {"width": 1336.0, "height": 1144.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is transparent while mounted")
        );
        let surface = summarize_terminal_surface_for_app_control(&[host], false);
        assert_eq!(
            surface.get("problem").and_then(Value::as_str),
            Some("active terminal host is transparent while mounted")
        );
        assert_eq!(
            surface.get("foreground_input_ready").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn terminal_host_problem_rejects_input_enabled_transcript_browser_surface() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "/ T R A N S C R I P T /\n• Published v2.1.50: https://github.com/yggdrasilhq/yggterm/releases/tag/v2.1.50\n\n──────────────────── 87% ─\n ↑/↓ to scroll   pgup/pgdn to page   home/end to jump\n q to quit   esc/← to edit prev   → to edit next   enter to edit message",
            "cursor_line_text": " q to quit   esc/← to edit prev   → to edit next   enter to edit message",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": true,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is still showing the transcript browser")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_split_canvas_transcript_browser_surface() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "/ T R A N S C R I P T / / / / / / / / / / /\n  x86_64/arm64, and Debian artifacts. I’m checking the final release asset list.",
            "text_tail": "• Published v2.1.50: https://github.com/yggdrasilhq/yggterm/releases/tag/v2.1.50\n\n──────────────────────────────────── 87% ─\n ↑/↓ to scroll   pgup/pgdn to page   home/end to jump\n q to quit   esc/← to edit prev   → to edit next   enter to edit message",
            "buffer_text_sample": "/ T R A N S C R I P T / / / / / / / / / / /\n  x86_64/arm64, and Debian artifacts. I’m checking the final release asset list.",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 91,
            "data_event_count": 1,
            "xterm_buffer_kind": "alternate",
            "xterm_cursor_hidden": true,
            "host_rect": {"left": 277.0, "top": 40.0, "width": 883.0, "height": 872.0},
            "host_content_width": 883.0,
            "host_content_height": 872.0,
            "screen_rect": {"width": 883.0, "height": 872.0},
            "viewport_rect": {"width": 883.0, "height": 872.0},
            "helpers_rect": {"width": 883.0, "height": 872.0},
            "helper_textarea_rect": {"left": -9723.0, "top": 40.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is still showing the transcript browser")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_codex_role_transcript_surface() {
        let host = json!({
            "session_path": "remote-session://dev/live-codex",
            "text_sample": "USER:\nPlease fix yggterm.\n\nASSISTANT:\nI am checking the daemon state.",
            "text_tail": "USER:\nPlease fix yggterm.\n\nASSISTANT:\nI am checking the daemon state.",
            "buffer_text_sample": "USER:\nPlease fix yggterm.",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "cursor_node_count": 0,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 91,
            "data_event_count": 1,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": 277.0, "top": 40.0, "width": 883.0, "height": 872.0},
            "host_content_width": 883.0,
            "host_content_height": 872.0,
            "screen_rect": {"width": 883.0, "height": 872.0},
            "viewport_rect": {"width": 883.0, "height": 872.0},
            "helpers_rect": {"width": 883.0, "height": 872.0},
            "helper_textarea_rect": {"left": -9723.0, "top": 40.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is still showing the transcript browser")
        );
    }

    #[test]
    fn describe_viewport_snapshot_rejects_input_enabled_transcript_browser_surface() {
        let snapshot = json!({
            "active_session_path": "remote-session://dev/live-codex",
            "active_view_mode": "Terminal",
            "shell": {
                "terminal_attach_in_flight": [],
                "notifications": []
            },
            "active_surface_requests": []
        });
        let dom = json!({
            "titlebar_title_text": "yggterm",
            "titlebar_summary_text": "",
            "titlebar_button_tooltip": "",
            "titlebar_menu_open": false,
            "preview_text_sample": "",
            "preview_viewport_rect": null,
            "preview_visible_block_ids": [],
            "preview_font_family": "Inter",
            "preview_visible_entries": [],
            "preview_rendered_sections": [],
            "preview_fallback_context_visible": false,
            "preview_fallback_context_text": "",
            "preview_timestamp_labels": [],
            "preview_window": null,
            "shell_text_sample": "",
            "document_editor_count": 0,
            "document_body_sample": "",
            "terminal_hosts": [{
                "session_path": "remote-session://dev/live-codex",
                "child_count": 1,
                "xterm_present": true,
                "screen_present": true,
                "viewport_present": true,
                "rows_present": false,
                "canvas_count": 4,
                "host_stdin_enabled": true,
                "helper_textarea_focused": true,
                "xterm_cursor_hidden": true,
                "text_sample": "/ T R A N S C R I P T /\n• Published v2.1.50",
                "text_tail": "──────────────────── 87% ─\n ↑/↓ to scroll   pgup/pgdn to page   home/end to jump\n q to quit   esc/← to edit prev   → to edit next   enter to edit message",
                "cursor_line_text": "",
                "resume_overlay_visible": false,
                "resume_overlay_text": "",
                "resume_overlay_excerpt": "",
                "resume_overlay_kind": "hidden",
                "resume_overlay_phase": "hidden",
                "resume_overlay_effective_failed": false
            }],
            "terminal_resume_overlay": {
                "visible": false,
                "text_sample": "",
                "excerpt": "",
                "kind": "",
                "phase": "hidden",
                "effective_failed": false
            },
            "preview_visible_block_count": 0,
            "preview_scroll_count": 1
        });
        let viewport = describe_viewport_snapshot(&snapshot, &dom);
        assert_eq!(viewport.get("ready").and_then(Value::as_bool), Some(false));
        assert_eq!(
            viewport.get("interactive").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            viewport
                .get("terminal_settled_kind")
                .and_then(Value::as_str),
            Some("problem")
        );
        assert_eq!(
            viewport
                .get("active_terminal_surface")
                .and_then(|surface| surface.get("problem")),
            Some(&Value::String(
                "active terminal host is still showing the transcript browser".to_string()
            ))
        );
    }

    #[test]
    fn describe_viewport_snapshot_rejects_focused_host_for_different_session() {
        let snapshot = json!({
            "active_session_path": "remote-session://practice/current",
            "active_session_source": "LiveSsh",
            "active_view_mode": "Terminal",
            "active_title": "practice",
            "shell": {
                "terminal_attach_in_flight": [],
                "notifications": []
            },
            "active_surface_requests": []
        });
        let dom = json!({
            "titlebar_title_text": "practice",
            "titlebar_summary_text": "",
            "titlebar_button_tooltip": "",
            "titlebar_menu_open": false,
            "preview_text_sample": "",
            "preview_viewport_rect": null,
            "preview_visible_block_ids": [],
            "preview_font_family": "Inter",
            "preview_visible_entries": [],
            "preview_rendered_sections": [],
            "preview_fallback_context_visible": false,
            "preview_fallback_context_text": "",
            "preview_timestamp_labels": [],
            "preview_window": null,
            "shell_text_sample": "",
            "document_editor_count": 0,
            "document_body_sample": "",
            "terminal_hosts": [{
                "session_path": "remote-session://dev/other",
                "child_count": 1,
                "xterm_present": true,
                "screen_present": true,
                "viewport_present": true,
                "rows_present": false,
                "canvas_count": 4,
                "host_stdin_enabled": true,
                "raw_input_enabled": true,
                "helper_textarea_focused": true,
                "host_has_active_element": true,
                "terminal_content_source": "daemon_pty",
                "text_sample": "› wrong session",
                "text_tail": "› wrong session",
                "cursor_line_text": "› wrong session",
                "resume_overlay_visible": false,
                "resume_overlay_text": "",
                "resume_overlay_excerpt": "",
                "resume_overlay_kind": "hidden",
                "resume_overlay_phase": "hidden",
                "resume_overlay_effective_failed": false
            }],
            "terminal_resume_overlay": {
                "visible": false,
                "text_sample": "",
                "excerpt": "",
                "kind": "",
                "phase": "hidden",
                "effective_failed": false
            },
            "preview_visible_block_count": 0,
            "preview_scroll_count": 0
        });
        let viewport = describe_viewport_snapshot(&snapshot, &dom);
        assert_eq!(viewport.get("ready").and_then(Value::as_bool), Some(false));
        assert_eq!(
            viewport
                .get("terminal_settled_kind")
                .and_then(Value::as_str),
            Some("problem")
        );
        assert_eq!(
            viewport.get("reason").and_then(Value::as_str),
            Some(
                "active terminal host identity mismatch: selected remote-session://practice/current but focused host belongs to remote-session://dev/other"
            )
        );
        assert_eq!(
            viewport
                .get("active_terminal_identity_problem")
                .and_then(Value::as_str),
            viewport.get("reason").and_then(Value::as_str)
        );
    }

    #[test]
    fn describe_viewport_snapshot_ignores_stale_retained_helper_focus_for_different_session() {
        let snapshot = json!({
            "active_session_path": "remote-session://practice/current",
            "active_session_source": "LiveSsh",
            "active_view_mode": "Terminal",
            "active_title": "practice",
            "shell": {
                "terminal_attach_in_flight": [],
                "notifications": []
            },
            "active_surface_requests": []
        });
        let dom = json!({
            "titlebar_title_text": "practice",
            "titlebar_summary_text": "",
            "titlebar_button_tooltip": "",
            "titlebar_menu_open": false,
            "preview_text_sample": "",
            "preview_viewport_rect": null,
            "preview_visible_block_ids": [],
            "preview_font_family": "Inter",
            "preview_visible_entries": [],
            "preview_rendered_sections": [],
            "preview_fallback_context_visible": false,
            "preview_fallback_context_text": "",
            "preview_timestamp_labels": [],
            "preview_window": null,
            "shell_text_sample": "",
            "document_editor_count": 0,
            "document_body_sample": "",
            "terminal_hosts": [
                {
                    "host_id": "practice-host",
                    "session_path": "remote-session://practice/current",
                    "child_count": 1,
                    "xterm_present": true,
                    "screen_present": true,
                    "viewport_present": true,
                    "rows_present": true,
                    "canvas_count": 4,
                    "host_stdin_enabled": true,
                    "raw_input_enabled": true,
                    "effective_input_focus": true,
                    "helper_textarea_focused": true,
                    "host_has_active_element": true,
                    "document_focused": true,
                    "is_active_session_host": true,
                    "active": true,
                    "mounted_entry_host_connected": true,
                    "terminal_content_source": "daemon_pty",
                    "text_sample": "Done. Added these in the ThinkBook x layer:\n- x+q -> play/pause\n› git commit/push\n• Committed and pushed.",
                    "text_tail": "Done. Added these in the ThinkBook x layer:\n- x+q -> play/pause\n› git commit/push\n• Committed and pushed.",
                    "cursor_line_text": "› git commit/push",
                    "data_event_count": 4,
                    "last_raw_payload_length": 48,
                    "last_raw_payload_line_count": 1,
                    "rows": 30,
                    "cols": 120,
                    "cursor_y": 29,
                    "cursor_node_count": 1,
                    "resume_overlay_visible": false,
                    "resume_overlay_text": "",
                    "resume_overlay_excerpt": "",
                    "resume_overlay_kind": "hidden",
                    "resume_overlay_phase": "hidden",
                    "resume_overlay_effective_failed": false
                },
                {
                    "host_id": "dev-retained-host",
                    "session_path": "remote-session://dev/other",
                    "child_count": 1,
                    "xterm_present": true,
                    "screen_present": true,
                    "viewport_present": true,
                    "rows_present": true,
                    "canvas_count": 4,
                    "host_stdin_enabled": false,
                    "raw_input_enabled": false,
                    "effective_input_focus": false,
                    "helper_textarea_focused": true,
                    "host_has_active_element": true,
                    "document_focused": false,
                    "is_active_session_host": false,
                    "active": false,
                    "terminal_content_source": "daemon_pty",
                    "text_sample": "› stale retained dev host",
                    "text_tail": "› stale retained dev host",
                    "cursor_line_text": "› stale retained dev host",
                    "resume_overlay_visible": false,
                    "resume_overlay_text": "",
                    "resume_overlay_excerpt": "",
                    "resume_overlay_kind": "hidden",
                    "resume_overlay_phase": "hidden",
                    "resume_overlay_effective_failed": false
                }
            ],
            "terminal_resume_overlay": {
                "visible": false,
                "text_sample": "",
                "excerpt": "",
                "kind": "",
                "phase": "hidden",
                "effective_failed": false
            },
            "preview_visible_block_count": 0,
            "preview_scroll_count": 0
        });
        let viewport = describe_viewport_snapshot(&snapshot, &dom);
        assert_eq!(
            viewport
                .get("active_terminal_identity_problem")
                .and_then(Value::as_str),
            None
        );
        assert_eq!(viewport.get("ready").and_then(Value::as_bool), Some(true));
        assert_eq!(
            viewport
                .get("terminal_settled_kind")
                .and_then(Value::as_str),
            Some("interactive")
        );
    }

    #[test]
    fn viewport_accepts_remote_live_web_view() {
        let snapshot = json!({
            "active_session_path": "remote-session://dev/live-codex",
            "active_session_source": "LiveSsh",
            "active_view_mode": "Rendered",
            "active_title": "Debug Live",
            "shell": {
                "terminal_attach_in_flight": [],
                "notifications": []
            },
            "active_surface_requests": []
        });
        let dom = json!({
            "titlebar_title_text": "Debug Live",
            "titlebar_summary_text": "",
            "titlebar_button_tooltip": "",
            "titlebar_menu_open": false,
            "preview_text_sample": "May 03, 2026 11:15PM UTC+0530\nThis Codex session stays attached to the daemon and opens inline in the main terminal viewport.",
            "preview_viewport_rect": {"left": 297.0, "top": 70.0, "width": 843.0, "height": 202.0},
            "preview_visible_block_ids": ["preview-block-live-0"],
            "preview_font_family": "Inter",
            "preview_visible_entries": [],
            "preview_rendered_sections": [],
            "preview_fallback_context_visible": false,
            "preview_fallback_context_text": "",
            "preview_timestamp_labels": ["May 03, 2026 11:15PM UTC+0530"],
            "preview_window": null,
            "shell_text_sample": "",
            "document_editor_count": 0,
            "document_body_sample": "",
            "terminal_hosts": [],
            "terminal_resume_overlay": {
                "visible": false,
                "text_sample": "",
                "excerpt": "",
                "kind": "",
                "phase": "hidden",
                "effective_failed": false
            },
            "preview_visible_block_count": 1,
            "preview_scroll_count": 1
        });
        let viewport = describe_viewport_snapshot(&snapshot, &dom);
        assert_eq!(viewport.get("ready").and_then(Value::as_bool), Some(true));
        assert_eq!(
            viewport
                .get("terminal_settled_kind")
                .and_then(Value::as_str),
            Some("preview")
        );
        assert_eq!(viewport.get("reason").and_then(Value::as_str), None);
    }

    #[test]
    fn terminal_host_problem_rejects_offscreen_active_host() {
        let host = json!({
            "text_sample": "",
            "cursor_line_text": "",
            "host_stdin_enabled": false,
            "helper_textarea_focused": false,
            "xterm_present": true,
            "screen_present": true,
            "viewport_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 6,
            "data_event_count": 0,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": -2643.0, "top": 16.0, "width": 883.0, "height": 904.0},
            "host_content_width": 883.0,
            "host_content_height": 904.0,
            "screen_rect": {"width": 883.0, "height": 904.0},
            "viewport_rect": {"width": 883.0, "height": 904.0},
            "helpers_rect": {"width": 883.0, "height": 904.0},
            "helper_textarea_rect": {"left": -12643.0, "top": 16.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is mounted offscreen")
        );
    }

    #[test]
    fn terminal_host_problem_rejects_blank_normal_buffer_surface() {
        let host = json!({
            "text_sample": "",
            "cursor_line_text": "",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 1,
            "render_event_count": 3,
            "data_event_count": 1,
            "blank_rows_below_cursor": 28,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host exists but xterm surface is empty")
        );
    }

    #[test]
    fn terminal_host_problem_accepts_canvas_prompt_from_buffer_when_dom_rows_absent() {
        let host = json!({
            "text_sample": "",
            "cursor_line_text": "pi@jojo:~$",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "xterm_present": true,
            "screen_present": true,
            "rows_present": false,
            "canvas_count": 4,
            "render_event_count": 12,
            "data_event_count": 1,
            "blank_rows_below_cursor": 28,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_accepts_visually_hidden_offscreen_helper_textarea() {
        let host = json!({
            "text_sample": "pi@dev:~$",
            "cursor_line_text": "pi@dev:~$",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "helper_textarea_present": true,
            "helper_textarea_opacity": "0",
            "helper_textarea_background": "rgba(0, 0, 0, 0)",
            "helper_textarea_outline_style": "none",
            "helper_textarea_outline_color": "rgba(0, 0, 0, 0)",
            "helper_textarea_box_shadow": "none",
            "helper_textarea_clip_path": "inset(50%)",
            "helper_textarea_clip": "rect(0px, 0px, 0px, 0px)",
            "helper_textarea_pointer_events": "none",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 1,
            "render_event_count": 3,
            "data_event_count": 1,
            "blank_rows_below_cursor": 28,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": 304.0, "top": 68.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": -10000.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(terminal_host_problem_for_app_control(&host), None);
    }

    #[test]
    fn terminal_host_problem_rejects_hidden_helper_textarea_anchored_inside_host() {
        let host = json!({
            "text_sample": "pi@dev:~$",
            "cursor_line_text": "pi@dev:~$",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "helper_textarea_present": true,
            "helper_textarea_opacity": "0",
            "helper_textarea_background": "rgba(0, 0, 0, 0)",
            "helper_textarea_outline_style": "none",
            "helper_textarea_outline_color": "rgba(0, 0, 0, 0)",
            "helper_textarea_box_shadow": "none",
            "helper_textarea_clip_path": "inset(50%)",
            "helper_textarea_clip": "rect(0px, 0px, 0px, 0px)",
            "helper_textarea_pointer_events": "none",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 1,
            "render_event_count": 3,
            "data_event_count": 1,
            "blank_rows_below_cursor": 28,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": 304.0, "top": 68.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 304.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some(
                "active terminal host helper textarea drifted outside the expected hidden contract"
            )
        );
    }

    #[test]
    fn terminal_host_problem_rejects_visibly_mounted_helper_textarea() {
        let host = json!({
            "text_sample": "pi@dev:~$",
            "cursor_line_text": "pi@dev:~$",
            "host_stdin_enabled": true,
            "helper_textarea_focused": true,
            "helper_textarea_present": true,
            "helper_textarea_opacity": "1",
            "helper_textarea_background": "rgb(255, 255, 255)",
            "helper_textarea_outline_style": "auto",
            "helper_textarea_outline_color": "rgb(59, 130, 246)",
            "helper_textarea_box_shadow": "rgb(59, 130, 246) 0px 0px 0px 1px",
            "helper_textarea_clip_path": "none",
            "helper_textarea_clip": "auto",
            "helper_textarea_pointer_events": "auto",
            "xterm_present": true,
            "screen_present": true,
            "rows_present": true,
            "canvas_count": 1,
            "render_event_count": 3,
            "data_event_count": 1,
            "blank_rows_below_cursor": 28,
            "xterm_buffer_kind": "normal",
            "xterm_cursor_hidden": false,
            "host_rect": {"left": 304.0, "top": 68.0, "width": 840.0, "height": 830.0},
            "host_content_width": 840.0,
            "host_content_height": 830.0,
            "screen_rect": {"width": 840.0, "height": 830.0},
            "viewport_rect": {"width": 840.0, "height": 830.0},
            "helpers_rect": {"width": 840.0, "height": 830.0},
            "helper_textarea_rect": {"left": 304.0, "top": 68.0, "width": 1.0, "height": 1.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some(
                "active terminal host helper textarea is visibly mounted instead of visually hidden"
            )
        );
    }

    #[test]
    fn transport_error_detector_ignores_saved_transcript_connection_text() {
        let data = "\
The recovered notes explain why a previous SSH connection to dev timed out during a clone.
That sentence is part of the saved Codex transcript, not the live terminal transport.
The prompt stayed usable after the restore.
";
        assert!(!terminal_chunk_is_transport_error(data));
    }

    #[test]
    fn transport_error_detector_ignores_bulleted_saved_transcript_ssh_failure() {
        let data = "\
I did not cut a release.
The final rerun of the rebuilt Linux smoke wrapper was interrupted by an external network failure, not by Yggterm or Plasma:
- ssh: connect to host 192.0.2.10 port 22: No route to host
So the current state is:
";
        assert!(!terminal_chunk_is_transport_error(data));
        assert!(terminal_chunk_has_meaningful_output(data));
    }

    #[test]
    fn transport_error_detector_keeps_line_shaped_shared_connection_failure() {
        let data = "Shared connection to 192.0.2.10 closed.\r\n";
        assert!(terminal_chunk_is_transport_error(data));
    }

    #[test]
    fn transport_error_detector_keeps_wrapped_shared_connection_fragment() {
        let data = "Shared connection to 192.0.2.\r\n";
        assert!(terminal_chunk_is_transport_error(data));
    }

    #[test]
    fn transport_error_detector_keeps_line_shaped_ssh_failure() {
        let data = "ssh: connect to host 192.0.2.10 port 22: No route to host\r\n";
        assert!(terminal_chunk_is_transport_error(data));
    }

    #[test]
    fn transport_error_detector_flags_tail_internal_session_not_found() {
        let data = "\
I inspected the active session and the prompt is still usable.
The previous command output stays in scrollback.
There are enough normal rows before the stale transport line.
The old detector only sampled the head of the terminal text.
This needs to behave like a bottom-of-viewport problem.
› Error: terminal session not found: local://019d0000-0000-7000-8000-000000000001
Shared connection to 192.0.2.14 closed.
›
";
        assert!(terminal_chunk_is_transport_error(data));
    }

    #[test]
    fn transport_error_detector_ignores_prose_about_missing_sessions() {
        let data = "\
The fix should explain why a terminal session not found message leaked into the viewport.
That sentence is part of the saved Codex transcript and is not a live transport error line.
";
        assert!(!terminal_chunk_is_transport_error(data));
    }

    #[test]
    fn local_codex_scaffold_is_not_meaningful_terminal_output() {
        let data = "> This Codex session stays attached to the daemon and opens inline in the main terminal viewport.\r\n\r\nCodex is launched locally and will receive /quit when the daemon shuts down.\r\n";
        assert!(terminal_chunk_is_local_codex_scaffold(data));
        assert!(!terminal_chunk_has_meaningful_output(data));

        let host = json!({
            "session_path": "remote-session://dev/fresh",
            "text_sample": data,
            "text_tail": data,
            "xterm_present": true,
            "screen_present": true,
            "canvas_count": 1,
            "host_content_width": 800.0,
            "host_content_height": 600.0,
            "host_rect": {"left": 0.0, "top": 0.0, "width": 800.0, "height": 600.0}
        });
        assert_eq!(
            terminal_host_problem_for_app_control(&host),
            Some("active terminal host is still showing local Codex scaffold content")
        );
    }

    #[test]
    fn local_codex_scaffold_detector_allows_real_boot_output() {
        let data = "> This Codex session stays attached to the daemon and opens inline in the main terminal viewport.\r\n\r\nCodex is launched locally and will receive /quit when the daemon shuts down.\r\n\r\n• Booting MCP server\r\n";
        assert!(!terminal_chunk_is_local_codex_scaffold(data));
        assert!(terminal_chunk_has_meaningful_output(data));
    }

    #[test]
    fn terminal_bootstrap_activation_epoch_tracks_active_terminal_session_focus_cycles() {
        assert_eq!(
            terminal_bootstrap_activation_epoch(
                WorkspaceViewMode::Terminal,
                Some("local://active"),
                "local://active",
                42,
            ),
            42
        );
        assert_eq!(
            terminal_bootstrap_activation_epoch(
                WorkspaceViewMode::Terminal,
                Some("local://other"),
                "local://active",
                42,
            ),
            0
        );
        assert_eq!(
            terminal_bootstrap_activation_epoch(
                WorkspaceViewMode::Rendered,
                Some("local://active"),
                "local://active",
                42,
            ),
            0
        );
    }
}
