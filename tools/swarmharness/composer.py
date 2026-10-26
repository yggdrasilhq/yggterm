"""composer — the fleet lesson miner (owner directive 2026-10-26; sol architecture
2026-10-26; v2 per sol turn-25 conformance review, 13 findings closed).

A sibling deterministic service (maintenance.py pattern) subordinate to the
dream orchestrator: it mines enrolled session corpora for RECURRING fault
patterns, shapes them into falsifiable claims, and proposes work — it never
dispatches, never spawns workers, never publishes lessons by itself.

Identity model (sol t25 #1/#3): a source has a stable LINEAGE (enrolled
path) and a content GENERATION token (size + head64KiB + tail64KiB hashes).
Incidents are counted per LINEAGE — an appended line is the same lineage,
never a second incident. Observations bind their generation; a generation
change mid-read aborts that source with an enumerated gap, never records
cross-generation evidence.

Storage contract (sol t25 #2/#11): observations.jsonl enforces a closed
typed schema, recomputed dedupe keys, and uniqueness at commit — identical
replay is a no-op, conflicting same-key records are an integrity error.

Detectors (sol t25 #6/#7/#8): lexical signature hits are UNVERIFIED SYMPTOM
candidates (medium severity, never "high"); cmd-failure-loop requires the
command fingerprint to co-occur with an error signature. Fingerprints are
computed over NORMALIZED text (case/digits/whitespace collapsed — transport
metadata excluded). Patterns partition by detector version. Pages carry
incomplete trailing lines forward; spans are byte offsets.

Bounds (sol t25 #5/#9/#10/#12): every tick has page/byte budgets and
backlog reporting; the outbox has a capacity cap; exploration lanes get
RESERVED emission slots over the seen-filtered candidate list; cursors are
validated (corruption resets explicitly — the deduping replay protects);
needs snapshots are validated and freshness-checked.

Privacy: host-local extraction; observations carry fingerprints, counts and
byte spans — never message bodies. Zero model calls.

Brain-pair attendance law (owner 2026-10-26): the dream orchestrator and
the composer are the SWARM INTELLIGENCE area — the model/harness pair that
attends to them (mining analysis, sweep dreams, editorial verdicts,
orchestration decisions) is the brainiest pair in swarm-config.yaml,
currently sol-high/codex, and nothing else. Mining SCOPE stays all sessions
of all harnesses; attendance is brain-pair only.
"""
import fcntl
import hashlib
import json
import os
import re
from datetime import datetime, timezone
from pathlib import Path

from swarmconfig import atomic_write_json
from session_review import SessionReview

DEFAULT_STATE = Path("~/.yggterm/swarm/composer")
DETECTOR_VERSION = "det-v3"
SUPPORTED_SOURCES = ("zcode_rollout", "codex_session", "queue_run",
                     "review_chain", "board")
_SEVERITIES = ("high", "medium", "low")
_MAX_CARRY = 65536  # oversized-event policy bound


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _sha(data):
    if isinstance(data, str):
        data = data.encode()
    return hashlib.sha256(data).hexdigest()


_TRANSPORT_TOKEN = re.compile(
    r"\b(ts|id|call_id|call|session|request_id|req_id|timestamp|trace|span|uid)"
    r"\"?\s*[:=]\s*(?:\"(?:\\.|[^\"\\])*\"|[^\s,\]\}]+)", re.I)
_BRACKET = re.compile(r"\[([^\]]*)\]")


def _normalize(text):
    """Normalized detector evidence. ONLY recognized transport tokens
    (ts=, id=, call=, ...) and transport-looking brackets collapse —
    substantive JSON content (api_key, endpoint, distinct names) is
    PRESERVED so genuinely different faults fingerprint differently
    (sol t26 #7/N2). Digits and case collapse after."""
    s = text.strip().lower()
    s = _TRANSPORT_TOKEN.sub("[M]", s)

    def _br(m):
        inner = m.group(1)
        return "[M]" if _TRANSPORT_TOKEN.search(inner) else m.group(0)

    s = _BRACKET.sub(_br, s)
    s = re.sub(r"\d+", "N", s)
    s = re.sub(r"\s+", " ", s)
    return s


def generation(path):
    """Content generation token from ONE opened handle (fstat + reads off
    the same fd) — no stat/read race can pair old bytes with a new size
    (sol t26 #4). Covers tail mutations (sol t25 #3)."""
    path = Path(path)
    with path.open("rb") as f:
        st = os.fstat(f.fileno())
        head = f.read(65536)
        if st.st_size > 65536:
            f.seek(-65536, os.SEEK_END)
            tail = f.read(65536)
        else:
            tail = b""
    return f"sz{st.st_size}:{_sha(head)[:12]}:{_sha(tail)[:12]}"


def generation_and_size(path):
    """(generation_token, size) off ONE opened handle — census metadata can
    never pair an old generation with a new byte count (sol t26 #4)."""
    path = Path(path)
    with path.open("rb") as f:
        st = os.fstat(f.fileno())
        head = f.read(65536)
        if st.st_size > 65536:
            f.seek(-65536, os.SEEK_END)
            tail = f.read(65536)
        else:
            tail = b""
    token = f"sz{st.st_size}:{_sha(head)[:12]}:{_sha(tail)[:12]}"
    return token, st.st_size


