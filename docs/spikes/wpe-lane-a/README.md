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


---

# Spike B — CPU readback of the exported EGLImage (2026-07-27)

Spike A left ONE primitive unproven, and it is the one `capture-element` and
the lore-anchored pixel rung both need: getting CPU pixels back out of the
exported frame. **It works.** `src/bin/wpe-readback.rs`.

## Result — two fixtures, two predictable colours

```
$ wpe-readback http://127.0.0.1:8742/red.html  out --expect-rgb ff0000 --bench 20
[readback] EGL 1.5 surfaceless: ok
[readback] ES2 pbuffer EGLConfig: ok
[readback] GLES2 context current on EGL_NO_SURFACE: ok
[readback] glEGLImageTargetTexture2DOES resolved: ok
[readback] frame 1 640x480 blank=true
[readback] frame 2 640x480 blank=false
[readback] centre_rgba=255,0,0,255   fnv1a64=7c733d5f51f38325   channel_order=RGBA
[readback] ACCEPTANCE=PASS

$ wpe-readback http://127.0.0.1:8742/blue.html out --expect-rgb 0000ff --bench 20
[readback] centre_rgba=0,0,255,255   fnv1a64=9e9a3cb195d70325
[readback] ACCEPTANCE=PASS
```

Exact colours, opaque alpha, different checksums. The PNG was validated
independently by Python's `zlib`: signature good, **every chunk CRC verifies**,
and the IDAT inflates to exactly `(width*4+1)*height` bytes. Opening it shows a
solid field of the fixture's colour.

## ⚠ The failure this design caught — "readback succeeded" is not evidence

The first run reported a complete, error-free pipeline: context current,
extension resolved, framebuffer complete, `glReadPixels` clean, 1.2 MB written,
timings collected — and **the buffer was 307,200 identical `(0,0,0,0)` pixels.**
The compositor exports an initial frame BEFORE the page paints, and that blank
frame was being captured.

Nothing in "did the call succeed" could see this. Only the two-fixture
*predictable colour* acceptance could, which is why the brief asked for it and
why "bytes > 0" would have shipped a lie. The binary now skips blank frames and
accepts the first PAINTED one, and reports `blank_frames` so the skip is
visible rather than silent.

## Entry points actually needed — 21 new, not "~6"

Spike A sized this at "~6 GL entry points". The real count is **21 new foreign
declarations** (total 40 with spike A's 19):

| group | n | functions |
| --- | --- | --- |
| fdo image accessors | 3 | `wpe_fdo_egl_exported_image_get_egl_image` / `_get_width` / `_get_height` |
| EGL context | 5 | `eglBindAPI`, `eglChooseConfig`, `eglCreateContext`, `eglMakeCurrent`, `eglGetProcAddress` |
| GL readback | 12 | `glGenTextures`, `glBindTexture`, `glTexParameteri`, `glDeleteTextures`, `glGenFramebuffers`, `glBindFramebuffer`, `glFramebufferTexture2D`, `glCheckFramebufferStatus`, `glDeleteFramebuffers`, `glReadPixels`, `glFinish`, `glGetError` |
| resolved at runtime | 1 | `glEGLImageTargetTexture2DOES` |

Three structural facts the sizing missed:

1. **`glEGLImageTargetTexture2DOES` is an EXTENSION** (`GL_OES_EGL_image`), not
   a link-time symbol. It must come from `eglGetProcAddress`, so the real build
   needs a proc-address loader, not just a `-l` line.
2. **Spike A needed no GL context at all** — it never touched a pixel. Readback
   needs a real GLES2 context, `libGLESv2` linked, and
   `EGL_KHR_surfaceless_context` to make it current on `EGL_NO_SURFACE`.
3. **`eglChooseConfig` defaults `EGL_SURFACE_TYPE` to `EGL_WINDOW_BIT`**, and a
   surfaceless display has no window configs, so the obvious attribute list
   returns ZERO matches. Ask for `EGL_PBUFFER_BIT` (a
   `EGL_KHR_no_config_context` fallback is in the code but was not needed here).

Also: GL's origin is bottom-left and PNG's is top-left, so the readback flips
rows. On a solid-colour fixture that error is invisible — worth knowing before
someone debugs an upside-down `capture-element`.

## Cost — warm, N=20, `import + FBO attach + glReadPixels + glFinish`

| viewport | megapixels | min | **p50** | max | per megapixel |
| --- | --- | --- | --- | --- | --- |
| 640×480 | 0.31 | 1241 µs | **1293 µs** | 2298 µs | 4.2 ms |
| 1280×720 | 0.92 | 3923 µs | **4235 µs** | 6289 µs | 4.6 ms |
| 1920×1080 | 2.07 | 9895 µs | **10881 µs** | 14586 µs | 5.2 ms |

Linear in pixel count at roughly **5 ms per megapixel**. This is a synchronous
`glReadPixels` + `glFinish`, i.e. the naive worst case; a PBO round-trip would
overlap it.

## Revised Lane-A sizing

Spike B does not change the verdict, it firms it up and moves cost from
"unknown" to "known and acceptable". The engine binding is now measured at
**40 hand-written declarations** rather than 19 — still far below the gir
toolchain it replaces, still no codegen and no new workspace dependency, but
the readback half needs a GL loader and a small amount of genuine GL state
management (texture, FBO, row flip) that the invocation half did not. The
performance answer is the useful one: **~11 ms to get a 1080p frame into host
memory, ~4 ms at 720p, linear in pixels.** That is comfortably interactive for
the pixel rung's actual use — one capture per agent click, not per animation
frame — so the rung is not gated on a PBO pipeline, and `capture-element` crops
cost proportionally less because they read a sub-rect. The remaining unknown
for Lane A is no longer pixels; it is input routing
(`wpe_view_backend_dispatch_*`) and per-view process lifecycle, both of which
are first-party code the plan already accepts owning.

## Running it

```sh
cd docs/spikes/wpe-lane-a
python3 -m http.server 8742 --bind 127.0.0.1 --directory fixture &
cargo build --release --bin wpe-readback
env -u DISPLAY -u WAYLAND_DISPLAY ./target/release/wpe-readback \
    http://127.0.0.1:8742/red.html /tmp/red --expect-rgb ff0000 --bench 20
```

Exit 0 = PASS. `--size WxH` sets the offscreen viewport.
