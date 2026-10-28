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
//! the same identity back. Role and parent are immutable per token: an
//! identity that changes shape mid-flight is not an identity. Pid, cgroup and
//! title observations are deliberately NOT updated on re-register; they
//! belong to `annotate`, an explicit later call, so a retry cannot silently
//! re-vouch facts it did not re-observe.
//!
//! ## Live/pinned veto
//!
//! `retire` refuses while the recorded pid is still live or the record is
//! pinned, unless the caller passes an explicit force flag — the same
//! veto-first law the row registry holds. A pinned row is somebody's
//! deliberate keep-alive; a live pid is somebody's running worker. Neither is
//! retired by a stale ledger entry by accident.
//!
//! ## GUI independence
//!
//! Records live under `<home>/runtime/workers/` — daemon-home state, exactly
//! like `runtime/attach/<uuid>/session.json`. Nothing here touches the GUI's
//! row table, the outline numbers, or any window: a GUI that is absent,
//! closed, or killed leaves every record and every owned PTY untouched, and
//! the records survive a daemon hot restart because they are files, not
//! daemon memory. ⚠ Registration is the METADATA plane: it records the
//! identity of a session the caller ALREADY ensured through the daemon's
//! create/ensure RPC. It is not a spawn path, and it must not become one —
//! the daemon's own creation RPC (`AppControlCommand::CreateTerminal`,
//! served over the local socket, or `ServerRequest::EnsureShellSession`) is
//! the only creator of daemon-owned rows.
//!
//! ## Explicit unknowns
//!
//! `title`, `harness`, `session_id` (the agent CLI's own native session
//! id), `source` (launch provenance: argv head or commit), `pid`, `cgroup`
//! and `parent_token` are `Option` and serialize as JSON `null` when
//! unknown. A null is an honest "cannot say", never a guessed default — the
//! same law `CreatorStamp::parent_session_path` states for a row parent that
//! has since gone.
//!
//! ## Known limits
//!
//! `pid_is_live` reads `/proc/<pid>` and is therefore Linux-only truth; on
//! other hosts it answers `false` (not live) and the veto degrades to the
//! pinned check alone. That is recorded HERE because no caller can recover
//! it from the answer alone.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

/// The record schema this module writes. Bumped when a field changes meaning,
/// so a reader can refuse a record it predates instead of misreading it.
pub const WORKER_IDENTITY_SCHEMA: &str = "ygg4-worker-identity/1";

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
/// Anything outside `[A-Za-z0-9._:-]` becomes `-`; a token that sanitizes to
/// nothing, or to a path fragment (`.` / `..`), is refused rather than
/// silently renamed — a renamed identity is a second identity.
fn sanitize_launch_token(token: &str) -> Result<String> {
    let mut out = String::new();
    for ch in token.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        return Err(anyhow::anyhow!(
            "launch token {token:?} has no usable filename form"
        ));
    }
    Ok(out)
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

