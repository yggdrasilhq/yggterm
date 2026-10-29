#!/usr/bin/env python3
"""Portable minimal root steering; private policy never belongs in the seed."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import sys
import uuid

spec = importlib.util.spec_from_file_location("ygg_steer_fs", Path(__file__).with_name("ygg-fs.py"))
fs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fs)
SEED = Path(__file__).with_name("ygg-steer-seed.json")
BEGIN = "<!-- BEGIN ygg-steer:root -->"
END = "<!-- END ygg-steer:root -->"


def digest(value):
    if isinstance(value, str): value = value.encode()
    return hashlib.sha256(value).hexdigest()


def _encoded(doc):
    return json.dumps(doc, sort_keys=True, separators=(",", ":")).encode()


def _organization(home=None, env=None, explicit=None):
    if os.name != "posix":
        raise fs.Held("local steering mutations and target adapters are qualified only on POSIX")
    selected = fs.resolve(home=home, env=env, explicit=explicit)
    if not selected["initialized"]:
        raise fs.Held("initialize the root identity before steering")
    return Path(selected["physical_path"]) / "steer", selected


def initialize(home=None, env=None, explicit=None):
    org, selected = _organization(home, env, explicit)
    seed = fs._read(SEED)
    if seed.get("schema") != 1 or seed.get("version") != 1:
        raise fs.Held("unsupported steering seed")
    key = digest(_encoded(seed))
    with fs._file_lock(org, "publication.lock"):
        fs._atomic_json(org / "seeds" / (key + ".json"), seed, create_once=True)
        if digest(_encoded(fs._read(org / "seeds" / (key + ".json")))) != key:
            raise fs.Held("installed seed bytes differ from their immutable identity")
        fs._atomic_json(org / "overlays/policy.json", {"schema": 1, "suffix": ""}, create_once=True)
        fs._atomic_json(org / "active.json", {"schema": 1, "seed": key}, create_once=True)
        active, _, _ = _load(org)
        return {"status": "initialized", "root_id": selected["root_id"],
                "seed": active["seed"], "legacy_repository_retired": False,
                "fleet_stamped": False}


def _load(org):
    for directory in (org, org / "seeds", org / "overlays"):
        with fs._directory(directory): pass
    active = fs._read(org / "active.json")
    key = active.get("seed", "")
    if active.get("schema") != 1 or not isinstance(key, str) or len(key) != 64 or any(c not in "0123456789abcdef" for c in key):
        raise fs.Held("invalid steering activation")
    seed = fs._read(org / "seeds" / (key + ".json"))
    if seed.get("schema") != 1 or seed.get("version") != 1 or digest(_encoded(seed)) != key:
        raise fs.Held("steering seed identity mismatch")
    _targets(seed)
    policy = fs._read(org / "overlays/policy.json")
    if policy.get("schema") != 1 or not isinstance(policy.get("suffix"), str):
        raise fs.Held("invalid private steering policy")
    return active, seed, policy


def _targets(seed):
    targets = seed.get("targets")
    if not isinstance(targets, dict) or not targets:
        raise fs.Held("invalid steering target table")
    identities = set()
    for harness, value in targets.items():
        if not isinstance(harness, str) or not harness or not isinstance(value, str) or not value:
            raise fs.Held("invalid steering target mapping")
        path = Path(value)
        if (path.is_absolute() or ".." in path.parts or "\\" in value
                or path.as_posix() != value or value == "."):
            raise fs.Held("noncanonical or unsafe steering target mapping")
        if value in identities:
            raise fs.Held("two harness adapters claim one normalized target")
        identities.add(value)
    return targets


def _read_target(target):
    if not fs._present(target):
        return None
    with fs._directory(target.parent) as parent:
        fd = os.open(target.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(fd, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise fs.Held("steering target is not an owned regular file")
            return stream.read()


def render(previous, body):
    text = previous.decode("utf-8") if previous is not None else ""
    open_guard = None
    seen = set()
    token = re.compile(r"<!-- (BEGIN|END) ([\w:./-]+) -->")
    for prefix in re.finditer(r"<!--\s*(?:BEGIN|END)\b", text, re.IGNORECASE):
        matched = token.match(text, prefix.start())
        if matched is None:
            raise fs.Held("malformed or incomplete managed region delimiter")
        action, name = matched.groups()
        if action == "BEGIN":
            if open_guard or name in seen:
                raise fs.Held("duplicate or nested managed region")
            open_guard = name
            seen.add(name)
        else:
            if name != open_guard:
                raise fs.Held("unbalanced managed region")
            open_guard = None
    if open_guard:
        raise fs.Held("unclosed foreign managed region")
    if re.search(r"<!--\s*(?:BEGIN|END)\b", body, re.IGNORECASE):
        raise fs.Held("seed/private suffix cannot inject managed region guards")
    if text.count(BEGIN) != text.count(END) or text.count(BEGIN) > 1:
        raise fs.Held("ambiguous or unbalanced steering guard")
    block = BEGIN + "\n" + body.rstrip() + "\n" + END
    if BEGIN in text:
        if text.index(END) < text.index(BEGIN):
            raise fs.Held("reversed steering guard")
        left, start = text.split(BEGIN)
        old, right = start.split(END)
        if "<!-- BEGIN " in old or "<!-- END " in old:
            raise fs.Held("foreign managed region nested in owned steering guard")
        return (left + block + right).encode()
    if END in text:
        raise fs.Held("reversed steering guard")
    return (text + ("\n\n" if text else "") + block + "\n").encode()


def plan(harness, home=None, env=None, explicit=None):
    org, selected = _organization(home, env, explicit)
    home = Path(home or Path.home()).absolute()
    active, seed, policy = _load(org)
    targets = _targets(seed)
    file_ids = set()
    for mapping in targets.values():
        candidate = home / mapping
        if fs._present(candidate):
            info = candidate.lstat()
            if stat.S_ISREG(info.st_mode):
                key = (info.st_dev, info.st_ino)
                if key in file_ids:
                    raise fs.Held("harness targets share one existing file identity")
                file_ids.add(key)
    relative = targets.get(harness)
    if not isinstance(relative, str) or Path(relative).is_absolute() or ".." in Path(relative).parts:
        raise fs.Held("unknown harness or unsafe target mapping")
    if list(targets.values()).count(relative) != 1:
        raise fs.Held("two harness adapters claim one target")
    target = home / relative
    previous = _read_target(target)
    output = render(previous, seed["root_markdown"] + "\n\n" + policy["suffix"])
    doc = {"schema": 1, "root_id": selected["root_id"], "seed": active["seed"],
           "policy_digest": digest(_encoded(policy)), "harness": harness,
           "target": str(target), "input_digest": digest(previous) if previous is not None else None,
           "output_digest": digest(output), "output": output.decode(),
           "changed": previous != output}
    doc["plan_id"] = digest(_encoded(doc))
    return doc


def stamp(expected, home=None, env=None, explicit=None):
    org, selected = _organization(home, env, explicit)
    # Coordinate with the shipped memory writer as well as other steer writers.
    # Uncooperative external editors are outside the trusted-writer guarantee.
    effective = os.environ if env is None else env
    if "YGGTERM_MEMORY_ROOT" in effective:
        raise fs.Held("custom memory roots require a qualified native-writer fence adapter")
    native_home = Path(home or Path.home()).absolute()
    # Steering-state overrides cannot enroll or redirect the independent
    # native memory writer. Validate its ordinary root separately.
    native_env = {k: v for k, v in effective.items() if k not in ("YGG_ROOT", "YGGTERM_HOME")}
    native = fs.resolve(home=native_home, env=native_env)
    if not native["initialized"] or not native["same_legacy_store"]:
        raise fs.Held("native memory hub root is not compatibly enrolled")
    # The memory owner's default is target-home/.yggterm/memory, independent
    # of a custom steering-state --root. Resolve the approved root alias.
    memory = Path(native["physical_path"]) / "memory"
    with fs._file_lock(memory, ".ygg-memory.lock"), fs._file_lock(org, "publication.lock"):
        current = plan(expected.get("harness"), home=home, env=env, explicit=explicit)
        if _encoded(expected) != _encoded(current):
            raise fs.Held("steering plan changed; retain drift and make a new plan")
        if not current["changed"]:
            return {"status": "unchanged", "plan_id": current["plan_id"]}
        target = Path(current["target"])
        with fs._directory(target.parent, create=True) as parent:
            temp = target.name + ".new-" + uuid.uuid4().hex
            try:
                fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent)
                with os.fdopen(fd, "wb") as stream:
                    stream.write(current["output"].encode())
                    stream.flush()
                    os.fsync(stream.fileno())
                latest = _read_target(target)
                if (digest(latest) if latest is not None else None) != current["input_digest"]:
                    raise fs.Held("target drifted before publication")
                os.replace(temp, target.name, src_dir_fd=parent, dst_dir_fd=parent)
                os.fsync(parent)
            finally:
                try: os.unlink(temp, dir_fd=parent)
                except FileNotFoundError: pass
        if digest(_read_target(target)) != current["output_digest"]:
            raise fs.Held("steering target readback differs from plan")
        receipt = {"status": "stamped", "plan_id": current["plan_id"],
                   "root_id": selected["root_id"], "target": str(target),
                   "output_digest": current["output_digest"], "scope": "local-target-only"}
        fs._atomic_json(org / "receipts" / (current["plan_id"] + ".json"), receipt)
        return receipt


def main(argv=None):
    p = argparse.ArgumentParser(prog="ygg steer", description="Versioned seed, private policy and exact local target plans")
    p.add_argument("--root", help="exact Ygg state root")
    sub = p.add_subparsers(dest="verb", required=True)
    for verb in ("init", "plan", "check", "stamp"):
        s = sub.add_parser(verb)
        s.add_argument("--json", action="store_true")
        if verb != "init": s.add_argument("harness")
        if verb == "plan": s.add_argument("--save", action="store_true", help="record this private plan for stamp")
        if verb == "stamp": s.add_argument("--plan", required=True, help="path to the saved exact plan")
    a = p.parse_args(argv)
    try:
        kw = {"explicit": a.root}
        if a.verb == "init": result = initialize(**kw)
        elif a.verb == "stamp":
            expected = fs._read(Path(a.plan))
            if expected.get("harness") != a.harness: raise fs.Held("plan names a different harness")
            result = stamp(expected, **kw)
        else:
            result = plan(a.harness, **kw)
            if a.verb == "check":
                result = {"status": "DRIFT" if result["changed"] else "match", "harness": a.harness,
                          "target": result["target"], "plan_id": result["plan_id"]}
            elif a.save:
                org, _ = _organization(**kw)
                path = org / "plans" / (result["plan_id"] + ".json")
                fs._atomic_json(path, result, create_once=True)
                result = {**result, "saved_plan": str(path)}
        if a.json: print(json.dumps(result, indent=2, sort_keys=True))
        elif a.verb == "init": print(f"Steering initialized; seed {result['seed'][:12]}; private policy preserved; no fleet stamps")
        else:
            print(f"{result.get('status', 'change planned' if result.get('changed') else 'unchanged')}: {result.get('target', a.harness)}")
            if result.get("saved_plan"): print("  saved plan: " + result["saved_plan"])
        return 1 if result.get("status") == "DRIFT" else 0
    except (fs.Held, OSError, ValueError, KeyError, TypeError) as exc:
        if a.json: print(json.dumps({"status": "HOLD", "detail": str(exc)}))
        else: print(f"ygg steer: HOLD: {exc}", file=sys.stderr)
        return 75


if __name__ == "__main__":
    raise SystemExit(main())
