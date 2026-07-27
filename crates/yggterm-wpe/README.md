# yggterm-wpe

The headless WPE WebKit engine for yggterm's agent surfaces — **Lane A,
increment 1**.

Four spikes (`docs/spikes/wpe-lane-a/`, `docs/spikes/pty-fd-handoff/`) emptied
the Lane-A unknown list. This is where the proven parts become a library.

## Status

Headless only. **No GUI integration, no consumers, not a workspace member.**
The crate builds and tests standalone; wiring is a later increment.

> **Why it is not in the root `members` list.** Building it requires
> `libwpewebkit-2.0-dev`, `libwpebackend-fdo-1.0-dev` and `libgles-dev`. Adding
> it to the workspace would make those a hard prerequisite for building
> *anything* in the repo, on every fleet machine — including the GUI host, which
> has no reason to carry them yet. Nothing is lost by staying detached while the
> crate has no consumers. **Increment 2 must decide** between a feature flag and
> a documented prerequisite.

## Shape

```text
Engine          one-time headless bring-up; owns EGL + the GLES2 context
  └── View      one page: navigate, readback, click, type
Supervisor      owns N views + the process→view map WebKit does not provide
```

| module | what it owns |
| --- | --- |
| `ffi` | all 42 foreign declarations, `pub(crate)` — no raw handle escapes |
| `keysym` | ASCII → (XKB keysym, evdev code); refuses what it cannot type |
| `frame` | `Frame`: RGBA8, **top-left origin**, blankness, fingerprint |
| `view` | one view; the single `static` export client; input; readback |
| `supervisor` | N views, descendant process walk, detect + explicit restart |

## Public API

```rust
Engine::new_headless() -> Result<Engine>     // once per process; second is refused
Engine::view(w, h)     -> Result<View>
Engine::pump()

View::load_uri / reload / title / uri / is_loading / load_finished
View::last_frame() -> Option<&Frame>         // NEVER blank
View::painted_since_load() -> bool           // not the same as "has a frame"
View::forget_frame() / frames_exported() / blank_frames_skipped()
View::click(x, y) / click_centre()           // motion → down → up
View::type_text(&str) -> Result<()>          // refuses untypable characters
View::press_key(keysym) / web_process_terminated()

Supervisor::new(&Engine) / open(uri, w, h, timeout) -> Result<ViewId>
Supervisor::view / view_mut / ids / len
Supervisor::web_processes() / web_process_of(id) / terminated()
Supervisor::restart(id, timeout)             // EXPLICIT, never automatic
Supervisor::kill_web_process_of(id)
Supervisor::pump_until(timeout, cond) / await_frame(id, timeout, accept)
```

## Every spike gotcha is a shape, not a comment

| Gotcha (and what it cost) | How the crate forecloses it |
| --- | --- |
| The fdo client struct is **stored by pointer**; a per-view local dangles and SIGSEGVs later, at a different place each run | Exactly ONE client, in `static` storage. No per-view allocation exists to drop early. Locked by a test that fails if a second `FdoEglClient` literal appears. |
| `wpe_loader_init` must run first | `View` can only come from an `Engine`, and `Engine::new_headless` is the only constructor. |
| A pointer BUTTON without a preceding MOTION hit-tests at (0,0) | No raw button dispatch is exposed. `click()` sends motion→down→up; a test asserts exactly three dispatches in the right order. |
| `key_code` is an **XKB keysym**, not ASCII — a wrong one is silently swallowed | No key code crosses the API. `type_text` takes text; `keysym` owns the encoding and **refuses** characters it cannot type rather than guessing. |
| `eglChooseConfig` defaults to `EGL_WINDOW_BIT`, of which a surfaceless display has **none** | Handled inside bring-up. |
| The **first exported frame is blank**, and every success check passes on it | `last_frame()` can never return a blank frame; skips are counted, not silent. |
| GL's origin is bottom-left, every image format's is top-left | `Frame` is flipped once, at the source. |
| Web processes are bubblewrap **grandchildren** with `comm` truncated to 15 chars | `web_processes()` walks descendants and prefix-matches; a unit test spawns a real grandchild so a direct-children regression fails. |

## Two bugs this crate found that the spikes did not

The spikes proved the primitives; assembling them under a stricter definition of
"restored" exposed two more:

1. **`restart()` must wait for a post-load paint, not any non-blank frame.**
   Recovering a killed view paints an intermediate **white** frame first —
   non-blank, and completely wrong. Returning on it reports a restored surface
   that is still empty. Hence `View::painted_since_load()`, which is deliberately
   not the same question as "has a frame".
2. **`reload()` does not recover a view whose web process was killed.** There is
   no document left to reload; the load completes against nothing and the view
   settles white. Recovery **re-navigates to the view's own URI**, which is why
   `Supervisor` remembers it. (Spike C reported reload working — under this
   crate's stricter settle it reliably does not, and re-navigation is correct in
   both cases.)

## Tests

```sh
cd crates/yggterm-wpe
CARGO_BUILD_JOBS=6 nice -n 19 cargo test -- --test-threads=8
```

15 unit tests plus one headless integration test that runs nine scenarios in
order against a real engine. **The colour readback is the assertion
instrument** — the fixture turns green only on `pointerdown` and blue only on a
`keydown` whose `e.key === "x"`, so a colour read out of the compositor's frame
proves a real event reached WebCore. A DOM query would prove only that
JavaScript runs.

The integration test is one `#[test]`, not nine, because libwpe's loader, the
EGL display and the current GL context are process-global and killing a web
process is observable to every view — the scenarios genuinely cannot run
concurrently, and a mutex pretending otherwise would be theatre. Its fixture
server is 40 lines of `std::net`, so the suite needs no Python and no network.

Requires `libwpewebkit-2.0-dev`, `libwpebackend-fdo-1.0-dev`, `libgles-dev` and
a readable DRM render node. No display server.
