//! §4 of [`docs/cli-integration-layer.md`] — the handoff-witnessed ownership
//! ledger.
//!
//! > *"At handoff, the predecessor writes a ledger record per owned row:
//! > `adopted { by_pid, pty_fd: moved }` or `died_with_me { store_session_id,
//! > resume_argv }`. … The successor answers reattach from the ledger: adopted
//! > PTYs attach with zero discovery; `died_with_me` rows get an immediate
//! > `resume <store_session_id>` spawn."*
//!
//! ## The defect this owns, measured
//!
//! When a row could not ride a daemon swap, the next resume for it had NO
//! positive answer to *"who owns this session NOW?"* — so
//! [`crate::wait_for_external_agent_resume_to_clear`] went looking with blind
//! `/proc` scans, sleeping 3 000 ms per iteration behind a banner that
//! re-renders every 10 s. The owner's felt "12 s so far" before a codex row
//! reattaches is that loop paying for knowledge the daemon already had: the
//! daemon ORCHESTRATED the swap, so it witnessed, per row, whether the PTY
//! moved and to whom. The ledger is that witness, persisted.
//!
//! ## The writer/consumer contract
//!
//! - **One witness per record: the predecessor that ran the handoff.** It
//!   writes at sweep end ([`record_handoff`]), replacing only ITS OWN previous
//!   records — a daemon can never rewrite what another daemon vouched for.
//! - **The consumer that is served by a record clears it**
//!   ([`clear_satisfied`]) — the hot-restart-queue law: *a record that
//!   outlives its own satisfaction lies about ownership.*
//! - **Staleness is re-derived on every read, never trusted:**
//!   an `adopted` record is only as true as its adopter's liveness (one
//!   `/proc/<pid>` probe — a single positive check, never a scan) plus a TTL;
//!   a `died_with_me` record serves the immediate reattach wave and then
//!   expires. An invalid record is pruned on read and answers
//!   [`LedgerAnswer::NoAnswer`], which hands the question back to the
//!   pre-ledger behaviour (the /proc wait and its named banner).
//!
//! ## Why a file, and why the writes race only harmlessly
//!
//! Like the hot-restart queue, this is a HOST fact: it must outlive the
//! process that wrote it — that is the entire point. Writers are the handoff
//! daemon and the (short-lived) resume wrappers that prune/consume; both write
//! whole files through a temp-name + rename, so a reader never sees a torn
//! file and a crash mid-write leaves the previous complete ledger. Two writers
//! interleaving lose an update, never a truth: a lost prune is retried by the
//! next read (staleness is re-derived), and a lost handoff write degrades to
//! [`LedgerAnswer::NoAnswer`] — today's behaviour, honestly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use yggterm_core::SessionKind;

/// An `adopted` record survives at most this long even with its adopter
/// alive. The ledger exists to serve the reattach window around a swap; a
/// row that is truly live is answered by the daemon's own runtime map, which
/// is always fresher than any file.
pub const ADOPTED_RECORD_TTL_MS: u64 = 60 * 60 * 1000;

/// A `died_with_me` record serves the immediate reattach wave after a swap —
/// the minutes in which the owner (or the GUI's own reconcile) retries the
/// rows that did not ride the handoff. Past that, the holders it names have
/// either been resumed (the record consumed) or long since exited, and a
/// stale record would lie about ownership.
pub const DIED_WITH_ME_TTL_MS: u64 = 5 * 60 * 1000;

/// What the predecessor witnessed about one owned row, per §4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipRecord {
    pub kind: SessionKind,
    /// The session id a resume asks for — the id the guard scans /proc for.
    pub session_id: String,
    /// The predecessor's runtime key for the row, so a consumer can connect
    /// the record to the row it saw in the sidebar.
    pub runtime_key: String,
    pub disposition: OwnershipDisposition,
    pub written_at_ms: u64,
    /// The witness. A record is only as true as this process's word.
    pub written_by_pid: u32,
    pub from_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OwnershipDisposition {
    /// The row's PTY crossed to the successor and was seated (the handoff ack
    /// is the commit point — an ack'd adoption means the successor HOLDS it).
    Adopted { by_pid: u32, by_start_time: u64 },
    /// The row dies when the writer exits; here is how to open its
    /// conversation again. An immediate `resume` from this record replaces
    /// the 120 s /proc wait — the holder it would find is a designed corpse.
    DiedWithMe {
        store_session_id: String,
        resume_argv: Vec<String>,
    },
}

