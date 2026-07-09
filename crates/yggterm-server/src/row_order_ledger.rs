//! Durable Live Sessions row-order ledger, scoped per client.
//!
//! `live_session_order` (in [`crate::YggtermServer`]) is the single source of
//! truth for the order of rows that are live RIGHT NOW. What it cannot answer
//! is "where should this row go when it comes back?" — a row that leaves the
//! live set (runtime exit, restart demotion, manual close + reconnect) loses
//! its slot and re-enters at the daemon-native position. The ledger is the
//! daemon-owned memory of row slots that outlives liveness.
//!
//! Scopes: multiple yggterm GUIs (and headless clients) can attach to the same
//! host daemon, and each may keep its own row arrangement. A scope is a stable
//! client identity string (e.g. `gui:jojo:/home/pi/.yggterm`); the daemon
//! stores one ordered ledger per scope, so a session can hold a slot in
//! several clients' ledgers at once. The [`SHARED_ROW_ORDER_SCOPE`] scope is
//! the daemon-native order every order mutation also records into; clients
//! that never declare a scope simply live on the shared ledger.
//!
//! Ownership: the ledger observes and remembers order — it never mutates
//! `live_session_order` itself. Placement decisions are returned to the
//! daemon request handlers, which apply them through the existing order
//! primitives, so there is exactly one writer of live order.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Scope every order mutation records into, and the fallback scope for
/// placement lookups from clients that never declared an identity.
pub const SHARED_ROW_ORDER_SCOPE: &str = "shared";

/// Upper bound of remembered rows per scope. Non-live rows beyond the cap are
/// dropped from the tail — the ledger remembers arrangements, it is not an
/// archive.
const MAX_ROWS_PER_SCOPE: usize = 1000;

const LEDGER_FILE_NAME: &str = "row-order-ledger.json";

/// Where a row (re)entering the live set should land, per the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowLedgerPlacement {
    /// The ledger remembers this row at the top (or every remembered
    /// predecessor is gone).
    Front,
    /// Place directly below this currently-live row.
    AfterLive(String),
    /// The ledger has never seen this row in this scope (or its fallback):
    /// keep the caller's native placement.
    Unknown,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RowOrderLedger {
    /// scope -> ordered session paths (live and remembered-non-live mixed).
    scopes: BTreeMap<String, Vec<String>>,
}

impl RowOrderLedger {
    pub fn ledger_path(home_dir: &Path) -> PathBuf {
        home_dir.join(LEDGER_FILE_NAME)
    }

    pub fn load(home_dir: &Path) -> Self {
        let path = Self::ledger_path(home_dir);
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, home_dir: &Path) -> Result<()> {
        let path = Self::ledger_path(home_dir);
        let raw = serde_json::to_string_pretty(self).context("serializing row-order ledger")?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
    }

