"""composer — the fleet lesson miner (owner directive 2026-10-26; sol architecture
2026-10-26; v2 per sol turn-25 conformance review, 13 findings closed).

A sibling deterministic service (maintenance.py pattern) subordinate to the
dream orchestrator: it mines enrolled session corpora for RECURRING fault
patterns, shapes them into falsifiable claims, and proposes work — it never
dispatches, never spawns workers, never publishes lessons by itself.

Identity model (sol t25 #1/#3): a source has a stable LINEAGE (enrolled
path) and a content GENERATION token (size + head64KiB + tail64KiB hashes).
Every page additionally carries .rid — the (dev, ino, size) of the ONE
handle that read it — and the tick commits only when that read identity
equals the census lineage, so a path swapped away for the read and
restored before validation can never launder another inode's bytes under
the census identity (CSR-05 EDITOR-01). The validation→commit sub-window
stays unlocked by design (no pathname lock exists); it is caught by the
next pass's checks, never claimed closed. Incidents are counted per
LINEAGE — an appended line is the same lineage,
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

Retrieval-doors doctrine (owner 2026-10-26, after the self-improvement
falsification report C2/C6): every proposal this kernel emits MUST name its
DOOR — steer (law; routed as a yggsteer SSOT proposal, never a direct
edit), ygg-memory (durable fact; NOW.md-first on recall), msgGraph board
(conversation), ynpm skill note (routing fact), or the YGG-FHS placement
oracle (where artifacts go). The kernel does NOT restate that law — the
stamped steer files are the constitution and this file defers to them;
proposals CITE the door + rule so consumers retrieve fast instead of
re-deriving.

Jev recon doctrine (owner 2026-10-26): mining sweep and triage passes MAY
use Jev as System-1 (bounded classification/routing/tagging of candidate
faults, per the steer Jev budget law) to get holistic vision fast; System-1
output is candidate-shaping ONLY — never generation, never authority, never
a published lesson. Due diligence on any claim this kernel elevates is
System-2 (the brain pair) before it leaves the outbox.

Stale-instruction GC (owner 2026-10-26, C6 falsified the claim that the
fleet GCs stale instructions): proposing GC of stale instructions is a
FIRST-CLASS maintenance proposal class — dated facts past the 30-day
staleness law, dead paths (verify against origin/main first), superseded
doors, unaged FAIL backlogs. Propose with evidence; never execute removal
from this kernel.

Enrollment gate (C2): a composer with no enrolled sources has no causal
path to fleet improvement — conformance-green is not production. Until an
owner-approved enrollment manifest exists (default-deny, per-source, per
the privacy law), this kernel's outbox stays empty BY DESIGN and nothing
may claim composer-driven improvement.

Append-stream progression (F1 repair, 2026-10-29): a source that GREW is
no longer reset. The cursor carries an integrity ANCHOR — full head/tail
64KiB hashes of the consumed prefix plus per-page chunk hashes — and a
growth is accepted as append-only when the head window verifies equal.
Windows covering the whole consumed prefix (prefix <= 128KiB) verify the
FULL prefix; larger prefixes are SAMPLED: the interior is re-verified
rotationally, a bounded number of chunk bytes per tick, with the
rotation position advancing by the chunks ACTUALLY verified (CSR-01) so
every anchored chunk is revisited within len(chunks) passes (ceil(total
chunk bytes / budget) for uniform chunks). Honest statement of the
integrity tradeoff: the generation token is a SAMPLED token
(size + two 64KiB windows), not a full-content hash — a same-size
interior rewrite is invisible to it, so the anchor's rotation is the
detector, and detection is EVENTUAL (within the rotation period), not
instantaneous; content that mutates and mutates back between checks is
undetectable at any sampling depth; UNANCHORED bytes (beyond the chunk
cap, or a crash gap not yet reconciled) have NO rotation coverage until
anchored. The tick notes name this explicitly:
"prefix integrity: full" (windows proved the whole consumed prefix) vs
"prefix integrity: sampled" (windows proved the head; rotation owns the
interior). A sampled pass is never presented as full integrity.

Fair cohort scheduling (CSR-02 repair, 2026-10-29): the enrolled cohort
is a finite list per census; each pass serves it from a persisted
round-robin offset advanced by the sources actually served, so a source
that grows faster than the page budget drains it cannot permanently
starve a later enrolled prefix — every enrolled source gets its page
chance within len(cohort) consecutive passes when the budget is at least
one page. The scheduler is order-only state; losing it reorders, never
loses evidence.

Crash-gap reconciliation (CSR-03 repair, 2026-10-29; frontier checkpoint
per the editor round-1 finding CSR-03-EDITOR-01): the cursor commits per
page, the anchor once per pass; a crash between the two can leave the
cursor AHEAD of the anchored frontier. Recovery detects the mismatch and
re-reads the gap from the anchored frontier carrying the FRONTIER
DETECTOR CHECKPOINT — the valid detector state AS OF the frontier, saved
inside the anchor entry in the SAME atomic write, so it is always
consistent with the frontier and never contaminated by already-read gap
bytes (prefix context survives: retained evidence pairs and hunt windows
still close recurrence after recovery, exactly as in an uncrashed run).
A legacy anchor without a checkpoint degrades EXPLICITLY to fresh state
(typed note), never a silent reuse of possibly-contaminated state. Prior
evidence survives via the v2 dedupe, chunk hashes come from re-read RAW
bytes, and the reconciled cursor persists even when the pass has no
budget left, so the reconcile is one-time and crash-replay idempotent.
Bytes that changed while unobserved stay distinct evidence at their own
offsets.

Observation identity v2 (F2 repair, 2026-10-29): the dedupe identity is
(source_kind, lineage, span, detector, detector_version, fingerprint) —
generation EXCLUDED. The same logical occurrence (same bytes at the same
offsets, same normalized fault) dedupes across appends and re-scans;
appends do not shift byte offsets, so a re-scanned prefix re-derives the
same identity and stays one row. A genuinely distinct occurrence (same
fault text at a different offset) or changed content (different
fingerprint) remains distinct evidence. The row's `generation` field is
PROVENANCE (the generation the observation was made in), not identity;
replay under a newer generation is a no-op that preserves the original
row immutably. Migration of v1 stores (identity embedded generation) is
explicit: rekeyed under v2, generation-era duplicates coalesced to the
earliest row with the counts reported in the tick notes, foreign-version
rows preserved verbatim — no silent discard, no faked identity.

Durability (acceptance 4): every tick body runs under a per-state-dir
flock — concurrent ticks serialize, a crashed holder releases by process
death, and observation/cursor writes stay ordered observations-first so
a crash can only leave the cursor behind (re-read + dedupe), never
orphan evidence; files are fsynced before rename and the directory after
it.
"""
import fcntl
import hashlib
import json
import os
import re
from contextlib import contextmanager
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

