#!/usr/bin/env python3
"""[s56-merge-key] The merge-key / Q4-matrix driver.

One GUI launch per trial (the launcher is tools/uxspeed/mergekey-rig.sh):

  main    exe-proof, settle, a rig-owned victim row, the warm-merge leg
          (expander rounds = >=100 warm builds of real uncached merges), the
          six Q4 first-switch-paint scenario classes with ZERO-mismatch DOM
          verification, a search-active segment driven by `server app search
          set` (reported SEPARATELY — its stored-rows input is a fresh Vec
          per call by design; sol Q4), and capture of the component_window
          spans (mean+max per window — max is the per-call spike
          instrument), the snapshot cache outcomes (the warm-build
          counter), the sidebar merge_rows_breakdown lines (key_ms) and the
          projection init counter (server app state).

  search  a GUI launched with YGGTERM_SEARCH_QUERY armed from boot — the
          boot-armed search cross-check.

  --analyze  merges the trial JSONs into the Q4 verdict table.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import statistics
import subprocess
import sys
import time


def now_ms() -> int:
    return int(time.time() * 1000)


class Rig:
    def __init__(self, home: str):
        self.home = home
        self.timeout_s = 25.0
        self._rows_before: list[dict] = []

    def verb(self, *argv: str, timeout: float | None = None) -> dict:
        cmd = ["yggterm", "server", "app", *argv]
        t0 = time.perf_counter()
        try:
            proc = subprocess.Popen(
                cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, start_new_session=True)
            out, err = proc.communicate(timeout=timeout or self.timeout_s)
        except subprocess.TimeoutExpired:
            try:
                proc.kill()
            except OSError:
                pass
            return {"ok": False, "error": "timeout", "wall_ms": 99999}
        reply = {"ok": proc.returncode == 0,
                 "wall_ms": int((time.perf_counter() - t0) * 1000)}
        try:
            reply["json"] = json.loads(out)
        except (json.JSONDecodeError, ValueError):
            reply["json"] = None
            reply["raw"] = ((out or "") + (err or ""))[-800:]
        if proc.returncode != 0:
            reply["error"] = (err or out or "")[-400:]
        return reply

    def dom_eval(self, js: str, timeout: float = 15, retries: int = 2) -> dict:
        last = {"error": "no attempt"}
        for _ in range(retries + 1):
            r = self.verb("dom-eval", js, timeout=timeout)
            data = (r.get("json") or {}).get("data")
            if isinstance(data, dict):
                if data.get("error") is not None:
                    last = {"error": str(data["error"])}
                else:
                    return {"result": data.get("result")}
            else:
                last = {"error": f"no data: {r.get('error') or str(r.get('raw', ''))[:200]}"}
            time.sleep(0.4)
        return last

    ROWS_JS = """const content = document.querySelector("[data-sidebar-content]");
let sc = content;
while (sc) { if (sc.scrollTop) sc.scrollTop = 0; sc = sc.parentElement; }
window.scrollTo(0, 0);
const rows = Array.from(document.querySelectorAll("[data-sidebar-row-path]"))
  .map(el => el.getAttribute("data-sidebar-row-path"));
dioxus.send({ rows, count: rows.length,
  sidebar_present: !!content,
  autohide: document.querySelector("[data-sidebar-auto-hide]")?.getAttribute("data-sidebar-auto-hide") ?? null });"""

    GROUPS_JS = """const ex = Array.from(document.querySelectorAll("[data-sidebar-group-expander]"));
const st = ex.map(b => ({ p: (b.closest("[data-sidebar-row-path]") || {}).getAttribute?.("data-sidebar-row-path"),
                          e: b.getAttribute("data-sidebar-group-expanded") }));
