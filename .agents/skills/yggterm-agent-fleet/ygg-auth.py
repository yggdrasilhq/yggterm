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
  moves the active cache to the next eligible local profile. Existing runs
  may need restart/resume. `fallback` accepts only typed consumer errors under
  an explicit host-local policy; it never replays a request. Login requires
  the owner's browser; usage and refresh also touch provider endpoints.

LAWS THIS TOOL LIVES UNDER:
  - Harness Isolation Law (SKILL.md §5): private harness stores are private.
    ygg-auth is the ONE sanctioned writer, whole-file atomic swap only —
    never edit a harness auth file piecemeal.
  - Memory-sync law: credentials never travel between machines. The store is
    per-host, never in ~/.yggterm/memory, never posted, never logged.
  - Redaction: no verb prints a token value. Only email, plan, account id,
    expiry. Errors never echo file contents.

VERBS:
  policy status [--harness H] [--json] validated host-local policy
  fallback --error-code CODE --event-id ID --expected-active-hash HASH
                                   one policy-authorized local activation
  status  [--harness H|all] [--json] live binding + stored profile inventory
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
  give    <host> <slug> [--harness H] [--dest P]
                                   export a stored profile VERBATIM to a consumer
                                   host (llmproxy et al pick it up in-container)
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
  exec    [--harness H] [--preflight-only] [--measure] [--json]
          [--typed-failure CODE --event-id ID --observed-active-hash HASH]
          -- <native argv...>
                                   prove the bound grant, then replace this
                                   process with the native argv (signals, args
                                   and the exit code are the native CLI's own;
                                   no restart, no replay). --preflight-only is
                                   a bounded read that prints the activation
                                   receipt and spawns nothing. An expired but
                                   refreshable grant takes ONE canonical
                                   refresh; a typed adapter-observed failure
                                   takes ONE policy-authorized fallback.
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
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import math
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

STORE_ROOT = os.environ.get("YGG_AUTH_HOME") or os.path.expanduser("~/.yggterm/auth")
LOCK_FILE = os.path.join(STORE_ROOT, ".lock")
DEFAULT_COOLDOWN_MINUTES = 30
def _self_name():
    """This host's name in the fleet roster. The kernel hostname can differ from the
    roster alias (one host reports a vendor name), and provenance writer stamps must
    use the alias or every peer comparison silently misses this host. An explicit
    `.host-alias` file wins; the kernel name is the fallback."""
    try:
        with open(os.path.join(STORE_ROOT, ".host-alias")) as f:
            alias = f.read().strip()
            if alias:
                return alias
    except OSError:
        pass
    return socket.gethostname()


HOSTNAME = _self_name()
FLEET_HOSTS_FILE = os.path.join(STORE_ROOT, ".fleet-hosts")
SSH_DESTS_FILE = os.path.join(STORE_ROOT, ".ssh-dests.json")
PROVENANCE_FILE = os.path.join(STORE_ROOT, ".provenance")
# llmproxy era (2026-09-30): the `litellm` lxc guest is decommissioned; the
# LITELLM_* names survive as the deprecated alias for the llmproxy guest.
LLMPROXY_SSH_HOST = "llmproxy"   # direct ssh target (wired where the owner uses it)
LITELLM_VIA_HOST = "manin"       # fallback jump: lxc-attach into the llmproxy guest
LITELLM_CONTAINER = "llmproxy"
LITELLM_TOKEN_DIR = "/root/chatgpt_tokens"
LLMPROXY_SERVICE = "llmproxy.service"
ASSIGNMENTS_FILE = os.path.join(STORE_ROOT, ".assignments.json")

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
    if harness == "zcode":
        # 2.0.0 (owner 2026-10-25): the portable zcode auth is the PLAINTEXT
        # provider_config.json — the enc:v1 credentials.json values are
        # safeStorage-bound to the host keyring and cannot decrypt elsewhere
        # (the retired zcode-tui startup sync shipped exactly this file).
        override = os.environ.get("ZCODE_AUTH_FILE")
        return override or os.path.join(os.path.expanduser("~"), ".zcode", "v2", "provider_config.json")
    if harness == "autoclaw":
        override = os.environ.get("AUTOCLAW_PLAN_FILE")
        return override or os.path.join(os.path.expanduser("~"), ".config", "zcode-tui", "autoclaw-plan.json")
    raise KeyError(harness)


HARNESSES = {
    "codex": {
        "auth_file": harness_auth_file,
        "login": "device-code (auth.openai.com; the owner completes the browser step)",
        "live_note": "switch/rotate/login tear down the codex app-server daemon — it caches auth "
                     "at startup and every TUI delegates auth to it; a TUI that read auth itself "
                     "keeps its loaded tokens until restarted",
    },
    "claude": {
        "auth_file": harness_auth_file,
        "login": None,
        "live_note": "swap supported; login flow not implemented (add when the need lands)",
    },
    # Plan harnesses (zcode-tui 2.0.0): the official apps are the SSOTs and
    # login is a per-host, owner-performed step inside them; ygg-auth
    # replicates the resulting portable auth records with lineage.
    "zcode": {
        "auth_file": harness_auth_file,
        "login": None,
        "live_note": "plaintext provider_config.json (the zai-api key that authenticates turns); "
                     "credentials.json is host-keyring-bound and is NOT replicated — per-host "
                     "desktop sign-in mints it; the coding-plan api key is picked up by consumers (llmproxy's internal seeder)",
    },
    "autoclaw": {
        "auth_file": harness_auth_file,
        "login": None,
        "live_note": "the mined AutoClaw plan (identity+credential, host-agnostic — the "
                     "agent-refresh pair answers from any machine); mined by "
                     "`zcode-tui autoclaw setup` where the official app is signed in",
    },
}


class AuthError(Exception):
    def __init__(self, code, message, details=None):
        super().__init__(message)
        self.code = code
        self.details = details or {}


# Provider identity for receipts — public names only, never account data.
PROVIDERS = {"codex": "openai", "claude": "anthropic", "zcode": "zai", "autoclaw": "zai"}


def provider_of(harness):
    return PROVIDERS.get(harness, "unknown")


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


def identity_of(record, harness="codex"):
    """Claims only — this dict is what every verb is allowed to print."""
    if harness in ("zcode", "autoclaw"):
        return _plan_identity(record, harness)
    if harness == "claude":
        oauth = record.get("claudeAiOauth") or {}
        exp = oauth.get("expiresAt")
        return {"slug": None, "email": "unknown", "plan": "unknown", "account_id": "none",
                "expires_at": exp / 1000 if isinstance(exp, (int, float)) else None,
                "expired": isinstance(exp, (int, float)) and exp < time.time() * 1000}
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


def _zcode_account_uuid(record):
    """The zai account UUID, when the host also carries the (host-bound)
    credentials.json — its account-provider key names carry it. Identity
    hint only; the replicated record itself is provider_config.json."""
    cred = os.path.join(os.path.dirname(HARNESSES["zcode"]["auth_file"]("zcode")), "credentials.json")
    try:
        doc = json.loads(open(cred, encoding="utf-8").read())
    except (OSError, ValueError):
        return None
    for k in doc:
        if k.startswith("account-provider:coding-plan:"):
            parts = k.split(":")
            if len(parts) >= 6:
                return parts[5]
    return None


