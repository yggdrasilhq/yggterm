#!/usr/bin/env python3
"""Static source-of-truth contract checks for stable Yggterm.

This script intentionally checks only deterministic architecture invariants.
It is not a substitute for smoke tests, app-control probes, telemetry queries,
or screenshots. It exists to stop the shortcut classes recorded in
docs/architecture-audit-2026-05-16.md from quietly returning.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FAILURES: list[str] = []


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def fail(message: str) -> None:
    FAILURES.append(message)


def require_contains(path: str, needle: str, reason: str) -> None:
    text = read(path)
    if needle not in text:
        fail(f"{path}: missing {needle!r} ({reason})")


def require_regex(path: str, pattern: str, reason: str) -> None:
    text = read(path)
    if not re.search(pattern, text, flags=re.MULTILINE | re.DOTALL):
        fail(f"{path}: missing pattern {pattern!r} ({reason})")


def js_arrow_function_body(text: str, name: str) -> str | None:
    marker = f"const {name} = "
    start = text.find(marker)
    if start < 0:
        return None
    brace = text.find("{", start)
    if brace < 0:
        return None
    depth = 0
    for index in range(brace, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[brace + 1 : index]
    return None


def check_doc_cross_links() -> None:
    audit = ROOT / "docs/architecture-audit-2026-05-16.md"
    if not audit.exists():
        fail("docs/architecture-audit-2026-05-16.md: audit document is required")
        return
    audit_text = read("docs/architecture-audit-2026-05-16.md")
    for heading in [
        "## Authority Table",
        "## Failure Answers",
        "## Shortcut Classes To Ban",
        "## Required Investigation Order",
        "## Stable Release Gate",
    ]:
        if heading not in audit_text:
            fail(f"docs/architecture-audit-2026-05-16.md: missing {heading}")
    for path in [
        "AGENTS.md",
        "DESIGN.md",
        "docs/xterm.md",
        "docs/protocol.md",
        "docs/sessions.md",
        "docs/theme.md",
        "docs/telemetry.md",
    ]:
        require_contains(
            path,
            "docs/architecture-audit-2026-05-16.md",
            "canonical contracts must point to the source-of-truth audit",
        )


def check_agents_operating_law() -> None:
    require_contains(
        "AGENTS.md",
        "Before fixing any regression, name the authoritative source of truth",
        "future fixes must start from ownership, not symptoms",
    )
    require_contains(
        "AGENTS.md",
        "Never promote an observer into product truth",
        "app-control/telemetry/screenshots are witnesses only",
    )
    require_contains(
        "AGENTS.md",
        "Do not patch a symptom by adding a second source of truth",
        "shortcut classes must be banned at operator level",
    )
    require_contains(
        "AGENTS.md",
        "~/.tmp/yggterm",
        "live incident artifacts must not fill /tmp and pressure swap",
    )
    if "/tmp/yggterm-incident.jsonl" in read("AGENTS.md"):
        fail("AGENTS.md: live incident command still points at /tmp")


def check_lossless_terminal_write_contract() -> None:
    shell = read("crates/yggterm-shell/src/shell.rs")
    bridge = read("crates/yggterm-shell/src/terminal_write_bridge.rs")
    policy = read("crates/yggterm-shell/src/terminal_write_policy.rs")
    require_regex(
        "crates/yggterm-shell/src/terminal_write_policy.rs",
        r"fn coalesce_high_volume_terminal_frames\(data: &str\) -> String\s*\{\s*data\.to_string\(\)\s*\}",
        "Rust test helper must stay lossless",
    )
    require_regex(
        "crates/yggterm-shell/src/terminal_write_policy.rs",
        r"fn trim_high_volume_terminal_frame_buffer\(data: &str\) -> String\s*\{\s*data\.to_string\(\)\s*\}",
        "Rust frame trim helper must stay lossless",
    )
    for name in [
        "terminal_write_should_frame_budget",
        "terminal_output_is_high_volume_frame_like",
        "terminal_output_is_inline_status_rewrite_frame",
    ]:
        if re.search(
            rf"^\s*(?:pub\(crate\)\s+)?fn {name}\(",
            shell,
            flags=re.MULTILINE,
        ):
            fail(
                "crates/yggterm-shell/src/shell.rs: "
                f"{name} must live in terminal_write_policy.rs"
            )
        if f"fn {name}(" not in policy:
            fail(
                "crates/yggterm-shell/src/terminal_write_policy.rs: "
                f"missing {name}"
            )
    for name in ["coalesceSynchronizedOutputFrames", "coalesceHighVolumeTerminalPayload"]:
        body = js_arrow_function_body(shell, name)
        if body is None:
            fail(f"crates/yggterm-shell/src/shell.rs: missing JS helper {name}")
            continue
        forbidden = ["slice(", "substring(", "substr(", ".pop(", ".shift(", ".sort("]
        for marker in forbidden:
            if marker in body:
                fail(
                    "crates/yggterm-shell/src/shell.rs: "
                    f"{name} contains {marker!r}; PTY write batching must be lossless"
                )
    require_contains(
        "crates/yggterm-shell/src/terminal_write_bridge.rs",
        "self.pending.push_str(&data);",
        "terminal write bridge must append PTY bytes in order",
    )
    require_contains(
        "crates/yggterm-shell/src/terminal_write_bridge.rs",
        "std::mem::take(&mut self.pending)",
        "terminal write bridge must flush the exact pending byte string",
    )
    for marker in [
        ".truncate(",
        ".drain(",
        ".replace(",
        ".retain(",
        ".split_off(",
        ".remove(",
        ".pop(",
    ]:
        if marker in bridge:
            fail(
                "crates/yggterm-shell/src/terminal_write_bridge.rs: "
                f"contains {marker!r}; PTY write staging must not rewrite pending bytes"
            )
    require_regex(
        "docs/xterm.md",
        r"must never drop,\s+reorder,\s+deduplicate,\s+trim,\s+or rewrite PTY bytes",
        "terminal byte fidelity must be documented",
    )
    require_contains(
        "docs/xterm.md",
        "crates/yggterm-shell/src/terminal_write_policy.rs",
        "terminal write policy module must be documented",
    )
    require_contains(
        "docs/xterm.md",
        "crates/yggterm-shell/src/terminal_write_bridge.rs",
        "terminal write bridge module must be documented",
    )


def check_no_release_terminal_overlay_substitution() -> None:
    require_regex(
        "crates/yggterm-shell/src/shell.rs",
        r"const terminalSessionAllowsLowPowerTui = \(\) => \{\{\s*return false;\s*\}\};",
        "low-power TUI text overlay must stay disabled in stable builds",
    )
    require_contains(
        "docs/xterm.md",
        "shell-owned overlays",
        "terminal overlay prohibition must stay documented",
    )
    require_contains(
        "DESIGN.md",
        "Do not cover terminal defects with Yggterm-owned decorative layers",
        "design law must reject screenshot repair",
    )


def check_stable_theme_contract() -> None:
    require_regex(
        "crates/yggterm-shell/src/theme_contract.rs",
        r"fn shell_css_backdrop_filter_enabled\(\) -> bool\s*\{\s*false\s*\}",
        "stable shell must not enable CSS backdrop blur",
    )
    require_regex(
        "crates/yggterm-shell/src/theme_contract.rs",
        r"fn shell_live_blur_supported\(\) -> bool\s*\{\s*false\s*\}",
        "stable shell must not enable live blur",
    )
    require_regex(
        "crates/yggterm-shell/src/theme_contract.rs",
        r"fn shell_full_window_css_blur_enabled\(\) -> bool\s*\{\s*false\s*\}",
        "stable shell must not enable full-window CSS blur",
    )
    require_regex(
        "crates/yggterm-shell/src/theme_contract.rs",
        r"fn linux_compositor_blur_active_for_app_control\(\) -> bool\s*\{\s*false\s*\}",
        "stable app-control must report compositor blur inactive",
    )
    require_contains(
        "crates/yggui/src/theme.rs",
        "const STABLE_THEME_ALPHA: f32 = 0.96;",
        "stable alpha must be pinned",
    )
    require_contains(
        "crates/yggui/src/theme.rs",
        "const STABLE_THEME_GRAIN: f32 = 0.0;",
        "stable grain must be pinned",
    )
    require_contains(
        "crates/yggui/src/theme.rs",
        "next.alpha = STABLE_THEME_ALPHA;",
        "saved alpha must clamp before rendering",
    )
    require_contains(
        "crates/yggui/src/theme.rs",
        "next.grain = STABLE_THEME_GRAIN;",
        "saved grain must clamp before rendering",
    )
    for needle in [
        "live_blur_supported=false",
        "css_backdrop_filter_enabled=false",
        "compositor_blur_active=false",
        "material_blur_px=0",
    ]:
        require_contains("docs/theme.md", needle, "stable theme observability contract")


def check_hot_update_contract() -> None:
    shell = read("crates/yggterm-shell/src/shell.rs")
    policy = read("crates/yggterm-shell/src/hot_update_policy.rs")
    for name in [
        "startup_daemon_hot_swap_reason",
        "startup_stale_daemon_hot_swap_target",
        "daemon_update_state_json",
    ]:
        if re.search(
            rf"^\s*(?:pub\(crate\)\s+)?fn {name}\(",
            shell,
            flags=re.MULTILINE,
        ):
            fail(
                "crates/yggterm-shell/src/shell.rs: "
                f"{name} must live in hot_update_policy.rs"
            )
        if not re.search(rf"fn {name}(?:<[^>]+>)?\(", policy):
            fail(f"crates/yggterm-shell/src/hot_update_policy.rs: missing {name}")
    require_contains(
        "docs/protocol.md",
        "crates/yggterm-shell/src/hot_update_policy.rs",
        "hot-update policy module must be documented",
    )


def check_ui_telemetry_contract() -> None:
    shell = read("crates/yggterm-shell/src/shell.rs")
    telemetry = read("crates/yggterm-shell/src/ui_telemetry.rs")
    for needle in [
        "const UI_TELEMETRY_FILENAME",
        "const UI_TELEMETRY_ROTATED_FILENAME",
        "const UI_TELEMETRY_MAX_BYTES",
        "append_bounded_jsonl_record",
    ]:
        if needle in shell:
            fail(
                "crates/yggterm-shell/src/shell.rs: "
                f"{needle} must live in ui_telemetry.rs"
            )
        if needle not in telemetry:
            fail(f"crates/yggterm-shell/src/ui_telemetry.rs: missing {needle}")
    require_contains(
        "crates/yggterm-shell/src/shell.rs",
        "ui_telemetry_should_record(&mut self.recent_ui_telemetry",
        "shell telemetry method must use shared throttle policy",
    )
    require_contains(
        "crates/yggterm-shell/src/shell.rs",
        "append_ui_telemetry_event(event, payload)",
        "shell telemetry method must use shared append policy",
    )
    require_contains(
        "docs/telemetry.md",
        "crates/yggterm-shell/src/ui_telemetry.rs",
        "ui telemetry owner module must be documented",
    )


def check_session_copy_policy_contract() -> None:
    shell = read("crates/yggterm-shell/src/shell.rs")
    policy = read("crates/yggterm-shell/src/session_copy_policy.rs")
    for name in [
        "env_copy_generation_enabled",
        "copy_generation_start_allowed",
        "humanized_terminal_title",
        "title_looks_like_abbreviated_shell_label",
        "title_is_low_signal_for_copy",
        "title_needs_generation_from_visible_titles",
    ]:
        if re.search(
            rf"^\s*(?:pub\(crate\)\s+)?fn {name}\(",
            shell,
            flags=re.MULTILINE,
        ):
            fail(
                "crates/yggterm-shell/src/shell.rs: "
                f"{name} must live in session_copy_policy.rs"
            )
        if not re.search(rf"fn {name}\(", policy):
            fail(f"crates/yggterm-shell/src/session_copy_policy.rs: missing {name}")
    require_contains(
        "docs/sessions.md",
        "crates/yggterm-shell/src/session_copy_policy.rs",
        "session copy policy module must be documented",
    )


def check_terminal_retained_replay_policy_contract() -> None:
    shell = read("crates/yggterm-shell/src/shell.rs")
    policy = read("crates/yggterm-shell/src/terminal_retained_replay_policy.rs")
    for name in [
        "retained_ready_remote_host_should_reuse_bootstrap",
        "retained_rehydrate_identity_key",
        "retained_ready_remote_host_rehydrate_mode",
        "daemon_retained_snapshot_replay_identity_key",
        "daemon_retained_snapshot_replay_should_start",
    ]:
        if re.search(
            rf"^\s*(?:pub\(crate\)\s+)?fn {name}\(",
            shell,
            flags=re.MULTILINE,
        ):
            fail(
                "crates/yggterm-shell/src/shell.rs: "
                f"{name} must live in terminal_retained_replay_policy.rs"
            )
        if not re.search(rf"fn {name}\(", policy):
            fail(
                "crates/yggterm-shell/src/terminal_retained_replay_policy.rs: "
                f"missing {name}"
            )
    require_contains(
        "docs/xterm.md",
        "crates/yggterm-shell/src/terminal_retained_replay_policy.rs",
        "retained terminal replay policy module must be documented",
    )


def main() -> int:
    check_doc_cross_links()
    check_agents_operating_law()
    check_lossless_terminal_write_contract()
    check_no_release_terminal_overlay_substitution()
    check_stable_theme_contract()
    check_hot_update_contract()
    check_ui_telemetry_contract()
    check_session_copy_policy_contract()
    check_terminal_retained_replay_policy_contract()
    if FAILURES:
        for failure in FAILURES:
            print(f"ARCHITECTURE CONTRACT FAILED: {failure}", file=sys.stderr)
        return 1
    print("architecture contracts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
