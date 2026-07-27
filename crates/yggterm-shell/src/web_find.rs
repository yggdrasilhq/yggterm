//! Find-in-page: the single owner of what a find request MEANS.
//!
//! Three surfaces ask the same question — the Ctrl+F bar over a web viewport,
//! the `server app web find` verb an agent drives, and the engine bridge in
//! `vendor/dioxus-desktop/src/web_surface.rs` — so the answer lives in exactly
//! one place. This module owns:
//!
//! * the **engine option mask** (case-insensitive, wrap-around) and the match
//!   **cap**, both of which decide whether a reported count is the TRUTH;
//! * the **position cycle** (`3/17` -> `4/17` -> ... -> `1/17`), because
//!   WebKit's find controller does not expose which match is selected — nobody
//!   does, so somebody has to count, and it is here;
//! * the **keyboard-ownership contract**: which keys the bar claims, and the
//!   promise that it BORROWS focus and gives it back to whoever held it.
//!
//! Nothing here talks to the engine. The engine call site takes the mask, the
//! cap and the step from here and returns the engine's number verbatim — so a
//! wrong count is always a bug in one of the two, never a disagreement between
//! two copies of the same rule.

use std::cell::RefCell;
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// Engine options — the numbers WebKit is handed
// ---------------------------------------------------------------------------

/// `WEBKIT_FIND_OPTIONS_CASE_INSENSITIVE`. A find bar that is case-SENSITIVE by
/// default is every browser's find bar except ours would have been.
pub const FIND_OPTIONS_CASE_INSENSITIVE: u32 = 1;

/// `WEBKIT_FIND_OPTIONS_BACKWARDS`. Deliberately NEVER in the mask — see
/// [`find_options_for`].
pub const FIND_OPTIONS_BACKWARDS: u32 = 8;

/// `WEBKIT_FIND_OPTIONS_WRAP_AROUND`. Without it the engine stops dead at the
/// last match and "next" silently does nothing, which reads as a broken bar.
pub const FIND_OPTIONS_WRAP_AROUND: u32 = 16;

/// The `max_match_count` handed to `webkit_find_controller_count_matches`.
///
/// **This constant is the difference between a count and a lie.** WebKit does
/// not report "more than N" when a page has more matches than the cap — it
/// reports the CAP, as if that were the total. A find bar capped at 100 tells a
/// user reading a long log page that there are exactly 100 hits, every time,
/// forever. Uncapped is the only honest value, and the engine walks the page
/// once either way.
pub const FIND_MAX_MATCH_COUNT: u32 = u32::MAX;

/// One step of a find interaction, as the UI and the verb speak it.
///
/// The engine bridge has its own `FindAction` (a vendored crate cannot depend on
/// this one); [`FindStep::as_verb`] is the only place the two names meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindStep {
    /// A fresh search: highlight every match, select the first.
    Search,
    /// Advance the selection to the next match of the current search.
    Next,
    /// Retreat the selection to the previous match.
    Prev,
    /// End the search — highlights and selection go away.
    Close,
}

impl FindStep {
    /// The wire name the app-control verb and the trace use.
    pub fn as_verb(self) -> &'static str {
        match self {
            FindStep::Search => "search",
            FindStep::Next => "next",
            FindStep::Prev => "prev",
            FindStep::Close => "close",
        }
    }

    /// Parse the verb's `--next` / `--prev` / `--close` selection.
    pub fn from_verb(raw: &str) -> Option<FindStep> {
        match raw {
            "search" => Some(FindStep::Search),
            "next" => Some(FindStep::Next),
            "prev" | "previous" => Some(FindStep::Prev),
            "close" | "finish" => Some(FindStep::Close),
            _ => None,
        }
    }
}

