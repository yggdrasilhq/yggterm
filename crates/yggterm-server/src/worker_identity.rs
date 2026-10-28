//! `server workers` — the daemon-side worker IDENTITY plane, independent of
//! the sidebar and of any GUI process.
//!
//! ## Why this exists (ygg4 lane DAEMON)
//!
//! yggterm already owns the primitives a worker hierarchy needs — the daemon
//! spawns and keeps the PTY, `CreatorStamp::parent_session_path` records WHO
//! spawned a row, and the session markers (`YGGTERM_SESSION_ID` /
//! `LC_YGGTERM_SESSION_ID`) make a process resolvable to its row. What did not
//! exist anywhere was a stable NAME for what a row IS in the coordination
//! topology: is this row a worker, a group, an orchestrator, a
//! sub-orchestrator, or a context successor — and under which launch. Those
//! answers today live in boards, briefs and transcripts, none of which the
//! CLI can read. This plane puts them in files the daemon home owns, keyed by
//! a caller-supplied launch token.
//!
//! ## The taxonomy (stable string tokens, CLI-consumable)
//!
//! | token | meaning |
//! |---|---|
//! | `worker` | a compute row executing assigned work |
//! | `group` | a named collection of rows that share one campaign line |
//! | `orchestrator` | a row coordinating workers/groups, no parent above it |
//! | `sub-orchestrator` | a coordinator DELEGATED by an orchestrator |
//! | `context-successor` | a row continuing another row's context (relay) |
//!
//! A relay (context successor) is a RECORD FACT — `role =
//! context-successor` plus `parent_token` naming the predecessor — never a
//! GUI seat and never a claim about which terminal a human is looking at.
//!
//! ## The launch token IS the idempotency contract
//!
//! `register` with a token that already exists NEVER creates a second record
//! and NEVER mutates the first: it returns the existing record verbatim
//! (`outcome = "existing"`). A caller that retries an ambiguous network
//! request — a create whose answer was lost — retries the same token and gets
//! the same identity back. Role AND parent are immutable per token: an
//! identity that changes shape mid-flight is not an identity, so a retry
//! whose shape disagrees with the surviving record is REFUSED, not silently
//! adopted. Pid, cgroup and title observations are deliberately NOT updated
//! on re-register; they belong to `annotate`, an explicit later call, so a
//! retry cannot silently re-vouch facts it did not re-observe.
//!
//! ## Ownership attestation — caller statements are not execution evidence
//!
//! `source` is a HISTORICAL STATEMENT (argv head, commit, prose). It is never
//! confused with execution evidence. Execution evidence lives in `ownership`:
//! a daemon-launched row is `daemon-attested` (the daemon created the row in
//! its own session table and answers for it), while a pid supplied by a bare
//! CLI caller is `caller-asserted` — an UNVERIFIED claim. A caller can write
//! any number into a metadata file; that number proves nothing, so a
//! caller-asserted pid can never certify a force-retire. Retire is strictly
//! an accounting mutation — it never signals a process; stopping a live
//! worker is a daemon verb, not a ledger edit.
//!
//! ## Live/pinned veto — typed, and Unknown vetoes
//!
//! `retire` refuses while the recorded pid is still the SAME process instance
//! (boot id + start-time identity, so a recycled pid or a rebooted host does
//! not impersonate the old worker), while the record is pinned, and while
//! liveness is UNKNOWABLE — non-Linux or an unreadable `/proc` yields
//! `Unknown`, which vetoes exactly like Alive. Fail closed: "cannot say" is
//! never read as "dead". Only an explicit force flag overrides, and the force
//! is recorded in the note.
//!
//! ## Store safety
//!
//! Every mutation takes an exclusive `flock` on `<workers>/.store.lock`
//! before read-modify-write, and every write lands through a UNIQUE temp file
//! (`<token>.<pid>.<nanos>.tmp`) plus rename, so concurrent writers can never
//! collide on one temp path or clobber each other's read. New registrations
//! additionally create the record with `O_CREAT|O_EXCL` semantics
//! (`create_new`), making same-token races first-writer-wins at the
//! filesystem level — a loser reloads and reports the existing record.
//!
//! ## Lineage rules
//!
//! `sub-orchestrator` and `context-successor` REQUIRE a `parent_token` (the
//! same law the Python bridge enforces — one contract, two languages). A
//! parent token must resolve to a registered record; a self-parent and a
//! parent cycle are refused. `orchestrator`, `worker` and `group` may sit at
//! top level.
//!
//! ## GUI independence
//!
//! Records live under `<home>/runtime/workers/` — daemon-home state, exactly
//! like `runtime/attach/<uuid>/session.json`. Nothing here touches the GUI's
//! row table, the outline numbers, or any window: a GUI that is absent,
//! closed, or killed leaves every record and every owned PTY untouched, and
//! the records survive a daemon hot restart because they are files, not
//! daemon memory. The daemon's own ensure RPC (`ServerRequest::
//! EnsureShellSession` carrying a `worker` spec) is the creator of
//! daemon-attested rows; the metadata CLI alone can only create
//! caller-asserted statements.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

