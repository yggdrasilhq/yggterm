#!/usr/bin/env python3
"""No fleet verb reads a name that nothing in scope binds.

    python3 tests/test_no_verb_reads_a_name_nothing_binds.py

⛔⛔ THE HOLE THIS PINS, found 2026-08-22 while trying to make two different CLIs
greet each other. `ygg-deliver._reap_if_never_briefed` read `row_kind` as a free
name. `row_kind` is a LOCAL of `main()`, and a module-level function cannot see
another function's locals — so **every call raised `NameError`**, the interlock
that decides whether to destroy an un-briefed row was unreachable, and the caller
got a traceback and exit 1 where the contract promises 6.

⚠ WHY IT SHIPPED THROUGH A GREEN SUITE, WHICH IS THE PART WORTH KEEPING. Its two
callsites are the two DELIVERY-FAILURE paths — the timeout and the refused submit.
Nothing routine goes down them, so no test, no run and no live use had ever
executed the line. **Python resolves a global at call time, so an unbound name in
a branch nobody takes is indistinguishable from correct code until the day
something is already going wrong** — which is precisely the day it runs.

⇒ The generalisation, and the reason this is a scan rather than one more unit
test: the branches that most need to work are the ones hardest to reach, so their
correctness has to be established WITHOUT executing them.

⚖ WHY NOT `pyflakes`. It is not installed on these hosts and PEP 668 refuses the
install, so a gate depending on it would silently not run — the failure mode this
whole area keeps repeating. This is ~90 lines of stdlib `ast` and it ships with
the verbs it guards.

⛔ THE SCANNER'S OWN TWO MISTAKES ARE PINNED BELOW as polarity cases, because both
were made writing it and both produced a QUIET wrong answer:
  · descending into function bodies while collecting MODULE names makes every
    local look like a global, and the scan then finds nothing — it reported clean
    over the very defect it was written for;
  · not tracking the scope CHAIN makes every legitimate closure a finding, and 30
    false alarms teach a reader to stop believing it.
"""
import ast
import builtins
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
VERBS = os.path.dirname(HERE)

#: Names the interpreter injects into every module. Not builtins, not assigned.
IMPLICIT = {"__file__", "__name__", "__doc__", "__package__", "__spec__",
            "__loader__", "__builtins__"}


def module_names(tree):
    """Bindings visible at MODULE scope. ⛔ Never descends into a def or class —
    doing so is what made the first draft report clean over a real finding."""
    out = set()
    stack = list(tree.body)
    while stack:
        node = stack.pop()
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            out.add(node.name)
            continue                          # ⛔ its body is a DIFFERENT scope
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            for alias in node.names:
                out.add((alias.asname or alias.name).split(".")[0])
            continue
        for field in ("body", "orelse", "finalbody", "handlers"):
            stack.extend(getattr(node, field, None) or [])
        if isinstance(node, ast.ExceptHandler) and node.name:
            out.add(node.name)
        for inner in ast.walk(node):
            if isinstance(inner, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                out.add(inner.name)
            elif isinstance(inner, ast.Name) and isinstance(inner.ctx, ast.Store):
                out.add(inner.id)
    return out


def scope_binds(fn):
    """Every name bound anywhere inside `fn` — lambdas, comprehensions, `global`
    declarations and `except … as` included. Over-approximates deliberately: a
    missed finding costs one bug, a false alarm costs the gate's credibility."""
    out = set()
    for node in ast.walk(fn):
        if isinstance(node, ast.arguments):
            for arg in node.args + node.kwonlyargs + node.posonlyargs:
                out.add(arg.arg)
            if node.vararg:
                out.add(node.vararg.arg)
            if node.kwarg:
                out.add(node.kwarg.arg)
        elif isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            out.add(node.id)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            out.add(node.name)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            for alias in node.names:
                out.add((alias.asname or alias.name).split(".")[0])
        elif isinstance(node, ast.ExceptHandler) and node.name:
            out.add(node.name)
        elif isinstance(node, (ast.Global, ast.Nonlocal)):
            out.update(node.names)
        elif isinstance(node, ast.comprehension):
            for target in ast.walk(node.target):
                if isinstance(target, ast.Name):
                    out.add(target.id)
    return out


def own_loads(fn):
    """Name LOADS belonging to `fn` itself. A nested scope answers for its own."""
    out, stack = [], list(ast.iter_child_nodes(fn))
    while stack:
        node = stack.pop()
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef,
                             ast.Lambda, ast.ClassDef)):
            continue
        if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load):
            out.append(node)
        stack.extend(ast.iter_child_nodes(node))
    return out


