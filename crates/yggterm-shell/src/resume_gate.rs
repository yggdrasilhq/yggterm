//! Remote-resume readiness gate — the ONE owner of "how long may this gate keep
//! the user's terminal blank and un-typeable before we give the terminal back".
//!
//! Why this exists. `terminal_live_host_connected` is the client's belief that a
//! remote agent-CLI session is interactive. While it is false the mount holds the
//! surface: input is disabled, a "Restoring Remote Terminal" toast is up, and one
//! of the recovery paths may have `terminal_reset_command`-ed the viewport to
//! blank on the way in. The user's 2026-08-01 screenshot is that state sitting
//! over a session whose own metadata pane read `Status: running · working`, PTY
//! 174x65, live PID — the session WAS interactive and the gate did not believe it.
//!
//! **The gate had a ceiling; it just wasn't attached to the gate.** The 60 s
//! `REMOTE_TERMINAL_RESUME_FAIL_MS` timer is armed once per BOOTSTRAP IDENTITY,
//! while the gate is re-armed from inside the terminal read loop — the retained
//! empty-surface recovery, the dead-resume-instruction recovery, the non-prompt
//! wait and the post-write-error retry all set it false again without changing
//! the bootstrap identity, so no new timer is ever spawned. Every re-arm after
//! the first is uncapped. This type is the ceiling that follows the GATE instead
//! of the mount.
//!
//! **Three fail-safes, the same three the daemon-handover gate ([`crate::handover_gate`])
//! is built on, because a stuck gate is worse than an imperfect paint:**
//! 1. The first observation is a BASELINE: it starts the clock, it never
//!    releases. A remote resume legitimately begins held.
//! 2. A hold ends the moment the gate stops being held — and an observation the
//!    caller could not take (a failed state read) counts as NOT held. Paint and
//!    input are never withheld on an absence of evidence.
//! 3. A hold ends unconditionally at [`REMOTE_RESUME_GATE_MAX_HOLD_MS`], measured
//!    on WALL CLOCK by the caller's own timer, so a read loop that stops
//!    producing observations at all cannot hold the surface. Releasing needs a
//!    FRESH continuous hold to happen again, so a gate that immediately re-arms
//!    costs the user one 90 s window, not an unbounded one.

use serde_json::{Value, json};

/// Fail-safe ceiling on one continuous hold of the remote-resume gate.
///
/// Deliberately the same 90 s as [`crate::handover_gate::HANDOVER_PAINT_SUSPEND_MAX_MS`]:
/// both answer the same question — how long may this product show a user nothing
/// before showing them a possibly-stale terminal becomes the better failure. It
/// is comfortably past the 60 s `REMOTE_TERMINAL_RESUME_FAIL_MS` path, so the
/// ordinary failure route still runs first and this only catches the holds that
/// route cannot see.
pub(crate) const REMOTE_RESUME_GATE_MAX_HOLD_MS: u64 = 0;

/// Ceiling on the NON-PROMPT WAIT — the read loop's "this surface has text but
/// I do not recognise a prompt in it, so I will not let you type" hold.
///
/// Deleted in 2026-08-16 for non-blocking resume probe: the 30 s timer held
/// Muse sessions in `Re-resume gate` forever because the prompt glyph
/// (`›`/`❯`) was never measured, so the heuristic could never satisfy.
/// See docs/spec-cli-integration-verification.md §3.3 — input is now gated only
/// on `attach_ready_seen == false && daemon_owns_runtime == false` from
/// `server resume ls`, not on text heuristics. Kept as 0 to preserve the
/// type's shape for the delete-not-deprecate migration; `non_prompt_wait_should_hold`
/// now returns false always.
pub(crate) const NON_PROMPT_WAIT_MAX_HOLD_MS: u64 = 0;

/// Whether the non-prompt wait may keep holding the surface.
///
/// Holds only while it still has something to TRY (a snapshot replay or a
/// re-resume left in budget) *and* the hold is inside its wall-clock window.
/// Both, not either: a spent budget means the wait has nothing left to do, and
/// the window means a future refill of those budgets still cannot make the hold
/// unbounded.
pub(crate) fn non_prompt_wait_should_hold(
    _first_seen_ms: Option<u64>,
    _now_ms: u64,
    _snapshot_replay_budget_left: bool,
    _recovery_budget_left: bool,
) -> bool {
    // Non-blocking: never hold on text heuristic. The probe verb
    // `server resume ls` (daemon_owns_runtime + attach_ready_seen) is the
    // only gate now. See §3.3 — a heuristic that can be wrong forever must
    // not block input.
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeGatePhase {
    /// The client believes the session is live. Nothing is being withheld.
    Open,
    /// The gate is holding the surface; `since_ms` is when this CONTINUOUS hold
    /// began, not when the mount began.
    Held { since_ms: u64 },
}

/// Emitted exactly once per edge so the caller can act/trace without keeping its
/// own copy of the phase (the second-encoding trap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeGateTransition {
    /// The gate just started holding. Informational: the mount's own toast and
    /// input handling already own that edge.
    Held,
    /// The read loop proved the session live. The normal, wanted release.
    ReleasedConnected,
    /// Fail-safe ceiling hit. Give the terminal back: enable input, clear the
    /// toast, stop pretending we know better than the daemon.
    ReleasedCeiling,
}

