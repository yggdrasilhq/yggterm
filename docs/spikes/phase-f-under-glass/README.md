# Phase F spike — under-glass web surfaces

Proves the two Phase F primitives on plain webkit2gtk (GTK3), no engine
swap. See `docs/web-under-glass.md` for the plan this feeds.

What it builds: a window with a **page** webview (red; turns yellow when
its DOM receives a click) as the `gtk::Overlay` base child, and a **shell**
webview above it — transparent background, DOM draws a titlebar (turns
magenta on click) plus an opaque frame with a rounded transparent hole —
with an input-shape hole punched over the page rect.

PASS looks like:

1. **Paint**: red page visible through the rounded hole; frame covers the
   page's square corners; titlebar draws over the page. (Falsifies the
   "native child is a subsurface that draws above everything" belief —
   WebKitGTK 2.52 composites in-widget; z-order + alpha are honored.)
2. **Input**: a click inside the hole reaches the page (yellow); a click on
   the titlebar stays with the shell (magenta). GTK-level `button-press`
   logs on stderr say which widget got each event.

3. **Keyboard** (F.-1 extension, verified 2026-07-19): click hole → typed key
   reaches the page DOM ("PAGE GOT KEY"); click titlebar → next key reaches
   the shell DOM ("SHELL GOT KEY"). Click-to-focus round-trips both ways.
   Wayland keyboard injection needs the same held-connection trick as the
   pointer: `wtype -s 2500 <key>` (in-stream sleep so the client binds
   `wl_keyboard` before the key event; instant wtype keys are lost).

Verified 2026-07-18/19 on webkit2gtk 2.52.4, both backends:
- Wayland: sway `WLR_BACKENDS=headless` + grim, GPU render node present.
  Input needs a **held** virtual-pointer connection (pywayland +
  `zwlr_virtual_pointer_v1`, sleep ~2s after create so the client binds
  `wl_pointer` before the button event; transient `wlrctl` clicks are lost).
- X11: Xvfb + xdotool.

Traps this spike caught (already baked into the plan):

- `GtkOverlay` wraps each overlay child in an intermediate GdkWindow with
  an empty event mask — shape it too, or unhandled events bubble to the
  toplevel instead of falling through to the page.
- Never shape the toplevel: on X11 `parent()` of a toplevel is the root
  window, not `None`; guard the ancestor walk by identity, not by `None`.

Run: `cargo build` here, launch under any compositor, click the hole
center and the titlebar, watch stderr + a screenshot.

## F.0.1 bisection knobs (2026-07-19)

Used to isolate the production failure; all combinations PASS except the last:

- `SPIKE_TREE=prod` — production widget tree (backdrop Box base child, page
  webview in a `gtk::Fixed` overlay child, shell in a GtkBox overlay child
  reordered topmost).
- `SPIKE_WIN=rgba` — RGBA visual + app_paintable toplevel (tao-transparent).
- `SPIKE_SHELL_AC=always` — hardware-acceleration-policy ALWAYS + real AC
  content (WebGL canvas + 3D-transformed layer) in the shell.
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` — **reproduces the app failure**: the
  SHM presentation path clears the shell's transparent regions through the
  page beneath (hole = black on an opaque window). This was the production
  root cause — yggterm set it as an llvmpipe-crash workaround.
