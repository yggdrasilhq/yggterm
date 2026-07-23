#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetainedRehydrateMode {
    InitialRead,
    CollapsedScrollbackRecovery,
}

impl RetainedRehydrateMode {
    pub(crate) fn as_key(self) -> &'static str {
        match self {
            Self::InitialRead => "initial-read",
            Self::CollapsedScrollbackRecovery => "collapsed-scrollback-recovery",
        }
    }
}

// XTERM-BUG: blank-viewport-client-snapshot-poison
// Whether a retained-rehydrate reveal may fall back to the daemon's AUTHORITATIVE
// screen snapshot when the retained payload is a cursor-addressed (codex) frame with
// no plain scrollback history. For a CollapsedScrollbackRecovery reveal (the live
// broken case: switch-back / surface re-reveal) or any codex-like session, the daemon
// screen frame is the real current content and MUST be offered, so the client's
// reconcile-from-daemon path (daemon_screen_snapshot) engages. Otherwise the selector
// returns daemon_retained_snapshot, which the client downgrades to its own sparse
// xterm_session_snapshot -> clip + truncated/broken composer bottom paint.
pub(crate) fn retained_rehydrate_allow_screen_fallback(
    mode: RetainedRehydrateMode,
    codex_like: bool,
) -> bool {
    codex_like || matches!(mode, RetainedRehydrateMode::CollapsedScrollbackRecovery)
}
/// The minimum scrollback depth (baseY) for a client buffer to count as a "rich"
/// transcript worth protecting from a collapse-reseed.
pub(crate) const RICH_CLIENT_MIN_NONBLANK: u32 = 6;

/// Cold-re-resume vacuum guard (sum-total run #3 redesign of the reverted 2.8.64
/// guard): refuse a daemon-frame reseed that would VACUUM a substantially richer
/// client buffer — but ONLY when the frame was read from a DIFFERENT runtime
/// spawn than the one the client buffer was seeded from (`incoming_from_new_runtime`).
///
/// The scenario: a codex runtime exits (e.g. a system update drops the SSH
/// transport) → the daemon cold-re-resumes it on a FRESH PTY whose vt100 screen is
/// ~8 lines (codex repaints in place, so the conversation was NEVER in the daemon
/// scrollback — it lives ONLY in the client xterm buffer) → reseeding the client
/// from that sparse frame collapses the whole transcript (live-caught: base_y
/// 1801 → 32).
///
/// WHY the runtime-spawn AND is load-bearing (the 2.8.64 lesson): codex daemon
/// frames are ALWAYS small relative to an accumulated client baseY, so a blanket
/// magnitude ratio fires on every normal codex reveal and gates the
/// reveal-reconcile into a persistent shadow + gate notification (sum-test obs
/// #2/#4/#5 — reverted in 2.8.65). With the spawn-id signal, a same-runtime
/// reveal can NEVER guard; only a genuinely replaced runtime arms the ratio.
/// Unknown spawn ids (0, e.g. older daemon or the user/agent reconcile that owns
/// the churn decision) keep `incoming_from_new_runtime` false → fails open.
///
/// Failure mode is benign by construction: at worst it keeps slightly-stale rich
/// content that the fresh runtime's next comparably-rich frame replaces normally
/// — never a vacuum.
pub(crate) fn retained_replay_would_vacuum_richer_client(
    client_richness: u32,
    incoming_richness: u32,
    incoming_from_new_runtime: bool,
) -> bool {
    incoming_from_new_runtime
        && client_richness >= RICH_CLIENT_MIN_NONBLANK
        && incoming_richness.saturating_mul(3) < client_richness
}

/// Boring retained reveal (spec-boring-session-loads, lane 1): how the
/// bootstrap read loop should (re)enter the daemon chunk stream for a host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetainedRevealReadPlan {
    /// The host's xterm entry survived backgrounding with this stream's
    /// content already painted: resume reads at the saved cursor and APPEND
    /// only the missed delta — no reset, no full replay. The harness locks
    /// the safety property (stream-split invariance: split feeds equal the
    /// un-split feed; tools/xterm-harness/boring_reveal_resume.test.js), so
    /// the revealed buffer ends exactly as if the session never detached.
    ResumeAppend { cursor: u64 },
    /// Fresh mount (or no healthy saved cursor): read from 0, full replay.
    FullReplay,
}

