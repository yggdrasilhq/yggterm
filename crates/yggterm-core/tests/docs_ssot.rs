//! The docs SSOT law is enforced by the suite, not by good intentions.
//!
//! On 2026-08-02 a session was spent reporting five bugs the user had already
//! fixed, because three files each claimed to answer "what is open" and the one
//! being read was stale. `docs/docs-ssot.md` states the rule; this test is what
//! makes it hold. See `scripts/check-docs-ssot.sh` for the three conditions.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;

#[test]
fn the_bug_file_lists_only_open_items() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve");
    let script = repo.join("scripts").join("check-docs-ssot.sh");
    if !script.exists() {
        // A packaged crate has no scripts/ beside it; nothing to enforce there.
        return;
    }

    let out = Command::new("bash")
        .arg(&script)
        .current_dir(&repo)
        .output()
        .expect("check-docs-ssot.sh must be runnable");

    assert!(
        out.status.success(),
        "docs SSOT violated — the bug queue is lying, which is the one thing it \
         may not do.\n\nstdout:\n{}\nstderr:\n{}\n\nFix the docs, not this test: \
         a fixed entry is DELETED (git remembers it), every entry declares one \
         Status, and no second file may reproduce the queue. See docs/docs-ssot.md.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
