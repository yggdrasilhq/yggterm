# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on guihost.

## ⚠ READ FIRST — the state of the machine, 2026-07-26

- ⭐ **SUPERSEDED 2026-07-31: under-glass is now armed BY DEFAULT on a
  hardware-GL host** (`under_glass_default_armed`, `apps/yggterm/src/main.rs`;
  commit "under-glass is the standard, not a flag you have to know"). The user
  quit the GUI, relaunched it the ordinary way, found the web surface no longer
  sat flush, and settled it: *"I could not understand why our software needs an
  extra flag to be correct."* The earlier pairing recorded here — "guihost runs
  hardware GL with Phase F under-glass OFF", adopted because arming put a
  background agent's page over the user's entire window — is **no longer the
  configuration**. What changed is that the incident's real cause was degraded
  paint, not stacking, and unrevealed surfaces are now structurally unmapped
  (pixel-proven). Escape hatches unchanged: `YGGTERM_WEB_SURFACE_UNDER_GLASS=0`,
  or `YGGTERM_WEB_SURFACE_LEGACY_STACK=1` which beats everything; the software-GL
  demotion still refuses to arm where DMABuf would SIGSEGV.
  `YGGTERM_FORCE_SOFTWARE_GL=1` still reverts the GL side of everything.
- **The GPU CPU win is NOT established.** Every large figure previously recorded
  here was an artefact of comparing windows with different paint exposure. See
  `docs/optimization-pass.md` and field guide §7.3 before quoting any number.
- **Two supernumerary daemons persist** holding unmigratable `local://` shells.
  That is the durable half of the chaining bug, still open.
- ✅ **The vault agents on dev and guihost are current and unlocked (2026-07-31).**
  Neither needed an unlock in the end. guihost was already satisfied; dev's binary
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
  ⚠ **Not live-verified on guihost** — this was a test-discipline lane, product
  behaviour is unchanged and the deploy happens separately.
  **The habit stands regardless: before trusting ANY test in a report, mutate
  the production call site yourself.**

## ⭐ USER-REPORTED, ychrome-as-main-browser (2026-07-30) — OPEN

The user made ychrome their daily driver and reported a list. **Ten of those are
FIXED and deployed** (see CHANGELOG 2.12.18 and campaign ROUND 28: the app tab's
✕ quitting the app, left-neighbour close selection, duplicate→about:blank, blank
overwriting a real tab, the row menu clipping over a page, disabled caching,
ungranted clipboard, the 320 MB memory cap that distorted video audio, a revealed
page never taking the keyboard, dead back/forward). **These remain, each already
root-caused — read the mechanism before opening the file, and grep the named
symbol rather than trusting a line number:**

- ~~**The vertical tab rail is not the cwdtree.**~~ **FIXED** on
  `lane/dev/rail-is-the-cwdtree` — awaiting the user's own eyes. There is ONE
  row-tree ordering engine now (`yggui::reorder_row_tree`; the flat helper is
  gone, a flat list being the degenerate case), `AppPaneWidget::ListRow` carries
  `depth`/`expanded`/`expand_action` so contributed panes are trees too, and
  `WebTabsRailBody` draws every row — folder and tab alike — through
  `SessionStyleRow` + the shared `RowDisclosureChevron`. Folders sit above tabs,
  drag both reorders and re-parents, folders and tabs rename in place by
  double-click or the row menu, and "New folder" opens with its placeholder
  selected.
  **The user's three follow-up gaps are also FIXED**, on
  `lane/dev/rail-folder-icons-nesting`: (1) a group row now wears the cwd
  tree's FOLDER GLYPH (`RowFolderIcon`, filled = open / outline = shut) in its
  leading slot with the chevron in a new always-visible trailing `expander`
  slot, exactly as the cwd tree does — one owner, and contributed panes
  inherit it; (2) the whole drag EXPERIENCE is one object,
  `yggui::RowDragGesture`, which every row list drives: ghost card, dim, drop
  line/ring, spring-loaded auto-expand, Escape-to-cancel, release-over-nothing,
  and a committed drop no longer also clicking the row. yedit gets all of it
  with zero app-side change; (3) `WebTabFolder::parent` makes rail folders
  nest arbitrarily deep, with the descendant-drop refusal proven on the rail
  path and the nesting round-tripping through `tabs.json`.
  **Known limit, stated honestly:** spring-load and every hover rule are driven
  by pointer MOVE events, so a pointer held perfectly still (zero motion, not
  even sub-pixel jitter) over a shut folder never re-fires the hover and the
  folder does not spring. Real pointers jitter; an automated pointer that
  issues exactly one move does not. If this ever bites a user, the fix is a
  render-loop tick that re-evaluates the dwell, not a second hover path.
  **The user's DENSITY follow-up is also FIXED**, on `lane/dev/rail-density`
  ("significant waste of horizontal space on each row … 2 spaces worth of more
  indentation in the folder"). Measured on the live rail: 51px of leading
  gutter, of which 26px was an icon box every TAB row reserved and never
  filled, plus a 6px gap for an expander it never drew — because the rail
  handed those slots `rsx!{}` (an empty element) where `None` means an absent
  slot. `SessionRowDensity::Rail` now declares `status_column_px: None` (ONE
  20px mark column: the folder glyph or the loading dot, with the dot riding
  the icon as a corner badge when a contributed row has both) and
  `indent_step_px: 19` (12 + two space-advances of the row's own 12px Inter).
  Gutter 51px → 34px; every rail row gains label width at every depth
  (+23px at depth 0, +16 at depth 1, +9 at depth 2). The cwdtree is
  byte-identical — it draws into both its columns, so it keeps them.
- **Downloads have no destination choice, no progress, and lie about history.**
  `decide-destination` is SYNCHRONOUS, so a blocking save dialog there freezes
  every terminal in the app (staged destination + async picker is the shape).
  Progress needs the retained `Download` handle polled (`connect_received_data`,
  `estimated_progress`) rather than new events. The two toasts never coalesce
  (`upsert_job_notification("download:<id>")` + `finish_job_notification`).
  Dismissing a toast DELETES it from the panel — one `Vec<ToastNotification>` is
  both queue and history; add `dismissed`, split the verbs, and move the
  visibility predicate into yggui as ONE exported fn (the app-control snapshot
  re-encodes it today). **And a download that outlives its last tab emits
  nothing at all** — the drain sits after two early `continue`s in the reconcile
  loop; move it above them.
- **No printing at all.** `WebSurfaceHost::print` modelled on `find` + GTK
  `PrintOperation::run_dialog` (its "Print to File" is the PDF destination users
  actually want). Do NOT build a Chrome-style preview: nothing in the tree can
  display a PDF, and every settings change would mean re-rendering the document.
- ⚠ **Cross-cutting, now SOLVED for the mechanism, still open for print:** a
  browser accelerator claimed in the shell's DOM keydown cannot fire while a page
  holds GTK focus. The claimer exists — `connect_window_chord_claimer` on the
  TOPLEVEL window (`web_surface.rs`), matching a table the shell pushes
  (`claimed_chords_for` in `shell.rs`). Adding `Ctrl+P` is ONE row in that table
  plus one arm at the terminus; it is left out only because there is no print
  path to route it to yet.
- **Fullscreen video takes the WINDOW fullscreen** (KDE then hides its panels
  until the video is un-fullscreened, even from another session). User-settled
  design: fullscreen fills the VIEWPORT by default, with an ychrome setting for
  the "real fullscreen" experience. WebKit's enter/leave-fullscreen signals are
  not bound at all yet.
- **Cloudflare managed challenges fail (brilliant.org login).** THREE converging
  causes, all ours: the passkey shim replaces `navigator.credentials` on EVERY
  page (a fingerprint mismatch a challenge can see) — install it lazily and
  per-origin; the adblock filter may be eating `/cdn-cgi/challenge-platform/` or
  `challenges.cloudflare.com` (zero-build test: disable adblock for that profile;
  durable: an `ignore-previous-rules` allowlist entry); and **a profile whose
  write-lock is held elsewhere silently opens EPHEMERAL**, so cookies never
  persist and the challenge loops forever — make that refusal open the jar
  READ-ONLY as its own comment claims, and stop degrading silently. Nothing is
  diagnosable today because no main-frame load status is traced.
- **False/stale gates.** `runtime_status_handoff_active()` is
  `preserved_terminal_owner_count > 0` — a STEADY STATE, true for 65+ h because
  two sessions are parked on an older daemon, so any mount arms the veil. Arm on
  a genuine daemon-IDENTITY transition instead (`pid:version` differs from last
  observed) and let the awaiting-key slice only SCOPE which surfaces are veiled.
  The notice is also raised unconditionally — it never consults
  `active_view_mode`, which is why it covered a yedit document and claimed "the
  terminal is paused". ⚠ **The self-check must run IN-PROCESS on the 2.5 s tick:
  `server app state` REFRESHES the observation, so an external probe cannot
  measure staleness and can itself arm the gate.**
- **yedit: the wrap gutter drifts, and chrome draws over text.** The gutter takes
  one fractional `getComputedStyle(...).lineHeight` and uses it for BOTH the
  per-entry height and the row count, so error accumulates down the file — emit
  one gutter block per LOGICAL line at the MIRROR child's measured
  `offsetHeight`, and make the gutter self-verifying (sum === `scrollHeight -
  padding`; on mismatch stop drawing numbers and stamp an observable field). The
  "Document | Terminal" pill floats over an editor with no reserved space —
  recommended fix is to move it into the titlebar's existing surface-switch slot
  and delete both floating pills. Toasts need an anchor owned by the active
  viewport kind (top-center over a terminal, bottom-right over a document).

## ⭐ USER-REPORTED, ychrome round 2 (2026-07-31)

**Five of these are CLOSED by the user's own eyes** and removed per this file's
rule. What they were, so a reader of the git log can find the fixes: the F11
fullscreen trap with no escape (same deaf-chord root cause as the missing
Ctrl+F — claimed at the toplevel window, not per-surface); Ctrl+F itself;
"the viewport recomputes / ychrome does not sit flush" (under-glass is now the
DEFAULT, `5b0280a` — no flag); clipboard image paste (WebKitGTK never puts the
image in the paste event's DataTransfer, proven in a vanilla webkit2gtk process,
so a shim re-delivers it); and screen tearing, which was **XWayland** — see
[[finding-yggterm-must-run-wayland-native]], and note the 21-arm/1,190-frame
detector found nothing precisely because on Wayland there is nothing to find.

**Shipped but NOT yet confirmed by the user:**

- **Fullscreen video drew the chrome over the picture.** Fixed and pixel-proven
  (172,645 chrome pixels over a flat test page → **123**, all inside the 10 px
  corner radius; restore after a distraction-free → fullscreen → exit cycle is
  zero differing pixels). ⚠ **This was a regression from making under-glass the
  default**, and the shape is worth remembering: `web_surface_place_page_rect`
  had promised for months that "a page on the whole screen owns every pixel of
  the window" and delivered only the GEOMETRIC half — it suppressed chrome
  *claims* and never stopped the shell *painting*, because an opaque page above
  the DOM used to occlude chrome for free.
- **Tab placement + context menu.** One owner (`web_tab_placement`) replaces
  three independent `push` sites; spawn-below-opener with cascade; omnibox focus
  on `foreground && Blank` only, so a middle-clicked link never steals the caret.
  ⚠ **My screenshot diagnosis was WRONG and the lane corrected it**: the menu was
  not clipping off-screen (it sat 12 px clear of the edge, flipping correctly) —
  `RowMenuItem::disabled` was appending its *reason* to the *label*, so
  `Close tab` became a 60-character sentence in a 216 px box. Six existing locks
  had been asserting the reason belongs in the label, pinning the bug in place.

**STILL OPEN:**

- **★★★ ychrome's userscripts never inject, so YouTube plays ads and
  SponsorBlock is silent.** Lane `lane/dev/userscript-injection`.
  **Proven live**: yggterm's own shims (`__yggtermClipboardImagePasteShim`,
  `__yggtermCloseShim`, `__yggtermScrollNavShim`, `__yggtermThemeColorShim`) are
  ALL present in the page while `window.__ytAdDefense` and `window.__ysb` are
  `undefined` and `window.fetch` is still native. The policy endpoint serves all
  five scripts correctly (right worlds, right matches, `document-start`) and the
  **adblock half of the same policy works** — the 146,748-rule set compiled and
  attached (95 MB). So the loss is specific to the userscript path.
  ⚠ **Two of my theories were falsified**: "born before policy, so recreate it"
  (the user reopened the tab and ads returned) and "re-arm exhausted
  `policy_attempts` when the app respawns" (deployed, no effect — kept as an
  unmerged candidate patch, not discarded, since it may be a real latent bug).
  ⭐ Strongest untested suspicion: **a NAME is what makes a world isolated** —
  the plain userscript constructor IS the main world, and a NULL world name
  fails `assertion 'worldName' failed` and refuses the script. The ychrome
  engine lane hit exactly that. Check the attach path first.
  Note YouTube pre-rolls come from **googlevideo.com, the same host as the
  video**, so no network rule can touch them; the main-world `adPlacements`
  strip is the only defence, which is why this presents as "adblock is broken".

