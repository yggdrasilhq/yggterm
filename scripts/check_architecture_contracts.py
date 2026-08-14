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
    """Read a contract's subject, or record its absence and keep going.

    ⛔ This used to let `FileNotFoundError` escape, and that turned a moved file
    into an ABORT rather than a failure. On 2026-08-02 the 3.0.0 separation took
    `crates/yggui` out to libyggterm (commit 3a51d499); four assertions still
    named `crates/yggui/src/theme.rs`, so from that day the script died on the
    5th of its 10 check groups and the five after it — including the hot-update
    and GUI-binary contracts — never ran again. Seven days, in CI, on every push.

    A crash is not a failing assertion; it is no assertion at all, and it hides
    every assertion behind it. A contract whose subject has left the repo is an
    UNENFORCED INVARIANT and must say so by name, in the same report as
    everything else. See [[finding-a-red-target-hides-every-test-behind-it]].
    """
    try:
        return (ROOT / path).read_text(encoding="utf-8")
    except FileNotFoundError:
        fail(
            f"{path}: SUBJECT FILE IS MISSING — every contract naming it is "
            "unenforced. Either the file moved (re-point the contract) or it "
            "left the repo (delete the contract and say where the invariant "
            "went); do not leave it dangling."
        )
        return ""


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
    # ⛔ The alpha/grain pin is NOT asserted here any more, and its absence is
    # deliberate rather than an oversight: `crates/yggui` left this repo for
    # libyggterm in the 3.0.0 separation (3a51d499, 2026-08-02) and now arrives
    # as a git dependency pinned by tag. A contract in this repo cannot reach
    # into a pinned dep's source, and the four assertions that tried to were
    # what crashed this whole script for seven days.
    #
    # ⚠ The invariant itself is real and currently UNGUARDED — the constants
    # still hold their values in libyggterm's `crates/yggui/src/theme.rs`, but
    # nothing over there asserts them, so a tag bump could change the stable
    # theme silently. Its guard belongs in libyggterm's own CI; tracked in this
    # repo's queue because this repo is what depends on it.
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
        "const UI_TELEMETRY_RETENTION",
        "append_retained_jsonl_record",
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


def check_generation_context_reads_every_agent_cli() -> None:
    """Nothing that feeds the copy generator may pick a decoder by hand.

    The defect this closes: title/précis/summary generation called the CODEX
    tail reader unconditionally. A Claude Code JSONL shares no record type with
    a Codex rollout, so those reads returned an empty message list — no error,
    no warning — and the summariser wrote a confident timeline entry with
    nothing to write it from. The sidebar then described projects that do not
    exist, on rows the owner works in.

    ⚠ The failure mode is what makes a static contract worth having here: a
    wrong decoder does not throw, it returns EMPTY. Nothing downstream can tell
    "this session said nothing" from "I cannot read this file", so the mistake
    is invisible at runtime and only a rule about the CALL SITE catches it.
    """
    single_cli_readers = (
        "read_codex_transcript_messages",
        "read_codex_transcript_messages_limited",
        "read_codex_transcript_messages_tail_limited",
        "read_codex_transcript_entries",
        "read_claude_code_transcript_messages",
        "read_claude_code_transcript_entries",
    )
    # Each producer, and the function inside it that must stay CLI-agnostic.
    producers = [
        ("crates/yggterm-core/src/titles.rs", "fn extract_tail_context("),
        ("crates/yggterm-server/src/lib.rs", "fn remote_summary_for_path("),
        ("crates/yggterm-server/src/lib.rs", "pub fn run_remote_generation_context("),
    ]
    for path, marker in producers:
        text = read(path)
        if not text:
            continue
        if marker not in text:
            fail(
                f"{path}: {marker!r} is gone — the contract that its generation "
                "context reads every agent CLI is now unenforced. Re-point it at "
                "the function that took over, or delete it and say where the "
                "invariant went."
            )
            continue
        body = text.split(marker, 1)[1].split("\nfn ", 1)[0].split("\npub fn ", 1)[0]
        # Coverage floor: a split that captured nothing makes the rest vacuous.
        if len(body) < 80:
            fail(f"{path}: could not capture the body of {marker!r} ({len(body)} bytes)")
            continue
        for reader in single_cli_readers:
            if re.search(rf"\b{reader}\s*\(", body):
                fail(
                    f"{path}: {marker!r} calls {reader} — generation context must "
                    "go through read_agent_transcript_messages* so a session whose "
                    "CLI the caller did not think of yields its own words rather "
                    "than an empty string."
                )


def check_the_preview_excerpt_is_the_last_context_source() -> None:
    """The scan's preview excerpt may not be the FIRST thing a summary is written from.

    `remote_context` is the machine scan's one-line row preview, built from a
    12-message tail. Measured on a live host it was **120 to 243 bytes** — the
    opening sentence of the session's first message. It used to be returned
    before the transcript was even considered, so remote rows were summarised
    from one real sentence, and the model kept that sentence and invented an
    objective, a result and a blocker to sit around it. That is the exact shape
    the reported summaries had.

    ⚠ It stays as a LAST resort on purpose — a row with no readable transcript
    is better served by a thin hint than by nothing, and the generator's own
    floor refuses anything too thin before it reaches a model.
    """
    text = read("crates/yggterm-shell/src/shell.rs")
    if not text:
        return
    marker = "fn generation_context_for_target("
    if marker not in text:
        fail(
            "crates/yggterm-shell/src/shell.rs: "
            f"{marker!r} is gone — the rule that the transcript outranks the "
            "preview excerpt is now unenforced. Re-point it, or delete it and "
            "say where the invariant went."
        )
        return
    body = text.split(marker, 1)[1].split("\nfn ", 1)[0]
    if "fetch_remote_generation_context(" not in body:
        fail(
            "crates/yggterm-shell/src/shell.rs: could not capture the body of "
            f"{marker!r} ({len(body)} bytes)"
        )
        return
    first_excerpt = body.find("remote_context")
    transcript = body.find("fetch_remote_generation_context(")
    if first_excerpt < transcript:
        fail(
            "crates/yggterm-shell/src/shell.rs: generation_context_for_target "
            "reads remote_context before it tries the transcript. That excerpt "
            "is a 120-byte row preview; summarising from it is what produced "
            "confident descriptions of projects that do not exist."
        )


