#!/usr/bin/env python3
"""[slice 3] bounded retrieval + dependency-scoped fences: safe commits under
sustained concurrency, eventual service for a starving namespace, rare early
facts retrievable after repeated compaction, and restatement never laundering
into corroboration.

    python3 tests/test_dream_bounded.py [path-to-ygg-memory.py]

⛔ THE HOLES THIS PINS (round-3 gap review Q-D gaps 1+2, slice-3 falsifiers):
gap 1 — "compose against an immutable prefix; leave events after W pending;
add overdue-service priority" — the stop-the-world fence let ANY concurrent
write refuse a finished synthesis forever, and nothing guaranteed a starving
small namespace ever got served. gap 2 — "retain claims by stable identity
with evidence links, bounded retrieval; distinguish new evidence confirming
an old claim from mere restatement" — wholesale claim injection exhausts
context at multi-year scale, and copied repetition must never count as
independent evidence.
"""

import datetime
import importlib.util
import json
import sys
import tempfile
import time
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
        self.force = False
        self.quiet = False
        self.__dict__.update(kw)


def journal_door(module, root, ns, filename, content, ts=None, origin=None):
    ns_dir = module.get_namespace_dir(root, ns)
    ns_dir.mkdir(parents=True, exist_ok=True)
    (ns_dir / filename).write_text(content, encoding="utf-8")
    module.append_journal_entry(root, ns, filename, "door", "update", "zcode", f"test {filename}")
    if ts is not None:  # rewrite the record's clock for age-sensitive tests
        journal = root / "journal.jsonl"
        rows = [json.loads(l) for l in journal.read_text().splitlines() if l.strip()]
        for row in rows:
            if row.get("file") == filename and row.get("ns") == ns:
                row["ts"] = ts
        journal.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")


GOOD_NOW = """## §CURRENT_STATUS
- bounded slice landed [as-of: 2026-10-16; proof: deployed]

## §NEXT_UNITS
- next queued [as-of: 2026-10-16; proof: claimed]

## §ACTIVE_TRAPS
- none live [as-of: 2026-10-16; proof: measured-live]

## §POINTERS
- door-a.md
"""

GOOD_RESPONSE = f"""=== NOW.MD ===
{GOOD_NOW}
=== OPS ===
[]
=== CLAIMS ===
[
  {{"claim_id": "c1", "text": "bounded slice landed", "as_of": "2026-10-16", "proof": "deployed", "source_refs": ["seq#1/door-a.md"]}},
  {{"claim_id": "c2", "text": "next queued", "as_of": "2026-10-16", "proof": "claimed", "source_refs": ["seq#1/door-a.md"]}},
  {{"claim_id": "c3", "text": "none live", "as_of": "2026-10-16", "proof": "measured-live", "source_refs": ["seq#1/door-a.md"]}}
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


def test_fence_is_dependency_scoped(module, failures):
    """Gap-1 falsifier: sustained writes during composition must not refuse
    the commit forever — only a write ON a composed door does."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-bounded"
        journal_door(module, root, ns, "door-a.md", "# a\n\nalpha\n")
        staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        if staged is None:
            failures.append("fence: prepare staged nothing")
            return
        # composer writes its response; MEANWHILE the fleet writes elsewhere
        pending = module.get_dream_dir(root, ns) / "pending"
        (pending / "RESPONSE.md").write_text(GOOD_RESPONSE, encoding="utf-8")
        (pending / "CRITIC.md").write_text(GOOD_CRITIC, encoding="utf-8")
        journal_door(module, root, ns, "door-z.md", "# z\n\nunrelated concurrent write\n")
        try:
            module.cmd_dream(Args(root=str(root), ns=ns, apply=True, dry_run=True))
        except SystemExit as error:
            failures.append(f"fence: an UNRELATED concurrent write refused the apply: {error}")
        # ...but a write to a door the batch covered is a real lost-update
        journal_door(module, root, ns, "door-a.md", "# a\n\nCONCURRENT EDIT\n")
        try:
            module.cmd_dream(Args(root=str(root), ns=ns, apply=True, dry_run=True))
            failures.append("fence: a concurrent write ON a composed door did not refuse")
        except SystemExit as error:
            if "composed door" not in str(error):
                failures.append(f"fence: wrong scoped-refusal message: {error}")


