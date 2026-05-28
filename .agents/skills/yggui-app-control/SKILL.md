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
