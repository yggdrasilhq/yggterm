#!/usr/bin/env python3
"""trace_slice — grep a yggterm event-trace.jsonl by time window, event
name(s), and/or payload substring, with LOCAL timestamps. The campaign's
every-sitting instrument (the [11.187] law: probe your own slices).

Usage:
  trace_slice.py <trace.jsonl> [--from "2026-10-05 08:19:40"] [--to "08:20:30"]
                  [--name mount_open,js_ready] [--payload 4314d6ee] [--limit 100]

--from/--to accept "HH:MM:SS" (today) or "YYYY-MM-DD HH:MM:SS"; the window
is inclusive. Output: "<local ts> <component> <name> <payload[:220]>".
"""
import argparse, json, datetime, sys

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("trace")
    ap.add_argument("--from", dest="frm")
    ap.add_argument("--to", dest="to")
    ap.add_argument("--name")
    ap.add_argument("--payload")
    ap.add_argument("--limit", type=int, default=100)
    a = ap.parse_args()
    names = set(a.name.split(",")) if a.name else None
    def parse(s):
        if not s:
            return None
        if len(s) <= 8:
            s = datetime.date.today().strftime("%Y-%m-%d ") + s
        return datetime.datetime.strptime(s, "%Y-%m-%d %H:%M:%S")
    lo = parse(a.frm); hi = parse(a.to)
    out = 0
    with open(a.trace, errors="replace") as f:
        for line in f:
            try:
                e = json.loads(line)
            except Exception:
                continue
            ts = e.get("ts_ms")
            if not isinstance(ts, (int, float)):
                continue
            dt = datetime.datetime.fromtimestamp(ts / 1000)
            if lo and dt < lo: continue
            if hi and dt > datetime.datetime.combine(hi.date(), hi.time()) if False else hi and dt > hi: continue
            name = e.get("name", "")
            if names and name not in names: continue
            p = str(e.get("payload", {}))
            if a.payload and a.payload not in p and a.payload not in line: continue
            print(dt.strftime("%d %H:%M:%S.%f")[:-3], e.get("component"), name, p[:220])
            out += 1
            if out >= a.limit: break
    print(f"-- {out} event(s)", file=sys.stderr)

if __name__ == "__main__":
    main()
