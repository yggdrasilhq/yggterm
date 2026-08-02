# Pending bugs

**This file is the ONE answer to "what is open".** It lists open items only; an
entry is deleted in the same commit as its verified fix, and git remembers it.
The rules, the owner table for every other question, and how to search the
archive are in [`docs-ssot.md`](docs-ssot.md). `scripts/check-docs-ssot.sh`
enforces them.

Statuses: **OPEN** · **FIXED IN CODE — LIVE PROOF OWED** (name the observation
that would falsify it) · **AWAITING A DECISION** (name who decides).

Closed narratives from before 2026-08-02 are in
[`archive/pending-bugs-closed-2026-08-02.md`](archive/pending-bugs-closed-2026-08-02.md).


## The launcher OFFERS the GUI host's apps for a row on another machine

**Status:** FIXED IN CODE — LIVE PROOF OWED

**The observation owed:** on a daemon+GUI carrying this fix, right-click a
`remote-cc://dev/…` row and see dev's own apps in the menu — specifically
`yggdrasil-maker` (installed on dev, absent on the GUI host) PRESENT, and the
GUI host's own `yrdp` ABSENT. The GUI reads `RemoteMachineSnapshot::apps`, which
only a daemon can fill, so this needs a daemon handover as well as a GUI swap;
the GUI host went off the network before either could be done, and the binary is
deployed and waiting.

**The remote half IS proven** (2026-08-02, new binary installed at
`~/.yggterm/bin/yggterm` on both dev and oc):

```
dev: {"name":"ychrome",…} {"name":"yedit",…} {"name":"yggdrasil-maker",…}
oc : {"name":"ychrome",…} {"name":"yedit",…} {"name":"yrdp",…}
```

Two machines, two different registries, each its own — and `yggdrasil-maker` is
exactly the app that a right-click on a dev row could never show before. The
back-compat path is proven the same way: oc on its OLD binary answered
`Error: unsupported server command: remote`, a clean refusal, which is what
makes `fetch_remote_machine_apps` return `None` and keep that machine's previous
list instead of blanking its menu.

User-reported 2026-08-02: *"why does ychrome only launch on jojo even if I right
click on dev or oc sessions. This is undesired behavior."*

⛔ **The first filed root cause was HALF WRONG, and the wrong half is the one
the title carried.** It claimed the right-click "offers, and launches, the GUI
host's ychrome". The OFFER half is true. **The LAUNCH half is false**, and
believing it would have sent the next session to rewrite a launch path that is
already correct.

**What the repro actually proved** (2026-08-02, jojo 2.12.24 + dev):
`terminal_launch_context_for_row` resolves a `remote-cc://dev/…` row through
`remote_machine_for_sidebar_row`, which falls through to `row.host_label`
(`"dev"`), finds the machine, and returns
`Remote { ssh_target: "dev" }` — locked by
`a_remote_row_offers_its_own_machines_apps_and_launches_there`. Driven live: a
session created on dev, `echo MACHINE=$(hostname)` → `MACHINE=dev`, then the
manifest's own command typed in → `ychrome` running **on dev** (pid on dev, not
on the GUI host), which then declared its surface up the ssh chain and
`web ensure` on the GUI host answered
`rebuilt_from_daemon_declare: true, tabs: 1`. The remote path works end to end.

⚠ **The first repro was a false negative and is worth remembering.** It launched
bare `ychrome`, which stops at the profile picker, and the picker declare is
`("web-surface", "pick") => Retention::Ignore` — the daemon deliberately never
retains a prompt awaiting a human. So `web ensure` answered `no_declare` for a
reason that had nothing to do with the machine, and on a `--no-activate` session
there was no mounted xterm host to parse it live either. **A discriminator that
answers the same way on the working host is not a discriminator.** Re-run with
`--profile <name>` so the real `open` declare fires.

**What was actually broken.** `cached_app_registry()`
(`crates/yggterm-server/src/lib.rs`) scans the DAEMON'S OWN home, and every
launcher surface read that one list for every row. So the menu beside a dev row
was drawn from the GUI host's registry: an app installed only on dev never
appeared, an app installed only on the GUI host was offered for execution on
dev, and the manifest's ABSOLUTE `binary` path — which by contract means
something only on the host that wrote it — was the path typed into dev's PTY.
On this fleet the paths happen to coincide, which is exactly why it looked like
it worked and read as "it always launches jojo's ychrome".

That is the single-source-of-truth mismatch `CLAUDE.md` forbids: "which apps
exist" was keyed to a HOST while "where does this session run" is keyed to a
MACHINE, with nothing making them agree.

**The fix.** A machine reports its own registry (`server remote apps`, one
manifest per line, pruned by the same scanner the local host uses), the daemon
fetches it on the existing refresh and stores it on `RemoteMachineSnapshot::apps`,
and the GUI resolves "which apps does this row have" through
`app_registry_for_row` — which calls **the same `remote_machine_for_sidebar_row`
that decides where the launch runs**. Offer and execution are now two readings of
one fact. `resolve_app_verb_for_row` closes the other half: a clicked entry
resolves against the registry it was drawn from, so a remote app's menu item can
never be a silent no-op. A failed fetch keeps the machine's previous list
(`None` ≠ `Some(vec![])`), so one flaky ssh round trip cannot blank a host's menus.

⚠ Adjacent, do not conflate: the ychrome ENGINE not existing off the GUI host is
a separate, already-closed ychrome entry.

⚠ **Still open and NOT this entry:** the browser surface itself always renders in
the GUI host's process, so a dev-launched ychrome uses the GUI host's web
profiles and cookie jars while its vault/settings panes come from dev's ychrome
daemon. That split is architectural, was not reported, and needs the user's call
before anyone "fixes" it.

## A ychrome session whose last tab is closed should close itself

**Status:** OPEN

*(the yggterm half is built and locked; the ychrome half is not, and the item
does not close until both are live)*

User-reported 2026-08-02: *"In a specific ychrome session, if all tabs are
closed then ychrome session itself should close itself."*

Today the ychrome CLI keeps its session alive after its last tab goes, leaving a
row that owns nothing — the inverse of the settled rule that a row with no
runtime is fine and desirable (`docs/settled-calls.md` call #4). That rule is
about a row the USER can click to restart; this is a live session with a live
process and nothing to show, which is different and is clutter.

**The yggterm half, built 2026-08-02.** `WebSurfaceUiState::last_content_tab_closed`
is latched in `web_surface_close_tab` — the ONE removal path, so every close verb
reports it by construction and no bulk close has to remember — and cleared by the
next tab that opens, including a popup and an undo. It rides the `/ping` the GUI
already sends per session as `&last_tab_closed=1`, on every ping while set rather
than once, so a dropped tick costs nothing and the app owes no acknowledgement.
An app that does not know the param ignores it like any unknown query value.

⛔ **It is a latched EVENT and must never become the count `tabs.len() == 1`.**
A surface holds nothing but its app tab in the window between the app declaring
and its first page arriving, so a signal derived from the count would order every
ychrome to quit at launch. Locked by
`closing_the_last_content_tab_signals_the_app_but_having_none_yet_does_not`,
whose second half is exactly that case, and mutation-proven by removing the latch.

**What ychrome still owes.** Its `/ping` handler (`src/daemon.rs`, the
`request.path == "/ping"` arm) reads `session` and `ack` from the query and must
also read `last_tab_closed`, recording it on that `SessionEntry`. The view
client's `drive_surface` loop already talks to the daemon every ~4 s through
`declare_current`, so that reply is where the answer belongs: on seeing it, the
loop sets its `stop` flag and falls into the shutdown it already has
(`emit_close` → `deregister` → the `close` OSC), which is precisely what Ctrl+C
does. Nothing new needs writing on the teardown side.

⚠ Do not implement this by having the app poll for tabs — that is a second
encoding of a count the GUI already owns.

⚠ Live proof owed for the whole item, and it needs both halves plus a GUI swap.


## Two supernumerary daemons persist holding unmigratable local:// shells

**Status:** OPEN

**Two supernumerary daemons persist** holding unmigratable `local://` shells.
That is the durable half of the chaining bug, still open.
- ✅ **The vault agents on dev and jojo are current and unlocked (2026-07-31).**
Neither needed an unlock in the end. jojo was already satisfied; dev's binary
predated the `socket` field that the card-fill path's socket lookup reads, so
it was rebuilt, installed, and moved across with `ychrome-vault handover` —
the unlocked session hands to the new binary instead of re-locking, so the
refresh cost ZERO unlocks. Both now report `agent_stale:false`,
`state:unlocked`, `undecryptable:0`, `socket:…/vault/agent.sock`, and both
agree at 1116 items (dev resynced from 1115 on the handover).
⚠ The handover verb is the way to refresh a vault binary. Do NOT
`stop-agent` for a version bump — that re-locks and costs the user an unlock.
- ✅ **The five could-only-pass locks are all closed.** The last one — the
web-surface reclaim family, where reverting all four production call sites
left the suite green — is replaced by `shell::web_surface_reclaim_locks`,
eight tests that drive `web_surface_reclaim_background_pass` (the function the
reconcile loop calls) through a fake host, plus a structural lock on the loop's
own argument list. Twenty-one mutations, one per production call site, each
proven RED and restored. Field guide §7.1 has the shape.
⚠ **Not live-verified on jojo** — this was a test-discipline lane, product
behaviour is unchanged and the deploy happens separately.
**The habit stands regardless: before trusting ANY test in a report, mutate
the production call site yourself.**


## Agent engine: ctl fill is documented but has no route

**Status:** FIXED IN CODE — LIVE PROOF OWED

**The observation owed:** `ychrome ctl fill page_id=<p> entry=<item>` answering
`{"filled":"filled"}` against a real login form, with the page-side field
length read back afterwards. It cannot be observed yet: the dev daemon is
still serving pre-fix code and **refuses to retire under 4 live surfaces, one
of them the operator's linked WhatsApp Web session** (a restart costs a phone
QR re-scan). ychrome's contract makes that handover the operator's call, so
the binary is installed and waiting at `~/.local/bin/ychrome`; the verb goes
live on the next `ychrome daemon restart`.

