//! `server map` — the LIVE daemon attach map, in one verb.
//!
//! ⛔ **THE QUESTION THIS ANSWERS WAS HAND-ASSEMBLED EVERY SESSION, WRONG MOST
//! TIMES.** Every trace-fixing session opens with the same ritual: `ps` for
//! daemons, `readlink /proc/<pid>/exe` for what they run, an `ls` of
//! `client-instances/` for who is attached, the hot-update owners file for what
//! is preserved, a dial per versioned socket to see which one answers — and
//! each hand-rolled cut answered a different subset. The owner's words when
//! asking for this verb: daemon attach issues, multiple sockets, hot restarts.
//! Those issues all live in the gaps BETWEEN the hand-assembled reads: a client
//! whose daemon died under it, a holder whose pid is a corpse, a socket that
//! answers but slowly, a deploy the running daemon never rotated onto. One read
//! must see them together.
//!
//! The map is a HOST fact, like the census it sits beside
//! ([`crate::daemon::daemon_census`]): it reads the home dir, the process
//! table, and dialled sockets — it never asks one daemon to describe its
//! siblings, because the daemon that would answer is itself one of the rows.
//!
//! ## The three states, and what each one costs
//!
//! Every row carries exactly one of `ok` / `warning` / `failed`. The states are
//! a DIAGNOSIS, not a decoration:
//!
//! - **ok** — probed and healthy on every axis this map knows.
//! - **warning** — alive but degraded, with the reason named: a slow answer, a
//!   binary replaced on disk under a live process, a live client whose daemon
//!   endpoint answers nothing (the hot-restart attach wound), a holder whose
//!   pid is gone but whose record is fresh enough that the registry itself
//!   would keep it. A warning is a thing to watch, not to kill.
//! - **failed** — provably broken: a stale client record, an orphaned holder,
//!   a daemon process that answers nothing, a remote machine offline at last
//!   refresh. A failed row is where a fix session starts.
//!
//! Multiple findings on one row merge to the WORST state, keeping every reason.
//!
//! ## The probes are the point
//!
//! This verb is the dynamic-tracing doctrine built into an instrument: it dials
//! every versioned socket itself with a bounded budget and times the answer,
//! checks every recorded pid against the process table, and reads the machine
//! of a remote row off the row's own path
//! ([`yggterm_core::agent_scheme::remote_row_machine_key`]). A file that says
//! "attached" is a claim; the probe is the verification. And so the run is
//! itself traceable: every non-ok row is emitted onto the event trace
//! (component `server-map`), throttled, so the next defect hunt can read what
//! this map saw.

use anyhow::Context as _;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use yggterm_core::agent_scheme::remote_row_machine_key;
use yggterm_core::append_trace_event;

use crate::daemon::daemon_census;
use crate::daemon::default_endpoint;
use crate::daemon::persisted_remote_machines_for_map;
use crate::daemon::preserved_owner_census_rows;
use crate::daemon::stale_daemon_answer_warning;
use crate::daemon::status_with_io_timeout;
use crate::daemon::versioned_status_probe_endpoints;
use crate::daemon::DaemonNameVerdict;
use crate::daemon::HostDaemonCensus;
use crate::daemon::PeerDaemonSummary;
use crate::daemon::ServerEndpoint;
use crate::daemon::ServerRuntimeStatus;
use crate::ClientInstanceRecord;
use crate::RemoteMachineHealth;

/// How long the map waits for ONE daemon's status answer before the row reads
/// failed. ⛔ Bounded far below the general request timeout on purpose: a map
/// that takes half a minute per wedged daemon is a map nobody runs while the
/// stack is on fire, which is exactly when it is needed. A daemon that misses a
/// 2 s budget for a status it normally answers in single-digit milliseconds is
/// failed for the purposes of this map no matter what a longer wait would say.
pub const MAP_PROBE_BUDGET_MS: u64 = 2_000;

/// A status answer past this is WARNING, not ok. The accept-loop floor was
/// measured at 150–230 µs and a quiet status build costs single-digit
/// milliseconds, so a quarter second is a thousand-fold degradation — visible,
/// and still far from "the daemon is gone".
pub const SLOW_ANSWER_MS: u64 = 250;

/// The registry itself keeps a dead-pid holder entry for this window before it
/// calls the entry a true orphan (see the prune in
/// `PreservedTerminalOwnerRegistry::load`). The map uses the SAME window so the
/// two instruments can never disagree about the boundary: inside it a dead pid
/// is a warning ("the registry would keep this — it may be a race"), past it a
/// failure.
pub const PRESERVED_OWNER_RECENT_MS: u64 = 5 * 60 * 1000;

/// At most this many flagged rows are emitted onto the event trace per run, so
/// a graveyard (measured once at 800+ entries) cannot flood the trace. The run
/// event carries the true counts; the cap only bounds the per-row echo.
const MAX_FLAGGED_TRACE_EVENTS: usize = 20;

// ---------------------------------------------------------------------------
// The states
// ---------------------------------------------------------------------------

/// One row's verdict. `warning` and `failed` always carry the reason — a state
/// without its reason is a mystery the reader has to re-derive, and the whole
/// point of this map is that the derivation already happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MapState {
    Ok,
    Warning { reason: String },
    Failed { reason: String },
}

impl MapState {
    pub fn ok() -> Self {
        MapState::Ok
    }

    pub fn warning(reason: impl Into<String>) -> Self {
        MapState::Warning {
            reason: reason.into(),
        }
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        MapState::Failed {
            reason: reason.into(),
        }
    }

    /// Worst wins: failed > warning > ok, and equal states join their reasons
    /// so a row with two findings still reads both.
    pub fn merge(self, other: MapState) -> MapState {
        use MapState::*;
        let join = |a: &str, b: &str| -> String {
            if a.is_empty() {
                b.to_string()
            } else if b.is_empty() || b == a {
                a.to_string()
            } else {
                format!("{a}; {b}")
            }
        };
        match (self, other) {
            (Ok, any) => any,
            (any, Ok) => any,
            (Failed { reason }, Warning { .. }) => Failed { reason },
            (Warning { .. }, Failed { reason }) => Failed { reason },
            (Warning { reason: a }, Warning { reason: b }) => Warning {
                reason: join(&a, &b),
            },
            (Failed { reason: a }, Failed { reason: b }) => Failed {
                reason: join(&a, &b),
            },
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MapState::Ok => "ok",
            MapState::Warning { .. } => "warning",
            MapState::Failed { .. } => "failed",
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            MapState::Ok => None,
            MapState::Warning { reason } | MapState::Failed { reason } => Some(reason),
        }
    }
}

