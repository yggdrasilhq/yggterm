//! Automations — scheduled agent-CLI sessions, and the promise that they get
//! cleaned up.
//!
//! See `docs/automations.md` for the spec of record. The three decisions this
//! module implements, settled by the user on 2026-08-01:
//!
//! - **D1** — this record is the SSOT and the OS timer is a DERIVED artifact
//!   generated from it. Nothing here reads a `.timer` file back; the renderer
//!   writes them and [`sync`] reconciles them.
//! - **D2** — a run's session auto-closes on idle-TTL (through the EXISTING
//!   ephemeral reaper in `session_tenancy.rs`, not a second reaper here) while
//!   the wall-clock deadline only ever raises a notice.
//! - **D3** — a run missed to a sleeping machine is honoured late only inside
//!   its grace window.
//!
//! # Everything in this module is PURE
//!
//! No clock read, no filesystem read, no environment read reaches the decision
//! functions: `now_ms` and `utc_offset_secs` are arguments. That is the
//! no-non-determinism rule taken literally, and it is what lets every guard
//! below be tested against a fixed instant instead of against "whenever the
//! suite happened to run". The store functions at the bottom are the only ones
//! that touch a disk, and they take the path.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset, Weekday};

use yggterm_core::SessionKind;

use crate::session_tenancy::EphemeralReapReason;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Default idle-TTL before a run's session is closed by the existing reaper.
///
/// **30 minutes, and deliberately generous.** D2's accepted risk is that PTY
/// silence is not proof an agent finished — a Claude Code session paused on a
/// question is silent, and this rule will close it. The bound on that risk is
/// that the close is graceful and the CLI's own JSONL survives, so a wrong
/// close costs a `claude -r`, not the work. A tighter default would trade that
/// cheap loss for a frequent one.
pub const DEFAULT_IDLE_TTL_SECS: u64 = 30 * 60;

/// Default wall-clock budget after which a still-open run raises a NOTICE.
/// Never a close — that is the whole point of D2's split.
pub const DEFAULT_DEADLINE_SECS: u64 = 6 * 60 * 60;

/// Default D3 catch-up window: a 3 a.m. boot still runs the midnight job; a
/// 9 a.m. one does not.
pub const DEFAULT_GRACE_SECS: u64 = 6 * 60 * 60;

/// How many runs are kept per automation. Bounded so the store cannot grow
/// without limit on a fortnightly job that outlives the machine.
pub const RUN_HISTORY_LIMIT: usize = 20;

/// Filename for the automations store, under the daemon home dir
/// (`~/.yggterm/`). Kept OUT of `server-state.json`: automations are a distinct
/// concern from per-session runtime state, and folding them in would churn the
/// 52 `PersistedLiveSession` / 23 `PersistedDaemonState` literal sites for
/// nothing.
pub const AUTOMATIONS_FILE: &str = "automations.json";

pub fn automations_path(home_dir: &Path) -> PathBuf {
    home_dir.join(AUTOMATIONS_FILE)
}

// ---------------------------------------------------------------------------
// The calendar
// ---------------------------------------------------------------------------

/// The SUPPORTED SUBSET of systemd's `OnCalendar` syntax.
///
/// One syntax on all three platforms — systemd's, because it is the most
/// expressive of the three and it is what the user already thinks in. The
/// macOS and Windows renderers translate FROM this rather than each inventing a
/// dialect, which is the same single-owner rule the rest of the project runs
/// on.
///
/// **The subset is refused at create time rather than approximated.** An
/// expression we cannot evaluate exactly would schedule the user's midnight
/// infra job at some other hour, and silently: the failure mode of a guess here
/// is a job that runs while they are working, which is the one outcome the
/// whole feature exists to avoid.
pub const SUPPORTED_CALENDAR_SUBSET: &str =
    "supported: `[<weekdays> ]*-*-* HH:MM[:SS]`, plus the aliases `daily`, `weekly` \
     and `hourly`. Weekdays are `Sun`, a comma list `Mon,Wed,Fri`, or a range \
     `Mon..Fri`. Examples: `Sun *-*-* 00:00:00` (the fortnightly-infra case), \
     `*-*-* 03:30:00` (nightly), `Mon..Fri *-*-* 09:00`. Anything richer than \
     this — explicit dates, minute repetition, timezone suffixes — is REFUSED \
     rather than approximated, because a schedule we evaluate differently from \
     systemd would fire at an hour nobody chose.";

/// A parsed calendar expression. `weekdays` empty means "every day".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarExpr {
    #[serde(default)]
    pub weekdays: Vec<u8>,
    pub hour: u8,
    pub minute: u8,
    #[serde(default)]
    pub second: u8,
}

fn weekday_index(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "mon" | "monday" => 0,
        "tue" | "tues" | "tuesday" => 1,
        "wed" | "weds" | "wednesday" => 2,
        "thu" | "thur" | "thurs" | "thursday" => 3,
        "fri" | "friday" => 4,
        "sat" | "saturday" => 5,
        "sun" | "sunday" => 6,
        _ => return None,
    })
}

fn weekday_number(day: Weekday) -> u8 {
    match day {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}

fn parse_weekday_spec(spec: &str) -> Result<Vec<u8>, String> {
    let mut days = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((from, to)) = part.split_once("..") {
            let from = weekday_index(from.trim())
                .ok_or_else(|| format!("unknown weekday {:?}. {SUPPORTED_CALENDAR_SUBSET}", from))?;
            let to = weekday_index(to.trim())
                .ok_or_else(|| format!("unknown weekday {:?}. {SUPPORTED_CALENDAR_SUBSET}", to))?;
            // Ranges wrap (`Fri..Mon` is four days), because systemd's do.
            let mut day = from;
            loop {
                days.push(day);
                if day == to {
                    break;
                }
                day = (day + 1) % 7;
            }
        } else {
            days.push(
                weekday_index(part).ok_or_else(|| {
                    format!("unknown weekday {:?}. {SUPPORTED_CALENDAR_SUBSET}", part)
                })?,
            );
        }
    }
    days.sort_unstable();
    days.dedup();
    Ok(days)
}

