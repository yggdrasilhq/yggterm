//! The deploy verb must refuse a build that is behind `origin/main`.
//!
//! ⛔ WHY THIS IS A TEST AND NOT A COMMENT. On 2026-08-13 the version numbers
//! `3.0.117`–`3.0.120` were each allocated TWICE within minutes, because nothing
//! arbitrates one: a cluster reads `Cargo.toml`, adds one, and pushes, and a
//! cluster that read the same file first takes the same number. The expensive
//! half is not the collision — it is that a cluster which built before rebasing
//! deploys a binary LACKING another cluster's commit. That deploy lands over the
//! first, the GUI re-execs onto it (pid unchanged, so `/proc/<pid>/exe` still
//! reads clean), and the first cluster's live probe comes back RED against a
//! binary that never carried its fix. Reading that as *"my root cause was
//! wrong"* is the most expensive wrong conclusion available, and it cost a full
//! build → deploy → restart → probe cycle.
//!
//! ⇒ `--version` cannot see this: both builds report the same string, which is
//! why the census never caught it. Ancestry can, and it catches it BEFORE the
//! bytes land rather than after a day of re-deriving a correct diagnosis.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn git(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git must run");
    assert!(
        out.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn write(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write test file");
}

/// A throwaway fleet: a bare upstream, the cluster's checkout, and a second
/// cluster that pushes a fix the first one has not pulled.
///
/// Hand-rolled rather than pulling in `tempfile` — no crate in this workspace
/// carries it as a dev-dependency, and one test is not the reason to start.
struct Fleet {
    root: PathBuf,
    mine: PathBuf,
}

impl Drop for Fleet {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stage_two_clusters(script: &Path) -> Fleet {
    let root = std::env::temp_dir().join(format!(
        "ygg-deploy-fleet-guard-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("mkdir fleet root");
    let upstream = root.join("upstream.git");
    let mine = root.join("mine");
    let theirs = root.join("theirs");

    std::fs::create_dir_all(&upstream).expect("mkdir upstream");
    git(&upstream, &["init", "--bare", "-b", "main", "."]);

    std::fs::create_dir_all(&mine).expect("mkdir mine");
    git(&mine, &["init", "-b", "main", "."]);
    write(&mine.join("base.txt"), "base\n");
    git(&mine, &["add", "-A"]);
    git(&mine, &["commit", "--no-gpg-sign", "-m", "base"]);
    git(&mine, &["remote", "add", "origin", &upstream.to_string_lossy()]);
    git(&mine, &["push", "-q", "origin", "main"]);

    // The other cluster ships a fix while this one is mid-build.
    git(
        &root,
        &[
            "clone",
            "-q",
            &upstream.to_string_lossy(),
            &theirs.to_string_lossy(),
        ],
    );
    write(&theirs.join("their-fix.txt"), "the other cluster's fix\n");
    git(&theirs, &["add", "-A"]);
    git(
        &theirs,
        &["commit", "--no-gpg-sign", "-m", "the other cluster's fix"],
    );
    git(&theirs, &["push", "-q", "origin", "main"]);

    // The script locates its own repo, so it has to live inside the checkout it
    // is judging — which is also how it behaves in the fleet.
    let scripts = mine.join("scripts");
    std::fs::create_dir_all(&scripts).expect("mkdir scripts");
    let dest = scripts.join("deploy-fleet.sh");
    std::fs::copy(script, &dest).expect("copy deploy-fleet.sh");
    let mut perms = std::fs::metadata(&dest).expect("stat script").permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    std::fs::set_permissions(&dest, perms).expect("chmod script");

    Fleet { root, mine }
}

fn run_deploy(mine: &Path) -> Output {
    Command::new("bash")
        .arg(mine.join("scripts/deploy-fleet.sh"))
        .args(["--dry-run", "--hosts", "local"])
        .current_dir(mine)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("deploy-fleet.sh must run")
}

#[test]
fn a_build_behind_origin_main_is_refused_and_the_missing_commits_are_named() {
    let script = repo_root().join("scripts/deploy-fleet.sh");
    if !script.exists() {
        return; // packaged crate, no scripts/ beside it
    }
    let fleet = stage_two_clusters(&script);
    let out = run_deploy(&fleet.mine);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a deploy from a tree behind origin/main must not proceed.\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("not a descendant of origin/main"),
        "the refusal must say what is wrong.\nstderr:\n{stderr}"
    );
    // ⛔ ASSERT ON THE PAYLOAD, NOT ON THE WRAPPER. A refusal that fires but
    // cannot name what the build is missing sends the operator to re-derive it,
    // and that re-derivation is the day this defect actually costs.
    assert!(
        stderr.contains("the other cluster's fix"),
        "the refusal must NAME the commits this build would revert.\nstderr:\n{stderr}"
    );
}

#[test]
fn the_same_tree_passes_the_ancestry_gate_once_it_is_rebased() {
    let script = repo_root().join("scripts/deploy-fleet.sh");
    if !script.exists() {
        return;
    }
    let fleet = stage_two_clusters(&script);
    git(&fleet.mine, &["pull", "-q", "--rebase", "origin", "main"]);
    let out = run_deploy(&fleet.mine);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // ⚠ It still exits non-zero — there are no build products in a temp repo —
    // but on the NEXT check, which is the proof that the gate is a gate and not
    // a blanket refusal that would have "passed" this test by always failing.
    assert!(
        !stderr.contains("not a descendant of origin/main"),
        "a rebased tree must clear the ancestry gate.\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("missing build product"),
        "the run must reach the build-product check.\nstderr:\n{stderr}"
    );
}
