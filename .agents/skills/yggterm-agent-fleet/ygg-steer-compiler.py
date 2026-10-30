"""Read-only compiler for explicitly enrolled private steering source parts.

This compiles proposed projections. It neither activates policy nor stamps a
target, imports a repository, or authorizes retirement of an old source.
"""
from __future__ import annotations
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat

spec = importlib.util.spec_from_file_location("ygg_compiler_fs", Path(__file__).with_name("ygg-fs.py"))
fs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fs)


def sha(value):
    return hashlib.sha256(value).hexdigest()


def canonical(doc):
    return json.dumps(doc, sort_keys=True, separators=(",", ":")).encode()


def _relative(value):
    if not isinstance(value, str) or not value:
        raise fs.Held("missing private-source relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or "\\" in value or path.as_posix() != value or value == ".":
        raise fs.Held("noncanonical private-source relative path")
    return path


def _tag(value):
    if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.:/-]{0,127}", value):
        raise fs.Held("invalid managed-region identity")
    return value


def _owner_marker(text, tag):
    return re.search(r"<!--\s*(?:BEGIN|END)\s+" + re.escape(tag) + r"(?=\s|-->|$)", text, re.IGNORECASE)


def read_source(root, relative, expected_digest):
    if not isinstance(expected_digest, str) or not re.fullmatch(r"[a-f0-9]{64}", expected_digest):
        raise fs.Held("private source needs a sealed content digest")
    path = Path(root).absolute() / _relative(relative)
    with fs._directory(path.parent) as directory:
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(fd, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise fs.Held("private source is not an owned regular file")
            if info.st_size > 4 * 1024 * 1024:
                raise fs.Held("private source part exceeds the bounded compiler input")
            raw = stream.read(4 * 1024 * 1024 + 1)
    if len(raw) > 4 * 1024 * 1024 or sha(raw) != expected_digest:
        raise fs.Held("private source content differs from its proposed revision")
    return raw.decode("utf-8")


def foreign_region(text, tag):
    tag = _tag(tag)
    begin, end = f"<!-- BEGIN {tag} -->", f"<!-- END {tag} -->"
    # Preserve exact complete bytes. Similar/incomplete spellings for this
    # owner cannot be treated as an absent guard.
    prefixes = list(re.finditer(r"<!--\s*(?:BEGIN|END)\s+" + re.escape(tag) + r"(?=\s|-->|$)", text, re.IGNORECASE))
    if not prefixes:
        return None
    if text.count(begin) != 1 or text.count(end) != 1 or len(prefixes) != 2:
        raise fs.Held("ambiguous or incomplete foreign-owner region")
    left, right = text.index(begin), text.index(end)
    if right < left:
        raise fs.Held("reversed foreign-owner region")
    region = text[left:right + len(end)]
    if re.search(r"<!--\s*(?:BEGIN|END)\b", text[left + len(begin):right], re.IGNORECASE):
        raise fs.Held("nested foreign-owner region")
    after = right + len(end)
    if text[after:after + 2] == "\r\n": region += "\r\n"
    elif text[after:after + 1] == "\n": region += "\n"
    return region


def compile_projection(root, manifest, harness, live=b""):
    if (not isinstance(manifest, dict)
            or set(manifest) - {"schema", "compiler_version", "targets"}
            or type(manifest.get("schema")) is not int or manifest["schema"] != 1
            or type(manifest.get("compiler_version")) is not int or manifest["compiler_version"] != 1):
        raise fs.Held("unsupported private steering manifest")
    if not isinstance(live, bytes) or len(live) > 8 * 1024 * 1024:
        raise fs.Held("unsupported or oversized live target snapshot")
    targets = manifest.get("targets")
    if not isinstance(targets, dict) or harness not in targets or len(targets) > 128:
        raise fs.Held("unknown private harness mapping")
    target_names = set()
    for name, value in targets.items():
        if (not isinstance(name, str) or not isinstance(value, dict)
                or set(value) - {"target", "parts", "preserve"}):
            raise fs.Held("invalid private target table")
        target = str(_relative(value.get("target")))
        if target in target_names:
            raise fs.Held("colliding private harness targets")
        target_names.add(target)
    selected = targets[harness]
    parts = selected.get("parts")
    if not isinstance(parts, list) or not parts or len(parts) > 256:
        raise fs.Held("invalid private source-part list")
    preserved = selected.get("preserve", [])
    if not isinstance(preserved, list) or len(preserved) > 32:
        raise fs.Held("invalid foreign-owner list")
    reserved = {_tag(row.get("tag")) for row in preserved if isinstance(row, dict)}
    if len({tag.casefold() for tag in reserved}) != len(preserved):
        raise fs.Held("duplicate or invalid foreign-owner identity")
    planned = []
    for part in parts:
        if isinstance(part, dict) and part.get("mode") == "guarded":
            planned.append(_tag(part.get("guard")))
    if len({tag.casefold() for tag in planned}) != len(planned):
        raise fs.Held("overlapping managed-region ownership")
    if {tag.casefold() for tag in planned} & {tag.casefold() for tag in reserved}:
        raise fs.Held("overlapping managed-region ownership")
    owned, chunks, references = set(), [], []
    total = 0
    for part in parts:
        if not isinstance(part, dict) or set(part) - {"source", "sha256", "mode", "guard"}:
            raise fs.Held("unknown private source-part fields")
        body = read_source(root, part.get("source"), part.get("sha256"))
        total += len(body.encode())
        if total > 8 * 1024 * 1024:
            raise fs.Held("private projection exceeds the bounded compiler input")
        references.append({"source": part["source"], "sha256": part["sha256"]})
        # Ownership applies to the complete projection, regardless of which
        # part appears first or whether an older literal contains comments.
        for tag in reserved | set(planned):
            if _owner_marker(body, tag):
                raise fs.Held("source contains a separately managed region identity")
        if part.get("mode") == "literal":
            # Historical owner-authored text is opaque and content-bound.
            # It may contain old annotated guards; do not reparse/rewrite it.
            if part.get("guard") is not None:
                raise fs.Held("literal source cannot also own a new wrapper")
            chunks.append(body.rstrip())
        elif part.get("mode") == "guarded":
            tag = _tag(part.get("guard"))
            if tag in owned or tag in reserved:
                raise fs.Held("overlapping managed-region ownership")
            owned.add(tag)
            chunks.append(f"<!-- BEGIN {tag} -->\n{body.rstrip()}\n<!-- END {tag} -->\n")
        else:
            raise fs.Held("unknown private source-part mode")
    # Joining literal fragments introduces whitespace, which is legal inside
    # a marker prefix. Validate the assembled projection as well as bodies.
    source_projection = "\n\n".join(chunks)
    for tag in reserved:
        if _owner_marker(source_projection, tag):
            raise fs.Held("composed source contains a foreign-owner marker")
    for tag in planned:
        prefixes = list(re.finditer(r"<!--\s*(?:BEGIN|END)\s+" + re.escape(tag)
                                   + r"(?=\s|-->|$)", source_projection, re.IGNORECASE))
        begin, end = f"<!-- BEGIN {tag} -->", f"<!-- END {tag} -->"
        if (len(prefixes) != 2 or source_projection.count(begin) != 1
                or source_projection.count(end) != 1
                or source_projection.index(begin) > source_projection.index(end)):
            raise fs.Held("composed source has ambiguous owned-region markers")
    live_text = live.decode("utf-8")
    for row in preserved:
        if set(row) - {"tag", "required"} or type(row.get("required", False)) is not bool:
            raise fs.Held("invalid foreign-owner requirements")
        region = foreign_region(live_text, row["tag"])
        if region is None:
            if row.get("required", False): raise fs.Held("required foreign-owner region is absent")
        else:
            chunks.append(region)
    output = "\n\n".join(chunks).encode()
    return {"schema": 1, "compiler_version": 1, "harness": harness,
            "target": selected["target"], "manifest_digest": sha(canonical(manifest)),
            "input_digest": sha(live), "output_digest": sha(output),
            "source_references": references, "output": output,
            "authority": "proposal-only", "requires_private_projection_review": True,
            "target_written": False}
