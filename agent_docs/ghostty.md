# Ghostty Embedding Notes

Updated against local checkout:

- Ghostty repo: `/home/pi/gh/ghostty`
- Local Ghostty HEAD after pull: `c9e100621`
- Date: 2026-03-19

## Executive Summary

Ghostty is not yet a clean, portable "drop this widget into any app" terminal embedding solution.

The upstream direction is now clearly split into two layers:

1. `libghostty-vt`
   - This is the part Ghostty is actively turning into a reusable cross-platform library.
   - It handles terminal parsing, terminal state, scrollback, input encoding, formatting, modes, etc.
   - It already has a C API and examples.
   - It is usable today, but explicitly unstable.

2. full `libghostty`
   - This is still effectively the Ghostty app/runtime ABI.
   - It is heavily used by Ghostty's macOS app.
   - It is not yet documented or packaged as a generally reusable embedding API.
   - Linux does not currently expose a stable embeddable surface API in the same way.

For Yggterm, that means:

- macOS has a plausible future path around full `libghostty`.
- Linux does not yet have a real embedded Ghostty surface path we should rely on.
- the safest near-term architecture is:
  - `yggterm-server` owns sessions/PTY/runtime
  - Ghostty external process remains the Linux terminal host fallback
  - we study whether `libghostty-vt` can be used for preview/render-side or custom host work later
  - we avoid promising true Linux embedding until upstream makes it real

## What Upstream Says

### README

Ghostty's README now states:

- Ghostty is breaking `libghostty` into smaller reusable libraries, starting with `libghostty-vt`.
- `libghostty-vt` is already available for Zig and C.
- `libghostty-vt` targets macOS, Linux, Windows, and WebAssembly.
- the API is not stable yet.
- the ultimate embedding story is not hypothetical because the macOS app itself consumes `libghostty`.

Source:

- `/home/pi/gh/ghostty/README.md`

Relevant section:

- "Cross-platform `libghostty` for Embeddable Terminals"

### Build System

The build file is more conservative than the README marketing summary:

- when `app_runtime == .none`, Ghostty builds `libghostty`
- comment says `libghostty` is not stable for general purpose use
- comment says it is heavily used on macOS but "isn't built to be reusable yet"
- on Darwin, the build mainly produces an XCFramework
- on non-Darwin, shared/static library artifacts are installed, but that does not mean the API is ready for general embedding

Source:

- `/home/pi/gh/ghostty/build.zig`

This is the most important engineering reality check in the repo.

## What `libghostty-vt` Actually Is

### Public positioning

`include/ghostty/vt.h` describes `libghostty-vt` as:

- a virtual terminal emulator library
- focused on parsing terminal escape sequences
- terminal state maintenance
- cursor/screen/scrollback/style handling
- input encoding
- formatting
- explicitly incomplete and unstable

Source:

- `/home/pi/gh/ghostty/include/ghostty/vt.h`

### Capabilities already exposed

`src/lib_vt.zig` and the C headers show that the reusable terminal-core layer already includes:

- terminal state object
- terminal resize
- VT byte stream ingestion
- viewport scroll
- mode get/set
- formatter support
- key encoding
- mouse encoding
- focus encoding
- paste helpers
- size report encoding
- parser and formatting APIs

Important sources:

- `/home/pi/gh/ghostty/src/lib_vt.zig`
- `/home/pi/gh/ghostty/include/ghostty/vt/terminal.h`
- `/home/pi/gh/ghostty/example/README.md`

### Important limitation

The current `ghostty_terminal_vt_write(...)` C API is explicitly read-only with respect to terminal responses and side effects:

- it processes VT input and updates terminal state
- sequences that require output/response are ignored
- the docs say future callback APIs are needed for output and side-effect sequences

This matters because a full terminal embedding story eventually needs:

- DSR/DECRPM/CSI responses
- clipboard interactions
- title changes
- OSC behavior
- query/response loops
- high-performance render-state access

That work is still underway.

## What Full `libghostty` Is Today

`include/ghostty.h` says the quiet part out loud:

- the embedding API docs are only in Zig source files
- it is not meant to be a general purpose embedding API yet
- the only consumer is the macOS app
- the API is "built to be more general purpose", but not there yet

