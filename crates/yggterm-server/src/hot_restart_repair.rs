//! §5 of [`docs/spec-hot-restart-relay-gate.md`] — **the repair that makes the
//! deadline safe.**
//!
//! > *"After 30 minutes of waiting, force the swap, stalling the working
//! > sessions — and then inject `continue` into every session that was
//! > interrupted. … The two halves are one ruling and must ship together. A
//! > deadline alone is what the campaign memory forbids, and rightly."*
//!
//! ## Why the list is a FILE and not a variable
//!
//! Spec §8: *"the interrupted set must be recorded across the swap. `continue`
//! is owed to a list that is computed before the old daemon dies and consumed
//! after the new one is up; it cannot be re-derived afterwards, because after
//! the swap every interrupted session looks idle."*
//!
//! That sentence is the whole design. The process that knows who was
//! interrupted is the process that is about to exit, and the process that can
//! repair them does not exist yet. Nothing in memory spans that gap.
//!
//! ## Who is owed a `continue`, exactly
//!
//! Only the sessions the forced shutdown actually interrupted — the blockers
//! that were NOT [`crate::daemon::hot_restart_blocker_is_deadline_exempt`]. A
//! session that was idle, or parked at a question, must never be nudged:
//! *"nudging a session parked by design trains its reader to ignore the
//! signal"*, which is the same guard the fleet skill's stall-recovery section
//! states.
//!
//! ⛔ **At most once, and the record is spent on DISPATCH, not on success.** A
//! `continue` that fails to land is a lost repair; a `continue` sent twice is an
//! unasked-for turn in someone's session, and the second is worse. So the keys
//! leave the record the moment a repair is dispatched for them.
//!
//! ⭐ **With ONE exception, and it is not a weakening of that rule.** A submit
//! that reports [`crate::terminal::PromptSubmitOutcome::NotReady`] is proof that
//! *nothing was written*: the readiness probe never echoed, and the submit path
//! clears the composer on its way out. Those keys go back via
//! [`requeue_unsubmitted`], under their ORIGINAL window. Without it a session
//! the deadline interrupted is never repaired at all — measured live, where a
//! just-re-resumed agent CLI had not brought its input loop up inside the submit
//! timeout, and the record had already been spent. **That is the deadline
//! shipping alone, which is the thing the ruling forbids.**
//!
//! ⛔ **And the record EXPIRES.** A repair that arrives long after the
//! interruption is not a repair — the session has been back for half an hour and
//! a `continue` is then exactly the unprompted nudge §5 forbids. Past
//! [`REPAIR_WINDOW_MS`] the record is dropped, loudly, rather than honoured.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How long after a forced swap a `continue` is still a repair rather than an
/// interruption of its own.
///
/// Ten minutes: long enough for a cold shutdown, a recovery spawn and a re-resume
/// of every agent on the host (the slowest of those is the re-resume, measured in
/// tens of seconds), and short enough that a record left behind by a host that
/// never came back cannot fire into a session that has been working for an hour.
pub const REPAIR_WINDOW_MS: u64 = 600_000;

/// The sessions a forced swap interrupted, written by the daemon that is about
/// to die.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptedSessions {
    /// When the forced swap happened. [`REPAIR_WINDOW_MS`] is measured from it.
    pub recorded_at_ms: u64,
    /// The daemon that did the interrupting. A repairer must not be that daemon
    /// — it is the one whose PTYs died, so any session it still owns is one the
    /// interruption did not reach.
    pub recorded_by_pid: u32,
    pub recorded_by_version: String,
    /// Why the deadline fired, for the trace and for `server daemons`.
    pub reason: String,
    /// Session keys owed exactly one `continue`.
    pub sessions: Vec<String>,
}

pub fn record_path(home_dir: &Path) -> PathBuf {
    home_dir.join("hot-restart-interrupted.json")
}

pub fn load(home_dir: &Path) -> Option<InterruptedSessions> {
    let bytes = fs::read(record_path(home_dir)).ok()?;
    serde_json::from_slice::<InterruptedSessions>(&bytes).ok()
}

pub fn save(home_dir: &Path, record: &InterruptedSessions) -> io::Result<()> {
    let path = record_path(home_dir);
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let encoded = serde_json::to_vec_pretty(record)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&tmp, encoded)?;
    fs::rename(&tmp, &path)
}

pub fn clear(home_dir: &Path) {
    let _ = fs::remove_file(record_path(home_dir));
}

