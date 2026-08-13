//! ROW SETS — the containment relation behind collapsible groups of live rows.
//!
//! A **row set** is a set of live-session rows collected under one of them and
//! collapsible as a unit. `DESIGN.md` §"Row sets" is the contract; this module
//! is the model it describes, and nothing here decides how anything is drawn.
//!
//! ⛔ **THE NOUN.** Terminal splits are already **groups** and the cwd tree has
//! **folders**; a third meaning of either word makes every future bug report
//! ambiguous. `section` is taken too (`AppPaneWidget::Section`, and "section
//! cards" in `DESIGN.md`). So: **row set**, everywhere, in code and in the CLI.
//!
//! ⭐ **A ROW SET *IS* ITS HEAD — there is no separate set id.** The head is an
//! ordinary row that happens to have members, and a member may itself be a head,
//! which is what makes nesting fall out rather than being bolted on. The
//! alternative — minting ids — would create a second name for a row and a second
//! thing to keep in step with the row's own identity, which is the failure the
//! project's single-source-of-truth law exists to prevent.
//!
//! ⚠ **A row set means NOTHING but arrangement.** No ownership, no lifecycle, no
//! supervision. Membership is arbitrary: the outline numbers are the *default*
//! arrangement and never a restriction on it, so nothing in this module may
//! consult a seat number, and nothing outside it may read membership to decide
//! behaviour.
//!
//! Three invariants, all enforced here rather than trusted:
//!
//! 1. **One parent.** Adding a row to a set removes it from whatever set held
//!    it. A row in two sets has no defined place in the order.
//! 2. **No cycles.** A head may not become a descendant of itself, however many
//!    hops away. A cycle makes [`RowSets::visible_rows`] non-terminating, so it
//!    is refused at the edge rather than defended against on the render path.
//! 3. **Collapse state is INDEPENDENT of visibility.** Collapsing an outer set
//!    hides its inner sets without touching their own flags, so re-expanding
//!    restores each exactly as it was. ⛔ Flattening the inner sets open on
//!    expand is the failure users notice, and it is the one that gets skipped.

use std::collections::{HashMap, HashSet};

/// Why an arrangement was refused. Named rather than a bare `bool` because each
/// of these is a different mistake and the caller reports them differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSetRefusal {
    /// A row cannot be its own head.
    SelfParent,
    /// `head` is already somewhere beneath `member`, so this would close a loop.
    /// Carries the path that would have been re-entered.
    WouldCycle { through: String },
}

/// The containment relation over row paths, plus each set's collapsed flag.
///
/// Empty is the normal state: a sidebar with no row sets holds no entries here
/// and every row is top level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowSets {
    /// head path → its members, in the order they are drawn under it.
    members: HashMap<String, Vec<String>>,
    /// member path → the head that holds it. Derived from `members` and kept in
    /// step by every mutator, so the "one parent" invariant is a lookup rather
    /// than a scan.
    parent: HashMap<String, String>,
    /// Heads the user has collapsed. A head not listed here is expanded, so a
    /// brand-new set is open — a set that hid its members the moment it was
    /// created would look like the rows had been deleted.
    collapsed: HashSet<String>,
}

impl RowSets {
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The head that holds `path`, if any.
    pub fn parent_of(&self, path: &str) -> Option<&str> {
        self.parent.get(path).map(String::as_str)
    }

    /// `path`'s members in draw order. Empty for a row that heads nothing —
    /// which is every row, until someone arranges one.
    pub fn members_of(&self, path: &str) -> &[String] {
        self.members.get(path).map_or(&[], Vec::as_slice)
    }

    /// Does `path` head a set? A head with an empty member list is not a set:
    /// the last member leaving dissolves it, so an empty husk can never linger
    /// and offer a disclosure control that opens onto nothing.
    pub fn is_head(&self, path: &str) -> bool {
        self.members.get(path).is_some_and(|m| !m.is_empty())
    }

