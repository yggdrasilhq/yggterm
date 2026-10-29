"""One location contract; root aliases preserve legacy data and lock inodes.

This tranche never moves existing data or deletes/quarantines objects. Owner
adapters and fenced reachability must qualify destructive operations first.
"""
from __future__ import annotations
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import uuid
from contextlib import contextmanager

CONTRACT_PATH = Path(__file__).with_name("ygg-fs-locations.json")

class Held(RuntimeError):
    pass


def contract():
    doc = json.loads(CONTRACT_PATH.read_text())
    if doc.get("schema") != 1 or doc.get("contract_version") != 1:
        raise Held("unsupported filesystem location contract")
    return doc


def _read(path):
    try:
        if not stat.S_ISREG(path.lstat().st_mode):
            raise Held(f"metadata is not a regular file: {path.name}")
        doc = json.loads(path.read_text())
    except (OSError, ValueError) as exc:
        raise Held(f"unreadable metadata: {path.name} ({type(exc).__name__})") from exc
    if not isinstance(doc, dict):
        raise Held(f"invalid metadata: {path.name}")
    return doc


def _config_dir(home, env):
    value = env.get("XDG_CONFIG_HOME")
    if value is not None and not value.strip():
        raise Held("XDG_CONFIG_HOME is empty")
    return (Path(value).expanduser().absolute() if value else home / ".config") / "ygg"


def _host_id(home, env, create=False):
    path = _config_dir(home, env) / "host.json"
    if not os.path.lexists(path):
        if not create:
            return None
        _atomic_json(path, {"schema": 1, "host_id": str(uuid.uuid4())}, create_once=True)
    doc = _read(path)
    try:
        if doc.get("schema") != 1 or str(uuid.UUID(doc["host_id"])) != doc["host_id"]:
            raise ValueError()
    except (KeyError, ValueError, TypeError):
        raise Held("incompatible host identity")
    return doc["host_id"]


def _identity(root):
    marker = root / "root.json"
    if not os.path.lexists(marker):
        return None
    doc = _read(marker)
    try:
        valid = (doc.get("schema") == 1 and doc.get("contract_version") == 1
                 and type(doc.get("layout_epoch")) is int
                 and doc["layout_epoch"] >= 1
                 and str(uuid.UUID(doc["root_id"])) == doc["root_id"]
                 and str(uuid.UUID(doc["host_id"])) == doc["host_id"]
                 and doc.get("reader_versions") == [1] and doc.get("writer_versions") == [1])
    except (KeyError, TypeError, ValueError):
        valid = False
    if not valid:
        raise Held("incompatible root identity; no automatic rewrite")
    return doc


def _canonical(path):
    try:
        return path.resolve(strict=True)
    except (OSError, RuntimeError) as exc:
        raise Held(f"root cannot be resolved: {path} ({type(exc).__name__})") from exc


def _present(path):
    return os.path.lexists(path)


def _same(first, second):
    try:
        return first.samefile(second)
    except OSError as exc:
        raise Held("cannot verify root identity") from exc


