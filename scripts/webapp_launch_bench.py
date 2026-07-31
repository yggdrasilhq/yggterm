#!/usr/bin/env python3
"""Cold-vs-warm webapp launch benchmark: WebKitGTK (ours) vs Chromium (theirs).

The user's benchmark is literally "Chromium-based browsers launch webapps fast".
So the instrument has to be able to run BOTH engines, on the SAME page, under
the SAME display server, with the SAME cold/warm protocol, and read the SAME
numbers out of both. Anything less is comparing against a memory of Chromium.

How the two arms are made comparable
------------------------------------
Both engines are asked for their own `performance` entries, which are defined by
the same specs in both. The end-to-end figure is computed identically on each
side and contains no polling granularity:

    startup_ms  = timeOrigin      - spawn_epoch_ms      (exec -> navigation start)
    page_ms     = loadEventEnd                          (navigation start -> load)
    launch_ms   = startup_ms + page_ms

`spawn_epoch_ms` is the wall clock immediately before `exec`. `timeOrigin` is
the engine's own epoch for navigation start. Neither side gets to define
"loaded" differently.

The cold/warm protocol IS the measurement
-----------------------------------------
  cold  the engine profile directory is DELETED first. No HTTP cache, no
        compiled-code cache, no cookies. This is a first-ever visit.
  warm  the profile directory from the immediately preceding run is kept, and a
        NEW PROCESS is started. This is the case the user is complaining about:
        they have used the webapp before and are opening it again.

Warm is the interesting arm. Cold is the control that proves warm was warm.

Traps this script refuses to fall into
--------------------------------------
* It never reports a single run. `--repeat` runs are taken and the MEDIAN is
  reported, with min/max, because a first run after a build has page-cache
  effects that have nothing to do with the engine.
* It gives the engine time to FLUSH its disk cache before killing it
  (`--settle-ms`, default 2000). This one bit me: WebKit's network process
  writes cache records asynchronously after the page has loaded, so a probe that
  exits the instant `load` fires leaves the cache with nothing but its 8-byte
  salt file. The first version of this script defaulted to 0 and produced a
  confident "WebKitGTK never serves anything from cache" that was entirely an
  artifact of its own impatience.
* It refuses to call a warm run warm unless the page's own resource timings say
  no bytes came off the network. Note the asymmetry that forces: Chromium
  reports `transferSize == 0, decodedBodySize > 0` for a cache hit, while
  **WebKitGTK reports both as 0**, so `cached_bytes` reads 0 on WebKit even on a
  perfect cache hit. `network_bytes == 0` is therefore the portable test, and
  `cached_bytes` is only meaningful on the Chromium arm.
* It records the exact GL / cache-model environment into the output, because
  `docs/optimization-pass.md` has a standing list of confounders that each
  produced a confident wrong number on this codebase already.

Usage:
    scripts/webapp_launch_bench.py --fixture --repeat 5
    scripts/webapp_launch_bench.py --url https://www.khanacademy.org/ --repeat 3
    scripts/webapp_launch_bench.py --fixture --engines webkit
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import pathlib
import shutil
import signal
import statistics
import subprocess
import sys
import time
import urllib.request

REPO = pathlib.Path(__file__).resolve().parent.parent

COLLECT_JS = r"""
(function () {
  function num(v) { return (typeof v === 'number' && isFinite(v)) ? v : null; }
  var nav = performance.getEntriesByType('navigation')[0] || {};
  return JSON.stringify({
    ok: true,
    reason: 'cdp',
    timeOrigin: num(performance.timeOrigin),
    href: String(location.href),
    nav: {
      startTime: num(nav.startTime),
      fetchStart: num(nav.fetchStart),
      domainLookupStart: num(nav.domainLookupStart),
      domainLookupEnd: num(nav.domainLookupEnd),
      connectStart: num(nav.connectStart),
      connectEnd: num(nav.connectEnd),
      requestStart: num(nav.requestStart),
      responseStart: num(nav.responseStart),
      responseEnd: num(nav.responseEnd),
      domInteractive: num(nav.domInteractive),
      domContentLoadedEventStart: num(nav.domContentLoadedEventStart),
      domContentLoadedEventEnd: num(nav.domContentLoadedEventEnd),
      domComplete: num(nav.domComplete),
      loadEventEnd: num(nav.loadEventEnd),
      transferSize: num(nav.transferSize),
      decodedBodySize: num(nav.decodedBodySize)
    },
    paint: (performance.getEntriesByType('paint') || []).map(function (e) {
      return { name: e.name, startTime: num(e.startTime) };
    }),
    marks: (performance.getEntriesByType('mark') || []).map(function (e) {
      return { name: e.name, startTime: num(e.startTime) };
    }),
    measures: (performance.getEntriesByType('measure') || []).map(function (e) {
      return { name: e.name, startTime: num(e.startTime), duration: num(e.duration) };
    }),
    resources: (performance.getEntriesByType('resource') || []).map(function (e) {
      return {
        name: e.name, initiatorType: e.initiatorType,
        startTime: num(e.startTime), responseStart: num(e.responseStart),
        responseEnd: num(e.responseEnd), duration: num(e.duration),
        transferSize: num(e.transferSize),
        encodedBodySize: num(e.encodedBodySize),
        decodedBodySize: num(e.decodedBodySize)
      };
    })
  });
})()
"""


# --------------------------------------------------------------------------
# shared analysis
# --------------------------------------------------------------------------


def summarize(page: dict, spawn_epoch_ms: float) -> dict:
    """Reduce one engine's raw `performance` dump to the comparable phases."""
    if not page or not page.get("ok"):
        return {"ok": False, "error": (page or {}).get("error", "no page report")}
    nav = page.get("nav") or {}
    origin = page.get("timeOrigin")
    resources = page.get("resources") or []

    cached = [r for r in resources if (r.get("transferSize") or 0) == 0 and (r.get("decodedBodySize") or 0) > 0]
    networked = [r for r in resources if (r.get("transferSize") or 0) > 0]
    cached_bytes = sum(r.get("decodedBodySize") or 0 for r in cached)
    network_bytes = sum(r.get("transferSize") or 0 for r in networked)

    scripts = [r for r in resources if r.get("initiatorType") == "script"]
    script_bytes = sum(r.get("decodedBodySize") or 0 for r in scripts)

    # The fixture brackets each bundle in a `performance.measure` named s<i>.
    # Sum of those is fetch+parse+compile+execute for all bundles; on a run
    # where they were cache hits it is parse+compile+execute alone.
    bundle_measures = [m for m in (page.get("measures") or []) if (m.get("name") or "").startswith("s") and (m.get("name") or "")[1:].isdigit()]
    bundle_ms = sum(m.get("duration") or 0 for m in bundle_measures)

    paints = {p["name"]: p["startTime"] for p in (page.get("paint") or [])}

    startup_ms = (origin - spawn_epoch_ms) if origin else None
    load_end = nav.get("loadEventEnd")
    return {
        "ok": True,
        "startup_ms": round(startup_ms, 1) if startup_ms is not None else None,
        "ttfb_ms": _r(nav.get("responseStart")),
        "response_end_ms": _r(nav.get("responseEnd")),
        "first_paint_ms": _r(paints.get("first-paint")),
        "first_contentful_paint_ms": _r(paints.get("first-contentful-paint")),
        "dom_interactive_ms": _r(nav.get("domInteractive")),
        "dom_content_loaded_ms": _r(nav.get("domContentLoadedEventEnd")),
        "load_event_end_ms": _r(load_end),
        "launch_ms": round(startup_ms + load_end, 1) if (startup_ms is not None and load_end) else None,
        "script_exec_ms": round(bundle_ms, 1) if bundle_measures else None,
        "resources": len(resources),
        "cached_resources": len(cached),
        "cached_bytes": cached_bytes,
        "network_bytes": network_bytes,
        "script_bytes": script_bytes,
    }


