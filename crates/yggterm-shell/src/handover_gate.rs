//! Daemon-handover paint gate — the ONE client-side owner of "a daemon
//! handover is in progress for THIS client, so stop painting the terminal".
//!
//! Why this exists (user-settled call #7, `docs/pending-bugs.md`): *"On a
//! daemon version change the GUI host burns. Spawn a notification (daemon is
//! changing, please wait), stop drawing the terminal for the duration, and
//! entertain the user. The render cost during handover is the thing being
//! avoided, so the fix is to stop painting, not to paint a spinner harder."*
//!
//! **Source of truth.** The predicate is derived from the DAEMON'S OWN report —
//! `ServerRuntimeStatus::preserved_terminal_owner_keys`, the keys this daemon
//! knows about but does NOT yet hold, i.e. the ones a predecessor still owns
//! ([`crate::hot_update_policy::runtime_status_handoff_active`] is the single
//! owner of that reading, shared with the daemon-update panel). It is never
//! inferred from a version mismatch, a cursor rewind, or any other client-side
//! tell: those are symptoms of a handover, not the daemon saying one is
//! happening.
//!
//! **Scoped to what this client paints.** A handover only concerns us while one
//! of the sessions THIS client has mounted is among the keys awaiting adoption.
//! Another agent's session migrating on a lingering older daemon is normal
//! (⚖ the constitution: version-coexisting daemons) and must not veil the
//! user's viewport.
//!
//! **Three fail-safes, because a stuck veil is worse than the burn:**
//! 1. The FIRST observation is a baseline and never suspends. A GUI that starts
//!    while an older daemon happens to hold a preserved session must not open
//!    behind a veil. (The shell never manufactures an observation out of a
//!    failed status fetch, so that baseline is always a real daemon reading.)
//! 2. A suspension ends the moment the daemon's report stops justifying it —
//!    the successor adopted our sessions, or the report no longer mentions a
//!    handoff at all. Paint is never held back on an absence of evidence.
//! 3. A suspension ends unconditionally at [`HANDOVER_PAINT_SUSPEND_MAX_MS`],
//!    measured on WALL CLOCK by [`HandoverPaintGate::tick`] so a daemon that
//!    dies mid-handover — and therefore stops producing observations at all —
//!    cannot hold the veil. The handover that hit the ceiling is LATCHED so it
//!    cannot re-arm; only a genuinely new fingerprint arms the gate again.

use serde_json::{Value, json};

/// Fail-safe ceiling on a single suspension. The handover's re-resume storm is
/// seconds, not minutes; anything past this is a signal we lost, and painting a
/// possibly-stale terminal beats holding a veil over a working session.
/// (Task brief: bounded, 60–120 s.)
pub(crate) const HANDOVER_PAINT_SUSPEND_MAX_MS: u64 = 90_000;

/// What the client observed about the daemon's handover state on one poll.
/// Built by [`handover_observation_from_parts`] so the two facts always come
/// from the same status read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct HandoverObservation {
    /// The daemon's own report: it is serving keys a predecessor still owns.
    pub handoff_in_flight: bool,
    /// At least one runtime key THIS client has mounted is among them.
    pub client_sessions_awaiting_adoption: bool,
    /// Identity of this handover: the daemon serving us plus the client-relevant
    /// slice of the preserved set. Changes when a NEW handover step touches one
    /// of our sessions; stable while the same one is in flight, so a resolved
    /// handover cannot re-arm itself.
    pub fingerprint: String,
}

impl HandoverObservation {
    /// The whole decision in one place: a handover the daemon reports, that
    /// involves a session this client is painting.
    pub(crate) fn wants_suspend(&self) -> bool {
        self.handoff_in_flight && self.client_sessions_awaiting_adoption
    }
}

