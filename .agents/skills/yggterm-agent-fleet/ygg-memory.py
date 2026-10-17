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
  - Support harness-scoped steering (`target_harness`) to avoid cross-harness conflation
  - Bidirectionally sync with harness-native memory stores
  - Mesh sync with peer hosts over SSH (roster resolved by `resolve_fleet_mesh`)
"""

import argparse
import datetime
import fcntl
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.parse
import uuid
from pathlib import Path, PurePosixPath

DEFAULT_MEMORY_ROOT = Path(os.environ.get("YGGTERM_MEMORY_ROOT", Path.home() / ".yggterm" / "memory"))
ARCHIVE_ROOT = Path.home() / ".yggterm" / "memory-archive"

# ⛔ THE FLEET ROSTER IS CONFIGURATION, NOT A CONSTANT. It used to be the
# literal string of three host aliases, hardcoded here as the `--mesh` default
# and again in `ygg-memory-sync`. Both files ship in a PUBLIC repo, so every
# push carried the fleet's private host names, and a second copy meant the two
# could drift. The roster now lives outside every checkout, one alias per line,
# beside the privacy guard's own answer key — the same pattern for the same
# reason.
#
# ⇒ Resolution order, each step answering what the next cannot:
#   1. --mesh          — an operator said so; nothing outranks that.
#   2. $YGG_FLEET_MESH — a one-run override, space- or comma-separated.
#   3. the roster file — the machine's own standing answer.
# There is deliberately NO built-in fallback: a default would put the names
# back in the tree, and a silent empty mesh would make `sync-fleet` a no-op
# that reports success. An unresolved roster raises instead.
FLEET_MESH_FILE = Path(os.environ.get("YGG_FLEET_MESH_FILE", Path.home() / ".config" / "ygg-fleet" / "mesh"))


def resolve_fleet_mesh(explicit=None):
    """Return the fleet's peer ssh aliases. The ONE owner of that question."""
    def _split(raw):
        return [h for h in re.split(r"[\s,]+", raw.strip()) if h and not h.startswith("#")]

    if explicit:
        return _split(explicit)
    env = os.environ.get("YGG_FLEET_MESH", "")
    if env.strip():
        return _split(env)
    if FLEET_MESH_FILE.is_file():
        hosts = []
        for line in FLEET_MESH_FILE.read_text(encoding="utf-8", errors="replace").splitlines():
            line = line.split("#", 1)[0].strip()
            if line:
                hosts.extend(_split(line))
        if hosts:
            return hosts
    raise SystemExit(
        "ygg-memory: no fleet mesh roster.\n"
        f"  tried: --mesh (not given), $YGG_FLEET_MESH (unset), {FLEET_MESH_FILE} (absent or empty).\n"
        "  Write one ssh alias per line into that file, or pass --mesh explicitly."
    )


STEERING_HEADER = """# Memory Index

> 🌐 **UNIFIED FLEET MEMORY**: Before deep memory recall, campaign continuation, or after a handover, run `ygg-memory sync --harness <me> --ns=<area>` first. Then inspect `status`/`diff` and open only the relevant door with `get --file=<door> --ns=<area>`. If another host may have learned something, add `--fleet` to the sync.
> ⛔ **Doors, not rooms.** Rules (`feedback-/spec-/reference-/user-`) · ledgers (`campaign-/project-`) · findings (`finding-/bug-class-`) · steers (`steer-<harness>-`).
> One line, one door. Detail belongs in the target file, never here.
"""

GLOBAL_NAMESPACE = "_global"
MANAGED_BLOCK_BEGIN = "<!-- BEGIN yggterm-memory -->"
MANAGED_BLOCK_END = "<!-- END yggterm-memory -->"


def validate_namespace(namespace: str) -> str:
    """Return a single safe namespace directory name."""
    if not isinstance(namespace, str) or not namespace:
        raise ValueError("memory namespace must not be empty")
    if namespace in {".", "..", "-"} or any(char in namespace for char in ("/", "\\", "\x00")):
        raise ValueError(f"invalid memory namespace: {namespace!r}")
    if any(ord(char) < 32 for char in namespace):
        raise ValueError(f"invalid control character in memory namespace: {namespace!r}")
    return namespace


def validate_door_filename(filename: str) -> str:
    """Return a namespace-relative door path that cannot escape its namespace.

    A door is usually a single Markdown filename. Doors that live in a
    namespace subdirectory (e.g. ``campaign-cli-integration/codex.md``, the
    per-CLI doors of the 3.3.0 integration campaign) name that subpath —
    the journal, watermarks and sync all key doors by this string, so a
    subpath door is a first-class door everywhere a flat one is. Every
    path segment must be a plain, non-hidden filename: no absolute paths,
    no ``..``, no backslashes, no control characters.
    """
    if not isinstance(filename, str) or not filename:
        raise ValueError("memory door filename must not be empty")
    if "\\" in filename or "\x00" in filename:
        raise ValueError(f"invalid character in memory door filename: {filename!r}")
    if any(ord(char) < 32 for char in filename):
        raise ValueError(f"invalid control character in memory door filename: {filename!r}")
    segments = filename.split("/")
    if any(segment in ("", ".", "..") for segment in segments):
        raise ValueError(
            f"memory door must be a relative path under its namespace: {filename!r}"
        )
    return filename


def memory_bridge(harness: str) -> str:
    """Small always-loaded door; the hub remains the room and source of truth."""
    canonical = normalize_harness_name(harness)
    return (
        f"{MANAGED_BLOCK_BEGIN}\n"
        "## Yggterm fleet memory\n\n"
        "Yggterm synchronizes semantic memory before managed CLI startup and by a catch-up timer. "
        "Before deep recall, campaign continuation, or after a handover, run the area gate "
        f"`ygg-memory sync --harness {canonical} --ns=<area>` first, then inspect status/diff and open only the relevant door "
        "with `ygg-memory get --file <name> --ns=<area>`. Publish durable findings through `ygg-memory publish`; "
        "never copy credentials, sessions, databases, indexes, or lock files between machines.\n\n"
        "This harness name is `autoclaw` when you are running inside AutoClaw (the z.ai desktop agent "
        "— its sessions snapshot this file as pi-user/AGENTS.md), or `pi` in the standalone pi CLI. "
        "Use the name that matches where you are running right now; each keeps its own watermark. "
        "The current project namespace is derived from the cwd.\n"
        f"{MANAGED_BLOCK_END}"
    )


def strip_managed_block(content: str) -> str:
    pattern = re.compile(
        rf"\n?{re.escape(MANAGED_BLOCK_BEGIN)}.*?{re.escape(MANAGED_BLOCK_END)}\n?",
        re.DOTALL,
    )
    return pattern.sub("\n", content).strip()


def write_managed_block(
    path: Path,
    harness: str,
    user_content: str | None = None,
    managed_block: str | None = None,
) -> bool:
    """Preserve user-owned text and replace only yggterm's delimited block."""
    existing = path.read_text(encoding="utf-8") if path.is_file() else ""
    base = strip_managed_block(existing) if user_content is None else user_content.strip()
    block = managed_block if managed_block is not None else memory_bridge(harness)
    # The blank-line boundary matches yggsteer's assemble(): a guarded part
    # ends with "\n" and parts join on "\n\n", so a stamped file carries three
    # newlines before a trailing foreign block. Rendering anything else made
    # steer check and this sync fight over one line, each rewriting the other.
    rendered = ((base + "\n\n\n") if base else "") + block + "\n"
    if rendered == existing:
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(rendered, encoding="utf-8")
    temp.replace(path)
    return True


def native_document_content(
    harness: str,
    label: str,
    payload: str,
    *,
    target_harness: str | None = None,
    native_path: str | None = None,
) -> str:
    target = target_harness or harness
    path_line = f"native_path: {urllib.parse.quote(native_path, safe='/._~-')}\n" if native_path else ""
    return (
        "---\n"
        f"name: native-{harness}-{label}\n"
        f"description: Native {harness} {label}, synchronized across the working fleet.\n"
        "type: user\n"
        f"target_harness: {target}\n"
        + path_line
        + "---\n\n"
        "<!-- yggterm-native-payload -->\n"
        + payload.rstrip()
        + "\n"
    )


def native_document_payload(content: str) -> str:
    marker = "<!-- yggterm-native-payload -->"
    return content.split(marker, 1)[1].lstrip("\n") if marker in content else content


def text_sha256(content: str) -> str:
    return hashlib.sha256(content.encode()).hexdigest()


def version_for_digest(root: Path, namespace: str, filename: str, digest: str | None) -> str | None:
    if not digest:
        return None
    found = None
    for record in read_journal_entries(root, namespace=namespace):
        if record.get("file") == filename and record.get("digest") == digest:
            found = record.get("version_id")
    return found


def _safe_native_relative(raw: str) -> Path | None:
    decoded = urllib.parse.unquote(raw)
    relative = Path(decoded)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        return None
    return relative


def _native_document_path(content: str) -> Path | None:
    match = re.search(r"^native_path:[ \t]*(.+)$", content, re.MULTILINE)
    return _safe_native_relative(match.group(1).strip()) if match else None


_native_door_causal_cache: dict = {}


def _door_causal_state(root: Path, ns: str, filename: str) -> tuple[str | None, str | None]:
    """(latest journal action, last upserted content digest) for one door.

    One lazy scan per (root, namespace) per process, invalidated per door
    after a local write. This is the causal-store consult the mirror
    projection owed: a door whose journal ends in a delete is RETIRED, and
    an unchanged source must not resurrect it on every sync tick (the
    2026-09-08..10 _global churn — ~100 re-creates fighting one resolved
    deletion).
    """
    ns_key = (str(root), ns)
    index = _native_door_causal_cache.get(ns_key)
    if index is None:
        index = {}
        for record in read_journal_entries(root, namespace=ns):
            fname = record.get("file")
            if not fname or not record.get("version_id"):
                continue
            if record.get("action") == "delete":
                index[fname] = ("delete", index.get(fname, (None, None))[1])
            elif record.get("digest"):
                index[fname] = ("upsert", record["digest"])
        _native_door_causal_cache[ns_key] = index
    return index.get(filename, (None, None))


def _door_causal_forget(root: Path, ns: str, filename: str) -> None:
    index = _native_door_causal_cache.get((str(root), ns))
    if index is not None:
        index.pop(filename, None)


def _is_causally_deleted_unchanged(root: Path, ns: str, filename: str, content_digest: str | None) -> bool:
    """True when the door is retired by the causal store AND the would-be
    upserted content is byte-identical to what was deleted — i.e. the only
    thing standing behind this write is the churn, not a real change."""
    if content_digest is None:
        return False
    latest_action, last_digest = _door_causal_state(root, ns, filename)
    return latest_action == "delete" and (last_digest is None or last_digest == content_digest)


def _sync_native_document(
    root: Path,
    harness: str,
    path: Path,
    namespace: str,
    filename: str,
    label: str,
    *,
    target_harness: str,
    managed_block: str | None,
    native_path: str | None = None,
) -> tuple[int, int, int]:
    """Three-way synchronize one sanctioned native Markdown document.

    ``managed_block=None`` means a pure semantic document: restore the exact
    payload without injecting a bridge. Instruction/MEMORY files pass a block
    that remains adapter-owned and is stripped before change detection.
    """
    hub = get_namespace_dir(root, namespace) / filename
    local_exists = path.is_file()
    raw_local = path.read_text(encoding="utf-8") if local_exists else ""
    local_payload = strip_managed_block(raw_local) if managed_block is not None else raw_local.rstrip()
    semantic_local_exists = local_exists if managed_block is None else bool(local_payload)
    hub_exists = hub.is_file()
    hub_payload = native_document_payload(hub.read_text(encoding="utf-8")).rstrip() if hub_exists else ""
    local_digest = text_sha256(local_payload) if semantic_local_exists else None
    hub_digest = text_sha256(hub_payload) if hub_exists else None
    # The door this native content would produce, and its journal-domain
    # digest (sha256 of the hub FILE), computed once for both the upsert
    # churn guard and the native-side deletion guard below.
    candidate_hub = (
        native_document_content(
            harness,
            label,
            local_payload,
            target_harness=target_harness,
            native_path=native_path,
        )
        if semantic_local_exists
        else None
    )
    candidate_hub_digest = text_sha256(candidate_hub) if candidate_hub is not None else None

    watermark = load_watermark(root, harness)
    state_key = f"{namespace}/{filename}"
    state = watermark.setdefault("native_documents", {}).setdefault(state_key, {})
    base_digest = state.get("delivered_payload_digest", state.get("delivered_digest"))
    base_version = state.get("delivered_version")
    ingested = delivered = deleted = 0

    def publish_local(causal_base: str | None) -> None:
        nonlocal ingested, deleted
        if semantic_local_exists:
            # The churn guard: a door the causal store retired stays retired
            # while the produced door content is byte-identical to what was
            # deleted. A real content change (a different digest than the
            # deleted one) is a genuine resurrection and passes.
            if _is_causally_deleted_unchanged(
                root, namespace, filename, candidate_hub_digest
            ):
                return
            hub.write_text(candidate_hub, encoding="utf-8")
            action = "upsert"
            ingested += 1
        else:
            if hub.exists():
                hub.unlink()
            action = "delete"
            deleted += 1
        append_journal_entry(
            root,
            namespace,
            filename,
            "user",
            action,
            harness,
            f"Native {harness} {label}",
            target_harness=target_harness,
            base_version=causal_base,
        )
        _door_causal_forget(root, namespace, filename)
        materialize_store(root)

    if base_digest is None:
        if semantic_local_exists and not hub_exists:
            publish_local(None)
        elif semantic_local_exists and hub_exists and local_digest != hub_digest:
            # Two pre-existing stores have no common delivery base. Keep both.
            publish_local(None)
    else:
        local_changed = (local_digest != base_digest) if semantic_local_exists else True
        hub_changed = (hub_digest != base_digest) if hub_exists else True
        if local_changed and not hub_changed:
            publish_local(base_version or version_for_digest(root, namespace, filename, file_sha256(hub) if hub_exists else None))
        elif local_changed and hub_changed and local_digest != hub_digest:
            # Both descend from the delivered base. The event graph retains two
            # heads and materialize_store makes the conflict observable.
            publish_local(base_version)

    hub_exists = hub.is_file()
    hub_content = hub.read_text(encoding="utf-8") if hub_exists else ""
    hub_payload = native_document_payload(hub_content).rstrip() if hub_exists else ""
    final_digest = text_sha256(hub_payload) if hub_exists else None
    if managed_block is None:
        rendered = hub_payload + ("\n" if hub_payload else "")
        existing = path.read_text(encoding="utf-8") if path.is_file() else None
        if hub_exists and existing != rendered:
            if path.is_file():
                backup_native_file(root, harness, namespace, path)
            path.parent.mkdir(parents=True, exist_ok=True)
            temp = path.with_suffix(path.suffix + ".tmp")
            temp.write_text(rendered, encoding="utf-8")
            temp.replace(path)
            delivered += 1
        elif not hub_exists and path.exists():
            # A retired door must not eat the native source: when the causal
            # store retired this door and the content is unchanged, the
            # native file is the only surviving copy — leave it alone.
            if _is_causally_deleted_unchanged(root, namespace, filename, candidate_hub_digest):
                pass
            else:
                backup_native_file(root, harness, namespace, path)
                path.unlink()
                delivered += 1
    else:
        before = path.read_text(encoding="utf-8") if path.is_file() else None
        write_managed_block(path, harness, hub_payload if hub_exists else "", managed_block)
        if before != path.read_text(encoding="utf-8"):
            delivered += 1

    if final_digest is None:
        state.clear()
    else:
        state["delivered_payload_digest"] = final_digest
        state["delivered_hub_digest"] = file_sha256(hub)
        state["delivered_version"] = version_for_digest(root, namespace, filename, state["delivered_hub_digest"])
    save_watermark(root, watermark)
    return ingested, delivered, deleted


def sync_instruction_document(root: Path, harness: str, path: Path) -> tuple[int, int, int]:
    """Synchronize one CLI-owned global instruction file without owning user text."""
    return _sync_native_document(
        root,
        harness,
        path,
        GLOBAL_NAMESPACE,
        f"native-{harness}-global-instructions.md",
        "global instructions",
        target_harness=harness,
        managed_block=memory_bridge(harness),
    )


def backup_native_file(root: Path, harness: str, namespace: str, path: Path) -> None:
    """Content-addressed safety copy only when an adapter will mutate a file."""
    if not path.is_file():
        return
    digest = file_sha256(path)
    # Subpath doors keep their directory prefix in the backup path so two
    # sub/sub.md and sub2/sub.md cannot evict each other's safety copies.
    target = root.parent / "memory-backups" / harness / namespace / path.parent.name / path.name / f"{digest}.md"
    if not target.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)


def _sync_native_tree(
    root: Path,
    harness: str,
    namespace: str,
    directory: Path,
    prefix: str,
    *,
    target_harness: str,
    exclude=None,
) -> tuple[int, int, int]:
    """Mirror semantic Markdown files while leaving indexes/state outside it."""
    exclude = exclude or (lambda _relative: False)
    local = {}
    if directory.is_dir():
        for candidate in directory.rglob("*.md"):
            relative = candidate.relative_to(directory)
            if not exclude(relative):
                local[relative.as_posix()] = candidate

    hub = {}
    hub_dir = get_namespace_dir(root, namespace)
    pattern = f"native-{harness}-{prefix}-*.md"
    for candidate in hub_dir.glob(pattern):
        relative = _native_document_path(candidate.read_text(encoding="utf-8", errors="replace"))
        if relative is not None and not exclude(relative):
            hub[relative.as_posix()] = candidate.name

    ingested = delivered = deleted = 0
    for relative in sorted(set(local) | set(hub)):
        safe_relative = _safe_native_relative(relative)
        if safe_relative is None:
            continue
        filename = hub.get(relative)
        if filename is None:
            slug = re.sub(r"[^a-z0-9]+", "-", safe_relative.stem.lower()).strip("-")[:40] or "memory"
            key = hashlib.sha256(relative.encode()).hexdigest()[:12]
            filename = f"native-{harness}-{prefix}-{key}-{slug}.md"
        inc, outc, delc = _sync_native_document(
            root,
            harness,
            directory / safe_relative,
            namespace,
            filename,
            f"{prefix} {relative}",
            target_harness=target_harness,
            managed_block=None,
            native_path=relative,
        )
        ingested += inc
        delivered += outc
        deleted += delc
    return ingested, delivered, deleted


def ingest_read_only_native_document(
    root: Path,
    harness: str,
    path: Path,
    filename: str,
    label: str,
    *,
    target_harness: str = "all",
) -> tuple[int, int, int]:
    """Export generated semantic output without ever writing it back directly."""
    watermark = load_watermark(root, harness)
    state = watermark.setdefault("read_only_native", {}).setdefault(filename, {})
    hub = get_namespace_dir(root, GLOBAL_NAMESPACE) / filename
    if path.is_file():
        payload = path.read_text(encoding="utf-8", errors="replace")
        digest = text_sha256(payload)
        if state.get("digest") != digest:
            hub.write_text(
                native_document_content(harness, label, payload, target_harness=target_harness),
                encoding="utf-8",
            )
            append_journal_entry(
                root,
                GLOBAL_NAMESPACE,
                filename,
                "user",
                "upsert",
                harness,
                f"Native {harness} {label}",
                target_harness=target_harness,
            )
            state["digest"] = digest
            save_watermark(root, watermark)
            return 1, 0, 0
    elif state:
        if hub.exists():
            hub.unlink()
        append_journal_entry(
            root,
            GLOBAL_NAMESPACE,
            filename,
            "user",
            "delete",
            harness,
            f"deleted native {harness} {label}",
            target_harness=target_harness,
        )
        state.clear()
        save_watermark(root, watermark)
        return 0, 0, 1
    return 0, 0, 0


class HarnessMemoryAdapter:
    """The one harness-specific owner of native-memory I/O."""

    name = "unknown"

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        """Return namespaces, ingested, delivered, deleted."""
        raise NotImplementedError

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        raise NotImplementedError


class ProjectMemoryAdapter(HarnessMemoryAdapter):
    """Adapter for CLIs whose native memory is one directory per project."""

    def __init__(self, harness: str, project_root: Path):
        self.name = harness
        self.project_root = project_root

    def local_dir(self, namespace: str) -> Path:
        return self.project_root / namespace / "memory"

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        return _sync_project_memory_namespace(root, harness, namespace, self.local_dir(namespace))

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        namespaces = set()
        for directory in (root / "namespaces").glob("*"):
            if directory.is_dir():
                namespaces.add(directory.name)
        if self.project_root.exists():
            for directory in self.project_root.glob("*/memory"):
                namespaces.add(directory.parent.name)
        ingested = delivered = deleted = 0
        for namespace in sorted(namespaces):
            inc, outc, delc = self.sync_namespace(root, harness, namespace)
            ingested += inc
            delivered += outc
            deleted += delc
        return len(namespaces), ingested, delivered, deleted


class InstructionBridgeAdapter(HarnessMemoryAdapter):
    """Backend for CLIs whose durable cross-session surface is instructions."""

    def __init__(self, harness: str, instruction_path: Path):
        self.name = harness
        self.instruction_path = instruction_path

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        return sync_instruction_document(root, self.name, self.instruction_path)

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        ingested, delivered, deleted = self.sync_namespace(root, harness, GLOBAL_NAMESPACE)
        return 1, ingested, delivered, deleted


class AntigravityMemoryAdapter(InstructionBridgeAdapter):
    """Antigravity always loads AGENTS.md plus each Markdown rule file."""

    def __init__(self, home: Path):
        super().__init__("antigravity", home / ".agents" / "AGENTS.md")
        self.rules_dir = home / ".agents" / "rules"

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        inc, outc, delc = sync_instruction_document(root, self.name, self.instruction_path)
        i2, o2, d2 = _sync_native_tree(
            root,
            self.name,
            GLOBAL_NAMESPACE,
            self.rules_dir,
            "rule",
            target_harness=self.name,
        )
        return inc + i2, outc + o2, delc + d2


class ClaudeMemoryAdapter(ProjectMemoryAdapter):
    def __init__(self, home: Path):
        super().__init__("claude", home / ".claude" / "projects")
        self.home = home
        self.instruction_path = home / ".claude" / "CLAUDE.md"

    def local_dir(self, namespace: str) -> Path:
        if namespace == GLOBAL_NAMESPACE:
            return self.home / ".claude" / "memory"
        return super().local_dir(namespace)

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        inc, outc, delc = _sync_project_memory_namespace(root, harness, namespace, self.local_dir(namespace))
        i2, o2, d2 = sync_instruction_document(root, self.name, self.instruction_path)
        return inc + i2, outc + o2, delc + d2

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        namespaces = {
            directory.name for directory in (root / "namespaces").glob("*") if directory.is_dir()
        }
        if self.project_root.exists():
            namespaces.update(directory.parent.name for directory in self.project_root.glob("*/memory"))
        namespaces.add(GLOBAL_NAMESPACE)
        ingested = delivered = deleted = 0
        for namespace in sorted(namespaces):
            inc, outc, delc = _sync_project_memory_namespace(
                root, harness, namespace, self.local_dir(namespace)
            )
            ingested += inc
            delivered += outc
            deleted += delc
        inc, outc, delc = sync_instruction_document(root, self.name, self.instruction_path)
        return len(namespaces), ingested + inc, delivered + outc, deleted + delc


class MuseMemoryAdapter(ProjectMemoryAdapter):
    """Muse native memory lives under the XDG data dir.

    Personal and personal-project scopes are Claude-compatible Markdown in
    ``~/.local/share/muse/memory/projects/<slug>/``, where ``<slug>`` is the
    workspace path slug plus a 16-hex disambiguator (``home-user-proj``
    plus hash for ``/home/user/proj``). The suffix cannot be derived from
    the namespace, so it is resolved by directory scan; namespaces with zero
    or ambiguous matches are skipped — never synced against a guessed path,
    and never against another harness's store. ``--local-dir`` remains an
    explicit operator override that bypasses resolution.
    """

    SLUG_SUFFIX_RE = re.compile(r"-[0-9a-f]{16}$")

    def __init__(self, home: Path):
        super().__init__("muse", home / ".local" / "share" / "muse" / "memory" / "projects")
        self.home = home

    def resolve_slug_dir(self, namespace: str) -> Path | None:
        core = namespace[1:] if namespace.startswith("-") else namespace
        if not core or not self.project_root.is_dir():
            return None
        matches = [
            d for d in self.project_root.iterdir()
            if d.is_dir()
            and (m := self.SLUG_SUFFIX_RE.search(d.name)) is not None
            and d.name[: m.start()] == core
        ]
        return matches[0] if len(matches) == 1 else None

    def local_dir(self, namespace: str) -> Path | None:
        if namespace == GLOBAL_NAMESPACE:
            return None
        return self.resolve_slug_dir(namespace)

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        native = self.local_dir(namespace)
        if native is None:
            return 0, 0, 0
        return _sync_project_memory_namespace(root, harness, namespace, native)

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        namespaces = {d.name for d in (root / "namespaces").glob("*") if d.is_dir()}
        if self.project_root.is_dir():
            for d in self.project_root.iterdir():
                if d.is_dir() and (m := self.SLUG_SUFFIX_RE.search(d.name)) is not None:
                    namespaces.add("-" + d.name[: m.start()])
        ingested = delivered = deleted = synced = 0
        for namespace in sorted(namespaces):
            if self.local_dir(namespace) is None:
                continue
            inc, outc, delc = self.sync_namespace(root, harness, namespace)
            synced += 1
            ingested += inc
            delivered += outc
            deleted += delc
        return synced, ingested, delivered, deleted


class KimiMemoryAdapter(HarnessMemoryAdapter):
    """Kimi has no self-memory DB; user skills are its safe global bridge."""

    name = "kimi"

    def __init__(self, home: Path):
        self.skill_path = home / ".kimi" / "skills" / "yggterm-memory" / "SKILL.md"

    def _ensure_skill(self) -> int:
        content = (
            "---\n"
            "name: yggterm-memory\n"
            "description: Mandatory fleet-memory door for startup, recall, handovers, and durable learning; use ygg-memory before claiming context is unavailable.\n"
            "---\n\n"
            "# Yggterm fleet memory\n\n"
            + memory_bridge("kimi")
        )
        if self.skill_path.is_file() and self.skill_path.read_text(encoding="utf-8") == content:
            return 0
        self.skill_path.parent.mkdir(parents=True, exist_ok=True)
        self.skill_path.write_text(content, encoding="utf-8")
        return 1

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        return 0, self._ensure_skill(), 0

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        return 1, 0, self._ensure_skill(), 0


class QwenMemoryAdapter(HarnessMemoryAdapter):
    """Qwen Code 0.21.14 managed auto-memory: native topic docs + rebuilt index."""

    name = "qwen"

    def __init__(self, home: Path, cwd: Path):
        self.home = home
        self.cwd = cwd.resolve()
        self.instruction_path = home / ".qwen" / "QWEN.md"

    @staticmethod
    def _git_root(cwd: Path) -> Path:
        current = cwd.resolve()
        while current.parent != current:
            if (current / ".git").exists():
                return current
            current = current.parent
        return cwd.resolve()

    @staticmethod
    def _sanitize_cwd(path: Path) -> str:
        return re.sub(r"[^A-Za-z0-9]", "-", str(path))

    def project_memory_dir(self, cwd: Path | None = None) -> Path:
        project = self._git_root((cwd or self.cwd).resolve())
        return self.home / ".qwen" / "projects" / self._sanitize_cwd(project) / "memory"

    def user_memory_dir(self) -> Path:
        return self.home / ".qwen" / "memories"

    @staticmethod
    def _qwen_type(kind: str) -> str:
        if kind in {"user", "feedback", "project", "reference"}:
            return kind
        if kind in {"campaign", "spec"}:
            return "project"
        return "reference"

    def _project(self, root: Path, harness: str, namespace: str, memory_dir: Path) -> tuple[int, int, int]:
        ns_dir = get_namespace_dir(root, namespace)
        target = memory_dir / "pinned" / "yggterm"
        target.mkdir(parents=True, exist_ok=True)
        expected = set()
        delivered = deleted = 0
        for door in sorted(ns_dir.glob("*.md")):
            if door.name == "MEMORY.md":
                continue
            original = door.read_text(encoding="utf-8")
            kind, summary, target_h = extract_metadata_and_summary(original, door.name)
            if not matches_target_harness(target_h, harness):
                continue
            expected.add(door.name)
            projected = (
                "---\n"
                f"type: {self._qwen_type(kind)}\n"
                f"name: {Path(door.name).stem}\n"
                f"description: {summary.replace(chr(10), ' ')[:120]}\n"
                "---\n\n"
                + original
            )
            dest = target / door.name
            if not dest.is_file() or dest.read_text(encoding="utf-8") != projected:
                dest.write_text(projected, encoding="utf-8")
                delivered += 1
        for old in target.glob("*.md"):
            if old.name not in expected:
                old.unlink()
                deleted += 1
        self._rebuild_index(memory_dir)
        return 0, delivered, deleted

    @staticmethod
    def _frontmatter_value(content: str, key: str) -> str:
        match = re.search(rf"^{re.escape(key)}:[ \t]*(.+)$", content, re.MULTILINE)
        return match.group(1).strip().strip('"\'') if match else ""

    def _rebuild_index(self, memory_dir: Path) -> None:
        memory_dir.mkdir(parents=True, exist_ok=True)
        lines = []
        for doc in sorted(memory_dir.rglob("*.md")):
            if doc.name == "MEMORY.md":
                continue
            content = doc.read_text(encoding="utf-8", errors="replace")
            if not content.startswith("---\n"):
                continue
            kind = self._frontmatter_value(content, "type")
            if kind not in {"user", "feedback", "project", "reference"}:
                continue
            title = self._frontmatter_value(content, "name") or kind
            description = self._frontmatter_value(content, "description") or kind
            rel = doc.relative_to(memory_dir).as_posix()
            rel = urllib.parse.quote(rel, safe="/._~-")
            line = f"- [{title}]({rel}) — {description}".replace("\n", " ")
            lines.append(line[:149] + "…" if len(line) > 150 else line)
        rendered = "\n".join(lines[:200])
        index = memory_dir / "MEMORY.md"
        if not index.is_file() or index.read_text(encoding="utf-8") != rendered:
            index.write_text(rendered, encoding="utf-8")

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        memory_dir = self.project_memory_dir()
        inc, outc, delc = _sync_native_tree(
            root,
            harness,
            namespace,
            memory_dir,
            "project",
            target_harness="all",
            exclude=lambda relative: relative.name == "MEMORY.md" or relative.parts[:2] == ("pinned", "yggterm"),
        )
        i1, o1, d1 = self._project(root, harness, namespace, memory_dir)
        inc, outc, delc = inc + i1, outc + o1, delc + d1
        i0, o0, d0 = _sync_native_tree(
            root,
            harness,
            GLOBAL_NAMESPACE,
            self.user_memory_dir(),
            "user",
            target_harness="all",
            exclude=lambda relative: relative.name == "MEMORY.md" or relative.parts[:2] == ("pinned", "yggterm"),
        )
        i3, o3, d3 = self._project(root, harness, GLOBAL_NAMESPACE, self.user_memory_dir())
        i2, o2, d2 = sync_instruction_document(root, self.name, self.instruction_path)
        return inc + i0 + i2 + i3, outc + o0 + o2 + o3, delc + d0 + d2 + d3

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        pairs = {detect_namespace(self.cwd): self.project_memory_dir()}
        hub_namespaces = [path.name for path in (root / "namespaces").glob("*") if path.is_dir()]
        for memory_dir in (self.home / ".qwen" / "projects").glob("*/memory"):
            for namespace in hub_namespaces:
                if self._sanitize_cwd(Path(namespace)) == memory_dir.parent.name:
                    pairs[namespace] = memory_dir
                    break
        ingested = delivered = deleted = 0
        for namespace, memory_dir in sorted(pairs.items()):
            inc, outc, delc = _sync_native_tree(
                root,
                harness,
                namespace,
                memory_dir,
                "project",
                target_harness="all",
                exclude=lambda relative: relative.name == "MEMORY.md" or relative.parts[:2] == ("pinned", "yggterm"),
            )
            i1, o1, d1 = self._project(root, harness, namespace, memory_dir)
            inc, outc, delc = inc + i1, outc + o1, delc + d1
            ingested += inc
            delivered += outc
            deleted += delc
        i0, o0, d0 = _sync_native_tree(
            root,
            harness,
            GLOBAL_NAMESPACE,
            self.user_memory_dir(),
            "user",
            target_harness="all",
            exclude=lambda relative: relative.name == "MEMORY.md" or relative.parts[:2] == ("pinned", "yggterm"),
        )
        i3, o3, d3 = self._project(root, harness, GLOBAL_NAMESPACE, self.user_memory_dir())
        i2, o2, d2 = sync_instruction_document(root, self.name, self.instruction_path)
        return len(pairs) + 1, ingested + i0 + i2 + i3, delivered + o0 + o2 + o3, deleted + d0 + d2 + d3


