# Phase F — web surfaces under the glass

**Status: F.0 landed but OPT-IN (`YGGTERM_WEB_SURFACE_UNDER_GLASS=1`) after a
live incident; F.0.1 (the hole redesign) is the next work item.**
Owner surface: web surfaces (ychrome pilot). Repo side: yggterm shell +
vendored `dioxus-desktop` web-surface host. GUI-only; no wire change.

**Incident (2026-07-19, first live deploy):** the transparency-chain
implementation cleared inline backgrounds on EVERY `[data-ws-page]` ancestor
up to `<html>` — shared app containers included — from the geometry eval.
Three compounding flaws: unscoped (app-wide background loss), mutation fights
the renderer (Dioxus re-renders rewrite `style`), unrestorable (the restore
branch dies with the eval when surfaces close / the loop idles). The live GUI
broke app-wide within minutes. Mitigated by relaunching with the legacy
override; the chain-clearing is now REMOVED and the geometry eval is
sample-only (tripwire-enforced). Second live finding, fix kept: xterm
canvases are opaque glass-DOM pixels that occlude the page — the terminal
host now hides under an active overlay (`data-web-surface-owns-viewport` +
`WEB_UNDER_GLASS_CSS`, visibility-only so layout never moves).

**F.0.1 PROGRESS (2026-07-19 pt2, dev sandbox, real ychrome surface, commit
8047946) — partial, page still not compositing:**
- DONE + probe-verified: terminal host frame background transparent + xterm
  canvas hidden under an active web overlay (the c5b4871 terminal-occlusion is
  GONE); web overlay panel + page placeholder transparent; the window is now
  built with an RGBA visual when under-glass is armed (GTK gives the glass
  webview an alpha channel to composite the hole onto the page webview below
  ONLY when the top-level window is transparent — the raw-webkit spike got
  this for free from the compositor; the wry window defaults opaque).
