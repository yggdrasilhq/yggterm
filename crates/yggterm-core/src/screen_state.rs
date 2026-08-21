//! What a row's SCREEN says its state is — and, for each state, what a caller
//! may do about it.
//!
//! # Why this exists
//!
//! Every instrument a spawner has is DOWNSTREAM OF A WRITE. `submit` reports
//! that bytes were accepted; `input-check` reports that a write was echoed; a
//! transcript reports that text reached a file. A pty accepts bytes whether or
//! not anything is consuming them, so a brief that is QUEUED behind a modal and
//! a brief that was DELIVERED are byte-identical from the writer's side, and
//! all three instruments answer yes for both. The screen is the only surface on
//! which the two differ.
//!
//! Measured 2026-08-21 on a row spawned into a directory its CLI had not seen:
//! `submitted:true` (82 bytes), `consuming_input:true`, and the transcript file
//! did not exist for a further **14.5 seconds** — so the recipe's own
//! last-resort check ("the only step that cannot lie") returns a false negative
//! across the whole window in which a spawner is deciding whether its delegate
//! is alive.
//!
//! # The pairing rule
//!
//! ⛔ A classifier that only FIRES is half a classifier. The failure this fleet
//! has paid for most is typing into a row that was not ready for it, so every
//! state here carries its REMEDY and its PROHIBITION as data, next to the
//! predicate that detects it. A caller that can name a state can therefore also
//! name what it must not do, without consulting a document that may have
//! drifted.
//!
//! # What is deliberately NOT here
//!
//! A row whose turn was CUT MID-FLIGHT — a daemon swap re-resumed it on a fresh
//! pty, the process is alive and at rest, and nothing will ever finish the turn
//! — has no screen signature at all. Its tell is the ABSENCE of progress beside
//! a live process, which is a question about time and cannot be answered from a
//! single frame. It belongs to the watcher that samples repeatedly, not here.

use crate::agent_cli::AGENT_CLIS;

/// What a row's screen says it is doing right now.
///
/// ⛔ ORDER IS MEANING. [`classify_screen`] returns the FIRST variant whose
/// predicate holds, and the order below is not cosmetic — several of these
/// states are true at the same time and reporting the wrong one is what gets a
/// person typed over. A question picker is mid-turn, so `working` is also true
/// while it is up; a plan-limit dialog is a picker whose options spend money.
/// In each pair the more specific and more dangerous state must win.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowScreenState {
    /// The screen could not be read at all — an unowned key, a runtime that has
    /// gone. ⛔ NOT the same as a blank screen, and never to be collapsed with
    /// it: "I could not look" and "I looked and it was empty" license opposite
    /// actions.
    Unreadable,
    /// A first-run modal stands before the composer exists.
    StartupGate,
    /// A limit/billing dialog whose options are not equivalent: the highlight
    /// may sit on an option that spends money.
    PlanLimitChoice,
    /// An owner-facing question picker. The CLI is mid-turn and reading
    /// navigation keys only, so typed sentences vanish without a trace.
    QuestionPicker,
    /// The CLI is parked on a usage limit with its own auto-continue armed.
    LimitWait,
    /// A turn is in flight.
    Working,
    /// Nothing is holding the row: it is at its composer, reading input.
    Ready,
}

