#!/usr/bin/env python3
"""
check-cwdtree.py — manual ground-truth verifier for `server cwdtree ls`.

For each host (local + fleet ssh targets), this script:

1. Calls `yggterm-headless server cwdtree ls --json` on that host (the
   Rust verb that groups `scan_all_durable_sessions` by cwd with AGENT_CLIS
   icon/kind dispatch).
2. Manually walks the raw store files on that host via `ssh <host> find ...`
   and re-parses each file in Python, *independently* of Rust, to produce
   ground truth cwd-grouping + icon dispatch. Compares group membership,
   icon mismatches, and missing/extra sessions.

Usage:
  python3 scripts/check-cwdtree.py                          # all hosts
  python3 scripts/check-cwdtree.py --host oc --host dev
  python3 scripts/check-cwdtree.py --json > report.json
  python3 scripts/check-cwdtree.py --verbose

Exit 0 when every host's verb matches manual walk; exit 2 on mismatch.
"""

import argparse
import json
import os
import shlex
import subprocess
import sys
import re
from pathlib import Path
from collections import defaultdict

# Keep in sync with AGENT_CLIS in crates/yggterm-core/src/agent_cli.rs
# Glyphs/colors must match Rust's icon_glyph / brand_color exactly — a stale
# C_ vs >_ would read as a mismatch even when the verb is truthful.
CLI_STORES = [
    {"slug": "codex", "globs": [".codex/sessions/**/rollout-*.jsonl"], "exclude": [".bak."], "kind": "codex", "glyph": ">_", "color": "#0f766e"},
    {"slug": "codex-litellm", "globs": [".codex-litellm/sessions/**/rollout-*.jsonl"], "exclude": [".bak."], "kind": "codex-litellm", "glyph": ">_", "color": "#0369a1"},
    {"slug": "claude-code", "globs": [".claude/projects/*/*.jsonl"], "exclude": [], "kind": "claude-code", "glyph": "*_", "color": "#c2410c"},
    {"slug": "pi", "globs": [".pi/agent/sessions/*/*.jsonl"], "exclude": [], "kind": "pi", "glyph": "π_", "color": "#be185d"},
    {"slug": "qwen", "globs": [".qwen/projects/*/chats/*.jsonl"], "exclude": [], "kind": "qwen", "glyph": "Q_", "color": "#6d28d9"},
    {"slug": "antigravity", "globs": [".gemini/antigravity-cli/conversations/*.db"], "exclude": ["-shm", "-wal"], "kind": "antigravity", "glyph": "A_", "color": "#1557b0"},
    {"slug": "grok", "globs": [".grok/sessions/*/*/summary.json"], "exclude": [], "kind": "grok-build", "glyph": "G_", "color": "#000000"},
    {"slug": "muse", "globs": [".local/share/muse/sessions/**/session.jsonl"], "exclude": ["/subagent/", "/tool-outputs/"], "kind": "muse", "glyph": "M_", "color": "#86198f"},
]

KIND_TO_GLYPH = {c["kind"]: c["glyph"] for c in CLI_STORES}
KIND_TO_COLOR = {c["kind"]: c["color"] for c in CLI_STORES}


def fleet_hosts():
    hosts = ["local"]
    try:
        out = subprocess.check_output(["yggterm-headless", "server", "daemons", "--json"], text=True, timeout=5)
        data = json.loads(out)
        for entry in data.get("daemons", []) + data.get("machines", []):
            label = entry.get("label") or entry.get("host") or entry.get("machine_key")
            if label and label not in hosts:
                hosts.append(label)
    except Exception:
        pass
    try:
        out = subprocess.check_output(["bash", "-c", "grep -h '^Host ' ~/.ssh/config 2>/dev/null | awk '{print $2}'"], text=True)
        for h in out.split():
            if h not in hosts and h not in ("*",):
                hosts.append(h)
    except Exception:
        pass
    env = os.environ.get("YGGTERM_CHECK_HOSTS")
    if env:
        hosts = [h.strip() for h in env.split(",") if h.strip()]
    return hosts


def run_on_host(host, cmd, timeout=15):
    if host == "local":
        full = cmd
    else:
        full = f"ssh -o ConnectTimeout=5 -o BatchMode=yes {shlex.quote(host)} {shlex.quote(cmd)}"
    try:
        out = subprocess.check_output(full, shell=True, text=True, timeout=timeout, stderr=subprocess.STDOUT)
        return out, None
    except subprocess.CalledProcessError as e:
        return e.output, f"exit {e.returncode}"
    except Exception as e:
        return "", str(e)


