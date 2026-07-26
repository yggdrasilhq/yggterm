//! Row tenancy — who created a live row, what is RUNNING inside it, and whether
//! the creator declared the row ephemeral.
//!
//! Daemon-owned PTYs are deliberately immortal: the row surviving IS the
//! feature. The cost is that everything running INSIDE a row becomes an
//! immortal tenant that no surface accounts for — an interactive probe an agent
//! opened and abandoned keeps burning CPU for days and nothing names it. This
//! module is the accounting layer for that, in three pieces that share one
//! encoding:
//!
//! 1. **Tenant cost visibility** ([`row_tenant_report`]) — instrumentation, no
//!    policy. ON DEMAND ONLY: nothing here runs on a timer, so the idle cost is
//!    exactly zero. One `/proc` snapshot serves every row in one request.
//!    Anything it cannot measure is reported as a NAMED reason with the numeric
//!    fields left empty — a faked zero would read as "this row is cheap", which
//!    is the failure this whole module exists to end.
//! 2. **Ownership stamping** ([`CreatorStamp`]) — a headless create records the
//!    creating process and, optionally, its purpose, so provenance survives the
//!    creator by days.
//! 3. **Pre-declared ephemerality** ([`EphemeralDeclaration`]) — OPT IN AT
//!    CREATION ONLY. The creator declares up front "reap this row when my owner
//!    process is gone, or after N idle seconds". The default is unchanged:
//!    unmarked rows and rows the user made are never touched.
//!
//! **Single encoding.** Both stamps live in the session's metadata entries — the
//! same list `Runtime Persistence` (keep-alive) uses — and the persisted row
//! carries those same strings verbatim, so a daemon handover restores exactly
//! what was declared. Encode/parse live here and nowhere else.

use std::collections::{BTreeMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use yggterm_core::cli_flag_value;

/// The host token written when this machine cannot name itself. It is NOT an
/// identity: two hosts that both fall back both write it, so a declaration
/// carrying it can never be matched against a local pid — see
/// [`owner_liveness_on_host`].
pub const UNKNOWN_HOST_TOKEN: &str = "unknown-host";

/// Metadata label carrying [`CreatorStamp`].
pub const CREATED_BY_METADATA_LABEL: &str = "Created By";
/// Metadata label carrying [`EphemeralDeclaration`].
pub const EPHEMERAL_METADATA_LABEL: &str = "Ephemeral";

/// Longest purpose text kept on a stamp. Provenance, not a changelog.
const MAX_PURPOSE_CHARS: usize = 200;
/// Most tenant processes listed per row. A runaway tree should be visible as a
/// COUNT plus its worst offenders, not as an unbounded payload.
pub const MAX_LISTED_TENANTS: usize = 16;

/// Command names that are the row's own shell rather than a tenant. Tenant age
/// deliberately ignores these: an idle login shell is what a row IS.
const SHELL_COMMAND_NAMES: &[&str] = &[
    "sh", "bash", "dash", "zsh", "fish", "ksh", "mksh", "tcsh", "csh", "ash", "busybox",
];

// ---------------------------------------------------------------------------
// Piece 2 — ownership stamping
// ---------------------------------------------------------------------------

/// Who asked for this row. Written once, at creation, by the CLI that created
/// it; never rewritten afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorStamp {
    /// The creating process on `host`. Dead by the time anyone reads this, in
    /// the case this exists for — the value is the AUDIT trail, not a handle.
    pub pid: u32,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

impl CreatorStamp {
    pub fn new(pid: u32, host: &str, purpose: Option<&str>) -> Self {
        Self {
            pid,
            host: sanitize_token(host),
            purpose: purpose.map(sanitize_free_text).filter(|p| !p.is_empty()),
        }
    }

    /// `pid=<n> host=<h>[ purpose=<free text>]`. `purpose` is always last
    /// because it is the only field allowed to contain spaces.
    pub fn encode(&self) -> String {
        let mut encoded = format!("pid={} host={}", self.pid, self.host);
        if let Some(purpose) = self.purpose.as_deref() {
            encoded.push_str(" purpose=");
            encoded.push_str(purpose);
        }
        encoded
    }

    pub fn parse(value: &str) -> Option<Self> {
        let (head, purpose) = split_trailing_free_text(value, "purpose=");
        let mut pid = None;
        let mut host = None;
        for token in head.split_whitespace() {
            if let Some(rest) = token.strip_prefix("pid=") {
                pid = rest.parse::<u32>().ok();
            } else if let Some(rest) = token.strip_prefix("host=") {
                host = Some(rest.to_string());
            }
        }
        Some(Self {
            pid: pid?,
            host: host?,
            purpose,
        })
    }
}

// ---------------------------------------------------------------------------
// Piece 3 — pre-declared ephemerality
// ---------------------------------------------------------------------------

/// The creator's up-front declaration that this row is disposable.
///
/// Both rules are optional and BOTH are honoured when both are present —
/// whichever fires first reaps the row. A declaration with neither rule can
/// never reap anything, which is the safe reading of "the caller declared
/// nothing".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralDeclaration {
    /// The process whose death means the row is abandoned. Only meaningful on
    /// `owner_host`; a daemon on any other host reports the owner UNKNOWN and
    /// never reaps on this rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    pub owner_host: String,
    /// Reap after this many seconds with no PTY output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_ttl_secs: Option<u64>,
}

impl EphemeralDeclaration {
    pub fn new(owner_pid: Option<u32>, owner_host: &str, idle_ttl_secs: Option<u64>) -> Self {
        Self {
            owner_pid,
            owner_host: sanitize_token(owner_host),
            idle_ttl_secs: idle_ttl_secs.filter(|ttl| *ttl > 0),
        }
    }

    /// `[owner-pid=<n> ]owner-host=<h>[ idle-ttl-secs=<n>]`.
    pub fn encode(&self) -> String {
        let mut encoded = String::new();
        if let Some(owner_pid) = self.owner_pid {
            encoded.push_str(&format!("owner-pid={owner_pid} "));
        }
        encoded.push_str(&format!("owner-host={}", self.owner_host));
        if let Some(ttl) = self.idle_ttl_secs {
            encoded.push_str(&format!(" idle-ttl-secs={ttl}"));
        }
        encoded
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut owner_pid = None;
        let mut owner_host = None;
        let mut idle_ttl_secs = None;
        for token in value.split_whitespace() {
            if let Some(rest) = token.strip_prefix("owner-pid=") {
                owner_pid = rest.parse::<u32>().ok();
            } else if let Some(rest) = token.strip_prefix("owner-host=") {
                owner_host = Some(rest.to_string());
            } else if let Some(rest) = token.strip_prefix("idle-ttl-secs=") {
                idle_ttl_secs = rest.parse::<u64>().ok().filter(|ttl| *ttl > 0);
            }
        }
        Some(Self {
            owner_pid,
            owner_host: owner_host?,
            idle_ttl_secs,
        })
    }

    /// A declaration that can never fire is a declaration the caller did not
    /// finish making. THE predicate behind the create-time refusal
    /// ([`EPHEMERAL_NEEDS_AN_EXPLICIT_RULE`]) — refusing at the point of typing
    /// rather than leaving a row marked-but-immortal — and read there, not only
    /// by tests.
    pub fn declares_a_rule(&self) -> bool {
        self.owner_pid.is_some() || self.idle_ttl_secs.is_some()
    }
}

