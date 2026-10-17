#!/usr/bin/env python3
"""[moe routing + task pickup 2026-10-17, T-002] the MoE dreamer locks.

    python3 tests/test_dream_moe.py [path-to-ygg-memory.py]

Pins the 2026-10-17 charter amendments (owner):
(1) MoE COMPRESSION ROUTING — the composer ROUTES knowledge to its expert
    treatment (deterministic→code, system1→jev, synthesis→composer-instruction,
    legal→legal-flash) instead of grinding it into NOW.md prose; routed items
    persist as a bounded per-ns journal the Jev/work-registry consumers read.
(2) CROSS-LAYER PATTERNS — Yang seeds minted from sweep-wide observations,
    each with a falsifier and a 7d untested expiry, stored FLEET-WIDE.
(3) TASK PICKUP — after a clean dream tick the dreamer takes the top
    tokenomics-ranked OPEN ledger task within its routing rule (codex =
    moderate-to-small ONLY; GLM = 1 big + 2 moderate/day), claims it on the
    ledger door publish-safely, and emits the work brief. Cross-agent
    coordination IS the ledger: only open tasks are taken, only by a
    publish that lands.
"""

import datetime
import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GOOD_NOW = """## §CURRENT_STATUS
- MoE routing landed and deployed [as-of: 2026-10-17; proof: deployed]

## §NEXT_UNITS
- Jev offload sweep next [as-of: 2026-10-17; proof: claimed]

## §ACTIVE_TRAPS
- Heredocs get shell-eaten [as-of: 2026-10-17; proof: measured-live]

## §POINTERS
- campaign-memory-work.md — the campaign door
"""

GOOD_CLAIMS = """[
  {"claim_id": "c1", "text": "MoE routing landed and deployed", "as_of": "2026-10-17", "proof": "deployed", "source_refs": ["seq#1/door-a.md"]},
  {"claim_id": "c2", "text": "Jev offload sweep next", "as_of": "2026-10-17", "proof": "claimed", "source_refs": ["seq#1/door-a.md"]},
  {"claim_id": "c3", "text": "Heredocs get shell-eaten", "as_of": "2026-10-17", "proof": "measured-live", "source_refs": ["seq#1/door-a.md"]}
]"""

GOOD_CRITIC = """=== VERDICTS ===
[
  {"claim_id": "c1", "verdict": "upheld", "counter_example": null},
  {"claim_id": "c2", "verdict": "upheld", "counter_example": null},
  {"claim_id": "c3", "verdict": "upheld", "counter_example": null}
]
"""

GOOD_ROUTING = """[
  {"item": "door-a repeats the same tag decision", "class": "system1", "route": "jev", "why": "repeated semantic classification", "source_refs": ["seq#1/door-a.md"]},
  {"item": "epoch derivable from the journal seq", "class": "deterministic", "route": "code", "why": "mechanically derivable", "source_refs": ["seq#1/door-a.md"]}
]"""

GOOD_PATTERNS = """[
  {"pattern": "the same defect door pattern recurs across memory and boards", "layers": ["memory", "boards"], "hypothesis": "one adapter defect class spans both layers", "falsifier": "an audit of both layers finds disjoint defect classes", "source_refs": ["seq#1/door-a.md"]}
]"""

BASE_RESPONSE = f"""=== NOW.MD ===
{GOOD_NOW}
=== OPS ===
[]
=== CLAIMS ===
{GOOD_CLAIMS}
=== OWNER-NOW ===
null
=== ROUTING ===
{GOOD_ROUTING}
=== PATTERNS ===
{GOOD_PATTERNS}
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
        self.model = "gpt-6.1-sol"
        self.effort = "high"
        self.codex_bin = None
        self.__dict__.update(kw)


def journal_door(module, root, ns, filename, content, harness="zcode", origin=None, action="create"):
    ns_dir = module.get_namespace_dir(root, ns)
    dest = ns_dir / filename
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(content, encoding="utf-8")
    return module.append_journal_entry(
        root, ns, filename, "door", action, harness, f"test {filename}", origin=origin
    )


def build_store(module, home: Path) -> Path:
    root = home / "memory"
    root.mkdir()
    journal_door(module, root, "-t-dream", "door-a.md", "# door a\n\nalpha content\n")
    journal_door(module, root, "-t-dream", "NOW.md", "previous rollup\n", origin="dreamer")
    return root


SYNTHETIC_LEDGER = """## §LAW

