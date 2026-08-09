//! The architecture guard must run in the suite, not only in CI.
//!
//! On 2026-08-02 the 3.0.0 separation took `crates/yggui` out to libyggterm
//! (3a51d499). Four assertions still named `crates/yggui/src/theme.rs`, so
//! `scripts/check_architecture_contracts.py` raised `FileNotFoundError` on the
//! 5th of its 10 check groups — and the five after it, including the hot-update
//! and GUI-binary contracts, never ran again. It went unnoticed for seven days
//! because the only thing that ran the script was a CI step nobody was reading.
//!
//! ⇒ A guard whose own breakage is invisible to `cargo test` is a guard you find
//! out about later. This is the sibling of `docs_ssot.rs` and exists for the same
//! reason: the law is enforced by the suite, not by good intentions.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;

#[test]
fn the_architecture_contracts_still_have_subjects_to_check() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve");
    let script = repo
        .join("scripts")
        .join("check_architecture_contracts.py");
    if !script.exists() {
        // A packaged crate has no scripts/ beside it; nothing to enforce there.
        return;
    }

    let Ok(out) = Command::new("python3")
        .arg(&script)
        .current_dir(&repo)
        .output()
    else {
        // No python3 on this machine: the CI step still covers it, and failing
        // here would only punish a developer for a missing interpreter.
        return;
    };

    assert!(
        out.status.success(),
        "architecture contracts violated.\n\nstdout:\n{}\nstderr:\n{}\n\n\
         If a failure says SUBJECT FILE IS MISSING, the contract outlived the \
         file it guards: re-point it if the file moved, or delete it and say \
         where the invariant went. Do not leave it dangling — a dangling \
         contract used to abort the whole run and take every later check with \
         it. See scripts/check_architecture_contracts.py.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
