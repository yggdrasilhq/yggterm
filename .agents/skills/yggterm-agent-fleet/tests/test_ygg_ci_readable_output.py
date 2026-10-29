#!/usr/bin/env python3
"""The ci verbs read as text by default; --json is the only structured plane.

    python3 tests/test_ygg_ci_readable_output.py [--ygg-ci <path to ygg-ci.py>]

The defects pinned here: the operator-facing
verbs dumped raw dicts into human output (status conflicts, why's
disarm/hold/quarantine, events printed a truncated JSON body per line), and
two verbs leaked human text into the --json plane — `cache --json` printed
readable lines BEFORE the JSON (stdout unparseable) and `events --json` on a
missing event file printed human text and no JSON at all. The JSON payload
shapes are pinned unchanged here; only the human plane and the two stdout
leaks are new behavior. Every fixture is invented; nothing here touches real
config, subscriptions, builds, or hosts — all state runs under an isolated
YGGTERM_HOME in ~/.yggterm/scratchpad.
"""
import argparse
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

NOW = int(time.time())
SCRATCH_ROOT = Path.home() / ".yggterm" / "scratchpad" / "test-ygg-ci-readable"

EVENTS = [
    {"event_id": "ci-fixture000001", "at": NOW - 600, "project": "yggterm",
     "kind": "build_failed", "host": "buildhost",
     "lanes": ["lane/example/one"],
     "log_tail": "error: fixture compile failure in example_app"},
    {"event_id": "ci-fixture000002", "at": NOW - 120, "project": "yggterm",
     "kind": "built_and_pushed", "host": "buildhost", "sha": "a" * 40,
     "lanes": ["lane/example/one"], "deploy_ok": True},
]

BUILD = {
    "id": "example--20261029-080000--abcd12345678", "project": "example",
    "at": NOW - 840, "host": "buildhost", "main": "c" * 40, "sha": "a" * 40,
    "lanes": [{"lane": "lane/example/one", "tip": "b" * 40}],
    "conflicts": [{"lane": "lane/example/two", "tip": "d" * 40,
                   "reason": "conflict",
                   "stderr": "CONFLICT (content): Merge conflict in fixture.txt"}],
    "status": "build-failed", "build_ok": False,
    "sccache": {"verdict": "ok", "requests": 42, "hits": 41, "misses": 1,
                "hit_rate": 97.6},
}

QUARANTINE = {"yggterm": {"lane/example/stuck": {"tip": "b" * 40,
                                                 "expires": NOW + 600}}}