def resolve(home=None, env=None, explicit=None):
    home = Path(home or Path.home()).absolute()
    env = os.environ if env is None else env
    doc = contract()
    modern, legacy = home / doc["canonical_directory"], home / doc["legacy_directory"]
    supplied = []
    for name, value in [("--root", explicit), ("YGG_ROOT", env.get("YGG_ROOT")),
                        ("YGGTERM_HOME", env.get("YGGTERM_HOME"))]:
        if value is not None:
            if not str(value).strip():
                raise Held(f"{name} is empty; an override is an exact state root")
            path = Path(str(value)).expanduser().absolute()
            supplied.append((name, path))
    if supplied:
        path = supplied[0][1]
        for _, other in supplied[1:]:
            if other != path and (not path.exists() or not other.exists() or not _same(path, other)):
                raise Held("conflicting exact root overrides; no implicit precedence")
        root, selection = path, supplied[0][0]
        if _present(root):
            physical = _canonical(root)
        else:
            physical = root
    else:
        for candidate in (modern, legacy):
            if _present(candidate) and not candidate.exists():
                raise Held(f"broken or looping root alias: {candidate.name}")
        if modern.exists() and legacy.exists() and not _same(modern, legacy):
            raise Held("two independent roots exist; no timestamp selection or merge")
        locator_path = _config_dir(home, env) / "root-location.json"
        locator = _read(locator_path) if os.path.lexists(locator_path) else None
        if locator is not None:
            if locator.get("schema") != 1 or locator.get("contract_version") != 1:
                raise Held("unsupported bootstrap root locator")
            root = Path(locator.get("path", ""))
            if not root.is_absolute() or not root.exists():
                raise Held("selected bootstrap root is unavailable; no legacy fallback")
            identity = _identity(root)
            if identity is None or any(identity.get(key) != locator.get(key) for key in ("root_id", "layout_epoch")):
                raise Held("bootstrap locator and selected root identity disagree")
            selection = "bootstrap-locator"
        elif modern.exists():
            root, selection = modern, "canonical"
        elif legacy.exists():
            root, selection = legacy, "legacy-compatibility"
        else:
            root, selection = modern, "fresh-uninitialized"
        physical = _canonical(root) if root.exists() else root
        if _present(root) and (root.is_symlink() and physical.parent != home.resolve()):
            raise Held("default root alias escapes the enrolled home; use an explicit root")
    if root.exists() and not root.is_dir():
        raise Held("selected root is not a directory")
    identity = _identity(root) if root.exists() else None
    if identity and identity.get("host_id") != _host_id(home, env):
        raise Held("root belongs to a different or uninitialized host identity")
    if identity and identity.get("migration_state", "compatible") != "compatible":
        raise Held("root migration is not in a compatible state; recover before writes")
    return {"schema": 1, "contract_version": doc["contract_version"],
            "path": str(root), "physical_path": str(physical), "selection": selection,
            "initialized": identity is not None, "root_id": identity.get("root_id") if identity else None,
            "layout_epoch": identity.get("layout_epoch") if identity else None,
            "same_legacy_store": legacy.exists() and root.exists() and _same(root, legacy),
            "subtree_conversion_complete": False,
            "physical_relocation_complete": root.exists() and not root.is_symlink() and root == modern}


@contextmanager
def _directory(path, create=False):
    """Traverse owned output directories without following ancestor links.

    The selected store's one compatibility alias is resolved by its caller;
    metadata subdirectories are never aliases. Retain handles for publication
    so a changed pathname cannot redirect writes into another directory.
    """
    path = Path(path).absolute()
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    fd = os.open(path.anchor, flags)
    try:
        for part in path.parts[1:]:
            try:
                child = os.open(part, flags, dir_fd=fd)
            except FileNotFoundError:
                if not create:
                    raise
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                    os.fsync(fd)
                except FileExistsError:
                    pass
                child = os.open(part, flags, dir_fd=fd)
            if create:
                # Replay may encounter a directory created just before a
                # previous parent-fsync failure. Existing names need the same
                # durability acknowledgement as newly created names.
                os.fsync(fd)
            os.close(fd)
            fd = child
        info = os.fstat(fd)
        if info.st_uid != os.getuid() or info.st_mode & 0o022:
            raise Held("metadata directory is not privately owned")
        yield fd
    finally:
        os.close(fd)


@contextmanager
def _file_lock(directory, name):
    if os.name != "posix":
        raise Held("bootstrap alias mutation is not yet qualified on this platform")
    import fcntl
    with _directory(directory, create=True) as parent:
        fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK,
                     0o600, dir_fd=parent)
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise Held("bootstrap lock is not an owned regular file")
            os.fsync(parent)
            fcntl.flock(fd, fcntl.LOCK_EX)
            yield
        finally:
            os.close(fd)


def _bootstrap_lock(home, env):
    # External locator lock survives any future physical store relocation.
    return _file_lock(_config_dir(Path(home), env), "root-bootstrap.lock")


