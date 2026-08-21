#!/usr/bin/env python3
"""ygg-deliver — hand a message to a row when the row can take it, and PROVE it landed.

⛔⛔ WHY THIS EXISTS. Submitting into a busy row types into its composer while it
is mid-turn. The standing law says check `busy` before any submit and never retry
a `submitted:false` — but the law leaves the caller holding a message and no way
to deliver it, so the message either gets forced in anyway or is quietly dropped.
Both have happened. A correction that arrives late is worth far more than one that
arrives spliced into someone's half-written turn, and worth infinitely more than
one that is never sent.

⇒ So: wait for the row to go idle, submit once, and then prove delivery from the
TRANSCRIPT rather than from the verb's own answer. `submitted: true` describes the
WRITE; only the transcript says the row read it, and it lags the submit by ~15 s.

USAGE
    ygg-deliver.py <row-uri-or-uuid> --message <file> [--ack TOKEN]
                   [--wait-min 30] [--host <gui-host>]

⛔ It refuses a row listed in `never-arm.tsv`. That file marks rows a PERSON is
using, and its meaning is stronger than "do not wake": typing into one splices
into what they have half-written and submits the fusion as their turn.
"""
import argparse, glob, json, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
YGG = "~/.local/bin/yggterm-headless"
POLL_S = 20


def log(m):
    print(f"{time.strftime('%H:%M:%S')} ygg-deliver {m}", file=sys.stderr)


def gui_host():
    r = subprocess.run([os.path.join(HERE, "..", "..", "..", "scripts", "ygg-live-host.sh"),
                        "--quiet"], capture_output=True, text=True, timeout=60)
    return (r.stdout or "").strip() or os.environ.get("YGG_GUI_HOST", "")


def app(host, argstr, stdin_path=None):
    cmd = f"{YGG} server app {argstr}"
    if stdin_path:
        cmd += f" < {stdin_path}"
    r = subprocess.run(["ssh", "-n", host, cmd], capture_output=True, text=True, timeout=180)
    try:
        return json.loads(r.stdout)
    except Exception:
        return {"error": (r.stderr or r.stdout or "unparseable").strip()[:200]}


def never_armed(uuid):
    for path in glob.glob(os.path.expanduser("~/.yggterm/relay/never-arm.tsv")):
        for line in open(path):
            if line.startswith("#") or not line.strip():
                continue
            if line.split("\t", 1)[0].strip() == uuid:
                return line.split("\t", 1)[-1].strip()
    return None


def find_row(host, target):
    rows = (app(host, "rows --json").get("data") or {}).get("rows") or []
    for r in rows:
        if target in (r.get("full_path") or ""):
            return r
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("target")
    ap.add_argument("--message", required=True, help="file holding the message")
    ap.add_argument("--ack", help="token to grep the transcript for; default: first word of line 1")
    ap.add_argument("--wait-min", type=float, default=30.0)
    ap.add_argument("--host")
    a = ap.parse_args()

    host = a.host or gui_host()
    if not host:
        log("⛔ no GUI host")
        return 2
    if not os.path.exists(a.message):
        log(f"⛔ no such message file: {a.message}")
        return 2
    first = open(a.message).readline().strip()
    ack = a.ack or (first.split()[0] if first else "")

    row = find_row(host, a.target)
    if not row:
        log(f"⛔ no row matches {a.target}")
        return 3
    uri = row["full_path"]
    uuid = uri.rsplit("/", 1)[-1]

    why = never_armed(uuid)
    if why:
        log(f"⛔ REFUSED: {uuid[:8]} is on never-arm.tsv — {why}")
        return 4

    # ⚠ The wait is the whole point. A row that is working will read this at the
    # top of its next turn; a row typed into mid-turn may never read it at all.
    deadline = time.time() + a.wait_min * 60
    while True:
        row = find_row(host, uuid) or row
        if not row.get("busy"):
            break
        if time.time() > deadline:
            log(f"⛔ still busy after {a.wait_min:g}m ({row.get('busy_reason')}) — NOT delivered")
            return 5
        log(f"busy ({row.get('busy_reason')}) — waiting {POLL_S}s")
        time.sleep(POLL_S)

    remote = f"/tmp/ygg-deliver-{uuid[:8]}.txt"
    subprocess.run(["scp", "-q", a.message, f"{host}:{remote}"], timeout=120)
    reply = app(host, f"terminal submit '{uri}' --stdin", stdin_path=remote)
    data = reply.get("data") or {}
    if not data.get("submitted"):
        # ⛔ NEVER RETRY. `submitted:false` means the row was mid-output, not that
        # it is unreachable, and a retry is a second write into the same composer.
        log(f"⛔ submitted:false ({reply.get('error')}) — NOT retried, by law")
        return 6
    log(f"submitted {data.get('bytes')}B, proving delivery from the transcript")

    if not ack:
        log("⚠ no ack token — delivery UNPROVEN")
        return 0
    for _ in range(9):
        time.sleep(15)
        hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{uuid}.jsonl"))
        if hits and ack in open(hits[0], errors="ignore").read()[-400000:]:
            log(f"transcript carries {ack}: True")
            return 0
    log(f"⚠ transcript does not carry {ack} after ~2m — delivery UNPROVEN")
    return 7


if __name__ == "__main__":
    sys.exit(main())