dioxus.send({ groups: st });"""

    def painted_rows(self) -> list[str]:
        r = self.dom_eval(self.ROWS_JS)
        if "result" in r:
            res = r["result"] or {}
            self.last_painted_meta = {"sidebar_present": res.get("sidebar_present"),
                                      "autohide": res.get("autohide")}
            return list(res.get("rows") or [])
        self.last_painted_meta = {"error": r.get("error")}
        return []

    def groups_state(self) -> list[dict]:
        r = self.dom_eval(self.GROUPS_JS)
        return list(((r.get("result") or {}).get("groups") or [])) if "result" in r else []

    def click_expander(self, path: str) -> bool:
        js = f"""
const btn = Array.from(document.querySelectorAll("[data-sidebar-group-expander]"))
  .find(b => (b.closest("[data-sidebar-row-path]") || {{}}).getAttribute?.("data-sidebar-row-path") === {json.dumps(path)});
if (!btn) {{ dioxus.send({{clicked: false}}); }} else {{ btn.click(); dioxus.send({{clicked: true}}); }}
"""
        r = self.dom_eval(js, timeout=10)
        return bool((r.get("result") or {}).get("clicked"))

    def live_rows(self) -> list[dict]:
        r = self.verb("rows", "--json")
        return ((r.get("json") or {}).get("data") or {}).get("rows") or []

    def spawn_row(self, title: str) -> dict:
        before = set(x.get("full_path", "") for x in self.live_rows())
        # small-world mode (HOME redirected): agent-CLI kinds cannot find
        # their store/auth under the scratch home — spawn a plain shell.
        kind = ["--kind", "shell"] if os.environ.get("S56_SMALL_WORLD") == "1" else []
        # small-world spawns land slow (the CLI launch re-scans the scratch
        # home) — raise BOTH the CLI's own app-control request timeout
        # (--timeout-ms, default 15s server-side) and the verb budget.
        small = os.environ.get("S56_SMALL_WORLD") == "1"
        r = self.verb("terminal", "new", "--title", title, "--cwd", "/tmp",
                      *kind, *(["--timeout-ms", "60000"] if small else []),
                      "--no-activate",
                      timeout=70.0 if small else None)
        if not r["ok"]:
            return {"error": f"spawn failed: {r.get('error')}"}
        data = (r.get("json") or {}).get("data") or {}
        path = ((data.get("launch") or {}).get("session_path")
                or data.get("session_path") or "")
        if not path:
            for row in self.live_rows():
                p = row.get("full_path", "")
                if p and p not in before:
                    path = p
                    break
        return {"path": path, "title": title}

    def remove_row(self, path: str) -> bool:
        return bool(self.verb("session", "remove", path)["ok"])

    def find_key(self, node, key):
        if isinstance(node, dict):
            for k, v in node.items():
                if k == key:
                    return v
                found = self.find_key(v, key)
                if found is not None:
                    return found
        elif isinstance(node, list):
            for v in node:
                found = self.find_key(v, key)
                if found is not None:
                    return found
        return None

    def projection_init_count(self) -> int | None:
        r = self.verb("state", timeout=30)
        data = (r.get("json") or {}).get("data") or {}
        v = self.find_key(data, "projection_init_count")
        return int(v) if v is not None else None

    def frame_epoch(self) -> int | None:
        r = self.verb("state", timeout=30)
        data = (r.get("json") or {}).get("data") or {}
        v = self.find_key(data, "root_render_count")
        return int(v) if v is not None else None

    def wait_frame_advance(self, baseline: int | None, cap_s: float = 4.0) -> int | None:
        """Wait until the app renders at least one NEW frame past `baseline`
        — a DOM read before that would serve a stale frame (the smoke-8
        remove flake: the webview's ack lag left the old row painted)."""
        t0 = time.time()
        last = baseline
        while time.time() - t0 < cap_s:
            cur = self.frame_epoch()
            if cur is not None and (baseline is None or cur > baseline):
                return cur
            time.sleep(0.4)
        return last

    def browser_row_count(self) -> int | None:
        r = self.verb("state", timeout=30)
        data = (r.get("json") or {}).get("data") or {}
        v = self.find_key(data, "row_count")
        return int(v) if v is not None else None

    def perf_lines(self, name: str) -> list[dict]:
        out = []
        for path in glob.glob(os.path.join(self.home, "**", "perf-telemetry.jsonl"),
                              recursive=True):
            for line in open(path, errors="replace"):
                try:
                    e = json.loads(line)
                except Exception:
                    continue
                if e.get("name") == name:
                    out.append(e)
        return out

    def component_windows(self) -> list[dict]:
        out = []
        for path in glob.glob(os.path.join(self.home, "**", "event-trace.jsonl"),
                              recursive=True):
            for line in open(path, errors="replace"):
                try:
                    e = json.loads(line)
                except Exception:
                    continue
                if e.get("name") == "component_window":
                    out.append(e)
        return out

    def exe_proof(self) -> str | None:
        for path in glob.glob(os.path.join(self.home, "**", "event-trace.jsonl"),
                              recursive=True):
            for line in open(path, errors="replace"):
                if "executable_path" in line:
                    try:
                        e = json.loads(line)
                    except Exception:
                        continue
                    ex = ((e.get("payload") or {}).get("executable_path")
                          or e.get("executable_path"))
                    if ex:
                        return ex
        return None


def wait_for(pred, timeout_s: float, poll: float = 0.5):
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        v = pred()
        if v:
            return v
        time.sleep(poll)
    return None


def expand_rounds(rig: Rig, want: bool, max_rounds: int = 4, max_clicks: int = 14):
    """Click differing expanders (bounded). ⛔ NEVER open the whole world: a
    20k-session tree expanded flat starves the webview's edit-ack plane
    (measured smoke7 — the GUI logged unacked batches and the DOM went
    dark). The paint matrix needs the live region + a few tree rows only."""
    clicked = 0
    for _ in range(max_rounds):
        st = rig.groups_state()
        targets = [g["p"] for g in st if g.get("p") and (g.get("e") == "true") != want]
        if not targets or clicked >= max_clicks:
            break
        for path in targets[:14]:
            if rig.click_expander(path):
                clicked += 1
        time.sleep(0.7)
    return clicked


def phase_main(home: str, out_path: str) -> int:
    rig = Rig(home)
    report: dict = {"phase": "main", "home": home, "started": now_ms()}
    report["exe_served"] = rig.exe_proof()

    time.sleep(17)  # two 8s refresh polls kick the first tree scan
    # READY GATE: wait for the daemon's own local_tree_scan to have run at
    # least TWICE (cold + one refresh) with a non-empty world. ⛔ row_count
    # is the VISIBLE-row count (a collapsed 20k tree reports ~7), never the
    # world size — a row_count gate is unsatisfiable and just burns minutes.
    t_gate = time.time()
    scans: list[dict] = []
    while time.time() - t_gate < 420:
        scans = [e for e in rig.perf_lines("local_tree_scan")]
        ok = [e for e in scans
              if (e.get("payload", {}).get("meta", {}) or {}).get("total_sessions", 0) > 0]
        if len(ok) >= 2:
            break
        time.sleep(6)
    # small-world mode: HOME was redirected, so the ready bar is the staged
    # count itself (S56_EXPECT_ROWS), not the real store's 20k.
    last_scan = (scans[-1].get("payload", {}).get("meta", {}) or {}) if scans else {}
    report["readiness_wait_s"] = round(time.time() - t_gate, 1)
    report["tree_scan_meta"] = last_scan
    report["scan_count"] = len(scans)
    rig._rows_before = rig.live_rows()
    report["live_rows_at_start"] = len(rig._rows_before)

    # ---- rig-owned victim row (the same-count swap needs a row we own)
    victim = rig.spawn_row(f"s56-victim-{now_ms() % 100000}")
    report["victim"] = victim
    time.sleep(1.0)

    # ---- warm leg: expand-all rounds then collapse-all rounds, repeated.
    # Every toggle changes expanded_paths => an UNCACHED merge over the whole
    # row world = a real build. The cache-outcome counters count them.
    warm_t0 = now_ms()
    clicks = 0
    for i in range(6):
        clicks += expand_rounds(rig, want=(i % 2 == 0))
        time.sleep(1.0)
    time.sleep(2.0)  # let the last merge + webview ack settle
    # END EXPANDED: the paint matrix needs the live region and the tree rows
    # inside the virtualized DOM window (a collapsed world paints ~2 rows).
    clicks += expand_rounds(rig, want=True)
    time.sleep(1.5)
    report["warm_clicks"] = clicks
    report["warm_t0_ms"] = warm_t0
    report["warm_t1_ms"] = now_ms()

    report["projection_init_count"] = rig.projection_init_count()

    # ---- paint matrix
    paints: list[dict] = []
    prefix = f"s56p{now_ms() % 100000}"
    base_rows = rig.painted_rows()
    report["painted_sample"] = base_rows[:12]

    def paint(name: str, expect_present: list[str], expect_absent: list[str]):
        epoch_base = rig.frame_epoch()
        def check():
            rows = rig.painted_rows()
            if all(p in rows for p in expect_present) and \
               all(p not in rows for p in expect_absent):
                return rows
            return None
        rows = wait_for(check, 16.0)
        if rows is None:
            # one freshness-guarded retry: advance a frame, re-read
            rig.wait_frame_advance(epoch_base)
            rows = wait_for(check, 8.0)
        paints.append({
            "scenario": name, "ok": rows is not None,
            "painted_count": len(rows) if rows is not None else None,
            "expected_present": expect_present, "expected_absent": expect_absent,
            "painted_sample": (rows or [])[:8],
            "read_meta": getattr(rig, "last_painted_meta", None),
        })

    # (a) add
    a = rig.spawn_row(f"{prefix}-add")
    if "error" in a or not a.get("path"):
        paints.append({"scenario": "add", "ok": False, "error": a})
    else:
        paint("add", [a["path"]], [])

    # (b) remove
    if a.get("path"):
        rig.remove_row(a["path"])
        paint("remove", [], [a["path"]])

    # (c) same-count swap: remove the victim, add a new row — count constant,
    # content moved. A count-keyed cache would serve the old world.
    c = rig.spawn_row(f"{prefix}-swap")
    if "error" not in c and c.get("path") and victim.get("path"):
        rig.remove_row(victim["path"])
        paint("same_count_swap", [c["path"]], [victim["path"]])
    else:
        paints.append({"scenario": "same_count_swap", "ok": False,
                       "error": {"c": c, "victim": victim}})

    # (d) short-ID collisions: two titles sharing a 40-char prefix
    longp = f"{prefix}-{'x' * 40}"
    d1 = rig.spawn_row(longp + "-aaaa")
    d2 = rig.spawn_row(longp + "-bbbb")
    if all("error" not in d and d.get("path") for d in (d1, d2)):
        paint("short_id_collision", [d1["path"], d2["path"]], [])
    else:
        paints.append({"scenario": "short_id_collision", "ok": False,
                       "error": [d1, d2]})

    # (e) collapse / expand one visible group
    st = [g for g in rig.groups_state() if g.get("e") == "true" and g.get("p")]
    if st:
        target = st[0]["p"]
        before = rig.painted_rows()
        if rig.click_expander(target):
            time.sleep(1.4)
            after = rig.painted_rows()
            paints.append({"scenario": "collapse", "ok": len(after) < len(before),
                           "group": target, "painted_before": len(before),
                           "painted_after": len(after)})
            if rig.click_expander(target):
                time.sleep(1.4)
                back = rig.painted_rows()
                paints.append({"scenario": "expand", "ok": len(back) == len(before),
                               "painted_back": len(back), "expected": len(before)})
            else:
                paints.append({"scenario": "expand", "ok": False, "error": "no expander"})
        else:
            paints.append({"scenario": "collapse", "ok": False, "error": "no expander"})
    else:
        paints.append({"scenario": "collapse", "ok": False, "error": "no expanded group"})

    # (f) search-active segment: verb-driven, ts-bounded, reported separately
    search_t0 = now_ms()
    r = rig.verb("search", "set", "proj", timeout=30)
    time.sleep(9.0)
    search_rows = rig.painted_rows()
    state = (rig.verb("state", timeout=30).get("json") or {}).get("data") or {}
    matches = rig.find_key(state, "search_sidebar_matches")
    match_count = len(matches) if isinstance(matches, list) else None
    rig.verb("search", "clear", timeout=30)
    time.sleep(1.5)
    # THE BAR: no painted row may lack the query (a stale unfiltered row is
    # the cache-key bug class). Painted-empty + state-matches>0 is reported
    # as-is, never silently folded into the search-inactive bars.
    non_matching = [p for p in search_rows if "proj" not in p.lower()
                    and not p.startswith("__")]
    paints.append({
        "scenario": "search_active",
        "ok": len(non_matching) == 0 and (match_count or 0) > 0,
        "painted_rows_search": len(search_rows),
        "state_match_count": match_count,
        "non_matching_painted": non_matching[:8],
        "painted_sample": search_rows[:8],
    })
    report["search_t0_ms"] = search_t0
    report["search_t1_ms"] = now_ms()
    report["search_painted"] = len(search_rows)

    report["paints"] = paints
    report["paints_total"] = len(paints)
    report["paints_ok"] = sum(1 for p in paints if p.get("ok"))

    # ---- capture spans + outcomes + breakdown
    wins = rig.component_windows()
    phases: dict = {}
    outcomes: list[dict] = []
    for w in wins:
        p = w.get("payload") or {}
        ts = w.get("ts_ms")
        for c in p.get("components") or []:
            name = c.get("component", "")
            if name.startswith(("snapshot", "app.snapshot", "app.merge")):
                phases.setdefault(name, []).append(
                    {"ts": ts, "renders": c.get("renders"),
                     "mean_ms": c.get("mean_ms"), "max_ms": c.get("max_ms")})
        sco = p.get("snapshot_cache_outcomes")
        if isinstance(sco, dict):
            outcomes.append({"ts": ts, "outcomes": sco})
    report["phases"] = phases
    report["component_windows"] = len(wins)
    report["cache_outcomes"] = outcomes
    bd = rig.perf_lines("merge_rows_breakdown")
    report["breakdown_count"] = len(bd)
    report["key_ms"] = [b.get("payload", {}).get("key_ms") for b in bd]
    report["merge_total_ms"] = [b.get("payload", {}).get("total_ms") for b in bd]
    report["cache_hits"] = sum(1 for b in bd if (b.get("payload") or {}).get("cache_hit"))
    report["finished"] = now_ms()

    with open(out_path, "w") as fh:
        json.dump(report, fh, indent=1)
    print(f"main: paints {report['paints_ok']}/{report['paints_total']} ok, "
          f"windows {len(wins)}, breakdown {len(bd)}, warm_clicks {clicks}, "
          f"init_count {report['projection_init_count']}")
    return 0


def phase_search(home: str, out_path: str) -> int:
    rig = Rig(home)
    report: dict = {"phase": "search", "home": home, "started": now_ms()}
    report["exe_served"] = rig.exe_proof()
    time.sleep(14)
    # a fresh GUI with no open rows shows the startpage, not the sidebar —
    # spawn one row so the sidebar surface exists, then read it
    rig.spawn_row(f"s56-searcharm-{now_ms() % 100000}")
    time.sleep(9.0)
    rows = wait_for(lambda: rig.painted_rows() or None, 12.0) or []
    report["painted_rows_search"] = len(rows)
    report["painted_sample"] = rows[:12]
    matching = [p for p in rows if "proj" in p.lower()]
    report["matching_rows"] = len(matching)
    report["all_rows_match_query"] = len(matching) == len(rows) and len(rows) > 0
    report["projection_init_count"] = rig.projection_init_count()
    with open(out_path, "w") as fh:
        json.dump(report, fh, indent=1)
    print(f"search: rows {len(rows)}, matching {len(matching)}")
    return 0


def analyze(out_dir: str) -> int:
    main = [json.load(open(p)) for p in sorted(glob.glob(os.path.join(out_dir, "trial-*.json")))
            if "-search" not in p]
    search = [json.load(open(p)) for p in glob.glob(os.path.join(out_dir, "trial-*-search.json"))]
    verdict: dict = {"main_trials": len(main), "search_trials": len(search)}

    # search-inactive slice: exclude the search-active segment's windows
    def slice_phases(r, lo, hi):
        out = {}
        for name, rows in (r.get("phases") or {}).items():
            out[name] = [x for x in rows if lo <= (x.get("ts") or 0) <= hi]
        return out

    warm_means, warm_maxes, merge_means, merge_maxes = [], [], [], []
    misses_total = 0
    for r in main:
        s = slice_phases(r, 0, r.get("search_t0_ms") or (1 << 62))
        for row in s.get("snapshot.p.browser", []):
            warm_means.append(row.get("mean_ms"))
            warm_maxes.append(row.get("max_ms"))
        for row in s.get("snapshot.p.merge", []):
            merge_means.append(row.get("mean_ms"))
            merge_maxes.append(row.get("max_ms"))
        for o in r.get("cache_outcomes") or []:
            if (o.get("ts") or 0) <= (r.get("search_t0_ms") or 0):
                for k, v in (o.get("outcomes") or {}).items():
                    if "miss" in str(k):
                        misses_total += int(v or 0)
    verdict["warm_builds_misses"] = misses_total
    if warm_means:
        verdict["browser_mean_ms"] = {
            "n": len(warm_means), "median": statistics.median(warm_means),
            "p95": sorted(warm_means)[int(len(warm_means) * 0.95)],
            "max": max(warm_means)}
        verdict["browser_max_worst"] = max(warm_maxes)
        verdict["every_warm_window_le_2ms"] = all(m <= 2.0 for m in warm_means)
    if merge_means:
        verdict["merge_mean_ms"] = {"n": len(merge_means),
                                    "median": statistics.median(merge_means)}
        spikes = sum(1 for mn, mx in zip(merge_means, merge_maxes)
                     if mn is not None and mx is not None and mx > 3 * max(mn, 0.01))
        verdict["merge_spiky_windows"] = spikes
    keys = [k for r in main for k in (r.get("key_ms") or []) if k is not None]
    if keys:
        verdict["key_ms"] = {"n": len(keys), "median": round(statistics.median(keys), 3),
                             "max": round(max(keys), 3)}
    verdict["projection_init"] = [{"home": r.get("home"),
                                   "count": r.get("projection_init_count")} for r in main]
    paints = [p for r in main for p in (r.get("paints") or [])]
    verdict["paints"] = {
        "total": len(paints),
        "ok": sum(1 for p in paints if p.get("ok")),
        "mismatches": [p for p in paints if not p.get("ok")],
    }
    verdict["search_arm"] = [
        {"rows": r.get("painted_rows_search"), "matching": r.get("matching_rows"),
         "all_match": r.get("all_rows_match_query")} for r in search]
    verdict["search_segments"] = [
        {"painted": r.get("search_painted")} for r in main]
    print(json.dumps(verdict, indent=1))
    with open(os.path.join(out_dir, "q4-verdict.json"), "w") as fh:
        json.dump(verdict, fh, indent=1)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--phase", choices=("main", "search", "analyze"), required=True)
    ap.add_argument("--home")
    ap.add_argument("--out")
    ap.add_argument("pos", nargs="*")
    args = ap.parse_args()
    if args.phase == "analyze":
        return analyze(args.pos[0] if args.pos else "/tmp/s56-rig")
    if not args.home or not args.out:
        print("--home and --out required", file=sys.stderr)
        return 2
    if args.phase == "main":
        return phase_main(args.home, args.out)
    return phase_search(args.home, args.out)


if __name__ == "__main__":
    raise SystemExit(main())
