//! Autoclean for the versioned daemon sockets in `$YGGTERM_HOME`
//! (`server-<major>-<minor>-<patch>.sock`).
//!
//! Counted on the GUI host 2026-08-05: ~700 of them, back to `server-2-1-2`,
//! with seven daemons alive. Nothing had ever swept them and every deploy adds
//! one more — the same shape as the clipboard staging dir before
//! `clipboard_sweep.rs`, so this module follows that module's shape: per-host,
//! this daemon's OWN `$YGGTERM_HOME` only, fail-safe on any read it cannot
//! complete.
//!
//! ## The predicate, and why it is not "connect failed"
//!
//! docs/pending-bugs.md states the constraint that shapes everything here:
//!
//! > Do NOT unlink a socket merely because `connect` fails. A daemon
//! > mid-restart has a moment with no listener, and deleting its address there
//! > turns a hiccup into a lost daemon.
//!
//! So liveness is proved POSITIVELY and cheaply, from the kernel's own table
//! (`/proc/net/unix`), not from a client-side connect that can fail for a dozen
//! reasons that have nothing to do with the file being garbage. Three
//! consequences:
//!
//! 1. **One read answers for all ~700 paths.** The predecessor sweep
//!    (`cleanup_dead_versioned_server_sockets`) issued one `status()` request
//!    per socket on every daemon start; this issues none.
//! 2. **A symlink alias whose target is a live daemon is KEPT.**
//!    `refresh_legacy_server_socket_aliases` back-aliases every legacy version
//!    onto the running socket, so most entries in a long-lived
//!    `$YGGTERM_HOME` are symlinks that resolve to a listening path. They are
//!    serving, not litter, and this module leaves them alone. **See the
//!    census note at the bottom of this comment.**
//! 3. **The mid-restart window cannot be hit**, because a socket is unlinked
//!    only after it has been observed dead in an EARLIER sweep round at least
//!    [`SOCKET_DEAD_CONFIRM_MS`] ago. The ledger is rewritten from scratch each
//!    round, so a version that comes back to life loses its death mark and
//!    starts over. A restart window is seconds; confirmation is a day.
//!
//! ## Fail-safe bias
//!
//! Every unknown keeps the file: a name this module cannot parse as a versioned
//! server socket is never touched (the name format has exactly one owner,
//! [`crate::daemon::parse_versioned_server_socket_name`], and this module reuses
//! it); a kernel table that will not read makes the whole census incomplete and
//! the round removes nothing; an entry whose metadata will not stat is kept; a
//! ledger that will not read is treated as empty, which means "first sighting"
//! for everything and therefore no deletions this round.
//!
//! ## Bequest litter (2026-09-04)
//!
//! A same-version handoff renames its socket and lock to `*.retired-<pid>`
//! (`daemon::bequeath_version_socket_to_same_version_successor`). Those names
//! parse as neither canonical sockets nor aliases, so the sweep skipped them
//! as NotOurs — measured on the GUI host: 76 such files within two days of
//! the bequest landing, while its dead-sighting ledger stayed empty. They are
//! now classified by the pid their name carries: alive ⇒ load-bearing (the
//! preserved-owner registry hands adopters this exact path), gone ⇒ the usual
//! re-proved-sighting removal. The pid witness needs no census, so this one
//! branch stays decidable even when the kernel socket table cannot be read.
//!
//! ## The pty-handoff graveyard (2026-09-06)
//!
//! Every daemon spawn binds a `pty-handoff-<version>.sock` listener (the
//! same-version successor rebinds the same name), and until now nothing ever
//! swept the name shape — it parsed as NotOurs. Counted on the GUI host
//! 2026-09-06: **218 of them back to August**, beside the retired-socket and
//! lock litter the branches above already reap. They classify by the same
//! census the server sockets use: bound here, or a live daemon of that
//! version anywhere on the host, ⇒ load-bearing; otherwise the ordinary
//! re-proved-sighting rule. The parse lives beside the name's builder
//! (`pty_handoff::parse_handoff_socket_name`) so the shape has one owner.
//!
//! ## Platform
//!
//! Unix only (the socket names are), and the liveness proof is Linux's
//! `/proc/net/unix`. On a non-Linux unix the census reports incomplete and the
//! sweep is a no-op — it never guesses. Windows does not compile this module at
//! all (`#[cfg(unix)]` at the `mod` site), exactly like the socket-name owner it
//! borrows from.
//!
//! ## Census note (measured, not assumed) — 2026-08-06
//!
//! On the GUI host: 676 `server-*.sock`, of which **670 are symlink aliases
//! resolving to a live daemon** and 6 are real sockets, **all 6 listening**.
//! Under the rule above — the rule the bug entry asks for — the correct verdict
//! for all 676 is KEEP, and this sweep collects nothing there today. That is not
//! the sweep failing; it is the entry's premise ("a file no process will ever
//! bind again") not matching the population. The aliases are regenerated on
//! every daemon bind from the 675 empty scope directories under
//! `$YGGTERM_HOME/client-instances/`, which `refresh_legacy_server_socket_aliases`
//! reads as alias candidates. Retiring a LIVE alias is a behaviour change — an
//! old client that loses its alias falls back to spawning its own daemon — so it
//! is a policy call for the user, not something this sweep may infer. What this
//! module guarantees is the part that is unambiguous: a socket nothing is
//! listening on, whose version is no live daemon's, stops accumulating.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::daemon::{parse_retired_server_socket_artifact, parse_versioned_server_socket_name};
use crate::pty_handoff::parse_handoff_socket_name;
use yggterm_core::append_trace_event;

/// How long a socket must have been continuously observed dead before it is
/// unlinked. A daemon mid-restart is dark for milliseconds; this is a day, and
/// it must be re-proved on a later round (see [`run_socket_sweep`]).
pub(crate) const SOCKET_DEAD_CONFIRM_MS: u64 = 24 * 60 * 60 * 1000;

/// Interval gate for the chore tick, matching `clipboard_sweep`.
pub(crate) const SOCKET_SWEEP_INTERVAL_MS: u64 = 6 * 60 * 60 * 1000;

/// Dead-sighting ledger: `<socket file name>\t<first-seen-dead millis>`.
/// Leading dot so it can never itself parse as a versioned socket name.
const DEAD_LEDGER_NAME: &str = ".socket-sweep-dead";

