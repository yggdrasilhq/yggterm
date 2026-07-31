# Screen tearing in the web-surface viewport — investigation, 2026-07-31

> **Verdict: root cause NOT proven, and the framing it was given was falsified.**
> Both technical meanings of "tearing" are now ruled out for this window, on the
> user's own GPU. What the user is seeing is therefore something else, and the
> §"What would settle it" section names the one cheap observation that separates
> the two remaining candidates. **No production code was changed** — the Iron Law
> is no fixes without root cause, and a speculative render change on the daily
> driver would be worse than the symptom.

## 1. The symptom

> "screen tearing while scroll or animations"

— in the ychrome browser viewport, which is a yggterm web surface, on guihost
(the live KDE/Wayland GUI host).

## 2. The instrument

Tearing is a thing the *eye* sees, so the tempting method is to capture frames
and squint. That is not good enough and the field guide says so. This
investigation built a decoder instead.

- `tools/tear-probe/page.html` paints every content band with a colour that
  **encodes its own content-space row index plus a checksum**:
  `r = (i>>8)&0xFF`, `g = i&0xFF`, `b = 0xA5 ^ ((i*7)&0xFF)`.
- `tools/tear-probe/analyze.py` decodes each captured pixel back to a content
  row. For a frame, `D(y) = BAND*i(y) - y` is constant (within the band ramp) if
  the whole frame came from ONE scroll/animation position. **Two plateaus of `D`
  = a tear, and the step between them is its magnitude in pixels.**
- A blended pixel — subpixel scroll edge, scaling, a cross-fade, chrome —
  **fails the checksum and is discarded rather than mis-decoded.** The analyzer
  can refuse, which is the point.
- `scripts/web-tear-probe.sh` runs one A/B arm end to end: fresh sandbox GUI
  with the arm's env, loopback fixture, OSC 7717 `web-surface` declare, load
  driver, frame burst, decode, number.
- `scripts/underglass-sandbox.sh` gained `burst`, `scroll`, `backend`,
  `--under-glass`, `--env` and a daemon-scoped `stop` sweep.

**The detector was proven before it was trusted.** Five synthetic frames with
hostile random-noise "chrome" around a page region:

| synthetic frame | expected | reported |
|---|---|---|
| clean | no seam | `seams=0 torn_rows=0 max_dev=2` |
| horizontal tear, 37 px apart | TORN, 37 | `TORN seams=1 torn_rows=480 max_dev=37` |
| horizontal tear, **4 px** apart (one band) | TORN, 4 | `TORN seams=1 torn_rows=120 max_dev=4` |
| vertical seam (left half old, right half new) | 100 split rows | `split=100` |
| 100 unpainted rows mid-page | 100 unpainted | `unpainted=100` |

So the sensitivity floor is **one band (4 px)**, and vertical seams and
unpainted flashes are separately counted.

Every arm also carries a **load gauge**: `distinct_positions`, the number of
distinct content positions across the burst. An arm whose frames all show the
same position never reached the renderer, and the driver *refuses* to let its
zero-tear count stand as evidence (field guide §7.3).

## 3. What was measured

**Environment parity, checked rather than assumed.** dev and guihost both run
WebKitGTK **2.52.5** and Mesa **26.1.5**; every arm ran the same yggterm
**2.12.19** binary. Every arm asserts its own backend before it measures, from
the GUI's in-process startup trace (never `/proc/<pid>/environ`, per field guide
§7.4) — `gdk_backend=wayland` on all of them, with XWayland *disabled in the
sandbox compositor* so an X client could not have started even by accident.
That matches the live GUI, whose trace reads
`gdk_backend=wayland, policy=kde_wayland_native_default`.

### dev (Intel `iris`, headless sway) — 940 frames

