#!/usr/bin/env python3
"""SPA-navigation decomposition bench: WebKitGTK vs Chromium vs ychrome.

The sibling of `webapp_launch_bench.py`. That one measures a LAUNCH — process
spawn to `load`. This one measures what happens *after* the app is up: an
in-page route change with no new process, no new surface and no top-level
navigation, which is the shape of clicking a chat in open-webui's sidebar.
`yggterm-webprobe` cannot answer this: its whole lifetime is one launch.

Three arms, one probe (`scripts/spa_nav_probe.js`), so a difference between
arms is a difference in the thing under test and not in the instrument:

  --arm webkit    plain WebKitGTK via PyGObject. Same profile jar and same
                  cache model as the app, with NONE of yggterm around it.
                  This is the control that decides "engine or us".
  --arm chromium  Chromium/Helium over CDP, same display, same window size.
                  localStorage is copied from the WebKit profile jar so both
                  engines start the app in the same state, not merely
                  authenticated. Credentials are passed to the engine and are
                  never printed, logged or written to the report.
  --arm ychrome   the product path: a web surface hosted by a running yggterm
                  GUI, driven through `server app web eval`.
                  ⚠ the poll that reads the result back runs JS on the page's
                  own main thread every 400 ms, so this arm carries an
                  observer cost the other two do not. Treat it as an
                  order-of-magnitude check, never as a precise third number.

  --arm layout    the controlled microbenchmark (webkit + chromium): a flat
                  document of N rows, timing a full style recalc + layout and
                  a `scrollWidth` read taken while layout is dirty. Answers
                  whether an engine's layout cost tracks node COUNT or
                  document COMPLEXITY.

A plan is JSON: {"steps":[{"chat":{"index":0},"note":"...","opts":{...}}]}
`opts` reaches `__spa.run` unchanged — see the probe for `observe`, `reflow`,
`stack_for`, `pre_css`, `dispatch`, `quiet_ms`.

⚠ `{"index":n}` means "the nth chat link the sidebar is showing RIGHT NOW",
never a fixed id: open-webui reorders the sidebar by recency as you navigate,
so a captured id list rots into link-not-found after a few switches.
"""
import argparse
import asyncio
import json
import os
import sqlite3
import subprocess
import sys
import time
import urllib.parse
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
PROBE = os.path.join(HERE, "spa_nav_probe.js")
LAYOUT_BENCH = os.path.join(HERE, "spa_nav_layout_bench.js")


def load_probe():
    with open(PROBE) as f:
        return f.read()


def read_localstorage(profile, origin_file):
    """Every key the WebKit profile holds for the origin.

    Copying only the auth token is not enough: the app's sidebar open/closed
    state lives here too, and a Chromium arm without it renders no sidebar and
    silently measures nothing. Values are returned for injection and are never
    logged — this project's rule is redact or assert on length.
    """
    path = os.path.join(profile, "localstorage", origin_file)
    conn = sqlite3.connect(path)
    out = {}
    for key, value in conn.execute("select key, value from ItemTable"):
        out[key] = value.decode("utf-16-le", "ignore") if isinstance(value, (bytes, bytearray)) else value
    return out


