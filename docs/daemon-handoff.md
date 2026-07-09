# Daemon handoff: no session left behind

**Spec.** Daemons update themselves, and an upgrade never costs the user a
session. Not a patch bump, not a minor bump, not a major bump, not a protocol
break. "Magically updated daemons, no sessions lost" is a yggterm USP, not a
nice-to-have.

## The invariants

1. **A session is never lost to an upgrade.**
2. **Daemons update themselves.** A newer binary on disk is enough. The running
   daemon hands off to it, *keeping its PTY file descriptors*, and lingers as the
   preserved owner while progressive migration drains its sessions one at a time
   as each goes idle. The user orchestrates nothing.
3. **No version relation may BLOCK a handoff.** Version skew is something to
   transition across, never a reason to refuse.
4. **A client NEVER spawns a daemon beside one that owns terminal runtimes** —
   whatever the version relation. If we cannot hand off yet, we attach and wait.
   We do not fork the world.
5. **A breaking protocol change ships its own transition protocol in the binary.**
   Compatibility is the newer side's responsibility, always.

## Handoff is not destructive — this is the load-bearing fact

Two separate bugs came from forgetting it, and both cost the user real work.

The hot-restart handoff **preserves** every runtime. Its own success message is
`preserving N live terminal runtime(s)`: the old daemon keeps its PTY fds and
lingers as the preserved owner while the successor adopts the streams. Nothing is
re-resumed and no in-flight turn is interrupted.

The **cold shutdown** fallback is the destructive one: it kills the PTYs, and the
next client recovery-spawns a daemon that re-resumes every agent.

Guards belong on the second, never the first.

| Mechanism | Destructive? | Guarded by |
| --- | --- | --- |
| Hot-restart handoff | No — preserves PTYs | nothing; it is always safe |
| Progressive migration release | Per-session | `session_is_migratable_now` (idle, no draft, no foreground command, not "working") |
| Cold shutdown / self-retire | **Yes** — kills PTYs | `hot_update_idle_gate_block_reason` |

### The two bugs

- **The idle gate guarded the wrong branch.** It sat in front of both, so one
  active agent session deferred the *preserving* handoff for all seventeen. And
  since progressive migration only starts after a handoff, the machinery built to
  tolerate a few busy sessions could never start because a few sessions were busy.
  jojo sat on 2.9.63 for a day.
- **"Incompatible" versions were refused.** `daemon_versions_share_patch_line`
  demanded the same major *and* minor before preserving or handing off, on the
  premise that a cross-version restart would be destructive. It is not. The rule
  stranded sessions across every minor bump, and then let the GUI fall through to
  spawning a rival daemon beside the one holding them.

## Version policy

- **Handoff compatibility keys on the MAJOR version** (`daemon_versions_can_hand_off`).
- **Preservation keys on nothing.** A daemon that owns terminal runtimes is
  preserved whatever its version — never spawn beside a runtime owner.
- **A major bump owes a transition protocol.** Until it ships, a client attaches
  to the old daemon and preserves it rather than stranding its sessions. The
  transition protocol's job: drive the old daemon's `HotRestart` (every version
  has understood it) and adopt its preserved-owner registry, whose
  `schema_version` is the versioned contract.

## Never close a session by typing into it

Writing `/exit\r` (Claude Code), `/quit\r` (codex) or `exit\r` (shells) into a
PTY appends the text to whatever the user has already typed **and submits it**.
It also never bought a graceful exit: the old code waited 300ms, then SIGKILLed.

`shutdown_all` is the only thing that writes, and it is reached from exactly one
request: `ServerRequest::Shutdown`. `RetireDaemon` never touches terminals;
neither does the handoff; neither does progressive migration's release. Since
2.9.66, `terminal_stop_command` returns `None` for anything with a prompt, so
even `shutdown_all` signals (SIGHUP → SIGTERM → SIGKILL).

Daemons older than 2.9.66 still write, and we cannot teach them. So
`yggterm_server::shutdown` is the single chokepoint: a legacy daemon is asked to
`RetireDaemon` and, if it lingers, signalled. Closing the PTY master delivers
SIGHUP to its children — what a terminal emulator does when its window closes.

## Driving it

```sh
# Bring every reachable local daemon onto this binary's version, preserving PTYs.
yggterm-headless server update-daemons --force
```

`--force` bypasses the daemon's same-version target check (the dev/agent deploy
case). It does not bypass the idle gate, which now guards only the cold shutdown.
The command never sends `Shutdown`.

Before any deploy, check the **daemon's** version, not the binary's:

```sh
yggterm-headless server status        # server_version
ps -eo pid,lstart,cmd | grep "[s]erver daemon"
```

More than one daemon should no longer happen on its own. If it does, that is a
bug — read `docs/daemon-handoff.md` and the incident notes before "fixing" the
handoff, which is working as designed.
