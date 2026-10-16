#!/usr/bin/env python3
"""[11.230] closure lock: the branch-deleted sweep consumes EVERY shape of a
sub whose lane branch is gone from origin — merged-under-rebase (consumed),
a tip that never landed (EXPIRED, with the sub_expired_abandoned event), and
no tip recorded at all (EXPIRED — the forever-wedge hole) — the live branch
survives, sub files are unlinked, and the drop_sub conflict path consumes
its sub too. An abandoned lane is never a retryable merge.

    python3 tests/test_yggci_branch_deleted_sweep.py
"""
import importlib.util
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SKILL = HERE.parent
FAILURES = []


def check(name, ok, detail=""):
    if ok:
        print(f"  ok  {name}")
    else:
        FAILURES.append(name)
        print(f"  ⛔ {name}  {detail}")


def load_module(tmp):
    sys.path.insert(0, str(SKILL))
    spec = importlib.util.spec_from_file_location("ygg_ci_under_test", SKILL / "ygg-ci.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    mod.LOGPATH = tmp / "ci-test.log"
    mod.CI_STATE = tmp
    mod.SUBS = tmp / "subs"
    mod.SUBS.mkdir(parents=True, exist_ok=True)
    mod._STDOUT_IS_LOG = False
    return mod


def fake_sub(mod, project, lane, tip):
    p = mod._sub_path(project, lane)
    tip_json = f'"{tip}"' if tip else "null"
    p.write_text('{"lane": "%s", "tip_at_enlist": %s}' % (lane, tip_json))
    return p


def main():
    tmp = Path(tempfile.mkdtemp(prefix="yggci-11230-sweep-test-"))
    mod = load_module(tmp)
    pcfg = {"remote": "origin"}
    repo = "/nonexistent-repo"  # every git_rev answers None: every branch "deleted"

    ancestry = {"aaa": True, "bbb": False}  # forced verdicts, no git involved
    mod.git_is_ancestor = lambda r, a, b: ancestry.get(a, False)
    mod.git_rev = lambda r, ref: "deadbeef" if ref == "origin/lane/d/alive" else None

    events = []
    mod._emit_event = lambda project, kind, dry=False, **f: events.append((kind, f))

    subs = [{"lane": "lane/a/merged", "tip_at_enlist": "aaa"},
            {"lane": "lane/b/abandoned", "tip_at_enlist": "bbb"},
            {"lane": "lane/c/no-tip", "tip_at_enlist": None},
            {"lane": "lane/d/alive", "tip_at_enlist": "ddd"}]
    for s in subs:
        fake_sub(mod, "yggterm", s["lane"], s["tip_at_enlist"])

    swept = mod._sweep_branch_deleted_subs("yggterm", pcfg, subs, repo, "mainsha")
    by_lane = {s["lane"]: s for s in swept}

    check("merged-under-rebase sub consumed as consumed-merged",
          by_lane.get("lane/a/merged", {}).get("disposition") == "consumed-merged")
    check("never-landed tip swept as expired-abandoned",
          by_lane.get("lane/b/abandoned", {}).get("disposition") == "expired-abandoned")
    check("no-tip sub swept as expired-abandoned (the forever-wedge hole)",
          by_lane.get("lane/c/no-tip", {}).get("disposition") == "expired-abandoned")
    check("sweep removed exactly the three branch-deleted subs",
          [s["lane"] for s in subs] == ["lane/d/alive"])
    check("all three sub files unlinked",
          not any(mod._sub_path("yggterm", lane).exists()
                  for lane in ("lane/a/merged", "lane/b/abandoned", "lane/c/no-tip")))
    check("the live branch's sub file survives",
          mod._sub_path("yggterm", "lane/d/alive").exists())
    check("EXPIRED paths emitted sub_expired_abandoned exactly twice",
          sum(1 for k, _ in events if k == "sub_expired_abandoned") == 2)
    check("the consumed-merged arm emits no expiry event",
          all(f.get("lane") != "lane/a/merged" for k, f in events if k == "sub_expired_abandoned"))

    # ── the drop_sub conflict path consumes its sub (H1: the dead flag) ──
    fake_sub(mod, "yggterm", "lane/b/abandoned", "bbb")
    conflicts = [{"lane": "lane/b/abandoned", "tip": "bbb",
                  "reason": "branch-deleted-abandoned", "drop_sub": True},
                 {"lane": "lane/e/hunk", "tip": "eee", "reason": "conflict"}]
    fake_sub(mod, "yggterm", "lane/e/hunk", "eee")
    mod._consume_expired_conflicts("yggterm", conflicts)
    check("drop_sub conflict consumed its sub file",
          not mod._sub_path("yggterm", "lane/b/abandoned").exists())
    check("plain conflict leaves its sub file alone",
          mod._sub_path("yggterm", "lane/e/hunk").exists())

    # ── idempotence: an absent sub file is not an error ──
    try:
        mod._consume_expired_conflicts(
            "yggterm", [{"lane": "lane/x/gone", "reason": "branch-deleted-abandoned",
                         "drop_sub": True}])
        check("absent sub file drop is a no-op", True)
    except Exception as e:  # noqa: BLE001
        check("absent sub file drop is a no-op", False, str(e))

    print()
    if FAILURES:
        print(f"{len(FAILURES)} FAILED")
        return 1
    print("All [11.230] branch-deleted sweep tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
