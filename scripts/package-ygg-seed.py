#!/usr/bin/env python3
"""Build the GUI-free seed archive from already built native components.

Compilation belongs to the integration/release build plane. This script
never installs, activates a generation, changes PATH, or starts a daemon.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x86_64",
    "aarch64-unknown-linux-gnu": "linux-aarch64",
    "x86_64-apple-darwin": "macos-x86_64",
    "aarch64-apple-darwin": "macos-aarch64",
    "x86_64-pc-windows-msvc": "windows-x86_64",
    "aarch64-pc-windows-msvc": "windows-aarch64",
}


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def source(path: Path) -> Path:
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(ROOT) or not resolved.is_file():
        raise ValueError(f"payload source is outside the repository or not a file: {path}")
    return resolved


def copy_source(path: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source(path), target)


def version_gate(path: Path, expected: str) -> None:
    args = [str(path), "version", "--json"] if path.stem == "ygg" else [str(path), "--version"]
    result = subprocess.run(args, capture_output=True, text=True, timeout=20, check=True)
    answer = result.stdout.strip()
    if path.stem == "ygg":
        doc = json.loads(answer)
        frontend = doc.get("frontend", {})
        if doc.get("schema_version") != 1 or frontend.get("name") != "ygg" or frontend.get("version") != expected:
            raise ValueError(f"unexpected native frontend version from {path.name}: {answer!r}")
    elif answer not in (expected, f"ynpm {expected}", f"ynpx {expected}"):
        raise ValueError(f"unexpected component version from {path.name}: {answer!r}")


def package(binary_dir: Path, target: str, output: Path) -> Path:
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    label = TARGETS[target]
    suffix = ".exe" if label.startswith("windows-") else ""
    binaries = {"ygg": binary_dir / ("ygg" + suffix),
                "ynpm": binary_dir / ("ynpm" + suffix)}
    # Run the exact selected products on the native release runner. A build
    # artifact or a GUI job's success is not a substitute for this gate.
    for path in binaries.values():
        if not path.is_file():
            raise ValueError(f"missing native seed product: {path.name}")
        version_gate(path.resolve(), version)
    output.mkdir(parents=True, exist_ok=True)
    scratch = Path.home() / ".yggterm" / "scratchpad" / "seed-packages"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="seed-", dir=scratch) as temp:
        stage = Path(temp)
        # Seed bins live at the archive ROOT: ynpm seed-activate's bin gate
        # reads root-relative `bins` and moves each into <generation>/bin/.
        for name, path in binaries.items():
            shutil.copy2(path, stage / (name + suffix))
        # ynpx is the same manager product with argv[0]-selected launch mode.
        shutil.copy2(binaries["ynpm"], stage / ("ynpx" + suffix))
        # The native frontend owns its embedded IR, progressive skill and
        # local docs inventory. Do not maintain a second payload table here.
        payload = stage / "share/ygg"
        subprocess.run([str(binaries["ygg"].resolve()), "payload", "install",
                        "--dir", str(payload)], check=True, timeout=30)
        subprocess.run([str(binaries["ygg"].resolve()), "payload", "verify",
                        "--dir", str(payload)], check=True, timeout=30)
        fleet = ROOT / ".agents/skills/yggterm-agent-fleet"
        declared = json.loads((fleet / "ygg-verbs.json").read_text())
        names = set()
        for key in ("tier_base", "tier_gui", "tier_data", "owner_managed"):
            for name in declared.get(key, []):
                if not isinstance(name, str) or Path(name).name != name:
                    raise ValueError("invalid declared component filename")
                names.add(name)
        for name in sorted(names):
            copy_source(fleet / name, stage / "share/ygg/components" / name)
        for path in sorted((ROOT / "tools/swarmharness").rglob("*")):
            if path.is_file() and path.suffix in (".py", ".json", ".yaml"):
                copy_source(path, stage / "share/ygg/kernel" / path.relative_to(ROOT / "tools/swarmharness"))
        if sha(payload / "commands.json") != sha(ROOT / "tools/swarmharness/introspection/commands.json"):
            raise ValueError("native embedded command IR does not match the source being packaged")
        # SSOT containment: an installed generation must never reach back
        # into the repository it was built from. Every staged text file is
        # scanned for the absolute repository path; binary products are
        # skipped (a NUL in the first pages marks them and their embedded
        # build paths are a compiler concern, not a payload reference).
        repo_marker = str(ROOT).encode()
        for path in sorted(stage.rglob("*")):
            if not path.is_file() or path.stat().st_size == 0:
                continue
            with path.open("rb") as handle:
                head = handle.read(4096)
            if b"\x00" in head:
                continue
            if repo_marker in path.read_bytes():
                raise ValueError(
                    "staged payload references the build repository: "
                    + str(path.relative_to(stage))
                )
        entries = [{"path": p.relative_to(stage).as_posix(), "sha256": sha(p),
                    "size": p.stat().st_size, "executable": bool(p.stat().st_mode & 0o111)}
                   for p in sorted(stage.rglob("*")) if p.is_file()]
        manifest = {
            "schema_version": 1, "kind": "ygg-seed", "version": version,
            "target": target, "label": label,
            "command_contract_sha256": sha(stage / "share/ygg/commands.json"),
            "files": entries,
            "qualification": "native component version gates passed; role and installer qualification are separate",
            "worker_readiness": "not implied by seed installation",
        }
        (stage / "seed-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        extension = ".zip" if suffix else ".tar.gz"
        artifact = output / ("ygg-seed-" + label + extension)
        # Do not leave an apparently final archive after a failed writer.
        candidate = artifact.with_name(artifact.name + ".part")
        try:
            if suffix:
                with zipfile.ZipFile(candidate, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                    for p in sorted(stage.rglob("*")):
                        if p.is_file():
                            archive.write(p, p.relative_to(stage).as_posix())
            else:
                with tarfile.open(candidate, "w:gz") as archive:
                    for p in sorted(stage.rglob("*")):
                        if p.is_file():
                            archive.add(p, arcname=p.relative_to(stage).as_posix(), recursive=False)
            os.replace(candidate, artifact)
        finally:
            candidate.unlink(missing_ok=True)
        artifact.with_name(artifact.name + ".sha256").write_text(sha(artifact) + "  " + artifact.name + "\n")
        (output / ("ygg-seed-" + label + ".manifest.json")).write_text(json.dumps(manifest, indent=2) + "\n")
    return artifact


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary-dir", type=Path, required=True)
    p.add_argument("--target", choices=TARGETS, required=True)
    p.add_argument("--output", type=Path, default=ROOT / "dist")
    args = p.parse_args()
    print(package(args.binary_dir, args.target, args.output))


if __name__ == "__main__":
    main()