/// What the ledger answers for one reattach question. `NoAnswer` is not a
/// failure — it is the honest "I have nothing that is still true", and the
/// caller falls back to the pre-ledger behaviour (the /proc wait and its
/// named banner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerAnswer {
    Adopted { by_pid: u32 },
    DiedWithMe {
        store_session_id: String,
        resume_argv: Vec<String>,
    },
    NoAnswer,
}

impl LedgerAnswer {
    /// The one word a trace event carries.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Adopted { .. } => "adopted",
            Self::DiedWithMe { .. } => "died_with_me",
            Self::NoAnswer => "no_answer",
        }
    }
}

pub fn ledger_path(home_dir: &Path) -> PathBuf {
    home_dir.join("ownership-ledger.json")
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

/// Single-pid liveness — the positive check the whole ledger replaces the
/// /proc SCAN with. One readlink-free existence probe, never a readdir.
pub fn process_is_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        // A zombie holds no PTYs and satisfies no writer — its witness died
        // without managing to exit. Read the state, not just the existence.
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => match stat.rsplit_once(')') {
                Some((_, rest)) => {
                    rest.split_whitespace().next().is_some_and(|state| state != "Z")
                }
                None => true,
            },
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Adoption (the only writer of `Adopted`) is Linux-only, so on any
        // other target there is no witness to contradict; the TTL alone
        // bounds the record.
        let _ = pid;
        true
    }
}

/// Is `record` still TRUE at `now`? Pure, so the staleness law is testable
/// and cannot drift from what [`lookup`] enforces.
pub fn record_is_still_true(record: &OwnershipRecord, now: u64) -> bool {
    let age_ok = now.saturating_sub(record.written_at_ms)
        <= match record.disposition {
            OwnershipDisposition::Adopted { .. } => ADOPTED_RECORD_TTL_MS,
            OwnershipDisposition::DiedWithMe { .. } => DIED_WITH_ME_TTL_MS,
        };
    match record.disposition {
        OwnershipDisposition::Adopted { by_pid, .. } => age_ok && process_is_alive(by_pid),
        OwnershipDisposition::DiedWithMe { .. } => age_ok,
    }
}

pub fn load(home_dir: &Path) -> Vec<OwnershipRecord> {
    let Ok(bytes) = fs::read(ledger_path(home_dir)) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<OwnershipRecord>>(&bytes).unwrap_or_default()
}

pub fn save(home_dir: &Path, records: &[OwnershipRecord]) -> io::Result<()> {
    let path = ledger_path(home_dir);
    // Write-then-rename — the hot-restart-queue pattern: several processes on
    // this host may write, and a half-written ledger must read as "no answer",
    // never as a false one.
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let encoded = serde_json::to_vec_pretty(records)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&tmp, encoded)?;
    fs::rename(&tmp, &path)
}

/// THE writer verb: the handoff daemon records what this sweep witnessed,
/// replacing its OWN previous records wholesale and touching nobody else's.
fn write_records(home_dir: &Path, records: Vec<OwnershipRecord>) -> io::Result<()> {
    let writer_pid = std::process::id();
    let mut merged: Vec<OwnershipRecord> = load(home_dir)
        .into_iter()
        .filter(|record| record.written_by_pid != writer_pid)
        .collect();
    merged.extend(records);
    save(home_dir, &merged)
}

/// Record what one handoff sweep witnessed: every moved row becomes an
/// `adopted` record naming the successor that acked it. Rows that did NOT
/// move stay unrecorded — on a partial sweep the predecessor keeps serving
/// them, and a `died_with_me` written for a row its writer still serves would
/// be exactly the lie the staleness law exists to kill.
pub fn record_handoff(home_dir: &Path, records: Vec<OwnershipRecord>) -> io::Result<()> {
    write_records(home_dir, records)
}

/// Ask the ledger who owns `(kind, session_id)` NOW. Staleness is re-derived
/// here — an invalid record prunes itself from the file (best-effort) and
/// answers [`LedgerAnswer::NoAnswer`].
pub fn lookup(home_dir: &Path, kind: SessionKind, session_id: &str) -> LedgerAnswer {
    let records = load(home_dir);
    let now = now_ms();
    let mut newest: Option<&OwnershipRecord> = None;
    let mut stale = Vec::<OwnershipRecord>::new();
    for record in &records {
        if record.kind != kind || record.session_id != session_id {
            continue;
        }
        if !record_is_still_true(record, now) {
            stale.push(record.clone());
            continue;
        }
        newest = Some(match newest {
            Some(current) if current.written_at_ms >= record.written_at_ms => current,
            _ => record,
        });
    }
    let answer = match newest.map(|record| &record.disposition) {
        Some(OwnershipDisposition::Adopted { by_pid, .. }) => LedgerAnswer::Adopted { by_pid: *by_pid },
        Some(OwnershipDisposition::DiedWithMe {
            store_session_id,
            resume_argv,
        }) => LedgerAnswer::DiedWithMe {
            store_session_id: store_session_id.clone(),
            resume_argv: resume_argv.clone(),
        },
        None => LedgerAnswer::NoAnswer,
    };
    if !stale.is_empty() {
        prune(home_dir, &stale);
    }
    answer
}