def verb_on_host(host):
    cmd = "yggterm-headless server cwdtree ls --json --limit 10000 2>&1 || yggterm server cwdtree ls --json --limit 10000 2>&1"
    out, err = run_on_host(host, cmd)
    if err:
        return None, err, out
    try:
        data = json.loads(out)
        return data, None, out
    except Exception as e:
        return None, f"json parse: {e}", out[:2000]


def parse_cwd_from_file(host, path, cli_slug):
    """Return cwd for this session file via ssh, independent of Rust."""
    if cli_slug == "muse":
        if "/subagent/" in path or "/tool-outputs/" in path:
            return None, None
        # Try DB first (same as Rust primary), fallback to route_facts cwd via JSONL grep
        home_out, _ = run_on_host(host, "echo $HOME")
        home = home_out.strip() if home_out else os.path.expanduser("~")
        # Extract session_id from parent dir name
        sid = Path(path).parent.name
        # Query session-index.db workspace_root
        # workspace_root is cwdForTitle in DB; we use same as Rust
        cmd = f"sqlite3 {shlex.quote(home + '/.local/share/muse/session-index.db')} \"SELECT workspace_root FROM sessions WHERE session_id='{sid}' LIMIT 1;\" 2>/dev/null | head -n 1"
        out, _ = run_on_host(host, cmd)
        if out and out.strip():
            return out.strip(), None
        # Fallback: grep route_facts cwd
        cmd2 = f"grep -m1 'route_facts' {shlex.quote(path)} 2>/dev/null | head -c 4000"
        out2, _ = run_on_host(host, cmd2)
        if out2:
            m = re.search(r'\"cwd\"\s*:\s*\"([^\"]+)\"', out2)
            if m:
                cwd = m.group(1)
                # Muse legacy cwd workaround: if cwd < 8 chars, treat as title
                if len(cwd) < 8:
                    return None, cwd
                return cwd, None
        # Last fallback: HOME
        return home, None
    elif cli_slug == "claude-code":
        # Mirror Rust's read_cc_session_identity_fields: scan the whole file,
        # prefer the latest cwd whose encoding matches the parent project dir
        # (the session may have /cd'd), else the last cwd seen.
        # Encoding: alnum and '-' kept, everything else -> '-' (see cc_project_dir_encoding).
        session_id = Path(path).stem
        parent = Path(path).parent.name
        def cc_encode(cwd):
            return "".join(c if c.isalnum() or c == "-" else "-" for c in cwd)
        # Collect all cwds in order via grep -o for speed, then pick per Rust rules
        cmd = f"grep -o '\"cwd\":\"[^\"]*\"' {shlex.quote(path)} 2>/dev/null | head -n 200"
        out, _ = run_on_host(host, cmd)
        cwds = []
        if out:
            for m in re.finditer(r'"cwd":"([^"]+)"', out):
                cand = m.group(1).strip()
                if cand:
                    cwds.append(cand)
        # Also consider relocatedCwd as fallback
        if not cwds:
            cmd2 = f"grep -o '\"relocatedCwd\":\"[^\"]*\"' {shlex.quote(path)} 2>/dev/null | head -n 5"
            out2, _ = run_on_host(host, cmd2)
            if out2:
                for m in re.finditer(r'"relocatedCwd":"([^"]+)"', out2):
                    cand = m.group(1).strip()
                    if cand:
                        cwds.append(cand)
        if not cwds:
            return None, None
        # Prefer last placement-confirmed cwd
        for cand in reversed(cwds):
            if cc_encode(cand) == parent:
                return cand, None
        # Else last cwd overall
        return cwds[-1], None
    elif cli_slug in ("codex", "codex-litellm"):
        # Rust's read_codex_session_identity walks the whole file and
        # recurses into payload (find_string_field), so top-level j.get("cwd")
        # misses the real cwd at payload.cwd. Mirror that recursion here.
        def find_cwd_rec(v):
            if isinstance(v, dict):
                for kk, vv in v.items():
                    if kk == "cwd" and isinstance(vv, str) and vv.strip():
                        return vv.strip()
                    r = find_cwd_rec(vv)
                    if r:
                        return r
            elif isinstance(v, list):
                for it in v:
                    r = find_cwd_rec(it)
                    if r:
                        return r
            return None
        # Walk enough lines — Rust walks until both id and cwd found.
        cmd = f"cat {shlex.quote(path)} 2>/dev/null | head -c 50000"
        out, _ = run_on_host(host, cmd)
        for line in out.splitlines():
            line=line.strip()
            if not line: continue
            try:
                j=json.loads(line)
                cwd = find_cwd_rec(j)
                if cwd:
                    return cwd, None
            except: continue
        # Fallback grep for files where json is truncated by head -c
        cmd2 = f"grep -m1 '\"cwd\"' {shlex.quote(path)} 2>/dev/null | head -c 2000"
        out2, _ = run_on_host(host, cmd2)
        if out2:
            m = re.search(r'\"cwd\"\s*:\s*\"([^\"]+)\"', out2)
            if m:
                return m.group(1), None
        return None, None
    else:
        # pi/qwen/grok/antigravity: minimal
        cmd = f"head -n 5 {shlex.quote(path)} 2>/dev/null | head -c 8000"
        out, _ = run_on_host(host, cmd)
        for line in out.splitlines():
            try:
                j=json.loads(line)
                if j.get("cwd") or j.get("info", {}).get("cwd"):
                    return j.get("cwd") or j.get("info", {}).get("cwd"), None
            except: continue
        return None, None