def _r(v):
    return round(v, 1) if isinstance(v, (int, float)) else None


# --------------------------------------------------------------------------
# WebKitGTK arm (our engine, via yggterm-webprobe)
# --------------------------------------------------------------------------


def run_webkit(probe: pathlib.Path, url: str, profile: pathlib.Path, timeout_ms: int, env: dict, settle_ms: int) -> dict:
    spawn_epoch_ms = time.time() * 1000.0
    proc = subprocess.run(
        [
            str(probe),
            "--url", url,
            "--profile", str(profile),
            "--timeout-ms", str(timeout_ms),
            # Without this the network process is killed mid-flush and the next
            # run finds an empty cache. See the module docstring.
            "--settle-ms", str(settle_ms),
        ],
        capture_output=True,
        text=True,
        env=env,
        timeout=(timeout_ms + settle_ms) / 1000.0 + 30,
    )
    if proc.returncode != 0:
        return {"ok": False, "error": f"probe exit {proc.returncode}: {proc.stderr[-400:]}"}
    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError as err:
        return {"ok": False, "error": f"probe output not JSON: {err}: {proc.stdout[:200]}"}
    summary = summarize(report.get("page") or {}, spawn_epoch_ms)
    summary["phases_ms"] = report.get("phases_ms")
    summary["processes"] = report.get("processes")
    summary["disk_cache"] = report.get("disk_cache")
    summary["env"] = report.get("env")
    return summary