- **open-webui is slow when switching chats from its sidebar.** Untouched, and
  deliberately NOT folded into the launch-speed work: that is SPA navigation, no
  new process and no page load, so the ~650 ms WebProcess startup finding does
  not explain it. Needs its own diagnosis.

- **Webapp launch speed.** Root-caused, partly fixed. **Caching was the wrong
  hypothesis** — the HTTP disk cache already works and contributes 0 ms (0 bytes
  off the network on a warm arm). ~650 ms is WebKit **WebProcess startup**, per
  surface, and a second surface in a live process does NOT skip it, so
  prewarming one spare would not help. Second term is JS parse/compile with no
  code cache (~174 ms) — **JavaScriptCore has no persistent bytecode cache in
  the GTK port at all**, so that gap versus Chromium's V8 code cache is
  structural, not a setting. The adblock compile (17,180 ms → 3.7 ms via
  load-first keyed on a content hash) IS fixed. PSON / `WebProcessCache` is the
  only route to the 650 ms and is deliberately untouched: `process-swap-on-
  cross-site-navigation-enabled` is construct-only and the GTK constructor
  shadows it, and `prewarmGlobally()` is Cocoa-only.


- **★★ `web do click --css` MIS-CLICKS A HIDDEN DUPLICATE — the same ancestor-hit
  bug the engine just fixed, still live in the surface plane** (found 2026-07-31
  by `lane/dev/engine-click-hittable` while fixing ychrome's copy; **not fixed
  here**, because three lanes were editing `shell.rs` at the time).
  Two defects, both in `crates/yggterm-shell/src/shell.rs` — grep the symbols,
  the numbers move:
  1. `var onTarget = hit===el || (el.contains&&el.contains(hit)) || (hit&&hit.contains&&hit.contains(el));`
     (was ~`:57306`). ⛔ **`<body>` contains every element on the page**, so the
     third clause accepts *any* candidate on *any* normal page. A click that
     lands on an ANCESTOR reaches the ancestor, not the node you named. Drop the
     clause: `hit === el || el.contains(hit)`.
  2. `web_css_matcher_js` (was ~`:57378`) does `querySelectorAll(sel)[nth]` with
     `hidden:0` **hard-coded and no liveness filter at all**. Only the *Role*
     matcher runs `__yggLive`.
  Net effect: on a page with a `visibility:hidden` duplicate ahead of the real
  control, the decoy passes `visible` (it has a real rect) AND passes `onTarget`
  (body contains it) and gets the click — reported as success. The plane gets
  the `0x0` case right and gets role/text targets right; **its CSS path is
  wrong.** The engine's fixed resolver (`ychrome` `c547aa6`, `src/engine/hit.rs`)
  is the reference: classify with the liveness predicate, pin, `scrollIntoView`,
  let the scroll settle, re-measure, then require `hit === el || el.contains(hit)`.
  ⚠ Keep the refusal vocabulary identical across both planes
  (`no_hittable_match`, `detached_node`, `target_moved`, `zero_size_element`) —
  two planes with different words for the same refusal is the divergence
  AGENTS.md forbids, and avoiding it is why the engine borrowed these names.
