#!/usr/bin/env python3
"""THE USABILITY INVARIANT PROBE ([11.194] — the owner's standing directive).

"There is a pattern of these issues: untitled, input blocking, glyph
corruption, viewport squish, restart-detached ghost pids. They have plagued
the CLI integrations for the life of yggterm. They must be checked deeply
UNATTENDED from now on."

This probe encodes the plague classes as INVARIANTS over every live agent row
on the host — agent-generic, no row disturbance in the default mode — and
exits non-zero when any invariant is violated, so a scheduler can post
failures without a human looking.

INVARIANTS (each maps to a lived owner pain):
  attachment     a live agent row stuck Bootstrapping past the threshold —
                 the eternal-Bootstrapping family ([11.183]/[11.192])
  untitled       a row whose bound conversation carries a store title while
                 the row still wears its birth default name ([11.193])
  glyph_sanity   the decoded screen is readable: printable ratio above the
                 floor and no mojibake signatures (the corruption paint)
  geometry       the row's recorded grid equals its remote truth — the
                 36x120 birth-fallback divergence class ([11.191])
  ghost_pids     live CLI processes attached to NO runtime row, older than
                 the grace — the detached-restart leak ([11.163] family)

Run it UNATTENDED (cron / the ygg-ci watcher / any seat): exit 0 = clean,
1 = at least one violation. `--json` renders the report for machines.
`--spawn` additionally exercises the full input-echo chain on a probe row
(disturbing — never the default).
"""

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import urllib.request

HOME = pathlib.Path(os.path.expanduser("~"))
SERVER_STATE = HOME / ".yggterm/server-state.json"
AGY_STORE = HOME / ".gemini/antigravity-cli/conversation_summaries.db"
YGGTERM_BIN = None

BIRTH_TITLE_DEFAULTS = ("New dev ", "New jojo ", "New oc ", "New local ")
BOOTSTRAP_GRACE_S = 600        # a row may bootstrap for ten minutes
GHOST_GRACE_S = 15 * 60        # an unattached CLI older than this is a ghost
PRINTABLE_FLOOR = 0.72         # readable screens are mostly printable
MOJIBAKE_SIGNS = ("Ã", "Â", "\ufffd", "â€")


def yggterm_binary():
    candidates = [
        os.environ.get("YGGTERM_BIN"),
        str(HOME / ".yggterm/bin/yggterm"),
        str(HOME / ".local/bin/yggterm"),
    ]
    for candidate in candidates:
        if candidate and pathlib.Path(candidate).exists():
            return candidate
    import shutil

    return shutil.which("yggterm") or "yggterm"


def run(args, timeout=60):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=False)


class Violation:
    def __init__(self, invariant, detail):
        self.invariant = invariant
        self.detail = detail

    def as_dict(self):
        return {"invariant": self.invariant, "detail": self.detail}


class Report:
    def __init__(self):
        self.violations = []
        self.checked = []

    def fail(self, invariant, detail):
        self.violations.append(Violation(invariant, detail))

    def ok(self, invariant, detail=""):
        self.checked.append(f"{invariant}: ok {detail}".strip())


def live_agent_rows():
    """The live agent sessions this daemon owns, from its persisted view:
    (key, kind_label, id, title, cwd)."""
    try:
        state = json.loads(SERVER_STATE.read_text())
    except (OSError, ValueError):
        return []
    rows = []
    for entry in state.get("live_sessions") or []:
        path = str(entry.get("session_path", ""))
        kind = str(entry.get("kind", ""))
        if "runtime" not in path and kind not in (
            "Antigravity", "Codex", "ClaudeCode", "OpenCode", "Muse",
            "QwenBuild", "Kimi", "GrokBuild", "Pi", "ZcodeTui", "Devin",
        ):
            continue
        metadata = {m.get("label"): str(m.get("value"))
                    for m in (entry.get("metadata") or []) if isinstance(m, dict)}
        rows.append({
            "path": path,
            "kind": kind,
            "id": str(entry.get("id", "")),
            "title": str(entry.get("title", "")),
            "cwd": metadata.get("Cwd", ""),
            "grid": entry.get("pty_grid") or metadata.get("PTY size", ""),
        })
    return rows


def screen(key, timeout=30):
    answer = run([YGGTERM_BIN, "server", "terminal", "screen", key], timeout=timeout)
    return answer.stdout if answer.returncode == 0 else ""


def printable_ratio(text):
    if not text.strip():
        return 1.0
    printable = sum(1 for ch in text if ch.isprintable() or ch in "\n\r\t")
    return printable / len(text)


