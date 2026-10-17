#!/usr/bin/env python3
"""[auth rotation 2026-10-17] the composer transport is fault-tolerant of codex
auth/limit failures: classify → ygg-auth rotate → retry once → persist means
all accounts limited (the caller's backoff is the designed wait).

Run: python3 tests/test_dream_auth_rotation.py [path-to-ygg-memory.py]
"""
import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT = HERE.parent / "ygg-memory.py"
failures = []


def check(ok, label):
    print(f"  {'ok' if ok else 'FAIL'}: {label}")
    if not ok:
        failures.append(label)


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_authrot", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeCompleted:
    def __init__(self, returncode, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def with_fake_transport(module, codex_results, rotate_log=None):
    """Patch subprocess.run so codex calls follow codex_results (a list popped
    per call) and ygg-auth rotate calls append to rotate_log and succeed."""
    state = {"codex_calls": 0}
    real_run = subprocess.run

    def fake_run(cmd, *a, **kw):
        joined = " ".join(str(c) for c in cmd)
        if "ygg-auth" in joined or "fake-auth" in joined:
            if rotate_log is not None:
                rotate_log.append(joined)
            return FakeCompleted(0, stdout="rotated to ananyakundu_gmail.com\n")
        state["codex_calls"] += 1
        idx = min(state["codex_calls"] - 1, len(codex_results) - 1)
        return codex_results[idx]

    module.subprocess.run = fake_run
    return state


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT)

    print("[1] the classifier")
    check(module._codex_limit_auth_failure(
        "Error: You've reached your usage limit until 9pm"), "usage-limit text classified")
    check(module._codex_limit_auth_failure(
        "429 Too Many Requests"), "429 classified")
    check(module._codex_limit_auth_failure(
        "token expired; please log in again"), "auth-expiry classified")
    check(not module._codex_limit_auth_failure(
        "dream: composer exited 1: …panic: assertion failed"), "ordinary failure NOT classified")
    check(not module._codex_limit_auth_failure(""), "empty tail NOT classified")

    print("[2] rotation + retry recovers")
    os.environ["YGG_DREAM_AUTH_ROTATE"] = "1"
    with tempfile.NamedTemporaryFile("w", prefix="fake-auth-", suffix=".py", delete=False) as fh:
        fh.write("# fake auth runner\n")
        fake_auth = fh.name
    os.environ["YGG_AUTH_RUNNER"] = fake_auth
    rotate_log = []
    state = with_fake_transport(
        module,
        [FakeCompleted(1, stderr="Error: rate limit reached, try later"),
         FakeCompleted(0, stdout="=== NOW.MD === ok")],
        rotate_log,
    )
    out = module._codex_exec("codex", "gpt-6.1-sol", "high", "prompt")
    check(out.startswith("=== NOW.MD"), "retry after rotation succeeded")
    check(state["codex_calls"] == 2, "codex called exactly twice (1 fail + 1 retry)")
    check(len(rotate_log) == 1 and "rotate" in rotate_log[0],
          "ygg-auth rotate invoked exactly once")

    print("[3] all accounts limited → persistent failure, rotation recorded")
    rotate_log.clear()
    state2 = with_fake_transport(
        module,
        [FakeCompleted(1, stderr="usage limit reached"),
         FakeCompleted(1, stderr="usage limit reached")],
        rotate_log,
    )
    try:
        module._codex_exec("codex", "gpt-6.1-sol", "high", "prompt")
        check(False, "all-limited must raise")
    except SystemExit as e:
        msg = str(e)
        check("PERSISTED after account rotation" in msg
              and "backoff is the wait" in msg, "persisted message names the wait")
    check(state2["codex_calls"] == 2 and len(rotate_log) == 1,
          "exactly one rotation attempt in the all-limited case")

    print("[4] non-limit failure never rotates")
    rotate_log.clear()
    state3 = with_fake_transport(
        module, [FakeCompleted(1, stderr="panic: something broke")], rotate_log)
    try:
        module._codex_exec("codex", "gpt-6.1-sol", "high", "prompt")
        check(False, "non-limit failure must raise")
    except SystemExit as e:
        check("composer exited 1" in str(e), "plain failure message preserved")
    check(state3["codex_calls"] == 1 and not rotate_log,
          "no rotation and no retry on non-limit failures")

    print("[5] kill switch restores raw behavior")
    os.environ["YGG_DREAM_AUTH_ROTATE"] = "0"
    rotate_log.clear()
    state4 = with_fake_transport(
        module, [FakeCompleted(1, stderr="usage limit reached")], rotate_log)
    try:
        module._codex_exec("codex", "gpt-6.1-sol", "high", "prompt")
        check(False, "must still raise under the kill switch")
    except SystemExit:
        pass
    check(state4["codex_calls"] == 1 and not rotate_log,
          "YGG_DREAM_AUTH_ROTATE=0: single call, no rotation")

    os.environ.pop("YGG_DREAM_AUTH_ROTATE", None)
    os.environ.pop("YGG_AUTH_RUNNER", None)
    os.unlink(fake_auth)

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\nauth-rotation locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