/// Interval marker, explicit rather than an mtime (a restored/copied
/// `$YGGTERM_HOME` must not fake recency) — same reasoning as
/// `clipboard_sweep`'s marker.
const SWEEP_MARKER_NAME: &str = ".socket-sweep-last-ms";

/// Why an entry survived a round. Carried so the trace event can say which rule
/// spared a file, instead of only counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeepReason {
    /// The census could not be completed — nothing is deletable this round.
    CensusIncomplete,
    /// This daemon's own socket.
    OwnSocket,
    /// The path, or what it resolves to, is bound by a listening process.
    Listening,
    /// A daemon of exactly this version is alive (bound elsewhere).
    LiveDaemonVersion,
    /// The entry could not be stat'd; absence of proof keeps the file.
    Unreadable,
    /// A bequest's retired artifact whose retiree pid is still alive — the
    /// preserved-owner registry hands adopters this exact path while the
    /// lingerer drains, so the file is load-bearing however dead its name
    /// looks to the census.
    RetiredOwnerAlive,
    /// The alias/lock resolves to something live, but the install history
    /// could not be read, so whether the version is still rollback-able is
    /// unknown — kept, because absence of proof is never a deletion.
    RollbackSetUnknown,
}

/// One entry's verdict. `Remove` is the only branch that unlinks anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocketVerdict {
    /// The name is not a versioned server socket — this module does not manage
    /// it and must never touch it.
    NotOurs,
    Keep(KeepReason),
    /// Dead, but not yet dead in an earlier round: record the sighting and look
    /// again next time.
    ConfirmLater,
    /// Dead now and dead at least `SOCKET_DEAD_CONFIRM_MS` ago.
    Remove,
}

/// What this host's daemons are, proved from the kernel rather than from a
/// client-side connect. Construct with [`LiveDaemonCensus::gather`]; the
/// `from_parts` constructor exists so the predicate can be tested without a
/// daemon, and so a caller that already holds a probe result can contribute it.
#[derive(Debug, Clone, Default)]
pub(crate) struct LiveDaemonCensus {
    /// Filesystem paths a process is currently LISTENING on.
    listening: HashSet<PathBuf>,
    /// Versions of daemons proved live, from the names of the listening paths.
    live_versions: HashSet<(u64, u64, u64)>,
    /// False ⇒ some input could not be gathered ⇒ the sweep removes nothing.
    complete: bool,
    /// Versions still installed under `versions/` — the only versions a
    /// rollback can return to, so the only versions whose socket-name ALIAS
    /// is load-bearing. Empty-but-known means "nothing can roll back".
    rollback_versions: HashSet<(u64, u64, u64)>,
    /// False ⇒ the versions directory could not be read ⇒ alias retention is
    /// unrestricted (fail-safe: an unreadable install history keeps aliases).
    rollback_known: bool,
}

impl LiveDaemonCensus {
    /// Build a census from `listening` paths that are already proved live.
    /// `complete=false` disables every deletion, which is what any failed read
    /// must produce.
    pub(crate) fn from_parts(listening: HashSet<PathBuf>, complete: bool) -> Self {
        let live_versions = listening
            .iter()
            .filter_map(|path| parse_versioned_server_socket_name(path))
            .collect();
        Self {
            listening,
            live_versions,
            complete,
            rollback_versions: HashSet::new(),
            rollback_known: false,
        }
    }

    /// State the installed-versions set explicitly (builder; tests and
    /// `gather` are the callers).
    pub(crate) fn with_rollback_versions(
        mut self,
        versions: HashSet<(u64, u64, u64)>,
        known: bool,
    ) -> Self {
        self.rollback_versions = versions;
        self.rollback_known = known;
        self
    }

    /// Read the kernel's unix-socket table and keep the listening paths that
    /// live in `home_dir`. Any failure returns an incomplete census rather than
    /// an empty one — "I could not look" must never read as "nothing is alive".
    pub(crate) fn gather(home_dir: &Path) -> Self {
        let Some(all) = listening_unix_socket_paths() else {
            return Self::from_parts(HashSet::new(), false);
        };
        let listening = all
            .into_iter()
            .filter(|path| path.parent() == Some(home_dir))
            .collect();
        Self::from_parts(listening, true).with_rollback_versions(
            Self::installed_rollback_versions(home_dir),
            home_dir.join("versions").is_dir(),
        )
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }

