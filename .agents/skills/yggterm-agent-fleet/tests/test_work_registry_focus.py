#!/usr/bin/env python3
"""[slice 2] work-class registry + focus docs: authority enforced in code,
revoke-before-commit, deduped owner forwards, focus that reorders but never
grants, and focus-doc expiry/max semantics.

    python3 tests/test_work_registry_focus.py [path-to-ygg-memory.py]

⛔ THE HOLES THIS PINS (sol round-4 Q2+Q3, slice-2 falsifier):
"revoke authority pre-commit, or focus forbidden work — any resulting write
fails." The registry gate lives INSIDE the serialized budget authority (a
revoke is visible to every host's next check), settle re-checks authority
before committing (a class disabled between admit and settle voids the
work), refusals forward the narrow owner remainder exactly ONCE, and the
focus doc reorders the dreamer's ranking — it can never resurrect a blocked
namespace or grant a class the registry does not.
"""

import datetime
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"

POLICY = json.dumps(
    {
        "totals": {"input": 100_000, "generated": 10_000, "cash_cents": 50},
        "buckets": {name: {**spec} for name, spec in {
            "work:callgraph": {"input": 12_000, "generated": 1_200, "cash_cents": 6, "borrowable": True},
            "work:dossiergraph": {"input": 10_000, "generated": 1_000, "cash_cents": 5, "borrowable": True},
            "work:infra": {"input": 7_000, "generated": 700, "cash_cents": 4, "borrowable": True},
        }.items()},
    }
)


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def cli_env() -> dict:
    env = dict(os.environ)
    env["YGG_BUDGET_AUTHORITY"] = "local"
    env["YGG_BUDGET_POLICY_JSON"] = POLICY
    return env


def run_cli(module, root: Path, *verb_args: str, expect_code: int | None = None) -> dict:
    run_ = subprocess.run(
        [sys.executable, str(MODULE_PATH), "--root", str(root), *verb_args],
        capture_output=True, text=True, env=cli_env(), timeout=60,
    )
    if expect_code is not None and run_.returncode != expect_code:
        raise AssertionError(
            f"{' '.join(verb_args)}: exit {run_.returncode} != {expect_code}\n"
            f"stdout: {run_.stdout}\nstderr: {run_.stderr}"
        )
    try:
        return json.loads(run_.stdout)
    except json.JSONDecodeError:
        raise AssertionError(f"non-JSON output: {run_.stdout!r} / {run_.stderr!r}")


def forward_rows(module, root: Path) -> list[dict]:
    path = module.work_forward_path(root)
    if not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def write_focus_doc(module, root: Path, body: str) -> None:
    ns_dir = module.get_namespace_dir(root, "_global")
    ns_dir.mkdir(parents=True, exist_ok=True)
    (ns_dir / "dream-focus.md").write_text(body, encoding="utf-8")


FOCUS_DOC_TEMPLATE = """---
name: dream-focus
description: test doc
metadata:
  type: now
---

## Active focus (newest intent wins; overlapping boosts take the max)

{active}

## Standing constraints (always read)

{standing}
"""


