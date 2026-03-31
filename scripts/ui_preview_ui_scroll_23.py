#!/usr/bin/env python3
import argparse
import json
import random
import time
from pathlib import Path

from ui_preview_23 import (
    FORBIDDEN_PREVIEW_MARKERS,
    app_open,
    app_screenshot_preview,
    app_scroll_preview,
    app_state,
    collect_preview_targets,
    expand_groups_until_target,
    launch_local_client,
    preview_expected_turn_issues,
    preview_ready,
    preview_semantic_issues,
    titlebar_matches_viewport,
    wait_for_window,
    wait_until,
    write_json,
    load_server_state,
    expected_preview_turns_for_session,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Run 23 preview scroll/render checks per session, including fast jumps and "
            "slow reverse passes, and fail on blank mid-scroll states or assistant width drift."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=2323)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.12)
    parser.add_argument("--ready-budget", type=float, default=2.5)
    parser.add_argument("--step-settle-sec", type=float, default=0.18)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-preview-ui-scroll-23")
    return parser.parse_args()


def require_preview_ready(state: dict, session_path: str) -> dict:
    if not preview_ready(state, session_path):
        viewport = state.get("viewport") or {}
        raise RuntimeError(viewport.get("reason") or "preview viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with preview viewport")
    return state


def preview_payload(state: dict) -> dict:
    return ((state.get("viewport") or {}).get("preview") or {})


def visible_entries(state: dict) -> list[dict]:
    return list(preview_payload(state).get("visible_entries") or [])


def visible_texts(entries: list[dict]) -> list[str]:
    return [(entry.get("text") or "").strip() for entry in entries if (entry.get("text") or "").strip()]


def markup_leaks(entries: list[dict]) -> list[str]:
    blob = "\n".join(visible_texts(entries))
    hits = []
    for marker in FORBIDDEN_PREVIEW_MARKERS:
        if marker.lower() in blob.lower():
            hits.append(marker)
    for marker in ("<image name=", "</image>", "```"):
        if marker.lower() in blob.lower():
            hits.append(marker)
    return list(dict.fromkeys(hits))


def step_ratios(rng: random.Random) -> list[float]:
    base = [
        0.0,
        1.0,
        0.92,
        0.08,
        0.75,
        0.18,
        0.58,
        0.24,
        0.86,
        0.34,
        0.66,
        0.42,
        0.96,
        0.12,
        0.52,
        0.28,
        0.78,
        0.04,
        0.62,
        0.16,
        0.48,
        0.88,
        0.0,
    ]
    jittered = []
    for ratio in base:
        if ratio in (0.0, 1.0):
            jittered.append(ratio)
        else:
            jittered.append(min(1.0, max(0.0, ratio + rng.uniform(-0.035, 0.035))))
    return jittered


def width_buckets(states: list[dict]) -> dict[str, list[int]]:
    buckets: dict[str, list[int]] = {}
    for state in states:
        for entry in visible_entries(state):
            if entry.get("tone") != "assistant":
                continue
            text = (entry.get("text") or "").strip()
            block_ix = entry.get("block_ix")
            width = int(entry.get("width") or 0)
            if not text or block_ix is None or block_ix < 0 or width <= 0:
                continue
            key = f"{block_ix}:{text[:96]}"
            buckets.setdefault(key, []).append(width)
    return buckets


def width_shift_issues(states: list[dict]) -> list[str]:
    issues = []
    for key, widths in width_buckets(states).items():
        if len(widths) < 2:
            continue
        span = max(widths) - min(widths)
        if span > 56:
            issues.append(f"assistant width drift {span}px for {key}")
    return issues


