#!/usr/bin/env python3
"""ygg-land — the orchestrator merges a lane's branch into main, safely and serially.

⛔⛔ WHY THIS EXISTS. A dozen lanes work in a dozen worktrees, and until now each
one pushed to `main` itself. Two consequences, both reported by the owner:

  · **DIVERGENCE.** A lane that lands and a lane that does not are invisible to
    each other, and the roll builds from `origin/main` — so work that never
    merged never ships, while its author believes it did. Nothing anywhere says
    which branches are ahead.
  · **TWO AGENTS PUSHING BUILDS.** Landing and releasing were the same act for
    whoever happened to be holding the branch, so two lanes could allocate two
    versions and write the same three binaries on the same three hosts. The
    census then names a commit that is a mixture no tree ever held.

⇒ **The split this verb enforces: a LANE pushes its own branch and says it is
  ready. The ORCHESTRATOR lands it and is the only thing that rolls.** One merge
  point, one release point, and the state of every lane is answerable.

USAGE
    ygg-land.py status                     # every lane branch, ahead/behind main
    ygg-land.py land <branch> [--apply]    # merge one branch into main
    ygg-land.py land --all [--apply]       # every branch that is ready

⛔ DRY BY DEFAULT. Landing rewrites the branch every other lane builds on.
"""
import argparse
import os
import subprocess
import sys
import time

#: Default: the yggterm checkout this script lives in. ⚠ NOT the only repo an
#: orchestrator lands — ytop carries lane branches from two seats whose work must
#: merge harmoniously, and landing them by hand loses every guard below.
REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))


def log(msg):
    print(f"{time.strftime('%H:%M:%S')} ygg-land {msg}")


def git(*args, cwd=None, check=False):
    # ⛔ `cwd=REPO` as a DEFAULT ARGUMENT binds at import and would ignore --repo
    # entirely, while every call site still looked correct.
    cwd = cwd or REPO
    r = subprocess.run(["git", "-C", cwd, *args], capture_output=True, text=True, timeout=300)
    if check and r.returncode != 0:
        raise RuntimeError((r.stderr or r.stdout).strip()[:300])
    return r


def lane_branches():
    git("fetch", "-q", "origin")
    out = git("for-each-ref", "--format=%(refname:short)", "refs/heads/").stdout
    return [b.strip() for b in out.splitlines() if b.strip().startswith("lane/")]


def ahead_behind(branch):
    """⛔⛔ A COMMIT COUNT ANSWERS "HOW MANY SHAs", NOT "WHAT IS UNLANDED".

    `rev-list --count` compares refs. A lane that rebased, or whose patches were
    landed by cherry-pick, leaves a branch whose commits are all *already in main
    under different SHAs* — and the count reports it as carrying unlanded work
    forever. Measured 2026-08-21: `lane/dev/11.9-cli-practice` read ahead=3 with
    all three commits equivalent to main's. It had nothing to land.

    ⚠ And believing the count is not merely cosmetic: merging such a branch takes
    its OLD version of every file it touched, so the "land" silently reverts
    whatever main did to those files since — the pre-rebase-branch-is-a-revert
    trap. `git cherry` is the instrument that knows the difference, because it
    compares PATCHES.
    """
    r = git("rev-list", "--left-right", "--count", f"origin/main...{branch}")
    try:
        behind, refs_ahead = (int(x) for x in r.stdout.split())
    except Exception:
        return None, None
    c = git("cherry", "origin/main", branch)
    if c.returncode != 0:
        return refs_ahead, behind
    unlanded = [l for l in c.stdout.splitlines() if l.startswith("+")]
    return len(unlanded), behind


def refs_ahead(branch):
    """The raw SHA count, kept only so status can SHOW the gap rather than hide it."""
    r = git("rev-list", "--count", f"origin/main..{branch}")
    try:
        return int(r.stdout.strip())
    except Exception:
        return 0


def stale_base(branch):
    """Days since the branch left main, and how much of its footprint main has since
    rewritten. Both together say whether a merge is a landing or a partial revert."""
    mb = git("merge-base", "origin/main", branch).stdout.strip()
    if not mb:
        return None, 0, 0
    age = git("log", "-1", "--format=%ct", mb).stdout.strip()
    try:
        days = (time.time() - int(age)) / 86400
    except Exception:
        days = 0
    mine = set(git("diff", "--name-only", f"{mb}..{branch}").stdout.split())
    theirs = set(git("diff", "--name-only", f"{mb}..origin/main").stdout.split())
    return days, len(mine), len(mine & theirs)