    pub fn is_collapsed(&self, head: &str) -> bool {
        self.collapsed.contains(head)
    }

    /// Collapse or expand ONE set. Touches no other set's flag — that is the
    /// whole point of storing the flags apart from the containment.
    pub fn set_collapsed(&mut self, head: &str, collapsed: bool) {
        if collapsed {
            self.collapsed.insert(head.to_string());
        } else {
            self.collapsed.remove(head);
        }
    }

    /// Put `member` into `head`'s set at `index` (clamped; `None` appends).
    ///
    /// Detaches `member` from any set that held it first, so the one-parent
    /// invariant holds by construction rather than by the caller remembering.
    pub fn insert_member(
        &mut self,
        head: &str,
        member: &str,
        index: Option<usize>,
    ) -> Result<(), RowSetRefusal> {
        if head == member {
            return Err(RowSetRefusal::SelfParent);
        }
        // ⛔ The cycle check runs BEFORE the detach. Checking afterwards would
        // read a relation this call has already half-changed, and refuse from a
        // state neither the caller nor the store ever agreed to be in.
        if let Some(through) = self.first_ancestor_at_or_below(head, member) {
            return Err(RowSetRefusal::WouldCycle { through });
        }
        self.detach(member);
        let slot = self.members.entry(head.to_string()).or_default();
        let at = index.map_or(slot.len(), |index| index.min(slot.len()));
        slot.insert(at, member.to_string());
        self.parent.insert(member.to_string(), head.to_string());
        Ok(())
    }

    /// Take `member` out of whatever set holds it. It becomes top level; its own
    /// members and collapsed flag come with it, because leaving a set says
    /// nothing about what it holds.
    pub fn detach(&mut self, member: &str) {
        let Some(head) = self.parent.remove(member) else {
            return;
        };
        if let Some(slot) = self.members.get_mut(&head) {
            slot.retain(|path| path != member);
            if slot.is_empty() {
                self.members.remove(&head);
                self.collapsed.remove(&head);
            }
        }
    }

    /// The head is going away — the set DISSOLVES and its members take its
    /// place, in order, wherever it sat.
    ///
    /// ⚖ Not a refusal, and not a cascade. Closing a session is a request about
    /// that session; answering it with a question about bookkeeping, or by
    /// silently taking the members down too, are both worse than promoting them.
    /// Returns the promoted members so the caller can report what moved.
    pub fn dissolve(&mut self, head: &str) -> Vec<String> {
        let promoted = self.members.remove(head).unwrap_or_default();
        self.collapsed.remove(head);
        let grandparent = self.parent.remove(head);
        for member in &promoted {
            self.parent.remove(member);
        }
        if let Some(grandparent) = grandparent {
            if let Some(slot) = self.members.get_mut(&grandparent) {
                let at = slot
                    .iter()
                    .position(|path| path == head)
                    .unwrap_or(slot.len());
                slot.remove(at.min(slot.len().saturating_sub(1)));
                for (offset, member) in promoted.iter().enumerate() {
                    slot.insert((at + offset).min(slot.len()), member.clone());
                }
                for member in &promoted {
                    self.parent.insert(member.clone(), grandparent.clone());
                }
                if slot.is_empty() {
                    self.members.remove(&grandparent);
                    self.collapsed.remove(&grandparent);
                }
            }
        }
        promoted
    }

    /// Is `path` hidden by an ANCESTOR being collapsed?
    ///
    /// ⚠ A head's own collapsed flag does not hide the head — it hides what is
    /// under it. A row that hid itself when collapsed could never be reopened.
    pub fn is_hidden(&self, path: &str) -> bool {
        let mut current = self.parent_of(path);
        let mut guard = 0usize;
        while let Some(head) = current {
            if self.is_collapsed(head) {
                return true;
            }
            guard += 1;
            if guard > self.parent.len() {
                // Unreachable while `insert_member` refuses cycles; the render
                // path still may not hang if a persisted file was hand-edited.
                return false;
            }
            current = self.parent_of(head);
        }
        false
    }

