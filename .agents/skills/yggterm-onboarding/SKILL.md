---
name: yggterm-onboarding
description: What yggterm sets up on first use, where every piece lives, and how to mold or disable each one. Use when a user asks what got installed, wants to change memory-sync behavior, arm or stop dreaming, add a host, or uninstall anything.
---

# yggterm onboarding — the bare essentials and every mold point

yggterm scaffolds itself on first use so an agent seat can work immediately:
zero permission prompts for the **bare essentials**, nothing hard-coded, and
every piece moldable afterwards. This skill explains what exists and where.

## What first-run bootstrap created

| piece | where | what it does |
|---|---|---|
| memory hub | `~/.yggterm/memory/` | the semantic-memory store all harnesses converge on |
| sync verb | `~/.local/bin/ygg-memory` | native↔hub sync, `now`/`asks`/`doctor`/`dream` verbs |
| sync timer | `~/.config/systemd/user/ygg-memory-sync.{service,timer}` | the 60s recovery-convergence tick; startup syncs are immediate, the timer covers "no seat running" |
| doctor timer | `~/.config/systemd/user/ygg-memory-doctor.timer` | hourly deterministic health probes + bounded self-repair, independent of the tick |
| ask-user ledger | `~/.yggterm/memory/namespaces/_global/ask-user-ledger.md` | where agent-blocked-on-you items compile for one gentle daily review |
| dream config dir | `~/.yggterm/config/dream/` | created but NOT armed — dreaming is opt-in |

## The one arming decision: dreaming

Auto-dreaming costs model tokens and runs on ONE primary host. Arm it deliberately:

```sh
mkdir -p ~/.yggterm/config/dream && touch ~/.yggterm/config/dream/enabled   # on exactly ONE host
```

Additional scoped dreamers (optional, per-host):

```sh
mkdir -p ~/.yggterm/config/dream/dreamers.d
cat > ~/.yggterm/config/dream/dreamers.d/docs-graph.json <<'JSON'
{"name":"docs-graph","ns_scope":["-home-…-docs-graph"],
 "composers":"opencode:opencode/deepseek-v4.1-flash-free,codex",
 "expires_at":"2026-10-23","enabled":true}
JSON
```

A secondary dreamer only ever dreams its `ns_scope`, auto-disarms past
`expires_at`, and still passes budget admission — scopes and spend gates are
never bypassed by adding dreamers.

## Mold points (nothing is hard-coded)

- **Timers**: edit the installed units; add `# YGGTERM_UNIT_TUNED` anywhere in
  the installed unit and bootstrap will never overwrite your version.
- **Composer chain**: `YGG_DREAM_COMPOSERS=opencode:opencode/<model>,codex`
  (free transports first by default; `codex` restores the paid-only chain).
- **Health thresholds**: `YGG_COMPOSER_FAILURE_LIMIT` (default 3),
  `YGG_COMPOSER_SKIP_WINDOW_S` (3600), `YGG_COMPOSER_ESCALATE_AFTER` (8).
- **Fleet mesh**: `~/.config/ygg-fleet/mesh` — empty/absent means solo (no ssh peers).
- **Harness set**: `~/.config/ygg-fleet/memory-sync.conf` `harnesses=` line.
- **Memory root**: `--root` on any verb; the default is the one convention.

## Daily rhythm for the owner

`ygg-memory now` (or your seat's session-start gate) shows what awaits you.
Per day you get ONE review invitation across all sessions and harnesses; on
agreement, up to three contextualized questions. `ygg-memory asks close <id>
--ruling "..."` records the decision; a physical action stays
pending-verification until `--verified` confirms the handoff.

## Disable / uninstall any piece

```sh
systemctl --user disable --now ygg-memory-sync.timer    # stop the tick (intent is respected; the doctor will not re-enable it)
systemctl --user disable --now ygg-memory-doctor.timer  # stop health probes
rm ~/.yggterm/config/dream/enabled                      # disarm the dreamer
```
