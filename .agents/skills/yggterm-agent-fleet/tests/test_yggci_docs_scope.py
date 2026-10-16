#!/usr/bin/env python3
"""[docs-scope] the skip decision is conservative: all-markdown skips,
anything else (or nothing) builds.

    python3 tests/test_yggci_docs_scope.py
"""
import importlib.util
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SKILL = HERE.parent
FAILURES = []


def check(name, ok, detail=""):
    if ok:
        print(f"  ok  {name}")
    else:
        FAILURES.append(name)
        print(f"  ⛔ {name}  {detail}")


def load_module(tmp):
    sys.path.insert(0, str(SKILL))
    spec = importlib.util.spec_from_file_location("ygg_ci_under_test", SKILL / "ygg-ci.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    mod.LOGPATH = tmp / "ci-test.log"
    mod.CI_STATE = tmp
    mod._STDOUT_IS_LOG = False
    return mod


def main():
    tmp = Path(tempfile.mkdtemp(prefix="yggci-docs-scope-test-"))
    mod = load_module(tmp)
    f = mod._is_docs_scope

    check("pure queue markdown skips",
          f(["docs/pending-bugs.md", "docs/archive/pending-bugs-closed-2026-10-16.md"]))
    check("a single root readme skips", f(["README.md"]))
    check("nested skill markdown skips", f([".agents/skills/x/SKILL.md"]))
    check("one rust file builds", not f(["docs/pending-bugs.md", "crates/yggterm/src/lib.rs"]))
    check("one toml builds", not f(["Cargo.toml"]))
    check("one python file builds", not f(["tools/probe.py"]))
    check("empty diff builds (fail-open)", not f([]))
    check("whitespace-only entries are ignored",
          f(["docs/a.md", "  ", "docs/b.md"]))
    check("a markdown-lookalike extension does not skip", not f(["docs/notes.mdx"]))
    check("markdown in a hidden dir with code sibling builds",
          not f([".github/workflows/ci.yml", "docs/a.md"]))

    print()
    if FAILURES:
        print(f"{len(FAILURES)} FAILED")
        return 1
    print("All docs-scope tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
