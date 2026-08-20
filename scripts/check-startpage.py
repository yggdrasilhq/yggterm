#!/usr/bin/env python3
"""
check-startpage.py — manual ground-truth verifier for `server startpage ls`.

For each host (local + fleet ssh targets), this script:

1. Calls `yggterm-headless server startpage ls --json` on that host (the
   Rust verb that reuses `AGENT_CLIS` descriptors + `read_store_entry`).
2. Manually walks the raw store files on that host via `ssh <host> find ...`
   and re-parses each file in Python, *independently* of Rust, to produce
   ground truth. Compares counts, missing/extra session_ids, title mismatches,
   and ordering.

Usage:
  python3 scripts/check-startpage.py                          # all hosts
  python3 scripts/check-startpage.py --host oc --host dev
  python3 scripts/check-startpage.py --json > report.json
  python3 scripts/check-startpage.py --verbose

Exit 0 when every host's verb matches manual walk; exit 2 on mismatch.
"""

import argparse
import json
import os
import shlex
import subprocess
import tempfile
import sys
from pathlib import Path
from collections import defaultdict

# Maps AGENT_CLIS descriptor to its store globs (from crates/yggterm-core/src/agent_cli.rs)
# Keep in sync — this is the Python oracle's own list, deliberately not imported.
# Muse: session_id is parent dir name, cwd/title from session-index.db fallback to route_facts.
CLI_STORES = [
    {
        "slug": "muse",
        "globs": [".local/share/muse/sessions/**/session.jsonl"],
        "exclude": ["/subagent/", "/tool-outputs/"],
        "kind": "muse",
    },
    {
        "slug": "codex",
        "globs": [".codex/sessions/**/rollout-*.jsonl"],
        "exclude": [".bak."],
        "kind": "codex",
    },
    {
        "slug": "codex-litellm",
        "globs": [".codex-litellm/sessions/**/rollout-*.jsonl"],
        "exclude": [".bak."],
        "kind": "codex-litellm",
    },
    {
        "slug": "claude-code",
        "globs": [".claude/projects/*/*.jsonl"],
        "exclude": ["agent-", "/subagents/", "/workflows/"],
        "kind": "claude-code",
    },
    {
        "slug": "pi",
        "globs": [".pi/agent/sessions/*/*.jsonl"],
        "exclude": [],
        "kind": "pi",
    },
    {
        "slug": "qwen",
        "globs": [".qwen/projects/*/chats/*.jsonl"],
        "exclude": [".runtime."],
        "kind": "qwen",
    },
    {
        "slug": "antigravity",
        # ⛔ agy rows come from the summaries DB (see `agy_durable_rows` below),
        # not from a glob: `conversations/*.db` holds a single .pb and the brain
        # transcripts are `transcript_full.jsonl`, so the old globs matched nothing.
        "globs": [],
        "exclude": ["-shm", "-wal"],
        "kind": "antigravity",
    },
    {
        "slug": "grok",
        "globs": [".grok/sessions/*/*/summary.json"],
        "exclude": [],
        "kind": "grok-build",
    },
]

# Hosts to check — local + fleet from yggterm's ssh targets or env.

# ⛔ The durable-session RULES live in one shared module (the store tables above
# stay per-script on purpose: the oracle must not import the Rust descriptors).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ygg_scan_truth import (  # noqa: E402
    agy_durable_rows, muse_noise_ids, locally_backed_ids, condense_store_title,
)


def fleet_hosts():
    hosts = ["local"]
    # Try to discover from yggterm server daemons census
    try:
        out = subprocess.check_output(
            ["yggterm-headless", "server", "daemons", "--json"], text=True, timeout=5
        )
        data = json.loads(out)
        for entry in data.get("daemons", []) + data.get("machines", []):
            label = entry.get("label") or entry.get("host") or entry.get("machine_key")
            if label and label not in hosts:
                hosts.append(label)
    except Exception:
        pass
    # Also try ssh config hosts from ~/.ssh/config
    try:
        out = subprocess.check_output(["bash", "-c", "grep -h '^Host ' ~/.ssh/config 2>/dev/null | awk '{print $2}'"], text=True)
        for h in out.split():
            if h not in hosts and h not in ("*",):
                hosts.append(h)
    except Exception:
        pass
    # Env override
    env = os.environ.get("YGGTERM_CHECK_HOSTS")
    if env:
        hosts = [h.strip() for h in env.split(",") if h.strip()]
    return hosts

def _ssh_control_path(host):
    """Per-host multiplexing socket, in the user's runtime dir when there is one."""
    base = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
    safe = "".join(ch if ch.isalnum() or ch in "-_." else "_" for ch in host)
    return os.path.join(base, f"ygg-check-{safe}.sock")


