#!/usr/bin/env python3
"""composer_review — advisory brain-pair proposal review (truthful gate).

Runs after each composer tick. When the outbox gains NEW proposal content
since the last dispatch marker, it stages an immutable snapshot of the exact
canonical proposal bytes under a content-bound batch identity, writes a
criterion-bound checkpoint brief, runs the configured reviewer headlessly,
and accepts the batch ONLY when the reviewer output validates in full.

Truth laws (owner ygg4 repair unit, 2026-10-29):

- Identity is content-bound. Every proposal id is the hash of its canonical
  JSON bytes (every field covered — no '?' fallback), and the batch id
  additionally covers the review protocol version. A same-count outbox with
  changed content is a NEW batch; an unchanged replay is idempotent.
- The reviewer output must bind to the exact batch: one typed verdict line
  per proposal id (accept|rebut|defer + a reason) and a footer echoing THIS
  batch id with counts matching the real proposal distribution. A bare
  "BATCH" substring, a fabricated or wrong batch id, wrong/duplicate/
  missing ids or counts, a partial or malformed response, a nonzero exit,
  a timeout, an absent transport, or a storage failure NEVER advance the
  success marker. Review debt is retained and retried under a bounded
  cooldown instead of being hidden.
- A malformed/partial outbox is a typed observable hold: the last marker is
  never clobbered, prior verdict records are preserved, and the hold is
  appended to review-holds.jsonl. Concurrent dispatches serialize on a
  state lock; a stale outbox basis (changed while the review ran) is
  rejected without applying its verdict.
- The verdicts are ADVISORY brain-pair triage only — proposed to the owning
  campaign boards. The marker is NOT independent worker-completion
  acceptance, NOT a canonical completion receipt, and executes no repairs.
  A model failure retains review debt.

All paths $HOME-relative: runs on any host with a composer install.
"""
import fcntl
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

REVIEW_PROTOCOL = "composer-review-v2"

STATE = Path(os.environ.get(
    "COMPOSER_STATE", str(Path.home() / ".yggterm/swarm/composer")))
OUTBOX = STATE / "outbox.jsonl"
MARKER = STATE / "dispatch.json"
REVIEWS = STATE / "reviews"
LOCK = STATE / "review.lock"
ATTEMPTS = STATE / "review-attempts.json"
HOLDS = STATE / "review-holds.jsonl"
OBSERVATIONS = STATE / "observations.jsonl"
NEEDS = STATE / "needs.json"
CURSORS = STATE / "cursors.json"

BOARD = "infra/meta"
MODEL = os.environ.get("COMPOSER_REVIEW_MODEL", "gpt-6.1-sol")
EFFORT = os.environ.get("COMPOSER_REVIEW_EFFORT", "medium")
TIMEOUT_S = float(os.environ.get("COMPOSER_REVIEW_TIMEOUT_S", "540"))
COOLDOWN_S = float(os.environ.get("COMPOSER_REVIEW_COOLDOWN_S", "900"))
MAX_CONSECUTIVE_FAILURES = int(
    os.environ.get("COMPOSER_REVIEW_MAX_FAILURES", "3"))
SILENT = os.environ.get("COMPOSER_REVIEW_SILENT") == "1"

SNAPSHOT_NAME = "outbox-snapshot.jsonl"
MIN_REASON_CHARS = 10
VERDICT_RE = re.compile(
    r"^VERDICT\s+([0-9a-f]{16})\s+(accept|rebut|defer)\s+(?:—|--|-)\s+(.+)$")
FOOTER_RE = re.compile(
    r"^BATCH\s+([0-9a-f]{16})\s+reviewed=(\d+)\s+accepts=(\d+)\s+"
    r"rebut=(\d+)\s+defers=(\d+)\s*$")


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _canonical(row):
    """Deterministic serialization — the bytes identity is bound to."""
    return json.dumps(row, sort_keys=True, separators=(",", ":"))


def proposal_identity(row):
    """Content-bound proposal id over the canonical bytes: any field change
    (including unknown fields) yields a different id."""
    return hashlib.sha256(_canonical(row).encode()).hexdigest()[:16]


def batch_identity(ids):
    """Batch id covers the review protocol version plus the exact set of
    proposal ids; row count alone can never collide."""
    return hashlib.sha256(
        (REVIEW_PROTOCOL + "\n" + "\n".join(sorted(ids))).encode()
    ).hexdigest()[:16]


