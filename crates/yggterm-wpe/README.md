# yggterm-wpe

The headless WPE WebKit engine for yggterm's agent surfaces — **Lane A**.
Increment 1 is the engine; increment 2 is the agent verb plane (below).

Four spikes (`docs/spikes/wpe-lane-a/`, `docs/spikes/pty-fd-handoff/`) emptied
the Lane-A unknown list. This is where the proven parts become a library.

## Status

Headless only. **No GUI integration, no consumers, not a workspace member.**
The crate builds and tests standalone; wiring is a later increment.

> **Why it is not in the root `members` list.** Building it requires
> `libwpewebkit-2.0-dev`, `libwpebackend-fdo-1.0-dev` and `libgles-dev`. Adding
> it to the workspace would make those a hard prerequisite for building
> *anything* in the repo, on every fleet machine — including the GUI host, which
> has no reason to carry them yet. **Settled by the integrator (round 5): the
> crate STAYS a non-member and the WPE dev stack is a documented prerequisite
> for building THESE crates only.** Membership is revisited only when a fleet
> consumer actually wires in.

## Shape

```text
Engine          one-time headless bring-up; owns EGL + the GLES2 context
  └── View      one page: navigate, readback, click, type
Supervisor      owns N views + the process→view map WebKit does not provide
```

| module | what it owns |
| --- | --- |
| `ffi` | all 48 foreign declarations, `pub(crate)` — no raw handle escapes |
| `json` | a minimal JSON reader/writer for the line protocol |
| `png` | a minimal PNG encoder for the capture verbs |
| `agent` | the verb plane (increment 2), testable without a socket |
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
View::painted_current_document() -> bool     // not the same as "has a frame"
View::forget_frame() / frames_exported() / blank_frames_skipped()
View::click(x, y) / click_centre()           // motion → down → up
View::type_text(&str) -> Result<()>          // refuses untypable characters
View::press_key(keysym) / web_process_terminated()

Supervisor::new(&Engine) / open(uri, w, h, timeout) -> Result<ViewId>
Supervisor::view / view_mut / ids / len
Supervisor::web_processes() / web_process_of(id) / terminated()
Supervisor::restart(id, timeout)             // EXPLICIT, never automatic
Supervisor::kill_web_process_of(id)
Supervisor::eval(id, script, timeout) -> Result<String>   // pumps for you
Supervisor::pump_until(timeout, cond) / await_frame(id, timeout, accept)
Frame::crop(x, y, w, h) / to_png()
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

1. **`restart()` must wait for a picture of the CURRENT document, not any
   non-blank frame.** Recovering a killed view paints an intermediate **white**
   frame first — non-blank, and completely wrong. Returning on it reports a
   restored surface that is still empty. Hence
   `View::painted_current_document()`, which is deliberately not the same
   question as "has a frame".
   ⚠ The first fix for this was itself wrong and is recorded as such: "painted
   after load-finished" never becomes true for a small document whose paint
   precedes the load-finished signal, so every navigation timed out. The
   criterion is document-generation based, plus a bounded settle window.
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

27 unit tests plus two headless integration suites — the engine's nine
scenarios and the verb plane's end-to-end round trip — each run in order against
one real engine. **The colour readback is the assertion instrument** — the fixture turns green only on `pointerdown` and blue only on a
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


---

# Increment 2 — the agent verb plane (`yggterm-wpe-agent`)

The engine as **its own supervised process**, speaking one JSON object per line
over a Unix socket.

```sh
yggterm-wpe-agent /run/user/1000/yggterm-wpe.sock
```

## Why a separate process

`Engine` is a process singleton — libwpe's loader, the EGL display and the
current GL context are all per-process, and the crate refuses a second engine.
Hosting it inside the daemon would permanently couple the daemon's lifetime to
WebKit's, and an engine crash would take the daemon with it, which the
constitution forbids. The daemon spawns, probes and restarts this instead.

**Startup is honest.** The engine is brought up BEFORE the socket is bound, so
a supervisor that can connect is entitled to assume the engine works. If the WPE
stack is missing, the binary names the failure on stderr and exits non-zero — it
never lingers as a daemon that answers every verb with an error, which a
supervisor cannot tell apart from a working engine with a bad page.

## Protocol

One JSON object per line, both directions. Every response echoes `id` and
carries `ok`; a failure carries `error` and never a partial result. A malformed
line still gets a well-formed answer — leaving a caller waiting on a socket for
a reply that never comes is the worst failure mode of a line protocol.

```text
-> {"id":"1","verb":"ensure","session":"a","url":"http://…/p","width":320,"height":240}
<- {"id":"1","ok":true,"view":0,"created":true}
-> {"id":"2","verb":"click","session":"a","selector":"#go"}
<- {"id":"2","ok":true,"x":42,"y":17}
-> {"id":"3","verb":"read-back","session":"a","selector":"#out"}
<- {"id":"3","ok":true,"text":"clicked","value":null}
```

## Verbs as shipped

| verb | arguments | answers |
| --- | --- | --- |
| `ensure` | `session`, `url?`, `width?`, `height?` | `view`, `created` — idempotent per session key |
| `navigate` | `session`, `url` | `title`, `uri` (after a current-document paint) |
| `eval` | `session`, `script` | `result` — **typed**, not stringified |
| `click` | `session`, `selector` | `x`, `y` of the point clicked |
| `type` | `session`, `text` | `typed` (character count) |
| `read-back` | `session`, `selector` | `text`, `value` |
| `capture-view` | `session`, `path` | `path`, `width`, `height`, `bytes` |
| `capture-element` | `session`, `selector`, `path` | same, cropped to the element's rect |
| `restart` | `session` | `previous_web_process`, `web_process` |
| `status` | — | `views[]` (incl. `web_process_terminated`), `web_processes[]` |

All verbs accept `timeout_ms`.

## Three rules the plane enforces

1. **Ambiguity is refused, with the count.** A selector matching 0 or 2+ nodes
   is an error naming how many it matched — never a silent first-match.
   "It clicked something" is the worst possible outcome for an agent.
2. **Recovery is surfaced, never automatic.** `status` names a terminated view;
   only an explicit `restart` brings it back. A plane that quietly re-spawns a
   crashing view turns a visible fault into an invisible loop.
3. **A failure is never an empty success.** A page that throws surfaces the
   engine's own message; an untypable character is refused with a reason. An
   empty string would be read as a legitimate `""`.

## Two things that bit, and are now shape

- **A `GError` is not a `GObject`.** Freeing one with `g_object_unref` corrupts
  the heap; the agent died silently — no panic, no message — on the very first
  `eval`. It is `g_error_free`, and the FFI declaration says so.
- **Input dispatch is asynchronous into the web process.** A `click` verb that
  returned immediately let the caller's own next `read-back` observe the page
  *before* the click landed, which reads exactly like "the click did nothing".
  `click` and `type` settle briefly before returning.

## Tests

`tests/agent_verbs.rs` spawns the **real binary**, connects over the **real
socket** and drives every verb — deliberately not a library-level test of
`AgentState`, because a process talking JSON-per-line is what the daemon will
actually depend on. It then kills one session's web process and proves that
`status` names it, the *other* session keeps answering throughout, and an
explicit `restart` returns a fresh document that takes input again on a new pid.
