#!/usr/bin/env python3
"""A row's transcript is found whichever CLI wrote it — not just the reference one.

    python3 tests/test_transcript_lookup_covers_every_cli.py

⛔ THE HOLE THIS PINS: eleven callsites across seven fleet verbs answered "has this
row ever written a word?" with ONE CLI's store hardcoded. The registry declares a
store for every registered CLI and they share no layout, so for a row of any other
CLI the glob returned nothing — **and nothing is not an error, it is the same
answer a row that has genuinely never done anything gives.** One of the callsites
that consumed that answer REAPS the row.

⚠ This test builds a fake home whose directory shapes match the ones measured on
real machines, and requires the resolver to find the planted file in each. The ids
and paths are invented; only the SHAPES are real, and the shapes are the thing that
broke. It also asserts the defect it replaces — that the old single glob sees one
store out of many — so a regression cannot pass by making both answers agree.
"""
import importlib.util
import os
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("ygg_transcript", HERE / "ygg_transcript.py")
T = importlib.util.module_from_spec(spec)
spec.loader.exec_module(T)

FAILURES = []


def check(name, got, want):
    if got != want:
        FAILURES.append(f"{name}: got {got!r}, wanted {want!r}")


OTHER = "11112222-3333-4444-8555-666677778888"

# ⛔ A DISTINCT ID PER CLI, and that is not tidiness — it is what the world looks
#    like. An earlier draft planted ONE id in every store at once, so the lookup
#    that holds only a uuid had six equally valid answers and returned whichever
#    file was written last. That fixture was testing a tie that cannot occur, and
#    it failed the resolver for being unable to read minds.
IDS = {
    "claude-code": "0fedcba9-8765-4321-8fed-cba987654321",
    "codex":       "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
    "pi":          "2b3c4d5e-6f7a-4b8c-8d9e-1f2a3b4c5d6e",
    "muse":        "3c4d5e6f-7a8b-4c9d-8e9f-2a3b4c5d6e7f",
    "antigravity": "4d5e6f7a-8b9c-4d0e-8f9a-3b4c5d6e7f80",
    "grok-build":  "5e6f7a8b-9c0d-4e1f-8a9b-4c5d6e7f8091",
}

# Directory shapes as measured on real machines — a decorated file name, a plain
# one, a timestamp-prefixed one, an id-as-directory, and one nested several levels
# above its file.
PLANTED = {
    "claude-code": ".claude/projects/-invented-project/{id}.jsonl",
    "codex":       ".codex/sessions/2026/01/02/rollout-2026-01-02T03-04-05-{id}.jsonl",
    "pi":          ".pi/agent/sessions/--invented--/2026-01-02T03-04-05-000Z_{id}.jsonl",
    "muse":        ".local/share/muse/sessions/2026/01/02/{id}/session.jsonl",
    "antigravity": ".gemini/antigravity-cli/brain/{id}/.system_generated/logs/transcript_full.jsonl",
    "grok-build":  ".grok/sessions/%2Finvented%2Fcwd/{id}/summary.json",
}
PLANTED = {cli: rel.format(id=IDS[cli]) for cli, rel in PLANTED.items()}

home = Path(tempfile.mkdtemp(prefix="ygg-transcript-fixture-"))
try:
    for cli, rel in PLANTED.items():
        path = home / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f'{{"note":"invented fixture for {cli}","ack":"GREETING-TOKEN"}}\n')

    for cli, rel in PLANTED.items():
        want, sid = str(home / rel), IDS[cli]
        check(f"{cli}: found when the CLI is known",
              T.transcript_of(sid, kind=cli, home=str(home)), want)
        # ⛔ The watchdogs mostly hold a uuid and nothing else, so the blind lookup
        #    is the one that actually runs in production — and it must land on the
        #    right store out of six without being told which.
        check(f"{cli}: found when only the uuid is known",
              T.transcript_of(sid, home=str(home)), want)
        check(f"{cli}: the ack inside it is readable",
              T.carries(sid, "GREETING-TOKEN", kind=cli, home=str(home)), True)

    check("a session that wrote nothing is reported as such",
          T.has_transcript(OTHER, home=str(home)), False)
    check("a CLI that declares no store resolves to nothing, rather than guessing",
          T.templates_for("opencode"), [])

    # ⛔ REPRODUCE THE DEFECT. If a future change made the resolver fall back to one
    #    store, every check above could still pass for that store alone — so pin
    #    that the old single-glob answer is WRONG for the others.
    import glob as _g
    old_seen = {cli for cli, sid in IDS.items()
                if _g.glob(os.path.join(str(home), f".claude/projects/*/{sid}.jsonl"))}
    check("the fixture reproduces the defect (one hardcoded store sees only itself)",
          old_seen, {"claude-code"})
    new_seen = {cli for cli, sid in IDS.items() if T.has_transcript(sid, home=str(home))}
    check("and the resolver sees all of them", new_seen, set(PLANTED))

    # The remote arm must carry every store too, or ssh answers for one CLI.
    cmd = T.remote_find_command(IDS["codex"])
    for fragment in (".codex/sessions", ".claude/projects", "antigravity-cli"):
        if fragment not in cmd:
            FAILURES.append(f"remote_find_command omits {fragment}: an ssh probe would "
                            f"answer for one CLI and read as silence for the rest")

    # ⛔⛔ `**` IS NOT A GLOB IN `sh`. Three stores are declared with it, Python
    #    expands it and a POSIX shell does not — so an `ls`-shaped remote probe
    #    returns nothing for those three, silently, and every caller reads that as
    #    "this row has never written a word": the defect this module exists to end,
    #    reintroduced one layer down. It was caught by RUNNING the command, not by
    #    reading it, which is why the guard is here and not in a comment.
    for kind in (None, "codex", "muse", "codex-litellm"):
        if "**" in T.remote_find_command(IDS["codex"], kind=kind):
            FAILURES.append(f"remote_find_command({kind!r}) still emits `**`, which a "
                            f"POSIX shell does not expand — it will match nothing over "
                            f"ssh and the silence will read as an empty transcript")
finally:
    shutil.rmtree(home, ignore_errors=True)

if FAILURES:
    print("FAIL")
    for f in FAILURES:
        print("  ⛔", f)
    sys.exit(1)
print(f"ok — every declared store is reachable ({len(PLANTED)} CLIs planted and found)")