/// What the CLI that is creating a row declares about it.
///
/// `created_by` is stamped on EVERY agent CLI create — provenance is not
/// opt-in. `ephemeral` is opt-in and only ever set from an explicit
/// `--ephemeral`, which is what keeps user- and GUI-created rows permanently
/// out of the reaper's reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTerminalTenancy {
    pub created_by: CreatorStamp,
    pub ephemeral: Option<EphemeralDeclaration>,
}

/// Why a bare `--ephemeral` is REFUSED rather than defaulted.
///
/// Measured on this fleet, not reasoned about: under `bash -c "<cli>"` the
/// parent this CLI would record is the wrapper `bash`, which exits within
/// milliseconds of the create; under `ssh <host> "<cli>"` it is the
/// `sshd-session` process, which dies at disconnect. Either way a defaulted
/// owner is a process that is ALREADY GONE when the first chore tick runs, so
/// `--ephemeral` would reap the row inside a minute — the exact opposite of
/// what the caller asked for, and unrecoverable once the row is closed. The
/// two honest spellings are named in the message because the refusal has to
/// teach, not just refuse.
pub const EPHEMERAL_NEEDS_AN_EXPLICIT_RULE: &str = "--ephemeral needs a rule this daemon can honestly check. Pass \
     --ephemeral-owner-pid <pid> naming a process you KNOW outlives this create (your own \
     pid — not the shell that wrapped it), or --ephemeral-idle-ttl-secs <n> for a \
     TTL-only declaration, or both. There is no default owner: under `bash -c \"<cli>\"` \
     the recorded parent is the wrapper bash and under `ssh <host> \"<cli>\"` it is \
     sshd-session, and both are dead within milliseconds of the create, so a defaulted \
     owner would reap the row on the next chore tick.";

/// Parse the tenancy flags of an agent CLI `terminal new`.
///
/// | flag | effect |
/// |---|---|
/// | `--purpose <text>` | recorded on the stamp; always optional |
/// | `--ephemeral` | opt IN to the reaper; needs one of the two rules below |
/// | `--ephemeral-owner-pid <pid>` | reap once that pid leaves `/proc` |
/// | `--ephemeral-idle-ttl-secs <n>` | reap after `n` seconds with no output |
///
/// **A bare `--ephemeral` is refused** — see [`EPHEMERAL_NEEDS_AN_EXPLICIT_RULE`]
/// for the measurement behind that. A TTL-only declaration (`--ephemeral
/// --ephemeral-idle-ttl-secs <n>`) names no owner at all: `owner_pid` is `None`
/// and [`ephemeral_reap_reason`] never consults owner liveness for it.
///
/// The ephemeral rule flags are refused without `--ephemeral`, so a typo cannot
/// leave a caller believing it armed a rule it did not. Every flag is read
/// through the ONE argv rule ([`yggterm_core::cli_flag_value`]), so
/// `--ephemeral-owner-pid=4242` and `--ephemeral-owner-pid 4242` are the same
/// declaration; a parser that honoured only the spaced form silently discarded
/// the inline one and fell back to the default that no longer exists.
pub fn create_terminal_tenancy_from_args(
    args: &[String],
    creator_pid: u32,
    host: &str,
) -> Result<CreateTerminalTenancy, String> {
    let ephemeral_requested = args.iter().any(|arg| arg == "--ephemeral");
    let owner_pid = cli_flag_value(args, "--ephemeral-owner-pid")
        .map(|raw| {
            raw.parse::<u32>()
                .map_err(|_| format!("--ephemeral-owner-pid expects a pid, got {raw:?}"))
        })
        .transpose()?;
    let idle_ttl_secs = cli_flag_value(args, "--ephemeral-idle-ttl-secs")
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| format!("--ephemeral-idle-ttl-secs expects seconds, got {raw:?}"))
                .and_then(|secs| {
                    (secs > 0)
                        .then_some(secs)
                        .ok_or_else(|| "--ephemeral-idle-ttl-secs must be above zero".to_string())
                })
        })
        .transpose()?;
    if !ephemeral_requested && (owner_pid.is_some() || idle_ttl_secs.is_some()) {
        return Err(
            "--ephemeral-owner-pid / --ephemeral-idle-ttl-secs need --ephemeral".to_string(),
        );
    }
    let ephemeral =
        ephemeral_requested.then(|| EphemeralDeclaration::new(owner_pid, host, idle_ttl_secs));
    // A declaration that can never fire is a declaration the caller did not
    // finish making — read off the ONE predicate that decides that, so the
    // refusal and the reap decision cannot drift apart.
    if ephemeral
        .as_ref()
        .is_some_and(|declaration| !declaration.declares_a_rule())
    {
        return Err(EPHEMERAL_NEEDS_AN_EXPLICIT_RULE.to_string());
    }
    Ok(CreateTerminalTenancy {
        created_by: CreatorStamp::new(creator_pid, host, cli_flag_value(args, "--purpose")),
        ephemeral,
    })
}

/// The ONE place either binary turns its argv into a tenancy declaration.
/// `yggterm` and `yggterm-headless` are both the agent CLI and the flags must
/// mean the same thing on either, so neither binary carries a copy of this.
pub fn agent_cli_create_terminal_tenancy(args: &[String]) -> anyhow::Result<CreateTerminalTenancy> {
    create_terminal_tenancy_from_args(args, std::process::id(), &local_host_token())
        .map_err(|message| anyhow::anyhow!(message))
}

/// What this daemon can honestly say about the declared owner process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerLiveness {
    Alive,
    Gone,
    /// The owner lives on a host this daemon cannot see, or the declaration
    /// named no owner. NEVER reaps.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EphemeralReapReason {
    OwnerGone,
    IdleTtl,
}

impl EphemeralReapReason {
    /// The trace event name. These two strings are the contract with the
    /// operator reading `server trace`.
    pub fn trace_name(self) -> &'static str {
        match self {
            Self::OwnerGone => "ephemeral_owner_gone",
            Self::IdleTtl => "ephemeral_idle_ttl",
        }
    }
}

/// One row the reap pass is allowed to consider: it carries a declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EphemeralCandidate {
    pub session_path: String,
    pub declaration: EphemeralDeclaration,
    /// Seconds since this row's PTY last produced output. `None` when the
    /// daemon holds no runtime for the row and therefore cannot say.
    pub idle_secs: Option<u64>,
    /// Whether the row's own runtime is still running. A row whose runtime
    /// already exited is left to the ordinary paths.
    pub running: bool,
}

