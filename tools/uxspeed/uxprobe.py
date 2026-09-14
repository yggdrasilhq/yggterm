#!/usr/bin/env python3
"""uxprobe — the UX-SPEED campaign's synthetic timing + accuracy harness.

Times the campaign's UX actions verb→effect using the server app verb
family (the GUI's own sanctioned automation surface) and ASSERTS accuracy,
on self-created scratch rows only. Blast-radius law: rows not created by
this run are never mutated; teardown removes exactly what the run spawned.

Actions:
  spawn  — server app terminal new → wait first_frame in ytrace
  drag   — server app drag begin/hover/drop reorder of two scratch rows
  group  — server app row-set --into / --out on two scratch rows
  menu   — server app terminal probe-context-menu on a scratch row
  close  — server app session remove of the scratch rows (the teardown,
           instrumented — every spawned row is closed exactly once)

Report: schema-keyed JSON, honest nulls for anything not measured.
A p50 without its iteration count is not a measurement.

Usage:
  python3 tools/uxspeed/uxprobe.py --actions spawn,drag,menu --iters 3 \
      --out /tmp/uxspeed-report.json [--artifacts DIR] [--keep] [-v]
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import time

PROBE_TITLE_PREFIX = "uxspeed-probe-"
YGGTERM = "yggterm"
YTRACE = "ytrace"


def now_ms() -> int:
    return int(time.time() * 1000)


class Probe:
    def __init__(self, args):
        self.args = args
        self.timeout_s = args.timeout_ms / 1000.0
        self.artifacts = args.artifacts
        if self.artifacts:
            os.makedirs(self.artifacts, exist_ok=True)
        self.spawned_paths: list[str] = []
        self.trace_events_seen: dict[str, set] = {}

    # ---- instruments -------------------------------------------------

    def verb(self, *argv: str, timeout: float | None = None) -> dict:
        """Run one `yggterm server app …` verb; return parsed reply + wall ms."""
        cmd = [YGGTERM, "server", "app", *argv]
        t0 = time.perf_counter()
        try:
            proc = subprocess.run(
                cmd, capture_output=True, text=True,
                timeout=timeout or self.timeout_s,
            )
        except subprocess.TimeoutExpired:
            return {"ok": False, "error": "verb timeout", "wall_ms": None,
                    "raw": ""}
        wall_ms = int((time.perf_counter() - t0) * 1000)
        reply: dict = {"ok": proc.returncode == 0, "wall_ms": wall_ms}
        try:
            reply["json"] = json.loads(proc.stdout)
        except (json.JSONDecodeError, ValueError):
            reply["json"] = None
            reply["raw"] = (proc.stdout + proc.stderr)[-2000:]
        if proc.returncode != 0:
            reply["error"] = (proc.stderr or proc.stdout)[-500:]
        self.dump_artifact(f"verb-{'-'.join(argv[:3])}-{now_ms()}.json", reply)
        return reply

    def ping_overhead_ms(self, samples: int = 5) -> float | None:
        """CLI round-trip floor: every verb measurement includes this."""
        walls = []
        for _ in range(samples):
            r = self.verb("clients")
            if r["ok"] and r["wall_ms"] is not None:
                walls.append(r["wall_ms"])
            time.sleep(0.05)
        return statistics.median(walls) if walls else None

    def ytrace_events(self, since_ms: int, lines: int = 400) -> list[dict]:
        try:
            proc = subprocess.run(
                [YTRACE, "tail", "--lines", str(lines), "--json"],
                capture_output=True, text=True, timeout=15)
            events = json.loads(proc.stdout)
        except Exception:
            return []
        return [e for e in events if isinstance(e, dict)
                and e.get("ts_ms", 0) >= since_ms]

    def rows(self) -> list[dict]:
        r = self.verb("rows", "--json")
        if not r["ok"] or not r.get("json"):
            return []
        return (r["json"].get("data") or {}).get("rows") or []

    def find_row(self, path: str) -> dict | None:
        for row in self.rows():
            if row.get("full_path") == path:
                return row
        return None

    def rows_order_index(self, path: str) -> int | None:
        for i, row in enumerate(self.rows()):
            if row.get("full_path") == path:
                return i
        return None

    def dump_artifact(self, name: str, payload) -> None:
        if not self.artifacts:
            return
        safe = "".join(c if c.isalnum() or c in "-._" else "_" for c in name)
        try:
            with open(os.path.join(self.artifacts, safe), "w") as fh:
                json.dump(payload, fh, indent=1, default=str)
        except OSError:
            pass

    # ---- scratch-row lifecycle ----------------------------------------

    def clean_stale(self) -> int:
        """Remove scratch rows left by an earlier crashed run (ours by title)."""
        removed = 0
        for row in self.rows():
            label = row.get("label") or ""
            if label.startswith(PROBE_TITLE_PREFIX):
                path = row.get("full_path")
                if path and self.remove_row(path):
                    removed += 1
        return removed

    def remove_row(self, path: str) -> bool:
        """session remove + poll-gone. ONLY legal on paths this run created
        (or stale scratch rows matching the probe title prefix)."""
        r = self.verb("session", "remove", path, timeout=30)
        verified = bool((r.get("json") or {}).get("data", {}).get("verified"))
        deadline = time.time() + 15
        while time.time() < deadline:
            if self.find_row(path) is None:
                if path in self.spawned_paths:
                    self.spawned_paths.remove(path)
                return verified
            time.sleep(0.08)
        return False

    def spawn_scratch_row(self, index: int, activate: bool = False) -> dict:
        """One scratch terminal row; returns the probe row-dict (not a report)."""
        title = f"{PROBE_TITLE_PREFIX}{index}-{now_ms()}"
        argv = ["terminal", "new", "--title", title, "--cwd", "/tmp"]
        if not activate:
            argv.append("--no-activate")
        t0 = now_ms()
        r = self.verb(*argv)
        if not r["ok"]:
            return {"error": f"terminal new failed: {r.get('error')}"}
        path = self.extract_session_path(r, title)
        if not path:
            return {"error": "no session path in reply or rows diff"}
        self.spawned_paths.append(path)
        row = {"path": path, "title": title, "cli_ms": r["wall_ms"],
               "t0_ms": t0}
        data = (r.get("json") or {}).get("data") or {}
        row["daemon_processed_ms"] = (
            (r["json"].get("completed_at_ms") - t0)
            if r.get("json", {}).get("completed_at_ms") else None)
        row["queued"] = data.get("queued")
        paint = self.wait_paint_milestones(path, since_ms=t0)
        row["paint"] = paint
        return row

    def extract_session_path(self, reply: dict, title: str) -> str | None:
        """Pull local://<uuid> out of the reply, else fall back to a live
        rows lookup by the probe title."""
        def walk(node):
            if isinstance(node, str) and node.startswith("local://"):
                return node
            if isinstance(node, dict):
                for v in node.values():
                    hit = walk(v)
                    if hit:
                        return hit
            if isinstance(node, list):
                for v in node:
                    hit = walk(v)
                    if hit:
                        return hit
            return None
        hit = walk(reply.get("json"))
        if hit:
            return hit
        for row in self.rows():
            if (row.get("label") or "").startswith(title):
                return row.get("full_path")
        return None

    def wait_paint_milestones(self, path: str, since_ms: int,
                              timeout_s: float | None = None) -> dict:
        """Milestone ladder for one spawn, keyed by first ts per marker.

        first_frame (first WRITE→frame) does NOT fire for idle shells —
        the paint answer is xterm_paint/settle (or mount_open). Which
        marker answered is part of the result; a null ladder entry that
        never fired is a fact, not an error.
        """
        uuid = path.split("://")[-1]
        markers = {
            "open_attempt_begin": ("terminal_open_attempt", "begin"),
            "mount_begin": ("terminal_mount", "begin"),
            "mount_open": ("xterm_paint", "mount_open"),
            "paint_settle": ("xterm_paint", "settle"),
            "first_frame": ("xterm_paint", "first_frame"),
        }
        found: dict[str, dict] = {}
        deadline = time.time() + (timeout_s or self.timeout_s)
        while time.time() < deadline and len(found) < len(markers):
            for e in self.ytrace_events(since_ms):
                for key, (cat, name) in markers.items():
                    if key in found:
                        continue
                    if e.get("category") != cat or e.get("name") != name:
                        continue
                    if uuid not in json.dumps(e):
                        continue
                    found[key] = {"ts_ms": e["ts_ms"],
                                  "offset_ms": e["ts_ms"] - since_ms,
                                  "payload": e.get("payload")}
            if len(found) < len(markers):
                time.sleep(0.25)
        ff = found.get("first_frame", {})
        paint_marker = ("first_frame" if "first_frame" in found
                        else "paint_settle" if "paint_settle" in found
                        else "mount_open" if "mount_open" in found
                        else None)
        return {"milestones": found,
                "paint_marker": paint_marker,
                "spawn_to_paint_ms": found[paint_marker]["offset_ms"]
                if paint_marker else None,
                "open_to_write_ms": ff.get("payload", {}).get("open_to_write_ms"),
                "write_to_frame_ms": ff.get("payload", {}).get("write_to_frame_ms"),
                "blank_frames_before_write":
                    ff.get("payload", {}).get("blank_frames_before_write")}

    # ---- actions -------------------------------------------------------

    def action_spawn(self, iters: int) -> dict:
        out = {"iterations": []}
        for i in range(iters):
            row = self.spawn_scratch_row(i, activate=True)
            if "error" in row:
                out["iterations"].append({"error": row["error"]})
                continue
            paint = row["paint"]
            present = self.find_row(row["path"]) is not None
            acc = []
            if not present:
                acc.append("row absent from server app rows after spawn")
            if paint.get("paint_marker") is None:
                acc.append("no paint milestone fired within timeout")
            out["iterations"].append({
                "spawn_to_paint_ms": paint.get("spawn_to_paint_ms"),
                "paint_marker": paint.get("paint_marker"),
                "daemon_processed_ms": row.get("daemon_processed_ms"),
                "queued": row.get("queued"),
                "cli_new_ms": row["cli_ms"],
                "milestone_offsets_ms": {k: v["offset_ms"]
                                         for k, v in paint["milestones"].items()},
                "open_to_write_ms": paint.get("open_to_write_ms"),
                "write_to_frame_ms": paint.get("write_to_frame_ms"),
                "blank_frames_before_write": paint.get("blank_frames_before_write"),
                "accuracy_failures": acc,
            })
        return summarize(out, key="spawn_to_paint_ms",
                         rate_key="blank_frames_before_write")

    def action_drag(self, iters: int) -> dict:
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows", "iterations": []}
        a_path, b_path = rows_ready[0], rows_ready[1]
        out = {"iterations": []}
        for i in range(iters):
            t0 = now_ms()
            rb = self.verb("drag", "begin", a_path)
            rh = self.verb("drag", "hover", b_path, "--placement", "before")
            rd = self.verb("drag", "drop")
            commit_ms = now_ms() - t0
            settled, settle_ms = self.wait_order(a_path, b_path)
            events = {e.get("name") for e in self.ytrace_events(t0)
                      if e.get("name")}
            acc = []
            if not (rb["ok"] and rh["ok"] and rd["ok"]):
                acc.append("verb failure begin=%s hover=%s drop=%s errors=%s" % (
                    rb["ok"], rh["ok"], rd["ok"],
                    [v.get("error") for v in (rb, rh, rd) if not v["ok"]]))
            if not settled:
                acc.append("order did not reflect [A before B] within timeout")
            out["iterations"].append({
                "commit_ms": commit_ms,
                "settle_ms": settle_ms,
                "begin_ms": rb["wall_ms"], "hover_ms": rh["wall_ms"],
                "drop_ms": rd["wall_ms"],
                "accuracy_failures": acc,
                "trace_events_in_window": sorted(e for e in events
                                                 if e and "drag" in e.lower())
                                                or None,
            })
            # drag back so iterations stay independent
            t0 = now_ms()
            self.verb("drag", "begin", a_path)
            self.verb("drag", "hover", b_path, "--placement", "after")
            self.verb("drag", "drop")
            self.wait_order(b_path, a_path)
        return summarize(out, key="commit_ms")

    def action_group(self, iters: int) -> dict:
        rows_ready = self.ensure_two_scratch_rows()
        if len(rows_ready) < 2:
            return {"error": "could not establish two scratch rows", "iterations": []}
        a_path, b_path = rows_ready[0], rows_ready[1]
        out = {"iterations": []}
        for i in range(iters):
            t0 = now_ms()
            ri = self.verb("row-set", a_path, "--into", b_path)
            nested, settle_ms = self.wait_depth(a_path, expected_child_of=b_path)
            into_ms = now_ms() - t0
            acc = []
            if not ri["ok"]:
                acc.append(f"row-set --into failed: {ri.get('error')}")
            if not nested:
                acc.append("A did not become a child of B within timeout")
            # undo immediately — sticky verbs must not leak state
            ro = self.verb("row-set", a_path, "--out")
            unnested, out_settle_ms = self.wait_depth(a_path, expected_child_of=None)
            if not ro["ok"] or not unnested:
                acc.append("row-set --out did not restore the flat order")
            out["iterations"].append({
                "into_commit_ms": into_ms, "into_settle_ms": settle_ms,
                "out_settle_ms": out_settle_ms,
                "into_cli_ms": ri["wall_ms"],
                "accuracy_failures": acc,
            })
        return summarize(out, key="into_commit_ms")

    def action_menu(self, iters: int) -> dict:
        rows_ready = self.ensure_two_scratch_rows()
        if not rows_ready:
            return {"error": "no scratch row to probe", "iterations": []}
        path = rows_ready[0]
        out = {"iterations": []}
        for i in range(iters):
            self.verb("terminal", "focus", path)
            time.sleep(0.2)
            t0 = now_ms()
            r = self.verb("terminal", "probe-context-menu", path)
            menu_ms = now_ms() - t0
            reply = r.get("json") or {}
            data = reply.get("data") or {}
            items = data.get("items") or data.get("menu_items") or []
            acc = []
            if not r["ok"]:
                acc.append(f"probe-context-menu failed: {r.get('error')}")
            elif not items:
                acc.append("menu reply refused or empty: accepted=%s reason=%s"
                           % (data.get("accepted"), data.get("reason")))
            events = sorted({n for ev in self.ytrace_events(t0)
                             if isinstance((n := ev.get("name")), str)
                             and "menu" in n.lower()})
            out["iterations"].append({
                "menu_open_ms": menu_ms,
                "cli_ms": r["wall_ms"],
                "item_count": len(items) if items else None,
                "accuracy_failures": acc,
                "trace_events_in_window": events or None,
            })
        return summarize(out, key="menu_open_ms")

    def action_close(self) -> dict:
        """The teardown, instrumented — every spawned row closed exactly once."""
        out = {"iterations": []}
        for path in list(self.spawned_paths):
            row = self.find_row(path)
            t0 = now_ms()
            ok = self.remove_row(path)
            out["iterations"].append({
                "close_to_gone_ms": now_ms() - t0,
                "verified": ok,
                "accuracy_failures": [] if ok else ["row did not leave the live order"],
            })
        return summarize(out, key="close_to_gone_ms")

    # ---- waiters --------------------------------------------------------

    def wait_order(self, a_path: str, b_path: str,
                   timeout_s: float = 5.0) -> tuple[bool, int | None]:
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout_s:
            ia, ib = self.rows_order_index(a_path), self.rows_order_index(b_path)
            if ia is not None and ib is not None and ib - ia == 1:
                return True, int((time.perf_counter() - t0) * 1000)
            time.sleep(0.3)
        return False, None

    def wait_depth(self, a_path: str, expected_child_of: str | None,
                   timeout_s: float = 5.0) -> tuple[bool, int | None]:
        """Relative-depth wait: scratch rows live INSIDE a group already, so
        nesting is A.depth == B.depth + 1, never an absolute depth."""
        b = self.find_row(expected_child_of) if expected_child_of else None
        b_depth = b.get("depth", 0) if b else 0
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < timeout_s:
            a = self.find_row(a_path)
            if expected_child_of:
                if a and a.get("depth") == b_depth + 1:
                    return True, int((time.perf_counter() - t0) * 1000)
            elif a and a.get("depth") == b_depth:
                return True, int((time.perf_counter() - t0) * 1000)
            time.sleep(0.3)
        return False, None

    def ensure_two_scratch_rows(self) -> list[str]:
        live = [p for p in self.spawned_paths if self.find_row(p) is not None]
        while len(live) < 2:
            row = self.spawn_scratch_row(len(live))
            if "error" in row:
                break
            live = [p for p in self.spawned_paths
                    if self.find_row(p) is not None]
        return live[:2]


def summarize(out: dict, key: str, rate_key: str | None = None) -> dict:
    iters = out.get("iterations") or []
    vals = [i[key] for i in iters
            if isinstance(i.get(key), (int, float))]
    out["n"] = len(iters)
    out[f"{key}_p50"] = statistics.median(vals) if vals else None
    out[f"{key}_max"] = max(vals) if vals else None
    if rate_key:
        nums = [i.get(rate_key) for i in iters]
        out[f"{rate_key}_positive_rate"] = (
            sum(1 for v in nums if isinstance(v, (int, float)) and v > 0)
            / len(nums)) if nums else None
    fails = [f for i in iters for f in i.get("accuracy_failures", [])]
    out["accuracy_failures"] = fails
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--actions", default="spawn,drag,group,menu,close")
    ap.add_argument("--iters", type=int, default=3)
    ap.add_argument("--out", default="/tmp/uxspeed-report.json")
    ap.add_argument("--artifacts", default="/tmp/uxspeed-artifacts")
    ap.add_argument("--timeout-ms", type=int, default=12000)
    ap.add_argument("--keep", action="store_true",
                    help="do not close the scratch rows (debugging)")
    ap.add_argument("-v", action="store_true")
    args = ap.parse_args()

    probe = Probe(args)
    actions = [a.strip() for a in args.actions.split(",") if a.strip()]
    report: dict = {"schema": "uxspeed-uxprobe-v1",
                    "started": now_ms(), "plane": "live-gui",
                    "scratch_title_prefix": PROBE_TITLE_PREFIX,
                    "actions": {}}

    stale = probe.clean_stale()
    if stale:
        log(f"removed {stale} stale scratch row(s) from an earlier run")

    report["cli_overhead_ms"] = probe.ping_overhead_ms()
    report["cli_overhead_end_ms"] = None
    log(f"cli overhead floor: {report['cli_overhead_ms']} ms")

    try:
        for action in actions:
            log(f"action {action} …")
            if action == "spawn":
                report["actions"]["spawn"] = probe.action_spawn(args.iters)
            elif action == "drag":
                report["actions"]["drag"] = probe.action_drag(args.iters)
            elif action == "group":
                report["actions"]["group"] = probe.action_group(args.iters)
            elif action == "menu":
                report["actions"]["menu"] = probe.action_menu(args.iters)
            elif action == "close":
                report["actions"]["close"] = probe.action_close()
            else:
                report["actions"][action] = {"error": f"unknown action {action}"}
            log(f"  {json.dumps(report['actions'][action], default=str)[:300]}")
    finally:
        if not args.keep:
            log("teardown: closing scratch rows …")
            teardown = probe.action_close()
            if "close" not in report["actions"]:
                report["actions"]["close"] = teardown
        report["ended"] = now_ms()
        report["rows_left_behind"] = len(probe.spawned_paths)
        end_floor = probe.ping_overhead_ms(samples=3)
        report["cli_overhead_end_ms"] = end_floor
        if end_floor and report["cli_overhead_ms"]:
            drift = end_floor - report["cli_overhead_ms"]
            report["harness_load_drift_ms"] = drift
            log(f"overhead drift {drift:+d} ms (the probe's own pollution proxy)")
        with open(args.out, "w") as fh:
            json.dump(report, fh, indent=1, default=str)
        log(f"report → {args.out}")
    return 0


def log(msg: str) -> None:
    print(f"[uxprobe] {msg}", file=sys.stderr, flush=True)


if __name__ == "__main__":
    sys.exit(main())
