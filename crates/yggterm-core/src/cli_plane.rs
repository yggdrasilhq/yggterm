//! THE CLI PLANE — one trace category, one vocabulary, every registered agent
//! CLI.
//!
//! ## Why this module exists
//!
//! Ten agent CLIs are registered ([`crate::agent_cli::AGENT_CLIS`]) and one of
//! them — Claude Code — does very nearly everything right. The others each
//! diverge somewhere, and the diagnostic question is never *"is this CLI
//! broken"* but **"where does this CLI's path leave the one that works"**.
//!
//! That question could not be asked before this module, because the interesting
//! moments were not on the trace plane at all, or were on it in four different
//! vocabularies:
//!
//! * a row's BIRTH was traced as `server/session/live_session_birth`, which
//!   names the prior active pointer (it was added for a phantom-spawn hunt) and
//!   says nothing about which descriptor drove the row or how it was keyed;
//! * the composed LAUNCH was traced nowhere, so "which argv shape did this CLI
//!   get" was answerable only by reading `managed_cli`;
//! * TITLE had exactly one event, emitted from one of the two chores, at the
//!   point a lookup FAILED — never at the point a row was skipped;
//! * INTEGRATION (resume, re-resume, the scheme a row is re-resolved to) had
//!   no vocabulary of its own — `restore` gives it one, and it is the moment
//!   where "this CLI comes back and the reference one does not" is settled.
//!
//! ⇒ So this module is not "more logging". It is **one grammar** whose events
//! share `slug` and `session_path`, so a single filter on `category=="cli"`
//! yields one CLI row's whole life, and two CLIs' lives can be diffed.
//!
//! ## ⛔ THE RULE THAT SHAPES EVERY EVENT HERE: A SKIP IS AN OUTCOME
//!
//! `cli/store_title_miss` was added precisely so a failed title pickup could be
//! told apart from an empty store. It was emitted only where a *lookup* failed,
//! and the chore that owned it never looked at remote rows — so an untitled
//! remote Antigravity row sat in the sidebar through a **40,000-event trace
//! window with zero `store_title_miss` events**. A probe that never fires and a
//! system with nothing to report are the same reading.
//!
//! ⇒ Every classifier here enumerates the SKIPS as first-class outcomes
//! ([`CliTitleOutcome`]), and every sweep ends in a [`CliTitleSweep::finish`]
//! that emits even when nothing happened. Silence now means the chore did not
//! run, which is a different — and findable — fault.
//!
//! ## ⛔ AND THE RULE THAT KEEPS IT AFFORDABLE
//!
//! Trace retention is a BYTE budget shared by every lane. A probe that emits
//! one span per flush once took 48.7% of all trace bytes and halved everybody
//! else's window. So:
//!
//! * per-row title events are **edge-triggered** — a row re-reporting the same
//!   outcome emits nothing (the local chore's predecessor logged 96 identical
//!   misses for one row in 91 minutes);
//! * the per-tick sweep is emitted when its counts CHANGE, or once per
//!   [`SWEEP_HEARTBEAT`] otherwise, so the plane is never silent and never
//!   chatty;
//! * birth / launch are inherently per-session-event and are not throttled.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::SessionKind;
use crate::agent_cli::{AGENT_CLIS, agent_cli_descriptor, row_icon_kind};
use crate::agent_scheme;

/// The one category every event in this module is filed under. One filter,
/// one plane: `ytrace tail --category cli`.
pub const CLI_PLANE_CATEGORY: &str = "cli";

/// How long a sweep whose counts have not changed may stay silent before it
/// reports anyway.
///
/// ⚖ The number is a trade between two failures that look identical from a
/// reader's chair: a chore that stopped running, and a chore that is running
/// with nothing to say. One minute distinguishes them at a cost of ~60 small
/// events an hour.
const SWEEP_HEARTBEAT: Duration = Duration::from_secs(60);

/// The GUI projection is larger than a title-chore sweep because it names all
/// registered CLIs, including zeroes. Twenty minutes keeps an unchanged audit
/// alive without making the observer part of the heat problem it is measuring.
const PROJECTION_SWEEP_HEARTBEAT: Duration = Duration::from_secs(20 * 60);

/// The slug an unregistered kind reports. Never a CLI name — a row whose kind
/// no descriptor serves is a finding, and giving it a plausible slug would hide
/// it in the CLI it was mistaken for.
const UNREGISTERED_SLUG: &str = "unregistered";

/// The lowercase wire slug for `kind`, from the registry.
fn slug_of(kind: SessionKind) -> &'static str {
    agent_cli_descriptor(kind)
        .map(|descriptor| descriptor.slug)
        .unwrap_or(UNREGISTERED_SLUG)
}

/// How a row's key relates to the scheme registry — the "which scheme did its
/// path take" half of a birth.
///
/// ⛔ Derived from [`agent_scheme::SESSION_PATH_SCHEMES`], never from a prefix
/// match written here: a hand-written scheme list is the recorded bug class
/// this whole registry exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliKeyScheme {
    /// The registered prefix the path starts with, e.g. `remote-agy://`.
    /// `None` ⇒ no registered scheme matched, which is itself the finding.
    pub prefix: Option<&'static str>,
    /// Whether the scheme's own declared CLI agrees with the row's kind.
    ///
    /// ⭐ THE SCHEME-TWIN DETECTOR. A row kinded Antigravity but keyed
    /// `remote-session://` (codex's historical remote scheme) is a real,
    /// filed defect — two rows for one session under two keys. `false` here
    /// names it at birth instead of leaving it to be found by its symptoms.
    /// A kind-agnostic scheme (`local://` hosts every local CLI) agrees with
    /// everything.
    pub kind_agrees: bool,
    /// `true` when the scheme names a runtime on another machine.
    pub remote: bool,
}

impl CliKeyScheme {
    /// Classify `session_path` for a row of `kind`.
    pub fn of(kind: SessionKind, session_path: &str) -> Self {
        let trimmed = session_path.trim_start();
        let scheme = agent_scheme::SESSION_PATH_SCHEMES
            .iter()
            .filter(|scheme| !scheme.prefix.is_empty())
            .filter(|scheme| trimmed.starts_with(scheme.prefix))
            // Longest prefix wins: nothing in the registry currently nests, but
            // a future `remote-cc-x://` beside `remote-cc://` must not be
            // classified as its shorter neighbour.
            .max_by_key(|scheme| scheme.prefix.len());
        match scheme {
            Some(scheme) => Self {
                prefix: Some(scheme.prefix),
                // `None` = kind-agnostic by declaration, so it cannot disagree.
                kind_agrees: scheme.kind.is_none_or(|declared| declared == kind),
                remote: scheme.locality == agent_scheme::SchemeLocality::Remote,
            },
            None => Self {
                prefix: None,
                kind_agrees: false,
                remote: false,
            },
        }
    }

    fn payload(&self) -> Value {
        json!({
            "scheme": self.prefix,
            "scheme_kind_agrees": self.kind_agrees,
            "remote": self.remote,
        })
    }
}

/// Where a live agent row's session id came from — the one bit that decides
/// whether the row and the CLI's own store can ever be keyed the same.
///
/// ⚖ [`crate::agent_cli::AgentCliDescriptor::id_assigned_at_birth`] is the
/// declaration; this is the observation of it at a particular birth, so a CLI
/// whose launch path forgot to pass the id is visible as a disagreement rather
/// than as a title that never lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliIdOrigin {
    /// yggterm minted the id and the CLI was told to use it
    /// (`claude --session-id <uuid>`). The row and the transcript match from
    /// the first byte.
    Row,
    /// The CLI mints its own and yggterm rebinds the row once it appears.
    /// Everything keyed on the row's birth id is wrong until that lands.
    Cli,
}

