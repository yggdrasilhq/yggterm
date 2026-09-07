#!/usr/bin/env python3
"""ygg-row — the fleet row REGISTRY: who is working, on what, as what, at what number.

⚖ WHY THIS EXISTS (owner, 2026-09-07 — replaces the "do not touch until told"
row blocker). The blocker prevented unsupervised spawn chaos by banning the
row primitive outright. Production removes the ban and replaces it with
ACCOUNTABILITY: every agent, row or external-GUI-harness bridge subscriber,
is visible in one registry with a claimed number, a role, a harness, a model,
a purpose and a freshness. The law is CONDITIONAL claiming:

    · SURE the work belongs to a campaign line? claim it (one verb, at spawn).
    · Vague one-off chat? continue UNCLAIMED — that is a first-class state.
    · The owner may attach/renumber a one-off to a campaign N at any time.

THE NUMBER GRAMMAR (owner's visual language — one number space per campaign,
shared with the campaign's memory/defect numbering like 11.X = cli-integration):

    N      a campaign (campaigns.json maps N -> name; 11 = cli-integration)
    N.X    a worker / subsession row (X never reused after release)
    N.0    the campaign's MASTER orchestrator (exists only when one exists)
    N.X.0  a sub-orchestrator under N.X
    N.XR   a relay

`list` is the one-stop view: id, role, MODEL, harness, session, host, cwd,
purpose, freshness — claims first, then bridge subscriber registrations.
Cross-host: `--all-hosts` ssh-fan-outs to the fleet hosts and merges.

STATE (per host, never msggraph — yggterm is public, msggraph is private):
    ~/.yggterm/rows/campaigns.json     N -> {name, claimed_by, claimed_at}
    ~/.yggterm/rows/claims/<id>.json   one file per claim (delete = release)
    ~/.yggterm/rows/bridges/<sub>.json bridge subscriber registrations
    ~/.yggterm/rows/journal.jsonl      append-only claim/release/attach/renumber

Usage:
    ygg_row.py campaign 11 --name cli-integration            # register N
    ygg_row.py claim --campaign 11 [--role worker|orchestrator|relay]
                     [--parent N.X] [--id N.X] [--model glm-5.1]
                     [--purpose "..."] [--bridge-sub name] [--row-path p]
                     [--ttl-hours 24]        # idempotent per session
    ygg_row.py release [<id>]          # default: this session's claim
    ygg_row.py renumber <old> --to <new>
    ygg_row.py heartbeat [<id>] [--model m]
    ygg_row.py list [--all-hosts] [--campaign N] [--json]
    ygg_row.py show <id> | gc [--dry-run] | register-bridge --name s --harness h
                                         [--session u] [--model m] [--pid p]
"""

import argparse, json, os, re, socket, subprocess, sys, time
from pathlib import Path

ID_RE = re.compile(r"^\d+(\.\d+)+(R|\.0)*$")
ROLES = ("worker", "orchestrator", "relay")
REMOTE_PATH = "~/gh/yggterm/.agents/skills/yggterm-agent-fleet/ygg_row.py"
FRESH_SECS = 600          # heartbeat newer than this = FRESH (owner: ~10 min)
DEFAULT_TTL_HOURS = 24    # reap a claim whose heartbeat is older than this


def rows_dir():
    d = Path(os.environ.get("YGGTERM_HOME", str(Path.home() / ".yggterm"))) / "rows"
    (d / "claims").mkdir(parents=True, exist_ok=True)
    (d / "bridges").mkdir(parents=True, exist_ok=True)
    return d


def this_session():
    sid = os.environ.get("YGGTERM_SESSION_ID", "")
    return sid.replace("cc-runtime://", "") or f"anon-{os.getpid()}"


def journal(store, ev, **kw):
    with (store / "journal.jsonl").open("a") as f:
        f.write(json.dumps({"ts": time.time(), "ev": ev, "host": socket.gethostname(),
                            "session": this_session(), **kw}, default=str) + "\n")


def write_json(path, obj):
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(obj, indent=1, default=str))
    tmp.rename(path)


def load_json(path, default=None):
    try:
        return json.loads(path.read_text())
    except Exception:
        return default


def fresh(claim):
    hb = claim.get("heartbeat", 0)
    return (time.time() - hb) < FRESH_SECS


