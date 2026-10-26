#!/usr/bin/env python3
"""startup-evidence audit runner — swarm-harness2 turn-6 unit 1.

Launches a yggterm GUI binary hermetically (scratch HOME/XDG_DATA_HOME under
Xvfb + dbus-run-session, the mergekey-rig launch shape), collects the F11
startup evidence (panic signatures, component_window events, daemon
readiness, screenshot), proves WHICH binary actually served the run
([11.231]: the launcher may silently hand off to the direct build — the
bundle carries the trace's own executable_path plus pre/post hashes), and
emits an immutable evidence bundle with a PASS / FAIL / UNKNOWN verdict.

Laws this runner encodes:
  - [11.231]: a rig that launches a non-installed binary without
    redirecting XDG_DATA_HOME measures the installed build — exe-proof comes
    from the run's own trace, never from the path you passed.
  - sol turn 6 / Q3: missing evidence never produces PASS; an instrument
    fault caps the verdict at UNKNOWN; identity mismatch certifies nothing.
  - Verdict scope: hermetic environments certify the ARTIFACT. The owner's
    real-home production profile is deeptest territory (owner-present), not
    this runner's.

Usage:
  python3 tools/audit/startup_evidence.py                      # 1 hermetic small-world trial of the installed build
  python3 tools/audit/startup_evidence.py --binary /path/to/yggterm-headless --trials 3
  python3 tools/audit/startup_evidence.py --selftest           # pure decision tests + live fail/mismatch controls

Stdlib only, no repo imports.
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import time
import uuid
import zlib
from datetime import datetime, timezone
from pathlib import Path

SCHEMA = "startup-evidence-v1"
DEFAULT_BINARY = os.path.expanduser("~/.local/bin/yggterm")  # the GUI binary, not the headless CLI
DEFAULT_OUT_ROOT = os.path.expanduser("~/.local/state/yggterm-audit")
DEFAULT_DISPLAY = ":87"  # mergekey rig owns :84 — never fight it for a display
READY_WAIT_S = 90
SETTLE_S = 6
SMALL_WORLD_SESSIONS = 8

PANIC_PATTERNS = (
    re.compile(r"Encountered panic", re.IGNORECASE),
    re.compile(r"Panic while rendering", re.IGNORECASE),
)

# ---------------------------------------------------------------------------
# Pure decision layer (unit-tested by --selftest; no I/O in these functions)
# ---------------------------------------------------------------------------


def scan_panic_lines(text: str) -> list[str]:
    """GUI-log lines carrying a render/startup panic signature."""
    hits = []
    for line in text.splitlines():
        if any(p.search(line) for p in PANIC_PATTERNS):
            hits.append(line.strip()[:400])
    return hits


def panic_trace_events(home: str) -> list[dict]:
    """Structured panic witnesses from the run's own event trace.

    The F11 render panic never reaches stderr — the GUI's render error
    handler paints `Encountered panic: Any { .. }` on screen and swallows
    it — but the panic hook records category=panic events with the payload
    the type-erased overlay hides.
    """
    events = []
    for path in trace_files(home):
        with open(path, errors="replace") as fh:
            for line in fh:
                if '"category": "panic"' not in line and '"category":"panic"' not in line:
                    continue
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = event.get("payload") or {}
                events.append(
                    {
                        "name": event.get("name"),
                        "location": str(payload.get("location") or "")[:200],
                        "message": str(payload.get("payload") or payload.get("message") or "")[:400],
                    }
                )
    return events


# --- visual judge: pure PNG decode + pixel classification -------------------
#
# The first baseline proved the runner blind without this: the render panic
# is PAINTED (red monospace on black) and never reaches the log. The judge
# can only add failure knowledge or withhold PASS — above the healthy floor
# it says nothing, and the indeterminate band caps at UNKNOWN.


def decode_png_rgb(data: bytes, stride: int = 4) -> list[tuple[int, int, int]]:
    """Decode an 8-bit RGB/RGBA PNG into strided RGB samples. Pure stdlib."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    pos, idat = 8, b""
    w = h = 0
    bd = ct = None
    while pos + 8 <= len(data):
        ln = struct.unpack(">I", data[pos : pos + 4])[0]
        typ = data[pos + 4 : pos + 8]
        if typ == b"IHDR":
            w, h, bd, ct = struct.unpack(">IIBB", data[pos + 8 : pos + 18])
        elif typ == b"IDAT":
            idat += data[pos + 8 : pos + 8 + ln]
        elif typ == b"IEND":
            break
        pos += 12 + ln
    if bd != 8 or ct not in (2, 6):
        raise ValueError(f"unsupported PNG shape: bitdepth={bd} colortype={ct}")
    ch = 3 if ct == 2 else 4
    raw = zlib.decompress(idat)
    row_stride = w * ch

    def paeth(a: int, b: int, c: int) -> int:
        p = a + b - c
        pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
        return a if pa <= pb and pa <= pc else (b if pb <= pc else c)

    samples: list[tuple[int, int, int]] = []
    prev = bytearray(row_stride)
    pos = 0
    for y in range(h):
        f = raw[pos]
        pos += 1
        line = bytearray(raw[pos : pos + row_stride])
        pos += row_stride
        if f == 1:
            for i in range(ch, row_stride):
                line[i] = (line[i] + line[i - ch]) & 255
        elif f == 2:
            for i in range(row_stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(row_stride):
                a = line[i - ch] if i >= ch else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(row_stride):
                a = line[i - ch] if i >= ch else 0
                c = prev[i - ch] if i >= ch else 0
                line[i] = (line[i] + paeth(a, prev[i], c)) & 255
        if y % stride == 0:
            for x in range(0, w, stride):
                o = x * ch
                samples.append((line[o], line[o + 1], line[o + 2]))
        prev = line
    if not samples:
        raise ValueError("decoded no pixels")
    return samples


def classify_pixels(samples: list[tuple[int, int, int]]) -> dict:
    total = len(samples)
    content = sum(1 for r, g, b in samples if (r > 150 and g < 90 and b < 90) is False and (r > 40 or g > 40 or b > 40))
    red = sum(1 for r, g, b in samples if r > 150 and g < 90 and b < 90)
    return {"content": content / total, "red": red / total}


def visual_verdict(fractions: dict) -> str:
    """panic_visual | blank_visual | healthy | indeterminate.

    Calibrated on the 2026-10-26 installed-build panic frame
    (content=0.000000, red=0.000393): the panic overlay is a red monospace
    band on an otherwise pure-black window. A healthy dark-theme GUI still
    carries sidebar text, chrome and borders — content well above the
    healthy floor. The indeterminate band never passes.
    """
    content, red = fractions["content"], fractions["red"]
    if red > 0.00005 and content < 0.004:
        return "panic_visual"
    if content < 0.0008:
        return "blank_visual"
    if content >= 0.01:
        return "healthy"
    return "indeterminate"


def file_sha256(path: str | None) -> str | None:
    if not path or not os.path.isfile(path):
        return None
    h = hashlib.sha256()
    try:
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 16), b""):
                h.update(chunk)
    except OSError:
        return None
    return h.hexdigest()


