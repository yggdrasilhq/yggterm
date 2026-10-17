#!/usr/bin/env python3
"""[slice 1] fleet budget admission: the race falsifier, midnight safety,
crash-release TTL, bucket borrow rules, and the dream-run budget gate.

    python3 tests/test_dream_budget.py [path-to-ygg-memory.py]

⛔ THE HOLE THIS PINS (sol round-4 Q4/Q5 slice 1 + round-3 gap review):
soft bucket allocations under HARD fleet-wide limits are worth nothing if
two hosts can both squeeze through the last allowance — the named slice
falsifier is "race two requests against the last available allowance; both
admitted = fail". The ONE serialized check-and-reserve point (flock on the
authority host) is the fix; these tests exercise it through separate OS
processes hitting the same ledger file, the same discipline the ssh
transport drives on dev. A hold made at 23:59 stays charged across
midnight (books to its reservation day); an orphaned hold self-releases by
TTL; the verification reserve is never borrowable; and `dream --run` must
refuse to compose on a refused admission — a quota cap is the designed
sleep, not an invitation to spend locally.
"""

import concurrent.futures
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

TINY_POLICY = json.dumps(
    {
        "totals": {"input": 100, "generated": 40, "cash_cents": 20},
        "buckets": {
            "yin": {"input": 60, "generated": 30, "cash_cents": 10, "borrowable": True},
            "reserve:verification": {"input": 20, "generated": 10, "cash_cents": 5, "borrowable": False},
        },
    }
)

GOOD_NOW = """## §CURRENT_STATUS
- budget slice landed [as-of: 2026-10-16; proof: deployed]

## §NEXT_UNITS
- next slice queued [as-of: 2026-10-16; proof: claimed]

## §ACTIVE_TRAPS
- watch the race [as-of: 2026-10-16; proof: measured-live]

## §POINTERS
- campaign door
"""