def age_str(ts):
    s = int(time.time() - ts)
    for div, unit in ((86400, "d"), (3600, "h"), (60, "m")):
        if s >= div:
            return f"{s // div}{unit}"
    return f"{s}s"


# ── allocation ──────────────────────────────────────────────────────────────
def allocate(store, n, role, explicit_id):
    """Allocate the next free X under campaign N (or honour an explicit id)."""
    if explicit_id:
        if not ID_RE.match(explicit_id):
            sys.exit(f"⛔ id `{explicit_id}` does not match the grammar N.X[.0|R]")
        p = store / "claims" / f"{explicit_id}.json"
        if p.exists():
            sys.exit(f"⛔ {explicit_id} is already claimed — see `ygg_row.py show {explicit_id}`")
        return explicit_id
    lock = store / "claims" / f".alloc-{n}.lock"
    deadline = time.time() + 10
    fd = None
    while time.time() < deadline:
        try:
            fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            break
        except FileExistsError:
            if time.time() - lock.stat().st_mtime > 30:
                lock.unlink(missing_ok=True)   # stale lock from a dead holder
            time.sleep(0.05)
    if fd is None:
        sys.exit("⛔ could not allocate: allocation lock busy")
    try:
        used = {c.get("x") for c in map(lambda p: load_json(p, {}) or {}, (store / "claims").glob("*.json"))
                if c.get("n") == n and c.get("x") is not None}
        x = 1
        while x in used:
            x += 1
        if role == "orchestrator":
            return f"{n}.0"
        suffix = "R" if role == "relay" else ""
        return f"{n}.{x}{suffix}"
    finally:
        os.close(fd)
        lock.unlink(missing_ok=True)


def resolve_campaign(store, spec):
    camps = load_json(store / "campaigns.json", {}) or {}
    if spec in camps:
        return str(spec), camps[spec]["name"]
    for n, c in camps.items():
        if c.get("name") == spec:
            return n, c["name"]
    sys.exit(f"⛔ campaign `{spec}` is not registered — `ygg_row.py campaign <N> --name <name>` first")


# ── verbs ───────────────────────────────────────────────────────────────────
def cmd_campaign(a):
    store = rows_dir()
    camps = load_json(store / "campaigns.json", {}) or {}
    n = str(a.n)
    if n in camps and not a.name:
        print(json.dumps({"n": n, **camps[n]}))
        return
    if n in camps and camps[n]["name"] != a.name:
        sys.exit(f"⛔ {n} is already `{camps[n]['name']}` — renumbering a campaign is the owner's call")
    camps[n] = {"name": a.name, "claimed_by": this_session(), "claimed_at": time.time()}
    write_json(store / "campaigns.json", camps)
    journal(store, "campaign", n=n, name=a.name)
    print(f"✓ campaign {n} = {a.name}")


def cmd_claim(a):
    store = rows_dir()
    n, name = resolve_campaign(store, a.campaign)
    prior = find_own(store, n)
    if prior and not a.id:
        print(f"= this session already holds {prior['id']} (campaign {n}) — idempotent")
        return
    rid = allocate(store, n, a.role, a.id)
    if a.role == "orchestrator" and not rid.endswith(".0"):
        sys.exit("⛔ --role orchestrator must end in .0 (master N.0 or sub N.X.0)")
    if a.role == "relay" and not rid.endswith("R"):
        sys.exit("⛔ --role relay must end in R (N.XR)")
    claim = {"id": rid, "n": n, "x": rid.split(".")[1].rstrip("R"), "campaign": name,
             "role": a.role, "harness": a.harness or os.environ.get("YGGTERM_HARNESS", "unknown"),
             "session": this_session(), "host": socket.gethostname(), "pid": os.getpid(),
             "model": a.model, "cwd": os.getcwd(), "purpose": a.purpose,
             "parent": a.parent, "bridge_sub": a.bridge_sub, "row_path": a.row_path,
             "ttl_hours": a.ttl_hours, "claimed_at": time.time(), "heartbeat": time.time()}
    write_json(store / "claims" / f"{rid}.json", claim)
    journal(store, "claim", id=rid, n=n, role=a.role, purpose=a.purpose)
    print(f"✓ claimed {rid} ({name}, {a.role})" + (f" — {a.purpose}" if a.purpose else ""))


def find_own(store, n=None):
    for p in sorted((store / "claims").glob("*.json")):
        c = load_json(p, {}) or {}
        if c.get("session") == this_session() and (n is None or c.get("n") == n):
            return c
    return None