**What shipped** (ychrome `95b116a`, plus the doc correction): `/engine/fill`
takes `{page_id, entry, user?}` and answers
`{ok, entry, filled: "filled"|"user-only"|"no-fields"}`, with a locked vault
answering `502 vault: …` so it cannot be mistaken for a page error. The
secret goes agent → eval script → dropped, exactly as the sidebar's own fill
action does; the reply names fields, never values. Two tests hold it: `fill`
is in `DRIVES_A_PAGE` (else the governor could park the page between `open`
and `fill` and the fill would land on nothing while answering 200), and the
route is asserted never to mention a password field.

⚠ **The fix landed inside a commit whose message is about something else.**
Another agent's `git add` swept these files up mid-edit, so `95b116a` reads
as a stale-port docs commit and contains the engine route. It was already
pushed, and rewriting shared history in THIS repo is the trap that cost the
relicence pass, so it stays. **Do not go looking for a `feat(engine)` commit
— there isn't one.** The general lesson: on a fleet where agents share a
checkout, `git add -A` is not a safe habit; stage explicit paths.

*(original report, 2026-08-01, found while an agent filed a GitHub support
ticket headlessly: `agent-engine.md` §4 documented `ychrome ctl fill` but the
daemon exposed no `/engine/fill`, so the agent had to post raw input events
to the daemon socket with the secret in a 0600 file to keep it off the
command line.)*
>
> Earlier rounds closed: the tab rail becoming the cwdtree with folder icons,
> nesting, the drag gesture and the density pass; Cloudflare challenges;
> userscripts not injecting and the YouTube 2x-ads symptom; adblock/SponsorBlock
> exhaustiveness; open-webui sidebar switching; fullscreen chrome over the
> picture; tab placement and the row menu; the mis-clicked hidden duplicate;
> `ychrome-vault totp` on a skewed clock; the two HTTP caches; the frosted
> close-button chip; and background tabs destroyed on a clock.

**TWO functional items remain, plus one design call and a live render batch.**


## A degraded profile cannot be made genuinely READ-ONLY

**Status:** OPEN

**A degraded profile cannot be made genuinely READ-ONLY — DESIGN CALL, decided
2026-08-01.** The silence half is done (`WebSurfaceJarMode` owns the decision,
the spelling and the notice). WebKitGTK has no read-only jar, so "genuinely
read-only" means giving the loser a COPY of the profile's cookies — and the
objection was that every agent shadow surface would then duplicate the user's
live session cookies to a second place on disk, in a browser carrying
brokerage sessions.
**Decision: option 2, narrowed — copy the `cookies` file ONLY, into a scratch
dir wiped at teardown, and ONLY for a surface the USER opened; an agent shadow
surface keeps today's jarless behaviour and its notice.** That fixes the
reported symptom (a second surface on a held profile stays logged in for its
life) while removing the objection outright, because the shadow path was the
whole exposure. A startup sweep clears crash leftovers.

### ⭐ THE RENDER-PIPELINE BATCH (user, 2026-08-01) — untouched for a long time

The user's words: *"gating sessions have become ridiculously buggy. We have not
touched the rendering pipeline for a long time and bugs have piled up."* Three
symptoms, and the last two are strongly suspected to share a root:

1. **The stuck viewport after a copy.** Selecting and copying in a session
 leaves the viewport pinned: it shows **"2 new messages (ctrl+End) ↓"** while
 output keeps arriving and never follows it. Switching sessions unsticks it —
 which points at a remount clearing state that nothing else clears. Suspect
 the follow-prompt / user-scrollback guard treating a selection (or the scroll
 a selection causes) as sticky and never releasing after the copy completes.
 Screenshot also shows `sent 50 chars via OSC 52`.
2. ✅ **Claude Code ALWAYS starts with a broken bottom**, plus glyph corruption
 while switching into CC sessions. A TUI refresh fixes it every time — so the
 daemon's screen is right and the CLIENT is painting less than it holds.

 **ROOT-CAUSED AND FIXED IN CODE 2026-08-01 (`lane/dev/render-pipeline`). It
 is the GATE — the user's two complaints were one bug.** The handover veil
 does not merely cover the viewport; while `handoverPaintSuspended` is true
 the host does *no visible paint at all*. On release it used to run
 `requestVisiblePaint(false)` — a damage-tracked partial paint. Every row the
 read loop wrote during the veil is already in the buffer with its damage
 consumed, so the resume presents **less than the client holds**. That is the
 broken bottom, and it is DETERMINISTIC ("always") because the gate arms on
 `preserved_terminal_owner_count > 0`, a steady state, so every mount goes
 through it.

 The glyph half is the same line. `clearTerminalTextureAtlas()` lives *inside*
 the forced-refresh branch of the visible-paint funnel, and its own comment
 already names the symptom: while a window is backgrounded the WebGL glyph
 atlas goes stale, so a switch-in that does not force a refresh "paints cells
 against a stale atlas -> wrong-glyph garble". A non-forced resume skips the
 heal. **One dropped `forceFullRefresh` produces both halves.**

 Compounding it, `requestVisiblePaint` checked `handoverPaintSuspended` and
 returned *above* the `pendingVisiblePaintForceFullRefresh` latch — the one
 thing that survives coalescing — so a full refresh demanded during the veil
 was DESTROYED, not deferred. The drop site's comment claimed "the resume path
 repaints from the daemon's own bytes"; the resume path deliberately does no
 daemon replay (field guide §5) and passed `false`. Two sites owned "who
 repaints after the veil" and they disagreed.

 **Live evidence on jojo 2.12.22, the user's own GUI (pid 2094127):**
 `daemon_handover/handover_paint_suspended` → `handover_paint_resumed` at
 16:12:44→16:14:15, 16:28:37→16:30:09 and 18:03:10→18:04:41 — three windows of
 **~91 s each in which terminals painted nothing**, every one released by
 `resumed_timed_out` (the 90 s `suspend_ceiling_ms`), every one with
 `fingerprint == resolved_fingerprint == pid=2050347:2.12.22`: **same daemon,
 same version, no update in flight.** Reproduced on a shadow client
 (`agent-render`, pid 2184903, 18:00:52) — screenshot shows the veil over the
 viewport beside a rail reading Client 2.12.22 / Daemon 2.12.22 / uptime 2h23m
 / "5 owned · 9 total · 4 preserved".

 **The fix, two edits in the terminal host script:** latch the full-refresh
 demand *before* the suspension can return (drop the FRAME, never the DEMAND),
 and resume with `redrawTerminal('handover-paint-resume')` — the exact repaint
 the user performs by hand, atlas clear + `term.refresh(0, rows-1)` over the
 CLIENT's own buffer. It is **not** a daemon-screen replay, and it is
 deliberately **not** gated on output silence: an agent CLI is never silent
 (see §THE PATTERN BEHIND THREE SEPARATE BUGS below), and this is not
 speculative correction — it is the settle of a window we ourselves blanked.
 Locks: `a_suspended_host_defers_a_full_refresh_demand_instead_of_destroying_it`
 and `a_handover_paint_resume_redraws_the_whole_client_buffer`, both red-proven
 by restoring the two production statements.

 ⚠ **Two things still owed.** (a) **Not live-verified** — jojo runs 2.12.22,
 which predates both this and the false-gate arming fix (`c88324e`, on main,
 undeployed). After the next deploy, confirm by opening a CC session and
 grepping the trace for a manual-redraw with reason `handover-paint-resume`,
 and confirm the veil no longer arms on a steady preserved-owner count.
 (b) `c88324e` stops the *false* arming; this fix is still required, because a
 REAL handover would leave exactly the same broken bottom without it.

3. ⚠ **YouTube frame judder with "overlaps"** while YouTube's own stats-for-nerds
 reports almost no dropped frames. ⚠ That reading FALSIFIES the decode
 explanation: if frames are not being dropped, the decoder is keeping up and
 the fault is in PRESENTATION, not decode. "Overlaps" reads as stale frame
 content persisting, i.e. damage/compositing, not pipeline. The
 `GST_PLUGIN_FEATURE_RANK` default shipped in 2.12.22 is still correct on its
 own merits but is NOT the explanation for this.

 **NOT ROOT-CAUSED. It does NOT share a root with symptom 2** — that was the
 working hypothesis and it is dead: symptom 2 is deterministic and lives in
 the handover veil, which is off most of the time. What the 2026-08-01 pass
 found instead is a real, previously-unread presentation-layer suspect, and
 what it eliminated:

 **`app_render_storm` is live, large and unexplained.** On jojo 2026-08-01 the
 Dioxus root rendered at **85–118 renders/s for a continuous 30 minutes**
 (16:57:40→17:27:40) and in 202 one-minute windows across the trace, against a
 calm baseline of 0.8–0.9/s. Measured cost while it ran at 33–47/s: the GUI's
 **main thread at ~42% of one core** (`/proc/<pid>/task/<pid>/stat`, 2 s
 deltas — not the `ps` lifetime average, which lies). That thread is the GTK
 main loop, which is where the UI process composites every web surface's
 DMABuf, so it is a plausible mechanism for frames that decode on time and
 *present* late or twice. **Plausible is not proven — nothing here measured a
 frame.**

 **The autopsy has been shipped since run 4 and was never read (see §Residual
 threads). It has now been read, and it answers its own discriminator:**
 `forced_wakes: 0`, `unattributed: 506–510 of 512`, `shellstate_mut: 1–6`. Per
 the arm site's own comment that means **NOT a caller of ours over-scheduling**
 — do not go audit `schedule_update` call sites, that lane is closed.

 Eliminated, each with the measurement that killed it:
 - **Terminal output forwarding** — 2.0 forwards/s while the root rendered at
   85/s. Decoupled.
 - **`safe_shell_mut` / any ShellState field** — 1–6 mutations per 512 renders.
 - **The handover veil** — 191 of 202 storm windows fall outside every paint
   suspension (19% storm rate inside vs 5% outside: enriched, not causal).

 Left standing: an app()-scope `use_signal` written outside `safe_shell_mut`,
 or a Dioxus-internal wake (a task/eval/future resolving every frame).
 ⚠ **The instrument cannot currently tell those apart, and its blind spot is
 load-bearing:** `FORCED_WAKE_TOTAL` only wraps the `schedule_update()`
 closure app() hands to its own 21 callers, so "forced_wakes: 0" means "none
 of *our* 21 asked" — it can never see a Dioxus-internal wake. Next step is to
 widen the autopsy (per-`use_signal` write attribution, or a Dioxus scope-wake
 hook), not to guess. Strongest correlate to chase first:
 `terminal_mount/forward_protocol_only_output` runs **75× higher** during
 storms (15.1/min vs 0.2/min) while `terminal_io/dispatch` is flat.


