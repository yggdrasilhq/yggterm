#!/usr/bin/env bash
# scripts/deploy-dev.sh — Deterministic dev binary deployment and drift detection.
#
# ⛔ WHY THIS EXISTS:
# During development and active iteration, yggterm has canonical binary paths
# on every host:
#   1. ~/.local/bin/yggterm
#   2. ~/.local/bin/yggterm-headless
#   3. ~/.local/bin/ynpm
#   4. ~/.yggterm/bin/yggterm
#   5. ~/.yggterm/bin/yggterm-headless
#   6. ~/.yggterm/bin/ynpm
#
# When developers or agents deploy ad-hoc (e.g. `scp target/release/yggterm host:~/.local/bin/`),
# they often miss ~/.yggterm/bin or yggterm-headless, or fail to restart the daemon.
# This causes subtle version drifts where different CLI invocations, SSH handoffs,
# or running daemons execute mismatched binary versions.
#
# This tool provides:
#   1. Deterministic atomic installation to all 4 canonical paths on every target host.
#   2. Cryptographic checksum read-back verification (guaranteeing exact byte match).
#   3. Automatic daemon stack restart convergence (`server stack restart --force`).
#   4. Instant census and drift detection (`--check` / `--census`).
#
# Usage:
#   scripts/deploy-dev.sh [OPTIONS]
#
# Options:
#   --build, -b          Build release binaries (cargo build --release) before deploying
#   --debug              Use target/debug instead of target/release
#   --from <dir>         Read binaries from custom directory (default: target/release)
#   --host <host>        Deploy to a specific host (e.g. 'local', 'remote-host', 'workstation')
#   --hosts "<h1 h2>"    Deploy to explicit list of hosts
#   --local-only, -l     Deploy only to local machine
#   --remote-only, -r    Deploy only to remote live host (resolved via ygg-live-host.sh)
#   --no-restart         Do not restart running server daemons after deploying
#   --restart, -R        Force restart running server daemons after deploying (default)
#   --check, -c          Audit mode: report disk hashes and running daemons, flag drifts
#   --dry-run, -n        Show actions without modifying files or restarting daemons
#   --quiet, -q          Suppress informational logs
#   --help, -h           Show this help message
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FROM="$REPO/target/release"
DO_BUILD=0
BUILD_PROFILE="release"
RESTART=1
CHECK_ONLY=0
DRY_RUN=0
QUIET=0
HOSTS_OVERRIDE=""
LOCAL_ONLY=0
REMOTE_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --build|-b)
      DO_BUILD=1
      shift
      ;;
    --debug)
      BUILD_PROFILE="debug"
      FROM="$REPO/target/debug"
      shift
      ;;
    --from)
      FROM="$2"
      shift 2
      ;;
    --host)
      HOSTS_OVERRIDE="$2"
      shift 2
      ;;
    --hosts)
      HOSTS_OVERRIDE="$2"
      shift 2
      ;;
    --local-only|-l)
      LOCAL_ONLY=1
      shift
      ;;
    --remote-only|-r)
      REMOTE_ONLY=1
      shift
      ;;
    --restart|-R)
      RESTART=1
      shift
      ;;
    --no-restart)
      RESTART=0
      shift
      ;;
    --check|-c|--census)
      CHECK_ONLY=1
      shift
      ;;
    --dry-run|-n)
      DRY_RUN=1
      shift
      ;;
    --quiet|-q)
      QUIET=1
      shift
      ;;
    --help|-h)
      sed -n '2,32p' "$0" | sed 's/^# //' | sed 's/^#//'
      exit 0
      ;;
    *)
      echo "deploy-dev: unknown option '$1' (see --help)" >&2
      exit 2
      ;;
  esac
done

log() {
  [ "$QUIET" = 1 ] || echo "deploy-dev: $*"
}

warn() {
  echo "deploy-dev: ⚠ $*" >&2
}

err() {
  echo "deploy-dev: ⛔ $*" >&2
}

# Resolve target hosts
HOSTS=()
if [ -n "$HOSTS_OVERRIDE" ]; then
  for h in $HOSTS_OVERRIDE; do
    HOSTS+=("$h")
  done
elif [ "$LOCAL_ONLY" = 1 ]; then
  HOSTS=("local")
elif [ "$REMOTE_ONLY" = 1 ]; then
  LIVE_REMOTE=$("$REPO/scripts/ygg-live-host.sh" 2>/dev/null || true)
  if [ -z "$LIVE_REMOTE" ] || [ "$LIVE_REMOTE" = "local" ] || [ "$LIVE_REMOTE" = "$(hostname -s 2>/dev/null)" ]; then
    err "could not resolve a remote live GUI host (ygg-live-host.sh gave '${LIVE_REMOTE:-none}')"
    exit 1
  fi
  HOSTS=("$LIVE_REMOTE")
