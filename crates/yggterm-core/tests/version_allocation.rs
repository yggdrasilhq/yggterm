//! A version number must be allocated from `origin/main` and claimed alone.
//!
//! ⛔ WHY THIS IS A TEST AND NOT A COMMENT. On 2026-08-13 the numbers
//! `3.0.117`–`3.0.120` were each allocated TWICE within minutes. Nothing
//! arbitrated one: a cluster read the version out of its **working** `Cargo.toml`,
//! added one, and carried that number for the length of a build, while a second
//! cluster that had read the same file before the first pushed took the same
//! number. Four consecutive numbers each meant two builds, so every "is my fix
//! live?" check written against `--version` was answering a different question
//! from the one asked.
//!
//! The local file is exactly as stale as the last time this checkout pulled,
//! which on a three-host fleet is a coin flip — so the number has one legitimate
//! source, and these tests hold the script to it.

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

fn manifest(version: &str) -> String {
    // A dependency pinned to the same string sits below the workspace version on
    // purpose: a bump that rewrites every `version = ` line in the file would
    // repin it, and the test would rather catch that here than in a build.
    format!(
        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\n\n\
         [workspace.dependencies]\nsomedep = {{ version = \"{version}\" }}\n"
    )
}

fn version_line(body: &str) -> String {
    body.lines()
        .find_map(|line| line.strip_prefix("version = "))
        .map(|rest| rest.trim_matches('"').to_string())
        .expect("a version line")
}

struct Fleet {
    root: PathBuf,
    mine: PathBuf,
    upstream: PathBuf,
}

impl Drop for Fleet {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A bare upstream sitting at `origin_version`, and a checkout of it.
fn stage(script: &Path, origin_version: &str) -> Fleet {
    let root = std::env::temp_dir().join(format!(
        "ygg-version-allocation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("mkdir root");
    let upstream = root.join("upstream.git");
    let mine = root.join("mine");

    std::fs::create_dir_all(&upstream).expect("mkdir upstream");
    git(&upstream, &["init", "--bare", "-b", "main", "."]);

    std::fs::create_dir_all(&mine).expect("mkdir mine");
    git(&mine, &["init", "-b", "main", "."]);
    write(&mine.join("Cargo.toml"), &manifest(origin_version));
    git(&mine, &["add", "-A"]);
    git(&mine, &["commit", "--no-gpg-sign", "-m", "base"]);
    git(&mine, &["remote", "add", "origin", &upstream.to_string_lossy()]);
    git(&mine, &["push", "-q", "origin", "main"]);

    let scripts = mine.join("scripts");
    std::fs::create_dir_all(&scripts).expect("mkdir scripts");
    let dest = scripts.join("bump-version.sh");
    std::fs::copy(script, &dest).expect("copy bump-version.sh");
    let mut perms = std::fs::metadata(&dest).expect("stat script").permissions();
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    std::fs::set_permissions(&dest, perms).expect("chmod script");

    Fleet {
        root,
        mine,
        upstream,
    }
}

fn run_bump(mine: &Path, args: &[&str]) -> Output {
    Command::new("bash")
        .arg(mine.join("scripts/bump-version.sh"))
        .args(args)
        .current_dir(mine)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("bump-version.sh must run")
}

fn script() -> Option<PathBuf> {
    let path = repo_root().join("scripts/bump-version.sh");
    path.exists().then_some(path)
}

/// ⛔ THE DEFECT ITSELF. The working file says `3.0.9` — a number this cluster
/// picked for itself and never pushed — while `origin/main` says `3.0.5`. Any
/// script that reads the file it is standing in allocates `3.0.10` and collides
/// with whoever really holds the line.
#[test]
fn the_number_is_taken_from_origin_and_not_from_the_local_file() {
    let Some(script) = script() else { return };
    let fleet = stage(&script, "3.0.5");
    write(&fleet.mine.join("Cargo.toml"), &manifest("3.0.9"));

    let out = run_bump(&fleet.mine, &["--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "dry run must succeed.\n{stderr}");
    assert_eq!(
        stdout.trim(),
        "3.0.6",
        "the number must come from origin/main (3.0.5), not from the local \
         file (3.0.9, which would give 3.0.10).\nstderr:\n{stderr}"
    );
}

/// The gate is a gate, not a blanket refusal: with nothing unpushed, the same
/// checkout allocates and the number lands on `origin/main`.
///
/// ⛔ ASSERTS ON THE PAYLOAD. "A commit was pushed" is the wrapper; what matters
/// is that the pushed tree carries the new number AND that it did not sweep up
/// the untracked work sitting beside it. This is a shared checkout — another
/// session's in-flight file is normally in the tree, and a `commit -a` would
/// publish it under a release message.
#[test]
fn the_bump_lands_on_origin_carrying_only_the_version() {
    let Some(script) = script() else { return };
    let fleet = stage(&script, "3.0.5");
    write(
        &fleet.mine.join("another-session-work.txt"),
        "half-finished work belonging to someone else\n",
    );

    let out = run_bump(&fleet.mine, &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the bump must succeed.\n{stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3.0.6");

    let pushed = String::from_utf8_lossy(
        &git(&fleet.upstream, &["show", "main:Cargo.toml"]).stdout,
    )
    .to_string();
    assert_eq!(
        version_line(&pushed),
        "3.0.6",
        "origin/main must carry the allocated number"
    );
    assert!(
        pushed.contains("somedep = { version = \"3.0.5\" }"),
        "only the workspace version may move; a pinned dependency must not be \
         repinned by the bump.\npushed manifest:\n{pushed}"
    );

    let files = String::from_utf8_lossy(
        &git(
            &fleet.upstream,
            &["show", "--name-only", "--format=", "main"],
        )
        .stdout,
    )
    .to_string();
    assert_eq!(
        files.trim(),
        "Cargo.toml",
        "the bump must go up ALONE — it swept another session's work into a \
         release commit.\nfiles:\n{files}"
    );
}

/// A checkout holding unpushed commits is refused by name, because pushing the
/// bump would drag them along under a `chore(release)` message.
#[test]
fn a_checkout_with_unpushed_work_is_refused_and_the_work_is_named() {
    let Some(script) = script() else { return };
    let fleet = stage(&script, "3.0.5");
    write(&fleet.mine.join("mine.txt"), "this cluster's own fix\n");
    git(&fleet.mine, &["add", "-A"]);
    git(
        &fleet.mine,
        &["commit", "--no-gpg-sign", "-m", "this cluster's own fix"],
    );

    let out = run_bump(&fleet.mine, &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "an ahead checkout must be refused");
    assert!(
        stderr.contains("this cluster's own fix"),
        "the refusal must NAME what is unpushed, or the operator re-derives \
         it.\nstderr:\n{stderr}"
    );
}