def check_mirror_never_outranks_a_scan_on_the_summary() -> None:
    """The remote-metadata mirror may not overwrite a scanned summary.

    The scan reads the owning host's title store — the one owner of "does this
    session have a summary" — so its answer wins, including when the answer is
    "none". The mirror is a cache of that.

    ⚠ This is the second field to need the rule. The title had it first, and the
    comment in `overlay_mirrored_remote_sessions` records why: mirror-wins froze
    every Claude Code title at its first scanned value, because the pass
    re-mirrors after overlaying and writes the stale value straight back. The
    summary sat one field below with the bug still in it, and there it was worse
    — a cached summary GATES its own regeneration, so a wrong one suppressed the
    work that would have replaced it, and deleting it from the store could not
    reach the screen.
    """
    text = read("crates/yggterm-server/src/lib.rs")
    if not text:
        return
    marker = "fn overlay_mirrored_remote_sessions("
    if marker not in text:
        fail(
            "crates/yggterm-server/src/lib.rs: "
            f"{marker!r} is gone — the rule that a scanned summary outranks the "
            "mirror is now unenforced. Re-point it, or delete it and say where "
            "the invariant went."
        )
        return
    body = text.split(marker, 1)[1].split("\nfn ", 1)[0]
    # Coverage floor: a split that captured nothing makes the rest vacuous.
    if "mirrored_by_id" not in body:
        fail(
            "crates/yggterm-server/src/lib.rs: could not capture the body of "
            f"{marker!r} ({len(body)} bytes)"
        )
        return
    if re.search(r"session\.cached_summary\s*=", body):
        fail(
            "crates/yggterm-server/src/lib.rs: overlay_mirrored_remote_sessions "
            "assigns session.cached_summary from the mirror. The scan read the "
            "owning host's title store and is the SSOT — including when it says "
            "there is none. Mirror-wins makes a deleted summary immortal, "
            "because the pass re-mirrors what it just overlaid."
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


def check_gui_binary_resolution_contract() -> None:
    """The GUI-launcher scripts must not read YGGTERM_BIN as a launch target.

    YGGTERM_BIN is the DAEMON's own executable (it exports it into every PTY it
    owns), so inside any daemon-owned row it names `yggterm-headless` — a build
    with no GUI. `shadow-client.sh` defaulting through it made the agent-first
    test surface unusable for every in-session agent, which pushed agents onto
    the user's live GUI (docs/pending-bugs.md, J8a/J8b).

    Locked here rather than in a unit test because the failure is in shell
    scripts, and because the probe those scripts use is only sound while the two
    Rust help printers keep disagreeing about `install`.
    """
    owner = "scripts/lib/gui-binary.sh"
    require_contains(
        owner,
        "yggterm_resolve_gui_binary",
        "the one owner of GUI-binary resolution must define the resolver",
    )
    for launcher in ["scripts/shadow-client.sh", "scripts/underglass-sandbox.sh"]:
        text = read(launcher)
        if "lib/gui-binary.sh" not in text:
            fail(
                f"{launcher}: must source {owner} rather than resolving a GUI "
                "binary of its own"
            )
        for banned in ['"$YGGTERM_BIN"', "${YGGTERM_BIN:-$", "$YGGTERM_BIN "]:
            if banned in text:
                fail(
                    f"{launcher}: reads YGGTERM_BIN as a launch target ({banned!r}). "
                    "That variable is the daemon's own exe — headless in every "
                    "daemon-owned row, and suffixed ' (deleted)' after a hot "
                    "restart. Use yggterm_resolve_gui_binary."
                )

    # The probe is a text discriminator against the binaries' own `--help`, so
    # it is only sound while `install` stays a GUI-only top-level command. If
    # this ever fails, FIX THE PROBE — do not delete the check, because a probe
    # that answers "GUI" for a headless build reopens the original bug.
    require_regex(
        "scripts/lib/gui-binary.sh",
        r"grep -qE '\^\[\[:space:\]\]\+\[\^\[:space:\]\]\+ install\$'",
        "the GUI probe must match the help line it was written against",
    )
    require_regex(
        "apps/yggterm/src/main.rs",
        r"^  yggterm install$",
        "the GUI's main help must keep the line scripts/lib/gui-binary.sh probes for",
    )
    headless_help = read("apps/yggterm/src/bin/yggterm-headless.rs")
    if re.search(r"^  yggterm-headless install$", headless_help, flags=re.MULTILINE):
        fail(
            "apps/yggterm/src/bin/yggterm-headless.rs: the headless help now "
            "advertises `install`, which makes scripts/lib/gui-binary.sh unable "
            "to tell the two builds apart — pick a new discriminator in the same "
            "change"
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
    check_generation_context_reads_every_agent_cli()
    check_mirror_never_outranks_a_scan_on_the_summary()
    check_the_preview_excerpt_is_the_last_context_source()
    check_terminal_retained_replay_policy_contract()
    check_gui_binary_resolution_contract()
    if FAILURES:
        for failure in FAILURES:
            print(f"ARCHITECTURE CONTRACT FAILED: {failure}", file=sys.stderr)
        return 1
    print("architecture contracts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
