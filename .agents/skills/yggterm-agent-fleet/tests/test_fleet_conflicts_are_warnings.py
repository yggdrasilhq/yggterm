#!/usr/bin/env python3
"""Preserved sync-fleet conflicts are WARNINGS (exit 0), not convergence failures.

    python3 tests/test_fleet_conflicts_are_warnings.py [path-to-ygg-memory.py]

⛔ THE HOLE THIS PINS, measured 2026-10-18 on a fleet host: its memory tick
had been dead since 2026-08-22 (OnBootSec boot-skip — the timer unit pins that
shape in systemd/ygg-memory-sync.timer). The first real sync-fleet after repair found 9
preserved update-vs-delete conflicts in _global (the known zcode native-memory
flap class) and cmd_sync_fleet raised "fleet memory did not converge:
failed=9-preserved-conflict(s)" — exit 1 every tick, on a convergence leg that
converged fine. The exchange landed, evidence is preserved under conflicts/,
and the tick re-runs every minute: a semantic conflict is a fact to surface,
not a failure to alarm. Meanwhile a REAL infra failure (unreachable peer,
failed import) reads identical to a routine conflict in the raise — the worst
possible alarm polarity for a watchdog-adjacent path.

Strictness stays where it belongs:
  - `ygg-memory sync` (the strict recall gate) STILL fails closed on conflicts
    (pinned in test_ygg_memory.py);
  - `ygg-memory resolve` is the human/agent door that clears a conflict;
  - sync-fleet reports conflicts/missing-objects as "warnings" in its report
    and in the human output line, with a pointer to resolve.
"""

import contextlib
import importlib.util
import io
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_SCRIPT = HERE.parent / "ygg-memory.py"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def load_module(script_path):
    spec = importlib.util.spec_from_file_location("ygg_memory_conflict", str(script_path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Args:
    def __init__(self, root, **kw):
        self.root = str(root)
        self.mesh = []
        self.quick = True
        self.json = False
        self.quiet = False
        for key, value in kw.items():
            setattr(self, key, value)


def run():
    script = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SCRIPT
    mod = load_module(script)
    tmp_root = Path(tempfile.mkdtemp(prefix="ygg-mem-conflict-test-"))

    # Fixture — one door with two divergent causal heads (no base version).
    # Same recipe test_ygg_memory.py uses for its strict-gate polarity case.
    ns = "-conflict-area"
    ns_dir = mod.get_namespace_dir(tmp_root, ns)
    door = ns_dir / "campaign-conflict.md"
    door.write_text("# first head\n", encoding="utf-8")
    mod.append_journal_entry(tmp_root, ns, door.name, "campaign", "create", "claude", "first head")
    door.write_text("# second head\n", encoding="utf-8")
    mod.append_journal_entry(tmp_root, ns, door.name, "campaign", "update", "claude", "second head", base_version=None)

    report = mod.materialize_store(tmp_root)
    check("fixture produces a preserved conflict", report.get("conflicts", 0) >= 1, str(report)[:160])

    # Case 1 — _run_fleet_sync with an empty mesh (no ssh, no network) must
    # RETURN despite the preserved conflict, carrying it as a warning.
    try:
        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            fleet = mod._run_fleet_sync(tmp_root, [])
        check(
            "preserved conflict does not raise in _run_fleet_sync",
            True,
        )
        check(
            "conflict surfaces as a warning in the report",
            any("preserved-conflict" in w for w in fleet.get("warnings", [])),
            str(fleet.get("warnings"))[:160],
        )
    except RuntimeError as error:
        check("preserved conflict does not raise in _run_fleet_sync", False, f"raised: {error}")

    # Case 2 — cmd_sync_fleet exits clean (no exception) with the warning in
    # its human output, pointing at resolve.
    try:
        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            mod.cmd_sync_fleet(Args(tmp_root))
        out = captured.getvalue()
        check("cmd_sync_fleet returns instead of raising on conflicts", True)
        check(
            "human output carries the warning and the resolve door",
            "preserved-conflict" in out and "resolve" in out,
            out.strip()[:160],
        )
    except RuntimeError as error:
        check("cmd_sync_fleet returns instead of raising on conflicts", False, f"raised: {error}")

    # Case 3 — polarity: real failures still converge-fail. An unreachable
    # peer is still a failure; warnings never mask it.
    outcome = mod._fleet_outcome(unreachable=["peerx"], failures=[])
    check("unreachable peer still reported as failure", "unreachable" in outcome and "peerx" in outcome, outcome[:120])
    outcome = mod._fleet_outcome(unreachable=[], failures=["peerx:import-journal"])
    check("import failure still reported as failure", "peerx:import-journal" in outcome, outcome[:120])
    outcome = mod._fleet_outcome(unreachable=[], failures=[])
    check("clean run yields empty outcome", outcome == "", repr(outcome[:80]))

    # Case 4 — the timer unit shape: OnStartupSec (manager-relative), never
    # OnBootSec (boot-relative skip = the silent dead-timer shape).
    unit = (HERE.parent / "systemd" / "ygg-memory-sync.timer")
    if unit.is_file():
        text = unit.read_text(encoding="utf-8")
        directives = [ln.strip() for ln in text.splitlines() if ln.strip() and not ln.strip().startswith("#")]
        # ⛔ OnCalendar only: boot/manager-relative elapses (OnBootSec) get
        # skipped by slow boot, and the monotonic OnUnitActiveSec leg decays
        # to no-next-elapse on long-lived managers (both died silently). A
        # wall-clock schedule always yields a next elapse.
        check("timer uses OnCalendar (wall-clock, always a next elapse)",
              any(d.startswith("OnCalendar=*-*-* *:*:30") for d in directives))
        check("timer uses no relative elapse directives",
              not any(d.startswith(("OnBootSec", "OnStartupSec", "OnUnitActiveSec")) for d in directives))
    else:
        check("timer unit exists in skill systemd/", False, str(unit))

    print()
    if FAILURES:
        print(f"FAILED: {len(FAILURES)} check(s): {', '.join(FAILURES)}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(run())
