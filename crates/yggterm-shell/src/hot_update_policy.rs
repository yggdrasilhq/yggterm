use serde_json::{Value, json};
use std::collections::HashSet;
use yggterm_core::current_version;
use yggterm_server::{ServerEndpoint, ServerRuntimeStatus};

fn daemon_version_triplet(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn daemon_versions_share_patch_line(left: &str, right: &str) -> bool {
    let Some((left_major, left_minor, _)) = daemon_version_triplet(left) else {
        return false;
    };
    let Some((right_major, right_minor, _)) = daemon_version_triplet(right) else {
        return false;
    };
    left_major == right_major && left_minor == right_minor
}

fn runtime_status_has_owned_terminal_runtime(runtime_status: &ServerRuntimeStatus) -> bool {
    if runtime_status.owned_terminal_session_count > 0
        || !runtime_status.owned_terminal_session_keys.is_empty()
    {
        return true;
    }
    let preserved_keys = runtime_status
        .preserved_terminal_owner_keys
        .iter()
        .collect::<HashSet<_>>();
    if runtime_status
        .terminal_session_keys
        .iter()
        .any(|key| !preserved_keys.contains(key))
    {
        return true;
    }
    if runtime_status.terminal_session_keys.is_empty()
        && runtime_status.preserved_terminal_owner_count == 0
        && runtime_status.preserved_terminal_owner_keys.is_empty()
        && runtime_status.terminal_session_count > 0
    {
        return true;
    }
    runtime_status
        .terminal_session_count
        .saturating_sub(runtime_status.preserved_terminal_owner_count)
        > 0
}

fn runtime_status_owned_runtime_is_authorized(
    runtime_status: &ServerRuntimeStatus,
    authorized_runtime_keys: Option<&HashSet<String>>,
) -> bool {
    let Some(authorized_runtime_keys) = authorized_runtime_keys else {
        return true;
    };
    if authorized_runtime_keys.is_empty() {
        return false;
    }
    if !runtime_status.owned_terminal_session_keys.is_empty() {
        return runtime_status
            .owned_terminal_session_keys
            .iter()
            .all(|key| authorized_runtime_keys.contains(key));
    }
    let preserved_keys = runtime_status
        .preserved_terminal_owner_keys
        .iter()
        .collect::<HashSet<_>>();
    let inferred_owned_keys = runtime_status
        .terminal_session_keys
        .iter()
        .filter(|key| !preserved_keys.contains(key))
        .collect::<Vec<_>>();
    if !inferred_owned_keys.is_empty() {
        return inferred_owned_keys
            .iter()
            .all(|key| authorized_runtime_keys.contains(*key));
    }
    runtime_status.owned_terminal_session_count == 0
        && runtime_status.terminal_session_count == runtime_status.preserved_terminal_owner_count
}

fn runtime_status_covers_authorized_runtime_keys(
    runtime_status: &ServerRuntimeStatus,
    authorized_runtime_keys: Option<&HashSet<String>>,
) -> bool {
    let Some(authorized_runtime_keys) = authorized_runtime_keys else {
        return true;
    };
    if authorized_runtime_keys.is_empty() {
        return false;
    }
    let represented_keys = runtime_status
        .terminal_session_keys
        .iter()
        .chain(runtime_status.owned_terminal_session_keys.iter())
        .chain(runtime_status.preserved_terminal_owner_keys.iter())
        .collect::<HashSet<_>>();
    authorized_runtime_keys
        .iter()
        .all(|key| represented_keys.contains(key))
}

fn runtime_status_owned_runtime_is_authorized_for_startup_target(
    runtime_status: &ServerRuntimeStatus,
    authorized_runtime_keys: Option<&HashSet<String>>,
    active_client_count: usize,
) -> bool {
    if runtime_status_owned_runtime_is_authorized(runtime_status, authorized_runtime_keys)
        && runtime_status_covers_authorized_runtime_keys(runtime_status, authorized_runtime_keys)
    {
        return true;
    }
    active_client_count > 0
        && !runtime_status.owned_terminal_session_keys.is_empty()
        && runtime_status_covers_authorized_runtime_keys(runtime_status, authorized_runtime_keys)
}

fn startup_daemon_should_preserve_stale_runtime_for_startup_target(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
    authorized_runtime_keys: Option<&HashSet<String>>,
    active_client_count: usize,
) -> bool {
    let expected_version = expected_version.trim();
    let runtime_version = runtime_status.server_version.trim();
    if expected_version.is_empty()
        || runtime_version.is_empty()
        || runtime_version == expected_version
    {
        return false;
    }
    daemon_versions_share_patch_line(runtime_version, expected_version)
        && runtime_status_has_owned_terminal_runtime(runtime_status)
        && runtime_status_owned_runtime_is_authorized_for_startup_target(
            runtime_status,
            authorized_runtime_keys,
            active_client_count,
        )
}

pub(crate) fn startup_daemon_hot_swap_reason_for_startup_target(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
    authorized_runtime_keys: Option<&HashSet<String>>,
    active_client_count: usize,
) -> Option<&'static str> {
    let expected_version = expected_version.trim();
    let runtime_version = runtime_status.server_version.trim();
    if expected_version.is_empty()
        || runtime_version.is_empty()
        || runtime_version == expected_version
    {
        return None;
    }
    if !runtime_status_covers_authorized_runtime_keys(runtime_status, authorized_runtime_keys) {
        return None;
    }
    if startup_daemon_should_preserve_stale_runtime_for_startup_target(
        runtime_status,
        expected_version,
        authorized_runtime_keys,
        active_client_count,
    ) {
        return Some("startup_hot_update_handoff");
    }
    if runtime_status_has_owned_terminal_runtime(runtime_status) {
        return None;
    }
    Some("startup_version_reconcile")
}