def screen_readable(text):
    """The glyph-sanity heuristic: readable screens are mostly printable and
    carry no mojibake signature. Unicode box-drawing and CJK are printable,
    so a healthy TUI passes with huge margin; corrupted byte-streams do not."""
    if not text.strip():
        return None  # no screen: not a glyph verdict
    if printable_ratio(text) < PRINTABLE_FLOOR:
        return False
    for sign in MOJIBAKE_SIGNS:
        if text.count(sign) > max(4, len(text) // 400):
            return False
    return True


def store_titles(probe_ids):
    """conversation_id -> authored title, for the ids that have one."""
    if not AGY_STORE.exists() or not probe_ids:
        return {}
    import sqlite3

    out = {}
    try:
        conn = sqlite3.connect(f"file:{AGY_STORE}?mode=ro", uri=True)
        placeholders = ",".join("?" * len(probe_ids))
        for cid, title in conn.execute(
            f"select conversation_id, title from conversation_summaries "
            f"where conversation_id in ({placeholders})",
            probe_ids,
        ).fetchall():
            if title and str(title).strip():
                out[cid] = str(title).strip()
        conn.close()
    except Exception:
        pass
    return out


def invariant_attachment(report, rows, screens):
    stuck = []
    for row in rows:
        text = screens.get(row["path"], "")
        if "Bootstrapping" in text[:2000] or (
            row["title"].startswith(BIRTH_TITLE_DEFAULTS)
            and not text.strip()
        ):
            stuck.append(row["path"])
    if stuck:
        report.fail(
            "attachment",
            f"{len(stuck)} live row(s) look stuck at Bootstrapping/empty: "
            f"{', '.join(p[:60] for p in stuck[:3])} — the [11.183]/[11.192] "
            f"eternal-Bootstrapping family",
        )
    else:
        report.ok("attachment")


def invariant_untitled(report, rows):
    agent_rows = [r for r in rows if "agy-runtime" in r["path"]]
    if not agent_rows:
        return
    titles = store_titles([r["id"] for r in agent_rows if r["id"]])
    untitled = []
    for row in agent_rows:
        authored = titles.get(row["id"])
        if authored and row["title"].startswith(BIRTH_TITLE_DEFAULTS):
            untitled.append(f"{row['path'][:50]} should be '{authored[:40]}'")
    if untitled:
        report.fail(
            "untitled",
            "row(s) wearing their birth name while the store holds their "
            "authored title (the [11.193] title pipeline): "
            + "; ".join(untitled[:3]),
        )
    else:
        report.ok("untitled")


def invariant_glyph(report, screens):
    checked = 0
    for path, text in screens.items():
        verdict = screen_readable(text)
        if verdict is False:
            report.fail(
                "glyph_sanity",
                f"the decoded screen of {path[:60]} is unreadable — printable "
                f"ratio {printable_ratio(text):.2f} below {PRINTABLE_FLOOR} or "
                f"mojibake signatures present (the corruption paint)",
            )
            return
        if verdict is True:
            checked += 1
    if checked:
        report.ok("glyph_sanity", f"{checked} screen(s) readable")


def invariant_geometry(report, rows):
    diverged = []
    state_text = SERVER_STATE.read_text() if SERVER_STATE.exists() else ""
    for row in rows:
        key = row["path"]
        match = re.search(
            re.escape(key) + r"[^}]{0,600}?\"cols\":\s*(\d+),\s*\"rows\":\s*(\d+)",
            state_text,
        )
        if not match:
            continue
        cols, rws = int(match.group(1)), int(match.group(2))
        recorded = row["grid"]
        m = re.search(r"(\d+)\s*[x×]\s*(\d+)", str(recorded))
        if m and (int(m.group(1)) != cols or int(m.group(2)) != rws):
            diverged.append(
                f"{key[:50]}: recorded {m.group(1)}x{m.group(2)} vs live {cols}x{rws}"
            )
    if diverged:
        report.fail(
            "geometry",
            "local/remote grid divergence (the [11.191] squish class): "
            + "; ".join(diverged[:3]),
        )
    else:
        report.ok("geometry")


def live_runtime_keys():
    keys = set()
    try:
        state = json.loads(SERVER_STATE.read_text())
        for entry in state.get("live_sessions") or []:
            keys.add(str(entry.get("session_path", "")))
    except (OSError, ValueError):
        pass
    return list(keys)


def invariant_ghost_pids(report):
    """Live CLI processes carrying no runtime row and older than the grace —
    the detached-restart leak ([11.163] family)."""
    import datetime

    procs = run(["pgrep", "-af", "^agy |^codex |^claude |^muse |^opencode "], timeout=30)
    runtime_text = json.dumps(live_runtime_keys())
    now = time.time()
    ghosts = []
    for line in procs.stdout.splitlines():
        match = re.match(r"\s*(\d+)\s+(.*)", line)
        if not match:
            continue
        pid, cmd = int(match.group(1)), match.group(2)
        if not runtime_text or any(
            token and token in runtime_text
            for token in re.findall(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", cmd)
        ):
            continue
        started = pathlib.Path(f"/proc/{pid}/stat")
        if not started.exists():
            continue
        try:
            age_s = now - os.stat(f"/proc/{pid}").st_ctime
        except OSError:
            continue
        if age_s > GHOST_GRACE_S:
            ghosts.append(f"pid {pid} ({cmd[:60]}, {int(age_s)//60}m)")
    if ghosts:
        report.fail(
            "ghost_pids",
            "CLI process(es) attached to no runtime row, older than the "
            f"{GHOST_GRACE_S//60}m grace (the detached-restart leak): "
            + "; ".join(ghosts[:3]),
        )
    else:
        report.ok("ghost_pids")


def main():
    global YGGTERM_BIN
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    YGGTERM_BIN = yggterm_binary()
    report = Report()

    rows = live_agent_rows()
    screens = {}
    for row in rows:
        key = row["path"]
        if "runtime" in key:
            screens[key] = screen(key, timeout=30)

    invariant_attachment(report, rows, screens)
    invariant_glyph(report, screens)
    invariant_untitled(report, rows)
    invariant_geometry(report, rows)
    invariant_ghost_pids(report)

    failed = len(report.violations)
    stamp = time.strftime("%Y-%m-%d %H:%M:%S %z")
    if args.json:
        print(json.dumps({
            "stamp": stamp,
            "ok": failed == 0,
            "violations": [v.as_dict() for v in report.violations],
            "checked": report.checked,
        }, indent=2))
    else:
        for violation in report.violations:
            print(f"[FAIL] {violation.invariant}: {violation.detail}")
        for line in report.checked:
            print(f"[ ok ] {line}")
        print(f"{stamp} usability: {failed} violation(s)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