impl CliIdOrigin {
    /// What the registry says this CLI does.
    pub fn declared_for(kind: SessionKind) -> Self {
        match agent_cli_descriptor(kind).is_some_and(|descriptor| descriptor.id_assigned_at_birth) {
            true => Self::Row,
            false => Self::Cli,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Cli => "cli",
        }
    }
}

/// Which of yggterm's two title chores is speaking.
///
/// ⛔ Recorded on every title event because the filed defect IS the gap between
/// them: each chore skipped a class of row believing the other served it, and
/// neither said so. A reader who cannot tell the chores apart cannot see a gap
/// between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliTitleChore {
    /// Reads this machine's own CLI stores off local disk.
    Local,
    /// Reads another machine's CLI stores over ssh.
    Remote,
}

impl CliTitleChore {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local_store",
            Self::Remote => "remote_store",
        }
    }
}

/// What happened when a chore considered one row's title. **Every arm is
/// emitted**, including the skips — see the module note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliTitleOutcome {
    /// The CLI's own store answered and the row was renamed.
    PickedUp,
    /// The store was asked about this id and had nothing — and the row was
    /// wearing a detector-caught fallback, so the chore inserted the birth
    /// title instead of leaving the lie on the row. The daemon-side half of
    /// the ACT VII lesson: a detector that filters the lie is half a fix —
    /// the replacement must be INSERTED, else the stale label rides the
    /// preserve path forever (measured live 2026-09-02: opencode anchor rows
    /// answering `Remote OpenCode {shorthash}` for days because every tick
    /// recorded `no_title_in_store` and stopped there).
    InsertedBirthTitle,
    /// The store was asked about this id and had nothing. A real miss.
    NoTitleInStore,
    /// The store could not be consulted at all — ssh refused, host down,
    /// remote binary too old. ⛔ Distinct from [`Self::NoTitleInStore`]: one
    /// says the CLI has no title, the other says nobody asked.
    StoreUnreachable,
    /// ⭐ This CLI's store layout is UNMEASURED, so no lookup is attempted
    /// rather than a plausible path being guessed at
    /// (`read_live_store_title: None`). Eight of ten registered CLIs are here,
    /// and before this event that fact appeared in no trace at all.
    SkippedNoReader,
    /// The owner named this row. A refused delta never stops being a delta, so
    /// polling it forever is the recorded livelock; skipping it is correct.
    SkippedOwnerTitled,
    /// The row is idle and its title is not a placeholder — nothing to do.
    SkippedTitleSettled,
    /// ⛔⛔ **THE FILED DEFECT'S OWN EVENT.** This chore does not serve this
    /// row's key scheme. Pair it with `served_by` in the payload: `"nobody"`
    /// is the bug, and it is now a string in a trace rather than a gap between
    /// two functions that each assumed the other.
    SkippedSchemeServedElsewhere,
}

impl CliTitleOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::PickedUp => "picked_up",
            Self::InsertedBirthTitle => "inserted_birth_title",
            Self::NoTitleInStore => "no_title_in_store",
            Self::StoreUnreachable => "store_unreachable",
            Self::SkippedNoReader => "skipped_no_reader",
            Self::SkippedOwnerTitled => "skipped_owner_titled",
            Self::SkippedTitleSettled => "skipped_title_settled",
            Self::SkippedSchemeServedElsewhere => "skipped_scheme_served_elsewhere",
        }
    }

    /// Every arm, for the lock tests and for a reader building a dashboard.
    pub const ALL: &'static [Self] = &[
        Self::PickedUp,
        Self::InsertedBirthTitle,
        Self::NoTitleInStore,
        Self::StoreUnreachable,
        Self::SkippedNoReader,
        Self::SkippedOwnerTitled,
        Self::SkippedTitleSettled,
        Self::SkippedSchemeServedElsewhere,
    ];

    /// Whether this outcome means the row still has no title from its CLI.
    ///
    /// ⚠ Deliberately NOT "is this an error". `SkippedTitleSettled` and
    /// `SkippedOwnerTitled` are healthy; `SkippedNoReader` is a declared,
    /// accepted gap; only the first three are a row waiting on something.
    pub fn leaves_row_untitled(self) -> bool {
        !matches!(
            self,
            Self::PickedUp
                | Self::InsertedBirthTitle
                | Self::SkippedOwnerTitled
                | Self::SkippedTitleSettled
        )
    }
}

/// One row's title outcome for one chore tick.
fn title_payload(
    session_path: &str,
    kind: SessionKind,
    session_id: &str,
    chore: CliTitleChore,
    outcome: CliTitleOutcome,
    detail: Value,
) -> Value {
    let mut payload = json!({
        "session_path": session_path,
        "slug": slug_of(kind),
        "session_id": session_id,
        "chore": chore.label(),
        "outcome": outcome.label(),
    });
    merge(&mut payload, CliKeyScheme::of(kind, session_path).payload());
    merge(&mut payload, detail);
    payload
}

/// The tick summary one chore emits.
fn sweep_payload(
    chore: CliTitleChore,
    considered: usize,
    untitled: usize,
    counts: &BTreeMap<&'static str, usize>,
) -> Value {
    json!({
        "chore": chore.label(),
        "considered": considered,
        "left_untitled": untitled,
        "by_outcome": counts,
    })
}

/// The edge-trigger memory: the last signature emitted per (chore, row).
///
/// ⚖ A `Mutex<HashMap>` and not a lock-free structure because it is touched
/// once per row per chore tick — tens of times a minute, not thousands. Poison
/// is impossible to care about here: an emitter that loses this map re-emits,
/// which costs bytes and never lies.
fn edge_state() -> &'static Mutex<HashMap<(CliTitleChore, String), String>> {
    static STATE: OnceLock<Mutex<HashMap<(CliTitleChore, String), String>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The last sweep emitted per chore: its counts and when it went out.
fn sweep_state() -> &'static Mutex<HashMap<CliTitleChore, (BTreeMap<&'static str, usize>, Instant)>>
{
    static STATE: OnceLock<
        Mutex<HashMap<CliTitleChore, (BTreeMap<&'static str, usize>, Instant)>>,
    > = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What the GUI actually put in an agent row's title slot.
///
/// This is deliberately a classification, never the title text: the trace can
/// answer whether Codex ended at a birth placeholder or a short hash without
/// copying the user's work into the observability plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRenderedTitleQuality {
    Usable,
    Empty,
    BirthPlaceholder,
    ShortHash,
    RawPath,
    GenericPlaceholder,
    LowSignal,
}

impl CliRenderedTitleQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Empty => "empty",
            Self::BirthPlaceholder => "birth_placeholder",
            Self::ShortHash => "short_hash",
            Self::RawPath => "raw_path",
            Self::GenericPlaceholder => "generic_placeholder",
            Self::LowSignal => "low_signal",
        }
    }

    pub fn is_usable(self) -> bool {
        self == Self::Usable
    }
}

fn title_has_short_hash_shape(title: &str) -> bool {
    let words = title.split_whitespace().collect::<Vec<_>>();
    let is_short_hash = |word: &str| {
        (word.len() == 7 || word.len() == 8)
            && word.chars().all(|ch| ch.is_ascii_hexdigit())
    };
    is_short_hash(title)
        || (words.len() >= 2
            && words[0].eq_ignore_ascii_case("remote")
            && words.last().is_some_and(|word| is_short_hash(word)))
        || (words.len() >= 2
            && is_short_hash(words[0])
            && (words[1] == "-" || words[1] == "·" || words[1].starts_with('/')))
}