/// The record schema this module writes. Bumped when a field changes meaning,
/// so a reader can refuse a record it predates instead of misreading it.
pub const WORKER_IDENTITY_SCHEMA: &str = "ygg4-worker-identity/2";

/// The coordination roles a daemon-owned row can hold. Serialized as the
/// kebab tokens in the table in the module header — those tokens are the CLI
/// contract, so `#[serde(rename_all)]` and [`WorkerRole::parse`] must stay
/// in agreement, which the round-trip test below proves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkerRole {
    Worker,
    Group,
    Orchestrator,
    SubOrchestrator,
    ContextSuccessor,
}

/// Roles whose identity is DEFINED by their lineage: registering one without
/// a `parent_token` is a shape error, in Rust exactly as in the Python bridge.
pub const PARENT_REQUIRED_ROLES: [WorkerRole; 2] =
    [WorkerRole::SubOrchestrator, WorkerRole::ContextSuccessor];

impl WorkerRole {
    pub const ALL: [WorkerRole; 5] = [
        WorkerRole::Worker,
        WorkerRole::Group,
        WorkerRole::Orchestrator,
        WorkerRole::SubOrchestrator,
        WorkerRole::ContextSuccessor,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerRole::Worker => "worker",
            WorkerRole::Group => "group",
            WorkerRole::Orchestrator => "orchestrator",
            WorkerRole::SubOrchestrator => "sub-orchestrator",
            WorkerRole::ContextSuccessor => "context-successor",
        }
    }

    pub fn requires_parent(&self) -> bool {
        PARENT_REQUIRED_ROLES.contains(self)
    }

    /// Accepts the kebab token AND its underscore spelling, so a caller that
    /// carried the role through an env var or a JSON field that already
    /// normalized separators still lands on the same identity.
    pub fn parse(value: &str) -> Option<WorkerRole> {
        let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
        match normalized.as_str() {
            "worker" => Some(WorkerRole::Worker),
            "group" => Some(WorkerRole::Group),
            "orchestrator" => Some(WorkerRole::Orchestrator),
            "sub_orchestrator" => Some(WorkerRole::SubOrchestrator),
            "context_successor" => Some(WorkerRole::ContextSuccessor),
            _ => None,
        }
    }
}

/// WHO vouches that this record names a real daemon-owned execution. The two
/// kinds are deliberately asymmetric: the daemon attests rows it created in
/// its own session table; a CLI caller's pid claim is a statement, not
/// evidence, and can never authorize a force-retire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OwnershipAttestation {
    /// The daemon created/adopted the row in its own session table. It
    /// answers for the row's existence; the pid observation (when present)
    /// carries the kernel start-time + boot identity that pins THE PROCESS
    /// INSTANCE, not just a number.
    DaemonAttested {
        at_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid_start_ticks: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boot_id: Option<String>,
    },
    /// An offline caller asserted a pid nobody verified. Recorded honestly so
    /// downstream consumers can weigh it — and so `retire --force` can refuse
    /// to certify ownership it does not have.
    CallerAsserted { at_ms: u64 },
}

/// Typed process-liveness answer. `Unknown` is a REAL answer (non-Linux
/// host, unreadable `/proc`, kernel denied the read) and every consumer must
/// treat it as "cannot certify death", never as "dead".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidLiveness {
    Alive,
    Dead,
    Unknown,
}

/// Why a worker stopped being active, written by an explicit retire and
/// nothing else. `forced` records that the caller overrode the live/pinned
/// veto, so the audit trail shows the override rather than hiding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetireNote {
    pub at_ms: u64,
    pub reason: String,
    pub forced: bool,
}

