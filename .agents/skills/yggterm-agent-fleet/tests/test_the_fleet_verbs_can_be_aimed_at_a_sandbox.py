#!/usr/bin/env python3
"""Every fleet verb honours the aim it is given — the binary AND the home.

    python3 tests/test_the_fleet_verbs_can_be_aimed_at_a_sandbox.py

⛔ THE HOLE THIS PINS. `ygg-deliver`, `ygg-spawn`, `ygg-monitor` and `ygg-fold`
each carried their own copy of `ssh <gui-host> ~/.local/bin/yggterm-headless
server app …`. The binary was a module constant and the home was whatever that
binary resolved at the far end, which is the real one: `--host` moved which
MACHINE answered and nothing moved which HOME it answered about. So there was no
way to rehearse any of them, and every defect in them — including three
destructive ones fixed the same week — had to be found on somebody's live
desktop, on rows they were working in.

⭐ **HOW IT RATIFIES, AND WHY THE OBVIOUS GATE WOULD BE GREEN OVER THE DEFECT.**
Asserting that `ygg_appctl` builds the right command tests `ygg_appctl`. The
defect was never there — it was that the VERBS did not ask it. So each verb is
run as the shipped script, end to end, with `$YGG_HEADLESS_BIN` pointing at a
recording shim: the evidence is the shim's own log of what was executed and what
`YGGTERM_HOME` it was executed with. That is the PRODUCER's record, not a
restatement of the intent.

⚖ **What it would take to make this pass while the bug is back:** a verb would
have to invoke the shim, with the sandbox home in its environment, and still be
reading a constant — which is a contradiction. Restoring the literal in any one
of the four turns that verb's case red, verified by mutation before this file was
committed.

⚠ The static half at the end is a SECOND, weaker check and is labelled as one: a
verb could pass it by hardcoding some other path. It exists to catch a new
literal being added next to a correct call, which the behavioural half cannot
see because the correct call still happens.

Every fixture is invented.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))

FAILURES = []
DEFAULT_BIN = "~/.local/bin/yggterm-headless"

#: An invented row, in the shape `app rows --json` publishes. The scheme is a
#: LOCAL one so nothing here tries to ssh anywhere: this gate must run on a
#: machine with no fleet at all.
ROW_ID = "3f7c1d90-2222-4333-8444-555566667777"
ROW_URI = f"cc-runtime://{ROW_ID}"
ROW = {"full_path": ROW_URI, "session_id": ROW_ID, "label": "sandbox fixture",
       "outline_prefix": "0.1", "icon_kind": "claude-code", "busy": False,
       "busy_reason": "idle", "session_cwd": "/nonexistent/fixture"}

#: ⛔ A STAND-IN `ssh`, because THE REMOTE ARM IS WHERE THE DEFECT LIVED. On the
#: local arm a home exported by the caller reaches the child through ordinary
#: inheritance, so a verb that dropped it would still look right. Over ssh the
#: far end starts a fresh login shell and inherits nothing: the home has to be
#: written INTO the command string or it is silently the real one. That is the
#: half a local-only gate cannot see, and it is the half that mattered.
SSH_SHIM = r'''#!/usr/bin/env python3
import json, os, subprocess, sys
args = [a for a in sys.argv[1:] if a not in ("-n", "-o", "BatchMode=yes")]
host, cmd = args[0], " ".join(args[1:])
with open(os.environ["YGG_SSH_LOG"], "a") as fh:
    fh.write(json.dumps({"host": host, "cmd": cmd}) + "\n")
sys.exit(subprocess.run(["bash", "-c", cmd]).returncode)
'''

SHIM = r'''#!/usr/bin/env python3
"""A stand-in for yggterm-headless that RECORDS instead of doing."""
import json, os, sys
log = os.environ["YGG_SHIM_LOG"]
with open(log, "a") as fh:
    fh.write(json.dumps({"argv": sys.argv[1:],
                         "YGGTERM_HOME": os.environ.get("YGGTERM_HOME"),
                         "exe": sys.argv[0]}) + "\n")
a = " ".join(sys.argv[1:])
row = json.loads(os.environ["YGG_SHIM_ROW"])
if "input-check" in a:
    out = {"data": {"consuming_input": True}}
elif "terminal submit" in a:
    out = {"data": {"submitted": True, "bytes": 11}}
elif "terminal new" in a:
    out = {"error": "shim refuses to birth a row"}
elif "read-buffer" in a:
    out = {"data": {"text": "fixture screen\n"}}
elif "rows" in a:
    out = {"data": {"rows": [row]}}
else:
    out = {"data": {}}
print(json.dumps(out))
'''


def shim_env(sb, log, binary):
    e = dict(os.environ)
    e.update({"YGGTERM_HOME": str(sb), "YGG_HEADLESS_BIN": str(binary),
              "YGG_SHIM_LOG": str(log), "YGG_SHIM_ROW": json.dumps(ROW),
              # ⛔ Nothing here may resolve a real GUI host: a gate that reaches
              #    the fleet is a gate that behaves differently on every machine.
              "YGG_GUI_HOST": "", "YGG_FOLD_NEVER": ""})
    return e


def run_verb(name, argv, sb, log, binary, timeout=180):
    return subprocess.run([sys.executable, str(HERE / name)] + argv,
                          capture_output=True, text=True, timeout=timeout,
                          env=shim_env(sb, log, binary), cwd=str(HERE))


def calls(log):
    return [json.loads(l) for l in Path(log).read_text().splitlines() if l.strip()]


def check(verb, log, sb, proc):
    """The three things that make a verb aimable, asked of what actually ran."""
    got = calls(log)
    if not got:
        FAILURES.append(f"{verb}: made NO call through the aimed binary at all. "
                        f"rc={proc.returncode} stderr={proc.stderr.strip()[-300:]}")
        return
    wrong_home = [c for c in got if c["YGGTERM_HOME"] != str(sb)]
    if wrong_home:
        FAILURES.append(f"{verb}: {len(wrong_home)}/{len(got)} call(s) ran with "
                        f"YGGTERM_HOME={wrong_home[0]['YGGTERM_HOME']!r}, not the aimed "
                        f"{str(sb)!r} — the home did not travel with the binary")
    print(f"  {verb}: {len(got)} call(s) through the aimed binary, "
          f"all with the aimed home; first = {' '.join(got[0]['argv'][:4])}")


print("BEHAVIOURAL — each verb run as the shipped script, against a recording shim")
sb = Path(tempfile.mkdtemp(prefix="ygg-aim-sandbox-"))
try:
    binary = sb / "bin" / "yggterm-headless"
    binary.parent.mkdir(parents=True)
    binary.write_text(SHIM)
    binary.chmod(0o755)

    # ---- ygg-deliver ------------------------------------------------------
    log = sb / "deliver.log"
    log.write_text("")
    msg = sb / "message.txt"
    # ⚠ A blank first line means NO ack token, so the verb reports delivery
    #    UNPROVEN and returns instead of waiting two minutes on a transcript that
    #    a shim cannot write. What is under test is the SUBMIT, not the proof.
    msg.write_text("\ninvented message body\n")
    proc = run_verb("ygg-deliver.py", [ROW_ID, "--message", str(msg), "--wait-min", "1"],
                    sb, log, binary)
    check("ygg-deliver", log, sb, proc)
    if not any("submit" in " ".join(c["argv"]) for c in calls(log)):
        FAILURES.append("ygg-deliver: never reached the SUBMIT through the aimed binary — "
                        "the one call that writes into somebody's row")

    # ---- ygg-spawn --------------------------------------------------------
    log = sb / "spawn.log"
    log.write_text("")
    brief = sb / "brief.txt"
    brief.write_text("FIXTURE-TOKEN an invented brief\n")
    proc = run_verb("ygg-spawn.py", ["--seat", "0.1", "--title", "fixture",
                                     "--purpose", "fixture", "--cwd", str(sb),
                                     "--brief", str(brief), "--no-group"],
                    sb, log, binary)
    check("ygg-spawn", log, sb, proc)
    if not any("terminal" in " ".join(c["argv"]) and "new" in " ".join(c["argv"])
               for c in calls(log)):
        FAILURES.append("ygg-spawn: never reached `terminal new` through the aimed binary")

    # ---- ygg-fold ---------------------------------------------------------
    log = sb / "fold.log"
    log.write_text("")
    proc = run_verb("ygg-fold.py", ["sweep"], sb, log, binary)
    check("ygg-fold", log, sb, proc)

    # ---- ygg-monitor ------------------------------------------------------
    log = sb / "monitor.log"
    log.write_text("")
    proc = run_verb("ygg-monitor.py", ["tick", "--dry-run"], sb, log, binary)
    check("ygg-monitor", log, sb, proc)

    # ---- the stores follow the home too -----------------------------------
    # ⛔ A rehearsal that writes its subscriptions, wake ledger and harvests into
    #    the LIVE relay dir is worse than no rehearsal: the real watchdogs then
    #    read a roster full of rows that never existed.
    print("STORES — the fleet's own bookkeeping follows the aimed home")
    for mod, attr in (("ygg-monitor.py", "STATE"), ("ygg-fold.py", "RELAY"),
                      ("ygg-booter.py", "STATE"), ("ygg-babysit.py", "STATE"),
                      ("ygg-board.py", "STATE")):
        out = subprocess.run(
            [sys.executable, "-c",
             "import importlib.util,sys;"
             f"sp=importlib.util.spec_from_file_location('m',{str(HERE / mod)!r});"
             "m=importlib.util.module_from_spec(sp);sys.argv=['m'];"
             "exec('try:\\n sp.loader.exec_module(m)\\nexcept SystemExit:\\n pass');"
             f"print(m.{attr})"],
            capture_output=True, text=True, timeout=120,
            env=shim_env(sb, sb / "unused.log", binary))
        where = (out.stdout or "").strip()
        if not where.startswith(str(sb)):
            FAILURES.append(f"{mod}:{attr} is {where or out.stderr.strip()[-200:]!r} — "
                            f"outside the aimed home {sb}")
        else:
            print(f"  {mod}:{attr} -> {where}")
    # ---- the remote arm ---------------------------------------------------
    print("REMOTE — the home is written INTO the command, because it does not "
          "survive ssh")
    sshdir = sb / "sshbin"
    sshdir.mkdir()
    (sshdir / "ssh").write_text(SSH_SHIM)
    (sshdir / "ssh").chmod(0o755)
    # scp has nothing to prove here and everything to break; a copy is a copy.
    (sshdir / "scp").write_text("#!/bin/sh\nshift 2>/dev/null; exec cp \"$1\" "
                                "\"$(echo \"$2\" | sed 's/^[^:]*://')\"\n")
    (sshdir / "scp").chmod(0o755)

    log = sb / "remote.log"
    log.write_text("")
    sshlog = sb / "ssh.log"
    sshlog.write_text("")
    env = shim_env(sb, log, binary)
    env["PATH"] = f"{sshdir}:{env['PATH']}"
    env["YGG_SSH_LOG"] = str(sshlog)
    msg = sb / "remote-message.txt"
    msg.write_text("\ninvented remote message\n")
    proc = subprocess.run([sys.executable, str(HERE / "ygg-deliver.py"), ROW_ID,
                           "--message", str(msg), "--wait-min", "1",
                           "--host", "invented-machine"],
                          capture_output=True, text=True, timeout=180,
                          env=env, cwd=str(HERE))
    hops = [json.loads(l) for l in sshlog.read_text().splitlines() if l.strip()]
    if not hops:
        FAILURES.append(f"remote arm: ygg-deliver made no ssh hop at all. "
                        f"rc={proc.returncode} stderr={proc.stderr.strip()[-300:]}")
    homeless = [h for h in hops if f"YGGTERM_HOME={sb}" not in h["cmd"]]
    if homeless:
        FAILURES.append(f"remote arm: {len(homeless)}/{len(hops)} ssh command(s) carry no "
                        f"YGGTERM_HOME — the aimed home stayed on this machine. "
                        f"first: {homeless[0]['cmd'][:140]}")
    unaimed = [h for h in hops if str(binary) not in h["cmd"]]
    if unaimed:
        FAILURES.append(f"remote arm: {len(unaimed)}/{len(hops)} ssh command(s) run a binary "
                        f"other than the aimed one. first: {unaimed[0]['cmd'][:140]}")
    if hops and not homeless and not unaimed:
        print(f"  ygg-deliver: {len(hops)} ssh hop(s) to invented-machine, every one "
              f"carrying the aimed home and binary")
finally:
    subprocess.run(["rm", "-rf", str(sb)])

# ---- the weaker, static half ----------------------------------------------
print("STATIC (secondary) — no verb re-introduces a literal binary path")
for name in ("ygg-deliver.py", "ygg-spawn.py", "ygg-monitor.py", "ygg-fold.py"):
    text = (HERE / name).read_text()
    for n, line in enumerate(text.splitlines(), 1):
        if DEFAULT_BIN in line and not line.lstrip().startswith("#"):
            FAILURES.append(f"{name}:{n} holds a literal {DEFAULT_BIN} — the aim is a "
                            f"module constant again")
print(f"  scanned 4 verbs for {DEFAULT_BIN}")

if FAILURES:
    print("\nFAIL")
    for f in FAILURES:
        print("  ⛔ " + f)
    sys.exit(1)
print("\nPASS — 4 verbs aimable end to end, both transports, 5 stores follow the home")