/// Classify a rendered row title with the same title-law recognizers that
/// decide whether it may be kept. The more specific shapes run first so a
/// sweep says *how* a row failed rather than reducing every defect to
/// `generic_placeholder`.
pub fn classify_rendered_title(title: &str) -> CliRenderedTitleQuality {
    let title = title.trim();
    if title.is_empty() {
        return CliRenderedTitleQuality::Empty;
    }
    if crate::agent_cli::is_new_row_birth_title(title) {
        return CliRenderedTitleQuality::BirthPlaceholder;
    }
    if title.starts_with('/')
        || title.contains("/home/")
        || title.to_ascii_lowercase().starts_with("c:\\")
    {
        return CliRenderedTitleQuality::RawPath;
    }
    if title_has_short_hash_shape(title) {
        return CliRenderedTitleQuality::ShortHash;
    }
    if crate::looks_like_generated_fallback_title(title) {
        return CliRenderedTitleQuality::GenericPlaceholder;
    }
    if crate::looks_like_low_signal_generated_copy(title) {
        return CliRenderedTitleQuality::LowSignal;
    }
    CliRenderedTitleQuality::Usable
}

/// The final projection facts for one agent row. These inputs come from the
/// GUI row builder, after live-title enrichment: the point whose absence from
/// the CLI plane allowed healthy title chores and a broken sidebar to coexist.
pub struct CliProjectionObservation<'a> {
    pub session_path: &'a str,
    pub kind: SessionKind,
    pub rendered_title: &'a str,
    pub icon_kind: &'a str,
    pub kind_source: &'a str,
    pub presence: &'a str,
    pub live_member: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliProjectionInspection {
    pub slug: &'static str,
    pub title_quality: CliRenderedTitleQuality,
    pub expected_icon_kind: &'static str,
    pub icon_matches_kind: bool,
}

/// Inspect one final GUI projection without emitting. App-control publishes
/// this answer on each row; the sweep below publishes the bounded trace view.
pub fn inspect_projection(
    kind: SessionKind,
    rendered_title: &str,
    icon_kind: &str,
) -> CliProjectionInspection {
    let expected_icon_kind = row_icon_kind(kind).unwrap_or(UNREGISTERED_SLUG);
    CliProjectionInspection {
        slug: slug_of(kind),
        title_quality: classify_rendered_title(rendered_title),
        expected_icon_kind,
        icon_matches_kind: icon_kind == expected_icon_kind,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
struct ProjectionSlugCounts {
    projected_rows: usize,
    unique_sessions: usize,
    usable_titles: usize,
    empty_titles: usize,
    birth_placeholders: usize,
    short_hash_titles: usize,
    raw_path_titles: usize,
    generic_placeholders: usize,
    low_signal_titles: usize,
    icon_mismatches: usize,
    authoritative_kind_rows: usize,
    inferred_kind_rows: usize,
}

fn empty_projection_counts() -> BTreeMap<&'static str, ProjectionSlugCounts> {
    AGENT_CLIS
        .iter()
        .map(|descriptor| (descriptor.slug, ProjectionSlugCounts::default()))
        .collect()
}

fn projection_sweep_payload(
    projected_rows: usize,
    rows_with_findings: usize,
    counts: &BTreeMap<&'static str, ProjectionSlugCounts>,
) -> Value {
    json!({
        "projected_rows": projected_rows,
        "rows_with_findings": rows_with_findings,
        "by_slug": counts,
    })
}

fn projection_edge_state() -> &'static Mutex<HashMap<(String, String), String>> {
    static STATE: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn projection_sweep_state() -> &'static Mutex<Option<(String, Instant)>> {
    static STATE: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Emit the GUI end of the CLI plane.
///
/// Initial healthy rows are represented only by the aggregate. A bad row emits
/// an edge immediately; when that exact row becomes healthy, the changed edge
/// emits once more so the trace records the recovery. The aggregate includes
/// every registered CLI even at zero, preventing an unintegrated CLI from
/// looking healthy merely because it never projected a row.
pub fn emit_projection_snapshot(component: &str, rows: &[CliProjectionObservation<'_>]) {
    let mut counts = empty_projection_counts();
    let mut unique_sessions = BTreeMap::<&'static str, HashSet<&str>>::new();
    let mut seen_edges = HashSet::<(String, String)>::new();
    let mut rows_with_findings = 0usize;

    for row in rows {
        let inspection = inspect_projection(row.kind, row.rendered_title, row.icon_kind);
        let slug_counts = counts.entry(inspection.slug).or_default();
        slug_counts.projected_rows += 1;
        unique_sessions
            .entry(inspection.slug)
            .or_default()
            .insert(row.session_path);
        match inspection.title_quality {
            CliRenderedTitleQuality::Usable => slug_counts.usable_titles += 1,
            CliRenderedTitleQuality::Empty => slug_counts.empty_titles += 1,
            CliRenderedTitleQuality::BirthPlaceholder => slug_counts.birth_placeholders += 1,
            CliRenderedTitleQuality::ShortHash => slug_counts.short_hash_titles += 1,
            CliRenderedTitleQuality::RawPath => slug_counts.raw_path_titles += 1,
            CliRenderedTitleQuality::GenericPlaceholder => slug_counts.generic_placeholders += 1,
            CliRenderedTitleQuality::LowSignal => slug_counts.low_signal_titles += 1,
        }
        if !inspection.icon_matches_kind {
            slug_counts.icon_mismatches += 1;
        }
        if row.kind_source == "authoritative" {
            slug_counts.authoritative_kind_rows += 1;
        } else {
            slug_counts.inferred_kind_rows += 1;
        }

        let finding = !inspection.title_quality.is_usable() || !inspection.icon_matches_kind;
        if finding {
            rows_with_findings += 1;
        }
        let key = (row.session_path.to_string(), row.presence.to_string());
        seen_edges.insert(key.clone());
        let signature = format!(
            "{}|{}|{}|{}|{}",
            inspection.title_quality.label(),
            inspection.icon_matches_kind,
            row.kind_source,
            row.live_member,
            inspection.slug,
        );
        let (changed, had_previous) = match projection_edge_state().lock() {
            Ok(mut state) => {
                let previous = state.insert(key, signature.clone());
                (previous.as_deref() != Some(signature.as_str()), previous.is_some())
            }
            Err(_) => (true, false),
        };
        if changed && (finding || had_previous) {
            crate::perf::ytrace_emit_event(
                component,
                CLI_PLANE_CATEGORY,
                "projection",
                json!({
                    "session_path": row.session_path,
                    "slug": inspection.slug,
                    "kind": format!("{:?}", row.kind),
                    "kind_source": row.kind_source,
                    "presence": row.presence,
                    "live_member": row.live_member,
                    "title_quality": inspection.title_quality.label(),
                    "icon_kind": row.icon_kind,
                    "expected_icon_kind": inspection.expected_icon_kind,
                    "icon_matches_kind": inspection.icon_matches_kind,
                    "finding": finding,
                }),
            );
        }
    }

    for (slug, sessions) in unique_sessions {
        if let Some(slug_counts) = counts.get_mut(slug) {
            slug_counts.unique_sessions = sessions.len();
        }
    }
    if let Ok(mut state) = projection_edge_state().lock() {
        state.retain(|key, _| seen_edges.contains(key));
    }

    let payload = projection_sweep_payload(rows.len(), rows_with_findings, &counts);
    let signature = serde_json::to_string(&payload).unwrap_or_default();
    let due = match projection_sweep_state().lock() {
        Ok(mut state) => {
            let now = Instant::now();
            let due = state.as_ref().is_none_or(|(previous, at)| {
                previous != &signature
                    || now.duration_since(*at) >= PROJECTION_SWEEP_HEARTBEAT
            });
            if due {
                *state = Some((signature, now));
            }
            due
        }
        Err(_) => true,
    };
    if due {
        crate::perf::ytrace_emit_event(
            component,
            CLI_PLANE_CATEGORY,
            "projection_sweep",
            payload,
        );
    }
}

/// A row was created for an agent CLI.
///
/// Answers, in one record: which descriptor resolved, how the row is keyed and
/// whether that key agrees with the kind, where the session id comes from, and
/// which machine it lives on.
///
/// ⚖ **Not a duplicate of `server/session/live_session_birth`.** That event
/// answers "a row appeared, what was active before it" — it covers plain shells
/// too and exists for a focus-steal investigation. This one answers "which CLI
/// descriptor drove this row and how is it keyed", is emitted only for agent
/// kinds, and derives every field from the registry rather than from the
/// callsite. Neither can be reconstructed from the other.
fn birth_payload(
    session_path: &str,
    kind: SessionKind,
    session_id: &str,
    machine: Option<&str>,
    cwd_present: bool,
) -> Value {
    let scheme = CliKeyScheme::of(kind, session_path);
    let mut payload = json!({
        "session_path": session_path,
        "slug": slug_of(kind),
        "kind": format!("{kind:?}"),
        "session_id": session_id,
        "id_origin": CliIdOrigin::declared_for(kind).label(),
        "machine": machine,
        "cwd_present": cwd_present,
        // A CLI with no measured store reader can never be titled by its own
        // store, whatever the chores do. Saying so at BIRTH means the reader
        // does not have to wait for a title that was never coming.
        "store_title_reader": agent_cli_descriptor(kind)
            .is_some_and(|descriptor| descriptor.read_live_store_title.is_some()),
    });
    merge(&mut payload, scheme.payload());
    payload
}

/// A row was created for an agent CLI.
pub fn emit_birth(
    component: &str,
    session_path: &str,
    kind: SessionKind,
    session_id: &str,
    machine: Option<&str>,
    cwd_present: bool,
) {
    let payload = birth_payload(session_path, kind, session_id, machine, cwd_present);
    crate::perf::ytrace_emit_event(component, CLI_PLANE_CATEGORY, "birth", payload);
}

/// What the store answered when the composer asked it to vouch for a resume
/// id — the third moment of a CLI row's life in v2 grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliResumeVouch {
    /// The CLI's own store holds the id: resume it.
    Vouched,
    /// Consulted and absent: re-birth the row rather than resume a phantom.
    /// This is the arm that prevents agy's silent empty-replacement
    /// conversation (`warning: conversation … not found` → brand-new
    /// session, exit 0).
    Absent,
    /// The store could not be read; resume anyway (the fail-open contract —
    /// re-birthing because a store could not be READ would destroy live
    /// sessions on every remote row).
    Unanswerable,
}

impl CliResumeVouch {
    pub fn label(self) -> &'static str {
        match self {
            Self::Vouched => "vouched",
            Self::Absent => "absent",
            Self::Unanswerable => "unanswerable",
        }
    }
}

