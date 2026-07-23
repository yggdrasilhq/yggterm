# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

## Standing traps / other open bugs

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
  is born in a **PARTIAL `term.open()`**, and this is proven deterministically
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