def lineage_id(path):
    """Lineage identity from the INODE (st_dev:st_ino) after realpath —
    symlinks AND hard links collapse to one physical source (sol t28 #1)."""
    rp = Path(path).resolve()
    st = os.stat(rp)
    return _sha(f"{st.st_dev}:{st.st_ino}")[:16]


# ------------------------------------------------------------ detector set

_HARNESS_SIGNATURES = [
    ("ssh-argv-join", re.compile(r"syntax error near unexpected token|bash: -c: line")),
    ("heredoc-expansion", re.compile(r"command not found:?\s*$", re.M)),
    ("nameerror-typo", re.compile(r"NameError: name '.*' is not defined")),
    ("kwarg-dup", re.compile(r"got multiple values for keyword argument")),
    ("pyc-stale", re.compile(r"Unsupported bytecode|bad marshal data")),
    ("param-shadow", re.compile(r"got an unexpected keyword argument")),
]
_CMD_PREFIXES = ("$ ", "+ ")
KNOWN_DETECTORS = (f"sig-{sid}" for sid in
                   ("ssh-argv-join", "heredoc-expansion", "nameerror-typo",
                    "kwarg-dup", "pyc-stale", "param-shadow"))
KNOWN_DETECTORS = tuple(KNOWN_DETECTORS) + ("cmd-failure-loop",)


def _detect_line(line):
    for sig_id, rx in _HARNESS_SIGNATURES:
        if rx.search(line):
            return sig_id
    return None


def make_observation(detector_id, severity, source_kind, gen, span, fp,
                     lineage, status="unverified-symptom"):
    body = {"detector_id": detector_id, "detector_version": DETECTOR_VERSION,
            "severity": severity, "status": status,
            "source_kind": source_kind, "generation": gen,
            "lineage_id": lineage, "span": span, "fingerprint": fp,
            "at": _now()}
    body["dedupe_key"] = _sha(_canonical(
        [source_kind, lineage, gen, span, detector_id, DETECTOR_VERSION,
         fp]))
    return body


_OBS_KEYS = {"detector_id", "detector_version", "severity", "status",
             "source_kind", "generation", "lineage_id", "span",
             "fingerprint", "at", "dedupe_key"}


def validate_observation(obs):
    """Closed schema + recomputed key (sol t25 #11). Storage refuses
    anything else — forged fields, unknown severities, arbitrary bodies."""
    if not isinstance(obs, dict) or set(obs) != _OBS_KEYS:
        missing = _OBS_KEYS - set(obs or {})
        extra = set(obs or {}) - _OBS_KEYS
        return (f"schema violation: missing={sorted(missing)} "
                f"extra={sorted(extra)}")
    if obs["detector_version"] != DETECTOR_VERSION:
        return f"foreign detector version {obs['detector_version']}"
    if obs["severity"] not in _SEVERITIES:
        return f"invalid severity {obs['severity']!r}"
    if obs["status"] not in ("verified-symptom", "unverified-symptom"):
        return f"invalid status {obs['status']!r}"
    for k in ("detector_id", "source_kind", "generation", "span",
              "fingerprint", "at", "dedupe_key", "lineage_id"):
        if not isinstance(obs[k], str) or not obs[k]:
            return f"{k} must be a nonempty string"
    key = _sha(_canonical(
        [obs["source_kind"], obs["lineage_id"], obs["generation"],
         obs["span"], obs["detector_id"], obs["detector_version"],
         obs["fingerprint"]]))
    if key != obs["dedupe_key"]:
        return "dedupe key mismatch (forged or stale observation)"
    return None


def extract(source_kind, path, gen, lineage, page_text, byte_start,
            line_offs=None):
    """Single-page detector pass with ABSOLUTE byte-offset spans. Returns
    (observations, sig_at). Command-loop state is handled by
    extract_page_with_state."""
    observations = []
    lines = page_text.splitlines(keepends=True)  # CRLF-exact offsets (P6)
    if line_offs is None or len(line_offs) < len(lines):
        line_offs = []
        off = byte_start
        for line in lines:
            line_offs.append(off)
            off += len(line.encode("utf-8", errors="replace"))
    sig_at = {}
    for i, line in enumerate(lines):
        bare = line.rstrip("\r\n")
        sig = _detect_line(bare)
        if sig:
            sig_at[i] = sig
            observations.append(make_observation(
                f"sig-{sig}", "medium", source_kind, gen,
                f"bytes@{line_offs[i]}", _sha(_normalize(bare))[:16],
                lineage, status="unverified-symptom"))
    return observations, sig_at


