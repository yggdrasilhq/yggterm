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
pub(crate) const REMOTE_RESUME_GATE_MAX_HOLD_MS: u64 = 90_000;

/// Ceiling on the NON-PROMPT WAIT — the read loop's "this surface has text but
/// I do not recognise a prompt in it, so I will not let you type" hold.
///
/// That gate is the worst-shaped one on this path because it is SELF-FEEDING:
/// its own arm condition includes `poisoned_by_retry`
/// (`terminal_attach_in_flight || resume_notification_visible`) and entering it
/// sets both. Its disarm is a TEXT HEURISTIC — the surface must start matching
/// `terminal_surface_has_prompt_ready_text` — which a streaming agent frame, or
/// any CLI whose prompt we do not model, may never satisfy. Once its two
/// recovery budgets were spent it simply re-toasted every 120 ms forever.
///
/// A recognition heuristic is allowed to be wrong. It is not allowed to be
/// wrong FOREVER while the user looks at a terminal they cannot type into.
pub(crate) const NON_PROMPT_WAIT_MAX_HOLD_MS: u64 = 30_000;

/// Whether the non-prompt wait may keep holding the surface.
///
/// Holds only while it still has something to TRY (a snapshot replay or a
/// re-resume left in budget) *and* the hold is inside its wall-clock window.
/// Both, not either: a spent budget means the wait has nothing left to do, and
/// the window means a future refill of those budgets still cannot make the hold
/// unbounded.
pub(crate) fn non_prompt_wait_should_hold(
    first_seen_ms: Option<u64>,
    now_ms: u64,
    snapshot_replay_budget_left: bool,
    recovery_budget_left: bool,
) -> bool {
    if !snapshot_replay_budget_left && !recovery_budget_left {
        return false;
    }
    match first_seen_ms {
        // No observed start means the caller has not seen the condition
        // persist; it is not holding anything yet.
        None => true,
        Some(first_seen_ms) => now_ms.saturating_sub(first_seen_ms) < NON_PROMPT_WAIT_MAX_HOLD_MS,
    }
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
        ResumeGatePhase::Held { since_ms } => {
            if !held {
                return (
                    ResumeGatePhase::Open,
                    Some(ResumeGateTransition::ReleasedConnected),
                );
            }
            if now_ms.saturating_sub(since_ms) >= REMOTE_RESUME_GATE_MAX_HOLD_MS {
                // Release, and re-open the phase rather than latching: the read
                // loop may legitimately re-arm, and the next hold gets its own
                // full window. A latch here would mean "one release per mount,
                // then uncapped forever", which is the bug wearing a hat.
                return (
                    ResumeGatePhase::Open,
                    Some(ResumeGateTransition::ReleasedCeiling),
                );
            }
            (ResumeGatePhase::Held { since_ms }, None)
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
        assert_eq!(
            gate.observe(true, 20_000),
            None,
            "still held, no repeat edge"
        );
        assert_eq!(
            gate.observe(false, 30_000),
            Some(ResumeGateTransition::ReleasedConnected)
        );
        assert_eq!(gate.held_since_ms(), None);
        assert_eq!(
            gate.to_app_state_json(0)["ceiling_release_count"],
            0,
            "a normal release must not be counted as a ceiling release"
        );
    }

    #[test]
    fn a_hold_the_read_loop_never_resolves_hits_the_wall_clock_ceiling() {
        // THE USER'S BUG: the read loop stops producing evidence (or produces
        // evidence that never satisfies the prompt heuristic) and the surface is
        // held blank and un-typeable forever.
        let mut gate = baselined_gate();
        assert_eq!(gate.observe(true, 2_000), Some(ResumeGateTransition::Held));
        assert_eq!(
            gate.observe(true, 2_000 + REMOTE_RESUME_GATE_MAX_HOLD_MS - 1),
            None,
            "one millisecond short of the ceiling must still hold"
        );
        assert_eq!(
            gate.observe(true, 2_000 + REMOTE_RESUME_GATE_MAX_HOLD_MS),
            Some(ResumeGateTransition::ReleasedCeiling)
        );
        assert_eq!(gate.held_since_ms(), None);
        assert_eq!(gate.to_app_state_json(0)["ceiling_release_count"], 1);
    }

    #[test]
    fn a_gate_that_re_arms_after_a_ceiling_release_gets_a_fresh_full_window() {
        // The read loop's recovery paths re-arm the gate on their own. That must
        // cost the user ONE more bounded window, never an unbounded one, and it
        // must not be released instantly either (that would defeat the gate for
        // every genuinely-slow resume that follows).
        let mut gate = baselined_gate();
        gate.observe(true, 2_000);
        assert_eq!(
            gate.observe(true, 2_000 + REMOTE_RESUME_GATE_MAX_HOLD_MS),
            Some(ResumeGateTransition::ReleasedCeiling)
        );
        let rearm_ms = 2_000 + REMOTE_RESUME_GATE_MAX_HOLD_MS + 500;
        assert_eq!(
            gate.observe(true, rearm_ms),
            Some(ResumeGateTransition::Held)
        );
        assert_eq!(
            gate.observe(true, rearm_ms + REMOTE_RESUME_GATE_MAX_HOLD_MS - 1),
            None,
            "the second hold must get its own full window, not the remainder of the first"
        );
        assert_eq!(
            gate.observe(true, rearm_ms + REMOTE_RESUME_GATE_MAX_HOLD_MS),
            Some(ResumeGateTransition::ReleasedCeiling)
        );
        assert_eq!(gate.to_app_state_json(0)["ceiling_release_count"], 2);
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
        // A read loop that goes silent still gets ticked by the watchdog; the
        // release must depend on elapsed time, not on how many samples arrived.
        let mut gate = baselined_gate();
        gate.observe(true, 2_000);
        for tick in 1..=40u64 {
            assert_eq!(
                gate.observe(true, 2_000 + tick * 100),
                None,
                "sample {tick} released early on sample count rather than the clock"
            );
        }
        assert_eq!(
            gate.observe(true, 2_000 + REMOTE_RESUME_GATE_MAX_HOLD_MS),
            Some(ResumeGateTransition::ReleasedCeiling)
        );
    }

    #[test]
    fn the_non_prompt_wait_stops_holding_once_it_has_nothing_left_to_try() {
        // The reported shape: both recovery budgets spent, the heuristic still
        // unsatisfied, and the old code looped at 120 ms re-raising "Restoring
        // Remote Terminal" with input disabled, forever.
        assert!(
            !non_prompt_wait_should_hold(Some(1_000), 1_100, false, false),
            "a wait with no remaining recovery must release, not re-toast"
        );
        assert!(non_prompt_wait_should_hold(Some(1_000), 1_100, true, false));
        assert!(non_prompt_wait_should_hold(Some(1_000), 1_100, false, true));
    }

    #[test]
    fn the_non_prompt_wait_has_a_wall_clock_ceiling_even_with_budget_left() {
        // Budgets are attempt counts, not time. If a refill ever appears (or an
        // attempt is not consumed on some path), the hold must still end.
        assert!(non_prompt_wait_should_hold(
            Some(1_000),
            1_000 + NON_PROMPT_WAIT_MAX_HOLD_MS - 1,
            true,
            true
        ));
        assert!(
            !non_prompt_wait_should_hold(
                Some(1_000),
                1_000 + NON_PROMPT_WAIT_MAX_HOLD_MS,
                true,
                true
            ),
            "full budget is not a licence to hold the terminal indefinitely"
        );
    }

    #[test]
    fn the_non_prompt_wait_does_not_release_before_it_has_started_holding() {
        // No first-seen observation = the settle window has not even begun; the
        // ceiling must not fire pre-emptively and defeat the gate's purpose.
        assert!(non_prompt_wait_should_hold(None, 999_999, true, true));
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