def load_ci(path, home):
    """Import ygg-ci.py with YGGTERM_HOME pointed at the fixture home."""
    spec = importlib.util.spec_from_file_location("ygg_ci_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    old = os.environ.get("YGGTERM_HOME")
    os.environ["YGGTERM_HOME"] = str(home)
    # script execution puts the script's dir on sys.path (for ygg_appctl);
    # importlib loading from tests/ does not — add and remove it here.
    sys.path.insert(0, str(path.resolve().parent))
    argv, sys.argv = sys.argv, [str(path), "status"]
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.argv = argv
        sys.path.remove(str(path.resolve().parent))
        if old is None:
            os.environ.pop("YGGTERM_HOME", None)
        else:
            os.environ["YGGTERM_HOME"] = old
    return mod


def run_ci(ygg_ci, home, *args, timeout=120):
    env = dict(os.environ)
    env["YGGTERM_HOME"] = str(home)
    return subprocess.run([sys.executable, str(ygg_ci), *args],
                          capture_output=True, text=True, env=env,
                          timeout=timeout)


def new_home(run_root, name):
    home = run_root / name
    (home / "relay" / "ci" / "subs").mkdir(parents=True)
    (home / "relay" / "ci" / "builds").mkdir(parents=True)
    return home


def seed_events(home, events):
    p = home / "relay" / "ci" / "events.jsonl"
    p.write_text("".join(json.dumps(e) + "\n" for e in events))


def seed_config(home, repo, host=None):
    cfg = {"projects": {"yggterm": {
        "repo": str(repo), "remote": "origin", "main_branch": "main",
        "integration_branch": "integration", "build": "cargo build --release",
        "deploy": "", "host": host or "fixture-host", "interval": 300}}}
    (home / "relay" / "ci" / "ci.json").write_text(json.dumps(cfg))


def seed_sub(mod, home, lane, repo):
    rec = {"lane": lane, "project": "yggterm", "repo": str(repo),
           "tip_at_enlist": None, "enlisted_at": NOW, "by": "fixturesess",
           "want": "next"}
    p = mod._sub_path("yggterm", lane)
    p.write_text(json.dumps(rec, indent=2))
    return p


def seed_build(home, rec):
    (home / "relay" / "ci" / "builds" / f"{rec['id']}.json").write_text(
        json.dumps(rec, indent=2))


def seed_quarantine(home, q):
    (home / "relay" / "ci" / "quarantine.json").write_text(json.dumps(q))


def make_repo(run_root):
    """A real local git repo on main, nothing remote, nothing pushed."""
    repo = run_root / "repo-example"
    repo.mkdir(parents=True)

    def g(*a):
        subprocess.run(["git", *a], cwd=repo, check=True,
                       capture_output=True)
    g("init", "-b", "main")
    g("config", "user.email", "fixture@example.test")
    g("config", "user.name", "Fixture")
    (repo / "README.md").write_text("fixture\n")
    g("add", "-A")
    g("commit", "-m", "fixture base")
    return repo


def make_repo_with_remote(run_root):
    """main + lane/example/dry pushed to a local bare remote (file transport)."""
    remote = run_root / "remote-example.git"
    subprocess.run(["git", "init", "--bare", str(remote)], check=True,
                   capture_output=True)
    repo = run_root / "work-example"
    repo.mkdir(parents=True)

    def g(*a):
        subprocess.run(["git", *a], cwd=repo, check=True,
                       capture_output=True)
    g("init", "-b", "main")
    g("config", "user.email", "fixture@example.test")
    g("config", "user.name", "Fixture")
    (repo / "README.md").write_text("fixture\n")
    g("add", "-A")
    g("commit", "-m", "fixture base")
    g("remote", "add", "origin", str(remote))
    g("push", "-q", "origin", "main")
    g("checkout", "-b", "lane/example/dry")
    (repo / "FEATURE.md").write_text("fixture feature\n")
    g("add", "-A")
    g("commit", "-m", "fixture feature")
    g("push", "-q", "origin", "lane/example/dry")
    g("checkout", "main")
    return repo


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ygg-ci",
                    default=Path(__file__).resolve().parent.parent / "ygg-ci.py")
    args = ap.parse_args()
    ygg_ci = Path(args.ygg_ci).resolve()
    failures = []
    run_root = SCRATCH_ROOT / time.strftime("%Y%m%d-%H%M%S")
    run_root.mkdir(parents=True, exist_ok=True)
    try:
        # ── pure render helpers ──────────────────────────────────────────
        m = load_ci(ygg_ci, new_home(run_root, "pure-home"))
        if "clock ahead 2m" not in m._build_line({"at": NOW + 120, "status": "built"}, NOW):
            failures.append("future source timestamps must not render as fresh zero-age")
        if "2m ago" not in m._build_line({"at": NOW - 120, "status": "built"}, NOW):
            failures.append("normal timestamp age suffix")
        cases = [(m._age(None), "?"), (m._age(45), "45s"), (m._age(720), "12m"),
                 (m._age(11500), "3.2h"), (m._age(5 * 86400), "5.0d")]
        for got, want in cases:
            if got != want:
                failures.append(f"_age({got!r}) != {want!r}")
        line = m._event_line(EVENTS[0])
        if "build FAILED" not in line or "lane/example/one" not in line:
            failures.append(f"event line lost words: {line!r}")
        if "{" in line:
            failures.append(f"event line carries JSON: {line!r}")
        line = m._event_line(EVENTS[1])
        if "built and pushed" not in line or "deployed" not in line:
            failures.append(f"success event line lost words: {line!r}")
        line = m._build_line(BUILD, now=NOW)
        if "⛔" not in line or "build-failed" not in line or "1 conflict(s)" not in line:
            failures.append(f"build line lost status/conflicts: {line!r}")
        q = m._quarantine_lines(QUARANTINE["yggterm"], now=NOW)[0]
        if "until" not in q or "left" not in q or "lane/example/stuck" not in q:
            failures.append(f"quarantine line lost expiry words: {q!r}")
        q2 = m._quarantine_lines({"lane/example/stuck":
                                  {"tip": "b" * 40, "expires": NOW - 600}},
                                 now=NOW)[0]
        if "EXPIRED" not in q2:
            failures.append(f"expired quarantine not marked: {q2!r}")
        # legacy plain-tip quarantine entries normalize
        q3 = m._quarantine_lines({"lane/legacy": "e" * 40}, now=NOW)[0]
        if "lane/legacy" not in q3 or "{" in q3:
            failures.append(f"legacy quarantine entry mishandled: {q3!r}")

        # ── status: human plane ─────────────────────────────────────────
        home = new_home(run_root, "status-home")
        seed_build(home, BUILD)
        r = run_ci(ygg_ci, home, "status", "--project", "example")
        if r.returncode != 0:
            failures.append(f"status human rc={r.returncode} stderr={r.stderr[-200:]}")
        for want in ("fleet single build plane", "watcher:", "subscriptions:",
                     "build-failed", "lane/example/two", "next:"):
            if want not in r.stdout:
                failures.append(f"status human missing {want!r}:\n{r.stdout}")
        if "conflicts: [" in r.stdout or '{"' in r.stdout:
            failures.append(f"status human carries a dump:\n{r.stdout}")
        # status: --json parseable with unchanged shape
        r = run_ci(ygg_ci, home, "status", "--project", "example", "--json")
        try:
            info = json.loads(r.stdout)
            if info.get("last_build", {}).get("status") != "build-failed":
                failures.append("status --json lost last_build status")
            for k in ("host", "watcher_alive", "subscribed", "log"):
                if k not in info:
                    failures.append(f"status --json lost key {k}")
        except Exception as e:
            failures.append(f"status --json unparseable: {e}\n{r.stdout[:200]}")
        # status: cold state (no heartbeat, no builds, watcher down)
        home = new_home(run_root, "status-cold-home")
        r = run_ci(ygg_ci, home, "status")
        if "not running" not in r.stdout or "none yet" not in r.stdout \
                or "subscribe" not in r.stdout:
            failures.append(f"status cold state lost words:\n{r.stdout}")

        # ── events ──────────────────────────────────────────────────────
        home = new_home(run_root, "events-home")
        seed_events(home, EVENTS)
        r = run_ci(ygg_ci, home, "events")
        if r.returncode != 0:
            failures.append(f"events human rc={r.returncode}")
        if "built and pushed" not in r.stdout or "build FAILED" not in r.stdout:
            failures.append(f"events human lost kind words:\n{r.stdout}")
        if '"kind"' in r.stdout or "event_id" in r.stdout:
            failures.append(f"events human carries JSON:\n{r.stdout}")
        if "events.jsonl" not in r.stdout:
            failures.append("events human lost provenance footer")
        r = run_ci(ygg_ci, home, "events", "--json")
        try:
            ev = json.loads(r.stdout)
            if {e["kind"] for e in ev} != {"build_failed", "built_and_pushed"}:
                failures.append(f"events --json changed data: {ev}")
        except Exception as e:
            failures.append(f"events --json unparseable: {e}\n{r.stdout[:200]}")
        # ⛔ the leak: events --json on a MISSING file must be parseable []
        home = new_home(run_root, "events-empty-home")
        r = run_ci(ygg_ci, home, "events", "--json")
        if r.stdout.strip() != "[]":
            failures.append(f"events --json on missing file not '[]': {r.stdout[:120]!r}")
        r = run_ci(ygg_ci, home, "events")
        if "no events yet" not in r.stdout:
            failures.append(f"events human missing-file state lost:\n{r.stdout}")
        # filtered to zero
        seed_events(home, EVENTS)
        r = run_ci(ygg_ci, home, "events", "--kind", "never_happened_fixture")
        if "no events match the filters" not in r.stdout:
            failures.append(f"events zero-match state lost:\n{r.stdout}")

        # ── cache: --json purity (the old code printed human lines first) ─
        home = new_home(run_root, "cache-home")
        seed_build(home, BUILD)
        r = run_ci(ygg_ci, home, "cache", "--host", __import__("platform").node().split(".")[0], "--json")
        try:
            rep = json.loads(r.stdout)
            if "hosts" not in rep or "recent_builds" not in rep:
                failures.append(f"cache --json lost keys: {list(rep)}")
            if not any(b.get("id") == BUILD["id"] for b in rep["recent_builds"]):
                failures.append("cache --json lost the seeded build verdict")
        except Exception as e:
            failures.append(f"cache --json unparseable: {e}\n{r.stdout[:300]}")
        r = run_ci(ygg_ci, home, "cache", "--host",
                   __import__("platform").node().split(".")[0])
        if "compile-cache health" not in r.stdout or "next:" not in r.stdout:
            failures.append(f"cache human lost headings:\n{r.stdout}")
        if '{"' in r.stdout:
            failures.append(f"cache human carries JSON:\n{r.stdout}")
        if BUILD["id"] not in r.stdout:
            failures.append("cache human lost the recent-build verdict row")

        # ── list ────────────────────────────────────────────────────────
        home = new_home(run_root, "list-home")
        r = run_ci(ygg_ci, home, "list")
        if "no ci subscriptions" not in r.stdout or "subscribe" not in r.stdout:
            failures.append(f"list empty state lost:\n{r.stdout}")
        r = run_ci(ygg_ci, home, "list", "--json")
        if json.loads(r.stdout) != {"subs": []}:
            failures.append("list --json empty shape changed")
        mod2 = load_ci(ygg_ci, home)
        seed_sub(mod2, home, "lane/example/dry", run_root)
        r = run_ci(ygg_ci, home, "list")
        if "lane/example/dry" not in r.stdout or "want=next" not in r.stdout \
                or "ci subscriptions" not in r.stdout:
            failures.append(f"list rows lost:\n{r.stdout}")
        r = run_ci(ygg_ci, home, "list", "--json")
        if len(json.loads(r.stdout)["subs"]) != 1:
            failures.append("list --json lost the seeded sub")

        # ── config ──────────────────────────────────────────────────────
        home = new_home(run_root, "config-home")
        seed_config(home, run_root / "repo-example", host="fixture-host")
        r = run_ci(ygg_ci, home, "config", "--project", "yggterm")
        if "build recipe" not in r.stdout or "gates:" not in r.stdout \
                or "fixture-host" not in r.stdout:
            failures.append(f"config human lost fields:\n{r.stdout}")
        r = run_ci(ygg_ci, home, "config", "--project", "yggterm", "--json")
        cfg = json.loads(r.stdout)
        if cfg.get("yggterm", {}).get("repo") != str(run_root / "repo-example"):
            failures.append("config --json changed data")
        r = run_ci(ygg_ci, home, "config")
        if "ci config" not in r.stdout or "yggterm" not in r.stdout:
            failures.append(f"config all-projects human lost:\n{r.stdout}")

        # ── editor round-1 repairs, pinned as regressions ────────────────
        # F2: an EXPIRING disarm logs through disarm_state(); status --json
        # must keep that diagnostic on stderr and stdout parseable.
        home = new_home(run_root, "status-disarm-home")
        seed_build(home, BUILD)
        (home / "relay" / "ci" / "ci.disarmed").write_text(json.dumps(
            {"since": NOW - 100, "until": NOW - 10, "hours": 0.01,
             "note": "fixture expired disarm"}))
        r = run_ci(ygg_ci, home, "status", "--project", "example", "--json")
        try:
            json.loads(r.stdout)
        except Exception as e:
            failures.append(f"status --json polluted by disarm-expiry log: {e}\n"
                            f"{r.stdout[:200]!r}")
        if "disarm EXPIRED" not in r.stderr:
            failures.append(f"disarm-expiry diagnostic lost from stderr:\n{r.stderr[-200:]!r}")
        # F1: the clean early-return carries conflicts as an int count; the
        # tick summary must render it without iterating it.
        import contextlib
        import io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            m._tick_human_summary("yggterm", {"status": "clean", "merged": 0,
                                              "conflicts": 1})
        out = buf.getvalue()
        if "nothing new to integrate (merged 0, conflicts 1)" not in out:
            failures.append(f"clean-with-conflict-count summary wrong:\n{out!r}")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            m._tick_human_summary("yggterm", {"status": "clean", "swept": [
                {"lane": "lane/example/gone", "disposition": "expired-abandoned"}]})
        if "swept from subscriptions: lane/example/gone (expired abandoned)" not in buf.getvalue():
            failures.append(f"swept subs not summarized:\n{buf.getvalue()!r}")

        # ── why: quarantine words, missing-repo exit ────────────────────
        home = new_home(run_root, "why-home")
        repo = make_repo(run_root)
        seed_config(home, repo)
        seed_quarantine(home, QUARANTINE)
        r = run_ci(ygg_ci, home, "why")
        if r.returncode != 0:
            failures.append(f"why rc={r.returncode} stderr={r.stderr[-300:]}")
        for want in ("quarantined (1)", "lane/example/stuck", "until", "main checkout"):
            if want not in r.stdout:
                failures.append(f"why missing {want!r}:\n{r.stdout}")
        if "{'" in r.stdout or '{"' in r.stdout:
            failures.append(f"why carries a dump:\n{r.stdout}")
        home = new_home(run_root, "why-norepo-home")
        seed_config(home, run_root / "no-such-repo-fixture")
        r = run_ci(ygg_ci, home, "why")
        if r.returncode != 2 or "fix the repo path" not in r.stdout:
            failures.append(f"why missing-repo contract changed: rc={r.returncode}")

        # ── tick --dry-run: rehearsal wording, subs untouched ───────────
        home = new_home(run_root, "tick-home")
        repo = make_repo_with_remote(run_root)
        seed_config(home, repo)
        mod3 = load_ci(ygg_ci, home)
        sub_p = seed_sub(mod3, home, "lane/example/dry", repo)
        r = run_ci(ygg_ci, home, "tick", "--dry-run")
        if r.returncode != 0:
            failures.append(f"tick --dry-run rc={r.returncode} stderr={r.stderr[-300:]}")
        for want in ("rehearsal only", "per-lane marks above", "nothing was",
                     "next: ygg ci tick --project yggterm"):
            if want not in r.stdout:
                failures.append(f"tick --dry-run missing {want!r}:\n{r.stdout}")
        if "'{" in r.stdout or '{"' in r.stdout:
            failures.append(f"tick human carries a dump:\n{r.stdout}")
        if not sub_p.exists():
            failures.append("tick --dry-run MUTATED the subscription (sweep consumed it)")
        r = run_ci(ygg_ci, home, "tick", "--dry-run", "--json")
        try:
            res = json.loads(r.stdout)
            status = res["yggterm"]["status"]
            if status not in ("dry", "clean"):
                failures.append(f"tick --json unexpected status {status}")
        except Exception as e:
            failures.append(f"tick --json unparseable: {e}\n{r.stdout[:200]}")
    finally:
        shutil.rmtree(run_root, ignore_errors=True)

    if failures:
        print(f"⛔ {len(failures)} failure(s):")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("✅ ygg-ci readable-output contract holds: text reads, --json parses, "
          "no dumps, states loud, subscriptions untouched by dry-run")
    return 0


if __name__ == "__main__":
    sys.exit(main())