/// THE reap decision. Pure, so the ruling is one readable expression and every
/// clause can be mutated to prove its own lock.
///
/// **Keep-alive is not an input here, and that is not an override.** Read off
/// the shipped code: keep-alive governs whether a runtime survives the GUI
/// WINDOW closing (`PrepareClientClose` closes `non_keep_alive_live_session_paths`
/// and keeps the rest). It has never governed an explicit close —
/// `remove_session_should_detach_keep_alive_runtime` is a constant `false`, so
/// `RemoveSession` destroys a keep-alive row exactly like any other. A reap is
/// an explicit close, so a declared-ephemeral row is reaped whether or not it is
/// marked keep-alive; there is no keep-alive branch to skip. The user is
/// protected by a different rule entirely — the flag exists only on the agent
/// CLI create path, so a row the user made carries no declaration and never
/// reaches this function at all.
///
/// **A declaration that names no owner never asks about one.** A TTL-only
/// declaration is `owner_pid: None`, and no liveness verdict — not even `Gone`
/// — may reap it: the caller declared a silence rule and nothing else.
pub fn ephemeral_reap_reason(
    candidate: &EphemeralCandidate,
    owner: OwnerLiveness,
) -> Option<EphemeralReapReason> {
    if !candidate.running {
        return None;
    }
    if candidate.declaration.owner_pid.is_some() && owner == OwnerLiveness::Gone {
        return Some(EphemeralReapReason::OwnerGone);
    }
    let ttl = candidate.declaration.idle_ttl_secs?;
    let idle_secs = candidate.idle_secs?;
    (idle_secs >= ttl).then_some(EphemeralReapReason::IdleTtl)
}

/// The world the reap pass acts on. The daemon runtime is un-mockable, so the
/// pass takes its whole world through this trait and a test asserts on what the
/// pass DID rather than on what a helper returned (field guide §7.1).
pub trait EphemeralReapHost {
    fn candidates(&self) -> Vec<EphemeralCandidate>;
    fn owner_liveness(&self, declaration: &EphemeralDeclaration) -> OwnerLiveness;
    /// Close the row GRACEFULLY through the daemon's one close path — the same
    /// tombstone-then-remove the user's own close takes.
    fn close_row(&mut self, session_path: &str, reason: EphemeralReapReason);
    fn trace(&self, session_path: &str, reason: EphemeralReapReason, idle_secs: Option<u64>);
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EphemeralReapOutcome {
    pub considered: usize,
    pub reaped: Vec<(String, EphemeralReapReason)>,
}

impl EphemeralReapOutcome {
    pub fn did_anything(&self) -> bool {
        !self.reaped.is_empty()
    }
}

/// One pass over the declared-ephemeral rows. Called from an EXISTING daemon
/// chore tick — this module adds no timer of its own.
pub fn ephemeral_session_reap_pass<H: EphemeralReapHost>(host: &mut H) -> EphemeralReapOutcome {
    let candidates = host.candidates();
    let mut outcome = EphemeralReapOutcome {
        considered: candidates.len(),
        reaped: Vec::new(),
    };
    for candidate in candidates {
        let owner = host.owner_liveness(&candidate.declaration);
        let Some(reason) = ephemeral_reap_reason(&candidate, owner) else {
            continue;
        };
        host.trace(&candidate.session_path, reason, candidate.idle_secs);
        host.close_row(&candidate.session_path, reason);
        outcome.reaped.push((candidate.session_path, reason));
    }
    outcome
}

// ---------------------------------------------------------------------------
// Piece 1 — per-row tenant cost visibility
// ---------------------------------------------------------------------------

/// One process read out of `/proc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcEntry {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    pub comm: String,
    pub cmdline: String,
    /// utime + stime, in clock ticks.
    pub cpu_ticks: u64,
    /// Field 22 of `/proc/<pid>/stat`, in clock ticks since boot.
    pub start_ticks: u64,
}

/// A whole-`/proc` reading. Built ONCE per request and shared by every row, so
/// the cost of the verb does not scale with the row count.
#[derive(Debug, Clone)]
pub struct ProcSnapshot {
    entries: BTreeMap<u32, ProcEntry>,
    children: BTreeMap<u32, Vec<u32>>,
    clock_ticks_per_sec: u64,
    uptime_secs: u64,
}

impl ProcSnapshot {
    pub fn new(entries: Vec<ProcEntry>, clock_ticks_per_sec: u64, uptime_secs: u64) -> Self {
        let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for entry in &entries {
            children.entry(entry.ppid).or_default().push(entry.pid);
        }
        Self {
            entries: entries
                .into_iter()
                .map(|entry| (entry.pid, entry))
                .collect(),
            children,
            clock_ticks_per_sec: clock_ticks_per_sec.max(1),
            uptime_secs,
        }
    }

    pub fn get(&self, pid: u32) -> Option<&ProcEntry> {
        self.entries.get(&pid)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn cpu_seconds(&self, ticks: u64) -> f64 {
        ticks as f64 / self.clock_ticks_per_sec as f64
    }

    fn age_secs(&self, entry: &ProcEntry) -> u64 {
        let started_secs = entry.start_ticks / self.clock_ticks_per_sec;
        self.uptime_secs.saturating_sub(started_secs)
    }

    /// Strict descendants of `root`, breadth first, cycle-safe.
    ///
    /// RESIDUAL (round-25 review, unbuilt): a tenant that REPARENTS away — a
    /// daemonised child, or anything orphaned to pid 1 — leaves this walk and
    /// stops being counted, while it is still very much running and still very
    /// much the row's fault. The honest fix is a second reading keyed on the
    /// PTY's session id (`/proc/<pid>/stat` field 6, the sid every process
    /// under the row inherits) unioned with this tree; it is deliberately not
    /// built here because it widens what the reaper's cost numbers mean, and
    /// this lane only measures. Until then an under-count is the failure
    /// direction — the verb never invents a tenant it cannot see.
    fn descendants(&self, root: u32) -> Vec<&ProcEntry> {
        let mut seen: HashSet<u32> = HashSet::from([root]);
        let mut queue: VecDeque<u32> = VecDeque::from([root]);
        let mut found = Vec::new();
        while let Some(pid) = queue.pop_front() {
            let Some(kids) = self.children.get(&pid) else {
                continue;
            };
            for kid in kids {
                if !seen.insert(*kid) {
                    continue;
                }
                if let Some(entry) = self.entries.get(kid) {
                    found.push(entry);
                }
                queue.push_back(*kid);
            }
        }
        found
    }
}

/// One process running inside a row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenantProcess {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
    pub cpu_seconds: f64,
    pub age_secs: u64,
    /// `true` when this process is the row's own shell rather than a tenant
    /// workload. Reported, not hidden — the caller can see what was discounted.
    pub is_shell: bool,
}

/// Why a row could not be measured. Named reasons only; a row that could not be
/// walked reports one of these AND leaves every number empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantReportGap {
    NotSupportedOnPlatform,
    ProcUnreadable,
    NoLocalRuntime,
    /// The row's runtime could not be reached to be measured. **Names the
    /// CONDITION, never the topology.** The user must never have to learn that
    /// more than one daemon exists (the constitution), so a row whose PTY lives
    /// in another daemon is PROXIED to it and merged — this reason is left for
    /// the case where that owner genuinely cannot answer.
    RuntimeUnreachable,
    RuntimeNotRunning,
    RootPidUnavailable,
    RootPidNotInProc,
}

impl TenantReportGap {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotSupportedOnPlatform => "not_supported_on_platform",
            Self::ProcUnreadable => "proc_unreadable",
            Self::NoLocalRuntime => "no_local_runtime",
            Self::RuntimeUnreachable => "runtime_unreachable",
            Self::RuntimeNotRunning => "runtime_not_running",
            Self::RootPidUnavailable => "root_pid_unavailable",
            Self::RootPidNotInProc => "root_pid_not_in_proc",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::NotSupportedOnPlatform => {
                "process accounting needs /proc; this platform has none"
            }
            Self::ProcUnreadable => "/proc could not be read on this host",
            Self::NoLocalRuntime => "no runtime is registered for this row",
            Self::RuntimeUnreachable => "the row's runtime did not answer; retry in a moment",
            Self::RuntimeNotRunning => "the row's runtime process has exited",
            Self::RootPidUnavailable => "the runtime reports no process id",
            Self::RootPidNotInProc => "the runtime's process id is no longer in /proc",
        }
    }
}