| arm | variable | frames | distinct positions | **torn** | max_dev_px | unpainted |
|---|---|---|---|---|---|---|
| `baseline-anim3d` | `translate3d` compositing layer | 60 | 60 | **0** | 2 | 0 |
| `anim2d` | `top:` repaint path, no layer | 60 | 59 | **0** | 2 | 0 |
| `scroll` | real keyboard scroll through the compositor | 60 | 60 | **0** | 2 | 0 |
| `jsscroll` | rAF-driven document scroll | 40 | 40 | **0** | 2 | 0 |
| `noglass-anim3d` | under-glass **0** | 60 | 59 | **0** | 2 | 0 |
| `noglass-anim2d` | under-glass **0** | 60 | 58 | **0** | 2 | 0 |
| `noglass-scroll` | under-glass **0** | 60 | 46 | **0** | 2 | 0 |
| `shm-anim3d` | `WEBKIT_DISABLE_DMABUF_RENDERER=1` | 60 | 57 | **0** | 2 | 0 |
| `shm-anim2d` | `WEBKIT_DISABLE_DMABUF_RENDERER=1` | 60 | 59 | **0** | 2 | 0 |
| `softgl-anim3d` | `YGGTERM_FORCE_SOFTWARE_GL=1` | 60 | 60 | **0** | 2 | 0 |
| `softgl-anim2d` | `YGGTERM_FORCE_SOFTWARE_GL=1` | 60 | 60 | **0** | 2 | 0 |
| `heavy-scroll` | expensive per-frame paint | 60 | 35 | **0** | 2 | 0 |
| `heavy-anim2d` | expensive per-frame paint | 60 | 58 | **0** | 2 | 0 |
| `always-anim3d` | **`WEBKIT_FORCE_COMPOSITING_MODE=1`** | 60 | 60 | **0** | 2 | 0 |
| `always-heavy-scroll` | **`WEBKIT_FORCE_COMPOSITING_MODE=1`** | 60 | 60 | **0** | 2 | 0 |
| `vblank-heavy-scroll` | `WEBKIT_FORCE_VBLANK_TIMER=1` | 60 | 60 | **0** | 2 | 0 |
| `nocomposite-anim3d` | `WEBKIT_DISABLE_COMPOSITING_MODE=1` | — | — | REFUSED | — | — |
| `nocomposite-scroll` | `WEBKIT_DISABLE_COMPOSITING_MODE=1` | — | — | REFUSED | — | — |

⚠ **`WEBKIT_DISABLE_COMPOSITING_MODE=1` breaks the web surface outright** — the
probe page never appeared at all, twice. That is a separate finding, not a tear
measurement, and it means the documented top-precedence GL escape hatch
(`docs/optimization-pass.md:222`) currently costs the user every web surface.
Not filed in `docs/pending-bugs.md` from this lane to keep the merge clean; it
is a real bug and wants an entry.

### guihost (AMD `radeonsi`, the user's own GPU, isolated headless sway) — 250 frames

Run in a private compositor with a private `HOME`/`YGGTERM_HOME` and the D-Bus
refusal address, so it never touched the user's session; the user's GUI (pid
313747) was verified alive and unmodified before and after, and every sandbox
process was swept afterwards by `YGGTERM_HOME` match.

| arm | frames | distinct positions | **torn** | max_dev_px | unpainted |
|---|---|---|---|---|---|
| `guihost-jsscroll` | 50 | 50 | **0** | 2 | 0 |
| `guihost-anim3d` | 50 | 50 | **0** | 2 | 0 |
| `guihost-anim2d` | 50 | 50 | **0** | 2 | 0 |
| `guihost-glass-jsscroll` (under-glass **1**) | 50 | 50 | **0** | 2 | 0 |
| `guihost-heavy-jsscroll` | 50 | 50 | **0** | 2 | 0 |

**21 arms, 1,190 frames, zero torn frames, zero vertical seams, zero unpainted
rows**, with a detector that catches a 4-pixel seam and a load gauge proving the
renderer was working in every arm.

## 4. What that rules out, and how

### 4.1 Content tearing — FALSIFIED

The brief's framing was: *"true scanout tearing is almost certainly impossible
here, so what the user sees is intra-window CONTENT tearing: a single composited
frame containing a half-updated page."*

A `wl_surface` commit is atomic, so `grim` copying the composited output can
only ever show whole client buffers — which means a torn `grim` frame is exactly
and only a content tear. **1,190 such frames on the user's own GPU, engine
version, Mesa version and binary produced none.** Content tearing is falsified,
including under the two stacking modes, both renderer paths, both GL paths, both
compositing-mode policies, and an expensive-paint load.

### 4.2 Scanout tearing — STRUCTURALLY IMPOSSIBLE for this window

Not argued from defaults — read off guihost:

- KWin **does** implement tearing: `wp_tearing_control_v1` appears **3** times in
  `kwin_wayland`/`libkwin.so.6`. So "the compositor cannot tear" is false.
- **Nothing in yggterm's stack can ask for it.** `wp_tearing_control_v1` /
  `tearing_control` appears **0** times in `libgdk-3.so.0`, **0** times in
  `libwebkit2gtk-4.1.so.0`, and **0** times in Mesa's EGL/GBM.
- No `vblank_mode` / `__GL_SYNC_TO_VBLANK` anywhere in the GUI process
  environment.
- `~/.config/kwinrc` `[Compositing]` carries only `OpenGLIsUnsafe=false`; the
  panel is eDP-1 at **60.003 Hz**; `vrrPolicy` is `Automatic` on one output and
  `Never` on the other.

A surface that never requests async presentation is presented at vblank. The
yggterm window cannot tear on scanout.