# --------------------------------------------------------------------------
# arm: plain WebKitGTK
# --------------------------------------------------------------------------
class WebKitArm:
    SENTINEL = object()

    def __init__(self, profile, width, height):
        import gi

        gi.require_version("Gtk", "3.0")
        gi.require_version("WebKit2", "4.1")
        from gi.repository import Gtk, WebKit2

        self.Gtk = Gtk
        self.WebKit2 = WebKit2
        manager = WebKit2.WebsiteDataManager(
            base_data_directory=profile, base_cache_directory=profile
        )
        self.ctx = WebKit2.WebContext.new_with_website_data_manager(manager)
        # Mirror of the app's own policy (configure_linux_webkit_memory_policy).
        self.ctx.set_cache_model(WebKit2.CacheModel.WEB_BROWSER)
        self.view = WebKit2.WebView.new_with_context(self.ctx)
        self.view.get_settings().set_enable_smooth_scrolling(True)
        self.win = Gtk.Window()
        self.win.set_default_size(width, height)
        self.win.add(self.view)
        self.win.show_all()
        self._res = self.SENTINEL
        self._loaded = False
        self.view.connect("load-changed", self._on_load)

    def _on_load(self, _view, event):
        if event == self.WebKit2.LoadEvent.FINISHED:
            self._loaded = True

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            while self.Gtk.events_pending():
                self.Gtk.main_iteration_do(False)
            time.sleep(0.002)

    def _cb(self, view, result, _u):
        try:
            value = view.evaluate_javascript_finish(result)
            self._res = value.to_json(0) if value is not None else "null"
        except Exception as exc:  # noqa: BLE001
            self._res = json.dumps({"__eval_error": str(exc)})

    def ev(self, js, timeout=120):
        self._res = self.SENTINEL
        self.view.evaluate_javascript(js, -1, None, None, None, self._cb, None)
        end = time.time() + timeout
        while self._res is self.SENTINEL and time.time() < end:
            while self.Gtk.events_pending():
                self.Gtk.main_iteration_do(False)
            time.sleep(0.002)
        if self._res is self.SENTINEL:
            raise RuntimeError("eval timeout")
        raw, self._res = self._res, self.SENTINEL
        try:
            return json.loads(raw)
        except Exception:
            return raw

    def load(self, url, timeout=120):
        self._loaded = False
        self.view.load_uri(url)
        end = time.time() + timeout
        while not self._loaded and time.time() < end:
            while self.Gtk.events_pending():
                self.Gtk.main_iteration_do(False)
            time.sleep(0.002)
        return self._loaded


def run_webkit(args, plan):
    width, height = (int(v) for v in args.size.split("x"))
    arm = WebKitArm(args.profile, width, height)
    t0 = time.time()
    ok = arm.load(args.url)
    arm.pump(args.boot_settle)
    report = {
        "label": args.label or "webkit-plain",
        "engine": "webkitgtk",
        "load_ok": ok,
        "boot_ms": round((time.time() - t0) * 1000, 1),
        "runs": [],
    }
    arm.ev(load_probe())
    report["supported"] = _maybe_json(arm.ev("JSON.stringify(__spa.supported())"))
    report["links"] = _maybe_json(arm.ev("JSON.stringify(__spa.links())"))
    for step in plan["steps"]:
        spec = json.dumps(step["chat"])
        opts = json.dumps(step.get("opts", {}))
        arm.ev("window.__r=null; __spa.run(%s,%s).then(function(r){window.__r=r;});" % (spec, opts))
        deadline = time.time() + step.get("timeout_s", 60)
        result = None
        while time.time() < deadline:
            arm.pump(0.15)
            got = arm.ev("window.__r===null?null:JSON.stringify(window.__r)")
            if got and got != "null":
                result = _maybe_json(got)
                break
        report["runs"].append({"chat": step["chat"], "note": step.get("note", ""), "result": result})
        arm.pump(step.get("cooldown_s", 1.0))
    return report


def _maybe_json(value):
    if isinstance(value, str) and value[:1] in "{[":
        try:
            return json.loads(value)
        except Exception:
            return value
    return value


# --------------------------------------------------------------------------
# arm: Chromium over CDP
# --------------------------------------------------------------------------
class CDP:
    def __init__(self, ws):
        self.ws = ws
        self.id = 0

    async def send(self, method, params=None, timeout=120):
        self.id += 1
        mid = self.id
        await self.ws.send(json.dumps({"id": mid, "method": method, "params": params or {}}))
        end = time.time() + timeout
        while time.time() < end:
            msg = json.loads(await asyncio.wait_for(self.ws.recv(), timeout=max(1, end - time.time())))
            if msg.get("id") == mid:
                if "error" in msg:
                    raise RuntimeError("%s: %s" % (method, json.dumps(msg["error"])[:200]))
                return msg.get("result", {})
        raise TimeoutError(method)

    async def ev(self, expr, await_promise=False, timeout=120):
        res = await self.send(
            "Runtime.evaluate",
            {"expression": expr, "returnByValue": True, "awaitPromise": await_promise, "userGesture": True},
            timeout=timeout,
        )
        if res.get("exceptionDetails"):
            raise RuntimeError("js: " + json.dumps(res["exceptionDetails"])[:300])
        return res.get("result", {}).get("value")


