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
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

from booking_outbox import write_intent, update_intent, default_path

LEDGER = os.path.expanduser(
    "~/.yggterm/memory/namespaces/_global/fleet-task-ledger.md")
WEIGHT = {"big": 4.0, "moderate": 2.0, "small": 1.0}
BLOCK_RE = re.compile(r"^#{3,} *(?:#+ *)?\[(T-\d+(?:-[A-Z])?)\] *(.+)$", re.M)  # -C suffix ids legal (T-024-C class); extra `#`s tolerated — a "### ### [T-022]" heading once swallowed T-022 into T-021's field dict and resurrected a done task into the batch (T-035, 2026-10-23)
# §BATCH daily-brief form: "- **T-022 [BIG] (open) — Title.** body" — the
# status lives inline in the parens (may nest one level, e.g.
# "(claimed(x, y) — note)"). Parsing it killed the claim-race class measured
# 2026-10-18: §BATCH ids were invisible to --claim, so seats hand-edited the
# door and a stale snapshot overwrite hit T-022 (infra/meta ACK-d0d108a714).
BATCH_ITEM_RE = re.compile(
    r"^- \*\*(?P<id>T-\d+(?:-[A-Za-z]+)?) \[(?P<size>[A-Za-z-]+)\] "
    r"\((?P<status>(?:[^()]|\([^()]*\))*)\) — (?P<title>.+?)\.\*\*",
    re.M)


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
        statuses = []
        for line in block.splitlines():
            line = line.strip()
            if not (line.startswith("- ") and ":" in line):
                continue
            for seg in line[2:].split(" ; "):
                if ":" in seg:
                    k, v = seg.split(":", 1)
                    k = k.strip()
                    if k == "status":
                        statuses.append(v.strip())
                    else:
                        task[k] = v.strip()
        if statuses:
            # done is ABSORBING: a stale auto-claim line appended after a done
            # line must not resurrect the task (T-022 sat in the waitlist that way)
            task["status"] = next((s for s in statuses if s.startswith("done")),
                                  statuses[-1])
        tasks.append(task)
    # §BATCH daily briefs: same task plane, inline-status form. They are the
    # already-prepared batch, so plan() reports them as-is (no re-ranking).
    for m in BATCH_ITEM_RE.finditer(text):
        if any(t["id"] == m.group("id") for t in tasks):
            continue
        tasks.append({"id": m.group("id"), "title": m.group("title").strip(),
                      "size": m.group("size").lower(), "status": m.group("status").strip(),
                      "batch": "true"})
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
    ready, held, gated, blocked, done, claimed = [], [], [], [], [], []
    batch = []
    for t in tasks:
        if t.get("batch"):
            batch.append(t)  # the already-prepared daily batch — reported as-is
            continue
        status = t.get("status", "open")
        if status.startswith("done"):
            done.append(t)
        elif status.startswith("blocked"):
            blocked.append(t)
        elif status.startswith("claimed"):
            # TAKEN — must never re-enter a daily batch (measured 2026-10-18:
            # claimed rows fell through to ready and T-022 took the big slot)
            claimed.append(t)
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
        "daily_batch": {"law": "§BATCH prepared briefs (reported as-is, not re-ranked)",
                        "tasks": [{"id": t["id"], "title": t["title"],
                                   "size": t.get("size", ""),
                                   "status": t.get("status", "")} for t in batch]},
        "codex_queue": {"law": "moderate-to-small only (owner correction)",
                        "tasks": codex},
        "glm_waitlist": glm_wait,
        "manual_fire": manual,
        "held_for_deps": [{"id": t["id"], "title": t["title"],
                           "deps": t.get("deps", "")} for t in held],
        "owner_gated": [{"id": t["id"], "title": t["title"]} for t in gated],
        "blocked": [{"id": t["id"], "title": t["title"],
                     "status": t.get("status", "")} for t in blocked],
        "claimed": [{"id": t["id"], "title": t["title"],
                     "status": t.get("status", "")} for t in claimed],
        "done": [{"id": t["id"], "title": t["title"]} for t in done],
    }




def _batch_item(text, tid):
    """Locate a §BATCH list-item entry by exact task id (or None)."""
    return re.compile(
        r"^- \*\*" + re.escape(tid) + r" \[[A-Za-z-]+\] "
        r"\((?P<status>(?:[^()]|\([^()]*\))*)\) — (?P<title>.+?)\.\*\*",
        re.M).search(text)


