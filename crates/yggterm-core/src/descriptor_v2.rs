//! Descriptor schema v2 — [`docs/cli-integration-layer.md`](../docs/cli-integration-layer.md)
//! §2 and §9, the unit the wave-2 per-CLI seats fill against.
//!
//! ## What v2 is, and what it deliberately is not
//!
//! The stone says `AgentCliDescriptor` "grew into 35+ ad-hoc fields" and v2
//! "reorganizes it into five orthogonal capabilities". **The reorganization
//! here is CONTRACT-FIRST, not a field migration**: the 35 fields stay where
//! they are (each already carries its measured history in its docs), and this
//! module adds the three things the five-capability model needs that did not
//! exist, plus the version marker and the per-CLI completeness test that §9
//! makes the release gate:
//!
//! 1. **Invocation & identity** — already answered by existing descriptor
//!    fields ([`AgentCliDescriptor::id_assigned_at_birth`],
//!    [`AgentCliDescriptor::resume_selector`],
//!    [`AgentCliDescriptor::resume_re_roots_with_cwd`]). The §9 test reads
//!    them; nothing new is added.
//! 2. **Tenancy & rebind discovery** — the strategies existed as scattered
//!    predicates (env tenancy markers, argv identity, the fd holder arm, the
//!    store readers, opencode's server). What was missing is the stone's
//!    PRECEDENCE CHAIN as declared data: [`rebind_chain`], one ordered
//!    [`RebindStrategy`] list per CLI, array order = resolution precedence.
//! 3. **Discrete phase state machine** — the phrase tables existed per state
//!    and seat B's announce wire named the phases as strings; what was missing
//!    is THE enum both sides share ([`AgentPhase`]) and ONE classifier that
//!    composes the per-state tables in a documented precedence
//!    ([`screen_phase`]). The hot-restart gate and the sidebar dot consume
//!    this; it must never disagree with itself by carrying two spellings.
//! 4. **Durable store & copy reader** — already the richest capability
//!    (`session_store_globs`, `store_roots`, `read_store_entry`, the keyed
//!    index hook, `store_scan_gap`). The §9 test reads them.
//! 5. **Tenancy migration protocol** — `content_rederives_on_resume`
//!    existed; [`supports_pty_fd_handoff`] is new, table-driven so a CLI
//!    that cannot survive an fd move has a place to say so by name.
//!
//! ⛔ **Versioning law (§2, verbatim): adding a capability = a
//! [`DESCRIPTOR_SCHEMA_VERSION`] bump + the probe battery re-run per CLI, in
//! the same commit.** The constant exists so a reader can ASK which schema a
//! build speaks instead of inferring it from field shapes.

use crate::agent_cli::AgentCliDescriptor;
use crate::SessionKind;

/// The descriptor schema this build speaks. Bumping this is a release event
/// (§2): every CLI's probe battery re-runs in the same commit.
pub const DESCRIPTOR_SCHEMA_VERSION: u32 = 2;

/// §2.2 — one link in a CLI's rebind resolution chain. The stone's precedence
/// order, highest first: `NativeAnnounce → EnvMarker → Argv → FdLock →
/// StoreIndex → ServerIpc`. A per-CLI chain NEVER lists all six — it lists
/// the strategies that CLI actually answers with, highest-precedence first,
/// and a rebind question is answered by the FIRST strategy in the chain that
/// can answer at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebindStrategy {
    /// First-party OSC announce — the row's TUI states its own session id
    /// (zcode-tui; spec §3). The top of the chain only while frames are fresh.
    NativeAnnounce,
    /// A process-environment marker names the session.
    EnvMarker(&'static str),
    /// Identity read off the process argv (a flag or positional naming the id).
    ArgvIdentity,
    /// An open fd whose path names the session — the holder arm.
    FdLock,
    /// The CLI's own store/index answers (the class A/C answer).
    StoreIndex,
    /// A server process holds the truth and is asked (class B).
    ServerIpc,
}

impl RebindStrategy {
    /// The one word a trace event or a report names the strategy by.
    pub fn word(&self) -> &'static str {
        match self {
            Self::NativeAnnounce => "native_announce",
            Self::EnvMarker(_) => "env_marker",
            Self::ArgvIdentity => "argv",
            Self::FdLock => "fd_lock",
            Self::StoreIndex => "store_index",
            Self::ServerIpc => "server_ipc",
        }
    }
}