def find_claim(store, ident):
    p = store / "claims" / f"{ident}.json"
    if p.exists():
        return p, load_json(p, {}) or {}
    for pp in (store / "claims").glob("*.json"):     # by session uuid
        c = load_json(pp, {}) or {}
        if c.get("session", "").startswith(ident) or c.get("bridge_sub") == ident:
            return pp, c
    sys.exit(f"⛔ no claim matches `{ident}`")


def cmd_release(a):
    store = rows_dir()
    if a.id:
        p, c = find_claim(store, a.id)
    else:
        c = find_own(store)
        if not c:
            print("= this session holds no claim"); return
        p = store / "claims" / f"{c['id']}.json"
    p.unlink()
    journal(store, "release", id=c["id"])
    print(f"✓ released {c['id']}")


def cmd_renumber(a):
    store = rows_dir()
    p, c = find_claim(store, a.old)
    newp = store / "claims" / f"{a.to}.json"
    if newp.exists():
        sys.exit(f"⛔ {a.to} is already claimed")
    c["id"], c["renumbered_from"] = a.to, a.old
    c["heartbeat"] = time.time()
    p.unlink(); write_json(newp, c)
    journal(store, "renumber", old=a.old, new=a.to)
    print(f"✓ {a.old} → {a.to}")


def cmd_heartbeat(a):
    store = rows_dir()
    if a.id:
        p, c = find_claim(store, a.id)
    else:
        c = find_own(store)
        if not c:
            print("= no claim to heartbeat"); return
        p = store / "claims" / f"{c['id']}.json"
    c["heartbeat"] = time.time()
    if a.model: c["model"] = a.model
    c["pid"] = c.get("pid") or os.getpid()
    write_json(p, c)
    print(f"♥ {c['id']}")


def cmd_register_bridge(a):
    """A bridge subscriber registers itself (UNCLAIMED — registration ≠ claim)."""
    store = rows_dir()
    reg = {"sub": a.name, "harness": a.harness, "session": a.session or this_session(),
           "host": socket.gethostname(), "pid": a.pid or os.getpid(), "model": a.model,
           "cwd": a.cwd or os.getcwd(), "registered_at": time.time(),
           "heartbeat": time.time(), "ttl_hours": a.ttl_hours}
    write_json(store / "bridges" / f"{a.name}.json", reg)
    journal(store, "bridge-register", sub=a.name, harness=a.harness)
    print(f"✓ bridge `{a.name}` registered in the row registry (unclaimed)")


def known_hosts(store):
    """Peer hosts are OBSERVED IN THE REGISTRY — no hardcoded fleet names in a
    public repo. --all-hosts fans out to every host that has ever claimed here,
    plus --hosts overrides."""
    hosts = set()
    for sub in ("claims", "bridges"):
        for p in (store / sub).glob("*.json"):
            h = (load_json(p, {}) or {}).get("host")
            if h:
                hosts.add(h)
    return sorted(hosts - {socket.gethostname()})


def collect(store):
    out = []
    for p in sorted((store / "claims").glob("*.json")):
        c = load_json(p, {}) or {}
        c["_kind"] = "claim"
        out.append(c)
    for p in sorted((store / "bridges").glob("*.json")):
        b = load_json(p, {}) or {}
        b["_kind"] = "bridge"
        out.append(b)
    return out


def render(entries, ttl_default=DEFAULT_TTL_HOURS):
    hdr = f"{'ID':<10} {'ROLE':<12} {'MODEL':<16} {'HARNESS':<10} {'FRESH':<6} {'HOST':<7} PURPOSE"
    lines = [hdr, "-" * len(hdr)]
    for c in entries:
        rid = c.get("id") or f"({c.get('sub', '?')})"
        role = c.get("role") or ("bridge-sub" if c.get("_kind") == "bridge" else "?")
        hb = c.get("heartbeat", 0)
        ttl = (c.get("ttl_hours") or ttl_default) * 3600
        state = "FRESH" if time.time() - hb < FRESH_SECS else ("stale" if time.time() - hb < ttl else "REAPED")
        purpose = (c.get("purpose") or c.get("cwd") or "")[:48]
        lines.append(f"{rid:<10} {role:<12} {c.get('model') or '-':<16} "
                     f"{c.get('harness') or '-':<10} {state:<6} {c.get('host') or '-':<7} {purpose}")
    return "\n".join(lines)


