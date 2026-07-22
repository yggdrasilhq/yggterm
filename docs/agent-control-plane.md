# The Agent Control Plane — surface-generic engine verbs + shadow clients

**Status: SPEC (2026-07-19; eng-reviewed 2026-07-20, four findings folded — see
"Eng-review amendments" at the end). Approved direction, nothing here built yet
except where marked "exists today". This is slice 1 of the shadow/engine
campaign; the user settled the ordering as spec → slices 2→3 → (Phase F.2 splits
wait). It is the normative doc for agent control of libyggterm surfaces.**

**Eng-review outcome (2026-07-20), binding on the sections below:** `do` is the
single click-delivery primitive (the existing synthetic `Pointer`/`Grid` paths
refold onto it — F1); slice 2 is split into a spike-gated **2a → GO/NO-GO → 2b**
because the `isTrusted` injection premise is unproven and both review models
called it the central feasibility gate (F2); a normative **Action & lifecycle
correctness** section is added because handle-staleness, focus-retargeting, and
the reap predicate are non-determinism traps (F3); **read/capture/journal secret
redaction** extends the standing no-secret rule to the new output door (F4).

This doc unifies two threads the user has been steering:

- the **engine-verb layer** — a small set of surface-generic verbs (read / do /
  wait / probe / capture / creds / otp) an agent drives, cheapest-route-first;
- the **shadow interaction model** — how an agent's control coexists with the
  human at the GUI without yanking their viewport, and how an agent's presence
  (cursor, grid) is shown when co-present.

It sits above two adapters it references but does not restate:

- `web-under-glass.md` (this repo) — Phase F under-glass compositing, the reason
  a backgrounded web page stays alive and addressable.
- ychrome `docs/agent-engine.md` — the headless WPE engine (offscreen page farm,
  100s of pages, governor). That is the **web adapter's slice-4 substrate**; the
  verb *semantics* below are the same on it so a script written against the GUI
  plane runs on the farm unchanged.

## The pain (north star)

Every agent screenshot, session-switch, or probe today drives the **one** GUI.
The human's viewport gets yanked — the user's exact words: *"we continuously
conflict, both trying to use the same GUI."* Worse, the only trusted pointer is
the one seat pointer: during the F.1 smokes an agent servo'd it flawlessly into
the user's *other* window while they were relaxing in it. The user: *"That's why
I asked for the shadow session first."*

The dream, in the user's words (2026-07-19):

- Shadow **pointers** spawned by yggterm on surfaces the user does not even see.
- The click **grid** seen by the agent, not the user.
- In an automated region: the agent's cursor and the user's native cursor
  visible **separately**. Multiple agents' cursors.
- Even with the GUI minimized or on another surface: screenshots, grid, cursor
  work — while the user remains undisturbed.

## The taxonomy (the user's names — keep them)

