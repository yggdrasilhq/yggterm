#!/usr/bin/env python3
"""composer_review — the automatic loop-closure trigger (owner directive
2026-10-26: "Make sure this is automatic when it happens... composer or any
brain unit should be able to run anywhere throughout the fleet").

Runs after each composer tick. When the outbox gains NEW proposal ids since
the last dispatch marker, it stages a bounded brief (proposals + observation
stats + the cohort reconciliation outlook) and runs the brain-pair review
headlessly (codex exec, gpt-6.1-sol per the attendance law) with a fixed
verdict shape. Verdicts land in the state dir and ONE board post per batch.

Laws honored:
- deterministic trigger (file state + marker), idempotent (unchanged batch
  = no-op; failed review = marker untouched, retried next tick);
- the kernel never publishes — THIS is the brain-pair attendance step, and
  its verdicts are relayed with provenance; repairs are NOT auto-executed,
  they are proposed to the owning campaign boards;
- S1 slot: a Jev triage pass (cheap classification of proposals before the
  S2 review) is the designed next refinement — not faked here.

All paths $HOME-relative: runs on any host with a composer install.
"""
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

STATE = Path(os.environ.get(
    "COMPOSER_STATE", str(Path.home() / ".yggterm/swarm/composer")))
OUTBOX = STATE / "outbox.jsonl"
MARKER = STATE / "dispatch.json"
REVIEWS = STATE / "reviews"
BOARD = "infra/meta"
MODEL = os.environ.get("COMPOSER_REVIEW_MODEL", "gpt-6.1-sol")
EFFORT = os.environ.get("COMPOSER_REVIEW_EFFORT", "medium")


def post_board(body, host):
    """One warning/note post per review cycle; board append via the shared
    dataset file (msgboard may be absent on this host)."""
    try:
        p = Path.home() / "data/msggraph/boards/infra/meta/board.jsonl"
        if not p.exists():
            return "board-absent"
        post = {"ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "board": BOARD,
                "from": {"row_uuid": f"composer-review-{host}",
                         "harness": "composer-review", "seat": None},
                "kind": "note", "body": body[:3500]}
        with p.open("a") as f:
            f.write(json.dumps(post) + "\n")
        return "posted"
    except OSError as e:
        return f"board-error: {e}"


def main():
    host = os.uname().nodename.split(".")[0].lower()
    if not OUTBOX.exists():
        return 0  # nothing ever proposed — silent no-op
    rows = [json.loads(l) for l in OUTBOX.read_text().splitlines() if l.strip()]
    if not rows:
        return 0
    ids = sorted(r.get("id") or r.get("dedupe_key", "?") for r in rows)
    batch = hashlib.sha256("\n".join(ids).encode()).hexdigest()[:16]
    marker = {}
    if MARKER.exists():
        try:
            marker = json.loads(MARKER.read_text())
        except json.JSONDecodeError:
            marker = {}
    if marker.get("batch") == batch and marker.get("verdict_ok"):
        return 0  # already reviewed this exact batch

    REVIEWS.mkdir(parents=True, exist_ok=True)
    stage = REVIEWS / f"batch-{batch}"
    stage.mkdir(exist_ok=True)
    (stage / "outbox-snapshot.jsonl").write_text(
        "\n".join(json.dumps(r) for r in rows) + "\n")
    obs_n = 0
    obs_path = STATE / "observations.jsonl"
    if obs_path.exists():
        obs_n = sum(1 for _ in obs_path.open())
    brief = f"""# Composer outbox review — batch {batch} (automatic, {host})

You are the brain-pair reviewer for the fleet lesson miner. The composer
mined {len(rows)} proposal(s) from {obs_n} observations. Proposals are
CANDIDATE fault patterns (unverified symptoms by kernel law). For EACH
proposal return one line:
  <proposal-id> accept|rebut|defer — <one-paragraph basis>

Rules: accept means "worth a repair work-item on the owning campaign";
rebut must name the disconfirming evidence class; defer names the missing
measurement. You are NOT executing repairs. Patterns need >=2 recurrences
to be eligible — single-observation proposals should usually be defer.
End with ONE line: BATCH <batch> reviewed=<n> accepts=<n> rebut=<n> defers=<n>
"""
    (stage / "brief.md").write_text(brief)
    codex = None
    for cand in (Path.home() / ".yggterm/ynpm/bin/codex", "codex"):
        if cand.exists() if isinstance(cand, Path) else True:
            codex = str(cand)
            break
    cmd = [codex, "exec", "--skip-git-repo-check",
           "--dangerously-bypass-approvals-and-sandbox",
           "-m", MODEL, "-c", f'model_reasoning_effort="{EFFORT}"',
           brief +
           "\nFull proposal snapshot: ./outbox-snapshot.jsonl in this directory."
           " Read it before verdicting."]
    try:
        r = subprocess.run(cmd, cwd=stage, capture_output=True, text=True,
                           timeout=540)
        verdict = (r.stdout or "") + ("\n[stderr]\n" + r.stderr if r.returncode else "")
        ok = r.returncode == 0 and "BATCH" in r.stdout
    except (subprocess.TimeoutExpired, FileNotFoundError) as e:
        verdict, ok = f"review transport failed: {e}", False
    (stage / "verdict.md").write_text(verdict or "(empty)")
    if not ok:
        # marker untouched — next tick retries the same batch
        print(json.dumps({"batch": batch, "review": "failed-retry-next-tick"}))
        return 0
    MARKER.write_text(json.dumps(
        {"batch": batch, "dispatched_at": time.strftime("%Y-%m-%dT%H:%M:%SZ",
                                                        time.gmtime()),
         "review": str(stage / "verdict.md"), "verdict_ok": True}))
    tail = "\n".join(verdict.strip().splitlines()[-6:])[:1500]
    print(json.dumps({"batch": batch, "review": "ok",
                      "board": post_board(
                          f"composer-review {host}: batch {batch} reviewed "
                          f"automatically ({len(rows)} proposals, {obs_n} "
                          f"observations). Verdict tail:\n{tail}\n"
                          f"Full: {stage}/verdict.md. Repairs are PROPOSED, "
                          f"not executed — owning campaigns pick them up.",
                          host)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
