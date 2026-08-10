//! A settled call is law, and the suite is what makes that true.
//!
//! On 2026-08-10 the owner had settled *"always restart the GUI, do not ask"* in
//! `docs/settled-calls.md`. A session quoted that rule correctly, obeyed it once,
//! and then stopped restarting anyway — because a neighbouring note in
//! `pending-bugs.md` said "one deploy per session", and a DAEMON-binary rule got
//! read onto a GUI action. He had to ask *"why did that steer not work?"*.
//!
//! ⛔ The lesson is why this file exists at all: **the rule was already written,
//! and writing it again is not a fix.** A steer that lives only in prose can be
//! outweighed by neighbouring prose, and nobody finds out until the owner
//! notices the behaviour. Only something that FAILS can hold a rule in place.
//! See `scripts/check-settled-calls.sh` for the two conditions it can check
//! exactly — it is deliberately narrow, because a guard that cries wolf gets
//! bypassed and that is worse than no guard.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;

#[test]
fn no_document_re_scopes_a_settled_call() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve");
    let script = repo.join("scripts").join("check-settled-calls.sh");
    if !script.exists() {
        // A packaged crate has no scripts/ beside it; nothing to enforce there.
        return;
    }

    let out = Command::new("bash")
        .arg(&script)
        .current_dir(&repo)
        .output()
        .expect("check-settled-calls.sh must be runnable");

    assert!(
        out.status.success(),
        "a document re-scopes something the owner settled.\n\nstdout:\n{}\nstderr:\n{}\n\n\
         Fix the DOCUMENT, not this test. Two rules: (1) \"deploy per session\" is a \
         DAEMON-binary rule and must say so, because a GUI restart has none of that \
         blast radius and the owner has settled that it needs no permission; (2) only \
         docs/settled-calls.md may gate a GUI restart on permission, and only for an \
         ACTIVE ychrome row. If you believe a new gate is warranted, it is his call to \
         make and it belongs in settled-calls.md with his words.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
