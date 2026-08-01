#!/usr/bin/env python3
"""Can a native web surface be made SMALLER? — the instrument for the one
question this repo has no other way to answer.

WHY IT EXISTS. A web surface's size is invisible to every instrument yggterm
owns. `app screenshot`'s default backend composites the DOM and is blind to
native children; the reconciler's applied map holds the rect the shell *asked*
for, not the one GTK took; and the widget's allocation is not published
anywhere. So a page painting at the wrong size looks, from the inside, exactly
like a page painting at the right one — the only witnesses are the pixels and
`window.innerWidth` inside the page itself.

WHAT IT MEASURES. The widget tree `WebSurfaceHost` builds, in miniature and
with nothing else in it:

    GtkWindow
      └── GtkOverlay
            ├── base child          (stands in for the shell webview / glass)
            └── GtkFixed            halign/valign = START,
                  │                 margin-start/top = x/y, size-request = w×h
                  └── WebKitWebView  put at (0,0), size-request = w×h

`apply_bounds()` below is `vendor/dioxus-desktop/src/web_surface.rs`'s
`apply_bounds`, line for line. Each variant then replays one real host path
around a resize and prints what the PAGE believes its width is.

WHAT IT FOUND (2026-08-01, WebKitGTK 4.1 / GTK 3, hardware and llvmpipe alike):

    A  resize while visible                       shrinks    ✓
    B  hide → resize → show                       STUCK WIDE ✗   ← the bug
    C  detach → resize → re-attach → show         shrinks    ✓
    D  hide widget and container → resize → show  STUCK WIDE ✗
    E  hide → resize → show → resize again        STUCK WIDE ✗   ← same turn is too late
    F  hide → show → resize                       shrinks    ✓   ← THE FIX
    G  hide → size-request only → show → resize   STUCK WIDE ✗
    H  hide → resize → (next turn) show + resize  STUCK WIDE ✗

Two GTK/WebKit facts explain the whole table, and both are needed:

  * GTK drops `size_allocate` on a widget whose visible flag is false — so the
    one call that actually resizes the page does nothing while it is hidden;
  * a `WebKitWebViewBase` answers `get_preferred_width` with its OWN CURRENT
    VIEW SIZE, so once the view is wider than the request, the natural size a
    layout pass reads is still the old, larger one.

Growing is therefore free and shrinking is not: the only thing that breaks the
loop is an allocation the widget can process. Any geometry written while hidden
does not merely miss — it poisons the apply that follows it (E, G, H), which is
why the rule in `apply_bounds` is to record while hidden and re-assert AFTER the
show, and never to place a surface before showing it.

RUNNING IT (no display needed; ~15 s per variant):

    xvfb-run -a -s '-screen 0 1920x1200x24' python3 scripts/webview-shrink-probe.py
    xvfb-run -a -s '-screen 0 1920x1200x24' python3 scripts/webview-shrink-probe.py B

Needs python3-gi with Gtk 3.0 and WebKit2 4.1 typelibs (Debian:
`python3-gi gir1.2-gtk-3.0 gir1.2-webkit2-4.1`). Exits non-zero if any variant
it ran left the page at the wrong width.
"""

from __future__ import annotations

import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gdk, GLib, Gtk, WebKit2  # noqa: E402

#: The window, and the two page rects the user's own report is made of: a
#: 1400 px viewport with the cwd tree docked, 1665 px with it hidden.
WIN = (1920, 1200)
NARROW = (269, 4, 1400, 1192)
WIDE = (4, 4, 1665, 1192)

VERDICTS: list[tuple[str, bool, str]] = []


def _alloc(x: int, y: int, w: int, h: int) -> Gdk.Rectangle:
    rect = Gdk.Rectangle()
    rect.x, rect.y, rect.width, rect.height = x, y, w, h
    return rect