/// The resume-or-rebirth decision, emitted at the ONE builder every non-CC
/// resume flows through, so a phantom resume — the silent-conversation-
/// replacement class — is a count on a probe instead of a missing transcript
/// somebody notices later. Bounded by row-open events, not by ticks.
pub fn emit_resume_decision(
    component: &str,
    kind: SessionKind,
    session_id: &str,
    vouch: CliResumeVouch,
    re_birth: bool,
) {
    if agent_cli_descriptor(kind).is_none() {
        return;
    }
    let payload = json!({
        "session_id": session_id,
        "slug": slug_of(kind),
        "kind": format!("{kind:?}"),
        "vouch": vouch.label(),
        "action": if re_birth { "rebirth" } else { "resume" },
        "id_origin": CliIdOrigin::declared_for(kind).label(),
    });
    crate::perf::ytrace_emit_event(component, CLI_PLANE_CATEGORY, "resume_decision", payload);
}

/// A persisted row was restored, and RE-KEYED on the way in.
///
/// The fourth moment of a CLI row's life, and the one that had no vocabulary at
/// all. Restore does not merely reload a row — it re-resolves it: a raw storage
/// path becomes a runtime key, a machine key is case-folded, and for a CLI that
/// mints its own session id the persisted id outranks the key it was born under
/// (while for a CLI born carrying the row's uuid the key stays authoritative,
/// because preferring a stored id there once repointed a live row at another
/// session's transcript).
///
/// ⇒ That decision is where "this CLI resumes and the reference one does not"
/// is actually settled, and it was previously visible only by diffing two
/// snapshots taken either side of a restart. `rekeyed` says whether the row
/// moved; the two schemes say what it moved BETWEEN.
///
/// ⚖ Emitted once per agent row per restart — bounded by the row count, not by
/// a tick — and not at all for a plain shell, which is not on this plane.
pub fn emit_restore(component: &str, from_path: &str, to_path: &str, kind: SessionKind) {
    if let Some(payload) = restore_payload(from_path, to_path, kind) {
        crate::perf::ytrace_emit_event(component, CLI_PLANE_CATEGORY, "restore", payload);
    }
}

/// `None` for a kind no CLI descriptor serves — a plain shell is not on this
/// plane, and giving it a row here would make the CLI counts meaningless.
fn restore_payload(from_path: &str, to_path: &str, kind: SessionKind) -> Option<Value> {
    agent_cli_descriptor(kind)?;
    let mut payload = json!({
        "session_path": to_path,
        "from_path": from_path,
        "slug": slug_of(kind),
        "kind": format!("{kind:?}"),
        "rekeyed": from_path != to_path,
        "from_scheme": CliKeyScheme::of(kind, from_path).prefix,
        "id_origin": CliIdOrigin::declared_for(kind).label(),
    });
    merge(&mut payload, CliKeyScheme::of(kind, to_path).payload());
    Some(payload)
}

/// What a composed CLI invocation looks like, without quoting the command.
///
/// ⛔ **The SHAPE, never the command string.** The composed line carries the
/// user's own cwd, their configured flags and an exported environment; putting
/// it in a trace turns a diagnostic plane into a disclosure surface. Every
/// field here is a classification, and the classification is what the
/// divergence question actually needs: *"codex got `resume <id>`, Claude Code
/// got `--resume <id>`, this one got no selector at all"*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliInvocationShape<'a> {
    /// `launch`, `resume`, or `resume_picker`.
    pub action: &'a str,
    /// The token the id rides on — `resume`, `--resume`, `--session-id`,
    /// `--conversation`. Empty when the action carries no id.
    pub selector: &'a str,
    /// Whether an id was actually passed. ⚠ `selector` non-empty and this
    /// `false` is a resume that will land in the CLI's own picker.
    pub carries_id: bool,
    /// Whether the invocation re-roots the CLI at an explicit cwd (`-C "$PWD"`
    /// for codex; Claude Code takes the process cwd).
    pub re_roots_with_cwd: bool,
    /// How many extra-arg tokens the launch carried. A count, not the tokens:
    /// a model id or a permission flag is the user's configuration.
    pub extra_arg_tokens: usize,
    /// `exec`-prefixed, i.e. this process is replaced and the row's PTY is the
    /// CLI itself.
    pub persistent: bool,
}

