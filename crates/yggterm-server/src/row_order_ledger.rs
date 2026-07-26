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
//! client identity string (e.g. `gui:jojo:/home/user/.yggterm`); the daemon
//! stores one ordered ledger per scope, so a session can hold a slot in
//! several clients' ledgers at once. The [`SHARED_ROW_ORDER_SCOPE`] scope is
//! the daemon-native order every order mutation also records into; clients
//! that never declare a scope simply live on the shared ledger.
//!
//! Ownership: the ledger observes and remembers order — it never mutates
//! `live_session_order` itself. Placement decisions are returned to the
//! daemon request handlers, which apply them through the existing order
//! primitives, so there is exactly one writer of live order.
//!
//! # The restore half (2026-07-26)
//!
//! Recording was only ever half the feature. Across the 2.12.15 daemon bump the
//! ledger came out byte-identical (143 entries, the user's curated order intact)
//! and *nothing read it back*: rows restored from the state file landed first,
//! rows adopted from peer daemons were woven in after, and the user's two live
//! sessions moved from positions 1-2 to 6-7 — the third hand re-curation of that
//! sidebar in one day. [`reconcile_order_with_remembered`] is the missing read:
//! it takes the order a rebuild produced and the order the user left, and gives
//! back the user's order for every row the ledger knows while leaving rows the
//! ledger has never seen exactly where the rebuild anchored them.
//!
//! It is a PERMUTATION of the rebuilt order and nothing else. It cannot add a
//! row, which is what makes it safe next to `live_row_tombstones` — a closed row
//! sitting in the ledger (the ledger deliberately remembers non-live rows) can
//! never be resurrected by the restore, because the restore only ever emits rows
//! the caller already holds.

use std::collections::BTreeMap;
use std::collections::HashMap;
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

/// Where the pre-swap row-order snapshots land, next to the hand-written
/// `pre-gui-restart-*` snapshots an agent used to have to make by hand.
const MANUAL_SNAPSHOT_DIR_NAME: &str = "manual-snapshots";

/// Filename prefix of a pre-swap snapshot. Public so tooling can list them
/// without re-spelling the convention.
pub const PRE_DAEMON_SWAP_SNAPSHOT_PREFIX: &str = "pre-daemon-swap-";

/// How many pre-swap snapshots to keep. A daemon chain can swap several times a
/// day; the snapshot is a safety net for the last few swaps, not an archive.
const MAX_PRE_DAEMON_SWAP_SNAPSHOTS: usize = 32;

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

/// Reconcile a freshly rebuilt live order against the order the user left.
///
/// The rule, and the ONE place it is decided:
/// * a row the ledger remembers takes the LEDGER's relative order;
/// * a row the ledger has never seen keeps its ANCHORED placement — it stays
///   immediately after the same remembered row it followed in `current`
///   (front-anchored when it followed none), and rows sharing an anchor keep
///   their relative order from `current`.
///
/// This is the exact inverse of [`RowOrderLedger::record_live_order`], which
/// anchors remembered-but-not-live rows against their live neighbours. Both
/// halves therefore agree about what "kept its slot" means.
///
/// **The output is always a permutation of `current`** (deduplicated). It never
/// emits a row `current` does not contain, so a ledger entry for a row that was
/// closed — the ledger remembers non-live rows on purpose — cannot resurrect it,
/// and `live_row_tombstones` is not bypassed by this path.
pub fn reconcile_order_with_remembered(current: &[String], remembered: &[String]) -> Vec<String> {
    if current.is_empty() || remembered.is_empty() {
        return current.to_vec();
    }
    let mut remembered_rank: HashMap<&str, usize> = HashMap::new();
    for (rank, row) in remembered.iter().enumerate() {
        remembered_rank.entry(row.as_str()).or_insert(rank);
    }

    let mut known: Vec<String> = Vec::new();
    let mut front_unknown: Vec<String> = Vec::new();
    let mut unknown_after: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut last_known: Option<String> = None;
    for row in current {
        if !seen.insert(row.as_str()) {
            continue;
        }
        if remembered_rank.contains_key(row.as_str()) {
            known.push(row.clone());
            last_known = Some(row.clone());
        } else {
            match last_known.as_deref() {
                Some(anchor) => unknown_after
                    .entry(anchor.to_string())
                    .or_default()
                    .push(row.clone()),
                None => front_unknown.push(row.clone()),
            }
        }
    }

    // Rank is the ledger's first occurrence of the row, so it is unique per
    // known row and the sort is total — no tie-breaking, no non-determinism.
    known.sort_by_key(|row| remembered_rank[row.as_str()]);

    let mut next = Vec::with_capacity(current.len());
    next.append(&mut front_unknown);
    for row in known {
        if let Some(trailing) = unknown_after.remove(row.as_str()) {
            next.push(row);
            next.extend(trailing);
        } else {
            next.push(row);
        }
    }
    next
}

