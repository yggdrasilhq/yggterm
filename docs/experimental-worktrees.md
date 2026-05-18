# Experimental Worktree Protocol

This document records the local workflow for keeping `main` stable while several
experimental Yggterm integrations move in parallel.

## Standing Decisions

- `main` is the stable, release-ready branch. End-user releases come from
  `main`.
- Experimental branches are named `experimental/<feature>`.
- Each experiment gets a sibling worktree under `~/gh/yggterm--<feature>` so
  separate Codex sessions can work without sharing a dirty tree.
- Experiments rebase onto `origin/main` daily before new work starts. Do not
  merge `main` into an experiment branch.
- The default local rebase helper is
  `scripts/rebase_experimental_worktrees.sh`. It fetches once, rebases only clean
  experiment worktrees, skips dirty or missing worktrees, and stops on conflicts.
- Experimental releases use `yggterm-` prefixed binary and package names. The
  stable binary remains `yggterm`.

## Worktree Layout

| Worktree | Branch | Channel Binary | Scope |
| --- | --- | --- | --- |
| `~/gh/yggterm` | `main` | `yggterm` | Stable releases and shared infrastructure |
| `~/gh/yggterm--alpha-blur` | `experimental/alpha-blur` | `yggterm-alpha-blur` | Visual/compositor experiment |
| `~/gh/yggterm--paper-integration` | `experimental/paper-integration` | `yggterm-paper` | Paper surface inside Yggterm |
| `~/gh/yggterm--openwebui-integration` | `experimental/openwebui-integration` | `yggterm-openwebui` | OpenWebUI workflow integration |
| `~/gh/yggterm--excalidraw-obsidian-integration` | `experimental/excalidraw-obsidian-integration` | `yggterm-excalidraw-obsidian` | Obsidian/Excalidraw workflow integration |
| `~/gh/yggterm--cellulose-integration` | `experimental/cellulose-integration` | `yggterm-cellulose` | Cellulose spreadsheet surface inside Yggterm |

## Setup Commands

Run from `~/gh/yggterm`:

```bash
git fetch origin --prune
git worktree add ../yggterm--alpha-blur experimental/alpha-blur
git worktree add ../yggterm--paper-integration -b experimental/paper-integration origin/main
git worktree add ../yggterm--openwebui-integration -b experimental/openwebui-integration origin/main
git worktree add ../yggterm--excalidraw-obsidian-integration -b experimental/excalidraw-obsidian-integration origin/main
git worktree add ../yggterm--cellulose-integration -b experimental/cellulose-integration origin/main
git branch --unset-upstream experimental/paper-integration
git branch --unset-upstream experimental/openwebui-integration
git branch --unset-upstream experimental/excalidraw-obsidian-integration
git branch --unset-upstream experimental/cellulose-integration
```

If a branch already exists, omit `-b` and pass the branch name.

## Daily Rebase

Before making new changes in an experiment:

```bash
cd ~/gh/yggterm
scripts/rebase_experimental_worktrees.sh
```

Manual equivalent inside a single experiment worktree:

```bash
git fetch origin --prune
git status --short
git rebase origin/main
```

Rules:

- rebase only after the worktree is clean, or after intentionally stashing local
  work
- resolve conflicts in the experiment worktree, then continue the rebase there
- push rebased experiment branches with `git push --force-with-lease` only after
  reviewing the rewritten branch
- never run unattended release or install commands from a conflicted worktree

## Agent Bootstrap

Agents working on this project should treat this file as the standing workflow
for experimental branch hygiene.

At the start of an experimental-branch session, an agent should:

1. read `AGENTS.md` and this file
2. verify the current worktree path and branch
3. run `git fetch origin --prune`
4. rebase the experiment branch onto `origin/main` only when the worktree is
   clean
5. stop and report conflicts instead of merging `main`
6. keep release artifacts, binary names, and state homes channel-specific

For multi-worktree maintenance, prefer:

```bash
cd ~/gh/yggterm
scripts/rebase_experimental_worktrees.sh
```

The helper is intentionally conservative. It skips dirty, missing, or
wrong-branch worktrees and exits on the first rebase conflict.

## Scheduling Options

Daily rebasing can be scheduled with either user systemd timers or cron. The
scheduled job must run the same conservative helper and must not auto-stash,
auto-resolve, commit, push, install, or publish.

Preferred systemd user timer:

```bash
mkdir -p ~/.config/systemd/user
$EDITOR ~/.config/systemd/user/yggterm-experiment-rebase.service
$EDITOR ~/.config/systemd/user/yggterm-experiment-rebase.timer
systemctl --user daemon-reload
systemctl --user enable --now yggterm-experiment-rebase.timer
systemctl --user list-timers yggterm-experiment-rebase.timer --all
systemctl --user status yggterm-experiment-rebase.timer --no-pager
```

Service file:

```ini
[Unit]
Description=Rebase Yggterm experimental worktrees
Documentation=file:%h/gh/yggterm/docs/experimental-worktrees.md

[Service]
Type=oneshot
Environment=YGGTERM_EXPERIMENT_ROOT=%h/gh
ExecStart=%h/gh/yggterm/scripts/rebase_experimental_worktrees.sh
```

Timer file:

```ini
[Unit]
Description=Daily Yggterm experimental worktree rebase

[Timer]
OnCalendar=*-*-* 06:15:00
Persistent=true
RandomizedDelaySec=10m
Unit=yggterm-experiment-rebase.service

[Install]
WantedBy=timers.target
```

Cron fallback:

```cron
15 6 * * * YGGTERM_EXPERIMENT_ROOT="$HOME/gh" "$HOME/gh/yggterm/scripts/rebase_experimental_worktrees.sh" >> "$HOME/.tmp/yggterm/experimental-rebase-cron.log" 2>&1
```

On this machine, cron is unavailable, so the daily local rebase pass is wired as
a user systemd timer. The local timer runs around 06:15 local time with a small
randomized delay. Logs are written under
`~/.tmp/yggterm/experimental-rebase-YYYYMMDD.log`.

## Experimental Releases

Experimental release channels are for testing, not general consumption.

- Binary names, package names, desktop files, app ids, and install metadata must
  carry the channel identity. Examples: `yggterm-paper`,
  `yggterm-paper-headless`, and a Paper-specific desktop identity.
- Stable direct installs and self-update metadata must not be overwritten by
  experimental installs.
- Use isolated state by default:

```bash
YGGTERM_HOME="$HOME/.yggterm-experimental/paper"
```

- When intentionally testing migration or compatibility with the stable
  `~/.yggterm` home, snapshot the state files first and record the exact channel
  build under test.
- The default experimental GitHub Actions release may produce Linux x64
  artifacts, a `.deb`, and checksums only.
- Use the full Linux/macOS/Windows matrix when the feature touches installers,
  desktop identity, compositor behavior, terminal runtime behavior, or any
  platform-specific code.
- Tag names should identify the channel, for example
  `experimental-paper-v2.6.40.1`, so they cannot be mistaken for stable tags.

## Promotion To Main

An experiment can move toward `main` only after:

- the feature scope doc in `docs/experiments/` is updated with the actual shipped
  behavior
- unstable channel identity is removed or converted into stable feature flags
- runtime, session, app-control, and release-gate docs are reconciled if the
  experiment touched those contracts
- deterministic smokes or unit tests cover the defect classes and user-visible
  workflows added by the experiment
- release notes explain what users will actually feel

## Experiment-Local Agent Docs

Experimental worktrees can behave like separate projects while the feature is in
flight. It is acceptable for a branch to carry task-specific changes to
`AGENTS.md`, helper docs, scripts, checklists, or local workflow notes when that
helps agents work safely inside the experiment.

Those files are branch-local by default. They must not be merged into `main`
accidentally just because the feature code is ready.

Before opening or landing a promotion PR, review every branch-local instruction
file and sort it into one of these outcomes:

- **drop:** temporary experiment notes, branch-specific operator shortcuts,
  throwaway checklists, and instructions that only made sense while exploring
- **merge:** durable product, architecture, design, release, smoke-test, or
  workflow rules that should become stable project policy
- **keep branch-only:** instructions that remain useful for an ongoing
  experimental release channel but should not affect stable Yggterm
- **move:** reusable app/platform guidance that belongs in another repo or
  shared YggUI document rather than in Yggterm `main`

For `AGENTS.md` specifically, prefer a deliberate reconciliation commit. Do not
let a branch-specific mission, proof bar, binary name, release channel, or
experimental shortcut silently replace the stable agent contract.

## Standalone App Boundary

Paper and Cellulose are intended to be independently valuable standalone apps:

- local checkouts: `~/gh/paper` and `~/gh/cellulose`
- GitHub owner: `github.com/avikalpa`
- license: Apache-2.0
- Yggterm integration should embed or adapt their app-grade surfaces rather than
  turning Yggterm into the only host

The integrated Yggterm variants use Yggterm's metadata tree for left-side
navigation and store their Yggterm-owned data under the selected `YGGTERM_HOME`.
The standalone apps may use their own file or database layouts.