def decide(obs: dict) -> dict:
    """Compute the verdict from an observation dict. Never I/O.

    Required obs keys:
      daemon_ready, ready_after_s, panic_lines (list), panic_trace (list),
      component_window_count, identity_ok (bool|None), identity_reasons
      (list), screenshot_captured, screenshot_unsupported, visual (dict with
      verdict field, or None), instrument_faults (list)
    Verdicts:
      UNKNOWN  instrument fault, a missing required leg (missing evidence
               never PASS), or an indeterminate visual frame
      FAIL     the product demonstrably failed its startup contract
      PASS     every leg observed green
    """
    reasons: list[str] = []
    verdict = "PASS"

    if obs.get("instrument_faults"):
        return {
            "verdict": "UNKNOWN",
            "reasons": [f"instrument fault: {f}" for f in obs["instrument_faults"]],
            "product_failure_observed": False,
        }

    if not obs.get("daemon_ready"):
        reasons.append(
            f"product never became ready within {obs.get('ready_wait_s', READY_WAIT_S)}s"
        )
        verdict = "FAIL"

    for line in obs.get("panic_lines", []):
        reasons.append(f"panic signature in gui log: {line}")
        verdict = "FAIL"

    for event in obs.get("panic_trace", []):
        reasons.append(
            f"panic in run trace: {event.get('name')} at {event.get('location') or '?'}"
            f" — {event.get('message')[:160]}"
        )
        verdict = "FAIL"

    if obs.get("component_window_count", 0) == 0:
        reasons.append(
            "zero component_window events — the F11 DOM-empty signature "
            "(first Root render died; sidebar never drew)"
        )
        verdict = "FAIL"

    visual = obs.get("visual")
    if visual is not None:
        v = visual.get("verdict")
        if v in ("panic_visual", "blank_visual"):
            reasons.append(
                f"visual frame reads {v} "
                f"(content={visual['fractions']['content']:.6f}, red={visual['fractions']['red']:.6f})"
            )
            verdict = "FAIL"
        elif v == "indeterminate":
            reasons.append("visual frame indeterminate — capped at UNKNOWN")
            if verdict == "PASS":
                verdict = "UNKNOWN"
    elif obs.get("screenshot_captured"):
        reasons.append(obs.get("visual_note") or "visual frame not judged — capped at UNKNOWN")
        if verdict == "PASS":
            verdict = "UNKNOWN"

    # Identity and the visual leg gate PASS only: unproven identity can never
    # upgrade a run to PASS, but it also cannot launder a demonstrable product
    # failure (above) into UNKNOWN — the failure stands on its own observations.
    identity_ok = obs.get("identity_ok")
    if identity_ok is not True:
        reasons.extend(obs.get("identity_reasons", ["artifact identity unproven"]))
        if verdict == "PASS":
            verdict = "UNKNOWN"

    if not obs.get("screenshot_captured"):
        if obs.get("screenshot_unsupported"):
            reasons.append("visual leg unavailable (capture unsupported) — capped at UNKNOWN")
        else:
            reasons.append("visual leg missing (screenshot capture failed) — capped at UNKNOWN")
        if verdict == "PASS":
            verdict = "UNKNOWN"

    if verdict == "PASS":
        reasons.append(
            f"ready in {obs.get('ready_after_s')}s, {obs.get('component_window_count')} "
            "component_window events, no panic signature, identity proven, screenshot captured"
        )
    return {"verdict": verdict, "reasons": reasons, "product_failure_observed": verdict == "FAIL"}