// ---------------------------------------------------------------------------
// The rows
// ---------------------------------------------------------------------------

/// One bound socket name of an unreachable daemon, flattened for the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundNameRow {
    pub path: String,
    pub verdict: String,
    pub is_request_socket: bool,
}

/// One daemon on this host: the reached ones carry their timed probe, the
/// stranded ones carry their bound-name verdicts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DaemonMapRow {
    pub pid: u32,
    pub version: String,
    pub build_commit: String,
    pub endpoint: String,
    /// Milliseconds the map's own status dial took. `None` on a stranded row
    /// (nothing answered) — never silently zero, because "instant" and "not
    /// asked" are different facts.
    pub answer_ms: Option<u64>,
    pub uptime_ms: u64,
    pub owned_terminal_session_count: usize,
    pub preserved_terminal_owner_count: usize,
    pub live_terminal_session_count: usize,
    pub stored_terminal_session_count: usize,
    pub hot_restart_pending: bool,
    pub hot_restart_blocker_count: usize,
    pub permanent_blocker_count: usize,
    /// This daemon's binary was replaced on disk under it — a retire candidate.
    pub exe_deleted: bool,
    pub is_default_endpoint: bool,
    /// Only on a FAILED (stranded) row: every socket name the process bound,
    /// with what became of each name.
    pub bound_names: Vec<BoundNameRow>,
    pub state: MapState,
}

/// One client-instance record: a process that CLAIMS to be attached to a
/// daemon endpoint. The claim is verified against the process table and
/// against the endpoint's own probe result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientMapRow {
    pub pid: u32,
    /// The slice-4 role, or `legacy` on a record that predates it (which reads
    /// as active everywhere else in the codebase — the map says which it saw).
    pub role: String,
    pub client_id: Option<String>,
    pub build_commit: Option<String>,
    pub display: Option<String>,
    /// The client-instances scope directory the record was read from — the
    /// endpoint the process registered under, as the filesystem spells it.
    pub scope: String,
    /// The daemon endpoint this scope resolves to among the sockets the map
    /// probed, when one matches.
    pub daemon_endpoint: Option<String>,
    /// That endpoint's state label, when matched. `None` = no probed socket
    /// matches this scope (an old home's leftover scope is not a failure of
    /// the record).
    pub endpoint_state: Option<String>,
    pub age_ms: u64,
    pub state: MapState,
}

/// One preserved terminal owner: an external holder process keeping a session's
/// runtime alive across daemon generations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreservedOwnerMapRow {
    pub runtime_key: String,
    /// The machine the runtime lives on, read off the runtime key's own path —
    /// `None` means this host.
    pub machine: Option<String>,
    pub endpoint: String,
    pub owner_server_pid: u32,
    pub owner_server_version: String,
    pub pid_alive: bool,
    pub age_ms: u64,
    pub state: MapState,
}

/// One remote machine this home knows about, from the persisted state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteMachineMapRow {
    pub machine_key: String,
    pub label: String,
    pub ssh_target: String,
    pub deploy_state: String,
    pub scanned_session_count: usize,
    pub state: MapState,
}

/// One live session row a reached daemon advertises, with the machine it runs
/// on. This is the "how is yggterm connected FROM WHICH MACHINE" answer at row
/// granularity: a local row says this host, a remote row names its machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionMapRow {
    pub daemon_pid: u32,
    pub key: String,
    pub id: String,
    pub title: String,
    pub kind: String,
    /// The ssh target the row names, or empty for a row on this host.
    pub ssh_target: String,
    pub keep_alive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct MapSummary {
    pub ok: usize,
    pub warning: usize,
    pub failed: usize,
}

impl MapSummary {
    fn count(&mut self, state: &MapState) {
        match state {
            MapState::Ok => self.ok += 1,
            MapState::Warning { .. } => self.warning += 1,
            MapState::Failed { .. } => self.failed += 1,
        }
    }
}

/// The whole map, as one JSON-serializable document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachMapReport {
    pub host: String,
    pub generated_at_ms: u64,
    pub probe_budget_ms: u64,
    /// The census's own coverage verdict — did the process-table read reach
    /// every daemon on this host? ⛔ An unreadable process table makes this
    /// "unverified", never "complete".
    pub coverage: String,
    pub daemon_processes_on_host: Option<usize>,
    pub daemons: Vec<DaemonMapRow>,
    pub clients: Vec<ClientMapRow>,
    pub preserved_owners: Vec<PreservedOwnerMapRow>,
    pub remote_machines: Vec<RemoteMachineMapRow>,
    pub sessions: Vec<SessionMapRow>,
    pub summary: MapSummary,
}

// ---------------------------------------------------------------------------
// The probe
// ---------------------------------------------------------------------------

/// One versioned socket the map dialled, with the answer or the refusal.
#[derive(Debug, Clone)]
pub(crate) struct ProbedDaemon {
    pub label: String,
    pub endpoint: ServerEndpoint,
    /// `Some((status, answer_ms))` when the dial was answered inside budget.
    pub answered: Option<(ServerRuntimeStatus, u64)>,
    /// The error text when nothing usable came back inside budget.
    pub unanswered_error: Option<String>,
}

/// Dial every versioned server socket this home holds, timing each answer.
///
/// ⛔ Budgeted per dial at [`MAP_PROBE_BUDGET_MS`], not at the general request
/// timeout: the map must finish while the stack it describes is still alive.
#[cfg(unix)]
pub(crate) fn probe_daemons(home_dir: &Path) -> Vec<ProbedDaemon> {
    versioned_status_probe_endpoints(home_dir)
        .into_iter()
        .map(|(endpoint, label)| {
            let started = Instant::now();
            let outcome =
                status_with_io_timeout(&endpoint, Duration::from_millis(MAP_PROBE_BUDGET_MS));
            let answer_ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok(status) => ProbedDaemon {
                    label,
                    endpoint,
                    answered: Some((status, answer_ms)),
                    unanswered_error: None,
                },
                Err(error) => ProbedDaemon {
                    label,
                    endpoint,
                    answered: None,
                    unanswered_error: Some(format!("{error:#}")),
                },
            }
        })
        .collect()
}