fn parse_clock(spec: &str) -> Result<(u8, u8, u8), String> {
    let mut parts = spec.split(':');
    let hour = parts.next().unwrap_or_default();
    let minute = parts
        .next()
        .ok_or_else(|| format!("{spec:?} is not a HH:MM[:SS] time. {SUPPORTED_CALENDAR_SUBSET}"))?;
    let second = parts.next().unwrap_or("0");
    if parts.next().is_some() {
        return Err(format!(
            "{spec:?} has too many `:` groups. {SUPPORTED_CALENDAR_SUBSET}"
        ));
    }
    let parse = |raw: &str, name: &str, max: u8| -> Result<u8, String> {
        raw.parse::<u8>()
            .ok()
            .filter(|value| *value <= max)
            .ok_or_else(|| format!("{name} {raw:?} is out of range. {SUPPORTED_CALENDAR_SUBSET}"))
    };
    Ok((
        parse(hour, "hour", 23)?,
        parse(minute, "minute", 59)?,
        parse(second, "second", 59)?,
    ))
}

/// Parse the supported subset, or refuse with a message that TEACHES the
/// subset rather than merely rejecting.
pub fn parse_calendar(expr: &str) -> Result<CalendarExpr, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(format!("empty calendar expression. {SUPPORTED_CALENDAR_SUBSET}"));
    }
    match expr.to_ascii_lowercase().as_str() {
        "daily" | "midnight" => {
            return Ok(CalendarExpr {
                weekdays: Vec::new(),
                hour: 0,
                minute: 0,
                second: 0,
            });
        }
        "weekly" => {
            return Ok(CalendarExpr {
                weekdays: vec![0],
                hour: 0,
                minute: 0,
                second: 0,
            });
        }
        "hourly" => {
            return Err(format!(
                "`hourly` fires 24 times a day and an automation opens an agent session each \
                 time. {SUPPORTED_CALENDAR_SUBSET}"
            ));
        }
        _ => {}
    }

    let tokens: Vec<&str> = expr.split_whitespace().collect();
    let (weekdays, date_and_time) = match tokens.as_slice() {
        [date, clock] if date.contains('-') => (Vec::new(), [*date, *clock]),
        [days, date, clock] => (parse_weekday_spec(days)?, [*date, *clock]),
        [clock] if clock.contains(':') => (Vec::new(), ["*-*-*", *clock]),
        _ => {
            return Err(format!(
                "cannot read {expr:?} as a calendar expression. {SUPPORTED_CALENDAR_SUBSET}"
            ));
        }
    };
    if date_and_time[0] != "*-*-*" {
        return Err(format!(
            "{:?} pins a date, and this scheduler evaluates recurring calendars only. \
             {SUPPORTED_CALENDAR_SUBSET}",
            date_and_time[0]
        ));
    }
    let (hour, minute, second) = parse_clock(date_and_time[1])?;
    Ok(CalendarExpr {
        weekdays,
        hour,
        minute,
        second,
    })
}

impl CalendarExpr {
    fn matches_weekday(&self, day: Weekday) -> bool {
        self.weekdays.is_empty() || self.weekdays.contains(&weekday_number(day))
    }

    /// The instant this expression denotes on `date`, in the given offset.
    fn instant_on(&self, date: Date, utc_offset_secs: i32) -> Option<i64> {
        let offset = UtcOffset::from_whole_seconds(utc_offset_secs).ok()?;
        let clock = Time::from_hms(self.hour, self.minute, self.second).ok()?;
        Some(
            PrimitiveDateTime::new(date, clock)
                .assume_offset(offset)
                .unix_timestamp(),
        )
    }

    /// The most recent occurrence at or before `now_ms`.
    ///
    /// **This, and not a stored `next_run_at`, is what the grace guard measures
    /// against.** systemd does not tell a late `Persistent=true` fire when it
    /// was originally due, and a due-instant we persisted ourselves is state
    /// that can drift out of step with the timer that actually fires. Deriving
    /// it from the calendar is stateless and self-correcting: the same
    /// expression and the same instant always yield the same answer, whoever is
    /// asking and however many restarts ago the record was written.
    pub fn previous_occurrence_at_or_before(
        &self,
        now_ms: u64,
        utc_offset_secs: i32,
    ) -> Option<u64> {
        let offset = UtcOffset::from_whole_seconds(utc_offset_secs).ok()?;
        let now = OffsetDateTime::from_unix_timestamp((now_ms / 1000) as i64)
            .ok()?
            .to_offset(offset);
        let mut date = now.date();
        // 8 days covers every weekday selection, including a single one.
        for _ in 0..8 {
            if self.matches_weekday(date.weekday()) {
                if let Some(instant) = self.instant_on(date, utc_offset_secs) {
                    if instant * 1000 <= now_ms as i64 {
                        return u64::try_from(instant * 1000).ok();
                    }
                }
            }
            date = date.previous_day()?;
        }
        None
    }

    /// The first occurrence strictly after `after_ms`. Used to show the user
    /// (and the agent plane) when a job will next run without shelling out to
    /// `systemd-analyze calendar`.
    pub fn next_occurrence_after(&self, after_ms: u64, utc_offset_secs: i32) -> Option<u64> {
        let offset = UtcOffset::from_whole_seconds(utc_offset_secs).ok()?;
        let after = OffsetDateTime::from_unix_timestamp((after_ms / 1000) as i64)
            .ok()?
            .to_offset(offset);
        let mut date = after.date();
        for _ in 0..8 {
            if self.matches_weekday(date.weekday()) {
                if let Some(instant) = self.instant_on(date, utc_offset_secs) {
                    if instant * 1000 > after_ms as i64 {
                        return u64::try_from(instant * 1000).ok();
                    }
                }
            }
            date = date.next_day()?;
        }
        None
    }

    /// Render back to `OnCalendar` for the generated systemd unit. Round-trips
    /// with [`parse_calendar`], which is the property that keeps the unit and
    /// the record from meaning different things.
    pub fn to_on_calendar(&self) -> String {
        let days = if self.weekdays.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = self
                .weekdays
                .iter()
                .map(|day| match day {
                    0 => "Mon",
                    1 => "Tue",
                    2 => "Wed",
                    3 => "Thu",
                    4 => "Fri",
                    5 => "Sat",
                    _ => "Sun",
                })
                .collect();
            format!("{} ", names.join(","))
        };
        format!(
            "{days}*-*-* {:02}:{:02}:{:02}",
            self.hour, self.minute, self.second
        )
    }
}

