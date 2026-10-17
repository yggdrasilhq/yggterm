#!/usr/bin/env python3
"""The fleet task-ledger planner (owner directive 2026-10-17).

Parses the ledger door (_global/fleet-task-ledger.md — synced to every
host by ordinary memory sync), sorts open tasks by tokenomics, and emits
the daily execution batches:

  GLM idle   exactly 1 BIG + 2 MODERATE (the owner's 3/day allowance)
  codex      moderate-to-SMALL only (owner correction 2026-10-17)
  manual     everything else incl. owner-gated/blocked, fireable by id

Routing hints in each entry's `route:` line refine placement (e.g. an
entry routed codex never takes a GLM slot even if top-scoring). deps
other than `none` hold a task out of automatic sorting (listed with its
deps instead). Run from any host: the hub door is local everywhere.

  python3 plan.py                     # today's batches, human-readable
  python3 plan.py --json              # machine form
  python3 plan.py --selftest          # synthetic ordering check
"""
import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

LEDGER = os.path.expanduser(
    "~/.yggterm/memory/namespaces/_global/fleet-task-ledger.md")
WEIGHT = {"big": 4.0, "moderate": 2.0, "small": 1.0}
BLOCK_RE = re.compile(r"^### \[(T-\d+)\] (.+)$", re.M)


def parse_ledger(text):
    tasks = []
    heads = list(BLOCK_RE.finditer(text))
    for i, m in enumerate(heads):
        body_start = m.end()
        body_end = heads[i + 1].start() if i + 1 < len(heads) else len(text)
        block = text[body_start:body_end]
        task = {"id": m.group(1), "title": m.group(2).strip()}
        # fields are "- key: value" bullets; the first bullet packs
        # several fields as "size: X ; value: Y ; est: Z" — split on " ; "
        for line in block.splitlines():
            line = line.strip()
            if not (line.startswith("- ") and ":" in line):
                continue
            for seg in line[2:].split(" ; "):
                if ":" in seg:
                    k, v = seg.split(":", 1)
                    task[k.strip()] = v.strip()
        tasks.append(task)
    return tasks


def score(task):
    size = "big" if "big" in task.get("size", "") else (
        "moderate" if "moderate" in task.get("size", "") else "small")
    value = 0
    m = re.search(r"(\d)", task.get("value", "0"))
    if m:
        value = int(m.group(1))
    return value / WEIGHT[size], size


def plan(tasks):
    ready, held, gated, blocked, done = [], [], [], [], []
    for t in tasks:
        status = t.get("status", "open")
        if status.startswith("done"):
            done.append(t)
        elif status.startswith("blocked"):
            blocked.append(t)
        elif "OWNER-GATED" in t.get("route", "").upper() or "gated" in status:
            gated.append(t)
        elif not t.get("deps", "none").strip().lower().startswith("none"):
            held.append(t)
        else:
            ready.append(t)
    ranked = sorted(ready, key=lambda t: -score(t)[0])

    glm_big, glm_mod, codex, manual, glm_wait = [], [], [], [], []
    for t in ranked:
        s, size = score(t)
        route = t.get("route", "").lower()
        entry = {"id": t["id"], "title": t["title"], "size": size,
                 "score": round(s, 2), "route": t.get("route", "")}
        wants_glm = "glm" in route or route.startswith("any") or route == ""
        wants_codex = "codex" in route or route.startswith("any") or route == ""
        if wants_glm and not glm_big and size == "big":
            glm_big.append(entry)
        elif wants_glm and len(glm_mod) < 2 and size == "moderate":
            glm_mod.append(entry)
        elif wants_codex and size in ("moderate", "small"):
            codex.append(entry)
        elif wants_glm:
            glm_wait.append(entry)  # glm-routed but today's batch is full
        else:
            manual.append(entry)
    glm = glm_big + glm_mod  # the big slot reads first
    return {
        "glm_idle_batch": {"law": "exactly 1 BIG + 2 MODERATE (owner 3/day)",
                           "tasks": glm},
        "codex_queue": {"law": "moderate-to-small only (owner correction)",
                        "tasks": codex},
        "glm_waitlist": glm_wait,
        "manual_fire": manual,
        "held_for_deps": [{"id": t["id"], "title": t["title"],
                           "deps": t.get("deps", "")} for t in held],
        "owner_gated": [{"id": t["id"], "title": t["title"]} for t in gated],
        "blocked": [{"id": t["id"], "title": t["title"],
                     "status": t.get("status", "")} for t in blocked],
        "done": [{"id": t["id"], "title": t["title"]} for t in done],
    }




