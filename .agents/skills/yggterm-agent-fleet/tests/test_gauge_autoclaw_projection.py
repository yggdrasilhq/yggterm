#!/usr/bin/env python3
"""The autoclaw gauge must gauge the NEXT request, not the last completed one.

    python3 tests/test_gauge_autoclaw_projection.py [path-to-gauge]

⛔ THE HOLE THIS PINS, measured 2026-10-18: read_usage_autoclaw returned the
LAST assistant row's contextTokens — the prompt weight of the request that
ALREADY finished. Every relay/compact/stop decision made off the gauge was one
turn stale; the harness's own GUI indicator (fresher) read HIGHER, and the
gap widens exactly when it matters (long tool-result turns). contextTokens is
the PROMPT weight of that request; the assistant's output does NOT carry into
the next request's prompt (AutoClaw strips thinking; measured growth
~0.2-3k/turn even with 6-12k output tokens). So the projection is
self-calibrating: last + median(recent inter-request growth), with compaction
drops excluded from the median. Fix must never report LESS than the last
observed request (max semantics), and a fresh single-row transcript projects
its only value unchanged.
"""

import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_SCRIPT = HERE.parent / "context-relay-gauge.py"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def load_module(script_path):
    spec = importlib.util.spec_from_file_location("gauge_projection", str(script_path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run():
    script = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SCRIPT
    mod = load_module(script)
    project = mod.project_next_request

    series = [88220, 90286, 91872, 92782, 93030, 93651, 94086]
    proj = project(series)
    check("projects above the last completed request", proj > series[-1], str(proj))
    check("projection is modest (no output-token blowup)", proj - series[-1] <= 5000, f"delta={proj - series[-1]}")

    check("single row projects itself", project([50000]) == 50000, str(project([50000])))
    check("empty series projects zero", project([]) == 0, str(project([])))

    compacted = [90000, 95000, 30000, 31000, 32000]
    proj = project(compacted)
    check("compaction drop excluded from median", proj >= 32000 and proj <= 34000, str(proj))

    check("negative-only series stays at last", project([70000, 60000]) == 60000, str(project([70000, 60000])))

    src = Path(script).read_text(encoding="utf-8")
    check("run_autoclaw publishes projected_next", '"projected_next"' in src)
    check("effective uses max(used, projected)", "max(used, projected" in src)

    print()
    if FAILURES:
        print(f"FAILED: {len(FAILURES)} check(s): {', '.join(FAILURES)}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(run())