- **user active client control** — the human on the GUI (today's normal).
- **agent[n] shadow client control** — each agent drives its OWN client view of
  the same sessions; its screenshots / switches / probes touch only its shadow.
- **user-and-agent active compositor control** — today's shared-GUI mode, kept
  as an explicit co-presence mode (post-F.1 an agent can draw over pages).

Orchestrator agents and the stale `experimental/automations` branch are example
*consumers* of this plane, never its definition.

## The escalation ladder — THE LAW of the plane

**Path of least resistance, cheapest rung first, always.** This is not a
guideline; it is the ordering every verb implementation and every agent recipe
must obey. The visible-cursor / grid / servo machinery — the whole reason this
campaign exists — is the **most expensive rung**: presence display and
last-resort control, never the default path.

| Rung | Cost | What it is |
|---|---|---|
| 1. **Semantic** | cheapest | CLI/daemon verbs: `read` (forms/tables/readable/links → JSON), targeted `eval`, direct `navigate`, vault `fill`, `otp`. No pixels, no pointer. |
| 2. **Events** | mid | inspector-protocol `wait` (url-match / network-idle / selector); `do` = GTK-injected `isTrusted` clicks/keys on a **selector** (engine resolves the rect). No seat pointer moves. |
| 3. **Pixels** | most | `capture` (screenshot), the agent grid, and — dead last — a servo'd visible cursor. Declared last resort. |

**Cheapest-route logging = site-lore, as shipped.** The discovered cheapest
route per website is recorded in `lore/<domain>.md` (git-shared source of truth,
fleet newest-wins sync); each host rebuilds its own gitignored sqlite index from
the synced markdown. Method entries carry per-date outcome stats so rotted
selectors auto-demote (facebook's rotted once already) and flag re-learning.
Engine `do` verbs replay lore method blocks. A future model's better route for a
site is expected to *replace* the entry — the lore is living, not frozen.

## The decomposition — what needs a full shadow client, and what does not

The single most important finding (discussed with the user, pt10): **most of the
dream does NOT need the full N-client shadow architecture.** The pt9 under-glass
soft stash accidentally built the hard part.

| Capability | Needs | Slice |
|---|---|---|
| Act without moving the seat pointer | `do` verb — GTK-level event injection into the target webview (`isTrusted` true) | 2 |
| Probe / act on a surface the user is not viewing | soft-stashed surfaces stay **alive and attached** under glass; verbs addressed by `--session` reach them | 2 |
| Snapshot a backgrounded / minimized surface | `webkit.snapshot` renders offscreen — works while backgrounded, one-shot stills only | 2 (exists today, needs proof-while-backgrounded) |
| Keep a surface past the background hold | an agent **lease** pinning it | 2 |
| Create a surface the user never sees | headless surface-create | 2 |
| Agent-only click grid | composite the grid into the **returned screenshot** (capture-side overlay), never into the live DOM | 3 |
| Distinct visible agent cursors (co-presence) | under glass, chrome DOM draws over pages → one overlay glyph per agent, driven by engine pointer state — pure GUI | 3 |
| True shadow clients (own viewport / active-session per agent, idle-host farm, video-rate watching, different geometry) | protocol client identity + roles (takeover guard), jar leases, input arbitration; headless WPE farm | 4 (3.0.0) |

The line: **slices 2–3 ride the existing GUI-hosted surfaces** (one GUI, no new
client). Only slice 4 introduces additional view clients and the headless farm.

## The two planes, one vocabulary

The verbs below have identical semantics on both planes; only the substrate
differs. A recipe targets a **surface handle** (`--session <path>` on the GUI
plane, `--page <id>` on the farm) and otherwise reads the same.

- **GUI-hosted plane (slices 2–3).** Verbs are app-control commands, extended
  with surface addressing, executed **through the running GUI** because they
  touch GUI-owned webviews/DOM. This is where the soft-stashed surfaces, the
  grid overlay, and the agent cursors live. Mount: the existing app-control
  channel (`server app …`), not a new socket.
- **Headless farm plane (slice 4).** Verbs are the ychrome engine API on
  `WPEDisplayHeadless`, no GUI at all — the substrate for GUI-minimized /
  GUI-closed / idle-host-farmed automation and true per-agent shadow clients. Design:
  ychrome `docs/agent-engine.md`, mounted under `/engine/*` on the per-host
  ychrome daemon (settled amendment, that doc §3).

A script must be able to move from plane to plane by changing only the handle
flag. That constraint is what forces one verb table, defined once, here.

**Verb ↔ transport mapping (the two must not become two vocabularies).** The
CLI verbs below (`read`/`do`/`wait`/`capture`/`probe`/`fill`/`otp`) are the
stable, user-facing surface. Each maps to a transport verb per plane; the CLI
name is what agents and site-lore method blocks reference. ychrome
`agent-engine.md` §4 already defines the farm-plane HTTP endpoints under older
names — this table is the binding, and that doc's endpoints are renamed to
match (or aliased) so a recipe reads identically on both planes:

| CLI verb | GUI plane (app-control) | Farm plane (engine HTTP) |
|---|---|---|
| `read --as snapshot` | `WebSurfaceEval` + extractor | `POST /dom {mode:snapshot}` |
| `do click/type/key` | new injected-input command (slice 2) | `POST /input` |
| `wait --until …` | new event-wait command (slice 2) | `POST /wait` |
| `capture` | `WebSurfaceScreenshot` | `POST /shot` |
| `probe` | new (slice 2) | `GET /metrics` + `/journal` mine |
| `fill` / `otp` | `WebSurfaceFill` / `WebSurfaceTotp` | `POST /fill` |

*(Whether to rename the farm endpoints or alias them is a small implementation
call for the ychrome adapter; the CLI vocabulary here is fixed.)*

**Honest limits (stated once, do not oversell).** A GUI-hosted verb needs a
running GUI: with the GUI **minimized**, slices 2–3 work (surfaces stay alive
under glass, snapshot renders offscreen); with the GUI **fully closed**, there
is no web engine at all — GUI-less web automation is the headless farm, slice 4.
`capture` via `webkit.snapshot` is one-shot and slow — probe stills, never
video-rate watching. Same-surface co-presence input interleaving is inherent:
the user always preempts.

## The verb layer — surface-generic contract

Verbs are defined against the **libyggterm surface contract**
(`.agents/skills/libyggterm-surfaces/SKILL.md`), so a new app inherits agent
control by implementing the adapter hooks, not by adding verbs. Each verb names
the adapter hook it calls; per-surface behavior is the adapter's, not the
plane's.

### `read` — structured observation (rung 1)

```
read <handle> --as forms|tables|readable|links|snapshot|text|html
  → JSON
```

- **web**: injected extractor → the interactable tree
  `[{role,text,selector,rect,value?}…]` (buttons, links, inputs, selects,
  textareas, `[role]`, `[contenteditable]`); `readable` = article extraction;
  `tables` = row/col JSON. Exists in embryo as `WebSurfaceEval` today.
- **yedit / document surface**: the pane **schema + values** channels — no DOM,
  the structured read is the schema itself.
- **yggterm terminal**: the daemon vt100 screen (source of truth for content).
- **cellulose** (future spreadsheet app): sheet cells as a range → JSON.

`read` never moves a pointer and never mutates. It is the default an agent
reaches for first.

### `do` — trusted action (rung 2)

```
do <handle> click   --selector <css>            # engine resolves rect, scrolls in
do <handle> click   --x <n> --y <n> [--button …]
do <handle> type    --text "…"                  # to the focused element
do <handle> key     --key Enter --mods ctrl
do <handle> scroll  --dy <n> [--x --y]
do <handle> move    --x --y                     # real hover — menus/tooltips
```

- **web**: dispatched through the webview backend's event API so WebKit treats it
  as seat input — focus moves, `:hover` applies, default actions fire,
  `isTrusted` is **true**. This is the whole point: it retires the "synthetic
  clicks over-report, Enter under-delivers" instrument-lying class. **No seat
  pointer is moved** — the injection targets the specific webview, so a
  backgrounded surface is actionable and the user's real cursor is never
  hijacked (the Helium incident cannot recur through this path).
- **yedit / document**: maps to pane action POSTs (the app performs the edit on
  its host and returns a fresh schema).
- **terminal**: `terminal send` (the orchestrator's `do`).
- Selector-addressed clicks are sugar over coordinate clicks: one resolver
  (`getBoundingClientRect` → scroll into view → dispatch real coords), shared by
  `do` and `read`.
- **`do` is the ONE click-delivery primitive (F1, single source of truth).**
  Today `AppControlCommand::Pointer` delivers clicks via
  `document::eval(app_control_pointer_script(…))` — JS-synthetic (`isTrusted`
  false), main-webview only — and `Grid`'s click-through does the same. Once
  `do` lands, `Pointer` and `Grid`'s click-through are refactored to *resolve
  coordinates then call `do`'s injection path*, so exactly one mechanism owns
  "click a surface." The existing grid machinery (`ClickGridParams`, `Show`/
  refine) is not thrown away — it is reused for slice-3's capture-side agent
  grid (which is therefore ~80% built, not net-new).
- **Coordinate space, focus targeting, and freshness are normative** — see the
  "Action & lifecycle correctness" section. In short: coords are CSS px in the
  target webview's document space (the resolver applies page zoom and scroll,
  and rejects cross-frame targets in v1); `do type` resolves and focuses its
  target element rather than trusting ambient focus; every selector dispatch
  re-checks the surface generation so a click queued before a navigation never
  fires against the wrong document.

**Spike risk — the CENTRAL feasibility gate, not a slice detail (F2).** The
premise that a GTK/GDK-synthesized event yields `isTrusted: true` inside a
WebKitGTK WebView **without moving the seat pointer** is unproven in this stack
(the F.-1 keyboard spike used a *real* seat pointer crossing into the page, not
synthesized events). Slice 2a proves-or-kills it before any engine is frozen
around it; the spike also evaluates WebKitGTK's built-in **automation/WebDriver
input dispatch** (a Layer-1 path) against raw GDK synthesis. Delivery into an
**unmapped/minimized** webview is a second, narrower question — a demoted-but-
attached surface is expected to accept events; a fully unmapped one may need a
transient off-screen map, or `do` on it defers to the farm plane. On NO-GO,
`do` moves to the farm plane (slice 4) and 2b still ships read/wait/capture/
lease without trusted `do`.

### `wait` — event-driven synchronization (rung 2)

```
wait <handle> --until load:committed|load:finished
                     |idle:<ms>|selector:<css>[:visible]|js:<expr>
              --timeout <ms>
  → {met:true, elapsed_ms} | {met:false, reason}
```

The engine does the polling (100 ms cadence for `js`, **per surface**, not a
global tick), so scripts never hand-roll a screenshot-poll loop again. This is
the verb that kills the "screenshot until it looks done" anti-pattern the
current instruments force. **Mechanism honesty:** `load:*` rides WebKit's
`load-changed` signal (already feeding `page_state`); `idle:<ms>` is an
in-process quiescence heuristic (no DOM mutations / no pending `load` for the
window), NOT DevTools network-idle — true network-idle waterfall is a `probe`
capability on the inspector protocol (a later slice), and this verb must not
claim it in v1.

### `probe` — the dtrace layer (rung 1, read-only)

```
probe <handle> --what net|console|exceptions|paint|timing|metrics
  → JSON
```

Network waterfall, console lines, uncaught exceptions, paint/timing, per-surface
resource metrics. Designed in, not bolted on: every probe answer is journaled.

### `capture` — pixels, last resort (rung 3)

```
capture <handle> --mode viewport|full [--grid] [--out <path>]
  → PNG (+ JSON manifest when --grid)
```

- **web**: `webkit.snapshot(FullDocument)` — faithful for laid-out DOM, works
  while the surface is backgrounded/minimized (offscreen render, not a
  compositor grab). Exists today as `WebSurfaceScreenshot`. One-shot, slow —
  probe stills, not video. **Freshness contract (do not overclaim):** the
  snapshot reflects the last committed paint; canvas/video/async-painted
  regions and a truly throttled backgrounded surface can lag. `capture` returns
  a `{captured_ms, page_state, throttled}` manifest so a caller can tell a
  fresh frame from a stale one; the slice-2a spike measures backgrounded-
  snapshot staleness and, if needed, briefly promotes-under-lease → captures →
  demotes (risk table).
- `--grid` composites the click grid **into the returned image only** (see
  Agent presence), never into the live DOM.
- terminal: the faithful xterm composite (`capture_backend=xterm_canvas_composite`).

### `fill` / `otp` — identity (rung 1)

```
fill <handle> [--entry <name>] [--user <u>]     # vault autofill, host-exact
otp  <handle> [--entry …] [--user …]            # vault TOTP → one-time field
```

`fill`/`otp` exist today (`WebSurfaceFill`/`WebSurfaceTotp`): the GUI resolves
the page's real origin from the engine (the page cannot lie about it), matches
the vault host-exact, injects. Key material never leaves the GUI host except
into the matching page. **`otp` v1 = vault-TOTP only** (what exists); the
layered strategy (vault-TOTP → SMS-forwarder feed → email-OTP, one verb trying
the ladder) is a separate campaign item and is *not* scoped here — do not
overpromise it in v1. The **passkey user-presence invariant is unchanged**: an
agent can trigger a ceremony but never self-approve; the human gate is the
presence dialog.

## Addressing, leases, and headless create (slice 2)

- **Surface addressing already exists for the read/identity verbs.** The
  `WebSurfaceEval`/`WebSurfaceScreenshot`/`WebSurfaceFill`/`WebSurfaceTotp`
  app-control commands already carry `session_path: Option<String>` (None =
  active), and `WEB_SURFACE_NATIVE_IDS` maps `(session_path, tab_id) →
  native_id`, republished at every reconcile-tick exit. Under the soft stash a
  backgrounded surface stays in the applied map (demoted, not removed), so its
  native id stays in that registry and `resolve_live_web_surface(session)`
  resolves it — **a backgrounded surface is reachable by `--session` today**;
  the pt9 stash is what makes shadow-probing free. So slice 2's read/capture/
  identity work is mostly *proving* the existing addressing reaches
  backgrounded surfaces + the structured `read --as` extractor, not new
  addressing. The stale code comment "missing entry = backgrounded" predates
  the stash and must be corrected: missing now means *closed or hold-expired*.
- **A handle is not durable as `(session_path, native_id)` alone.** The
  reconciler can destroy and recreate a surface (hold expiry, tab swap, profile
  change) and reuse a native id, so a command or lease queued against a bare
  pair can act on the wrong surface. The durable handle is
  `(session_path, tab_id, generation)` — the generation bumps on every
  recreate; every addressed verb carries it and fails closed with
  `stale_handle` when it no longer matches (see Action & lifecycle).
- **Lease has one owner.** A lease rides the `AppliedWebSurface` entry the
  reconciler already owns (it is where stash timing lives) — a lease is just a
  later reap deadline on the same struct, never a second registry that could
  disagree with the applied map. **Reap predicate (normative):** a stashed
  surface is reaped iff `now ≥ max(stashed_at + background_hold, lease_until)`
  — the lease strictly extends, never shortens, the hold.
- **Agent lease.** `lease <handle> --ttl <s>` pins a surface past the
  `background_hold` (default 600 s) so a long shadow run is not reaped
  mid-flight. The lease is journaled; it never triggers takeover (see below); it
  expires so a dead agent leaks nothing. Multiple agents may co-lease one
  surface (read/do interleave; the human always preempts). A lease taken while
  the surface is still foreground survives the later foreground→stash transition
  (it is written to the entry, not to the stash timer).
- **Headless surface-create.** `open --session <path> --url … --headless` mounts
  a surface that is created but never revealed — the reconciler places it
  demoted with a lease and no page hole. Dream §2: OSC surface-create must not
  defer while the window is backgrounded. On the farm plane this is just
  `/open`.

## Action & lifecycle correctness (slice 2b — normative, F3)

The dangerous edges of "act on a surface the user is not watching" are lifecycle
and input-targeting races, not the verbs themselves. This repo's rules forbid
non-determinism; these are the invariants slice 2b must hold, each with an
acceptance test. This whole section is gated behind the 2a GO — if the
`isTrusted` spike returns NO-GO, `do`/type/key move to the farm plane and only
the read/capture/lease invariants below apply.

**Lifecycle**

- **Durable handle = `(session_path, tab_id, generation)`.** The generation is a
  monotonic counter bumped by the reconciler on every webview recreate for that
  `(session_path, tab_id)`. It is published alongside the native id in
  `WEB_SURFACE_NATIVE_IDS`. Every addressed verb carries the generation it was
  issued against.
- **Fail closed on stale handle.** A verb whose generation no longer matches the
  live entry returns `{error: "stale_handle", current_generation}` and performs
  no action — never a best-effort dispatch against whatever now occupies the id.
- **Reap predicate:** `reap iff now ≥ max(stashed_at + background_hold,
  lease_until)` (repeated from the lease section because it is the invariant a
  test pins).
- **Cancellation on recreate/close.** When the reconciler destroys or recreates
  a surface, every queued/in-flight verb for the old generation is cancelled
  with `stale_handle` and journaled — nothing silently re-binds.
- **GTK-main-thread ownership.** All surface mutation (event injection, lease
  writes, generation bumps) happens on the GTK main thread via the existing
  app-control dispatch; app-control worker threads only enqueue. This is the one
  writer discipline the reconciler already uses — no second writer.

**Input targeting**

- **Coordinate space is defined.** `--x --y` are CSS pixels in the target
  webview's *document* space; the resolver applies the surface's current page
  zoom and scroll offset before dispatch. Under-glass hole geometry and covers
  are irrelevant (injection targets the widget, not the seat). v1 rejects
  cross-frame / nested-iframe targets with `unsupported_frame` rather than
  guessing.
- **`do type` resolves and focuses its target.** It does not trust ambient
  focus (global mutable state a redirect, dialog, or human keystroke can move).
  It focuses the resolved element, verifies focus landed, then dispatches keys;
  if focus cannot be established it returns `focus_failed`, not a blind type.
- **Selector freshness guard.** Between resolve and dispatch the engine
  re-checks the surface generation and (for selectors) that the element still
  matches at the resolved rect; a navigation or reflow in the gap aborts with
  `target_moved` instead of clicking whatever now sits there.

**Concurrency and preemption**

- **Per-surface FIFO, one in flight.** Concurrent `do`s on one surface serialize
  by arrival; deterministic, no timing-dependent interleave. Both journaled.
- **Human preemption is defined, not aspirational.** Real seat input to a
  surface (focus-in from the user, a keystroke, a pointer button) sets a
  preempt flag on that surface: the in-flight agent verb is allowed to finish
  its single atomic dispatch, the rest of that agent's queued batch for the
  surface is cancelled with `preempted`, and the cancellation is journaled. The
  human is never queued behind the agent. Detection rides the existing
  focus/input signals the shell already tracks; it is not another queue
  participant.

## Agent presence — grid and cursors (slice 3, the most expensive rung)

Everything here is **pure GUI DOM** under glass (chrome draws over pages), and
every piece is opt-in / co-presence-only. None of it is on the default path.

- **Agent-only click grid. ✅ SHIPPED + live-proven (2026-07-21).**
  `server app screenshot --grid [COLSxROWS] [--grid-refine CELL]` composites the
  grid into the returned PNG and returns a cell manifest at
  `data.post_process.grid`; the page is never touched. Geometry moved to ONE
  owner — `yggterm_core::click_grid::GridGeometry` — shared with the live DOM
  grid, so the two cannot disagree about where `B7` is. The manifest reports
  **both** coordinate spaces (`capture` = clickable, `image` = what the eye
  reads) plus `capture_size`, so a HiDPI host cannot silently mis-aim.
  **Acceptance gate 5 passed live on the desktop host:** a plain screenshot
  taken immediately after a `--grid` capture shows no grid at all. Composes with
  `--crop`/`--region`/`--scale`. Details: `docs/yggui-click-grid.md`.
- **Visible distinct agent cursors — cursor v1. ✅ SHIPPED + live-proven
  (2026-07-21).** Agent identity rides the app-control request (`--agent <id>`
  or `$YGGTERM_AGENT`); every pointer verb (`pointer move/press/click/drag`,
  `grid click/hover`) publishes that agent's position into
  `yggterm_core::agent_presence`, and the GUI draws a coloured arrow tagged
  `agent-N` for agents working the session the user is viewing. Identity→index→
  colour is assigned once per window and is stable for its life. Presence is
  readable at `app state` → `agent_presence` (`visible` = what the user can see
  now; `live` = every agent inside its TTL, whatever the user is viewing).
  Pointers expire after `AGENT_CURSOR_TTL_MS` (8 s) via a **one-shot** CSS fade
  — never a repeating animation, which would re-introduce the per-element blink
  cost that halving idle GUI CPU removed.

  ⚠ **Verifying it needs `--backend os`.** The default capture composites the
  xterm canvas OVER a DOM snapshot, so an agent cursor that overlaps the
  terminal viewport is missing from the frame *even though the user sees it*.
  This cost a real debugging detour: the first proof frame looked empty while
  the overlay was painting correctly. Probe with the compositor backend, or
  place the cursor over the sidebar where the composite is not blind.

  The settled rule, and all v1 does:
  - no co-presence on/off toggle (explicitly parked — *"We both do not know what
    co-presence on means"*);
  - no ghost-cursor mimicry, no visibility modes;
  - multiple agents = multiple overlay glyphs, distinct colors + tags.

  This is *"user-and-agent active compositor control"* made literal and minimal.

## Where it mounts

- **GUI plane:** extend the existing app-control command set (the `AppControlCommand`
  enum, `server app …`) with the addressed verbs — no new socket, token, or
  lifecycle. `is_read_only()` already classifies pure-observation verbs so they
  skip the shell re-render; `read`/`probe`/`capture`/`wait` join that set.
- **Farm plane:** the ychrome daemon's `/engine/*` subsystem (ychrome
  `agent-engine.md` §3 amendment) — shared journal, session registry doubles as
  the promote-to-visible target list. One daemon per host per user.

The two mounts are deliberate: GUI-owned webviews can only be driven from inside
the GUI process; the headless farm has no GUI to route through. The verb
*surface* is identical; the *transport* is whichever owns the substrate.

## Slices (execution order — F.2 splits wait until 2–3 land)

1. **SPEC (this doc)** → plan-eng-review → fold. *Deliverable of this session
   (done 2026-07-20; four findings folded).*
2. **Engine core — spike-gated (F2).** Split so the unproven primitive cannot
   sink the cheap wins or freeze an engine around a hole:
   - **2a — PROOF, then STOP (short, next session).** Two independent proofs on
     the live host: (i) `--session`-addressed `read` + `capture` against a
     **stashed/backgrounded** surface actually return that surface's content
     (the addressing already resolves it — this proves it end-to-end, incl. the
     stale-error-string fix and backgrounded-snapshot freshness); (ii) whether
     `isTrusted`-true input injection into a target WebView is possible **at
     all** without moving the seat pointer. **✅ done 2026-07-20 = GO** (GDK
     event → `WidgetExt::event` yields trusted input, no seat move;
     `docs/spikes/slice2a-istrusted-inject`); the read/capture-on-stashed live
     proof is still owed on a clean sandbox. **GO/NO-GO gate → GO.**
   - **2b — ENGINE, only on GO (1–2 sessions).** `do` (single injection
     primitive, F1) + the Action & lifecycle correctness invariants (F3) +
     `wait` + agent lease + headless create + the addressed command protocol.
     **On NO-GO:** `do`/type/key defer to the farm plane (slice 4); 2b still
     ships `read`/`wait`/`capture`/`lease` without trusted `do`, and the "act
     without the seat on the GUI plane" promise is honestly deferred.
     **2b progress (2026-07-20): the `do` injection PRIMITIVE is BUILT** —
     `AppControlCommand::WebSurfaceDo` (verbs `click`/`click_selector`/`move`/
     `scroll`/`type`/`key`), the vendored `WebSurfaceHost::inject_*` GDK-synth
     path (the proven slice-2a recipe, ported next to the existing `eval`/
     `snapshot` host methods) with a `surface_not_mapped` fail-closed guard, the
     `desktop.inject_web_surface_*` delegates, the shell `web_surface_do_for`
     handler (selector→rect resolution + `elementFromPoint` freshness guard +
     document→viewport→widget coord mapping, zoom applied next to the webview),
     and the `server app web do …` CLI. Compiles across the workspace; unit
     tests (keyval/mods/button mapping) + a `WebSurfaceDo` serde round-trip are
     green. **Owed:** the live proof (gate 2 trusted-click differential + part-(i)
     `--session` on a real soft-stashed surface) on an uncrowded host — dev is a
     crowded multi-daemon home with another agent's GUI, so no deploy/competing
     GUI was stood up. Still to build in 2b: the F3 generation-handle staleness
     machinery, `read`/`wait`/`lease`/`headless`, and keyboard-injection live
     proof (`click` is proven; `type`/`key` synthesis is best-effort until a
     live character-insertion check passes).
3. **Agent presence — ✅ DONE (2026-07-21).** Capture-side grid (gate 5) and
   cursor v1 (gate 6) both shipped and live-proven; see Agent presence above.
4. **3.0.0 — true shadow clients + idle-host farm.** Protocol client identity + roles
   (takeover guard), jar leases, input arbitration; headless WPE farm (ychrome
   agent-engine.md Phases A–E). The headless-sway recipe proven this campaign is
   the pattern. **Decomposed into deployable sub-slices 4.0–4.4 in the build plan
   below** — ordered by risk, only 4.0 bumps the protocol.

## Slice 4 build plan — true shadow clients + idle-host farm (3.0.0)

Slices 2–3 gave the agent everything the *soft-stashed GUI surfaces* can give:
read/do/wait/capture/lease on a surface the user is not watching, an agent-only
grid, per-agent cursors. Slice 4 is the **residual the GUI-hosted plane cannot
reach**, and only that residual:

- a **per-agent viewport** whose active session and geometry are independent of
  the user's GUI, so a probe never yanks the user's view (the "whooping"
  annoyance, pt10), and
- **GUI-closed / idle-host web automation** via a headless WPE farm on an idle
  box (oc, idle by design).

The user settled the shape (pt10): the engine verbs already cover most of the
win, so slice 4 adds only the two things above. It is the **riskiest work in the
repo** — it changes the daemon protocol and touches the session-ownership path
that both the 2.11.5 dead-sessions class and the row-dropping handoff live in —
so it is decomposed into sub-slices **ordered by risk, each independently
deployable**, and only the first bumps the protocol.

### Two identity layers already exist; slice 4 adds a third — do NOT conflate them

Reusing either existing identity for the takeover guard would be the
second-encoding mistake this repo forbids.

| Layer | Where (verified in tree) | Lifetime | Answers |
|---|---|---|---|
| **App-control agent identity** (slice 3) | `AGENT_IDENTITY_OVERRIDE: RwLock<Option<String>>` in `app_control.rs`, set once from `--agent` | process-global, one invocation | which colour/tag the agent's cursor draws |
| **Runtime owner** (exists today) | `PreservedTerminalOwner` / `owner_server_pid` / the hot-update-owners file in `daemon.rs` | one daemon per PTY, survives hot-restart | who holds the PTY master fd — a hot-restart successor *takes over* this |
| **View-client identity + role** (slice 4, NEW) | per-connection on `ServerRequest`, see 4.0 | one live client connection | is this client Active or Shadow — gates takeover + input |

The app-control field is process-global and per-invocation: right for "one CLI
call is one agent," wrong for a long-lived daemon serving many connections. The
runtime-owner is about the *PTY*, not the *viewer*. The new layer answers only
"may this client take runtime ownership / drive input," and lives on the
connection — not on `AGENT_IDENTITY_OVERRIDE`, not on a new global.

### 4.0 — Client identity + role in the daemon protocol (foundation; the one protocol bump)

Everything after this is additive on the handshake it introduces.

- **Encoding (eng-review D4).** Identity/role rides as an `Option<ClientIdentity>`
  field with `#[serde(default)]` on the existing attach/connect request(s) —
  **never a new `ServerRequest` variant**. `ServerRequest` is
  `#[serde(tag="kind")]` with no `deny_unknown_fields`, so an unknown *variant*
  hard-fails an older chain daemon (the row-drop path) while an unknown *field* is
  safely ignored. `client_role ∈ {Active, Shadow}`; `client_id` is
  **server-issued per connection**, not a client assertion, and role/identity are
  **frozen after registration** (no mid-connection role change). Anonymous /
  pre-4.0 connections default to `Active`, so the user's GUI is unchanged and the
  26-row handoff check still passes. Both-direction round-trip tests are required
  (old-client→new-daemon = None = Active; new-client→old-daemon = field ignored).
- **Fail closed, never silently downgrade (eng-review D7).** A silently-ignored
  role is a privilege *escalation*, not a safe default: a Shadow whose role an old
  daemon drops would be treated as Active exactly during the mixed-version window
  when the guard matters most. So the daemon **advertises role-enforcement** (in
  the `Status`/handshake reply) and a `Shadow` client **refuses to operate**
  against a daemon that cannot enforce it. Active/anonymous clients are unaffected.
- **SSOT:** the role lives in one place, the per-client connection/session-handle
  the daemon already keeps. It is *not* derived from `AGENT_IDENTITY_OVERRIDE`
  and *not* a new global.
- **Takeover guard = default-deny allowlist (eng-review D3).** A `Shadow` may
  issue **only** an enumerated read/observe set (`Status`, `Snapshot`,
  `WorkingFlags`, `TerminalRead`, `TerminalSnapshot`, row/state queries); **every
  other `ServerRequest` variant is refused** with `shadow_cannot_own`, journaled.
  Expressed as an **exhaustive `match` with no wildcard arm**, so a newly-added
  variant fails the build until its access is classified and the default is
  refuse — the ownership boundary cannot develop a silent hole. A Shadow never
  writes the hot-update owners file and never becomes `owner_server_pid`.
- **Hot-restart provenance (eng-review D9).** The hot-restart handoff must carry
  role/epoch provenance **or** force re-registration — a successor connection with
  no role must **not** default to Active. Test a mixed **N / N-1 / N-2** parent
  chain, not just one restart.
- **Protocol version.** `SERVER_PROTOCOL_VERSION` is the crate version, so the
  envelope change stamps a real bump → a **daemon** deploy to the live host (the
  row-dropping-handoff path). The only sub-slice needing a coordinated deploy
  window; 4.1–4.4 ride the already-deployed handshake.

Acceptance gates 10, 15.

### 4.1 — Input arbitration across clients (the human-always-preempts half)

- Extend the **existing** per-surface human-preempt (slice-2b, one arbiter) to a
  **per-session input lease** keyed by `(client_id, role)`. The Active client
  (the human) holds standing priority; a Shadow may drive input to a session only
  while it holds a transient input lease **and** the Active client is not focused
  on that session. Real seat input from the Active client revokes the lease
  immediately, cancels the Shadow's queued batch (`preempted`, journaled), and is
  never queued behind the agent — the exact machinery slice-2b already defines
  for one surface, now keyed across clients.
- **SSOT:** one arbiter, the slice-2b preempt path extended — not a second lease
  table.
- **Arbitrate semantic transactions, not bytes (eng-review D9).** The lease is
  keyed on a stable `client_id` + connection epoch with fencing; input is
  arbitrated as **atomic semantic transactions** (bracketed paste, mouse drag,
  modifier+key sequences), never split mid-sequence into byte chunks. On
  disconnect/restart, queued and pending input for that client is cancelled, not
  replayed.

**★ TRANSPORT FORK — a build-time finding that needs the user's call (2026-07-22).**
Grounding the "extend the one arbiter across clients" language against the tree
surfaces a fork the spec did not settle, and it decides whether 4.1 bumps the
protocol:
- **A Shadow's *only* drivable input is web-surface input.** Terminal input is
  role-gated away entirely — `role_gate` denies `TerminalWrite` to a Shadow (gate
  10) — so the "keystroke" in gate 11 can only be the Active client's *own* seat
  input, and the input a Shadow drives is clicks/typing into an *under-glass web
  surface*, never PTY bytes.
- **Web-surface input is per-GUI-process and never reaches the daemon.** The
  gate-9 arbiter (`AgentInputArbiter`) is a **process-global** in the GUI, and a
  Shadow is a **separate GUI process** with its own webview + its own arbiter. So
  "one arbiter extended across clients" cannot be literally one shell-side arbiter
  — the two clients share no shell state. The only cross-process meeting point is
  the daemon, which does not see web injections.
- **Therefore two coherent shapes, and they differ on the protocol:**
  - **(A) Subsume into the 4.2 profile write-lock.** Whoever holds the profile
    write-lock is the sole web-surface writer; 4.1's "input lease" becomes
    *Active-priority preemption* of that lock (the Active client can reclaim it
    from a Shadow on seat input). Extends 4.2's already-deployed lock rather than
    adding a second lease table — matches "one arbiter, rides the existing
    handshake" — **but** 4.2 grants for process-lifetime with no preemptive
    revoke, so adding revoke likely needs a small protocol addition
    (`PreemptProfileWriteLock` or a flag) → a bump + daemon deploy after all.
  - **(B) New daemon-side transient input lease** keyed by `(client_id, role)`,
    Active-priority, revoke-on-seat-input. Cleanest conceptually, but new
    `ServerRequest` variants → `protocol_shape_stamp` bump → a live-host **daemon**
    deploy (the row-dropping-handoff path), which the doc's "4.1 rides the existing
    handshake" line was trying to avoid.
- **★ DECIDED: shape (A) (user, 2026-07-22).** The profile write-lock IS the
  input authority for a web surface (two `WebContext`s on one profile corrupt it,
  so writers are already mutually exclusive), so 4.1 = make the 4.2 write-lock
  Active-preemptible. Grounding it removed the protocol-bump worry entirely: the
  role rides the **already-deployed 4.0 envelope**, so Active-priority contention
  is a daemon-logic change with **no `ServerRequest`/`Response` shape change** (no
  `protocol_shape_stamp` bump, no version fight). Sub-slices:
  - **4.1a ✅ LANDED (`3fe9662`, main).** `ProfileWriteLockHolder.role`, new
    `AcquireOutcome::PreemptedShadow` (Active takes over a live Shadow holder;
    every peer-vs-peer contention stays `profile_busy`), role sourced from the
    envelope in the acquire/release handlers. Unit-tested (14 write-lock tests).
    Inert in production until 4.1b/c wire it.
  - **4.1b ✅ LANDED (main, this session).** The GUI's native web-surface
    reconciler (`web_surface_native_reconcile_loop` in `shell.rs`) acquires the
    profile write-lock the first time a profile's persistent surface opens and
    releases it (end-of-tick) when that profile's last surface is gone — one lock
    per jar, not per surface, reference-tracked against the reconciler's `applied`
    set (a merely-*stashed* surface keeps its entry, hence its lock). On `Busy`
    (a peer holds the lock) the surface opens **read-only** — an ephemeral, no-jar
    `WebContext` (`profile_dir: None`) — never a second writer. Because an Active
    GUI preempts a Shadow holder (4.1a), this read-only path only bites
    Active-vs-Active (a rare second GUI on one profile) and a Shadow writing a
    profile the user holds. **Role-sourcing fix (SSOT):** the acquire/release
    client helpers now stamp the calling *process's* slice-4.0 identity
    (`send_request` → `current_client_identity()`) instead of the 4.2-era hardcoded
    `Active`, so a shadow view client acquires as `Shadow` (preemptible) and the
    user's GUI as `Active` (preempts) — the same wire path gate 15 already
    live-proves. Pure lock-leak guard `web_profile_write_locks_to_release`
    (+2 unit tests); workspace green. **Owed:** the coordinated daemon+GUI deploy
    to jojo for live proof (needs 4.1a on jojo's daemon, which predates it), best
    landed together with 4.1c.
  - **4.1c ✅ LANDED (main, this session).** The Shadow's `do` chokepoint
    (`web_surface_do_for`, shell.rs) now checks `ProfileWriteLockReport` (already
    Allow-for-Shadow) before injecting: if the report no longer names this shadow
    as the holder of the surface's active-tab profile — because an Active client
    preempted it (4.1a) — the verb is refused `preempted` and the batch cancelled
    through the SHARED preempt primitive `AgentInputArbiter::preempt_surface` (the
    same one seat input uses; `note_human_input` is now a named alias for that
    cause — one table, not two). So the shadow's later verbs are refused by the
    gate-9 `admit` path until it re-observes and, if it regains the lock, starts a
    fresh batch. **Only Shadows pay the round-trip** (an Active client holds the
    lock by construction, or drives its own ephemeral read-only surface from 4.1b);
    ephemeral profiles are exempt (own in-memory context, no shared jar); and it
    **fails CLOSED** — a shadow that cannot reach the daemon to confirm refuses,
    matching the shadow doctrine (D7). This closes the residual two-writer window
    4.1b left: a shadow whose jar-backed context outlives its preemption can no
    longer take *agent-driven* writes to the jar. (Residual, documented: the
    preempted shadow's live WebContext can still emit background/JS writes until
    its surface closes — a heavier destroy-on-loss mitigation is out of 4.1c's
    scope.) Pure `write_lock_report_holds` (+ unit test) + the arbiter
    shared-primitive test; workspace green.
  - **★ 4.1 gate correction (main, this session) — the sandbox wire proof caught
    a bug unit tests could not.** The 4.0 `role_gate` DENIED a Shadow's
    `AcquireProfileWriteLock`/`ReleaseProfileWriteLock` (`shadow_cannot_own`),
    treating the write-lock as ownership. But 4.1a's whole `PreemptedShadow`
    branch REQUIRES a Shadow to hold the lock first (its own
    `active_preempts_a_live_shadow_holder` test acquires as Shadow), so with the
    gate denying acquire that branch was **dead code** and a shadow could never
    co-browse. Fixed: the write-lock is a PREEMPTIBLE coordination lease (an
    Active client preempts it instantly), NOT terminal ownership — so acquire/
    release are now Allow-for-Shadow, while non-preemptible PTY ownership
    (TerminalWrite/Resize/Restart/FocusLive) stays denied. The unit tests that
    asserted Deny were flipped; the 4.1b shadow `Err` fallback is now role-aware
    (an Active GUI opens jar-backed on a daemon error, a Shadow FAILS CLOSED to
    read-only — never writes a jar it could not confirm it holds).
  - **★ Wire proof shipped + PASSING (this session).** New hidden CLI verb
    `yggterm server write-lock <acquire|hold|report|release> [--profile <name>]`
    drives the daemon lock directly (identity from `--client-role`/`--client-id`,
    which — same trap as `--agent` — must come AFTER the subcommand). `hold`
    parks holding the lock so a live holder can be contended. Proven against an
    isolated sandbox daemon: a live Shadow holder of `work` is **preempted** by an
    Active acquire (`preempted_shadow`, writable); a Shadow never preempts a peer
    Shadow, and an Active never preempts a peer Active (both `profile_busy`). This
    verb also unblocks the gate-11/16 live proofs.
  **4.1b/4.1c LIVE proof still owed** on jojo — a coordinated daemon+GUI deploy
  (jojo's 2.12.1 daemon predates 4.1a AND this gate fix, so PreemptedShadow is not
  live there until the swap).
- The transport-independent *core* — client-keyed batches with Active-priority
  preemption — is pure logic that both shapes reuse and is what
  `AgentBatch::client_id` already seats. It is unit-testable with no daemon and no
  deploy; only its **wiring** waits on the fork above.

Acceptance gate 11.

### 4.2 — Profile write-lock (single-writer, profile-safety)

- **Renamed from "jar lease" (eng-review D5) and widened (D9).** Distinct from the
  slice-2b *surface lease* (reconciler-owned, keep-alive): this is a
  **daemon-owned single-writer lock across clients** — two owners at two layers
  that never share state. It covers the **whole mutable profile context**, not
  just the cookie jar: SQLite WAL/SHM, IndexedDB, service workers, caches,
  downloads. Two `WebContext`s on one profile corrupt it, so the daemon grants the
  write-lock to exactly **one** live web client at a time for its process
  lifetime; a second writer gets `profile_busy` (or an explicit read-only mirror).
  Define crash/restart recovery and lock ordering against the surface lease. One
  owner = a profile-write-lock table on the daemon keyed by profile; releasing
  frees it for the next client.

Acceptance gate 12.

### 4.3 — The shadow view client (headless-compositor recipe)

- A second yggterm **view** client process attaches to the same daemon with
  `client_role = Shadow`, under a headless compositor (`WLR_BACKENDS=headless`
  sway — proven this campaign; the dev sandbox was the pattern). It has its **own**
  active session and geometry, independent of the user's GUI.
- Addressed by `--client <name>` on the app-control verbs so a probe names its
  target client; its screenshots/switches touch only its shadow view (captured
  via `grim`). The user's GUI is never driven — the pt10 "whooping" annoyance is
  gone.
  **✅ `--client <name>` ROUTING BUILT + verified (2026-07-22).** No protocol
  change: app-control already targets a worker by pid (`AppControlRequest.preferred_pid`,
  `app_control_requests_pending_for_worker`), so `--client` is a resolve-name→pid
  layer, GUI-only deploy. Each GUI worker now records its slice-4 `client_id`
  (`current_client_identity().client_id`) in its `ClientInstanceRecord`, and the
  one resolver `choose_app_control_pid` gained a `requested_client` arg
  (`YGGTERM_APP_CONTROL_CLIENT`, set from `--client`): `--pid` wins if both given;
  else the sole worker whose `client_id` matches; two workers under one name fail
  loudly rather than guess. Wired into both CLI entry points (`yggterm` +
  `yggterm-headless`) and the `--help`; `server app clients` surfaces `client_id`
  so names are discoverable. Proven end-to-end through the real binary on a dev
  sandbox: `clients` lists the id; an unknown `--client` errors with the available
  list; a known `--client` resolves to the pid and enqueues a *targeted* request.
  7 resolver unit tests + all touched crate tests green.
  **✅ LIVE-PROVEN with a real shadow GUI (2026-07-22).** An isolated 2.12.1
  sandbox daemon (`role_enforcement:true`) + a shadow launched via
  `scripts/shadow-client.sh start --name shadow-demo` (headless sway; it survived
  the fail-closed role gate = attached as Shadow). `server app clients` listed
  `client_id=shadow-demo`; **`server app state --client shadow-demo` returned
  `handled_by_pid` = the shadow's own pid with real state** — the verb *claimed*
  by the named shadow, not the default worker; the negative control `--client
  bogus-name` was refused (name-gated). A grim capture showed the shadow's own
  full viewport. This is the recipe the gate-11 / gate-16 live proofs now build on
  (both need to aim a verb at a specific view client).
- Both clients read the **same daemon truth** (Phase-2 doctrine: daemon = truth,
  view client = disposable); the shadow adds a view, never a second source of
  state.
- **Read-only geometry (eng-review D8).** A Shadow **never** drives PTY winsize,
  terminal focus, or scroll. A differently-sized shadow view issuing `SIGWINCH`
  would reflow the CLI and scramble the user's live frame (the known
  frame-corruption class) *without* claiming ownership, so the takeover guard
  alone does not cover it. The shadow view uses a **fixed/canonical grid**
  (letterboxing any size difference) or a **read-only replay** of the active grid.
- **On-demand lifecycle (eng-review D6).** A shadow client is spawned when an
  agent first drives it and **reaped after an idle TTL** (no verb in N minutes →
  torn down; the next verb re-spawns), reusing the slice-2b reap-deadline
  discipline. The full compositor + WebKit stack (100s of MB) is paid only during
  active work, never by idle shadows.
- **User geometry changes are first-class, never an agent alarm (eng-review
  D11 — user-raised).** The inverse of the read-only-geometry guard: the Active
  client (the user) is the SOLE driver of a session's canonical geometry, and
  changing it — docking a sidebar, resizing the window — must not break agents
  mid-flight. Three things make it safe: (1) the shipped auto-hide sidebars are
  **overlays** (hover-reveal over the viewport, `spec-sidebar-auto-hide-hover-overlay`),
  so the common "open a sidebar" case reflows **nothing** — the PTY winsize is
  untouched; only a docked/explicit resize drives `SIGWINCH`. (2) When a real
  resize does happen it reflows the ONE canonical grid **once** (the shadow
  follows it, never fights it — D8), and any agent web verb in flight against the
  old geometry **fails closed** via the existing slice-2b generation-handle /
  selector-freshness guard (`stale_handle` / `target_moved`), so the agent
  re-observes and retries instead of clicking a stale coordinate. (3) The change
  is journaled as an active-client geometry event, so an agent sees "geometry
  changed by the user" and simply re-captures — a changed frame is a normal
  reflow, not corruption to panic over. Geometry authority is single-owner and
  user-priority; the agent absorbs it, it never absorbs the agent.

Acceptance gates 13, 19.

### 4.4 — The idle-host farm plane (oc)

- Mount the ychrome headless WPE engine (`agent-engine.md`, `WPEDisplayHeadless`)
  under `/engine/*` on the per-host ychrome daemon, so **GUI-closed** web
  automation runs the same verb vocabulary with `--page <id>` instead of
  `--session <path>`. oc (idle by design) is the natural farm; fleet routing
  already exists.
- **The binding is the verb table above** (§ The two planes): a recipe moves
  GUI-plane → farm-plane by changing only the handle flag. The farm endpoints are
  renamed/aliased to the CLI verbs so a site-lore method block reads identically
  on both planes.
- **Host-local profiles (eng-review D9).** Profiles are explicitly **host-local
  and non-shareable** — a local write-lock does not fence a profile copied or
  synced across hosts, so "same profile, two hosts" is forbidden rather than
  distributed-locked. Cross-host automation runs against the target host's own
  profile.
- **`--page` is a capability, not an address (eng-review D9).** A farm page
  reference is an **opaque, unguessable capability** scoped to `{host identity,
  engine epoch, profile grant, caller identity, expiry}`. A stale, forged,
  post-reboot, or cross-host `--page` is rejected — it must not resurrect a page,
  reach another host's page, or expose its cookies.
- **4.4 trust + placement sub-spec — DECIDED by user 2026-07-22** (the eng-review
  D9 gate; single-user fleet, so "idle host" resolves to "one of the user's own
  hosts, driven by the user's own agents"):
  - **Credentials/profiles: real host-local profiles, UNGATED.** The farm uses a
    host's real per-host profile exactly like the GUI — no separate farm profile,
    no per-run credential grant. Justified: every host and credential is the
    single user's own; there is no second tenant to fence out. (I advised the
    gated variant; the user chose ungated deliberately.) Profiles stay
    **host-local + non-shareable** (no "same profile, two hosts").
  - **Scheduling: EXPLICIT-ONLY.** A page is created only when a verb NAMES the
    target host. No automatic idle-host placement — automation never lands on a
    host the caller did not name.
  - **`--page` stays an opaque, unguessable capability** scoped to `{host
    identity, engine epoch, expiry}` — the one protection kept regardless of the
    ungated-profile choice: a stale/forged/post-reboot/cross-host `--page` is
    rejected so it can never resurrect a dead page or reach another host's page.
  - Still owed as ENGINEERING defaults (not user decisions): host-becomes-busy
    preemption, TTL/timeout-kill, orphan cleanup, health/version-compat, page
    migration — standard lifecycle, to be built with the engine.
- **⛔ BUILD REALITY (confirmed 2026-07-22): the engine 4.4 rides DOES NOT EXIST
  yet.** ychrome has no engine module / no `/engine/*` / no `WPEDisplayHeadless`;
  `wpe-webkit-2.0` is not installed on the fleet (Debian availability uncertain,
  per `ychrome/docs/agent-engine.md` §9 risk register). ychrome's own §8 makes
  **Phase A a gating spike** (prove WPEDisplayHeadless + one WPEWebView + PNG
  readback + isTrusted-input differential + a bindings decision) that has NOT been
  done. So 4.4's build = the full ychrome agent-engine project (Phase A spike → B
  engine daemon+API → C fleet), THEN the yggterm `--page` verb wiring — a
  multi-session native-engine effort, not a finish-now task. The trust gate above
  is resolved; the BUILD is the ychrome campaign, tracked in
  `ychrome/docs/agent-engine.md`.

Acceptance gates 14, 17, 18.

### Deploy discipline (slice 4 is where the deploy rules bite hardest)

- 4.0 is the **only** protocol bump. It cannot use the client-side or GUI-only
  recipes — it needs a **daemon** swap on the live host, the row-dropping-handoff
  path. **Count `server app rows` before and after** every daemon swap, and
  coordinate a window: the live host also hosts other agents' campaigns, and can
  host the very session the user is viewing.
- 4.1–4.2 are daemon-side but ride the 4.0 handshake (deploy with or after 4.0).
- 4.3–4.4 stand up **new processes** (a shadow client, a farm engine); they
  change nothing about the user's existing GUI or daemon and can be proven on an
  idle box without touching the live viewport.

### Open questions — eng-review resolutions (2026-07-21)

1. **Client identity durability.** `client_id` is **server-issued per connection**
   and frozen after registration (eng-review D4/D9). Resolved for identity; the
   one residual detail is whether a reconnecting shadow *reclaims its prior view*
   or starts fresh — decide at 4.0 build (default: fresh view, lease re-taken).
2. **Shadow read scope across the hot-restart chain.** Resolved (eng-review D9):
   the handoff carries role/epoch provenance or forces re-registration; a
   successor connection with no role is never Active. Tested across N/N-1/N-2.
3. **Profile contention policy.** Resolved (eng-review D5/D9): `profile_busy`
   hard-fail is the default; an explicit read-only mirror is the opt-in.
4. **Farm auth.** Resolved (eng-review D9): profiles are host-local + non-shareable
   and `--page` is an opaque capability scoped to `{host, engine epoch, profile
   grant, caller, expiry}`; the full cross-host trust model is 4.4's own sub-spec
   + eng-review gate before it builds.

## Acceptance (F-style, the user's words made testable)

Each gate is a live proof on the desktop host (screenshot / journal / probe),
not a code claim.

0. **Slice-2a proof gate (GO/NO-GO, precedes everything else).** (a) `read` +
   `capture` against a **backgrounded** surface return its real content (not the
   active surface's, not an error) — **✅ engine half proven (2026-07-20)**: on a
   hidden/unmapped webview, `eval` returns correct state and `webkit.snapshot`
   returns a **fresh** frame (center pixel = a color painted after the last
   visible render, not a stale cache — `docs/spikes/slice2a-istrusted-inject`);
   and the reconciler keeps a demoted surface in `applied` → registry →
   `resolve_live_web_surface` until the hold expires (verified in code). A clean
   live yggterm proof that `--session` resolves a real soft-stashed surface is
   still owed on an uncrowded sandbox.
   (b) whether `isTrusted`-true injection into a target WebView is achievable
   without the seat pointer — **✅ PASS / GO (2026-07-20)**: a `gdk_event_new`
   button event filled with the webview's `GdkWindow` + seat device and
   delivered via `WidgetExt::event` yields a trusted click at the right coords,
   no seat move (`docs/spikes/slice2a-istrusted-inject`). **`do` ships on the
   GUI plane (2b), not deferred to the farm.**
1. **Undisturbed shadow probe.** An agent runs `read`, `capture`, and a `do`
   click against a **backgrounded** session while the user stays on a different
   session. The user's viewport never switches; a screenshot before and after
   the run is identical. Journal shows the verbs hit the addressed surface.
2. **Trusted action, no seat pointer.** A `do click --selector` on a login
   button mutates page state that a synthetic (untrusted) click provably does
   not (`isTrusted` differential), and the seat pointer does not move (no cursor
   warp in a full-desktop capture). The Helium-incident class cannot recur.
3. **Event-driven wait.** A crawl uses `wait --until idle:500` / `selector:` and
   completes with **zero** screenshot-poll loops in its journal.
4. **Cheapest-route honored.** A logged-in site flow runs entirely on rung-1/2
   verbs from its site-lore method block; `capture` appears only where the lore
   marks pixels required.
5. **Agent grid is agent-only. ✅ PASSED (2026-07-21, live desktop host).**
   `screenshot --grid` returned a gridded 1920x1160 frame with a 96-cell
   manifest; the very next capture of the same screen was grid-free, because the
   overlay only ever exists in the returned image. `--grid-refine C4 --crop`
   verified too: sub-cells `C4.1`..`C4.9`, and `capture.x - crop.x == image.x`
   held exactly.
6. **Cursor v1. ✅ PASSED (2026-07-21, live desktop host).** Two agents
   (`--agent codex-alpha`, `--agent claude-beta`) drove `pointer move` against
   the viewed session; a compositor grab showed exactly two arrows at the
   requested coordinates with distinct colours and the tags `agent-1 move` /
   `agent-2 move` (≈1050 clustered pixels of each agent's colour at its point).
   Switching the viewport to another session dropped `agent_presence.visible`
   to 0 while `live` still held both bound to the original session, and the
   frame contained **no** cursor pixels (3 scattered near-matches frame-wide
   versus ~1050 clustered when shown). The user's session was restored
   afterwards and contract violations stayed empty.
7. **Secret hygiene (F4).** After a vault `fill`, `read --as html` and `capture`
   of that page show the secret field **masked**, and the journal line carries
   the field name + action but never the value. A grep of the trace/journal for
   the filled secret returns nothing.
8. **Stale handle fails closed (F3).** A verb issued against generation N after
   the surface was recreated (now generation N+1) returns `stale_handle` and
   performs no action — proven by forcing a recreate mid-batch and confirming
   the queued `do` did not fire against the new document.
9. **Human preempts (F3).** With an agent mid-batch on a surface, real seat
   input cancels the rest of the agent's queue (`preempted` in the journal), the
   human's input is not queued behind the agent, and no further agent verb from
   that batch dispatches.
   **Status (2026-07-21): half built, and this gate's framing was wrong.**
   *Ordering / one-verb-in-flight was never missing* — the app-control pump
   (`process_pending_app_control_requests`) already drains ONE request at a time
   behind an `app_control_drain_in_flight` guard and awaits it before the next,
   so verbs cannot overlap and already dispatch in arrival order. Building a
   queue for it would have been a second encoding of ordering. What WAS missing
   is preemption, now owned by `crates/yggterm-shell/src/agent_input_arbiter.rs`:
   per-surface agent **batches** (keyed by the slice-3 `--agent` identity),
   cancelled as a unit on human input, with later verbs from a cancelled batch
   refused `preempted` at the `do` chokepoint and journaled. Unit-tested.
   **Seat-input detector: BUILT (2026-07-21).** The page cannot make this
   distinction — yggterm's injection sets `send_event = 0` and the real seat
   device precisely so WebKit trusts it, so `isTrusted` is true for agent input
   too. It is therefore made at the **webview layer**, where we know what we
   ourselves produced: every injection ends in one *synchronous*
   `WidgetExt::event(...)`, so `deliver_injected_event` sets a flag around that
   single call and a GTK observer
   (`connect_seat_input_observer`, button-press / key-press / scroll — **not**
   pointer motion, which is drift rather than intent) counts only events seen
   with the flag clear. Because GTK delivery is synchronous and single-threaded
   this is a lexical scope, **not a timing window** — it satisfies the
   no-non-determinism rule. The shell drains the count at the `do` chokepoint
   via `DesktopContext::take_web_surface_seat_input` and preempts the batch.
   Unit-tested in both directions, the important one being that the agent's own
   injection is never counted as the human (a false positive there would make
   every batch preempt itself on its second verb).
   ⛔ **Still owed: the LIVE proof.** It needs a real seat clicking a real web
   surface while an agent batch runs — a human action, not something the agent
   can synthesize (any event it could generate would go through the injection
   path and be correctly excluded). Until then this gate is code-complete and
   unit-proven, not live-proven.
   **Corollary for slice 4.1:** it must extend THIS batch table by keying on
   `(client_id, role)` — the `AgentBatch::client_id` seat already exists — not
   add a parallel lease table.
10. **Shadow cannot take over (slice 4.0).** A `Shadow` connection issuing an
    ownership-claiming request (preserved-owner import / `DropTerminalRuntime` /
    a keep-alive owner write) is refused with `shadow_cannot_own` and journaled;
    the PTY's `owner_server_pid` is unchanged; an anonymous/Active client issuing
    the same request still succeeds (no regression). The daemon swap that lands
    4.0 preserves the row count (26 → 26).
11. **Human preempts a shadow across clients (slice 4.1).** With a Shadow client
    mid-batch driving input to session S, the user focusing S on the Active
    client revokes the shadow's input lease within one dispatch, cancels the rest
    of its batch (`preempted`), and the user's keystroke is not queued behind it.
12. **Profile single-writer (slice 4.2).** Two clients requesting a writable
    context on one profile: the second gets `profile_busy` (or the declared
    read-only mirror); the profile is never opened by two `WebContext`s at once;
    releasing the write-lock lets the next client acquire it.
13. **Undisturbed shadow viewport (slice 4.3).** A shadow client switches
    sessions and captures its viewport (grim) while the user's GUI stays on its
    own session — a compositor grab of the user's display before and after is
    identical; both clients' state came from the one daemon.
14. **Plane-portable recipe (slice 4.4).** One site-lore method block runs
    unchanged on the GUI plane (`--session`) and the farm plane (`--page`),
    changing only the handle flag; both planes' journals record the same verb
    sequence.
15. **No silent downgrade across the version chain (4.0, eng-review D7).** A new
    Shadow client to every OLDER daemon in a live N/N-1/N-2 restart chain is
    **refused** (role-enforcement not advertised → Shadow fails closed), never
    silently treated as Active. An Active/anonymous client to the same chain still
    attaches unchanged (26→26 rows).
16. **Shadow cannot reflow the active session (4.3, eng-review D8).** Active and
    Shadow views at different viewport sizes leave the active terminal's rows,
    columns, and pixels **byte-identical**; a Shadow attempting to set winsize /
    focus / scroll is refused and the user's live frame does not repaint.
    **✅ ENFORCED BY CONSTRUCTION at the 4.0 role boundary (verified in tree
    2026-07-22).** The D8 reflow risk is closed *at the protocol*, not merely by a
    later 4.3 guard: `role_gate` (daemon.rs) already denies a `Shadow` every path
    that could reflow the user's PTY — `TerminalResize` (winsize/SIGWINCH),
    `TerminalWrite` (keystrokes), `TerminalRestart` (carries `initial_cols/rows`),
    and `FocusLive`. The *only* PTY-resize path in the daemon is the
    `TerminalResize` handler; the entire Shadow **Allow-set**
    (`TerminalRead`/`TerminalSnapshot`/`TerminalRetainedSnapshot`/`TerminalHistory`/
    `Snapshot`) carries only `path`/`cursor` — **no `cols`/`rows`** — so an observe
    request cannot smuggle a reflow either. A Shadow therefore has *no wire* to
    reflow, keystroke, or focus the user's session, regardless of its own view
    size. Regression-guarded by `role_gate_allows_only_read_observe_for_shadow` +
    the exhaustive no-wildcard `match` (a new geometry-bearing variant fails the
    build until classified). **Still owed:** (a) the live end-to-end demonstration
    (two views at different sizes, compositor grab byte-identical); (b) the
    shadow's OWN-view **canonical/letterboxed grid** so its render is coherent when
    the daemon refuses its fit-resize — a shadow-side *cosmetic*, not a user-safety
    gap (the refusal already protects the user; only the shadow's own frame is
    mismatched until this lands). The shell layer does not yet know it is a Shadow
    (`crates/yggterm-shell` has no `ClientRole::Shadow` reference), so today a
    shadow GUI would fire a fit-`TerminalResize` on focus and be *denied* — safe
    for the user, incoherent for the shadow until the canonical grid is built.
17. **Profile integrity under concurrency (4.2, eng-review D9).** Two clients
    cannot mutate any shared profile component (jar, WAL/SHM, IndexedDB, service
    workers, caches) at once — the second writer gets `profile_busy`; a crash /
    restart recovers to a single writer, never two.
18. **Farm capabilities fail closed (4.4, eng-review D9).** Forged, stale,
    cross-host, and post-reboot `--page` capabilities are rejected; farm
    loss/restart cannot route input or a screenshot to a reused page/profile
    identity.
19. **User geometry change doesn't break agents (4.1/4.3, eng-review D11).** While
    an agent runs verbs against a session, the user docks a sidebar / resizes: the
    canonical grid reflows **once**, an in-flight web verb aborts with
    `stale_handle` / `target_moved` (never a wrong-target click), the agent's next
    capture reflects the new geometry, and no agent verb fires against stale
    coordinates. An overlay-sidebar toggle reflows nothing (PTY winsize unchanged).

### Unit-test requirements (eng-review test pass — fold at build time)

The gates above are live proofs; these unit tests (Rust, in-crate `#[cfg(test)]`,
matching the slice-2b/3 convention) back the pure logic and must ship with each
sub-slice:

- **4.0** — `role_gate(&ServerRequest)` exhaustive-match: Shadow refused on every
  mutating variant, allowed on the read set (backs gate 10/D3). `ClientIdentity`
  serde both directions (old↔new, backs D4). `shadow_cannot_own` emits its journal
  line.
- **4.1** — input lease grant/deny by focus state; per-session FIFO determinism;
  preempt cancels the queued batch and journals it; queued input cancelled on
  disconnect.
- **4.2** — profile-write-lock acquire → second writer `profile_busy` → release →
  re-acquire; crash-recovery re-establishes a single writer.

## Risks and spikes

| Risk | Signal | Mitigation / fallback |
|---|---|---|
| ~~`isTrusted`-true injection may be impossible in WebKitGTK without the seat (the central gate)~~ **RESOLVED — GO (2026-07-20)** | slice-2a proof (done) | `gdk_event_new` button + webview `GdkWindow`/seat device → `WidgetExt::event` = trusted click, no seat move (`docs/spikes/slice2a-istrusted-inject`). `do` on the GUI plane. Remaining sub-risk: delivery into a *demoted/unmapped* webview (below) |
| ~~Injection into a not-visible webview may differ from the mapped case~~ **RESOLVED (2026-07-20)** | spike | injection into an **unmapped** webview delivers nothing (`events=[]`); a **mapped** one (incl. the soft-stash demote, which stays mapped) works. So `do` works on soft-stashed surfaces; a hard-stashed/hidden surface needs a transient off-screen map or defers to the farm. read + capture DO work while hidden (capture fresh) |
| Surface recreated under a queued verb/lease (reused native id) | slice-2b | durable handle `(session, tab, generation)`; verbs fail closed with `stale_handle`; cancellation on recreate (Action & lifecycle) |
| GTK/WebKit event delivery into an unmapped/minimized webview | slice-2a spike | transient off-screen map for the injection; else defer hidden-surface `do` to the farm plane (same verb) |
| `webkit.snapshot` on a truly backgrounded surface returns blank/stale | slice-2 spike | soft-stash keeps it attached+composited; if snapshot still needs a live view, briefly promote-under-lease, capture, demote |
| Two agents `do` the same surface concurrently | slice-2 | per-surface input serialized **FIFO by arrival**, one in flight at a time, both journaled (deterministic ordering, no timing-dependent interleave); human preempts |
| Lease outlives a dead agent | always | TTL + journaled; reconciler reaps on expiry exactly like the background hold |
| Jar single-writer (farm + GUI open one profile) | slice-4 | daemon leases a jar to one live client at a time (shadow-client hard part #2) |
| Shadow client triggers takeover (the 2.11.5 dead-sessions class) | slice-4 | client identity + role (active vs shadow) in the daemon protocol; default-deny allowlist (4.0); a shadow NEVER takes over |
| Silent role downgrade on an old chain daemon = privilege escalation | slice-4.0 (eng-review D7) | daemon advertises role-enforcement; a Shadow fails closed against a daemon that cannot enforce — never silently promoted to Active (gate 15) |
| Shadow reflows the user's live session via SIGWINCH without owning it | slice-4.3 (eng-review D8) | a Shadow never drives winsize/focus/scroll; fixed canonical grid or read-only replay (gate 16) |
| Anti-bot flags injected input | slice-2/4 | same UA/identity + real `isTrusted` events; no evasion beyond honesty (standing rule) |

## Security

- Same-user-only: app-control is already local; the farm socket is 0600 + bearer
  token. No network exposure.
- **Audit is the journal.** Every addressed verb is attributable and replayable
  in reading order — no silent driving. This is the mitigation, not capability
  crippling: engine pages are real authenticated browsing as the user.
- Passkey user-presence invariant unchanged (above). Vault fill stays
  origin-exact, per-fill journal line. No secret ever rides a schema, OSC, or
  command envelope (standing platform rules).
- **Output redaction — the new door (F4).** `read`/`capture` add
  whole-page-content outputs and the journal logs every action, so the standing
  no-secret rule extends here: `input[type=password]` and vault-filled fields
  are **masked in `read` and `capture` output**; the journal records
  `{field_name, action}`, never `{value}`; a `do type` of secret material is
  flagged and its text is not logged. This is not a restriction on the agent
  (allow-default is unchanged) — it stops the vault's own secrets from spilling
  into agent-readable output and on-disk traces (acceptance gate 7).
- Per-profile `agent_drive: allow|deny` (default allow — the owner's explicit
  decision) gates `open`/`do` on the farm plane.

## Settled decisions (do not relitigate — user, 2026-07-19 pt10)

1. **Ordering:** spec → slices 2→3 → **F.2 splits wait**. Beats splits on daily
   value. ("Yes.")
2. **Cursor v1 = MVP, no co-presence toggle.** The one rule above; nothing else.
3. **Escalation ladder is the law of the engine** — cheapest rung first; the
   visible cursor/grid/servo is the most expensive rung, never the default.
4. **Cheapest-route logging = site-lore, as shipped** — markdown SOT + derived
   sqlite; entries carry outcome stats, auto-demote on rot, improve over time.
5. **Surface-generic, not ychrome-only.** The verb layer mounts on the platform
   contract; web is the first adapter, then yedit (document), terminals (the
   orchestrator consumer), cellulose (future). Each new app inherits agent
   control by implementing the adapter hooks.
6. **The engine mounts inside the ychrome daemon** for the farm plane
   (`/engine/*`, shared journal/registry) — no second socket/token/lifecycle.

## Interim mitigation (cheap, pre-slice-2)

Until `--session` addressing lands, add **probe etiquette** to yggui: after any
probe that had to switch the active session, restore the user's prior active
session. Kills the viewport-yank annoyance for the cost of one extra switch, and
is a strict subset of what slice 2 makes unnecessary.

## Eng-review amendments (2026-07-20)

`/plan-eng-review` (Claude) + an independent Codex outside voice, both folded
with the owner's per-finding approval. The load-bearing "exists today" claims
were verified against the code (`resolve_live_web_surface`,
`WEB_SURFACE_NATIVE_IDS` mirror of the reconciler's `applied` map,
`is_read_only`, the soft-stash demote path) — the central "a backgrounded
surface is reachable by `--session` today" claim holds.

1. **F1 — `do` is the single click primitive.** The existing synthetic
   `AppControlCommand::Pointer`/`Grid` click paths (`document::eval` of a JS
   event script, `isTrusted` false) refold onto `do`; one owner of "click a
   surface." The grid machinery is reused for slice-3's capture-side grid.
2. **F2 — slice 2 is spike-gated (2a → GO/NO-GO → 2b).** Both models called the
   `isTrusted` hidden-WebView injection the central unproven gate and warned
   against freezing leases/headless/protocol/cursor around it. 2a proves-or-
   kills it (and addressed read/capture on a stashed surface) first.
3. **F3 — Action & lifecycle correctness is normative.** Durable
   `(session, tab, generation)` handles that fail closed on stale, the reap
   predicate `max(hold, lease)`, defined coordinate space, resolve-and-focus
   `do type`, selector freshness guard, and real human-preemption semantics.
4. **F4 — output redaction.** The standing no-secret rule extends to
   `read`/`capture` output and the journal.

**Honest scope caveat (Codex #10, does NOT reopen settled decision #5):** the
verb layer stays surface-generic by decision, but **web is the only fleshed-out
adapter**; the terminal/document/cellulose columns are one-line mappings, not
implemented contracts. Slice 2 builds the web adapter only — the others inherit
the vocabulary when their adapter is written, and the spec should not be read as
claiming they work yet.

**Discarded as a false positive:** Codex read *"F.2 splits wait until after
slices 2–3"* as the `wait` **verb** being contradictorily deferred. "Splits" =
the F.2 pane-split feature; the `wait` verb is correctly in slice 2b.

**Deferred, not folded (Codex, lower-priority — the owner may revisit):** the
site-lore-as-action-policy trust concern (a poisoned fleet-shared lore entry
driving `do`) — lore is a shipped, separate system the control plane only
references; revisit when `do` actually replays lore method blocks (queue
item 3), not in this spec.

## Field findings — live co-browse dogfood (2026-07-22)

A real co-browse task (external login + admin actions on a third-party web app,
human present) was run end-to-end against the CURRENT build to answer "are we
there yet." Render + DOM-drive + surface isolation held under real load (a fresh
surface, dozens of live sessions untouched). These are the gaps it surfaced,
each mapped to where it belongs:

1. **The co-browse recipe drives the shared foreground GUI, not the shadow
   path — and nothing steers an agent to the shadow path.** The task opened a
   surface, ran `web eval` against the *active* surface, and used full-app
   `screenshot` — i.e. "user-and-agent active compositor control", which yanks
   the human's viewport. The slice-2b shadow reach (`--session`-addressed
   `eval`/`do`/`read` + per-surface `web screenshot` on a soft-stashed
   backgrounded surface) already exists and is live-proven, but was not the
   path taken. → The **restore-user's-active-session-after-probe etiquette**
   this spec names as the pre-shadow mitigation ("The pain" / taxonomy) needs
   to be the *documented default* for co-browse, and the `--session` +
   `web screenshot --session` idiom made the obvious one. Process gap, not a
   code gap — but it is why the human got disturbed.

2. **Headless-create (`open --headless`, slice 2b) is the missing piece for a
   fully-undisturbed NEW surface.** Confirmed live: a surface must be
   foregrounded once to register before `--session` reaches it (a ychrome
   launched/left backgrounded returns `web surface not live (backgrounded or
   not yet revealed)`). Until create-but-never-reveal lands, every *new*
   co-browse necessarily flashes the viewport once. This raises `open
   --headless` from "nice slice-2b verb" to "the thing blocking undisturbed
   co-browse." Best interim flow: foreground once to create+register → restore
   the human's session → drive backgrounded via `--session`.

3. **The `wait` verb exists but is easy to miss → navigate/eval races.**
   `location.href=…` immediately followed by `web eval` races the load; the
   built `wait --until load_finished|js|selector` solves it but wasn't reached
   for (three retries were spent URL-guarding instead). → Either document
   "always `web wait` after navigation" in the recipe, or add `--await-load`
   to the navigate/eval path so the race is unspellable.

4. **Credential plane is host-split; `fill`/`otp` (rung 1) have no cross-host
   path.** The vault agent is per-host. When the driving host ≠ the GUI host,
   there is no secure fill: piping the secret across would breach F4's no-echo
   rule, so the agent stalls at the secret until the human unlocks the GUI
   host's vault. The rung-1 `fill`/`otp` verbs implicitly assume same-host. →
   Define fill/otp routing over the daemon's authenticated socket (the GUI
   host's vault fills in-process, secret never crosses a host boundary); ties
   into slice-4 client identity. Minimum viable: surface a "vault locked on the
   GUI host — unlock there" state so the human knows the exact unblock.

5. **`fill` is specced at rung 1 but not wired as an app-control verb.** Even
   with an unlocked local vault and a strict host `match`, nothing auto-filled
   on load and there is no `server app web fill <host>` to trigger the *native*
   in-surface fill — the task hand-rolled injection via `eval --stdin`. Wiring
   `web fill` (native vault fill, secret stays in-process) closes both this and
   the safe-injection story. Overlaps #4.

6. **Site-lore IS built and pushed — but absent on hosts that haven't pulled
   (a fleet-sync gap, not a build gap).** The `ychrome-site-lore` skill
   (`lore.py` + `lore/<domain>.md` SOT + gitignored sqlite index) exists and is
   on `origin/main` (commit 9c2e557); the escalation ladder's "logging = site-
   lore, as shipped" is accurate. The trap: it is a normal git-tracked skill, so
   a host that hasn't `git pull`ed the ychrome repo (jojo, during this run) has
   no lore and reports it "missing" — reading that as "not built" is wrong (I
   made exactly that error by checking jojo, not dev). → Co-browse setup should
   `git pull` ychrome (or check `origin/main..main`) before concluding lore is
   absent. Mentors.debian.net lore was seeded this run.

7. **Robustness: broken-markup submit controls.** Real pages orphan a submit
   `<input>` from its `<form>` (a parsed-empty-form quirk); a naive `.click()`
   silently no-ops. A `do submit` / "activate this control as the page intends"
   helper (build-and-POST carrying the page's CSRF) would harden `do` against
   markup the agent does not control.

**Deploy state (2026-07-22, jojo v2.12.1) — CORRECTED.** The slice-2b engine
verbs ARE deployed: `do`/`read`/`wait`/`lease` all run on the live 2.12.1 binary
(slice-2b `f6128ec` is an ancestor of the 2.12.1 release, so they shipped in it).
An earlier draft of this note claimed they were absent — that was a bad instrument
read: `server app web --help` prints a STATIC usage string that listed only
`eval`/`screenshot`/`devtools`, so `--help` looked like the whole surface. The
real probe is running the verb (an old binary answers `unsupported app web
action`; the live one returns structured JSON / validates `--until`). The usage
string is now fixed to list every verb. Genuine remaining gaps: (a) headless
surface-create (create-but-never-reveal) is still unbuilt, so a NEW co-browse
surface must be revealed once; (b) `--session` on a backgrounded surface is
bounded by the ~600s reap hold unless a `lease` extends it — the single `not live`
seen this run is consistent with hold-expiry, not a missing feature. Lesson for
the file: never conclude "not deployed" from a usage string.
