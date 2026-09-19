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
//! Expiry is the end for a QUIET close — but not for one an offerer keeps
//! contesting. An eternal offerer (a daemonized opencode serve re-projecting
//! every session its store holds, every mirror tick) would turn the TTL into
//! a scheduled resurrection: measured 2026-09-19 19:34 IST on the muse lab
//! host, the 09-16 ghost despawns' tombstones expired at the same
//! wall-clock minute and six rows walked back in through the lawful
//! restore/import doors, rearranging the owner's Live pane. So a door that
//! just REFUSED a row on this plane re-arms the close at the half-life
//! ([`LiveRowTombstones::touch_close`], wired at every veto door through
//! [`crate::rearm_live_row_closes_among`]). A close nobody contests still
//! expires from its original timestamp; a user re-entry still clears it.
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

/// A vetoed automatic offer re-arms the close once it is this old (half the
/// TTL). Renewal exists because some offerers are ETERNAL: a daemonized
/// opencode `serve --service` re-projects every session its store holds on
/// every mirror tick, forever, so a bare 3-day tombstone against it is a
/// scheduled resurrection — jojo 2026-09-19 19:34 IST, the 09-16 ghost
/// despawns' tombstones expired at the same wall-clock minute and six rows
/// walked back in through the lawful restore/import doors. Renewing at
/// half-life keeps writes rare (one per half-age per refused row, not one per
/// mirror tick) while a continuously-offered row can never reach
/// [`TOMBSTONE_TTL_SECS`]. A quiet close still expires from the ORIGINAL
/// timestamp; a user re-entry still clears through the reconcile chokepoint.
pub(crate) const TOMBSTONE_RENEWAL_AGE_SECS: u64 = TOMBSTONE_TTL_SECS / 2;

/// Hard cap on remembered closes. Oldest recorded entries are evicted first.
pub const MAX_TOMBSTONES: usize = 512;

/// Hard cap on remembered departures, evicted oldest-first. Deliberately larger
/// than [`MAX_TOMBSTONES`]: a veto set is asked about one row at a time and is
/// emptied by re-entries, while this is a log somebody reads backwards, and the
/// GUI close of a busy machine can retire many rows in one act.
pub const MAX_DEPARTURES: usize = 2_048;

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

/// WHY a row left the live set.
///
/// `docs/spec-app-row-survival.md` §3: *a row leaving the live set for any
/// reason other than an explicit user close must leave a record saying which
/// reason.* The distinction that matters is exactly this two-way one — was this
/// row NAMED and closed, or did it go because of what it was?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RowDeparture {
    /// Somebody named this row and closed it: the user's own close, an agent's
    /// `session remove`, or the ephemeral reaper taking a row whose own creator
    /// declared it disposable. All three are deliberate acts on a named row, and
    /// the reaper additionally traces its own reap reason.
    ExplicitClose,
    /// The GUI window closed and this row was not keep-alive, so
    /// `PrepareClientClose` took it. **Nobody asked for THIS row to go** — it
    /// went because of a property it had. This is the evaporation the spec was
    /// written against, and telling it apart from the line above is the whole
    /// point of this ledger.
    GuiCloseDisposable,
    /// ⛔ **The row was never closed at all — it was left OUT of the state file.**
    /// It is still in the running daemon's live order; the SUCCESSOR daemon
    /// simply never learns it existed, so the row vanishes at the next restart
    /// with nothing having closed it.
    ///
    /// This is the one that actually took the owner's row group on 2026-08-13,
    /// and it is why the two stores disagreed: the old daemon still held the rows
    /// and still listed them, while the new one had never had them. Neither close
    /// path runs, so before this variant existed the departure left no record
    /// anywhere except a trace event in a file that rotates per GUI launch.
    PersistDropped,
}

impl RowDeparture {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExplicitClose => "explicit-close",
            Self::GuiCloseDisposable => "gui-close-disposable",
            Self::PersistDropped => "persist-dropped",
        }
    }
}

