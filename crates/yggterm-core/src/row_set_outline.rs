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

use std::collections::{HashMap, HashSet};

use crate::row_set::RowSets;
use crate::session_outline::{parse_outline_key, OutlineKey};

/// The arrangement the sidebar draws, derived from the seats the rows hold.
///
/// `rows` is `(row path, stored outline prefix)` in the order the sidebar means
/// to draw them; members land under their head in that same order.
/// `collapsed_heads` is the user's own state — which sets they shut — and is the
/// one half of an arrangement that CANNOT be derived, so it is passed in.
///
/// ⚖ When an explicit arrangement exists (the `row-set` verb and the inside-band
/// drag, both still to be built), it becomes a third argument here and wins
/// per-row over the derived edge: this function stays the single answer to
/// "what does the sidebar draw", rather than the caller merging two of them.
pub fn sidebar_row_sets<'a, I>(rows: I, collapsed_heads: &HashSet<String>) -> RowSets
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let mut sets = outline_row_sets(rows);
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
    let mut sets = RowSets::default();
    for (path, segments) in &seated {
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

    #[test]
    fn no_seats_at_all_costs_nothing() {
        let sets = sidebar_row_sets(rows(&[("a", ""), ("b", "")]), &HashSet::new());
        assert!(sets.is_empty());
    }
}