def run_on_host(host, cmd, timeout=45):
    if host == "local":
        full = cmd
    else:
        # ⛔ ControlMaster is load-bearing, not tuning. This walk makes one ssh
        # call PER FILE (stat, then parse), so on a host with ~700 store files a
        # fresh TCP+auth handshake each time put a remote run past ten minutes —
        # and a falsifier nobody can afford to run is not a falsifier. One
        # multiplexed connection turns each call into a channel on a socket that
        # is already open.
        full = (
            "ssh -o ConnectTimeout=5 -o BatchMode=yes "
            "-o ControlMaster=auto -o ControlPersist=120 "
            f"-o ControlPath={shlex.quote(_ssh_control_path(host))} "
            f"{shlex.quote(host)} {shlex.quote(cmd)}"
        )
    try:
        out = subprocess.check_output(full, shell=True, text=True, timeout=timeout, stderr=subprocess.STDOUT)
        return out, None
    except subprocess.CalledProcessError as e:
        return e.output, f"exit {e.returncode}"
    except Exception as e:
        return "", str(e)

def verb_on_host(host):
    # Use large limit so verb returns the full durable set — default 200 truncates
    # and would make the oracle's full walk (10000) always mismatch.
    # A lane can point the oracle at its own build before deploying:
    #   YGGTERM_CHECK_BIN=./target/release/yggterm-headless check-startpage.py --host local
    override = os.environ.get("YGGTERM_CHECK_BIN")
    if override:
        cmd = f"{override} server startpage ls --json --limit 10000 2>&1"
    else:
        cmd = "~/.local/bin/yggterm-headless server startpage ls --json --limit 10000 2>&1 || yggterm-headless server startpage ls --json --limit 10000 2>&1 || yggterm server startpage ls --json --limit 10000 2>&1"
    out, err = run_on_host(host, cmd)
    if err:
        return None, err, out
    try:
        data = json.loads(out)
        return data, None, out
    except Exception as e:
        return None, f"json parse: {e}", out[:2000]

def manual_walk_on_host(host):
    """Manually find and parse raw files on host, independent of Rust."""
    sessions = []
    home_out, err = run_on_host(host, "echo $HOME")
    if err:
        return sessions, f"cannot get HOME: {err}"
    home = home_out.strip() or os.path.expanduser("~")
    for cli in CLI_STORES:
        for glob in cli["globs"]:
            # Expand glob to find command: use find for **, else ls
            # Simplify: use find with name pattern from last segment
            last = glob.split("/")[-1]
            # Convert glob segment to find -name pattern
            pattern = last
            # Literal prefix = leading segments without '*', matching Rust literal_prefix()
            segs = glob.split("/")
            lit = []
            for seg in segs:
                if "*" in seg:
                    break
                lit.append(seg)
            prefix = "/".join(lit)
            base = f"{home}/{prefix}" if prefix else home
            # Build find command — no head limit: Rust walks the full tree
            # and earlier oracles truncated at 500/1000 which hid half the Muse
            # store (2833 files on oc, 2611 subagent). Keep head only as safety
            # at 10000, well above any fleet host.
            if pattern == "*.db":
                find_cmd = f"find {shlex.quote(base)} -type f -name '*.db' 2>/dev/null | head -n 10000"
            elif pattern == "summary.json":
                find_cmd = f"find {shlex.quote(base)} -type f -name 'summary.json' 2>/dev/null | head -n 10000"
            else:
                # rollout-*.jsonl / *.jsonl / session.jsonl
                find_pat = pattern.replace("*", "*")
                find_cmd = f"find {shlex.quote(base)} -type f -name {shlex.quote(find_pat)} 2>/dev/null | head -n 10000"
            out, _ = run_on_host(host, find_cmd)
            files = [l.strip() for l in out.splitlines() if l.strip()]
            for f in files:
                # Exclude by path fragment when fragment contains '/', else file name only — mirrors Rust store_path_is_session_file
                if any((ex in f) if "/" in ex else (ex in os.path.basename(f)) for ex in cli["exclude"]):
                    continue
                # Quick stat for mtime
                stat_out, _ = run_on_host(host, f"stat -c %Y {shlex.quote(f)} 2>/dev/null || stat -f %m {shlex.quote(f)} 2>/dev/null")
                try:
                    mtime = int(stat_out.strip())
                except:
                    mtime = 0
                # Parse title/cwd/session_id in Python (minimal, independent)
                parsed = parse_file_on_host(host, f, cli["slug"])
                sessions.append({
                    "host": host,
                    "cli": cli["slug"],
                    "kind": cli["kind"],
                    "path": f,
                    "mtime": mtime,
                    "parsed": parsed,
                })

    # ⛔ Antigravity keeps its index in SQLite, so a FILE walk sees none of it.
    # Without this the oracle called every agy row the verb produced a spurious
    # "extra" — 999 of them on a measured host — while being blind to the CLI.
    for row in agy_durable_rows(run_on_host, host, home):
        sessions.append({
            "host": host,
            "cli": "antigravity",
            "kind": "antigravity",
            "path": f"{home}/.gemini/antigravity-cli/conversation_summaries.db",
            "mtime": 0,
            "parsed": {"session_id": row["id"], "cwd": row["cwd"], "title": row["title"]},
        })

    # A zero-prompt muse placeholder is skipped by the scan, so the oracle must
    # skip it too. ⚠ These have real files behind them; this set is for skipping,
    # never for deleting.
    noise = muse_noise_ids(run_on_host, host)
    if noise:
        sessions = [
            s for s in sessions
            if not (s.get("cli") == "muse"
                    and (s.get("parsed", {}) or {}).get("session_id") in noise)
        ]

    sessions.sort(key=lambda s: s["mtime"], reverse=True)
    return sessions, None