def _splice_batch_status(text, m, new_blob):
    """Replace only the inline status blob inside the matched parens."""
    return text[:m.start("status")] + new_blob + text[m.end("status"):]


def mark_done(text, tid, proof, by, date):
    """Transform the ledger markdown: task status -> done, §BOOK one-liner.
    Pure text transform (no writes) — the caller publishes via ygg-memory.
    Understands both entry forms: §QUEUE `### [T-xxx]` headers and §BATCH
    `- **T-xxx [SIZE] (status) — Title.**` briefs."""
    m = re.search(r"^### \[" + re.escape(tid) + r"\] (.+)$", text, re.M)
    if not m:
        bm = _batch_item(text, tid)
        if bm is None:
            raise SystemExit(f"task {tid} not found in the ledger (neither "
                             "§QUEUE header nor §BATCH item)")
        if bm.group("status").startswith("done"):
            raise SystemExit(f"task {tid} is already done")
        text = _splice_batch_status(text, bm, f"done({proof}) [by {by}, {date}]")
        book_hdr = "## §BOOK — one-liners of finished work"
        if book_hdr in text:
            line = f"- {date} {tid} done({proof}) [by {by}] — {bm.group('title').strip()}"
            text = text.replace(book_hdr, book_hdr + "\n" + line, 1)
        return text
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


def _claim_refusal(status):
    """Fail-closed claim grammar (2026-10-22): `open` claims; explicit owned
    states refuse outright; UNRECOGNIZED mid-grind prose (T-026 'STOOD UP …',
    measured 2026-10-20 — the safe --claim path was blind to it and seats fell
    back to hand-edit+publish, the exact error surface this path exists to
    kill) refuses with the --claim-force takeover path. Returns None when the
    claim may proceed."""
    s = status.strip()
    if s.startswith("open"):
        if "gated" in s:
            return "task is owner-gated — only the owner opens it (never force)"
        return None
    if s.startswith("done("):
        return "task is already done — claims are meaningless on it"
    if s.startswith("gated"):
        return "task is owner-gated — only the owner opens it (never force)"
    owned = ("claimed(", "blocked(", "armed", "STOOD UP")
    if s.startswith(owned):
        if s.startswith("blocked(") or s.startswith("claimed("):
            return (f"task is {s.split('—')[0].strip()[:60]} — another seat "
                    "holds it; --claim-force documents a takeover")
        return (f"task is in an explicit non-open state ({s[:40]}…) — "
                "--claim-force documents a takeover")
    return ("task status is unrecognized mid-grind prose — refusing "
            "fail-closed; pass --claim-force for a documented takeover "
            "(or book via get→edit→publish)")


def mark_claimed(text, tid, by, date, note="", force=False):
    """Transform the ledger markdown: task status -> claimed(by, date).
    Pure text transform — the caller publishes via ygg-memory. The claim is
    ID-KEYED (same discipline as mark_done): the seat hands the task id and
    this matches the exact block — hand regex over the door text has the
    mis-target hazard measured 2026-10-17 (a claim edit skipped past a
    just-claimed T-002 and landed on T-003). Refuses when the task is no
    longer open in the text being claimed (grammar in _claim_refusal;
    force=True is the documented-takeover path, never valid on done/gated).
    Understands both entry forms: §QUEUE `### [T-xxx]` headers and §BATCH
    `- **T-xxx [SIZE] (status) — Title.**` briefs (the 2026-10-18 T-022 claim
    race happened because the §BATCH form was hand-edited)."""
    m = re.search(r"^### \[" + re.escape(tid) + r"\] (.+)$", text, re.M)
    if not m:
        bm = _batch_item(text, tid)
        if bm is None:
            raise SystemExit(f"task {tid} not found in the ledger (neither "
                             "§QUEUE header nor §BATCH item)")
        refusal = _claim_refusal(bm.group("status"))
        if refusal and (not force or refusal.startswith("task is already done")
                        or refusal.startswith("task is owner-gated")):
            raise SystemExit(f"task {tid}: {refusal}")
        suffix = f" — {note}" + (" [force-takeover]" if refusal else "")
        return _splice_batch_status(text, bm, f"claimed({by}, {date}){suffix}")
    nxt = re.search(r"^### \[T-", text[m.end():], re.M)
    block_end = m.end() + nxt.start() if nxt else len(text)
    block = text[m.end():block_end]
    sm = re.search(r"^- status: (.*)$", block, re.M)
    if not sm:
        raise SystemExit(f"task {tid} has no status line to claim")
    refusal = _claim_refusal(sm.group(1))
    if refusal and (not force or refusal.startswith("task is already done")
                    or refusal.startswith("task is owner-gated")):
        raise SystemExit(f"task {tid}: {refusal}")
    suffix = f" — {note}" + (" [force-takeover]" if refusal else "")
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

