//! THE OUTLINE ORDER — one owner for "where does this row sit?".
//!
//! The owner numbers his sidebar like a book: `0`, `1`, `1.1`, `2`, `4`, `5.1`,
//! `5.2`, `6`. His acceptance criteria for the sort, in his own words, are
//! *"sorts are cheap and never break"*, and every rule below exists to make one
//! of those two words true:
//!
//! - **cheap** — this is a pure function of a row's stored prefix, so it can run
//!   on EVERY projection of the live list rather than as a batch someone has to
//!   remember to invoke. A sort that must be invoked is wrong between the spawn
//!   and the invocation, which on the night of 2026-08-07 was about sixty
//!   seconds, four times over.
//! - **never break** — TOTAL (every row gets a position, including rows with no
//!   number), IDEMPOTENT (sorting a sorted list is the identity), and STABLE
//!   (rows that tie, and rows with no number at all, keep the relative order
//!   they arrived in, so an unnumbered row never jitters between frames).
//!
//! ⛔ **The trap this module exists to close: compare segments as INTEGERS.**
//! A lexicographic sort of `"10"` and `"2"` puts `10` first, which is correct
//! string order and wrong outline order — and it looks right until the owner has
//! a tenth lobe, which is the worst moment for a sort to start lying. Every
//! comparison here is over `u64` segments.
//!
//! The prefix itself is a stored fact on the row (`outline_prefix`), NEVER
//! parsed back out of the title: the title is prose the agent CLI rewrites
//! whenever it likes, and a number encoded in prose is destroyed by every
//! re-title. That is the same single-source-of-truth law the rest of this
//! project runs on.

/// A parsed outline position, comparable the way the owner reads it.
///
/// `Numbered` sorts ahead of `Unnumbered` so the campaign rows sit at the top
/// and un-numbered noise collects below, which is exactly how the owner uses
/// the region: nine campaign rows he reads, and roughly twenty he scrolls past.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutlineKey {
    /// `1.1` → `[1, 1]`. Compared element-wise as integers, and a proper prefix
    /// sorts FIRST (`1` before `1.1`), because a parent precedes its children.
    Numbered(Vec<u64>),
    /// No usable number. Sorts last, and ties with every other unnumbered row so
    /// a stable sort preserves their incoming order untouched.
    Unnumbered,
}

/// Parse a stored prefix into its comparable key.
///
/// Accepts `1`, `1.1`, `5.2.3`, and tolerates the trailing dot the owner writes
/// when he types a heading (`"2."`). Anything with a non-numeric segment is
/// [`OutlineKey::Unnumbered`] rather than an error: this function is on the
/// render path for every row, and a row with a malformed prefix must still get
/// a position. Refusing to place it would be exactly the silent breakage the
/// acceptance criteria forbid.
pub fn parse_outline_key(prefix: Option<&str>) -> OutlineKey {
    let Some(prefix) = prefix else {
        return OutlineKey::Unnumbered;
    };
    let trimmed = prefix.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return OutlineKey::Unnumbered;
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('.') {
        let Ok(value) = segment.trim().parse::<u64>() else {
            return OutlineKey::Unnumbered;
        };
        segments.push(value);
    }
    OutlineKey::Numbered(segments)
}

/// Normalise a prefix the way it will be stored and rendered, or refuse it.
///
/// The setter uses this so a row can never hold a prefix the sort cannot read —
/// `"2."` and `" 2 "` both become `"2"`, and `"lobe-2"` is refused BY NAME
/// rather than accepted and then silently sorted last. ⛔ A setter that accepts
/// a value the sort ignores is the "reports the request, not the effect" defect
/// wearing a new hat.
pub fn normalize_outline_prefix(prefix: &str) -> Result<Option<String>, String> {
    let trimmed = prefix.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        // The documented way to CLEAR a prefix, so it is a success, not a
        // refusal.
        return Ok(None);
    }
    match parse_outline_key(Some(trimmed)) {
        OutlineKey::Numbered(segments) => Ok(Some(
            segments
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("."),
        )),
        OutlineKey::Unnumbered => Err(format!(
            "outline prefix {prefix:?} is not a dotted number like 1, 1.1 or 5.2 \
             (segments are compared as integers, so every segment must be one)"
        )),
    }
}

