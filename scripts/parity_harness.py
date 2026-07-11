#!/usr/bin/env python3
"""Tier 2 of the render parity harness: MANUAL vs YGGTERM, byte-for-byte.

CLAUDE.md's wrapper-vs-manual parity rule says: if a session opened via yggterm
renders differently from the equivalent command typed into a plain shell, that is
a yggterm bug — and the fix belongs in yggterm's wrapper/handoff path, never in
extra CLI flags. This harness MEASURES that rule instead of arguing about it.

It runs the SAME agent-CLI command twice:

  A. MANUAL   — spawned directly on a raw PTY of a fixed size (the control; this
                is what the user does when they give up on the GUI and it "works
                flawlessly").
  B. YGGTERM  — spawned through the daemon's terminal runtime, reading the bytes
                the GUI would forward to xterm.

...then diffs the two byte streams and the two final screens. Any divergence is a
yggterm bug by definition of the parity rule.

Tier 1 of the harness (the FAITHFUL-PIPE invariant over `batch_terminal_chunks`)
lives in the Rust tests and guards the batch seam only. Tier 2 is what catches
launch/env/PTY-size divergence — the "TUI not recognized" class, where the CLI
never even negotiates the terminal correctly because the wrapper handed it a
different world than a plain shell does.

Usage:
    scripts/parity_harness.py --cmd 'claude --version'         # smoke
    scripts/parity_harness.py --cmd 'claude -r <uuid>' --cwd ~/gh/yggterm --settle 6
    scripts/parity_harness.py --cmd 'codex resume <uuid>' --rows 44 --cols 176

Exit status is 0 when the streams agree, 1 when they diverge (so it can gate CI).

NOTE ON HONESTY: this compares the manual PTY against the DAEMON's view of the
session. It proves the launch/env/geometry/PTY half of the wrapper. It does NOT
prove the GUI's xterm paint — that needs a faithful pixel (see
feedback-verify-visual-with-faithful-pixel). Do not report a green run here as
"the rendering is fixed"; report it as "the bytes leaving the wrapper match the
bytes leaving a plain shell".
"""

from __future__ import annotations

import argparse
import errno
import os
import pty
import select
import shutil
import subprocess
import sys
import termios
import fcntl
import struct
import time
from dataclasses import dataclass


DEFAULT_ROWS = 44
DEFAULT_COLS = 176
# A TUI paints, then idles. Give it time to finish its first full frame before we
# stop reading, or we diff two half-drawn screens and learn nothing.
DEFAULT_SETTLE_S = 5.0


@dataclass
class Capture:
    label: str
    raw: bytes

    @property
    def screen(self) -> str:
        """The visible screen, as a vt100 would render it. Requires pyte; falls back
        to a control-stripped view so the harness still runs without it."""
        try:
            import pyte  # type: ignore
        except ImportError:
            return _strip_controls(self.raw.decode("utf-8", "replace"))
        screen = pyte.Screen(DEFAULT_COLS, DEFAULT_ROWS)
        stream = pyte.Stream(screen)
        stream.feed(self.raw.decode("utf-8", "replace"))
        return "\n".join(line.rstrip() for line in screen.display).rstrip()