/// Build the observation from the two things it is made of. Pure so the wiring
/// above it (which status, which mounted keys) is the only thing left to get
/// wrong.
///
/// `preserved_keys` is the daemon's `preserved_terminal_owner_keys`;
/// `client_runtime_keys` is every runtime key this client currently has a
/// mounted terminal host for.
pub(crate) fn handover_observation_from_parts(
    handoff_in_flight: bool,
    daemon_identity: &str,
    preserved_keys: &[String],
    client_runtime_keys: &[String],
) -> HandoverObservation {
    let mut awaiting = preserved_keys
        .iter()
        .filter(|key| client_runtime_keys.iter().any(|mounted| mounted == *key))
        .cloned()
        .collect::<Vec<_>>();
    awaiting.sort();
    awaiting.dedup();
    HandoverObservation {
        handoff_in_flight,
        client_sessions_awaiting_adoption: !awaiting.is_empty(),
        fingerprint: format!("{daemon_identity}|{}", awaiting.join(",")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandoverPaintPhase {
    Painting,
    Suspended { since_ms: u64 },
}

/// Emitted exactly once per edge so the caller can notify/trace without keeping
/// its own copy of the phase (the second-encoding trap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandoverPaintTransition {
    /// Paint just stopped: notify the user, veil the viewport.
    Suspended,
    /// The successor adopted our sessions — resume by the normal read path.
    ResumedAdopted,
    /// Fail-safe ceiling hit. Resume anyway; this handover will not re-arm.
    ResumedTimedOut,
}

impl HandoverPaintTransition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Suspended => "suspended",
            Self::ResumedAdopted => "resumed_adopted",
            Self::ResumedTimedOut => "resumed_timed_out",
        }
    }
}

/// The pure transition. `resolved_fingerprint` is the last handover this gate
/// finished with (or the startup baseline); a handover may only arm the gate
/// once.
pub(crate) fn handover_paint_phase_next(
    phase: HandoverPaintPhase,
    resolved_fingerprint: Option<&str>,
    observation: &HandoverObservation,
    now_ms: u64,
) -> (HandoverPaintPhase, Option<HandoverPaintTransition>) {
    match phase {
        HandoverPaintPhase::Painting => {
            if !observation.wants_suspend() {
                return (HandoverPaintPhase::Painting, None);
            }
            if resolved_fingerprint == Some(observation.fingerprint.as_str()) {
                // Already dealt with this exact handover (adopted, or the
                // ceiling ran out). Re-arming would be a veil the user can
                // never clear.
                return (HandoverPaintPhase::Painting, None);
            }
            (
                HandoverPaintPhase::Suspended { since_ms: now_ms },
                Some(HandoverPaintTransition::Suspended),
            )
        }
        HandoverPaintPhase::Suspended { since_ms } => {
            if !observation.wants_suspend() {
                return (
                    HandoverPaintPhase::Painting,
                    Some(HandoverPaintTransition::ResumedAdopted),
                );
            }
            if now_ms.saturating_sub(since_ms) >= HANDOVER_PAINT_SUSPEND_MAX_MS {
                return (
                    HandoverPaintPhase::Painting,
                    Some(HandoverPaintTransition::ResumedTimedOut),
                );
            }
            (HandoverPaintPhase::Suspended { since_ms }, None)
        }
    }
}

/// The client-side owner of the predicate. One instance, on `ShellState`.
#[derive(Debug, Clone)]
pub(crate) struct HandoverPaintGate {
    phase: HandoverPaintPhase,
    /// `None` until the first observation. The first one is a BASELINE: whatever
    /// the daemon looks like when we first look is "normal", so a GUI that starts
    /// beside a lingering preserved owner never opens behind a veil.
    resolved_fingerprint: Option<String>,
    last_observation: HandoverObservation,
    last_observed_at_ms: u64,
    suspend_count: u64,
    last_transition: Option<HandoverPaintTransition>,
    last_transition_at_ms: u64,
}

impl Default for HandoverPaintGate {
    fn default() -> Self {
        Self {
            phase: HandoverPaintPhase::Painting,
            resolved_fingerprint: None,
            last_observation: HandoverObservation::default(),
            last_observed_at_ms: 0,
            suspend_count: 0,
            last_transition: None,
            last_transition_at_ms: 0,
        }
    }
}

impl HandoverPaintGate {
    /// ⛔ The ONE question every paint/read site asks. Never re-derive it.
    pub(crate) fn paint_suspended(&self) -> bool {
        matches!(self.phase, HandoverPaintPhase::Suspended { .. })
    }

    pub(crate) fn suspended_since_ms(&self) -> Option<u64> {
        match self.phase {
            HandoverPaintPhase::Suspended { since_ms } => Some(since_ms),
            HandoverPaintPhase::Painting => None,
        }
    }

    /// Feed a fresh daemon observation. Returns the edge, if this crossed one.
    pub(crate) fn observe(
        &mut self,
        observation: HandoverObservation,
        now_ms: u64,
    ) -> Option<HandoverPaintTransition> {
        self.last_observed_at_ms = now_ms;
        if self.resolved_fingerprint.is_none() {
            // Baseline. Fail-safe 1.
            self.resolved_fingerprint = Some(observation.fingerprint.clone());
            self.last_observation = observation;
            return None;
        }
        self.last_observation = observation;
        self.settle(now_ms)
    }

