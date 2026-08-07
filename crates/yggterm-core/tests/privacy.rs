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
