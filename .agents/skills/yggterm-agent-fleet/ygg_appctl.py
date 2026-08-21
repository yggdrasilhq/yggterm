"""ygg_appctl — the ONE owner of *how* a fleet verb reaches a row plane.

⛔⛔ WHY THIS FILE EXISTS: THE VERBS COULD NOT BE AIMED ANYWHERE BUT AT A LIVING
PERSON'S DESKTOP, SO EVERY DEFECT IN THEM WAS FOUND THERE.

`ygg-deliver`, `ygg-spawn`, `ygg-monitor` and `ygg-fold` each carried their own
copy of one line — `ssh <gui-host> ~/.local/bin/yggterm-headless server app …`.
Two facts are welded into it and neither could be moved:

  · the BINARY is a module constant, so there is no way to run the shipped verb
    against a binary you just built or copied into a sandbox;
  · the HOME is whatever that binary resolves at the far end, which is the real
    one. `--host` moves which MACHINE answers and never which HOME, and a
    `YGGTERM_HOME` exported by the caller does not survive the ssh — a probe
    inherits the caller's environment, not the action's.

⇒ So a sandbox row plane was reachable (field guide: *A SANDBOX GUI NEEDS A
PRIVATE BUS AND COMPOSITING OFF*) and these four verbs still could not be pointed
at it. Every defect in them has therefore been found on the owner's live desktop,
including three destructive ones fixed in the week this was written: a reap that
raised `NameError` on every call, a reap that destroyed rows on a boolean that
could not see them, and a narrowing that missed the largest CLI family.

⚖ **The asymmetry that makes this worth a module rather than four edits.** A verb
that cannot be rehearsed is not merely untested — it is *only ever* tested by the
run that matters, on rows somebody is working in. The cost of the missing seam is
paid entirely by the person whose lane gets destroyed by the branch nobody ran.

USE
    import ygg_appctl
    plane = ygg_appctl.resolve(a.host)          # says out loud what it aimed at
    rows  = plane.app_json("rows --json")
    plane.run("server status")

AIMING IT (env, because that is what the caller of a sandbox already sets)
    YGGTERM_HOME=$SB                 the home — and, on its own, enough
    YGG_HEADLESS_BIN=$SB/bin/yggterm-headless
    YGG_APPCTL_HOST=local            explicit transport override; `local` = no ssh

⭐ **A NON-DEFAULT `YGGTERM_HOME` IMPLIES THE LOCAL TRANSPORT, AND NOTHING ELSE
DOES.** A home is a PATH, and a path is a fact about one machine; ssh-ing a local
sandbox path at another machine's daemon names a directory that does not exist
there. So a home that differs from `$HOME/.yggterm` — the daemon's own default,
`resolve_yggterm_home` in yggterm-core — switches the transport to this machine.
⛔ A caller who merely exports the REAL home changes nothing, which is the point:
the inference has to be inert for everybody who is not sandboxing. An explicit
`--host` beats it either way, so a sandbox on another machine is still reachable.

⚠ **AND THE HOME IS CARRIED INTO THE REMOTE COMMAND, not just into our own
environment.** That is the half that looks done and is not: exporting
`YGGTERM_HOME` in the shell that runs a verb does nothing at all if the verb
reaches the plane over ssh, because the far end starts a fresh login shell. The
variable has to be written into the command string, and it is.
"""

import json
import os
import shlex
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

#: The binary a fleet verb drives. Overridable so a sandbox can be handed a
#: freshly built one; the default is the fleet install, as it always was.
ENV_BIN = "YGG_HEADLESS_BIN"
DEFAULT_BIN = "~/.local/bin/yggterm-headless"

#: The state plane. Same name the daemon itself reads, deliberately — a caller
#: setting up a sandbox exports this anyway, and a second name for the same
#: concept is a second thing that can disagree.
ENV_HOME = "YGGTERM_HOME"

#: Explicit transport, for the case the inference cannot cover: a sandbox on
#: ANOTHER machine, or a caller who wants the local transport with the default
#: home. `local` (or `localhost`, or `-`) means "no ssh".
ENV_HOST = "YGG_APPCTL_HOST"

_LOCAL_NAMES = {"local", "localhost", "-", "127.0.0.1", "::1"}


def default_home():
    """`$HOME/.yggterm` — the daemon's own default, restated here on purpose.

    ⛔ It is restated rather than imported because the owner is Rust
    (`yggterm_core::resolve_yggterm_home`) and a Python verb cannot call it. The
    only thing that depends on this value is the transport INFERENCE below, and
    the inference is a convenience: if it ever drifts, a caller passing
    `--host`/`$YGG_APPCTL_HOST` is unaffected, and nothing is destroyed by it.
    """
    return os.path.join(os.path.expanduser("~"), ".yggterm")


