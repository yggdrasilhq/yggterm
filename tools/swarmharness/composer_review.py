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
  changed content is a NEW batch; an unchanged replay is idempotent. A row
  with duplicate JSON keys (any nesting depth) or keys that collide after
  Unicode normalization has no unambiguous field set and is a typed
  non-canonical hold, never an identity that silently drops a field.
- The reviewer output must bind to the exact batch: one typed verdict line
  per proposal id (accept|rebut|defer + a reason) and a footer echoing THIS
  batch id with counts matching the real proposal distribution. VERDICT-
  shaped lines are validated across the WHOLE response, and BATCH is a
  reserved line-leading token: any BATCH-shaped line that is not the one
  valid footer — malformed, duplicate, or wrong-id with bogus counts —
  rejects the whole response. Trailing commentary that is not verdict- or
  BATCH-shaped stays tolerated. A bare "BATCH" substring, a fabricated or
  wrong batch id, wrong/duplicate/missing ids or counts, a partial or
  malformed response, a nonzero exit, a timeout, an absent transport, or a
  storage failure NEVER advance the success marker. Review debt is retained
  and retried under a bounded cooldown instead of being hidden; storage
  faults AFTER the reviewer ran enter the same retry accounting, and the
  retry ledger itself must be readable AND writable before any paid call —
  a broken ledger blocks dispatch (fail-closed) rather than letting cooldown
  be silently defeated.
- A malformed/partial outbox is a typed observable hold: the last marker is
  never clobbered, prior verdict records are preserved, and the hold is
  appended to review-holds.jsonl. Concurrent dispatches serialize on a
  state lock; a stale outbox basis (changed while the review ran) is
  rejected without applying its verdict.
- The checkpoint brief carries the proposals' evidence inline (bounded)
  and points at the staged snapshot (outbox-snapshot.jsonl) as the
  authoritative bytes, so the reviewer can always read the full record.
- The verdicts are ADVISORY brain-pair triage only — proposed to the owning
  campaign boards. The marker is NOT independent worker-completion
  acceptance, NOT a canonical completion receipt, and executes no repairs.
  A model failure retains review debt.

All paths $HOME-relative: runs on any host with a composer install.
"""
import fcntl
import hashlib
import json
import math
import os
import re
import subprocess
import sys
import tempfile
import time
import unicodedata
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
# Case/plural tolerant on STRUCTURE; every binding property (batch id, id
# set, counts vs real dispositions, dedupe, reason length) stays strict.
VERDICT_RE = re.compile(
    r"^VERDICT\s+([0-9a-f]{16})\s+(accept|rebut|defer)\s+(?:—|--|-)\s+(.+)$",
    re.IGNORECASE)
FOOTER_RE = re.compile(
    r"^BATCH\s+([0-9a-f]{16})\s+reviewed=(\d+)\s+accepts?=(\d+)\s+"
    r"rebuts?=(\d+)\s+defers?=(\d+)\s*$", re.IGNORECASE)


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _nfc(v):
    """Unicode-normalize recursively so two encodings of the SAME string
    (NFC vs NFD) carry one identity, not two. A dict whose keys collide
    after normalization raises ValueError: silently dropping a field would
    change the row's content while identity claimed to cover every field."""
    if isinstance(v, str):
        return unicodedata.normalize("NFC", v)
    if isinstance(v, list):
        return [_nfc(x) for x in v]
    if isinstance(v, dict):
        out = {}
        for k, x in v.items():
            nk = _nfc(k)
            if nk in out:
                raise ValueError(
                    f"key-normalization collision: two keys normalize to "
                    f"{nk!r} — the row has no unambiguous field set")
            out[nk] = _nfc(x)
        return out
    return v


def _canonical(row):
    """Deterministic serialization — the bytes identity is bound to. NFC-
    normalized, true UTF-8 (no \\u escaping of non-ASCII, RFC 8785 style);
    NaN/Infinity are refused (ValueError): they have no canonical form; a
    key-normalization collision is refused for the same reason (F-1). The
    encode/decode round-trip is lossless AND makes lone Unicode surrogates
    raise (UnicodeEncodeError, a ValueError) HERE — at the canonical check
    the loader types them non-canonical — instead of crashing identity
    computation later: identity bytes must be UTF-8-encodable by
    construction."""
    text = json.dumps(_nfc(row), sort_keys=True, separators=(",", ":"),
                      allow_nan=False, ensure_ascii=False)
    return text.encode("utf-8").decode("utf-8")


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


