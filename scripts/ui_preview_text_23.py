#!/usr/bin/env python3
import argparse
import json
import random
import time
from pathlib import Path

from ui_preview_23 import (
    app_open,
    app_screenshot_preview,
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
            "Open preview sessions, snapshot the live viewport shortly after open and again "
            "after a dwell period, and fail if visible transcript content mutates without user input."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=2317)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.15)
    parser.add_argument("--ready-budget", type=float, default=2.5)
    parser.add_argument("--settle-budget", type=float, default=8.0)
    parser.add_argument("--dwell-sec", type=float, default=23.0)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-preview-text-23")
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


def normalized_visible_signature(state: dict) -> list[dict]:
    preview = preview_payload(state)
    signature = []
    visible_entries = list(preview.get("visible_entries") or [])
    if visible_entries:
        for entry in visible_entries:
            signature.append(
                {
                    "tone": entry.get("tone"),
                    "text": (entry.get("text") or "").strip(),
                    "height": entry.get("height"),
                    "top": entry.get("top"),
                    "block_ix": entry.get("block_ix"),
                }
            )
        return signature
    for index, section in enumerate(preview.get("rendered_sections") or []):
        lines = [line.strip() for line in (section.get("lines") or []) if line and line.strip()]
        text = "\n".join(([section.get("title") or ""] + lines)).strip()
        signature.append(
            {
                "tone": "section",
                "text": text,
                "height": section.get("height"),
                "top": section.get("top"),
                "block_ix": -(index + 1),
            }
        )
    return signature


def block_texts(signature: list[dict]) -> list[str]:
    return [item.get("text", "") for item in signature if item.get("text")]


def compare_signatures(early: list[dict], late: list[dict]) -> list[str]:
    issues = []
    if not early:
        issues.append("early snapshot had no visible entries")
        return issues
    if not late:
        issues.append("late snapshot had no visible entries")
        return issues
    if len(late) < max(1, len(early) // 2):
        issues.append(
            f"late visible entry count collapsed from {len(early)} to {len(late)}"
        )
    early_texts = block_texts(early)
    late_texts = block_texts(late)
    overlap = len(set(early_texts) & set(late_texts))
    if overlap == 0:
        issues.append("late snapshot shares no visible transcript text with early snapshot")
    elif overlap < max(1, min(len(early_texts), len(late_texts)) // 3):
        issues.append(
            f"late snapshot overlap is too small ({overlap} shared blocks)"
        )
    return issues


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

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
            entry = {
                "path": session_path,
                "label": target["label"],
                "kind": target["kind"],
                "dwell_sec": args.dwell_sec,
            }
            try:
                entry["open"] = app_open(args.host, args.bin, session_path, args.timeout_ms)
                initial_state = app_state(args.host, args.bin, args.timeout_ms)
                initial_viewport = initial_state.get("viewport") or {}
                entry["initial_state_dump"] = write_json(
                    out_dir / f"preview-text-{index:02d}-initial.json",
                    initial_state,
                )
                entry["initial_loading"] = (
                    (initial_viewport.get("reason") == "preview still loading")
                    or not preview_ready(initial_state, session_path)
                )
                _, early_state = wait_until(
                    f"preview early {session_path}",
                    max(args.ready_budget, args.settle_budget),
                    args.poll,
                    lambda: require_preview_ready(app_state(args.host, args.bin, args.timeout_ms), session_path),
                )
                early_png = out_dir / f"preview-text-{index:02d}-early.png"
                entry["early_screenshot"] = app_screenshot_preview(
                    args.host,
                    args.bin,
                    early_png,
                    args.timeout_ms,
                )
                early_signature = normalized_visible_signature(early_state)
                time.sleep(args.dwell_sec)
                _, late_state = wait_until(
                    f"preview late {session_path}",
                    args.ready_budget,
                    args.poll,
                    lambda: require_preview_ready(app_state(args.host, args.bin, args.timeout_ms), session_path),
                )
                late_png = out_dir / f"preview-text-{index:02d}-late.png"
                entry["late_screenshot"] = app_screenshot_preview(
                    args.host,
                    args.bin,
                    late_png,
                    args.timeout_ms,
                )
                late_signature = normalized_visible_signature(late_state)
                early_preview = preview_payload(early_state)
                late_preview = preview_payload(late_state)
                entry.update(
                    {
                        "early_state_dump": write_json(
                            out_dir / f"preview-text-{index:02d}-early.json",
                            early_state,
                        ),
                        "late_state_dump": write_json(
                            out_dir / f"preview-text-{index:02d}-late.json",
                            late_state,
                        ),
                        "early_visible_signature": early_signature,
                        "late_visible_signature": late_signature,
                        "early_semantic_issues": preview_semantic_issues(early_state),
                        "late_semantic_issues": preview_semantic_issues(late_state),
                        "early_expected_turn_issues": preview_expected_turn_issues(
                            early_state, expected_turns
                        ),
                        "late_expected_turn_issues": preview_expected_turn_issues(
                            late_state, expected_turns
                        ),
                        "stability_issues": compare_signatures(early_signature, late_signature),
                        "early_window": early_preview.get("window") or {},
                        "late_window": late_preview.get("window") or {},
                        "early_text_sample": early_preview.get("text_sample") or "",
                        "late_text_sample": late_preview.get("text_sample") or "",
                        "early_visible_count": early_preview.get("visible_block_count") or 0,
                        "late_visible_count": late_preview.get("visible_block_count") or 0,
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
                    out_dir / f"preview-text-{index:02d}-failure.json",
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
        "launch_event": launch_event,
        "dwell_sec": args.dwell_sec,
        "open_failures": sum(1 for item in results if item.get("error")),
        "initial_loading_count": sum(1 for item in results if item.get("initial_loading")),
        "early_semantic_failures": sum(
            1 for item in results if item.get("early_semantic_issues")
        ),
        "late_semantic_failures": sum(
            1 for item in results if item.get("late_semantic_issues")
        ),
        "early_expected_turn_failures": sum(
            1 for item in results if item.get("early_expected_turn_issues")
        ),
        "late_expected_turn_failures": sum(
            1 for item in results if item.get("late_expected_turn_issues")
        ),
        "stability_failures": sum(1 for item in results if item.get("stability_issues")),
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