def manual_walk_on_host(host):
    sessions = []
    home_out, err = run_on_host(host, "echo $HOME")
    if err:
        return sessions, f"cannot get HOME: {err}"
    home = home_out.strip() or os.path.expanduser("~")
    for cli in CLI_STORES:
        for glob in cli["globs"]:
            segs = glob.split("/")
            lit = []
            for seg in segs:
                if "*" in seg:
                    break
                lit.append(seg)
            prefix = "/".join(lit)
            base = f"{home}/{prefix}" if prefix else home
            last = glob.split("/")[-1]
            pattern = last
            if pattern == "*.db":
                find_cmd = f"find {shlex.quote(base)} -type f -name '*.db' 2>/dev/null | head -n 10000"
            elif pattern == "summary.json":
                find_cmd = f"find {shlex.quote(base)} -type f -name 'summary.json' 2>/dev/null | head -n 10000"
            else:
                find_cmd = f"find {shlex.quote(base)} -type f -name {shlex.quote(pattern)} 2>/dev/null | head -n 10000"
            out, _ = run_on_host(host, find_cmd)
            files = [l.strip() for l in out.splitlines() if l.strip()]
            for f in files:
                # Exclude by path fragment (covers Muse /subagent/) or file name
                if any(ex in f for ex in cli["exclude"]):
                    continue
                # stat mtime
                stat_out, _ = run_on_host(host, f"stat -c %Y {shlex.quote(f)} 2>/dev/null || stat -f %m {shlex.quote(f)} 2>/dev/null")
                try:
                    mtime = int(stat_out.strip())
                except:
                    mtime = 0
                cwd, title = parse_cwd_from_file(host, f, cli["slug"])
                if cli["slug"] == "muse" and cwd is None and title is None:
                    # skipped subagent fallback already; skip if no cwd
                    # Use home as fallback like Rust does, so include with home cwd
                    cwd = home
                if not cwd:
                    cwd = home
                # session_id: for Muse parent dir, for others stem/parsed
                if cli["slug"] == "muse":
                    session_id = Path(f).parent.name
                else:
                    session_id = Path(f).stem
                    if cli["slug"] in ("codex", "codex-litellm"):
                        # stem is rollout-<timestamp>-<uuid>
                        if session_id.startswith("rollout-"):
                            session_id = session_id[len("rollout-"):]
                            parts = session_id.split("-")
                            if len(parts) >= 5:
                                session_id = "-".join(parts[-5:])
                sessions.append({
                    "host": host,
                    "cli": cli["slug"],
                    "kind": cli["kind"],
                    "path": f,
                    "cwd": cwd,
                    "mtime": mtime,
                    "session_id": session_id,
                    "glyph": cli["glyph"],
                })
    sessions.sort(key=lambda s: s["mtime"], reverse=True)
    return sessions, None


