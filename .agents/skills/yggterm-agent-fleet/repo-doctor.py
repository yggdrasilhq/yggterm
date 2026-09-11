#!/usr/bin/env python3
"""repo-doctor - verify a fleet repo against ORIGIN/main before any cross-repo claim.

Dream ACK-4fbea73342 (2026-09-07): a stale local checkout nearly produced a
false defect ("yggui-icons exists?" grep-found nothing because the local
main sat behind origin). Cross-repo claims must be verified against
origin/main, never the local checkout - this verb is that check, one line.

Usage:
  repo-doctor <repo>                      fetch + report local-vs-origin/main
  repo-doctor <repo> --ancestor <sha>     is <sha> on origin/<branch>?
  repo-doctor <repo> --grep PATTERN       search ORIGIN/<branch> commit log
  repo-doctor <repo> --branch main        compare against another upstream branch

<repo> is a path, or a fleet name resolved via ~/git/<name> then ~/gh/<name>.

Exit codes: 0 = even (or subject landed / pattern found),
1 = behind/diverged (or subject not landed / pattern not found), 2 = error.
"""
import argparse
import subprocess
import sys
from pathlib import Path

FLEET_REPOS = {
    "yggterm": ["~/gh/yggterm"],
    "yggdrasil": ["~/gh/yggdrasil"],
    "ydesign": ["~/git/ydesign", "~/gh/ydesign"],
    "jyas": ["~/git/jyas"],
    "practice-rs": ["~/git/practice-rs"],
    "dossiergraph-manager": ["~/data/dossiergraph/git/dossiergraph-manager"],
}

REPO = None


def die(msg):
    print(f"repo-doctor: {msg}", file=sys.stderr)
    sys.exit(2)


def resolve_repo(name):
    p = Path(name).expanduser()
    if p.exists():
        return p.resolve()
    for base in ("~/git", "~/gh"):
        c = Path(base, name).expanduser()
        if c.exists():
            return c.resolve()
    die(f"repo not found: {name} (tried the path, ~/git/<name>, ~/gh/<name>)")


def git(*args):
    r = subprocess.run(["git", "-C", str(REPO), *args], capture_output=True, text=True)
    if r.returncode != 0:
        die(f"git {' '.join(args)}: {r.stderr.strip()[:200]}")
    return r.stdout.strip()


def sha_of(ref):
    r = subprocess.run(["git", "-C", str(REPO), "rev-parse", "--short", ref],
                       capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def main():
    global REPO
    ap = argparse.ArgumentParser(prog="repo-doctor",
                                 description=__doc__.splitlines()[0])
    ap.add_argument("repo", help="repo path or fleet name")
    ap.add_argument("--ancestor", metavar="SHA", default=None,
                    help="exit 0 if SHA is an ancestor of origin/<branch>")
    ap.add_argument("--grep", metavar="PATTERN", default=None,
                    help="search ORIGIN/<branch> commit log (not the local checkout)")
    ap.add_argument("--branch", default="main", help="upstream branch (default main)")
    ap.add_argument("--no-fetch", action="store_true",
                    help="skip `git fetch origin` (you just fetched)")
    a = ap.parse_args()

    REPO = resolve_repo(a.repo)
    if not (REPO / ".git").exists():
        die(f"{REPO} is not a git repository")

    origin_branch = f"origin/{a.branch}"
    if not a.no_fetch:
        git("fetch", "origin", "--quiet")
    if sha_of(origin_branch) is None:
        die(f"{REPO} has no {origin_branch}")

    if a.ancestor:
        sha = sha_of(a.ancestor) or a.ancestor
        r = subprocess.run(["git", "-C", str(REPO), "merge-base", "--is-ancestor",
                            sha, origin_branch])
        when = git("log", "-1", "--format=%cs", origin_branch)
        if r.returncode == 0:
            print(f"LANDED: {sha} is on {origin_branch} of {REPO.name} (origin tip {when})")
            sys.exit(0)
        print(f"NOT LANDED: {sha} is NOT on {origin_branch} of {REPO.name} "
              f"(origin tip {when})")
        sys.exit(1)

    if a.grep is not None:
        r = subprocess.run(["git", "-C", str(REPO), "log", origin_branch,
                            "--oneline", "--grep", a.grep, "-1"],
                           capture_output=True, text=True)
        if r.stdout.strip():
            print(f"FOUND on {origin_branch}: {r.stdout.strip()}")
            sys.exit(0)
        print(f"NOT FOUND on {origin_branch}: no commit matches '{a.grep}'")
        sys.exit(1)

    head = sha_of("HEAD")
    head_branch = git("rev-parse", "--abbrev-ref", "HEAD")
    counts = git("rev-list", "--left-right", "--count",
                 f"HEAD...{origin_branch}").split()
    behind, ahead = int(counts[1]), int(counts[0])
    head_desc = git("log", "-1", "--format=%h %cs", "HEAD")
    origin_desc = git("log", "-1", "--format=%h %cs", origin_branch)

    if behind == 0 and ahead == 0:
        verdict, code = "EVEN with origin", 0
    elif ahead > 0 and behind == 0:
        verdict, code = f"{ahead} commit(s) AHEAD of origin (unpushed)", 1
    elif behind > 0 and ahead == 0:
        verdict, code = f"local {head_branch} is {behind} BEHIND {origin_branch}", 1
    else:
        verdict = (f"DIVERGED: local {head_branch} is {ahead} ahead and "
                   f"{behind} behind {origin_branch}")
        code = 1
    print(f"{REPO.name}: {verdict}; local HEAD {head_desc}; {origin_branch} {origin_desc}")
    sys.exit(code)


if __name__ == "__main__":
    main()
