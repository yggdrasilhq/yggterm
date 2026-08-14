//! The fabricated-summary detector must keep both of its arms.
//!
//! `scripts/audit-summary-grounding.py` is what tells a summary that describes
//! the session from one that describes a project nobody here has. A detector is
//! only worth something if it has been shown to do BOTH — to fire on a summary
//! that invented its subject, and to stay quiet on an honest paraphrase. A rule
//! that flagged everything would satisfy the first half on its own and read as
//! diligence.
//!
//! ⚠ The quiet arm is the fragile one and the reason this runs in the suite. The
//! detector compares word STEMS, because a summariser rewriting "evicts" as
//! "evicting" is doing its job, not inventing; drop the stemming and the honest
//! arm fails while the loud arm keeps passing, which looks like a stricter tool
//! rather than a broken one.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;

#[test]
fn the_summary_grounding_detector_both_fires_and_stays_quiet() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve");
    let script = repo.join("scripts").join("audit-summary-grounding.py");
    if !script.exists() {
        // A packaged crate has no scripts/ beside it; nothing to enforce there.
        return;
    }

    let Ok(out) = Command::new("python3")
        .arg(&script)
        .arg("--selftest")
        .current_dir(&repo)
        .output()
    else {
        // No python3 here: failing would only punish a missing interpreter.
        return;
    };

    assert!(
        out.status.success(),
        "the summary-grounding detector no longer separates a fabricated summary \
         from an honest one.\n\nstdout:\n{}\nstderr:\n{}\n\nBoth arms must hold. \
         If the rule changed on purpose, RE-MEASURE the threshold against a real \
         store and update the constant's comment with the new numbers — a \
         threshold carried over from a different rule is worse than none.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