/// Put a rebuilt live-row order back the way the user left it.
///
/// THE one application point of [`reconcile_order_with_remembered`]: it reads
/// the rebuilt order out of the server, reconciles it against `remembered`, and
/// hands the result to `replace_live_session_order` — the single writer of live
/// order — so the restore adds no ordering primitive of its own. A no-op
/// reconcile does not touch the server at all.
///
/// `remembered` must be the ledger as it stood BEFORE this daemon started
/// recording (see `DaemonRuntime::booted_with_row_order`); reconciling against a
/// ledger this daemon has already overwritten with the scramble is a no-op by
/// construction.
pub(crate) fn restore_live_row_order(
    server: &mut crate::YggtermServer,
    remembered: &[String],
) -> crate::LiveSessionOrderUpdate {
    let current = server.live_session_order_keys().to_vec();
    let reconciled = reconcile_order_with_remembered(&current, remembered);
    if reconciled == current {
        return crate::LiveSessionOrderUpdate::default();
    }
    server.replace_live_session_order(&reconciled)
}

/// A pre-swap record of the sidebar's row order, written just before a daemon
/// bump/handover can disturb it.
///
/// The user asked for this by name: *"If destroyed this order is supposed to be
/// snapshotted properly"*, after hand-re-curating the sidebar twice in a day and
/// finding that only a manually-made `pre-gui-restart-*` snapshot existed. The
/// restore path ([`reconcile_order_with_remembered`]) is the automatic recovery;
/// this file is the receipt that makes a manual one possible when the automatic
/// one is not enough.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreDaemonSwapRowOrderSnapshot {
    pub captured_at_unix: u64,
    pub reason: String,
    pub server_version: String,
    pub server_pid: u32,
    /// The daemon's live-row order at capture time, in sidebar order.
    pub live_order: Vec<String>,
    /// The whole ledger, every scope, exactly as it stood.
    pub ledger: RowOrderLedger,
}

fn pre_daemon_swap_snapshot_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(MANUAL_SNAPSHOT_DIR_NAME)
}

/// Write a pre-swap row-order snapshot into
/// `<home>/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json` and prune
/// the directory back to [`MAX_PRE_DAEMON_SWAP_SNAPSHOTS`] pre-swap files.
///
/// The pid is part of the name because a daemon chain can have two daemons
/// preparing inside the same second, and a snapshot that overwrites another
/// daemon's snapshot is the bug this exists to prevent, one layer down.
/// Hand-written `pre-gui-restart-*` snapshots share the directory and are never
/// pruned — the prefix scopes the sweep.
pub fn write_pre_daemon_swap_row_order_snapshot(
    home_dir: &Path,
    snapshot: &PreDaemonSwapRowOrderSnapshot,
) -> Result<PathBuf> {
    let dir = pre_daemon_swap_snapshot_dir(home_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!(
        "{PRE_DAEMON_SWAP_SNAPSHOT_PREFIX}{}-{}.json",
        snapshot.captured_at_unix, snapshot.server_pid
    ));
    let raw =
        serde_json::to_string_pretty(snapshot).context("serializing pre-daemon-swap snapshot")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    prune_pre_daemon_swap_row_order_snapshots(&dir);
    Ok(path)
}

