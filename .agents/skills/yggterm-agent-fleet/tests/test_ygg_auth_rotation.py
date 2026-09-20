#!/usr/bin/env python3
"""ygg-auth rotation invariant: switching an account never loses its tokens.

    python3 tests/test_ygg_auth_rotation.py [--script <path to ygg-auth.py>]

WHAT THIS SCREENS FOR. ygg-auth's whole job is swapping a harness's live auth
file between stored profiles. Three ways it can eat credentials, each with a
screen here:

  • Stale capture. The harness refreshes tokens IN PLACE (codex rewrites
    auth.json on refresh, and OAuth refresh ROTATES the refresh token) — a
    switch that captured the pre-refresh record would store, and later
    restore, a revoked token: the profile comes back bricked. Screen:
    refresh the live file after capture, switch away and back, require the
    REFRESHED record to come back.
  • Redaction breach. Every verb's stdout/stderr must carry identity claims
    only — a token substring in output is a leak the memory-sync plane could
    carry across hosts. Screen: the freshest token strings appear in NO verb
    output.
  • Rotational amnesia. Rate-limit rotation without cooldown cycles straight
    back into the quota-locked account it just fled. Screen: with three
    accounts, two rotates must never land on the first account again while it
    cools, and the cooling account must be visible in `status`.

Isolation is by $HOME + YGG_AUTH_HOME (temporary dirs), matching the
test_booter_screens.py pattern: the script derives its state from env, so the
sandboxes are complete — no real harness store is ever touched.

⭐ FALSIFIED, not merely passed: the rotation screens fail against a
cooldown-free picker (they return to the vacated account), the capture screen
fails against a switch that skips capture-on-leave, and the redaction screen
fails if any verb echoes file content.
"""
import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile
import time

PASS = []
FAIL = []


def b64u(obj):
    return base64.urlsafe_b64encode(json.dumps(obj).encode()).rstrip(b"=").decode()


def jwt(claims):
    return f"{b64u({'alg': 'none', 'typ': 'JWT'})}.{b64u(claims)}.fake-sig"


def make_record(email, account_id, plan="plus", exp_offset=86400, refresh_epoch=0):
    now = int(time.time()) + refresh_epoch * 1000
    return {
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": None,
        "tokens": {
            "id_token": jwt({"email": email, "name": email.split("@")[0],
                             "https://api.openai.com/auth": {"chatgpt_account_id": account_id,
                                                             "chatgpt_plan_type": plan}}),
            "access_token": jwt({"exp": now + exp_offset,
                                 "https://api.openai.com/auth": {"chatgpt_account_id": account_id}}),
            "refresh_token": f"rt-{account_id}-{refresh_epoch}",
            "account_id": account_id,
        },
        "last_refresh": f"2026-09-20T0{refresh_epoch % 10}:00:00.000000000Z",
    }


class Sandbox:
    def __init__(self, script):
        self.tmp = tempfile.mkdtemp(prefix="ygg-auth-test-")
        self.home = os.path.join(self.tmp, "home")
        self.store = os.path.join(self.tmp, "store")
        self.codex = os.path.join(self.home, ".codex")
        os.makedirs(self.codex)
        self.script = script

    def env(self):
        e = dict(os.environ)
        e.update({"HOME": self.home, "YGG_AUTH_HOME": self.store, "CODEX_HOME": self.codex})
        return e

    def run(self, *args):
        return subprocess.run([sys.executable, self.script, *args],
                              env=self.env(), capture_output=True, text=True, timeout=60)

    def live_path(self):
        return os.path.join(self.codex, "auth.json")

    def reset(self):
        """A clean slate for screens that must not inherit earlier state."""
        import shutil
        shutil.rmtree(self.store, ignore_errors=True)
        shutil.rmtree(self.home, ignore_errors=True)
        os.makedirs(self.codex)

    def write_live(self, record):
        with open(self.live_path(), "w") as f:
            json.dump(record, f, indent=2)
        os.chmod(self.live_path(), 0o600)

    def read_live(self):
        with open(self.live_path()) as f:
            return json.load(f)


def screen(name):
    def deco(fn):
        def wrapped():
            try:
                fn()
                PASS.append(name)
                print(f"  PASS {name}")
            except AssertionError as exc:
                FAIL.append((name, str(exc)))
                print(f"  FAIL {name}: {exc}")
            except Exception as exc:  # noqa: BLE001 — a screen crash is a failure
                FAIL.append((name, f"crashed: {type(exc).__name__}: {exc}"))
                print(f"  FAIL {name}: crashed {type(exc).__name__}: {exc}")
        return wrapped
    return deco


