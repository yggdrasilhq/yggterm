//! THE DEFAULT ARRANGEMENT — the row sets a sidebar has before anyone arranges
//! one by hand.
//!
//! `DESIGN.md` §"Row sets" settles that membership is arbitrary and that "the
//! outline numbers are the *default* arrangement, never a restriction on it".
//! This module is that default. It is also the ONLY place a seat number may
//! produce a containment edge: [`crate::row_set`] refuses to consult a seat and
//! [`crate::session_outline`] refuses to know what a set is, so the bridge lives
//! in a third module rather than being smuggled into either of them.
//!
//! **THE RULE, as the owner stated it:** group `N.x` rows under `N.0` as the
//! header, and `N.x.y` rows under their `N.x` header where applicable. So a
//! head is found by looking, in order, for:
//!
//! 1. the **`.0` seat of the row's own level** — `6.1` and `6.2` sit under
//!    `6.0`, and `6.1.1` sits under `6.1.0` if a sub-orchestrator holds that
//!    seat;
//! 2. failing that, the **bare parent seat** — `6.1.1` sits under `6.1`, and
//!    `6.0` itself sits under a plain `6` when one exists.
//!
//! One rule, applied at every depth, which is what makes the nesting recursive
//! rather than two hard-coded levels.
//!
//! ⭐ **DERIVED EVERY FRAME, NEVER STORED.** The seats already say this; writing
//! the containment down as well would be the same fact in two places, and the
//! copy would go stale the moment a delegate is reseated or reaped — the
//! single-source-of-truth law, and the same reasoning that keeps
//! `outline_prefix_heads_a_group` a live question rather than a flag.
//!
//! ⚠ **A HEAD IS A ROW, NOT A HEADING.** If nobody holds `6.0`, the rows `6.1`
//! and `6.2` stay top level rather than collecting under an invented `6`
//! placeholder: a set's head is an ordinary session row, and a synthetic one
//! could not be clicked, closed or driven.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::row_set::{RowSetRefusal, RowSets};
use crate::session_outline::{parse_outline_key, OutlineKey};

/// The arrangement the USER built by hand — what a drag or a verb said.
///
/// ⛔ **THIS IS NOT A SECOND KIND OF GROUP.** There is one model of what a group
/// IS — [`RowSets`], a containment relation over row paths — and two inputs that
/// can produce an edge in it: this, and the seats. `DESIGN.md` §"Row sets"
/// settles the precedence in the sentence that has been there since the noun was
/// chosen: *membership is arbitrary and the user may put any rows together — the
/// outline numbers are the DEFAULT arrangement, never a restriction on it.*
/// So the seats fill in for rows nobody has arranged, and a hand-arranged row
/// keeps the answer its owner gave it.
///
/// ⭐ **A DRAG WRITES MEMBERSHIP AND NEVER A SEAT.** Forming a set by
/// renumbering would rewrite `outline_prefix` on rows the user created, which
/// the row-hygiene law forbids outright — and it would fail anyway on the rows
/// he most needs to group, since an un-numbered head has no number for a member
/// to inherit. Membership needs no seat, so a set of un-numbered rows is
/// ordinary rather than a special case.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowArrangement {
    /// The containment the user built.
    #[serde(default)]
    pub sets: RowSets,
    /// Rows the user pulled OUT to the top level.
    ///
    /// ⚠ **A THIRD STATE, AND IT HAS TO EXIST.** "No entry in `sets`" cannot
    /// mean both *never arranged* and *deliberately loose*: without this, a row
    /// dragged out of `6.0` would be silently re-adopted by its seat on the very
    /// next frame, and the user's gesture would appear to do nothing.
    #[serde(default)]
    pub detached: HashSet<String>,
}

impl RowArrangement {
    /// Has the user given this row an answer of their own — either a head, or a
    /// deliberate place at the top level?
    pub fn answered(&self, path: &str) -> bool {
        self.sets.parent_of(path).is_some() || self.detached.contains(path)
    }

    /// Put `member` under `head` by hand.
    pub fn attach(
        &mut self,
        head: &str,
        member: &str,
        index: Option<usize>,
    ) -> Result<(), RowSetRefusal> {
        self.sets.insert_member(head, member, index)?;
        self.detached.remove(member);
        Ok(())
    }

    /// Take `member` out to the top level by hand, and REMEMBER that it was a
    /// choice — see the note on [`Self::detached`].
    pub fn detach(&mut self, member: &str) {
        self.sets.detach(member);
        self.detached.insert(member.to_string());
    }

