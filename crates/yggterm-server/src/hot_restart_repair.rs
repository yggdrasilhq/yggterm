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
    Repair { sessions: Vec<String> },
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
    RepairVerdict::Repair { sessions: mine }
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
    fn the_owed_continue_survives_the_daemon_that_owes_it() {
        // §8: the list is computed before the old daemon dies and consumed after
        // the new one is up. That gap is the entire reason this is a file.
        let home = scratch_home("survives");
        assert!(record(&home, &interrupted(1_000, 111, &["local://a"])));
        assert_eq!(
            take_repairable(&home, 2_000, 222, &["local://a".to_string()]),
            RepairVerdict::Repair {
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
    fn a_forced_swap_that_interrupted_nobody_writes_nothing() {
        let home = scratch_home("empty");
        assert!(!record(&home, &interrupted(1_000, 111, &[])));
        assert!(load(&home).is_none());
        let _ = fs::remove_dir_all(&home);
    }
}