fn runtime_status_has_live_terminal_runtime(runtime_status: &ServerRuntimeStatus) -> bool {
    runtime_status_has_owned_terminal_runtime(runtime_status)
        || runtime_status.terminal_session_count > 0
        || !runtime_status.terminal_session_keys.is_empty()
        || runtime_status.preserved_terminal_owner_count > 0
        || !runtime_status.preserved_terminal_owner_keys.is_empty()
        || runtime_status.restored_live_sessions > 0
}

pub(crate) fn startup_daemon_should_preserve_stale_runtime(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
) -> bool {
    startup_daemon_should_preserve_stale_runtime_with_authorized_keys(
        runtime_status,
        expected_version,
        None,
    )
}

fn startup_daemon_should_preserve_stale_runtime_with_authorized_keys(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
    authorized_runtime_keys: Option<&HashSet<String>>,
) -> bool {
    let expected_version = expected_version.trim();
    let runtime_version = runtime_status.server_version.trim();
    if expected_version.is_empty()
        || runtime_version.is_empty()
        || runtime_version == expected_version
    {
        return false;
    }
    daemon_versions_share_patch_line(runtime_version, expected_version)
        && runtime_status_has_owned_terminal_runtime(runtime_status)
        && runtime_status_owned_runtime_is_authorized(runtime_status, authorized_runtime_keys)
}

pub(crate) fn runtime_status_is_current_app_version(runtime_status: &ServerRuntimeStatus) -> bool {
    runtime_status.server_version.as_str() == current_version().as_str()
}

pub(crate) fn runtime_status_matches_current_app(runtime_status: &ServerRuntimeStatus) -> bool {
    runtime_status_is_current_app_version(runtime_status)
}

pub(crate) fn startup_daemon_hot_update_pending_reason(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
) -> Option<&'static str> {
    if startup_daemon_should_preserve_stale_runtime(runtime_status, expected_version) {
        Some("session_survival_preserved_owner")
    } else {
        None
    }
}

#[cfg(test)]
pub(crate) fn startup_daemon_hot_swap_reason(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
) -> Option<&'static str> {
    startup_daemon_hot_swap_reason_with_authorized_keys(runtime_status, expected_version, None)
}

pub(crate) fn startup_daemon_hot_swap_reason_with_authorized_keys(
    runtime_status: &ServerRuntimeStatus,
    expected_version: &str,
    authorized_runtime_keys: Option<&HashSet<String>>,
) -> Option<&'static str> {
    let expected_version = expected_version.trim();
    let runtime_version = runtime_status.server_version.trim();
    if expected_version.is_empty()
        || runtime_version.is_empty()
        || runtime_version == expected_version
    {
        return None;
    }
    if startup_daemon_should_preserve_stale_runtime_with_authorized_keys(
        runtime_status,
        expected_version,
        authorized_runtime_keys,
    ) {
        return Some("startup_hot_update_handoff");
    }
    if runtime_status_has_owned_terminal_runtime(runtime_status) {
        return None;
    }
    Some("startup_version_reconcile")
}

