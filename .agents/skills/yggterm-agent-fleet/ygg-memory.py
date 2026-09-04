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
from pathlib import Path

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

> 🌐 **UNIFIED FLEET MEMORY**: Before deep memory recall or after campaign handovers, consult `ygg-memory status --harness <me>` or `ygg-memory diff` to catch updates from Claude, Grok, Codex, Gemini, Zcode, or Muse. Ingest full or partial diffs as needed.
> ⛔ **Doors, not rooms.** Rules (`feedback-/spec-/reference-/user-`) · ledgers (`campaign-/project-`) · findings (`finding-/bug-class-`) · steers (`steer-<harness>-`).
> One line, one door. Detail belongs in the target file, never here.
"""

GLOBAL_NAMESPACE = "_global"
MANAGED_BLOCK_BEGIN = "<!-- BEGIN yggterm-memory -->"
MANAGED_BLOCK_END = "<!-- END yggterm-memory -->"


def memory_bridge(harness: str) -> str:
    """Small always-loaded door; the hub remains the room and source of truth."""
    canonical = normalize_harness_name(harness)
    return (
        f"{MANAGED_BLOCK_BEGIN}\n"
        "## Yggterm fleet memory\n\n"
        "Yggterm synchronizes semantic memory before managed CLI startup and by a catch-up timer. "
        "Before deep recall, after a handover, or whenever another machine or CLI may have learned "
        f"something, run `ygg-memory status --harness {canonical}` and `ygg-memory diff --harness {canonical}`, then open only the relevant door "
        "with `ygg-memory get --file <name>`. Publish durable findings through `ygg-memory publish`; "
        "never copy credentials, sessions, databases, indexes, or lock files between machines.\n\n"
        f"This harness name is `{canonical}`. The current project namespace is derived from the cwd.\n"
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

    watermark = load_watermark(root, harness)
    state_key = f"{namespace}/{filename}"
    state = watermark.setdefault("native_documents", {}).setdefault(state_key, {})
    base_digest = state.get("delivered_payload_digest", state.get("delivered_digest"))
    base_version = state.get("delivered_version")
    ingested = delivered = deleted = 0

    def publish_local(causal_base: str | None) -> None:
        nonlocal ingested, deleted
        if semantic_local_exists:
            hub.write_text(
                native_document_content(
                    harness,
                    label,
                    local_payload,
                    target_harness=target_harness,
                    native_path=native_path,
                ),
                encoding="utf-8",
            )
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
    target = root.parent / "memory-backups" / harness / namespace / path.name / f"{digest}.md"
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
        lines = ["# Memory index", ""]
        for doc in sorted(memory_dir.rglob("*.md")):
            if doc.name == "MEMORY.md":
                continue
            content = doc.read_text(encoding="utf-8", errors="replace")
            if not content.startswith("---\n"):
                continue
            title = self._meta(content, "name") or doc.stem
            description = self._meta(content, "description") or title
            rel = urllib.parse.quote(doc.relative_to(memory_dir).as_posix(), safe="/._~-")
            line = f"- [{title}]({rel}) — {description}".replace("\n", " ")
            lines.append(line[:199] + "…" if len(line) > 200 else line)
        rendered = "\n".join(lines) + "\n"
        index = memory_dir / "MEMORY.md"
        if not index.is_file() or index.read_text(encoding="utf-8") != rendered:
            index.write_text(rendered, encoding="utf-8")

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
            exclude=lambda relative: relative.name == "MEMORY.md"
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
            raise RuntimeError(f"Lock held by another process: {lock_path}")
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


def materialize_store(root: Path) -> dict:
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
        heads = causal_heads_for(root, namespace, filename, records)
        if not heads:
            continue

        live = get_namespace_dir(root, namespace) / filename
        conflict_dir = root / "conflicts" / namespace / filename
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
    ns_dir = get_namespace_dir(root, ns)
    target = ns_dir / args.file
    if not target.exists():
        print(f"Error: Door '{args.file}' not found in namespace '{ns}'.", file=sys.stderr)
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
    if args.lines is not None:
        shown = shown[: max(args.lines, 0)]
    if len(shown) != total:
        cut = f" (match /{args.grep}/)" if args.grep else ""
        print(
            f"[ygg-memory] showing {len(shown)} of {total} lines of {args.file}{cut}"
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
            for fpath in ns_dir.glob("*.md"):
                content = fpath.read_text(encoding="utf-8")
                _, _, target_h = extract_metadata_and_summary(content, fpath.name)
                if matches_target_harness(target_h, harness):
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
        dest_filename = source_path.name
        dest_path = ns_dir / dest_filename

        content = source_path.read_text(encoding="utf-8")
        kind, summary, extracted_target = extract_metadata_and_summary(content, dest_filename)

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
            target_info = f" [target: {target_harness}]" if target_harness != "all" else ""
            print(f"Published '{dest_filename}' to namespace '{ns}' (seq #{record['seq']}){target_info}.")
    finally:
        _flock_close(lock)


def cmd_resolve(args):
    """Resolve every current causal head with one reviewed Markdown result."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    namespace = detect_namespace(override=args.ns)
    source = Path(args.using).resolve()
    if not source.is_file():
        raise SystemExit(f"ygg-memory: resolution source does not exist: {source}")
    lock = _flock_open(root / ".ygg-memory.lock")
    try:
        heads = causal_heads_for(root, namespace, args.file)
        raw_heads = causal_heads_for(root, namespace, args.file, coalesce=False)
        if len(heads) < 2:
            raise SystemExit(f"ygg-memory: {namespace}/{args.file} has fewer than two divergent heads")
        destination = get_namespace_dir(root, namespace) / args.file
        if source != destination.resolve():
            shutil.copyfile(source, destination)
        content = destination.read_text(encoding="utf-8")
        kind, summary, target = extract_metadata_and_summary(content, args.file)
        record = append_journal_entry(
            root,
            namespace,
            args.file,
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
        remaining = causal_heads_for(root, namespace, args.file)
        if len(remaining) != 1:
            raise RuntimeError(f"resolution did not converge {args.file}: {len(remaining)} heads remain")
        if args.json:
            print(json.dumps({"status": "ok", "record": record, "report": report}))
        else:
            print(f"Resolved {namespace}/{args.file}; superseded {len(heads)} divergent heads.")
    finally:
        _flock_close(lock)


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
    names = {
        path.name for path in local_dir.glob("*.md")
    } | {
        path.name for path in ns_dir.glob("*.md")
    } | set(sync_state)

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
                kind, summary, native_target = extract_metadata_and_summary(content, fname)
                shutil.copyfile(local, hub)
                action = "upsert"
                in_count += 1
            else:
                previous = hub.read_text(encoding="utf-8") if hub_exists else ""
                kind, summary, native_target = extract_metadata_and_summary(previous, fname)
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


def cmd_sync_harness(args):
    """Bidirectional sync between harness-local directory and ~/.yggterm/memory."""
    root = Path(args.root)
    harness = detect_harness(args.harness)
    adapter = get_harness_adapter(harness)
    lock = _flock_open(root / ".ygg-memory.lock")

    try:
        migrate_legacy_store(root)
        if getattr(args, "all", False):
            namespace_count, total_in, total_out, total_del = adapter.sync_all(root, harness)
            watermark = load_watermark(root, harness)
            mark_events_seen(root, watermark, None, harness)
            watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            save_watermark(root, watermark)
            if args.json:
                print(json.dumps({"status": "ok", "harness": harness, "namespaces": namespace_count, "pulled_in": total_in, "pushed_out": total_out, "deleted": total_del}))
            else:
                print(f"Harness sync completed ({harness} all {namespace_count} ns): {total_in} ingested, {total_out} propagated, {total_del} deleted.")
            return

        ns = detect_namespace(override=args.ns)
        # ``--local-dir`` is an explicit test/operator override for project-style
        # adapters.  Codex deliberately has no such escape hatch: its inbox is
        # the only sanctioned native write surface.
        if getattr(args, "local_dir", None):
            if not isinstance(adapter, ProjectMemoryAdapter):
                raise ValueError(f"{harness} does not support --local-dir; use its native adapter")
            inc, outc, delc = _sync_project_memory_namespace(root, harness, ns, Path(args.local_dir))
        else:
            inc, outc, delc = adapter.sync_namespace(root, harness, ns)
        watermark = load_watermark(root, harness)
        mark_events_seen(root, watermark, ns, harness)
        watermark["last_sync_ts"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save_watermark(root, watermark)
        if args.json:
            print(json.dumps({"status": "ok", "harness": harness, "namespace": ns, "pulled_in": inc, "pushed_out": outc, "deleted": delc}))
        else:
            extra = f", {delc} deleted" if delc else ""
            print(f"Harness sync completed ({harness} <-> {ns}): {inc} ingested, {outc} propagated{extra}.")
    finally:
        _flock_close(lock)


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

    local_lock = _flock_open(root / ".ygg-memory.lock", timeout_seconds=60)
    try:
        migrate_legacy_store(root)
    finally:
        _flock_close(local_lock)
    (root / "objects").mkdir(parents=True, exist_ok=True)
    journal_file = get_journal_path(root)
    script_dir = Path(__file__).resolve().parent
    failures = []

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
            for destination in (".local/bin", ".yggterm/bin"):
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
            failures.append(f"{peer}:import-journal")
    if materialized.get("conflicts"):
        failures.append(f"{materialized['conflicts']}-preserved-conflict(s)")
    if materialized.get("missing_objects"):
        failures.append(f"{materialized['missing_objects']}-missing-object(s)")
    if unreachable or failures:
        details = []
        if unreachable:
            details.append("unreachable=" + ",".join(unreachable))
        if failures:
            details.append("failed=" + ",".join(failures))
        raise RuntimeError("fleet memory did not converge: " + "; ".join(details))
    return {
        "peers": live_peers,
        "pulled_objects": pulled_objects,
        "pushed_objects": pushed_objects,
        **materialized,
    }


def cmd_sync_fleet(args):
    """Mesh synchronize semantic objects/events across reachable SSH peers."""
    root = Path(args.root)
    mesh = resolve_fleet_mesh(args.mesh)
    report = _run_fleet_sync(root, mesh, quick=getattr(args, "quick", False))
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
    common_parser = argparse.ArgumentParser(add_help=False)
    common_parser.add_argument("--root", default=str(DEFAULT_MEMORY_ROOT), help="Root path for ~/.yggterm/memory")
    common_parser.add_argument("--harness", default=None, help="Agent CLI harness name")
    common_parser.add_argument("--ns", default=None, help="Project namespace (e.g. -home-pi-gh-yggterm)")
    common_parser.add_argument("--json", action="store_true", help="Format output as JSON for tool calls")

    parser = argparse.ArgumentParser(description="Unified Cross-Harness Fleet Memory Tool", parents=[common_parser])
    subparsers = parser.add_subparsers(dest="subcommand", required=True)

    # status
    p_status = subparsers.add_parser("status", parents=[common_parser], help="Check if harness memory is behind")

    # diff
    p_diff = subparsers.add_parser("diff", parents=[common_parser], help="View delta doors since last sync")
    p_diff.add_argument("--filter", default=None, help="Filter diffs by kind, topic, or keyword")

    # get
    p_get = subparsers.add_parser("get", parents=[common_parser], help="Retrieve body of a specific memory door")
    p_get.add_argument("--file", required=True, help="Memory filename (e.g. finding-pty-grid-ssot.md)")
    p_get.add_argument("--lines", type=int, default=None, help="Show only the first N lines (a loud slice marker goes to stderr)")
    p_get.add_argument("--grep", default=None, help="Show only lines matching this regex (a loud slice marker goes to stderr)")

    # ack
    p_ack = subparsers.add_parser("ack", parents=[common_parser], help="Advance harness watermark")
    p_ack.add_argument("--all", action="store_true", help="Acknowledge all doors up to latest sequence")
    p_ack.add_argument("--files", default=None, help="Comma-separated filenames to selectively acknowledge")

    # publish
    p_pub = subparsers.add_parser("publish", parents=[common_parser], help="Publish a local file into unified memory")
    p_pub.add_argument("--file", required=True, help="Source markdown file to publish")
    p_pub.add_argument("--kind", default=None, help="Kind (finding, campaign, spec, feedback, steer)")
    p_pub.add_argument("--summary", default=None, help="One-line summary description")
    p_pub.add_argument("--target-harness", "--scope", dest="target_harness", default=None, help="Target harness scope ('all' or specific: gemini, claude, grok, codex)")

    # resolve
    p_resolve = subparsers.add_parser("resolve", parents=[common_parser], help="Resolve all divergent heads of one door")
    p_resolve.add_argument("--file", required=True, help="Conflicted door filename")
    p_resolve.add_argument("--using", required=True, help="Reviewed Markdown file containing the merged result")

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
    elif args.subcommand == "resolve":
        cmd_resolve(args)
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
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
