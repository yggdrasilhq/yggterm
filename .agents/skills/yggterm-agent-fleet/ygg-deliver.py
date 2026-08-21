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

⚠ **A LONG WAIT CAN OUTLIVE THE MESSAGE.** Measured 2026-08-21: a correction was
armed for a working lane, waited the full 30 minutes, and by the time the wait
expired the lane had independently found the same thing and built the fix — so the
refusal to deliver was the right answer for the wrong reason. The verb cannot know
this; the CALLER must re-read a message that waited long before re-arming it. A
timeout here is a prompt to re-check the content, not just to retry.

⛔ It refuses a row listed in `never-arm.tsv`. That file marks rows a PERSON is
using, and its meaning is stronger than "do not wake": typing into one splices
into what they have half-written and submits the fusion as their turn.
"""
import argparse, glob, json, os, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from ygg_rowarg import row_session_id  # noqa: E402
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


def _reap_if_never_briefed(uuid, uri, host, a):
    """⛔⛔ A ROW THAT WAS NEVER BRIEFED MUST NOT OUTLIVE THE ATTEMPT TO BRIEF IT.

    A spawn whose submit failed used to leave its row seated and empty — holding a
    seat, counted as WORKING by every sweep because a live process with no
    transcript is unclassifiable, and doing nothing at all. Three accumulated,
    one alive for over two hours, and each new failure added another.

    ⚖ THE TEST IS "HAS IT EVER WRITTEN A WORD", NOT "DID WE FAIL". A row that took
    an earlier brief and is merely busy has a transcript and is never touched here
    — losing a working lane to a delivery timeout would be far worse than the
    debris this cleans up. Only a row with NO transcript at all is reaped, because
    that row has, demonstrably, never been given anything to do.
    """
    if glob.glob(os.path.expanduser(f"~/.claude/projects/*/{uuid}.jsonl")):
        log("  the row has a transcript — it has been briefed before, so it STAYS")
        return 6
    fold = os.path.join(HERE, "ygg-fold.py")
    if not os.path.exists(fold):
        log(f"  ⚠ no fold verb here — row {uuid[:8]} is left seated and un-briefed")
        return 6
    log(f"  reaping {uuid[:8]}: it has never written a transcript, so it was never "
        f"briefed and cannot become anything")
    subprocess.run([fold, "row", uuid, "--force", "--apply"], timeout=300)
    return 6


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("target")
    # ⚠ `--message` holds a PATH, which its name does not say. An orchestrator
    # with the text already in hand passes the text, gets "no such message file"
    # with 7 KB of brief where a filename should be, and has to go and read the
    # source to find out which of the two the flag wanted. Both are accepted now,
    # as SEPARATE flags rather than by sniffing the value: a heuristic would read
    # a one-line message that happens to name a file as a file.
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--message", help="PATH to a file holding the message")
    src.add_argument("--text", help="the message itself, inline")
    ap.add_argument("--ack", help="token to grep the transcript for; default: first word of line 1")
    ap.add_argument("--wait-min", type=float, default=30.0)
    ap.add_argument("--host")
    a = ap.parse_args()

    host = a.host or gui_host()
    if not host:
        log("⛔ no GUI host")
        return 2
    if a.text is not None:
        body = a.text
    else:
        if not os.path.exists(a.message):
            log(f"⛔ --message takes a PATH and there is no file at: {a.message[:120]}")
            log("   To pass the text itself, use --text.")
            return 2
        body = open(a.message).read()
    first = body.splitlines()[0].strip() if body.strip() else ""
    ack = a.ack or (first.split()[0] if first else "")

    row = find_row(host, a.target)
    if not row:
        log(f"⛔ no row matches {a.target}")
        return 3
    uri = row["full_path"]
    # ⛔ The ROW's own id, never a slice of its address. `full_path` is an
    #    ADDRESS, and only a live `scheme://host/<uuid>` one ends in the id —
    #    a row at rest is a store path, and one CLI's store names every
    #    session file identically, so slicing collapsed five rows onto one.
    #    Everything below keys on this: the never-arm guard that stops us
    #    typing over a person, the temp file, and the reap.
    uuid = row_session_id(row)

    why = never_armed(uuid)
    if why:
        log(f"⛔ REFUSED: {uuid[:8]} is on never-arm.tsv — {why}")
        return 4

    # ⚠ The wait is the whole point. A row that is working will read this at the
    # top of its next turn; a row typed into mid-turn may never read it at all.
    # ⛔⛔ `busy:false` IS NOT READINESS, AND A COLD-START ROW PROVES IT. A row whose
    # CLI is still booting reports busy:false — nothing is working yet — and then
    # refuses the submit with `submitted:false`, because the composer is drawn well
    # before the input loop is live. Measured 2026-08-21 on a freshly spawned lane:
    # busy:false, submit refused, one second after creation.
    #
    # ⇒ Two questions, two probes. `busy` answers "is somebody mid-turn"; only
    #   `input-check consuming_input` answers "can this row take bytes at all".
    #   Waiting on both means `submitted:false` is rare rather than routine, which
    #   matters because the law forbids retrying it — so the ONE submit has to be
    #   aimed well.
    deadline = time.time() + a.wait_min * 60
    while True:
        row = find_row(host, uuid) or row
        if row.get("busy"):
            why = f"busy ({row.get('busy_reason')})"
        else:
            v = app(host, f"terminal input-check '{uri}' --check-timeout-ms 20000")
            if (v.get("data") or {}).get("consuming_input"):
                break
            why = "not consuming input yet (still starting up?)"
        if time.time() > deadline:
            log(f"⛔ {why} after {a.wait_min:g}m — NOT delivered")
            return _reap_if_never_briefed(uuid, uri, host, a)
        log(f"{why} — waiting {POLL_S}s")
        time.sleep(POLL_S)

    # ⛔ ONE ENCODING OF "THE MESSAGE". This used to scp `a.message` — the PATH —
    # which silently made the file the source of truth and the loaded `body` a
    # decorative copy. With `--text` there is no path at all, and any future edit
    # applied to `body` would have been shipped as the untouched original. The
    # bytes that were read are the bytes that are sent.
    local = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False)
    local.write(body)
    local.close()
    remote = f"/tmp/ygg-deliver-{uuid[:8]}.txt"
    try:
        subprocess.run(["scp", "-q", local.name, f"{host}:{remote}"], timeout=120)
    finally:
        os.unlink(local.name)
    reply = app(host, f"terminal submit '{uri}' --stdin", stdin_path=remote)
    data = reply.get("data") or {}
    if not data.get("submitted"):
        # ⛔ NEVER RETRY. `submitted:false` means the row was mid-output, not that
        # it is unreachable, and a retry is a second write into the same composer.
        log(f"⛔ submitted:false ({reply.get('error')}) — NOT retried, by law")
        return _reap_if_never_briefed(uuid, uri, host, a)
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
