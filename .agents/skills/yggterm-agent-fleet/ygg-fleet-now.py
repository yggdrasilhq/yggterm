#!/usr/bin/env python3
"""ygg-fleet-now.py — the FLEET-NOW view (T-006, structure dream ACK-15c236ca5a).

One view over every namespace's dreamer-synthesized NOW.md: the first
§CURRENT_STATUS line plus the ⚠/⛔ trap lines of each namespace, so a routing
seat or the owner reads the whole fleet in ~3k tokens instead of 26 cold door
reads (~14k). It is a VIEW, not a store — the hub doors stay SSOT, nothing is
written anywhere, and the output is regenerated on every run so it cannot go
stale the way a cached rollup would.

Usage:
  ygg-fleet-now.py [--root ~/.yggterm/memory] [--all-lines] [--json] [--ns-filter SUBSTR]
  python3 ygg-fleet-now.py --json          # machine form for boards/tools
"""
import argparse, json, os, sys

DEF_ROOT = os.path.expanduser("~/.yggterm/memory/namespaces")
PINNED_FIRST = ("_global",)
TRAP_MARKS = ("⚠", "⛔")


def parse_now(path):
    """Return (as_of, first_status_line, trap_lines, status_count)."""
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError as e:
        return None, f"(unreadable: {e})", [], 0
    as_of = None
    for line in text.splitlines():
        ls = line.strip()
        if ls.startswith("as_of:"):
            as_of = ls.split(":", 1)[1].strip()
            break
    in_status, status_lines, traps = False, [], []
    for line in text.splitlines():
        s = line.rstrip()
        if s.startswith("## "):
            in_status = "CURRENT_STATUS" in s
            continue
        if not s or s.startswith("#"):
            continue
        if any(m in s for m in TRAP_MARKS):
            traps.append(s.strip())
        if in_status and s.startswith("- ") and not status_lines:
            status_lines.append(s[2:].strip())
    first = status_lines[0] if status_lines else "(empty §CURRENT_STATUS)"
    return as_of, first, traps, len(status_lines)


def main():
    ap = argparse.ArgumentParser(description="fleet-wide NOW.md rollup view (read-only)")
    ap.add_argument("--root", default=DEF_ROOT)
    ap.add_argument("--all-lines", action="store_true",
                    help="include every §CURRENT_STATUS line, not just the first")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--ns-filter", default=None, help="substring filter on namespace name")
    a = ap.parse_args()
    if not os.path.isdir(a.root):
        print(f"no hub namespaces dir at {a.root}", file=sys.stderr)
        sys.exit(2)
    rows = []
    ns_total = 0
    for ns in sorted(os.listdir(a.root)):
        nd = os.path.join(a.root, ns)
        if not os.path.isdir(nd):
            continue
        ns_total += 1
        if a.ns_filter and a.ns_filter.lower() not in ns.lower():
            continue
        p = os.path.join(nd, "NOW.md")
        if not os.path.exists(p):
            continue
        as_of, first, traps, n_status = parse_now(p)
        rows.append({"ns": ns, "as_of": as_of, "first": first,
                     "traps": traps if a.all_lines else traps[:2],
                     "status_lines": n_status})
    rows.sort(key=lambda r: (r["ns"] not in PINNED_FIRST, r["ns"]))
    if a.json:
        print(json.dumps({"namespaces_total": ns_total, "now_files": len(rows),
                          "rows": rows}, ensure_ascii=False, indent=1))
        return
    print(f"FLEET-NOW — {len(rows)}/{ns_total} namespaces have a NOW rollup "
          f"(view only; doors are SSOT — `ygg-memory get --file NOW.md --ns=<ns>`)\n")
    for r in rows:
        stamp = r["as_of"] or "?"
        print(f"## {r['ns']}  [as-of: {stamp}]")
        print(f"  {r['first']}")
        for t in r["traps"]:
            print(f"  {t}")
        print()
    total_chars = sum(len(r["first"]) + sum(len(t) for t in r["traps"]) + len(r["ns"]) + 20
                      for r in rows)
    print(f"(~{total_chars // 4} tokens; {sum(len(r['traps']) for r in rows)} trap lines surfaced)")


if __name__ == "__main__":
    main()
