#!/usr/bin/env python3
"""Tests for ygg-disperse.py (dream ACK-a0bd107330, spec-ygg-verb-dispersal.md).

Runs the verb against a sandbox replica tree + PATH dir (env overrides), with
the real skill dir as the SSOT. No fleet host is touched.
"""
import json
import os
import subprocess
import sys
import shutil
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DISPERSE = HERE.parent / "ygg-disperse.py"
SSOT = HERE.parent
SPEC = json.loads((SSOT / "ygg-verbs.json").read_text())
ALL_FILES = SPEC["tier_base"] + SPEC.get("owner_managed", []) + SPEC["tier_gui"]
FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  - ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def run(*args, env_extra=None):
    env = dict(os.environ)
    env.update(env_extra or {})
    return subprocess.run([sys.executable, str(DISPERSE), *args],
                          capture_output=True, text=True, env=env)


def sandbox():
    root = Path(tempfile.mkdtemp(prefix="ygg-disperse-test-"))
    env = {
        "YGG_DISPERSE_YGGTERM": str(root / "yggterm"),
        "YGG_DISPERSE_LOCALBIN": str(root / "localbin"),
    }
    return root, env


def main():
    root, env = sandbox()
    bin_dir = root / "yggterm" / "bin" / "ygg"
    manifest = root / "yggterm" / "config" / "ygg-verbs" / "manifest.json"

    # 1. install into the sandbox: replicas + manifest + base-tier links
    r = run("install", "--json", env_extra=env)
    check("install exits 0", r.returncode == 0, r.stderr[:300])
    out = json.loads(r.stdout)
    check("install reports every file", out.get("installed") == len(ALL_FILES),
          f"{out.get('installed')} != {len(ALL_FILES)}")
    check("replicas exist on disk", all((bin_dir / f).is_file() for f in ALL_FILES))
    check("manifest written", manifest.is_file())
    man = json.loads(manifest.read_text())
    check("manifest records a repo commit", man.get("source_commit") not in (None, ""),
          str(man.get("source_commit")))
    check("manifest hashes every file", set(man["files"]) == set(ALL_FILES))
    links_ok = all((root / "localbin" / n).is_symlink() for n in SPEC["tier_base"])
    check("base tier PATH-linked", links_ok)
    resolves = all((root / "localbin" / n).resolve() == bin_dir / n
                   for n in SPEC["tier_base"])
    check("links resolve into the replica tree", resolves)

    # 2. verify is green on a fresh install
    r = run("verify", "--json", env_extra=env)
    check("verify green after install", r.returncode == 0, r.stdout[:300])

    # 3. adoption: a hand-staged PATH copy is replaced by a link
    hand = root / "localbin" / "ygg-auth.py"
    hand.unlink()  # a real file, not a write through the step-1 symlink
    hand.write_text("#!/bin/sh\necho hand-staged junk\n")
    hand.chmod(0o755)
    r = run("install", "--json", env_extra=env)
    out = json.loads(r.stdout)
    check("re-install exits 0", r.returncode == 0, r.stderr[:300])
    check("hand copy adopted as link", hand.is_symlink()
          and hand.resolve() == bin_dir / "ygg-auth.py")
    check("adoption logged", any("ygg-auth.py" in a for a in out.get("adopted", [])),
          str(out.get("adopted")))

    # 4. drift: mutate a replica -> verify goes red and names the file
    (bin_dir / "repo-doctor.py").write_text(
        (bin_dir / "repo-doctor.py").read_text() + "\n# hand edit\n")
    r = run("verify", "--json", env_extra=env)
    check("verify red on replica drift", r.returncode == 1, r.stdout[:300])
    check("drift names the file", "drifted: repo-doctor.py" in r.stdout)

    # 5. link-missing is drift too
    (root / "localbin" / "ygg-board.py").unlink()
    r = run("verify", "--json", env_extra=env)
    check("verify red on missing link", r.returncode == 1)
    check("drift names the link", "link-missing" in r.stdout)

    # 6. re-install heals both, and status agrees
    run("install", env_extra=env)
    r = run("verify", env_extra=env)
    check("re-install heals drift", r.returncode == 0, r.stdout[:300])
    r = run("status", "--json", env_extra=env)
    st = json.loads(r.stdout)
    check("status green", r.returncode == 0 and st.get("ok") is True)

    # 7. prune: a replica from an older shipment list is removed on re-install
    stale = bin_dir / "ygg-verb-that-retired.py"
    stale.write_text("# retired\n")
    man["files"]["ygg-verb-that-retired.py"] = "x"
    manifest.write_text(json.dumps(man))
    r = run("install", "--json", env_extra=env)
    out = json.loads(r.stdout)
    check("stale replica pruned", not stale.exists()
          and "ygg-verb-that-retired.py" in out.get("pruned", []))
    r = run("verify", env_extra=env)
    check("verify green after prune", r.returncode == 0, r.stdout[:300])

    # 8. bin/ root hand-staged copies are adopted: links in, bytes preserved
    bin_root = root / "yggterm" / "bin"
    dup = bin_root / "ygg-procfind.sh"
    shutil.copy2(SSOT / "ygg-procfind.sh", dup)          # identical duplicate
    odd = bin_root / "ygg-board.py"
    odd.write_text("#!/bin/sh\necho stale hand edit\n")  # differing bytes
    r = run("install", "--json", env_extra=env)
    out = json.loads(r.stdout)
    check("root duplicate retired to link", dup.is_symlink()
          and dup.resolve() == bin_dir / "ygg-procfind.sh")
    check("root differing copy preserved", odd.is_symlink()
          and any(o.endswith("ygg-board.py") for o in out.get("preserved", []))
          and Path(out["preserved"][0]).is_file()
          and "stale hand edit" in Path(out["preserved"][0]).read_text())
    r = run("verify", env_extra=env)
    check("verify green after root adoption", r.returncode == 0, r.stdout[:300])

    # 9. owner-managed files (ygg-memory self-disperses its runners and
    #    unlinks symlinks by design; bootstrap.sh installs ygg-memory-sync):
    #    shipped into bin/ygg but never linked or adopted by the dispersal
    for name in SPEC.get("owner_managed", []):
        (bin_root / name).write_text(f"#!/bin/sh\necho owner-managed {name}\n")
    r = run("install", "--json", env_extra=env)
    out = json.loads(r.stdout)
    untouched = all((bin_root / n).is_file() and not (bin_root / n).is_symlink()
                    for n in SPEC.get("owner_managed", []))
    check("owner-managed root copies untouched", untouched)
    not_linked = all(not (root / "localbin" / n).is_symlink()
                     for n in SPEC.get("owner_managed", []))
    check("owner-managed not PATH-linked", not_linked)
    check("owner-managed not adopted",
          not any(n in " ".join(out.get("adopted", []))
                  for n in SPEC.get("owner_managed", [])))
    r = run("verify", env_extra=env)
    check("verify green with owner-managed present", r.returncode == 0, r.stdout[:300])

    print()
    if FAILURES:
        print(f"FAILED: {len(FAILURES)}: {', '.join(FAILURES)}")
        sys.exit(1)
    print("all ygg-disperse tests passed")


if __name__ == "__main__":
    main()