# ---------------------------------------------------------------------------
# Trace readers (the same event-trace.jsonl glob idiom as tools/uxspeed/mergekey_rig.py)
# ---------------------------------------------------------------------------


def trace_files(home: str) -> list[str]:
    return sorted(glob.glob(os.path.join(home, "**", "event-trace.jsonl"), recursive=True))


def served_executable_path(home: str) -> str | None:
    for path in trace_files(home):
        with open(path, errors="replace") as fh:
            for line in fh:
                if "executable_path" not in line:
                    continue
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = event.get("payload") or {}
                exe = payload.get("executable_path") or event.get("executable_path")
                if exe:
                    return exe
    return None


def count_trace_events(home: str, name: str) -> int:
    count = 0
    for path in trace_files(home):
        with open(path, errors="replace") as fh:
            for line in fh:
                if f'"name": "{name}"' not in line and f'"name":"{name}"' not in line:
                    continue
                try:
                    if json.loads(line).get("name") == name:
                        count += 1
                except json.JSONDecodeError:
                    continue
    return count


def trace_excerpt(home: str, max_lines: int = 200) -> list[str]:
    """The lines the verdict relied on: register/identity + component_window + head."""
    keep: list[str] = []
    for path in trace_files(home):
        with open(path, errors="replace") as fh:
            for i, line in enumerate(fh):
                if (
                    i < 20
                    or "executable_path" in line
                    or '"component_window"' in line
                    or "panic" in line.lower()
                ):
                    keep.append(line.rstrip("\n")[:600])
    return keep[:max_lines]


# ---------------------------------------------------------------------------
# Harness
# ---------------------------------------------------------------------------


def stage_small_world(home_dir: str, sessions: int, tag: str) -> str:
    """Stage a tiny codex store inside the scratch home (mergekey-rig shape)."""
    base = os.path.join(home_dir, ".codex", "sessions", "2026", "01", "02", tag)
    os.makedirs(base, exist_ok=True)
    cwds = [f"/home/pi/proj-{c}" for c in "abcdefgh"]
    for ix in range(sessions):
        sid = str(uuid.uuid4())
        body = (
            json.dumps({"type": "session_meta", "payload": {"id": sid, "cwd": cwds[ix % len(cwds)]}})
            + "\n"
            + json.dumps(
                {"type": "event_msg", "payload": {"type": "user_message", "message": f"audit stage {ix}"}}
            )
            + "\n"
        )
        with open(os.path.join(base, f"rollout-{sid}.jsonl"), "w") as fh:
            fh.write(body)
    return base


