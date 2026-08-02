# Archived bug narratives — closed on or before 2026-08-02

**This is history, not a queue.** Nothing here is open; every entry below was
verified fixed, shipped, or superseded. It is kept because the reasoning is
often the only record of WHY something is shaped the way it is, and it is kept
OUT of `pending-bugs.md` because a queue that lists dead work is a queue nobody
can trust. See [`../docs-ssot.md`](../docs-ssot.md).

Search it, do not read it: `rg -n '<what you remember>' docs/archive/`


## ⭐ A revealed web surface kept its OLD size

⭐ A revealed web surface kept its OLD size — FIXED AND VERIFIED LIVE ON 2.12.24 (2026-08-01)

User report, with a screenshot of a yRDP desktop sitting 132 px to the right of
where it belongs and its far edge cut off: *"My viewport does not auto resize…
hiding cwdtree does not repin/scale to the new viewport size."* Reached by
hiding the cwd tree, switching INTO the surface's session, showing the tree
again, and switching back.

**Measured, not inferred.** The page itself is the only witness — `app
screenshot`'s default backend cannot see native children, and the reconciler's
applied map holds the rect the shell ASKED for:

```
yggterm server app web eval --session live::<uuid> --script 'innerWidth'
→ 1665      # the viewport width with the cwd tree HIDDEN
          # while `dom.main_surface_body_rect` said 1400 and the page's own
          # x had already followed the tree back to 269
```

A rect that never existed as a measurement (fresh x, stale width) proved the
shell had pushed the right rect and GTK had taken only half of it.

**Root cause: geometry written to a HIDDEN webview is not merely ignored, it
poisons the next write.** GTK drops `size_allocate` on an invisible widget, and
a `WebKitWebViewBase` answers `get_preferred_width` with its own current view
size — so once the view is wider than its size request, every later layout pass
still reads the larger natural size. Growing is free; shrinking needs an
allocation the widget can process. `WebSurfaceHost::unstash` placed the surface
BEFORE showing it, so a reveal after the window narrowed dropped the new size
and the shell's change-gate (`entry.bounds != rect`) then suppressed every
retry: the surface was wrong for the rest of its life.

**Fix (`vendor/dioxus-desktop/src/web_surface.rs`):** `apply_bounds` records the
rect always and writes it only to a visible view; `unstash`, `set_visible(true)`
and `set_throttled(false)` show first and place after. Locked structurally by
`engine_visibility_locks::a_revealed_surface_is_placed_after_it_is_shown_not_before`
(all three mutations proven red), and the GTK behaviour itself is measurable
with `scripts/webview-shrink-probe.py`, which replays all eight paths.

✅ **VERIFIED LIVE ON JOJO, 2.12.24 (2026-08-01, 23:2x).** Driven end to end in a
throwaway shadow client (`scripts/shadow-client.sh`) against a scratch Xvfb+VNC
target, replaying the user's own sequence and reading the page each step:

| step | page `innerWidth` | `main_surface_body_rect.width` |
|---|---|---|
| revealed, tree shown | 1396 | 1396 |
| tree hidden while the surface was BACKGROUNDED, then switched back | 1661 | 1661 |
| tree shown while watching | 1396 | 1396 |
| switched away and back again | 1396 | 1396 |

Row three is the one that used to stick at 1661 for the life of the surface.
The instrument, for the next time: `server app web eval --session <path>
--script 'innerWidth'` against `.data.dom.main_surface_body_rect.width`.


## ⭐ GATE BUG #2

**⭐ GATE BUG #2 — "Restoring Remote Terminal" holds a BLANK viewport over a
session that is already running. ROOT-CAUSED AND FIXED IN CODE 2026-08-01
(`lane/dev/gate-bugs`), NOT YET LIVE-VERIFIED.** A DIFFERENT gate from the
handover veil fixed in 2.12.23. The toast said *"The viewport will switch in
once the session is truly interactive"* while the metadata pane beside it read
**`Status: running · working`**, PTY 174x65, a live PID.

**The root cause is not the predicate. It is that the ceiling was attached to
the wrong thing.** `terminal_live_host_connected` starts false for every
remote resume and is the client's belief that the session is interactive;
while it is false the mount disables input, keeps the toast up, and one
recovery path `terminal_reset_command`s the viewport to blank on the way in.
Its only ceiling was the 60 s `REMOTE_TERMINAL_RESUME_FAIL_MS` timer, and that
timer is armed **once per BOOTSTRAP IDENTITY** — while the gate is re-armed
from **inside the terminal read loop** (retained-empty-surface recovery,
dead-resume-instruction recovery, the non-prompt wait, the post-write-error
retry), none of which change the bootstrap identity. **Every re-arm after the
first was uncapped.** Falsified along the way: this is not a signal a remote-CC
session on an older daemon fails to emit — jojo's own traces show the identical
session kind reaching `attach_ready` in ~1 s, repeatedly, across the daemon
version split. The gate is version-blind; it is the *hold* that had no bound.

**Four holes, all closed, `crate::resume_gate` now owning the decisions:**
1. **No ceiling on the gate itself.** A watchdog now samples the gate on its
   own wall clock (2 s poll, 90 s continuous-hold ceiling) and releases it —
   believe the session, drop the toast, enable input.
