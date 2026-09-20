#!/usr/bin/env python3
"""ygg-auth — the fleet auth-rotation plane (base tier, like booter/monitor/ygg-ci).

DEFECT THIS EXISTS FOR (owner, 2026-09-20):
  One subscription per harness strands the fleet: when codex hits its usage
  limit, every codex row on the host parks until it resets, and the only
  remedy was a human hand-editing ~/.codex/auth.json. The owner keeps
  multiple paid subscriptions per harness precisely so the fleet can rotate;
  the switch just did not exist. Proven prototype: switch-chatgpt.py on the
  litellm LXC (device-code login + per-account snapshots), productized here
  for the fleet's own harnesses.

SHAPE:
  Per harness, ONE live auth file (~/.codex/auth.json for codex) and a local
  profile store ~/.yggterm/auth/<harness>/<email-slug>.json holding VERBATIM
  harness auth records. Verbatim, not normalized: restore = byte-faithful
  swap, so format evolution (codex added auth_mode + last_refresh) cannot
  corrupt a stored profile.

  capture-on-leave is the rotation invariant: before a switch overwrites the
  live file it re-snapshots the CURRENT live record into its own profile,
  because the harness refreshes tokens in place while running. Switching is
  therefore lossless in both directions.

  Agents rotate themselves: on a usage-limit wall, `ygg-auth.py rotate`
  moves the harness to the next fresh profile — no human in the loop. login
  (device-code flow) is the one verb that touches the network and needs the
  owner's browser.

LAWS THIS TOOL LIVES UNDER:
  - Harness Isolation Law (SKILL.md §5): private harness stores are private.
    ygg-auth is the ONE sanctioned writer, whole-file atomic swap only —
    never edit a harness auth file piecemeal.
  - Memory-sync law: credentials never travel between machines. The store is
    per-host, never in ~/.yggterm/memory, never posted, never logged.
  - Redaction: no verb prints a token value. Only email, plan, account id,
    expiry. Errors never echo file contents.

VERBS:
  status  [--harness H] [--json]   live identity + every stored profile
  list    [--harness H] [--json]   stored profiles only
  capture [--harness H] [--name S] snapshot the live record into the store
  switch  <slug> [--harness H] [--json]  capture-on-leave, then atomic swap
  rotate  [--harness H] [--fast] [--cooldown-minutes N] [--json]
                                   rotate off the current profile (rate-limit
                                   verb); ranks candidates by MEASURED headroom
                                   from the usage endpoint, clock heuristic when
                                   --fast or the API is down; the vacated account
                                   goes on cooldown (default 30 min)
  login   [--harness H] [--name S] [--no-notify]
                                   device-code OAuth (codex only; owner completes
                                   it); the code also lands on a pinned infra/auth
                                   card that auto-closes when the poll ends
  import  <file> [--harness H] [--name S]
                                   convert a foreign auth record (the litellm
                                   prototype's flat {access_token, refresh_token,
                                   id_token, expires_at, account_id}) into this
                                   harness's shape and store it — seeds profiles
                                   without a browser login
  fetch   <host> <slug> [--harness H] [--activate]
                                   copy a profile from another fleet host's store
                                   over ssh (owner-authorized use; the record is
                                   pulled, never pushed) and optionally activate it.
                                   Destinations self-learn: a working user@host is
                                   remembered under its bare alias
  usage   [--harness H] [--slug S] [--json]
                                   measured rate-limit state per profile from
                                   chatgpt.com/backend-api/codex/usage — per-plan
                                   windows (5h/7d on plus, longer on free) with
                                   used_percent and resets; read-only, never refreshes
  refresh [<slug>] [--harness H] [--force]
                                   renew tokens via the OAuth refresh grant and
                                   persist atomically (default: the live profile).
                                   ⛔ single-writer, now ENFORCED from provenance:
                                   refreshing a lineage owned by another host
                                   refuses without --force
  fleet   [--hosts a,b,c] [--save]
                                   accounts × hosts matrix over ssh — who is live
                                   on what, per host (dream ACK-68ce18afa6)
  doctor  [--harness H] [--json]   flag stale replicas against each lineage's
                                   recorded writer; re-fetch where it says STALE
  provenance [--harness H] [--json]
                                   the raw lineage sidecar (writer, stamps)
  adopt   [--harness H] [--name S] [--json]
                                   adopt the account the litellm proxy currently
                                   holds — its live auth.json is the only fresh
                                   copy of its lineages (dream ACK-447fefa6b4)
  litellm-switch <slug> [--harness H]
                                   push a stored profile INTO litellm, restart the
                                   stack, verify health; litellm owns the lineage
                                   afterward — re-adopt to re-sync the fleet
  harnesses                        the registry: what each harness supports

Applies to NEW harness invocations: a running codex/claude session keeps the
tokens it loaded at start and refreshes within its account.

CODEX_HOME / CLAUDE_CONFIG_DIR are honored, so `codex-litellm`
(~/.codex-litellm) can carry its own rotation set: CODEX_HOME=~/.codex-litellm
ygg-auth.py rotate.

Exit codes: 0 ok · 2 nothing to rotate to · 3 live auth file missing ·
4 login not supported for the harness · 5 auth/network protocol failure ·
6 bad usage (unknown harness/slug).
"""
import argparse
import base64
import fcntl
import hashlib
import json
import os
import re
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

STORE_ROOT = os.environ.get("YGG_AUTH_HOME") or os.path.expanduser("~/.yggterm/auth")
LOCK_FILE = os.path.join(STORE_ROOT, ".lock")
DEFAULT_COOLDOWN_MINUTES = 30
HOSTNAME = socket.gethostname()
FLEET_HOSTS_FILE = os.path.join(STORE_ROOT, ".fleet-hosts")
SSH_DESTS_FILE = os.path.join(STORE_ROOT, ".ssh-dests.json")
PROVENANCE_FILE = os.path.join(STORE_ROOT, ".provenance")
LITELLM_VIA_HOST = "manin"
LITELLM_CONTAINER = "litellm"
LITELLM_TOKEN_DIR = "/root/chatgpt_tokens"
LITELLM_CONTAINER_NAME = "root-litellm-1"

CHATGPT_AUTH_BASE = "https://auth.openai.com"
CHATGPT_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
CODEX_USER_AGENT = "codex_cli_rs/0.0.0"
# Measured live 2026-09-20 (dev, real token): bearer + ChatGPT-Account-Id →
# {email, plan_type, rate_limit:{allowed, limit_reached, primary_window{used_percent,
# limit_window_seconds, reset_after_seconds, reset_at}, secondary_window{...}}}.
CODEX_USAGE_URL = "https://chatgpt.com/backend-api/codex/usage"
DEVICE_CODE_TIMEOUT_SECONDS = 900
POLL_INTERVAL_SECONDS = 5


def harness_auth_file(harness):
    """The harness's live auth file, honoring the CLIs' own relocation env vars."""
    home = os.environ.get("CODEX_HOME") if harness == "codex" else None
    if harness == "codex":
        return os.path.join(home or os.path.expanduser("~/.codex"), "auth.json")
    if harness == "claude":
        home = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")
        return os.path.join(home, ".credentials.json")
    raise KeyError(harness)


HARNESSES = {
    "codex": {
        "auth_file": harness_auth_file,
        "login": "device-code (auth.openai.com; the owner completes the browser step)",
        "live_note": "running codex sessions keep the tokens they loaded at start",
    },
    "claude": {
        "auth_file": harness_auth_file,
        "login": None,
        "live_note": "swap supported; login flow not implemented (add when the need lands)",
    },
}


