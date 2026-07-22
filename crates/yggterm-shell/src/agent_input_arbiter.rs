//! Agent-input preemption — **the human always wins** (agent control plane,
//! acceptance gate 9 / F3).
//!
//! # What this owns, and what it deliberately does not
//!
//! Gate 9 has two halves. Only one of them was missing:
//!
//! - **Ordering / one-verb-in-flight is ALREADY OWNED** by the app-control
//!   request pump: `process_pending_app_control_requests` takes ONE request at a
//!   time behind an `app_control_drain_in_flight` guard and awaits it before the
//!   next. Verbs from two agents therefore cannot overlap, and they dispatch in
//!   arrival order for free. Adding a queue here would be a second encoding of
//!   ordering that could silently disagree with the pump — so this module does
//!   **not** queue, sequence, or dispatch anything.
//! - **Preemption was missing entirely.** Nothing cancelled an agent's remaining
//!   work when the user took a surface back, so a verb the agent planned before
//!   the human acted could still land afterwards.
//!
//! So this is a small, sharp thing: per-surface **batch** state, and the rule
//! that once the human touches a surface, every batch that was driving it is
//! refused from then on.
//!
//! # Batches
//!
//! An agent's run of verbs is a [`AgentBatch`], keyed by the slice-3 agent
//! identity (`--agent`). Preemption cancels a BATCH, not a single verb: the
//! point is that nothing *further* from that run lands after the user acted.
//! An agent that re-observes the surface and starts a new batch is welcome —
//! preemption is not a lockout.
//!
//! # Slice 4.1 forward-compatibility — one table, not two
//!
//! Slice 4.1 extends THIS state across clients by keying on `(client_id, role)`;
//! [`AgentBatch::client_id`] is that seat, unused (`None`) while there is one
//! client. 4.1 must not add a parallel lease table beside it. Slice 4.1c's
//! write-lock-loss preemption reuses [`AgentInputArbiter::preempt_surface`] —
//! the shared cancel-all-batches primitive — rather than a second cancel path.
//!
//! # What still needs plumbing (be honest about this)
//!
//! *Detecting* real seat input is not decided here, and it is not trivial:
//! yggterm's injection produces `isTrusted: true` events (the slice-2a spike),
//! so a page-side listener **cannot** tell agent input from human input. The
//! caller decides what counts and calls [`AgentInputArbiter::note_human_input`].
//! Until a detector is wired, the refusal path below is exercised by tests but
//! never fires in production — see `docs/agent-control-plane.md` gate 9.

/// Why a verb was refused. Also the journal reason.
pub const PREEMPTED: &str = "preempted";

/// A surface an agent can drive. `generation` is the slice-2b incarnation
/// counter: a recreated surface is a DIFFERENT surface, so work aimed at the
/// old one can never fire against the new document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceKey {
    pub session_path: String,
    pub generation: u64,
}

impl SurfaceKey {
    pub fn new(session_path: impl Into<String>, generation: u64) -> Self {
        Self {
            session_path: session_path.into(),
            generation,
        }
    }
}

/// One agent's run of verbs against one surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBatch {
    /// Stable per-run identity — the slice-3 agent id (`--agent`), or a
    /// synthesized one for an anonymous caller.
    pub batch_id: String,
    /// Slice-4.1 seat: which CLIENT owns this batch. `None` = the single-client
    /// world gate 9 covers. Present so 4.1 keys this table rather than adding one.
    pub client_id: Option<String>,
}

impl AgentBatch {
    pub fn new(batch_id: impl Into<String>) -> Self {
        Self {
            batch_id: batch_id.into(),
            client_id: None,
        }
    }
}

/// May this verb proceed?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitOutcome {
    /// Proceed. (The app-control pump provides the ordering.)
    Allowed,
    /// The human took this surface; the batch is cancelled and stays cancelled.
    Preempted,
    /// The surface was recreated under this batch (generation moved on).
    StaleSurface,
}

/// What a human-input event cancelled, for the journal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreemptReport {
    pub cancelled_batches: Vec<String>,
}

impl PreemptReport {
    pub fn is_empty(&self) -> bool {
        self.cancelled_batches.is_empty()
    }
}

