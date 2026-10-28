#!/usr/bin/env python3
"""ygg-auth daemon teardown: a codex switch must reach the RUNNING daemon, not just the file.

    python3 tests/test_ygg_auth_daemon_teardown.py [--script <path to ygg-auth.py>]

WHAT THIS SCREENS FOR. codex runs a managed `codex app-server` daemon per
CODEX_HOME and every TUI delegates auth to it; the daemon reads auth.json ONCE
at startup (measured on dev 2026-10-28: `ygg auth switch` reported success,
the on-disk file held the new account, and the owner's TUI kept serving the
OLD one for hours). Screens, all black-box against the script:

  • Daemon reach. A switch with a daemon bound to the live home must tear the
    daemon down and say so. Screen: fake daemon (cmdline contains codex +
    app-server, CODEX_HOME = sandbox home) is dead after switch, and the
    switch output names it in daemons_stopped.
  • Relocated homes are safe. A daemon on a DIFFERENT CODEX_HOME (the
    yggterm served-codex shape) must survive the switch untouched.
  • TUIs are safe. A `codex resume` process on the SAME home must survive —
    the teardown family is app-server/code-mode-host, never the owner's
    live sessions.

Isolation is by $HOME + YGG_AUTH_HOME + CODEX_HOME (temporary dirs), matching
test_ygg_auth_rotation.py: the fake daemons' CODEX_HOME points inside the
sandbox, so the detector can only ever match what this test spawned.

⭐ FALSIFIED, not merely passed: the reach screen fails against a switch that
only swaps the file (daemon survives), the relocated-home and TUI screens
fail against any teardown that matches on cmdline alone.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time

PASS = []
FAIL = []


def b64u(obj):
    import base64
    return base64.urlsafe_b64encode(json.dumps(obj).encode()).rstrip(b"=").decode()


def jwt(claims):
    return f"{b64u({'alg': 'none', 'typ': 'JWT'})}.{b64u(claims)}.fake-sig"


def make_record(email, account_id, plan="plus", refresh_epoch=0):
    now = int(time.time()) + refresh_epoch * 1000
    return {
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": None,
        "tokens": {
            "id_token": jwt({"email": email, "name": email.split("@")[0],
                             "https://api.openai.com/auth": {"chatgpt_account_id": account_id,
                                                             "chatgpt_plan_type": plan}}),
            "access_token": jwt({"exp": now + 86400,
                                 "https://api.openai.com/auth": {"chatgpt_account_id": account_id}}),
            "refresh_token": f"rt-{account_id}-{refresh_epoch}",
            "account_id": account_id,
        },
        "last_refresh": f"2026-09-20T0{refresh_epoch % 10}:00:00.000000000Z",
    }


class Sandbox:
    def __init__(self, script):
        self.tmp = tempfile.mkdtemp(prefix="ygg-auth-daemon-test-")
        self.home = os.path.join(self.tmp, "home")
        self.store = os.path.join(self.tmp, "store")
        self.codex = os.path.join(self.home, ".codex")
        os.makedirs(self.codex)
        self.script = script
        self.spawns = []

    def env(self):
        e = dict(os.environ)
        # YGG_AUTH_NO_PREFRESH: these screens test teardown, not token
        # refresh — keep them hermetic and off the network.
        e.update({"HOME": self.home, "YGG_AUTH_HOME": self.store,
                  "CODEX_HOME": self.codex, "YGG_AUTH_NO_PREFRESH": "1"})
        return e

    def run(self, *args):
        return subprocess.run([sys.executable, self.script, *args],
                              env=self.env(), capture_output=True, text=True, timeout=120)

    def live_path(self):
        return os.path.join(self.codex, "auth.json")

    def write_live(self, record):
        with open(self.live_path(), "w") as f:
            json.dump(record, f, indent=2)
        os.chmod(self.live_path(), 0o600)

    def read_live(self):
        with open(self.live_path()) as f:
            return json.load(f)

    def spawn_fake(self, subcommand_tail, codex_home, ignore_sigterm=False, home=None):
        """A process the daemon detector can see: /proc cmdline is exactly
        [codex-binary, *subcommand_tail] — argv[0] a codex-looking path,
        argv[1] the subcommand (a script file NAMED as the subcommand runs
        under executable=python, so no -c noise lands in the subcommand
        slot). environ: HOME=<home|self.home>, CODEX_HOME=codex_home; None
        means unset (the daemon then resolves its home from its own HOME).
        ignore_sigterm spawns a SIGTERM-resistant daemon (exercises the
        SIGKILL escalation)."""
        e = dict(os.environ)
        e.update({"HOME": home or self.home})
        if codex_home is not None:
            e["CODEX_HOME"] = codex_home
        else:
            e.pop("CODEX_HOME", None)
        if ignore_sigterm:
            body = "import signal, time\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\ntime.sleep(300)"
        else:
            body = "import time; time.sleep(300)"
        with open(os.path.join(self.tmp, subcommand_tail[0]), "w") as f:
            f.write(body + "\n")
        codex_argv0 = os.path.join(self.tmp, "vendored", "bin", "codex")
        proc = subprocess.Popen([codex_argv0, *subcommand_tail], executable=sys.executable,
                                cwd=self.tmp, env=e, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.spawns.append(proc)
        deadline = time.time() + 5
        while time.time() < deadline:
            try:
                with open(f"/proc/{proc.pid}/environ", "rb"):
                    return proc  # visible in /proc, env readable
            except OSError:
                time.sleep(0.05)
        raise AssertionError(f"fake process {proc.pid} never became visible in /proc")

    def alive(self, proc):
        return proc.poll() is None

    def cleanup(self):
        import signal
        for proc in self.spawns:
            if proc.poll() is None:
                try:
                    proc.send_signal(signal.SIGTERM)
                except ProcessLookupError:
                    pass
        for proc in self.spawns:
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()


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


def two_profiles():
    """a@x.io captured, b@x.io live — the standard switch setup."""
    S.write_live(make_record("a@x.io", "acct-a"))
    assert S.run("capture").returncode == 0
    S.write_live(make_record("b@x.io", "acct-b"))
    assert S.run("capture").returncode == 0
    S.write_live(make_record("b@x.io", "acct-b"))


@screen("switch tears down a daemon bound to the live home and names it")
def s_daemon_reach():
    two_profiles()
    daemon = S.spawn_fake(("app-server",), S.codex)
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    assert S.read_live()["tokens"]["account_id"] == "acct-a", "live is not the target account"
    deadline = time.time() + 5
    while S.alive(daemon) and time.time() < deadline:
        time.sleep(0.1)
    assert not S.alive(daemon), "daemon bound to the live home survived the switch"
    out = json.loads(r.stdout)
    assert daemon.pid in out.get("daemons_stopped", []), \
        f"daemons_stopped {out.get('daemons_stopped')} does not name the daemon pid {daemon.pid}"
    assert "stopped" in r.stderr, f"stderr carried no teardown note: {r.stderr!r}"


@screen("a daemon on a different CODEX_HOME survives the switch untouched")
def s_relocated_home_safe():
    other = os.path.join(S.tmp, "other-codex-home")
    os.makedirs(other)
    S.write_live(make_record("b@x.io", "acct-b"))
    relocated = S.spawn_fake(("app-server",), other)
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    time.sleep(0.5)
    assert S.alive(relocated), "switch killed a daemon belonging to a DIFFERENT codex home"
    out = json.loads(r.stdout)
    assert relocated.pid not in out.get("daemons_stopped", []), "relocated daemon reported as stopped"


@screen("a codex TUI process on the same home survives the switch")
def s_tui_safe():
    S.write_live(make_record("b@x.io", "acct-b"))
    tui = S.spawn_fake(("resume",), S.codex)
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    time.sleep(0.5)
    assert S.alive(tui), "switch killed a codex TUI process — teardown matched the wrong family"
    out = json.loads(r.stdout)
    assert tui.pid not in out.get("daemons_stopped", []), "TUI reported as a stopped daemon"


@screen("switch with no daemon running is unchanged and reports none stopped")
def s_no_daemon_noop():
    S.write_live(make_record("b@x.io", "acct-b"))
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    assert S.read_live()["tokens"]["account_id"] == "acct-a"
    out = json.loads(r.stdout)
    assert out.get("daemons_stopped") == [], \
        f"phantom daemons reported: {out.get('daemons_stopped')} (stderr: {r.stderr!r})"
    assert "stopped" not in r.stderr, f"teardown note without a teardown: {r.stderr!r}"


@screen("a daemon with CODEX_HOME unset and a foreign HOME survives the switch")
def s_default_home_is_the_daemons_own():
    """The default-home trap: 'unset CODEX_HOME' resolves against the
    DAEMON's HOME, not the switch process's expanduser. Comparing both
    defaults with one expanduser once matched — and killed — a real host
    daemon (HOME=/home/pi, CODEX_HOME unset) from a sandboxed switch whose
    own HOME made expanduser point at the sandbox."""
    S.write_live(make_record("b@x.io", "acct-b"))
    foreign_home = os.path.join(S.tmp, "foreign-home")
    os.makedirs(foreign_home)
    # CODEX_HOME unset + HOME=foreign-home: this daemon serves
    # foreign-home/.codex, never the sandbox codex dir the switch targets.
    foreign_daemon = S.spawn_fake(("app-server",), None, home=foreign_home)
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    time.sleep(0.5)
    assert S.alive(foreign_daemon), \
        f"switch killed a daemon whose CODEX_HOME was merely unset (stderr: {r.stderr!r})"
    out = json.loads(r.stdout)
    assert foreign_daemon.pid not in out.get("daemons_stopped", []), "default-home daemon reported as stopped"


@screen("a TUI whose PROMPT TEXT says app-server survives the switch")
def s_prompt_text_safe():
    """Substring matching once matched `codex resume 'Please explain
    app-server'` — the owner's live session with an unlucky prompt. The
    daemon family must match as exact ARGV TOKENS after a codex argv[0]."""
    S.write_live(make_record("b@x.io", "acct-b"))
    tui = S.spawn_fake(("resume", "Please explain app-server"), S.codex)
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    time.sleep(0.5)
    assert S.alive(tui), "switch killed a TUI whose prompt text mentioned app-server"
    out = json.loads(r.stdout)
    assert tui.pid not in out.get("daemons_stopped", []), "prompt-text TUI reported as a stopped daemon"


@screen("a SIGTERM-resistant daemon still dies via SIGKILL and is named")
def s_sigkill_escalation():
    S.write_live(make_record("b@x.io", "acct-b"))
    stubborn = S.spawn_fake(("app-server",), S.codex, ignore_sigterm=True)
    r = S.run("switch", "a@x.io", "--json")
    assert r.returncode == 0, f"switch exit {r.returncode}: {r.stderr}"
    deadline = time.time() + 8
    while S.alive(stubborn) and time.time() < deadline:
        time.sleep(0.1)
    assert not S.alive(stubborn), "SIGTERM-resistant daemon survived the SIGKILL escalation"
    out = json.loads(r.stdout)
    assert stubborn.pid in out.get("daemons_stopped", []), "escalated kill not reported as stopped"


@screen("a stale flush during the post-swap sweep is detected and repaired")
def s_post_sweep_repair():
    """White-box (importlib): inject a sweep that flushes the OLD record
    after the swap — exactly what a dying daemon can do. switch() must
    re-verify post-sweep and repair from the in-store record instead of
    reporting success over a stale file."""
    import importlib.util
    two_profiles()
    stale = make_record("b@x.io", "acct-b")
    e = S.env()
    for key in ("HOME", "YGG_AUTH_HOME", "CODEX_HOME", "YGG_AUTH_NO_PREFRESH"):
        os.environ[key] = e[key]  # STORE_ROOT resolves at module import time
    try:
        spec = importlib.util.spec_from_file_location("yggauth_wb", os.path.abspath(S.script))
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)

        real_stop = m.stop_codex_daemons
        def fake_stop(home, grace_seconds=3.0):
            calls = getattr(fake_stop, "calls", 0) + 1
            fake_stop.calls = calls
            if calls >= 2:  # the post-swap sweep: a dying daemon flushes stale tokens
                with open(S.live_path(), "w") as f:
                    json.dump(stale, f)
                return [999999]
            return []
        m.stop_codex_daemons = fake_stop
        result = m.switch("codex", "a@x.io", lock_held=True)
        assert result["switched_to"] == "a_x.io"
        assert S.read_live()["tokens"]["account_id"] == "acct-a", \
            "switch reported success over a stale file — post-sweep repair missing"
        assert 999999 in result["daemons_stopped"]
    finally:
        m.stop_codex_daemons = real_stop
        for key in ("HOME", "YGG_AUTH_HOME", "CODEX_HOME", "YGG_AUTH_NO_PREFRESH"):
            os.environ.pop(key, None)


def main():
    global S
    parser = argparse.ArgumentParser()
    parser.add_argument("--script", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), os.pardir, "ygg-auth.py"))
    args = parser.parse_args()
    S = Sandbox(os.path.abspath(args.script))

    print("ygg-auth daemon teardown screens:")
    try:
        s_daemon_reach()
        s_relocated_home_safe()
        s_tui_safe()
        s_prompt_text_safe()
        s_sigkill_escalation()
        s_default_home_is_the_daemons_own()
        s_post_sweep_repair()
        s_no_daemon_noop()
    finally:
        S.cleanup()

    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        for name, why in FAIL:
            print(f"  ⛔ {name}: {why}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