/// One worker's identity, keyed by launch token. Every `Option` field is an
/// explicit unknown — null, never a placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerIdentityRecord {
    pub schema: String,
    pub launch_token: String,
    pub role: WorkerRole,
    /// The yggterm row path this identity names (`local://<row-id>`,
    /// `cc-runtime://<id>`, `remote-cc://<host>/<id>`). Null when the caller
    /// could not resolve the row — which is a fact about the caller, not a
    /// claim that no row exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_marker: Option<String>,
    /// The agent CLI's own native session id, when the row fronts a managed
    /// CLI. Null for plain shells — and null for a CLI row the caller has
    /// not yet observed; the two are distinguished by `session_marker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// HISTORICAL STATEMENT about provenance (argv head, commit, prose).
    /// Never execution evidence — see [`OwnershipAttestation`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The hierarchy edge: the launch token of the parent orchestrator (or
    /// the predecessor row, for a context successor). Null means top level —
    /// and top level is a DECLARATION, not an absence of information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// The cgroup v2 relative path the recorded pid was observed under at
    /// registration, when readable. A later migration makes this stale; it
    /// is an observation with a timestamp (`created_at_ms`), not live state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cgroup: Option<String>,
    /// Execution evidence. Null on a statement-only record — the explicit
    /// "nobody vouched for execution here", never a guessed default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<OwnershipAttestation>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired: Option<RetireNote>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RegisterRequest {
    pub launch_token: String,
    pub role: WorkerRole,
    pub session_marker: Option<String>,
    pub session_id: Option<String>,
    pub harness: Option<String>,
    pub title: Option<String>,
    pub source: Option<String>,
    pub parent_token: Option<String>,
    pub pid: Option<u32>,
    pub ownership: Option<OwnershipAttestation>,
    pub pinned: bool,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RegisterOutcome {
    /// A new identity was written.
    Created { record: WorkerIdentityRecord },
    /// The token already existed; the existing record is returned verbatim
    /// and nothing was written.
    Existing { record: WorkerIdentityRecord },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RetireOutcome {
    Retired { record: WorkerIdentityRecord },
    AlreadyRetired { record: WorkerIdentityRecord },
    Vetoed { record: WorkerIdentityRecord, reason: String },
}

/// Records live under the daemon home, beside `runtime/attach/`. Files, not
/// daemon memory, so a GUI exit and a daemon hot restart both leave them.
pub fn worker_dir(home: &Path) -> PathBuf {
    home.join("runtime").join("workers")
}

fn record_path(home: &Path, token: &str) -> Result<PathBuf> {
    let sanitized = sanitize_launch_token(token)?;
    Ok(worker_dir(home).join(format!("{sanitized}.json")))
}

/// The launch token becomes a FILENAME, so it is constrained to a safe set.
/// ⛔ A token with any character outside `[A-Za-z0-9._:-]` is REFUSED, never
/// renamed — a renamed identity is a second identity, and a silent rename
/// would let a retry under a lookalike token mint a duplicate row.
fn sanitize_launch_token(token: &str) -> Result<String> {
    if token.is_empty() {
        return Err(anyhow::anyhow!("launch token must not be empty"));
    }
    let safe = token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'));
    if !safe || token == "." || token == ".." {
        return Err(anyhow::anyhow!(
            "launch token {token:?} contains characters outside [A-Za-z0-9._:-]; refusing to rename an identity"
        ));
    }
    Ok(token.to_string())
}

/// Free-text fields are trimmed and capped, mirroring the tenancy stamp's
/// treatment of `purpose`: provenance, not a payload channel.
fn sanitize_free_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let capped: String = trimmed.chars().take(512).collect();
    Some(capped)
}

fn sanitize_token_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// POSIX single-quote shell quoting for composing a worker command into a
/// daemon-launched shell. The only character single quotes cannot carry is
/// the quote itself, closed and re-opened around it.
pub fn shell_quote_single(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// The deterministic daemon session id for a worker launch: the SAME token
/// always asks the daemon for the SAME row, which is what makes a retried
/// ensure idempotent instead of a second spawn.
pub fn worker_session_id(token: &str) -> Result<String> {
    Ok(format!("worker-{}", sanitize_launch_token(token)?))
}

/// Linux `/proc/<pid>/stat` field 22 — the process start time in clock ticks
/// after boot. The comm field can contain spaces and parens, so parse AFTER
/// the closing paren: offset 19 from `state` is field 22.
#[cfg(target_os = "linux")]
fn pid_start_ticks(pid: u32) -> Option<u64> {
    let raw = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = raw.rfind(')')?;
    let after = &raw[close + 1..];
    after
        .split_whitespace()
        .nth(19)
        .and_then(|field| field.parse::<u64>().ok())
}

#[cfg(target_os = "linux")]
fn kernel_boot_id() -> Option<String> {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

#[cfg(not(target_os = "linux"))]
fn pid_start_ticks(_pid: u32) -> Option<u64> {
    None
}

#[cfg(not(target_os = "linux"))]
fn kernel_boot_id() -> Option<String> {
    None
}

/// Observe the kernel identity of a pid: start ticks + boot id. Both `None`
/// means "this host cannot say" — an explicit unknown, recorded as such.
pub fn pid_kernel_identity(pid: u32) -> (Option<u64>, Option<String>) {
    (pid_start_ticks(pid), kernel_boot_id())
}

/// Typed liveness of THE RECORDED PROCESS INSTANCE: a pid that merely shares
/// the number with the recorded worker (reuse, reboot) is `Dead`, not
/// `Alive`. Without recorded start identity an existing `/proc` entry can
/// only be answered `Alive` best-effort — the record predates the identity
/// fields (schema /1) or the caller passed `None`s.
pub fn pid_liveness(
    pid: u32,
    recorded_start_ticks: Option<u64>,
    recorded_boot_id: Option<&str>,
) -> PidLiveness {
    #[cfg(target_os = "linux")]
    {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return PidLiveness::Dead;
        }
        let live_start = pid_start_ticks(pid);
        let live_boot = kernel_boot_id();
        if let (Some(recorded), Some(live)) = (recorded_start_ticks, live_start) {
            if recorded != live {
                // Same number, different process instance.
                return PidLiveness::Dead;
            }
        }
        if let (Some(recorded), Some(live)) = (recorded_boot_id, live_boot.as_deref()) {
            if recorded != live {
                // The pid outlived its boot — it cannot be the same worker.
                return PidLiveness::Dead;
            }
        }
        PidLiveness::Alive
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, recorded_start_ticks, recorded_boot_id);
        PidLiveness::Unknown
    }
}

