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
//!   no vocabulary of its own.
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
use crate::agent_cli::agent_cli_descriptor;
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
            Self::PickedUp | Self::SkippedOwnerTitled | Self::SkippedTitleSettled
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
    /// is one heartbeat sweep per chore.
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

        let sweeps_per_hour = 2 * (3600 / SWEEP_HEARTBEAT.as_secs() as usize);
        let steady_bytes_per_hour = sweeps_per_hour * sweep_bytes;

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
        for (name, payload) in [("birth", &birth), ("launch", &launch)] {
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
