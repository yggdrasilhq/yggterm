#!/usr/bin/env python3
import json
import sys
from pathlib import Path


REQUIRED_TOP_LEVEL = {
    "bundle_version",
    "app_id",
    "feature_id",
    "title",
    "status",
    "category",
    "user_value",
    "source_repo",
    "links",
    "macro",
    "proof",
    "changelog",
}
REQUIRED_LINKS = {"changelog_section", "site_demo_path"}
REQUIRED_MACRO = {"script", "args"}
REQUIRED_PROOF = {"screenshots", "recording", "trace", "app_state"}
REQUIRED_CHANGELOG = {"section", "summary"}


def validate_bundle(bundle_dir: Path) -> list[str]:
    issues: list[str] = []
    manifest_path = bundle_dir / "manifest.json"
    summary_path = bundle_dir / "summary.md"
    if not manifest_path.exists():
        return [f"{bundle_dir}: missing manifest.json"]
    if not summary_path.exists():
        issues.append(f"{bundle_dir}: missing summary.md")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [f"{manifest_path}: invalid json: {exc}"]

    missing = REQUIRED_TOP_LEVEL - set(manifest.keys())
    if missing:
        issues.append(f"{manifest_path}: missing top-level fields: {sorted(missing)}")

    nested_checks = (
        ("links", REQUIRED_LINKS),
        ("macro", REQUIRED_MACRO),
        ("proof", REQUIRED_PROOF),
        ("changelog", REQUIRED_CHANGELOG),
    )
    for name, required in nested_checks:
        value = manifest.get(name)
        if not isinstance(value, dict):
            issues.append(f"{manifest_path}: {name} must be an object")
            continue
        nested_missing = required - set(value.keys())
        if nested_missing:
            issues.append(f"{manifest_path}: {name} missing fields: {sorted(nested_missing)}")

    if manifest.get("app_id") != "yggterm":
        issues.append(f"{manifest_path}: app_id must be 'yggterm'")
    return issues


def main() -> int:
    root = Path(__file__).resolve().parents[1] / "artifacts" / "demos" / "unreleased"
    bundles = sorted(path for path in root.iterdir() if path.is_dir())
    issues: list[str] = []
    if not bundles:
        issues.append(f"{root}: no demo bundles found")
    for bundle in bundles:
        issues.extend(validate_bundle(bundle))
    if issues:
        for issue in issues:
            print(issue, file=sys.stderr)
        return 1
    print(f"validated {len(bundles)} demo bundle(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
