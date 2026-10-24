#!/usr/bin/env python3
"""ygg-fleet — the fleet recon collapse (owner 2026-10-24: "all those recons
and checks throughout the fleet can be simplified to handful of tool calls").

  ygg agents ps [--json]        live agent sessions: row registry + /proc liveness
  ygg fleet pressure [--json]   one view: host load, zai plan quotas, queue depth, dream state
  ygg board scan [--json]       unrealized dreams on the msgGraph boards

Every subcommand degrades honestly per host: a dead peer is a line in the
output, never a crash — supervision must work from a partially alive fleet.
"""
import json, os, subprocess, sys
from datetime import datetime, timezone
from pathlib import Path

YGGBIN = Path.home() / ".yggterm/bin/ygg"
FLEET_HOSTS_FILE = Path.home() / ".yggterm/auth/.fleet-hosts"


def run(cmd, timeout=30):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", f"timeout after {timeout}s"
    except FileNotFoundError as e:
        return 127, "", str(e)


def fleet_hosts():
    try:
        raw = FLEET_HOSTS_FILE.read_text().split()
        hosts = [x for h in raw if not h.startswith("#") for x in h.split(",") if x.strip()]
        return hosts or []
    except Exception:
        return []


def ssh(host, cmd, timeout=30):
    return run(["ssh", "-o", "ConnectTimeout=10", host, cmd], timeout)


def cmd_agents(as_json):
    """Row registry first (daemon hosts), /proc liveness second (everything)."""
    rows, errors = [], []
    rc, out, err = run([str(YGGBIN / "ygg_row.py"), "list", "--all-hosts", "--json"], 30)
    if rc == 0:
        try:
            rows = json.loads(out)
        except Exception:
            errors.append("row registry returned non-json")
    else:
        errors.append(f"row registry unavailable ({(err or out).strip()[:80] or rc})")
    # prune dead pids
    live = []
    for r in rows if isinstance(rows, list) else []:
        pid = r.get("pid")
        if pid and not Path(f"/proc/{pid}").exists():
            r["dead"] = True
        live.append(r)
    if as_json:
        print(json.dumps({"sessions": live, "errors": errors}, indent=1))
        return
    print(f"agents ps — {len(live)} registered rows" + (f" ({len(errors)} registry errors)" if errors else ""))
    for r in live:
        flag = " DEAD" if r.get("dead") else ""
        print(f"  {str(r.get('row') or r.get('id') or '?'):8} {str(r.get('host') or '?'):6} pid={r.get('pid')} {str(r.get('claim') or r.get('title') or '')[:60]}{flag}")
    for e in errors:
        print(f"  ! {e}")


def zai_plans_local():
    """zcode-tui plans --json from the local install (start plan + grants)."""
    rc, out, err = run(["zcode-tui", "plans", "--json"], 45)
    if rc != 0:
        # non-interactive PATHs often lack the ynpm bin dir — try the canonical paths
        for cand in (Path.home() / ".yggterm/ynpm/bin/zcode-tui",):
            if cand.exists():
                rc, out, err = run([str(cand), "plans", "--json"], 45)
                break
    if rc != 0:
        return None
    try:
        d = json.loads(out)
        return {"glm53_full_allowed": d.get("glm53_full_allowed"),
                "offpeak": d.get("offpeak"),
                "plans": [{"name": p.get("name"), "status": p.get("status"), "ends_at": p.get("ends_at")} for p in d.get("plans", [])]}
    except Exception:
        return None


def cmd_pressure(as_json):
    out = {"generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"), "hosts": {}, "zai_plans": None, "queue": None, "dream": None}
    # local host
    rc, up, _ = run(["uptime"], 10)
    out["hosts"][os.uname().nodename] = {"uptime": up.strip() or "n/a"}
    # peers
    for h in fleet_hosts():
        rc, up, err = ssh(h, "uptime", 20)
        out["hosts"][h] = {"uptime": up.strip() if rc == 0 else f"UNREACHABLE: {(err or up).strip()[:60]}"}
    # zai plan quotas (local credentials)
    out["zai_plans"] = zai_plans_local()
    # executor queue depth (dev by convention — honest line if unreachable)
    rc, q, err = ssh("dev", "cd ~/gh/zcode-tui && ~/.bun/bin/bun run src/bin/queue.ts status 2>/dev/null | head -10", 40)
    out["queue"] = q.strip() if rc == 0 else f"unreachable: {(err or q).strip()[:80]}"
    # dream orchestrator state (local)
    rc, d, _ = run(["python3", str(YGGBIN / "ygg-dream-orchestrator.py"), "status", "--json"], 60)
    if rc == 0:
        try:
            out["dream"] = json.loads(d)
        except Exception:
            out["dream"] = None
    if as_json:
        print(json.dumps(out, indent=1))
        return
    print("fleet pressure @", out["generated_at"])
    for h, v in out["hosts"].items():
        print(f"  {h:8} {v['uptime'][:90]}")
    if out["zai_plans"]:
        p = out["zai_plans"]
        print(f"  zai plans: glm53_full={'ALLOWED' if p.get('glm53_full_allowed') else 'blocked'} offpeak={'ok' if (p.get('offpeak') or {}).get('can_take') else 'gated'}")
    if out["queue"]:
        print("  queue:")
        for line in str(out["queue"]).splitlines():
            print("   ", line)
    if out["dream"]:
        d = out["dream"]
        print(f"  dream: staged={d['today']['staged']} backlog={d['queue']['backlog']} running={d['queue']['running']} armed={d['armed']}")


def cmd_board_scan(as_json):
    rc, out, err = run([str(YGGBIN / "ygg-board.py"), "read", "dreams/features", "--json"], 30)
    if rc != 0:
        msg = f"board read failed: {(err or out).strip()[:100]}"
        print(json.dumps({"error": msg}) if as_json else msg)
        return
    try:
        posts = json.loads(out)
    except Exception:
        posts = []
    open_dreams = [p for p in (posts if isinstance(posts, list) else [])
                   if not any((a.get("kind") == "answer") for a in (p.get("answers") or []))] if isinstance(posts, list) else []
    if as_json:
        print(json.dumps({"open_dreams": open_dreams, "total": len(posts) if isinstance(posts, list) else 0}, indent=1))
        return
    print(f"board scan dreams/features — {len(open_dreams)} open of {len(posts) if isinstance(posts, list) else 0}")
    for p in open_dreams[:15]:
        print(f"  {str(p.get('ack') or p.get('id') or '?')[:16]} {str(p.get('title') or p.get('text') or '')[:90]}")


def main():
    args = [a for a in sys.argv[1:] if a != "--json"]
    as_json = "--json" in sys.argv
    verb = args[0] if args else ""
    if verb == "agents":
        cmd_agents(as_json)
    elif verb in ("fleet", "pressure"):
        cmd_pressure(as_json)
    elif verb == "board":
        cmd_board_scan(as_json)
    else:
        print("usage: ygg-fleet.py <agents|fleet|board> [--json]")
        sys.exit(2)


if __name__ == "__main__":
    main()