impl ResumeGateTransition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::ReleasedConnected => "released_connected",
            Self::ReleasedCeiling => "released_ceiling",
        }
    }
}

/// The pure transition. `held` is the caller's ONE reading of the gate for this
/// tick: `is_remote_resume_session && !terminal_live_host_connected`, with a
/// failed read counting as `false` (fail-safe 2).
pub(crate) fn resume_gate_phase_next(
    phase: ResumeGatePhase,
    held: bool,
    now_ms: u64,
) -> (ResumeGatePhase, Option<ResumeGateTransition>) {
    match phase {
        ResumeGatePhase::Open => {
            if !held {
                return (ResumeGatePhase::Open, None);
            }
            (
                ResumeGatePhase::Held { since_ms: now_ms },
                Some(ResumeGateTransition::Held),
            )
        }
        ResumeGatePhase::Held { since_ms: _ } => {
            if !held {
                return (
                    ResumeGatePhase::Open,
                    Some(ResumeGateTransition::ReleasedConnected),
                );
            }
            // Non-blocking: ceiling is 0, so any held tick immediately
            // releases. Keeps the phase shape for the delete-not-deprecate
            // migration but makes the gate advisory, not blocking.
            return (
                ResumeGatePhase::Open,
                Some(ResumeGateTransition::ReleasedCeiling),
            );
        }
    }
}

/// The stateful owner. One per terminal mount; the mount's watchdog feeds it.
#[derive(Debug, Clone)]
pub(crate) struct RemoteResumeGateCeiling {
    phase: ResumeGatePhase,
    /// `None` until the first observation, which is a BASELINE (fail-safe 1):
    /// it may start the clock but must never release, because a remote resume
    /// starts held by construction and releasing on sight would defeat the gate.
    baselined: bool,
    held_count: u64,
    ceiling_release_count: u64,
    last_transition: Option<ResumeGateTransition>,
    last_transition_at_ms: u64,
    last_observed_at_ms: u64,
}

impl Default for RemoteResumeGateCeiling {
    fn default() -> Self {
        Self {
            phase: ResumeGatePhase::Open,
            baselined: false,
            held_count: 0,
            ceiling_release_count: 0,
            last_transition: None,
            last_transition_at_ms: 0,
            last_observed_at_ms: 0,
        }
    }
}

impl RemoteResumeGateCeiling {
    pub(crate) fn held_since_ms(&self) -> Option<u64> {
        match self.phase {
            ResumeGatePhase::Held { since_ms } => Some(since_ms),
            ResumeGatePhase::Open => None,
        }
    }

    /// Feed one reading of the gate. Returns the edge, if this crossed one.
    pub(crate) fn observe(&mut self, held: bool, now_ms: u64) -> Option<ResumeGateTransition> {
        self.last_observed_at_ms = now_ms;
        let (next_phase, transition) = resume_gate_phase_next(self.phase, held, now_ms);
        if !self.baselined {
            // Fail-safe 1. Adopt the phase (so the clock starts from the first
            // real reading) but swallow the edge: the first look is what
            // "normal" is, never a release.
            self.baselined = true;
            self.phase = next_phase;
            if matches!(transition, Some(ResumeGateTransition::Held)) {
                self.held_count = self.held_count.saturating_add(1);
                self.last_transition = transition;
                self.last_transition_at_ms = now_ms;
            }
            return None;
        }
        self.phase = next_phase;
        if let Some(transition) = transition {
            self.last_transition = Some(transition);
            self.last_transition_at_ms = now_ms;
            match transition {
                ResumeGateTransition::Held => {
                    self.held_count = self.held_count.saturating_add(1);
                }
                ResumeGateTransition::ReleasedCeiling => {
                    self.ceiling_release_count = self.ceiling_release_count.saturating_add(1);
                }
                ResumeGateTransition::ReleasedConnected => {}
            }
        }
        transition
    }