def wait_ready(binary: str, deadline_s: int, env: dict) -> tuple[bool, int]:
    start = time.time()
    while time.time() - start < deadline_s:
        try:
            probe = subprocess.run(
                [binary, "server", "app", "clients"],
                capture_output=True, timeout=15, env=env,
            )
        except (subprocess.TimeoutExpired, OSError):
            probe = None  # a hung or unusable probe is "not ready yet", not a crash
        if probe is not None and probe.returncode == 0:
            return True, int(time.time() - start)
        time.sleep(1)
    return False, deadline_s


def capture_screenshot(binary: str, out_png: str, env: dict) -> tuple[bool, bool]:
    """Returns (captured, unsupported). Unsupported must be the server's own word."""
    try:
        proc = subprocess.run(
            [binary, "server", "app", "screenshot", out_png],
            capture_output=True, text=True, timeout=45, env=env,
        )
    except (subprocess.TimeoutExpired, OSError):
        return False, False
    if proc.returncode == 0 and os.path.isfile(out_png) and os.path.getsize(out_png) > 0:
        return True, False
    combined = (proc.stdout + proc.stderr).lower()
    return False, "unsupported" in combined or "not supported" in combined


def check_instruments() -> list[str]:
    faults = []
    for tool in ("Xvfb", "dbus-run-session"):
        if not shutil.which(tool):
            faults.append(f"{tool} not on PATH")
    if not os.path.exists("/tmp/.X11-unix") and not glob.glob("/tmp/.X11-unix/X*"):
        pass  # Xvfb creates its own socket; absence beforehand is fine
    return faults


