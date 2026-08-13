//! §4 of [`docs/spec-hot-restart-relay-gate.md`] — **queue, do not poll.**
//!
//! > *"A swap request is QUEUED, not attempted-and-abandoned. … a request that
//! > cannot run now runs at the next boundary, and it is never lost. One request
//! > is in flight at a time; a newer build supersedes a queued older one rather
//! > than adding a second entry."*
//!
//! ## The defect this owns, measured
//!
//! Every party that wants a daemon swap tries **once** and forgets:
//!
//! - the GUI's startup reconcile (`reconcile_stale_daemon_on_startup`) runs at
//!   launch and, when it declines, returns `false` and leaves nothing behind;
//! - the daemon's own disk-binary retire poll attempts the preserving handoff
//!   and then **breaks out of its loop**, so the thread that would retry is
//!   gone.
//!
//! Measured on the GUI host 2026-08-13. Its 3.0.118 daemon logged
//! `daemon_self_retire {retire_trigger: "disk_binary_replaced"}` **exactly once**
//! (13:45:52), answered `daemon_self_retire_handoff_ok` ten seconds later, and
//! emitted nothing further — while a 3.0.120 binary sat on disk and the GUI ran
//! 3.0.120. Forty-five minutes later the host still had no daemon at the GUI's
//! version and **nothing anywhere recorded that one was owed.** On the workshop
//! host the same shape had accumulated **eighteen coexisting daemons, the oldest
//! alive for 20.6 days**.
//!
//! ⇒ The intent is the thing that goes missing, not the mechanism. A swap that
//! cannot happen at 13:45 is not wrong, it is early; what makes it permanent is
//! that by 13:46 nobody remembers it was wanted.
//!
//! ## Why the record is a HOST fact, not a daemon field
//!
//! "This host owes a swap to version X" outlives the daemon being replaced —
//! that is the entire point — so it cannot live in that daemon's status. It is a
//! single-slot file under `~/.yggterm`, which is also why the supersede rule is
//! expressible at all: one cell, and a write either wins on version or is
//! refused. A list would have re-created the "two requests disagree about the
//! target" problem the spec's *"one request in flight"* sentence forbids.
//!
//! ⛔ **Superseding must not reset the clock when the target is unchanged.**
//! §5's deadline is measured from `requested_at_ms`, so a re-request for the
//! version already queued has to leave that stamp alone; otherwise a host that
//! re-requests every poll can never reach any deadline, which is the
//! never-converging shape the constitution forbids, rebuilt one layer up.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::daemon::parse_daemon_version_triple;

/// The one swap this host owes, and everything a reader needs to say why it is
/// still owed. §8: *"if a swap is waiting, something must be nameable as the
/// thing it waits for"* — `last_outcome` is that name, carried across process
/// deaths because the file is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedHotRestart {
    /// The daemon version this host is trying to get to.
    pub target_version: String,
    /// The binary that serves that version. Recorded rather than re-derived so a
    /// drainer that is not the original requester still knows what to spawn.
    pub daemon_executable: String,
    /// Who queued it (`disk_binary_replaced_self_retire`, `gui_startup_reconcile`,
    /// a relay boundary …). Free-form on purpose: it is for the trace, never a
    /// branch.
    pub requested_by: String,
    /// When the intent was FIRST formed for this target. §5's deadline reads it,
    /// so a re-request for the same target must not move it.
    pub requested_at_ms: u64,
    /// How many times a drainer has tried since. Zero means queued and untried.
    pub attempts: u32,
    #[serde(default)]
    pub last_attempt_ms: Option<u64>,
    /// What the last attempt actually did. `None` = never attempted.
    #[serde(default)]
    pub last_outcome: Option<String>,
}

/// What [`decide_queue`] did with an incoming request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueDecision {
    /// Nothing was queued; this is now the request in flight.
    Queued,
    /// A newer build replaced the queued older one — one entry, not two.
    Superseded { replaced_version: String },
    /// The same target was already queued. The stored entry wins, clock intact.
    Unchanged,
    /// Refused, and the reason is the message a reader gets.
    Ignored { reason: &'static str },
}

impl QueueDecision {
    /// Does this decision require the caller to WRITE? `Unchanged`/`Ignored` do
    /// not, and writing on them is exactly how the §5 clock would get reset.
    pub fn writes(&self) -> bool {
        matches!(self, Self::Queued | Self::Superseded { .. })
    }

    /// The one word a trace event carries.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Superseded { .. } => "superseded",
            Self::Unchanged => "unchanged",
            Self::Ignored { .. } => "ignored",
        }
    }
}

