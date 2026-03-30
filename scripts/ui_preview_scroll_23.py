#!/usr/bin/env python3
import argparse
import json
import random
import subprocess
from pathlib import Path

from ui_preview_23 import (
    FORBIDDEN_PREVIEW_MARKERS,
    app_open,
    app_rows,
    app_screenshot_preview,
    app_scroll_preview,
    app_state,
    collect_preview_targets,
    expand_groups_until_target,
    launch_local_client,
    preview_ready,
    read_png_size,
    run_process,
    titlebar_matches_viewport,
    wait_for_window,
    wait_until,
    write_json,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Open 23 previewable rows, jump to varied scroll positions, and verify "
            "viewport-only screenshots plus virtual-window behavior."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=2303)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.12)
    parser.add_argument("--ready-budget", type=float, default=2.3)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-preview-scroll-23")
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


def require_surface_idle(state: dict) -> dict:
    requests = state.get("active_surface_requests") or []
    if requests:
        raise RuntimeError(f"surface requests still pending: {len(requests)}")
    return state


def pick_ratio(rng: random.Random, index: int) -> float:
    bucket = [0.0, 0.08, 0.16, 0.24, 0.35, 0.5, 0.66, 0.82, 0.94, 1.0]
    return bucket[(index + rng.randrange(len(bucket))) % len(bucket)]


