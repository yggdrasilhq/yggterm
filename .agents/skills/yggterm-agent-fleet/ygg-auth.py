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
HOSTNAME = socket.gethostname()
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
    record = live_record(harness, required=True)
    ident = identity_of(record, harness)
    if not ident["slug"] and not name:
        raise AuthError(3, "live record carries no usable identity; pass --name to store it anyway")
    slug = slug_for(name or ident["slug"])
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


def switch(harness, slug, lock_held=False, expected_active_hash=None):
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
    if os.path.exists(live_path):
        swapped = False
        for _ in range(3):
            before = _live_fingerprint(live_path)
            current = live_record(harness, required=True)
            if expected_active_hash is not None and record_hash(current) != expected_active_hash:
                raise AuthError(5, "active auth changed during fallback; refusing replacement")
            current_ident = identity_of(current, harness)
            outgoing = active_slug(harness, current) or ("unbound-" + record_hash(current)[:12])
            if record_hash(current) != record_hash(target_record):
                captured = _capture_locked(harness, current, outgoing,
                                           os.path.join(store_dir(harness), outgoing + ".json"), current_ident)
            if _live_fingerprint(live_path) != before:
                continue  # the harness rewrote mid-read; its tokens would be lost — redo
            atomic_write_json(live_path, target_record)
            swapped = True
            break
        if not swapped:
            raise AuthError(5, f"the live auth file kept changing during capture ({live_path}); not switching")
    else:
        atomic_write_json(live_path, target_record)

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
            print("⚠️  the live auth file drifted during daemon teardown — re-writing "
                  f"{slug}", file=sys.stderr)
            atomic_write_json(live_path, target_record)
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
    if home and home != HOSTNAME and not getattr(args, "force", False):
        raise AuthError(
            6, f"{slug}'s deterministic home is {home!r} — this host is {HOSTNAME!r}. "
               f"Run the switch there, reassign with `assign {slug} {HOSTNAME}`, or override with --force (owner).")
    with fleet_lock():
        daemons_stopped_pre = []
        if harness == "codex":
            # the refresh and the swap must both happen with no daemon alive
            # to race them or flush stale tokens over them; switch() re-runs
            # this as a cheap no-op for the rotate/fetch-activate paths
            daemons_stopped_pre = stop_codex_daemons(os.path.dirname(HARNESSES[harness]["auth_file"](harness)))
            if daemons_stopped_pre:
                print(f"⬇  stopped codex app-server daemon(s) {daemons_stopped_pre} — they cache auth "
                      f"at startup and would keep serving the previous account", file=sys.stderr)
        # deterministic hygiene: refresh the OUTGOING grant before capturing it,
        # so the stored copy holds the newest token, never a consumed one.
        # Single-writer guard: only refresh when THIS host owns the live lineage.
        if harness == "codex" and os.environ.get("YGG_AUTH_NO_PREFRESH") != "1":
            try:
                live_rec = live_record(harness, required=True)
                ident = identity_of(live_rec)
                writer = get_writer(harness, ident.get("slug") or "")
                if writer not in (None, "unknown", HOSTNAME):
                    print(f"⚠️  pre-switch refresh skipped: {ident.get('slug')}'s lineage is owned by "
                          f"{writer!r} — switching proceeds with the stored copy as-is",
                          file=sys.stderr)
                else:
                    fresh = refresh_grant(live_rec)
                    atomic_write_json(HARNESSES[harness]["auth_file"](harness), fresh)
            except Exception as exc:
                print(f"⚠️  pre-switch refresh skipped ({type(exc).__name__})", file=sys.stderr)
        result = switch(harness, args.slug, lock_held=True)
        if daemons_stopped_pre:
            result["daemons_stopped"] = daemons_stopped_pre + [
                pid for pid in result.get("daemons_stopped", []) if pid not in daemons_stopped_pre]
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


def meter_snapshot(harness, record, measure=False):
    """Identity-safe quota meter for the exec receipt. Measuring is explicit
    (--measure): it is the preflight's ONLY optional network touch and exists
    only for adapters with quota_measurement. Everything else stays unknown —
    an unknown meter is never reported as free."""
    if not measure:
        return {"measured": False, "state": "unknown"}
    if not capabilities(harness)["quota_measurement"]:
        return {"measured": False, "state": "unsupported"}
    snap = usage_snapshot(record)
    if not snap.get("ok"):
        return {"measured": False, "state": "unknown",
                "error_class": "http" if snap.get("http_status") else "unavailable"}

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
    atomic_write_json(HARNESSES[harness]["auth_file"](harness), fresh)
    atomic_write_json(os.path.join(store_dir(harness), slug + ".json"), fresh)
    stamp_provenance(harness, slug, record=fresh, source="exec-preflight-refresh",
                     writer_updated_at=time.time())
    record = live_record(harness, required=True)
    gate.update(record_hash=record_hash(record), expired=False, refreshed_once=True,
                health_state="unknown")
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
    fallback_result = None
    if args.typed_failure:
        # ONE policy-authorized fallback for the adapter's typed pre-turn
        # failure. The canonical verb enforces the policy gate, event dedup,
        # the observed-hash CAS and never replays the failed request.
        fallback_result = cmd_fallback(argparse.Namespace(
            harness=harness, error_code=args.typed_failure, event_id=args.event_id,
            expected_active_hash=args.observed_active_hash, json=False))
    with fleet_lock():
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
    policy = rotation_policy(harness)
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
                       "fallback": bool(fallback_result)},
        "policy": {"mode": policy["mode"], "automatic_fallback": policy["automatic_fallback"],
                   "source_authority": policy["source_authority"]},
        "meter": meter,
        "native_health": "unknown",
        "actual_effect": "unknown",
        "session_action": "child_native_auth_proof_required",
        "native_exec": {"spawned": not args.preflight_only, "argv_len": len(native_argv)},
    }
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
    require_token_harness(require_harness(args.harness), "import")

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

    ident = identity_of(record, harness)
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
            # "activated": True — dream ACK-0b31f4f3f5: without it the success
            # printer fell into the stored-not-live branch and KeyError'd on
            # 'slug' AFTER a fully successful fetch+switch (exit 1 on success).
            return {**switch(harness, slug, lock_held=True), "fetched_from": used_dest,
                    "activated": True}
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
    return {"ok": True,
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
            lines.append(f"  {p['slug']:<40} {p.get('plan', 'unknown'):<10}{suffix}")
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
            print(json.dumps(out, indent=2) if args.harness == "all" else render_table(out))
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
            print(json.dumps(out, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
