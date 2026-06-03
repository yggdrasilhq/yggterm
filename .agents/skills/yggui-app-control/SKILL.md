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

## App State

```bash
LIVE_HOST=$(cat .agents/config/live-host)
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app state" | python3 -m json.tool 2>/dev/null || true
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
current conversation. Pattern:

```bash
LIVE_HOST=$(cat .agents/config/live-host)
S="remote-session://dev/<uuid>"   # a granted session
# 1) focus first — input is gated on focus; a cold-attached session is often
#    input_enabled=false until focused (focusing re-enables it).
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal focus '$S'"
# 2) type a prompt
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal probe-type '$S' --data 'continue' --enter"
```

**PRECONDITION — codex/CC must be at its interactive composer.** probe-type input only
lands when the agent CLI is at its live input prompt. Check the probe-type result:
`visible_echo: true` ⇒ it landed; **`reason: visible_echo_missing`** ⇒ it did NOT —
the CLI is showing normal-buffer content (a transcript / finished output), not the
composer, so nothing you send registers. Don't keep sending into a non-composer; ask
the user to bring the CLI to its idle prompt, or capture-and-confirm first.

### Arrow keys / menu navigation (no extra flags needed)
probe-type `--data` is sent as **raw PTY bytes** (via the xterm core trigger), so send
escape sequences directly using bash `$'...'` quoting. Down-arrow is `\x1b[B` (normal
cursor mode) or `\x1bOB` (application cursor mode — check
`app state` → `xterm_application_cursor_keys_mode`):

```bash
# codex "full access" via /permissions: open menu, Down twice, Enter
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal probe-type '$S' --data '/permissions' --enter"
ssh "$LIVE_HOST" "~/.local/bin/yggterm server app terminal probe-type '$S' --data \$'\x1b[B\x1b[B\r'"
```
**Verify the menu opened with a screenshot BEFORE sending Enter** — sending arrows+Enter
blind into a non-menu risks selecting the wrong permission level. (Codex full-access
selector = Down ×2 from the top, per the user.)

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
