//! Daemon-owned single-writer lock over a web profile's mutable storage
//! (agent control plane, slice 4.2).
//!
//! # Why
//!
//! A web profile is one directory of mutable state — cookie jar, SQLite
//! WAL/SHM, IndexedDB, service workers, caches, downloads. Opening two
//! `WebContext`s on the same profile corrupts it. Once more than one live web
//! client can exist (the user's GUI, a shadow view client, a farm page), some
//! authority has to say which one may write. The daemon is the only process
//! that sees every client, so the lock lives here.
//!
//! # Two leases at two layers — do not conflate (eng-review D5)
//!
//! | | owner | granularity | lifetime |
//! |---|---|---|---|
//! | **surface lease** (slice 2b) | GUI reconciler | one web *surface* | short TTL, keep-alive |
//! | **profile write-lock** (this) | daemon | one *profile* jar | holder's process lifetime |
//!
//! They never share state. **Lock ordering is profile-write-lock FIRST, then
//! the surface lease** — always that direction, so two clients converging on
//! the same profile can never deadlock holding half of each. A client that
//! fails to acquire the write-lock must not go on to take the surface lease.
//!
//! # Crash recovery
//!
//! The lock is held for the holder's *process lifetime*, and holders die
//! (crash, SIGKILL, host restart) without releasing. A dead holder's lock is
//! therefore reclaimable: every acquire re-checks the incumbent's liveness and
//! takes over the lock if that process is gone, so a crash can never wedge a
//! profile permanently. Liveness is injected (see [`ProcessLiveness`]) so the
//! recovery path is unit-testable without spawning processes.

use std::collections::BTreeMap;

use yggterm_core::web_profile::{normalize_web_profile, web_profile_is_ephemeral};

/// Refusal reason handed to a second writer. One source of truth so the wire
/// string, the daemon response, and the tests cannot drift.
pub const PROFILE_BUSY: &str = "profile_busy";

/// Who holds a profile write-lock. `client_id` is the slice-4.0 per-request
/// client identity; `pid` is what makes crash recovery possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileWriteLockHolder {
    pub client_id: String,
    pub pid: u32,
}

impl ProfileWriteLockHolder {
    pub fn new(client_id: impl Into<String>, pid: u32) -> Self {
        Self {
            client_id: client_id.into(),
            pid,
        }
    }

    /// Same live client? Both halves must match: a recycled pid with a
    /// different `client_id` is a DIFFERENT client, and a same-named client
    /// that restarted under a new pid is a different process. Requiring both
    /// keeps a restarted client from inheriting its own dead lock silently —
    /// it reacquires through the normal (journaled) reclaim path instead.
    fn is(&self, other: &ProfileWriteLockHolder) -> bool {
        self.client_id == other.client_id && self.pid == other.pid
    }
}

/// One granted lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileWriteLockEntry {
    pub profile: String,
    pub holder: ProfileWriteLockHolder,
    pub acquired_at_ms: u64,
}

/// Is this pid still running? Injected so crash recovery is testable.
pub trait ProcessLiveness {
    fn is_alive(&self, pid: u32) -> bool;
}

/// Production liveness: does `/proc/<pid>` exist. (Same probe the daemon's
/// other process checks use; a zombie still has its entry, and a zombie holder
/// has not yet been reaped, so treating it as alive is correct.)
pub struct SystemLiveness;

impl ProcessLiveness for SystemLiveness {
    #[cfg(target_os = "linux")]
    fn is_alive(&self, pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    #[cfg(not(target_os = "linux"))]
    fn is_alive(&self, _pid: u32) -> bool {
        // No cheap portable probe: fail SAFE by treating the holder as alive,
        // so the lock is never stolen from a living writer. A stale lock is
        // recoverable (explicit release); a double writer corrupts the jar.
        true
    }
}

/// Result of an acquire attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// The lock is now held by the requester.
    Granted,
    /// The requester already held it — acquire is idempotent, so a client that
    /// re-asks (retry, reconnect) is not punished with `profile_busy`.
    AlreadyHeld,
    /// The previous holder's process was gone; its lock was reclaimed and
    /// granted to the requester. Carried out so the daemon can journal it.
    ReclaimedFromDead { previous: ProfileWriteLockHolder },
    /// A different, live client holds it.
    Busy { held_by: ProfileWriteLockHolder },
    /// An ephemeral profile keeps no shared state on disk, so every surface
    /// gets its own in-memory context and no lock is required.
    Ephemeral,
}