/// One row's departure, in the words a human asking *"where did my row go?"*
/// needs: which row, what it was called, why it went, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveRowDeparture {
    pub identity: String,
    pub path: String,
    pub title: String,
    pub reason: RowDeparture,
    pub at: u64,
    /// The finer-grained cause, when the reason has one. A persist drop has
    /// three of them and they mean different things — `not_recoverable`,
    /// `not_in_protected_runtime_keys`, `mapper_returned_none` — so a reader who
    /// only learns "it was dropped" still has to go to the trace to find out
    /// which gate took it. `Option` because the close paths have no sub-cause:
    /// an invented one would be worse than none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The row's kind and cwd AT the close, when the departing session was
    /// still resolvable. This is the identity half of a close: the stored-open
    /// of a closed local row re-derives the row from this record instead of
    /// guessing from the key's shape — the [11.156] measured defect (a shell
    /// row re-opened as `cd 'local:/' && codex resume <id>`). `Option` +
    /// serde default so records written before these fields read unchanged,
    /// and an older daemon writing the file only loses re-open fidelity,
    /// never a veto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LiveRowTombstones {
    /// normalized row identity -> unix seconds the close was recorded.
    /// `BTreeMap` so iteration and eviction are ordering-stable.
    entries: BTreeMap<String, u64>,
    /// Every departure, including the ones `entries` vetoes.
    ///
    /// ⚖ TWO QUESTIONS, ONE FILE, AND THEY ARE NOT THE SAME QUESTION.
    /// `entries` answers *"may this row be imported back?"* — a veto set that is
    /// cleared the moment the row legitimately returns, because a deny-list that
    /// only grows is worse than the bug it fixes. This answers *"what happened to
    /// the row that is not here?"*, which a re-entry must NOT erase: the record
    /// that a row evaporated an hour ago is still true after the user recreates
    /// it, and erasing it is how the loss became undiagnosable the first time.
    ///
    /// Same file because the shared read-modify-write discipline above is the
    /// hard part and duplicating it into a second file is how the whole-map
    /// clobber comes back. `#[serde(default)]` so a daemon of any vintage still
    /// reads the veto set; an older one drops this field when it writes, which
    /// costs history and never costs a veto.
    #[serde(default)]
    departures: Vec<LiveRowDeparture>,
}