def _strip_controls(text: str) -> str:
    import re

    text = re.sub(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", "", text)
    text = re.sub(r"\x1b[@-Z\\-_]|\x1b\[[0-?]*[ -/]*[@-~]", "", text)
    return text


def _set_winsize(fd: int, rows: int, cols: int) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def capture_manual(cmd: str, cwd: str, rows: int, cols: int, settle_s: float) -> Capture:
    """THE CONTROL. Spawn the command on a raw PTY exactly as a plain shell would:
    a login-ish env, a correctly-sized terminal, no wrapper in between."""
    pid, fd = pty.fork()
    if pid == 0:  # child
        try:
            os.chdir(os.path.expanduser(cwd))
            # Match what a normal terminal emulator exports. Deliberately minimal:
            # the parity rule says the wrapper must not need MORE than this.
            os.environ["TERM"] = "xterm-256color"
            os.environ["COLORTERM"] = "truecolor"
            os.environ["LINES"] = str(rows)
            os.environ["COLUMNS"] = str(cols)
            os.execvp("bash", ["bash", "-lc", cmd])
        except Exception:  # pragma: no cover - child cannot report cleanly
            os._exit(127)

    _set_winsize(fd, rows, cols)
    raw = _drain(fd, settle_s)
    _reap(pid, fd)
    return Capture("manual", raw)


def _drain(fd: int, settle_s: float) -> bytes:
    """Read until the stream goes quiet for `settle_s`, or the hard cap elapses."""
    chunks: list[bytes] = []
    deadline = time.monotonic() + settle_s * 4
    last_data = time.monotonic()
    while time.monotonic() < deadline:
        if time.monotonic() - last_data > settle_s:
            break
        try:
            ready, _, _ = select.select([fd], [], [], 0.25)
        except (OSError, ValueError):
            break
        if not ready:
            continue
        try:
            data = os.read(fd, 65536)
        except OSError as exc:
            if exc.errno in (errno.EIO, errno.EBADF):
                break
            raise
        if not data:
            break
        chunks.append(data)
        last_data = time.monotonic()
    return b"".join(chunks)


def _reap(pid: int, fd: int) -> None:
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.kill(pid, 15)
    except ProcessLookupError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass


def capture_yggterm(cmd: str, cwd: str, rows: int, cols: int, settle_s: float) -> Capture:
    """THE WRAPPER. Drive the same command through yggterm's daemon terminal runtime
    and collect the bytes it would hand to xterm.

    Uses the headless CLI so the harness measures the SHIPPING path, not a
    reimplementation of it. If the CLI lacks a raw-stream affordance, we say so
    loudly rather than silently comparing against something else — a harness that
    quietly measures the wrong thing is worse than no harness."""
    cli = shutil.which("yggterm-headless")
    if not cli:
        raise SystemExit(
            "yggterm-headless not on PATH — build/deploy it first; refusing to fake the wrapper side"
        )

    probe = subprocess.run(
        [cli, "server", "terminal", "--help"],
        capture_output=True,
        text=True,
        timeout=20,
    )
    help_text = (probe.stdout or "") + (probe.stderr or "")
    if "raw-stream" not in help_text:
        raise SystemExit(
            "PARITY HARNESS GAP: `yggterm-headless server terminal` exposes no raw-stream\n"
            "read, so the wrapper's byte stream cannot be captured without reimplementing\n"
            "the read path (which would measure the harness, not the product).\n\n"
            "Next step for the parity campaign: add\n"
            "  yggterm-headless server terminal raw-stream --session <path> --for <secs>\n"
            "emitting the exact bytes the GUI forwards to `term.write`. Then this function\n"
            "captures side B and the diff below becomes the wrapper-vs-manual gate.\n"
            "See campaign-render-pipeline-parity-rework."
        )

    out = subprocess.run(
        [cli, "server", "terminal", "raw-stream", "--command", cmd, "--cwd", cwd,
         "--rows", str(rows), "--cols", str(cols), "--for", str(settle_s)],
        capture_output=True,
        timeout=settle_s * 6 + 30,
    )
    return Capture("yggterm", out.stdout)


def report(manual: Capture, wrapper: Capture) -> int:
    print(f"manual : {len(manual.raw):>8} bytes")
    print(f"yggterm: {len(wrapper.raw):>8} bytes")

    if manual.raw == wrapper.raw:
        print("\nPARITY OK — the wrapper's byte stream is identical to a plain shell's.")
        return 0

    print("\nPARITY VIOLATION — yggterm's bytes differ from the manual baseline.")

    # Carriage returns are the class whose loss produces the staircase/interleave
    # garble, so call them out explicitly (this is the bug class of 2026-07-11).
    m_cr, w_cr = manual.raw.count(b"\r"), wrapper.raw.count(b"\r")
    if m_cr != w_cr:
        print(f"  ! carriage returns: manual={m_cr} yggterm={w_cr} (CR LOSS => wrong-column paint)")

    for i, (a, b) in enumerate(zip(manual.raw, wrapper.raw)):
        if a != b:
            lo = max(0, i - 60)
            print(f"  first divergence at byte {i}:")
            print(f"    manual : {manual.raw[lo:i + 60]!r}")
            print(f"    yggterm: {wrapper.raw[lo:i + 60]!r}")
            break
    else:
        short, long_ = sorted((manual.raw, wrapper.raw), key=len)
        print(f"  one stream is a prefix of the other; extra tail: {long_[len(short):][:180]!r}")

    if manual.screen != wrapper.screen:
        print("\n  final screens ALSO differ (this is what the user sees):")
        import difflib

        diff = difflib.unified_diff(
            manual.screen.splitlines(),
            wrapper.screen.splitlines(),
            fromfile="manual",
            tofile="yggterm",
            lineterm="",
        )
        for line in list(diff)[:60]:
            print(f"    {line}")
    else:
        print("\n  final screens match — divergence is in the byte path but converged on screen.")
    return 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cmd", required=True, help="the agent-CLI command, e.g. 'claude -r <uuid>'")
    ap.add_argument("--cwd", default=os.getcwd())
    ap.add_argument("--rows", type=int, default=DEFAULT_ROWS)
    ap.add_argument("--cols", type=int, default=DEFAULT_COLS)
    ap.add_argument("--settle", type=float, default=DEFAULT_SETTLE_S)
    ap.add_argument("--manual-only", action="store_true", help="capture the control side only")
    args = ap.parse_args()

    manual = capture_manual(args.cmd, args.cwd, args.rows, args.cols, args.settle)
    if args.manual_only:
        cr_count = manual.raw.count(b"\r")
        print(f"manual: {len(manual.raw)} bytes, {cr_count} CRs")
        print(manual.screen)
        return 0

    wrapper = capture_yggterm(args.cmd, args.cwd, args.rows, args.cols, args.settle)
    return report(manual, wrapper)


if __name__ == "__main__":
    sys.exit(main())