def _atomic_json(path, doc, create_once=False):
    path = Path(path)
    with _directory(path.parent, create=True) as parent:
        temp = path.name + ".new-" + uuid.uuid4().hex
        try:
            fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o600, dir_fd=parent)
            with os.fdopen(fd, "w") as stream:
                json.dump(doc, stream, indent=2, sort_keys=True)
                stream.write("\n")
                stream.flush()
                os.fsync(stream.fileno())
            if create_once:
                try:
                    os.link(temp, path.name, src_dir_fd=parent, dst_dir_fd=parent,
                            follow_symlinks=False)
                except FileExistsError:
                    return False
            else:
                try:
                    existing = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
                except FileNotFoundError:
                    pass
                else:
                    if not stat.S_ISREG(existing.st_mode) or existing.st_uid != os.getuid():
                        raise Held("metadata output is not an owned regular file")
                os.replace(temp, path.name, src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)
            return True
        finally:
            try:
                os.unlink(temp, dir_fd=parent)
                os.fsync(parent)
            except FileNotFoundError:
                pass


def _repair_default_aliases(home, root):
    names = (contract()["canonical_directory"], contract()["legacy_directory"])
    if root.parent != home.resolve() or root.name not in names:
        raise Held("default locator root needs an explicit relocation adapter")
    with _directory(home) as parent:
        for name in names:
            alias = home / name
            if not _present(alias):
                try:
                    os.symlink(root.name, name, dir_fd=parent, target_is_directory=True)
                    os.fsync(parent)
                except FileExistsError:
                    pass
            if not _same(alias, root):
                raise Held("compatibility alias did not resolve to the enrolled store")
            # Also resync an existing alias after an interrupted prior fsync.
            os.fsync(parent)


def _recover_locator_alias(home, env):
    """Mutation-only recovery of a missing default alias, never new genesis."""
    locator = _config_dir(home, env) / "root-location.json"
    if not _present(locator):
        return
    doc = _read(locator)
    target = Path(doc.get("path", ""))
    names = (contract()["canonical_directory"], contract()["legacy_directory"])
    if _present(target) or target.exists():
        return
    if (doc.get("schema") != 1 or doc.get("contract_version") != 1
            or target not in [home / name for name in names]):
        raise Held("missing locator root has no approved alias recovery")
    other = home / next(name for name in names if name != target.name)
    if not other.exists():
        raise Held("selected bootstrap root is unavailable; no replacement genesis")
    physical = _canonical(other)
    with _file_lock(physical, "root-bootstrap.lock"):
        identity = _identity(physical)
        if (identity is None or identity.get("host_id") != _host_id(home, env)
                or identity.get("migration_state") != "compatible"
                or any(identity.get(k) != doc.get(k) for k in ("root_id", "layout_epoch"))):
            raise Held("locator recovery candidate identity disagrees")
        _repair_default_aliases(home, physical)


def initialize(home=None, env=None, explicit=None):
    home = Path(home or Path.home()).absolute()
    env = os.environ if env is None else env
    with _bootstrap_lock(home, env):
        default = explicit is None and not any(k in env for k in ("YGG_ROOT", "YGGTERM_HOME"))
        if default:
            _recover_locator_alias(home, env)
        selected = resolve(home, env, explicit)
        root = Path(selected["physical_path"])
        with _file_lock(root, "root-bootstrap.lock"):
            # Different XDG profiles have different external locks. The store
            # lock and create-once identity publication protect their common root.
            host = _host_id(home, env, create=True)
            marker = _identity(root)
            if marker is None:
                candidate = {"schema": 1, "contract_version": 1, "root_id": str(uuid.uuid4()),
                             "host_id": host, "reader_versions": [1], "writer_versions": [1],
                             "layout_epoch": 1, "migration_state": "compatible"}
                _atomic_json(root / "root.json", candidate, create_once=True)
                marker = _identity(root)
            if marker["host_id"] != host:
                raise Held("root belongs to a different host identity; no overwrite")
            if default:
                _repair_default_aliases(home, root)
            result = resolve(home, env, explicit)
            if default:
                locator = _config_dir(home, env) / "root-location.json"
                record = {"schema": 1, "root_id": marker["root_id"],
                          "path": result["path"], "layout_epoch": marker["layout_epoch"],
                          "contract_version": 1, "state": "compatible-alias-only"}
                _atomic_json(locator, record, create_once=True)
                # Read the selected locator again after a no-clobber race.
                result = resolve(home, env, explicit)
            return result


