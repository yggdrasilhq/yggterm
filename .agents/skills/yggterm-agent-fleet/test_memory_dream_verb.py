#!/usr/bin/env python3
"""[mem-dream slice 2] dream verb + search verb: delta isolation, budget,
response validation refusals, critic gate, journaled apply, provenance search.

    python3 tests/test_memory_dream_verb.py [path-to-ygg-memory.py]

⛔ THE HOLES THIS PINS (design door memory-dreaming-design, 2026-10-06):
capture without consolidation means every session re-derives orientation
from >60k tokens of raw doors; a dreamer that ingests its own output, or
an un-stamped claim, or a contested claim left standing, rebuilds the
confabulation problem one layer up. The laws under test: ingestion
isolation (dreamer-origin + owned basenames never enter the brief), the
per-claim [as-of; proof] stamps with a closed proof vocabulary, the
MANDATORY adversarial critic verdicts, contested-claim demotion, the ops
whitelist (append-section / propose-tombstone only), byte caps on NOW.md
and owner-now, and watermark advance only on successful apply.
"""

import importlib.util
import json
import re
import sys
import tempfile
from pathlib import Path


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def journal_door(module, root, ns, filename, content, harness="zcode", origin=None, action="update"):
    ns_dir = module.get_namespace_dir(root, ns)
    dest = ns_dir / filename
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(content, encoding="utf-8")
    return module.append_journal_entry(
        root, ns, filename, "door", action if action != "create" else "create",
        harness, f"test {filename}", origin=origin,
    )


GOOD_NOW = """## §CURRENT_STATUS
- Slice 2 dream verb landed and deployed [as-of: 2026-10-06; proof: deployed]

## §NEXT_UNITS
- Slice 3 H_vocab phase telemetry [as-of: 2026-10-06; proof: claimed]

## §ACTIVE_TRAPS
- Remote heredocs get shell-eaten [as-of: 2026-10-06; proof: measured-live]

## §POINTERS
- campaign-yggterm.md — the campaign door
"""

GOOD_RESPONSE = f"""=== NOW.MD ===
{GOOD_NOW}
=== OPS ===
[]
=== CLAIMS ===
[
  {{"claim_id": "c1", "text": "Slice 2 dream verb landed and deployed", "as_of": "2026-10-06", "proof": "deployed", "source_refs": ["seq#1/door-a.md"]}},
  {{"claim_id": "c2", "text": "Slice 3 H_vocab phase telemetry", "as_of": "2026-10-06", "proof": "claimed", "source_refs": ["seq#1/door-a.md"]}},
  {{"claim_id": "c3", "text": "Remote heredocs get shell-eaten", "as_of": "2026-10-06", "proof": "measured-live", "source_refs": ["seq#1/door-a.md"]}}
]
=== OWNER-NOW ===
null
"""

GOOD_CRITIC = """=== VERDICTS ===
[
  {"claim_id": "c1", "verdict": "upheld", "counter_example": null},
  {"claim_id": "c2", "verdict": "upheld", "counter_example": null},
  {"claim_id": "c3", "verdict": "upheld", "counter_example": null}
]
"""


class Args:
    def __init__(self, **kw):
        self.subcommand = "dream"
        self.root = None
        self.harness = "zcode"
        self.ns = None
        self.json = False
        self.prepare = False
        self.apply = False
        self.run = False
        self.status = False
        self.phase = "synthesis"
        self.extra_input = []
        self.budget = None
        self.dry_run = False
        self.composer = "sol"
        self.model = "gpt-6.1-sol"
        self.effort = "high"
        self.codex_bin = None
        self.all = False
        self.ignore_case = False
        self.max_lines = 50
        self.pattern = ""
        self.__dict__.update(kw)


def build_store(module, home: Path) -> Path:
    root = home / "memory"
    root.mkdir()
    ns = "-t-dream"
    module.validate_namespace(ns)
    journal_door(module, root, ns, "door-a.md", "# door a\n\nalpha content\n", action="create")
    journal_door(module, root, ns, "door-b.md", "# door b\n\nbeta content\n", action="create")
    # Dreamer output must NEVER enter the dream input set.
    journal_door(module, root, ns, "door-c.md", "composer echo\n", origin="dreamer")
    journal_door(module, root, ns, "NOW.md", "previous rollup\n", origin="dreamer")
    return root