#[cfg(not(unix))]
pub(crate) fn probe_daemons(_home_dir: &Path) -> Vec<ProbedDaemon> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Classification — pure, so every rule here is testable without a daemon
// ---------------------------------------------------------------------------

/// The findings for one REACHED daemon, worst-merged.
///
/// `probe_answer_ms`/`probe_error` describe the map's own dial: an error with
/// no answer inside budget reads failed, a slow answer reads warning. The
/// remaining inputs are the other findings, already computed by the caller —
/// `None` means "no such finding".
pub(crate) fn classify_reached_daemon(
    probe_answer_ms: Option<u64>,
    probe_error: Option<&str>,
    exe_deleted: bool,
    owns_zero_while_peer_owns_more: Option<String>,
    older_than_installed: Option<String>,
) -> MapState {
    let mut state = MapState::ok();
    if let Some(error) = probe_error {
        state = state.merge(MapState::failed(format!(
            "no status answer within the map probe budget: {error}"
        )));
    } else if let Some(ms) = probe_answer_ms {
        if ms > SLOW_ANSWER_MS {
            state = state.merge(MapState::warning(format!(
                "slow status answer ({ms} ms — a healthy daemon answers in single-digit ms)"
            )));
        }
    }
    if exe_deleted {
        state = state.merge(MapState::warning(
            "binary replaced on disk under the live process — retire candidate",
        ));
    }
    if let Some(note) = owns_zero_while_peer_owns_more {
        state = state.merge(MapState::warning(note));
    }
    if let Some(note) = older_than_installed {
        state = state.merge(MapState::warning(note));
    }
    state
}

/// The findings for one STRANDED daemon: it is failed by definition — alive in
/// the process table, answering nothing — and the bound-name verdicts say WHY
/// a dial cannot reach it.
pub(crate) fn classify_stranded_daemon(bound_names: &[BoundNameRow]) -> MapState {
    let summary = if bound_names.is_empty() {
        "process alive but answered nothing; the kernel bind table could not say which names it holds"
            .to_string()
    } else {
        let names: Vec<String> = bound_names
            .iter()
            .map(|name| format!("{} [{}]", name.path, name.verdict))
            .collect();
        format!(
            "process alive but answered nothing; names it bound: {}",
            names.join(", ")
        )
    };
    MapState::failed(summary)
}

/// The findings for one client-instance record.
///
/// `endpoint_state` is the probed state of the daemon endpoint the record's
/// scope resolves to, when that socket is one the map probed. A LIVE client on
/// a FAILED endpoint is the exact signature of the attach wounds this map
/// exists to expose — the process thinks it is attached to a daemon nothing
/// answers — and it must read as a warning on the CLIENT row, not hide inside
/// the daemon section.
pub(crate) fn classify_client_record(
    record_parses: bool,
    pid_alive: bool,
    exe_deleted: bool,
    endpoint_state: Option<&MapState>,
) -> MapState {
    if !record_parses {
        return MapState::failed("record is unreadable as a client instance record");
    }
    if !pid_alive {
        return MapState::failed(
            "record is stale: the pid is gone (or the process was reused) — nobody is attached",
        );
    }
    let mut state = MapState::ok();
    if exe_deleted {
        state = state.merge(MapState::warning(
            "client binary replaced on disk under the live process",
        ));
    }
    if let Some(MapState::Failed { reason }) = endpoint_state {
        state = state.merge(MapState::warning(format!(
            "process is alive but its daemon endpoint answers nothing — the attach is a corpse: {reason}"
        )));
    }
    state
}

/// The findings for one preserved owner. The recent/dead boundary is the
/// registry's own — see [`PRESERVED_OWNER_RECENT_MS`].
pub(crate) fn classify_preserved_owner(
    pid_alive: bool,
    age_ms: u64,
    owner_version_reachable: bool,
) -> MapState {
    if pid_alive {
        if owner_version_reachable {
            MapState::ok()
        } else {
            MapState::warning(
                "holder is alive but claims a version no reachable daemon runs — its adopter may be gone",
            )
        }
    } else if age_ms < PRESERVED_OWNER_RECENT_MS {
        MapState::warning(format!(
            "holder pid is gone but the entry is recent ({}) — the registry itself would still keep it",
            human_duration(age_ms)
        ))
    } else {
        MapState::failed(format!(
            "orphaned holder: pid gone for {}, nobody can adopt this runtime",
            human_duration(age_ms)
        ))
    }
}