def test_registry_seeds_and_clamps(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        registry = module.load_work_registry(root)
        if sorted(registry["classes"].keys()) != sorted(
            ["callgraph", "dossiergraph", "infra-updates", "legacy-notes-conversion", "learning-linkages", "consult-docs"]
        ):
            failures.append(f"seeds: expected the six owner classes, got {sorted(registry['classes'].keys())}")
        if not all(cls["enabled"] for cls in registry["classes"].values()):
            failures.append("seeds: classes must default to enabled")
        # a persisted RAISE is clamped back to the seed; a lowering holds.
        reg_path = module.work_registry_path(root)
        reg_path.parent.mkdir(parents=True, exist_ok=True)
        reg_path.write_text(json.dumps(
            {"classes": {"callgraph": {"enabled": False, "per_item": {"input": 99_000, "generated": 9_000, "cash_cents": 90}}}}
        ))
        registry = module.load_work_registry(root)
        callgraph = registry["classes"]["callgraph"]
        if callgraph["enabled"]:
            failures.append("clamp: stored disable was ignored")
        if callgraph["per_item"]["input"] != 8_000:
            failures.append(f"clamp: raised per-item ceiling leaked through: {callgraph['per_item']}")
        if registry["classes"]["dossiergraph"]["per_item"]["input"] != 12_000:
            failures.append("clamp: untouched class drifted from seed")


def test_class_admission_uses_class_bucket(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        admit = run_cli(module, root, "dream-budget", "--admit", "--class", "callgraph",
                        "--input", "5000", "--generated", "400", "--cash-cents", "2", expect_code=0)
        if not admit.get("admitted") or admit.get("bucket") != "work:callgraph":
            failures.append(f"class-bucket: admission did not route to the class bucket: {admit}")
        ledger = json.loads((root / "dream" / "budget" / "ledger.json").read_text())
        res = ledger["reservations"].get(admit["id"], {})
        if res.get("class_id") != "callgraph":
            failures.append(f"class-bucket: reservation lost its class identity: {res}")


def test_revoke_before_commit_refuses_settle(module, failures):
    """THE SLICE FALSIFIER: authority revoked between admit and commit —
    the settle must refuse, book nothing, and forward the remainder once."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        admit = run_cli(module, root, "dream-budget", "--admit", "--class", "dossiergraph",
                        "--input", "6000", "--generated", "500", "--cash-cents", "3", expect_code=0)
        run_cli(module, root, "work-registry", "--disable", "--class-id", "dossiergraph")
        settle = run_cli(module, root, "dream-budget", "--settle", admit["id"],
                         "--input", "6000", "--generated", "500", "--cash-cents", "3", expect_code=3)
        if settle.get("reason") != "class-revoked":
            failures.append(f"revoke: settle did not refuse on the revoked class: {settle}")
        ledger = json.loads((root / "dream" / "budget" / "ledger.json").read_text())
        spent = ledger["spent"].get(module.budget_today(), {}).get("_total", {})
        if any(spent.get(dim, 0) > 0 for dim in ("input", "generated", "cash_cents")):
            failures.append(f"revoke: refused work still booked spend: {spent}")
        kinds = [row["kind"] for row in forward_rows(module, root)]
        if "settle-refused-class-revoked" not in kinds:
            failures.append(f"revoke: no owner remainder forwarded: {kinds}")
        # re-admission while disabled must refuse + forward, not sneak through.
        again = run_cli(module, root, "dream-budget", "--admit", "--class", "dossiergraph",
                        "--input", "1000", "--generated", "100", "--cash-cents", "1")
        if again.get("admitted"):
            failures.append("revoke: disabled class admitted fresh work")


def test_refusals_forward_exactly_once(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for _ in range(3):
            run_cli(module, root, "dream-budget", "--admit", "--class", "rocket-fleet",
                    "--input", "100", "--generated", "10")
        rows = forward_rows(module, root)
        if len(rows) != 1:
            failures.append(f"dedup: repeated refusals must forward ONCE, got {len(rows)}: {rows}")
        first = rows[0] if rows else {}
        for field in ("decision_needed", "evidence", "anticipated_default", "resume"):
            if field not in first:
                failures.append(f"dedup: forward missing the narrow-remainder field '{field}': {first}")
        over = run_cli(module, root, "dream-budget", "--admit", "--class", "infra-updates",
                       "--input", "999_999", "--generated", "10")
        if over.get("admitted") or not over.get("reason", "").startswith("over-item-ceiling"):
            failures.append(f"ceiling: over-ceiling estimate admitted: {over}")
        kinds = [row["kind"] for row in forward_rows(module, root)]
        if kinds.count("over-item-ceiling") != 1:
            failures.append(f"ceiling: over-ceiling forward wrong: {kinds}")


def test_focus_parsing_expiry_and_max(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        today = module.budget_today()
        today_d = datetime.date.fromisoformat(today)
        yesterday = (today_d - datetime.timedelta(days=1)).isoformat()
        soon = (today_d + datetime.timedelta(days=2)).isoformat()
        write_focus_doc(module, root, FOCUS_DOC_TEMPLATE.format(
            active=(
                f"- **Focus: stale work** scope: ns-a [as-of: {yesterday}; expires: {yesterday}]\n"
                f"- **Focus: fresh explicit** scope: ns-a [as-of: {today}; expires: {soon}]\n"
                f"- **Focus: default expiry** scope: ns-b [as-of: {today}]\n"
                f"- **Focus: exclusive burst** scope: ns-c exclusive --for 24h [as-of: {today}]\n"
                f"- **Focus: also ns-b boost** scope: ns-b [as-of: {today}]"
            ),
            standing=(
                f"- The 10-day journal backlog (-home-pi-gh-yggterm) consolidates first "
                f"whenever no higher focus is active. [as-of: {today}]"
            ),
        ))
        parsed = module.parse_focus_doc((module.get_namespace_dir(root, "_global") / "dream-focus.md").read_text(), today=today)
        texts = [d["text"] for d in parsed["directives"]]
        if any("stale work" in t for t in texts):
            failures.append("expiry: an expired directive survived pruning")
        if len(parsed["directives"]) != 4:
            failures.append(f"expiry: expected 4 active directives, got {len(parsed['directives'])}: {texts}")
        ns_a = module.focus_view(root, "ns-a", today=today)
        ns_b = module.focus_view(root, "ns-b", today=today)
        ns_c = module.focus_view(root, "ns-c", today=today)
        if ns_a["weight"] != 4:
            failures.append(f"focus: ns-a expected boost weight 4, got {ns_a['weight']}")
        if ns_b["weight"] != 4:
            failures.append(f"max: overlapping boosts multiplied or dropped: got {ns_b['weight']} (want 4, max not 16/1)")
        if not ns_c.get("exclusive"):
            failures.append(f"exclusive: the exclusive directive did not resolve: {ns_c}")
        backlog_ns = module.focus_view(root, "-home-pi-gh-yggterm", today=today)
        if backlog_ns["weight"] != 2:
            failures.append(f"standing: backlog namespace expected standing weight 2, got {backlog_ns['weight']}")
        stray = module.focus_view(root, "ns-unrelated", today=today)
        if stray["weight"] != 1 or stray["exclusive"]:
            failures.append(f"unrelated: a stray namespace must be unfocused: {stray}")
        if not stray.get("unresolved") and ns_a["matched"]:
            pass  # resolved scopes are the documented mechanism


def test_focus_cannot_grant_authority(module, failures):
    """Focus forbidden work: a boost naming a disabled/unknown class changes
    nothing — the registry gate sits on a plane the doc never touches."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        write_focus_doc(module, root, FOCUS_DOC_TEMPLATE.format(
            active="- **Focus: infra updates now** scope: work:infra-updates exclusive [as-of: 2026-10-16]",
            standing="",
        ))
        run_cli(module, root, "work-registry", "--disable", "--class-id", "infra-updates")
        refused = run_cli(module, root, "dream-budget", "--admit", "--class", "infra-updates",
                          "--input", "500", "--generated", "50", "--cash-cents", "1")
        if refused.get("admitted"):
            failures.append(f"authority: focused-but-disabled class admitted: {refused}")
        unknown = run_cli(module, root, "dream-budget", "--admit", "--class", "moon-shot",
                          "--input", "500", "--generated", "50", "--cash-cents", "1")
        if unknown.get("admitted"):
            failures.append(f"authority: focused-but-undeclared class admitted: {unknown}")