/// The `WebKitFindOptions` mask for a step.
///
/// Identical for every searching step, on purpose:
///
/// * `CASE_INSENSITIVE` because that is what a find bar means by "find";
/// * `WRAP_AROUND` because next-past-the-end must return to the top (and the
///   position cycle in [`advance_position`] promises exactly that);
/// * **never `BACKWARDS`** — direction is `webkit_find_controller_search_previous`'s
///   job. Setting the flag as well double-reverses: the engine searches backwards
///   *and* `search_previous` inverts, so "previous" walks FORWARD. The flag is
///   named here only so the next reader can see the omission is a decision.
pub fn find_options_for(step: FindStep) -> u32 {
    let _ = step;
    FIND_OPTIONS_CASE_INSENSITIVE | FIND_OPTIONS_WRAP_AROUND
}

// ---------------------------------------------------------------------------
// Position cycle
// ---------------------------------------------------------------------------

/// Where the selection lands after `step`, given the 1-based `position` it is at
/// now and the page's `count`.
///
/// WebKit tells us HOW MANY matches there are and moves the selection, but never
/// says which one is selected. So the position is ours to keep, and it must
/// wrap in lockstep with `WEBKIT_FIND_OPTIONS_WRAP_AROUND` or the label would
/// drift out of step with the highlight the user is looking at.
///
/// `0` means "no match selected" and is the only value a zero-match page can
/// hold.
pub fn advance_position(position: u32, count: u32, step: FindStep) -> u32 {
    if count == 0 || matches!(step, FindStep::Close) {
        return 0;
    }
    match step {
        FindStep::Search => 1,
        FindStep::Next => {
            if position >= count {
                1
            } else {
                position + 1
            }
        }
        FindStep::Prev => {
            if position <= 1 {
                count
            } else {
                position - 1
            }
        }
        FindStep::Close => 0,
    }
}

/// The `3/17` label. `0/0` when there is nothing to show — the honest reading of
/// "no matches", and what every browser draws.
pub fn position_label(position: u32, count: u32) -> String {
    format!("{position}/{count}")
}

// ---------------------------------------------------------------------------
// Keyboard ownership
// ---------------------------------------------------------------------------

/// Who held the keyboard before the find bar borrowed it, so close can hand it
/// straight back. The bar never DECIDES where focus should go — it only
/// remembers where focus came from.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum FindFocusOrigin {
    /// Shell chrome (a button, the omnibox, nothing in particular).
    #[default]
    Chrome,
    /// The terminal for this session was typing-ready. Ctrl+F over a web
    /// surface can still arrive with a terminal armed underneath.
    Terminal(String),
    /// The page itself had the keyboard (the normal browsing case).
    Page,
}

/// Where focus is being moved TO. Only ever `FindInput` on open and the
/// recorded origin on close: those are the only two moves the bar may make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindFocusTarget {
    FindInput,
    Terminal(String),
    Page,
    Chrome,
}

impl From<&FindFocusOrigin> for FindFocusTarget {
    fn from(origin: &FindFocusOrigin) -> Self {
        match origin {
            FindFocusOrigin::Chrome => FindFocusTarget::Chrome,
            FindFocusOrigin::Terminal(session) => FindFocusTarget::Terminal(session.clone()),
            FindFocusOrigin::Page => FindFocusTarget::Page,
        }
    }
}

/// A key as the find bar sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindKey {
    /// A printable character typed into the field.
    Char(String),
    Enter,
    ShiftEnter,
    Escape,
    /// Anything else (arrows, function keys, modifiers).
    Other(String),
}

/// What the bar does with a key it claimed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindKeyAction {
    /// The character goes into the field; the caller re-searches incrementally.
    Type(String),
    Next,
    Prev,
    Close,
}

/// Where a keystroke goes while a web surface owns the viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindRoute {
    /// The find bar claimed it.
    Bar(FindKeyAction),
    /// The bar did not claim it — it belongs to whoever holds focus, and the
    /// bar does not get to say who that is.
    NotOurs,
}

/// Does an open find bar block the terminal beneath from accepting input?
///
/// **Only while its input is focused.** An open-but-unfocused bar (the user
/// clicked back into the page, or into the terminal) claims nothing: a find bar
/// that wedges terminal input for as long as it is on screen is worse than no
/// find bar, which is the whole reason this predicate exists as its own named
/// thing instead of `find.is_some()` at the call site.
pub fn find_bar_blocks_terminal_input(bar_focused: bool) -> bool {
    bar_focused
}

