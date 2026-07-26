# PTY fd-handoff spike — `SCM_RIGHTS` (level (b) groundwork)

The constitution's durable half: a plain shell pins its daemon forever because
a PTY cannot move between daemons. `docs/pending-bugs.md` ("LEVEL (b) —
LOSSLESS") maps where this would slot in and names the actual design question —
**not** the `sendmsg` call, but who owns the child after the fd moves.

**All three risky primitives work.** Verified 2026-07-27 on Debian sid,
glibc 2.42, x86_64.

## Result

```
$ ./target/release/pty-fd-handoff-spike
[send] pid=3285983 spawned bash -i as pid 3285984 on master fd 3
[send] master fd sent; exiting so the shell re-parents
[spike] 1. SCM_RIGHTS: received master fd 5, payload "shell_pid=3285984"
[spike] 2. sender exited: exit status: 0
[spike] 3. shell pid 3285984: alive=true PPid=1 (was 3285983) — tracked via /proc, NOT waitpid
[spike] 4. WROTE to the received fd and READ the shell's answer (MARKAFTER-HANDOFF) — with the sender already gone
[spike] 5. RESIZED via the received fd: 0x0 -> 100x40
[spike] 6. the SHELL sees the new size (stty size -> 40 100)
[spike] 7. the PTY still works with no owning process anywhere
[spike] ACCEPTANCE=PASS
```

Step 6 matters more than step 5: `TIOCSWINSZ` succeeding only proves a kernel
struct changed. Asking the shell (`stty size` → `40 100`) proves the resize
reached the process, i.e. `SIGWINCH` was delivered over a PTY whose original
owner no longer exists.

**The acceptance can fail.** Negative control: attach the payload but NOT the
ancillary data (`msg_control: null, msg_controllen: 0`) and the run reports
`FAIL at recvmsg: no ancillary data arrived (msg_controllen=0) — the fd did NOT
travel`, exit 1. A check that can only pass is worth nothing.

## What worked verbatim

- **`SCM_RIGHTS` over `UnixStream`** with hand-written `msghdr`/`cmsghdr`.
  `#[repr(C)]` reproduces glibc's padding correctly (`msg_namelen` is a `u32`
  followed by 4 bytes before the next pointer) — that layout is the classic
  place this goes wrong. `CMSG_SPACE(sizeof(int))` = 24, `CMSG_LEN` = 20.
- **The payload rides the same `sendmsg` as the fd**, carrying `shell_pid=`.
  This is load-bearing, not convenience: the receiver cannot learn the pid from
  the fd, and a second channel could arrive out of order with it.
- **Driving the received RAW fd**: `write`, non-blocking `read`, `TIOCSWINSZ`
  and `TIOCGWINSZ` all behave normally. Nothing about the fd remembers that it
  was created by another process.
- **The ownership decision, settled with evidence.** The sender exits
  immediately after the transfer. The shell's `PPid` becomes **1**, it stays
  alive, and commands typed afterwards still run and still answer. The receiver
  tracks liveness with `/proc/<pid>` + `kill(pid, 0)`.

## The ownership decision, stated plainly

**The predecessor must NOT stay alive as a reaper** — that defeats the entire
point of the handoff. The spike takes the other branch and it holds: the child
re-parents to init and the successor uses `/proc` liveness.

What is lost, and must be designed around rather than discovered later:

- **No exit status, ever.** `waitpid` is gone, so the successor learns "the
  shell exited" but never *how*. Anything that reports an exit code for a
  handed-off session is reporting a guess.
- **Nobody reaps it.** The shell becomes init's child, so it is reaped by init
  — fine — but the successor must kill it explicitly on session close, since
  dropping the fd only sends `SIGHUP` to the foreground group.
- **PID reuse is a real hazard.** `/proc/<pid>` liveness is only sound if the
  identity is confirmed, not just the number. Carry the shell's
  **start time** (`/proc/<pid>/stat` field 22) beside the pid and compare both;
  a bare pid check will eventually attach to a stranger.