async def _chromium(args, plan):
    import websockets

    store = read_localstorage(args.profile, args.origin_file)
    width, height = (int(v) for v in args.size.split("x"))
    proc = subprocess.Popen(
        [
            args.chromium,
            "--remote-debugging-port=%d" % args.port,
            "--user-data-dir=" + args.user_data_dir,
            "--no-first-run",
            "--no-default-browser-check",
            # Without a window manager the window can read as occluded; this
            # removes a throttle rather than granting an advantage. Say so
            # rather than letting a reader wonder.
            "--disable-background-timer-throttling",
            "--window-size=%d,%d" % (width, height),
            "--remote-allow-origins=*",
            "about:blank",
        ],
        env=dict(os.environ, DISPLAY=args.display),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    ws_url = None
    for _ in range(120):
        try:
            with urllib.request.urlopen("http://127.0.0.1:%d/json/list" % args.port, timeout=2) as resp:
                for tab in json.load(resp):
                    if tab.get("type") == "page":
                        ws_url = tab["webSocketDebuggerUrl"]
                        break
            if ws_url:
                break
        except Exception:
            pass
        time.sleep(0.5)
    if not ws_url:
        proc.kill()
        raise SystemExit("chromium debug endpoint never came up")

    report = {"label": args.label or "chromium", "engine": "chromium", "runs": []}
    async with websockets.connect(ws_url, max_size=64 * 1024 * 1024, open_timeout=30) as ws:
        cdp = CDP(ws)
        await cdp.send("Page.enable")
        await cdp.send("Runtime.enable")
        report["ua"] = (await cdp.ev("navigator.userAgent"))[:120]
        t0 = time.time()
        await cdp.send("Page.navigate", {"url": args.url})
        await asyncio.sleep(args.boot_settle)
        await cdp.send(
            "Runtime.evaluate",
            {
                "expression": "(function(o){for(var k in o)localStorage.setItem(k,o[k]);"
                "return Object.keys(o).length;})(%s)" % json.dumps(store),
                "returnByValue": True,
            },
        )
        await cdp.send("Page.navigate", {"url": args.url})
        await asyncio.sleep(args.boot_settle)
        report["boot_ms"] = round((time.time() - t0) * 1000, 1)
        report["ls_keys"] = sorted(store.keys())  # names only, never values
        await cdp.ev(load_probe())
        report["supported"] = await cdp.ev("__spa.supported()")
        report["links"] = await cdp.ev("__spa.links()")
        for step in plan["steps"]:
            result = await cdp.ev(
                "__spa.run(%s,%s)" % (json.dumps(step["chat"]), json.dumps(step.get("opts", {}))),
                await_promise=True,
                timeout=step.get("timeout_s", 90),
            )
            report["runs"].append({"chat": step["chat"], "note": step.get("note", ""), "result": result})
            await asyncio.sleep(step.get("cooldown_s", 1.0))
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except Exception:
        proc.kill()
    return report


# --------------------------------------------------------------------------
# arm: ychrome (the product path)
# --------------------------------------------------------------------------
def _web_eval(gui, script, timeout=120):
    proc = subprocess.run(
        [gui, "server", "app", "web", "eval", "--stdin"],
        input=script,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if proc.returncode != 0:
        return {"__cli_error": (proc.stderr or proc.stdout)[:300]}
    try:
        payload = json.loads(proc.stdout)
    except Exception:
        return {"__cli_error": proc.stdout[:300]}
    if not payload.get("data", {}).get("accepted"):
        return {"__refused": payload.get("error") or payload.get("data")}
    return payload["data"].get("value")


def run_ychrome(args, plan):
    report = {"label": args.label or "ychrome", "engine": "webkitgtk-ychrome", "runs": []}
    report["install"] = _web_eval(args.gui, load_probe())
    report["supported"] = _maybe_json(_web_eval(args.gui, "JSON.stringify(__spa.supported())"))
    report["links"] = _maybe_json(_web_eval(args.gui, "JSON.stringify(__spa.links())"))
    for step in plan["steps"]:
        _web_eval(
            args.gui,
            "window.__r=null; __spa.run(%s,%s).then(function(r){window.__r=r;}); 'started'"
            % (json.dumps(step["chat"]), json.dumps(step.get("opts", {}))),
        )
        deadline = time.time() + step.get("timeout_s", 90)
        result = None
        while time.time() < deadline:
            time.sleep(0.4)
            got = _web_eval(args.gui, "window.__r===null?'PENDING':JSON.stringify(window.__r)")
            if isinstance(got, str) and got != "PENDING" and got[:1] == "{":
                result = json.loads(got)
                break
        report["runs"].append({"chat": step["chat"], "note": step.get("note", ""), "result": result})
        time.sleep(step.get("cooldown_s", 1.0))
    return report


# --------------------------------------------------------------------------
# arm: the controlled layout microbenchmark
# --------------------------------------------------------------------------
def run_layout(args):
    with open(LAYOUT_BENCH) as f:
        bench = f.read()
    out = {}
    width, height = (int(v) for v in args.size.split("x"))
    arm = WebKitArm(args.profile if os.path.isdir(args.profile) else "/tmp", width, height)
    arm.load("about:blank")
    arm.pump(1.0)
    out["webkit"] = _maybe_json(arm.ev(bench, timeout=600))

    async def chromium_bench():
        import websockets

        proc = subprocess.Popen(
            [
                args.chromium,
                "--remote-debugging-port=%d" % (args.port + 1),
                "--user-data-dir=" + args.user_data_dir,
                "--no-first-run",
                "--no-default-browser-check",
                "--window-size=%d,%d" % (width, height),
                "--remote-allow-origins=*",
                "about:blank",
            ],
            env=dict(os.environ, DISPLAY=args.display),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        ws_url = None
        for _ in range(120):
            try:
                with urllib.request.urlopen("http://127.0.0.1:%d/json/list" % (args.port + 1), timeout=2) as resp:
                    for tab in json.load(resp):
                        if tab.get("type") == "page":
                            ws_url = tab["webSocketDebuggerUrl"]
                            break
                if ws_url:
                    break
            except Exception:
                pass
            time.sleep(0.5)
        async with websockets.connect(ws_url, max_size=64 * 1024 * 1024, open_timeout=30) as ws:
            cdp = CDP(ws)
            await cdp.send("Runtime.enable")
            value = await cdp.ev(bench, timeout=600)
        proc.terminate()
        return _maybe_json(value)

    out["chromium"] = asyncio.run(chromium_bench())
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--arm", required=True, choices=["webkit", "chromium", "ychrome", "layout"])
    ap.add_argument("--plan", help="JSON plan (not used by --arm layout)")
    ap.add_argument("--out", required=True)
    ap.add_argument("--label")
    ap.add_argument("--url", required=True, help="the SPA to measure, e.g. an open-webui origin")
    ap.add_argument("--profile", default="", help="WebKit profile jar (auth + cache state)")
    ap.add_argument("--origin-file", default="",
                    help="WebKit localstorage filename for the origin; derived from --url when omitted")
    ap.add_argument("--user-data-dir", default="/tmp/spa-nav-chromium")
    ap.add_argument("--chromium", default="/usr/bin/helium")
    ap.add_argument("--gui", default="yggterm", help="GUI binary for --arm ychrome")
    ap.add_argument("--display", default=os.environ.get("DISPLAY", ":0"))
    ap.add_argument("--port", type=int, default=9333)
    ap.add_argument("--size", default="1600x1000")
    ap.add_argument("--boot-settle", type=float, default=6.0)
    args = ap.parse_args()

    if not args.origin_file:
        # WebKitGTK names these `<scheme>_<host>_<port>.localstorage`.
        parts = urllib.parse.urlsplit(args.url)
        args.origin_file = "%s_%s_0.localstorage" % (parts.scheme, parts.hostname)
    plan = json.load(open(args.plan)) if args.plan else {"steps": []}
    if args.arm == "webkit":
        report = run_webkit(args, plan)
    elif args.arm == "chromium":
        report = asyncio.run(_chromium(args, plan))
    elif args.arm == "ychrome":
        report = run_ychrome(args, plan)
    else:
        report = run_layout(args)
    with open(args.out, "w") as f:
        json.dump(report, f, indent=1)
    print(json.dumps({"wrote": args.out, "runs": len(report.get("runs", []))}))


if __name__ == "__main__":
    main()