// ---------------------------------------------------------------------------
// The trace — the falsification instrument, in production
// ---------------------------------------------------------------------------

/// Something the find bar did that could touch somebody else's keyboard.
///
/// This is not test scaffolding: production pushes every one of these, and the
/// `web find` verb reports the recent ring back to the caller, so an agent can
/// falsify "the find bar stole my focus" from the outside with the same evidence
/// the unit lock reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindTraceEvent {
    /// The bar opened over `session`, borrowing from `origin`.
    Opened {
        session: String,
        origin: FindFocusOrigin,
    },
    /// The bar moved keyboard focus. The ONLY legal targets are `FindInput`
    /// (on open) and the recorded origin (on close).
    FocusMoved {
        to: FindFocusTarget,
        reason: &'static str,
    },
    /// The bar claimed a key.
    KeyClaimed { key: String, action: &'static str },
    /// The bar declined a key and let it go where it was already going.
    KeyReleased { key: String },
    /// A step was asked of the engine.
    EngineStep { step: &'static str, query: String },
    /// The bar closed and gave the keyboard back.
    Closed { session: String },
}

impl FindTraceEvent {
    /// Does this event mean the find bar reached into a TERMINAL — took its
    /// focus or delivered it a key? This is the predicate the focus lock reads;
    /// naming it here (rather than pattern-matching in the test) is what keeps
    /// the lock honest when the event set grows.
    pub fn touches_terminal(&self) -> bool {
        match self {
            FindTraceEvent::FocusMoved { to, .. } => matches!(to, FindFocusTarget::Terminal(_)),
            _ => false,
        }
    }

    /// Does it mean the find bar let a key through to somebody else?
    pub fn released_a_key(&self) -> bool {
        matches!(self, FindTraceEvent::KeyReleased { .. })
    }
}

/// How many events the ring keeps. Big enough for a whole open->type->close
/// burst plus slack, small enough that it can never be a leak.
pub const FIND_TRACE_CAPACITY: usize = 128;

thread_local! {
    static FIND_TRACE: RefCell<VecDeque<FindTraceEvent>> =
        const { RefCell::new(VecDeque::new()) };
}

/// Record one find-bar action. Called from production, never from tests only.
pub fn trace(event: FindTraceEvent) {
    FIND_TRACE.with(|ring| {
        let mut ring = ring.borrow_mut();
        if ring.len() >= FIND_TRACE_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(event);
    });
}

/// The recent ring, oldest first.
pub fn trace_snapshot() -> Vec<FindTraceEvent> {
    FIND_TRACE.with(|ring| ring.borrow().iter().cloned().collect())
}

/// Drop the ring. Used when a burst window is being measured.
pub fn trace_clear() {
    FIND_TRACE.with(|ring| ring.borrow_mut().clear());
}

// ---------------------------------------------------------------------------
// The bar's state
// ---------------------------------------------------------------------------

/// One surface's open find bar. `None` on the surface means no bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebFindState {
    /// What is in the field right now.
    pub query: String,
    /// The engine's answer for `query`. `0` until an answer arrives.
    pub match_count: u32,
    /// 1-based index of the highlighted match; `0` when there is none.
    pub position: u32,
    /// Who had the keyboard when the bar opened, so close can give it back.
    pub origin: FindFocusOrigin,
    /// Is the field focused? The bar claims keys ONLY while this is true.
    pub bar_focused: bool,
    /// The last query the ENGINE was actually asked to search for. A `Next`
    /// against a different query has to restart the search rather than step a
    /// stale one.
    pub engine_query: String,
}

/// The engine request that ENDS a search: `search_finish`, no query.
///
/// Standalone rather than only a method, because closing must reach the engine
/// even when the bar is already gone from the shell's state. The bar's teardown
/// (which hands the keyboard back, synchronously) and the engine call (which is
/// async) are independently ordered, and `search_finish` is the one call that
/// drops the highlights — a close that could be skipped because the state got
/// there first would leave the page painted yellow with no bar to explain it.
pub fn close_request() -> (FindStep, String) {
    (FindStep::Close, String::new())
}

