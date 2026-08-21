#!/usr/bin/env python3
"""ygg-procfind must not report the query that carried it.

The tool's guard is "a command line identical to one of my ancestors' is mine,
not a target". That catches ancestors, forked subshells and pipeline siblings,
and it is blind to the one shape a fleet query almost always has: the call
arrives over `ssh <host>` while the caller is standing on that same host, so an
`ssh` client process sits in the target's own process table carrying the search
pattern in its argv, in a tree sshd started fresh.

Measured live before the fix: rows with no agent process reported two pids and
rows with a live agent reported four, the constant two being the caller's shell
and its ssh. The differences still looked right, which is what made it costly —
the absolute counts were read as evidence that dead rows were alive.

The guard added for it is the script's own basename: nothing this tool is asked
to find is ever invoked by the tool's name, so a command line containing it can
only be a copy of the running search.
"""
import os
import subprocess
import sys
import time
import unittest
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
PROCFIND = os.path.join(os.path.dirname(HERE), "ygg-procfind.sh")


def _spawn(cmdline_words):
    """A sleeping process whose /proc/<pid>/cmdline carries cmdline_words verbatim.

    ⚠ Deliberately NOT `bash -c "... ; sleep 30"`: bash execs the last simple
    command in place, so such a decoy becomes a bare `sleep 30` and loses the
    very argv the test is about. Trailing arguments to `python -c` land in argv
    and stay there.
    """
    proc = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(30)"] + cmdline_words,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    for _ in range(100):
        try:
            with open(f"/proc/{proc.pid}/cmdline", "rb") as fh:
                if cmdline_words[-1].encode() in fh.read():
                    break
        except OSError:
            pass
        time.sleep(0.02)
    return proc


def _match(pattern):
    r = subprocess.run(
        ["bash", PROCFIND, "match", pattern],
        capture_output=True, text=True, timeout=30,
    )
    return [ln for ln in r.stdout.split() if ln.strip()]


class ProcfindExcludesTheDeliveredQuery(unittest.TestCase):
    def test_an_ssh_shaped_carrier_is_not_reported_as_a_target(self):
        token = f"ygg-procfind-token-{uuid.uuid4().hex[:12]}"
        # Exactly the shape that fooled it: an ssh client on the target host
        # whose argv holds both the pattern and this script's name. It is neither
        # an ancestor of the search nor byte-identical to one.
        carrier = _spawn(["ssh", "-n", "somehost", "bash",
                          "/tmp/ygg-procfind.sh", "match", token])
        self.addCleanup(carrier.kill)
        found = _match(token)
        self.assertNotIn(
            str(carrier.pid), found,
            "the ssh that delivered the query was reported as a match for it",
        )

    def test_a_real_target_carrying_the_pattern_is_still_reported(self):
        # The control that keeps the guard from being a way to find nothing: a
        # process holding the pattern and NOT naming this script must survive.
        token = f"ygg-procfind-token-{uuid.uuid4().hex[:12]}"
        target = _spawn(["some-agent-cli", "--session-id", token])
        self.addCleanup(target.kill)
        found = _match(token)
        self.assertIn(
            str(target.pid), found,
            "an ordinary process carrying the pattern was excluded",
        )


if __name__ == "__main__":
    sys.exit(0 if unittest.main(exit=False).result.wasSuccessful() else 1)