- STILL BLOCKED (root-caused, not fixed): the web surface renders through
  opaque `web_frame_bg` BACKING WRAPPERS that the host/overlay/page CSS does
  not reach. Confirmed one at the **depth-2 parent of `[data-ws-overlay]`**
  (#f4f4f2, no data-attribute) in the **MainSurface split render path**. There
  are TWO web-surface render paths (the `TerminalCanvas` host path AND the
  MainSurface split path); each has its own opaque backing wrappers. The
  incident's ancestor walk existed to catch ALL of them — the correct
  replacement is **render-time conditional transparency on each backing
  wrapper** (mark it, or make its `background` conditional at the source),
  across BOTH paths. Legacy stacking proved the page renders and grim captures
  it, so this is a DOM-layering gap, not a capture artifact.
- VERIFY METHOD THAT WORKS ON THE SANDBOX (no KDE needed): headless sway +
  `grim` DOES capture native webviews (legacy stacking showed example.com);
  `--backend os` faithful capture REFUSES on non-KDE. Create a real surface:
  `server app terminal new` → send `<abs-path>/ychrome --profile p <url>` into
  it. Probe the hole ancestor chain for `<OPAQUE>` backgrounds; compare the
  hole pixel to backdrop (theme bg) vs the page's own bg to disambiguate.

**F.0.1 — the hole, redesigned (remaining work):**
1. **No runtime DOM mutation.** Render-time conditional styling from a
   snapshot flag: the reconcile loop mirrors `under_glass` into ShellState on
   change; RenderSnapshot carries it; rsx conditionals do the rest.
2. **One owner for app background painting.** The hole is unimplementable
   while backgrounds are scattered across shared containers. Consolidate to a
   designated app-background layer; session-view-scoped elements (viewport
   wrapper, overlay column, `[data-ws-page]`) get conditional transparent
   backgrounds; the background layer gets the hole treatment (evenodd
   clip-path from the same geometry sample) if it overlaps page rects.
3. **Probe first:** full ancestor-background inventory at the hole rect, in
   the dev sandbox with a synthetic surface (printf OSC declare + python
   /ping — no ychrome setup needed).
4. **Verify gate that would have caught the incident:** the headless smoke
   MUST open a real web surface (synthetic router), screenshot the hole, then
   close the surface and screenshot again — backgrounds intact both times.

## The problem (north star)

A web page today is a native child webview stacked **above** the shell
webview's DOM. Two user-visible consequences:

1. **Corners cannot be molded.** The viewport frame is DOM; the page is a
   native sibling above it. CSS `border-radius` on `[data-ws-page]` cannot
   round a widget that draws over the DOM, so the page sits *inside* the
   frame with an inset gap instead of the frame cutting its corners — "a
   square TV with a rounded photo frame laid over it" is impossible.
2. **The titlebar reflows the page.** The auto-hide titlebar is DOM; it
   cannot draw over the page, so the reconciler clamps the webview below the
   titlebar's bottom edge — revealing the titlebar *resizes* the page
   instead of floating over it.

Terminals never had either problem because xterm paints into a canvas that
IS the shell DOM. Phase F gives web surfaces the same property — not by
re-rendering them, but by putting the chrome above them.

**Why the full inversion:** a corner-mask widget above the pages would mold
the corners alone in a day. The input-model inversion below is bought for
the *general* property — titlebar and any chrome floating over live pages —
which is the stated acceptance, not just the corners.

## What the spike falsified (2026-07-18, headless Wayland + X11 harness)

The long-standing belief was that a WebKitGTK child webview is a **Wayland
subsurface** that draws above everything, so only offscreen compositing
(WPE → texture) could fix this. The spike disproved that:

- WebKitGTK 2.52 (GTK3) composites accelerated content **in-widget** (DMABuf
  import painted in the widget's draw). There is no subsurface. GTK widget
  z-order and alpha compositing are fully honored, on Wayland and X11.
- The "draws above all DOM" symptom was always plain widget stacking: the
  shell webview is the `gtk::Overlay`'s base child and page webviews are
  `add_overlay`'d above it. Flip the order and the chrome wins.
- A shell webview with `webkit_web_view_set_background_color(alpha 0)` and a
  transparent DOM hole shows the page **through** the hole, with the DOM
  frame (rounded corners, `box-shadow` spread) covering the page's square
  corners, and a DOM titlebar drawing **over** the page. Pixel-proven.
- **Input pass-through works**: punching the page rect out of the shell's
  GdkWindow input shape routes pointer events in the hole to the page
  webview below — full DOM click handling in the page, chrome regions stay
  shell-interactive, and GDK crossing events track the hole boundary.
  Proven on both backends (the picking code is shared, client-side GDK).

Scope of proof: pointer + paint on this fleet's exact webkit/driver combo.
Keyboard round-trip (type into the under-glass page, then a shell hotkey)
is a **pre-F.0 spike extension** — run it in the harness before writing
F.0 code. Arbitrary-machine variance is handled by the runtime self-probe
(below), not by extrapolating the spike.

Spike traps (both cost a debugging round; bake them into the implementation):

- **`GtkOverlay` wraps every overlay child in an intermediate GdkWindow with
  an empty event mask.** Shaping only the webview's own window is not
  enough: the unshaped intermediate still picks, selects nothing, and GDK
  bubbles to the *toplevel* (an ancestor) — never the page (a sibling).
  Shape every window from the webview **up to but excluding the toplevel**.
- **Never shape the toplevel.** On X11 its `parent()` is the root window,
  not `None` — a naive ancestor walk shapes the toplevel and clicks fall
  through the whole application.

Spike source: `docs/spikes/phase-f-under-glass/` (re-run on the target host
before F.0 lands; it needs only a compositor and a pointer).

## The architecture: under-glass pages

Invert the stack. The shell webview becomes the TOP layer ("the glass");
page webviews live under it.

```
gtk::Overlay
├─ base child: background widget (theme background color)
├─ overlay: page webview containers (one gtk::Fixed per surface, as today)
└─ overlay (topmost): SHELL webview — transparent background
```

- **Paint**: shell DOM paints all chrome opaquely; the `[data-ws-page]`
  element becomes a transparent hole with the molded frame (rounded
  corners, hairlines, focus rings) drawn around/over the page edges. With
  no page open there is no hole — the shell is visually identical to today.
- **Input**: the shell's **input region** = full window − (hole rects −
  chrome-cover rects), applied as a GdkWindow input shape (with the
  ancestor walk above). Events in holes reach the pages exactly as before
  the restack; everywhere else the shell owns input.
- **ONE geometry sample, two outputs.** Hole rects and place rects come from
  the SAME `[data-ws-page]`/`[data-ws-pinned-*]` sample in the SAME
  reconcile eval. A second sampler for the region is forbidden — two
  samplers can diverge, and divergence here is an input-correctness bug,
  not a cosmetic one.
- **Chrome over pages is declared, not inferred**: any DOM element that can
  legitimately cover a page (auto-hide titlebar, dialogs, pickers, toasts,
  KeyTips) carries `data-covers-web-surface`; its rect is subtracted from
  the holes. The titlebar is merely the first such element — revealing it
  becomes an input-region update, **not a geometry clamp**: the page never
  moves.
- **Covers push synchronously, not on the tick.** The `[data-ws-page]`
  geometry oracle is documented starvable (seconds, under output flood) —
  acceptable when it only places pixels, unacceptable as the input
  authority. A shell-side MutationObserver on `data-covers-web-surface`
  mount/unmount/resize pushes a region update **immediately**; the
  reconcile tick remains as idempotent self-heal only. Without this, a
  dialog is visible for one-to-several ticks while clicks on it land in
  the page — an invisible misclick.
- **Reveal trigger — host motion observer.** Once the clamp dies, the
  titlebar's hover-reveal zone sits inside the hole, so the shell never
  sees the mousemove. The host attaches a GTK `motion-notify` observer on
  page webviews (`Propagation::Proceed` — observe, never consume) and
  forwards edge-zone motion to the shell, which runs its normal reveal
  logic. The same observer generalizes to any hover-triggered chrome. (A
  reserved shell-owned top strip was considered and rejected: it steals
  the page's top-edge input and solves only the titlebar.)
- **Modal stash retires with the covers mechanism (F.1).** Today
  `has_modal_over_viewport()` stashes every surface because pages draw
  above DOM. Under glass, a modal's rect becomes a cover and the page
  stays visible beneath it. Retiring the stash and landing the covers push
  are ONE work item — retiring it alone makes every modal a click-through
  window; keeping it makes "dialogs float over pages" false.
- **Shell-topmost is an invariant with three writers.** `open()`,
  `unstash()`, and `build_popup_webview` (born inside WebKit's synchronous
  `create` handler) all `add_overlay` — which appends ON TOP. Every attach
  point must restack below the glass (`gtk_overlay_reorder_overlay`), and
  a debug assertion verifies the shell is the last overlay child after
  every attach.
- **Safety invariant (blast radius).** A bad input shape dead-zones the
  whole shell — terminals included. Therefore: the shape exists only while
  ≥1 live page rect exists; zero pages, any assembly error, or any
  inconsistency ⇒ **remove the shape entirely** (unshaped glass = chrome
  fully interactive, pages temporarily mouse-unreachable — the safe
  direction). Clear + reapply on `web-process-terminated` of the shell as
  well as pages.
- **Runtime self-probe + auto-fallback.** At startup (and on first surface
  open), render a known pixel under the glass and read it back. Probe
  failure ⇒ automatic legacy (pages-above) stacking for the session. The
  legacy path stays alive as this auto-fallback — it is not a user-facing
  mode; the probe owns the decision, which keeps the no-fallback-layers
  doctrine intact (one authority, no silent divergence). It is also the
  landing spot if the performance budget is blown.
- **Corner wedges: accepted discrepancy.** The visual hole is rounded; the
  input hole is rectangular. The 24px corner wedges look like chrome but
  route input to the page. Accepted and documented — the wedges sit at the
  page's extreme corners where misrouted clicks are near-harmless. Revisit
  with region strips only if it is ever noticed in use.

### What falls out for free

- Molded rounded corners + seamless frame (the headline).
- Titlebar reveal without page reflow (the other headline).
- The first-paint overflow class becomes invisible: a mis-sized page at
  birth is *under* the chrome, so nothing can overflow the frame visually.
- Pane-focus on page click (Phase 3 residual): the host observes GTK
  `button-press` on page webviews (`Propagation::Proceed`) and updates the
  split focus ring — same observer family as the motion observer.
- Split gutters/rings may draw over page edges — clean 4-pane web splits.

### What is deliberately unchanged

Per-surface `WebContext` (profile jars), SOCKS egress, userscripts,
adblock, the `yggterm-appctl://` passkey bridge, related-view popups,
devtools, zoom, `snapshot_full_page`, the OSC/declare protocol, tab model,
session identity. The restack moves widgets; it does not touch the engine,
the network path, or any wire format. This is the decisive advantage over
every alternative below.

## What already exists (reused, not rebuilt)

- Reconciler rect sampling (`[data-ws-page]`, `[data-ws-pinned-*]`,
  titlebar bottom) — extended to emit the input region from the same
  sample; not duplicated.
- `apply_bounds` + per-surface `gtk::Fixed` containers — unchanged.
- `last_place_rect` change-gating pattern — reused for apply-on-change of
  the input region.
- The whole egress/profile/popup/passkey/adblock substrate — untouched.
- The spike harness — becomes the standing integration proof.

## Rejected alternatives

- **WPE WebKit offscreen → texture** ("Phase F classic"): an engine swap.
  Loses or re-plumbs related-view popups, appctl custom scheme, per-view
  proxy, content filters; still leaves the unsolved problem of transporting
  pixels into the shell DOM at interactive rates; Linux-only. Rejected —
  the spike removed its premise.
- **GTK4 / webkitgtk-6.0 migration**: in-scene-graph webviews would also
  work, but tao/wry/dioxus-desktop are GTK3; migrating the whole windowing
  stack is a campaign of its own with zero cross-platform payoff. Deferred
  indefinitely, not needed for this.
- **Corner-mask-only widget** (native rounded-corner masks above pages,
  input-transparent): molds the corners for a day's work but can never
  float DOM chrome over the page — fails acceptance 2. Noted as the cheap
  middle path deliberately not taken.
- **Reserved top strip as reveal trigger**: rejected for the motion
  observer (steals page top-edge input; titlebar-only fix).
- **More CSS/margin tweaks on the current stack**: cannot work; the page is
  above the DOM. (The earlier inset fix stopped overflow but can never mold.)

## NOT in scope

- WPE/offscreen compositing in any form — premise falsified.
- GTK4 migration — separate campaign if ever.
- Non-Linux implementations of the three platform primitives — contract
  only (below); no Windows/macOS/iOS/Android code in Phase F.
- Arc-accurate input regions for corner wedges — accepted discrepancy.
- KeyTips app-contribution tenancy — still waits for 3.0.0 (layer unbuilt).
- Profile-jar sync, agent-engine implementation — own campaigns, unchanged.

## Platform portability (design constraint, stated once)

yggterm is planned for Linux/Windows/macOS/iOS/Android. The Phase F
**contract** is platform-neutral and must be stated at the shell level:

> Pages render under the chrome glass. Chrome that covers a page declares
> `data-covers-web-surface`. The platform host provides three primitives:
> (1) under-glass z-order, (2) a transparent shell layer, (3) an input
> pass-through region.

macOS/iOS: WKWebView is an ordinary view — z-order, alpha, `hitTest`
override are native vocabulary. Android: WebView is a View with full
touch-dispatch control. **Windows: requires WebView2 visual-layer
(DirectComposition) hosting, which wry does not use today — the default
HWND hosting gives neither alpha glass nor per-region pass-through; label
this "requires wry visual-hosting work, unverified", not free.** Only
GTK3/Linux ever had the "native child draws above" trap, and the spike
shows even that was a stacking choice. Keep the region assembly +
covers-rects logic in shared shell code; only the three primitives are
per-platform.

## Risks and probes

| # | Risk | Probe / mitigation |
|---|------|--------------------|
| 1 | WebKit recreates GdkWindows (crash recovery, re-realize) → shape lost | Region push idempotent on the reconcile tick; reapply on `map-event` + `web-process-terminated` (pages AND shell) |
| 2 | Shell transparency glitches on the production (wry-built) webview | F.0 gate: re-run spike on target host; runtime self-probe auto-falls back to legacy stacking |
| 3 | Click-through: DOM overlay over a hole missing `data-covers-web-surface` | Debug tripwire logs DOM elements intersecting holes above the frame's z-index; QA checklist item |
| 4 | Cover-rect latency (starvable oracle) → visible-but-not-clickable chrome | Synchronous MutationObserver push on cover mount/unmount; tick is self-heal only |
| 5 | Bad region push dead-zones the shell (terminals included) | Safety invariant: zero pages or any assembly error ⇒ remove shape entirely |
| 6 | Full-window alpha blend cost | **Budget: p95 input-to-paint latency delta ≤ one frame (16ms) vs legacy on a scroll-heavy page, measured with existing latency telemetry.** Blown budget ⇒ self-probe fallback becomes default, Phase F pauses |
| 7 | HiDPI: region units | GDK input shapes take logical window coords — same units the reconciler samples; test once at scale=2 in the headless harness |
| 8 | Drag from page into chrome mid-selection | GDK implicit grab pins the stream to the pressed window — correct by default; verify with a live text-drag |
| 9 | IME/text-input focus for under-glass pages on Wayland | GTK routes text-input by focus widget, unchanged; verify a compose-key input once |
| 10 | Keyboard focus under glass unproven | Pre-F.0 spike extension: type into page + shell hotkey round-trip in the harness |
| 11 | Transparency chain: one opaque ancestor/theme rule hides the page silently | Tripwire samples computed backgrounds along the hole's ancestor chain in BOTH themes; DESIGN.md rule: chrome over pages is opaque (no translucency-over-content vocabulary for web panes) |

## Implementation phases (each live-verified before the next)

- **F.-1 — spike extension (prerequisite).** Keyboard round-trip in the
  harness: click hole → type → page DOM received text; then a shell-side
  key while shell focused → shell received it. Pointer already proven.
- **F.0 — restack behind the probe.** Vendored host: shell attaches as the
  topmost overlay child; transparent shell background; pages under it;
  restack-below-glass at ALL THREE attach points (`open`, `unstash`,
  popup-create) + debug assertion "shell is last overlay child"; runtime
  self-probe with auto-fallback to legacy stacking; static input region =
  page rects (titlebar keeps today's clamp in F.0) minus a coarse static
  cover set (toast container, KeyTips layer — modals still stash in F.0,
  so dialogs are safe until F.1); safety invariant wired (zero pages ⇒ no
  shape). **F.0 gates:** focus-cascade audit (`web_surface_owns_viewport`
  consumers + `hostOwnsActiveTerminalInput` under the new event flow)
  BEFORE the restack lands; spike re-run on the target host; faithful
  `--backend os` capture is the required eye from day one; F.0 soak
  assertion: no interactive chrome intersects a hole.
- **F.1 — the molded frame + floating titlebar.** Region assembly from
  holes − `data-covers-web-surface` rects as a PURE function with unit
  tests (single hole; titlebar overlap; dialog fully covering a hole; two
  pages + pinned pane; hidden surface ⇒ no hole); synchronous cover push
  (MutationObserver) + apply-on-change gate (cache last-applied region);
  covers-attribute tripwire test (declare-forwarder tripwire family);
  host motion observer wired to the reveal logic; delete the titlebar
  clamp; retire the modal stash (modal rect = cover, same work item); DOM
  molds the frame (rounded corners over page edges, remove the interim
  inset); transparency-chain tripwire + DESIGN.md opacity rule. Live
  proof: 4 corner crops show the frame cutting the page; `[data-ws-page]`
  rect identical before/during/after a titlebar reveal cycle; typing +
  link clicks + a dialog over a live page all work.
- **F.2 — splits + focus.** Page-click pane-focus observer; rings/gutters
  over page edges; pinned panes get holes. Live proof: 2-pane and 4-pane
  web splits, focus ring follows page clicks.
- **F.3 — sweep.** Popups, picker mode, hidden surfaces, stale overlay,
  screenshot-backend docs, DESIGN.md molded-frame vocabulary, SKILL
  updates. The legacy stack is NOT deleted — it remains the self-probe's
  auto-fallback; revisit its lifetime when public distribution ships.

## Acceptance (the user's own words, made testable)

1. A screenshot shows the web viewport molded like the rest of yggterm:
   rounded frame cutting all four page corners, no inset gap.
2. Hover-revealing the titlebar draws it **over** the page; the sampled
   `[data-ws-page]` rect does not change during the reveal.
3. The page remains fully usable: type, scroll, click links, complete a
   login (site-lore method), passkey get, SOCKS egress check, adblock on.
4. A split with a pinned page paints both panes and focus follows clicks.
5. A dialog opened over a live page is clickable the instant it is visible
   (no cover-latency misclick window).

## Implementation Tasks

Synthesized from the eng review + outside voice. Checkbox as shipped.

- [x] **T1 (P1)** — spike harness — keyboard round-trip extension (F.-1) —
      PASSED 2026-07-19 (page and shell each receive keys after their click;
      Wayland injection needs a held virtual keyboard, `wtype -s 2500`)
- [x] **T2 (P1)** — vendored host — restack + transparency + 3-writer
      reorder (open/unstash/popup-create) — code landed, live proof pending
- [x] **T3 (P1)** — vendored host — self-probe (env override + engine ≥2.40 +
      Wayland native-window walk per surface open) + auto-demote — code landed
- [x] **T4 (P1)** — shell.rs — focus-cascade audit: PASSES with no change.
      The Rust reclaim already stands down when the active session has a live
      web surface (`web_surface_owns_viewport`); page clicks never reach shell
      DOM; the JS cascade's focus() is internal to the shell webview and
      cannot yank GTK focus from a page webview (F.-1 proved the round-trip)
- [x] **T5 (P1)** — safety invariant: zero pages ⇒ full region push (idle
      path); empty-holes ⇒ full region in `glass_input_region`
- [x] **T6 (P2)** — covers v1: `data-covers-web-surface` stamped on toast
      CARDS (not the pointer-events:none stack root); KeyTips badges need no
      cover (decorative, clicks should pass to the page)
- [x] **T7 (P1)** — `glass_input_region` pure fn + 6 unit tests (pulled
      forward from F.1 — cairo regions need no display)
- [ ] **T8 (P1)** — shell.rs — synchronous cover push (MutationObserver) + apply-on-change (F.1)
- [ ] **T9 (P1)** — shell.rs — motion observer → reveal logic; delete clamp (F.1)
- [ ] **T10 (P1)** — shell.rs — modal stash retirement tied to covers (F.1)
- [ ] **T11 (P2)** — tests — covers-attribute tripwire + transparency-chain tripwire (F.1)
- [ ] **T12 (P2)** — shell.rs — pane-focus click observer; split rings over pages (F.2)
- [ ] **T13 (P3)** — docs/DESIGN.md — molded-frame vocabulary, opacity rule, capture docs (F.3)

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 0 | — | — |
| Codex Review | `/codex review` | Independent 2nd opinion | 0 | — | (codex not installed; Claude subagent ran as outside voice) |
| Eng Review | `/plan-eng-review` | Architecture & tests (required) | 1 | CLEAR (PLAN) | 7 issues (3 arch, 1 quality, 2 test gaps, 1 perf), 0 critical gaps open |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | — | — |
| DX Review | `/plan-devex-review` | Developer experience gaps | 0 | — | — |

- **CROSS-MODEL:** outside voice (Claude subagent, fresh context) raised 13
  findings; 10 folded directly, 3 decided by the user (reveal trigger →
  host motion observer; legacy lifetime → self-probe + keep as
  auto-fallback; corner wedges → accepted, documented). No finding rejected
  without a decision.
- **VERDICT:** ENG CLEARED — ready to implement (F.-1 → F.0 first).

NO UNRESOLVED DECISIONS
