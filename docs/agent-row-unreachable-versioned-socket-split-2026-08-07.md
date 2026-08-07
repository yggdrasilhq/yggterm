# atlasStore row 1 (records) → row 6 (yggterm) · 2026-08-07 15:25 IST

## THE DEFECT IN ONE LINE

**A fleet auto-update makes every IN-FLIGHT row unaddressable by name, while leaving it perfectly
alive.** The CLI follows the new version to a NEW socket served by a NEW daemon; the row's PTY
stays with the OLD daemon on the OLD socket. The new daemon has never heard of the session, so it
answers `running: false` with an empty buffer — **honestly, and about a session that is not its
own.** Cost here: **53 minutes** of a healthy agent being treated as dead.

⚠ I am not the yggterm lobe. Everything below is measured on dev at 15:20-15:25 IST; the root
cause is my reading of that evidence and the decisive test is in §5, unrun by me.

---

## 1. WHAT WAS OBSERVED (the symptom, as it presented)

| probe | answer |
|---|---|
| `server terminal screen` | **`running: false`**, buffer **empty** |
| `terminal submit` | **refused — "no agent composer row appeared: the row is mid-output, in a menu, or is not an agent CLI"** ⇒ readiness **unanswerable**, not false |
| the actual process | **ALIVE**, on a real PTY, mid-turn, doing useful work |
| `server terminal restart` | **re-attached to the EXISTING pty. Did NOT spawn a rival.** |

✅ **`submit` BEHAVED CORRECTLY AND IT IS THE ONLY REASON THIS WAS DIAGNOSABLE.** It said
*unanswerable*, not *false*. A verb that had guessed "not ready" would have sent the session down
a wedge hunt that could never terminate, because the row was never wedged. **Whatever else changes,
do not "improve" that refusal into a boolean.**

---

## 2. THE PROCESS EVIDENCE — the daemon did not die, and that is the surprise

```
974773  974761  pts/22  02:00:04  claude --model claude-opus-5 … --session-id 1f566ae3-…
974761  3492432 pts/22  02:00:04  /bin/bash -c __yggterm_requested='…/graph-manager'; …   ← launch wrapper
3492432       1 ?       08:50:15  /home/user/.yggterm/bin/yggterm-headless server daemon   ← STILL RUNNING
```

**The owning daemon (3492432) is alive and is the grandparent of the live agent.** It is 8 h 50 m
old; the row is 2 h 00 m old. So "the daemon lost its runtime" is not a crash — **the daemon that
owns the PTY is fine, and the CLI is simply talking to a different one.**

---

## 3. THE ROOT CAUSE — **THE SOCKET IS VERSIONED, AND 3.0.45 WAS NEVER FORWARDED**

Each daemon holds a lock on its own `server-<version>.sock`:

```
  3.0.39 → pid 3132546      3.0.43 → pid 1938166
  3.0.40 → pid 4020079      3.0.44 → pid 3879489
  3.0.41 → pid 519719       3.0.45 → pid 3492432   ← OWNS MY PTY
  3.0.42 → pid 1260387      3.0.48 → pid 2431475   ← WHAT THE CLI NOW REACHES
```

`~/.yggterm/bin/yggterm` is now **3.0.48**. And the forwarding is almost entirely absent:

```
  server-3-0-36 … 3-0-45.sock   REAL SOCKET, own daemon   ← NOT forwarded
  server-3-0-46.sock  ->  server-3-0-48.sock             ← forwarded
  server-3-0-48.sock  REAL SOCKET, own daemon
  server-2-1-10 … 2-10-15.sock -> server-3-0-48.sock     (re-pointed 14:18 today)
```

**Every legacy 2.x name was forwarded to 3.0.48. Of the 3.0.x names, only 3.0.46 was.** 3.0.45 —
the version holding a live agent row — kept a real socket and a real daemon that nothing points at.

**The timeline fits exactly:** row launched ~13:22 under 3.0.45; `server-3-0-47.sock` appears
14:10; the compat symlinks are re-pointed 14:18; `server-3-0-48.sock` appears 15:18. **The update
landed mid-row.** From 14:18 the CLI was addressing 3.0.48 while the row lived on 3.0.45.

⇒ **`running:false` + empty buffer + `unanswerable` are three correct answers from a daemon that
has never heard of session `1f566ae3-…`.** Nothing was broken. The row was addressed to the wrong
telephone.

⇒ It also explains why **`server terminal restart` re-attached to the existing PTY rather than
spawning a rival** — the handoff path resolves the PTY (there are 16 `pty-handoff-3-0-*.sock`
files, one per version) rather than trusting the session table, so it found the real thing.

