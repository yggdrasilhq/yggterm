#!/usr/bin/env python3
"""THE DRAFT-GUARD PROVENANCE PROBE for the programmatic-send path ([11.223]).

The E2E law applies: drive the REAL daemon verbs (newline-JSON on the daemon
socket — the same wire every CLI verb and the GUI speak) and assert on the
row's own PTY screen (`server terminal screen`), the one instrument that
cannot lie about what a user would see. Red baseline measured on production
cd735bef 2026-10-03 (send-2 refused pending_draft held_len=11 for the probe's
own 11 bytes; the empty-write remedy cleared nothing) before the fix; the fix
(43ca1f00) flips legs L3+L5 green and the daemon trace carries
terminal_input_draft_autocleared with clear_ok=true.

Legs:
  L1 one-shot `echo TAGA\\r` on a fresh ENSURED shell row executes
  L2 `echo TAGB` (no newline) is accepted and does NOT execute
  L3 the next send (`echo TAGC\\r`) executes — THE [11.223] DEFECT if refused
  L4 a Ctrl+U remedy still works on a stuck line
  L5 the empty write + resend path executes (the old hint's promise)

Reaps every row it creates. Usage: python3 tools/e2e/draft_guard_probe.py
"""
import glob
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HOME = os.path.expanduser("~")
BIN = os.environ.get("YGGTERM_BIN", HOME + "/.yggterm/bin/yggterm")
TAG = os.environ.get("DRAFT_PROBE_TAG", "DGP")

OUT = []


def daemon_socket():
    """The live daemon's unix socket — majority realpath of the server-*.sock
    family (the [11.213] recipe; the serving name rotates with the version)."""
    counts = {}
    for link in glob.glob(os.path.join(HOME, ".yggterm/server-*.sock")):
        try:
            tgt = os.path.realpath(link)
            counts[tgt] = counts.get(tgt, 0) + 1
        except OSError:
            continue
    return max(counts.items(), key=lambda kv: kv[1])[0] if counts else None


def wire(request, timeout=60):
    """One newline-JSON request to the daemon socket (anonymous envelope)."""
    stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    stream.settimeout(timeout)
    try:
        stream.connect(daemon_socket())
        stream.sendall((json.dumps(request) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            chunk = stream.recv(65536)
            if not chunk:
                break
            buf += chunk
    finally:
        stream.close()
    lines = buf.decode(errors="replace").strip().splitlines()
    return json.loads(lines[0]) if lines else {}


def screen(key):
    r = subprocess.run([BIN, "server", "terminal", "screen", key],
                       capture_output=True, text=True, timeout=30)
    return r.stdout if r.returncode == 0 else ""


def leg(name, ok, evidence):
    OUT.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name + " :: " +
          evidence[:260].replace("\n", " | "))


def guarded_write(key, data):
    r = wire({"kind": "terminal_write", "path": key, "data": data,
              "refuse_if_draft": True})
    return str(r.get("message", ""))


def main():
    d = subprocess.run([BIN, "server", "daemons"], capture_output=True,
                       text=True, timeout=30)
    print("DAEMONS", d.stdout[:160].replace("\n", " "))

    cwd = tempfile.mkdtemp(prefix="draft-guard-probe-")
    r = wire({"kind": "start_local_session", "session_kind": "shell",
              "cwd": cwd, "title_hint": "[11.223] draft-guard probe",
              "activate": False})

    def find_key(node):
        if isinstance(node, dict):
            sp = node.get("session_path") or node.get("key")
            if isinstance(sp, str) and ("local" in sp or "shell" in sp):
                if node.get("cwd") == cwd or \
                        (node.get("title") or "").startswith("[11.223]"):
                    return sp
            for v in node.values():
                k = find_key(v)
                if k:
                    return k
        elif isinstance(node, list):
            for v in node:
                k = find_key(v)
                if k:
                    return k
        return None

    key = find_key(r)
    if not key:
        rows = subprocess.run([BIN, "server", "rows", "live", "--json"],
                              capture_output=True, text=True, timeout=30)
        try:
            for row in json.loads(rows.stdout).get("rows", []):
                if (row.get("title") or "").startswith("[11.223]"):
                    key = row.get("key") or row.get("session_path")
                    break
        except Exception:
            pass
    print("ROW", key, "CWD", cwd)
    if not key:
        return 2

    try:
        for _ in range(40):
            if screen(key).strip():
                break
            time.sleep(0.25)
        # ENSURE first — the GUI mount always does; a raw birth + immediate
        # write otherwise hits the lost-runtime relaunch arm and drops the
        # keystroke (named ack, measured).
        wire({"kind": "terminal_ensure", "path": key})
        for _ in range(40):
            if screen(key).strip():
                break
            time.sleep(0.25)

        r1, = guarded_write(key, f"echo {TAG}A\r"),
        s1 = screen(key)
        leg("L1 one-shot newline executes",
            f"{TAG}A" in s1 and "refus" not in r1.lower(), f"ack={r1[:120]}")

        r2 = guarded_write(key, f"echo {TAG}B")
        time.sleep(0.4)
        leg("L2 no-newline accepted, not executed",
            "refus" not in r2.lower(), f"ack={r2[:120]}")

        r3 = guarded_write(key, f"echo {TAG}C\r")
        time.sleep(0.5)
        s3 = screen(key)
        leg("L3 second send executes (THE [11.223] DEFECT if refused)",
            f"{TAG}C" in s3 and "draft" not in r3.lower(), f"ack={r3[:200]}")

        r4 = guarded_write(key, "\u0015")
        time.sleep(0.3)
        r5 = guarded_write(key, f"echo {TAG}D\r")
        time.sleep(0.5)
        s5 = screen(key)
        leg("L4 ctrl-u remedy then send executes",
            f"{TAG}D" in s5, f"clear={r4[:60]} send={r5[:140]}")

        r6 = guarded_write(key, f"echo {TAG}E")
        time.sleep(0.3)
        r7 = guarded_write(key, "")
        time.sleep(0.3)
        r8 = guarded_write(key, f"echo {TAG}F\r")
        time.sleep(0.5)
        s8 = screen(key)
        leg("L5 empty write then send executes (the old hint's promise)",
            f"{TAG}F" in s8, f"empty={r7[:60]} send={r8[:140]}")
    finally:
        subprocess.run([BIN, "server", "remove", key],
                       capture_output=True, timeout=60)
        subprocess.run([BIN, "server", "rows", "despawn", key],
                       capture_output=True, timeout=60)
        shutil.rmtree(cwd, ignore_errors=True)

    fails = [n for n, ok in OUT if not ok]
    print("SUMMARY", "ALL-PASS" if not fails else f"RED: {fails}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