    /// Re-run the decision against the LAST observation. The suspension ceiling
    /// is wall-clock, so it must be able to expire on a tick that costs no IPC —
    /// otherwise a daemon that stops answering mid-handover holds the veil for
    /// as long as it stays silent.
    pub(crate) fn tick(&mut self, now_ms: u64) -> Option<HandoverPaintTransition> {
        if !self.paint_suspended() {
            return None;
        }
        self.settle(now_ms)
    }

    fn settle(&mut self, now_ms: u64) -> Option<HandoverPaintTransition> {
        let (next_phase, transition) = handover_paint_phase_next(
            self.phase,
            self.resolved_fingerprint.as_deref(),
            &self.last_observation,
            now_ms,
        );
        self.phase = next_phase;
        if let Some(transition) = transition {
            self.last_transition = Some(transition);
            self.last_transition_at_ms = now_ms;
            match transition {
                HandoverPaintTransition::Suspended => {
                    self.suspend_count = self.suspend_count.saturating_add(1);
                }
                HandoverPaintTransition::ResumedAdopted
                | HandoverPaintTransition::ResumedTimedOut => {
                    // Latch the handover we just finished with so it cannot
                    // arm the gate a second time.
                    self.resolved_fingerprint = Some(self.last_observation.fingerprint.clone());
                }
            }
        }
        transition
    }