def extract_page_with_state(source_kind, path, gen, lineage, page_text,
                            byte_start, cmd_fps, pending_fps=None,
                            att_offs=None, line_offs=None):
    """Detectors over one page of COMPLETE lines, with persistent per-source
    loop state. State carries fingerprints, window counters and first-attempt
    adjacency offsets — never content (sol t26 N3). Returns
    (observations, pending_fps, att_offs)."""
    # Adjacency protocol (sol t30 P2, t25 #6): IN-PAGE, a repeated command
    # with an error in the three lines after ANY of its instances closes at
    # that error line (settled semantics). CROSS-PAGE, a repeated command
    # with no in-page closure carries a HUNT {fingerprint, three-line
    # window, byte offset}; every page line decrements the window; an error
    # line closes the hunt while its window still reaches that line, AT THE
    # ERROR LINE'S OFFSET; a hunt past its window expires (the old
    # head-of-page shortcut matched pending fingerprints to distant,
    # unrelated errors). State carries fingerprints + counters only — never
    # content (sol t26 N3).
    hunting = []
    for p in pending_fps or []:
        if isinstance(p, dict) and isinstance(p.get("fp"), str) \
                and isinstance(p.get("window"), int) \
                and isinstance(p.get("byte_off"), int):
            hunting.append({"fp": p["fp"], "window": p["window"],
                            "byte_off": p["byte_off"]})
    att_offs = dict(att_offs or {})
    observations = []
    lines = page_text.splitlines(keepends=True)  # CRLF-exact offsets (P6)
    if line_offs is None or len(line_offs) < len(lines):
        line_offs = []
        off = byte_start
        for line in lines:
            line_offs.append(off)
            off += len(line.encode("utf-8", errors="replace"))
    bare = [line.rstrip("\r\n") for line in lines]

    # 1. signature observations; error lines are the closure anchors
    err_lines = []
    for i, line in enumerate(bare):
        sig = _detect_line(line)
        if sig:
            err_lines.append(i)
            observations.append(make_observation(
                f"sig-{sig}", "medium", source_kind, gen,
                f"bytes@{line_offs[i]}", _sha(_normalize(line))[:16],
                lineage, status="unverified-symptom"))

    # 2. count command instances across the page
    instances = {}
    for i, line in enumerate(bare):
        s = line.strip()
        if s.startswith(_CMD_PREFIXES):
            fp = _sha(_normalize(s))[:16]
            cmd_fps[fp] = cmd_fps.get(fp, 0) + 1
            instances.setdefault(fp, []).append(i)

    # 3. previous-page hunts close while their window still reaches an
    # error line of THIS page. A closure on a count>=2 fingerprint emits
    # immediately; on a count-1 fingerprint it RETAINS the error offset
    # until recurrence (sol t32 F32.2) — count-1 commands carry hunts too.
    # Unspent windows ride on.
    still = []
    resolved = set()  # fingerprints that emitted OR retained this call
    for h in hunting:
        close = next((e for e in err_lines if h["window"] - (e + 1) >= 0),
                     None)
        if h["fp"] in resolved:
            continue  # consumed fingerprints never double-close (F33.1)
        if close is not None:
            if cmd_fps.get(h["fp"], 0) >= 2:
                observations.append(make_observation(
                    "cmd-failure-loop", "medium", source_kind, gen,
                    f"bytes@{line_offs[close]}", h["fp"], lineage,
                    status="unverified-symptom"))
                att_offs.pop(h["fp"], None)  # evidence consumed once
            else:
                att_offs.setdefault(h["fp"], line_offs[close])
            resolved.add(h["fp"])
        elif h["window"] - len(bare) > 0:
            still.append({**h, "window": h["window"] - len(bare)})

    # 4. in-page closure (t25 #6). First-attempt retention (sol t31 F31.3,
    # t32 F32.2): a count-1 command with an adjacent error retains that
    # error's offset; recurrence emits at the retained offset — page
    # boundaries no longer decide whether a loop exists. Every instance
    # carries its remaining window; a fingerprint that emitted here never
    # re-carries (no double emission).
    for fp, idxs in instances.items():
        if fp in resolved:
            continue  # already closed/retained via a carried hunt (F33.1)
        hit = next((e for i in idxs for e in err_lines if i < e <= i + 3),
                   None)
        if hit is not None:
            if cmd_fps[fp] >= 2:
                observations.append(make_observation(
                    "cmd-failure-loop", "medium", source_kind, gen,
                    f"bytes@{line_offs[hit]}", fp, lineage,
                    status="unverified-symptom"))
                att_offs.pop(fp, None)  # fresh closure consumes evidence
            else:
                att_offs.setdefault(fp, line_offs[hit])
            resolved.add(fp)
        elif fp in att_offs and cmd_fps[fp] >= 2:
            observations.append(make_observation(
                "cmd-failure-loop", "medium", source_kind, gen,
                f"bytes@{att_offs[fp]}", fp, lineage,
                status="unverified-symptom"))
            att_offs.pop(fp, None)  # emit once per retained evidence pair
            resolved.add(fp)
        rem = 3 - (len(bare) - 1 - idxs[-1])
        if rem > 0 and fp not in resolved:
            still.append({"fp": fp, "window": rem,
                          "byte_off": line_offs[idxs[-1]]})
    pending_fps = still
    # dedupe observations by dedupe key (a fingerprint both carried and
    # locally looped emits once)
    uniq = {}
    for o in observations:
        uniq[o["dedupe_key"]] = o
    return list(uniq.values()), pending_fps, att_offs


