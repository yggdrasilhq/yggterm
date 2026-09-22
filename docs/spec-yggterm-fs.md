<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Spec — the `~/.yggterm` filesystem organization

**Status:** normative target, 2026-09-13 (owner-directed: the directory
"accumulates socket junk"). The daemon and its tools created this tree
organically; this spec is the organization it grows into. It governs where
NEW state goes and which owner moves OLD state; nothing moves by ad-hoc
agent action.

## The law

1. **One subtree, one owner.** Every path under `~/.yggterm/` has exactly
   one owning component (the daemon, ynpm, ygg-memory, a skill, a product).
   An agent that writes outside its owner's subtree is writing junk.
2. **Ephemeral state lives in `run/`.** Sockets, pids, locks and other
   boot-scoped files are `run/` residents — wipeable whenever the daemon is
   stopped. Anything that must survive a restart is not `run/`.
3. **Configuration lives in `config/<tool>/`.** Human- or agent-edited
   configuration gets one subdirectory per tool (`config/ydesign/`,
   `config/ynpm/`, …). Loose JSON files at the root are legacy.
4. **Logs and traces rotate under their own roots.** `logs/` for human
   logs; `trace/` for machine event traces with their rotations. Neither
   ever sits at the tree root.
5. **Skills install to `skills/<name>/`.** The organized skill root: every
   installed skill copy (for example `skills/ydesign/SKILL.md`) lives here
   so harnesses load skills from one path on every host.
6. **Scratch is prefixed.** `scratchpad/` is shared between seats and
   hosts-in-spirit: every scratch name carries a unique prefix. Never a
   bare `tmp/`.
7. **Dispersed verbs are replicas.** The ygg verb fleet installs to
   `bin/ygg/` as replicas of the repo SSOT — provenance-recorded,
   drift-checked, never hand-edited; `ygg-disperse` is the only writer.
   See `docs/spec-ygg-verb-dispersal.md`.

## The target layout

```
~/.yggterm/
  apps/            # ynpm: generations, registration, rollback (ynpm-owned; nothing else writes here)
  bin/             # the GUI binary + rollback siblings (deploy-owned)
    ygg/           #   dispersed ygg verb replicas (ygg-disperse-owned; spec-ygg-verb-dispersal.md)
  bridge/          # per-subsession mailboxes (the fleet bridge)
  cli-staging/     # managed-CLI download staging
  config/          # configuration, one subdir per tool
    ydesign/       #   projects.json (the ydesign registry, 1.0.0)
    ynpm/          #   channel + recipe state
    ygg-verbs/     #   dispersal manifest (provenance of the bin/ygg replicas)
  kasten/          # the zettelkasten data
  logs/            # daemon.log, launch logs, incident snapshots (rotating)
  managed-cli/     # per-CLI adapter state + dist
  manual-snapshots/
  memory/          # the fleet memory hub mirror (ygg-memory-owned)
  run/             # SOCKETS, PIDS, LOCKS — ephemeral by definition
  scratchpad/      # agent scratch, unique-prefixed names only
  skills/          # installed skill copies (skills/<name>/SKILL.md)
  trace/           # event-trace.jsonl + rotations, ytrace streams
  automations.json # scheduled automations
  install-state.json
  workspace.db
```

Product subtrees (`ychrome/`, `yedit/`, `ymacs/`, `ynpm/`, `yrdp/`,
`web-*`, …) are owned by their products and follow the same laws internally:
config under `config/`, ephemera under `run/`, logs under `logs/`.

## Migration

Legacy paths (root-level `server-*.sock`, root-level `*.log`,
`ytrace*.jsonl` rotations, loose state JSON) keep working until their OWNER
component moves them: the daemon moves its sockets to `run/` and its logs to
`logs/` in a release; agents never `mv` live state by hand. New components
MUST start on the target layout. A legacy path that outlives its migrator is
a defect against this spec, filed in `docs/pending-bugs.md`.