/// The pure supersede rule, so it can be tested without a filesystem and cannot
/// drift from what the file ends up holding.
pub fn decide_queue(
    existing: Option<&QueuedHotRestart>,
    incoming: &QueuedHotRestart,
) -> QueueDecision {
    let Some(incoming_triple) = parse_daemon_version_triple(incoming.target_version.trim()) else {
        // ⛔ Fail CLOSED on an unreadable target. A queue entry drives a spawn;
        // one whose version nobody can compare could never be superseded and
        // would pin the slot against every real request behind it.
        return QueueDecision::Ignored {
            reason: "target version is not a version triple",
        };
    };
    let Some(existing_entry) = existing else {
        return QueueDecision::Queued;
    };
    let Some(existing_triple) = parse_daemon_version_triple(existing_entry.target_version.trim())
    else {
        // A stored entry we cannot parse is not a reason to refuse real work.
        return QueueDecision::Superseded {
            replaced_version: existing_entry.target_version.clone(),
        };
    };
    if incoming_triple > existing_triple {
        return QueueDecision::Superseded {
            replaced_version: existing_entry.target_version.clone(),
        };
    }
    if incoming_triple == existing_triple {
        return QueueDecision::Unchanged;
    }
    QueueDecision::Ignored {
        reason: "a newer swap is already queued",
    }
}

/// Is this request already satisfied — i.e. is a daemon at or above the target
/// live on this host?
///
/// ⚠ At or ABOVE, never equality. A host that jumped two builds while the entry
/// sat in the queue has converged past it, and an equality test would keep
/// asking for a version that is now behind the running one — a queue that can
/// only be cleared by hitting a number it has already passed.
pub fn satisfied_by(request: &QueuedHotRestart, live_version: &str) -> bool {
    let (Some(live), Some(target)) = (
        parse_daemon_version_triple(live_version.trim()),
        parse_daemon_version_triple(request.target_version.trim()),
    ) else {
        return false;
    };
    live >= target
}

pub fn queue_path(home_dir: &Path) -> PathBuf {
    home_dir.join("hot-restart-queue.json")
}

pub fn load(home_dir: &Path) -> Option<QueuedHotRestart> {
    let bytes = fs::read(queue_path(home_dir)).ok()?;
    serde_json::from_slice::<QueuedHotRestart>(&bytes).ok()
}

pub fn save(home_dir: &Path, request: &QueuedHotRestart) -> io::Result<()> {
    let path = queue_path(home_dir);
    // Write-then-rename: several daemons on this host may be writing, and a
    // half-written slot reads as "nothing is owed", which is the failure this
    // whole file exists to remove.
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let encoded = serde_json::to_vec_pretty(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&tmp, encoded)?;
    fs::rename(&tmp, &path)
}

pub fn clear(home_dir: &Path) {
    let _ = fs::remove_file(queue_path(home_dir));
}

/// Load, decide, and write only when the decision says to. Returns what it did
/// so the caller can trace it.
pub fn enqueue(home_dir: &Path, incoming: &QueuedHotRestart) -> QueueDecision {
    let existing = load(home_dir);
    let decision = decide_queue(existing.as_ref(), incoming);
    if decision.writes() {
        let _ = save(home_dir, incoming);
    }
    decision
}

/// Record that a drainer tried, and what came of it. Keeps `requested_at_ms`.
pub fn record_attempt(home_dir: &Path, now_ms: u64, outcome: &str) {
    let Some(mut request) = load(home_dir) else {
        return;
    };
    request.attempts = request.attempts.saturating_add(1);
    request.last_attempt_ms = Some(now_ms);
    request.last_outcome = Some(outcome.to_string());
    let _ = save(home_dir, &request);
}

/// Has enough time passed since the last attempt to try again?
///
/// A queued swap that retried every poll would spawn a successor every 20 s on a
/// host where the swap cannot converge — turning a lost intent into a fork bomb,
/// which is a worse bug than the one being fixed. `None` (never attempted) is
/// always due.
pub fn attempt_is_due(request: &QueuedHotRestart, now_ms: u64, interval_ms: u64) -> bool {
    match request.last_attempt_ms {
        None => true,
        Some(last) => now_ms.saturating_sub(last) >= interval_ms,
    }
}