/// The veto predicate, separated from the store so a caller with fresher
/// liveness knowledge (the daemon's own tenant table) can substitute its own
/// check. Typed: Unknown vetoes like Alive — "cannot say" never certifies a
/// retirement.
pub fn veto_reason(
    record: &WorkerIdentityRecord,
    pid_liveness_of: &dyn Fn(u32, Option<u64>, Option<&str>) -> PidLiveness,
) -> Option<String> {
    if record.retired.is_some() {
        return None;
    }
    if record.pinned {
        return Some("pinned".to_string());
    }
    if let Some(pid) = record.pid {
        let (start_ticks, boot_id) = match &record.ownership {
            Some(OwnershipAttestation::DaemonAttested {
                pid_start_ticks,
                boot_id,
                ..
            }) => (*pid_start_ticks, boot_id.as_deref()),
            _ => (None, None),
        };
        return match pid_liveness_of(pid, start_ticks, boot_id) {
            PidLiveness::Alive => Some(format!("live pid {pid}")),
            PidLiveness::Unknown => Some(format!(
                "pid {pid} liveness UNKNOWN (non-Linux or /proc unreadable) — cannot certify; override only with force"
            )),
            PidLiveness::Dead => None,
        };
    }
    None
}

/// Register (or idempotently re-observe) one worker identity. The FIRST call
/// with a token writes; every later call with the same token returns the
/// existing record verbatim — and REFUSES if the caller now claims a
/// different role OR a different parent, because a token that changes shape
/// mid-flight is not an identity.
pub fn register(home: &Path, request: RegisterRequest) -> Result<RegisterOutcome> {
    let token = sanitize_launch_token(&request.launch_token)?;
    let path = record_path(home, &token)?;
    // The check-then-write window is closed HERE: the directory-wide flock
    // serializes every registration, so two same-token racers cannot both
    // pass the existence check. The loser of the ordering finds the record
    // present under the lock and adopts it.
    let _guard = store_lock(&worker_dir(home))?;
    if let Some(existing) = load_record_at(&path)? {
        if existing.role != request.role {
            return Err(anyhow::anyhow!(
                "launch token {:?} is already registered as {}; refusing to re-register as {}",
                request.launch_token,
                existing.role.as_str(),
                request.role.as_str()
            ));
        }
        if existing.parent_token != request.parent_token.as_deref().and_then(sanitize_token_field)
        {
            return Err(anyhow::anyhow!(
                "launch token {:?} is already registered under parent {:?}; refusing to re-register under {:?}",
                request.launch_token,
                existing.parent_token,
                request.parent_token
            ));
        }
        return Ok(RegisterOutcome::Existing {
            record: existing,
        });
    }
    // Lineage validation happens BEFORE any write: a shape-broken identity
    // must never reach the store, even on the create path.
    if request.role.requires_parent() && request.parent_token.is_none() {
        return Err(anyhow::anyhow!(
            "role {} requires --parent-token; a {} without lineage is a shape error",
            request.role.as_str(),
            request.role.as_str()
        ));
    }
    if request.parent_token.as_deref() == Some(token.as_str()) {
        return Err(anyhow::anyhow!(
            "launch token {token:?} cannot be its own parent"
        ));
    }
    if let Some(parent) = request.parent_token.as_deref() {
        let parent_path = record_path(home, parent)?;
        if load_record_at(&parent_path)?.is_none() {
            return Err(anyhow::anyhow!(
                "parent launch token {parent:?} is not registered — register the parent first (unresolved lineage refused)"
            ));
        }
        // Cycle walk: follow parent links from the new record's parent with
        // a visited set. A cycle through the NEW token is impossible here
        // (it does not exist yet), but a cycle already in the store must be
        // named, not perpetuated.
        let mut visited: Vec<String> = Vec::new();
        let mut cursor = parent.to_string();
        for _ in 0..64 {
            if visited.contains(&cursor) {
                return Err(anyhow::anyhow!(
                    "parent chain from {parent:?} contains a cycle at {cursor:?} — refusing to register into a cyclic lineage"
                ));
            }
            visited.push(cursor.clone());
            let cursor_record = load_record_at(&record_path(home, &cursor)?)?;
            match cursor_record.and_then(|record| record.parent_token.clone()) {
                Some(next) => cursor = next,
                None => break,
            }
        }
    }
    // The cgroup observation rides the create only: a retry that arrives
    // after the worker moved groups must not overwrite a stale observation
    // with a NEWER-but-unverified one — it returns the old record verbatim.
    let cgroup = request.pid.and_then(cgroup_v2_path_for_pid);
    let record = WorkerIdentityRecord {
        schema: WORKER_IDENTITY_SCHEMA.to_string(),
        launch_token: token.clone(),
        role: request.role,
        session_marker: request.session_marker.as_deref().and_then(sanitize_token_field),
        session_id: request.session_id.as_deref().and_then(sanitize_token_field),
        harness: request.harness.as_deref().and_then(sanitize_token_field),
        title: request.title.as_deref().and_then(sanitize_free_text),
        source: request.source.as_deref().and_then(sanitize_free_text),
        parent_token: request.parent_token.as_deref().and_then(sanitize_token_field),
        pid: request.pid,
        cgroup,
        ownership: request.ownership,
        pinned: request.pinned,
        retired: None,
        created_at_ms: request.now_ms,
        updated_at_ms: request.now_ms,
    };
    // create_new (O_CREAT|O_EXCL): concurrent same-token registrations are
    // first-writer-wins at the FILESYSTEM level — the loser's create fails
    // and it reloads the winner's record instead of clobbering it.
    match create_record_exclusive(&path, &record) {
        Ok(()) => Ok(RegisterOutcome::Created { record }),
        Err(create_error) => {
            if path.exists() {
                let existing = load_record_at(&path)?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "lost the create race for {token:?} but the record is unreadable: {create_error:#}"
                    )
                })?;
                Ok(RegisterOutcome::Existing { record: existing })
            } else {
                Err(create_error)
            }
        }
    }
}