# ⛔ [mem-dream slice 1, 2026-10-06] THE INDEX IS A DOOR, AND THE DOOR WAS
# BRICKED: the zcode index rebuilt from EVERY *.md under the memory tree, so
# ~550 legacy per-sitting archive files (dated/sitting/session/wave-N/
# delivery/-history names — most written top-level before the archive/
# convention existed) made MEMORY.md 906 lines / 176.7KB, past the harness
# loader's partial-load cut: index lines past the cut NEVER loaded and their
# doors were unreachable through the index while fully present on disk.
# Shelving is now ONE classifier shared by the index split and the
# shelve-archives verb (never a second encoding): shelved docs leave the
# injected MEMORY.md for ARCHIVE-INDEX.md (never auto-loaded); living doors
# keep the byte-identical line format.
_SHELVED_DOC_TYPES = {"sitting-archive", "archive"}
_SHELVED_NAME_RE = re.compile(r"(sitting|session|wave-\d+|delivery|-history)", re.IGNORECASE)
_DATED_NAME_RE = re.compile(r"-20\d{2}-\d{2}-\d{2}")


def _is_shelved_doc(relative_parts, name: str, doc_type: str) -> bool:
    # archive-in-parts is a STRAY catcher: the canonical home is
    # <project>/memory-archive/ OUTSIDE the scanned memory tree (the zcode
    # client regenerates MEMORY.md with rglob-all on its own schedule —
    # measured 2026-10-06 — so anything in-tree re-enters the index no
    # matter what this tool does).
    if "archive" in relative_parts:
        return True
    if doc_type.strip().lower() in _SHELVED_DOC_TYPES:
        return True
    if _SHELVED_NAME_RE.search(name):
        return True
    return bool(_DATED_NAME_RE.search(name))


class ZcodeMemoryAdapter(HarnessMemoryAdapter):
    """Zcode auto-memory under ``~/.zcode/cli/memories/projects`` plus an AGENTS.md bridge.

    The zcode project slug (``default-<hash>``) is opaque — never derivable
    from a cwd — so every discovered ``projects/*/memory`` dir carries the
    global namespace. Hub doors are delivered into ``pinned/yggterm/``; native
    memories live flat at the top level and ingest through the native tree.
    MEMORY.md is zcode's always-loaded one-line-per-door index: rebuilt from
    the files themselves, never synced.
    """

    name = "zcode"

    def __init__(self, home: Path):
        self.home = home
        self.projects_root = home / ".zcode" / "cli" / "memories" / "projects"
        self.instruction_path = home / ".zcode" / "AGENTS.md"

    def memory_dirs(self) -> list:
        if not self.projects_root.is_dir():
            return []
        return sorted(
            directory / "memory"
            for directory in self.projects_root.iterdir()
            if directory.is_dir() and (directory / "memory").is_dir()
        )

    @staticmethod
    def _frontmatter_block(content: str) -> str:
        match = re.match(r"^---\n(.*?)\n---\n", content, re.DOTALL)
        return match.group(1) if match else ""

    @classmethod
    def _meta(cls, content: str, key: str) -> str:
        # zcode-native files nest type under `metadata:`; hub doors keep it
        # flat — the indented-or-not match covers both. YAML folds long
        # scalars across indented continuation lines; a continuation is
        # indented text that is not itself a `key:` line.
        block = cls._frontmatter_block(content)
        match = re.search(rf"^[ \t]*{re.escape(key)}:[ \t]*(.*)$", block, re.MULTILINE)
        if not match:
            return ""
        parts = [match.group(1).strip()]
        rest = block[match.end():]
        if rest.startswith("\n"):
            rest = rest[1:]
        for line in rest.splitlines():
            if not re.match(r"^[ \t]+\S", line) or re.match(r"^[ \t]+[A-Za-z_][A-Za-z0-9_]*:(\s|$)", line):
                break
            parts.append(line.strip())
        value = " ".join(part for part in parts if part)
        return value.strip("\"'").replace('\\"', '"')

    def _rebuild_index(self, memory_dir: Path) -> None:
        memory_dir.mkdir(parents=True, exist_ok=True)
        living = ["# Memory index", ""]
        shelved = ["# Archive index — shelved docs, never auto-loaded", ""]
        for doc in sorted(memory_dir.rglob("*.md")):
            if doc.name in ("MEMORY.md", "ARCHIVE-INDEX.md"):
                continue
            content = doc.read_text(encoding="utf-8", errors="replace")
            if not content.startswith("---\n"):
                continue
            title = self._meta(content, "name") or doc.stem
            description = self._meta(content, "description") or title
            rel = urllib.parse.quote(doc.relative_to(memory_dir).as_posix(), safe="/._~-")
            line = f"- [{title}]({rel}) — {description}".replace("\n", " ")
            line = line[:199] + "…" if len(line) > 200 else line
            if _is_shelved_doc(
                doc.relative_to(memory_dir).parts, doc.name, self._meta(content, "type")
            ):
                shelved.append(line)
            else:
                living.append(line)
        rendered = "\n".join(living) + "\n"
        index = memory_dir / "MEMORY.md"
        if not index.is_file() or index.read_text(encoding="utf-8") != rendered:
            index.write_text(rendered, encoding="utf-8")
        external_archive = memory_dir.parent / "memory-archive"
        if external_archive.is_dir():
            count = sum(1 for _ in external_archive.glob("*.md"))
            shelved.append(
                f"- [shelved archives](../memory-archive/) — {count} docs, "
                "per-host native history, never synced, never auto-loaded"
            )
        archive_index = memory_dir / "ARCHIVE-INDEX.md"
        archive_rendered = "\n".join(shelved) + "\n"
        if not archive_index.is_file() or archive_index.read_text(
            encoding="utf-8"
        ) != archive_rendered:
            archive_index.write_text(archive_rendered, encoding="utf-8")

    def _project(self, root: Path, harness: str, memory_dir: Path) -> tuple:
        ns_dir = get_namespace_dir(root, GLOBAL_NAMESPACE)
        target = memory_dir / "pinned" / "yggterm"
        target.mkdir(parents=True, exist_ok=True)
        expected = set()
        delivered = deleted = 0
        for door in sorted(ns_dir.glob("*.md")):
            if door.name == "MEMORY.md":
                continue
            kind, summary, door_target = extract_metadata_and_summary(door.read_text(encoding="utf-8"), door.name)
            if not matches_target_harness(door_target, harness):
                continue
            expected.add(door.name)
            dest = target / door.name
            content = door.read_text(encoding="utf-8")
            if not dest.is_file() or dest.read_text(encoding="utf-8") != content:
                dest.write_text(content, encoding="utf-8")
                delivered += 1
        for old in target.glob("*.md"):
            if old.name not in expected:
                old.unlink()
                deleted += 1
        ingested, outc, delc = _sync_native_tree(
            root,
            harness,
            GLOBAL_NAMESPACE,
            memory_dir,
            "memory",
            target_harness="all",
            exclude=lambda relative: relative.name in ("MEMORY.md", "ARCHIVE-INDEX.md")
            or "archive" in relative.parts
            or relative.parts[:2] == ("pinned", "yggterm"),
        )
        self._rebuild_index(memory_dir)
        return ingested, delivered + outc, deleted + delc

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple:
        # The zcode store is global; a per-cwd namespace has no native home here.
        ingested = delivered = deleted = 0
        for memory_dir in self.memory_dirs():
            i, o, d = self._project(root, harness, memory_dir)
            ingested += i
            delivered += o
            deleted += d
        i2, o2, d2 = sync_instruction_document(root, harness, self.instruction_path)
        return ingested + i2, delivered + o2, deleted + d2

    def sync_all(self, root: Path, harness: str) -> tuple:
        memory_dirs = self.memory_dirs()
        ingested = delivered = deleted = 0
        for memory_dir in memory_dirs:
            i, o, d = self._project(root, harness, memory_dir)
            ingested += i
            delivered += o
            deleted += d
        i2, o2, d2 = sync_instruction_document(root, harness, self.instruction_path)
        return max(1, len(memory_dirs)), ingested + i2, delivered + o2, deleted + d2


class AutoclawMemoryAdapter(HarnessMemoryAdapter):
    """AutoClaw (z.ai desktop, pi-runtime) native memory per project workspace.

    AutoClaw reads exactly one root steer file, ``~/.pi/agent/AGENTS.md``
    (verified via its pi-resources manifest, 2026-10-16), and has no
    auto-injected memory index — the steer law makes reading the workspace
    index deliberate. Unlike zcode's opaque project slugs, an AutoClaw
    workspace is a real path, so each project's ``memory/`` dir maps to its
    cwd-derived namespace (claude-style per-project ns, not zcode's
    forced-global). Native memories ingest through the standard
    project-memory sync with the usual exclusions; ``MEMORY.md`` /
    ``ARCHIVE-INDEX.md`` are rebuilt from the files, never synced. Every
    sync touches the dream heartbeat so the dreamer's quiescence gate sees
    AutoClaw activity.
    """

    name = "autoclaw"

    HEARTBEAT_REL = Path(".yggterm/config/dream/autoclaw-heartbeat")

    def __init__(self, home: Path):
        self.home = home
        self.instruction_path = home / ".pi" / "agent" / "AGENTS.md"

    def workspaces(self) -> list:
        """AutoClaw project workspaces on this host (discovery, no hardcode)."""
        found = []
        for proj_dir in sorted((self.home / ".config").glob("AutoClaw*/accounts/*/projects/*")):
            if proj_dir.is_dir():
                found.append(proj_dir)
        return found

    @staticmethod
    def namespace_for(workspace: Path) -> str:
        return detect_namespace(workspace)

    def _touch_heartbeat(self) -> None:
        # The heartbeat can never fail a sync.
        try:
            hb = self.home / self.HEARTBEAT_REL
            hb.parent.mkdir(parents=True, exist_ok=True)
            hb.touch()
        except OSError:
            pass

    def local_dir(self, workspace: Path) -> Path:
        return workspace / "memory"

    def _project(self, root: Path, harness: str, workspace: Path) -> tuple:
        self._touch_heartbeat()
        ns = self.namespace_for(workspace)
        ing, outc, delc = _sync_project_memory_namespace(root, harness, ns, self.local_dir(workspace))
        i2, o2, d2 = sync_instruction_document(root, harness, self.instruction_path)
        return ing + i2, outc + o2, delc + d2

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple:
        ing, outc, delc = sync_instruction_document(root, harness, self.instruction_path)
        for workspace in self.workspaces():
            if self.namespace_for(workspace) != namespace:
                continue
            i, o, d = self._project(root, harness, workspace)
            ing += i
            outc += o
            delc += d
        return ing, outc, delc

    def sync_all(self, root: Path, harness: str) -> tuple:
        workspaces = self.workspaces()
        ing, outc, delc = 0, 0, 0
        for workspace in workspaces:
            i, o, d = self._project(root, harness, workspace)
            ing += i
            outc += o
            delc += d
        i2, o2, d2 = sync_instruction_document(root, harness, self.instruction_path)
        return max(1, len(workspaces)), ing + i2, outc + o2, delc + d2


class GrokMemoryAdapter(HarnessMemoryAdapter):
    """Grok 1.0.5 native MEMORY.md; SQLite/session state remains CLI-owned."""

    name = "grok"

    def __init__(self, home: Path, cwd: Path):
        self.home = home
        self.cwd = cwd.resolve()
        self.memory_root = home / ".grok" / "memory"
        self.config_path = home / ".grok" / "config.toml"

    def _enable(self) -> None:
        content = self.config_path.read_text(encoding="utf-8") if self.config_path.is_file() else ""
        section = re.search(r"(?ms)^\[memory\]\n(.*?)(?=^\[|\Z)", content)
        if section:
            body = section.group(1)
            if re.search(r"(?m)^enabled\s*=", body):
                body = re.sub(r"(?m)^enabled\s*=.*$", "enabled = true", body)
            else:
                body = "enabled = true\n" + body
            updated = content[: section.start(1)] + body + content[section.end(1) :]
        else:
            updated = content.rstrip() + ("\n\n" if content.strip() else "") + "[memory]\nenabled = true\n"
        if updated != content:
            self.config_path.parent.mkdir(parents=True, exist_ok=True)
            self.config_path.write_text(updated, encoding="utf-8")

    def project_memory_file(self, cwd: Path | None = None) -> Path | None:
        expected = str((cwd or self.cwd).resolve())
        if not self.memory_root.is_dir():
            return None
        for candidate in self.memory_root.glob("*/MEMORY.md"):
            try:
                first = candidate.read_text(encoding="utf-8", errors="replace").splitlines()[0]
            except (OSError, IndexError):
                continue
            if first == f"# Project Memory — {expected}":
                return candidate
        return None

    def _sync_memory_file(self, root: Path, harness: str, namespace: str, path: Path, label: str) -> tuple[int, int, int]:
        ns_dir = get_namespace_dir(root, namespace)
        lines = []
        for door in sorted(ns_dir.glob("*.md")):
            if door.name in {"MEMORY.md", f"grok-native-{label}.md"}:
                continue
            content = door.read_text(encoding="utf-8")
            _, summary, target_h = extract_metadata_and_summary(content, door.name)
            if matches_target_harness(target_h, harness):
                lines.append(f"- {summary} — {door}")
        bridge = memory_bridge("grok").replace(
            MANAGED_BLOCK_END,
            ("\n\n### Current fleet doors\n\n" + "\n".join(lines[:200]) if lines else "") + f"\n{MANAGED_BLOCK_END}",
        )
        return _sync_native_document(
            root,
            harness,
            path,
            namespace,
            f"grok-native-{label}.md",
            label,
            target_harness="all",
            managed_block=bridge,
        )

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        self._enable()
        global_file = self.memory_root / "MEMORY.md"
        inc, outc, delc = self._sync_memory_file(root, harness, GLOBAL_NAMESPACE, global_file, "global-memory")
        project = self.project_memory_file()
        if project is not None:
            i2, o2, d2 = self._sync_memory_file(root, harness, namespace, project, "project-memory")
            inc, outc, delc = inc + i2, outc + o2, delc + d2
        return inc, outc, delc

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        self._enable()
        ingested = delivered = deleted = 0
        i1, o1, d1 = self._sync_memory_file(
            root, harness, GLOBAL_NAMESPACE, self.memory_root / "MEMORY.md", "global-memory"
        )
        ingested += i1
        delivered += o1
        deleted += d1
        count = 1
        if self.memory_root.is_dir():
            for candidate in sorted(self.memory_root.glob("*/MEMORY.md")):
                try:
                    first = candidate.read_text(encoding="utf-8", errors="replace").splitlines()[0]
                except (OSError, IndexError):
                    continue
                prefix = "# Project Memory — "
                if not first.startswith(prefix):
                    continue
                project = Path(first[len(prefix) :]).expanduser()
                namespace = detect_namespace(project)
                inc, outc, delc = self._sync_memory_file(
                    root, harness, namespace, candidate, "project-memory"
                )
                ingested += inc
                delivered += outc
                deleted += delc
                count += 1
        return count, ingested, delivered, deleted


class GeminiMemoryAdapter(ProjectMemoryAdapter):
    """Google Gemini CLI private memory; distinct from Antigravity's brain."""

    def __init__(self, home: Path, cwd: Path):
        self.home = home
        self.cwd = cwd.resolve()
        self.instruction_path = home / ".gemini" / "GEMINI.md"
        super().__init__("gemini", home / ".gemini" / "tmp")

    def local_dir(self, namespace: str) -> Path:
        registry = self.home / ".gemini" / "projects.json"
        if registry.is_file():
            try:
                mapping = json.loads(registry.read_text(encoding="utf-8")).get("projects", {})
                slug = mapping.get(str(self.cwd))
                if slug:
                    return self.project_root / slug / "memory"
            except (OSError, ValueError, AttributeError):
                pass
        for marker in self.project_root.glob("*/.project_root"):
            try:
                if Path(marker.read_text(encoding="utf-8").strip()).resolve() == self.cwd:
                    return marker.parent / "memory"
            except OSError:
                continue
        # Gemini 0.49 migrates this legacy SHA-256 directory into its claimed
        # slug during Storage.initialize(); preflight can safely seed it first.
        return self.project_root / hashlib.sha256(str(self.cwd).encode()).hexdigest() / "memory"

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        inc, outc, delc = _sync_project_memory_namespace(root, harness, namespace, self.local_dir(namespace))
        i2, o2, d2 = sync_instruction_document(root, self.name, self.instruction_path)
        return inc + i2, outc + o2, delc + d2

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        pairs = {detect_namespace(self.cwd): self.local_dir(detect_namespace(self.cwd))}
        registry = self.home / ".gemini" / "projects.json"
        if registry.is_file():
            try:
                mapping = json.loads(registry.read_text(encoding="utf-8")).get("projects", {})
                for project, slug in mapping.items():
                    pairs[detect_namespace(Path(project))] = self.project_root / slug / "memory"
            except (OSError, ValueError, AttributeError):
                pass
        for marker in self.project_root.glob("*/.project_root"):
            try:
                project = Path(marker.read_text(encoding="utf-8").strip())
            except OSError:
                continue
            pairs[detect_namespace(project)] = marker.parent / "memory"
        ingested = delivered = deleted = 0
        for namespace, memory_dir in sorted(pairs.items()):
            inc, outc, delc = _sync_project_memory_namespace(root, harness, namespace, memory_dir)
            ingested += inc
            delivered += outc
            deleted += delc
        i2, o2, d2 = sync_instruction_document(root, self.name, self.instruction_path)
        return len(pairs), ingested + i2, delivered + o2, deleted + d2


class CodexMemoryAdapter(HarnessMemoryAdapter):
    """Deliver unified doors through Codex's supported ad-hoc update inbox.

    Codex's ``memories/`` directory is generated runtime state, not a project
    memory store.  Writing or rsyncing it bypasses the Codex memory ingestor and
    can overwrite another host's generated state.  The inbox is append-only, so
    changed and deleted fleet doors are represented as explicit update notes.
    """

    name = "codex"

    def __init__(self, home: Path | None = None):
        codex_home = Path(os.environ.get("CODEX_HOME", (home or Path.home()) / ".codex"))
        self.memory_root = codex_home / "memories"
        self.notes_dir = codex_home / "memories" / "extensions" / "ad_hoc" / "notes"
        self.instruction_path = codex_home / "AGENTS.md"

    def _export_generated_summary(self, root: Path) -> tuple[int, int, int]:
        # The summary is Codex's compact durable semantic output. MEMORY.md is
        # a machine-local registry of rollout paths and is intentionally not
        # exported without its session artifacts.
        origin = memory_origin(root).replace("-", "")[:12]
        return ingest_read_only_native_document(
            root,
            self.name,
            self.memory_root / "memory_summary.md",
            f"native-codex-{origin}-memory-summary.md",
            "generated memory summary",
            target_harness="all",
        )

    @staticmethod
    def _slug(value: str, limit: int = 72) -> str:
        return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")[:limit] or "door"

    def _write_note(self, namespace: str, filename: str, digest: str, action: str, content: str = "") -> None:
        self.notes_dir.mkdir(parents=True, exist_ok=True)
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        stem = self._slug(f"fleet-{namespace}-{Path(filename).stem}")
        note = self.notes_dir / f"{stamp}-{stem}-{digest[:12]}.md"
        suffix = 1
        while note.exists():
            note = self.notes_dir / f"{stamp}-{stem}-{digest[:12]}-{suffix}.md"
            suffix += 1
        operation = "Delete" if action == "delete" else "Add or update"
        body = (
            f"# {operation} fleet memory door\n\n"
            f"- Source: yggterm unified fleet memory\n"
            f"- Namespace: `{namespace}`\n"
            f"- Door: `{filename}`\n"
            f"- SHA-256: `{digest}`\n\n"
        )
        if action == "delete":
            body += "Remove the previously imported door with this source identity.\n"
        else:
            body += "## Door content\n\n" + content
        note.write_text(body, encoding="utf-8")

    def _deliver_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        ns_dir = get_namespace_dir(root, namespace)
        watermark = load_watermark(root, harness)
        state = watermark.setdefault("native_delivery", {}).setdefault("codex", {}).setdefault(namespace, {})
        delivered = deleted = 0
        present = {}
        for door in ns_dir.glob("*.md"):
            if door.name == "MEMORY.md":
                continue
            content = door.read_text(encoding="utf-8")
            _, _, target = extract_metadata_and_summary(content, door.name)
            if not matches_target_harness(target, harness):
                continue
            digest = file_sha256(door)
            present[door.name] = digest
            if state.get(door.name) != digest:
                self._write_note(namespace, door.name, digest, "upsert", content)
                state[door.name] = digest
                delivered += 1
        for filename, digest in list(state.items()):
            if filename not in present:
                self._write_note(namespace, filename, digest, "delete")
                del state[filename]
                deleted += 1
        watermark["last_seq"] = get_latest_seq(root)
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)
        return 0, delivered, deleted

    def sync_namespace(self, root: Path, harness: str, namespace: str) -> tuple[int, int, int]:
        native_in, native_out, native_del = self._export_generated_summary(root)
        inc, outc, delc = self._deliver_namespace(root, harness, namespace)
        bridge_in, bridge_out, bridge_del = sync_instruction_document(root, self.name, self.instruction_path)
        return (
            native_in + inc + bridge_in,
            native_out + outc + bridge_out,
            native_del + delc + bridge_del,
        )

    def sync_all(self, root: Path, harness: str) -> tuple[int, int, int, int]:
        native_in, native_out, native_del = self._export_generated_summary(root)
        namespaces = sorted(directory.name for directory in (root / "namespaces").glob("*") if directory.is_dir())
        ingested, delivered, deleted = native_in, native_out, native_del
        for namespace in namespaces:
            inc, outc, delc = self._deliver_namespace(root, harness, namespace)
            ingested += inc
            delivered += outc
            deleted += delc
        inc, outc, delc = sync_instruction_document(root, self.name, self.instruction_path)
        return len(namespaces), ingested + inc, delivered + outc, deleted + delc


def get_harness_adapter(harness: str, home: Path | None = None, cwd: Path | None = None) -> HarnessMemoryAdapter:
    """Return an explicit native backend; never infer one from a path pattern."""
    normalized = normalize_harness_name(harness)
    home = (home or Path.home()).resolve()
    cwd = (cwd or Path.cwd()).resolve()
    if normalized == "codex":
        return CodexMemoryAdapter(home)
    if normalized == "claude":
        return ClaudeMemoryAdapter(home)
    if normalized == "muse":
        return MuseMemoryAdapter(home)
    if normalized == "antigravity":
        return AntigravityMemoryAdapter(home)
    if normalized == "opencode":
        return InstructionBridgeAdapter("opencode", home / ".config" / "opencode" / "AGENTS.md")
    if normalized == "pi":
        return InstructionBridgeAdapter("pi", home / ".pi" / "agent" / "AGENTS.md")
    if normalized == "kimi":
        return KimiMemoryAdapter(home)
    if normalized == "qwen":
        return QwenMemoryAdapter(home, cwd)
    if normalized == "grok":
        return GrokMemoryAdapter(home, cwd)
    if normalized == "gemini":
        return GeminiMemoryAdapter(home, cwd)
    if normalized == "zcode":
        return ZcodeMemoryAdapter(home)
    if normalized == "autoclaw":
        return AutoclawMemoryAdapter(home)
    raise ValueError(f"Unsupported harness memory backend: {normalized}")


def _flock_open(lock_path: Path, timeout_seconds: float = 10):
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    f = open(lock_path, "a+")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        # Native startup waits briefly; fleet import/migration can explicitly
        # wait longer for a native all-pass without holding any network lock.
        attempts = max(1, int(timeout_seconds / 0.5))
        for _ in range(attempts):
            time.sleep(0.5)
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                pass
        else:
            raise RuntimeError(
                f"Lock held by another process: {lock_path} "
                f"(waited {timeout_seconds:.0f}s — another ygg-memory verb is "
                "mid-run; retry shortly, or let the catch-up tick cover it)"
            )
    return f


def _flock_close(f):
    if f:
        try:
            fcntl.flock(f, fcntl.LOCK_UN)
            f.close()
        except Exception:
            pass


def normalize_harness_name(name: str) -> str:
    if not name:
        return "unknown"
    n = name.strip().lower()
    if n in ("agy", "antigravity", "antigravity-cli"):
        return "antigravity"
    if n in ("gemini", "gemini-cli", "gemini_cli"):
        return "gemini"
    if n in ("cc", "claude_code", "claude"):
        return "claude"
    if n in ("codex-litellm", "codex_litellm"):
        return "codex"
    if n in ("grok-build", "grok_build", "grok"):
        return "grok"
    if n in ("muse-code", "muse_code", "muse"):
        return "muse"
    if n in ("qwen-code", "qwen_code", "qwen"):
        return "qwen"
    if n in ("open-code", "open_code", "opencode"):
        return "opencode"
    if n in ("kimi-code", "kimi_code", "kimi"):
        return "kimi"
    if n in ("pi-coding-agent", "pi_coding_agent", "pi"):
        return "pi"
    if n in ("zcode-cli", "zcode_cli", "zcode"):
        return "zcode"
    if n in ("autoclaw", "autoclaw-cli", "autoclaw_cli", "autoclaw-desktop"):
        return "autoclaw"
    return n


def detect_harness(override: str = None) -> str:
    if override:
        return normalize_harness_name(override)
    if os.environ.get("CLAUDE_PROJECT_DIR") or os.environ.get("CLAUDE_SESSION_ID"):
        return "claude"
    if os.environ.get("ANTIGRAVITY_SESSION"):
        return "antigravity"
    if os.environ.get("GEMINI_CLI"):
        return "gemini"
    if os.environ.get("CODEX_SESSION") or os.environ.get("CODEX_HOME"):
        return "codex"
    if os.environ.get("GROK_SESSION"):
        return "grok"
    if os.environ.get("MUSE_SESSION"):
        return "muse"
    if os.environ.get("ZCODE_APP_VERSION") or os.environ.get("ZCODE_RUNTIME_ENV"):
        return "zcode"
    return "unknown"


def detect_namespace(cwd: Path = None, override: str = None) -> str:
    if override:
        ns = override.strip()
        if ns == GLOBAL_NAMESPACE:
            # The reserved global namespace is its own name — the dash-slug
            # convention would invent "-_global", which no adapter ever
            # reads (the mirror projection writes _global directly).
            return validate_namespace(ns)
        if not ns.startswith("-"):
            ns = "-" + ns.replace("/", "-").replace("\\", "-").strip("-")
        return validate_namespace(ns)
    target = (cwd or Path.cwd()).resolve()
    # Normalize absolute path to slug: /home/user/proj -> -home-user-proj
    slug = str(target).replace("/", "-").replace("\\", "-")
    return validate_namespace(slug)


def get_namespace_dir(root: Path, namespace: str, create: bool = True) -> Path:
    namespace = validate_namespace(namespace)
    d = root / "namespaces" / namespace
    if create:
        d.mkdir(parents=True, exist_ok=True)
    return d


def get_watermark_path(root: Path, harness: str) -> Path:
    d = root / "watermarks"
    d.mkdir(parents=True, exist_ok=True)
    return d / f"{normalize_harness_name(harness)}.json"