/// The ISO-8601 week number of an instant, in the given offset.
///
/// The input to the every-N-weeks parity guard. ISO weeks, not "days since
/// epoch / 7", because ISO weeks start on Monday and are what a human means by
/// "every other week"; the two disagree by a day and the disagreement would
/// show up as a job that fires on the wrong Sunday.
pub fn iso_week_number(instant_ms: u64, utc_offset_secs: i32) -> Option<u32> {
    let offset = UtcOffset::from_whole_seconds(utc_offset_secs).ok()?;
    let at = OffsetDateTime::from_unix_timestamp((instant_ms / 1000) as i64)
        .ok()?
        .to_offset(offset);
    // ISO week alone repeats every year, so a bare `week % 2` flips whenever a
    // year holds 53 weeks. Counting whole weeks from a fixed epoch instead
    // makes the parity monotonic across year boundaries — which is what "every
    // two weeks, forever" actually means.
    let epoch = Date::from_calendar_date(1970, Month::January, 5).ok()?; // a Monday
    let days = (at.date() - epoch).whole_days();
    u32::try_from(days.div_euclid(7)).ok()
}

// ---------------------------------------------------------------------------
// The record
// ---------------------------------------------------------------------------

/// Why a run ended up the way it did. Every value is recorded on the run and
/// readable through `automation runs`, so "why did my session vanish" and "why
/// did my job not run" both have an answer that is not a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// A session was opened and the prompt was injected.
    Ran,
    /// D3: the fire arrived later than `grace_secs` after the due instant.
    SkippedOutOfGrace,
    /// The every-N-weeks parity guard said this was an off week.
    SkippedOffCadence,
    /// E1: the previous run's session was still live, so it was re-prompted
    /// instead of a duplicate being spawned.
    ReusedLiveSession,
    /// The session could not be opened at all.
    SpawnFailed,
}

impl RunOutcome {
    /// Whether the generated unit should exit 0 for this outcome.
    ///
    /// A SKIP IS A SUCCESS. Both skips are the designed behaviour, and a unit
    /// that exited non-zero on them would leave `systemctl --user list-timers`
    /// reporting a permanently failed timer for a job that is working exactly
    /// as specified — which is worse than useless, because it trains the user
    /// to ignore the one place the OS reports real failures.
    pub fn is_success(self) -> bool {
        !matches!(self, RunOutcome::SpawnFailed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RunOutcome::Ran => "ran",
            RunOutcome::SkippedOutOfGrace => "skipped_out_of_grace",
            RunOutcome::SkippedOffCadence => "skipped_off_cadence",
            RunOutcome::ReusedLiveSession => "reused_live_session",
            RunOutcome::SpawnFailed => "spawn_failed",
        }
    }
}

/// How a run's session ended. `Never` is the honest answer while it is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// The existing ephemeral reaper closed it on the idle rule (D2).
    EphemeralIdleTtl,
    /// The existing ephemeral reaper closed it because its owner went away.
    EphemeralOwnerGone,
    /// A human closed it.
    User,
    /// Still open.
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRun {
    pub run_id: String,
    /// The instant the calendar said this run was for — DERIVED from the
    /// expression, never read back from the timer.
    pub due_at_ms: u64,
    pub started_at_ms: u64,
    pub outcome: RunOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at_ms: Option<u64>,
    #[serde(default = "close_reason_never")]
    pub close_reason: CloseReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn close_reason_never() -> CloseReason {
    CloseReason::Never
}

impl AutomationRun {
    /// Whether this run still holds an open session — the predicate both the
    /// deadline chore and E1's reuse check read, so they cannot disagree about
    /// what "still running" means.
    pub fn is_open(&self) -> bool {
        self.session_path.is_some() && self.closed_at_ms.is_none()
    }
}

/// A scheduled automation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Automation {
    /// Stable slug. Also the generated unit's filename, so it is constrained to
    /// what systemd and every filesystem accept — see [`validate_id`].
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which CLI the session runs. `SessionKind`, never a Codex-or-CC boolean:
    /// a future first-class agent CLI must not need this field widened.
    pub agent_kind: SessionKind,
    pub machine_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub prompt: String,
    pub calendar: CalendarExpr,
    /// 1 = every occurrence. 2 = the user's fortnightly job.
    #[serde(default = "default_every_n")]
    pub every_n_weeks: u32,
    /// The week-count this automation's parity is measured against, stamped at
    /// create time. Stored rather than derived so that editing the calendar
    /// never silently shifts which fortnight the job lands in.
    #[serde(default)]
    pub anchor_week: u32,
    #[serde(default = "default_grace")]
    pub grace_secs: u64,
    #[serde(default = "default_idle_ttl")]
    pub idle_ttl_secs: u64,
    #[serde(default = "default_deadline")]
    pub deadline_secs: u64,
    /// false ⇒ `--no-activate`. **Default false**: at midnight nobody is
    /// watching, and a scheduled run that steals focus is a bug.
    #[serde(default)]
    pub attach: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at_ms: Option<u64>,
    #[serde(default)]
    pub runs: Vec<AutomationRun>,
}

fn default_true() -> bool {
    true
}
fn default_every_n() -> u32 {
    1
}
fn default_grace() -> u64 {
    DEFAULT_GRACE_SECS
}
fn default_idle_ttl() -> u64 {
    DEFAULT_IDLE_TTL_SECS
}
fn default_deadline() -> u64 {
    DEFAULT_DEADLINE_SECS
}

/// An id has to survive being a systemd unit filename and a path component.
pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("an automation id is 1..=64 characters".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "automation id {id:?} may hold only ASCII letters, digits, `-` and `_` — it becomes a \
             systemd unit filename, and anything else there is a unit that cannot be enabled"
        ));
    }
    Ok(())
}

impl Automation {
    /// The most recent run, if any. Runs are kept newest-last.
    pub fn latest_run(&self) -> Option<&AutomationRun> {
        self.runs.last()
    }

    /// E1's question: does this automation already hold a live session?
    pub fn open_run(&self) -> Option<&AutomationRun> {
        self.runs.iter().rev().find(|run| run.is_open())
    }

    pub fn record_run(&mut self, run: AutomationRun) {
        self.last_run_at_ms = Some(run.started_at_ms);
        self.runs.push(run);
        if self.runs.len() > RUN_HISTORY_LIMIT {
            let excess = self.runs.len() - RUN_HISTORY_LIMIT;
            self.runs.drain(0..excess);
        }
    }

    /// The next instant this automation will fire and be HONOURED — the parity
    /// guard is applied, so a fortnightly job reports the Sunday it will
    /// actually work on rather than the one the timer merely wakes on.
    pub fn next_honoured_run_after(&self, after_ms: u64, utc_offset_secs: i32) -> Option<u64> {
        let mut cursor = after_ms;
        for _ in 0..64 {
            let candidate = self.calendar.next_occurrence_after(cursor, utc_offset_secs)?;
            if self.cadence_honours(candidate, utc_offset_secs) {
                return Some(candidate);
            }
            cursor = candidate;
        }
        None
    }