else
  # Default: local + live remote host if different
  HOSTS=("local")
  LIVE_REMOTE=$("$REPO/scripts/ygg-live-host.sh" 2>/dev/null || true)
  if [ -n "$LIVE_REMOTE" ] && [ "$LIVE_REMOTE" != "local" ] && [ "$LIVE_REMOTE" != "$(hostname -s 2>/dev/null)" ]; then
    HOSTS+=("$LIVE_REMOTE")
  fi
fi

# De-duplicate hosts
UNIQUE_HOSTS=()
declare -A SEEN_HOSTS=()
for h in "${HOSTS[@]}"; do
  if [ -z "${SEEN_HOSTS[$h]:-}" ]; then
    SEEN_HOSTS["$h"]=1
    UNIQUE_HOSTS+=("$h")
  fi
done
HOSTS=("${UNIQUE_HOSTS[@]}")

# Host classification & self-token
SELF_TOKEN=$(mktemp "$HOME/.ygg-dev-deploy-self-XXXXXX" 2>/dev/null || true)
[ -n "$SELF_TOKEN" ] && trap 'rm -f "$SELF_TOKEN"' EXIT
declare -A HOST_IS_SELF=()
declare -A HOST_UNREACHABLE=()

classify_host() {
  local host="$1" probe
  if [ "$host" = "local" ] || [ "$host" = "$(hostname -s 2>/dev/null)" ] ||
     { [ -n "${YGG_FLEET_SELF:-}" ] && [ "$host" = "$YGG_FLEET_SELF" ]; }; then
    HOST_IS_SELF["$host"]=1
    return 0
  fi
  if [ -z "$SELF_TOKEN" ]; then
    return 0
  fi
  probe=$(ssh -o BatchMode=yes -o ConnectTimeout=6 "$host" \
    "test -e '$SELF_TOKEN' && echo SELF || echo REMOTE" 2>/dev/null < /dev/null || echo "UNREACHABLE")
  case "$probe" in
    SELF)
      HOST_IS_SELF["$host"]=1
      ;;
    REMOTE)
      ;;
    *)
      HOST_UNREACHABLE["$host"]=1
      ;;
  esac
}

is_self() {
  [ "${HOST_IS_SELF[$1]:-0}" = 1 ]
}

run_on_host() {
  local host="$1"
  shift
  if is_self "$host"; then
    bash -c "$*" < /dev/null
  else
    ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" "$*" < /dev/null
  fi
}

for h in "${HOSTS[@]}"; do
  classify_host "$h"
done

# Check mode: inspect and audit without deploying
if [ "$CHECK_ONLY" = 1 ]; then
  log "running census and drift audit across ${#HOSTS[@]} host(s): ${HOSTS[*]}"
  CENSUS_FAIL=0
  for host in "${HOSTS[@]}"; do
    if [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ]; then
      warn "host '$host' is unreachable via ssh"
      CENSUS_FAIL=1
      continue
    fi
    echo "==================== host: $host ===================="
    AUDIT_CMD='
      declare -A HASHES=()
      PATHS=(
        "$HOME/.local/bin/yggterm"
        "$HOME/.local/bin/yggterm-headless"
        "$HOME/.local/bin/ynpm"
        "$HOME/.yggterm/bin/yggterm"
        "$HOME/.yggterm/bin/yggterm-headless"
        "$HOME/.yggterm/bin/ynpm"
      )
      DRIFT=0
      for p in "${PATHS[@]}"; do
        if [ -x "$p" ]; then
          v=$("$p" --version 2>/dev/null || echo "ERR")
          c=$("$p" --build-commit 2>/dev/null || echo "(pre-flag)")
          m=$(md5sum "$p" 2>/dev/null | cut -c1-10)
          HASHES["$p"]="$m"
          printf "  [disk] %-42s %-10s %-16s md5:%s\n" "$p" "$v" "$c" "$m"
        else
          printf "  [disk] %-42s %-10s (missing)\n" "$p" "MISSING"
          DRIFT=1
        fi
      done
      # Check if local/bin vs yggterm/bin match
      if [ -n "${HASHES[$HOME/.local/bin/yggterm]:-}" ] && [ -n "${HASHES[$HOME/.yggterm/bin/yggterm]:-}" ]; then
        if [ "${HASHES[$HOME/.local/bin/yggterm]}" != "${HASHES[$HOME/.yggterm/bin/yggterm]}" ]; then
          echo "  ⛔ DRIFT: ~/.local/bin/yggterm and ~/.yggterm/bin/yggterm binary hashes differ!"
          DRIFT=1
        fi
      fi
      if [ -n "${HASHES[$HOME/.local/bin/yggterm-headless]:-}" ] && [ -n "${HASHES[$HOME/.yggterm/bin/yggterm-headless]:-}" ]; then
        if [ "${HASHES[$HOME/.local/bin/yggterm-headless]}" != "${HASHES[$HOME/.yggterm/bin/yggterm-headless]}" ]; then
          echo "  ⛔ DRIFT: ~/.local/bin/yggterm-headless and ~/.yggterm/bin/yggterm-headless binary hashes differ!"
          DRIFT=1
        fi
      fi
      if [ -n "${HASHES[$HOME/.local/bin/ynpm]:-}" ] && [ -n "${HASHES[$HOME/.yggterm/bin/ynpm]:-}" ]; then
        if [ "${HASHES[$HOME/.local/bin/ynpm]}" != "${HASHES[$HOME/.yggterm/bin/ynpm]}" ]; then
          echo "  ⛔ DRIFT: ~/.local/bin/ynpm and ~/.yggterm/bin/ynpm binary hashes differ!"
          DRIFT=1
        fi
      fi
      if [ -x "$HOME/.yggterm/bin/yggterm-headless" ]; then
        echo "  [running daemons]"
        "$HOME/.yggterm/bin/yggterm-headless" server daemons 2>/dev/null | sed "s/^/    /" || echo "    <none>"
      fi
      exit $DRIFT
    '
    if is_self "$host"; then
      bash -c "$AUDIT_CMD" || CENSUS_FAIL=1
    else
      ssh "$host" bash -c "'$AUDIT_CMD'" || CENSUS_FAIL=1
    fi
  done
  if [ "$CENSUS_FAIL" != 0 ]; then
    warn "drift or missing binaries detected during audit"
    exit 1
  else
    log "census complete — all audited hosts are consistent"
    exit 0
  fi
