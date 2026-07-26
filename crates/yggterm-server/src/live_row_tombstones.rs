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
//!
//! # The file has N writers, so nothing here may write a private snapshot
//!
//! `removed-rows.json` lives in `~/.yggterm`, the home EVERY daemon on this
//! machine shares — and this host routinely runs three chained daemons. The
//! first cut of this module loaded the map once at construction and wrote the
//! whole map back on every change, which is last-writer-wins over a stale
//! snapshot: daemon A boots, daemon C records the user's close, then A saves
//! its boot-time copy and the close is GONE — including when A merely expires
//! one of its own entries. The successor then loads the truncated file and
//! re-adopts the row the user closed. That is the exact bug this module exists
//! to prevent, one layer down.
//!
//! So every mutation here is a read-modify-write of the SHARED file under an
//! exclusive `flock`, never a write of this process's copy
//! ([`LiveRowTombstones::mutate_shared`]), and the in-memory copy is a cache
//! that is re-read before it is trusted ([`LiveRowTombstones::refresh`]).
//! `record`/`clear`/`gc`/`save` remain as the in-memory primitives the RMW is
//! built from; a caller outside this module that reaches for them is writing a
//! private snapshot again, which `daemon.rs`'s structural lock refuses.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const TOMBSTONE_FILE_NAME: &str = "removed-rows.json";

/// How long we are willing to wait for another daemon's read-modify-write
/// before giving up on the lock. The critical section is a read plus an
/// atomic-rename write of a file capped at [`MAX_TOMBSTONES`] entries, so a
/// healthy holder is done in microseconds; this bound exists only so a wedged
/// process cannot stall the daemon, which calls in under its runtime lock.
const LOCK_WAIT_BUDGET_MS: u64 = 500;
const LOCK_RETRY_SLEEP_MS: u64 = 5;

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

/// The identities that ENTERED the live order since the last reconcile.
///
/// Clearing a tombstone must be keyed on a row *arriving*, not on a row being
/// present: a stale peer that has held the row since before the user closed it
/// reports it live at every single reconcile, so "present ⇒ clear" hands that
/// peer a veto-eraser and the close never sticks. A row that was absent last
/// time and is here now got here by a real user action, which is precisely the
/// signal that the veto has served its purpose.
///
/// A newtype rather than a `Vec<String>` parameter on purpose: the whole live
/// set and the newly-entered subset are both "a list of identities", and
/// passing the wrong one is silent. Here it does not compile.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnteredLiveRows(Vec<String>);

impl EnteredLiveRows {
    /// `live` minus `seen`, sorted — the daemon's live order is walked into a
    /// `HashSet`, whose iteration order is not ours to depend on.
    pub fn since(seen: &HashSet<String>, live: &HashSet<String>) -> Self {
        let mut entered: Vec<String> = live.difference(seen).cloned().collect();
        entered.sort();
        Self(entered)
    }