/// Keep only the newest [`MAX_PRE_DAEMON_SWAP_SNAPSHOTS`] pre-swap snapshots.
/// Best-effort: a failed sweep must never fail the swap it was recording.
fn prune_pre_daemon_swap_row_order_snapshots(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(PRE_DAEMON_SWAP_SNAPSHOT_PREFIX) && name.ends_with(".json")
                })
        })
        .collect();
    if names.len() <= MAX_PRE_DAEMON_SWAP_SNAPSHOTS {
        return;
    }
    // Fixed-width unix seconds sort chronologically as text until the year 2286.
    names.sort();
    let excess = names.len() - MAX_PRE_DAEMON_SWAP_SNAPSHOTS;
    for path in names.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
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

    /// The 2.12.15 shape, exactly: the ledger holds the user's curated order,
    /// the handover rebuild produced restored rows first and adopted rows woven
    /// in after, and the two live rows had fallen from the top to the middle.
    /// Reconciling must hand the curated order back — and must leave a row the
    /// ledger has never seen where the anchored import walk put it.
    #[test]
    fn reconcile_restores_ledger_order_and_anchors_unknown_rows() {
        let remembered = rows(&["live-1", "live-2", "old-a", "old-b", "old-c"]);
        let rebuilt = rows(&["old-a", "old-b", "adopted-new", "old-c", "live-1", "live-2"]);
        assert_eq!(
            reconcile_order_with_remembered(&rebuilt, &remembered),
            rows(&["live-1", "live-2", "old-a", "old-b", "adopted-new", "old-c"]),
            "ledger rows take the ledger's order; the unknown row stays under old-b"
        );
    }

    #[test]
    fn reconcile_keeps_a_front_anchored_unknown_row_at_the_front() {
        let remembered = rows(&["a", "b"]);
        let rebuilt = rows(&["fresh", "b", "a"]);
        assert_eq!(
            reconcile_order_with_remembered(&rebuilt, &remembered),
            rows(&["fresh", "a", "b"]),
            "a row ahead of every remembered row is front-anchored, mirroring record_live_order"
        );
    }

    /// The restore is a PERMUTATION and nothing else. The ledger deliberately
    /// remembers rows that are not live, including rows the user CLOSED (whose
    /// closes `live_row_tombstones` vetoes on every import path) — so if the
    /// reconcile could emit a remembered row the caller does not hold, the
    /// restore would become a resurrection door that bypasses the veto entirely.
    #[test]
    fn reconcile_never_emits_a_remembered_row_the_caller_does_not_hold() {
        let remembered = rows(&["a", "closed-by-user", "b"]);
        let rebuilt = rows(&["b", "a"]);
        let reconciled = reconcile_order_with_remembered(&rebuilt, &remembered);
        assert_eq!(reconciled, rows(&["a", "b"]));
        assert!(
            !reconciled.iter().any(|row| row == "closed-by-user"),
            "the closed row is in the ledger and must stay out of the restored order"
        );
    }

    #[test]
    fn reconcile_is_the_identity_when_the_ledger_knows_nothing() {
        let rebuilt = rows(&["a", "b", "c"]);
        assert_eq!(
            reconcile_order_with_remembered(&rebuilt, &[]),
            rebuilt,
            "an empty ledger must not reshuffle anything"
        );
        assert_eq!(
            reconcile_order_with_remembered(&rebuilt, &rows(&["x", "y"])),
            rebuilt,
            "a ledger with no rows in common must not reshuffle anything"
        );
    }

    #[test]
    fn reconcile_round_trips_against_record_live_order() {
        // record → scramble → reconcile must land back on what was recorded.
        let mut ledger = RowOrderLedger::default();
        let curated = rows(&["r1", "r2", "r3", "r4"]);
        ledger.record_live_order(SHARED_ROW_ORDER_SCOPE, &curated);
        let scrambled = rows(&["r3", "r1", "r4", "r2"]);
        assert_eq!(
            reconcile_order_with_remembered(&scrambled, ledger.scope_rows(SHARED_ROW_ORDER_SCOPE)),
            curated
        );
    }

    #[test]
    fn pre_daemon_swap_snapshot_writes_and_prunes() {
        let dir =
            std::env::temp_dir().join(format!("yggterm-preswap-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp home");
        let mut ledger = RowOrderLedger::default();
        ledger.record_live_order(SHARED_ROW_ORDER_SCOPE, &rows(&["a", "b"]));

        let mut written = Vec::new();
        for index in 0..(MAX_PRE_DAEMON_SWAP_SNAPSHOTS + 3) {
            let snapshot = PreDaemonSwapRowOrderSnapshot {
                captured_at_unix: 1_700_000_000 + index as u64,
                reason: "prepare_update_restart".to_string(),
                server_version: "2.12.15".to_string(),
                server_pid: 4242,
                live_order: rows(&["a", "b"]),
                ledger: ledger.clone(),
            };
            written.push(
                write_pre_daemon_swap_row_order_snapshot(&dir, &snapshot).expect("write snapshot"),
            );
        }

        let last = written.last().expect("at least one snapshot");
        let name = last.file_name().and_then(|name| name.to_str()).unwrap();
        assert!(
            name.starts_with(PRE_DAEMON_SWAP_SNAPSHOT_PREFIX) && name.ends_with("-4242.json"),
            "snapshot name should carry the prefix and the writing pid, got {name}"
        );
        let reloaded: PreDaemonSwapRowOrderSnapshot =
            serde_json::from_str(&std::fs::read_to_string(last).expect("read snapshot"))
                .expect("snapshot round-trips");
        assert_eq!(reloaded.live_order, rows(&["a", "b"]));
        assert_eq!(
            reloaded.ledger.scope_rows(SHARED_ROW_ORDER_SCOPE),
            rows(&["a", "b"])
        );

        let remaining = std::fs::read_dir(dir.join(MANUAL_SNAPSHOT_DIR_NAME))
            .expect("snapshot dir")
            .flatten()
            .count();
        assert_eq!(
            remaining, MAX_PRE_DAEMON_SWAP_SNAPSHOTS,
            "the sweep must keep the directory at the cap"
        );
        assert!(last.exists(), "the newest snapshot must survive the sweep");
        let _ = std::fs::remove_dir_all(&dir);
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