def load_watermark(root: Path, harness: str) -> dict:
    h = normalize_harness_name(harness)
    p = get_watermark_path(root, h)
    if not p.exists():
        return {
            "harness": h,
            "last_seq": 0,
            "last_sync_ts": None,
            "namespaces": {},
        }
    try:
        with open(p, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {
            "harness": h,
            "last_seq": 0,
            "last_sync_ts": None,
            "namespaces": {},
        }


def save_watermark(root: Path, watermark: dict):
    h = normalize_harness_name(watermark["harness"])
    p = get_watermark_path(root, h)
    temp = p.with_suffix(".tmp")
    with open(temp, "w", encoding="utf-8") as f:
        json.dump(watermark, f, indent=2)
    temp.replace(p)


def get_journal_path(root: Path) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    return root / "journal.jsonl"


def memory_origin(root: Path) -> str:
    """Stable per-store identity; hostnames are neither unique nor immutable."""
    root.mkdir(parents=True, exist_ok=True)
    path = root / ".origin-id"
    if path.is_file():
        value = path.read_text(encoding="utf-8", errors="replace").strip()
        if value:
            return value
    value = str(uuid.uuid4())
    try:
        with path.open("x", encoding="utf-8") as handle:
            handle.write(value + "\n")
    except FileExistsError:
        value = path.read_text(encoding="utf-8").strip()
    return value


def object_path(root: Path, digest: str) -> Path:
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", digest):
        raise ValueError(f"invalid content-object digest: {digest!r}")
    return root / "objects" / digest[:2] / digest


def store_content_object(root: Path, source: Path) -> str:
    digest = file_sha256(source)
    target = object_path(root, digest)
    if not target.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        temp = target.with_suffix(".tmp")
        shutil.copyfile(source, temp)
        temp.replace(target)
    return digest


def record_event_id(record: dict) -> str:
    existing = record.get("event_id")
    if existing:
        return str(existing)
    # Stable migration identity for legacy records. Different hosts with truly
    # identical legacy events collapse; different content/timestamps survive.
    payload = json.dumps(record, sort_keys=True, separators=(",", ":")).encode()
    return "legacy-" + hashlib.sha256(payload).hexdigest()


def latest_version_for(root: Path, ns: str, filename: str) -> str | None:
    latest = None
    for record in read_journal_entries(root, namespace=ns):
        if record.get("file") == filename and record.get("version_id"):
            latest = record["version_id"]
    return latest


def matches_target_harness(entry_target: str, query_harness: str) -> bool:
    """Check if entry target matches query harness."""
    if not query_harness or query_harness == "all":
        return True
    if not entry_target or entry_target in ("all", "*", ""):
        return True
    ent = normalize_harness_name(entry_target)
    qh = normalize_harness_name(query_harness)
    return ent == qh


def read_journal_entries(root: Path, after_seq: int = 0, namespace: str = None, target_harness: str = None) -> list:
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
                    if namespace is not None and rec.get("ns") != namespace:
                        continue
                    if target_harness is not None:
                        ent_target = rec.get("target_harness", "all")
                        if not matches_target_harness(ent_target, target_harness):
                            continue
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


def unseen_journal_entries(root: Path, watermark: dict, namespace: str, harness: str) -> list:
    seen = set(watermark.get("seen_event_ids", []))
    vector = watermark.get("seen_origin_seq", {}).get(namespace, {})
    legacy_seq = watermark.get("last_seq", 0)
    result = []
    for record in read_journal_entries(root, after_seq=0, namespace=namespace, target_harness=harness):
        event_id = record.get("event_id")
        if event_id:
            origin = record.get("origin")
            if event_id not in seen and (not origin or record.get("seq", 0) > vector.get(origin, 0)):
                result.append(record)
        elif record.get("seq", 0) > legacy_seq:
            result.append(record)
    return result


def mark_events_seen(root: Path, watermark: dict, namespace: str | None, harness: str) -> None:
    vectors = watermark.setdefault("seen_origin_seq", {})
    for record in read_journal_entries(root, after_seq=0, namespace=namespace, target_harness=harness):
        origin = record.get("origin")
        if record.get("event_id") and origin:
            vector = vectors.setdefault(record.get("ns", ""), {})
            vector[origin] = max(vector.get(origin, 0), record.get("seq", 0))
    # Selective acknowledgements older than the full vector no longer need an
    # unbounded UUID entry.
    covered = {
        record_event_id(record)
        for record in read_journal_entries(root)
        if record.get("origin")
        and record.get("seq", 0)
        <= vectors.get(record.get("ns", ""), {}).get(record.get("origin"), 0)
    }
    watermark["seen_event_ids"] = sorted(set(watermark.get("seen_event_ids", [])) - covered)
    watermark["last_seq"] = get_latest_seq(root)


_AUTOMATIC_BASE = object()


def append_journal_entry(
    root: Path,
    ns: str,
    filename: str,
    kind: str,
    action: str,
    harness: str,
    summary: str,
    target_harness: str = "all",
    origin: str | None = None,
    base_version=_AUTOMATIC_BASE,
    base_versions: list[str] | None = None,
) -> dict:
    jpath = get_journal_path(root)
    latest = get_latest_seq(root)
    next_seq = latest + 1
    now_ts = int(time.time())
    iso_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
    event_id = str(uuid.uuid4())
    origin = origin or memory_origin(root)
    previous = latest_version_for(root, ns, filename) if base_version is _AUTOMATIC_BASE else base_version
    # A delete is total: it must supersede every head visible in the journal,
    # not only the latest version, or two concurrent deleters (per-host native
    # GC on stale views) fork the lineage and the door never converges —
    # the 2026-09-08 _global storm (135 doors, up to 8 unconnected delete
    # heads each) is that exact failure.
    if action == "delete" and not base_versions:
        base_versions = [
            head["version_id"]
            for head in causal_heads_for(root, ns, filename, coalesce=False)
        ]
    digest = None
    source = root / "namespaces" / ns / filename
    if action != "delete" and source.is_file():
        digest = store_content_object(root, source)
    # Content identity and causal version identity are deliberately different:
    # A -> B -> A reuses an object digest but is a new causal revision.
    version_id = f"event:{event_id}"
    record = {
        "seq": next_seq,
        "event_id": event_id,
        "origin": origin,
        "ts": now_ts,
        "iso": iso_ts,
        "ns": ns,
        "file": filename,
        "kind": kind,
        "action": action,
        "harness": normalize_harness_name(harness),
        "target_harness": normalize_harness_name(target_harness) if target_harness != "all" else "all",
        "summary": summary,
        "digest": digest,
        "version_id": version_id,
        "base_version": previous,
        "base_versions": base_versions or [],
    }
    with open(jpath, "a", encoding="utf-8") as f:
        f.write(json.dumps(record) + "\n")
    return record


def _event_semantic_key(record: dict) -> str:
    digest = record.get("digest")
    return f"object:{digest}" if digest else f"delete:{record_event_id(record)}"


def causal_heads_for(
    root: Path,
    namespace: str,
    filename: str,
    records: list[dict] | None = None,
    *,
    coalesce: bool = True,
) -> list[dict]:
    records = records if records is not None else read_journal_entries(root, namespace=namespace)
    records = [
        record for record in records if record.get("file") == filename and record.get("version_id")
    ]
    superseded = set()
    for record in records:
        if record.get("base_version"):
            superseded.add(record["base_version"])
        superseded.update(record.get("base_versions", []))
    heads = [record for record in records if record.get("version_id") not in superseded]
    if not coalesce:
        return heads
    semantic = {}
    for record in heads:
        semantic.setdefault(_event_semantic_key(record), record)
    return list(semantic.values())


def materialize_store(root: Path, namespaces: set[str] | None = None) -> dict:
    """Materialize content-addressed event heads without hiding divergence.

    A causal successor names ``base_version`` and supersedes that version. Two
    unconnected heads are concurrent. A deterministic live copy keeps readers
    working, while every divergent head is copied under ``conflicts/`` and the
    non-zero conflict count makes the condition observable to timers/onboarding.
    """
    groups = {}
    for record in read_journal_entries(root):
        if not record.get("version_id"):
            continue
        key = (record.get("ns", ""), record.get("file", ""))
        if not all(key):
            continue
        groups.setdefault(key, []).append(record)

    written = deleted = conflicts = missing_objects = 0
    for (namespace, filename), records in groups.items():
        if namespaces is not None and namespace not in namespaces:
            continue
        if filename == "MEMORY.md" or filename.endswith("/MEMORY.md"):
            # Namespace indexes are per-host hand-curated state. Index-line
            # edits carry no journal events, so materializing a journaled
            # MEMORY.md head silently rewound every hand-appended line
            # (fleet-wide 2026-08-22..09-09 defect, reintroduced by the
            # 09-09 deploy that predated this fix). Old heads stay in the
            # store; they are simply never materialized again. The guard
            # covers subdirectory doors too (e.g. sub/MEMORY.md).
            continue
        heads = causal_heads_for(root, namespace, filename, records)
        if not heads:
            continue

        live = get_namespace_dir(root, namespace) / filename
        conflict_dir = root / "conflicts" / namespace / filename.replace("/", "__")
        if conflict_dir.exists():
            shutil.rmtree(conflict_dir)

        if len(heads) > 1:
            conflicts += 1
            conflict_dir.mkdir(parents=True, exist_ok=True)
            for head in heads:
                event_id = record_event_id(head)
                digest = head.get("digest")
                if digest and object_path(root, digest).is_file():
                    shutil.copyfile(object_path(root, digest), conflict_dir / f"{event_id}.md")
                else:
                    (conflict_dir / f"{event_id}.delete.md").write_text(
                        json.dumps(head, indent=2) + "\n", encoding="utf-8"
                    )

        current_digest = file_sha256(live) if live.is_file() else None
        chosen = next((head for head in heads if head.get("digest") == current_digest), None)
        if chosen is None:
            chosen = max(
                heads,
                key=lambda record: (
                    record.get("ts", 0),
                    record.get("origin", ""),
                    record_event_id(record),
                ),
            )
        digest = chosen.get("digest")
        if not digest:
            if live.exists():
                live.unlink()
                deleted += 1
            continue
        source = object_path(root, digest)
        if not source.is_file():
            missing_objects += 1
            continue
        if current_digest != digest:
            live.parent.mkdir(parents=True, exist_ok=True)
            temp = live.with_suffix(live.suffix + ".tmp")
            shutil.copyfile(source, temp)
            temp.replace(live)
            written += 1
    return {
        "written": written,
        "deleted": deleted,
        "conflicts": conflicts,
        "missing_objects": missing_objects,
    }


def extract_metadata_and_summary(content: str, filename: str = "") -> tuple:
    """Extract frontmatter kind, description/summary, and target_harness."""
    kind = "other"
    summary = ""
    target_harness = "all"

    # Infer from filename prefix first (e.g. steer-gemini-*.md, steer-claude-*.md)
    if filename:
        lower_name = filename.lower()
        if lower_name.startswith("steer-"):
            parts = lower_name.split("-", 2)
            if len(parts) >= 2:
                candidate = parts[1]
                if candidate in ("gemini", "agy", "antigravity"):
                    target_harness = "gemini"
                    kind = "steer"
                elif candidate in ("claude", "cc"):
                    target_harness = "claude"
                    kind = "steer"
                elif candidate in ("grok", "codex", "muse"):
                    target_harness = candidate
                    kind = "steer"

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
                elif line.startswith("target_harness:") or line.startswith("scope:"):
                    raw_val = line.split(":", 1)[1].strip().strip('"').strip("'")
                    target_harness = normalize_harness_name(raw_val) if raw_val != "all" else "all"
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

    return kind, summary or "Updated memory door", target_harness


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

    new_entries = unseen_journal_entries(root, watermark, ns, harness)
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

    entries = unseen_journal_entries(root, watermark, ns, harness)
    if args.filter:
        flt = args.filter.lower()
        entries = [e for e in entries if flt in e.get("kind", "").lower() or flt in e.get("file", "").lower() or flt in e.get("summary", "").lower()]

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
        target = e.get("target_harness", "all")
        target_str = f" -> {target}" if target != "all" else ""
        summ = e.get("summary", "")
        print(f"[#{seq} | {kind}{target_str}] {fname} (by {author}): {summ}")


def cmd_get(args):
    root = Path(args.root)
    ns = detect_namespace(override=args.ns)
    filename = validate_door_filename(args.file)
    ns_dir = get_namespace_dir(root, ns, create=False)
    target = ns_dir / filename
    if not target.exists():
        print(f"Error: Door '{filename}' not found in namespace '{ns}'.", file=sys.stderr)
        sys.exit(1)
    with open(target, "r", encoding="utf-8") as f:
        lines = f.readlines()
    total = len(lines)
    shown = lines
    if args.grep:
        try:
            pat = re.compile(args.grep)
        except re.error as exc:
            print(f"Error: bad --grep pattern: {exc}", file=sys.stderr)
            sys.exit(1)
        shown = [line for line in shown if pat.search(line)]
    # [slice 4] corrections on EVERY read path: a dismissed line never
    # leaves through the direct door read either.
    shown = [line for line in shown if not owner_filter_line(root, line)]
    if args.lines is not None:
        shown = shown[: max(args.lines, 0)]
    if len(shown) != total:
        cut = f" (match /{args.grep}/)" if args.grep else ""
        print(
            f"[ygg-memory] showing {len(shown)} of {total} lines of {filename}{cut}"
            " — this is a SLICE; run without --lines/--grep for the whole door.",
            file=sys.stderr,
        )
    sys.stdout.write("".join(shown))


def cmd_ack(args):
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        watermark = load_watermark(root, harness)
        latest_seq = get_latest_seq(root)
        ns_map = watermark.setdefault("namespaces", {}).setdefault(ns, {})

        if args.all:
            mark_events_seen(root, watermark, ns, harness)
            watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            # Record current hashes of all doors in ns matching this harness
            ns_dir = get_namespace_dir(root, ns)
            for fpath in sorted(ns_dir.rglob("*.md")):
                if not fpath.is_file():
                    continue
                fname_key = fpath.relative_to(ns_dir).as_posix()
                content = fpath.read_text(encoding="utf-8")
                _, _, target_h = extract_metadata_and_summary(content, fpath.name)
                if matches_target_harness(target_h, harness):
                    ns_map[fname_key] = file_sha256(fpath)
            save_watermark(root, watermark)
            if args.json:
                print(json.dumps({"status": "ok", "acked": "all", "seq": latest_seq}))
            else:
                print(f"Acknowledged all doors for {harness} up to seq #{latest_seq}.")
        elif args.files:
            acked_files = []
            ns_dir = get_namespace_dir(root, ns)
            requested_files = [validate_door_filename(f.strip()) for f in args.files.split(",") if f.strip()]
            for fname in requested_files:
                fpath = ns_dir / fname
                if fpath.exists():
                    ns_map[fname] = file_sha256(fpath)
                    acked_files.append(fname)
            watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            selected = set(acked_files)
            seen = set(watermark.get("seen_event_ids", []))
            for record in read_journal_entries(root, namespace=ns, target_harness=harness):
                if record.get("file") in selected and record.get("event_id"):
                    seen.add(record["event_id"])
            watermark["seen_event_ids"] = sorted(seen)
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

    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        ns_dir = get_namespace_dir(root, ns)
        # The door name is the source basename unless --dest names a
        # namespace-relative subpath (e.g. campaign-cli-integration/codex.md).
        # Journal, watermark and index all key on this string, so a subpath
        # door is a first-class door everywhere a flat one is.
        if getattr(args, "as_name", None) and getattr(args, "dest", None):
            raise SystemExit("ygg-memory: --as and --dest are mutually exclusive")
        if getattr(args, "as_name", None):
            if "/" in args.as_name:
                raise SystemExit(
                    "ygg-memory: --as takes a flat basename (no '/'); "
                    "use --dest for subdirectory doors"
                )
            dest_filename = validate_door_filename(args.as_name.strip())
        elif getattr(args, "dest", None) and args.dest.strip():
            dest_filename = validate_door_filename(args.dest.strip())
        else:
            dest_filename = source_path.name
        dest_path = ns_dir / dest_filename
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        # Dream ACK-bccf702553: warn when minting a basename the namespace
        # has never seen — scratch-name publishes are how junk duplicate
        # doors get minted (ten of them in -home-pi-gh-yggterm alone).
        if not dest_path.exists() and not any(
            r.get("file") == dest_filename
            for r in read_journal_entries(root, namespace=ns)
        ):
            print(
                f"WARNING: '{dest_filename}' is a basename namespace '{ns}' has "
                f"never seen — you may be minting a duplicate of an existing "
                f"door. Use --as/--dest with the existing door name to update "
                f"in place.",
                file=sys.stderr,
            )

        content = source_path.read_text(encoding="utf-8")
        kind, summary, extracted_target = extract_metadata_and_summary(content, Path(dest_filename).name)

        if args.summary:
            summary = args.summary.strip()
        if args.kind:
            kind = args.kind.strip()

        target_harness = extracted_target
        if args.target_harness:
            raw_target = args.target_harness.strip()
            target_harness = normalize_harness_name(raw_target) if raw_target != "all" else "all"

        is_new = not dest_path.exists()
        action = "create" if is_new else "update"

        if source_path != dest_path:
            shutil.copy2(source_path, dest_path)
        record = append_journal_entry(root, ns, dest_filename, kind, action, harness, summary, target_harness=target_harness)
        _door_causal_forget(root, ns, dest_filename)

        # Update publisher's own watermark for this file
        watermark = load_watermark(root, harness)
        watermark.setdefault("namespaces", {}).setdefault(ns, {})[dest_filename] = file_sha256(dest_path)
        watermark["last_seq"] = max(watermark.get("last_seq", 0), record["seq"])
        if record.get("event_id"):
            origin = record.get("origin")
            if origin:
                vector = watermark.setdefault("seen_origin_seq", {}).setdefault(ns, {})
                vector[origin] = max(vector.get(origin, 0), record.get("seq", 0))
            else:
                seen = set(watermark.get("seen_event_ids", []))
                seen.add(record["event_id"])
                watermark["seen_event_ids"] = sorted(seen)
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)

        # Update the namespace index (per-host hand state — deliberately NOT
        # journaled; materialize_store never touches MEMORY.md). Idempotent
        # per door filename; a failure here never fails the publish.
        try:
            memory_index = ns_dir / "MEMORY.md"
            hook = (summary or kind or "door").replace("\n", " ").strip() or "door"
            door_label = Path(dest_filename).name
            index_line = f"- [{door_label}]({dest_filename}) — {hook}"
            if memory_index.exists():
                idx_content = memory_index.read_text(encoding="utf-8")
                if f"]({dest_filename})" not in idx_content:
                    if "UNIFIED FLEET MEMORY" not in idx_content:
                        idx_content = STEERING_HEADER + "\n" + idx_content
                    if not idx_content.endswith("\n"):
                        idx_content += "\n"
                    memory_index.write_text(idx_content + index_line + "\n", encoding="utf-8")
            else:
                memory_index.write_text(
                    STEERING_HEADER + f"\n## Doors\n\n{index_line}\n", encoding="utf-8"
                )
        except OSError as index_error:
            print(f"ygg-memory: index line not updated: {index_error}", file=sys.stderr)

        if args.json:
            print(json.dumps({"status": "ok", "record": record}))
        else:
            target_info = f" [target: {target_harness}]" if target_harness != "all" else ""
            print(f"Published '{dest_filename}' to namespace '{ns}' (seq #{record['seq']}){target_info}.")
    finally:
        _flock_close(lock)


def cmd_delete(args):
    """Retire a door: journal action=delete with total supersede.

    The verb form of the delete the sync's native path already journals:
    the record supersedes every causal head of the door (append_journal_entry
    computes the full base_versions set), so both concurrent deleters and
    older content heads converge — a door deleted here STAYS deleted, and a
    later publish resurrects it deliberately. The content objects stay in
    the store, so nothing is unrecoverable. The next sync-harness carries
    the retirement into the native store (backup first); native mirror
    trees whose source content is unchanged leave their source file alone.
    """
    root = Path(args.root)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=args.ns)
    filename = validate_door_filename(args.file)
    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        ns_dir = get_namespace_dir(root, ns)
        target = ns_dir / filename
        kind, target_h = "other", "all"
        if target.is_file():
            kind, _, target_h = extract_metadata_and_summary(
                target.read_text(encoding="utf-8"), Path(filename).name
            )
        else:
            known = any(
                record.get("file") == filename and record.get("version_id")
                for record in read_journal_entries(root, namespace=ns)
            )
            if not known:
                print(
                    f"Error: Door '{filename}' not found in namespace '{ns}'.",
                    file=sys.stderr,
                )
                sys.exit(1)
        record = append_journal_entry(
            root,
            ns,
            filename,
            kind,
            "delete",
            harness,
            f"deleted {filename}",
            target_harness=target_h,
        )
        _door_causal_forget(root, ns, filename)
        if target.exists():
            target.unlink()
        materialize_store(root)

        # Our own retirement is not news to us: drop the door's sync state
        # (the next sync-harness then retires the native copy with a backup)
        # and acknowledge the event so status stays quiet.
        watermark = load_watermark(root, harness)
        watermark.setdefault("namespaces", {}).setdefault(ns, {}).pop(filename, None)
        watermark.setdefault("sync_state", {}).setdefault(ns, {}).pop(filename, None)
        watermark.setdefault("sync_versions", {}).setdefault(ns, {}).pop(filename, None)
        if record.get("origin"):
            vector = watermark.setdefault("seen_origin_seq", {}).setdefault(ns, {})
            vector[record["origin"]] = max(vector.get(record["origin"], 0), record.get("seq", 0))
        seen = set(watermark.get("seen_event_ids", []))
        if record.get("event_id"):
            seen.add(record["event_id"])
        watermark["seen_event_ids"] = sorted(seen)
        watermark["last_seq"] = max(watermark.get("last_seq", 0), record["seq"])
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)

        if args.json:
            print(json.dumps({"status": "ok", "record": record}))
        else:
            print(f"Deleted door '{filename}' from namespace '{ns}' (seq #{record['seq']}).")
    finally:
        _flock_close(lock)


def cmd_resolve(args):
    """Resolve every current causal head with one reviewed Markdown result."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    namespace = detect_namespace(override=args.ns)
    filename = validate_door_filename(args.file)
    source = Path(args.using).resolve()
    if not source.is_file():
        raise SystemExit(f"ygg-memory: resolution source does not exist: {source}")
    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        heads = causal_heads_for(root, namespace, filename)
        raw_heads = causal_heads_for(root, namespace, filename, coalesce=False)
        if len(heads) < 2:
            raise SystemExit(f"ygg-memory: {namespace}/{filename} has fewer than two divergent heads")
        destination = get_namespace_dir(root, namespace, create=False) / filename
        if source != destination.resolve():
            shutil.copyfile(source, destination)
        content = destination.read_text(encoding="utf-8")
        kind, summary, target = extract_metadata_and_summary(content, filename)
        record = append_journal_entry(
            root,
            namespace,
            filename,
            kind,
            "resolve",
            harness,
            summary,
            target_harness=target,
            base_version=None,
            # Supersede every causal version, including identical duplicate
            # events that semantic conflict display intentionally coalesces.
            base_versions=[head["version_id"] for head in raw_heads],
        )
        report = materialize_store(root)
        remaining = causal_heads_for(root, namespace, filename)
        if len(remaining) != 1:
            raise RuntimeError(f"resolution did not converge {filename}: {len(remaining)} heads remain")
        if args.json:
            print(json.dumps({"status": "ok", "record": record, "report": report}))
        else:
            print(f"Resolved {namespace}/{args.file}; superseded {len(heads)} divergent heads.")
    finally:
        _flock_close(lock)


def _relative_md_names(directory: Path, exclude=None) -> set[str]:
    """Namespace-relative names of every Markdown file under ``directory``.

    Subpath names are what make subdirectory doors syncable: the three-way
    sync keys every door by this string, and ``local_dir / name`` /
    ``ns_dir / name`` resolve it on either side.
    """
    if not directory.is_dir():
        return set()
    names = set()
    for path in directory.rglob("*.md"):
        if not path.is_file():
            continue
        relative = path.relative_to(directory).as_posix()
        if exclude and exclude(relative):
            continue
        names.add(relative)
    return names


def _is_native_pinned_door(relative: str) -> bool:
    """The native ``pinned/yggterm/`` subtree is the global namespace's
    delivery shelf, owned by the fleet-door projection — never project-door
    sync, or every sync would ingest the shared global doors into every
    project namespace."""
    parts = PurePosixPath(relative).parts
    return len(parts) >= 2 and parts[0] == "pinned" and parts[1] == "yggterm"


def _sync_project_memory_namespace(root: Path, harness: str, ns: str, local_dir: Path) -> tuple:
    """Three-way sync one project-memory directory without mtime arbitration."""
    local_dir.mkdir(parents=True, exist_ok=True)
    ns_dir = get_namespace_dir(root, ns)
    in_count = 0
    out_count = 0
    del_count = 0
    watermark = load_watermark(root, harness)
    backend_identity = str(local_dir.resolve())
    backend_identities = watermark.setdefault("sync_backend_v2", {})
    if backend_identities.get(ns) != backend_identity:
        # v1 watermarks did not identify their native path. Reusing one after
        # correcting a fake/guessed backend turns "new correct directory is
        # empty" into hundreds of false user deletions. A new path starts with
        # no three-way base and receives/imports by content instead.
        watermark.setdefault("sync_state", {})[ns] = {}
        watermark.setdefault("sync_versions", {})[ns] = {}
        backend_identities[ns] = backend_identity
    sync_state = watermark.setdefault("sync_state", {}).setdefault(ns, {})
    version_state = watermark.setdefault("sync_versions", {}).setdefault(ns, {})
    names = _relative_md_names(
        local_dir, exclude=_is_native_pinned_door
    ) | _relative_md_names(ns_dir) | set(sync_state)

    for fname in sorted(names):
        local = local_dir / fname
        hub = ns_dir / fname
        local_exists = local.is_file()
        hub_exists = hub.is_file()
        local_digest = file_sha256(local) if local_exists else None
        hub_digest = file_sha256(hub) if hub_exists else None
        target = "all"
        if hub_exists:
            _, _, target = extract_metadata_and_summary(hub.read_text(encoding="utf-8"), fname)
        if hub_exists and not matches_target_harness(target, harness):
            # A private door for another harness is not ours to overwrite. If
            # this adapter previously delivered an unchanged copy, withdraw it.
            if fname in sync_state and local_digest == sync_state[fname]:
                backup_native_file(root, harness, ns, local)
                local.unlink()
                out_count += 1
            sync_state.pop(fname, None)
            version_state.pop(fname, None)
            continue

        base_digest = sync_state.get(fname)
        base_version = version_state.get(fname)

        def publish_native(causal_base: str | None) -> None:
            nonlocal in_count, del_count
            if local_exists:
                content = local.read_text(encoding="utf-8")
                kind, summary, native_target = extract_metadata_and_summary(content, Path(fname).name)
                hub.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(local, hub)
                action = "upsert"
                in_count += 1
            else:
                previous = hub.read_text(encoding="utf-8") if hub_exists else ""
                kind, summary, native_target = extract_metadata_and_summary(previous, Path(fname).name)
                if hub.exists():
                    hub.unlink()
                action = "delete"
                summary = f"deleted {fname}"
                del_count += 1
            append_journal_entry(
                root,
                ns,
                fname,
                kind,
                action,
                harness,
                summary,
                target_harness=native_target,
                base_version=causal_base,
            )
            materialize_store(root)

        if base_digest is None:
            if local_exists and not hub_exists:
                publish_native(None)
            elif local_exists and hub_exists and local_digest != hub_digest:
                publish_native(None)
        else:
            local_changed = local_digest != base_digest
            hub_changed = hub_digest != base_digest
            if local_changed and not hub_changed:
                publish_native(base_version or version_for_digest(root, ns, fname, hub_digest))
            elif local_changed and hub_changed and local_digest != hub_digest:
                publish_native(base_version)

        hub_exists = hub.is_file()
        hub_digest = file_sha256(hub) if hub_exists else None
        if hub_exists:
            content = hub.read_text(encoding="utf-8")
            _, _, target = extract_metadata_and_summary(content, fname)
        if hub_exists and matches_target_harness(target, harness):
            if not local.is_file() or file_sha256(local) != hub_digest:
                if local.is_file():
                    backup_native_file(root, harness, ns, local)
                local.parent.mkdir(parents=True, exist_ok=True)
                temp = local.with_suffix(local.suffix + ".tmp")
                shutil.copyfile(hub, temp)
                temp.replace(local)
                out_count += 1
            if sync_state.get(fname) != hub_digest or not version_state.get(fname):
                version_state[fname] = version_for_digest(root, ns, fname, hub_digest)
            sync_state[fname] = hub_digest
        elif not hub_exists and fname in sync_state:
            if local.is_file():
                backup_native_file(root, harness, ns, local)
                local.unlink()
                out_count += 1
            sync_state.pop(fname, None)
            version_state.pop(fname, None)

    save_watermark(root, watermark)
    return in_count, out_count, del_count


def _sync_harness_namespace(
    root: Path,
    harness: str,
    namespace: str,
    local_dir: str | None = None,
) -> dict:
    """Synchronize one native area and return its convergence report."""
    adapter = get_harness_adapter(harness)
    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        migrate_legacy_store(root)
        if local_dir:
            if not isinstance(adapter, ProjectMemoryAdapter):
                raise ValueError(f"{harness} does not support --local-dir; use its native adapter")
            ingested, delivered, deleted = _sync_project_memory_namespace(
                root, harness, namespace, Path(local_dir)
            )
        else:
            ingested, delivered, deleted = adapter.sync_namespace(root, harness, namespace)

        # The area gate must report a broken causal store instead of silently
        # handing the caller a partially materialized door.
        materialized = materialize_store(root, namespaces={namespace, GLOBAL_NAMESPACE})
        watermark = load_watermark(root, harness)
        if not materialized["conflicts"] and not materialized["missing_objects"]:
            mark_events_seen(root, watermark, namespace, harness)
            watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            save_watermark(root, watermark)
        return {
            "status": "ok",
            "harness": harness,
            "namespace": namespace,
            "pulled_in": ingested,
            "pushed_out": delivered,
            "deleted": deleted,
            **materialized,
        }
    finally:
        _flock_close(lock)


def cmd_sync_harness(args):
    """Bidirectional sync between harness-local directory and ~/.yggterm/memory."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    adapter = get_harness_adapter(harness)

    if not getattr(args, "all", False):
        ns = detect_namespace(override=args.ns)
        result = _sync_harness_namespace(root, harness, ns, getattr(args, "local_dir", None))
        if args.json:
            print(json.dumps(result))
        else:
            extra = f", {result['deleted']} deleted" if result["deleted"] else ""
            print(f"Harness sync completed ({harness} <-> {ns}): {result['pulled_in']} ingested, {result['pushed_out']} propagated{extra}.")
        return

    # 60s, not the 10s startup default: --all is the seat's explicit
    # session-end push and competes with every other seat's verbs. It still
    # fails loudly on timeout on purpose — a seat must know its push did not
    # land (the catch-up tick is recovery, not an excuse).
    lock = _flock_open(root / ".ygg-memory.lock", timeout_seconds=60)

    try:
        migrate_legacy_store(root)
        namespace_count, total_in, total_out, total_del = adapter.sync_all(root, harness)
        materialized = materialize_store(root)
        watermark = load_watermark(root, harness)
        mark_events_seen(root, watermark, None, harness)
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)
        if args.json:
            print(json.dumps({"status": "ok", "harness": harness, "namespaces": namespace_count, "pulled_in": total_in, "pushed_out": total_out, "deleted": total_del, **materialized}))
        else:
            print(f"Harness sync completed ({harness} all {namespace_count} ns): {total_in} ingested, {total_out} propagated, {total_del} deleted.")
    finally:
        _flock_close(lock)