def test_collect_isolation(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        deltas, captured = module.collect_dream_deltas(root, "-t-dream", 0)
        files = [d["file"] for d in deltas]
        if files != ["door-b.md", "door-a.md"]:
            failures.append(f"isolation: expected [door-b, door-a] newest-first, got {files}")
        if captured <= 0:
            failures.append(f"isolation: captured seq {captured} not advanced")
        if any("composer echo" in d["content"] or "previous rollup" in d["content"] for d in deltas):
            failures.append("isolation: dreamer-origin content leaked into deltas")


def test_prepare_budget_and_gate(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        # Clean tick on a fresh watermark with everything already consumed.
        staged = module.prepare_dream(root, "-t-dream", "synthesis", [], 10 ** 9)
        if staged is None:
            failures.append("prepare: returned clean tick although 4 events are new")
        else:
            brief_path, manifest = staged
            text = brief_path.read_text(encoding="utf-8")
            if "door-a.md" not in text or "door-b.md" not in text:
                failures.append("prepare: brief missing door deltas")
            raw_section = re.sub(r"## CONTRACT.*?## RAW", "", text, flags=re.DOTALL)
            raw_section = re.sub(r"## CURRENT ROLLUP.*?## RAW", "", raw_section, flags=re.DOTALL)
            raw_section = raw_section.split("## EXTRA")[0]
            if "door-c.md" in raw_section or "NOW.md" in raw_section or "composer echo" in raw_section or "previous rollup" in raw_section:
                failures.append("prepare: dreamer output present in brief input sections")
            if "(none — first dream)" in text:
                failures.append("prepare: current NOW.md rollup not embedded")
            if manifest["captured_seq"] <= 0:
                failures.append("prepare: manifest captured_seq not set")
        # Budget: force everything dropped.
        staged2 = module.prepare_dream(root, "-t-dream", "synthesis", [], 10)
        if staged2 is None or staged2[1]["dropped_for_budget"] != 2:
            failures.append(f"prepare: budget drop accounting wrong: {staged2 and staged2[1]}")


def write_pending(module, root, ns, response=GOOD_RESPONSE, critic=GOOD_CRITIC):
    pending = module.get_dream_dir(root, ns) / "pending"
    (pending / "RESPONSE.md").write_text(response, encoding="utf-8")
    if critic is not None:
        (pending / "CRITIC.md").write_text(critic, encoding="utf-8")


def test_apply_happy_and_watermark(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        ns = "-t-dream"
        staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        write_pending(module, root, ns)
        module.cmd_dream(Args(apply=True, ns=ns, root=str(root)))
        ns_dir = module.get_namespace_dir(root, ns, create=False)
        now = (ns_dir / "NOW.md").read_text(encoding="utf-8")
        if not now.startswith("---\nname: NOW\n"):
            failures.append("apply: NOW.md frontmatter not minted")
        if "## §CURRENT_STATUS" not in now:
            failures.append("apply: NOW.md body missing sections")
        events = [r for r in module.read_journal_entries(root, namespace=ns) if r.get("file") == "NOW.md"]
        if not events or events[-1].get("origin") != "dreamer":
            failures.append("apply: NOW.md not journaled with dreamer origin")
        wm = module.load_dream_watermark(root, ns)
        if wm["last_seq"] != staged[1]["captured_seq"]:
            failures.append(f"apply: watermark {wm['last_seq']} != captured {staged[1]['captured_seq']}")
        if not wm.get("dreams"):
            failures.append("apply: dream record not appended to watermark")
        dream_dir = module.get_dream_dir(root, ns, create=False)
        if not (dream_dir / "done").is_dir() or not list((dream_dir / "done").rglob("OPLOG.md")):
            failures.append("apply: done/OPLOG.md missing")
        if (dream_dir / "pending").exists():
            failures.append("apply: pending/ not consumed")
        # Second prepare is a clean tick now.
        if module.prepare_dream(root, ns, "synthesis", [], 10 ** 9) is not None:
            failures.append("apply: post-apply prepare not a clean tick")


def expect_refusal(module, failures, label, response=GOOD_RESPONSE, critic=GOOD_CRITIC, ns="-t-dream"):
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        write_pending(module, root, ns, response=response, critic=critic)
        try:
            module.cmd_dream(Args(apply=True, ns=ns, root=str(root)))
        except SystemExit as exit_error:
            if exit_error.code != 2:
                failures.append(f"refusal {label}: exit code {exit_error.code} != 2")
            return
        failures.append(f"refusal {label}: apply did not refuse")


def test_apply_refusals(module, failures):
    missing_section = GOOD_RESPONSE.replace("## §ACTIVE_TRAPS\n- Remote heredocs get shell-eaten [as-of: 2026-10-06; proof: measured-live]\n\n", "")
    unstamped = GOOD_RESPONSE.replace(
        "- Slice 3 H_vocab phase telemetry [as-of: 2026-10-06; proof: claimed]",
        "- Slice 3 H_vocab phase telemetry",
    )
    bad_proof = GOOD_RESPONSE.replace("proof: claimed", "proof: vibes")
    frontmatter = GOOD_RESPONSE.replace("=== NOW.MD ===\n", "=== NOW.MD ===\n---\nname: NOW\n---\n", 1)
    oversize = GOOD_RESPONSE.replace(
        "## §POINTERS",
        "- " + ("x" * 3000) + "\n\n## §POINTERS",
    )
    no_critic = (GOOD_RESPONSE, None)
    contested = (
        GOOD_RESPONSE,
        GOOD_CRITIC.replace('{"claim_id": "c1", "verdict": "upheld", "counter_example": null}',
                            '{"claim_id": "c1", "verdict": "contested", "counter_example": "door-a says otherwise"}'),
    )
    bad_op = GOOD_RESPONSE.replace("[]", json.dumps([{"op": "rewrite-door", "file": "door-a.md", "reason": "nope"}]))
    bad_marker = GOOD_RESPONSE.replace("[]", json.dumps([{"op": "append-section", "file": "door-a.md", "section": "§NOPE", "entry": "- x", "reason": "r"}]))
    tombstone_missing = GOOD_RESPONSE.replace("[]", json.dumps([{"op": "propose-tombstone", "file": "ghost.md", "reason": "r"}]))

    expect_refusal(module, failures, "missing-section", response=missing_section)
    expect_refusal(module, failures, "unstamped-bullet", response=unstamped)
    expect_refusal(module, failures, "bad-proof-vocab", response=bad_proof)
    expect_refusal(module, failures, "composer-frontmatter", response=frontmatter)
    expect_refusal(module, failures, "oversize-now", response=oversize)
    expect_refusal(module, failures, "no-critic-verdicts", response=no_critic[0], critic=no_critic[1])
    expect_refusal(module, failures, "contested-undemoted", response=contested[0], critic=contested[1])
    expect_refusal(module, failures, "unknown-op", response=bad_op)
    expect_refusal(module, failures, "section-marker-missing", response=bad_marker)
    expect_refusal(module, failures, "tombstone-ghost-door", response=tombstone_missing)

    # Contested but visibly demoted in the rollup is ALLOWED (no refusal).
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        ns = "-t-dream"
        module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        demoted = GOOD_RESPONSE.replace(
            "- Slice 2 dream verb landed and deployed [as-of: 2026-10-06; proof: deployed]",
            "- Slice 2 dream verb landed and deployed [as-of: 2026-10-06; proof: deployed] [CONTESTED: door-a says otherwise]",
        )
        write_pending(module, root, ns, response=demoted, critic=contested[1])
        try:
            module.cmd_dream(Args(apply=True, ns=ns, root=str(root)))
        except SystemExit as exit_error:
            failures.append(f"contested-demoted: wrongly refused (code {exit_error.code})")


def test_append_section_op(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        ns = "-t-dream"
        journal_door(module, root, ns, "door-gotchas.md",
                     "# door\n\n## §GOTCHAS\n\n- old gotcha\n\n## §NEXT\n\nnext body\n")
        module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        ops = json.dumps([
            {"op": "append-section", "file": "door-gotchas.md", "section": "§GOTCHAS",
             "entry": "- new dream gotcha [dream 2026-10-06]", "reason": "surfaced by dream"}
        ])
        response = GOOD_RESPONSE.replace("[]", ops)
        write_pending(module, root, ns, response=response)
        module.cmd_dream(Args(apply=True, ns=ns, root=str(root)))
        door = (module.get_namespace_dir(root, ns, create=False) / "door-gotchas.md").read_text(encoding="utf-8")
        gotchas = door.split("§GOTCHAS")[1].split("## §NEXT")[0]
        if "new dream gotcha" not in gotchas:
            failures.append("append-section: entry not inside §GOTCHAS section")
        if "old gotcha" not in gotchas:
            failures.append("append-section: old entries lost")
        if "next body" not in door.split("## §NEXT")[1]:
            failures.append("append-section: following section damaged")


def test_owner_now(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        ns = "-t-dream"
        module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        owner = json.dumps({"body": "- mood: focused [as-of: 2026-10-06]", "evidence": ["seq#1/door-a.md"]})
        response = re.sub(r"=== OWNER-NOW ===\nnull", f"=== OWNER-NOW ===\n{owner}", GOOD_RESPONSE)
        write_pending(module, root, ns, response=response)
        module.cmd_dream(Args(apply=True, ns=ns, root=str(root)))
        owner_door = module.get_namespace_dir(root, "_global", create=False) / "owner-now.md"
        if not owner_door.is_file():
            failures.append("owner-now: door not published to _global")
        else:
            text = owner_door.read_text(encoding="utf-8")
            if "type: owner-now" not in text or "mood: focused" not in text:
                failures.append("owner-now: content/frontmatter wrong")


def test_search(module, failures, captured_stdout):
    import contextlib
    import io

    with tempfile.TemporaryDirectory() as tmp:
        root = build_store(module, Path(tmp))
        journal_door(module, root, "-t-other", "needle.md", "# other\n\nNEEDLE here\n")
        with contextlib.redirect_stdout(io.StringIO()) as buffer:
            module.cmd_search(Args(subcommand="search", ns="-t-other", pattern="needle", ignore_case=True, max_lines=50, root=str(root)))
        out = buffer.getvalue()
        if "-t-other/needle.md:3:" not in out or "NEEDLE here" not in out:
            failures.append(f"search: provenance hit missing: {out!r}")
        with contextlib.redirect_stdout(io.StringIO()) as buffer:
            module.cmd_search(Args(subcommand="search", ns=None, all=True, pattern="no-such-token-zz", ignore_case=False, max_lines=50, root=str(root)))
        if "no matches" not in buffer.getvalue():
            failures.append("search: no-match case not reported")
        with contextlib.redirect_stdout(io.StringIO()) as buffer:
            module.cmd_search(Args(subcommand="search", ns=None, all=True, pattern="content", ignore_case=False, max_lines=1, root=str(root)))
        if "capped at 1" not in buffer.getvalue():
            failures.append("search: max-lines cap not applied")


def test_self_disperse_guard(module, failures):
    """[11.236] sync-fleet may push runner bytes only from the repo SSOT."""
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        ssot_dir = home / "gh/yggterm/.agents/skills/yggterm-agent-fleet"
        ssot_dir.mkdir(parents=True)
        (ssot_dir / "ygg-memory.py").write_text("SSOT BYTES\n", encoding="utf-8")
        worktree = home / "gh/yggterm--stale"
        worktree.mkdir()
        stale = worktree / "ygg-memory.py"
        stale.write_text("STALE BYTES\n", encoding="utf-8")
        fresh_copy = home / ".local/bin"
        fresh_copy.mkdir()
        twin = fresh_copy / "ygg-memory.py"
        twin.write_text("SSOT BYTES\n", encoding="utf-8")

        if module._self_disperse_source(stale.resolve(), home=home) is not None:
            failures.append("disperse-guard: stale module would disperse (must be None)")
        if module._self_disperse_source(twin.resolve(), home=home) != fresh_copy.resolve():
            failures.append("disperse-guard: SSOT-identical install must disperse")
        if module._self_disperse_source((ssot_dir / "ygg-memory.py").resolve(), home=home) != ssot_dir:
            failures.append("disperse-guard: the SSOT itself must disperse")
        (ssot_dir / "ygg-memory.py").unlink()
        if module._self_disperse_source(stale.resolve(), home=home) != worktree.resolve():
            failures.append("disperse-guard: no-SSOT fallback must keep legacy behavior")


def main() -> int:
    default = Path(__file__).resolve().parent.parent / "ygg-memory.py"
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else default)
    failures: list[str] = []
    test_collect_isolation(module, failures)
    test_prepare_budget_and_gate(module, failures)
    test_apply_happy_and_watermark(module, failures)
    test_apply_refusals(module, failures)
    test_append_section_op(module, failures)
    test_owner_now(module, failures)
    test_search(module, failures, None)
    test_self_disperse_guard(module, failures)
    if failures:
        print(f"dream-verb tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("dream-verb tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
