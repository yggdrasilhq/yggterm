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
| Progressive migration release | Per-session | `session_is_migratable_now` (idle, no draft, no foreground command, not "working", and — for agent rows — the CLI's own transcript stale past the same threshold, [11.64]) |
| Cold shutdown / self-retire | **Yes** — kills PTYs | `hot_update_idle_gate_block_reason` |

### The two bugs

- **The idle gate guarded the wrong branch.** It sat in front of both, so one
  active agent session deferred the *preserving* handoff for all seventeen. And
  since progressive migration only starts after a handoff, the machinery built to
  tolerate a few busy sessions could never start because a few sessions were busy.
  guihost sat on 2.9.63 for a day.
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

## The orphan-zero contract (2026-09-05, tightened after the all-CLI attach plague)

At fleet update velocity — a dev host redeploys many times a day — daemon
generations turn over constantly, and **every generation boundary is a chance
to orphan the CLI children of the dying daemon.** An orphan (reparented to
init, its terminal and owning daemon both gone) is a HANDOVER FAULT, never a
user problem: the user cannot close a terminal that does not exist, and a
resume that refuses forever behind "end it with `kill …`" is the mechanism
telling a human to do its janitorial work. The contract:

1. **A clean handover leaves zero orphaned holders.** The preserved owner
   keeps its children alive by design; only a death WITHOUT handover (crash,
   SIGKILL, cold-shutdown race) creates orphans, so the recovery arms exist
   for them and must converge without a human.
2. **The environ marker proves yggterm BIRTH, not the session's name.** It is
   fixed at exec and only yggterm writes it, so a marker naming ANY row —
   including a row uuid that is not the agent session id (the
   `opencode-runtime://<uuid>` vs `ses_…` split, [11.63]) — is the full
   "ours" proof; argv names which session the process holds. Demanding the
   marker repeat the session id silently narrowed the recovery to the one
   CLI family whose rows are session-named.
3. **Dead output makes any orphan a corpse, whatever its stamp.** A
   `/dev/pts/N` entry exists only while some process holds that PTY's master;
   both output fds pointing into vanished pts devices means nobody can ever
   read the holder again, and no live parent can adopt it (owner-ruled
   2026-09-05). Everything still observable — a live parent, a live pts, a
   file, a pipe, an unreadable fd — keeps wait-and-banner: "cannot say dead"
   never widens the kill.
4. **The reap is bounded and named.** One SIGTERM round to the process tree,
   one short yield, one rescan; the reap events carry `reason`
   (`stranded_orphan` | `orphaned_dead_output`) and a surviving refusal
   carries per-holder `why_not_reaped` notes, so the next banner arrives
   with its own diagnosis instead of costing a hand investigation.
5. **Adoption must be POSSIBLE at binding, and must CONVERGE after it**
   (from [11.56]; the convergence half implemented in
   `lane/cli/same-version-adoption`). Binding first is still correct — the
   canonical name must answer or old clients find no daemon at all — but the
   handover is only honest when the preserved owner's sessions actually
   drain. The migration drain's adopter gate now probes the canonical
   endpoint FIRST: a live answer from a foreign pid there IS the adopter
   (the same-version successor that took our name, or a bequest alias to a
   newer one); the strictly-newer-version probe remains the cross-version
   arm. A stalled drain announces `progressive_migration_no_adopter` once a
   minute, and the first adopter seen is traced with the arm that answered
   (`progressive_migration_adopter_seen`). Before this, the gate accepted
   only a strictly-newer VERSION peer — invisible in the fleet's dominant
   same-version newer-build swaps — so predecessors kept every PTY until
   process death and successors stayed owned:0.