def cmd_sync(args):
    """Strict area gate used before campaign-memory recall."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    if not args.ns:
        raise SystemExit("ygg-memory sync requires an explicit --ns=<area>")
    ns = detect_namespace(override=args.ns)

    fleet_report = None
    if getattr(args, "fleet", False):
        fleet_report = _run_fleet_sync(root, resolve_fleet_mesh(getattr(args, "mesh", None)), quick=getattr(args, "quick", False))

    result = _sync_harness_namespace(root, harness, ns, getattr(args, "local_dir", None))
    if result["conflicts"] or result["missing_objects"]:
        print(
            f"ygg-memory: area sync did not converge for {ns}: "
            f"{result['conflicts']} conflict(s), {result['missing_objects']} missing object(s)",
            file=sys.stderr,
        )
        raise SystemExit(2)
    if fleet_report is not None:
        result["fleet"] = fleet_report
    if args.json:
        print(json.dumps(result))
    else:
        print(
            f"Area sync completed ({harness} <-> {ns}): "
            f"{result['pulled_in']} ingested, {result['pushed_out']} propagated, "
            f"{result['deleted']} deleted."
        )


def _merge_journals(local_path: Path, peer_content: str):
    records_by_key = {}
    if local_path.exists():
        for line in local_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                try:
                    d = json.loads(line)
                    k = record_event_id(d)
                    records_by_key[k] = d
                except Exception:
                    pass
    for line in peer_content.splitlines():
        line = line.strip()
        if line:
            try:
                d = json.loads(line)
                k = record_event_id(d)
                if k not in records_by_key:
                    records_by_key[k] = d
            except Exception:
                pass
    sorted_records = sorted(
        records_by_key.values(),
        key=lambda r: (r.get("ts", 0), r.get("origin", ""), r.get("seq", 0), record_event_id(r)),
    )
    local_path.parent.mkdir(parents=True, exist_ok=True)
    with open(local_path, "w", encoding="utf-8") as f:
        for r in sorted_records:
            f.write(json.dumps(r) + "\n")


def migrate_legacy_store(root: Path) -> int:
    """Snapshot pre-v2 namespace files, including old recursive rsync nests."""
    marker = root / ".v2-migrated"
    if marker.is_file():
        return 0
    records = read_journal_entries(root)
    represented = {
        (record.get("ns"), record.get("file"), record.get("digest"))
        for record in records
        if record.get("digest")
    }
    current_versions = {}
    for record in records:
        if record.get("version_id"):
            current_versions[(record.get("ns"), record.get("file"))] = record["version_id"]
    next_seq = max((record.get("seq", 0) for record in records), default=0)
    origin = memory_origin(root)
    migrated = 0
    pending = []
    namespace_root = root / "namespaces"
    for source in sorted(namespace_root.rglob("*.md")):
        relative = source.relative_to(namespace_root)
        if len(relative.parts) < 2:
            continue
        namespace = source.parent.name
        if namespace == "namespaces" or not (namespace.startswith("-") or namespace == GLOBAL_NAMESPACE):
            continue
        nested_legacy = len(relative.parts) > 2
        digest = store_content_object(root, source)
        if (namespace, source.name, digest) in represented:
            continue
        content = source.read_text(encoding="utf-8", errors="replace")
        kind, summary, target = extract_metadata_and_summary(content, source.name)
        next_seq += 1
        event_id = str(uuid.uuid4())
        version = f"event:{event_id}"
        pending.append(
            {
                "seq": next_seq,
                "event_id": event_id,
                "origin": origin,
                "ts": int(time.time()),
                "iso": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                "ns": namespace,
                "file": source.name,
                "kind": kind,
                "action": "upsert",
                "harness": "legacy-migration",
                "target_harness": target,
                "summary": summary,
                "digest": digest,
                "version_id": version,
                # Canonical files form the pre-v2 live line. A stranded nested
                # rsync copy has no proved ancestry, so retain it as a parallel
                # head instead of silently placing it before or after live.
                "base_version": None if nested_legacy else current_versions.get((namespace, source.name)),
                "base_versions": [],
            }
        )
        if not nested_legacy:
            current_versions[(namespace, source.name)] = version
        migrated += 1
    if pending:
        journal = get_journal_path(root)
        with journal.open("a", encoding="utf-8") as handle:
            for record in pending:
                handle.write(json.dumps(record) + "\n")
    marker.write_text("content-addressed-event-store-v2\n", encoding="utf-8")
    return migrated


def _fleet_ssh_command(connect_timeout: int) -> list[str]:
    return [
        "ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        f"ConnectTimeout={connect_timeout}",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "LogLevel=ERROR",
    ]


def _self_disperse_source(module_path: Path, home: Path | None = None) -> Path | None:
    """[11.236] The runner self-dispersal may push bytes only from the repo SSOT.

    ``_run_fleet_sync`` scps ``ygg-memory(.py)`` from the RUNNING module's
    directory to every peer — unguarded, that is a last-writer-wins fleet
    re-stale: a host whose installed copy lags (or a seat running sync-fleet
    from a worktree module) silently overwrites newer runners everywhere,
    exactly the 2026-10-06 [11.236] regression. Returns the directory to
    disperse from, or None to SKIP dispersal loudly (the sanctioned installer
    is ``ygg-disperse install --fleet``). A missing SSOT keeps the legacy
    behavior — nothing better to compare against.
    """
    base = (home or Path.home()).resolve()
    ssot_dir = base / "gh/yggterm/.agents/skills/yggterm-agent-fleet"
    if module_path.parent == ssot_dir:
        return module_path.parent
    ssot_file = ssot_dir / "ygg-memory.py"
    if not ssot_file.is_file():
        return module_path.parent
    if ssot_file.read_bytes() == module_path.read_bytes():
        return module_path.parent
    return None


def _peer_busy_lock(stderr_text: str) -> bool:
    """A peer whose hub lock is busy is DEFERRING, not diverging — the same
    law cmd_sync_fleet applies to the local lock (measured 2026-10-06:
    'did not converge: failed=dev:migrate' while dev's own tick held its
    lock past the 60s patience; the catch-up tick re-runs the exchange)."""
    return "Lock held by another process" in str(stderr_text or "")


def _fleet_outcome(unreachable: list, failures: list) -> str:
    """The convergence detail line, or '' when the run converged (a run with
    only busy-peer deferrals converges — the deferred peers are simply not
    exchanged this round)."""
    details = []
    if unreachable:
        details.append("unreachable=" + ",".join(unreachable))
    if failures:
        details.append("failed=" + ",".join(failures))
    return "; ".join(details)


def _run_fleet_sync(root: Path, mesh: list[str], quick: bool = False) -> dict:
    """Exchange immutable objects + event IDs; namespace files never rsync."""
    local_host = socket.gethostname()
    peers = [host for host in mesh if host != local_host]
    ssh_cmd = _fleet_ssh_command(1 if quick else 5)
    live_peers = []
    unreachable = []
    for peer in peers:
        try:
            result = subprocess.run(
                ssh_cmd + [peer, "true"], capture_output=True, timeout=2 if quick else 7
            )
        except subprocess.TimeoutExpired:
            unreachable.append(peer)
            continue
        if result.returncode == 0:
            live_peers.append(peer)
        else:
            unreachable.append(peer)

    # Dream ACK-d7795fc74c: block generously for a busy hub instead of a
    # fail-fast; on expiry cmd_sync_fleet reports a deferral (success), and
    # operators can raise YGG_MEMORY_FLEET_LOCK_WAIT (seconds) in known-busy
    # windows.
    local_lock = _flock_open(
        root / ".ygg-memory.lock",
        timeout_seconds=float(os.environ.get("YGG_MEMORY_FLEET_LOCK_WAIT", "60")),
    )
    try:
        migrate_legacy_store(root)
    finally:
        _flock_close(local_lock)
    (root / "objects").mkdir(parents=True, exist_ok=True)
    journal_file = get_journal_path(root)
    script_dir = _self_disperse_source(Path(__file__).resolve())
    if script_dir is None:
        print(
            "ygg-memory: sync-fleet runner self-dispersal SKIPPED — the running "
            "module is not the repo SSOT (stale install or worktree copy); run "
            "'ygg-disperse install --fleet' to install current runners",
            file=sys.stderr,
        )
    failures = []
    deferrals = []

    if not quick:
        for peer in live_peers:
            prepared = subprocess.run(
                ssh_cmd
                + [
                    peer,
                    "mkdir -p ~/.local/bin ~/.yggterm/bin ~/.yggterm/memory/objects; "
                    "for d in ~/.local/bin ~/.yggterm/bin; do "
                    "for f in ygg-memory ygg-memory.py; do "
                    "test ! -L \"$d/$f\" || unlink \"$d/$f\"; done; done",
                ],
                capture_output=True,
            )
            if prepared.returncode != 0:
                failures.append(f"{peer}:prepare")
            for destination in (() if script_dir is None else (".local/bin", ".yggterm/bin")):
                copied = subprocess.run(
                    [
                        "scp",
                        "-q",
                        "-o",
                        "BatchMode=yes",
                        str(script_dir / "ygg-memory.py"),
                        str(script_dir / "ygg-memory"),
                        f"{peer}:{destination}/",
                    ],
                    capture_output=True,
                )
                if copied.returncode != 0:
                    failures.append(f"{peer}:deploy-{destination}")
            made_executable = subprocess.run(
                ssh_cmd
                + [
                    peer,
                    "chmod +x ~/.local/bin/ygg-memory ~/.local/bin/ygg-memory.py ~/.yggterm/bin/ygg-memory ~/.yggterm/bin/ygg-memory.py",
                ],
                capture_output=True,
            )
            if made_executable.returncode != 0:
                failures.append(f"{peer}:chmod")

    # Every peer must objectify its own legacy live view before we pull its
    # journal or import ours. This handshake is the upgrade safety boundary.
    ready_peers = []
    for peer in live_peers:
        migrated = subprocess.run(
            ssh_cmd + [peer, "~/.local/bin/ygg-memory migrate --json"],
            capture_output=True,
            text=True,
        )
        if migrated.returncode == 0:
            ready_peers.append(peer)
        elif _peer_busy_lock(migrated.stderr):
            deferrals.append(f"{peer}:migrate-busy")
        else:
            failures.append(f"{peer}:migrate")
    live_peers = ready_peers

    ssh_transport = " ".join(ssh_cmd)
    pulled_objects = pushed_objects = 0
    incoming_journals = []
    for peer in live_peers:
        pull = subprocess.run(
            [
                "rsync",
                "-az",
                "--ignore-existing",
                "--exclude=*.tmp",
                "--itemize-changes",
                "-e",
                ssh_transport,
                f"{peer}:~/.yggterm/memory/objects/",
                str(root / "objects") + "/",
            ],
            capture_output=True,
            text=True,
        )
        if pull.returncode != 0:
            failures.append(f"{peer}:pull-objects")
        pulled_objects += sum(1 for line in pull.stdout.splitlines() if line.startswith(">f"))
        journal = subprocess.run(
            ssh_cmd + [peer, "cat ~/.yggterm/memory/journal.jsonl 2>/dev/null || true"],
            capture_output=True,
            text=True,
        )
        if journal.stdout:
            incoming_journals.append(journal.stdout)

    # Network calls never occur while this lock is held. Two hosts can sync at
    # once without each waiting on the other's local lock.
    local_lock = _flock_open(root / ".ygg-memory.lock", timeout_seconds=60)
    try:
        for incoming in incoming_journals:
            _merge_journals(journal_file, incoming)
        materialized = materialize_store(root)
        journal_content = journal_file.read_text(encoding="utf-8") if journal_file.is_file() else ""
    finally:
        _flock_close(local_lock)
    for peer in live_peers:
        push = subprocess.run(
            [
                "rsync",
                "-az",
                "--ignore-existing",
                "--exclude=*.tmp",
                "--itemize-changes",
                "-e",
                ssh_transport,
                str(root / "objects") + "/",
                f"{peer}:~/.yggterm/memory/objects/",
            ],
            capture_output=True,
            text=True,
        )
        if push.returncode != 0:
            failures.append(f"{peer}:push-objects")
        pushed_objects += sum(1 for line in push.stdout.splitlines() if line.startswith(">f"))
        command = "~/.local/bin/ygg-memory import-journal"
        imported = subprocess.run(
            ssh_cmd + [peer, command],
            input=journal_content,
            capture_output=True,
            text=True,
        )
        if imported.returncode != 0:
            if _peer_busy_lock(imported.stderr):
                deferrals.append(f"{peer}:import-busy")
            else:
                failures.append(f"{peer}:import-journal")
    if materialized.get("conflicts"):
        failures.append(f"{materialized['conflicts']}-preserved-conflict(s)")
    if materialized.get("missing_objects"):
        failures.append(f"{materialized['missing_objects']}-missing-object(s)")
    detail = _fleet_outcome(unreachable, failures)
    if detail:
        raise RuntimeError("fleet memory did not converge: " + detail)
    return {
        "peers": live_peers,
        **({"deferred": deferrals} if deferrals else {}),
        "pulled_objects": pulled_objects,
        "pushed_objects": pushed_objects,
        **materialized,
    }


def cmd_sync_fleet(args):
    """Mesh synchronize semantic objects/events across reachable SSH peers."""
    root = Path(args.root)
    mesh = resolve_fleet_mesh(args.mesh)
    try:
        report = _run_fleet_sync(root, mesh, quick=getattr(args, "quick", False))
    except RuntimeError as error:
        # ⛔ Busy hub ≠ broken sync. sync-fleet is the recovery convergence
        # leg — the catch-up tick re-runs it — so a busy lock is a DEFERRAL,
        # not a failure. Exiting 1 here read as breakage in every seat's
        # session-end ritual whenever another seat's long sync held the hub
        # (measured 2026-09-10: three seats, one flock, two false alarms).
        # A non-lock RuntimeError is a real fault and still raises.
        if not str(error).startswith("Lock held by another process"):
            raise
        if getattr(args, "json", False):
            print(json.dumps({"status": "deferred", "reason": str(error)}))
        else:
            print(
                "Fleet memory sync deferred — the hub lock is busy; "
                "the catch-up tick will cover it."
            )
        return
    if getattr(args, "json", False):
        print(json.dumps(report))
    elif not getattr(args, "quiet", False):
        peers = ", ".join(report["peers"]) or "none reachable"
        print(
            f"Fleet memory sync: {peers}; {report['pulled_objects']} objects pulled, "
            f"{report['pushed_objects']} pushed, {report['conflicts']} conflicts."
        )


def cmd_import_journal(args):
    root = Path(args.root)
    incoming = sys.stdin.read()
    lock = _flock_open(root / ".ygg-memory.lock", timeout_seconds=60)
    try:
        _merge_journals(get_journal_path(root), incoming)
        report = materialize_store(root)
        if getattr(args, "json", False):
            print(json.dumps(report))
    finally:
        _flock_close(lock)


def cmd_migrate(args):
    """Private upgrade endpoint: capture this host before accepting a peer."""
    root = Path(args.root)
    lock = _flock_open(root / ".ygg-memory.lock", timeout_seconds=60)
    try:
        migrated = migrate_legacy_store(root)
        report = materialize_store(root)
        output = {"migrated": migrated, **report}
        if getattr(args, "json", False):
            print(json.dumps(output))
    finally:
        _flock_close(lock)


def shelve_legacy_archives(adapter, memory_dir: Path, apply: bool) -> tuple:
    """[mem-dream slice 1] Move top-level shelved-class docs into archive/.

    Returns ``(candidates, moved)``. Dry-run reports candidates and moves
    nothing; apply moves (skip-on-collision, per-move log to the caller) and
    the caller rebuilds the tiered indexes. Sitting archives become native
    per-host history at <project>/memory-archive/ — OUTSIDE the scanned
    memory tree, invisible to client-side index regenerators and to native
    sync; the next sync-harness journaled-deletes their hub mirrors
    (recoverable by design) — doors remain the cross-host SSOT. A legacy
    in-tree archive/ directory (the pre-1.1 scheme) is migrated into
    memory-archive/ whole.
    """
    candidates = []
    for doc in sorted(memory_dir.glob("*.md")):
        if doc.name in ("MEMORY.md", "ARCHIVE-INDEX.md"):
            continue
        content = doc.read_text(encoding="utf-8", errors="replace")
        if not content.startswith("---\n"):
            continue
        if _is_shelved_doc((), doc.name, adapter._meta(content, "type")):
            candidates.append(doc)
    moved = 0
    if apply:
        archive_dir = memory_dir.parent / "memory-archive"
        archive_dir.mkdir(parents=True, exist_ok=True)
        # Legacy in-tree archive/ (pre-1.1): migrate residents whole.
        legacy_dir = memory_dir / "archive"
        if legacy_dir.is_dir():
            for doc in sorted(legacy_dir.glob("*.md")):
                dest = archive_dir / doc.name
                if dest.exists():
                    if doc.read_bytes() == dest.read_bytes():
                        doc.unlink()
                        moved += 1
                    else:
                        print(f"  skip (differs in memory-archive/): {doc.name}")
                    continue
                doc.rename(dest)
                moved += 1
            if not any(legacy_dir.iterdir()):
                legacy_dir.rmdir()
        for doc in candidates:
            dest = archive_dir / doc.name
            if dest.exists():
                print(f"  skip (exists in memory-archive/): {doc.name}")
                continue
            doc.rename(dest)
            moved += 1
    return candidates, moved


def cmd_shelve_archives(args):
    """[mem-dream slice 1] Tier the native store: dry-run by default."""
    harness = detect_harness(args.harness)
    adapter = get_harness_adapter(harness, cwd=Path.cwd())
    if not isinstance(adapter, ZcodeMemoryAdapter):
        print(f"shelve-archives: harness {harness} has no tiered index; nothing to do")
        return
    total_candidates = total_moved = 0
    for memory_dir in adapter.memory_dirs():
        candidates, moved = shelve_legacy_archives(adapter, memory_dir, args.apply)
        total_candidates += len(candidates)
        total_moved += moved
        if not candidates:
            continue
        print(f"{memory_dir}: {len(candidates)} shelved-class top-level docs")
        for doc in candidates[:3]:
            print(f"  e.g. {doc.name}")
        if args.apply:
            adapter._rebuild_index(memory_dir)
        else:
            print("  (dry-run — pass --apply to move)")
    if args.apply:
        print(f"shelve-archives: moved {total_moved} docs into archive/; indexes rebuilt")
    if getattr(args, "json", False):
        print(json.dumps({"harness": harness, "candidates": total_candidates, "moved": total_moved}))


# -----------------------------------------------------------------------------
# [mem-dream slice 2] dream verb + search verb
#
# Design door: memory-dreaming-design (default zcode project). The dream is a
# deterministic bookend pair around a pluggable composer — `--prepare` stages
# the raw-delta brief, an LLM composes (sol subprocess via --run, or the GLM
# seat reading the staged brief), `--apply` validates and lands the result
# through the journaled publish path. The composer NEVER writes frontmatter,
# doors, or indexes directly: apply mints metadata, enforces the per-claim
# as-of/proof stamps, and the adversarial critic verdicts are mandatory when
# claims exist. Ingestion isolation: the collector never reads dreamer-origin
# events or the dreamer-owned doors (NOW.md / owner-now.md), so the dreamer
# cannot consolidate its own output.
# -----------------------------------------------------------------------------

DREAM_ORIGIN = "dreamer"
DREAM_OWNED_BASENAMES = {"NOW.md", "owner-now.md"}
DREAM_NOW_MAX_BYTES = 2600
DREAM_OWNER_NOW_MAX_BYTES = 1100
DREAM_INPUT_BUDGET_BYTES = 160_000
DREAM_PER_FILE_BYTES = 22_000
DREAM_EXTRA_INPUT_BYTES = 32_000
DREAM_MAX_OPS = 3
DREAM_OP_ENTRY_MAX = 400
NOW_REQUIRED_SECTIONS = ("§CURRENT_STATUS", "§NEXT_UNITS", "§ACTIVE_TRAPS", "§POINTERS")
NOW_STAMPED_SECTIONS = ("§CURRENT_STATUS", "§NEXT_UNITS", "§ACTIVE_TRAPS")
DREAM_PROOF_VOCAB = ("deployed", "suite", "measured-live", "claimed")

# ── [slice 1] FLEET BUDGET POLICY — sol round-4 Q4, owner-ratified ──────────
# Soft bucket allocations under HARD fleet-wide totals. One authoritative day
# boundary (UTC); unused allocations are borrowable by eligible work; no daily
# debt or carryover. The verification reserve is NOT borrowable — recovery is
# reserved before execution, never raided for ordinary admissions.
BUDGET_DIMS = ("input", "generated", "cash_cents")
BUDGET_TOTALS = {"input": 100_000, "generated": 10_000, "cash_cents": 50}
BUDGET_BUCKETS = {
    "yin": {"input": 20_000, "generated": 2_000, "cash_cents": 10, "borrowable": True},
    "yang": {"input": 15_000, "generated": 1_500, "cash_cents": 8, "borrowable": True},
    "work:callgraph": {"input": 12_000, "generated": 1_200, "cash_cents": 6, "borrowable": True},
    "work:dossiergraph": {"input": 10_000, "generated": 1_000, "cash_cents": 5, "borrowable": True},
    "work:infra": {"input": 7_000, "generated": 700, "cash_cents": 4, "borrowable": True},
    "work:conversion": {"input": 16_000, "generated": 1_600, "cash_cents": 8, "borrowable": True},
    "work:consult-docs": {"input": 10_000, "generated": 1_000, "cash_cents": 5, "borrowable": True},
    "reserve:verification": {"input": 10_000, "generated": 1_000, "cash_cents": 5, "borrowable": False},
}
# Jev keeps its OWN routing envelope (Q4) — tracked here so status can show it,
# never a budget bypass: routing through Jev still enters a generative ledger.
BUDGET_JEV_ENVELOPE_CENTS = 15
BUDGET_RESERVATION_TTL_S = 7200  # crash safety: an orphaned hold self-releases
BUDGET_ARCHIVE_DAYS = 8
BUDGET_RECEIPTS_MAX = 400
BUDGET_DREAM_BUCKET = "yin"  # a dream compose TIGHTENS memory (Yin)
BUDGET_DREAM_GEN_ESTIMATE = 2_000  # tokens: composer + critic generated ceiling
BUDGET_DREAM_CASH_CENTS = 4  # round-5 measured scale for a compose-sized pass
BUDGET_TOKENS_PER_BYTE = 4  # the standard chars/token text estimate

DREAM_CONTRACT = """You are the DREAM COMPOSER for this namespace. Produce a
response in EXACTLY this machine-parsed marker format (plain markers, no code
fences around the NOW.MD block):

