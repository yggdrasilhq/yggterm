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
pub(crate) fn retained_ready_remote_host_should_reuse_bootstrap(
    is_remote_resume_session: bool,
    retained: bool,
    resume_ready: bool,
    host_id_present: bool,
) -> bool {
    is_remote_resume_session && retained && resume_ready && host_id_present
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