/// What to ask the engine for, given the bar's state — or its ABSENCE.
///
/// THE one answer to "what does this step ask the engine", used by the Ctrl+F
/// bar and the `web find` verb alike. `find == None` means no bar: there is
/// nothing to search for, but there may still be a search to END.
pub fn engine_request_for(
    find: Option<&WebFindState>,
    step: FindStep,
) -> Option<(FindStep, String)> {
    if matches!(step, FindStep::Close) {
        return Some(close_request());
    }
    find?.engine_request(step)
}

impl WebFindState {
    /// Open the bar over `session`, borrowing the keyboard from `origin`.
    pub fn open(session: &str, origin: FindFocusOrigin) -> WebFindState {
        trace(FindTraceEvent::Opened {
            session: session.to_string(),
            origin: origin.clone(),
        });
        trace(FindTraceEvent::FocusMoved {
            to: FindFocusTarget::FindInput,
            reason: "find_bar_open",
        });
        WebFindState {
            query: String::new(),
            match_count: 0,
            position: 0,
            origin,
            bar_focused: true,
            engine_query: String::new(),
        }
    }

    /// Route one keystroke. THE one place that decides whether the find bar or
    /// somebody else gets a key.
    ///
    /// An unfocused bar claims NOTHING — see [`find_bar_blocks_terminal_input`].
    pub fn route_key(&self, key: &FindKey) -> FindRoute {
        if !self.bar_focused {
            trace(FindTraceEvent::KeyReleased {
                key: describe_key(key),
            });
            return FindRoute::NotOurs;
        }
        let action = match key {
            FindKey::Char(text) => FindKeyAction::Type(text.clone()),
            FindKey::Enter => FindKeyAction::Next,
            FindKey::ShiftEnter => FindKeyAction::Prev,
            FindKey::Escape => FindKeyAction::Close,
            FindKey::Other(_) => {
                trace(FindTraceEvent::KeyReleased {
                    key: describe_key(key),
                });
                return FindRoute::NotOurs;
            }
        };
        trace(FindTraceEvent::KeyClaimed {
            key: describe_key(key),
            action: action_name(&action),
        });
        FindRoute::Bar(action)
    }

    /// The step this state wants from the engine for `step`, and the query to
    /// run it with. `None` when there is nothing to ask (an empty field).
    pub fn engine_request(&self, step: FindStep) -> Option<(FindStep, String)> {
        if matches!(step, FindStep::Close) {
            return Some(close_request());
        }
        if self.query.is_empty() {
            return None;
        }
        // A next/prev whose query has moved on since the engine last searched
        // is a NEW search, not a step: `webkit_find_controller_search_next`
        // walks whatever the controller is holding, which would be the old word.
        let step = match step {
            FindStep::Search => FindStep::Search,
            _ if self.engine_query != self.query => FindStep::Search,
            other => other,
        };
        Some((step, self.query.clone()))
    }

    /// Fold the engine's answer in: `count` is the engine's number, verbatim.
    ///
    /// The position is advanced by `step` and then CLAMPED into the count — a
    /// page that shrank under an incremental search (typing one more letter)
    /// must not leave the label pointing past the end.
    pub fn apply_engine_count(&mut self, step: FindStep, query: &str, count: u32) {
        self.engine_query = query.to_string();
        self.match_count = count;
        self.position = advance_position(self.position, count, step);
        if self.position > count {
            self.position = if count == 0 { 0 } else { count };
        }
    }

    /// Note that the engine was asked for `step`.
    pub fn note_engine_step(&self, step: FindStep, query: &str) {
        trace(FindTraceEvent::EngineStep {
            step: step.as_verb(),
            query: query.to_string(),
        });
    }

    /// Close the bar and give the keyboard back to whoever lent it.
    ///
    /// Returns the origin so the caller can perform the actual focus move; the
    /// trace records the move it is REQUIRED to make, and the focus lock reads
    /// that the move went to the origin and nowhere else.
    pub fn close(self, session: &str) -> FindFocusOrigin {
        trace(FindTraceEvent::FocusMoved {
            to: FindFocusTarget::from(&self.origin),
            reason: "find_bar_close",
        });
        trace(FindTraceEvent::Closed {
            session: session.to_string(),
        });
        self.origin
    }