def _no_duplicate_keys(pairs):
    """json object_pairs_hook that refuses duplicate keys at ANY nesting
    depth: json.loads otherwise silently keeps the last value and the
    first key's evidence is discarded before the NFC collision check can
    ever see it (editor finding F-5, round-4 review)."""
    seen = set()
    for k, _ in pairs:
        if k in seen:
            raise ValueError(f"duplicate JSON key {k!r} in one object")
        seen.add(k)
    return dict(pairs)


def load_outbox_bytes(data):
    """-> (rows, faults) over ONE raw byte read (the same bytes the basis
    digest covers, so parse and basis can never disagree). Partial/
    malformed lines are typed, never raised."""
    faults, rows = [], []
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as e:
        return [], [{"line": 0, "fault": "undecodable-outbox",
                     "detail": str(e)[:120]}]
    for ln, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line, object_pairs_hook=_no_duplicate_keys)
        except json.JSONDecodeError as e:
            faults.append({"line": ln, "fault": "malformed-json",
                           "detail": str(e)[:120]})
            continue
        except ValueError as e:
            faults.append({"line": ln, "fault": "duplicate-key",
                           "detail": str(e)[:120]})
            continue
        if not isinstance(row, dict):
            faults.append({"line": ln, "fault": "not-an-object"})
            continue
        try:
            _canonical(row)
        except ValueError as e:
            faults.append({"line": ln, "fault": "non-canonical-row",
                           "detail": str(e)[:120] or
                           "row has no canonical JSON form"})
            continue
        rows.append(row)
    return rows, faults


def load_outbox(path=None):
    """Wrapper for callers that just want the current file parsed."""
    try:
        return load_outbox_bytes(Path(path or OUTBOX).read_bytes())
    except OSError as e:
        return [], [{"line": 0, "fault": "unreadable-outbox",
                     "detail": str(e)[:120]}]


def observation_count():
    try:
        with OBSERVATIONS.open(encoding="utf-8") as f:
            return sum(1 for _ in f)
    except (OSError, UnicodeError):
        return 0


def needs_state():
    try:
        d = json.loads(NEEDS.read_text(encoding="utf-8"))
        return {"available": bool(d.get("available")),
                "reason": str(d.get("reason", ""))[:120]}
    except (OSError, ValueError):
        return {"available": False, "reason": "needs snapshot absent"}


def resource_bound():
    """Native read bound actually reached by the miner (cursor bytes), when
    the state carries it; absence is stated, never invented."""
    try:
        cur = json.loads(CURSORS.read_text(encoding="utf-8"))
        total = sum(int(c.get("offset", 0)) for c in cur.values()
                    if isinstance(c, dict))
        return {"sources_tracked": len(cur), "bytes_cursor_total": total}
    except (OSError, ValueError, TypeError):
        return {"sources_tracked": 0, "bytes_cursor_total": 0,
                "note": "cursor state absent"}


def record_hold(status, detail, batch=None):
    """Append-only typed hold log — prior records are never rewritten."""
    row = {"at": _now(), "status": status, "detail": str(detail)[:400]}
    if batch:
        row["batch"] = batch
    try:
        HOLDS.parent.mkdir(parents=True, exist_ok=True)
        with HOLDS.open("a", encoding="utf-8") as f:
            f.write(json.dumps(row) + "\n")
    except (OSError, UnicodeError):
        pass  # the typed stdout record remains the observable
    return row


def load_attempts():
    try:
        raw = ATTEMPTS.read_text(encoding="utf-8")
    except FileNotFoundError:
        return {}
    data = json.loads(raw)
    if not isinstance(data, dict):
        raise ValueError("retry ledger must be an object")
    count = data.get("consecutive_failures", 0)
    if type(count) is not int or count < 0:
        raise ValueError("retry failure count must be a nonnegative integer")
    epoch = data.get("last_failure_epoch")
    if epoch is not None or count:
        if (type(epoch) not in (int, float) or not math.isfinite(epoch)
                or epoch < 0):
            raise ValueError("retry failure epoch must be finite and nonnegative")
    for key in ("last_failure_at", "last_failure_reason", "last_batch"):
        if key in data and not isinstance(data[key], str):
            raise ValueError(f"retry {key} must be text")
    return data