/// The persisted remote-machine health, mapped onto the map's states. `cached`
/// means the machine's view is stale — alive-but-degraded, a warning; `offline`
/// means the last refresh could not reach it at all.
pub(crate) fn classify_remote_machine(health: &RemoteMachineHealth) -> MapState {
    match health {
        RemoteMachineHealth::Healthy => MapState::ok(),
        RemoteMachineHealth::Cached => MapState::warning(
            "serving a cached view — the last refresh did not reach the machine",
        ),
        RemoteMachineHealth::Offline => MapState::failed("offline at last refresh"),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `3.2.70` → `(3, 2, 70)`. Anything else is `None` — a version the map cannot
/// parse is "cannot say", never a sighting of staleness.
pub(crate) fn parse_version_triple(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// The newest version still installed under `versions/` — the only versions a
/// rollback can return to, and the yardstick a reached daemon's version is
/// measured against. `None` = the directory is absent or unreadable, i.e.
/// "cannot say", which is why the daemon warning it feeds is optional.
pub(crate) fn newest_installed_version(home_dir: &Path) -> Option<(u64, u64, u64)> {
    let entries = std::fs::read_dir(home_dir.join("versions")).ok()?;
    let mut newest: Option<(u64, u64, u64)> = None;
    for entry in entries.flatten() {
        if let Some(triple) = parse_version_triple(&entry.file_name().to_string_lossy()) {
            if newest.map(|current| triple > current).unwrap_or(true) {
                newest = Some(triple);
            }
        }
    }
    newest
}

/// A duration a human reads at a glance.
pub(crate) fn human_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds < 90 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 90 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 36 {
        return format!("{hours}h{}m", minutes % 60);
    }
    format!("{}d{}h", hours / 24, hours % 24)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|raw| raw.trim().to_string())
        .ok()
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn bound_name_verdict_label(verdict: &DaemonNameVerdict) -> String {
    match verdict {
        DaemonNameVerdict::Diverted { now_reaches } => format!("diverted → {now_reaches}"),
        DaemonNameVerdict::Unlinked => "unlinked".to_string(),
        DaemonNameVerdict::Present => "present".to_string(),
        DaemonNameVerdict::Unknown => "unknown".to_string(),
    }
}

fn format_version(triple: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", triple.0, triple.1, triple.2)
}

// ---------------------------------------------------------------------------
// Gathering — the io halves, kept separate from the pure classification
// ---------------------------------------------------------------------------

/// Every client-instance record on the host, across ALL endpoint scopes,
/// including the stale and unreadable ones. ⛔ This deliberately does NOT reuse
/// `active_client_instance_records`: "active" filters to live pids, and a stale
/// record is exactly the failed row the map must show, not a row it may never
/// see.
#[cfg(unix)]
pub(crate) fn gather_client_rows(
    home_dir: &Path,
    scope_to_endpoint: &BTreeMap<String, (String, Option<MapState>)>,
) -> Vec<ClientMapRow> {
    let mut rows = Vec::new();
    let root = home_dir.join("client-instances");
    let Ok(scopes) = std::fs::read_dir(&root) else {
        return rows;
    };
    for scope in scopes.flatten() {
        if !scope.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let scope_name = scope.file_name().to_string_lossy().into_owned();
        let record_paths = match crate::client_instance_record_paths(&scope.path()) {
            Ok(paths) => paths,
            // ⛔ An unreadable scope is one row that SAYS so, never a silent
            // skip — the same lesson the record enumeration itself was taught
            // (absent is an answer; unreadable is "could not ask").
            Err(error) => {
                rows.push(ClientMapRow {
                    pid: 0,
                    role: "unreadable-scope".to_string(),
                    client_id: None,
                    build_commit: None,
                    display: None,
                    scope: scope_name,
                    daemon_endpoint: None,
                    endpoint_state: None,
                    age_ms: 0,
                    state: MapState::failed(format!(
                        "the scope directory could not be read: {error:#}"
                    )),
                });
                continue;
            }
        };
        for path in record_paths {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            let (daemon_endpoint, endpoint_state) = match scope_to_endpoint.get(&scope_name) {
                Some((label, state)) => (
                    Some(label.clone()),
                    Some(state.clone().unwrap_or(MapState::ok())),
                ),
                None => (None, None),
            };
            let parsed: Result<ClientInstanceRecord> = std::fs::read(&path)
                .with_context(|| format!("reading {}", path.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<ClientInstanceRecord>(&bytes)
                        .context("parsing client instance record")
                });
            let record = match parsed {
                Ok(record) => record,
                Err(error) => {
                    rows.push(ClientMapRow {
                        pid: 0,
                        role: "unreadable-record".to_string(),
                        client_id: file_name,
                        build_commit: None,
                        display: None,
                        scope: scope_name.clone(),
                        daemon_endpoint,
                        endpoint_state: endpoint_state.as_ref().map(|s| s.label().to_string()),
                        age_ms: 0,
                        state: MapState::failed(format!(
                            "record is unreadable as a client instance record: {error:#}"
                        )),
                    });
                    continue;
                }
            };
            let pid_alive = crate::client_instance_record_matches_live_process(&record);
            let exe_deleted = std::fs::read_link(format!("/proc/{}/exe", record.pid))
                .map(|link| link.to_string_lossy().ends_with(" (deleted)"))
                .unwrap_or(false);
            let age_ms = now_ms().saturating_sub(record.started_at_ms as u64);
            let state =
                classify_client_record(true, pid_alive, exe_deleted, endpoint_state.as_ref());
            rows.push(ClientMapRow {
                pid: record.pid,
                role: record
                    .client_role
                    .clone()
                    .unwrap_or_else(|| "legacy".to_string()),
                client_id: record.client_id.clone(),
                build_commit: record.build_commit.clone(),
                display: record.display.clone(),
                scope: scope_name.clone(),
                daemon_endpoint,
                endpoint_state: endpoint_state.as_ref().map(|s| s.label().to_string()),
                age_ms,
                state,
            });
        }
    }
    rows
}

#[cfg(not(unix))]
pub(crate) fn gather_client_rows(
    _home_dir: &Path,
    _scope_to_endpoint: &BTreeMap<String, (String, Option<MapState>)>,
) -> Vec<ClientMapRow> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Assembly — pure, given the gathered facts
// ---------------------------------------------------------------------------

/// Build the report from already-gathered facts. Pure: every state decision in
/// the map is reachable from this function in a test.
pub(crate) fn assemble_report(
    host: String,
    generated_at_ms: u64,
    census: HostDaemonCensus,
    probes: &[ProbedDaemon],
    clients: Vec<ClientMapRow>,
    owners: Vec<PreservedOwnerMapRow>,
    machines: Vec<RemoteMachineMapRow>,
    newest_installed: Option<(u64, u64, u64)>,
    default_label: String,
) -> AttachMapReport {
    let peer_summaries: Vec<PeerDaemonSummary> = probes
        .iter()
        .filter_map(|probe| {
            probe.answered.as_ref().map(|(status, _)| PeerDaemonSummary {
                pid: status.server_pid,
                version: status.server_version.clone(),
                owned_terminal_session_count: status.owned_terminal_session_count,
            })
        })
        .collect();

    let mut sessions = Vec::new();
    let mut daemons = Vec::new();

    for probe in probes {
        let Some((status, answer_ms)) = probe.answered.as_ref() else {
            continue;
        };
        // exe-deleted is a process-table fact the census already gathered for
        // its own rows; match on pid rather than re-reading /proc here.
        let exe_deleted = census
            .reached
            .iter()
            .find(|row| row.pid == status.server_pid)
            .map(|row| row.exe_deleted)
            .unwrap_or(false);
        let owns_zero = stale_daemon_answer_warning(
            status.server_pid,
            status.owned_terminal_session_count,
            &peer_summaries,
        );
        let older_than_installed =
            newest_installed.and_then(|newest| match parse_version_triple(&status.server_version)
            {
                Some(mine) if mine >= newest => None,
                Some(_) => Some(format!(
                    "older than the newest installed version ({}) — a deploy has landed that this daemon has not rotated onto",
                    format_version(newest)
                )),
                // An unparseable running version is not evidence of staleness.
                None => None,
            });
        let state = classify_reached_daemon(
            Some(*answer_ms),
            None,
            exe_deleted,
            owns_zero,
            older_than_installed,
        );
        if status.advertises_live_session_rows {
            for live in &status.live_terminal_sessions {
                sessions.push(SessionMapRow {
                    daemon_pid: status.server_pid,
                    key: live.key.clone(),
                    id: live.id.clone(),
                    title: live.title.clone(),
                    kind: format!("{:?}", live.kind),
                    ssh_target: live.ssh_target.clone(),
                    keep_alive: live.keep_alive,
                });
            }
        }
        daemons.push(DaemonMapRow {
            pid: status.server_pid,
            version: status.server_version.clone(),
            build_commit: status.server_build_commit.clone(),
            endpoint: probe.label.clone(),
            answer_ms: Some(*answer_ms),
            uptime_ms: status.daemon_uptime_ms,
            owned_terminal_session_count: status.owned_terminal_session_count,
            preserved_terminal_owner_count: status.preserved_terminal_owner_count,
            live_terminal_session_count: status.live_terminal_sessions.len(),
            stored_terminal_session_count: status.stored_terminal_session_count,
            hot_restart_pending: status.hot_restart_pending,
            hot_restart_blocker_count: status.hot_restart_blockers.len(),
            permanent_blocker_count: status
                .hot_restart_blockers
                .iter()
                .filter(|blocker| blocker.permanent)
                .count(),
            exe_deleted,
            is_default_endpoint: probe.label == default_label,
            bound_names: Vec::new(),
            state,
        });
    }

    // Stranded daemons: in the process table, answering nothing.
    for stranded in &census.stranded {
        let bound_names: Vec<BoundNameRow> = stranded
            .bound_names
            .iter()
            .map(|name| BoundNameRow {
                path: name.path.clone(),
                verdict: bound_name_verdict_label(&name.verdict),
                is_request_socket: name.is_request_socket,
            })
            .collect();
        let state = classify_stranded_daemon(&bound_names);
        daemons.push(DaemonMapRow {
            pid: stranded.pid,
            version: "<unanswered>".to_string(),
            build_commit: String::new(),
            endpoint: String::new(),
            answer_ms: None,
            uptime_ms: stranded.uptime_ms.unwrap_or(0),
            owned_terminal_session_count: 0,
            preserved_terminal_owner_count: 0,
            live_terminal_session_count: 0,
            stored_terminal_session_count: 0,
            hot_restart_pending: false,
            hot_restart_blocker_count: 0,
            permanent_blocker_count: 0,
            exe_deleted: stranded.exe_deleted,
            is_default_endpoint: false,
            bound_names,
            state,
        });
    }

    // Oldest first, the census's own ordering reason: a map is read to find
    // what has been lingering longest.
    daemons.sort_by(|left, right| {
        right
            .uptime_ms
            .cmp(&left.uptime_ms)
            .then(left.pid.cmp(&right.pid))
    });

    let mut summary = MapSummary::default();
    for daemon in &daemons {
        summary.count(&daemon.state);
    }
    for client in &clients {
        summary.count(&client.state);
    }
    for owner in &owners {
        summary.count(&owner.state);
    }
    for machine in &machines {
        summary.count(&machine.state);
    }

    AttachMapReport {
        host,
        generated_at_ms,
        probe_budget_ms: MAP_PROBE_BUDGET_MS,
        coverage: census.coverage().to_string(),
        daemon_processes_on_host: census.daemon_processes_on_host,
        daemons,
        clients,
        preserved_owners: owners,
        remote_machines: machines,
        sessions,
        summary,
    }
}

// ---------------------------------------------------------------------------
// The verb
// ---------------------------------------------------------------------------

/// `server map` — print (or JSON) the live daemon attach map, and trace every
/// flagged row onto the event trace so the next defect hunt sees what this run
/// saw.
pub fn run_server_attach_map(home_dir: &Path, json: bool) -> Result<()> {
    let generated_at_ms = now_ms();
    let host = hostname();
    let census = daemon_census(home_dir);
    let probes = probe_daemons(home_dir);

    // Map the probed endpoints to their client-instance scope spellings once,
    // so a client record's claimed endpoint can be checked against what the
    // socket actually answered. Only the dial facts classify here — the
    // daemon-level findings (exe-deleted, owns-zero) belong to the daemon rows,
    // not to every client that claims the endpoint.
    let mut scope_to_endpoint: BTreeMap<String, (String, Option<MapState>)> = BTreeMap::new();
    for probe in &probes {
        let scope = crate::client_instance_scope_label(&probe.endpoint);
        let state = match probe.answered.as_ref() {
            Some((_, answer_ms)) => {
                classify_reached_daemon(Some(*answer_ms), None, false, None, None)
            }
            None => MapState::failed(
                probe
                    .unanswered_error
                    .clone()
                    .unwrap_or_else(|| "no answer".to_string()),
            ),
        };
        scope_to_endpoint.insert(scope, (probe.label.clone(), Some(state)));
    }

    let clients = gather_client_rows(home_dir, &scope_to_endpoint);

    let reachable_versions: Vec<String> = probes
        .iter()
        .filter_map(|probe| {
            probe
                .answered
                .as_ref()
                .map(|(status, _)| status.server_version.clone())
        })
        .collect();
    let owners = preserved_owner_census_rows(home_dir)
        .into_iter()
        .map(|row| {
            let age_ms = generated_at_ms.saturating_sub(row.created_at_ms);
            let owner_version_reachable = reachable_versions
                .iter()
                .any(|version| *version == row.owner_server_version);
            let state =
                classify_preserved_owner(row.pid_alive, age_ms, owner_version_reachable);
            let machine = remote_row_machine_key(&row.runtime_key).map(|raw| raw.to_string());
            PreservedOwnerMapRow {
                runtime_key: row.runtime_key,
                machine,
                endpoint: row.endpoint,
                owner_server_pid: row.owner_server_pid,
                owner_server_version: row.owner_server_version,
                pid_alive: row.pid_alive,
                age_ms,
                state,
            }
        })
        .collect();

    // An unreadable persisted state is ONE row that says why, never an empty
    // list that reads "none".
    let machines: Vec<RemoteMachineMapRow> = match persisted_remote_machines_for_map(home_dir) {
        Ok(snapshots) => snapshots
            .into_iter()
            .map(|snapshot| {
                let state = classify_remote_machine(&snapshot.health);
                RemoteMachineMapRow {
                    machine_key: snapshot.machine_key,
                    label: snapshot.label,
                    ssh_target: snapshot.ssh_target,
                    deploy_state: format!("{:?}", snapshot.remote_deploy_state),
                    scanned_session_count: snapshot.sessions.len(),
                    state,
                }
            })
            .collect(),
        Err(error) => vec![RemoteMachineMapRow {
            machine_key: "<unreadable>".to_string(),
            label: String::new(),
            ssh_target: String::new(),
            deploy_state: String::new(),
            scanned_session_count: 0,
            state: MapState::failed(format!(
                "the persisted remote machines could not be read: {error:#}"
            )),
        }],
    };

    let default_label = crate::daemon::owner_endpoint_label_for_map(&default_endpoint(home_dir));

    let report = assemble_report(
        host.clone(),
        generated_at_ms,
        census,
        &probes,
        clients,
        owners,
        machines,
        newest_installed_version(home_dir),
        default_label,
    );

    emit_map_trace_events(home_dir, &report);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("serializing the attach map")?
        );
    } else {
        print!("{}", format_attach_map(&report));
    }
    Ok(())
}

