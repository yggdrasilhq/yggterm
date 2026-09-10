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

        # 9b. The strict one-area gate is explicit, testable, and reports a
        # converged namespace without requiring the live native store.
        class ArgsSync:
            pass

        args_gate = ArgsSync()
        args_gate.root = str(root)
        args_gate.harness = "claude"
        args_gate.ns = ns
        args_gate.local_dir = str(claude_dir)
        args_gate.fleet = False
        args_gate.mesh = None
        args_gate.quick = False
        args_gate.json = True
        mod.cmd_sync(args_gate)
        check("strict area sync gate completed", True)

        # Global options must work before the subcommand as well as after it.
        saved_argv = sys.argv[:]
        try:
            sys.argv = [
                "ygg-memory.py", "--root", str(root), "--harness", "claude",
                "sync", f"--ns={ns}", "--local-dir", str(claude_dir), "--json",
            ]
            mod.main()
            global_options_before_subcommand = True
        except SystemExit:
            global_options_before_subcommand = False
        finally:
            sys.argv = saved_argv
        check("global options survive parsing before the subcommand", global_options_before_subcommand)

        # 9c. User-controlled namespace and door paths cannot escape the hub.
        try:
            mod.validate_namespace("-safe/../outside")
            safe_namespace = False
        except ValueError:
            safe_namespace = True
        check("namespace traversal is rejected", safe_namespace)
        try:
            mod.validate_door_filename("../outside.md")
            safe_filename = False
        except ValueError:
            safe_filename = True
        check("door filename traversal is rejected", safe_filename)
        try:
            mod.object_path(root, "../outside")
            safe_object = False
        except ValueError:
            safe_object = True
        check("object digest traversal is rejected", safe_object)

        missing_ns = "-missing-read-only-namespace"
        args_get = type("ArgsGet", (), {
            "root": str(root), "ns": missing_ns, "file": "missing.md",
            "grep": None, "lines": None,
        })()
        try:
            mod.cmd_get(args_get)
        except SystemExit:
            pass
        check("missing get does not create a namespace directory",
              not (root / "namespaces" / missing_ns).exists())

        # 9d. The strict gate fails closed on a real divergent causal area.
        conflict_root = tmp_root / "conflict-memory"
        conflict_ns = "-conflict-area"
        conflict_dir = mod.get_namespace_dir(conflict_root, conflict_ns)
        conflict_file = conflict_dir / "campaign-conflict.md"
        conflict_file.write_text("# first head\n", encoding="utf-8")
        mod.append_journal_entry(
            conflict_root, conflict_ns, conflict_file.name, "campaign", "create",
            "claude", "first head",
        )
        conflict_file.write_text("# second head\n", encoding="utf-8")
        mod.append_journal_entry(
            conflict_root, conflict_ns, conflict_file.name, "campaign", "update",
            "claude", "second head", base_version=None,
        )
        conflict_native = tmp_root / "conflict-native"
        args_conflict = ArgsSync()
        args_conflict.root = str(conflict_root)
        args_conflict.harness = "claude"
        args_conflict.ns = conflict_ns
        args_conflict.local_dir = str(conflict_native)
        args_conflict.fleet = False
        args_conflict.mesh = None
        args_conflict.quick = False
        args_conflict.json = True
        try:
            mod.cmd_sync(args_conflict)
            conflict_failed_closed = False
        except SystemExit as exc:
            conflict_failed_closed = exc.code == 2
        check("strict area sync fails closed on conflicts", conflict_failed_closed)
        check("failed area sync does not advance its watermark",
              mod.load_watermark(conflict_root, "claude").get("last_sync_ts") is None)

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

        # 12. Subpath doors — the campaign-cli-integration/<cli>.md shape
        # (dream ACK-9cdea1ec3a / ACK-2dc9a05503): publish --dest, get, ack,
        # and the three-way sync must treat a subpath door exactly like a
        # flat one, while pinned/yggterm stays OUT of project-door sync.
        check("validator accepts subpath door",
              mod.validate_door_filename("campaign-cli-integration/codex.md")
              == "campaign-cli-integration/codex.md")
        for bad in ("../escape.md", "/abs.md", "a\\b.md", "sub//x.md",
                    "sub/./x.md", "sub/../x.md", "sub/", "./x.md", ".."):
            try:
                mod.validate_door_filename(bad)
                check(f"validator rejects {bad!r}", False)
            except ValueError:
                check(f"validator rejects {bad!r}", True)

        sub_door_src = tmp_root / "codex-door.md"
        sub_door_src.write_text("""---
name: campaign-cli-integration-codex
description: "Per-CLI door for codex (11.6.1) — measured reattach facts."
metadata:
  type: campaign
---

# codex — integration door
""", encoding="utf-8")
        args_pub.dest = "campaign-cli-integration/codex.md"
        args_pub.file = str(sub_door_src)
        args_pub.root = str(root)
        args_pub.ns = ns
        args_pub.harness = "claude"
        args_pub.json = True
        mod.cmd_publish(args_pub)
        sub_door = ns_dir / "campaign-cli-integration" / "codex.md"
        check("publish --dest wrote subdirectory door", sub_door.exists())
        all_entries = mod.read_journal_entries(root, after_seq=0, namespace=ns)
        check("journal keys the subpath door by its subpath",
              any(e.get("file") == "campaign-cli-integration/codex.md" for e in all_entries))
        check("namespace index links the subpath",
              "](campaign-cli-integration/codex.md)" in mem_index.read_text(encoding="utf-8"))
        republish_markers = mem_index.read_text(encoding="utf-8").count(
            "](campaign-cli-integration/codex.md)")
        mod.cmd_publish(args_pub)
        check("publish --dest is idempotent in the index",
              mem_index.read_text(encoding="utf-8").count(
                  "](campaign-cli-integration/codex.md)") == republish_markers)

        class ArgsGet:
            pass

        args_get = ArgsGet()
        args_get.root = str(root)
        args_get.ns = ns
        args_get.file = "campaign-cli-integration/codex.md"
        args_get.grep = None
        args_get.lines = None
        import contextlib
        import io as _io
        buf = _io.StringIO()
        with contextlib.redirect_stdout(buf):
            mod.cmd_get(args_get)
        check("get --file reads a subpath door", "integration door" in buf.getvalue())

        # The hub subpath door must reach the native store as a subpath file.
        mod.cmd_sync_harness(args_sync)
        check("harness sync delivered subpath door to claude dir",
              (claude_dir / "campaign-cli-integration" / "codex.md").exists())

        # A native subpath file must ingest into the hub as a subpath door.
        native_sub = claude_dir / "notes" / "lane-note.md"
        native_sub.parent.mkdir(parents=True, exist_ok=True)
        native_sub.write_text("# lane note\n", encoding="utf-8")
        mod.cmd_sync_harness(args_sync)
        check("harness sync ingested native subpath file to hub",
              (ns_dir / "notes" / "lane-note.md").exists())

        # ack --all must record subpath keys, not skip them.
        class ArgsAck:
            pass

        args_ack = ArgsAck()
        args_ack.root = str(root)
        args_ack.harness = "claude"
        args_ack.ns = ns
        args_ack.all = True
        args_ack.files = None
        args_ack.json = True
        mod.cmd_ack(args_ack)
        wm_claude = mod.load_watermark(root, "claude")
        check("ack --all recorded the subpath door",
              wm_claude.get("namespaces", {}).get(ns, {}).get(
                  "campaign-cli-integration/codex.md") is not None)

        # The native pinned/yggterm shelf is the global namespace's delivery
        # target and must NEVER ingest as project doors.
        pinned = claude_dir / "pinned" / "yggterm" / "global-door.md"
        pinned.parent.mkdir(parents=True, exist_ok=True)
        pinned.write_text("# global door\n", encoding="utf-8")
        mod.cmd_sync_harness(args_sync)
        check("pinned/yggterm native subtree never ingested as project doors",
              not (ns_dir / "pinned" / "yggterm" / "global-door.md").exists()
              and not (ns_dir / "global-door.md").exists())

        # 13. The delete verb — journaled total supersede, recoverable by
        # re-publishing (dream ACK-64299ad7cd).
        class ArgsDelete:
            pass

        args_del = ArgsDelete()
        args_del.root = str(root)
        args_del.harness = "claude"
        args_del.ns = ns
        args_del.file = "finding-pty-grid-ssot.md"
        args_del.json = True
        mod.cmd_delete(args_del)
        check("delete verb removed the hub door",
              not (ns_dir / "finding-pty-grid-ssot.md").exists())
        door_entries = [e for e in mod.read_journal_entries(root, after_seq=0, namespace=ns)
                        if e.get("file") == "finding-pty-grid-ssot.md"]
        del_records = [e for e in door_entries if e.get("action") == "delete"]
        check("delete journaled with action=delete", bool(del_records))
        check("delete superseded prior heads",
              bool(del_records[-1].get("base_versions")))
        wm_del = mod.load_watermark(root, "claude")
        check("delete dropped the door from the watermark map",
              wm_del.get("namespaces", {}).get(ns, {}).get("finding-pty-grid-ssot.md") is None)

        args_del.file = "no-such-door-anywhere.md"
        try:
            mod.cmd_delete(args_del)
            check("delete of unknown door refused", False)
        except SystemExit:
            check("delete of unknown door refused", True)

        args_pub.dest = None
        args_pub.file = str(dummy_door)
        args_pub.root = str(root)
        args_pub.ns = ns
        args_pub.harness = "claude"
        args_pub.json = True
        mod.cmd_publish(args_pub)
        check("republish after delete resurrects the door",
              (ns_dir / "finding-pty-grid-ssot.md").exists())

        # 14. The churn guard — a mirror door the causal store retired is not
        # resurrected by an unchanged source, the native source survives, and
        # a real content change resurrects deliberately (dream
        # ACK-8fa18b0cc2).
        mirror_native = tmp_root / "mirror_native"
        mirror_native.mkdir(parents=True, exist_ok=True)
        churn = mirror_native / "churn-note.md"
        churn.write_text("# churn\n", encoding="utf-8")
        mod._native_door_causal_cache.clear()
        mod._sync_native_tree(root, "zcode", mod.GLOBAL_NAMESPACE, mirror_native,
                              "memory", target_harness="all")
        glob_dir = mod.get_namespace_dir(root, mod.GLOBAL_NAMESPACE)
        mirrors = sorted(glob_dir.glob("native-zcode-memory-*-churn-note.md"))
        check("mirror tree created the churn door", len(mirrors) == 1)
        mirror_name = mirrors[0].name

        args_del.file = mirror_name
        args_del.ns = mod.GLOBAL_NAMESPACE
        args_del.harness = "zcode"
        mod.cmd_delete(args_del)
        check("churn door deleted through the verb",
              not (glob_dir / mirror_name).exists())

        mod._native_door_causal_cache.clear()
        mod._sync_native_tree(root, "zcode", mod.GLOBAL_NAMESPACE, mirror_native,
                              "memory", target_harness="all")
        check("unchanged source did NOT resurrect the retired mirror",
              not (glob_dir / mirror_name).exists())
        check("native churn source survived the retired door", churn.exists())

        churn.write_text("# churn CHANGED\n", encoding="utf-8")
        mod._native_door_causal_cache.clear()
        mod._sync_native_tree(root, "zcode", mod.GLOBAL_NAMESPACE, mirror_native,
                              "memory", target_harness="all")
        check("changed source resurrects the mirror deliberately",
              (glob_dir / mirror_name).exists())

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
