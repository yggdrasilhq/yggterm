//! The OpenCode tab mirror — one yggterm row per open opencode2 session tab.
//!
//! opencode2 is client-server: the row's PTY hosts a TUI (a window), the
//! background service owns the sessions, and one window renders N tabs
//! (docs/cli-integration.md, Issue Heading 26). The mirror keeps yggterm's
//! one-row-per-session invariant: every ACTIVE (open-tab) session gets a real
//! row, keyed by its service id, seated under the opencode anchor row, with
//! the launch line `opencode2 --session <ses_id>`.
//!
//! Why real rows and not a side table: opencode2's service is BUILT for
//! several windows on one session (measured 2026-08-29 — a second
//! `opencode2 --session` on a session another TUI had open painted the same
//! conversation, 447 KB of it, with no conflict). The identity catastrophe
//! ("two processes, one session id") belongs to CLIs whose PROCESS owns the
//! conversation; here the service owns it. So a tab row is an ordinary row
//! and every verb — monitor, booter, submit, context gauge — works on it
//! with no special case.
//!
//! The service is the truth; this mirror is a projection. Rows the mirror
//! created are marked `Source: opencode-tab-mirror` and are the ONLY rows it
//! may retire. A row the user engaged (opened — a PTY exists) is never
//! retired for leaving the active set: it has become a window, and windows
//! close when the user closes them.

use crate::YggtermServer;
use yggterm_core::opencode_service::OpencodeServiceSession;

pub const TAB_SOURCE_METADATA: &str = "opencode-tab-mirror";
pub const MIRROR_INTERVAL_MS: u64 = 5_000;
pub const TAB_SESSION_ID_METADATA: &str = "Tab Session Id";
pub const SPAWN_BUDGET_PER_TICK: usize = 1;
const VIEWED_METADATA: &str = "Tab Viewed Ms";

/// What a sync tick should do, decided as a pure function so the diff is
/// testable without a service, a daemon, or rows.
#[derive(Debug, Default, PartialEq)]
pub struct TabSyncPlan {
    /// Active sessions with no mirror row yet.
    pub spawn: Vec<OpencodeServiceSession>,
    /// Session ids whose tab closed AND whose row was never engaged.
    pub retire: Vec<String>,
    /// The tab row key the human just focused — viewed recency moved to a
    /// session this mirror already mirrors.
    pub focus: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct OwnedTab {
    pub(crate) key: String,
    pub(crate) viewed_epoch_ms: u128,
    pub(crate) engaged: bool,
}

/// The mirror's own rows, read back OUT of the row plane (Source metadata) —
/// the mirror keeps no second bookkeeping, so its state and the rows can
/// never disagree.
fn owned_tabs_from(
    sessions: &std::collections::BTreeMap<String, crate::ManagedSessionView>,
) -> std::collections::HashMap<String, OwnedTab> {
    let mut out = std::collections::HashMap::new();
    for (key, session) in sessions {
        let is_mirror = session
            .metadata
            .iter()
            .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA);
        if !is_mirror {
            continue;
        }
        let Some(ses) = session
            .metadata
            .iter()
            .find(|m| m.label == TAB_SESSION_ID_METADATA)
            .map(|m| m.value.clone())
        else {
            continue;
        };
        let engaged = session.terminal_process_id.is_some()
            || matches!(
                session.launch_phase,
                crate::TerminalLaunchPhase::Running
                    | crate::TerminalLaunchPhase::RemoteBootstrap
                    | crate::TerminalLaunchPhase::BridgePending
            );
        let viewed = session
            .metadata
            .iter()
            .find(|m| m.label == VIEWED_METADATA)
            .and_then(|m| m.value.parse::<u128>().ok())
            .unwrap_or(0);
        out.insert(ses, OwnedTab {
            key: key.clone(),
            viewed_epoch_ms: viewed,
            engaged,
        });
    }
    out
}

pub fn plan_tab_sync(
    active: &[OpencodeServiceSession],
    owned: &std::collections::HashMap<String, OwnedTab>,
) -> TabSyncPlan {
    let mut spawn = Vec::new();
    for ses in active {
        if !owned.contains_key(&ses.id) {
            spawn.push(ses.clone());
        }
    }
    let mut retire = Vec::new();
    for (ses, tab) in owned {
        if !active.iter().any(|s| &s.id == ses) && !tab.engaged {
            retire.push(ses.clone());
        }
    }
    // Focus-follow: the human's focused tab is the most recently VIEWED one,
    // and following is due only when that view moved PAST what this mirror
    // has already recorded for the row — a no-op on quiet ticks.
    let mut focus = None;
    if let Some(newest) = active
        .iter()
        .filter(|s| s.viewed_epoch_ms > 0)
        .max_by_key(|s| s.viewed_epoch_ms)
    {
        if let Some(tab) = owned.get(&newest.id) {
            if newest.viewed_epoch_ms > tab.viewed_epoch_ms {
                focus = Some(newest.id.clone());
            }
        }
    }
    TabSyncPlan {
        spawn,
        retire,
        focus,
    }
}