# ---- append-stream integrity anchors (F1/F6 repair) ----
_ANCHOR_WINDOW = 65536       # head/tail verification window (bytes)
_ANCHOR_FULL_LIMIT = 131072  # windows cover the whole prefix up to here
_MAX_ANCHOR_CHUNKS = 1024    # per-source chunk cap (~266 MiB at 256 KiB pages)
_ANCHOR_FILE = "anchors.json"

# ---- observation identity version (F2 repair) ----
_OBS_IDENTITY_VERSION = 2
_STORE_META = "observations.meta.json"

# ---- fair cohort scheduling (CSR-02 repair) ----
_SCHEDULER_FILE = "scheduler.json"


def _load_scheduler(state):
    """Persisted round-robin serving offset over the enrolled cohort
    (CSR-02). Advisory ONLY: a lost or corrupt file reorders which source
    is served first — it can never lose evidence, cursor or anchor state —
    so it resets to 0 without a note, unlike substantive state."""
    try:
        rr = json.loads((Path(state) / _SCHEDULER_FILE).read_text()).get("rr")
        if type(rr) is int and rr >= 0:
            return rr
    except Exception:
        pass
    return 0


def _save_scheduler(state, rr):
    _atomic_write_json(Path(state) / _SCHEDULER_FILE, {"rr": int(rr)})


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


def source_identity(path):
    """(generation_token, size, lineage_id) from ONE opened handle —
    lineage (st_dev:st_ino), size and both content windows are read off
    the same fd, so a stat/read race can never pair one source's lineage
    with another's generation or byte count (acceptance 3; sol t26 #4
    extended to identity). Symlinks AND hard links collapse to one
    physical source."""
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
    return token, st.st_size, _sha(f"{st.st_dev}:{st.st_ino}")[:16]


def lineage_id(path):
    """Lineage identity from the INODE (st_dev:st_ino) of the opened
    source — symlinks AND hard links collapse to one physical source
    (sol t28 #1). Reads through source_identity for one-fd consistency."""
    return source_identity(path)[2]


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


def observation_identity(source_kind, lineage, span, detector_id,
                         detector_version, fingerprint):
    """Stable LOGICAL observation identity, v2 (F2 repair): generation
    EXCLUDED. Appends never shift byte offsets, so the same occurrence
    re-derived after an append or an explicit re-scan re-computes the
    same identity and stays ONE row; a distinct occurrence (different
    offset) or changed content (different fingerprint) stays distinct
    evidence. The generation remains on the row as provenance."""
    return "v2-" + _sha(_canonical(
        [source_kind, lineage, span, detector_id, detector_version,
         fingerprint]))


def make_observation(detector_id, severity, source_kind, gen, span, fp,
                     lineage, status="unverified-symptom"):
    body = {"detector_id": detector_id, "detector_version": DETECTOR_VERSION,
            "severity": severity, "status": status,
            "source_kind": source_kind, "generation": gen,
            "lineage_id": lineage, "span": span, "fingerprint": fp,
            "at": _now()}
    body["dedupe_key"] = observation_identity(
        source_kind, lineage, span, detector_id, DETECTOR_VERSION, fp)
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
    key = observation_identity(
        obs["source_kind"], obs["lineage_id"], obs["span"],
        obs["detector_id"], obs["detector_version"], obs["fingerprint"])
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
                entry["status"] = "ok"
                entry["generation"], entry["bytes"], entry["lineage_id"] = \
                    source_identity(p)
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

