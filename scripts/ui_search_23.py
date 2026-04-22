#!/usr/bin/env python3
import argparse
import json
import random
import re
import subprocess
from pathlib import Path

from ui_stress_23 import (
    ReadinessPending,
    app_open,
    app_rows,
    app_state,
    choose_session_targets,
    launch_local_client,
    server_inventory,
    wait_for_app_state,
    wait_for_openable_rows,
    wait_until,
    write_state_dump,
    preview_ready,
    viewport_failure_reason,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a deterministic 23-pass search QA lane against a live Yggterm GUI."
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--out-dir", default="/tmp/yggterm-search-23")
    parser.add_argument("--launch-local", action="store_true")
    return parser.parse_args()


def run_process(argv: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=check, text=True, capture_output=True)


def run_control(host: str, command: str, *, check: bool = True) -> subprocess.CompletedProcess:
    if host == "local":
        return run_process(["bash", "-lc", command], check=check)
    return run_process(["ssh", host, command], check=check)


def run_json(host: str, command: str) -> dict:
    result = run_control(host, command)
    return json.loads(result.stdout)


def app_search(host: str, binary: str, query: str, timeout_ms: int, focused: bool | None) -> dict:
    cmd = f"{binary} server app search set --query {json.dumps(query)} --timeout-ms {timeout_ms}"
    if focused is not None:
        cmd += f" --focus {'on' if focused else 'off'}"
    return run_json(host, cmd)


def app_search_clear(host: str, binary: str, timeout_ms: int) -> dict:
    return run_json(host, f"{binary} server app search clear --timeout-ms {timeout_ms}")


def choose_preview_target(args: argparse.Namespace, inventory: dict, rows_payload: dict) -> dict:
    candidates = choose_session_targets(inventory, rows_payload, random.Random(args.seed), args.count * 2)
    if not candidates:
        raise RuntimeError("no openable sessions available for search lane")
    for candidate in candidates:
        if candidate.get("kind") in {"Document", "Session"}:
            return {
                "full_path": candidate["full_path"],
                "label": candidate["label"],
                "view": "preview",
            }
    return {
        "full_path": candidates[0]["full_path"],
        "label": candidates[0]["label"],
        "view": "preview",
    }


def ensure_preview_open(args: argparse.Namespace, target: dict) -> dict:
    session_path = target["full_path"]
    app_open(args.host, args.bin, session_path, "preview", args.timeout_ms)

    def _has_real_preview_content(state: dict) -> bool:
        viewport = state.get("viewport") or {}
        preview = viewport.get("preview") or {}
        text_sample = (preview.get("text_sample") or "").strip()
        visible_block_count = int(preview.get("visible_block_count") or 0)
        rendered_sections = preview.get("rendered_sections") or []
        if visible_block_count > 0 or bool(rendered_sections):
            return True
        if not text_sample:
            return False
        lowered = text_sample.lower()
        placeholder_markers = (
            "refreshing preview",
            "fetching rendered transcript",
            "preparing the remote preview surface",
            "waiting for transcript hydration",
            "preview unavailable",
        )
        return not any(marker in lowered for marker in placeholder_markers)

    def _probe() -> dict:
        state = app_state(args.host, args.bin, args.timeout_ms)
        if not preview_ready(state, session_path):
            raise ReadinessPending(viewport_failure_reason(state), state)
        if not _has_real_preview_content(state):
            raise ReadinessPending("preview content still placeholder", state)
        return state

    _, state = wait_until("search preview open", 8.0, 0.1, _probe)
    return state


def normalize_words(text: str) -> list[str]:
    return [
        word
        for word in re.findall(r"[A-Za-z0-9][A-Za-z0-9/_\\.-]{2,}", text or "")
        if len(word) >= 4
    ]


def preview_content_lines(state: dict) -> list[str]:
    preview = ((state.get("viewport") or {}).get("preview") or {})
    text_sample = preview.get("text_sample") or ""
    lines: list[str] = []
    for raw_line in text_sample.splitlines():
        line = raw_line.strip().strip("`")
        if len(line) < 4:
            continue
        lowered = line.lower()
        if lowered in {"jojo", "oc", "local"}:
            continue
        if any(
            marker in lowered
            for marker in (
                "refreshing preview",
                "fetching rendered transcript",
                "preparing the remote preview surface",
                "waiting for transcript hydration",
                "preview unavailable",
            )
        ):
            continue
        if re.fullmatch(r"[A-Z][a-z]{2} \d{1,2}, \d{4} .*", line):
            continue
        lines.append(line)
    return lines


def build_sidebar_queries(rows_payload: dict, count: int) -> list[str]:
    queries: list[str] = []
    seen: set[str] = set()
    for row in rows_payload.get("rows") or []:
        label = (row.get("label") or "").strip()
        full_path = (row.get("full_path") or "").strip()
        detail = (row.get("detail_label") or "").strip()
        for source in (label, detail):
            words = normalize_words(source)
            if len(words) >= 2:
                query = " ".join(words[:2])
                lower = query.lower()
                if lower not in seen:
                    seen.add(lower)
                    queries.append(query)
            elif words:
                lower = words[0].lower()
                if lower not in seen:
                    seen.add(lower)
                    queries.append(words[0])
        if full_path:
            path_parts = [part for part in full_path.split("/") if len(part) >= 3]
            for part in path_parts[:2]:
                lower = part.lower()
                if lower not in seen:
                    seen.add(lower)
                    queries.append(part)
        if len(queries) >= count:
            break
    return queries[:count]


def build_content_queries(state: dict, count: int) -> list[str]:
    queries: list[str] = []
    seen: set[str] = set()
    for line in preview_content_lines(state):
        query = line[:64].strip()
        lower = query.lower()
        if lower and lower not in seen:
            seen.add(lower)
            queries.append(query)
        if len(queries) >= count:
            return queries[:count]
    if queries:
        return queries[:count]
    sources = [(((state.get("viewport") or {}).get("preview") or {}).get("text_sample")) or ""]
    for source in sources:
        words = normalize_words(source)
        if len(words) >= 2:
            query = " ".join(words[:2])
            lower = query.lower()
            if lower not in seen:
                seen.add(lower)
                queries.append(query)
        for word in words:
            lower = word.lower()
            if lower not in seen:
                seen.add(lower)
                queries.append(word)
        if len(queries) >= count:
            break
    return queries[:count]


def build_search_cases(args: argparse.Namespace, rows_payload: dict, preview_state: dict) -> list[dict]:
    sidebar_queries = build_sidebar_queries(rows_payload, 10)
    content_queries = build_content_queries(preview_state, 10)
    cases: list[dict] = [{"kind": "sidebar", "query": query} for query in sidebar_queries]
    cases.extend({"kind": "content", "query": query} for query in content_queries)
    cases.extend(
        [
            {"kind": "command", "query": "/preview"},
            {"kind": "command", "query": "/terminal"},
            {"kind": "no_match", "query": "zzzyggtermsearchnomatch"},
        ]
    )
    if len(cases) < args.count:
        filler = sidebar_queries + content_queries or ["jojo", "preview"]
        index = 0
        while len(cases) < args.count:
            cases.append({"kind": "sidebar", "query": filler[index % len(filler)]})
            index += 1
    rng = random.Random(args.seed)
    rng.shuffle(cases)
    return cases[: args.count]


def validate_case(case: dict, state: dict) -> str | None:
    shell = state.get("shell") or {}
    search = state.get("search") or {}
    query = case["query"]
    if shell.get("search_query") != query or search.get("query") != query:
        return "search query did not settle"
    if case["kind"] not in {"clear", "command"} and not search.get("active"):
        return "search unexpectedly inactive"
    if case["kind"] == "sidebar" and (search.get("sidebar_match_count") or 0) <= 0:
        return "sidebar query returned no matches"
    if case["kind"] == "content" and (search.get("content_hit_count") or 0) <= 0:
        return "content query returned no hits"
    if case["kind"] == "command":
        if (search.get("command_suggestion_count") or 0) <= 0:
            return "command query returned no suggestions"
        if (search.get("content_hit_count") or 0) != 0:
            return "command query leaked content hits"
    if case["kind"] == "no_match":
        if (search.get("sidebar_match_count") or 0) != 0 or (search.get("content_hit_count") or 0) != 0:
            return "no-match query unexpectedly matched"
    if (shell.get("notifications_count") or 0) != 0:
        return "search triggered notifications"
    return None


def validate_clear_state(state: dict) -> str | None:
    shell = state.get("shell") or {}
    search = state.get("search") or {}
    if (shell.get("search_query") or "") != "":
        return "clear left a shell search query behind"
    if (search.get("query") or "") != "":
        return "clear left a search query behind"
    if search.get("active"):
        return "clear left search active"
    if (search.get("sidebar_match_count") or 0) != 0:
        return "clear left sidebar matches"
    if (search.get("content_hit_count") or 0) != 0:
        return "clear left content hits"
    if (shell.get("notifications_count") or 0) != 0:
        return "clear triggered notifications"
    return None


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    launch = None
    if args.host == "local" and args.launch_local:
        launch, _ = launch_local_client(args.bin)

    baseline_state = wait_for_app_state(args.host, args.bin, args.timeout_ms)
    baseline_dump = write_state_dump(out_dir, "baseline", baseline_state)
    inventory = server_inventory(args.host)
    rows_payload = wait_for_openable_rows(args, inventory)
    preview_target = choose_preview_target(args, inventory, rows_payload)
    preview_state = ensure_preview_open(args, preview_target)
    preview_dump = write_state_dump(out_dir, "preview_ready", preview_state)

    cases = build_search_cases(args, rows_payload, preview_state)
    results = []
    failures = 0
    clear_failures = 0
    for index, case in enumerate(cases):
        response = app_search(args.host, args.bin, case["query"], args.timeout_ms, True)
        state = ((response.get("data") or {}).get("state")) or app_state(args.host, args.bin, args.timeout_ms)
        state_dump = write_state_dump(out_dir, f"search-{index:02d}", state)
        error = validate_case(case, state)
        if error:
            failures += 1
        clear_response = app_search_clear(args.host, args.bin, args.timeout_ms)
        clear_state = ((clear_response.get("data") or {}).get("state")) or app_state(
            args.host, args.bin, args.timeout_ms
        )
        clear_dump = write_state_dump(out_dir, f"search-{index:02d}-clear", clear_state)
        clear_error = validate_clear_state(clear_state)
        if clear_error:
            clear_failures += 1
        results.append(
            {
                "index": index,
                "kind": case["kind"],
                "query": case["query"],
                "sidebar_match_count": ((state.get("search") or {}).get("sidebar_match_count")),
                "content_hit_count": ((state.get("search") or {}).get("content_hit_count")),
                "command_suggestion_count": (
                    (state.get("search") or {}).get("command_suggestion_count")
                ),
                "error": error,
                "clear_error": clear_error,
                "state_dump": state_dump,
                "clear_dump": clear_dump,
            }
        )

    final_state = wait_for_app_state(args.host, args.bin, args.timeout_ms)
    final_dump = write_state_dump(out_dir, "final", final_state)
    summary = {
        "host": args.host,
        "seed": args.seed,
        "count": args.count,
        "baseline_dump": baseline_dump,
        "preview_target": preview_target,
        "preview_dump": preview_dump,
        "rows_seen": rows_payload.get("row_count"),
        "search_failures": failures,
        "clear_failures": clear_failures,
        "final_dump": final_dump,
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
    return 0 if failures == 0 and clear_failures == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