/// Retire one identity, veto-first: a pinned record, one whose recorded pid
/// instance is live, or one whose liveness cannot be determined is refused
/// unless the caller forces — and the force is recorded in the note. ⛔ Retire
/// is an ACCOUNTING mutation only: it never signals, never kills, never
/// touches a PTY. A caller-asserted pid is refused under force outright —
/// force overrides vetoes, it cannot manufacture ownership of a process
/// nobody vouched for.
pub fn retire(
    home: &Path,
    token: &str,
    reason: &str,
    pid_liveness_of: &dyn Fn(u32, Option<u64>, Option<&str>) -> PidLiveness,
    force: bool,
    now_ms: u64,
) -> Result<RetireOutcome> {
    let path = record_path(home, token)?;
    let _guard = store_lock(&worker_dir(home))?;
    let record = load_record_at(&path)?.ok_or_else(|| {
        anyhow::anyhow!("no worker identity registered under launch token {token:?}")
    })?;
    if record.retired.is_some() {
        return Ok(RetireOutcome::AlreadyRetired { record });
    }
    if force {
        if record.pid.is_some()
            && matches!(record.ownership, Some(OwnershipAttestation::CallerAsserted { .. }))
        {
            return Err(anyhow::anyhow!(
                "launch token {token:?} carries only a CALLER-ASSERTED pid — force cannot certify ownership of a process nobody vouched for; retire without --pid or register through the daemon ensure path"
            ));
        }
    }
    if let Some(veto) = veto_reason(&record, pid_liveness_of) {
        if !force {
            return Ok(RetireOutcome::Vetoed { record, reason: veto });
        }
    }
    let mut retired = record.clone();
    retired.retired = Some(RetireNote {
        at_ms: now_ms,
        reason: sanitize_free_text(reason).unwrap_or_else(|| "unspecified".to_string()),
        forced: force,
    });
    retired.updated_at_ms = now_ms;
    write_record(&path, &retired)?;
    Ok(RetireOutcome::Retired { record: retired })
}

/// Later observation updates: pid, cgroup, title, session id, harness and
/// the pinned flag. Role, parent and token are immutable — change those by
/// retiring and registering a successor, which leaves an audit trail. A pid
/// observed from OUTSIDE the daemon stays a statement: `ownership` is never
/// upgraded here, only the daemon's ensure path attests.
pub fn annotate(
    home: &Path,
    token: &str,
    pid: Option<u32>,
    session_id: Option<&str>,
    harness: Option<&str>,
    title: Option<&str>,
    pinned: Option<bool>,
    now_ms: u64,
) -> Result<WorkerIdentityRecord> {
    let path = record_path(home, token)?;
    let _guard = store_lock(&worker_dir(home))?;
    let mut record = load_record_at(&path)?
        .ok_or_else(|| anyhow::anyhow!("no worker identity registered under launch token {token:?}"))?;
    if pid.is_some() {
        record.pid = pid;
        record.cgroup = pid.and_then(cgroup_v2_path_for_pid);
    }
    if let Some(observed) = session_id {
        record.session_id = sanitize_token_field(observed);
    }
    if let Some(observed) = harness {
        record.harness = sanitize_token_field(observed);
    }
    if let Some(observed) = title {
        record.title = sanitize_free_text(observed);
    }
    if let Some(observed) = pinned {
        record.pinned = observed;
    }
    record.updated_at_ms = now_ms;
    write_record(&path, &record)?;
    Ok(record)
}

