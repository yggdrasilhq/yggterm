#!/usr/bin/env python3
"""composer_review truthfulness suite — OFFLINE hermetic controls.

Encodes the ygg4 repair acceptance (owner unit 2026-10-29): content-bound
batch identity, strict typed reviewer-output validation, immutable batch
snapshots, typed holds for malformed outbox / stale basis / storage
failures, serialized concurrent dispatch, and review debt that stays
distinct from mining success. Zero network, zero paid model calls — the
reviewer transport is a local fake binary.

Run:  python3 tools/swarmharness/test_composer_review.py
"""
import fcntl
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE / "composer_review.py"
TICK = HERE / "composer-tick.py"
PASS, FAIL = [], []
# the harness itself prints check lines with non-ASCII separators; it must
# survive the same C-locale hosts it tests for
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError, OSError):
        pass


def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    tag = "ok  " if cond else "FAIL"
    line = f"{tag} {name}"
    if detail:
        line += f" — {detail}"
    print(line)


def mkproposal(pattern_id):
    """A kernel-shaped claim proposal (composer.py propose_claims shape)."""
    return {"kind": "claim_proposal", "pattern_id": pattern_id,
            "detector_id": "sig-ssh-argv-join", "detector_version": "det-v3",
            "assertion": f"probe assertion for {pattern_id}",
            "disproof": "controls do not separate; incidents one lineage",
            "strata": ["board"], "lane": "demand", "evidence": [],
            "created_at": "2026-10-29T00:00:00Z", "status": "proposed"}


FAKE = r'''#!/usr/bin/env python3
"""Fake reviewer transport: derives the verdict from the STAGED BRIEF so
only a truthful brief can ever satisfy the contract. Mode via env."""
import json, os, re, sys, time
from pathlib import Path

mode = os.environ.get("COMPOSER_FAKE_MODE", "valid")
if os.environ.get("COMPOSER_FAKE_COUNT"):
    with open(os.environ["COMPOSER_FAKE_COUNT"], "a") as f:
        f.write("1\n")

brief = Path("brief.md").read_text(encoding="utf-8")
ids = re.findall(r"^- ([0-9a-f]{16}) pattern=", brief, re.M)
footer = re.search(r"^BATCH ([0-9a-f]{16}) reviewed=(\d+)", brief, re.M)
if not footer or not ids:
    print("fake transport: brief lacked the output contract", file=sys.stderr)
    sys.exit(1)
batch, reviewed = footer.group(1), int(footer.group(2))

if os.environ.get("COMPOSER_FAKE_SLEEP"):
    time.sleep(float(os.environ["COMPOSER_FAKE_SLEEP"]))

def verdicts(pids, skip=(), dup=False):
    lines = []
    for p in pids:
        if p in skip:
            continue
        lines.append(f"VERDICT {p} defer — single-observation proposal; "
                     f"needs a second independent lineage before triage")
    if dup and lines:
        lines.append(lines[-1])
    return lines

if mode == "valid":
    for l in verdicts(ids):
        print(l)
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)}")
elif mode == "valid-loose":
    # structural variants that must NOT fail: lowercase verdict keyword,
    # uppercase hex ids, plural footer keys, fences + commentary around it
    print("Reviewing the staged batch. My verdict lines follow.")
    print("```")
    for p in ids:
        print(f"verdict {p.upper()} defer -- single-observation proposal; "
              f"needs a second independent lineage before triage")
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebuts=0 "
          f"defers={len(ids)}")
    print("All proposals deferred pending a second lineage.")
    print("```")
elif mode == "valid-delay":
    time.sleep(float(os.environ.get("COMPOSER_FAKE_SLEEP", "1.2")))
    for l in verdicts(ids):
        print(l)
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)}")
elif mode == "stale":
    state = Path(os.environ["COMPOSER_STATE"])
    with (state / "outbox.jsonl").open("a") as f:
        f.write(json.dumps({"kind": "claim_proposal",
                            "pattern_id": "late-arrival",
                            "note": "arrived while review ran"}) + "\n")
    for l in verdicts(ids):
        print(l)
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)}")
elif mode == "wrong-batch":
    for l in verdicts(ids[:1]):
        print(l)
    print(f"BATCH {'0' * 16} reviewed=1 accepts=0 rebut=0 defers=1")
elif mode == "bare":
    print("BATCH")
elif mode == "impossible-counts":
    for l in verdicts(ids[:1]):
        print(l)
    print(f"BATCH {batch} reviewed=1 accepts=99 rebut=0 defers=0")
elif mode == "missing-one":
    for l in verdicts(ids, skip={ids[0]}):
        print(l)
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)-1}")
elif mode == "dup-id":
    for l in verdicts(ids, dup=True):
        print(l)
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)}")
elif mode == "unknown-id":
    print("VERDICT 0123456789abcdef defer — fabricated id not in this batch")
    for l in verdicts(ids):
        print(l)
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)}")
elif mode == "short-reason":
    for p in ids:
        print(f"VERDICT {p} defer - no")
    print(f"BATCH {batch} reviewed={len(ids)} accepts=0 rebut=0 defers={len(ids)}")
elif mode == "exit1":
    print("model unavailable: quota exhausted", file=sys.stderr)
    sys.exit(1)
elif mode == "sleep-forever":
    time.sleep(30)
    sys.exit(0)
else:
    print(f"fake transport: unknown mode {mode}", file=sys.stderr)
    sys.exit(1)
'''