def _plan_identity(record, harness):
    """Identity for the plan-harness records — claims/fingerprints only."""
    if harness == "zcode":
        # provider_config.json: the zai-api key is the identity-bearing part;
        # never print it. Account UUID when the sibling credentials exist.
        key = None
        try:
            for rule in (record.get("config", {}).get("providerConfigRules", {}).get("providerRules") or []):
                access = rule.get("config", {}).get("access") or {}
                if access.get("type") == "api-key" and access.get("apiKey"):
                    key = access["apiKey"]
                    break
        except AttributeError:
            key = None
        uuid = _zcode_account_uuid(record)
        return {
            "slug": f"zai-{uuid[:8]}" if uuid else None,
            "email": "unknown",
            "plan": "zai-coding-plan",
            "account_id": uuid or "none",
            "key_fingerprint": hashlib.sha256(key.encode()).hexdigest()[:12] if key else None,
            "expires_at": None,
            "expired": False,
        }
    # autoclaw: the mined plan; the agentaccess JWT carries user_id + jti
    # (the account email) + exp — measured live 2026-10-25.
    auth = record.get("auth") or {}
    claims = decode_jwt_claims(auth.get("accessToken") or "")
    email = claims.get("jti") if "@" in str(claims.get("jti") or "") else None
    exp = claims.get("exp") or 0
    try:
        expired = bool(exp) and time.time() > float(exp)
    except (TypeError, ValueError):
        expired = False
    return {
        "slug": slug_for(email) if email else (f"autoclaw-{claims.get('user_id')}" if claims.get("user_id") else None),
        "email": email or "unknown",
        "plan": "autoclaw-credits",
        "account_id": str(claims.get("user_id") or "none"),
        "expires_at": exp or None,
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
    """Missing sidecar = no cooldowns. A PRESENT but corrupt sidecar fails
    closed (fresh-P4-cooldown): reading corruption as an empty table would
    silently erase a live exclusion and re-activate a cooling account."""
    path = cooldown_path(harness)
    if not os.path.exists(path):
        return {}
    try:
        doc = read_json(path)
    except (OSError, ValueError) as exc:
        raise AuthError(5, f"cooldown metadata at {path} is unreadable; refusing "
                           "to treat corruption as 'no cooldown'",
                        {"reason": "invalid_metadata"}) from exc
    if not isinstance(doc, dict) or any(
            not isinstance(slug, str) or not slug
            or isinstance(until, bool) or not isinstance(until, (int, float))
            or not math.isfinite(until)
            for slug, until in doc.items()):
        raise AuthError(5, f"cooldown metadata at {path} is malformed; refusing "
                           "to treat corruption as 'no cooldown'",
                        {"reason": "invalid_metadata"})
    return doc


def save_cooldowns(harness, data):
    atomic_write_json(cooldown_path(harness), data)


def mark_cooldown(harness, slug, minutes):
    if minutes <= 0:
        return None
    until = time.time() + minutes * 60
    data = load_cooldowns(harness)
    until = max(until, data.get(slug, 0))
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
    """Missing sidecar = no lineage. A PRESENT but corrupt sidecar fails
    closed with a schema-validated read (fresh-P4-lineage): reading corruption
    as an empty table would turn a held foreign-writer grant refreshable."""
    path = provenance_path(harness)
    if not os.path.exists(path):
        return {}
    try:
        doc = read_json(path)
    except (OSError, ValueError) as exc:
        raise AuthError(5, f"lineage metadata at {path} is unreadable; refusing "
                           "to treat corruption as 'no writer'",
                        {"reason": "invalid_metadata"}) from exc
    if not isinstance(doc, dict) or any(
            not isinstance(slug, str) or not slug or not isinstance(entry, dict)
            for slug, entry in doc.items()):
        raise AuthError(5, f"lineage metadata at {path} is malformed; refusing "
                           "to treat corruption as 'no writer'",
                        {"reason": "invalid_metadata"})
    for entry in doc.values():
        writer = entry.get("writer")
        if (not isinstance(writer, str) or not writer.strip()
                or writer != writer.strip() or writer.lower() == "unknown"
                or any(ord(c) < 32 for c in writer)):
            raise AuthError(5, "lineage metadata has invalid writer evidence",
                            {"reason": "invalid_metadata"})
    return doc


def save_provenance(harness, prov):
    atomic_write_json(provenance_path(harness), prov)


def record_hash(record):
    return hashlib.sha256(json.dumps(record, sort_keys=True).encode()).hexdigest()[:16]


def stamp_provenance(harness, slug, record=None, **kw):
    prov = load_provenance(harness)
    entry = prov.get(slug) or {}
    entry.update(kw)
    entry.setdefault("writer", HOSTNAME)
    if record is not None:
        entry["content_hash"] = record_hash(record)
    entry["updated_at"] = time.time()
    prov[slug] = entry
    save_provenance(harness, prov)
    return entry


def get_writer(harness, slug):
    return (load_provenance(harness).get(slug) or {}).get("writer")


# Self-learning ssh destinations — dream ACK-62a8b4abe4. Hosts disagree about
# who an alias is (one host: root, another: pi); remember per host which
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
    if not lock_held:
        with fleet_lock():
            return capture(harness, name=name, lock_held=True)
    record = live_record(harness, required=True)
    ident = identity_of(record, harness)
    if not ident["slug"] and not name:
        raise AuthError(3, "live record carries no usable identity; pass --name to store it anyway")
    slug = slug_for(name or ident["slug"])
    path = os.path.join(store_dir(harness), f"{slug}.json")
    return _capture_locked(harness, record, slug, path, ident)


def _native_epoch(record):
    """Native refresh timestamp orders records; expiry/mtime do not prove lineage."""
    value = record.get("last_refresh")
    if type(value) is not str:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return parsed.timestamp() if parsed.tzinfo is not None else None
    except (ValueError, OverflowError):
        return None


def _same_codex_identity(left, right):
    a, b = identity_of(left), identity_of(right)
    return bool(a["slug"] and a["slug"] == b["slug"] and
                a["account_id"] != "none" and a["account_id"] == b["account_id"])


def reconcile_native_codex():
    """Capture a native login/refresh under the vault fence, without activation.

    A newer native refresh may establish a local writer. Ambiguous/downgrade
    evidence holds; callers cannot use a later filesystem mtime as authority.
    """
    with fleet_lock():
        for _ in range(3):
            path = HARNESSES['codex']['auth_file']('codex')
            before = _live_fingerprint(path)
            native = live_record('codex', required=True)
            ident = identity_of(native)
            if not ident['slug'] or ident['account_id'] == 'none' or not usable_record(native, 'codex'):
                raise AuthError(5, 'native identity is not eligible for reconciliation')
            target = os.path.join(store_dir('codex'), ident['slug'] + '.json')
            if os.path.exists(target):
                stored = read_json(target)
                if record_hash(native) == record_hash(stored):
                    return {'reconciled': ident['slug'], 'changed': False, 'native_unchanged': True}
                if not _same_codex_identity(native, stored):
                    raise AuthError(5, 'native and stored account identity differ; retaining both')
                a, b = _native_epoch(native), _native_epoch(stored)
                if a is None or b is None or a <= b:
                    raise AuthError(5, 'native freshness is ambiguous or older; refusing vault downgrade',
                                    {'reason': 'native_freshness_unresolved'})
            if _live_fingerprint(path) != before:
                continue
            _capture_locked('codex', native, ident['slug'], target, ident)
            return {'reconciled': ident['slug'], 'changed': True, 'native_unchanged': True}
        raise AuthError(5, 'native login kept changing; reconciliation held')


def cmd_reconcile(args):
    if require_harness(args.harness) != 'codex':
        raise AuthError(6, 'native reconciliation currently supports codex only')
    result = reconcile_native_codex()
    if getattr(args, 'fleet', False):
        # Each enrolled peer PULLS this writer's stored portable profile.
        # No native activation, no keyrings, no remote refresh, no broad sync.
        script = open(os.path.abspath(__file__), 'rb').read()
        peers = {}
        hosts = load_fleet_hosts()
        if not hosts:
            raise AuthError(5, 'fleet pull requested without enrolled hosts; local reconciliation retained')
        expected = record_hash(read_json(os.path.join(store_dir('codex'), result['reconciled']+'.json')))
        for host in hosts:
            if host == HOSTNAME:
                continue
            proc = None
            for destination in ssh_dest_candidates(host):
                proc = ssh_script(destination, ['fetch', HOSTNAME, result['reconciled'], '--harness', 'codex', '--json'], script)
                if proc is not None and proc.returncode == 0:
                    remember_ssh_dest(destination)
                    break
            try:
                reply = json.loads(proc.stdout) if proc is not None and proc.returncode == 0 else {}
            except (ValueError, AttributeError):
                reply = {}
            verified = (isinstance(reply,dict) and reply.get('slug') == result['reconciled']
                        and reply.get('activated') is False and reply.get('stored_record_hash') == expected)
            peers[host] = {'stored': verified,
                           'active_account_changed': False}
        result['peers'] = peers
        if any(not v['stored'] for v in peers.values()):
            raise AuthError(5, 'native login reconciled locally; some fleet pulls held', result)
    return result


def _capture_locked(harness, record, slug, path, ident):
    atomic_write_json(path, record)
    stamp_provenance(harness, slug, record=record, writer=HOSTNAME, source="capture",
                     writer_updated_at=time.time())
    return {**ident, "slug": slug, "path": path}


def usable_record(record, harness):
    if not isinstance(record, dict):
        return False
    if harness == "codex":
        tokens = record.get("tokens") if isinstance(record.get("tokens"), dict) else record
        return bool(tokens.get("access_token") or record.get("OPENAI_API_KEY"))
    if harness == "claude":
        return isinstance(record.get("claudeAiOauth"), dict) and bool(record["claudeAiOauth"].get("accessToken"))
    if harness == "autoclaw":
        return isinstance(record.get("auth"), dict) and bool(record["auth"].get("accessToken"))
    try:
        rules = record["config"]["providerConfigRules"]["providerRules"]
        return any(rule["config"].get("access", {}).get("type") == "api-key"
                   and bool(rule["config"]["access"].get("apiKey")) for rule in rules)
    except (KeyError, TypeError, AttributeError):
        return False


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
            profiles.append({"slug": fname[:-5], "path": path, "filename": fname,
                             "expired": False, "unreadable": True})
            continue
        if not isinstance(record, dict):
            profiles.append({"slug": fname[:-5], "path": path, "filename": fname,
                             "expired": False, "unreadable": True})
            continue
        try:
            ident = identity_of(record, harness)
        except (TypeError, ValueError, AttributeError):
            ident = {"expired": False}
        ident.update({"slug": fname[:-5], "path": path, "filename": fname,
                      "invalid_record": not usable_record(record, harness)})
        profiles.append(ident)
    return profiles


def _live_fingerprint(path):
    st = os.stat(path)
    return (st.st_mtime_ns, st.st_size)


def _proc_env(pid):
    try:
        with open(f"/proc/{pid}/environ", "rb") as fh:
            raw = fh.read().split(b"\0")
    except (OSError, ValueError):
        return None
    env = {}
    for chunk in raw:
        key, sep, value = chunk.partition(b"=")
        if sep:
            env[key.decode("utf-8", "replace")] = value.decode("utf-8", "replace")
    return env


def _pid_starttime(pid):
    """The /proc stat starttime field, or None when the pid is gone/reused —
    the identity pin that keeps a recycled pid from reading as a survivor."""
    try:
        with open(f"/proc/{pid}/stat", "r") as fh:
            return fh.read().rsplit(")", 1)[1].split()[19]
    except (OSError, IndexError):
        return None


def _pid_gone(pid):
    try:
        with open(f"/proc/{pid}/stat", "r") as fh:
            return fh.read().rsplit(")", 1)[1].split()[0] == "Z"
    except (OSError, IndexError):
        return True


def codex_daemon_pids(home):
    """PIDs of the codex app-server family bound to `home`.

    codex runs a managed `codex app-server` daemon per CODEX_HOME and every
    TUI delegates auth to it; the daemon reads auth.json ONCE at startup, so
    a swapped file never reaches a running daemon (measured on dev,
    2026-10-28: switch reported success while the TUI kept the old account).

    Matching is structural, by the process's OWN environment — never by
    substring:
      • the home codex would resolve FOR IT must equal `home`: CODEX_HOME
        when set, else $HOME/.codex from that process's HOME (never this
        process's expanduser: a sandboxed switch has a different HOME, and
        equating the two once matched — and killed — an unrelated live
        daemon, caught by the teardown screens); a process with neither
        CODEX_HOME nor HOME is left alone;
      • argv[0] must BE a codex binary and argv[1] the daemon SUBCOMMAND:
        basename `codex` + argv[1] `app-server`, or a `codex-code-mode-host`
        binary — the ynpm node wrapper, other tools, and any resume PROMPT
        (even the single word "app-server" at a later position) never
        qualify, because a prompt can never sit in the subcommand slot.

    TUI processes (`codex resume …`) therefore never match, and zombies are
    skipped: a reaped-late daemon still exposes cmdline/environ but can
    serve no auth. Without /proc (non-Linux), warns and returns [] — the
    caller proceeds, telling the user to restart codex by hand.
    """
    try:
        entries = os.listdir("/proc")
    except OSError:
        print("⚠️  no /proc here — cannot check for running codex daemons; "
              "if codex is running, restart it to pick up the new auth", file=sys.stderr)
        return []
    target = os.path.realpath(home)
    pids = []
    for entry in entries:
        if not entry.isdigit():
            continue
        pid = int(entry)
        if pid == os.getpid():
            continue
        env = _proc_env(pid)
        if env is None:
            continue
        if _pid_gone(pid):
            continue
        env_home = env.get("CODEX_HOME")
        if not env_home:
            daemon_home_env = env.get("HOME")
            if not daemon_home_env:
                continue  # cannot resolve the daemon's own home — leave it alone
            env_home = os.path.join(daemon_home_env, ".codex")
        if os.path.realpath(env_home) != target:
            continue
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as fh:
                argv = [tok.decode("utf-8", "replace") for tok in fh.read().split(b"\0") if tok]
        except OSError:
            continue
        if not argv:
            continue
        exe = os.path.basename(argv[0])
        # subcommand shape: `codex app-server …` or a codex-code-mode-host
        # binary. argv[1] is the SUBCOMMAND slot — a resume PROMPT (even the
        # single word "app-server") can never sit there, so prompt text can
        # never summon the kill list onto the owner's live session.
        is_family = exe.startswith("codex-code-mode-host") or (
            exe == "codex" and len(argv) > 1 and argv[1] == "app-server")
        if not is_family:
            continue
        pids.append(pid)
    return sorted(pids)


def stop_codex_daemons(home, grace_seconds=3.0):
    """Pull the codex app-server family for `home` down: SIGTERM, wait for
    exit, SIGKILL stragglers, wait again. Returns only the pids CONFIRMED
    stopped (death checked against the pre-kill starttime, so a recycled
    pid never reads as a survivor); a pid that outlives SIGKILL is reported
    and omitted — never counted as stopped."""
    pids = codex_daemon_pids(home)
    if not pids:
        return []
    starts = {pid: _pid_starttime(pid) for pid in pids}

    def dead(pid):
        if _pid_gone(pid):
            return True  # zombie: the parent has not reaped it yet, but it can serve no auth
        now = _pid_starttime(pid)
        return now is None or now != starts.get(pid)

    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
    deadline = time.time() + grace_seconds
    while time.time() < deadline and not all(dead(pid) for pid in pids):
        time.sleep(0.1)
    for pid in pids:
        if not dead(pid):
            try:
                os.kill(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
    deadline = time.time() + 1.0
    while time.time() < deadline and not all(dead(pid) for pid in pids):
        time.sleep(0.1)
    stopped = [pid for pid in pids if dead(pid)]
    for pid in pids:
        if pid not in stopped:
            print(f"⚠️  codex app-server pid {pid} did not stop — proceeding; restart it manually",
                  file=sys.stderr)
    return stopped


def _create_native_if_absent(path, record):
    """Publish complete native bytes without overwriting a concurrent login."""
    directory = os.path.dirname(path)
    os.makedirs(directory, mode=0o700, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix='.auth-create-', dir=directory)
    try:
        with os.fdopen(descriptor, 'w') as output:
            os.fchmod(output.fileno(), 0o600)
            json.dump(record, output, indent=2)
            output.flush();os.fsync(output.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError:
            raise AuthError(5, 'native login appeared before publication; retained') from None
        folder = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:os.fsync(folder)
        finally:os.close(folder)
    finally:
        os.unlink(temporary)


def _exchange_native_paths(left, right):
    """Linux exchange preserves the exact displaced inode. No replace fallback."""
    import ctypes
    if not sys.platform.startswith('linux'):
        raise AuthError(5, 'native exchange unsupported on this platform; activation held')
    library = ctypes.CDLL(None, use_errno=True)
    exchange = getattr(library, 'renameat2', None)
    if exchange is None:
        raise AuthError(5, 'native exchange unavailable; activation held')
    exchange.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    exchange.restype = ctypes.c_int
    if exchange(-100, os.fsencode(left), -100, os.fsencode(right), 2) != 0:
        raise AuthError(5, 'native exchange failed; recovery slots retained', {'errno': ctypes.get_errno()})
    descriptor = os.open(os.path.dirname(right), os.O_RDONLY | os.O_DIRECTORY)
    try:os.fsync(descriptor)
    finally:os.close(descriptor)


def _publish_native_preserving_displaced(path, record, expected_record):
    """Retain every displaced grant; concurrent native login wins or holds.

    Atomic exchange is the publication linearization point. A concurrent
    native writer need not honor our flock. Recovery slots remain private
    evidence, including when storage or restoration cannot be reconciled.
    """
    descriptor, slot = tempfile.mkstemp(prefix='.auth-displaced-', dir=os.path.dirname(path))
    with os.fdopen(descriptor, 'w') as output:
        os.fchmod(output.fileno(), 0o600)
        json.dump(record, output, indent=2);output.flush();os.fsync(output.fileno())
    _exchange_native_paths(slot, path)
    displaced = read_json(slot)
    # Store the displaced record before any possible restoration exchange.
    def retain(value):
        atomic_write_json(os.path.join(store_dir('codex'), '.recovery', record_hash(value)+'.json'), value)
    retain(displaced)
    if record_hash(displaced) == record_hash(expected_record):
        return  # The exact expected record was replaced; slot remains pinned.
    ours = record_hash(record)
    for _ in range(3):
        current = read_json(path)
        if record_hash(current) != ours:
            # A later native writer already won. Preserve it in place.
            retain(current)
            raise AuthError(5, 'concurrent native login retained; activation held')
        _exchange_native_paths(slot, path)
        previous = read_json(slot)
        retain(previous)
        if record_hash(previous) == ours:
            raise AuthError(5, 'concurrent native login restored; activation held')
        # Another login committed between our read and restoration. It is
        # now in slot, and the record just restored is the one we own.
        ours = record_hash(displaced)
        displaced = previous
    raise AuthError(5, 'concurrent native publication unresolved; all displaced grants retained')


def switch(harness, slug, lock_held=False, expected_active_hash=None, expected_absent=False):
    """capture-on-leave, then byte-faithful atomic swap to the named profile.

    The live file is re-read if the harness rewrote it mid-swap: OAuth refresh
    rotates refresh tokens, so capturing a stale record would store (and later
    restore) a revoked token — consult 2026-09-20. The replace happens only
    when the file is fingerprint-unchanged from the read; three attempts, then
    fail loudly rather than swap silently stale.
    """
    if not lock_held:
        with fleet_lock():
            return switch(harness, slug, lock_held=True, expected_active_hash=expected_active_hash,
                          expected_absent=expected_absent)
    slug = slug_for(slug)  # accept the raw email; store/lookup use one normalization
    target = os.path.join(store_dir(harness), f"{slug}.json")
    if not os.path.exists(target):
        known = ", ".join(p["slug"] or "?" for p in stored_profiles(harness)) or "(none stored)"
        raise AuthError(6, f"no stored profile {slug!r} for {harness}; stored: {known}")
    live_path = HARNESSES[harness]["auth_file"](harness)
    target_record = read_json(target)
    current = live_record(harness)
    entry_hash = record_hash(current) if current is not None else None
    expected_absent = expected_absent or current is None
    if expected_absent and current is not None:
        raise AuthError(5, 'native login appeared after expected absence; retained')
    if expected_active_hash is not None and (current is None or record_hash(current) != expected_active_hash):
        raise AuthError(5, 'active auth changed during fallback; refusing replacement')
    # Re-select the target AFTER capture. Previously same-account activation
    # captured the fresh native login then overwrote it from the stale local
    # variable read above. A same-account no-op must never roll auth backward.
    if harness == 'codex' and current is not None and _same_codex_identity(current, target_record):
        if record_hash(current) != record_hash(target_record):
            a, b = _native_epoch(current), _native_epoch(target_record)
            if a is None or b is None or a <= b:
                raise AuthError(5, 'same-account freshness unresolved; retaining native login')
            _capture_locked(harness, current, slug, target, identity_of(current))
        return {'switched_to': slug, 'identity': identity_of(current),
                'captured_before_leaving': None, 'daemons_stopped': [],
                'activation': 'already_active_native_preserved', 'native_health': 'unknown'}
    # (identity calls below are harness-scoped)

    # The app-server daemon caches auth at startup and every TUI delegates
    # auth to it — swapping under a live daemon leaves every session on the
    # OLD account while status already reports the new one (dev 2026-10-28,
    # dream ACK-59bf86a578). Pull it down FIRST so nothing writes or serves
    # the old tokens across the swap; sweep again after, for a daemon a
    # client respawned mid-window (it may have read the previous file).
    daemon_home = os.path.dirname(live_path)
    daemons_stopped = []
    if harness == "codex":
        daemons_stopped = stop_codex_daemons(daemon_home)
        if daemons_stopped:
            print(f"⬇  stopped codex app-server daemon(s) {daemons_stopped} — they cache auth "
                  f"at startup and would keep serving the previous account", file=sys.stderr)

    captured = None
    outgoing_record_hash = None
    after_stop = live_record(harness)
    if (record_hash(after_stop) if after_stop is not None else None) != entry_hash:
        raise AuthError(5, 'native login changed during initial teardown; activation held')
    if os.path.exists(live_path):
        swapped = False
        for _ in range(3):
            before = _live_fingerprint(live_path)
            current = live_record(harness, required=True)
            if expected_absent:
                raise AuthError(5, 'native login appeared during activation; retained')
            outgoing_record_hash = record_hash(current)
            if expected_active_hash is not None and record_hash(current) != expected_active_hash:
                raise AuthError(5, "active auth changed during fallback; refusing replacement")
            current_ident = identity_of(current, harness)
            outgoing = active_slug(harness, current) or ("unbound-" + record_hash(current)[:12])
            if record_hash(current) != record_hash(target_record):
                captured = _capture_locked(harness, current, outgoing,
                                           os.path.join(store_dir(harness), outgoing + ".json"), current_ident)
            if _live_fingerprint(live_path) != before:
                continue  # the harness rewrote mid-read; its tokens would be lost — redo
            if harness == 'codex':
                _publish_native_preserving_displaced(live_path, target_record, current)
            else:
                atomic_write_json(live_path, target_record)
            swapped = True
            break
        if not swapped:
            raise AuthError(5, f"the live auth file kept changing during capture ({live_path}); not switching")
    else:
        _create_native_if_absent(live_path, target_record)

    swapped = identity_of(live_record(harness, required=True), harness)
    if swapped["slug"] and swapped["slug"] != slug:
        raise AuthError(5, f"post-switch verification failed: live file reports {swapped['slug']!r}")
    if harness == "codex":
        respawned = stop_codex_daemons(daemon_home)
        if respawned:
            print(f"⬇  stopped respawned codex app-server daemon(s) {respawned} — spawned during "
                  f"the swap window, may have read the previous auth", file=sys.stderr)
            daemons_stopped += respawned
        # a dying daemon can flush its OLD tokens after the swap — re-verify
        # the live file post-sweep and repair from the same in-store record.
        # Drift is judged by digest, the same standard the fallback verb
        # demands: an identity that cannot report a slug (minimal records)
        # must never read as drift.
        for _ in range(3):
            current = live_record(harness, required=True)
            swapped = identity_of(current, harness)
            if record_hash(current) == record_hash(target_record):
                break
            if _same_codex_identity(current, target_record):
                a, b = _native_epoch(current), _native_epoch(target_record)
                if a is not None and b is not None and a > b:
                    # Native re-login/refresh won after activation. Preserve it
                    # and update the vault; never replay a consumed older grant.
                    _capture_locked(harness, current, slug, target, swapped)
                    target_record = current
                    break
            if record_hash(current) != outgoing_record_hash:
                raise AuthError(5, 'unexpected native auth change during switch; retaining it for reconciliation')
            print("⚠️  the live auth file drifted during daemon teardown — re-writing "
                  f"{slug}", file=sys.stderr)
            _publish_native_preserving_displaced(live_path, target_record, current)
        else:
            raise AuthError(5, f"the live auth file kept drifting during daemon teardown ({live_path})")
    if record_hash(live_record(harness, required=True)) != record_hash(target_record):
        raise AuthError(5, "post-switch verification failed: active record differs from target")
    return {"switched_to": slug, "identity": swapped, "captured_before_leaving": captured,
            "daemons_stopped": daemons_stopped}


SCHEMA_VERSION = 1
TYPED_FAILURES = ("auth_invalid", "token_expired", "rate_limited")


def capabilities(harness):
    require_harness(harness)
    return {"capture": True, "switch": True, "rotate": True, "fallback": True,
            "oauth_refresh": harness == "codex", "quota_measurement": harness == "codex",
            "login": harness == "codex", "native_exec": True,
            "native_health_verification": False,
            "host_keyring_migration": False}


def rotation_policy(harness):
    path = os.path.join(store_dir(harness), ".policy.json")
    if not os.path.exists(path):
        return {"version": 1, "mode": "assignment", "automatic_fallback": False,
                "source_authority": "legacy-default"}
    try:
        doc = read_json(path)
        keys = {"version", "mode", "host", "profiles", "source_authority", "automatic_fallback"}
        valid = (isinstance(doc, dict) and set(doc) == keys and type(doc["version"]) is int
                 and doc["version"] == 1 and doc["mode"] in ("assignment", "local-pool")
                 and doc["host"] == HOSTNAME and isinstance(doc["profiles"], list)
                 and bool(doc["profiles"]) and all(isinstance(x, str) and x and slug_for(x) == x for x in doc["profiles"])
                 and isinstance(doc["source_authority"], str) and bool(doc["source_authority"].strip())
                 and type(doc["automatic_fallback"]) is bool)
    except (OSError, ValueError, TypeError, KeyError):
        valid = False
    if not valid:
        raise AuthError(6, "invalid rotation policy; refusing selection", {"reason": "invalid_policy"})
    return doc


def cmd_policy(args):
    harness = require_harness(args.harness)
    return {"schema_version": SCHEMA_VERSION, "harness": harness, "policy": rotation_policy(harness)}


def health_path(harness):
    return os.path.join(store_dir(harness), ".health.json")


def read_metadata(path):
    if not os.path.exists(path):
        return {}
    try:
        doc = read_json(path)
        if not isinstance(doc, dict):
            raise ValueError()
        return doc
    except (OSError, ValueError):
        raise AuthError(5, "auth metadata unreadable; refusing operation", {"reason": "invalid_metadata"})


def profile_health(harness, slug, record):
    path = health_path(harness)
    data = read_metadata(path)
    entry = data.get(slug, {})
    if not isinstance(entry, dict) or (entry and entry.get("state") not in ("reauthorization_required", "failed")):
        raise AuthError(5, "invalid auth health metadata", {"reason": "invalid_metadata"})
    if entry.get("record_hash") != record_hash(record):
        return {"state": "unknown", "reason": None}
    return entry


def record_failure(harness, slug, reason):
    # Never retain server response bodies: they may echo credentials.
    if reason not in (*TYPED_FAILURES, "refresh_invalid"):
        raise AuthError(6, "unsupported failure reason")
    path = health_path(harness)
    data = read_metadata(path)
    record = read_json(os.path.join(store_dir(harness), slug + ".json"))
    prior = data.get(slug, {})
    event = {"reason": reason, "at": time.time(), "record_hash": record_hash(record)}
    data[slug] = {**event, "state": "reauthorization_required" if reason in ("auth_invalid", "refresh_invalid") else "failed",
                  "history": prior.get("history", []) + [event]}
    atomic_write_json(path, data)


def active_slug(harness, record=None):
    record = live_record(harness) if record is None else record
    if record is None:
        return None
    digest = record_hash(record)
    for p in stored_profiles(harness):
        if not p.get("unreadable") and record_hash(read_json(p["path"])) == digest:
            return p["slug"]
    return identity_of(record, harness)["slug"]


def rotate_score(p, measure=True):
    """Ascending key: measured available, unknown, measured exhausted.

    Negative headroom puts MORE remaining capacity first; only existing
    windows participate. Missing/invalid meters remain unknown.
    """
    if not measure:
        return (2, 0, p["slug"])
    try:
        snap = usage_snapshot(read_json(p["path"]))
    except (OSError, ValueError):
        snap = {"ok": False}
    if snap.get("http_status") in (401, 403):
        return (4, 0, p["slug"])
    if snap.get("ok"):
        windows = [w for w in (snap.get("primary"), snap.get("secondary")) if w is not None]
        used = [w.get("used_percent") if isinstance(w, dict) else None for w in windows]
        valid = bool(used) and all(type(x) in (int, float) and math.isfinite(x) and 0 <= x <= 100 for x in used)
        if snap.get("limit_reached") or snap.get("allowed") is False:
            return (3, 0, p["slug"])
        if valid:
            headroom = min(100 - x for x in used)
            return (0 if headroom > 0 else 3, -headroom, p["slug"])
    return (2, 0, p["slug"])


def selection_inventory(harness):
    policy = rotation_policy(harness)
    current = active_slug(harness)
    counts = dict(stored=0, current=0, policy_filtered=0, expired=0, cooling=0,
                  reauthorization_required=0, failed=0, unreadable=0, invalid_record=0, eligible=0)
    candidates, reasons = [], {}
    for p in stored_profiles(harness):
        counts["stored"] += 1
        slug = p["slug"]
        if slug == current:
            reason = "current"
        elif ((policy["mode"] == "local-pool" and slug not in policy["profiles"])
              or (policy["mode"] == "assignment" and assignment_of(slug) not in (None, HOSTNAME))):
            reason = "policy_filtered"
        elif p.get("unreadable"):
            reason = "unreadable"
        elif p.get("invalid_record"):
            reason = "invalid_record"
        else:
            health = profile_health(harness, slug, read_json(p["path"]))
            if health["state"] in ("reauthorization_required", "failed"):
                reason = health["state"]
            elif p["expired"]:
                reason = "expired"
            elif cooling_until(harness, slug):
                reason = "cooling"
            else:
                reason = "eligible"
                candidates.append(p)
        counts[reason] += 1
        reasons[slug] = reason
    return candidates, {"counts": counts, "reasons": reasons, "policy": policy}


def pick_rotate_target(harness, measure=True):
    candidates, report = selection_inventory(harness)
    if not candidates:
        raise AuthError(2, "no eligible local profile; inspect selection counts and reasons", report)
    measure = measure and capabilities(harness)["quota_measurement"]
    ranked = [(rotate_score(p, measure), p) for p in candidates]
    for score, p in ranked:
        if score[0] == 4:
            record_failure(harness, p["slug"], "auth_invalid")
            report["reasons"][p["slug"]] = "reauthorization_required"
            report["counts"]["reauthorization_required"] += 1
        elif score[0] == 3:
            report["reasons"][p["slug"]] = "quota_exhausted"
    usable = [(score, p) for score, p in ranked if score[0] not in (3, 4)]
    if not usable:
        report["counts"]["quota_exhausted"] = sum(score[0] == 3 for score, _ in ranked)
        report["counts"]["eligible"] = 0
        raise AuthError(2, "no eligible profile after quota/auth measurement", report)
    return min(usable, key=lambda item: item[0])[1], True


def require_token_harness(harness, verb):
    if harness != "codex":
        raise AuthError(6, f"{verb} unsupported for {harness}; local capture/switch/rotate remain available",
                        {"reason": "unsupported_operation", "capabilities": capabilities(harness)})


def cmd_status(args):
    if args.harness == "all":
        return {"schema_version": SCHEMA_VERSION, "harnesses": {
            h: cmd_status(argparse.Namespace(harness=h)) for h in HARNESSES},
            "coverage": "registered adapters only"}
    harness = require_harness(args.harness)
    ident = identity_of(live_record(harness), harness) if os.path.exists(HARNESSES[harness]["auth_file"](harness)) else None
    profiles = stored_profiles(harness)
    prov = load_provenance(harness)
    binding = active_slug(harness)
    for p in profiles:
        p["live"] = p["slug"] == binding
        p["stored_freshness"] = "unreadable" if p.get("unreadable") else ("expired" if p["expired"] else ("clock_fresh" if p.get("expires_at") else "unknown"))
        p["auth_health"] = ({"state": "unknown", "reason": "unreadable"} if p.get("unreadable")
                            else profile_health(harness, p["slug"], read_json(p["path"])))
        p["quota"] = {"state": "unknown", "measured": False}
        cooling = cooling_until(harness, p["slug"]) if p["slug"] else None
        p["cooling_until"] = cooling
        p["cooldown_minutes_left"] = round((cooling - time.time()) / 60) if cooling else 0
        p["provenance"] = prov.get(p["slug"] or "")
    _, selection = selection_inventory(harness)
    out = {
        "schema_version": SCHEMA_VERSION,
        "active_binding": binding, "native_health": "unknown",
        "active_record_hash": record_hash(live_record(harness)) if ident else None,
        "capabilities": capabilities(harness), "selection": selection,
        "harness": harness,
        "host": HOSTNAME,
        "auth_file": HARNESSES[harness]["auth_file"](harness),
        "live": ident,
        "profiles": [{k: v for k, v in p.items() if k != "path"} for p in profiles],
    }
    if harness == 'codex' and ident:
        native = live_record(harness)
        match = next((p for p in profiles if p['slug'] == binding and not p.get('unreadable')), None)
        out['native_vault_sync'] = ('missing' if match is None else
            'in_sync' if record_hash(read_json(match['path'])) == record_hash(native) else 'drift_reconcile_required')
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
    slug = slug_for(args.slug)
    home = assignment_of(slug)
    if home and home != HOSTNAME and not getattr(args, "force", False) and not _store_is_reader(harness, slug):
        raise AuthError(
            6, f"{slug}'s deterministic home is {home!r} — this host is {HOSTNAME!r}. "
               f"Run the switch there, reassign with `assign {slug} {HOSTNAME}`, or override with --force (owner).")
    with fleet_lock():
        health = {'state': 'unknown', 'measured': False}
        original_native = live_record(harness)
        expected_native_hash = record_hash(original_native) if original_native is not None else None
        if harness == 'codex' and os.environ.get('YGG_AUTH_NO_PREFRESH') != '1' and _store_is_reader(harness, slug):
            # A reader copy is refreshed by PULLING from its writer, never by a grant here.
            cur = read_json(_store_path(harness, slug))
            exp = access_expiry(cur)
            if exp is None or exp - time.time() <= ACCESS_PULL_AHEAD_S:
                writer = get_writer(harness, slug) or "unknown"
                try:
                    bundle = pull_access(harness, slug, writer, _self_bytes())
                    apply_reader_bundle(harness, slug, bundle, writer, lock_held=True)
                except AuthError as exc:
                    recover_lineage(harness, slug, _self_bytes(), exc, lock_held=True)
        if harness == 'codex' and os.environ.get('YGG_AUTH_NO_PREFRESH') != '1':
            target_path = os.path.join(store_dir(harness), slug + '.json')
            target = read_json(target_path) if os.path.exists(target_path) else None
            if target is not None:
                native = live_record(harness)
                same_live = native is not None and _same_codex_identity(native, target)
                candidate = native if same_live else target
                probe = usage_snapshot(candidate)
                if probe.get('ok') is not True:
                    # Refresh only a dormant lineage owned here, never the
                    # outgoing account or a grant cached by the live runtime.
                    if same_live or get_writer(harness, slug) != HOSTNAME:
                        raise AuthError(5, 'target authentication not verified; native login retained',
                                        {'reason': 'target_auth_unverified', 'http_status': probe.get('http_status')})
                    if probe.get('http_status') not in (401, 403) and not identity_of(candidate)['expired']:
                        raise AuthError(5, 'target authentication probe unavailable; switch held')
                    before_refresh = live_record(harness)
                    if (record_hash(before_refresh) if before_refresh is not None else None) != expected_native_hash:
                        raise AuthError(5, 'native login changed during target probe; refresh held')
                    fresh = refresh_grant(candidate)
                    # A rotated grant must be retained even if validation later
                    # fails. Validate identity before native activation.
                    atomic_write_json(os.path.join(store_dir(harness), '.recovery', record_hash(fresh) + '.json'), fresh)
                    if not _same_codex_identity(candidate, fresh):
                        raise AuthError(5, 'refreshed target identity changed; activation held')
                    _capture_locked(harness, fresh, slug, target_path, identity_of(fresh))
                    probe = usage_snapshot(fresh)
                    if probe.get('ok') is not True:
                        raise AuthError(5, 'refreshed target retained but authentication unverified; switch held')
                health = {'state': 'usage_authenticated', 'measured': True,
                          'quota_allowed': probe.get('allowed'), 'limit_reached': probe.get('limit_reached')}
        if original_native is None and live_record(harness) is not None:
            raise AuthError(5, 'native login appeared during validation; activation held')
        result = switch(harness, args.slug, lock_held=True, expected_active_hash=expected_native_hash,
                        expected_absent=original_native is None)
        result['target_authentication'] = health
    return result


def cmd_rotate(args):

    harness = require_harness(args.harness)
    with fleet_lock():
        target, _ = pick_rotate_target(harness, measure=not args.fast)
        vacated_slug = active_slug(harness)
        vacated = {"slug": vacated_slug} if vacated_slug else None
        result = switch(harness, target["slug"], lock_held=True)
        cooldown = None
        if (vacated and vacated["slug"] and vacated["slug"] != target["slug"]
                and args.cooldown_minutes > 0):
            cooldown = {"slug": vacated["slug"],
                        "until": mark_cooldown(harness, vacated["slug"], args.cooldown_minutes)}
    result.update(schema_version=SCHEMA_VERSION, outcome="activated", actual_effect="unknown",
                  native_health="unknown", session_action="resume_or_restart_required",
                  replay_authorized=False, vacated_to_cooldown=cooldown)
    return result


def cmd_fallback(args):
    """One typed consumer event, one local activation, no request replay."""
    harness = require_harness(args.harness)
    if args.error_code not in TYPED_FAILURES or not args.event_id or len(args.event_id) > 128:
        raise AuthError(6, "fallback requires a typed error and bounded event id")
    with fleet_lock():
        policy = rotation_policy(harness)
        if not policy["automatic_fallback"]:
            raise AuthError(6, "automatic fallback disabled by policy", {"reason": "policy_disabled"})
        event_path = os.path.join(store_dir(harness), ".fallback-events.json")
        events = read_metadata(event_path)
        request = {"error_code": args.error_code, "expected_active_hash": args.expected_active_hash}
        if args.event_id in events:
            event = events[args.event_id]
            if not isinstance(event, dict) or "request" not in event:
                raise AuthError(5, "invalid fallback event metadata", {"reason": "invalid_metadata"})
            if event["request"] != request:
                raise AuthError(6, "event id reused with a different request")
            if "result" not in event:
                raise AuthError(5, "fallback event already attempted; inspect status before retry",
                                {"reason": "event_incomplete", "replay_authorized": False})
            return event["result"]
        record = live_record(harness, required=True)
        if record_hash(record) != args.expected_active_hash:
            raise AuthError(5, "active auth changed since consumer error; refusing fallback")
        slug = active_slug(harness, record)
        if not slug:
            raise AuthError(6, "active profile has no vault binding; capture with an explicit name first")
        # Persist intent BEFORE any auth mutation. Crash/retry cannot switch twice.
        events[args.event_id] = {"request": request, "at": time.time()}
        atomic_write_json(event_path, events)
        captured = capture(harness, name=slug, lock_held=True)
        if record_hash(read_json(captured["path"])) != args.expected_active_hash:
            raise AuthError(5, "active auth changed during fallback capture; refusing activation")
        if args.error_code == "rate_limited":
            mark_cooldown(harness, slug, DEFAULT_COOLDOWN_MINUTES)
        else:
            record_failure(harness, captured["slug"], args.error_code)
        target, _ = pick_rotate_target(harness, measure=False)
        result = switch(harness, target["slug"], lock_held=True, expected_active_hash=args.expected_active_hash)
        result.update(schema_version=SCHEMA_VERSION, outcome="activated", event_id=args.event_id,
                      actual_effect="unknown", native_health="unknown", replay_authorized=False,
                      session_action="resume_or_restart_required")
        events[args.event_id]["result"] = result
        atomic_write_json(event_path, events)
        return result


def bound_slug_by_digest(harness, record):
    """The vault slug whose stored record is byte-identical to `record`, or None.

    Unlike active_slug this never falls back to identity heuristics: a binding
    is PROVEN only by digest equality (the same standard the fallback verb
    demands before it will activate anything)."""
    digest = record_hash(record)
    for p in stored_profiles(harness):
        if p.get("unreadable"):
            continue
        try:
            if record_hash(read_json(p["path"])) == digest:
                return p["slug"]
        except (OSError, ValueError):
            continue
    return None


def _quota_evidence(snap):
    """True only when the snapshot carries at least one supported, well-formed
    quota reading: a usable window with a numeric used_percent, or an explicit
    boolean limit verdict. HTTP success alone is NOT quota evidence
    (fresh-P6): an empty payload stays unmeasured, never zero."""
    for w in (snap.get("primary"), snap.get("secondary")):
        if isinstance(w, dict):
            used = w.get("used_percent")
            if (type(used) in (int, float) and not isinstance(used, bool)
                    and math.isfinite(used) and 0 <= used <= 100):
                return True
    return any(type(flag) is bool for flag in (snap.get("limit_reached"), snap.get("allowed")))


def meter_snapshot(harness, record, measure=False):
    """Identity-safe quota meter for the exec receipt. Measuring is explicit
    (--measure): it is the preflight's ONLY optional network touch and exists
    only for adapters with quota_measurement. Everything else stays unknown —
    an unknown meter is never reported as free, and a response without valid
    quota evidence is never labeled measured (fresh-P6)."""
    if not measure:
        return {"measured": False, "state": "unknown"}
    if not capabilities(harness)["quota_measurement"]:
        return {"measured": False, "state": "unsupported"}
    snap = usage_snapshot(record)
    if not snap.get("ok"):
        return {"measured": False, "state": "unknown",
                "error_class": "http" if snap.get("http_status") else "unavailable"}
    if not _quota_evidence(snap):
        return {"measured": False, "state": "unknown",
                "error_class": "no_quota_evidence"}

    def win(w):
        if not isinstance(w, dict):
            return None
        return {"used_percent": w.get("used_percent"),
                "reset_in_minutes": w.get("reset_in_minutes")}

    return {"measured": True, "state": "measured", "plan_type": snap.get("plan_type"),
            "limit_reached": snap.get("limit_reached"), "allowed": snap.get("allowed"),
            "primary": win(snap.get("primary")), "secondary": win(snap.get("secondary"))}


def _exec_gate(harness, allow_refresh):
    """Prove the bound grant before any native spawn. Returns
    (gate, record, decided_hash, action).

    Fail-closed: unusable records, unprovable vault bindings, recorded auth
    failures and foreign lineages block the launch — expiry alone is NOT proof
    of a dead grant, but an expired grant must still be renewed before a
    process may use it: exactly ONE canonical refresh, single-writer lineage
    enforced, when allow_refresh. No other network happens here."""
    record = live_record(harness, required=True)
    if not usable_record(record, harness):
        raise AuthError(5, "live auth record is unusable; the grant cannot be proven",
                        {"reason": "grant_unproven"})
    slug = bound_slug_by_digest(harness, record)
    ident = identity_of(record, harness)
    gate = {"active_binding": slug, "record_hash": record_hash(record),
            "expired": bool(ident.get("expired")), "refreshable": False,
            "health_state": "unknown"}
    if slug is None:
        raise AuthError(5, "live record matches no vault profile — capture it first "
                           "(`capture`); an unproven grant cannot launch",
                        {"reason": "grant_unproven", **gate})
    health = profile_health(harness, slug, record)
    gate["health_state"] = health["state"]
    if health["state"] in ("reauthorization_required", "failed"):
        raise AuthError(5, f"profile {slug} carries a recorded auth failure "
                           f"({health.get('reason')}); reauthorization or a "
                           "policy-authorized fallback must come first",
                        {"reason": "auth_health_failed", **gate})
    if not gate["expired"]:
        return gate, record, gate["record_hash"], None
    tokens = record.get("tokens") if isinstance(record.get("tokens"), dict) else record
    refreshable = bool(tokens.get("refresh_token")) and capabilities(harness)["oauth_refresh"]
    gate["refreshable"] = refreshable
    writer = get_writer(harness, slug)
    if writer not in (None, "unknown", HOSTNAME):
        # Lineage law before any refresh path: a foreign writer's grant is
        # HELD here — never copied, never refreshed on this host.
        raise AuthError(5, f"{slug}'s lineage is owned by {writer}; refresh there — "
                           "this host holds the grant, it does not copy foreign secrets",
                        {"reason": "foreign_writer_held", **gate})
    if not allow_refresh:
        raise AuthError(5, "access grant is expired; preflight is a read-only receipt",
                        {"reason": "refresh_required" if refreshable else "reauthorization_required",
                         **gate})
    if not refreshable:
        raise AuthError(5, "access grant is expired and this adapter has no refresher",
                        {"reason": "reauthorization_required", **gate})
    # ONE bounded refresh through the canonical adapter. refresh_profile
    # records refresh_invalid on a typed rejection; there is no retry loop.
    fresh = refresh_profile(harness, slug, record)
    if not usable_record(fresh, harness):
        raise AuthError(5, "refresh returned an unusable grant; holding the launch",
                        {"reason": "grant_unproven", **gate})
    pre_refresh_hash = gate["record_hash"]
    # fresh-P3: the fleet lock cannot serialize a NATIVE writer. Re-read the
    # live file before persisting: if a native owner rewrote the grant while
    # we were refreshing, their newer grant is authoritative — never overwrite
    # it with our older minted response. The minted response is preserved as
    # lineage (its refresh token may still be live), never activated.
    current_native = live_record(harness, required=True)
    raced = record_hash(current_native) != pre_refresh_hash
    if raced:
        atomic_write_json(os.path.join(store_dir(harness), f".refresh-lineage-{slug}.json"),
                          {"schema_version": SCHEMA_VERSION, "reason": "refresh_race_preserved",
                           "slug": slug, "pre_refresh_hash": pre_refresh_hash,
                           "minted_response_hash": record_hash(fresh),
                           "native_record_hash": record_hash(current_native),
                           "minted": fresh})
        stamp_provenance(harness, slug, record=current_native,
                         source="exec-preflight-refresh-race-preserved",
                         writer_updated_at=time.time())
        record = current_native
    else:
        atomic_write_json(HARNESSES[harness]["auth_file"](harness), fresh)
        atomic_write_json(os.path.join(store_dir(harness), slug + ".json"), fresh)
        stamp_provenance(harness, slug, record=fresh, source="exec-preflight-refresh",
                         writer_updated_at=time.time())
        record = fresh
    gate.update(record_hash=record_hash(record), refreshed_once=True,
                health_state="unknown", refresh_persisted=not raced)
    if not usable_record(record, harness):
        raise AuthError(5, "native rebound grant is unusable; holding the launch",
                        {"reason": "grant_unproven", **gate})
    # fresh-P5: recompute expiry with the canonical identity rules on the
    # record that will actually be launched — a hardcoded expired=False would
    # admit a dead grant. A grant still expired after its one refresh is held.
    gate["expired"] = bool(identity_of(record, harness).get("expired"))
    if gate["expired"]:
        raise AuthError(5, "refreshed grant remains expired by the canonical identity "
                           "rules; holding the launch",
                        {"reason": "refresh_still_expired", **gate})
    if raced:
        rebound = bound_slug_by_digest(harness, record)
        if rebound is None:
            raise AuthError(5, "native grant rewritten during refresh matches no vault "
                               "profile; capture it before launch",
                            {"reason": "grant_unproven", **gate})
        gate["active_binding"] = rebound
        rebound_health = profile_health(harness, rebound, record)
        gate["health_state"] = rebound_health["state"]
        if rebound_health["state"] in ("reauthorization_required", "failed"):
            raise AuthError(5, "native rebound profile carries a recorded auth failure",
                            {"reason": "auth_health_failed", **gate})
    return gate, record, gate["record_hash"], "refresh_once"


def cmd_exec(args):
    """Prove the bound grant, then (unless --preflight-only) replace this
    process with the native argv: the child inherits the activated native
    cache, its own signals, args and exit code. No restart and no replay —
    a nonzero child exit is observed, never retried; a new launch needs a new
    parent decision (with a fresh typed event when policy authorizes one)."""
    harness = require_harness(args.harness)
    native_argv = list(args.native_argv or [])
    if native_argv and native_argv[0] == "--":
        # argparse's REMAINDER keeps the `--`; a native argv never starts with it
        native_argv = native_argv[1:]
    if not args.preflight_only and not native_argv:
        raise AuthError(6, "exec needs native argv after `--` (or --preflight-only)")
    if bool(args.typed_failure) != bool(args.event_id) or \
            (args.typed_failure and not args.observed_active_hash):
        raise AuthError(6, "--typed-failure needs --event-id and --observed-active-hash together")
    if args.preflight_only and args.typed_failure:
        # fresh-P1: a typed fallback is a mutating activation — it switches the
        # binding, cools or fails the old profile, captures, and journals an
        # event. Preflight-only is a read-only receipt, so the mutating
        # combination is rejected BEFORE any effect can happen.
        raise AuthError(6, "--preflight-only is read-only and cannot execute a typed "
                           "fallback; run the fallback without it",
                        {"reason": "preflight_read_only"})
    # fresh-P2: policy and every required metadata file are validated BEFORE
    # any refresh, activation, metering or state write — an invalid policy
    # must have zero side effects.
    policy = rotation_policy(harness)
    fallback_result = None
    if args.typed_failure:
        # ONE policy-authorized fallback for the adapter's typed pre-turn
        # failure. The canonical verb enforces the policy gate, event dedup,
        # the observed-hash CAS and never replays the failed request.
        fallback_result = cmd_fallback(argparse.Namespace(
            harness=harness, error_code=args.typed_failure, event_id=args.event_id,
            expected_active_hash=args.observed_active_hash, json=False))
    with fleet_lock():
        # A metadata writer can finish while this caller waits for the lock.
        # Validate the policy for the serialized decision, before any effect.
        policy = rotation_policy(harness)
        gate, record, decided_hash, action = _exec_gate(harness,
                                                        allow_refresh=not args.preflight_only)
        meter = meter_snapshot(harness, record, measure=args.measure)
    # Bounded post-lock re-check: a concurrent native rewrite between the
    # decision and the spawn means the verified binding is gone — refuse
    # rather than launch against unverified auth. (An in-flight in-place
    # refresh by the harness itself remains the capture-on-leave norm.)
    current = live_record(harness, required=True)
    if record_hash(current) != decided_hash:
        raise AuthError(5, "native auth changed between verification and launch; refusing to spawn",
                        {"reason": "concurrent_change", "decided_hash": decided_hash,
                         "observed_hash": record_hash(current)})
    receipt = {
        "schema_version": SCHEMA_VERSION,
        "verb": "exec",
        "outcome": "ready",
        "harness": harness,
        "provider": provider_of(harness),
        "host": HOSTNAME,
        "active_binding": gate["active_binding"],
        "grant_binding": {"proven": True, "record_hash": gate["record_hash"],
                          "expired": gate["expired"], "refreshable": gate["refreshable"],
                          "health_state": gate["health_state"]},
        "activation": {"performed": action == "refresh_once",
                       "kind": action or "none_required",
                       "fallback": bool(fallback_result)},        "policy": {"mode": policy["mode"], "automatic_fallback": policy["automatic_fallback"],
                   "source_authority": policy["source_authority"]},
        "meter": meter,
        "native_health": "unknown",
        "actual_effect": "unknown",
        "session_action": "child_native_auth_proof_required",
        "native_exec": {"spawned": not args.preflight_only, "argv_len": len(native_argv)},
    }
    if action == "refresh_once":
        # Truthful receipt (fresh-P1/fresh-P3): report whether the minted
        # grant was actually persisted, or preserved-as-lineage after a race.
        receipt["activation"]["minted_grant_persisted"] = bool(gate.get("refresh_persisted"))
    if fallback_result:
        receipt["fallback_event_id"] = args.event_id
        receipt["fallback_switched_to"] = fallback_result.get("switched_to")
    if args.preflight_only:
        return receipt
    if args.json:
        sys.stderr.write(json.dumps(receipt) + "\n")
        sys.stderr.flush()
    return _exec_spawn(native_argv, receipt)


def _exec_spawn(native_argv, receipt=None):
    """Replace this process with the native argv — same pid, so the parent's
    signal target, argv and exit code are the native CLI's own. The receipt
    rides along only for offline seams; os.execvp never returns here."""
    try:
        os.execvp(native_argv[0], native_argv)
    except OSError as exc:
        raise AuthError(6, f"native process could not start: {native_argv[0]!r} "
                           f"({exc.strerror or type(exc).__name__})",
                        {"reason": "native_exec_failed"})


def cmd_harnesses(args):
    out = {}
    for name, spec in sorted(HARNESSES.items()):
        out[name] = {
            "auth_file": spec["auth_file"](name),
            "login": spec["login"],
            "live_note": spec["live_note"],
            "capabilities": capabilities(name),
        }
    return {"schema_version": SCHEMA_VERSION, "harnesses": out, "store": STORE_ROOT,
            "coverage": "registered adapters only; other harnesses unsupported"}


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


def device_login(harness, name=None, notify=True, force=False):
    """Ported from the proven litellm prototype: codex device-code OAuth.
    Dream ACK-52ebf788f0: the code is the one thing a human must touch, so it
    also goes where the owner actually looks — a pinned msgGraph card that
    auto-closes when the poll completes."""
    if HARNESSES[harness]["login"] is None:
        raise AuthError(4, f"login is not implemented for {harness}: {HARNESSES[harness]['live_note']}")

    if not force:
        _sign_in_gate(harness, name)

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
        if harness == "codex":
            # same daemon trap as switch(): the freshly logged-in record must
            # be what a running app-server daemon serves, not its startup cache
            stop_codex_daemons(os.path.dirname(live_path))
        captured = None
        if os.path.exists(live_path):
            current = live_record(harness, required=True)
            current_ident = identity_of(current, harness)
            if current_ident["slug"] and current_ident["slug"] != slug:
                captured = _capture_locked(harness, current, current_ident["slug"],
                                           os.path.join(store_dir(harness), f"{current_ident['slug']}.json"),
                                           current_ident)
        atomic_write_json(os.path.join(store_dir(harness), f"{slug}.json"), record)
        atomic_write_json(live_path, record)
        if harness == "codex":
            respawned = stop_codex_daemons(os.path.dirname(live_path))
            if respawned:
                print(f"⬇  stopped respawned codex app-server daemon(s) {respawned} after login",
                      file=sys.stderr)
            final = identity_of(live_record(harness, required=True), harness)
            if final["slug"] and final["slug"] != slug:
                # a dying daemon flushed stale tokens over the fresh login
                print("⚠️  the live auth file drifted during login teardown — re-writing "
                      f"{slug}", file=sys.stderr)
                atomic_write_json(live_path, record)
    return {"logged_in": slug, "identity": ident, "captured_before_leaving": captured}


def cmd_login(args):
    require_token_harness(require_harness(args.harness), "login")

    return device_login(require_harness(args.harness), name=args.name,
                        notify=not args.no_notify, force=getattr(args, "force", False))


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
    require_token_harness(require_harness(args.harness), "import")

    if not args.file:
        raise AuthError(6, "import needs the path to the foreign auth record")
    return import_foreign(args.file, require_harness(args.harness), name=args.name)


def fetch_remote(host, harness, slug, activate=False):
    """Install a READER copy of an account whose refresh grant lives on `host`
    (or on the writer `host` reports). No refresh token crosses: the writer serves
    an access bundle, and this host stores it with the reader sentinel."""
    require_harness(harness)
    slug = slug_for(slug)
    script_bytes = _self_bytes()
    writer = host
    ev = _remote_evidence(host, harness, slug, script_bytes)
    if ev.get("kind") == "reader" and ev.get("writer") and ev["writer"] != host:
        writer = ev["writer"]
    bundle = pull_access(harness, slug, writer, script_bytes)
    with fleet_lock():
        apply_reader_bundle(harness, slug, bundle, writer, lock_held=True)
    if activate:
        # Re-enter through the checked activation route after releasing the lock.
        result = cmd_switch(argparse.Namespace(harness=harness, slug=slug, force=False))
        return {**result, "fetched_from": writer, "activated": True}
    return {"fetched_from": writer, "slug": slug, "writer": writer, "activated": False,
            "access_expires_at": bundle.get("access_expires_at")}


# ── Lineage law (2026-11-16) ─────────────────────────────────────────────────
# A refresh token ROTATES on every use. Two hosts holding copies of one family
# race: whichever refreshes first kills the other, and the loser is told
# "refresh grant rejected: HTTP 401". That is not an expiry; it is a second
# writer. The architecture that ends it:
#   WRITER  the one host whose record holds the live refresh grant. Only the
#           writer calls the refresh grant, and only when the access token is
#           close to expiry, so the family is used at least every ~7 days and
#           an account never idles long enough to need a sign-in.
#   READER  every other host. It holds the access + id token and a sentinel in
#           place of the refresh token. A reader cannot rotate the family, and
#           the provider rejecting a sentinel costs the family nothing.
#   Readers pull a fresh access bundle from the writer before expiry (sync).
#           Refresh grants never cross hosts.
#   RECOVERY a dead writer grant moves the lineage to the newest live copy
#           elsewhere (one candidate: an older token is a rotated-out one).
#   SIGN-IN refused while any fleet host refreshed the account within 30 days,
#           or while a host that may hold it cannot be read.
READER_RT_PREFIX = "ygg-reader:"
ACCESS_REFRESH_AHEAD_S = 3 * 86400   # writer refreshes when the access token has < 3 days left
ACCESS_PULL_AHEAD_S = 2 * 86400      # reader pulls when the access token has < 2 days left
SIGN_IN_IDLE_FLOOR_S = 30 * 86400    # sign-in only after 30 days with no refresh anywhere


def _self_bytes():
    """This script's own bytes. Sent over ssh so a peer runs exactly this revision."""
    with open(os.path.abspath(__file__), "rb") as f:
        return f.read()


def _tokens_of(record):
    return record.get("tokens") if isinstance(record.get("tokens"), dict) else record


def is_reader_record(record):
    rt = _tokens_of(record).get("refresh_token")
    return isinstance(rt, str) and rt.startswith(READER_RT_PREFIX)


def access_expiry(record):
    exp = decode_jwt_claims(_tokens_of(record).get("access_token") or "").get("exp")
    return float(exp) if type(exp) in (int, float) else None


def access_bundle(record):
    """The only thing a writer hands a reader: access + id token, never the grant."""
    t = _tokens_of(record)
    return {"access_token": t.get("access_token"), "id_token": t.get("id_token"),
            "account_id": t.get("account_id"), "last_refresh": record.get("last_refresh"),
            "access_expires_at": access_expiry(record)}


def reader_record(bundle, writer):
    return {"auth_mode": "chatgpt", "OPENAI_API_KEY": None,
            "tokens": {"id_token": bundle["id_token"], "access_token": bundle["access_token"],
                       "refresh_token": READER_RT_PREFIX + writer,
                       "account_id": bundle["account_id"]},
            "last_refresh": bundle["last_refresh"]}


def _store_path(harness, slug):
    return os.path.join(store_dir(harness), slug + ".json")


def _store_is_reader(harness, slug):
    path = _store_path(harness, slug)
    try:
        return os.path.exists(path) and is_reader_record(read_json(path))
    except (OSError, ValueError):
        return False


def local_lineage(harness, slug):
    """The newest copy of one account on THIS host: (source, record), or (None, None).
    Codex rewrites the live file in place, so live is often newer than the store;
    native refresh order decides, and a tie goes to live."""
    found = []
    live = live_record(harness)
    if live is not None and identity_of(live, harness)["slug"] == slug:
        found.append(("live", live))
    path = _store_path(harness, slug)
    if os.path.exists(path):
        found.append(("store", read_json(path)))
    if not found:
        return None, None
    return max(found, key=lambda c: ((_native_epoch(c[1]) or 0), c[0] == "live"))


def _restart_codex_for(harness):
    if harness == "codex":
        # codex app-server daemons cache the grant they were started with; a
        # rotated-out grant left in a daemon's memory fails its next refresh.
        stop_codex_daemons(os.path.dirname(HARNESSES[harness]["auth_file"](harness)))


def _persist_writer(harness, slug, rec, refreshed):
    """Keep this host's writer copy canonical. A refresh also rewrites the live
    file when it IS this lineage, and restarts the daemons that cache it."""
    atomic_write_json(_store_path(harness, slug), rec)
    stamp_provenance(harness, slug, record=rec, writer=HOSTNAME, role="writer",
                     source="refresh" if refreshed else "capture", writer_updated_at=time.time())
    if refreshed:
        live = live_record(harness)
        if live is not None and identity_of(live, harness)["slug"] == slug:
            atomic_write_json(HARNESSES[harness]["auth_file"](harness), rec)
            _restart_codex_for(harness)


def _writer_access(harness, slug):
    """Writer side. Returns (access bundle, refreshed?) for a lineage this host
    refreshes. A READ never rotates: the grant runs only when the access token is
    near expiry. The refresh token is never in the return value."""
    with fleet_lock():
        src, rec = local_lineage(harness, slug)
        if rec is None:
            raise AuthError(6, f"no {slug} lineage on this host", {"reason": "no_lineage"})
        if is_reader_record(rec):
            writer = get_writer(harness, slug)
            raise AuthError(6, f"{slug} is a reader copy here; its writer is {writer}",
                            {"reason": "not_writer", "writer": writer})
        exp = access_expiry(rec)
        if exp is not None and exp - time.time() > ACCESS_REFRESH_AHEAD_S:
            if src == "live":
                _persist_writer(harness, slug, rec, refreshed=False)
            return access_bundle(rec), False
        if profile_health(harness, slug, rec).get("state") == "reauthorization_required":
            # The provider refused this exact grant. Presenting it again on every sync
            # adds nothing and repeats the refusal; a new grant changes the hash and re-arms.
            raise AuthError(5, f"{slug}: this grant was refused by the provider and is not presented again",
                            {"reason": "reauth_required"})
        why = _refresh_gate(fleet_evidence(harness, slug, _self_bytes()), _native_epoch(rec) or 0)
        if why:
            raise AuthError(5, f"refresh held for {slug}: {why}", {"reason": "held_unproven"})
        fresh = refresh_profile(harness, slug, rec)   # raises refresh_invalid on a dead grant
        _persist_writer(harness, slug, fresh, refreshed=True)
        return access_bundle(fresh), True


def _refresh_gate(rows, local_epoch):
    """A grant may be presented to the provider ONLY when this host can prove it holds
    the newest grant: the roster is known, every peer answered, and no peer holds a newer
    writer grant. Anything less holds the refresh. Presenting a rotated-out grant can get
    the whole family revoked, and a peer that cannot be read might hold a newer one."""
    if not load_fleet_hosts():
        return "fleet roster missing: cannot prove this grant is the newest"
    peers = [r for r in rows if r.get("host") != HOSTNAME]
    if any(r.get("kind") == "unreachable" for r in peers):
        return "a peer is unreachable: cannot prove this grant is the newest"
    newer = [r for r in peers if r.get("kind") == "writer" and (r.get("last_refresh_epoch") or 0) > local_epoch]
    if newer:
        return f"{newer[0].get('host')} holds a newer grant: this one is superseded"
    return None


def apply_reader_bundle(harness, slug, bundle, writer, lock_held=False, supersede=False):
    """Reader side: install an access-only copy of a lineage whose writer is `writer`.
    Refuses to demote a writer copy that is NEWER than the bundle (that would throw
    away this host's live grant), unless `supersede`, which only recovery passes
    after this host's own grant has just been rejected."""
    if not lock_held:
        with fleet_lock():
            return apply_reader_bundle(harness, slug, bundle, writer, lock_held=True, supersede=supersede)
    for key in ("access_token", "id_token", "account_id", "last_refresh"):
        if not isinstance(bundle.get(key), str) or not bundle.get(key):
            raise AuthError(5, f"{writer} returned an incomplete access bundle", {"reason": "bad_bundle"})
    rec = reader_record(bundle, writer)
    new_epoch = _native_epoch(rec) or 0
    _src, local = local_lineage(harness, slug)
    live = live_record(harness)
    live_here = live is not None and identity_of(live, harness)["slug"] == slug
    if not supersede:
        if local is not None and not is_reader_record(local) and (_native_epoch(local) or 0) >= new_epoch:
            raise AuthError(5, f"this host holds a writer grant for {slug} at least as new as {writer}'s; "
                               "not demoting it", {"reason": "local_writer_newer"})
        if live_here and not is_reader_record(live) and (_native_epoch(live) or 0) >= new_epoch:
            raise AuthError(5, f"the live {slug} login here is a writer at least as new as {writer}'s; "
                               "not demoting it", {"reason": "live_writer_newer"})
    atomic_write_json(_store_path(harness, slug), rec)
    stamp_provenance(harness, slug, record=rec, writer=writer, role="reader",
                     source="sync", writer_updated_at=time.time())
    if live_here:
        atomic_write_json(HARNESSES[harness]["auth_file"](harness), rec)
        _restart_codex_for(harness)
    return {"slug": slug, "writer": writer, "access_expires_at": bundle.get("access_expires_at")}


def pull_access(harness, slug, writer, script_bytes):
    """Ask the writer for its current access bundle over ssh. The writer runs THIS
    revision from stdin, so no remote install is needed. The bundle is parsed here
    and never printed; the refresh grant is never requested."""
    last = "no destination tried"
    for dest in ssh_dest_candidates(writer):
        proc = ssh_script(dest, ["access", "--harness", harness, "--slug", slug, "--for-reader"],
                          script_bytes, timeout=60)
        if proc is None:
            last = f"ssh {dest} timed out"
            continue
        if proc.returncode != 0:
            last = f"ssh {dest}: {proc.stderr.decode('utf-8', 'replace').strip()[-200:] or proc.returncode}"
            continue
        try:
            bundle = json.loads(proc.stdout.decode("utf-8"))
        except ValueError:
            last = f"{dest}: unparseable access bundle"
            continue
        remember_ssh_dest(dest)
        return bundle
    raise AuthError(5, f"could not pull {slug} access from {writer}: {last}", {"reason": "pull_failed"})


def local_evidence(harness, slug):
    """What this host holds for one account: kinds and times only, never a token."""
    _src, rec = local_lineage(harness, slug)
    if rec is None:
        return {"kind": "absent"}
    if is_reader_record(rec):
        return {"kind": "reader", "writer": get_writer(harness, slug),
                "access_expires_at": access_expiry(rec)}
    of_record = get_writer(harness, slug)
    return {"kind": "writer" if of_record in (None, HOSTNAME) else "orphan",
            "of_record": of_record, "last_refresh_epoch": _native_epoch(rec),
            "access_expires_at": access_expiry(rec)}


def _remote_evidence(host, harness, slug, script_bytes):
    last = "no destination tried"
    for dest in ssh_dest_candidates(host):
        proc = ssh_script(dest, ["evidence", "--harness", harness, "--slug", slug, "--json"],
                          script_bytes, timeout=30)
        if proc is None:
            last = f"ssh {dest} timed out"
            continue
        if proc.returncode == 0:
            try:
                ev = json.loads(proc.stdout.decode("utf-8"))
            except ValueError:
                last = f"{dest}: unparseable evidence"
                continue
            if isinstance(ev, dict) and "kind" in ev:
                remember_ssh_dest(dest)
                return ev
        last = f"ssh {dest}: {proc.stderr.decode('utf-8', 'replace').strip()[-160:] or proc.returncode}"
    raise AuthError(5, f"could not read {slug} evidence from {host}: {last}", {"reason": "unreachable"})


def fleet_evidence(harness, slug, script_bytes):
    """One row per fleet host, self included. Peers answer through their own read-only
    `evidence` verb: it never refreshes and never prints a token."""
    rows = [{**local_evidence(harness, slug), "host": HOSTNAME}]
    for host in load_fleet_hosts():
        if host == HOSTNAME:
            continue
        try:
            rows.append({**_remote_evidence(host, harness, slug, script_bytes), "host": host})
        except AuthError:
            rows.append({"host": host, "kind": "unreachable"})
    return rows


def _reauth_refused(slug, rows, failure):
    """No live grant anywhere. Sign-in is the last resort, and only after 30 idle days."""
    ages = [f"{r['host']}={(time.time() - r['last_refresh_epoch']) / 86400:.1f}d"
            for r in rows if r.get("last_refresh_epoch")]
    newest = max((r["last_refresh_epoch"] for r in rows if r.get("last_refresh_epoch")), default=None)
    if newest is not None and time.time() - newest < SIGN_IN_IDLE_FLOOR_S:
        msg = (f"{slug}: a grant was refused inside the 30-day window ({', '.join(ages)}). "
               "That is not an expiry. Investigate before any sign-in.")
        reason = "refused_inside_window"
    else:
        msg = (f"{slug}: no host holds a live grant ({', '.join(ages) or 'no copies'}). "
               "Sign-in is the only path now.")
        reason = "reauth_required"
    return AuthError(5, msg, {"reason": reason, "failure": str(failure)[:200]})


def recover_lineage(harness, slug, script_bytes, failure, lock_held=False):
    """This host's grant for the lineage is dead. Only a copy NEWER than the dead one can
    be live: any older copy was rotated out before it, and presenting it can make the
    provider revoke the whole family. Try the newest such copy, and only that one. On
    success this host becomes a reader of it."""
    _src, rec = local_lineage(harness, slug)
    dead_epoch = (_native_epoch(rec) or 0) if rec is not None else 0
    rows = fleet_evidence(harness, slug, script_bytes)
    cands = [r for r in rows if r.get("kind") == "writer" and r.get("host") != HOSTNAME
             and (r.get("last_refresh_epoch") or 0) > dead_epoch]
    if not cands:
        raise _reauth_refused(slug, rows, failure)
    best = max(cands, key=lambda r: r["last_refresh_epoch"])
    try:
        bundle = pull_access(harness, slug, best["host"], script_bytes)
        apply_reader_bundle(harness, slug, bundle, best["host"], lock_held=lock_held, supersede=True)
    except AuthError as exc:
        raise _reauth_refused(slug, rows, f"{failure}; {best['host']}: {exc}")
    return {"slug": slug, "state": "lineage_moved", "writer": best["host"]}


def sync_lineage(harness, slug, script_bytes):
    """One account on this host. A reader pulls when due. A writer keeps its grant
    alive when due. A writer-capable copy this host does not own is demoted by pulling
    from its writer. Never refreshes a grant it does not own."""
    _src, rec = local_lineage(harness, slug)
    if rec is None:
        return {"slug": slug, "state": "absent"}
    of_record = get_writer(harness, slug)
    if is_reader_record(rec) or of_record not in (None, HOSTNAME):
        writer = of_record or "unknown"
        exp = access_expiry(rec)
        if is_reader_record(rec) and exp is not None and exp - time.time() > ACCESS_PULL_AHEAD_S:
            return {"slug": slug, "state": "reader_fresh", "writer": writer}
        try:
            bundle = pull_access(harness, slug, writer, script_bytes)
            apply_reader_bundle(harness, slug, bundle, writer)
            return {"slug": slug, "state": "reader_pulled", "writer": writer}
        except AuthError as exc:
            if not is_reader_record(rec):
                return {"slug": slug, "state": "orphan_unresolved", "writer": writer,
                        "note": str(exc)[:160]}
            return recover_lineage(harness, slug, script_bytes, exc)
    # A writer-capable copy that another host holds a NEWER grant for is a rotated-out
    # member of the family. Presenting it can make the provider revoke the whole family,
    # so it is never refreshed here: it is demoted by pulling from the newest writer.
    local_epoch = _native_epoch(rec) or 0
    rows = fleet_evidence(harness, slug, script_bytes)
    newer = [r for r in rows if r.get("host") != HOSTNAME and r.get("kind") == "writer"
             and (r.get("last_refresh_epoch") or 0) > local_epoch]
    if newer:
        best = max(newer, key=lambda r: r["last_refresh_epoch"])
        try:
            bundle = pull_access(harness, slug, best["host"], script_bytes)
            apply_reader_bundle(harness, slug, bundle, best["host"])
            return {"slug": slug, "state": "superseded_moved", "writer": best["host"]}
        except AuthError as exc:
            return {"slug": slug, "state": "superseded_unresolved", "writer": best["host"],
                    "note": str(exc)[:160]}
    try:
        _bundle, refreshed = _writer_access(harness, slug)
        return {"slug": slug, "state": "writer_refreshed" if refreshed else "writer_fresh"}
    except AuthError as exc:
        if exc.details.get("reason") == "held_unproven":
            return {"slug": slug, "state": "held", "note": str(exc)[:200]}
        if exc.details.get("reason") == "reauth_required":
            return {"slug": slug, "state": "reauth_required", "note": str(exc)[:200]}
        if exc.details.get("reason") != "refresh_invalid":
            raise
        return recover_lineage(harness, slug, script_bytes, exc)


def stored_slugs(harness):
    slugs = set()
    d = store_dir(harness)
    if os.path.isdir(d):
        slugs |= {f[:-5] for f in os.listdir(d) if f.endswith(".json") and not f.startswith(".")}
    live = live_record(harness)
    if live is not None and identity_of(live, harness)["slug"]:
        slugs.add(identity_of(live, harness)["slug"])
    return sorted(slugs)


def cmd_sync(args):
    """Timer verb: keep every lineage on this host alive. Readers pull, writers refresh
    when due. Safe to run any time; a fresh copy costs nothing and makes no request."""
    harness = require_harness(args.harness)
    require_token_harness(harness, "sync")
    script_bytes = _self_bytes()
    rows = []
    scope = [slug_for(args.slug)] if getattr(args, "slug", None) else stored_slugs(harness)
    for slug in scope:
        try:
            rows.append(sync_lineage(harness, slug, script_bytes))
        except AuthError as exc:
            rows.append({"slug": slug, "state": "error", "reason": exc.details.get("reason"),
                         "message": str(exc)[:240]})
    return {"harness": harness, "lineages": rows}


def cmd_access(args):
    """Writer side of a reader's pull. Emits the access bundle on stdout and never the
    refresh grant. Refuses a terminal, so a token cannot land in scrollback or a log."""
    if not args.for_reader:
        raise AuthError(6, "access is a machine verb for readers; pass --for-reader")
    if sys.stdout.isatty():
        raise AuthError(6, "refusing to print access tokens to a terminal")
    harness = require_harness(args.harness)
    if not args.slug:
        raise AuthError(6, "access needs --slug")
    bundle, _refreshed = _writer_access(harness, slug_for(args.slug))
    sys.stdout.write(json.dumps(bundle))
    sys.stdout.flush()
    return {"_printed": True}


def cmd_evidence(args):
    harness = require_harness(args.harness)
    if not args.slug:
        raise AuthError(6, "evidence needs --slug")
    return local_evidence(harness, slug_for(args.slug))


def _sign_in_gate(harness, name):
    """Sign-in is the last resort. Refused while any fleet host refreshed this account
    within 30 days, and refused while a host that may hold it cannot be read."""
    if not name:
        raise AuthError(6, "login needs --name <account email>, so the fleet can be checked "
                           "for a live lineage first", {"reason": "name_required"})
    slug = slug_for(name)
    rows = fleet_evidence(harness, slug, _self_bytes())
    epochs = [r["last_refresh_epoch"] for r in rows if r.get("kind") == "writer" and r.get("last_refresh_epoch")]
    if epochs:
        newest = max(epochs)
        age = time.time() - newest
        if age < SIGN_IN_IDLE_FLOOR_S:
            on = [r["host"] for r in rows if r.get("kind") == "writer" and r.get("last_refresh_epoch") == newest]
            raise AuthError(5, f"{slug} is live: refreshed {age / 86400:.1f} days ago on {', '.join(on)}. "
                               "No sign-in. Run `ygg auth sync` to move it here.", {"reason": "lineage_live"})
    unreachable = [r["host"] for r in rows if r.get("kind") == "unreachable"]
    if unreachable:
        raise AuthError(5, f"cannot prove {slug} has no live lineage: unreachable {', '.join(unreachable)}. "
                           "Retry when they answer, or --force (owner).", {"reason": "lineage_unproven"})


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
        return {"ok": False, "error": f"HTTP {exc.code}", "http_status": exc.code}
    except (urllib.error.URLError, OSError, ValueError) as exc:
        return {"ok": False, "error": type(exc).__name__}

    rl = payload.get("rate_limit") or {}

    def window(w):
        if not isinstance(w, dict):
            return None
        return {"used_percent": w.get("used_percent"),
                "window_minutes": round(w.get("limit_window_seconds", 0) / 60),
                "reset_at": w.get("reset_at"),
                "reset_in_minutes": round(w.get("reset_after_seconds", 0) / 60)}

    pw, sw = window(rl.get("primary_window")), window(rl.get("secondary_window"))
    # BANKED = OpenAI granted a rate-limit-reset credit that is still available
    # (owner 2026-09-30: it is the credit, not window fullness — windows are
    # the windows). Paid plans only; free accounts have no reset credits.
    gift = ((payload.get("rate_limit_reset_credits") or {}).get("available_count")) or 0
    banked = gift > 0 and (payload.get("plan_type") or "") not in ("", "free")
    quota = {"ok": True,
            "email": payload.get("email"),
            "plan_type": payload.get("plan_type"),
            "limit_reached": rl.get("limit_reached") if type(rl.get("limit_reached")) is bool else None,
            "allowed": rl.get("allowed") if type(rl.get("allowed")) is bool else None,
            "primary": pw,
            "secondary": sw,
            "banked": banked,
            "banked_weekly_left": (round(max(0, 100 - sw["used_percent"]))
                                   if sw and type(sw.get("used_percent")) in (int, float) else None),
            "reset_credits": payload.get("rate_limit_reset_credits")}
    quota["evidence"] = "quota" if _quota_evidence(quota) else "none"
    return quota


def load_assignments():
    if os.path.exists(ASSIGNMENTS_FILE):
        try:
            return read_json(ASSIGNMENTS_FILE) or {}
        except ValueError:
            return {}
    return {}


def save_assignments(a):
    atomic_write_json(ASSIGNMENTS_FILE, a)


def assignment_of(slug):
    """slug -> home host, or None when unassigned (unassigned = any host may hold it)."""
    a = load_assignments().get(slug or "")
    return (a or {}).get("host") if isinstance(a, dict) else None


def push_assignments():
    """Propagate the assignments file to every saved fleet host (deterministic
    only works if every host reads the same table)."""
    hosts = load_fleet_hosts()
    pushed, failed = [], []
    for host in hosts:
        if host == HOSTNAME:
            continue
        proc = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host,
             f"mkdir -p {STORE_ROOT} && cat > {ASSIGNMENTS_FILE}.tmp && "
             f"mv {ASSIGNMENTS_FILE}.tmp {ASSIGNMENTS_FILE}"],
            input=(json.dumps(load_assignments(), indent=2) + "\n").encode(),
            capture_output=True, timeout=20)
        (pushed if proc.returncode == 0 else failed).append(host)
    return {"pushed": pushed, "failed": failed}


def cmd_assign(args):
    """Deterministic home assignment: an account lives on ONE host. switch/rotate
    refuse to activate it elsewhere (owner override: --force)."""
    if not args.slug or not args.host:
        raise AuthError(6, "assign needs a slug and a host (or --clear with a slug)")
    a = load_assignments()
    if args.clear:
        a.pop(slug_for(args.slug), None)
    else:
        a[slug_for(args.slug)] = {"host": args.host,
                                  "since": now_rfc3339_nanos(), "by": f"{HOSTNAME}/{os.environ.get('USER','agent')}"}
    with fleet_lock():
        save_assignments(a)
    prop = push_assignments()
    return {"assignments": a, "propagated": prop}


def cmd_redeem(args):
    require_token_harness(require_harness(args.harness), "redeem")

    """Redeem the account's banked rate-limit-reset credit (fresh weekly + 5h).
    The same action as llmproxy's web "use reset" button — explicit call only."""
    harness = require_harness(args.harness)
    live_path = HARNESSES[harness]["auth_file"](harness)
    if args.slug:
        slug = slug_for(args.slug)
        path = os.path.join(store_dir(harness), f"{slug}.json")
        if not os.path.exists(path):
            raise AuthError(6, f"no stored profile {slug!r} for {harness}")
        record = read_json(path)
    elif os.path.exists(live_path):
        record = live_record(harness, required=True)
    else:
        raise AuthError(6, "no live record; pass a slug")
    tokens = record.get("tokens") or record
    headers = {"Authorization": "Bearer " + tokens["access_token"],
               "ChatGPT-Account-Id": tokens.get("account_id", ""),
               "User-Agent": CODEX_USER_AGENT, "Content-Type": "application/json"}
    base = "https://chatgpt.com/backend-api"
    req = urllib.request.Request(base + "/codex/rate-limit-reset-credits?client_version=1.2.0",
                                 headers=headers)
    with urllib.request.urlopen(req, timeout=20) as resp:
        credits = [c for c in json.loads(resp.read().decode()).get("credits", [])
                   if c.get("status") == "available"]
    if not credits:
        return {"redeemed": False, "reason": "no available reset credit",
                "slug": args.slug or "(live)"}
    req = urllib.request.Request(
        base + "/codex/rate-limit-reset-credits/consume?client_version=1.2.0",
        data=json.dumps({"redeem_request_id": credits[0]["id"]}).encode(),
        headers=headers, method="POST")
    with urllib.request.urlopen(req, timeout=30) as resp:
        out = json.loads(resp.read().decode())
    return {"redeemed": out.get("code") == "reset",
            "credit": out.get("credit", {}), "slug": args.slug or "(live)"}


def cmd_usage(args):
    require_token_harness(require_harness(args.harness), "usage")

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
    if is_reader_record(record):
        raise AuthError(6, "reader copy: the refresh grant belongs to this account's writer; run `ygg auth sync`",
                        {"reason": "not_writer"})
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
        reason = "refresh_invalid" if exc.code in (400, 401, 403) else "refresh_unavailable"
        raise AuthError(5, f"refresh grant rejected: HTTP {exc.code}", {"reason": reason, "http_status": exc.code})
    except (urllib.error.URLError, OSError, ValueError) as exc:
        raise AuthError(5, "refresh grant unavailable", {"reason": "refresh_unavailable"})
    if not isinstance(got, dict):
        raise AuthError(5, 'refresh response is not an object')
    if isinstance(got.get('refresh_token'), str) and got['refresh_token']:
        recovery = json.loads(json.dumps(record))
        container = recovery['tokens'] if isinstance(recovery.get('tokens'), dict) else recovery
        container['refresh_token'] = got['refresh_token']
        if isinstance(got.get('access_token'), str):
            container['access_token'] = got['access_token']
        if isinstance(got.get('id_token'), str):
            container['id_token'] = got['id_token']
        atomic_write_json(os.path.join(store_dir('codex'), '.recovery', record_hash(recovery) + '.json'), recovery)
    if not got.get("access_token"):
        raise AuthError(5, "refresh response missing access_token")
    new = json.loads(json.dumps(record))  # deep copy, verbatim shape preserved
    container = new["tokens"] if "tokens" in new else new
    container["access_token"] = got.get("access_token")
    if got.get("refresh_token"):  # rotation: the old one is dead the moment this returns
        container["refresh_token"] = got["refresh_token"]
    if got.get("id_token"):
        container["id_token"] = got["id_token"]
    if "last_refresh" in new:
        new["last_refresh"] = now_rfc3339_nanos()
    # fresh-P5: the minted grant defines the new expiry — reconcile the
    # persisted top-level expires_at from the response. Retaining the dead
    # token's expiry made the refreshed record read as expired forever.
    if "expires_at" in new:
        got_in = got.get("expires_in")
        if type(got_in) in (int, float) and got_in > 0 and math.isfinite(got_in):
            new["expires_at"] = time.time() + got_in
        else:
            claims = decode_jwt_claims(container.get("access_token") or "")
            if claims.get("exp"):
                new["expires_at"] = claims["exp"]
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
    """llmproxy transport: direct `ssh llmproxy` first (the wired path), then
    the manin jump with lxc-attach for hosts without the direct config."""
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", LLMPROXY_SSH_HOST,
           cmd_argv]
    try:
        proc = subprocess.run(cmd, input=stdin_bytes, capture_output=True, timeout=timeout)
        if proc.returncode == 0:
            return proc
    except subprocess.TimeoutExpired:
        proc = None
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
    restart = _litellm_ssh(f"systemctl restart {LLMPROXY_SERVICE}", timeout=60)
    if restart is None or restart.returncode != 0:
        raise AuthError(5, "llmproxy auth written but the service restart failed — check "
                           f"{LLMPROXY_SERVICE} on {LLMPROXY_SSH_HOST}")
    healthy = False
    for _ in range(6):
        time.sleep(2)
        probe = _litellm_ssh("python3 -c \"import urllib.request;print(urllib.request.urlopen("
                             "'http://127.0.0.1:4000/health',timeout=5).status)\"",
                             timeout=20)
        if probe is not None and probe.returncode == 0 and b"200" in probe.stdout:
            healthy = True
            break
    stamp_provenance(harness, slug, writer="litellm", source="litellm-switch",
                     writer_updated_at=time.time())
    return {"litellm_switched": slug, "healthy": healthy, "note": "litellm owns this lineage now; re-adopt after its refreshes"}




