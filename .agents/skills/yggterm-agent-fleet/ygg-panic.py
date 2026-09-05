#!/usr/bin/env python3
"""ygg-panic — the resource watchdog for seat 6.7, built like the booter.

    ygg-panic.py subscribe --row <row-address> [--interval 3600]
    ygg-panic.py tick [--dry-run]      # measure once; wake on breach
    ygg-panic.py watch [--forever]     # the loop (systemd timer or nohup)
    ygg-panic.py status | list | unsubscribe --row <row>

⭐ WHY THIS EXISTS RATHER THAN A CRON. Owner directive 2026-08-14: *the panic
subscription should be on you and your successors, and successors must not
forget to arm it.* A session-scoped cron dies with the session that made it, so
the watch silently stops at exactly the moment a seat hands over — which is the
moment nobody is looking. The subscription therefore lives on DISK, keyed to the
SEAT, and any successor inherits it by claiming the seat rather than by
remembering a step.

⛔ IT DOES NOT TYPE INTO ROWS ITSELF, AND THAT IS DELIBERATE. Writing into a row
a human may be typing in is how this campaign spliced a message into someone's
half-finished sentence, and the safe way to wake a row is already built, tested
and screened in `ygg-booter.py` (it refuses a choice prompt, refuses an
unreadable screen, checks busy). Duplicating that would mean duplicating its
safety, and the second copy is always the one that rots. ⇒ On breach this
records the evidence and hands the WAKE to the booter.

⛔ MEASUREMENT LAWS THIS ENCODES, each paid for:
  * memory is `rss + swap`, never `rss` — RSS FALLS while the footprint climbs
  * `/tmp` on the laptop is a tmpfs, so bytes there are RAM
  * the fan has NO tachometer on that machine; it is proxied by sustained
    package power and die temperature
  * BLIND IS NOT BROKEN — an unreadable probe is reported as unknown, never as
    a pass and never as a breach
"""
import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
STATE = Path.home() / ".yggterm" / "relay" / "panic"
SUBS = STATE / "subs"
LATEST = STATE / "latest.json"
HISTORY = STATE / "history.jsonl"
BOOTER = HERE / "ygg-booter.py"
# ⛔ Repo-relative, because the scripts are the SSOT for the thresholds and this
#    file must not grow a second copy of them.
REPO = HERE.parent.parent.parent
PANIC_SH = REPO / "scripts" / "ygg-resource-panic.sh"
GUARD_SH = REPO / "scripts" / "ygg-scratch-guard.sh"


def log(msg):
    print(f"{time.strftime('%H:%M:%S')} ygg-panic {msg}", file=sys.stderr)


def row_uuid(row):
    return (row or "").rstrip("/").split("/")[-1]


def sub_path(row):
    return SUBS / f"{row_uuid(row)}.json"


def subs():
    if not SUBS.is_dir():
        return []
    out = []
    for f in sorted(SUBS.glob("*.json")):
        try:
            out.append(json.loads(f.read_text()))
        except Exception:
            # ⛔ An unreadable subscription is NOT an absent one. Say so, and
            #    keep the file: deleting it would silently unwatch a seat.
            out.append({"row": f.stem, "unreadable": True})
    return out


def cmd_subscribe(a):
    if not a.row or "://" not in a.row:
        log("⛔ subscribe needs --row <scheme://machine/uuid>, not a bare uuid")
        return 2
    SUBS.mkdir(parents=True, exist_ok=True)
    rec = {
        "row": a.row,
        "interval_s": a.interval,
        "seat": a.seat,
        "subscribed_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "note": a.note or "resource panic watch",
    }
    sub_path(a.row).write_text(json.dumps(rec, indent=2))
    back = [s for s in subs() if s.get("row") == a.row]
    log(f"subscribed {row_uuid(a.row)[:8]} every {a.interval}s")
    log(f"read-back: {'present' if back else '⛔ ABSENT — it did not land'}")
    return 0 if back else 1


