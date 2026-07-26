//! Durable record of Live Sessions rows the USER closed.
//!
//! Removing a row is a local, immediate edit: [`crate::YggtermServer::remove_live_session`]
//! drops it from `sessions` and `live_session_order`, and the next persist
//! writes a state file without it. That is the whole of "closed" — there is no
//! record anywhere that the row was closed, only the absence of the row.
//!
//! Absence is not enough in a fleet of daemons. Every cross-daemon import path
//! asks one question of an incoming row — "do I already have it?" — and a row
//! the user just closed answers *no*. A peer daemon that never saw the close
//! still holds the row in memory and still advertises it, so the close is
//! undone by the next import. That is not hypothetical: a lingering 2.11.3
//! daemon re-added 9 dead rows on guihost on 2026-07-18 and jammed the Live pane,
//! which is why the takeover path grew its owns-runtime refusal. This module is
//! the missing half of that refusal — the memory that makes a close STICK even
//! when the peer legitimately owns the runtime it is offering back.
//!
//! Ownership: this file remembers closes and nothing else. It never mutates
//! `live_session_order`, never decides what is recoverable, and is consulted
//! only as a veto by the import admission predicate. Keys are
//! [`crate::normalized_live_row_identity`] folds, the same identity the import
//! walk de-dups on — a row must not slip past its own tombstone by coming back
//! under an equivalent runtime key.
//!
//! A deny-list that only grows is a worse bug than the one it fixes, so:
//! entries expire after [`TOMBSTONE_TTL_SECS`], the set is capped at
//! [`MAX_TOMBSTONES`] (oldest evicted first), and any row that legitimately
//! re-enters the live order clears its own tombstone through the
//! reconcile chokepoint in `daemon::persist`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const TOMBSTONE_FILE_NAME: &str = "removed-rows.json";

/// How long a close is remembered. Long enough to outlive the daemon chain that
/// caused the resurrection (peers linger for hours, not days), short enough
/// that a row the user closed last week is not un-addable today.
pub const TOMBSTONE_TTL_SECS: u64 = 3 * 24 * 60 * 60;

/// Hard cap on remembered closes. Oldest recorded entries are evicted first.
pub const MAX_TOMBSTONES: usize = 512;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LiveRowTombstones {
    /// normalized row identity -> unix seconds the close was recorded.
    /// `BTreeMap` so iteration and eviction are ordering-stable.
    entries: BTreeMap<String, u64>,
}

impl LiveRowTombstones {
    pub fn tombstone_path(home_dir: &Path) -> PathBuf {
        home_dir.join(TOMBSTONE_FILE_NAME)
    }

    /// Load and immediately expire. A daemon that has been down longer than the
    /// TTL must not come back holding stale vetoes.
    pub fn load(home_dir: &Path, now: u64) -> Self {
        let path = Self::tombstone_path(home_dir);
        let mut loaded: Self = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        loaded.gc(now);
        loaded
    }

    pub fn save(&self, home_dir: &Path) -> Result<()> {
        let path = Self::tombstone_path(home_dir);
        let raw = serde_json::to_string_pretty(self).context("serializing live-row tombstones")?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
    }

    /// Remember that `identity` was closed here. Returns true when the set
    /// changed (a re-close of an already-tombstoned row refreshes nothing —
    /// keeping the ORIGINAL timestamp is what makes the TTL a real expiry
    /// rather than a sliding window that never ends).
    pub fn record(&mut self, identity: &str, now: u64) -> bool {
        if self.entries.contains_key(identity) {
            return false;
        }
        self.entries.insert(identity.to_string(), now);
        self.evict_over_cap();
        true
    }

    /// Forget the close of `identity` — the row is legitimately back.
    pub fn clear(&mut self, identity: &str) -> bool {
        self.entries.remove(identity).is_some()
    }

    /// Should an incoming import of `identity` be refused?
    pub fn blocks(&self, identity: &str, now: u64) -> bool {
        self.entries
            .get(identity)
            .is_some_and(|recorded| !Self::expired(*recorded, now))
    }

    /// Drop expired entries. Returns true when the set changed.
    pub fn gc(&mut self, now: u64) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|_, recorded| !Self::expired(*recorded, now));
        let changed = self.entries.len() != before;
        changed | self.evict_over_cap()
    }

    pub fn identities(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn expired(recorded: u64, now: u64) -> bool {
        now.saturating_sub(recorded) >= TOMBSTONE_TTL_SECS
    }

    fn evict_over_cap(&mut self) -> bool {
        if self.entries.len() <= MAX_TOMBSTONES {
            return false;
        }
        // Oldest close first; identity break the tie so eviction is
        // deterministic for equal timestamps.
        let mut ordered: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(identity, recorded)| (*recorded, identity.clone()))
            .collect();
        ordered.sort();
        let excess = self.entries.len() - MAX_TOMBSTONES;
        for (_, identity) in ordered.into_iter().take(excess) {
            self.entries.remove(&identity);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_close_blocks_until_the_ttl_expires() {
        let mut tombstones = LiveRowTombstones::default();
        assert!(tombstones.record("id::dead-shell", 1_000));
        assert!(tombstones.blocks("id::dead-shell", 1_000));
        assert!(tombstones.blocks("id::dead-shell", 1_000 + TOMBSTONE_TTL_SECS - 1));
        assert!(!tombstones.blocks("id::dead-shell", 1_000 + TOMBSTONE_TTL_SECS));
        assert!(!tombstones.blocks("id::never-closed", 1_000));
    }

    #[test]
    fn re_closing_does_not_slide_the_expiry_window() {
        let mut tombstones = LiveRowTombstones::default();
        assert!(tombstones.record("id::row", 0));
        // A second close much later must not extend the original deadline.
        assert!(!tombstones.record("id::row", TOMBSTONE_TTL_SECS - 1));
        assert!(!tombstones.blocks("id::row", TOMBSTONE_TTL_SECS));
    }

    #[test]
    fn reopening_clears_the_tombstone() {
        let mut tombstones = LiveRowTombstones::default();
        tombstones.record("id::row", 10);
        assert!(tombstones.clear("id::row"));
        assert!(!tombstones.blocks("id::row", 10));
        assert!(!tombstones.clear("id::row"));
    }

    #[test]
    fn gc_expires_and_load_gcs() {
        let dir =
            std::env::temp_dir().join(format!("yggterm-tombstone-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let mut tombstones = LiveRowTombstones::default();
        tombstones.record("id::old", 0);
        tombstones.record("id::fresh", TOMBSTONE_TTL_SECS);
        tombstones.save(&dir).expect("save");

        let loaded = LiveRowTombstones::load(&dir, TOMBSTONE_TTL_SECS + 1);
        assert!(!loaded.blocks("id::old", TOMBSTONE_TTL_SECS + 1));
        assert!(loaded.blocks("id::fresh", TOMBSTONE_TTL_SECS + 1));
        assert_eq!(loaded.identities().collect::<Vec<_>>(), vec!["id::fresh"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_deny_list_is_capped_oldest_first() {
        let mut tombstones = LiveRowTombstones::default();
        for index in 0..(MAX_TOMBSTONES + 5) {
            tombstones.record(&format!("id::row-{index:04}"), index as u64);
        }
        assert_eq!(tombstones.len(), MAX_TOMBSTONES);
        assert!(!tombstones.blocks("id::row-0000", 0));
        assert!(!tombstones.blocks("id::row-0004", 0));
        assert!(tombstones.blocks("id::row-0005", 0));
    }
}