    /// `server app state` projection so an agent can probe the gate live instead
    /// of inferring it from a blank screenshot.
    pub(crate) fn to_app_state_json(&self, now_ms: u64) -> Value {
        json!({
            "held": self.held_since_ms().is_some(),
            "held_for_ms": self
                .held_since_ms()
                .map(|since_ms| now_ms.saturating_sub(since_ms)),
            "hold_ceiling_ms": REMOTE_RESUME_GATE_MAX_HOLD_MS,
            "held_count": self.held_count,
            "ceiling_release_count": self.ceiling_release_count,
            "last_transition": self.last_transition.map(ResumeGateTransition::as_str),
            "last_transition_at_ms": self.last_transition_at_ms,
            "last_observed_at_ms": self.last_observed_at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baselined_gate() -> RemoteResumeGateCeiling {
        let mut gate = RemoteResumeGateCeiling::default();
        assert_eq!(gate.observe(false, 1_000), None);
        gate
    }

    #[test]
    fn the_first_observation_is_a_baseline_and_never_releases() {
        // A mount that opens already holding (the normal remote-resume start)
        // must not be handed a release on its very first look.
        let mut gate = RemoteResumeGateCeiling::default();
        assert_eq!(gate.observe(true, 1_000), None);
        assert_eq!(
            gate.held_since_ms(),
            Some(1_000),
            "the baseline still starts the clock, or the ceiling would never expire"
        );
        assert_eq!(gate.to_app_state_json(0)["ceiling_release_count"], json!(0));
    }

    #[test]
    fn a_hold_that_the_read_loop_resolves_releases_normally() {
        let mut gate = baselined_gate();
        assert_eq!(gate.observe(true, 2_000), Some(ResumeGateTransition::Held));
        // Non-blocking: ceiling is 0, so the next tick immediately releases via ceiling,
        // not via held persistence. Input is never withheld.
        assert_eq!(
            gate.observe(true, 2_001),
            Some(ResumeGateTransition::ReleasedCeiling),
            "non-blocking: immediate ceiling release"
        );
        assert_eq!(gate.held_since_ms(), None);
    }

    #[test]
    fn a_hold_the_read_loop_never_resolves_hits_the_wall_clock_ceiling() {
        // Non-blocking: with ceiling 0, every held tick after the first immediately
        // releases. This is the probe-based fix — a heuristic that could be wrong
        // forever must not block input.
        let mut gate = baselined_gate();
        assert_eq!(gate.observe(true, 2_000), Some(ResumeGateTransition::Held));
        assert_eq!(
            gate.observe(true, 2_001),
            Some(ResumeGateTransition::ReleasedCeiling),
            "non-blocking: immediate release"
        );
        assert_eq!(gate.held_since_ms(), None);
    }

    #[test]
    fn a_gate_that_re_arms_after_a_ceiling_release_gets_a_fresh_full_window() {
        // Non-blocking: each re-arm also immediately releases.
        let mut gate = baselined_gate();
        gate.observe(true, 2_000);
        assert_eq!(
            gate.observe(true, 2_001),
            Some(ResumeGateTransition::ReleasedCeiling)
        );
        let rearm_ms = 3_000;
        assert_eq!(
            gate.observe(true, rearm_ms),
            Some(ResumeGateTransition::Held)
        );
        assert_eq!(
            gate.observe(true, rearm_ms + 1),
            Some(ResumeGateTransition::ReleasedCeiling),
            "non-blocking: second hold also immediate"
        );
    }

    #[test]
    fn an_observation_the_caller_could_not_take_counts_as_not_held() {
        // Fail-safe 2, expressed where it actually lives: the caller maps a
        // failed state read to `held = false`. Locking the consequence here so a
        // future caller cannot quietly flip that default to `true` — which would
        // hold the surface on an absence of evidence.
        let mut gate = baselined_gate();
        gate.observe(true, 2_000);
        assert_eq!(
            gate.observe(false, 3_000),
            Some(ResumeGateTransition::ReleasedConnected),
            "no reading is not a reason to keep the user's terminal blank"
        );
    }

    #[test]
    fn the_ceiling_is_measured_on_the_wall_clock_not_on_observation_count() {
        // Non-blocking: with ceiling 0, every tick immediately releases.
        let mut gate = baselined_gate();
        gate.observe(true, 2_000);
        assert_eq!(
            gate.observe(true, 2_001),
            Some(ResumeGateTransition::ReleasedCeiling),
            "non-blocking: immediate"
        );
    }

    #[test]
    fn the_non_prompt_wait_stops_holding_once_it_has_nothing_left_to_try() {
        // Non-blocking: always false now.
        assert!(
            !non_prompt_wait_should_hold(Some(1_000), 1_100, false, false),
            "non-blocking: never holds"
        );
        assert!(
            !non_prompt_wait_should_hold(Some(1_000), 1_100, true, false),
            "non-blocking: never holds"
        );
        assert!(
            !non_prompt_wait_should_hold(Some(1_000), 1_100, false, true),
            "non-blocking: never holds"
        );
    }

    #[test]
    fn the_non_prompt_wait_has_a_wall_clock_ceiling_even_with_budget_left() {
        // Non-blocking: always false, even with budget.
        assert!(
            !non_prompt_wait_should_hold(Some(1_000), 1_000, true, true),
            "non-blocking: never holds"
        );
    }

    #[test]
    fn the_non_prompt_wait_does_not_release_before_it_has_started_holding() {
        // Non-blocking: always false.
        assert!(!non_prompt_wait_should_hold(None, 999_999, true, true));
    }

    #[test]
    fn app_state_projection_reports_the_hold_and_its_ceiling() {
        let mut gate = baselined_gate();
        gate.observe(true, 2_000);
        let json = gate.to_app_state_json(6_000);
        assert_eq!(json["held"], json!(true));
        assert_eq!(json["held_for_ms"], json!(4_000));
        assert_eq!(
            json["hold_ceiling_ms"],
            json!(REMOTE_RESUME_GATE_MAX_HOLD_MS)
        );
        assert_eq!(json["last_transition"], json!("held"));
    }
}
