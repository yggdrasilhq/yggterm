// Consolidated scroll-mode controller — the single owner of "where should the
// viewport be" (campaign TODO-1+TODO-3, see [[audit-viewport-scroll-control-flow]]).
//
// WHY THIS EXISTS: the viewport/scroll class (blink-shadow, broken-bottom,
// jump-to-30%, lock-at-top) was a tangle of two IMPLICIT states
// (`scrollbackIntent` = PromptFollow|UserScrollback) plus a thicket of time
// guards (promptFollowLayoutGuard, suppressProgrammaticScrollIntent, ...). The
// live captures proved the failure: the follow lands at a STALE baseY (mid-load
// / alt-screen) and never re-asserts after content settles, across MULTIPLE
// trigger paths (retained-replay, fit-cascade, alt->normal), so the viewport is
// stranded above/below the true bottom until some unrelated event catches up —
// the transient the user sees. Piecemeal patches at the movers regressed before
// ("guard forceXtermViewportY -> top-jump"). This module makes the DECISION
// explicit and testable; the JS keeps the unguarded low-level applier
// (forceXtermViewportY) and only consults the mode + these functions.
//
// THE RULE: follow EXECUTES only in Following. Pinned/Selecting never auto-move.
// Transitions are NOT suppressed by output activity (that suppression was the
// "can't scroll away during output" bug). On a content/layout SETTLE, a Following
// session re-asserts to the CURRENT baseY (this is the fix for the stranded
// viewport — it covers every trigger path because it keys off settle, not the
// trigger). forceXtermViewportY stays unguarded; the decision lives here.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScrollMode {
    /// Pinned to the live bottom; output + settle re-assert to baseY.
    Following,
    /// User scrolled up past the threshold; sticky until they return to bottom.
    /// Output and settle must NOT move the viewport.
    Pinned,
    /// A non-empty selection is active; the viewport must not move (so a drag /
    /// copy is never yanked). Restores the prior mode when the selection clears.
    Selecting,
}

impl ScrollMode {
    pub(crate) fn as_key(self) -> &'static str {
        match self {
            Self::Following => "following",
            Self::Pinned => "pinned",
            Self::Selecting => "selecting",
        }
    }
}

/// Events that can change the scroll mode. Deliberately explicit so every
/// transition is enumerable + testable (vs the old implicit guard soup).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScrollEvent {
    /// User scrolled up and is now `lines_from_bottom` above the live bottom.
    UserScrolledUp { lines_from_bottom: u32 },
    /// The viewport reached (or returned to) the live bottom.
    ReachedBottom,
    /// A non-empty selection became active.
    SelectionStarted,
    /// The selection was cleared (empty).
    SelectionCleared,
    /// The user typed at the prompt (keystroke / paste / submit).
    TypedAtPrompt,
    /// The session was switched to / revealed / freshly mounted.
    SwitchedOrRevealed,
}

/// How far above the bottom the user must scroll before we treat it as an
/// intentional Pinned scrollback (below this is jitter / momentum and stays
/// Following). Mirrors the old keepVisibleMargin intent but as one explicit knob.
pub(crate) const PIN_THRESHOLD_LINES: u32 = 2;

