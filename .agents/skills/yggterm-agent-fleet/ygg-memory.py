#!/usr/bin/env python3
"""Unified Cross-Harness Fleet Memory Management Tool (`ygg-memory.py`).

Maintains the host-resident, cross-harness memory synchronization hub under
`~/.yggterm/memory`.

Enables any agent CLI harness (Claude, Gemini/Antigravity, Grok, Codex, Muse,
etc.) to:
  - Check diffs with cheap token-efficient toolcalls (<40 tokens)
  - Ingest selective/impatient or full memory diffs
  - Advance harness watermarks
  - Publish new memory findings/campaign ledgers
  - Bidirectionally sync with harness-native memory stores
  - Mesh sync with peer hosts (***, dev, oc) over SSH
"""

import argparse
import datetime
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_MEMORY_ROOT = Path(os.environ.get("YGGTERM_MEMORY_ROOT", Path.home() / ".yggterm" / "memory"))
BACKUP_ROOT = Path.home() / ".yggterm" / "memory-backups"
ARCHIVE_ROOT = Path.home() / ".yggterm" / "memory-archive"
LOCK_FILE = DEFAULT_MEMORY_ROOT / ".ygg-memory.lock"

STEERING_HEADER = """# Memory Index

> 🌐 **UNIFIED FLEET MEMORY**: Before deep memory recall or after campaign handovers, consult `ygg-memory status --harness <me>` or `ygg-memory diff` to catch updates from Claude, Grok, Codex, or Gemini. Ingest full or partial diffs as needed.
> ⛔ **Doors, not rooms.** Rules (`feedback-/spec-/reference-/user-`) · ledgers (`campaign-/project-`) · findings (`finding-/bug-class-`).
> One line, one door. Detail belongs in the target file, never here.
"""


def _flock_open(lock_path: Path):
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    f = open(lock_path, "a+")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        # Wait up to 10 seconds for concurrent write
        for _ in range(20):
            time.sleep(0.5)
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                pass
        else:
            raise RuntimeError(f"Lock held by another process: {lock_path}")
    return f


def _flock_close(f):
    if f:
        try:
            fcntl.flock(f, fcntl.LOCK_UN)
            f.close()
        except Exception:
            pass


def detect_harness(override: str = None) -> str:
    if override:
        return override.strip().lower()
    if os.environ.get("CLAUDE_PROJECT_DIR") or os.environ.get("CLAUDE_SESSION_ID"):
        return "claude"
    if os.environ.get("GEMINI_CLI") or os.environ.get("ANTIGRAVITY_SESSION"):
        return "gemini"
    if os.environ.get("CODEX_SESSION") or os.environ.get("CODEX_HOME"):
        return "codex"
    if os.environ.get("GROK_SESSION"):
        return "grok"
    if os.environ.get("MUSE_SESSION"):
        return "muse"
    return "unknown"


def detect_namespace(cwd: Path = None, override: str = None) -> str:
    if override:
        ns = override.strip()
        if not ns.startswith("-"):
            ns = "-" + ns.replace("/", "-").strip("-")
        return ns
    target = (cwd or Path.cwd()).resolve()
    # Normalize absolute path to slug: /home/user/proj -> -home-user-proj
    slug = str(target).replace("/", "-")
    return slug


def get_namespace_dir(root: Path, namespace: str) -> Path:
    d = root / "namespaces" / namespace
    d.mkdir(parents=True, exist_ok=True)
    return d


def get_watermark_path(root: Path, harness: str) -> Path:
    d = root / "watermarks"
    d.mkdir(parents=True, exist_ok=True)
    return d / f"{harness}.json"