def env_home():
    """The aimed `YGGTERM_HOME`, or None for the default. One reader, one answer."""
    return os.environ.get(ENV_HOME) or None


def relay_dir(home=None):
    """`<home>/relay` as a LOCAL filesystem path, for stores read before a resolve.

    ⚖ Module-level as well as on `Plane`, because several verbs build their store
    paths at import time — before anything has been resolved — and a second
    spelling of the same expression is exactly the divergence this module exists
    to remove.
    """
    return os.path.join(os.path.expanduser(home or env_home() or default_home()), "relay")


class Plane:
    """One row plane, and everything needed to talk to it.

    ⛔ `host is None` means THIS MACHINE and no ssh. It does not mean "unknown"
    and it never means "could not resolve" — that is `resolve()` returning None,
    which callers must treat as a refusal, exactly as `ygg_host` requires.
    """

    def __init__(self, host, binary, home, why=""):
        self.host = host or None
        self.binary = binary
        self.home = home
        self.why = why

    # ---- addressing -------------------------------------------------------

    @property
    def local(self):
        return self.host is None

    def describe(self):
        return (f"host={self.host or 'local (no ssh)'} "
                f"home={self.home or 'the default (~/.yggterm)'} "
                f"bin={self.binary}{(' — ' + self.why) if self.why else ''}")

    def env_prefix(self):
        """`YGGTERM_HOME='…' ` for the remote command string, or ''."""
        return f"{ENV_HOME}={shlex.quote(self.home)} " if self.home else ""

    def shell(self, argstr):
        """The command as it must read ON THE TARGET, env and all."""
        return f"{self.env_prefix()}{self.binary} {argstr}"

    def argv(self, argstr, stdin_path=None):
        """argv for `subprocess`, ssh-wrapped when the plane is remote.

        `stdin_path` names a file ON THE TARGET for the remote arm and on this
        machine for the local one — `put()` is what makes those the same file.
        """
        if self.local:
            return [os.path.expanduser(self.binary)] + shlex.split(argstr)
        cmd = self.shell(argstr)
        if stdin_path:
            cmd += f" < {shlex.quote(stdin_path)}"
        return ["ssh", "-n", self.host, cmd]

    def env(self):
        """The environment for a LOCAL run. Remote runs carry it in the string."""
        if not self.home:
            return None
        e = dict(os.environ)
        e[ENV_HOME] = self.home
        return e

    # ---- running ----------------------------------------------------------

    def run(self, argstr, stdin_path=None, timeout=180, check_shell=False):
        """Run one command on this plane. Returns the CompletedProcess.

        ⚠ Never raises on a non-zero exit: every caller here reads the OUTPUT,
        and a transport failure has to stay distinguishable from an empty answer.
        """
        argv = self.argv(argstr, stdin_path=stdin_path)
        kw = dict(capture_output=True, text=True, timeout=timeout)
        if self.local:
            kw["env"] = self.env()
            # ⛔ `ssh -n` closes stdin on the remote arm; the local arm has to do
            #    the same, or a verb run from a terminal can block on the tty.
            kw["stdin"] = open(stdin_path) if stdin_path else subprocess.DEVNULL
        try:
            return subprocess.run(argv, **kw)
        except Exception as exc:
            # ⛔ A BAD AIM IS A TRANSPORT FAILURE, NOT A CRASH. A binary that does
            #    not exist, a timeout, an ssh that cannot connect — every caller
            #    here already knows how to refuse to conclude from an unreadable
            #    answer, and none of them know how to survive a traceback. A
            #    mistyped `$YGG_HEADLESS_BIN` must read as "the plane did not
            #    answer", which is exactly what it is.
            return subprocess.CompletedProcess(argv, 127, "", f"{type(exc).__name__}: {exc}")
        finally:
            if self.local and stdin_path and kw.get("stdin") not in (None, subprocess.DEVNULL):
                kw["stdin"].close()

    def app_json(self, argstr, stdin_path=None, timeout=180):
        """`server app <argstr>`, parsed. The error shape the verbs already use.

        ⛔ NOT `json.loads(out[out.find('{'):])`. At least one verb replies with
        TWO concatenated JSON objects and that idiom reads the first and discards
        the rest WITHOUT RAISING — a truncated answer a watchdog then acts on
        confidently. `raw_decode` stops at the first document and can say there
        was more.
        """
        r = self.run(f"server app {argstr}", stdin_path=stdin_path, timeout=timeout)
        out = r.stdout or ""
        if "{" not in out:
            return {"error": (r.stderr or out or "unparseable").strip()[:200]}
        try:
            obj, end = json.JSONDecoder().raw_decode(out[out.find("{"):])
        except Exception:
            return {"error": (r.stderr or out or "unparseable").strip()[:200]}
        if not isinstance(obj, dict):
            return {"error": "reply was not an object"}
        if out[out.find("{") + end:].strip().startswith("{"):
            obj.setdefault("__trailing_documents__", True)
        return obj

    def run_shell(self, shell_cmd, timeout=180):
        """Run a shell LINE the caller composed, on this plane.

        For a pipeline or a redirect — `printf … | <plane.shell(…)>`. The local
        arm goes through `bash -c` so the same string means the same thing on
        both, which is the property that makes a sandbox rehearsal worth
        anything: a verb that took a different code path when aimed at a sandbox
        would be proving the sandbox rather than the verb.
        """
        argv = (["bash", "-c", shell_cmd] if self.local
                else ["ssh", "-n", self.host, shell_cmd])
        try:
            return subprocess.run(argv, capture_output=True, text=True, timeout=timeout,
                                  env=self.env() if self.local else None,
                                  stdin=subprocess.DEVNULL)
        except Exception as exc:
            return subprocess.CompletedProcess(argv, 127, "", f"{type(exc).__name__}: {exc}")

    def at(self, host):
        """The SAME aim, pointed at another machine — or at this one.

        ⚖ A row's own host and the plane's host are different questions and this
        keeps them that way: liveness and rendered screens are asked of the
        machine that owns the row, while the binary and the home stay whatever
        the caller aimed this run at. Falsy `host` means this machine, which is
        the convention every caller here already used.
        """
        if (host or None) == self.host:
            return self
        return Plane(host or None, self.binary, self.home,
                     f"{self.why} (re-aimed at {host or 'this machine'})" if self.why else "")

    def home_path(self, *parts):
        """A path INSIDE the aimed yggterm home, as it reads on the target.

        ⛔ Not `expanduser('~/.yggterm/…')`. That is this expression with the
        home left at its default, and writing the default down again is the
        second encoding that makes a sandbox run scribble on the live plane.
        """
        return os.path.join(self.home or "~/.yggterm", *parts)

    def put(self, local_path, remote_path):
        """Make `local_path` readable on the target; return the path to use.

        ⚖ On the local plane there is nothing to copy, and copying anyway is not
        harmless: the destination is a fixed `/tmp` name, so a verb aimed at its
        own machine would overwrite the file it is about to read.
        """
        if self.local:
            return local_path
        subprocess.run(["scp", "-q", local_path, f"{self.host}:{remote_path}"], timeout=120)
        return remote_path

    def relay_dir(self):
        """`<home>/relay` — the fleet's own bookkeeping, which lives in the home.

        `~/.yggterm/relay` was never a third place; it is this expression with
        the home left at its default. Saying so lets a sandbox run keep its
        ledgers, wake records, subscriptions and harvests out of the live
        plane's — otherwise a rehearsal writes into the roster the real
        watchdogs read, which is worse than not rehearsing at all.
        """
        return relay_dir(self.home)


