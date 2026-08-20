"""Durable-session rules shared by check-startpage / check-cwdtree / check-titles.

⛔ SINGLE OWNER of "which stored conversations are resumable sessions", on the
oracle side. The Rust side's owner is `yggterm_core::startpage`
(`antigravity_row_is_durable`, `kind_has_dedicated_scanner`,
`is_noise_session_file`). These two must agree; when they disagree the oracle
exits non-zero, which is the entire point of having two implementations.

Before 2026-08-20 the oracles walked FILES only. Antigravity keeps its index in
SQLite, so the walk saw none of it and reported every agy row the verb produced
as a spurious "extra" — 999 of them. A checker that cannot see a whole CLI is
not a check.
"""

import json
import shlex

SCRATCH_ROOTS = ("/tmp", "/var/tmp", "/private/tmp", "/private/var/folders")


def path_is_ephemeral_scratch(path):
    """Mirror of `yggterm_core::path_is_ephemeral_scratch`.

    A PATH test, deliberately not an existence test: two batch conversations in
    the measured store still had live /tmp dirs, and probing the filesystem
    would make the answer change as /tmp is reaped.
    """
    return any(path == root or path.startswith(root + "/") for root in SCRATCH_ROOTS)


def antigravity_row_is_durable(workspace_uris, step_count):
    """Mirror of `yggterm_core::startpage::antigravity_row_is_durable`.

    ⛔ `killed` decides nothing — every row of a measured 999-row store had
    killed=0. Only `step_count` and `workspace_uris` carry signal.
    """
    try:
        raw = json.loads(workspace_uris)
    except Exception:
        return False
    if not isinstance(raw, list):
        return False
    roots = []
    for uri in raw:
        if not isinstance(uri, str):
            continue
        p = uri[7:] if uri.startswith("file://") else uri
        p = p.strip().rstrip("/")
        if p:
            roots.append(p)
    if not roots or step_count <= 0:
        return False
    return not any(path_is_ephemeral_scratch(r) for r in roots)


# Runs ON the host being checked. Uses python3's sqlite3 module rather than the
# `sqlite3` CLI, which is not installed on every fleet host — the old checker
# shelled out to it and silently got nothing where it was absent.
_AGY_DUMP = r"""
import json, os, sqlite3
home = os.path.expanduser("~")
db = os.path.join(home, ".gemini/antigravity-cli/conversation_summaries.db")
rows = []
if os.path.exists(db):
    try:
        conn = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
        for cid, uris, steps, lm, title, preview in conn.execute(
            "select conversation_id, workspace_uris, step_count, last_modified_time,"
            " title, preview from conversation_summaries"
        ):
            rows.append({"id": cid, "uris": uris, "steps": steps,
                         "lm": lm, "title": title, "preview": preview})
    except Exception:
        rows = []
print(json.dumps(rows))
"""

_MUSE_NOISE_DUMP = r"""
import json, os, sqlite3
home = os.path.expanduser("~")
db = os.path.join(home, ".local/share/muse/session-index.db")
noise = []
if os.path.exists(db):
    try:
        conn = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
        for sid, pc, title in conn.execute(
            "select session_id, prompt_count, title from sessions"
        ):
            t = (title or "").strip().lower()
            if pc == 0 and t in ("", "new session", "new muse code session"):
                noise.append(sid)
    except Exception:
        noise = []
print(json.dumps(noise))
"""


def _run_python(run_on_host, host, script):
    out, err = run_on_host(host, "python3 -c " + shlex.quote(script))
    if err:
        return None
    try:
        return json.loads(out.strip().splitlines()[-1])
    except Exception:
        return None


def agy_durable_rows(run_on_host, host, home):
    """The agy conversations that are resumable sessions, as the verb sees them.

    Returns a list of dicts (id, cwd, title, mtime_ms) or [] when unreadable.
    """
    rows = _run_python(run_on_host, host, _AGY_DUMP)
    if not rows:
        return []
    out = []
    for r in rows:
        if not antigravity_row_is_durable(r.get("uris") or "", r.get("steps") or 0):
            continue
        roots = []
        try:
            for uri in json.loads(r["uris"]):
                p = uri[7:] if uri.startswith("file://") else uri
                p = p.strip().rstrip("/")
                if p:
                    roots.append(p)
        except Exception:
            pass
        out.append({
            "id": r["id"],
            "cwd": roots[0] if roots else home,
            "title": (r.get("title") or "").strip() or (r.get("preview") or "").strip() or None,
        })
    return out