### [T-110] claimed elsewhere
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: claimed(zcode, 2026-10-17) — taken

### [T-111-C] suffix-id formal row
- size: moderate ; value: 4 ; est: x
- route: glm-idle ; deps: none
- status: open

### [T-105] held for deps
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: mail mechanics verified
- status: open

### [T-106] second moderate
- size: moderate ; value: 4 ; est: x
- route: any ; deps: none
- status: open

### [T-107] mid-grind prose status
- size: moderate ; value: 4 ; est: x
- route: glm-idle ; deps: none
- status: STOOD UP 2026-10-20 — policy door live; grind in flight

### [T-108] blocked with no owner gate
- size: small ; value: 2 ; est: x
- route: codex ; deps: none
- status: blocked(owner-scan-pending — corpus not uploaded yet)

### [T-109] done with stale trailing claim
- size: big ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: done(merged report) [by zcode, 2026-10-18]
- status: claimed(orch-x, 2026-10-22) — auto-claim residue

### ### [T-112] heading carries stray extra hashes
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: open

## §BOOK — one-liners of finished work
"""
    p = plan(parse_ledger(synthetic))
    ids = [t["id"] for t in p["glm_idle_batch"]["tasks"]]
    # T-111-C (suffix id, value 4) ties T-106 at 2.0 and sits earlier in
    # the file — stable sort keeps it in the moderate slot; T-112 (the
    # stray-hash heading, value 5) outranks it and takes the other slot
    assert ids == ["T-102", "T-103", "T-112"], ids
    # T-035 regressions (2026-10-23, measured on the seq #101999 door):
    # a done task with a stale trailing auto-claim stays done (done is
    # absorbing), and a `### ### [T-x]` heading parses as its OWN block
    # instead of being absorbed into the previous task's field dict
    # (which had resurrected T-021 as a big claimed pick).
    tasks = {t["id"]: t for t in parse_ledger(synthetic)}
    assert tasks["T-109"]["status"].startswith("done"), tasks["T-109"]["status"]
    assert tasks["T-112"]["title"].startswith("heading carries"), tasks["T-112"]
    assert all(t["id"] != "T-109" for t in p["glm_idle_batch"]["tasks"])
    assert all(t["id"] != "T-105" for t in p["glm_idle_batch"]["tasks"])
    assert p["owner_gated"] and p["owner_gated"][0]["id"] == "T-104"
    assert {t["id"] for t in p["claimed"]} == {"T-110"}, p["claimed"]
    assert all(t["id"] != "T-110" for t in p["glm_idle_batch"]["tasks"])
    # T-111-C lost its moderate slot to T-112 (higher score) — waitlist, not batch
    assert all(t["id"] != "T-111-C" for t in p["glm_idle_batch"]["tasks"])
    assert any(t["id"] == "T-111-C" for t in p["glm_waitlist"])
    assert p["held_for_deps"][0]["id"] == "T-105"
    # T-106 (route any) lost its glm moderate slot to T-111-C -> codex queue
    assert [x["id"] for x in p["codex_queue"]["tasks"]] == ["T-106", "T-101"]
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
    # fail-closed claim grammar (2026-10-22): mid-grind prose and owned
    # statuses refuse; --claim-force is the documented-takeover path and is
    # itself refused on done()/gated
    try:
        mark_claimed(synthetic, "T-107", "zcode", "2026-10-22")
        raise AssertionError("mid-grind prose claim should refuse fail-closed")
    except SystemExit:
        pass
    forced = mark_claimed(synthetic, "T-107", "zcode", "2026-10-22",
                          "documented takeover", force=True)
    assert ("- status: claimed(zcode, 2026-10-22) — documented takeover "
            "[force-takeover]" in forced)
    try:
        mark_claimed(synthetic, "T-108", "zcode", "2026-10-22")
        raise AssertionError("blocked() claim should refuse")
    except SystemExit:
        pass
    forced_block = mark_claimed(synthetic, "T-108", "zcode", "2026-10-22",
                                "deps re-verified", force=True)
    assert "claimed(zcode, 2026-10-22) — deps re-verified [force-takeover]" \
        in forced_block
    done_text = mark_done(synthetic, "T-101", "x", "zcode", "2026-10-17")
    for always_refused in (False, True):
        try:
            mark_claimed(done_text, "T-101", "zcode", "2026-10-22",
                         force=always_refused)
            raise AssertionError("done() claim must refuse even with force")
        except SystemExit:
            pass
    try:
        mark_claimed(synthetic, "T-104", "zcode", "2026-10-22", force=True)
        raise AssertionError("gated claim must refuse even with force")
    except SystemExit:
        pass
    # §BATCH form: claim/done on list-item briefs, refusal when not open
    batch = synthetic + """