fn startup_daemon_status_recoverable_weight(runtime_status: &ServerRuntimeStatus) -> usize {
    runtime_status
        .terminal_session_count
        .saturating_mul(4)
        .saturating_add(runtime_status.terminal_session_keys.len().saturating_mul(2))
        .saturating_add(runtime_status.restored_live_sessions)
        .saturating_add(runtime_status.managed_session_count)
}

fn startup_daemon_status_owned_runtime_weight_with_authorized_keys(
    runtime_status: &ServerRuntimeStatus,
    authorized_runtime_keys: Option<&HashSet<String>>,
) -> usize {
    if !runtime_status_owned_runtime_is_authorized(runtime_status, authorized_runtime_keys) {
        return 0;
    }
    runtime_status
        .owned_terminal_session_count
        .saturating_mul(4)
        .saturating_add(
            runtime_status
                .owned_terminal_session_keys
                .len()
                .saturating_mul(2),
        )
        .saturating_add(usize::from(runtime_status_has_owned_terminal_runtime(
            runtime_status,
        )))
}

fn startup_daemon_status_owned_runtime_weight_for_startup_target(
    runtime_status: &ServerRuntimeStatus,
    authorized_runtime_keys: Option<&HashSet<String>>,
    active_client_count: usize,
) -> usize {
    if runtime_status_owned_runtime_is_authorized(runtime_status, authorized_runtime_keys) {
        return startup_daemon_status_owned_runtime_weight_with_authorized_keys(
            runtime_status,
            authorized_runtime_keys,
        );
    }
    if active_client_count == 0 || runtime_status.owned_terminal_session_keys.is_empty() {
        return 0;
    }
    runtime_status
        .owned_terminal_session_count
        .saturating_mul(4)
        .saturating_add(
            runtime_status
                .owned_terminal_session_keys
                .len()
                .saturating_mul(2),
        )
        .saturating_add(usize::from(runtime_status_has_owned_terminal_runtime(
            runtime_status,
        )))
}

pub(crate) fn daemon_update_state_json(runtime_status: Option<&ServerRuntimeStatus>) -> Value {
    let expected_version = current_version();
    let Some(runtime_status) = runtime_status else {
        return json!({
            "state": "unknown",
            "current_gui_version": expected_version.as_str(),
            "active_daemon_version": Value::Null,
            "active_daemon_build_id": Value::Null,
            "active_daemon_pid": Value::Null,
            "version_mismatch": Value::Null,
            "session_survival_required": Value::Null,
            "hot_update_pending": Value::Null,
            "update_priority": "observe_only",
            "preserved_owner_pids": [],
            "preserved_runtime_keys": [],
        });
    };

    let active_version = runtime_status.server_version.as_str();
    let version_mismatch = active_version != expected_version.as_str();
    let session_survival_required = runtime_status_has_live_terminal_runtime(runtime_status);
    let handoff_active = runtime_status.preserved_terminal_owner_count > 0
        || !runtime_status.preserved_terminal_owner_keys.is_empty();
    let hot_update_pending =
        startup_daemon_hot_update_pending_reason(runtime_status, expected_version.as_str())
            .is_some();
    let state = if handoff_active {
        "hot_update_handoff_active"
    } else if !version_mismatch {
        "current"
    } else if hot_update_pending {
        "hot_update_pending"
    } else if session_survival_required {
        "stale_live_incompatible"
    } else {
        "stale_no_live"
    };
    let update_priority = if handoff_active {
        "handoff_preserve_sessions"
    } else if hot_update_pending {
        "defer_update_preserve_sessions"
    } else if !version_mismatch {
        "current"
    } else if session_survival_required {
        "manual_recovery_required_preserve_sessions"
    } else {
        "safe_to_restart"
    };
    let preserved_owner_pids = if hot_update_pending {
        vec![runtime_status.server_pid]
    } else {
        Vec::new()
    };
    let preserved_runtime_keys = if hot_update_pending {
        runtime_status.terminal_session_keys.clone()
    } else {
        Vec::new()
    };

    json!({
        "state": state,
        "current_gui_version": expected_version.as_str(),
        "active_daemon_version": active_version,
        "active_daemon_build_id": runtime_status.server_build_id,
        "active_daemon_pid": runtime_status.server_pid,
        "version_mismatch": version_mismatch,
        "session_survival_required": session_survival_required,
        "owned_terminal_session_count": runtime_status.owned_terminal_session_count,
        "owned_terminal_session_keys": &runtime_status.owned_terminal_session_keys,
        "terminal_session_count": runtime_status.terminal_session_count,
        "terminal_session_keys": &runtime_status.terminal_session_keys,
        "restored_live_sessions": runtime_status.restored_live_sessions,
        "managed_session_count": runtime_status.managed_session_count,
        "same_patch_line": daemon_versions_share_patch_line(active_version, expected_version.as_str()),
        "hot_update_pending": hot_update_pending,
        "hot_update_handoff_active": handoff_active,
        "hot_update_pending_reason": startup_daemon_hot_update_pending_reason(
            runtime_status,
            expected_version.as_str(),
        ),
        "update_priority": update_priority,
        "preserved_owner_pids": preserved_owner_pids,
        "preserved_runtime_keys": preserved_runtime_keys,
        "handoff_owner_count": runtime_status.preserved_terminal_owner_count,
        "handoff_runtime_keys": &runtime_status.preserved_terminal_owner_keys,
    })
}