GOOD_RESPONSE = f"""=== NOW.MD ===
{GOOD_NOW}
=== OPS ===
[]
=== CLAIMS ===
[
  {{"claim_id": "c1", "text": "budget slice landed", "as_of": "2026-10-16", "proof": "deployed", "source_refs": ["seq#1/door-a.md"]}},
  {{"claim_id": "c2", "text": "next slice queued", "as_of": "2026-10-16", "proof": "claimed", "source_refs": ["seq#1/door-a.md"]}},
  {{"claim_id": "c3", "text": "watch the race", "as_of": "2026-10-16", "proof": "measured-live", "source_refs": ["seq#1/door-a.md"]}}
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


def cli_env() -> dict:
    # the capacity-refusal falsifiers are HARD-CAPS-mode contracts (the
    # owner-instantiated device); soft mode is tested separately below
    env = dict(os.environ)
    env["YGG_BUDGET_AUTHORITY"] = "local"
    env["YGG_BUDGET_POLICY_JSON"] = TINY_POLICY
    env["YGG_BUDGET_CAPS"] = "hard"
    return env


def run_cli(module, root: Path, *budget_args: str, expect_code: int | None = None) -> dict:
    run_ = subprocess.run(
        [sys.executable, str(MODULE_PATH), "--root", str(root), "dream-budget", *budget_args],
        capture_output=True, text=True, env=cli_env(), timeout=60,
    )
    if expect_code is not None and run_.returncode != expect_code:
        raise AssertionError(
            f"dream-budget {' '.join(budget_args)}: exit {run_.returncode} != {expect_code}\n"
            f"stdout: {run_.stdout}\nstderr: {run_.stderr}"
        )
    try:
        return json.loads(run_.stdout)
    except json.JSONDecodeError:
        raise AssertionError(f"dream-budget produced non-JSON: {run_.stdout!r} / {run_.stderr!r}")


def journal_one_delta(module, root: Path) -> None:
    ns = "-t-budget"
    module.validate_namespace(ns)
    ns_dir = module.get_namespace_dir(root, ns)
    ns_dir.mkdir(parents=True, exist_ok=True)
    (ns_dir / "door-a.md").write_text("# door a\n\nalpha content\n", encoding="utf-8")
    module.append_journal_entry(root, ns, "door-a.md", "door", "create", "zcode", "test door-a")


def seed_ledger(root: Path, *, spent: dict | None = None, day: str | None = None,
                reservations: dict | None = None) -> None:
    ledger_path = root / "dream" / "budget" / "ledger.json"
    ledger = {"version": 1, "spent": spent or {}, "reservations": reservations or {}}
    if day:
        ledger["day"] = day
    ledger_path.parent.mkdir(parents=True, exist_ok=True)
    ledger_path.write_text(json.dumps(ledger), encoding="utf-8")


def spent_dims(ledger: dict, day: str, bucket: str = "_total") -> dict:
    return {dim: ledger["spent"].get(day, {}).get(bucket, {}).get(dim, 0) for dim in ("input", "generated", "cash_cents")}


def test_race_last_allowance(module, failures):
    """THE SLICE FALSIFIER: race requests against the last allowance through
    separate OS processes — both admitted means the slice failed."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        today = module.budget_today()
        # 12 input-token tokens of headroom; three racers each want 8.
        seed_ledger(root, spent={today: {"_total": {"input": 88, "generated": 0, "cash_cents": 0},
                                        "yin": {"input": 88, "generated": 0, "cash_cents": 0}}})
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
            results = list(
                pool.map(
                    lambda _: run_cli(module, root, "--admit", "--bucket", "yin",
                                      "--input", "8", "--generated", "2", "--cash-cents", "1",
                                      "--purpose", "race"),
                    range(3),
                )
            )
        admitted = [r for r in results if r.get("admitted")]
        refused = [r for r in results if not r.get("admitted")]
        if len(admitted) != 1:
            failures.append(f"race: expected exactly 1 admission against the last allowance, got {len(admitted)}: {results}")
        if len(refused) != 2 or not all("exhausted:input" in r.get("reason", "") for r in refused):
            failures.append(f"race: refusals wrong: {refused}")
        ledger = json.loads((root / "dream" / "budget" / "ledger.json").read_text())
        holds = sum(int(r.get("input", 0)) for r in ledger["reservations"].values() if r.get("state") == "reserved")
        if spent_dims(ledger, today)["input"] + holds > 100:
            failures.append(f"race: overbooked — spent {spent_dims(ledger, today)} + holds {holds} > hard cap 100")


def test_midnight_inflight_books_to_reservation_day(module, failures):
    """A hold made before the day boundary stays charged across midnight and
    settles into ITS day; the new day starts fresh, no carryover, no debt."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        yesterday = "2026-10-15"
        admit = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "30",
                        "--generated", "5", "--cash-cents", "2", "--ttl", "600",
                        expect_code=0)
        # rewind the ledger's clock: the reservation was made "yesterday"
        ledger_path = root / "dream" / "budget" / "ledger.json"
        ledger = json.loads(ledger_path.read_text())
        ledger["day"] = yesterday
        ledger["spent"][yesterday] = ledger["spent"].pop(module.budget_today())
        ledger["reservations"][admit["id"]]["day"] = yesterday
        ledger_path.write_text(json.dumps(ledger))
        settle = run_cli(module, root, "--settle", admit["id"], "--input", "25",
                         "--generated", "4", "--cash-cents", "2", "--actual-source", "measured",
                         expect_code=0)
        if not settle.get("settled") or settle.get("booked_day") != yesterday or not settle.get("late"):
            failures.append(f"midnight: settle did not book to the reservation day: {settle}")
        ledger = json.loads(ledger_path.read_text())
        if ledger["day"] != module.budget_today():
            failures.append(f"midnight: day did not roll: {ledger['day']}")
        if spent_dims(ledger, yesterday)["input"] != 25:
            failures.append(f"midnight: actual not booked to yesterday: {spent_dims(ledger, yesterday)}")
        if spent_dims(ledger, module.budget_today())["input"] != 0:
            failures.append("midnight: new day is not fresh (carryover/debt)")
        if not any(r.get("kind") == "day-roll" for r in ledger["receipts"]):
            failures.append("midnight: no day-roll receipt")


def test_ttl_crash_release(module, failures):
    """An orphaned hold (crashed worker) self-releases by TTL — capacity is
    not lost forever."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        admit = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "95",
                        "--generated", "5", "--cash-cents", "2", "--ttl", "1", expect_code=0)
        again = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "10",
                        "--generated", "2", "--cash-cents", "1")
        if again.get("admitted"):
            failures.append("ttl: a live hold did not block admission")
        time.sleep(1.3)
        status = run_cli(module, root, "--status")
        if admit["id"] not in (status.get("expired_released_now") or []):
            failures.append(f"ttl: expired hold not swept on the next op: {status.get('expired_released_now')}")
        after = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "10",
                        "--generated", "2", "--cash-cents", "1", expect_code=0)
        if not after.get("admitted"):
            failures.append(f"ttl: released capacity not reusable: {after}")