## §BATCH — three ready-to-fire briefs

- **T-201 [BIG] (open) — batch big task.** Body here.
- **T-202 [MODERATE] (open) — batch moderate.** Body.
- **T-203 [MODERATE] (claimed(zcode, 2026-10-17) — in flight) — batch claimed.** Body.
"""
    pb = plan(parse_ledger(batch))
    assert [t["id"] for t in pb["daily_batch"]["tasks"]] == ["T-201", "T-202", "T-203"]
    assert all(t["id"] not in {x["id"] for x in pb["glm_idle_batch"]["tasks"]}
               for t in pb["daily_batch"]["tasks"])
    bc = mark_claimed(batch, "T-201", "zcode", "2026-10-18", "race-free now")
    assert "(claimed(zcode, 2026-10-18) — race-free now) — batch big task.**" in bc
    assert "(claimed(zcode, 2026-10-17) — in flight) — batch claimed.**" in bc  # nested blob untouched
    # nested-paren status must parse as ONE blob (a greedy [^)]* matched the
    # inner paren and mis-split status/title — regression caught 2026-10-18)
    bm203 = _batch_item(batch, "T-203")
    assert bm203.group("status") == "claimed(zcode, 2026-10-17) — in flight"
    assert bm203.group("title") == "batch claimed"
    for bad_batch, bad_id in ((bc, "T-201"), (bc, "T-203")):
        try:
            mark_claimed(bad_batch, bad_id, "zcode", "2026-10-18")
            raise AssertionError(f"batch claim {bad_id} should refuse")
        except SystemExit:
            pass
    bd = mark_done(batch, "T-202", "results.json", "zcode", "2026-10-18")
    assert "(done(results.json) [by zcode, 2026-10-18]) — batch moderate.**" in bd
    assert "- 2026-10-18 T-202 done(results.json) [by zcode] — batch moderate" in bd
    try:
        mark_done(bd, "T-202", "x", "zcode", "2026-10-18")
        raise AssertionError("batch done should refuse already-done")
    except SystemExit:
        pass
    try:
        mark_claimed(batch, "T-999", "zcode", "2026-10-18")
        raise AssertionError("unknown batch id should refuse")
    except SystemExit:
        pass
    print("selftest green (plan + mark_done + mark_claimed, §QUEUE + §BATCH forms)")
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
    ap.add_argument("--from-outbox", action="store_true",
                    help="guarded replay: do not recursively create an intent")
    ap.add_argument("--claim", metavar="TASK_ID",
                    help="book a claim (id-keyed, fail-closed on owned/"
                         "mid-grind statuses): rewrites the ledger door via "
                         "ygg-memory publish; the safe way to mark a task taken")
    ap.add_argument("--claim-force", action="store_true",
                    help="with --claim: documented-takeover path — overrides "
                         "claimed(/blocked(/mid-grind prose refusals (never "
                         "done(/gated); records [force-takeover] in the status")
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

    publication_receipt = None

    def publish_ledger(new_text, ok_line):
        """Publish via the ygg-memory runner. The PATH may carry a bash
        wrapper (ygg-memory, no .py — measured 2026-10-17: running it under
        sys.executable is a SyntaxError) — prefer the .py and exec wrappers
        directly. The ok_line prints ONLY on success: a booking that did not
        publish is a failure, never a confirmation."""
        nonlocal publication_receipt
        tmp = tempfile.NamedTemporaryFile("w", suffix=".md", delete=False,
                                          encoding="utf-8")
        tmp.write(new_text)
        tmp.close()
        try:
            runner = shutil.which("ygg-memory.py") or shutil.which(
                "ygg-memory") or os.path.expanduser("~/.local/bin/ygg-memory.py")
            prefix = [sys.executable] if runner.endswith(".py") else []
            r = subprocess.run(prefix + [runner, "publish", "--file", tmp.name,
                                         "--ns=_global", "--as",
                                         "fleet-task-ledger.md", "--json"],
                               capture_output=True, text=True, timeout=120)
        except Exception as e:
            print(f"ledger publish FAILED ({type(e).__name__}) — the booking did NOT land")
            return 1
        finally:
            os.unlink(tmp.name)
        print(r.stdout.strip() or r.stderr.strip())
        if r.returncode != 0:
            print("ledger publish FAILED — the booking did NOT land")
            return r.returncode or 1
        try:
            report = json.loads(r.stdout)
            record = report["record"]
            if (report.get("status") != "ok" or record.get("ns") != "_global"
                    or record.get("file") != "fleet-task-ledger.md"
                    or record.get("digest") != hashlib.sha256(new_text.encode()).hexdigest()
                    or not record.get("event_id")):
                raise ValueError("publication identity mismatch")
            publication_receipt = "ygg-memory:event:" + record["event_id"]
        except (ValueError, KeyError, TypeError):
            print("ledger publish UNCERTAIN — no matching positive publication receipt")
            return 1
        print(ok_line)
        return 0

    if a.claim:
        text = open(a.ledger, encoding="utf-8").read()
        new = mark_claimed(text, a.claim, a.by, a.day, a.note,
                           force=a.claim_force)
        if a.dry_run:
            print(new)
            return 0
        return publish_ledger(
            new, f"booked {a.claim} claimed({a.by}) — the task is taken; execution in flight")
    if a.done:
        if not a.proof:
            print("--done needs --proof \"<ref>\"")
            return 2
        intent = None
        # Retain BEFORE entering the contested publication path. Dry-run is pure.
        if not a.dry_run and not a.from_outbox:
            intent = write_intent(a.done, a.proof, a.by, ledger=a.ledger, day=a.day)
        try:
            text = open(a.ledger, encoding="utf-8").read()
            try:
                new = mark_done(text, a.done, a.proof, a.by, a.day)
            except SystemExit:
                # Crash after publish but before outbox acknowledgement: only
                # exact task/proof/by + BOOK readback can establish idempotence.
                matching = next((t for t in parse_ledger(text) if t["id"] == a.done), None)
                book = f"{a.done} done({a.proof}) [by {a.by}]"
                if not (a.from_outbox and matching and
                        matching.get("status", "").startswith(f"done({a.proof}) [by {a.by},")
                        and book in text.split("## §BOOK", 1)[-1]):
                    raise
                new = text
            if a.dry_run:
                print(new)
                return 0
            # Even replay of an already-booked subject uses the publisher;
            # an unsynchronized local door alone cannot mint a receipt.
            rc = publish_ledger(new, f"booked {a.done} done({a.proof}) [by {a.by}] — §BOOK line added")
        except (Exception, SystemExit) as e:
            print(f"booking FAILED ({type(e).__name__})")
            rc = 1
        if rc:
            if intent:
                print(f"booking RETAINED in the outbox: {intent['intent_id']}")
            else:
                print("booking replay pending — retained in the outbox")
            return rc
        receipt = publication_receipt
        print("BOOKING_RECEIPT " + json.dumps({"task": a.done, "proof": a.proof,
              "by": a.by, "receipt_ref": receipt}))
        if intent:
            update_intent(default_path(), intent["intent_id"], state="done",
                          receipt_ref=receipt, next_attempt_at=None)
        return 0
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
    if p["daily_batch"]["tasks"]:
        print("\n## daily batch (§BATCH prepared briefs, as-is — claim by id)")
        for t in p["daily_batch"]["tasks"]:
            st = t.get("status", "open")
            mark = "TAKEN" if not st.startswith("open") else "open"
            print(f"  {t['id']} [{t['size']}] ({mark}) {t['title']}"
                  + ("" if mark == "open" else f" — {st[:70]}"))
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