impl AcquireOutcome {
    /// May the requester write the profile after this outcome?
    pub fn is_writable(&self) -> bool {
        match self {
            Self::Granted
            | Self::AlreadyHeld
            | Self::ReclaimedFromDead { .. }
            | Self::Ephemeral => true,
            Self::Busy { .. } => false,
        }
    }
}

/// Result of a release attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// Released; the profile is now free for the next client.
    Released,
    /// Nothing held this profile.
    NotHeld,
    /// Someone else holds it — a release never steals a live client's lock.
    NotHolder { held_by: ProfileWriteLockHolder },
    /// Ephemeral profiles are never locked.
    Ephemeral,
}

/// The daemon's profile write-lock table: at most ONE holder per profile.
#[derive(Debug, Default)]
pub struct ProfileWriteLockTable {
    entries: BTreeMap<String, ProfileWriteLockEntry>,
}

impl ProfileWriteLockTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the write-lock for `profile`.
    ///
    /// Idempotent for the current holder, reclaims a dead holder's lock (crash
    /// recovery), and otherwise refuses with [`AcquireOutcome::Busy`] — never
    /// granting two live writers.
    pub fn acquire(
        &mut self,
        profile: Option<&str>,
        holder: ProfileWriteLockHolder,
        now_ms: u64,
        liveness: &dyn ProcessLiveness,
    ) -> AcquireOutcome {
        let profile = normalize_web_profile(profile);
        if web_profile_is_ephemeral(&profile) {
            return AcquireOutcome::Ephemeral;
        }
        let mut reclaimed = None;
        if let Some(current) = self.entries.get(&profile) {
            if current.holder.is(&holder) {
                return AcquireOutcome::AlreadyHeld;
            }
            if liveness.is_alive(current.holder.pid) {
                return AcquireOutcome::Busy {
                    held_by: current.holder.clone(),
                };
            }
            // Holder is gone: its lock is recoverable, not permanent.
            reclaimed = Some(current.holder.clone());
        }
        self.entries.insert(
            profile.clone(),
            ProfileWriteLockEntry {
                profile,
                holder,
                acquired_at_ms: now_ms,
            },
        );
        match reclaimed {
            Some(previous) => AcquireOutcome::ReclaimedFromDead { previous },
            None => AcquireOutcome::Granted,
        }
    }

    /// Release `profile` if `holder` is the current holder.
    pub fn release(
        &mut self,
        profile: Option<&str>,
        holder: &ProfileWriteLockHolder,
    ) -> ReleaseOutcome {
        let profile = normalize_web_profile(profile);
        if web_profile_is_ephemeral(&profile) {
            return ReleaseOutcome::Ephemeral;
        }
        match self.entries.get(&profile) {
            None => ReleaseOutcome::NotHeld,
            Some(current) if current.holder.is(holder) => {
                self.entries.remove(&profile);
                ReleaseOutcome::Released
            }
            Some(current) => ReleaseOutcome::NotHolder {
                held_by: current.holder.clone(),
            },
        }
    }

    /// Current holder of `profile`, if any.
    pub fn holder(&self, profile: Option<&str>) -> Option<&ProfileWriteLockEntry> {
        self.entries.get(&normalize_web_profile(profile))
    }

    /// Drop every lock whose holder process is gone, returning what was
    /// reclaimed so the daemon can journal it. Acquire already self-heals per
    /// profile; this is the sweep for reporting and for freeing profiles nobody
    /// is currently asking for.
    pub fn reap_dead(&mut self, liveness: &dyn ProcessLiveness) -> Vec<ProfileWriteLockEntry> {
        let dead: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| !liveness.is_alive(entry.holder.pid))
            .map(|(profile, _)| profile.clone())
            .collect();
        dead.into_iter()
            .filter_map(|profile| self.entries.remove(&profile))
            .collect()
    }

    /// Every currently held lock, for `Status`/diagnostics.
    pub fn entries(&self) -> impl Iterator<Item = &ProfileWriteLockEntry> {
        self.entries.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Liveness stub: every pid is alive except those explicitly killed.
    struct FakeLiveness {
        dead: HashSet<u32>,
    }

    impl FakeLiveness {
        fn all_alive() -> Self {
            Self {
                dead: HashSet::new(),
            }
        }
        fn with_dead(pids: &[u32]) -> Self {
            Self {
                dead: pids.iter().copied().collect(),
            }
        }
    }

    impl ProcessLiveness for FakeLiveness {
        fn is_alive(&self, pid: u32) -> bool {
            !self.dead.contains(&pid)
        }
    }

    fn holder(id: &str, pid: u32) -> ProfileWriteLockHolder {
        ProfileWriteLockHolder::new(id, pid)
    }

    // Gate 12: acquire -> second writer profile_busy -> release -> re-acquire.
    #[test]
    fn second_writer_is_refused_until_the_first_releases() {
        let live = FakeLiveness::all_alive();
        let mut table = ProfileWriteLockTable::new();
        let first = holder("gui", 100);
        let second = holder("shadow", 200);

        assert_eq!(
            table.acquire(Some("work"), first.clone(), 1, &live),
            AcquireOutcome::Granted
        );
        // A second LIVE client is refused — never two writers on one jar.
        let busy = table.acquire(Some("work"), second.clone(), 2, &live);
        assert_eq!(
            busy,
            AcquireOutcome::Busy {
                held_by: first.clone()
            }
        );
        assert!(!busy.is_writable());
        // The refusal did not disturb the incumbent.
        assert_eq!(table.holder(Some("work")).unwrap().holder, first);

        assert_eq!(
            table.release(Some("work"), &first),
            ReleaseOutcome::Released
        );
        // Released -> the next client can take it.
        assert_eq!(
            table.acquire(Some("work"), second.clone(), 3, &live),
            AcquireOutcome::Granted
        );
        assert_eq!(table.holder(Some("work")).unwrap().holder, second);
    }

    #[test]
    fn acquire_is_idempotent_for_the_current_holder() {
        let live = FakeLiveness::all_alive();
        let mut table = ProfileWriteLockTable::new();
        let me = holder("gui", 100);
        assert_eq!(
            table.acquire(Some("work"), me.clone(), 1, &live),
            AcquireOutcome::Granted
        );
        // A retry/reconnect must not lock the client out of its own profile.
        let again = table.acquire(Some("work"), me.clone(), 2, &live);
        assert_eq!(again, AcquireOutcome::AlreadyHeld);
        assert!(again.is_writable());
        // Still exactly one entry, still the original grant time.
        assert_eq!(table.entries().count(), 1);
        assert_eq!(table.holder(Some("work")).unwrap().acquired_at_ms, 1);
    }

    // Gate 12, second half: crash recovery re-establishes a SINGLE writer.
    #[test]
    fn dead_holders_lock_is_reclaimed_not_wedged_forever() {
        let mut table = ProfileWriteLockTable::new();
        let crashed = holder("gui", 100);
        let next = holder("shadow", 200);
        assert_eq!(
            table.acquire(Some("work"), crashed.clone(), 1, &FakeLiveness::all_alive()),
            AcquireOutcome::Granted
        );
        // The holder dies without releasing.
        let after_crash = FakeLiveness::with_dead(&[100]);
        assert_eq!(
            table.acquire(Some("work"), next.clone(), 2, &after_crash),
            AcquireOutcome::ReclaimedFromDead { previous: crashed }
        );
        // Exactly ONE writer after recovery — the new one.
        assert_eq!(table.entries().count(), 1);
        assert_eq!(table.holder(Some("work")).unwrap().holder, next);
    }

    #[test]
    fn a_restarted_client_reusing_its_name_does_not_inherit_the_dead_lock() {
        let mut table = ProfileWriteLockTable::new();
        let old = holder("gui", 100);
        // Same client_id, NEW pid — a different process.
        let restarted = holder("gui", 101);
        table.acquire(Some("work"), old.clone(), 1, &FakeLiveness::all_alive());
        let outcome = table.acquire(Some("work"), restarted, 2, &FakeLiveness::with_dead(&[100]));
        // Goes through the journaled reclaim path, not a silent AlreadyHeld.
        assert_eq!(outcome, AcquireOutcome::ReclaimedFromDead { previous: old });
    }

    #[test]
    fn release_never_steals_a_live_clients_lock() {
        let live = FakeLiveness::all_alive();
        let mut table = ProfileWriteLockTable::new();
        let owner = holder("gui", 100);
        let other = holder("shadow", 200);
        table.acquire(Some("work"), owner.clone(), 1, &live);

        assert_eq!(
            table.release(Some("work"), &other),
            ReleaseOutcome::NotHolder {
                held_by: owner.clone()
            }
        );
        assert_eq!(table.holder(Some("work")).unwrap().holder, owner);
        assert_eq!(
            table.release(Some("other-profile"), &owner),
            ReleaseOutcome::NotHeld
        );
    }

    #[test]
    fn locks_are_per_profile_not_global() {
        let live = FakeLiveness::all_alive();
        let mut table = ProfileWriteLockTable::new();
        assert_eq!(
            table.acquire(Some("work"), holder("gui", 100), 1, &live),
            AcquireOutcome::Granted
        );
        // A different profile is a different jar — no contention.
        assert_eq!(
            table.acquire(Some("personal"), holder("shadow", 200), 2, &live),
            AcquireOutcome::Granted
        );
        assert_eq!(table.entries().count(), 2);
    }

    #[test]
    fn profile_names_normalize_so_one_jar_is_one_lock() {
        let live = FakeLiveness::all_alive();
        let mut table = ProfileWriteLockTable::new();
        let first = holder("gui", 100);
        table.acquire(Some("work"), first.clone(), 1, &live);
        // Same directory reached by a differently-spelled name must NOT yield a
        // second lock (the corruption hole the shared normalizer closes).
        assert_eq!(
            table.acquire(Some(" work "), holder("shadow", 200), 2, &live),
            AcquireOutcome::Busy { held_by: first }
        );
        // Unnamed/unsafe names collapse onto "default", also a single lock.
        assert_eq!(
            table.acquire(None, holder("a", 300), 3, &live),
            AcquireOutcome::Granted
        );
        assert_eq!(
            table.acquire(Some("../escape"), holder("b", 400), 4, &live),
            AcquireOutcome::Busy {
                held_by: holder("a", 300)
            }
        );
    }

    #[test]
    fn ephemeral_profile_needs_no_lock() {
        let live = FakeLiveness::all_alive();
        let mut table = ProfileWriteLockTable::new();
        // Every temp surface gets its own in-memory context: nothing to corrupt,
        // so concurrent "writers" are fine and nothing is ever recorded.
        assert_eq!(
            table.acquire(Some("temp"), holder("gui", 100), 1, &live),
            AcquireOutcome::Ephemeral
        );
        assert_eq!(
            table.acquire(Some("temp"), holder("shadow", 200), 2, &live),
            AcquireOutcome::Ephemeral
        );
        assert_eq!(table.entries().count(), 0);
        assert_eq!(
            table.release(Some("temp"), &holder("gui", 100)),
            ReleaseOutcome::Ephemeral
        );
    }

    #[test]
    fn reap_dead_frees_only_dead_holders_locks() {
        let mut table = ProfileWriteLockTable::new();
        let live = FakeLiveness::all_alive();
        table.acquire(Some("work"), holder("gui", 100), 1, &live);
        table.acquire(Some("personal"), holder("shadow", 200), 2, &live);

        let reaped = table.reap_dead(&FakeLiveness::with_dead(&[200]));
        assert_eq!(reaped.len(), 1);
        assert_eq!(reaped[0].profile, "personal");
        assert_eq!(reaped[0].holder, holder("shadow", 200));
        // The live holder keeps its lock.
        assert_eq!(table.entries().count(), 1);
        assert_eq!(
            table.holder(Some("work")).unwrap().holder,
            holder("gui", 100)
        );
    }
}
