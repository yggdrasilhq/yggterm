# WPE Lane-A reachability spike

Answers ONE question for `docs/optimization-pass.md` §WS2 Lane A: **can Rust
drive WPEWebKit headless on this fleet today?**

**Yes — but not by the route the plan assumed.** Verified 2026-07-26 on a
headless Linux (Debian sid) server host: no X server, no Wayland compositor,
no `sway --headless`, no Xvfb.

## Result

```
[spike] binding_surface_fn_count=19
[spike] DISPLAY=None WAYLAND_DISPLAY=None
[spike] +0ms   wpe_loader_init: ok
[spike]        EGL 1.5 vendor="Mesa Project" version="1.5"
[spike] +45ms  wpe_fdo_initialize_for_egl_display: ok
[spike] +45ms  exportable fdo backend: ok (1280x720 offscreen)
[spike] +51ms  load_uri(http://127.0.0.1:8731/)
[spike] +262ms load-changed: 0     (WEBKIT_LOAD_STARTED)
[spike] +266ms load-changed: 2     (WEBKIT_LOAD_COMMITTED)
[spike] +278ms load-changed: 3     (WEBKIT_LOAD_FINISHED)
[spike] +310ms first frame exported
[spike] ---- RESULT ----
[spike] load_finished=true  load_failed=false  timed_out=false
[spike] title="YGGTERM-LANE-A-FIXTURE-OK"
[spike] uri="http://127.0.0.1:8731/"
[spike] frames_exported=2
[spike] ACCEPTANCE=PASS
```

Acceptance was "load-finished + content arrived (title or snapshot bytes)".
Both halves hold, and the frame count is a **stronger** signal than the brief
asked for: `frames_exported=2` means the compositor actually produced pixel
buffers offscreen, not merely that the DOM parsed.

## Three findings that correct the plan

1. **`WPEDisplayHeadless` does not exist here.** Debian's `libwpewebkit-2.0`
   is built WITHOUT `ENABLE_WPE_PLATFORM`: there is no `libWPEPlatform-*.so`,
   no `wpe-platform-*.pc`, and `nm -D` finds **zero** `wpe_display_*` symbols.
   The optimization-pass text naming `WPEDisplayHeadless` as the Lane-A
   mechanism is not reachable from distro packages. The working route is the
   **legacy libwpe + WPEBackend-fdo "exportable" backend**, which runs an
   in-process nested Wayland compositor (libwayland-server) and hands rendered
   buffers to our callbacks — no display server involved.

2. **There is no `.gir` for WPE in Debian.** `libwpewebkit-2.0-dev` ships only
   two `.pc` files and headers; `/usr/share/gir-1.0` and the typelib dir carry
   nothing for WPE (`gir1.2-webkit-*` exist only for the **GTK** port). So the
   plan's stated binding strategy — "regenerate gir bindings against WPE's
   `.gir` and vendor them" — cannot be executed off distro packages. It would
   need WPE rebuilt from source with introspection enabled.
   **This does not invalidate the conclusion, it improves it:** the surface
   actually required is 19 hand-written `extern "C"` declarations (see
   `BINDING_SURFACE_FN_COUNT` in `src/main.rs`), which is smaller than the gir
   toolchain it replaces. Hand-written FFI is the recommendation.

3. **`libwpebackend-fdo-1.1-dev` does not exist**; sid has the `1.0` ABI
   (`libwpebackend-fdo-1.0-dev` 1.16.1-1+b1). And there is no
   `libWPEBackend-default.so`, so `wpe_loader_init("libWPEBackend-fdo-1.0.so")`
   is **mandatory**, not optional — omitting it is the first thing that will
   fail in a real build.

## Package versions (as installed, Debian sid, 2026-07-26)

| package | version |
| --- | --- |
| `libwpewebkit-2.0-dev` / `-1` | 2.52.5-1 (`wpe-webkit-2.0` 2.52.5) |
| `libwpebackend-fdo-1.0-dev` | 1.16.1-1+b1 (`wpebackend-fdo-1.0` 1.16.1) |
| `libwpe-1.0-dev` | 1.16.3-1+b1 (`wpe-1.0` 1.16.3) |
| Mesa EGL | 26.1.5-1, EGL 1.5, surfaceless platform |
| rustc / cargo | 1.94.0 |