def run_trial(binary: str, args, attempt_id: str, out_dir: Path) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)
    faults = check_instruments()
    manifest: dict = {
        "schema": SCHEMA,
        "attempt_id": attempt_id,
        "started_at_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "binary": {"declared": os.path.realpath(binary), "launcher_sha256": file_sha256(binary)},
        "environment": {},
        "observations": {},
        "identity": {},
        "instrument_faults": faults,
        "verdict": None,
        "reasons": [],
        "bundle_files": {},
    }
    if faults:
        manifest["verdict"] = "UNKNOWN"
        manifest["reasons"] = [f"instrument fault: {f}" for f in faults]
        return manifest

    stamp = attempt_id
    scratch = f"/tmp/audit-se-{stamp}"
    home_dir = os.path.join(scratch, "home")
    os.makedirs(home_dir, exist_ok=True)
    stage_small_world(home_dir, SMALL_WORLD_SESSIONS, f"audit-{stamp}")
    display = args.display
    xvfb = subprocess.Popen(
        ["Xvfb", display, "-screen", "0", "1600x1000x24", "-ac"],
        stdout=open(out_dir / "xvfb.log", "w"), stderr=subprocess.STDOUT,
        start_new_session=True,  # own process group — the group kill must never reach us
    )
    time.sleep(1.5)
    # Every probe and the GUI share the SAME hermetic env — an ambient-env
    # probe would answer from the owner's live production daemon instead.
    probe_env = {
        **os.environ,
        "DISPLAY": display, "GDK_BACKEND": "x11", "YGGTERM_FORCE_X11_BACKEND": "1",
        "YGGTERM_HOME": home_dir, "XDG_DATA_HOME": f"{home_dir}/xdg-share", "HOME": home_dir,
    }
    gui_log_path = out_dir / "gui.log"
    env_shell = (
        f'export DISPLAY={display} GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 '
        f'RUST_BACKTRACE=1 YGGTERM_HOME="{home_dir}" XDG_DATA_HOME="{home_dir}/xdg-share" '
        f'HOME="{home_dir}"; exec "{binary}"'
    )
    gui = subprocess.Popen(
        ["dbus-run-session", "--", "bash", "-c", env_shell],
        stdout=open(gui_log_path, "w"), stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL, start_new_session=True,
    )
    manifest["environment"] = {
        "home": home_dir, "xdg_data_home": f"{home_dir}/xdg-share",
        "display": display, "small_world": True,
        "small_world_sessions": SMALL_WORLD_SESSIONS,
        "binary_declared": os.path.realpath(binary),
    }
    try:
        ready, ready_s = wait_ready(binary, READY_WAIT_S, probe_env)
        if ready:
            time.sleep(SETTLE_S)
        gui_log = gui_log_path.read_text(errors="replace") if gui_log_path.exists() else ""
        served = served_executable_path(home_dir)
        shot_path = out_dir / "screenshot.png"
        captured, unsupported = (False, False)
        visual = None
        visual_note = None
        if ready:
            # The composite capture is occasionally slow to become available;
            # retry the OBSERVATION (never the product verdict) and record
            # every attempt — flakiness is data, not something to erase.
            attempts = []
            for try_no in (1, 2):
                captured, unsupported = capture_screenshot(binary, str(shot_path), probe_env)
                attempts.append({"try": try_no, "captured": captured, "unsupported": unsupported})
                if captured:
                    break
                time.sleep(3)
            if not captured:
                visual_note = f"screenshot capture failed after {len(attempts)} attempts: {attempts}"
            if captured:
                try:
                    samples = decode_png_rgb(shot_path.read_bytes())
                    fractions = classify_pixels(samples)
                    visual = {"verdict": visual_verdict(fractions), "fractions": fractions}
                except ValueError as exc:
                    visual = None
                    visual_note = f"visual frame undecodable: {exc}"
        served_pre = file_sha256(served)
        identity_reasons: list[str] = []
        identity_ok: bool | None
        if served is None:
            identity_ok = None
            identity_reasons.append("trace carries no executable_path — the run's identity is unproven")
        else:
            served_post = file_sha256(served)
            manifest["binary"]["served_executable_path"] = served
            manifest["binary"]["served_sha256_pre"] = served_pre
            manifest["binary"]["served_sha256_post"] = served_post
            manifest["binary"]["handoff_recorded"] = served != os.path.realpath(binary)
            if served_pre is None:
                identity_ok = False
                identity_reasons.append(f"served exe unreadable: {served}")
            elif served_pre != served_post:
                identity_ok = False
                identity_reasons.append("served exe changed mid-run (artifact replacement)")
            elif args.strict_identity and served != os.path.realpath(binary):
                identity_ok = False
                identity_reasons.append(
                    f"strict identity: trace served {served}, declared {os.path.realpath(binary)} — [11.231] handoff"
                )
            else:
                identity_ok = True
        obs = {
            "daemon_ready": ready,
            "ready_after_s": ready_s if ready else None,
            "ready_wait_s": READY_WAIT_S,
            "panic_lines": scan_panic_lines(gui_log),
            "panic_trace": panic_trace_events(home_dir),
            "component_window_count": count_trace_events(home_dir, "component_window"),
            "identity_ok": identity_ok,
            "identity_reasons": identity_reasons,
            "screenshot_captured": captured,
            "screenshot_unsupported": unsupported,
            "screenshot_attempts": attempts if ready else [],
            "visual": visual,
            "visual_note": visual_note,
            "instrument_faults": [],
        }
        manifest["observations"] = obs
        manifest["identity"] = {
            "served": served, "ok": identity_ok, "reasons": identity_reasons,
        }
        if captured:
            manifest["bundle_files"]["screenshot.png"] = file_sha256(str(shot_path))
        verdict = decide(obs)
        manifest["verdict"] = verdict["verdict"]
        manifest["reasons"] = verdict["reasons"]
        manifest["product_failure_observed"] = verdict["product_failure_observed"]
        # Raw witnesses leave the scratch BEFORE teardown — the audit bundle,
        # not /tmp, is where an independent auditor replays from.
        for ix, tp in enumerate(trace_files(home_dir)):
            dst = out_dir / f"trace-{ix}.jsonl"
            try:
                shutil.copyfile(tp, dst)
                manifest["bundle_files"][dst.name] = file_sha256(str(dst))
            except OSError:
                pass
        (out_dir / "trace-excerpt.jsonl").write_text("\n".join(trace_excerpt(home_dir)))
        manifest["bundle_files"]["trace-excerpt.jsonl"] = file_sha256(str(out_dir / "trace-excerpt.jsonl"))
    finally:
        # The GUI spawns children (dbus daemon, tab mirrors, the server) whose
        # cmdlines do NOT carry the scratch path — only the process group kill
        # reaches them; a lone pkill -f leaks writers that recreate scratch
        # entries after rmtree.
        import signal

        for proc in (gui, xvfb):
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                try:
                    proc.terminate()
                except Exception:
                    pass
        time.sleep(2)
        for proc in (gui, xvfb):
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        subprocess.run(["pkill", "-f", scratch], capture_output=True)
        time.sleep(1)
        shutil.rmtree(scratch, ignore_errors=True)
        if os.path.exists(scratch):
            time.sleep(2)
            shutil.rmtree(scratch, ignore_errors=True)
            manifest["scratch_leftover"] = os.path.exists(scratch)

    for name in ("gui.log", "xvfb.log"):
        path = out_dir / name
        if path.exists():
            manifest["bundle_files"][name] = file_sha256(str(path))
    manifest["finished_at_utc"] = datetime.now(timezone.utc).isoformat(timespec="seconds")
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    manifest["bundle_files"]["manifest.json"] = file_sha256(str(manifest_path))
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


