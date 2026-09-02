#!/usr/bin/env python3
"""Unit tests for ygg-memory.py unified cross-harness memory system.

Tests status, diff, get, ack (all and selective), publish, and sync-harness
in an isolated scratch directory without touching live user memory.
Includes tests for harness-scoped steering (target_harness).
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

        # 2. Publish a shared memory door from Claude
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
        args_pub.target_harness = None
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
        entries = mod.read_journal_entries(root, after_seq=0, namespace=ns, target_harness="grok")
        check("journal contains 1 entry for grok", len(entries) == 1)
        check("journal summary extracted correctly", "PTY grid SSOT divergence" in entries[0]["summary"])

        # 4. Publish a Gemini-only steer door
        dummy_steer = tmp_root / "steer-gemini-subagent-dispatch.md"
        dummy_steer.write_text("""---
name: steer-gemini-subagent-dispatch
description: "Always schedule subagents in background and check status via manage_task."
metadata:
  type: steer
  target_harness: gemini
---

# Steer: Gemini Subagent Dispatch
Rules for Antigravity/Gemini CLI.
""", encoding="utf-8")

        args_pub.file = str(dummy_steer)
        mod.cmd_publish(args_pub)
        check("latest_seq is now 2", mod.get_latest_seq(root) == 2)

        # 5. Check scoping isolation:
        # Claude checking status should NOT see the Gemini-only steer (behind by 0 since Claude published #1)
        claude_entries = mod.read_journal_entries(root, after_seq=1, namespace=ns, target_harness="claude")
        check("claude ignores gemini-only steer", len(claude_entries) == 0)

        # Gemini checking status should see the Gemini-only steer (#2)
        gemini_entries = mod.read_journal_entries(root, after_seq=1, namespace=ns, target_harness="gemini")
        check("gemini sees gemini-only steer", len(gemini_entries) == 1)
        check("gemini steer has target_harness set to gemini", gemini_entries[0]["target_harness"] == "gemini")

        # 6. Publish a second shared door (Campaign ledger)
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
        check("latest_seq is now 3", mod.get_latest_seq(root) == 3)

        # 7. Selective Ack for Grok (ingest only campaign)
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

        # 8. Global Ack for Grok
        args_ack.all = True
        args_ack.files = None
        mod.cmd_ack(args_ack)

        grok_wm = mod.load_watermark(root, "grok")
        check("global ack brought grok last_seq to 3", grok_wm.get("last_seq") == 3)

        # 9. Bidirectional Sync Harness for Claude
        claude_dir = tmp_root / "claude_memory"
        claude_dir.mkdir(parents=True, exist_ok=True)
        local_finding = claude_dir / "feedback-agent-first.md"
        local_finding.write_text("""# Feedback: Agent First
Never wait on human if clear directive exists.
""", encoding="utf-8")

        class ArgsSyncHarness:
            pass

        args_sync = ArgsSyncHarness()
        args_sync.root = str(root)
        args_sync.harness = "claude"
        args_sync.ns = ns
        args_sync.local_dir = str(claude_dir)
        args_sync.json = True

        mod.cmd_sync_harness(args_sync)

        # Verify feedback-agent-first.md reached unified namespace
        check("harness sync ingested feedback-agent-first.md to unified root", (ns_dir / "feedback-agent-first.md").exists())
        # Verify shared doors propagated to claude_memory
        check("harness sync pushed finding-pty-grid-ssot.md to claude dir", (claude_dir / "finding-pty-grid-ssot.md").exists())
        check("harness sync pushed campaign-6.0-orchestrator.md to claude dir", (claude_dir / "campaign-6.0-orchestrator.md").exists())
        # CRITICAL CHECK: Gemini-only steer must NOT be pushed to claude_memory!
        check("harness sync DID NOT push steer-gemini to claude dir", not (claude_dir / "steer-gemini-subagent-dispatch.md").exists())

        # 10. Muse adapter resolves the XDG native store, never another harness's.
        fake_home = (tmp_root / "fakehome").resolve()
        muse_projects = fake_home / ".local" / "share" / "muse" / "memory" / "projects"
        slug_core = "home-user-proj"
        slug_dir = muse_projects / f"{slug_core}-0123456789abcdef"
        slug_dir.mkdir(parents=True, exist_ok=True)
        lookalike = muse_projects / f"{slug_core}-closed-aaaaaaaaaaaaaaa1"
        lookalike.mkdir(parents=True, exist_ok=True)

        muse_adapter = mod.get_harness_adapter("muse", home=fake_home)
        check("muse adapter rooted at XDG store",
              str(muse_adapter.project_root) == str(muse_projects))
        resolved = muse_adapter.local_dir("-" + slug_core)
        check("muse adapter resolves slug dir",
              resolved is not None and resolved == slug_dir)
        check("muse adapter never resolves under .claude",
              resolved is not None and ".claude" not in str(resolved))
        check("muse adapter matches lookalike exactly, not by prefix",
              muse_adapter.local_dir("-" + slug_core + "-closed") == lookalike)
        check("muse adapter returns None for unknown namespace",
              muse_adapter.local_dir("-no-such-workspace") is None)
        check("muse adapter returns None for global namespace",
              muse_adapter.local_dir(mod.GLOBAL_NAMESPACE) is None)
        # Ambiguous duplicate slug cores must not sync anywhere.
        (muse_projects / f"{slug_core}-bbbbbbbbbbbbbbb2").mkdir(parents=True, exist_ok=True)
        check("muse adapter returns None on ambiguous slug cores",
              muse_adapter.local_dir("-" + slug_core) is None)

        # 11. Muse sync_namespace end-to-end against an isolated hub.
        closed_ns = "-" + slug_core + "-closed"
        (lookalike / "closed-note.md").write_text("# closed\n", encoding="utf-8")
        muse_hub = tmp_root / "musehub"
        muse_adapter.sync_namespace(muse_hub, "muse", closed_ns)
        check("muse sync ingested native note to hub",
              (muse_hub / "namespaces" / closed_ns / "closed-note.md").exists())
        hub_door_src = tmp_root / "muse-hub-door.md"
        hub_door_src.write_text("# hub door\n", encoding="utf-8")
        args_pub.root = str(muse_hub)
        args_pub.harness = "muse"
        args_pub.ns = closed_ns
        args_pub.file = str(hub_door_src)
        mod.cmd_publish(args_pub)
        muse_adapter.sync_namespace(muse_hub, "muse", closed_ns)
        check("muse sync delivered hub door to slug dir",
              (lookalike / "muse-hub-door.md").exists())
        before = sorted(p.name for p in muse_projects.iterdir())
        check("muse sync of unknown namespace is a no-op",
              muse_adapter.sync_namespace(muse_hub, "muse", "-no-such-workspace") == (0, 0, 0))
        check("muse sync of unknown namespace created no dirs",
              sorted(p.name for p in muse_projects.iterdir()) == before)

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