1. test law

## §QUEUE

### [T-101] small codex task
- size: small ; value: 3 ; est: x
- route: codex (small) ; deps: none
- status: open

### [T-102] big glm task
- size: big ; value: 5 ; est: x
- route: glm-idle (big slot) ; deps: none
- status: open

### [T-103] moderate glm task
- size: moderate ; value: 4 ; est: x
- route: glm-idle (moderate slot) ; deps: none
- status: open

### [T-104] moderate any task
- size: moderate ; value: 2 ; est: x
- route: any ; deps: none
- status: open

### [T-105] held task
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: waiting on something
- status: open

### [T-106] gated task
- size: small ; value: 4 ; est: x
- route: OWNER-GATED ; deps: none
- status: open (gated)

## §BOOK — one-liners of finished work
"""


def main() -> int:
    default = Path(__file__).resolve().parent.parent / "ygg-memory.py"
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else default)
    failures = []

    def check(condition, message):
        if condition:
            print(f"  ok: {message}")
        else:
            failures.append(message)
            print(f"  FAIL: {message}")

    env_backup = {
        k: os.environ.get(k)
        for k in ("YGG_DREAM_TASK_PICKUP", "YGG_DREAM_TASK_ROUTE", "YGG_DREAM_PICKUP_CAPS")
    }

    try:
        print("[1] the contract carries the MoE blocks; parse tolerates their absence")
        if "=== ROUTING ===" not in module.DREAM_CONTRACT or "=== PATTERNS ===" not in module.DREAM_CONTRACT:
            failures.append("contract: DREAM_CONTRACT missing the ROUTING/PATTERNS blocks")
        else:
            print("  ok: contract carries === ROUTING === and === PATTERNS ===")
        parsed_old = module.parse_dream_response(BASE_RESPONSE.split("=== ROUTING ===")[0])
        if parsed_old.get("routing") != [] or parsed_old.get("patterns") != []:
            failures.append("parse: absence of MoE blocks must parse as empty lists")
        else:
            print("  ok: older-format response parses with routing=[] patterns=[]")
        parsed = module.parse_dream_response(BASE_RESPONSE)
        if len(parsed["routing"]) != 2 or len(parsed["patterns"]) != 1:
            failures.append("parse: MoE blocks not parsed from the full response")
        else:
            print("  ok: ROUTING (2) + PATTERNS (1) parsed")
        errors = module.validate_dream_response(parsed, module.parse_critic_verdicts(GOOD_CRITIC), Path(tempfile.mkdtemp()))
        if errors:
            failures.append(f"validate: good MoE response refused: {errors}")
        else:
            print("  ok: good MoE response validates clean")

        print("[2] validator refuses malformed routing/patterns")

        def expect_refusal(label, bad_text):
            with tempfile.TemporaryDirectory() as tmp:
                root = build_store(module, Path(tmp))
                module.prepare_dream(root, "-t-dream", "synthesis", [], 10 ** 9)
                bad = BASE_RESPONSE
                if isinstance(bad_text, tuple):
                    old, new = bad_text
                    bad = bad.replace(old, new, 1)
                pending = module.get_dream_dir(root, "-t-dream") / "pending"
                (pending / "RESPONSE.md").write_text(bad, encoding="utf-8")
                (pending / "CRITIC.md").write_text(GOOD_CRITIC, encoding="utf-8")
                try:
                    module.cmd_dream(Args(apply=True, ns="-t-dream", root=str(root)))
                    failures.append(f"refusal {label}: apply did not refuse")
                except SystemExit:
                    print(f"  ok: {label} refused")
                except Exception as err:
                    failures.append(f"refusal {label}: wrong error {err!r}")

        expect_refusal("unknown routing class", ('"class": "system1"', '"class": "vibes"'))
        expect_refusal("route/class mismatch", ('"class": "system1", "route": "jev"', '"class": "system1", "route": "code"'))
        expect_refusal("pattern without falsifier", ('"falsifier": "an audit', '"falsifier": null'))
        expect_refusal("pattern with one layer", ('"layers": ["memory", "boards"]', '"layers": ["memory"]'))
        expect_refusal("routing without source_refs", ('"why": "repeated semantic classification", "source_refs": ["seq#1/door-a.md"]', '"why": "repeated semantic classification", "source_refs": []'))

        print("[3] apply persists the routing + pattern journals")
        with tempfile.TemporaryDirectory() as tmp:
            root = build_store(module, Path(tmp))
            module.prepare_dream(root, "-t-dream", "synthesis", [], 10 ** 9)
            pending = module.get_dream_dir(root, "-t-dream") / "pending"
            (pending / "RESPONSE.md").write_text(BASE_RESPONSE, encoding="utf-8")
            (pending / "CRITIC.md").write_text(GOOD_CRITIC, encoding="utf-8")
            module.cmd_dream(Args(apply=True, ns="-t-dream", root=str(root)))
            routing_path = module.get_dream_dir(root, "-t-dream", create=False) / "routing.jsonl"
            if not routing_path.is_file():
                failures.append("apply: dream/<ns>/routing.jsonl not written")
            else:
                rows = [json.loads(l) for l in routing_path.read_text().splitlines() if l.strip()]
                if len(rows) != 2 or rows[0].get("ns") != "-t-dream" or rows[0].get("route") != "jev":
                    failures.append(f"apply: routing.jsonl rows wrong: {rows}")
                elif not rows[0].get("journal_seq") or not rows[0].get("captured_seq"):
                    failures.append("apply: routing rows carry no dream provenance")
                else:
                    print("  ok: routing.jsonl written with ns + dream provenance")
            patterns_path = root / "dream" / "patterns.jsonl"
            if not patterns_path.is_file():
                failures.append("apply: fleet-wide dream/patterns.jsonl not written")
            else:
                rows = [json.loads(l) for l in patterns_path.read_text().splitlines() if l.strip()]
                row = rows[0] if rows else {}
                if row.get("minted_by_ns") != "-t-dream" or not row.get("falsifier"):
                    failures.append(f"apply: patterns.jsonl rows wrong: {row}")
                elif row.get("status") != "open" or not row.get("opened_at") or not row.get("expires_at"):
                    failures.append(f"apply: pattern row missing expiry stamps: {row}")
                else:
                    opened = datetime.date.fromisoformat(row["opened_at"])
                    expires = datetime.date.fromisoformat(row["expires_at"])
                    if (expires - opened).days != module.DREAM_PATTERN_TTL_DAYS:
                        failures.append(f"apply: pattern expiry != +{module.DREAM_PATTERN_TTL_DAYS}d")
                    else:
                        print("  ok: fleet-wide patterns.jsonl with falsifier + 7d expiry")
            wm = module.load_dream_watermark(root, "-t-dream")
            record = (wm.get("dreams") or [{}])[-1]
            if record.get("routing") != 2 or record.get("patterns") != 1:
                failures.append(f"apply: dream record missing MoE counts: {record}")
            else:
                print("  ok: dream record carries routing/patterns counts")

        print("[4] the ledger claim transform")
        claimed = module.ledger_claim_task(SYNTHETIC_LEDGER, "T-102", "glm", "2026-10-17", "auto-pickup test")
        if "- status: claimed(glm, 2026-10-17) — auto-pickup test" not in claimed:
            failures.append("claim: open task not transformed to claimed")
        elif "### [T-103]" not in claimed or "size: big" not in claimed.split("### [T-102]")[1]:
            failures.append("claim: other tasks damaged by the transform")
        else:
            print("  ok: open task claimed; siblings untouched")
        for tid, why in (("T-102", "already claimed"), ("T-999", "missing")):
            try:
                module.ledger_claim_task(claimed if tid == "T-102" else SYNTHETIC_LEDGER, tid, "glm", "2026-10-17", "x")
                failures.append(f"claim: {tid} ({why}) did not refuse")
            except ValueError:
                print(f"  ok: {why} task refused")

        print("[5] parse + tokenomics sort")
        tasks = module.parse_ledger_tasks(SYNTHETIC_LEDGER)
        if len(tasks) != 6:
            failures.append(f"parse: expected 6 tasks, got {len(tasks)}")
        sizes = {t["id"]: module._ledger_task_size(t) for t in tasks}
        if sizes["T-102"] != "big" or sizes["T-103"] != "moderate" or sizes["T-101"] != "small":
            failures.append(f"size: wrong classification {sizes}")
        elif module._ledger_task_score(tasks[1]) != 5 / 4:
            failures.append(f"score: big weight wrong {module._ledger_task_score(tasks[1])}")
        else:
            print("  ok: sizes + value/weight scoring correct")

        print("[6] the pickup end-to-end in a sandbox (glm class)")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "memory"
            (root / "namespaces" / "_global").mkdir(parents=True)
            (root / "namespaces" / "_global" / "fleet-task-ledger.md").write_text(SYNTHETIC_LEDGER, encoding="utf-8")
            os.environ["YGG_DREAM_TASK_ROUTE"] = "glm"
            os.environ.pop("YGG_DREAM_TASK_PICKUP", None)
            os.environ.pop("YGG_DREAM_PICKUP_CAPS", None)

            first = module._dream_task_pickup(root)
            if not first or "TASK PICKUP" not in first or "T-103" not in first:
                failures.append(f"pickup: first glm pickup wrong: {first!r}")
            elif "LAST STEP" not in first:
                failures.append("pickup: brief missing the mandatory LAST STEP")
            else:
                print(f"  ok: first pickup = top-scoring eligible (T-103, score 2.0)")
            door = root / "namespaces" / "_global" / "fleet-task-ledger.md"
            if "claimed(glm" not in door.read_text():
                failures.append("pickup: ledger door not claimed in the sandbox")
            else:
                print("  ok: ledger door claimed publish-safely (sandbox)")
            state_path = root / "dream" / "ledger-pickups.json"
            state = json.loads(state_path.read_text()) if state_path.is_file() else {}
            if not any(p.get("id") == "T-103" for p in state.get("pickups", [])):
                failures.append(f"pickup: state not recorded: {state}")
            else:
                print("  ok: daily pickup state recorded")

            second = module._dream_task_pickup(root)
            if not second or "T-102" not in second:
                failures.append(f"pickup: second glm pickup wrong (expected the big): {second!r}")
            else:
                print("  ok: second pickup = the big slot task (T-102)")
            third = module._dream_task_pickup(root)
            if not third or "T-104" not in third:
                failures.append(f"pickup: third glm pickup wrong (expected T-104): {third!r}")
            else:
                print("  ok: third pickup = the any-routed moderate (T-104)")
            fourth = module._dream_task_pickup(root)
            if fourth is not None:
                failures.append(f"pickup: glm allowance not exhausted: {fourth!r}")
            else:
                print("  ok: glm daily allowance exhausted → None")

        print("[7] the codex routing rule (moderate-to-small ONLY, never glm-routed)")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "memory"
            (root / "namespaces" / "_global").mkdir(parents=True)
            (root / "namespaces" / "_global" / "fleet-task-ledger.md").write_text(SYNTHETIC_LEDGER, encoding="utf-8")
            os.environ["YGG_DREAM_TASK_ROUTE"] = "codex"
            first = module._dream_task_pickup(root)
            if not first or "T-101" not in first:
                failures.append(f"pickup: codex first pickup wrong: {first!r}")
            else:
                print("  ok: codex takes the codex-routed small (T-101)")
            second = module._dream_task_pickup(root)
            if not second or "T-104" not in second:
                failures.append(f"pickup: codex second pickup wrong (expected the any moderate): {second!r}")
            else:
                print("  ok: codex takes the any-routed moderate (T-104)")
            if "T-102" in (first or "") + (second or ""):
                failures.append("pickup: codex took a glm-routed BIG — the owner law is broken")
            else:
                print("  ok: codex never took a big or a glm-routed task")

        print("[8] the kill switch + non-open refusal")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "memory"
            (root / "namespaces" / "_global").mkdir(parents=True)
            door = root / "namespaces" / "_global" / "fleet-task-ledger.md"
            os.environ["YGG_DREAM_TASK_ROUTE"] = "glm"
            os.environ["YGG_DREAM_TASK_PICKUP"] = "0"
            door.write_text(SYNTHETIC_LEDGER, encoding="utf-8")
            if module._dream_task_pickup(root) is not None:
                failures.append("kill switch: pickup ran despite YGG_DREAM_TASK_PICKUP=0")
            else:
                print("  ok: YGG_DREAM_TASK_PICKUP=0 disables pickup")
            os.environ["YGG_DREAM_TASK_PICKUP"] = "1"
            door.write_text(SYNTHETIC_LEDGER.replace("- status: open\n", "- status: claimed(glm, 2026-10-17) — x\n"), encoding="utf-8")
            if module._dream_task_pickup(root) is not None:
                failures.append("non-open: pickup stole a claimed task")
            else:
                print("  ok: fully-claimed ledger → no pickup")
    finally:
        for key, value in env_backup.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    print()
    if failures:
        print(f"MoE dream locks: {len(failures)} FAILURE(S)")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("MoE dream locks: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
