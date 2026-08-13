//! Lock: this repo is PUBLIC and must not carry the maintainer's private
//! working material. `scripts/check-privacy.sh` is the law; this test is what
//! makes it enforced rather than advisory.
//!
//! Sibling of `docs_ssot.rs`. See `AGENTS.md` §PRIVACY for why it exists: the
//! leak vector is not a credential, it is an agent writing a REAL example into
//! a fixture. No secret scanner catches that; a shape-matching checker does.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<crate> has a repo root two levels up")
        .to_path_buf()
}

#[test]
fn the_public_repo_carries_no_private_material() {
    let root = repo_root();
    let script = root.join("scripts/check-privacy.sh");
    assert!(script.is_file(), "missing {}", script.display());

    let out = Command::new("bash")
        .arg(&script)
        .current_dir(&root)
        .output()
        .expect("run scripts/check-privacy.sh");

    if !out.status.success() {
        panic!(
            "privacy check failed — this repo is public.\n\
             INVENT the example instead of copying a real one; see AGENTS.md §PRIVACY.\n\
             \n--- stderr ---\n{}\n--- stdout ---\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        );
    }
}

/// Build a throwaway git repo carrying `scripts/check-privacy.sh` plus one
/// fixture, run the checker in it, and report whether it refused.
///
/// Isolated on purpose: writing probe files into the real tree would race
/// other tests and leave residue in a shared checkout.
fn checker_refuses(fixture: &str) -> (bool, String) {
    let root = repo_root();
    let tmp = std::env::temp_dir().join(format!(
        "ygg-privacy-probe-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("scripts")).expect("scratch dirs");
    std::fs::copy(
        root.join("scripts/check-privacy.sh"),
        tmp.join("scripts/check-privacy.sh"),
    )
    .expect("copy checker");
    std::fs::write(tmp.join("probe.md"), fixture).expect("write fixture");

    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&tmp)
            .output()
            .expect("git")
    };
    git(&["init", "-q"]);
    // The checker scans tracked AND untracked-but-not-ignored files, so an
    // `init` alone is enough; no commit needed.

    let out = Command::new("bash")
        .arg(tmp.join("scripts/check-privacy.sh"))
        .current_dir(&tmp)
        .output()
        .expect("run checker");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = std::fs::remove_dir_all(&tmp);
    (!out.status.success(), report)
}

/// ⛔ THE HALF THAT WAS MISSING. The test above only ever observes the checker
/// PASS, so a checker that had been quietly broken — or one that always exits
/// 0 — would satisfy it forever. On 2026-08-13 that was not hypothetical:
/// three separate blind spots were found while the suite was green.
///
/// Every case below is a defect that was live in this repo, and each has its
/// opposite in `the_checker_does_not_flag_invented_examples`, so the gate
/// cannot pass by simply refusing everything.
#[test]
fn the_checker_actually_refuses_what_it_claims_to_catch() {
    // ⛔ EVERY FIXTURE IS ASSEMBLED AT RUNTIME. A test that proves the checker
    // catches X cannot contain a literal X, or the checker catches its own
    // test file and this suite can never be green. Writing them out directly
    // failed exactly that way — and one of them was a REAL address, i.e. the
    // test for the leak rule was itself the leak. Keep the pieces inert.
    let name = format!("/home/{}", "zzprobename");
    let lan = format!("10.{}.12.13", 11);

    // 1. A bare home path with NO trailing slash. The detector required one,
    //    so `/home/<name>` in prose sailed through and five occurrences sat on
    //    the public branch while the checker reported "ok".
    let (refused, report) = checker_refuses(&format!("a path in prose: {name}\n"));
    assert!(
        refused,
        "checker accepted a bare /home/<name> with no trailing slash.\n{report}"
    );

    // 2. A real path sharing its LINE with an allowlisted placeholder. The
    //    allowlist was applied per LINE, so the single line most likely to
    //    quote a real path — the one documenting a scrub — was the one line
    //    guaranteed to pass.
    let (refused, report) =
        checker_refuses(&format!("fixtures (`{name}` -> `/home/user` in src/x.rs)\n"));
    assert!(
        refused,
        "a placeholder on the same line laundered a real home path.\n{report}"
    );

    // 3. A private LAN address must still be caught (the class is unchanged;
    //    this pins it so the rewrite above cannot have cost us the other rules).
    let (refused, report) = checker_refuses(&format!("the box answers on {lan}\n"));
    assert!(refused, "checker accepted an RFC1918 address.\n{report}");
}

/// The other side of the gate: a checker that flags correct work gets switched
/// off, and then it protects nothing. Its own comments say so, and on
/// 2026-08-13 a full-history sweep returned 22 row-taxonomy hits that were ALL
/// invented labels the allowlist had not been told about.
#[test]
fn the_checker_does_not_flag_invented_examples() {
    let (refused, report) = checker_refuses(concat!(
        "invented home paths: /home/user /home/user/proj /home/example\n",
        "a CI path: /home/runner/work/repo\n",
        "documentation ranges: 192.0.2.10 198.51.100.4 203.0.113.9\n",
        "invented lane labels: \"1.1 atlasstore: records\" \"5.1 lumenstore: vendor research\"\n",
        "and \"7.2 topicb: continue\" and \"3 widgets: refactor\"\n",
    ));
    assert!(
        !refused,
        "checker flagged the invented examples it exists to encourage — \
         add the label/placeholder to its allowlist in the same commit that \
         invents it.\n{report}"
    );
}