    /// Versions a rollback can still return to: the names under `versions/`.
    /// Directory names are dot-separated (`3.2.61`); socket names are
    /// dash-separated. A failed read yields an empty set AND `false` — the
    /// caller distinguishes "nothing installed" from "could not look".
    fn installed_rollback_versions(home_dir: &Path) -> HashSet<(u64, u64, u64)> {
        let mut versions = HashSet::new();
        let Ok(entries) = fs::read_dir(home_dir.join("versions")) else {
            return versions;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let mut parts = name.split('.');
            let (Some(major), Some(minor), Some(patch)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if parts.next().is_some() {
                continue;
            }
            if let (Ok(major), Ok(minor), Ok(patch)) = (
                major.parse::<u64>(),
                minor.parse::<u64>(),
                patch.parse::<u64>(),
            ) {
                versions.insert((major, minor, patch));
            }
        }
        versions
    }

    pub(crate) fn live_socket_count(&self) -> usize {
        self.listening.len()
    }

    /// The listening paths themselves, not just the tally.
    ///
    /// The sweep only ever needed to know whether a name is alive.
    /// `working_flag_owner_discovery` needs to ADDRESS the live daemons, and
    /// this census is the one place on the host that knows which they are
    /// without issuing a single request — which is the whole reason discovery
    /// reads it rather than probing `status` per socket.
    pub(crate) fn listening_paths(&self) -> &HashSet<PathBuf> {
        &self.listening
    }
}

/// Does a LIVE daemon still answer to this NAME?
///
/// ⛔⛔ **THIS IS THE ONLY INSTRUMENT THAT SURVIVES THE NAME BEING DIVERTED,
/// AND THAT IS THE ENTIRE POINT.** A `connect` through the path answers about
/// whatever the path resolves to *now*; once an alias has been re-pointed, that
/// is a different daemon, and the probe cheerfully reports "somebody is there"
/// about the very process that took the name. The kernel, by contrast, records
/// the path a socket was BOUND to and keeps reporting it after that path has
/// been unlinked or replaced — so this answers "whose address is this name",
/// which is the question a re-point actually needs to ask.
///
/// ⇒ Measured on the build host 2026-08-22: a daemon alive 28 h, holding 83
/// pty masters, still listed by `/proc/net/unix` as bound to its own versioned
/// name — while that name on disk was a symlink to a newer daemon's socket.
/// Nothing could dial it, and every session it owned was undrivable.
///
/// ⚖ **An incomplete census answers YES.** "I could not look" must never
/// license taking an address away; the fail-safe direction here is to leave the
/// name alone, exactly as the sweep's fail-safe direction is to leave the file
/// alone.
pub(crate) fn socket_name_is_a_live_daemons_address(
    census: &LiveDaemonCensus,
    candidate: &Path,
) -> bool {
    if !census.is_complete() {
        return true;
    }
    census.listening.contains(candidate)
}

/// Linux's `/proc/net/unix`, or `None` on any platform/read where it cannot be
/// consulted (⇒ incomplete census ⇒ no deletions).
#[cfg(target_os = "linux")]
fn listening_unix_socket_paths() -> Option<HashSet<PathBuf>> {
    let text = fs::read_to_string("/proc/net/unix").ok()?;
    Some(parse_listening_unix_socket_paths(&text))
}

#[cfg(not(target_os = "linux"))]
fn listening_unix_socket_paths() -> Option<HashSet<PathBuf>> {
    None
}

/// Parse `/proc/net/unix` into the set of filesystem paths that are BOUND AND
/// LISTENING.
///
/// Columns: `Num RefCount Protocol Flags Type St Inode Path`. `St` is the
/// socket state and `01` is `SS_UNCONNECTED` on a listener; the authoritative
/// marker is the `SO_ACCEPTCON` bit (`0x10000`) in `Flags`, so both are
/// required — a connected peer of a live daemon shares the path string but has
/// neither. Abstract sockets (`@name`) and pathless rows are skipped: they can
/// never be a file we would unlink.
fn parse_listening_unix_socket_paths(text: &str) -> HashSet<PathBuf> {
    const SO_ACCEPTCON: u64 = 0x10000;
    let mut listening = HashSet::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            continue;
        }
        let Ok(flags) = u64::from_str_radix(fields[3], 16) else {
            continue; // the header row lands here
        };
        if flags & SO_ACCEPTCON == 0 {
            continue;
        }
        if fields[5] != "01" {
            continue;
        }
        let path = fields[7];
        if !path.starts_with('/') {
            continue; // abstract namespace
        }
        listening.insert(PathBuf::from(path));
    }
    listening
}

/// The whole deletion decision for ONE directory entry, pure and total.
///
/// `canonical` is `fs::canonicalize(path)` (a symlink alias resolves to the
/// socket it points at; a dangling alias yields `None`), and `readable` is
/// whether `fs::symlink_metadata` succeeded — the two are passed in rather than
/// read here so the rule can be tested without a filesystem. `own_identity`
/// is the sweeping daemon's own socket path, already canonicalized once by the
/// caller (a syscall per entry across ~700 entries is not free).
/// Positively prove a pid alive from procfs, the same direction the census
/// reads liveness from. `None` ⇒ this kernel gives no answer (no procfs, e.g.
/// a non-Linux unix) ⇒ the caller must keep the file: absence of proof is
/// never a deletion.
fn proc_pid_alive(pid: u32) -> Option<bool> {
    let proc_fs = Path::new("/proc");
    if !proc_fs.is_dir() {
        return None;
    }
    Some(proc_fs.join(pid.to_string()).exists())
}