def cmd_give(args):
    """Export a stored profile VERBATIM to an authorized consumer host over
    ssh (owner 2026-10-25: llmproxy is INTERNAL — yggterm carries no llmproxy
    config knowledge; ygg-auth hands the auth to `ssh llmproxy`, and the
    llmproxy project picks it up inside its own container). The copy lands at
    <host>:~/.yggterm/auth-given/<harness>/<slug>.json (0600) unless --dest;
    provenance records the give. The consumer owns what happens next."""
    harness = require_harness(args.harness)
    if not args.host or not args.slug:
        raise AuthError(6, "give needs <host> <slug>")
    slug = slug_for(args.slug)
    store_file = os.path.join(store_dir(harness), f"{slug}.json")
    if not os.path.exists(store_file):
        raise AuthError(6, f"no stored profile {slug!r} for {harness}")
    record = read_json(store_file)
    dest = args.dest or f"~/.yggterm/auth-given/{harness}/{slug}.json"
    payload = (json.dumps(record, indent=2) + "\n").encode()
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", args.host,
           f"mkdir -p ~/.yggterm/auth-given/{harness} && "
           f"chmod 700 ~/.yggterm/auth-given ~/.yggterm/auth-given/{harness} 2>/dev/null; "
           f"cat > {dest}.tmp && chmod 600 {dest}.tmp && mv {dest}.tmp {dest}"]
    proc = subprocess.run(cmd, input=payload, capture_output=True, timeout=30)
    if proc.returncode != 0:
        raise AuthError(5, f"give to {args.host} failed: {proc.stderr.decode('utf-8', 'replace').strip()[:160]}")
    stamp_provenance(harness, slug, record=record, writer=HOSTNAME, source="give",
                     given_to=args.host, writer_updated_at=time.time())
    return {"gave": slug, "harness": harness, "to": args.host, "dest": dest}