/// The saved cursor is keyed by (session_path, host_id): the host id is only
/// reused when the GUI retained the SAME terminal host element (and thus the
/// same painted xterm entry) across the background/reveal cycle, so presence
/// of a positive saved cursor IS the "retained and already painted" signal —
/// a scenario signal, never a magnitude ratio (the 2.8.64 lesson). The store
/// must only be written by a healthy read loop and cleared on every reset /
/// cursor-rewind / fault-recovery path, so a stale or churned host never
/// resumes.
pub(crate) fn retained_reveal_read_plan(saved_cursor: Option<u64>) -> RetainedRevealReadPlan {
    match saved_cursor {
        Some(cursor) if cursor > 0 => RetainedRevealReadPlan::ResumeAppend { cursor },
        _ => RetainedRevealReadPlan::FullReplay,
    }
}

/// When resuming (ResumeAppend), the InitialRead retained-rehydrate task is
/// pure churn: it re-reads the stream from 0 and reset+replays it into the
/// already-painted buffer — that reset IS the blink-blink shadow the spec
/// rejects. Suppress it. A CollapsedScrollbackRecovery rehydrate is a FAULT
/// path (the surface is already known broken) and must stay armed.
pub(crate) fn retained_reveal_resume_suppresses_rehydrate(
    plan: RetainedRevealReadPlan,
    mode: RetainedRehydrateMode,
) -> bool {
    matches!(plan, RetainedRevealReadPlan::ResumeAppend { .. })
        && matches!(mode, RetainedRehydrateMode::InitialRead)
}

/// Outcome of the FIRST read after a ResumeAppend, decided from the daemon's
/// own stream signals (deterministic, no heuristics):
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResumedReadOutcome {
    /// Contiguous tail: append the chunks. The boring reveal.
    AppendChunks,
    /// The live ring trimmed below our cursor (`resync_required`): the
    /// returned tail skips a contiguous middle, so appending it would corrupt
    /// the buffer (docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream).
    /// Skip the chunks, advance the cursor past the gap, and arm the
    /// scrollback-preserving visible-screen reconcile (the shipped 2.8.69
    /// daemon vt100 write) to repaint the authoritative bottom. The client
    /// keeps its scrollback; only the unrecoverable gap middle is absent —
    /// strictly more than the old reset+replay preserved. NEVER replay
    /// history into the buffer here (the gap-fix cascade trap: history
    /// written into an alternate-screen TUI corrupted codex on switch-back).
    SkipGapReconcileScreen,
    /// The daemon's cursor rewound below ours: the runtime was re-created
    /// (cold re-resume). The existing rewind path owns this — full reset +
    /// replay of the fresh stream.
    FullResetReplay,
}

pub(crate) fn resumed_read_outcome(
    resync_required: bool,
    cursor_rewound: bool,
) -> ResumedReadOutcome {
    if cursor_rewound {
        ResumedReadOutcome::FullResetReplay
    } else if resync_required {
        ResumedReadOutcome::SkipGapReconcileScreen
    } else {
        ResumedReadOutcome::AppendChunks
    }
}

pub(crate) fn retained_ready_remote_host_should_reuse_bootstrap(
    is_remote_resume_session: bool,
    retained: bool,
    resume_ready: bool,
    host_id_present: bool,
) -> bool {
    is_remote_resume_session && retained && resume_ready && host_id_present
}