/// What one row costs, or a named reason why that is not knowable here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowTenantReport {
    pub session_path: String,
    pub runtime_key: String,
    /// The reason this row could not be measured. `None` = the numbers below
    /// are real.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_pgid: Option<u32>,
    /// The command the PTY's foreground process group is running — the thing
    /// the user would see if they clicked this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_count: Option<usize>,
    /// utime+stime summed over the row's whole process tree, root included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_cpu_seconds: Option<f64>,
    /// Age of the oldest NON-SHELL descendant — "how long has something been
    /// squatting in this row".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_tenant_age_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_tenant_command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenants: Vec<TenantProcess>,
    /// Set when `tenants` was capped; the counts above are still complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenants_listed_of: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<CreatorStamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<EphemeralDeclaration>,
}

impl RowTenantReport {
    /// A row that cannot be measured. Every number stays empty on purpose: a
    /// zero here would read as "this row is cheap", and inventing that is the
    /// exact dishonesty this verb exists to remove.
    pub fn unavailable(session_path: &str, runtime_key: &str, gap: TenantReportGap) -> Self {
        Self {
            session_path: session_path.to_string(),
            runtime_key: runtime_key.to_string(),
            unavailable_reason: Some(gap.reason().to_string()),
            unavailable_detail: Some(gap.detail().to_string()),
            root_pid: None,
            foreground_pgid: None,
            foreground_command: None,
            tenant_count: None,
            tree_cpu_seconds: None,
            oldest_tenant_age_secs: None,
            oldest_tenant_command: None,
            tenants: Vec::new(),
            tenants_listed_of: None,
            created_by: None,
            ephemeral: None,
        }
    }
}

/// Walk one row's process tree out of an already-taken `/proc` snapshot.
pub fn row_tenant_report(
    snapshot: &ProcSnapshot,
    session_path: &str,
    runtime_key: &str,
    root_pid: u32,
    foreground_pgid: Option<u32>,
) -> RowTenantReport {
    if snapshot.get(root_pid).is_none() {
        return RowTenantReport::unavailable(
            session_path,
            runtime_key,
            TenantReportGap::RootPidNotInProc,
        );
    }
    let root = snapshot.get(root_pid).expect("root checked just above");
    let descendants = snapshot.descendants(root_pid);
    let tree_ticks = root.cpu_ticks + descendants.iter().map(|entry| entry.cpu_ticks).sum::<u64>();

    let mut tenants: Vec<TenantProcess> = descendants
        .iter()
        .map(|entry| TenantProcess {
            pid: entry.pid,
            ppid: entry.ppid,
            command: display_command(entry),
            cpu_seconds: snapshot.cpu_seconds(entry.cpu_ticks),
            age_secs: snapshot.age_secs(entry),
            is_shell: command_is_shell(&entry.comm),
        })
        .collect();

    let oldest = tenants
        .iter()
        .filter(|tenant| !tenant.is_shell)
        .max_by_key(|tenant| tenant.age_secs)
        .cloned();

    tenants.sort_by(|left, right| {
        right
            .cpu_seconds
            .partial_cmp(&left.cpu_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.age_secs.cmp(&left.age_secs))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    let tenant_count = tenants.len();
    let tenants_listed_of = (tenant_count > MAX_LISTED_TENANTS).then_some(tenant_count);
    tenants.truncate(MAX_LISTED_TENANTS);

    let foreground_command = foreground_pgid
        .and_then(|pgid| snapshot.get(pgid))
        .map(display_command);

    RowTenantReport {
        session_path: session_path.to_string(),
        runtime_key: runtime_key.to_string(),
        unavailable_reason: None,
        unavailable_detail: None,
        root_pid: Some(root_pid),
        foreground_pgid,
        foreground_command,
        tenant_count: Some(tenant_count),
        tree_cpu_seconds: Some(snapshot.cpu_seconds(tree_ticks)),
        oldest_tenant_age_secs: oldest.as_ref().map(|tenant| tenant.age_secs),
        oldest_tenant_command: oldest.map(|tenant| tenant.command),
        tenants,
        tenants_listed_of,
        created_by: None,
        ephemeral: None,
    }
}

fn display_command(entry: &ProcEntry) -> String {
    if entry.cmdline.trim().is_empty() {
        entry.comm.clone()
    } else {
        entry.cmdline.clone()
    }
}

fn command_is_shell(comm: &str) -> bool {
    let name = comm.trim_start_matches('-');
    SHELL_COMMAND_NAMES
        .iter()
        .any(|shell| name.eq_ignore_ascii_case(shell))
}

/// Parse one `/proc/<pid>/stat` line.
///
/// The comm field is parenthesised and may itself contain spaces AND
/// parentheses (`(tmux: server)`, `(a) b)`), so the only safe split is at the
/// LAST `)`. Everything after that is field 3 onwards, which is why the offsets
/// below are `field - 3`.
pub fn parse_proc_stat_line(line: &str) -> Option<ProcEntry> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close < open {
        return None;
    }
    let pid = line[..open].trim().parse::<u32>().ok()?;
    let comm = line[open + 1..close].to_string();
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    Some(ProcEntry {
        pid,
        ppid: rest.get(1)?.parse().ok()?,
        pgid: rest.get(2)?.parse().ok()?,
        comm,
        cmdline: String::new(),
        cpu_ticks: rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?,
        start_ticks: rest.get(19)?.parse().ok()?,
    })
}

/// `/proc/<pid>/cmdline` is NUL-separated; an empty one means a kernel thread
/// (or a process that scrubbed its argv), in which case the caller falls back
/// to `comm`.
pub fn decode_proc_cmdline(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    text.split('\0')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Seconds since boot, out of the first field of `/proc/uptime`.
pub fn parse_proc_uptime(raw: &str) -> Option<u64> {
    raw.split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .map(|secs| secs as u64)
}

#[cfg(unix)]
pub fn read_proc_snapshot() -> Result<ProcSnapshot, TenantReportGap> {
    let uptime_raw =
        std::fs::read_to_string("/proc/uptime").map_err(|_| TenantReportGap::ProcUnreadable)?;
    let uptime_secs = parse_proc_uptime(&uptime_raw).ok_or(TenantReportGap::ProcUnreadable)?;
    let clock_ticks_per_sec = clock_ticks_per_sec();
    let dir = std::fs::read_dir("/proc").map_err(|_| TenantReportGap::ProcUnreadable)?;
    let mut entries = Vec::new();
    for item in dir.flatten() {
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.parse::<u32>().is_err() {
            continue;
        }
        // A process can exit between the readdir and the read; that is not a
        // degradation of the whole snapshot, it is one fewer tenant.
        let Ok(stat) = std::fs::read_to_string(item.path().join("stat")) else {
            continue;
        };
        let Some(mut entry) = parse_proc_stat_line(&stat) else {
            continue;
        };
        if let Ok(raw) = std::fs::read(item.path().join("cmdline")) {
            entry.cmdline = decode_proc_cmdline(&raw);
        }
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(TenantReportGap::ProcUnreadable);
    }
    Ok(ProcSnapshot::new(entries, clock_ticks_per_sec, uptime_secs))
}

#[cfg(not(unix))]
pub fn read_proc_snapshot() -> Result<ProcSnapshot, TenantReportGap> {
    Err(TenantReportGap::NotSupportedOnPlatform)
}

#[cfg(unix)]
fn clock_ticks_per_sec() -> u64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 { ticks as u64 } else { 100 }
}

/// Is the declared owner still around?
///
/// Deliberately biased AGAINST reaping: pid reuse can make a dead owner look
/// alive, which keeps a row that should have gone; the reverse would destroy a
/// row whose owner is working. Only a host that matches the declaration answers
/// at all.
///
/// **The unknown-host sentinel is never a match.** [`local_host_token`] falls
/// back to [`UNKNOWN_HOST_TOKEN`] when this machine cannot name itself, and
/// [`sanitize_token`] writes the same string for an empty one — so two
/// different machines can both carry it, and matching them would let daemon A
/// answer "gone" about a pid that only ever existed on machine B. A row is
/// worth more than a reap.
#[cfg(unix)]
pub fn owner_liveness_on_host(
    declaration: &EphemeralDeclaration,
    local_host: &str,
) -> OwnerLiveness {
    let Some(owner_pid) = declaration.owner_pid else {
        return OwnerLiveness::Unknown;
    };
    let local_host = sanitize_token(local_host);
    if local_host == UNKNOWN_HOST_TOKEN || declaration.owner_host == UNKNOWN_HOST_TOKEN {
        return OwnerLiveness::Unknown;
    }
    if declaration.owner_host != local_host {
        return OwnerLiveness::Unknown;
    }
    if std::path::Path::new(&format!("/proc/{owner_pid}")).exists() {
        OwnerLiveness::Alive
    } else {
        OwnerLiveness::Gone
    }
}

#[cfg(not(unix))]
pub fn owner_liveness_on_host(
    _declaration: &EphemeralDeclaration,
    _local_host: &str,
) -> OwnerLiveness {
    OwnerLiveness::Unknown
}

/// The machine identity a stamp is scoped to. Same reading as the attach
/// banner's, so a stamp written by one component matches one read by another.
pub fn local_host_token() -> String {
    let raw = std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|value| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| UNKNOWN_HOST_TOKEN.to_string());
    sanitize_token(&raw)
}

