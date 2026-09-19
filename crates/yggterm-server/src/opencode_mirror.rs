//! The OpenCode tab mirror — one yggterm row per session the service's store
//! lists.
//!
//! opencode2 is client-server: the row's PTY hosts a TUI (a window), the
//! background service owns the sessions, and one window renders N tabs
//! (docs/cli-integration.md, Issue Heading 26). The mirror keeps yggterm's
//! one-row-per-session invariant: every session in the service's store list
//! gets a real row, keyed by its service id, seated under the opencode anchor
//! row, with the launch line `opencode2 --session <ses_id>`.
//!
//! ⛔ MEASURED (2026-09-10 decode, beta-19271): the server has NO surface
//! that says which tabs are open — the active set is the WORKING set
//! (in-flight executions, empty-when-idle BY DESIGN), and the in-TUI switch
//! writes nothing server-side. Mirroring the working set ([11.6.3-a]) made
//! rows blink into existence with a turn and retire when it settled. So the
//! mirror's UNIVERSE is the store list (turn recency orders it), the working
//! set is a per-session status on it, and which session a window RENDERS is
//! answered where the truth lives: the OSC window title (`OC | <title>`).
//! ⭐ MEASURED 2026-09-17 (2.0.3): there is now ALSO a client-side surface —
//! `.local/state/opencode/latest/tui/tabs.json`, cwd-keyed tabs with their
//! `sessionID` ([`yggterm_core::opencode_service::tui_tabs`]) — which names a
//! window's bound session directly and paints far more reliably than the OSC
//! route title (three drives captured only the generic `OpenCode` there). A
//! bind/divergence verdict for resumed rows should read THIS file; the OSC
//! title stays the in-TUI-switch follow signal.
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
//! retired for leaving the working set: it has become a window, and windows
//! close when the user closes them.

use crate::YggtermServer;
use yggterm_core::opencode_service::OpencodeServiceSession;

pub const TAB_SOURCE_METADATA: &str = "opencode-tab-mirror";
pub const MIRROR_INTERVAL_MS: u64 = 5_000;
pub const TAB_SESSION_ID_METADATA: &str = "Tab Session Id";
/// The anchor's currently-rendered session — the tab the human is LOOKING at
/// in the TUI. This is how the metadata pane speaks opencode's dynamicity
/// language: a uuid-keyed anchor row is not A session, it is A WINDOW onto
/// whichever session is focused, and this entry names it. The truth source
/// is the anchor's own OSC window title first (the surface the TUI actually
/// writes per session route), the service's viewed-focus stream second —
/// API-writer-only on beta-19271, so dormant there but kept for builds with
/// a writer ([11.6.3-a]).
pub const VIEWING_SESSION_METADATA: &str = "Viewing Tab Session Id";
pub const SPAWN_BUDGET_PER_TICK: usize = 1;
const VIEWED_METADATA: &str = "Tab Viewed Ms";

/// What a sync tick should do, decided as a pure function so the diff is
/// testable without a service, a daemon, or rows.
#[derive(Debug, Default, PartialEq)]
pub struct TabSyncPlan {
    /// Listed sessions with no mirror row yet.
    pub spawn: Vec<OpencodeServiceSession>,
    /// Session ids no longer in the store list AND whose row was never
    /// engaged. Mirror projections only — a wrapper/anchor row is the user's
    /// row and is never retired here ([11.148]).
    pub retire: Vec<String>,
    /// [11.148] Mirror twins whose session a live non-mirror row already
    /// pins — born in the seconds before the pin landed (or before the id
    /// arm existed) and never retracted. Retraction removes the projection
    /// row and drops NO tombstone: when the pinning row later dies, the
    /// session legitimately re-projects (the keep-alive law).
    pub retract: Vec<String>,
    /// The tab row key the human just focused — viewed recency moved to a
    /// session this mirror already mirrors. (Dormant on beta-19271: no TUI
    /// flow writes the viewed times; kept for builds with a writer.)
    /// Mirror projections only: a pinned wrapper row IS the thing being
    /// looked at — nothing to move.
    pub focus: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct OwnedTab {
    pub(crate) key: String,
    pub(crate) viewed_epoch_ms: u128,
    pub(crate) engaged: bool,
    /// The row owning this tab is the mirror's OWN projection (key-shaped
    /// `opencode-runtime://` or `Source`-stamped). A wrapper/anchor row that
    /// pinned the session in its own `id` owns it WITHOUT being a projection:
    /// the mirror never retires, retitles or focus-claims it, and its pin is
    /// what retracts an unengaged twin ([11.148]).
    pub(crate) mirror_owned: bool,
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
/// are never adopted BY KEY — a rebound anchor still owns through its id,
/// as a non-projection owner).
///
/// ⭐ [11.148] THE ROW PLANE'S OWN ID IS THE THIRD ARM: a WRAPPER row (keyed
/// `local://…`) discovers its service session POST-HOC and the pin lands in
/// `session.id` (authoritative, service-vouched) — it writes no Tab Session
/// Id metadata, so the two-arm read called its session un-owned and the sync
/// spawned a keep-alive twin for a session a live row already renders. One
/// book, not two: the id IS the ownership record (and it survives a daemon
/// restart, which the metadata historically does not).
fn mirror_tab_session_id(
    kind: crate::SessionKind,
    key: &str,
    stamped: Option<&str>,
    id: &str,
) -> Option<String> {
    if kind != crate::SessionKind::OpenCode {
        return None;
    }
    if let Some(ses) = stamped.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(ses.to_string());
    }
    if let Some(rest) = key.strip_prefix("opencode-runtime://") {
        if rest.starts_with("ses_") {
            return Some(rest.to_string());
        }
    }
    let id = id.trim();
    id.starts_with("ses_").then(|| id.to_string())
}

/// The liveness marks that make a row a WINDOW (a real PTY or a live launch
/// phase). Shared by the owned read and the [11.148] retraction filter —
/// one predicate, or the two readers can disagree about what "opened" means.
fn row_is_engaged(session: &crate::ManagedSessionView) -> bool {
    session.terminal_process_id.is_some()
        || matches!(
            session.launch_phase,
            crate::TerminalLaunchPhase::Running
                | crate::TerminalLaunchPhase::RemoteBootstrap
                | crate::TerminalLaunchPhase::BridgePending
        )
}

/// The row at a twin's spawn key is the mirror's projection — Source-stamped.
/// The key shape alone is near-proof (only the mirror creates those rows),
/// but retraction REMOVES a row, so removals confirm the stamp first.
fn mirror_projection_row(session: &crate::ManagedSessionView) -> bool {
    session
        .metadata
        .iter()
        .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
}

fn owned_tabs_from(
    sessions: &std::collections::BTreeMap<String, crate::ManagedSessionView>,
) -> std::collections::HashMap<String, OwnedTab> {
    let mut out: std::collections::HashMap<String, OwnedTab> = std::collections::HashMap::new();
    for (key, session) in sessions {
        let Some(ses) = mirror_tab_session_id(
            session.kind,
            key,
            session
                .metadata
                .iter()
                .find(|m| m.label == TAB_SESSION_ID_METADATA)
                .map(|m| m.value.as_str()),
            &session.id,
        ) else {
            continue;
        };
        // [11.148] mirror_owned is the PROJECTION mark: the mirror's own key
        // shape or its Source stamp. The id arm's owners (a wrapper row, a
        // rebound anchor) are the user's real rows — never mirror bookkeeping
        // targets. (The old computed-but-unused `is_mirror` counted a ses_
        // id as a projection mark, which would have armed retire against a
        // wrapper row the moment the id arm existed.)
        let mirror_owned = key.starts_with("opencode-runtime://")
            || session
                .metadata
                .iter()
                .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA);
        let engaged = row_is_engaged(session);
        let viewed = session
            .metadata
            .iter()
            .find(|m| m.label == VIEWED_METADATA)
            .and_then(|m| m.value.parse::<u128>().ok())
            .unwrap_or(0);
        let tab = OwnedTab {
            key: key.clone(),
            viewed_epoch_ms: viewed,
            engaged,
            mirror_owned,
        };
        // Two rows can claim one session mid-flight (the twin born in the
        // seconds before the wrapper's pin landed). The AUTHORITATIVE row
        // wins the slot: the plan must see the wrapper's ownership, not the
        // projection's.
        use std::collections::hash_map::Entry;
        match out.entry(ses) {
            Entry::Occupied(mut e) if e.get().mirror_owned && !mirror_owned => {
                e.insert(tab);
            }
            Entry::Vacant(e) => {
                e.insert(tab);
            }
            _ => {}
        }
    }
    out
}

/// The ses ids pinned by STABLE live non-mirror rows — the set that may
/// retract a twin ([11.148]). The ANCHOR's pin is excluded: its bound id
/// follows the tab the human is viewing, so retracting on it would flap a
/// projection (retract, re-project) on every tab switch.
pub(crate) fn pinned_stable_from(
    owned: &std::collections::HashMap<String, OwnedTab>,
    anchor_key: Option<&str>,
) -> std::collections::HashSet<String> {
    owned
        .iter()
        .filter(|(_, tab)| {
            !tab.mirror_owned
                && tab.engaged
                && anchor_key != Some(tab.key.as_str())
        })
        .map(|(ses, _)| ses.clone())
        .collect()
}

