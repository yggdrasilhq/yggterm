# Slice-2a spike — trusted input injection into a WebKitGTK WebView

**Question (the central slice-2 feasibility gate, `docs/agent-control-plane.md`
F2):** can an agent's `do` verb deliver a click/key to a page's WebView and have
the page treat it as **real input** (`isTrusted === true`), **without moving the
user's seat pointer**? Both review models (Claude + Codex) called this the
unproven premise the whole engine rests on.

**Answer: GO — proven on webkit2gtk 2.52 (2026-07-20).**

## Result

```
A control (JS dispatchEvent):     BTN-CLICK  isTrusted=FALSE  @210,145   (readout is honest)
B GDK button (WidgetExt::event):  BTN-MOUSEDOWN isTrusted=TRUE @210,145
                                  WIN-MOUSEDOWN isTrusted=TRUE @210,145
                                  BTN-CLICK     isTrusted=TRUE @210,145   (WebKit synthesized the
                                                                          click from press+release)
C test_widget_send_key:           handled=true
```

- Probe A (a JS `dispatchEvent(new MouseEvent('click'))`) reports
  `isTrusted=false`, proving the readout distinguishes trusted input from
  scripted input.
- Probe B — a **synthesized GDK button event delivered to the WebView widget** —
  reports `isTrusted=true` for mousedown AND a full synthesized click, at the
  exact page coordinates. No seat pointer was warped: the event was handed to
  the widget, not injected at the seat.
- Probe C — `gtk::test_widget_send_key` — is handled; keyboard injection is
  viable too.

## Hidden-surface phase (soft-stash / unmapped) — added 2026-07-20

The page is painted magenta (`#c800c8`), then the webview is `hide()`-unmapped
and re-probed:

```
D read (eval) while hidden:     PASS    bg=rgb(200,0,200)            (eval returns correct state)
E inject while hidden:          FAIL    events=[]                   (unmapped widget drops the GDK event)
F capture (snapshot) while hidden: PASS+FRESH  500x400 center_rgb=(200,0,200)  (re-rendered the magenta,
                                                                     NOT a stale cached frame)
```

- **read + capture reach a non-visible surface.** Capture is even proven
  **fresh** — the snapshot's center pixel is the magenta painted *after* the
  last visible frame, so `webkit.snapshot` re-renders current state while
  hidden. This kills the "backgrounded snapshot returns blank/stale" worry for
  the hidden case and is the engine half of gate 0(i).
- **inject needs a MAPPED target.** `WidgetExt::event` on an unmapped webview
  delivers nothing (`webview.window()` still exists, but events aren't
  processed). The under-glass **soft-stash keeps surfaces mapped** (demote =
  occluded, still realized+mapped), so `do` works on soft-stashed surfaces; a
  legacy hard-stashed / fully-hidden surface needs a transient off-screen map
  or defers to the farm plane. This is the spec's F2 sub-risk, confirmed with a
  defined fallback.

## The proven recipe (what slice-2b `do` uses)

```rust
// No seat pointer. Deliver straight to the target webview's widget.
let ev_ptr = gdk::ffi::gdk_event_new(GDK_BUTTON_PRESS);      // or _RELEASE
let bev = ev_ptr as *mut gdk::ffi::GdkEventButton;
(*bev).window = webview.window().to_glib_full();             // the webview's realized GdkWindow
(*bev).send_event = 0;                                       // look like windowing-system input
(*bev).x = x; (*bev).y = y;                                  // CSS px in the webview's doc space
(*bev).button = 1;
(*bev).device = default_seat.pointer().to_glib_full();       // a device is required
let event: gdk::Event = from_glib_full(ev_ptr);
WidgetExt::event(&webview, &event);                          // -> isTrusted TRUE, no seat move
```

Press + release on the same element makes WebKit synthesize the `click`.
`send_event = 0` matters — a non-zero (SendEvent) flag is the honest lever if a
future WebKit ever gates on it; here 0 yields fully trusted events.

## Implications for `docs/agent-control-plane.md`

- **Acceptance gate 0(ii) = PASS.** Slice 2 proceeds to **2b** on the GUI plane;
  `do` does NOT defer to the farm plane.
- The **F1** decision (one injection primitive) rides this recipe: the existing
  synthetic `Pointer`/`Grid` JS paths (`document::eval`, isTrusted=false) refold
  onto `WidgetExt::event`.
- Injection into an **unmapped** surface is proven to fail → soft-stash keeps
  surfaces mapped (works); hard-stash/hidden needs a transient map or the farm.
- read + capture on a non-visible surface proven (capture fresh) — the engine
  half of gate 0(i). The remaining owed piece is a live yggterm proof that
  `--session` resolves a real soft-stashed surface (verified in code:
  shell.rs:3391-3402), on an uncrowded host.
- Still to build in 2b (not feasibility blockers): coordinate mapping under
  page zoom/scroll, and the F3 lifecycle guards (generation handles, freshness,
  preemption).

## Run it

```
WEBKIT_DISABLE_DMABUF_RENDERER=1 xvfb-run -a cargo run --release
```

(`WEBKIT_DISABLE_DMABUF_RENDERER=1` only because Xvfb is software-only; the live
app runs the DMABuf path.)