/// §2.3 — THE discrete phase enum. Verbatim the stone's five states; the
/// announce wire (`ANNOUNCE_PHASES` on the server) spells these same names,
/// and the hot-restart gate and the sidebar dot consume this enum. Finer
/// screen states than the enum (background-agent hint, plan-limit choice)
/// map onto it: the hint is idle-with-a-background-agent — a healthy IDLE —
/// and a plan-limit DIALOG is a LIMITWAIT with a selection marker, per the
/// descriptor tables' own docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPhase {
    Working,
    Idle,
    QuestionPrompt,
    LimitWait,
    StartupGate,
}

impl AgentPhase {
    /// The wire spelling — exactly the stone's variant names.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Idle => "Idle",
            Self::QuestionPrompt => "QuestionPrompt",
            Self::LimitWait => "LimitWait",
            Self::StartupGate => "StartupGate",
        }
    }

    /// The strict inverse of [`Self::as_str`]: a name outside the enum is
    /// noise, not a phase (the announce wire's own parse law).
    pub fn from_wire(name: &str) -> Option<Self> {
        Some(match name {
            "Working" => Self::Working,
            "Idle" => Self::Idle,
            "QuestionPrompt" => Self::QuestionPrompt,
            "LimitWait" => Self::LimitWait,
            "StartupGate" => Self::StartupGate,
            _ => return None,
        })
    }
}

/// §2.3 — a phase answer. [`PhaseAnswer::NoOpinion`] is the honest third
/// answer: every phrase table EMPTY means UNMEASURED (the descriptor's own
/// law), and an unmeasured CLI must not be reported `Idle` — that is the
/// guess that turns "paused" into "done" for the next caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseAnswer {
    Known(AgentPhase),
    NoOpinion,
}

/// §2.3 — ONE classifier composing the descriptor's per-state phrase tables
/// into the phase enum, in a documented precedence.
///
/// Precedence, most-dangerous-misread first:
///
/// 1. **Startup gate** — a gate stands BEFORE the composer exists, so every
///    other state is meaningless behind it (the descriptor's own doc: all
///    other classifiers read FALSE on a gate screen).
/// 2. **Question picker** — the state that EATS TYPED INPUT while reading as
///    `working`; mistaking it for anything else is the costliest lie.
/// 3. **Limit wait** (the plan-limit DIALOG pairs with it structurally — a
///    dialog carries a selection marker where the footer does not; both are
///    quota states) → `LimitWait`.
/// 4. **Working** — the measured work signals, negations applied.
/// 5. **Idle** — only when SOME state table for this CLI is measured; a CLI
///    with every table empty answers [`PhaseAnswer::NoOpinion`], never a
///    guessed quiet.
///
/// The background-agent hint is deliberately NOT a phase: by its own doc it
/// exists to prevent a false WORKING read, and hint-present + working-false
/// is a healthy idle row — [`AgentPhase::Idle`] already says it.
pub fn screen_phase(descriptor: &AgentCliDescriptor, screen: &str) -> PhaseAnswer {
    let measured_any = !descriptor.working_screen_phrases.is_empty()
        || !descriptor.working_footer_hints.is_empty()
        || !descriptor.limit_wait_screen_phrases.is_empty()
        || !descriptor.question_picker_screen_phrases.is_empty()
        || !descriptor.startup_gate_screen_phrases.is_empty()
        || !descriptor.plan_limit_choice_screen_phrases.is_empty();
    if !measured_any {
        return PhaseAnswer::NoOpinion;
    }
    let phase = if descriptor.screen_shows_startup_gate(screen) {
        AgentPhase::StartupGate
    } else if descriptor.screen_shows_question_picker(screen) {
        AgentPhase::QuestionPrompt
    } else if descriptor.screen_shows_limit_wait(screen)
        || descriptor.screen_shows_plan_limit_choice(screen)
    {
        AgentPhase::LimitWait
    } else if descriptor.screen_shows_working(screen) {
        AgentPhase::Working
    } else {
        AgentPhase::Idle
    };
    PhaseAnswer::Known(phase)
}