pub(crate) fn classify_socket_entry(
    path: &Path,
    canonical: Option<&Path>,
    readable: bool,
    census: &LiveDaemonCensus,
    own_identity: Option<&Path>,
    first_seen_dead_ms: Option<u64>,
    now_ms: u64,
) -> SocketVerdict {
    // 1. Only files this module can prove the socket layer named.
    let Some(version) = parse_versioned_server_socket_name(path) else {
        // 1b. The other shape this layer mints: a same-version bequest's
        //     retired socket/lock. Its name carries the retiree's pid, which
        //     is a DIRECT liveness witness — stronger than the census, which
        //     only knows who is listening right now. While the pid lives, the
        //     file is load-bearing (the preserved-owner registry points
        //     adopters at this exact path), so it is kept whatever the census
        //     says; once the pid is gone it is garbage and dies by the same
        //     re-proved-sighting rule as every other removal. This branch
        //     needs no census: /proc present ⇒ the pid check is complete
        //     proof; /proc absent ⇒ Unreadable keeps the file. Version is
        //     deliberately ignored here — a live daemon of the same version
        //     elsewhere says nothing about THIS retired path's owner.
        if let Some((_version, pid)) = parse_retired_server_socket_artifact(path) {
            return match proc_pid_alive(pid) {
                Some(true) => SocketVerdict::Keep(KeepReason::RetiredOwnerAlive),
                Some(false) => match first_seen_dead_ms {
                    Some(first) if now_ms.saturating_sub(first) >= SOCKET_DEAD_CONFIRM_MS => {
                        SocketVerdict::Remove
                    }
                    _ => SocketVerdict::ConfirmLater,
                },
                None => SocketVerdict::Keep(KeepReason::Unreadable),
            };
        }
        // 1c. A versioned LOCK file (`server-X-Y-Z.sock.lock`, no retiree
        //     pid) was invisible to this sweep as NotOurs — the 2026-09-05
        //     audit counted 785 of them beside the aliases. Its fate follows
        //     its socket: alive while that version is listening, live
        //     somewhere, or still installed; litter on the usual terms
        //     otherwise; kept while the install history is unreadable.
        if !census.complete {
            return SocketVerdict::Keep(KeepReason::CensusIncomplete);
        }
        if let Some(stem) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".lock"))
            .and_then(|stem| parse_versioned_server_socket_name(Path::new(stem)).map(|_| stem))
        {
            let version = parse_versioned_server_socket_name(Path::new(stem))
                .expect("stem re-parsed after lock strip");
            let sibling = path.with_file_name(format!(
                "server-{}-{}-{}.sock",
                version.0, version.1, version.2
            ));
            if census.listening.contains(&sibling)
                || census.live_versions.contains(&version)
                || (census.rollback_known && census.rollback_versions.contains(&version))
            {
                return SocketVerdict::Keep(KeepReason::LiveDaemonVersion);
            }
            if !census.rollback_known {
                return SocketVerdict::Keep(KeepReason::RollbackSetUnknown);
            }
            return match first_seen_dead_ms {
                Some(first) if now_ms.saturating_sub(first) >= SOCKET_DEAD_CONFIRM_MS => {
                    SocketVerdict::Remove
                }
                _ => SocketVerdict::ConfirmLater,
            };
        }
        // 1d. A pty-handoff listener's socket (`pty-handoff-X-Y-Z.sock`, the
        //     name shape `pty_handoff::handoff_socket_path` mints). The
        //     2026-09-06 GUI-host census counted 218 of them back to August —
        //     every daemon spawn mints one and nothing ever swept it. Fate:
        //     alive while it is itself bound (a live daemon's handoff
        //     listener) or while a daemon of that version lives anywhere on
        //     the host (a same-version successor rebinds the same name);
        //     litter on the ordinary re-proved-sighting terms otherwise. No
        //     rollback arm, unlike the lock files above: a rolled-back daemon
        //     binds this path FRESH at spawn (the bind replaces the file), so
        //     a dead socket of an installed version is never load-bearing.
        if let Some(version) = parse_handoff_socket_name(path) {
            if !census.complete {
                return SocketVerdict::Keep(KeepReason::CensusIncomplete);
            }
            if census.listening.contains(path) {
                return SocketVerdict::Keep(KeepReason::Listening);
            }
            if census.live_versions.contains(&version) {
                return SocketVerdict::Keep(KeepReason::LiveDaemonVersion);
            }
            return match first_seen_dead_ms {
                Some(first) if now_ms.saturating_sub(first) >= SOCKET_DEAD_CONFIRM_MS => {
                    SocketVerdict::Remove
                }
                _ => SocketVerdict::ConfirmLater,
            };
        }
        return SocketVerdict::NotOurs;
    };
    // 2. No proof of what is alive ⇒ no deletions at all.
    if !census.complete {
        return SocketVerdict::Keep(KeepReason::CensusIncomplete);
    }
    // 3. Never the sweeping daemon's own address.
    if let Some(own) = own_identity
        && (path == own || canonical == Some(own))
    {
        return SocketVerdict::Keep(KeepReason::OwnSocket);
    }
    // 4. Positive liveness: something is listening HERE.
    if census.listening.contains(path) {
        return SocketVerdict::Keep(KeepReason::Listening);
    }
    // 4b. An ALIAS to a listening socket is load-bearing only while a client
    //     of the alias's own version can still exist — i.e. that version is
    //     still installed under `versions/` and a rollback could return to
    //     it. The 2026-09-05 audit measured 845 alias files kept forever on
    //     the GUI host (every version ever deployed, all resolving to the
    //     live daemon), because this arm kept anything that resolved. An
    //     alias for an uninstalled version is litter on the ordinary
    //     re-proved-sighting terms. An unreadable install history keeps
    //     every alias: fail-safe.
    if let Some(target) = canonical
        && census.listening.contains(target)
    {
        match parse_versioned_server_socket_name(path) {
            Some(version) if census.rollback_versions.contains(&version) => {
                return SocketVerdict::Keep(KeepReason::Listening);
            }
            Some(_) if !census.rollback_known => {
                return SocketVerdict::Keep(KeepReason::RollbackSetUnknown);
            }
            _ => match first_seen_dead_ms {
                Some(first) if now_ms.saturating_sub(first) >= SOCKET_DEAD_CONFIRM_MS => {
                    return SocketVerdict::Remove;
                }
                _ => return SocketVerdict::ConfirmLater,
            },
        }
    }

    // 5. A daemon of this exact version is alive somewhere else.
    if census.live_versions.contains(&version) {
        return SocketVerdict::Keep(KeepReason::LiveDaemonVersion);
    }
    // 6. Could not stat it ⇒ keep. (A dangling symlink DOES stat: it is a
    //    readable entry with no target, and it is garbage.)
    if !readable {
        return SocketVerdict::Keep(KeepReason::Unreadable);
    }
    // 7. Dead. Unlink only on a re-proved sighting.
    match first_seen_dead_ms {
        Some(first) if now_ms.saturating_sub(first) >= SOCKET_DEAD_CONFIRM_MS => {
            SocketVerdict::Remove
        }
        _ => SocketVerdict::ConfirmLater,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SocketSweepOutcome {
    pub removed: usize,
    pub awaiting_confirmation: usize,
    pub kept_live: usize,
    /// The census could not be completed; nothing was removed this round.
    pub degraded: bool,
}

impl SocketSweepOutcome {
    pub fn did_anything(&self) -> bool {
        self.removed > 0 || self.awaiting_confirmation > 0 || self.degraded
    }
}

/// Read the dead-sighting ledger. An unreadable or malformed ledger is an empty
/// one, which makes every candidate a first sighting — fail-safe by
/// construction.
fn read_dead_ledger(home_dir: &Path) -> HashMap<String, u64> {
    let mut ledger = HashMap::new();
    let Ok(text) = fs::read_to_string(home_dir.join(DEAD_LEDGER_NAME)) else {
        return ledger;
    };
    for line in text.lines() {
        let Some((name, ms)) = line.split_once('\t') else {
            continue;
        };
        let Ok(ms) = ms.trim().parse::<u64>() else {
            continue;
        };
        ledger.insert(name.to_string(), ms);
    }
    ledger
}

/// Rewrite the ledger from scratch with exactly the entries still dead. A
/// socket that came back to life is simply absent from the new file, so its
/// death clock restarts if it dies again.
fn write_dead_ledger(home_dir: &Path, ledger: &HashMap<String, u64>) {
    let mut lines: Vec<String> = ledger
        .iter()
        .map(|(name, ms)| format!("{name}\t{ms}"))
        .collect();
    lines.sort(); // deterministic file content
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    let _ = fs::write(home_dir.join(DEAD_LEDGER_NAME), body);
}

/// One sweep over `home_dir`'s versioned server sockets.
///
/// `own_socket` is the sweeping daemon's own address, which is never a
/// candidate even if the census somehow missed it.
pub(crate) fn run_socket_sweep(
    home_dir: &Path,
    own_socket: Option<&Path>,
    census: &LiveDaemonCensus,
    now_ms: u64,
) -> SocketSweepOutcome {
    let mut outcome = SocketSweepOutcome {
        degraded: !census.is_complete(),
        ..SocketSweepOutcome::default()
    };

    let previous = read_dead_ledger(home_dir);
    let mut next: HashMap<String, u64> = HashMap::new();
    let own_identity =
        own_socket.map(|own| fs::canonicalize(own).unwrap_or_else(|_| own.to_path_buf()));

    let Ok(entries) = fs::read_dir(home_dir) else {
        return outcome; // no home dir, nothing to sweep
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let canonical = fs::canonicalize(&path).ok();
        let readable = fs::symlink_metadata(&path).is_ok();
        let first_seen = previous.get(&name).copied();
        let verdict = classify_socket_entry(
            &path,
            canonical.as_deref(),
            readable,
            census,
            own_identity.as_deref(),
            first_seen,
            now_ms,
        );
        match verdict {
            SocketVerdict::NotOurs => {}
            SocketVerdict::Keep(_) => outcome.kept_live += 1,
            SocketVerdict::ConfirmLater => {
                next.insert(name, first_seen.unwrap_or(now_ms));
                outcome.awaiting_confirmation += 1;
            }
            SocketVerdict::Remove => {
                if fs::remove_file(&path).is_ok() {
                    outcome.removed += 1;
                    // A removed retired artifact whose pid was our liveness
                    // witness IS a preserved-owner death — the 2026-09-05
                    // audit SIGKILLed a lingerer and NOTHING traced it; the
                    // death gets a name here or the plane stays self-blind.
                    if let Some((_, pid)) = parse_retired_server_socket_artifact(&path) {
                        append_trace_event(
                            home_dir,
                            "daemon",
                            "lifecycle",
                            "retired_owner_gone",
                            serde_json::json!({
                                "pid": pid,
                                "name": name,
                            }),
                        );
                    }
                } else {
                    // Could not unlink — keep the sighting so the next round
                    // retries rather than restarting the clock.
                    next.insert(name, first_seen.unwrap_or(now_ms));
                }
            }
        }
    }
    write_dead_ledger(home_dir, &next);
    outcome
}

/// Interval gate for the chore thread, mirroring
/// `clipboard_sweep::run_clipboard_sweep_if_due`.
pub(crate) fn run_socket_sweep_if_due(
    home_dir: &Path,
    own_socket: Option<&Path>,
    now_ms: u64,
) -> Option<SocketSweepOutcome> {
    let marker = home_dir.join(SWEEP_MARKER_NAME);
    if let Ok(text) = fs::read_to_string(&marker)
        && let Ok(last_ms) = text.trim().parse::<u64>()
        && now_ms.saturating_sub(last_ms) < SOCKET_SWEEP_INTERVAL_MS
    {
        return None;
    }
    let census = LiveDaemonCensus::gather(home_dir);
    let outcome = run_socket_sweep(home_dir, own_socket, &census, now_ms);
    let _ = fs::write(&marker, now_ms.to_string());
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// Unique per-test scratch root — the workspace carries no tempdir crate,
    /// same pattern as `clipboard_sweep`'s tests.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "yggterm-socket-sweep-{tag}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Scratch(root)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const DAY_MS: u64 = 24 * 60 * 60 * 1000;

    fn census_with(listening: &[&Path]) -> LiveDaemonCensus {
        LiveDaemonCensus::from_parts(
            listening.iter().map(|p| p.to_path_buf()).collect(),
            true,
        )
    }

    /// A plain file standing in for a leftover socket inode: the sweep only
    /// ever consults the NAME and the liveness census, never the file type.
    fn touch(path: &Path) {
        fs::write(path, b"").unwrap();
    }

    // ---- the predicate ----

    #[test]
    fn a_live_daemons_socket_is_kept() {
        let tmp = Scratch::new("live");
        let live = tmp.path().join("server-3-0-32.sock");
        touch(&live);
        let census = census_with(&[live.as_path()]);
        assert_eq!(
            classify_socket_entry(
                &live,
                Some(&live),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::Listening),
            "a listening path is never garbage, however old the dead sighting"
        );
    }

    #[test]
    fn an_alias_resolving_to_a_live_daemon_is_kept() {
        let tmp = Scratch::new("alias");
        let live = tmp.path().join("server-3-0-32.sock");
        let alias = tmp.path().join("server-2-1-2.sock");
        touch(&live);
        std::os::unix::fs::symlink(&live, &alias).unwrap();
        // The alias version is INSTALLED (a rollback can return to it), so
        // the alias is serving, not litter — see the ancient-alias test for
        // the uninstalled counterpart ([2026-09-05 audit]).
        let census = census_with(&[live.as_path()])
            .with_rollback_versions([(2, 1, 2), (3, 0, 32)].into_iter().collect(), true);
        assert_eq!(
            classify_socket_entry(
                &alias,
                Some(&live),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::Listening),
            "back-aliases point at the running daemon and are serving, not litter"
        );
    }

    #[test]
    fn an_unparseable_filename_is_never_ours() {
        let tmp = Scratch::new("unparseable");
        let census = census_with(&[]);
        for name in [
            "server.sock",
            "server-3-0.sock",
            "server-3-0-32-1.sock",
            "session-titles.db",
            ".socket-sweep-dead",
        ] {
            let path = tmp.path().join(name);
            assert_eq!(
                classify_socket_entry(&path, None, true, &census, None, Some(0), 100 * DAY_MS),
                SocketVerdict::NotOurs,
                "{name} is not a versioned server socket and must be untouched"
            );
        }
    }

    #[test]
    fn a_listening_handoff_socket_is_kept() {
        let tmp = Scratch::new("handoff-live");
        let handoff = tmp.path().join("pty-handoff-3-2-71.sock");
        let census = census_with(&[handoff.as_path()]);
        assert_eq!(
            classify_socket_entry(
                &handoff,
                Some(&handoff),
                true,
                &census,
                None,
                None,
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::Listening),
            "a bound handoff listener belongs to a live daemon — never garbage"
        );
    }

    #[test]
    fn a_handoff_socket_of_a_live_version_elsewhere_is_kept() {
        let tmp = Scratch::new("handoff-version");
        let handoff = tmp.path().join("pty-handoff-3-2-71.sock");
        // A same-version successor REBINDS the same name, so between one
        // daemon's exit and the next's bind the file can be unbound while its
        // version is still alive — the census's live-version arm covers that
        // window, and the bound-elsewhere case (`pty-handoff` of a version
        // whose daemon listens on a different socket file) besides.
        let server = tmp.path().join("server-3-2-71.sock");
        let census = census_with(&[server.as_path()]);
        assert_eq!(
            classify_socket_entry(
                &handoff,
                Some(&handoff),
                true,
                &census,
                None,
                None,
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::LiveDaemonVersion),
            "a live daemon of this version may rebind the name at any moment"
        );
    }

    #[test]
    fn a_dead_version_handoff_socket_is_removed_after_confirmation() {
        let tmp = Scratch::new("handoff-dead");
        let handoff = tmp.path().join("pty-handoff-3-0-101.sock");
        let live = tmp.path().join("server-3-2-71.sock");
        let census = census_with(&[live.as_path()]);
        let now = 100 * DAY_MS;
        assert_eq!(
            classify_socket_entry(&handoff, Some(&handoff), true, &census, None, None, now),
            SocketVerdict::ConfirmLater,
            "a first sighting never deletes"
        );
        assert_eq!(
            classify_socket_entry(
                &handoff,
                Some(&handoff),
                true,
                &census,
                None,
                Some(now - SOCKET_DEAD_CONFIRM_MS),
                now
            ),
            SocketVerdict::Remove,
            "a version no daemon holds, dead across the confirm window, is the graveyard"
        );
    }

    #[test]
    fn an_incomplete_census_keeps_every_handoff_socket() {
        let tmp = Scratch::new("handoff-degraded");
        let handoff = tmp.path().join("pty-handoff-3-0-101.sock");
        let census = LiveDaemonCensus::from_parts(HashSet::new(), false);
        assert_eq!(
            classify_socket_entry(
                &handoff,
                Some(&handoff),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::CensusIncomplete),
            "\"I could not look\" keeps the file, however dead its name looks"
        );
    }

    #[test]
    fn a_dead_socket_of_a_version_no_daemon_holds_is_removed_after_confirmation() {
        let tmp = Scratch::new("dead");
        let live = tmp.path().join("server-3-0-32.sock");
        let dead = tmp.path().join("server-2-12-5.sock");
        let census = census_with(&[live.as_path()]);
        let now = 100 * DAY_MS;
        assert_eq!(
            classify_socket_entry(&dead, Some(&dead), true, &census, None, None, now),
            SocketVerdict::ConfirmLater,
            "a first sighting never deletes — the mid-restart window lives here"
        );
        assert_eq!(
            classify_socket_entry(
                &dead,
                Some(&dead),
                true,
                &census,
                None,
                Some(now - SOCKET_DEAD_CONFIRM_MS + 1),
                now
            ),
            SocketVerdict::ConfirmLater,
            "a sighting younger than the confirmation window is not yet proof"
        );
        assert_eq!(
            classify_socket_entry(
                &dead,
                Some(&dead),
                true,
                &census,
                None,
                Some(now - SOCKET_DEAD_CONFIRM_MS),
                now
            ),
            SocketVerdict::Remove
        );
    }

    #[test]
    fn an_incomplete_census_keeps_everything() {
        let tmp = Scratch::new("degraded");
        let dead = tmp.path().join("server-2-12-5.sock");
        let census = LiveDaemonCensus::from_parts(HashSet::new(), false);
        assert!(!census.is_complete());
        assert_eq!(
            classify_socket_entry(
                &dead,
                Some(&dead),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::CensusIncomplete),
            "a probe we could not complete must never read as 'nothing is alive'"
        );
    }

    #[test]
    fn an_unstattable_entry_is_kept() {
        let tmp = Scratch::new("unreadable");
        let dead = tmp.path().join("server-2-12-5.sock");
        let census = census_with(&[]);
        assert_eq!(
            classify_socket_entry(
                &dead,
                None,
                false,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::Unreadable),
            "absence of proof keeps the file"
        );
    }

    #[test]
    fn the_sweeping_daemons_own_socket_is_kept_even_without_a_census_hit() {
        let tmp = Scratch::new("own");
        let own = tmp.path().join("server-3-0-33.sock");
        touch(&own);
        let census = census_with(&[]);
        assert_eq!(
            classify_socket_entry(
                &own,
                Some(&own),
                true,
                &census,
                Some(&own),
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::OwnSocket)
        );
    }

    #[test]
    fn a_version_held_by_a_live_daemon_bound_elsewhere_is_kept() {
        let tmp = Scratch::new("liveversion");
        let live = tmp.path().join("server-3-0-32.sock");
        let same_version_elsewhere = tmp.path().join("sub").join("server-3-0-32.sock");
        let census = census_with(&[live.as_path()]);
        assert_eq!(
            classify_socket_entry(
                &same_version_elsewhere,
                Some(&same_version_elsewhere),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::LiveDaemonVersion)
        );
    }

    // ---- a same-version bequest's retired artifacts ----

    /// A retired pid that is provably alive under test: this very process.
    fn a_live_pid() -> u32 {
        std::process::id()
    }

    /// A pid that is almost certainly gone: wait(2) has reaped it. Pid reuse
    /// would need wraparound inside the test's microsecond window.
    fn a_dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id();
        child.wait().expect("reap true");
        pid
    }

    #[test]
    fn a_retired_artifact_of_a_live_pid_is_kept_even_when_its_version_looks_dead() {
        let tmp = Scratch::new("retired-live");
        let retired = tmp
            .path()
            .join(format!("server-3-2-50.sock.retired-{}", a_live_pid()));
        let census = census_with(&[]);
        assert_eq!(
            classify_socket_entry(
                &retired,
                None,
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::RetiredOwnerAlive),
            "a draining lingerer's retired path is load-bearing — the registry \
             hands adopters this exact name — whatever the census says"
        );
    }

    #[test]
    fn an_ancient_alias_to_a_live_socket_dies_after_the_confirm_window() {
        // [2026-09-05 audit] 845 alias symlinks — every version ever deployed,
        // all resolving to the live daemon — were kept forever by the old
        // resolve-and-keep rule. An alias is load-bearing only for a version
        // still installed; the rest are litter on the usual terms.
        let tmp = Scratch::new("ancient-alias");
        let live = tmp.path().join("server-3-2-61.sock");
        let alias = tmp.path().join("server-2-8-96.sock");
        let census = LiveDaemonCensus::from_parts([live.clone()].into_iter().collect(), true)
            .with_rollback_versions([(3, 2, 61)].into_iter().collect(), true);
        assert_eq!(
            classify_socket_entry(
                &alias,
                Some(&live),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Remove,
            "an alias for an uninstalled version cannot serve any rollback"
        );
    }

    #[test]
    fn an_alias_for_an_installed_version_is_kept() {
        let tmp = Scratch::new("rollback-alias");
        let live = tmp.path().join("server-3-2-61.sock");
        let alias = tmp.path().join("server-3-2-60.sock");
        let census = LiveDaemonCensus::from_parts([live.clone()].into_iter().collect(), true)
            .with_rollback_versions([(3, 2, 60), (3, 2, 61)].into_iter().collect(), true);
        assert_eq!(
            classify_socket_entry(&alias, Some(&live), true, &census, None, Some(0), 100 * DAY_MS),
            SocketVerdict::Keep(KeepReason::Listening),
            "a rollback can still return to this version; its alias is load-bearing"
        );
    }

    #[test]
    fn an_alias_is_kept_while_the_install_history_is_unreadable() {
        let tmp = Scratch::new("unknown-rollback");
        let live = tmp.path().join("server-3-2-61.sock");
        let alias = tmp.path().join("server-2-8-96.sock");
        let census = LiveDaemonCensus::from_parts([live.clone()].into_iter().collect(), true);
        assert_eq!(
            classify_socket_entry(&alias, Some(&live), true, &census, None, Some(0), 100 * DAY_MS),
            SocketVerdict::Keep(KeepReason::RollbackSetUnknown),
            "could not look at versions/ ⇒ absence of proof is never a deletion"
        );
    }

    #[test]
    fn an_ancient_version_lock_file_dies_after_the_confirm_window() {
        let tmp = Scratch::new("ancient-lock");
        let lock = tmp.path().join("server-2-4-31.sock.lock");
        let census = LiveDaemonCensus::from_parts(HashSet::new(), true)
            .with_rollback_versions([(3, 2, 61)].into_iter().collect(), true);
        assert_eq!(
            classify_socket_entry(&lock, Some(&lock), true, &census, None, Some(0), 100 * DAY_MS),
            SocketVerdict::Remove,
            "a lock for a version that is neither live nor installed is litter"
        );
    }

    #[test]
    fn the_live_daemons_lock_is_kept() {
        let tmp = Scratch::new("live-lock");
        let sock = tmp.path().join("server-3-2-61.sock");
        let lock = tmp.path().join("server-3-2-61.sock.lock");
        let census = LiveDaemonCensus::from_parts([sock.clone()].into_iter().collect(), true)
            .with_rollback_versions([(3, 2, 61)].into_iter().collect(), true);
        assert_eq!(
            classify_socket_entry(&lock, Some(&lock), true, &census, None, None, 100 * DAY_MS),
            SocketVerdict::Keep(KeepReason::LiveDaemonVersion),
            "the lock rides its socket's liveness"
        );
    }

    #[test]
    fn a_retired_lock_file_is_judged_like_the_retired_socket() {
        let tmp = Scratch::new("retired-lock");
        let retired_lock = tmp
            .path()
            .join(format!("server-3-2-50.sock.lock.retired-{}", a_live_pid()));
        let census = census_with(&[]);
        assert_eq!(
            classify_socket_entry(
                &retired_lock,
                None,
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::RetiredOwnerAlive),
            "the bequest renames the lock first; a live retiree holds its flock"
        );
    }

    #[test]
    fn a_retired_artifact_of_a_dead_pid_dies_by_the_same_confirmation() {
        let tmp = Scratch::new("retired-dead");
        let retired = tmp
            .path()
            .join(format!("server-3-2-50.sock.retired-{}", a_dead_pid()));
        let census = census_with(&[]);
        let now = 100 * DAY_MS;
        assert_eq!(
            classify_socket_entry(&retired, None, true, &census, None, None, now),
            SocketVerdict::ConfirmLater,
            "first sighting of a drained lingerer's litter records, never deletes"
        );
        assert_eq!(
            classify_socket_entry(
                &retired,
                None,
                true,
                &census,
                None,
                Some(now - SOCKET_DEAD_CONFIRM_MS),
                now
            ),
            SocketVerdict::Remove,
            "a re-proved dead pid is the same garbage rule as everywhere else"
        );
    }

    #[test]
    fn a_retired_artifact_is_judged_by_its_pid_even_when_the_census_is_degraded() {
        let tmp = Scratch::new("retired-degraded");
        let retired = tmp
            .path()
            .join(format!("server-3-2-50.sock.retired-{}", a_dead_pid()));
        let degraded = LiveDaemonCensus::from_parts(HashSet::new(), false);
        assert_eq!(
            classify_socket_entry(
                &retired,
                None,
                true,
                &degraded,
                None,
                None,
                100 * DAY_MS
            ),
            SocketVerdict::ConfirmLater,
            "the pid witness does not need the kernel socket table; an unreadable \
             census must not read as 'keep forever' for names that prove themselves"
        );
    }

    #[test]
    fn a_retired_name_with_a_garbage_pid_is_not_ours() {
        let tmp = Scratch::new("retired-garbage");
        let census = census_with(&[]);
        for name in [
            "server-3-2-50.sock.retired-abc",
            "server-3-2-50.sock.retired-0",
            "server-3-2-50.sock.lock.retired-",
            "server-abc.sock.retired-42",
        ] {
            let path = tmp.path().join(name);
            assert_eq!(
                classify_socket_entry(&path, None, true, &census, None, Some(0), 100 * DAY_MS),
                SocketVerdict::NotOurs,
                "{name} does not carry a usable pid witness and must be untouched"
            );
        }
    }

    // ---- the kernel table parser ----

    #[test]
    fn proc_net_unix_yields_only_listening_filesystem_paths() {
        let text = "\
Num       RefCount Protocol Flags    Type St Inode Path
0000000000000000: 00000002 00000000 00010000 0001 01 123456 /home/user/.yggterm/server-3-0-32.sock
0000000000000000: 00000003 00000000 00000000 0001 03 123457 /home/user/.yggterm/server-3-0-32.sock
0000000000000000: 00000002 00000000 00010000 0001 01 123458 @/tmp/.X11-unix/X0
0000000000000000: 00000002 00000000 00010000 0001 01 123459
0000000000000000: 00000002 00000000 00010000 0001 01 123460 /run/user/1000/bus
";
        let listening = parse_listening_unix_socket_paths(text);
        assert!(listening.contains(Path::new("/home/user/.yggterm/server-3-0-32.sock")));
        assert!(listening.contains(Path::new("/run/user/1000/bus")));
        assert_eq!(
            listening.len(),
            2,
            "connected peers, abstract sockets and pathless rows are not listeners"
        );
    }

    #[test]
    fn a_real_listener_shows_up_in_the_gathered_census() {
        let tmp = Scratch::new("gather");
        let path = tmp.path().join("server-9-9-9.sock");
        let _listener = UnixListener::bind(&path).unwrap();
        let census = LiveDaemonCensus::gather(tmp.path());
        if !census.is_complete() {
            return; // non-Linux unix: the census correctly refuses to guess
        }
        assert!(census.live_socket_count() >= 1);
        assert_eq!(
            classify_socket_entry(
                &path,
                Some(&path),
                true,
                &census,
                None,
                Some(0),
                100 * DAY_MS
            ),
            SocketVerdict::Keep(KeepReason::Listening),
            "a socket this process is really listening on must survive the sweep"
        );
    }

    // ---- the round trip over a directory ----

    #[test]
    fn two_rounds_are_required_before_a_dead_socket_is_unlinked() {
        let tmp = Scratch::new("rounds");
        let home = tmp.path();
        let live = home.join("server-3-0-32.sock");
        let listener = UnixListener::bind(&live).unwrap();
        let dead = home.join("server-2-12-5.sock");
        let alias = home.join("server-2-1-2.sock");
        touch(&dead);
        std::os::unix::fs::symlink(&live, &alias).unwrap();
        fs::write(home.join("session-titles.db"), b"not a socket").unwrap();

        let census = LiveDaemonCensus::from_parts(
            [live.clone()].into_iter().collect(),
            true,
        );
        let now = 100 * DAY_MS;

        // `own_socket` is deliberately None: the live socket and its alias must
        // survive on the LISTENING proof alone. Passing `Some(&live)` here would
        // spare the alias by the own-socket rule instead (it canonicalizes onto
        // `live`) and the round trip would prove nothing about aliases.
        let first = run_socket_sweep(home, None, &census, now);
        assert_eq!(first.removed, 0, "round one may never delete");
        assert_eq!(first.awaiting_confirmation, 1);
        assert!(dead.exists());

        let second = run_socket_sweep(home, None, &census, now + SOCKET_DEAD_CONFIRM_MS);
        assert_eq!(second.removed, 1);
        assert!(!dead.exists(), "the confirmed-dead socket is gone");
        assert!(live.exists(), "the live daemon's socket survives");
        assert!(alias.exists(), "an alias onto a live daemon survives");
        assert!(
            home.join("session-titles.db").exists(),
            "a file that is not a versioned socket is never touched"
        );
        drop(listener);
    }

    #[test]
    fn a_socket_that_comes_back_to_life_loses_its_death_mark() {
        let tmp = Scratch::new("resurrect");
        let home = tmp.path();
        let path = home.join("server-2-12-5.sock");
        touch(&path);
        let now = 100 * DAY_MS;

        let dead_census = LiveDaemonCensus::from_parts(HashSet::new(), true);
        let first = run_socket_sweep(home, None, &dead_census, now);
        assert_eq!(first.awaiting_confirmation, 1);

        // The version comes back — a daemon binds it again.
        let live_census = LiveDaemonCensus::from_parts([path.clone()].into_iter().collect(), true);
        let second = run_socket_sweep(home, None, &live_census, now + SOCKET_DEAD_CONFIRM_MS);
        assert_eq!(second.removed, 0);
        assert_eq!(second.kept_live, 1);

        // …and dies again: the clock restarts, so the very next round after the
        // old deadline must still not delete it.
        let third = run_socket_sweep(home, None, &dead_census, now + SOCKET_DEAD_CONFIRM_MS + 1);
        assert_eq!(third.removed, 0, "resurrection resets the death clock");
        assert_eq!(third.awaiting_confirmation, 1);
        assert!(path.exists());
    }

    #[test]
    fn an_incomplete_census_removes_nothing_over_a_whole_directory() {
        let tmp = Scratch::new("degraded-round");
        let home = tmp.path();
        let dead = home.join("server-2-12-5.sock");
        touch(&dead);
        let census = LiveDaemonCensus::from_parts(HashSet::new(), false);
        let now = 100 * DAY_MS;
        let first = run_socket_sweep(home, None, &census, now);
        let second = run_socket_sweep(home, None, &census, now + 10 * DAY_MS);
        assert!(first.degraded && second.degraded);
        assert_eq!(second.removed, 0);
        assert!(dead.exists());
    }

    #[test]
    fn a_dangling_alias_is_collected_like_any_other_dead_socket() {
        let tmp = Scratch::new("dangling");
        let home = tmp.path();
        let alias = home.join("server-2-1-2.sock");
        std::os::unix::fs::symlink(home.join("server-2-9-9.sock"), &alias).unwrap();
        let census = LiveDaemonCensus::from_parts(HashSet::new(), true);
        let now = 100 * DAY_MS;
        assert_eq!(run_socket_sweep(home, None, &census, now).removed, 0);
        let second = run_socket_sweep(home, None, &census, now + SOCKET_DEAD_CONFIRM_MS);
        assert_eq!(second.removed, 1);
        assert!(fs::symlink_metadata(&alias).is_err());
    }

    #[test]
    fn sweep_is_interval_gated_by_the_marker_file() {
        let tmp = Scratch::new("interval");
        let home = tmp.path();
        let now = 100 * DAY_MS;
        assert!(run_socket_sweep_if_due(home, None, now).is_some());
        assert!(
            run_socket_sweep_if_due(home, None, now + 1000).is_none(),
            "a second sweep inside the interval must not run"
        );
        assert!(run_socket_sweep_if_due(home, None, now + SOCKET_SWEEP_INTERVAL_MS + 1).is_some());
    }
}