def refresh_profile(harness, slug, record):
    try:
        fresh = refresh_grant(record)
    except AuthError as exc:
        if exc.details.get("reason") == "refresh_invalid" and slug:
            record_failure(harness, slug, "refresh_invalid")
        raise
    return fresh


def cmd_refresh(args):

    require_token_harness(require_harness(args.harness), "refresh")

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
            fresh = refresh_profile(harness, slug, record)
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
        slug = active_slug(harness, record)
        writer = get_writer(harness, slug) if slug else None
        if writer not in (None, "unknown", HOSTNAME) and not args.force:
            raise AuthError(5, "live lineage owned by another writer; refresh there")
        if slug:
            capture(harness, name=slug, lock_held=True)
        fresh = refresh_profile(harness, slug, record)
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
        state = ("EXPIRED" if live["expired"] else
                 "clock current" if live.get("expires_at") else "expiry unknown")
        lines.append(f"live: {live['email']} ({live['plan']}) account {live['account_id']} — {state}")
    else:
        lines.append("live: (no auth file — run `login` or `switch`)")
    lines.append(f"native auth health: {status.get('native_health', 'unknown')}; quota: unmeasured")
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
            elif not p.get("expires_at"):
                marks.append("expiry unknown")
            marks.append("health " + ((p.get("auth_health") or {}).get("state") or "unknown"))
            suffix = ("  " + " ".join(marks)) if marks else ""
            lines.append(f"  {p['slug']:<40} {p.get('plan', 'unknown'):<10}{suffix}")
    else:
        lines.append("profiles: (none stored — `capture` the current account or `login` a new one)")
    return "\n".join(lines)