def main() -> int:
    args = parse_args()
    rng = random.Random(args.seed)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

    wait_for_window(args.host, args.bin, args.timeout_ms)
    wait_until(
        "surface idle after launch",
        4.0,
        args.poll,
        lambda: require_surface_idle(app_state(args.host, args.bin, args.timeout_ms)),
    )
    rows_payload = expand_groups_until_target(args.host, args.bin, args.timeout_ms, args.count)
    targets = collect_preview_targets(rows_payload, rng, args.count)
    if not targets:
        raise RuntimeError("no preview targets available")

    results = []
    for index, target in enumerate(targets):
        session_path = target["full_path"]
        ratio = pick_ratio(rng, index)
        entry = {
            "path": session_path,
            "label": target["label"],
            "kind": target["kind"],
            "ratio": ratio,
        }
        try:
            entry["open"] = app_open(args.host, args.bin, session_path, args.timeout_ms)
            _, initial_state = wait_until(
                f"preview baseline {session_path}",
                args.ready_budget,
                args.poll,
                lambda: require_preview_ready(app_state(args.host, args.bin, args.timeout_ms), session_path),
            )
            initial_preview = preview_payload(initial_state)
            initial_window = initial_preview.get("window") or {}
            initial_max_top = max(
                0.0,
                float(initial_window.get("scroll_height_px") or 0)
                - float(initial_window.get("client_height_px") or 0),
            )
            entry["initial_state_dump"] = write_json(
                out_dir / f"preview-scroll-{index:02d}-initial.json",
                initial_state,
            )
            entry["scroll"] = app_scroll_preview(
                args.host,
                args.bin,
                timeout_ms=args.timeout_ms,
                ratio=ratio,
            )
            _, scrolled_state = wait_until(
                f"preview scroll {session_path}",
                args.ready_budget,
                args.poll,
                lambda: require_preview_ready(app_state(args.host, args.bin, args.timeout_ms), session_path),
            )
            scrolled_preview = preview_payload(scrolled_state)
            scrolled_window = scrolled_preview.get("window") or {}
            screenshot_path = out_dir / f"preview-scroll-{index:02d}.png"
            entry["screenshot"] = app_screenshot_preview(
                args.host,
                args.bin,
                screenshot_path,
                args.timeout_ms,
            )
            screenshot_dom = (((entry["screenshot"].get("data") or {}).get("dom")) or {})
            viewport_rect = (
                screenshot_dom.get("preview_viewport_rect")
                or scrolled_preview.get("viewport_rect")
                or {}
            )
            png_width, png_height = read_png_size(screenshot_path)
            font_family = (
                (screenshot_dom.get("preview_font_family") or scrolled_preview.get("font_family") or "")
                .strip()
            )
            text_sample = (
                (screenshot_dom.get("preview_text_sample") or scrolled_preview.get("text_sample") or "")
                .strip()
            )
            scroll_data = (entry["scroll"].get("data") or {}).get("scroll") or {}
            max_top = float(
                scroll_data.get("max_top_px")
                or max(
                    0.0,
                    float(scrolled_window.get("scroll_height_px") or 0)
                    - float(scrolled_window.get("client_height_px") or 0),
                )
                or initial_max_top
            )
            applied_top = float(
                scroll_data.get("applied_top_px") or scrolled_window.get("scroll_top_px") or 0.0
            )
            expected_top = max_top * ratio
            significant_scroll = max_top >= 160.0 and ratio >= 0.16
            visible_ids_before = initial_preview.get("visible_block_ids") or []
            visible_ids_after = scrolled_preview.get("visible_block_ids") or []
            visible_ids_changed = visible_ids_before != visible_ids_after
            scroll_effect_ok = True
            if significant_scroll:
                scroll_effect_ok = applied_top >= min(max_top, max(60.0, expected_top * 0.45))
            virtualization_ok = (
                isinstance(scrolled_window.get("start_index"), int)
                and isinstance(scrolled_window.get("end_index"), int)
                and scrolled_window.get("end_index", 0) >= scrolled_window.get("start_index", 0)
                and scrolled_window.get("total_count", 0) >= scrolled_window.get("end_index", 0)
                and (
                    scrolled_window.get("total_count", 0) <= scrolled_window.get("end_index", 0)
                    or float(scrolled_window.get("overscan_px") or 0.0) > 0.0
                )
            )
            entry.update(
                {
                    "state_dump": write_json(
                        out_dir / f"preview-scroll-{index:02d}.json",
                        scrolled_state,
                    ),
                    "preview_png": str(screenshot_path),
                    "png_width": png_width,
                    "png_height": png_height,
                    "viewport_rect": viewport_rect,
                    "viewport_size_matches": (
                        abs(png_width - int(round(viewport_rect.get("width") or 0))) <= 4
                        and abs(png_height - int(round(viewport_rect.get("height") or 0))) <= 4
                    ),
                    "font_family": font_family,
                    "font_family_ok": any(token in font_family.lower() for token in ("serif", "georgia", "iowan")),
                    "titlebar_matches_viewport": titlebar_matches_viewport(scrolled_state),
                    "forbidden_hits": [
                        marker
                        for marker in FORBIDDEN_PREVIEW_MARKERS
                        if marker.lower() in text_sample.lower()
                    ],
                    "initial_window": initial_window,
                    "scrolled_window": scrolled_window,
                    "initial_max_top_px": initial_max_top,
                    "max_top_px": max_top,
                    "expected_top_px": round(expected_top, 2),
                    "applied_top_px": applied_top,
                    "visible_ids_before": visible_ids_before,
                    "visible_ids_after": visible_ids_after,
                    "visible_ids_changed": visible_ids_changed,
                    "scroll_effect_ok": scroll_effect_ok,
                    "virtualization_ok": virtualization_ok,
                }
            )
        except Exception as error:  # noqa: BLE001
            state = {}
            try:
                state = app_state(args.host, args.bin, args.timeout_ms)
            except Exception:
                pass
            entry["error"] = str(error)
            entry["state_dump"] = write_json(
                out_dir / f"preview-scroll-{index:02d}-failure.json",
                state,
            )
        results.append(entry)
        try:
            wait_until(
                f"surface idle after {session_path}",
                4.0,
                args.poll,
                lambda: require_surface_idle(app_state(args.host, args.bin, args.timeout_ms)),
            )
        except Exception:
            pass

    summary = {
        "host": args.host,
        "count": args.count,
        "executed_count": len(results),
        "ready_budget_s": args.ready_budget,
        "window_spawn_elapsed_ms": ((launch_event or {}).get("payload") or {}).get("elapsed_ms"),
        "open_failures": len([item for item in results if item.get("error")]),
        "titlebar_failures": len([item for item in results if not item.get("titlebar_matches_viewport", False)]),
        "viewport_size_failures": len([item for item in results if not item.get("viewport_size_matches", False)]),
        "font_failures": len([item for item in results if not item.get("font_family_ok", False)]),
        "forbidden_content_failures": len([item for item in results if item.get("forbidden_hits")]),
        "scroll_effect_failures": len([item for item in results if not item.get("scroll_effect_ok", False)]),
        "virtualization_failures": len([item for item in results if not item.get("virtualization_ok", False)]),
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))

    if launch is not None and launch.poll() is None:
        launch.terminate()
        try:
            launch.wait(timeout=2)
        except subprocess.TimeoutExpired:
            launch.kill()
        run_process(["bash", "-lc", "pkill -f 'yggterm server daemon' || true"], check=False)

    return 0 if (
        summary["open_failures"] == 0
        and summary["titlebar_failures"] == 0
        and summary["viewport_size_failures"] == 0
        and summary["font_failures"] == 0
        and summary["forbidden_content_failures"] == 0
        and summary["scroll_effect_failures"] == 0
        and summary["virtualization_failures"] == 0
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
