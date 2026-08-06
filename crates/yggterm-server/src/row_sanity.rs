//! Row sanity — reading the live session table and deciding what may be cleared.
//!
//! `session_tenancy` answers "what IS this row" (the [`RowHygieneVerdict`]).
//! This module answers the two questions a person actually asks of that answer:
//! **what is on my table**, and **what can go**. It owns no measurement of its
//! own; feeding it the reports is the caller's job, which is what keeps the
//! decision pure and therefore testable without a daemon.
//!
//! # The four rules are enforced HERE, not remembered by callers
//!
//! `docs/agent-row-hygiene.md` states them; a doc cannot enforce anything, so
//! every one is a branch below and every one has a test:
//!
//! 1. **Only an agent's own row is ever swept.** A row with no creator stamp is
//!    the user's and is never this policy's business, however idle it looks.
//! 2. **A row is judged only by the daemon that can see its work.** An
//!    unmeasurable row is never swept — locally, a `remote-*` row is an ssh
//!    bridge and "nothing is running" is meaningless. Sweeping on that reading
//!    would take the row the user is *currently talking to an agent through*
//!    first.
//! 3. **Clearing is two-stage and stage one is reversible.** Stage one only
//!    RECORDS that a row looked clearable; nothing is killed. A row must still
//!    be clearable a full grace period later before stage two closes it, and a
//!    row that stops being empty at any point leaves the process entirely.
//! 4. **Absence of proof keeps the row.** An unknown idle clock, a degraded
//!    walk, a missing stamp — every failure mode resolves to "leave it alone".

use serde::{Deserialize, Serialize};

use crate::session_tenancy::{RowHygieneVerdict, RowTenantReport};

/// How long an empty plate must have been silent before stage one will even
/// look at it.
///
/// Half an hour, not minutes: an agent that finished a step and is composing
/// its next command has an idle PTY, and a sweep that takes the row out from
/// under it is worse than any amount of clutter.
pub const EMPTY_PLATE_MIN_IDLE_SECS: u64 = 1_800;

/// How long stage one's record must stand before stage two may close the row.
///
/// The second look is the whole safety of the design: a row that was idle for
/// half an hour and is still idle an hour later was genuinely finished, whereas
/// one that came back to life clears its own record.
pub const STAGE_TWO_GRACE_SECS: u64 = 3_600;

/// What a row should have done to it this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepAction {
    /// Nothing. Carries the reason so a report can say WHY a row survived,
    /// which is the difference between a policy and a black box.
    Keep,
    /// Stage one: record that this row looked clearable. Kills nothing.
    Record,
    /// Stage two: this row has been clearable across the grace. It may close.
    Close,
}

/// A row's verdict for THIS round, with the reason attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowSweepDecision {
    pub session_path: String,
    pub action: SweepAction,
    /// Why — in words a person can act on, never a code.
    pub reason: String,
    /// The PTY's own silence, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_secs: Option<u64>,
}

/// One row's stage-one record: when it was FIRST seen clearable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SweepRecord {
    pub session_path: String,
    pub first_seen_clearable_ms: u64,
}