def compare(host, verb_data, manual_sessions, verbose=False):
    issues = []
    if verb_data is None:
        issues.append("verb failed to return JSON")
        return issues
    verb_groups = verb_data.get("groups", [])
    verb_rows = []
    for g in verb_groups:
        for r in g.get("sessions", []):
            verb_rows.append(r)
            # Check icon dispatch
            exp_glyph = KIND_TO_GLYPH.get(r.get("kind"))
            if exp_glyph and r.get("icon_glyph") != exp_glyph:
                issues.append(f"icon mismatch {r.get('session_id','')[:8]} kind {r.get('kind')}: verb {r.get('icon_glyph')!r} != expected {exp_glyph!r}")
    verb_ids = set(r.get("session_id") for r in verb_rows)
    manual_ids = set(s["session_id"] for s in manual_sessions)
    # Group counts
    verb_group_cwds = set(g.get("cwd") for g in verb_groups)
    manual_groups = defaultdict(list)
    for s in manual_sessions:
        manual_groups[s["cwd"]].append(s)
    manual_group_cwds = set(manual_groups.keys())
    # Symmetric diff on ids
    verb_only = verb_ids - manual_ids
    manual_only = manual_ids - verb_ids
    if verb_only:
        issues.append(f"verb has {len(verb_only)} ids not in manual walk (extra): {sorted(list(verb_only))[:5]}")
    if manual_only:
        issues.append(f"manual has {len(manual_only)} ids not in verb (missing): {sorted(list(manual_only))[:5]}")
    # Cwd grouping drift
    missing_cwds = manual_group_cwds - verb_group_cwds
    extra_cwds = verb_group_cwds - manual_group_cwds
    if missing_cwds:
        issues.append(f"manual groups missing from verb: {sorted(list(missing_cwds))[:5]}")
    if extra_cwds:
        issues.append(f"verb groups extra vs manual: {sorted(list(extra_cwds))[:5]}")
    # Count check
    manual_count = len(manual_sessions)
    verb_count = len(verb_rows)
    if abs(verb_count - manual_count) > 10:
        issues.append(f"count drift: verb {verb_count} vs manual {manual_count}")
    # Group-count check
    if abs(len(verb_groups) - len(manual_groups)) > 2:
        issues.append(f"group count drift: verb {len(verb_groups)} vs manual {len(manual_groups)}")
    if verbose:
        print(f"[{host}] verb {verb_count} rows in {len(verb_groups)} groups, manual {manual_count} in {len(manual_groups)}")
        if verb_groups[:3]:
            print(f"  verb top groups: {[(g.get('cwd','')[:50], g.get('session_count')) for g in verb_groups[:3]]}")
            print(f"  manual top: {[(cwd[:50], len(v)) for cwd,v in list(sorted(manual_groups.items(), key=lambda kv: max(s['mtime'] for s in kv[1]), reverse=True))[:8]]}")
    return issues


def main():
    ap = argparse.ArgumentParser(description="Check cwdtree ls against manual jsonl walk")
    ap.add_argument("--host", action="append", dest="hosts", help="host to check (repeatable)")
    ap.add_argument("--json", action="store_true", help="machine-readable report")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--limit", type=int, default=200, help="max rows per host")
    args = ap.parse_args()

    hosts = args.hosts if args.hosts else fleet_hosts()
    if not hosts:
        hosts = ["local"]

    report = {}
    all_ok = True
    for host in hosts:
        verb_data, verb_err, verb_raw = verb_on_host(host)
        if verb_err:
            report[host] = {"ok": False, "error": verb_err, "raw": verb_raw[:2000]}
            all_ok = False
            if not args.json:
                print(f"[{host}] VERB ERROR: {verb_err}")
                print(verb_raw[:500])
            continue
        manual_sessions, manual_err = manual_walk_on_host(host)
        if manual_err:
            report[host] = {"ok": False, "error": manual_err, "verb": verb_data}
            all_ok = False
            continue
        issues = compare(host, verb_data, manual_sessions, verbose=args.verbose)
        ok = len(issues) == 0
        if not ok:
            all_ok = False
        report[host] = {
            "ok": ok,
            "issues": issues,
            "verb_count": sum(g.get("session_count", 0) for g in verb_data.get("groups", [])),
            "verb_groups": len(verb_data.get("groups", [])),
            "manual_count": len(manual_sessions),
            "manual_groups": len(set(s["cwd"] for s in manual_sessions)),
            "host": host,
            "durable_count": verb_data.get("durable_count"),
        }
        if not args.json:
            status = "OK" if ok else "MISMATCH"
            print(f"[{host}] {status}: verb {report[host]['verb_count']} in {report[host]['verb_groups']} groups, manual {report[host]['manual_count']} in {report[host]['manual_groups']}")
            for iss in issues:
                print(f"  - {iss}")

    if args.json:
        print(json.dumps(report, indent=2))
    if not all_ok:
        sys.exit(2)
    else:
        if not args.json:
            print("All hosts match — cwdtree ls is truthful.")
        sys.exit(0)


if __name__ == "__main__":
    main()
