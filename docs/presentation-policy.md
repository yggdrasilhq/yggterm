# The presentation policy — what yggterm runs as, per platform

> **This is the SSOT for display backend, GL, frame delivery and video decode.**
> The table lives in code (`crates/yggterm-core/src/presentation_policy.rs`) so
> it can be tested; this file is the reasoning and the law. If the two ever
> disagree, the code is right and this file is a bug.

## Why this exists

The logic was never missing. `apps/yggterm/src/main.rs` has decided the backend,
the terminal renderer and the GL/DMABuf plan for a long time, each as a pure,
tested function. What was missing was **one place to look up what this product
is supposed to run as** — the decision was three functions in a binary's
`main.rs`, Linux-only, each of which politely steps aside when it finds an
environment variable already set.

That politeness is the hole. An agent sets a variable, the policy defers, and
the GUI is now running as something nobody chose. The user has paid for this
repeatedly:

> *"I have seen you countless times, I changed this flag that flag. No. Stick to
> a default set of flags."*
>
> *"I have seen agents testing on dev on Xvfb and then suddenly deciding to
> restart my yggterm in XWayland and hours lost on bug finding (this has
> happened multiple times)."*

## ⛔ THE LAW

1. **A sanctioned default is not a suggestion.** No agent may set, export or
   "just try" any variable in `PRESENTATION_VARS` against the user's running
   GUI. To test an arm, use the sandbox — `scripts/underglass-sandbox.sh` or
   `scripts/web-tear-probe.sh` — which builds a throwaway GUI with its own env
   and its own daemon. **The user's GUI is not a test rig.**
2. **The user's session decides the backend, not the agent.** A Wayland session
   runs Wayland-native. Forcing X11 gives XWayland, which changes compositing,
   input latency and the terminal renderer at once, and every measurement taken
   on that GUI afterwards describes a machine the user does not run.
3. **The headless row does not travel.** Xvfb and headless sway ARE X11, so
   `GDK_BACKEND=x11` is correct *there*. Carrying that lesson onto the user's
   Wayland desktop is the specific, repeated mistake. Learning a flag under Xvfb
   tells you nothing about their machine.
4. **`/proc/<pid>/environ` CANNOT tell you what is in force.** Every one of
   these is applied with `set_var` after exec, so the process environment shows
   the *launch* env and nothing later. Read the
   `gui/startup/linux_desktop_backend_policy` trace event — the decision
   reporting itself. This has misled at least two investigations, including one
   in this very session.
5. **Deviations must be loud.** `presentation_policy::deviations()` compares a
   reported environment against the table and names the variable, the value and
   the consequence. "Why is my GUI behaving strangely" should be one lookup.

## The table (Linux, the only platform that builds today)

### `linux-wayland` — the user's desktop

| variable | value | why, in one line |
| --- | --- | --- |
| `GDK_BACKEND` | `wayland` | Must be set EARLY and explicitly: the vendored dioxus `app.rs` forces `x11` whenever it finds this unset. |
| `WINIT_UNIX_BACKEND` | `wayland` | The window layer's half; split from GDK it can disagree with the toolkit. |
| `LIBGL_ALWAYS_SOFTWARE` | **absent** | Present ⇒ every frame rasterises on the CPU, at 4x–22x the frame cost. Inherited from probe harnesses. |
| `GALLIUM_DRIVER` | **absent** | `llvmpipe` here is software GL wearing the GPU's name. |
| `WEBKIT_DISABLE_COMPOSITING_MODE` | **absent** | Setting it **breaks the web surface outright**. It is not a performance knob. |
| `WEBKIT_DISABLE_DMABUF_RENDERER` | **absent** | The zero-copy path frames and video travel on. Disabling it is invisible in a screenshot and shows up only as heat and judder. |
| `YGGTERM_WEB_SURFACE_UNDER_GLASS` | `1` | The current default, AND what keeps the vendored DMABuf-disable from firing. These are ONE decision, not two. |
| `GST_PLUGIN_FEATURE_RANK` | `vah264dec:MAX,vah265dec:MAX,vavp9dec:MAX,vaav1dec:MAX` | Hardware video decode — see §Judder below. |
| `YGGTERM_ENABLE_XTERM_CANVAS` | `1` | xterm.js's WebGL renderer; only presents with accelerated compositing on, so it is downstream of the GL rows. |