impl YggtermServer {
    /// Apply one mirror tick UNDER THE DAEMON LOCK. All service IO happened
    /// before this call (`fetch` in the chore, which holds no lock).
    pub fn apply_opencode_tab_mirror(
        &mut self,
        active: &[OpencodeServiceSession],
    ) {
        let owned = owned_tabs_from(&self.sessions);
        let plan = plan_tab_sync(active, &owned);
        let mut spawned = 0usize;
        let mut retired = 0usize;
        for ses in plan.spawn.iter().take(SPAWN_BUDGET_PER_TICK) {
            // ⛔ SILENT INSERT, never `ensure_remote_runtime_agent_session`:
            // ensure ACTIVATES the row and flips the workspace to Terminal —
            // for a bulk mirror that is an activation and mount storm (4
            // activations per tick, measured 2026-08-29: open-attempt ×378,
            // identity sync errors ×1100). A mirrored tab is a PROJECTION
            // until the user opens it: born Queued, no PTY, no activation.
            // The launch line resumes the session by its real service id, so
            // a click opens a window onto exactly that conversation
            // (multi-client is opencode2's native design).
            let key = format!("opencode-runtime://{}", ses.id);
            if self.sessions.contains_key(&key) {
                continue;
            }
            let target = crate::local_session_target(
                crate::SessionKind::OpenCode,
                ses.directory.as_deref(),
            );
            let fallback_title = format!("OpenCode tab {}", &ses.id[..ses.id.len().min(12)]);
            self.insert_live_session_with_launch(
                &key,
                &ses.id,
                crate::SessionKind::OpenCode,
                &target,
                Some(
                    ses.title
                        .clone()
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or(fallback_title),
                ),
                false,
                false,
            );
            if let Some(session) = self.sessions.get_mut(&key) {
                session.launch_command =
                    crate::remote_persistent_resume_shell_command_with_terminal_appearance(
                        crate::SessionKind::OpenCode,
                        &ses.id,
                        ses.directory.as_deref(),
                        None,
                    );
                session.launch_phase = crate::TerminalLaunchPhase::Queued;
                session.remote_deploy_state = crate::RemoteDeployState::NotRequired;
                crate::upsert_session_metadata(
                    &mut session.metadata,
                    "Source",
                    TAB_SOURCE_METADATA.to_string(),
                );
                crate::upsert_session_metadata(
                    &mut session.metadata,
                    TAB_SESSION_ID_METADATA,
                    ses.id.clone(),
                );
                if let Some(dir) = &ses.directory {
                    crate::upsert_session_metadata(&mut session.metadata, "Cwd", dir.clone());
                }
                if ses.viewed_epoch_ms > 0 {
                    crate::upsert_session_metadata(
                        &mut session.metadata,
                        VIEWED_METADATA,
                        ses.viewed_epoch_ms.to_string(),
                    );
                }
            }
            spawned += 1;
            if let Some(session) = self.sessions.get_mut(&key) {
                if let Some(title) = ses.title.as_deref().filter(|t| !t.trim().is_empty()) {
                    session.title = title.to_string();
                }
                crate::upsert_session_metadata(
                    &mut session.metadata,
                    "Source",
                    TAB_SOURCE_METADATA.to_string(),
                );
                if let Some(dir) = &ses.directory {
                    crate::upsert_session_metadata(&mut session.metadata, "Cwd", dir.clone());
                }
                if ses.viewed_epoch_ms > 0 {
                    crate::upsert_session_metadata(
                        &mut session.metadata,
                        VIEWED_METADATA,
                        ses.viewed_epoch_ms.to_string(),
                    );
                }
            }
            // Seat under the anchor: the opencode TUI row, if one is live.
            // Adjacency is the owner's rule — a tab appears directly below
            // the opencode row it belongs to.
            if let Some(prefix) = self.next_opencode_tab_seat() {
                self.set_session_outline_prefix(&key, &prefix);
            }
        }
        for ses in &plan.retire {
            let key = owned.get(ses).map(|t| t.key.clone());
            let Some(key) = key else { continue };
            if self.remove_live_session(&key).unwrap_or(false) {
                retired += 1;
            }
        }
        let mut focused = None;
        if let Some(ses_id) = &plan.focus {
            // Follow the human's tab switch only while they are already in
            // the opencode context (the anchor or a mirrored tab is the
            // active row); elsewhere in the GUI a tab switch must not yank
            // the viewport.
            let in_context = self
                .active_session_path
                .as_deref()
                .map(|current: &str| {
                    current.starts_with("opencode-runtime://")
                        || self.sessions.get(current).is_some_and(|s| {
                            s.metadata
                                .iter()
                                .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
                        })
                })
                .unwrap_or(false);
            if in_context {
                focused = owned
                    .get(ses_id)
                    .map(|t| t.key.clone())
                    .or_else(|| self.sessions.keys().find(|k| k.contains(ses_id)).cloned());
                if let Some(key) = focused.clone() {
                    self.set_active_session_path(
                        Some(key),
                        crate::ActivationOrigin {
                            kind: crate::ActivationOriginKind::AppControl,
                            site: "opencode_tab_mirror_focus",
                        },
                    );
                }
            }
            // Record the view we followed so the next tick compares against
            // it instead of re-following every tick.
            if let Some(newest) = active.iter().find(|s| &s.id == ses_id) {
                let key = owned
                    .get(ses_id)
                    .map(|t| t.key.clone())
                    .or_else(|| focused.clone());
                if let Some(key) = key {
                    if let Some(session) = self.sessions.get_mut(&key) {
                        if newest.viewed_epoch_ms > 0 {
                            crate::upsert_session_metadata(
                                &mut session.metadata,
                                VIEWED_METADATA,
                                newest.viewed_epoch_ms.to_string(),
                            );
                        }
                    }
                }
            }
        }
        if spawned > 0 || retired > 0 || plan.focus.is_some() {
            if let Ok(home_dir) = crate::resolve_yggterm_home() {
                yggterm_core::append_trace_event(
                    &home_dir,
                    "daemon",
                    "opencode_mirror",
                    "tab_sync",
                    serde_json::json!({
                        "spawned": spawned,
                        "retired": retired,
                        "focus": plan.focus,
                        "active_tabs": active.len(),
                    }),
                );
            }
        }
    }

