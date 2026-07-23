# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

## Standing traps / other open bugs

- **★★★ ROOT-CAUSED + FIXED 2026-07-23 — "the viewport blinks and stops taking
  keystrokes for a few seconds" was PROGRESSIVE MIGRATION EATING ITS OWN LIVE
  SESSIONS.** Reported as "I cannot type in the current session if I switch to
  another window, say chromium, and come back"; refined live by the user to
  "fast blinks, no input, settled within 3 sec".
  **Cause.** `spawn_progressive_session_migration` (daemon.rs) is the retiring-
  predecessor handoff: each tick it releases the oldest-idle agent session so a
  newer successor can adopt it. Its guard asked only *"is any other daemon
  reachable"* — not *"is it a SUCCESSOR"*. jojo ran three daemons: the live
  2.12.3 the GUI was attached to (owner of every session) plus abandoned 2.11.4
  and 2.11.5 lingerers from earlier deploys. The newest daemon read two OLDER
  peers as its successor and began releasing its own live agent PTYs, one per
  5 s tick. Nothing adopted them, so the client re-ensured each released session
  on the SAME daemon → `terminal_runtime spawn` → agent-CLI **re-resume** →
  blank/blinking viewport that swallows keystrokes for ~3 s, roughly once a
  minute, forever. Trace signature: `progressive_migration_session_released
  {runtime_removed:true}` followed ~15 s later by `terminal_runtime spawn` +
  `first_bytes` for the same runtime key; **the same key was released seven
  times in twenty minutes and never converged.**
  **Why the window-switch framing was a red herring:** the trigger is the idle
  timer (`idle_ms` 45–50 s every time), and being away in another window is just
  the most common way to be idle. A genuine compositor focus-out/focus-in cycle
  reproduced nothing (clean recovery at 6 s and at 118 s away).
  **Fixes (2.12.4):** only a strictly NEWER peer counts as a migration successor
  (unparseable versions fail closed); and a runtime key that comes back into our
  hands is released at most `MAX_MIGRATION_RELEASES_PER_KEY` times, with a loud
  `progressive_migration_session_returned` trace — lingering beats churning.
  Regression locks: `only_a_strictly_newer_daemon_counts_as_a_migration_successor`,
  `a_returning_runtime_key_stops_being_a_migration_candidate`.
  **Second contributing defect FIXED:** a freshly created agent session was NOT
  keep-alive, and `keep_alive` is the only input to `restart_protected_runtime`
  in `terminal_reuse_needs_restart` — so an unprotected agent row also had its
  PTY re-spawned on any transient stale/blank remote-attach reading. Agent CLI
  kinds are now born keep-alive (`session_kind_persists_by_default`); restore
  stays authoritative so an explicit opt-out and the update-restore distinction
  both survive.
  **Probes shipped alongside** (keep them; they are how the next one is decided
  in a single command): `ui/window_focus/transition`, `ui/input_policy/applied`,
  and `app state` → `input_dead_ms` / `passive_focus_recovery_state` /
  `input_dead_active_element`. Also fixed: the passive focus watchdog bailed on
  `document.hasFocus()`, a measured Wayland false negative for a foreground
  window and redundant with `inputEnabled`.
  ⚠ **KNOWN GAP:** `PersistedLiveSession.keep_alive` is a bool, so "user turned
  keep-alive OFF for an agent session" and "born before 2.12.4" are
  indistinguishable across a restart. Pre-2.12.4 agent rows stay blue until
  marked once. A tri-state (unset/on/off) is the real fix if that ever matters.
  Full entry: [`docs/xterm-bugs.md#input-dead-after-window-refocus`](xterm-bugs.md#input-dead-after-window-refocus).

- **★ NEW 2026-07-23: an agent row born through `open_or_focus_session` is NOT
  keep-alive — the born-green rule has a second birth site it never covered.**
  Live-caught during the 2.12.5 deploy recovery: all 13 `remote-cc://dev/*` rows
  reconnected via `yggterm server connect` came back `keep_alive: false` (each
  had been `true` before the swap), under a 2.12.5 daemon that carries the
  born-keep-alive fix. Cause: `server connect` → `ServerRequest::OpenStoredSession`
  → `open_or_focus_session` → `build_session` — a SECOND live-row birth
  constructor, and only `insert_live_session_with_launch` applies
  `session_kind_persists_by_default`. Consequence is not cosmetic (same family as
  the round-7 second fix): `keep_alive` is the only input to
  `restart_protected_runtime`, so every connect-birthed agent row is eligible for
  a PTY re-spawn/re-resume on any transient stale-attach reading. Fix direction:
  apply the born-green default at the `build_session`-birth site too (only when
  the row is newly created — an existing row's flag must never be overwritten),
  or better, collapse the two birth sites into one owner. Recovery meanwhile:
  `server app terminal keep <path>` per row (verified working).
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

- **★ A GUI RESTART CAN LEAVE APP CONTROL PERMANENTLY UNREACHABLE — the client
  instance record vanishes after a SUCCESSFUL register (jojo, 2026-07-22).**
  After a routine GUI-only swap the GUI was alive, visible, and usable by the
  user, but every `server app …` verb failed with **"no live Yggterm GUI client
  is registered for app control"** — i.e. the entire agent control plane was
  down, with no symptom the user would ever notice. The whole yggui workflow
  (screenshot, state, dom-eval, probes) is dead in this state.
  **Evidence, not inference.** The trace shows a normal, successful
  registration: `launch_shell_register_begin` → `register` (with the exact
  record path under `client-instances/unix--home-pi--yggterm-server-2-12-3-sock/`)
  → `launch_shell_register_end {ok: true}` → `duplicate_app_instance_suppressed`
  — a sequence **byte-identical to the two prior GUIs that registered fine**.
  Yet the directory was empty afterwards, with an mtime in the same second as
  the register. So the record was written and then deleted, while its process
  stayed alive.
  **Falsified, so don't re-derive:** the scan predicate is fine — a
  hand-reconstructed record with the live pid and correct `process_start_ticks`
  **survives** repeated `active_client_instance_paths_for_scan` passes, so
  `client_instance_record_matches_live_process` is not the deleter. Nor is
  `terminate_superseded_client_instances`: it skips its own record explicitly
  (`linux_client_record_requires_app_id_isolation` returns false when
  `record_pid == current_pid`). **The deleter is unidentified — do not guess it,
  instrument it:** the cheap next step is a trace event at every
  `fs::remove_file` in `cleanup_stale_client_instances` carrying the removing
  pid, the removed pid, and which predicate rejected it.
  **★ RECOVERY WITHOUT ANOTHER RESTART (verified — this is the valuable half).**
  `CLIENT_INSTANCE` is a `OnceCell`, so the GUI never re-registers; but the
  record is just a file, and recreating it restores control immediately. With
  the live GUI pid:
  ```bash
  D=~/.yggterm/client-instances/unix--home-pi--yggterm-server-<VER>-sock
  TICKS=$(awk '{print $22}' /proc/<GUIPID>/stat)   # field 22 = starttime
  printf '{"pid":<GUIPID>,"started_at_ms":<MS>,"client_id":null,
  "linux_desktop_app_id":"dev.yggterm.Yggterm","process_start_ticks":'$TICKS',
  "executable_path":"/home/pi/.local/bin/yggterm","display":":1",
  "wayland_display":"wayland-0","xdg_session_id":"","xdg_runtime_dir":"/run/user/1000",
  "xauthority":""}' > $D/<GUIPID>-<MS>.json
  ```
  The filename **must** be `<pid>-<started_at_ms>.json` (the pid is parsed back
  out of it and must match the record) and `<MS>` should be the `started_at_ms`
  from the `register` trace event. Confirm with `server app clients`.
  ⚠ **Check `server app clients` after every GUI restart** — a restart that
  looks clean can silently take the control plane down, and the failure is
  invisible until an agent tries to use it.

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

- **★ NEW 2026-07-23, NOT ROOT-CAUSED: the blink is on a CROSS-PATHWAY session
  switch.** User: *"I switched it from a local-cc session to this remote-cc.
  Switching out to a remote-cc to this remote-cc solved it. The local-cc also had
  a blinking issue when closing."* So **local-cc → remote-cc blinks; remote-cc →
  remote-cc does not** — same-pathway switches are fine. Trace for the episode:
  **11 mounts in 15 minutes** for the one session, the mount generation going
  BACKWARD (`m3` → `m1`), **each reveal CONSTRUCTING TWICE ~0.5 s apart**,
  `terminal_render_health_unhealthy` at construction, and
  `remote_pty_resize_failed {error: "…terminal session not found:
  cc-runtime://<id>"}` right after each mount (5 in 10 min; `remote_yggterm_retry_total`
  held steady at 17, so this is NOT the runaway cache-reset spin).
  Two things to chase, in order: (1) **one reveal should mount once** — the double
  construct is the blink; (2) why the remote daemon does not recognise the
  `cc-runtime://` key after a local→remote switch. This is the user-visible face
  of the pathway drift recorded as spec work in the campaign
  (`{remote,local}×{cc,codex}` unification) — make "switch local-cc → remote-cc"
  a first-class acceptance case there.

- **B4 ROOT CAUSE FOUND (jojo, 2026-07-22): the cold-restore refusal is
  ALL-OR-NOTHING, and rows owned by NOBODY have no recovery source.** Measured
  end-to-end on the live 2.12.2 → 2.12.3 swap: the sidebar went **25 rows → 12**,
  losing 13 `remote-cc://dev/*` rows. This is NOT the dormant-adoption gap that
  `ad4a595` closed — that fix is present and ran.
  **Mechanism, with the trace evidence:**
  1. The successor boots, sees other live daemons, and
     `may_cold_restore_live_sessions` refuses → `daemon.rs` runs
     `saved.live_sessions.clear()`, wiping **all 19** persisted live rows
     (`cold_restore_of_live_sessions_refused {skipped_live_sessions: 19}`).
  2. The deep reconcile then runs and can only re-adopt a row that some
     **reachable daemon actively holds**:
     `recover_missing_preserved_owner_live_sessions_from_reachable_daemons` needs
     a live runtime, `adopt_missing_dormant_sessions_from_reachable_daemons` needs
     the row in the predecessor's `stored_terminal_sessions`.
     Trace: `preserved_owner_deep_reconcile_ran {before: 7, after: 7}` — adopted
     nothing.
  3. **Exactly the predecessor's 6 `terminal_session_keys` survived.** The other
     13 were live-listed but owned by no daemon (the normal state for an agent-CLI
     row whose PTY has exited but which is still attachable), so no adoption path
     could see them and they went invisible.
  **Why the obvious one-line fix is WRONG:** simply retaining unowned rows from
  the file re-opens the 2026-07-09 incident the refusal exists to prevent (19
  *closed* sessions resurrected from a stale `server-state.json`) — see
  `cold_restore_of_live_sessions_is_refused_beside_a_live_owner`. The file is not
  the truth while a predecessor is alive.
  **★ FIXED 2026-07-23 (`287d6e8`, shipped in 2.12.5) exactly as designed below — the
  live predecessor now advertises `live_terminal_sessions` +
  `advertises_live_session_rows` on `ServerRuntimeStatus` (sourced from
  `persisted_state().live_sessions`, so wire and file cannot drift), and a THIRD
  reconcile pass `adopt_missing_live_session_rows_from_reachable_daemons` adopts
  the rows this daemon lacks. Add-only, gated on the new `live_session_row_exists`;
  a pre-B4 predecessor's silence is skipped rather than read as an empty set.
  Locks: `a_pre_b4_daemon_status_does_not_claim_to_advertise_live_rows`,
  `adopting_a_live_row_that_already_exists_is_a_no_op`.**
  **★★ TWO-DAEMON SANDBOX PROOF DONE 2026-07-23 (real daemons, real sockets, real
  PTY, isolated `YGGTERM_HOME`):** predecessor 2.12.5 with 3 unowned agent rows +
  1 owned pin-shell PTY; successor 2.12.6 booted beside it → trace shows
  `cold_restore_of_live_sessions_refused {skipped_live_sessions: 4}` followed by
  `preserved_owner_live_session_rows_adopted {adopted_row_keys: [all 3 agent
  rows]}`; successor ended with 4/4 rows and keep-alive flags byte-preserved
  (pre-fix behaviour on the same scenario: 1/4). Sandbox trap for reruns: the
  pinned shell must have a stdin that never closes — an EOF exits the shell, the
  predecessor then owns nothing, and the refusal (correctly) never fires.
  **The fail-safe arm is LIVE-PROVEN on the 2.12.4→2.12.5 jojo swap:** the
  successor logged the refusal (skipped 20, predecessor owning 7) and adopted
  NOTHING from the non-advertising 2.12.4 predecessor — silence skipped, not
  misread; the expected one-last-time drop (20→7) was recovered by the connect
  loop. **The adoption arm still needs its live proof on the next real swap
  (2.12.5→newer, both sides advertising); expect ZERO row loss there, then
  delete this entry.**
  Original design note follows.
  **Correct fix (designed, NOW BUILT):** mirror what `stored_terminal_sessions`
  already does for dormant rows — have the predecessor **advertise its live rows
  on `ServerRuntimeStatus`** (`live_terminal_sessions: Vec<PersistedLiveSession>` +
  an `advertises_live_session_rows` fail-safe flag, both `#[serde(default)]`, which
  does NOT trip the protocol shape stamp since `ServerRuntimeStatus` is not part of
  `ServerRequest`/`ServerResponse` — precedent: `remote_yggterm_retry_total`), and
  add a third reconcile pass that adopts the live rows this daemon lacks. Source
  the field from `persisted_state().live_sessions` so persistence and the wire
  cannot diverge (no mirrored filter). Then the refusal can stay all-or-nothing:
  the live predecessor, not the file, supplies the rows.
  **Recovery until then** (works, verified this run): capture
  `server app rows` **BEFORE** any deploy, diff after, then
  `yggterm server connect '<path>'` each missing row — ⚠ `connect` is on the
  `yggterm` binary, not `yggterm-headless`, and ⚠ use `ssh -n` in the loop or ssh
  eats the loop's stdin and you silently reconnect only the first row. Rows return
  in 5–10 s. Note `yggterm server reorder` only reorders rows with a **live
  runtime** (`replace_live_session_order` filters on
  `managed_session_is_live_runtime_session`), so it cannot restore the order of
  reconnected dormant rows — cosmetic, but do not expect byte-identical order.

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

## Fixed in 2.10.2 — confirm live, then delete this section

- **Working-dot lag (10–45 s after the agent finished).** Root cause: the
  focused-terminal defer suspended ALL background snapshot applies, and
  snapshots were the only carrier of `working` flags. Fixed with the
  lightweight `WorkingFlags` daemon poll (2.5 s, defer-exempt) + in-place
  patching. Verify: watch `working_edge` events (source
  `working_flags_poll`) in `~/.yggterm/ui-telemetry.jsonl` while a background
  agent finishes; the dot should clear within ~3 s.
- **Collapsed local machine row never blinked.** Root cause: local agent
  sessions carry the loopback `ssh_target: "localhost"` for restore, which the
  local root's `ssh_target.is_none()` check excluded. Verify: collapse the
  local root while a local CC session works — the row must show the blinking
  working dot (`server app rows` → the `local` group row reports
  `busy: true, busy_reason: group_descendant_working`).

- **Clipboard-staging dir grows forever — no autoclean anywhere (user-confirmed
  2026-07-16).** `stage_local_clipboard_png` (shell.rs) and
  `stage_remote_clipboard_png` (yggterm-server lib.rs, incl. the python ssh
  fallback) write every image paste to `~/.yggterm/clipboard/` and nothing ever
  prunes it: pi is at 204 files / 182 MB since April, files up to 15 MB each.
  Left alone this fills the drive.
  **The careful part — these files are NOT free to delete.** The staged PATH is
  what gets pasted into agent CLIs (codex / Claude Code image attach), so agent
  session transcripts reference these paths; resuming a 15-day-old session and
  re-reading its image must not hit file-not-found. Design constraints for the
  fix (deterministic, explicit thresholds, per the no-non-determinism rule):
  1. Two-stage TTL, sweep in each host daemon's chore tick, oldest-first by
     filename (names embed epoch millis, so name order = age order):
     age > 45 days → move to `~/.yggterm/clipboard/.trash/`; trash age >
     45 more days → delete. A "something not found" event then has a 90-day
     recovery window and the trash hop is itself reversible.
  2. Before trashing, reference-check the (unique) filename against that
     host's agent transcript stores (`~/.codex/sessions`, `~/.claude/projects`
     JSONLs); referenced files get their clock reset, never silently dropped.
  3. Size backstop: if the live dir exceeds 1 GB, evict oldest-to-trash down
     to the cap (same reference check applies).
  4. Local and remote hosts each sweep their OWN dir (the daemon is per-host);
     no cross-host deletion.

## Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.
