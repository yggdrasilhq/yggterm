#!/usr/bin/env python3
"""[11.240] the cache verdict is HONEST about why a compile missed the
cache: ok serves requests, idle compiles nothing, bypassed means the
wrapper/binary wire is absent, non-cacheable means sccache refused the
call shapes (crate-type/incremental). The old single-cause STALE
claimed a broken wire for both shapes.

    python3 tests/test_yggci_cache_verdict.py
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
    tmp = Path(tempfile.mkdtemp(prefix="yggci-11240-verdict-test-"))
    mod = load_module(tmp)

    pre = {"requests_executed": 100, "hits": 50, "misses": 50}
    served = {"requests_executed": 103, "hits": 52, "misses": 51}

    mod._cache_wire_probe = lambda: {"wrapper": True, "binary": True}

    v = mod._cache_verdict(pre, served, "   Compiling yggterm v0.1.0")
    check("served requests are ok", v["verdict"] == "ok" and v["requests"] == 3)

    v = mod._cache_verdict(pre, pre, "")
    check("nothing compiled is idle", v["verdict"] == "idle")

    v = mod._cache_verdict(pre, pre, "   Compiling yggterm v0.1.0")
    check("wire present + compiling + zero requests = non-cacheable",
          v["verdict"] == "non-cacheable")
    check("the verdict carries the wire readout",
          v.get("wire", {}).get("wrapper") is True)

    mod._cache_wire_probe = lambda: {"wrapper": False, "binary": False}
    v = mod._cache_verdict(pre, pre, "   Compiling yggterm v0.1.0")
    check("wire absent + compiling + zero requests = bypassed",
          v["verdict"] == "bypassed")
    check("bypassed verdict names the missing wire",
          v.get("wire", {}).get("wrapper") is False)

    v = mod._cache_verdict(None, served, "x")
    check("unreadable stats are absent", v["verdict"] == "absent")

    check("the false STALE name is gone from the verdict space",
          all("STALE" not in str(mod._cache_verdict(pre, pre, o).get("verdict"))
              for o in ("", "   Compiling x")))

    print()
    if FAILURES:
        print(f"{len(FAILURES)} FAILED")
        return 1
    print("All [11.240] cache-verdict tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