# --------------------------------------------------------------------------
# Chromium arm (Helium), via the DevTools protocol
# --------------------------------------------------------------------------


async def _cdp_collect(ws_url: str, timeout_s: float) -> dict:
    import websockets

    async with websockets.connect(ws_url, max_size=64 * 1024 * 1024) as ws:
        msg_id = 0

        async def call(method, params=None):
            nonlocal msg_id
            msg_id += 1
            mine = msg_id
            await ws.send(json.dumps({"id": mine, "method": method, "params": params or {}}))
            while True:
                raw = json.loads(await ws.recv())
                if raw.get("id") == mine:
                    if "error" in raw:
                        raise RuntimeError(raw["error"])
                    return raw.get("result", {})

        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            res = await call("Runtime.evaluate", {"expression": "document.readyState", "returnByValue": True})
            if res.get("result", {}).get("value") == "complete":
                break
            await asyncio.sleep(0.05)
        # One turn of the task queue so loadEventEnd is populated, exactly like
        # the WebKit probe's `setTimeout(..., 0)` after `load`.
        await asyncio.sleep(0.05)
        res = await call("Runtime.evaluate", {"expression": COLLECT_JS, "returnByValue": True})
        return json.loads(res["result"]["value"])


def run_chromium(binary: str, url: str, profile: pathlib.Path, timeout_ms: int, env: dict, settle_ms: int) -> dict:
    profile.mkdir(parents=True, exist_ok=True)
    port_file = profile / "DevToolsActivePort"
    if port_file.exists():
        port_file.unlink()
    args = [
        binary,
        f"--user-data-dir={profile}",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-backgrounding-occluded-windows",
        "--disable-renderer-backgrounding",
        "--no-service-autorun",
        "--password-store=basic",
        url,
    ]
    spawn_epoch_ms = time.time() * 1000.0
    proc = subprocess.Popen(args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    try:
        deadline = time.monotonic() + timeout_ms / 1000.0
        port = None
        while time.monotonic() < deadline:
            if port_file.exists():
                try:
                    port = int(port_file.read_text().splitlines()[0])
                    break
                except (ValueError, IndexError):
                    pass
            time.sleep(0.02)
        if port is None:
            return {"ok": False, "error": "chromium never wrote DevToolsActivePort"}

        target = None
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=2) as resp:
                    targets = json.loads(resp.read())
                page_targets = [t for t in targets if t.get("type") == "page" and t.get("webSocketDebuggerUrl")]
                if page_targets:
                    target = page_targets[0]
                    break
            except Exception:
                pass
            time.sleep(0.02)
        if target is None:
            return {"ok": False, "error": "no chromium page target appeared"}

        page = asyncio.run(_cdp_collect(target["webSocketDebuggerUrl"], timeout_ms / 1000.0))
        # Same flush grace as the WebKit arm. Both engines write their disk
        # cache asynchronously; killing either one early makes the NEXT run
        # measure a cold cache while calling itself warm.
        time.sleep(settle_ms / 1000.0)
        return summarize(page, spawn_epoch_ms)
    finally:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            proc.terminate()
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                proc.kill()