def location(name, **kwargs):
    selected = resolve(**kwargs)
    spec = contract()["locations"].get(name)
    if spec is None:
        if name.startswith("app.") and re.fullmatch(r"[a-z][a-z0-9-]{0,63}", name[4:]):
            spec = {"owner": name[4:], "path": "libyggterm/" + name[4:],
                    "class": "app-config", "conversion": "new-location-not-yet-consumed"}
        else:
            raise Held("unknown location ID")
    return {**selected, "location_id": name, "owner": spec["owner"],
            "location_path": str(Path(selected["path"]) / spec["path"]),
            "class": spec["class"], "conversion": spec.get("conversion", "layout-preserved"),
            "target_path": str(Path(selected["path"]) / spec.get("target", spec["path"]))}


def gc_plan(**kwargs):
    selected = resolve(**kwargs)
    root = Path(selected["physical_path"])
    if not selected["initialized"]:
        raise Held("initialize the root identity before recording a GC plan")
    rows = []
    for entry in sorted(root.iterdir(), key=lambda p: p.name):
        st = entry.lstat()
        rows.append({"name": entry.name, "device": st.st_dev, "inode": st.st_ino,
                     "kind": "link" if stat.S_ISLNK(st.st_mode) else "directory" if stat.S_ISDIR(st.st_mode) else "file",
                     "action": "retain", "reason": "no qualified owner reachability and launch fence"})
    # Metadata only; never walk secrets/session stores or infer disposal
    # authority from age, location, process absence, or free disk space.
    basis = {"schema": 1, "root_id": selected["root_id"], "layout_epoch": selected["layout_epoch"],
             "policy_version": contract()["gc_policy"]["version"], "objects": rows,
             "eligible_objects": 0, "reclaimed_bytes": 0, "mode": "plan-only",
             "missing_capabilities": contract()["gc_policy"]["apply_requires"]}
    digest = hashlib.sha256(json.dumps(basis, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    basis["plan_id"] = digest
    basis["observed_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
    _atomic_json(root / "gc/plans" / (digest + ".json"), basis)
    return basis


def cli(argv=None):
    parser = argparse.ArgumentParser(prog="ygg fs", description="Stable root identity and owner-aware retention planning")
    parser.add_argument("--root", help="exact state root override")
    sub = parser.add_subparsers(dest="verb", required=True)
    for name in ("status", "init", "path", "gc"):
        p = sub.add_parser(name)
        p.add_argument("--json", action="store_true")
        if name == "path": p.add_argument("location", help="stable ID, e.g. kernel.swarm or app.ychrome")
        if name == "gc": p.add_argument("action", choices=["plan", "apply"])
    args = parser.parse_args(argv)
    try:
        kwargs = {"explicit": args.root}
        if args.verb == "init": result = initialize(**kwargs)
        elif args.verb == "status": result = resolve(**kwargs)
        elif args.verb == "path": result = location(args.location, **kwargs)
        elif args.action == "plan": result = gc_plan(**kwargs)
        else: raise Held("GC apply unavailable: owner reference/fence adapters are not qualified; no objects changed")
        if args.json:
            print(json.dumps(result, indent=2, sort_keys=True))
        elif args.verb == "gc":
            print(f"GC plan {result['plan_id'][:12]}: {len(result['objects'])} objects retained, 0 eligible; 0 bytes reclaimed")
            print("  automatic deletion disabled; owner reference and launch-fence adapters still required")
        else:
            print(result.get("location_path", result["path"]))
            print(f"  root {result['root_id'] or 'uninitialized'}; contract 1; selection {result['selection']}")
            if result.get("conversion"):
                print(f"  {result['conversion']}")
            if not result['subtree_conversion_complete']:
                print("  application and kernel subtree conversion remains incomplete")
        return 0
    except (Held, OSError, ValueError) as exc:
        if args.json: print(json.dumps({"status": "HOLD", "detail": str(exc)}))
        else: print(f"ygg fs: HOLD — {exc}", file=sys.stderr)
        return 75

if __name__ == "__main__":
    raise SystemExit(cli())