fi

# Build step if requested
if [ "$DO_BUILD" = 1 ]; then
  log "building $BUILD_PROFILE binaries: yggterm, yggterm-headless, ynpm..."
  if [ "$BUILD_PROFILE" = "release" ]; then
    cargo build --release --bin yggterm --bin yggterm-headless --bin ynpm
  else
    cargo build --bin yggterm --bin yggterm-headless --bin ynpm
  fi
fi

GUI_SRC="$FROM/yggterm"
HL_SRC="$FROM/yggterm-headless"
YNPM_SRC="$FROM/ynpm"

for f in "$GUI_SRC" "$HL_SRC" "$YNPM_SRC"; do
  if [ ! -x "$f" ]; then
    err "missing executable build product: $f (run with --build or cargo build --$BUILD_PROFILE)"
    exit 1
  fi
done

GUI_VER=$("$GUI_SRC" --version 2>/dev/null || echo "unknown")
HL_VER=$("$HL_SRC" --version 2>/dev/null || echo "unknown")
YNPM_VER=$("$YNPM_SRC" --version 2>/dev/null || echo "unknown")
if [ "$GUI_VER" != "$HL_VER" ] || [ "$GUI_VER" != "$YNPM_VER" ]; then
  err "binary version mismatch: yggterm=$GUI_VER vs yggterm-headless=$HL_VER vs ynpm=$YNPM_VER"
  exit 1
fi

GUI_MD5=$(md5sum "$GUI_SRC" | awk '{print $1}')
HL_MD5=$(md5sum "$HL_SRC" | awk '{print $1}')
YNPM_MD5=$(md5sum "$YNPM_SRC" | awk '{print $1}')
BUILD_COMMIT="unknown"
if git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1; then
  BUILD_COMMIT=$(git -C "$REPO" rev-parse --short=12 HEAD)
  if [ -n "$(git -C "$REPO" status --porcelain | head -1)" ]; then
    BUILD_COMMIT="${BUILD_COMMIT}-dirty"
  fi
fi

log "source build: $GUI_VER (commit: $BUILD_COMMIT)"
log "  yggterm:          md5 ${GUI_MD5:0:12}... ($GUI_SRC)"
log "  yggterm-headless: md5 ${HL_MD5:0:12}... ($HL_SRC)"
log "  ynpm:             md5 ${YNPM_MD5:0:12}... ($YNPM_SRC)"
log "deploying to ${#HOSTS[@]} host(s): ${HOSTS[*]}"