class AuthError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def require_harness(harness):
    if harness not in HARNESSES:
        raise AuthError(6, f"unknown harness {harness!r}; known: {', '.join(sorted(HARNESSES))}")
    return harness


def store_dir(harness):
    return os.path.join(STORE_ROOT, harness)


def decode_jwt_claims(token):
    try:
        parts = token.split(".")
        if len(parts) < 2:
            return {}
        payload = parts[1] + "=" * (-len(parts[1]) % 4)
        return json.loads(base64.urlsafe_b64decode(payload).decode("utf-8"))
    except Exception:
        return {}


def identity_of(record):
    """Claims only — this dict is what every verb is allowed to print."""
    tokens = record.get("tokens") if isinstance(record.get("tokens"), dict) else record
    id_token = tokens.get("id_token") or ""
    access_token = tokens.get("access_token") or ""
    id_claims = decode_jwt_claims(id_token) if id_token else {}
    access_claims = decode_jwt_claims(access_token) if access_token else {}
    auth_claim = (id_claims.get("https://api.openai.com/auth")
                  or access_claims.get("https://api.openai.com/auth") or {})
    profile_claim = access_claims.get("https://api.openai.com/profile") or {}
    email = id_claims.get("email") or profile_claim.get("email")
    account_id = tokens.get("account_id") or auth_claim.get("chatgpt_account_id")
    expires_at = record.get("expires_at") or access_claims.get("exp") or 0
    try:
        expired = bool(expires_at) and time.time() > float(expires_at)
    except (TypeError, ValueError):
        expired = False
    plan = auth_claim.get("chatgpt_plan_type")
    if not plan and not access_token and record.get("OPENAI_API_KEY"):
        plan = "api-key"
    return {
        "slug": slug_for(email) if email else None,
        "email": email or "unknown",
        "plan": plan or "unknown",
        "account_id": account_id or "none",
        "expires_at": expires_at,
        "expired": expired,
    }


def slug_for(email):
    return re.sub(r"[^a-zA-Z0-9_\-.]", "_", email).lower()


# Cooldown state is OPERATIONAL, not identity — the one thing that cannot be
# derived from the stored record (consult 2026-09-20: blind round-robin cycles
# straight back into a quota-locked account). Sidecar file, never credentials.
def cooldown_path(harness):
    return os.path.join(store_dir(harness), ".cooldown.json")


def load_cooldowns(harness):
    try:
        return read_json(cooldown_path(harness))
    except (OSError, ValueError):
        return {}


def save_cooldowns(harness, data):
    atomic_write_json(cooldown_path(harness), data)


def mark_cooldown(harness, slug, minutes):
    if minutes <= 0:
        return None
    until = time.time() + minutes * 60
    data = load_cooldowns(harness)
    data[slug] = until
    save_cooldowns(harness, data)
    return until


def cooling_until(harness, slug):
    until = load_cooldowns(harness).get(slug)
    return float(until) if until and float(until) > time.time() else None


# Provenance — dream ACK-68ce18afa6. The auth record itself stays VERBATIM
# (restore = byte-faithful swap), so lineage facts live in a sidecar:
#   slug -> {writer, writer_updated_at, source, fetched_from, updated_at}
# writer = the host that owns the lineage (the only one that may refresh it);
# writer_updated_at = the writer's own mutation stamp, copied at fetch time,
# which is what `doctor` compares to flag stale replicas.
def provenance_path(harness):
    return os.path.join(store_dir(harness), ".provenance.json")


def load_provenance(harness):
    try:
        return read_json(provenance_path(harness))
    except (OSError, ValueError):
        return {}


def save_provenance(harness, prov):
    atomic_write_json(provenance_path(harness), prov)


def record_hash(record):
    return hashlib.sha256(json.dumps(record, sort_keys=True).encode()).hexdigest()[:16]


def stamp_provenance(harness, slug, record=None, **kw):
    prov = load_provenance(harness)
    entry = prov.get(slug) or {}
    entry.update(kw)
    if record is not None:
        entry["content_hash"] = record_hash(record)
    entry["updated_at"] = time.time()
    prov[slug] = entry
    save_provenance(harness, prov)
    return entry


def get_writer(harness, slug):
    return (load_provenance(harness).get(slug) or {}).get("writer")


# Self-learning ssh destinations — dream ACK-62a8b4abe4. Hosts disagree about
# who an alias is (oc: dev=root, jojo: dev=pi); remember per host which
# destination actually reached a store, and try memories before the raw alias.
def load_ssh_dests():
    try:
        return read_json(SSH_DESTS_FILE)
    except (OSError, ValueError):
        return {}


def remember_ssh_dest(dest):
    if "@" not in dest:
        return
    bare = dest.split("@", 1)[1]
    mem = load_ssh_dests()
    variants = mem.get(bare, [])
    if dest not in variants:
        mem[bare] = [dest] + variants
        atomic_write_json(SSH_DESTS_FILE, mem)


def ssh_dest_candidates(dest):
    if "@" in dest:
        return [dest]
    mem = load_ssh_dests().get(dest, [])
    return (mem + [dest]) if dest not in mem else mem


def load_fleet_hosts():
    try:
        with open(FLEET_HOSTS_FILE) as f:
            return [h.strip() for h in f.read().split(",") if h.strip()]
    except OSError:
        return []


def save_fleet_hosts(hosts):
    os.makedirs(STORE_ROOT, exist_ok=True)
    with open(FLEET_HOSTS_FILE, "w") as f:
        f.write(",".join(hosts))