/// The identity a deliberate close remembered about a row: enough to
/// re-derive what the user closed — its kind, its cwd, the title it carried
/// ([11.156]). `kind` travels as the string `SessionKind` serializes to and
/// is parsed at this edge, so a caller never sees a kind-shaped string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedRowIdentity {
    pub kind: Option<yggterm_core::SessionKind>,
    pub cwd: Option<String>,
    pub title: Option<String>,
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
        let departures_before = shared.departures.len();
        mutate(&mut shared);
        // ⛔ BOTH halves, or the ledger is write-only. This compared `entries`
        // alone, so a mutation that recorded ONLY a departure computed
        // `changed = false` and never reached `save` — the record would have
        // been dropped on the floor by the one function that exists to publish
        // it, which is the same shape of silence the ledger is here to end.
        let changed = shared.entries != before || shared.departures.len() != departures_before;
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

    /// Re-arm a close that just REFUSED an automatic offer — the write-side
    /// twin of the read door [`crate::live_row_closes_remembered_among`].
    /// [`Self::record_close`] deliberately never refreshes (the original
    /// timestamp is what makes the TTL a real expiry), and that stays right
    /// for the row itself; this moves the timestamp forward only once the
    /// entry is past [`TOMBSTONE_RENEWAL_AGE_SECS`], so an offerer that keeps
    /// pushing the row back cannot wait out the grave. An EXPIRED entry cannot
    /// be re-armed by a late offer — expiry is the designed end, and a load
    /// has already gc'd it, so this is a no-op there, never a resurrection of
    /// the record.
    pub fn touch_close(&mut self, home_dir: &Path, identity: &str, now: u64) -> Result<bool> {
        self.mutate_shared(home_dir, |shared| {
            if let Some(recorded) = shared.entries.get_mut(identity) {
                if Self::expired(*recorded, now) {
                    // An expired close cannot be re-armed by a late offer:
                    // expiry is the designed end. The shared file can still
                    // hold what a fresh load would have gc'd — drop it here,
                    // so the file converges to the gc'd view instead of the
                    // touch reviving a dead record.
                    shared.entries.remove(identity);
                } else if now.saturating_sub(*recorded) >= TOMBSTONE_RENEWAL_AGE_SECS {
                    *recorded = now;
                }
            }
        })
    }

    /// Write down that a row left, and why. Shared read-modify-write like every
    /// other write here, because this file has N daemons behind it.
    pub fn record_departure(
        &mut self,
        home_dir: &Path,
        departure: LiveRowDeparture,
    ) -> Result<bool> {
        self.mutate_shared(home_dir, |shared| {
            shared.push_departure(departure.clone());
        })
    }

    /// Newest first — the order the question is asked in.
    pub fn departures(&self) -> Vec<LiveRowDeparture> {
        let mut ordered = self.departures.clone();
        ordered.sort_by(|left, right| right.at.cmp(&left.at));
        ordered
    }

    /// The newest deliberate close (`RowDeparture::ExplicitClose`) recorded
    /// for `identity`, as identity fields. A stored-open of a closed local row
    /// consults this instead of synthesizing a row from the key alone;
    /// `None` means the close carried no usable identity — an older record, a
    /// departure that was not a user close — and the caller must refuse by
    /// name rather than guess ([11.156]).
    pub fn closed_row_identity(&self, identity: &str) -> Option<ClosedRowIdentity> {
        let departure = self.departures.iter().rev().find(|departure| {
            departure.identity == identity && departure.reason == RowDeparture::ExplicitClose
        })?;
        Some(ClosedRowIdentity {
            kind: departure
                .kind
                .as_deref()
                .and_then(|kind| serde_json::from_str(kind).ok()),
            cwd: departure.cwd.clone(),
            title: (!departure.title.trim().is_empty()).then(|| departure.title.clone()),
        })
    }

    fn push_departure(&mut self, departure: LiveRowDeparture) {
        self.departures.push(departure);
        self.gc_departures(0);
    }

    /// Expire and cap the ledger. `now = 0` only caps, which is what a fresh
    /// append wants: a record must not be evicted by the clock of the daemon
    /// that happens to be appending next to it.
    fn gc_departures(&mut self, now: u64) -> bool {
        let before = self.departures.len();
        if now != 0 {
            self.departures
                .retain(|departure| !Self::expired(departure.at, now));
        }
        if self.departures.len() > MAX_DEPARTURES {
            // Oldest first, and the tail is what we keep.
            self.departures.sort_by_key(|departure| departure.at);
            let excess = self.departures.len() - MAX_DEPARTURES;
            self.departures.drain(..excess);
        }
        self.departures.len() != before
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
        changed | self.evict_over_cap() | self.gc_departures(now)
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

    /// ⛔ A DEPARTURE-ONLY WRITE MUST REACH THE FILE. `mutate_shared` decided
    /// "did anything change?" by comparing `entries` alone, so recording a
    /// departure and nothing else computed `changed = false` and skipped `save`
    /// — the ledger would have been silently write-only, which is the exact
    /// failure it exists to end, reproduced one layer down.
    ///
    /// The round-trip through the shared file is the point; asserting on the
    /// in-memory copy would pass either way.
    #[test]
    fn a_departure_reaches_the_shared_file_even_when_no_tombstone_changed() {
        let dir = std::env::temp_dir().join(format!(
            "yggterm-departure-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let mut ledger = LiveRowTombstones::default();
        ledger
            .record_departure(
                &dir,
                LiveRowDeparture {
                    identity: "id::vanished".to_string(),
                    path: "local://vanished".to_string(),
                    title: "New Ychrome".to_string(),
                    reason: RowDeparture::GuiCloseDisposable,
                    at: 1_000,
                    detail: None,
                    kind: None,
                    cwd: None,
                },
            )
            .expect("record the departure");

        let reloaded = LiveRowTombstones::load(&dir, 1_000);
        let recorded = reloaded.departures();
        assert_eq!(
            recorded.len(),
            1,
            "the departure must be on disk — no tombstone changed, and that is \
             what made this write invisible"
        );
        assert_eq!(recorded[0].reason, RowDeparture::GuiCloseDisposable);
        assert_eq!(recorded[0].title, "New Ychrome");
        assert!(
            !reloaded.blocks("id::vanished", 1_000),
            "a departure is a RECORD, not a veto: the GUI close does not carry \
             the user's authority to forbid a peer offering the row back"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two reasons are the whole point, and a re-entry must not erase the
    /// history the way it erases the veto. A row that evaporated an hour ago
    /// still evaporated after the user recreates it — and "it is back now" is
    /// exactly the answer that made the first loss undiagnosable.
    #[test]
    fn a_reopened_row_lifts_its_veto_and_keeps_its_history() {
        let mut ledger = LiveRowTombstones::default();
        ledger.record("id::row", 10);
        ledger.push_departure(LiveRowDeparture {
            identity: "id::row".to_string(),
            path: "local://row".to_string(),
            title: "a row".to_string(),
            reason: RowDeparture::ExplicitClose,
            at: 10,
            detail: None,
            kind: None,
            cwd: None,
        });
        ledger.push_departure(LiveRowDeparture {
            identity: "id::row".to_string(),
            path: "local://row".to_string(),
            title: "a row".to_string(),
            reason: RowDeparture::GuiCloseDisposable,
            at: 20,
            detail: None,
            kind: None,
            cwd: None,
        });

        assert!(ledger.clear("id::row"));
        assert!(!ledger.blocks("id::row", 20));
        let history = ledger.departures();
        assert_eq!(history.len(), 2, "clearing a veto must not clear the record");
        assert_eq!(
            history[0].reason,
            RowDeparture::GuiCloseDisposable,
            "newest first — the order the question is asked in"
        );
        assert_eq!(history[1].reason, RowDeparture::ExplicitClose);
        assert_ne!(
            RowDeparture::ExplicitClose.label(),
            RowDeparture::GuiCloseDisposable.label(),
            "if the two reasons read alike, the ledger answers nothing"
        );
    }

    #[test]
    fn reopening_clears_the_tombstone() {
        let mut tombstones = LiveRowTombstones::default();
        tombstones.record("id::row", 10);
        assert!(tombstones.clear("id::row"));
        assert!(!tombstones.blocks("id::row", 10));
        assert!(!tombstones.clear("id::row"));
    }

    /// THE 2026-09-19 LAW: a close an offerer keeps contesting must not be
    /// wait-out-able. The original-timestamp rule in `record` makes the TTL a
    /// real expiry for a QUIET close; `touch_close` re-arms one that just
    /// refused an offer, at the half-life, so an eternal offerer (an opencode
    /// serve's store, re-projected every mirror tick) cannot hold a countdown
    /// against the user's close. Measured live: the 09-16 ghost despawns'
    /// tombstones expired at 19:34 and the six rows walked back in within
    /// seconds, offers still standing every ~26 s.
    #[test]
    fn a_contested_close_rearms_at_the_half_life_and_a_quiet_one_expires() {
        let dir =
            std::env::temp_dir().join(format!("yggterm-tombstone-rearm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let half = TOMBSTONE_TTL_SECS / 2;

        // The contested close: refused at the half-life, re-armed, and still
        // blocking well past where the ORIGINAL timestamp would have expired.
        let mut contested = LiveRowTombstones::default();
        contested.record("id::row", 0);
        contested.save(&dir).expect("save");
        let mut rearmed = LiveRowTombstones::load(&dir, half);
        assert!(rearmed.blocks("id::row", half), "the refusal fires first");
        assert!(
            rearmed.touch_close(&dir, "id::row", half).expect("rearm writes"),
            "a touch past the renewal age is a real change"
        );
        assert!(
            rearmed.blocks("id::row", TOMBSTONE_TTL_SECS + half - 1),
            "the renewal restarted the countdown: alive past the original horizon"
        );
        // The quiet control: same original close, no offers, long dead.
        let mut quiet = LiveRowTombstones::default();
        quiet.record("id::row", 0);
        assert!(
            !quiet.blocks("id::row", TOMBSTONE_TTL_SECS + half - 1),
            "without renewal the close expires from its original timestamp"
        );

        // Below the renewal age a touch changes nothing and writes nothing:
        // mirror ticks must not churn the shared file.
        let mut young = LiveRowTombstones::default();
        young.record("id::row", 10);
        young.save(&dir).expect("save");
        let mut fresh = LiveRowTombstones::load(&dir, 10);
        assert!(
            !fresh
                .touch_close(&dir, "id::row", 10 + 60)
                .expect("young touch is a no-op"),
            "below the renewal age there is nothing to re-arm"
        );
        assert_eq!(
            fresh.entries.get("id::row"),
            Some(&10),
            "the original close timestamp is untouched below the renewal age"
        );

        // An EXPIRED close cannot be re-armed by a late offer — expiry is the
        // designed end, and a fresh load has already gc'd the entry.
        let mut expired = LiveRowTombstones::default();
        expired.record("id::row", 0);
        expired.save(&dir).expect("save");
        let mut gone = LiveRowTombstones::load(&dir, TOMBSTONE_TTL_SECS + 1);
        assert!(!gone.blocks("id::row", TOMBSTONE_TTL_SECS + 1));
        assert!(
            gone.touch_close(&dir, "id::row", TOMBSTONE_TTL_SECS + 1)
                .expect("expired touch converges the file"),
            "dropping the dead record IS the change — not a renewal"
        );
        assert!(
            gone.entries.get("id::row").is_none(),
            "a late offer must not resurrect the record"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every door that refuses a birth on this plane must re-arm the close it
    /// just refused — the re-arm is what makes the TTL compatible with eternal
    /// offerers. Source-scan lock: the wiring is one call at each veto site,
    /// and a site that loses it silently reintroduces the 2026-09-19
    /// scheduled resurrection.
    #[test]
    fn every_tombstone_veto_door_rearms_the_close_it_refused() {
        let mirror = include_str!("opencode_mirror.rs");
        let lib = include_str!("lib.rs");
        let daemon = include_str!("daemon.rs");
        let needle = "rearm_live_row_closes_among(";
        assert!(
            mirror.matches(needle).count() >= 1,
            "the opencode mirror veto must re-arm the graves it refuses — it is              the eternal offerer behind the 2026-09-19 resurrection"
        );
        assert!(
            lib.contains("fn rearm_live_row_closes_among"),
            "the shared re-arm door must exist in lib.rs beside the read door"
        );
        assert!(
            lib.matches(needle).count() >= 2,
            "lib.rs carries the batch-restore and the recovery-sweep veto call sites"
        );
        assert!(
            daemon.matches(needle).count() >= 2,
            "both cross-daemon import callers (B4 adoption + superseded              takeover) must re-arm what they refuse"
        );
        assert!(
            lib.contains("touch_close("),
            "the re-arm must go through the shared RMW door, never a private              snapshot write"
        );
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

    /// The read-only door the GUI asks "did the user close this row?" through,
    /// so `web ensure` cannot revive a surface under a closed session.
    ///
    /// Two things it must get right, both of which fail silently:
    ///   - it FOLDS the key. A local runtime row is addressed by several
    ///     equivalent spellings over its life, and asking about the raw key
    ///     walks straight past the row's own tombstone. Drop
    ///     `normalized_live_row_identity` from
    ///     `live_row_close_is_remembered` and the `codex://` read goes false.
    ///   - it WRITES NOTHING. `removed-rows.json` is shared by every daemon on
    ///     the machine, so a reader that published its own copy would erase
    ///     peers' closes — the exact bug one layer down.
    #[test]
    fn the_read_only_close_query_folds_the_key_and_leaves_the_file_alone() {
        let dir = std::env::temp_dir().join(format!(
            "yggterm-tombstone-read-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // The close, written the way the daemon writes it.
        let mut tombstones = LiveRowTombstones::default();
        tombstones
            .record_close(
                &dir,
                &crate::normalized_live_row_identity("local://5f2a"),
                now_secs(),
            )
            .expect("record the close");
        let path = LiveRowTombstones::tombstone_path(&dir);
        let before = std::fs::read(&path).expect("the close was published");

        // Every spelling of that one row answers the same way.
        for spelling in [
            "local://5f2a",
            "codex://5f2a",
            "codex-runtime://5f2a",
            "codex-litellm://5f2a",
        ] {
            assert!(
                crate::live_row_close_is_remembered(&dir, spelling),
                "{spelling} slipped past its own tombstone"
            );
        }
        assert!(!crate::live_row_close_is_remembered(
            &dir,
            "local://never-closed"
        ));

        // …and asking changed nothing on disk.
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            before,
            "the read-only query rewrote the shared deny-list"
        );
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

    /// [11.156]: the identity half of a close. A deliberate close remembers
    /// the row's kind/cwd/title so the stored-open re-open door can re-derive
    /// the row instead of guessing from the key's shape; the newest
    /// ExplicitClose wins; a departure that is not a user close never answers.
    #[test]
    fn a_deliberate_close_remembers_the_rows_identity_for_the_reopen_door() {
        let mut ledger = LiveRowTombstones::default();
        ledger.push_departure(LiveRowDeparture {
            identity: "id::shell".to_string(),
            path: "local://shell".to_string(),
            title: String::new(),
            reason: RowDeparture::GuiCloseDisposable,
            at: 10,
            detail: None,
            kind: None,
            cwd: None,
        });
        ledger.push_departure(LiveRowDeparture {
            identity: "id::shell".to_string(),
            path: "local://shell".to_string(),
            title: "proof shell".to_string(),
            reason: RowDeparture::ExplicitClose,
            at: 20,
            detail: None,
            kind: Some(serde_json::to_string(&yggterm_core::SessionKind::Shell).unwrap()),
            cwd: Some("/tmp/ygg-1156-proof".to_string()),
        });

        let remembered = ledger
            .closed_row_identity("id::shell")
            .expect("the deliberate close remembers the row's identity");
        assert_eq!(remembered.kind, Some(yggterm_core::SessionKind::Shell));
        assert_eq!(remembered.cwd.as_deref(), Some("/tmp/ygg-1156-proof"));
        assert_eq!(remembered.title.as_deref(), Some("proof shell"));

        // A row that was never deliberately closed answers None — the re-open
        // must refuse by name, not borrow a GuiCloseDisposable record.
        assert!(ledger.closed_row_identity("id::absent").is_none());

        // An older-format record (no identity fields on disk) still loads and
        // answers with what it has; the caller's guard treats that as
        // underdetermined rather than guessing.
        let legacy = serde_json::from_str::<LiveRowTombstones>(
            "{\"entries\":{},\"departures\":[{\"identity\":\"id::old\",\
             \"path\":\"local://old\",\"title\":\"old row\",\"reason\":\
             \"explicit-close\",\"at\":5}]}",
        )
        .expect("an older record without the identity fields still loads");
        let old = legacy
            .closed_row_identity("id::old")
            .expect("the close is still a deliberate close");
        assert_eq!(old.kind, None);
        assert_eq!(old.cwd, None);
        assert_eq!(old.title.as_deref(), Some("old row"));
    }
}