class Probe:
    """One window, one surface, one variant."""

    def __init__(self, variant: str) -> None:
        self.variant = variant
        self.window = Gtk.Window()
        self.window.set_default_size(*WIN)
        self.overlay = Gtk.Overlay()
        self.window.add(self.overlay)
        self.overlay.add(Gtk.DrawingArea())

        self.fixed = Gtk.Fixed()
        self.fixed.set_halign(Gtk.Align.START)
        self.fixed.set_valign(Gtk.Align.START)
        self.view = WebKit2.WebView()

        x, y, w, h = NARROW
        self.fixed.set_margin_start(x)
        self.fixed.set_margin_top(y)
        self.fixed.set_size_request(w, h)
        self.view.set_size_request(w, h)
        self.fixed.put(self.view, 0, 0)
        self.overlay.add_overlay(self.fixed)
        self.view.load_html("<body style='margin:0;background:#262a33'></body>", None)
        self.window.show_all()

    # -- the host's own placement, line for line ---------------------------

    def apply_bounds(self, x: int, y: int, w: int, h: int) -> None:
        self.fixed.set_margin_start(max(0, x))
        self.fixed.set_margin_top(max(0, y))
        self.fixed.set_size_request(max(1, w), max(1, h))
        self.view.set_size_request(max(1, w), max(1, h))
        # wry's `set_bounds` on a GtkFixed parent: a direct size_allocate.
        self.view.size_allocate(_alloc(0, 0, max(1, w), max(1, h)))
        self.overlay.queue_resize()

    def size_request_only(self, x: int, y: int, w: int, h: int) -> None:
        self.fixed.set_margin_start(max(0, x))
        self.fixed.set_margin_top(max(0, y))
        self.fixed.set_size_request(max(1, w), max(1, h))
        self.view.set_size_request(max(1, w), max(1, h))

    def show_view(self) -> None:
        self.view.set_visible(True)
        self.fixed.show_all()

    # -- the variants ------------------------------------------------------

    def shrink(self) -> None:
        getattr(self, f"shrink_{self.variant.lower()}")()

    def shrink_a(self) -> None:
        self.apply_bounds(*NARROW)

    def shrink_b(self) -> None:
        self.view.set_visible(False)
        self.apply_bounds(*NARROW)
        self.show_view()
        self.overlay.queue_resize()

    def shrink_c(self) -> None:
        self.overlay.remove(self.fixed)
        self.apply_bounds(*NARROW)
        self.overlay.add_overlay(self.fixed)
        self.apply_bounds(*NARROW)
        self.show_view()
        self.overlay.queue_resize()

    def shrink_d(self) -> None:
        self.view.set_visible(False)
        self.fixed.set_visible(False)
        self.apply_bounds(*NARROW)
        self.fixed.set_visible(True)
        self.show_view()
        self.overlay.queue_resize()

    def shrink_e(self) -> None:
        self.view.set_visible(False)
        self.apply_bounds(*NARROW)
        self.show_view()
        self.apply_bounds(*NARROW)
        self.overlay.queue_resize()

    def shrink_f(self) -> None:
        self.view.set_visible(False)
        self.show_view()
        self.apply_bounds(*NARROW)
        self.overlay.queue_resize()

    def shrink_g(self) -> None:
        self.view.set_visible(False)
        self.size_request_only(*NARROW)
        self.show_view()
        self.apply_bounds(*NARROW)
        self.overlay.queue_resize()

    def shrink_h(self) -> None:
        self.view.set_visible(False)
        self.apply_bounds(*NARROW)

        def reveal() -> bool:
            self.show_view()
            self.apply_bounds(*NARROW)
            self.overlay.queue_resize()
            return False

        GLib.idle_add(reveal)

    # -- reading the only witness there is ---------------------------------

    def report(self, label: str, then, settle_ms: int = 800) -> None:
        def got(view, result) -> None:
            try:
                inner = view.evaluate_javascript_finish(result).to_int32()
            except Exception as exc:  # noqa: BLE001
                inner = f"error: {exc}"
            fixed = self.fixed.get_allocation()
            view_alloc = self.view.get_allocation()
            minimum, natural = self.view.get_preferred_width()
            print(
                f"[{self.variant}] {label:<24} page={inner!s:>6}  "
                f"fixed={fixed.width}  view={view_alloc.width}  "
                f"view_pref=(min={minimum}, nat={natural})",
                flush=True,
            )
            self.last = inner
            GLib.timeout_add(settle_ms, then)

        self.view.evaluate_javascript("window.innerWidth", -1, None, None, None, got)

    def run(self) -> None:
        def born() -> bool:
            self.report("born at 1400", grow)
            return False

        def grow() -> bool:
            self.apply_bounds(*WIDE)
            GLib.timeout_add(700, lambda: self.report("grown to 1665", shrink) or False)
            return False

        def shrink() -> bool:
            self.shrink()
            GLib.timeout_add(
                900, lambda: self.report("asked back to 1400", verdict) or False
            )
            return False

        def verdict() -> bool:
            ok = self.last == NARROW[2]
            VERDICTS.append(
                (
                    self.variant,
                    ok,
                    "shrinks"
                    if ok
                    else f"STUCK at {self.last} — the page never took the smaller rect",
                )
            )
            Gtk.main_quit()
            return False

        GLib.timeout_add(1200, born)
        GLib.timeout_add(30_000, lambda: Gtk.main_quit() or False)
        Gtk.main()
        self.window.destroy()


#: The paths the host actually takes. B, D, E, G and H are the KNOWN-BAD ones:
#: they are here to stay measurable, not to pass.
MUST_SHRINK = {"A", "C", "F"}
ALL_VARIANTS = "ABCDEFGH"


def main() -> int:
    asked = [v.upper() for v in sys.argv[1:]]
    for variant in asked:
        if not hasattr(Probe, f"shrink_{variant.lower()}"):
            print(f"no such variant: {variant}", file=sys.stderr)
            return 2

    # ONE PROCESS PER VARIANT, always. Two probes in one process share a main
    # loop: the first one's pending timeouts fire into the second one's window
    # and both answers are garbage — measured, and it read exactly like a real
    # failure, which is the one thing an instrument may never do.
    if len(asked) != 1:
        import subprocess

        results: list[tuple[str, bool, str]] = []
        for variant in asked or list(ALL_VARIANTS):
            done = subprocess.run(
                [sys.executable, __file__, variant],
                capture_output=True,
                text=True,
            )
            sys.stdout.write(done.stdout)
            verdict = [
                line for line in done.stdout.splitlines() if line.startswith("VERDICT ")
            ]
            if not verdict:
                results.append((variant, False, "the probe printed no verdict"))
                continue
            _, name, state, note = verdict[-1].split(" ", 3)
            results.append((name, state == "ok", note))
        print()
        for variant, ok, note in results:
            print(f"  {variant}  {'ok  ' if ok else 'FAIL'}  {note}")
        broken = [v for v, ok, _ in results if v in MUST_SHRINK and not ok]
        if broken:
            print(f"\nthe host's own paths no longer shrink: {', '.join(broken)}")
            return 1
        return 0

    Probe(asked[0]).run()
    variant, ok, note = VERDICTS[-1]
    print(f"VERDICT {variant} {'ok' if ok else 'fail'} {note}")
    return 0 if ok or variant not in MUST_SHRINK else 1


if __name__ == "__main__":
    sys.exit(main())
