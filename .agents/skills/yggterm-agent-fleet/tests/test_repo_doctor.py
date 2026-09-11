#!/usr/bin/env python3
"""Tests for repo-doctor.py (dream ACK-4fbea73342).

Builds a scratch bare origin + working clone and exercises the even,
behind, --ancestor and --grep paths plus the unknown-repo error path.
No fleet repo is touched.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DOCTOR = HERE.parent / "repo-doctor.py"
FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  - ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def sh(*args, cwd=None):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


def run_doctor(*args):
    return subprocess.run([sys.executable, str(DOCTOR), *args],
                          capture_output=True, text=True)


def main():
    tmp = Path(tempfile.mkdtemp(prefix="repodoctor-"))
    try:
        origin = tmp / "origin.git"
        sh("git", "init", "--bare", "-b", "main", str(origin))
        work = tmp / "work"
        sh("git", "clone", "-q", str(origin), str(work))
        (work / "a.txt").write_text("a\n")
        sh("git", "-C", str(work), "add", "-A")
        sh("git", "-C", str(work), "-c", "user.name=t", "-c", "user.email=t@t",
           "commit", "-qm", "seed commit needle-one")
        sh("git", "-C", str(work), "push", "-q", "origin", "main")
        base = sh("git", "-C", str(work), "rev-parse", "HEAD").stdout.strip()

        # even
        r = run_doctor(str(work))
        check("even repo exits 0", r.returncode == 0, r.stdout + r.stderr)
        check("even verdict printed", "EVEN" in r.stdout)

        # someone else pushes; local is now behind
        other = tmp / "other"
        sh("git", "clone", "-q", str(origin), str(other))
        (other / "b.txt").write_text("b\n")
        sh("git", "-C", str(other), "add", "-A")
        sh("git", "-C", str(other), "-c", "user.name=t", "-c", "user.email=t@t",
           "commit", "-qm", "second commit needle-two")
        sh("git", "-C", str(other), "push", "-q", "origin", "main")

        r = run_doctor(str(work))
        check("behind repo exits 1", r.returncode == 1, r.stdout + r.stderr)
        check("behind verdict printed", "BEHIND" in r.stdout and "1 BEHIND" in r.stdout)

        # --ancestor: base landed, unpushed tip not
        r = run_doctor(str(work), "--ancestor", base)
        check("ancestor landed exits 0", r.returncode == 0 and "LANDED" in r.stdout,
              r.stdout + r.stderr)
        (work / "c.txt").write_text("c\n")
        sh("git", "-C", str(work), "add", "-A")
        sh("git", "-C", str(work), "-c", "user.name=t", "-c", "user.email=t@t",
           "commit", "-qm", "unpushed tip")
        tip = sh("git", "-C", str(work), "rev-parse", "HEAD").stdout.strip()
        r = run_doctor(str(work), "--ancestor", tip)
        check("unpushed tip not landed exits 1", r.returncode == 1 and "NOT LANDED" in r.stdout,
              r.stdout + r.stderr)

        # --grep searches ORIGIN/main, not the stale local checkout
        r = run_doctor(str(work), "--grep", "needle-two")
        check("grep finds a commit only origin has",
              r.returncode == 0 and "FOUND" in r.stdout and "needle-two" in r.stdout,
              r.stdout + r.stderr)
        r = run_doctor(str(work), "--grep", "no-such-needle")
        check("grep miss exits 1", r.returncode == 1 and "NOT FOUND" in r.stdout)

        # unknown fleet name errors with exit 2
        r = run_doctor("definitely-not-a-repo-xyz")
        check("unknown repo exits 2", r.returncode == 2 and "not found" in r.stderr)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if FAILURES:
        print(f"\n{len(FAILURES)} checks failed: {', '.join(FAILURES)}")
        sys.exit(1)
    print("\nAll repo-doctor tests passed.")


if __name__ == "__main__":
    main()