def ssh_script(host_dest, argv, script_bytes, timeout=40):
    """Run this script on a remote host via stdin (no remote install needed)."""
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host_dest,
           "python3", "-", *argv]
    try:
        return subprocess.run(cmd, input=script_bytes, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None


def read_json(path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def atomic_write_json(path, record, mode=0o600):
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=directory, prefix=".tmp-auth-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(record, f, indent=2)
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp, mode)
        os.chmod(directory, 0o700)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise
    try:
        dir_fd = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(dir_fd)
        finally:
            os.close(dir_fd)
    except OSError:
        pass


class fleet_lock:
    """Exclusive fleet-wide lock over every mutating verb; readers need none."""

    def __init__(self):
        self.fd = None

    def __enter__(self):
        os.makedirs(STORE_ROOT, exist_ok=True)
        self.fd = os.open(LOCK_FILE, os.O_RDWR | os.O_CREAT, 0o600)
        fcntl.flock(self.fd, fcntl.LOCK_EX)
        return self

    def __exit__(self, *exc):
        fcntl.flock(self.fd, fcntl.LOCK_UN)
        os.close(self.fd)
        return False


def live_record(harness, required=False):
    path = HARNESSES[harness]["auth_file"](harness)
    if not os.path.exists(path):
        if required:
            raise AuthError(3, f"no live auth file at {path} — run `login` first")
        return None
    try:
        return read_json(path)
    except (OSError, ValueError) as exc:
        raise AuthError(3, f"live auth file at {path} is unreadable ({type(exc).__name__}); refusing to touch it")


def capture(harness, name=None, lock_held=False):
    """Snapshot the live record verbatim into the store. Returns (slug, path)."""
    record = live_record(harness, required=True)
    ident = identity_of(record)
    if not ident["slug"] and not name:
        raise AuthError(3, "live record carries no usable identity; pass --name to store it anyway")
    slug = name or ident["slug"]
    path = os.path.join(store_dir(harness), f"{slug}.json")
    if not lock_held:
        with fleet_lock():
            return _capture_locked(harness, record, slug, path, ident)
    return _capture_locked(harness, record, slug, path, ident)


def _capture_locked(harness, record, slug, path, ident):
    atomic_write_json(path, record)
    stamp_provenance(harness, slug, record=record, writer=HOSTNAME, source="capture",
                     writer_updated_at=time.time())
    return {**ident, "slug": slug, "path": path}


def stored_profiles(harness):
    directory = store_dir(harness)
    if not os.path.isdir(directory):
        return []
    profiles = []
    for fname in sorted(os.listdir(directory)):
        if not (fname.startswith("auth_") or fname.endswith(".json")) or fname.startswith("."):
            continue
        if not fname.endswith(".json"):
            continue
        path = os.path.join(directory, fname)
        try:
            record = read_json(path)
        except (OSError, ValueError):
            continue
        ident = identity_of(record)
        ident.update({"path": path, "filename": fname})
        profiles.append(ident)
    return profiles


def _live_fingerprint(path):
    st = os.stat(path)
    return (st.st_mtime_ns, st.st_size)


def switch(harness, slug, lock_held=False):
    """capture-on-leave, then byte-faithful atomic swap to the named profile.

    The live file is re-read if the harness rewrote it mid-swap: OAuth refresh
    rotates refresh tokens, so capturing a stale record would store (and later
    restore) a revoked token — consult 2026-09-20. The replace happens only
    when the file is fingerprint-unchanged from the read; three attempts, then
    fail loudly rather than swap silently stale.
    """
    slug = slug_for(slug)  # accept the raw email; store/lookup use one normalization
    target = os.path.join(store_dir(harness), f"{slug}.json")
    if not os.path.exists(target):
        known = ", ".join(p["slug"] or "?" for p in stored_profiles(harness)) or "(none stored)"
        raise AuthError(6, f"no stored profile {slug!r} for {harness}; stored: {known}")
    live_path = HARNESSES[harness]["auth_file"](harness)
    target_record = read_json(target)

    captured = None
    if os.path.exists(live_path):
        swapped = False
        for _ in range(3):
            before = _live_fingerprint(live_path)
            current = live_record(harness, required=True)
            current_ident = identity_of(current)
            if current_ident["slug"] and current_ident["slug"] != slug:
                captured = _capture_locked(harness, current, current_ident["slug"],
                                           os.path.join(store_dir(harness), f"{current_ident['slug']}.json"),
                                           current_ident)
            if _live_fingerprint(live_path) != before:
                continue  # the harness rewrote mid-read; its tokens would be lost — redo
            atomic_write_json(live_path, target_record)
            swapped = True
            break
        if not swapped:
            raise AuthError(5, f"the live auth file kept changing during capture ({live_path}); not switching")
    else:
        atomic_write_json(live_path, target_record)

    swapped = identity_of(live_record(harness, required=True))
    if swapped["slug"] and swapped["slug"] != slug:
        raise AuthError(5, f"post-switch verification failed: live file reports {swapped['slug']!r}")
    return {"switched_to": slug, "identity": swapped, "captured_before_leaving": captured}


def rotate_score(p, measure=True):
    """(tier, headroom, slug): tier 0 measured-ok, 1 measured-limited,
    2 unmeasurable (expired access), 3 measurement failed. Headroom is the
    primary window's free percent — higher is better. Module-level so tests
    can drive it with a stubbed usage_snapshot."""
    if not measure:
        return (0, 0, p["slug"])
    try:
        snap = usage_snapshot(read_json(p["path"]))
    except (OSError, ValueError):
        snap = {"ok": False, "error": "unreadable"}
    if snap.get("ok"):
        pri = snap.get("primary") or {}
        used = pri.get("used_percent")
        tier = 1 if snap.get("limit_reached") else 0
        return (tier, 100 - (used if isinstance(used, (int, float)) else 100), p["slug"])
    if p["expired"]:
        return (2, 0, p["slug"])  # dead access: measurable only after activation
    return (3, 0, p["slug"])


def pick_rotate_target(harness, measure=True):
    """Next profile to rotate to (dream ACK-37c41cbde1): when measurement is
    on, candidates rank by MEASURED headroom from the usage endpoint — an
    account at 2% beats one at 100% regardless of slug order. Falls back to
    the clock heuristic (slug order + cooldowns) when the API is unreachable
    or --fast was passed."""
    profiles = stored_profiles(harness)
    if not profiles:
        raise AuthError(2, f"no stored profiles for {harness} — nothing to rotate to; run `capture` or `login`")
    current = identity_of(live_record(harness)) if os.path.exists(HARNESSES[harness]["auth_file"](harness)) else None
    current_slug = current["slug"] if current else None
    others = [p for p in profiles if p["slug"] and p["slug"] != current_slug]
    if not others:
        raise AuthError(2, f"the only profile is the live one ({current_slug}); capture or login another account first")

    def cooldown_left(p):
        until = cooling_until(harness, p["slug"])
        return (until - time.time()) if until else 0.0

    def warn(message):
        print(f"⚠️  {message}", file=sys.stderr)

    measured = {}
    if measure:
        for p in others:
            measured[p["slug"]] = rotate_score(p, measure=True)
        if all(t == 3 for t, _, _ in measured.values()):
            warn("usage endpoint unreachable for every candidate — falling back to slug order")
            measure = False

    fresh = [p for p in others if not p["expired"] and cooldown_left(p) <= 0]
    if fresh:
        if measure:
            pick = min(fresh, key=lambda p: measured.get(p["slug"], (0, 0, p["slug"])))
        else:
            pick = min(fresh, key=lambda p: p["slug"])
        return pick, True
    if all(cooldown_left(p) > 0 for p in others) and all(not p["expired"] for p in others):
        warn("every other profile is rate-limit cooling; rotating to the one closest to its cooldown end")
    elif not any(not p["expired"] for p in others):
        warn("every other profile is expired; rotating to the least-stale one anyway")
    else:
        warn("no fully fresh profile; rotating to the best available")
    pool = [p for p in others if not p["expired"]] or others
    if measure:
        return min(pool, key=lambda p: (cooldown_left(p), measured.get(p["slug"], (0, 0, p["slug"])))), False
    return min(pool, key=lambda p: (cooldown_left(p), p["slug"])), False


def cmd_status(args):
    harness = require_harness(args.harness)
    ident = identity_of(live_record(harness)) if os.path.exists(HARNESSES[harness]["auth_file"](harness)) else None
    profiles = stored_profiles(harness)
    prov = load_provenance(harness)
    for p in profiles:
        p["live"] = bool(ident and p["slug"] and p["slug"] == ident["slug"])
        cooling = cooling_until(harness, p["slug"]) if p["slug"] else None
        p["cooling_until"] = cooling
        p["cooldown_minutes_left"] = round((cooling - time.time()) / 60) if cooling else 0
        p["provenance"] = prov.get(p["slug"] or "")
    out = {
        "harness": harness,
        "host": HOSTNAME,
        "auth_file": HARNESSES[harness]["auth_file"](harness),
        "live": ident,
        "profiles": [{k: v for k, v in p.items() if k != "path"} for p in profiles],
    }
    return out


def cmd_list(args):
    out = cmd_status(args)
    return {"harness": out["harness"], "profiles": out["profiles"]}


def cmd_capture(args):
    harness = require_harness(args.harness)
    with fleet_lock():
        got = capture(harness, name=args.name, lock_held=True)
    return {"captured": got["slug"], "path": got["path"], "email": got["email"], "plan": got["plan"]}


def cmd_switch(args):
    harness = require_harness(args.harness)
    if not args.slug:
        raise AuthError(6, "switch needs a profile slug (see `list`)")
    with fleet_lock():
        result = switch(harness, args.slug, lock_held=True)
    return result


def cmd_rotate(args):
    harness = require_harness(args.harness)
    with fleet_lock():
        target, _ = pick_rotate_target(harness, measure=not args.fast)
        vacated = identity_of(live_record(harness)) if os.path.exists(HARNESSES[harness]["auth_file"](harness)) else None
        result = switch(harness, target["slug"], lock_held=True)
        cooldown = None
        if (vacated and vacated["slug"] and vacated["slug"] != target["slug"]
                and args.cooldown_minutes > 0):
            cooldown = {"slug": vacated["slug"],
                        "until": mark_cooldown(harness, vacated["slug"], args.cooldown_minutes)}
    result["vacated_to_cooldown"] = cooldown
    return result


def cmd_harnesses(args):
    out = {}
    for name, spec in sorted(HARNESSES.items()):
        out[name] = {
            "auth_file": spec["auth_file"](name),
            "login": spec["login"],
            "live_note": spec["live_note"],
        }
    return {"harnesses": out, "store": STORE_ROOT}


def now_rfc3339_nanos():
    dt = datetime.now(timezone.utc)
    return dt.strftime("%Y-%m-%dT%H:%M:%S") + ".%09dZ" % (dt.microsecond * 1000)


def _msgboard_post(args_list):
    """Best-effort msgGraph post; the plane exists on fleet hosts, not everywhere."""
    msgboard = os.path.expanduser("~/data/msggraph/bin/msgboard")
    if not os.path.exists(msgboard):
        return None
    try:
        proc = subprocess.run([msgboard, *args_list], capture_output=True, text=True, timeout=20)
    except (subprocess.TimeoutExpired, OSError):
        return None
    m = re.search(r"ACK-[0-9a-f]+", proc.stdout)
    return m.group(0) if m else None


def device_login(harness, name=None, notify=True):
    """Ported from the proven litellm prototype: codex device-code OAuth.
    Dream ACK-52ebf788f0: the code is the one thing a human must touch, so it
    also goes where the owner actually looks — a pinned msgGraph card that
    auto-closes when the poll completes."""
    if HARNESSES[harness]["login"] is None:
        raise AuthError(4, f"login is not implemented for {harness}: {HARNESSES[harness]['live_note']}")

    def post_json(url, payload, as_form=False):
        if as_form:
            data = urllib.parse.urlencode(payload).encode()
            headers = {"Content-Type": "application/x-www-form-urlencoded"}
        else:
            data = json.dumps(payload).encode()
            headers = {"Content-Type": "application/json"}
        headers["User-Agent"] = CODEX_USER_AGENT
        req = urllib.request.Request(url, data=data, headers=headers)
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read().decode("utf-8"))

    try:
        code = post_json(f"{CHATGPT_AUTH_BASE}/api/accounts/deviceauth/usercode",
                         {"client_id": CHATGPT_CLIENT_ID})
    except (urllib.error.URLError, OSError, ValueError) as exc:
        raise AuthError(5, f"device-code request failed: {exc}")
    device_auth_id = code.get("device_auth_id")
    user_code = code.get("user_code") or code.get("usercode")
    interval = int(code.get("interval", POLL_INTERVAL_SECONDS))
    if not device_auth_id or not user_code:
        raise AuthError(5, "device-code response missing device_auth_id/user_code")

    print(f"  1) open  {CHATGPT_AUTH_BASE}/codex/device  in a browser")
    print(f"  2) enter the code:  {user_code}")
    print(f"  polling every {interval}s (timeout {DEVICE_CODE_TIMEOUT_SECONDS // 60} min)…", file=sys.stderr)
    notify_ack = None
    if notify:
        notify_ack = _msgboard_post([
            "post", "infra/auth", "--kind", "question", "--ttl-days", "0",
            "--tags", "ygg-auth,device-login",
            "--body",
            f"⏳ device login needed on {HOSTNAME} ({harness}): open "
            f"{CHATGPT_AUTH_BASE}/codex/device and enter code {user_code} "
            f"(expires in {DEVICE_CODE_TIMEOUT_SECONDS // 60} min). This card "
            f"auto-closes when the poll completes.",
        ])
        if notify_ack:
            print(f"  card posted: {notify_ack} (infra/auth)", file=sys.stderr)

    deadline = time.time() + DEVICE_CODE_TIMEOUT_SECONDS
    auth_code_data = None
    while time.time() < deadline:
        try:
            got = post_json(f"{CHATGPT_AUTH_BASE}/api/accounts/deviceauth/token",
                            {"device_auth_id": device_auth_id, "user_code": user_code})
            if all(k in got for k in ("authorization_code", "code_challenge", "code_verifier")):
                auth_code_data = got
                break
        except urllib.error.HTTPError as exc:
            if exc.code not in (403, 404):
                raise AuthError(5, f"device poll failed: HTTP {exc.code}")
        except (urllib.error.URLError, OSError, ValueError) as exc:
            raise AuthError(5, f"device poll failed: {exc}")
        sys.stderr.write(".")
        sys.stderr.flush()
        time.sleep(interval)
    if auth_code_data is None:
        if notify_ack:
            _msgboard_post(["answer", "infra/auth", notify_ack, "--body",
                            f"⛔ poll timed out on {HOSTNAME} — run login again for a fresh code"])
        raise AuthError(5, "timed out waiting for browser authorization")

    try:
        tokens = post_json(
            f"{CHATGPT_AUTH_BASE}/oauth/token",
            {"grant_type": "authorization_code", "code": auth_code_data["authorization_code"],
             "redirect_uri": f"{CHATGPT_AUTH_BASE}/deviceauth/callback",
             "client_id": CHATGPT_CLIENT_ID, "code_verifier": auth_code_data["code_verifier"]},
            as_form=True)
    except (urllib.error.URLError, OSError, ValueError) as exc:
        raise AuthError(5, f"token exchange failed: {exc}")
    if not all(k in tokens for k in ("access_token", "refresh_token", "id_token")):
        raise AuthError(5, "token exchange response missing required fields")

    record = {
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": None,
        "tokens": {
            "id_token": tokens["id_token"],
            "access_token": tokens["access_token"],
            "refresh_token": tokens["refresh_token"],
            "account_id": tokens.get("account_id") or identity_of(tokens).get("account_id") or "none",
        },
        "last_refresh": now_rfc3339_nanos(),
    }
    ident = identity_of(record)
    slug = name or ident["slug"] or "unnamed-account"
    if notify_ack:
        _msgboard_post(["answer", "infra/auth", notify_ack, "--body",
                        f"✅ login completed on {HOSTNAME} as {ident['email']} ({ident['plan']}); "
                        f"profile {slug} stored + live"])

    with fleet_lock():
        live_path = HARNESSES[harness]["auth_file"](harness)
        captured = None
        if os.path.exists(live_path):
            current = live_record(harness, required=True)
            current_ident = identity_of(current)
            if current_ident["slug"] and current_ident["slug"] != slug:
                captured = _capture_locked(harness, current, current_ident["slug"],
                                           os.path.join(store_dir(harness), f"{current_ident['slug']}.json"),
                                           current_ident)
        atomic_write_json(os.path.join(store_dir(harness), f"{slug}.json"), record)
        atomic_write_json(live_path, record)
    return {"logged_in": slug, "identity": ident, "captured_before_leaving": captured}