def cmd_unsubscribe(a):
    p = sub_path(a.row)
    if p.exists():
        p.unlink()
        log(f"unsubscribed {row_uuid(a.row)[:8]}")
    else:
        log("no such subscription")
    return 0


def cmd_list(_a):
    s = subs()
    if not s:
        log("⛔ NOBODY IS SUBSCRIBED — the resource watch is not running")
        return 1
    for r in s:
        log(f"{row_uuid(r.get('row'))[:8]}  seat={r.get('seat')}  every {r.get('interval_s')}s")
    return 0


def run(cmd, timeout=300):
    """Run a probe. Returns (exit_code|None, text). None means COULD NOT ASK."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except Exception as exc:
        return None, f"probe failed to run: {exc}"


def measure():
    """One sweep, ordered by the owner's priority: MEMORY > CPU > SPACE."""
    result = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
    rc, out = run([str(PANIC_SH), "--json"])
    if rc is None:
        # ⛔ BLIND, not clear. A watchdog that reports "fine" when it could not
        #    look is worse than no watchdog.
        result["panic"] = "unknown"
        result["detail"] = out
    else:
        try:
            result.update(json.loads(out.strip().splitlines()[-1]))
        except Exception:
            result["panic"] = "unknown"
            result["detail"] = out[-800:]
    return result


def breached(r):
    """True only on a REAL breach. `unknown` is reported, never treated as one."""
    return bool(r.get("panic") is True)


def cmd_tick(a):
    STATE.mkdir(parents=True, exist_ok=True)
    r = measure()
    LATEST.write_text(json.dumps(r, indent=2))
    with HISTORY.open("a") as fh:
        fh.write(json.dumps(r) + "\n")

    summary = r.get("summary", "")
    log(f"panic={r.get('panic')} | {summary}")

    if r.get("panic") == "unknown":
        log("⚠ at least one probe was BLIND this tick — reported, not scored")

    if not breached(r):
        log("clear")
        return 0

    # SPACE breaches self-heal: reaping stale scratch needs no human.
    if "tmpfs" in str(r.get("panics", "")):
        rc, _ = run([str(GUARD_SH), "--enforce"], timeout=600)
        log(f"ran the scratch guard with --enforce (rc={rc})")

    watchers = subs()
    if not watchers:
        log("⛔ BREACH WITH NO SUBSCRIBER — nothing will be woken. Subscribe a seat.")
        return 1

    for w in watchers:
        if w.get("unreadable"):
            log(f"⚠ subscription {w.get('row')} is unreadable — skipping, NOT deleting")
            continue
        if a.dry_run:
            log(f"[dry-run] would wake {row_uuid(w['row'])[:8]}")
            continue
        # ⛔ The booter owns waking. It screens for a choice prompt, an
        #    unreadable screen and a busy row; this tool must not learn to type.
        rc, out = run([sys.executable, str(BOOTER), "subscribe",
                       "--row", w["row"], "--campaign", "panic",
                       "--note", f"resource panic: {summary[:180]}"], timeout=120)
        log(f"handed the wake to the booter for {row_uuid(w['row'])[:8]} (rc={rc})")
    return 1


def cmd_watch(a):
    while True:
        cmd_tick(a)
        if not a.forever:
            return 0
        time.sleep(max(60, a.interval))


def cmd_status(_a):
    if LATEST.exists():
        log(LATEST.read_text().strip()[:600])
    else:
        log("no tick has run yet")
    return cmd_list(_a)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("action", choices=["subscribe", "unsubscribe", "tick",
                                       "watch", "status", "list"])
    ap.add_argument("--row")
    ap.add_argument("--seat", default="6.7")
    ap.add_argument("--note")
    ap.add_argument("--interval", type=int, default=3600)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--forever", action="store_true")
    a = ap.parse_args()
    return {
        "subscribe": cmd_subscribe, "unsubscribe": cmd_unsubscribe,
        "tick": cmd_tick, "watch": cmd_watch,
        "status": cmd_status, "list": cmd_list,
    }[a.action](a)


if __name__ == "__main__":
    sys.exit(main())