    /// The live opencode TUI row (the mirror's seating anchor) and the next
    /// free sub-seat under it: `<anchor outline>.<n+1>`.
    fn next_opencode_tab_seat(&self) -> Option<String> {
        let anchor = self.sessions.values().find(|s| {
            s.kind == crate::SessionKind::OpenCode
                && s.session_path.starts_with("opencode-runtime://")
                && !s
                    .metadata
                    .iter()
                    .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
        })?;
        let base = anchor.outline_prefix.clone().unwrap_or_default();
        if base.is_empty() {
            return None;
        }
        let used = self
            .sessions
            .values()
            .filter(|s| {
                s.metadata
                    .iter()
                    .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
            })
            .count();
        Some(format!("{base}.{}", used + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ses(id: &str, viewed: u128) -> OpencodeServiceSession {
        OpencodeServiceSession {
            id: id.to_string(),
            title: Some(id.to_string()),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: viewed,
            viewed_epoch_ms: viewed,
            running: true,
        }
    }

    fn owned(
        ses: &str,
        _key: &str,
        viewed: u128,
        engaged: bool,
    ) -> (String, OwnedTab) {
        (
            ses.to_string(),
            OwnedTab {
                key: format!("opencode-runtime://{ses}"),
                viewed_epoch_ms: viewed,
                engaged,
            },
        )
    }

    #[test]
    fn a_new_tab_spawns_a_cold_row_and_a_closed_unengaged_tab_retires() {
        let active = vec![ses("ses_new000000000000000000001", 100)];
        let owned_map = std::collections::HashMap::from([owned(
            "ses_gone00000000000000000001",
            "opencode-runtime://ses_gone00000000000000000001",
            50,
            false,
        )]);
        let plan = plan_tab_sync(&active, &owned_map);
        assert_eq!(plan.spawn.len(), 1);
        assert_eq!(plan.spawn[0].id, "ses_new000000000000000000001");
        assert_eq!(plan.retire, vec!["ses_gone00000000000000000001"]);
        // Focus: the new session's view (100) moved past nothing we mirror —
        // no row exists for it yet, so nothing to follow this tick.
        assert_eq!(plan.focus, None);
    }

    #[test]
    fn an_engaged_row_is_never_retired_for_losing_its_tab() {
        let active = vec![]; // the tab closed everywhere
        let owned_map = std::collections::HashMap::from([owned(
            "ses_engaged00000000000000001",
            "opencode-runtime://ses_engaged00000000000000001",
            50,
            true, // the user opened it: it is a window now
        )]);
        let plan = plan_tab_sync(&active, &owned_map);
        assert!(plan.retire.is_empty(), "windows close when the user closes them");
        assert!(plan.spawn.is_empty());
    }

    #[test]
    fn focus_follows_a_freshly_viewed_session_and_stands_down_when_quiet() {
        let active = vec![ses("ses_mirrored0000000000000000001", 9_000)];
        let owned_map = std::collections::HashMap::from([owned(
            "ses_mirrored0000000000000000001",
            "opencode-runtime://ses_mirrored0000000000000000001",
            1_000,
            false,
        )]);
        let plan = plan_tab_sync(&active, &owned_map);
        assert_eq!(
            plan.focus.as_deref(),
            Some("ses_mirrored0000000000000000001"),
            "the human just focused this tab"
        );
        // After the mirror records the view, the same state is a no-op.
        let settled = std::collections::HashMap::from([owned(
            "ses_mirrored0000000000000000001",
            "opencode-runtime://ses_mirrored0000000000000000001",
            9_000,
            false,
        )]);
        let plan = plan_tab_sync(&active, &settled);
        assert_eq!(plan.focus, None, "quiet tick — nothing to follow");
    }
}