/// Merge an incoming interrupted set into whatever is already recorded.
///
/// ⛔ **Union, never overwrite.** Two daemons on one host can both be forced
/// past the deadline — the shape that stacked eighteen of them in the first
/// place — and the second writer overwriting the first would silently un-owe
/// every `continue` the first was owed. The stamp follows the NEWEST write,
/// because the repair window is about how long ago the most recent interruption
/// happened.
pub fn merge_for_write(
    existing: Option<&InterruptedSessions>,
    incoming: &InterruptedSessions,
) -> InterruptedSessions {
    let mut merged = incoming.clone();
    if let Some(existing) = existing {
        for key in &existing.sessions {
            if !merged.sessions.contains(key) {
                merged.sessions.push(key.clone());
            }
        }
    }
    merged
}

/// Record that a forced swap is about to interrupt these sessions.
///
/// Called BEFORE the shutdown, by the daemon doing the interrupting. An empty
/// list writes nothing: a forced swap that interrupted nobody owes nobody.
pub fn record(home_dir: &Path, incoming: &InterruptedSessions) -> bool {
    if incoming.sessions.is_empty() {
        return false;
    }
    let merged = merge_for_write(load(home_dir).as_ref(), incoming);
    save(home_dir, &merged).is_ok()
}

/// What [`take_repairable`] concluded, so the caller can trace a decision that
/// otherwise leaves no evidence either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairVerdict {
    /// Nothing recorded, or nothing this daemon can repair yet.
    Nothing,
    /// The record aged out. It is cleared, and the reader is told, because a
    /// silently dropped repair is indistinguishable from one that never
    /// happened.
    Expired { age_ms: u64, sessions: Vec<String> },
    /// These keys are owed a `continue` from this daemon, right now.
    ///
    /// `origin` travels with them so a key whose submit reports NotReady can be
    /// put back under the SAME window — see [`requeue_unsubmitted`].
    Repair {
        origin: RepairOrigin,
        sessions: Vec<String>,
    },
}

/// Decide what this daemon owes, and SPEND it in the same step.
///
/// Pure decision + a write, deliberately together: the at-most-once property
/// depends on the keys leaving the record atomically with the caller learning
/// about them. A `should_repair()` that a caller then forgot to follow with a
/// `mark_repaired()` is the same one-shot-with-no-target shape that cost this
/// project the version skew in the first place.
pub fn take_repairable(
    home_dir: &Path,
    now_ms: u64,
    my_pid: u32,
    owned_keys: &[String],
) -> RepairVerdict {
    let Some(record) = load(home_dir) else {
        return RepairVerdict::Nothing;
    };
    let age_ms = now_ms.saturating_sub(record.recorded_at_ms);
    if age_ms > REPAIR_WINDOW_MS {
        clear(home_dir);
        return RepairVerdict::Expired {
            age_ms,
            sessions: record.sessions,
        };
    }
    if record.recorded_by_pid == my_pid {
        // We are the daemon that did the interrupting, so nothing we still own
        // was interrupted by it. Repairing here would nudge a session that never
        // lost its PTY.
        return RepairVerdict::Nothing;
    }
    let mine: Vec<String> = record
        .sessions
        .iter()
        .filter(|key| owned_keys.iter().any(|owned| owned == *key))
        .cloned()
        .collect();
    if mine.is_empty() {
        return RepairVerdict::Nothing;
    }
    let remaining: Vec<String> = record
        .sessions
        .iter()
        .filter(|key| !mine.contains(key))
        .cloned()
        .collect();
    let origin = RepairOrigin {
        recorded_at_ms: record.recorded_at_ms,
        recorded_by_pid: record.recorded_by_pid,
        recorded_by_version: record.recorded_by_version.clone(),
        reason: record.reason.clone(),
    };
    if remaining.is_empty() {
        clear(home_dir);
    } else {
        let _ = save(
            home_dir,
            &InterruptedSessions {
                sessions: remaining,
                ..record
            },
        );
    }
    RepairVerdict::Repair {
        origin,
        sessions: mine,
    }
}