#[cfg(test)]
pub(crate) fn startup_stale_daemon_hot_swap_target<I>(
    statuses: I,
    expected_version: &str,
) -> Option<(ServerEndpoint, ServerRuntimeStatus)>
where
    I: IntoIterator<Item = (ServerEndpoint, ServerRuntimeStatus)>,
{
    startup_stale_daemon_hot_swap_target_with_client_counter(statuses, expected_version, |_| 0)
}

#[cfg(test)]
pub(crate) fn startup_stale_daemon_hot_swap_target_with_client_counter<I, F>(
    statuses: I,
    expected_version: &str,
    active_client_count: F,
) -> Option<(ServerEndpoint, ServerRuntimeStatus)>
where
    I: IntoIterator<Item = (ServerEndpoint, ServerRuntimeStatus)>,
    F: Fn(&ServerEndpoint) -> usize,
{
    startup_stale_daemon_hot_swap_target_with_authorized_keys(
        statuses,
        expected_version,
        None,
        active_client_count,
    )
}

pub(crate) fn startup_stale_daemon_hot_swap_target_with_authorized_keys<I, F>(
    statuses: I,
    expected_version: &str,
    authorized_runtime_keys: Option<&HashSet<String>>,
    active_client_count: F,
) -> Option<(ServerEndpoint, ServerRuntimeStatus)>
where
    I: IntoIterator<Item = (ServerEndpoint, ServerRuntimeStatus)>,
    F: Fn(&ServerEndpoint) -> usize,
{
    statuses
        .into_iter()
        .filter(|(endpoint, runtime_status)| {
            startup_daemon_hot_swap_reason_for_startup_target(
                runtime_status,
                expected_version,
                authorized_runtime_keys,
                active_client_count(endpoint),
            )
            .is_some()
        })
        .max_by_key(|(endpoint, runtime_status)| {
            let active_clients = active_client_count(endpoint);
            let explicit_owned_runtime_authorized =
                !runtime_status.owned_terminal_session_keys.is_empty()
                    && runtime_status_owned_runtime_is_authorized_for_startup_target(
                        runtime_status,
                        authorized_runtime_keys,
                        active_clients,
                    );
            (
                usize::from(explicit_owned_runtime_authorized),
                usize::from(active_clients > 0),
                active_clients,
                usize::from(
                    startup_daemon_should_preserve_stale_runtime_for_startup_target(
                        runtime_status,
                        expected_version,
                        authorized_runtime_keys,
                        active_clients,
                    ),
                ),
                startup_daemon_status_owned_runtime_weight_for_startup_target(
                    runtime_status,
                    authorized_runtime_keys,
                    active_clients,
                ),
                usize::from(startup_daemon_status_recoverable_weight(runtime_status) > 0),
                startup_daemon_status_recoverable_weight(runtime_status),
                daemon_version_triplet(&runtime_status.server_version).unwrap_or((0, 0, 0)),
                runtime_status.server_build_id as usize,
                runtime_status.server_pid as usize,
            )
        })
}

pub(crate) fn startup_authorized_hot_update_runtime_keys_from_sources(
    state_keys: HashSet<String>,
    owner_keys: HashSet<String>,
) -> Option<HashSet<String>> {
    if !state_keys.is_empty() {
        return Some(state_keys);
    }
    if owner_keys.is_empty() {
        None
    } else {
        Some(owner_keys)
    }
}
