#!/usr/bin/env bash
set -euo pipefail

ROOT="${YGGTERM_EXPERIMENT_ROOT:-$HOME/gh}"
MAIN_WORKTREE="${YGGTERM_MAIN_WORKTREE:-$ROOT/yggterm}"
BASE_REF="${YGGTERM_EXPERIMENT_BASE:-origin/main}"
LOG_DIR="${YGGTERM_EXPERIMENT_REBASE_LOG_DIR:-$HOME/.tmp/yggterm}"
BRANCHES="${YGGTERM_EXPERIMENT_BRANCHES:-experimental/alpha-blur experimental/paper-integration experimental/openwebui-integration experimental/excalidraw-obsidian-integration experimental/cellulose-integration}"

mkdir -p "$LOG_DIR"
LOG_PATH="$LOG_DIR/experimental-rebase-$(date +%Y%m%d).log"

log() {
  printf '%s %s\n' "$(date -Is)" "$*" | tee -a "$LOG_PATH"
}

if ! git -C "$MAIN_WORKTREE" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  log "missing main worktree: $MAIN_WORKTREE"
  exit 1
fi

log "fetching origin in $MAIN_WORKTREE"
git -C "$MAIN_WORKTREE" fetch origin --prune

for branch in $BRANCHES; do
  feature="${branch#experimental/}"
  worktree="$ROOT/yggterm--$feature"

  if ! git -C "$worktree" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    log "skip $branch: missing worktree $worktree"
    continue
  fi

  current_branch="$(git -C "$worktree" rev-parse --abbrev-ref HEAD)"
  if [ "$current_branch" != "$branch" ]; then
    log "skip $branch: $worktree is on $current_branch"
    continue
  fi

  if [ -n "$(git -C "$worktree" status --porcelain)" ]; then
    log "skip $branch: worktree has uncommitted changes"
    continue
  fi

  log "rebasing $branch onto $BASE_REF"
  if git -C "$worktree" rebase "$BASE_REF"; then
    log "rebased $branch"
  else
    log "rebase stopped for $branch; resolve in $worktree"
    exit 1
  fi
done

log "experimental rebase pass complete"