/// Decide what happens to one row.
///
/// `now_ms` and the record set are passed in rather than read, so the whole
/// policy is a pure function of its inputs and a test can pin the clock.
pub fn decide_row(
    report: &RowTenantReport,
    record: Option<&SweepRecord>,
    now_ms: u64,
    degraded: bool,
) -> RowSweepDecision {
    let session_path = report.session_path.clone();
    let keep = |reason: &str, idle: Option<u64>| RowSweepDecision {
        session_path: session_path.clone(),
        action: SweepAction::Keep,
        reason: reason.to_string(),
        idle_secs: idle,
    };

    // RULE 4, first and hardest: a walk that could not complete tells us
    // nothing about ANY row, so the whole round is off.
    if degraded {
        return keep("the measurement was degraded — nothing is swept this round", None);
    }

    let Some(hygiene) = report.hygiene.as_ref() else {
        return keep("no verdict — an unclassified row is never swept", None);
    };

    match hygiene {
        // RULE 1.
        RowHygieneVerdict::UserRow => keep("yours — not this policy's business", None),
        // RULE 2.
        RowHygieneVerdict::Unmeasurable { reason } => keep(
            &format!("cannot be judged from this host ({reason})"),
            None,
        ),
        RowHygieneVerdict::Occupied {
            tenant_count,
            oldest_tenant_age_secs,
            ..
        } => {
            let age = oldest_tenant_age_secs
                .map(|secs| format!(", oldest {}", human_duration(secs)))
                .unwrap_or_default();
            keep(
                &format!("something is running in it ({tenant_count} tenant(s){age})"),
                None,
            )
        }
        RowHygieneVerdict::EmptyPlate { idle_secs } => {
            // RULE 4 again: a plate whose age is unknown is not clearable. A
            // faked zero would be a lie, and this is the branch that refuses it.
            let Some(idle) = idle_secs else {
                return keep("empty, but its idle clock is unknown — proof is absent", None);
            };
            if *idle < EMPTY_PLATE_MIN_IDLE_SECS {
                return keep(
                    &format!(
                        "empty but only idle {} — an agent between steps looks like this",
                        human_duration(*idle)
                    ),
                    Some(*idle),
                );
            }
            // RULE 3: the two stages.
            match record {
                None => RowSweepDecision {
                    session_path,
                    action: SweepAction::Record,
                    reason: format!(
                        "empty and idle {} — recorded; closes after {} if still empty",
                        human_duration(*idle),
                        human_duration(STAGE_TWO_GRACE_SECS)
                    ),
                    idle_secs: Some(*idle),
                },
                Some(record) => {
                    let held_ms = now_ms.saturating_sub(record.first_seen_clearable_ms);
                    if held_ms / 1_000 >= STAGE_TWO_GRACE_SECS {
                        RowSweepDecision {
                            session_path,
                            action: SweepAction::Close,
                            reason: format!(
                                "empty and idle {}, and still empty {} after it was recorded",
                                human_duration(*idle),
                                human_duration(held_ms / 1_000)
                            ),
                            idle_secs: Some(*idle),
                        }
                    } else {
                        RowSweepDecision {
                            session_path,
                            action: SweepAction::Keep,
                            reason: format!(
                                "recorded {} ago — waiting out the {} grace",
                                human_duration(held_ms / 1_000),
                                human_duration(STAGE_TWO_GRACE_SECS)
                            ),
                            idle_secs: Some(*idle),
                        }
                    }
                }
            }
        }
    }
}

/// The whole table's decisions, plus the records that should be kept for next
/// round.
///
/// ⛔ A row that is no longer clearable DROPS its record — rule 3's "a row that
/// stops being empty at any point leaves the process entirely". Without this a
/// row could bank grace while it was busy and be closed the moment it went
/// quiet, which inverts the whole design.
pub fn plan_sweep(
    reports: &[RowTenantReport],
    records: &[SweepRecord],
    now_ms: u64,
    degraded: bool,
) -> (Vec<RowSweepDecision>, Vec<SweepRecord>) {
    let decisions: Vec<RowSweepDecision> = reports
        .iter()
        .map(|report| {
            let record = records
                .iter()
                .find(|record| record.session_path == report.session_path);
            decide_row(report, record, now_ms, degraded)
        })
        .collect();

    let mut next_records = Vec::new();
    for decision in &decisions {
        match decision.action {
            SweepAction::Record => next_records.push(SweepRecord {
                session_path: decision.session_path.clone(),
                first_seen_clearable_ms: now_ms,
            }),
            SweepAction::Close => {
                // The row is going; its record goes with it.
            }
            SweepAction::Keep => {
                // Carry a record ONLY while the row is still waiting out the
                // grace. Any other Keep means it stopped being clearable.
                if decision.reason.starts_with("recorded ") {
                    if let Some(existing) = records
                        .iter()
                        .find(|record| record.session_path == decision.session_path)
                    {
                        next_records.push(existing.clone());
                    }
                }
            }
        }
    }
    (decisions, next_records)
}