2. **The 60 s timer's deferral consumed the ceiling.** `deferred_for_output_
   progress` did `return`, and no replacement timer is ever armed for that
   mount. It now re-checks (bounded by
   `REMOTE_TERMINAL_RESUME_TIMEOUT_MAX_DEFERRALS`) instead of dying.
3. **The timer read "this toast should not be visible" and did nothing.** With
   `still_waiting_for_resume == false` it silently no-oped, leaving the 1.2 s
   slow toast up with nothing else able to take it down. It now clears it.
4. **The one wall-clock ceiling on this path made the gate STRICTER.**
   `resume_overlay_timed_out` is set at 60 s and every branch of
   `host_should_accept_input` requires it false, so the "ceiling" permanently
   force-disabled input. The 90 s release now clears it and
   `resume_overlay_failed`, and a lock derives that requirement from the input
   gate's own text so a new latch cannot be added to one side only.

**Two sibling gates of the same shape, also fixed:** the **non-prompt wait**
(`retained_remote_surface_should_wait_for_prompt_ready`) is SELF-FEEDING — its
arm includes `poisoned_by_retry` and entering it sets both halves — and once
its two recovery budgets were spent it re-raised "Restoring Remote Terminal"
every 120 ms with input disabled forever, disarmed only by a prompt TEXT
heuristic a streaming agent frame may never satisfy; it now releases
(`NON_PROMPT_WAIT_MAX_HOLD_MS`, 30 s). And the JS **handover veil** mirror is
cleared only by the terminal read loop, so a loop that breaks while suspended
left an opaque cover over the viewport that the Rust gate's own 90 s ceiling
could never reach; the loop now lifts it on exit.

⚠ **Not live-verified.** 17 mutations proven red across `resume_gate::tests`
and `shell::resume_gate_wiring_locks`; `cargo test -p yggterm-shell --lib` is
green at 1783. After deploy, confirm on jojo by grepping the trace for
`terminal_mount/resume_gate_ceiling` (every hold edge is traced with its
`held_for_ms` and `hold_ceiling_ms`), and for
`retained_non_prompt_surface_wait_released` /
`resume_timeout_cleared_stale_notification` /
`handover_paint_resumed_on_read_loop_exit`. A `released_ceiling` transition in
normal use is itself a bug report: it means some path held the gate for 90 s.


## ★★★ NOTIFICATION AUDIO IS SILENT IN THE WEBVIEW

**★★★ NOTIFICATION AUDIO IS SILENT IN THE WEBVIEW — PROVEN BY A/B ON THE
LIVE HOST, AND THE FIX WAS TO LEAVE WEBKIT (2026-07-26, user-reported
regression: "I used to hear the double chime when copying or when the agent
ended its turn. That is also absent" — ⏳ FIXED IN-TREE AT 2.12.17, DEPLOY
AND A LISTENING CHECK OWED).**
**The A/B that convicted WebKit, same speaker, same sink, minutes apart:**
| path | result |
|---|---|
| WebKit `AudioContext` (the shipped path), `ctx.resume()` added, gains x6, fired seconds after a real user click | **SILENT** |
| native PCM synthesis → platform sink (`pw-play`) | **AUDIBLE**, user confirmed "I heard double chime just now" |
Everything downstream was verified innocent first (default connected sink, a
system test tone clearly heard through it, our own sink-inputs present,
`Mute: no`, `Corked: no`, and PipeWire reporting the sink SUSPENDED →
**RUNNING** for the webview chime). So WebKit opens a real stream and fills
it with silence; the autoplay/gesture gate is the leading theory and an agent
cannot synthesize a qualifying gesture anyway.
**WHAT SHIPPED (in-tree, 2.12.17).** `yggterm server app audio play|tune`
renders the chime to PCM in **native Rust** and pipes it to
`pw-play`/`paplay`/`aplay` — no webview, no GUI, no daemon — so an AGENT can
ring the user, and a chime no longer depends on the user being at the
keyboard. ⚠ The verbs are on the **`yggterm` binary, NOT `yggterm-headless`**;
a missing player is an ERROR naming every binary it looked for, never a
silent success, because silent success is the whole defect class this
answers. `yggterm_core::notification_audio` is the ONE owner of the tune —
two players now exist, and a tune with two owners becomes two chimes, so the
webview script spells neither notes nor envelope and walks the registry's
published breakpoints instead.
**THE TUNE IS THE MEASURED, USER-APPROVED SPEC** (derived from real cabin-
chime recordings by FFT + onset/envelope analysis — **re-measure rather than
re-tune**): meaning is carried by PATTERN (one chime = info/success, hi-lo =
warning, hi-lo x3 = error); the pair is a descending **minor third** with a
**1.03 s** gap, and that slow tempo is most of what makes it calm rather than
urgent; the envelope is a **53-point measured table** with a sustain shoulder
(still 89% of peak at 150 ms) that no exponential reproduces; the TPDF dither
keepalive spans the **whole render**, not just the front, because the tune is
mostly silence by duration and a front-only pre-roll leaves every later note
exposed to a sleepy A2DP sink; **pre-roll 0.70 s, flush tail 1.10 s**;
`--volume` scales the envelope OUTPUT, not the peak fed into it.
⚠ **Instrument note, unchanged and load-bearing:** "the eval returned without
error" is not evidence of sound, and neither is a RUNNING sink. The only
honest instruments here are the user's ears and an A/B against a known-good
native player.
**WHAT IS OWED — this entry stays until it is heard.** 2.12.17 is not
deployed. After the bump, listen for parity with what was approved: a single
that **ends clean** (no resonant aftertaste), a pair that sounds
**unhurried**, the error tone's **later pairs unclipped** (the case the
whole-render dither exists for), and **half volume = the same shape, quieter**
(the exponential model failed exactly here, producing 52% of peak). Also
confirm the GUI-side chime is audible on a real user notification, not just
the CLI verb.
**Not built, deliberately named:** `audio state` (it must report the shell
webview's `AudioContext`, which needs an app-control round trip; the chime
script already records the data at `window.__yggtermChimeAudio`, the
transport is missing) and any `--save` render-to-file. The verb refuses
`state` by name rather than answering with native-side facts under a name
that promises webview ones.
**Until the deploy,** `~/.local/bin/ygg-chime` on the GUI host is still the
only way an agent can ring the user; it was the auditioning surface the
approved tune was measured on, and the product registry now carries that
tune.


## ★★★ WE FORCED SOFTWARE GL ON A HOST THAT HAS WORKING HARDWARE GL

**★★★ WE FORCED SOFTWARE GL ON A HOST THAT HAS WORKING HARDWARE GL. The
premise is fixed and DEPLOYED (2.12.14); ⛔ THE CPU WIN IS NOT ESTABLISHED AND
THE FIRST NUMBERS WERE AN ArecordsFACT. This entry STAYS until a matched-load
measurement settles it — see "what is actually true" below.**
⛔ **Read this before repeating any figure from this entry.** The 18x/5x render
improvements first recorded here compared an evening of real use against an
overnight window with **23x less terminal activity** (`terminal_read` 9.22/min
vs 0.40/min, measured from the same daemon on both sides). The kill shot is
internal to the after-window: **`gpu_ms` was ZERO in 523 of its 532 render
ticks** — the CPU did not move to the GPU, it simply was not being spent
because nothing was painting. At matched exposure the render tree went
**0.297 → 0.264 cores (~11%, n=8 post ticks, low confidence)**.
⛔ **The "regression" half of this entry is WITHDRAWN (2026-07-26).** It said
the plateaued GUI reads **0.358-0.373 cores against a pre-fix evening p50 of
0.297**, and a separate probe called that a **~2.3x GUI-role regression** from
hardware GL. Both sides of that comparison differ in `window_focused`:
**1,194 of the 1,256 `render/gui` rows in the retained corpus are unfocused,
and ALL 1,131 pre-fix rows are** — while every number quoted as "after" came
from a focused window. On the same hardware arm, in the SAME process
(generation 1419187), focus moves `render/gui` p50 from **0.026 (n=524
unfocused) to 0.080 (n=8 focused)**, and across generations on that arm to
**0.179 — 3x to 7x, larger than the 2.3x being claimed.** Focus is not
cosmetic: `tick_hot_warmer` does SSH work only when focused. The comparison is
void and the regression is **unmeasured**, not disproven — the GUI process
really does hold DRM fds and submit GPU work it never did under llvmpipe, so
a real cost there remains possible. `window_focused` is now trap #4 in
`docs/optimization-pass.md` → "The standing measurement traps"; the A/B that
settles it is `scripts/gl_ab_experiment.sh` + `scripts/gl_ab_analyze.py`.
✅ **What DOES hold:** the GPU is genuinely rasterizing (0 → 7 DRM fds on
`amdgpu`; 5,886.8 ms of engine time over 300 s deduped on `drm-client-id` =
1.96%), the policy publishes `hardware_gl_probed`, the surface paints cleanly,
and there have been no coredumps.
**What a real verification needs:** the same `terminal_read` rate on both
sides, several hundred render ticks, and `window_focused` held constant.
**The live proof, on the GUI host, before → after the swap:**
| | before (GUI 1151877) | after (GUI 1419187) |
|---|---|---|
| DRM render-node fds held | **0** | **7** (GUI 3, web content 4, `amdgpu`) |
| GPU engine time accumulated | **0 ns** | GUI 335 ms, web content 698 ms |
| VRAM allocated | — | 268 MB (`drm-total-vram`) |
| idle CPU, whole tree | ⛔ 0.449 → 0.065 was busy-vs-quiet, NOT idle-vs-idle |
| published policy | — | `YGGTERM_WEBKIT_GL_POLICY: hardware_gl_probed` |
**The structural half is the part that cannot be argued with**: llvmpipe never
opens a DRM node, so 0 fds → 7 fds on `/dev/dri/renderD128` with
`drm-driver: amdgpu` IS the fix working. It is also NOT cheap-because-broken —
a faithful screenshot right after the swap shows the active session painting
cleanly at 168×63 with the full sidebar and rail.
⚠ **Be honest about the CPU number.** 0.449 → 0.065 is a same-host, same-daemon,
both-idle comparison, but the new GUI was minutes old with ONE terminal host
mounted while the old one had been up 5.5 h. Some of that drop is a fresh
process, not the GPU. Re-measure after the GUI has been up for hours before
quoting 7x as the steady-state figure.
⚠ **`YGGTERM_WEB_SURFACE_UNDER_GLASS=1` is now armed in production for the
first time** (it follows from hardware GL). Under-glass previously crash-looped
this host under llvmpipe, which is a different premise — but watch
`coredumpctl`. `YGGTERM_FORCE_SOFTWARE_GL=1` restores the old behaviour whole.
⚠ **The GPU gauge needs the client dedup.** `drm-engine-*` counters are
per-DRM-CLIENT, and duplicated fds each report the same cumulative value, so a
naive per-fd sum over-counts by the fd count (measured 5.00x on Xorg, 4.00x on
a compositor). `render_probe` dedups on `drm-client-id`; anything hand-rolled
must too.
⚠ **Zero engine time in a window means IDLE, not software.** The first
post-swap read said "software rasterization" on a host that had just switched
to hardware, because nothing painted during it. Read the DRM-fd count FIRST —
that is structural — and engine time second.
⛔ **Do not read the FIX paragraph below as the shipped design; it is
superseded.** What landed instead of the one-env-var workaround: the binary
PROBES the host (`crates/yggterm-core/src/gl_probe.rs` — Surfaceless platform
only, never GBM; `renderD*` only, never `card0`; no disk cache; in a child
process so a Mesa SIGSEGV is one `Unknown` and not a dead window), one policy
turns that into `hardware_gl` (`linux_webkit_gl_policy_from_input`), and
`shm_force_for_arming` now refuses SHM on a hardware host so the three settings
cannot be split apart. `YGGTERM_FORCE_SOFTWARE_GL=1` restores the old
behaviour; `YGGTERM_ENABLE_WEBKIT_COMPOSITING=1` still forces hardware. The
five shell + three python launcher re-encodings are deleted and the launcher
marker is `v4` so installed launchers get rewritten.
**WHAT IS STILL OWED, and why this entry is still open:** a GUI swap on the
live host and then, in order — `server app desktop-identity` shows, under the
client's `webkit_gl_environment`, `YGGTERM_WEBKIT_GL_POLICY=hardware_gl_probed`
with `LIBGL_ALWAYS_SOFTWARE` and `GALLIUM_DRIVER` and
`WEBKIT_DISABLE_DMABUF_RENDERER` all ABSENT and
`YGGTERM_WEB_SURFACE_UNDER_GLASS=1`. ⚠⚠ **Read it from `webkit_gl_environment`,
never from the `exec_environ` map beside it**: that map is
`/proc/<pid>/environ`, i.e. the environment the process was LAUNCHED with, and
every GL key is written after exec — it shows nothing on a fresh launch and the
PREDECESSOR's decision after a hot restart; `drm-engine-gfx` PRESENT and RISING in
`/proc/<webproc>/fdinfo/*` (this is the decisive gauge — a CPU number alone
proves nothing); a `render_top` cores delta against the §3a baseline under the
same workload; three relaunches including one through the supervisor, all
reading the same policy (that is the env-inheritance poisoning risk proven
closed on the host, not just in a unit test); and a faithful terminal
screenshot, because hardware GL changes the presentation path for the xterm
WebGL renderer and "CPU went down" is not evidence the terminal still paints.
⚠ It also arms Phase F under-glass in production for the first time.
⚠ **Installed users were never in the measured before-state**: their launcher
exported `WEBKIT_DISABLE_COMPOSITING_MODE=1`, which is hardware GL libraries
with compositing OFF while the WebGL renderer is still selected — a fifth
combination outside the four-way matrix below. The 22x does not describe them.

The original finding, unchanged:
`configure_linux_webkit_compositing()` (`apps/yggterm/src/main.rs`)
set `LIBGL_ALWAYS_SOFTWARE=1` + `GALLIUM_DRIVER=llvmpipe` +
`WEBKIT_DISABLE_DMABUF_RENDERER=1` unless `YGGTERM_ENABLE_WEBKIT_COMPOSITING=1`
was set. Its comment justified this as *"jojo: AMD iGPU exposing only
llvmpipe."* ⛔ **That premise is FALSE on jojo and has been for some time.**
`eglinfo` platform matrix, jojo, 2026-07-25: GBM → `llvmpipe`, but **Wayland →
`AMD Radeon 780M (radeonsi, phoenix, ACO)`**, Surfaceless → same, Device →
same. Only the **GBM** probe fails, and it fails because it opens `card0` and
gets `EACCES` on `DRM_IOCTL_AMDGPU_INFO` — the compositor holds DRM master.
Every ioctl on `/dev/dri/renderD128` **succeeds**. One EACCES on the wrong
node was read as "this host has no GPU," and hardware GL was disabled product-wide.
**MEASURED** (same page, same duration, CPU-seconds from `/proc`, not `ps %CPU`;
standalone WebKitGTK 2.52.4, same lib the GUI loads):
| workload | soft GL + SHM (today) | hw GL + DMABUF | ratio |
|---|---|---|---|
| **WebGL glyph grid (= xterm.js 6's renderer)** | **151.56 s / 20 s = 756% of a core** | 6.85 s (34%) | **22x** |
| CSS animation | 15.33 s (77%) | 4.12 s (21%) | 3.7x |
| DOM/JS-heavy | 11.13 s (45%) | 6.44 s (26%) | 1.7x |
| static idle page | 1.36 s (5.4%) | 0.96 s (3.8%) | 1.4x |
**★★★ The 22x row is the product-defining one, and it is NOT about browsing.**
xterm 6 REMOVED the 2D canvas renderer, so the TERMINAL draws through the WebGL
addon (`xterm_webgl_enabled_for_wayland`). Under llvmpipe every keystroke and
every line of streaming agent output is software-rasterized across 16 threads.
This is why the fan spins with no ychrome open at all.
⚠⚠ **DO NOT "fix" this by clearing the DMABUF flag alone.** The guard's LOGIC
is sound; only its premise is wrong. Measured: hardware GL + SHM = 15.82 s (as
bad as software), and software GL + DMABUF = **34.14 s, the WORST of the four**
(llvmpipe emulating the compositor). The three settings are one decision.
⚠ **The safety net is not buying the stability it was added for.** 26 GUI
coredumps in 10 days, still crashing 2026-07-25; **24 of 26 contain zero
GL/Mesa/EGL frames**, and the one genuine WebKit SEGV is in JavaScriptCore GC.
We are paying 4-22x CPU to prevent crashes that are not GL crashes.
**FIX** = set `YGGTERM_ENABLE_WEBKIT_COMPOSITING=1` for the GUI. That ONE var
unlocks all three gates: the installed launcher stops exporting
`WEBKIT_DISABLE_COMPOSITING_MODE=1` (`install.rs:1303`), the GL safety net is
skipped, and under-glass arms — which clears the SHM force via
`shm_force_for_arming(true, _) == Clear`. Confirmed against this repo's own
unit tests (`main.rs:5066`, `:5099`). **The real fix is to stop hard-coding the
premise**: probe the Wayland/Surfaceless EGL platform at startup and pick the
path from what the host actually reports, instead of defaulting to the slowest
configuration and requiring an opt-out. `render_probe` is the natural owner.
**VERIFY** with `drm-engine-gfx` in `/proc/<webproc>/fdinfo/*` (nonzero ⇒ the
GPU is really rasterizing) plus a CPU-seconds delta — never `ps %CPU`.


## ★★ app open CANNOT OPEN A TERMINAL SESSION ON A SHADOW CLIENT

**★★ `app open` CANNOT OPEN A TERMINAL SESSION ON A SHADOW CLIENT (found
2026-07-25; ROOT-CAUSED and FIXED 2026-07-25, see the four changes below).**
⛔ **This entry's first two diagnoses were BOTH wrong about the mechanism.
The trace settles it — read the correction before re-deriving anything.**
**What the trace actually shows** (`event-trace.jsonl`, category `role_gate`,
live on jojo with a shadow `agent-r20`): the shadow's terminal lane dies on
TWO daemon refusals, neither of which is a grid problem.
1. `{"category":"role_gate","name":"shadow_refused","payload":{"request":
   "focus_live"}}` — `app open` routes the shadow's view switch through
   `spawn_focus_live_session_row`, which calls `focus_live_with_view`, which
   mutates the **daemon's SHARED active session**. Denied. No snapshot comes
   back, so nothing applies the switch, and the poll then reports the session
   the viewport is still SHOWING. **That is why `active_session_path` never
   moved** — the open question the previous revision left is answered, and it
   was never the grid.
2. `{"request":"terminal_ensure"}` → `shadow_cannot_own`, repeatedly. The
   mount treats an ensure error as fatal and `return`s before the read stream
   starts, so **the shadow's terminal viewport was blank BY CONSTRUCTION** —
   for every session, not just a no-activate one.
⛔ **"The unsized-PTY grid divergence" was real but MIS-SCOPED.** It is not
about `--no-activate` at all: the shadow reports the same violation against a
fully-sized session the user is actively using (`Client viewport 167×57
diverges from daemon PTY grid 168×63`). The invariant the old entry missed is
that **the session/view contract assumes ONE viewer per session**; a second
viewer with its own window size violates it by construction. So "give a
no-activate spawn a grid" would have fixed one instance and left the class.
**THE FIX (shipped): the shadow is a READ-ONLY VIEWER, enforced on the CLIENT
side too.** The daemon's role gate is unchanged — the ownership boundary was
never wrong; the client was wrong to ask. `client_is_shadow_viewer()`
(`crates/yggterm-shell/src/shell.rs`) now gates four call sites:
- `terminal_ensure_with_retry_async` → skipped, traced
  `terminal_mount/shadow_read_only_attach`. The mount proceeds to the read
  stream, which needs only `TerminalRead`/`TerminalSnapshot`/`TerminalHistory`
  — all already `Allow`.
- `terminal_resize_async` → skipped. D8 says a shadow must never drive
  SIGWINCH; now it never asks, instead of asking and treating the refusal as
  a mount fault.
- `spawn_focus_live_session_row` → client-local `restore_active_session`
  instead of the daemon's shared `focus_live`.
- `YggtermServer::apply_snapshot` → a shadow that has already chosen a session
  KEEPS it; without this the next refresh re-imposed the daemon's active path
  and the shadow followed the user around seconds after `app open` landed.
**Fidelity: the viewer adapts to the session, not the reverse.** Since the
shadow may not resize the PTY, its xterm pins to the daemon's grid
(`window.__yggtermShadowPinnedGrid`, set from the session's "PTY size" at
mount); `proposedTerminalFitDimensions` returns the pin and the row-fit guard
stands down, so the two grids are equal by construction and the contract check
stays a real regression detector. `scripts/shadow-client.sh` now defaults to
**2560×1440**, because a window smaller than the pinned grid CLIPS rows out of
every screenshot (1920×1080 gave 167×57 against a live 168×63 and lost six).
**Still true:** document surfaces were never affected — they replace the
viewport, which is why the yedit rail work was provable via `right-panel` on
the same session whose `app open` failed.


## ★★ WHEN THE GUI DIES, NOTHING BRINGS IT BACK

**★★ WHEN THE GUI DIES, NOTHING BRINGS IT BACK — measured 2026-07-25, then
BUILT + LIVE-PROVEN the same day (`supervisor.rs`).** The design below stands
as written; this is what shipped and how it was proven.
**Shipped:** `yggterm --supervise` forks the real GUI as a CHILD and waits on
it, so it learns the exact status and applies `Restart=on-abnormal` itself:
restart only on `SIGSEGV`/`SIGABRT`/`SIGBUS`, never on an exit code (a clean
quit, a `status=130` SIGINT handler, or an update handoff), never on
`SIGTERM`/`SIGINT`/`SIGKILL` (someone asked), never if the child died inside
10s (that is a crash-on-startup loop, not a recovery), and at most 5 times an
hour. The desktop entry's `Exec=` now carries the flag — `TryExec=` stays a
bare path, because the spec defines that as a program to look up — and an
entry written before the supervisor is treated as stale so it gets rewritten.
`StartupWMClass` still matches: the CHILD owns the window.
**Live proof (jojo, 2026-07-25):** launched through the shim (supervisor
1031410 → child 1031412, child registered as the active client);
`kill -ABRT` the child; the supervisor logged
`window died on signal Signalled(6) after 229681ms — restarting (1/5 this
hour)`, forked child 1034390, which registered and painted a faithful frame
with all 31 rows intact.
⚠ **`kill -SEGV` is NOT a valid test of this** — the process survived it
untouched (something in the runtime handles SIGSEGV and returned), so the
supervisor correctly saw nothing. Real faults DO kill it (ten core dumps in
the measurement below). Use `SIGABRT` to exercise the policy.
⛔ **`server app launch` deliberately does NOT supervise.** That launcher polls
for a client record whose pid is the pid it SPAWNED; under a supervisor that
pid is the shim's while the record is the child's, so `--wait-visible` would
wait forever and report `registered: false`. Supervising the agent path needs
the poll to match the child first.
⚠⚠ **DO NOT run `yggterm install integrate` on the live host to pick up the
new desktop entry.** `refresh_linux_integration` also OVERWRITES
`~/.local/bin/yggterm` (and `-headless`) with a launcher SCRIPT pointing at
`preferred_executable_for(context, ...)` — on jojo that is the direct-install
channel's recorded `active_version`, **2.9.48**, so a one-line desktop fix
would silently replace the deployed 2.12.12 binaries with a script pointing at
a months-old build. On a dev-deployed host, edit the `Exec=` line of
`~/.local/share/applications/dev.yggterm.Yggterm.desktop` directly (done on
jojo 2026-07-25) and leave the launcher alone. The generated entry is correct
for a real install; it is the launcher-rewrite half that fights a hand deploy.
The measurement that produced the policy, kept because the numbers are the
argument:
**The measurement (jojo, `systemctl --user` + `coredumpctl`), because the raw
"42 failed `Yggterm@*` units" number is misleading and I nearly quoted it:**
- **10 units `Result=core-dump`** — genuine crashes. `coredumpctl` dates them
  across 2026-07-08 .. **2026-07-24 16:11**, several per week, latest the day
  before this entry. SIGSEGV mostly, three SIGABRT.
- 31 `Result=exit-code`, of which **10 are `code=exited; status=130`** — the
  process's OWN handler exiting on SIGINT. Those are deliberate agent/user
  kills, NOT crashes. 1 `status=9` (SIGKILL).
⇒ **~10 real crashes, not 42.** Do not cite the failed-unit count as a crash
count; sort by `Result` first.
**Why nothing restarts:** the unit is a TRANSIENT service the desktop
environment creates per launch (`app-dev.yggterm.Yggterm@<uuid>.service`,
`Restart=no`, `ExecStart=/home/user/.local/bin/yggterm`). We do not author it,
so we cannot set a restart policy on it.
**The policy that is exactly right is `Restart=on-abnormal`, NOT
`on-failure`.** `on-failure` also restarts on a non-zero *exit code*, which
would fight every deliberate shutdown above (`status=130`). `on-abnormal`
restarts on signal-death, core dump, timeout and watchdog only — i.e. a
segfault comes back and a clean quit stays quit. Whatever mechanism is
chosen must reproduce that distinction.
**Two implementations, and the second is the one to build:**
1. *Daemon-side supervision.* The daemon outlives the GUI and already stores
   everything a relaunch needs on `ClientInstanceRecord` (`executable_path`,
   `display`, `wayland_display`, `xdg_runtime_dir`, `xauthority`), and
   `launch_app_background` already knows how to spawn a GUI from a non-GUI
   context. ⛔ **But the daemon is not the GUI's parent, so it cannot see the
   exit status** — it can only tell "the process is gone", which is exactly
   the distinction that matters. It would have to infer intent from a missing
   `PrepareClientClose`, and a SIGKILLed GUI (seen once) has none either, so
   it would race the agent's own relaunch loop and produce two windows.
2. **A supervisor shim (preferred).** `Exec=` launches yggterm in supervise
   mode; it forks the real GUI as a CHILD and `waitpid`s, so it learns the
   status EXACTLY (`WIFSIGNALED` + `WTERMSIG` in `SIGSEGV|SIGABRT|SIGBUS`)
   and applies `on-abnormal` semantics itself. No systemd dependency, works
   on every Unix, and the DE's unit keeps its `StartupWMClass` because the
   child owns the window. Needs a restart budget (N per hour, and refuse if
   the child died in under ~10s, so a crash-on-startup cannot loop).
   ⚠ Check the update/exec-handoff path first: an in-place `exec()` keeps the
   pid (supervisor sees nothing, correct) but a spawn-successor-and-exit-0
   handoff must read as a CLEAN exit, or every update would spawn a duplicate.
   Both are safe under the shipped rule, which restarts on signals ONLY.
⛔ **"Not live-provable without a daemon+GUI swap" was wrong**, and worth
keeping as a lesson: it needs a GUI-only swap plus a launch through the shim,
and the crash can be delivered on purpose. Proving it cost one `kill -ABRT`.


## ★ AGENT SHADOW CLIENT FOR THE TERMINAL LANE

**★ AGENT SHADOW CLIENT FOR THE TERMINAL LANE — E2E LIVE ON JOJO
(2026-07-23, user directive "complete the agent client system e2e"):**
the slice-4.3 shadow view client now runs against the LIVE daemon
(sway+grim installed on jojo; `scripts/shadow-client.sh start --name
agent-1` attaches as Shadow through the role gate), and probes drive it
with `--client agent-1` — live-proven that `app open --client agent-1`
switches ONLY the shadow's active session while the user's worker stays
put (pid-targeted state on both workers). **Routing hole found + fixed
the same night:** untargeted app-control verbs resolved to the NEWEST
worker, so a running shadow silently captured every untargeted verb —
which reads exactly like the shadow yanking the user's session (it was
an instrument lie, not propagation). `ClientInstanceRecord` now carries
`client_role`; untargeted verbs prefer the sole ACTIVE client for reads
AND mutations (user scripts keep working while shadows run);
only-shadows-alive mutations fail loudly; legacy records read Active.
The probe workflow is codified in
`.agents/skills/yggui-app-control/SKILL.md` §THE SHADOW-PROBE LAW.
**COMPLETED same night (user: "finish the remaining gaps"):**
`terminal new --no-activate` (create without switching the user's view;
activation handed back before the next render, so nothing flashes) and
**headless surface-create** (`web ensure --session <path>` — see
docs/agent-control-plane.md, "✅ BUILT 2026-07-23"). The user ruled the
shadow client FIRST-CLASS (bug-bash pixels of a non-active view), with
the platform caution: sway+grim is the Linux backend only — yggterm goes
Windows/macOS (+ mobile in a private repo), so shadow-view work stays
behind a per-platform backend seam and the core plane must never grow a
compositor dependency. **Still recorded:** ★ Dream §2 — daemon-side OSC
declare ingestion: the web-surface declare is parsed by the CLIENT-side
terminal eval script, so a never-revealed session's FIRST declare needs
one brief reveal+restore (~5s); after it, `web ensure` re-materializes
headless forever (live-proven: background `web read` + per-surface
screenshot with the user's view untouched). The fix is parsing/
registering declares daemon-side (or a bounded GUI-side chunk scan on
`ensure`) so even the first declare is invisible. Also: terminal-lane
agent PRESENCE (a badge when an agent drives a session's terminal —
pointer verbs have the cursor, `terminal send` shows nothing); on-demand
shadow lifecycle (auto-spawn + idle reap, D6).


## Blank viewport from a DETACHED term.element

**Blank viewport from a DETACHED `term.element` (jojo, 2026-07-22).** The
viewport paints nothing — background only — while the session is alive, the
daemon screen is correct, and **every health field reports healthy**. Cause:
`term.element` is out of the DOM (`isConnected:false`, rect 0×0) while an
empty husk — `div.terminal.xterm` holding only `.xterm-viewport`, no
`.xterm-screen`/rows/canvas — occupies the host. It never self-heals because
all three `rebindCurrentHost` reopen guards read false against that husk (it
matches `.xterm`; the renderable-layer check requires the absent
`.xterm-screen`), and `ensureVisibleHost` short-circuits on `emitPaint()`,
whose `visible` is satisfied by any child.
**Probes shipped 2026-07-22 (`terminal_host_element_detached`, host-attachment
fields in `app state`, mutation breadcrumbs).** **FIX LANDED in code 2026-07-22
(`rebindCurrentHost` now treats `termElementOutsideHost` — `term.element` not in
the live host — as a fourth reopen trigger, so the reopen re-appends
term.element and drops the husk; guarded by
`terminal_eval_script_probes_detached_term_element`).**
⛔ **THAT FIX SHIPPED A REGRESSION IN 2.12.2 — corrected in `f0aca70`.** Its
premise ("it can only fire when term.element is genuinely elsewhere, which is
itself the bug") is FALSE for a **backgrounded** host: a parked session's host
leaves the DOM entirely, taking `term.element` with it, so the trigger read
"broken" forever on every parked session and `emit_resize` re-fired the reopen
continuously. Measured live: **3931 `rebind_host` events in 5 minutes (~13/s)**,
WebKitWebProcess pinned at 26%, the viewport blinking ~2x/s, mount generations
churning `m8 -> m9 -> m10` in 364 ms, and — because the churn never let focus
settle on the xterm helper textarea — **a session the user switched to came up
blank and REFUSED KEYBOARD INPUT.** The same-host reopen is now gated on
`liveHost.isConnected`. After: 0 rebinds in 25 s idle, one per switch,
WebKit 26.0% -> 16.1%, GUI 10.7% -> 4.8%.
**Generalise: any repair/reopen trigger must first ask whether the thing it is
repairing is on screen at all.** A repair loop on a parked host is invisible
except as heat. Full write-up, the
trace signature that dates past occurrences, and the open questions:
[`docs/xterm-bugs.md#detached-term-element-blank-viewport`](xterm-bugs.md#detached-term-element-blank-viewport).
Recovery with no restart: re-append `term.element` and drop the husk via
`server app dom-eval`.
**★ THE REPAIR HALF IS NOW FIXED (`7247eb7`, live-proven 2026-07-22).** The
reason no repair path ever healed this: **`term.open()` is a no-op on an
already-opened terminal** (it early-returns once `term.element` exists,
without re-parenting), so every "wipe the host, then re-open" recovery rebuilt
nothing and stranded the surface outside the DOM. `ensureVisibleHost`'s
last-resort `rebuild_blank_host` was exactly that shape. Now one owner,
`attachTerminalSurfaceToHost`, MOVES `term.element` back, called
unconditionally after every wipe; pinned by
`tools/xterm-harness/host_reopen_is_a_noop.test.js` against the real bundle.
**Two leads corrected by live measurement:** the husk is born **AT MOUNT**,
not on switch-back under heavy streaming (every earliest-episode autopsy shows
the same same-millisecond `constructed` → `renderer_decision` →
`snapshot_restored` → `rebind_host term_outside_host=true` → detach sequence);
and **the reveal ghost is NOT involved** (zero ghost nodes live; the
attach≫release gap is an accounting artefact — `releaseRevealGhost` is gated on
`isConnected`, so a wipe that already removed the ghost suppresses the event).
**★★ THE CREATION HALF IS NOW ROOT-CAUSED AND FIXED (2026-07-22).** The husk
is born in a **PArecordsAL `term.open()`**, and this is proven deterministically
against the shipped bundle by
`tools/xterm-harness/husk_is_born_in_a_partial_open.test.js` — not inferred
from a live symptom. `open()` appends the bare `.xterm` root to the host
**first** and appends the viewport/screen fragment **last**, so any throw in
between leaves a connected, empty root: exactly
`orphan_root_without_screen=true xterm_roots=1 screen_in_host=false
rows_in_host=false screen_canvases=0`. The mount's `term.open(host)` was
**unguarded**, so that throw also abandoned the rest of the mount (OSC
suppressors, bell, observers) — which is why the autopsy always showed the
husk born at mount, in one millisecond.
**Why it looked unrepairable, and why it is not.** `open()`'s early-return
guard is `this.element && this._coreBrowserService`, and `_coreBrowserService`
is assigned **late** inside `open()`. A partial open therefore sets `element`
but never arms the guard, so a second `open()` really does rebuild — but only
if the husk root is removed first; leave it and the rebuild strands it as an
**orphan beside the new root**. That is where the autopsy's orphan roots come
from, and it explains the 18/18 "constructed ≥2×" correlation without needing
two live closures.
**Fix:** `terminalSurfaceIsComplete` is now the one owner of "surface or
husk?". The mount retries an incomplete open (after discarding the husk) and
emits `terminal_mount_open_incomplete`; `attachTerminalSurfaceToHost` refuses
to MOVE a husk and rebuilds it instead. Guarded by
`terminal_eval_script_rebuilds_a_husk_instead_of_moving_it`.
**✅ "SPECIES B" IS FIXED TOO (2026-07-22) — and it was never a second
species.** It was written up here as *"a terminal that opened completely and
lost its screen subtree afterwards"*, with the open question *"who removes
`.xterm-screen` from an already-opened terminal?"* **Nobody does. There was
never a completely-opened terminal.** `_coreBrowserService` — the second half
of `open()`'s early-return guard — is assigned in the **middle** of `open()`,
six services before `element.appendChild(fragment)` finally puts the screen
into the root. So the husk's birth window is not one window but two, split by
that single assignment:

| throw lands | root in host | guard | screen | |
|---|---|---|---|---|
| before `_coreBrowserService` | yes | unarmed | no | species A — `open()` rebuilds it |
| **after** `_coreBrowserService` | yes | **armed** | no | "species B" — `open()` is a no-op |

Same birth site, same mount, same millisecond; only the throw's position
differs. Measured element-by-element, first in jsdom against the shipped
bundle (`tools/xterm-harness/husk_species_b_is_a_late_partial_open.test.js`)
and then **in the live WebKit engine on jojo**, where the band is real and the
husk's DOM signature is identical to species A's.
**The fix follows from that:** the armed guard is *stale*, not authoritative —
it guards a terminal that never finished opening. So when the rebuild does not
take, the surface owner clears `term._core.element`, which disarms the guard,
and re-opens; `open()` then runs its whole body and builds a real surface.
Proven live in real WebKit: husk (no screen) → plain `open()` → still no
screen → disarm → screen present, `.xterm-rows` in the host, and
`term.write()` read back verbatim from the buffer. New mode
`rebuilt_from_husk_disarmed` distinguishes it in the mutation log.
⚠ The private `_core` shape is **feature-detected**: an xterm bump that moves
it degrades to the old put-the-husk-back behaviour (`rebuild_from_husk_failed`,
remount required) rather than half-repairing silently.
⚠ **`term.element` on the public `Terminal` is a delegating getter** — reading
or assigning `term._coreBrowserService` / `term.element` on the wrapper
silently does nothing. An earlier draft of the harness probed the wrapper and
concluded "the guard never arms", which was the instrument lying, not xterm.
Probe `term._core`.


## ★ ROOT-CAUSED + FIXED 2026-07-23 (2.12.7): the vanishing client-instance

**★ ROOT-CAUSED + FIXED 2026-07-23 (2.12.7): the vanishing client-instance
record was a TOCTOU in the register itself.** `register_client_instance`
wrote non-atomically — `create_new` produced an EMPTY file, the JSON landed
in a later `write_all` — and every `server app …` CLI probe runs
`cleanup_stale_client_instances`, whose "undeserializable → delete"
predicate ate any record read in that window. The register then wrote to
the unlinked inode successfully and traced `ok:true`, which is why the
2026-07-22 incident showed a byte-identical-to-healthy register with an
empty directory one second later, and why both previously-suspected
deleters were correctly falsified. **Fix:** the record is staged in a
`tmp/` subdirectory the cleanup pass skips, then renamed into place
(atomic); every removal is now traced
(`client_instance_record_removed` with removing pid, removed pid, and the
rejecting predicate) so any residual deleter convicts itself. Locks:
`register_client_instance_publishes_a_complete_record_atomically`,
`cleanup_stale_client_instances_skips_the_atomic_write_staging_dir`.
Live: `server app clients` returned exactly 1 after BOTH 2.12.7 swaps.
The manual record-reconstruction recovery recipe lives in git history of
this file (pre-2026-07-23) if ever needed. Remove this entry after a few
more clean GUI restarts.


## ★★ THE CLICK RENDER STORM

**★★ THE CLICK RENDER STORM — root-caused live 2026-07-23 (user repro:
"clicking anywhere in the claude TUI produces the blink … UI gets laggy and
fans spin"), fix = single-live-owner stand-down, felt-confirmation pending.**
Mechanism, proven with a tagged-node MutationObserver on the live host: a
click-driven re-open re-dispatches the terminal eval script for a hostId
whose PREVIOUS closure is still alive (`constructed …-m1` fired 3× for one
hostId: GUI start + both click episodes — the mount-epoch reuse keeps the
LABEL but not the closure). Both closures then FIGHT for the host: each
one's placement repair sees the other's element and evicts it — measured
ONE click → **560 host childList mutations in 3 s**, two roots (the WebGL
original vs a `xterm-dom-renderer-owner-N` twin) alternating at 25–50 ms,
each wipe re-firing the other closure's ResizeObserver. The storm is also
the DOM-event flood that starves the GTK input region (laggy UI) and burns
CPU (fans). It settles only when one side's circuit breaker loses.
**Fix (GUI-only): ownership tokens.** Registration into
`__yggtermXtermHosts[hostId]` is last-writer-wins and now stamps an
`ownerToken`; a closure that finds a newer token STANDS DOWN completely
(rebind/redraw/render-health refuse, ResizeObserver disconnects, traced
`superseded_closure_stand_down`) instead of competing. Locks: the
ownership/gate asserts in the eval-script test.
**Also fixed:** SGR mouse-report bursts (a click on a mouse-tracking TUI =
`\x1b[<b;x;yM` ≈ 12–14 bytes on onData) were classified as pastes — 226
bogus `xterm_paste_event`/hour measured.
**THE IN-SESSION ARM OF THE ZOOM IS FIXED + LIVE-PROVEN (2026-07-23, the
§7.3 stable-epoch generalization).** The chain was:
`bootstrap_identity = {mount}:{generation}:{activation_epoch}` and
`terminal_bootstrap_activation_epoch` returns `latest_open_request_id` for
the ACTIVE session — so every gesture-free open request at output
boundaries re-ran the full bootstrap (new closure, new Terminal, ghost
cover, fit+restore = the felt zoom) for every arm EXCEPT remote-codex,
whose `remote_resume_stable_bootstrap_epoch` pin is the §7.3 codex-only
hole. Shipped: `retained_ever_ready_host_should_pin_bootstrap_epoch`
(kind- and locality-agnostic: retained + ever-ready + daemon-owns-runtime
+ no latched fault + no failed/timed-out overlay) — and the pin FREEZES
the epoch at its in-effect value instead of zeroing it, because zeroing
would change the identity once at engagement and re-bootstrap every
session right after readiness (the birth-remount class round 8 killed).
Paired with a once-per-visibility-transition nudge
(`stable_epoch_reveal_nudge`: registry `emitResize` + `redrawTerminal`)
so a pinned reveal that reuses a surviving closure cannot come up blank;
it deliberately never fires on request bumps while the host is on screen.
**Live proof: 3-minute quiet window on the actively-streaming remote-cc
session = 0 bootstrap events (pre-swap same session: 4–5 per 10 min).**
**STILL OPEN — the SWITCH-reveal re-bootstrap, now DESIGN-COMPLETE
(sharpened 2026-07-23 late, do NOT re-diagnose):** every switch recreates
the terminal COMPONENT INSTANCE (fresh `last_bootstrap_identity` ⇒
`bootstrap_reset` fires WITH `mount_epoch_reused` on the same render —
for remote-CODEX too), so no activation-epoch pin can help. The premount
keep-set (HOT-tier, cap 8) retains the EPOCH and the JS closure — the
xterm closure genuinely survives in `__yggtermXtermHosts` with its
painted buffer, and the saved-cursor `ResumeAppend` read plan already
makes the re-read delta-only — but the single-live-owner stand-down
(the click-storm fix) GUARANTEES the fresh dispatch's new closure
supersedes the survivor and rebuilds from scratch. **The fix is an
ADOPTION path in the mount script:** before constructing, if the registry
holds a live entry for this hostId with a COMPLETE surface
(`terminalSurfaceIsComplete`), call a new closure-exposed
`adoptHost(newHostElement)` on the survivor — it re-points the closure's
`host` binding, moves `term.element` in via `attachTerminalSurfaceToHost`
(refuses husks by construction), re-attaches host interactions +
ResizeObserver + surface contract — and the new script EXITS WITHOUT
REGISTERING (so the survivor's ownerToken stays newest; no stand-down
fires). ⚠ The hard part is the RUST bootstrap contract: the dispatching
bootstrap task must treat "adopted" as constructed+painted (emit a
compatible event or a dedicated `adopted` signal) or it will stall into
timeout recovery — the snapshot-poison minefield. Skip the snapshot seed
on adoption (the buffer is live); the reveal nudge shipped this round is
the repaint half. Prove on {local,remote}×{cc,codex}×{idle,streaming}:
second reveals must show ZERO `bootstrap_reset` and no construct, with
scrollback intact. Also still open: the residual "slight zoom, no blink"
ghost-geometry mismatch on covered switches (pixel-diff ghost frame vs
first settled frame on New Yedit).
The in-session arm is user-confirmed fixed (2026-07-23 "all good");
keep this entry until the adoption path lands.


## Rendering stability: user RE-REPORTED blinking + blank-on-switch 2026-07-23

**Rendering stability: user RE-REPORTED blinking + blank-on-switch 2026-07-23
("blinking and waiting on blank sessions only fixed by switching again and in
session blinking") — a THIRD defect found + fixed same day: the render-health
ink probe was blind and its recovery loop WAS the in-session blink.**
`sampleCanvasInk` judged "canvas blank" from ANY canvas in the host (reveal
ghost, overlays) while the canvas that actually paints text was either absent
(DOM renderer) or unreadable (WebGL — `getContext('2d')` returns null on a
GPU-context canvas). Measured in the hour before the fix: **110 false
`terminal_render_health_unhealthy` edges and 47 `render_health` repaints**,
each repaint = atlas clear + full refresh + forced host rebind (a visible
blink), and each rebind's wipe window produced fresh `term_element_detached`
readings that scheduled the NEXT repaint — self-sustaining. Backgrounded
hosts accumulated the same false "unhealthy", which the reveal path consumes
to force a repaint at switch-in (the switch-in blink/blank). The 2026-07-20
fix attempt (ba2fe8c, drawImage readback) had corrupted the glyph atlas and
was reverted; the diagnosis was right, the readback was the poison.
**Fix (2026-07-23, GUI-only):** ink sampled ONLY from `.xterm-screen` render
canvases; an unreadable (GPU-context) layer marks the sample `unsampleable`
and FORBIDS the canvas-blank verdict (no GPU touch, no readback); a detach
verdict must persist ≥900 ms (the racing `detached_ms=0` reads 28–642 ms
after `rebind_host_attach` no longer count); the attachment-state mirror
gained the missing `termElementOutsideHost` guard so `unrepairable` stops
false-alarming. **Live: 3 min post-swap under heavy streaming = 0 unhealthy
edges, 0 repaints, 0 rebinds** (was ~5/2/several per 3 min), and the active
host's ink reads `unreadable_layers:1, unsampleable:true, status:healthy` —
the exact state that previously fired the loop. Locks:
`unreadable_layers` + `detachedPersistedMs` + guard asserts in the eval-script
test. **Remove this paragraph once the user confirms switching no longer
blinks and no blank-on-switch recurs across a few days.**


## Cross-pathway blink (local-cc → remote-cc switch)

**Cross-pathway blink (local-cc → remote-cc switch) — BOTH DEFECTS FIXED in
2.12.7 (2026-07-23), user gesture-confirmation pending.** The trace signature
was "each reveal CONSTRUCTS TWICE ~0.5 s apart" + `remote_pty_resize_failed
{terminal session not found: cc-runtime://<id>}` mid-switch.
**Root cause of the double construct — TWO writers, one shape:** the reveal
guard in `resolve_active_open_mount_epoch` requires `!attach_in_flight` AND
`was_ever_ready`, so the re-assert that lands right after any open request
completes (the `latest_open_request_id` bump re-runs the mount-key effect)
cold-remounted a session being born ~0.6 s into its FIRST attach; and
`invalidate_retained_remote_non_prompt_surface` treated the benign
"host exists but xterm surface is empty" reading of a 0.7 s-old settling
attempt as a fault (attempt 13 `source: retained_fault_recovery` in the
trace) and bumped the epoch directly. Both now reuse the settling host
while the latest attempt is inside its own recovery budget; a hung attach
ages out and remounts normally. **Live-proven on the 2.12.7 GUI swap: one
`bootstrap_spawn_scheduled` then `mount_epoch_reused` — previously every
birth was a pair.** Locks:
`open_reassert_reuses_the_host_while_its_first_attach_is_settling`,
the `attempt_settling` suppression in the invalidation path.
**The resize ordering half:** the remote daemon does not own the
`cc-runtime://` key yet while its ensure/resume is in flight mid-switch;
the resize worker now re-queues a not-found grid up to 5× (2 s apart,
newer client grid wins) instead of dropping it. Remove this entry once the
user confirms a local-cc → remote-cc switch no longer blinks.


## Drag-selection still freezes the TUI on 2.12.19

**Drag-selection still freezes the TUI on 2.12.19** (user-reported 2026-07-31),
despite a81c7366 making the per-event handler O(1) with a trailing-edge flush.

✅ **RESIDUAL FOUND AND FIXED IN CODE 2026-07-31.** Making per-EVENT work O(1) was
necessary and not sufficient: the deferred half coalesced to
`requestAnimationFrame`, so during a live drag over a streaming session
`term.getSelection()` — the O(selected-cells) serialization — still ran **once per
animation frame (~60/s)** on the same webview thread as the xterm write pump. The
commit title said a drag's cost must not grow with the selection it drags; per
*frame*, it still did.

Nothing can observe `__yggtermPrimarySelection` mid-drag — every reader flushes
synchronously first (pointerdown, pointerup, and `primarySelectionTextForPaste`
flushes every host before it reads) — so while the pointer is down the rAF flush
is pure redundancy. It is now skipped, and drag-end does the single serialization.
Cost during a drag is O(1) per event AND O(1) per frame.
Lock: `a_live_drag_schedules_no_per_frame_selection_work`, red-proven by deleting
the guard.

✅ **USER-CONFIRMED BY HAND 2026-07-31 on the deployed build ("I just drag
tested. It works.").** That closes it — the user's hands outrank the
instruments here.

⚠ **One loose end in the INSTRUMENT, not the fix.** The two gestures the trace
caught at 12:04:35 / 12:04:37 both logged `selection_events=0` and
`selected_chars=0`, so they read as clicks rather than the selecting drag —
meaning the confirmed drag either was not captured, or `selected_chars` is not
populating. Mechanism to check: at pointerup the flush only does work when
`primarySelectionSyncPending` is true, so a drag that fires no
`onSelectionChange` leaves `entry.primarySelectionLength` untouched and the
report reads 0. That is arguably correct (no selection change ⇒ nothing to
record) but it makes the field useless for sizing a real drag, which was its
whole point. Worth one pass before trusting `selected_chars` in any future
measurement.

- ✅ **THESE THREE BUG CLASSES ARE NOW PROVABLE FROM TELEMETRY (2026-07-31).** Each
of them survived because the instrument could not see it. What was added:

1. **Defer chains, not defer samples.** `screen_reconcile_deferred_recent_output`
   is rate-limited to one line per 10 s, so a raw line count understated the real
   deferral total **2.8×** (71 lines for 198 deferrals) and a continuous chain
   read as scattered singletons. Every line now carries `defer_chain_depth` and
   `defer_chain_ms`, reset only when a reconcile actually runs — so "the corrector
   is starved right now, and has been for 106 s" is readable instead of
   inferable. A forced repaint emits `screen_reconcile_forced_deadline` with the
   chain depth/age that triggered it. **When reading these, use
   `suppressed_since_last` — never the line count.**
2. **Drag lifecycle.** The terminal selection path emitted one event per copy and
   nothing during a drag, which is why a user-reported freeze was invisible for a
   release. Drag-end now emits `drag_selection_complete` with `selection_events`
   (streaming sessions multiply these), `drag_ms`, `flush_ms` (the one real
   serialization) and `selected_chars`. A freeze is now a number.
3. **Veil disposition and live veil state.** Telemetry showed 17
   `cold_mount_veil_attached` against 11 `..._released` — six veils with no
   disposition, because a host torn down under its veil hit a bare `return` in the
   settle poll. That path now reports `reason=host_torn_down`, so every veil
   leaves a record. And `server app state` gained `cold_mount_veil_count` +
   `cold_mount_veil_oldest_age_ms` on each terminal host, because from outside the
   webview a stuck veil looked exactly like a hung session and nothing reported
   either. Lock: `a_cold_mount_veil_reports_every_disposition`.

**The general rule this pays for:** an absence-gate needs a deadline, and any
mechanism that can degrade the viewport needs to report its own state — otherwise
the next report of "it never opened" is another argument instead of a measurement.

- ⚠ **`main` ships with 6 RED tests in `yggterm-shell` (found 2026-07-31).** All in
the retention/bridge/snapshot family:
`inactive_retained_ready_session_keeps_bridge_mounted_but_pauses_reads`,
`retained_background_session_trickles_reads_instead_of_pausing`,
`prune_terminal_attach_in_flight_drops_background_retained_attach`,
`shell_snapshot_retains_live_local_stored_codex_sessions`,
`shell_snapshot_trims_inactive_live_payloads_for_sidebar_and_retention`,
`sync_live_terminal_retention_keeps_active_not_fresh_inactive_live_sessions`.
Verified pre-existing by stashing all local work and re-running on clean `main`
— 1606 pass, these 6 fail. Unknown whether the tests encode a superseded model
or the retention behaviour genuinely regressed; per
`feedback-locks-survive-contract-changes`, **rewrite a test that encodes the old
model, never weaken it** — but decide that deliberately, with the retention
contract in hand. Likely cause of it going unnoticed: `cargo test | tail` eats
the exit code under `pipefail`, which that same memory already records as a trap.