/// The single transition function: given the current mode + the prior mode
/// (needed to restore after a selection clears) + an event, return the next
/// (mode, prior_to_remember). `prior` is the mode to restore to when a Selecting
/// state ends; callers persist it alongside the mode.
pub(crate) fn next_scroll_mode(
    current: ScrollMode,
    prior_before_selecting: ScrollMode,
    event: ScrollEvent,
) -> (ScrollMode, ScrollMode) {
    match event {
        // Selection is the highest-priority pin: never move during a drag/copy.
        // Remember what we were doing so we can restore it on clear.
        ScrollEvent::SelectionStarted => {
            let prior = if current == ScrollMode::Selecting {
                prior_before_selecting
            } else {
                current
            };
            (ScrollMode::Selecting, prior)
        }
        // Selection cleared: restore the mode we were in before selecting.
        ScrollEvent::SelectionCleared => {
            if current == ScrollMode::Selecting {
                (prior_before_selecting, prior_before_selecting)
            } else {
                (current, prior_before_selecting)
            }
        }
        // A real scroll-up past the threshold pins (sticky). Within the threshold
        // we stay Following (jitter/momentum). Never overrides an active selection.
        ScrollEvent::UserScrolledUp { lines_from_bottom } => {
            if current == ScrollMode::Selecting {
                (current, prior_before_selecting)
            } else if lines_from_bottom >= PIN_THRESHOLD_LINES {
                (ScrollMode::Pinned, prior_before_selecting)
            } else {
                (ScrollMode::Following, prior_before_selecting)
            }
        }
        // Reaching the bottom always returns to Following (unless selecting —
        // a selection at the bottom must still not auto-follow).
        ScrollEvent::ReachedBottom => {
            if current == ScrollMode::Selecting {
                (current, prior_before_selecting)
            } else {
                (ScrollMode::Following, prior_before_selecting)
            }
        }
        // Typing at the prompt re-engages Following ONLY if already Following
        // (the genuine product choice from the audit: do NOT yank a user who is
        // reading scrollback / has scrolled up just because they typed). If they
        // were Selecting, keep selecting (their drag may be in progress).
        ScrollEvent::TypedAtPrompt => match current {
            ScrollMode::Following => (ScrollMode::Following, prior_before_selecting),
            ScrollMode::Pinned => (ScrollMode::Pinned, prior_before_selecting),
            ScrollMode::Selecting => (ScrollMode::Selecting, prior_before_selecting),
        },
        // A switch/reveal/mount always starts Following (the user wants the live
        // bottom of the session they just opened). Clears any stale Pinned/Selecting.
        ScrollEvent::SwitchedOrRevealed => (ScrollMode::Following, ScrollMode::Following),
    }
}

/// Should the follow executor run right now (scroll to baseY)? ONLY in Following.
/// This is the guard that replaces the scattered `scrollbackIntent !== 'UserScrollback'`
/// checks — one predicate, one owner.
pub(crate) fn should_follow_now(mode: ScrollMode) -> bool {
    matches!(mode, ScrollMode::Following)
}

/// Detect a genuine USER scroll-up from a viewport-position sample, the trigger
/// that should emit `ScrollEvent::UserScrolledUp`. Harness-locked invariant
/// (tools/xterm-harness/scroll_follow_probe.test.js): writing output NEVER
/// decreases the xterm viewport ydisp — at the bottom it auto-follows
/// (ydisp increases), when scrolled up it leaves ydisp unchanged while baseY
/// grows. The ONLY thing that decreases ydisp is a real user scroll-up (any
/// gesture: wheel, scrollbar drag, PageUp, touch). So `cur < prev` uniquely
/// identifies a user scroll-up — EXCEPT when WE moved the viewport
/// programmatically (forceXtermViewportY), which must be excluded via the
/// `programmatic` flag. This is why a passive burst-strand (viewport below base
/// with ydisp UNCHANGED) does NOT flip to Pinned and is re-followed instead.
pub(crate) fn user_scroll_up_detected(prev_ydisp: i64, cur_ydisp: i64, programmatic: bool) -> bool {
    !programmatic && cur_ydisp < prev_ydisp
}

