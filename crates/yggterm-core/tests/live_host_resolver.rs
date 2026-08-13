//! "Which host is the live GUI on?" has exactly one owner.
//!
//! ⛔ WHY THIS IS A TEST. Fifteen recipes — fourteen across the agent skills and
//! one python default — read `.agents/config/live-host` directly. That path is
//! gitignored — correctly, the alias is infrastructure and this repo is public —
//! so the file exists **only on the machine whose name it holds**, which is the
//! one machine that never needs telling. Sessions run headless by standing
//! directive, so in practice every recipe died on its first line with
//! `No such file or directory`, and the one checkout that could read it was the
//! one that did not have to.
//!
//! `scripts/ygg-live-host.sh` is now the single owner. The config file is its
//! **cache**, not a second source of truth, and the difference is invisible at
//! the callsite — which is exactly why a rule in prose does not hold and this
//! does. A copied recipe is how the second source came back the first time.

#![cfg(unix)]

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

/// Every agent-facing recipe file: the skills, and the scripts they drive.
fn recipe_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.join(".agents/skills"), root.join("scripts")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            // The resolver's own header quotes the retired form to explain what
            // it replaced; it is the one file allowed to say it.
            if path.file_name().is_some_and(|n| n == "ygg-live-host.sh") {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "md" | "sh" | "py") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn no_recipe_reads_the_live_host_cache_instead_of_asking_the_resolver() {
    let root = repo_root();
    if !root.join("scripts/ygg-live-host.sh").exists() {
        return; // packaged crate, no scripts/ beside it
    }

    let mut offenders = Vec::new();
    for path in recipe_files(&root) {
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in body.lines().enumerate() {
            // The recipe form, not the prose: the ⛔ warnings that tell an agent
            // NOT to do this name the path without assigning from it.
            if line.contains("LIVE_HOST=$(cat") || line.contains("LIVE_HOST=`cat") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these recipes read the gitignored cache directly instead of calling \
         scripts/ygg-live-host.sh, so they resolve on the live host alone and \
         fail on every headless checkout:\n{}",
        offenders.join("\n")
    );
}

/// ⛔ ASSERTS ON THE PAYLOAD, not on the absence of a string. A rewire that
/// deleted the offending lines without putting the resolver in their place
/// would pass the test above and leave `$LIVE_HOST` empty — and an empty
/// `$LIVE_HOST` makes `ssh "$LIVE_HOST" cmd` run cmd **locally**, which is a
/// worse failure than the one being fixed, because it succeeds.
#[test]
fn the_recipes_assign_live_host_from_the_resolver_and_stop_on_failure() {
    let root = repo_root();
    let skill = root.join(".agents/skills/yggui-app-control/SKILL.md");
    if !skill.exists() {
        return;
    }
    let body = std::fs::read_to_string(&skill).expect("read the yggui skill");

    let assignments: Vec<&str> = body
        .lines()
        .filter(|line| line.trim_start().starts_with("LIVE_HOST="))
        .collect();
    assert!(
        !assignments.is_empty(),
        "the yggui skill must still show how to resolve the live host"
    );
    for line in &assignments {
        assert!(
            line.contains("scripts/ygg-live-host.sh"),
            "every LIVE_HOST assignment must come from the resolver: {line}"
        );
        assert!(
            line.contains("|| exit 1"),
            "an unguarded assignment leaves $LIVE_HOST empty on failure, and \
             `ssh \"$LIVE_HOST\" cmd` then runs cmd locally: {line}"
        );
    }
}

/// ⛔ AND A LITERAL HOST NAME IN CODE IS THE SAME BUG WEARING A DIFFERENT
/// COSTUME. The cache-read failed loudly on line one; a hardcoded name fails
/// *quietly*, because the tool carrying it still works on the hosts it can
/// reach. One unresolvable literal blinded four separate tools in a single day
/// — a fleet deploy that skipped the only host a UI change can be proven on,
/// three fleet supervisors that then rendered the missing host's rows as dead,
/// and a daemon audit whose default invocation reported confidently on two
/// hosts while omitting the one with the most daemons on it.
///
/// ⚠ AND IT IS SCOPED TO A HOST POSITION, because the first version of this
/// check was not and immediately flagged two search-fixture lines that resolve
/// nothing. A lock that fires on prose or on data teaches people to delete the
/// lock. What is forbidden is the name reaching ssh or a host list.
#[cfg(unix)]
fn names_a_host_literally(line: &str) -> bool {
    if !line.contains("guihost") {
        return false;
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false; // a comment explaining the trap is not the trap
    }
    // A host POSITION: something that will be ssh-ed to, or a list of things
    // that will be. Anything else carrying the token is data, not a target.
    line.contains("ssh")
        || line.contains("hosts")
        || line.contains("HOSTS")
        || line.contains("--host")
}

#[test]
fn the_hardcoded_host_check_can_say_both_yes_and_no() {
    // ⛔ BOTH CONTROLS. A predicate that has collapsed to `false` passes the
    // sweep below on an offending tree and reads exactly like a clean repo.
    assert!(names_a_host_literally(
        r#"    parser.add_argument("hosts", nargs="*", default=["local", "oc", "guihost"])"#
    ));
    assert!(names_a_host_literally(r#"HOSTS="dev guihost oc""#));
    assert!(!names_a_host_literally(
        r#"        if lowered in {"guihost", "oc", "local"}:"#
    ));
    assert!(!names_a_host_literally(
        "# the live GUI on guihost is Wayland-native"
    ));
    assert!(!names_a_host_literally("HOSTS=\"dev oc\""));
}

#[test]
fn no_script_hardcodes_a_gui_host_name_where_a_program_will_resolve_it() {
    let root = repo_root();
    if !root.join("scripts/ygg-live-host.sh").exists() {
        return;
    }

    let mut offenders = Vec::new();
    for path in recipe_files(&root) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "sh" | "py") {
            continue; // prose may name it; only executable text may not
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in body.lines().enumerate() {
            if names_a_host_literally(line) {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these lines name a GUI host literally in a position a program will try \
         to resolve. The name does not resolve off the machine it describes, so \
         the tool keeps working on the other hosts and silently drops this one. \
         Call scripts/ygg-live-host.sh (shell) or scripts/ygg_live_host.py \
         (python) instead:\n{}",
        offenders.join("\n")
    );
}