Source:

- `/home/pi/gh/ghostty/include/ghostty.h`

The platform enum in that header is also telling:

- `GHOSTTY_PLATFORM_MACOS`
- `GHOSTTY_PLATFORM_IOS`

No Linux or Windows platform enum is present there today.

That strongly suggests:

- full `libghostty` embedding is still a Darwin-first surface/runtime ABI
- Linux/Windows are not first-class consumers of the same public surface contract yet

## Linux Runtime Reality

Ghostty's Linux app is GTK-based, and its "surface" concept on Linux is tied to the GTK application runtime.

Relevant files:

- `/home/pi/gh/ghostty/src/apprt/gtk/Surface.zig`
- `/home/pi/gh/ghostty/src/apprt/gtk/class/surface.zig`
- `/home/pi/gh/ghostty/src/main_ghostty.zig`

Important observations:

1. Linux is implemented as a Ghostty-owned GTK runtime, not a small embeddable widget library.
2. `apprt/gtk/Surface.zig` is runtime glue around Ghostty's own GTK class/surface model.
3. the GTK surface wrapper assumes a fully initialized Ghostty surface/core relationship.
4. this is internal application-runtime plumbing, not a documented consumer-facing embed API.

So on Linux today, "embed Ghostty" really means one of these:

- launch external Ghostty windows
- fork/adapt Ghostty's GTK surface internals and accept upstream friction
- wait for upstream to expose a stable embeddable surface host
- build a custom terminal host around `libghostty-vt` plus our own renderer/input/PTY integration

The second option is the most dangerous. It looks tempting because the code exists, but it is not shaped for reuse.

## macOS Runtime Reality

macOS is the strongest future embedding path.

Why:

- upstream explicitly says the native macOS app is a `libghostty` consumer
- `main_ghostty.zig` is process initialization code for the `ghostty` app and `libghostty`
- Darwin build targets explicitly include XCFramework packaging

Implication:

- if we want a real embedded Ghostty host in Yggterm soonest, macOS is the most plausible platform
- however, even there we should treat the API as unstable and upstream-internal first

## Windows Reality

`libghostty-vt` claims Windows compatibility.

That does **not** mean full Ghostty UI embedding on Windows is ready.

Practical interpretation:

- terminal-core logic is becoming portable
- full UI/runtime embedding is still not exposed as a reusable Windows story
- pausing Windows releases in Yggterm was the right call

## Upstream Activity Worth Watching

Recent GitHub activity strongly confirms that upstream is building out the reusable terminal-core layer first.

### Pull requests

- PR #11506
  - https://github.com/ghostty-org/ghostty/pull/11506
  - Added initial C API for terminal + formatter
  - Explicitly says formatter is **not** a rendering API
  - Explicitly says future work includes callback systems and `terminal.RenderState` C API

- PR #11579
  - https://github.com/ghostty-org/ghostty/pull/11579
  - Added terminal mode query/set and DECRPM report encoding to the C API

- PR #11577
  - https://github.com/ghostty-org/ghostty/pull/11577
  - Added focus encoding exposure in C/Zig APIs

- PR #11553
  - https://github.com/ghostty-org/ghostty/pull/11553
  - Added mouse encoding Zig + C API

- PR #11607
  - https://github.com/ghostty-org/ghostty/pull/11607
  - Extracted size report encoding

- PR #11609
  - https://github.com/ghostty-org/ghostty/pull/11609
  - Ensured libghostty C docs examples build/run in CI

- PR #11089
  - https://github.com/ghostty-org/ghostty/pull/11089
  - Parser fuzzing for `libghostty-vt`

- PR #8895
  - https://github.com/ghostty-org/ghostty/pull/8895
  - Early shared-library boilerplate + custom allocator API for `libghostty-vt`

- PR #8840
  - https://github.com/ghostty-org/ghostty/pull/8840
  - `ghostty-vt` Zig module

### Issues / platform-path signals

- Issue #11011
  - https://github.com/ghostty-org/ghostty/issues/11011
  - visionOS/xros embedded platform path
  - shows upstream is willing to evolve embedding, but on Apple platforms first