/// The run lands on the event trace (component `server-map`), plus one throttled
/// echo per flagged row. ⛔ The true counts live on the run event even when the
/// per-row echo is capped — the cap bounds noise, never the truth.
fn emit_map_trace_events(home_dir: &Path, report: &AttachMapReport) {
    append_trace_event(
        home_dir,
        "server-map",
        "map",
        "run",
        serde_json::json!({
            "host": report.host,
            "coverage": report.coverage,
            "daemons": report.daemons.len(),
            "clients": report.clients.len(),
            "preserved_owners": report.preserved_owners.len(),
            "remote_machines": report.remote_machines.len(),
            "sessions": report.sessions.len(),
            "ok": report.summary.ok,
            "warnings": report.summary.warning,
            "failures": report.summary.failed,
        }),
    );
    let mut flagged: Vec<serde_json::Value> = Vec::new();
    for daemon in &report.daemons {
        if let Some(reason) = daemon.state.reason() {
            flagged.push(serde_json::json!({
                "section": "daemon", "identity": format!("pid {}", daemon.pid),
                "state": daemon.state.label(), "reason": reason,
            }));
        }
    }
    for client in &report.clients {
        if let Some(reason) = client.state.reason() {
            flagged.push(serde_json::json!({
                "section": "client", "identity": format!("pid {}", client.pid),
                "state": client.state.label(), "reason": reason,
            }));
        }
    }
    for owner in &report.preserved_owners {
        if let Some(reason) = owner.state.reason() {
            flagged.push(serde_json::json!({
                "section": "preserved_owner", "identity": owner.runtime_key,
                "state": owner.state.label(), "reason": reason,
            }));
        }
    }
    for machine in &report.remote_machines {
        if let Some(reason) = machine.state.reason() {
            flagged.push(serde_json::json!({
                "section": "remote_machine", "identity": machine.machine_key,
                "state": machine.state.label(), "reason": reason,
            }));
        }
    }
    let suppressed = flagged.len().saturating_sub(MAX_FLAGGED_TRACE_EVENTS);
    for row in flagged.into_iter().take(MAX_FLAGGED_TRACE_EVENTS) {
        append_trace_event(home_dir, "server-map", "map", "flagged", row);
    }
    if suppressed > 0 {
        append_trace_event(
            home_dir,
            "server-map",
            "map",
            "flagged_suppressed",
            serde_json::json!({ "suppressed": suppressed }),
        );
    }
}