# ------------------------------------------------------------ census

def census(sources, cutoff=None, state_dir=None):
    """Enumerate enrolled corpora frozen at cutoff. Nonregular/unreadable
    sources are gaps — a directory or unavailable stat is NEVER 'ok'
    (sol t25 #4)."""
    manifest = {"kind": "census", "cutoff": cutoff or _now(),
                "created_at": _now(), "sources": [], "gaps": [],
                "total_bytes": 0, "lineages": 0, "_lineage_set": set()}
    for kind, paths in (sources or {}).items():
        if kind not in SUPPORTED_SOURCES:
            manifest["gaps"].append(
                {"source_kind": kind, "status": "unsupported_kind"})
            continue
        for path in paths:
            p = Path(path).expanduser()
            entry = {"source_kind": kind, "path": str(p)}
            if not p.exists():
                entry["status"] = "missing"
                manifest["gaps"].append(entry)
                continue
            if not p.is_file():
                entry["status"] = "not-a-regular-file"
                manifest["gaps"].append(entry)
                continue
            try:
                entry["lineage_id"] = lineage_id(p)
            except OSError as exc:
                entry["status"] = f"error: {exc}"[:80]
                manifest["gaps"].append(entry)
                manifest["sources"].append(entry)
                continue
            try:
                entry["status"] = "ok"
                entry["generation"], entry["bytes"] = \
                    generation_and_size(p)
                manifest["total_bytes"] += entry["bytes"]
                manifest.setdefault("_lineage_set", set())
                manifest["_lineage_set"].add(entry["lineage_id"])
                manifest["lineages"] = len(manifest["_lineage_set"])
            except OSError as exc:
                entry["status"] = f"error: {exc}"[:80]
                manifest["gaps"].append(entry)
            manifest["sources"].append(entry)
    manifest["_lineage_count"] = len(manifest.get("_lineage_set", set()))
    manifest["lineages"] = manifest["_lineage_count"]
    manifest.pop("_lineage_set", None)
    return manifest


# ------------------------------------------------------------ read

def read_page(path, cursor=0, max_bytes=262144):
    """Bounded page of COMPLETE records. Bytes are buffered RAW, framed by
    the BYTE splitlines grammar (LF, CRLF and bare CR all terminate a
    record — sol t33 F33.4), and decoded once per emitted block; cursor
    arithmetic is byte-exact. The cursor only ever advances over EMITTED
    bytes (progress guaranteed, sol t28 V3-N1). The oversized bound applies
    to the individual leading unfinished record. Returns (text,
    next_cursor, done, line_offs) — RAW-BYTE start offsets of the emitted
    complete records (sol t31 F31.4)."""
    if not isinstance(max_bytes, int) or isinstance(max_bytes, bool) \
            or max_bytes <= 0:
        raise ValueError(
            f"max_bytes must be a positive int, got {max_bytes!r} (F33.6)")
    p = Path(path)
    size = p.stat().st_size
    raw = bytearray()
    pos = cursor
    eof = False
    with p.open("rb") as f:
        f.seek(pos)
        while True:
            chunk = f.read(max_bytes)
            if not chunk:
                eof = True
                break  # EOF: everything read is final
            raw += chunk
            pos += len(chunk)
            pieces = raw.splitlines(keepends=True)  # byte record grammar
            if pieces and (pieces[0].endswith(b"\n")
                           or pieces[0].endswith(b"\r")):
                break  # at least one complete record is available
            if len(raw) > _MAX_CARRY:
                # the leading UNFINISHED record alone exceeds the policy
                # bound: drop exactly that record, advance past it (F33.4)
                first = pieces[0] if pieces else raw
                marker = f"<<oversized-event dropped at bytes@{cursor}>>\n"
                return marker, cursor + len(first), False, [cursor]
    complete = []
    partial = b""
    if raw:
        pieces = raw.splitlines(keepends=True)
        if pieces and not (pieces[-1].endswith(b"\n")
                           or pieces[-1].endswith(b"\r")):
            partial = pieces[-1]
            pieces = pieces[:-1]
        complete = pieces
    if eof and partial:
        # the trailing unterminated record is the FINAL tail — emit it now
        complete.append(partial)
        partial = b""
    emitted = b"".join(complete)
    # raw-BYTE record starts: piece lengths are exact byte counts in the
    # same framing grammar; replacement decoding stays detection-only
    starts, s = [], 0
    for piece in complete:
        starts.append(cursor + s)
        s += len(piece)
    text = emitted.decode("utf-8", errors="replace")
    next_cursor = min(cursor + len(emitted), size)  # byte-exact
    # done means EMITTED everything (sol t30 P5): an unterminated tail
    # record must still be read — the handle reaching EOF is not done
    return text, next_cursor, next_cursor >= size, starts