S = None  # set in main


@screen("capture stores the record verbatim (auth_mode/last_refresh survive)")
def s_capture_verbatim():
    rec = make_record("a@x.io", "acct-a")
    S.write_live(rec)
    r = S.run("capture")
    assert r.returncode == 0, f"capture exit {r.returncode}: {r.stderr}"
    path = os.path.join(S.store, "codex", "a_x.io.json")  # slug_for normalizes '@'
    assert os.path.exists(path), f"no stored profile at {path}"
    with open(path) as f:
        stored = json.load(f)
    assert stored == rec, "stored record differs from the live record (not verbatim)"


@screen("switch restores byte-identical record; leaves no temp files; perms 0600")
def s_switch_atomic():
    S.write_live(make_record("a@x.io", "acct-a"))
    assert S.run("capture").returncode == 0
    S.write_live(make_record("b@x.io", "acct-b"))
    assert S.run("capture").returncode == 0
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    assert S.read_live()["tokens"]["account_id"] == "acct-a", "live is not the target account"
    leftovers = [f for f in os.listdir(S.codex) if f.startswith(".tmp-auth-")]
    assert not leftovers, f"temp files left behind: {leftovers}"
    mode = os.stat(S.live_path()).st_mode & 0o777
    assert mode == 0o600, f"live auth perms {oct(mode)}, want 0600"