# --------------------------------------------------------------------------
# driver
# --------------------------------------------------------------------------


def median_of(runs: list[dict], key: str):
    values = [r[key] for r in runs if r.get("ok") and isinstance(r.get(key), (int, float))]
    if not values:
        return None
    return round(statistics.median(values), 1)


def spread_of(runs: list[dict], key: str):
    values = [r[key] for r in runs if r.get("ok") and isinstance(r.get(key), (int, float))]
    if not values:
        return None
    return (round(min(values), 1), round(max(values), 1))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", help="page to launch; omit with --fixture")
    parser.add_argument("--fixture", action="store_true", help="serve and measure the deterministic heavy fixture")
    parser.add_argument("--copies", type=int, default=8, help="fixture JS bundle count")
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--timeout-ms", type=int, default=60000)
    parser.add_argument(
        "--settle-ms",
        type=int,
        default=2000,
        help="grace after load before killing the engine, so its disk cache flushes",
    )
    parser.add_argument("--engines", default="webkit,chromium")
    parser.add_argument("--chromium", default=None, help="chromium/helium binary (auto-detected)")
    parser.add_argument("--probe", default=None, help="path to yggterm-webprobe")
    parser.add_argument("--workdir", default=None)
    parser.add_argument("--out", default=None, help="write the full JSON result here")
    args = parser.parse_args()

    if not args.url and not args.fixture:
        parser.error("pass --url or --fixture")

    engines = [e.strip() for e in args.engines.split(",") if e.strip()]
    probe = pathlib.Path(args.probe) if args.probe else REPO / "target" / "release" / "yggterm-webprobe"
    if "webkit" in engines and not probe.exists():
        env_target = os.environ.get("CARGO_TARGET_DIR")
        alt = pathlib.Path(env_target) / "release" / "yggterm-webprobe" if env_target else None
        if alt and alt.exists():
            probe = alt
        else:
            print(f"missing probe binary at {probe}; build with:\n  cargo build -p yggterm-webprobe --release", file=sys.stderr)
            return 2

    chromium = args.chromium
    if "chromium" in engines and not chromium:
        for candidate in ("helium", "chromium", "chromium-browser", "google-chrome", "brave-browser"):
            found = shutil.which(candidate)
            if found:
                chromium = found
                break
    if "chromium" in engines and not chromium:
        print("NO CHROMIUM-CLASS BROWSER FOUND — the comparison arm is unavailable on this host.", file=sys.stderr)
        print("Reporting the WebKit arm alone. Do not compare it against a remembered number.", file=sys.stderr)
        engines = [e for e in engines if e != "chromium"]

    workdir = pathlib.Path(args.workdir) if args.workdir else pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / "ygg-launch-bench"
    workdir.mkdir(parents=True, exist_ok=True)

    fixture_proc = None
    url = args.url
    if args.fixture:
        fixture_proc = subprocess.Popen(
            [sys.executable, str(REPO / "scripts" / "webapp_launch_fixture.py"), "--port", "0", "--copies", str(args.copies), "--print-port"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        port = int(fixture_proc.stdout.readline().strip())
        url = f"http://127.0.0.1:{port}/"

    env = dict(os.environ)
    result = {
        "url": url,
        "repeat": args.repeat,
        "fixture": bool(args.fixture),
        "fixture_copies": args.copies if args.fixture else None,
        "display": env.get("DISPLAY"),
        "chromium_binary": chromium,
        "chromium_version": None,
        "arms": {},
    }
    if chromium:
        try:
            result["chromium_version"] = subprocess.run([chromium, "--version"], capture_output=True, text=True, timeout=20).stdout.strip()
        except Exception:
            pass

    try:
        for engine in engines:
            for arm in ("cold", "warm"):
                runs = []
                for i in range(args.repeat):
                    profile = workdir / f"{engine}-profile"
                    if arm == "cold":
                        shutil.rmtree(profile, ignore_errors=True)
                    elif i == 0:
                        # Guarantee the warm arm is warm: one priming run into a
                        # profile that starts empty, so warm measures a SECOND
                        # visit rather than whatever the cold arm happened to
                        # leave behind.
                        shutil.rmtree(profile, ignore_errors=True)
                        if engine == "webkit":
                            run_webkit(probe, url, profile, args.timeout_ms, env, args.settle_ms)
                        else:
                            run_chromium(chromium, url, profile, args.timeout_ms, env, args.settle_ms)
                    profile.mkdir(parents=True, exist_ok=True)
                    if engine == "webkit":
                        runs.append(run_webkit(probe, url, profile, args.timeout_ms, env, args.settle_ms))
                    else:
                        runs.append(run_chromium(chromium, url, profile, args.timeout_ms, env, args.settle_ms))
                result["arms"][f"{engine}-{arm}"] = {
                    "runs": runs,
                    "median": {
                        key: median_of(runs, key)
                        for key in (
                            "startup_ms", "ttfb_ms", "response_end_ms", "first_paint_ms",
                            "first_contentful_paint_ms", "dom_interactive_ms",
                            "dom_content_loaded_ms", "load_event_end_ms", "launch_ms",
                            "script_exec_ms", "cached_bytes", "network_bytes",
                            "cached_resources", "resources",
                        )
                    },
                    "launch_ms_range": spread_of(runs, "launch_ms"),
                }
    finally:
        if fixture_proc:
            fixture_proc.terminate()

    # ---- report -----------------------------------------------------------
    print()
    print(f"url                {url}")
    print(f"repeat             {args.repeat} (median reported)")
    if result["chromium_version"]:
        print(f"chromium arm       {result['chromium_version']}")
    else:
        print("chromium arm       ABSENT on this host — no like-for-like comparison")
    print()
    cols = [
        ("startup_ms", "exec->nav"),
        ("ttfb_ms", "TTFB"),
        ("first_contentful_paint_ms", "FCP"),
        ("dom_interactive_ms", "interactive"),
        ("script_exec_ms", "JS exec"),
        ("load_event_end_ms", "load"),
        ("launch_ms", "LAUNCH"),
        ("cached_bytes", "cachedB"),
        ("network_bytes", "netB"),
    ]
    header = f"{'arm':<18}" + "".join(f"{label:>13}" for _, label in cols)
    print(header)
    print("-" * len(header))
    for name, data in result["arms"].items():
        row = f"{name:<18}"
        for key, _ in cols:
            value = data["median"].get(key)
            row += f"{'-' if value is None else value:>13}"
        print(row)
    print()
    for name, data in result["arms"].items():
        bad = [r for r in data["runs"] if not r.get("ok")]
        if bad:
            print(f"!! {name}: {len(bad)}/{len(data['runs'])} runs failed: {bad[0].get('error')}")
        if name.endswith("warm") and (data["median"].get("network_bytes") or 0) > 1024:
            print(
                f"!! {name}: pulled {data['median']['network_bytes']} bytes off the network — "
                "this arm was NOT warm, its number means nothing"
            )

    if args.out:
        pathlib.Path(args.out).write_text(json.dumps(result, indent=2))
        print(f"\nfull result -> {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
