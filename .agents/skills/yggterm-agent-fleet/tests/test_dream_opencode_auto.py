#!/usr/bin/env python3
"""[owner law 2026-10-17] dream agents run with FULL permissions: the opencode
transport spawns `opencode run --auto ...` by default (YGG_OPENCODE_AUTO=0
restores approval-gated runs) and the codex transport keeps
--dangerously-bypass-approvals-and-sandbox.

Run: python3 tests/test_dream_opencode_auto.py [path-to-ygg-memory.py]
"""
import importlib.util
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT = HERE.parent / "ygg-memory.py"
failures = []


def check(ok, label):
    print(f"  {'ok' if ok else 'FAIL'}: {label}")
    if not ok:
        failures.append(label)


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_ocauto", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeCompleted:
    def __init__(self, returncode, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def capture_cmds(monkey_module, results):
    """Patch subprocess.run; record every cmd list, return canned results."""
    seen = []
    real_run = subprocess.run

    def fake_run(cmd, *a, **kw):
        seen.append([str(c) for c in cmd])
        result = results.pop(0) if results else FakeCompleted(0, "ok")
        if isinstance(result, Exception):
            raise result
        return result

    monkey_module.subprocess.run = fake_run
    return seen, lambda: setattr(monkey_module.subprocess, "run", real_run)


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT)
    env_key = "YGG_OPENCODE_AUTO"

    print("opencode --auto default:")
    seen, restore = capture_cmds(module, [FakeCompleted(0, "reply")])
    old = os.environ.pop(env_key, None)
    try:
        out = module._opencode_exec("opencode/glm-5.3-flash-free", "compose")
        check(len(seen) == 1, "one opencode invocation")
        cmd = seen[0]
        check(cmd[1:3] == ["run", "--auto"], f"--auto follows run: {cmd[:3]}")
        check("-m" in cmd and "compose" in cmd, "model + prompt preserved")
        check(out == "reply", "reply passes through")
    finally:
        restore()
        if old is not None:
            os.environ[env_key] = old

    print("YGG_OPENCODE_AUTO=0 kill switch:")
    seen, restore = capture_cmds(module, [FakeCompleted(0, "reply")])
    os.environ[env_key] = "0"
    try:
        module._opencode_exec("m", "compose")
        check(len(seen) == 1 and "--auto" not in seen[0],
              "no --auto when the kill switch is set")
    finally:
        restore()
        os.environ.pop(env_key, None)

    print("codex keeps its bypass flag:")
    seen, restore = capture_cmds(module, [FakeCompleted(0, "reply")])
    try:
        module._codex_exec("codex", "gpt-6.1-sol", "high", "compose")
        cmd = " ".join(seen[0])
        check("--dangerously-bypass-approvals-and-sandbox" in cmd,
              "codex spawn carries --dangerously-bypass-approvals-and-sandbox")
    finally:
        restore()

    print()
    if failures:
        print(f"FAILED ({len(failures)}): {failures}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