def record_failure(reason, batch=None):
    """Persist one failure into the retry ledger. A write failure here is
    tolerated ONLY because the admission gate below refuses any paid
    transport call while the ledger is unusable: at most one invocation
    can run unaccounted per ledger-breakage event (the gate blocks every
    subsequent dispatch until the ledger is repaired), so cooldown can
    never be silently defeated (editor finding F-7, round-4 review)."""
    try:
        a = load_attempts()
    except (OSError, UnicodeError, ValueError):
        # Preserve corrupt or unavailable history. The next admission holds;
        # never erase a broken retry bound by treating it as zero failures.
        return None
    a["consecutive_failures"] = int(a.get("consecutive_failures", 0)) + 1
    a["last_failure_at"] = _now()
    a["last_failure_epoch"] = time.time()
    a["last_failure_reason"] = str(reason)[:200]
    if batch:
        a["last_batch"] = batch
    try:
        ATTEMPTS.write_text(json.dumps(a, indent=1), encoding="utf-8")
    except (OSError, UnicodeError):
        pass
    return a


def attempts_ledger_usable():
    """Fail-closed admission gate, checked BEFORE any paid transport
    invocation. The ledger must be readable AND writable, or bounded
    retry accounting is unprovable: load_attempts would report an empty
    history and cooldown would never engage. Returns None when usable,
    else the typed fault string."""
    try:
        load_attempts()
    except (OSError, UnicodeError, ValueError) as e:
        return f"retry ledger invalid or unreadable: {str(e)[:120]}"
    try:
        if ATTEMPTS.exists():
            with ATTEMPTS.open("a", encoding="utf-8"):
                pass
        else:
            # An absent ledger means no failures. Probe writability in a
            # separate file: stale-basis and staging holds must not leave an
            # empty, invalid JSON ledger that prevents the next healthy retry.
            with tempfile.NamedTemporaryFile(
                    dir=ATTEMPTS.parent, prefix=".retry-probe-") as probe:
                probe.write(b"{}\n")
                probe.flush()
                os.fsync(probe.fileno())
    except (OSError, UnicodeError) as e:
        return f"retry ledger unwritable: {str(e)[:120]}"
    return None


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
        return json.loads(MARKER.read_text(encoding="utf-8"))
    except (OSError, ValueError):
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
    digest = hashlib.sha256(payload.encode("utf-8")).hexdigest()
    existing = snapshot_digest(batch_dir)
    if existing is not None:
        return digest if existing == digest else None
    batch_dir.mkdir(parents=True, exist_ok=True)
    (batch_dir / SNAPSHOT_NAME).write_text(payload, encoding="utf-8")
    return digest


def _evidence_lines(ev):
    """Bounded inline evidence for one proposal: the first rows verbatim,
    the rest pointed at the staged snapshot. A non-list evidence field is
    still shown, never silently counted away."""
    if not isinstance(ev, list):
        ev = [ev]
    out = []
    for i, row in enumerate(ev[:3]):
        try:
            text = json.dumps(row, sort_keys=True, ensure_ascii=False)
        except (TypeError, ValueError):
            text = repr(row)
        out.append(f"    [{i}] {text[:400]}")
    if len(ev) > 3:
        out.append(f"    ... {len(ev) - 3} more evidence row(s) — read the "
                   f"full bytes from {SNAPSHOT_NAME}")
    return out


def stage_brief(batch_dir, batch, entries):
    """Criterion-bound checkpoint brief: baseline, evidence, falsifier,
    missingness and the native read bound — then the exact output contract.
    The evidence is shown inline (bounded) and the staged snapshot is
    explicitly pointed at: the reviewer can always reach the complete
    canonical proposal bytes, so the count-only summary can never be the
    whole story. Returns the brief text (written by the caller, so write
    errors stay typed at the dispatch site)."""
    blocks = []
    for e in entries:
        r = e["row"]
        ev = r.get("evidence")
        if isinstance(ev, list):
            ev_rows = ev
        elif ev is None:
            ev_rows = []
        else:
            # a non-list evidence field is still evidence: shown as one
            # row, never counted away and never a crash (editor finding
            # F-8, round-4 review)
            ev_rows = [ev]
        blocks.append(
            f"- {e['id']} pattern={r.get('pattern_id', '(none)')} "
            f"detector={r.get('detector_id', '(none)')}:"
            f"{r.get('detector_version', '(none)')} "
            f"lane={r.get('lane', '(none)')}\n"
            f"  assertion: {str(r.get('assertion', '(missing)'))[:300]}\n"
            f"  falsifier: {str(r.get('disproof', '(missing)'))[:300]}\n"
            f"  evidence rows: {len(ev_rows)}"
            + ("" if ev_rows else "  [MISSINGNESS: no machine-checkable "
                                   "evidence rows attached — treat as defer "
                                   "candidate]")
            + ("\n" + "\n".join(_evidence_lines(ev_rows)) if ev_rows else ""))
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