def cmd_login(args):
    return device_login(require_harness(args.harness), name=args.name,
                        notify=not args.no_notify)


def import_foreign(path, harness, name=None, writer=None):
    """Convert a flat litellm-prototype record into the harness's own shape.

    The prototype on the litellm LXC stores {access_token, refresh_token,
    id_token, expires_at, account_id}; codex wants auth_mode + tokens.{...} +
    last_refresh. The OAuth grants are the same — conversion re-wraps them,
    it does not mint anything.
    """
    require_harness(harness)
    try:
        flat = read_json(path)
    except (OSError, ValueError) as exc:
        raise AuthError(6, f"cannot read {path}: {type(exc).__name__}")
    if not isinstance(flat, dict) or not all(k in flat for k in ("access_token", "refresh_token", "id_token")):
        raise AuthError(6, "not a flat litellm auth record (need access_token, refresh_token, id_token)")
    if "tokens" in flat:
        raise AuthError(6, "record already carries a tokens block — import the flat prototype format, or use `capture`")
    record = {
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": None,
        "tokens": {
            "id_token": flat["id_token"],
            "access_token": flat["access_token"],
            "refresh_token": flat["refresh_token"],
            "account_id": flat.get("account_id") or identity_of(flat).get("account_id") or "none",
        },
        "last_refresh": now_rfc3339_nanos(),
    }
    ident = identity_of(record)
    slug = slug_for(name) if name else ident["slug"]
    if not slug:
        raise AuthError(6, "record carries no usable identity; pass --name to store it anyway")
    with fleet_lock():
        atomic_write_json(os.path.join(store_dir(harness), f"{slug}.json"), record)
    stamp_provenance(harness, slug, record=record, writer=writer or HOSTNAME, source="import",
                     writer_updated_at=time.time())
    return {"imported": slug, "identity": {k: v for k, v in ident.items() if k != "slug"}}