// ---------------------------------------------------------------------------
// The human report
// ---------------------------------------------------------------------------

/// The text a human reads. Pure, so the wording is testable and cannot drift
/// from the data.
pub fn format_attach_map(report: &AttachMapReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "THE DAEMON ATTACH MAP — host {} · probe budget {} ms · coverage: {}\n",
        report.host, report.probe_budget_ms, report.coverage
    ));
    if let Some(total) = report.daemon_processes_on_host {
        out.push_str(&format!("daemon processes on this host: {total}\n"));
    }

    out.push_str(&format!("\nDAEMONS ({})\n", plural(report.daemons.len(), "row", "rows")));
    if report.daemons.is_empty() {
        out.push_str("  (none reachable and none stranded — either a quiet host or a home that never held one)\n");
    }
    for daemon in &report.daemons {
        out.push_str(&format_daemon_row(daemon));
    }

    out.push_str(&format!(
        "\nCLIENTS OF RECORD ({}) — a record is a CLAIM of attachment; the probe verified it\n",
        plural(report.clients.len(), "record", "records")
    ));
    if report.clients.is_empty() {
        out.push_str("  (no client-instance records under this home)\n");
    }
    for client in &report.clients {
        let id = client
            .client_id
            .clone()
            .unwrap_or_else(|| "(anonymous)".to_string());
        let endpoint = client
            .daemon_endpoint
            .clone()
            .unwrap_or_else(|| format!("scope {}", client.scope));
        let endpoint_note = match client.endpoint_state.as_deref() {
            Some("failed") => " · that endpoint: FAILED".to_string(),
            Some("warning") => " · that endpoint: warning".to_string(),
            _ => String::new(),
        };
        out.push_str(&format!(
            "  {:<8} pid {:<7} {:<9} {:<18} → {:<34} age {}{}{}\n",
            client.state.label(),
            client.pid,
            client.role,
            id,
            endpoint,
            human_duration(client.age_ms),
            endpoint_note,
            reason_suffix(&client.state),
        ));
    }

    out.push_str(&format!(
        "\nPRESERVED OWNERS ({}) — external holders keeping runtimes alive across daemon generations\n",
        plural(report.preserved_owners.len(), "entry", "entries")
    ));
    if report.preserved_owners.is_empty() {
        out.push_str("  (no preserved-owner entries — nothing is being kept alive by a holder)\n");
    }
    for owner in &report.preserved_owners {
        let machine = owner
            .machine
            .clone()
            .unwrap_or_else(|| "this host".to_string());
        out.push_str(&format!(
            "  {:<8} pid {:<7} v{:<9} holder of {} (machine: {}) · age {}{}\n",
            owner.state.label(),
            owner.owner_server_pid,
            owner.owner_server_version,
            owner.runtime_key,
            machine,
            human_duration(owner.age_ms),
            reason_suffix(&owner.state),
        ));
    }

    out.push_str(&format!(
        "\nREMOTE MACHINES ({})\n",
        plural(report.remote_machines.len(), "machine", "machines")
    ));
    if report.remote_machines.is_empty() {
        out.push_str("  (this home knows no remote machines)\n");
    }
    for machine in &report.remote_machines {
        let name = if machine.label.is_empty() {
            machine.machine_key.clone()
        } else {
            format!("{} ({})", machine.label, machine.machine_key)
        };
        out.push_str(&format!(
            "  {:<8} {} · ssh {} · deploy {} · sessions {}{}\n",
            machine.state.label(),
            name,
            machine.ssh_target,
            machine.deploy_state,
            machine.scanned_session_count,
            reason_suffix(&machine.state),
        ));
    }

    out.push_str(&format!(
        "\nLIVE SESSIONS ({}) — as the reached daemons advertise them\n",
        plural(report.sessions.len(), "row", "rows")
    ));
    if report.sessions.is_empty() {
        out.push_str("  (no live session rows advertised by any reachable daemon)\n");
    }
    for session in &report.sessions {
        let machine = if session.ssh_target.is_empty() {
            "this host".to_string()
        } else {
            session.ssh_target.clone()
        };
        out.push_str(&format!(
            "  pid {:<7} {} [{}] · {} · keep_alive {}\n",
            session.daemon_pid, session.key, session.kind, machine, session.keep_alive,
        ));
    }

    out.push_str(&format!(
        "\nSUMMARY: {} ok · {} warning · {} failed — every non-ok row above is on the event trace (component server-map)\n",
        report.summary.ok, report.summary.warning, report.summary.failed
    ));
    out
}

