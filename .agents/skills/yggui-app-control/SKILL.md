---
name: yggui-app-control
description: Drive end-to-end agent automation against the live yggterm desktop — screenshots, app state, telemetry streams, terminal spawn/send, kill+relaunch — so the agent can build, deploy, test, and reflect without the user touching the GUI.
---

# YggUI App Control

This skill is the agent's hands and eyes on the live yggterm desktop. Use it to:

1. **Observe**: screenshots, `app state`, `app rows`, `server snapshot`, `server trace tail` — anything the user could see by looking at the screen, you can see programmatically.
2. **Drive the app**: `app open <session>`, `app terminal new`, `app terminal send <session> --stdin`, `app maximize`, `app resize-window`, `app session remove` — anything the user could do with mouse/keyboard, you can do via these commands.
3. **Restart loop**: kill the GUI (SIGTERM), `app launch` a fresh one, screenshot, probe — the full build → deploy → restart → verify cycle without handing back to the user (see [`feedback-agent-restart-test-loop`] in memory).
4. **Reflect / test hypotheses**: spawn a fresh terminal, run a probe command (`codex resume <id>`, `for i in {1..500}; do echo line $i; done`, etc.), screenshot, query state — verify behavior on the live system rather than reasoning from code alone.
5. **Verify before claiming shipped**: per CLAUDE.md, "compiled binary on disk + passing unit tests" is not proof. Exercise the affordance live via this skill and quote the evidence (screenshot path, state field value, telemetry event) in the user-facing report.

This was the explicit design intent: yggterm is agent-first controllable for everything from a remote console.

## ⛔ BEFORE YOU RESTART THE GUI: the presentation policy binds you

This skill hands you `app launch` and the kill-and-relaunch loop. That power is
exactly how the user's GUI has repeatedly ended up running as something nobody
chose. **Read `docs/presentation-policy.md`.**

- **Never launch or relaunch the user's GUI with a `PRESENTATION_VARS` variable
  set** — `GDK_BACKEND`, `LIBGL_ALWAYS_SOFTWARE`, `GALLIUM_DRIVER`,
  `WEBKIT_DISABLE_DMABUF_RENDERER`, `WEBKIT_DISABLE_COMPOSITING_MODE`,
  `YGGTERM_WEB_SURFACE_UNDER_GLASS`, `GST_PLUGIN_FEATURE_RANK`,
  `YGGTERM_ENABLE_XTERM_CANVAS`. `app launch` with a clean environment is the
  only sanctioned way to bring their GUI back.
- **What you learned under Xvfb does not travel.** Headless sway and Xvfb are
  X11, so `GDK_BACKEND=x11` is correct there and WRONG on the user's Wayland
  desktop. Restarting their GUI with it has cost hours, more than once. To test
  an arm, use `scripts/underglass-sandbox.sh` — a throwaway GUI with its own env
  and its own daemon.
- **A relaunched GUI inherits the DAEMON's environment**, and the daemon can be
  days old with a frozen env. After any relaunch, confirm the arming from the
  `gui/startup/linux_desktop_backend_policy` trace event — NOT from
  `/proc/<pid>/environ`, which cannot see `set_var` after exec.
- **"Is it XWayland?"** is answered by counting X11 sockets in
  `/proc/<gui-pid>/fd` (must be 0) and finding the GUI on `wayland-0` in
  `ss -xp`. An empty `xwininfo` means the instrument failed, not that the answer
  is no.

## Scope — Dioxus DESKTOP surface only (observability + automation, by agents for agents)

This skill is an agent's "human eye + keyboard/mouse" for a **Dioxus desktop UX**: select an element (like a cwd-tree pick), navigate, screenshot the running app, measure animation/timing, iterate a feature — and when a flow repeats, write it as an **ad-hoc automation script, check it in, and rerun it** (a first-class record→replay "Macro" affordance is a future TODO, not built yet).

