# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

## ⚠ READ FIRST — the state of the machine, 2026-07-26

- **jojo runs hardware GL with Phase F under-glass OFF.** That pairing is new
  and deliberate: arming under-glass put a background agent's page over the
  user's entire window. `YGGTERM_FORCE_SOFTWARE_GL=1` reverts everything.
- **The GPU CPU win is NOT established.** Every large figure previously recorded
  here was an artefact of comparing windows with different paint exposure. See
  `docs/optimization-pass.md` and field guide §7.3 before quoting any number.
- **Two supernumerary daemons persist** holding unmigratable `local://` shells.
  That is the durable half of the chaining bug, still open.
- **The vault agents on dev and jojo still need ONE `ychrome-vault unlock` each**
  — the 401-on-write fix is installed but the running agents predate it.
- **Five could-only-pass locks shipped in two rounds.** Before trusting ANY
  test in a report, mutate the production call site yourself. Field guide §7.1.

## Standing traps / other open bugs

- **★★★ USER-SETTLED CALLS + FEATURE REQUESTS (2026-07-26, verbatim intent).**
  These answer questions an agent asked; do NOT re-litigate them.
  1. **PLAIN SHELLS ARE FIRST-CLASS AND MUST SURVIVE A DAEMON BUMP.** Settled by
     the user. The 2.12.15 bump lost `local://b7ccbab4` ("ychrome HTTP Fixture
     Support") because a plain shell cannot migrate — no `SCM_RIGHTS` fd passing
     exists anywhere in the tree, so the only way to move a PTY is
     kill-and-re-resume, and a shell is not re-resumable. **That is now a BUG,
     not a documented limitation.** Two levels of fix, both wanted: (a) the ROW
     must survive even when the PTY cannot, so the user can restart it with a
     click; (b) properly, lossless fd handoff so the PTY survives too.
  2. **THE ROW-ORDER LEDGER IS WRITE-ONLY ON RESTORE — BUILD THE RESTORE PATH.**
     Verified across the 2.12.15 bump: the ledger was byte-identical before and
     after (143 entries, the user's curated order intact) and *nothing read it
     back*. Restored rows land first, adopted live rows are appended after, so
     the user's two live sessions moved from positions 1-2 to 6-7 and they had
     to re-curate by hand for the third time in a day. The snapshot half works;
     only the restore is missing. There is also **no reorder verb**, so an agent
     cannot hand the order back programmatically — add one.
  3. ✅ **RESURRECTION IS FIXED, PROVEN ACROSS A REAL VERSION BUMP.** 8 closed
     rows, 8 tombstones kept, **0 resurrected**, 0 orphaned processes, and the
     daemon self-retired gracefully in 40 s. Keep this result; it is the
     baseline any future change to the import path must not regress.
  4. **A ROW WITH NO RUNTIME IS CORRECT AND DESIRABLE.** User's words: *"No
     runtime is none of our business. The user can click to start it."* The
     model is explicitly GTA 5 vs Crysis — an asset that is not rendered but
     looks rendered. Do NOT reap runtime-less rows; freeing the runtime while
     keeping the row IS the feature.
  5. **yedit AND ychrome CLOSE ON EVERY RESTART AND MUST NOT.** They should stay
     up and stay on their **libyggterm surface**, not fall back to the terminal
     surface.
  6. **★★★ AGENTS MUST DRIVE SHADOW SURFACES EVEN WHILE THE USER'S GUI IS
     CLOSED.** Felt concretely: the a services portal and records agents each drove a ychrome
     session row and the GUI host burned. This is the same requirement as
     server-side rendering — agent browsing should never have been on the GUI
     host (docs/optimization-pass.md WS2, `ychrome/docs/agent-engine.md`).
     Wanted as a real feature, not a workaround.
  7. **DAEMON HANDOVER MUST TELL THE USER AND STOP DRAWING.** On a daemon
     version change the GUI host burns. Spawn a notification ("daemon is
     changing, please wait"), **stop drawing the terminal for the duration**, and
     entertain the user. The render cost during handover is the thing being
     avoided, so the fix is to stop painting, not to paint a spinner harder.
  8. **AUDIO NOTIFICATIONS NEED A PRE-ROLL.** Bluetooth speakers clip roughly the
     first ~300 ms while the link wakes, so the start of every notification is
     lost. The user suggested ~150 ms and invited a better figure. **Use ~400 ms
     of very-low-amplitude noise, not silence** — many A2DP stacks drop or fail
     to prime the link on pure digital silence, so the pre-roll needs a little
     real energy (a dither-level noise floor is enough to be inaudible). Better
     still, make it adaptive: skip the pre-roll when another notification played
     within the last few seconds, since the link is already awake.

- **★★★ USER REQUIREMENTS FOR THE SESSION-ROW LIFECYCLE (stated 2026-07-26, after
  curating the list by hand TWICE).** The user's words: *"A daemon bump and
  restart should not destroy the row order and number of sessions. If destroyed
  this order is supposed to be snapshotted properly. And lastly all the rows not
  connected should die (gracefully is recommended)."*
  1. **A daemon bump must preserve row ORDER and COUNT.** ✅ Verified for a
     GUI-only restart 2026-07-26: 21 rows, byte-identical order across the swap
     (snapshot at `~/.yggterm/manual-snapshots/pre-gui-restart-*`). ⚠ **NOT yet
     verified across a DAEMON bump**, which is the case that actually breaks it —
     rows are re-imported from peer daemons there. The anchored-placement fix
     (`import_peer_live_rows_in_order`) is live but has never been exercised by a
     real daemon swap. **Prove it on the next bump before claiming it.**
  2. **If order is destroyed it must be recoverable from a snapshot.**
     `~/.yggterm/row-order-ledger.json` already records order+membership and
     `removed-rows.json` records closes — but nothing RESTORES from them
     automatically, and an agent had to reconstruct by hand. Build the restore
     path, and make a daemon bump write a pre-swap snapshot the way a deploy
     does.
  3. **⚠ "All rows not connected should die" NEEDS A DECISION, because as
     written it contradicts requirement 1 and the product's core promise.**
     Right now 9 of the user's own curated 21 rows have no runtime — they are
     agent-CLI rows kept precisely so click = resume works
     (`snapshot_session_is_agent_store_recoverable`, the first-class-session
     contract in CLAUDE.md). Reaping every runtime-less row would cut the
     curated list to 12 and delete the resumable history the whole product is
     built on. The user is unlikely to mean that. **Ask what "not connected"
     means to them** — most likely: a row whose CLI transcript is gone, or one
     the user closed, or a plain shell whose PTY died (which IS a husk and
     should go) — and then implement exactly that. Do not guess; an agent
     already deleted seven of their sessions on a guess today.

- **★★★ WE FORCED SOFTWARE GL ON A HOST THAT HAS WORKING HARDWARE GL. The
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

- **★★ THE DAEMONS CHAIN, AND ONE IDLE `bash -i` IS WHY (root-caused
  2026-07-25; the RPC half FIXED, the durable half OPEN).**
  ⛔ **First, a correction to this entry's own earlier wording.** I filed the
  observed "13 Running -> 9" across the 2.12.11 swap as *"a hot restart kills
  live PTYs, violating keep-alive."* That is WRONG. The trace shows those seven
  are `progressive_migration_session_released` events — the **designed**
  kill-and-re-resume by which an agent session is handed to the successor. That
  is exactly why click = resume recovered one at 168x63 with real scrollback.
  Rows never dropped (24 -> 26). Nothing is violated by the release itself.
  **What IS wrong:** the drain that performs those releases had exactly one
  call site — the `disk_binary_replaced` self-retire branch — so an explicit
  `HotRestart` RPC (what a deploy sends) preserved its PTYs and started no
  drain at all. jojo only appeared to migrate because the middle daemon still
  had a thread alive from an earlier self-retire. **FIXED** — the accept loop
  now starts the drain on a preserving handoff, locked both directions.
  **STILL OPEN — the durable half.** `session_kind_is_migratable_agent`
  (`daemon.rs`) admits only `Codex | CodexLiteLlm | ClaudeCode`: a plain shell
  is not re-resumable, and there is **no fd passing anywhere in the tree**
  (`SCM_RIGHTS`/`sendmsg` -> zero hits), so the only way to move a PTY is
  kill-and-re-resume. Therefore **one idle `bash -i` pins its daemon at its
  birth version forever**, and the daemon can never reach empty hands:
  `daemon_should_idle_shutdown` refuses while any terminal session remains, and
  the stale-daemon sweep refuses a local shell. Live on jojo: three of the four
  stranded keys are `bash -i`. Fixing this needs lossless fd-handoff
  (`SCM_RIGHTS`) — that is the real work. A cheaper MITIGATION, and a policy
  call not an obvious win: let a **non-keep-alive** shell on a lingering
  predecessor be reaped so the daemon can drain, trading that shell's live
  scrollback for convergence. It must never touch a keep-alive shell.
  **Diagnose** by `~/.yggterm/hot-update-terminal-owners.json` (runtime key ->
  owner socket + pid) and PTY ancestry — never by row count, which stays
  healthy throughout.

  **★ THE CHEAPER MITIGATION WAS COSTED AND DELIBERATELY NOT BUILT
  (2026-07-26). Do not re-propose it without re-reading this.** Ground truth
  from each daemon's own socket that morning: the 2.12.10 daemon (51 h) owned
  exactly two sessions, both `kind=shell keep_alive=false` — "Secrets Fetch
  Failure Debug" and "Workspace Shell". The 2.12.13 daemon owned an agent
  session **and `local://1c17bfad` "New Yedit", `kind=shell keep_alive=TRUE`**.
  So the reap frees **ONE** of the two supernumerary daemons; the other holds a
  keep-alive shell it may never touch. Say "one daemon", never "36% of the
  total".
  **And what it now recovers is close to nothing.** Both of that daemon's
  costs were global loops, not ownership, and both are fixed above it: the
  perf-incident monitor's whole-corpus re-read (measured 334.7 MB per 90 s,
  byte-identical in all three daemons) and the machine-wide transcript walk
  (908.1 / 908.1 / 454.0 MB per 90 s). With those gone a superseded daemon
  holding two idle shells costs ~0.001 cores and ~33 MB of RSS. **Weigh that
  against killing a live PTY with 51 hours of state in it, named "Secrets Fetch
  Failure Debug" — plausibly a human's debugging session. It is not worth it.**
  **The defect underneath is real and stays open:** the non-keep-alive reap has
  exactly ONE call site, `ServerRequest::PrepareClientClose` (`daemon.rs`;
  `non_keep_alive_live_session_paths` has no other caller). A GUI that is
  SIGKILLed, crashes, or is swapped by a deploy never sends it, so a shell the
  user never marked keep-alive outlives the GUI it contracted to die with —
  AGENTS.md: second-class sessions "survive GUI death IFF marked keep-alive".
  Both 51-hour shells on jojo are that bug, not a policy gap. The right fix is
  to close the path (reap on the successor's first tick when the predecessor's
  client is provably gone), NOT to add a scheduled killer: pace it one per tick,
  oldest-idle-first, with an idle-age floor, gated on `daemon_is_superseded`,
  tracing the title and idle age of every reap. Never touch `keep_alive=true`
  (shell OR agent), an agent kind (those are RELEASED for lossless re-resume by
  `select_next_migration_candidate`, never killed), `remote-session://` /
  `SshShell` rows, or the ~1,167 `server-*.sock` entries — those are symlink
  ALIASES forming the cross-version compat plane, not litter.

- **★★ `app open` CANNOT OPEN A TERMINAL SESSION ON A SHADOW CLIENT (found
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

- **★★ A SESSION STRANDED ON A `preserved` OWNER HAS NO DECLARES, AND THE RAIL
  REBUILD FAILS SILENTLY (found 2026-07-25).** The declare-rebuild
  (`1c88d4a`) asks the daemon that answers `terminal_app_declares`. But after a
  hot restart that could not hand over every PTY, the old daemon keeps owning
  the leftovers: they appear under `preserved_terminal_owner_keys` on the new
  daemon, NOT `owned_terminal_session_keys`. An app running in such a session
  declares to the OLD daemon, so the new one answers "no declares" — and
  because "no declare at all" is not a refusal, **nothing traces**. Observed on
  jojo: `right-panel pane:notes` on a shadow dispatched
  `terminal_app_declares`, completed in ~860 ms, applied nothing, and emitted
  no `daemon_declare_*` reason. The rail simply never appeared.
  ⛔ **CORRECTION to this entry's own fix list.** It proposed "have the
  surviving daemon proxy declares for the sessions it lists as preserved."
  **That proxy already exists** and shipped with the feature (`cb4eff9`):
  `ServerRequest::TerminalAppDeclares` resolves
  `preserved_owner_endpoint_for_request` and forwards to the owning daemon. So
  the design was never missing. Two other things were, both now proven from the
  live trace:
  1. **The owner could not answer.** `TerminalAppDeclares` shipped in 2.12.10,
     and the stranded session's owner was **2.12.9** — it cannot deserialize the
     request, so it writes nothing. The proxy dutifully reported
     `preserved_owner_request_failed {error: "parsing daemon response: \"\""}`.
  2. **The client threw that away.** Both rebuild paths ended in
     `.unwrap_or_default()`, collapsing "the fetch FAILED" into "there are no
     declares" — which is the whole of "no error, no trace." **FIXED**: they now
     branch, and trace `daemon_declare_unavailable` (with the error) separately
     from `daemon_declare_absent` (reached the owner, genuinely nothing there).
  So the remaining gap is only that a pre-2.12.10 owner is unanswerable, which
  the daemon could detect up front from the recorded
  `PreservedTerminalOwnerEntry.owner_server_version` instead of issuing a
  request it knows will fail. Once the daemon chain converges (entry above),
  this stops arising at all.
  **Diagnose** by comparing the two key lists in `server status` — a session in
  `preserved_terminal_owner_keys` is on the old owner. **Unblock** with a fresh
  session on the current daemon (proven: same yedit, fresh session, full rail
  rebuilt on a shadow that never saw the declare).


- **★★★ THE FIFTH FOCUS PATH — IT IS NOT JAVASCRIPT. Root-caused and fixed in
  code 2026-07-26; NOT YET DEPLOYED (needs a GUI bump).** The user, mid-session:
  *"the shadow session spawn took focus away from my viewport and this session
  … it is stealing my focus again and again while working."* Four earlier
  rounds all found JS thieves, and the guard that came out of round four
  (`UI_FOCUS_OWNER_SELECTORS` + the source scan) is intact and innocent here —
  because **this thief never touches the DOM**.
  **The mechanism.** A native web surface is a WebKitGTK webview parented in the
  SAME GtkWindow as the shell's own webview. `gtk_widget_grab_focus` on it sets
  the **GtkWindow's focus widget**, so keyboard focus leaves the shell webview
  while the window stays active and the shell's `activeElement` stays exactly
  where it was. Two call sites did it:
  1. **At birth.** wry's `WebViewAttributes` default is `focused: true`
     (`vendor/wry/src/lib.rs:853`), which `grab_focus()`es in `new_gtk`
     (`vendor/wry/src/webkitgtk/mod.rs:385`). Nothing in the tree ever called
     `with_focused`, so a **headless** `web ensure` surface — created and
     demoted in the same tick, never revealed, no pixel on screen — took the
     keyboard the instant it was built. That is the "spawn took focus away".
  2. **Per verb.** `inject_key` grabbed the focus for every injected keystroke
     and never gave it back, under a comment asserting the grab was
     "widget-local — it does not move the seat's global focus on screen". True
     of the SEAT, false of the TOPLEVEL. `do type` / `do fill` / `fill-vault` /
     `fill-card` / `totp` all route here; `do click` re-takes it through
     WebKit's own focus-on-button-press.
  **The instrument that finally saw it** (every JS-side probe is blind to this,
  and so is `active_session_path`, which never moved): read
  `document.hasFocus()` in the shell AND in the surface **at the same moment**.
  Live on jojo, 16:04: shell `hasFocus:false` / `activeElement`
  `textarea.xterm-helper-textarea` / `window_focused_at_last_watchdog:true`,
  while the invisible agent surface reported `hasFocus:true`,
  `activeElement:INPUT#identityproof`. A surface reporting `hasFocus:true`
  **falsifies** "the window is simply unfocused" — the window is active and the
  agent's page owns its keyboard. The user's terminal recorded its last
  keystroke at 15:47:57 and none for the next 40 minutes
  (`input_batch_flush_count` frozen at 236). 17 focus-taking verbs ran on that
  surface between 15:46:42 and 15:59:24, plus the birth grab at 15:45:29.381.
  **The rule now encoded:** *an agent may BORROW the window's keyboard focus
  around an injection; it may never keep it, and a surface nobody can see never
  gets it at all.* `note_focus_owner_before_injection` books the lender,
  `schedule_focus_giveback` returns it 150 ms after the burst's last event (one
  give-back per burst, so a multi-key fill still costs the page one `blur`, not
  one per character), and it refuses to take focus back off any widget the human
  moved it to meanwhile. `open()` now takes `focused`, which the shell wires to
  `want_visible`. Locked by
  `no_web_surface_takes_the_window_keyboard_focus_without_giving_it_back`.
  ⚠⚠ **It IS keystroke cross-contamination, one direction, CAUGHT LIVE.** A
  passive `keydown` recorder installed in the agent's page logged three
  `isTrusted:true` `Escape` presses — 16:09:35.815, 16:09:51.001, 16:23:42.600 —
  with **no agent verb within ±8 s of any of them** (the agent's last verb ran
  at 16:05:46). That is the human, pressing Escape at a terminal that had
  stopped answering, and landing in an invisible a services portal form instead. The
  other direction is structurally impossible and stays that way: `synth_key`
  hands the event to the surface widget with `gtk_widget_event`, which never
  traverses the toplevel's focus chain, so an agent's characters can never
  reach the user's terminal.
  ⚠⚠⚠ **AND THE ARBITER DID NOT NOTICE — a second bug, still open.** Real seat
  input on a surface is supposed to increment the arbiter's counter and refuse
  the agent's next `do` with `preempted`. Zero `agent_input/preempted` events
  exist for `local://b556fb1b…` and no verb was refused, across the whole
  incident. The reason is the **injection-credit ledger leaking across the
  inter-verb gap**: `grant_injection_credits` books one credit per injected
  event, `note_seat_input` spends a credit instead of counting a human, and
  unspent credits are only dropped by `take_seat_input_count` — whose own
  comment names the hazard exactly ("carrying it forward would let it swallow a
  LATER real gesture, turning a fix for the agent into a bug for the user") and
  which the shell nevertheless calls only at the START of the next verb
  (`web_do_open_lane`'s gate, and between a batch's actions), never at the end
  of the verb that granted them. `do fill --text "0000000000"` grants a dozen
  credits (select-all + delete + ten characters) and — because delivery is
  synchronous, so the lexical `INJECTING_EVENT` flag already suppressed every
  one of them — leaves the whole dozen sitting there. The user's next dozen
  real keystrokes are then silently absorbed as "ours". **That is why
  `0000000000hg` took two of the user's characters into the field with no
  preempt and no journal**, and it is a live co-browse defect
  in its own right: on a surface the human is genuinely sharing, their first N
  keystrokes after any agent verb are invisible to the gate that exists to
  protect them. Fix direction: expire a credit on a short clock (a credit
  unspent 250 ms after an injection cannot belong to a synchronous dispatch)
  instead of holding it until the next verb reads the counter. NOT done here —
  the ledger is what stops the single-shot `do` defect, and it should not be
  changed without a live loop to prove the replacement.
  The other reported corruption — `fill --text "Example Fixture Road"` reporting
  `chars:19 delivered:true` while the field held `Ja` — is **not explained by
  either of the above** (17 characters lost, not 2 gained) and stays open; look
  at the page's controlled-input re-render, not at focus.
  ⚠ **Focus-safe verbs, for anyone driving a surface over a working human:**
  `web eval` / `read` / `wait` / `frames` / `screenshot` / `capture-element`
  never touch GTK focus (guest-JS `element.focus()` sets DOM focus only). Only
  `do` and the `fill*`/`totp` family take it.

- **★★ THE FOURTH FOCUS PATH — FOUND AND FIXED 2026-07-24 (2.12.9). Read this
  before ever "fixing" a focus steal again.** The user could not type in yedit;
  three previous fixes all missed, because every one of them hardened something
  NAMED like a focus path (the reclaim script, the input-policy script, the
  `uiOwnsFocus` allowlist, the covered-host `pointer-events:none`). The actual
  thief is the shell root's **`onclick` handler** in `fn app()`: it fires for
  every click anywhere in the window and `document::eval`s a script that
  refocuses the active terminal's helper textarea. It bailed out for a live WEB
  surface — the same bug was found and fixed there once ("click the new-profile
  field and it loses focus immediately") — but nobody taught it about the
  DOCUMENT surface, which did not exist yet when that bail was written.
  **How it was finally caught** (the method matters more than the fix): patch
  `HTMLElement.prototype.focus` on the live GUI to log any call landing on an
  `.xterm-helper-textarea`, AND wrap the registry's `focusTerminal` /
  `setInputEnabled` / `term.focus` so a hit says WHICH closure ran; then drive a
  REAL `server app pointer click` into the editor. The log read: click lands in
  the editor, ~93 ms later `helper.focus()` fires with an EMPTY marks list and a
  `global code@dioxus://index.html` stack — i.e. a freshly-eval'd script, not
  any registry closure. That empty marks list is what convicted the click
  handler. ⚠ A JS `el.focus()` probe passes while the bug is live; only a real
  pointer click reproduces it, because the thief is a DOM click handler.
  **Fixes:** the Rust bail now includes `document_surface_visible_for`, the
  script is extracted as `root_click_terminal_focus_script` carrying the shared
  `UI_FOCUS_OWNER_SELECTORS` guard (so it also stops yanking focus out of the
  sidebar, the theme editor and settings fields), and
  `every_helper_textarea_focus_site_is_guarded_or_a_recorded_probe` scans the
  source so a FIFTH script cannot hide the same way — enumerating these by hand
  is exactly what let this one survive three rounds.

- **★★ WHEN THE GUI DIES, NOTHING BRINGS IT BACK — measured 2026-07-25, then
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

- **★★ AGENT WEB-SURFACE AUTOMATION HARD-CRASHES THE GUI (WebKitGTK
  segfault) — diagnosed 2026-07-24 on jojo; LAYER 1 (crash surface) FIXED +
  LIVE-VERIFIED at 2.12.8 (`c3c7086`), LAYER 2 (routing/isolation) OPEN.**
  **UPDATE 2026-07-24 (dev agent):** the raw-coordinate `do click` path was the
  culprit — it synthesized a native GDK button event with NO hit-test, unlike
  `ClickSelector`. Fixed in `web_surface_do_for`: the `Click{x,y}` arm now evals
  `document.elementFromPoint(vx,vy)` FIRST and refuses (never injecting) if it
  returns null or the eval fails — which both confirms a live element is present
  AND round-trips through the web content process, so a page that cannot lay out
  fails there instead of taking a synthetic click into a dying frame. Live-proven
  on the fixed GUI (jojo pid 3290202, GUI-only swap, daemon + all 6 sessions
  preserved): a blind click at (5000,5000) into a MAPPED 1mg surface is refused
  with "no live element … refusing a blind native click"; a valid `--selector`
  click succeeds; the GUI survives every blind click that previously segfaulted
  it. Prefer `do click --selector`. **STILL OPEN (layer 2):** a WebKit-internal
  race on a *valid* element is not fully preventable from the UI process — the
  ultimate belt is process isolation (run agent web surfaces in a shadow/child
  process that can die alone) or GUI auto-restart (the transient scope has no
  `Restart=`), plus the SHADOW-PROBE routing so agent web verbs never drive the
  user's foreground surface. Those are the remaining fixes.
  A `web_surface_do` synthetic click injected into a `local://<uuid>` web
  surface segfaulted WebKitGTK and killed the entire GUI process. dmesg:
  `yggterm[<pid>]: segfault at 48 ... error 4 in
  libwebkit2gtk-4.1.so.0.21.8` — a null-pointer read (deref at struct
  offset 0x48) inside WebKit. The GUI's last two trace events before death
  were the trigger: a `web_surface_eval` DOM scrape
  (`document.querySelectorAll("*")`) then a `web_surface_do` primary click
  at (122, 514). **Not OOM** (no oom-kill at crash time, memory healthy);
  **not a Rust panic** (`panic.log` untouched — a native C++ crash bypasses
  the Rust panic hook, so the process just takes SIGSEGV). The GUI runs as
  a one-shot transient systemd scope (`app-dev.yggterm.Yggterm@<uuid>`, no
  `Restart=`), so once it died nothing relaunched the window. The daemon
  (separate process) survived and kept owning every PTY, so all live agent
  sessions were unaffected — the crash was cosmetic to the work, but the
  user lost the window.
  **Two failure layers, both need a fix:**
  (1) *Crash surface:* a synthetic-click / DOM-eval into a WebKitGTK web
  surface can null-deref inside WebKit. The injection path must be guarded
  (validate the target surface/element is live before dispatch; catch/
  isolate the webview call so a bad injection cannot take down the whole
  GUI process — ideally the web surface is a child that can die alone).
  (2) *Routing violation:* this web-surface automation was aimed at the
  user's **active GUI** instead of a shadow view-client — exactly what the
  SHADOW-PROBE LAW forbids (untargeted verbs route to the active client =
  the user's GUI). `web do/eval/wait` verbs should refuse to drive the
  active user GUI and require a shadow/backgrounded target, or spawn one.
  **Broader pattern:** this is not a one-off — ~20 yggterm segfaults in a
  single day's dmesg (webkit/glib/libc) and dozens of `failed`
  `Yggterm@*.service` scopes; the web-surface automation path (landed
  2026-07-23 in the agent-client no-activate + shadow-probe commits) is the
  freshest suspect. **Recovery gotcha (found live):** a leftover shadow
  view-client intercepts GUI relaunch — a plain `yggterm` launch and
  `server app launch` both get handled by the registered shadow (it tries
  to focus its own headless `wayland-1` window and fails) instead of
  spawning the primary GUI. Tear the shadow down first
  (`scripts/shadow-client.sh stop --name agent-1`), then launch the primary
  GUI with the KDE `wayland-0` env — it re-attaches to the surviving daemon
  with no re-resume (live-verified: 6 owned · 6 total · 0 preserved).

- **★ AGENT SHADOW CLIENT FOR THE TERMINAL LANE — E2E LIVE ON JOJO
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

- **★ USER RE-CONFIRMED 2026-07-23 (during the 2.12.7 session): codex sessions
  still paint COLD-START JSON GIBBERISH** — raw conversation prose as wrapped
  plain text, duplicated turns, no codex TUI chrome, on a cold launch. This is
  the motivating repro of `docs/spec-agent-cli-harness.md` (§7.6: the attach
  seed has TWO WRITERS by construction — daemon seed + client reveal replay),
  and its structural fix is the spec's phase 0/3. The spec build is gated on
  the user's explicit go; when given, the acceptance test is: a cold-launched
  codex session must be pixel-indistinguishable from a manual
  `ssh -t <machine> codex resume <UUID>`.
  **Same report, swap-window frames:** two clipboard frames captured at 13:41
  (broken bottom-line interleave, then a blank viewport) fall inside the
  GUI-swap settling window ~1–3 min after the 2.12.7 GUI relaunch; the surface
  settled clean by 13:47 (faithful screenshot, bottom intact) and mount churn
  stopped. Deploy-window transients are a documented class (field guide §4.4);
  what changed in 2.12.7 is that input returns in seconds, births mount once,
  and a detected ring gap reconciles — the remaining swap-window paint
  transient is the attach-seed seam the harness spec owns.

- **libyggterm apps over a MANUAL ssh hop say "not inside yggterm"
  (user-confirmed 2026-07-23).** Spawn a local yggterm terminal, `ssh <host>`,
  run `yedit` there → detection fails because `YGGTERM_SESSION_ID` does not
  cross a user-typed ssh hop. TWO halves:
  1. **Detection — ACTIVE on jojo-local (2026-07-23, 2.12.8 daemon swap):**
     the daemon exports `LC_YGGTERM_SESSION_ID` at PTY spawn (the iTerm2
     `LC_TERMINAL` trick — stock OpenSSH forwards `LC_*` both ways by
     default), and yedit falls back to it. Live-proven: a fresh jojo PTY
     echoes the session key from `$LC_YGGTERM_SESSION_ID`. ⚠ PTYs owned by
     REMOTE machines' daemons (dev/oc fleet, B1-parked) still predate the
     export until those daemons bump.
  2. **Control-channel attribution — DESIGNED, NOT BUILT:** even with
     detection, the app's declared control endpoint is loopback on the REMOTE
     host, and the GUI resolves forwards from the SESSION's `ssh_target` —
     which is local for a manual hop, so the fetch dials the wrong machine and
     the surface dies as "not responding". Design: the declare payload carries
     the app host's identity (`gethostname()`); the GUI maps it to a known
     remote machine (requires a hostname↔machine mapping the remote-machine
     registry does not hold yet — `RemoteMachineSnapshot` has `ssh_target` and
     `label` only, and oc's hostname ≠ its alias) and spawns the `ssh -L`
     against that machine. Until built, the honest state is: detection works
     (post-bump), surface takeover over a manual hop does not; running the app
     in a session yggterm itself opened on that host works fully.

- **Blank viewport from a DETACHED `term.element` (jojo, 2026-07-22).** The
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

- **★ ROOT-CAUSED + FIXED 2026-07-23 (2.12.7): the vanishing client-instance
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

- **THE STALE-DAEMON TRAP — read before diagnosing ANY "the fix didn't work".**
  A deploy that lands new binaries does NOT mean the new code is running. The
  daemon's idle gate defers its own retirement while any owned session is
  actively working — and on a campaign machine an agent session is ~always
  working, so the daemon can stay pinned indefinitely. On jojo 2026-07-11 the
  daemon ran **2.10.3 for 19h44m while 2.10.13 sat on disk**: the CR-faithful
  sanitizer fix and the CC re-birth fix from campaign run 1 were compiled,
  deployed, and never executed. Both bugs were still live for the user, and run 1
  had recorded them as "fixed on branch, live-verify pending" — the gap was
  invisible.
  **Always check `yggterm-headless server status → server_version` against the
  on-disk binary BEFORE concluding anything about a fix.** As of 2.10.14 the
  metadata sidebar's Daemon section surfaces version, uptime, a
  newer-build-on-disk flag, and the daemon's own deferral reason, plus a manual
  hot-restart button — so this is visible in the product rather than only to an
  agent who thinks to look.

- **★★ THE CLICK RENDER STORM — root-caused live 2026-07-23 (user repro:
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

- **Rendering stability: user RE-REPORTED blinking + blank-on-switch 2026-07-23
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

- **Cross-pathway blink (local-cc → remote-cc switch) — BOTH DEFECTS FIXED in
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

- **Live-path frame corruption on busy CC sessions (jojo, 2026-07-10).** While
  an agent streams heavily, the CLIENT xterm buffer accumulates single-cell
  holes (`t ik` for `think`, including the user's own composer echo), merged
  rows, and whole frames interleaved at wrong positions — while the daemon
  vt100 screen stays clean and no `resync_required`/`cursor_rewound` events
  fire. So bytes are lost/mutated between the daemon read and `term.write` in
  the GUI. The ATTACH-seed variant of this class is fixed in 2.10.4 (viewport
  reconcile chunk); the live-path variant is still open. Prime suspects:
  (a) `batch_terminal_chunks` sanitizers rewriting live frames (the
  `observation` rejoin converts `\r\n`→`\n` and strips "noise" lines whenever
  a batch lacks alt-screen/hide-cursor/high-volume markers — content-triggered,
  so yggterm-dev sessions whose transcripts CONTAIN transport-noise phrases are
  hit hardest); (b) `terminal_write_bridge.stage_or_immediate` ordering under
  frame-budget mode. 2.10.4 ships the probes to convict: mine
  `terminal_forward_divergence` + `terminal_write_send_failed` in
  `event-trace.jsonl` and run the client-buffer vs daemon-screen diff recipe in
  `.agents/skills/yggui-app-control/SKILL.md` while a session streams.
  **UPDATE 2026-07-11 (telemetry campaign run 1): suspect (a) CONFIRMED.**
  `terminal_forward_divergence` fired on jojo (4/5 events on `local://`/`live::`
  sessions, drops of 1-11 bytes), and code trace convicted the sanitizers:
  `strip_internal_terminal_transport_noise_lines` did `.replace("\r\n","\n")` over
  the whole batch (content-gated on transport phrases, so it hits local dev
  sessions), and `strip_low_signal_terminal_noise_lines` used `str::lines().join`
  - both drop carriage returns, so xterm paints the next line at the wrong column
  (the staircase/interleave garble). Fixed in 2.10.13: both now `split('\n')`
  (CR-faithful); regression test
  `batch_terminal_chunks_preserves_carriage_returns_in_kept_lines`; the probe now
  emits `cr_dropped`. Suspect (b) not yet investigated.

  **UPDATE 2026-07-11 (run 2): the CR fix was NOT the whole bug — the excision
  itself is.** User re-reported (in different words): "local sessions are dropping
  chars sometimes and replacing the rendering with spaces." Run 1 sized the drops
  at 1-11 bytes and assumed CR loss was the entire mechanism. Re-mining
  `terminal_forward_divergence` found the real magnitude on the user's OWN session:

      local://20e56a8b   raw 9153  → forwarded 8474   = 679 bytes dropped
      local://20e56a8b   raw 23991 → forwarded 23312  = 679 bytes dropped

  679 bytes is a whole-line EXCISION, not a lost `\r`. Mechanism:
  `strip_internal_terminal_transport_noise_lines` content-matches three phrases
  (`terminal session not found`, `ignoring stale yggterm daemon…`, `hot update
  failed…`) and on a hit ALSO sets `drop_following_transport_tail_lines = 3` —
  deleting the matched line **plus the next three lines** of whatever the CLI was
  painting. A Claude Code session whose conversation quotes those phrases (an agent
  working on this very bug does) has four lines removed mid-frame. The daemon vt100
  screen stays clean, so every daemon-side instrument reports the session healthy —
  which is why this survived a run. Making the excision CR-faithful stopped the
  staircase garble but not the deletion.

  **Why it was NOT fixed in 2.10.14:** the excision cannot simply be removed. `ssh`
  writes `Shared connection to <ip> closed.` into the PTY, and yggterm's remote
  helper prints `Error: terminal session not found: <key>` to its stdout, which IS
  the PTY. Both arrive inside cursor-hide control batches, so no content-based or
  branch-based rule separates them from CLI output (5 existing tests lock this).
  The real fix is **per-session attach-phase state** — sanitize only while the
  launch wrapper owns the PTY, be a faithful pipe once the CLI does. That is the
  "collapse the forks / delete the accreted fixes" step of
  `campaign-render-pipeline-parity-rework`, which the user sequenced AFTER the
  parity harness. Deliberately not rushed into a deploy. The measurement, the
  mechanism, and the reason it can't be a one-liner are recorded in code at
  `batch_terminal_chunks`. **This is the next thing to do on that campaign.**

  **UPDATE 2026-07-20 (run 5): now USER-BLOCKING, and it reproduces hardest on
  the busiest remote-CC session.** The user reported a session that "100% never
  renders", where closing and reopening the GUI — their standing workaround —
  had stopped working. Named session: `remote-cc://dev/029a3955…`
  ("libyggterm Rebase"). Evidence gathered this run:

  - **The corruption is in the client BUFFER, not the paint.** `app terminal
    read-buffer --mode screen` shows three different screen states interleaved
    character-by-character on the same rows (an old report, a test-code frame, a
    `/context` usage panel, plus a stray line-number column). The faithful
    screenshot merely renders that corrupt buffer honestly, so this is NOT a
    canvas/renderer problem — do not chase the renderer again.
  - **It survives every repair that does not fix the pipe.** Two real SIGWINCHes
    (PTY winsize verified changing 63×167 → 62×166 → 63×167 on dev, so CC
    definitely re-authored its frame) left the buffer byte-identical in the
    corrupt regions; GUI restarts and repeated `app open` reveals do not stick.
    The attach/replay seed is clean (fixed in 2.10.4), so a fresh reveal paints
    correctly and then **re-corrupts within seconds** of live streaming.
  - **Why THIS session and not the neighbouring one.** CC on dev is writing
    ~1.2 MB/s (`/proc/<pid>/io` write_bytes +6 MB in 5 s). High throughput means
    more batches, and the excision is content-triggered — and this session's
    transcript is saturated with the exact transport phrases the sanitizer
    matches ("dropped", "eval failed", "never armed", and it literally quotes
    `terminal session not found`). The calm local session in the same window
    showed no such corruption. That is the "hit hardest" prediction above,
    confirmed on a session the user cannot use.

  **CORRECTION, same run — the sanitizers are NOT the cause of THIS symptom.**
  It was tempting to file the above under suspect (a) because it matches the
  narrative, but the probe refuses it: `terminal_forward_divergence` fired
  **3 times in the whole trace, all on an unrelated `live::5d0e22ed…` plain
  shell, and ZERO times on `remote-cc://dev/029a3955`**. The GUI forwards the
  daemon's bytes faithfully for the corrupted session. Two further facts clear
  the excision specifically: the per-line predicate requires a SCHEME-QUALIFIED
  match (`local://`, `remote-session://`, `codex-runtime://` — note
  `cc-runtime://` is absent), so prose quoting the phrase is already guarded by
  `batch_terminal_chunks_keeps_prose_about_missing_sessions`. An attach-phase
  gate for `batch_terminal_chunks` was written and then **reverted unshipped**
  because it fixed a bug this session does not have. Suspect (a) remains real
  for the sessions where divergence DOES fire; it is simply not this.

  **The actual mechanism, read off the raw stream.** The agent CLI paints by
  skipping unchanged cells with cursor-forward, not by overwriting them — the
  daemon-side bytes for this session are literally
  `❯ On\x1b[C the\x1b[C meta\x1b[C page` and `t\x1b[8C html`, i.e. every space
  and every run of spaces is a CUF. **Cells that CUF skips keep whatever was
  already in them.** So once the client buffer's base state diverges from the
  frame the CLI believes is on screen, every skipped region shows stale content
  and the CLI never rewrites it — permanent, character-by-character
  interleaving, exactly what is on screen. It re-corrupts within seconds of a
  clean reveal because the very next diff frame paints against the wrong base.

  **Next step (unverified hypothesis, do not ship on it):** find where the
  post-attach live stream resumes relative to where the attach replay stopped.
  A seam — overlap or gap — between the replayed snapshot and the live stream
  would leave the client buffer holding a base the CLI never authored, which is
  all it takes. A gap is consistent with a high-throughput session being hit
  hardest (~1.2 MB/s here). Note that two real SIGWINCHes did NOT repair it,
  which needs explaining: a resize normally forces a full repaint, so either CC
  did not receive it or its own full repaint is also CUF-based against a stale
  model. Settle that first — it discriminates between "client base is wrong"
  and "CLI model is wrong".

  **FIX SHIPPED 2026-07-23 (2.12.7): the seam is the chunk-ring mid-stream
  gap, and `read()` now appends the viewport reconcile after the surviving
  tail whenever `resync_required` fires** — the live-path twin of the 2.10.4
  attach-seed reconcile (viewport-only, alt-screen-safe, no history
  injection, so it does not re-open the 2.8.12/14 trap). Daemon trace
  `mid_stream_gap_reconciled` fires per reconcile; lock:
  `pty_read_with_trimmed_middle_appends_viewport_reconcile_after_tail`. Full
  design + trap analysis:
  [`docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream`](xterm-bugs.md#chunk-ring-trim-drops-mid-stream).
  **Remove this entry once re-measured under a busy streaming session**
  (read-buffer vs daemon-screen diff staying clean while
  `mid_stream_gap_reconciled` fires; the SIGWINCH question is answered by the
  mechanism — CC's repaint is diff-based against its own model, so only
  re-anchoring the client base can help, which is exactly what the reconcile
  does).

  **★★ UPDATE 2026-07-25 — A SECOND MECHANISM IN THIS FAMILY, FOUND WITH THE
  SHADOW LANE, ROOT-CAUSED AND FIXED. It also answers this entry's own open
  question about the SIGWINCHes, and that answer is NOT the one guessed above.**
  Reproduced on jojo against a live `remote-cc` session, and settled with GROUND
  TRUTH for once: the CC transcript on the remote says `of exam manipulation`
  and the terminal painted `uof examrnmanipulation`. Full write-up, including
  the socket-probe recipe that measures a screen payload's true width:
  [`docs/xterm-bugs.md#screen-model-wider-than-viewer`](xterm-bugs.md#screen-model-wider-than-viewer).
  - **The daemon's vt100 SCREEN MODEL had drifted wider than its own PTY** —
    model ~204 columns against a 168x63 PTY and a 168x63 viewer. Everything past
    column 168 is a ghost from when the grid was wider, because the CLI cannot
    paint wider than the grid it was handed.
  - **Why that garbles rather than overflows:** the screen is serialized with
    absolute `CSI r;cH` per row and `CSI nC` for runs of blanks. In a narrower
    terminal each over-long row WRAPS, shifting every row below; the later
    absolute jumps land on that spill, and the blank-runs skip cells instead of
    clearing them, so the spill shows through in the gaps. Same CUF mechanism
    this entry already names — but the wrong base is manufactured INSIDE a
    single reconcile write, not inherited from a stream seam.
  - **⛔ The SIGWINCH answer above is wrong.** It is not (only) that CC repaints
    diff-wise: `TerminalSession::resize` returned `resize_noop` after comparing
    the PTY alone, so a resize to the size the PTY already had **never touched
    the stale model**. Two real SIGWINCHes could not have repaired it.
  - **FIXED in three layers** (2 daemon, 1 client): the served screen is clipped
    to the session's own PTY width at the one place it is served
    (`screen_snapshot_clipped_to_pty_width`); the resize fast path now compares
    the model too and repairs it (`resize_screen_model_repaired`); and the client
    reconcile measures the payload and refuses to paint one wider than its own
    grid (`screen_reconcile_clipped_to_viewer_width`) — which is what protects a
    viewer attached to an OLDER daemon, the live case here.
  - ✅ **All three layers are now DEPLOYED on jojo (2.12.13, daemon pid
    1152900, 2026-07-25 evening).** ⚠ But read the next line before assuming a
    given session is covered.
  - ⚠ **A daemon-side fix only covers the sessions that daemon OWNS.** After the
    swap, `local://5220ce5d` (the 120x36 shell with the 295-wide model) is still
    served at 295, because a plain shell is not migratable and stays with its
    2.12.12 birth daemon — the durable half of the daemon-chaining bug above.
    For every such stranded session the CLIENT clip is the whole protection, and
    it is proven. Post-swap the two daemon-side events are correctly SILENT: a
    daemon that has just started has no drifted model to clip, so silence here
    is the expected reading, not evidence of a working fix.
  - ✅ **The guard HAS now refused an oversized payload live (2026-07-25
    evening, before the 2.12.13 deploy).** Walking every session on the live
    2.12.12 daemon found a **plain `bash -i` shell** — not an agent session —
    with a **120x36 PTY and a model still painting to column 295**. Revealed on a
    read-only shadow running the 2.12.13 client (pinned to the daemon's 120-wide
    grid), it traced `screen_reconcile_clipped_to_viewer_width
    {"screen_max_column":295,"viewer_cols":120}` and painted clean text — with
    the user's GUI untouched. Two consequences worth carrying: **this class is
    not CC-specific** (any session that outlives a window resize can carry it),
    and the mixed-version case layer 3 was written for is now demonstrated, not
    argued (the rail read `Client 2.12.13 · daemon is on 2.12.12`).
  - ⚠ **Why it reads as intermittent:** the drift heals on any resize whose grid
    DIFFERS from the cached one; only a resize to the size the PTY already has
    hits the `resize_noop` hole. Same session garbles, "fixes itself" after a
    window resize, garbles again.
  - ★ **Method note:** this was found on a shadow client with the user's GUI
    untouched, and the decisive step was reading the daemon's screen off the
    socket instead of trusting any summary field. `server snapshot` is NOT that
    instrument — for a session on a preserved owner it answers with the stale
    stored launch seed, which looks like a healthy session with nothing wrong.

- **Remote CC session stays permanently blank: `resume-cc` deadlocks before it
  launches the CLI (dev, 2026-07-20).** User-reported as "it never renders", and
  it is NOT a render bug — the xterm buffer is genuinely empty (0 non-whitespace
  chars), so the blank viewport is honest. On the remote host the wrapper
  `yggterm server remote resume-cc <uuid> <cwd> --require-existing` sits in
  `unix_stream_read_generic` (blocked on a daemon unix socket) for many minutes
  with **no children** — it never spawns `claude` at all, so the PTY produces
  nothing forever. `Status` in the metadata rail reads `bootstrapping · idle`.

  **Neither workaround clears it.** Re-clicking the row just logs
  `terminal_bootstrap_existing_lease_skip` ("bootstrap skipped because an
  existing attach lease ...") — three attempts in a row did that here, none
  reaching `ready`. A full GUI restart does NOT fix it either (verified: fresh
  GUI, re-open, still 0 chars), which rules out GUI-side in-memory lease state
  as the blocker and matches the user's "even the workarounds do not work".

  **Recovery that DOES work:** kill the stuck wrapper on the remote host
  (`pgrep -af "resume-cc <uuid>"`, it has no children and holds no user work);
  the next open spawns a fresh wrapper which does launch `claude --resume`, and
  the session comes back with full scrollback. Confirmed end-to-end on
  `remote-cc://dev/75874380…`.

  **Prime suspect: the dev daemon fleet.** dev is still running **six**
  `yggterm-headless server daemon` processes (the consolidation item carried
  from telemetry run 3, [[finding-adopt-gap-untypeable-fixed-2113]]). A helper
  that connects to a stale/wrong daemon socket and blocks forever on read is
  exactly this signature. Fix direction: (1) consolidate dev's daemons, (2) give
  `resume-cc` a connect/read deadline so it can never block indefinitely before
  spawning the CLI, and (3) make `terminal_bootstrap_existing_lease_skip`
  reclaim a lease whose attach never reached ready, instead of deferring to it
  forever.

  **FIXES SHIPPED 2026-07-23 (2.12.7, both halves of the recorded direction):**
  (2) the wrapper bridge now bails after 120 s if the daemon claims `running`
  but the runtime has produced ZERO output ever
  (`bridge_running_no_output_deadline` trace; idle-but-healthy sessions are
  unaffected — the flag is has-ever-produced-output), so the next open spawns
  a fresh wrapper instead of requiring a manual pkill; deployed to dev's
  `~/.yggterm/bin` where the wrapper runs. (3) a re-click now RECLAIMS a
  bootstrap lease whose attach never reached ready after 45 s
  (`terminal_bootstrap_lease_reclaimed_stale_attach`; lock:
  `terminal_bootstrap_lease_reclaims_stale_never_ready_attach`). (1) dev
  daemon consolidation stays parked with B1 (user call: investigate-only).
  Remove this entry once a wedged resume recovers without manual intervention.

## Deployed live on jojo, faithful-gesture confirmation pending

- **Middle-click a link in a web surface → new tab (2.10.15, c6542edc).** Root
  cause found + fixed: the surface's WebView wired no `new_window_req_handler`, so
  WebKit's `create` signal (middle-click, ctrl/cmd-click, `target="_blank"`,
  `window.open`) returned a null widget and the link was dropped. Now routed into
  yggterm's tab model — background tab for middle/ctrl-click, foreground for
  `window.open`/`_blank`; egress + profile inherited. Unit-tested on the tab-model
  half. Kept GUI-only (no protocol bump) so it deploys against a running
  same-version daemon with no changeover. **Deployed to jojo 2026-07-11** via a
  GUI-only restart (new `~/.local/bin/yggterm` build, SIGTERM+relaunch, the three
  live daemons untouched — verified same PIDs before/after; new GUI pid confirmed
  answering app-control). **Still pending:** a FAITHFUL confirmation, which needs a
  real middle-click — the Xvfb harness is native-surface-blind, app-control clicks
  never reach a child webview, WebKitGTK blocks synthetic `window.open` (no user
  gesture), and jojo's Wayland input injection is unreliable (ydotoold). Ask the
  user to middle-click a link in a ychrome surface; confirm via the
  `web_surface / new_tab_from_link` trace event.

## Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.