=== NOW.MD ===
<markdown body ONLY — no frontmatter, at most 2600 bytes, exactly these four
section headings: `## §CURRENT_STATUS`, `## §NEXT_UNITS`, `## §ACTIVE_TRAPS`,
`## §POINTERS`. EVERY bullet line under the first three sections ends with an
inline stamp `[as-of: YYYY-MM-DD; proof: <deployed|suite|measured-live|claimed>]`
taken from the raw deltas — never invent a stamp or a date. Proof vocabulary:
deployed = landed + deployed + read-back verified; suite = tests green only;
measured-live = instrumented live evidence in the deltas; claimed = stated by
a seat but unproven. NOW.md wholly REPLACES the previous rollup: carry forward
still-true claims (re-verify against the deltas), prune dead ones, and when a
newer delta CONTRADICTS an old claim keep the falsification visible under
§ACTIVE_TRAPS with a `[FALSIFIED by seq#N/<file>]` stamp. §POINTERS cites door
filenames and board ACKs (citations, not claims — no stamps needed).>
=== OPS ===
<JSON array, possibly []. Each element is either
  {"op": "append-section", "file": "<door filename>", "section": "<section
   marker exactly as it appears in that door, e.g. §GOTCHAS>", "entry": "<one
   markdown bullet starting with '- ', at most 400 chars>", "reason": "<why>"}
or {"op": "propose-tombstone", "file": "<door filename>", "reason": "<why>"}.
At most 3 ops. These are the ONLY op types: never rewrite door bodies, never
touch indexes, never delete.>
=== CLAIMS ===
<JSON array with one entry per stamped bullet in NOW.MD:
  {"claim_id": "c1", "text": "<the bullet without its stamp>", "as_of":
   "YYYY-MM-DD", "proof": "<vocab word>", "source_refs": ["seq#N/<file>", ...]}.
source_refs must cite deltas that exist below.>
=== OWNER-NOW ===
<JSON object {"body": "<markdown, at most 1100 bytes, the owner's fast-rotting
context: mood/energy, location/travel, active commitments, working hours,
communication shape — each line stamped [as-of: YYYY-MM-DD]; include only what
the deltas EVIDENCE>", "evidence": ["seq#N/<file>", ...]} — or the single word
null when the deltas contain no owner-state evidence.>"""


def get_dream_dir(root: Path, ns: str, create: bool = True) -> Path:
    ns = validate_namespace(ns)
    d = root / "dream" / ns
    if create:
        d.mkdir(parents=True, exist_ok=True)
    return d


def load_dream_watermark(root: Path, ns: str) -> dict:
    """Scratch cache FIRST, journal floor SECOND (round-4: amnesiac
    watermarks). The scratch file is per-host; the journal is the fleet
    truth. When any host dreamed this ns, its NOW.md publish (origin
    dreamer) is in the journal at a seq >= what was consumed — deriving
    the floor from it means a foreign-host dream never re-collects from
    seq 0. The floor is conservative by construction (a few records
    re-read at most; latest-content dedup makes that harmless)."""
    p = get_dream_dir(root, ns, create=False) / "watermark.json"
    default = {"ns": ns, "last_seq": 0, "last_dream_ts": None, "dreams": []}
    watermark = dict(default)
    try:
        with open(p, "r", encoding="utf-8") as f:
            loaded = json.load(f)
        if isinstance(loaded, dict):
            watermark.update(loaded)
    except Exception:
        pass
    floor = 0
    jpath = get_journal_path(root)
    if jpath.exists():
        with open(jpath, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except Exception:
                    continue
                if (
                    rec.get("ns") == ns
                    and rec.get("file") == "NOW.md"
                    and rec.get("origin") == DREAM_ORIGIN
                ):
                    # The publish event fires a few records AFTER the dream
                    # consumed its deltas; the EXACT consumed floor rides the
                    # summary marker so nothing is skipped and nothing is
                    # re-collected beyond the dream's own watermark.
                    marker = re.search(
                        r"\[consumed seq (\d+)\]", str(rec.get("summary", ""))
                    )
                    if marker:
                        floor = max(floor, int(marker.group(1)))
    if floor > watermark.get("last_seq", 0):
        watermark["last_seq"] = floor
    return watermark


def save_dream_watermark(root: Path, ns: str, watermark: dict) -> None:
    d = get_dream_dir(root, ns)
    p = d / "watermark.json"
    temp = p.with_suffix(".tmp")
    with open(temp, "w", encoding="utf-8") as f:
        json.dump(watermark, f, indent=2)
    temp.replace(p)


def _dream_slice(text: str, cap: int) -> str:
    if len(text.encode("utf-8")) <= cap:
        return text
    head = 2000
    tail = max(0, cap - head - 200)
    return (
        text[:head]
        + "\n…[dream: middle truncated, head+tail slice only]…\n"
        + (text[-tail:] if tail else "")
    )


def collect_dream_deltas(root: Path, ns: str, after_seq: int) -> tuple[list[dict], int]:
    """Latest content per door since ``after_seq``, ingestion-isolated.

    Walks ns journal events in seq order: a delete drops the door, a content
    event (create/update/resolve) records it — so the collector returns the
    CURRENT text of every door that changed, not an event stream. Dreamer
    output (origin or owned basenames) never enters the input set.
    """
    state: dict[str, dict] = {}
    captured = after_seq
    for rec in sorted(read_journal_entries(root, after_seq=after_seq, namespace=ns), key=lambda r: r.get("seq", 0)):
        captured = max(captured, rec.get("seq", 0))
        fname = rec.get("file") or ""
        if not fname or Path(fname).name in DREAM_OWNED_BASENAMES:
            continue
        if rec.get("origin") == DREAM_ORIGIN:
            continue
        if rec.get("action") == "delete":
            state.pop(fname, None)
            continue
        if not rec.get("digest"):
            continue
        state[fname] = rec
    deltas = []
    for fname, rec in state.items():
        op = object_path(root, rec["digest"])
        if op.is_file():
            content = op.read_text(encoding="utf-8", errors="replace")
        else:
            content = f"(content object {rec['digest'][:12]} missing from store)"
        deltas.append(
            {
                "file": fname,
                "seq": rec.get("seq", 0),
                "iso": rec.get("iso", ""),
                "action": rec.get("action", ""),
                "kind": rec.get("kind", ""),
                "content": _dream_slice(content, DREAM_PER_FILE_BYTES),
            }
        )
    deltas.sort(key=lambda d: d["seq"], reverse=True)
    return deltas, captured


DREAM_RETAINED_CLAIMS_BUDGET_BYTES = 20_000
DREAM_CLAIM_REFS_MAX = 24
DREAM_CLAIM_CONFIRMATIONS_MAX = 5


def select_retained_claims(root: Path, ns: str, state: dict) -> tuple[list[str], list[str], int]:
    """[slice 3] Ranked bounded retrieval over the persistent claim store.

    Order: temporal work first (expired/due — the composer must see what it
    must re-verify), then the rest served through a ROTATING window persisted
    in the dream watermark, byte-budgeted. Returns (selected_ids, omitted_ids,
    rotation_offset_for_next_dream). Nothing is dropped from the STORE — only
    from this brief; the omitted ids travel with the brief as an index."""
    claims = state.get("claims", {})
    if not claims:
        return [], [], 0
    today = budget_today()
    # [slice 4] owner-dismissed claims are never staged for revival — they
    # are counted separately so the manifest shows the correction took hold
    dismissed = [cid for cid in claims if claim_is_dismissed(root, cid, claims[cid])]
    claims = {cid: e for cid, e in claims.items() if cid not in set(dismissed)}
    if not claims:
        return [], [], 0

    def rank(cid: str):
        entry = claims[cid]
        status = claim_status(entry, today)
        return (0 if status in ("due", "expired") else 1, str(entry.get("last_seen", "")), cid)

    ordered = sorted(claims.keys(), key=rank)
    urgent = [cid for cid in ordered if rank(cid)[0] == 0]
    tail = [cid for cid in ordered if rank(cid)[0] != 0]

    watermark = load_dream_watermark(root, ns)
    offset = int(watermark.get("claim_rotation", 0)) % len(tail) if tail else 0
    rotated = tail[offset:] + tail[:offset]

    selected: list[str] = []
    omitted: list[str] = []
    used = 0
    for cid in urgent + rotated:
        size = len(json.dumps({cid: claims[cid]}).encode("utf-8"))
        if used + size > DREAM_RETAINED_CLAIMS_BUDGET_BYTES and selected:
            omitted.append(cid)
            continue
        selected.append(cid)
        used += size
    # keep the store order stable for the brief; the rotation advances past
    # what THIS dream served so the next one starts where it ended
    served_tail = len([cid for cid in selected if cid in set(tail)])
    rotation = (offset + served_tail) % len(tail) if tail else 0
    watermark["claim_rotation"] = rotation
    save_dream_watermark(root, ns, watermark)
    return selected, omitted, rotation


def prepare_dream(
    root: Path,
    ns: str,
    phase: str,
    extra_inputs: list[str],
    budget: int,
) -> tuple[Path, dict] | None:
    """Stage pending/BRIEF.md + MANIFEST.json. None on a clean tick."""
    watermark = load_dream_watermark(root, ns)
    deltas, captured = collect_dream_deltas(root, ns, watermark["last_seq"])
    extras = []
    for raw in extra_inputs or []:
        path = Path(raw).expanduser().resolve()
        if not path.is_file():
            raise SystemExit(f"dream: --extra-input not a file: {path}")
        extras.append(
            {
                "path": str(path),
                "content": _dream_slice(
                    path.read_text(encoding="utf-8", errors="replace"), DREAM_EXTRA_INPUT_BYTES
                ),
            }
        )
    if not deltas and not extras:
        return None

    kept, used, dropped = [], 0, 0
    for delta in deltas:
        size = len(delta["content"].encode("utf-8"))
        if used + size > budget:
            dropped += 1
            continue
        kept.append(delta)
        used += size

    now_path = get_namespace_dir(root, ns, create=False) / "NOW.md"
    current_now = now_path.read_text(encoding="utf-8", errors="replace") if now_path.is_file() else "(none — first dream)"
    # [slice 4] corrections on EVERY read path: the previous rollup and the
    # raw deltas enter the brief already owner-qualified — the composer can
    # never revive through its inputs what the owner dismissed on its outputs.
    current_now = apply_owner_filters(root, current_now)

    pending = get_dream_dir(root, ns) / "pending"
    if pending.exists():
        shutil.rmtree(pending)
    pending.mkdir(parents=True)

    # [slice 4] COVERAGE MANIFEST: every delta accounted for — represented,
    # deferred (budget), or excluded (why). An acknowledged event need not
    # appear in NOW, but its disposition must be recoverable. The watermark
    # never advances past unaccounted work (sol: a byte-limited brief must
    # never advance its watermark to an unprocessed journal head).
    manifest_entries = [
        {"seq": d["seq"], "file": d["file"], "action": d["action"], "disposition": "represented"}
        for d in kept
    ] + [
        {"seq": d["seq"], "file": d["file"], "action": d["action"], "disposition": "deferred", "why": "input budget"}
        for d in deltas[len(kept):]
    ]

    # [slice 4] RETAINED CLAIMS: the composer receives the persistent claim
    # state (evidence-linked) so it does not relearn the world from the newest
    # batch. Previous NOW prose is context, NOT evidence — claims carry dates.
    # [slice 3] BOUNDED RETRIEVAL (gap-2 ruling): injecting every retained
    # claim exhausts context at multi-year scale. Selection is ranked —
    # due/expired temporal work first, then newest — and the remainder is
    # served by a ROTATING window (state in the dream watermark) so every
    # claim is eventually seen; what was omitted goes out as an id index,
    # and `claims --query` reaches any rare early fact on demand.
    state = load_claim_state(root, ns)
    selected_claims, omitted_claims, rotation = select_retained_claims(root, ns, state)
    retained = {cid: state["claims"][cid] for cid in selected_claims}

    lines = [
        f"# DREAM BRIEF — {ns} — phase {phase} — prepared {datetime.datetime.now(datetime.timezone.utc).isoformat()}",
        "",
        "## CONTRACT",
        "",
        DREAM_CONTRACT,
        "",
        "## RETAINED CLAIMS (persistent, evidence-linked — reuse these; do not re-derive; superseded/expired ones are dead)",
        "",
        json.dumps(retained, indent=1) if retained else "(none yet)",
        "",
        (
            f"(+{len(omitted_claims)} more claim(s) NOT staged this dream — retrieve by id with "
            f"`ygg-memory claims --ns={ns} --query <text>` or --id: {', '.join(omitted_claims[:24])}"
            f"{' …' if len(omitted_claims) > 24 else ''})"
        )
        if omitted_claims
        else "",
        "",
        "## CURRENT ROLLUP (previous NOW.md — REPLACE semantics; prune contradicted claims)",
        "",
        current_now,
        "",
        "## RAW DELTAS (newest first; provenance = journal seq / door / action / ts)",
        "",
    ]
    for delta in kept:
        lines.append(f"### [seq #{delta['seq']} | {delta['file']} | {delta['action']} | {delta['iso']}]")
        lines.append(apply_owner_filters(root, delta["content"]))
        lines.append("")
    if dropped:
        lines.append(f"(input budget: {dropped} oldest delta(s) DROPPED for budget — the rollup may miss them)")
        lines.append("")
    if extras:
        lines.append("## EXTRA INPUTS (staged by the seat; provenance = path)")
        lines.append("")
        for extra in extras:
            lines.append(f"### [{extra['path']}]")
            lines.append(extra["content"])
            lines.append("")
    lines.append(
        f"## GATE — dream watermark seq {watermark['last_seq']} → captured seq {captured}; "
        f"{len(kept)} delta door(s), {used} bytes; extra inputs: {len(extras)}"
    )
    brief_path = pending / "BRIEF.md"
    brief_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    manifest = {
        "ns": ns,
        "phase": phase,
        "prepared_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "watermark_seq": watermark["last_seq"],
        "captured_seq": captured,
        "delta_count": len(kept),
        "delta_files": [d["file"] for d in kept],
        "dropped_for_budget": dropped,
        "extra_inputs": [e["path"] for e in extras],
        "budget": budget,
        "brief_bytes": brief_path.stat().st_size,
        # [slice 2] the owner's focus doc read AT STAGING (the doc is the
        # SSOT; the CLI is optional sugar) — recorded so briefs/oplogs show
        # WHY this namespace was prioritized.
        "focus": focus_view(root, ns),
        # [slice 4] coverage manifest: per-event disposition, recoverable
        "coverage": manifest_entries,
        "retained_claims_count": len(state.get("claims", {})),
        "retained_selected": selected_claims,
        "retained_omitted": len(omitted_claims),
        # [slice 4] the review surface: how many stored claims the owner's
        # dismissals kept OUT of this brief
        "retained_dismissed": sum(1 for cid, e in state.get("claims", {}).items() if claim_is_dismissed(root, cid, e)),
        "claim_rotation": rotation,
    }
    (pending / "MANIFEST.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return brief_path, manifest


def _dream_json_block(raw: str) -> str:
    return re.sub(r"```[a-zA-Z]*\n?|```", "", raw).strip()


def parse_dream_response(text: str) -> dict:
    def between(start: str, end: str | None) -> str:
        pattern = re.escape(start) + r"\s*\n(.*?)(?=\n=== |\Z)"
        match = re.search(pattern, text, re.DOTALL)
        if not match:
            raise ValueError(f"response is missing the {start} marker block")
        return match.group(1).strip()

    now_md = between("=== NOW.MD ===", "=== OPS ===")
    ops_raw = _dream_json_block(between("=== OPS ===", "=== CLAIMS ==="))
    claims_raw = _dream_json_block(between("=== CLAIMS ===", "=== OWNER-NOW ==="))
    owner_raw = _dream_json_block(between("=== OWNER-NOW ===", None))
    return {
        "now_md": now_md,
        "ops": json.loads(ops_raw) if ops_raw else [],
        "claims": json.loads(claims_raw) if claims_raw else [],
        "owner_now": json.loads(owner_raw) if owner_raw else None,
    }


def parse_critic_verdicts(text: str) -> list[dict]:
    match = re.search(r"=== VERDICTS ===\s*\n(.*?)(?=\n=== |\Z)", text, re.DOTALL)
    if not match:
        raise ValueError("critic output is missing the === VERDICTS === marker block")
    verdicts = json.loads(_dream_json_block(match.group(1)))
    if not isinstance(verdicts, list):
        raise ValueError("critic VERDICTS block is not a JSON array")
    return verdicts


def validate_dream_response(parsed: dict, verdicts: list[dict], ns_dir: Path) -> list[str]:
    errors: list[str] = []
    now_md = parsed["now_md"]
    if now_md.startswith("---"):
        errors.append("NOW.MD block must not carry frontmatter — apply mints it deterministically")
    if len(now_md.encode("utf-8")) > DREAM_NOW_MAX_BYTES:
        errors.append(f"NOW.MD block is {len(now_md.encode('utf-8'))} bytes > {DREAM_NOW_MAX_BYTES}")
    for section in NOW_REQUIRED_SECTIONS:
        if section not in now_md:
            errors.append(f"NOW.MD is missing section {section}")

    current_section = None
    stamped_bullets = 0
    for line in now_md.splitlines():
        stripped = line.strip()
        for section in NOW_REQUIRED_SECTIONS:
            if section in stripped and (stripped.startswith("#") or stripped.startswith("§")):
                current_section = section
        if re.match(r"^(-|\*|\d+\.)\s", stripped):
            if current_section in NOW_STAMPED_SECTIONS:
                stamped_bullets += 1
                if "[as-of:" not in stripped:
                    errors.append(f"unstamped bullet in {current_section}: {stripped[:70]}")
                else:
                    stamp = re.search(r"\[as-of:\s*([^\];]+)[;]?\s*proof:\s*([^\]]+)\]", stripped)
                    if not stamp:
                        errors.append(f"malformed stamp on: {stripped[:70]}")
                    elif stamp.group(2).strip() not in DREAM_PROOF_VOCAB:
                        errors.append(
                            f"proof '{stamp.group(2).strip()}' not in vocabulary {DREAM_PROOF_VOCAB}: {stripped[:70]}"
                        )
    if stamped_bullets == 0:
        errors.append("NOW.MD carries no stamped claim bullets — a rollup with no claims is not a rollup")

    claim_ids = set()
    for claim in parsed["claims"]:
        cid = claim.get("claim_id")
        if not cid or cid in claim_ids:
            errors.append(f"claim entry missing/dupe claim_id: {json.dumps(claim)[:80]}")
            continue
        claim_ids.add(cid)
        if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", str(claim.get("as_of", ""))):
            errors.append(f"claim {cid} as_of is not YYYY-MM-DD")
        if claim.get("proof") not in DREAM_PROOF_VOCAB:
            errors.append(f"claim {cid} proof not in vocabulary: {claim.get('proof')!r}")
        if not claim.get("text"):
            errors.append(f"claim {cid} has empty text")
        if not claim.get("source_refs"):
            errors.append(f"claim {cid} cites no source_refs")

    if parsed["claims"] and not verdicts:
        errors.append("CRITIC.md verdicts missing — the adversarial critic gate is mandatory when claims exist")
    verdict_map = {v.get("claim_id"): v for v in verdicts if isinstance(v, dict)}
    for cid in claim_ids:
        verdict = verdict_map.get(cid)
        if verdict is None:
            errors.append(f"claim {cid} has no critic verdict")
        elif verdict.get("verdict") not in ("upheld", "contested"):
            errors.append(f"claim {cid} verdict not upheld|contested: {verdict.get('verdict')!r}")
    # A contested claim may survive ONLY visibly demoted in the rollup.
    for claim in parsed["claims"]:
        verdict = verdict_map.get(claim.get("claim_id"))
        if verdict and verdict.get("verdict") == "contested":
            fragment = (claim.get("text") or "")[:40]
            if fragment:
                for line in now_md.splitlines():
                    if fragment in line and "[CONTESTED" not in line and "[FALSIFIED" not in line:
                        errors.append(f"contested claim left undemoted: {fragment}")

    ops = parsed["ops"]
    if len(ops) > DREAM_MAX_OPS:
        errors.append(f"{len(ops)} ops > {DREAM_MAX_OPS}")
    for op in ops:
        kind = op.get("op")
        door = ns_dir / str(op.get("file", ""))
        if not op.get("reason"):
            errors.append(f"op {kind} on {op.get('file')!r} has no reason")
        if kind == "append-section":
            if not door.is_file():
                errors.append(f"append-section target door does not exist: {op.get('file')!r}")
                continue
            entry = str(op.get("entry", ""))
            if not entry.startswith("- "):
                errors.append(f"append-section entry must be a '- ' bullet: {entry[:60]}")
            if len(entry) > DREAM_OP_ENTRY_MAX:
                errors.append(f"append-section entry > {DREAM_OP_ENTRY_MAX} chars")
            section = str(op.get("section", ""))
            if not section or section not in door.read_text(encoding="utf-8", errors="replace"):
                errors.append(f"append-section marker {section!r} not found in {op.get('file')!r}")
        elif kind == "propose-tombstone":
            if not door.is_file():
                errors.append(f"propose-tombstone target door does not exist: {op.get('file')!r}")
        else:
            errors.append(f"unknown op type: {kind!r}")

    owner_now = parsed["owner_now"]
    if owner_now:
        body = str(owner_now.get("body", ""))
        if len(body.encode("utf-8")) > DREAM_OWNER_NOW_MAX_BYTES:
            errors.append(f"owner-now body > {DREAM_OWNER_NOW_MAX_BYTES} bytes")
        if not owner_now.get("evidence"):
            errors.append("owner-now carries no evidence refs")
    return errors


def _dream_first_claim_line(now_md: str) -> str:
    for line in now_md.splitlines():
        stripped = line.strip()
        if re.match(r"^(-|\*)\s", stripped) and "[as-of:" in stripped:
            text = re.sub(r"\s*\[[^\]]*\]\s*$", "", stripped).lstrip("-* ").strip()
            return text[:110]
    return ""


def _upsert_ns_index_line(ns_dir: Path, filename: str, hook: str) -> None:
    """Mirror of publish's index upsert for dreamer-owned doors."""
    try:
        memory_index = ns_dir / "MEMORY.md"
        index_line = f"- [{Path(filename).stem}]({filename}) — {hook}"
        if memory_index.exists():
            idx_content = memory_index.read_text(encoding="utf-8")
            if f"]({filename})" not in idx_content:
                if "UNIFIED FLEET MEMORY" not in idx_content:
                    idx_content = STEERING_HEADER + "\n" + idx_content
                if not idx_content.endswith("\n"):
                    idx_content += "\n"
                memory_index.write_text(idx_content + index_line + "\n", encoding="utf-8")
        else:
            memory_index.write_text(
                STEERING_HEADER + f"\n## Doors\n\n{index_line}\n", encoding="utf-8"
            )
    except OSError as index_error:
        print(f"ygg-memory: index line not updated: {index_error}", file=sys.stderr)


# ── [slice 3, sol 2026-10-16] CORRECTABLE OWNER CONTEXT ─────────────────────
# Two logical layers (sol Q3), both selectable across namespaces through ONE
# owner identity (_global):
#   durable-owner.md  — preferences, constraints, recurring practices, important
#                       background, surfacing instructions. Persists until
#                       corrected or superseded (NOT fast-rotting).
#   owner-now.md      — current commitments, temporary availability, travel,
#                       explicit energy/mood. Short validity, evidence dates.
# And an AUTHORITATIVE corrections record that takes effect on the NEXT memory
# READ — without waiting for quiescence, cooldown, or composition, and immune
# to an old archive resurrecting a dismissed claim.
DURABLE_OWNER_FILENAME = "durable-owner.md"
OWNER_CORRECTIONS_FILENAME = "owner-corrections.json"


def _owner_corrections_path(root: Path) -> Path:
    return root / "owner" / OWNER_CORRECTIONS_FILENAME


def load_owner_corrections(root: Path) -> dict:
    """The authoritative owner record: corrections, dismissals, surfacing
    instructions. Owner-authored (never dreamer-generated) — a campaign dream
    must never overwrite a newer owner update learned elsewhere."""
    path = _owner_corrections_path(root)
    if not path.is_file():
        return {"corrections": [], "dismissals": [], "surfacing": []}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        for key in ("corrections", "dismissals", "surfacing"):
            if not isinstance(data.get(key), list):
                data[key] = []
        return data
    except Exception:
        return {"corrections": [], "dismissals": [], "surfacing": []}


def owner_correct(root: Path, kind: str, target: str, text: str = "", *, scope: str = "_global") -> dict:
    """Apply one authoritative owner correction/dismissal/surfacing instruction.

    kind ∈ {correct, dismiss, surface}. Takes effect on the next READ.
    Append-only journal of owner authority — never deleted, so a dismissed
    claim can never be resurrected by replaying an old archive."""
    data = load_owner_corrections(root)
    entry = {
        "ts": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "authority": "owner",
        "kind": kind,
        "target": target,
        "text": text,
        "scope": scope,
    }
    key = {"correct": "corrections", "dismiss": "dismissals", "surface": "surfacing"}.get(kind)
    if key is None:
        raise ValueError(f"unknown owner correction kind: {kind}")
    data[key].append(entry)
    path = _owner_corrections_path(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    return entry


def owner_dismissed_targets(root: Path) -> list[str]:
    """[slice 4] The dismiss targets in force — exact claim ids, `claim:<id>`
    forms, and text fragments, all of which must die on EVERY read path."""
    return [d.get("target", "") for d in load_owner_corrections(root).get("dismissals", []) if d.get("target")]


def owner_filter_line(root: Path, line: str) -> bool:
    """True when ONE line is dismissed (shared by every line-oriented read
    path, so paraphrase copies and merged bullets die identically)."""
    for target in owner_dismissed_targets(root):
        if target and target in line:
            return True
    return False


def claim_is_dismissed(root: Path, cid: str, entry: dict) -> bool:
    """[slice 4] A claim is dismissed when the owner targeted its id, its
    `claim:<id>` form, or any text fragment it carries."""
    text = str(entry.get("text", ""))
    for target in owner_dismissed_targets(root):
        if not target:
            continue
        if target == cid or target == f"claim:{cid}" or target in text:
            return True
    return False


def claim_owner_correction(root: Path, cid: str, entry: dict) -> str | None:
    """[slice 4] The authoritative replacement text for a corrected claim,
    or None — 'false' is corrected, 'stop surfacing' is dismissal, 'surface
    under these circumstances' is surfacing; the three never conflate."""
    text = str(entry.get("text", ""))
    for c in load_owner_corrections(root).get("corrections", []):
        target = c.get("target", "")
        if target and (target == cid or target == f"claim:{cid}" or target in text):
            return c.get("text") or ""
    return None


def apply_owner_filters(root: Path, text: str) -> str:
    """Read-time application (slice 3): strip dismissed content and append
    owner surfacing instructions. Cheap, deterministic, composition-free."""
    out_lines = [line for line in text.splitlines() if not owner_filter_line(root, line)]
    out = "\n".join(out_lines)
    surf = load_owner_corrections(root).get("surfacing", [])
    if surf:
        out += "\n\n## owner surfacing instructions\n"
        for s in surf:
            if s.get("text"):
                out += f"- {s['text']} (as-of {str(s.get('ts', ''))[:10]})\n"
    return out


def durable_owner_context(root: Path) -> str:
    """The durable owner layer, read at session start."""
    path = get_namespace_dir(root, GLOBAL_NAMESPACE, create=False) / DURABLE_OWNER_FILENAME
    if not path.is_file():
        return ""
    content = path.read_text(encoding="utf-8")
    if content.startswith("---\n"):
        end = content.find("\n---\n", 4)
        if end != -1:
            content = content[end + 5:]
    return content.strip()


# ── [slice 2, sol 2026-10-16] TIME-AWARE MEMORY DELIVERY ─────────────────────
# Three DISTINCT dates per claim (never conflated): evidence time (when the
# underlying observation was made), validity boundary (when it stops being
# usable as current, if known), review deadline (when it needs checking; passing
# it does not prove it false). Generation time stays separate — rewriting a
# claim today must never make old evidence look newly observed.
# Ages are computed AT READ, never stored (no composer tokens maintaining
# changing age strings).
CLAIMS_FILENAME = "claims.json"


def _claims_path(root: Path, ns: str) -> Path:
    return get_dream_dir(root, ns, create=False) / CLAIMS_FILENAME


def load_claim_state(root: Path, ns: str) -> dict:
    """Persistent, evidence-linked claim state for one namespace."""
    path = _claims_path(root, ns)
    if not path.is_file():
        return {"ns": ns, "claims": {}}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data.get("claims"), dict):
            data["claims"] = {}
        return data
    except Exception:
        return {"ns": ns, "claims": {}}


def save_claim_state(root: Path, ns: str, state: dict) -> None:
    path = _claims_path(root, ns)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")


def merge_claims(state: dict, claims: list, as_of: str) -> dict:
    """Fold a dream's CLAIMS into persistent state (slice 2 + slice 4).

    Preserves supersession edges and evidence dates: a claim older in evidence
    is NEVER re-stamped by a newer dream just because it was restated.
    [slice 3] restatement vs confirmation: the same text recurring with NO
    new sources is a restatement (counted, never new corroboration — several
    summaries repeating one source are ONE evidentiary lineage); genuinely
    NEW source refs are recorded as dated confirmations while the original
    evidence stays first-class. Temporal fields are optional and validated;
    unknown values are dropped rather than invented."""
    store = state.setdefault("claims", {})
    for claim in claims:
        if not isinstance(claim, dict):
            continue
        cid = str(claim.get("claim_id") or "").strip()
        text = str(claim.get("text") or "").strip()
        if not cid or not text:
            continue
        existing = store.get(cid, {})
        new_refs = [str(r) for r in (claim.get("source_refs") or [])]
        old_refs = [str(r) for r in (existing.get("source_refs") or [])]
        entry = {
            "text": text,
            "as_of": str(claim.get("as_of") or as_of),
            "proof": str(claim.get("proof") or "claimed"),
            # generation is separate from evidence
            "last_seen": as_of,
            "source_refs": (old_refs + [r for r in new_refs if r not in old_refs])[:DREAM_CLAIM_REFS_MAX],
        }
        for field in ("valid_from", "valid_until", "review_after"):
            value = claim.get(field) or existing.get(field)
            if value and re.match(r"^\d{4}-\d{2}-\d{2}$", str(value)):
                entry[field] = str(value)
        supersedes = claim.get("supersedes") or existing.get("supersedes")
        if supersedes:
            entry["supersedes"] = supersedes
        # evidence date is sticky: a restatement does not re-date the evidence
        if existing.get("as_of"):
            entry["as_of"] = existing["as_of"]
        if existing:
            fresh_refs = [r for r in new_refs if r not in old_refs]
            if fresh_refs:
                # genuinely newer evidence confirming the old claim — the
                # original lineage stays first-class, the confirmation is dated
                confirmed = list(existing.get("confirmed") or [])
                confirmed.append({"as_of": as_of, "refs": fresh_refs[:5]})
                entry["confirmed"] = confirmed[-DREAM_CLAIM_CONFIRMATIONS_MAX:]
            elif existing.get("text") == text:
                # a restatement: same claim, no new evidence — counted, never
                # promoted to corroboration
                entry["restatements"] = int(existing.get("restatements", 0)) + 1
        store[cid] = entry
        # supersession edge: mark the superseded claim
        if supersedes and supersedes in store:
            store[supersedes]["superseded_by"] = cid
    return state


def claim_status(entry: dict, today: str) -> str:
    """ok | expired | due | superseded — computed at read, never stored."""
    if entry.get("superseded_by"):
        return "superseded"
    valid_until = entry.get("valid_until")
    if valid_until and str(valid_until) < today:
        return "expired"
    review_after = entry.get("review_after")
    if review_after and str(review_after) <= today:
        return "due"
    return "ok"


def temporal_envelope(root: Path, ns: str, *, today: str | None = None) -> str:
    """The read-time temporal envelope (slice 2): today's date, synthesis age,
    and per-claim ages/statuses. Returned WITHOUT spending composer tokens."""
    today = today or datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    lines = [f"Now: {today} (UTC)"]
    watermark = load_dream_watermark(root, ns)
    last_dream = watermark.get("last_dream_ts")
    if last_dream:
        try:
            age_days = (
                datetime.datetime.now(datetime.timezone.utc)
                - datetime.datetime.fromisoformat(str(last_dream))
            ).days
            lines.append(f"Synthesis: {age_days}d old (watermark seq {watermark.get('last_seq', 0)})")
        except Exception:
            pass
    state = load_claim_state(root, ns)
    claims = {
        cid: e
        for cid, e in state.get("claims", {}).items()
        # [slice 4] dismissed claims are dead on this read path too
        if not claim_is_dismissed(root, cid, e)
    }
    if claims:
        expired = sorted(c for c, e in claims.items() if claim_status(e, today) == "expired")
        due = sorted(c for c, e in claims.items() if claim_status(e, today) == "due")
        dead = sorted(c for c, e in claims.items() if claim_status(e, today) == "superseded")
        lines.append(
            f"Claims: {len(claims)} tracked · {len(expired)} expired · {len(due)} due · {len(dead)} superseded"
        )
        for cid in expired[:5]:
            entry = claims[cid]
            lines.append(f"  [{cid}] EXPIRED {entry.get('valid_until')} — evidence {entry.get('as_of')}: {entry.get('text', '')[:90]}")
        for cid in due[:5]:
            entry = claims[cid]
            lines.append(f"  [{cid}] REVIEW DUE {entry.get('review_after')} — {entry.get('text', '')[:90]}")
        for cid in dead[:3]:
            entry = claims[cid]
            lines.append(f"  [{cid}] SUPERSEDED by {entry.get('superseded_by')}: {entry.get('text', '')[:80]}")
    return "\n".join(lines)


def temporal_work_due(root: Path, ns: str, *, today: str | None = None) -> bool:
    """Slice 2 trigger: work_due = new_source_delta OR temporal_deadline_due.
    Without the second term a plan never ages when conversations stop."""
    today = today or datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    state = load_claim_state(root, ns)
    for entry in state.get("claims", {}).values():
        if claim_status(entry, today) in ("expired", "due"):
            return True
        valid_until = entry.get("valid_until")
        # a claim whose validity just ended within the last 2 days is fresh work
        if valid_until and valid_until <= today:
            return True
    return False


def _dream_publish_door(
    root: Path,
    ns: str,
    filename: str,
    content: str,
    harness: str,
    kind: str,
    summary: str,
) -> dict:
    ns_dir = get_namespace_dir(root, ns)
    dest = ns_dir / filename
    dest.write_text(content, encoding="utf-8")
    record = append_journal_entry(
        root, ns, filename, kind, "create" if not _door_existed(root, ns, filename) else "update",
        harness, summary, origin=DREAM_ORIGIN,
    )
    return record


def _door_existed(root: Path, ns: str, filename: str) -> bool:
    return any(r.get("file") == filename for r in read_journal_entries(root, namespace=ns))


def _dream_append_section(door_path: Path, section: str, entry: str) -> None:
    lines = door_path.read_text(encoding="utf-8", errors="replace").splitlines()
    header_idx = next((i for i, line in enumerate(lines) if section in line), None)
    if header_idx is None:
        raise ValueError(f"section marker {section!r} vanished from {door_path.name}")
    next_idx = len(lines)
    for i in range(header_idx + 1, len(lines)):
        stripped = lines[i].strip()
        if stripped.startswith("#") or (stripped.startswith("§") and section not in stripped):
            next_idx = i
            break
    insert_at = next_idx
    while insert_at > header_idx + 1 and not lines[insert_at - 1].strip():
        insert_at -= 1
    lines.insert(insert_at, "")
    lines.insert(insert_at + 1, entry)
    door_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _resolve_codex_bin(explicit: str | None) -> str:
    if explicit:
        return explicit
    candidate = Path.home() / ".yggterm" / "ynpm" / "bin" / "codex"
    if candidate.is_file():
        return str(candidate)
    found = shutil.which("codex")
    if found:
        return found
    raise SystemExit("dream: no codex binary found (pass --codex-bin)")


# [auth rotation 2026-10-17, owner law: "codex dreaming must be fault-tolerant
# of auth and limits — rotate between the three codex accounts when limits are
# hit; when all limits are hit there is nothing to do but wait"] The composer
# transport classifies auth/limit failures and rotates the active codex
# account via the fleet auth plane (ygg-auth rotate = switch to the freshest
# profile), retrying ONCE. A failure that persists after rotation means all
# accounts are limited — the caller's designed backoff IS the wait, and the
# next tick's rotate lands on whichever account reset first. Kill switch:
# YGG_DREAM_AUTH_ROTATE=0 restores the raw single-account behavior (tests,
# deliberate single-account runs). YGG_AUTH_RUNNER overrides the runner path.
CODEX_LIMIT_AUTH_MARKERS = (
    "rate limit", "usage limit", "ratelimit", "429", "quota",
    "hit your", "you've reached", "you have reached", "session expired",
    "token expired", "unauthorized", "forbidden", "authentication",
    "auth.json", "please log in", "please sign in", "walled",
)


def _codex_limit_auth_failure(text: str) -> bool:
    lowered = (text or "").lower()
    return any(marker in lowered for marker in CODEX_LIMIT_AUTH_MARKERS)


def _ygg_auth_runner() -> str | None:
    override = os.environ.get("YGG_AUTH_RUNNER")
    if override:
        return override
    candidates = (
        shutil.which("ygg-auth"),
        os.path.expanduser("~/.local/bin/ygg-auth"),
        str(Path(__file__).resolve().parent / "ygg-auth.py"),
        os.path.expanduser(
            "~/gh/yggterm/.agents/skills/yggterm-agent-fleet/ygg-auth.py"),
    )
    for candidate in candidates:
        if candidate and os.path.exists(candidate):
            return candidate
    return None


def _codex_exec(codex_bin: str, model: str, effort: str, prompt: str, timeout: int = 900) -> str:
    cmd = [
        codex_bin, "exec", "--skip-git-repo-check",
        "--dangerously-bypass-approvals-and-sandbox",
        "-m", model, "-c", f'model_reasoning_effort="{effort}"', prompt,
    ]

    def run_once():
        try:
            return subprocess.run(
                cmd, capture_output=True, text=True, timeout=timeout, cwd="/tmp"
            ), None
        except subprocess.TimeoutExpired:
            return None, f"dream: composer timed out after {timeout}s"

    result, timed_out = run_once()
    if timed_out:
        raise SystemExit(timed_out)
    if result.returncode != 0:
        tail = (result.stdout or "")[-400:] + (result.stderr or "")[-400:]
        rotation_enabled = (
            _codex_limit_auth_failure(tail)
            and os.environ.get("YGG_DREAM_AUTH_ROTATE", "1") != "0"
        )
        if rotation_enabled:
            runner = _ygg_auth_runner()
            if runner:
                rotate_cmd = (
                    [sys.executable, runner, "rotate"]
                    if runner.endswith(".py") else [runner, "rotate"]
                )
                try:
                    rot = subprocess.run(
                        rotate_cmd, capture_output=True, text=True, timeout=120
                    )
                    rot_lines = (rot.stdout or rot.stderr or "").strip().splitlines()
                    rot_note = rot_lines[-1] if rot_lines else "rotate ran"
                except (subprocess.TimeoutExpired, OSError) as rot_err:
                    rot_note = f"rotate unavailable ({rot_err})"
                print(
                    f"dream: composer auth/limit failure; ygg-auth rotate → {rot_note}",
                    file=sys.stderr,
                )
                retried, timed_out2 = run_once()
                if timed_out2:
                    raise SystemExit(timed_out2)
                if retried.returncode == 0:
                    return retried.stdout or ""
                tail2 = (retried.stdout or "")[-400:] + (retried.stderr or "")[-400:]
                raise SystemExit(
                    f"dream: composer limit/auth failure PERSISTED after account "
                    f"rotation ({rot_note}) — all codex accounts likely limited; "
                    f"the designed backoff is the wait. exited {retried.returncode}: …{tail2}"
                )
        raise SystemExit(f"dream: composer exited {result.returncode}: …{tail}")
    return result.stdout or ""


CRITIC_INSTRUCTIONS = """You are the ADVERSARIAL CRITIC for a dream rollup.
Below are the RAW DELTAS the composer drew from, the composer's CLAIMS, and
the drafted NOW.MD. For EVERY claim: try to falsify it against the raw
deltas — find an empirical counter-example, a contradicting measurement, a
newer fact that supersedes it, or a source_ref that does not actually
support the text. Respond in EXACTLY this format (no code fences):

=== VERDICTS ===
[{"claim_id": "<id>", "verdict": "upheld|contested", "counter_example":
 "<direct quote from the raw deltas, or null>"}]"""


DREAM_SUMMARY_PREFIXES = ("dream ", "owner-now update")
DREAM_QUIESCENCE_SECONDS = 15 * 60


def dream_live_activity(root: Path, ns: str) -> list:
    """Non-dream journal records for THIS ns (plus _global, which the
    dreamer also writes) — the quiescence and fencing input. The dream's
    own records are excluded so a dream never blocks or fences itself."""
    entries = list(read_journal_entries(root, namespace=ns))
    if ns != "_global":
        entries += read_journal_entries(root, namespace="_global")
    live = [
        rec
        for rec in entries
        if not str(rec.get("summary", "")).startswith(DREAM_SUMMARY_PREFIXES)
    ]
    live.sort(key=lambda rec: rec.get("ts", 0))
    return live


DREAM_AUTOCLAW_HEARTBEAT = Path(".yggterm/config/dream/autoclaw-heartbeat")


def dream_autoclaw_activity_age(home: Path | None = None) -> float | None:
    """Age in seconds of the AutoClaw activity heartbeat, or None if absent.

    AutoClaw sessions do not sit in yggterm rows, so the journal-based
    quiescence signal cannot see them. The autoclaw adapter (and the steer
    ritual) touches this heartbeat file on every sync/session start; the
    gate treats a fresh heartbeat as live activity.
    """
    hb = (home or Path.home()) / DREAM_AUTOCLAW_HEARTBEAT
    try:
        return time.time() - hb.stat().st_mtime
    except OSError:
        return None


def dream_quiescence_violation(root: Path, ns: str) -> str:
    """Gate 3 (round-3 consult): the dreamer never competes with an active
    writer — a journal write inside the cooldown window defers the dream.
    AutoClaw activity (heartbeat) counts as live writer activity too."""
    live = dream_live_activity(root, ns)
    if not live:
        return ""
    age = time.time() - float(live[-1].get("ts", 0))
    if age < DREAM_QUIESCENCE_SECONDS:
        return (
            f"ns {ns} is ACTIVE (last non-dream write {int(age)}s ago, "
            f"< {DREAM_QUIESCENCE_SECONDS}s quiescence) — dream deferred"
        )
    hb_age = dream_autoclaw_activity_age(root.parent.parent if root.parent.name == "memory" else Path.home())
    if hb_age is not None and hb_age < DREAM_QUIESCENCE_SECONDS:
        return (
            f"ns {ns} is ACTIVE (autoclaw heartbeat {int(hb_age)}s ago, "
            f"< {DREAM_QUIESCENCE_SECONDS}s quiescence) — dream deferred"
        )
    return ""


def dream_fence_violation(
    root: Path, ns: str, captured_seq: int, composed_files: list[str] | None = None
) -> str:
    """Gate 4 (round-3 consult), made DEPENDENCY-SCOPED in [slice 3] per the
    gap-1 ruling: the old stop-the-world fence let ANY journal advance
    refuse a finished synthesis forever under sustained fleet writes. Now a
    post-prepare event only fences the dream when it touches a door the
    composed batch actually covered (the same lost-update surface); events
    on unrelated doors stay PENDING for the next dream, and read-time
    owner corrections still override everything. With no scope supplied the
    conservative whole-namespace behavior stands. Dream-origin records
    never fence a dream."""
    advanced = [
        rec
        for rec in dream_live_activity(root, ns)
        if rec.get("seq", 0) > captured_seq
    ]
    if not advanced:
        return ""
    if composed_files is None:
        return (
            f"journal advanced since prepare ({len(advanced)} record(s) after "
            f"seq {captured_seq}) — re-prepare before apply"
        )
    scope = {str(f) for f in composed_files}
    # [fence fix 2026-10-16] dream_live_activity folds in ALL _global records
    # (the dreamer writes owner-now.md there for every ns dream), so a bare
    # filename match fences cross-namespace: one _global/advisors-doctrine.md
    # mirror record blocked a -home-pi-data-msggraph dream composing
    # msggraph/advisors-doctrine.md FOREVER (measured 2026-10-16 live, seq
    # 33180 vs watermark 33143). Namespace-qualify the match: a record in the
    # dreamed ns matches by filename; a _global record only matches when the
    # dreamed ns IS _global. Records with no file OR no ns attribution still
    # refuse (conservative).
    def _touches_composed_batch(rec) -> bool:
        if not rec.get("file"):
            return True
        if str(rec.get("file")) not in scope:
            return False
        rec_ns = rec.get("ns")
        if rec_ns is None:
            return True
        return rec_ns == ns or (rec_ns == "_global" and ns == "_global")

    conflicting = [rec for rec in advanced if _touches_composed_batch(rec)]
    if conflicting:
        return (
            f"journal advanced on composed door(s) since prepare "
            f"({len(conflicting)} record(s) after seq {captured_seq} touch "
            f"the composed batch) — re-prepare before apply"
        )
    return ""


DREAM_AUTO_COOLDOWN_S = 2 * 60 * 60
DREAM_AUTO_BACKOFF_S = 60 * 60
CANARY_MEMORY_MAX_LINES = 400
# [slice 1, sol 2026-10-16] the progress contract: every auto-dream invocation
# writes a bounded execution receipt; the canary (INDEPENDENT of the dreamer —
# a dreamer cannot report its own absence) checks receipt liveness + synthesis
# progress deadlines. Backoff is a FAILURE state with a retry time; it never
# refreshes a healthy timestamp.
DREAM_RECEIPT_MAX = 64          # bounded receipt log (drop oldest)
DREAM_PROGRESS_DEADLINE_S = 6 * 3600   # pending deltas older than this = violation


def _dream_receipt(root: Path, outcome: str, reason: str = "", ns: str = "") -> None:
    """Append one bounded execution receipt (slice 1). Valid outcomes: idle,
    deferred, running, failed, applied. Never raises."""
    try:
        stamp_dir = root / "dream"
        stamp_dir.mkdir(parents=True, exist_ok=True)
        path = stamp_dir / "receipts.jsonl"
        records = []
        if path.is_file():
            records = [json.loads(l) for l in path.read_text().splitlines() if l.strip()][-DREAM_RECEIPT_MAX + 1:]
        records.append({
            "ts": time.time(),
            "outcome": outcome,
            "ns": ns,
            "reason": reason[:200],
        })
        path.write_text("".join(json.dumps(r) + "\n" for r in records), encoding="utf-8")
    except Exception:
        pass  # the receipt can never fail the tick


def _dream_backoff_remaining(root: Path) -> float:
    """Seconds of backoff left, or 0."""
    stamp = root / "dream" / "auto-backoff.json"
    try:
        data = json.loads(stamp.read_text(encoding="utf-8"))
        return max(0.0, DREAM_AUTO_BACKOFF_S - (time.time() - float(data.get("ts", 0))))
    except Exception:
        return 0.0


def _dream_pending_summary(root: Path, ns: str) -> dict:
    """Per-namespace coverage state (slice 1): deltas pending, oldest pending
    age, and whether the progress deadline is blown."""
    try:
        watermark = load_dream_watermark(root, ns)
    except Exception:
        watermark = {"last_seq": 0}
    consumed = watermark.get("last_seq", 0)
    live = [r for r in dream_live_activity(root, ns) if r.get("seq", 0) > consumed]
    now = time.time()
    oldest = min((now - float(r.get("ts", now)) for r in live), default=0.0)
    return {
        "pending": len(live),
        "oldest_pending_s": int(oldest),
        "overdue": bool(live) and oldest > DREAM_PROGRESS_DEADLINE_S,
    }


def _dream_auto_backoff(root: Path, reason: str) -> None:
    stamp_dir = root / "dream"
    stamp_dir.mkdir(parents=True, exist_ok=True)
    (stamp_dir / "auto-backoff.json").write_text(
        json.dumps({"ts": time.time(), "reason": reason[:300]}), encoding="utf-8"
    )


def _dream_auto(root: Path, ns_filter: str, args) -> str:
    """The auto-dreamer (round-4 design): activity-driven, ONE namespace per
    invocation, cooldown + quiescence + pending-manifest filters, a 1h
    backoff stamp on any composer/apply failure, and it NEVER raises — the
    tick calls it. Composer failures during a quota cap are the designed
    sleep (round-4 Q3: never stage briefs across a long cap window — the
    read-version fence would refuse them anyway)."""
    backoff = root / "dream" / "auto-backoff.json"
    if backoff.is_file():
        try:
            stamp = json.loads(backoff.read_text(encoding="utf-8"))
            remaining = DREAM_AUTO_BACKOFF_S - (time.time() - float(stamp.get("ts", 0)))
            if remaining > 0:
                _dream_receipt(root, "deferred", f"backoff {int(remaining)}s: {str(stamp.get('reason', '?'))[:120]}")
                return f"auto-dream: backing off ({str(stamp.get('reason', '?'))[:80]})"
        except Exception:
            pass
    candidates = []
    dream_root = root / "dream"
    if dream_root.is_dir():
        for ns_dir in sorted(dream_root.iterdir()):
            if not ns_dir.is_dir():
                continue
            ns = ns_dir.name
            if ns_filter and ns_filter != "_global" and ns != ns_filter:
                continue
            watermark = load_dream_watermark(root, ns)
            last_ts = watermark.get("last_dream_ts")
            if last_ts:
                try:
                    if (
                        time.time()
                        - datetime.datetime.fromisoformat(str(last_ts)).timestamp()
                        < DREAM_AUTO_COOLDOWN_S
                    ):
                        continue
                except Exception:
                    pass
            if dream_quiescence_violation(root, ns):
                continue
            if (ns_dir / "pending" / "MANIFEST.json").is_file():
                continue
            consumed = watermark.get("last_seq", 0)
            delta = sum(
                1 for rec in dream_live_activity(root, ns) if rec.get("seq", 0) > consumed
            )
            # [slice 2 trigger] work_due = new_source_delta OR temporal_deadline_due:
            # an expired/due claim with ZERO new deltas still deserves a dream.
            if delta <= 0 and not temporal_work_due(root, ns):
                continue
            if delta <= 0:
                delta = 1  # deadline-driven dream (temporal work, no deltas)
            # [slice 1 fairness] score = delta * age pressure so a starving
            # small namespace (old pending, few deltas) can outrank a churny
            # big one; the pure-delta sort starved smaller namespaces.
            summary = _dream_pending_summary(root, ns)
            age_hours = summary["oldest_pending_s"] / 3600.0
            candidates.append((delta * (1.0 + min(age_hours, 24.0) / 6.0), delta, ns, summary))
    if not candidates:
        _dream_receipt(root, "idle", "no candidate (clean tick)")
        return "auto-dream: no candidate (clean tick)"
    # [slice 2] FOCUS APPLIES TO RANKING ONLY — never to eligibility. The
    # quiescence/cooldown/pending filters above already ran; a boost cannot
    # resurrect a blocked namespace (focus reorders, it does not grant).
    ranked = []
    for score, delta_count, ns_name, summary in candidates:
        focus = focus_view(root, ns_name)
        if focus["exclusive"] and not focus["matched"]:
            continue  # exclusive mode: all DISCRETIONARY work goes to the scope
        ranked.append(
            (score * focus["weight"], delta_count, ns_name, summary, focus["weight"])
        )
    if not ranked:
        _dream_receipt(root, "deferred", "all candidates excluded by an exclusive focus")
        return "auto-dream: deferred (exclusive focus excludes every candidate)"
    # [slice 3] OVERDUE-SERVICE ROTATION (gap-1 ruling: the fairness formula
    # had no service guarantee — a one-delta namespace could lose forever to
    # continuously replenished ones scoring above the age cap). A namespace
    # whose pending work has blown the progress deadline jumps the whole
    # queue, oldest pending first; the fairness score decides WITHIN each
    # class. Pending age derives from the journal, so attempts and blanket
    # deferrals never reset it — service is eventual, not merely likely.
    overdue = sorted(
        (r for r in ranked if r[3]["overdue"]),
        key=lambda r: -r[3]["oldest_pending_s"],
    )
    chosen = overdue[0] if overdue else max(ranked, key=lambda r: r[0])
    _, _, ns, _, weight = chosen
    # ⛔ `--ns <ns>` breaks when ns starts with a dash (every namespace does):
    # argparse reads the value as another option ("expected one argument").
    # The `=` form is the documented convention — the auto-dreamer died on
    # this for 10 days (2026-10-06..16, backoff-stamped each tick).
    base = [sys.executable, str(Path(__file__).resolve()), "--root", str(root), f"--ns={ns}"]
    try:
        run_ = subprocess.run(
            base
            + [
                "dream",
                "--run",
                "--model",
                getattr(args, "model", "gpt-6.1-sol"),
                "--effort",
                getattr(args, "effort", "high"),
            ],
            capture_output=True,
            text=True,
            timeout=900,
        )
    except subprocess.TimeoutExpired:
        _dream_auto_backoff(root, "composer timeout")
        _dream_receipt(root, "failed", "composer timeout", ns)
        return "auto-dream: composer timed out; 1h backoff stamped"
    if run_.returncode != 0:
        detail = (run_.stderr or run_.stdout or "").strip().splitlines()
        reason = detail[-1] if detail else "composer failed"
        _dream_auto_backoff(root, reason)
        _dream_receipt(root, "failed", reason, ns)
        return "auto-dream: composer failed; 1h backoff stamped"
    lines = (run_.stdout or "").strip().splitlines()
    _dream_receipt(root, "applied", lines[-1] if lines else "dreamed", ns)
    return f"auto-dream[{ns}]: {lines[-1] if lines else 'dreamed'}"


def cmd_now(args):
    """[slice 2+3] The session-start orientation read: durable owner context +
    NOW.md + the computed temporal envelope (claim ages, expiries,
    supersessions), with owner dismissals/surfacing applied at READ time.
    Ages are computed here, never stored — this is how a fresh session becomes
    time-aware without spending composer tokens."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    ns = detect_namespace(override=getattr(args, "ns", None))
    if not ns:
        raise SystemExit("now: --ns is required")
    durable = durable_owner_context(root)
    if durable:
        print("── durable owner context ──")
        print(apply_owner_filters(root, durable))
        print()
    ns_dir = get_namespace_dir(root, ns, create=False)
    now_path = ns_dir / "NOW.md"
    if now_path.is_file():
        content = now_path.read_text(encoding="utf-8")
        if content.startswith("---\n"):
            end = content.find("\n---\n", 4)
            if end != -1:
                content = content[end + 5:]
        print(apply_owner_filters(root, content.rstrip()))
    else:
        print(f"(no NOW.md yet for {ns} — the dreamer has not synthesized this namespace)")
    print()
    print("── temporal envelope ──")
    print(temporal_envelope(root, ns))


def cmd_owner(args):
    """[slice 3] The authoritative owner correction path: correct / dismiss /
    surface. Takes effect on the next read, no dreamer needed. [slice 4]
    `--affect TARGET` is the review surface: what is remembered, from which
    evidence, and what this correction currently touches."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    if getattr(args, "affect", None):
        print(json.dumps(owner_affect(root, args.affect), indent=None if getattr(args, "json", False) else 2))
        return
    kind = {"--correct": "correct", "--dismiss": "dismiss", "--surface": "surface"}
    chosen = None
    for flag, name in kind.items():
        if getattr(args, name, False):
            chosen = name
    if not chosen:
        raise SystemExit("owner: choose one of --correct / --dismiss / --surface")
    if getattr(args, "list", False):
        print(json.dumps(load_owner_corrections(root), indent=2))
        return
    target = getattr(args, "target", "") or ""
    text = getattr(args, "text", "") or ""
    entry = owner_correct(root, chosen, target, text, scope=getattr(args, "scope", "_global") or "_global")
    print(f"owner {chosen} recorded: {json.dumps(entry)[:200]}")
    print("(effective on the next memory read — no dream, no quiescence wait)")


def owner_affect(root: Path, target: str) -> dict:
    """[slice 4] The correction review surface: WHICH claims (with their
    evidence) and WHICH door lines the target currently matches — what the
    owner is dismissing/correcting, before it takes effect everywhere."""
    affect = {"target": target, "claims": [], "doors": [], "correction_in_force": None}
    data = load_owner_corrections(root)
    for kind in ("corrections", "dismissals", "surfacing"):
        for entry in data.get(kind, []):
            if entry.get("target") == target:
                affect["correction_in_force"] = {"kind": kind[:-1], **entry}
                break
        if affect["correction_in_force"]:
            break
    namespaces_root = root / "namespaces"
    if namespaces_root.is_dir():
        for ns_dir in sorted(p for p in namespaces_root.iterdir() if p.is_dir()):
            ns = ns_dir.name
            state = load_claim_state(root, ns)
            for cid, claim in state.get("claims", {}).items():
                if target == cid or target == f"claim:{cid}" or target in str(claim.get("text", "")):
                    affect["claims"].append({
                        "ns": ns, "id": cid, "text": str(claim.get("text", ""))[:140],
                        "as_of": claim.get("as_of"), "proof": claim.get("proof"),
                        "source_refs": (claim.get("source_refs") or [])[:3],
                    })
            for md in sorted(ns_dir.rglob("*.md")):
                try:
                    matches = sum(1 for line in md.read_text(encoding="utf-8", errors="replace").splitlines() if target in line)
                except OSError:
                    continue
                if matches:
                    affect["doors"].append({"ns": ns, "file": str(md.relative_to(ns_dir)), "lines": matches})
                    if len(affect["doors"]) >= 20:
                        return affect
    return affect


# ── [slice 5] CONNECTIONS — typed edges between resolvable objects ──────────
# Sol round-4 Q1: connection TYPE describes meaning, never authority. Edges
# wrap the native address systems (lobe://, [[hub links]], board ACK refs) —
# no replacement grammar is invented. The index is DERIVED and rebuildable;
# adapters are observation-only and never rewrite a source. Yin tightens
# (resolve unambiguous refs, dedupe, strengthen, mark broken/expired); Yang
# grows (hypothesis edges with a mandatory falsification test — "possibly
# related" fails admission, and an untested hypothesis expires from ordinary
# traversal after 7 days while staying visible as a hypothesis).
CONNECTION_STATUSES = ("observed", "owner-asserted", "hypothesis", "supported", "rejected", "expired")
CONNECTION_HYPOTHESIS_DAYS = 7
CONNECTIONS_REBUILD_MAX = 5_000


def connections_path(root: Path) -> Path:
    return root / "dream" / "connections.json"


def load_connections(root: Path) -> dict:
    path = connections_path(root)
    if not path.is_file():
        return {"version": 1, "edges": []}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data.get("edges"), list):
            data["edges"] = []
        return data
    except Exception:
        return {"version": 1, "edges": []}


def save_connections(root: Path, data: dict) -> None:
    path = connections_path(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=1) + "\n", encoding="utf-8")