def cmd_import(args):
    if not args.file:
        raise AuthError(6, "import needs the path to the foreign auth record")
    return import_foreign(args.file, require_harness(args.harness), name=args.name)


def fetch_remote(host, harness, slug, activate=False):
    """Pull one profile from another host's store over ssh and store it here.

    Pull, never push: the remote host is the authority for its own records
    (owner-authorized cross-host provisioning, 2026-09-20). ssh must work
    keyless (BatchMode) — the fleet standard. Destinations are self-learned
    (dream ACK-62a8b4abe4): a working user@host is remembered under its bare
    alias and tried first next time. Provenance (dream ACK-68ce18afa6): the
    remote's writer + writer stamp ride along, so doctor can compare.
    """
    require_harness(harness)
    slug = slug_for(slug)
    remote_path = f"~/.yggterm/auth/{harness}/{slug}.json"
    record = None
    used_dest = None
    last_err = "no destination tried"
    for dest in ssh_dest_candidates(host):
        cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", dest,
               "cat", remote_path]
        try:
            proc = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        except subprocess.TimeoutExpired:
            last_err = f"ssh {dest} timed out"
            continue
        if proc.returncode == 0:
            try:
                record = json.loads(proc.stdout)
            except ValueError:
                last_err = f"{dest}:{remote_path} is not valid JSON"
                continue
            used_dest = dest
            break
        last_err = f"ssh {dest}: {proc.stderr.strip()[:160]}"
    if record is None:
        raise AuthError(5, f"could not read {slug} from {host}: {last_err}")
    remember_ssh_dest(used_dest)

    ident = identity_of(record)
    if ident["slug"] and ident["slug"] != slug:
        raise AuthError(6, f"record at {used_dest}:{remote_path} identifies as {ident['slug']!r}, not {slug!r}")

    # carry the remote's lineage stamp when the remote keeps provenance
    writer, writer_updated_at = None, None
    prov_cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", used_dest,
                "cat", f"~/.yggterm/auth/{harness}/.provenance.json"]
    try:
        prov_proc = subprocess.run(prov_cmd, capture_output=True, text=True, timeout=20)
        if prov_proc.returncode == 0:
            remote_prov = (json.loads(prov_proc.stdout) or {}).get(slug) or {}
            writer = remote_prov.get("writer")
            writer_updated_at = remote_prov.get("writer_updated_at")
    except (subprocess.TimeoutExpired, ValueError):
        pass
    if writer is None:
        writer = used_dest.split("@")[-1]
    if writer_updated_at is None:
        writer_updated_at = time.time()

    with fleet_lock():
        atomic_write_json(os.path.join(store_dir(harness), f"{slug}.json"), record)
        stamp_provenance(harness, slug, record=record, writer=writer,
                         writer_updated_at=writer_updated_at,
                         source="fetch", fetched_from=used_dest)
        if activate:
            return {**switch(harness, slug, lock_held=True), "fetched_from": used_dest}
    return {"fetched_from": used_dest, "slug": slug,
            "writer": writer, "activated": False}


def cmd_fetch(args):
    if not args.host or not args.slug:
        raise AuthError(6, "fetch needs <host> <slug>")
    return fetch_remote(args.host, require_harness(args.harness), args.slug,
                        activate=bool(args.activate))


def usage_snapshot(record):
    """One measured rate-limit snapshot for a record. Network per call."""
    tokens = record.get("tokens") if isinstance(record.get("tokens"), dict) else record
    access = tokens.get("access_token") or ""
    if not access:
        return {"ok": False, "error": "no access token in record"}
    req = urllib.request.Request(CODEX_USAGE_URL, headers={
        "Authorization": f"Bearer {access}",
        "ChatGPT-Account-Id": tokens.get("account_id") or "",
        "User-Agent": CODEX_USER_AGENT,
        "Accept": "application/json",
    })
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", "replace")[:150]
        return {"ok": False, "error": f"HTTP {exc.code}: {detail}"}
    except (urllib.error.URLError, OSError, ValueError) as exc:
        return {"ok": False, "error": f"{type(exc).__name__}: {exc}"}

    rl = payload.get("rate_limit") or {}

    def window(w):
        if not isinstance(w, dict):
            return None
        return {"used_percent": w.get("used_percent"),
                "window_minutes": round(w.get("limit_window_seconds", 0) / 60),
                "reset_at": w.get("reset_at"),
                "reset_in_minutes": round(w.get("reset_after_seconds", 0) / 60)}

    return {"ok": True,
            "email": payload.get("email"),
            "plan_type": payload.get("plan_type"),
            "limit_reached": bool(rl.get("limit_reached")),
            "allowed": bool(rl.get("allowed")),
            "primary": window(rl.get("primary_window")),
            "secondary": window(rl.get("secondary_window"))}


def cmd_usage(args):
    harness = require_harness(args.harness)
    profiles = stored_profiles(harness)
    targets = []
    if args.slug:
        slug = slug_for(args.slug)
        targets = [p for p in profiles if p["slug"] == slug]
        if not targets:
            raise AuthError(6, f"no stored profile {slug!r} for {harness}")
    else:
        targets = profiles
    out = []
    live_path = HARNESSES[harness]["auth_file"](harness)
    live_ident = identity_of(live_record(harness)) if os.path.exists(live_path) else None
    for p in targets:
        record = read_json(p["path"])
        snap = usage_snapshot(record)
        out.append({"slug": p["slug"], "live": bool(live_ident and p["slug"] == live_ident["slug"]),
                    "access_expired": p["expired"], "usage": snap})
    return {"harness": harness, "accounts": out}