- **Two capture layers** (both faithful as of 2.8.0): **app-level** via `app screenshot` (the yggui/webview surface) and **OS-level** via the compositor (on KDE Wayland, Spectacle — see `finding-app-screenshot-unfaithful-on-wayland` in memory; the capture force-activates yggterm and refuses to capture any other window).
- **Web UX is OUT of scope.** Driving a web app (e.g. samplers / samplenotes-webapp running in Chrome) is the job of the **separate agent-browser CLI skill**, not this one. Clear lanes: this skill = Dioxus desktop; browser skill = web.
- **Today this drives yggterm.** It generalizes to any Dioxus desktop app only once app-control is extracted into a reusable crate (`finding-yggui-app-control-not-reusable` in memory) — relevant when samplers / samplenotes-webapp ship desktop builds, not now (they're webapp + Android in the current prototyping phase).

## Live Host

The live desktop host SSH alias is stored in `.agents/config/live-host` (one line, e.g. `jojo`).
The yggterm binary on that host is `~/.local/bin/yggterm`.

Read it:
```
LIVE_HOST=$(cat .agents/config/live-host)
```

## ⛔ THE SHADOW-PROBE LAW (user-directed 2026-07-23) — probe through the shadow client, never the user's GUI

Any probe that changes what the viewport shows — `app open`, view switches,
search, session inspection — MUST run against a **shadow view client**, not the
user's GUI. The user reported the foreground-driving probes directly ("can't
you use the agent control client mode — we built it for exactly this") and the
answer is yes, it works end-to-end.

**The shadow is a FIRST-CLASS tool (user ruling 2026-07-23):**
pixels-of-a-non-active-view is the most load-bearing bug-bash instrument that
does not disturb the user — don't treat it as a last resort. ⚠ Platform note
(same ruling): the `--client`/role model is platform-neutral; sway+grim is
only the LINUX backend of the shadow-view concept. yggterm is heading to
Windows/macOS (and Android/iOS in a private repo), so never let core-plane
code grow a compositor dependency — new shadow work goes behind a
per-platform backend seam.

```bash
# One-time per work session (idempotent; reuses a running shadow):
ssh "$LIVE_HOST" 'cd ~/gh/yggterm && ./scripts/shadow-client.sh start --name agent-1'

# Then aim every viewport-changing verb at it:
ssh "$LIVE_HOST" '~/.local/bin/yggterm server app open <session-path> --client agent-1'
ssh "$LIVE_HOST" '~/.local/bin/yggterm server app state --client agent-1'
ssh "$LIVE_HOST" 'cd ~/gh/yggterm && ./scripts/shadow-client.sh capture /tmp/shadow.png --name agent-1'

# Tear down at session end (the ritual's dangling-process duty):
ssh "$LIVE_HOST" 'cd ~/gh/yggterm && ./scripts/shadow-client.sh stop --name agent-1'
```

- ✅ **IT WORKS FROM INSIDE A YGGTERM SESSION AGAIN (fixed 2026-08-01).** The
  launcher used to default through `YGGTERM_BIN`, which the daemon exports into
  every PTY it owns as ITS OWN executable — so in any agent's row it launched
  `yggterm-headless` and died with "only supports server subcommands" (or, on a
  hot-restarted daemon, chased a `… (deleted)` path and reported "yggterm binary
  not found"). Every in-session agent hit this, and the only alternative left
  was the user's live GUI — the exact thing this law forbids. Resolution now has
  one owner, `scripts/lib/gui-binary.sh`: it probes each candidate's own
  `--help` for a GUI-only command and refuses a headless build by name.
  **Override with `YGGTERM_GUI_BIN=<path>`, never `YGGTERM_BIN`.**
- The shadow has its OWN active session and viewport — live-proven 2026-07-23:
  `open --client agent-1` switched the shadow while the user's worker stayed on
  their session (verify with pid-targeted `state --pid <user-gui-pid>`).
- **Untargeted verbs route to the ACTIVE client** (the user's GUI) even while
  shadows run — never rely on "newest wins," and never read the user's state
  through an untargeted verb *assuming* it's theirs on an older build: on
  pre-fix builds an untargeted read answers from the newest worker (the
  shadow), which looks exactly like the shadow yanking the user's session
  (instrument-lie, live-caught 2026-07-23).
- A Shadow is read-only for geometry/ownership by daemon role gate: it cannot
  `terminal new`/resize/focus. For SPAWNING probe sessions, use the user's
  worker with `terminal new --no-activate` (their view never moves), then
  `app open <session> --client <shadow>` to look at it.
- The shadow's screenshots come from `shadow-client.sh capture` (grim on its
  own compositor) or `app screenshot --client agent-1`.
- ✅ **THE SHADOW'S TERMINAL VIEWPORT PAINTS (fixed 2026-07-25).** It used to
  render blank, and the old note here blamed "the role gate denies it the PTY
  attach." That was the right observation with the wrong owner: the gate was
  correct, the CLIENT was wrong to ask. A shadow now takes the **read-only**
  path — it never sends `terminal_ensure`/`terminal_resize`/`focus_live`, and
  paints from the read stream, which the gate already allowed. `app open
  <session> --client <shadow>` switches its viewport and shows real scrollback
  while the user's GUI does not move.
  - **Its xterm pins to the daemon's PTY grid**, because a shadow may not
    resize the PTY (D8) and a differently-sized viewer would wrap the frame
    wrongly. So `session_view_contract_violations` stays `[]` and the frame is
    faithful — a shadow screenshot is now valid pixel proof for a terminal bug.
  - ⚠ Start it big: `--size 2560x1440` (now the default). A window SMALLER
    than the pinned grid clips rows out of every capture, silently.
  - A session with no live runtime still shows nothing on a shadow — it may
    not start one. That is the honest limit, not a bug.
  - `terminal read-buffer <session>` and `server snapshot` remain the cheapest
    way to read CONTENT (safe untargeted; they never move the user's view).

### Background work WITHOUT the shadow (the verbs plane — prefer these for ACTION)

The settled model ([[spec-agent-shadow-client-control]]): agents ACT on the
user's own GUI against backgrounded sessions; the user can switch in anytime
and watch (agent-presence cursor). The enabling verbs:

```bash
# Spawn a session WITHOUT switching the user's view (agent probe/work spawns).
# ALWAYS pass --purpose (and --agent): with no --title the row is named
# "Agent <identity> <kind>: <purpose>" instead of inheriting the cwd-shaped
# default, which renders identically to a human's shell in the same directory
# and makes your row unfindable by any title probe.
#
# ⚠ --agent is a TRAILING flag on the subcommand, NOT a global prefix.
# `yggterm --agent <id> server app terminal new ...` produces EMPTY OUTPUT
# (corrected 2026-07-28 — the global form was documented here and is wrong).
yggterm server app terminal new --kind shell --no-activate \
  --purpose "what this session is for" --agent <id>

# ⚠ Sleep ~3s before the first `terminal send`. Sending into a session whose
# shell has not finished starting is SILENTLY SWALLOWED: the send still answers
# accepted:true with a byte count, no process appears, and the failure surfaces
# far away as `web ensure` reporting "the daemon has no web-surface declare
# (a plain shell, or the app already closed its surface)".
# Materialize a BACKGROUNDED session's declared web surfaces into the soft
# stash (created + demoted + leased, never revealed) so web do/read/wait
# verbs can drive them immediately. Ensure AWAITS the app's surface policy
# (userscripts + the yggterm-appctl:// signer bridge — the passkey plane)
# before building, bounded at 8s; on timeout it refuses with
# `reason: policy_gate_not_ready` + the gate state instead of silently
# building an unprotected surface — just retry once the app's control
# endpoint serves /policy. Every response reports `policy_gate:
# ready|absent|pending|abandoned`; "ready" is the one that has WebAuthn.
yggterm server app web ensure --session <path> [--ttl <secs>]
# Then automate invisibly:
yggterm server app web do click --selector "#submit" --session <path>
yggterm server app web read --as readable --session <path>
yggterm server app web wait --until load:finished --session <path>
```

Tear the session down with `app session remove <path>` and **read the verdict,
not `accepted` alone**: the response carries `verified`, and when it is false a
named `verified_refusal` (`row_still_listed`, `processes_survived`,
`runtime_pid_unobservable`) plus `live_processes` (pid + command) for anything
that outlived the teardown. The removal signals only the PTY child, so an app
you started under that shell can and does survive — `verified:false` with pids
is the truth, and reporting "session removed" off a `verified:false` response
is exactly the lie this contract exists to make impossible. The verb REPORTS
survivors; it does not kill them, so reap them yourself and re-verify.

Recipe for invisible ychrome automation on a live session: `terminal new
--no-activate --purpose ...` (or target an existing session) → `terminal send
<session>` to run `ychrome <url>` there → `web ensure --session <session>` →
`do/read/wait`. Pass `--agent <id>` so the user sees your cursor if they
switch in. ✅ **The Dream §2 seam is CLOSED at 2.12.10: no reveal is needed,
ever.** The daemon now reads the OSC declare off the PTY itself and keeps the
app's latest payload, so `web ensure` materializes a NEVER-revealed session's
surface — and rebuilds one the reaper collected — with the user's view
untouched (`rebuilt_from_daemon_declare: true` in the response says that is
what happened). It only rebuilds on an explicit `ensure`: a heartbeat must
never resurrect a surface the app or the user closed.

### The rest of the `web` plane (2026-07-25)

⚠ **Either binary runs it, and they cannot diverge (fixed 2026-07-31).** The
whole `web` plane used to live in the GUI binary's `main.rs` only, so
`yggterm-headless server app web <anything>` — the binary CLAUDE.md and these
skills point agents at — answered `unsupported app control command: web`, which
reads as "this build does not have the feature". The plane now has ONE owner,
`crates/yggterm-server/src/app_control_web_cli.rs`; both binaries route their
`"web"` arm straight to it and render the same generated usage block under
their own name. Every example below works verbatim with `yggterm-headless` in
place of `yggterm`. A test (`both_binaries_route_the_web_plane_to_its_one_owner`
in `apps/yggterm/src/main.rs`) fails the build if either binary grows a verb
dispatch of its own again.

`yggterm server app web --help` is generated from the dispatcher's own verb
list, so it cannot go stale — **never conclude "not deployed" from a usage
string**, but do trust this one. Full reference:
`docs/web-surfaces.md#the-server-app-web-verb-plane-2026-07-25`. The parts that
change how you work:

```bash
# Address an element the way a HUMAN would, resolved at click time. Gateway and
# bank UIs have no stable ids.
yggterm server app web do click --text "Proceed to Pay" --exact --session <path>
yggterm server app web do click --role button --label "Continue" --session <path>

# N verbs behind ONE gate. The human still wins — at the batch's START as well
# as mid-run: a click waiting when the batch opens refuses it (`preempted`),
# and real seat input mid-run aborts the remainder with `remaining: n`. That
# refusal consumes the count, so simply re-issuing the batch opens it.
#   ⚠ READ THE ENVELOPE, not just `accepted`: `{requested, attempted,
#   succeeded, failed}`. `accepted` is TRUE only when the batch ran to the end
#   AND every action succeeded — a 31-field fill where every selector missed is
#   `accepted:false, succeeded:0, failed:31`, never "the form filled".
yggterm server app web batch --script fill-form.txt --session <path>

# Split the flow: script it on curl, hand the session over, hand it back.
yggterm server app web cookies --import login.jar --session <path>
#   ⚠ the jar is per-PROFILE; an unqualified surface is `default`, the USER'S
#   OWN browsing jar. Check the `profile` field in the response.
#   An EXPORTED jar is written 0600 — it is a live credential, not a report.

# Pixels of ONE element, in-page — works on a surface nobody has ever seen.
yggterm server app web capture-element --selector img#captcha out.png --split 6

# See into iframes. `read` with NO --frame searches every reachable frame.
#   `result` is still the TOP DOCUMENT's answer (the old shape, unchanged);
#   `frames[]` is added beside it. Nothing that read `.result` broke.
yggterm server app web frames --session <path>
yggterm server app web read --as forms --frame billdesk --session <path>

# Wait THROUGH a redirect chain (read from the engine, not the page).
yggterm server app web wait --until url:matches:'^https://auth\.' --session <path>
yggterm server app web wait --until settled:800 --session <path>

# The one async bridge — `eval` cannot return a Promise.
yggterm server app web await --script fetch-status.js --session <path>

# A credential without ever seeing it: the CLI names the item and field.
yggterm server app web fill-vault --item sbi --field password \
  --role textbox --label "Password" --session <path>

# A payment card, same rule. Four boxes, four calls; each one reads the vault
# AGENT SOCKET (`card-secret`) — there is no CLI verb that prints a PAN and
# there never will be, which is why aiming this at the CLI once answered
# `vault_cli_no_card_op` at a live gateway.
yggterm server app web fill-card --item 'HDFC Regalia' --field number  --selector '#pan'    --session <path>
yggterm server app web fill-card --item 'HDFC Regalia' --field expiry  --selector '#exp'    --session <path>   # MM/YY
yggterm server app web fill-card --item 'HDFC Regalia' --field code    --selector '#cvv2'   --session <path>
yggterm server app web fill-card --item 'HDFC Regalia' --field holder  --selector '#name'   --session <path>
# also: --field exp-month (MM) / exp-year (as stored, usually YYYY) for split forms

# The profile PICKER CARD's row menu, addressable (added 2026-08-01). Those
# verbs used to exist ONLY on a card an agent had no way to raise, so the
# avatar sidecar's persistence contract could not be verified at all.
yggterm server app web profile list
yggterm server app web profile show work
yggterm server app web profile avatar work --emoji 🚀      # "Change avatar…"
yggterm server app web profile avatar work --default       # "Use the default avatar"
yggterm server app web profile protect work                # "Protect profile"
yggterm server app web profile unprotect work
```

**`profile` reads and writes host state on disk, not a GUI round trip** — the
card re-reads `~/.yggterm/web-profiles/<name>/profile.json` on every render, so
the verb works with no GUI running and the card shows the change on its next
render. Both go through ONE core function
(`yggterm_core::web_profile::update_profile_meta_in`), which is what keeps
`agent_drive` — a key ychrome owns and yggterm has no field for — alive across
a write. **`list`/`show` report `unknown_keys`, and that field IS the
persistence proof**; a write that empties it is the regression. Refusals match
the card exactly: `protect default` answers *"default is always protected"*, the
same sentence the card's disabled entry shows, and an avatar the picker's own
field would reject is refused here too. Flags go AFTER the name.

**`fill-card`'s only gate is the vault UNLOCK.** Every Bitwarden client can read
a card cipher and `ychrome-vault` is one, so there is no grant and no per-use
consent (the user's ruling, 2026-07-26 — do not re-propose one). A locked vault
answers `vault_locked` naming `ychrome-vault unlock`; a ychrome-vault too old to
report its socket answers `vault_agent_socket_unknown`, and the remedy is to
install the new one and `ychrome-vault handover` (which keeps the unlock). The
answer is `{item, field, chars, matched}` — a name and a length, never the
value — and every release leaves one line in `~/.yggterm/vault/audit.log`
naming FIELDS, never values. Contract: `ychrome/docs/vault.md`.

**Which tabs actually hold a web process — `server app state` →
`web_surface_tabs`.** Every DESIRED tab joined against the realized webview
registry, so "a hundred tabs, how many web processes?" is answerable instead of
guessed:

```bash
yggterm server app state | jq '.web_surface_tabs
  | {tabs, views, tabs_without_webview, contexts, per_tab_rss}'
yggterm server app state | jq '.web_surface_tabs.rows[]
  | select(.state != "no_webview")'
```

Per row: `state` (`visible` / `stashed` / `live` / `no_webview`), `webview`,
`native_id`, `generation`, `ever_revealed`, `stashed_for_ms` (how long it has
been off screen — the age the reclaim hold is read against), `reaps_in_window`,
`active_tab`, `split_pinned`, `leased`. Background tabs of the session the user
is looking at are reclaimed on their own hold now
(`~/.yggterm/web-surface.json` `tab_background_hold_secs`, default 600 s), and a
reclaimed tab comes back through the ordinary lazy-create path when it is
selected — see `docs/web-surfaces.md`.

⚠ **`no_webview` does NOT distinguish "never visited" from "reclaimed"** — after
the destroy they are the same object with the same cost. `reaps_in_window` is
the only hint and it forgets after the thrash window.

⚠ **There is no per-tab RSS and you must not synthesize one.** WebKitGTK pools
web processes per `WebContext`, so bytes are not attributable to a tab; the
payload says so in `per_tab_rss`. Read (`views`, `contexts`) against the render
probe's process RSS instead.

**Recovering a dead tab without destroying the session.** `web ensure` probes
LIVENESS (it round-trips an eval through the content process, because tabs /
handles / engine flags all stay true over a corpse). Compare
`generation_before` with `generation_after` — `healed: true` means a NEW page,
not the same one. `web reload --session` and `web close --session` reach the
same recovery directly. Refusals now name the fact that failed (`no_declare`,
`declare_stale` = the app EXITED, relaunch it; `declare_url_scheme_refused`;
`daemon_declare_unavailable` = the fetch failed, which is NOT an absent
declare; `session_closed` = see below).

⛔ **`session_closed` — do not retry, and do not tear your session down.**
`ensure` refuses a session whose runtime is gone AND whose row the user closed.
Reviving one gives you a live page with NO row: the user cannot see it, cannot
click into it, and cannot take it back. That is not hypothetical — an agent
drove a real page for an hour that way. The remedy is your own session:

```bash
yggterm server app terminal new          # your own row, visible to the user
```

**Create ONE session per run and LEAVE IT UP.** Tearing your session down as a
courtesy at the end of a run is what left the next run with nothing to attach to
but an orphan. Visibility beats tidiness: a row the user can see and co-browse
is the point. Closing a session now also ends its web surfaces, so a torn-down
run leaves nothing behind to attach to.

**`seat_input_on_unrevealed_surface`.** Input was observed on a surface no
client has ever shown. Your verb is refused, but you are NOT preempted and your
batch is not cancelled — the user cannot have taken a page they have never seen,
and the old code said they did, which locked the lane until a new incarnation.
Reveal the session (open its row) and re-run.

**Before deploying, check whether someone is driving:**

```bash
yggterm server app state | jq .agent_leases
# `server app update restart` REFUSES with agent_lease_active while a lease is
# live; --force overrides. (It cannot stop `pkill yggterm`.)
```

**Reading a refusal.** `js_result_unsupported` = your script returned a Promise
or a DOM object; the page is FINE. `webview_unreachable` = nobody answered;
run `web ensure`. Those two used to share one string and the ambiguity cost a
field run ten minutes.

⚠ **Before reading any refusal, check a webview exists.**
`server app state | jq .web_surface_tabs` — `views: 0` / `contexts: 0` means
no content process anywhere, so every verb fails and its error string is noise.
Note `web ensure` can answer `healed: true` about the declare/tab while no
view is realized; that pair misleads badly. Probe with `eval '1+1'`: if a
trivial script is refused, go look at `web_surface_tabs` rather than at your
script.

**Version mismatch is now honest.** App-control is a filesystem dropbox, so CLI
and GUI must be swapped together; a verb this GUI does not implement answers
`unsupported_command_kind` instead of timing out. And a `--frame` request to a
GUI that predates frames HARD-FAILS on the missing `frame_resolved` echo rather
than silently querying the top document.

## Screenshot

```bash
LIVE_HOST=$(cat .agents/config/live-host)
SHOT=/tmp/yggui-shot-$(date +%s).png
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/yggui-shot.png" \
  && scp "$LIVE_HOST:/tmp/yggui-shot.png" "$SHOT" \
  && echo "$SHOT"
```

Then read the file with the Read tool to display it visually.

### Crop + zoom for legibility (USE THIS — don't avoid the tool)

A full 1920px frame renders illegibly small when you read it back (159×63 glyphs
scaled to fit). That is NOT a reason to distrust or skip the screenshot — it's a
reason to crop/zoom. The capture is faithful (DOM renderer → WebKit snapshot is
accurate; on KDE/Wayland Spectacle is correctly *skipped* when the window is
unfocused, per the privacy gate, and the WebKit-DOM fallback is faithful). Use the
post-process flags to get a legible view of the region you care about:

```bash
# Just the terminal viewport, doubled — best default for reading terminal content
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/term.png --region terminal --scale 2"
# A specific strip (e.g. the bottom rows / composer) at 3x — pixel crop x,y,w,h
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/strip.png --crop 277,930,1335,230 --scale 3"
```

- `--region terminal` auto-crops to the active terminal viewport (rect from app state).
- `--crop x,y,w,h` is an explicit pixel crop in screenshot coordinates (the same
  coordinates as `active_terminal_hosts[0].rows_rect` in `app state`).
- `--scale n` nearest-neighbour upscales after cropping (2–3 is usually right).
- The response records what it did under `data.post_process`.

### Aiming a click: `--grid` (agent-only, the user never sees it)

Don't guess pixel coordinates off a screenshot. Ask for a labelled grid:

```bash
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/g.png --grid"
# then, to zoom in on one cell:
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/g.png --grid --grid-refine C4"
```

- The grid is composited into the **returned PNG only** — the live page is never
  touched, so this is safe to run while the user is working, and a screenshot
  they take at the same moment is grid-free. (This is the difference from
  `server app grid show`, which paints into the real page until its TTL.)
- Read the cell manifest from `data.post_process.grid`. Each cell carries
  `capture` coords (**click these**) and `image` coords (what you see).
  `capture_size` should equal `window.inner_size` from `app state` — when it
  does, capture pixels are CSS pixels and `capture.cx/cy` go straight to a click.
- Composes with `--crop`/`--region`/`--scale`; the grid spans the cropped area.
- Full contract: `docs/yggui-click-grid.md`.

### Say who you are: `--agent <id>` (agent presence, cursor v1)

Pass `--agent <id>` on any app-control command **after** the subcommand
(`server app pointer move --x 760 --y 430 --agent codex-alpha`), or export
`YGGTERM_AGENT`. A leading `--agent` is rejected by the subcommand classifier.

The window then shows your pointer as a coloured `agent-N` arrow while the user
is viewing the session you are working — so a human watching the screen can see
that something else is driving, and which one. Presence is readable at
`app state` → `agent_presence`:

- `visible` — pointers the user can see right now (agents on the viewed session)
- `live` — every agent inside the TTL, whatever the user is looking at
- pointers expire after `ttl_ms` (8 s)

⚠️ **Confirming a cursor visually requires `--backend os`.** The default backend
pastes the xterm canvas over a DOM snapshot, so a cursor sitting over the
terminal viewport is absent from the frame **even though the user sees it**.
Either grab with `--backend os`, or place the probe cursor over the sidebar.
A frame that seems to show "no agent cursor" is not evidence unless it came
from the compositor backend.

### Native web surfaces need `--backend os` (v2.9.57+)

The default capture backends are **blind to native child webviews** — the
web-surface webviews layered over the page area (2.9.56 substrate). The
xterm-canvas composite pastes canvas over a DOM snapshot, and a native GTK
widget is in NEITHER layer, so a web surface simply does not appear in a
default `app screenshot` frame. When verifying anything about a web surface,
pass `--backend os`:

```bash
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/surface.png --backend os"
```

**The capture now tells you when it is blind** (2026-07-10). If a native surface
is visible and you did not pass `--backend os`, the response carries
`capture_native_web_surface_visible: true` and `capture_faithful: false`, with a
reason saying so. **Read those fields before reasoning from the pixels.**

⚠️ **`capture_faithful: true` was never a claim about native surfaces.** It means
"the xterm canvas in this frame is real". An agent once cropped the right rail
from a default-backend frame, saw a perfect vault pane, and called the feature
live-verified — while the native page was in fact painted straight across that
rail (a native child draws above ALL DOM). The crop was faithful; the screen was
broken. If a web surface is on screen, `--backend os` is the ONLY honest eye.

- Forces an OS-compositor grab of the yggterm window (Spectacle on KDE Wayland,
  X11 window grab on X11) — native surfaces AND the accelerated xterm canvas are
  both in the frame; `capture_faithful` is true by construction.
- On Wayland this RAISES/FOCUSES the yggterm window first (KWin force-activate)
  because Spectacle grabs the active window. Brief focus steal from the user is
  the cost of a faithful native pixel.
- **No silent fallback**: if the window cannot be focused (privacy gate — never
  capture another app's window), the command returns an ERROR instead of quietly
  handing back a DOM frame that would lie about the surface. Handle the error;
  don't retry in a tight loop while the user is actively refusing focus.
- `--region` / `--crop` / `--scale` compose with it as usual.
- A non-visual cross-check that a surface webview exists at all: each live
  surface adds a `WebKitWebProcess`+`WebKitNetworkProcess` pair under the GUI pid
  (`pgrep -a -P <guipid> -f WebKitWebProcess`).

If a future need isn't covered (e.g. annotate, side-by-side), EXTEND the tool —
that's the point of agent-first observability — don't fall back to "the screenshot
is too small to use."

## App State

```bash
LIVE_HOST=$(cat .agents/config/live-host)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app state" | python3 -m json.tool 2>/dev/null || true
```

### `handover_paint` — is the terminal deliberately not drawing? (2026-07-26)

A daemon handover re-resumes every session on a fresh PTY, and that repaint
storm is the GUI host's most expensive minute. The client now DETECTS the
handover from the daemon's own report and stops painting for its duration, so a
frozen-looking viewport during a swap may be correct behaviour rather than a
bug. Check before investigating:

```json
"handover_paint": { "paint_suspended": true, "suspended_for_ms": 4200,
                    "suspend_ceiling_ms": 90000, "handoff_in_flight": true,
                    "client_sessions_awaiting_adoption": true,
                    "fingerprint": "pid=…:2.12.17|local://…",
                    "suspend_count": 1, "last_transition": "suspended" }
```

- `paint_suspended: true` ⇒ **no PTY read, no `term.write`, no render-health
  sampling, no visible paint**, and a static veil over the viewport. A
  screenshot taken now shows the veil, not the terminal — it is not a capture
  failure and not a blank-frame bug.
- The predicate is the DAEMON'S own `preserved_terminal_owner_keys` intersected
  with the runtime keys this client has mounted. Another agent's session
  migrating on a lingering older daemon reads `handoff_in_flight: true` with
  `client_sessions_awaiting_adoption: false` and veils nothing.
- It resumes when the successor adopts our keys, when the status goes
  unreadable, or at `suspend_ceiling_ms`. A suspension that hit the ceiling
  latches its `fingerprint` into `resolved_fingerprint` and cannot re-arm.
- Trace: component `daemon_handover`, events `handover_paint_suspended` /
  `handover_paint_resumed` (one pair from `ShellState`, one per mounted bridge).

### Drag gestures — TWO independent ones, and they read differently

The cwd tree and the contributed app rail (yedit's file list, ychrome's tabs)
run separate gesture machines, so a stuck drag shows up in a different field
depending on which surface it started on:

- **cwd tree** — `drag_paths` (empty ⇒ no drag), `drag_hover_target`,
  `drag_pointer`.
- **contributed rail** — `app_pane_row_drag` (2026-07-26; before that the rail
  gesture was invisible here) and `app_pane_row_drop_target`:

```json
"app_pane_row_drag": { "pane": "notes", "row": "7f3a…",
                       "armed": false, "dragging": true }
```

`armed: true` = the button is down but the pointer has not travelled the 6px
threshold, so this is still a CLICK: it paints no dim and accepts no drop
target. `dragging: true` = the live gesture, and it is exactly the flag that
draws the row's dim. **A non-null `app_pane_row_drag` with no mouse button held
is a bug** — the gesture must not outlive the button. That was the "rail rows
look lighter for no reason, and then reorder themselves" report: nothing ended
a drag released outside a row, so the row stayed dimmed and the next pointer
move re-armed a drop target.

## Session recovery — reconnect stranded sessions, fix row order (v2.9.63+)

These are **daemon-direct** commands (`server …`, not `server app …`): they need no
GUI and no click. A session that exists but is not in **Live Sessions** — alive on
its host, reachable only from the CWD tree — is *stranded* ("in the void").

```bash
LIVE_HOST=$(cat .agents/config/live-host)

# What exists but is NOT live? (remote scans minus the live set, NEWEST FIRST)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server connect --list"
# -> {connectable_count, live_session_count, connectable:[{path,title,cwd,modified_epoch,live_runtime}]}
# A busy host has HUNDREDS of scanned sessions; recency ordering surfaces what the
# user was just working on. Do NOT bulk-connect the whole list.

# Pull one back into Live Sessions and attach/resume its terminal.
# ORDER-PRESERVING by default: existing rows keep their exact positions and the
# connected row lands LAST. --after <path> places it under an anchor; --top
# restores the daemon-native prepend.
ssh "$LIVE_HOST" "~/.local/bin/yggterm server connect 'remote-cc://dev/<uuid>'"
# -> {connected:true, row_placement:"end", order_preserved:true, active_session_path, ...}
ssh "$LIVE_HOST" "~/.local/bin/yggterm server connect '<path>' --after '<anchor-path>'"
ssh "$LIVE_HOST" "~/.local/bin/yggterm server connect '<path>' --view preview"  # don't launch a terminal

# Capture / restore the Live Sessions row order (these round-trip)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server order" > /tmp/order.bak      # one path per line
ssh "$LIVE_HOST" "~/.local/bin/yggterm server reorder --stdin" < /tmp/order.bak
ssh "$LIVE_HOST" "~/.local/bin/yggterm server reorder '<path1>' '<path2>'" # listed rows -> TOP
# -> {requested:[...], applied:[...], skipped:[{path,reason}], live_session_count,
#     changed, order:[...], message}
# NON-ZERO EXIT when `skipped` is non-empty. On a daemon older than the honest-
# response fix the report carries applied:null + applied_unreported_by_daemon:true
# — that build silently ignored rows with no runtime, so verify with `server order`.

# Inspect the durable row-order LEDGER (v2.9.64+): per-client-scope memory of
# row slots, including rows that are NOT currently live.
ssh "$LIVE_HOST" "~/.local/bin/yggterm server ledger"                      # all scopes
ssh "$LIVE_HOST" "~/.local/bin/yggterm server ledger --scope gui:jojo"     # one GUI's ledger
ssh "$LIVE_HOST" "~/.local/bin/yggterm server reorder --stdin --scope gui:jojo" < /tmp/order.bak
```

**Row order is durable AND remembered across liveness (v2.9.64+).** Since 2.9.62 the
daemon persists non-keep-alive rows in order, so ordering survives a restart. Since
2.9.64 the daemon additionally keeps a **row-order ledger** (`row-order-ledger.json`):
every order change is recorded per client scope (the GUI records under `gui:<host>`,
CLI/daemon-native under `shared`), and a row that LEAVES the live set keeps its
remembered slot — when it is reconnected/opened again it is placed back below its
nearest remembered live neighbor instead of landing at a native position. A row the
ledger has never seen keeps the old behavior. Multiple GUIs attached to the same
daemon each get their own ledger scope (a session can hold a slot in several scopes
at once); placement falls back to the `shared` scope when the client's own scope
doesn't know the row. `server order` + `server reorder --stdin` still round-trip —
**take a backup before any batch operation.**

**The ledger now RESTORES, and a daemon bump leaves a receipt (in-tree 2026-07-26,
not yet live-proven).** Each handover rebuild pass ends by reconciling the assembled
row list against the ledger as the daemon booted with it: rows the ledger remembers
take the ledger's order, rows it has never seen keep the slot the anchored import
walk gave them, and the result is a permutation — so nothing tombstoned in
`removed-rows.json` can come back through it. Every bump also writes
`~/.yggterm/manual-snapshots/pre-daemon-swap-<unix-secs>-<pid>.json` (live order +
the whole ledger), from the outgoing daemon on `PrepareUpdateRestart` and from the
incoming daemon before it imports a row; newest 32 kept, hand-made
`pre-gui-restart-*` snapshots in that directory are never swept.

**What `connect` does** — the headless twin of clicking a row, issuing the SAME
daemon requests as the GUI (one source of truth):
- a session the daemon already tracks → `FocusLive` (kind-agnostic; also un-hides a
  row the snapshot runtime-truth filter was suppressing, because it launches the runtime);
- a scan-only **Codex** row (`remote-session://`) → `OpenRemoteSession`;
- everything else, notably a **Claude Code** row (`remote-cc://`) → `OpenStoredSession`
  carrying kind + id + **cwd** + title.

**Traps (all live-caught):**
- `OpenRemoteSession` is **Codex-only**. Sending a `remote-cc://` uuid through it
  fails (`saved Codex session … no longer available`) *and leaves an orphan
  `remote-session://` row*. `connect` handles the branch for you — don't hand-roll it.
- Always let `connect` pass the scanned **cwd**: the resume runs `claude -r` /
  `codex resume` inside the session's directory.
- `connect` is order-preserving since 2.9.63; with `--top` (or on any older build) it
  **prepends** and buries the user's ordering. Capture `server order` before a batch.
- `reorder` never drops a row: listed paths go first, every unlisted live row is
  appended after, so a partial list is safe. It also never ADDS one — a path that
  is not already a Live Sessions row is refused (`skipped`), not created. Dormant
  rows (no runtime) reorder like any other row since the honest-response fix;
  before it they were silently ignored while the response echoed success.
- Verify a reconnect with the session's `status_line`/`last_launch_error` from
  `server snapshot`, not `app terminal read-buffer` — the GUI may not have mounted
  the xterm yet even though the resume is healthy. Since 2.12.10 `read-buffer`
  answers from the DAEMON screen in that case (`source: "daemon_screen"`,
  `client_host: "missing"`) instead of refusing, so a `--no-activate` work
  session can be read without revealing it; `--mode cells` still needs a real
  client host (attributes live only in xterm's buffer).

## Click Grid (agent pointer targeting — main webview AND ychrome pages)

Full spec: `docs/yggui-click-grid.md`. Labeled grid overlay for the vision loop:
show → screenshot → read the cell label next to the target → click the cell.
The GUI resolves cells to coordinates server-side; never read pixel coordinates
off a screenshot yourself.

```bash
LIVE_HOST=$(cat .agents/config/live-host)
# 1. Draw the grid (default 12×8, auto-targets ychrome page if one is live)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app grid show --cols 12 --rows 8"
# 2. Screenshot to choose (grid over a ychrome page needs --backend os)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app screenshot /tmp/grid.png --backend os"
# 3. Click a cell — or refine first for small targets
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app grid click B7 --refine"   # subdivides B7 into 1-9
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app grid click B7.5"          # clicks the middle ninth
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app grid hover C3 --keep"
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app grid hide"
```

- `--target main|surface|auto` — `surface` injects the grid INSIDE the active
  session's native child webview (ychrome page, canvas/3D) in page coordinates;
  a window-level synthetic click can never reach a native child widget.
  `--region terminal` (main target) restricts the grid to the viewport.
- Click/hover responses include the hit element (`tag`, `id`, `cls`, `text`) —
  ALWAYS check it to verify you hit what you aimed at.
- A click hides the grid unless `--keep`; TTL auto-hides after 120 s.
- **When to use which targeting tool:** semantic first — `app dom-eval`
  (main webview) or `app web eval` (ychrome page DOM) with querySelector →
  rect is more precise and self-verifying. The grid is for surfaces without
  usable semantics (canvas, 3D, unfamiliar pages) and quick vision-loop work.
- **Trust caveat (applies to ALL synthetic pointer/key paths):** dispatched
  events are untrusted — listeners fire but WebKit withholds native default
  actions, notably FOCUS on inputs. To focus + type: `grid click` (or
  `pointer click`) the input, then `app dom-eval "…querySelector(…).focus()"`
  (or `app web eval` in a page), then `app key type`. Note `app key type`
  into Dioxus controlled inputs must go through the prototype value setter +
  InputEvent (dom-eval), or the signal never updates and a re-render wipes
  the text.

## Client-buffer read + daemon diff (rendering-corruption probe, 2026-07-10)

THE instrument for the client-buffer garble class (holes, merged rows,
interleaved frames): the daemon vt100 screen is clean while the CLIENT xterm
buffer is corrupt, so a daemon-only probe proves nothing (CLAUDE.md misstep
#3). `window.__yggtermXtermHosts[hostId].term` is exposed — read the client
viewport rows directly via dom-eval, focus-independent:

```bash
LIVE_HOST=$(cat .agents/config/live-host)
# 1. Client xterm viewport rows (the ACTIVE host), via translateToString
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app dom-eval '
  const hosts = window.__yggtermXtermHosts || {};
  const path = String(window.__yggtermActiveTerminalSessionPath || \"\");
  const entry = Object.values(hosts).find(e => e && e.sessionPath === path && e.term);
  if (!entry) return { error: \"no active host entry\", path };
  const buf = entry.term.buffer.active;
  const rows = [];
  for (let i = 0; i < entry.term.rows; i++) {
    const ln = buf.getLine(Number(buf.viewportY || 0) + i);
    rows.push(ln && ln.translateToString ? ln.translateToString(true) : \"\");
  }
  return { path, rows };'"
# 2. Daemon truth for the same session
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server snapshot" # -> active_session.terminal_lines
# 3. Diff row-by-row. Client rows with single-cell holes / merged content that
#    the daemon does NOT have = client write-path corruption, NOT a PTY bug.
```

Companion trace events (mine `~/.yggterm/event-trace.jsonl`):
- `terminal_forward_divergence` — the GUI forwarded FEWER/DIFFERENT bytes to
  xterm than the daemon sent (batch sanitizers rewrote or dropped a live
  chunk). `dropped_entirely: true` = a whole batch vanished.
- `terminal_write_send_failed` — an `eval.send(Write)` failed; those bytes are
  permanently lost to the client buffer (cursor already advanced).
- `terminal_render_health_unhealthy` — canvas/paint layer only; its
  `cursor_line_text` IS a client-buffer read (translateToString), so holes in
  it are buffer corruption evidence, not just paint.

## DOM Eval (main-webview JS probe)

```bash
# Evaluate JS in the MAIN webview (Dioxus chrome); script body must `return`
# a JSON-serializable value. The missing eye that `app web eval` (child
# webviews) cannot provide: focus state, rects, attributes of GUI elements.
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app dom-eval 'return {active: String(document.activeElement.tagName)}'"
```

### ⛔ The `return` trap — a missing `return` is indistinguishable from "absent"

The script is spliced into an **async function body**, not evaluated as an
expression:

```js
result = await (async () => { <YOUR SCRIPT> })();
dioxus.send(result === undefined ? null : result);
```

So an expression-style probe yields `{"result": null}` — **the exact same
response as a property that does not exist.** This burned a real investigation:
probing a freshly deployed telemetry field returned `null`, which read as "the
probe did not ship", and the next move was nearly to re-deploy a working binary.

**Always put a `sanity` term in the probe.** If `sanity` comes back, `null` means
absent; if `sanity` is missing too, your script never returned:

```bash
# WRONG — silently null, proves nothing
... dom-eval 'Object.keys(window.__yggtermXtermHosts)'
# RIGHT — self-validating
... dom-eval 'return JSON.stringify({sanity: 1+1, hosts: Object.keys(window.__yggtermXtermHosts||{})})'
```

### Multi-line probes: send a FILE, not a quoted string

Nested quoting through `ssh` + shell + JSON mangles anything non-trivial. Write
the probe locally, `scp` it, and expand it on the remote side:

```bash
cat > /tmp/probe.js <<'EOF'
var h = window.__yggtermXtermHosts || {};
return JSON.stringify({
  sanity: 1 + 1,
  entries: Object.keys(h).map(function (k) {
    return {host: k, paintRatePerSec: h[k].paintRatePerSec, repaintStormMs: h[k].repaintStormMs};
  })
});
EOF
scp -q /tmp/probe.js "$LIVE_HOST":/tmp/probe.js
ssh -n "$LIVE_HOST" '~/.local/bin/yggterm-headless server app dom-eval "$(cat /tmp/probe.js)"'
```

### Reading the terminal render probes (what proves the GUI is the NEW build)

`window.__yggtermXtermHosts[<hostId>]` carries the live per-host render
telemetry. `paintRatePerSec` / `repaintStormMs` (the ~50 Hz repaint-storm probe)
only appear **after a paint window elapses**, so an idle terminal legitimately
has no values yet — `('paintRatePerSec' in v)` is the field-exists check, and it
is the cheapest proof that a GUI-side probe actually shipped:

```json
{"host": "yggterm-terminal-remote-cc---dev-1c2de8c",
 "hasPaintRateField": true, "paintRatePerSec": 1, "repaintStormMs": 0}
```

`hasPaintRateField: true` ⇒ the new eval script is running. `repaintStormMs > 0`
⇒ a sustained ≥30 paints/s storm is happening RIGHT NOW — the garbled-blink
pathology that plain paint-count health scores "healthy".

The daemon-side twin is `server status → remote_yggterm_retry_total`: **present
but 0** proves the probe shipped; *climbing fast between polls* is the wedged
remote-command spin. Since it is `#[serde(default)]`, a pre-probe daemon also
reports `0` — distinguish them by asking whether the key exists at all, not by
its value.

## Split groups (viewport panes — terminal, document, pinned web tab)

```bash
LIVE_HOST=$(cat .agents/config/live-host)
# Group two sessions into co-visible panes (forces keep-alive on members)
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server app split create [--axis side-by-side|stacked] <path> <path>"
# SPLIT-TABS (2.11.4+, libyggterm Phase 3): pin ONE web tab of a session's
# surface into its own pane — pane 0 keeps the surface chrome + active tab,
# pane 1 is the pinned tab, pure page. Tab ids from the surface's tab strip
# (app tab = 0, user tabs count up).
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server app split web-tab <session_path> <tab_id> [--axis ...]"
# Focus a pane. The PANE INDEX form is the ONLY way to focus a pinned web
# pane (its native webview swallows pointer events). The response's
# `focused_web_host` field is the focus-tenancy probe: which page the
# chrome/page-context owner now answers with for that session.
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server app split focus <session_path> [pane_index]"
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server app split ratio <group_id> <0.0..1.0>"
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server app split ungroup <group_id>"
# State: `server app state` → data.split_view (members are bare paths for
# terminal panes, {session, view:{web:{tab}}} objects for pinned panes).
```

⚠ These verbs exist ONLY in the `yggterm-headless` CLI (the GUI binary's
`server app` parser lacks `split`). ⚠ Split/document verification needs
`--backend os` — the composite screenshot paints the active canvas
full-bleed and is blind to both panes (trap 11) and to native webviews
(trap 2). Pinned-pane geometry probe without pixels: dom-eval the
`[data-ws-pinned-session]`/`[data-ws-pinned-tab]` placeholder rect against
`[data-ws-page]`.

## Terminal Probe (type text into live terminal)

```bash
LIVE_HOST=$(cat .agents/config/live-host)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal probe-type --mode xterm --data '__PROBE__'"
```

## Driving + monitoring user-granted sessions (end-user testing)

The user may explicitly **grant** specific live sessions for the agent to drive and
monitor as a production end-user test (e.g. "I give you access to my erome systemd
and samplenotes sessions"). Only drive sessions the user has explicitly granted in the
current conversation.

**Use `terminal send`, NOT `terminal probe-type`, to drive a session.** They are
different tools:
- **`server app terminal send <S> --data 'X'`** (or `--stdin`) is the DRIVER. It writes
  the bytes straight to the daemon → remote PTY (`AppControlCommand::SendTerminalInput`
  → `terminal_write_app_control_input_async`). Returns `{accepted:true, bytes:N}` when
  the bytes were written. This is what reaches codex/CC's stdin.
- **`server app terminal probe-type <S> --data 'X'`** is a DIAGNOSTIC ONLY. It simulates
  a keypress *inside the webview* (xterm `triggerDataEvent` / DOM KeyboardEvents) and
  reports whether the input gate + echo accepted it. It does NOT reliably reach the
  remote PTY — the JS-simulated `onData` queues locally but the synthetic dispatch
  doesn't drive the real transport the way a hardware keypress does. **A
  `visible_echo_missing` from probe-type does NOT mean input can't be sent** — it means
  the JS simulation didn't echo. Don't conclude "input is broken" from probe-type; use
  `send` to actually drive, then read state to confirm.

```bash
LIVE_HOST=$(cat .agents/config/live-host)
S="remote-session://dev/<uuid>"   # a granted session
# PREFERRED for prompt insertion: `terminal submit` is readiness-gated — it WAITS
# until the session is at an idle interactive codex prompt, then sends; it refuses
# (writes nothing) if the session never becomes ready within --ready-timeout-ms.
# This is the SAFE insertion path. A raw `send` of "...\r" into a session that is
# mid-task, at a menu, or showing a pending update prompt fires Enter into the wrong
# thing (observed live: `/permissions\r` confirmed a pending codex self-update).
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal submit '$S' --data 'What is the status now?' --ready-timeout-ms 30000"
# -> {submitted:true, waited_ms} OR {submitted:false, reason:"...did not reach an idle interactive prompt..."}

# Raw `send` (NO readiness gate) — only when you KNOW the session is at its composer
# (you just confirmed it, or you're answering a menu you can see). Enter is part of
# the data — append \r, or codex won't submit.
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal focus '$S'"
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal send '$S' --data \$'continue\r'"
```

### Arrow keys / menu navigation
`send --data` is raw PTY bytes, so send escape sequences directly with bash `$'...'`.
Down-arrow is `\x1b[B` (normal cursor mode) or `\x1bOB` (application cursor mode — check
`app state` → `xterm_application_cursor_keys_mode`):

```bash
# codex "full access" via /permissions: open menu, Down twice, Enter
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal send '$S' --data \$'/permissions\r'"
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal send '$S' --data \$'\x1b[B\x1b[B\r'"
```
**Confirm the menu opened BEFORE sending arrows+Enter** — blind arrow+Enter into a
non-menu risks selecting the wrong permission level. (Codex full-access selector =
Down ×2 from the top, per the user.) BUT see the observability caveat below: on
KDE/Wayland the screenshot and per-call buffer reads can be stale/inconsistent for a
retained remote session, so "confirm visually" may not be reliable — when in doubt,
don't navigate a destructive menu blind.

### Forcing a repaint
`server app terminal redraw <S>` forces a client repaint/re-read (the programmatic
equivalent of the user pressing `<Esc>` to un-stick a "muffled"/half-painted remote
TUI). Use it after `send` if the viewport looks stale.

### Observability caveat (KDE/Wayland, retained remote sessions) — IMPORTANT
For a remote session that is in a retained/hot-but-not-live-attached state, the
observability surface is currently UNRELIABLE and the readings contradict each other:
- `server app screenshot` can return a STALE frame (Wayland snapshot fallback) that
  doesn't reflect the latest paint.
- `probe-scroll` `visible_text` reads **inconsistently call-to-call** — sometimes the
  live composer text, sometimes empty (`xterm_session_snapshot_reason: focus_released`).
- `redraw`'s own embedded snapshot may show live content while the next probe-scroll
  shows empty.
This inconsistency is itself a tracked bug (see the convergent root cause:
client viewport not reliably live-attached/repainting for retained remote sessions —
the same root as the user-visible "muffled rendering until I press Esc"). Until it's
fixed, cross-check at least two surfaces and treat a single read as low-confidence.

### Rapid-frame capture of loading artifacting
Loading/switch artifacting is transient and inconsistent — hard to describe in words.
Capture a burst of frames right after sending a prompt:

```bash
# ~10 frames, ~1s apart, then pull a strategic subset to inspect
ssh "$LIVE_HOST" 'for i in $(seq 1 10); do ~/.local/bin/yggterm server app screenshot /tmp/load-$i.png >/dev/null 2>&1; sleep 0.6; done'
for i in 1 3 5 7 9; do scp -q "$LIVE_HOST:/tmp/load-$i.png" /tmp/load-$i.png; done
```
Then Read the frames and compare adjacent ones for the artifact (squished width, blank
flash, scroll jump, broken prompt region). Cross-check with `probe-scroll`'s
`dom_census` + buffer state — screenshots can be fuzzy/stale; the xterm buffer text and
counters are the ground truth.

## Panel Navigation

```bash
# Show settings panel
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app panel settings"
# Theme switch
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app theme light"
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app theme dark"
```

## Force Hot-Restart (dev / agent deploys)

When deploying a same-version build (the version_string didn't bump but
the binary did), the daemon's auto-restart never fires — see the
`bug-class-auto-hot-restart-version-gated` memory. To force a hot-restart
that preserves live sessions through a same-version handoff:

```bash
LIVE_HOST=$(cat .agents/config/live-host)
ssh "$LIVE_HOST" "~/.local/bin/yggterm-headless server monitor \
    --scenario hot-restart \
    --daemon-exe /home/user/.local/share/yggterm/direct/versions/<VERSION>/yggterm-headless \
    --expected-version <VERSION> \
    --expected-build-id <NEW_BUILD_ID> \
    --force \
    --reason 'agent deploy <commit-sha>'"
```

What `--force` does (added 2026-05-26):
- Tells the daemon to bypass the "same-version handoff not allowed when
  live runtimes are present" refusal.
- Sessions still preserved via the normal hot-update handoff (new daemon
  takes over PTY ownership before the old daemon exits).

**Bootstrap caveat**: `--force` is honored only when the RUNNING daemon
is the new build. If you're invoking this with the OLD daemon still
running and same version, it refuses (the old daemon doesn't know about
the `force` field — `#[serde(default)]` falls back to false). For
first-time bootstrap of this feature you'll need a natural daemon
restart or a one-time version-patch bump.

### PREFER a plain GUI restart over manual `hot-restart --all` (2026-07-09 lesson)

For a dev/agent deploy (new version on disk), the SIMPLE and correct path is:
**deploy the binaries, then restart the GUI** (SIGTERM the GUI pid, relaunch
`~/.local/bin/yggterm` with the desktop env — `DISPLAY`/`WAYLAND_DISPLAY`/
`XDG_RUNTIME_DIR`/`DBUS_SESSION_BUS_ADDRESS`/`XAUTHORITY` from `/proc/<gui>/environ`,
detached via `setsid nohup … </dev/null &`).

> ⛔ **COPY THE WHOLE ENV, NOT THE FIVE VARIABLES NAMED ABOVE.** The `YGGTERM_`
> prefix carries this host's policy flags, and hand-listing variables silently
> drops whichever one you did not think of.
>
> ⚠ **The under-glass example here has INVERTED — read this before acting on
> older notes.** This warning used to say jojo runs with
> `YGGTERM_WEB_SURFACE_UNDER_GLASS=0` and that dropping it re-arms the F.0
> incident. **As of 2026-07-31 under-glass is armed BY DEFAULT** (user directive:
> a web surface that needs an extra flag to sit flush is wrong by default — see
> `under_glass_default_armed` in `apps/yggterm/src/main.rs`). So an unset variable
> now means ARMED, which is the correct path, and the drop-the-variable trap
> points the other way: a host deliberately running `=0` loses its opt-out if you
> hand-list. The rule is unchanged and now matters in both directions — **copy
> the whole `YGGTERM_` set and do not reason about which flags matter.**
>
> Do this instead of hand-listing variables:
> ```bash
> GUI=$(pgrep -x yggterm | head -1)
> tr '\0' '\n' < /proc/$GUI/environ | grep -E '^(DISPLAY|WAYLAND_DISPLAY|XDG_RUNTIME_DIR|DBUS_SESSION_BUS_ADDRESS|XAUTHORITY|XDG_SESSION_TYPE|HOME|PATH|YGGTERM_)' > /tmp/gui-env
> # …SIGTERM, swap the binary, then:
> ( set -a; while IFS= read -r l; do export "$l"; done < /tmp/gui-env; set +a
>   setsid nohup ~/.local/bin/yggterm >/tmp/yggterm-gui.log 2>&1 </dev/null & )
> ```
> The `YGGTERM_` prefix match is the point — it carries whatever policy flags this
> host needs without you having to know their names.

> ⚠ **ANOTHER AGENT CAN DEPLOY OVER YOU, AND IT LOOKS LIKE YOUR FIX NEVER LANDED.**
> On 2026-07-31 a GUI deploy was verified live (a new `app state` field went from
> `null` to a value), and ~6 minutes later both binaries were replaced by a second
> agent's paired deploy — reverting it, with no error anywhere. **`app state`
> agreeing once is not durable proof.** Before reporting a fix as live, confirm the
> RUNNING binary is yours:
> `strings ~/.local/bin/yggterm | grep -c <a string only your change introduces>`
> (plain `grep -c` on a binary returns 0 — it prints "Binary file matches" instead
> of counting, so it will lie to you). Then `git fetch` and rebase before
> redeploying, or you revert their work exactly as they reverted yours. A newer GUI whose own-version socket
is absent falls back to the running older daemon via `resolve_client_daemon_endpoint`
(logs `gui/startup/daemon_version_mismatch`), serves every session the older daemon
owns with **no re-resume**, and drives that daemon's cooperative hot-update *when the
fleet next idles*. There is NO breaking protocol change between adjacent patch
versions (new request fields are `#[serde(default)]`; new request variants the GUI
never sends unprompted), so a 2.9.x GUI talks to a 2.9.(x-x) daemon fine.

**Do NOT run `server monitor --scenario hot-restart --all` to "land" a deploy while
a busy daemon owns the active fleet.** The idle gate correctly DEFERS the busy
daemon's handoff (any owned agent session active within `HOT_UPDATE_IDLE_THRESHOLD_MS`
= 300s blocks it, and you cannot set `YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE` on an
already-running daemon — it reads its own env live), but a LESS-busy older daemon's
handoff can still succeed, spawning a standalone newer daemon that owns **0** of the
active PTYs. That orphan then binds `server-<newver>.sock`, so a restarted GUI picks
the EMPTY newer daemon over the full older one — sessions show as recovery targets and
`remote-cc://` rows (no cross-daemon proxy; that only covers `remote-session://`
keep-alive) get re-resumed on open. Recovery if you hit this: SIGTERM the orphan
(verify `owned_terminal_session_count == 0` first — safe, it owns no PTYs), `rm` its
stale `.sock`, then restart the GUI so it falls back to the daemon that owns the
sessions. Note the headless CLI's `status`/`snapshot`/`terminal screen` pin to their
OWN-version socket (no fallback like the GUI), so after removing a newer orphan those
probes go blind — verify via the file-based `~/.yggterm/event-trace.jsonl` and
`server app …` (PID-routed app-control, no daemon spawn) instead.

## When to use

- After any UI change: take a before screenshot, apply the fix, take an after screenshot.
- Before reporting a UI change as done: verify visually with a live screenshot.
- When diagnosing a discrepancy between sidebar and start page: take a screenshot and read app state together.
- When debugging session layout, icons, or colors: always verify in the live app, not just from code review.