/// Put back keys whose `continue` was **never written**.
///
/// ⛔ **This does not weaken at-most-once, and the distinction is the whole
/// reason it is safe.** The rule protects against a `continue` that LANDED being
/// sent again. [`crate::terminal::PromptSubmitOutcome::NotReady`] is the one
/// outcome that is *proof nothing landed*: the readiness probe never echoed, so
/// the program was not consuming input, and the submit path clears the composer
/// on its way out rather than leaving text behind. Returning that key is a
/// retry of something that did not happen.
///
/// ⛔ **The stamp NEVER moves forward.** [`REPAIR_WINDOW_MS`] is measured from
/// the interruption, and a requeue that restamped would rebuild the
/// never-converging clock this project has already fixed once — a repair that
/// keeps failing would then stay owed for ever and eventually fire a `continue`
/// into a session that has been working for an hour. Past the window it expires
/// exactly as it would have.
///
/// ⭐ Found by §5's own falsifier: a forced swap's repair came back `not_ready`
/// because a just-re-resumed agent CLI had not brought its input loop up within
/// the submit timeout. The record had already been spent, so that session was
/// interrupted and never repaired — **the deadline shipping alone**, which is
/// precisely what the ruling forbids.
pub fn requeue_unsubmitted(home_dir: &Path, origin: &RepairOrigin, sessions: &[String]) -> bool {
    if sessions.is_empty() {
        return false;
    }
    let existing = load(home_dir);
    let incoming = InterruptedSessions {
        // max(), so a NEWER interruption recorded while we were submitting keeps
        // its own window rather than being dragged back by our older one.
        recorded_at_ms: existing
            .as_ref()
            .map(|existing| existing.recorded_at_ms.max(origin.recorded_at_ms))
            .unwrap_or(origin.recorded_at_ms),
        recorded_by_pid: origin.recorded_by_pid,
        recorded_by_version: origin.recorded_by_version.clone(),
        reason: origin.reason.clone(),
        sessions: sessions.to_vec(),
    };
    save(home_dir, &merge_for_write(existing.as_ref(), &incoming)).is_ok()
}

/// Who recorded a repair, and when — carried out of [`take_repairable`] so an
/// unsubmitted key can be put back under its ORIGINAL window rather than a
/// fresh one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairOrigin {
    pub recorded_at_ms: u64,
    pub recorded_by_pid: u32,
    pub recorded_by_version: String,
    pub reason: String,
}

/// One line for `server daemons`, so a repair still owed says so where a human
/// is already looking.
///
/// §8's rule applied to the other half of §5: a host that interrupted somebody
/// and has not yet made it good is in a state a reader must be able to see. It
/// is also the only way to notice a repair that is quietly aging out.
pub fn format_pending_repair(record: &InterruptedSessions, now_ms: u64) -> String {
    let age_ms = now_ms.saturating_sub(record.recorded_at_ms);
    format!(
        "  repair owed: `continue` for {count} session(s) interrupted {age_s}s ago by pid {pid} \
         ({version}), window {window_s}s — {sessions}\n",
        count = record.sessions.len(),
        age_s = age_ms / 1_000,
        pid = record.recorded_by_pid,
        version = record.recorded_by_version,
        window_s = REPAIR_WINDOW_MS / 1_000,
        sessions = record.sessions.join(", "),
    )
}

