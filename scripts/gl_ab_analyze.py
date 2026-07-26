#!/usr/bin/env python3
"""Gate and report a GL A/B/C run produced by scripts/gl_ab_experiment.sh.

This analyzer exists to REFUSE. The same measurement has now produced four
confident wrong answers, one per confounder, and in every case the number
looked fine:

  1. exposure      an evening of real use against an overnight window with 23x
                   less terminal activity -> "18x faster"
  2. idle-vs-soft  gpu_ms == 0 read as "software rasterization" on a host that
                   had just switched to hardware; it meant nothing was painting
                   (523 of 532 ticks)
  3. bad control   copy_scan "certifying" a render change while measuring its
                   own fix, landed in the same deploy
  4. focus         1,194 of 1,256 render/gui rows unfocused, all pre-fix rows
                   among them, every "after" number focused -> a 3-7x focus
                   effect reported as a 2.3x GL "regression"

So every contrast is gated four ways plus a drift control, and a run that
passes none of them prints WHY rather than a number. "This settles nothing" is
a valid, intended outcome.

Usage:
  gl_ab_analyze.py <outdir>        analyze a run
  gl_ab_analyze.py --self-test     prove the gates FIRE (no run needed)
"""

from __future__ import annotations

import json
import random
import statistics
import sys
from pathlib import Path

# A software arm may show NO GPU engine time at all. Any is disqualifying.
SOFTWARE_ARM_GPU_MS_TOLERANCE = 0
# A hardware arm whose samples are mostly gpu_ms == 0 is an IDLE window, not a
# GPU arm. This is exactly the 523-of-532 signature that produced the withdrawn
# 18x claim.
HARDWARE_ARM_MAX_IDLE_FRACTION = 0.10
# Arms whose paint exposure medians differ by more than this are not comparable
# at all; the contrast is refused outright rather than adjusted.
EXPOSURE_MAX_RELATIVE_GAP = 0.20
BOOTSTRAP_ROUNDS = 2000
BOOTSTRAP_SEED = 20260726  # fixed: an analyzer that answers differently on
# re-run cannot settle an argument


class Refusal(Exception):
    """A gate fired. Carries the reason, which is the whole product."""


def load_samples(outdir: Path) -> list[dict]:
    path = outdir / "samples.jsonl"
    if not path.exists():
        raise Refusal(f"no samples at {path} — the run produced nothing to analyze")
    rows = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            rows.append(json.loads(line))
    if not rows:
        raise Refusal(f"{path} is empty — every sample was refused by the measurement")
    return rows


def focus_intervals(trace_path: Path) -> list[tuple[int, int]]:
    """Focused intervals as a STEP FUNCTION from ui/window_focus/transition.

    Reconstructed rather than read per sample, because focus is a property of
    the WINDOW over time and a sample that straddles a transition is neither
    focused nor unfocused — it is uninterpretable, and averaging it in is
    exactly how the 2.3x "regression" happened. An interval left open at the
    end of the trace stays open.
    """
    if not trace_path.exists():
        return []
    events = []
    for line in trace_path.read_text(errors="replace").splitlines():
        if '"window_focus"' not in line:
            continue
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("category") != "window_focus" or event.get("name") != "transition":
            continue
        events.append((int(event["ts_ms"]), bool((event.get("payload") or {}).get("focused"))))
    events.sort()
    intervals: list[tuple[int, int]] = []
    open_at: int | None = None
    for ts, focused in events:
        if focused and open_at is None:
            open_at = ts
        elif not focused and open_at is not None:
            intervals.append((open_at, ts))
            open_at = None
    if open_at is not None:
        intervals.append((open_at, 1 << 62))
    return intervals


def wholly_inside(sample: dict, intervals: list[tuple[int, int]]) -> bool:
    t0, t1 = int(sample["t0_ms"]), int(sample["t1_ms"])
    return any(start <= t0 and t1 <= end for start, end in intervals)


def exposure(perf_path: Path, t0: int, t1: int) -> int:
    """xterm_write_flush events inside the window: how much actually painted."""
    if not perf_path.exists():
        return 0
    count = 0
    for line in perf_path.read_text(errors="replace").splitlines():
        if "xterm_write_flush" not in line:
            continue
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("name") != "xterm_write_flush":
            continue
        if t0 <= int(event.get("ts_ms", 0)) <= t1:
            count += 1
    return count