def mark_done(text, tid, proof, by, date):
    """Transform the ledger markdown: task status -> done, §BOOK one-liner.
    Pure text transform (no writes) — the caller publishes via ygg-memory."""
    m = re.search(r"^### \[" + re.escape(tid) + r"\] (.+)$", text, re.M)
    if not m:
        raise SystemExit(f"task {tid} not found in the ledger")
    title = m.group(1).strip()
    nxt = re.search(r"^### \[T-", text[m.end():], re.M)
    block_end = m.end() + nxt.start() if nxt else len(text)
    block = text[m.end():block_end]
    if re.search(r"^- status: done", block, re.M):
        raise SystemExit(f"task {tid} is already done")
    new_block, n = re.subn(r"^- status: .*$",
                           f"- status: done({proof}) [by {by}, {date}]",
                           block, count=1, flags=re.M)
    if n != 1:
        raise SystemExit(f"task {tid} has no status line to book")
    text = text[:m.end()] + new_block + text[block_end:]
    book_hdr = "## §BOOK — one-liners of finished work"
    if book_hdr in text:
        line = f"- {date} {tid} done({proof}) [by {by}] — {title}"
        text = text.replace(book_hdr, book_hdr + "\n" + line, 1)
    return text


def mark_claimed(text, tid, by, date, note=""):
    """Transform the ledger markdown: task status -> claimed(by, date).
    Pure text transform — the caller publishes via ygg-memory. The claim is
    ID-KEYED (same discipline as mark_done): the seat hands the task id and
    this matches the exact block — hand regex over the door text has the
    mis-target hazard measured 2026-10-17 (a claim edit skipped past a
    just-claimed T-002 and landed on T-003). Refuses when the task is no
    longer open in the text being claimed."""
    m = re.search(r"^### \[" + re.escape(tid) + r"\] (.+)$", text, re.M)
    if not m:
        raise SystemExit(f"task {tid} not found in the ledger")
    nxt = re.search(r"^### \[T-", text[m.end():], re.M)
    block_end = m.end() + nxt.start() if nxt else len(text)
    block = text[m.end():block_end]
    if not re.search(r"^- status: open", block, re.M):
        raise SystemExit(f"task {tid} is no longer open — another seat took it first")
    suffix = f" — {note}" if note else ""
    new_block, n = re.subn(r"^- status: .*$",
                           f"- status: claimed({by}, {date}){suffix}",
                           block, count=1, flags=re.M)
    if n != 1:
        raise SystemExit(f"task {tid} has no status line to claim")
    return text[:m.end()] + new_block + text[block_end:]