#[derive(Debug, Default)]
struct SurfaceLane {
    /// Batches that have driven this surface and are still live.
    active_batches: Vec<String>,
    /// Batches the human cancelled. A late verb from one of these is refused
    /// rather than silently landing behind the user's back.
    preempted_batches: Vec<String>,
}

/// Per-surface agent batch + preemption state.
#[derive(Debug, Default)]
pub struct AgentInputArbiter {
    lanes: std::collections::BTreeMap<SurfaceKey, SurfaceLane>,
}

impl AgentInputArbiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide whether a verb from `batch` may drive `surface`, recording the
    /// batch as active when it may. `live_generation` is the surface's CURRENT
    /// incarnation (slice-2b F3).
    pub fn admit(
        &mut self,
        surface: &SurfaceKey,
        batch: &AgentBatch,
        live_generation: u64,
    ) -> AdmitOutcome {
        if surface.generation != live_generation {
            return AdmitOutcome::StaleSurface;
        }
        let lane = self.lanes.entry(surface.clone()).or_default();
        if lane
            .preempted_batches
            .iter()
            .any(|id| id == &batch.batch_id)
        {
            return AdmitOutcome::Preempted;
        }
        if !lane.active_batches.contains(&batch.batch_id) {
            lane.active_batches.push(batch.batch_id.clone());
        }
        AdmitOutcome::Allowed
    }

    /// **A preemption event landed on `surface`** — cancel every batch driving
    /// it, so no further verb from those runs is admitted. This is the SHARED
    /// primitive behind both preemption causes, one mechanism / one table:
    /// - real seat input (gate 9, [`Self::note_human_input`]);
    /// - slice-4.1c write-lock loss (a `Shadow` whose profile write-lock an
    ///   `Active` client preempted must stop injecting into that jar).
    /// The CAUSE lives only in the caller's journal, never in a second lane map.
    pub fn preempt_surface(&mut self, surface: &SurfaceKey) -> PreemptReport {
        let Some(lane) = self.lanes.get_mut(surface) else {
            return PreemptReport::default();
        };
        let mut report = PreemptReport::default();
        for batch_id in lane.active_batches.drain(..) {
            if !lane.preempted_batches.contains(&batch_id) {
                lane.preempted_batches.push(batch_id.clone());
            }
            report.cancelled_batches.push(batch_id);
        }
        report
    }

    /// **Real seat input arrived on `surface`** (gate 9). Delegates to
    /// [`Self::preempt_surface`]: the human's input is never queued behind an
    /// agent — the pump is already serial, and after this call the agent simply
    /// has nothing left to dispatch.
    pub fn note_human_input(&mut self, surface: &SurfaceKey) -> PreemptReport {
        self.preempt_surface(surface)
    }

    /// Has this batch been preempted on this surface?
    pub fn is_preempted(&self, surface: &SurfaceKey, batch_id: &str) -> bool {
        self.lanes
            .get(surface)
            .is_some_and(|lane| lane.preempted_batches.iter().any(|id| id == batch_id))
    }

    /// Drop a surface's state (surface closed or recreated).
    pub fn forget(&mut self, surface: &SurfaceKey) {
        self.lanes.remove(surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> SurfaceKey {
        SurfaceKey::new("web://session-a", 1)
    }

    // Acceptance gate 9: after real seat input, no further verb from the
    // agent's batch dispatches.
    #[test]
    fn human_input_cancels_the_batch_and_later_verbs_are_refused() {
        let s = surface();
        let mut arb = AgentInputArbiter::new();
        let a = AgentBatch::new("agent-a");
        assert_eq!(arb.admit(&s, &a, 1), AdmitOutcome::Allowed);

        let report = arb.note_human_input(&s);
        assert_eq!(report.cancelled_batches, vec!["agent-a".to_string()]);

        // The agent's next verb — planned before the human acted — is refused.
        assert_eq!(arb.admit(&s, &a, 1), AdmitOutcome::Preempted);
        assert!(arb.is_preempted(&s, "agent-a"));
    }

    #[test]
    fn every_batch_driving_the_surface_is_cancelled_not_just_one() {
        let s = surface();
        let mut arb = AgentInputArbiter::new();
        let a = AgentBatch::new("agent-a");
        let b = AgentBatch::new("agent-b");
        arb.admit(&s, &a, 1);
        arb.admit(&s, &b, 1);

        let report = arb.note_human_input(&s);
        assert_eq!(report.cancelled_batches.len(), 2);
        assert_eq!(arb.admit(&s, &a, 1), AdmitOutcome::Preempted);
        assert_eq!(arb.admit(&s, &b, 1), AdmitOutcome::Preempted);
    }

    #[test]
    fn preempt_does_not_touch_other_surfaces() {
        let mut arb = AgentInputArbiter::new();
        let s1 = SurfaceKey::new("web://session-a", 1);
        let s2 = SurfaceKey::new("web://session-b", 1);
        let a = AgentBatch::new("agent-a");
        arb.admit(&s1, &a, 1);
        arb.admit(&s2, &a, 1);

        // The user typing into surface 1 says nothing about surface 2.
        arb.note_human_input(&s1);
        assert!(arb.is_preempted(&s1, "agent-a"));
        assert!(!arb.is_preempted(&s2, "agent-a"));
        assert_eq!(arb.admit(&s2, &a, 1), AdmitOutcome::Allowed);
    }

    #[test]
    fn a_new_batch_may_drive_the_surface_after_a_preempt() {
        let s = surface();
        let mut arb = AgentInputArbiter::new();
        let old = AgentBatch::new("agent-a");
        arb.admit(&s, &old, 1);
        arb.note_human_input(&s);

        // Preemption cancels a RUN, it does not lock the surface forever — an
        // agent that re-observes and starts fresh is allowed.
        let fresh = AgentBatch::new("agent-a-run-2");
        assert_eq!(arb.admit(&s, &fresh, 1), AdmitOutcome::Allowed);
    }

    #[test]
    fn a_recreated_surface_refuses_verbs_aimed_at_the_old_incarnation() {
        let mut arb = AgentInputArbiter::new();
        let stale = SurfaceKey::new("web://session-a", 1);
        let a = AgentBatch::new("agent-a");
        // The surface has moved on to generation 2 (slice-2b F3).
        assert_eq!(arb.admit(&stale, &a, 2), AdmitOutcome::StaleSurface);
        // ...and a stale verb must not register the batch as active either.
        assert!(!arb.is_preempted(&stale, "agent-a"));
    }

    #[test]
    fn human_input_on_an_idle_surface_is_a_no_op() {
        let mut arb = AgentInputArbiter::new();
        let report = arb.note_human_input(&surface());
        assert!(report.is_empty());
    }

    #[test]
    fn forget_clears_preemption_with_the_surface() {
        let s = surface();
        let mut arb = AgentInputArbiter::new();
        let a = AgentBatch::new("agent-a");
        arb.admit(&s, &a, 1);
        arb.note_human_input(&s);
        assert!(arb.is_preempted(&s, "agent-a"));

        arb.forget(&s);
        assert!(!arb.is_preempted(&s, "agent-a"));
    }

    // Slice 4.1c: write-lock loss cancels the batch through the SAME primitive
    // seat input uses — a preempted Shadow's later verbs are refused, exactly as
    // after a human touch. (`note_human_input` is a named alias for this cause.)
    #[test]
    fn preempt_surface_is_the_shared_primitive_for_write_lock_loss() {
        let s = surface();
        let mut arb = AgentInputArbiter::new();
        let shadow_batch = AgentBatch {
            batch_id: "shadow-run".into(),
            client_id: Some("shadow-1".into()),
        };
        assert_eq!(arb.admit(&s, &shadow_batch, 1), AdmitOutcome::Allowed);
        // The user's Active GUI preempted the shadow's profile write-lock.
        let report = arb.preempt_surface(&s);
        assert_eq!(report.cancelled_batches, vec!["shadow-run".to_string()]);
        // The shadow's next verb is refused — it may not write the jar it lost.
        assert_eq!(arb.admit(&s, &shadow_batch, 1), AdmitOutcome::Preempted);
    }

    #[test]
    fn the_client_id_seat_for_slice_4_1_is_carried_but_unused_today() {
        // Guards the "one table, not two" decision: 4.1 keys admission on
        // (client_id, role) using THIS struct rather than a parallel table.
        let batch = AgentBatch::new("agent-a");
        assert!(batch.client_id.is_none());
        let cross_client = AgentBatch {
            batch_id: "agent-a".into(),
            client_id: Some("shadow-1".into()),
        };
        assert_eq!(cross_client.client_id.as_deref(), Some("shadow-1"));
    }
}