def _descend(node, visible, path, hits):
    """⛔ A NESTED def CLOSES OVER its parent's locals; a MODULE-LEVEL one cannot.
    That difference is the entire discriminator, so the chain of enclosing scopes
    is carried down rather than rediscovered per function."""
    for child in ast.iter_child_nodes(node):
        if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
            inner = visible | scope_binds(child)
            for name in own_loads(child):
                if name.id not in inner:
                    hits.append((path, name.lineno, child.name, name.id))
            _descend(child, inner, path, hits)
        else:
            _descend(child, visible, path, hits)


def scan(paths):
    hits = []
    known = set(dir(builtins)) | IMPLICIT
    for path in paths:
        with open(path) as handle:
            tree = ast.parse(handle.read(), path)
        _descend(tree, module_names(tree) | known, path, hits)
    return hits


# ── The scanner must fail on the shapes that matter and stay quiet on the rest ──
# ⛔ A gate that has never been watched failing proves nothing. Every LEGAL case
#    below is real Python that runs; every BUG case raises NameError when called.
POLARITY = [
    ("a module-level function reading a caller's local — THE DEFECT", True,
     "def helper():\n    return leaked\ndef main():\n    leaked = 1\n    return helper()\n"),
    ("a function reading a sibling's local — the same defect, second shape", True,
     "def one():\n    value = 1\ndef two():\n    return value\n"),
    ("a nested function closing over its parent — LEGAL", False,
     "def main():\n    kept = 1\n    def helper():\n        return kept\n    return helper()\n"),
    ("a comprehension's own binder — LEGAL", False,
     "def main():\n    return [item for item in range(3)]\n"),
    ("a lambda's argument — LEGAL", False,
     "def main():\n    return (lambda value: value)(1)\n"),
    ("a module-level global — LEGAL", False,
     "SETTING = 1\ndef helper():\n    return SETTING\n"),
    ("a walrus binding used after it — LEGAL", False,
     "def main():\n    if (found := 2):\n        return found\n"),
    ("an `except … as` binder — LEGAL", False,
     "def main():\n    try:\n        pass\n    except OSError as err:\n        return err\n"),
    ("a global declared in one function and assigned in another — LEGAL", False,
     "def read():\n    global LATE\n    return LATE\ndef write():\n    global LATE\n    LATE = 1\n"),
    ("`__file__`, which the interpreter injects — LEGAL", False,
     "import os\ndef helper():\n    return os.path.dirname(__file__)\n"),
]


def main():
    failures = []

    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        for name, should_flag, source in POLARITY:
            probe = os.path.join(tmp, "probe.py")
            with open(probe, "w") as handle:
                handle.write(source)
            flagged = bool(scan([probe]))
            if flagged != should_flag:
                failures.append(
                    f"polarity: {name} — expected flagged={should_flag}, got {flagged}")

    verbs = sorted(os.path.join(VERBS, f) for f in os.listdir(VERBS)
                   if f.endswith(".py"))
    if len(verbs) < 5:
        failures.append(f"only {len(verbs)} verb(s) found under {VERBS} — "
                        "the scan is looking in the wrong place")
    for path, line, func, name in scan(verbs):
        failures.append(f"{os.path.basename(path)}:{line} in {func}() "
                        f"reads unbound name {name!r}")

    if failures:
        print("FAIL")
        for failure in failures:
            print(f"  {failure}")
        return 1
    print(f"PASS — {len(POLARITY)} polarity cases, {len(verbs)} verbs, no unbound reads")
    return 0


if __name__ == "__main__":
    sys.exit(main())