- **Adblock/SponsorBlock not exhaustive; YouTube ads played at 2x.**
  `lane/dev/adblock-exhaustive`. ✅ **The 2x symptom is root-caused and cured on
  guihost (2026-07-31).** The deployed `youtube-adblock.js` was the pre-`d05a871`
  copy lacking its `// ==UserScript==` block, so `@world main` defaulted to
  `Isolated`, where its `window.fetch`/XHR/`ytInitialPlayerResponse` patches are
  invisible to the page — leaving only the fallback that sets
  `playbackRate = 16` (WebKit clamps it, hence "2x"). `idcac.js` and
  `sponsorblock.js` were stale the same way; all three redeployed. ⚠ Injection
  is per-webview at creation, so an existing tab keeps the script it was born
  with — a NEW tab is required, not a reload.
  **Still owed (the durable half):** a freshness check with ONE owner so a
  bundled asset newer than the installed copy cannot sit dead; a LOUD refusal on
  an unparseable/absent metadata block instead of the silent `Isolated` default
  that caused this; and a real filter-list pipeline — today's ruleset is
  `assets/web-adblock/rules.json`, **10 KB / 59 hand-written domain regexes plus
  exactly ONE `css-display-none` rule with 8 selectors, referenced by no code
  path at all** (a human must `cp` it into `~/.yggterm/web-adblock/`; done on
  guihost, never on dev). No ABP/uBO syntax parser exists anywhere — no `##`, no
  `##+js()`, no `$redirect=`. WebKit content blockers offer no redirect action,
  so surrogates are impossible and untranslatable rules must be COUNTED and
  reported, never silently dropped. SponsorBlock EXISTS and is real; idcac is a
  hand-written ~140-line approximation, not the upstream ruleset.

  ✅ **The RE-REPORT (ads still playing after the ychrome-side cure) was a
  SECOND, yggterm-side bug, root-caused and fixed on `lane/dev/userscript-injection`
  (2026-07-31).** Nothing was wrong with the wire, the engine, or the scripts:
  `/policy` served all five with the right `@world`/`@match`, and staging them on
  a webview through the vendored wry injects all four placement quadrants
  (verified against the byte-identical live wire on a standalone WebKitGTK
  harness). The break was upstream of all of it — the session had **no sidebar
  contribution at all**, so `web_surface_policy_gate()` answered `Absent` and
  every webview it ever built got `userscripts: []`, no ruleset, no UA and no
  signer bridge. `app_surface_restore_targets` asked the daemon ONCE per
  (session, PTY pid) whether an app had declared; a row exists ~3 s before
  `ychrome` declares, so the one ask legitimately missed and was never repeated.
  It is per SESSION, which is why closing and reopening the TAB never helped.
  Fixed by a backoff re-ask (2.5 s doubling to a 60 s ceiling) and by splitting
  the candidate filter's AND over the rail and web halves — a session that had
  its web surface but not its contribution was excluded from the sweep forever.
  ⚠ **Two instrument traps this cost a day to:** `window.__ytAdDefense` and
  `window.__ysb` do not exist in any script (the real globals are `__yga_*` and
  `__ysb_*`), and an isolated-world global is invisible to `web eval`, which runs
  in the page's world — so probe main-world scripts by their global and
  isolated-world ones by a DOM effect.

- **The WPE agent engine.** `lane/dev/wpe-engine-phase-a`.
  ⛔ **The spec's core premise is factually wrong and §3 has been corrected:
  Debian's WPE WebKit 2.52.5 ships NO WPEPlatform at all** — not "gaps in it".
  Independently verified: `wpe-platform-1.0`/`-2.0` `.pc` absent, zero headers
  declaring `wpe_display_headless_new`, **zero** `wpe_display` symbols exported
  from `libWPEWebKit-2.0.so.1`, and `/usr/include/wpe-webkit-2.0/` holds only
  `jsc` and `wpe`. WPEPlatform is an upstream build flag Debian leaves off, so
  the version number in the spec was doing all the persuading and none of the
  deciding. §9's sanctioned fallback fired: the substrate is **WebKitGTK + an
  engine-owned Xvfb** behind the same verbs, and it delivered the two things the
  risk register feared it would not — trusted input and faithful snapshots.
  Phase A gate PASSES on dev (`ychrome engine gate`, five journaled proofs,
  re-runnable). Bindings decision recorded in a new §9.1: **the gir crates**, no
  bindgen, no `build.rs`. Phase B started: `/engine/*` router, 10 pages opened
  concurrently in 1340 ms. Owed: `/nav` `/wait` `/dom` and the input events
  (refused BY NAME today, never silently dropped), the socket plumbing is
  unit-tested but never run against a deployed daemon, the gate has not been
  re-run on guihost, and phases C/D/E remain. **Phase F stays out of scope.**

- **⚠ `WEBKIT_DISABLE_COMPOSITING_MODE=1` BREAKS THE WEB SURFACE OUTRIGHT** (found
  2026-07-31 by the tearing lane, reproduced twice: the page never appeared).
  This is the **top-precedence GL escape hatch** per `docs/optimization-pass.md:222`
  (`WEBKIT_DISABLE_COMPOSITING_MODE` › `YGGTERM_FORCE_SOFTWARE_GL` ›
  `YGGTERM_ENABLE_WEBKIT_COMPOSITING` › the EGL child probe), so the documented
  way to force software presentation currently costs the user every web surface.
  Either fix it or stop documenting it as the escape hatch — a hatch that
  destroys the thing it is meant to rescue is worse than none.

- **⚠ THE GL PROBE NEVER RUNS ON THE LIVE HOST.** guihost reports
  `webkit_gl_policy: hardware_gl_forced`, not `probed`, because
  `YGGTERM_ENABLE_WEBKIT_COMPOSITING=1` sits in the launcher env and outranks
  the probe. Any reasoning that assumes the live host measured its own GL is
  wrong. Noted while investigating tearing; not itself known to be a defect,
  but it silently invalidates a premise agents keep reusing.

- **⚠ FLAKY UNDER LOAD: `render_probe::tests::process_still_running_answers_from_proc_and_refuses_a_recycled_pid`**
  (seen 2026-07-31 at load average **37.5**, with seven lanes building in
  parallel). It spawns `sleep 30` and asserts the child reads as running;
  the failing assertion is `render_probe.rs:1905`, the FIRST one. Re-run in
  isolation: **5/5 green**, and the full suite was green 20 minutes earlier, so
  it is load-induced, not a regression — no lane touched this file.
  **Not diagnosed, and worth diagnosing rather than retrying.**
  `process_still_running` (`render_probe.rs:741-749`) is narrow: it fails only if
  `/proc/<pid>/stat` is unreadable, `stat.pid != pid`, `stat.comm != comm`, or the
  state is `Z`/`X`. The test reads `comm` from `/proc` itself at :1901 and then
  passes it straight back, so a comm mismatch implies the value CHANGED between
  two adjacent reads — i.e. the first read caught the child mid-`spawn`, before
  `exec` replaced its `comm`. If that is it, the product code is fine and the
  TEST is racy; but this is a **liveness probe** in a project whose central
  lesson is that instruments lie, so "probably just flaky" is not an acceptable
  resting place. Prove which side is wrong before trusting either.

## Standing traps / other open bugs

- **★★ "YCHROME SUDDENLY QUIT TO TERMINAL" — a fleet binary deploy arms a
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

- **★★ THE YCHROME VIEWPORT Z-ORDER — UNDER-GLASS ARMED ON THE LIVE HOST,
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

- **★★★ REMOTE ROWS WEDGE IN `RemoteBootstrap` AFTER A DAEMON VERSION HANDOVER
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

