#!/usr/bin/env python3
"""ygg-bridge — the BROKER for external GUI harnesses (zcode et al): row control
and fleet messaging WITHOUT the row primitive.

⚖ WHY THIS EXISTS. External GUI harnesses hold no yggterm row: no PTY, no seat,
no OSC surface — the daemon does not own their process. Bridge is the ONE
sanctioned, audited, revocable path around that line (owner, 2026-09-07):

    · an external harness SUBSCRIBES: ~/.yggterm/bridge/<sub>/ with meta.json,
      inbox.jsonl (append-only, msgGraph post shape), heartbeat, cursor.
    · ROWS, monitor, booter and other bridges POST to it; it reads with a
      cursor (incremental — never re-read the whole jsonl into context) and
      `wait`s on a thread id (a GUI harness has no between-turns timer).
    · PRIVILEGED VERBS (spawn/submit/despawn a row) are INTENTS: the bridge
      CLI executes them through ygg_appctl — the ONE owner of row-plane verbs —
      and posts intent+outcome pairs back. The audit trail IS the inbox.
    · subscribe also REGISTERS the harness in the ygg-row registry (unclaimed)
      and spawns a DETACHED TOUCHER that keeps the heartbeat alive while the
      session PROCESS is /proc-alive — liveness is process, not conversation:
      an idle GUI harness must not die of quiet.
    · DEATH: heartbeat stale > ttl (default 24h) → GC reaps the whole
      subscription thread. Fresh = heartbeat < 10 min.

Per-host by design: you control the rows of the host whose bridge you talk to.
Sub names default to <harness>-<session-short> — never share one.

Usage:
    ygg_bridge.py subscribe [--name s] [--capabilities c1,c2] [--ttl-hours 24]
                            [--no-toucher]
    ygg_bridge.py post --to <sub> | --broadcast monitor|booter
                       [--kind note] [--thread id] (--body t | --body-file f)
    ygg_bridge.py read [<sub>] [--json]
    ygg_bridge.py wait --thread <id> [--timeout 300]
    ygg_bridge.py intent spawn-row --cli <kind> --cwd <dir> [--title t]
                                   [--purpose p] [--machine-key k]
                                   [--brief-file f] [--brief-text t]
                                   [--row-campaign N] [--row-role worker]
    ygg_bridge.py intent submit --row <path> (--text t | --stdin)
    ygg_bridge.py intent despawn --row <path>
    ygg_bridge.py touch --sub <s> --pid <p>        # the toucher's loop
    ygg_bridge.py list | status | unsubscribe [<sub>] | gc [--dry-run]
"""

import argparse, json, os, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
FRESH_SECS = 600
DEFAULT_TTL_HOURS = 24


def bridge_dir():
    d = Path(os.environ.get("YGGTERM_HOME", str(Path.home() / ".yggterm"))) / "bridge"
    (d / "_broadcast").mkdir(parents=True, exist_ok=True)
    return d


def this_session():
    return os.environ.get("YGGTERM_SESSION_ID", "").replace("cc-runtime://", "") or f"anon-{os.getpid()}"


def harness_name():
    return os.environ.get("YGGTERM_HARNESS", "zcode")


def default_sub():
    return f"{harness_name()}-{this_session()[:8]}"


def sub_dir(name, create=False):
    d = bridge_dir() / name
    if create:
        d.mkdir(parents=True, exist_ok=True)
    return d


def load(path, default=None):
    try:
        return json.loads(path.read_text())
    except Exception:
        return default


def write_json(path, obj):
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(obj, indent=1, default=str))
    tmp.rename(path)


def append_post(inbox, post):
    with open(inbox, "a") as f:
        f.write(json.dumps(post, default=str) + "\n")


def make_post(kind, thread, body, ttl_days=7):
    return {"ts": time.time(), "from": this_session(), "from_harness": harness_name(),
            "kind": kind, "thread": thread, "ttl_days": ttl_days, "body": body}


def check_pid(pid):
    try:
        return bool(Path(f"/proc/{pid}/cmdline").exists())
    except Exception:
        return False