def annotate(rows: list[dict], outdir: Path) -> list[dict]:
    by_arm: dict[str, list[dict]] = {}
    for row in rows:
        by_arm.setdefault(row["arm"], []).append(row)
    for arm, arm_rows in by_arm.items():
        intervals = focus_intervals(outdir / f"{arm}.event-trace.jsonl")
        perf = outdir / f"{arm}.perf-telemetry.jsonl"
        for row in arm_rows:
            row["focused"] = wholly_inside(row, intervals) if intervals else None
            row["exposure"] = exposure(perf, int(row["t0_ms"]), int(row["t1_ms"]))
    return rows


def gate_arm_integrity(arm: str, rows: list[dict]) -> None:
    gpu = [float(row.get("gpu_ms") or 0) for row in rows]
    if arm.startswith("S"):
        hot = sum(1 for value in gpu if value > SOFTWARE_ARM_GPU_MS_TOLERANCE)
        if hot:
            raise Refusal(
                f"arm {arm} is nominally SOFTWARE but {hot}/{len(gpu)} samples "
                f"show GPU engine time — llvmpipe never opens a DRM node, so "
                f"this arm was not software and the contrast is meaningless"
            )
    else:
        idle = sum(1 for value in gpu if value <= 0)
        fraction = idle / len(gpu)
        if fraction > HARDWARE_ARM_MAX_IDLE_FRACTION:
            raise Refusal(
                f"arm {arm} is nominally HARDWARE but {idle}/{len(gpu)} samples "
                f"({fraction:.0%}) show zero GPU time — that is an IDLE window, "
                f"not a GPU arm. This is the 523-of-532 signature that produced "
                f"the withdrawn 18x claim"
            )


def gate_focus(arm: str, rows: list[dict]) -> list[dict]:
    if all(row.get("focused") is None for row in rows):
        raise Refusal(
            f"arm {arm} has no ui/window_focus/transition trace, so focus cannot "
            f"be established per sample. Focus moves render/gui by 3-7x; a "
            f"contrast taken without it measures focus"
        )
    kept = [row for row in rows if row.get("focused")]
    dropped = len(rows) - len(kept)
    if not kept:
        raise Refusal(f"arm {arm}: all {len(rows)} samples straddle or sit outside a focused interval")
    print(f"  {arm}: {len(kept)} samples inside a focused interval, {dropped} dropped")
    return kept


def hodges_lehmann(a: list[float], b: list[float]) -> float:
    """Median of all pairwise a-b differences: the shift, robust to outliers."""
    return statistics.median([x - y for x in a for y in b])


def bootstrap_ci(a: list[float], b: list[float]) -> tuple[float, float]:
    rng = random.Random(BOOTSTRAP_SEED)
    shifts = []
    for _ in range(BOOTSTRAP_ROUNDS):
        ra = [rng.choice(a) for _ in a]
        rb = [rng.choice(b) for _ in b]
        shifts.append(statistics.median(ra) - statistics.median(rb))
    shifts.sort()
    lo = shifts[int(0.025 * len(shifts))]
    hi = shifts[int(0.975 * len(shifts)) - 1]
    return lo, hi


def matched_exposure_strata(a: list[dict], b: list[dict]) -> tuple[list[float], list[float]]:
    """Recompute the contrast inside matched exposure bins.

    Even arms that PASS the >20% gate can differ enough within their spread to
    move a small contrast, so the stratified figure is reported alongside the
    raw one and the two must agree in sign before anything is claimed.
    """
    bins_a: dict[int, list[float]] = {}
    bins_b: dict[int, list[float]] = {}
    for rows, bins in ((a, bins_a), (b, bins_b)):
        for row in rows:
            key = int(row["exposure"]) // 10
            bins.setdefault(key, []).append(float(row["cores"]))
    shared = sorted(set(bins_a) & set(bins_b))
    return (
        [value for key in shared for value in bins_a[key]],
        [value for key in shared for value in bins_b[key]],
    )