def cmd_list(a):
    store = rows_dir()
    entries = collect(store)
    if a.campaign:
        n, _ = resolve_campaign(store, a.campaign)
        entries = [e for e in entries if e.get("n") == n]
    if a.all_hosts:
        for h in (a.hosts.split(",") if a.hosts else known_hosts(store)):
            try:
                r = subprocess.run(["ssh", h, "python3", REMOTE_PATH, "list", "--json"],
                                   capture_output=True, text=True, timeout=25)
                for e in json.loads(r.stdout or "[]"):
                    e.setdefault("host", h)
                    entries.append(e)
            except Exception as ex:
                print(f"⚠ {h}: {ex}", file=sys.stderr)
    entries.sort(key=lambda c: (str(c.get("id") or c.get("sub"))))
    if a.json:
        print(json.dumps(entries, indent=1, default=str)); return
    if not entries:
        print("= nothing registered here"); return
    print(render(entries))


def cmd_gc(a):
    store = rows_dir()
    reaped = []
    for p in sorted((store / "claims").glob("*.json")):
        c = load_json(p, {}) or {}
        ttl = (c.get("ttl_hours") or DEFAULT_TTL_HOURS) * 3600
        if time.time() - c.get("heartbeat", 0) > ttl:
            reaped.append((p, c))
    for p in sorted((store / "bridges").glob("*.json")):
        b = load_json(p, {}) or {}
        ttl = (b.get("ttl_hours") or DEFAULT_TTL_HOURS) * 3600
        if time.time() - b.get("heartbeat", 0) > ttl:
            reaped.append((p, b))
    for p, c in reaped:
        if a.dry_run:
            print(f"would reap {c.get('id') or c.get('sub')} (heartbeat {age_str(c.get('heartbeat', 0))} old)")
        else:
            p.unlink()
            journal(store, "gc-reap", id=c.get("id") or c.get("sub"))
            print(f"♻ reaped {c.get('id') or c.get('sub')}")
    if not reaped:
        print("= nothing to reap")


def cmd_show(a):
    store = rows_dir()
    _, c = find_claim(store, a.id)
    print(json.dumps(c, indent=1, default=str))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="cmd", required=True)

    c = sp.add_parser("campaign", help="register a campaign number N")
    c.add_argument("n"); c.add_argument("--name", required=True); c.set_defaults(fn=cmd_campaign)

    c = sp.add_parser("claim", help="claim a row id (CONDITIONAL: only when sure it belongs to a line of work)")
    c.add_argument("--campaign", required=True); c.add_argument("--role", choices=ROLES, default="worker")
    c.add_argument("--parent"); c.add_argument("--id"); c.add_argument("--model")
    c.add_argument("--purpose"); c.add_argument("--bridge-sub"); c.add_argument("--row-path")
    c.add_argument("--harness"); c.add_argument("--ttl-hours", type=int, default=DEFAULT_TTL_HOURS)
    c.set_defaults(fn=cmd_claim)

    c = sp.add_parser("release"); c.add_argument("id", nargs="?"); c.set_defaults(fn=cmd_release)
    c = sp.add_parser("renumber"); c.add_argument("old"); c.add_argument("--to", required=True); c.set_defaults(fn=cmd_renumber)
    c = sp.add_parser("heartbeat"); c.add_argument("id", nargs="?"); c.add_argument("--model"); c.set_defaults(fn=cmd_heartbeat)
    c = sp.add_parser("register-bridge")
    c.add_argument("--name", required=True); c.add_argument("--harness", required=True)
    c.add_argument("--session"); c.add_argument("--model"); c.add_argument("--pid", type=int)
    c.add_argument("--cwd"); c.add_argument("--ttl-hours", type=int, default=DEFAULT_TTL_HOURS)
    c.set_defaults(fn=cmd_register_bridge)

    c = sp.add_parser("list")
    c.add_argument("--all-hosts", action="store_true"); c.add_argument("--hosts")
    c.add_argument("--campaign"); c.add_argument("--json", action="store_true")
    c.set_defaults(fn=cmd_list)

    c = sp.add_parser("gc"); c.add_argument("--dry-run", action="store_true"); c.set_defaults(fn=cmd_gc)
    c = sp.add_parser("show"); c.add_argument("id"); c.set_defaults(fn=cmd_show)

    a = p.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
