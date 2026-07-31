#!/usr/bin/env python3
"""A deterministic "heavy webapp" for launch-latency measurement.

Why a fixture instead of just pointing at youtube.com: a real site changes its
payload between the two arms of an A/B, serves different bytes to different
engines (User-Agent sniffing), and adds seconds of network variance on top of
the effect being measured. Every one of those turns a cold-vs-warm delta into
noise. This server holds the payload FIXED so the only thing that differs
between cold and warm is what the engine kept.

What it serves:

  GET /                     the app shell.  `Cache-Control: no-cache`
  GET /js/bundle-<i>.js     one real, minified, production JS bundle
                            (this repo's vendored xterm.js / addon-webgl.js).
                            `Cache-Control: public, max-age=31536000, immutable`

That header split is not arbitrary — it is the modern webapp convention
(revalidated shell, immutable hashed assets) and it is exactly the shape
Chromium's V8 code cache is built to exploit. Measuring against anything else
would be measuring a strawman.

Each bundle is prefixed with a unique comment so the copies are distinct SOURCE
STRINGS as well as distinct URLs. Without that, an engine whose compilation
cache is keyed on source text (V8's is) would compile copy 0 and hand the same
compiled code to copies 1..N, and the fixture would silently measure one bundle
while claiming to measure N.

Every script tag is bracketed by `performance.mark`s. Classic scripts execute
synchronously in document order, so mark(end) - mark(start) is
fetch + parse + compile + execute for that one bundle. On a run where the
resource was a cache hit (`transferSize == 0`), the fetch term is gone and the
interval is parse + compile + execute — which is the number the V8 code cache
attacks and the number JavaScriptCore has to pay every time.

Usage:
    scripts/webapp_launch_fixture.py --port 8099 --copies 8
    scripts/webapp_launch_fixture.py --port 0 --copies 8 --print-port
"""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPO = pathlib.Path(__file__).resolve().parent.parent
# Real, minified, production bundles that ship in this repo. Using real code
# matters: synthetic generated JS does not exercise a parser or a bytecode
# generator the way shipped library code does.
SOURCES = [
    REPO / "assets" / "xterm" / "xterm.js",
    REPO / "assets" / "xterm" / "addon-webgl.js",
]


def build_bundles(copies: int) -> list[bytes]:
    payloads = []
    for source in SOURCES:
        if not source.exists():
            print(f"fixture: missing {source}", file=sys.stderr)
            sys.exit(2)
        payloads.append(source.read_bytes())
    out = []
    for index in range(copies):
        base = payloads[index % len(payloads)]
        # Unique prefix => unique source string => no cross-copy compilation
        # cache sharing in either engine.
        out.append(f"/* ygg-launch-fixture copy {index} */\n".encode() + base)
    return out


INDEX_TEMPLATE = """<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>ygg launch fixture</title>
<style>
  body {{ background:#262a33; color:#e5e5e5; font:14px system-ui, sans-serif; margin:0; }}
  #app {{ padding:24px; }}
  .box {{ height:120px; background:#2f3542; margin:8px 0; border-radius:6px; }}
</style>
</head>
<body>
<script>performance.mark('shell-start');</script>
<div id="app"><h1>ygg launch fixture</h1>{boxes}</div>
{scripts}
<script>
  performance.mark('scripts-done');
  performance.measure('scripts-total', 'shell-start', 'scripts-done');
  // A real webapp does DOM work before it is usable. Keep it small and
  // deterministic: this is a launch probe, not a rendering benchmark.
  (function () {{
    var app = document.getElementById('app');
    for (var i = 0; i < 200; i++) {{
      var el = document.createElement('div');
      el.className = 'row';
      el.textContent = 'row ' + i;
      app.appendChild(el);
    }}
    performance.mark('app-ready');
    performance.measure('app-ready-total', 'shell-start', 'app-ready');
  }})();
</script>
</body>
</html>
"""


class Handler(BaseHTTPRequestHandler):
    bundles: list[bytes] = []
    index: bytes = b""

    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):  # noqa: D102 - quiet by design
        pass

    def _send(self, body: bytes, ctype: str, cache: str):
        etag = '"%s"' % hashlib.sha256(body).hexdigest()[:32]
        if self.headers.get("If-None-Match") == etag:
            self.send_response(304)
            self.send_header("ETag", etag)
            self.send_header("Cache-Control", cache)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", cache)
        self.send_header("ETag", etag)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802 - stdlib interface
        path = self.path.split("?", 1)[0]
        if path in ("/", "/index.html"):
            self._send(self.index, "text/html; charset=utf-8", "no-cache")
            return
        if path.startswith("/js/bundle-") and path.endswith(".js"):
            try:
                index = int(path[len("/js/bundle-") : -len(".js")])
            except ValueError:
                self.send_error(404)
                return
            if 0 <= index < len(self.bundles):
                self._send(
                    self.bundles[index],
                    "application/javascript",
                    "public, max-age=31536000, immutable",
                )
                return
        self.send_error(404)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8099)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument(
        "--copies",
        type=int,
        default=8,
        help="number of distinct JS bundles the app loads (default 8 ~ 5.9 MB)",
    )
    parser.add_argument("--print-port", action="store_true")
    args = parser.parse_args()

    bundles = build_bundles(args.copies)
    scripts = []
    for index in range(args.copies):
        scripts.append(f"<script>performance.mark('s{index}-start');</script>")
        scripts.append(f'<script src="/js/bundle-{index}.js"></script>')
        scripts.append(
            f"<script>performance.mark('s{index}-end');"
            f"performance.measure('s{index}','s{index}-start','s{index}-end');</script>"
        )
    boxes = "".join('<div class="box"></div>' for _ in range(6))
    Handler.bundles = bundles
    Handler.index = INDEX_TEMPLATE.format(
        scripts="\n".join(scripts), boxes=boxes
    ).encode()

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    total = sum(len(b) for b in bundles)
    port = server.server_address[1]
    if args.print_port:
        print(port, flush=True)
    print(
        f"fixture: http://{args.host}:{port}/  "
        f"{args.copies} bundles, {total / 1e6:.2f} MB of JS",
        file=sys.stderr,
        flush=True,
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        thread.join()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
