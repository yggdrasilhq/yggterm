#!/usr/bin/env python3
"""Which live daemons can no longer be reached by their own name?

⛔⛔ THE QUESTION `server daemons` CANNOT ANSWER, AND THE REASON IS STRUCTURAL.
That verb enumerates versioned socket NAMES and dials them. A daemon whose name
has been re-pointed at a successor has no name left to enumerate, so the one
instrument anybody reaches for to find it is blind to it by construction —
measured on a build host: `server daemons` reported ONE daemon while a second
was alive beside it holding 83 PTY masters.

⇒ This asks the KERNEL instead, which cannot be fooled by a filesystem name:

    /proc/net/unix   ->  which path each socket was BOUND to, and its inode
    /proc/<pid>/fd   ->  which process holds that inode
    os.path.islink   ->  whether that path still resolves to that socket

An unlinked or replaced unix socket is still bound and still accepting; it is
only unreachable. That is why a stranded daemon looks healthy from the inside
and absent from the outside, and why every session it owns answers "no session
here matches" from every other daemon on the host.

⚠ THIS SCRIPT'S PROPER HOME IS `server daemons`, AND HALF OF IT ALREADY EXISTS.
`daemon_process_pids(home)` in `daemon.rs` is the same `/proc` scan, is already
correct, and already carries the `None is "could not ask"` warning — but it has
exactly ONE caller (the drafts sweep), which is why `server rows drafts` reports
`daemons_running_but_never_reached` while `server daemons` reports one daemon on
the same host. What this adds on top is only the NAME-RESOLUTION check: is the
path a daemon is bound to still resolving to that daemon?

⇒ Fold BOTH into `server daemons` and delete this file. Two answers to one
question is what this repo forbids, and at the moment there are three.

Usage:  scripts/daemon-name-census.py [YGGTERM_HOME]
Exit 1 if any live daemon has lost its name, so a watcher can gate on it.
Read-only: it opens nothing, signals nothing, and never writes.
"""
import os
import re
import sys


def bound_server_sockets(home):
    """path -> inode, straight from the kernel's own bind table."""
    bound = {}
    prefix = os.path.join(home, "server-")
    try:
        table = open("/proc/net/unix", errors="replace")
    except OSError:
        return bound
    with table:
        for line in table:
            parts = line.split()
            # The bound path is last; the inode is the field before it. A socket
            # with no name at all has fewer fields and is simply not ours.
            if len(parts) >= 8 and parts[-1].startswith(prefix):
                bound[parts[-1]] = parts[-2]
    return bound


def inode_holders():
    """inode -> pid. Unreadable processes are skipped, never counted as absent."""
    holders = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        fd_dir = f"/proc/{entry}/fd"
        try:
            names = os.listdir(fd_dir)
        except OSError:
            continue  # ⛔ "could not read" is not "does not hold"
        for fd in names:
            try:
                target = os.readlink(f"{fd_dir}/{fd}")
            except OSError:
                continue
            match = re.fullmatch(r"socket:\[(\d+)\]", target)
            if match:
                holders[match.group(1)] = entry
    return holders


def master_count(pid):
    """How many PTY masters this process holds.

    ⚠ `/dev/ptmx`, never `fuser` on a slave: a master is an unnamed descriptor
    and can never appear among the holders of `/dev/pts/N`, healthy or not.
    """
    fd_dir = f"/proc/{pid}/fd"
    try:
        names = os.listdir(fd_dir)
    except OSError:
        return None
    total = 0
    for fd in names:
        try:
            if os.readlink(f"{fd_dir}/{fd}").endswith("ptmx"):
                total += 1
        except OSError:
            continue
    return total


def main():
    home = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
        "YGGTERM_HOME", os.path.join(os.path.expanduser("~"), ".yggterm")
    )
    bound = bound_server_sockets(home)
    if not bound:
        print(f"no daemon is bound to a versioned socket under {home}")
        return 0
    holders = inode_holders()

    print(f"{'bound name':<38} {'pid':>9} {'masters':>8}  reaches")
    print("-" * 78)
    stranded = []
    for path, inode in sorted(bound.items()):
        pid = holders.get(inode)
        masters = master_count(pid) if pid else None
        if os.path.islink(path):
            target = os.path.basename(os.path.realpath(path))
            verdict = f"⛔ {target} — NAME LOST"
            stranded.append((path, pid, masters))
        else:
            verdict = "✅ itself"
        print(
            f"{os.path.basename(path):<38} {pid or '?':>9} "
            f"{'?' if masters is None else masters:>8}  {verdict}"
        )
    print("-" * 78)
    if not stranded:
        print("every live daemon still answers to its own name")
        return 0
    held = sum(m or 0 for _, _, m in stranded)
    print(
        f"{len(stranded)} live daemon(s) cannot be reached by their own name, "
        f"holding {held} PTY master(s) between them."
    )
    print(
        "Their sessions are alive and undrivable: the GUI dials the name, a "
        "different daemon answers, and it owns none of those rows."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