    /// Forget every answer about rows that are no longer on the sidebar.
    ///
    /// Returns true when anything changed, so a caller can skip a persist it
    /// does not need.
    pub fn retain_live(&mut self, live: &HashSet<String>) -> bool {
        let mut changed = self.sets.retain_live(live);
        let departed: Vec<String> = self
            .detached
            .iter()
            .filter(|path| !live.contains(*path))
            .cloned()
            .collect();
        for path in departed {
            self.detached.remove(&path);
            changed = true;
        }
        changed
    }
}

/// The arrangement the sidebar draws, derived from the seats the rows hold.
///
/// `rows` is `(row path, stored outline prefix)` in the order the sidebar means
/// to draw them; members land under their head in that same order.
/// `collapsed_heads` is the user's own state — which sets they shut — and is the
/// one half of an arrangement that CANNOT be derived, so it is passed in.
///
/// `arrangement` is what the user said by hand, and it WINS per row: a row they
/// have arranged keeps their answer, and every other row falls to its seat. One
/// function is the single answer to "what does the sidebar draw", rather than a
/// caller merging two of them.
pub fn sidebar_row_sets<'a, I>(
    rows: I,
    arrangement: &RowArrangement,
    collapsed_heads: &HashSet<String>,
) -> RowSets
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let mut sets = outline_row_sets_with(rows, arrangement);
    for head in collapsed_heads {
        // A flag on a row that heads nothing is kept OUT of the model rather
        // than carried in it: `is_collapsed` would then answer true for a row
        // with no disclosure control, and the render path would have to
        // second-guess it. The user's own store still remembers, so the set
        // shuts again the moment it has members.
        if sets.is_head(head) {
            sets.set_collapsed(head, true);
        }
    }
    sets
}

/// The containment relation the seats imply, with no collapse state.
///
/// Split out from [`sidebar_row_sets`] because "what nests under what" is worth
/// asserting on its own — every test below is about this half.
pub fn outline_row_sets<'a, I>(rows: I) -> RowSets
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    outline_row_sets_with(rows, &RowArrangement::default())
}

/// The same, starting from what the user arranged by hand.
///
/// The seats fill in only where the user has said nothing — see
/// [`RowArrangement`] for why "said nothing" and "said top level" have to be
/// distinguishable.
pub fn outline_row_sets_with<'a, I>(rows: I, arrangement: &RowArrangement) -> RowSets
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let mut sets = arrangement.sets.clone();
    let seated: Vec<(&str, Vec<u64>)> = rows
        .into_iter()
        .filter_map(|(path, prefix)| match parse_outline_key(prefix) {
            OutlineKey::Numbered(segments) => Some((path, segments)),
            OutlineKey::Unnumbered => None,
        })
        .collect();
    // ⚠ FIRST row wins a contested seat. Two rows may legitimately hold `6.1`
    // for a moment during a handover, and the alternative — letting the later
    // one take the seat — would move an established set under a row that is
    // about to be reaped. Both are still drawn; only the HEAD is decided here.
    let mut head_by_seat: HashMap<&[u64], &str> = HashMap::new();
    for (path, segments) in &seated {
        head_by_seat.entry(segments.as_slice()).or_insert(path);
    }
    for (path, segments) in &seated {
        // The user's own answer stands. A seat may fill a hole; it may not
        // overrule a hand.
        if arrangement.answered(path) {
            continue;
        }
        let Some(head) = head_seat_candidates(segments)
            .into_iter()
            .find_map(|candidate| {
                head_by_seat
                    .get(candidate.as_slice())
                    .copied()
                    .filter(|head| head != path)
            })
        else {
            continue;
        };
        // A refusal here is a cycle or a self-parent, neither of which the
        // candidate rule can produce (every candidate is either shorter or ends
        // in the 0 a row cannot itself be while asking). Ignored rather than
        // unwrapped: a render path may not panic on a sidebar's shape.
        let _ = sets.insert_member(head, path, None);
    }
    sets
}