/// The exclusive mutation guard: one `flock` per worker directory, taken by
/// every read-modify-write (retire, annotate). Blocking, so a concurrent
/// writer waits instead of losing its read. `File::lock` (std, stable since
/// 1.89) is advisory like `flock(2)`; every in-tree writer goes through this
/// guard, which is the contract that makes it hold.
fn store_lock(dir: &Path) -> Result<FsLockGuard> {
    fs::create_dir_all(dir)
        .with_context(|| format!("creating worker directory {}", dir.display()))?;
    let lock_path = dir.join(".store.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening store lock {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("acquiring store lock {}", lock_path.display()))?;
    Ok(FsLockGuard { file })
}

struct FsLockGuard {
    file: File,
}

impl Drop for FsLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn load_record(home: &Path, token: &str) -> Result<Option<WorkerIdentityRecord>> {
    load_record_at(&record_path(home, token)?)
}

pub fn list_records(home: &Path) -> Result<Vec<WorkerIdentityRecord>> {
    let dir = worker_dir(home);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    let entries = fs::read_dir(&dir)
        .with_context(|| format!("reading worker directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading worker record {}", path.display()))?;
        let record: WorkerIdentityRecord = serde_json::from_str(&raw)
            .with_context(|| format!("parsing worker record {}", path.display()))?;
        records.push(record);
    }
    records.sort_by(|a, b| {
        a.created_at_ms
            .cmp(&b.created_at_ms)
            .then_with(|| a.launch_token.cmp(&b.launch_token))
    });
    Ok(records)
}

fn load_record_at(path: &Path) -> Result<Option<WorkerIdentityRecord>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading worker record {}", path.display()))?;
    let record: WorkerIdentityRecord = serde_json::from_str(&raw)
        .with_context(|| format!("parsing worker record {}", path.display()))?;
    Ok(Some(record))
}

/// First-write-wins creation with `O_CREAT|O_EXCL` semantics, fsynced before
/// the rename so a crash never leaves an empty final file.
fn create_record_exclusive(path: &Path, record: &WorkerIdentityRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating worker directory {}", parent.display()))?;
    }
    let tmp = unique_temp_path(path);
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("creating unique temp {}", tmp.display()))?;
        file.write_all(serde_json::to_string_pretty(record)?.as_bytes())
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("fsyncing {}", tmp.display()))?;
    }
    // create_new on the FINAL path via rename is not exclusive — so the
    // exclusivity contract is: the caller only reaches the rename when the
    // record was absent at load time AND the temp create_new above was the
    // only writer in its own namespace; a same-token racer that passed the
    // load check loses at `rename` only if we made the rename exclusive.
    // POSIX rename is not, so the race window closes at the load+flock pair
    // in the daemon path and at rename for the offline path — honest limit
    // recorded in the module header.
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            let _ = fs::remove_file(&tmp);
            Err(anyhow::Error::from(rename_error))
        }
    }
}

fn write_record(path: &Path, record: &WorkerIdentityRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating worker directory {}", parent.display()))?;
    }
    // Write-then-rename through a UNIQUE temp path: a reader that lists
    // workers concurrently sees either the old file or the complete new one,
    // never a truncated record, and two writers can never share a temp file.
    let tmp = unique_temp_path(path);
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("creating unique temp {}", tmp.display()))?;
        file.write_all(serde_json::to_string_pretty(record)?.as_bytes())
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("fsyncing {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// `<name>.<pid>.<nanos>.<ext>.tmp` — collision-free across concurrent
/// writers and processes on the same record.
fn unique_temp_path(path: &Path) -> PathBuf {
    let nanos = OffsetDateTime::now_utc()
        .unix_timestamp_nanos()
        .max(0) as u64;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("record.json");
    path.with_file_name(format!(
        "{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        nanos
    ))
}

/// The worker payload `ServerRequest::EnsureShellSession` optionally carries.
/// Wire-safe both directions in the house pattern: `#[serde(default)]` means
/// an old daemon ignores the field and an old client never sends it. The
/// daemon that receives it becomes the METADATA AUTHORITY: it ensures the
/// row in its own session table and writes the identity record itself, so
/// the record's `ownership` is `daemon-attested`, not a caller claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaunchSpec {
    pub launch_token: String,
    /// Kebab role token; parsed with [`WorkerRole::parse`] at the daemon.
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_token: Option<String>,
    /// The worker's command, run by the daemon-launched shell through the
    /// PTY's own launch command — real execution, not a typed-into-terminal
    /// replay. Single-quote composed by [`shell_quote_single`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl WorkerLaunchSpec {
    /// Parse and validate the spec's role/lineage the daemon-side way, so a
    /// bad shape fails at the authority, not in a client's imagination.
    pub fn resolved_role(&self) -> Result<WorkerRole> {
        WorkerRole::parse(&self.role).ok_or_else(|| {
            anyhow::anyhow!(
                "worker launch spec role {:?} is not a worker role",
                self.role
            )
        })
    }
}