def contrast(name: str, left: str, right: str, arms: dict[str, list[dict]], drift: float | None) -> None:
    if left not in arms or right not in arms:
        print(f"  {name} ({left} - {right}): NOT RUN")
        return
    a = arms[left]
    b = arms[right]
    exp_a = statistics.median([row["exposure"] for row in a])
    exp_b = statistics.median([row["exposure"] for row in b])
    if max(exp_a, exp_b) > 0:
        gap = abs(exp_a - exp_b) / max(exp_a, exp_b)
        if gap > EXPOSURE_MAX_RELATIVE_GAP:
            print(
                f"  {name} ({left} - {right}): REFUSED — paint exposure differs by "
                f"{gap:.0%} (median flushes {exp_a:.0f} vs {exp_b:.0f}). "
                f"Unequal exposure is confounder #1; it produced an 18x 'win'."
            )
            return

    cores_a = [float(row["cores"]) for row in a]
    cores_b = [float(row["cores"]) for row in b]
    shift = hodges_lehmann(cores_a, cores_b)
    lo, hi = bootstrap_ci(cores_a, cores_b)
    strat_a, strat_b = matched_exposure_strata(a, b)
    strat = hodges_lehmann(strat_a, strat_b) if strat_a and strat_b else float("nan")

    verdict = "a result"
    if lo <= 0 <= hi:
        verdict = "NOT SIGNIFICANT (the CI spans zero)"
    elif drift is not None and abs(shift) <= abs(drift):
        verdict = f"INDISTINGUISHABLE FROM DRIFT (|shift| {abs(shift):.4f} <= |S2-S| {abs(drift):.4f})"
    elif strat == strat and (strat > 0) != (shift > 0):
        verdict = "REFUSED — raw and exposure-matched contrasts disagree in SIGN"

    print(
        f"  {name} ({left} - {right}): {shift:+.4f} cores "
        f"[95% CI {lo:+.4f}, {hi:+.4f}] exposure-matched {strat:+.4f} -> {verdict}"
    )


def analyze(outdir: Path) -> int:
    rows = annotate(load_samples(outdir), outdir)
    arms: dict[str, list[dict]] = {}
    for row in rows:
        arms.setdefault(row["arm"], []).append(row)

    print(f"GL A/B/C run: {outdir}")
    print(f"raw samples: {len(rows)} across arms {sorted(arms)}")
    print("\ngates:")
    kept: dict[str, list[dict]] = {}
    refusals = []
    for arm in sorted(arms):
        try:
            gate_arm_integrity(arm, arms[arm])
            kept[arm] = gate_focus(arm, arms[arm])
        except Refusal as refusal:
            refusals.append(str(refusal))
            print(f"  {arm}: VOID — {refusal}")

    if refusals:
        print("\nTHIS RUN SETTLES NOTHING. Voided arms:")
        for refusal in refusals:
            print(f"  - {refusal}")
        return 2

    print("\nper arm (cores, focused samples only):")
    for arm in sorted(kept):
        values = sorted(float(row["cores"]) for row in kept[arm])
        print(
            f"  {arm}: n={len(values)} median={statistics.median(values):.4f} "
            f"sd={statistics.pstdev(values):.4f} "
            f"median_exposure={statistics.median([r['exposure'] for r in kept[arm]]):.0f}"
        )

    drift = None
    if "S" in kept and "S2" in kept:
        drift = hodges_lehmann(
            [float(row["cores"]) for row in kept["S2"]],
            [float(row["cores"]) for row in kept["S"]],
        )
        print(f"\ndrift control S2 - S: {drift:+.4f} cores — nothing smaller than this is a result")
    else:
        print("\n⚠ no drift control (needs both S and S2): every contrast below is UNBOUNDED by drift")

    print("\ncontrasts:")
    contrast("the GL flip alone", "G", "S", kept, drift)
    contrast("under-glass alone", "H", "G", kept, drift)
    contrast("2.12.14 end to end", "H", "S", kept, drift)
    print(
        "\n⚠ cores is not the user's complaint — the FAN is. Moving raster to the "
        "GPU can lower cores and raise package power. Quote amdgpu power1_average "
        "beside any win, and say it is a proxy: RAPL energy_uj is root-only on jojo."
    )
    return 0


