#!/usr/bin/env python3
"""[slice 4] corrections on EVERY read path: a dismissed claim never returns
as an active assertion — not through the direct door read, search, NOW, the
temporal envelope, the claims verb, or the composer's brief — and the
review surface shows what a correction touches before it lands.

    python3 tests/test_owner_read_paths.py [path-to-ygg-memory.py]

⛔ THE HOLE THIS PINS (round-3 gap review Q-D gap 3, slice-4 falsifier):
"dismiss a claim, then encounter its paraphrase, duplicate and archived form
through every supported read path — it must not return as an active
assertion." The founding slices proved the NOW path only; line filtering on
one path is not resistance. Also pinned: the three correction kinds never
conflate (false=corrected, stop-surfacing=dismissed, circumstances=surfaced),
and the owner gets a review surface answering what is remembered, from
which evidence, and what the correction will affect.
"""

import contextlib
import importlib.util
import io
import json
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"

CLAIM_TEXT = "the fleet ledger lives on dev under flock"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Args:
    def __init__(self, **kw):
        self.subcommand = "owner"
        self.root = None
        self.harness = "zcode"
        self.ns = None
        self.json = False
        self.correct = False
        self.dismiss = False
        self.surface = False
        self.target = ""
        self.text = ""
        self.scope = "_global"
        self.list = False
        self.affect = None
        self.query = ""
        self.id = None
        self.status = "all"
        self.dismissed = False
        self.limit = 20
        self.grep = None
        self.lines = None
        self.all = False
        self.ignore_case = False
        self.max_lines = 50
        self.pattern = ""
        self.file = None
        self.__dict__.update(kw)


def capture(module, fn, *a, **kw) -> str:
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        fn(*a, **kw)
    return buf.getvalue()


def build_world(module, root: Path, ns: str) -> None:
    """A door carrying the claim, a dreamer NOW carrying it, a delta carrying
    it, and two stored claims: c1 (the original) + c2 (a verbatim duplicate —
    the 'copied bullet' form a composer loves to mint)."""
    ns_dir = module.get_namespace_dir(root, ns)
    ns_dir.mkdir(parents=True, exist_ok=True)
    (ns_dir / "door-a.md").write_text(
        f"# door a\n\n- {CLAIM_TEXT} [as-of: 2026-10-16; proof: suite]\n- unrelated line\n",
        encoding="utf-8",
    )
    (ns_dir / "NOW.md").write_text(
        f"## §CURRENT_STATUS\n- {CLAIM_TEXT} [as-of: 2026-10-16; proof: suite]\n",
        encoding="utf-8",
    )
    module.append_journal_entry(root, ns, "door-a.md", "door", "update", "zcode", "test delta")
    module.save_claim_state(root, ns, {
        "ns": ns,
        "claims": {
            "c1": {"text": CLAIM_TEXT, "as_of": "2026-10-16", "proof": "suite",
                   "last_seen": "2026-10-16", "source_refs": ["seq#1/door-a.md"]},
            "c2": {"text": f"copy: {CLAIM_TEXT}", "as_of": "2026-10-16", "proof": "claimed",
                   "last_seen": "2026-10-16", "source_refs": ["seq#1/door-a.md"]},
        },
    })