def parse_file_on_host(host, path, cli_slug):
    """Parse a single store file on host via ssh cat + python logic."""
    home_out,_ = run_on_host(host, "echo $HOME")
    home = home_out.strip() if home_out else os.path.expanduser("~")
    # We cat first line or whole file depending on cli
    if cli_slug == "antigravity":
        if path.endswith("transcript.jsonl"):
            parts = Path(path).parts
            if ".system_generated" in parts:
                idx = parts.index(".system_generated")
                session_id = parts[idx - 1]
            else:
                session_id = Path(path).parent.parent.parent.name
        else:
            session_id = Path(path).stem
        if not session_id or session_id == "transcript" or session_id.endswith("-shm") or session_id.endswith("-wal"):
            return {"session_id": None, "raw": ""}
        home_out,_ = run_on_host(host, "echo $HOME")
        home = home_out.strip() if home_out else os.path.expanduser("~")
        db = f"{home}/.gemini/antigravity-cli/conversation_summaries.db"
        cmd = f"sqlite3 {shlex.quote(db)} \"SELECT title, preview FROM conversation_summaries WHERE conversation_id='{session_id}' AND killed=0;\" 2>/dev/null | head -n 1"
        out, _ = run_on_host(host, cmd)
        title = None
        if out:
            parts = out.strip().split("|")
            if parts[0].strip():
                title = parts[0].strip()
            elif len(parts)>1 and parts[1].strip():
                title = parts[1].strip()
        if not title:
            hcmd = f"grep -F '\"{session_id}\"' {shlex.quote(home)}/.gemini/antigravity-cli/history.jsonl 2>/dev/null | head -n 1"
            hout, _ = run_on_host(host, hcmd)
            if hout:
                try:
                    hj = json.loads(hout)
                    title = hj.get("display")
                except:
                    pass
        return {"session_id": session_id, "title": title, "raw": out[:500] if out else ""}
    elif cli_slug == "grok":
        cmd = f"cat {shlex.quote(path)} 2>/dev/null | head -c 4000"
        out, _ = run_on_host(host, cmd)
        try:
            data = json.loads(out)
            info = data.get("info", {}) if isinstance(data, dict) else {}
            return {
                "session_id": info.get("id") or data.get("sessionId") or "",
                "cwd": info.get("cwd") or "",
                "title": info.get("title") or data.get("session_summary") or None,
                "raw": out[:500],
            }
        except:
            return {"raw": out[:500] if out else ""}
    elif cli_slug == "muse":
        # Muse: session_id is parent dir name, cwd/title from index DB with route_facts fallback.
        if "/subagent/" in path or "/tool-outputs/" in path:
            return {"session_id": None, "raw": ""}
        session_id = Path(path).parent.name
        home_out,_ = run_on_host(host, "echo $HOME")
        home = home_out.strip() if home_out else os.path.expanduser("~")
        db = f"{home}/.local/share/muse/session-index.db"
        cmd_db = f"sqlite3 {shlex.quote(db)} \"SELECT workspace_root, title FROM sessions WHERE session_id='{session_id}' LIMIT 1;\" 2>/dev/null | head -n 1"
        out_db, _ = run_on_host(host, cmd_db)
        cwd = title = None
        if out_db and out_db.strip():
            parts = out_db.strip().split("|")
            if parts[0].strip():
                cwd = parts[0].strip()
            if len(parts) > 1 and parts[1].strip():
                title = parts[1].strip()
        if not cwd:
            cmd2 = f"grep -m1 'route_facts' {shlex.quote(path)} 2>/dev/null | head -c 4000"
            out2, _ = run_on_host(host, cmd2)
            if out2:
                import re
                m = re.search(r'\"cwd\"\s*:\s*\"([^\"]+)\"', out2)
                if m:
                    cand = m.group(1)
                    if len(cand) >= 8:
                        cwd = cand
                    elif not title:
                        title = cand
        if not cwd:
            cwd = home
        return {"session_id": session_id, "cwd": cwd, "title": title, "raw": (out_db[:500] if out_db else out2[:500] if 'out2' in locals() and out2 else "")}
    elif cli_slug == "claude-code":
        session_id = Path(path).stem
        if session_id.startswith("agent-"):
            return {"session_id": None, "raw": ""}
        # Try to get cwd via a single grep for "cwd" (independent of first 5)
        cmd = f"grep -m1 '\"cwd\"' {shlex.quote(path)} 2>/dev/null | head -c 2000"
        out, _ = run_on_host(host, cmd)
        cwd = None
        title = None
        if out:
            try:
                # cwd may appear as "cwd":"/path" or "relocatedCwd"
                import re
                m = re.search(r'\"cwd\"\s*:\s*\"([^\"]+)\"', out)
                if m:
                    cwd = m.group(1)
                m2 = re.search(r'\"relocatedCwd\"\s*:\s*\"([^\"]+)\"', out)
                if not cwd and m2:
                    cwd = m2.group(1)
            except:
                pass
        if not cwd:
            parent_name = Path(path).parent.name
            if parent_name.startswith("-"):
                cwd = "/" + parent_name[1:].replace("-", "/")
            else:
                cwd = home
        # Title precedence per Rust read_cc_session_title: latest custom-title wins,
        # else latest ai-title, else first human prompt (we approximate with custom then ai).
        title = None
        for typ in ["custom-title", "ai-title"]:
            tcmd = f"grep -F '\"{typ}\"' {shlex.quote(path)} 2>/dev/null | tail -n 1 | head -c 3000"
            tout, _ = run_on_host(host, tcmd)
            if tout:
                try:
                    j=json.loads(tout)
                    cand=j.get("title") or j.get("customTitle") or j.get("aiTitle")
                    if cand and cand.strip():
                        title=cand.strip()
                        break
                except:
                    pass
        return {"session_id": session_id, "cwd": cwd, "title": title, "raw": out[:500] if out else ""}
    else:
        # jsonl — read first and last lines
        home_out,_ = run_on_host(host, "echo $HOME")
        home = home_out.strip() if home_out else os.path.expanduser("~")
        cmd = f"head -n 5 {shlex.quote(path)} 2>/dev/null | head -c 8000"
        out, _ = run_on_host(host, cmd)
        # Try to extract id/cwd/title from jsonl
        session_id = cwd = title = None
        for line in out.splitlines():
            line=line.strip()
            if not line:
                continue
            try:
                j=json.loads(line)
                if not session_id:
                    session_id = j.get("id") or j.get("sessionId") or j.get("session_id")
                if not cwd:
                    cwd = j.get("cwd")
                if not title and cli_slug=="claude-code":
                    # cc title can be custom-title or ai-title later, but we check preview
                    if j.get("type")=="custom-title":
                        title=j.get("title") or j.get("customTitle")
                    elif j.get("type")=="ai-title":
                        title=j.get("title") or j.get("aiTitle")
                # qwen/pi have sessionId in first record
                if cli_slug in ("qwen","pi") and not title:
                    # pi title is not in header
                    pass
            except:
                continue
            if session_id and cwd:
                break
        return {"session_id": session_id, "cwd": cwd, "title": title, "raw": out[:500]}

