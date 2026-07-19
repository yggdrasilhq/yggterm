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
- Still to prove in 2b (not blockers, the injection premise is settled):
  delivery into a **demoted/soft-stashed** webview (this spike used a mapped,
  visible one — the demoted case is the narrower F2 sub-risk), coordinate
  mapping under page zoom/scroll, and the F3 lifecycle guards.

## Run it

```
WEBKIT_DISABLE_DMABUF_RENDERER=1 xvfb-run -a cargo run --release
```

(`WEBKIT_DISABLE_DMABUF_RENDERER=1` only because Xvfb is software-only; the live
app runs the DMABuf path.)
