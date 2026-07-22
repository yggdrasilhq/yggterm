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

A desktop host may deliberately run software GL — see the GL section of the
campaign notes before "fixing" that. Consequences that drive real bugs:

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