/// The consumer that was SERVED by a record clears it — the
/// hot-restart-queue law. A resume that has launched from a `died_with_me`
/// record has satisfied it; leaving it would answer the NEXT reattach with a
/// witness about a holder that is already gone.
pub fn clear_satisfied(home_dir: &Path, kind: SessionKind, session_id: &str) {
    let records = load(home_dir);
    let remaining: Vec<OwnershipRecord> = records
        .into_iter()
        .filter(|record| !(record.kind == kind && record.session_id == session_id))
        .collect();
    let _ = save(home_dir, &remaining);
}

fn prune(home_dir: &Path, stale: &[OwnershipRecord]) {
    let stale_keys: Vec<(SessionKind, String)> = stale
        .iter()
        .map(|record| (record.kind, record.session_id.clone()))
        .collect();
    let remaining: Vec<OwnershipRecord> = load(home_dir)
        .into_iter()
        .filter(|record| {
            !stale_keys
                .iter()
                .any(|(kind, session_id)| *kind == record.kind && *session_id == record.session_id)
        })
        .collect();
    let _ = save(home_dir, &remaining);
}

/// Build the records one handoff sweep earns, from what it already knows.
///
/// `identity` is one triple per agent row the predecessor owned:
/// `(runtime_key, kind, session_id)`. Rows without a store identity (shells,
/// documents) are the caller's to exclude, and any adopted key without a
/// triple here is SKIPPED — a record that cannot name the session a resume
/// would ask for answers nothing.
pub fn handoff_ownership_records(
    identity: &[(String, SessionKind, String)],
    adopted: &[(String, (u32, u64))],
    now_ms: u64,
    writer_pid: u32,
    from_version: &str,
) -> Vec<OwnershipRecord> {
    adopted
        .iter()
        .filter_map(|(runtime_key, (by_pid, by_start_time))| {
            let (kind, session_id) = identity
                .iter()
                .find(|(key, _, _)| key == runtime_key)
                .map(|(_, kind, id)| (*kind, id.clone()))?;
            Some(OwnershipRecord {
                kind,
                session_id,
                runtime_key: runtime_key.clone(),
                disposition: OwnershipDisposition::Adopted {
                    by_pid: *by_pid,
                    by_start_time: *by_start_time,
                },
                written_at_ms: now_ms,
                written_by_pid: writer_pid,
                from_version: from_version.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zcode-ownership-ledger-{}-{}-{tag}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn adopted_record(now: u64, by_pid: u32) -> OwnershipRecord {
        OwnershipRecord {
            kind: SessionKind::Codex,
            session_id: "sess-abc".to_string(),
            runtime_key: "codex:sess-abc".to_string(),
            disposition: OwnershipDisposition::Adopted {
                by_pid,
                by_start_time: 1,
            },
            written_at_ms: now,
            written_by_pid: 111,
            from_version: "3.2.108".to_string(),
        }
    }

    fn died_with_me_record(now: u64) -> OwnershipRecord {
        OwnershipRecord {
            kind: SessionKind::Codex,
            session_id: "sess-abc".to_string(),
            runtime_key: "codex:sess-abc".to_string(),
            disposition: OwnershipDisposition::DiedWithMe {
                store_session_id: "sess-abc".to_string(),
                resume_argv: vec!["resume".to_string(), "sess-abc".to_string()],
            },
            written_at_ms: now,
            written_by_pid: 111,
            from_version: "3.2.108".to_string(),
        }
    }

    #[test]
    fn an_adopted_record_answers_while_its_adopter_lives() {
        let home = scratch_home("adopted-live");
        let now = now_ms();
        save(&home, &[adopted_record(now, std::process::id())]).unwrap();
        assert_eq!(
            lookup(&home, SessionKind::Codex, "sess-abc"),
            LedgerAnswer::Adopted {
                by_pid: std::process::id()
            }
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_adopted_record_cannot_outlive_its_adopter() {
        let home = scratch_home("adopted-dead");
        // A pid that has really exited: spawn a child, take ITS pid, and reap
        // it — the reaped pid is provably dead, not merely unlikely.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        assert!(!process_is_alive(dead_pid));
        save(&home, &[adopted_record(now_ms(), dead_pid)]).unwrap();
        assert_eq!(lookup(&home, SessionKind::Codex, "sess-abc"), LedgerAnswer::NoAnswer);
        // ⛔ the stale record prunes itself — it must not survive to answer
        // again.
        assert!(load(&home).is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_died_with_me_record_expires_at_its_ttl() {
        let home = scratch_home("died-ttl");
        let now = now_ms();
        save(&home, &[died_with_me_record(now - DIED_WITH_ME_TTL_MS - 1)]).unwrap();
        assert_eq!(lookup(&home, SessionKind::Codex, "sess-abc"), LedgerAnswer::NoAnswer);
        assert!(load(&home).is_empty());
        // Inside the window it answers, and names the resume it owes.
        save(&home, &[died_with_me_record(now)]).unwrap();
        assert_eq!(
            lookup(&home, SessionKind::Codex, "sess-abc"),
            LedgerAnswer::DiedWithMe {
                store_session_id: "sess-abc".to_string(),
                resume_argv: vec!["resume".to_string(), "sess-abc".to_string()],
            }
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_crash_mid_write_leaves_the_previous_complete_ledger() {
        let home = scratch_home("crash-torn");
        let now = now_ms();
        save(&home, &[adopted_record(now, std::process::id())]).unwrap();
        // A writer died between temp-write and rename: its temp file holds
        // garbage. The ledger must still answer from the complete file, and
        // must never read the temp.
        let path = ledger_path(&home);
        let tmp = path.with_extension(format!("json.tmp.{}", u32::MAX));
        fs::write(&tmp, b"{not json at all").unwrap();
        assert!(matches!(
            lookup(&home, SessionKind::Codex, "sess-abc"),
            LedgerAnswer::Adopted { .. }
        ));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn record_handoff_replaces_the_writers_own_records_only() {
        let home = scratch_home("one-writer");
        let now = now_ms();
        // A FOREIGN witness's record for its own row (its written_by_pid is
        // not this process).
        let mut foreign = adopted_record(now, std::process::id());
        foreign.written_by_pid = 111;
        foreign.session_id = "sess-foreign".to_string();
        save(&home, &[foreign]).unwrap();
        // THIS writer records its sweep, then a retried sweep — its own
        // previous record must be replaced, the foreign witness untouched.
        let mut first = died_with_me_record(now);
        first.written_by_pid = std::process::id();
        first.session_id = "sess-mine-1".to_string();
        write_records(&home, vec![first]).unwrap();
        let mut second = died_with_me_record(now);
        second.written_by_pid = std::process::id();
        second.session_id = "sess-mine-2".to_string();
        write_records(&home, vec![second]).unwrap();
        let records = load(&home);
        assert!(
            records
                .iter()
                .any(|r| r.session_id == "sess-foreign" && r.written_by_pid == 111)
        );
        assert!(!records.iter().any(|r| r.session_id == "sess-mine-1"));
        assert!(records.iter().any(|r| r.session_id == "sess-mine-2"));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn clear_satisfied_removes_exactly_the_satisfied_record() {
        let home = scratch_home("consume");
        let now = now_ms();
        let mut other = died_with_me_record(now);
        other.session_id = "sess-other".to_string();
        save(&home, &[died_with_me_record(now), other]).unwrap();
        clear_satisfied(&home, SessionKind::Codex, "sess-abc");
        let records = load(&home);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "sess-other");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_newest_true_record_wins_when_two_witnesses_answer() {
        let home = scratch_home("newest");
        let now = now_ms();
        let mut older = died_with_me_record(now - 1_000);
        older.written_by_pid = 111;
        let newer = adopted_record(now, std::process::id());
        save(&home, &[older, newer]).unwrap();
        // Successor adopted the row AFTER the predecessor recorded the death
        // sentence: the adoption is the newer truth.
        assert!(matches!(
            lookup(&home, SessionKind::Codex, "sess-abc"),
            LedgerAnswer::Adopted { .. }
        ));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn handoff_records_skip_adopted_keys_without_identity() {
        let identity = vec![(
            "codex:sess-abc".to_string(),
            SessionKind::Codex,
            "sess-abc".to_string(),
        )];
        let adopted = vec![
            ("codex:sess-abc".to_string(), (4242, 7)),
            ("shell:key-without-store".to_string(), (4243, 8)),
        ];
        let records = handoff_ownership_records(&identity, &adopted, 1_000, 111, "3.2.108");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "sess-abc");
        assert_eq!(
            records[0].disposition,
            OwnershipDisposition::Adopted {
                by_pid: 4242,
                by_start_time: 7
            }
        );
    }
}