fn sanitize_token(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|ch| if ch.is_whitespace() { '-' } else { ch })
        .collect();
    if cleaned.is_empty() {
        UNKNOWN_HOST_TOKEN.to_string()
    } else {
        cleaned
    }
}

fn sanitize_free_text(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_PURPOSE_CHARS).collect()
}

/// `head`, plus the text after the FIRST occurrence of `marker` if there is one.
fn split_trailing_free_text<'a>(value: &'a str, marker: &str) -> (&'a str, Option<String>) {
    match value.find(marker) {
        Some(at) => {
            let tail = value[at + marker.len()..].trim();
            (&value[..at], (!tail.is_empty()).then(|| tail.to_string()))
        }
        None => (value, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(owner_pid: Option<u32>, ttl: Option<u64>) -> EphemeralDeclaration {
        EphemeralDeclaration::new(owner_pid, "host-a", ttl)
    }

    fn candidate(
        declaration: EphemeralDeclaration,
        idle_secs: Option<u64>,
        running: bool,
    ) -> EphemeralCandidate {
        EphemeralCandidate {
            session_path: "local://row".to_string(),
            declaration,
            idle_secs,
            running,
        }
    }

    #[test]
    fn creator_stamp_round_trips_including_a_spaced_purpose() {
        let stamp = CreatorStamp::new(4242, "host-a", Some("  probe   the   queue \n"));
        assert_eq!(stamp.purpose.as_deref(), Some("probe the queue"));
        let parsed = CreatorStamp::parse(&stamp.encode()).expect("stamp parses back");
        assert_eq!(parsed, stamp);

        let bare = CreatorStamp::new(7, "host-b", None);
        assert_eq!(CreatorStamp::parse(&bare.encode()), Some(bare));
    }

    #[test]
    fn creator_stamp_refuses_a_value_missing_its_identity() {
        assert!(CreatorStamp::parse("host=host-a purpose=x").is_none());
        assert!(CreatorStamp::parse("pid=12").is_none());
        assert!(CreatorStamp::parse("").is_none());
    }

    #[test]
    fn ephemeral_declaration_round_trips_every_rule_combination() {
        for owner_pid in [None, Some(99u32)] {
            for ttl in [None, Some(1_800u64)] {
                let declared = declaration(owner_pid, ttl);
                let parsed =
                    EphemeralDeclaration::parse(&declared.encode()).expect("declaration parses");
                assert_eq!(parsed, declared, "round trip for {owner_pid:?}/{ttl:?}");
            }
        }
        assert!(EphemeralDeclaration::parse("idle-ttl-secs=5").is_none());
    }

    #[test]
    fn a_zero_idle_ttl_is_not_a_rule() {
        let declared = EphemeralDeclaration::new(None, "host-a", Some(0));
        assert_eq!(declared.idle_ttl_secs, None);
        assert!(!declared.declares_a_rule());
        assert!(declaration(Some(3), None).declares_a_rule());
        assert!(declaration(None, Some(3)).declares_a_rule());
    }

    #[test]
    fn a_gone_owner_reaps_and_a_live_one_does_not() {
        let candidate = candidate(declaration(Some(11), None), Some(0), true);
        assert_eq!(
            ephemeral_reap_reason(&candidate, OwnerLiveness::Gone),
            Some(EphemeralReapReason::OwnerGone)
        );
        assert_eq!(
            ephemeral_reap_reason(&candidate, OwnerLiveness::Alive),
            None
        );
    }

    #[test]
    fn an_unknown_owner_never_reaps() {
        let candidate = candidate(declaration(Some(11), None), Some(999_999), true);
        assert_eq!(
            ephemeral_reap_reason(&candidate, OwnerLiveness::Unknown),
            None,
            "a daemon that cannot see the owner must not guess it is dead"
        );
    }

    /// THE TTL-ONLY LOCK (round-25 review, finding P0). A declaration that
    /// names no owner must never be reaped by an owner verdict — not even
    /// `Gone`, which is what a daemon answers about a pid that was never
    /// declared. This is the shape a caller is left with once a bare
    /// `--ephemeral` is refused, so if it can be owner-reaped the refusal
    /// pushed callers from one footgun onto another.
    #[test]
    fn a_ttl_only_declaration_is_never_reaped_by_an_owner_verdict() {
        let ttl_only = declaration(None, Some(3_600));
        assert_eq!(ttl_only.owner_pid, None);
        for owner in [
            OwnerLiveness::Gone,
            OwnerLiveness::Alive,
            OwnerLiveness::Unknown,
        ] {
            assert_eq!(
                ephemeral_reap_reason(&candidate(ttl_only.clone(), Some(10), true), owner),
                None,
                "a TTL-only row must wait out its TTL, whatever {owner:?} says about an \
                 owner it never named"
            );
        }
        assert_eq!(
            ephemeral_reap_reason(&candidate(ttl_only, Some(3_600), true), OwnerLiveness::Gone),
            Some(EphemeralReapReason::IdleTtl),
            "and when it does fire, it fires as the rule the caller actually declared"
        );
    }

    #[test]
    fn the_idle_ttl_fires_only_at_or_past_the_declared_ttl() {
        let declared = declaration(None, Some(60));
        assert_eq!(
            ephemeral_reap_reason(
                &candidate(declared.clone(), Some(59), true),
                OwnerLiveness::Unknown
            ),
            None
        );
        assert_eq!(
            ephemeral_reap_reason(
                &candidate(declared.clone(), Some(60), true),
                OwnerLiveness::Unknown
            ),
            Some(EphemeralReapReason::IdleTtl)
        );
        assert_eq!(
            ephemeral_reap_reason(&candidate(declared, None, true), OwnerLiveness::Unknown),
            None,
            "a row whose idle time is unknown cannot be idle-reaped"
        );
    }

    #[test]
    fn a_declaration_with_no_rule_can_never_reap() {
        let declared = EphemeralDeclaration::new(None, "host-a", None);
        assert_eq!(
            ephemeral_reap_reason(
                &candidate(declared, Some(u64::MAX), true),
                OwnerLiveness::Unknown
            ),
            None
        );
    }

    #[test]
    fn an_already_exited_runtime_is_left_alone() {
        assert_eq!(
            ephemeral_reap_reason(
                &candidate(declaration(Some(11), Some(1)), Some(9_999), false),
                OwnerLiveness::Gone
            ),
            None
        );
    }

    #[test]
    fn reap_reason_trace_names_are_the_documented_ones() {
        assert_eq!(
            EphemeralReapReason::OwnerGone.trace_name(),
            "ephemeral_owner_gone"
        );
        assert_eq!(
            EphemeralReapReason::IdleTtl.trace_name(),
            "ephemeral_idle_ttl"
        );
    }

    struct FakeReapHost {
        candidates: Vec<EphemeralCandidate>,
        liveness: OwnerLiveness,
        closed: Vec<(String, EphemeralReapReason)>,
        traced: std::cell::RefCell<Vec<(String, EphemeralReapReason)>>,
    }

    impl FakeReapHost {
        fn with(candidates: Vec<EphemeralCandidate>, liveness: OwnerLiveness) -> Self {
            Self {
                candidates,
                liveness,
                closed: Vec::new(),
                traced: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl EphemeralReapHost for FakeReapHost {
        fn candidates(&self) -> Vec<EphemeralCandidate> {
            self.candidates.clone()
        }
        fn owner_liveness(&self, _declaration: &EphemeralDeclaration) -> OwnerLiveness {
            self.liveness
        }
        fn close_row(&mut self, session_path: &str, reason: EphemeralReapReason) {
            self.closed.push((session_path.to_string(), reason));
        }
        fn trace(&self, session_path: &str, reason: EphemeralReapReason, _idle: Option<u64>) {
            self.traced
                .borrow_mut()
                .push((session_path.to_string(), reason));
        }
    }

    fn row(path: &str, declared: EphemeralDeclaration, idle: Option<u64>) -> EphemeralCandidate {
        EphemeralCandidate {
            session_path: path.to_string(),
            declaration: declared,
            idle_secs: idle,
            running: true,
        }
    }

    #[test]
    fn the_pass_closes_only_the_rows_whose_rule_fired_and_traces_each() {
        let mut host = FakeReapHost::with(
            vec![
                row("local://ripe", declaration(None, Some(30)), Some(31)),
                row("local://fresh", declaration(None, Some(30)), Some(2)),
            ],
            OwnerLiveness::Unknown,
        );
        let outcome = ephemeral_session_reap_pass(&mut host);
        assert_eq!(outcome.considered, 2);
        assert_eq!(
            outcome.reaped,
            vec![("local://ripe".to_string(), EphemeralReapReason::IdleTtl)]
        );
        assert_eq!(
            host.closed,
            vec![("local://ripe".to_string(), EphemeralReapReason::IdleTtl)]
        );
        assert_eq!(
            host.traced.borrow().as_slice(),
            &[("local://ripe".to_string(), EphemeralReapReason::IdleTtl)],
            "every reap must be traceable to the rule that caused it"
        );
        assert!(outcome.did_anything());
    }

    #[test]
    fn a_pass_over_no_declared_rows_closes_nothing() {
        let mut host = FakeReapHost::with(Vec::new(), OwnerLiveness::Unknown);
        let outcome = ephemeral_session_reap_pass(&mut host);
        assert_eq!(outcome, EphemeralReapOutcome::default());
        assert!(host.closed.is_empty());
        assert!(!outcome.did_anything());
    }

    fn proc(pid: u32, ppid: u32, comm: &str, cpu_ticks: u64, start_ticks: u64) -> ProcEntry {
        ProcEntry {
            pid,
            ppid,
            pgid: pid,
            comm: comm.to_string(),
            cmdline: format!("{comm} --flag"),
            cpu_ticks,
            start_ticks,
        }
    }

    /// A shell row with an abandoned remote probe under it — the shape the
    /// whole module exists for. The long-lived subshell (500) is older than the
    /// probe on purpose: it is what the shell discount has to step over.
    fn abandoned_probe_snapshot() -> ProcSnapshot {
        ProcSnapshot::new(
            vec![
                proc(100, 1, "bash", 10, 100_000),
                proc(200, 100, "ssh", 50, 20_000),
                proc(300, 200, "sh", 1, 20_500),
                proc(400, 100, "grep", 2, 990_000),
                proc(500, 100, "bash", 0, 5_000),
            ],
            100,
            10_000,
        )
    }

    #[test]
    fn a_row_report_sums_the_whole_tree_and_ages_the_oldest_non_shell_tenant() {
        let snapshot = abandoned_probe_snapshot();
        let report = row_tenant_report(&snapshot, "local://row", "local://row", 100, Some(200));
        assert_eq!(report.unavailable_reason, None);
        assert_eq!(report.tenant_count, Some(4));
        // (10 + 50 + 1 + 2 + 0) ticks at 100 Hz.
        assert_eq!(report.tree_cpu_seconds, Some(0.63));
        assert_eq!(report.foreground_command.as_deref(), Some("ssh --flag"));
        // uptime 10_000 s minus the ssh start at 200 s.
        assert_eq!(report.oldest_tenant_age_secs, Some(9_800));
        assert_eq!(report.oldest_tenant_command.as_deref(), Some("ssh --flag"));
        assert_eq!(
            report.tenants.first().map(|tenant| tenant.pid),
            Some(200),
            "the costliest tenant leads the list"
        );
    }

    #[test]
    fn the_rows_own_shell_is_never_the_oldest_tenant() {
        // The shell under the ssh is older than the grep, but it is a shell.
        let snapshot = abandoned_probe_snapshot();
        let report = row_tenant_report(&snapshot, "local://row", "local://row", 100, None);
        let shell_ages: Vec<u64> = report
            .tenants
            .iter()
            .filter(|tenant| tenant.is_shell)
            .map(|tenant| tenant.age_secs)
            .collect();
        assert!(
            shell_ages
                .iter()
                .any(|age| *age > report.oldest_tenant_age_secs.expect("a tenant age")),
            "the discounted shell really is older, so the filter is doing work"
        );
        assert_eq!(report.foreground_command, None);
        assert_eq!(report.foreground_pgid, None);
    }

    #[test]
    fn a_row_with_no_tenants_reports_zero_tenants_and_no_tenant_age() {
        let snapshot = ProcSnapshot::new(vec![proc(100, 1, "bash", 10, 100_000)], 100, 10_000);
        let report = row_tenant_report(&snapshot, "local://row", "local://row", 100, Some(100));
        assert_eq!(report.tenant_count, Some(0));
        assert_eq!(report.oldest_tenant_age_secs, None);
        assert_eq!(report.tree_cpu_seconds, Some(0.1));
    }

    #[test]
    fn a_root_that_left_proc_degrades_instead_of_reporting_zeros() {
        let snapshot = ProcSnapshot::new(vec![proc(100, 1, "bash", 10, 0)], 100, 10_000);
        let report = row_tenant_report(&snapshot, "local://row", "local://row", 999, None);
        assert_eq!(
            report.unavailable_reason.as_deref(),
            Some("root_pid_not_in_proc")
        );
        assert_eq!(report.tree_cpu_seconds, None);
        assert_eq!(report.tenant_count, None);
        assert!(report.tenants.is_empty());
    }

    #[test]
    fn every_gap_names_itself_and_leaves_every_number_empty() {
        for gap in [
            TenantReportGap::NotSupportedOnPlatform,
            TenantReportGap::ProcUnreadable,
            TenantReportGap::NoLocalRuntime,
            TenantReportGap::RuntimeUnreachable,
            TenantReportGap::RuntimeNotRunning,
            TenantReportGap::RootPidUnavailable,
            TenantReportGap::RootPidNotInProc,
        ] {
            let report = RowTenantReport::unavailable("p", "r", gap);
            assert_eq!(report.unavailable_reason.as_deref(), Some(gap.reason()));
            assert!(report.unavailable_detail.is_some());
            assert_eq!(report.tree_cpu_seconds, None);
            assert_eq!(report.tenant_count, None);
            assert_eq!(report.oldest_tenant_age_secs, None);
            assert_eq!(report.root_pid, None);
            assert!(!gap.reason().is_empty());
        }
    }

    /// THE CONSTITUTION LOCK on this verb's vocabulary: the user must never
    /// have to learn that more than one daemon exists, so a gap may name the
    /// CONDITION ("the runtime did not answer") and never the topology ("ask
    /// the other daemon"). A row on a preserved owner is proxied and merged;
    /// what is left is an honest "could not reach it".
    #[test]
    fn no_gap_leaks_the_daemon_topology_to_the_person_reading_it() {
        for gap in [
            TenantReportGap::NotSupportedOnPlatform,
            TenantReportGap::ProcUnreadable,
            TenantReportGap::NoLocalRuntime,
            TenantReportGap::RuntimeUnreachable,
            TenantReportGap::RuntimeNotRunning,
            TenantReportGap::RootPidUnavailable,
            TenantReportGap::RootPidNotInProc,
        ] {
            for text in [gap.reason(), gap.detail()] {
                let text = text.to_ascii_lowercase();
                for leak in [
                    "daemon",
                    "preserved_owner",
                    "preserved owner",
                    "owner daemon",
                ] {
                    assert!(
                        !text.contains(leak),
                        "{:?} says {text:?}, which tells the user about {leak}",
                        gap
                    );
                }
            }
        }
    }

    #[test]
    fn the_tenant_list_is_capped_but_the_count_is_not() {
        let mut entries = vec![proc(1_000, 1, "bash", 0, 0)];
        for pid in 0..(MAX_LISTED_TENANTS as u32 + 5) {
            entries.push(proc(2_000 + pid, 1_000, "worker", pid as u64, 0));
        }
        let snapshot = ProcSnapshot::new(entries, 100, 10_000);
        let report = row_tenant_report(&snapshot, "p", "r", 1_000, None);
        assert_eq!(report.tenant_count, Some(MAX_LISTED_TENANTS + 5));
        assert_eq!(report.tenants.len(), MAX_LISTED_TENANTS);
        assert_eq!(report.tenants_listed_of, Some(MAX_LISTED_TENANTS + 5));
    }

    #[test]
    fn a_descendant_cycle_cannot_hang_the_walk() {
        let snapshot = ProcSnapshot::new(
            vec![
                proc(10, 1, "bash", 1, 0),
                proc(20, 10, "a", 1, 0),
                proc(30, 20, "b", 1, 0),
                // A ppid loop cannot happen on a healthy kernel; the walk must
                // still terminate if /proc is read mid-reparent.
                ProcEntry {
                    ppid: 30,
                    ..proc(20, 30, "a", 1, 0)
                },
            ],
            100,
            10_000,
        );
        assert_eq!(
            row_tenant_report(&snapshot, "p", "r", 10, None).tenant_count,
            Some(2)
        );
    }

    #[test]
    fn a_proc_stat_line_splits_at_the_last_paren_not_the_first() {
        let line = "4242 (weird (name) proc) S 4200 4100 4100 34816 4242 4194304 \
                    100 200 0 0 700 800 0 0 20 0 1 0 123456 5 6 7 8";
        let entry = parse_proc_stat_line(line).expect("a parsed stat line");
        assert_eq!(entry.pid, 4242);
        assert_eq!(entry.comm, "weird (name) proc");
        assert_eq!(entry.ppid, 4200);
        assert_eq!(entry.pgid, 4100);
        assert_eq!(entry.cpu_ticks, 1_500);
        assert_eq!(entry.start_ticks, 123_456);
    }

    #[test]
    fn a_truncated_stat_line_is_refused_rather_than_guessed() {
        assert!(parse_proc_stat_line("4242 (bash) S 1 1").is_none());
        assert!(parse_proc_stat_line("not a stat line").is_none());
    }

    #[test]
    fn a_nul_separated_cmdline_becomes_one_readable_command() {
        assert_eq!(decode_proc_cmdline(b"ssh\0-t\0host\0"), "ssh -t host");
        assert_eq!(decode_proc_cmdline(b""), "");
    }

    #[test]
    fn uptime_reads_the_first_field_only() {
        assert_eq!(parse_proc_uptime("123456.78 987654.32\n"), Some(123_456));
        assert_eq!(parse_proc_uptime(""), None);
    }

    #[test]
    fn a_command_named_like_a_login_shell_counts_as_the_rows_own_shell() {
        assert!(command_is_shell("bash"));
        assert!(command_is_shell("-zsh"));
        assert!(!command_is_shell("ssh"));
        assert!(!command_is_shell("htop"));
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    /// THE DOCTRINE LOCK at the flag parser: opting in takes an explicit
    /// `--ephemeral`. Everything else — including a create that names a
    /// purpose — produces a row the reaper can never see, because the reap
    /// pass's only candidates are rows carrying a declaration.
    #[test]
    fn only_an_explicit_ephemeral_flag_arms_the_reaper() {
        let plain = create_terminal_tenancy_from_args(
            &argv(&["new", "--kind", "shell", "--purpose", "probe"]),
            11,
            "host-a",
        )
        .expect("a create with no ephemeral flag is valid");
        assert_eq!(plain.ephemeral, None, "no flag, no reap rule");
        assert_eq!(plain.created_by.pid, 11);
        assert_eq!(plain.created_by.purpose.as_deref(), Some("probe"));

        let declared = create_terminal_tenancy_from_args(
            &argv(&["new", "--ephemeral", "--ephemeral-owner-pid", "4242"]),
            11,
            "host-a",
        )
        .expect("an ephemeral create naming its owner is valid");
        assert_eq!(
            declared.ephemeral,
            Some(EphemeralDeclaration::new(Some(4242), "host-a", None)),
        );
    }

    /// THE OWNER-DEFAULT LOCK (round-25 review, finding P0). A bare
    /// `--ephemeral` used to default the owner to the caller's PARENT, which
    /// under `bash -c` is the wrapper shell and under `ssh host "<cli>"` is
    /// sshd-session — both already dead when the first chore tick runs. Arming
    /// owner-gone against a corpse reaps the row within the minute, so the
    /// only honest answer is to refuse and say what the two valid forms are.
    #[test]
    fn a_bare_ephemeral_is_refused_and_names_both_valid_forms() {
        let error = create_terminal_tenancy_from_args(&argv(&["new", "--ephemeral"]), 11, "host-a")
            .expect_err("a bare --ephemeral must be refused, never defaulted");
        assert_eq!(error, EPHEMERAL_NEEDS_AN_EXPLICIT_RULE);
        assert!(
            error.contains("--ephemeral-owner-pid") && error.contains("--ephemeral-idle-ttl-secs"),
            "the refusal must name both ways out, or it just blocks the caller"
        );

        // Both ways out work, and a TTL-only declaration names NO owner.
        let ttl_only = create_terminal_tenancy_from_args(
            &argv(&["new", "--ephemeral", "--ephemeral-idle-ttl-secs", "1800"]),
            11,
            "host-a",
        )
        .expect("a TTL-only declaration is a complete declaration")
        .ephemeral
        .expect("it is a declaration");
        assert_eq!(
            ttl_only,
            EphemeralDeclaration::new(None, "host-a", Some(1_800))
        );
        assert_eq!(ttl_only.owner_pid, None, "TTL-only names no owner at all");
        assert!(
            create_terminal_tenancy_from_args(
                &argv(&["new", "--ephemeral", "--ephemeral-owner-pid", "4242"]),
                11,
                "host-a",
            )
            .is_ok(),
            "an explicitly named owner is the other valid form"
        );
    }

    #[test]
    fn the_ephemeral_rules_are_read_off_their_own_flags() {
        let declared = create_terminal_tenancy_from_args(
            &argv(&[
                "new",
                "--ephemeral",
                "--ephemeral-owner-pid",
                "4242",
                "--ephemeral-idle-ttl-secs",
                "1800",
            ]),
            11,
            "host-a",
        )
        .expect("both rules are valid together");
        assert_eq!(
            declared.ephemeral,
            Some(EphemeralDeclaration::new(Some(4242), "host-a", Some(1_800)))
        );
    }

    /// THE INLINE-SPELLING LOCK (round-25 review, finding P1). `--flag=value`
    /// is the costly direction: a `windows(2)` exact-match parser DISCARDED it
    /// silently and fell back to the default, so an agent that wrote
    /// `--ephemeral-owner-pid=4242` armed something it never asked for. Both
    /// spellings go through the one argv rule, on every tenancy flag.
    #[test]
    fn the_inline_flag_spelling_is_honoured_on_every_tenancy_flag() {
        let inline = create_terminal_tenancy_from_args(
            &argv(&[
                "new",
                "--purpose=aged ssh probe",
                "--ephemeral",
                "--ephemeral-owner-pid=4242",
                "--ephemeral-idle-ttl-secs=1800",
            ]),
            11,
            "host-a",
        )
        .expect("the inline spelling is a valid declaration");
        let spaced = create_terminal_tenancy_from_args(
            &argv(&[
                "new",
                "--purpose",
                "aged ssh probe",
                "--ephemeral",
                "--ephemeral-owner-pid",
                "4242",
                "--ephemeral-idle-ttl-secs",
                "1800",
            ]),
            11,
            "host-a",
        )
        .expect("the spaced spelling is a valid declaration");
        assert_eq!(
            inline, spaced,
            "one parser: --flag=value and --flag value must declare the same tenancy"
        );
        assert_eq!(
            inline.ephemeral,
            Some(EphemeralDeclaration::new(Some(4242), "host-a", Some(1_800))),
            "an inline owner pid must be READ, not discarded"
        );
        assert_eq!(inline.created_by.purpose.as_deref(), Some("aged ssh probe"));
    }

    /// A typo must refuse, not silently create an unarmed row the caller
    /// believes is disposable.
    #[test]
    fn ephemeral_rule_flags_without_the_opt_in_are_refused() {
        for args in [
            argv(&["new", "--ephemeral-owner-pid", "5"]),
            argv(&["new", "--ephemeral-idle-ttl-secs", "60"]),
            argv(&["new", "--ephemeral-owner-pid=5"]),
        ] {
            assert!(
                create_terminal_tenancy_from_args(&args, 11, "host-a").is_err(),
                "{args:?} names a reap rule with nothing to attach it to"
            );
        }
        for bad in [
            argv(&["new", "--ephemeral", "--ephemeral-owner-pid", "nope"]),
            argv(&["new", "--ephemeral", "--ephemeral-owner-pid=nope"]),
            argv(&["new", "--ephemeral", "--ephemeral-idle-ttl-secs", "0"]),
            argv(&["new", "--ephemeral", "--ephemeral-idle-ttl-secs", "soon"]),
        ] {
            assert!(
                create_terminal_tenancy_from_args(&bad, 11, "host-a").is_err(),
                "{bad:?} must be refused rather than rounded to a default"
            );
        }
    }

    #[test]
    fn an_owner_on_another_host_is_unknown_never_gone() {
        let declared = EphemeralDeclaration::new(Some(1), "host-a", None);
        assert_eq!(
            owner_liveness_on_host(&declared, "host-b"),
            OwnerLiveness::Unknown
        );
    }

    /// The unknown-host sentinel is a FALLBACK, not an identity: two machines
    /// that cannot name themselves both write it, so matching them would let
    /// this daemon answer "gone" about a pid that only ever existed elsewhere.
    #[test]
    fn the_unknown_host_sentinel_never_answers_a_liveness_question() {
        let declared =
            EphemeralDeclaration::new(Some(std::process::id()), UNKNOWN_HOST_TOKEN, None);
        assert_eq!(
            owner_liveness_on_host(&declared, UNKNOWN_HOST_TOKEN),
            OwnerLiveness::Unknown,
            "a sentinel host token is not a machine identity and must not match one"
        );
        assert_eq!(
            owner_liveness_on_host(
                &EphemeralDeclaration::new(Some(std::process::id()), "host-a", None),
                UNKNOWN_HOST_TOKEN,
            ),
            OwnerLiveness::Unknown,
            "and a daemon that cannot name ITSELF must not answer either"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_liveness_reads_this_hosts_proc_for_a_matching_host() {
        let host = local_host_token();
        let alive = EphemeralDeclaration::new(Some(std::process::id()), &host, None);
        assert_eq!(owner_liveness_on_host(&alive, &host), OwnerLiveness::Alive);
        // pid 0 is never a userspace process, so /proc/0 never exists.
        let gone = EphemeralDeclaration {
            owner_pid: Some(0),
            owner_host: host.clone(),
            idle_ttl_secs: None,
        };
        assert_eq!(owner_liveness_on_host(&gone, &host), OwnerLiveness::Gone);
    }
}