Aux processes live at `/usr/lib/x86_64-linux-gnu/wpe-webkit-2.0/`
(`WPEWebProcess`, `WPENetworkProcess`, `WPEGPUProcess`) and are found
automatically — no `WEBKIT_EXEC_PATH` needed.

## Binding surface — 19 functions

| library | n | functions |
| --- | --- | --- |
| `libwpe-1.0` | 1 | `wpe_loader_init` |
| `libWPEBackend-fdo-1.0` | 5 | `wpe_fdo_initialize_for_egl_display`, `wpe_view_backend_exportable_fdo_egl_create`, `..._get_view_backend`, `..._dispatch_frame_complete`, `..._egl_dispatch_release_exported_image` |
| `libEGL` | 3 | `eglGetPlatformDisplay`, `eglInitialize`, `eglQueryString` |
| `libWPEWebKit-2.0` | 5 | `webkit_web_view_backend_new`, `webkit_web_view_new`, `webkit_web_view_load_uri`, `webkit_web_view_get_title`, `webkit_web_view_get_uri` |
| `libglib-2.0` / `libgobject-2.0` | 5 | `g_main_loop_new`, `g_main_loop_run`, `g_main_loop_quit`, `g_timeout_add`, `g_signal_connect_data` |

Plus one `#[repr(C)]` struct (`wpe_view_backend_exportable_fdo_egl_client`,
five function pointers). No bindgen, no gir, no `glib` crate.

**The one non-obvious contract:** the `export_fdo_egl_image` callback MUST
release the image and call `dispatch_frame_complete`, or WebKit stalls waiting
for the frame ack and the page never advances past the first paint.

## Running it

```sh
cd docs/spikes/wpe-lane-a
python3 -m http.server 8731 --bind 127.0.0.1 --directory fixture &
cargo build --release
env -u DISPLAY -u WAYLAND_DISPLAY ./target/release/wpe-lane-a-spike http://127.0.0.1:8731/
```

Exit 0 = PASS. Needs `libwpewebkit-2.0-dev` + `libwpebackend-fdo-1.0-dev`
installed and a DRM render node (`/dev/dri/renderD*`) readable.

## Wall-clock cost

- Spike itself: ~1 session-hour end to end, including package install and
  header archaeology.
- **Cold first run: ~14 s.** That is WebKit populating its shader/gresource
  caches, and it is once-per-host, not once-per-page.
- **Warm run: 278 ms to load-finished, 310 ms to first exported frame**, from
  process start, including EGL init (45 ms) and web-process spawn. Reproduced
  identically across runs.
- Mesa selected a hardware render node (`iris`), not `llvmpipe`, with no
  display server present — so Lane A gets GPU compositing on a headless host,
  which was the open question behind the whole workstream.

## Sizing the real Lane-A build

The engine half is small and de-risked: a `yggterm-wpe` crate wrapping these
19 symbols is a few hundred lines, has no build-time codegen, no gir
toolchain, and no new workspace dependency — materially cheaper than the
"vendor and patch generated bindings" cost the plan budgeted. The real work is
everything the exportable backend hands us rather than does for us, and it is
all first-party code we were going to own anyway per the settled reasoning:
(a) **input routing** — `wpe_view_backend_dispatch_*` for keyboard/pointer/
touch, which is exactly the "views are not widgets, input routing becomes our
code" property the plan wants; (b) **frame consumption** — today the spike
releases each `EGLImage` immediately; the agent engine must instead import it
(`glEGLImageTargetTexture2DOES` + `glReadPixels`, or dmabuf export straight to
the consumer) to serve `capture-element` and the pixel rung, roughly 6 more GL
entry points and the only piece this spike did NOT prove; (c) **process
lifecycle** — one `WPEWebProcess` per view, plus the network/GPU processes,
under our supervision rather than GTK's. Recommendation: build the agent
engine on this route, drop `WPEDisplayHeadless` from the plan, and treat
CPU readback as the next spike since it is the sole unproven primitive.