class Sandbox:
    def __init__(self):
        self.root = Path(tempfile.mkdtemp(
            prefix=f"composer-review-{uuid.uuid4().hex[:6]}-"))
        self.home = self.root / "home"
        self.home.mkdir()
        self.state = self.root / "state"
        self.state.mkdir()
        self.count = self.root / "transport-count"

    def install_fake(self):
        binp = self.root / "bin"
        binp.mkdir(exist_ok=True)
        fake = binp / "fake-codex"
        fake.write_text(FAKE, encoding="utf-8")
        fake.chmod(0o755)
        return str(fake)

    def env(self, **over):
        e = dict(os.environ)
        e["HOME"] = str(self.home)
        e["COMPOSER_STATE"] = str(self.state)
        e["COMPOSER_REVIEW_SILENT"] = "1"
        e["PATH"] = "/usr/bin:/bin"
        e.update({k: str(v) for k, v in over.items()})
        return e

    def run_review(self, **over):
        e = self.env(**over)
        return subprocess.run([sys.executable, str(SRC)],
                              capture_output=True, encoding="utf-8", errors="replace", env=e,
                              cwd=str(self.root), timeout=60)

    def outbox_write(self, rows_or_text):
        p = self.state / "outbox.jsonl"
        if isinstance(rows_or_text, str):
            p.write_text(rows_or_text)
        else:
            p.write_text("".join(json.dumps(r) + "\n"
                                 for r in rows_or_text))

    def marker(self):
        p = self.state / "dispatch.json"
        return json.loads(p.read_text()) if p.exists() else None

    def holds(self):
        p = self.state / "review-holds.jsonl"
        if not p.exists():
            return []
        return [json.loads(l) for l in p.read_text().splitlines() if l.strip()]

    def cleanup(self):
        for d in (self.root,):
            for sub in d.rglob("*"):
                try:
                    os.chmod(sub, 0o755)
                except OSError:
                    pass
            try:
                os.chmod(d, 0o755)
            except OSError:
                pass
        shutil.rmtree(self.root, ignore_errors=True)