def muse_noise_ids(run_on_host, host):
    """Session ids the muse index marks as zero-prompt placeholders.

    ⚠ These have real files behind them (12 KB of lifecycle records is normal),
    so this set is for SKIPPING during a scan and must never drive a delete.
    """
    ids = _run_python(run_on_host, host, _MUSE_NOISE_DUMP)
    return set(ids or [])


_EXISTS_DUMP = r"""
import json, os, sys
paths = json.loads(sys.stdin.read())
# ⛔ isfile, not exists. A LIVE session with no transcript yet is injected with
# its CWD as storage_path (dual presence is spec), and a cwd is a directory that
# exists — so an existence test calls it store-backed and then reports it as
# drift for not being in a file walk it can never be in.
print(json.dumps([p for p in paths if p and os.path.isfile(p)]))
"""


def row_is_peer_scanned(row):
    """A row the daemon scanned from ANOTHER machine.

    ⛔ The honest discriminator is the machine segment in the display path, not
    the filesystem. A peer row is published as `remote-<slug>://<machine>/<id>`
    while a LOCAL row of the same CLI is `remote-<slug>://<id>` with no machine.
    Path existence cannot tell them apart: fleet hosts share home layouts, so a
    legacy store file scanned on one machine exists at the same path on another
    and reads as local.
    """
    display = (row.get("display_path") or "").strip()
    if "://" not in display:
        return False
    scheme, _, rest = display.partition("://")
    if not scheme.startswith("remote-"):
        return False
    return "/" in rest


def locally_backed_ids(run_on_host, host, rows):
    """Of `rows`, the ids backed by a real store FILE on THIS host.

    Everything else is a row a local file walk cannot contain, for one of two
    honest reasons: the daemon scanned it from a peer machine, or it is a live
    session with no transcript written yet.

    ⛔ `startpage`/`cwdtree` show the FLEET: the daemon merges sessions scanned
    over ssh from peer machines. A local file walk cannot see those, so counting
    them as "verb has ids not in manual walk" makes the oracle permanently red on
    any host with peers — 65 of them on a measured host — and buries the real
    mismatches it exists to surface.

    Scheme alone cannot decide this: a LOCAL agy row is published under
    `remote-agy://` too. The store file's presence is the honest test.
    """
    by_path = {}
    for r in rows:
        sid = r.get("session_id")
        sp = r.get("storage_path") or ""
        if not sid:
            continue
        if row_is_peer_scanned(r):
            continue  # another machine's row; never in a local walk
        if sp:
            by_path.setdefault(sp, []).append(sid)
    if not by_path:
        return set()
    payload = json.dumps(sorted(by_path))
    cmd = "python3 -c " + shlex.quote(_EXISTS_DUMP) + " <<'YGGEOF'\n" + payload + "\nYGGEOF"
    out, err = run_on_host(host, cmd)
    if err:
        return set()
    try:
        present = json.loads(out.strip().splitlines()[-1])
    except Exception:
        return set()
    ids = set()
    for p in present:
        ids.update(by_path.get(p, []))
    return ids


STORE_TITLE_MAX_CHARS = 72


def condense_store_title(raw):
    """Mirror of `yggterm_core::agent_cli::condense_store_title`.

    ⛔ The oracle must model the SHIPPED contract, not the one it was written
    against. A store `title` column is not always a title — one CLI records the
    first prompt verbatim and never updates it — so a row label is the first
    sentence, word-boundary clamped to 72 chars, with the full text kept as the
    row's `detail`. An oracle still comparing the raw store text reports drift
    that does not exist, which is how a checker teaches people to ignore it.
    """
    trimmed = (raw or "").strip()
    if not trimmed:
        return None
    if len(trimmed) <= STORE_TITLE_MAX_CHARS and ". " not in trimmed:
        return trimmed
    first_sentence = trimmed
    for i, ch in enumerate(trimmed):
        if ch in ".!?":
            first_sentence = trimmed[: i + 1]
            break
    candidate = first_sentence.strip().rstrip(".!?").strip() or trimmed
    if len(candidate) <= STORE_TITLE_MAX_CHARS:
        return candidate
    clamped = ""
    for word in candidate.split():
        projected = len(word) if not clamped else len(clamped) + 1 + len(word)
        if projected > STORE_TITLE_MAX_CHARS:
            break
        clamped = word if not clamped else clamped + " " + word
    return clamped or candidate[:STORE_TITLE_MAX_CHARS]
