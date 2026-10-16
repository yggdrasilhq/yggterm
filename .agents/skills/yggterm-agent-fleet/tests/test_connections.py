#!/usr/bin/env python3
"""[slice 5] connections: the whole-network observation adapters + Yin/Yang
minimal falsifiable slice (sol round-4 Q1).

    python3 tests/test_connections.py [path-to-ygg-memory.py]

⛔ THE SLICE FALSIFIER (Q1): one hub document, one board thread, and two
lobe records — repair one unambiguous broken connection; create one testable
Yang edge; replay ordinary retrieval and exploratory retrieval. FAIL if the
hypothesis appears as fact, a source is overwritten, an ambiguous link
resolves silently, or rerunning creates duplicates.
"""

import hashlib
import importlib.util
import json
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest_tree(root: Path) -> dict:
    return {
        str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(root.rglob("*")) if p.is_file()
    }


def test_hub_adapter_ambiguity_and_idempotence(module, failures):
    """Unambiguous [[link]] resolves to a derived edge; ambiguous and missing
    links stay UNRESOLVED (never guessed); a rerun creates no duplicates and
    rewrites no source."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-conn"
        ns_dir = module.get_namespace_dir(root, ns)
        ns_dir.mkdir(parents=True, exist_ok=True)
        (ns_dir / "alpha.md").write_text("see [[beta]] and [[ghost]] and [[dup]]\n", encoding="utf-8")
        (ns_dir / "beta.md").write_text("backlink [[alpha]]\n", encoding="utf-8")
        (ns_dir / "dup one.md").write_text("x\n", encoding="utf-8")
        (ns_dir / "dup two.md").write_text("x\n", encoding="utf-8")
        before = digest_tree(ns_dir)
        first = module.scan_hub_links(root, ns)
        if first["created"] != 2:  # alpha→beta and beta→alpha
            failures.append(f"hub: expected 2 unambiguous edges, got {first}")
        unresolved_links = {(u["from"], u["link"]) for u in first["unresolved"]}
        if (f"{ns}/alpha", "ghost") not in unresolved_links:
            failures.append("hub: an unresolved link was silently dropped or guessed")
        if (f"{ns}/alpha", "dup") not in unresolved_links:
            failures.append("hub: an AMBIGUOUS link (two matching doors) resolved silently")
        edges_after_first = len(module.load_connections(root)["edges"])
        second = module.scan_hub_links(root, ns)
        if len(module.load_connections(root)["edges"]) != edges_after_first:
            failures.append("hub: RERUN created duplicate edges")
        if second["created"] != 0:
            failures.append(f"hub: rerun was not idempotent: {second}")
        after = digest_tree(ns_dir)
        if before != after:
            failures.append("hub: the ADAPTER REWROTE SOURCE FILES (observation-only violation)")
        # the derived edge is the only new write, and it lives in the dream dir
        if not module.connections_path(root).is_file():
            failures.append("hub: no derived connection index written")


def test_board_adapter_replies_and_replay(module, failures):
    """A board reply becomes a replies-to edge between the two immutable
    posts; foreign refs are skipped; replaying the append-only board adds
    nothing."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        boards = Path(tmp) / "boards" / "infra" / "meta"
        boards.mkdir(parents=True)
        (boards / "board.jsonl").write_text(
            "\n".join([
                json.dumps({"ts": "t1", "board": "infra/meta", "kind": "question",
                            "ack_token": "ACK-q1", "refs": ["some-repo-path"], "body": "q"}),
                json.dumps({"ts": "t2", "board": "infra/meta", "kind": "note",
                            "ack_token": "ACK-a1", "refs": ["ACK-q1"], "body": "an answer"}),
                json.dumps({"ts": "t3", "board": "infra/meta", "kind": "note",
                            "ack_token": "ACK-a2", "refs": [], "body": "no refs"}),
            ]) + "\n",
            encoding="utf-8",
        )
        first = module.scan_board_references(root, boards.parent.parent, "infra/meta")
        if first["created"] != 1:
            failures.append(f"board: expected exactly 1 replies-to edge, got {first}")
        index = module.connections_index(root)
        edge = index["edges"][0] if index["edges"] else None
        if not edge or edge["relation"] != "replies-to" or not edge["to"].endswith("ACK-q1"):
            failures.append(f"board: edge shape wrong: {edge}")
        second = module.scan_board_references(root, boards.parent.parent, "infra/meta")
        if second["created"] != 0:
            failures.append(f"board: REPLAY duplicated edges: {second}")