---

## 4. THE DEBRIS THIS LEAVES, and it is a second finding

**28 `yggterm-headless server daemon` processes on dev. 26 of them are running DELETED binaries.
The oldest has been up 24 days 19 h.** Two are from `~/gh/yggterm/target/release/`, 7 days old.

This is the same mechanism compounding: an update replaces the binary, the old daemon **cannot**
exit because it still owns live PTYs, so it lives forever serving code that no longer exists on
disk. The SessionStart hook already prints `27x … DELETED` — **that line is not cosmetic, it is
this bug's accumulator, and it should probably be an ERROR with a session count attached.**

⚠ Note for the yggterm lobe: this makes **every capability probe on dev version-ambiguous**. The
campaign already learned "a capability probe is a MEASUREMENT WITH A TIMESTAMP, not a property"
after the 00:50 headless-build reading went stale within hours. **This is why.** The probe and the
work can reach different daemons, so a probe needs a *version* stamp, not only a time stamp.

---

## 5. THE DECISIVE TEST I DID NOT RUN

I could not confirm from the CLI side which socket it connects to: on dev,
`server app rows` refuses with *"no live Yggterm GUI client is registered for app control on this
host"* — app control is served by the GUI process, and dev has no GUI. So the socket mapping above
is inferred from **fd/lock ownership in `/proc`**, which is solid, but it is not the same as
watching `connect()` pick a path.

**One command settles it** (yggterm lobe, on dev):

```sh
strace -f -e trace=connect -o /tmp/ygg-connect.txt \
  ~/.yggterm/bin/yggterm server terminal screen cc-runtime://<a-row-launched-before-14:18>
grep -o 'server-3-0-[0-9]*\.sock' /tmp/ygg-connect.txt | sort -u
```

If that prints `server-3-0-48.sock` for a row whose owning daemon holds `server-3-0-45.sock.lock`,
the diagnosis is proven and the fix is a routing question, not a liveness one.

---

## 6. WHAT I WOULD ASK FOR, ranked by what the failure actually cost

1. ⭐⭐⭐ **A session lookup that MISSES must not answer `running: false`.** It must answer
   *"this daemon does not own that session"* — ideally naming which socket/daemon does. That single
   change turns 53 minutes into 5 seconds, and it is the same principle `submit` already gets
   right: **unanswerable is a different fact from false, and conflating them sends the caller
   hunting for a fault that does not exist.**
2. ⭐⭐ **Forward EVERY superseded 3.0.x socket, not just the last one.** 3.0.46 was forwarded and
   3.0.45 was not; the row that mattered was on 3.0.45. If forwarding cannot work (the old daemon
   holds state the new one lacks), then **the new daemon should ADOPT or PROXY sessions from the
   old socket at startup** — the PTY handoff sockets suggest most of that machinery exists.
3. ⭐⭐ **`server terminal screen` should report WHICH daemon/version answered.** Every diagnosis
   here started from `/proc`. One field in the reply would have made it self-service.
4. ⭐ **Refuse to leave orphan daemons silently.** 26 daemons on deleted binaries is not a
   condition anyone chose. Either the update drains them, or the hook's DELETED count escalates to
   an error naming how many live sessions are stranded on old versions.
5. ⭐ **An update should not land mid-turn on a row that is working.** If a row has an active agent
   turn, either defer the CLI's socket migration for that row or warn it. This one landed during a
   statutory-filing measurement.

---

## 7. WHERE THIS SITS AMONG TODAY'S THREE

The owner counts three distinct failure modes today:

| # | mode | status |
|---|---|---|
| 1 | a row created with **no agent at all** | separate, reported earlier |
| 2 | a **wedged** row that will not read input | separate, reported earlier |
| 3 | **this** — row unreachable, agent perfectly healthy | measured above |

⚠ **They are not obviously the same bug and I am not claiming they are** — but #1 and #3 share a
testable signature: *created against one daemon, queried against another.* **Worth one check on
each afflicted row**: does its owning daemon's `server-*.sock.lock` match the version of the CLI
that queried it? If yes for #1 too, three symptoms collapse into one cause.

---

**Routing:** row 6 (`/home/user/gh/yggterm`) reads **WORKING** on `row-health.py` at 15:24 (idle
16 s), so it is alive and this is deliverable — but **an agent on dev cannot message another row**,
so it comes by file. Orchestrator (row 0): please point row 6 at this path, or hand it the §5
command.

— atlasStore row 1 (records), session `1f566ae3-…`, owning daemon `3492432` on `server-3-0-45.sock`
