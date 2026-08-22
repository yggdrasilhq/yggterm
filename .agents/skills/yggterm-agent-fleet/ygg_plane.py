"""How a fleet verb REACHES the row plane — which binary, which home, which machine.

⛔⛔ **THE DEFECT THIS EXISTS TO END: four spellings of one question, none of them
aimable anywhere but the live desktop.** Measured 2026-08-22:

    ygg-deliver          "~/.local/bin/yggterm-headless"      via ssh
    ygg-spawn            expanduser("~/.local/bin/yggterm-headless")  via ssh
    ygg_host._probe      "~/.yggterm/bin/yggterm"             via ssh
    ygg_host._candidates "yggterm-headless"                   from $PATH

⚠ **The probe therefore tested a DIFFERENT BINARY from the one the callers went on
to use** — two separate installs, both present, kept in step by nothing. A host can
answer the probe and fail the call, and the caller reads that as a fact about the
row rather than about the transport, which is the exact confusion `ygg_host`'s own
header was written about.

⛔ **And none of them could be pointed at a sandbox.** The path was a module
constant and the home it resolved was always the real one; `--host` moves which
MACHINE answers, never which HOME. So every fleet verb that can destroy a row has
only ever been exercised against the live desktop — three defects in the reap alone
were found that way in one session, one of them by running it on somebody's real
plane. A verb trusted with a destructive decision should be provable somewhere
harmless first.

⇒ One owner. The binary comes from `$YGGTERM_BIN` or the install, the home from
`$YGGTERM_HOME`, and **a sandbox home means the plane is on THIS machine** — there
is no other kind of sandbox daemon, and ssh would not carry the variable anyway.
`YGGTERM_HOME` is unset in a normal agent row (checked before relying on it), so
its presence is an unambiguous signal rather than a guess.
"""
import json
import os
import socket
import subprocess

#: Overrides the binary a fleet verb drives the plane with. Set it to a sandbox
#: copy to exercise a verb without touching anyone's desktop.
BIN_ENV = "YGGTERM_BIN"
#: The isolated state plane. Present ⇒ the daemon is local and is not the fleet's.
HOME_ENV = "YGGTERM_HOME"

DEFAULT_BIN = "~/.local/bin/yggterm-headless"


def local_binary():
    """The binary that answers `server app` ON THIS MACHINE.

    ⭐ `YGGTERM_BIN` is not an override invented here — **the daemon exports its
    own executable into every PTY it owns**, so a row already knows which daemon
    it belongs to, and in a sandbox row that is the sandbox's binary. Same lesson
    as `session_id` and `machine_key`: the answer was already published.

    ⛔ **BUT IT CAN NAME A FILE THAT NO LONGER EXISTS.** It comes from
    `/proc/self/exe`, so on a hot-restarted daemon it reads `…/yggterm-headless
    (deleted)` and every caller that trusted it chased a path with no file behind
    it — documented in `docs/agent-control-plane.md` after it cost several lanes
    their ability to live-verify at all. ⇒ Validated here, with the install as the
    fallback, so the export is used when it is true and ignored when it is stale.
    """
    exported = (os.environ.get(BIN_ENV) or "").strip()
    if exported and os.path.isfile(exported) and os.access(exported, os.X_OK):
        return exported
    return os.path.expanduser(DEFAULT_BIN)


def remote_binary():
    """The binary to name on ANOTHER machine — the install path, unexpanded.

    ⛔ Never this process's `YGGTERM_BIN`: that is THIS daemon's executable, and
    naming it on another host asserts the two installs sit at the same path. They
    do here and that is luck, not a contract. `~` is left for the far shell to
    resolve, or a verb run from one account addresses another account's install.
    """
    return DEFAULT_BIN


def sandbox_home():
    """The isolated home this process is aimed at, or None for the fleet's own."""
    return (os.environ.get(HOME_ENV) or "").strip() or None


def runs_locally(host):
    """Is the plane on THIS machine? ⛔ A sandbox home is always local."""
    if sandbox_home():
        return True
    if not host:
        return True
    return host == socket.gethostname()


def stage(host, local_path):
    """Put a local file where the plane can read it, and return the path THERE.

    ⚠ A no-op locally. Remotely this is the step every caller open-coded, each
    with its own temp name — and one of those names was derived from a mis-sliced
    row id, so two rows collided on the same file. See `ygg_rowarg.row_session_id`.
    """
    if runs_locally(host):
        return local_path
    remote = f"/tmp/ygg-stage-{os.path.basename(local_path)}"
    subprocess.run(["scp", "-q", local_path, f"{host}:{remote}"], timeout=120)
    return remote


def app(host, argstr, stdin_path=None, timeout=180):
    """Run one `server app` verb against the resolved plane and decode its reply.

    ⛔ Returns `{"error": …}` rather than raising, because every caller here is a
    watchdog and an exception in one of them stops it supervising — the failure
    `ygg_host`'s header exists to prevent. **An `error` is a statement about the
    TRANSPORT, never about the row**, and callers must not read it as one.
    """
    if runs_locally(host):
        argv = [local_binary(), "server", "app"] + _split(argstr)
        env = dict(os.environ)
        done = _run(argv, env, stdin_path, timeout)
    else:
        cmd = f"{remote_binary()} server app {argstr}"
        if stdin_path:
            cmd += f" < {stdin_path}"
        done = _run(["ssh", "-n", host, cmd], None, None, timeout)
    if done is None:
        return {"error": "the plane did not answer within the timeout"}
    try:
        return json.loads(done.stdout)
    except Exception:
        return {"error": (done.stderr or done.stdout or "unparseable").strip()[:200]}


def _run(argv, env, stdin_path, timeout):
    try:
        if stdin_path:
            with open(stdin_path) as handle:
                return subprocess.run(argv, stdin=handle, capture_output=True,
                                      text=True, timeout=timeout, env=env)
        return subprocess.run(argv, capture_output=True, text=True,
                              timeout=timeout, env=env)
    except Exception:
        return None


def _split(argstr):
    """Split a verb's argument string the way a shell would.

    ⛔ The remote arm hands this to a shell and the local arm does not, so the two
    must agree about quoting or a row path with a space is one argument on one
    machine and two on the other. `shlex` is what the shell does.
    """
    import shlex
    return shlex.split(argstr)


def describe():
    """One line naming the plane this process will drive — for a verb's log.

    ⭐ Every instrument must state its subject in its own output; that is the rule
    `ygg_host`'s header ends on, and this is that rule applied to the transport.
    """
    home = sandbox_home()
    if home:
        return f"plane: sandbox {home} via {local_binary()} (local)"
    return f"plane: fleet via {remote_binary()} over ssh"
