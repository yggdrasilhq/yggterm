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
                     [--ttl-hours 24] [--pid supervisor-pid]  # idempotent per session
    ygg_row.py release [<id>]          # default: this session's claim
    ygg_row.py renumber <old> --to <new>
    ygg_row.py heartbeat [<id>] [--model m] [--pid supervisor-pid]
    ygg_row.py list [--all-hosts] [--campaign N] [--json]
    ygg_row.py show <id> | gc [--dry-run] | register-bridge --name s --harness h
                                         [--session u] [--model m] [--pid p]
"""

import argparse, errno, fcntl, json, math, os, re, socket, subprocess, sys, tempfile, time
from contextlib import contextmanager
from pathlib import Path

ID_RE = re.compile(r"^\d+(\.\d+)+(R|\.0)*$")
ROLES = ("worker", "orchestrator", "relay")
REMOTE_PATH = "~/gh/yggterm/.agents/skills/yggterm-agent-fleet/ygg_row.py"
FRESH_SECS = 600          # heartbeat newer than this = FRESH (owner: ~10 min)
DEFAULT_TTL_HOURS = 24    # metadata expiry; not evidence of a dead process


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


@contextmanager
def mutation_fence(store):
    # Same stable inode as claim allocation/publication, for both stores.
    with (store / "claims" / ".claim-transaction.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def local_identity():
    if not sys.platform.startswith("linux"):
        raise OSError("unsupported_platform")
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    if not boot:
        raise OSError("missing_boot_identity")
    if os.readlink("/proc/self") != str(os.getpid()):
        raise OSError("procfs_pid_namespace_mismatch")
    return {"platform": "linux", "host": socket.gethostname(), "boot_id": boot,
            "uid": os.getuid(), "pid_namespace": os.readlink("/proc/self/ns/pid")}


def process_sample(pid):
    # Read identity twice so a PID exiting/reappearing during sampling cannot
    # bind one process's uid to another process's start time.
    path = Path(f"/proc/{pid}")
    first = (path / "stat").read_text().rsplit(")", 1)[1].split()
    uid = path.stat().st_uid
    second = (path / "stat").read_text().rsplit(")", 1)[1].split()
    if first[19] != second[19]:
        raise OSError("process_changed_during_sample")
    return {"starttime": second[19], "uid": uid, "state": second[0]}


def bind_process(pid):
    if pid is None:
        return {"pid": None, "process_owner": None}
    if type(pid) is not int or not 0 < pid <= 2**31 - 1:
        sys.exit("⛔ --pid must be a positive live same-UID Linux supervisor PID")
    try:
        identity = local_identity()
        sample = process_sample(pid)
        if sample["uid"] != identity["uid"] or sample["state"] in ("Z", "X", "x"):
            raise OSError("not a live same-UID process")
    except (OSError, ValueError, IndexError) as ex:
        sys.exit(f"⛔ cannot bind --pid: {ex}")
    return {"pid": pid, "process_owner": {**identity, "starttime": sample["starttime"]}}


def process_liveness(record):
    def result(state, reason):
        return {"state": state, "reason": reason}
    if record.get("_record_error"):
        return result("UNKNOWN", "malformed_record")
    pid, owner = record.get("pid"), record.get("process_owner")
    if pid is None and owner is None:
        return result("UNBOUND", "no_explicit_supervisor")
    if type(pid) is not int or not 0 < pid <= 2**31 - 1 or not isinstance(owner, dict):
        return result("UNKNOWN", "legacy_or_invalid_binding")
    if (not isinstance(owner.get("starttime"), str)
            or not owner["starttime"].isdigit()):
        return result("UNKNOWN", "invalid_starttime")
    try:
        identity = local_identity()
        if any(owner.get(k) != v for k, v in identity.items()):
            return result("UNKNOWN", "foreign_host_boot_or_uid")
        if record.get("host") != identity["host"]:
            return result("UNKNOWN", "foreign_record_host")
        try:
            sample = process_sample(pid)
        except FileNotFoundError:
            # Procfs absence is not death: a mount can hide only this target.
            # A namespace-bound kernel PID lookup is an independent witness.
            if not hasattr(os, "pidfd_open"):
                return result("UNKNOWN", "no_positive_death_witness")
            try:
                fd = os.pidfd_open(pid)
            except OSError as exc:
                if exc.errno == errno.ESRCH:
                    return result("DEAD", "kernel_pid_absent")
                return result("UNKNOWN", "kernel_pid_visibility_unknown")
            else:
                os.close(fd)
                return result("UNKNOWN", "kernel_pid_exists_procfs_hidden")
        if sample["uid"] != identity["uid"]:
            return result("UNKNOWN", "different_uid")
        if sample["starttime"] != owner["starttime"]:
            return result("REUSED", "starttime_mismatch")
        if sample["state"] in ("Z", "X", "x"):
            return result("DEAD", "bound_process_exited")
        return result("ALIVE", "identity_matches")
    except (OSError, ValueError, IndexError):
        return result("UNKNOWN", "unsupported_or_inaccessible_process")


def freshness(record):
    hb = record.get("heartbeat")
    ttl = record.get("ttl_hours", DEFAULT_TTL_HOURS)
    if (record.get("_record_error") or type(hb) not in (int, float)
            or type(ttl) not in (int, float)
            or abs(hb) > 2**63 - 1 or abs(ttl) > 2**63 - 1
            or not math.isfinite(hb)
            or not math.isfinite(ttl) or hb <= 0 or ttl <= 0):
        return "UNKNOWN"
    age = time.time() - hb
    return "FRESH" if age < FRESH_SECS else ("stale" if age < ttl * 3600 else "EXPIRED")


def fresh(claim):
    return freshness(claim) == "FRESH"


def pin_state(record):
    # Recognize existing boolean pin spellings. Missing/ambiguous policy is
    # never permission to remove an old or externally authored record.
    values = [record[k] for k in ("owner_pinned", "pinned", "pin") if k in record]
    if any(v is not False for v in values):
        return "PINNED_OR_UNKNOWN"
    return "UNPINNED" if values else "UNKNOWN"


def read_record(path):
    try:
        obj = json.loads(path.read_text())
        if isinstance(obj, dict):
            return obj
        reason = "non_object_json"
    except (OSError, ValueError):
        reason = "unreadable_or_invalid_json"
    # Bounded typed witness; never include arbitrary malformed payload bytes.
    return {"id": path.stem, "_record_error": reason}


def gc_eligible(record):
    return (not record.get("_record_error") and freshness(record) == "EXPIRED"
            and pin_state(record) == "UNPINNED"
            and process_liveness(record)["state"] == "DEAD")


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
        if explicit_id.split(".")[0] != n:
            sys.exit("⛔ explicit id belongs to another campaign")
        p = store / "claims" / f"{explicit_id}.json"
        if os.path.lexists(p):
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
        # Reserve filenames even when their JSON is malformed or unknown.
        # Stored x values are strings; integer candidates must compare with
        # integers. Never interpret an unreadable occupied seat as free.
        used = {int(p.stem.split(".")[1].rstrip("R"))
                for p in (store / "claims").glob("*.json")
                if ID_RE.fullmatch(p.stem)
                and p.stem.split(".")[0] == n}
        x = 1
        while x in used:
            x += 1
        if role == "orchestrator":
            if os.path.lexists(store / "claims" / f"{n}.0.json"):
                sys.exit(f"⛔ {n}.0 is already claimed")
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
    # Hold one stable flock inode through lookup, allocation AND durable
    # publication. The old allocator released its lock before the write.
    with mutation_fence(store):
        _claim_locked(a, store, n, name)


def _claim_locked(a, store, n, name):
    prior = find_own(store, n)
    if prior and not a.id:
        if getattr(a, "pid", None) is not None:
            requested = bind_process(a.pid)
            if any(prior.get(k) != v for k, v in requested.items()):
                sys.exit("⛔ existing claim has a different binding; use heartbeat --pid explicitly")
        print(f"= this session already holds {prior['id']} (campaign {n}) — idempotent")
        return
    rid = allocate(store, n, a.role, a.id)
    if a.role == "orchestrator" and not rid.endswith(".0"):
        sys.exit("⛔ --role orchestrator must end in .0 (master N.0 or sub N.X.0)")
    if a.role == "relay" and not rid.endswith("R"):
        sys.exit("⛔ --role relay must end in R (N.XR)")
    claim = {"id": rid, "n": n, "x": rid.split(".")[1].rstrip("R"), "campaign": name,
             "role": a.role, "harness": a.harness or os.environ.get("YGGTERM_HARNESS", "unknown"),
             "session": this_session(), "host": socket.gethostname(),
             **bind_process(getattr(a, "pid", None)), "owner_pinned": False,
             "model": a.model, "cwd": os.getcwd(), "purpose": a.purpose,
             "parent": a.parent, "bridge_sub": a.bridge_sub, "row_path": a.row_path,
             "ttl_hours": a.ttl_hours, "claimed_at": time.time(), "heartbeat": time.time()}
    destination = store / "claims" / f"{rid}.json"
    fd, temporary = tempfile.mkstemp(prefix=".claim-", dir=destination.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(claim, stream, indent=1)
            stream.flush()
            os.fsync(stream.fileno())
        # Never replace a seat introduced by an external/legacy writer
        # between allocation and publication, including a dangling symlink.
        os.link(temporary, destination)
        directory = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except FileExistsError:
        sys.exit(f"⛔ {rid} became occupied before publication; claim retained")
    finally:
        os.unlink(temporary)
    journal(store, "claim", id=rid, n=n, role=a.role, purpose=a.purpose)
    print(f"✓ claimed {rid} ({name}, {a.role})" + (f" — {a.purpose}" if a.purpose else ""))


def find_own(store, n=None):
    for p in sorted((store / "claims").glob("*.json")):
        c = load_json(p, {}) or {}
        if not isinstance(c, dict):
            continue  # opaque occupied seat; allocation reserves its filename
        if c.get("session") == this_session() and (n is None or c.get("n") == n):
            return c
    return None


def find_claim(store, ident):
    p = store / "claims" / f"{ident}.json"
    if p.exists():
        return p, read_record(p)
    for pp in (store / "claims").glob("*.json"):     # by session uuid
        c = read_record(pp)
        if (isinstance(c.get("session"), str) and c["session"].startswith(ident)) or c.get("bridge_sub") == ident:
            return pp, c
    sys.exit(f"⛔ no claim matches `{ident}`")


def cmd_release(a):
    store = rows_dir()
    with mutation_fence(store):
        return _release_locked(a, store)


def _release_locked(a, store):
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
    with mutation_fence(store):
        return _renumber_locked(a, store)


def _renumber_locked(a, store):
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
    with mutation_fence(store):
        return _heartbeat_locked(a, store)


def _heartbeat_locked(a, store):
    if a.id:
        p, c = find_claim(store, a.id)
    else:
        c = find_own(store)
        if not c:
            print("= no claim to heartbeat"); return
        p = store / "claims" / f"{c['id']}.json"
    c["heartbeat"] = time.time()
    if a.model: c["model"] = a.model
    if c.get("_record_error"):
        sys.exit("⛔ cannot heartbeat malformed metadata")
    if getattr(a, "pid", None) is not None:
        c.update(bind_process(a.pid))
    write_json(p, c)
    print(f"♥ {c['id']}")


def cmd_register_bridge(a):
    """A bridge subscriber registers itself (UNCLAIMED — registration ≠ claim)."""
    store = rows_dir()
    with mutation_fence(store):
        return _register_bridge_locked(a, store)


def _register_bridge_locked(a, store):
    reg = {"sub": a.name, "harness": a.harness, "session": a.session or this_session(),
           "host": socket.gethostname(), **bind_process(getattr(a, "pid", None)), "model": a.model,
           "owner_pinned": False,
           "cwd": a.cwd or os.getcwd(), "registered_at": time.time(),
           "heartbeat": time.time(), "ttl_hours": a.ttl_hours}
    destination = store / "bridges" / f"{a.name}.json"
    if destination.exists():
        previous = read_record(destination)
        if previous.get("_record_error"):
            sys.exit("⛔ cannot replace malformed bridge metadata")
        if getattr(a, "pid", None) is None:
            reg.update({k: previous.get(k) for k in ("pid", "process_owner")})
        # Registration refresh must not silently unpin an existing subscriber.
        reg.pop("owner_pinned")
        reg.update({k: previous[k] for k in ("owner_pinned", "pinned", "pin") if k in previous})
    write_json(destination, reg)
    journal(store, "bridge-register", sub=a.name, harness=a.harness)
    print(f"✓ bridge `{a.name}` registered in the row registry (unclaimed)")


def known_hosts(store):
    """Peer hosts are OBSERVED IN THE REGISTRY — no hardcoded fleet names in a
    public repo. --all-hosts fans out to every host that has ever claimed here,
    plus --hosts overrides."""
    hosts = set()
    for sub in ("claims", "bridges"):
        for p in (store / sub).glob("*.json"):
            h = read_record(p).get("host")
            if isinstance(h, str) and h:
                hosts.add(h)
    return sorted(hosts - {socket.gethostname()})


def collect(store):
    out = []
    for directory, kind in (("claims", "claim"), ("bridges", "bridge")):
        for p in sorted((store / directory).glob("*.json")):
            c = read_record(p)
            c.update({"_kind": kind, "heartbeat_freshness": freshness(c),
                      "process_liveness": process_liveness(c), "pin_state": pin_state(c)})
            out.append(c)
    return out


def render(entries, ttl_default=DEFAULT_TTL_HOURS):
    hdr = f"{'ID':<10} {'ROLE':<12} {'MODEL':<16} {'HARNESS':<10} {'FRESH':<7} {'PROCESS':<7} {'HOST':<7} PURPOSE"
    lines = [hdr, "-" * len(hdr)]
    for c in entries:
        rid = str(c.get("id") or f"({c.get('sub', '?')})")
        role = str(c.get("role") or ("bridge-sub" if c.get("_kind") == "bridge" else "?"))
        state = freshness(c)
        live = process_liveness(c)["state"]
        purpose = str(c.get("purpose") or c.get("cwd") or "")[:48]
        lines.append(f"{rid:<10} {role:<12} {str(c.get('model') or '-'):<16} "
                     f"{str(c.get('harness') or '-'):<10} {state:<7} {live:<7} {str(c.get('host') or '-'):<7} {purpose}")
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
    """Remove expired metadata only; never signal a process.

    Cooperative claim/heartbeat/rebind/registration mutations share this fence.
    Legacy writers bypassing the fence are not certified race-safe.
    """
    store = rows_dir()
    reaped = []
    for directory in ("claims", "bridges"):
        for p in sorted((store / directory).glob("*.json")):
            # Do not follow links outside the store or unlink opaque records.
            if p.is_symlink() or not gc_eligible(read_record(p)):
                continue
            with mutation_fence(store):
                # The pre-lock observation is only a candidate. Reread every
                # criterion after any in-flight heartbeat/rebind/pin completes.
                c = read_record(p)
                if p.is_symlink() or not gc_eligible(c):
                    continue
                ident = c.get("id") or c.get("sub") or p.stem
                if a.dry_run:
                    print(f"would reap {ident} (expired metadata, bound owner dead)")
                else:
                    p.unlink()
                    journal(store, "gc-reap", id=ident)
                    print(f"♻ reaped {ident}")
                reaped.append(p)
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
    c.add_argument("--harness"); c.add_argument("--pid", type=int); c.add_argument("--ttl-hours", type=int, default=DEFAULT_TTL_HOURS)
    c.set_defaults(fn=cmd_claim)

    c = sp.add_parser("release"); c.add_argument("id", nargs="?"); c.set_defaults(fn=cmd_release)
    c = sp.add_parser("renumber"); c.add_argument("old"); c.add_argument("--to", required=True); c.set_defaults(fn=cmd_renumber)
    c = sp.add_parser("heartbeat"); c.add_argument("id", nargs="?"); c.add_argument("--model"); c.add_argument("--pid", type=int); c.set_defaults(fn=cmd_heartbeat)
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