# ------------------------------------------------------------ store

def load_observations(state_dir=None):
    path = (Path(state_dir).expanduser() if state_dir
            else DEFAULT_STATE.expanduser()) / "observations.jsonl"
    if not path.exists():
        return []
    out = []
    for line in path.read_text().splitlines():
        try:
            out.append(json.loads(line))
        except Exception:
            continue
    return out


def _atomic_append_jsonl(path, rows):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    existing = []
    if path.exists():
        existing = [ln for ln in path.read_text().splitlines() if ln.strip()]
    tmp = path.with_name(path.name + f".tmp{os.getpid()}")
    with tmp.open("w") as f:
        for ln in existing:
            f.write(ln + "\n")
        for row in rows:
            f.write(_canonical(row) + "\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def _atomic_write_json(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + f".tmp{os.getpid()}")
    tmp.write_text(_canonical(obj))
    os.replace(tmp, path)


def commit_batch(state_dir, entry, observations, cursor, stats):
    """Atomically commit observations + cursor. Storage enforces the closed
    schema and KEY UNIQUENESS: identical replay is a no-op; a conflicting
    same-key record is an integrity error (sol t25 #2/#11)."""
    state = Path(state_dir).expanduser()
    state.mkdir(parents=True, exist_ok=True)
    existing = {o["dedupe_key"]: o for o in load_observations(state)}
    to_append = []
    for obs in observations:
        fault = validate_observation(obs)
        if fault:
            raise ValueError(f"observation rejected: {fault}")
        if obs["detector_id"] not in KNOWN_DETECTORS:
            raise ValueError(
                f"unknown detector id {obs['detector_id']!r}")
        if obs["generation"] != entry.get("generation") or \
                obs["lineage_id"] != entry.get("lineage_id"):
            raise ValueError(
                "observation contradicts its source entry "
                "(generation/lineage mismatch)")
        if obs["source_kind"] not in SUPPORTED_SOURCES:
            raise ValueError(f"unknown source kind {obs['source_kind']!r}")
        if obs["source_kind"] != entry.get("source_kind"):
            raise ValueError(
                "observation stratum contradicts its source entry "
                "(sol t31 F31.6)")
        prev = existing.get(obs["dedupe_key"])
        if prev is not None:
            # `at` is observation time, not identity: replay after a cursor
            # reset re-derives the same key with a fresh timestamp and MUST
            # stay a no-op. Everything else must match exactly.
            ident = lambda o: {k: o[k] for k in o if k != "at"}
            if _canonical(ident(prev)) != _canonical(ident(obs)):
                raise ValueError(
                    f"integrity error: conflicting record for "
                    f"{obs['dedupe_key'][:12]}")
            continue  # identical replay -> no-op
        to_append.append(obs)
        existing[obs["dedupe_key"]] = obs
    if to_append:
        _atomic_append_jsonl(state / "observations.jsonl", to_append)
    cursors, _notes = _load_cursors(state)
    cursors[f"{entry.get('source_kind')}::{entry['path']}"] = {
        "cursor": cursor, "generation": entry.get("generation"),
        "lineage_id": entry.get("lineage_id"),
        "cmd_fps": stats.get("cmd_fps", {}),
        "pending_fps": list(stats.get("pending_fps", [])),
        "att_offs": dict(stats.get("att_offs", {})), "at": _now()}
    _atomic_write_json(state / "cursors.json", cursors)
    return {"appended": len(to_append)}


def _load_cursors(state):
    """Cursor validation (sol t25 #10): structure, types, bounds checked by
    the caller against file size; structurally invalid entries dropped with
    an explicit reset (the deduping replay protects progress)."""
    cpath = state / "cursors.json"
    if not cpath.exists():
        return {}, []
    try:
        raw = json.loads(cpath.read_text())
    except Exception:
        return {}, ["cursors.json unreadable — all cursors reset"]
    if not isinstance(raw, dict):
        return {}, ["cursors.json not a mapping — all cursors reset"]
    clean, notes = {}, []
    for k, v in raw.items():
        if not isinstance(v, dict):
            notes.append(f"cursor entry {k[:40]}: not a mapping — reset")
            continue
        if "::" not in k:
            notes.append(f"legacy cursor entry {k[:40]} — reset (F32.1)")
            continue
        c = v.get("cursor")
        if not isinstance(c, int) or isinstance(c, bool) or c < 0:
            notes.append(f"cursor entry {k[:40]}: invalid cursor — reset")
            continue
        if not isinstance(v.get("generation"), str):
            notes.append(f"cursor entry {k[:40]}: invalid generation — reset")
            continue
        fps = v.get("cmd_fps", {})
        pfps = v.get("pending_fps", [])
        aos = v.get("att_offs", {})
        if (not isinstance(fps, dict) or any(
                not isinstance(fp, str) or type(n) is not int or n < 0
                for fp, n in fps.items())
                or not isinstance(pfps, list) or any(
                not isinstance(p, dict) or not isinstance(p.get("fp"), str)
                or type(p.get("window")) is not int
                or not 1 <= p["window"] <= 3  # protocol bounds (F31.5)
                or type(p.get("byte_off")) is not int or p["byte_off"] < 0
                for p in pfps)
                or not isinstance(aos, dict) or any(
                not isinstance(a, str) or type(w) is not int or w < 0
                for a, w in aos.items())):
            notes.append(f"cursor entry {k[:40]}: invalid detector state — "
                         "reset")
            continue
        # rebuilt with ONLY known keys: legacy/stale shapes (recent_fps,
        # carry, string pending entries) can never ride along (t30/t31)
        lin = v.get("lineage_id")
        if not isinstance(lin, str) or not lin:
            # null/empty identity cannot detect inode replacement — the
            # whole entry resets (sol t33 F33.3)
            notes.append(f"cursor entry {k[:40]}: invalid lineage — reset")
            continue
        clean[k] = {"cursor": c, "generation": v["generation"],
                    "lineage_id": lin,
                    "cmd_fps": fps,
                    "pending_fps": [{"fp": p["fp"], "window": p["window"],
                                     "byte_off": p["byte_off"]}
                                    for p in pfps],
                    "att_offs": dict(aos)}
    return clean, notes


def _page_state(cstate, fresh):
    """Detector state for one source's pages. Any reset (out-of-bounds
    cursor, generation change) reads FRESH — stale counters or adjacency
    fingerprints from old content would manufacture loops or fake
    cross-page pairs (t26 N4; the v5 tick reset was clobbered by the
    unconditional cstate reload — caught in t30)."""
    if fresh:
        return {}, [], {}
    return (dict(cstate.get("cmd_fps", {})),
            list(cstate.get("pending_fps", [])),
            dict(cstate.get("att_offs", {})))


# ------------------------------------------------------------ patterns

def group_patterns(observations, min_incidents=2, scanned_lineages=None):
    """Patterns partition by (detector_id, detector_version, fingerprint) —
    detector versions never mix silently (sol t25 #7). Incidents = distinct
    LINEAGES; opportunities = ALL lineages scanned (symptomatic or clean —
    sol t26 #13)."""
    groups = {}
    lineages = set()
    for obs in observations:
        lineages.add(obs.get("lineage_id"))
        key = (obs["detector_id"], obs["detector_version"],
               obs.get("fingerprint"))
        groups.setdefault(key, []).append(obs)
    total = max(len(lineages), scanned_lineages or 0)
    patterns = []
    for (detector_id, version, fp), obs_list in sorted(groups.items()):
        incidents = len({o["lineage_id"] for o in obs_list})
        severity = max((o.get("severity", "low") for o in obs_list),
                       key=lambda s: {"high": 3, "medium": 2,
                                      "low": 1}.get(s, 1))
        patterns.append({
            "pattern_id": _sha(_canonical(
                [detector_id, version, fp]))[:16],
            "detector_id": detector_id, "detector_version": version,
            "fingerprint": fp, "severity": severity,
            "independent_incidents": incidents,
            "opportunities": total,
            "min_incidents": min_incidents,
            "eligible": incidents >= min_incidents,
            "strata": sorted({o.get("source_kind", "?")
                              for o in obs_list}),
            "evidence": [o["dedupe_key"][:12] for o in obs_list[:8]],
        })
    return patterns


# ------------------------------------------------------------ needs + rank

def read_needs(state_dir=None, max_age_days=7):
    """Validated needs snapshot; malformed, stale, or absent snapshots are
    EXPLICITLY unavailable (sol t25 #12) — never silently presumed."""
    path = (Path(state_dir).expanduser() if state_dir
            else DEFAULT_STATE.expanduser()) / "needs.json"
    if not path.exists():
        return {"available": False,
                "reason": "needs snapshot absent — evidence-based fallback"}
    try:
        needs = json.loads(path.read_text())
    except Exception as exc:
        return {"available": False, "reason": f"unreadable: {exc}"[:100]}
    if not isinstance(needs, dict) or type(needs.get("revision")) is not int:
        return {"available": False, "reason": "malformed needs snapshot"}
    fetched = needs.get("fetched_at")
    if not isinstance(fetched, str):
        return {"available": False,
                "reason": "needs snapshot has no fetch time"}
    try:
        age = datetime.now(timezone.utc) - datetime.fromisoformat(
            fetched.replace("Z", "+00:00"))
    except Exception:
        return {"available": False, "reason": "needs fetch time unparsable"}
    if age.total_seconds() > max_age_days * 86400:
        return {"available": False,
                "reason": f"needs snapshot stale "
                          f"({age.total_seconds() / 86400:.1f}d)"}
    items = needs.get("items")
    if not isinstance(items, list):
        return {"available": False, "reason": "needs items must be a list"}
    clean = []
    for n in items:
        if (not isinstance(n, dict) or not isinstance(n.get("need_id"), str)
                or not isinstance(n.get("urgency"), (int, float))
                or isinstance(n.get("urgency"), bool)
                or not 0 <= float(n["urgency"]) <= 10
                or not isinstance(n.get("relevant_fault_families"), list)
                or any(not isinstance(f, str) for f in
                       (n.get("relevant_fault_families") or []))):
            return {"available": False,
                    "reason": "needs items failed validation"}
        clean.append(n)
    return {"available": True,
            "needs": {"revision": needs["revision"], "items": clean}}


def rank_patterns(patterns, needs, exploration_share=0.2):
    """Needs-linked urgency multiplier + severity*incidents; deterministic
    tie-break; exploration lane flagged for the reserved tail (sol Q4)."""
    need_families = {}
    if needs.get("available"):
        for n in (needs["needs"].get("items") or []):
            for fam in (n.get("relevant_fault_families") or []):
                need_families[fam] = max(need_families.get(fam, 0),
                                         float(n.get("urgency", 1)))
    sev_w = {"high": 3.0, "medium": 2.0, "low": 1.0}
    scored = []
    for pat in patterns:
        if not pat["eligible"]:
            continue
        score = sev_w.get(pat.get("severity", "medium"), 2.0) \
            * (1 + pat["independent_incidents"])
        if pat["detector_id"] in need_families:
            score *= (1 + need_families[pat["detector_id"]])
        scored.append((score, pat))
    scored.sort(key=lambda sp: (-sp[0], sp[1]["pattern_id"]))
    ranked = [{"score": round(s, 3), **pat} for s, pat in scored]
    n_explore = max(1, int(len(ranked) * exploration_share)) if ranked else 0
    for i, pat in enumerate(ranked):
        pat["lane"] = ("exploration" if i >= len(ranked) - n_explore
                       else "demand")
    return ranked


# ------------------------------------------------------------ outbox + tick

def _outbox_ids(outbox):
    seen = set()
    if Path(outbox).exists():
        for line in Path(outbox).read_text().splitlines():
            try:
                seen.add(json.loads(line).get("pattern_id"))
            except Exception:
                continue
    return seen


def propose_claims(state_dir, ranked, cap=5, max_outbox=200):
    """Emit ClaimProposals with RESERVED exploration slots over the
    seen-filtered FULL candidate list (sol t25 #9): no permanent starvation.
    Outbox capacity enforced with explicit reporting."""
    state = Path(state_dir).expanduser()
    outbox = state / "outbox.jsonl"
    seen = _outbox_ids(outbox)
    existing_n = len(seen)
    if existing_n >= max_outbox:
        return [], {"capacity": "outbox full", "existing": existing_n}
    if not isinstance(cap, int) or isinstance(cap, bool) or cap < 0:
        return [], {"capacity": "invalid claim_cap", "cap": cap}
    if cap == 0:
        return [], {"capacity": "ok", "existing": existing_n,
                    "note": "claim_cap=0 — no emissions (F32.5)"}
    reserve = max(1, int(round(cap * 0.2))) if cap >= 2 else 1
    unseen = [p for p in ranked if p["pattern_id"] not in seen]
    demand = [p for p in unseen if p["lane"] == "demand"]
    explore = [p for p in unseen if p["lane"] == "exploration"]
    picked = demand[:max(0, cap - reserve)] + explore[:reserve]
    for p in demand[max(0, cap - reserve):] + explore[reserve:]:
        if len(picked) < cap:
            picked.append(p)
        else:
            break
    emitted = []
    for pat in picked:
        if existing_n + len(emitted) >= max_outbox:
            break  # per-emission capacity: overflow impossible (t26#5)
        proposal = {
            "kind": "claim_proposal", "pattern_id": pat["pattern_id"],
            "detector_id": pat["detector_id"],
            "detector_version": pat["detector_version"],
            "assertion": (f"Detector {pat['detector_id']} recurs across "
                          f"{pat['independent_incidents']} independent "
                          f"lineages of {pat['opportunities']} scanned "
                          f"(strata {pat['strata']}); a fixture reproducing "
                          "the normalized fingerprint trips the same "
                          "detector and the control fixture does not."),
            "disproof": "controls do not separate; incidents one lineage",
            "strata": pat["strata"], "lane": pat["lane"],
            "evidence": pat.get("evidence", []),
            "created_at": _now(), "status": "proposed"}
        _atomic_append_jsonl(outbox, [proposal])
        emitted.append(proposal)
    return emitted, {"capacity": "ok", "existing": existing_n}


def tick(sources, state_dir=None, max_bytes_per_source=262144,
         claim_cap=5, max_pages=10, max_outbox=200):
    """One BOUNDED pass (sol t25 #5): page budget stops the pass with
    explicit backlog reporting; generation validated around every commit;
    corrupted cursors reset explicitly; zero model calls."""
    if not isinstance(max_bytes_per_source, int) \
            or isinstance(max_bytes_per_source, bool) \
            or max_bytes_per_source <= 0:
        raise ValueError(
            "max_bytes_per_source must be a positive int, got "
            f"{max_bytes_per_source!r} (F33.6)")
    state = Path(state_dir).expanduser() if state_dir \
        else DEFAULT_STATE.expanduser()
    report = {"at": _now(), "census_sources": 0, "gaps": 0,
              "pages_read": 0, "observations": 0, "patterns": 0,
              "proposals": 0, "backlog_remaining": 0, "notes": []}
    manifest = census(sources, state_dir=state)
    report["census_sources"] = len(manifest["sources"])
    report["gaps"] = len(manifest["gaps"])
    cursors, cursor_notes = _load_cursors(state)
    report["notes"].extend(cursor_notes)
    try:
        raw_scanned = json.loads((state / "scanned.json").read_text())
        if isinstance(raw_scanned, list) and all(
                isinstance(x, str) and x for x in raw_scanned):
            scanned = set(raw_scanned)
        else:
            raise ValueError("invalid cohort shape")
    except FileNotFoundError:
        # missing state is recovered the same as malformed state — a lost
        # file must not silently erase completed scans (sol t33 F33.2)
        report["notes"].append(
            "scanned.json absent — cohort rebuilt from validated cursors")
        scanned = {c["lineage_id"] for c in cursors.values()
                   if isinstance(c.get("lineage_id"), str)
                   and c["lineage_id"]}
    except Exception:
        # corruption never INVENTS cohort members (sol t32 F32.4): rebuild
        # from validated cursor lineage ids and say so
        report["notes"].append(
            "scanned.json invalid — cohort rebuilt from validated cursors")
        scanned = {c["lineage_id"] for c in cursors.values()
                   if isinstance(c.get("lineage_id"), str)
                   and c["lineage_id"]}
    pages = 0
    backlog = 0
    for entry in manifest["sources"]:
        if entry.get("status") != "ok":
            continue
        if pages >= max_pages:
            backlog += 1
            continue
        p = Path(entry["path"])
        size = p.stat().st_size
        cstate = cursors.get(
            f"{entry['source_kind']}::{entry['path']}", {})
        cur = cstate.get("cursor", 0)
        fresh = False
        if not isinstance(cur, int) or cur < 0 or cur > size:
            if cur:
                report["notes"].append(
                    f"cursor out of bounds on {entry['path']}: reset "
                    "(replay dedupes)")
            cur, fresh = 0, True
        if cstate.get("generation") not in (None, entry["generation"]):
            report["notes"].append(
                f"source generation changed: {entry['path']} — reprocessing "
                "with FRESH detector state")
            cur, fresh = 0, True
        if cstate.get("lineage_id") not in (None, entry["lineage_id"]):
            report["notes"].append(
                f"lineage changed on {entry['path']} — reprocessing with "
                "FRESH detector state (sol t32 F32.1)")
            cur, fresh = 0, True
        gen = entry["generation"]
        lineage = entry["lineage_id"]
        cmd_fps, pending_fps, att_offs = _page_state(cstate, fresh)
        read_any = False
        deferred = False
        while pages < max_pages and cur < size:
            text, nxt, done, line_offs = read_page(p, cur,
                                                   max_bytes_per_source)
            if not text:
                break
            obs, pending_fps, att_offs = extract_page_with_state(
                entry["source_kind"], entry["path"], gen, lineage, text,
                cur, cmd_fps, pending_fps, att_offs=att_offs,
                line_offs=line_offs)
            pages += 1
            read_any = True
            report["pages_read"] += 1
            report["observations"] += len(obs)
            # generation validated BEFORE recording (sol t25 #3)
            if generation(p) != gen:
                report["notes"].append(
                    f"generation flapped mid-read: {entry['path']} — "
                    "page discarded, source deferred")
                deferred = True  # counted in backlog + gaps (F33.5)
                break
            commit_batch(state, entry, obs, nxt,
                         {"cmd_fps": cmd_fps, "pending_fps": pending_fps,
                          "att_offs": att_offs})
            cur = nxt
            if done:
                break
        if read_any:
            scanned.add(lineage)
        if deferred:
            backlog += 1
            report["gaps"] += 1
        elif cur < size and pages >= max_pages:
            backlog += 1
    report["backlog_remaining"] = backlog
    # opportunities use the MEASURED scanned cohort (sol t31 F31.2), never
    # the enrolled count; both are reported
    _atomic_write_json(state / "scanned.json", sorted(scanned))
    report["scanned_lineages"] = len(scanned)
    report["enrolled_lineages"] = manifest.get("lineages", 0)
    patterns = group_patterns(load_observations(state),
                              scanned_lineages=len(scanned))
    report["patterns"] = sum(1 for p in patterns if p["eligible"])
    needs = read_needs(state)
    if not needs.get("available"):
        report["notes"].append(needs["reason"])
    ranked = rank_patterns(patterns, needs)
    proposals, cap_note = propose_claims(state, ranked, cap=claim_cap,
                                         max_outbox=max_outbox)
    report["proposals"] = len(proposals)
    if cap_note.get("capacity") != "ok":
        report["notes"].append(f"outbox {cap_note['capacity']}")
    report["notes"].append(f"pass bounded: {pages} pages this pass")
    return report
