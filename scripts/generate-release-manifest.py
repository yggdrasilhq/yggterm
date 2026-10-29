#!/usr/bin/env python3
"""Generate dist/release-manifest.json from the packaged seed artifacts.

The manifest is the single document ynpm seed-activate trusts: schema-1,
seed and desktop roles kept separate, real sha256 digests only. This
generator hard-fails on a missing checksum file or a placeholder digest —
a release with unverifiable bytes must not be described, and it must not
be published (the release workflow's publish predicate re-checks this).
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import tomllib


ROOT = Path(__file__).resolve().parents[1]

# Archive filename label -> release-manifest platform identifier
# (crates/ynpm/src/release_contract.rs KNOWN_RELEASE_PLATFORMS).
PLATFORMS = {
    "linux-x86_64": "linux-x64",
    "linux-aarch64": "linux-arm64",
    "macos-x86_64": "macos-x64",
    "macos-aarch64": "macos-arm64",
    "windows-x86_64": "windows-x64",
    "windows-aarch64": "windows-arm64",
}

# Bin names are BARE program names on every platform: the seed launcher
# keys the active.json pointer by bare name and resolves the `.exe`
# suffix itself on Windows. The Windows ARCHIVE still carries
# `ygg.exe`/`ynpm.exe`/`ynpx.exe` files (package-ygg-seed.py stages them
# with the suffix); only the declared bins table is bare.
SEED_BINS = ["ygg", "ynpm", "ynpx"]


def release_version() -> str:
    doc = tomllib.loads((ROOT / "Cargo.toml").read_text())
    return doc["workspace"]["package"]["version"]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def seed_artifacts(dist: Path) -> list[dict]:
    artifacts = []
    for archive in sorted(dist.glob("ygg-seed-*")):
        if archive.name.endswith((".sha256", ".manifest.json", ".json")):
            continue
        stem = archive.name
        for suffix in (".tar.gz", ".zip"):
            if stem.endswith(suffix):
                stem = stem[: -len(suffix)]
                break
        else:
            continue
        label = stem.removeprefix("ygg-seed-")
        platform = PLATFORMS.get(label)
        if platform is None:
            raise SystemExit(f"unmapped seed archive label: {archive.name}")
        checksum = archive.with_name(archive.name + ".sha256")
        if not checksum.is_file():
            raise SystemExit(
                f"missing checksum for {archive.name}: a seed artifact without "
                "its .sha256 is never described in a release manifest"
            )
        recorded = checksum.read_text().split()[0].strip().lower()
        actual = sha256_file(archive)
        if recorded != actual:
            raise SystemExit(f"checksum drift for {archive.name}: {recorded} != {actual}")
        windows = platform.startswith("windows")
        artifacts.append({
            "platform": platform,
            "format": "zip" if windows else "tar.gz",
            "filename": archive.name,
            "sha256": actual,
            "bins": SEED_BINS,
        })
    return artifacts


def build_manifest(dist: Path, include_desktop: Path | None) -> dict:
    version = release_version()
    manifest: dict = {
        "schema": 1,
        "release": {"version": version, "tag": f"v{version}"},
        "packages": [],
    }
    artifacts = seed_artifacts(dist)
    if not artifacts:
        raise SystemExit("no seed archives in dist: refusing to emit a seedless manifest")
    manifest["packages"].append({
        "id": "ygg-seed",
        "role": "seed",
        "summary": "GUI-free native ygg + ynpm seed (the bootstrap and worker package)",
        "artifacts": artifacts,
    })
    if include_desktop is not None:
        doc = json.loads(include_desktop.read_text())
        if doc.get("schema") != 1:
            raise SystemExit("desktop manifest fragment must be schema 1")
        for package in doc.get("packages", []):
            if package.get("role") != "desktop":
                continue
            # A desktop entry rides along only with real, present artifacts.
            for artifact in package.get("artifacts", []):
                if not (dist / artifact["filename"]).is_file():
                    raise SystemExit(
                        f"desktop artifact {artifact['filename']} is absent from dist; "
                        "a desktop package is never merged from placeholders"
                    )
                digest_file = dist / (artifact["filename"] + ".sha256")
                recorded = artifact.get("sha256", "").lower()
                if recorded.startswith("0" * 8):
                    raise SystemExit(
                        f"desktop artifact {artifact['filename']} carries a placeholder digest"
                    )
                if digest_file.is_file() and digest_file.read_text().split()[0].lower() != recorded:
                    raise SystemExit(f"desktop checksum drift for {artifact['filename']}")
            manifest["packages"].append(package)
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument(
        "--include-desktop-manifest", type=Path, default=None,
        help="optional schema-1 JSON fragment whose role:desktop packages are merged in",
    )
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args()
    manifest = build_manifest(args.dist, args.include_desktop_manifest)
    output = args.output or (args.dist / "release-manifest.json")
    output.write_text(json.dumps(manifest, indent=2) + "\n")
    print(output)


if __name__ == "__main__":
    main()