def test_focus_ranking_in_auto(module, failures):
    """Focus reorders the auto-dreamer's ranking; exclusive mode excludes
    non-matching candidates; eligibility filters are NEVER overridden."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)

        def seed_ns(name: str, seq: int, age_s: float) -> None:
            dream_dir = root / "dream" / name
            dream_dir.mkdir(parents=True, exist_ok=True)
            (dream_dir / "watermark.json").write_text(json.dumps(
                {"ns": name, "last_seq": seq - 1, "last_dream_ts": None, "dreams": []}))
            # a fresh watermark file must exist; deltas come from the journal
            write_journal(root, [{"seq": seq, "ts": time.time() - age_s, "ns": name,
                                  "file": "d.md", "origin": "host-a", "summary": "write"}])

        def write_journal(root: Path, records: list) -> None:
            journal = root / "journal.jsonl"
            journal.parent.mkdir(parents=True, exist_ok=True)
            with journal.open("a", encoding="utf-8") as handle:
                for record in records:
                    handle.write(json.dumps(record) + "\n")

        captured = []
        original_receipt, original_run = module._dream_receipt, module.subprocess.run

        def fake_receipt(r, outcome, reason="", ns=""):
            captured.append((outcome, ns))
            return original_receipt(r, outcome, reason, ns)

        def fake_run(*a, **kw):
            class R:
                returncode = 1
                stdout = ""
                stderr = "stubbed composer failure"
            return R()

        module._dream_receipt, module.subprocess.run = fake_receipt, fake_run

        class Args:
            model = "gpt-6.1-sol"
            effort = "high"

        try:
            # Equal raw scores (1 pending delta each): the focus doc is the
            # ONLY thing that can decide — ns-small is boosted ×4.
            seed_ns("ns-big", 104, 4000)
            seed_ns("ns-small", 102, 4000)
            write_journal(root, [
                {"seq": s, "ts": time.time() - 4000, "ns": "ns-big", "file": "d.md",
                 "origin": "host-a", "summary": "write"} for s in range(100, 105)
            ])
            write_focus_doc(module, root, FOCUS_DOC_TEMPLATE.format(
                active="- **Focus: the small one** scope: ns-small [as-of: 2026-10-16]",
                standing="",
            ))
            (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
            module._dream_auto(root, "", Args())
            picked = next((ns for outcome, ns in captured if outcome in ("failed", "applied")), None)
            if picked != "ns-small":
                failures.append(f"ranking: the focused ns should outrank the churny one, picked {picked}")
            # Exclusive focus on ns-small excludes ns-big entirely.
            captured.clear()
            write_focus_doc(module, root, FOCUS_DOC_TEMPLATE.format(
                active="- **Focus: exclusive small** scope: ns-small exclusive [as-of: 2026-10-16]",
                standing="",
            ))
            (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
            message = module._dream_auto(root, "", Args())
            picked = next((ns for outcome, ns in captured if outcome in ("failed", "applied")), None)
            if picked != "ns-small":
                failures.append(f"exclusive: the matching ns must still run, picked {picked} ({message})")
            # A blocked (cooldown) namespace is NOT resurrected by its boost.
            captured.clear()
            dream_dir = root / "dream" / "ns-small"
            (dream_dir / "watermark.json").write_text(json.dumps(
                {"ns": "ns-small", "last_seq": 102,
                 "last_dream_ts": datetime.datetime.now(datetime.timezone.utc).isoformat(), "dreams": []}))
            (root / "dream" / "auto-backoff.json").unlink(missing_ok=True)
            message = module._dream_auto(root, "", Args())
            if "ns-small" in message:
                failures.append(f"eligibility: focus resurrected a cooled-down namespace: {message}")
        finally:
            module._dream_receipt, module.subprocess.run = original_receipt, original_run


def test_prepare_manifest_records_focus(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-focus"
        module.validate_namespace(ns)
        ns_dir = module.get_namespace_dir(root, ns)
        ns_dir.mkdir(parents=True, exist_ok=True)
        (ns_dir / "door.md").write_text("# door\n\ncontent\n", encoding="utf-8")
        module.append_journal_entry(root, ns, "door.md", "door", "create", "zcode", "test door")
        write_focus_doc(module, root, FOCUS_DOC_TEMPLATE.format(
            active=f"- **Focus: this campaign** scope: {ns} [as-of: 2026-10-16]", standing=""))
        staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        if staged is None:
            failures.append("manifest: prepare staged nothing")
            return
        _, manifest = staged
        focus = manifest.get("focus", {})
        if focus.get("ns") != ns or focus.get("weight") != 4:
            failures.append(f"manifest: focus view missing or unresolved in the manifest: {focus}")


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_registry_seeds_and_clamps(module, failures)
    test_class_admission_uses_class_bucket(module, failures)
    test_revoke_before_commit_refuses_settle(module, failures)
    test_refusals_forward_exactly_once(module, failures)
    test_focus_parsing_expiry_and_max(module, failures)
    test_focus_cannot_grant_authority(module, failures)
    test_focus_ranking_in_auto(module, failures)
    test_prepare_manifest_records_focus(module, failures)
    if failures:
        print(f"work-registry+focus tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("work-registry+focus tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