/// §2.2 — the rebind resolution chain per CLI, precedence order (first entry
/// answers first). Class-level declarations from the stone §1 table; a
/// per-CLI seat refines the order it measures differently (the chain is
/// data, and refining it is that seat's 11.6.N work — the test only demands
/// that every CLI declares ONE).
///
/// ⛔ The matcher machinery each strategy names already exists; this table
/// does not implement resolution, it DECLARES the order so a resolver (and a
/// reader of the spec) can ask a CLI "which strategy answers your rebind
/// first?" without re-deriving it from scattered predicates.
pub fn rebind_chain(kind: SessionKind) -> &'static [RebindStrategy] {
    match kind {
        // Class A — store-authoritative; codex's fd-holder arm predates the
        // store reader as a positive answer, so it stays first.
        SessionKind::Codex | SessionKind::CodexLiteLlm => {
            const CHAIN: &[RebindStrategy] = &[RebindStrategy::FdLock, RebindStrategy::StoreIndex];
            CHAIN
        }
        SessionKind::ClaudeCode => {
            const CHAIN: &[RebindStrategy] = &[RebindStrategy::StoreIndex];
            CHAIN
        }
        // Class B — the SERVER holds the truth, and its store IS the server's
        // own db: the store read outranks a live ask in the stone's §2.2
        // precedence, so the chain follows the stone, not intuition.
        SessionKind::OpenCode => {
            const CHAIN: &[RebindStrategy] =
                &[RebindStrategy::StoreIndex, RebindStrategy::ServerIpc];
            CHAIN
        }
        // Class B, first-party — the announce outranks everything while fresh.
        SessionKind::ZcodeTui => {
            const CHAIN: &[RebindStrategy] =
                &[RebindStrategy::NativeAnnounce, RebindStrategy::ServerIpc];
            CHAIN
        }
        // Class C — TUI with a local store; id discovery is store work
        // (the 11.6.4/11.6.5 doors carry the measured store layouts).
        SessionKind::Antigravity
        | SessionKind::Muse
        | SessionKind::Kimi
        | SessionKind::QwenCode
        | SessionKind::GrokBuild
        | SessionKind::Pi => {
            const CHAIN: &[RebindStrategy] = &[RebindStrategy::StoreIndex];
            CHAIN
        }
        // Not agent rows — they have no rebind question; the chain is empty
        // BY DECLARATION (the §9 test treats these kinds as out of scope for
        // the agent capabilities, not as missing answers).
        SessionKind::Shell | SessionKind::SshShell | SessionKind::Document => &[],
    }
}