    pub fn identities(&self) -> &[String] {
        &self.0
    }
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
        let mut loaded = Self::read_shared(home_dir);
        loaded.gc(now);
        loaded
    }

    /// Re-read the shared file into this cache. The veto is only as good as its
    /// freshness: a close recorded by a peer AFTER we booted is invisible to a
    /// map loaded at construction, so the import admission passes would adopt
    /// back the very row the user just closed.
    pub fn refresh(&mut self, home_dir: &Path, now: u64) {
        *self = Self::load(home_dir, now);
    }

    /// Read the shared file. A file we cannot parse is NOT silently an empty
    /// deny-list: it is preserved beside the original and reported, because the
    /// only other symptom is closed rows quietly coming back.
    fn read_shared(home_dir: &Path) -> Self {
        let path = Self::tombstone_path(home_dir);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&raw) {
            Ok(loaded) => loaded,
            Err(error) => {
                let corrupt = path.with_extension("json.corrupt");
                let preserved = std::fs::write(&corrupt, raw.as_bytes()).is_ok();
                tracing::warn!(
                    %error,
                    path = %path.display(),
                    preserved,
                    "unreadable live-row tombstone file: every remembered close is being \
                     dropped, so rows the user closed can come back"
                );
                Self::default()
            }
        }
    }

    pub fn save(&self, home_dir: &Path) -> Result<()> {
        let path = Self::tombstone_path(home_dir);
        let raw = serde_json::to_string_pretty(self).context("serializing live-row tombstones")?;
        // Per-process tmp name: the publish is atomic only if no other daemon
        // is writing the same tmp file underneath us.
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, raw).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
    }

    /// Apply `mutate` to the SHARED on-disk set under an exclusive lock, publish
    /// the result, and adopt it as this process's cache. Returns true when the
    /// file changed.
    ///
    /// This is the only write path. `mutate`'s own return value is ignored on
    /// purpose — whether the set changed is decided by comparing before/after,
    /// so one owner answers it and a mutator that mis-reports cannot corrupt
    /// the file or skip a needed write.
    fn mutate_shared(
        &mut self,
        home_dir: &Path,
        mutate: impl FnOnce(&mut Self),
    ) -> Result<bool> {
        let _guard = TombstoneFileLock::acquire(home_dir);
        let mut shared = Self::read_shared(home_dir);
        let before = shared.entries.clone();
        mutate(&mut shared);
        let changed = shared.entries != before;
        if changed {
            shared.save(home_dir)?;
        }
        *self = shared;
        Ok(changed)
    }

    /// Remember that `identity` was closed, without losing any close a peer
    /// daemon recorded while we were holding a stale copy.
    pub fn record_close(&mut self, home_dir: &Path, identity: &str, now: u64) -> Result<bool> {
        self.mutate_shared(home_dir, |shared| {
            shared.record(identity, now);
        })
    }

    /// Expire old closes and forget the closes of rows that just re-entered the
    /// live order. Both halves apply to the SHARED set, so a row re-opened on a
    /// daemon that booted before the close still lifts its own veto — the map
    /// this daemon loaded is not the only place the close can live.
    pub fn reconcile(
        &mut self,
        home_dir: &Path,
        now: u64,
        entered: &EnteredLiveRows,
    ) -> Result<bool> {
        self.mutate_shared(home_dir, |shared| {
            shared.gc(now);
            for identity in entered.identities() {
                shared.clear(identity);
            }
        })
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

    /// The set is asked about ONE identity at a time in production (`blocks`);
    /// enumerating it is a test affordance, and leaving it un-gated would be a
    /// standing invitation to walk the deny-list instead of querying it.
    #[cfg(test)]
    pub fn identities(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

/// Exclusive advisory lock over the shared tombstone file, held for one
/// read-modify-write. `flock` is released by the kernel when the fd closes, so
/// a daemon that dies mid-write cannot wedge its peers.
struct TombstoneFileLock {
    #[cfg(unix)]
    file: std::fs::File,
}

impl TombstoneFileLock {
    /// Best-effort: on a busy lock we retry within [`LOCK_WAIT_BUDGET_MS`] and
    /// then proceed WITHOUT it. That is still a read-modify-write of the shared
    /// file — the worst case degrades from "no lost close" to "at most the one
    /// close racing inside a microsecond-wide window", never back to the
    /// whole-map clobber. Blocking the daemon indefinitely on a wedged peer
    /// would be the worse trade.
    fn acquire(home_dir: &Path) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let path = LiveRowTombstones::tombstone_path(home_dir).with_extension("json.lock");
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .ok()?;
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_millis(LOCK_WAIT_BUDGET_MS);
            loop {
                let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
                if rc == 0 {
                    return Some(Self { file });
                }
                let error = std::io::Error::last_os_error();
                let busy = matches!(
                    error.raw_os_error(),
                    Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
                );
                if !busy || std::time::Instant::now() >= deadline {
                    tracing::warn!(
                        %error,
                        path = %path.display(),
                        "proceeding without the live-row tombstone lock"
                    );
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(LOCK_RETRY_SLEEP_MS));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = home_dir;
            None
        }
    }
}

impl Drop for TombstoneFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            unsafe {
                let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
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