def refresh_grant(record):
    """OAuth refresh grant — the same client_id codex itself uses. ROTATES the
    refresh token: the caller MUST persist the returned record immediately."""
    tokens = record.get("tokens") if isinstance(record.get("tokens"), dict) else record
    refresh_token = tokens.get("refresh_token")
    if not refresh_token:
        raise AuthError(6, "record carries no refresh_token — cannot refresh")
    body = urllib.parse.urlencode({
        "grant_type": "refresh_token", "refresh_token": refresh_token,
        "client_id": CHATGPT_CLIENT_ID,
    }).encode()
    req = urllib.request.Request(f"{CHATGPT_AUTH_BASE}/oauth/token", data=body, headers={
        "Content-Type": "application/x-www-form-urlencoded",
        "User-Agent": CODEX_USER_AGENT,
    })
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            got = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", "replace")[:150]
        raise AuthError(5, f"refresh grant rejected: HTTP {exc.code} {detail}")
    except (urllib.error.URLError, OSError, ValueError) as exc:
        raise AuthError(5, f"refresh grant failed: {exc}")
    if not got.get("access_token"):
        raise AuthError(5, "refresh response missing access_token")
    new = json.loads(json.dumps(record))  # deep copy, verbatim shape preserved
    container = new["tokens"] if "tokens" in new else new
    container["access_token"] = got["access_token"]
    if got.get("refresh_token"):  # rotation: the old one is dead the moment this returns
        container["refresh_token"] = got["refresh_token"]
    if got.get("id_token"):
        container["id_token"] = got["id_token"]
    if "last_refresh" in new:
        new["last_refresh"] = now_rfc3339_nanos()
    return new


def _fleet_host_status(host, script_bytes):
    """One host's status --json, via stdin-executed script (no remote install).
    Returns the parsed payload or an {"error": ...} dict. Self is handled by
    the caller — this always goes over ssh."""
    last = {"error": f"no destination for {host}"}
    for dest in ssh_dest_candidates(host):
        proc = ssh_script(dest, ["status", "--json"], script_bytes)
        if proc is None:
            last = {"error": f"ssh {dest} timed out"}
            continue
        if proc.returncode == 0:
            try:
                parsed = json.loads(proc.stdout.decode("utf-8"))
                remember_ssh_dest(dest)
                return parsed
            except ValueError:
                last = {"error": f"{dest}: unparseable output"}
                continue
        err = proc.stderr.decode("utf-8", "replace").strip()[:120]
        last = {"error": f"ssh {dest}: {err or proc.returncode}"}
    return last


def cmd_fleet(args):
    harness = require_harness(args.harness)
    if args.hosts:
        hosts = [h.strip() for h in args.hosts.split(",") if h.strip()]
        if args.save:
            save_fleet_hosts(hosts)
    else:
        hosts = load_fleet_hosts() or [HOSTNAME]
    script_bytes = open(os.path.abspath(__file__), "rb").read()
    rows = {}
    for host in hosts:
        if host == HOSTNAME:
            rows[host] = cmd_status(type("NS", (), {"harness": harness})())
        else:
            rows[host] = _fleet_host_status(host, script_bytes)
    slugs = []
    for row in rows.values():
        for p in row.get("profiles", []) or []:
            s = p.get("slug")
            if s and s not in slugs:
                slugs.append(s)
    return {"harness": harness, "hosts": hosts, "accounts": slugs, "rows": rows}


def cmd_doctor(args):
    harness = require_harness(args.harness)
    prov = load_provenance(harness)
    findings = []
    hosts_needed = set()
    for slug, entry in prov.items():
        writer = entry.get("writer")
        if not writer or writer in (HOSTNAME, "unknown", "litellm"):
            continue
        hosts_needed.add(writer)
    script_bytes = open(os.path.abspath(__file__), "rb").read()
    remote_prov = {}
    for host in hosts_needed:
        for dest in ssh_dest_candidates(host):
            proc = ssh_script(dest, ["provenance", "--harness", harness, "--json"], script_bytes, timeout=30)
            if proc is not None and proc.returncode == 0:
                try:
                    remote_prov[host] = json.loads(proc.stdout.decode("utf-8"))
                    remember_ssh_dest(dest)
                except ValueError:
                    remote_prov[host] = {"error": "unparseable"}
                break
        else:
            remote_prov[host] = {"error": "unreachable"}

    for slug in sorted(prov):
        entry = prov[slug]
        writer = entry.get("writer")
        if not writer:
            findings.append({"slug": slug, "state": "no-lineage",
                             "note": "no provenance recorded; capture or fetch to stamp it"})
        elif writer == HOSTNAME:
            findings.append({"slug": slug, "state": "writer-here",
                             "note": "this host owns the lineage"})
        elif writer == "litellm":
            findings.append({"slug": slug, "state": "writer-litellm",
                             "note": "lineage rotates inside litellm; re-adopt from its live auth.json, never refresh here"})
        elif writer == "unknown":
            findings.append({"slug": slug, "state": "writer-unknown",
                             "note": "imported without a known writer; treat as dormant"})
        else:
            remote = remote_prov.get(writer) or {}
            if "error" in remote:
                findings.append({"slug": slug, "state": "writer-unreachable", "writer": writer,
                                 "note": f"could not reach {writer}: {remote['error']}"})
            else:
                their_entry = remote.get(slug) or {}
                their_hash = their_entry.get("content_hash")
                my_hash = entry.get("content_hash")
                if their_hash and my_hash:
                    state = "fresh" if their_hash == my_hash else "STALE"
                    note = ("records identical — stamp drift only" if state == "fresh"
                            else f"writer's copy differs — re-fetch from {writer}")
                    findings.append({"slug": slug, "state": state, "writer": writer, "note": note})
                    continue
                theirs = their_entry.get("writer_updated_at") or 0
                mine = entry.get("writer_updated_at") or 0
                if theirs > mine + 1:
                    mins = round((theirs - mine) / 60)
                    findings.append({"slug": slug, "state": "STALE", "writer": writer,
                                     "note": f"writer moved ahead ~{mins}m ago — re-fetch from {writer}"})
                else:
                    findings.append({"slug": slug, "state": "fresh", "writer": writer})
    return {"harness": harness, "host": HOSTNAME, "findings": findings}


def cmd_provenance(args):
    return load_provenance(require_harness(args.harness))