def build_batch(rows):
    """-> (batch_id, entries). Identical duplicate rows collapse to one
    proposal (same content, same id)."""
    by_id = {}
    for r in rows:
        by_id.setdefault(proposal_identity(r), r)
    ids = sorted(by_id)
    return batch_identity(ids), [{"id": i, "row": by_id[i]} for i in ids]


def load_outbox(path=None):
    """-> (rows, faults). Partial/malformed lines are typed, never raised."""
    faults, rows = [], []
    try:
        text = Path(path or OUTBOX).read_text()
    except (OSError, UnicodeDecodeError) as e:
        return [], [{"line": 0, "fault": "unreadable-outbox",
                     "detail": str(e)[:120]}]
    for ln, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as e:
            faults.append({"line": ln, "fault": "malformed-json",
                           "detail": str(e)[:120]})
            continue
        if not isinstance(row, dict):
            faults.append({"line": ln, "fault": "not-an-object"})
            continue
        rows.append(row)
    return rows, faults


def observation_count():
    try:
        with OBSERVATIONS.open() as f:
            return sum(1 for _ in f)
    except OSError:
        return 0


def needs_state():
    try:
        d = json.loads(NEEDS.read_text())
        return {"available": bool(d.get("available")),
                "reason": str(d.get("reason", ""))[:120]}
    except (OSError, json.JSONDecodeError):
        return {"available": False, "reason": "needs snapshot absent"}


def resource_bound():
    """Native read bound actually reached by the miner (cursor bytes), when
    the state carries it; absence is stated, never invented."""
    try:
        cur = json.loads(CURSORS.read_text())
        total = sum(int(c.get("offset", 0)) for c in cur.values()
                    if isinstance(c, dict))
        return {"sources_tracked": len(cur), "bytes_cursor_total": total}
    except (OSError, json.JSONDecodeError, TypeError, ValueError):
        return {"sources_tracked": 0, "bytes_cursor_total": 0,
                "note": "cursor state absent"}


def record_hold(status, detail, batch=None):
    """Append-only typed hold log — prior records are never rewritten."""
    row = {"at": _now(), "status": status, "detail": str(detail)[:400]}
    if batch:
        row["batch"] = batch
    try:
        HOLDS.parent.mkdir(parents=True, exist_ok=True)
        with HOLDS.open("a") as f:
            f.write(json.dumps(row) + "\n")
    except OSError:
        pass  # the typed stdout record remains the observable
    return row


def load_attempts():
    try:
        return json.loads(ATTEMPTS.read_text())
    except (OSError, json.JSONDecodeError):
        return {}


def record_failure(reason, batch=None):
    a = load_attempts()
    a["consecutive_failures"] = int(a.get("consecutive_failures", 0)) + 1
    a["last_failure_at"] = _now()
    a["last_failure_epoch"] = time.time()
    a["last_failure_reason"] = str(reason)[:200]
    if batch:
        a["last_batch"] = batch
    try:
        ATTEMPTS.write_text(json.dumps(a, indent=1))
    except OSError:
        pass
    return a


def record_success():
    try:
        if ATTEMPTS.exists():
            ATTEMPTS.unlink()
    except OSError:
        pass


def cooldown_active():
    a = load_attempts()
    if int(a.get("consecutive_failures", 0)) < MAX_CONSECUTIVE_FAILURES:
        return None
    epoch = a.get("last_failure_epoch")
    if not isinstance(epoch, (int, float)):
        return None
    elapsed = time.time() - epoch
    if elapsed < COOLDOWN_S:
        return {"retry_in_s": int(COOLDOWN_S - elapsed),
                "consecutive_failures": a.get("consecutive_failures"),
                "last_failure_reason": a.get("last_failure_reason")}
    return None


def read_marker():
    try:
        return json.loads(MARKER.read_text())
    except (OSError, json.JSONDecodeError):
        return {}


def snapshot_digest(batch_dir):
    p = batch_dir / SNAPSHOT_NAME
    if not p.exists():
        return None
    return hashlib.sha256(p.read_bytes()).hexdigest()


def stage_snapshot(batch_dir, entries):
    """Write once; an existing snapshot for this batch dir must match byte
    for byte — batch records are immutable. Returns the digest or None on
    conflict."""
    payload = "".join(_canonical(e["row"]) + "\n" for e in entries)
    digest = hashlib.sha256(payload.encode()).hexdigest()
    existing = snapshot_digest(batch_dir)
    if existing is not None:
        return digest if existing == digest else None
    batch_dir.mkdir(parents=True, exist_ok=True)
    (batch_dir / SNAPSHOT_NAME).write_text(payload)
    return digest