impl RowScreenState {
    /// Stable machine-readable token. This is what scripts branch on, so it is
    /// part of the contract and may not be reworded for taste.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::StartupGate => "startup_gate",
            Self::PlanLimitChoice => "plan_limit_choice",
            Self::QuestionPicker => "question_picker",
            Self::LimitWait => "limit_wait",
            Self::Working => "working",
            Self::Ready => "ready",
        }
    }

    /// Whether a caller may WRITE to this row without further thought.
    ///
    /// ⛔ False for every state except [`Self::Ready`], including
    /// [`Self::Working`]: bytes sent to a working row are accepted by the pty
    /// and land in the composer of whatever turn comes next.
    pub fn may_type(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Whether a person, rather than a machine, has to clear this.
    pub fn needs_a_human(self) -> bool {
        matches!(self, Self::PlanLimitChoice | Self::QuestionPicker)
    }

    /// What to DO about it.
    pub fn remedy(self) -> &'static str {
        match self {
            Self::Unreadable => {
                "ask the daemon that owns the row; if none does, the row is gone"
            }
            Self::StartupGate => {
                "read WHICH option the marker sits on, then send a lone carriage \
                 return with `terminal send` (not `submit`), then READ THE SCREEN \
                 BACK to confirm the gate is gone"
            }
            Self::PlanLimitChoice => "escalate to the owner and leave the screen alone",
            Self::QuestionPicker => "notify the owner — this row is asking a question and is waiting for an answer",
            Self::LimitWait => "wait — the CLI's own auto-continue resumes the turn",
            Self::Working => "wait, or re-read the screen later",
            Self::Ready => "write to it",
        }
    }

    /// What must NOT be done about it. Kept beside the remedy because the two
    /// have been got the wrong way round, and pressing Enter at the wrong modal
    /// is the expensive direction.
    pub fn prohibition(self) -> &'static str {
        match self {
            Self::Unreadable => {
                "⛔ do not treat an unreadable screen as an idle one — refuse on doubt"
            }
            Self::StartupGate => {
                "⛔ never press blind: the same shape carries dialogs whose options \
                 spend money, so confirm the highlighted option first"
            }
            Self::PlanLimitChoice => {
                "⛔⛔ NEVER PRESS ENTER — a bare carriage return SELECTS the \
                 highlighted option, and the options here are not equivalent"
            }
            Self::QuestionPicker => {
                "⛔ type nothing: the widget reads single keys, so a typed sentence \
                 is swallowed and appears nowhere"
            }
            Self::LimitWait => {
                "⛔ do not type: the turn resumes by itself and your bytes would land \
                 in the composer of a turn that is about to continue"
            }
            Self::Working => "⛔ do not type: bytes are queued into the NEXT turn",
            Self::Ready => "—",
        }
    }
}

/// Every state this crate can name, in the precedence [`classify_screen`] uses.
///
/// ⭐ EXISTS TO BE PRINTED. A classifier set that only a reader of this file can
/// enumerate is one an agent will re-derive from a screenshot at three in the
/// morning, and the re-derivation is where the prohibitions get lost. `server
/// screen --states` renders this, so the remedies and the things one must never
/// do are available to whoever is about to act.
pub const ALL_ROW_SCREEN_STATES: &[RowScreenState] = &[
    RowScreenState::Unreadable,
    RowScreenState::StartupGate,
    RowScreenState::PlanLimitChoice,
    RowScreenState::QuestionPicker,
    RowScreenState::LimitWait,
    RowScreenState::Working,
    RowScreenState::Ready,
];

/// Whether a visible row carries a SELECTION MARKER sitting on a NUMBERED
/// option — the structural signature of a modal that Enter would answer.
///
/// ⛔ THIS IS THE TEST THAT NEEDS A RENDERED GRID, and it is why the plain-rows
/// reading exists. A modal is drawn with absolute cursor moves, so on the raw
/// byte stream the marker and its option land in the same enormous
/// newline-delimited run as the rest of the screen and "on the same row" cannot
/// be asked at all. Given real rows it is a one-line test.
pub fn screen_has_selected_numbered_option(sample: &str, marker: char) -> bool {
    sample.lines().any(|line| {
        let Some(after) = line.split_once(marker).map(|(_, tail)| tail) else {
            return false;
        };
        let after = after.trim_start();
        let mut chars = after.chars();
        // `❯ 1. Yes, …` — a digit, then a `.` or `)` close behind it.
        matches!(chars.next(), Some(d) if d.is_ascii_digit())
            && matches!(chars.next(), Some('.') | Some(')'))
    })
}