def cmd_status(_a):
    rows = []
    for b in lane_branches():
        ahead, behind = ahead_behind(b)
        if ahead is None:
            log(f"⚠ {b}: cannot compare")
            continue
        rows.append((b, ahead, behind))
    if not rows:
        log("no lane branches")
        return 0
    phantom = 0
    for b, ahead, behind in sorted(rows, key=lambda r: -r[1]):
        state = "READY  " if ahead else "landed "
        note = ""
        raw = refs_ahead(b)
        if raw > ahead:
            note = f"  ({raw - ahead} commit(s) already in main under other SHAs)"
            phantom += 1
        log(f"{state} {b:<40} ahead={ahead:<4} behind={behind}{note}")
    log(f"— {sum(1 for r in rows if r[1])} branch(es) carrying unlanded work")
    if phantom:
        log(f"  {phantom} branch(es) look ahead by SHA and are not: their patches are in main.")
    return 0


#: Paths whose change cannot alter a build product. Used ONLY to decide whether a
#: re-merge needs another `cargo check`, never to skip a guard.
INERT = ("docs/", "CHANGELOG.md", "README.md", ".agents/", "scripts/")

#: A branch whose base is older than this, and whose files main has since changed,
#: is a revert wearing a merge. Landing it needs an operator to say so out loud.
STALE_BASE_DAYS = 5
STALE_OK = False


def touches_build(from_ref, to_ref):
    out = git("diff", "--name-only", from_ref, to_ref).stdout.splitlines()
    return any(not any(f.startswith(i) for i in INERT) and not f.endswith(".md") for f in out if f.strip())


def land_one(branch, apply_it, attempts=5):
    """⛔ A PUSH REJECTION IS THE NORMAL OUTCOME HERE, NOT AN ERROR.

    Main advances constantly on this fleet — the orchestrator itself pushes to it
    while landing — so a merge that takes three minutes of `cargo check` routinely
    finds main moved underneath by the time it pushes. Giving up there means a
    branch can never land on a busy day, which is exactly the divergence this verb
    exists to remove. So it re-merges onto the new main and tries again, and it
    re-runs the expensive check only when the newly arrived commits actually touch
    the build.
    """
    # ⛔⛔ THE RETRY MUST BE CHEAP OR IT CANNOT WIN. `cargo check` takes about three
    # minutes; on a fleet where lanes land every couple of minutes, a three-minute
    # critical section loses the push every single time — measured, three attempts
    # in a row, each one re-running a check whose answer had not changed.
    # ⇒ The check is re-run only when the commits that ARRIVED since the last one
    #   actually touch the build. The first cut asked whether the BRANCH differed
    #   from main, which is true by construction on every attempt, so the skip
    #   never fired and every retry paid full price.
    needs_check = True
    for attempt in range(1, attempts + 1):
        result = _land_once(branch, apply_it, attempt, attempts, needs_check)
        if isinstance(result, tuple):
            _, needs_check = result
            continue
        if result != "retry":
            return result
    log(f"  ⛔ {branch}: main moved under {attempts} attempts — a human should land it")
    return False