def test_bucket_borrow_and_verification_reserve(module, failures):
    """Unused allocations are borrowable (soft buckets, hard totals); the
    verification reserve is NEVER borrowed — recovery is reserved first."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        today = module.budget_today()
        # yin bucket spent to exactly its cap (60/30/10); hard totals still
        # have 40 input of room → the admission must borrow, not refuse.
        seed_ledger(root, spent={today: {"_total": {"input": 60, "generated": 30, "cash_cents": 10},
                                        "yin": {"input": 60, "generated": 30, "cash_cents": 10}}})
        borrowed = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "10",
                           "--generated", "2", "--cash-cents", "1", expect_code=0)
        if not borrowed.get("admitted") or not borrowed.get("borrowed"):
            failures.append(f"borrow: yin admission past its soft bucket cap should borrow, got {borrowed}")
        # verification reserve at cap → refused even with hard room.
        seed_ledger(root, spent={today: {"_total": {"input": 10, "generated": 2, "cash_cents": 1},
                                        "reserve:verification": {"input": 20, "generated": 10, "cash_cents": 5}}})
        raid = run_cli(module, root, "--admit", "--bucket", "reserve:verification",
                       "--input", "10", "--generated", "2", "--cash-cents", "1")
        if raid.get("admitted"):
            failures.append(f"reserve: verification reserve was raided past its cap: {raid}")
        if "bucket-exhausted" not in raid.get("reason", ""):
            failures.append(f"reserve: wrong refusal reason: {raid}")


def test_refusal_shapes(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        zero = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "0",
                       "--generated", "0", "--cash-cents", "0")
        if zero.get("admitted") or zero.get("reason") != "zero-estimate":
            failures.append(f"shapes: zero estimate must refuse: {zero}")
        unknown = run_cli(module, root, "--admit", "--bucket", "nope", "--input", "5")
        if unknown.get("admitted") or not unknown.get("reason", "").startswith("unknown-bucket"):
            failures.append(f"shapes: unknown bucket must refuse: {unknown}")
        missing = run_cli(module, root, "--settle", "does-not-exist", "--input", "5", expect_code=3)
        if missing.get("settled"):
            failures.append(f"shapes: unknown settle id must refuse: {missing}")


def test_settle_refunds_headroom_conservation(module, failures):
    """Admission holds the ESTIMATE; settle books the ACTUAL and frees the
    difference — spent + holds never exceed a cap through a full lifecycle."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        today = module.budget_today()
        first = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "50",
                        "--generated", "10", "--cash-cents", "3", expect_code=0)
        second = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "51",
                         "--generated", "10", "--cash-cents", "3")
        if second.get("admitted"):
            failures.append("conservation: hold was not counted against admission")
        settled = run_cli(module, root, "--settle", first["id"], "--input", "20",
                          "--generated", "4", "--cash-cents", "1", expect_code=0)
        if settled.get("actual", {}).get("input") != 20:
            failures.append(f"conservation: settle lost actuals: {settled}")
        third = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "50",
                        "--generated", "10", "--cash-cents", "3", expect_code=0)
        if not third.get("admitted"):
            failures.append(f"conservation: headroom not refunded after settle: {third}")
        released = run_cli(module, root, "--release", third["id"], expect_code=0)
        if not released.get("released"):
            failures.append(f"conservation: release failed: {released}")
        fourth = run_cli(module, root, "--admit", "--bucket", "yin", "--input", "50",
                         "--generated", "10", "--cash-cents", "3", expect_code=0)
        if not fourth.get("admitted"):
            failures.append(f"conservation: released capacity not reusable: {fourth}")
        ledger = json.loads((root / "dream" / "budget" / "ledger.json").read_text())
        if spent_dims(ledger, today)["input"] + 50 > 100:
            failures.append("conservation: overbooked ledger")


