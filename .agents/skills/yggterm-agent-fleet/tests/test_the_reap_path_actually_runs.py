#!/usr/bin/env python3
"""The branch that DESTROYS a row is executed by a test, because it is the one that must be.

    python3 tests/test_the_reap_path_actually_runs.py

⛔ THE HOLE THIS PINS, and it is embarrassing in an instructive way. `ygg-deliver`'s
`_reap_if_never_briefed` — the function that force-folds a row — referred to a
variable that existed only in `main()`. Every call raised `NameError` instead of
deciding anything. It survived review, a full fleet suite, and a live run, because
**nothing ever executed it**: it only fires when a delivery times out or is refused,
which is exactly the path a healthy test run never takes.

⚠ It failed SAFE, and that is luck rather than design — an exception happens not to
reap. A different typo in the same place reaps a working lane.

⇒ The rule this encodes: **the more destructive a branch, the more certain it is that
nobody has run it.** Cheap paths get exercised incidentally; the expensive ones are
guarded by conditions that a passing test suite is designed never to meet.

Every fixture is invented.
"""
import importlib.util
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))

spec = importlib.util.spec_from_file_location("deliver_under_test", HERE / "ygg-deliver.py")
deliver = importlib.util.module_from_spec(spec)
argv, sys.argv = sys.argv, ["ygg-deliver.py"]
try:
    spec.loader.exec_module(deliver)
except SystemExit:
    pass
finally:
    sys.argv = argv

FAILURES = []

ID = "8badf00d-1111-4222-8333-444444444444"
home = Path(tempfile.mkdtemp(prefix="ygg-reap-fixture-"))
try:
    # A row that HAS written something. The reap must refuse it — and must do so by
    # RUNNING, not by raising.
    planted = home / ".codex/sessions/2026/01/02"
    planted.mkdir(parents=True)
    (planted / f"rollout-2026-01-02T03-04-05-{ID}.jsonl").write_text(
        '{"type":"event_msg","payload":{"type":"agent_message","message":"invented"}}\n')

    calls = []
    real_has = deliver.ygg_transcript.has_transcript
    deliver.ygg_transcript.has_transcript = (
        lambda uuid, kind=None, home=str(home): calls.append((uuid, kind))
        or real_has(uuid, kind=kind, home=home))
    # ⛔ Neutralise the actual fold: this test proves the DECISION, and a test that
    #    can reap something is a test that will eventually reap something real.
    folded = []
    deliver.subprocess.run = lambda *a, **k: folded.append(a) or None

    try:
        result = deliver._reap_if_never_briefed(ID, f"remote-xx://invented/{ID}",
                                                "invented-host", None, "codex")
    except NameError as exc:
        FAILURES.append(f"the reap path raised NameError instead of deciding: {exc} — "
                        f"this is the branch that destroys a row")
        result = None
    except Exception as exc:                       # noqa: BLE001
        FAILURES.append(f"the reap path raised {type(exc).__name__}: {exc}")
        result = None

    if not calls:
        FAILURES.append("the reap never asked whether the row had written anything")
    elif calls[0][1] != "codex":
        FAILURES.append(f"the row's CLI was not passed through: got {calls[0][1]!r} — "
                        f"so a non-reference row is judged against every store instead "
                        f"of its own, which is slower and, on a name collision, wrong")
    if folded:
        FAILURES.append("a row with a transcript was folded — the interlock is inverted")
    if result != 6:
        FAILURES.append(f"expected the keep-it return code 6, got {result!r}")
finally:
    shutil.rmtree(home, ignore_errors=True)

if FAILURES:
    print("FAIL")
    for f in FAILURES:
        print("  ⛔", f)
    sys.exit(1)
print("ok — the reap path runs, asks the right question, and keeps a row that has spoken")
