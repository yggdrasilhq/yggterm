"""Where a row's transcript lives — for EVERY agent CLI, not just the one we grew up on.

⛔⛔ **THE DEFECT THIS EXISTS TO END.** Eleven callsites across seven fleet verbs
answered *"has this row ever written a word?"* with one hardcoded glob — the
reference CLI's store. The registry declares a store for every registered CLI and
they share no layout, so for a row of any other CLI that glob returns nothing —
**and nothing is not an error, it is an answer**, the same answer a row that has
genuinely never done anything gives.

⇒ The consequences ran from cosmetic to destructive:

    ygg-spawn     cannot prove a brief arrived   → a lane is born unverifiable
    ygg-monitor   reads no prose, no mtime       → a working row looks stalled
    ygg-deliver   cannot prove delivery, AND     → **reaps a row it just wrote to**
                  treats the silence as "never briefed"

⚠ `ygg-deliver`'s reap is the one that matters, and its own comment states the
asymmetry it meant to respect: *losing a working lane to a delivery timeout would
be far worse than the debris this cleans up.* For a row outside the reference store
that test can only ever answer no, so the interlock was correctly reasoned and
reading the wrong shelf.

⭐ **The id position is the only thing the registry does not already publish**, so
that is all `cli-stores.json` adds — and a Rust lock
(`the_fleet_transcript_table_matches_the_registry`) fails the build if the store
half of it drifts from the registry. Six of the templates were resolved against a
REAL store on disk, not a fixture; the ones that were not say so.

⛔ **A CLI that declares no store resolves to nothing, deliberately.** Guessing a
path would make an unmeasured CLI indistinguishable from a measured one, which is
the failure this area keeps repeating.
"""
import glob as _glob
import json
import os

_HERE = os.path.dirname(os.path.abspath(__file__))
_TABLE = None


def _table():
    global _TABLE
    if _TABLE is None:
        with open(os.path.join(_HERE, "cli-stores.json")) as handle:
            _TABLE = json.load(handle)["clis"]
    return _TABLE


def templates_for(kind=None):
    """The transcript templates to try, in order.

    `kind` is a row's `icon_kind` — the registry's own slug for the CLI. Passing it
    is a narrowing, never a requirement: a caller that only holds a uuid (the
    watchdogs mostly do) gets every declared store tried, which is correct because
    a session id is unique across them and a glob that matches nothing costs a
    syscall.
    """
    table = _table()
    if kind and kind in table:
        entry = table[kind]
        return [entry["transcript"]] if entry.get("transcript") else []
    return [e["transcript"] for e in table.values() if e.get("transcript")]


def transcript_paths(uuid, kind=None, home=None):
    """Every local file that could be this session's transcript, newest first."""
    if not uuid:
        return []
    home = home or os.path.expanduser("~")
    hits = []
    for template in templates_for(kind):
        pattern = os.path.join(home, template.replace("{id}", uuid))
        hits.extend(_glob.glob(pattern, recursive=True))
    # ⚠ Newest first: a CLI that rotates or re-opens a session writes more than one
    #   file, and the caller almost always wants the one still being appended to.
    return sorted(set(hits), key=lambda p: _mtime(p), reverse=True)


def _mtime(path):
    try:
        return os.path.getmtime(path)
    except OSError:
        return 0


def transcript_of(uuid, kind=None, home=None):
    """The single best transcript path for this session, or None."""
    hits = transcript_paths(uuid, kind, home)
    return hits[0] if hits else None


def has_transcript(uuid, kind=None, home=None):
    """⛔ Has this row EVER written a word? The question a reap turns on.

    Read the docstring at the top of this file before using this to destroy
    anything: the old answer was "no" for every CLI but one.
    """
    return bool(transcript_paths(uuid, kind, home))


def carries(uuid, token, kind=None, home=None, tail_bytes=400_000):
    """Does this session's transcript contain `token`? — the delivery proof.

    ⚖ A substring search over the raw file is deliberate and portable: every
    declared store is line-oriented text, and the ack only has to be FOUND, not
    parsed. Reading a record's structure differs per CLI and is a separate problem
    from finding the file, which is the one that was blocking every CLI at once.
    """
    for path in transcript_paths(uuid, kind, home):
        try:
            with open(path, errors="ignore") as handle:
                try:
                    handle.seek(max(0, os.path.getsize(path) - tail_bytes))
                except OSError:
                    pass
                if token in handle.read():
                    return True
        except OSError:
            continue
    return False


def remote_find_command(uuid, kind=None):
    """One shell line that prints this session's transcript paths on another host.

    ⛔ Built here rather than in each verb because a verb that writes its own glob
    writes the reference CLI's, which is how this started.

    ⛔⛔ **`**` IS NOT A GLOB IN `sh`.** Three of the declared stores nest their
    sessions under dated directories and are declared with `**`, which Python's
    `glob(recursive=True)` expands and a POSIX shell does not — it is a bash
    extension and `globstar` is off by default. So the obvious `ls` form returns
    NOTHING for those three, over ssh, silently, and the callers read that silence
    as "this row has never written a word" — which is the exact defect this module
    was written to end, reintroduced one layer down. Caught by running it, not by
    reading it.

    ⇒ `find -path` instead, where a single `*` spans `/` and no extension is
    needed. The search is rooted at the fixed leading part of each template so it
    walks one store rather than a home directory.
    """
    if not uuid:
        return "true"
    parts = []
    for template in templates_for(kind):
        pattern = template.replace("{id}", uuid)
        if "*" not in pattern:
            # No wildcard left once the id is substituted — name the file directly.
            parts.append("ls -1d " + _shell_quote_home(pattern) + " 2>/dev/null")
            continue
        root = _static_root(pattern)
        parts.append(
            "find " + _shell_quote_home(root) + " -path "
            + _shell_quote_home(pattern.replace("**", "*")) + " 2>/dev/null"
        )
    if not parts:
        return "true"
    # ⚠ Newest first, to match the local resolver's ordering.
    return "{ " + " ; ".join(parts) + " ; } | xargs -r ls -1dt 2>/dev/null"