def selftest():
    synthetic = """
### [T-101] low value small
- size: small ; value: 1 ; est: x
- route: codex ; deps: none
- status: open

### [T-102] high value big
- size: big ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: open

### [T-103] high value moderate
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: open

### [T-104] gated panel
- size: small ; value: 4 ; est: x
- route: OWNER-GATED ; deps: none
- status: open (gated)

### [T-105] held for deps
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: mail mechanics verified
- status: open

### [T-106] second moderate
- size: moderate ; value: 4 ; est: x
- route: any ; deps: none
- status: open

## §BOOK — one-liners of finished work
"""
    p = plan(parse_ledger(synthetic))
    ids = [t["id"] for t in p["glm_idle_batch"]["tasks"]]
    assert ids == ["T-102", "T-103", "T-106"], ids  # big slot first
    assert all(t["id"] != "T-105" for t in p["glm_idle_batch"]["tasks"])
    assert p["owner_gated"] and p["owner_gated"][0]["id"] == "T-104"
    assert p["held_for_deps"][0]["id"] == "T-105"
    assert p["codex_queue"]["tasks"][0]["id"] == "T-101"
    booked = mark_done(synthetic, "T-101", "main abc123", "glm-idle", "2026-10-17")
    assert "- status: done(main abc123) [by glm-idle, 2026-10-17]" in booked
    assert "- 2026-10-17 T-101 done(main abc123) [by glm-idle]" in booked
    claimed = mark_claimed(synthetic, "T-102", "zcode", "2026-10-17", "on a lane")
    assert "- status: claimed(zcode, 2026-10-17) — on a lane" in claimed
    assert claimed.count("- status: open") == synthetic.count("- status: open") - 1
    for bad_text, bad_id in ((claimed, "T-102"), (synthetic, "T-999")):
        try:
            mark_claimed(bad_text, bad_id, "zcode", "2026-10-17")
            raise AssertionError(f"claim {bad_id} should refuse")
        except SystemExit:
            pass
    print("selftest green (plan + mark_done + mark_claimed)")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ledger", default=LEDGER)
    ap.add_argument("--day", default=datetime.date.today().isoformat())
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--done", metavar="TASK_ID",
                    help="book a task done: rewrites the ledger door via "
                         "ygg-memory publish (hub doors are never edited "
                         "in place) + adds the §BOOK one-liner")
    ap.add_argument("--claim", metavar="TASK_ID",
                    help="book a claim (id-keyed, refuses a no-longer-open "
                         "task): rewrites the ledger door via ygg-memory "
                         "publish; the safe way to mark a task taken")
    ap.add_argument("--note", default="",
                    help="with --claim: the claim note (what/where)")
    ap.add_argument("--proof", help="proof ref for --done (lane/main sha, board ACK, door seq)")
    ap.add_argument("--by", default="glm-idle-session",
                    help="who completed it (default: glm-idle-session)")
    ap.add_argument("--dry-run", action="store_true",
                    help="with --done/--claim: print the transform, do not publish")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.claim:
        text = open(a.ledger, encoding="utf-8").read()
        new = mark_claimed(text, a.claim, a.by, a.day, a.note)
        if a.dry_run:
            print(new)
            return 0
        tmp = tempfile.NamedTemporaryFile("w", suffix=".md", delete=False,
                                          encoding="utf-8")
        tmp.write(new)
        tmp.close()
        runner = shutil.which("ygg-memory") or os.path.expanduser(
            "~/.local/bin/ygg-memory.py")
        r = subprocess.run([sys.executable, runner, "publish", "--file", tmp.name,
                            "--ns=_global", "--as", "fleet-task-ledger.md"],
                           capture_output=True, text=True)
        os.unlink(tmp.name)
        print(r.stdout.strip() or r.stderr.strip())
        print(f"booked {a.claim} claimed({a.by}) — the task is taken; "
              "execution in flight")
        return r.returncode
    if a.done:
        if not a.proof:
            print("--done needs --proof \"<ref>\"")
            return 2
        text = open(a.ledger, encoding="utf-8").read()
        new = mark_done(text, a.done, a.proof, a.by, a.day)
        if a.dry_run:
            print(new)
            return 0
        tmp = tempfile.NamedTemporaryFile("w", suffix=".md", delete=False,
                                          encoding="utf-8")
        tmp.write(new)
        tmp.close()
        runner = shutil.which("ygg-memory") or os.path.expanduser(
            "~/.local/bin/ygg-memory.py")
        r = subprocess.run([sys.executable, runner, "publish", "--file", tmp.name,
                            "--ns=_global", "--as", "fleet-task-ledger.md"],
                           capture_output=True, text=True)
        os.unlink(tmp.name)
        print(r.stdout.strip() or r.stderr.strip())
        print(f"booked {a.done} done({a.proof}) [by {a.by}] — §BOOK line added; "
              "if the day's batch is exhausted, refresh §TODAY with a fresh plan run")
        return r.returncode
    if not os.path.exists(a.ledger):
        print(json.dumps({"error": f"ledger not found: {a.ledger}"}))
        return 2
    p = plan(parse_ledger(open(a.ledger, encoding="utf-8").read()))
    p["day"] = a.day
    if a.json:
        print(json.dumps(p, indent=1))
        return 0
    print(f"# fleet task plan — {a.day}")
    print(f"\n## GLM idle batch (1 big + 2 moderate — the 3/day allowance)")
    for t in p["glm_idle_batch"]["tasks"]:
        print(f"  {t['id']} [{t['size']}] (score {t['score']}) {t['title']}")
    print("\n## codex queue (moderate-to-small only)")
    for t in p["codex_queue"]["tasks"]:
        print(f"  {t['id']} [{t['size']}] (score {t['score']}) {t['title']}")
    if p["glm_waitlist"]:
        print("\n## GLM waitlist (glm-routed; next batches)")
        for t in p["glm_waitlist"]:
            print(f"  {t['id']} [{t['size']}] (score {t['score']}) {t['title']}")
    print("\n## manual fire (any harness, by id)")
    for t in p["manual_fire"]:
        print(f"  {t['id']} [{t['size']}] {t['title']}")
    if p["held_for_deps"]:
        print("\n## held for deps")
        for t in p["held_for_deps"]:
            print(f"  {t['id']} — deps: {t['deps']}")
    if p["owner_gated"]:
        print("\n## owner-gated")
        for t in p["owner_gated"]:
            print(f"  {t['id']} {t['title']}")
    if p["blocked"]:
        print("\n## blocked")
        for t in p["blocked"]:
            print(f"  {t['id']} — {t['status']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
