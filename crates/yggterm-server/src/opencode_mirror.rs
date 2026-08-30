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
/// The session id a row carries for this mirror, if it is one of ours.
///
/// ⛔ ADOPTION BY KEY SHAPE, measured 2026-08-29: rows created before the
/// stamp existed (and rows whose metadata did not survive a daemon takeover)
/// carry NO `Tab Session Id` — `owned` read 0 while 3 live mirror rows were
/// on screen, so every tab looked new forever and nothing converged. The key
/// shape is the identity: an OpenCode row keyed `opencode-runtime://ses_…`
/// embeds the SERVICE's own id (uuid-keyed rows are anchors or phantoms and
/// are never adopted).
fn mirror_tab_session_id(kind: crate::SessionKind, key: &str, stamped: Option<&str>) -> Option<String> {
    if kind != crate::SessionKind::OpenCode {
        return None;
    }
    if let Some(ses) = stamped.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(ses.to_string());
    }
    let rest = key.strip_prefix("opencode-runtime://")?;
    rest.starts_with("ses_").then(|| rest.to_string())
}

fn owned_tabs_from(
    sessions: &std::collections::BTreeMap<String, crate::ManagedSessionView>,
) -> std::collections::HashMap<String, OwnedTab> {
    let mut out = std::collections::HashMap::new();
    for (key, session) in sessions {
        let Some(ses) = mirror_tab_session_id(
            session.kind,
            key,
            session
                .metadata
                .iter()
                .find(|m| m.label == TAB_SESSION_ID_METADATA)
                .map(|m| m.value.as_str()),
        ) else {
            continue;
        };
        let is_mirror = session
            .metadata
            .iter()
            .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
            || session
                .metadata
                .iter()
                .any(|m| m.label == TAB_SESSION_ID_METADATA)
            || ses.starts_with("ses_");
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

/// The honest display title for a mirrored session. The v2 preview writes
/// the placeholder `New session - <iso>` until the first prompt lands, which
/// reads as a generic weird row name (owner, 2026-08-30) — for those, the
/// working directory's own name is the meaningful handle.
fn mirror_display_title(ses: &OpencodeServiceSession) -> Option<String> {
    let raw = ses.title.as_deref()?.trim();
    if raw.is_empty() || raw.starts_with("New session") {
        return None;
    }
    Some(raw.to_string())
}

fn directory_display_name(directory: Option<&String>) -> Option<String> {
    let dir = directory?;
    let name = dir.rsplit('/').find(|seg| !seg.is_empty())?;
    (!name.is_empty()).then(|| name.to_string())
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
        // ⛔ A tick whose silence is indistinguishable from not having run is
        // the §7 sin in mirror form: report EVERY tick — counts, not contents
        // (no ids, no titles) — so "why did nothing happen" is answerable
        // from the trace alone. Every 12th tick in detail, the rest a line.
        if let Ok(home_dir) = crate::resolve_yggterm_home() {
            static TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n % 12 == 0 {
                yggterm_core::append_trace_event(
                    &home_dir,
                    "daemon",
                    "opencode_mirror",
                    "tick_state",
                    serde_json::json!({
                        "active_tabs": active.len(),
                        "active_ids": active.iter().map(|s| s.id.len()).collect::<Vec<_>>().len(),
                        "owned": owned.len(),
                        "plan_spawn": plan.spawn.len(),
                        "plan_retire": plan.retire.len(),
                        "plan_focus": plan.focus.is_some(),
                    }),
                );
            }
        }
        let mut spawned = 0usize;
        let mut retired = 0usize;
        // ⛔ ADOPTION IS UNLIMITED; ONLY NEW INSERTS ARE BUDGETED. Daemon
        // takeovers restore rows WITHOUT their metadata (measured 2026-08-29:
        // owned fell 4→1 across a takeover), so after every generation the
        // whole active set re-enters plan.spawn as adoptions. Budgeting them
        // made post-takeover convergence crawl at one row per tick. Adoption
        // is metadata-only on rows that already exist — it costs nothing and
        // must complete in one tick; the budget gates only genuinely NEW
        // inserts (the cold-trickle guard the budget was written for).
        let mut insert_budget = SPAWN_BUDGET_PER_TICK;
        for ses in plan.spawn.iter() {
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
                // Already seeded (an earlier build or a pre-takeover
                // generation may have created it without the stamp) — ADOPT:
                // stamp ownership and the session id so the next tick
                // recognizes it. Metadata-only, never budgeted.
                if let Some(session) = self.sessions.get_mut(&key) {
                    let needs_stamp = !session
                        .metadata
                        .iter()
                        .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA);
                    if needs_stamp {
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
                            crate::upsert_session_metadata(
                                &mut session.metadata,
                                "Cwd",
                                dir.clone(),
                            );
                        }
                    }
                }
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
            if insert_budget == 0 {
                continue;
            }
            insert_budget -= 1;
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
                let display = mirror_display_title(ses)
                    .or_else(|| directory_display_name(ses.directory.as_ref())
                        .map(|d| format!("{d} — new session")));
                if let Some(title) = display {
                    if !session.title_is_explicit {
                        session.title = title;
                    }
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
        // Title sync: the row name IS the tab name. The service title is
        // authoritative for mirror rows (the human renames tabs in the TUI,
        // not in the sidebar), so drift is corrected every tick — placeholders
        // excepted (a never-prompted session's `New session - <iso>` would
        // UN-name a row; its directory name holds the handle instead).
        for ses in active {
            let Some(title) = mirror_display_title(ses) else {
                continue;
            };
            let Some(tab) = owned.get(&ses.id) else {
                continue;
            };
            if let Some(session) = self.sessions.get_mut(&tab.key) {
                if !session.title_is_explicit && session.title != title {
                    session.title = title;
                }
            }
        }
        // Anchor-as-header: the opencode TUI row becomes its tab group's
        // header, titled by the tab the human is looking at (most recently
        // viewed) — the owner's contract, 2026-08-30. A hand-titled anchor is
        // respected and left alone.
        if let Some(anchor_key) = self.opencode_anchor_key() {
            let explicit = self
                .sessions
                .get(&anchor_key)
                .map(|a| a.title_is_explicit)
                .unwrap_or(true);
            if !explicit {
                if let Some(newest) = active
                    .iter()
                    .filter(|s| s.viewed_epoch_ms > 0)
                    .max_by_key(|s| s.viewed_epoch_ms)
                {
                    if let Some(title) = mirror_display_title(newest) {
                        if let Some(anchor) = self.sessions.get_mut(&anchor_key) {
                            anchor.title = title;
                        }
                    }
                }
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
    fn opencode_anchor_key(&self) -> Option<String> {
        self.sessions
            .values()
            .find(|s| {
                s.kind == crate::SessionKind::OpenCode
                    && s.session_path.starts_with("opencode-runtime://")
                    && !s
                        .metadata
                        .iter()
                        .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
            })
            .map(|a| a.session_path.clone())
    }

    fn next_opencode_tab_seat(&self) -> Option<String> {
        let anchor_key = self.opencode_anchor_key()?;
        let anchor = self.sessions.get(&anchor_key)?;
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

#[cfg(test)]
mod adoption_tests {
    use super::*;

    #[test]
    fn adoption_is_by_key_shape_and_never_touches_uuid_anchors() {
        // Stamped rows answer directly.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::OpenCode,
                "opencode-runtime://ses_abc000000000000000000001",
                Some("ses_abc000000000000000000001"),
            ),
            Some("ses_abc000000000000000000001".to_string())
        );
        // ⛔ THE ADOPTION CASE, measured 2026-08-29: rows created before the
        // stamp existed carry no metadata at all — the key IS the identity.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::OpenCode,
                "opencode-runtime://ses_abc000000000000000000001",
                None,
            ),
            Some("ses_abc000000000000000000001".to_string())
        );
        // uuid-keyed rows are anchors or phantoms — never mirror rows.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::OpenCode,
                "opencode-runtime://0d841111-1111-4111-8111-111111111111",
                None,
            ),
            None
        );
        // Other kinds are never mirror rows.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::ClaudeCode,
                "opencode-runtime://ses_abc000000000000000000001",
                None,
            ),
            None
        );
    }
}
