//! Where a RUNNING process publishes the source it was built from.
//!
//! ⛔ IT EXISTS BECAUSE `--build-commit` CAN ONLY ANSWER FOR A BINARY YOU CAN
//! STILL EXECUTE. The stamp closed *"which source is this file?"* and left
//! *"which source is this process?"* open, and those are different questions the
//! moment a deploy replaces a binary under a live process — which is the normal
//! case, not an edge one. Measured on the desktop host 2026-08-13: the GUI's
//! `/proc/<pid>/exe` hashed to a value that matched NO file on the machine, so
//! the running build could not be named at all, while the on-disk copy answered
//! a commit confidently. One number stood in for two planes, which is the exact
//! failure the stamp was added to end.
//!
//! ⇒ A process that is already running has to SAY its identity, because nothing
//! outside it can still derive one. The value is compiled in, so it describes
//! the code that is executing rather than whatever now sits at the path it was
//! loaded from.
//!
//! ⚠ THE VALUE HAS ONE OWNER AND THIS IS NOT IT. `apps/yggterm/build.rs` stamps
//! it and `apps/yggterm::build_identity` reads it; both binaries are built from
//! that one crate, so both carry the same stamp. This module only *distributes*
//! it to the library crates that report on the wire, and it is deliberately not
//! another `option_env!`: a second read site in a library would compile to
//! `None` (the env is set for the binary crate alone) and publish `unstamped`
//! forever while looking like a source of truth.
//!
//! ⛔ AND THE FALLBACK IS NAMED, NOT FAKED. A process that never declared its
//! commit reports `unstamped` — a state the census must be able to see and
//! refuse, because a plausible-looking wrong commit is worse than an admitted
//! gap.

use std::sync::OnceLock;

/// What a process reports before (or without) declaring its build.
///
/// Deliberately not a sha-shaped string: a census comparing this against a git
/// rev must fail to match, and must be able to say WHY it did not.
pub const UNSTAMPED: &str = "unstamped";

static BUILD_COMMIT: OnceLock<&'static str> = OnceLock::new();

/// Publish the commit this process was built from. Called once, early, by each
/// binary's `main` — the only place the stamp is visible.
///
/// Idempotent and first-write-wins, so a re-entrant startup path cannot make the
/// reported identity change under a reader mid-run.
pub fn declare_build_commit(commit: &'static str) {
    let _ = BUILD_COMMIT.set(commit);
}

/// The commit this process was built from, or [`UNSTAMPED`].
pub fn build_commit() -> &'static str {
    BUILD_COMMIT.get().copied().unwrap_or(UNSTAMPED)
}

/// Stamp the terminal-identity env vars with THIS build's own values.
///
/// ⛔ IT EXISTS BECAUSE AN INHERITED IDENTITY LIES, AND THE FLEET READS IT.
/// A GUI relaunched by a supervisor, a roll script, or its own convergence
/// restart inherits the launcher's environment — including
/// `TERM_PROGRAM_VERSION` — and every version census that reads a process's
/// `/proc/<pid>/environ` then reports the GRANDPARENT's version as this
/// process's. Measured live 2026-08-27 on the desktop host: a GUI running
/// 3.1.64 code (its `/proc/<pid>/exe` resolved to the 3.1.64 file) carried
/// `TERM_PROGRAM_VERSION=3.1.60` in its own environ, three versions stale,
/// purely because the process chain that relaunched it was older. The same
/// session had already used that exact env var to version-check processes.
///
/// Called once, early, by each binary's `main` — the same single-threaded
/// moment `declare_build_commit` needs (`set_var` is unsound once a thread
/// exists). Always OVERWRITES with this build's truth; there is no legitimate
/// inherited value to preserve, because the one thing this variable must never
/// be is older than the code that is running.
pub fn stamp_terminal_identity_env() {
    // SAFETY: single-threaded startup, per the caller contract above and the
    // same guarantee `declare_build_commit`'s call sites already rely on.
    unsafe {
        std::env::set_var("TERM_PROGRAM", EXPORTED_TERM_PROGRAM);
        std::env::set_var("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        std::env::set_var("YGGTERM_TERM_PROGRAM", YGGTERM_TERM_PROGRAM);
    }
}

/// What rows export as `TERM_PROGRAM`. Same value the terminal-spawn path uses,
/// so a shell integration sniffing the variable sees one answer everywhere.
use crate::managed_cli::{EXPORTED_TERM_PROGRAM, YGGTERM_TERM_PROGRAM};

#[cfg(test)]
mod tests {
    use super::*;

    /// ⛔ BOTH CONTROLS, ONE RUN. A reader that has collapsed to a constant
    /// cannot be caught by testing the answer you hoped for: proving it can say
    /// `unstamped` is worthless without proving it can also say a real commit,
    /// and vice versa. The `OnceLock` makes the order load-bearing, so the two
    /// assertions have to live in one test.
    #[test]
    fn it_reports_unstamped_until_a_build_declares_itself() {
        assert_eq!(
            build_commit(),
            UNSTAMPED,
            "an undeclared process must admit it has no identity rather than \
             invent one"
        );
        declare_build_commit("0123456789ab");
        assert_eq!(build_commit(), "0123456789ab");
        // First write wins: a later declaration cannot move the identity of a
        // process that is already answering questions about itself.
        declare_build_commit("ffffffffffff");
        assert_eq!(build_commit(), "0123456789ab");
    }

    /// Both binaries' mains must stamp the terminal identity env. A future
    /// entry point that skips it re-imports the measured lie: a GUI running
    /// current code while its own environ names a version three behind, which
    /// is exactly what every `/proc/<pid>/environ` version census then reads.
    #[test]
    fn both_entry_points_stamp_the_terminal_identity_env() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for path in [
            manifest_dir.join("../../apps/yggterm/src/main.rs"),
            manifest_dir.join("../../apps/yggterm/src/bin/yggterm-headless.rs"),
        ] {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert!(
                source.contains("declare_build_commit")
                    && source.contains("stamp_terminal_identity_env"),
                "{} must declare its build commit AND stamp the terminal \
                 identity env — an unstamped environ lies about the running code",
                path.display()
            );
        }
    }
}
