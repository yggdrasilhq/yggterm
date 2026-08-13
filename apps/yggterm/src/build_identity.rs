//! The one owner of *"which source built this binary?"*, shared by both bins.
//!
//! ⛔ IT EXISTS BECAUSE THE VERSION CANNOT ANSWER THAT QUESTION. Nothing
//! arbitrates a version number: two clusters that read `Cargo.toml` before
//! either pushed take the same one, and on 2026-08-13 four consecutive numbers
//! each meant two builds. A deploy from a pre-rebase tree lands over another
//! cluster's fix, the GUI re-execs (pid unchanged, `/proc/<pid>/exe` clean), and
//! the live probe reads RED against a binary that never carried the fix.
//!
//! ⇒ `--version` answers *"which release line is this?"* and stays exactly as
//! it was, because daemons rendezvous on it. `--build-commit` answers *"which
//! source is this?"*, and it is the only one of the two that is an identity.
//! The check an agent actually wants, on the host that runs the binary:
//!
//! ```text
//! [ "$(yggterm --build-commit)" = "$(git rev-parse --short=12 HEAD)" ]
//! ```

/// The commit this binary was built from, stamped by `build.rs`.
///
/// `unknown` when built outside a git checkout (a packaged crate, a source
/// tarball) — a real state, reported rather than faked. A `-dirty` suffix means
/// the tree carried uncommitted work at build time, so the commit names the
/// nearest ancestor and not the source.
pub fn build_commit() -> &'static str {
    option_env!("YGGTERM_BUILD_COMMIT").unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⛔ Asserting on the PAYLOAD, not on the wrapper. A test that only asked
    /// "did a string come back" passes just as happily against the fallback,
    /// which is the failure this stamp exists to make visible: an unstamped
    /// build reporting an identity it does not have.
    #[test]
    fn the_build_states_the_commit_it_was_built_from() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.git");
        if !repo.exists() {
            // Built from a tarball or a packaged crate: `unknown` is the
            // truthful answer there, and demanding a sha would only punish it.
            return;
        }
        let commit = build_commit();
        assert_ne!(
            commit, "unknown",
            "this build carries no commit stamp — build.rs did not run its git \
             probe, so the fleet census cannot name which source is on a host"
        );
        let core = commit.strip_suffix("-dirty").unwrap_or(commit);
        assert!(
            core.len() >= 7 && core.chars().all(|c| c.is_ascii_hexdigit()),
            "build commit {commit:?} is not a git object id"
        );
    }
}