fn reason_suffix(state: &MapState) -> String {
    match state.reason() {
        Some(reason) => format!(" · {reason}"),
        None => String::new(),
    }
}

fn format_daemon_row(daemon: &DaemonMapRow) -> String {
    let mut line = if daemon.answer_ms.is_some() {
        format!(
            "  {:<8} pid {:<7} v{:<9} build {:<10} {} · answered in {}ms · up {} · owns {} · preserved {} · live {} · stored {}",
            daemon.state.label(),
            daemon.pid,
            daemon.version,
            if daemon.build_commit.is_empty() {
                "(pre-field)"
            } else {
                &daemon.build_commit
            },
            daemon.endpoint,
            daemon.answer_ms.unwrap_or(0),
            human_duration(daemon.uptime_ms),
            daemon.owned_terminal_session_count,
            daemon.preserved_terminal_owner_count,
            daemon.live_terminal_session_count,
            daemon.stored_terminal_session_count,
        )
    } else {
        format!(
            "  {:<8} pid {:<7} · up {} · counts unavailable when nothing answers",
            daemon.state.label(),
            daemon.pid,
            human_duration(daemon.uptime_ms),
        )
    };
    if daemon.is_default_endpoint {
        line.push_str(" · default");
    }
    if daemon.exe_deleted {
        line.push_str(" · exe-deleted");
    }
    if daemon.hot_restart_pending {
        line.push_str(&format!(
            " · hot-restart pending ({} blocker(s), {} permanent)",
            daemon.hot_restart_blocker_count, daemon.permanent_blocker_count
        ));
    }
    line.push_str(&reason_suffix(&daemon.state));
    line.push('\n');
    for name in &daemon.bound_names {
        line.push_str(&format!(
            "           bound name {} [{}]{}\n",
            name.path,
            name.verdict,
            if name.is_request_socket {
                " · REQUEST socket"
            } else {
                ""
            },
        ));
    }
    line
}

