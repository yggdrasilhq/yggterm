# Agent field guide

How to measure, deploy, and verify yggterm without fooling yourself. This is the
durable half of what agent sessions keep re-learning; the volatile half (current
queue, this week's findings) lives in the agent's own notes, not here.

**Scope note.** This file is public. Describe hosts by role — "the live desktop
host", `$LIVE_HOST` (read from `.agents/config/live-host`), "a remote machine" —
never by address, and never paste session ids, transcripts, credentials, or
anything that resolves on the public internet. See `SECURITY.md`.

## 1. The instruments lie — know which, and how

Every entry below cost a session at least once.

| Instrument | Lies when | Use instead |
|---|---|---|
| `app screenshot` (default backend) | A native child webview is on screen — the composite pastes canvas over a DOM snapshot and a GTK widget is in neither layer | `--backend os` |
| `app screenshot` after any GL/compositing change | `toDataURL` returns the canvas backing buffer even when nothing composites to screen; reports `capture_faithful:true` over a black screen | `--backend os`, or the user's eyes |
| `app screenshot` on a client that has switched sessions **(fixed 2026-07-25, `e0dc6c1` — keep the cross-check habit)** | It composited every `isVisible` host ordered by `mountedAt`, and a session you switch BACK to is REVEALED not re-mounted — so its host is the OLDEST while the host it replaced is the newest and stays visible for a while. The stale host was drawn ON TOP: a near-blank frame with `capture_faithful: true` while the terminal was painted fine. Nine rapid switches reproduced it every time | `shadow-client.sh capture` (grim = the compositor's own pixels). The payload now reports `active_session_path` beside the path it drew — if they differ, the frame is not your session |
| `server status` | It pins to its own version's socket and can answer from — or spawn — an empty orphan daemon | `server app …` (PID-routed) |
| A `MutationObserver` / DOM-mutation count | Something animates via CSS. Animations mutate nothing; a page can present frames forever at 0 mutations | Count presented frames (below) |
| `terminal_host_count` / `active_terminal_host_count` | Detached-but-alive xterm entries exist. It counts hosts in the DOM; `window.__yggtermXtermHosts` can hold more | Enumerate the JS host map |
| A `requestAnimationFrame` probe you installed | Always. rAF self-sustains at refresh rate, so it measures itself | An external frame counter |
| `eglinfo` / `glxinfo` over SSH | Always — no seat session means the driver falls back to software | Whether seat-session processes hold `/dev/dri/render*` fds |
| `/proc/<pid>/environ` | The process called `std::env::set_var` at runtime (yggterm does this for GL and arming decisions) | The app's own reported state |
| The daemon's `terminal_lines` | You are chasing a CLIENT paint bug. That is the daemon's vt100 screen — comparing it to itself proves nothing about what the client painted | A faithful pixel, or the client buffer |
| A verb's own `accepted` / `is_trusted` | Always treat as an assumption, not an observation | Read back the page-side *effect* |
| `dom-eval` returning `{"result": null}` | Your script had no `return`. The body is spliced into an async function, so an *expression* yields `undefined` → `null` — identical to a field that does not exist | Include a `sanity: 1+1` term in every probe |
| `yggterm --version` | You need the **protocol** version. It reports the `yggterm` package; the daemon uses `yggterm-server`'s, which a version-only bump may not recompile | The daemon's own socket name, `server-<v>.sock` |
| `server reorder`'s response | The rows have no live runtime. It reports `"requested": N` and echoes your list even when it reordered nothing | Re-read `server app rows` |
| `--help` for any `server app` verb | Often — the help text goes stale while the parser gains verbs | The match arm in `apps/yggterm/src/main.rs` |
| A `#[serde(default)]` telemetry field reading `0` | The peer predates the field entirely — absent and zero are the same wire value | Ask whether the KEY exists, not its value |
| A third-party call you *believe* has an effect (`term.open(host)`, a `.focus()`, a `.dispose()`) | The library early-returns on a state you are already in. It does not throw, so the whole repair is a silent no-op and the code around it looks right forever | Assert the EFFECT (`host.childElementCount`, `document.activeElement`), or pin the behaviour in `tools/xterm-harness` |
| A directory listing of `~/.yggterm/server-*.sock` | Always, if you read it as litter. jojo holds **633** of them going back to 2.1.x and **every one accepts a connection**: all but the current version are SYMLINKS the daemon retargets to its live socket at startup (`refresh_legacy_server_socket_aliases`) so an older client can still find it. Sweeping them deletes the cross-version compatibility plane | `ls -l` (they are symlinks, not sockets), or connect and read `server_version` |

**The rule underneath all of them:** if the symptom is visual, the proof is a
faithful pixel. Telemetry that says "healthy" while the user sees a broken screen
means the telemetry is wrong, not the user.

**The generalisation worth carrying to any codebase:** the dangerous instrument
is not one that fails loudly — it is one whose *failure value is indistinguishable
from a legitimate negative result*. `null`, `0`, `[]`, and "unchanged" are all
answers a broken probe gives just as readily as a healthy system does. Whenever a
probe can return one of those, make it carry a term that proves the probe itself
ran (a `sanity` value, a key-existence check, a known-nonzero control). A probe
that cannot fail loudly must be made to succeed loudly.

**★ The same rule applies to REPAIRS, not just probes (learned 2026-07-22, at the
cost of several sessions).** A repair built on a primitive that silently does
nothing is indistinguishable from a repair that ran and failed to help — so the
investigation goes looking for a *second* bug that does not exist. `term.open()`
was the case: it early-returns once `term.element` exists, so `host.innerHTML=""`
+ `term.open(host)` looked like a rebuild and was pure loss. Three sessions of
husk investigation widened the *guards* around a repair that could never have
worked. **Before deciding a fix didn't fix it, verify the primitive underneath it
does what you assume** — and when the primitive belongs to a vendored library,
prove it in `tools/xterm-harness/` (jsdom + the EXACT shipped bundle, minutes to
write) instead of arguing from a live symptom. The harness turns
"upstream probably does X" into a test that fails when a version bump changes X.

## 2. Profiling recipes that work

No `perf` on a typical desktop host (`perf_event_paranoid=3`), but these do:

- **Per-thread CPU** — read `utime+stime` from `/proc/<pid>/task/*/stat` twice N
  seconds apart. Thread names tell you the subsystem immediately. Include the
  daemon and the WebKit child, not just the GUI.
- **Poor-man's profiler** — `eu-stack -p <pid>` in a loop (~12 samples). One
  busy sample among idle `ppoll`s is still a real attribution.
- **Syscall shape** — `strace -c -p <pid>` for 5s. A hot loop shows up instantly
  as a `clock_gettime` count; repeated `openat`/`mkdir`/`statx` means something
  is re-opening a store on a hot path.
- **Presented frames** — count `memfd_create` on the GUI process. Each new
  buffer is a presented frame. This is the honest "is the app repainting?"
  number, and it is invisible to every DOM-side probe.
- **In-page timing** — wrap the function under suspicion from `app dom-eval`,
  accumulate into a `window.__probe` object, read it back in a later call.
  Instrument *all* candidates, not the one you suspect; the answer is often that
  your suspect costs nothing.

**Hold the workload fixed.** The single most common measurement error here is
comparing two conditions under different load — a CPU/thermal A/B is evidence
only if the same session is doing the same thing in both windows. When the agent
itself drives a live session, run the whole A/B inside ONE script so the agent
emits nothing during the sampling windows.

## 3. Rendering cost model (software-GL hosts)

⛔ **"A desktop host may deliberately run software GL" is no longer standing
guidance, and on the live host it was never true.** The GUI used to hard-code
`LIBGL_ALWAYS_SOFTWARE=1` behind an opt-out nobody set, on a premise (one EACCES
on `card0` under a GBM probe) that was measured false on the very host it named.
The binary now PROBES (`yggterm_core::gl_probe`) and publishes its answer as
`YGGTERM_WEBKIT_GL_POLICY`; read that before assuming anything about a host.

The cost model below still holds **wherever the probe genuinely lands on
software** (a headless server host, a VM with no render node, a host with
`YGGTERM_FORCE_SOFTWARE_GL=1`), and the frame-count findings stand on their own
merits on any host. Consequences that drive real bugs:

- Every repaint costs a full-window CPU blit (`cairo_paint` / `pixman_blt`) on
  the GUI main thread. **Cost tracks the number of presented frames, not the
  number of pixels that changed.**
- Therefore: N independently-phased animations cost N times one animation.
  Paint containment (`contain:paint`, `will-change`) and removing
  `backdrop-filter` do **not** help — measured, twice. Cut frames instead.
- The app owns exactly ONE blink animation, on `:root`, published as an
  inherited custom property (`--yggterm-status-dot-blink`). Any new indicator
  reads that phase; none declares its own animation. See DESIGN.md, "One clock
  for every blink."
- A CSS animation's phase is anchored to when its element was created. You
  cannot phase-lock per-element animations with a computed `animation-delay`:
  changing the delay does not restart the animation, so re-rendered rows drift.
- **Is the GPU actually rasterizing?** `drm-engine-gfx` in
  `/proc/<webproc>/fdinfo/*`, nonzero and RISING across two reads. The repo owns
  that gauge now — `render_top` prints a `gpu_ms` column per role, and a `-`
  there means the counter was unreadable, which is not the same as a zero.

## 4. Deploy protocol

### 4.0 First decide which KIND of deploy this is — it changes everything

| Change lives in | Version | What restarts | Cost to the user |
|---|---|---|---|
| CLI path only (arg parsing, screenshot post-processing, manifest building) | any | nothing | **zero** — run the new binary as a client from `/tmp` |
| GUI only (`shell.rs`) | **KEEP THE CURRENT VERSION** | GUI only | small — one blank re-attach |
| Daemon (`daemon.rs`, `lib.rs`, protocol) | bump | GUI **and** daemon together | real — re-attach symptom class on every live session |

⛔ **A GUI-only patch must NOT bump the version.** A newer GUI classifies the
older daemon as stale and spawns a successor — which is exactly the daemon
handoff (and its frame corruption) you were avoiding. Same version = the daemon
is untouched, its PID does not change, and PTYs never move.

⛔ **Never leave the GUI and daemon on different versions.** An *older GUI*
fights a *newer daemon*: it classifies it as stale and tries to displace it
forever (measured: ~24,000 events at ~2,500/min for 27 minutes, user saw frozen
frames and "daemon connection lost"). The `version_mismatch` warning only fires
when the GUI is newer, which is why the dangerous direction looks safe.

### 4.1 The version-stamp landmine — VERIFY, never trust `--version`

`SERVER_PROTOCOL_VERSION` is `env!("CARGO_PKG_VERSION")` of the **yggterm-server**
crate. A version-only bump in `Cargo.toml` does **not** always force that crate to
recompile, so a release build can ship a binary whose `--version` reads 2.12.2
(the `yggterm` package) while its baked protocol constant is still 2.11.0. The
deployed GUI then reads its own protocol as older than the live daemon, silently
refuses to swap, and you get a mixed-version wedge: a retry spin, a session that
cannot reconnect, broken typing, and a ~50 Hz garbled blink. **This cost hours.**

```bash
cargo clean -p yggterm-server          # before ANY release build after a bump
cargo build --release --bin yggterm --bin yggterm-headless
```

**Then prove the stamp** — `--version` cannot prove it, because it reads a
different crate's version. The socket name is derived from the protocol constant
(`format!("server-{}.sock", SERVER_PROTOCOL_VERSION.replace('.', "-"))`), so a
throwaway daemon in an isolated home spells it out:

```bash
SB=$(mktemp -d)
YGGTERM_HOME="$SB" ./target/release/yggterm-headless server daemon > "$SB/d.log" 2>&1 &
sleep 3; grep -o 'server-[0-9-]*\.sock' "$SB/d.log"   # -> server-2-12-3.sock
kill %1
```

⚠ The socket does **not** live under `YGGTERM_HOME` when that path is long — it
falls back to `/run/user/<uid>/yggterm/h-<hash>/`. Read the path out of the
daemon's own log line, don't `find` the home dir. Clean up that runtime dir too.

### 4.2 Deploy to all FOUR paths

The live host runs the daemon from `~/.local/bin/`, but remote wrappers invoke
`~/.yggterm/bin/`. Miss one and you get a split-version fleet:

```
~/.local/bin/yggterm    ~/.local/bin/yggterm-headless
~/.yggterm/bin/yggterm  ~/.yggterm/bin/yggterm-headless
```

`cp -a` each to `*.rollback` first, then **`mv` the new binary in — never `cp`**
(cp over a running binary is `ETXTBSY`).

⚠ **Cap the rollbacks — they are ~48 MB each and pile up fast.** A single
`*.rollback` per dir would be overwritten each swap, but a dated/named rollback
(`.rollback-<fix>`) accumulates: a day of swaps left 26 of them (~1.2 GB) on a
host already thrashing swap. After staging the new binary, prune to the two
newest per dir: `ls -t "$dir"/yggterm.rollback* | tail -n +3 | xargs -r rm -f`.
The junk that makes the fan angry is often our own deploy residue.

### 4.3 The recipe that works (cross-version, no fight)

```bash
# 0. CAPTURE THE GROUND TRUTH FIRST — this list is what makes recovery exact
ssh $H '~/.local/bin/yggterm-headless server app rows' > rows-before.json
# 1. scp -> rollbacks -> mv into all four paths (above)
# 2. GUI and daemon TOGETHER, in one window:
ssh $H 'kill -TERM <gui-pid>'                       # wait for actual exit
ssh $H 'yggterm-headless server app launch --wait-visible'
```

The new GUI spawns the new-version daemon, which adopts from the old daemon that
is *still alive holding PTY fds* — doing both together skips the version-fight
window entirely. The old daemon staying alive is correct and deliberate:
`hot_restart_should_defer_for_session_survival` returns true while it owns PTY
fds, so sessions are parented by the **daemon**, never the GUI.

⚠ The hot-restart is **blocked while any owned session is "working"** — and the
agent's own session counts, so `hot_restart_block_reason` will name *you*. The
tool that forces it safely is `yggterm-headless server update-daemons --force`
(progressive, preserves PTYs, ungated handoff) — run it from a **correctly
stamped** binary or it will refuse for the wrong reason.

### 4.4 After deploying — the checks, in order

1. **Row count before vs after.** Expect a drop; see §4.5. Nothing is lost, but
   *invisible is lost from where the user sits*, and it was the user who noticed
   last time.
2. `server status` → `server_version`, `server_pid`, `role_enforcement`.
3. **A faithful pixel.** `server app screenshot` and then **Read the PNG**. Check
   `capture_faithful: true`; a `linux_webkit_snapshot` fallback frame is
   canvas-blind and lies about the terminal.
4. **Exercise the fix and quote the evidence.** If you cannot, say so plainly —
   "code is on disk, the running daemon predates the fix" — never "shipped".

**Deploying re-introduces transient symptoms.** A daemon swap re-resumes agent
CLIs on fresh PTYs, and that window looks exactly like the squish/broken-bottom
bug class. Never measure a symptom the deploy itself causes, and never declare a
post-deploy surface healthy without looking at it.

### 4.5 Expect to lose Live Sessions rows — and know the recovery

Every daemon swap drops the rows that no daemon actively owns (root cause and the
designed fix: `docs/pending-bugs.md`, "B4 ROOT CAUSE"). Measured on 2.12.2 →
2.12.3: **25 rows → 12**. Exactly the predecessor's owned keys survive.

```bash
# after the deploy, diff against the ground truth captured in §4.3
comm -23 paths-before.txt paths-after.txt > missing.txt
while read -r p; do
  ssh -n $H "~/.local/bin/yggterm server connect '$p'"
done < missing.txt
```

Three traps in those four lines, all of which have bitten:

- ⚠ **`connect` is on the `yggterm` binary, NOT `yggterm-headless`** — headless
  answers "unsupported server command".
- ⚠ **`ssh -n` or the loop silently reconnects only the FIRST row.** Plain `ssh`
  reads stdin, so it swallows the rest of `missing.txt`. The loop *looks* like it
  worked because the one row it did process succeeded.
- ⚠ **`yggterm server reorder` cannot restore dormant rows' order.**
  `replace_live_session_order` filters on `managed_session_is_live_runtime_session`,
  so rows without a live runtime are ignored — the call still reports
  `"requested": 19` and echoes your list back, which reads exactly like success.
  Verify order by re-reading `server app rows`, never by the reorder response.

Rows reappear 5–10 s later. Re-check the count **again once the predecessor has
actually exited** — the drop can be delayed: the predecessor holds dormant rows
until its own disk-binary poll retires it, which can be ~20 minutes later.

## 5. Destructive operations — know before you type

- Any `reconcile` / daemon-screen replay is a full reset + re-seed to the current
  screen. On a healthy session it collapses scrollback and can blank the
  viewport. Run it only on a surface already confirmed broken.
- Never type into a live agent prompt to "test" it.
- Restore the user's active session after any probe that had to switch away.

## 7. Gotchas that cost this project real time (2026-07-26)

Each of these produced a CONFIDENT WRONG ANSWER, which is worse than an error.
They are ordered by how much time they burned.

### 7.1 A lock that mutates the new helper is not a lock

**Five could-only-pass locks shipped in two rounds.** Every one tested a
freshly-added helper with hand-built arguments instead of the wiring that was
the actual defect:

- the "20 sequential verbs deliver" test passed a **literal zero** seat-input
  count, synthesizing away the exact bug it existed for;
- a GL lock asserted `f(x) == Clear iff x`, restating a function that *is*
  `if x { Clear } else { Apply }`;
- a memo-key lock built struct literals and never called the production key
  builder — a tautology over `derive(PartialEq)`;
- a source scan anchored on a string occurring **zero times** in the file, so
  `unwrap_or(len)` silently widened its "scope" to 75% of the file including
  the test module;
- the web-surface reclaim locks: reverting **all four production call sites**
  left the suite green at the identical pass count the report cited as proof.

**The rule: mutate the PRODUCTION CALL SITE, run it, see red, restore, and quote
the red output.** If the loop is not directly testable, extract the decision
into a pure function the loop CALLS, so reverting the loop's wiring changes the
pure function's observed input. A test that calls the helper directly is
structurally incapable of observing what the loop passes it.

**All five are now closed** (`41b7b1b` GL, `4a2c836` memo key, ROUND-15 scan
coverage floor, the batch locks in `web_do_verb_tests`, and
`shell::web_surface_reclaim_locks` for the reclaim family). The reclaim one is
the worked example of the rule, because it is the case where the wiring lived in
an `async` loop holding a live `DesktopContext` — nothing a test can call:

1. **The loop keeps no policy and no bookkeeping.** It reads `/proc`, reads the
   configured hold, names its backgrounded surfaces, and calls
   `web_surface_reclaim_background_pass`. Every decision — which pressure
   reading, which surface's audio, whether the reap is recorded, whether a soft
   stash demotes or detaches — moved into that pass.
2. **The world is a trait.** `WebSurfaceBackgroundHost` carries destroy / stash /
   demote / throttle / clear-loading / trace, so a test asserts on what the pass
   DID, not on what a helper returned. `LiveWebSurfaceBackgroundHost` is the only
   un-mockable code left and is four one-line methods.
3. **The residual seam is locked structurally.** The loop's own argument list
   still cannot be reached, so
   `the_reclaim_pass_call_site_is_wired_to_the_live_machine` scans it — over
   `yggterm_core::agent_cli::product_lines`, the workspace's ONE test-module skip
   rule, so the lock's own text cannot satisfy the needles it looks for.

**Twenty-one mutations were run against it, one per production call site, each
proven red and restored.** If you touch this family, re-run them: the script
shape is in the round-25 report, and a lock nobody re-proves decays into the
thing this section is about.

### 7.2 Never run several workflow lanes on `main` in one checkout

Three fix lanes were pointed at `main` in the same working tree. Two edited
`shell.rs` simultaneously; 1,222 uncommitted insertions interleaved, the tree
stopped compiling mid-edit, and stopping them left half-finished refactors
(renamed functions with un-updated test call sites, a test written for a
function that was never written). **Use `isolation: 'worktree'` for every
parallel lane, always** — even when the file sets look disjoint, because agents
run the whole suite and will "fix" each other's in-flight edits.

### 7.3 A measurement must be able to refuse

Three separate A/B arms each produced a confident wrong number, all from the
same failure — the load never reached the renderer and nothing said so:
comparing an evening of real use against an overnight window with **23x less
terminal activity**; piping the paint load to `/dev/null`; and restarting the
GUI between arms so it came back displaying a **different session**. That last
arm read 0.27 cores and looked like a 5x win.

`scripts/gl_ab_measure.sh` now refuses to print a number unless the window is
focused, the session under test is the one actually displayed, the active
session did not change mid-arm, and the arm cost more than a floor.

**Corollary — the confound that invalidated both a "win" and a "regression":**
render rows carry `window_focused`, and unfocused→focused alone moves
`render/gui` p50 by **7x**. Before comparing any two render windows, bucket them
by `window_focused` and by `daemon_request/terminal_read` rate. `gpu_ms` zero in
523 of 532 ticks means nothing painted, not that the GPU got faster.

### 7.4 Instruments that lie about their own subject

- **`set_var`/`remove_var` do NOT change `/proc/<pid>/environ`.** glibc
  reallocates the environ array on the heap; the kernel keeps exposing the
  exec-time stack page. Anything the process set AFTER exec is invisible there,
  and after a hot restart the child's `/proc` shows the PREDECESSOR's values.
  Publish a runtime decision from the process's OWN view.
- **`drm-engine-*` is per DRM CLIENT, not per fd.** Duplicated fds share one
  `struct file` and each repeats the same cumulative counter, so a per-fd sum
  over-counts by the fd count — measured **5.00x on Xorg, 4.00x on a
  compositor**. Dedup on `drm-client-id`.
- **Zero GPU engine time in a window means IDLE, not software.** Read the
  DRM-fd count first: llvmpipe never opens a DRM node, so that is the
  structural answer; engine time is a workload answer.
- **`grep -c` on a binary counts LINES.** Use `strings | grep -c`, and pick a
  string the fix definitely contains — a format string, not a code identifier
  that may be inlined away.

### 7.5 The reaper can be the cause

Reclaim that destroys a page which is immediately re-created reclaims nothing
and pays for a fresh web process every cycle. Measured live: **166
`background_hold_expired` closes against 166 re-opens in fifteen minutes**, one
churned process ballooning to 3.9 GB, on a host whose swap was 100% full —
the reaper reacting to pressure it was substantially creating, while the user
could not hold keyboard focus long enough to type a prompt. Any pressure-driven
reclaim needs hysteresis: a target that keeps coming back is not a target.

### 7.6 A working GPU is not consent to arm a compositing mode

Hardware GL armed Phase F under-glass by default, and the user's entire window
became a background agent's page. Under glass the shell webview is TRANSPARENT
by construction, so anything that stops the shell painting shows whatever
surface is behind it, full screen — and it fired exactly when memory was
exhausted, which is when the shell is least likely to paint. Under the legacy
opaque stack the same starvation is invisible. Keep the GL decision and the
compositing decision separate; `shm_force_for_arming` already refuses SHM on a
hardware host, so hardware GL keeps DMABuf either way.

### 7.7 `kill -TERM` on the daemon is not the graceful drain

The graceful path is the binary-replacement self-retire, which drains in idle
order. But it defers while ANY owned session was active in the last 300 s, so
under agent load it never converges. Killing instead cost **~7 agent PTYs**;
rows and transcripts survived and each resumes on a click, but in-flight work
was interrupted. Decide deliberately, and tell the user the cost BEFORE doing
it.

### 7.8 The agent outlives the binary

`ychrome-vault` and `yedit` both serve OLD code from a running process after
the binary on disk is replaced — the vault agent had been serving pre-fix code
for 42 hours with its exe deleted, while the fixed binary sat installed. A fix
that is "deployed" is not running until the process that serves it restarts.
Check `/proc/<pid>/exe` for ` (deleted)` and compare process start time against
the fix's commit time.

### 7.9 Source of truth for a tool's own source

The deployed `yedit` binary's features existed **only** as untracked files on
the build host — no git repo, no remote, no copy anywhere. Before editing any
fleet tool, confirm its source is in version control and pushed.

### 7.10 The row ledger is the authoritative record of what existed — consult it FIRST

`~/.yggterm/row-order-ledger.json` records which live-session rows existed and
in what order, and `~/.yggterm/removed-rows.json` records closes. It exists
precisely because ghost rows are a recurring problem here.

**It was not consulted, and the user lost seven rows.** Asked to remove ghost
sessions, I went to `server-state.json` plus agent-transcript mtimes and built a
"untouched for ≥3 days" heuristic — while the ledger sat there holding every
row's identity and position. Two of the seven removed were real work
("Commit and continue v3 parity", "Fileables campaign"), and the user had to ask
for them back.

**The trap that made a bad method look verified.** The user said the list should
be "20-21". The ≥3.0-day cut left exactly 21, and that coincidence read as
confirmation. It was not evidence — it was a threshold fitted to a target the
user had supplied, then quoted back as if it had been derived. **A heuristic that
agrees with the number you were given is not corroborated; it is circular.**

Same failure at the other end: after a daemon restart the sidebar showed 28 rows
and that count was reported as a sign the restart went well, without ever
comparing it against the ledger's recorded set — which is the only thing that
can say whether those 28 are the RIGHT 28. The user had to point out twice that
the number itself was the bug.

**Rules:**
- Before adding, removing, or judging any row, read the ledger. It is the record;
  `server-state.json` is the current belief, and the whole ghost class is the two
  disagreeing.
- Restoring a removed row means clearing its tombstone in `removed-rows.json`
  first, then re-opening the session — otherwise the veto silently wins.
- Never delete a user's session on a heuristic. Show the list, name the evidence
  per row, and let the user choose. Removal is recoverable (the CLI transcript
  survives), but their attention is not.

## 6. Where the deep material lives

- `docs/pending-bugs.md` — open, user-confirmed bugs. The work queue.
- `docs/xterm-bugs.md` — the terminal bug registry, by class.
- `docs/agent-control-plane.md` — the engine verb layer and shadow model, with
  the slice execution order.
- `docs/web-under-glass.md` — Phase F: under-glass web compositing, phases and
  acceptance gates.
- `docs/protocol.md`, `docs/sessions.md`, `docs/daemon-handoff.md` — session
  identity, persistence, handoff.
- `docs/split-view.md`, `docs/alt-keytips.md`, `docs/web-surfaces.md` — feature
  specs.
- `DESIGN.md` — colors, typography, spacing, interaction vocabulary. Consult it
  before styling anything; add durable decisions there rather than in comments.
- `.agents/skills/yggui-app-control/SKILL.md` — the agent's hands and eyes on
  the live desktop.