def test_dream_run_gated_by_budget(module, failures):
    """`dream --run` composes ONLY through an admission: a refused budget is
    the quota-cap sleep (no composer call), a healthy budget settles the
    burned tokens and stamps the reservation into the manifest."""
    # a real brief is ~1.5k input tokens — size the policy for the composer,
    # not for the racer tests (which run in their own subprocess env).
    dream_policy = json.dumps(
        {"totals": {"input": 20_000, "generated": 6_000, "cash_cents": 40},
         "buckets": {"yin": {"input": 20_000, "generated": 6_000, "cash_cents": 40, "borrowable": True}}}
    )
    saved_env = os.environ.get("YGG_BUDGET_POLICY_JSON")
    saved_authority = os.environ.get("YGG_BUDGET_AUTHORITY")
    saved_caps = os.environ.get("YGG_BUDGET_CAPS")
    os.environ["YGG_BUDGET_POLICY_JSON"] = dream_policy
    os.environ["YGG_BUDGET_AUTHORITY"] = "local"
    os.environ["YGG_BUDGET_CAPS"] = "hard"
    os.environ["YGG_BUDGET_CAPS"] = "hard"
    try:
        _dream_run_gated_body(module, failures)
    finally:
        if saved_env is None:
            os.environ.pop("YGG_BUDGET_POLICY_JSON", None)
        else:
            os.environ["YGG_BUDGET_POLICY_JSON"] = saved_env
        if saved_authority is None:
            os.environ.pop("YGG_BUDGET_AUTHORITY", None)
        else:
            os.environ["YGG_BUDGET_AUTHORITY"] = saved_authority
        if saved_caps is None:
            os.environ.pop("YGG_BUDGET_CAPS", None)
        else:
            os.environ["YGG_BUDGET_CAPS"] = saved_caps