def resolve(explicit_host=None, verbose=True, probe=True):
    """The plane to drive, or None. ⛔ None means REFUSE TO CONCLUDE.

    Order, and each step is stated in the returned plane's `why`:
      1. an explicit `--host` (`local` = this machine)
      2. `$YGG_APPCTL_HOST`
      3. a NON-DEFAULT `$YGGTERM_HOME` ⇒ this machine, because a home is a path
      4. `ygg_host.resolve_gui_host()`, which probes until something answers
    """
    binary = os.environ.get(ENV_BIN) or DEFAULT_BIN
    home = os.environ.get(ENV_HOME) or None

    def done(host, why):
        p = Plane(host, binary, home, why)
        if verbose:
            print(f"ygg-appctl: {p.describe()}", file=sys.stderr, flush=True)
        return p

    for value, why in ((explicit_host, "named by --host"),
                       (os.environ.get(ENV_HOST), f"named by ${ENV_HOST}")):
        if value:
            v = str(value).strip()
            return done(None if v.lower() in _LOCAL_NAMES else v, why)

    if home and os.path.abspath(os.path.expanduser(home)) != os.path.abspath(default_home()):
        return done(None, f"${ENV_HOME} is a sandbox path, so the plane is this machine")

    if not probe:
        return None
    import ygg_host  # noqa: E402  (imported late: it shells out on import-time use)
    host = ygg_host.resolve_gui_host(verbose=verbose)
    if not host:
        return None
    return done(host, "resolved by ygg-host")