def test_overdue_service_rotation(module, failures):
    """Gap-1 falsifier: a small namespace whose pending work blew the
    progress deadline is served EVENTUALLY — it jumps a churny big one —
    and blanket deferrals never reset its age."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        old_ts = time.time() - (module.DREAM_PROGRESS_DEADLINE_S + 4000)

        def seed_ns(name, watermark_seq, delta_ts):
            d = root / "dream" / name
            d.mkdir(parents=True, exist_ok=True)
            (d / "watermark.json").write_text(json.dumps(
                {"ns": name, "last_seq": watermark_seq, "last_dream_ts": None, "dreams": []}))
            journal = root / "journal.jsonl"
            journal.parent.mkdir(parents=True, exist_ok=True)
            with journal.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(
                    {"seq": watermark_seq + 1, "ts": delta_ts, "ns": name,
                     "file": "d.md", "origin": "host-a", "summary": "write"}) + "\n")

        # ns-hot: high score (many fresh deltas). ns-starved: 1 very old delta.
        seed_ns("ns-hot", 200, time.time() - 4000)
        seed_ns("ns-starved", 300, old_ts)
        for seq in range(202, 208):
            with (root / "journal.jsonl").open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(
                    {"seq": seq, "ts": time.time() - 4000, "ns": "ns-hot",
                     "file": "d.md", "origin": "host-a", "summary": "write"}) + "\n")

        captured = []
        original_receipt, original_run = module._dream_receipt, module.subprocess.run

        def fake_receipt(r, outcome, reason="", ns=""):
            captured.append((outcome, ns))
            return original_receipt(r, outcome, reason, ns)

        def fake_run(*a, **kw):
            class R:
                returncode = 1
                stdout = ""
                stderr = "stubbed"
            return R()

        module._dream_receipt, module.subprocess.run = fake_receipt, fake_run

        class A:
            model = "gpt-6.1-sol"
            effort = "high"

        try:
            module._dream_auto(root, "", A())
            picked = next((ns for outcome, ns in captured if outcome == "failed"), None)
            if picked != "ns-starved":
                failures.append(f"rotation: the overdue namespace must jump the queue, picked {picked}")
            # deferral must not reset the age: the second tick serves it again
            captured.clear()
            (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
            module._dream_auto(root, "", A())
            picked = next((ns for outcome, ns in captured if outcome == "failed"), None)
            if picked != "ns-starved":
                failures.append(f"rotation: the overdue ns lost its place after a deferral, picked {picked}")
        finally:
            module._dream_receipt, module.subprocess.run = original_receipt, original_run


def test_bounded_retrieval_keeps_rare_facts(module, failures):
    """Gap-2 falsifier: repeated compaction over a large corpus, then a query
    about a rare early fact — correct retrieval with evidence resolution."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-bounded"
        journal_door(module, root, ns, "door-a.md", "# a\n\nalpha\n")
        claims = {}
        for i in range(60):
            claims[f"c{i:03d}"] = {
                "text": f"bulk claim number {i} about topic {i % 7}",
                "as_of": "2026-10-01",
                "proof": "claimed",
                "last_seen": "2026-10-01",
                "source_refs": [f"seq#{i}/door-old.md"],
            }
        claims["c-rare"] = {
            "text": "the rare early fact: heredoc backticks get shell-eaten on oc",
            "as_of": "2026-09-08",
            "proof": "measured-live",
            # early EVIDENCE, but seen recently — a compaction window that
            # ranks by recency must still leave it retrievable
            "last_seen": "2026-10-05",
            "source_refs": ["seq#12/door-ancient.md"],
        }
        module.save_claim_state(root, ns, {"ns": ns, "claims": claims})
        saved_budget = module.DREAM_RETAINED_CLAIMS_BUDGET_BYTES
        module.DREAM_RETAINED_CLAIMS_BUDGET_BYTES = 900  # force heavy compaction
        try:
            staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
            brief_text = staged[0].read_text(encoding="utf-8")
            manifest = staged[1]
            if manifest["retained_omitted"] <= 0:
                failures.append("bounded: compaction omitted nothing (budget not binding)")
            if "rare early fact" in brief_text:
                failures.append("bounded: the brief carried the whole store despite the budget")
            if "c-rare" in manifest["retained_selected"]:
                failures.append("bounded: the recency-ranked window picked the rare claim first")
        finally:
            module.DREAM_RETAINED_CLAIMS_BUDGET_BYTES = saved_budget
        # the rare early fact is one evidence-linked query away
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            module.cmd_claims(Args(subcommand="claims", root=str(root), ns=ns,
                                   query="rare early fact", id=None, status="all", limit=20))
        rows = json.loads(buf.getvalue())
        if len(rows) != 1 or rows[0]["id"] != "c-rare" or not rows[0]["source_refs"]:
            failures.append(f"retrieval: rare-fact query failed: {rows}")
        # rotation: the next dream serves a DIFFERENT slice of the tail
        module.DREAM_RETAINED_CLAIMS_BUDGET_BYTES = 900
        try:
            journal_door(module, root, ns, "door-b.md", "# b\n\nbeta\n")
            staged2 = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
            first_ids = set(staged[1]["retained_selected"])
            second_ids = set(staged2[1]["retained_selected"])
            if not (second_ids - first_ids):
                failures.append("rotation: the second compaction served the identical slice")
        finally:
            module.DREAM_RETAINED_CLAIMS_BUDGET_BYTES = saved_budget