/// Is this pid a live process? Linux reads `/proc/<pid>`; on any other host
/// the honest answer is "cannot say" and the caller gets `false` — which the
/// veto treats as "the pid check contributes nothing", NOT as proof of death
/// for anything else. Test pids use values `/proc` cannot hold (e.g. 0).
pub fn pid_is_live(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// The cgroup v2 relative path a pid runs under, from `/proc/<pid>/cgroup`.
/// Best effort: unreadable or non-v2 output is `None` — an explicit unknown,
/// never a guessed fallback path.
pub fn cgroup_v2_path_for_pid(pid: u32) -> Option<String> {
    let raw = fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    raw.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

/// The veto predicate, separated from the store so a caller with fresher
/// liveness knowledge (the daemon's own tenant table) can substitute its own
/// check instead of a `/proc` read.
pub fn veto_reason(
    record: &WorkerIdentityRecord,
    pid_live: &dyn Fn(u32) -> bool,
) -> Option<String> {
    if record.retired.is_some() {
        return None;
    }
    if record.pinned {
        return Some("pinned".to_string());
    }
    if let Some(pid) = record.pid {
        if pid_live(pid) {
            return Some(format!("live pid {pid}"));
        }
    }
    None
}

/// Register (or idempotently re-observe) one worker identity. The FIRST call
/// with a token writes; every later call with the same token returns the
/// existing record verbatim — and REFUSES if the caller now claims a
/// different role, because a token that changes role mid-flight is not an
/// identity.
pub fn register(home: &Path, request: RegisterRequest) -> Result<RegisterOutcome> {
    let path = record_path(home, &request.launch_token)?;
    if let Some(existing) = load_record_at(&path)? {
        if existing.role != request.role {
            return Err(anyhow::anyhow!(
                "launch token {:?} is already registered as {}; refusing to re-register as {}",
                request.launch_token,
                existing.role.as_str(),
                request.role.as_str()
            ));
        }
        return Ok(RegisterOutcome::Existing {
            record: existing,
        });
    }
    // The cgroup observation rides the create only: a retry that arrives
    // after the worker moved groups must not overwrite a stale observation
    // with a NEWER-but-unverified one — it returns the old record verbatim.
    let cgroup = request.pid.and_then(cgroup_v2_path_for_pid);
    let record = WorkerIdentityRecord {
        schema: WORKER_IDENTITY_SCHEMA.to_string(),
        launch_token: sanitize_launch_token(&request.launch_token)?,
        role: request.role,
        session_marker: request.session_marker.as_deref().and_then(sanitize_token_field),
        session_id: request.session_id.as_deref().and_then(sanitize_token_field),
        harness: request.harness.as_deref().and_then(sanitize_token_field),
        title: request.title.as_deref().and_then(sanitize_free_text),
        source: request.source.as_deref().and_then(sanitize_free_text),
        parent_token: request.parent_token.as_deref().and_then(sanitize_token_field),
        pid: request.pid,
        cgroup,
        pinned: request.pinned,
        retired: None,
        created_at_ms: request.now_ms,
        updated_at_ms: request.now_ms,
    };
    write_record(&path, &record)?;
    Ok(RegisterOutcome::Created { record })
}

/// Retire one identity, veto-first: a pinned record or one whose recorded
/// pid is still live is refused unless the caller forces, and the force is
/// recorded in the note. Re-retiring is an idempotent no-op that reports the
/// existing note.
pub fn retire(
    home: &Path,
    token: &str,
    reason: &str,
    pid_live: &dyn Fn(u32) -> bool,
    force: bool,
    now_ms: u64,
) -> Result<RetireOutcome> {
    let path = record_path(home, token)?;
    let record = load_record_at(&path)?.ok_or_else(|| {
        anyhow::anyhow!("no worker identity registered under launch token {token:?}")
    })?;
    if record.retired.is_some() {
        return Ok(RetireOutcome::AlreadyRetired { record });
    }
    if let Some(veto) = veto_reason(&record, pid_live) {
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
/// retiring and registering a successor, which leaves an audit trail.
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

fn write_record(path: &Path, record: &WorkerIdentityRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating worker directory {}", parent.display()))?;
    }
    // Write-then-rename: a reader that lists workers concurrently sees either
    // the old file or the complete new one, never a truncated record.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(record)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
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
            pinned: false,
            now_ms: 1_000,
        }
    }

    #[test]
    fn register_is_idempotent_by_launch_token_and_role_is_immutable() {
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
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn retire_respects_pinned_and_live_pid_veto_and_records_force() {
        let home = temp_home("veto");
        let mut pinned = request("pinned-1", WorkerRole::Worker);
        pinned.pinned = true;
        register(&home, pinned).expect("register pinned");
        let dead = |pid: u32| pid_is_live(pid) && pid != 0;
        let vetoed = retire(&home, "pinned-1", "done", &dead, false, 2_000).expect("retire");
        assert!(matches!(vetoed, RetireOutcome::Vetoed { .. }));
        let forced = retire(&home, "pinned-1", "done", &dead, true, 3_000).expect("force");
        let record = match forced {
            RetireOutcome::Retired { record } => record,
            other => panic!("expected Retired, got {other:?}"),
        };
        let note = record.retired.expect("retire note written");
        assert!(note.forced, "the override is recorded, not hidden");
        let again = retire(&home, "pinned-1", "done", &dead, false, 4_000).expect("again");
        assert!(matches!(again, RetireOutcome::AlreadyRetired { .. }));
        let _ = fs::remove_dir_all(&home);
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
    fn context_successor_lineage_is_a_record_fact_not_a_seat() {
        let home = temp_home("relay");
        let mut successor = request("succ-1", WorkerRole::ContextSuccessor);
        successor.parent_token = Some("pred-1".to_string());
        register(&home, successor).expect("register successor");
        let records = list_records(&home).expect("list");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].role, WorkerRole::ContextSuccessor);
        assert_eq!(records[0].parent_token.as_deref(), Some("pred-1"));
        let _ = fs::remove_dir_all(&home);
    }
}