    /// `top_level` is the sidebar's own order for the rows that belong to no
    /// set. Returns the rows to draw, each with its nesting depth, expanding
    /// each set in place and stopping at any collapsed head.
    ///
    /// The caller supplies the top-level order because this module does not own
    /// it: Live Sessions order is durable user layout with its own owner, and a
    /// row set rearranges nothing about it.
    pub fn visible_rows<'a>(
        &'a self,
        top_level: impl IntoIterator<Item = &'a str>,
    ) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for path in top_level {
            if self.parent_of(path).is_some() {
                // Drawn under its head, not at the top level.
                continue;
            }
            self.push_visible(path, 0, &mut out, &mut seen);
        }
        out
    }

    fn push_visible(
        &self,
        path: &str,
        depth: usize,
        out: &mut Vec<(String, usize)>,
        seen: &mut HashSet<String>,
    ) {
        if !seen.insert(path.to_string()) {
            return;
        }
        out.push((path.to_string(), depth));
        if self.is_collapsed(path) {
            return;
        }
        for member in self.members_of(path) {
            self.push_visible(member, depth + 1, out, seen);
        }
    }

    /// The first path on `from`'s ancestor chain (inclusive) that is `target`,
    /// i.e. "would linking these close a loop".
    fn first_ancestor_at_or_below(&self, from: &str, target: &str) -> Option<String> {
        let mut current = Some(from);
        let mut guard = 0usize;
        while let Some(path) = current {
            if path == target {
                return Some(path.to_string());
            }
            guard += 1;
            if guard > self.parent.len() + 1 {
                return None;
            }
            current = self.parent_of(path);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrange(pairs: &[(&str, &str)]) -> RowSets {
        let mut sets = RowSets::default();
        for (head, member) in pairs {
            sets.insert_member(head, member, None).expect("arrangeable");
        }
        sets
    }

    #[test]
    fn a_set_draws_its_members_under_its_head_and_collapses_them_away() {
        let mut sets = arrange(&[("a", "b"), ("a", "c")]);
        let order = ["a", "z"];
        assert_eq!(
            sets.visible_rows(order),
            vec![
                ("a".into(), 0),
                ("b".into(), 1),
                ("c".into(), 1),
                ("z".into(), 0)
            ]
        );
        sets.set_collapsed("a", true);
        // The head stays; a row that hid itself could never be reopened.
        assert_eq!(
            sets.visible_rows(order),
            vec![("a".into(), 0), ("z".into(), 0)]
        );
    }

    /// ⛔ THE ONE USERS NOTICE. Collapsing an outer set must not disturb the
    /// inner flags, so re-expanding restores each inner set as it was rather
    /// than flattening them all open.
    #[test]
    fn an_outer_collapse_preserves_every_inner_sets_own_state() {
        let mut sets = arrange(&[("a", "b"), ("b", "c"), ("a", "d"), ("d", "e")]);
        sets.set_collapsed("b", true);
        let order = ["a"];
        assert_eq!(
            sets.visible_rows(order),
            vec![
                ("a".into(), 0),
                ("b".into(), 1),
                ("d".into(), 1),
                ("e".into(), 2)
            ],
            "b is shut, d is open"
        );

        sets.set_collapsed("a", true);
        assert_eq!(sets.visible_rows(order), vec![("a".into(), 0)]);
        // The inner flags are untouched while hidden.
        assert!(sets.is_collapsed("b"));
        assert!(!sets.is_collapsed("d"));

        sets.set_collapsed("a", false);
        assert_eq!(
            sets.visible_rows(order),
            vec![
                ("a".into(), 0),
                ("b".into(), 1),
                ("d".into(), 1),
                ("e".into(), 2)
            ],
            "re-expanding restores the inner sets, it does not flatten them open"
        );
    }

    #[test]
    fn membership_is_arbitrary_and_a_row_has_exactly_one_parent() {
        // Nothing here consults a seat number: any row may join any set.
        let mut sets = arrange(&[("a", "b")]);
        sets.insert_member("z", "b", None).expect("re-homeable");
        assert_eq!(sets.parent_of("b"), Some("z"));
        assert!(sets.members_of("a").is_empty());
        // The emptied head is no longer a set — no husk offering a disclosure
        // control that opens onto nothing.
        assert!(!sets.is_head("a"));
    }

    #[test]
    fn a_cycle_is_refused_at_the_edge_rather_than_defended_against_on_the_render_path() {
        let mut sets = arrange(&[("a", "b"), ("b", "c")]);
        assert_eq!(
            sets.insert_member("c", "a", None),
            Err(RowSetRefusal::WouldCycle { through: "a".into() })
        );
        assert_eq!(
            sets.insert_member("a", "a", None),
            Err(RowSetRefusal::SelfParent)
        );
        // ⛔ And the refusal leaves the relation exactly as it was — a check
        // that ran after the detach would strand `a` outside the set it was in.
        assert_eq!(sets.parent_of("b"), Some("a"));
        assert_eq!(sets.parent_of("c"), Some("b"));
    }

    /// Removing a non-empty set's head promotes its members in order, into the
    /// head's own place. Refusing would answer a bookkeeping question the user
    /// did not ask; cascading would delete rows they never named.
    #[test]
    fn removing_a_head_dissolves_the_set_and_promotes_its_members_in_place() {
        let mut sets = arrange(&[("outer", "head"), ("outer", "tail"), ("head", "one"), ("head", "two")]);
        let promoted = sets.dissolve("head");
        assert_eq!(promoted, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(
            sets.visible_rows(["outer"]),
            vec![
                ("outer".into(), 0),
                ("one".into(), 1),
                ("two".into(), 1),
                ("tail".into(), 1)
            ],
            "the members take the head's place, in order, ahead of what followed it"
        );
        assert_eq!(sets.parent_of("one"), Some("outer"));
    }

    #[test]
    fn dissolving_a_top_level_head_leaves_its_members_top_level() {
        let mut sets = arrange(&[("head", "one"), ("head", "two")]);
        assert_eq!(sets.dissolve("head"), vec!["one".to_string(), "two".to_string()]);
        assert!(sets.is_empty());
        assert_eq!(
            sets.visible_rows(["one", "two"]),
            vec![("one".into(), 0), ("two".into(), 0)]
        );
    }

    /// The sidebar's own order owns the top level; a row set rearranges nothing
    /// about it, and a member is drawn under its head rather than twice.
    #[test]
    fn the_top_level_order_is_the_callers_and_a_member_is_never_drawn_twice() {
        let sets = arrange(&[("b", "a")]);
        assert_eq!(
            sets.visible_rows(["a", "b", "c"]),
            vec![("b".into(), 0), ("a".into(), 1), ("c".into(), 0)],
            "`a` is drawn under `b`, not at the position the flat order gave it"
        );
    }

    #[test]
    fn no_arrangement_at_all_is_the_ordinary_case_and_costs_nothing() {
        let sets = RowSets::default();
        assert!(sets.is_empty());
        assert!(!sets.is_hidden("anything"));
        assert_eq!(
            sets.visible_rows(["a", "b"]),
            vec![("a".into(), 0), ("b".into(), 0)]
        );
    }

    #[test]
    fn a_hand_edited_cycle_cannot_hang_the_render_path() {
        // `insert_member` refuses cycles, so this state is only reachable by
        // editing the persisted file. It must still terminate.
        let mut sets = arrange(&[("a", "b")]);
        sets.parent.insert("a".into(), "b".into());
        sets.members.entry("b".into()).or_default().push("a".into());
        let _ = sets.is_hidden("a");
        let _ = sets.visible_rows(["a"]);
    }
}