def blank_step_issue(state: dict) -> str | None:
    entries = visible_entries(state)
    preview = preview_payload(state)
    window = preview.get("window") or {}
    if entries:
        return None
    visible_count = int(preview.get("visible_block_count") or 0)
    total_count = int(window.get("total_count") or 0)
    if visible_count == 0 and total_count > 0:
        return "preview visible entries collapsed to zero while total_count stayed positive"
    if not (preview.get("text_sample") or "").strip() and total_count > 0:
        return "preview text sample went blank while total_count stayed positive"
    return None


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)

    launch = None
    if args.host == "local" and args.launch_local:
        launch, _ = launch_local_client(args.bin)

    wait_for_window(args.host, args.bin, args.timeout_ms)
    server_state = load_server_state() if args.host == "local" else {}
    rows_payload = expand_groups_until_target(args.host, args.bin, args.timeout_ms, args.count)
    targets = collect_preview_targets(rows_payload, rng, args.count)
    if not targets:
        raise RuntimeError("no preview targets available")

    results = []
    try:
        for index, target in enumerate(targets):
            session_path = target["full_path"]
            expected_turns = expected_preview_turns_for_session(server_state, session_path, args.bin)
            ratios = step_ratios(rng)
            entry = {
                "path": session_path,
                "label": target["label"],
                "kind": target["kind"],
                "ratios": ratios,
            }
            step_states = []
            step_results = []
            try:
                entry["open"] = app_open(args.host, args.bin, session_path, args.timeout_ms)
                _, state = wait_until(
                    f"preview baseline {session_path}",
                    args.ready_budget,
                    args.poll,
                    lambda: require_preview_ready(app_state(args.host, args.bin, args.timeout_ms), session_path),
                )
                step_states.append(state)
                first_shot = out_dir / f"preview-ui-scroll-{index:02d}-step-00.png"
                app_screenshot_preview(args.host, args.bin, first_shot, args.timeout_ms)

                for step_index, ratio in enumerate(ratios, start=1):
                    app_scroll_preview(
                        args.host,
                        args.bin,
                        timeout_ms=args.timeout_ms,
                        ratio=ratio,
                    )
                    time.sleep(args.step_settle_sec)
                    _, state = wait_until(
                        f"preview scroll step {step_index} {session_path}",
                        args.ready_budget,
                        args.poll,
                        lambda: require_preview_ready(app_state(args.host, args.bin, args.timeout_ms), session_path),
                    )
                    step_states.append(state)
                    entries = visible_entries(state)
                    preview = preview_payload(state)
                    window = preview.get("window") or {}
                    semantic_issues = (
                        preview_semantic_issues(state)
                        + preview_expected_turn_issues(state, expected_turns)
                    )
                    blank_issue = blank_step_issue(state)
                    step_entry = {
                        "step": step_index,
                        "ratio": ratio,
                        "visible_entry_count": len(entries),
                        "visible_block_count": preview.get("visible_block_count") or 0,
                        "text_sample": preview.get("text_sample") or "",
                        "window": window,
                        "semantic_issues": semantic_issues,
                        "markup_leaks": markup_leaks(entries),
                        "blank_issue": blank_issue,
                    }
                    if step_index in (1, 11, 23):
                        shot = out_dir / f"preview-ui-scroll-{index:02d}-step-{step_index:02d}.png"
                        app_screenshot_preview(args.host, args.bin, shot, args.timeout_ms)
                        step_entry["screenshot"] = str(shot)
                    step_results.append(step_entry)

                width_issues = width_shift_issues(step_states)
                blank_issues = [item["blank_issue"] for item in step_results if item.get("blank_issue")]
                semantic_issues = [
                    issue
                    for item in step_results
                    for issue in item.get("semantic_issues") or []
                ]
                markup_issues = [
                    issue
                    for item in step_results
                    for issue in item.get("markup_leaks") or []
                ]
                entry.update(
                    {
                        "step_results": step_results,
                        "step_state_dump": write_json(
                            out_dir / f"preview-ui-scroll-{index:02d}.json",
                            {"steps": step_states},
                        ),
                        "width_issues": list(dict.fromkeys(width_issues)),
                        "blank_issues": list(dict.fromkeys(blank_issues)),
                        "semantic_issues": list(dict.fromkeys(semantic_issues)),
                        "markup_issues": list(dict.fromkeys(markup_issues)),
                    }
                )
            except Exception as error:  # noqa: BLE001
                state = {}
                try:
                    state = app_state(args.host, args.bin, args.timeout_ms)
                except Exception:
                    pass
                entry["error"] = str(error)
                entry["failure_state_dump"] = write_json(
                    out_dir / f"preview-ui-scroll-{index:02d}-failure.json",
                    state,
                )
            results.append(entry)
    finally:
        if launch is not None and launch.poll() is None:
            launch.terminate()
            try:
                launch.wait(timeout=3)
            except Exception:
                launch.kill()

    summary = {
        "count": len(results),
        "scroll_checks_per_session": 23,
        "open_failures": sum(1 for item in results if item.get("error")),
        "blank_failures": sum(1 for item in results if item.get("blank_issues")),
        "width_failures": sum(1 for item in results if item.get("width_issues")),
        "semantic_failures": sum(1 for item in results if item.get("semantic_issues")),
        "markup_failures": sum(1 for item in results if item.get("markup_issues")),
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))
    return 0 if (
        summary["open_failures"] == 0
        and summary["blank_failures"] == 0
        and summary["width_failures"] == 0
        and summary["semantic_failures"] == 0
        and summary["markup_failures"] == 0
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