def _connection_edge_id(from_addr: str, relation: str, to_addr: str) -> str:
    return hashlib.sha256(f"{from_addr}|{relation}|{to_addr}".encode("utf-8")).hexdigest()[:16]


def connection_view_status(edge: dict, today: str | None = None) -> str:
    """Computed, never stored: an untested hypothesis past its review date
    is EXPIRED in every view (history keeps the original status)."""
    today = today or budget_today()
    if edge.get("status") == "hypothesis" and not edge.get("tested_at"):
        if str(edge.get("review_by", "9999")) < today:
            return "expired"
    return edge.get("status", "observed")


def connection_observe(
    root: Path, from_addr: str, relation: str, to_addr: str, *,
    status: str = "observed", source_ref: str | None = None, created_by: str = "seat",
    hypothesis: str | None = None, falsification_test: str | None = None, review_days: int | None = None,
) -> dict:
    """Yin admission for one edge. Identical (from, relation, to) triples
    NEVER duplicate: a rerun strengthens the existing edge's evidence
    instead — several summaries repeating one source stay one lineage."""
    if status not in CONNECTION_STATUSES:
        raise ValueError(f"connection: unknown status {status}")
    if status == "hypothesis":
        # Yang admission: a hypothesis with no concrete test is noise
        if not hypothesis or not falsification_test:
            raise ValueError("connection: a hypothesis needs BOTH a hypothesis and a falsification test ('possibly related' fails admission)")
        if not review_days:
            review_days = CONNECTION_HYPOTHESIS_DAYS
    data = load_connections(root)
    edges = data.setdefault("edges", [])
    eid = _connection_edge_id(from_addr, relation, to_addr)
    today = budget_today()
    for edge in edges:
        if edge.get("id") == eid:
            if status in ("observed", "owner-asserted") and source_ref and source_ref not in edge.get("source_refs", []):
                edge["source_refs"] = (edge.get("source_refs", []) + [source_ref])[-10:]
                edge["strengthened_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
                save_connections(root, data)
                return {"action": "strengthened", "id": eid, "status": connection_view_status(edge, today)}
            return {"action": "deduplicated", "id": eid, "status": connection_view_status(edge, today)}
    entry = {
        "id": eid, "from": from_addr, "relation": relation, "to": to_addr,
        "status": status,
        "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "created_by": created_by,
        "source_refs": [source_ref] if source_ref else [],
    }
    if status == "hypothesis":
        entry.update({
            "hypothesis": hypothesis,
            "falsification_test": falsification_test,
            "review_by": (datetime.date.fromisoformat(today) + datetime.timedelta(days=review_days)).isoformat(),
        })
    edges.append(entry)
    data["edges"] = edges[-CONNECTIONS_REBUILD_MAX:]
    save_connections(root, data)
    return {"action": "created", "id": eid, "status": connection_view_status(entry, today)}


def connection_test(root: Path, edge_id: str, outcome: str, evidence: str | None = None) -> dict:
    """Run the named falsification test: supported or rejected. A rejection
    is history — never traversed again — and the edge records WHAT test ran."""
    if outcome not in ("supported", "rejected"):
        raise ValueError("connection: outcome is supported|rejected")
    data = load_connections(root)
    for edge in data.get("edges", []):
        if edge.get("id") == edge_id:
            edge["status"] = outcome
            edge["tested_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            if evidence:
                edge["source_refs"] = (edge.get("source_refs", []) + [evidence])[-10:]
            save_connections(root, data)
            return {"tested": True, "id": edge_id, "status": outcome}
    return {"tested": False, "reason": f"unknown edge {edge_id}"}


def connections_index(root: Path, *, explore: bool = False, today: str | None = None) -> dict:
    """The derived view. Ordinary retrieval EXCLUDES hypothesis edges unless
    exploration is requested; even then the hypothetical status survives."""
    today = today or budget_today()
    data = load_connections(root)
    out = []
    for edge in data.get("edges", []):
        status = connection_view_status(edge, today)
        if status == "rejected":
            continue  # history only
        if status in ("hypothesis", "expired") and not explore:
            continue
        row = {k: edge.get(k) for k in ("id", "from", "relation", "to", "status", "source_refs", "created_by")}
        row["status"] = status
        if status == "hypothesis":
            row["hypothesis"] = edge.get("hypothesis")
            row["falsification_test"] = edge.get("falsification_test")
            row["review_by"] = edge.get("review_by")
        out.append(row)
    return {"edges": out, "explore": explore, "today": today}


def scan_hub_links(root: Path, ns: str) -> dict:
    """Hub adapter (observation-only): [[name]] links resolve WITHIN the
    namespace against door stems. Ambiguous (2+ doors) and unresolved (0)
    links are RECORDED as unresolved — never silently guessed to a target.
    Unambiguous links become/strengthen derived edges. Sources untouched."""
    ns_dir = get_namespace_dir(root, ns, create=False)
    if not ns_dir.is_dir():
        raise SystemExit(f"connections: no namespace {ns}")
    stems: dict[str, list[str]] = {}
    doors = sorted(ns_dir.rglob("*.md"))
    for path in doors:
        stems.setdefault(path.stem.lower(), []).append(path.stem)
    created = strengthened = 0
    unresolved = []
    for path in doors:
        rel = str(path.relative_to(ns_dir))
        text = path.read_text(encoding="utf-8", errors="replace")
        for name in re.findall(r"\[\[([^\]\[]+)\]\]", text):
            matches = stems.get(name.strip().lower(), [])
            if len(matches) == 1:
                result = connection_observe(
                    root, f"{ns}/{path.stem}", "links", f"{ns}/{matches[0]}",
                    source_ref=f"hub:{ns}/{rel}", created_by="yin-hub-adapter",
                )
                if result["action"] == "created":
                    created += 1
                elif result["action"] == "strengthened":
                    strengthened += 1
            else:
                unresolved.append({"from": f"{ns}/{path.stem}", "link": name, "why": "ambiguous" if len(matches) > 1 else "no door"})
    return {"ns": ns, "created": created, "strengthened": strengthened, "unresolved": unresolved}


def scan_board_references(root: Path, boards_root: Path, board: str) -> dict:
    """Board adapter (observation-only): a post whose refs cite another
    post's ACK token becomes an observed `replies-to` edge between the two
    immutable posts. A board reply proves that someone replied — never
    agreement, support, or completion (Q1). Foreign ref forms are skipped,
    never guessed. Append-only boards replay identically (dedupe by design)."""
    path = boards_root / board / "board.jsonl"
    if not path.is_file():
        raise SystemExit(f"connections: no board journal at {path}")
    created = strengthened = deduplicated = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            post = json.loads(line)
        except json.JSONDecodeError:
            continue
        ack = post.get("ack_token")
        if not ack:
            continue
        for ref in post.get("refs", []) or []:
            if isinstance(ref, str) and ref.startswith("ACK-"):
                result = connection_observe(
                    root, f"board:{board}/{ack}", "replies-to", f"board:{board}/{ref}",
                    source_ref=f"board:{board}", created_by="yin-board-adapter",
                )
                if result["action"] == "created":
                    created += 1
                elif result["action"] == "strengthened":
                    strengthened += 1
                else:
                    deduplicated += 1
    return {"board": board, "created": created, "strengthened": strengthened, "deduplicated": deduplicated}


def scan_lobe_references(root: Path, lobes_dir: Path) -> dict:
    """Lobe adapter (observation-only): `lobe://<lobe>/<path>#<anchor>`
    references found INSIDE lobe records become observed edges between lobe
    addresses. The native address is preserved verbatim — no re-grammar."""
    if not lobes_dir.is_dir():
        raise SystemExit(f"connections: no lobes dir at {lobes_dir}")
    created = strengthened = deduplicated = 0
    unresolved = []
    for path in sorted(lobes_dir.rglob("*")):
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for match in re.findall(r"lobe://[A-Za-z0-9_\-./]+(?:#[A-Za-z0-9_\-]+)?", text):
            result = connection_observe(
                root, f"lobe:{path.relative_to(lobes_dir)}", "references", match,
                source_ref=f"lobe:{path.relative_to(lobes_dir)}", created_by="yin-lobe-adapter",
            )
            if result["action"] == "created":
                created += 1
            elif result["action"] == "strengthened":
                strengthened += 1
            else:
                deduplicated += 1
    return {"lobes_dir": str(lobes_dir), "created": created, "strengthened": strengthened, "deduplicated": deduplicated, "unresolved": unresolved}


def cmd_connections(args):
    """[slice 5] The connection plane: observe (Yin), propose (Yang), test,
    index, and the observation-only adapters (hub links, board ACK refs,
    lobe:// references). Sources are never rewritten; reruns never duplicate."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    if getattr(args, "index", False):
        result = connections_index(root, explore=bool(getattr(args, "explore", False)))
    elif getattr(args, "observe", False):
        result = connection_observe(
            root, args.from_addr, args.relation, args.to_addr,
            status=args.status, source_ref=args.source_ref, created_by=args.created_by,
        )
    elif getattr(args, "propose", False):
        result = connection_observe(
            root, args.from_addr, args.relation, args.to_addr,
            status="hypothesis", source_ref=args.source_ref, created_by=args.created_by,
            hypothesis=args.hypothesis, falsification_test=args.falsification_test,
            review_days=args.review_days,
        )
    elif getattr(args, "test", None):
        result = connection_test(root, args.test, args.outcome, evidence=args.source_ref)
    elif getattr(args, "scan_hub", None):
        result = scan_hub_links(root, args.scan_hub)
    elif getattr(args, "scan_board", None):
        boards_root = Path(os.environ.get("MSGGRAPH_BOARDS", str(Path.home() / "data" / "msggraph" / "boards")))
        result = scan_board_references(root, boards_root, args.scan_board)
    elif getattr(args, "scan_lobes", None):
        result = scan_lobe_references(root, Path(args.scan_lobes))
    else:
        raise SystemExit("connections: choose --index / --observe / --propose / --test ID / --scan-hub NS / --scan-board B / --scan-lobes DIR")
    print(json.dumps(result, indent=None if getattr(args, "json", False) else 2))


def cmd_claims(args):
    """[slice 3] Bounded retrieval over the persistent claim store — the
    compaction-safety guarantee made reachable: after any amount of brief
    capping, a rare early fact, an exception, or a correction is one
    evidence-linked query away."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    ns = detect_namespace(override=getattr(args, "ns", None))
    if not ns:
        raise SystemExit("claims: --ns is required")
    today = budget_today()
    state = load_claim_state(root, ns)
    rows = []
    query = (getattr(args, "query", "") or "").lower()
    want_status = getattr(args, "status", "all") or "all"
    include_dismissed = bool(getattr(args, "dismissed", False))
    for cid, entry in state.get("claims", {}).items():
        dismissed = claim_is_dismissed(root, cid, entry)
        if dismissed and not include_dismissed:
            # [slice 4] a dismissed claim does not resurface unqualified —
            # `--dismissed` is the review surface, never the default
            continue
        if include_dismissed and not dismissed:
            continue
        status = claim_status(entry, today)
        if want_status != "all" and status != want_status:
            continue
        if query and query not in str(entry.get("text", "")).lower():
            continue
        claim_id = getattr(args, "id", None)
        if claim_id and cid != claim_id:
            continue
        correction = claim_owner_correction(root, cid, entry)
        rows.append(
            {
                "id": cid,
                "status": status,
                "owner_dismissed": dismissed,
                **({"owner_correction": correction} if correction is not None else {}),
                **{k: entry.get(k) for k in ("text", "as_of", "proof", "last_seen", "valid_until", "review_after", "superseded_by", "supersedes", "source_refs", "confirmed", "restatements")},
            }
        )
    rows.sort(key=lambda r: str(r.get("last_seen", "")), reverse=True)
    rows = rows[: max(1, int(getattr(args, "limit", 20)))]
    print(json.dumps(rows, indent=None if getattr(args, "json", False) else 2))


def cmd_budget(args):
    """[slice 1] Fleet budget admission: atomically reserve token/cash against
    the ONE serialized authority. Exit 0 = admitted/settled; exit 3 = refused
    (a DEFER, the designed quota-cap sleep — never an error to retry later)."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    if getattr(args, "budget_status", False):
        op = {"action": "status"}
    elif getattr(args, "caps", None):
        op = {"action": "caps", "mode": args.caps, "for_days": args.for_days, "reason": args.purpose}
    elif getattr(args, "admit", False):
        op = {
            "action": "admit", "bucket": args.bucket,
            "input": args.input, "generated": args.generated, "cash_cents": args.cash_cents,
            "ttl": args.ttl, "purpose": args.purpose,
            "class": getattr(args, "work_class", None),
        }
    elif getattr(args, "settle", None):
        op = {
            "action": "settle", "id": args.settle,
            "input": args.input, "generated": args.generated, "cash_cents": args.cash_cents,
            "actual_source": args.actual_source,
        }
    elif getattr(args, "release", None):
        op = {"action": "release", "id": args.release}
    else:
        raise SystemExit("dream-budget: choose --admit / --settle ID / --release ID / --status")
    result = budget_op(root, op)
    print(json.dumps(result, indent=None if getattr(args, "json", False) else 2))
    refused = (
        (op["action"] == "admit" and not result.get("admitted"))
        or (op["action"] in ("settle", "release") and not (result.get("settled") or result.get("released")))
    )
    if refused:
        raise SystemExit(3)


def cmd_budget_op(args):
    """Hidden plumbing verb: the authority host executes one op from stdin
    under its local ledger flock. Always LOCAL — the router already decided
    this host is the authority when it ssh'd here."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    try:
        op = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError as error:
        raise SystemExit(f"_budget-op: bad stdin JSON: {error}")
    print(json.dumps(budget_run_local(root, op)))


def cmd_work_registry(args):
    """[slice 2] The owner-declared work-class registry. Mutations ride the
    budget authority plane (serialized with admissions) and can only TIGHTEN
    the seed ceilings — raising a budget is always outside autonomous
    authority (sol round-4 Q2)."""
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    if getattr(args, "list_registry", False):
        result = budget_op(root, {"action": "registry-read"})
        print(json.dumps(result, indent=None if getattr(args, "json", False) else 2))
        return
    class_id = args.class_id
    if not class_id:
        raise SystemExit("work-registry: --class-id is required for mutations")
    op = {"action": "registry-update", "class_id": class_id}
    if args.enable:
        op["enabled"] = True
    elif args.disable:
        op["enabled"] = False
    if args.input is not None or args.generated is not None or args.cash_cents is not None:
        seed = WORK_REGISTRY_CLASSES.get(class_id, {}).get("per_item", {})
        op["per_item"] = {
            "input": args.input if args.input is not None else seed.get("input", 0),
            "generated": args.generated if args.generated is not None else seed.get("generated", 0),
            "cash_cents": args.cash_cents if args.cash_cents is not None else seed.get("cash_cents", 0),
        }
    if "enabled" not in op and "per_item" not in op:
        raise SystemExit("work-registry: choose --enable / --disable / --set-per-item dimensions")
    result = budget_op(root, op)
    print(json.dumps(result, indent=None if getattr(args, "json", False) else 2))
    if not result.get("updated"):
        raise SystemExit(3)


def cmd_canary(args):
    """[mem-dream 3/4] Deterministic health probes, zero LLM: the index
    line budget (a client regenerator re-bloating MEMORY.md cannot recur
    silently), the archive pointer, NOW.md size, and dream-watermark
    monotonicity. Exit 1 names every violation; the tick logs failures."""
    violations = []
    home = Path.home()
    projects_root = home / ".zcode" / "cli" / "memories" / "projects"
    if projects_root.is_dir():
        for memory_dir in sorted(projects_root.glob("*/memory")):
            index = memory_dir / "MEMORY.md"
            if index.is_file():
                lines = sum(1 for _ in index.open(encoding="utf-8", errors="replace"))
                if lines > CANARY_MEMORY_MAX_LINES:
                    violations.append(
                        f"{memory_dir}: MEMORY.md {lines} lines > {CANARY_MEMORY_MAX_LINES} — a regenerator re-bloated the index?"
                    )
            if (memory_dir.parent / "memory-archive").is_dir() and not (
                memory_dir / "ARCHIVE-INDEX.md"
            ).is_file():
                violations.append(
                    f"{memory_dir}: memory-archive/ exists but the ARCHIVE-INDEX.md pointer is missing"
                )
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    namespaces = root / "namespaces"
    if namespaces.is_dir():
        for ns_dir in sorted(namespaces.glob("*")):
            now = ns_dir / "NOW.md"
            if now.is_file() and now.stat().st_size > DREAM_NOW_MAX_BYTES + 512:
                violations.append(
                    f"{ns_dir.name}: NOW.md {now.stat().st_size}B over budget"
                )
    dream_root = root / "dream"
    if dream_root.is_dir():
        for ns_dir in sorted(dream_root.glob("*/")):
            try:
                watermark = json.loads((ns_dir / "watermark.json").read_text(encoding="utf-8"))
            except Exception:
                continue
            seqs = [
                entry.get("seq")
                for entry in watermark.get("dreams", [])
                if isinstance(entry, dict) and entry.get("seq") is not None
            ]
            if any(later < earlier for earlier, later in zip(seqs, seqs[1:])):
                violations.append(f"{ns_dir.name}: dream watermark seqs not monotonic")

    # ── [slice 1, sol 2026-10-16] DREAM LIVENESS + PROGRESS CONTRACT ──────────
    # The canary is the INDEPENDENT monitor (a dreamer cannot report its own
    # absence — the 2026-10-06..16 silent death passed every prior probe).
    # Two separate checks: (1) EXECUTION liveness — did the controller tick, or
    # is it in a standing failure backoff? (2) SYNTHESIS progress — with work
    # pending, is source coverage advancing within the declared service bound?
    if dream_root.is_dir():
        receipt_path = dream_root / "receipts.jsonl"
        backoff_path = dream_root / "auto-backoff.json"
        remaining = 0.0
        try:
            stamp = json.loads(backoff_path.read_text(encoding="utf-8"))
            remaining = max(0.0, DREAM_AUTO_BACKOFF_S - (time.time() - float(stamp.get("ts", 0))))
            if remaining > 0:
                violations.append(
                    f"dreamer: standing FAILURE backoff ({int(remaining)}s left: "
                    f"{str(stamp.get('reason', '?'))[:120]}) — execution not healthy"
                )
        except Exception:
            pass
        last_receipt_ts = 0.0
        try:
            lines = [l for l in receipt_path.read_text(encoding="utf-8").splitlines() if l.strip()]
            if lines:
                last_receipt_ts = float(json.loads(lines[-1]).get("ts", 0))
        except Exception:
            pass
        # A live tick should leave receipts; absence over 2h with any pending
        # namespace means the controller is not running at all.
        for ns_dir in sorted(dream_root.glob("*/")):
            summary = _dream_pending_summary(root, ns_dir.name)
            if summary["pending"] <= 0:
                continue
            if summary["overdue"]:
                violations.append(
                    f"dreamer: ns {ns_dir.name} has {summary['pending']} pending deltas, "
                    f"oldest {summary['oldest_pending_s']}s > {DREAM_PROGRESS_DEADLINE_S}s — "
                    "synthesis progress deadline BLOWN (degraded freshness)"
                )
            if remaining <= 0 and last_receipt_ts and (
                time.time() - last_receipt_ts > DREAM_PROGRESS_DEADLINE_S
            ):
                violations.append(
                    f"dreamer: no execution receipt for {int(time.time() - last_receipt_ts)}s "
                    f"while ns {ns_dir.name} has {summary['pending']} pending — controller not ticking"
                )
    if violations:
        for violation in violations:
            print(f"canary: {violation}", file=sys.stderr)
        raise SystemExit(1)
    print("canary: all green" if not getattr(args, "json", False) else json.dumps({"status": "ok"}))


# ── [slice 1] FLEET BUDGET ADMISSION — one serialized authority ─────────────
# The race falsifier (two requests against the last allowance: both admitted
# = fail) requires a SINGLE serialized check-and-reserve point. The ledger
# lives on the authority host (dev — the same plane ygg-ci/deploy leases use)
# under flock; every other host reaches it through ssh. Authority-unreachable
# means DEFERRED, never a local-only guess: a hub-side journal merge is
# eventually consistent and cannot serialize admissions.

BUDGET_AUTHORITIES = {"dev", "oc", "jojo"}  # hosts that hold the ledger locally


def budget_authority_host() -> str:
    return os.environ.get("YGG_BUDGET_AUTHORITY", "dev").strip().lower()


def budget_ledger_path(root: Path) -> Path:
    return root / "dream" / "budget" / "ledger.json"


# ── [slice 2] WORK-CLASS REGISTRY — owner-declared, enforced in code ────────
# Sol round-4 Q2: the registry grants only explicitly listed operations;
# missing or contradictory authority produces a narrow forwarding item, never
# self-interpreted permission. Seeds are the OWNER ceilings (Q2 table): a
# mutation may tighten them, never raise them ("changing the registry's own
# permissions, or raising its budgets" is always outside autonomous authority).
WORK_REGISTRY_CLASSES = {
    "callgraph": {
        "per_item": {"input": 8_000, "generated": 600, "cash_cents": 4},
        "bucket": "work:callgraph",
        "forbidden": ["new-engine-purchase", "arbitrary-identity-merge"],
        "owner_authorization_ref": "sol-round4 2026-10-16 (Q2 table)",
    },
    "dossiergraph": {
        "per_item": {"input": 12_000, "generated": 1_200, "cash_cents": 8},
        "bucket": "work:dossiergraph",
        "forbidden": ["outward-action", "verified-bundle-without-hash-check"],
        "owner_authorization_ref": "sol-round4 2026-10-16 (Q2 table)",
    },
    "infra-updates": {
        "per_item": {"input": 4_000, "generated": 400, "cash_cents": 3},
        "bucket": "work:infra",
        "forbidden": ["jojo-reboot", "outside-target-allowlist"],
        "owner_authorization_ref": "sol-round4 2026-10-16 (Q2 table)",
    },
    "legacy-notes-conversion": {
        "per_item": {"input": 8_000, "generated": 800, "cash_cents": 5},
        "bucket": "work:conversion",
        "forbidden": ["semantic-rewrite", "invent-missing-material"],
        "owner_authorization_ref": "sol-round4 2026-10-16 (Q2 table)",
    },
    "learning-linkages": {
        "per_item": {"input": 4_000, "generated": 400, "cash_cents": 2},
        "bucket": "work:learning",
        "forbidden": ["alter-learner-achievements", "pattern-to-fit"],
        "owner_authorization_ref": "sol-round4 2026-10-16 (Q2 table)",
    },
    "consult-docs": {
        "per_item": {"input": 12_000, "generated": 1_200, "cash_cents": 8},
        "bucket": "work:consult-docs",
        "forbidden": ["amend-law", "self-authorize-implementation", "unattended-sol-high"],
        "owner_authorization_ref": "sol-round4 2026-10-16 (Q2 table)",
    },
}


def work_registry_path(root: Path) -> Path:
    return root / "dream" / "budget" / "work-registry.json"


def work_forward_path(root: Path) -> Path:
    return root / "dream" / "budget" / "forwarded.jsonl"


def load_work_registry(root: Path) -> dict:
    """The effective registry: seed ceilings clamped with any persisted
    (owner/seat-declared) TIGHTENINGS. A missing or corrupt file leaves the
    seeds standing — enforcement never defaults to permissive."""
    registry = {"version": 1, "classes": {}}
    path = work_registry_path(root)
    if path.is_file():
        try:
            registry = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            pass  # corrupt overrides fall back to seeds
    classes = {}
    for class_id, seed in WORK_REGISTRY_CLASSES.items():
        stored = (registry.get("classes") or {}).get(class_id, {})
        per_item = {
            dim: min(int(stored.get("per_item", {}).get(dim, seed["per_item"][dim])), seed["per_item"][dim])
            for dim in BUDGET_DIMS
        }
        classes[class_id] = {
            "enabled": bool(stored.get("enabled", True)),
            "per_item": per_item,
            "bucket": seed["bucket"],
            "forbidden": seed["forbidden"],
            "owner_authorization_ref": seed["owner_authorization_ref"],
            "updated_at": stored.get("updated_at"),
        }
    return {"version": registry.get("version", 1), "classes": classes}


def _budget_forward_tail(root: Path, limit: int = 20) -> list[dict]:
    """The bounded view of the owner-remainder queue (newest last)."""
    path = work_forward_path(root)
    if not path.is_file():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            rows.append(json.loads(line))
        except Exception:
            continue
    return rows[-limit:]


def work_registry_forward(root: Path, item: dict) -> None:
    """The narrow owner remainder (Q2: decision needed + evidence +
    anticipated default + resume checkpoint). Deduped — the same blockage
    appears ONCE in the queue, not once per refused attempt."""
    key = f"{item.get('kind')}:{item.get('class_id', '')}:{item.get('reason', '')}"
    seen = set()
    path = work_forward_path(root)
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                seen.add(json.loads(line).get("key"))
            except Exception:
                continue
    if key in seen:
        return
    item = {"key": key, "ts": datetime.datetime.now(datetime.timezone.utc).isoformat(), **item}
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(item) + "\n")


# ── [slice 2] FOCUS DOCS — the owner's priority doc (memory doc, not CLI) ──
# SSOT: hub door `_global/dream-focus.md` (owner-authored origin). Semantics
# per sol round-4 Q3 + owner addendum: boost ×4, 7d default expiry (explicit
# `expires:` honored), exclusive ≤24h, overlapping boosts take the MAX, and a
# selector that resolves to nothing stays UNRESOLVED instead of silently
# choosing a target. Focus reorders; it never grants authority.
FOCUS_DOC_NAMESPACE = "_global"
FOCUS_DOC_NAME = "dream-focus.md"
FOCUS_BOOST_WEIGHT = 4
FOCUS_STANDING_WEIGHT = 2
FOCUS_DEFAULT_DAYS = 7
FOCUS_EXCLUSIVE_MAX_S = 24 * 3600


def parse_focus_doc(text: str, today: str | None = None) -> dict:
    """Parse the owner's focus doc into active directives + standing
    constraints. Expiry is COMPUTED here, never stored: entries past their
    `expires:` (default today+7d) are pruned from the active set."""
    today = today or budget_today()
    today_d = datetime.date.fromisoformat(today)
    section = None
    directives: list[dict] = []
    constraints: list[dict] = []
    pending: dict | None = None

    def _flush():
        nonlocal pending
        if pending is None:
            return
        target = directives if pending["_section"] == "active" else constraints
        target.append({k: v for k, v in pending.items() if not k.startswith("_")})
        pending = None

    for raw_line in text.splitlines():
        line = raw_line.rstrip()
        if line.startswith("## "):
            _flush()
            heading = line[3:].strip().lower()
            section = "active" if "active focus" in heading else ("standing" if "standing" in heading else None)
            continue
        if section is None or line.startswith("#") or not line.strip():
            _flush()
            continue
        if "<!--" in line or line.strip().startswith("Owner:"):
            continue
        if line.lstrip().startswith("- "):
            _flush()
            pending = {
                "_section": section,
                "text": line.lstrip()[2:].strip(),
                "as_of": None,
                "expires": None,
                "exclusive": False,
                "scopes": [],
                "namespaces": [],
            }
        elif pending is not None:
            pending["text"] += " " + line.strip()
        else:
            continue
        body = pending["text"]
        stamp = re.search(r"\[as-of:\s*(\d{4}-\d{2}-\d{2})", body)
        if stamp:
            pending["as_of"] = stamp.group(1)
        expiry = re.search(r"expires:\s*(\d{4}-\d{2}-\d{2})", body)
        if expiry:
            pending["expires"] = expiry.group(1)
        scope_body = body
        if re.search(r"\bexclusive\b", scope_body, re.IGNORECASE):
            pending["exclusive"] = True
            # the bare flag ends the scope list — but only AFTER the scope
            # marker (a directive title may legitimately say "exclusive")
            scope_marker = scope_body.lower().find("scope:")
            excl_after = scope_body.lower().find("exclusive", scope_marker + 6) if scope_marker != -1 else -1
            if excl_after != -1:
                scope_body = scope_body[:excl_after]
        for chunk in re.findall(
            r"scope:\s*([a-zA-Z0-9_,\.\-]+(?:[\s,]+[a-zA-Z0-9_,\.\-]+)*)\s*(?=\]|\||;|$|\[)",
            scope_body,
        ):
            pending["scopes"] += [tok for tok in re.split(r"[\s,]+", chunk) if tok]
        pending["namespaces"] += re.findall(r"(-[a-z0-9][a-z0-9\-]{3,}|_global)", body)
    _flush()

    active = []
    for directive in directives:
        expires = directive.get("expires")
        if not expires:
            if directive.get("exclusive"):
                # exclusive caps at ≤24h when the owner gives no duration
                expires = (today_d + datetime.timedelta(days=1)).isoformat()
            else:
                expires = (today_d + datetime.timedelta(days=FOCUS_DEFAULT_DAYS)).isoformat()
        directive = {**directive, "expires": expires}
        if expires >= today:
            active.append(directive)
    return {"today": today, "directives": active, "constraints": constraints}


def focus_scope_matches(selector: str, target: str) -> bool:
    """A selector resolves against a namespace or work-class id by normalized
    substring; anything else stays unresolved (never a silent guess)."""
    def norm(value: str) -> str:
        return re.sub(r"[\s_]+", "-", value.strip().lower()).strip("-")

    sel, tgt = norm(selector), norm(target)
    if not sel:
        return False
    return sel in tgt or tgt in sel


def _read_ns_file(root: Path, ns: str, filename: str) -> str:
    path = get_namespace_dir(root, ns, create=False) / filename
    if not path.is_file():
        return ""
    return path.read_text(encoding="utf-8", errors="replace")


def focus_view(root: Path, ns: str, today: str | None = None) -> dict:
    """The resolved focus state for one namespace: weight (MAX across
    matching directives — never multiplied), exclusive participation, and
    exactly what resolved vs stayed unresolved."""
    doc = parse_focus_doc(_read_ns_file(root, FOCUS_DOC_NAMESPACE, FOCUS_DOC_NAME), today=today)
    matched, unresolved = [], []
    weight = 1
    exclusive = False
    for directive in doc["directives"]:
        selectors = directive.get("scopes") or []
        if not selectors:
            unresolved.append(directive["text"][:90])
            continue
        if any(focus_scope_matches(selector, ns) for selector in selectors):
            matched.append(directive["text"][:90])
            weight = max(weight, FOCUS_BOOST_WEIGHT)
            if directive.get("exclusive"):
                exclusive = True
    if weight == 1:
        for constraint in doc["constraints"]:
            if any(focus_scope_matches(token, ns) for token in constraint.get("namespaces", [])):
                weight = FOCUS_STANDING_WEIGHT
                matched.append("(standing) " + constraint["text"][:80])
                break
    return {"ns": ns, "weight": weight, "exclusive": exclusive, "matched": matched, "unresolved": unresolved}


def budget_today() -> str:
    return datetime.datetime.now(datetime.timezone.utc).date().isoformat()


def _budget_policy() -> dict:
    """A ledger may carry a SMALLER sandbox policy (tests); it can never
    exceed the ratified fleet policy — a hand-edited bigger cap silently
    re-opening the spend floodgates is exactly what this overlay exists
    to prevent."""
    def _min_dim(policy: dict, ratified: dict) -> dict:
        return {dim: min(int(policy.get(dim, ratified[dim])), ratified[dim]) for dim in BUDGET_DIMS}

    policy = {
        "totals": dict(BUDGET_TOTALS),
        "buckets": {name: {**spec, "borrowable": bool(spec.get("borrowable", True))} for name, spec in BUDGET_BUCKETS.items()},
    }
    ledger_policy = os.environ.get("YGG_BUDGET_POLICY_JSON")
    if not ledger_policy:
        return policy
    try:
        supplied = json.loads(ledger_policy)
        policy["totals"] = _min_dim(supplied.get("totals", {}), BUDGET_TOTALS)
        buckets = {}
        for name, spec in supplied.get("buckets", {}).items():
            ratified = policy["buckets"].get(name)
            if ratified is None:
                buckets[name] = {**_min_dim(spec, BUDGET_TOTALS), "borrowable": bool(spec.get("borrowable", True))}
            else:
                buckets[name] = {**_min_dim(spec, ratified), "borrowable": bool(spec.get("borrowable", ratified["borrowable"]))}
        policy["buckets"] = buckets
    except Exception:
        pass  # a malformed override leaves the ratified policy standing
    return policy


def _budget_zero_dims() -> dict:
    return {dim: 0 for dim in BUDGET_DIMS}


def _budget_dims_all_nonnegative(dims: dict) -> bool:
    return all(isinstance(dims.get(dim, 0), int) and dims.get(dim, 0) >= 0 for dim in BUDGET_DIMS)


def _budget_sum_holds(reservations: dict, day: str, bucket: str | None = None) -> dict:
    """Open (state=reserved) holds, keyed to their RESERVATION day: a hold
    made at 23:59 stays charged across midnight while in flight (Q4)."""
    total = _budget_zero_dims()
    for res in reservations.values():
        if res.get("state") != "reserved":
            continue
        if day is not None and res.get("day") != day:
            continue
        if bucket is not None and res.get("bucket") != bucket:
            continue
        for dim in BUDGET_DIMS:
            total[dim] += int(res.get(dim, 0))
    return total


def _budget_sweep_expired(ledger: dict, now_s: float) -> list[dict]:
    released = []
    for res_id, res in list(ledger.get("reservations", {}).items()):
        if res.get("state") == "reserved" and float(res.get("expires_at", 0)) < now_s:
            res["state"] = "expired"
            res["settled_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            released.append(
                {
                    "kind": "expired", "id": res_id, "bucket": res.get("bucket"),
                    "day": res.get("day"), "purpose": str(res.get("purpose", ""))[:120],
                }
            )
    return released


def _budget_roll_day(ledger: dict) -> str | None:
    today = budget_today()
    if ledger.get("day") == today:
        return None
    previous = ledger.get("day")
    ledger["day"] = today
    spent = ledger.setdefault("spent", {})
    spent.setdefault(today, {"_total": _budget_zero_dims()})
    # bound the archive: the receipts carry the durable history
    for day in sorted(list(spent.keys()))[: max(0, len(spent) - BUDGET_ARCHIVE_DAYS)]:
        spent.pop(day, None)
    return previous


def _budget_receipt(ledger: dict, entry: dict) -> None:
    entry["ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    receipts = ledger.setdefault("receipts", [])
    receipts.append(entry)
    ledger["receipts"] = receipts[-BUDGET_RECEIPTS_MAX:]


def _budget_op(root: Path, op: dict) -> dict:
    """THE serialized budget core. Runs under the ledger flock (locally or on
    the authority host via _budget-op). Sweeps expired holds, rolls the day,
    then executes admit/settle/release/status."""
    ledger_path = budget_ledger_path(root)
    now_s = time.time()
    if ledger_path.is_file():
        try:
            ledger = json.loads(ledger_path.read_text(encoding="utf-8"))
        except Exception:
            ledger = {}
    else:
        ledger = {}
    ledger.setdefault("version", 1)
    ledger["policy"] = _budget_policy()
    ledger.setdefault("spent", {}).setdefault(budget_today(), {"_total": _budget_zero_dims()})
    ledger.setdefault("reservations", {})
    expired = _budget_sweep_expired(ledger, now_s)
    for entry in expired:
        _budget_receipt(ledger, entry)
    rolled = _budget_roll_day(ledger)
    if rolled:
        _budget_receipt(ledger, {"kind": "day-roll", "from": rolled, "to": ledger["day"]})

    action = op.get("action")
    policy = ledger["policy"]
    totals = policy["totals"]
    spent = ledger["spent"]
    day = ledger["day"]
    today_spent = spent.setdefault(day, {"_total": _budget_zero_dims()})

    if action == "admit":
        bucket = str(op.get("bucket", ""))
        class_refusal = None
        class_id = op.get("class")
        if class_id:
            # [slice 2] AUTHORITY GATE — enforced here in serialized code,
            # never interpreted by the model. Missing/contradictory authority
            # produces a narrow forward, not self-granted permission. Focus
            # never reaches this plane: a boost cannot grant a class.
            cls = load_work_registry(root)["classes"].get(str(class_id))
            if cls is None:
                work_registry_forward(root, {
                    "kind": "unknown-class", "class_id": str(class_id),
                    "reason": "class not in the owner registry",
                    "decision_needed": f"declare work class '{class_id}' or stop attempting it",
                    "evidence": ["sol round-4 Q2: registry grants only listed operations"],
                    "anticipated_default": "work stays blocked; no admission",
                    "resume": "after the owner declares the class",
                })
                class_refusal = f"unknown-class:{class_id}"
            elif not cls["enabled"]:
                work_registry_forward(root, {
                    "kind": "class-disabled", "class_id": str(class_id),
                    "reason": "class is disabled in the registry",
                    "decision_needed": f"re-enable '{class_id}' or keep it blocked",
                    "evidence": [f"registry updated_at {cls.get('updated_at')}"],
                    "anticipated_default": "work stays blocked; no admission",
                    "resume": "after the owner re-enables the class",
                })
                class_refusal = f"class-disabled:{class_id}"
            else:
                bucket = cls["bucket"]
                per_item = cls["per_item"]
                est_check = {dim: int(op.get(dim, 0)) for dim in BUDGET_DIMS}
                over = [dim for dim in BUDGET_DIMS if est_check[dim] > per_item[dim]]
                if over:
                    work_registry_forward(root, {
                        "kind": "over-item-ceiling", "class_id": str(class_id),
                        "reason": f"estimate exceeds the per-item ceiling on {','.join(over)}",
                        "decision_needed": "split the item, or raise the ceiling (owner only)",
                        "evidence": [f"estimate {est_check} vs per-item {per_item}"],
                        "anticipated_default": "forward to the owner's active session",
                        "resume": "with a split estimate or an owner raise",
                    })
                    class_refusal = f"over-item-ceiling:{','.join(over)}"
        if class_refusal:
            result = {"admitted": False, "reason": class_refusal}
        else:
            spec = policy["buckets"].get(bucket)
            if spec is None:
                result = {"admitted": False, "reason": f"unknown-bucket:{bucket}"}
            elif not _budget_dims_all_nonnegative(op):
                result = {"admitted": False, "reason": "bad-estimate"}
            else:
                est = {dim: int(op.get(dim, 0)) for dim in BUDGET_DIMS}
                if all(v == 0 for v in est.values()):
                    result = {"admitted": False, "reason": "zero-estimate"}
                else:
                    holds_total = _budget_sum_holds(ledger["reservations"], None)
                    # [owner ruling 2026-10-16] CAPS MODE: quality of work
                    # takes precedence over tokenomics; agents may empty
                    # provider quotas but never WASTE tokens (per-item
                    # ceilings, estimate discipline, and dedupe still guard
                    # that). The hard totals are an owner-instantiated
                    # device — set only when the owner says so for a period
                    # (`dream-budget --caps hard`). Default is soft: the
                    # ledger still serializes, accounts, and flags
                    # overborrow, but capacity never refuses work; dreams
                    # stall on PROVIDER quota hits (the designed backoff),
                    # not on our arithmetic.
                    caps_mode = ledger.get("caps_mode", "soft")
                    if os.environ.get("YGG_BUDGET_CAPS"):  # test/sandbox override
                        caps_mode = os.environ["YGG_BUDGET_CAPS"]
                    if caps_mode == "hard" and ledger.get("caps_until") and day > ledger["caps_until"]:
                        caps_mode = "soft"  # the owner-instantiated window elapsed; auto-restore
                    hard_room = {dim: totals[dim] - today_spent["_total"].get(dim, 0) - holds_total[dim] for dim in BUDGET_DIMS}
                    exhausted = [dim for dim in BUDGET_DIMS if est[dim] > hard_room[dim]]
                    if exhausted and caps_mode == "hard":
                        result = {
                            "admitted": False,
                            "reason": "exhausted:" + ",".join(exhausted),
                            "headroom": hard_room,
                        }
                    else:
                        bucket_spent = today_spent.setdefault(bucket, _budget_zero_dims())
                        holds_bucket = _budget_sum_holds(ledger["reservations"], None, bucket)
                        bucket_room = {dim: spec[dim] - bucket_spent.get(dim, 0) - holds_bucket[dim] for dim in BUDGET_DIMS}
                        borrowed = caps_mode == "soft" or any(est[dim] > bucket_room[dim] for dim in BUDGET_DIMS)
                        if borrowed and caps_mode == "hard" and not spec.get("borrowable", True):
                            result = {"admitted": False, "reason": f"bucket-exhausted:{bucket}", "bucket_room": bucket_room}
                        else:
                            res_id = uuid.uuid4().hex[:16]
                            ttl = max(1, int(op.get("ttl", BUDGET_RESERVATION_TTL_S)))
                            reservation = {
                                "day": day, "bucket": bucket, "state": "reserved",
                                "caps_mode": caps_mode,
                                "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                                "expires_at": now_s + ttl, "host": op.get("host", ""), "pid": op.get("pid"),
                                "purpose": str(op.get("purpose", ""))[:200], "borrowed": borrowed,
                                "class_id": str(class_id) if class_id else None,
                                **est,
                            }
                            ledger["reservations"][res_id] = reservation
                            _budget_receipt(ledger, {"kind": "admitted", "id": res_id, "bucket": bucket, **est, "borrowed": borrowed, "purpose": reservation["purpose"]})
                            result = {
                                "admitted": True, "id": res_id, "bucket": bucket,
                                "estimate": est, "borrowed": borrowed, "expires_at": reservation["expires_at"],
                                "day": day,
                            }
    elif action == "settle":
        res_id = str(op.get("id", ""))
        res = ledger["reservations"].get(res_id)
        if res is None:
            result = {"settled": False, "reason": "unknown-id"}
        elif res["state"] not in ("reserved", "expired"):
            result = {"settled": False, "reason": f"already-{res['state']}"}
        elif res.get("class_id") and not load_work_registry(root)["classes"].get(res["class_id"], {}).get("enabled"):
            # [slice 2] THE PRE-COMMIT RECHECK (Q2 admission flow step 6):
            # authority revoked between admit and settle invalidates the
            # pending write — the class may not commit, the hold releases.
            res["state"] = "settle-refused"
            res["settled_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            _budget_receipt(ledger, {"kind": "settle-refused", "id": res_id, "bucket": res["bucket"], "why": "class-revoked", "class_id": res["class_id"]})
            work_registry_forward(root, {
                "kind": "settle-refused-class-revoked", "class_id": res["class_id"],
                "reason": f"reservation {res_id} finished after its class was revoked",
                "decision_needed": "keep the class disabled (work is void) or re-enable and redo",
                "evidence": [f"reservation {res_id} admitted {res.get('created_at')}"],
                "anticipated_default": "work product discarded, not committed",
                "resume": "after an owner ruling on the class",
            })
            result = {"settled": False, "reason": "class-revoked", "id": res_id}
        else:
            actual = {dim: int(op.get(dim, 0)) for dim in BUDGET_DIMS}
            res_day_spent = spent.setdefault(res["day"], {"_total": _budget_zero_dims()})
            res_day_spent["_total"] = {dim: res_day_spent["_total"].get(dim, 0) + actual[dim] for dim in BUDGET_DIMS}
            bucket_spent = res_day_spent.setdefault(res["bucket"], _budget_zero_dims())
            bucket_spent.update({dim: bucket_spent.get(dim, 0) + actual[dim] for dim in BUDGET_DIMS})
            late = res["state"] == "expired" or res["day"] != day
            res["state"] = "settled-late" if late else "settled"
            res["settled_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            res["actual"] = actual
            res["actual_source"] = str(op.get("actual_source", "measured"))
            _budget_receipt(ledger, {"kind": "settled", "id": res_id, "bucket": res["bucket"], "day": res["day"], **actual, "late": late})
            result = {"settled": True, "id": res_id, "booked_day": res["day"], "actual": actual, "late": late}
    elif action == "release":
        res_id = str(op.get("id", ""))
        res = ledger["reservations"].get(res_id)
        if res is None:
            result = {"released": False, "reason": "unknown-id"}
        elif res["state"] != "reserved":
            result = {"released": False, "reason": f"already-{res['state']}"}
        else:
            res["state"] = "released"
            res["settled_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            _budget_receipt(ledger, {"kind": "released", "id": res_id, "bucket": res["bucket"], "purpose": str(res.get("purpose", ""))[:120]})
            result = {"released": True, "id": res_id}
    elif action == "caps":
        # [owner ruling 2026-10-16] The owner (through a seat, on his spoken
        # direction) instantiates or lifts a hard-cap window. Soft is the
        # standing state: quality first, no waste, provider quotas may empty.
        mode = op.get("mode")
        if mode not in ("hard", "soft"):
            result = {"set": False, "reason": "mode is hard|soft"}
        else:
            ledger["caps_mode"] = mode
            for_days = int(op.get("for_days") or 0)
            ledger["caps_until"] = (
                (datetime.date.fromisoformat(day) + datetime.timedelta(days=for_days)).isoformat()
                if mode == "hard" and for_days > 0 else None
            )
            _budget_receipt(ledger, {"kind": "caps-mode", "mode": mode, "until": ledger.get("caps_until"), "reason": str(op.get("reason", ""))[:160]})
            result = {"set": True, "caps_mode": mode, "caps_until": ledger.get("caps_until"), "reason": op.get("reason", "")}
    elif action == "registry-update":
        # [slice 2] Registry mutations ride the SAME serialized plane as
        # admissions (a revoke must be visible to every host's next check).
        # Stored deltas are TIGHTENINGS only: load_work_registry clamps
        # against the owner seed ceilings on every read.
        class_id = str(op.get("class_id", ""))
        if class_id not in WORK_REGISTRY_CLASSES:
            result = {"updated": False, "reason": f"unknown-class:{class_id}"}
        else:
            reg_path = work_registry_path(root)
            stored = {"version": 1, "classes": {}}
            if reg_path.is_file():
                try:
                    stored = json.loads(reg_path.read_text(encoding="utf-8"))
                except Exception:
                    pass
            entry = (stored.setdefault("classes", {}).setdefault(class_id, {}))
            changes = {}
            if op.get("enabled") is not None:
                entry["enabled"] = bool(op["enabled"])
                changes["enabled"] = bool(op["enabled"])
            if op.get("per_item"):
                seed = WORK_REGISTRY_CLASSES[class_id]["per_item"]
                entry["per_item"] = {
                    dim: min(int(op["per_item"].get(dim, entry.get("per_item", {}).get(dim, seed[dim]))), seed[dim])
                    for dim in BUDGET_DIMS
                }
                changes["per_item"] = entry["per_item"]
            entry["updated_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            reg_path.parent.mkdir(parents=True, exist_ok=True)
            tmp = reg_path.with_suffix(".json.tmp")
            tmp.write_text(json.dumps(stored, indent=1), encoding="utf-8")
            tmp.replace(reg_path)
            _budget_receipt(ledger, {"kind": "registry-update", "class_id": class_id, **changes})
            result = {"updated": True, "class_id": class_id, "changes": changes, "effective": load_work_registry(root)["classes"][class_id]}
    elif action == "registry-read":
        result = {"classes": load_work_registry(root)["classes"]}
    elif action == "status":
        holds_total = _budget_sum_holds(ledger["reservations"], None)
        hard_room = {dim: totals[dim] - today_spent["_total"].get(dim, 0) - holds_total[dim] for dim in BUDGET_DIMS}
        buckets = {}
        for name, spec in policy["buckets"].items():
            bucket_spent = today_spent.get(name, _budget_zero_dims())
            holds_bucket = _budget_sum_holds(ledger["reservations"], None, name)
            buckets[name] = {
                "cap": {dim: spec[dim] for dim in BUDGET_DIMS},
                "spent": {dim: bucket_spent.get(dim, 0) for dim in BUDGET_DIMS},
                "holds": holds_bucket,
                "room": {dim: spec[dim] - bucket_spent.get(dim, 0) - holds_bucket[dim] for dim in BUDGET_DIMS},
                "borrowable": bool(spec.get("borrowable", True)),
            }
        result = {
            "day": day,
            "caps_mode": ledger.get("caps_mode", "soft"),
            "caps_until": ledger.get("caps_until"),
            "totals": totals,
            "hard_room": hard_room,
            "holds_total": holds_total,
            "open_reservations": {
                rid: {k: res.get(k) for k in ("day", "bucket", "created_at", "expires_at", "purpose", "borrowed", "class_id", "input", "generated", "cash_cents", "host")}
                for rid, res in ledger["reservations"].items() if res.get("state") == "reserved"
            },
            "buckets": buckets,
            "work_registry": load_work_registry(root)["classes"],
            "forwarded": _budget_forward_tail(root),
            "jev_envelope_cents": BUDGET_JEV_ENVELOPE_CENTS,
            "expired_released_now": [entry.get("id") for entry in expired],
            "rolled_from": rolled,
            "recent_receipts": ledger.get("receipts", [])[-12:],
        }
    else:
        result = {"error": f"unknown action {action}"}

    ledger_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = ledger_path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(ledger, indent=1), encoding="utf-8")
    tmp.replace(ledger_path)
    return result


def budget_run_local(root: Path, op: dict) -> dict:
    lock = _flock_open(root / ".ygg-budget.lock", timeout_seconds=30)
    try:
        return _budget_op(root, op)
    finally:
        _flock_close(lock)


def budget_run_remote(host: str, op: dict) -> dict:
    module = os.environ.get("YGG_BUDGET_REMOTE_MODULE", "~/.local/bin/ygg-memory.py")
    command = _fleet_ssh_command(10) + [
        host,
        f"python3 {module} _budget-op",
    ]
    run_ = subprocess.run(command, input=json.dumps(op), capture_output=True, text=True, timeout=60)
    if run_.returncode != 0:
        return {"admitted": False, "error": "authority-unreachable", "detail": (run_.stderr or "")[-300:]}
    return json.loads(run_.stdout)


def budget_op(root: Path, op: dict) -> dict:
    """Route one budget op to the serialized authority: locally when this IS
    the authority host, over ssh otherwise. op["host"] records the requester."""
    op = {**op, "host": socket.gethostname()}
    authority = budget_authority_host()
    hostname = socket.gethostname().lower()
    is_authority = authority == "local" or (hostname in BUDGET_AUTHORITIES and authority == hostname)
    if is_authority:
        return budget_run_local(root, op)
    return budget_run_remote(authority, op)


def budget_estimate_tokens(text_bytes: int) -> int:
    return max(1, text_bytes // BUDGET_TOKENS_PER_BYTE)


def dream_budget_admission(root: Path, input_bytes: int) -> dict:
    """[slice 1] The dream compose admission: estimate the composer+critic
    passes, atomically reserve against the fleet ledger. Refusal means DEFER
    (the designed quota-cap sleep), never a local guess."""
    return budget_op(
        root,
        {
            "action": "admit",
            "bucket": BUDGET_DREAM_BUCKET,
            "input": budget_estimate_tokens(input_bytes),
            "generated": BUDGET_DREAM_GEN_ESTIMATE,
            "cash_cents": BUDGET_DREAM_CASH_CENTS,
            "purpose": f"dream compose {input_bytes}B",
        },
    )


def dream_budget_settle(root: Path, reservation: dict, input_bytes: int, output_bytes: int) -> dict:
    return budget_op(
        root,
        {
            "action": "settle",
            "id": reservation["id"],
            "input": budget_estimate_tokens(input_bytes),
            "generated": budget_estimate_tokens(output_bytes),
            "cash_cents": reservation["estimate"]["cash_cents"],
            "actual_source": "byte-estimate",
        },
    )


def cmd_dream(args):
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    harness = detect_harness(args.harness)
    ns = detect_namespace(override=getattr(args, "ns", None))
    if not ns:
        raise SystemExit("dream: --ns is required")

    if getattr(args, "auto", False):
        message = _dream_auto(root, getattr(args, "ns", None), args)
        if getattr(args, "json", False):
            print(json.dumps({"status": message}))
        elif not getattr(args, "quiet", False):
            print(message)
        return

    if args.status:
        watermark = load_dream_watermark(root, ns)
        pending = get_dream_dir(root, ns, create=False) / "pending"
        state = {
            "ns": ns,
            "dream_watermark_seq": watermark.get("last_seq", 0),
            "last_dream_ts": watermark.get("last_dream_ts"),
            "pending_brief": (pending / "BRIEF.md").is_file(),
            "last_dreams": watermark.get("dreams", [])[-3:],
        }
        if getattr(args, "json", False):
            print(json.dumps(state))
        else:
            print(json.dumps(state, indent=2))
        return

    if args.prepare or args.run:
        if not getattr(args, "force", False):
            violation = dream_quiescence_violation(root, ns)
            if violation:
                raise SystemExit(f"dream: REFUSED — {violation}")
        staged = prepare_dream(
            root, ns, args.phase, args.extra_input, args.budget or DREAM_INPUT_BUDGET_BYTES
        )
        if staged is None:
            message = f"dream: no delta since seq {load_dream_watermark(root, ns)['last_seq']} — clean tick, no brief written"
            if getattr(args, "json", False):
                print(json.dumps({"status": "clean-tick"}))
            else:
                print(message)
            if not args.run:
                return
            raise SystemExit("dream --run: nothing to dream (clean tick)")

    if args.run:
        pending = get_dream_dir(root, ns) / "pending"
        brief = (pending / "BRIEF.md").read_text(encoding="utf-8")
        # [slice 1] BUDGET ADMISSION before any composer call — no compose
        # without a fleet-wide atomic reserve. Refusal is the designed
        # quota-cap sleep (round-3 sol Q-D: the red canary exposes it; the
        # tick backs off).
        admission = dream_budget_admission(root, len(brief.encode("utf-8")))
        if not admission.get("admitted"):
            headroom = json.dumps(admission.get("headroom", {}))
            raise SystemExit(
                f"dream: DEFERRED — budget admission refused "
                f"({admission.get('reason', 'unknown')}; hard room {headroom}; "
                f"retry after spend/settle or the next day boundary)"
            )
        try:
            manifest_path = pending / "MANIFEST.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["budget_reservation"] = admission["id"]
            manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
            codex_bin = _resolve_codex_bin(args.codex_bin)
            print(f"dream: composer pass ({args.model}, effort {args.effort})… [budget {admission['id']}{', borrowed' if admission.get('borrowed') else ''}]")
            response = _codex_exec(codex_bin, args.model, args.effort, brief + "\n\nProduce the response now.")
            (pending / "RESPONSE.md").write_text(response, encoding="utf-8")
            print("dream: critic pass…")
            critic_prompt = (
                brief
                + "\n\n===== COMPOSER OUTPUT =====\n"
                + response
                + "\n\n"
                + CRITIC_INSTRUCTIONS
            )
            critic = _codex_exec(codex_bin, args.model, args.effort, critic_prompt)
            (pending / "CRITIC.md").write_text(critic, encoding="utf-8")
        except BaseException:
            budget_op(root, {"action": "release", "id": admission["id"]})
            raise
        # Both passes ran: the tokens are burned whether or not apply
        # succeeds — settle now, charged to the admission's day (survives
        # midnight: an in-flight reservation books to its own day).
        settle = dream_budget_settle(
            root, admission,
            len((brief + "\n\nProduce the response now." + critic_prompt).encode("utf-8")),
            len((response + critic).encode("utf-8")),
        )
        if not settle.get("settled"):
            print(f"dream: budget settle skipped ({settle.get('reason')}) — hold expires by TTL", file=sys.stderr)
        args.apply = True

    if args.apply:
        dream_dir = get_dream_dir(root, ns, create=False)
        pending = dream_dir / "pending"
        manifest_path = pending / "MANIFEST.json"
        response_path = pending / "RESPONSE.md"
        if not manifest_path.is_file() or not response_path.is_file():
            raise SystemExit(f"dream: no pending dream in {pending} (run --prepare, compose RESPONSE.md first)")
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        try:
            parsed = parse_dream_response(response_path.read_text(encoding="utf-8"))
            critic_path = pending / "CRITIC.md"
            verdicts = parse_critic_verdicts(critic_path.read_text(encoding="utf-8")) if critic_path.is_file() else []
        except ValueError as error:
            print(f"dream: REFUSED — {error}", file=sys.stderr)
            raise SystemExit(2)
        # [slice 3] the fence scope is the composed batch itself plus the
        # doors the ops write: an unrelated concurrent write stays pending
        # for the next dream instead of refusing this one forever.
        composed_scope = list(manifest.get("delta_files", []))
        composed_scope += [op.get("file") for op in parsed["ops"] if op.get("file")]
        fence = dream_fence_violation(root, ns, manifest.get("captured_seq", 0), composed_scope)
        if fence:
            raise SystemExit(f"dream: REFUSED — {fence}")
        ns_dir = get_namespace_dir(root, ns, create=False)
        errors = validate_dream_response(parsed, verdicts, ns_dir)
        if errors:
            print(f"dream: REFUSED — {len(errors)} validation error(s):", file=sys.stderr)
            for error in errors:
                print(f"  - {error}", file=sys.stderr)
            raise SystemExit(2)

        contested = sum(1 for v in verdicts if v.get("verdict") == "contested")
        if getattr(args, "dry_run", False):
            plan = {
                "dry_run": True,
                "now_md_bytes": len(parsed["now_md"].encode("utf-8")),
                "claims": len(parsed["claims"]),
                "contested": contested,
                "ops": parsed["ops"],
                "owner_now": bool(parsed["owner_now"]),
            }
            print(json.dumps(plan, indent=2))
            return

        lock = _flock_open(root / ".ygg-memory.lock")
        try:
            today = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
            description = _dream_first_claim_line(parsed["now_md"]) or "ns rollup"
            now_content = (
                "---\n"
                "name: NOW\n"
                f"description: {description}\n"
                "metadata:\n"
                "  type: now\n"
                f"  origin: {DREAM_ORIGIN}\n"
                f"  as_of: {today}\n"
                f"  phase: {manifest['phase']}\n"
                "---\n\n"
                + parsed["now_md"]
                + "\n"
            )
            record = _dream_publish_door(
                root, ns, "NOW.md", now_content, harness, "now",
                f"dream rollup {today} (phase {manifest['phase']}, {len(parsed['claims'])} claims, {contested} contested) [consumed seq {manifest['captured_seq']}]",
            )
            _upsert_ns_index_line(ns_dir, "NOW.md", description)

            if parsed["owner_now"]:
                owner = parsed["owner_now"]
                owner_content = (
                    "---\n"
                    "name: owner-now\n"
                    "description: fast-rotting owner context (dreamer-maintained)\n"
                    "metadata:\n"
                    "  type: owner-now\n"
                    f"  origin: {DREAM_ORIGIN}\n"
                    f"  as_of: {today}\n"
                    "---\n\n"
                    + owner["body"]
                    + "\n"
                )
                _dream_publish_door(
                    root, "_global", "owner-now.md", owner_content, harness, "owner-now",
                    f"owner-now update {today} (evidence: {', '.join(owner.get('evidence', [])[:3])})",
                )
                _upsert_ns_index_line(get_namespace_dir(root, "_global"), "owner-now.md", "fast-rotting owner context (dreamer-maintained)")

            applied_ops = []
            for op in parsed["ops"]:
                if op["op"] == "append-section":
                    door_path = ns_dir / op["file"]
                    _dream_append_section(door_path, op["section"], op["entry"])
                    _dream_publish_door(
                        root, ns, op["file"], door_path.read_text(encoding="utf-8"), harness, "door",
                        f"dream op: append {op['section']} ({op['reason'][:80]})",
                    )
                    applied_ops.append({**op, "applied": True})
                else:
                    applied_ops.append({**op, "applied": False, "note": "proposal logged only"})
        finally:
            _flock_close(lock)

        # [slice 2] fold CLAIMS into persistent claim state (supersession
        # edges preserved; evidence dates sticky). Contested claims are NOT
        # stored as ok — the critic's demotion is respected.
        try:
            state = load_claim_state(root, ns)
            state = merge_claims(state, parsed["claims"], today)
            save_claim_state(root, ns, state)
        except Exception as state_error:
            print(f"dream: claim-state persistence skipped: {state_error}", file=sys.stderr)

        watermark = load_dream_watermark(root, ns)
        dream_record = {
            "applied_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "captured_seq": manifest["captured_seq"],
            "deltas": manifest["delta_count"],
            "now_bytes": len(parsed["now_md"].encode("utf-8")),
            "claims": len(parsed["claims"]),
            "contested": contested,
            "ops": len(parsed["ops"]),
            "journal_seq": record["seq"],
        }
        watermark["last_seq"] = manifest["captured_seq"]
        watermark["last_dream_ts"] = dream_record["applied_at"]
        watermark.setdefault("dreams", []).append(dream_record)
        watermark["dreams"] = watermark["dreams"][-20:]
        save_dream_watermark(root, ns, watermark)

        done = dream_dir / "done" / f"{dream_record['applied_at'].replace(':', '').replace('-', '')[:15]}-seq{manifest['captured_seq']}"
        done.mkdir(parents=True, exist_ok=True)
        oplog = [
            f"# DREAM OPLOG — {ns} — {dream_record['applied_at']}",
            "",
            f"manifest: {json.dumps(manifest)}",
            f"watermark: seq {manifest['watermark_seq']} → {manifest['captured_seq']}",
            f"brief sha256: {file_sha256(pending / 'BRIEF.md')}",
            f"response sha256: {file_sha256(response_path)}",
            f"critic sha256: {file_sha256(critic_path)}" if critic_path.is_file() else "critic: (none — zero claims)",
            f"verdicts: {json.dumps(verdicts)}",
            f"ops: {json.dumps(applied_ops, indent=2)}",
            f"dream record: {json.dumps(dream_record)}",
        ]
        (done / "OPLOG.md").write_text("\n".join(oplog) + "\n", encoding="utf-8")
        for name in ("BRIEF.md", "MANIFEST.json", "RESPONSE.md", "CRITIC.md"):
            source = pending / name
            if source.is_file():
                shutil.move(str(source), str(done / name))
        shutil.rmtree(pending, ignore_errors=True)

        print(
            f"dream: applied — NOW.md {dream_record['now_bytes']}B, {dream_record['claims']} claims "
            f"({contested} contested), {len(applied_ops)} op(s); watermark → seq {watermark['last_seq']}; "
            f"oplog {done / 'OPLOG.md'}"
        )
        print("dream: run `ygg-memory sync-harness --all` to propagate NOW.md into native stores")


def cmd_search(args):
    root = Path(getattr(args, "root", None) or DEFAULT_MEMORY_ROOT)
    explicit_ns = getattr(args, "ns", None)
    if args.all or not explicit_ns:
        namespaces_root = root / "namespaces"
        if not namespaces_root.is_dir():
            print(f"search: no namespaces under {root}")
            return
        targets = sorted(d.name for d in namespaces_root.iterdir() if d.is_dir())
        if explicit_ns and not args.all:
            targets = [explicit_ns]
    else:
        targets = [explicit_ns]
    flags = re.IGNORECASE if args.ignore_case else 0
    try:
        pattern = re.compile(args.pattern, flags)
    except re.error as error:
        raise SystemExit(f"search: invalid regex: {error}")
    hits = 0
    for ns in targets:
        ns_dir = root / "namespaces" / ns
        if not ns_dir.is_dir():
            print(f"search: namespace not found: {ns}", file=sys.stderr)
            continue
        for md in sorted(ns_dir.rglob("*.md")):
            rel = md.relative_to(ns_dir)
            marker = " [index]" if md.name == "MEMORY.md" else ""
            try:
                text_lines = md.read_text(encoding="utf-8", errors="replace").splitlines()
            except OSError:
                continue
            for lineno, line in enumerate(text_lines, 1):
                if pattern.search(line):
                    # [slice 4] a dismissed line is not a search hit either —
                    # no read path revives owner-dismissed content
                    if owner_filter_line(root, line):
                        continue
                    print(f"{ns}/{rel}:{lineno}{marker}: {line.strip()[:200]}")
                    hits += 1
                    if hits >= args.max_lines:
                        print(f"search: capped at {args.max_lines} matching lines (raise --max-lines)")
                        return
    if hits == 0:
        print(f"search: no matches for /{args.pattern}/")


def cmd_pin_runners(args):
    """[11.236] Pin the owner-managed ygg-memory runners to a repo SSOT.

    The verb dispersal plane deliberately never touches the ygg-memory
    runner pair (owner_managed) — the tool self-disperses to PEERS via
    sync-fleet, which leaves the LOCAL installed copies stale and once
    reinstalled OLD bytes mid-churn (measured 2026-10-06). After any
    merge touching the runner pair, run this from (or pointing at) an
    up-to-date checkout on EVERY host and read the report back.
    """
    repo = Path(args.repo).expanduser()
    source = repo / ".agents" / "skills" / "yggterm-agent-fleet" / "ygg-memory.py"
    if not source.is_file():
        print(f"pin-runners: no runner at {source} — pass --repo <yggterm checkout>")
        return
    source_bytes = source.read_bytes()
    targets = [
        Path.home() / ".local" / "bin" / "ygg-memory.py",
        Path.home() / ".yggterm" / "bin" / "ygg-memory.py",
    ]
    pinned = 0
    for target in targets:
        if target.parent.is_dir() or target.parent.name == "bin":
            target.parent.mkdir(parents=True, exist_ok=True)
            if not target.is_file() or target.read_bytes() != source_bytes:
                target.write_bytes(source_bytes)
                pinned += 1
                print(f"  pinned: {target}")
            else:
                print(f"  current: {target}")
    if getattr(args, "json", False):
        print(json.dumps({"pinned": pinned}))
    print(f"pin-runners: {pinned} copies updated; verify with a read-back grep")


def _sync_adapter_once(root: Path, harness: str) -> tuple[int, int, int, int]:
    adapter = get_harness_adapter(harness, cwd=Path.cwd())
    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        migrate_legacy_store(root)
        result = adapter.sync_all(root, harness)
        watermark = load_watermark(root, harness)
        mark_events_seen(root, watermark, None, harness)
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)
        return result
    finally:
        _flock_close(lock)


def cmd_startup(args):
    """Bounded pre-launch convergence. Failure is visible but never eats the PTY."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    errors = []
    totals = [0, 0, 0, 0]
    for phase in ("local-before", "fleet", "local-after"):
        try:
            if phase == "fleet":
                try:
                    mesh = resolve_fleet_mesh(getattr(args, "mesh", None))
                except SystemExit:
                    # A one-machine installation is complete. sync-fleet still
                    # fails loudly when invoked explicitly without a roster.
                    continue
                _run_fleet_sync(root, mesh, quick=True)
            else:
                result = _sync_adapter_once(root, harness)
                totals = [left + right for left, right in zip(totals, result)]
        except Exception as error:
            errors.append(f"{phase}: {error}")
    if errors:
        print("ygg-memory startup warning: " + " | ".join(errors), file=sys.stderr)
    if getattr(args, "json", False):
        print(json.dumps({"harness": harness, "namespaces": totals[0], "totals": totals[1:], "warnings": errors}))


def main():
    # The same options are accepted before or after the subcommand. Subparser
    # defaults must be suppressed or argparse will overwrite a value parsed
    # before the subcommand with None (observed with --harness codex sync ...).
    common_parser = argparse.ArgumentParser(add_help=False, argument_default=argparse.SUPPRESS)
    common_parser.add_argument("--root", help="Root path for ~/.yggterm/memory")
    common_parser.add_argument("--harness", help="Agent CLI harness name")
    common_parser.add_argument("--ns", help="Project namespace (e.g. -home-pi-gh-yggterm)")
    common_parser.add_argument("--json", action="store_true", help="Format output as JSON for tool calls")

    parser = argparse.ArgumentParser(description="Unified Cross-Harness Fleet Memory Tool")
    parser.add_argument("--root", default=str(DEFAULT_MEMORY_ROOT), help="Root path for ~/.yggterm/memory")
    parser.add_argument("--harness", default=None, help="Agent CLI harness name")
    parser.add_argument("--ns", default=None, help="Project namespace (e.g. -home-pi-gh-yggterm)")
    parser.add_argument("--json", action="store_true", help="Format output as JSON for tool calls")
    subparsers = parser.add_subparsers(dest="subcommand", required=True)

    # status
    p_status = subparsers.add_parser("status", parents=[common_parser], help="Check if harness memory is behind")

    # diff
    p_diff = subparsers.add_parser("diff", parents=[common_parser], help="View delta doors since last sync")
    p_diff.add_argument("--filter", default=None, help="Filter diffs by kind, topic, or keyword")

    # get
    p_get = subparsers.add_parser("get", parents=[common_parser], help="Retrieve body of a specific memory door")
    p_get.add_argument("--file", required=True, help="Memory door path relative to the namespace (e.g. finding-pty-grid-ssot.md or campaign-cli-integration/codex.md)")
    p_get.add_argument("--lines", type=int, default=None, help="Show only the first N lines (a loud slice marker goes to stderr)")
    p_get.add_argument("--grep", default=None, help="Show only lines matching this regex (a loud slice marker goes to stderr)")

    # ack
    p_ack = subparsers.add_parser("ack", parents=[common_parser], help="Advance harness watermark")
    p_ack.add_argument("--all", action="store_true", help="Acknowledge all doors up to latest sequence")
    p_ack.add_argument("--files", default=None, help="Comma-separated filenames to selectively acknowledge")

    # publish
    p_pub = subparsers.add_parser("publish", parents=[common_parser], help="Publish a local file into unified memory")
    p_pub.add_argument("--file", required=True, help="Source markdown file to publish")
    p_pub.add_argument("--dest", default=None, help="Door path relative to the namespace; may include subdirectories (e.g. campaign-cli-integration/codex.md). Defaults to the source basename.")
    p_pub.add_argument("--as", dest="as_name", default=None, help="Door basename shorthand (flat name only; mutually exclusive with --dest). Renames the published door instead of minting a scratch-name duplicate.")
    p_pub.add_argument("--kind", default=None, help="Kind (finding, campaign, spec, feedback, steer)")
    p_pub.add_argument("--summary", default=None, help="One-line summary description")
    p_pub.add_argument("--target-harness", "--scope", dest="target_harness", default=None, help="Target harness scope ('all' or specific: gemini, claude, grok, codex)")

    # delete
    p_del = subparsers.add_parser("delete", parents=[common_parser], help="Retire a memory door (journaled delete; recoverable by re-publishing)")
    p_del.add_argument("--file", required=True, help="Door path relative to the namespace (may include subdirectories)")

    # resolve
    p_resolve = subparsers.add_parser("resolve", parents=[common_parser], help="Resolve all divergent heads of one door")
    p_resolve.add_argument("--file", required=True, help="Conflicted door filename")
    p_resolve.add_argument("--using", required=True, help="Reviewed Markdown file containing the merged result")

    # strict area gate
    p_sync = subparsers.add_parser("sync", parents=[common_parser], help="Strictly sync one memory area before recall")
    p_sync.add_argument("--local-dir", default=None, help=argparse.SUPPRESS)
    p_sync.add_argument("--fleet", action="store_true", help="Sync the fleet mesh before the local area")
    p_sync.add_argument("--mesh", default=None, help="Optional fleet roster override used with --fleet")
    p_sync.add_argument("--quick", action="store_true", help="Use quick fleet probes with --fleet")

    # sync-harness
    p_sync_h = subparsers.add_parser("sync-harness", parents=[common_parser], help="Bi-directional sync with local harness store")
    p_sync_h.add_argument("--local-dir", default=None, help="Explicit local harness memory directory")
    p_sync_h.add_argument("--all", action="store_true", help="Sync all namespaces (unified + all harness local dirs)")

    # sync-fleet
    p_sync_f = subparsers.add_parser("sync-fleet", parents=[common_parser], help="Mesh sync ~/.yggterm/memory across SSH peers")
    p_sync_f.add_argument("--mesh", default=None,
                          help="Space-separated peer SSH hosts; default reads $YGG_FLEET_MESH "
                               "then ~/.config/ygg-fleet/mesh")
    p_sync_f.add_argument("--quick", action="store_true", help="Skip deployment and use one-second SSH probes")
    p_sync_f.add_argument("--quiet", action="store_true", help="Suppress success output")

    # import-journal: private fleet transport endpoint, stdin is JSONL.
    subparsers.add_parser("import-journal", parents=[common_parser], help=argparse.SUPPRESS)

    # migrate: private fleet upgrade handshake.
    subparsers.add_parser("migrate", parents=[common_parser], help=argparse.SUPPRESS)

    # startup: yggterm-managed, bounded pre-launch convergence.
    p_startup = subparsers.add_parser("startup", parents=[common_parser], help="Converge fleet + native memory before CLI launch")
    p_startup.add_argument("--mesh", default=None, help="Optional fleet roster override")
    p_shelve = subparsers.add_parser(
        "shelve-archives",
        parents=[common_parser],
        help="Tier the native store: move legacy top-level sitting archives into archive/ and rebuild the tiered indexes (dry-run by default)",
    )
    p_shelve.add_argument("--apply", action="store_true", help="perform the moves (default: dry-run report)")
    p_pin = subparsers.add_parser(
        "pin-runners",
        parents=[common_parser],
        help="Pin the owner-managed ygg-memory runner to a repo SSOT (run on every host after a merge touching it; [11.236])",
    )
    p_pin.add_argument("--repo", default=str(Path.home() / "gh" / "yggterm"), help="yggterm checkout to pin from")

    # dream [mem-dream slice 2]
    p_dream = subparsers.add_parser(
        "dream",
        parents=[common_parser],
        help="Consolidate a namespace: prepare a raw-delta brief, compose (sol or seat), apply the validated rollup",
    )
    dream_actions = p_dream.add_mutually_exclusive_group(required=True)
    dream_actions.add_argument("--prepare", action="store_true", help="stage pending/BRIEF.md + MANIFEST.json from journal deltas (clean tick if nothing new)")
    dream_actions.add_argument("--apply", action="store_true", help="validate + land pending/RESPONSE.md (+CRITIC.md) through the journaled publish path")
    dream_actions.add_argument("--run", action="store_true", help="prepare + compose via codex (sol) + critic + apply in one shot")
    dream_actions.add_argument("--status", action="store_true", help="dream watermark, pending state, recent dreams")
    dream_actions.add_argument("--auto", action="store_true", help="the auto-dreamer: pick the hungriest eligible namespace (delta-sorted, cooldown + quiescence filtered) and run prepare->compose->critic->apply; never raises (tick-safe)")
    p_dream.add_argument("--phase", default="synthesis", help="phase schema for the composer: recon|synthesis (declared; H_vocab auto-proposal is slice 3)")
    p_dream.add_argument("--extra-input", action="append", default=[], metavar="PATH", help="stage an extra raw input (board dump, ci.log tail); repeatable")
    p_dream.add_argument("--budget", type=int, default=None, help=f"delta input budget in bytes (default {DREAM_INPUT_BUDGET_BYTES})")
    p_dream.add_argument("--dry-run", action="store_true", help="with --apply: validate and print the plan without writing")
    p_dream.add_argument("--composer", default="sol", help="composer transport for --run (sol; the seat-composer path is prepare/apply by hand)")
    p_dream.add_argument("--model", default="gpt-6.1-sol", help="composer model for --run")
    p_dream.add_argument("--effort", default="high", help="composer reasoning effort for --run")
    p_dream.add_argument("--codex-bin", default=None, help="explicit codex binary path for --run")
    p_dream.add_argument(
        "--force",
        action="store_true",
        help="override the quiescence gate (owner-directed testing only)",
    )
    p_dream.add_argument("--quiet", action="store_true", help="suppress informational output")
    p_canary = subparsers.add_parser(
        "canary",
        parents=[common_parser],
        help="Deterministic memory health probes: index line budget, archive pointer, NOW.md size, watermark monotonicity (zero LLM)",
    )

    # search [mem-dream slice 2]
    p_search = subparsers.add_parser(
        "search",
        parents=[common_parser],
        help="Grep hub doors with provenance (JIT retrieval when the door name is unknown)",
    )
    p_search.add_argument("pattern", help="regex to match")
    p_search.add_argument("--all", action="store_true", help="search every namespace (default when --ns is omitted)")

    # [slice 2] the read-time temporal envelope verb
    p_now = subparsers.add_parser("now", parents=[common_parser], help="Read NOW.md + the temporal envelope (claim ages, expiries, supersessions) — the session-start orientation read")
    p_now.set_defaults(fn=cmd_now)

    # [slice 3] authoritative owner corrections
    p_owner = subparsers.add_parser("owner", parents=[common_parser], help="Authoritative owner corrections: --correct/--dismiss/--surface (effective on next read); --list shows the record")
    p_owner.add_argument("--correct", action="store_true")
    p_owner.add_argument("--dismiss", action="store_true")
    p_owner.add_argument("--surface", action="store_true")
    p_owner.add_argument("--target", default="", help="the claim id / text fragment the correction targets")
    p_owner.add_argument("--text", default="", help="the corrected text or surfacing instruction")
    p_owner.add_argument("--scope", default="_global")
    p_owner.add_argument("--list", action="store_true")
    p_owner.add_argument("--affect", metavar="TARGET", default=None, help="[slice 4] review surface: which claims (with evidence) and door lines this target currently touches")
    p_owner.set_defaults(fn=cmd_owner)

    # [slice 1] fleet budget admission
    p_budget = subparsers.add_parser(
        "dream-budget",
        parents=[common_parser],
        help="[slice 1] Atomically reserve/settle dream token+cash against the ONE serialized fleet authority (refusal = deferred, exit 3)",
    )
    budget_actions = p_budget.add_mutually_exclusive_group(required=True)
    budget_actions.add_argument("--admit", action="store_true", help="reserve an estimate (refused on per-item/registry gates always; on capacity only in hard-caps mode)")
    budget_actions.add_argument("--caps", choices=["hard", "soft"], help="[owner ruling 2026-10-16] instantiate (hard) or lift (soft) a cap window — soft is the standing state: quality first, no waste, provider quotas may empty; dreams stall on quota hits, not on our arithmetic")
    budget_actions.add_argument("--settle", metavar="ID", help="book actual spend for a reservation (charged to its reservation day)")
    budget_actions.add_argument("--release", metavar="ID", help="release a hold unspent (abandoned work)")
    budget_actions.add_argument("--status", action="store_true", dest="budget_status", help="ledger view: caps, spend, holds, headroom, receipts")
    p_budget.add_argument("--bucket", default=BUDGET_DREAM_BUCKET, help=f"policy bucket (default {BUDGET_DREAM_BUCKET})")
    p_budget.add_argument("--input", type=int, default=0, help="input-token dimension (estimate or actual)")
    p_budget.add_argument("--generated", type=int, default=0, help="generated-token dimension")
    p_budget.add_argument("--cash-cents", type=int, default=0, dest="cash_cents", help="cash dimension in US cents")
    p_budget.add_argument("--ttl", type=int, default=BUDGET_RESERVATION_TTL_S, help=f"admission hold TTL seconds (default {BUDGET_RESERVATION_TTL_S}; crash safety)")
    p_budget.add_argument("--for-days", type=int, default=None, help="with --caps hard: the window length (auto-restores soft after it)")
    p_budget.add_argument("--purpose", default="", help="what this reservation is for / the reason for a caps change (recorded in receipts)")
    p_budget.add_argument("--actual-source", default="measured", help="how settle actuals were obtained (measured|byte-estimate)")
    p_budget.add_argument("--class", dest="work_class", default=None, help="[slice 2] work-class id — enforces the owner registry (unknown/disabled class refuses + forwards; over-ceiling estimate refuses + forwards)")
    p_budget.set_defaults(fn=cmd_budget)

    # [slice 3] bounded claim retrieval
    p_claims = subparsers.add_parser(
        "claims",
        parents=[common_parser],
        help="[slice 3] Query the persistent claim store (evidence-linked; survives any amount of brief compaction)",
    )
    p_claims.add_argument("--query", default="", help="case-insensitive substring over claim text")
    p_claims.add_argument("--id", default=None, help="exact claim id")
    p_claims.add_argument("--status", default="all", choices=["all", "ok", "due", "expired", "superseded"], help="temporal filter")
    p_claims.add_argument("--dismissed", action="store_true", help="[slice 4] show ONLY owner-dismissed claims (the review view; the default read never revives them)")
    p_claims.add_argument("--limit", type=int, default=20)
    p_claims.set_defaults(fn=cmd_claims)

    # [slice 5] the connection plane (Yin/Yang over native addresses)
    p_conn = subparsers.add_parser(
        "connections",
        parents=[common_parser],
        help="[slice 5] Typed edges between resolvable objects: --observe (Yin, deduping), --propose (Yang hypothesis + mandatory falsification test), --test, --index [--explore], --scan-hub/--scan-board/--scan-lobes (observation-only adapters)",
    )
    conn_actions = p_conn.add_mutually_exclusive_group(required=True)
    conn_actions.add_argument("--index", action="store_true", help="the derived edge view (ordinary retrieval: hypotheses and expired edges excluded)")
    conn_actions.add_argument("--explore", action="store_true", help="the exploratory view: hypothesis edges INCLUDED, hypothetical status preserved")
    conn_actions.add_argument("--observe", action="store_true", help="Yin: record/strengthen one observed edge (identical triples dedupe, never duplicate)")
    conn_actions.add_argument("--propose", action="store_true", help="Yang: propose a hypothesis edge — needs --hypothesis AND --falsification-test ('possibly related' fails admission)")
    conn_actions.add_argument("--test", metavar="EDGE_ID", help="run a hypothesis's falsification test: --outcome supported|rejected")
    conn_actions.add_argument("--scan-hub", metavar="NS", help="Yin hub adapter: resolve [[links]] within a namespace (ambiguous stays unresolved)")
    conn_actions.add_argument("--scan-board", metavar="BOARD", help="Yin board adapter: ACK refs → replies-to edges (replies prove reply, not agreement)")
    conn_actions.add_argument("--scan-lobes", metavar="DIR", help="Yin lobe adapter: lobe:// references inside lobe records → edges")
    p_conn.add_argument("--from", dest="from_addr", default="", help="from address (ns/door, board:<b>/<ACK>, lobe://…, or any stable id)")
    p_conn.add_argument("--relation", default="references", help="edge relation (links|references|replies-to|supports|corrects|fulfills|…)")
    p_conn.add_argument("--to", dest="to_addr", default="", help="to address")
    p_conn.add_argument("--status", default="observed", choices=CONNECTION_STATUSES, help="epistemic status for --observe")
    p_conn.add_argument("--source", dest="source_ref", default=None, help="provenance reference (evidence the edge came from)")
    p_conn.add_argument("--created-by", dest="created_by", default="seat")
    p_conn.add_argument("--hypothesis", default=None, help="the concrete hypothesis (mandatory for --propose)")
    p_conn.add_argument("--falsification-test", dest="falsification_test", default=None, help="the observable falsification test (mandatory for --propose)")
    p_conn.add_argument("--review-days", type=int, default=None, help="hypothesis review window (default 7d; untested past it = expired from ordinary views)")
    p_conn.add_argument("--outcome", default="supported", choices=["supported", "rejected"], help="test outcome for --test")
    p_conn.set_defaults(fn=cmd_connections)

    # [slice 2] owner-declared work-class registry
    p_registry = subparsers.add_parser(
        "work-registry",
        parents=[common_parser],
        help="[slice 2] The owner work-class registry: --list, or tighten/enforce (--enable/--disable/--set-per-item; raising a seed ceiling is refused by design)",
    )
    registry_actions = p_registry.add_mutually_exclusive_group()
    registry_actions.add_argument("--list", action="store_true", dest="list_registry", help="show the effective registry (seeds + tightenings)")
    registry_actions.add_argument("--enable", action="store_true", help="enable a class")
    registry_actions.add_argument("--disable", action="store_true", help="disable a class (admissions refuse + forward; in-flight settles REFUSE — authority revoked pre-commit)")
    p_registry.add_argument("--class-id", default=None, help="the work class to mutate")
    p_registry.add_argument("--input", type=int, default=None, help="per-item input ceiling (lowering only)")
    p_registry.add_argument("--generated", type=int, default=None, help="per-item generated ceiling (lowering only)")
    p_registry.add_argument("--cash-cents", type=int, default=None, dest="cash_cents", help="per-item cash ceiling in cents (lowering only)")
    p_registry.set_defaults(fn=cmd_work_registry)

    # hidden plumbing: the authority host executes one op from stdin
    p_budget_op = subparsers.add_parser(
        "_budget-op", parents=[common_parser], help=argparse.SUPPRESS,
    )
    p_budget_op.set_defaults(fn=cmd_budget_op)

    p_search.add_argument("-i", "--ignore-case", action="store_true")
    p_search.add_argument("--max-lines", type=int, default=50)

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
    elif args.subcommand == "delete":
        cmd_delete(args)
    elif args.subcommand == "resolve":
        cmd_resolve(args)
    elif args.subcommand == "sync":
        cmd_sync(args)
    elif args.subcommand == "sync-harness":
        cmd_sync_harness(args)
    elif args.subcommand == "sync-fleet":
        cmd_sync_fleet(args)
    elif args.subcommand == "import-journal":
        cmd_import_journal(args)
    elif args.subcommand == "migrate":
        cmd_migrate(args)
    elif args.subcommand == "startup":
        cmd_startup(args)
    elif args.subcommand == "shelve-archives":
        cmd_shelve_archives(args)
    elif args.subcommand == "pin-runners":
        cmd_pin_runners(args)
    elif args.subcommand == "canary":
        cmd_canary(args)
    elif args.subcommand == "dream":
        cmd_dream(args)
    elif args.subcommand == "search":
        cmd_search(args)
    elif args.subcommand == "now":
        cmd_now(args)
    elif args.subcommand == "owner":
        cmd_owner(args)
    elif args.subcommand == "dream-budget":
        cmd_budget(args)
    elif args.subcommand == "work-registry":
        cmd_work_registry(args)
    elif args.subcommand == "claims":
        cmd_claims(args)
    elif args.subcommand == "connections":
        cmd_connections(args)
    elif args.subcommand == "_budget-op":
        cmd_budget_op(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    try:
        main()
    except ValueError as error:
        print(f"ygg-memory: {error}", file=sys.stderr)
        sys.exit(2)