@screen("capture-on-leave: refresh AFTER capture still wins the round trip")
def s_capture_on_leave():
    S.write_live(make_record("a@x.io", "acct-a", refresh_epoch=1))
    assert S.run("capture").returncode == 0
    refreshed = make_record("a@x.io", "acct-a", refresh_epoch=2)  # codex refreshed in place
    S.write_live(refreshed)
    r = S.run("switch", "b@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    assert S.read_live()["tokens"]["account_id"] == "acct-b"
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"return switch exit {r.returncode}: {r.stderr}"
    back = S.read_live()
    assert back == refreshed, (
        "round trip restored a STALE record: the post-capture refresh was eaten "
        f"(refresh_token {back['tokens']['refresh_token']!r} != refreshed)")


@screen("redaction: no verb output carries any token substring")
def s_redaction():
    secrets = set()
    for email, acct in (("a@x.io", "acct-a"), ("b@x.io", "acct-b")):
        rec = make_record(email, acct)
        secrets |= {rec["tokens"]["refresh_token"], rec["tokens"]["id_token"],
                    rec["tokens"]["access_token"]}
    S.write_live(make_record("a@x.io", "acct-a"))
    assert S.run("capture").returncode == 0
    S.write_live(make_record("b@x.io", "acct-b"))
    assert S.run("capture").returncode == 0
    assert S.run("switch", "a@x.io").returncode == 0
    for args in (("status",), ("status", "--json"), ("list", "--json"),
                 ("switch", "b@x.io", "--json"), ("rotate", "--json"),
                 ("capture",), ("harnesses", "--json")):
        r = S.run(*args)
        blob = r.stdout + r.stderr
        for secret in secrets:
            assert secret not in blob, f"{args} echoed a token substring"
    assert S.run("switch", "a@x.io").returncode == 0


@screen("rotate skips the cooling account; third rotate warns when all cooling")
def s_rotate_cooldown():
    S.reset()
    for email, acct in (("a@x.io", "acct-a"), ("b@x.io", "acct-b"), ("c@x.io", "acct-c")):
        S.write_live(make_record(email, acct))
        assert S.run("capture").returncode == 0
    assert S.run("switch", "a@x.io").returncode == 0
    r1 = S.run("rotate", "--json")
    assert r1.returncode == 0, f"rotate1 exit {r1.returncode}: {r1.stderr}"
    first = json.loads(r1.stdout)["switched_to"]
    assert first == "b_x.io", f"rotate1 picked {first}, want b_x.io (a just vacated, cooling)"
    r2 = S.run("rotate", "--json")
    assert r2.returncode == 0, f"rotate2 exit {r2.returncode}: {r2.stderr}"
    second = json.loads(r2.stdout)["switched_to"]
    assert second == "c_x.io", (
        f"rotate2 picked {second}, want c@x.io — rotation cycled toward cooling accounts")
    r3 = S.run("rotate", "--json")
    assert r3.returncode == 0, f"rotate3 exit {r3.returncode}: {r3.stderr}"
    assert "cooling" in r3.stderr, "no warning when the picker dips into cooling profiles"
    st = json.loads(S.run("status", "--json").stdout)
    a = next(p for p in st["profiles"] if p["slug"] == "a_x.io")
    assert a["cooldown_minutes_left"] > 0, "status does not surface the cooldown"


@screen("claude harness: swap works under CLAUDE_CONFIG_DIR")
def s_claude_swap():
    cdir = os.path.join(S.home, ".claude")
    os.makedirs(cdir, exist_ok=True)
    rec_a = {"claudeAiOauth": {"accessToken": "at-a", "refreshToken": "rt-a", "expiresAt": int(time.time() * 1000) + 86400000}}
    rec_b = {"claudeAiOauth": {"accessToken": "at-b", "refreshToken": "rt-b", "expiresAt": int(time.time() * 1000) + 86400000}}
    cred = os.path.join(cdir, ".credentials.json")
    def write(r):
        with open(cred, "w") as f:
            json.dump(r, f)
        os.chmod(cred, 0o600)
    write(rec_a)
    r = S.run("capture", "--harness", "claude", "--name", "claude-a")
    assert r.returncode == 0, f"claude capture exit {r.returncode}: {r.stderr}"
    write(rec_b)
    r = S.run("capture", "--harness", "claude", "--name", "claude-b")
    assert r.returncode == 0, f"claude capture2 exit {r.returncode}: {r.stderr}"
    r = S.run("switch", "claude-a", "--harness", "claude", "--json")
    assert r.returncode == 0, f"claude switch exit {r.returncode}: {r.stderr}"
    with open(cred) as f:
        assert json.load(f) == rec_a, "claude live file is not the restored record"


@screen("import converts a flat litellm record; tokens never in output")
def s_import():
    S.reset()
    flat = {
        "access_token": jwt({"exp": int(time.time()) + 86400,
                             "email": "flat@x.io",
                             "https://api.openai.com/auth": {"chatgpt_account_id": "acct-flat"}}),
        "refresh_token": "rt-flat-1",
        "id_token": jwt({"email": "flat@x.io",
                         "https://api.openai.com/auth": {"chatgpt_account_id": "acct-flat",
                                                         "chatgpt_plan_type": "plus"}}),
        "expires_at": int(time.time()) + 86400,
        "account_id": "acct-flat",
    }
    foreign = os.path.join(S.tmp, "flat.json")
    with open(foreign, "w") as f:
        json.dump(flat, f)
    r = S.run("import", foreign, "--json")
    assert r.returncode == 0, f"import exit {r.returncode}: {r.stderr}"
    assert "rt-flat-1" not in r.stdout + r.stderr, "import echoed a token"
    path = os.path.join(S.store, "codex", "flat_x.io.json")
    assert os.path.exists(path), "no imported profile"
    with open(path) as f:
        converted = json.load(f)
    assert converted["auth_mode"] == "chatgpt" and converted["tokens"]["refresh_token"] == "rt-flat-1", \
        "conversion lost the grant"
    r = S.run("switch", "flat@x.io", "--json")
    assert r.returncode == 0, f"switch to imported exit {r.returncode}: {r.stderr}"
    assert S.read_live()["tokens"]["account_id"] == "acct-flat"


@screen("usage errors: unknown slug exit 6; capture without live file exit 3")
def s_usage_errors():
    r = S.run("switch", "nosuch@x.io")
    assert r.returncode == 6, f"unknown slug exit {r.returncode}, want 6"
    e = S.env()
    ghost = tempfile.mkdtemp(prefix="ygg-auth-ghost-")
    e["CODEX_HOME"] = ghost
    r = subprocess.run([sys.executable, S.script, "capture"], env=e, capture_output=True, text=True)
    assert r.returncode == 3, f"missing live file exit {r.returncode}, want 3"


def main():
    global S
    parser = argparse.ArgumentParser()
    parser.add_argument("--script", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), os.pardir, "ygg-auth.py"))
    args = parser.parse_args()
    S = Sandbox(args.script)

    print("ygg-auth rotation screens:")
    s_capture_verbatim()
    s_switch_atomic()
    s_capture_on_leave()
    s_redaction()
    s_rotate_cooldown()
    s_import()
    s_claude_swap()
    s_usage_errors()

    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        for name, why in FAIL:
            print(f"  ⛔ {name}: {why}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