- **★ THE SUPERVISOR DIES WITH ITS CHILD — confirmed twice in one day (guihost,
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

- **★★ WEBAUTHN / PASSKEYS ARE UNREACHABLE ON AN AGENT-CREATED SURFACE
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

- **★★ AGENT CO-BROWSE CANNOT COMPLETE AN OTP LOGIN — the logged-in plane stops
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

- **★★ THE DAEMON'S ENVIRONMENT IS FROZEN AT LAUNCH AND POISONS EVERY SESSION IT
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
  fresh environment instead of copying its own? guihost and dev daemons are
  currently clean, so this is latent everywhere, live nowhere.

- **★★ `web ensure` MINTS ONE WEB PROCESS PER TAB, revealed or not (measured on
  guihost, 2.12.17, 2026-07-27 — J8a).** The docs promise "thirty rows, not thirty
  webviews", and the RESTORE path honors it (a never-selected restored tab has
  no webview) — but one `web ensure` on a declared surface builds a
  `WebKitWebProcess` for EVERY tab, on a surface never revealed and never
  visited. The law is exactly linear: processes = tabs + 2, ~108 MB RSS /
  18.4 MB PSS per webview (PSS floor, trivial static fixture), so 100 tabs =
  102 processes / ~11.4 GB RSS in one call. 2.12.18's per-tab reclaim drains
  the pile after the hold (5 s under pressure), but the MINT-time spike is
  unbounded. Fix: ensure mints the ACTIVE tab's webview only; the rest stay
  tab-model-only until revealed/selected (the restore path's exact rule), plus
  a live-webview LRU budget so no path can pile past a cap. Evidence:
  `~/.local/share/ygg-j8-baseline/` on guihost.
  **CONFIRMED STILL OPEN on 2.12.18 (guihost, 2026-07-27 — J8b):** 25 tabs seeded
  and `ensure`d on a surface that was never revealed → **27 GUI web processes
  before anything was shown**. The per-tab hold governs background tabs of a
  session the user IS looking at, so it never fires here; the mint-time spike is
  untouched by the reclaim lane. Lazy-ensure is the outstanding half.

- **★★ A SECOND VIEWER DOUBLES EVERY WEBVIEW, AND `session remove` STRANDS THE
  SHADOW'S SET FOREVER (guihost, 2.12.17, 2026-07-27 — J8a).** Webviews are
  per-CLIENT: revealing a 10-tab surface on a shadow client built a second full
  set (11 more processes). Then `session remove` answered `verified:true`,
  reaped the ACTIVE client's webviews, and left the shadow's **21 webviews
  (2.3 GB)** alive with no row anywhere — only `shadow-client.sh stop` freed
  them. Same family as the remote-cc entry below: the teardown verifies one
  side and claims the whole. Fix: the remove path must sweep every client's
  applied set for the session, or refuse with the shadow named.
  **REPRODUCES on 2.12.18 (guihost, 2026-07-27 — J8b).** Two fixture sessions
  removed, both `verified:true` with reaped pids named: the GUI fell to **1**
  webview while the shadow kept **3** (952 MB total) for rows that existed
  nowhere; `shadow-client.sh stop` freed them (4 → 1, 952 → 495 MB). Smaller
  only because per-tab reclaim had already collapsed most of the set — the
  defect itself is unchanged.

- **GUI process died mid-J8a with 51 webviews applied (guihost, 2.12.17 GUI 27779
  → fresh 325652 at 12:17:22, 2026-07-27). Cause UNDETERMINED** — no panic in
  the trace, no readable OOM record; the 50-webview ramp stage had completed
  one minute earlier, so the correlation is owned, not proven. The daemon
  never blinked and every row survived (the constitution held). Watch item:
  if a fresh GUI dies again near a large applied-webview count, this becomes
  the top entry; the webview budget above is the mitigation either way.

- **`scripts/shadow-client.sh` is broken for every in-session agent (guihost,
  2026-07-27 — J8a).** The daemon exports `YGGTERM_BIN=<yggterm-headless>`
  into rows it owns; the script defaults through `YGGTERM_BIN`, so inside any
  daemon-owned row it launches the headless binary and dies with "only
  supports server subcommands". Workaround, verbatim:
  `YGGTERM_BIN=$HOME/.local/bin/yggterm scripts/shadow-client.sh …`. Fix: the
  script must refuse a headless binary (probe `--version` output) or default
  to the GUI binary path explicitly.
  **STILL OPEN on 2.12.18 (guihost, 2026-07-27 — J8b):** `/proc/<shell>/environ`
  of a daemon-owned row still carries `YGGTERM_BIN=/home/user/.local/bin/yggterm-headless`.
  ⚠ Verify this one from `/proc`, not from `echo $YGGTERM_BIN` after an `unset`
  in the same shell — that self-polluted probe reads "fixed" and is a lie.

- **The profile PICKER CARD is unreachable from the agent control plane (guihost,
  2.12.18, 2026-07-27 — J8b).** 2.12.18's avatar/permanence verbs — "Change
  avatar…", "Use the default avatar", "Protect profile" (disabled reason
  *"default is always protected"*) — live only on the picker card's row menu
  (`web_profile_menu_items`, ids `web-profile-change-avatar` /
  `web-profile-protect`). Nothing an agent can drive reaches that surface: the
  rail/strip badge opens the profile SWITCHER menu (`webprofile:<name>` entries
  only), a sidebar profile chip opens the shared session row menu,
  `server app command list` carries **0** profile/avatar commands, and
  `server app start-page` reports no profile cards. Consequence: the avatar
  PERSISTENCE contract — a "Change avatar…" write must preserve unknown sidecar
  keys such as `agent_drive` — **could not be live-verified at all**, and it is
  the one clause of the 2.12.18 maiden-run checklist with no live proof. Fix:
  give the picker an addressable entry point (a command-plane id, or a
  documented route), or expose the avatar/protect writes as `server app` verbs.

- **`WebKitNetworkProcess` accumulates per profile churn (guihost, 2026-07-27 —
  J8a: 3 → 10 across one baseline run).** One network process per WebContext
  is the design; contexts for torn-down profiles are not always reaped with
  their last webview. Small (network processes are lighter than web
  processes), but it is a leak shape — audit `web_context_key` retirement.