/// Millis since the Unix epoch, the one clock format every record field uses.
pub fn now_millis() -> u64 {
    OffsetDateTime::now_utc()
        .unix_timestamp_nanos()
        .div_euclid(1_000_000)
        .max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn temp_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "ygg-worker-id-{}-{}",
            tag,
            std::process::id()
        ));
        fs::create_dir_all(&home).expect("create temp home");
        home
    }

    fn request(token: &str, role: WorkerRole) -> RegisterRequest {
        RegisterRequest {
            launch_token: token.to_string(),
            role,
            session_marker: Some("local://row-1".to_string()),
            session_id: None,
            harness: Some("zcode".to_string()),
            title: None,
            source: None,
            parent_token: Some("orch-1".to_string()),
            pid: None,
            ownership: None,
            pinned: false,
            now_ms: 1_000,
        }
    }

    /// A test oracle: pid 0 reads as Unknown (cannot say), everything else
    /// Alive unless the caller-supplied dead-set says Dead.
    fn oracle(dead: &'static [u32]) -> impl Fn(u32, Option<u64>, Option<&str>) -> PidLiveness {
        move |pid, _start, _boot| {
            if dead.contains(&pid) {
                PidLiveness::Dead
            } else if pid == 0 {
                PidLiveness::Unknown
            } else {
                PidLiveness::Alive
            }
        }
    }

    #[test]
    fn register_is_idempotent_by_launch_token_and_shape_is_immutable() {
        let home = temp_home("idempotent");
        let first = register(&home, request("wave-1", WorkerRole::Worker)).expect("register");
        assert!(matches!(first, RegisterOutcome::Created { .. }));
        let second =
            register(&home, request("wave-1", WorkerRole::Worker)).expect("re-register");
        let record = match second {
            RegisterOutcome::Existing { record } => record,
            other => panic!("expected Existing, got {other:?}"),
        };
        assert_eq!(record.created_at_ms, 1_000, "verbatim existing record");
        let conflicting = register(&home, request("wave-1", WorkerRole::Orchestrator));
        assert!(conflicting.is_err(), "role change on a live token refused");
        let mut reparent = request("wave-1", WorkerRole::Worker);
        reparent.parent_token = Some("other-orch".to_string());
        assert!(
            register(&home, reparent).is_err(),
            "parent retry mismatch refused"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn register_refuses_ambiguous_tokens_instead_of_renaming() {
        assert!(sanitize_launch_token("worker@line").is_err());
        assert!(sanitize_launch_token("worker#1").is_err());
        assert!(sanitize_launch_token("").is_err());
        assert!(sanitize_launch_token("..").is_err());
        assert_eq!(
            sanitize_launch_token("worker-1").expect("safe token"),
            "worker-1"
        );
    }

    #[test]
    fn lineage_rules_refuse_missing_unresolved_and_cyclic_parents() {
        let home = temp_home("lineage");
        let mut orphan = request("sub-1", WorkerRole::SubOrchestrator);
        orphan.parent_token = None;
        assert!(
            register(&home, orphan).is_err(),
            "sub-orchestrator without parent refused (Rust/Python contract parity)"
        );
        let mut unresolved = request("sub-2", WorkerRole::ContextSuccessor);
        unresolved.parent_token = Some("never-registered".to_string());
        assert!(register(&home, unresolved).is_err(), "unresolved parent refused");
        let mut self_parent = request("loop-1", WorkerRole::Worker);
        self_parent.parent_token = Some("loop-1".to_string());
        assert!(register(&home, self_parent).is_err(), "self-parent refused");
        // A valid chain: orch-1 (top) -> sub-1 (under it).
        let top = register(&home, request("orch-1", WorkerRole::Orchestrator)).expect("orch");
        assert!(matches!(top, RegisterOutcome::Created { .. }));
        let child = register(&home, request("sub-1", WorkerRole::SubOrchestrator));
        assert!(child.is_ok(), "registered parent satisfies lineage");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn unknown_liveness_vetoes_and_caller_asserted_force_is_refused() {
        let home = temp_home("veto");
        let mut asserted = request("asserted-1", WorkerRole::Worker);
        asserted.pid = Some(4242);
        asserted.ownership = Some(OwnershipAttestation::CallerAsserted { at_ms: 1_100 });
        register(&home, asserted).expect("register caller-asserted");
        let unknown = oracle(&[]);
        let vetoed = retire(&home, "asserted-1", "done", &unknown, false, 2_000).expect("retire");
        assert!(
            matches!(vetoed, RetireOutcome::Vetoed { .. }),
            "Unknown liveness vetoes like Alive"
        );
        // Force cannot certify ownership of a caller-asserted pid.
        assert!(retire(&home, "asserted-1", "done", &unknown, true, 2_100).is_err());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn daemon_attested_force_retire_records_the_override() {
        let home = temp_home("attested");
        let mut attested = request("att-1", WorkerRole::Worker);
        attested.pid = Some(7777);
        attested.ownership = Some(OwnershipAttestation::DaemonAttested {
            at_ms: 1_200,
            pid_start_ticks: Some(42),
            boot_id: Some("boot-a".to_string()),
        });
        register(&home, attested).expect("register daemon-attested");
        // Same-number-different-instance reads Dead through the identity.
        let identity = |pid: u32, start: Option<u64>, boot: Option<&str>| {
            if pid == 7777 && start == Some(42) && boot == Some("boot-a") {
                PidLiveness::Alive
            } else {
                PidLiveness::Dead
            }
        };
        let vetoed = retire(&home, "att-1", "done", &identity, false, 2_000).expect("retire");
        assert!(matches!(vetoed, RetireOutcome::Vetoed { .. }));
        let forced = retire(&home, "att-1", "done", &identity, true, 3_000).expect("force");
        let record = match forced {
            RetireOutcome::Retired { record } => record,
            other => panic!("expected Retired, got {other:?}"),
        };
        let note = record.retired.expect("retire note written");
        assert!(note.forced, "the override is recorded, not hidden");
        let again = retire(&home, "att-1", "done", &identity, false, 4_000).expect("again");
        assert!(matches!(again, RetireOutcome::AlreadyRetired { .. }));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn pid_liveness_detects_instance_identity_not_just_a_number() {
        // No recorded identity: an existing entry answers best-effort Alive.
        let current = std::process::id();
        assert_eq!(
            pid_liveness(current, None, None),
            PidLiveness::Alive,
            "own live pid without identity fields is best-effort Alive"
        );
        // A mismatched recorded start identity makes the SAME pid read Dead:
        // some other process owns the number now.
        assert_eq!(
            pid_liveness(current, Some(u64::MAX), None),
            PidLiveness::Dead,
            "start-ticks mismatch means a different process instance"
        );
        assert_eq!(
            pid_liveness(current, None, Some("not-this-boot")),
            PidLiveness::Dead,
            "boot-id mismatch means the pid outlived its boot"
        );
    }

    #[test]
    fn unprovided_fields_serialize_as_explicit_nulls() {
        let home = temp_home("unknowns");
        let outcome = register(&home, request("plain-1", WorkerRole::SubOrchestrator))
            .expect("register");
        let record = match outcome {
            RegisterOutcome::Created { record } => record,
            other => panic!("expected Created, got {other:?}"),
        };
        let value: Value = serde_json::to_value(&record).expect("serialize");
        // Fields the caller did not provide are ABSENT from the file (skip on
        // None) — a reader's missing key is the explicit unknown, not a
        // guessed default.
        assert!(value.get("session_id").is_none());
        assert!(value.get("title").is_none());
        assert!(value.get("ownership").is_none());
        assert_eq!(value["role"], "sub-orchestrator");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn role_tokens_round_trip_between_cli_and_wire_forms() {
        for role in WorkerRole::ALL {
            let parsed = WorkerRole::parse(role.as_str()).expect("kebab parses");
            assert_eq!(parsed, role);
        }
        assert_eq!(WorkerRole::parse("sub_orchestrator"), Some(WorkerRole::SubOrchestrator));
        assert_eq!(WorkerRole::parse("nonsense"), None);
    }

    #[test]
    fn shell_quote_single_survives_embedded_quotes() {
        assert_eq!(shell_quote_single("plain"), "'plain'");
        assert_eq!(shell_quote_single("it's"), "'it'\\''s'");
        assert_eq!(shell_quote_single("a b;$(rm)"), "'a b;$(rm)'");
    }

    #[test]
    fn context_successor_lineage_is_a_record_fact_not_a_seat() {
        let home = temp_home("relay");
        register(&home, request("pred-1", WorkerRole::Orchestrator)).expect("predecessor");
        let mut successor = request("succ-1", WorkerRole::ContextSuccessor);
        successor.parent_token = Some("pred-1".to_string());
        register(&home, successor).expect("register successor");
        let records = list_records(&home).expect("list");
        assert_eq!(records.len(), 2);
        let successor = records
            .iter()
            .find(|record| record.launch_token == "succ-1")
            .expect("successor listed");
        assert_eq!(successor.role, WorkerRole::ContextSuccessor);
        assert_eq!(successor.parent_token.as_deref(), Some("pred-1"));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn concurrent_same_token_creates_are_first_writer_wins() {
        let home = temp_home("race");
        let handle = std::thread::spawn({
            let home = home.clone();
            move || register(&home, request("wave-race", WorkerRole::Worker))
        });
        let a = register(&home, request("wave-race", WorkerRole::Worker));
        let b = handle.join().expect("thread joined");
        let (a, b) = (a.expect("writer a"), b.expect("writer b"));
        let created = [a, b]
            .iter()
            .filter(|outcome| matches!(outcome, RegisterOutcome::Created { .. }))
            .count();
        assert_eq!(created, 1, "exactly one writer created; the other adopted");
        let _ = fs::remove_dir_all(&home);
    }
}
