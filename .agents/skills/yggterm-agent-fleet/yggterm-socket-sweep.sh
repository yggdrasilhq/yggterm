#!/usr/bin/env bash
# SessionStart: remove genuinely dead entries from ~/.yggterm/.
#
# ⛔ READ THIS BEFORE "CLEANING UP" ~/.yggterm — IT IS NOT LITTER.
#
# yggterm names its control socket per version (server-<major>-<minor>-<patch>.sock).
# one host had 647 of them on 2026-08-02, going back to 2.1.x, which looks exactly
# like accumulated junk. It is not. Measured that day: 645 were SYMLINKS, all
# pointing at the CURRENT daemon's socket, and only 2 were real sockets — the
# live 2.12.24 daemon still owning sessions, and the new 3.0.0 one.
#
# That fan-in is the cross-version compatibility mechanism: a client built at
# any historical version connects to its own version's path and reaches today's
# daemon. It is how "the user must never have to know which daemon owns what"
# is actually implemented, and the daemon re-points the links when it starts.
# Deleting them breaks older clients for no gain.
#
# So this sweep is deliberately narrow. It removes only:
#   1. DANGLING symlinks — target no longer exists at all.
#   2. Real sockets that REFUSE a connection — nothing is listening.
# Both are positive tests. Anything that answers, or that resolves to something
# that answers, is left alone.
#
# ⛔ Never sweep by age, by version, or by "not the current version".
# Version-coexisting daemons are a constitutional guarantee: an older daemon
# still owning live sessions is doing its job and its socket is load-bearing.
#
# This hook must never fail a session: it always exits 0.

set -uo pipefail

DIR="${YGGTERM_DIR:-$HOME/.yggterm}"
DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

[ -d "$DIR" ] || exit 0

python3 - "$DIR" "$DRY" <<'PY' || exit 0
import os, socket, sys

d, dry = sys.argv[1], sys.argv[2] == "1"
dangling = dead = kept = 0

for name in sorted(os.listdir(d)):
    if not (name.startswith("server-") and name.endswith(".sock")):
        continue
    path = os.path.join(d, name)

    # 1. A symlink whose target is gone is dead weight; one that resolves is
    #    the compatibility fan-in and must survive.
    if os.path.islink(path):
        if not os.path.exists(path):          # follows the link
            if not dry:
                try: os.unlink(path)
                except OSError: continue
            dangling += 1
        else:
            kept += 1
        continue

    # 2. A real socket is dead only if a connect is refused.
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(2)
    try:
        s.connect(path)
        kept += 1
    except (ConnectionRefusedError, FileNotFoundError):
        if not dry:
            try: os.unlink(path)
            except OSError: pass
        dead += 1
    except OSError:
        kept += 1          # busy or slow: assume alive, never guess
    finally:
        s.close()

verb = "would remove" if dry else "removed"
print(f"yggterm-socket-sweep: {verb} {dangling} dangling link(s) + {dead} dead socket(s); "
      f"{kept} live//resolving left alone")
PY
exit 0
