#!/usr/bin/env python3
"""ygg-disperse - ship the ygg-* verb set to every fleet host (dream ACK-a0bd107330).

The skill dir this file lives in is the SSOT of the ygg verbs. This verb
installs faithful replicas into ~/.yggterm/bin/ygg/ on every fleet host,
records per-file provenance in ~/.yggterm/config/ygg-verbs/manifest.json,
PATH-links the base tier into ~/.local/bin, and verifies replicas have not
drifted. spec: docs/spec-ygg-verb-dispersal.md

Subcommands:
  install [--repo PATH] [--local] | [--fleet | --hosts LIST] [--json]
      Copy the verb set (ygg-verbs.json) from the repo into the replica tree
      and link the base tier into ~/.local/bin. Bare `install` is THIS HOST
      only; `--fleet` (the ~/.yggterm/auth/.fleet-hosts list) or `--hosts
      a,b,c` fans out: tar the staged set through the ssh pipe, then
      self-pipe this script to finalize remotely - the same idiom ygg-auth
      uses, no /tmp staging ever.
  finalize --commit SHA
      Runs ON a host after the tar extract (usually via the ssh self-pipe):
      normalize modes, write the manifest, install PATH links.
  verify [--hosts LIST] [--json]
      Exit 0 only when every replica matches the manifest sha AND every
      base-tier PATH link resolves into the replica tree; exit 1 with the
      drift list otherwise.
  status [--hosts LIST] [--json]
      The same evidence, reported not enforced.

Replicas are never hand-edited: the repo SSOT wins, drift is fixed by
re-running install.
"""
import argparse
import hashlib
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path

FLEET_HOSTS_FILE = Path("~/.yggterm/auth/.fleet-hosts").expanduser()
SELF = Path(__file__).resolve()
SCRIPT_NAME = "ygg-disperse.py"
LIST_NAME = "ygg-verbs.json"
EXEC_MODE = 0o755
DATA_MODE = 0o644


def yggterm_dir():
    return Path(os.environ.get("YGG_DISPERSE_YGGTERM", "~/.yggterm")).expanduser()


def replica_dir():
    return Path(os.environ.get("YGG_DISPERSE_BIN", yggterm_dir() / "bin" / "ygg"))


def manifest_path():
    return Path(os.environ.get(
        "YGG_DISPERSE_MANIFEST",
        yggterm_dir() / "config" / "ygg-verbs" / "manifest.json"))


def local_bin():
    return Path(os.environ.get("YGG_DISPERSE_LOCALBIN", "~/.local/bin")).expanduser()


def hostname():
    return socket.gethostname().split(".")[0].lower()


def die(msg):
    print(f"ygg-disperse: {msg}", file=sys.stderr)
    sys.exit(2)


def emit(out, args):
    print(json.dumps(out, indent=2, sort_keys=True) if getattr(args, "json", False)
          else json.dumps(out, sort_keys=True))


def load_shipment(ssot):
    """Read ygg-verbs.json and check every listed file exists in the SSOT dir."""
    spec_path = ssot / LIST_NAME
    if not spec_path.is_file():
        die(f"{spec_path} not found - --repo must point at the fleet skill dir's repo")
    spec = json.loads(spec_path.read_text())
    files = list(dict.fromkeys(spec["tier_base"] + spec.get("owner_managed", [])
                               + spec["tier_gui"]))
    if SCRIPT_NAME not in spec["tier_base"]:
        die(f"{LIST_NAME} must list {SCRIPT_NAME} in tier_base")
    missing = [f for f in files if not (ssot / f).is_file()]
    if missing:
        die(f"shipment list names files missing from {ssot}: {', '.join(missing)}")
    return spec, files


def resolve_ssot(explicit):
    if explicit:
        p = Path(explicit).expanduser().resolve()
        if not p.is_dir():
            die(f"--repo {p} is not a directory")
        return p
    # in-repo default: <repo>/.agents/skills/yggterm-agent-fleet/
    if (SELF.parent / LIST_NAME).is_file() and SELF.parent.parent.parent.name == ".agents":
        return SELF.parent
    for base in ("~/gh/yggterm", "~/git/yggterm"):
        cand = Path(base).expanduser() / ".agents" / "skills" / "yggterm-agent-fleet"
        if (cand / LIST_NAME).is_file():
            return cand
    die("cannot find the SSOT skill dir; pass --repo")