def test_dismissal_dies_on_every_read_path(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-corr"
        build_world(module, root, ns)
        module.owner_correct(root, "dismiss", CLAIM_TEXT)
        # 1. the direct door read
        out = capture(module, module.cmd_get, Args(root=str(root), ns=ns, file="door-a.md"))
        if CLAIM_TEXT in out or "unrelated line" not in out:
            failures.append(f"get: dismissal leaked (or nuked the door): {out!r}")
        # 2. search
        out = capture(module, module.cmd_search, Args(subcommand="search", root=str(root), pattern="ledger lives", all=True, ignore_case=False, max_lines=50))
        if CLAIM_TEXT in out:
            failures.append(f"search: a dismissed line came back as a hit: {out!r}")
        # 3. NOW + the temporal envelope
        out = capture(module, module.cmd_now, Args(subcommand="now", root=str(root), ns=ns))
        if CLAIM_TEXT in out:
            failures.append("now: the dismissed claim resurfaced in the rollup read")
        envelope = module.temporal_envelope(root, ns)
        if CLAIM_TEXT in envelope or "2 tracked" in envelope:
            failures.append(f"envelope: the dismissed claim still counts as live: {envelope!r}")
        # 4. the claims verb: default excludes; --dismissed qualifies
        rows = json.loads(capture(module, module.cmd_claims, Args(subcommand="claims", root=str(root), ns=ns)))
        if any(row["id"] in ("c1", "c2") for row in rows):
            failures.append(f"claims: dismissed claims in the default read: {rows}")
        rows = json.loads(capture(module, module.cmd_claims, Args(subcommand="claims", root=str(root), ns=ns, dismissed=True)))
        if sorted(r["id"] for r in rows) != ["c1", "c2"] or not all(r.get("owner_dismissed") for r in rows):
            failures.append(f"claims: the --dismissed review view is wrong: {rows}")
        # 5. the composer's brief: deltas + rollup + retained claims
        staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        if staged is None:
            failures.append("brief: prepare staged nothing")
            return
        brief_text, manifest = staged[0].read_text(encoding="utf-8"), staged[1]
        if CLAIM_TEXT in brief_text:
            failures.append("brief: a dismissed claim entered the composer's inputs")
        if manifest.get("retained_dismissed") != 2:
            failures.append(f"brief: the manifest lost the dismissal count: {manifest.get('retained_dismissed')}")


def test_correction_kinds_never_conflate(module, failures):
    """'false' is corrected (shown WITH the authoritative replacement),
    'stop surfacing' is dismissed (gone), 'surface under these
    circumstances' is appended as instruction — three different effects."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-corr"
        build_world(module, root, ns)
        module.owner_correct(root, "correct", "copy:", "the duplicate is TRUE but must cite the ledger door")
        rows = json.loads(capture(module, module.cmd_claims, Args(subcommand="claims", root=str(root), ns=ns)))
        c2 = next((r for r in rows if r["id"] == "c2"), None)
        if c2 is None:
            failures.append(f"correct: a CORRECTED claim vanished (should show qualified): {rows}")
        elif "cite the ledger door" not in (c2.get("owner_correction") or ""):
            failures.append(f"correct: the authoritative replacement text is missing: {c2}")
        c1 = next((r for r in rows if r["id"] == "c1"), None)
        if c1 is None or c1.get("owner_correction") is not None:
            failures.append(f"correct: the correction leaked onto an untargeted claim: {rows}")
        # surfacing appends instructions on the NOW read
        module.owner_correct(root, "surface", "", "prefer PTY proof over status JSON when declaring row fixes")
        out = capture(module, module.cmd_now, Args(subcommand="now", root=str(root), ns=ns))
        if "owner surfacing instructions" not in out or "PTY proof" not in out:
            failures.append("surface: the surfacing instruction did not ride the NOW read")


def test_affect_review_surface(module, failures):
    """The review surface answers: what is remembered, from which evidence,
    what will this correction affect."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-corr"
        build_world(module, root, ns)
        affect = module.owner_affect(root, CLAIM_TEXT)
        claim_ids = {c["id"] for c in affect["claims"]}
        if claim_ids != {"c1", "c2"}:
            failures.append(f"affect: expected both claims (original + verbatim duplicate), got {claim_ids}")
        evidence = [c for c in affect["claims"] if c["id"] == "c1"]
        if not evidence or not evidence[0]["source_refs"]:
            failures.append(f"affect: claim evidence missing from the review surface: {affect['claims']}")
        doors = [d for d in affect["doors"] if d["file"] == "door-a.md"]
        if not doors or doors[0]["lines"] < 1:
            failures.append(f"affect: the door hits are missing: {affect['doors']}")


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_dismissal_dies_on_every_read_path(module, failures)
    test_correction_kinds_never_conflate(module, failures)
    test_affect_review_surface(module, failures)
    if failures:
        print(f"owner-read-path tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("owner-read-path tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