    /// `server app state` projection so an agent can probe the predicate live.
    pub(crate) fn to_app_state_json(&self, now_ms: u64) -> Value {
        json!({
            "paint_suspended": self.paint_suspended(),
            "suspended_for_ms": self
                .suspended_since_ms()
                .map(|since_ms| now_ms.saturating_sub(since_ms)),
            "suspend_ceiling_ms": HANDOVER_PAINT_SUSPEND_MAX_MS,
            "handoff_in_flight": self.last_observation.handoff_in_flight,
            "client_sessions_awaiting_adoption": self
                .last_observation
                .client_sessions_awaiting_adoption,
            "fingerprint": self.last_observation.fingerprint,
            "resolved_fingerprint": self.resolved_fingerprint,
            "suspend_count": self.suspend_count,
            "last_transition": self.last_transition.map(HandoverPaintTransition::as_str),
            "last_transition_at_ms": self.last_transition_at_ms,
            "last_observed_at_ms": self.last_observed_at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(awaiting: bool, fingerprint: &str) -> HandoverObservation {
        HandoverObservation {
            handoff_in_flight: awaiting,
            client_sessions_awaiting_adoption: awaiting,
            fingerprint: fingerprint.to_string(),
        }
    }

    fn baselined_gate() -> HandoverPaintGate {
        let mut gate = HandoverPaintGate::default();
        // Fail-safe 1: the first observation only establishes the baseline.
        assert_eq!(gate.observe(observation(false, "d1|"), 1_000), None);
        gate
    }

    #[test]
    fn first_observation_is_a_baseline_and_never_veils_a_starting_gui() {
        let mut gate = HandoverPaintGate::default();
        // A GUI that starts while an older daemon still holds one of the
        // sessions it mounts must NOT open behind a veil.
        assert_eq!(
            gate.observe(observation(true, "d1|local://a"), 1_000),
            None,
            "the first observation is the baseline, not a handover edge"
        );
        assert!(!gate.paint_suspended());
    }

    #[test]
    fn handover_suspends_paint_then_resumes_when_the_successor_adopts() {
        let mut gate = baselined_gate();
        assert_eq!(
            gate.observe(observation(true, "d2|local://a"), 2_000),
            Some(HandoverPaintTransition::Suspended)
        );
        assert!(gate.paint_suspended());
        assert_eq!(gate.suspended_since_ms(), Some(2_000));

        // Still in flight -> no repeat edge, still suspended.
        assert_eq!(gate.observe(observation(true, "d2|local://a"), 5_000), None);
        assert!(gate.paint_suspended());

        // Adopted: our key left the preserved set.
        assert_eq!(
            gate.observe(observation(false, "d2|"), 9_000),
            Some(HandoverPaintTransition::ResumedAdopted)
        );
        assert!(!gate.paint_suspended());
    }

    #[test]
    fn suspension_times_out_and_the_same_handover_cannot_re_arm() {
        let mut gate = baselined_gate();
        assert_eq!(
            gate.observe(observation(true, "d2|local://a"), 2_000),
            Some(HandoverPaintTransition::Suspended)
        );
        // A tick costs no IPC and must still be able to expire the ceiling —
        // a daemon that goes silent mid-handover would otherwise hold the veil.
        assert_eq!(gate.tick(2_000 + HANDOVER_PAINT_SUSPEND_MAX_MS - 1), None);
        assert!(gate.paint_suspended());
        assert_eq!(
            gate.tick(2_000 + HANDOVER_PAINT_SUSPEND_MAX_MS),
            Some(HandoverPaintTransition::ResumedTimedOut)
        );
        assert!(!gate.paint_suspended());

        // The signal never cleared (an older daemon is lingering). The gate must
        // NOT re-arm on it, or the veil comes back forever.
        for step in 0..5 {
            assert_eq!(
                gate.observe(
                    observation(true, "d2|local://a"),
                    200_000 + step * HANDOVER_PAINT_SUSPEND_MAX_MS
                ),
                None,
                "a resolved handover must never re-arm the gate"
            );
            assert!(!gate.paint_suspended());
        }

        // A genuinely NEW handover (different daemon / different key set) does.
        assert_eq!(
            gate.observe(observation(true, "d3|local://a"), 900_000),
            Some(HandoverPaintTransition::Suspended)
        );
    }

    #[test]
    fn an_observation_with_no_handoff_evidence_resumes_paint() {
        let mut gate = baselined_gate();
        assert_eq!(
            gate.observe(observation(true, "d2|local://a"), 2_000),
            Some(HandoverPaintTransition::Suspended)
        );
        // Fail-safe 2: never hold paint back on an absence of evidence. (A
        // daemon that stops ANSWERING never reaches the gate at all — the shell
        // does not manufacture an observation from silence — so that half is
        // covered by the wall-clock ceiling above.)
        assert_eq!(
            gate.observe(HandoverObservation::default(), 3_000),
            Some(HandoverPaintTransition::ResumedAdopted)
        );
        assert!(!gate.paint_suspended());
    }

    #[test]
    fn another_agents_session_migrating_does_not_veil_this_client() {
        // ⚖ Version-coexisting daemons: a lingering older daemon holding
        // SOMEONE ELSE'S session is the normal state, not our handover.
        let observation = handover_observation_from_parts(
            true,
            "pid=7:2.12.16",
            &["local://other-agent".to_string()],
            &["local://mine".to_string()],
        );
        assert!(observation.handoff_in_flight);
        assert!(!observation.client_sessions_awaiting_adoption);
        assert!(!observation.wants_suspend());

        let mut gate = baselined_gate();
        assert_eq!(gate.observe(observation, 2_000), None);
        assert!(!gate.paint_suspended());
    }

    #[test]
    fn observation_fingerprint_tracks_the_client_relevant_slice_only() {
        let mine = vec!["local://mine".to_string()];
        let first = handover_observation_from_parts(
            true,
            "pid=7:2.12.16",
            &["local://mine".to_string(), "local://other".to_string()],
            &mine,
        );
        // Another agent's key coming and going must not move OUR fingerprint,
        // or every progressive-migration tick would re-veil the user.
        let second = handover_observation_from_parts(
            true,
            "pid=7:2.12.16",
            &["local://mine".to_string()],
            &mine,
        );
        assert_eq!(first.fingerprint, second.fingerprint);
        assert!(first.wants_suspend());

        // A different daemon serving us IS a new handover.
        let third = handover_observation_from_parts(
            true,
            "pid=9:2.12.17",
            &["local://mine".to_string()],
            &mine,
        );
        assert_ne!(first.fingerprint, third.fingerprint);
    }

    #[test]
    fn app_state_projection_reports_the_predicate_and_its_ceiling() {
        let mut gate = baselined_gate();
        gate.observe(observation(true, "d2|local://a"), 2_000);
        let json = gate.to_app_state_json(6_000);
        assert_eq!(json["paint_suspended"], json!(true));
        assert_eq!(json["suspended_for_ms"], json!(4_000));
        assert_eq!(
            json["suspend_ceiling_ms"],
            json!(HANDOVER_PAINT_SUSPEND_MAX_MS)
        );
        assert_eq!(json["last_transition"], json!("suspended"));
        assert_eq!(json["suspend_count"], json!(1));
    }
}