### 4.3 The `hardware_acceleration_policy = ON_DEMAND` lead — TESTED, NOT THE CAUSE

`WebKitSettings::set_hardware_acceleration_policy` is indeed never called in
production (only in `docs/spikes/phase-f-under-glass/src/main.rs:137`), so the
policy is WebKit's default `ON_DEMAND`. **This did not need a rebuild to test:**
the installed WebKitGTK 4.1 exports `WEBKIT_FORCE_COMPOSITING_MODE`, which is
the same switch `HardwareAccelerationPolicy::Always` flips. Both `always-*` arms
are as clean as their `ON_DEMAND` counterparts. Setting `Always` is not a fix for
this symptom, and shipping it would be a change with no evidence behind it.

### 4.4 The under-glass "recent change" lead — the premise was stale

The brief said under-glass is armed (`=1`) on the live host. **It is not, and the
authority is the GUI's own startup trace:**

| GUI pid | started | `web_surface_under_glass` | `webkit_gl_policy` |
|---|---|---|---|
| 155552 | 11:52:52 | `0` | `hardware_gl_probed` |
| 190181 | 12:15:49 | **`1`** | `hardware_gl_probed` |
| 204893 | 12:29:38 | **`1`** | `hardware_gl_probed` |
| **313747** | **13:40:08** | **`0`** | **`hardware_gl_forced`** |

`under_glass_default_armed` (`apps/yggterm/src/main.rs:4569-4615`) arms only on an
explicit `YGGTERM_WEB_SURFACE_UNDER_GLASS=1`; the 13:40 restart did not inherit
it, so the GUI the user is looking at **now** is on the legacy opaque stack. It
also reads `hardware_gl_forced`, not `hardware_gl_probed`, because
`YGGTERM_ENABLE_WEBKIT_COMPOSITING=1` is in the launcher environment and takes
precedence over the probe — so on the live host **the GL probe never runs.**

⚠ **This matters for the report itself:** the user's tearing report may have been
made while under-glass was armed (12:15–13:40) and the GUI has since restarted
without it. Whoever relays this must ask whether the symptom is still present
*right now*, because the answer changes which window we are talking about. In
any case `guihost-glass-jsscroll` measured under-glass `=1` on guihost's own GPU and
found nothing.

## 5. What remains, ranked

Both technical tearing mechanisms are excluded, so the symptom is something the
user *calls* tearing. Two candidates survive, and neither is proven.

1. **Judder / dropped frames / uneven frame pacing during scroll and animation.**
   This is what most users mean by "tearing" when there is no tear: content
   advancing in irregular jumps looks like it is ripping. guihost is a laptop with
   `pswpin` in the hundreds of millions, running many agent PTYs, WebGL xterm
   surfaces and several webviews at once. **This instrument cannot see it** —
   `grim` samples at ~100 ms, far coarser than a 16.7 ms frame, so it measures
   *what is in a frame*, never *when frames arrive*. Measuring it needs a
   presentation-timing probe, not a screenshot decoder.
2. **Chrome-versus-page desync from yggterm's own embedding.** Code evidence, not
   speculation: there is **zero** frame-clock integration in the product —
   `gtk_widget_add_tick_callback` and `GdkFrameClock` have 0 occurrences across
   `apps/`, `crates/`, `vendor/dioxus-desktop` and `vendor/wry`. Redraw is manual
   `queue_draw()`/`queue_resize()`, the page rect is reconciled on a free-running
   300 ms tokio tick with a 16 ms change beat
   (`WEB_SURFACE_RECONCILE_TICK_MS`/`_BEAT_MS`, `crates/yggterm-shell/src/shell.rs:3622,3628`),
   and reveal uses a fixed-millisecond ladder `[0,8,16,32,64,120,240,480] ms`.
   None of those cadences is a multiple of the panel's 16.666 ms frame, so
   repositioning and reveal work lands mid-frame by construction. That is a
   plausible mechanism for "the page and the chrome ripped apart for a moment"
   during a *shell* animation — but it does not obviously fire during a plain
   page scroll, and **nobody has observed it**. It is a hypothesis with code
   evidence and no symptom evidence.

## 6. What would settle it

In priority order. The first item is worth more than everything else here.

1. **Ask the user one question: does the same tearing happen in a window that is
   not yggterm?** A plain Firefox/Chromium window scrolling the same page, or
   KDE's own overview animation. If yes, the cause is the compositor/panel and
   not yggterm at all, and this whole lane is misdirected. If no, it is our
   surface. Ten seconds of the user's time replaces days of ours.
2. **Ask whether it is still happening on the current GUI** (pid 313747, started
   13:40, under-glass `0`) or whether the report predates that restart.