/// Read one rendered screen and name the state it is in.
///
/// `screen` must be the RENDERED GRID (one entry per visible row), not the raw
/// pty byte stream — see [`screen_has_selected_numbered_option`]. `None` means
/// the screen could not be read, which is [`RowScreenState::Unreadable`] and is
/// deliberately distinct from `Some("")`.
///
/// Kind-agnostic, like every other screen predicate in this crate: the reader
/// usually holds a runtime key rather than a CLI kind, and a phrase measured on
/// one CLI is evidence about that CLI only — an unmeasured CLI contributes no
/// phrases and therefore cannot arm a state by accident.
pub fn classify_screen(screen: Option<&str>) -> RowScreenState {
    let Some(screen) = screen else {
        return RowScreenState::Unreadable;
    };
    // A gate stands before the composer exists, so it outranks every state that
    // presumes one.
    if crate::screen_text_shows_agent_startup_gate(screen) {
        return RowScreenState::StartupGate;
    }
    // ⛔ BEFORE the picker and before the limit wait. A limit/billing dialog IS
    // a picker, and its phrases overlap the limit-wait footer's; the structural
    // test is what separates a dialog awaiting a keypress from a footer that is
    // merely reporting. Refuse-on-doubt lands here on purpose: mistaking a
    // dialog for a wait costs a delay, mistaking a wait for a dialog costs
    // nothing but a notification.
    if AGENT_CLIS.iter().any(|descriptor| {
        descriptor.screen_shows_plan_limit_choice(screen)
            && screen_has_selected_numbered_option(screen, descriptor.composer_marker)
    }) {
        return RowScreenState::PlanLimitChoice;
    }
    // ⛔ BEFORE working: a picker is mid-turn, so `working` is true while it is
    // up, and reporting "busy working" for a row that is asking the owner a
    // question is how a 27-minute wait got misdescribed.
    if crate::screen_text_shows_agent_question_picker(screen) {
        return RowScreenState::QuestionPicker;
    }
    if crate::screen_text_shows_agent_limit_wait(screen) {
        return RowScreenState::LimitWait;
    }
    if crate::screen_text_shows_agent_working(screen) {
        return RowScreenState::Working;
    }
    RowScreenState::Ready
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screen that started all of this: captured 2026-08-21 from a row
    /// spawned into a directory the CLI had not seen. Rendered to a grid, blank
    /// rows and all, because the blank rows are half the bug.
    fn trust_gate_grid() -> String {
        let mut rows = vec![
            "",
            "  Accessing workspace: /tmp/example-checkout",
            "",
            "  Quick safety check: Is this a project you created or one you trust?",
            "  (Like your own code, a well-known opensource project, or work from",
            "  your team). If not, take a moment to review what's in this folder first.",
            "",
            "  ❯ 1. Yes, I trust this folder",
            "    2. No, exit",
            "",
            "  Enter to confirm · Esc to cancel",
        ]
        .join("\n");
        // The rows a real 65-row screen carries beneath a modal, which is what
        // the old `.rev().take(10)` window filled itself with.
        rows.push_str(&"\n".repeat(30));
        rows
    }

    #[test]
    fn the_startup_gate_is_named_even_with_thirty_blank_rows_beneath_it() {
        assert_eq!(
            classify_screen(Some(&trust_gate_grid())),
            RowScreenState::StartupGate,
        );
    }

    /// ⛔ THE REGRESSION THIS FILE EXISTS FOR. `sample.lines().rev().take(n)`
    /// takes n LINES and discards the blank ones afterwards, so a modal with
    /// blank rows beneath it is invisible to it. Every classifier read false on
    /// the live gate. Pinning it by construction: the phrase must still be
    /// found when it sits further from the bottom than the window is deep.
    #[test]
    fn a_modal_is_still_found_when_blank_rows_push_it_out_of_a_naive_window() {
        let grid = trust_gate_grid();
        let naive_window: Vec<&str> = grid.lines().rev().take(10).collect();
        assert!(
            !naive_window
                .iter()
                .any(|line| line.to_ascii_lowercase().contains("trust this folder")),
            "the naive window must genuinely miss it, or this test proves nothing",
        );
        assert!(crate::screen_text_shows_agent_startup_gate(&grid));
    }

    #[test]
    fn a_gate_forbids_typing_and_says_so() {
        let state = classify_screen(Some(&trust_gate_grid()));
        assert!(!state.may_type());
        assert!(state.remedy().contains("terminal send"));
        assert!(state.prohibition().contains("never press blind"));
    }

    #[test]
    fn an_unreadable_screen_is_not_an_idle_one() {
        let state = classify_screen(None);
        assert_eq!(state, RowScreenState::Unreadable);
        assert!(!state.may_type());
    }

    /// A composer with nothing holding it is the only state that licenses a
    /// write, and it must not be confused with a blank unreadable screen.
    #[test]
    fn an_empty_but_readable_screen_is_ready() {
        let state = classify_screen(Some(""));
        assert_eq!(state, RowScreenState::Ready);
        assert!(state.may_type());
    }

    #[test]
    fn the_selection_marker_test_wants_a_numbered_option_not_just_the_glyph() {
        // A composer prompt carries the same glyph and is NOT a modal.
        assert!(!screen_has_selected_numbered_option("❯ write me a haiku", '❯'));
        assert!(screen_has_selected_numbered_option("  ❯ 2. Use API billing", '❯'));
        assert!(screen_has_selected_numbered_option("❯ 1) Yes", '❯'));
    }

    /// ⛔ A billing dialog must outrank the limit-wait footer, because their
    /// phrases overlap and only one of them is safe to leave alone.
    #[test]
    fn a_billing_dialog_outranks_the_limit_wait_footer() {
        let dialog = [
            "  You have hit your session limit.",
            "",
            "  \u{276f} 1. Stop and wait",
            "    2. Switch to a team account",
            "    3. Use API billing",
            "",
            "  Enter to confirm",
        ]
        .join("\n");
        let dialog = dialog.as_str();
        let state = classify_screen(Some(dialog));
        assert_eq!(state, RowScreenState::PlanLimitChoice);
        assert!(state.needs_a_human());
        assert!(state.prohibition().contains("NEVER PRESS ENTER"));
    }

    /// The same words WITHOUT a selectable option are the auto-continuing
    /// footer, which needs patience and not a person.
    #[test]
    fn the_limit_wait_footer_without_an_option_is_not_a_dialog() {
        let footer = "  Usage limit reached · continuing shortly · esc to cancel";
        let state = classify_screen(Some(footer));
        assert_eq!(state, RowScreenState::LimitWait);
        assert!(!state.needs_a_human());
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    /// ⛔ A new state that is not in the printed table is a state the fleet
    /// cannot learn about. The count is asserted deliberately: adding a variant
    /// should FAIL here and make the author put it in the listing, rather than
    /// shipping a classifier nobody can discover.
    #[test]
    fn every_state_is_listed_with_a_remedy_and_a_prohibition() {
        assert_eq!(
            ALL_ROW_SCREEN_STATES.len(),
            7,
            "a state was added or removed — put it in ALL_ROW_SCREEN_STATES too",
        );
        for state in ALL_ROW_SCREEN_STATES {
            assert!(!state.slug().is_empty());
            assert!(!state.remedy().is_empty(), "{} has no remedy", state.slug());
            assert!(
                !state.prohibition().is_empty(),
                "{} has no prohibition",
                state.slug(),
            );
        }
        // Only one state licenses a write, and it is not `working`.
        let writable: Vec<&str> = ALL_ROW_SCREEN_STATES
            .iter()
            .filter(|state| state.may_type())
            .map(|state| state.slug())
            .collect();
        assert_eq!(writable, vec!["ready"]);
    }
}
