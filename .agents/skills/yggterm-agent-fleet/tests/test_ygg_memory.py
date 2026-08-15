#!/usr/bin/env python3
"""Unit tests for ygg-memory.py unified cross-harness memory system.

Tests status, diff, get, ack (all and selective), publish, and sync-harness
in an isolated scratch directory without touching live user memory.
"""

import importlib.util
import json
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
YGG_MEMORY_SCRIPT = HERE.parent / "ygg-memory.py"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def load_module(script_path):
    spec = importlib.util.spec_from_file_location("ygg_memory", str(script_path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run_tests():
    mod = load_module(YGG_MEMORY_SCRIPT)
    tmp_root = Path(tempfile.mkdtemp(prefix="ygg-mem-test-"))

    try:
        ns = "-test-workspace-sample"
        root = tmp_root / "memory"

        # 1. Initial status on empty store
        wm = mod.load_watermark(root, "grok")
        check("watermark starts at seq 0", wm.get("last_seq") == 0)

        # 2. Publish a memory door from Claude
        dummy_door = tmp_root / "finding-pty-grid-ssot.md"
        dummy_door.write_text("""---
name: finding-pty-grid-ssot
description: "PTY grid SSOT divergence on Wayland resize — daemon holds grid dimensions."
metadata:
  type: finding
---

# Finding: PTY Grid SSOT
Daemon holds dimensions.
""", encoding="utf-8")

        class ArgsPublish:
            pass

        args_pub = ArgsPublish()
        args_pub.root = str(root)
        args_pub.harness = "claude"
        args_pub.ns = ns
        args_pub.file = str(dummy_door)
        args_pub.kind = None
        args_pub.summary = None
        args_pub.json = True

        mod.cmd_publish(args_pub)

        latest_seq = mod.get_latest_seq(root)
        check("publish advanced latest_seq to 1", latest_seq == 1)

        # Verify door published in namespace
        ns_dir = mod.get_namespace_dir(root, ns)
        published_door = ns_dir / "finding-pty-grid-ssot.md"
        check("published door file exists in namespace", published_door.exists())

        # Verify MEMORY.md steering header
        mem_index = ns_dir / "MEMORY.md"
        check("MEMORY.md was created", mem_index.exists())
        check("MEMORY.md contains steering header", "UNIFIED FLEET MEMORY" in mem_index.read_text(encoding="utf-8"))

        # 3. Status check for Grok (should be behind by 1)
        entries = mod.read_journal_entries(root, after_seq=0, namespace=ns)
        check("journal contains 1 entry", len(entries) == 1)
        check("journal summary extracted correctly", "PTY grid SSOT divergence" in entries[0]["summary"])

        # 4. Publish a second door (Campaign ledger)
        dummy_camp = tmp_root / "campaign-6.0-orchestrator.md"
        dummy_camp.write_text("""---
name: campaign-6.0-orchestrator
description: "Seat 6.0 orchestrator live cluster state & dead SHA sweep."
metadata:
  type: campaign
---

# The 6.0 Campaign
Handover log here.
""", encoding="utf-8")

        args_pub.file = str(dummy_camp)
        mod.cmd_publish(args_pub)

        check("latest_seq is now 2", mod.get_latest_seq(root) == 2)

        # 5. Selective Ack for Grok (ingest only campaign)
        class ArgsAck:
            pass

        args_ack = ArgsAck()
        args_ack.root = str(root)
        args_ack.harness = "grok"
        args_ack.ns = ns
        args_ack.all = False
        args_ack.files = "campaign-6.0-orchestrator.md"
        args_ack.json = True

        mod.cmd_ack(args_ack)

        grok_wm = mod.load_watermark(root, "grok")
        check("selective ack recorded campaign hash", "campaign-6.0-orchestrator.md" in grok_wm.get("namespaces", {}).get(ns, {}))

        # 6. Global Ack for Grok
        args_ack.all = True
        args_ack.files = None
        mod.cmd_ack(args_ack)

        grok_wm = mod.load_watermark(root, "grok")
        check("global ack brought grok last_seq to 2", grok_wm.get("last_seq") == 2)

        # 7. Bidirectional Sync Harness
        harness_dir = tmp_root / "gemini_memory"
        harness_dir.mkdir(parents=True, exist_ok=True)
        local_finding = harness_dir / "feedback-agent-first.md"
        local_finding.write_text("""# Feedback: Agent First
Never wait on human if clear directive exists.
""", encoding="utf-8")

        class ArgsSyncHarness:
            pass

        args_sync = ArgsSyncHarness()
        args_sync.root = str(root)
        args_sync.harness = "gemini"
        args_sync.ns = ns
        args_sync.local_dir = str(harness_dir)
        args_sync.json = True

        mod.cmd_sync_harness(args_sync)

        # Verify feedback-agent-first.md reached unified namespace
        check("harness sync ingested feedback-agent-first.md to unified root", (ns_dir / "feedback-agent-first.md").exists())
        # Verify existing unified doors propagated to gemini_memory
        check("harness sync pushed finding-pty-grid-ssot.md to harness dir", (harness_dir / "finding-pty-grid-ssot.md").exists())
        check("harness sync pushed campaign-6.0-orchestrator.md to harness dir", (harness_dir / "campaign-6.0-orchestrator.md").exists())

    finally:
        shutil.rmtree(tmp_root, ignore_errors=True)


if __name__ == "__main__":
    run_tests()
    if FAILURES:
        print(f"\n{len(FAILURES)} checks failed: {', '.join(FAILURES)}")
        sys.exit(1)
    else:
        print("\nAll ygg-memory unit tests passed.")
        sys.exit(0)