## A HOVER-REVEALED CONTRIBUTED RAIL PANE DRAWS ITS HEADER AND NONE OF ITS ROWS

**Status:** OPEN

**A HOVER-REVEALED CONTRIBUTED RAIL PANE DRAWS ITS HEADER AND NONE OF ITS ROWS**
(found 2026-08-01 while live-verifying the hover-reveal context-menu fix; NOT
caused by it — reproduced identically on the deployed 2.12.23 binary and on
the fixed build, on two separate shadows). Open a yedit session so the rail
shows its contributed `notes` pane: docked it has 27 `[data-app-pane-row]`
rows; hide the rail and hover-reveal it and the card reads `notes` with
**zero** rows and 7 nodes of content total. The reveal resolves the right MODE
(`right_panel_reveal_mode`) but the pane's schema is not there to render.
Consequence for verification as well as for the user: the one rail surface
with right-clickable rows cannot be exercised in the hidden+revealed state at
all, which is why `rail_autohide_pinned`'s new menu term is unit-proven but
not yet live-proven end-to-end.


## ⚠ TOOLING: agents have no first-class access layer for DELEGATE sessions

**Status:** OPEN (feature debt, user-requested 2026-08-02 — "easy for you
(agents) access layer on yggui automations")

The delegate-session pattern (a guiding agent launches an interactive
CC/codex session in a yggterm row for the user to answer) is now the standing
work pattern, and every step of it is hand-rolled: `--kind claude-code`
cannot pin model/permission-mode (and inherits the user's default model — a
live cost trap), a pending AskUserQuestion is invisible off-screen (JSONL
gets it only when answered; client read-buffer is blank for never-activated
rows), row order is only a debug field, and `terminal send`'s `accepted:true`
cannot see whether the old child still owns the PTY. Eight ranked, costed
feature asks: **[`docs/agent-bg-sessions-dream-2026-08-02.md`](agent-bg-sessions-dream-2026-08-02.md)**.
Interim recipe + traps: data-fabric skill §THE BG-SESSION PLANE.

## ⚠ TOOLING: app state's DOM debug snapshot times out on jojo, so every DOM

**Status:** OPEN

**⚠ TOOLING: `app state`'s DOM debug snapshot times out on jojo, so every DOM
probe field is unreadable there** (found 2026-08-01 while verifying the yedit
gutter). `dom_debug_snapshot_timeout` comes back on BOTH a shadow client and
the user's own GUI, and it takes the pre-existing `document_editor_count` with
it, so this is not new and not caused by any one lane. The cost is that a
field wired into the snapshot cannot be verified through the documented
instrument — the gutter's `document_wrap_gutters` had to be proven through
`dom-eval` instead. **Fix the timeout, or `app state` quietly stops being the
probe the field guide says it is.**


## ⚠ TOOLING: server app dom-eval ignores --client / --pid placed before

**Status:** OPEN

**⚠ TOOLING: `server app dom-eval` ignores `--client` / `--pid` placed before
the script.** It takes the script positionally at `args[3]`, so
`dom-eval --client shadow '<script>'` silently evaluates the STRING
`--client` — a successful-looking eval of the wrong thing, which is the
lie-of-success shape. The global override works only with the script FIRST.
Either parse the flags or refuse a script that looks like a flag; the skill's
example should be reordered either way.


## ★★ THE APP ACTION POST DOES NOT NAME ITS SESSION, AND THE DOCUMENT CHANNEL IS

**Status:** OPEN

**★★ THE APP ACTION POST DOES NOT NAME ITS SESSION, AND THE DOCUMENT CHANNEL IS
SESSION-SCOPED (oc, 2026-08-01). Cost: a 2.5-hour silent hang in yRDP.**
`document_pane_run_action` (`shell.rs:49802`) and `app_pane_run_action`
(`shell.rs:50203`) both send `{"pane", "action", "values", "value_keys"}` —
**no session** — and `app_pane_schema_url` adds none either. Yet the document
channel is session-scoped on OUR side and says so in its own doc comment: the
GUI resolves `control_url` *from* `session_path`
(`sidebar_control_url(&session_path)`), fetches per session, and applies the
reply per session. We know exactly which session acted and decline to say.
**What it costs an app.** A libyggterm app whose daemon is per-HOST but whose
view clients are per-SESSION cannot address its own answer. yRDP declares
`{"session": …}` on the OSC, does the work, and then has nowhere to send the
result: the connect outcome was filed under `""` while the client polled
`/events?session=<its id>`. The operator watched **"Connecting"** for two and a
half hours with the guest up, the RDP session live, and the viewer URL built
and finished in a mailbox with no reader. **No error, no log, nothing wrong to
find** — and the placeholder text ("a guest that is not running is started
first") actively pointed at the wrong subsystem.
**The fix**: echo the session the app declared back on the wire — the pane
fetch and the action POST of the document channel at minimum. It is page
context exactly like `host`/`zoom`/`secure`, which we already send, and it is
the one piece of context the app cannot derive.
⚠ **Every future libyggterm app with a durable per-host daemon hits this**, and
it fails SILENTLY and looks like the app's bug. yRDP now works around it by
registering clients on their own poll and refusing by name when it is genuinely
ambiguous (`daemon.py:route` in github.com/yggdrasilhq/yRDP, and §5 of its
`docs/architecture.md`) — that fallback is written to go cold the moment this
is fixed, so it does not need removing first.


## ★★ "YCHROME SUDDENLY QUIT TO TERMINAL"

**Status:** OPEN

**★★ "YCHROME SUDDENLY QUIT TO TERMINAL" — a fleet binary deploy arms a
refuse-exit landmine (user-reported 2026-07-30 night; live-diagnosed and
CURED on the live host, the DESIGN fix still owed).** Deploying a new
`ychrome` binary makes the RUNNING daemon stale; `ensure()` then "refuses
loudly on stale+busy" (round 27) — which means **every fresh `ychrome
<url>` invocation exits immediately** until someone runs `ychrome daemon
restart` by hand. To the user that is "ychrome suddenly quits to the
terminal", hours after an agent deployed binaries. Compounding it: two of
the live host's ychrome rows were LOCAL-VARIANT placeholder wedges (the
same class as the remote-row wedge below — planning banner, launch never
fired), so their relaunches went into dead PTYs. What cured it live:
`ychrome daemon restart` (honest handover, surfaces re-registered), then
cycling each session's CLI onto the new binary (`routable=yes` returns; a
session with no saved page comes back at the profile picker — honest).
**The design fix owed:** a new invocation against a stale-but-busy daemon
should ROUTE into the running daemon with a one-line stale warning — the
old code is still serving perfectly well — and retirement should happen on
the user's schedule, never as a precondition for opening a page. A refusal
is only honest when routing is genuinely impossible.
⚠ Related trap for agents: after ANY ychrome fleet deploy, the daemon on
every GUI host is stale by definition — hand it over as part of the deploy
(clients and daemon together per the round-29 mixed-version note), or the
user hits the landmine.


## ★★★ REMOTE ROWS WEDGE IN RemoteBootstrap AFTER A DAEMON VERSION HANDOVER

**Status:** OPEN

**★★★ REMOTE ROWS WEDGE IN `RemoteBootstrap` AFTER A DAEMON VERSION HANDOVER
(found live on the 2.12.18 → 2.12.19 bump, 2026-07-30 night).** After the
GUI+daemon swap, every `remote-cc://` / `remote-session://` row the new
daemon adopted sat permanently on the 5-line planning placeholder ("Queue
remote Yggterm resume … Daemon PTY: request main viewport terminal stream"):
the adopted row's launch request never fired, so no ssh chain was ever
spawned for the new owner, while the OLD daemon's viewer chain kept
streaming into a grid nobody views. 15 rows wedged; the user-facing symptom
is a blank viewport on every remote agent row — the product's core handoff,
dead until manual recovery. **Recovery that works, verbatim, per row:**
`yggterm-headless server terminal restart '<session>'` (all 15 accepted,
all reached `Running` with real content; a freshly re-attached CC repaints
fully on its next input/output — nudge an idle row with a bare Enter).
Root-cause candidates to investigate BEFORE the next bump: the adopted
row's `request_terminal_launch_for_path` queue never draining for
preserved-owner rows, and the one-viewer contract holding dev-side slots
for the old daemon's chain. ⚠ **Do NOT bump dev/oc daemons until this is
fixed** — their next version transition hits the same wedge, and
fleet-binary-sync may carry 2.12.19 binaries there on its own.
⚠ Related, observed in the same window: `server snapshot` on the NEW daemon
shows the placeholder for preserved-owner rows (the documented
snapshot-lies trap), and `update-daemons --force` PRESERVES runtimes on the
old socket rather than migrating them — neither is the recovery; the
per-row `terminal restart` is.


## ★ THE SUPERVISOR DIES WITH ITS CHILD

**Status:** OPEN

**★ THE SUPERVISOR DIES WITH ITS CHILD — confirmed twice in one day (jojo,
2.12.18, 2026-07-27, ~17:15 and ~23:10).** `kill -TERM <gui-child>` is the
documented GUI-swap recipe ("the supervisor relaunches the new binary"), but
both times the `yggterm --supervise` parent exited WITH the child and nothing
relaunched — the desktop went GUI-less until a manual
`setsid yggterm --supervise` with the desktop env re-exported. Round 26
recorded the recipe working, so either a regression or the supervisor treats
a TERM'd child as deliberate shutdown. Find the supervisor's child-exit
policy: a child that exits on SIGTERM during a binary swap must be
relaunched; only a supervisor-addressed TERM is a shutdown order. Recovery
recipe that works, verbatim: read WAYLAND/XDG/DBUS env off a live desktop
process → `setsid ~/.local/bin/yggterm --supervise </dev/null &`.


## ★★ WEBAUTHN / PASSKEYS ARE UNREACHABLE ON AN AGENT-CREATED SURFACE

**Status:** OPEN

**★★ WEBAUTHN / PASSKEYS ARE UNREACHABLE ON AN AGENT-CREATED SURFACE
(2026-07-28).** Full field report:
**[`docs/agent-passkey-gap-2026-07-28.md`](agent-passkey-gap-2026-07-28.md)**.
Written from a real deadline job (minting a Cloudflare DNS-01 token to renew
the expiring `*.gour.top` wildcard). The passkey machinery is built and
correct; it is simply **never wired to a surface an agent makes**:
1. **Surface policy is bound ONCE, at `open_web_surface` time**
   (`crates/yggterm-shell/src/shell.rs:8715`). A surface built while
   `web_surface_policy_gate()` is still `Pending` gets `userscripts: []` AND
   `signer_base: None`, permanently — nothing re-fits it when the policy
   lands. Our surface's own trace says `{"policy":false,"signer":null}` on
   every tab. Result: `window.PublicKeyCredential` is **undefined**, and the
   relying party (Cloudflare) renders "your browser does not support security
   key". A human wins that race by sitting still for a second; an agent never
   does. This is why "⚠ still owed: full crypto E2E against a real relying
   party" is still owed.
2. **A hand-injected shim cannot rescue it** — `yggterm-appctl://signer` is
   not a registered scheme on such a webview (`TypeError: Load failed`), so
   the fix must be in the construction path, not in a userscript.
3. **`web ensure` silently reset a live, logged-in page to `about:blank`**
   after the 600 s lease lapsed, reporting `healed: false, leased: true`. It
   discarded a half-finished 2FA. Survived only because the cookie jar is
   per-profile.
4. The 600 s lease is **unreadable and un-renewable**; `web eval` returns
   `null` for statement-form scripts (`if (…) {…}` has no completion value),
   which makes a click that DID fire look like a failure.
Smallest fix that makes passkeys real for agents: **have `web ensure` await
`SurfacePolicyGate::Ready`** before returning.

→ **Fix built (`lane/dev/ensure-policy-gate`), awaiting live verification.**
`web ensure` now awaits the policy gate before arming a build: bounded 8 s
wait, exhausted fetches are re-armed and re-driven in the same call, and a
gate that never lands refuses with `reason: policy_gate_not_ready` naming
the gate state — never a silent unprotected build. The exhausted state is
now named (`SurfacePolicyGate::Abandoned`) instead of folding into `Absent`,
and every ensure envelope reports `policy_gate`. Items 1–2 of this entry are
covered on the agent path; item 3 (silent `about:blank` reset) and item 4
(lease invisibility, `eval` statement-form nulls) are NOT fixed. Remove this
entry only after a live passkey ceremony on an agent-created surface.


## ★★ AGENT CO-BROWSE CANNOT COMPLETE AN OTP LOGIN

**Status:** OPEN

**★★ AGENT CO-BROWSE CANNOT COMPLETE AN OTP LOGIN — the logged-in plane stops
at the door (2026-07-28).** Full field report, seven confirmed defects and
nine costed feature asks: **[`docs/agent-cobrowse-gaps-2026-07-28.md`](agent-cobrowse-gaps-2026-07-28.md)**.
Written from a real job (building two diagnostic-lab orders end to end), not a
synthetic test. The headline four:
1. `web do fill --selector-set` refuses `surface_not_mapped` on a shadow
   surface, and the eval fallback fills segmented OTP boxes **visibly but
   without updating React state**, so the form posts an empty code and the
   site shows no error. Reads like a wrong OTP; is not. Same wall already
   recorded at a services portal. **An agent can read the SMS code off the phone in
   five seconds and then cannot type it.**
2. `el.click()` silently no-ops on many React handlers; a full
   `pointerover→…→pointerdown→mousedown→pointerup→mouseup→click` sequence at
   real coordinates works. Should be `web do click --gesture full`.
3. **ychrome is single-instance per profile and silently reuses the running
   session** — a second `ychrome --profile X <url>` replaces the existing
   page instead of opening a tab. Destroyed a live page mid-job.
   **✅ FIXED IN-TREE (ychrome merge d3dae32, 2026-07-31):** a routed url
   reports "opened as a new tab in the running session" and exits 0; an
   unrouted url on a stream with another pid's live anchor REFUSES by name
   (never a silent hijack); every anchor-here fallback names its reason.
   4 locks red-proven. ⚠ Live verify owed; residuals in the lane report
   (suspended-sibling anchor, no-arg picker path).
4. `YGGTERM_APP_CONTROL_PID` is honoured by `terminal new` but NOT by
   `web ensure`, which then refuses while naming that same variable.
   → **Fixed on `lane/dev/ensure-policy-gate`, awaiting live verification.**
   Root cause: BOTH binaries' `server app` dispatch blocks REMOVED the
   exported variable whenever the invocation carried no `--pid` flag, so the
   ambient default never survived to resolution and whether a verb appeared
   to honour it depended on the client roster. Targeting now goes through
   one owner (`yggterm_server::apply_app_control_target_overrides`): an
   explicit flag wins, no flag leaves the exported environment standing.
Highest-value asks, in order: trusted input into an unmapped surface (D1),
`--gesture full` (D2), verb-level `--expect` post-conditions (D3 — this run
reported five "successful" add-to-cart clicks that had all failed), and
multiple tabs per profile (D4).


## ★★ THE DAEMON'S ENVIRONMENT IS FROZEN AT LAUNCH AND POISONS EVERY SESSION IT

**Status:** OPEN

**★★ THE DAEMON'S ENVIRONMENT IS FROZEN AT LAUNCH AND POISONS EVERY SESSION IT
EVER SPAWNS — including across hot-restarts (oc, 2.12.18, 2026-07-28).**
Observed: on oc, `claude` in every yggterm-launched session died with
`Failed to authenticate. API Error: 403 ... Received Model Group=vercel/maa/deepseek-v4-pro`
— a retired custom-gateway config the user had already deleted from
`~/.profile` and `~/.bashrc`. Editing the rc files changed nothing, because
the rc files are not on the launch path at all.

Mechanism, all three links confirmed in the source:
1. `~/.profile` used to `. ~/.claude_code_env`, which exported
   `ANTHROPIC_BASE_URL` / `ANTHROPIC_API_KEY` / `ANTHROPIC_*_MODEL`. The
   daemon (PID 2397674, started Jul 27 17:09) captured that env at exec time
   and is orphaned to PID 1. The user deleted the file the next morning; the
   running daemon kept its copy.
2. `terminal.rs::shell_command()` builds `bash -c '<launch_command>'` — a
   **non-interactive, non-login** shell that never sources `~/.bashrc` or
   `~/.profile`. It calls `env_remove` only for
   `terminal_identity_env_removals()` (the TERM/appearance keys). Everything
   else is inherited from the daemon verbatim.
3. `lib.rs::spawn_daemon_process_from_executable()` (the hot-restart spawn
   path) does no `env_clear`/`env_remove` either — so a hot-restart *copies
   the stale environment onto its own successor*. Once a daemon is poisoned,
   the poison is immortal on that host; only a full daemon death breaks it,
   which the constitution forbids while sessions are live.

Net effect: any variable exported in whatever shell first started the daemon
becomes permanent, invisible, host-wide configuration for every agent CLI
yggterm launches, and the user has no rc-file edit that can reach it.

**Worked around, not fixed.** oc's `~/.claude/settings.json` now carries an
`env` block pinning `ANTHROPIC_BASE_URL` back to `https://api.anthropic.com`
and blanking the rest; Claude Code's settings `env` beats the inherited
process env (verified by running `claude` under the daemon's exact
`/proc/<pid>/environ` — the poisoned `ANTHROPIC_BASE_URL` is still inherited
and the call still authenticates through the subscription). That is a
Claude-Code-specific patch on one host; it does nothing for codex, for other
vars, or for the next host that catches this.

**The real fix is a design call, not yet made:** should the session-spawn
environment be re-derived from the user's login shell (allowlist) rather than
inherited from the daemon, and should hot-restart re-exec its successor with a
fresh environment instead of copying its own? jojo and dev daemons are
currently clean, so this is latent everywhere, live nowhere.


## A SECOND VIEWER STILL BUILDS ITS OWN WEBVIEWS (residual of the J8a entry;

**Status:** OPEN

**A SECOND VIEWER STILL BUILDS ITS OWN WEBVIEWS (residual of the J8a entry;
the STRANDING half is fixed, see below).** Webviews are per CLIENT, so a
shadow — or a second GUI — showing a 10-tab surface builds a second full set
(J8a: 11 more processes). That is what makes co-browsing work at all on the
current per-client surface model, and both sets are governed by the same
reclaim lane, so this is a cost rather than a defect. It is recorded only
because the memory arithmetic is easy to forget: a second viewer of a heavy
session roughly doubles the GUI-side web-process bill for as long as both are
shown. The half that WAS a defect — `session remove` answering `verified:true`
while the other client kept its set alive forever, with no row anywhere —
is fixed on 2026-08-01 (lane `lane/dev/webview-leaks`): every client now
sweeps the sessions it holds webviews for against the tombstone plane, in the
same conjunction `web ensure` refuses to revive on. Measured in the sandbox
before: GUI 2 → 1 webviews on a verified removal while the shadow stayed at 2;
after: the shadow drops with it. See `docs/web-surfaces.md` §Three ways a
process was minted for nothing.


## GUI process died mid-J8a with 51 webviews applied

**Status:** OPEN

**GUI process died mid-J8a with 51 webviews applied (jojo, 2.12.17 GUI 27779
→ fresh 325652 at 12:17:22, 2026-07-27). Cause UNDETERMINED** — no panic in
the trace, no readable OOM record; the 50-webview ramp stage had completed
one minute earlier, so the correlation is owned, not proven. The daemon
never blinked and every row survived (the constitution held).
**2026-08-01: the PRECONDITION is gone, the cause is not.** The only path that
could apply ~50 webviews to one GUI in a single call was `web ensure`'s
per-tab mint, fixed on `lane/dev/webview-leaks` and measured at 1 → 15 web
processes before / 1 → 1 after on a 13-tab surface. So this state is no longer
reachable by an agent verb — but nothing here explains WHY that GUI died, and
a user with 50 revealed tabs can still reach a similar count one reveal at a
time. Still a watch item: if a fresh GUI dies again near a large applied
webview count, this becomes the top entry.


## The profile picker CARD ITSELF still cannot be raised or photographed from

**Status:** OPEN

**The profile picker CARD ITSELF still cannot be raised or photographed from
the plane (successor to the J8b entry closed 2026-08-01).** Its row-menu
WRITES are now reachable — `server app web profile <list|show|avatar|protect|
unprotect>`, which is what closed the avatar-persistence hole — but nothing an
agent can drive opens the picker SURFACE, so the card's rendered avatar cannot
be screenshot-verified. The write is provable; the paint is not. Small, and
strictly narrower than the entry it replaces: `unknown_keys` in the verb's
answer covers the contract that had no proof at all. Fix when convenient: an
addressable route that reveals a picker surface (the rail/strip badge opens
the profile SWITCHER menu, `webprofile:<name>` entries only), after which the
existing `app screenshot --client <shadow>` does the rest.


## A WebKitNetworkProcess OUTLIVES the WebContext that started it, and

**Status:** OPEN

**A `WebKitNetworkProcess` OUTLIVES the `WebContext` that started it, and
nothing we own can reap it (WebKitGTK behaviour, measured 2026-08-01).** The
residual of the "accumulates per profile churn" entry, which is otherwise
fixed. Our half was real and is gone: every destroy-and-recreate used to mint
a fresh `WebContext` (5 reloads = 5 network processes; J8a's 3 → 10 is 7
recreates), because the sweep ran inside `close` and took the engine in the
gap before the create. With the sweep moved to the tick, five reloads leak
zero. What remains is that dropping the LAST reference to a context leaves its
network process running: with `web_context_count()` at 0 and every surface
gone, the GUI still held 2. So the standing bill is **one network process per
distinct `web_context_key` the GUI has EVER opened**, not per live context.
Small (they are far lighter than web processes) and bounded by profile count
per GUI generation, but a long-lived GUI that cycles many profiles pays it.
Not obviously ours to fix — the next step, if it ever matters, is whether
`WebsiteDataManager`/`WebContext` disposal has an explicit terminate we are
not calling, or whether webkit2gtk simply keeps them for reuse.


## server app open on a REMOVED row times out instead of naming the reason

**Status:** OPEN

**`server app open` on a REMOVED row times out instead of naming the reason
(minor, jojo 2.12.17, 2026-07-27).** Opening a deleted session path correctly
does NOT resurrect the row and correctly leaves the active session untouched
(no select/activate events fire), but the CLI answers
`Error: timed out waiting for app open to settle …`. Compare `web ensure` on
the same class of dead path, which is exemplary: `accepted:false`,
`reason:"session_closed"`, `row_close_remembered:true`, plus prose naming why
and what to do instead. Make `app open` refuse in that shape rather than
time out.


## ★★★ AN UNREVEALED AGENT SURFACE REPORTS visibilityState: "visible", SO

**Status:** OPEN

**★★★ AN UNREVEALED AGENT SURFACE REPORTS `visibilityState: "visible"`, SO
ITS PAGE ANIMATES AT FULL RATE AND THE GUI COMPOSITES IT — measured
2026-07-26 night, and this is very likely THE mechanism behind every
"agents make the GUI host hot" report in this campaign. ⏳ FIXED IN-TREE AT
2.12.17; THE LIVE A/B IS OWED AND IS THE ONLY THING THAT CLOSES THIS.**
Ground truth: a payment-gateway page on a headless, never-revealed surface
the user cannot even see (no row — see the entry above) reported
`visibilityState: "visible"` with **1 running animation** (a spinner). Cost,
measured over 20 s from `/proc` (never `ps %CPU`): **web content 0.241 cores
+ GUI 0.399 cores = 0.85 cores total against jojo's ~0.5-core idle floor**,
Tctl 61.6 °C — the user's fan spun up on a machine that had been silent all
evening, while they were touching nothing.
**Why it is a product bug, not the page's fault:** every browser throttles
`requestAnimationFrame`, CSS animations and timers on a hidden page — that is
the Page Visibility contract, and it is the ONLY thing that makes background
tabs cheap. Our unrevealed surfaces claim to be visible, so the page has no
way to know it is not on screen and paints forever, and the shell composites
every frame of a surface nobody is looking at.
**THE FIX, AS BUILT.** WebKitGTK derives `document.visibilityState` from
**widget mapping** — there is no page-visibility setter on this API — so
"hidden to the engine" means "the inner webview is not mapped". Three
independent halves each kept that from ever happening, and fixing any one
alone would have left the bug alive:
1. **Creates were born visible** — `open` ended in an unconditional
   `show_all()`, so even a headless create was realized and mapped. An
   unrevealed create now hides the inner view immediately.
2. **The headless create demoted but never throttled** — `demote` is a
   Z-order move, not a visibility one. The reconciler now throttles beside
   the demote and records `engine_visible:false` in the trace.
3. **The reclaim pass could never reach it later, and exempted the leased** —
   a headless surface is marked stashed in the same breath, so the background
   plan classified it `Wait` forever; and when reached, `throttle: !leased`
   exempted every leased surface while `web ensure` leases unconditionally.
   **A LEASE IS A CLAIM ON EXISTENCE, NOT EVIDENCE OF A VIEWER** — it says the
   surface must keep existing and nothing about anyone looking at it.
**The trap that makes or breaks it:** an unmapped webview silently DROPS
synthesized events, and hiding is exactly what unmaps — so this would have
turned every `do`/`fill`/`type`/`key` into `surface_not_mapped` on precisely
the surfaces agents drive. The engine host therefore **wakes a view it hid for
the length of an injection burst and re-hides it after** — borrow-and-give-
back with a per-surface re-arm token, the same shape as the keyboard-focus
loan, and the same rule that a give-back only takes back what is still ours.
If the wake does not map, it is undone and the injection REFUSED: a refusal is
honest, a dropped event is not. Visibility gates RENDERING, never the drive
path; the audio veto is untouched, and the decision is per GUI process, never
a daemon query.
⚠ Do NOT "fix" this by navigating agent surfaces to `about:blank` between
actions — that is the workaround (correct for an agent to do voluntarily,
and it is now in the agent brief) but it hides the defect and breaks any
flow whose page state must survive. Nothing in the shipped fix navigates:
DOM, scroll, JS heap and in-memory bearers survive hiding untouched.
⚠ **THE LIVE A/B THAT CLOSES THIS, owed after the bump** — telemetry alone
cannot settle a heat claim: (a) an unrevealed surface reports
`visibilityState:"hidden"`; (b) `web do` and `capture-element` both succeed on
that SAME still-hidden surface (the wake/re-hide working, not a surface that
was quietly revealed); (c) a `/proc` cores delta against the captured 2.12.16
baseline (0.241 web + 0.399 GUI against a ~0.5-core idle floor) under the same
spinner page; (d) a faithful screenshot across background → reveal, because a
page that stops painting while hidden must come back correct; and (e) audio
keeps playing on an unmapped view.


## ★★★ A LIVE, LEASED WEB SURFACE CAN EXIST WITH NO ROW

**Status:** OPEN

**★★★ A LIVE, LEASED WEB SURFACE CAN EXIST WITH NO ROW — the user cannot see
or reach an agent that is browsing with their profile (found live
2026-07-26 night by a filing agent; user-reported the same hour as "why is
the agent row not in my Live Sessions, I cannot connect to it").**
Sequence, all reproducible: a previous run closed its work session (correct
hygiene — the row is TOMBSTONED in `removed-rows.json` and its PTY is dead,
`running:false, line_count:0`), and the next run called
`web ensure --session <that dead path>`, which happily **revived and leased
the surface anyway** (`already has a live surface`, generation 1). Result:
an agent drove a real payment gateway for an hour, on the user's
cookie profile, with **zero rows containing that session id** — nothing in
`server app rows` reflected that a surface was alive and being driven.
**The state "surface alive, row absent" should not be representable.** Two
candidate fixes, one must be chosen: (a) a surface holding a lease KEEPS (or
resurrects) a row for as long as the lease lives — which also satisfies the
constitution's UX test that the user can SEE an agent's session and click in
to co-browse it; or (b) `web ensure` REFUSES a session whose runtime is dead
and whose row is tombstoned, naming that reason, so an agent must create its
own session (and therefore its own visible row) instead.
**⚠ COROLLARY, same incident:** `web fill-card` then began refusing
`accepted:false, reason:"preempted"` — *"the user took this surface"* — on a
surface the user **cannot see, click, or have touched**. The agent-input
arbiter's preempt marker can be set on an unrowed orphan, so the human is
blamed for taking something invisible to them; and because the lane is keyed
`(session_path, generation)` with `forget()` only on close/recreate, the only
cure is a new surface generation. This is the same credit-ledger class as the
entry on injection credits leaking across the inter-verb gap — fix them
together.
**⚠ Practice note that made this worse, now corrected in the agent brief:**
every filing run tore its session down as a courtesy, so by the third run the
only thing left to attach to was an orphan. Agents should create ONE session
per run and LEAVE IT UP; visibility beats tidiness.


## ★★★ web do FIDELITY ON RE-RENDERING DOMs

**Status:** OPEN

**★★★ `web do` FIDELITY ON RE-RENDERING DOMs — three reproducible defects,
one family (a live portal filing run, 2026-07-26 ~15:30-16:00 IST, jojo 2.12.15,
session `local://b556fb1b`, all self-reported SUCCESS while wrong):**
1. **`do fill` DROPS and INVENTS characters on React controlled inputs.**
   `fill --selector '#street' --text "Sample Fixture Road"` → response
   `chars:19, delivered:true, is_trusted:true, cleared_verified:[true]`, field
   held **"Ja"**. Earlier `#username` fill reported chars:10, field ended
   **"0000000000hg"** — two stray chars never passed in any `--text`, which
   then poisoned the portal API call (404). ⚠ The strays coincided with the
   live focus-theft window (entry below) — possible seat/agent input
   cross-contamination; the focus investigation owns that half.
2. **Clear-verification false-negative:** batch fill on an EMPTY `#Landmark`
   aborted `clear_failed (box(es) [0] of 1 still hold text)` — the field was
   empty; likely verifying the previously-focused element, not the target.
3. **`--role option --label X` resolves a STALE RECT in a scrolled MUI
   listbox:** `--label PASSPORT` → `accepted:true, delivered:true,
   is_trusted:true`, nothing happened. Working recipe: tag the `li` by id via
   eval + `scrollIntoView({block:'center'})` + `do click --selector`.
**Common shape: the verb resolves/verifies against DOM state that has moved
(framework re-render, scroll, focus change) and its self-report cannot go
red.** Fix direction: verify-by-readback of the TARGET's final value against
the requested text (honest failure when mismatched), clear-verify the
resolved target element only, re-resolve rects after scroll before injecting.
4. **Duplicate DOM ids across repeated form blocks break `--selector`
   targeting.** The portal renders two party form blocks with the SAME
   ids (`#Name`, `#District`…); `#id` selectors silently hit the FIRST, so
   the agent drove the complainant's field while aiming at the OP's — twice.
   Agent-side workaround: strip injected ids from previous holders before
   re-tagging; address via `querySelectorAll("[id='X']")[n]`. Verb-plane
   want: `--nth` on `--selector` (or an ambiguity warning in the response
   when a selector matches >1 node).
5. **A stale MUI popper stays mounted and poisons the next pick** — after a
   failed pick, `li[role=option]` still returns the OLD listbox's options,
   so the next selection silently matches the wrong list. Proven recipe:
   `web do key --key Escape` before each pick (that verb works). Candidate
   verb-plane fix: role/option resolution scoped to the NEWEST open listbox
   (aria-expanded owner), not the first match in document order.
Falsified the other way (keep): MUI async Autocompletes ARE drivable via
`do click` + `--role option`; headless file upload via DataTransfer works
(379 KB PDF through one `web eval --stdin`, no GTK chooser).

**STATUS 2026-07-26 — ALL FIVE HALVES ARE CODE-FIXED, NONE LIVE-VERIFIED.**
The fix is one mechanism, not five patches: **the matcher runs ONCE per verb
and its result is PINNED** (`window.__yggDoPins`), and every later step —
clear, clear-verification, the write, the readback, the rect re-measure —
addresses that handle instead of re-running the selector. A re-render can no
longer substitute a twin between any two steps of a verb.
- (1) `fill` now READS THE FIELD BACK through the pin and reports
  `verified` / `verify_reason` / `requested` / `held` / `first_mismatch`.
  `delivered: true` and `verified: false` co-exist and that is the point.
  Plain text/textarea inputs are written with the **native value-setter +
  bubbling `input`/`change`/blur** (`mechanism: native_setter`), the filing
  agent's proven workaround; real keys stay for segmented widgets and for
  secrets, and the response always names which ran.
- (2) clear-verification reads the PINNED nodes' state; a node the framework
  re-rendered away is `node_replaced`, its own refusal, never `clear_failed`.
- (3) resolution is TWO phases — pin+scroll, settle 120 ms, RE-MEASURE the
  pin — and `web_do_resolved_from_info` REFUSES any payload not stamped
  `phase: post_scroll`, so collapsing them back cannot pass silently. The
  response carries `resolved.rect_phase` + `is_connected`.
- (4) CSS targets resolve via `querySelectorAll(sel)[nth]`; `--nth` works on
  `--selector` (wire: `{"css":…,"nth":…}`, bare string still means nth 0) and
  every addressed response carries `match {matches,nth,hidden,ambiguous}`.
- (5) `role=option`/`menuitem` pools are filtered for liveness and scoped to
  the listbox an `aria-expanded` combobox owns (else the last visible one);
  a pool of only stale options refuses `stale_listbox_only`, never a click.
⚠ **Live verification is OWED**: no live portal re-run, no jojo deploy. The
daemon/GUI on jojo still runs the old behaviour until the next bump.
Remaining agent-side: the INVENTED characters in (1) are still attributed to
the concurrent focus-theft bug — the readback now catches them, it does not
prevent them.


## ★★ THERE IS NO CLIENT TO RENDER AGENT SURFACES INTO ON dev

**Status:** OPEN

**★★ THERE IS NO CLIENT TO RENDER AGENT SURFACES INTO ON dev (2026-07-26).**
The data-fabric default "co-browse on a SHADOW surface on dev" is currently
unusable: `server app clients` on dev → count 0 (no GUI, no shadow client),
so the filing agent had to fall back to the user's live GUI host. Fresh
evidence for settled call #6 (drive shadow surfaces with the GUI closed /
server-side rendering, docs/optimization-pass.md WS2): today agent browsing
physically requires the user's GUI host.


## ★★ THE DAEMONS CHAIN, AND ONE IDLE bash -i IS WHY (root-caused

**Status:** OPEN

**★★ THE DAEMONS CHAIN, AND ONE IDLE `bash -i` IS WHY (root-caused
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


## ★★ A SESSION STRANDED ON A preserved OWNER HAS NO DECLARES, AND THE RAIL

**Status:** OPEN

**★★ A SESSION STRANDED ON A `preserved` OWNER HAS NO DECLARES, AND THE RAIL
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


## ★★★ THE FIFTH FOCUS PATH

**Status:** OPEN

**★★★ THE FIFTH FOCUS PATH — IT IS NOT JAVASCRIPT. Root-caused 2026-07-26;
✅ THE FOCUS-BORROW FIX IS SHIPPED AND USER-CONFIRMED LIVE ON jojo. What keeps
this entry open is the SECOND bug it filed — the injection-credit ledger —
plus the unexplained `fill` corruption at the end.**
The user, mid-session:
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
✅ **VERIFIED LIVE AND CONFIRMED BY THE USER** on the deployed GUI: a
480-verb agent burst driven against a headless surface while the user worked
in their own session, and they felt nothing — no steal, no interruption —
with zero focus/select trace events inside the burst windows and screenshots
taken across them. The user's own experience is the instrument that settles
this one: every JS-side probe is blind to a GtkWindow focus move, which is
how four earlier rounds all missed it.
⚠⚠ **It IS keystroke cross-contamination, one direction, CAUGHT LIVE.** A
passive `keydown` recorder installed in the agent's page logged three
`isTrusted:true` `Escape` presses — 16:09:35.815, 16:09:51.001, 16:23:42.600 —
with **no agent verb within ±8 s of any of them** (the agent's last verb ran
at 16:05:46). That is the human, pressing Escape at a terminal that had
stopped answering, and landing in an invisible the portal form instead. The
other direction is structurally impossible and stays that way: `synth_key`
hands the event to the surface widget with `gtk_widget_event`, which never
traverses the toplevel's focus chain, so an agent's characters can never
reach the user's terminal.
⚠⚠⚠ **AND THE ARBITER DID NOT NOTICE — the second bug, and the reason this
entry is still here. ⏳ FIXED, BUT ON THE UNMERGED LEASED-SURFACE LANE, NOT IN
main AT 2.12.17, AND NEVER LIVE-VERIFIED.** Real seat
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
protect them.
**THE FIX, AS WRITTEN:** credits expire on a short clock. Each credit is
recorded with the millisecond it was granted, and anything older than
`INJECTION_CREDIT_TTL_MS` (**250 ms**) is dropped before spending — a credit
covers ONE injected event GTK may deliver late, not the whole gap until the
next verb. The clock is injected at the entry points so the expiry is tested
exactly rather than by sleeping. ⚠ **It is NOT in main.** It rides the
leased-surface-with-no-row lane, which is still hardening its locks and has
not been merged, so nothing in 2.12.17 changes this behaviour — and the
ledger's own doc comment in main still says, correctly for main, that nothing
here expires on a clock. **Live proof owed after that merge and a bump:** a
real co-browse loop where the user types immediately after an agent verb and
the arbiter counts every one of their keystrokes.
The other reported corruption — `fill --text "Sample Fixture Road"` reporting
`chars:19 delivered:true` while the field held `Ja` — is **not explained by
either of the above** (17 characters lost, not 2 gained) and stays open; look
at the page's controlled-input re-render, not at focus.
⚠ **Focus-safe verbs, for anyone driving a surface over a working human:**
`web eval` / `read` / `wait` / `frames` / `screenshot` / `capture-element`
never touch GTK focus (guest-JS `element.focus()` sets DOM focus only). Only
`do` and the `fill*`/`totp` family take it.


## ★★ AGENT WEB-SURFACE AUTOMATION HARD-CRASHES THE GUI (WebKitGTK

**Status:** OPEN

**★★ AGENT WEB-SURFACE AUTOMATION HARD-CRASHES THE GUI (WebKitGTK
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


## ★ USER RE-CONFIRMED 2026-07-23 (during the 2.12.7 session): codex sessions

**Status:** OPEN

**★ USER RE-CONFIRMED 2026-07-23 (during the 2.12.7 session): codex sessions
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


## libyggterm apps over a MANUAL ssh hop say "not inside yggterm"

**Status:** OPEN

**libyggterm apps over a MANUAL ssh hop say "not inside yggterm"
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


## Live-path frame corruption on busy CC sessions

**Status:** OPEN

**Live-path frame corruption on busy CC sessions (jojo, 2026-07-10).** While
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


## Remote CC session stays permanently blank: resume-cc deadlocks before it

**Status:** OPEN

**Remote CC session stays permanently blank: `resume-cc` deadlocks before it
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


## 3.0.0

**Status:** OPEN

3.0.0 — the product does not build for Windows or macOS (NOT NOW; ~2 months out)

**Verified 2026-07-31 against GitHub Actions. User-scheduled: cross-platform is
IN SCOPE for 3.0.0, but 3.0.0 is at least two months away and there is a lot of
work ahead of it — do NOT start this lane unprompted.** Recorded here so it is
not rediscovered a third time. Recovered from the archived memory
`ci-release-cross-platform-failing`.

Two separable facts:

1. **Windows x86_64/aarch64 and macOS aarch64 fail to COMPILE** — not to package.
 Every `release.yml` run has ended in `failure` since ≥2.8.14; the Linux jobs
 pass, so the red went unread for twenty releases. Run 29164529909 (v2.11.0)
 ends with `could not compile yggterm-server (lib) due to 7 previous errors`:
 `env_flag_truthy`, `retire_stale_daemons`, `run_duplicate_legacy_owned_runtime_prune`,
 `versioned_server_socket_alias_candidates`, `parse_versioned_server_socket_name`
 (×3), and `no variant UnixSocket for ServerEndpoint`.
2. **No GitHub release has published since v2.11.0 (2026-07-11)** while jojo runs
 2.12.19 — eight versions exist only as local binaries and fleet `scp`s. This
 half is independent of the compile failure and cheaper to fix.

**The shape, as far as it is actually verified:** the versioned-unix-socket
daemon layer is `#[cfg(unix)]` on its DEFINITIONS — `ServerEndpoint::UnixSocket`
(daemon.rs:934), `parse_versioned_server_socket_name` (daemon.rs:348),
`versioned_server_socket_alias_candidates` (daemon.rs:362),
`refresh_legacy_server_socket_aliases` (daemon.rs:412), `retire_stale_daemons`
(daemon.rs:11791) — and at least one caller is unconditional:
**`lib.rs:81` imports `retire_stale_daemons` with no `cfg`.**

⚠ **The full unguarded-caller list is NOT enumerated yet, and do not guess it
from grep.** An earlier pass in this file cited daemon.rs:432/811/816/962 as
unguarded; every one of those is in fact inside a `#[cfg(unix)]` function or
match arm, and the claim was wrong. Get the real list from the compiler:
`rustup target add x86_64-pc-windows-msvc` works on the fleet (no MSVC linker
needed for `cargo check`), then
`cargo check --target x86_64-pc-windows-msvc -p yggterm-server`.

**When the lane opens:** fix by giving the socket/daemon-topology layer a windows
arm or gating the call sites — not by adding more `#[cfg]` to definitions, which
is what produced this. Add a CI job that `cargo check`s the windows target on
every PR; without that lock it regresses the moment it is fixed.


## ⭐ False/stale gates

**Status:** FIXED IN CODE — LIVE PROOF OWED

**⭐ False/stale gates — PROVEN 2026-08-01 by user screenshot.** The veil
("Daemon updating. Sessions will settle in a moment.") covered the viewport
while the pane beside it read **Client 2.12.22 / Daemon 2.12.22, uptime 35m** —
same version, nothing updating. The giveaway is on the same pane: **"3 owned ·
8 total · 5 preserved"**. `runtime_status_handoff_active()` is
`preserved_terminal_owner_count > 0`, and preserved sessions are a STEADY
STATE, so the veil is armed permanently and every mount shows it. User: *"Daemons
are updating even when same daemon is present"*, *"gating itself is so annoying"*.
**Fix:** arm on a genuine daemon IDENTITY transition (`pid:version` differs
from last observed), and let the awaiting-key slice only SCOPE which surfaces
are veiled. The notice is also raised unconditionally without consulting
`active_view_mode`, which is why it covered a yedit document and claimed "the
terminal is paused". ⚠ The self-check must run IN-PROCESS on the 2.5 s tick:
`server app state` REFRESHES the observation, so an external probe cannot
measure staleness and can itself arm the gate.


## ★★ THE YCHROME VIEWPORT Z-ORDER

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ THE YCHROME VIEWPORT Z-ORDER — UNDER-GLASS ARMED ON THE LIVE HOST,
USER CONFIRMATION OWED (was: "every hidden un-hidden trigger breaks and
recomputes the viewport", 2026-07-30).** Phase F under-glass IS the fix the
user described (page at the BACK of the z-order, chrome floating above it,
the terminal-canvas property) and it is now ARMED on the live host
(relaunch env + `~/.config/plasma-workspace/env/yggterm-underglass.sh` for
future logins; `YGGTERM_WEB_SURFACE_UNDER_GLASS=0` reverts).
**Acceptance evidence (2026-07-30 night):**
- Sandbox (headless sway + persistent wlr virtual pointer +
  `scripts/underglass-sandbox.sh`, REAL seat input): click-through the
  glass hole reaches the page (counter incremented); titlebar auto-hide
  reveal via genuine top-edge pointer motion painted ONLY the titlebar
  band — page pixels BIT-IDENTICAL during the reveal, and the whole window
  BIT-IDENTICAL after the cycle; session-switch hide/unhide returned the
  page BIT-IDENTICAL; 5-cycle reveal soak: zero page-region drift; corner
  molding (rounded glass hole) pixel-proven; INCIDENT GUARD: a second,
  never-revealed surface painted ZERO pixels, and closing the active
  surface left no bleed.
- Live host: armed relaunch, rows intact, full-window compositor capture
  + all four viewport-corner crops show the page compositing under the
  chrome with molded corners.
**STILL OWED before closing:** the user's own by-eye/feel confirmation on
real hardware across THEIR triggers (sidebar overlays included — the
sandbox exercised the titlebar reveal and session switches; sidebar
overlays ride the same floats-over-glass machinery but were not separately
driven), and a few days' soak against the 2026-07-26 incident class (a
shell that cannot paint shows whatever is behind it — visibility-truth
keeps unrevealed pages unmapped, and the sandbox guard held, but the
incident fired on the LIVE host's env, so the soak is the honest closer).
Instruments now in-repo: `scripts/underglass-sandbox.sh` (isolated armed
GUI + real-pointer + fast grim frames; see the script header for the
virtual-pointer recipe and its traps).


## ★★ AGENT-SPAWNED TENANTS INSIDE DAEMON-OWNED ROWS ARE IMMORTAL

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ AGENT-SPAWNED TENANTS INSIDE DAEMON-OWNED ROWS ARE IMMORTAL — the leak
class behind recurring "mystery heat" (convicted 2026-07-27, user-spotted —
⏳ FIXED IN-TREE AT 2.12.17, LIVE VERIFICATION OWED).**
Seven aged `ssh <fleet-host>` clients (oldest ~5 days) were found hanging
under `bash -i` shell rows on the integrator host, one of them holding a
13.6-hour remote `htop` at 0.16 cores on the GUI host — the user's fan paid
for a probe an agent abandoned days earlier. **Mechanism, and why it is
structural:** an agent uses a shell row for an interactive probe
(`ssh <host>` → a TUI), then abandons the row. Daemon-owned PTYs are
deliberately immortal — the row surviving IS the feature (the GTA-5 model)
— so everything RUNNING INSIDE the row becomes an immortal tenant that no
surface accounts for. The session-start ritual now sweeps this class, but a
sweep repeated every session is an unfixed bug by definition. Product fix,
three pieces, each respecting the settled row doctrine (rows themselves are
never touched) — **all three are now built (2.12.17)**:
1. **Per-row tenant cost visibility (instrumentation, no policy).**
   `server terminal tenants [<session>]`. ONE `/proc` reading serves every
   row, on demand: no loop, no cache, no timer, zero idle cost. It reports
   the foreground command, the whole descendant tree with per-process CPU,
   and the age of the oldest NON-SHELL tenant (the row's own shell is
   discounted, or every row looks aged). A row it cannot walk reports a
   NAMED gap (`preserved_owner_daemon`, `no_local_runtime`,
   `runtime_not_running`, `root_pid_unavailable`, `root_pid_not_in_proc`,
   `proc_unreadable`, `not_supported_on_platform`) with **every number left
   empty** — a faked zero reads as "this row is cheap", which is the lie the
   verb exists to end. A row whose runtime belongs to an older preserved
   owner is PROXIED to that daemon rather than referred to it: a referral
   that the caller must chase by hand is the same archaeology dig this
   replaces.
2. **Ownership stamping on headless creates.** Every agent CLI `terminal new`
   records the creating pid, this host and an optional `--purpose` into the
   row's metadata, and the stamp rides the persisted row across a daemon
   handover (including the preserved-owner adoption import) — so provenance
   outlives its creator, which is the whole point.
3. **Pre-declared ephemerality, opt-in at creation.** `terminal new
   --ephemeral --ephemeral-owner-pid <pid>` or `--ephemeral-idle-ttl-secs
   <n>` = the agent explicitly declares AT CREATION "reap this session when
   my owner is gone / after N idle seconds". **A BARE `--ephemeral` is
   REFUSED** (`EPHEMERAL_NEEDS_AN_EXPLICIT_RULE`): measured, not reasoned
   about — under `bash -c "<cli>"` the parent this CLI would have recorded is
   the wrapper bash, gone in milliseconds, and under `ssh <host> "<cli>"` it
   is sshd-session, gone at disconnect, so the convenient default armed
   owner-gone against a corpse and killed the row on the next chore tick. The
   reap rides the EXISTING background chore tick and closes through the
   daemon's ONE close path (`close_live_session_row`, tombstone before
   remove), tracing `ephemeral_owner_gone` / `ephemeral_idle_ttl` — so it is
   consistent with the requirement-3 ruling: the close is agent-declared up
   front, an explicit close scheduled early. The DEFAULT is unchanged: leave
   the row up, visibility beats tidiness; a declaration is write-once and
   only the agent CLI create path can make one, so unmarked and user-created
   rows are untouchable (the no-reap ruling stands).
Non-product half already done: the ritual sweep gained the aged-ssh probe,
and the twin duty (an interactive probe is exited by the task that opened
it) is recorded in the fleet memory.
⚠ **LIVE VERIFICATION OWED — this entry stays until all four are done on the
live host** (2.12.17 is not deployed; the running daemon has none of this):
a `tenants` walk that actually finds an aged `ssh` tenant under a real row
and names its age; a create-then-stamp round trip read back after the
creating process is gone; ONE real TTL reap observed end to end (declaration
→ chore tick → tombstone → row gone, with the trace event); and the negative,
which is the one that matters most — **unmarked rows, including the user's,
untouched across that same tick.**


## ★★ AN AGENT'S TEARDOWN CAN REPORT SUCCESS AND LEAVE BOTH THE ROW AND THE

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ AN AGENT'S TEARDOWN CAN REPORT SUCCESS AND LEAVE BOTH THE ROW AND THE
APP PROCESS ALIVE (user-reported 2026-07-26 ~23:50, third variant of the
same class that night — ⏳ BOTH HALVES FIXED IN-TREE AT 2.12.17, LIVE PROOF
OWED).** A background agent's final report said "work session removed"; the
user still saw the row hours later. Ground truth: the row was live, and the
app process it hosted was still running under its `bash -i`, parented by the
daemon. Two things made it invisible to search:
1. **`terminal new --kind shell` names every session "Workspace Shell"**, so
   an agent's scratch row is indistinguishable by title from a human's shell
   — and the campaign record separately flags "Workspace Shell" as a name a
   HUMAN debugging session has used, which makes blind cleanup dangerous.
   ⇒ Give agent-created sessions a title carrying the agent identity and
   purpose (the chip already carries the app profile — the TITLE should too).
   **FIXED:** creation through the app-control plane — and only that plane,
   since every human door funnels through `start_local_session_placed` and
   never reaches it — synthesizes `Agent <identity> <kind>[: <purpose>]` from
   the request's own `agent` field plus a new `--purpose` flag, parsed
   identically by both binaries. An explicit `--title` still wins, and the
   synthesizer asks `looks_like_generated_fallback_title` about its OWN
   output before shipping it, because a title the copy layer discards falls
   straight back to the humanized cwd leaf — the exact bug it exists to
   prevent.
2. The row's only records marker was the small profile chip, so every
   title-based probe missed it while the user's eyes found it instantly.
**Also wanted, and now built:** a teardown verb that is verified, not
asserted. `session remove` used to hardcode `"accepted": true` on any
successful round trip — transport success and nothing else; it read true
while the daemon's own message said "no live session for this path", and true
while the PTY teardown (which signals ONLY the direct child, never its
descendants) left the hosted app running. It now answers from evidence:
census the PTY child's process tree from `/proc` before, re-read the row and
re-probe each censused pid after (matched on command name so a recycled pid
cannot pose as a survivor, rejecting zombies so a corpse cannot), with a
bounded settle so a child still handling the hangup is not misreported. One
pure owner, `verify_session_removal`, turns that into
`{verified, refusal, reaped, still_running}`, and `verified:false` carries a
NAMED refusal: `row_still_listed`, `processes_survived`, or
`runtime_pid_unobservable` — the last being the cross-version case the
constitution warns about (a row whose runtime belongs to an older preserved
owner reports no local pid, and that is **unverifiable, not clean**).
Reporting only: the verb does not kill survivors, because escalating to that
changes what a removal does to a human's shell and is a separate call.
⚠ **LIVE PROOF OWED (this entry stays until then):** on the live host, TRY TO
MAKE IT LIE — remove a session whose shell forked a process that outlives the
PTY, and confirm the verb says `verified:false` with `processes_survived` and
names them; and confirm an agent-created row wears its own name in the
sidebar the user is looking at.
Pairs with the leased-surface-with-no-row entry: the two failure modes are
opposites (invisible surface vs invisible-to-search row), and both are
fixed by making agent-owned artifacts NAME themselves.


## ★★ web fill-card ADVErecordsSED WHAT THE CREDENTIAL PLANE FORBADE

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ `web fill-card` ADVErecordsSED WHAT THE CREDENTIAL PLANE FORBADE (found live
2026-07-26 at a real payment gateway's card form — FIXED IN-TREE, LIVE
VERIFICATION AND A DEPLOY PENDING).** The verb's help offered `--field
number|expiry|code|holder` while every agent call came back
`vault_cli_no_card_op`: yggterm reached the vault through the **CLI**, which
deliberately has no card op, while `card-secret` existed all along as an
**agent-socket** op the ychrome sidebar was already using. The agent burned a
staged application and an OTP discovering this at the last step.
**Route (b) was taken and then simplified by the user's ruling:** every
Bitwarden client can read a card cipher, ychrome-vault is one, so the UNLOCK
is the boundary and the only one — no grant, no per-use consent. `fill-card`
now speaks the agent socket directly, the field set is
`number|code|holder|exp-month|exp-year|expiry`, the only policy refusal is
`vault_locked` (which names `ychrome-vault unlock`), and every release leaves
one line in `~/.yggterm/vault/audit.log` naming field names, never values.
ychrome branch `agent-card-path`, commit `13a3bfe`.
**What is still owed:** neither repo is pushed or deployed, and no PAN has
crossed the path into a real form. The yggterm half works against the ALREADY
RUNNING vault agent (it only uses `card-secret`, which ships today) except for
the socket-path lookup, which reads `socket` from `ychrome-vault status` — a
field the deployed ychrome-vault does not yet report. **So a live run needs
the new `ychrome-vault` installed + `ychrome-vault handover` first** (cheap:
handover keeps the unlock), or the verb refuses with
`vault_agent_socket_unknown` naming exactly that. yggterm deliberately does
NOT fall back to a hard-coded `~/.yggterm/vault/agent.sock`: ychrome owns that
path, and a second copy of it is what goes quietly wrong the day it moves.


## ★★ A --no-activate CREATE MADE WHILE NO SESSION IS ACTIVE STILL ACTIVATES

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ A `--no-activate` CREATE MADE WHILE NO SESSION IS ACTIVE STILL ACTIVATES
THE NEW ROW — the adjacent gap left behind when the sidebar-selection jump was
fixed (⏳ FIXED IN-TREE AT 2.12.17, LIVE PROOF OWED).** With the start page
showing, an agent's `terminal new --no-activate` pulled the viewport onto the
agent's row. Selection was preserved in that case; activation was not.
**CAUSE.** The create's hand-back captured the user's view as
`Option<(path, view_mode)>`, where `None` meant BOTH "no session was active"
and "nothing to hand back" — and the restore read the second meaning, so it
no-opped on exactly the case that needed it, leaving the daemon snapshot's
activation of the new row standing.
**FIX (GUI-only, no daemon or protocol change).** The viewport becomes a
NAMED state: `PreservedViewport` is either `StartPage` or
`Session { path, view_mode }`, so the outer `Option<PreservedUserView>` is the
only thing that still means "this create hands nothing back". The start-page
restore goes through the same SSOT setter the viewport history's own
`StartPage` entry uses, and `show_start_page_when_no_live_sessions` is forced
FALSE rather than restored — while that flag is true, every later snapshot
promotes the first live row back to active, which is precisely the row the
create was told not to activate, so restoring it would re-open the bug on the
next poll. The create response's `null` active path is now true as well as
honest: it already reported `null` while the shell had in fact activated the
new row.
**RESIDUAL, stated rather than hidden:** the hand-back is client-local. The
daemon still marks a newly started session active whatever the flag said, so
any path that adopts daemon truth wholesale re-adopts the new row. The honest
fix for that half is daemon-side.
⚠ **LIVE PROOF OWED at the next bump (a J7 item covers it):** with the GUI on
the start page, `terminal new --no-activate` must leave the start page
rendered and report a null active path.


## 💬 DISCUSSION for the dev agent

**Status:** AWAITING A DECISION

💬 DISCUSSION for the dev agent — a remote desktop wears browser chrome, and the protocol cannot say otherwise (2026-08-01)

**Not a bug report and not a decided fix — a design call that needs one.** Filed
from an end-to-end UX pass over yRDP driven the way a person drives it (a
throwaway shadow client, a scratch Xvfb+VNC target, real pointer clicks through
`server app pointer`), because the thing that stood out was not broken, it was
*wrong-looking*.

**What you see.** Connect to a remote desktop from the yRDP chooser and the
desktop is revealed as a web surface — correct, that IS the transport (x11vnc →
websockify → a noVNC page on loopback). What comes with it is the whole browser:

- an address bar reading `http://127.0.0.1:6102/index.html?quality=9&compression=0&bg=262a33`
- back / forward / reload / history buttons
- the tab rail, listing this "tab" beside unrelated ones

None of that is addressable by the user in any useful way. The URL is a bridge
detail — a port yRDP chose seconds ago — and it is the one piece of text in the
window that looks like something you could type into. Reload re-dials the
bridge; Back has nowhere to go. A Windows desktop is not a page you browse, and
the frame says it is.

**Why it cannot be fixed app-side today.** `TerminalEvent::WebSurface` carries
`{action, session, url, title, profile, start_page}` — there is no presentation
field, so an app has no way to say "this surface is not a web page". On the
GUI's side `web_chrome_hidden` already exists and already does exactly the right
thing (omnibox, find bar and tab strip all collapse) but it is wired to ONE
input: `snapshot.page_fullscreen`, i.e. an element-fullscreen page. The
mechanism is built; nothing but the engine can reach it.

**The shape of the decision** (the dev agent's to make, and the reason this is
here rather than in a commit):

1. **A declared flag** — `bare: true` / `presentation: "surface"` on the
 web-surface open, feeding the existing `web_chrome_hidden`. Smallest change,
 and it puts the choice with the app that knows what the surface IS. Cost: a
 protocol field, and a permanent question of who else gets to claim it (a page
 that hides the address bar is also how a phishing surface would like to
 render — worth stating that the flag comes off the PTY, which a page cannot
 write, so the trust boundary is the same one the declare already has).
2. **Infer it** — no chrome for a surface whose URL is loopback and whose opener
 declared a viewport pane. No protocol change; a heuristic that will be wrong
 for the next app, and inference is what the geometry contract's whole story
 is about not doing.
3. **A third surface kind** beside terminal and document — honest, and much more
 work: it needs its own context menu, its own switch in the titlebar, its own
 place in the presentation policy.
4. **Leave it.** Defensible while yRDP is the only consumer and the operator is
 the only user. It stops being defensible the moment a remote desktop is
 something a customer sees.

**What is NOT in question:** the surface plumbing itself is right — one canonical
session, N viewers, scaled never resized. This is only about what the GUI draws
around it.

Evidence and repro live in this session's yRDP UX pass; the scratch target
recipe (two `*.toml` files, Xvfb + x11vnc, `YRDP_TARGETS_DIR`/`YRDP_STATE_DIR`
pointed at a temp dir) reproduces the whole flow on any host with no guest and
no risk to a live one.