/// Durations a person reads without doing arithmetic.
pub fn human_duration(secs: u64) -> String {
    match secs {
        0..=89 => format!("{secs}s"),
        90..=5_399 => format!("{}m", (secs + 30) / 60),
        5_400..=86_399 => format!("{:.1}h", secs as f64 / 3_600.0),
        _ => format!("{:.1}d", secs as f64 / 86_400.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_tenancy::{RowTenantReport, TenantReportGap};

    const NOW: u64 = 10_000_000_000;

    fn report(path: &str, hygiene: RowHygieneVerdict) -> RowTenantReport {
        let mut report = RowTenantReport::unavailable(path, "runtime", TenantReportGap::NoLocalRuntime);
        report.unavailable_reason = None;
        report.unavailable_detail = None;
        report.hygiene = Some(hygiene);
        // Fixtures represent a daemon that DID check, unless a test says
        // otherwise — the unvouched case is exercised explicitly.
        report.locality_checked = true;
        report
    }

    /// ⛔⛔ THE NEAR MISS, 2026-08-06. The first real user ran this on jojo and
    /// it offered to CLOSE `live::c8a19e07` — a versestore lobe delegate five
    /// hours into its task, whose transcript had been written sixty seconds
    /// earlier. The row was created with `--machine-key dev`: registered on
    /// jojo, but the `claude` child lives on dev, so the local walk saw a PTY
    /// with no children and called it an empty plate.
    ///
    /// Rule 2 already forbade this. The rule was right; the classifier never
    /// asked where the work runs. It asks now, and an empty-looking bridge is
    /// UNMEASURABLE rather than clearable.
    #[test]
    fn a_row_whose_agent_runs_on_another_host_is_never_clearable() {
        let mut bridge = report(
            "live::c8a19e07",
            RowHygieneVerdict::EmptyPlate {
                idle_secs: Some(11_160),
            },
        );
        bridge.work_runs_on = Some("dev".to_string());
        // The verdict itself must refuse it...
        let verdict = crate::session_tenancy::row_hygiene_verdict(&bridge);
        assert!(
            matches!(verdict, RowHygieneVerdict::Unmeasurable { .. }),
            "a LiveSsh row's empty PTY is an ssh bridge, not an idle plate: {verdict:?}"
        );

        // ...and the sweep must refuse to act even if a stale record exists.
        bridge.hygiene = Some(verdict);
        let banked = SweepRecord {
            session_path: "live::c8a19e07".to_string(),
            first_seen_clearable_ms: NOW - STAGE_TWO_GRACE_SECS * 5 * 1_000,
        };
        let (decisions, next) = plan_sweep(&[bridge], &[banked], NOW, false);
        assert_eq!(
            decisions[0].action,
            SweepAction::Keep,
            "this is the decision that would have killed a working delegate"
        );
        assert!(next.is_empty(), "and its banked grace must be dropped");
    }

    /// The second line of defence, because the classifier fix lives in the
    /// DAEMON and an older one answers for its own rows without the field.
    #[test]
    fn apply_refuses_a_row_whose_locality_was_never_established() {
        // An old daemon's answer: measurable-looking, no locality field.
        let mut legacy = report(
            "live::from-an-old-daemon",
            RowHygieneVerdict::EmptyPlate {
                idle_secs: Some(9_000),
            },
        );
        legacy.locality_checked = false;
        let (decisions, _) = plan_sweep(&[legacy.clone()], &[], NOW, false);
        assert_eq!(decisions[0].action, SweepAction::Record);
        assert_eq!(
            unvouched_rows(&[legacy], &decisions).len(),
            1,
            "a row nobody vouched for must block --apply, not be swept on trust"
        );
    }

    /// RULE 1. The user's rows are not this policy's business, at any age.
    #[test]
    fn a_row_with_no_creator_stamp_is_never_swept() {
        let row = report("local://mine", RowHygieneVerdict::UserRow);
        let decision = decide_row(&row, None, NOW, false);
        assert_eq!(decision.action, SweepAction::Keep);
        assert!(decision.reason.contains("yours"));
    }

    /// RULE 2, and it is the one that would take the row the user is TALKING
    /// through: locally a remote row is an ssh bridge, so "nothing is running"
    /// is meaningless there.
    #[test]
    fn an_unmeasurable_row_is_never_swept_however_empty_it_looks() {
        let row = report(
            "remote-cc://dev/abc",
            RowHygieneVerdict::Unmeasurable {
                reason: "no_local_runtime".to_string(),
            },
        );
        let decision = decide_row(&row, None, NOW, false);
        assert_eq!(decision.action, SweepAction::Keep);
        assert!(decision.reason.contains("cannot be judged"));
    }

    /// RULE 4. A plate whose age is unknown is NOT clearable — a faked zero
    /// would be the dishonesty the verdict exists to remove.
    #[test]
    fn an_empty_plate_with_no_idle_clock_is_kept() {
        let row = report(
            "local://ageless",
            RowHygieneVerdict::EmptyPlate { idle_secs: None },
        );
        assert_eq!(decide_row(&row, None, NOW, false).action, SweepAction::Keep);
    }

    /// RULE 4 at the table level: one failed walk disarms the whole round,
    /// including rows that look perfectly clearable.
    #[test]
    fn a_degraded_measurement_sweeps_nothing_at_all() {
        let row = report(
            "local://old",
            RowHygieneVerdict::EmptyPlate {
                idle_secs: Some(99_999),
            },
        );
        let record = SweepRecord {
            session_path: "local://old".to_string(),
            first_seen_clearable_ms: 0,
        };
        let decision = decide_row(&row, Some(&record), NOW, true);
        assert_eq!(
            decision.action,
            SweepAction::Keep,
            "a walk that could not complete tells us nothing about ANY row"
        );
    }

    /// A freshly idle plate is an agent between steps, not litter.
    #[test]
    fn a_briefly_idle_plate_is_left_alone() {
        let row = report(
            "local://recent",
            RowHygieneVerdict::EmptyPlate {
                idle_secs: Some(EMPTY_PLATE_MIN_IDLE_SECS - 1),
            },
        );
        assert_eq!(decide_row(&row, None, NOW, false).action, SweepAction::Keep);
    }

    /// RULE 3. The first sight of a clearable row RECORDS; it never closes.
    #[test]
    fn stage_one_records_and_kills_nothing() {
        let row = report(
            "local://old",
            RowHygieneVerdict::EmptyPlate {
                idle_secs: Some(7_200),
            },
        );
        let decision = decide_row(&row, None, NOW, false);
        assert_eq!(decision.action, SweepAction::Record);
    }

    /// RULE 3. Stage two needs the grace to have actually elapsed.
    #[test]
    fn stage_two_waits_out_the_grace_then_closes() {
        let row = report(
            "local://old",
            RowHygieneVerdict::EmptyPlate {
                idle_secs: Some(7_200),
            },
        );
        let too_soon = SweepRecord {
            session_path: "local://old".to_string(),
            first_seen_clearable_ms: NOW - (STAGE_TWO_GRACE_SECS - 60) * 1_000,
        };
        assert_eq!(
            decide_row(&row, Some(&too_soon), NOW, false).action,
            SweepAction::Keep
        );

        let ripe = SweepRecord {
            session_path: "local://old".to_string(),
            first_seen_clearable_ms: NOW - (STAGE_TWO_GRACE_SECS + 60) * 1_000,
        };
        assert_eq!(
            decide_row(&row, Some(&ripe), NOW, false).action,
            SweepAction::Close
        );
    }

    /// ⛔ RULE 3's escape hatch, and the one that inverts the design if it is
    /// missing: a row that came back to life must LOSE its banked grace, or it
    /// would be closed the instant it next went quiet.
    #[test]
    fn a_row_that_stops_being_empty_leaves_the_process_entirely() {
        let busy = report(
            "local://revived",
            RowHygieneVerdict::Occupied {
                tenant_count: 1,
                oldest_tenant_age_secs: Some(10),
                oldest_tenant_command: Some("cargo build".to_string()),
            },
        );
        let banked = SweepRecord {
            session_path: "local://revived".to_string(),
            first_seen_clearable_ms: NOW - STAGE_TWO_GRACE_SECS * 10 * 1_000,
        };
        let (decisions, next) = plan_sweep(&[busy], &[banked], NOW, false);
        assert_eq!(decisions[0].action, SweepAction::Keep);
        assert!(
            next.is_empty(),
            "the record must be DROPPED — banking grace while busy would close \
             the row the moment it next went quiet"
        );
    }

    /// An occupied row is never clearable, but it IS the half the user cannot
    /// see today, so the reason has to name the tenant and its age.
    #[test]
    fn an_occupied_row_says_what_is_squatting_and_for_how_long() {
        let row = report(
            "local://ychrome",
            RowHygieneVerdict::Occupied {
                tenant_count: 1,
                oldest_tenant_age_secs: Some(324_000),
                oldest_tenant_command: Some("ychrome --profile ipindia".to_string()),
            },
        );
        let decision = decide_row(&row, None, NOW, false);
        assert_eq!(decision.action, SweepAction::Keep);
        assert!(decision.reason.contains("3.8d"), "{}", decision.reason);
    }

    #[test]
    fn durations_read_the_way_a_person_says_them() {
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(900), "15m");
        assert_eq!(human_duration(7_200), "2.0h");
        assert_eq!(human_duration(324_000), "3.8d");
    }
}

/// Where stage one's records live. Per host, beside the other daemon state.
pub fn sweep_records_path(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir.join("row-sweep-records.json")
}

/// Read stage one's records. Unreadable or corrupt reads as EMPTY, which per
/// rule 4 means every row starts its two stages again rather than being closed
/// on a record nobody can vouch for.
pub fn load_sweep_records(home_dir: &std::path::Path) -> Vec<SweepRecord> {
    std::fs::read_to_string(sweep_records_path(home_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Write stage one's records, atomically — a half-written record file that
/// fails to parse would silently reset every row's grace.
pub fn save_sweep_records(home_dir: &std::path::Path, records: &[SweepRecord]) {
    let path = sweep_records_path(home_dir);
    let Ok(encoded) = serde_json::to_string(records) else {
        return;
    };
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, encoded).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    } else {
        let _ = std::fs::remove_file(&temp);
    }
}

/// Rows that would be acted on but whose locality this daemon could not
/// establish — the second line of defence behind the classifier.
///
/// ⛔ The classifier now refuses a row whose work runs elsewhere, but that fix
/// lives in the DAEMON, and on a version-coexisting fleet an older daemon
/// answers for its own rows without the field. Trusting a single layer here
/// costs a live agent session, so `--apply` refuses outright rather than acting
/// on a table it cannot fully vouch for.
pub fn unvouched_rows(reports: &[RowTenantReport], decisions: &[RowSweepDecision]) -> Vec<String> {
    decisions
        .iter()
        .filter(|decision| matches!(decision.action, SweepAction::Record | SweepAction::Close))
        .filter(|decision| {
            reports
                .iter()
                .find(|report| report.session_path == decision.session_path)
                .map(|report| !report.locality_checked || report.work_runs_on.is_some())
                .unwrap_or(true)
        })
        .map(|decision| decision.session_path.clone())
        .collect()
}

/// Print the table the way a person asks about it.
///
/// Grouped by what the reader can DO about each group, not by enum order:
/// what may go, what is squatting (the half that was invisible), what is
/// waiting, and a single line for everything the policy will never touch. A
/// report that lists 38 rows flat is the clutter it was meant to describe.
pub fn print_row_sanity_report(
    reports: &[RowTenantReport],
    decisions: &[RowSweepDecision],
    degraded: bool,
    applied: bool,
) {
    let label = |path: &str| -> String {
        // The uuid tail is what distinguishes rows; the scheme is noise once
        // the group heading has said it.
        path.rsplit('/').next().unwrap_or(path).to_string()
    };
    let by_action = |want: SweepAction| -> Vec<&RowSweepDecision> {
        decisions.iter().filter(|d| d.action == want).collect()
    };

    let measured = reports
        .iter()
        .filter(|row| row.unavailable_reason.is_none())
        .count();
    println!(
        "THE TABLE — {} rows, {} measurable from here{}",
        reports.len(),
        measured,
        if degraded { " (DEGRADED)" } else { "" }
    );

    if degraded {
        println!(
            "\n  ⛔ The measurement did not complete, so nothing is swept this round.\n     \
             Absence of proof keeps the row — every one of them."
        );
        return;
    }

    let closing = by_action(SweepAction::Close);
    let recording = by_action(SweepAction::Record);

    if !closing.is_empty() {
        println!(
            "\n  {} — empty across the full grace",
            if applied { "CLOSED" } else { "WOULD CLOSE (dry run)" }
        );
        for decision in &closing {
            println!("    {}  {}", label(&decision.session_path), decision.reason);
        }
    }
    if !recording.is_empty() {
        println!(
            "\n  {} — first sight; nothing is killed",
            if applied { "RECORDED" } else { "WOULD RECORD (dry run)" }
        );
        for decision in &recording {
            println!("    {}  {}", label(&decision.session_path), decision.reason);
        }
    }

    // The occupied half: never clearable, and the thing nothing else shows.
    let mut squatting: Vec<(&RowSweepDecision, u64)> = Vec::new();
    for (decision, report) in decisions.iter().zip(reports.iter()) {
        if let Some(RowHygieneVerdict::Occupied {
            oldest_tenant_age_secs: Some(age),
            ..
        }) = report.hygiene.as_ref()
        {
            squatting.push((decision, *age));
        }
    }
    squatting.sort_by(|a, b| b.1.cmp(&a.1));
    if !squatting.is_empty() {
        println!("\n  OCCUPIED — not clearable, but this is what is holding them");
        for (decision, age) in &squatting {
            println!(
                "    {:>6}  {}",
                human_duration(*age),
                label(&decision.session_path)
            );
        }
    }

    let waiting = decisions
        .iter()
        .filter(|d| d.action == SweepAction::Keep && d.reason.starts_with("recorded "))
        .count();
    let untouchable = decisions.len() - closing.len() - recording.len() - squatting.len() - waiting;
    println!(
        "\n  {waiting} waiting out the grace · {untouchable} the policy will never touch \
         (yours, or not judgeable from here)"
    );
    if !applied && (!closing.is_empty() || !recording.is_empty()) {
        println!("\n  Nothing was changed. Re-run with --apply to act.");
    }
}