def repo_commit(ssot):
    """Short HEAD sha of the repo owning the SSOT dir, 'unknown' outside a repo."""
    marker = ssot
    for parent in [ssot, *ssot.parents]:
        if (parent / ".git").exists():
            marker = parent
            break
    r = subprocess.run(["git", "-C", str(marker), "rev-parse", "--short", "HEAD"],
                       capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else "unknown"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def atomic_symlink(target, link):
    """Point `link` at `target`; refuse to eat a directory in the way."""
    if link.is_symlink() and os.readlink(link) == str(target):
        return "kept"
    if link.is_dir() and not link.is_symlink():
        die(f"refusing to replace directory {link} with a PATH link")
    existed = os.path.lexists(link)
    tmp = link.with_name(link.name + ".newlink")
    if tmp.is_symlink() or tmp.exists():
        tmp.unlink()
    os.symlink(target, tmp)
    os.replace(tmp, link)
    return "replaced" if existed else "made"


def install_links(spec, bin_dir, out, source_dir=None):
    """PATH-link the base tier; a differing hand copy is preserved, not lost."""
    base = local_bin()
    base.mkdir(parents=True, exist_ok=True)
    links = {"made": 0, "kept": 0, "replaced": 0}
    stamp = time.strftime("%Y%m%d-%H%M%S")
    for name in spec["tier_base"]:
        link = base / name
        if link.is_file() and not link.is_symlink():
            same = source_dir is not None and \
                sha256(link) == sha256(source_dir / name)
            if not same:
                keep = manifest_path().parent / f"adopted-{stamp}-{name}"
                keep.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(link), str(keep))
                out.setdefault("preserved", []).append(str(keep))
            out.setdefault("adopted", []).append(str(link))
        what = atomic_symlink(bin_dir / name, link)
        links[what] += 1
    out["links"] = links


def build_manifest(spec, files, ssot, commit):
    bin_dir = replica_dir()
    # the shipment list ships with the tar, so it is a replica too - manifest it
    hashed = sorted(set(files) | {LIST_NAME})
    return {
        "installed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source_commit": commit,
        "ssot": str(ssot),
        "tier_base": spec["tier_base"],
        "owner_managed": spec.get("owner_managed", []),
        "executable": spec["executable"],
        "files": {name: sha256(bin_dir / name) for name in hashed},
    }


def write_manifest(manifest):
    p = manifest_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(p.parent), prefix=".tmp-disperse-")
    with os.fdopen(fd, "w") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
    os.replace(tmp, p)


def load_manifest():
    p = manifest_path()
    return json.loads(p.read_text()) if p.is_file() else None


def normalize_modes(spec, files):
    bin_dir = replica_dir()
    for name in files:
        os.chmod(bin_dir / name, EXEC_MODE if name in spec["executable"] else DATA_MODE)


def adopt_root_copies(files, out, owner_managed=()):
    """bin/ root hand-staged copies of shipment names become links into bin/ygg/.

    Identical duplicates are retired; differing bytes are preserved under
    config/ygg-verbs/ (nothing is lost, the root stops drifting). Anything
    that referenced the root path keeps working through the link.
    Owner-managed names are never touched: their component installs them
    itself (ygg-memory's sync-fleet unlinks symlinks on peers BY DESIGN).
    """
    root = replica_dir().parent
    adopted = out.setdefault("adopted", [])
    for name in files:
        if name in owner_managed or name == LIST_NAME:
            continue
        src = root / name
        if name == LIST_NAME or src.is_symlink() or not src.is_file():
            if src.is_dir() and not src.is_symlink():
                out.setdefault("skipped", []).append(f"{src} is a directory")
            continue
        if sha256(src) == sha256(replica_dir() / name):
            src.unlink()
        else:
            keep = manifest_path().parent / f"adopted-{time.strftime('%Y%m%d-%H%M%S')}-{name}"
            keep.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(src), str(keep))
            out.setdefault("preserved", []).append(str(keep))
        os.symlink(f"ygg/{name}", src)
        adopted.append(str(src))