def _static_root(pattern):
    """The longest leading run of path segments that carries no wildcard."""
    kept = []
    for segment in pattern.split("/"):
        if "*" in segment:
            break
        kept.append(segment)
    return "/".join(kept) or "."


def _shell_quote_home(relative):
    """`"$HOME"/'<literal>'` — the home expands, the pattern never does."""
    return '"$HOME"/' + "'" + relative.replace("'", "'\\''") + "'"


#: How much of a transcript's tail to parse when looking for its last records, and
#: the one escalation allowed before giving up.
#:
#: ⛔⛔ **WHY A BOUND EXISTS AT ALL, MEASURED 2026-08-22.** Three callers read a
#: transcript with `[json.loads(l) for l in open(path)]` — the WHOLE file, every
#: line parsed into memory. That was survivable only because they could see one
#: CLI's store, whose largest file on this fleet is 36 MB. Widening the lookup to
#: every store put a **1,481 MB** codex transcript in reach of the same line, on a
#: timer, and one of the three runs OVER SSH — so the gigabytes would have been
#: allocated on the remote machine, which is somebody's laptop.
#:
#: ⚠ The distribution is what makes this a trap rather than an obvious bug: the p95
#: codex transcript is **5.4 MB**. The maximum is 274x that. Anything sized by
#: eyeballing a typical file is wrong by two orders of magnitude on the tail, and
#: the tail is exactly where a long-running agent row ends up.
TAIL_BYTES = 2_000_000
TAIL_BYTES_ESCALATED = 16_000_000


def tail_records(path, max_bytes=TAIL_BYTES):
    """Parsed JSON records from the END of a transcript, oldest first.

    ⚠ A partial first line is DROPPED, not repaired: seeking into the middle of a
    file lands mid-record, and a half-record that happens to parse is worse than
    one that is skipped.
    """
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as handle:
            if size > max_bytes:
                handle.seek(size - max_bytes)
                handle.readline()          # discard the partial record
            blob = handle.read()
    except OSError:
        return []
    records = []
    for line in blob.decode("utf-8", "replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            records.append(json.loads(line))
        except ValueError:
            continue
    return records


def last_matching_record(path, predicate):
    """The newest record satisfying `predicate`, searching a bounded tail.

    ⚖ **The escalation is the correctness half.** A row whose last words are
    followed by a very long run of tool output would fall outside a single window,
    and answering "this row said nothing" about a row that spoke is the failure
    this whole module exists to stop. So: widen once, then stop — a bound that
    grows without limit is the unbounded read it replaced.
    """
    for window in (TAIL_BYTES, TAIL_BYTES_ESCALATED):
        for record in reversed(tail_records(path, window)):
            if predicate(record):
                return record
        try:
            if os.path.getsize(path) <= window:
                break                      # the whole file was already read
        except OSError:
            break
    return None


def prose_of(record):
    """The assistant's own words in one transcript record, or None.

    ⛔ **ONLY SHAPES THAT HAVE BEEN READ OFF A REAL STORE APPEAR HERE.** Each CLI
    spells a turn differently and the differences are not cosmetic — one nests the
    text two levels inside a payload, one puts it in a bare string beside a
    separate `thinking` field. A guessed fourth shape would return tool output as
    if it were the row's last words, which is worse than the blank it replaces:
    a wrong answer here goes into a stall verdict.

    ⇒ An unrecognised record returns None and the caller reports nothing, which is
    honest. Add a shape by MEASURING it, the way these three were.
    """
    kind = record.get("type")

    # Claude Code: assistant record, message.content[] blocks.
    if kind == "assistant":
        for block in (record.get("message") or {}).get("content") or []:
            if isinstance(block, dict) and block.get("type") == "text":
                text = (block.get("text") or "").strip()
                if text:
                    return text
        return None

    # Codex: the turn is a `response_item` and the role lives in the payload.
    # ⚠ The payload also carries non-message items (tool calls, reasoning), so the
    #   role AND the item type both have to be checked.
    if kind == "response_item":
        payload = record.get("payload") or {}
        if payload.get("type") == "message" and payload.get("role") == "assistant":
            for block in payload.get("content") or []:
                if isinstance(block, dict) and block.get("type") == "output_text":
                    text = (block.get("text") or "").strip()
                    if text:
                        return text
        return None

    # Antigravity: a bare string on the record. `thinking` sits beside it and is
    # deliberately NOT read — it is the model's scratchpad, not what it said.
    if kind == "PLANNER_RESPONSE":
        text = (record.get("content") or "").strip()
        return text or None

    return None


def last_prose(path):
    """The newest thing the agent in this transcript actually SAID, or ""."""
    if not path:
        return ""
    record = last_matching_record(path, lambda r: prose_of(r) is not None)
    return prose_of(record) if record is not None else ""