push_binary() {
  local host="$1" src="$2" dest="$3" want_md5="$4"
  local write_cmd="d=\$(eval echo $dest); mkdir -p \$(dirname \$d); cat > \$d.new && chmod 755 \$d.new && mv -f \$d.new \$d"
  if is_self "$host"; then
    bash -c "$write_cmd" < "$src"
  else
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$host" "$write_cmd" < "$src"
  fi
  # Verify read-back checksum
  local got_md5
  got_md5=$(run_on_host "$host" "md5sum \$(eval echo $dest) 2>/dev/null" | awk '{print $1}')
  if [ "$got_md5" = "$want_md5" ]; then
    printf "  ✅ %-14s %s\n" "$host" "$dest"
    return 0
  else
    printf "  ⛔ %-14s %s (READBACK %s != EXPECTED %s)\n" "$host" "$dest" "${got_md5:-<none>}" "$want_md5"
    return 1
  fi
}

DEPLOY_FAILED=0
for host in "${HOSTS[@]}"; do
  if [ "${HOST_UNREACHABLE[$host]:-0}" = 1 ]; then
    err "host '$host' is unreachable; skipping"
    DEPLOY_FAILED=1
    continue
  fi

  log "updating binaries on $host..."
  if [ "$DRY_RUN" = 1 ]; then
    echo "  · [dry-run] $host: would push to ~/.local/bin and ~/.yggterm/bin"
    continue
  fi

  push_binary "$host" "$GUI_SRC" "\$HOME/.local/bin/yggterm" "$GUI_MD5" || DEPLOY_FAILED=1
  push_binary "$host" "$HL_SRC" "\$HOME/.local/bin/yggterm-headless" "$HL_MD5" || DEPLOY_FAILED=1
  push_binary "$host" "$GUI_SRC" "\$HOME/.yggterm/bin/yggterm" "$GUI_MD5" || DEPLOY_FAILED=1
  push_binary "$host" "$HL_SRC" "\$HOME/.yggterm/bin/yggterm-headless" "$HL_MD5" || DEPLOY_FAILED=1
  push_binary "$host" "$YNPM_SRC" "\$HOME/.local/bin/ynpm" "$YNPM_MD5" || DEPLOY_FAILED=1
  push_binary "$host" "$YNPM_SRC" "\$HOME/.yggterm/bin/ynpm" "$YNPM_MD5" || DEPLOY_FAILED=1

  # Convergence / stack restart
  if [ "$RESTART" = 1 ] && [ "$DEPLOY_FAILED" = 0 ]; then
    log "restarting server stack on $host for immediate convergence..."
    RESTART_CMD='
      if [ -x "$HOME/.local/bin/yggterm-headless" ]; then
        "$HOME/.local/bin/yggterm-headless" server stack restart --force 2>&1
      elif [ -x "$HOME/.yggterm/bin/yggterm-headless" ]; then
        "$HOME/.yggterm/bin/yggterm-headless" server stack restart --force 2>&1
      fi
    '
    run_on_host "$host" "$RESTART_CMD" | sed 's/^/    /' || warn "stack restart on $host returned non-zero"

    # ⛔ VERIFY CONVERGENCE — the restart verb returns Ok when the REQUEST
    # round-trips, which says nothing about whether the OPERATION happened
    # (the field-guide Ok(()) trap, deploy edition). The daemon owns sessions
    # and may legitimately survive a stack restart; but then the fleet is
    # running the previous binary while this script claims convergence, and
    # every proof taken afterwards measures someone else's code. Ask the
    # daemon's own census, which reports its build commit and flags a binary
    # replaced underneath it.
    VERIFY_CMD='
      sleep 3
      OUT=$("$HOME/.local/bin/yggterm-headless" server daemons 2>/dev/null || "$HOME/.yggterm/bin/yggterm-headless" server daemons 2>/dev/null)
      echo "$OUT" | grep -q "REPLACED ON DISK" && echo "DRIFT: running daemon executes a binary that no longer exists on disk"
      RUNNING_BUILD=$(echo "$OUT" | awk "/^\\*/{print \$4; exit}")
      if [ -n "$RUNNING_BUILD" ] && [ -n "'"$BUILD_COMMIT"'" ] && [ "${RUNNING_BUILD:0:8}" != "${BUILD_COMMIT:0:8}" ]; then
        echo "DRIFT: daemon runs build ${RUNNING_BUILD:0:8} but deployed ${BUILD_COMMIT:0:8} — daemon owns sessions and survived the restart; upgrade it explicitly (session handover), do not trust this deploy for daemon-side behavior"
      else
        echo "converged: daemon build ${RUNNING_BUILD:0:8} matches deployment"
      fi
    '
    run_on_host "$host" "$VERIFY_CMD" | sed 's/^/    /'
  fi
done

if [ "$DEPLOY_FAILED" != 0 ]; then
  err "deployment failed on at least one target host/copy"
  exit 1
fi

log "✅ deterministic dev deployment complete across all target hosts ($GUI_VER, commit: $BUILD_COMMIT)"