## Scrollback carriage — design sketch only (NOT implemented)

The fd alone hands over a live terminal with an empty transcript. The map is
right that the ring has to travel beside it. Sketch:

- **Carrier**: the existing `terminal_snapshot` payload, sent as the *ordinary
  JSON line* on the same handoff connection immediately BEFORE the `sendmsg`
  that carries the fd. Ordering matters in that direction: the successor must
  be holding the transcript before it owns the fd, or output arriving between
  the two steps lands in a session with nowhere to put it.
- **What travels**: `chunks` (the ring), `seq`, `retained_bytes`, `spawn_id`.
  `spawn_id` is not optional — the client's cold-re-resume vacuum guard keys on
  "did this frame come from a different runtime spawn", and a handed-off
  session must present as the SAME spawn or every reveal will look like a
  runtime replacement.
- **The seam**: between the last chunk the predecessor read and the first the
  successor reads. The predecessor must stop its reader thread BEFORE
  serialising, send `seq` with the payload, and the successor resumes reading
  at that `seq`. Anything the kernel buffered in the PTY in between is still in
  the fd's buffer and arrives to the successor normally — which is exactly why
  the reader must stop first rather than race.
- **Failure mode to design for**: a partial handoff (transcript sent, fd send
  fails) must leave the PREDECESSOR owning the session. Send the fd last and
  treat `sendmsg` success as the commit point.

## Sizing the real integration

**Where it slots in** is already settled by the map:
`ServerRequest::HotRestart`'s preserving-handoff branch in `daemon.rs`, right
where it calls `PreservedTerminalOwnerRegistry::write_handoff`. That registry
(runtime key → owner socket + pid) is already precisely the list of fds to
send, and the send side needs no new plumbing into the pty layer —
`master.as_raw_fd()` is already reachable and is what
`foreground_process_group_leader` uses. The JSON-line wire has no room for
ancillary data, so this is an out-of-band `sendmsg` on the handoff connection,
not a new `ServerRequest` field.

**Does `portable_pty` accept a foreign fd? No — confirmed, not assumed.** In
portable-pty 0.9.0, `UnixMasterPty` and `PtyFd` are **private** (`struct`, not
`pub struct`) in `portable_pty::unix`, and the only construction path is
`openpty()`, which always creates a NEW pair. There is no `from_raw_fd` for a
master. So **the receive side needs its own local master type**, as the map
predicted.

The good news is that the type is small, because the trait is small.
`MasterPty` is eight methods, and this spike already implements the substance
of the hard ones against a raw fd in about forty lines:

| method | cost from a received fd |
| --- | --- |
| `resize` / `get_size` | `TIOCSWINSZ` / `TIOCGWINSZ` — done here |
| `try_clone_reader` / `take_writer` | `dup` the fd into a `File`; the once-only writer rule is a `RefCell<bool>` |
| `as_raw_fd` | trivial |
| `process_group_leader` | `tcgetpgrp` — the existing call site already does this |
| `tty_name` | `ptsname_r`, or `None` (the trait allows it) |
| `get_termios` | has a default impl |

So budget the receive side at **a few hundred lines for a
`ReceivedMasterPty`**, not a fork of the pty layer. The genuinely awkward part
is not `MasterPty` at all — it is `PtySessionRuntime.child:
Arc<Mutex<Box<dyn Child + Send + Sync>>>`, which has no honest implementation
after a handoff. `Child::wait`/`try_wait` cannot answer for a process that is
not ours, so that field needs to become an enum (`Owned(Box<dyn Child>)` vs
`Adopted { pid, start_time }`) and every caller taught which it is holding.
**That refactor, not the socket call, is the real cost of level (b).**

## Running it

```sh
cd docs/spikes/pty-fd-handoff
cargo build --release
./target/release/pty-fd-handoff-spike
```

Exit 0 = PASS. Needs a working `bash` on PATH. Linux-only by construction
(`SCM_RIGHTS` layout, `/proc` liveness, `forkpty` from libutil).