    /// The `3/17` the bar draws.
    pub fn label(&self) -> String {
        position_label(self.position, self.match_count)
    }
}

fn describe_key(key: &FindKey) -> String {
    match key {
        FindKey::Char(text) => text.clone(),
        FindKey::Enter => "Enter".to_string(),
        FindKey::ShiftEnter => "Shift+Enter".to_string(),
        FindKey::Escape => "Escape".to_string(),
        FindKey::Other(name) => name.clone(),
    }
}

fn action_name(action: &FindKeyAction) -> &'static str {
    match action {
        FindKeyAction::Type(_) => "type",
        FindKeyAction::Next => "next",
        FindKeyAction::Prev => "prev",
        FindKeyAction::Close => "close",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture page, with a KNOWN number of planted occurrences.
    const FIXTURE: &str = include_str!("../fixtures/find-in-page.html");

    /// The needle the fixture plants.
    const NEEDLE: &str = "yggterm";

    /// How many times the fixture plants it, in mixed case.
    const PLANTED: u32 = 17;

    /// The fixture's rendered TEXT — tags and the `<head>` stripped, which is
    /// what the engine searches. Deliberately crude: the fixture is written to
    /// make a crude reading correct (no scripts, no styles, no attributes that
    /// contain the needle), so this cannot quietly disagree with WebKit.
    fn fixture_text(html: &str) -> String {
        let body = html
            .split_once("<body>")
            .map(|(_, rest)| rest)
            .unwrap_or(html);
        let mut out = String::new();
        let mut in_tag = false;
        for ch in body.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(ch),
                _ => {}
            }
        }
        out
    }

    /// A reference count of `needle` in `text`, honouring the case flag.
    fn reference_count(text: &str, needle: &str, case_insensitive: bool) -> u32 {
        let (haystack, needle) = if case_insensitive {
            (text.to_lowercase(), needle.to_lowercase())
        } else {
            (text.to_string(), needle.to_string())
        };
        if needle.is_empty() {
            return 0;
        }
        let mut count = 0u32;
        let mut from = 0usize;
        while let Some(at) = haystack[from..].find(&needle) {
            count += 1;
            from += at + needle.len();
        }
        count
    }

    /// What `webkit_find_controller_count_matches` REPORTS, including the one
    /// behaviour that turns a count into a lie: the cap is reported as if it
    /// were the total, with no "at least" anywhere in the API.
    ///
    /// Test-only by construction — production never models the engine, it asks
    /// it. This exists so the two constants production DOES own
    /// (`FIND_MAX_MATCH_COUNT` and the option mask) can be red-proven without a
    /// display server.
    fn engine_counted_matches(text: &str, needle: &str, options: u32, max: u32) -> u32 {
        let case_insensitive = options & FIND_OPTIONS_CASE_INSENSITIVE != 0;
        reference_count(text, needle, case_insensitive).min(max)
    }

    // -- LOCK 1: the verb reports the TRUE match count ----------------------

    #[test]
    fn find_reports_the_true_match_count_on_the_fixture_page() {
        let text = fixture_text(FIXTURE);
        // The fixture is the premise; if it drifts, everything below is noise.
        assert_eq!(
            reference_count(&text, NEEDLE, true),
            PLANTED,
            "the fixture must plant exactly {PLANTED} case-insensitive occurrences of {NEEDLE:?}"
        );
        assert!(
            reference_count(&text, NEEDLE, false) < PLANTED,
            "the fixture must plant occurrences in MIXED case, or the \
             case-insensitivity half of this lock proves nothing"
        );

        let options = find_options_for(FindStep::Search);
        let reported = engine_counted_matches(&text, NEEDLE, options, FIND_MAX_MATCH_COUNT);
        assert_eq!(
            reported, PLANTED,
            "the count the verb reports must be the page's TRUE match count: \
             options={options:#x}, cap={FIND_MAX_MATCH_COUNT}"
        );

        // And the plumbing carries the engine's number verbatim into the report.
        let mut state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        state.query = NEEDLE.to_string();
        state.apply_engine_count(FindStep::Search, NEEDLE, reported);
        assert_eq!(state.match_count, PLANTED);
        assert_eq!(state.label(), format!("1/{PLANTED}"));
    }

    #[test]
    fn find_options_are_case_insensitive_wrapping_and_never_backwards() {
        for step in [FindStep::Search, FindStep::Next, FindStep::Prev] {
            let options = find_options_for(step);
            assert_ne!(
                options & FIND_OPTIONS_CASE_INSENSITIVE,
                0,
                "{step:?} must search case-insensitively"
            );
            assert_ne!(
                options & FIND_OPTIONS_WRAP_AROUND,
                0,
                "{step:?} must wrap; without it `next` dies at the last match"
            );
            assert_eq!(
                options & FIND_OPTIONS_BACKWARDS,
                0,
                "{step:?} must NOT set BACKWARDS: search_previous already \
                 reverses, and both together walk forwards"
            );
        }
    }

    // -- LOCK 2: next/prev cycle with wrap ---------------------------------

    #[test]
    fn next_and_prev_cycle_the_position_with_wrap() {
        let text = fixture_text(FIXTURE);
        let count = engine_counted_matches(
            &text,
            NEEDLE,
            find_options_for(FindStep::Search),
            FIND_MAX_MATCH_COUNT,
        );
        assert_eq!(count, PLANTED);

        let mut state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        state.query = NEEDLE.to_string();
        state.apply_engine_count(FindStep::Search, NEEDLE, count);
        assert_eq!(state.position, 1, "a fresh search selects the first match");

        // Walk all the way round and land back on 1.
        let mut seen = vec![state.position];
        for _ in 0..count {
            state.apply_engine_count(FindStep::Next, NEEDLE, count);
            seen.push(state.position);
        }
        assert_eq!(
            seen,
            (1..=count).chain(std::iter::once(1)).collect::<Vec<_>>(),
            "next must visit every match in order and WRAP to 1"
        );

        // And backwards off the front wraps to the last.
        assert_eq!(state.position, 1);
        state.apply_engine_count(FindStep::Prev, NEEDLE, count);
        assert_eq!(
            state.position, count,
            "prev from the first match must wrap to the LAST"
        );
        state.apply_engine_count(FindStep::Prev, NEEDLE, count);
        assert_eq!(state.position, count - 1);
    }

    #[test]
    fn an_incremental_search_that_shrinks_the_page_clamps_the_position() {
        let mut state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        state.query = NEEDLE.to_string();
        state.apply_engine_count(FindStep::Search, NEEDLE, 17);
        for _ in 0..9 {
            state.apply_engine_count(FindStep::Next, NEEDLE, 17);
        }
        assert_eq!(state.position, 10);
        // One more letter typed; only two matches survive.
        state.query = format!("{NEEDLE}x");
        state.apply_engine_count(FindStep::Search, &state.query.clone(), 2);
        assert_eq!(state.match_count, 2);
        assert!(
            state.position <= 2 && state.position >= 1,
            "position {} must stay inside 1..=2",
            state.position
        );
    }

    #[test]
    fn a_page_with_no_matches_has_no_position() {
        let mut state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        state.query = "zzzznotonthispage".to_string();
        state.apply_engine_count(FindStep::Search, "zzzznotonthispage", 0);
        assert_eq!(state.label(), "0/0");
        state.apply_engine_count(FindStep::Next, "zzzznotonthispage", 0);
        assert_eq!(
            state.label(),
            "0/0",
            "next on an empty result must not move"
        );
    }

    // -- LOCK 3: close clears, and a second search counts FRESH -------------

    #[test]
    fn close_finishes_the_search_and_a_second_search_counts_fresh() {
        let mut state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        state.query = NEEDLE.to_string();
        state.apply_engine_count(FindStep::Search, NEEDLE, PLANTED);
        assert_eq!(state.match_count, PLANTED);

        // Closing must ASK the engine to finish — that call is what drops the
        // highlights; forgetting it leaves a page painted yellow with no bar.
        let request = state.engine_request(FindStep::Close);
        assert_eq!(
            request,
            Some((FindStep::Close, String::new())),
            "close must issue a Close step to the engine (search_finish)"
        );
        state.close("local://fixture");

        // And the close reaches the engine even when the bar is ALREADY GONE.
        // The teardown is synchronous and the engine call is not, so on the
        // ordering where the state gets there first there is no bar left to ask
        // — and `search_finish` still has to happen, or the highlights outlive
        // the thing that made them.
        assert_eq!(
            engine_request_for(None, FindStep::Close),
            Some(close_request()),
            "closing must reach the engine with no bar in the state"
        );
        assert_eq!(
            engine_request_for(None, FindStep::Search),
            None,
            "with no bar there is nothing to search FOR — only a search to end"
        );

        // A fresh bar starts from nothing: no carried count, no carried
        // position, and an engine_query that forces a real search rather than a
        // step against state the engine no longer holds.
        let reopened = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        assert_eq!(reopened.match_count, 0);
        assert_eq!(reopened.position, 0);
        assert_eq!(reopened.engine_query, "");
        let mut reopened = reopened;
        reopened.query = NEEDLE.to_string();
        assert_eq!(
            reopened.engine_request(FindStep::Next),
            Some((FindStep::Search, NEEDLE.to_string())),
            "after a close, `next` must restart the search — the engine is no \
             longer holding one to step"
        );
    }

    #[test]
    fn a_next_against_a_changed_query_restarts_the_search() {
        let mut state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        state.query = NEEDLE.to_string();
        state.apply_engine_count(FindStep::Search, NEEDLE, PLANTED);
        assert_eq!(
            state.engine_request(FindStep::Next),
            Some((FindStep::Next, NEEDLE.to_string())),
            "same query: step it"
        );
        state.query = "surface".to_string();
        assert_eq!(
            state.engine_request(FindStep::Next),
            Some((FindStep::Search, "surface".to_string())),
            "changed query: search it, never step the stale one"
        );
    }

    #[test]
    fn an_empty_field_asks_the_engine_for_nothing() {
        let state = WebFindState::open("local://fixture", FindFocusOrigin::Page);
        assert_eq!(state.engine_request(FindStep::Search), None);
        assert_eq!(state.engine_request(FindStep::Next), None);
    }

    // -- LOCK 4: the focus lock --------------------------------------------

    /// Open the bar, type a word, step it, close it — and prove that nothing in
    /// that whole burst reached the terminal underneath.
    ///
    /// The J2 idiom: drive the PRODUCTION routing and PRODUCTION open/close,
    /// then read the production trace ring and require that no event in the
    /// window touched a terminal or leaked a key. A bar that forgot to check
    /// `bar_focused`, or that "helpfully" focused the terminal on close, both
    /// show up here as an event that must not exist.
    #[test]
    fn the_find_bar_never_touches_the_terminal_beneath_during_a_burst() {
        const SESSION: &str = "local://agent-42";
        trace_clear();

        // The terminal underneath was the keyboard owner when Ctrl+F landed —
        // the hardest case, because it is the one where a careless close has
        // something to steal.
        let origin = FindFocusOrigin::Terminal(SESSION.to_string());
        let mut state = WebFindState::open(SESSION, origin.clone());

        for ch in "yggterm".chars() {
            let key = FindKey::Char(ch.to_string());
            match state.route_key(&key) {
                FindRoute::Bar(FindKeyAction::Type(text)) => state.query.push_str(&text),
                other => panic!("a focused find bar must claim {ch:?}, got {other:?}"),
            }
            if let Some((step, query)) = state.engine_request(FindStep::Search) {
                state.note_engine_step(step, &query);
                state.apply_engine_count(step, &query, PLANTED);
            }
        }
        assert_eq!(state.query, NEEDLE);

        // Enter = next, Shift+Enter = previous.
        assert_eq!(
            state.route_key(&FindKey::Enter),
            FindRoute::Bar(FindKeyAction::Next)
        );
        state.apply_engine_count(FindStep::Next, NEEDLE, PLANTED);
        assert_eq!(
            state.route_key(&FindKey::ShiftEnter),
            FindRoute::Bar(FindKeyAction::Prev)
        );
        state.apply_engine_count(FindStep::Prev, NEEDLE, PLANTED);

        // Escape closes.
        assert_eq!(
            state.route_key(&FindKey::Escape),
            FindRoute::Bar(FindKeyAction::Close)
        );
        let restored = state.close(SESSION);
        assert_eq!(
            restored, origin,
            "close must return the keyboard to its lender"
        );

        let events = trace_snapshot();
        assert!(
            !events.is_empty(),
            "the trace is the instrument; an empty ring means the lock proved nothing"
        );

        // The burst window is exactly the span during which the bar HELD the
        // keyboard: from the open to the give-back. The give-back itself is the
        // window's closing edge, not an event inside it — returning the
        // keyboard to a terminal is the promise, and a window that contained it
        // would fail on the one move the bar is obliged to make.
        let opened_at = events
            .iter()
            .position(|event| matches!(event, FindTraceEvent::Opened { .. }))
            .expect("the burst must contain the open");
        let gave_back_at = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    FindTraceEvent::FocusMoved {
                        reason: "find_bar_close",
                        ..
                    }
                )
            })
            .expect("the burst must contain the give-back");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, FindTraceEvent::Closed { .. })),
            "the burst must contain the close"
        );
        let window = &events[opened_at..gave_back_at];

        // (a) No key the bar saw while focused escaped to anyone else.
        let leaked: Vec<_> = window
            .iter()
            .filter(|event| event.released_a_key())
            .collect();
        assert!(
            leaked.is_empty(),
            "a focused find bar leaked keys to whatever is underneath: {leaked:?}"
        );

        // (b) No focus move inside the window went to a terminal.
        let stolen: Vec<_> = window
            .iter()
            .filter(|event| event.touches_terminal())
            .collect();
        assert!(
            stolen.is_empty(),
            "the find bar moved focus onto a terminal DURING its own burst: {stolen:?}"
        );

        // (c) Exactly two focus moves in the whole burst: in, and back out to
        // the lender. Anything else is the bar deciding where focus belongs.
        let moves: Vec<&FindFocusTarget> = events
            .iter()
            .filter_map(|event| match event {
                FindTraceEvent::FocusMoved { to, .. } => Some(to),
                _ => None,
            })
            .collect();
        assert_eq!(
            moves,
            vec![
                &FindFocusTarget::FindInput,
                &FindFocusTarget::Terminal(SESSION.to_string()),
            ],
            "the bar may make exactly two focus moves: borrow, then give back"
        );

        // (d) And while the bar held the keyboard, the terminal's own input
        // gate was shut — the production predicate, not a restatement of it.
        assert!(
            find_bar_blocks_terminal_input(true),
            "a focused find bar must shut the terminal input gate"
        );
    }

    #[test]
    fn an_unfocused_find_bar_claims_nothing() {
        trace_clear();
        let mut state = WebFindState::open("local://agent-42", FindFocusOrigin::Page);
        // The user clicked back into the page: the bar is still on screen, and
        // it is now nobody's keyboard owner.
        state.bar_focused = false;
        for key in [
            FindKey::Char("a".to_string()),
            FindKey::Enter,
            FindKey::Escape,
        ] {
            assert_eq!(
                state.route_key(&key),
                FindRoute::NotOurs,
                "an unfocused bar must not claim {key:?}"
            );
        }
        assert!(
            !find_bar_blocks_terminal_input(false),
            "an open-but-unfocused bar must not wedge terminal input"
        );
    }

    #[test]
    fn the_trace_ring_is_bounded() {
        trace_clear();
        for index in 0..(FIND_TRACE_CAPACITY * 3) {
            trace(FindTraceEvent::KeyReleased {
                key: index.to_string(),
            });
        }
        assert_eq!(trace_snapshot().len(), FIND_TRACE_CAPACITY);
    }
}