def main():
    import composer_review as cr  # unit level

    # ---- U: identity is content-bound, never '?' ----
    a, b = mkproposal("fault-one"), mkproposal("fault-two")
    check("U1 proposal ids are 16-hex over canonical bytes, never '?'",
          re.fullmatch(r"[0-9a-f]{16}", cr.proposal_identity(a)) is not None)
    check("U2 same-count different content -> different ids AND batch",
          cr.proposal_identity(a) != cr.proposal_identity(b)
          and cr.batch_identity([cr.proposal_identity(a)])
          != cr.batch_identity([cr.proposal_identity(b)]))
    check("U3 identical content re-serialized differently -> same id",
          cr.proposal_identity(a)
          == cr.proposal_identity(json.loads(json.dumps(a))))
    a2 = dict(a, evidence=[{"line": 3, "src": "invented.log"}])
    check("U4 field-level change (unknown/extra field) -> new id",
          cr.proposal_identity(a) != cr.proposal_identity(a2))
    check("U5 batch covers protocol version",
          cr.batch_identity([cr.proposal_identity(a)]) != hashlib_batch(
              cr.proposal_identity(a)))

    # ---- R1 happy path: full valid verdict -> advisory-ok marker ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        m = sb.marker()
        check("R1 valid complete verdict -> advisory-ok marker + typed out",
              r.returncode == 0 and m is not None
              and m.get("status") == "advisory-ok"
              and json.loads(r.stdout.strip().splitlines()[-1]).get("review")
              == "advisory-ok", r.stdout[-200:] + r.stderr[-200:])
        check("R1b marker binds batch/snapshot/ids/protocol",
              m.get("protocol") == "composer-review-v2"
              and re.fullmatch(r"[0-9a-f]{16}", m.get("batch", "")) is not None
              and len(m.get("proposal_ids", [])) == 1
              and re.fullmatch(r"[0-9a-f]{64}", m.get("snapshot_sha256", ""))
              is not None)
        check("R1c snapshot + brief + verdict records staged immutable dir",
              (sb.state / "reviews" / f"batch-{m['batch']}"
               / "outbox-snapshot.jsonl").exists()
              and (sb.state / "reviews" / f"batch-{m['batch']}"
                   / "verdict.json").exists()
              and (sb.state / "reviews" / f"batch-{m['batch']}"
                   / "brief.md").exists())

        # ---- R2 idempotent replay: unchanged batch -> no second dispatch ----
        r2 = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        check("R2 unchanged replay is idempotent (no re-dispatch)",
              r2.returncode == 0 and "already-reviewed" in r2.stdout)

        # ---- R3 whitespace-reformatted outbox still same CONTENT batch ----
        sb.outbox_write("  " + json.dumps(a) + "  \n\n")
        r3 = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        check("R3 reformatted-but-identical content stays idempotent",
              r3.returncode == 0 and "already-reviewed" in r3.stdout)

        # ---- R4 same-count changed content -> NEW batch, re-review ----
        sb.outbox_write([mkproposal("entirely-different-fault")])
        r4 = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        m4 = sb.marker()
        check("R4 same-count changed proposal is a NEW reviewed batch",
              r4.returncode == 0 and m4 is not None
              and m4["batch"] != m["batch"]
              and m4["proposal_ids"] != m["proposal_ids"])
        n_dirs = len(list((sb.state / "reviews").iterdir()))
        check("R4b old batch record preserved alongside new",
              n_dirs >= 2)

        # ---- R20 marker semantics: advisory, never a completion receipt ----
        check("R20 marker is advisory triage (no verdict_ok, carries note)",
              "verdict_ok" not in m4
              and "NOT" in m4.get("advisory_note", "")
              and m4.get("advisory_note", "").startswith("brain-pair"))
    finally:
        sb.cleanup()

    # ---- R5-R10: every fake/bare/partial verdict shape is REJECTED ----
    reject_cases = [
        ("R5", "wrong-batch", "fabricated batch id rejected"),
        ("R6", "bare", "bare BATCH substring rejected"),
        ("R7", "impossible-counts", "impossible counts rejected"),
        ("R8", "missing-one", "missing verdict line rejected"),
        ("R9", "dup-id", "duplicate verdict id rejected"),
        ("R10", "unknown-id", "unknown proposal id rejected"),
        ("R10b", "short-reason", "empty/too-short reason rejected"),
    ]
    for tag, mode, claim in reject_cases:
        sb = Sandbox()
        try:
            fake = sb.install_fake()
            sb.outbox_write([a])
            r = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                              COMPOSER_FAKE_MODE=mode)
            m = sb.marker()
            holds = sb.holds()
            check(f"{tag} {claim}: rc=2, NO marker, typed hold recorded",
                  r.returncode == 2 and m is None and len(holds) >= 1
                  and holds[-1]["status"].startswith(("review-rejected",
                                                      "review-failed",
                                                      "hold")),
                  r.stdout.strip()[:160])
        finally:
            sb.cleanup()

    # ---- R11 transport exit != 0 ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="exit1")
        check("R11 nonzero transport exit never advances a marker",
              r.returncode == 2 and sb.marker() is None
              and "review-failed" in r.stdout
              and sb.holds()[-1]["status"] == "review-failed:transport-exit",
              r.stdout.strip()[:160])
    finally:
        sb.cleanup()

    # ---- R12 timeout ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                          COMPOSER_FAKE_MODE="sleep-forever",
                          COMPOSER_REVIEW_TIMEOUT_S="1")
        check("R12 transport timeout typed, marker untouched",
              r.returncode == 2 and sb.marker() is None
              and "review-failed:timeout" in r.stdout,
              r.stdout.strip()[:160])
    finally:
        sb.cleanup()

    # ---- R13 transport absent ----
    sb = Sandbox()
    try:
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=str(sb.root / "absent"),
                          COMPOSER_FAKE_MODE="valid")
        check("R13 absent transport typed unavailable, marker untouched",
              r.returncode == 2 and sb.marker() is None
              and "review-unavailable:transport-absent" in r.stdout)
    finally:
        sb.cleanup()

    # ---- R14 malformed outbox: typed hold, PRIOR marker preserved ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r1 = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                           COMPOSER_FAKE_MODE="valid")
        good_marker = sb.marker()
        sb.outbox_write(json.dumps(a) + "\n{broken partial json\n"
                        + json.dumps(mkproposal("x2")) + "\n")
        r2 = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                           COMPOSER_FAKE_MODE="valid")
        m2 = sb.marker()
        check("R14 malformed outbox -> typed hold rc=2, no crash",
              r2.returncode == 2 and "hold:outbox-malformed" in r2.stdout
              and "JSONDecodeError" not in r2.stderr)
        check("R14b prior valid marker NOT clobbered by the hold",
              m2 == good_marker and m2["status"] == "advisory-ok")
    finally:
        sb.cleanup()

    # ---- R15 concurrent dispatch serializes ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        env1 = sb.env(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid-delay",
                      COMPOSER_FAKE_SLEEP="1.2",
                      COMPOSER_FAKE_COUNT=str(sb.count))
        p1 = subprocess.Popen([sys.executable, str(SRC)], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, encoding="utf-8", errors="replace", env=env1)
        time.sleep(0.15)
        p2 = subprocess.Popen([sys.executable, str(SRC)], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, encoding="utf-8", errors="replace", env=env1)
        o1, _ = p1.communicate(timeout=30)
        o2, _ = p2.communicate(timeout=30)
        n_calls = len(sb.count.read_text().splitlines()) if sb.count.exists() else 0
        rcs = sorted([p1.returncode, p2.returncode])
        busy = ("busy" in o2) or ("busy" in o1)
        check("R15 concurrent dispatch: exactly ONE transport invocation",
              n_calls == 1, f"calls={n_calls}")
        check("R15b loser reports typed busy with distinct rc=3 (not 0/2)",
              busy and rcs == [0, 3] and sb.marker() is not None)
    finally:
        sb.cleanup()

    # ---- R16 stale basis: outbox mutated DURING review -> verdict refused ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="stale")
        holds = sb.holds()
        check("R16 outbox changed mid-review -> stale basis, verdict refused",
              r.returncode == 2 and sb.marker() is None
              and holds[-1]["status"] == "stale-basis"
              and (sb.state / "reviews").exists(),
              r.stdout.strip()[:160])
    finally:
        sb.cleanup()

    # ---- R17 snapshot immutability: conflicting batch record is a hold ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        m = sb.marker()
        snap = sb.state / "reviews" / f"batch-{m['batch']}" / \
            "outbox-snapshot.jsonl"
        snap.write_text("TAMPERED\n")
        marker_path = sb.state / "dispatch.json"
        marker_path.unlink()
        r2 = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        check("R17 tampered snapshot -> conflict hold, bytes NOT overwritten",
              r2.returncode == 2
              and "hold:snapshot-conflict" in r2.stdout
              and snap.read_text() == "TAMPERED\n")
    finally:
        sb.cleanup()

    # ---- R18 storage failure: unwritable reviews dir ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        reviews = sb.state / "reviews"
        reviews.mkdir()
        os.chmod(reviews, 0o555)
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        check("R18 storage failure typed, no marker, no false success",
              r.returncode == 2 and sb.marker() is None
              and "hold:storage-failure" in r.stdout,
              r.stdout.strip()[:160])
    finally:
        sb.cleanup()

    # ---- R19 cooldown after repeated failure, debt retained ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r1 = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                           COMPOSER_FAKE_MODE="exit1",
                           COMPOSER_REVIEW_MAX_FAILURES="1",
                           COMPOSER_REVIEW_COOLDOWN_S="99999")
        r2 = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                           COMPOSER_FAKE_MODE="valid",
                           COMPOSER_REVIEW_MAX_FAILURES="1",
                           COMPOSER_REVIEW_COOLDOWN_S="99999")
        check("R19 repeated failure -> cooldown backoff (debt visible)",
              r1.returncode == 2 and r2.returncode == 2
              and "hold:cooldown-backoff" in r2.stdout
              and "debt" in r2.stdout
              and "consecutive_failures" in
              (sb.state / "review-attempts.json").read_text())
    finally:
        sb.cleanup()

    # ---- R21 tick propagation: review debt distinct from mining success ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        kernel = sb.home / ".yggterm/swarm/composer/kernel"
        kernel.mkdir(parents=True)
        (kernel / "composer.py").write_text(
            "def tick(sources, state_dir=None, **kw):\n"
            "    return {'census_sources': 1, 'gaps': 0, 'pages_read': 0,\n"
            "            'observations': 1, 'patterns': 1, 'proposals': 1,\n"
            "            'scanned_lineages': 1, 'enrolled_lineages': 1,\n"
            "            'backlog_remaining': 0, 'notes': []}\n")
        cfg = sb.home / ".yggterm/config/swarm"
        cfg.mkdir(parents=True)
        (cfg / "composer-sources.json").write_text(
            json.dumps({"sources": {}, "bounds": {}}))
        sb.outbox_write([a])
        e = sb.env(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="exit1")
        r = subprocess.run([sys.executable, str(TICK)], capture_output=True,
                           encoding="utf-8", errors="replace", env=e, timeout=60)
        out = r.stdout
        check("R21 mining tick still SUCCEEDS (rc=0) when review fails",
              r.returncode == 0 and '"proposals": 1' in out)
        check("R21b review debt surfaced, never 'skipped' wording",
              "review-debt" in out and "review-step skipped" not in out,
              out[-300:])
        check("R21c tick log records the mining report (no false merge)",
              (sb.home / ".yggterm/log/composer-tick.jsonl").exists()
              and "review" not in json.loads(
                  (sb.home / ".yggterm/log/composer-tick.jsonl")
                  .read_text().splitlines()[0]))
    finally:
        sb.cleanup()

    # ---- R22 noop paths ----
    sb = Sandbox()
    try:
        r = sb.run_review()
        check("R22 absent outbox -> typed noop rc=0",
              r.returncode == 0 and "outbox-absent" in r.stdout)
        sb.outbox_write("")
        r = sb.run_review()
        check("R22b empty outbox -> typed noop rc=0",
              r.returncode == 0 and "outbox-empty" in r.stdout)
    finally:
        sb.cleanup()

    # ---- R23 tick: composer_review.main() EXCEPTION still surfaces debt ----
    sb = Sandbox()
    try:
        kernel = sb.home / ".yggterm/swarm/composer/kernel"
        kernel.mkdir(parents=True)
        (kernel / "composer.py").write_text(
            "def tick(sources, state_dir=None, **kw):\n"
            "    return {'census_sources': 1, 'gaps': 0, 'pages_read': 0,\n"
            "            'observations': 1, 'patterns': 1, 'proposals': 1,\n"
            "            'scanned_lineages': 1, 'enrolled_lineages': 1,\n"
            "            'backlog_remaining': 0, 'notes': []}\n")
        # a broken review module that RAISES from main() — kernel dir is
        # first on tick's sys.path, so it shadows the real module
        (kernel / "composer_review.py").write_text(
            "def main():\n"
            "    raise RuntimeError('broken review module')\n")
        cfg = sb.home / ".yggterm/config/swarm"
        cfg.mkdir(parents=True)
        (cfg / "composer-sources.json").write_text(
            json.dumps({"sources": {}, "bounds": {}}))
        sb.outbox_write([a])
        e = sb.env()
        r = subprocess.run([sys.executable, str(TICK)], capture_output=True,
                           encoding="utf-8", errors="replace", env=e, timeout=60)
        check("R23 tick rc=0 on review-module exception (mining unaffected)",
              r.returncode == 0 and '"proposals": 1' in r.stdout)
        check("R23b exception debt VISIBLE as review-debt (never silent)",
              "review-debt" in r.stdout and "broken review module" in r.stdout,
              r.stdout[-300:])
    finally:
        sb.cleanup()

    # ---- R24 held lock from another process -> distinct busy rc=3 ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        holder = open(sb.state / "review.lock", "a+")
        fcntl.flock(holder, fcntl.LOCK_EX)
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        fcntl.flock(holder, fcntl.LOCK_UN)
        holder.close()
        check("R24 lock contention -> rc=3 busy observable, no dispatch",
              r.returncode == 3 and "busy" in r.stdout
              and sb.marker() is None and not sb.count.exists(),
              r.stdout.strip()[:160])
    finally:
        sb.cleanup()

    # ---- R25 non-ASCII proposal under a C locale ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        uni = dict(mkproposal("unicode-fault"),
                   assertion="café probe — naïve assertion (中文)",
                   disproof="série de contrôles ne sépare pas; incidents")
        sb.outbox_write([uni])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid",
                          LC_ALL="C", LANG="C", PYTHONCOERCECLOCALE="0",
                          PYTHONUTF8="0")
        m = sb.marker()
        snap = None
        if m:
            snap = (sb.state / "reviews" / f"batch-{m['batch']}"
                    / "outbox-snapshot.jsonl").read_bytes()
        check("R25 non-ASCII proposal survives C locale (explicit utf-8)",
              r.returncode == 0 and m is not None
              and m.get("status") == "advisory-ok"
              and snap is not None and "café".encode("utf-8") in snap,
              r.stdout[-200:] + r.stderr[-200:])
    finally:
        sb.cleanup()

    # ---- R26 loose-but-bound verdict structure is ACCEPTED ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a])
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake,
                          COMPOSER_FAKE_MODE="valid-loose")
        m = sb.marker()
        check("R26 lowercase/uppercase/plural/fenced verdict accepted",
              r.returncode == 0 and m is not None
              and m.get("status") == "advisory-ok"
              and m.get("counts", {}).get("defers") == 1,
              r.stdout[-200:] + r.stderr[-200:])
    finally:
        sb.cleanup()

    # ---- R27 NaN row in outbox -> typed hold, never a crash ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write(json.dumps(a) + "\n"
                        + '{"kind": "claim_proposal", "weight": NaN}\n')
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        holds = sb.holds()
        check("R27 NaN/Infinity row -> typed non-canonical hold, no crash",
              r.returncode == 2 and "hold:outbox-malformed" in r.stdout
              and "non-canonical-row" in r.stdout
              and "ValueError" not in r.stderr and sb.marker() is None,
              r.stdout.strip()[:200] + r.stderr[-120:])
    finally:
        sb.cleanup()

    # ---- R28 marker binds the EXACT parsed bytes (single-read basis) ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write([a, mkproposal("second-fault")])
        data = (sb.state / "outbox.jsonl").read_bytes()
        expect = hashlib.sha256(data).hexdigest()
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        m = sb.marker()
        check("R28 marker outbox_sha256 == digest of the parsed bytes",
              r.returncode == 0 and m is not None
              and m.get("outbox_sha256") == expect
              and len(m.get("proposal_ids", [])) == 2)
    finally:
        sb.cleanup()

    # ---- R29 lone-surrogate row -> typed hold at the canonical check ----
    sb = Sandbox()
    try:
        fake = sb.install_fake()
        sb.outbox_write(json.dumps(a) + "\n"
                        + '{"kind": "claim_proposal", "note": "\\ud800"}\n')
        r = sb.run_review(COMPOSER_REVIEW_BIN=fake, COMPOSER_FAKE_MODE="valid")
        holds = sb.holds()
        check("R29 lone Unicode surrogate -> typed non-canonical hold, "
              "identity never crashes",
              r.returncode == 2 and "hold:outbox-malformed" in r.stdout
              and "non-canonical-row" in r.stdout
              and "UnicodeEncodeError" not in r.stderr and sb.marker() is None,
              r.stdout.strip()[:200] + r.stderr[-120:])
    finally:
        sb.cleanup()

    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  ")
        return 1
    return 0


def hashlib_batch(pid):
    """Test-side batch formula WITHOUT the protocol term — proves U5 that
    the real batch id differs from a protocol-less hash."""
    import hashlib
    return hashlib.sha256(("\n" + pid).encode()).hexdigest()[:16]


if __name__ == "__main__":
    sys.exit(main())
