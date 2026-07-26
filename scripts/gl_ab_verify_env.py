#!/usr/bin/env python3
"""Read ONE arm's published GL environment out of a `server app desktop-identity`
report, and say whether the keys that decide the GL path behind our back are
absent from it.

WHY THIS IS A FILE AND NOT A HEREDOC. It used to be inlined in
`gl_ab_experiment.sh` as

    read -r policy absent < <(python3 - "$arm" <<-'PY' <<<"$identity"

and both redirections target fd 0. The herestring is last, so it WON: `python3 -`
read the desktop-identity JSON as its program, raised `NameError: name 'true' is
not defined` on every run, and `verify_arm` died ~15 s into the first arm of
every experiment. Nothing in the shipped harness had ever completed an arm. A
separate file takes its input as a path, so there is exactly one thing on fd 0
and the whole class of bug is gone — and it can be self-tested without a GUI, a
compositor, or a desktop, which is what let this defect hide.

The keys come from ARGV on purpose. The list of "environment that must not be
allowed to decide the GL path" is owned by
`yggterm_core::gl_probe::GL_PROBE_STRIPPED_ENV`; the harness carries the shell
half in `SOFTWARE_GL_KEYS` and a Rust drift lock keeps the two together. If this
script kept its own copy that would be a THIRD encoding — which is exactly what
it was: it checked 3 of the 4 keys, and the one it dropped
(`WEBKIT_DISABLE_COMPOSITING_MODE`) was the one the same branch had just carved
an exemption for.

Usage:
  gl_ab_verify_env.py <desktop-identity.json> <key>...
  gl_ab_verify_env.py --self-test

Prints one line to stdout: "<policy> <yes|no>" — the resolved
YGGTERM_WEBKIT_GL_POLICY, and whether every named key is ABSENT from the
published environment.

Exit codes:
  0  a verdict was printed
  2  usage error, including "no keys given" — an absence assertion over an
     empty key set is vacuously true, so it is refused rather than answered
  3  the report carries no webkit_gl_environment at all. That is a different
     fact from "the policy key is missing" and the caller must not conflate
     them: it means the client never published, not that it resolved wrong.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

POLICY_KEY = "YGGTERM_WEBKIT_GL_POLICY"


class NoPublishedEnvironment(Exception):
    """The report contains no webkit_gl_environment node."""


def published_gl_environment(document: object) -> dict[str, str]:
    """Merge every `webkit_gl_environment` map in the report.

    The report nests one per registered client instance, and an arm launches
    exactly one client into its own YGGTERM_HOME, so in practice there is one.
    Merging rather than picking is deliberate: if a second client ever appears
    in an arm's private home, its keys land in the same map and the absence
    assertion fires on it too. Silently choosing one would hide it.
    """
    found = False
    merged: dict[str, str] = {}

    def walk(node: object) -> None:
        nonlocal found
        if isinstance(node, dict):
            environment = node.get("webkit_gl_environment")
            if isinstance(environment, dict):
                found = True
                merged.update({str(k): str(v) for k, v in environment.items()})
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(document)
    if not found:
        raise NoPublishedEnvironment
    return merged


def verdict(document: object, keys: list[str]) -> str:
    environment = published_gl_environment(document)
    policy = environment.get(POLICY_KEY, "MISSING")
    absent = all(key not in environment for key in keys)
    return f"{policy} {'yes' if absent else 'no'}"


def main(argv: list[str]) -> int:
    if len(argv) >= 1 and argv[0] == "--self-test":
        return self_test()
    if len(argv) < 2:
        print(
            "usage: gl_ab_verify_env.py <desktop-identity.json> <key>...\n"
            "refusing to answer with no keys: an absence assertion over an "
            "empty key set is vacuously true",
            file=sys.stderr,
        )
        return 2
    path = Path(argv[0])
    keys = argv[1:]
    try:
        document = json.loads(path.read_text(errors="replace"))
    except OSError as error:
        print(f"cannot read {path}: {error}", file=sys.stderr)
        return 2
    except ValueError as error:
        print(f"{path} is not JSON: {error}", file=sys.stderr)
        return 2
    try:
        print(verdict(document, keys))
    except NoPublishedEnvironment:
        print(
            f"{path} carries no webkit_gl_environment — the client never "
            f"published its GL environment, which is NOT the same as resolving "
            f"to the wrong policy",
            file=sys.stderr,
        )
        return 3
    return 0


# ---------------------------------------------------------------------------
# self-test: prove each answer and each refusal FIRES. A checker that can only
# say "yes" is worth nothing — which is how the 3-of-4 key list survived.
# ---------------------------------------------------------------------------
KEYS = [
    "LIBGL_ALWAYS_SOFTWARE",
    "GALLIUM_DRIVER",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
]


def _identity(environment: dict[str, str]) -> dict:
    """The shape `server app desktop-identity` actually emits: the GL map is
    nested under a client instance, not at the top level."""
    return {
        "desktop_file": {"name": "yggterm"},
        "clients": [
            {
                "pid": 4242,
                "webkit_gl_environment": environment,
            }
        ],
    }


def self_test() -> int:
    failures: list[str] = []

    def expect(label: str, got: str, want: str) -> None:
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    expect(
        "hardware-clean",
        verdict(_identity({POLICY_KEY: "hardware_gl_probed"}), KEYS),
        "hardware_gl_probed yes",
    )
    expect(
        "software-forced",
        verdict(
            _identity(
                {
                    POLICY_KEY: "software_gl_forced",
                    "LIBGL_ALWAYS_SOFTWARE": "1",
                    "GALLIUM_DRIVER": "llvmpipe",
                }
            ),
            KEYS,
        ),
        "software_gl_forced no",
    )
    # THE FINDING: the old inline check knew 3 keys. An arm that inherited
    # WEBKIT_DISABLE_COMPOSITING_MODE reported hardware and presented over SHM,
    # and the assertion said "yes, clean".
    expect(
        "inherited-compositing-key",
        verdict(
            _identity(
                {
                    POLICY_KEY: "hardware_gl_probed",
                    "WEBKIT_DISABLE_COMPOSITING_MODE": "1",
                }
            ),
            KEYS,
        ),
        "hardware_gl_probed no",
    )
    expect(
        "inherited-dmabuf-key",
        verdict(
            _identity(
                {
                    POLICY_KEY: "hardware_gl_probed",
                    "WEBKIT_DISABLE_DMABUF_RENDERER": "1",
                }
            ),
            KEYS,
        ),
        "hardware_gl_probed no",
    )
    expect(
        "no-policy-key",
        verdict(_identity({}), KEYS),
        "MISSING yes",
    )

    # A report with no GL map at all is a distinct outcome, not "MISSING".
    try:
        verdict({"desktop_file": {"name": "yggterm"}}, KEYS)
        failures.append("absent-environment: NO REFUSAL — this path cannot fail")
    except NoPublishedEnvironment:
        pass

    # And the CLI's refusals are refusals, not answers. Their diagnostics go to
    # stderr; swallow it here so a passing self-test reads as one line.
    import contextlib
    import io
    import tempfile

    def cli(args: list[str]) -> int:
        with contextlib.redirect_stderr(io.StringIO()):
            return main(args)

    if cli([]) != 2:
        failures.append("usage: an empty argv did not exit 2")
    with tempfile.TemporaryDirectory() as scratch:
        report = Path(scratch) / "identity.json"
        report.write_text(json.dumps(_identity({POLICY_KEY: "hardware_gl_probed"})))
        if cli([str(report)]) != 2:
            failures.append(
                "no-keys: a call with zero keys was answered instead of refused"
            )
        blank = Path(scratch) / "blank.json"
        blank.write_text(json.dumps({"clients": []}))
        if cli([str(blank), *KEYS]) != 3:
            failures.append("absent-environment: CLI did not exit 3")
        if cli([str(Path(scratch) / "missing.json"), *KEYS]) != 2:
            failures.append("missing-file: CLI did not exit 2")

    for failure in failures:
        print(f"SELF-TEST FAILED: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("gl_ab_verify_env self-test: all checks fired")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