class _Page(tuple):
    """The historical read_page 4-tuple (text, next_cursor, done,
    line_offs) carrying the raw emitted bytes for anchor hashing on
    `.raw`, and `.rid` — the (st_dev, st_ino, st_size) fstat'ed on the
    ONE opened handle that supplied the bytes (CSR-05 EDITOR-01). A path
    swapped away before the open, or restored after the read, cannot
    change what the handle was bound to, so .rid is the only identity the
    page can truthfully claim. Oversized-drop pages return raw=None: the
    skipped bytes are deliberately unanchored (typed in-content marker
    instead)."""

    def __new__(cls, text, nxt, done, line_offs, raw, rid):
        self = super().__new__(cls, (text, nxt, done, line_offs))
        self.raw = raw
        self.rid = rid
        return self


def read_page(path, cursor=0, max_bytes=262144):
    """Bounded page of COMPLETE records. Bytes are buffered RAW, framed by
    the BYTE splitlines grammar (LF, CRLF and bare CR all terminate a
    record — sol t33 F33.4), and decoded once per emitted block; cursor
    arithmetic is byte-exact. The cursor only ever advances over EMITTED
    bytes (progress guaranteed, sol t28 V3-N1). The oversized bound applies
    to the individual leading unfinished record. Returns (text,
    next_cursor, done, line_offs) — RAW-BYTE start offsets of the emitted
    complete records (sol t31 F31.4). The page carries .rid =
    (st_dev, st_ino, st_size) fstat'ed on this same reading handle — never
    a separate path stat — so the caller can bind the bytes to the identity
    of the inode that was actually read (CSR-05 EDITOR-01): size and both
    cursor bounds come off that handle too, so one race-free open pairs
    the bytes, the size and the identity."""
    if not isinstance(max_bytes, int) or isinstance(max_bytes, bool) \
            or max_bytes <= 0:
        raise ValueError(
            f"max_bytes must be a positive int, got {max_bytes!r} (F33.6)")
    p = Path(path)
    raw = bytearray()
    pos = cursor
    eof = False
    with p.open("rb") as f:
        # identity of the handle that supplies every byte below: fstat on
        # the SAME fd the reads flow through (CSR-05 EDITOR-01) — an
        # fd-bound rename/replace cannot change it, so a path that is
        # swapped away for this read and restored before validation is
        # exposed by comparing .rid against the census identity.
        st = os.fstat(f.fileno())
        size = st.st_size
        rid = (st.st_dev, st.st_ino, st.st_size)
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
                return _Page(marker, cursor + len(first), False, [cursor],
                             None, rid)
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
    return _Page(text, next_cursor, next_cursor >= size, starts, emitted,
                 rid)


# ------------------------------------------------------------ store

def load_observations_report(state_dir=None):
    """(rows, corrupt_lines, foreign_version_rows). Corrupt store lines
    are counted and reported by the tick — never silently skipped
    (acceptance 3); foreign-detector-version rows are kept verbatim."""
    path = (Path(state_dir).expanduser() if state_dir
            else DEFAULT_STATE.expanduser()) / "observations.jsonl"
    if not path.exists():
        return [], 0, 0
    out, corrupt, foreign = [], 0, 0
    for line in path.read_text().splitlines():
        try:
            row = json.loads(line)
        except Exception:
            corrupt += 1
            continue
        if not isinstance(row, dict):
            corrupt += 1
            continue
        if row.get("detector_version") != DETECTOR_VERSION:
            foreign += 1
        out.append(row)
    return out, corrupt, foreign


def load_observations(state_dir=None):
    return load_observations_report(state_dir)[0]


def _fsync_dir(path):
    dfd = os.open(str(path), os.O_RDONLY)
    try:
        os.fsync(dfd)
    finally:
        os.close(dfd)