/// The seats that could head a row at `segments`, best first.
///
/// Empty for a top-level seat: `6` has no head, which is what makes it a book
/// rather than a chapter.
fn head_seat_candidates(segments: &[u64]) -> Vec<Vec<u64>> {
    if segments.len() < 2 {
        return Vec::new();
    }
    let parent = &segments[..segments.len() - 1];
    let mut level_head = parent.to_vec();
    level_head.push(0);
    // `6.0` asking for `6.0` is itself; it falls through to `6`. That single
    // skip is what keeps the relation acyclic — a `.0` seat only ever looks
    // UPWARD, so two rows can never hold each other.
    if level_head == segments {
        return vec![parent.to_vec()];
    }
    vec![level_head, parent.to_vec()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<'a>(pairs: &'a [(&'a str, &'a str)]) -> Vec<(&'a str, Option<&'a str>)> {
        pairs
            .iter()
            .map(|(path, prefix)| (*path, (!prefix.is_empty()).then_some(*prefix)))
            .collect()
    }

    fn visible(sets: &RowSets, order: &[&str]) -> Vec<(String, usize)> {
        sets.visible_rows(order.iter().copied())
    }

    /// The owner's ask, verbatim: `N.x` under `N.0`, drawn as one outline.
    #[test]
    fn a_book_collects_under_the_seat_that_heads_it() {
        let sets = outline_row_sets(rows(&[
            ("orch", "6.0"),
            ("gate", "6.1"),
            ("deploy", "6.2"),
            ("ux", "6.3"),
        ]));
        assert_eq!(
            visible(&sets, &["orch", "gate", "deploy", "ux"]),
            vec![
                ("orch".into(), 0),
                ("gate".into(), 1),
                ("deploy".into(), 1),
                ("ux".into(), 1),
            ]
        );
    }

    /// The second half of the ask, and the reason the rule is one rule: a
    /// cluster that orchestrates its own units nests again, with no new code.
    #[test]
    fn a_cluster_that_orchestrates_its_own_units_nests_again() {
        let sets = outline_row_sets(rows(&[
            ("orch", "6.0"),
            ("gate", "6.1"),
            ("gate-a", "6.1.1"),
            ("gate-b", "6.1.2"),
            ("ux", "6.3"),
        ]));
        assert_eq!(
            visible(&sets, &["orch", "gate", "gate-a", "gate-b", "ux"]),
            vec![
                ("orch".into(), 0),
                ("gate".into(), 1),
                ("gate-a".into(), 2),
                ("gate-b".into(), 2),
                ("ux".into(), 1),
            ]
        );
    }

    /// A sub-orchestrator's own `.0` seat outranks the bare parent, so the two
    /// levels are the same rule rather than a special case for the top.
    #[test]
    fn a_sub_orchestrator_heads_its_own_level() {
        let sets = outline_row_sets(rows(&[
            ("gate", "6.1"),
            ("sub", "6.1.0"),
            ("one", "6.1.1"),
            ("two", "6.1.2"),
        ]));
        assert_eq!(
            visible(&sets, &["gate", "sub", "one", "two"]),
            vec![
                ("gate".into(), 0),
                ("sub".into(), 1),
                ("one".into(), 2),
                ("two".into(), 2),
            ]
        );
    }

    /// Several books run at once — the live sidebar always has more than one —
    /// and neither may reach into the other.
    #[test]
    fn concurrent_books_stay_apart() {
        let sets = outline_row_sets(rows(&[
            ("six", "6.0"),
            ("six-a", "6.1"),
            ("nine", "9.0"),
            ("nine-a", "9.1"),
        ]));
        assert_eq!(sets.parent_of("six-a"), Some("six"));
        assert_eq!(sets.parent_of("nine-a"), Some("nine"));
        assert_eq!(sets.parent_of("six"), None);
        assert_eq!(sets.parent_of("nine"), None);
    }

    /// ⛔ No head, no set. The rows keep their seats and their order and simply
    /// stay top level; inventing a `6` placeholder would put a row on screen
    /// that cannot be clicked, closed or driven.
    #[test]
    fn rows_whose_head_is_absent_stay_top_level() {
        let sets = outline_row_sets(rows(&[("gate", "6.1"), ("ux", "6.3")]));
        assert!(sets.is_empty());
        assert_eq!(
            visible(&sets, &["gate", "ux"]),
            vec![("gate".into(), 0), ("ux".into(), 0)]
        );
    }

    /// A bare `6` heads the book when it exists — the owner's convention is
    /// `N.0`, and this is the sidebar he had before the convention.
    #[test]
    fn a_bare_top_level_seat_still_heads_what_sits_beneath_it() {
        let sets = outline_row_sets(rows(&[("book", "6"), ("head", "6.0"), ("one", "6.1")]));
        assert_eq!(sets.parent_of("head"), Some("book"));
        assert_eq!(sets.parent_of("one"), Some("head"));
        assert_eq!(
            visible(&sets, &["book", "head", "one"]),
            vec![("book".into(), 0), ("head".into(), 1), ("one".into(), 2)]
        );
    }

    /// Unnumbered rows are the majority of a working sidebar and must be left
    /// exactly where they are.
    #[test]
    fn unnumbered_rows_are_never_arranged() {
        let sets = outline_row_sets(rows(&[
            ("orch", "6.0"),
            ("scratch", ""),
            ("gate", "6.1"),
            ("shell", ""),
        ]));
        assert_eq!(sets.parent_of("scratch"), None);
        assert_eq!(sets.parent_of("shell"), None);
        assert_eq!(
            visible(&sets, &["orch", "scratch", "gate", "shell"]),
            vec![
                ("orch".into(), 0),
                ("gate".into(), 1),
                ("scratch".into(), 0),
                ("shell".into(), 0),
            ],
            "the head's position carries its members; the loose rows keep theirs"
        );
    }

    /// Two rows holding one seat is a handover, not a corruption: both are
    /// drawn, the first holds the seat, and the second joins the set rather
    /// than vanishing or taking it over.
    #[test]
    fn a_contested_seat_keeps_both_rows_and_moves_neither_set() {
        let sets = outline_row_sets(rows(&[
            ("orch", "6.0"),
            ("gate-old", "6.1"),
            ("gate-new", "6.1"),
        ]));
        assert_eq!(sets.parent_of("gate-old"), Some("orch"));
        assert_eq!(sets.parent_of("gate-new"), Some("orch"));
        assert_eq!(
            visible(&sets, &["orch", "gate-old", "gate-new"]),
            vec![
                ("orch".into(), 0),
                ("gate-old".into(), 1),
                ("gate-new".into(), 1),
            ]
        );
    }

    /// The seat is a stored fact, so a malformed one is placed rather than
    /// dropped — the same promise `session_outline` makes about the sort.
    #[test]
    fn a_seat_the_outline_cannot_read_is_left_alone() {
        let sets = outline_row_sets(rows(&[("orch", "6.0"), ("odd", "lobe-6"), ("gate", "6.1")]));
        assert_eq!(sets.parent_of("odd"), None);
        assert_eq!(sets.parent_of("gate"), Some("orch"));
    }

    /// ⛔ The property the whole candidate rule exists to guarantee: no chain of
    /// heads can close a loop, whatever seats are live, so `visible_rows`
    /// terminates without a cycle guard being the thing that saves it.
    #[test]
    fn no_arrangement_of_seats_can_close_a_loop() {
        let seats = [
            "0", "1", "1.0", "1.1", "1.1.0", "1.1.1", "1.1.2", "1.2", "2", "2.0", "10", "10.0",
            "10.10",
        ];
        let paths: Vec<String> = seats.iter().map(|seat| format!("row-{seat}")).collect();
        let pairs: Vec<(&str, Option<&str>)> = paths
            .iter()
            .zip(seats.iter())
            .map(|(path, seat)| (path.as_str(), Some(*seat)))
            .collect();
        let sets = outline_row_sets(pairs);
        let order: Vec<&str> = paths.iter().map(String::as_str).collect();
        // Every row is drawn exactly once — the property a cycle would break.
        assert_eq!(sets.visible_rows(order.iter().copied()).len(), seats.len());
    }

    /// The user's collapse is the one half of an arrangement that cannot be
    /// derived, and it must survive being handed a head that currently heads
    /// nothing (a set whose members were all reaped).
    #[test]
    fn a_collapse_applies_to_a_head_and_is_ignored_for_a_row_that_heads_nothing() {
        let collapsed: HashSet<String> = ["orch".to_string(), "gate".to_string()]
            .into_iter()
            .collect();
        let sets = sidebar_row_sets(
            rows(&[("orch", "6.0"), ("gate", "6.1"), ("ux", "6.3")]),
            &RowArrangement::default(),
            &collapsed,
        );
        assert!(sets.is_collapsed("orch"));
        assert!(
            !sets.is_collapsed("gate"),
            "a row with no members offers no disclosure control, so it holds no flag"
        );
        assert_eq!(
            visible(&sets, &["orch", "gate", "ux"]),
            vec![("orch".into(), 0)],
            "the shut set hides its members and keeps its head"
        );
    }

    /// ⭐ THE ANSWER TO "HOW DO I MAKE A GROUP?" — a hand can collect rows that
    /// carry no number at all, because membership needs no seat.
    #[test]
    fn a_hand_can_group_rows_that_have_no_seats() {
        let mut arrangement = RowArrangement::default();
        arrangement.attach("shell-a", "shell-b", None).expect("attachable");
        arrangement.attach("shell-a", "shell-c", None).expect("attachable");
        let sets = sidebar_row_sets(
            rows(&[("shell-a", ""), ("shell-b", ""), ("shell-c", "")]),
            &arrangement,
            &HashSet::new(),
        );
        assert_eq!(
            visible(&sets, &["shell-a", "shell-b", "shell-c"]),
            vec![
                ("shell-a".into(), 0),
                ("shell-b".into(), 1),
                ("shell-c".into(), 1),
            ],
            "no seat was invented, and no row was renumbered, to make this set"
        );
    }

    /// A hand outranks a seat, per row, and only per row: the rows the user
    /// never touched keep falling to their numbers.
    #[test]
    fn a_hand_arranged_row_keeps_its_answer_and_the_rest_still_follow_their_seats() {
        let mut arrangement = RowArrangement::default();
        // Moved out of its own book and into another one, by hand.
        arrangement.attach("nine", "gate", None).expect("attachable");
        let sets = sidebar_row_sets(
            rows(&[
                ("orch", "6.0"),
                ("gate", "6.1"),
                ("ux", "6.3"),
                ("nine", "9.0"),
            ]),
            &arrangement,
            &HashSet::new(),
        );
        assert_eq!(sets.parent_of("gate"), Some("nine"), "the hand wins");
        assert_eq!(sets.parent_of("ux"), Some("orch"), "the untouched row still follows its seat");
    }

    /// ⛔ THE ONE A DERIVED DEFAULT GETS WRONG. A row dragged OUT must stay out;
    /// re-adopting it from its seat on the next frame makes the gesture look
    /// like it did nothing.
    #[test]
    fn a_row_dragged_out_of_its_set_is_not_re_adopted_by_its_seat() {
        let mut arrangement = RowArrangement::default();
        arrangement.detach("gate");
        let sets = sidebar_row_sets(
            rows(&[("orch", "6.0"), ("gate", "6.1"), ("ux", "6.3")]),
            &arrangement,
            &HashSet::new(),
        );
        assert_eq!(sets.parent_of("gate"), None, "it stays where it was dropped");
        assert_eq!(sets.parent_of("ux"), Some("orch"));
        assert_eq!(
            visible(&sets, &["orch", "gate", "ux"]),
            vec![("orch".into(), 0), ("ux".into(), 1), ("gate".into(), 0)]
        );
    }

    /// A hand cannot tie a knot the model refuses, and the refusal costs the
    /// rest of the arrangement nothing.
    #[test]
    fn a_hand_arrangement_that_would_cycle_is_refused_and_changes_nothing() {
        let mut arrangement = RowArrangement::default();
        arrangement.attach("a", "b", None).expect("attachable");
        assert!(arrangement.attach("b", "a", None).is_err());
        assert_eq!(arrangement.sets.parent_of("b"), Some("a"));
    }

    /// The user's answers are forgotten when their rows leave, so a path that
    /// comes back is not silently filed under a set it never joined.
    #[test]
    fn answers_about_departed_rows_are_forgotten() {
        let mut arrangement = RowArrangement::default();
        arrangement.attach("head", "member", None).expect("attachable");
        arrangement.detach("loose");
        let live: HashSet<String> = ["head".to_string()].into_iter().collect();
        assert!(arrangement.retain_live(&live));
        assert!(!arrangement.answered("member"));
        assert!(!arrangement.answered("loose"));
        assert!(!arrangement.retain_live(&live), "a second pass changes nothing");
    }

    /// The hand-built half survives a restart, and the derived half is rebuilt
    /// rather than stored — so a reseated row moves and an arranged row does not.
    #[test]
    fn only_the_hand_built_half_goes_to_disk() {
        let mut arrangement = RowArrangement::default();
        arrangement.attach("head", "member", None).expect("attachable");
        arrangement.detach("loose");
        let json = serde_json::to_string(&arrangement).expect("serializable");
        let back: RowArrangement = serde_json::from_str(&json).expect("loadable");
        assert_eq!(back, arrangement);
        assert!(back.answered("member") && back.answered("loose"));
    }

    #[test]
    fn no_seats_at_all_costs_nothing() {
        let sets = sidebar_row_sets(
            rows(&[("a", ""), ("b", "")]),
            &RowArrangement::default(),
            &HashSet::new(),
        );
        assert!(sets.is_empty());
    }
}