def render_fields(value, indent=0):
    """Readable metadata, using the same sanitized result as --json."""
    pad = " " * indent
    def scalar(item):
        if item is None:
            return "unknown"
        if isinstance(item, bool):
            return "yes" if item else "no"
        return str(item)
    lines = []
    if isinstance(value, dict):
        for key, item in value.items():
            label = str(key).replace("_", " ")
            if isinstance(item, (dict, list)) and item:
                lines.append(f"{pad}{label}:")
                lines.extend(render_fields(item, indent + 2))
            else:
                text = "none" if isinstance(item, (dict, list)) else scalar(item)
                lines.append(f"{pad}{label}: {text}")
    elif isinstance(value, list):
        for item in value:
            if isinstance(item, (dict, list)):
                lines.extend(render_fields(item, indent + 2))
                lines.append("")
            else:
                lines.append(f"{pad}- {scalar(item)}")
    else:
        lines.append(pad + scalar(value))
    return lines


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="ygg-auth.py",
        description="Fleet auth-rotation plane: fast multi-account switching for harness CLIs.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Refresh grants never leave their writer host; readers hold access tokens only. Output never prints tokens.")
    parser.add_argument("--harness", default="codex", help="target harness (default codex)")
    # Subparsers re-declare --harness with a SUPPRESS default so it may sit on
    # either side of the verb (`--harness claude capture` AND `capture --harness
    # claude`) without the sub-default clobbering the top-level value.
    sub_parent = argparse.ArgumentParser(add_help=False)
    sub_parent.add_argument("--harness", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
    sub = parser.add_subparsers(dest="verb", required=True)

    p = sub.add_parser("sync", parents=[sub_parent],
                       help="keep this host's lineages alive: readers pull access, writers refresh when due")
    p.add_argument("--slug", help="limit to one account (default: every account on this host)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_sync)

    p = sub.add_parser("access", parents=[sub_parent],
                       help="writer side of a reader pull (machine verb; emits access tokens on stdout only)")
    p.add_argument("--slug", required=True)
    p.add_argument("--for-reader", action="store_true")
    p.set_defaults(func=cmd_access)

    p = sub.add_parser("evidence", parents=[sub_parent],
                       help="what this host holds for one account: kinds and times, never tokens")
    p.add_argument("--slug", required=True)
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_evidence)

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

    p = sub.add_parser("reconcile", parents=[sub_parent], help="capture a newer native login without changing the active account")
    p.add_argument("--fleet", action="store_true", help="ask enrolled peers to pull this profile into their vaults; never activate")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_reconcile)

    p = sub.add_parser("switch", parents=[sub_parent], help="switch to a stored profile (capture-on-leave)")
    p.add_argument("slug", nargs="?", help="profile slug or account email from `list`")
    p.add_argument("--json", action="store_true")
    p.add_argument("--force", action="store_true",
                   help="override the deterministic home assignment (owner)")
    p.set_defaults(func=cmd_switch)

    p = sub.add_parser("rotate", parents=[sub_parent], help="switch to the freshest profile — the rate-limit verb")
    p.add_argument("--fast", action="store_true",
                   help="skip usage measurement; rank by slug order + cooldowns only")
    p.add_argument("--cooldown-minutes", type=int, default=DEFAULT_COOLDOWN_MINUTES,
                   help=f"put the vacated account on cooldown (default {DEFAULT_COOLDOWN_MINUTES}; 0 disables)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_rotate)

    p = sub.add_parser("policy", parents=[sub_parent], help="inspect validated local rotation policy")
    p.add_argument("action", choices=["status"])
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_policy)

    p = sub.add_parser("fallback", parents=[sub_parent], help="bounded typed-error activation for managed consumers")
    p.add_argument("--error-code", choices=TYPED_FAILURES, required=True)
    p.add_argument("--event-id", required=True)
    p.add_argument("--expected-active-hash", required=True)
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_fallback)

    p = sub.add_parser("exec", parents=[sub_parent],
                       help="preflight the bound grant, then launch the native argv under it")
    p.add_argument("--preflight-only", action="store_true",
                   help="bounded read: print the activation receipt, spawn nothing, mutate nothing")
    p.add_argument("--measure", action="store_true",
                   help="opt-in quota measurement for the receipt (codex only; one usage read)")
    p.add_argument("--typed-failure", choices=TYPED_FAILURES, default=None,
                   help="adapter-observed typed pre-turn failure; triggers ONE policy-authorized fallback")
    p.add_argument("--event-id", default=None,
                   help="bounded event id for --typed-failure (dedup/no-replay)")
    p.add_argument("--observed-active-hash", default=None,
                   help="the active record digest the adapter observed failing (CAS)")
    p.add_argument("native_argv", nargs=argparse.REMAINDER,
                   help="native argv after `--` (kept verbatim: spaces, quotes, flags)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_exec)

    p = sub.add_parser("login", parents=[sub_parent], help="device-code OAuth into a NEW account (codex)")
    p.add_argument("--force", action="store_true",
                   help="owner override: sign in even when the fleet shows a live lineage")
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

    p = sub.add_parser("give", parents=[sub_parent],
                       help="export a stored profile verbatim to a consumer host (llmproxy et al)")
    p.add_argument("host", help="ssh destination that may consume the auth")
    p.add_argument("slug", help="profile slug")
    p.add_argument("--dest", default=None, help="remote path (default ~/.yggterm/auth-given/<harness>/<slug>.json)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_give)

    p = sub.add_parser("usage", parents=[sub_parent], help="measured rate-limit state per profile")
    p.add_argument("--slug", help="one profile instead of all")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_usage)

    p = sub.add_parser("redeem", parents=[sub_parent],
                       help="redeem the account's banked rate-limit-reset credit")
    p.add_argument("slug", nargs="?", help="stored profile (default: the live account)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_redeem)

    p = sub.add_parser("assign", parents=[sub_parent],
                       help="assign an account its deterministic home host")
    p.add_argument("slug", nargs="?")
    p.add_argument("host", nargs="?")
    p.add_argument("--clear", action="store_true")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_assign)

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
        if args.json:
            print(json.dumps({"schema_version": SCHEMA_VERSION, "outcome": "error", "harness": args.harness,
                              "error": {"code": exc.code, "message": str(exc), **exc.details}}))
        else:
            print(f"⛔ {exc}", file=sys.stderr)
        return exc.code
    if args.json:
        print(json.dumps(out, indent=2))
    else:
        if args.verb == "status":
            if args.harness == "all":
                print("Auth status — registered adapters")
                for name, status in out.get("harnesses", {}).items():
                    print(f"\n{name}")
                    print("\n".join("  " + line for line in render_table(status).splitlines()))
                print(f"\nCoverage: {out.get('coverage', 'unknown')}")
            else:
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
            if out.get("switched_to") is not None:  # the discriminator that actually exists on the activated path
                idt = out["identity"]
                print(f"📡 fetched {out['switched_to']} from {out['fetched_from']} and activated: {idt['email']} ({idt['plan']})")
            else:
                print(f"📡 fetched {out['slug']} from {out['fetched_from']} — stored, not live (use `switch` or `--activate`)")
        elif args.verb == "access":
            pass  # cmd_access already wrote the bundle to stdout
        elif args.verb == "sync":
            for row in out["lineages"]:
                note = row.get("message") or row.get("note") or ""
                print(f"  {row['slug']:<38} {row['state']:<18} {(row.get('writer') or ''):<12} {note}")
            if not out["lineages"]:
                print("  no lineages on this host")
        elif args.verb == "evidence":
            print(json.dumps(out, indent=2))
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
                if u.get("limit_reached"):
                    flag = "⛔ LIMIT"
                elif u.get("banked"):
                    flag = f"⭐ BANKED {u.get('banked_weekly_left', 0)}%"
                elif (u.get("secondary") or {}).get("used_percent", 0) >= 100:
                    flag = "⏳ WALLED"
                elif u.get("banked"):
                    flag = "\U0001F381 BANKED RESET"
                else:
                    flag = "ok"
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
        elif args.verb == "exec" and args.preflight_only:
            print(f"✅ preflight ready: {out['harness']} bound to {out['active_binding']} "
                  f"({out['provider']}), record {out['grant_binding']['record_hash']}; "
                  f"meter {out['meter']['state']}; native_health unknown until the child proves its auth")
        elif args.verb == "exec":
            pass  # the process was replaced by the native argv; nothing of ours may print
        elif args.verb == "litellm-switch":
            health = "healthy" if out["healthy"] else "⚠ health check did not clear — check the container"
            print(f"🛰  litellm now runs {out['litellm_switched']} ({health}); lineage lives in litellm — re-adopt after its refreshes")
        else:
            print("Auth " + args.verb.replace("_", " "))
            print("\n".join(render_fields(out)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