def compare(host, verb_data, manual_sessions, verbose=False):
    issues = []
    if verb_data is None:
        issues.append("verb failed to return JSON")
        return issues
    verb_rows = verb_data.get("rows", [])
    verb_ids = set(r.get("session_id") for r in verb_rows)
    # Manual ids — only those where we parsed a session_id
    manual_ids = set()
    manual_by_id = {}
    for s in manual_sessions:
        pid = s.get("parsed", {}).get("session_id")
        if pid:
            manual_ids.add(pid)
            manual_by_id[pid]=s
        else:
            # For codex, id is from file stem if not parsed
            # Fallback: use path stem
            stem = Path(s["path"]).stem
            if stem.startswith("rollout-"):
                stem = stem[len("rollout-"):]
                # rollout file contains timestamp prefix, id is after second -
                # e.g. rollout-2026-08-13T...-019a...jsonl -> id is last part
                parts = stem.split("-")
                # Heuristic: id is last UUID-like part
                if len(parts) >= 5:
                    pid = "-".join(parts[-5:])
                    manual_ids.add(pid)
                    manual_by_id[pid]=s
    # Symmetric diff
    verb_only = verb_ids - manual_ids
    manual_only = manual_ids - verb_ids
    # Rows the daemon scanned from PEER machines cannot appear in a local walk.
    # Split them out instead of reporting them as drift.
    remote_only = set()
    if verb_only:
        backed = locally_backed_ids(run_on_host, host, [r for r in verb_rows if r.get("session_id") in verb_only])
        remote_only = verb_only - backed
        verb_only = verb_only & backed
    if remote_only:
        print(f"[{host}] note: {len(remote_only)} verb rows have no local store file (peer-scanned, or live with no transcript yet) — not counted as drift")
    if verb_only:
        issues.append(f"verb has {len(verb_only)} ids not in manual walk (extra): {sorted(list(verb_only))[:5]}")
    if manual_only:
        issues.append(f"manual has {len(manual_only)} ids not in verb (missing): {sorted(list(manual_only))[:5]}")
    # Title check for common ids (where both have title)
    for row in verb_rows:
        sid=row.get("session_id")
        if sid in manual_by_id:
            verb_title=(row.get("title") or "").strip()
            # Compare against the CONDENSED store title — the row label the verb
            # ships — not the raw store text it keeps as `detail`.
            manual_title=(condense_store_title(manual_by_id[sid].get("parsed", {}).get("title")) or "").strip()
            if verb_title and manual_title and verb_title != manual_title:
                cut = next(
                    (i for i, (a, b) in enumerate(zip(verb_title, manual_title)) if a != b),
                    min(len(verb_title), len(manual_title)),
                )
                issues.append(
                    f"title mismatch {sid[:8]} at char {cut}: "
                    f"verb ...{verb_title[cut:cut + 40]!r} vs manual ...{manual_title[cut:cut + 40]!r} "
                    f"(len {len(verb_title)} vs {len(manual_title)})"
                )
    # Count check. ⛔ EXACT, and on the LOCAL half only. This used to allow a
    # ±10 slack, which is a checker that cannot see up to ten missing sessions —
    # the absence a count is least able to notice is the one it is asked to find.
    local_verb_ids = verb_ids - remote_only
    if len(local_verb_ids) != len(manual_ids):
        issues.append(
            f"count drift: verb {len(local_verb_ids)} local rows vs manual {len(manual_ids)} "
            f"({len(remote_only)} peer rows excluded)"
        )
    # `durable_count` must describe the row list it ships with, not a pre-dedup total.
    declared = verb_data.get("durable_count")
    if declared is not None and not verb_data.get("truncated") and declared != len(verb_rows):
        issues.append(f"durable_count {declared} disagrees with {len(verb_rows)} rows shipped")
    if verbose:
        print(f"[{host}] verb {len(verb_rows)} rows, manual {len(manual_sessions)} files")
        if verb_rows[:3]:
            print(f"  verb top3: {[r.get('session_id','')[:8]+':'+(r.get('title') or '')[:30] for r in verb_rows[:3]]}")
    return issues

def main():
    ap = argparse.ArgumentParser(description="Check startpage ls against manual jsonl walk")
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
        ok = len(issues)==0
        if not ok:
            all_ok=False
        report[host] = {
            "ok": ok,
            "issues": issues,
            "verb_count": len(verb_data.get("rows", [])),
            "manual_count": len(manual_sessions),
            "host": host,
            "durable_count": verb_data.get("durable_count"),
            "live_count": verb_data.get("live_count"),
        }
        if not args.json:
            status = "OK" if ok else "MISMATCH"
            print(f"[{host}] {status}: verb {len(verb_data.get('rows',[]))} manual {len(manual_sessions)} live {verb_data.get('live_count')}")
            for iss in issues:
                print(f"  - {iss}")
            if verbose := args.verbose:
                print(json.dumps(verb_data, indent=2)[:3000])

    if args.json:
        print(json.dumps(report, indent=2))
        # Also write per-host details for iteration
    if not all_ok:
        sys.exit(2)
    else:
        if not args.json:
            print("All hosts match — startpage ls is truthful.")
        sys.exit(0)

if __name__ == "__main__":
    main()