    pub fn scope_rows(&self, scope: &str) -> &[String] {
        self.scopes
            .get(scope)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn scope_names(&self) -> impl Iterator<Item = &str> {
        self.scopes.keys().map(String::as_str)
    }

    /// Record the current live order into `scope`, preserving the slots of
    /// remembered rows that are NOT currently live: each keeps its position
    /// relative to the nearest preceding remembered row that is still in the
    /// live order (front-anchored when there is none). Returns true when the
    /// scope's ledger changed.
    pub fn record_live_order(&mut self, scope: &str, live_order: &[String]) -> bool {
        let live_set: HashSet<&str> = live_order.iter().map(String::as_str).collect();
        let old = self.scopes.get(scope).cloned().unwrap_or_default();

        // anchor (None = front) -> non-live rows remembered directly after it,
        // in their old relative order.
        let mut absent_after: BTreeMap<Option<usize>, Vec<String>> = BTreeMap::new();
        let mut last_live_anchor: Option<&str> = None;
        let mut seen: HashSet<&str> = HashSet::new();
        for entry in &old {
            if !seen.insert(entry.as_str()) {
                continue;
            }
            if live_set.contains(entry.as_str()) {
                last_live_anchor = Some(entry.as_str());
            } else {
                let anchor_ix = last_live_anchor
                    .and_then(|anchor| live_order.iter().position(|row| row == anchor));
                absent_after.entry(anchor_ix).or_default().push(entry.clone());
            }
        }

        let mut next = Vec::with_capacity(old.len().max(live_order.len()));
        if let Some(front_rows) = absent_after.get(&None) {
            next.extend(front_rows.iter().cloned());
        }
        for (ix, row) in live_order.iter().enumerate() {
            next.push(row.clone());
            if let Some(rows) = absent_after.get(&Some(ix)) {
                next.extend(rows.iter().cloned());
            }
        }
        // Cap: drop non-live remembered rows from the tail first, then hard-cap.
        while next.len() > MAX_ROWS_PER_SCOPE {
            if let Some(pos) = next.iter().rposition(|row| !live_set.contains(row.as_str())) {
                next.remove(pos);
            } else {
                next.truncate(MAX_ROWS_PER_SCOPE);
            }
        }

        if self.scopes.get(scope).is_some_and(|rows| rows == &next)
            || (next.is_empty() && !self.scopes.contains_key(scope))
        {
            return false;
        }
        self.scopes.insert(scope.to_string(), next);
        true
    }

    /// Where should `path`, about to (re)enter the live set, land? Looks up
    /// `scope` first and falls back to the shared scope when the row is
    /// unknown there. `is_live` answers whether a remembered predecessor is
    /// currently in the live order.
    pub fn placement_for(
        &self,
        scope: &str,
        path: &str,
        is_live: impl Fn(&str) -> bool,
    ) -> RowLedgerPlacement {
        for candidate_scope in [scope, SHARED_ROW_ORDER_SCOPE] {
            let rows = self.scope_rows(candidate_scope);
            let Some(row_ix) = rows.iter().position(|row| row == path) else {
                continue;
            };
            for anchor in rows[..row_ix].iter().rev() {
                if anchor != path && is_live(anchor) {
                    return RowLedgerPlacement::AfterLive(anchor.clone());
                }
            }
            return RowLedgerPlacement::Front;
        }
        RowLedgerPlacement::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn record_preserves_non_live_slots_between_live_neighbors() {
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order("shared", &rows(&["a", "b", "c", "d"]));
        // b leaves the live set; the ledger keeps its slot (an unchanged
        // ledger legitimately reports false).
        assert!(!ledger.record_live_order("shared", &rows(&["a", "c", "d"])));
        assert_eq!(ledger.scope_rows("shared"), rows(&["a", "b", "c", "d"]));
        // A reorder of the live rows keeps b anchored below a.
        assert!(ledger.record_live_order("shared", &rows(&["c", "a", "d"])));
        assert_eq!(ledger.scope_rows("shared"), rows(&["c", "a", "b", "d"]));
    }

    #[test]
    fn record_remembers_front_row_that_left() {
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order("shared", &rows(&["top", "mid", "low"]));
        ledger.record_live_order("shared", &rows(&["mid", "low"]));
        assert_eq!(ledger.scope_rows("shared"), rows(&["top", "mid", "low"]));
    }

    #[test]
    fn placement_restores_remembered_slot() {
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order("shared", &rows(&["a", "b", "c"]));
        ledger.record_live_order("shared", &rows(&["a", "c"]));
        let live = rows(&["a", "c"]);
        let is_live = |row: &str| live.iter().any(|entry| entry == row);
        assert_eq!(
            ledger.placement_for("shared", "b", is_live),
            RowLedgerPlacement::AfterLive("a".to_string())
        );
        assert_eq!(
            ledger.placement_for("shared", "unknown-row", is_live),
            RowLedgerPlacement::Unknown
        );
    }

    #[test]
    fn placement_front_when_all_predecessors_gone() {
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order("shared", &rows(&["a", "b"]));
        ledger.record_live_order("shared", &rows(&["a"]));
        // a also leaves; only remembered rows remain.
        ledger.record_live_order("shared", &rows(&[]));
        let is_live = |_: &str| false;
        assert_eq!(
            ledger.placement_for("shared", "a", is_live),
            RowLedgerPlacement::Front
        );
    }

    #[test]
    fn per_scope_orders_are_independent_with_shared_fallback() {
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order("shared", &rows(&["a", "b", "c"]));
        ledger.record_live_order("gui:jojo", &rows(&["c", "b", "a"]));
        assert_eq!(ledger.scope_rows("gui:jojo"), rows(&["c", "b", "a"]));
        assert_eq!(ledger.scope_rows("shared"), rows(&["a", "b", "c"]));
        // A row only the shared scope knows falls back for placement.
        ledger.record_live_order("shared", &rows(&["a", "b", "c", "d"]));
        let live = rows(&["a", "b", "c"]);
        let is_live = |row: &str| live.iter().any(|entry| entry == row);
        assert_eq!(
            ledger.placement_for("gui:jojo", "d", is_live),
            RowLedgerPlacement::AfterLive("c".to_string())
        );
    }

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("yggterm-ledger-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order("gui:jojo", &rows(&["a", "b"]));
        ledger.save(&dir).expect("save");
        let loaded = RowOrderLedger::load(&dir);
        assert_eq!(loaded.scope_rows("gui:jojo"), rows(&["a", "b"]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