3. **Ask what it looks like**: a horizontal line where the picture is offset (a
   true tear) versus the content lurching/stuttering (judder). Those have
   disjoint causes and the word does not distinguish them.
4. **Run the probe on the user's real screen.** The decoder works on any PNG, so
   the protocol is: serve `tools/tear-probe/page.html`, open
   `http://127.0.0.1:<port>/page.html?mode=jsscroll&band=4` in *their* ychrome,
   capture the full screen while it animates (`spectacle -b -n -f -o out.png`),
   and run `tools/tear-probe/analyze.py out.png --band 4`. That is the one
   measurement that covers the real KWin + real KMS scanout path this
   investigation could not reach. **It requires the user's own hands** — driving
   their viewport is off-limits to an agent.
5. **If it is judder**, build a presentation-timing probe instead: a page that
   records `requestAnimationFrame` deltas and reports the p50/p99 and the dropped
   frame count, read back over `server app web eval`. That measures *when*, which
   is the axis this instrument does not have.

## 7. Honest limits of what was proven

- The instrument sees **content tearing only**. Scanout tearing is invisible to
  it on any compositor, and so is frame pacing.
- The sandbox compositor is **sway (headless)**, not KWin on a real KMS output.
  Damage tracking, presentation scheduling and direct scanout all differ. The
  guihost arms remove the GPU and driver from the difference list; they do not
  remove the compositor or the display pipeline.
- The probe page is synthetic. A real site brings sticky headers, iframes, video
  and its own composited layers. The `heavy` mode raises paint cost but does not
  reproduce that structure.
- `distinct_positions` proves the renderer moved between captured frames. It does
  **not** prove the animation ran at the panel's refresh rate.
- The two `nocomposite-*` arms are REFUSALS, not zeros. They measured nothing
  because the surface never appeared.

## 8. The one production line this lane touched

`crates/yggterm-core/src/install.rs:1641-1663` — `web-tear-probe.sh` added to the
named exemption list in `no_shipped_script_decides_the_webkit_gl_path`. That lock
scans `scripts/` for anything that SETS `WEBKIT_DISABLE_COMPOSITING_MODE`, because
the §1a lesson was that launchers must stop deciding the GL path; it already
exempts the `gl_ab_*` harnesses on the stated ground that "an A/B whose arms
cannot force their own arm is not an A/B", and this probe is the same category —
a measurement tool, never installed, never on a launch path.

**The exemption was falsified before it was accepted.** Appending
`export WEBKIT_DISABLE_COMPOSITING_MODE=1` to the non-exempt
`scripts/underglass-sandbox.sh` turns the lock RED, naming that file:

```
these scripts still decide the GL path the binary now probes for:
  [".../scripts/underglass-sandbox.sh"]
test result: FAILED. 0 passed; 1 failed
```

and restoring the file turns it GREEN again. So the carve-out is narrow and the
scanner still bites. (Note while doing this: the matcher watches
`WEBKIT_DISABLE_COMPOSITING_MODE` and *nothing else* —
`YGGTERM_FORCE_SOFTWARE_GL` in a script sails straight through. A first mutation
with that variable stayed green and would have been read as "the lock is a hole"
if it had not been chased down.)

## 9. Reproducing the numbers

```bash
# one arm, end to end, prints its number
./scripts/web-tear-probe.sh run --label baseline-anim3d --mode anim3d --frames 60

# the standard matrix
./scripts/web-tear-probe.sh arms --frames 60

# on a GUI host with no numpy/PIL (guihost): capture there, decode elsewhere
YGGTERM_HEADLESS_BIN=~/.local/bin/yggterm YGGTERM_GUI_CLI=~/.local/bin/yggterm \
  ./scripts/web-tear-probe.sh run --label guihost-jsscroll --mode jsscroll --under-glass 0
rsync -a guihost:~/.tmp/yggterm-tear/ /tmp/yggterm-tear/guihost/
python3 tools/tear-probe/analyze.py /tmp/yggterm-tear/guihost/guihost-jsscroll --band 4
```

⚠ **`wlrctl pointer scroll` is a dead end** on wlrctl 0.2.2 with this wlroots:
the axis event is accepted and delivered nowhere. Proven, not assumed — a
`wheel` listener installed in the page counted **0** events across
`scroll 100 0`, `scroll 0 100` and `scroll -15 0`, while a `mousemove` listener
counted the pointer moves from the *same tool in the same run*. Keyboard
scrolling works (~21 px per `Down`, verified against `window.scrollY`) and is
the `scroll` driver. Virtual-keyboard auto-repeat does **not** happen — a 3 s
held `Down` moved `scrollY` by 0 — so it is one key event per step.