/// The expiry law on its own feet: clear a record already past
/// [`REPAIR_WINDOW_MS`], whoever asks. Returns `Some((age_ms, sessions))` when
/// it cleared, `None` when there was nothing past the window.
///
/// Why a standalone entry point beside [`take_repairable`]'s expired arm: that
/// arm's only caller was the disk-binary poll thread, and the poll thread
/// provably dies mid-linger — measured live on dev, 2026-09-02, TWICE the
/// same day: a forced swap wrote the record, the shutdown took the preserving
/// path and lingered, the poll thread exited, and the daemon went on serving
/// every request while no code path could age the record out. Each stranded
/// record then refused every deploy at the gate for hours, on a repair that by
/// this module's own law was already a dead letter ("dropped, loudly, rather
/// than honoured" — the dropping just had no survivor left to do it).
///
/// ⛔ Safe to run beside a dispatch: an expired record is never dispatched, so
/// no repair batch can be in flight against one — the two paths cannot both
/// act on the same record.
pub fn sweep_expired(home_dir: &Path, now_ms: u64) -> Option<(u64, Vec<String>)> {
    let record = load(home_dir)?;
    let age_ms = now_ms.saturating_sub(record.recorded_at_ms);
    if age_ms <= REPAIR_WINDOW_MS {
        return None;
    }
    clear(home_dir);
    Some((age_ms, record.sessions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "ygg-hot-restart-repair-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).expect("scratch home");
        home
    }

    fn interrupted(at_ms: u64, pid: u32, sessions: &[&str]) -> InterruptedSessions {
        InterruptedSessions {
            recorded_at_ms: at_ms,
            recorded_by_pid: pid,
            recorded_by_version: "1.2.3".to_string(),
            reason: "unit test".to_string(),
            sessions: sessions.iter().map(|key| key.to_string()).collect(),
        }
    }

    #[test]
    fn the_sweeper_ages_out_what_dispatch_never_reached() {
        // The measured class: the record outlives every thread that could have
        // run take_repairable's expired arm. sweep_expired is the same law with
        // no caller alive to depend on — past the window it clears, and says so.
        let home = scratch_home("sweeper-expired");
        assert!(record(&home, &interrupted(1_000, 111, &["local://a"])));
        let cleared = sweep_expired(&home, 1_000 + REPAIR_WINDOW_MS + 1);
        assert_eq!(
            cleared,
            Some((REPAIR_WINDOW_MS + 1, vec!["local://a".to_string()]))
        );
        assert!(load(&home).is_none(), "expired record must be gone");
    }

    #[test]
    fn the_sweeper_leaves_a_live_repair_alone() {
        // Inside the window the record is dispatch's to spend, not the
        // sweeper's — clearing it would un-owe a `continue` still owed.
        let home = scratch_home("sweeper-fresh");
        assert!(record(&home, &interrupted(1_000, 111, &["local://a"])));
        assert!(sweep_expired(&home, 1_000 + REPAIR_WINDOW_MS - 1).is_none());
        assert!(load(&home).is_some(), "live record must survive the sweep");
    }

    #[test]
    fn the_sweeper_and_an_absent_record_agree_to_do_nothing() {
        let home = scratch_home("sweeper-absent");
        assert!(sweep_expired(&home, 9_000).is_none());
    }

    #[test]
    fn the_owed_continue_survives_the_daemon_that_owes_it() {
        // §8: the list is computed before the old daemon dies and consumed after
        // the new one is up. That gap is the entire reason this is a file.
        let home = scratch_home("survives");
        assert!(record(&home, &interrupted(1_000, 111, &["local://a"])));
        assert_eq!(
            take_repairable(&home, 2_000, 222, &["local://a".to_string()]),
            RepairVerdict::Repair {
                origin: RepairOrigin {
                    recorded_at_ms: 1_000,
                    recorded_by_pid: 111,
                    recorded_by_version: "1.2.3".to_string(),
                    reason: "unit test".to_string(),
                },
                sessions: vec!["local://a".to_string()]
            }
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_continue_is_dispatched_at_most_once() {
        // ⛔ The one failure worse than a lost repair: an unasked-for turn typed
        // into someone's session, every twenty seconds, forever.
        let home = scratch_home("once");
        record(&home, &interrupted(1_000, 111, &["local://a"]));
        let owned = vec!["local://a".to_string()];
        assert!(matches!(
            take_repairable(&home, 2_000, 222, &owned),
            RepairVerdict::Repair { .. }
        ));
        assert_eq!(
            take_repairable(&home, 3_000, 222, &owned),
            RepairVerdict::Nothing,
            "the record must be spent on dispatch, not on success"
        );
        assert!(load(&home).is_none(), "an emptied record is removed");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_interrupting_daemon_never_repairs_its_own_survivors() {
        // A session this daemon still owns is one its own shutdown never
        // reached, so it was not interrupted and is owed nothing.
        let home = scratch_home("self");
        record(&home, &interrupted(1_000, 111, &["local://a"]));
        assert_eq!(
            take_repairable(&home, 2_000, 111, &["local://a".to_string()]),
            RepairVerdict::Nothing
        );
        assert!(
            load(&home).is_some(),
            "and it must leave the record for whoever DID adopt the session"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn only_the_keys_this_daemon_owns_are_taken() {
        // Several daemons can adopt different halves of an interrupted set, so
        // taking the whole list would discard every repair but this one's.
        let home = scratch_home("partial");
        record(&home, &interrupted(1_000, 111, &["local://a", "local://b"]));
        assert_eq!(
            take_repairable(&home, 2_000, 222, &["local://b".to_string()]),
            RepairVerdict::Repair {
                origin: RepairOrigin {
                    recorded_at_ms: 1_000,
                    recorded_by_pid: 111,
                    recorded_by_version: "1.2.3".to_string(),
                    reason: "unit test".to_string(),
                },
                sessions: vec!["local://b".to_string()]
            }
        );
        let left = load(&home).expect("the unadopted key survives");
        assert_eq!(left.sessions, vec!["local://a".to_string()]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_stale_record_is_dropped_rather_than_honoured() {
        // Past the window a `continue` is not a repair, it is an unprompted turn
        // in a session that has been working for half an hour.
        let home = scratch_home("stale");
        record(&home, &interrupted(1_000, 111, &["local://a"]));
        let verdict = take_repairable(
            &home,
            1_000 + REPAIR_WINDOW_MS + 1,
            222,
            &["local://a".to_string()],
        );
        assert!(matches!(verdict, RepairVerdict::Expired { .. }), "{verdict:?}");
        assert!(load(&home).is_none(), "an expired record is cleared, not left to fire later");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_second_forced_swap_does_not_un_owe_the_first() {
        // ⛔ Overwrite would silently drop every `continue` the earlier
        // interruption was owed — and two daemons forced past the deadline on
        // one host is the exact shape that stacked eighteen of them.
        let home = scratch_home("merge");
        record(&home, &interrupted(1_000, 111, &["local://a"]));
        record(&home, &interrupted(2_000, 333, &["local://b"]));
        let stored = load(&home).expect("stored");
        assert!(stored.sessions.contains(&"local://a".to_string()));
        assert!(stored.sessions.contains(&"local://b".to_string()));
        assert_eq!(
            stored.recorded_at_ms, 2_000,
            "the window follows the most recent interruption"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_continue_that_was_never_written_goes_back_on_the_record() {
        // ⭐ Found by §5's own falsifier: a forced swap's repair came back
        // `not_ready` because a just-re-resumed agent CLI had not brought its
        // input loop up inside the submit timeout. The record was already spent,
        // so that session was interrupted and never repaired — the deadline
        // shipping alone, which is exactly what the ruling forbids.
        let home = scratch_home("requeue");
        record(&home, &interrupted(1_000, 111, &["local://a"]));
        let RepairVerdict::Repair { origin, sessions } =
            take_repairable(&home, 2_000, 222, &["local://a".to_string()])
        else {
            panic!("the key should have been taken");
        };
        assert!(load(&home).is_none(), "spent on dispatch, as before");
        assert!(requeue_unsubmitted(&home, &origin, &sessions));
        let back = load(&home).expect("an unwritten continue is still owed");
        assert_eq!(back.sessions, vec!["local://a".to_string()]);
        assert_eq!(
            back.recorded_at_ms, 1_000,
            "⛔ the window is measured from the INTERRUPTION — a requeue that \
             restamped would keep a failing repair owed for ever"
        );
        // And it still expires on the original clock rather than a fresh one.
        assert!(matches!(
            take_repairable(&home, 1_000 + REPAIR_WINDOW_MS + 1, 222, &["local://a".to_string()]),
            RepairVerdict::Expired { .. }
        ));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_requeue_never_drags_a_newer_interruption_backwards() {
        // Two forced swaps can overlap. The window follows the most recent one,
        // so putting an older key back must not shorten the newer one's.
        let home = scratch_home("requeue-newer");
        let origin = RepairOrigin {
            recorded_at_ms: 1_000,
            recorded_by_pid: 111,
            recorded_by_version: "1.2.3".to_string(),
            reason: "older".to_string(),
        };
        record(&home, &interrupted(9_000, 333, &["local://newer"]));
        assert!(requeue_unsubmitted(&home, &origin, &["local://older".to_string()]));
        let merged = load(&home).expect("both are owed");
        assert_eq!(merged.recorded_at_ms, 9_000);
        assert!(merged.sessions.contains(&"local://older".to_string()));
        assert!(merged.sessions.contains(&"local://newer".to_string()));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn nothing_to_requeue_writes_nothing() {
        let home = scratch_home("requeue-empty");
        let origin = RepairOrigin {
            recorded_at_ms: 1_000,
            recorded_by_pid: 111,
            recorded_by_version: "1.2.3".to_string(),
            reason: "none".to_string(),
        };
        assert!(!requeue_unsubmitted(&home, &origin, &[]));
        assert!(load(&home).is_none());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_forced_swap_that_interrupted_nobody_writes_nothing() {
        let home = scratch_home("empty");
        assert!(!record(&home, &interrupted(1_000, 111, &[])));
        assert!(load(&home).is_none());
        let _ = fs::remove_dir_all(&home);
    }
}