def _litellm_ssh(cmd_argv, stdin_bytes=None, timeout=30):
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", LITELLM_VIA_HOST,
           f"lxc-attach -n {LITELLM_CONTAINER} -- {cmd_argv}"]
    try:
        return subprocess.run(cmd, input=stdin_bytes, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None


def cmd_adopt(args):
    """Dream ACK-447fefa6b4: adopt what the litellm proxy currently holds.
    Its live auth.json is the only place its lineages stay fresh (rotation)."""
    harness = require_harness(args.harness)
    proc = _litellm_ssh(f"cat {LITELLM_TOKEN_DIR}/auth.json")
    if proc is None:
        raise AuthError(5, f"ssh {LITELLM_VIA_HOST} → {LITELLM_CONTAINER} timed out")
    if proc.returncode != 0:
        raise AuthError(5, f"litellm read failed: {proc.stderr.decode('utf-8', 'replace').strip()[:160]}")
    try:
        flat = json.loads(proc.stdout.decode("utf-8"))
    except ValueError:
        raise AuthError(5, "litellm auth.json is not valid JSON")
    ident_preview = identity_of(flat)
    slug = slug_for(args.name) if args.name else ident_preview["slug"]
    if not slug:
        raise AuthError(6, "could not identify the litellm account; pass --name")
    record = convert_flat(flat, harness)
    ident = identity_of(record)
    with fleet_lock():
        atomic_write_json(os.path.join(store_dir(harness), f"{slug}.json"), record)
    stamp_provenance(harness, slug, record=record, writer="litellm", source="adopt",
                     writer_updated_at=time.time())
    return {"adopted": slug, "identity": {k: v for k, v in ident.items() if k != "slug"},
            "from": f"{LITELLM_VIA_HOST}:{LITELLM_CONTAINER}"}


def convert_flat(flat, harness):
    """Flat litellm record → the harness's own nested shape (verbatim codecs)."""
    if not isinstance(flat, dict) or not all(k in flat for k in ("access_token", "refresh_token", "id_token")):
        raise AuthError(6, "not a flat litellm auth record (need access_token, refresh_token, id_token)")
    if "tokens" in flat:
        raise AuthError(6, "record already carries a tokens block — import the flat prototype format, or use `capture`")
    return {
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": None,
        "tokens": {
            "id_token": flat["id_token"],
            "access_token": flat["access_token"],
            "refresh_token": flat["refresh_token"],
            "account_id": flat.get("account_id") or identity_of(flat).get("account_id") or "none",
        },
        "last_refresh": now_rfc3339_nanos(),
    }


def cmd_litellm_switch(args):
    """Push a stored profile INTO litellm (reverse of adopt), then restart its
    stack and verify health — the prototype's restart_litellm_stack, productized.
    ⚠ after this, litellm owns the lineage: refreshes there will orphan the
    fleet's copies (re-adopt to re-sync)."""
    harness = require_harness(args.harness)
    if not args.slug:
        raise AuthError(6, "litellm-switch needs a profile slug")
    slug = slug_for(args.slug)
    store_file = os.path.join(store_dir(harness), f"{slug}.json")
    if not os.path.exists(store_file):
        raise AuthError(6, f"no stored profile {slug!r} for {harness}")
    nested = read_json(store_file)
    tokens = nested.get("tokens") or nested
    flat = {
        "access_token": tokens.get("access_token"),
        "refresh_token": tokens.get("refresh_token"),
        "id_token": tokens.get("id_token"),
        "account_id": tokens.get("account_id"),
        "expires_at": (identity_of(nested).get("expires_at") or 0),
    }
    payload = (json.dumps(flat, indent=2) + "\n").encode()
    writer = _litellm_ssh(f"sh -c 'cat > {LITELLM_TOKEN_DIR}/auth.json.tmp && "
                          f"mv {LITELLM_TOKEN_DIR}/auth.json.tmp {LITELLM_TOKEN_DIR}/auth.json && "
                          f"chmod 600 {LITELLM_TOKEN_DIR}/auth.json'",
                          stdin_bytes=payload, timeout=30)
    if writer is None or writer.returncode != 0:
        err = writer.stderr.decode("utf-8", "replace").strip()[:160] if writer else "timed out"
        raise AuthError(5, f"writing litellm auth.json failed: {err}")
    snap = _litellm_ssh(f"sh -c 'cp {LITELLM_TOKEN_DIR}/auth.json {LITELLM_TOKEN_DIR}/auth_{slug}.json'")
    restart = _litellm_ssh(f"docker restart {LITELLM_CONTAINER_NAME}", timeout=60)
    if restart is None or restart.returncode != 0:
        raise AuthError(5, "litellm auth written but the container restart failed — check docker on "
                           f"{LITELLM_VIA_HOST}")
    healthy = False
    for _ in range(6):
        time.sleep(2)
        probe = _litellm_ssh("python3 -c \"import urllib.request;print(urllib.request.urlopen("
                             "'http://127.0.0.1:4000/health/liveliness',timeout=5).status)\"",
                             timeout=20)
        if probe is not None and probe.returncode == 0 and b"200" in probe.stdout:
            healthy = True
            break
    stamp_provenance(harness, slug, writer="litellm", source="litellm-switch",
                     writer_updated_at=time.time())
    return {"litellm_switched": slug, "healthy": healthy, "note": "litellm owns this lineage now; re-adopt after its refreshes"}


def cmd_refresh(args):
    harness = require_harness(args.harness)
    live_path = HARNESSES[harness]["auth_file"](harness)
    with fleet_lock():
        if args.slug:
            slug = slug_for(args.slug)
            store_file = os.path.join(store_dir(harness), f"{slug}.json")
            if not os.path.exists(store_file):
                raise AuthError(6, f"no stored profile {slug!r} for {harness}")
            writer = get_writer(harness, slug)
            if (writer and writer != HOSTNAME and writer != "unknown"
                    and not args.force):
                raise AuthError(
                    5, f"{slug}'s lineage is owned by {writer} (single-writer law, "
                       f"SKILL.md §3f) — refresh there, or re-fetch from {writer}; "
                       f"--force overrides")
            record = read_json(store_file)
            fresh = refresh_grant(record)
            atomic_write_json(store_file, fresh)
            stamp_provenance(harness, slug, record=fresh, source="refresh",
                             writer_updated_at=time.time())
            out = {"refreshed": slug, "persisted": "store"}
            live_ident = identity_of(live_record(harness)) if os.path.exists(live_path) else None
            if live_ident and live_ident["slug"] == slug:
                atomic_write_json(live_path, fresh)
                out["persisted"] = "store+live"
            return out
        record = live_record(harness, required=True)
        fresh = refresh_grant(record)
        ident = identity_of(fresh)
        out = {"refreshed": ident["slug"] or "live", "persisted": "live"}
        atomic_write_json(live_path, fresh)
        if ident["slug"]:
            atomic_write_json(os.path.join(store_dir(harness), f"{ident['slug']}.json"), fresh)
            stamp_provenance(harness, ident["slug"], record=fresh, source="refresh",
                             writer_updated_at=time.time())
            out["persisted"] = "live+store"
        return out


def render_table(status):
    lines = []
    live = status["live"]
    if live:
        state = "EXPIRED" if live["expired"] else "active"
        lines.append(f"live: {live['email']} ({live['plan']}) account {live['account_id']} — {state}")
    else:
        lines.append("live: (no auth file — run `login` or `switch`)")
    if status["profiles"]:
        lines.append("profiles:")
        for p in status["profiles"]:
            marks = []
            if p["live"]:
                marks.append("←live")
            elif p.get("cooldown_minutes_left"):
                marks.append(f"cooling {p['cooldown_minutes_left']}m")
            if p["expired"]:
                marks.append("EXPIRED")
            suffix = ("  " + " ".join(marks)) if marks else ""
            lines.append(f"  {p['slug']:<40} {p['plan']:<10}{suffix}")
    else:
        lines.append("profiles: (none stored — `capture` the current account or `login` a new one)")
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="ygg-auth.py",
        description="Fleet auth-rotation plane: fast multi-account switching for harness CLIs.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Credentials never leave the host. Output never prints tokens.")
    parser.add_argument("--harness", default="codex", help="target harness (default codex)")
    # Subparsers re-declare --harness with a SUPPRESS default so it may sit on
    # either side of the verb (`--harness claude capture` AND `capture --harness
    # claude`) without the sub-default clobbering the top-level value.
    sub_parent = argparse.ArgumentParser(add_help=False)
    sub_parent.add_argument("--harness", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
    sub = parser.add_subparsers(dest="verb", required=True)

    p = sub.add_parser("status", parents=[sub_parent], help="live identity + stored profiles")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_status)

    p = sub.add_parser("list", parents=[sub_parent], help="stored profiles")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_list)

    p = sub.add_parser("capture", parents=[sub_parent], help="snapshot the live record into the store")
    p.add_argument("--name", help="override the profile slug")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_capture)

    p = sub.add_parser("switch", parents=[sub_parent], help="switch to a stored profile (capture-on-leave)")
    p.add_argument("slug", nargs="?", help="profile slug or account email from `list`")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_switch)

    p = sub.add_parser("rotate", parents=[sub_parent], help="switch to the freshest profile — the rate-limit verb")
    p.add_argument("--fast", action="store_true",
                   help="skip usage measurement; rank by slug order + cooldowns only")
    p.add_argument("--cooldown-minutes", type=int, default=DEFAULT_COOLDOWN_MINUTES,
                   help=f"put the vacated account on cooldown (default {DEFAULT_COOLDOWN_MINUTES}; 0 disables)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_rotate)

    p = sub.add_parser("login", parents=[sub_parent], help="device-code OAuth into a NEW account (codex)")
    p.add_argument("--name", help="override the profile slug")
    p.add_argument("--no-notify", action="store_true",
                   help="do not post the device-code card to infra/auth")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_login)

    p = sub.add_parser("fleet", parents=[sub_parent],
                       help="accounts × hosts matrix over ssh (provenance-aware)")
    p.add_argument("--hosts", help="comma-separated host list (persisted with --save)")
    p.add_argument("--save", action="store_true", help="remember this host list as the default")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_fleet)

    p = sub.add_parser("doctor", parents=[sub_parent],
                       help="flag stale replicas against each lineage's writer")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_doctor)

    p = sub.add_parser("provenance", parents=[sub_parent],
                       help="the raw lineage sidecar (writer, stamps)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_provenance)

    p = sub.add_parser("adopt", parents=[sub_parent],
                       help="adopt the account the litellm proxy currently holds")
    p.add_argument("--name", help="override the profile slug")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_adopt)

    p = sub.add_parser("litellm-switch", parents=[sub_parent],
                       help="push a stored profile into litellm, restart, verify health")
    p.add_argument("slug", nargs="?", help="profile slug or account email")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_litellm_switch)

    p = sub.add_parser("import", parents=[sub_parent], help="convert a flat litellm-prototype record and store it")
    p.add_argument("file", nargs="?", help="path to the foreign auth record")
    p.add_argument("--name", help="override the profile slug")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_import)

    p = sub.add_parser("fetch", parents=[sub_parent], help="pull a profile from another fleet host's store")
    p.add_argument("host", nargs="?", help="ssh alias of the remote host")
    p.add_argument("slug", nargs="?", help="profile slug or account email")
    p.add_argument("--activate", action="store_true", help="also switch the live account to it")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_fetch)

    p = sub.add_parser("usage", parents=[sub_parent], help="measured rate-limit state per profile")
    p.add_argument("--slug", help="one profile instead of all")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_usage)

    p = sub.add_parser("refresh", parents=[sub_parent], help="renew tokens via the OAuth refresh grant")
    p.add_argument("slug", nargs="?", help="stored profile (default: the live account)")
    p.add_argument("--force", action="store_true",
                   help="override the single-writer refusal (rotates the token out from the recorded writer)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_refresh)

    p = sub.add_parser("harnesses", parents=[sub_parent], help="registry: what each harness supports")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_harnesses)

    args = parser.parse_args(argv)
    os.umask(0o077)  # consult 2026-09-20: nothing this tool writes is ever group/other-readable
    args.json = getattr(args, "json", False)
    try:
        out = args.func(args)
    except AuthError as exc:
        print(f"⛔ {exc}", file=sys.stderr)
        return exc.code
    if args.json:
        print(json.dumps(out, indent=2))
    else:
        if args.verb == "status":
            print(render_table(out))
        elif args.verb == "switch":
            idt = out["identity"]
            print(f"✅ switched to {idt['email']} ({idt['plan']})")
            if out.get("captured_before_leaving"):
                print(f"   previous account captured: {out['captured_before_leaving']['slug']}")
        elif args.verb == "rotate":
            idt = out["identity"]
            print(f"🔄 rotated to {idt['email']} ({idt['plan']})")
            if out.get("captured_before_leaving"):
                print(f"   previous account captured: {out['captured_before_leaving']['slug']}")
            vac = out.get("vacated_to_cooldown")
            if vac:
                mins = max(1, round((vac["until"] - time.time()) / 60))
                print(f"   {vac['slug']} cooling for ~{mins}m (skip on next rotates)")
        elif args.verb == "capture":
            print(f"💾 captured {out['captured']} → {out['path']}")
        elif args.verb == "login":
            idt = out["identity"]
            print(f"🎉 logged in as {idt['email']} ({idt['plan']}); profile {out['logged_in']} stored + live")
        elif args.verb == "import":
            idt = out["identity"]
            print(f"📥 imported {idt['email']} ({idt['plan']}) as {out['imported']} — stored, not live (use `switch`)")
        elif args.verb == "fetch":
            if out.get("activated"):
                idt = out["identity"]
                print(f"📡 fetched {out['switched_to']} from {out['fetched_from']} and activated: {idt['email']} ({idt['plan']})")
            else:
                print(f"📡 fetched {out['slug']} from {out['fetched_from']} — stored, not live (use `switch` or `--activate`)")
        elif args.verb == "usage":
            for acct in out["accounts"]:
                u = acct["usage"]
                name = acct["slug"] + (" ←live" if acct["live"] else "")
                if not u.get("ok"):
                    oneline = " ".join(u.get("error", "unknown error").split())
                    print(f"  {name:<44} ⛔ {oneline[:90]}")
                    continue
                pri, sec = u["primary"], u["secondary"]
                def w(x):
                    if not x:
                        return "—"
                    length = f"{x['window_minutes']}m-window"
                    return f"{length}: {x['used_percent']}% used, resets ~{x['reset_in_minutes']}m"
                flag = "⛔ LIMIT" if u["limit_reached"] else "ok"
                print(f"  {name:<44} {u['plan_type']:<6} {flag:<8} {w(pri)} | {w(sec)}")
        elif args.verb == "refresh":
            print(f"🔄 refreshed {out['refreshed']}; persisted to {out['persisted']}")
        elif args.verb == "fleet":
            for host, row in out["rows"].items():
                if row.get("error"):
                    print(f"  {host:<12} ⛔ {row['error'][:80]}")
                    continue
                live = (row.get("live") or {}).get("slug") or "(none)"
                marks = []
                for p in row.get("profiles", []) or []:
                    s = p.get("slug") or "?"
                    tag = "L" if p.get("live") else "·"
                    marks.append(f"{s}:{tag}")
                print(f"  {host:<12} live={live:<34} stored: {', '.join(marks) or '—'}")
            print(f"  (L = live on that host; accounts known: {len(out['accounts'])})")
        elif args.verb == "doctor":
            bad = 0
            for f in out["findings"]:
                mark = "⛔" if f["state"] in ("STALE",) else "·"
                if f["state"] in ("STALE", "writer-unreachable"):
                    bad += 1
                print(f"  {mark} {f['slug']:<38} {f['state']:<20} {f.get('note', '')[:70]}")
            if not out["findings"]:
                print("  no provenance recorded at all — capture or fetch to stamp lineages")
        elif args.verb == "adopt":
            idt = out["identity"]
            print(f"🛰  adopted {idt['email']} ({idt['plan']}) as {out['adopted']} from {out['from']} — writer is litellm; re-adopt to re-sync")
        elif args.verb == "litellm-switch":
            health = "healthy" if out["healthy"] else "⚠ health check did not clear — check the container"
            print(f"🛰  litellm now runs {out['litellm_switched']} ({health}); lineage lives in litellm — re-adopt after its refreshes")
        else:
            print(json.dumps(out, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