def install_local(args):
    ssot = resolve_ssot(args.repo)
    spec, files = load_shipment(ssot)
    commit = repo_commit(ssot)
    bin_dir = replica_dir()
    bin_dir.mkdir(parents=True, exist_ok=True)
    prev_manifest = load_manifest()
    prev = set(prev_manifest["files"]) if prev_manifest else set()

    out = {"host": hostname(), "commit": commit, "ssot": str(ssot)}
    for name in files:
        shutil.copy2(ssot / name, bin_dir / name)
    normalize_modes(spec, files)
    adopt_root_copies(files, out, spec.get("owner_managed", ()))
    out["installed"] = len(files)
    out["pruned"] = sorted(prev - set(files))
    for name in out["pruned"]:
        (bin_dir / name).unlink(missing_ok=True)

    write_manifest(build_manifest(spec, files, ssot, commit))
    install_links(spec, bin_dir, out, source_dir=ssot)
    emit(out, args)


def cmd_finalize(args):
    """Remote tail of a fleet install: the tar already extracted the files."""
    spec, files = load_shipment(replica_dir())
    normalize_modes(spec, files)
    out = {"host": hostname(), "commit": args.commit, "installed": len(files)}
    adopt_root_copies(files, out, spec.get("owner_managed", ()))
    write_manifest(build_manifest(spec, files, replica_dir(), args.commit))
    install_links(spec, replica_dir(), out, source_dir=replica_dir())
    emit(out, args)


def stage_tar(spec, files, ssot):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tar:
        for name in files + [LIST_NAME]:
            info = tar.gettarinfo(str(ssot / name), arcname=name)
            info.mode = EXEC_MODE if name in spec["executable"] else DATA_MODE
            with open(ssot / name, "rb") as f:
                tar.addfile(info, f)
    return buf.getvalue()


def ssh_run(dest, argv, input_bytes, timeout=120):
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", dest, *argv]
    try:
        return subprocess.run(cmd, input=input_bytes, capture_output=True,
                              timeout=timeout)
    except subprocess.TimeoutExpired:
        return None


def fleet_hosts(explicit):
    if explicit:
        return [h.strip() for h in explicit.split(",") if h.strip()]
    if FLEET_HOSTS_FILE.is_file():
        hosts = [h.strip() for h in FLEET_HOSTS_FILE.read_text().split(",") if h.strip()]
        if hosts:
            return hosts
    return [hostname()]


def cmd_install(args):
    hosts = fleet_hosts(args.hosts) if (args.fleet or args.hosts) else [hostname()]
    me = hostname()
    ssot = resolve_ssot(args.repo) if any(h != me for h in hosts) else None
    spec = files = commit = None
    if ssot is not None:
        spec, files = load_shipment(ssot)
        commit = repo_commit(ssot)
    results = {}
    failed = False
    for host in hosts:
        if host == me:
            r = subprocess.run(
                [sys.executable, str(SELF), "install", "--local", "--json",
                 *(["--repo", args.repo] if args.repo else [])],
                capture_output=True, text=True)
            results[host] = json.loads(r.stdout) if r.returncode == 0 else {
                "error": (r.stderr or r.stdout).strip()[:400]}
            failed |= r.returncode != 0
            continue
        tar_bytes = stage_tar(spec, files, ssot)
        p = ssh_run(host, ["mkdir", "-p", "~/.yggterm/bin/ygg",
                           "~/.yggterm/config/ygg-verbs", "&&",
                           "tar", "-xf", "-", "-C", "~/.yggterm/bin/ygg"], tar_bytes)
        if p is None or p.returncode != 0:
            results[host] = {"error": (p and p.stderr.decode()[:400]) or "ssh failed"}
            failed = True
            continue
        # self-pipe: finalize runs this very script on the host, from stdin
        self_bytes = (ssot / SCRIPT_NAME).read_bytes()
        p = ssh_run(host, ["python3", "-", "finalize", "--commit", commit,
                           "--json"], self_bytes)
        if p is None or p.returncode != 0:
            results[host] = {"error": (p and p.stderr.decode()[:400]) or "finalize failed"}
            failed = True
            continue
        results[host] = json.loads(p.stdout.decode())
    emit({"hosts": results}, args)
    sys.exit(1 if failed else 0)