### `linux-x11` — a real X11 session

`GDK_BACKEND=x11` (legitimate here), `YGGTERM_ENABLE_XTERM_CANVAS=0`,
`WEBKIT_DISABLE_COMPOSITING_MODE` absent. The mistake is X11 on a *Wayland*
session, not X11 on an X11 one.

### `linux-headless` — Xvfb / headless sway, CI and sandboxes

`GDK_BACKEND=x11`, `YGGTERM_ENABLE_XTERM_CANVAS=0`. ⚠ **This row is the one that
gets carried onto a user's machine by mistake.** It is correct here and wrong
there.

### Windows, macOS, Android, iOS

**Deliberately empty.** Windows and macOS do not build yet
(`docs/pending-bugs.md` §3.0.0) and mobile is further out. An empty row means
"undecided", never "the defaults are fine" — claiming a default for a platform
nobody has run is how a wrong default ships and then gets defended. Fill the row
when the platform builds, with a measurement, not a guess.

## §Judder — the 2026-08-01 finding that produced the decode row

The user reported a YouTube video lurching and dropping frames in an ychrome
surface, with no tearing in other browsers. Three hypotheses died in order:

1. **Under-glass / compositing.** Dead: judder is not a tear, and a compositor
   fault would show in other windows too.
2. **XWayland.** Dead, and measured rather than assumed — the GUI held **two
   connections to `/run/user/1000/wayland-0` and zero X11 sockets**. (`xwininfo`
   returned nothing at all including its own sanity check, so that instrument
   was blind, not negative. Do not read an empty `xwininfo` as "not X11".)
3. **Mis-armed GL.** Dead: the startup trace reported `gdk_backend=wayland`,
   `gl_probe_class=hardware`, `gl_probe_driver=radeonsi`,
   `libgl_always_software=None`, `webkit_disable_dmabuf_renderer=None`. The
   stack was armed correctly.

What was actually true: **both decoders were loaded into the video WebProcess.**
`libgstva.so` (hardware, 10 mappings) and `libgstlibav.so` (software ffmpeg)
side by side, with `libva.so.2` and the radeonsi `libgallium` mapped in, while
the process burned a steady **58–61% of one core** (two 2 s samples — not the
lifetime average, which lies). GStreamer picks between them by **rank**, and
nothing was ranking the hardware decoders up.

⚠ **Still not fully proven**, and the honest gap is worth stating: loaded is not
the same as selected. `gst-inspect-1.0` is not installed on the live host, so
the decoder actually chosen for that stream was never read directly. The rank
default is the standard remedy and the evidence is strong, but the closing
measurement is a `GST_DEBUG` capture on a controlled page, or a CPU re-measure
of the same video after this default lands.

## How to check a live host against the table

```
# the decision, reporting itself (NOT /proc/environ)
grep linux_desktop_backend_policy ~/.yggterm/event-trace*.jsonl | tail -1

# is it really Wayland-native? sockets, not xwininfo
ls -l /proc/<gui-pid>/fd | grep -c X11          # must be 0
ss -xp | grep wayland-0                          # must show the GUI

# is hardware decode loaded in the video process?
grep -c libgstva /proc/<webprocess-pid>/maps
```

Related: `docs/optimization-pass.md` (the optimization SSOT),
`docs/web-surface-tearing-2026-07-31.md` (the tearing instrument and what it
falsified), `docs/pending-bugs.md` §3.0.0 (why Windows/macOS rows are empty).
