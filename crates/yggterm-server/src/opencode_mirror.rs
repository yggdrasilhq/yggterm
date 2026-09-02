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
/// The anchor's currently-rendered session — the tab the human is LOOKING at
/// in the TUI, refreshed every tick from the service's viewed-focus stream.
/// This is how the metadata pane speaks opencode's dynamicity language: a
/// uuid-keyed anchor row is not A session, it is A WINDOW onto whichever
/// session is focused, and this entry names it.
pub const VIEWING_SESSION_METADATA: &str = "Viewing Tab Session Id";
/// The one-time verdict of asking whether a legacy row's `--session` arg
/// names a session the CLI could actually resume. Stamped so the 1-second
/// mirror loop never re-opens the store under the daemon lock.
pub const LAUNCH_SESSION_PROBE_METADATA: &str = "Launch Session Probe";
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
            // The anchor's DYNAMICITY, surfaced as metadata: which session the
            // TUI is rendering RIGHT NOW. The title follow above answers "what
            // am I looking at" in the sidebar; this answers it in the metadata
            // pane, where a row uuid that is not a session id could never
            // (owner directive 2026-09-02: the metadata system should
            // understand the CLI's dynamicity language — opencode's is the
            // viewed-tab focus stream).
            let viewing = active
                .iter()
                .filter(|s| s.viewed_epoch_ms > 0)
                .max_by_key(|s| s.viewed_epoch_ms)
                .map(|s| s.id.clone());
            if let Some(session) = self.sessions.get_mut(&anchor_key) {
                if let Some(ses_id) = &viewing {
                    crate::upsert_session_metadata(
                        &mut session.metadata,
                        VIEWING_SESSION_METADATA,
                        ses_id.clone(),
                    );
                } else {
                    session
                        .metadata
                        .retain(|m| m.label != VIEWING_SESSION_METADATA);
                }
            }
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
        // LAST: reconcile legacy rows' launch identities (rebind a real
        // `--session` arg; demote a phantom one). Runs every tick but is
        // memoized per row by its verdict stamp, so the store probe under the
        // lock happens once per row ever.
        self.reconcile_opencode_row_identities(active);
    }

    /// The live opencode TUI row (the mirror's seating anchor) and the next
    /// free sub-seat under it: `<anchor outline>.<n+1>`.
    fn opencode_anchor_key(&self) -> Option<String> {
        // ⛔ THE ANCHOR IS THE LIVE TUI, NOT THE FIRST ROW THAT QUALIFIES.
        // The original predicate took the first OpenCode row without the
        // mirror stamp — with several uuid-keyed rows in the set (anchors of
        // TUIs that died, phantom resumes of ids no store ever held — both
        // measured live 2026-09-02: four uuid rows, one real TUI) the
        // anchor-as-header title landed on an arbitrary dead row while the
        // real TUI kept its stale name. Prefer a row that is actually
        // RUNNING; only a set with no live TUI at all falls back to the
        // historical first-qualified order.
        let candidates: Vec<&crate::ManagedSessionView> = self
            .sessions
            .values()
            .filter(|s| {
                s.kind == crate::SessionKind::OpenCode
                    && s.session_path.starts_with("opencode-runtime://")
                    && !s
                        .metadata
                        .iter()
                        .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
            })
            .collect();
        candidates
            .iter()
            .find(|s| {
                s.terminal_process_id.is_some()
                    || matches!(
                        s.launch_phase,
                        crate::TerminalLaunchPhase::Running
                            | crate::TerminalLaunchPhase::RemoteBootstrap
                    )
            })
            .copied()
            .or_else(|| candidates.first().copied())
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

    /// The `--session <arg>` a row's launch command names, if any.
    ///
    /// The wrapper quotes its tokens (`opencode2 '--auto' '--session' 'x'`), so
    /// the scan is token-wise with the quotes stripped — a `contains("--session")`
    /// substring test would find the flag and then mis-take the NEXT word.
    fn launch_session_arg(launch_command: &str) -> Option<String> {
        let mut tokens = launch_command.split_whitespace().map(|token| {
            token
                .trim_matches('\'')
                .trim_matches('"')
                .to_string()
        });
        while let Some(token) = tokens.next() {
            if token == "--session" {
                return tokens.next().filter(|value| !value.is_empty());
            }
        }
        None
    }

    /// Reconcile a legacy row's launch identity with the sessions the CLI's
    /// service actually knows — the opencode half of the identity-rebind
    /// family (the codex and Claude Code twins read /proc fds and transcripts;
    /// opencode's truth is the launch line validated against the service and
    /// the store).
    ///
    /// ⛔ THE DEFECT THIS CLOSES (measured live 2026-09-02, pending-bugs
    /// [11.28] correction): restore re-launches uuid-keyed rows with
    /// `--session <row-uuid>` — an id NO store holds — and opencode2 mints a
    /// fresh `ses_` id and carries on. Every restore of such a row points a
    /// TUI at a conversation that does not exist, and the row's Restore line
    /// offers the same phantom resume to the human. Two outcomes, by what the
    /// launch arg provably names:
    ///
    /// - **Real session** (in the service's active set or the store): the row
    ///   is REBOUND to it — `apply_agent_runtime_session_id_to_live_session`
    ///   repoints id, launch and Launch metadata, and the kind's session-id
    ///   metadata is stamped so the pane stops showing the row seat.
    /// - **Phantom** (neither knows the id): the row is a WINDOW, not a
    ///   session — yggterm cannot know which conversation it renders (the
    ///   service exposes no window→session map, and the Observer Rule forbids
    ///   guessing), so its resume-of-a-phantom is DEMOTED to the bare TUI
    ///   start. The conversation is reachable from the TUI's own session
    ///   list; what stops happening is minting a fresh empty session on every
    ///   restore.
    ///
    /// The store probe runs ONCE per row: the verdict is stamped in metadata
    /// (`Launch Session Probe`), so the 1-second loop never re-opens sqlite
    /// under the daemon lock.
    pub(crate) fn reconcile_opencode_row_identities(
        &mut self,
        active: &[OpencodeServiceSession],
    ) -> usize {
        let Some(home) = crate::resolve_yggterm_home().ok() else {
            return 0;
        };
        let store_home = yggterm_core::startpage::agent_store_home(&home);
        let verdicts = self.reconcile_opencode_row_identities_in(&store_home, active);
        // The per-row verdicts are the observable; the emission lives in the
        // PRODUCTION wrapper only — the `*_in` test seam must never write the
        // fleet's trace files (measured 2026-09-02: test-fixture verdicts
        // polluted production ytrace).
        for (session_path, arg, verdict) in &verdicts {
            yggterm_core::append_trace_event(
                &home,
                "daemon",
                "opencode_mirror",
                "launch_session_reconciled",
                serde_json::json!({
                    "session_path": session_path,
                    "arg": arg,
                    "verdict": verdict,
                }),
            );
        }
        verdicts.len()
    }

    /// [`Self::reconcile_opencode_row_identities`] against an explicit store
    /// home — the test seam, so a test never reads the machine's real CLI
    /// store (a unit test that consults the user's own store passes or fails
    /// on THEIR data).
    pub(crate) fn reconcile_opencode_row_identities_in(
        &mut self,
        store_home: &std::path::Path,
        active: &[OpencodeServiceSession],
    ) -> Vec<(String, String, String)> {
        let active_ids: std::collections::HashSet<&str> = active
            .iter()
            .map(|session| session.id.as_str())
            .collect();
        // Collect first, mutate after: the rebind path rewrites row state and
        // the anchor phases above already ran on a stable map.
        let candidates: Vec<(String, String)> = self
            .sessions
            .iter()
            .filter(|(key, session)| {
                session.kind == crate::SessionKind::OpenCode
                    && key.starts_with("opencode-runtime://")
                    && !session
                        .metadata
                        .iter()
                        .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
                    && !session
                        .metadata
                        .iter()
                        .any(|m| m.label == LAUNCH_SESSION_PROBE_METADATA)
            })
            .filter_map(|(key, session)| {
                Self::launch_session_arg(&session.launch_command).map(|arg| (key.clone(), arg))
            })
            .collect();
        let mut verdicts: Vec<(String, String, String)> = Vec::new();
        for (key, arg) in candidates {
            let key_id = key.trim_start_matches("opencode-runtime://").to_string();
            let known = active_ids.contains(arg.as_str())
                || yggterm_core::agent_cli::opencode_store_index_holds_session(
                    store_home,
                    &arg,
                )
                .unwrap_or(false);
            let mut verdict = "probe failed".to_string();
            if known {
                if arg == key_id {
                    verdict = "self".to_string();
                } else {
                    // REBIND: the launch names a session this row is not keyed
                    // to — the CLI's id outranks the birth seat (the codex
                    // rebind's law). Repoints id, launch and Launch metadata.
                    if self.apply_agent_runtime_session_id_to_live_session(&key, &arg) {
                        let resolved = self
                            .resolve_session_storage_key(&key)
                            .map(str::to_string);
                        if let Some(resolved) = resolved {
                            if let Some(session) = self.sessions.get_mut(&resolved) {
                                crate::upsert_session_metadata(
                                    &mut session.metadata,
                                    "OpenCode Session",
                                    arg.clone(),
                                );
                            }
                        }
                        verdict = format!("rebound to {arg}");
                    } else {
                        verdict = "rebind refused".to_string();
                    }
                }
            } else {
                // DEMOTE: the phantom resume must not survive into the next
                // restore. Rebuild the launch as the bare TUI start (the same
                // wrapper, no `--session`), and repoint the Restore line the
                // pane offers to the bare-start form.
                let mut demoted = false;
                if let Some(key) = self.resolve_session_storage_key(&key).map(str::to_string) {
                    if let Some(session) = self.sessions.get_mut(&key) {
                        let cwd = session
                            .metadata
                            .iter()
                            .find(|m| m.label == "Cwd")
                            .map(|m| m.value.clone())
                            .or_else(|| {
                                session
                                    .metadata
                                    .iter()
                                    .find(|m| m.label == "Target")
                                    .map(|m| m.value.clone())
                            })
                            .filter(|value| !value.trim().is_empty());
                        if let Ok(bare) = crate::managed_cli::managed_cli_shell_command_with_terminal_appearance(
                            crate::SessionKind::OpenCode,
                            cwd.as_deref(),
                            crate::managed_cli::ManagedCliAction::Launch,
                            None,
                        ) {
                            let key_id = key
                                .trim_start_matches("opencode-runtime://")
                                .to_string();
                            session.launch_command = bare.clone();
                            crate::upsert_session_metadata(
                                &mut session.metadata,
                                "Launch",
                                crate::user_visible_launch_command(&bare),
                            );
                            if let Some(start_verb) =
                                crate::remote_agent_start_subcommand(crate::SessionKind::OpenCode)
                            {
                                crate::upsert_session_metadata(
                                    &mut session.metadata,
                                    "Restore",
                                    format!("yggterm server remote {start_verb} {key_id}"),
                                );
                            }
                            verdict = format!("phantom demoted (arg {arg})");
                            demoted = true;
                        }
                    }
                }
                if !demoted {
                    // keep the "probe failed" default — the row is re-probed
                    // next tick (no verdict stamp distinguishes a failed probe
                    // from an unprobed row, deliberately: a failed probe must
                    // be RETRIED, not remembered).
                }
            }
            if let Some(key) = self.resolve_session_storage_key(&key).map(str::to_string) {
                if let Some(session) = self.sessions.get_mut(&key) {
                    crate::upsert_session_metadata(
                        &mut session.metadata,
                        LAUNCH_SESSION_PROBE_METADATA,
                        verdict.clone(),
                    );
                }
            }
            verdicts.push((key, arg, verdict));
        }
        verdicts
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

mod anchor_tests {
    use super::*;

    fn server_with_two_anchors() -> (crate::YggtermServer, String, String) {
        let mut server = crate::YggtermServer::new(
            false,
            crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
            yggui_contract::UiTheme::ZedLight,
        );
        // The historical order bug: the DEAD anchor sorts first in the row
        // map, so the old first-qualified pick landed on it while the live
        // TUI sat untitled. Measured live 2026-09-02: four uuid rows, one
        // real TUI, and the anchor-as-header title on an arbitrary one.
        let dead = server.start_local_session(
            crate::SessionKind::OpenCode,
            Some("/home/user/proj"),
            Some("Remote OpenCode d4090efe"),
        );
        let live = server.start_local_session(
            crate::SessionKind::OpenCode,
            Some("/home/user/proj"),
            Some("Remote OpenCode 7e7d6c5e"),
        );
        let live_row = server
            .sessions
            .get_mut(&live)
            .expect("the live row exists");
        live_row.launch_phase = crate::TerminalLaunchPhase::Running;
        live_row.terminal_process_id = Some(4242);
        // The dead row must be dead ON PURPOSE: `start_local_session` may
        // birth rows in a running-looking phase depending on the host, and
        // the selection under test keys on the running marks.
        let dead_row = server
            .sessions
            .get_mut(&dead)
            .expect("the dead row exists");
        dead_row.launch_phase = crate::TerminalLaunchPhase::Queued;
        dead_row.terminal_process_id = None;
        // The rows this test mirrors are the runtime-spelled ones the real
        // plane serves (`opencode-runtime://<uuid>`); `start_local_session`
        // births rows under the `local://` seat key, so re-key both rows the
        // way the daemon's alias layer does before the mirror runs. The rows
        // are identified by their MARK (the live TUI carries a pid), never by
        // their title — a fallback-shaped title hint is filtered at birth and
        // proves nothing.
        for key in [&dead, &live] {
            if let Some(mut row) = server.sessions.remove(key) {
                let new_key = format!("opencode-runtime://{}", row.id);
                row.session_path = new_key.clone();
                server.sessions.insert(new_key, row);
            }
        }
        let dead_key = server
            .sessions
            .values()
            .find(|r| {
                r.kind == crate::SessionKind::OpenCode && r.terminal_process_id.is_none()
            })
            .map(|r| r.session_path.clone())
            .expect("dead fixture row");
        let live_key = server
            .sessions
            .values()
            .find(|r| r.terminal_process_id == Some(4242))
            .map(|r| r.session_path.clone())
            .expect("live fixture row");
        (server, dead_key, live_key)
    }

    #[test]
    fn the_anchor_is_the_live_tui_not_the_first_qualified_row() {
        let (server, _dead, live) = server_with_two_anchors();
        assert_eq!(
            server.opencode_anchor_key(),
            Some(live),
            "a RUNNING TUI outranks a dead uuid row in anchor selection",
        );
    }

    #[test]
    fn the_anchor_names_the_session_it_is_currently_viewing() {
        let (mut server, _dead, _live) = server_with_two_anchors();
        let viewed = |id: &str, viewed: u128| OpencodeServiceSession {
            id: id.to_string(),
            title: Some(format!("Tab {id}")),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: viewed,
            running: true,
        };
        // Two open tabs; ses_b was looked at LAST, so it is what the TUI
        // renders right now.
        let active = vec![
            viewed("ses_a0000000000000000000000001", 100),
            viewed("ses_b0000000000000000000000002", 200),
        ];
        server.apply_opencode_tab_mirror(&active);
        let anchor_key = server
            .opencode_anchor_key()
            .expect("an anchor exists");
        let anchor = server.sessions.get(&anchor_key).expect("anchor row");
        let viewing = anchor
            .metadata
            .iter()
            .find(|m| m.label == VIEWING_SESSION_METADATA)
            .map(|m| m.value.clone());
        assert_eq!(
            viewing.as_deref(),
            Some("ses_b0000000000000000000000002"),
            "the anchor's metadata pane entry must name the session the human \
             is LOOKING at — the CLI's dynamicity language, surfaced",
        );
        // And the header title follows the same focus (anchor-as-header).
        assert!(
            anchor.title.contains("ses_b")
                || anchor.title == "Tab ses_b0000000000000000000000002",
            "the anchor title follows the viewed tab, got {:?}",
            anchor.title
        );
    }

    #[test]
    fn a_quiet_service_clears_the_viewing_entry_instead_of_freezing_it() {
        let (mut server, _dead, _live) = server_with_two_anchors();
        let viewed = |id: &str, viewed: u128| OpencodeServiceSession {
            id: id.to_string(),
            title: Some(format!("Tab {id}")),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: viewed,
            running: true,
        };
        server.apply_opencode_tab_mirror(&vec![viewed(
            "ses_a0000000000000000000000001",
            100,
        )]);
        let with_tabs = {
            let anchor_key = server.opencode_anchor_key().expect("anchor");
            server
                .sessions
                .get(&anchor_key)
                .expect("anchor row")
                .metadata
                .iter()
                .any(|m| m.label == VIEWING_SESSION_METADATA)
        };
        assert!(with_tabs, "a viewed tab stamps the viewing entry");
        // The service's active set goes quiet (no tabs anywhere): a stale
        // "Viewing …" would now be a lie about the present.
        server.apply_opencode_tab_mirror(&[]);
        let anchor_key = server.opencode_anchor_key().expect("anchor");
        let anchor = server.sessions.get(&anchor_key).expect("anchor row");
        assert!(
            !anchor
                .metadata
                .iter()
                .any(|m| m.label == VIEWING_SESSION_METADATA),
            "no viewed tab anywhere must not leave a frozen 'Viewing' claim",
        );
    }

    /// The launch line's `--session` argument, read token-wise with the
    /// wrapper's quoting stripped. A substring test would find the flag and
    /// mis-take the next word; this is the parser the reconcile trusts.
    #[test]
    fn the_launch_session_arg_parser_survives_the_wrapper_quotes() {
        let wrapper = "opencode2 '--auto' '--session' 'd4090efe-4e12-42d9-938d-66f61801d2e7'";
        assert_eq!(
            super::YggtermServer::launch_session_arg(wrapper).as_deref(),
            Some("d4090efe-4e12-42d9-938d-66f61801d2e7")
        );
        assert_eq!(
            super::YggtermServer::launch_session_arg("opencode2 --auto").as_deref(),
            None,
            "a bare TUI start names no session — nothing to reconcile"
        );
    }

    fn phantom_arg_row() -> crate::YggtermServer {
        let mut server = crate::YggtermServer::new(
            false,
            crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
            yggui_contract::UiTheme::ZedLight,
        );
        let key = server.start_local_session(
            crate::SessionKind::OpenCode,
            Some("/home/user/proj"),
            None,
        );
        if let Some(row) = server.sessions.get_mut(&key) {
            row.session_path = format!("opencode-runtime://{}", row.id);
            row.launch_command = format!(
                "opencode2 '--auto' '--session' '{}'",
                row.id
            );
            row.metadata.push(crate::SessionMetadataEntry {
                label: "Cwd",
                value: "/home/user/proj".to_string(),
            });
        }
        server
    }

    fn rekey_runtime(server: &mut crate::YggtermServer, old_key: &str) -> String {
        let mut row = server.sessions.remove(old_key).expect("fixture row");
        let new_key = format!("opencode-runtime://{}", row.id);
        row.session_path = new_key.clone();
        server.sessions.insert(new_key.clone(), row);
        new_key
    }

    fn any_row_key(server: &crate::YggtermServer) -> String {
        server
            .sessions
            .keys()
            .next()
            .cloned()
            .expect("fixture row key")
    }

    /// ⛔ THE PHANTOM RESUME MUST NOT SURVIVE INTO THE NEXT RESTORE
    /// (pending-bugs [11.28] correction, 2026-09-02). A uuid-keyed row whose
    /// launch names `--session <uuid>` — an id the service AND the store both
    /// deny — is a WINDOW, not a session: yggterm cannot know which
    /// conversation it renders. The reconcile DEMOTES the resume-of-a-phantom
    /// to the bare TUI start, so the next restore stops minting a fresh empty
    /// session, and the pane's Restore line offers what would actually happen.
    #[test]
    fn a_phantom_session_arg_is_demoted_to_the_bare_start() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-oc-reconcile-{}",
            uuid::Uuid::new_v4()
        ));
        let mut server = phantom_arg_row();
        let first = any_row_key(&server);
        let key = rekey_runtime(&mut server, &first);
        let reconciled = server
            .reconcile_opencode_row_identities_in(&home, &[]);
        assert_eq!(reconciled.len(), 1, "the phantom-arg row is reconciled");
        let row = server.sessions.get(&key).expect("row stays at its key");
        assert!(
            !row.launch_command.contains("--session"),
            "the demoted launch must not name a session: {:?}",
            row.launch_command
        );
        assert!(
            row.launch_command.contains("--auto"),
            "the demoted launch is the bare TUI start: {:?}",
            row.launch_command
        );
        let restore = row
            .metadata
            .iter()
            .find(|m| m.label == "Restore")
            .map(|m| m.value.clone())
            .unwrap_or_default();
        assert!(
            restore.contains("start-opencode") && !restore.contains("resume"),
            "the Restore line offers the bare start, not the phantom resume: {restore:?}"
        );
        let probe = row
            .metadata
            .iter()
            .find(|m| m.label == LAUNCH_SESSION_PROBE_METADATA)
            .map(|m| m.value.clone())
            .unwrap_or_default();
        assert!(
            probe.contains("phantom"),
            "the verdict is stamped so the store probe never re-runs: {probe:?}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A real `--session` arg (the service knows it) REBINDS the row: the
    /// CLI's id outranks the birth seat, the kind's session-id metadata is
    /// stamped, and the verdict says so — the one-conversation-two-rows
    /// divergence closes at the identity plane.
    #[test]
    fn a_real_session_arg_rebinds_the_legacy_row() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-oc-reconcile-{}",
            uuid::Uuid::new_v4()
        ));
        let mut server = phantom_arg_row();
        let first = any_row_key(&server);
        let key = rekey_runtime(&mut server, &first);
        let real = "ses_real00000000000000000000001";
        if let Some(row) = server.sessions.get_mut(&key) {
            row.launch_command = format!("opencode2 '--auto' '--session' '{real}'");
        }
        let active = vec![OpencodeServiceSession {
            id: real.to_string(),
            title: Some("the real session".to_string()),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: 1,
            running: true,
        }];
        let reconciled = server
            .reconcile_opencode_row_identities_in(&home, &active);
        assert_eq!(reconciled.len(), 1, "the real-arg row is reconciled");
        let row = server.sessions.get(&key).expect("row stays at its key");
        assert_eq!(
            row.id, real,
            "the row's identity is the session the CLI actually runs"
        );
        let store_label = row
            .metadata
            .iter()
            .find(|m| m.label == "OpenCode Session")
            .map(|m| m.value.clone());
        assert_eq!(
            store_label.as_deref(),
            Some(real),
            "the kind's session-id metadata carries the REAL id — the pane \
             stops showing the row seat"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The verdict stamp is the memoization: a reconciled row is never
    /// re-probed (the store probe under the daemon lock happens once per row
    /// ever), and a consistent row (`--session <its own real id>`) stamps
    /// "self" and is left alone.
    #[test]
    fn a_reconciled_row_is_never_reprobed() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-oc-reconcile-{}",
            uuid::Uuid::new_v4()
        ));
        let mut server = phantom_arg_row();
        let first = any_row_key(&server);
        let key = rekey_runtime(&mut server, &first);
        assert_eq!(
            server
                .reconcile_opencode_row_identities_in(&home, &[])
                .len(),
            1
        );
        let after_first = server
            .sessions
            .get(&key)
            .expect("row")
            .launch_command
            .clone();
        assert_eq!(
            server
                .reconcile_opencode_row_identities_in(&home, &[])
                .len(),
            0,
            "a stamped row is not a candidate — the probe is memoized"
        );
        assert_eq!(
            server.sessions.get(&key).expect("row").launch_command,
            after_first,
            "the second tick changed nothing"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