# ── verbs ───────────────────────────────────────────────────────────────────
def spawn_toucher(name, pid):
    subprocess.Popen(
        ["python3", str(Path(__file__).resolve()), "touch", "--sub", name, "--pid", str(pid)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)


def cmd_subscribe(a):
    name = a.name or default_sub()
    d = sub_dir(name, create=True)
    meta = {"name": name, "harness": harness_name(), "session": this_session(),
            "host": os.uname().nodename, "pid": os.getpid(), "born": time.time(),
            "capabilities": (a.capabilities or "rows,monitor,booter").split(","),
            "ttl_hours": a.ttl_hours}
    write_json(d / "meta.json", meta)
    (d / "heartbeat").touch()
    if not (d / "inbox.jsonl").exists():
        (d / "inbox.jsonl").touch()
    try:
        import ygg_row
        ygg_row.cmd_register_bridge(type("A", (), {"name": name, "harness": meta["harness"],
                                                  "session": this_session(), "model": None,
                                                  "pid": meta["pid"], "cwd": os.getcwd(),
                                                  "ttl_hours": a.ttl_hours})())
    except SystemExit:
        pass
    except Exception as ex:
        print(f"⚠ registry registration skipped: {ex}", file=sys.stderr)
    toucher = ""
    if not a.no_toucher and check_pid(meta["pid"]):
        spawn_toucher(name, meta["pid"])
        toucher = " + detached toucher"
    print(f"✓ subscribed as `{name}` at {d}{toucher}")


def cmd_touch(a):
    """Detached toucher: keep heartbeat while the session process lives. Self-exits."""
    d = sub_dir(a.sub)
    while check_pid(a.pid):
        (d / "heartbeat").touch()
        time.sleep(30)
    # one last touch so peers see the exact moment of death, then let GC reap
    (d / "heartbeat").touch()
    append_post(bridge_dir() / "_broadcast" / "monitor.jsonl",
                make_post("warning", f"bridge-death:{a.sub}", f"bridge `{a.sub}` process (pid {a.pid}) is gone"))


def cmd_post(a):
    body = a.body or (Path(a.body_file).read_text() if a.body_file else None)
    if body is None:
        sys.exit("⛔ --body or --body-file required")
    if a.broadcast:
        inbox = bridge_dir() / "_broadcast" / f"{a.broadcast}.jsonl"
        inbox.parent.mkdir(parents=True, exist_ok=True)
    else:
        inbox = sub_dir(a.to, create=True) / "inbox.jsonl"
    append_post(inbox, make_post(a.kind, a.thread, body))
    print(f"✓ posted ({a.kind}) → {'_broadcast/' + a.broadcast if a.broadcast else a.to}")


def read_new(d, as_json=False):
    inbox = d / "inbox.jsonl"
    cursor_p = d / "cursor"
    off = load(cursor_p, {}) or {}
    pos = off.get("offset", 0)
    out = []
    if inbox.exists():
        with inbox.open("rb") as f:
            f.seek(pos)
            for raw in f:
                line = raw.decode(errors="replace").strip()
                if not line:
                    continue
                try:
                    out.append(json.loads(line))
                except Exception:
                    out.append({"ts": None, "kind": "corrupt", "body": line})
            newpos = f.tell()
        off["offset"] = newpos
        write_json(cursor_p, off)
    return out


def cmd_read(a):
    d = sub_dir(a.sub or default_sub())
    posts = read_new(d)
    if a.json:
        print(json.dumps(posts, indent=1, default=str)); return
    if not posts:
        print("= no new posts"); return
    for p in posts:
        who = f"{p.get('from_harness', '?')}:{str(p.get('from', ''))[:8]}"
        th = f" [#{p.get('thread')}]" if p.get('thread') else ""
        print(f"[{time.strftime('%H:%M:%S', time.localtime(p['ts'])) if p.get('ts') else '?'}] "
              f"{p.get('kind', 'note'):>9} {who}{th}\n  {p.get('body', '')}")


def cmd_wait(a):
    d = sub_dir(default_sub())
    deadline = time.time() + a.timeout
    while time.time() < deadline:
        inbox = d / "inbox.jsonl"
        if inbox.exists():
            for raw in inbox.read_text().splitlines():
                try:
                    p = json.loads(raw)
                except Exception:
                    continue
                if p.get("thread") == a.thread and p.get("from") != this_session():
                    print(json.dumps(p, indent=1, default=str))
                    return
        time.sleep(a.interval)
    sys.exit(f"⏱ no reply on thread {a.thread} within {a.timeout}s")


def run_intent(kind, argstr, stdin_path=None):
    """Execute a privileged verb through ygg_appctl and journal intent+outcome."""
    import ygg_appctl
    plane = ygg_appctl.resolve(verbose=False)
    t0 = time.time()
    try:
        r = plane.app_json(argstr, stdin_path=stdin_path, timeout=180)
        ok = bool(r.get("data") or r.get("ok"))
        return {"kind": kind, "ok": ok, "reply": r, "secs": round(time.time() - t0, 1)}
    except Exception as ex:
        return {"kind": kind, "ok": False, "error": str(ex), "secs": round(time.time() - t0, 1)}


def journal_intent(outcome, thread):
    d = sub_dir(default_sub(), create=True)
    append_post(d / "inbox.jsonl", make_post("intent" if not outcome["ok"] else "outcome", thread,
                                             json.dumps(outcome, default=str)[:2000]))


def cmd_intent(a):
    thread = f"intent:{int(time.time())}"
    if a.which == "spawn-row":
        argstr = (f"terminal new --kind {a.cli} --cwd {a.cwd} --title {a.title!r} "
                  f"--purpose {a.purpose!r} --no-activate")
        if a.machine_key:
            argstr += f" --machine-key {a.machine_key}"
        outcome = run_intent("spawn-row", argstr)
        row = ((outcome.get("reply") or {}).get("data") or {}).get("session_path")
        outcome["row"] = row
        if row and a.brief_file:
            outcome["brief"] = run_intent("submit-brief", f"terminal submit {row}", stdin_path=a.brief_file)
        if row and a.brief_text:
            bf = Path("/tmp") / f"bridge-brief-{this_session()[:8]}-{int(time.time())}.md"
            bf.write_text(a.brief_text)
            outcome["brief"] = run_intent("submit-brief", f"terminal submit {row}", stdin_path=str(bf))
            bf.unlink(missing_ok=True)
        if row and a.row_campaign:
            try:
                import ygg_row
                ygg_row.cmd_claim(type("A", (), {"campaign": a.row_campaign, "role": a.row_role,
                                                 "parent": None, "id": None, "model": None,
                                                 "purpose": a.purpose, "bridge_sub": default_sub(),
                                                 "row_path": row, "harness": a.cli,
                                                 "ttl_hours": ygg_row.DEFAULT_TTL_HOURS})())
                outcome["row_claimed"] = a.row_campaign
            except SystemExit as ex:
                outcome["row_claim_error"] = str(ex)
        print(json.dumps({"row": row, **{k: v for k, v in outcome.items() if k != "reply"}}, indent=1, default=str))
    elif a.which == "submit":
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(a.text or sys.stdin.read())
            tmp = f.name
        outcome = run_intent("submit", f"terminal submit {a.row}", stdin_path=tmp)
        os.unlink(tmp)
    else:  # despawn
        outcome = run_intent("despawn", f"session remove {a.row}")
    journal_intent(outcome, thread)


def cmd_unsubscribe(a):
    name = a.sub or default_sub()
    d = sub_dir(name)
    meta = load(d / "meta.json", {}) or {}
    (d / "_DEAD").write_text(json.dumps({"died": time.time(), "by": this_session()}))
    try:
        import ygg_row
        store = ygg_row.rows_dir()
        bp = store / "bridges" / f"{name}.json"
        if bp.exists():
            bp.unlink()
    except Exception:
        pass
    print(f"✓ unsubscribed `{name}` — inbox kept for GC to reap (peers see _DEAD)")


def cmd_gc(a):
    import shutil
    now = time.time()
    root = bridge_dir()
    for d in sorted(p for p in root.iterdir() if p.is_dir() and p.name != "_broadcast"):
        meta = load(d / "meta.json", {}) or {}
        ttl = (meta.get("ttl_hours") or DEFAULT_TTL_HOURS) * 3600
        hb = (d / "heartbeat")
        stale_for = now - (hb.stat().st_mtime if hb.exists() else 0)
        dead = (d / "_DEAD").exists()
        if stale_for > ttl:
            if a.dry_run:
                print(f"would reap `{d.name}` (heartbeat stale {int(stale_for // 60)}m)")
            else:
                shutil.rmtree(d)
                print(f"♻ reaped `{d.name}`")
        elif dead:
            if a.dry_run:
                print(f"would reap `{d.name}` (unsubscribed)")
            else:
                shutil.rmtree(d)
                print(f"♻ reaped `{d.name}`")
    for f in (root / "_broadcast").glob("*.jsonl"):
        keep, cutoff = [], now - 7 * 86400
        if f.exists():
            for raw in f.read_text().splitlines():
                try:
                    if json.loads(raw).get("ts", 0) > cutoff:
                        keep.append(raw)
                except Exception:
                    continue
            if len(keep) < len(f.read_text().splitlines()):
                f.write_text("\n".join(keep) + ("\n" if keep else ""))


def cmd_list(a):
    root = bridge_dir()
    for d in sorted(p for p in root.iterdir() if p.is_dir() and p.name != "_broadcast"):
        meta = load(d / "meta.json", {}) or {}
        hb = d / "heartbeat"
        fresh = hb.exists() and (time.time() - hb.stat().st_mtime) < FRESH_SECS
        n = sum(1 for _ in (d / "inbox.jsonl").open()) if (d / "inbox.jsonl").exists() else 0
        unread = 0
        cur = (load(d / "cursor", {}) or {}).get("offset", 0)
        if (d / "inbox.jsonl").exists():
            unread = max(0, (d / "inbox.jsonl").stat().st_size - cur)
        print(f"{'●' if fresh else '○'} {d.name:<28} {meta.get('harness', '?'):<8} "
              f"posts:{n:<4} unread~{unread}B  ttl:{meta.get('ttl_hours', '?')}h"
              f"{'  [DEAD]' if (d / '_DEAD').exists() else ''}")
    for f in sorted((root / "_broadcast").glob("*.jsonl")):
        print(f"  _broadcast/{f.name} ({f.stat().st_size}B)")


def cmd_status(a):
    name = default_sub()
    d = sub_dir(name)
    ok = (d / "meta.json").exists()
    print(json.dumps({"sub": name, "subscribed": ok,
                      "fresh": ok and (time.time() - (d / "heartbeat").stat().st_mtime) < FRESH_SECS}))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="cmd", required=True)

    c = sp.add_parser("subscribe")
    c.add_argument("--name"); c.add_argument("--capabilities"); c.add_argument("--ttl-hours", type=int, default=DEFAULT_TTL_HOURS)
    c.add_argument("--no-toucher", action="store_true"); c.set_defaults(fn=cmd_subscribe)

    c = sp.add_parser("post")
    c.add_argument("--to"); c.add_argument("--broadcast", choices=["monitor", "booter"])
    c.add_argument("--kind", default="note"); c.add_argument("--thread")
    c.add_argument("--body"); c.add_argument("--body-file")
    c.set_defaults(fn=cmd_post)

    c = sp.add_parser("read"); c.add_argument("sub", nargs="?"); c.add_argument("--json", action="store_true"); c.set_defaults(fn=cmd_read)
    c = sp.add_parser("wait"); c.add_argument("--thread", required=True); c.add_argument("--timeout", type=int, default=300)
    c.add_argument("--interval", type=float, default=2.0); c.set_defaults(fn=cmd_wait)

    c = sp.add_parser("intent"); c.add_argument("which", choices=["spawn-row", "submit", "despawn"])
    c.add_argument("--cli"); c.add_argument("--cwd"); c.add_argument("--title", default="bridge row")
    c.add_argument("--purpose", default=""); c.add_argument("--machine-key")
    c.add_argument("--brief-file"); c.add_argument("--brief-text")
    c.add_argument("--row-campaign"); c.add_argument("--row-role", default="worker")
    c.add_argument("--row"); c.add_argument("--text"); c.set_defaults(fn=cmd_intent)

    c = sp.add_parser("touch"); c.add_argument("--sub", required=True); c.add_argument("--pid", type=int, required=True); c.set_defaults(fn=cmd_touch)
    c = sp.add_parser("unsubscribe"); c.add_argument("sub", nargs="?"); c.set_defaults(fn=cmd_unsubscribe)
    c = sp.add_parser("list"); c.set_defaults(fn=cmd_list)
    c = sp.add_parser("status"); c.set_defaults(fn=cmd_status)
    c = sp.add_parser("gc"); c.add_argument("--dry-run", action="store_true"); c.set_defaults(fn=cmd_gc)

    a = p.parse_args()
    if a.cmd == "intent":
        if a.which in ("submit", "despawn") and not a.row:
            sys.exit("⛔ --row required")
        if a.which == "spawn-row" and (not a.cli or not a.cwd):
            sys.exit("⛔ --cli and --cwd required")
    a.fn(a)


if __name__ == "__main__":
    main()