fn plural(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

// ---------------------------------------------------------------------------
// Tests — the classification rules, pure and total
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_warnings_merge_and_keep_both_reasons() {
        let state = classify_reached_daemon(Some(400), None, true, None, None);
        match &state {
            MapState::Warning { reason } => {
                assert!(reason.contains("400 ms"), "{reason}");
                assert!(reason.contains("replaced on disk"), "{reason}");
            }
            other => panic!("expected warning, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_finding_beats_a_warning_and_names_itself() {
        let state = classify_reached_daemon(Some(5), Some("connection refused"), true, None, None);
        match &state {
            MapState::Failed { reason } => {
                assert!(reason.contains("probe budget"), "{reason}");
                assert!(reason.contains("connection refused"), "{reason}");
            }
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[test]
    fn a_fast_clean_answer_is_ok() {
        assert_eq!(
            classify_reached_daemon(Some(3), None, false, None, None),
            MapState::Ok
        );
    }

    #[test]
    fn a_slow_answer_is_a_warning_not_a_failure() {
        let state = classify_reached_daemon(Some(900), None, false, None, None);
        match &state {
            MapState::Warning { reason } => assert!(reason.contains("900 ms"), "{reason}"),
            other => panic!("expected warning, got {other:?}"),
        }
    }

    #[test]
    fn a_stranded_daemon_names_its_bound_names_and_their_verdicts() {
        let state = classify_stranded_daemon(&[BoundNameRow {
            path: "/home/x/.yggterm/server-3-1-12.sock".to_string(),
            verdict: "diverted → /home/x/.yggterm/server-3-2-0.sock".to_string(),
            is_request_socket: true,
        }]);
        match &state {
            MapState::Failed { reason } => {
                assert!(reason.contains("server-3-1-12.sock"), "{reason}");
                assert!(reason.contains("diverted"), "{reason}");
            }
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[test]
    fn a_stranded_daemon_without_names_says_the_bind_table_could_not_say() {
        match classify_stranded_daemon(&[]) {
            MapState::Failed { reason } => assert!(reason.contains("could not say"), "{reason}"),
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[test]
    fn a_dead_pid_is_a_failed_client_record_even_on_a_healthy_endpoint() {
        assert!(matches!(
            classify_client_record(true, false, false, Some(&MapState::ok())),
            MapState::Failed { .. }
        ));
    }

    #[test]
    fn a_live_client_on_a_failed_endpoint_is_a_warning_that_names_the_corpse() {
        let state = classify_client_record(
            true,
            true,
            false,
            Some(&MapState::failed("no answer within budget")),
        );
        match &state {
            MapState::Warning { reason } => {
                assert!(reason.contains("corpse"), "{reason}");
                assert!(reason.contains("no answer within budget"), "{reason}");
            }
            other => panic!("expected warning, got {other:?}"),
        }
    }

    #[test]
    fn an_unmatched_endpoint_scope_does_not_fail_a_live_record() {
        assert_eq!(
            classify_client_record(true, true, false, None),
            MapState::Ok,
            "an old home's leftover scope is not a failure of the record"
        );
    }

    #[test]
    fn an_unreadable_record_is_failed_by_definition() {
        assert!(matches!(
            classify_client_record(false, false, false, None),
            MapState::Failed { .. }
        ));
    }

    #[test]
    fn an_old_dead_preserved_owner_is_failed_and_a_recent_one_is_a_warning() {
        let old = classify_preserved_owner(false, 6 * 60 * 1000, true);
        let recent = classify_preserved_owner(false, 30 * 1000, true);
        assert!(matches!(old, MapState::Failed { .. }), "got {old:?}");
        assert!(matches!(recent, MapState::Warning { .. }), "got {recent:?}");
    }

    #[test]
    fn a_live_holder_on_an_unreachable_version_is_a_warning() {
        let state = classify_preserved_owner(true, 1000, false);
        assert!(matches!(state, MapState::Warning { .. }), "got {state:?}");
    }

    #[test]
    fn remote_health_maps_cached_to_warning_and_offline_to_failure() {
        assert_eq!(
            classify_remote_machine(&RemoteMachineHealth::Healthy),
            MapState::Ok
        );
        assert!(matches!(
            classify_remote_machine(&RemoteMachineHealth::Cached),
            MapState::Warning { .. }
        ));
        assert!(matches!(
            classify_remote_machine(&RemoteMachineHealth::Offline),
            MapState::Failed { .. }
        ));
    }

    #[test]
    fn version_triples_parse_and_refuse_ambiguity() {
        assert_eq!(parse_version_triple("3.2.70"), Some((3, 2, 70)));
        assert_eq!(parse_version_triple(" 3.2.70 "), Some((3, 2, 70)));
        assert_eq!(parse_version_triple("3.2"), None);
        assert_eq!(parse_version_triple("3.2.70.1"), None);
        assert_eq!(parse_version_triple("three"), None);
    }

    #[test]
    fn merge_keeps_the_worse_state() {
        let merged = MapState::warning("a").merge(MapState::failed("b"));
        match &merged {
            MapState::Failed { reason } => assert_eq!(reason, "b"),
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[test]
    fn the_human_report_names_every_section_and_the_summary() {
        let report = AttachMapReport {
            host: "test-host".to_string(),
            generated_at_ms: 0,
            probe_budget_ms: MAP_PROBE_BUDGET_MS,
            coverage: "complete".to_string(),
            daemon_processes_on_host: Some(1),
            daemons: vec![DaemonMapRow {
                pid: 1,
                version: "9.9.9".to_string(),
                build_commit: String::new(),
                endpoint: "unix-/tmp/x.sock".to_string(),
                answer_ms: Some(2),
                uptime_ms: 0,
                owned_terminal_session_count: 0,
                preserved_terminal_owner_count: 0,
                live_terminal_session_count: 0,
                stored_terminal_session_count: 0,
                hot_restart_pending: false,
                hot_restart_blocker_count: 0,
                permanent_blocker_count: 0,
                exe_deleted: false,
                is_default_endpoint: true,
                bound_names: Vec::new(),
                state: MapState::ok(),
            }],
            clients: Vec::new(),
            preserved_owners: Vec::new(),
            remote_machines: Vec::new(),
            sessions: Vec::new(),
            summary: MapSummary {
                ok: 1,
                warning: 0,
                failed: 0,
            },
        };
        let text = format_attach_map(&report);
        for section in [
            "DAEMONS",
            "CLIENTS OF RECORD",
            "PRESERVED OWNERS",
            "REMOTE MACHINES",
            "LIVE SESSIONS",
            "SUMMARY",
        ] {
            assert!(text.contains(section), "the report must name {section}:\n{text}");
        }
        assert!(text.contains("1 ok"), "{text}");
        assert!(
            text.contains("(pre-field)"),
            "a daemon without a build commit says so, never an empty cell:\n{text}"
        );
    }

    #[test]
    fn durations_a_human_reads_at_a_glance() {
        assert_eq!(human_duration(5_000), "5s");
        assert_eq!(human_duration(120_000), "2m");
        assert_eq!(human_duration(5 * 3_600_000 + 600_000), "5h10m");
        assert_eq!(human_duration(50 * 3_600_000), "2d2h");
        assert_eq!(human_duration(3 * 86_400_000), "3d0h");
    }
}