/// The §7.3 stable-epoch generalization (harness spec phase-3 scope; the
/// user-felt "zoom" in docs/pending-bugs.md): `bootstrap_activation_epoch`
/// returns `latest_open_request_id` for the active session, so EVERY open
/// request — switch reveal or the gesture-free requests observed at output
/// boundaries — re-ran the full bootstrap (new closure, new Terminal, ghost
/// cover, fit + restore = the felt zoom). The pre-existing pins
/// (`retained_ready_remote_host_should_reuse_bootstrap`,
/// `remote_resume_stable_bootstrap_epoch`) require `is_remote_resume_session`
/// — remote-CODEX-only — so cc and local sessions reconstructed per request
/// while remote-codex sat stable.
///
/// This predicate pins the epoch for ANY session — kind- and
/// locality-agnostic — whose host is retained, reached ready at least once,
/// and shows no fault:
/// - `daemon_owns_runtime` mirrors the reveal-reuse predicate
///   (`terminal_session_host_reusable_for_reveal`): a runtime the daemon lost
///   needs a real re-bootstrap, not a reveal.
/// - a latched fault or a failed/timed-out resume overlay unpins, so every
///   existing failure path re-bootstraps exactly as before.
/// - genuine fault recovery bumps the MOUNT epoch directly (never through the
///   activation epoch), so pinning here can never mask a remount.
///
/// The caller pairs the pin with a reveal NUDGE against the retained closure
/// (`emitResize` + `redrawTerminal`) — skipping the bootstrap without the
/// nudge risks a BLANK reveal for an idle session whose parked canvas lost
/// its backing store, which is worse than the zoom.
pub(crate) fn retained_ever_ready_host_should_pin_bootstrap_epoch(
    retained: bool,
    was_ever_ready: bool,
    daemon_owns_runtime: bool,
    latched_fault: bool,
    host_id_present: bool,
    resume_overlay_failed: bool,
    resume_overlay_timed_out: bool,
) -> bool {
    retained
        && was_ever_ready
        && daemon_owns_runtime
        && !latched_fault
        && host_id_present
        && !resume_overlay_failed
        && !resume_overlay_timed_out
}

pub(crate) fn retained_remote_host_should_rehydrate(
    is_remote_resume_session: bool,
    retained: bool,
    resume_ready: bool,
    retained_fault_recovery: bool,
    host_id_present: bool,
) -> bool {
    retained_ready_remote_host_should_reuse_bootstrap(
        is_remote_resume_session,
        retained,
        resume_ready,
        host_id_present,
    ) || (is_remote_resume_session && retained && retained_fault_recovery && host_id_present)
}

pub(crate) fn retained_rehydrate_identity_key(
    session_path: &str,
    mount_identity: &str,
    retained_ready_remote_host: bool,
    active_host_selected: bool,
    mode: Option<RetainedRehydrateMode>,
) -> String {
    let mode_key = mode
        .map(RetainedRehydrateMode::as_key)
        .unwrap_or("disabled");
    format!(
        "retained-rehydrate:{session_path}:{mount_identity}:{retained_ready_remote_host}:{active_host_selected}:{mode_key}"
    )
}

pub(crate) fn retained_rehydrate_ready_history_retry_reason(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some("active terminal host is only showing a plain shell prompt")
            | Some("active terminal host exists but xterm surface is empty")
            | Some("active terminal host is still showing generic Codex idle chrome")
            | Some("active remote terminal lost expected scrollback after retained replay")
            | Some("active remote terminal received scroll input but has no xterm scrollback")
            | Some("active remote Codex prompt surface has stale scrollback but no current prompt")
            | Some("active remote Codex prompt surface has no current input row")
    )
}

pub(crate) fn retained_ready_remote_host_rehydrate_mode(
    retained_ready_remote_host: bool,
    active_host_selected: bool,
    ready_attempt: bool,
    ready_history: bool,
    terminal_live_host_connected: bool,
    surface_problem: Option<&str>,
    // TODO-1 (campaign): a SETTLED-IDLE codex reveal may reconcile-from-daemon ONCE
    // (dedup'd per mount via the identity key) to repaint the authoritative bottom and
    // kill the reveal-shadow / broken-bottom-paint blink. This deliberately bypasses
    // the `terminal_live_host_connected` gate — which normally blocks rehydrate because
    // re-reading a LIVE session is the recovery-churn trap that broke live sessions
    // (incident-gap-fix-cascade-2026-06-03). It is SAFE ONLY because the caller passes
    // true exclusively when the surface is settled-idle (ready + no surface problem +
    // NOT a codex working surface), so there is no in-flight frame to churn. When in
    // doubt the caller passes false → old behavior (no reconcile), never a regression.
    codex_idle_reveal_reconcile: bool,
) -> Option<RetainedRehydrateMode> {
    if !retained_ready_remote_host || !active_host_selected || ready_attempt {
        return None;
    }
    // Connected sessions normally do NOT rehydrate (recovery-churn trap). The only
    // exception is a settled-idle codex reveal, which reconciles exactly once.
    if terminal_live_host_connected && !codex_idle_reveal_reconcile {
        return None;
    }
    if retained_rehydrate_ready_history_retry_reason(surface_problem) {
        return Some(RetainedRehydrateMode::CollapsedScrollbackRecovery);
    }
    if !ready_history {
        return Some(RetainedRehydrateMode::InitialRead);
    }
    // A settled-idle codex reveal with existing history still reconciles once to
    // repaint the daemon-authoritative bottom (the reveal-shadow fix).
    if codex_idle_reveal_reconcile {
        return Some(RetainedRehydrateMode::InitialRead);
    }
    None
}