- **★★ REMOTE-CC `session remove` REPORTS `verified:true` WHILE THE REMOTE AGENT
  KEEPS RUNNING (found + reproduced end-to-end on guihost, 2.12.17, 2026-07-27).**
  Removing a `remote-cc://` row reaps only the LOCAL ssh client and still
  answers a clean verified removal:
  ```
  verified:true   live_processes:[]   row_still_listed:false
  reaped_processes:[{"command":"ssh","pid":<local ssh client>}]
  ```
  The remote agent process was still alive on the remote host 90 s later, with
  no row anywhere pointing at it. **Root cause, and it is not a race:** the
  orphan's parent is the REMOTE host's own `yggterm-headless server daemon` —
  the remote runtime is deliberately daemon-owned so it survives ssh drops, so
  the local remove never asks the remote daemon to close its runtime. The remote
  daemon is left holding a live runtime for a row that no longer exists.
  **Why this one matters:** the teardown-honesty contract says a report must be
  verified or honestly refuse, and the LOCAL path already implements it
  perfectly — a local agent row removal names every pid it killed (shell, agent,
  and the agent's own MCP children) and a local shell removal names the tenant
  it reaped. The remote path simply does not cross the machine boundary, yet
  claims the same verification. Fix: proxy the close to the owning remote daemon
  and verify there, or refuse with a named reason when the remote half cannot be
  confirmed. **Never report `verified:true` for work done on only one side of an
  ssh hop.** Repro + evidence: guihost queue §DONE "J7 step 4 / Defect B".

- **`server app open` on a REMOVED row times out instead of naming the reason
  (minor, guihost 2.12.17, 2026-07-27).** Opening a deleted session path correctly
  does NOT resurrect the row and correctly leaves the active session untouched
  (no select/activate events fire), but the CLI answers
  `Error: timed out waiting for app open to settle …`. Compare `web ensure` on
  the same class of dead path, which is exemplary: `accepted:false`,
  `reason:"session_closed"`, `row_close_remembered:true`, plus prose naming why
  and what to do instead. Make `app open` refuse in that shape rather than
  time out.

- **★★ AGENT-SPAWNED TENANTS INSIDE DAEMON-OWNED ROWS ARE IMMORTAL — the leak
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

- **★★★ NOTIFICATION AUDIO IS SILENT IN THE WEBVIEW — PROVEN BY A/B ON THE
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


- **★★ AN AGENT'S TEARDOWN CAN REPORT SUCCESS AND LEAVE BOTH THE ROW AND THE
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


- **★★★ AN UNREVEALED AGENT SURFACE REPORTS `visibilityState: "visible"`, SO
  ITS PAGE ANIMATES AT FULL RATE AND THE GUI COMPOSITES IT — measured
  2026-07-26 night, and this is very likely THE mechanism behind every
  "agents make the GUI host hot" report in this campaign. ⏳ FIXED IN-TREE AT
  2.12.17; THE LIVE A/B IS OWED AND IS THE ONLY THING THAT CLOSES THIS.**
  Ground truth: a payment-gateway page on a headless, never-revealed surface
  the user cannot even see (no row — see the entry above) reported
  `visibilityState: "visible"` with **1 running animation** (a spinner). Cost,
  measured over 20 s from `/proc` (never `ps %CPU`): **web content 0.241 cores
  + GUI 0.399 cores = 0.85 cores total against guihost's ~0.5-core idle floor**,
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

- **★★★ A LIVE, LEASED WEB SURFACE CAN EXIST WITH NO ROW — the user cannot see
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

- **★★ `web fill-card` ADVErecordsSED WHAT THE CREDENTIAL PLANE FORBADE (found live
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

- **★★ A `--no-activate` CREATE MADE WHILE NO SESSION IS ACTIVE STILL ACTIVATES
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

- **★★★ `web do` FIDELITY ON RE-RENDERING DOMs — three reproducible defects,
  one family (a live portal filing run, 2026-07-26 ~15:30-16:00 IST, guihost 2.12.15,
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
  ⚠ **Live verification is OWED**: no live portal re-run, no guihost deploy. The
  daemon/GUI on guihost still runs the old behaviour until the next bump.
  Remaining agent-side: the INVENTED characters in (1) are still attributed to
  the concurrent focus-theft bug — the readback now catches them, it does not
  prevent them.

- **★★ THERE IS NO CLIENT TO RENDER AGENT SURFACES INTO ON dev (2026-07-26).**
  The data-fabric default "co-browse on a SHADOW surface on dev" is currently
  unusable: `server app clients` on dev → count 0 (no GUI, no shadow client),
  so the filing agent had to fall back to the user's live GUI host. Fresh
  evidence for settled call #6 (drive shadow surfaces with the GUI closed /
  server-side rendering, docs/optimization-pass.md WS2): today agent browsing
  physically requires the user's GUI host.

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

     **LEVEL (a) IS BUILT — ⚠ NOT LIVE-VERIFIED (the deploy happens after the
     lane that wrote it, so this entry STAYS until a real bump proves it).**
     What the cause chain actually was, and where each half is fixed:
     - The predecessor advertises its ROUTINE persist as `live_terminal_sessions`,
       and a routine persist synthesizes no `restore_reason` — so the successor
       adopted the shell (correctly: the peer still owned its PTY, which is the
       `peer_live_row_is_adoptable` rescue arm) but the adopted row carried no
       record of WHY it was there. Fixed by `peer_live_rows_marked_as_rescued`
       in `adopt_missing_live_session_rows_from_reachable_daemons`: rows the
       peer still owns are stamped with the handover restore reason before
       admission. The ownerless-agent arm is deliberately NOT stamped.
     - Then, the moment the predecessor retired and the PTY died,
       `apply_terminal_runtime_truth_to_snapshot` erased the row: a shell has no
       agent-store arm to fall back on. Fixed by
       `snapshot_session_is_handover_orphaned_row` — a row that crossed a daemon
       handover survives its runtime as a RUNTIME-LESS row
       (`TerminalLaunchPhase::RemoteBootstrap`, the same shape agent rows
       already use), and the click resolves through the ordinary
       `terminal_spec` → shell launch command at the recorded `Cwd`. Scrollback
       is lost at this level; the row, title, cwd and POSITION are not.
     - The discriminator is deliberately narrow: **a shell whose own PTY exited
       (the user typed `exit`) is still a husk and still disappears.** That is
       the same class `peer_live_row_is_adoptable` refuses, and widening the
       arm to "any Shell" would put guihost's three ownerless loopback shells
       (`local://3803a7ed`, `local://5220ce5d`, `local://a689ee28`) back on
       screen. It also does NOT weaken keep-alive: `PrepareClientClose` still
       removes a non-keep-alive shell from the live order outright, so a GUI
       close never reaches this filter.
     - **Known residual, level (a):** the mark is only re-applied on an
       update-restart persist and on a rescue adoption. A daemon killed WITHOUT
       a handoff (`kill -TERM`, a crash) writes only routine persists, so a
       shell row cold-restored from that file has no mark and is still hidden.
       Closing that means letting the routine persist carry the row's existing
       `Runtime Restore Reason` metadata instead of re-deriving it — cheap, but
       it also widens `live_session_restart_protected`, so it was left out
       rather than guessed at.
     - **Second residual:** nothing CLEARS the mark once the successor spawns
       its own PTY for the row, so a rescued shell the user later exits keeps a
       runtime-less row instead of becoming a husk. That reads as correct under
       call #4 ("no runtime is none of our business") and as a husk under
       requirement 3 below — which is exactly the question requirement 3 says to
       ask the user rather than guess. Clearing it at the `ensure_session`
       chokepoint is the obvious fix once that answer exists.

     **LEVEL (b) — LOSSLESS `SCM_RIGHTS` FD HANDOFF: where it would slot in.**
     ⏳ **INCREMENT 1 IS MERGED (2.12.17); INCREMENT 2 IS NOT BUILT.** Nothing
     is wired into the handoff yet, so level (b) changes no behaviour today —
     no PTY has ever moved. The map below is unchanged and is still the sizing
     document.
     - **Increment 1, merged — the child handle learns Owned vs Adopted.**
       `PtyChildHandle` is `Owned(Box<dyn Child>)` vs
       `Adopted { pid, start_time }`, and every call site is taught which it
       holds: `is_running()` replaces `try_wait().is_none()` everywhere, because
       the old shape forced every caller to think in exit statuses, which an
       adopted child can never supply. Three rules are enforced rather than
       described — an Adopted child NEVER reports an exit status (fabricating a
       success would be worse than returning nothing); killing it is explicit,
       since dropping the master only SIGHUPs the foreground group; and identity
       is **(pid, start_time)**, never the pid alone, gating SIGNALLING as well
       as reporting, which is the assertion that actually prevents killing a
       stranger after PID reuse. Found while building, not in the spike: an
       adopted child has a ZOMBIE WINDOW nothing reaps on our behalf, so `/proc`
       state `'Z'` must read as dead or every shutdown path waits out its full
       timeout on an already-dead process. `ReceivedMasterPty` — the master type
       `portable_pty` cannot build (`UnixMasterPty` and `PtyFd` are private and
       `openpty()` always creates a NEW pair) — is in-tree and under test but
       deliberately unused: `F_DUPFD_CLOEXEC` never plain `dup` (a plain `dup`
       leaks the master past exec, the slave's hangup never arrives and the
       shell never sees EOF), `EIO` mapped to EOF exactly as `PtyFd` does, and
       dropping the writer sends newline + the termios `VEOF` byte so the
       trait's documented EOF contract still holds. The adoption machinery is
       Linux-gated **at the variant**, so the module compiles on every target
       rather than only the one it was written on.
     - **Increment 2 — the `HotRestart` `sendmsg` wiring — is integrator-gated
       and NOT built.** Two decisions are already settled and should not be
       re-litigated when it is: the transcript payload travels BEFORE the fd,
       and **`sendmsg` success is the commit point** — after it the fd belongs
       to the successor, so nothing downstream may be recovered by re-sending.
     - **Who owns the fd.** `PtySessionRuntime` in
       `crates/yggterm-server/src/terminal.rs` holds
       `master: Arc<Mutex<Box<dyn MasterPty + Send>>>`, and
       `TerminalManager { sessions: HashMap<String, PtySessionRuntime> }` is the
       map keyed by runtime key. The raw fd is already reachable —
       `master.as_raw_fd()` is what `foreground_process_group_leader` uses for
       `tcgetpgrp` — so the SEND side needs no new plumbing into the pty layer.
     - **Who owns the child.** `PtySessionRuntime.child:
       Arc<Mutex<Box<dyn Child + Send + Sync>>>`. This is the part that does not
       travel: a `Child` handle cannot cross a process boundary, and the shell
       is the predecessor's direct child. After the fd moves, the successor can
       drive the PTY but cannot `waitpid` it; the predecessor must either stay
       alive as a reaper until it exits (defeats the point) or the child must be
       re-parented to init and the successor must fall back to
       `kill(pid, 0)` / `/proc` liveness. **Decide this before writing any
       `sendmsg`** — it is the actual design question, not the ancillary data.
       ✅ **DECIDED and built in increment 1:** re-parent to init, no lingering
       reaper, `/proc` liveness keyed on (pid, start_time).
     - **Who owns the scrollback.** The reader thread plus `chunks`,
       `seq`, `retained_bytes` and `spawn_id` on the same struct. The fd alone
       hands over a live terminal with an empty transcript, so the ring has to
       travel beside it (the existing `terminal_snapshot` payload is the obvious
       carrier) or the user gets a working shell with no history — barely better
       than level (a).
     - **Where `sendmsg` would live.** The wire is one JSON line per request over
       a `UnixStream` (`read_request` / `write_response` in `daemon.rs`), which
       has no room for ancillary data — `SCM_RIGHTS` needs a real `sendmsg`
       on the same socket, so this must be an out-of-band step on the handoff
       connection, not a new `ServerRequest` field. The natural site is the
       `ServerRequest::HotRestart` preserving-handoff branch in `daemon.rs`,
       immediately where it calls `PreservedTerminalOwnerRegistry::write_handoff`
       — that registry (`hot-update-terminal-owners.json`, runtime key → owner
       socket + pid) is already exactly the list of fds that would be sent, and
       `attempt_self_retire_preserving_handoff` is the caller that reaches it on
       a `disk_binary_replaced` retire.
     - **The receive side is the expensive half.** `portable_pty`'s `MasterPty`
       is a trait object with no `from_raw_fd` constructor, so the successor
       cannot rebuild a `PtySessionRuntime` from a received fd without either a
       local Unix master type implementing the trait or a fork of the pty layer.
       Budget the work there, not in the socket call.
     - **What it would retire.** `session_kind_is_migratable_agent` could then
       admit `Shell`, and `progressive_migration_session_released` would stop
       being kill-and-re-resume for every kind — which is also what unpins the
       supernumerary daemons that one idle `bash -i` keeps alive forever.
  2. **THE ROW-ORDER LEDGER WAS WRITE-ONLY ON RESTORE. ✅ FIXED AND PROVEN LIVE
     ACROSS THE 2.12.16 DAEMON BUMP — 22 rows before, 22 after, ORDER IDENTICAL,
     and the pre-swap receipt was written.** That was the maiden constitution
     deploy and it is the proof this clause asked for; the mechanism below is
     kept as the record of WHY it holds. Every future bump re-proves it silently
     through the J-battery, so a bump that scrambles the order is a REGRESSION,
     not a fresh discovery.
     The original defect, for the record: verified across the 2.12.15 bump, the
     ledger was byte-identical before and after (143 entries, the user's curated
     order intact) and *nothing read it back*. Restored rows land first, adopted
     live rows are appended after, so the user's two live sessions moved from
     positions 1-2 to 6-7 and they had to re-curate by hand for the third time
     in a day.
     What now exists (`crates/yggterm-server/src/row_order_ledger.rs`):
     - **The restore.** `reconcile_order_with_remembered` is the one owner of
       the rule — rows the ledger knows take the ledger's relative order; rows
       it has never seen keep the slot the anchored import walk
       (`import_peer_live_rows_in_order`) gave them, still under the same
       neighbour. Both handover rebuild passes
       (`run_deferred_preserved_owner_deep_reconcile` and
       `takeover_superseded_daemon_state`) end by applying it, **before** their
       own persist — `persist()` records the live order INTO the ledger, so
       persisting the freshly-imported scramble first would erase the very
       arrangement the restore reads. It reconciles against
       `DaemonRuntime::booted_with_row_order`, the ledger as this daemon booted,
       for the same reason.
     - **It cannot resurrect.** The reconcile is a permutation of the rows the
       daemon already holds, and `replace_live_session_order` separately refuses
       any path that is not already a row — two independent refusals, each
       locked by its own test, so a tombstoned row in the ledger stays out.
     - **The reorder verb exists and is fixed** (it had been added since this
       entry was written; the defect was that it ignored dormant rows and
       reported success anyway — field guide §4.5). It now moves dormant rows
       and answers with `applied` / `skipped` lists.
     - **The pre-swap receipt** lands at
       `~/.yggterm/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json`,
       written by the outgoing daemon on `PrepareUpdateRestart` and by the
       incoming daemon before it imports a row.
     **How it was closed, and how every bump re-checks it:** capture
     `server app rows` before and after the swap, confirm the order is
     unchanged, and confirm a `pre-daemon-swap-*` file appeared. Never on unit
     tests. (What this entry still does NOT cover: a plain shell's row surviving
     a bump — that is level (a) above, and it has not been exercised by a real
     swap yet.)
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
     **CAUSE (found, and it is not what the symptom says): the apps never
     closed.** They run in daemon-owned PTYs and survive the GUI fine. What dies
     is the CLIENT's memory of their surfaces — both `web_surfaces` and
     `sidebar_contributions` are built by an OSC 7717 parser that only exists
     while a terminal host is MOUNTED. After a relaunch the tables are empty,
     so every session paints the terminal surface. The OSC heartbeat cannot
     repair it: it reaches only a session whose host is mounted (so never a
     background row), and a two-tier app like yedit declares exactly ONCE and
     exits, so there is no heartbeat to catch at all. Both daemon-replay rebuild
     paths already existed — they were just wired to agent verbs only
     (`right-panel pane:<id>`, `web ensure --session`) and to nothing that runs
     on its own.
     **FIX (GUI-side, no daemon change, no version bump): `restore_app_surfaces_tick`**
     on the 2.5s working-flags poll tick drives those SAME two rebuild paths.
     Endpoint-probed liveness, never declare age (the rail half already gets
     this right — see "liveness is the ENDPOINT" below); a dead endpoint or an
     unanswerable preserved owner degrades to the terminal view with
     `daemon_declare_endpoint_dead` / `daemon_declare_unavailable` /
     `daemon_declare_absent` in the trace, never a blank surface. One ask per
     (session, `terminal_process_id`), so a handover that re-resumes a PTY
     re-arms it and nothing else becomes a per-tick daemon poll; 3 sessions per
     tick; active row first. It never activates a session, moves focus, or opens
     a rail — it restores surface STATE, which the user sees when they visit.
     ⚠ **NOT yet verified live on guihost.** Unit-locked only (4 decision locks +
     a wiring scan). The live proof owed: restart the GUI with a yedit and an
     ychrome session running, then confirm both come back on their own surface
     without a manual reopen, and that `app_surface_restore` appears in the
     trace.
  6. **★★★ AGENTS MUST DRIVE SHADOW SURFACES EVEN WHILE THE USER'S GUI IS
     CLOSED.** Felt concretely: two background filing agents each drove a ychrome
     session row and the GUI host burned. This is the same requirement as
     server-side rendering — agent browsing should never have been on the GUI
     host (docs/optimization-pass.md WS2, `ychrome/docs/agent-engine.md`).
     Wanted as a real feature, not a workaround.
  7. **DAEMON HANDOVER MUST TELL THE USER AND STOP DRAWING.** On a daemon
     version change the GUI host burns. Spawn a notification ("daemon is
     changing, please wait"), **stop drawing the terminal for the duration**, and
     entertain the user. The render cost during handover is the thing being
     avoided, so the fix is to stop painting, not to paint a spinner harder.

     ⏳ **BUILT, NOT YET LIVE-VERIFIED (GUI-side, no daemon change — it deploys
     without a version bump).** `crates/yggterm-shell/src/handover_gate.rs` is
     the one owner of the predicate, derived from the DAEMON'S OWN report
     (`preserved_terminal_owner_keys` — the keys it serves but a predecessor
     still owns), scoped to the runtime keys this client has mounted, and
     resolved when the successor adopts them. On the ON edge: a coalescing
     "Daemon updating" job notification, a static veil over the viewport, and
     the terminal read/write path stops — no daemon read, no `term.write`, no
     render-health sampling (so no recovery `redrawTerminal`), no visible-paint
     scheduling. Resume is the NORMAL read from the unchanged cursor, never a
     daemon-screen replay. Three fail-safes: the first observation is a baseline
     (a GUI starting beside a lingering preserved owner never opens veiled), an
     unreadable status resumes paint, and a 90 s ceiling ends any suspension and
     latches that handover so it cannot re-arm. Probe it at
     `server app state` → `handover_paint`; trace events
     `handover_paint_suspended` / `handover_paint_resumed` (component
     `daemon_handover`).
     **Still open:** (a) live proof on guihost across a real daemon bump — nobody
     has watched this fire yet; (b) detection latency is bounded by the
     runtime-status poll (10 s busy / 60 s idle), so the notification can land
     several seconds into the handover — a cheaper immediate trigger (e.g. an
     out-of-band status refresh on the read loop's cursor-rewind tell) is the
     obvious follow-up; (c) suspending reads for a long handover can overrun the
     daemon's 512-chunk ring, which lands on the existing `resync_required`
     path (scrollback-preserving screen reconcile), unchanged by this lane.
  8. **AUDIO NOTIFICATIONS NEED A PRE-ROLL.** Bluetooth speakers clip roughly the
     first ~300 ms while the link wakes, so the start of every notification is
     lost. The user suggested ~150 ms and invited a better figure. **Use
     very-low-amplitude noise, not silence** — many A2DP stacks drop or fail
     to prime the link on pure digital silence, so the pre-roll needs a little
     real energy (a dither-level noise floor is enough to be inaudible). Better
     still, make it adaptive: skip the pre-roll when another notification played
     within the last few seconds, since the link is already awake.

     ⚠ **THE DESCRIPTION THAT USED TO STAND HERE IS SUPERSEDED (2026-07-27).**
     It recorded the first webview implementation — a 400 ms pre-roll on an
     `AudioContext` the emitter closed 80 ms after the last note. Both of those
     figures are gone, and so is the premise: **the webview never made a sound
     at all** (see the notification-audio entry above for the A/B), so the path
     is now native Rust and the shipped numbers are the MEASURED ones:
     - **Pre-roll 0.70 s**, flush tail **1.10 s**, TPDF dither at ~-57 dBFS.
     - **The dither spans the WHOLE render, not just the front.** The tune is
       mostly silence by duration (1.03 s inside a pair, 2.4 s between pairs),
       so a front-only pre-roll leaves every later note exposed to a sink that
       went back to sleep — which is exactly the reported ending-clip. Locked:
       no 50 ms window of any rendered tone is digitally silent.
     - **The context is long-lived**, not opened and closed per chime; the
       closing context was itself part of the clipped-tail report.
     - The registry `yggterm_core::notification_audio` owns pre-roll, tail and
       dither for BOTH players, so the native CLI and the webview script cannot
       drift into two different chimes.
     **What survives from the old description, still true and still wanted:** the
     adaptive skip is real in the GUI path — one owner
     (`NOTIFICATION_CHIME_LAST_PLAYED_MS`, written only by the emitter) through
     the pure `notification_preroll_decision(now, last_played)` with a 10 s
     `NOTIFICATION_PREROLL_LINK_AWAKE_WINDOW_MS`, plus a
     `notification_sound_preroll {applied, reason, tone, preroll_ms,
     since_last_ms}` trace row; and a notification with sound off emits neither
     chime nor pre-roll. The native CLI takes `--preroll on|off|auto` and `auto`
     resolves to ON, deliberately: there is no shared state across CLI
     invocations to remember with, and a wasted pre-roll beats a clipped alert.
     **Still to do, unchanged:** hear it on guihost through the user's Bluetooth
     speaker (the whole point is a physical A2DP link) — first note intact on a
     cold link, later notes intact on a warm one — and confirm
     `notification_sound_preroll` in `server trace tail`. Do not close this
     entry until that is done.

- **★★★ USER REQUIREMENTS FOR THE SESSION-ROW LIFECYCLE (stated 2026-07-26, after
  curating the list by hand TWICE).** The user's words: *"A daemon bump and
  restart should not destroy the row order and number of sessions. If destroyed
  this order is supposed to be snapshotted properly. And lastly all the rows not
  connected should die (gracefully is recommended)."*
  1. **A daemon bump must preserve row ORDER and COUNT.** ✅ Verified for a
     GUI-only restart 2026-07-26 (21 rows, byte-identical order across the swap,
     snapshot at `~/.yggterm/manual-snapshots/pre-gui-restart-*`) and ✅ **PROVEN
     ACROSS A REAL DAEMON BUMP on 2.12.16** — the case that actually breaks it,
     where rows are re-imported from peer daemons: **22 rows before, 22 after,
     ORDER IDENTICAL**. The anchored-placement fix
     (`import_peer_live_rows_in_order`) has now been exercised by a real daemon
     swap. Every later bump re-proves this silently through the J-battery, so a
     scramble is a regression to bisect, not a new finding.
     ⚠ **What is still NOT exercised: a plain shell's row surviving a bump**
     (level (a) in standing-traps item 1). The 2.12.16 proof says nothing about
     that half.
  2. **If order is destroyed it must be recoverable from a snapshot.**
     ✅ **BUILT AND PROVEN LIVE across the 2.12.16 bump** (order identical, and
     the pre-swap receipt was written).
     `~/.yggterm/row-order-ledger.json` records order+membership and
     `removed-rows.json` records closes; the original defect was that nothing
     RESTORED from them automatically, and an agent had to reconstruct by hand.
     Both halves now exist and both ran — the automatic restore on every handover
     rebuild pass, and the pre-swap receipt at
     `~/.yggterm/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json`.
     See standing-traps item 2 above for the mechanism.
  3. ✅ **"All rows not connected should die" — DECIDED by the user
     (2026-07-26, asked directly): "not connected" means rows that were
     explicitly CLOSED — by the user or by an agent.** It does NOT mean
     runtime-less rows (call #4 stands: never reap those). That is exactly the
     tombstone plane: both the GUI close and an agent's session-remove flow
     through the same daemon handler (`tombstone_live_row` before
     `remove_live_session`), so the requirement is implemented and was proven
     across the 2.12.15 bump (8 closed, 8 tombstones kept, 0 resurrected).
     One recorded nuance, deliberate: the `PrepareClientClose` non-keep-alive
     reap does NOT tombstone — that is contract death (second-class shells die
     with their GUI), not an explicit close; the import admission predicate's
     owns-runtime refusal is what keeps those husks from coming back.

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
  was set. Its comment justified this as *"guihost: AMD iGPU exposing only
  llvmpipe."* ⛔ **That premise is FALSE on guihost and has been for some time.**
  `eglinfo` platform matrix, guihost, 2026-07-25: GBM → `llvmpipe`, but **Wayland →
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
  drain at all. guihost only appeared to migrate because the middle daemon still
  had a thread alive from an earlier self-retire. **FIXED** — the accept loop
  now starts the drain on a preserving handoff, locked both directions.
  **STILL OPEN — the durable half.** `session_kind_is_migratable_agent`
  (`daemon.rs`) admits only `Codex | CodexLiteLlm | ClaudeCode`: a plain shell
  is not re-resumable, and there is **no fd passing anywhere in the tree**
  (`SCM_RIGHTS`/`sendmsg` -> zero hits), so the only way to move a PTY is
  kill-and-re-resume. Therefore **one idle `bash -i` pins its daemon at its
  birth version forever**, and the daemon can never reach empty hands:
  `daemon_should_idle_shutdown` refuses while any terminal session remains, and
  the stale-daemon sweep refuses a local shell. Live on guihost: three of the four
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
  Both 51-hour shells on guihost are that bug, not a policy gap. The right fix is
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
  live on guihost with a shadow `agent-r20`): the shadow's terminal lane dies on
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
  guihost: `right-panel pane:notes` on a shadow dispatched
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


- **★★★ THE FIFTH FOCUS PATH — IT IS NOT JAVASCRIPT. Root-caused 2026-07-26;
  ✅ THE FOCUS-BORROW FIX IS SHIPPED AND USER-CONFIRMED LIVE ON guihost. What keeps
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
  Live on guihost, 16:04: shell `hasFocus:false` / `activeElement`
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
  **Live proof (guihost, 2026-07-25):** launched through the shim (supervisor
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
  `preferred_executable_for(context, ...)` — on guihost that is the direct-install
  channel's recorded `active_version`, **2.9.48**, so a one-line desktop fix
  would silently replace the deployed 2.12.12 binaries with a script pointing at
  a months-old build. On a dev-deployed host, edit the `Exec=` line of
  `~/.local/share/applications/dev.yggterm.Yggterm.desktop` directly (done on
  guihost 2026-07-25) and leave the launcher alone. The generated entry is correct
  for a real install; it is the launcher-rewrite half that fights a hand deploy.
  The measurement that produced the policy, kept because the numbers are the
  argument:
  **The measurement (guihost, `systemctl --user` + `coredumpctl`), because the raw
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
  segfault) — diagnosed 2026-07-24 on guihost; LAYER 1 (crash surface) FIXED +
  LIVE-VERIFIED at 2.12.8 (`c3c7086`), LAYER 2 (routing/isolation) OPEN.**
  **UPDATE 2026-07-24 (dev agent):** the raw-coordinate `do click` path was the
  culprit — it synthesized a native GDK button event with NO hit-test, unlike
  `ClickSelector`. Fixed in `web_surface_do_for`: the `Click{x,y}` arm now evals
  `document.elementFromPoint(vx,vy)` FIRST and refuses (never injecting) if it
  returns null or the eval fails — which both confirms a live element is present
  AND round-trips through the web content process, so a page that cannot lay out
  fails there instead of taking a synthetic click into a dying frame. Live-proven
  on the fixed GUI (guihost pid 3290202, GUI-only swap, daemon + all 6 sessions
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

- **★ AGENT SHADOW CLIENT FOR THE TERMINAL LANE — E2E LIVE ON GUIHOST
  (2026-07-23, user directive "complete the agent client system e2e"):**
  the slice-4.3 shadow view client now runs against the LIVE daemon
  (sway+grim installed on guihost; `scripts/shadow-client.sh start --name
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
  1. **Detection — ACTIVE on guihost-local (2026-07-23, 2.12.8 daemon swap):**
     the daemon exports `LC_YGGTERM_SESSION_ID` at PTY spawn (the iTerm2
     `LC_TERMINAL` trick — stock OpenSSH forwards `LC_*` both ways by
     default), and yedit falls back to it. Live-proven: a fresh guihost PTY
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

- **Blank viewport from a DETACHED `term.element` (guihost, 2026-07-22).** The
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
  and then **in the live WebKit engine on guihost**, where the band is real and the
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
  working, so the daemon can stay pinned indefinitely. On guihost 2026-07-11 the
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

- **Live-path frame corruption on busy CC sessions (guihost, 2026-07-10).** While
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
  `terminal_forward_divergence` fired on guihost (4/5 events on `local://`/`live::`
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
  Reproduced on guihost against a live `remote-cc` session, and settled with GROUND
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
  - ✅ **All three layers are now DEPLOYED on guihost (2.12.13, daemon pid
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

## Deployed live on guihost, faithful-gesture confirmation pending

- **Middle-click a link in a web surface → new tab.** **The 2.10.15 entry that
  used to sit here claimed this was fixed; it never was, and the claim survived
  fifteen releases because nobody could take the faithful pixel that would have
  falsified it.** What 2.10.15 (c6542edc) actually shipped was the
  `new_window_req_handler`, which fixes `window.open` and `target="_blank"` — and
  nothing else. WebKit raises its `create` signal for a NEW_WINDOW_ACTION only, so
  a middle-click on a plain `<a href>` with no `target` never reached that
  handler at all: it arrived as an ordinary navigation of the current frame, and
  with no navigation handler installed on a web surface (`web_surface.rs` never
  called `with_navigation_handler`, and wry only connected `decide-policy` when
  one was set) there was nobody listening. The click did NOTHING.
  - **Now built (lane `lane/dev/middle-click`, GUI-only, no protocol bump):** wry
    gained a `link_gesture_handler` and the `decide-policy` connection is hoisted
    above the navigation-handler gate, so the gesture is tested whether or not an
    embedder wants a navigation policy; a handled gesture answers
    `webkit_policy_decision_ignore`, which is what keeps the opener page put. The
    surface host queues a `SurfaceLinkOpen` (no webview — there is none to hand
    back) and the shell's reconcile loop OPENS the tab with the same
    `open_command_tab` + `navigate_web_surface_tab` pair the `open_tab` command
    uses, never the popup-adopt path (whose `effective_url == url` would leave
    the tab blank). Trace event: `web_surface / new_tab_from_link`. Adjacent
    one-liner fixed in the same lane: `build_popup_webview` installed no
    new-window handler, so a `target="_blank"` INSIDE an adopted popup was
    silently dropped one level down; both doors are now installed on popups too.
  - **Still pending, and this is the same gap that let the false claim stand:** a
    FAITHFUL confirmation, which needs a real middle-click. The Xvfb harness is
    native-surface-blind, app-control clicks never reach a child webview,
    WebKitGTK blocks synthetic `window.open` (no user gesture), and guihost's
    Wayland input injection is unreliable (ydotoold). Ask the user to
    middle-click a link in a ychrome surface; confirm via the
    `web_surface / new_tab_from_link` trace event. **Do not mark this fixed on
    unit tests again** — the locks prove the wiring, not that WebKit delivers a
    middle-click as a `LinkClicked` NavigationAction on this engine build.

## 3.0.0 — the product does not build for Windows or macOS (NOT NOW; ~2 months out)

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
2. **No GitHub release has published since v2.11.0 (2026-07-11)** while guihost runs
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

## Residual threads recovered from archived memory (2026-07-31)

These were root-caused in past sessions, never closed, and were sitting in
memory files that no session indexed. They are transplanted here so the SSOT is
actually the SSOT. **⚠ Every one is UNVERIFIED against today's build** — the
newest is 3 weeks old and much has shipped since. Re-check before working one;
some are certainly fixed already. Full narrative for each is in
`~/.claude/memory-archive/-home-user-gh-yggterm/<slug>.md`.

- ⭐ **Broken-bottom during a working turn — NO LONGER A LEAD. USER-CONFIRMED LIVE
  ON 2.12.19 (2026-07-31) AND ROOT-CAUSED TO ONE THRESHOLD.** User's words: "the
  bottom region of CC opens up always TUI frame broken and I fix with slash usage
  and scroll." That workaround is the tell — typing `/` makes Claude Code repaint
  its own footer, i.e. **the CLI fixes it because yggterm's corrector never runs.**

  The corrector is the reveal screen-reconcile in `shell.rs:~84866`, and its own
  comment already says it "IS the broken-bottom fix". It is gated on
  `screen_reconcile_output_quiet` = **1,200 ms with no forwarded output**
  (`SCREEN_RECONCILE_OUTPUT_QUIET_MS`, shell.rs:550). On a miss it rearms
  **+3,000 ms** (`SCREEN_RECONCILE_DEFER_REARM_MS`, shell.rs:555) — **with no
  maximum defer count and no deadline.** A working agent CLI drives a spinner
  continuously, so the quiet window never arrives.

  There IS a bypass, and its threshold is the bug:
  ```rust
  let reveal_incomplete = screen_reconcile_reason == "reveal_screen_reconcile"
      && last_host_health_visible_nonblank_rows < 3;
  ```
  It only rescues a viewport that has degraded to **fewer than 3 non-blank rows** —
  the catastrophic blank-frame case. **A partially broken bottom, which is what the
  user actually sees, has 40+ non-blank rows, so it takes no escape at all** and
  waits for a silence that never comes.

  **Measured in `~/.yggterm/event-trace.jsonl`, 83 minutes, 2026-07-31:**
  - **198 true deferrals** (71 traced + 127 hidden by the 10 s trace rate-limiter —
    read `suppressed_since_last`, the raw line count understates it 2.8×) against
    only **32 completed reconciles**: deferred 6:1.
  - Chains are long, not isolated: session `…828dc5021f0d` deferred ~36 consecutive
    times over 106 s (10:18:19→10:20:05); `…1369e395ee57` ~40 times over 110 s
    (10:29:38→10:31:28). Both escaped only via the blank bypass.
  - `reveal_forced_incomplete` fired **9 times** — nine occasions where the viewport
    had to rot to near-blank before anything corrected it.

  ✅ **FIXED IN CODE 2026-07-31 — `SCREEN_RECONCILE_DEFER_DEADLINE_MS = 12_000`.**
  The 1,200 ms quiet test stays as the *preferred* path so brief bursts still avoid
  a tear; once the correction has been owed for 12 s the reconcile is forced
  regardless of output. Chain age is measured from the FIRST defer of a chain, not
  the last rearm — the rearm cadence is 3 s, so "time since last defer" could never
  exceed it no matter how long the user stared at a broken frame. Locks:
  `a_deferred_reconcile_converges_even_while_output_never_stops` and
  `the_defer_clock_measures_the_chain_not_the_last_rearm`, both red-proven
  (`>=`→`>` on the boundary, and removing the no-chain guard, each fail).
  ⚠ **NOT YET LIVE-VERIFIED on guihost** — the running GUI is 2.12.19, which predates
  this. Confirm after the next deploy by watching for
  `terminal_mount/screen_reconcile_forced_deadline` in the trace.

  Related and also open: `sidebugs-webgl-artifacts-stale-frame-on-switch` calls
  stale-frames-after-switch "the biggest remaining UX bug".

- ⚠ **THE PATTERN BEHIND THREE SEPARATE BUGS: we gate corrective work on
  output-silence, and an agent CLI is never output-silent.** Same false assumption
  in three places, all live:
  1. the hot-restart idle gate — 300 s idle threshold on a clock that OUTPUT bumps
     (`daemon.rs:501`, `session_idle_for_ms`); measured 0-of-40 samples open;
  2. the screen reconcile above — 1,200 ms quiet, unbounded rearm;
  3. the 2.12.19 drag-stall fix's own root cause, quoted from a81c7366: selection
     events fire "per streamed write that shifts the buffer under a live selection
     — an agent CLI that streams constantly multiplies the events."

  Terminals for humans are mostly quiet; this product's primary workload never is.
  **Any new gate must have a deadline, or hang off a positive liveness signal
  rather than an absence-of-output signal** (see
  `finding-agent-session-liveness-is-invisible-to-os-signals`).

- **Drag-selection still freezes the TUI on 2.12.19** (user-reported 2026-07-31),
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

- **`screen_snapshot_clipped_to_pty_width` fires constantly and nobody has looked**
  — 108 times in 17 minutes on `local://43c47548…`, every one reporting
  `pty_cols: 171` against `screen_max_column: 260`. The daemon's vt100 screen is
  holding content 89 columns wider than the PTY it belongs to and discarding the
  overhang on every snapshot (`terminal.rs:2113`). Unexplained; may be benign
  post-resize residue, may be a second content-loss path.
- **LIVE-path frame corruption** (`finding-client-buffer-garble-attach-seed-and-live-path`)
  — the attach-seed half was fixed in 2.10.4; the live-path half was left open
  with probes already shipped to convict it.
- **New-codex-session UUIDv4 identity drift** (`finding-new-codex-session-bug-class`)
  — the rebind sets `session.id` to codex's ULID but never rekeys the map, so key
  and identity split. Named as the single cause of three symptoms. Not fixed.
  `finding-uuidv4-codex-session-drift` (still in memory/, cited by code) holds the
  Stage-2 remote-codex rebind that was never done.
- **Daemon-side `launch_phase` stuck at RemoteBootstrap** (`finding-stale-phase-15s-remount-blink`)
  — the GUI half shipped 2026-07-07; the daemon half was left open. This is
  plausibly the same wedge as ROUND 30's §THE WEDGE — check before treating them
  as two bugs.
- **`app_render_storm` cause** (`finding-render-storm-autopsy-armed-run4`) — fired
  21× in 10 days, all unattributed; a self-arming autopsy was shipped to catch it
  and the autopsies were never read.
- **codex composer split background** (`finding-codex-composer-bg-split-reflow`) —
  xterm.js reflow on column resize drops cells' bg attribute. Root-caused, fix
  pending, flagged trap-zone.
- **OSC 52 double copy-chime + replay refire** (`finding-osc52-copy-chime-replay-refire`)
  — no dedupe and no replay-suppression, so every reattach re-parses the embedded
  OSC. Root-caused code-grounded, never live-verified.
- **ibus cumulative input fix never landed** (`finding-ibus-cumulative-input`) —
  `GTK_IM_MODULE=gtk-im-context-simple`; fix was built in the 2.9.41 tree and
  never committed. An end user hit this.
- **Shipped-but-never-live-confirmed:** `finding-cc-blink-partial-2026-frame-flush`
  (2.9.38), `finding-codex-select-scroll-kick` (2.9.32),
  `finding-remote-cc-mislabeled-codex-gone-message` (2.9.50, deploy-pending).
- **Owed proofs:** full passkey crypto E2E against a real RP
  (`finding-passkey-browser-slice-shipped`) — gated on a vault unlock; guihost GUI
  '+'-menu render proof (`finding-launcher-registry-one-app-registry`).
- **Rows lost across a daemon swap** (`project-resume-after-2100-daemon-swap`) — 3
  live rows lost, 2 of them keep-alive, with a rescue file. Same family as
  `finding-daemon-handoff-drops-live-rows` (still in memory/, code-cited).
- **ychrome queued slices** (`campaign-zoom-system-rework`) — **per-site zoom and
  the settings pane BOTH SHIPPED** (verified on ychrome main 2026-07-31:
  `src/webzoom.rs` is 238 lines of per-site overrides behind a `/zoom` endpoint
  with a change-hash so the GUI refetches only when an override moved;
  `src/sidebar.rs` serves "Tabs", "Browser identity" and "Userscripts"
  sections). What is left of this slice is **session buddy**; **vertical tabs**
  is NOT a separate item, it is the rail-as-cwdtree entry in the
  ychrome-as-main-browser list above, and should be tracked there only.
- **Non-code todos** (`project-blackboard-clearing-2026-07-16`) —
  awesome_steer_prompts repo, app-infra forecast.

## Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.