    /// The every-N-weeks parity guard.
    ///
    /// `OnCalendar` cannot say "every second Sunday", so the timer fires every
    /// Sunday and this decides. Deterministic from the week-count of the due
    /// instant: same input, same answer, forever — no counter to drift and no
    /// state to lose across a reinstall.
    pub fn cadence_honours(&self, due_at_ms: u64, utc_offset_secs: i32) -> bool {
        if self.every_n_weeks <= 1 {
            return true;
        }
        let Some(week) = iso_week_number(due_at_ms, utc_offset_secs) else {
            // Cannot compute parity ⇒ do not silently skip the user's job.
            return true;
        };
        week % self.every_n_weeks == self.anchor_week % self.every_n_weeks
    }
}

/// D3's verdict on a fire that may have arrived late.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraceVerdict {
    /// Inside the window (including exactly on time).
    Honour,
    /// Later than `due + grace`. The run is skipped and rescheduled.
    OutOfGrace { late_by_secs: u64 },
}

/// D3, in one expression.
///
/// Note the direction: a fire that arrives EARLY is honoured. systemd can wake
/// a timer a moment before its calendar instant, and refusing that would drop
/// runs for a rounding error.
pub fn grace_verdict(due_at_ms: u64, now_ms: u64, grace_secs: u64) -> GraceVerdict {
    let late_by_ms = now_ms.saturating_sub(due_at_ms);
    if late_by_ms <= grace_secs.saturating_mul(1000) {
        GraceVerdict::Honour
    } else {
        GraceVerdict::OutOfGrace {
            late_by_secs: late_by_ms / 1000,
        }
    }
}

/// Whether a still-open run has burned its wall-clock budget and should raise a
/// notice. **This never closes anything** — that asymmetry IS D2.
pub fn run_has_passed_deadline(run: &AutomationRun, deadline_secs: u64, now_ms: u64) -> bool {
    run.is_open() && now_ms.saturating_sub(run.started_at_ms) > deadline_secs.saturating_mul(1000)
}

// ---------------------------------------------------------------------------
// Notices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// A run passed its deadline with its session still open.
    RunOverdue,
    /// A run could not open its session at all.
    SpawnFailed,
}

/// A PERSISTING notice, in the strict sense the user asked for: it survives a
/// GUI restart, a daemon restart and a reboot, and is cleared ONLY by the user
/// acting on it. Never by a timeout, and never by having been displayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationNotice {
    pub run_id: String,
    pub automation_id: String,
    pub kind: NoticeKind,
    pub raised_at_ms: u64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Bookkeeping — what the daemon chore does between runs
// ---------------------------------------------------------------------------

/// What one bookkeeping pass changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookkeepingOutcome {
    /// `(run_id, reason)` for runs whose session went away since last tick.
    pub closed: Vec<(String, CloseReason)>,
    /// Run ids that newly raised an overdue notice.
    pub raised: Vec<String>,
}

impl BookkeepingOutcome {
    pub fn did_anything(&self) -> bool {
        !self.closed.is_empty() || !self.raised.is_empty()
    }
}