## Proposal snapshot (authoritative bytes)
The complete canonical proposal bytes for THIS batch — every field,
including the full evidence rows summarized above — are staged in THIS
directory as {SNAPSHOT_NAME}. Read it before deciding: the summaries below
are bounded, the snapshot is the record.

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
  id set, or count makes the whole review invalid. EVERY VERDICT-shaped
  line anywhere in your output is validated against these rules — place
  all verdict lines before the footer, never repeat one, and keep any
  trailing commentary free of verdict-shaped text.
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
    Prose, markdown fences and structural case/plural variants are
    tolerated; every binding property (batch id, id set, counts, dedupe,
    reason length) is not. VERDICT-shaped lines are validated across the
    WHOLE response, not only above the footer: permitted trailing
    commentary must never conceal an unknown, duplicate, contradictory or
    malformed verdict line."""
    raw = [l.strip() for l in (stdout or "").splitlines() if l.strip()]
    # markdown fence lines (``` or ```lang) are wrapping, never content
    lines = [l for l in raw if not l.startswith("```")]
    if not lines:
        raise ValueError("empty stdout — no verdict")
    footers = [(i, FOOTER_RE.match(l)) for i, l in enumerate(lines)
               if FOOTER_RE.match(l)]
    if not footers:
        raise ValueError("no valid BATCH footer found")
    if len(footers) > 1:
        raise ValueError("multiple BATCH footers")
    _, footer = footers[0]
    footer_line = footer.group(0)
    if footer.group(1).lower() != expected_batch:
        raise ValueError(f"footer batch {footer.group(1)} != expected "
                         f"{expected_batch}")
    # BATCH is a reserved line-leading token: any BATCH-shaped line that is
    # not the ONE valid footer — a malformed footer, a second footer, a
    # wrong-id footer with bogus counts — rejects the whole response.
    # Ordinary commentary that merely mentions BATCH mid-line stays
    # tolerated (editor finding F-6, round-4 review).
    for l in lines:
        if l.upper().startswith("BATCH") and l != footer_line:
            raise ValueError(f"malformed or extra BATCH line: {l[:80]}")
    verdicts = {}
    for l in lines:
        if not l.upper().startswith("VERDICT"):
            continue
        m = VERDICT_RE.match(l)
        if not m:
            raise ValueError(f"malformed VERDICT line: {l[:80]}")
        pid = m.group(1).lower()
        disp = m.group(2).lower()
        reason = m.group(3).strip()
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
        with p.open("a", encoding="utf-8") as f:
            f.write(json.dumps(post) + "\n")
        return "posted"
    except (OSError, UnicodeError) as e:
        return f"board-error: {e}"


def write_marker(payload):
    """Atomic marker write; a corrupt previous marker is preserved as
    evidence instead of being silently destroyed."""
    prev_raw = None
    try:
        prev_raw = MARKER.read_text(encoding="utf-8")
    except OSError:
        prev_raw = None
    except UnicodeError:
        prev_raw = ""  # undecodable bytes == corrupt
    if prev_raw is not None:
        try:
            json.loads(prev_raw)
        except ValueError:
            try:
                MARKER.rename(MARKER.with_name(
                    f"dispatch.corrupt-{int(time.time())}.json"))
            except OSError:
                pass
    tmp = MARKER.with_name("dispatch.json.tmp")
    with tmp.open("w", encoding="utf-8") as f:
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
    # ONE raw read binds parse and basis: rows, faults and outbox_bytes_digest
    # all derive from the same bytes, so a proposal appended between the parse
    # and the digest can never be silently stamped over (no parse/digest split).
    try:
        data = OUTBOX.read_bytes()
    except OSError as e:
        return _hold_exit("hold:outbox-unreadable", str(e)[:200])
    rows, faults = load_outbox_bytes(data)
    outbox_bytes_digest = hashlib.sha256(data).hexdigest()
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

    ledger_fault = attempts_ledger_usable()
    if ledger_fault:
        return _hold_exit("hold:retry-ledger-unavailable", ledger_fault,
                          batch)
    try:
        cd = cooldown_active()
    except (OSError, UnicodeError, ValueError) as e:
        return _hold_exit("hold:retry-ledger-unavailable", str(e)[:200], batch)
    if cd:
        return _hold_exit("hold:cooldown-backoff",
                          f"consecutive review failures {cd['consecutive_failures']};"
                          f" retry in ~{cd['retry_in_s']}s"
                          f" (last: {cd['last_failure_reason']})", batch)

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
        (batch_dir / "brief.md").write_text(brief_text, encoding="utf-8")
    except (OSError, UnicodeError) as e:
        return _hold_exit("hold:storage-failure",
                          f"brief write failed: {e}", batch)

    codex = resolve_transport()
    if codex is None:
        record_failure("transport-absent", batch)
        return _hold_exit("review-unavailable:transport-absent",
                          f"no reviewer transport for model {MODEL}", batch)
    # ASCII pointer prompt — the full brief is staged as ./brief.md in the
    # transport cwd. The brief text itself is NEVER passed as argv: argv is
    # locale/ARG_MAX fragile (a C-locale host cannot even encode an
    # em-dash there) and the file is the authoritative bytes anyway.
    prompt = ("You are the composer brain-pair reviewer. Read ./brief.md in "
              "this directory IN FULL (it carries the checkpoint brief, the "
              "proposal snapshot pointer and the exact output contract), "
              "then follow it exactly.")
    cmd = [codex, "exec", "--skip-git-repo-check",
           "--dangerously-bypass-approvals-and-sandbox",
           "-m", MODEL, "-c", f'model_reasoning_effort="{EFFORT}"', prompt]
    try:
        # Deterministic decode of transport output: model CLIs emit UTF-8;
        # locale-locked hosts must not crash on it (errors=replace keeps
        # weird bytes visible without breaking validation).
        r = subprocess.run(cmd, cwd=batch_dir, capture_output=True,
                           encoding="utf-8", errors="replace",
                           timeout=TIMEOUT_S)
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
            (batch_dir / "verdict-rejected.txt").write_text(
                stdout, encoding="utf-8")
        except (OSError, UnicodeError):
            pass
        return _hold_exit("review-rejected:verdict-schema", str(e), batch)

    try:
        changed = (hashlib.sha256(OUTBOX.read_bytes()).hexdigest()
                   != outbox_bytes_digest)
    except OSError as e:
        # post-invocation storage fault: typed hold AND retry accounting,
        # so a persistent write fault cannot trigger repeated paid calls
        record_failure(f"basis-re-read: {e}", batch)
        return _hold_exit("hold:storage-failure",
                          f"basis re-read failed after review: {e}", batch)
    if changed:
        try:
            (batch_dir / "verdict-stale.txt").write_text(
                stdout, encoding="utf-8")
        except (OSError, UnicodeError):
            pass
        return _hold_exit("stale-basis",
                          "outbox changed while the review ran — verdict "
                          "not applied; next tick reviews the new batch",
                          batch)

    try:
        (batch_dir / "verdict.md").write_text(stdout, encoding="utf-8")
        (batch_dir / "verdict.json").write_text(json.dumps(
            {"protocol": REVIEW_PROTOCOL, "batch": batch,
             "snapshot_sha256": digest,
             "outbox_sha256": outbox_bytes_digest,
             "proposal_ids": ids,
             "verdicts": {pid: verdicts[pid] for pid in ids},
             "counts": counts, "model": MODEL, "effort": EFFORT,
             "advisory": True, "at": _now()}, indent=1) + "\n",
            encoding="utf-8")
        write_marker({
            "protocol": REVIEW_PROTOCOL, "batch": batch,
            "snapshot_sha256": digest, "outbox_sha256": outbox_bytes_digest,
            "proposal_ids": ids, "counts": counts,
            "status": "advisory-ok",
            "advisory_note": "brain-pair triage only — NOT independent "
                             "worker-completion acceptance, NOT a "
                             "completion receipt, no repairs executed",
            "dispatched_at": _now(), "review_dir": str(batch_dir)})
    except (OSError, UnicodeError) as e:
        # post-invocation storage fault: the paid transport call already
        # happened, so this failure MUST enter the retry accounting — a
        # persistent write fault otherwise re-invokes the reviewer every
        # tick with no cooldown (editor finding F-3, round-3 review).
        record_failure(f"storage-write: {e}", batch)
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
            # Contention is serialization working, not debt: distinct rc=3
            # observable so no caller can mistake busy for success OR debt.
            _emit({"review": "busy",
                   "why": "another dispatch holds the review lock"})
            return 3
        return _dispatch(host)
    finally:
        try:
            fcntl.flock(lockf, fcntl.LOCK_UN)
        except OSError:
            pass
        lockf.close()


if __name__ == "__main__":
    sys.exit(main())