/// One line for `server daemons`, so a host that owes a swap says so where a
/// human is already looking.
pub fn format_queued_swap(request: &QueuedHotRestart, now_ms: u64) -> String {
    let waiting_min = now_ms.saturating_sub(request.requested_at_ms) / 60_000;
    let outcome = request
        .last_outcome
        .as_deref()
        .unwrap_or("queued, not yet attempted");
    format!(
        "  swap owed → {version}: queued {waiting_min}m ago by {by}, {attempts} attempt(s), last: {outcome}\n",
        version = request.target_version,
        by = request.requested_by,
        attempts = request.attempts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(version: &str, at_ms: u64) -> QueuedHotRestart {
        QueuedHotRestart {
            target_version: version.to_string(),
            daemon_executable: "/opt/example/bin/example-headless".to_string(),
            requested_by: "unit_test".to_string(),
            requested_at_ms: at_ms,
            attempts: 0,
            last_attempt_ms: None,
            last_outcome: None,
        }
    }

    fn scratch_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "ygg-hot-restart-queue-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).expect("scratch home");
        home
    }

    #[test]
    fn an_empty_slot_takes_the_request() {
        assert_eq!(
            decide_queue(None, &request("3.0.120", 1_000)),
            QueueDecision::Queued
        );
    }

    #[test]
    fn a_newer_build_supersedes_rather_than_stacking() {
        let existing = request("3.0.118", 1_000);
        assert_eq!(
            decide_queue(Some(&existing), &request("3.0.120", 9_000)),
            QueueDecision::Superseded {
                replaced_version: "3.0.118".to_string(),
            }
        );
    }

    #[test]
    fn an_older_build_never_displaces_a_newer_queued_one() {
        let existing = request("3.0.120", 1_000);
        let decision = decide_queue(Some(&existing), &request("3.0.118", 9_000));
        assert!(matches!(decision, QueueDecision::Ignored { .. }));
        assert!(!decision.writes(), "an ignored request must not write");
    }

    #[test]
    fn re_requesting_the_same_target_does_not_restart_the_deadline_clock() {
        // ⛔ THE REGRESSION THIS EXISTS FOR. §5's deadline is measured from
        // `requested_at_ms`. A drainer that re-queues on every 20 s poll while
        // the swap cannot land would push that stamp forward forever, so the
        // 30-minute deadline could never be reached — the never-converging gate
        // rebuilt one layer above the gate it replaced.
        let home = scratch_home("clock");
        let first = request("3.0.120", 1_000);
        assert_eq!(enqueue(&home, &first), QueueDecision::Queued);
        let again = request("3.0.120", 9_999_000);
        assert_eq!(enqueue(&home, &again), QueueDecision::Unchanged);
        let stored = load(&home).expect("entry survives the re-request");
        assert_eq!(
            stored.requested_at_ms, 1_000,
            "the original intent's clock must survive a re-request"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unparseable_target_is_refused_rather_than_pinning_the_slot() {
        let decision = decide_queue(None, &request("nightly", 1_000));
        assert!(matches!(decision, QueueDecision::Ignored { .. }));
        assert!(!decision.writes());
    }

    #[test]
    fn a_host_that_jumped_past_the_target_counts_as_satisfied() {
        let queued = request("3.0.118", 1_000);
        assert!(satisfied_by(&queued, "3.0.120"), "at or above, not equality");
        assert!(satisfied_by(&queued, "3.0.118"));
        assert!(!satisfied_by(&queued, "3.0.117"));
        assert!(
            !satisfied_by(&queued, "unreadable"),
            "an unreadable live version must never clear a queued swap"
        );
    }

    #[test]
    fn a_retry_waits_out_its_interval() {
        let mut queued = request("3.0.120", 0);
        assert!(
            attempt_is_due(&queued, 0, 300_000),
            "an untried request is due immediately"
        );
        queued.last_attempt_ms = Some(100_000);
        assert!(!attempt_is_due(&queued, 200_000, 300_000));
        assert!(attempt_is_due(&queued, 400_000, 300_000));
    }

    #[test]
    fn an_attempt_is_recorded_without_disturbing_the_intent() {
        let home = scratch_home("attempt");
        enqueue(&home, &request("3.0.120", 1_000));
        record_attempt(&home, 60_000, "successor never bound the target socket");
        let stored = load(&home).expect("stored");
        assert_eq!(stored.attempts, 1);
        assert_eq!(stored.last_attempt_ms, Some(60_000));
        assert_eq!(stored.requested_at_ms, 1_000);
        assert_eq!(stored.target_version, "3.0.120");
        assert!(
            format_daemon_line_mentions_the_reason(&stored),
            "§8: a waiting swap must name what it waits for"
        );
        let _ = fs::remove_dir_all(&home);
    }

    fn format_daemon_line_mentions_the_reason(request: &QueuedHotRestart) -> bool {
        format_queued_swap(request, 120_000).contains("successor never bound")
    }
}