# ---------------------------------------------------------------------------
# self-test: prove the gates FIRE. A gate that can only pass is worth nothing.
# ---------------------------------------------------------------------------
def _sample(arm, cores, gpu_ms, t0, exposure_count, focused=True):
    return {
        "arm": arm,
        "cores": cores,
        "gpu_ms": gpu_ms,
        "t0_ms": t0,
        "t1_ms": t0 + 5000,
        "exposure": exposure_count,
        "focused": focused,
    }


def self_test() -> int:
    failures = []

    def expect_refusal(label, fn, needle):
        try:
            fn()
        except Refusal as refusal:
            if needle not in str(refusal):
                failures.append(f"{label}: refused, but not for {needle!r}: {refusal}")
            return
        failures.append(f"{label}: NO REFUSAL — this gate cannot fail")

    # 1. a software arm that touched the GPU is not a software arm
    expect_refusal(
        "software-arm-integrity",
        lambda: gate_arm_integrity("S", [_sample("S", 0.3, 0, 0, 40), _sample("S", 0.3, 12, 5000, 40)]),
        "nominally SOFTWARE",
    )
    # 2. the 523-of-532 signature: a "hardware" arm that was simply idle
    expect_refusal(
        "hardware-arm-idle",
        lambda: gate_arm_integrity("H", [_sample("H", 0.02, 0, i * 5000, 40) for i in range(532)]
                                   + [_sample("H", 0.02, 5, 9_000_000, 40) for _ in range(9)]),
        "IDLE window",
    )
    # 3. no focus evidence at all
    expect_refusal(
        "focus-missing",
        lambda: gate_focus("H", [_sample("H", 0.2, 5, 0, 40, focused=None)]),
        "focus cannot",
    )
    # 4. every sample straddles a transition
    expect_refusal(
        "focus-all-dropped",
        lambda: gate_focus("H", [_sample("H", 0.2, 5, 0, 40, focused=False)]),
        "straddle",
    )
    # 5. no samples at all
    expect_refusal(
        "no-samples",
        lambda: load_samples(Path("/nonexistent-gl-ab-dir")),
        "no samples",
    )

    # 6. the arms that SHOULD pass, do — a gate that refuses everything is as
    #    useless as one that refuses nothing.
    try:
        gate_arm_integrity("S", [_sample("S", 0.3, 0, i * 5000, 40) for i in range(20)])
        gate_arm_integrity("H", [_sample("H", 0.3, 7, i * 5000, 40) for i in range(20)])
        gate_focus("H", [_sample("H", 0.3, 7, i * 5000, 40) for i in range(20)])
    except Refusal as refusal:
        failures.append(f"clean-arms: refused a valid arm: {refusal}")

    # 7. the exposure gate refuses a mismatched contrast, and says which
    #    confounder it is
    import io
    import contextlib

    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        contrast(
            "test",
            "H",
            "S",
            {
                "H": [_sample("H", 0.30, 7, i * 5000, 100) for i in range(20)],
                "S": [_sample("S", 0.30, 0, i * 5000, 10) for i in range(20)],
            },
            None,
        )
    if "REFUSED" not in buffer.getvalue() or "exposure differs" not in buffer.getvalue():
        failures.append(f"exposure-gate: no refusal on a 10x exposure gap: {buffer.getvalue()!r}")

    # 8. a contrast smaller than the drift control is not a result
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        contrast(
            "test",
            "H",
            "S",
            {
                "H": [_sample("H", 0.30 + i * 0.0001, 7, i * 5000, 40) for i in range(30)],
                "S": [_sample("S", 0.29 + i * 0.0001, 0, i * 5000, 40) for i in range(30)],
            },
            0.5,
        )
    if "DRIFT" not in buffer.getvalue():
        failures.append(f"drift-gate: a 0.01 shift under 0.5 drift was not caught: {buffer.getvalue()!r}")

    if failures:
        print("SELF-TEST FAILED:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("SELF-TEST PASSED: all 8 gate behaviours verified (5 refusals fire, "
          "clean arms pass, exposure and drift gates fire)")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) > 1 and argv[1] == "--self-test":
        return self_test()
    if len(argv) < 2:
        print(__doc__)
        return 64
    try:
        return analyze(Path(argv[1]))
    except Refusal as refusal:
        print(f"THIS RUN SETTLES NOTHING: {refusal}")
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
