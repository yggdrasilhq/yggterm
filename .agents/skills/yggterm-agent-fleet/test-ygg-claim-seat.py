#!/usr/bin/env python3
"""Regression tests for the seat derivation inside ygg-claim.sh.

WHY THIS FILE EXISTS
    The derivation is a python program embedded in a shell script, so nothing
    imported it and nothing could test it. Three defects therefore shipped and
    were only found by reading a sidebar:

      * a top-level claim landed at a bare major ("11") instead of "N.0";
      * inheriting a predecessor seat kept the major and threw the MINOR away,
        so a successor to 11.4 landed at 11 and the handover silently promoted a
        lane to a top-level row;
      * sibling matching took the LOWEST major among rows whose title mentions
        the campaign token -- and campaign tokens are reused between waves, so a
        new delegate was seated into a RETIRED era, live and invisible to the
        orchestrator that spawned it.

    The tests below extract the real program out of the script and run it, so
    they cannot drift from the code the way a re-implementation would.

RUN
    python3 test-ygg-claim-seat.py        # exits non-zero on any failure
"""
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "ygg-claim.sh"


def extract_program(dest):
    """Pull the derivation program verbatim out of the shell script."""
    src = SCRIPT.read_text()
    i = src.index('PLAN="$(rows_json')
    j = src.index("python3 -c '", i) + len("python3 -c '")
    k = src.index("\n')", j)
    dest.write_text(src[j:k])
    return dest


def row(path, prefix=None, title="", kind="Session"):
    return {
        "full_path": path,
        "outline_prefix": prefix,
        "session_title": title,
        "label": ((prefix + " ") if prefix else "") + title,
        "kind": kind,
    }


def derive(prog, rows, uuid, campaign="", replace="", number="", inherit="0", home=None):
    env = dict(
        os.environ,
        UUID=uuid,
        CAMPAIGN=campaign,
        REPLACE=replace,
        NUMBER=number,
        INHERIT=inherit,
        FORCE_NUMBER="0",
    )
    if home:
        env["HOME"] = home
    p = subprocess.run(
        [sys.executable, str(prog)],
        input=json.dumps({"data": {"rows": rows}}),
        capture_output=True,
        text=True,
        env=env,
    )
    if p.returncode != 0:
        return "ERR:" + (p.stderr.strip().splitlines() or ["?"])[-1]
    m = re.search(r"^NUM=(\S+)", p.stdout, re.M)
    return m.group(1) if m else "NO-NUM"


def main():
    with tempfile.TemporaryDirectory() as td:
        prog = extract_program(pathlib.Path(td, "derive.py"))
        failures = []

        def check(label, got, want):
            ok = got == want
            print(("  PASS  " if ok else "  FAIL  ") + label + f" -> {got}"
                  + ("" if ok else f"   (expected {want})"))
            if not ok:
                failures.append(label)

        # A top-level claim is N.0. A bare major is not a seat: the parent would
        # render as a sibling of its own children and sort away from them.
        check(
            "fresh top-level claim is N.0",
            derive(prog,
                   [row("remote-cc://h/AAA", None, "alpha: orchestrator"),
                    row("remote-cc://h/BBB", "7.0", "beta orchestration")],
                   "AAA", "alpha"),
            "8.0")

        # Inheriting a seat means the WHOLE seat.
        replaced = [row("remote-cc://h/NEW", None, "render lane"),
                    row("remote-cc://h/OLD", "11.4", "render lane")]
        check("--inherit-number keeps the minor",
              derive(prog, replaced, "NEW", "render", replace="OLD", inherit="1"), "11.4")
        check("a plain --replace agrees with it",
              derive(prog, replaced, "NEW", "render", replace="OLD"), "11.4")

        # Succeeding a row that is itself mis-seated repairs the seat.
        check(
            "succeeding a bare-major head normalises to N.0",
            derive(prog,
                   [row("remote-cc://h/NEW", None, "alpha: orchestrator"),
                    row("remote-cc://h/OLD", "11", "alpha: orchestrator")],
                   "NEW", "alpha", replace="OLD", inherit="1"),
            "11.0")

        # Campaign tokens are reused across waves; the newest era must win.
        check(
            "sibling matching does not cross campaign eras",
            derive(prog,
                   [row("remote-cc://h/NEW", None, "gamma delegate"),
                    row("remote-cc://h/O1", "2.1", "gamma old wave"),
                    row("remote-cc://h/O3", "2.3", "gamma old wave"),
                    row("remote-cc://h/N0", "12.0", "gamma orchestrator"),
                    row("remote-cc://h/N1", "12.1", "gamma lane")],
                   "NEW", "gamma"),
            "12.2")

        # The spawner DECLARED the seat before the delegate ever claimed. A
        # declaration outranks any inference drawn from row titles.
        home = pathlib.Path(td, "fakehome")
        relay = home / ".yggterm" / "relay"
        relay.mkdir(parents=True)
        (relay / "spawned-by-ORCH.txt").write_text(
            "11.5|render|/somewhere|remote-cc://h/LEDGERUUID\n")
        check(
            "the spawner ledger outranks title inference",
            derive(prog,
                   [row("remote-cc://h/LEDGERUUID", None, "alpha lane"),
                    row("remote-cc://h/S1", "3.1", "alpha other")],
                   "LEDGERUUID", "alpha", home=str(home)),
            "11.5")

        # The queue entry named an exact falsifier; run it verbatim. An
        # orchestrator seated 12.0 with retired 2.x rows still present must seat
        # its delegate at 12.1.
        check(
            "the queue falsifier: 12.0 head with retired 2.x present",
            derive(prog,
                   [row("remote-cc://h/NEW", None, "gamma delegate"),
                    row("remote-cc://h/O1", "2.1", "gamma old wave"),
                    row("remote-cc://h/O3", "2.3", "gamma old wave"),
                    row("remote-cc://h/N0", "12.0", "gamma orchestrator")],
                   "NEW", "gamma"),
            "12.1")

        # THE REGRESSION THAT MATTERS MOST. Every session claims unasked, so a
        # SECOND claim is the common path. An already-seated row must not be
        # renumbered just because the campaign token matches foreign-era rows.
        check(
            "an existing seat is absolute even when siblings match",
            derive(prog,
                   [row("remote-cc://h/AAA", "12.2", "gamma delegate"),
                    row("remote-cc://h/O1", "2.1", "gamma old wave"),
                    row("remote-cc://h/O3", "2.3", "gamma old wave"),
                    row("remote-cc://h/N0", "12.0", "gamma orchestrator")],
                   "AAA", "gamma"),
            "12.2")

        check("an explicit bare --number is normalised too",
              derive(prog, [row("remote-cc://h/AAA", None, "x")], "AAA", "x", number="9"),
              "9.0")

        # Regression guards: claiming must stay idempotent, and an explicit
        # sub-seat must survive untouched.
        check("re-claiming an already-seated row is a no-op",
              derive(prog, [row("remote-cc://h/AAA", "6.3", "sidebar truth")], "AAA", "sidebar"),
              "6.3")
        check("an explicit N.x is left alone",
              derive(prog, [row("remote-cc://h/AAA", None, "x")], "AAA", "x", number="4.7"),
              "4.7")

        print()
        if failures:
            print(f"{len(failures)} FAILED: {failures}")
            return 1
        print("all seat-derivation tests pass")
        return 0


if __name__ == "__main__":
    sys.exit(main())