def stage_brief(batch_dir, batch, entries):
    """Criterion-bound checkpoint brief: baseline, evidence, falsifier,
    missingness and the native read bound — then the exact output contract.
    Returns the brief text (written by the caller, so write errors stay
    typed at the dispatch site)."""
    blocks = []
    for e in entries:
        r = e["row"]
        ev = r.get("evidence") or []
        blocks.append(
            f"- {e['id']} pattern={r.get('pattern_id', '(none)')} "
            f"detector={r.get('detector_id', '(none)')}:"
            f"{r.get('detector_version', '(none)')} "
            f"lane={r.get('lane', '(none)')}\n"
            f"  assertion: {str(r.get('assertion', '(missing)'))[:300]}\n"
            f"  falsifier: {str(r.get('disproof', '(missing)'))[:300]}\n"
            f"  evidence rows: {len(ev)}"
            + ("" if ev else "  [MISSINGNESS: no machine-checkable "
                             "evidence rows attached — treat as defer "
                             "candidate]"))
    bound = resource_bound()
    needs = needs_state()
    needs_line = (f"needs snapshot: available"
                  if needs.get("available") else
                  f"needs snapshot: UNAVAILABLE ({needs.get('reason')})")
    n = len(entries)
    return f"""# Composer outbox review — batch {batch} (advisory, automatic)

You are the brain-pair reviewer for the fleet lesson miner. This is an
ADVISORY triage of CANDIDATE fault patterns (unverified symptoms by kernel
law). You are NOT executing repairs; your verdicts are proposals for the
owning campaign boards.

## Checkpoint baseline
- protocol: {REVIEW_PROTOCOL}; batch: {batch}; proposals: {n}
- miner observations on record: {observation_count()}
- {needs_line}
- miner read bound reached: {bound["bytes_cursor_total"]} cursor bytes over
  {bound["sources_tracked"]} tracked source(s)

## Proposals (evidence + falsifier + missingness)
{chr(10).join(blocks)}

## Rules
- accept = "worth a repair work-item on the owning campaign"; rebut must
  name the disconfirming evidence class; defer names the missing
  measurement. Patterns need >=2 recurrences to be eligible —
  single-observation proposals should usually be defer.
- Your output binds to THIS batch. Return EXACTLY one verdict line per
  proposal id above, then the footer as the final line:

VERDICT <proposal-id> accept|rebut|defer — <one-paragraph basis>
...
BATCH {batch} reviewed={n} accepts=<a> rebut=<r> defers=<d>

- The counts must match your own verdict lines exactly. Any other batch id,
  id set, or count makes the whole review invalid.
"""


def resolve_transport():
    """The configured intelligent review route (model/effort env-driven).
    Never silently downgrades: a missing transport is a typed
    unavailability, not a cheaper model."""
    env_bin = os.environ.get("COMPOSER_REVIEW_BIN")
    if env_bin:
        p = Path(env_bin)
        return str(p) if p.is_file() else None
    ynpm = Path.home() / ".yggterm/ynpm/bin/codex"
    if ynpm.is_file():
        return str(ynpm)
    return "codex"  # PATH fallback


def parse_verdict(stdout, expected_ids, expected_batch):
    """Strict typed validation of the reviewer output. Returns
    (verdicts, counts); raises ValueError with the exact rejection reason.
    Prose is tolerated; verdict structure is not."""
    lines = [l.strip() for l in (stdout or "").splitlines() if l.strip()]
    if not lines:
        raise ValueError("empty stdout — no verdict")
    footer = FOOTER_RE.match(lines[-1])
    if not footer:
        raise ValueError("final nonempty line is not a valid BATCH footer")
    if footer.group(1) != expected_batch:
        raise ValueError(f"footer batch {footer.group(1)} != expected "
                         f"{expected_batch}")
    for l in lines[:-1]:
        if FOOTER_RE.match(l):
            raise ValueError("multiple BATCH footers")
    verdicts = {}
    for l in lines[:-1]:
        if not l.startswith("VERDICT"):
            continue
        m = VERDICT_RE.match(l)
        if not m:
            raise ValueError(f"malformed VERDICT line: {l[:80]}")
        pid, disp, reason = m.group(1), m.group(2), m.group(3).strip()
        if pid not in expected_ids:
            raise ValueError(f"verdict for unknown proposal id {pid}")
        if pid in verdicts:
            raise ValueError(f"duplicate verdict for {pid}")
        if len(reason) < MIN_REASON_CHARS:
            raise ValueError(f"reason too short for {pid}")
        verdicts[pid] = {"disposition": disp, "reason": reason}
    missing = [i for i in expected_ids if i not in verdicts]
    if missing:
        raise ValueError(f"missing verdicts for ids: {missing[:5]}")
    dist = {
        "accepts": sum(1 for v in verdicts.values()
                       if v["disposition"] == "accept"),
        "rebut": sum(1 for v in verdicts.values()
                     if v["disposition"] == "rebut"),
        "defers": sum(1 for v in verdicts.values()
                      if v["disposition"] == "defer")}
    if int(footer.group(2)) != len(expected_ids):
        raise ValueError(f"footer reviewed={footer.group(2)} != "
                         f"{len(expected_ids)} proposals")
    for key, idx in (("accepts", 3), ("rebut", 4), ("defers", 5)):
        if int(footer.group(idx)) != dist[key]:
            raise ValueError(f"footer {key}={footer.group(idx)} != real "
                             f"{dist[key]}")
    return verdicts, {"reviewed": len(expected_ids), **dist}