def load_watermark(root: Path, harness: str) -> dict:
    p = get_watermark_path(root, harness)
    if not p.exists():
        return {
            "harness": harness,
            "last_seq": 0,
            "last_sync_ts": None,
            "namespaces": {},
        }
    try:
        with open(p, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {
            "harness": harness,
            "last_seq": 0,
            "last_sync_ts": None,
            "namespaces": {},
        }


def save_watermark(root: Path, watermark: dict):
    p = get_watermark_path(root, watermark["harness"])
    temp = p.with_suffix(".tmp")
    with open(temp, "w", encoding="utf-8") as f:
        json.dump(watermark, f, indent=2)
    temp.replace(p)


def get_journal_path(root: Path) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    return root / "journal.jsonl"


def read_journal_entries(root: Path, after_seq: int = 0, namespace: str = None) -> list:
    jpath = get_journal_path(root)
    if not jpath.exists():
        return []
    entries = []
    with open(jpath, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
                if rec.get("seq", 0) > after_seq:
                    if namespace is None or rec.get("ns") == namespace:
                        entries.append(rec)
            except Exception:
                continue
    return entries


def get_latest_seq(root: Path) -> int:
    jpath = get_journal_path(root)
    if not jpath.exists():
        return 0
    latest = 0
    with open(jpath, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
                latest = max(latest, rec.get("seq", 0))
            except Exception:
                continue
    return latest


def append_journal_entry(root: Path, ns: str, filename: str, kind: str, action: str, harness: str, summary: str) -> dict:
    jpath = get_journal_path(root)
    latest = get_latest_seq(root)
    next_seq = latest + 1
    now_ts = int(time.time())
    iso_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
    record = {
        "seq": next_seq,
        "ts": now_ts,
        "iso": iso_ts,
        "ns": ns,
        "file": filename,
        "kind": kind,
        "action": action,
        "harness": harness,
        "summary": summary,
    }
    with open(jpath, "a", encoding="utf-8") as f:
        f.write(json.dumps(record) + "\n")
    return record


def extract_metadata_and_summary(content: str) -> tuple:
    """Extract frontmatter kind, description/summary, or first line hook."""
    kind = "other"
    summary = ""
    if content.startswith("---"):
        parts = content.split("---", 2)
        if len(parts) >= 3:
            fm = parts[1]
            for line in fm.splitlines():
                line = line.strip()
                if line.startswith("type:"):
                    kind = line.split(":", 1)[1].strip()
                elif line.startswith("description:"):
                    summary = line.split(":", 1)[1].strip().strip('"').strip("'")
            content_body = parts[2]
        else:
            content_body = content
    else:
        content_body = content

    if not summary:
        for line in content_body.splitlines():
            line = line.strip()
            if line.startswith("#"):
                summary = line.lstrip("#").strip()
                break
            elif line and not line.startswith("<!--") and not line.startswith(">"):
                summary = line[:120]
                break

    return kind, summary or "Updated memory door"


def file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


# -----------------------------------------------------------------------------
# Subcommands
# -----------------------------------------------------------------------------

def cmd_status(args):
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    watermark = load_watermark(root, harness)
    last_seq = watermark.get("last_seq", 0)
    latest_seq = get_latest_seq(root)

    new_entries = read_journal_entries(root, after_seq=last_seq, namespace=ns)
    # Deduplicate changed door filenames
    changed_doors = list(dict.fromkeys([e["file"] for e in new_entries if e.get("file")]))

    output = {
        "harness": harness,
        "namespace": ns,
        "behind": len(changed_doors),
        "last_seq": last_seq,
        "latest_seq": latest_seq,
        "last_sync_ts": watermark.get("last_sync_ts"),
        "changed_doors": changed_doors,
    }

    if args.json:
        print(json.dumps(output))
    else:
        if output["behind"] == 0:
            print(f"Memory up-to-date ({harness} on {ns} at seq #{last_seq})")
        else:
            print(f"Memory behind by {output['behind']} doors ({harness} at #{last_seq}, latest is #{latest_seq}):")
            for d in changed_doors[:10]:
                print(f"  - {d}")
            if len(changed_doors) > 10:
                print(f"  ... and {len(changed_doors) - 10} more")


def cmd_diff(args):
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    watermark = load_watermark(root, harness)
    last_seq = watermark.get("last_seq", 0)

    entries = read_journal_entries(root, after_seq=last_seq, namespace=ns)
    if args.filter:
        flt = args.filter.lower()
        entries = [e for e in entries if flt in e.get("kind", "").lower() or flt in e.get("file", "").lower()]

    if args.json:
        print(json.dumps({"harness": harness, "namespace": ns, "diff_entries": entries}))
        return

    if not entries:
        print(f"No unabsorbed diffs for {harness} in namespace {ns}.")
        return

    print(f"Diffs for {harness} in {ns} (since seq #{last_seq}):")
    for e in entries:
        seq = e.get("seq", 0)
        kind = e.get("kind", "door")
        fname = e.get("file", "unknown")
        author = e.get("harness", "unknown")
        summ = e.get("summary", "")
        print(f"[#{seq} | {kind}] {fname} (by {author}): {summ}")


def cmd_get(args):
    root = Path(args.root)
    ns = detect_namespace(override=args.ns)
    ns_dir = get_namespace_dir(root, ns)
    target = ns_dir / args.file
    if not target.exists():
        print(f"Error: Door '{args.file}' not found in namespace '{ns}'.", file=sys.stderr)
        sys.exit(1)
    with open(target, "r", encoding="utf-8") as f:
        print(f.read())


def cmd_ack(args):
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    lock = _flock_open(LOCK_FILE)
    try:
        watermark = load_watermark(root, harness)
        latest_seq = get_latest_seq(root)
        ns_map = watermark.setdefault("namespaces", {}).setdefault(ns, {})

        if args.all:
            watermark["last_seq"] = latest_seq
            watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            # Record current hashes of all doors in ns
            ns_dir = get_namespace_dir(root, ns)
            for fpath in ns_dir.glob("*.md"):
                ns_map[fpath.name] = file_sha256(fpath)
            save_watermark(root, watermark)
            if args.json:
                print(json.dumps({"status": "ok", "acked": "all", "seq": latest_seq}))
            else:
                print(f"Acknowledged all doors for {harness} up to seq #{latest_seq}.")
        elif args.files:
            acked_files = []
            ns_dir = get_namespace_dir(root, ns)
            for fname in [f.strip() for f in args.files.split(",") if f.strip()]:
                fpath = ns_dir / fname
                if fpath.exists():
                    ns_map[fname] = file_sha256(fpath)
                    acked_files.append(fname)
            watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            save_watermark(root, watermark)
            if args.json:
                print(json.dumps({"status": "ok", "acked": acked_files, "last_seq": watermark.get("last_seq", 0)}))
            else:
                print(f"Acknowledged {len(acked_files)} doors for {harness}: {', '.join(acked_files)}")
        else:
            print("Specify --all or --files <file1,file2>", file=sys.stderr)
            sys.exit(1)
    finally:
        _flock_close(lock)


def cmd_publish(args):
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    source_path = Path(args.file).resolve()
    if not source_path.exists():
        print(f"Error: Source file '{args.file}' does not exist.", file=sys.stderr)
        sys.exit(1)

    lock = _flock_open(LOCK_FILE)
    try:
        ns_dir = get_namespace_dir(root, ns)
        dest_filename = source_path.name
        dest_path = ns_dir / dest_filename

        content = source_path.read_text(encoding="utf-8")
        kind, summary = extract_metadata_and_summary(content)
        if args.summary:
            summary = args.summary.strip()
        if args.kind:
            kind = args.kind.strip()

        is_new = not dest_path.exists()
        action = "create" if is_new else "update"

        shutil.copy2(source_path, dest_path)
        record = append_journal_entry(root, ns, dest_filename, kind, action, harness, summary)

        # Update publisher's own watermark for this file
        watermark = load_watermark(root, harness)
        watermark.setdefault("namespaces", {}).setdefault(ns, {})[dest_filename] = file_sha256(dest_path)
        watermark["last_seq"] = max(watermark.get("last_seq", 0), record["seq"])
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)

        # Update root MEMORY.md if absent or add pointer
        memory_index = ns_dir / "MEMORY.md"
        if not memory_index.exists():
            memory_index.write_text(STEERING_HEADER + f"\n## Doors\n\n- [{dest_filename}]({dest_filename}) — {summary}\n", encoding="utf-8")
        else:
            idx_content = memory_index.read_text(encoding="utf-8")
            if not idx_content.startswith("> 🌐 **UNIFIED FLEET MEMORY**") and "UNIFIED FLEET MEMORY" not in idx_content:
                idx_content = STEERING_HEADER + "\n" + idx_content
                memory_index.write_text(idx_content, encoding="utf-8")

        if args.json:
            print(json.dumps({"status": "ok", "record": record}))
        else:
            print(f"Published '{dest_filename}' to namespace '{ns}' (seq #{record['seq']}).")
    finally:
        _flock_close(lock)


def cmd_sync_harness(args):
    """Bidirectional sync between harness-local directory and ~/.yggterm/memory."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    lock = _flock_open(LOCK_FILE)

    try:
        # Determine local harness memory dir
        if args.local_dir:
            local_dir = Path(args.local_dir).resolve()
        elif harness == "claude":
            local_dir = Path.home() / ".claude" / "projects" / ns / "memory"
        elif harness == "gemini":
            local_dir = Path.home() / ".gemini" / "projects" / ns / "memory"
        else:
            local_dir = Path.home() / f".{harness}" / "projects" / ns / "memory"

        local_dir.mkdir(parents=True, exist_ok=True)
        ns_dir = get_namespace_dir(root, ns)

        # Snapshot local first
        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
        backup_dir = BACKUP_ROOT / ns / stamp
        backup_dir.mkdir(parents=True, exist_ok=True)
        for f in local_dir.glob("*.md"):
            shutil.copy2(f, backup_dir)

        in_count = 0
        out_count = 0

        # Pass 1: Ingest from local harness -> unified root (if local newer or unified missing)
        for loc_file in local_dir.glob("*.md"):
            dest_file = ns_dir / loc_file.name
            if not dest_file.exists() or loc_file.stat().st_mtime > dest_file.stat().st_mtime:
                content = loc_file.read_text(encoding="utf-8")
                kind, summary = extract_metadata_and_summary(content)
                shutil.copy2(loc_file, dest_file)
                append_journal_entry(root, ns, loc_file.name, kind, "upsert", harness, summary)
                in_count += 1

        # Pass 2: Propagate from unified root -> local harness (if unified newer or local missing)
        for uni_file in ns_dir.glob("*.md"):
            dest_file = local_dir / uni_file.name
            if not dest_file.exists() or uni_file.stat().st_mtime > dest_file.stat().st_mtime:
                shutil.copy2(uni_file, dest_file)
                out_count += 1

        # Ensure steering header in local MEMORY.md
        loc_mem = local_dir / "MEMORY.md"
        if loc_mem.exists():
            txt = loc_mem.read_text(encoding="utf-8")
            if "UNIFIED FLEET MEMORY" not in txt:
                loc_mem.write_text(STEERING_HEADER + "\n" + txt, encoding="utf-8")

        watermark = load_watermark(root, harness)
        watermark["last_seq"] = get_latest_seq(root)
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)

        if args.json:
            print(json.dumps({"status": "ok", "harness": harness, "namespace": ns, "pulled_in": in_count, "pushed_out": out_count}))
        else:
            print(f"Harness sync completed ({harness} <-> {ns}): {in_count} ingested, {out_count} propagated.")
    finally:
        _flock_close(lock)


def cmd_sync_fleet(args):
    """Mesh synchronize ~/.yggterm/memory across reachable fleet hosts over SSH."""
    root = Path(args.root)
    mesh = [h.strip() for h in args.mesh.split() if h.strip()]
    lock = _flock_open(LOCK_FILE)

    try:
        # Detect local host
        local_host = os.uname().nodename
        peers = [h for h in mesh if h != local_host]
        live_peers = []

        ssh_cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "-o", "LogLevel=ERROR"]
        for p in peers:
            r = subprocess.run(ssh_cmd + [p, "true"], capture_output=True)
            if r.returncode == 0:
                live_peers.append(p)

        if not live_peers:
            print("No reachable peer hosts in fleet mesh.")
            return

        # Snapshot local
        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
        backup = BACKUP_ROOT / "fleet-mesh" / stamp
        backup.mkdir(parents=True, exist_ok=True)
        if root.exists():
            for sub in (root / "namespaces").glob("*/*.md"):
                sub_bak = backup / sub.parent.name
                sub_bak.mkdir(parents=True, exist_ok=True)
                shutil.copy2(sub, sub_bak)

        pulled = 0
        pushed = 0

        # Two-pass rsync with newest-wins per file
        for peer in live_peers:
            # Create remote memory dir if missing
            subprocess.run(ssh_cmd + [peer, "mkdir -p ~/.yggterm/memory/namespaces"], capture_output=True)
            # Pass 1: Pull from peer
            r_pull = subprocess.run([
                "rsync", "-az", "-u", "--itemize-changes", "-e", " ".join(ssh_cmd),
                f"{peer}:.yggterm/memory/namespaces/", str(root / "namespaces/")
            ], capture_output=True, text=True)
            pulled += len([ln for ln in r_pull.stdout.splitlines() if ln.startswith(">") or ln.startswith("<")])

            # Pass 2: Push union back out
            r_push = subprocess.run([
                "rsync", "-az", "-u", "--itemize-changes", "-e", " ".join(ssh_cmd),
                str(root / "namespaces/"), f"{peer}:.yggterm/memory/namespaces/"
            ], capture_output=True, text=True)
            pushed += len([ln for ln in r_push.stdout.splitlines() if ln.startswith(">") or ln.startswith("<")])

        print(f"Fleet memory sync complete across {len(live_peers)} peers ({', '.join(live_peers)}): {pulled} pulled, {pushed} pushed.")
    finally:
        _flock_close(lock)


def main():
    common_parser = argparse.ArgumentParser(add_help=False)
    common_parser.add_argument("--root", default=str(DEFAULT_MEMORY_ROOT), help="Root path for ~/.yggterm/memory")
    common_parser.add_argument("--harness", default=None, help="Agent CLI harness name (claude, gemini, grok, codex, muse)")
    common_parser.add_argument("--ns", default=None, help="Project namespace (e.g. -home-pi-gh-yggterm)")
    common_parser.add_argument("--json", action="store_true", help="Format output as JSON for tool calls")

    parser = argparse.ArgumentParser(description="Unified Cross-Harness Fleet Memory Tool", parents=[common_parser])
    subparsers = parser.add_subparsers(dest="subcommand", required=True)

    # status
    p_status = subparsers.add_parser("status", parents=[common_parser], help="Check if harness memory is behind")

    # diff
    p_diff = subparsers.add_parser("diff", parents=[common_parser], help="View delta doors since last sync")
    p_diff.add_argument("--filter", default=None, help="Filter diffs by kind or topic keyword")

    # get
    p_get = subparsers.add_parser("get", parents=[common_parser], help="Retrieve body of a specific memory door")
    p_get.add_argument("--file", required=True, help="Memory filename (e.g. finding-pty-grid-ssot.md)")

    # ack
    p_ack = subparsers.add_parser("ack", parents=[common_parser], help="Advance harness watermark")
    p_ack.add_argument("--all", action="store_true", help="Acknowledge all doors up to latest sequence")
    p_ack.add_argument("--files", default=None, help="Comma-separated filenames to selectively acknowledge")

    # publish
    p_pub = subparsers.add_parser("publish", parents=[common_parser], help="Publish a local file into unified memory")
    p_pub.add_argument("--file", required=True, help="Source markdown file to publish")
    p_pub.add_argument("--kind", default=None, help="Kind (finding, campaign, spec, feedback)")
    p_pub.add_argument("--summary", default=None, help="One-line summary description")

    # sync-harness
    p_sync_h = subparsers.add_parser("sync-harness", parents=[common_parser], help="Bi-directional sync with local harness store")
    p_sync_h.add_argument("--local-dir", default=None, help="Explicit local harness memory directory")

    # sync-fleet
    p_sync_f = subparsers.add_parser("sync-fleet", parents=[common_parser], help="Mesh sync ~/.yggterm/memory across SSH peers")
    p_sync_f.add_argument("--mesh", default="*** dev oc", help="Space-separated list of peer SSH hosts")

    args = parser.parse_args()

    if args.subcommand == "status":
        cmd_status(args)
    elif args.subcommand == "diff":
        cmd_diff(args)
    elif args.subcommand == "get":
        cmd_get(args)
    elif args.subcommand == "ack":
        cmd_ack(args)
    elif args.subcommand == "publish":
        cmd_publish(args)
    elif args.subcommand == "sync-harness":
        cmd_sync_harness(args)
    elif args.subcommand == "sync-fleet":
        cmd_sync_fleet(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
