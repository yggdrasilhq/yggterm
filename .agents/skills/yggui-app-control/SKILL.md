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
# -> {requested, live_session_count, changed, order:[...]}

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
  appended after, so a partial list is safe.
- Verify a reconnect with the session's `status_line`/`last_launch_error` from
  `server snapshot`, not `app terminal read-buffer` — the GUI may not have mounted
  the xterm yet (`accepted:false`) even though the resume is healthy.

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

## DOM Eval (main-webview JS probe)

```bash
# Evaluate JS in the MAIN webview (Dioxus chrome); script body must `return`
# a JSON-serializable value. The missing eye that `app web eval` (child
# webviews) cannot provide: focus state, rects, attributes of GUI elements.
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app dom-eval 'return {active: String(document.activeElement.tagName)}'"
```

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
    --daemon-exe /home/pi/.local/share/yggterm/direct/versions/<VERSION>/yggterm-headless \
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

## When to use

- After any UI change: take a before screenshot, apply the fix, take an after screenshot.
- Before reporting a UI change as done: verify visually with a live screenshot.
- When diagnosing a discrepancy between sidebar and start page: take a screenshot and read app state together.
- When debugging session layout, icons, or colors: always verify in the live app, not just from code review.