def post_board(body, host):
    """One advisory note post per completed review cycle; board append via
    the shared dataset file (msgboard may be absent on this host)."""
    if SILENT:
        return "silent(test)"
    try:
        p = Path.home() / "data/msggraph/boards/infra/meta/board.jsonl"
        if not p.exists():
            return "board-absent"
        post = {"ts": _now(), "board": BOARD,
                "from": {"row_uuid": f"composer-review-{host}",
                         "harness": "composer-review", "seat": None},
                "kind": "note", "body": body[:3500]}
        with p.open("a") as f:
            f.write(json.dumps(post) + "\n")
        return "posted"
    except OSError as e:
        return f"board-error: {e}"


def write_marker(payload):
    """Atomic marker write; a corrupt previous marker is preserved as
    evidence instead of being silently destroyed."""
    prev_raw = None
    try:
        prev_raw = MARKER.read_text()
    except OSError:
        prev_raw = None
    if prev_raw is not None:
        try:
            json.loads(prev_raw)
        except json.JSONDecodeError:
            try:
                MARKER.rename(MARKER.with_name(
                    f"dispatch.corrupt-{int(time.time())}.json"))
            except OSError:
                pass
    tmp = MARKER.with_name("dispatch.json.tmp")
    with tmp.open("w") as f:
        f.write(json.dumps(payload, indent=1) + "\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, MARKER)


def _emit(payload):
    print(json.dumps(payload))


def _hold_exit(status, detail, batch=None, rc=2):
    record_hold(status, detail, batch)
    if status.startswith("hold:"):
        kind = "hold"
    elif status.startswith("review-unavailable"):
        kind = "review-unavailable"
    elif status.startswith("review-rejected"):
        kind = "review-rejected"
    else:
        kind = "review-failed"
    _emit({"review": kind, "status": status, "batch": batch,
           "debt": "retained", "detail": str(detail)[:200]})
    return rc


def _dispatch(host):
    rows, faults = load_outbox()
    if faults:
        return _hold_exit("hold:outbox-malformed",
                          f"{len(faults)} malformed/partial outbox line(s): "
                          + json.dumps(faults[:3]))
    if not rows:
        _emit({"review": "noop", "why": "outbox-empty"})
        return 0
    batch, entries = build_batch(rows)
    ids = [e["id"] for e in entries]

    marker = read_marker()
    if (marker.get("protocol") == REVIEW_PROTOCOL
            and marker.get("batch") == batch
            and marker.get("status") == "advisory-ok"):
        _emit({"review": "noop", "why": "already-reviewed", "batch": batch})
        return 0

    cd = cooldown_active()
    if cd:
        return _hold_exit("hold:cooldown-backoff",
                          f"consecutive review failures {cd['consecutive_failures']};"
                          f" retry in ~{cd['retry_in_s']}s"
                          f" (last: {cd['last_failure_reason']})", batch)

    outbox_bytes_digest = hashlib.sha256(OUTBOX.read_bytes()).hexdigest()
    batch_dir = REVIEWS / f"batch-{batch}"
    try:
        digest = stage_snapshot(batch_dir, entries)
    except OSError as e:
        return _hold_exit("hold:storage-failure",
                          f"snapshot write failed: {e}", batch)
    if digest is None:
        return _hold_exit("hold:snapshot-conflict",
                          "existing snapshot for this batch differs — "
                          "immutable record never overwritten", batch)
    try:
        brief_text = stage_brief(batch_dir, batch, entries)
        (batch_dir / "brief.md").write_text(brief_text)
    except OSError as e:
        return _hold_exit("hold:storage-failure",
                          f"brief write failed: {e}", batch)

    codex = resolve_transport()
    if codex is None:
        record_failure("transport-absent", batch)
        return _hold_exit("review-unavailable:transport-absent",
                          f"no reviewer transport for model {MODEL}", batch)
    cmd = [codex, "exec", "--skip-git-repo-check",
           "--dangerously-bypass-approvals-and-sandbox",
           "-m", MODEL, "-c", f'model_reasoning_effort="{EFFORT}"',
           brief_text + "\nFull proposal snapshot: ./outbox-snapshot.jsonl"
           " in this directory. Read it before verdicting."]
    try:
        r = subprocess.run(cmd, cwd=batch_dir, capture_output=True,
                           text=True, timeout=TIMEOUT_S)
        stdout = r.stdout or ""
    except subprocess.TimeoutExpired as e:
        record_failure("timeout", batch)
        return _hold_exit("review-failed:timeout", str(e)[:200], batch)
    except FileNotFoundError as e:
        record_failure("transport-absent", batch)
        return _hold_exit("review-unavailable:transport-absent",
                          str(e)[:120], batch)
    if r.returncode != 0:
        record_failure(f"transport-exit-{r.returncode}", batch)
        return _hold_exit("review-failed:transport-exit",
                          f"rc={r.returncode} stderr tail: "
                          f"{(r.stderr or '')[-200:]}", batch)
    try:
        verdicts, counts = parse_verdict(stdout, set(ids), batch)
    except ValueError as e:
        record_failure(f"verdict-schema: {e}", batch)
        try:
            (batch_dir / "verdict-rejected.txt").write_text(stdout)
        except OSError:
            pass
        return _hold_exit("review-rejected:verdict-schema", str(e), batch)

    if hashlib.sha256(OUTBOX.read_bytes()).hexdigest() != outbox_bytes_digest:
        try:
            (batch_dir / "verdict-stale.txt").write_text(stdout)
        except OSError:
            pass
        return _hold_exit("stale-basis",
                          "outbox changed while the review ran — verdict "
                          "not applied; next tick reviews the new batch",
                          batch)

    try:
        (batch_dir / "verdict.md").write_text(stdout)
        (batch_dir / "verdict.json").write_text(json.dumps(
            {"protocol": REVIEW_PROTOCOL, "batch": batch,
             "snapshot_sha256": digest,
             "outbox_sha256": outbox_bytes_digest,
             "proposal_ids": ids,
             "verdicts": {pid: verdicts[pid] for pid in ids},
             "counts": counts, "model": MODEL, "effort": EFFORT,
             "advisory": True, "at": _now()}, indent=1) + "\n")
        write_marker({
            "protocol": REVIEW_PROTOCOL, "batch": batch,
            "snapshot_sha256": digest, "outbox_sha256": outbox_bytes_digest,
            "proposal_ids": ids, "counts": counts,
            "status": "advisory-ok",
            "advisory_note": "brain-pair triage only — NOT independent "
                             "worker-completion acceptance, NOT a "
                             "completion receipt, no repairs executed",
            "dispatched_at": _now(), "review_dir": str(batch_dir)})
    except OSError as e:
        return _hold_exit("hold:storage-failure",
                          f"verdict/marker write failed: {e}", batch)
    record_success()

    tail = "\n".join(stdout.strip().splitlines()[-6:])[:1500]
    _emit({"review": "advisory-ok", "batch": batch, "counts": counts,
           "snapshot_sha256": digest, "advisory": True,
           "board": post_board(
               f"composer-review {host}: batch {batch} reviewed advisorially"
               f" ({len(entries)} proposals, {observation_count()} "
               f"observations). Verdict tail:\n{tail}\n"
               f"Full: {batch_dir}/verdict.md. Repairs are PROPOSED, not "
               f"executed — owning campaigns pick them up. This is triage, "
               f"not completion acceptance.", host)})
    return 0


def main():
    host = os.uname().nodename.split(".")[0].lower()
    if not OUTBOX.exists():
        _emit({"review": "noop", "why": "outbox-absent"})
        return 0
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        lockf = open(LOCK, "a+")
    except OSError as e:
        record_hold("hold:storage-failure", f"state/lock open failed: {e}")
        _emit({"review": "review-failed", "status": "hold:storage-failure",
               "debt": "retained", "detail": str(e)[:200]})
        return 2
    try:
        try:
            fcntl.flock(lockf, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            _emit({"review": "busy",
                   "why": "another dispatch holds the review lock"})
            return 0
        return _dispatch(host)
    finally:
        try:
            fcntl.flock(lockf, fcntl.LOCK_UN)
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