def verify_local():
    """The drift evidence for THIS host. Also the remote payload of verify --hosts."""
    manifest = load_manifest()
    if manifest is None:
        return {"host": hostname(), "ok": False,
                "drift": ["no manifest - never installed"]}, False
    drift = []
    bin_dir = replica_dir()
    for name, digest in sorted(manifest["files"].items()):
        f = bin_dir / name
        if not f.is_file():
            drift.append(f"missing: {name}")
        elif sha256(f) != digest:
            drift.append(f"drifted: {name}")
    on_disk = {p.name for p in bin_dir.iterdir()
               if p.is_file() and not p.is_symlink()} if bin_dir.is_dir() else set()
    for extra in sorted(on_disk - set(manifest["files"])):
        drift.append(f"unmanifested: {extra}")
    base = local_bin()
    for name in manifest["tier_base"]:
        link = base / name
        if not link.is_symlink():
            drift.append(f"link-missing: {link}")
        elif not str(link.resolve()).startswith(str(replica_dir().resolve())):
            drift.append(f"link-elsewhere: {link}")
    return ({"host": hostname(), "ok": not drift, "commit": manifest["source_commit"],
             "replicas": len(manifest["files"]), "drift": drift}, not drift)


def cmd_verify(args):
    if not args.hosts and not args.fleet:
        out, ok = verify_local()
        emit(out, args)
        sys.exit(0 if ok else 1)
    results, ok = {}, True
    self_bytes = SELF.read_bytes() if SELF.is_file() else None
    if self_bytes is None:
        die("verify --hosts self-pipes this script; run it from the repo or a replica")
    for host in fleet_hosts(args.hosts):
        if host == hostname():
            out, host_ok = verify_local()
        else:
            p = ssh_run(host, ["python3", "-", "verify", "--json"], self_bytes)
            if p is None or p.returncode not in (0, 1):
                out, host_ok = {"host": host, "ok": False,
                                "drift": [(p and p.stderr.decode()[:400]) or "ssh failed"]}, False
            else:
                out, host_ok = json.loads(p.stdout.decode()), p.returncode == 0
        results[host] = out
        ok &= host_ok
    emit({"hosts": results}, args)
    sys.exit(0 if ok else 1)


def cmd_status(args):
    out, ok = verify_local()
    out["fleet"] = fleet_hosts(args.hosts) if args.hosts else fleet_hosts(None)
    emit(out, args)
    sys.exit(0 if ok else 1)


def main():
    ap = argparse.ArgumentParser(prog="ygg-disperse",
                                 description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("install",
                       help="ship the verb set to this host (--fleet/--hosts to fan out)")
    p.add_argument("--repo", help="SSOT repo skill dir (default: auto-detect)")
    p.add_argument("--hosts", help="comma list to fan out over ssh")
    p.add_argument("--fleet", action="store_true",
                   help="fan out to ~/.yggterm/auth/.fleet-hosts")
    p.add_argument("--local", action="store_true", help="this host only (default)")
    p.add_argument("--json", action="store_true")

    p = sub.add_parser("finalize", help="remote tail after tar extract")
    p.add_argument("--commit", required=True)
    p.add_argument("--json", action="store_true")
    p.set_defaults(fn=cmd_finalize)

    p = sub.add_parser("verify", help="exit 0 when replicas match the manifest")
    p.add_argument("--hosts", help="comma list to verify across the fleet")
    p.add_argument("--fleet", action="store_true",
                   help="verify the ~/.yggterm/auth/.fleet-hosts list")
    p.add_argument("--json", action="store_true")
    p.set_defaults(fn=cmd_verify)

    p = sub.add_parser("status", help="report install state (report-only)")
    p.add_argument("--hosts", help="fleet hint reported alongside")
    p.add_argument("--json", action="store_true")
    p.set_defaults(fn=cmd_status)

    args = ap.parse_args()
    if args.cmd == "install":
        if args.fleet or args.hosts:
            cmd_install(args)
        else:
            install_local(args)
    else:
        args.fn(args)


if __name__ == "__main__":
    main()