def _dream_run_gated_body(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        journal_one_delta(module, root)
        composer_calls = []

        def fake_composer(prompt, effort, codex_bin, model):
            composer_calls.append(prompt)
            return (GOOD_RESPONSE if "COMPOSER OUTPUT" not in prompt
                    else GOOD_CRITIC), "test-stub"

        original_composer = module._composer_exec
        module._composer_exec = fake_composer
        try:
            # (a) refusal path: pre-exhaust the hard input total (the
            # dream-sized policy — a 100-token seed would just admit).
            today = module.budget_today()
            seed_ledger(root, spent={today: {"_total": {"input": 20_000, "generated": 6_000, "cash_cents": 40}}})
            try:
                module.cmd_dream(Args(root=str(root), ns="-t-budget", run=True, force=True))
                failures.append("gate: refused budget did not stop dream --run")
            except SystemExit as exit_:
                if "DEFERRED — budget" not in str(exit_):
                    failures.append(f"gate: wrong refusal message: {exit_}")
            if composer_calls:
                failures.append("gate: composer ran DESPITE a refused budget admission")
            # (b) healthy path: compose runs twice (composer + critic) and
            # the reservation is settled with the burned bytes.
            seed_ledger(root)
            try:
                module.cmd_dream(Args(root=str(root), ns="-t-budget", run=True, force=True, dry_run=True))
            except SystemExit as exit_:
                failures.append(f"gate: healthy budget refused the dream: {exit_}")
            if len(composer_calls) != 2:
                failures.append(f"gate: expected composer+critic passes, got {len(composer_calls)} calls")
            ledger = json.loads((root / "dream" / "budget" / "ledger.json").read_text())
            settled = [r for r in ledger["reservations"].values() if r.get("state") in ("settled", "settled-late")]
            if len(settled) != 1:
                failures.append(f"gate: expected exactly one settled reservation, got {settled}")
            if spent_dims(ledger, today)["input"] <= 0 or spent_dims(ledger, today)["generated"] <= 0:
                failures.append(f"gate: settle booked nothing: {spent_dims(ledger, today)}")
        finally:
            module._composer_exec = original_composer


def test_budget_op_verb_shape(module, failures):
    """The hidden `_budget-op` plumbing verb: one op in on stdin, one JSON
    result out — the exact contract the ssh transport carries to dev."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run_ = subprocess.run(
            [sys.executable, str(MODULE_PATH), "--root", str(root), "_budget-op"],
            input=json.dumps({"action": "admit", "bucket": "yin", "input": 5,
                              "generated": 1, "cash_cents": 0, "purpose": "transport-shape"}),
            capture_output=True, text=True,
            env={**cli_env(), "PATH": os.environ.get("PATH", "")}, timeout=60,
        )
        if run_.returncode != 0:
            failures.append(f"transport: _budget-op failed: {run_.stdout!r} / {run_.stderr!r}")
            return
        result = json.loads(run_.stdout)
        if not result.get("admitted"):
            failures.append(f"transport: _budget-op admit failed: {result}")
        if result.get("host") is not None and "host" in result:
            failures.append("transport: host echo should ride the op, not the result")


def test_soft_caps_never_refuse_capacity(module, failures):
    """[owner ruling 2026-10-16] SOFT is the standing caps mode: capacity
    never refuses work — the ledger serializes, accounts, and flags
    overborrow; per-item ceilings and bad estimates still refuse (waste
    guards, not capacity guards); settle still books honestly."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        today = module.budget_today()
        seed_ledger(root, spent={today: {"_total": {"input": 100_000, "generated": 10_000, "cash_cents": 50},
                                        "yin": {"input": 100_000, "generated": 10_000, "cash_cents": 50}}})
        env = cli_env()
        env["YGG_BUDGET_CAPS"] = "soft"
        import subprocess as sp
        def soft_cli(*args):
            run_ = sp.run([sys.executable, str(MODULE_PATH), "--root", str(root), "dream-budget", *args],
                          capture_output=True, text=True, env=env, timeout=60)
            return json.loads(run_.stdout)
        past_cap = soft_cli("--admit", "--bucket", "yin", "--input", "40_000",
                            "--generated", "2_000", "--cash-cents", "5", "--purpose", "over-cap")
        if not past_cap.get("admitted"):
            failures.append(f"soft: capacity refused work (must admit + flag): {past_cap}")
        if not past_cap.get("borrowed"):
            failures.append(f"soft: over-cap admission not flagged borrowed: {past_cap}")
        if soft_cli("--admit", "--bucket", "yin", "--input", "0").get("admitted"):
            failures.append("soft: zero estimate admitted (waste guard must survive)")
        ceiling = soft_cli("--admit", "--class", "infra-updates", "--input", "999_999", "--generated", "10")
        if ceiling.get("admitted"):
            failures.append("soft: per-item ceiling breached (waste guard must survive)")
        settled = soft_cli("--settle", past_cap["id"], "--input", "39_000",
                           "--generated", "1_800", "--cash-cents", "4", "--actual-source", "measured")
        if not settled.get("settled"):
            failures.append(f"soft: settle broke: {settled}")
        ledger = json.loads((root / "dream" / "budget" / "ledger.json").read_text())
        if spent_dims(ledger, today)["input"] != 139_000:
            failures.append(f"soft: accounting drifted: {spent_dims(ledger, today)}")


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_race_last_allowance(module, failures)
    test_soft_caps_never_refuse_capacity(module, failures)
    test_midnight_inflight_books_to_reservation_day(module, failures)
    test_ttl_crash_release(module, failures)
    test_bucket_borrow_and_verification_reserve(module, failures)
    test_refusal_shapes(module, failures)
    test_settle_refunds_headroom_conservation(module, failures)
    test_dream_run_gated_by_budget(module, failures)
    test_budget_op_verb_shape(module, failures)
    if failures:
        print(f"dream-budget tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("dream-budget tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