def test_lobe_adapter_preserves_native_addresses(module, failures):
    """lobe:// references inside lobe records become edges with the native
    address preserved verbatim; two records referencing each other yield two
    edges; rerun dedupes."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        lobes = Path(tmp) / "lobes"
        (lobes / "fingraph").mkdir(parents=True)
        (lobes / "medgraph" / "plans").mkdir(parents=True)
        (lobes / "fingraph" / "theses.md").write_text(
            "depends on lobe://medgraph/plans/shared-labs.toml#labs\n", encoding="utf-8")
        (lobes / "medgraph" / "plans" / "index.md").write_text(
            "informed by lobe://fingraph/theses.md\n", encoding="utf-8")
        first = module.scan_lobe_references(root, lobes)
        if first["created"] != 2:
            failures.append(f"lobe: expected 2 cross-references, got {first}")
        index = module.connections_index(root)
        addresses = [(e["from"], e["to"]) for e in index["edges"]]
        if not any(to_addr == "lobe://medgraph/plans/shared-labs.toml#labs" for _, to_addr in addresses):
            failures.append(f"lobe: native address not preserved verbatim: {addresses}")
        second = module.scan_lobe_references(root, lobes)
        if second["created"] != 0:
            failures.append(f"lobe: rerun duplicated: {second}")


def test_yang_hypothesis_lifecycle(module, failures):
    """A hypothesis with a concrete test: invisible to ordinary retrieval,
    visible-and-labeled under exploration, never a fact; tested→supported
    joins ordinary views as supported; rejected is history; untested past
    review expires from ordinary views. 'Possibly related' fails admission."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        try:
            module.connection_observe(
                root, "a", "references", "b",
                status="hypothesis", hypothesis="maybe related", falsification_test=None,
            )
            failures.append("yang: a hypothesis WITHOUT a falsification test was admitted")
        except ValueError:
            pass
        proposed = module.connection_observe(
            root, "ns-a/door-x", "explains", "ns-b/door-y",
            status="hypothesis", created_by="test",
            hypothesis="door-x's trap causes door-y's recurring bug",
            falsification_test="replay door-x without the trap; if door-y's bug still reproduces, reject",
        )
        ordinary = module.connections_index(root)
        if any(e["id"] == proposed["id"] for e in ordinary["edges"]):
            failures.append("yang: an untested hypothesis appeared in ORDINARY retrieval as if fact")
        explore = module.connections_index(root, explore=True)
        row = next((e for e in explore["edges"] if e["id"] == proposed["id"]), None)
        if not row or row["status"] != "hypothesis" or not row.get("falsification_test"):
            failures.append(f"yang: exploration lost the hypothesis label or its test: {row}")
        # test it: supported joins ordinary retrieval AS supported
        module.connection_test(root, proposed["id"], "supported", evidence="replay-log-2026-10-16")
        ordinary = module.connections_index(root)
        row = next((e for e in ordinary["edges"] if e["id"] == proposed["id"]), None)
        if not row or row["status"] != "supported":
            failures.append(f"yang: a supported hypothesis did not surface as supported: {row}")
        # rejected edges are history
        rejected = module.connection_observe(
            root, "ns-a/m", "references", "ns-b/n", status="hypothesis",
            hypothesis="h2", falsification_test="t2",
        )
        module.connection_test(root, rejected["id"], "rejected")
        if any(e["id"] == rejected["id"] for e in module.connections_index(root, explore=True)["edges"]):
            failures.append("yang: a REJECTED edge still traverses")
        # untested hypothesis past its review window expires from ordinary views
        stale = module.connection_observe(
            root, "ns-a/p", "references", "ns-b/q", status="hypothesis",
            hypothesis="h3", falsification_test="t3", review_days=1,
        )
        data = module.load_connections(root)
        for edge in data["edges"]:
            if edge["id"] == stale["id"]:
                edge["review_by"] = "2026-10-01"  # rewind the clock
        module.save_connections(root, data)
        if any(e["id"] == stale["id"] for e in module.connections_index(root)["edges"]):
            failures.append("yang: an untested stale hypothesis still traverses ordinarily")
        row = next((e for e in module.connections_index(root, explore=True)["edges"] if e["id"] == stale["id"]), None)
        if not row or row["status"] != "expired":
            failures.append(f"yang: expiry not visible even under exploration: {row}")


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_hub_adapter_ambiguity_and_idempotence(module, failures)
    test_board_adapter_replies_and_replay(module, failures)
    test_lobe_adapter_preserves_native_addresses(module, failures)
    test_yang_hypothesis_lifecycle(module, failures)
    if failures:
        print(f"connections tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("connections tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