def _land_once(branch, apply_it, attempt, attempts, needs_check=True):
    ahead, behind = ahead_behind(branch)
    if ahead is None:
        log(f"⚠ {branch}: cannot compare — skipped")
        return False
    if ahead == 0:
        raw = refs_ahead(branch)
        if raw:
            log(f"· {branch}: nothing to land — its {raw} commit(s) are already in main "
                f"under other SHAs. ⛔ Merging it would restore its older copies of every "
                f"file it touched. Reset the ref instead: git branch -f {branch} origin/main")
        else:
            log(f"· {branch}: already landed")
        return False
    days, foot, overlap = stale_base(branch)
    if days and days > STALE_BASE_DAYS and overlap:
        log(f"⛔ {branch}: left main {days:.0f} days ago and main has since rewritten "
            f"{overlap} of the {foot} file(s) it touches. A merge here restores its older "
            f"copies. Cherry-pick the {ahead} commit(s), or rebase the lane, or pass --stale-ok.")
        if not STALE_OK:
            return False
        log(f"  --stale-ok: proceeding over the {days:.0f}-day-old base on an operator's say-so")
    log(f"landing {branch} ({ahead} commit(s), {behind} behind) attempt {attempt}/{attempts}")
    if not apply_it:
        log("  (dry run: nothing merged)")
        return False

    # ⛔ MERGE IN A SCRATCH WORKTREE, NEVER IN A LANE'S OWN CHECKOUT. A lane may be
    # mid-edit in its tree; checking main out under it destroys uncommitted work
    # and is invisible until that agent's next write fails.
    wt = f"/tmp/ygg-land-{os.getpid()}"
    git("worktree", "add", "-q", "--detach", wt, "origin/main")
    try:
        r = subprocess.run(["git", "-C", wt, "merge", "--no-ff", branch,
                            "-m", f"land: {branch}"], capture_output=True, text=True, timeout=600)
        if r.returncode != 0:
            log(f"  ⛔ merge conflict — NOT landed. The lane must rebase on main first:")
            for line in (r.stdout or "").strip().splitlines()[:6]:
                log(f"     {line}")
            return False

        # ⛔ THE GUARDS RUN ON THE MERGE RESULT, NOT ON THE BRANCH. A branch that
        # was clean alone can be dirty once merged — that is the entire reason a
        # merge is a separate act from a push.
        for name, cmd in (("privacy", ["scripts/check-privacy.sh"]),
                          ("docs-ssot", ["bash", "scripts/check-docs-ssot.sh"])):
            if not os.path.exists(os.path.join(wt, cmd[-1])):
                continue
            g = subprocess.run(cmd, cwd=wt, capture_output=True, text=True, timeout=600)
            if g.returncode != 0:
                log(f"  ⛔ {name} guard refused the MERGE RESULT — not landed")
                for line in ((g.stdout or "") + (g.stderr or "")).strip().splitlines()[-4:]:
                    log(f"     {line}")
                return False
            log(f"  {name}: ok")

        # ⛔ AND IT MUST COMPILE. Landing something that does not build blocks
        # every other lane, because they all branch from main.
        # ⚠ On a retry whose only new commits are inert, the check already passed on
        # the same code. Re-running it costs three minutes and buys a race.
        skip_check = not needs_check
        b = (subprocess.CompletedProcess([], 0) if skip_check else
             subprocess.run(["cargo", "check", "--workspace", "--quiet"],
                            cwd=wt, capture_output=True, text=True, timeout=1800))
        if skip_check:
            log("  cargo check: skipped — nothing new since the last one touches the build")
        if b.returncode != 0:
            log("  ⛔ the merge result does not compile — not landed")
            for line in ((b.stderr or "") if isinstance(b.stderr, str) else "").strip().splitlines()[-6:]:
                log(f"     {line}")
            return False
        elif b.returncode == 0:
            log("  cargo check: ok")

        # ⚠ A rejected push means main moved under us. That is ordinary on this
        # fleet, and the answer is to redo the merge on the new main rather than
        # to force anything.
        p = subprocess.run(["git", "-C", wt, "push", "origin", "HEAD:main"],
                           capture_output=True, text=True, timeout=600)
        if p.returncode != 0:
            before = git("rev-parse", "origin/main").stdout.strip()
            git("fetch", "-q", "origin")
            after = git("rev-parse", "origin/main").stdout.strip()
            moved = touches_build(before, after) if before != after else False
            log(f"  ⚠ push rejected — main moved" + (" and it touches the build" if moved
                else " (nothing new touches the build — the next attempt skips the check)")
                + ". Re-merging.")
            return ("retry", moved)
        log(f"  ✅ landed {branch}")
        return True
    finally:
        git("worktree", "remove", "--force", wt)


def prune_scratch():
    """⚠ THE `finally` THAT REMOVES THE SCRATCH WORKTREE DOES NOT RUN ON A KILL.

    A land that is interrupted — and on this fleet they are, main moves and
    operators change their minds — leaves /tmp/ygg-land-<pid> registered forever.
    Found 2026-08-21: one from a land killed hours earlier, still listed.
    """
    out = git("worktree", "list", "--porcelain").stdout
    for line in out.splitlines():
        if not line.startswith("worktree /tmp/ygg-land-"):
            continue
        path = line.split(" ", 1)[1]
        try:
            pid = int(path.rsplit("-", 1)[1])
        except Exception:
            continue
        if os.path.exists(f"/proc/{pid}"):
            continue
        git("worktree", "remove", "--force", path)
        log(f"  pruned scratch worktree of dead land {pid}")


def cmd_land(a):
    prune_scratch()
    targets = lane_branches() if a.all else [a.branch]
    if not a.all and not a.branch:
        log("⛔ name a branch, or pass --all")
        return 2
    landed = 0
    for b in targets:
        if land_one(b, a.apply):
            landed += 1
    log(f"— {'landed' if a.apply else 'would land'} {landed}")
    if not a.apply:
        log("  nothing was changed. Re-run with --apply.")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status")
    ld = sub.add_parser("land")
    ld.add_argument("branch", nargs="?")
    ld.add_argument("--all", action="store_true")
    ld.add_argument("--apply", action="store_true")
    ld.add_argument("--stale-ok", action="store_true",
                    help="land over a base older than %d days even though main has since "
                         "rewritten the files this branch touches" % STALE_BASE_DAYS)
    ap.add_argument("--repo", help="checkout to land in (default: this script's yggterm repo)")
    a = ap.parse_args()
    global STALE_OK, REPO
    STALE_OK = getattr(a, "stale_ok", False)
    if getattr(a, "repo", None):
        REPO = os.path.abspath(os.path.expanduser(a.repo))
    return cmd_status(a) if a.cmd == "status" else cmd_land(a)


if __name__ == "__main__":
    sys.exit(main())