/// The child prefix that follows `parent` given the children it already has.
///
/// `derive_child_outline_prefix(Some("5"), ["5.1"])` → `Some("5.2")`. Used at
/// spawn so the number is a FACT about who spawned whom rather than something
/// an orchestrator types and re-types after every restart.
///
/// Returns `None` when the parent itself is unnumbered — a child of an
/// unnumbered row has no outline position to inherit, and inventing a top-level
/// number for it would move an unrelated row.
pub fn derive_child_outline_prefix(
    parent_prefix: Option<&str>,
    existing_child_prefixes: impl IntoIterator<Item = String>,
) -> Option<String> {
    let OutlineKey::Numbered(parent) = parse_outline_key(parent_prefix) else {
        return None;
    };
    // The next free index among the parent's DIRECT children only: a grandchild
    // (`5.1.1`) must not push the next child to `5.3`.
    let mut highest = 0u64;
    for child in existing_child_prefixes {
        let OutlineKey::Numbered(segments) = parse_outline_key(Some(&child)) else {
            continue;
        };
        if segments.len() == parent.len() + 1 && segments.starts_with(&parent) {
            highest = highest.max(segments[parent.len()]);
        }
    }
    let mut child = parent;
    child.push(highest + 1);
    Some(
        child
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Sort rows into outline order IN PLACE, stably.
///
/// The one sort. Both the rendered sidebar and any verb that reports an order
/// call this, so "what order should these rows be in" has exactly one answer
/// and the verb cannot report an order the sidebar does not draw.
pub fn sort_by_outline<T>(rows: &mut [T], prefix_of: impl Fn(&T) -> Option<String>) {
    // `sort_by_key` is stable in Rust, which is the whole guarantee for
    // unnumbered rows: they all share one key and therefore keep their incoming
    // relative order rather than being shuffled by the sort's internals.
    rows.sort_by_cached_key(|row| parse_outline_key(prefix_of(row).as_deref()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(input: &[&str]) -> Vec<String> {
        let mut rows: Vec<String> = input.iter().map(|value| value.to_string()).collect();
        sort_by_outline(&mut rows, |row| {
            (!row.starts_with('_')).then(|| row.clone())
        });
        rows
    }

    #[test]
    fn the_owners_outline_sorts_the_way_he_reads_it() {
        assert_eq!(
            order(&["6", "1.1", "0", "5.2", "2", "1", "5.1", "4"]),
            vec!["0", "1", "1.1", "2", "4", "5.1", "5.2", "6"]
        );
    }

    /// ⛔ The trap the whole module exists for: a lexicographic sort puts `10`
    /// before `2` and looks correct until the tenth lobe exists.
    #[test]
    fn a_tenth_lobe_does_not_sort_before_the_second() {
        assert_eq!(
            order(&["10", "2", "1.10", "1.2"]),
            vec!["1.2", "1.10", "2", "10"]
        );
    }

    #[test]
    fn a_parent_precedes_its_children() {
        assert_eq!(order(&["1.1", "1", "1.1.1"]), vec!["1", "1.1", "1.1.1"]);
    }

    /// TOTAL and STABLE: rows with no number still get a position, they land
    /// last, and their own relative order is untouched.
    #[test]
    fn unnumbered_rows_land_last_in_the_order_they_arrived() {
        assert_eq!(
            order(&["_zulu", "2", "_alpha", "1"]),
            vec!["1", "2", "_zulu", "_alpha"]
        );
    }

    /// IDEMPOTENT — "sorts are cheap and never break" means a sort of a sorted
    /// list changes nothing, which is what makes it safe to run on every frame.
    #[test]
    fn sorting_a_sorted_list_is_the_identity() {
        let once = order(&["6", "1.1", "_x", "0", "2", "_y"]);
        let borrowed: Vec<&str> = once.iter().map(String::as_str).collect();
        assert_eq!(order(&borrowed), once);
    }

    #[test]
    fn a_malformed_prefix_is_placed_rather_than_dropped() {
        // Nothing is lost, and the unreadable prefix simply sorts with the
        // unnumbered rows instead of taking a numbered seat it cannot justify.
        assert_eq!(parse_outline_key(Some("lobe-2")), OutlineKey::Unnumbered);
        assert_eq!(parse_outline_key(Some("")), OutlineKey::Unnumbered);
        assert_eq!(parse_outline_key(None), OutlineKey::Unnumbered);
        assert_eq!(order(&["2", "lobe-2", "1"]).len(), 3);
    }

    #[test]
    fn a_trailing_dot_is_the_same_position_the_owner_typed() {
        assert_eq!(normalize_outline_prefix("2.").unwrap().as_deref(), Some("2"));
        assert_eq!(
            normalize_outline_prefix(" 5.2 ").unwrap().as_deref(),
            Some("5.2")
        );
        assert_eq!(normalize_outline_prefix("  ").unwrap(), None);
    }

    /// The setter refuses BY NAME rather than storing a value the sort ignores.
    #[test]
    fn a_prefix_the_sort_cannot_read_is_refused_not_accepted() {
        let error = normalize_outline_prefix("lobe-2").unwrap_err();
        assert!(error.contains("lobe-2"), "{error}");
    }

    #[test]
    fn a_child_number_follows_the_siblings_that_exist() {
        assert_eq!(
            derive_child_outline_prefix(Some("5"), ["5.1".into(), "5.2".into()]),
            Some("5.3".to_string())
        );
        assert_eq!(
            derive_child_outline_prefix(Some("5"), []),
            Some("5.1".to_string())
        );
    }

    /// A grandchild is not a sibling: `5.1.1` must not push the next child of
    /// `5` past `5.2`.
    #[test]
    fn a_grandchild_does_not_consume_a_child_number() {
        assert_eq!(
            derive_child_outline_prefix(Some("5"), ["5.1".into(), "5.1.1".into()]),
            Some("5.2".to_string())
        );
    }

    /// A child of an unnumbered row inherits nothing — inventing a top-level
    /// number for it would move a row nobody asked to move.
    #[test]
    fn an_unnumbered_parent_gives_its_child_no_number() {
        assert_eq!(derive_child_outline_prefix(None, []), None);
        assert_eq!(derive_child_outline_prefix(Some("scratch"), []), None);
    }
}

/// The seat a row's TITLE claims, for rows that predate `outline_prefix`.
///
/// ⚖ **This does not weaken the law at the top of this module — read it
/// carefully.** The stored prefix is still the only DURABLE owner, and this is
/// never written back into one. It exists for exactly one job: comparing a
/// brand-new numbered row against a sidebar whose rows were numbered by hand,
/// in prose, before the field existed.
///
/// The defect it closes, reported by the owner on 2026-08-07 in the shape
/// *"cant we inject the session row directly at the start in place instead of
/// moving it"*: spawning `--outline 5.4` into a list of ten rows titled
/// `0. …`, `1. …`, `5.3 …` seated it at the HEAD — correctly, by the rule as
/// written, because every one of those rows read `Unnumbered` and a numbered
/// row sorts ahead of every unnumbered one. The seat was right about a world in
/// which nothing else had a number, and that world was a migration artefact,
/// not the owner's sidebar. Ignoring evidence that is plainly on screen to
/// protect a law about durability is the law serving itself.
///
/// ⛔ **The parse is deliberately TIGHT, because the loose one has already been
/// paid for.** A bare leading integer is NOT enough: a CLI that re-titles a row
/// `2026 audit` would otherwise claim outline `2026` and jump the queue — the
/// same prose-collision that made `compose_outline_prefix`'s bare prefix-match
/// hide a number it was holding. Two shapes only, both of which the owner
/// actually types:
///
///   - a DOTTED number then whitespace — `5.1 gadgets: crypto`
///   - a bare number then a DOT and whitespace — `0. Aug 7 2026`
///
/// `2026 audit` matches neither. A re-title that destroys the number costs the
/// hint and nothing else: the row falls back to `Unnumbered`, which is the
/// behaviour it had before this function existed.
pub fn outline_key_from_title(title: &str) -> OutlineKey {
    let trimmed = title.trim_start();
    let head: String = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    let Some(rest) = trimmed.get(head.len()..) else {
        return OutlineKey::Unnumbered;
    };
    // The claim must END at a separator the owner writes, never mid-word.
    let dotted = head.contains('.') && !head.ends_with('.');
    let bare_with_dot = head.ends_with('.') && head.trim_end_matches('.').parse::<u64>().is_ok();
    if !(dotted || bare_with_dot) {
        return OutlineKey::Unnumbered;
    }
    if !rest.starts_with(char::is_whitespace) {
        return OutlineKey::Unnumbered;
    }
    parse_outline_key(Some(&head))
}

#[cfg(test)]
mod title_seat_hint_tests {
    use super::*;

    /// The owner's live sidebar on 2026-08-07, verbatim, plus the shapes that
    /// must NOT be mistaken for an outline claim.
    #[test]
    fn a_title_claims_a_seat_only_in_the_shapes_he_types() {
        for (title, expected) in [
            ("0. Aug 7 2026", OutlineKey::Numbered(vec![0])),
            ("1. widgets: intake", OutlineKey::Numbered(vec![1])),
            ("5.1 gadgets: vendor research + pipeline", OutlineKey::Numbered(vec![5, 1])),
            ("5.3 gadgets: mark status", OutlineKey::Numbered(vec![5, 3])),
            ("7.2 sprockets: Continue the sweep", OutlineKey::Numbered(vec![7, 2])),
        ] {
            assert_eq!(outline_key_from_title(title), expected, "title: {title}");
        }
        // ⛔ The prose collisions. A year, a version and a bare count are not
        // outline claims, and treating them as one would let a CLI's own
        // re-title jump the queue.
        for title in [
            "2026 audit",
            "3 notes to file",
            "v2 parity work",
            "Continue widgets campaign",
            "",
            "5.1gadgets",
        ] {
            assert_eq!(
                outline_key_from_title(title),
                OutlineKey::Unnumbered,
                "title must not claim a seat: {title}"
            );
        }
    }

    /// The hint must never outrank the stored fact — it only fills a hole.
    #[test]
    fn the_hint_orders_the_owners_sidebar_the_way_he_reads_it() {
        let mut keys: Vec<OutlineKey> = ["6. yggterm", "0. Aug 7 2026", "5.3 gadgets", "5.10 later", "5.2 gadgets"]
            .iter()
            .map(|title| outline_key_from_title(title))
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                OutlineKey::Numbered(vec![0]),
                OutlineKey::Numbered(vec![5, 2]),
                OutlineKey::Numbered(vec![5, 3]),
                // 10 after 2, as integers — the trap this module was built for.
                OutlineKey::Numbered(vec![5, 10]),
                OutlineKey::Numbered(vec![6]),
            ]
        );
    }
}
