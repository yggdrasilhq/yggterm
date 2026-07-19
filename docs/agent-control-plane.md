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

- **Agent-only click grid.** Composited into the screenshot `capture` returns
  (capture-side overlay + a JSON manifest of cell→coord), never injected into
  the page. The user never sees it. Trivial once capture is session-addressed.
- **Visible distinct agent cursors — cursor v1 (the ONE settled rule).** When an
  agent is working a session **and the user is viewing that same session's
  viewport**, the user sees that agent's colored pointer tagged `agent-N`,
  driven by the engine's pointer state for that agent. That is all v1 does:
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
     all** without moving the seat pointer — try GDK event synthesis AND
     WebKitGTK automation/WebDriver input dispatch. **GO/NO-GO gate.**
   - **2b — ENGINE, only on GO (1–2 sessions).** `do` (single injection
     primitive, F1) + the Action & lifecycle correctness invariants (F3) +
     `wait` + agent lease + headless create + the addressed command protocol.
     **On NO-GO:** `do`/type/key defer to the farm plane (slice 4); 2b still
     ships `read`/`wait`/`capture`/`lease` without trusted `do`, and the "act
     without the seat on the GUI plane" promise is honestly deferred.
3. **Agent presence (1 session).** Capture-side grid (**redirect the existing
   `Grid`/`ClickGridParams` machinery** to the returned image — ~80% built, not
   net-new); agent cursor overlays (cursor v1). Pure GUI.
4. **3.0.0 — true shadow clients + idle-host farm.** Protocol client identity + roles
   (takeover guard), jar leases, input arbitration; headless WPE farm (ychrome
   agent-engine.md Phases A–E). The headless-sway recipe proven this campaign is
   the pattern.

## Acceptance (F-style, the user's words made testable)

Each gate is a live proof on the desktop host (screenshot / journal / probe),
not a code claim.

0. **Slice-2a proof gate (GO/NO-GO, precedes everything else).** (a) `read` +
   `capture` against a **backgrounded** surface return its real content (not the
   active surface's, not an error). (b) A verdict — with evidence — on whether
   `isTrusted`-true injection into a target WebView is achievable without the
   seat pointer. This gate decides whether gates 2/6's `do` path is on the GUI
   plane or deferred to the farm plane.
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
5. **Agent grid is agent-only.** `capture --grid` returns a gridded image + cell
   manifest; a simultaneous human-side screenshot shows **no** grid in the live
   page.
6. **Cursor v1.** With an agent working session X and the user viewing X, the
   user sees exactly one `agent-N` colored pointer tracking the agent's actions;
   viewing session Y shows none.
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

## Risks and spikes

| Risk | Signal | Mitigation / fallback |
|---|---|---|
| **`isTrusted`-true injection may be impossible in WebKitGTK without the seat (the central gate)** | slice-2a proof | GO/NO-GO gate BEFORE any engine is built; try GDK synthesis + WebKitGTK automation input; NO-GO ⇒ `do` → farm plane, 2b ships read/wait/capture/lease without trusted `do` |
| Surface recreated under a queued verb/lease (reused native id) | slice-2b | durable handle `(session, tab, generation)`; verbs fail closed with `stale_handle`; cancellation on recreate (Action & lifecycle) |
| GTK/WebKit event delivery into an unmapped/minimized webview | slice-2a spike | transient off-screen map for the injection; else defer hidden-surface `do` to the farm plane (same verb) |
| `webkit.snapshot` on a truly backgrounded surface returns blank/stale | slice-2 spike | soft-stash keeps it attached+composited; if snapshot still needs a live view, briefly promote-under-lease, capture, demote |
| Two agents `do` the same surface concurrently | slice-2 | per-surface input serialized **FIFO by arrival**, one in flight at a time, both journaled (deterministic ordering, no timing-dependent interleave); human preempts |
| Lease outlives a dead agent | always | TTL + journaled; reconciler reaps on expiry exactly like the background hold |
| Jar single-writer (farm + GUI open one profile) | slice-4 | daemon leases a jar to one live client at a time (shadow-client hard part #2) |
| Shadow client triggers takeover (the 2.11.5 dead-sessions class) | slice-4 | client identity + role (active vs shadow) in the daemon protocol; a shadow NEVER takes over |
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