/// Reconcile the store against the world, once.
///
/// Two jobs, and the FIRST one is a correctness fix rather than housekeeping.
/// Nothing else ever stamps `closed_at_ms`, so without this a finished run stays
/// `is_open()` forever — and E1 would then re-prompt a session that no longer
/// exists instead of spawning a fresh one, every fortnight, silently.
///
/// `live_sessions` is every session path the daemon currently holds. `reaped`
/// is what the ephemeral reaper closed on THIS tick, which is the only source
/// that can distinguish "the TTL closed it" from "the user did" — a row that is
/// simply absent gets [`CloseReason::User`], because that is the honest reading
/// of "gone, and not by us".
///
/// Pure: `now_ms` is an argument, and the caller supplies the world.
pub fn bookkeeping_pass(
    store: &mut AutomationStore,
    live_sessions: &[String],
    reaped: &[(String, EphemeralReapReason)],
    now_ms: u64,
) -> BookkeepingOutcome {
    let mut outcome = BookkeepingOutcome::default();
    let mut overdue: Vec<AutomationNotice> = Vec::new();

    for automation in &mut store.automations {
        let deadline_secs = automation.deadline_secs;
        let automation_id = automation.id.clone();
        for run in &mut automation.runs {
            if !run.is_open() {
                continue;
            }
            let Some(session_path) = run.session_path.clone() else {
                continue;
            };
            if !live_sessions.iter().any(|live| live == &session_path) {
                let reason = reaped
                    .iter()
                    .find(|(path, _)| path == &session_path)
                    .map(|(_, why)| match why {
                        EphemeralReapReason::IdleTtl => CloseReason::EphemeralIdleTtl,
                        EphemeralReapReason::OwnerGone => CloseReason::EphemeralOwnerGone,
                    })
                    .unwrap_or(CloseReason::User);
                run.closed_at_ms = Some(now_ms);
                run.close_reason = reason;
                outcome.closed.push((run.run_id.clone(), reason));
                continue;
            }
            // D2: the deadline NEVER closes. It only ever names the run.
            if run_has_passed_deadline(run, deadline_secs, now_ms) {
                overdue.push(AutomationNotice {
                    run_id: run.run_id.clone(),
                    automation_id: automation_id.clone(),
                    kind: NoticeKind::RunOverdue,
                    raised_at_ms: now_ms,
                    message: format!(
                        "still running {}s after it started, past a {deadline_secs}s budget — \
                         left alone deliberately; close it yourself if it is stuck",
                        now_ms.saturating_sub(run.started_at_ms) / 1000
                    ),
                    session_path: Some(session_path),
                });
            }
        }
    }

    for notice in overdue {
        let run_id = notice.run_id.clone();
        if store.raise_notice(notice) {
            outcome.raised.push(run_id);
        }
    }
    outcome
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationStore {
    #[serde(default)]
    pub automations: Vec<Automation>,
    #[serde(default)]
    pub notices: Vec<AutomationNotice>,
}

impl AutomationStore {
    pub fn get(&self, id: &str) -> Option<&Automation> {
        self.automations.iter().find(|entry| entry.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Automation> {
        self.automations.iter_mut().find(|entry| entry.id == id)
    }

    /// Insert or replace by id, preserving insertion order for a replace.
    pub fn upsert(&mut self, automation: Automation) -> Option<Automation> {
        match self.get_mut(&automation.id) {
            Some(slot) => Some(std::mem::replace(slot, automation)),
            None => {
                self.automations.push(automation);
                None
            }
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<Automation> {
        let index = self.automations.iter().position(|entry| entry.id == id)?;
        // A removed automation's notices go with it: a notice about a job that
        // no longer exists is one the user can never act on, and a nag with no
        // remedy trains them to ignore the whole surface.
        self.notices.retain(|notice| notice.automation_id != id);
        Some(self.automations.remove(index))
    }

    /// Raise a notice, or leave the existing one alone. Idempotent BY RUN: the
    /// deadline chore ticks repeatedly over the same overdue run, and a notice
    /// that re-raised on every tick would be a counter, not a notice.
    pub fn raise_notice(&mut self, notice: AutomationNotice) -> bool {
        if self
            .notices
            .iter()
            .any(|existing| existing.run_id == notice.run_id && existing.kind == notice.kind)
        {
            return false;
        }
        self.notices.push(notice);
        true
    }

    pub fn dismiss_notice(&mut self, run_id: &str) -> usize {
        let before = self.notices.len();
        self.notices.retain(|notice| notice.run_id != run_id);
        before - self.notices.len()
    }

    /// THE derived answer to "is this session an automated one".
    ///
    /// There is deliberately no `automated` flag on the session. One owner for
    /// the question means Live and Automated are filtered VIEWS over one store
    /// and the cwd-tree node never moves — E3, and the reason the first draft
    /// dropped the flag it started with.
    pub fn automation_for_session(&self, session_path: &str) -> Option<&Automation> {
        self.automations.iter().find(|automation| {
            automation
                .runs
                .iter()
                .any(|run| run.is_open() && run.session_path.as_deref() == Some(session_path))
        })
    }

    pub fn session_is_automated(&self, session_path: &str) -> bool {
        self.automation_for_session(session_path).is_some()
    }
}

/// Load the store. A missing file is an empty store (first run); a corrupt one
/// is an error the caller decides about rather than a silent reset — losing a
/// user's schedule to a parse slip is not a recovery.
pub fn load_store(home_dir: &Path) -> std::io::Result<AutomationStore> {
    let path = automations_path(home_dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(AutomationStore::default())
        }
        Err(error) => Err(error),
    }
}

/// Persist atomically (write-temp-then-rename) so a crash mid-write cannot
/// leave the user with a half-written schedule.
pub fn save_store(home_dir: &Path, store: &AutomationStore) -> std::io::Result<()> {
    let path = automations_path(home_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(store)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-02 is a Sunday. Every instant below is built from this so the
    /// suite never asks what day it is today.
    const SUN_2026_08_02_MIDNIGHT_UTC_MS: u64 = 1_785_628_800_000;
    const IST: i32 = 5 * 3600 + 1800;

    fn at(year: i32, month: Month, day: u8, hour: u8, minute: u8, offset_secs: i32) -> u64 {
        let date = Date::from_calendar_date(year, month, day).unwrap();
        let clock = Time::from_hms(hour, minute, 0).unwrap();
        let offset = UtcOffset::from_whole_seconds(offset_secs).unwrap();
        (PrimitiveDateTime::new(date, clock)
            .assume_offset(offset)
            .unix_timestamp() as u64)
            * 1000
    }

    fn fortnightly_infra_job() -> Automation {
        Automation {
            id: "infra-upgrade".to_string(),
            enabled: true,
            agent_kind: SessionKind::ClaudeCode,
            machine_key: "jojo".to_string(),
            cwd: Some("/home/user/gh/yggterm".to_string()),
            prompt: "some time has passed, can you upgrade again".to_string(),
            calendar: parse_calendar("Sun *-*-* 00:00:00").unwrap(),
            every_n_weeks: 2,
            anchor_week: 0,
            grace_secs: DEFAULT_GRACE_SECS,
            idle_ttl_secs: DEFAULT_IDLE_TTL_SECS,
            deadline_secs: DEFAULT_DEADLINE_SECS,
            attach: false,
            title: None,
            created_at_ms: 0,
            last_run_at_ms: None,
            runs: Vec::new(),
        }
    }

    fn run_at(started_at_ms: u64, session: Option<&str>) -> AutomationRun {
        AutomationRun {
            run_id: format!("run-{started_at_ms}"),
            due_at_ms: started_at_ms,
            started_at_ms,
            outcome: RunOutcome::Ran,
            session_path: session.map(str::to_string),
            closed_at_ms: None,
            close_reason: CloseReason::Never,
            error: None,
        }
    }

    // ---- the calendar ----

    #[test]
    fn the_users_own_expression_parses_to_sunday_midnight() {
        let expr = parse_calendar("Sun *-*-* 00:00:00").unwrap();
        assert_eq!(expr.weekdays, vec![6]);
        assert_eq!((expr.hour, expr.minute, expr.second), (0, 0, 0));
    }

    #[test]
    fn a_calendar_expression_round_trips_through_the_unit_it_generates() {
        // The unit and the record must not be able to mean different things.
        for raw in [
            "Sun *-*-* 00:00:00",
            "Mon,Wed,Fri *-*-* 09:30:00",
            "*-*-* 03:30:00",
        ] {
            let parsed = parse_calendar(raw).unwrap();
            let rendered = parsed.to_on_calendar();
            assert_eq!(
                parse_calendar(&rendered).unwrap(),
                parsed,
                "{raw} rendered to {rendered} and came back different"
            );
        }
    }

    #[test]
    fn a_weekday_range_wraps_the_way_systemds_does() {
        assert_eq!(parse_calendar("Mon..Fri *-*-* 09:00").unwrap().weekdays, vec![0, 1, 2, 3, 4]);
        // Fri..Mon is four days, not an empty set.
        assert_eq!(parse_calendar("Fri..Mon *-*-* 09:00").unwrap().weekdays, vec![0, 4, 5, 6]);
    }

    #[test]
    fn an_expression_outside_the_subset_is_refused_and_teaches_the_subset() {
        for raw in ["2026-08-02 00:00:00", "hourly", "*:0/15", ""] {
            let error = parse_calendar(raw).unwrap_err();
            assert!(
                error.contains("supported:") || error.contains("agent session"),
                "{raw:?} was refused without teaching the subset: {error}"
            );
        }
    }

    #[test]
    fn a_pinned_date_is_refused_rather_than_approximated() {
        // The failure mode of guessing here is the user's midnight job running
        // at an hour nobody chose.
        assert!(parse_calendar("2026-08-02 00:00:00").is_err());
    }

    #[test]
    fn the_previous_occurrence_is_derived_not_stored() {
        let expr = parse_calendar("Sun *-*-* 00:00:00").unwrap();
        // 3 a.m. on that Sunday, IST.
        let now = at(2026, Month::August, 2, 3, 0, IST);
        let due = expr.previous_occurrence_at_or_before(now, IST).unwrap();
        assert_eq!(due, at(2026, Month::August, 2, 0, 0, IST));
    }

    #[test]
    fn the_previous_occurrence_reaches_back_a_whole_week_for_a_single_weekday() {
        let expr = parse_calendar("Sun *-*-* 00:00:00").unwrap();
        // Wednesday afternoon: the last Sunday was four days ago.
        let now = at(2026, Month::August, 5, 14, 0, IST);
        let due = expr.previous_occurrence_at_or_before(now, IST).unwrap();
        assert_eq!(due, at(2026, Month::August, 2, 0, 0, IST));
    }

    #[test]
    fn the_next_occurrence_is_strictly_after_the_instant_given() {
        let expr = parse_calendar("Sun *-*-* 00:00:00").unwrap();
        let midnight = at(2026, Month::August, 2, 0, 0, IST);
        assert_eq!(
            expr.next_occurrence_after(midnight, IST).unwrap(),
            at(2026, Month::August, 9, 0, 0, IST),
            "standing exactly on an occurrence must yield the NEXT one, not this one"
        );
    }

    #[test]
    fn the_offset_is_an_argument_so_midnight_means_local_midnight() {
        let expr = parse_calendar("Sun *-*-* 00:00:00").unwrap();
        let ist = expr
            .next_occurrence_after(SUN_2026_08_02_MIDNIGHT_UTC_MS, IST)
            .unwrap();
        let utc = expr
            .next_occurrence_after(SUN_2026_08_02_MIDNIGHT_UTC_MS, 0)
            .unwrap();
        assert_ne!(
            ist, utc,
            "if these agreed, a timezone bug would be invisible to this suite"
        );
    }

    // ---- D3, the grace guard ----

    #[test]
    fn a_three_am_boot_still_runs_the_midnight_job() {
        let due = at(2026, Month::August, 2, 0, 0, IST);
        let boot = at(2026, Month::August, 2, 3, 0, IST);
        assert_eq!(
            grace_verdict(due, boot, DEFAULT_GRACE_SECS),
            GraceVerdict::Honour
        );
    }

    #[test]
    fn a_nine_am_boot_does_not_ambush_the_user() {
        let due = at(2026, Month::August, 2, 0, 0, IST);
        let boot = at(2026, Month::August, 2, 9, 0, IST);
        assert!(matches!(
            grace_verdict(due, boot, DEFAULT_GRACE_SECS),
            GraceVerdict::OutOfGrace { .. }
        ));
    }

    #[test]
    fn a_wednesday_afternoon_boot_never_runs_sundays_job() {
        let due = at(2026, Month::August, 2, 0, 0, IST);
        let boot = at(2026, Month::August, 5, 14, 0, IST);
        assert!(matches!(
            grace_verdict(due, boot, DEFAULT_GRACE_SECS),
            GraceVerdict::OutOfGrace { .. }
        ));
    }

    #[test]
    fn an_early_fire_is_honoured_because_a_rounding_error_must_not_drop_a_run() {
        let due = at(2026, Month::August, 2, 0, 0, IST);
        assert_eq!(
            grace_verdict(due, due - 2_000, DEFAULT_GRACE_SECS),
            GraceVerdict::Honour
        );
    }

    #[test]
    fn exactly_on_the_grace_boundary_is_still_honoured() {
        let due = at(2026, Month::August, 2, 0, 0, IST);
        assert_eq!(
            grace_verdict(due, due + DEFAULT_GRACE_SECS * 1000, DEFAULT_GRACE_SECS),
            GraceVerdict::Honour
        );
    }

    // ---- the every-N-weeks parity guard ----

    #[test]
    fn a_fortnightly_job_honours_every_other_sunday_and_no_other() {
        let mut job = fortnightly_infra_job();
        let first = at(2026, Month::August, 2, 0, 0, IST);
        // Anchor to the first Sunday, then walk eight weeks.
        job.anchor_week = iso_week_number(first, IST).unwrap();
        let honoured: Vec<bool> = (0..8u64)
            .map(|week| {
                let sunday = first + week * 7 * DAY_MS;
                job.cadence_honours(sunday, IST)
            })
            .collect();
        assert_eq!(
            honoured,
            vec![true, false, true, false, true, false, true, false]
        );
    }

    #[test]
    fn the_parity_guard_does_not_flip_across_a_year_boundary() {
        // ISO week numbers restart every year and some years hold 53 weeks, so
        // a bare `iso_week % 2` would flip parity at new year and the job would
        // silently move to the other fortnight.
        let mut job = fortnightly_infra_job();
        let anchor = at(2026, Month::December, 6, 0, 0, IST);
        job.anchor_week = iso_week_number(anchor, IST).unwrap();
        for week in 0..12u64 {
            let sunday = anchor + week * 7 * DAY_MS;
            assert_eq!(
                job.cadence_honours(sunday, IST),
                week % 2 == 0,
                "week {week} after the anchor broke parity across the year boundary"
            );
        }
    }

    #[test]
    fn every_n_weeks_of_one_honours_everything() {
        let mut job = fortnightly_infra_job();
        job.every_n_weeks = 1;
        for week in 0..5u64 {
            let sunday = at(2026, Month::August, 2, 0, 0, IST) + week * 7 * DAY_MS;
            assert!(job.cadence_honours(sunday, IST));
        }
    }

    #[test]
    fn the_next_honoured_run_skips_the_off_week() {
        let mut job = fortnightly_infra_job();
        let first = at(2026, Month::August, 2, 0, 0, IST);
        job.anchor_week = iso_week_number(first, IST).unwrap();
        // Standing on an honoured Sunday, the next honoured one is a fortnight
        // out — NOT the Sunday the timer also wakes on.
        assert_eq!(
            job.next_honoured_run_after(first, IST).unwrap(),
            at(2026, Month::August, 16, 0, 0, IST)
        );
    }

    // ---- D2, the deadline that never closes ----

    #[test]
    fn an_overdue_run_is_named_but_the_verdict_never_closes_it() {
        let run = run_at(1_000_000, Some("live/jojo/0"));
        assert!(run_has_passed_deadline(&run, 60, 1_000_000 + 61_000));
        // The only thing the module offers is the QUESTION. There is no close
        // path in this file at all — that is D2's asymmetry, and it is enforced
        // by the reaper living in session_tenancy.rs and nowhere else.
        assert!(run.is_open(), "asking must not have mutated the run");
    }

    #[test]
    fn a_closed_run_is_never_overdue_however_long_it_ran() {
        let mut run = run_at(1_000_000, Some("live/jojo/0"));
        run.closed_at_ms = Some(1_000_500);
        run.close_reason = CloseReason::EphemeralIdleTtl;
        assert!(!run_has_passed_deadline(&run, 1, u64::MAX / 2));
    }

    #[test]
    fn a_run_that_never_opened_a_session_is_never_overdue() {
        let run = run_at(1_000_000, None);
        assert!(!run_has_passed_deadline(&run, 1, 1_000_000 + 999_999));
    }

    // ---- outcomes ----

    #[test]
    fn both_skips_exit_zero_so_the_timer_never_reads_as_failed() {
        assert!(RunOutcome::SkippedOutOfGrace.is_success());
        assert!(RunOutcome::SkippedOffCadence.is_success());
        assert!(RunOutcome::Ran.is_success());
        assert!(RunOutcome::ReusedLiveSession.is_success());
        assert!(!RunOutcome::SpawnFailed.is_success());
    }

    #[test]
    fn outcome_names_are_the_documented_ones() {
        // These strings are the contract with `automation runs --json` and with
        // docs/automations.md. Renaming one silently is a broken promise to
        // whoever is parsing it.
        assert_eq!(RunOutcome::Ran.as_str(), "ran");
        assert_eq!(
            RunOutcome::SkippedOutOfGrace.as_str(),
            "skipped_out_of_grace"
        );
        assert_eq!(
            RunOutcome::SkippedOffCadence.as_str(),
            "skipped_off_cadence"
        );
        assert_eq!(RunOutcome::ReusedLiveSession.as_str(), "reused_live_session");
        assert_eq!(RunOutcome::SpawnFailed.as_str(), "spawn_failed");
    }

    // ---- E1 / E3, the derived grouping ----

    #[test]
    fn an_automation_with_a_live_session_is_found_so_e1_reuses_it() {
        let mut job = fortnightly_infra_job();
        job.record_run(run_at(1_000, Some("live/jojo/3")));
        assert_eq!(job.open_run().unwrap().session_path.as_deref(), Some("live/jojo/3"));
    }

    #[test]
    fn a_closed_run_is_not_a_live_session_to_reuse() {
        let mut job = fortnightly_infra_job();
        let mut run = run_at(1_000, Some("live/jojo/3"));
        run.closed_at_ms = Some(2_000);
        job.record_run(run);
        assert!(job.open_run().is_none());
    }

    #[test]
    fn automated_is_derived_from_the_link_and_never_stored_on_the_session() {
        let mut store = AutomationStore::default();
        let mut job = fortnightly_infra_job();
        job.record_run(run_at(1_000, Some("live/jojo/3")));
        store.upsert(job);
        assert!(store.session_is_automated("live/jojo/3"));
        assert!(!store.session_is_automated("live/jojo/4"));
    }

    #[test]
    fn closing_the_run_un_automates_the_session_without_touching_it() {
        // E2: only the link changes. There is no flag on the session to unset,
        // which is exactly why there is nothing here that could go stale.
        let mut store = AutomationStore::default();
        let mut job = fortnightly_infra_job();
        job.record_run(run_at(1_000, Some("live/jojo/3")));
        store.upsert(job);
        store.get_mut("infra-upgrade").unwrap().runs[0].closed_at_ms = Some(2_000);
        assert!(!store.session_is_automated("live/jojo/3"));
    }

    // ---- the bookkeeping pass ----

    fn store_with_open_run(started_at_ms: u64) -> AutomationStore {
        let mut store = AutomationStore::default();
        let mut job = fortnightly_infra_job();
        job.record_run(run_at(started_at_ms, Some("live/jojo/3")));
        store.upsert(job);
        store
    }

    #[test]
    fn a_reaped_session_stamps_its_run_with_the_reapers_own_reason() {
        let mut store = store_with_open_run(1_000);
        let outcome = bookkeeping_pass(
            &mut store,
            &[],
            &[(
                "live/jojo/3".to_string(),
                EphemeralReapReason::IdleTtl,
            )],
            5_000,
        );
        assert_eq!(
            outcome.closed,
            vec![("run-1000".to_string(), CloseReason::EphemeralIdleTtl)]
        );
        let run = &store.automations[0].runs[0];
        assert_eq!(run.closed_at_ms, Some(5_000));
        assert_eq!(run.close_reason, CloseReason::EphemeralIdleTtl);
    }

    #[test]
    fn a_session_that_simply_vanished_is_recorded_as_closed_by_the_user() {
        // "Gone, and not by us" is the honest reading. Claiming the TTL did it
        // would put a reason in `automation runs` that never happened.
        let mut store = store_with_open_run(1_000);
        let outcome = bookkeeping_pass(&mut store, &[], &[], 5_000);
        assert_eq!(
            outcome.closed,
            vec![("run-1000".to_string(), CloseReason::User)]
        );
    }

    #[test]
    fn closing_a_run_is_what_lets_the_next_fortnight_spawn_fresh() {
        // THE correctness fix. Without this pass a finished run stays open()
        // forever and E1 re-prompts a session that no longer exists.
        let mut store = store_with_open_run(1_000);
        assert!(store.automations[0].open_run().is_some());
        bookkeeping_pass(&mut store, &[], &[], 5_000);
        assert!(store.automations[0].open_run().is_none());
    }

    #[test]
    fn a_live_session_is_left_entirely_alone() {
        let mut store = store_with_open_run(1_000);
        let outcome = bookkeeping_pass(&mut store, &["live/jojo/3".to_string()], &[], 5_000);
        assert!(!outcome.did_anything());
        assert!(store.automations[0].runs[0].is_open());
    }

    #[test]
    fn an_overdue_run_raises_a_notice_and_is_still_not_closed() {
        // D2's asymmetry, end to end.
        let mut store = store_with_open_run(1_000);
        let past_deadline = 1_000 + DEFAULT_DEADLINE_SECS * 1000 + 1;
        let outcome = bookkeeping_pass(
            &mut store,
            &["live/jojo/3".to_string()],
            &[],
            past_deadline,
        );
        assert_eq!(outcome.raised, vec!["run-1000".to_string()]);
        assert!(outcome.closed.is_empty(), "the deadline must never close");
        assert!(
            store.automations[0].runs[0].is_open(),
            "the session is still the user's to deal with"
        );
        assert_eq!(store.notices.len(), 1);
    }

    #[test]
    fn the_overdue_notice_is_raised_once_however_many_ticks_pass() {
        let mut store = store_with_open_run(1_000);
        let past = 1_000 + DEFAULT_DEADLINE_SECS * 1000 + 1;
        let live = ["live/jojo/3".to_string()];
        assert_eq!(bookkeeping_pass(&mut store, &live, &[], past).raised.len(), 1);
        for tick in 1..5 {
            assert!(
                bookkeeping_pass(&mut store, &live, &[], past + tick * 60_000)
                    .raised
                    .is_empty(),
                "a notice that re-raises every tick is a counter, not a notice"
            );
        }
        assert_eq!(store.notices.len(), 1);
    }

    #[test]
    fn a_run_inside_its_budget_raises_nothing() {
        let mut store = store_with_open_run(1_000);
        let outcome = bookkeeping_pass(
            &mut store,
            &["live/jojo/3".to_string()],
            &[],
            1_000 + DEFAULT_DEADLINE_SECS * 1000 - 1,
        );
        assert!(!outcome.did_anything());
    }

    #[test]
    fn a_skipped_run_holds_no_session_so_the_pass_ignores_it_entirely() {
        let mut store = AutomationStore::default();
        let mut job = fortnightly_infra_job();
        let mut skipped = run_at(1_000, None);
        skipped.outcome = RunOutcome::SkippedOutOfGrace;
        job.record_run(skipped);
        store.upsert(job);
        assert!(!bookkeeping_pass(&mut store, &[], &[], 9_999_999).did_anything());
    }

    // ---- history, notices, store ----

    #[test]
    fn run_history_is_bounded_and_keeps_the_newest() {
        let mut job = fortnightly_infra_job();
        for index in 0..(RUN_HISTORY_LIMIT as u64 + 5) {
            job.record_run(run_at(1_000 + index, None));
        }
        assert_eq!(job.runs.len(), RUN_HISTORY_LIMIT);
        assert_eq!(
            job.runs.last().unwrap().started_at_ms,
            1_000 + RUN_HISTORY_LIMIT as u64 + 4
        );
    }

    #[test]
    fn a_notice_is_idempotent_by_run_so_the_chore_cannot_turn_it_into_a_counter() {
        let mut store = AutomationStore::default();
        let notice = AutomationNotice {
            run_id: "run-1".to_string(),
            automation_id: "infra-upgrade".to_string(),
            kind: NoticeKind::RunOverdue,
            raised_at_ms: 10,
            message: "still running after 6h".to_string(),
            session_path: Some("live/jojo/3".to_string()),
        };
        assert!(store.raise_notice(notice.clone()));
        assert!(!store.raise_notice(notice.clone()));
        assert!(!store.raise_notice(AutomationNotice {
            raised_at_ms: 99,
            ..notice
        }));
        assert_eq!(store.notices.len(), 1);
    }

    #[test]
    fn only_dismissal_clears_a_notice() {
        let mut store = AutomationStore::default();
        store.raise_notice(AutomationNotice {
            run_id: "run-1".to_string(),
            automation_id: "infra-upgrade".to_string(),
            kind: NoticeKind::RunOverdue,
            raised_at_ms: 10,
            message: "overdue".to_string(),
            session_path: None,
        });
        assert_eq!(store.dismiss_notice("run-other"), 0, "a notice must not clear by accident");
        assert_eq!(store.dismiss_notice("run-1"), 1);
        assert!(store.notices.is_empty());
    }

    #[test]
    fn deleting_an_automation_takes_its_notices_with_it() {
        let mut store = AutomationStore::default();
        store.upsert(fortnightly_infra_job());
        store.raise_notice(AutomationNotice {
            run_id: "run-1".to_string(),
            automation_id: "infra-upgrade".to_string(),
            kind: NoticeKind::RunOverdue,
            raised_at_ms: 10,
            message: "overdue".to_string(),
            session_path: None,
        });
        store.remove("infra-upgrade");
        assert!(
            store.notices.is_empty(),
            "a notice about a job that no longer exists is one the user can never act on"
        );
    }

    #[test]
    fn upsert_replaces_in_place_and_keeps_order() {
        let mut store = AutomationStore::default();
        store.upsert(fortnightly_infra_job());
        let mut second = fortnightly_infra_job();
        second.id = "other".to_string();
        store.upsert(second);
        let mut edited = fortnightly_infra_job();
        edited.prompt = "changed".to_string();
        assert!(store.upsert(edited).is_some());
        assert_eq!(
            store
                .automations
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["infra-upgrade", "other"]
        );
        assert_eq!(store.get("infra-upgrade").unwrap().prompt, "changed");
    }

    #[test]
    fn an_id_that_could_not_be_a_unit_filename_is_refused() {
        assert!(validate_id("infra-upgrade").is_ok());
        assert!(validate_id("infra_upgrade_2").is_ok());
        for bad in ["", "has space", "has/slash", "has.dot", "unicodé"] {
            assert!(validate_id(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(validate_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn a_record_survives_a_json_round_trip_with_its_defaults() {
        let job = fortnightly_infra_job();
        let encoded = serde_json::to_string(&job).unwrap();
        assert_eq!(serde_json::from_str::<Automation>(&encoded).unwrap(), job);
        // And a record written before a field existed still loads.
        let minimal = serde_json::json!({
            "id": "legacy",
            "agent_kind": "claude_code",
            "machine_key": "jojo",
            "prompt": "go",
            "calendar": {"weekdays": [6], "hour": 0, "minute": 0},
            "created_at_ms": 0
        });
        let loaded: Automation = serde_json::from_value(minimal).unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.every_n_weeks, 1);
        assert_eq!(loaded.idle_ttl_secs, DEFAULT_IDLE_TTL_SECS);
        assert_eq!(loaded.deadline_secs, DEFAULT_DEADLINE_SECS);
        assert_eq!(loaded.grace_secs, DEFAULT_GRACE_SECS);
        assert!(!loaded.attach, "a scheduled run must not steal focus by default");
    }

    #[test]
    fn a_missing_store_is_empty_and_a_corrupt_one_is_an_error() {
        let dir = std::env::temp_dir().join(format!("ygg-automation-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_store(&dir).unwrap(), AutomationStore::default());

        let mut store = AutomationStore::default();
        store.upsert(fortnightly_infra_job());
        save_store(&dir, &store).unwrap();
        assert_eq!(load_store(&dir).unwrap(), store);

        std::fs::write(automations_path(&dir), b"{ not json").unwrap();
        assert!(
            load_store(&dir).is_err(),
            "a corrupt store must not silently become an empty one — that loses the schedule"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