/// A CLI invocation was composed. One event per launch/resume, emitted at the
/// single composer so no per-CLI arm can compose without being seen.
pub fn emit_launch(component: &str, kind: SessionKind, shape: CliInvocationShape<'_>) {
    crate::perf::ytrace_emit_event(
        component,
        CLI_PLANE_CATEGORY,
        "launch",
        launch_payload(kind, shape),
    );
}

/// One bounded identity-rebind poll for a CLI whose real transcript id is
/// learned after row birth. Counts only: cwd and ids are already present on the
/// birth/projection edges, while this event's job is to explain why the join
/// did or did not move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliIdentityPollStats {
    pub target_rows: usize,
    pub machines_queried: usize,
    pub query_failures: usize,
    pub identities_seen: usize,
    pub identities_with_birth_alias: usize,
    pub exact_alias_candidates: usize,
    pub cwd_candidates: usize,
    pub rebinds: usize,
    pub newly_exhausted: usize,
}

pub fn emit_identity_poll(
    component: &str,
    kind: SessionKind,
    stats: CliIdentityPollStats,
) {
    crate::perf::ytrace_emit_event(
        component,
        CLI_PLANE_CATEGORY,
        "identity_poll",
        identity_poll_payload(kind, stats),
    );
}

fn identity_poll_payload(kind: SessionKind, stats: CliIdentityPollStats) -> Value {
    json!({
        "slug": slug_of(kind),
        "kind": format!("{kind:?}"),
        "target_rows": stats.target_rows,
        "machines_queried": stats.machines_queried,
        "query_failures": stats.query_failures,
        "identities_seen": stats.identities_seen,
        "identities_with_birth_alias": stats.identities_with_birth_alias,
        "exact_alias_candidates": stats.exact_alias_candidates,
        "cwd_candidates": stats.cwd_candidates,
        "rebinds": stats.rebinds,
        "newly_exhausted": stats.newly_exhausted,
    })
}

/// One self-minting row's identity-bind outcome, emitted on VERDICT CHANGE
/// only. This is the probe point for the whole join plane: which arm bound
/// the row (owner-reported birth alias vs the Codex cwd compatibility guess),
/// which id won, and — the refusal classes behind the 2026-09-04
/// all-codex-rows-one-session collapse — why a row that needed an identity
/// got none. `session_path` names the row; ids ride as ids, never titles.
pub struct CliAgentIdentityDecision<'a> {
    pub slug: &'a str,
    pub session_path: &'a str,
    /// `bound` | `satisfied` | `no_candidate` | `ambiguous_cwd` |
    /// `exhausted` | `unchanged`
    pub verdict: &'static str,
    /// `birth_alias` | `birth_key_alias` | `none`
    pub arm: &'static str,
    pub chosen_id: Option<&'a str>,
    pub cwd_candidates: usize,
}

pub fn emit_agent_identity_decision(component: &str, decision: CliAgentIdentityDecision<'_>) {
    crate::perf::ytrace_emit_event(
        component,
        CLI_PLANE_CATEGORY,
        "agent_identity_decision",
        json!({
            "slug": decision.slug,
            "session_path": decision.session_path,
            "verdict": decision.verdict,
            "arm": decision.arm,
            "chosen_id": decision.chosen_id,
            "cwd_candidates": decision.cwd_candidates,
        }),
    );
}

/// A live self-minting row whose real transcript id could NOT be measured
/// this tick, with the named reason. The stamp starvation behind the
/// 2026-09-04 collapse lived here as a bare `continue`: a row whose terminal
/// pid the daemon could not see was never stamped and never counted, so the
/// precise owner-alias join starved and the cwd guess cross-wired every
/// same-directory row onto one transcript. Edge-triggered per
/// (session_path, reason).
pub fn emit_agent_identity_probe_unresolved(component: &str, session_path: &str, reason: &str) {
    crate::perf::ytrace_emit_event(
        component,
        CLI_PLANE_CATEGORY,
        "agent_identity_probe_unresolved",
        json!({ "session_path": session_path, "reason": reason }),
    );
}

/// A stamp REFUSED because another live row already carries the same
/// transcript id — one transcript, one row. Edge-triggered per
/// (row, id, holder); the pid-recycling flip-flop of 2026-09-04 wore this
/// exact shape before the refusal existed.
pub fn emit_agent_identity_stamp_duplicate(
    component: &str,
    session_path: &str,
    codex_session_id: &str,
    holder: &str,
) {
    crate::perf::ytrace_emit_event(
        component,
        CLI_PLANE_CATEGORY,
        "agent_identity_stamp_duplicate",
        json!({
            "session_path": session_path,
            "codex_session_id": codex_session_id,
            "holder": holder,
        }),
    );
}

/// One mirror tick's identity decision, for a CLI whose TUI renders sessions
/// the row was not born with.
///
/// Emitted on INTERESTING ticks only (a spawn, a retire, a focus, or a bound
/// identity that disagrees with what the service is viewing) — never on the
/// 5 s tick, which would cost ~17k events a day to say "still in sync". The
/// `diverged` outcome is the event this probe exists for: bound ≠ viewing and
/// no rebind happened, with the anchor, the candidate count and both ids on
/// the event instead of in a debugger. `anchor` is `None` when no row
/// qualified; `viewing`/`bound` are `None` when the service was quiet or the
/// anchor carries no id yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliMirrorTickDecision<'a> {
    pub anchor: Option<&'a str>,
    pub candidates: usize,
    pub viewing: Option<&'a str>,
    pub bound: Option<&'a str>,
    /// `in_sync` | `diverged` | `rebound` | `rebind_failed` | `no_anchor` |
    /// `anchor_not_live` | `no_viewing`.
    pub decision: &'static str,
    pub active_tabs: usize,
}

pub fn emit_mirror_tick(component: &str, kind: SessionKind, tick: CliMirrorTickDecision<'_>) {
    if agent_cli_descriptor(kind).is_none() {
        return;
    }
    let payload = mirror_tick_payload(kind, tick);
    crate::perf::ytrace_emit_event(component, CLI_PLANE_CATEGORY, "mirror_tick", payload);
}

fn mirror_tick_payload(kind: SessionKind, tick: CliMirrorTickDecision<'_>) -> Value {
    json!({
        "slug": slug_of(kind),
        "kind": format!("{kind:?}"),
        "anchor": tick.anchor,
        "candidates": tick.candidates,
        "viewing": tick.viewing,
        "bound": tick.bound,
        "decision": tick.decision,
        "active_tabs": tick.active_tabs,
    })
}

/// Why a composed launch/resume does NOT say what the descriptor declares.
///
/// Emitted ONLY on degrade — the faithful path is already covered by the
/// `cli/launch` shape event, and a second event per launch would double that
/// stream's bytes to restate agreement. Each reason names the rail that
/// refused: the ses_ guard (a phantom id the service would reject), the
/// store-vouch absent arm (an id the store never held), or the service-vouched
/// override (the live focus stream outranks a lagging store index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliLaunchContractBreach {
    SesGuardDegrade,
    StoreAbsentRebirth,
    ServiceVouchedResume,
}

impl CliLaunchContractBreach {
    pub fn label(self) -> &'static str {
        match self {
            Self::SesGuardDegrade => "ses_guard_degrade",
            Self::StoreAbsentRebirth => "store_absent_rebirth",
            Self::ServiceVouchedResume => "service_vouched_resume",
        }
    }
}