pub(crate) fn daemon_retained_snapshot_replay_identity_key(
    session_path: &str,
    mount_identity: &str,
    is_remote_resume_session: bool,
    host_is_active_session: bool,
    active_host_selected: bool,
) -> String {
    format!(
        "daemon-retained-replay:{session_path}:{mount_identity}:{is_remote_resume_session}:{host_is_active_session}:{active_host_selected}"
    )
}

pub(crate) fn daemon_retained_snapshot_replay_should_start(
    is_remote_resume_session: bool,
    remote_starting_codex_session: bool,
    codex_like_session: bool,
    host_is_active_session: bool,
    active_host_selected: bool,
    terminal_ready_for_retained_replay: bool,
    terminal_live_host_connected: bool,
    retained_snapshot_already_staged: bool,
) -> bool {
    let _ = (remote_starting_codex_session, codex_like_session);
    is_remote_resume_session
        && host_is_active_session
        && active_host_selected
        && terminal_ready_for_retained_replay
        && !terminal_live_host_connected
        && !retained_snapshot_already_staged
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn blank_host_snapshot_replay_should_start(
    is_remote_resume_session: bool,
    terminal_paint_seen: bool,
    terminal_geometry_ready: bool,
    has_transport_error: bool,
    cursor_line_text: &str,
    text_tail: &str,
    runtime_running: bool,
    terminal_live_host_connected: bool,
    attempts: u64,
    max_attempts: u64,
) -> bool {
    is_remote_resume_session
        && terminal_paint_seen
        && terminal_geometry_ready
        && !has_transport_error
        && cursor_line_text.trim().is_empty()
        && text_tail.trim().is_empty()
        && runtime_running
        && !terminal_live_host_connected
        && attempts < max_attempts
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn blank_host_snapshot_replay_from_read_should_start(
    is_remote_resume_session: bool,
    batched_output_empty: bool,
    runtime_running: bool,
    runtime_output_seen: bool,
    terminal_paint_seen: bool,
    terminal_geometry_ready: bool,
    terminal_has_visible_output: bool,
    terminal_live_host_connected: bool,
    attempts: u64,
    max_attempts: u64,
) -> bool {
    is_remote_resume_session
        && batched_output_empty
        && runtime_running
        && runtime_output_seen
        && terminal_paint_seen
        && terminal_geometry_ready
        && !terminal_has_visible_output
        && !terminal_live_host_connected
        && attempts < max_attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    // Boring retained reveal (spec-boring-session-loads lane 1): a positive
    // saved cursor — written only by a healthy loop for this exact
    // (session_path, host_id) — is the ONLY thing that arms ResumeAppend.
    #[test]
    fn retained_reveal_resumes_only_on_healthy_saved_cursor() {
        assert_eq!(
            retained_reveal_read_plan(Some(249)),
            RetainedRevealReadPlan::ResumeAppend { cursor: 249 }
        );
        assert_eq!(
            retained_reveal_read_plan(Some(0)),
            RetainedRevealReadPlan::FullReplay,
            "a zero cursor means nothing was consumed; full replay"
        );
        assert_eq!(retained_reveal_read_plan(None), RetainedRevealReadPlan::FullReplay);
    }

    // The reset+replay rehydrate is suppressed ONLY when resuming and ONLY for
    // the InitialRead flavor; the CollapsedScrollbackRecovery fault path stays
    // armed (the surface is already known broken — recovery owns the churn).
    #[test]
    fn resume_suppresses_initial_rehydrate_but_never_fault_recovery() {
        let resume = RetainedRevealReadPlan::ResumeAppend { cursor: 7 };
        assert!(retained_reveal_resume_suppresses_rehydrate(
            resume,
            RetainedRehydrateMode::InitialRead
        ));
        assert!(!retained_reveal_resume_suppresses_rehydrate(
            resume,
            RetainedRehydrateMode::CollapsedScrollbackRecovery
        ));
        assert!(!retained_reveal_resume_suppresses_rehydrate(
            RetainedRevealReadPlan::FullReplay,
            RetainedRehydrateMode::InitialRead
        ));
    }

    // The first resumed read is decided purely from the daemon's stream
    // signals: rewound cursor (runtime re-created) outranks a ring-trim gap;
    // a gap skips the discontiguous chunks and reconciles the screen instead
    // of appending (append-on-gap corrupts; replay-history-on-gap was the
    // gap-fix cascade trap). Contiguous tail = boring append.
    #[test]
    fn resumed_read_outcome_orders_rewind_over_gap_over_append() {
        assert_eq!(
            resumed_read_outcome(false, false),
            ResumedReadOutcome::AppendChunks
        );
        assert_eq!(
            resumed_read_outcome(true, false),
            ResumedReadOutcome::SkipGapReconcileScreen
        );
        assert_eq!(
            resumed_read_outcome(false, true),
            ResumedReadOutcome::FullResetReplay
        );
        assert_eq!(
            resumed_read_outcome(true, true),
            ResumedReadOutcome::FullResetReplay,
            "a rewound cursor means a fresh stream — the gap signal is moot"
        );
    }

    #[test]
    fn cold_re_resume_vacuum_guard_refuses_only_new_runtime_collapse() {
        // The live-caught vacuum: client baseY 1801, fresh-PTY frame 32 lines,
        // runtime replaced → KEEP the client.
        assert!(retained_replay_would_vacuum_richer_client(1801, 32, true));
        assert!(retained_replay_would_vacuum_richer_client(60, 8, true));
        // Threshold: rich client + near-empty frame from a new runtime → guard.
        assert!(retained_replay_would_vacuum_richer_client(
            RICH_CLIENT_MIN_NONBLANK,
            1,
            true
        ));
        // New runtime but the frame is comparably rich → a real reconcile, allow.
        assert!(!retained_replay_would_vacuum_richer_client(60, 55, true));
        assert!(!retained_replay_would_vacuum_richer_client(30, 12, true));
        // Client itself sparse (genuine blank/first reveal) → allow the replay.
        assert!(!retained_replay_would_vacuum_richer_client(5, 0, true));
        assert!(!retained_replay_would_vacuum_richer_client(0, 0, true));
    }

    // THE 2.8.64 REGRESSION LOCK: a SAME-runtime reveal must NEVER guard, no
    // matter how extreme the magnitude ratio — codex daemon frames are always
    // small vs an accumulated client baseY, and the blanket ratio gated every
    // normal codex reveal-reconcile into a persistent shadow + gate notification
    // (sum-test obs #2/#4/#5, reverted in 2.8.65). Unknown spawn (false) = same.
    #[test]
    fn same_runtime_reveal_never_guards_regardless_of_ratio() {
        assert!(!retained_replay_would_vacuum_richer_client(1801, 32, false));
        assert!(!retained_replay_would_vacuum_richer_client(10_000, 0, false));
        assert!(!retained_replay_would_vacuum_richer_client(
            RICH_CLIENT_MIN_NONBLANK,
            1,
            false
        ));
    }

    // XTERM-BUG: blank-viewport-client-snapshot-poison — a collapsed-scrollback
    // recovery reveal (the live switch-back/re-reveal case) OR any codex-like session
    // must allow the daemon screen-snapshot fallback so the client reconcile-from-daemon
    // path engages; a plain initial-read of a non-codex session must NOT.
    #[test]
    fn collapsed_recovery_or_codex_allows_daemon_screen_fallback() {
        assert!(retained_rehydrate_allow_screen_fallback(
            RetainedRehydrateMode::CollapsedScrollbackRecovery,
            false
        ));
        assert!(retained_rehydrate_allow_screen_fallback(
            RetainedRehydrateMode::InitialRead,
            true
        ));
        assert!(retained_rehydrate_allow_screen_fallback(
            RetainedRehydrateMode::CollapsedScrollbackRecovery,
            true
        ));
        assert!(!retained_rehydrate_allow_screen_fallback(
            RetainedRehydrateMode::InitialRead,
            false
        ));
    }

    #[test]
    fn retained_ready_remote_host_reuses_bootstrap_on_focus_return() {
        assert!(retained_ready_remote_host_should_reuse_bootstrap(
            true, true, true, true
        ));
        assert!(!retained_ready_remote_host_should_reuse_bootstrap(
            false, true, true, true
        ));
        assert!(!retained_ready_remote_host_should_reuse_bootstrap(
            true, false, true, true
        ));
        assert!(!retained_ready_remote_host_should_reuse_bootstrap(
            true, true, false, true
        ));
        assert!(!retained_ready_remote_host_should_reuse_bootstrap(
            true, true, true, false
        ));
    }

    // §7.3 generalization: a retained, ever-ready, daemon-owned, fault-free
    // host pins the bootstrap epoch REGARDLESS of kind or locality — this is
    // the predicate that stops cc/local sessions re-bootstrapping (the felt
    // zoom) on every open request while remote-codex sat stable.
    #[test]
    fn retained_ever_ready_host_pins_epoch_kind_and_locality_agnostic() {
        assert!(retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, true, true, false, true, false, false
        ));
    }

    // Every unpin direction re-bootstraps exactly as before: not retained, never
    // ready (a session being born must keep the per-request epoch), a runtime the
    // daemon lost, a latched fault, no host, and a failed/timed-out resume overlay.
    #[test]
    fn stable_epoch_pin_releases_on_every_fault_direction() {
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            false, true, true, false, true, false, false
        ));
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, false, true, false, true, false, false
        ));
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, true, false, false, true, false, false
        ));
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, true, true, true, true, false, false
        ));
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, true, true, false, false, false, false
        ));
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, true, true, false, true, true, false
        ));
        assert!(!retained_ever_ready_host_should_pin_bootstrap_epoch(
            true, true, true, false, true, false, true
        ));
    }

    #[test]
    fn retained_remote_host_rehydrates_fault_recovery_without_ready_path() {
        assert!(retained_remote_host_should_rehydrate(
            true, true, false, true, true
        ));
        assert!(retained_remote_host_should_rehydrate(
            true, true, true, false, true
        ));
        assert!(!retained_remote_host_should_rehydrate(
            true, false, false, true, true
        ));
        assert!(!retained_remote_host_should_rehydrate(
            true, true, false, true, false
        ));
        assert!(!retained_remote_host_should_rehydrate(
            false, true, false, true, true
        ));
    }

    #[test]
    fn retained_ready_remote_rehydrate_skips_ready_attempt() {
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, true, false, false, None, false),
            None
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, false, true, false, None, false),
            None
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, false, false, false, None, false),
            Some(RetainedRehydrateMode::InitialRead)
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(false, true, false, false, false, None, false),
            None
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, false, false, false, false, None, false),
            None
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, false, false, true, None, false),
            None
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(
                true,
                true,
                false,
                false,
                false,
                Some("active terminal host exists but xterm surface is empty"),
                false,
            ),
            Some(RetainedRehydrateMode::CollapsedScrollbackRecovery)
        );
    }

    // TODO-1 (campaign): a SETTLED-IDLE codex reveal reconciles-from-daemon ONCE even
    // when live-connected (the reveal-shadow fix), but a WORKING/non-idle codex reveal
    // must NEVER reconcile (recovery-churn trap that broke live sessions). The caller
    // passes codex_idle_reveal_reconcile=false whenever the surface isn't settled-idle.
    #[test]
    fn settled_idle_codex_reveal_reconciles_once_but_working_never_churns() {
        // Connected + ready_history + no problem: old behavior is None (no reconcile);
        // a settled-idle codex reveal flips it to a one-shot InitialRead reconcile.
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, false, true, true, None, false),
            None,
            "non-codex connected reveal must NOT reconcile (recovery-churn trap)"
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, false, true, true, None, true),
            Some(RetainedRehydrateMode::InitialRead),
            "settled-idle codex reveal reconciles once to repaint the authoritative bottom"
        );
        // The idle flag never overrides the hard gates: a ready_attempt in flight, a
        // non-active host, or a non-retained host still yields None even for codex.
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, true, true, true, None, true),
            None,
            "a ready attempt in flight must still suppress reconcile (no churn mid-attempt)"
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(false, true, false, true, true, None, true),
            None,
            "a non-retained host never reconciles regardless of codex idle"
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, false, false, true, true, None, true),
            None,
            "a non-active host never reconciles regardless of codex idle"
        );
        // Disconnected codex idle reveal behaves like the existing initial-read path.
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(true, true, false, false, false, None, true),
            Some(RetainedRehydrateMode::InitialRead)
        );
    }

    #[test]
    fn retained_ready_remote_rehydrate_retries_collapsed_scrollback_after_ready_history() {
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(
                true,
                true,
                false,
                true,
                false,
                Some("active terminal host is only showing a plain shell prompt"),
                false,
            ),
            Some(RetainedRehydrateMode::CollapsedScrollbackRecovery)
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(
                true,
                true,
                false,
                true,
                false,
                Some("active remote Codex prompt surface has no current input row"),
                false,
            ),
            Some(RetainedRehydrateMode::CollapsedScrollbackRecovery)
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(
                true,
                true,
                false,
                true,
                false,
                Some("active remote terminal lost expected scrollback after retained replay"),
                false,
            ),
            Some(RetainedRehydrateMode::CollapsedScrollbackRecovery)
        );
        assert_eq!(
            retained_ready_remote_host_rehydrate_mode(
                true,
                true,
                false,
                true,
                false,
                Some("active remote terminal is waiting for resume overlay"),
                false,
            ),
            None
        );
    }

    #[test]
    fn retained_rehydrate_identity_includes_mode() {
        let session_path = "remote-session://dev/019dbdc7";
        let key = retained_rehydrate_identity_key(
            session_path,
            "remote-session://dev/019dbdc7:7",
            true,
            true,
            Some(RetainedRehydrateMode::InitialRead),
        );
        let recovery_key = retained_rehydrate_identity_key(
            session_path,
            "remote-session://dev/019dbdc7:7",
            true,
            true,
            Some(RetainedRehydrateMode::CollapsedScrollbackRecovery),
        );
        let next_mount = retained_rehydrate_identity_key(
            session_path,
            "remote-session://dev/019dbdc7:8",
            true,
            true,
            Some(RetainedRehydrateMode::InitialRead),
        );
        assert_ne!(key, recovery_key);
        assert_ne!(key, next_mount);
        assert!(recovery_key.contains(":collapsed-scrollback-recovery"));
        assert!(!key.contains(":open-row-"));
    }

    #[test]
    fn daemon_retained_snapshot_replay_starts_only_after_ready_active_remote_session() {
        assert!(daemon_retained_snapshot_replay_should_start(
            true, false, false, true, true, true, false, false
        ));
        assert!(daemon_retained_snapshot_replay_should_start(
            true, false, true, true, true, true, false, false
        ));
        assert!(daemon_retained_snapshot_replay_should_start(
            true, true, true, true, true, true, false, false
        ));
        assert!(!daemon_retained_snapshot_replay_should_start(
            false, false, false, true, true, true, false, false
        ));
        assert!(!daemon_retained_snapshot_replay_should_start(
            true, false, false, false, true, true, false, false
        ));
        assert!(!daemon_retained_snapshot_replay_should_start(
            true, false, false, true, false, true, false, false
        ));
        assert!(!daemon_retained_snapshot_replay_should_start(
            true, false, false, true, true, false, false, false
        ));
        assert!(!daemon_retained_snapshot_replay_should_start(
            true, false, false, true, true, true, true, false
        ));
        assert!(!daemon_retained_snapshot_replay_should_start(
            true, false, false, true, true, true, false, true
        ));
        let key = daemon_retained_snapshot_replay_identity_key(
            "remote-session://dev/019dbdcf",
            "remote-session://dev/019dbdcf:7",
            true,
            true,
            true,
        );
        assert!(key.starts_with("daemon-retained-replay:remote-session://dev/019dbdcf"));
        assert!(key.contains(":true:true:true"));
    }

    #[test]
    fn blank_host_snapshot_replay_waits_after_retained_surface_connects() {
        assert!(blank_host_snapshot_replay_should_start(
            true, true, true, false, "", "", true, false, 0, 2,
        ));
        assert!(!blank_host_snapshot_replay_should_start(
            true, true, true, false, "", "", true, true, 0, 2,
        ));
        assert!(!blank_host_snapshot_replay_should_start(
            true,
            true,
            true,
            false,
            "› current prompt",
            "",
            true,
            false,
            0,
            2,
        ));
    }

    #[test]
    fn blank_host_snapshot_replay_from_read_waits_after_retained_surface_connects() {
        assert!(blank_host_snapshot_replay_from_read_should_start(
            true, true, true, true, true, true, false, false, 0, 2,
        ));
        assert!(!blank_host_snapshot_replay_from_read_should_start(
            true, true, true, true, true, true, false, true, 0, 2,
        ));
        assert!(!blank_host_snapshot_replay_from_read_should_start(
            true, true, true, true, true, true, true, false, 0, 2,
        ));
    }
}