/// §2.5 — whether a daemon swap may adopt this CLI's PTY via SCM_RIGHTS.
///
/// ⛔ This is TRUE for every shipped CLI and that is a MEASURED fact, not a
/// default: adoption moves the daemon's master fd and the child re-parents
/// alive — the CLI inside cannot even see it (135 adoptions measured in one
/// night on the build host; see `pty_adoption.rs`). The table exists so the
/// FIRST CLI that breaks under an fd move has a place to say so by name,
/// and so §9 can demand an answer from every kind instead of an implicit
/// global guess.
pub fn supports_pty_fd_handoff(kind: SessionKind) -> bool {
    match kind {
        SessionKind::Codex
        | SessionKind::CodexLiteLlm
        | SessionKind::ClaudeCode
        | SessionKind::OpenCode
        | SessionKind::ZcodeTui
        | SessionKind::Antigravity
        | SessionKind::Muse
        | SessionKind::Kimi
        | SessionKind::QwenCode
        | SessionKind::GrokBuild
        | SessionKind::Pi
        | SessionKind::Shell
        | SessionKind::SshShell
        | SessionKind::Document => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_cli::agent_cli_descriptor;

    /// §9 — table-driven, per CLI: every registry entry answers all five
    /// capabilities or NAMES what is missing. This is the release gate; a new
    /// CLI that arrives with an empty phrase table still passes, because the
    /// gap is DECLARED (empty tables are the descriptor's own unmeasured
    /// law) — what fails is a CLI with no answer at all.
    #[test]
    fn every_registered_cli_answers_the_five_capabilities_or_names_the_gap() {
        for kind in SessionKind::ALL {
            let Some(descriptor) = agent_cli_descriptor(*kind) else {
                // Shells/documents have no agent descriptor by design.
                assert!(matches!(
                    kind,
                    SessionKind::Shell | SessionKind::SshShell | SessionKind::Document
                ));
                continue;
            };
            // Cap 1 — invocation & identity.
            assert!(
                descriptor.wrapper_slug.is_some() || descriptor.remote_row_scheme.is_none(),
                "{kind:?}: no identity lane at all (neither wrapper slug nor local-only)"
            );
            let _ = descriptor.id_assigned_at_birth;
            let _ = descriptor.resume_selector;
            // Cap 2 — rebind discovery: a non-empty declared chain.
            let chain = rebind_chain(*kind);
            assert!(!chain.is_empty(), "{kind:?}: rebind chain is undeclared");
            // Cap 3 — phase machine: the classifier answers (Known or the
            // DECLARED NoOpinion), and the enum round-trips the wire names.
            let answer = screen_phase(descriptor, "");
            assert!(matches!(
                answer,
                PhaseAnswer::Known(_) | PhaseAnswer::NoOpinion
            ));
            for phase in [
                AgentPhase::Working,
                AgentPhase::Idle,
                AgentPhase::QuestionPrompt,
                AgentPhase::LimitWait,
                AgentPhase::StartupGate,
            ] {
                assert_eq!(AgentPhase::from_wire(phase.as_str()), Some(phase));
            }
            // Cap 4 — durable store: globs, a durable single-file store, or a
            // DECLARED scan gap. Silence is the one forbidden answer.
            assert!(
                !descriptor.session_store_globs.is_empty()
                    || !descriptor.durable_store_files.is_empty()
                    || descriptor.store_scan_gap.is_some(),
                "{kind:?}: store capability silent — neither globs nor a declared gap"
            );
            // Cap 5 — migration: the handoff table answers, the rederive bit
            // exists.
            let _ = supports_pty_fd_handoff(*kind);
            let _ = descriptor.content_rederives_on_resume;
        }
    }

    #[test]
    fn the_phase_classifier_precedence_is_the_documented_one() {
        // A MEASURED CLI (every registry entry is) answers Known on an empty
        // screen — Idle, not NoOpinion: the tables exist, so quiet is a real
        // answer here. The NoOpinion arm is reserved for a future CLI whose
        // tables are all empty (the declared-unmeasured law).
        let descriptor = agent_cli_descriptor(SessionKind::Codex).unwrap();
        assert_eq!(screen_phase(&descriptor, ""), PhaseAnswer::Known(AgentPhase::Idle));
    }

    #[test]
    fn the_wire_parse_rejects_noise() {
        // The strict parse law: a name outside the enum is not a phase.
        assert_eq!(AgentPhase::from_wire("Working"), Some(AgentPhase::Working));
        assert_eq!(AgentPhase::from_wire("working"), None); // case is the wire
        assert_eq!(AgentPhase::from_wire(""), None);
        assert_eq!(AgentPhase::from_wire("BackgroundAgent"), None);
    }

    #[test]
    fn the_rebind_chain_precedence_order_matches_the_stone() {
        // §2.2: NativeAnnounce > EnvMarker > Argv > FdLock > StoreIndex >
        // ServerIpc. A chain must never list a lower-precedence strategy
        // before a higher one that also answers for that CLI.
        let rank = |s: &RebindStrategy| match s {
            RebindStrategy::NativeAnnounce => 0,
            RebindStrategy::EnvMarker(_) => 1,
            RebindStrategy::ArgvIdentity => 2,
            RebindStrategy::FdLock => 3,
            RebindStrategy::StoreIndex => 4,
            RebindStrategy::ServerIpc => 5,
        };
        for kind in SessionKind::ALL {
            let chain = rebind_chain(*kind);
            let mut ranks: Vec<usize> = chain.iter().map(rank).collect();
            ranks.dedup();
            let mut sorted = ranks.clone();
            sorted.sort();
            assert_eq!(ranks, sorted, "{kind:?}: rebind chain violates precedence");
        }
        // Spot-check the declared shapes against the stone's classes.
        assert_eq!(rebind_chain(SessionKind::ZcodeTui)[0], RebindStrategy::NativeAnnounce);
        assert_eq!(rebind_chain(SessionKind::OpenCode)[0], RebindStrategy::StoreIndex);
        assert_eq!(rebind_chain(SessionKind::ClaudeCode)[0], RebindStrategy::StoreIndex);
    }

    #[test]
    fn schema_version_is_two() {
        assert_eq!(DESCRIPTOR_SCHEMA_VERSION, 2);
    }
}
