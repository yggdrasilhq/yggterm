#!/usr/bin/env python3
"""[free-first composer chain 2026-10-17] opencode free transports precede codex;
per-transport limit classification advances the chain; codex keeps rotation.

Run: python3 tests/test_dream_composer_chain.py [path-to-ygg-memory.py]
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
    spec = importlib.util.spec_from_file_location("ygg_memory_chain", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeCompleted:
    def __init__(self, returncode, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT)

    print("[1] the chain default")
    chain = module._composer_chain()
    check(chain[0] == ("opencode", "opencode/glm-5.3-flash-free"),
          "first transport = opencode GLM-5.3-Flash free")
    check(chain[1] == ("opencode", "opencode/deepseek-v4.1-flash-free"),
          "second transport = opencode DeepSeek 4.1 Flash free")
    check(chain[2][0] == "codex", "codex last")
    check(module._opencode_limit_failure("daily limit reached for the free tier"),
          "opencode limit text classified")
    check(not module._opencode_limit_failure("panic: real bug"),
          "ordinary opencode failure NOT classified")

    print("[2] free quota exhausted mid-chain falls through to codex")
    calls = []

    def fake_opencode(model, prompt, timeout=900):
        calls.append(("opencode", model))
        # the whole free quota is exhausted for the day
        raise SystemExit("dream: opencode composer exited 1: …daily limit reached")

    def fake_codex(bin_, model, effort, prompt, timeout=900):
        calls.append(("codex", model))
        return "=== NOW.MD === via codex"

    real_oc, real_codex = module._opencode_exec, module._codex_exec
    module._opencode_exec = fake_opencode
    module._codex_exec = fake_codex
    out, used = module._composer_exec("prompt", "high", "codex", "gpt-6.1-sol")
    check(used == "codex/gpt-6.1-sol" and str(out).endswith("via codex"),
          "falls to codex when both free transports are limited")
    check([c[0] for c in calls] == ["opencode", "opencode", "codex"],
          "tried glm, then deepseek, then codex")

    print("[3] first free transport healthy = codex never touched")
    calls.clear()
    module._opencode_exec = lambda m, p, timeout=900: f"ok via {m}"
    out, used = module._composer_exec("prompt", "high", "codex", "gpt-6.1-sol")
    check(used == "opencode/opencode/glm-5.3-flash-free" and calls == [],
          "codex untouched when GLM free works")

    print("[4] ordinary opencode failure raises (no silent fallthrough)")
    module._opencode_exec = lambda m, p, timeout=900: (_ for _ in ()).throw(
        SystemExit("dream: opencode composer exited 1: …panic: real bug"))
    try:
        module._composer_exec("prompt", "high", "codex", "gpt-6.1-sol")
        check(False, "must raise on non-limit failure")
    except SystemExit as e:
        check("panic: real bug" in str(e), "ordinary failure propagates")

    print("[5] codex-only chain override")
    os.environ["YGG_DREAM_COMPOSERS"] = "codex"
    module._opencode_exec = fake_opencode
    calls.clear()
    out, used = module._composer_exec("prompt", "high", "codex", "gpt-6.1-sol")
    check(used == "codex/gpt-6.1-sol" and calls == [("codex", "gpt-6.1-sol")],
          "YGG_DREAM_COMPOSERS=codex restores codex-only")
    os.environ.pop("YGG_DREAM_COMPOSERS", None)

    module._opencode_exec = real_oc
    module._codex_exec = real_codex

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\ncomposer-chain locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