pub fn emit_launch_contract(
    component: &str,
    kind: SessionKind,
    declared_selector: &str,
    shape: CliInvocationShape<'_>,
    breach: CliLaunchContractBreach,
) {
    if agent_cli_descriptor(kind).is_none() {
        return;
    }
    let mut payload = launch_payload(kind, shape);
    merge(
        &mut payload,
        json!({
            "declared_selector": declared_selector,
            "breach": breach.label(),
        }),
    );
    crate::perf::ytrace_emit_event(component, CLI_PLANE_CATEGORY, "launch_contract", payload);
}

/// One row's working-verdict transition, as the daemon's snapshot chore saw
/// it — the attributable form of the blinking dot.
///
/// `screen_signal` = the CLI's own in-flight chrome is on the live screen;
/// `recency_signal` = the PTY went active inside the recent window. A dot
/// blinking on recency alone reads differently from one the CLI's footer
/// drives, and until now the two were one bit. Edge-triggered by the caller:
/// this fn emits unconditionally, and the chore that owns the last-state set
/// decides when a transition happened.
pub fn emit_working_edge(
    component: &str,
    kind: SessionKind,
    session_path: &str,
    working: bool,
    screen_signal: bool,
    recency_signal: bool,
) {
    if agent_cli_descriptor(kind).is_none() {
        return;
    }
    let payload = json!({
        "session_path": session_path,
        "slug": slug_of(kind),
        "kind": format!("{kind:?}"),
        "edge": if working { "working" } else { "idle" },
        "screen_signal": screen_signal,
        "recency_signal": recency_signal,
    });
    crate::perf::ytrace_emit_event(component, CLI_PLANE_CATEGORY, "working_edge", payload);
}

fn launch_payload(kind: SessionKind, shape: CliInvocationShape<'_>) -> Value {
    json!({
            "slug": slug_of(kind),
            "kind": format!("{kind:?}"),
            "action": shape.action,
            "selector": shape.selector,
            "carries_id": shape.carries_id,
            "re_roots_with_cwd": shape.re_roots_with_cwd,
            "extra_arg_tokens": shape.extra_arg_tokens,
            "persistent": shape.persistent,
            "id_origin": CliIdOrigin::declared_for(kind).label(),
    })
}

/// One chore tick's pass over the live rows: it records an outcome per row and
/// **always reports at the end**.
///
/// ⛔ Built as an accumulator rather than a pair of free functions so the sweep
/// cannot be forgotten. The failure this closes is not hypothetical — it is the
/// filed one: a chore that skips a class of row and emits nothing is
/// indistinguishable from a chore with nothing to do.
pub struct CliTitleSweep {
    component: &'static str,
    chore: CliTitleChore,
    counts: BTreeMap<&'static str, usize>,
    /// Rows this tick considered, so [`Self::finish`] can drop edge-trigger
    /// memory for rows that no longer exist (an unbounded map keyed on session
    /// paths would grow for the life of the daemon).
    seen: HashSet<String>,
    /// Rows left with no title from their CLI, for the sweep's own summary.
    untitled: usize,
}

impl CliTitleSweep {
    pub fn new(component: &'static str, chore: CliTitleChore) -> Self {
        Self {
            component,
            chore,
            counts: BTreeMap::new(),
            seen: HashSet::new(),
            untitled: 0,
        }
    }

    /// Record one row's outcome, emitting `cli/title` only on an EDGE.
    ///
    /// `signature` distinguishes two occurrences of the same outcome that a
    /// reader must see separately — the new title for a pickup, the machine for
    /// an unreachable store. `None` ⇒ the outcome alone is the signature.
    pub fn record(
        &mut self,
        session_path: &str,
        kind: SessionKind,
        session_id: &str,
        outcome: CliTitleOutcome,
        signature: Option<&str>,
        detail: Value,
    ) {
        *self.counts.entry(outcome.label()).or_insert(0) += 1;
        self.seen.insert(session_path.to_string());
        if outcome.leaves_row_untitled() {
            self.untitled += 1;
        }

        let stamp = match signature {
            Some(signature) => format!("{}|{signature}", outcome.label()),
            None => outcome.label().to_string(),
        };
        let changed = match edge_state().lock() {
            Ok(mut state) => {
                let key = (self.chore, session_path.to_string());
                state.insert(key, stamp.clone()).as_deref() != Some(stamp.as_str())
            }
            // A poisoned map means re-emit: bytes, never a lie.
            Err(_) => true,
        };
        if !changed {
            return;
        }

        let payload = title_payload(
            session_path,
            kind,
            session_id,
            self.chore,
            outcome,
            detail,
        );
        crate::perf::ytrace_emit_event(self.component, CLI_PLANE_CATEGORY, "title", payload);
    }

    /// Emit the tick's summary and prune edge memory for departed rows.
    ///
    /// Emits when the counts differ from the last sweep, or once per
    /// [`SWEEP_HEARTBEAT`] otherwise — so an idle plane is quiet but never
    /// silent.
    pub fn finish(self) {
        if let Ok(mut state) = edge_state().lock() {
            state.retain(|(chore, path), _| *chore != self.chore || self.seen.contains(path));
        }

        let considered: usize = self.counts.values().sum();
        let due = match sweep_state().lock() {
            Ok(mut state) => {
                let now = Instant::now();
                let due = match state.get(&self.chore) {
                    Some((last_counts, last_at)) => {
                        *last_counts != self.counts
                            || now.duration_since(*last_at) >= SWEEP_HEARTBEAT
                    }
                    None => true,
                };
                if due {
                    state.insert(self.chore, (self.counts.clone(), now));
                }
                due
            }
            Err(_) => true,
        };
        if !due {
            return;
        }
        crate::perf::ytrace_emit_event(
            self.component,
            CLI_PLANE_CATEGORY,
            "title_sweep",
            sweep_payload(self.chore, considered, self.untitled, &self.counts),
        );
    }
}