- Issue #11223
  - https://github.com/ghostty-org/ghostty/issues/11223
  - stale-frame resize guard issue for embedded runtime
  - evidence that "embedded runtime" is real upstream terminology, but still not a polished cross-platform reusable surface story

## What This Means For Yggterm

## 1. We should separate "terminal core" from "terminal host"

This is already consistent with our current crate direction:

- `yggterm-server`
- `ghostty-bridge`
- `platform`
- `ui`

We should preserve that separation.

Practical interpretation:

- terminal core
  - session state
  - PTY ownership
  - attach/restore
  - metadata
  - remote orchestration
  - previews
  - eventual `libghostty-vt` experiments

- terminal host
  - actual on-screen terminal surface
  - input -> terminal encoding
  - render loop
  - clipboard/paste integration
  - selection
  - IME/text input

## 2. Linux should stay on external Ghostty for now

This is still the lowest-risk path.

Reasons:

- upstream does not expose a stable Linux embeddable surface API
- GTK internals exist but are not a reusable SDK
- trying to vendor/fork GTK surface internals would create a maintenance trap

So the current `linux-gtk-glue` / external Ghostty host fallback is still the correct production assumption.

## 3. macOS is the likely first true embedding target

If we decide to pursue real embedded Ghostty next, macOS should probably go first because:

- Ghostty already uses `libghostty` there
- the upstream path exists conceptually and in production
- XCFramework packaging is part of the build story

This does **not** mean it will be easy. It just means it is the least speculative embed path.

## 4. `libghostty-vt` is promising, but not enough by itself

We should not confuse:

- "reusable terminal emulator core"

with:

- "drop-in full terminal widget"

`libghostty-vt` is already becoming the first one.
It is not yet the second one.

If we try to build our own embedded terminal using `libghostty-vt`, we will still need to own:

- PTY IO loop
- callbacks/output handling for queries and side effects
- rendering
- font shaping/rendering
- selection
- cursor blinking
- clipboard
- IME/input method integration
- accessibility
- platform windowing

That is a large product in itself.

## 5. Best near-term Yggterm plan

My current recommendation is:

### Near-term

- Keep Linux terminal execution on external Ghostty windows launched/focused by `yggterm-server`
- Strengthen attach/restore/session ownership
- Improve handoff between the Dioxus shell and active Ghostty windows
- Make terminal mode in Yggterm clearly represent live session state even before full embedding

### Medium-term

- Study macOS `libghostty` embedding as the first real embedded host experiment
- Track upstream `libghostty-vt` C API growth, especially:
  - callbacks for side-effect sequences
  - render state API
  - better terminal introspection

### Long-term

- Reassess Linux embedding only when upstream either:
  - exposes a stable embeddable Linux surface host, or
  - `libghostty-vt` grows enough surface-oriented APIs that a custom Yggterm host becomes realistic

## Specific Questions We Still Need To Answer

1. How tightly is the macOS `libghostty` API coupled to Ghostty's app/global runtime state?
2. Can Yggterm host multiple embedded surfaces from one `libghostty` app instance cleanly?
3. Is upstream planning a renderer-facing C API (`RenderState`) soon, or is that still speculative?
4. Would upstream accept work toward a Linux embeddable surface host, or do they prefer external consumers to wait?
5. Is there any serious Windows host plan beyond `libghostty-vt` portability?

## Concrete Next Research Steps

1. Read the macOS `libghostty` consumer path in the Ghostty app and map:
   - app init
   - surface lifecycle
   - input
   - render callback flow

2. Audit GTK `class/surface.zig` and related renderer code to estimate how hard a Linux embed fork would be.

3. Watch these upstream areas closely:
   - `libghostty-vt` PRs
   - render-state API work
   - embedded runtime issues
   - Apple-platform embedding changes

4. Keep Yggterm's daemon/session model independent enough that the terminal host can change per platform.

## Bottom Line

The practical embedding story today is:

- macOS: plausible but unstable
- Linux: not really there yet
- Windows: terminal-core portability only, not host embedding

So the disciplined move for Yggterm is:

- do not overcommit to Linux embedded Ghostty yet
- keep the server/session model moving
- treat external Ghostty as the Linux production path for now
- use macOS as the first real embedded-host experiment when we decide to spend that effort