def _atomic_write_jsonl(path, rows):
    """Full atomic rewrite preserving row order (migration path)."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + f".tmp{os.getpid()}")
    with tmp.open("w") as f:
        for row in rows:
            f.write(_canonical(row) + "\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)
    _fsync_dir(path.parent)


def _atomic_append_jsonl(path, rows):
    """Atomic read-merge-write — the EXISTING rows are read back and
    rewritten with the new ones under the tick lock; a fresh-file
    append-temp that would lose rows is never used (acceptance 4)."""
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
    _fsync_dir(path.parent)


def _atomic_write_json(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + f".tmp{os.getpid()}")
    with tmp.open("w") as f:
        f.write(_canonical(obj))
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)
    _fsync_dir(path.parent)


@contextmanager
def _state_lock(state):
    """Exclusive inter-process tick lock (flock on state/lock).
    Concurrent ticks serialize; a crashed holder releases by process
    death. Cursor and evidence can therefore never interleave two
    writers (acceptance 4)."""
    state.mkdir(parents=True, exist_ok=True)
    fd = os.open(str(state / "lock"), os.O_RDWR | os.O_CREAT, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def migrate_observations(state_dir):
    """One-time EXPLICIT identity migration (acceptance 2): v1 stores
    embedded the generation in the dedupe identity, so the same logical
    fault re-minted a row per generation (audit F2). Under v2 the rows
    are rekeyed by observation_identity; generation-era duplicates
    (same logical occurrence, different generations) coalesce to the
    EARLIEST row — provenance lives on the kept row, the counts land in
    the tick notes. Foreign-detector-version rows are preserved
    verbatim. Never a silent discard; never a faked identity."""
    state = Path(state_dir).expanduser()
    meta_path = state / _STORE_META
    if meta_path.exists():
        try:
            if json.loads(meta_path.read_text()).get(
                    "identity_version") == _OBS_IDENTITY_VERSION:
                return []
        except Exception:
            pass  # unreadable meta: re-migrate deterministically below
    opath = state / "observations.jsonl"
    rows, corrupt, foreign = load_observations_report(state)
    rekeyed = coalesced = 0
    seen, kept = set(), []
    for o in rows:
        if o.get("detector_version") != DETECTOR_VERSION:
            kept.append(o)  # foreign rows verbatim — identity preserved
            continue
        k = observation_identity(o.get("source_kind"), o.get("lineage_id"),
                                 o.get("span"), o.get("detector_id"),
                                 o.get("detector_version"),
                                 o.get("fingerprint"))
        if k in seen:
            coalesced += 1
            continue
        seen.add(k)
        kept.append(dict(o, dedupe_key=k))
        rekeyed += 1
    if opath.exists():
        _atomic_write_jsonl(opath, kept)
    _atomic_write_json(meta_path, {
        "identity_version": _OBS_IDENTITY_VERSION,
        "dedupe_identity": "v2 (source_kind, lineage, span, detector, "
                           "version, fingerprint) — generation excluded",
        "migrated_at": _now()})
    bits = []
    if rekeyed:
        bits.append(f"rekeyed {rekeyed} rows")
    if coalesced:
        bits.append(f"coalesced {coalesced} generation-era duplicates "
                    "(earliest kept — provenance lives on the kept row)")
    if corrupt:
        bits.append(f"{corrupt} corrupt store lines skipped (pre-existing, "
                    "counted, not silent)")
    if foreign:
        bits.append(f"{foreign} foreign-version rows preserved verbatim")
    return [f"observation store migrated to identity v2: " + "; ".join(bits)] \
        if bits else []


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
            # `at` and `generation` are observation metadata, not identity:
            # replay after an append or an explicit re-scan re-derives the
            # same key with a fresh timestamp and possibly a newer
            # generation, and MUST stay a no-op that preserves the original
            # row immutably (F2). Everything else must match exactly.
            ident = lambda o: {k: o[k] for k in o
                               if k not in ("at", "generation")}
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


def _valid_det_state(fps, pfps, aos):
    """Shared detector-state validation (cursor entries and anchor
    checkpoints): fingerprints are nonempty strings with nonnegative int
    counts; hunts carry the protocol-bounded window; retained offsets are
    nonnegative ints (F31.5)."""
    return (isinstance(fps, dict) and all(
        isinstance(fp, str) and type(n) is int and n >= 0
        for fp, n in fps.items())
        and isinstance(pfps, list) and all(
        isinstance(p, dict) and isinstance(p.get("fp"), str)
        and type(p.get("window")) is int
        and 1 <= p["window"] <= 3  # protocol bounds (F31.5)
        and type(p.get("byte_off")) is int and p["byte_off"] >= 0
        for p in pfps)
        and isinstance(aos, dict) and all(
        isinstance(a, str) and type(w) is int and w >= 0
        for a, w in aos.items()))


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
        if not _valid_det_state(fps, pfps, aos):
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


# ------------------------------------------------------------ anchors

def _load_anchors(state):
    """Integrity anchors per source key, validated like cursors. Legacy
    stores have no anchors.json — absence is a typed reset-once condition
    in the tick (explicit migration, never a silent assumption)."""
    apath = state / _ANCHOR_FILE
    if not apath.exists():
        return {}, []
    try:
        raw = json.loads(apath.read_text())
    except Exception:
        return {}, [f"{_ANCHOR_FILE} unreadable — all anchors reset "
                    "(one-time explicit re-scan per affected source)"]
    if not isinstance(raw, dict):
        return {}, [f"{_ANCHOR_FILE} not a mapping — all anchors reset"]

    def _bad_entry(k):
        return f"anchor entry {k[:40]}: invalid — dropped (explicit re-scan)"

    clean, notes = {}, []
    for k, v in raw.items():
        if not isinstance(v, dict) or type(v.get("size")) is not int \
                or v["size"] < 0 or not isinstance(v.get("head_sha"), str) \
                or not isinstance(v.get("tail_sha"), str):
            notes.append(_bad_entry(k))
            continue
        chunks = v.get("chunks")
        if not isinstance(chunks, list) or any(
                not isinstance(c, dict)
                or type(c.get("start")) is not int or c["start"] < 0
                or type(c.get("len")) is not int or c["len"] < 0
                or not isinstance(c.get("sha"), str) or len(c["sha"]) != 64
                for c in chunks) or len(chunks) > _MAX_ANCHOR_CHUNKS:
            notes.append(_bad_entry(k))
            continue
        pos = v.get("verify_pos", 0)
        if type(pos) is not int or pos < 0:
            notes.append(_bad_entry(k))
            continue
        # frontier detector checkpoint (CSR-03): the valid detector state
        # AS OF the anchored frontier, saved in the SAME atomic write as
        # the frontier so the two can never diverge. None = legacy anchor
        # (typed absence — the gap reconcile degrades explicitly).
        det = v.get("det")
        if det is not None:
            if not isinstance(det, dict) or not _valid_det_state(
                    det.get("cmd_fps", {}), det.get("pending_fps", []),
                    det.get("att_offs", {})):
                notes.append(_bad_entry(k))
                continue
        clean[k] = {"size": v["size"], "head_sha": v["head_sha"],
                    "tail_sha": v["tail_sha"], "chunks": [
                        {"start": c["start"], "len": c["len"],
                         "sha": c["sha"]} for c in chunks],
                    "verify_pos": pos,
                    "det": None if det is None else {
                        "cmd_fps": dict(det.get("cmd_fps", {})),
                        "pending_fps": [
                            {"fp": p["fp"], "window": p["window"],
                             "byte_off": p["byte_off"]}
                            for p in det.get("pending_fps", [])],
                        "att_offs": dict(det.get("att_offs", {}))}}
    return clean, notes


def _save_anchors(state, anchors):
    _atomic_write_json(Path(state) / _ANCHOR_FILE, anchors)


def _refresh_anchor_windows(path, consumed):
    """Full head/tail hashes over the CONSUMED prefix (exactly [0,
    consumed) — never past the cursor), read from the live file — two
    bounded reads. For consumed <= 64KiB the head window IS the whole
    prefix (tail == head)."""
    head = tail = b""
    if consumed > 0:
        with Path(path).open("rb") as f:
            head = f.read(min(_ANCHOR_WINDOW, consumed))
            if consumed > _ANCHOR_WINDOW:
                f.seek(consumed - _ANCHOR_WINDOW)
                tail = f.read(_ANCHOR_WINDOW)
            else:
                tail = head
    return _sha(head), _sha(tail)


def _growth_verdict(path, anchor):
    """Immediate window verdict for a grown source: ("append", coverage)
    | ("mutated", None). coverage is "full" when the two windows prove the
    WHOLE consumed prefix unchanged (prefix <= 128KiB) and "sampled" when
    only the head is proved and the anchor rotation owns the interior."""
    old = anchor.get("size", 0)
    with Path(path).open("rb") as f:
        head = f.read(min(_ANCHOR_WINDOW, old)) if old > 0 else b""
    if _sha(head) != anchor.get("head_sha"):
        return "mutated", None
    if old <= _ANCHOR_WINDOW:
        return "append", "full"
    if old <= _ANCHOR_FULL_LIMIT:
        with Path(path).open("rb") as f:
            f.seek(old - _ANCHOR_WINDOW)
            old_tail = f.read(_ANCHOR_WINDOW)
        if _sha(old_tail) == anchor.get("tail_sha"):
            return "append", "full"
        return "mutated", None
    return "append", "sampled"


def _verify_prefix_chunks(path, anchor, budget_bytes):
    """Rotationally verify bounded anchor chunks of the consumed prefix —
    the detector for same-size interior rewrites the sampled generation
    token cannot see (F6). One bounded sweep per call, cyclic from
    verify_pos; cost never counts against the page budget. The position
    advances by the number of chunks ACTUALLY verified (CSR-01): at least
    one chunk per call (the first chunk is verified even past the budget)
    and at most budget_bytes worth, so every anchored chunk is revisited
    within len(chunks) calls and within ceil(total chunk bytes / budget)
    calls for uniform chunks — a persistent fault anywhere in the anchored
    prefix is seen in bounded time, never parked behind a position that
    never moves. Returns (ok, mismatch_note, verified_bytes, new_pos)."""
    chunks = anchor.get("chunks") or []
    if not chunks or budget_bytes <= 0:
        return True, None, 0, anchor.get("verify_pos", 0)
    pos = anchor.get("verify_pos", 0) % len(chunks)
    verified = 0
    checked = 0
    with Path(path).open("rb") as f:
        for k in range(len(chunks)):
            ch = chunks[(pos + k) % len(chunks)]
            if k > 0 and verified + ch["len"] > budget_bytes:
                break
            f.seek(ch["start"])
            data = f.read(ch["len"])
            if len(data) != ch["len"] or _sha(data) != ch["sha"]:
                return False, (
                    f"consumed-prefix mutation detected in anchor chunk "
                    f"[bytes@{ch['start']}+{ch['len']}] on {path} — content "
                    "changed under recorded evidence; re-scanning with "
                    "FRESH detector state"), verified, (pos + k) % len(chunks)
            verified += ch["len"]
            checked += 1
    return True, None, verified, (pos + checked) % len(chunks)


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
    explicit backlog reporting; growing sources PROGRESS from the stored
    cursor when the integrity anchor verifies the consumed prefix (F1
    repair), and same-size interior rewrites are caught by the anchor
    rotation regardless of the token (F6 repair); the enrolled cohort is
    served round-robin from a persisted offset so a hoarding source
    cannot starve later prefixes (CSR-02); a cursor ahead of its anchor
    is reconciled by re-reading the crash gap before anything is trusted
    (CSR-03); generation validated around every commit; corrupted cursors
    reset explicitly; zero model calls. The tick body runs under the
    state lock — concurrent ticks serialize (acceptance 4)."""
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
              "proposals": 0, "backlog_remaining": 0,
              "coverage_complete": 0, "new_tail_bytes": 0,
              "prefix_verified_bytes": 0, "notes": []}
    with _state_lock(state):
        report["notes"].extend(migrate_observations(state))
        manifest = census(sources, state_dir=state)
        report["census_sources"] = len(manifest["sources"])
        report["gaps"] = len(manifest["gaps"])
        cursors, cursor_notes = _load_cursors(state)
        report["notes"].extend(cursor_notes)
        anchors, anchor_notes = _load_anchors(state)
        report["notes"].extend(anchor_notes)
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
        # Fair bounded scheduling (CSR-02): the cohort is the FINITE list
        # of enrolled sources this census returned. Each pass serves it
        # starting at the persisted round-robin offset and advances that
        # offset by the number of sources actually served, so a hoarding
        # source — one that grows faster than the page budget drains it —
        # cannot permanently starve a later enrolled prefix: with a
        # per-pass budget of at least one page, every enrolled source is
        # given its page chance at least once within len(cohort)
        # consecutive passes. Closed-snapshot vs new-tail coverage
        # accounting is unchanged; the scheduler is order-only state.
        cohort = [e for e in manifest["sources"]
                  if e.get("status") == "ok"]
        rr = _load_scheduler(state) % max(1, len(cohort))
        order = cohort[rr:] + cohort[:rr]
        served = 0
        for entry in order:
            if entry.get("status") != "ok":
                continue
            if pages >= max_pages:
                backlog += 1
                continue
            served += 1
            p = Path(entry["path"])
            size = p.stat().st_size
            key = f"{entry['source_kind']}::{entry['path']}"
            cstate = cursors.get(key, {})
            anchor = anchors.get(key)
            cur = cstate.get("cursor", 0)
            gen = entry["generation"]
            lineage = entry["lineage_id"]
            fresh = False
            anchors_dirty = False
            reset_occurred = False
            gap_reconciled = False
            gap_checkpoint = None

            def _reset(note):
                nonlocal cur, fresh, anchor, anchors_dirty, reset_occurred
                nonlocal gap_reconciled, gap_checkpoint
                report["notes"].append(note)
                cur, fresh = 0, True
                reset_occurred = True
                # a reset invalidates EVERYTHING anchored: the captured
                # gap checkpoint belonged to the discarded prefix and
                # must never combine with replacement bytes (CSR-03
                # EDITOR-02 — a stale checkpoint surviving the reset
                # manufactured recurrence observations from evidence
                # that was just proven invalid)
                gap_reconciled = False
                gap_checkpoint = None
                if key in anchors:
                    anchors.pop(key)
                    anchors_dirty = True
                anchor = None

            # ---- identity + progression decision (F1/F6 repair) ----
            if not isinstance(cur, int) or isinstance(cur, bool) \
                    or cur < 0 or cur > size:
                if cur > size and anchor is not None \
                        and anchor["size"] > size:
                    _reset(f"source truncated: {entry['path']} "
                           f"({anchor['size']} -> {size} bytes) — explicit "
                           "re-scan with FRESH detector state")
                elif cur:
                    _reset(f"cursor out of bounds on {entry['path']}: reset "
                           "(replay dedupes)")
                else:
                    cur, fresh = 0, True
            elif cstate.get("lineage_id") not in (None, lineage):
                _reset(f"lineage changed on {entry['path']} — reprocessing "
                       "with FRESH detector state (sol t32 F32.1)")
            elif cur == 0 or cstate.get("generation") is None:
                fresh = True  # first read; nothing consumed to verify
            elif anchor is None:
                _reset(f"integrity anchor absent for consumed prefix on "
                       f"{entry['path']} — explicit one-time re-scan to "
                       "build the anchor (replay dedupes)")
            elif size < anchor["size"]:
                _reset(f"source truncated: {entry['path']} "
                       f"({anchor['size']} -> {size} bytes) — explicit "
                       "re-scan with FRESH detector state")
            else:
                # Cursor/anchor reconciliation BEFORE anything is trusted
                # (CSR-03): the cursor is committed per page while the
                # anchor is saved once at pass end, so a crash between the
                # two can leave the cursor AHEAD of the anchored frontier.
                # Those gap bytes were consumed but never anchored —
                # trusting the cursor alone would skip them forever and
                # the anchor's rotation could never verify them. The gap
                # is re-read from the anchored frontier with FRESH
                # detector state (the persisted detector state may already
                # include gap content); prior evidence is preserved — the
                # identity-v2 dedupe makes re-derived identical rows a
                # no-op, and any bytes that changed while unobserved stay
                # distinct evidence at their own offsets. Chunk hashes are
                # taken from the re-read RAW bytes, so the anchor never
                # blesses bytes nobody read.
                if cur > anchor["size"]:
                    gap_lo, gap_hi = anchor["size"], cur
                    cur = anchor["size"]
                    gap_reconciled = True
                    gap_checkpoint = anchor.get("det")
                    report["notes"].append(
                        f"cursor ahead of anchored prefix on "
                        f"{entry['path']} (crash gap "
                        f"[{gap_lo},{gap_hi})) — reconciling: gap re-read "
                        "from the anchored frontier with detector state "
                        "checkpointed AT the frontier (the frontier and "
                        "its checkpoint save in one atomic write, so it "
                        "is never contaminated by gap bytes); dedupe "
                        "keeps prior evidence")
                    if gap_checkpoint is None:
                        report["notes"].append(
                            f"frontier detector checkpoint absent on "
                            f"{entry['path']} (legacy anchor) — prefix "
                            "adjacency state unrecoverable, gap re-read "
                            "uses FRESH detector state (typed degraded "
                            "reconcile, never a silent reuse)")
                elif cur < anchor["size"]:
                    # safe direction (evidence commits before the cursor):
                    # trim chunks past the cursor so the re-read rebuilds
                    # contiguous non-overlapping coverage — never a silent
                    # overlap
                    trimmed = [c for c in anchor["chunks"]
                               if c["start"] < cur]
                    if len(trimmed) != len(anchor["chunks"]):
                        anchor["chunks"] = trimmed
                        anchors_dirty = True
                        report["notes"].append(
                            f"cursor behind anchored prefix on "
                            f"{entry['path']} — anchor chunks past the "
                            "cursor trimmed; re-read rebuilds coverage")
                # rotation re-verification runs on EVERY pass over an
                # anchored prefix — it is the detector for same-size
                # interior rewrites the sampled generation token cannot
                # see (F6); its cost never counts against the page budget
                ok, mismatch, vbytes, vpos = _verify_prefix_chunks(
                    p, anchor, max_bytes_per_source * 4)
                report["prefix_verified_bytes"] += vbytes
                if not ok:
                    _reset(mismatch)
                else:
                    anchor["verify_pos"] = vpos
                    anchors_dirty = True
                    if cstate.get("generation") not in (None, gen):
                        if size == anchor["size"]:
                            _reset(f"content mutated at unchanged size: "
                                   f"{entry['path']} — re-scanning with "
                                   "FRESH detector state")
                        else:
                            verdict, coverage = _growth_verdict(p, anchor)
                            if verdict == "mutated":
                                _reset(f"prefix content mutated across "
                                       f"growth on {entry['path']} — "
                                       "re-scanning with FRESH detector "
                                       "state")
                            else:
                                # the consumed prefix is retained; the
                                # delta is UNREAD tail (appended or
                                # otherwise changed — read fresh either
                                # way), never a re-observation of old
                                # bytes
                                report["notes"].append(
                                    f"prefix progression on {entry['path']}: "
                                    f"consumed prefix retained (integrity: "
                                    f"{coverage}); {size - anchor['size']} "
                                    "tail bytes to read")

            if gap_reconciled and gap_checkpoint is not None:
                # the frontier checkpoint IS the valid detector state at
                # the re-read start: prefix context (retained evidence
                # pairs, hunt windows, counts) survives the crash, and
                # the gap pages replay their transitions exactly once
                # from the same base the uncrashed run used (CSR-03
                # EDITOR-01 repair — fresh-state wiped valid prefix
                # context and permanently missed recurrence evidence)
                cmd_fps = dict(gap_checkpoint.get("cmd_fps", {}))
                pending_fps = list(gap_checkpoint.get("pending_fps", []))
                att_offs = dict(gap_checkpoint.get("att_offs", {}))
            else:
                cmd_fps, pending_fps, att_offs = _page_state(
                    cstate, fresh or gap_reconciled)
            local_chunks = [] if fresh else \
                [dict(c) for c in (anchor or {}).get("chunks", [])]
            prev_pos = 0 if fresh else (anchor or {}).get("verify_pos", 0)
            read_any = False
            deferred = False
            cap_noted = False
            src_bytes = 0
            utf8_hits = 0
            while pages < max_pages and cur < size:
                page = read_page(p, cur, max_bytes_per_source)
                text, nxt, done, line_offs = page
                if not text:
                    break
                utf8_hits += text.count("\ufffd")
                obs, pending_fps, att_offs = extract_page_with_state(
                    entry["source_kind"], entry["path"], gen, lineage, text,
                    cur, cmd_fps, pending_fps, att_offs=att_offs,
                    line_offs=line_offs)
                pages += 1
                read_any = True
                report["pages_read"] += 1
                report["observations"] += len(obs)
                src_bytes += nxt - cur  # bytes newly consumed this pass
                # (previously unread gap + appended tail)
                # identity validated BEFORE recording (sol t25 #3; CSR-05),
                # in two layers (CSR-05 EDITOR-01):
                # 1. THE PAGE IS BOUND TO THE HANDLE THAT ACTUALLY READ
                # IT: page.rid is fstat'ed on read_page's own opened
                # handle — the handle that supplied these bytes — and its
                # lineage must equal the census lineage the page was mined
                # under. This closes the restore geometry the old
                # path-only check was blind to: rename A aside, place a
                # byte-identical B for the read, restore A before
                # validation — the second handle then matched the census
                # and committed B's bytes as A, and the later rebind
                # re-mined them as a second "independent" incident
                # (eligible=true). Missing .rid (a page not built by this
                # read_page) cannot be bound and defers — fail closed.
                # 2. The post-read PATHNAME check is RETAINED for the
                # read→validation window (replacement after the read):
                # generation, size AND lineage re-read off ONE handle must
                # equal the census identity. Explicit residual limit: the
                # window between this validation and commit_batch is NOT
                # locked (no pathname lock exists on Linux for arbitrary
                # paths); a swap landing inside it is caught by the next
                # pass's checks, not by this one — stated, never claimed
                # impossible.
                i_gen, i_size, i_lineage = source_identity(p)
                rid = getattr(page, "rid", None)
                r_lineage = (_sha(f"{rid[0]}:{rid[1]}")[:16]
                             if rid else None)
                flapped = []
                if r_lineage != lineage:
                    flapped.append("read-lineage")
                if i_gen != gen:
                    flapped.append("generation")
                if i_size != entry["bytes"]:
                    flapped.append("size")
                if i_lineage != lineage:
                    flapped.append("lineage")
                if flapped:
                    report["notes"].append(
                        f"source identity flapped mid-read on "
                        f"{entry['path']} ({'+'.join(flapped)}) — page "
                        "discarded, source deferred")
                    deferred = True  # counted in backlog + gaps (F33.5)
                    break
                commit_batch(state, entry, obs, nxt,
                             {"cmd_fps": cmd_fps, "pending_fps": pending_fps,
                              "att_offs": att_offs})
                if page.raw is not None:
                    if len(local_chunks) < _MAX_ANCHOR_CHUNKS:
                        local_chunks.append({"start": cur,
                                             "len": len(page.raw),
                                             "sha": _sha(page.raw)})
                    elif not cap_noted:
                        report["notes"].append(
                            f"anchor chunk cap reached on {entry['path']} — "
                            "prefix integrity sampled beyond "
                            f"{_MAX_ANCHOR_CHUNKS} chunks")
                        cap_noted = True
                cur = nxt
                if done:
                    break
            if read_any:
                scanned.add(lineage)
                report["new_tail_bytes"] += src_bytes
            if (reset_occurred or gap_reconciled) and not read_any \
                    and not deferred:
                # a reset or gap reconciliation that consumed no pages
                # (budget starved behind an earlier source, or an empty
                # source) still persists its cursor so the reconcile is
                # ONE-TIME, never a per-pass note loop; the next pass
                # reads it as a plain fresh start (reset: cursor 0) or
                # from the anchored frontier (gap reconcile — carrying
                # the frontier checkpoint so the delayed reconcile never
                # downgrades to contaminated or empty state)
                gap_state = ({"cmd_fps": dict(gap_checkpoint.get("cmd_fps",
                                                                   {})),
                              "pending_fps": list(gap_checkpoint.get(
                                  "pending_fps", [])),
                              "att_offs": dict(gap_checkpoint.get(
                                  "att_offs", {}))}
                             if gap_reconciled and gap_checkpoint is not None
                             else {"cmd_fps": {}, "pending_fps": [],
                                   "att_offs": {}})
                commit_batch(state, entry, [], cur, gap_state)
            if deferred:
                backlog += 1
                report["gaps"] += 1
            elif cur < size and pages >= max_pages:
                backlog += 1
            if cur >= size:
                # closed SNAPSHOT coverage: everything visible at pass
                # start is consumed; bytes appended AFTER that are new
                # tail for the next pass — the two are reported apart
                report["coverage_complete"] += 1
            if utf8_hits:
                report["notes"].append(
                    f"utf8 replacement sequences in {entry['path']}: "
                    f"{utf8_hits} (detection-only decode; spans stay "
                    "byte-exact)")
            if not deferred and cur > 0:
                head_sha, tail_sha = _refresh_anchor_windows(p, min(cur, size))
                anchors[key] = {"size": cur, "head_sha": head_sha,
                                "tail_sha": tail_sha,
                                "chunks": local_chunks, "verify_pos": prev_pos,
                                # frontier detector checkpoint: the state
                                # as-of this frontier, saved in the SAME
                                # atomic write — crash recovery replays a
                                # gap from here instead of wiping valid
                                # prefix context (CSR-03 EDITOR-01)
                                "det": {"cmd_fps": cmd_fps,
                                        "pending_fps": pending_fps,
                                        "att_offs": att_offs},
                                "at": _now()}
                anchors_dirty = True
            if anchors_dirty:
                _save_anchors(state, anchors)
        if cohort:
            _save_scheduler(state, (rr + served) % len(cohort))
        report["backlog_remaining"] = backlog
        # opportunities use the MEASURED scanned cohort (sol t31 F31.2), never
        # the enrolled count; both are reported
        _atomic_write_json(state / "scanned.json", sorted(scanned))
        report["scanned_lineages"] = len(scanned)
        report["enrolled_lineages"] = manifest.get("lineages", 0)
        obs_rows, corrupt_n, _foreign = load_observations_report(state)
        if corrupt_n:
            report["notes"].append(
                f"observation store: {corrupt_n} corrupt lines skipped "
                "(counted, not silent)")
        patterns = group_patterns(obs_rows, scanned_lineages=len(scanned))
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