pub fn plan_tab_sync(
    sessions: &[OpencodeServiceSession],
    owned: &std::collections::HashMap<String, OwnedTab>,
    pinned_stable: &std::collections::HashSet<String>,
    rows: &std::collections::BTreeMap<String, crate::ManagedSessionView>,
) -> TabSyncPlan {
    let mut spawn = Vec::new();
    for ses in sessions {
        if !owned.contains_key(&ses.id) {
            spawn.push(ses.clone());
        }
    }
    let mut retire = Vec::new();
    for (ses, tab) in owned {
        if !tab.mirror_owned {
            // [11.148] a wrapper/anchor row is the user's row — the mirror
            // never retires it for losing its tab.
            continue;
        }
        if !sessions.iter().any(|s| &s.id == ses) && !tab.engaged {
            retire.push(ses.clone());
        }
    }
    // [11.148] RETRACTION: a twin whose session a live non-mirror row pins.
    // The owned slot for that ses belongs to the AUTHORITATIVE row (it wins
    // the collision), so the twin is found at its deterministic spawn key,
    // confirmed a Source-stamped projection, and skipped if the user OPENED
    // it (an engaged twin is a real second window, not a duplicate rail row).
    let mut retract: Vec<String> = pinned_stable
        .iter()
        .filter(|ses| {
            rows.get(&format!("opencode-runtime://{ses}"))
                .is_some_and(|twin| mirror_projection_row(twin) && !row_is_engaged(twin))
        })
        .cloned()
        .collect();
    retract.sort();
    // Focus-follow: the human's focused tab is the most recently VIEWED one,
    // and following is due only when that view moved PAST what this mirror
    // has already recorded for the row — a no-op on quiet ticks. Projections
    // only ([11.148]): a pinned wrapper row IS the thing being looked at.
    let mut focus = None;
    if let Some(newest) = sessions
        .iter()
        .filter(|s| s.viewed_epoch_ms > 0)
        .max_by_key(|s| s.viewed_epoch_ms)
    {
        if let Some(tab) = owned.get(&newest.id) {
            if tab.mirror_owned && newest.viewed_epoch_ms > tab.viewed_epoch_ms {
                focus = Some(newest.id.clone());
            }
        }
    }
    TabSyncPlan {
        spawn,
        retire,
        retract,
        focus,
    }
}

/// The bind verdict the TUI's own client-side tab maps answer for a pinned
/// session id under a row's cwd — the [11.134] closer's raw material
/// ([`yggterm_core::opencode_service::tui_tabs_across_instances`]).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TuiBindVerdict {
    /// Some instance dir names the pinned id under the cwd.
    Verified,
    /// An instance dir answers for the cwd with OTHER ids only — the window
    /// is rendering a different session than the row pins. Positive evidence
    /// only: a cwd nothing names is never divergence (a beta-era or
    /// other-shaped tabs.json must not cry wolf).
    Diverged(Vec<String>),
    /// No instance answers for this cwd yet; the entry lands within the
    /// launch's first seconds, so the check stays open for a later tick.
    Unknown,
}

pub(crate) fn tui_bind_verdict(
    instances: &[(String, std::collections::BTreeMap<String, Vec<String>>)],
    cwd: &str,
    pinned: &str,
) -> TuiBindVerdict {
    let mut others: Vec<String> = Vec::new();
    for (_, map) in instances {
        let Some(ids) = map.get(cwd) else {
            continue;
        };
        if ids.iter().any(|id| id == pinned) {
            return TuiBindVerdict::Verified;
        }
        for id in ids {
            if !others.contains(id) {
                others.push(id.clone());
            }
        }
    }
    if others.is_empty() {
        TuiBindVerdict::Unknown
    } else {
        TuiBindVerdict::Diverged(others)
    }
}

/// The once-per-daemon-generation record of bind verdicts already delivered.
/// Per-process = per-generation: a restored row re-verifies after a daemon
/// restart, per [11.134]'s design.
fn tui_bind_checks_done() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static DONE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    DONE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

fn tui_bind_check_key(row_key: &str, ses: &str) -> String {
    format!("{row_key}|{ses}")
}

#[cfg(test)]
pub(crate) fn tui_bind_check_was_delivered(row_key: &str, ses: &str) -> bool {
    tui_bind_checks_done()
        .lock()
        .map(|done| done.contains(&tui_bind_check_key(row_key, ses)))
        .unwrap_or(false)
}

/// The bind checks a tick owes: live local opencode rows carrying a pinned
/// ses id — not yet verdict-checked this generation — whose cwd the service
/// list can name. Non-local rows are skipped (`SessionSource::LiveLocal` is
/// the law — local births still carry `ssh_target: Some("localhost")`, so
/// the ssh field is not the discriminator): a remote row's tabs.json lives
/// on the remote home and this daemon cannot read it, so it must not invent
/// a verdict.
pub(crate) fn plan_tui_bind_checks(
    owned: &std::collections::HashMap<String, OwnedTab>,
    sessions: &std::collections::BTreeMap<String, crate::ManagedSessionView>,
    service_sessions: &[OpencodeServiceSession],
    screen_live: &std::collections::HashSet<String>,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (ses, tab) in owned {
        if tui_bind_checks_done()
            .lock()
            .map(|done| done.contains(&tui_bind_check_key(&tab.key, ses)))
            .unwrap_or(false)
        {
            continue;
        }
        let Some(row) = sessions.get(&tab.key) else {
            continue;
        };
        if row.source != crate::SessionSource::LiveLocal {
            continue;
        }
        if !YggtermServer::anchor_row_is_live(row, screen_live) {
            continue;
        }
        let Some(directory) = service_sessions
            .iter()
            .find(|s| &s.id == ses)
            .and_then(|s| s.directory.as_ref())
            .map(String::as_str)
            .filter(|d| !d.trim().is_empty())
        else {
            continue;
        };
        out.push((tab.key.clone(), ses.clone(), directory.to_string()));
    }
    out
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

/// opencode2's own window-title prefix for a session route (its app.tsx:
/// `renderer.setTerminalTitle(`OC | ${title}`)`).
const OPENCODE_WINDOW_TITLE_PREFIX: &str = "OC | ";

/// opencode2 truncates the title it puts in the window: over 40 chars
/// becomes the first 37 + "..." (same file). Compare in the SAME window.
fn opencode_title_window(title: &str) -> String {
    if title.chars().count() > 40 {
        let head: String = title.chars().take(37).collect();
        format!("{head}...")
    } else {
        title.to_string()
    }
}

/// The ONE sessions service session whose window-title form equals `name`;
/// `None` when none matches or when two match (ambiguous is not identity —
/// a fork view is fine to share, a shared NAME is not).
fn unique_session_by_title<'a>(
    sessions: &'a [OpencodeServiceSession],
    name: &str,
) -> Option<&'a OpencodeServiceSession> {
    let mut found: Option<&OpencodeServiceSession> = None;
    for session in sessions {
        if opencode_title_window(session.title.as_deref().unwrap_or("")) == name {
            if found.is_some() {
                return None;
            }
            found = Some(session);
        }
    }
    found
}

