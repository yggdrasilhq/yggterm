#!/usr/bin/env python3
"""A reap may destroy a row on ABSENT, never on "we could not look".

    python3 tests/test_a_destructive_check_says_whether_it_looked.py

⛔⛔ THE HOLE THIS PINS, measured on the live row plane 2026-08-22 while making
two different CLIs greet each other. `ygg-deliver`'s reap asked
`has_transcript()` — a BOOLEAN — and destroyed the row when it answered `False`.
Three different situations produce that same `False`:

    the row really never wrote a word          → destroying it is correct
    its CLI declares no transcript template    → we never looked
    its transcript is on ANOTHER MACHINE       → we looked in the wrong place

Only the first is evidence. The other two were **ten and thirty-four live rows**
on the day this was written — a kimi row whose transcript is 30 KB on disk, seven
opencode rows sharing a 315 KB store, and thirty-four rows belonging to another
host. On a four-row sample of the last group asked on the machine that owns them,
three had a transcript. Each would have been force-folded by a delivery that
merely ran late, and each was doing exactly what it was spawned to do.

⇒ `transcript_evidence` returns FOUND / ABSENT / UNMEASURABLE, and the caller
that destroys is made to say which one it is acting on. It still does NOT guess a
path for an unmeasured CLI — that is the mistake the table exists to refuse. It
reports that it cannot see, which is both the honest answer and the safe one.

⚠ THE SECOND DEFECT, WHICH THE FIRST FIX WOULD HAVE CREATED. Narrowing by a row's
`icon_kind` looked like a pure improvement until the PRODUCER was read: the codex
family reports the historical `session`, and **that one mark names two CLIs**. A
resolver returning the first match would have silently stopped finding one of
them in the act of fixing the other. `row_icon_kind` in yggterm-core owns the
spelling; a Rust lock ratifies every alias in the table against it.

Every fixture here is invented. The shapes are real; the ids and paths are not.
"""
import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


tr = _load("ygg_transcript", "ygg_transcript.py")
deliver = _load("ygg_deliver", "ygg-deliver.py")
rowarg = _load("ygg_rowarg", "ygg_rowarg.py")

TABLE = json.loads((HERE / "cli-stores.json").read_text())["clis"]

# Invented ids. Real-shaped, belonging to nothing.
WROTE = "3f9a1c60-11d4-4e7b-9a02-5c8e77b41d2a"
SILENT = "8b2e4470-6c19-42af-83d1-0f6a9e35c7b4"

FAILURES = []


def check(name, got, want):
    if got != want:
        FAILURES.append(f"{name}: got {got!r}, wanted {want!r}")


def seed(home, kind, session_id):
    """Write a file exactly where this CLI's declared template says one goes."""
    template = TABLE[kind]["transcript"]
    if not template:
        return None
    relative = template.replace("{id}", session_id)
    # Resolve the wildcards to a concrete, invented directory.
    parts = []
    for segment in relative.split("/"):
        if segment == "**":
            parts.append("2031")
        elif "*" in segment:
            parts.append(segment.replace("*", "rollout-2031-01-01T00-00-00")
                         if segment.startswith("rollout-") else segment.replace("*", "a-project"))
        else:
            parts.append(segment)
    path = os.path.join(home, *parts)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as handle:
        handle.write('{"note":"invented fixture"}\n')
    return path


with tempfile.TemporaryDirectory() as home:
    # ── The three answers, on a CLI that declares a template ────────────────
    seed(home, "claude-code", WROTE)
    check("a row that wrote, found",
          tr.transcript_evidence(WROTE, kind="claude-code", home=home), tr.FOUND)
    check("a row that never wrote, in a store we CAN read",
          tr.transcript_evidence(SILENT, kind="claude-code", home=home), tr.ABSENT)

    # ── The two that must never read as absence ─────────────────────────────
    for kind in [k for k, e in TABLE.items() if not e.get("transcript")]:
        check(f"{kind} declares no template, so we did not look",
              tr.transcript_evidence(SILENT, kind=kind, home=home), tr.UNMEASURABLE)
    check("an unknown kind means we do not know where to look",
          tr.transcript_evidence(SILENT, kind="not-a-registered-cli", home=home),
          tr.UNMEASURABLE)
    check("no kind at all is not evidence either",
          tr.transcript_evidence(SILENT, kind=None, home=home), tr.UNMEASURABLE)
    check("a host that cannot be ASKED is not an empty store",
          tr.transcript_evidence(SILENT, kind="claude-code", home=home,
                                 host="host.invalid.example", ssh_timeout=20),
          tr.UNMEASURABLE)

    # ── The historical mark names TWO CLIs, and narrowing must keep both ────
    codex_family = [k for k, e in TABLE.items() if e.get("icon_kind") == "session"]
    check("`session` names both codex variants", sorted(codex_family),
          ["codex", "codex-litellm"])
    check("narrowing on `session` keeps every candidate",
          len(tr.templates_for("session")), len(codex_family))
    for kind in codex_family:
        seed(home, kind, WROTE)
        check(f"a {kind} row is found through the `session` mark",
              tr.transcript_evidence(WROTE, kind="session", home=home), tr.FOUND)
        os.remove(seed(home, kind, WROTE))

    # ── Every entry declares the spelling a row reports ─────────────────────
    for slug, entry in TABLE.items():
        if not entry.get("icon_kind"):
            FAILURES.append(f"{slug} has no icon_kind in cli-stores.json")

    # ── ⛔ AND THE DESTRUCTIVE CALLER ITSELF ────────────────────────────────
    # ⚠ Patch the module `ygg-deliver` ACTUALLY HOLDS. The first draft of this
    #   block patched a second instance loaded under the same name, so all three
    #   arms fell straight through to the fold and the test reported a failure
    #   about the code instead of about itself. A stub fold verb stands in for the
    #   real one so the ABSENT arm reaches the call rather than the "no fold verb"
    #   early return — otherwise every arm looks identical and nothing is proven.
    stage = tempfile.mkdtemp()
    with open(os.path.join(stage, "ygg-fold.py"), "w") as handle:
        handle.write("#!/usr/bin/env python3\n")
    deliver.HERE = stage
    live = deliver.ygg_transcript
    original = live.transcript_evidence
    ran = {}

    class _Recorder:
        @staticmethod
        def run(*args, **kwargs):
            ran["called"] = True
            return type("Reply", (), {"returncode": 0, "stdout": "", "stderr": ""})()

    real_subprocess = deliver.subprocess
    deliver.subprocess = _Recorder
    try:
        for verdict in (live.FOUND, live.UNMEASURABLE, live.ABSENT):
            live.transcript_evidence = lambda *a, _v=verdict, **k: _v
            ran.clear()
            code = deliver._reap_if_never_briefed(SILENT, "claude-code", None)
            check(f"the reap DESTROYS on {verdict}", ran.get("called", False),
                  verdict == live.ABSENT)
            check(f"the reap's exit code on {verdict}", code, 6)
    finally:
        live.transcript_evidence = original
        deliver.subprocess = real_subprocess

if FAILURES:
    print("FAIL")
    for failure in FAILURES:
        print(f"  {failure}")
    sys.exit(1)
print("PASS — the destructive check reports whether it looked")