/// Fold `extra`'s keys into `base`. Both are objects by construction here.
fn merge(base: &mut Value, extra: Value) {
    let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) else {
        return;
    };
    for (key, value) in extra {
        base.insert(key.clone(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_row_is_kind_agnostic_and_therefore_always_agrees() {
        let scheme = CliKeyScheme::of(SessionKind::Antigravity, "local://a-b-c");
        assert_eq!(scheme.prefix, Some("local://"));
        assert!(scheme.kind_agrees);
        assert!(!scheme.remote);
    }

    #[test]
    fn a_remote_row_reports_its_own_clis_scheme() {
        let scheme = CliKeyScheme::of(SessionKind::ClaudeCode, "remote-cc://machine-a/a-b-c");
        assert_eq!(scheme.prefix, Some("remote-cc://"));
        assert!(scheme.kind_agrees);
        assert!(scheme.remote);
    }

    /// ⭐ The scheme-twin detector: an Antigravity row wearing codex's
    /// historical remote scheme is the filed "two rows, one session" defect,
    /// and it must be visible at birth rather than by its symptoms.
    #[test]
    fn a_row_keyed_under_another_clis_scheme_is_reported_as_disagreeing() {
        let scheme = CliKeyScheme::of(SessionKind::Antigravity, "remote-session://machine-a/a-b-c");
        assert_eq!(scheme.prefix, Some("remote-session://"));
        assert!(!scheme.kind_agrees, "codex's scheme must not accept an agy row");
    }

    #[test]
    fn an_unregistered_scheme_is_reported_rather_than_guessed_local() {
        let scheme = CliKeyScheme::of(SessionKind::Codex, "not-a-scheme://machine-a/a-b-c");
        assert_eq!(scheme.prefix, None);
        assert!(!scheme.kind_agrees);
    }

    /// The user's visible failure: the title chore can report a healthy tick
    /// while the final Codex row is still wearing its birth name. The GUI
    /// projection plane must call that state out explicitly.
    #[test]
    fn a_machine_named_codex_birth_title_is_a_projection_finding() {
        assert_eq!(
            classify_rendered_title("New devhost Codex"),
            CliRenderedTitleQuality::BirthPlaceholder
        );
        let inspection = inspect_projection(SessionKind::Codex, "New devhost Codex", "session");
        assert!(!inspection.title_quality.is_usable());
        assert!(inspection.icon_matches_kind);
    }

    #[test]
    fn projection_quality_names_short_hashes_and_raw_paths_separately() {
        assert_eq!(
            classify_rendered_title("a8f6dbd1"),
            CliRenderedTitleQuality::ShortHash
        );
        assert_eq!(
            classify_rendered_title("/home/user/projects/example"),
            CliRenderedTitleQuality::RawPath
        );
        assert_eq!(
            classify_rendered_title("Investigate terminal heat"),
            CliRenderedTitleQuality::Usable
        );
    }

    /// Codex's historical `session` wire icon is intentional. The detector
    /// compares against registry policy, not slug equality, while still
    /// catching a row that actually projects the terminal icon.
    #[test]
    fn projection_icon_agreement_uses_the_registrys_legacy_mapping() {
        assert!(
            inspect_projection(SessionKind::Codex, "Investigate terminal heat", "session")
                .icon_matches_kind
        );
        assert!(
            !inspect_projection(SessionKind::Codex, "Investigate terminal heat", "terminal")
                .icon_matches_kind
        );
    }

    #[test]
    fn projection_sweeps_name_every_registered_cli_even_at_zero() {
        let counts = empty_projection_counts();
        assert_eq!(counts.len(), AGENT_CLIS.len());
        for descriptor in AGENT_CLIS {
            assert!(
                counts.contains_key(descriptor.slug),
                "{} disappeared from the projection plane",
                descriptor.slug
            );
        }
    }

    #[test]
    fn identity_poll_distinguishes_alias_failure_from_cwd_failure() {
        let payload = identity_poll_payload(
            SessionKind::Codex,
            CliIdentityPollStats {
                target_rows: 2,
                machines_queried: 1,
                query_failures: 0,
                identities_seen: 3,
                identities_with_birth_alias: 0,
                exact_alias_candidates: 0,
                cwd_candidates: 0,
                rebinds: 0,
                newly_exhausted: 2,
            },
        );
        assert_eq!(payload["slug"], serde_json::json!("codex"));
        assert_eq!(payload["identities_seen"], serde_json::json!(3));
        assert_eq!(payload["exact_alias_candidates"], serde_json::json!(0));
        assert_eq!(payload["cwd_candidates"], serde_json::json!(0));
        assert_eq!(payload["newly_exhausted"], serde_json::json!(2));
    }

    #[test]
    fn a_diverged_mirror_tick_names_anchor_viewing_bound_and_verdict() {
        let payload = mirror_tick_payload(
            SessionKind::OpenCode,
            CliMirrorTickDecision {
                anchor: Some("opencode-runtime://d4090efe-4e12-42d9-938d-66f61801d2e7"),
                candidates: 5,
                viewing: Some("ses_f9cdde2f5ffep2W0tBiWE7qb3a"),
                bound: Some("d4090efe-4e12-42d9-938d-66f61801d2e7"),
                decision: "diverged",
                active_tabs: 2,
            },
        );
        assert_eq!(payload["slug"], serde_json::json!("opencode"));
        assert_eq!(payload["decision"], serde_json::json!("diverged"));
        assert_eq!(payload["candidates"], serde_json::json!(5));
        assert_eq!(
            payload["viewing"],
            serde_json::json!("ses_f9cdde2f5ffep2W0tBiWE7qb3a")
        );
        assert_eq!(
            payload["bound"],
            serde_json::json!("d4090efe-4e12-42d9-938d-66f61801d2e7")
        );
    }

    #[test]
    fn a_quiet_mirror_tick_is_representable_without_ids() {
        let payload = mirror_tick_payload(
            SessionKind::OpenCode,
            CliMirrorTickDecision {
                anchor: None,
                candidates: 0,
                viewing: None,
                bound: None,
                decision: "no_anchor",
                active_tabs: 0,
            },
        );
        assert_eq!(payload["decision"], serde_json::json!("no_anchor"));
        assert!(payload["viewing"].is_null());
    }

    #[test]
    fn a_contract_breach_names_the_declared_selector_and_the_refusing_rail() {
        let payload = {
            let mut base = launch_payload(
                SessionKind::OpenCode,
                CliInvocationShape {
                    action: "launch",
                    selector: "",
                    carries_id: false,
                    re_roots_with_cwd: false,
                    extra_arg_tokens: 0,
                    persistent: true,
                },
            );
            merge(
                &mut base,
                serde_json::json!({
                    "declared_selector": "--session",
                    "breach": CliLaunchContractBreach::SesGuardDegrade.label(),
                }),
            );
            base
        };
        assert_eq!(payload["action"], serde_json::json!("launch"));
        assert_eq!(payload["declared_selector"], serde_json::json!("--session"));
        assert_eq!(payload["breach"], serde_json::json!("ses_guard_degrade"));
    }

    /// The registry decides `id_origin`, so a CLI added tomorrow reports the
    /// truth without touching this module.
    #[test]
    fn the_id_origin_comes_from_the_registry() {
        assert_eq!(
            CliIdOrigin::declared_for(SessionKind::ClaudeCode),
            CliIdOrigin::Row,
            "Claude Code is launched with --session-id"
        );
        assert_eq!(
            CliIdOrigin::declared_for(SessionKind::Codex),
            CliIdOrigin::Cli,
            "codex mints its own id and the row is rebound"
        );
    }

    #[test]
    fn every_outcome_has_a_distinct_label() {
        let labels: HashSet<&str> = CliTitleOutcome::ALL
            .iter()
            .map(|outcome| outcome.label())
            .collect();
        assert_eq!(labels.len(), CliTitleOutcome::ALL.len());
    }

    /// ⛔ The classification a dashboard depends on: a healthy skip must not be
    /// counted as a row waiting on something, or the "left_untitled" gauge is
    /// noise and gets switched off.
    #[test]
    fn only_the_outcomes_that_leave_a_row_waiting_are_counted_as_untitled() {
        assert!(!CliTitleOutcome::PickedUp.leaves_row_untitled());
        assert!(!CliTitleOutcome::SkippedOwnerTitled.leaves_row_untitled());
        assert!(!CliTitleOutcome::SkippedTitleSettled.leaves_row_untitled());
        assert!(CliTitleOutcome::NoTitleInStore.leaves_row_untitled());
        assert!(CliTitleOutcome::StoreUnreachable.leaves_row_untitled());
        assert!(CliTitleOutcome::SkippedNoReader.leaves_row_untitled());
        assert!(CliTitleOutcome::SkippedSchemeServedElsewhere.leaves_row_untitled());
    }

    #[test]
    fn merging_a_detail_object_keeps_both_sides() {
        let mut base = json!({"a": 1});
        merge(&mut base, json!({"b": 2}));
        assert_eq!(base, json!({"a": 1, "b": 2}));
    }

    /// A plain shell is not a CLI row, and counting it as one would make every
    /// per-CLI figure on this plane a different number than it claims to be.
    #[test]
    fn a_restore_that_is_not_an_agent_row_is_not_on_this_plane() {
        assert!(restore_payload("local://a", "local://a", SessionKind::Shell).is_none());
        assert!(restore_payload("local://a", "local://a", SessionKind::ClaudeCode).is_some());
    }

    /// ⭐ The re-key is the whole point of the event: a row that came back
    /// under a different key took a different path from the reference CLI's,
    /// and the two schemes say what it moved between.
    #[test]
    fn a_restore_reports_the_scheme_a_row_moved_between() {
        let kept = restore_payload("local://abc", "local://abc", SessionKind::ClaudeCode)
            .expect("an agent row");
        assert_eq!(kept["rekeyed"], serde_json::json!(false));

        let moved = restore_payload(
            "local://abc",
            "cc-runtime://abc",
            SessionKind::ClaudeCode,
        )
        .expect("an agent row");
        assert_eq!(moved["rekeyed"], serde_json::json!(true));
        assert_eq!(moved["from_scheme"], serde_json::json!("local://"));
        assert_eq!(moved["scheme"], serde_json::json!("cc-runtime://"));
    }

    /// The bytes a written trace line costs beyond its payload — `ts_ms`, `pid`,
    /// `component`, `category`, `name` and the JSON punctuation around them.
    /// Measured off a live trace file, where a 195-byte line carried an 81-byte
    /// payload.
    const LINE_ENVELOPE_BYTES: usize = 114;

    /// ⛔⛔ STATE THE PROBE'S SHARE OF THE PLANE, AND KEEP STATING IT.
    ///
    /// Trace retention is a BYTE budget shared by every lane, so a probe's cost
    /// is taken out of everyone else's window. One lane's span-per-flush turned
    /// out to be 48.7% of all trace bytes and halved every other lane's
    /// retention — measured only after it had shipped.
    ///
    /// ⇒ The share is asserted here rather than written in a commit message,
    /// because a number in a commit message describes the day it was written
    /// and this one has to stay true. The live plane it is measured against
    /// carried **86.9 KiB/min over a 99.9-minute, 40,248-event window**.
    ///
    /// The steady state is what matters: birth and launch are per-session
    /// events (a person opening a row), while the two title chores tick
    /// continuously forever. With every row's outcome unchanged — the normal
    /// case, since a title is picked up once and then stays — an edge-triggered
    /// chore emits nothing per row, and the whole recurring cost of this module
    /// is one heartbeat sweep per title chore plus the slower final-projection
    /// heartbeat.
    #[test]
    fn the_cli_plane_states_its_share_of_the_trace_byte_budget() {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        // A deliberately unkind sweep: every outcome present at once, which is
        // the widest `by_outcome` map the payload can carry.
        for outcome in CliTitleOutcome::ALL {
            counts.insert(outcome.label(), 7);
        }
        let sweep = sweep_payload(CliTitleChore::Remote, 49, 21, &counts);
        let sweep_bytes = serde_json::to_string(&sweep).expect("payload serialises").len()
            + LINE_ENVELOPE_BYTES;

        let title_sweeps_per_hour = 2 * (3600 / SWEEP_HEARTBEAT.as_secs() as usize);
        let mut projection_counts = empty_projection_counts();
        for counts in projection_counts.values_mut() {
            *counts = ProjectionSlugCounts {
                projected_rows: 7,
                unique_sessions: 5,
                usable_titles: 2,
                empty_titles: 1,
                birth_placeholders: 1,
                short_hash_titles: 1,
                raw_path_titles: 1,
                generic_placeholders: 1,
                low_signal_titles: 0,
                icon_mismatches: 1,
                authoritative_kind_rows: 4,
                inferred_kind_rows: 3,
            };
        }
        let projection_sweep = projection_sweep_payload(70, 50, &projection_counts);
        let projection_sweep_bytes = serde_json::to_string(&projection_sweep)
            .expect("projection payload serialises")
            .len()
            + LINE_ENVELOPE_BYTES;
        let projection_sweeps_per_hour =
            3600 / PROJECTION_SWEEP_HEARTBEAT.as_secs() as usize;
        let steady_bytes_per_hour = title_sweeps_per_hour * sweep_bytes
            + projection_sweeps_per_hour * projection_sweep_bytes;

        // The measured plane, over the window this was calibrated against.
        const PLANE_KIB_PER_MIN: usize = 87;
        let plane_bytes_per_hour = PLANE_KIB_PER_MIN * 1024 * 60;
        let share_per_mille = 1000 * steady_bytes_per_hour / plane_bytes_per_hour;

        assert!(
            share_per_mille <= 10,
            "the CLI plane's steady-state cost is {steady_bytes_per_hour} bytes/hour, \
             {}.{}% of a {PLANE_KIB_PER_MIN} KiB/min plane — state the new share and \
             justify it before raising this ceiling",
            share_per_mille / 10,
            share_per_mille % 10
        );

        // The per-event half, bounded so a field added to birth or launch is a
        // decision rather than a drift.
        let birth = birth_payload(
            "remote-example://build-box/6f1c0d84-2a7b-4e59-9c30-8d51b2a4e7f6",
            SessionKind::ClaudeCode,
            "6f1c0d84-2a7b-4e59-9c30-8d51b2a4e7f6",
            Some("build-box"),
            true,
        );
        let launch = launch_payload(
            SessionKind::ClaudeCode,
            CliInvocationShape {
                action: "resume",
                selector: "--resume",
                carries_id: true,
                re_roots_with_cwd: false,
                extra_arg_tokens: 2,
                persistent: true,
            },
        );
        let restore = restore_payload(
            "local://6f1c0d84-2a7b-4e59-9c30-8d51b2a4e7f6",
            "cc-runtime://6f1c0d84-2a7b-4e59-9c30-8d51b2a4e7f6",
            SessionKind::ClaudeCode,
        )
        .expect("an agent row is on this plane");
        let identity_poll = identity_poll_payload(
            SessionKind::Codex,
            CliIdentityPollStats {
                target_rows: 7,
                machines_queried: 2,
                query_failures: 1,
                identities_seen: 9,
                identities_with_birth_alias: 5,
                exact_alias_candidates: 4,
                cwd_candidates: 3,
                rebinds: 4,
                newly_exhausted: 1,
            },
        );
        // Issue 31 probes ride the same per-event budget: a diverged tick
        // carries two full-length ids, so it is measured at full length.
        let mirror_tick = mirror_tick_payload(
            SessionKind::OpenCode,
            CliMirrorTickDecision {
                anchor: Some("opencode-runtime://d4090efe-4e12-42d9-938d-66f61801d2e7"),
                candidates: 5,
                viewing: Some("ses_f9cdde2f5ffep2W0tBiWE7qb3a"),
                bound: Some("d4090efe-4e12-42d9-938d-66f61801d2e7"),
                decision: "diverged",
                active_tabs: 2,
            },
        );
        for (name, payload) in [
            ("birth", &birth),
            ("launch", &launch),
            ("restore", &restore),
            ("identity_poll", &identity_poll),
            ("mirror_tick", &mirror_tick),
        ] {
            let bytes =
                serde_json::to_string(payload).expect("payload serialises").len() + LINE_ENVELOPE_BYTES;
            assert!(
                bytes <= 600,
                "the {name} event costs {bytes} bytes; it fires once per session event, \
                 so it is affordable, but a field was added without anyone saying so"
            );
        }
    }
}