fn directory_display_name(directory: Option<&String>) -> Option<String> {
    let dir = directory?;
    let name = dir.rsplit('/').find(|seg| !seg.is_empty())?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The spawn door of the tombstone plane for the opencode mirror. The
/// service is a PEER that keeps a user-closed session alive in its tab list,
/// and the mirror's universe is that tab list — so without this ask the
/// mirror re-spawned the row on every tick after the user closed it
/// (measured on the GUI host 2026-09-16: owner deletes, next tick
/// plan_spawn: 6, owned back to 6 within minutes; the [11.135] ghost
/// factory's third door). The same read-only door the import, restore,
/// recovery and adoption doors ask — one load for the whole batch. The
/// deliberate re-open stays permissive: the open verb clears the tombstone.
pub(crate) fn tombstoned_opencode_mirror_spawns(
    home: &std::path::Path,
    sessions: &[OpencodeServiceSession],
) -> Vec<String> {
    let keys: Vec<String> = sessions
        .iter()
        .map(|ses| format!("opencode-runtime://{}", ses.id))
        .collect();
    crate::live_row_closes_remembered_among(home, keys.iter().map(String::as_str))
}

impl YggtermServer {
    /// Apply one mirror tick UNDER THE DAEMON LOCK. All service IO happened
    /// before this call (`fetch` in the chore, which holds no lock).
    /// The [11.134] closer: once per daemon generation, ask every live pinned
    /// opencode row's TUI tab maps which session its window REALLY binds, and
    /// name the verdict on the trace plane. A verdict plane, not an actor —
    /// the anchor `diverged` posture: no rebind on this build. Unknown keeps
    /// the check open (a later tick retries); Verified/Diverged speak once
    /// per row per generation.
    pub(crate) fn run_tui_bind_checks(
        &self,
        home: &std::path::PathBuf,
        owned: &std::collections::HashMap<String, OwnedTab>,
        service_sessions: &[OpencodeServiceSession],
        screen_live: &std::collections::HashSet<String>,
    ) {
        for (row_key, ses, cwd) in
            plan_tui_bind_checks(owned, &self.sessions, service_sessions, screen_live)
        {
            let instances = yggterm_core::opencode_service::tui_tabs_across_instances(home);
            match tui_bind_verdict(&instances, &cwd, &ses) {
                TuiBindVerdict::Unknown => {}
                TuiBindVerdict::Verified => {
                    if let Ok(mut done) = tui_bind_checks_done().lock() {
                        done.insert(tui_bind_check_key(&row_key, &ses));
                    }
                    #[cfg(not(test))]
                    yggterm_core::append_trace_event(
                        home,
                        "daemon",
                        "opencode_mirror",
                        "opencode_bind_verified",
                        serde_json::json!({
                            "row": row_key,
                            "pinned": ses,
                            "cwd": cwd,
                        }),
                    );
                }
                TuiBindVerdict::Diverged(_tui_ids) => {
                    if let Ok(mut done) = tui_bind_checks_done().lock() {
                        done.insert(tui_bind_check_key(&row_key, &ses));
                    }
                    #[cfg(not(test))]
                    yggterm_core::append_trace_event(
                        home,
                        "daemon",
                        "opencode_mirror",
                        "opencode_bind_diverged",
                        serde_json::json!({
                            "row": row_key,
                            "pinned": ses,
                            "cwd": cwd,
                            "tui_ids": _tui_ids,
                        }),
                    );
                }
            }
        }
    }

    pub fn apply_opencode_tab_mirror(
        &mut self,
        sessions: &[OpencodeServiceSession],
        screen_live: &std::collections::HashSet<String>,
        terminal_titles: &std::collections::HashMap<String, String>,
    ) {
        let owned = owned_tabs_from(&self.sessions);
        // [11.148] the pin set that may retract a twin — computed before the
        // plan, from the same read (the anchor key is reused by the
        // anchor-as-header section below; one scan, one answer).
        let anchor_key = self.opencode_anchor_key(screen_live);
        let pinned_stable = pinned_stable_from(&owned, anchor_key.as_deref());
        let mut plan = plan_tab_sync(sessions, &owned, &pinned_stable, &self.sessions);
        // THE CLOSE OUTRANKS THE TAB. A service tab whose row the user closed
        // must not re-project — see `tombstoned_opencode_mirror_spawns`.
        if let Ok(home_dir) = crate::resolve_yggterm_home() {
            let vetoed: std::collections::HashSet<String> =
                tombstoned_opencode_mirror_spawns(&home_dir, &plan.spawn)
                    .into_iter()
                    .collect();
            if !vetoed.is_empty() {
                plan.spawn.retain(|ses| {
                    !vetoed.contains(&format!("opencode-runtime://{}", ses.id))
                });
                // The refusal re-arms the grave (see
                // LiveRowTombstones::touch_close): this mirror is an eternal
                // offerer on behalf of daemonized serves, so the veto must not
                // age out while the offers keep standing.
                crate::rearm_live_row_closes_among(&home_dir, vetoed.iter().map(String::as_str));
                yggterm_core::append_trace_event(
                    &home_dir,
                    "daemon",
                    "opencode_mirror",
                    "spawn_vetoed_closed_rows",
                    serde_json::json!({
                        "vetoed": vetoed,
                    }),
                );
            }
        }
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
                        "active_tabs": sessions.len(),
                        "active_ids": sessions.iter().map(|s| s.id.len()).collect::<Vec<_>>().len(),
                        "owned": owned.len(),
                        "plan_spawn": plan.spawn.len(),
                        "plan_retire": plan.retire.len(),
                        "plan_retract": plan.retract.len(),
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
        // whole listed set re-enters plan.spawn as adoptions. Budgeting them
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
            if let Some(prefix) = self.next_opencode_tab_seat(screen_live) {
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
        // [11.148] RETRACTION: a twin whose session a live non-mirror row
        // already pins is a DUPLICATE rail row for one TUI — remove it at its
        // deterministic spawn key. No tombstone: a session whose pinning row
        // later dies legitimately re-projects (the keep-alive law); only user
        // closes veto.
        let mut retracted: Vec<(String, String)> = Vec::new();
        for ses in &plan.retract {
            let key = format!("opencode-runtime://{ses}");
            if self.remove_live_session(&key).unwrap_or(false) {
                retracted.push((ses.clone(), key));
            }
        }
        if !retracted.is_empty() {
            #[cfg(not(test))]
            if let Ok(home_dir) = crate::resolve_yggterm_home() {
                yggterm_core::append_trace_event(
                    &home_dir,
                    "daemon",
                    "opencode_mirror",
                    "mirror_twin_retracted_pinned_elsewhere",
                    serde_json::json!({
                        "retracted": retracted,
                    }),
                );
            }
        }
        // Title sync: the row name IS the tab name. The service title is
        // authoritative for mirror rows (the human renames tabs in the TUI,
        // not in the sidebar), so drift is corrected every tick — placeholders
        // excepted (a never-prompted session's `New session - <iso>` would
        // UN-name a row; its directory name holds the handle instead).
        for ses in sessions {
            let Some(title) = mirror_display_title(ses) else {
                continue;
            };
            let Some(tab) = owned.get(&ses.id) else {
                continue;
            };
            if !tab.mirror_owned {
                // [11.148] the service title is authoritative for the mirror's
                // PROJECTIONS; a pinned wrapper/anchor row keeps its own name.
                continue;
            }
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
        if let Some(anchor_key) = anchor_key.clone() {
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
            //
            // [11.6.3-a]: the viewed stream is API-writer-only on beta-19271
            // (no TUI flow writes it — measured), so keyed on it alone this
            // starved to `no_viewing` on every quiet tick. The surface the
            // TUI DOES write is the per-window OSC title (`OC | <title>`),
            // and the anchor row IS the live TUI — its own title names the
            // session it renders. Precedence kept as documented: a real
            // viewed stream (this or a future build) outranks a possibly-
            // lagged title; the title answers when the stream is silent.
            let viewing = sessions
                .iter()
                .filter(|s| s.viewed_epoch_ms > 0)
                .max_by_key(|s| s.viewed_epoch_ms)
                .map(|s| s.id.clone())
                .or_else(|| {
                    let name = terminal_titles
                        .get(&anchor_key)?
                        .strip_prefix(OPENCODE_WINDOW_TITLE_PREFIX)?;
                    unique_session_by_title(sessions, name).map(|s| s.id.clone())
                });
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
            // ⭐ THE MIRROR-TICK REBIND: the anchor's bound IDENTITY follows
            // the session the TUI is actually rendering. The title follow and
            // the Viewing stamp are witnesses; this makes the underlying state
            // true — the row's id and its resume command become the viewed
            // session's, so the owner's click-to-resume lands in the session
            // he was LOOKING at (the handoff rule), a cold restart restores
            // that session, and the Dynamic title chore reads the right store
            // entry. Measured gap (dev, 2026-09-02): uuid-keyed anchors kept
            // their birth uuid as `id` for days — the phantom-resume class,
            // three live TUIs booting on ids no store ever held — while the
            // TUI rendered entirely different sessions.
            //
            // Rails: the anchor must be LIVE (a dead row renders nothing —
            // rebinding one would aim a resume at a ghost); the service's
            // focus stream is the vouch (the same stream tab-row births
            // already trust); and the rebind is a no-op on every tick where
            // the bound id already agrees, so a quiet TUI costs one compare.
            // One liveness predicate for the picker and the tick ([11.47]):
            // a screen-verdict `working` row is live even when pid/phase
            // bookkeeping was lost, and the rebind must hear about its
            // session switches just as the probe below does.
            let anchor_live = self
                .sessions
                .get(&anchor_key)
                .is_some_and(|a| Self::anchor_row_is_live(a, screen_live));
            if anchor_live {
                if let Some(ses_id) = viewing.clone() {
                    let bound = self.sessions.get(&anchor_key).map(|a| a.id.clone());
                    if bound.as_deref() != Some(ses_id.as_str())
                        && self.apply_agent_runtime_session_id_to_live_session_service_vouched(
                            &anchor_key,
                            &ses_id,
                        )
                    {
                        // BUS LAW (1cb614bcf): the unit tests run the real
                        // apply path for their return values; the fleet bus
                        // must not receive fixture rebinds (measured live
                        // 2026-09-04 04:04: ses_a0000.../ses_b0000... test
                        // rebinds read as real mirror decisions on dev).
                        #[cfg(not(test))]
                        if let Ok(home_dir) = crate::resolve_yggterm_home() {
                            yggterm_core::append_trace_event(
                                &home_dir,
                                "daemon",
                                "opencode_mirror",
                                "anchor_rebound_to_viewed_session",
                                serde_json::json!({
                                    "anchor": anchor_key,
                                    "bound_id": bound,
                                    "viewed_session": ses_id,
                                }),
                            );
                        }
                    }
                }
            }
            if !explicit {
                // The header follows the SAME viewing truth the metadata
                // stamp and the rebind use — one answer, three surfaces
                // ([11.6.3-a]: the OSC title keeps it alive when the viewed
                // stream is silent).
                if let Some(ses_id) = viewing.as_ref() {
                    if let Some(newest) = sessions.iter().find(|s| s.id == *ses_id) {
                        if let Some(title) = mirror_display_title(newest) {
                            if let Some(anchor) = self.sessions.get_mut(&anchor_key) {
                                anchor.title = title;
                            }
                        }
                    }
                }
            }
            // ⭐ PER-ROW TITLE IDENTITY (Issue Heading 34, Defect B): the
            // viewing truth stamps THE anchor; every OTHER live TUI carries
            // its own truth on the window-title plane — opencode2 titles its
            // window `OC | <session title>` per session route (measured in
            // its app.tsx; `OpenCode` on home/default = no signal). Bind a
            // live non-anchor row to the ONE service session its title
            // names, through the same service-vouched arm the anchor rebind
            // uses. Rails: the anchor is never overridden here (the focus
            // stream outranks a possibly-lagged title); dead rows never
            // bind; zero or multiple title matches bind nothing; a row
            // already bound in agreement costs one map lookup.
            let rows: Vec<(String, Option<String>)> = self
                .opencode_anchor_candidates()
                .into_iter()
                .filter(|row| Some(row.session_path.as_str()) != Some(anchor_key.as_str()))
                .map(|row| (row.session_path.clone(), Some(row.id.clone())))
                .collect();
            for (row_path, bound_id) in rows {
                let Some(pty_title) = terminal_titles.get(&row_path) else {
                    continue;
                };
                let Some(name) = pty_title.strip_prefix(OPENCODE_WINDOW_TITLE_PREFIX)
                else {
                    continue;
                };
                let Some(session) = unique_session_by_title(sessions, name) else {
                    continue;
                };
                let Some(bound) = bound_id else { continue };
                if bound == session.id {
                    continue;
                }
                let live = self
                    .sessions
                    .get(&row_path)
                    .is_some_and(|a| Self::anchor_row_is_live(a, screen_live));
                if !live {
                    continue;
                }
                if self.apply_agent_runtime_session_id_to_live_session_service_vouched(
                    &row_path,
                    &session.id,
                ) {
                    // BUS LAW: gated — tests run the apply path for state.
                    #[cfg(not(test))]
                    if let Ok(home_dir) = crate::resolve_yggterm_home() {
                        yggterm_core::append_trace_event(
                            &home_dir,
                            "daemon",
                            "opencode_mirror",
                            "row_rebound_to_title_session",
                            serde_json::json!({
                                "row": row_path,
                                "bound_id": bound,
                                "title_session": session.id,
                            }),
                        );
                    }
                }
            }
            // Issue 31 probe: the tick's identity decision, on the plane. The
            // anchor the picker chose, how many rows qualified, what the
            // service is viewing, what the anchor is bound to — and the
            // verdict. `diverged` (bound ≠ viewing, no rebind on this build)
            // forces emission: it is the event the 2026-09-03 four-stale-rows
            // incident needed and no instrument could give.
            let anchor_row = self.sessions.get(&anchor_key);
            let anchor_live = anchor_row.is_some_and(|a| Self::anchor_row_is_live(a, screen_live));
            let bound = anchor_row.map(|a| a.id.clone());
            let decision = if !anchor_live {
                "anchor_not_live"
            } else if viewing.is_none() {
                "no_viewing"
            } else if bound.as_deref() == viewing.as_deref() {
                "in_sync"
            } else {
                "diverged"
            };
            // The plane's own law: a sweep that never reports when quiet is
            // indistinguishable from a chore that stopped running. Interesting
            // ticks always speak; quiet ones heartbeat every five minutes
            // (~288 small events a day — the steady-state cost is stated in
            // the Issue 31 spec, not discovered later).
            static MIRROR_TICK_HEARTBEAT_LAST_MS: std::sync::OnceLock<std::sync::Mutex<u64>> =
                std::sync::OnceLock::new();
            let heartbeat_due = crate::current_millis_u64()
                .saturating_sub(
                    MIRROR_TICK_HEARTBEAT_LAST_MS
                        .get_or_init(|| std::sync::Mutex::new(0))
                        .lock()
                        .map(|guard| *guard)
                        .unwrap_or(0),
                )
                >= 300_000;
            if spawned > 0
                || retired > 0
                || plan.focus.is_some()
                || decision == "diverged"
                || heartbeat_due
            {
                #[cfg(not(test))]
                {
    yggterm_core::cli_plane::emit_mirror_tick(
                        "daemon",
                        crate::SessionKind::OpenCode,
                        yggterm_core::cli_plane::CliMirrorTickDecision {
                            anchor: Some(anchor_key.as_str()),
                            candidates: self.opencode_anchor_candidates().len(),
                            viewing: viewing.as_deref(),
                            bound: bound.as_deref(),
                            decision,
                            active_tabs: sessions.len(),
                        },
                    );
                }
                if let Ok(mut guard) = MIRROR_TICK_HEARTBEAT_LAST_MS
                    .get_or_init(|| std::sync::Mutex::new(0))
                    .lock()
                {
                    *guard = crate::current_millis_u64();
                }
            }
        } else {
            // No row qualified as anchor at all — and a tick that cannot name
            // its anchor is exactly as interesting as a diverged one: the
            // mirror is running with nothing to steer.
            if spawned > 0 || retired > 0 || plan.focus.is_some() {
                #[cfg(not(test))]
                {
    yggterm_core::cli_plane::emit_mirror_tick(
                        "daemon",
                        crate::SessionKind::OpenCode,
                        yggterm_core::cli_plane::CliMirrorTickDecision {
                            anchor: None,
                            candidates: 0,
                            viewing: sessions
                                .iter()
                                .filter(|s| s.viewed_epoch_ms > 0)
                                .max_by_key(|s| s.viewed_epoch_ms)
                                .map(|s| s.id.as_str()),
                            bound: None,
                            decision: "no_anchor",
                            active_tabs: sessions.len(),
                        },
                    );
                }
            }
        }
        // The [11.134] closer rides the same tick: once per generation, every
        // live pinned row's TUI tab map is asked which session it really
        // binds; the verdict speaks on the trace plane by name.
        if let Ok(home_dir) = crate::resolve_yggterm_home() {
            self.run_tui_bind_checks(&home_dir, &owned, sessions, screen_live);
        }
        let mut focused = None;
        if let Some(ses_id) = &plan.focus {
            // Follow the human's tab switch only while they are already in
            // the opencode context (the anchor or a mirrored tab is the
            // sessions row); elsewhere in the GUI a tab switch must not yank
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
            if let Some(newest) = sessions.iter().find(|s| &s.id == ses_id) {
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
        if spawned > 0 || retired > 0 || !retracted.is_empty() || plan.focus.is_some() {
            if let Ok(home_dir) = crate::resolve_yggterm_home() {
                yggterm_core::append_trace_event(
                    &home_dir,
                    "daemon",
                    "opencode_mirror",
                    "tab_sync",
                    serde_json::json!({
                        "spawned": spawned,
                        "retired": retired,
                        "retracted": retracted.len(),
                        "focus": plan.focus,
                        "active_tabs": sessions.len(),
                    }),
                );
            }
        }
    }

    /// Whether one row's liveness marks say a TUI is actually running on it.
    ///
    /// Three marks, one truth: the pid, a running launch phase, or — the mark
    /// restored rows keep losing — the DAEMON HOLDS A READABLE SCREEN for the
    /// PTY (`screen_live`, computed by the caller from `TerminalManager`
    /// under the same lock the mirror applies under). ⛔ `working` is NOT a
    /// mark here: it is stamped on SNAPSHOT projections, never on the
    /// internal rows this mirror reads — the 2026-09-03 18:46 read showed
    /// `anchor_not_live` persisting on the FIXED build because the predicate
    /// trusted a field the mirror can never see. Split out so the picker and
    /// the tick answer from ONE function.
    fn anchor_row_is_live(
        row: &crate::ManagedSessionView,
        screen_live: &std::collections::HashSet<String>,
    ) -> bool {
        row.terminal_process_id.is_some()
            || matches!(
                row.launch_phase,
                crate::TerminalLaunchPhase::Running | crate::TerminalLaunchPhase::RemoteBootstrap
            )
            || screen_live.contains(&row.session_path)
    }

    /// The live opencode TUI row (the mirror's seating anchor) and the next
    /// free sub-seat under it: `<anchor outline>.<n+1>`.
    fn opencode_anchor_candidates(&self) -> Vec<&crate::ManagedSessionView> {
        self.sessions
            .values()
            .filter(|s| {
                s.kind == crate::SessionKind::OpenCode
                    && s.session_path.starts_with("opencode-runtime://")
                    && !s
                        .metadata
                        .iter()
                        .any(|m| m.label == "Source" && m.value == TAB_SOURCE_METADATA)
            })
            .collect()
    }

    fn opencode_anchor_key(
        &self,
        screen_live: &std::collections::HashSet<String>,
    ) -> Option<String> {
        // ⛔ THE ANCHOR IS THE LIVE TUI, NOT THE FIRST ROW THAT QUALIFIES.
        // The original predicate took the first OpenCode row without the
        // mirror stamp — with several uuid-keyed rows in the set (anchors of
        // TUIs that died, phantom resumes of ids no store ever held — both
        // measured live 2026-09-02: four uuid rows, one real TUI) the
        // anchor-as-header title landed on an arbitrary dead row while the
        // real TUI kept its stale name. Prefer a row that is actually
        // RUNNING; only a set with no live TUI at all falls back to the
        // historical first-qualified order.
        //
        // ⭐ 2026-09-03: `working` is a LIVE mark too. The daemon's working
        // verdict exists ONLY when it holds a readable screen for the PTY
        // (`None` = no live screen), so `working.is_some()` is the daemon
        // saying "this PTY is alive and I am reading it" — independent of
        // pid bookkeeping, which restored rows lose. Measured: five owned
        // opencode rows all screen-verdict `working`, every one without
        // usable pid/phase marks — `anchor_not_live` short-circuited the
        // rebind on all five while real session switches streamed past.
        // The predicate and the screen classifier must agree about what
        // "live" means; they are the same daemon looking at the same PTY.
        let candidates = self.opencode_anchor_candidates();
        candidates
            .iter()
            .find(|s| Self::anchor_row_is_live(s, screen_live))
            .copied()
            .or_else(|| candidates.first().copied())
            .map(|a| a.session_path.clone())
    }

    fn next_opencode_tab_seat(
        &self,
        screen_live: &std::collections::HashSet<String>,
    ) -> Option<String> {
        let anchor_key = self.opencode_anchor_key(screen_live)?;
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

    fn empty_titles() -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn empty_screen_live() -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }


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
                mirror_owned: true,
            },
        )
    }

    // ---- [11.134] bind-divergence check fixtures ----------------------------

    fn server_with_live_pinned_row() -> (crate::YggtermServer, String) {
        let mut server = crate::YggtermServer::new(
            false,
            crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
            yggui_contract::UiTheme::ZedLight,
        );
        let key = server.start_local_session(
            crate::SessionKind::OpenCode,
            Some("/home/user/proj"),
            Some("Remote OpenCode bindvrfy"),
        );
        let row = server.sessions.get_mut(&key).expect("the row exists");
        row.launch_phase = crate::TerminalLaunchPhase::Running;
        row.terminal_process_id = Some(4242);
        row.source = crate::SessionSource::LiveLocal;
        let mut row = server.sessions.remove(&key).expect("the row exists");
        let new_key = format!("opencode-runtime://{}", row.id);
        row.session_path = new_key.clone();
        server.sessions.insert(new_key.clone(), row);
        (server, new_key)
    }

    #[test]
    fn tui_bind_verdict_reads_positive_evidence_only_across_siblings() {
        let pinned = "ses_bindpin00000000000000000001";
        let other = "ses_bindoth00000000000000000001";
        let beta: std::collections::BTreeMap<String, Vec<String>> = [(
            "/home/user/proj".to_string(),
            vec![other.to_string()],
        )]
        .into_iter()
        .collect();
        let latest: std::collections::BTreeMap<String, Vec<String>> = [(
            "/home/user/proj".to_string(),
            vec![pinned.to_string(), other.to_string()],
        )]
        .into_iter()
        .collect();
        let instances = vec![
            ("beta".to_string(), beta),
            ("latest".to_string(), latest),
        ];
        assert_eq!(
            tui_bind_verdict(&instances, "/home/user/proj", pinned),
            TuiBindVerdict::Verified,
            "a sibling instance naming the pinned id under the cwd is a bind — \
             latest/ being empty or stale must not veto it"
        );
        let only_other = vec![(
            "latest".to_string(),
            [(
                "/home/user/proj".to_string(),
                vec![other.to_string()],
            )]
            .into_iter()
            .collect::<std::collections::BTreeMap<String, Vec<String>>>(),
        )];
        assert_eq!(
            tui_bind_verdict(&only_other, "/home/user/proj", pinned),
            TuiBindVerdict::Diverged(vec![other.to_string()]),
            "a cwd named with OTHER ids only is divergence — positive evidence"
        );
        assert_eq!(
            tui_bind_verdict(&only_other, "/home/user/other-cwd", pinned),
            TuiBindVerdict::Unknown,
            "a cwd no instance names is Unknown — absence is never divergence"
        );
        assert_eq!(
            tui_bind_verdict(&Vec::new(), "/home/user/proj", pinned),
            TuiBindVerdict::Unknown,
            "an absent plane is Unknown, never a verdict"
        );
    }

    #[test]
    fn plan_tui_bind_checks_skips_dead_remote_and_already_done_rows() {
        let ses_id = "ses_bindpln00000000000000000001";
        let dead_ses = "ses_binddead0000000000000000001";
        let remote_ses = "ses_bindrem00000000000000000001";
        let done_ses = "ses_binddone0000000000000000001";
        let (mut server, live_key) = server_with_live_pinned_row();
        let rekey = |server: &mut crate::YggtermServer, key: &str| {
            let mut row = server.sessions.remove(key).expect("the row exists");
            let new_key = format!("opencode-runtime://{}", row.id);
            row.session_path = new_key.clone();
            server.sessions.insert(new_key.clone(), row);
            new_key
        };
        let dead_key = {
            let key = server.start_local_session(
                crate::SessionKind::OpenCode,
                Some("/home/user/proj"),
                Some("Remote OpenCode binddead"),
            );
            let row = server.sessions.get_mut(&key).expect("the row exists");
            row.launch_phase = crate::TerminalLaunchPhase::Queued;
            row.terminal_process_id = None;
            rekey(&mut server, &key)
        };
        let remote_key = {
            let key = server.start_local_session(
                crate::SessionKind::OpenCode,
                Some("/home/user/proj"),
                Some("Remote OpenCode bindremote"),
            );
            let row = server.sessions.get_mut(&key).expect("the row exists");
            row.launch_phase = crate::TerminalLaunchPhase::Running;
            row.terminal_process_id = Some(4243);
            // The ssh FIELD is not the discriminator (local births carry
            // Some("localhost") — measured in the scratch pass); the SOURCE
            // is.
            row.ssh_target = Some("localhost".to_string());
            row.source = crate::SessionSource::LiveSsh;
            rekey(&mut server, &key)
        };
        let owned_map = std::collections::HashMap::from([
            (
                ses_id.to_string(),
                OwnedTab {
                    key: live_key.clone(),
                    viewed_epoch_ms: 100,
                    engaged: true,
                    mirror_owned: true,
                },
            ),
            (
                dead_ses.to_string(),
                OwnedTab {
                    key: dead_key.clone(),
                    viewed_epoch_ms: 100,
                    engaged: true,
                    mirror_owned: true,
                },
            ),
            (
                remote_ses.to_string(),
                OwnedTab {
                    key: remote_key.clone(),
                    viewed_epoch_ms: 100,
                    engaged: true,
                    mirror_owned: true,
                },
            ),
            (
                done_ses.to_string(),
                OwnedTab {
                    key: live_key.clone(),
                    viewed_epoch_ms: 100,
                    engaged: true,
                    mirror_owned: true,
                },
            ),
        ]);
        if let Ok(mut done) = tui_bind_checks_done().lock() {
            done.insert(tui_bind_check_key(&live_key, done_ses));
        }
        let service = vec![
            ses(ses_id, 100),
            ses(dead_ses, 100),
            ses(remote_ses, 100),
            ses(done_ses, 100),
        ];
        assert_eq!(
            plan_tui_bind_checks(&owned_map, &server.sessions, &service, &empty_screen_live()),
            vec![(live_key.clone(), ses_id.to_string(), "/home/user/proj".to_string())],
            "only the live local not-yet-checked row with a service cwd owes a check"
        );
    }

    #[test]
    fn the_bind_check_delivers_once_and_keeps_unknown_open() {
        let ses_id = "ses_bindstate000000000000000001";
        let (server, row_key) = server_with_live_pinned_row();
        let owned_map = std::collections::HashMap::from([(
            ses_id.to_string(),
            OwnedTab {
                key: row_key.clone(),
                viewed_epoch_ms: 100,
                engaged: true,
                mirror_owned: true,
            },
        )]);
        let service = vec![ses(ses_id, 100)];
        let home = std::env::temp_dir().join(format!(
            "yggterm-bind-check-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        std::fs::create_dir_all(home.join(".local/state/opencode/latest/tui")).expect("tabs dir");
        let tabs_path = home.join(".local/state/opencode/latest/tui/tabs.json");

        // Unknown: the plane has not painted yet — no verdict, the check
        // stays open for a later tick.
        server.run_tui_bind_checks(&home, &owned_map, &service, &empty_screen_live());
        assert!(
            !tui_bind_check_was_delivered(&row_key, ses_id),
            "absence is never a verdict"
        );

        // The TUI paints the pinned id under the row cwd -> verified, once.
        std::fs::write(
            &tabs_path,
            format!(
                r#"{{"cwd":{{"/home/user/proj":{{"tabs":[{{"sessionID":"{ses_id}","title":"t"}}],"unread":{{}}}}}}}}"#
            ),
        )
        .expect("write tabs");
        server.run_tui_bind_checks(&home, &owned_map, &service, &empty_screen_live());
        assert!(tui_bind_check_was_delivered(&row_key, ses_id));

        // The once law: a later divergence (a legal in-TUI switch) does not
        // re-judge the row this generation.
        std::fs::write(
            &tabs_path,
            r#"{"cwd":{"/home/user/proj":{"tabs":[{"sessionID":"ses_bindsw0000000000000000001","title":"t"}],"unread":{}}}}"#,
        )
        .expect("rewrite tabs");
        server.run_tui_bind_checks(&home, &owned_map, &service, &empty_screen_live());
        assert!(
            tui_bind_check_was_delivered(&row_key, ses_id),
            "already-delivered rows are never re-checked this generation"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// LOCK: the bind check rides the apply tick and names its verdicts —
    /// a silent verdict plane is indistinguishable from a missing one.
    #[test]
    fn the_bind_check_rides_the_apply_tick_and_names_its_verdicts() {
        let source = include_str!("opencode_mirror.rs");
        let body = source
            .split("pub fn apply_opencode_tab_mirror")
            .nth(1)
            .expect("apply_opencode_tab_mirror exists")
            .split("\n    pub fn ")
            .next()
            .expect("the end of the apply fn");
        assert!(
            body.contains("run_tui_bind_checks("),
            "the apply tick must run the bind check"
        );
        assert!(
            source.contains("\"opencode_bind_verified\""),
            "the verified verdict must be traced by name"
        );
        assert!(
            source.contains("\"opencode_bind_diverged\""),
            "the diverged verdict must be traced by name — divergence the trace never names is [11.134] unfixed"
        );
        assert!(
            source.contains("\"tui_ids\""),
            "the diverged payload must carry BOTH ids — the pinned one and what the TUI really renders"
        );
    }

    #[test]
    fn tombstoned_opencode_mirror_spawns_blocks_the_closed_tab_only() {
        let home = std::env::temp_dir().join(format!(
            "yggterm-mirror-veto-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        std::fs::create_dir_all(&home).expect("create temp home");

        let closed = "ses_closed00000000000000000001";
        let open = "ses_opened00000000000000000001";
        // Production records the FOLDED identity (remove_session folds before
        // record_close); the ask folds through the same door, so the test
        // must record through the same fold.
        let closed_key = format!("opencode-runtime://{closed}");
        crate::live_row_tombstones::LiveRowTombstones::default()
            .record_close(
                &home,
                &crate::normalized_live_row_identity(&closed_key),
                crate::live_row_tombstones::now_secs(),
            )
            .expect("record the close the way remove_session does");

        let blocked = super::tombstoned_opencode_mirror_spawns(
            &home,
            &[ses(closed, 100), ses(open, 100)],
        );
        assert_eq!(
            blocked,
            vec![closed_key.clone()],
            "the service keeps the closed session as a tab; the mirror must not              re-project it — this is the peer-re-offer case the tombstone plane              was built for"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// LOCK: the mirror's apply keeps asking the tombstone plane before it
    /// spawns. A working veto is invisible (the row simply never appears), so
    /// a refactor that drops the ask would ship silently — the 2026-09-16
    /// ghost reports are what an unasked door looks like.
    #[test]
    fn apply_opencode_tab_mirror_asks_the_tombstone_plane_before_spawning() {
        let source = include_str!("opencode_mirror.rs");
        let body = source
            .split("pub fn apply_opencode_tab_mirror")
            .nth(1)
            .expect("apply_opencode_tab_mirror exists")
            .split("\n    pub fn ")
            .next()
            .expect("the end of the apply fn");
        let veto = body
            .find("tombstoned_opencode_mirror_spawns(")
            .expect("the mirror apply must ask the tombstone plane");
        let spawn_loop = body
            .find("for ses in plan.spawn.iter()")
            .expect("the spawn loop must still exist");
        assert!(
            veto < spawn_loop,
            "the tombstone ask must sit before the spawn loop — a veto that              runs after the row landed is too late"
        );
        assert!(
            body.contains("\"spawn_vetoed_closed_rows\""),
            "the veto must be traced, or a silent veto cannot be told from a              broken one"
        );
    }

    #[test]
    fn a_new_tab_spawns_a_cold_row_and_a_closed_unengaged_tab_retires() {
        let sessions = vec![ses("ses_new000000000000000000001", 100)];
        let owned_map = std::collections::HashMap::from([owned(
            "ses_gone00000000000000000001",
            "opencode-runtime://ses_gone00000000000000000001",
            50,
            false,
        )]);
        let plan = plan_tab_sync(&sessions, &owned_map, &std::collections::HashSet::new(), &std::collections::BTreeMap::new());
        assert_eq!(plan.spawn.len(), 1);
        assert_eq!(plan.spawn[0].id, "ses_new000000000000000000001");
        assert_eq!(plan.retire, vec!["ses_gone00000000000000000001"]);
        // Focus: the new session's view (100) moved past nothing we mirror —
        // no row exists for it yet, so nothing to follow this tick.
        assert_eq!(plan.focus, None);
    }

    #[test]
    fn an_engaged_row_is_never_retired_for_losing_its_tab() {
        let sessions = vec![]; // the tab closed everywhere
        let owned_map = std::collections::HashMap::from([owned(
            "ses_engaged00000000000000001",
            "opencode-runtime://ses_engaged00000000000000001",
            50,
            true, // the user opened it: it is a window now
        )]);
        let plan = plan_tab_sync(&sessions, &owned_map, &std::collections::HashSet::new(), &std::collections::BTreeMap::new());
        assert!(plan.retire.is_empty(), "windows close when the user closes them");
        assert!(plan.spawn.is_empty());
    }

    #[test]
    fn focus_follows_a_freshly_viewed_session_and_stands_down_when_quiet() {
        let sessions = vec![ses("ses_mirrored0000000000000000001", 9_000)];
        let owned_map = std::collections::HashMap::from([owned(
            "ses_mirrored0000000000000000001",
            "opencode-runtime://ses_mirrored0000000000000000001",
            1_000,
            false,
        )]);
        let plan = plan_tab_sync(&sessions, &owned_map, &std::collections::HashSet::new(), &std::collections::BTreeMap::new());
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
        let plan = plan_tab_sync(&sessions, &settled, &std::collections::HashSet::new(), &std::collections::BTreeMap::new());
        assert_eq!(plan.focus, None, "quiet tick — nothing to follow");
    }

    /// [11.6.3-a]'s red case at the plan level: an idle fleet's store list
    /// still mirrors. Under the old working-set universe this same tick
    /// spawned nothing and retired every unengaged row — rows blinked into
    /// existence with a turn and vanished when it settled.
    #[test]
    fn an_idle_fleets_store_list_still_mirrors_and_never_retires() {
        let mut idle = ses("ses_idle000000000000000000001", 0);
        idle.running = false;
        idle.viewed_epoch_ms = 0;
        // A listed session with no row yet still spawns, idle or not.
        let plan = plan_tab_sync(&[idle.clone()], &std::collections::HashMap::new(), &std::collections::HashSet::new(), &std::collections::BTreeMap::new());
        assert_eq!(
            plan.spawn.len(),
            1,
            "an idle listed session still gets its projection row"
        );
        // A row whose session is LISTED but idle is never retired.
        let owned_map = std::collections::HashMap::from([owned(
            "ses_idle000000000000000000001",
            "opencode-runtime://ses_idle000000000000000000001",
            0,
            false,
        )]);
        let plan = plan_tab_sync(&[idle], &owned_map, &std::collections::HashSet::new(), &std::collections::BTreeMap::new());
        assert!(
            plan.retire.is_empty(),
            "listed-but-idle never retires an unengaged row"
        );
        assert!(plan.spawn.is_empty());
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
                "ses_abc000000000000000000001",
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
                "ses_abc000000000000000000001",
            ),
            Some("ses_abc000000000000000000001".to_string())
        );
        // uuid-keyed rows are anchors or phantoms — never mirror rows BY KEY.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::OpenCode,
                "opencode-runtime://0d841111-1111-4111-8111-111111111111",
                None,
                "0d841111-1111-4111-8111-111111111111",
            ),
            None
        );
        // Other kinds are never mirror rows.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::ClaudeCode,
                "opencode-runtime://ses_abc000000000000000000001",
                None,
                "ses_abc000000000000000000001",
            ),
            None
        );
    }

    /// [11.148] THE ID ARM: a wrapper row's key carries no ses id and its
    /// spawn path writes no Tab Session Id metadata — the pin lives in the
    /// row plane's own `session.id`, and that alone must own the tab.
    #[test]
    fn a_wrapper_rows_own_discovered_id_owns_its_tab() {
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::OpenCode,
                "local://2bcd5a95-1111-4111-8111-111111111111",
                None,
                "ses_f477cab000000000000000000001",
            ),
            Some("ses_f477cab000000000000000000001".to_string())
        );
        // A uuid id (a fresh wrapper row before the pin, a phantom) owns
        // nothing.
        assert_eq!(
            mirror_tab_session_id(
                crate::SessionKind::OpenCode,
                "local://2bcd5a95-1111-4111-8111-111111111111",
                None,
                "0d841111-1111-4111-8111-111111111111",
            ),
            None
        );
    }
}

#[cfg(test)]
mod twin_retract_tests {
    use super::*;

    const SES: &str = "ses_twinretract00000000000001";

    fn listed(id: &str, viewed: u128) -> OpencodeServiceSession {
        OpencodeServiceSession {
            id: id.to_string(),
            title: Some(id.to_string()),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: viewed,
            viewed_epoch_ms: viewed,
            running: true,
        }
    }

    /// A wrapper-shaped live row: `local://…` key (NOT the mirror's key
    /// shape), optional discovered pin in the row plane's own id.
    fn server_with_wrapper_row(
        pinned: Option<&str>,
        engaged: bool,
    ) -> (crate::YggtermServer, String) {
        let mut server = crate::YggtermServer::new(
            false,
            crate::GhosttyHostSupport::shadow("test".to_string(), false, false),
            yggui_contract::UiTheme::ZedLight,
        );
        let key = server.start_local_session(
            crate::SessionKind::OpenCode,
            Some("/home/user/proj"),
            Some("opencode wrapper row"),
        );
        let row = server.sessions.get_mut(&key).expect("the row exists");
        if engaged {
            row.launch_phase = crate::TerminalLaunchPhase::Running;
            row.terminal_process_id = Some(4242);
        } else {
            // start_local_session leaves a fresh row in an ENGAGED phase —
            // the disengaged arm must say so explicitly.
            row.launch_phase = crate::TerminalLaunchPhase::Queued;
            row.terminal_process_id = None;
        }
        if let Some(ses) = pinned {
            row.id = ses.to_string();
        }
        (server, key)
    }

    /// A mirror PROJECTION row at the deterministic twin key, Source-stamped
    /// the way the spawn path stamps it.
    fn server_with_twin_row(engaged: bool) -> (crate::YggtermServer, String) {
        let twin_key = format!("opencode-runtime://{SES}");
        let (mut server, _) = server_with_wrapper_row(None, false);
        let mut row = server
            .sessions
            .values()
            .next()
            .expect("a row exists")
            .clone();
        if engaged {
            row.launch_phase = crate::TerminalLaunchPhase::Running;
            row.terminal_process_id = Some(4242);
        } else {
            row.launch_phase = crate::TerminalLaunchPhase::Queued;
            row.terminal_process_id = None;
        }
        crate::upsert_session_metadata(
            &mut row.metadata,
            "Source",
            TAB_SOURCE_METADATA.to_string(),
        );
        row.session_path = twin_key.clone();
        server.sessions.clear();
        server.sessions.insert(twin_key.clone(), row);
        (server, twin_key)
    }

    /// [11.148] THE DEFECT: a live wrapper row pinned its session — the tab
    /// is owned through the row plane's own id, the sync spawns NO twin, and
    /// the user's row is never a retire candidate.
    #[test]
    fn a_pinned_wrapper_row_suppresses_the_twin_and_is_never_retired() {
        let (server, key) = server_with_wrapper_row(Some(SES), true);
        let owned = owned_tabs_from(&server.sessions);
        let tab = owned
            .get(SES)
            .expect("the wrapper row owns its pin through the id arm");
        assert_eq!(tab.key, key, "the authoritative row holds the slot");
        assert!(!tab.mirror_owned, "a wrapper row is not a projection");
        assert!(tab.engaged);
        let pinned = pinned_stable_from(&owned, None);
        assert!(pinned.contains(SES), "a live wrapper pin is stable");
        let plan = plan_tab_sync(&[listed(SES, 100)], &owned, &pinned, &server.sessions);
        assert!(
            plan.spawn.is_empty(),
            "no twin for a session a live row already renders"
        );
        assert!(plan.retire.is_empty(), "the wrapper row is never retired");
        assert!(plan.retract.is_empty(), "the wrapper row is not a twin");
    }

    /// A twin born in the seconds before the pin landed retracts once the
    /// wrapper pins — the duplicate rail row goes away.
    #[test]
    fn an_unengaged_twin_whose_session_a_live_wrapper_row_pins_is_retracted() {
        let (mut server, _twin_key) = server_with_twin_row(false);
        let (mut wrapper_host, wrapper_key) = server_with_wrapper_row(Some(SES), true);
        let wrapper_row = wrapper_host
            .sessions
            .values()
            .next()
            .expect("the wrapper row exists")
            .clone();
        server.sessions.insert(wrapper_key.clone(), wrapper_row);
        let owned = owned_tabs_from(&server.sessions);
        assert_eq!(
            owned.get(SES).expect("owned").key,
            wrapper_key,
            "the authoritative row wins the slot over the projection"
        );
        let pinned = pinned_stable_from(&owned, None);
        let plan = plan_tab_sync(&[listed(SES, 100)], &owned, &pinned, &server.sessions);
        assert_eq!(
            plan.retract,
            vec![SES.to_string()],
            "the duplicate twin is named for retraction"
        );
    }

    /// An ENGAGED twin is a window the user opened — a real second client on
    /// a multi-native CLI — never a duplicate to delete.
    #[test]
    fn an_opened_twin_is_a_window_and_is_never_retracted() {
        let (mut server, _twin_key) = server_with_twin_row(true);
        let (mut wrapper_host, wrapper_key) = server_with_wrapper_row(Some(SES), true);
        let wrapper_row = wrapper_host
            .sessions
            .values()
            .next()
            .expect("the wrapper row exists")
            .clone();
        server.sessions.insert(wrapper_key.clone(), wrapper_row);
        let owned = owned_tabs_from(&server.sessions);
        let pinned = pinned_stable_from(&owned, None);
        let plan = plan_tab_sync(&[listed(SES, 100)], &owned, &pinned, &server.sessions);
        assert!(
            plan.retract.is_empty(),
            "an opened twin is the user's window, not a rail duplicate"
        );
    }

    /// The anchor's bound id follows the tab the human is viewing — its pin
    /// is UNSTABLE and must never arm retraction (a projection would flap
    /// per tab switch).
    #[test]
    fn the_anchor_pin_is_not_stable() {
        let ses_a = "ses_stablepin0000000000000001";
        let ses_b = "ses_anchorpin0000000000000001";
        let (mut server, wrapper_key) = server_with_wrapper_row(Some(ses_a), true);
        let anchor_key = format!("local://anchor-{wrapper_key}");
        let mut anchor = server
            .sessions
            .values()
            .next()
            .expect("a row exists")
            .clone();
        anchor.id = ses_b.to_string();
        anchor.terminal_process_id = Some(4243);
        anchor.launch_phase = crate::TerminalLaunchPhase::Running;
        server.sessions.insert(anchor_key.clone(), anchor);
        let owned = owned_tabs_from(&server.sessions);
        let pinned = pinned_stable_from(&owned, Some(&anchor_key));
        assert!(
            pinned.contains(ses_a),
            "a non-anchor wrapper pin is stable"
        );
        assert!(
            !pinned.contains(ses_b),
            "the anchor's pin never arms retraction"
        );
    }

    /// A dead pinning row owns nothing stable — no retraction, and the twin
    /// (or a fresh one) stays as the session's keep-alive.
    #[test]
    fn a_dead_wrapper_pin_keeps_the_twin() {
        let (server, _key) = server_with_wrapper_row(Some(SES), false);
        let (mut both, _twin_key) = server_with_twin_row(false);
        let wrapper_row = server
            .sessions
            .values()
            .next()
            .expect("the wrapper row exists")
            .clone();
        both.sessions.insert("local://dead-wrapper".to_string(), wrapper_row);
        let owned = owned_tabs_from(&both.sessions);
        let pinned = pinned_stable_from(&owned, None);
        assert!(
            !pinned.contains(SES),
            "a disengaged wrapper row is not a live pin"
        );
        let plan = plan_tab_sync(&[listed(SES, 100)], &owned, &pinned, &both.sessions);
        assert!(
            plan.retract.is_empty(),
            "the twin stays while no live row renders the session"
        );
    }
}

mod anchor_tests {
    use super::*;

    fn empty_titles() -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn empty_screen_live() -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }

    /// The 2026-09-03 18:46 live read: a row the daemon screen-verdicts
    /// `working` but that carries NO pid/phase marks (restored rows lose
    /// them) must be ANCHORABLE — the screen-live set is its liveness proof.
    /// Without it, anchor_not_live short-circuits the rebind forever.
    #[test]
    fn a_screen_verified_row_is_a_live_anchor_without_pid_marks() {
        let (mut server, _dead, live) = server_with_two_anchors();
        // Strip the pid/phase marks the old predicate keyed on.
        {
            let row = server.sessions.get_mut(&live).expect("live row");
            row.launch_phase = crate::TerminalLaunchPhase::Queued;
            row.terminal_process_id = None;
        }
        let screen_live: std::collections::HashSet<String> =
            [live.clone()].into_iter().collect();
        assert_eq!(
            server.opencode_anchor_key(&screen_live),
            Some(live.clone()),
            "a daemon-readable screen is liveness proof on its own"
        );
        // And the anchor block agrees with the picker (one function).
        let anchor_row = server.sessions.get(&live).expect("row");
        assert!(
            YggtermServer::anchor_row_is_live(anchor_row, &screen_live),
            "picker and tick share the one liveness predicate"
        );
    }

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
            server.opencode_anchor_key(&empty_screen_live()),
            Some(live),
            "a RUNNING TUI outranks a dead uuid row in anchor selection",
        );
    }

    #[test]
    fn the_anchor_rebinds_its_identity_to_the_session_it_is_viewing() {
        let (mut server, _dead, _live) = server_with_two_anchors();
        // The real plane's rows carry Cwd; the rebind's birth-command rebuild
        // reads it to recompose the resume.
        for row in server.sessions.values_mut() {
            crate::upsert_session_metadata(
                &mut row.metadata,
                "Cwd",
                "/home/user/proj".to_string(),
            );
        }
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
        let sessions = vec![
            viewed("ses_a0000000000000000000000001", 100),
            viewed("ses_b0000000000000000000000002", 200),
        ];
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &empty_titles());
        let anchor_key = server
            .opencode_anchor_key(&empty_screen_live())
            .expect("an anchor exists");
        let anchor = server.sessions.get(&anchor_key).expect("anchor row");
        // The anchor was born wearing its row uuid as its id — the
        // phantom-resume class. After the tick its identity is the session
        // the human is LOOKING at, and the resume command says so.
        assert_eq!(
            anchor.id, "ses_b0000000000000000000000002",
            "identity follows the viewed session, not the birth uuid",
        );
        assert!(
            anchor.launch_command.contains("ses_b0000000000000000000000002"),
            "the resume command must name the rebound session, got: {}",
            anchor.launch_command,
        );
    }

    #[test]
    fn a_dead_anchor_is_never_rebound_to_a_viewed_session() {
        let (mut server, dead, live) = server_with_two_anchors();
        // Nothing is rendering: strip the live marks from BOTH rows so the
        // anchor pick falls back to first-qualified on a corpse. A rebind
        // here would aim a resume at a ghost.
        for row in server.sessions.values_mut() {
            row.launch_phase = crate::TerminalLaunchPhase::Queued;
            row.terminal_process_id = None;
        }
        let viewed = |id: &str, viewed: u128| OpencodeServiceSession {
            id: id.to_string(),
            title: Some(format!("Tab {id}")),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: viewed,
            running: true,
        };
        let sessions = vec![viewed("ses_b0000000000000000000000002", 200)];
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &empty_titles());
        for key in [dead, live] {
            let row = server.sessions.get(&key).expect("fixture row survives");
            assert_ne!(
                row.id, "ses_b0000000000000000000000002",
                "a dead anchor must not adopt the viewed session id",
            );
        }
    }

    #[test]
    fn a_live_non_anchor_row_rebinds_to_the_session_its_window_title_names() {
        let (mut server, dead, live) = server_with_two_anchors();
        // Both rows LIVE: one wins the anchor pick (the focus stream's row),
        // the other is exactly Defect B's stampless anchor.
        for key in [&dead, &live] {
            let row = server.sessions.get_mut(key).expect("fixture row");
            row.launch_phase = crate::TerminalLaunchPhase::Running;
            row.terminal_process_id = Some(4242);
        }
        for key in [&dead, &live] {
            let new_key = key.clone();
            let row = server.sessions.get_mut(&new_key).expect("row");
            crate::upsert_session_metadata(
                &mut row.metadata,
                "Cwd",
                "/home/user/proj".to_string(),
            );
        }
        let anchor_key = server
            .opencode_anchor_key(&empty_screen_live())
            .expect("an anchor exists");
        let other = if anchor_key == dead { live.clone() } else { dead.clone() };
        let viewed = |id: &str, viewed: u128| OpencodeServiceSession {
            id: id.to_string(),
            title: Some(format!("Tab {id}")),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: viewed,
            running: true,
        };
        let sessions = vec![
            viewed("ses_a0000000000000000000000001", 100),
            viewed("ses_b0000000000000000000000002", 200),
        ];
        // The non-anchor TUI's window names ITS session — ses_a — while the
        // service's focus (most-recently-viewed) is ses_b. Only the anchor
        // follows focus; the other row follows its own title.
        let mut titles = empty_titles();
        titles.insert(
            other.clone(),
            "OC | Tab ses_a0000000000000000000000001".to_string(),
        );
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &titles);
        let row = server.sessions.get(&other).expect("row survives");
        assert_eq!(
            row.id, "ses_a0000000000000000000000001",
            "the non-anchor row binds the session its own window title names"
        );
        assert!(
            row.launch_command.contains("ses_a0000000000000000000000001"),
            "the resume command must name the title-bound session"
        );
        let anchor_row = server.sessions.get(&anchor_key).expect("anchor row");
        assert_ne!(
            anchor_row.id, "ses_a0000000000000000000000001",
            "the title plane must never override the anchor"
        );
    }

    #[test]
    fn an_ambiguous_or_prefixless_window_title_binds_nothing() {
        let (mut server, dead, live) = server_with_two_anchors();
        let viewed = |id: &str, viewed: u128| OpencodeServiceSession {
            id: id.to_string(),
            title: Some("Same".to_string()),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: viewed,
            running: true,
        };
        let sessions = vec![viewed("ses_a0000000000000000000000001", 100), viewed("ses_b0000000000000000000000002", 200)];
        let mut titles = empty_titles();
        titles.insert(live.clone(), "OC | Same".to_string());
        titles.insert(dead.clone(), "OpenCode".to_string());
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &titles);
        for key in [&dead, &live] {
            let row = server.sessions.get(key).expect("row survives");
            assert_ne!(
                row.id,
                "ses_a0000000000000000000000001",
                "ambiguous or prefixless titles are not identity"
            );
        }
    }

    #[test]
    fn a_dead_row_never_binds_from_its_window_title() {
        let (mut server, dead, live) = server_with_two_anchors();
        let viewed = |id: &str, viewed: u128| OpencodeServiceSession {
            id: id.to_string(),
            title: Some(format!("Tab {id}")),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 0,
            viewed_epoch_ms: viewed,
            running: true,
        };
        let sessions = vec![viewed("ses_a0000000000000000000000001", 100)];
        let mut titles = empty_titles();
        titles.insert(
            dead.clone(),
            "OC | Tab ses_a0000000000000000000000001".to_string(),
        );
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &titles);
        let row = server.sessions.get(&dead).expect("row survives");
        assert_ne!(
            row.id, "ses_a0000000000000000000000001",
            "a dead row renders nothing; its title is a ghost's word"
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
        let sessions = vec![
            viewed("ses_a0000000000000000000000001", 100),
            viewed("ses_b0000000000000000000000002", 200),
        ];
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &empty_titles());
        let anchor_key = server
            .opencode_anchor_key(&empty_screen_live())
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
        server.apply_opencode_tab_mirror(
            &vec![viewed("ses_a0000000000000000000000001", 100)],
            &empty_screen_live(),
            &empty_titles(),
        );
        let with_tabs = {
            let anchor_key = server.opencode_anchor_key(&empty_screen_live()).expect("anchor");
            server
                .sessions
                .get(&anchor_key)
                .expect("anchor row")
                .metadata
                .iter()
                .any(|m| m.label == VIEWING_SESSION_METADATA)
        };
        assert!(with_tabs, "a viewed tab stamps the viewing entry");
        // The service's working set goes quiet (no tabs anywhere): a stale
        // "Viewing …" would now be a lie about the present.
        server.apply_opencode_tab_mirror(&[], &empty_screen_live(), &empty_titles());
        let anchor_key = server.opencode_anchor_key(&empty_screen_live()).expect("anchor");
        let anchor = server.sessions.get(&anchor_key).expect("anchor row");
        assert!(
            !anchor
                .metadata
                .iter()
                .any(|m| m.label == VIEWING_SESSION_METADATA),
            "no viewed tab anywhere must not leave a frozen 'Viewing' claim",
        );
    }

    /// [11.6.3-a]'s felt half: with the viewed stream dormant (beta-19271
    /// writes no viewed times), the anchor's viewing truth comes from its own
    /// OSC window title — the one per-session surface the TUI writes. The
    /// same idle tick under the old code answered `no_viewing` and left the
    /// anchor's identity on its birth uuid (the phantom-resume class).
    #[test]
    fn the_anchor_reads_viewing_from_its_own_window_title_when_the_stream_is_dormant() {
        let (mut server, _dead, _live) = server_with_two_anchors();
        // The real plane's rows carry Cwd; the rebind's birth-command rebuild
        // reads it to recompose the resume.
        for row in server.sessions.values_mut() {
            crate::upsert_session_metadata(
                &mut row.metadata,
                "Cwd",
                "/home/user/proj".to_string(),
            );
        }
        // Every listed session is quiet: no viewed times, nothing working.
        let idle = |id: &str| OpencodeServiceSession {
            id: id.to_string(),
            title: Some(format!("Tab {id}")),
            directory: Some("/home/user/proj".to_string()),
            updated_epoch_ms: 1_000,
            viewed_epoch_ms: 0,
            running: false,
        };
        let sessions = vec![
            idle("ses_a0000000000000000000000001"),
            idle("ses_b0000000000000000000000002"),
        ];
        let anchor_key = server
            .opencode_anchor_key(&empty_screen_live())
            .expect("an anchor exists");
        // The anchor TUI's window names ses_b — the session it renders.
        let mut titles = empty_titles();
        titles.insert(
            anchor_key.clone(),
            "OC | Tab ses_b0000000000000000000000002".to_string(),
        );
        server.apply_opencode_tab_mirror(&sessions, &empty_screen_live(), &titles);
        let anchor = server.sessions.get(&anchor_key).expect("anchor row");
        assert_eq!(
            anchor.id, "ses_b0000000000000000000000002",
            "identity follows the session the window title names"
        );
        let viewing = anchor
            .metadata
            .iter()
            .find(|m| m.label == VIEWING_SESSION_METADATA)
            .map(|m| m.value.clone());
        assert_eq!(
            viewing.as_deref(),
            Some("ses_b0000000000000000000000002"),
            "the Viewing stamp survives an idle fleet through the title plane"
        );
        assert!(
            anchor.launch_command.contains("ses_b0000000000000000000000002"),
            "the resume command names the title-bound session"
        );
    }
}