# ---------------------------------------------------------------------------
# Selftest — pure decision tests + two live instrument controls
# ---------------------------------------------------------------------------


def selftest(binary: str, args) -> int:
    failures = []

    def expect(name, got, want):
        if got != want:
            failures.append(f"{name}: got {got!r}, want {want!r}")

    # --- pure decision layer -------------------------------------------------
    expect(
        "panic scan",
        scan_panic_lines("ok line\nEncountered panic: Any { .. }\nPanic while rendering component `Root`"),
        ["Encountered panic: Any { .. }", "Panic while rendering component `Root`"],
    )
    expect("panic scan clean", scan_panic_lines("nothing here"), [])

    green = {
        "daemon_ready": True, "ready_after_s": 7, "ready_wait_s": 90,
        "panic_lines": [], "panic_trace": [], "component_window_count": 12,
        "identity_ok": True, "identity_reasons": [],
        "screenshot_captured": True, "screenshot_unsupported": False,
        "visual": {"verdict": "healthy", "fractions": {"content": 0.2, "red": 0.0}},
        "instrument_faults": [],
    }
    expect("green PASS", decide(green)["verdict"], "PASS")
    expect(
        "missing screenshot never PASS",
        decide({**green, "screenshot_captured": False})["verdict"], "UNKNOWN",
    )
    expect(
        "panic FAIL", decide({**green, "panic_lines": ["Encountered panic: Any { .. }"]})["verdict"], "FAIL",
    )
    expect(
        "trace panic FAIL",
        decide(
            {
                **green,
                "panic_trace": [
                    {"name": "panic_hook", "location": "dioxus scope_context.rs:387", "message": "Unable to retrieve the hook"}
                ],
            }
        )["verdict"],
        "FAIL",
    )
    expect(
        "panic-pixel FAIL",
        decide({**green, "visual": {"verdict": "panic_visual", "fractions": {"content": 0.0, "red": 0.0004}}})["verdict"],
        "FAIL",
    )
    expect(
        "indeterminate visual caps UNKNOWN",
        decide({**green, "visual": {"verdict": "indeterminate", "fractions": {"content": 0.005, "red": 0.0}}})["verdict"],
        "UNKNOWN",
    )
    expect(
        "captured-but-unjudged caps UNKNOWN",
        decide({**green, "visual": None, "visual_note": "visual frame undecodable: bad PNG"})["verdict"],
        "UNKNOWN",
    )
    expect(
        "dom-empty FAIL", decide({**green, "component_window_count": 0})["verdict"], "FAIL",
    )
    expect(
        "identity mismatch certifies nothing",
        decide({**green, "identity_ok": None, "identity_reasons": ["no trace identity"]})["verdict"], "UNKNOWN",
    )
    expect(
        "never-ready FAIL", decide({**green, "daemon_ready": False, "ready_after_s": None})["verdict"], "FAIL",
    )
    expect(
        "instrument fault caps UNKNOWN",
        decide({**green, "instrument_faults": ["Xvfb not on PATH"]})["verdict"], "UNKNOWN",
    )

    # --- pixel classifier: calibrated on the 2026-10-26 panic frame ----------
    black_frame = [(0, 0, 0)] * 9900 + [(200, 30, 30)] * 4  # red text on pure black
    expect("black+red frame panic_visual", visual_verdict(classify_pixels(black_frame)), "panic_visual")
    busy_frame = [(90, 90, 90)] * 2000 + [(10, 10, 10)] * 8000  # dark-theme GUI content
    expect("busy frame healthy", visual_verdict(classify_pixels(busy_frame)), "healthy")
    almost_black = [(0, 0, 0)] * 9998 + [(2, 2, 2)] * 2
    expect("near-black blank_visual", visual_verdict(classify_pixels(almost_black)), "blank_visual")

    # --- live controls: a stub that panics + never serves --------------------
    with __import__("tempfile").TemporaryDirectory() as tmp:
        stub_fail = os.path.join(tmp, "stub-fail")
        with open(stub_fail, "w") as fh:
            fh.write('#!/bin/bash\nif [ "$1" = server ]; then sleep 300; fi\necho "Encountered panic: Any { .. }" >&2\nsleep 300\n')
        os.chmod(stub_fail, 0o755)
        args_stub = argparse.Namespace(display=":88", trials=1, out=None, strict_identity=False, no_small_world=True)
        m = run_trial(stub_fail, args_stub, f"selftest-fail-{uuid.uuid4().hex[:4]}", Path(tmp) / "fail")
        expect("fail-control verdict", m["verdict"], "FAIL")

        # --- live mismatch control: stub serves a trace naming another binary -
        # The fake trace is written at runtime into the harness's own scratch
        # YGGTERM_HOME, simulating the [11.231] handoff to a different binary.
        stub_mismatch = os.path.join(tmp, "stub-mismatch")
        with open(stub_mismatch, "w") as fh:
            fh.write('#!/bin/bash\nif [ "$1" = server ]; then sleep 300; fi\n')
            fh.write('mkdir -p "$YGGTERM_HOME/state"\n')
            fh.write(
                "echo '{\"name\":\"register\",\"payload\":{\"executable_path\":"
                '"/nonexistent/other-yggterm"}}\' > "$YGGTERM_HOME/state/event-trace.jsonl"\n'
            )
            fh.write("sleep 300\n")
        os.chmod(stub_mismatch, 0o755)
        args_mm = argparse.Namespace(display=":89", trials=1, out=None, strict_identity=False, no_small_world=True)
        m = run_trial(stub_mismatch, args_mm, f"selftest-mm-{uuid.uuid4().hex[:4]}", Path(tmp) / "mm")
        expect("mismatch control never PASS", m["verdict"] == "PASS", False)
        identity_reasons = " ".join(m.get("reasons", [])).lower()
        expect(
            "mismatch control names the identity fault",
            "unreadable" in identity_reasons or "identity" in identity_reasons,
            True,
        )

    if failures:
        print("SELFTEST FAILURES:")
        for f in failures:
            print(f"  {f}")
        return 1
    print("selftest green: pure decision layer + fail-control + mismatch-control")
    return 0


# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", default=DEFAULT_BINARY, help="GUI binary to audit (default: installed build)")
    parser.add_argument("--trials", type=int, default=1)
    parser.add_argument("--out", default=None, help="bundle root (default: ~/.local/state/yggterm-audit)")
    parser.add_argument("--display", default=os.environ.get("AUDIT_DISPLAY", DEFAULT_DISPLAY))
    parser.add_argument("--strict-identity", action="store_true",
                        help="trace executable_path must equal the declared binary (worktree audits)")
    parser.add_argument("--no-small-world", action="store_true", help="skip codex-store staging")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()

    if args.selftest:
        return selftest(args.binary, args)

    if not os.path.exists(args.binary):
        print(f"NO BINARY at {args.binary}", file=sys.stderr)
        return 2
    global SMALL_WORLD_SESSIONS
    if args.no_small_world:
        SMALL_WORLD_SESSIONS = 0
    out_root = Path(args.out or DEFAULT_OUT_ROOT)
    results = []
    for _ in range(args.trials):
        attempt_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:4]
        out_dir = out_root / attempt_id
        manifest = run_trial(args.binary, args, attempt_id, out_dir)
        results.append(manifest)
        print(
            f"{attempt_id} verdict={manifest['verdict']} "
            f"served={manifest.get('binary', {}).get('served_executable_path') or 'UNPROVEN'}"
        )
        for reason in manifest["reasons"]:
            print(f"  - {reason}")
        print(f"  bundle: {out_dir}/manifest.json")
    bad = [m for m in results if m["verdict"] != "PASS"]
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
