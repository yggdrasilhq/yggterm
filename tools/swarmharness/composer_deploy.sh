#!/bin/bash
# composer_deploy — install the fleet lesson miner on THIS host (portable,
# $HOME-relative; owner directive 2026-10-26: "composer or any brain unit
# should be able to run anywhere throughout the fleet").
#
# Usage: composer_deploy.sh [--repo PATH] [--ref GIT_REF] [--arm-timer]
#   --repo      yggterm checkout to copy from (default ~/gh/yggterm)
#   --ref       git ref to read the kernel from when the checkout lacks it
#               (default origin/lane/composer)
#   --arm-timer install + enable the hourly tick timer (arming is EXPLICIT:
#               one host, owner-directed — steer onboarding law)
#
# Enrollment is NEVER auto-enabled: the manifest template lands disabled
# (empty sources) at ~/.yggterm/config/swarm/composer-sources.json unless
# one already exists. Sources are owner-gated (default-deny).
set -eu
REPO="$HOME/gh/yggterm"; REF="origin/lane/composer"; ARM=0
while [ $# -gt 0 ]; do case "$1" in
  --repo) REPO="$2"; shift 2;; --ref) REF="$2"; shift 2;; --arm-timer) ARM=1; shift;;
  *) echo "unknown arg $1" >&2; exit 2;; esac; done
K="$HOME/.yggterm/swarm/composer/kernel"
mkdir -p "$K" "$HOME/.local/bin" "$HOME/.yggterm/config/swarm" "$HOME/.yggterm/log"
SRC="$REPO/tools/swarmharness"
for f in composer.py swarmconfig.py session_review.py; do
  if [ -f "$SRC/$f" ]; then cp "$SRC/$f" "$K/";
  else git -C "$REPO" show "$REF:tools/swarmharness/$f" > "$K/$f"; fi
done
SELF="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SELF/composer-tick.py" ] && [ "$SELF" != "$HOME/.local/bin" ]; then
  cp "$SELF"/composer-tick.py "$SELF"/composer_review.py "$HOME/.local/bin/"
fi
chmod +x "$HOME/.local/bin"/composer-tick.py "$HOME/.local/bin"/composer_review.py 2>/dev/null || true
M="$HOME/.yggterm/config/swarm/composer-sources.json"
if [ ! -f "$M" ]; then
  cat > "$M" <<'JSON'
{
  "owner_directive": "TEMPLATE - NOT ARMED. Sources stay EMPTY until the owner approves this host's enrollment (default-deny: anything unlisted is NOT enrolled).",
  "privacy_law": "fingerprints/counts/byte-spans only; never message bodies; zero model calls",
  "excluded_explicitly": ["~/.ssh", "~/.gnupg", "vaults", "credential stores", "auth.json", "~/.yggterm/auth"],
  "sources": {}, "bounds": {"max_bytes_per_source": 262144, "claim_cap": 5, "max_pages": 10, "max_outbox": 200}
}
JSON
  echo "manifest: TEMPLATE written (disarmed)"
else
  echo "manifest: existing enrollment preserved"
fi
if [ "$ARM" = "1" ]; then
  mkdir -p ~/.config/systemd/user
  cat > ~/.config/systemd/user/composer-tick.service <<UNIT
[Unit]
Description=composer tick - fleet lesson miner
[Service]
Type=oneshot
ExecStart=%h/.local/bin/composer-tick.py
TimeoutStartSec=600
UNIT
  cat > ~/.config/systemd/user/composer-tick.timer <<UNIT
[Unit]
Description=run composer tick hourly
[Timer]
OnCalendar=*:45
Persistent=true
[Install]
WantedBy=timers.target
UNIT
  systemctl --user daemon-reload && systemctl --user enable --now composer-tick.timer
  echo "timer: ARMED (hourly :45)"
fi
python3 -c "import sys, pathlib; sys.path.insert(0, str(pathlib.Path.home()/'.yggterm/swarm/composer/kernel')); import composer; print('kernel import ok')" 2>/dev/null || { echo "KERNEL IMPORT FAILED" >&2; exit 1; }
echo "composer deployed on $(hostname -s)"