/// THE core fix: when content/layout SETTLES (replay finished, output burst
/// ended, alt->normal buffer switch, fit completed), a Following session must
/// re-assert to the CURRENT baseY — because the earlier follow may have landed at
/// a stale baseY while content was still arriving. This keys off the settle, not
/// the trigger, so it covers ALL paths (retained-replay, fit-cascade, alt->normal)
/// uniformly. Returns true iff we should re-assert the follow on settle.
pub(crate) fn should_settle_follow(mode: ScrollMode, viewport_is_below_base: bool) -> bool {
    matches!(mode, ScrollMode::Following) && viewport_is_below_base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_or_reveal_always_starts_following_and_clears_stale_state() {
        for current in [ScrollMode::Following, ScrollMode::Pinned, ScrollMode::Selecting] {
            let (mode, prior) =
                next_scroll_mode(current, ScrollMode::Pinned, ScrollEvent::SwitchedOrRevealed);
            assert_eq!(mode, ScrollMode::Following, "switch/reveal must follow");
            assert_eq!(prior, ScrollMode::Following, "switch/reveal clears stale prior");
        }
    }

    #[test]
    fn scroll_up_past_threshold_pins_within_threshold_follows() {
        let (mode, _) = next_scroll_mode(
            ScrollMode::Following,
            ScrollMode::Following,
            ScrollEvent::UserScrolledUp { lines_from_bottom: PIN_THRESHOLD_LINES },
        );
        assert_eq!(mode, ScrollMode::Pinned);
        let (mode, _) = next_scroll_mode(
            ScrollMode::Following,
            ScrollMode::Following,
            ScrollEvent::UserScrolledUp { lines_from_bottom: 1 },
        );
        assert_eq!(mode, ScrollMode::Following, "jitter within threshold stays following");
    }

    #[test]
    fn reaching_bottom_returns_to_following() {
        let (mode, _) =
            next_scroll_mode(ScrollMode::Pinned, ScrollMode::Following, ScrollEvent::ReachedBottom);
        assert_eq!(mode, ScrollMode::Following);
    }

    #[test]
    fn selection_pins_and_restores_prior_mode() {
        // From Pinned -> Selecting remembers Pinned, restores it on clear.
        let (mode, prior) =
            next_scroll_mode(ScrollMode::Pinned, ScrollMode::Following, ScrollEvent::SelectionStarted);
        assert_eq!(mode, ScrollMode::Selecting);
        assert_eq!(prior, ScrollMode::Pinned);
        let (mode2, _) = next_scroll_mode(ScrollMode::Selecting, prior, ScrollEvent::SelectionCleared);
        assert_eq!(mode2, ScrollMode::Pinned, "selection clear restores the prior (Pinned)");

        // From Following -> Selecting -> clear restores Following.
        let (mode, prior) = next_scroll_mode(
            ScrollMode::Following,
            ScrollMode::Following,
            ScrollEvent::SelectionStarted,
        );
        assert_eq!(mode, ScrollMode::Selecting);
        assert_eq!(prior, ScrollMode::Following);
        let (mode2, _) = next_scroll_mode(ScrollMode::Selecting, prior, ScrollEvent::SelectionCleared);
        assert_eq!(mode2, ScrollMode::Following);
    }

    #[test]
    fn output_or_scroll_never_moves_during_selection() {
        // A selection must survive a scroll-up event (no auto-pin/move) and the
        // prior is preserved.
        let (mode, prior) = next_scroll_mode(
            ScrollMode::Selecting,
            ScrollMode::Following,
            ScrollEvent::UserScrolledUp { lines_from_bottom: 50 },
        );
        assert_eq!(mode, ScrollMode::Selecting);
        assert_eq!(prior, ScrollMode::Following);
        // ReachedBottom also must not break a selection.
        let (mode, _) =
            next_scroll_mode(ScrollMode::Selecting, ScrollMode::Following, ScrollEvent::ReachedBottom);
        assert_eq!(mode, ScrollMode::Selecting);
    }

    #[test]
    fn typing_at_prompt_does_not_yank_a_pinned_user() {
        // The genuine product choice: typing while Pinned (reading scrollback)
        // must NOT snap to bottom.
        let (mode, _) =
            next_scroll_mode(ScrollMode::Pinned, ScrollMode::Following, ScrollEvent::TypedAtPrompt);
        assert_eq!(mode, ScrollMode::Pinned);
        // Typing while Following stays Following.
        let (mode, _) = next_scroll_mode(
            ScrollMode::Following,
            ScrollMode::Following,
            ScrollEvent::TypedAtPrompt,
        );
        assert_eq!(mode, ScrollMode::Following);
    }

    #[test]
    fn follow_executes_only_in_following() {
        assert!(should_follow_now(ScrollMode::Following));
        assert!(!should_follow_now(ScrollMode::Pinned));
        assert!(!should_follow_now(ScrollMode::Selecting));
    }

    #[test]
    fn user_scroll_up_detected_only_on_nonprogrammatic_decrease() {
        // A real user scroll-up: ydisp decreased, not programmatic.
        assert!(user_scroll_up_detected(100, 80, false));
        // Programmatic move-up (forceXtermViewportY): excluded.
        assert!(!user_scroll_up_detected(100, 80, true));
        // Passive burst-strand: ydisp UNCHANGED while baseY grows -> NOT a scroll-up.
        assert!(!user_scroll_up_detected(80, 80, false));
        // Auto-follow on output: ydisp increased -> NOT a scroll-up.
        assert!(!user_scroll_up_detected(80, 100, false));
    }

    #[test]
    fn settle_follow_reasserts_only_when_following_and_stranded() {
        // The captured bug: Following but viewport stranded below base (lock-at-top
        // / broken-bottom) -> re-assert.
        assert!(should_settle_follow(ScrollMode::Following, true));
        // Already at bottom -> no-op.
        assert!(!should_settle_follow(ScrollMode::Following, false));
        // Pinned / Selecting -> never auto-move on settle (don't yank the user).
        assert!(!should_settle_follow(ScrollMode::Pinned, true));
        assert!(!should_settle_follow(ScrollMode::Selecting, true));
    }
}