def test_restatement_never_becomes_corroboration(module, failures):
    """Gap-2 ruling: same text + no new sources = restatement (counted, not
    corroboration); genuinely new refs = dated confirmation; the original
    evidence date stays sticky through both."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-bounded"
        state = module.merge_claims(
            {"ns": ns, "claims": {}},
            [{"claim_id": "c1", "text": "the gate refuses stale applies", "as_of": "2026-10-01",
              "proof": "suite", "source_refs": ["seq#1/door.md"]}],
            "2026-10-01",
        )
        # restated a week later by a different dream, same single source
        state = module.merge_claims(
            state,
            [{"claim_id": "c1", "text": "the gate refuses stale applies", "as_of": "2026-10-08",
              "proof": "suite", "source_refs": ["seq#1/door.md"]}],
            "2026-10-08",
        )
        c1 = state["claims"]["c1"]
        if c1.get("restatements") != 1:
            failures.append(f"restatement: expected restatements=1, got {c1.get('restatements')}")
        if c1["as_of"] != "2026-10-01":
            failures.append(f"sticky: a restatement re-dated the evidence: {c1['as_of']}")
        if c1["source_refs"] != ["seq#1/door.md"]:
            failures.append(f"lineage: a restatement multiplied sources: {c1['source_refs']}")
        # now genuinely NEW evidence confirms the claim
        state = module.merge_claims(
            state,
            [{"claim_id": "c1", "text": "the gate refuses stale applies", "as_of": "2026-10-16",
              "proof": "measured-live", "source_refs": ["seq#99/fresh.md"]}],
            "2026-10-16",
        )
        c1 = state["claims"]["c1"]
        confirmed = c1.get("confirmed") or []
        if not confirmed or confirmed[-1]["as_of"] != "2026-10-16" or "seq#99/fresh.md" not in confirmed[-1]["refs"]:
            failures.append(f"confirmation: new evidence not recorded as dated confirmation: {confirmed}")
        if c1["as_of"] != "2026-10-01" or c1["source_refs"][0] != "seq#1/door.md":
            failures.append("confirmation: the original evidence lineage was demoted")


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_fence_is_dependency_scoped(module, failures)
    test_overdue_service_rotation(module, failures)
    test_bounded_retrieval_keeps_rare_facts(module, failures)
    test_restatement_never_becomes_corroboration(module, failures)
    if failures:
        print(f"dream-bounded tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("dream-bounded tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
