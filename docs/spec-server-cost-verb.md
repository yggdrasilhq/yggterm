# Spec: `server cost` — the verb that answers "what is this host spending"

**Status: SPEC. 6.9 writes it, 6.7 builds it.** The measurements that justify it
and the field offsets it needs are in [`idle-cost-model.md`](idle-cost-model.md).

## Why this is a verb and not a recipe

There is no instrument that answers *"what is yggterm costing on this host right
now"*. The nearest thing lies:

- `server status → terminal_session_count` is **per-daemon**, and it fires its
  loudest false alarm during a handover — a real watch alerted `53 → 29` while
  the host had *gained* sessions.
- `ps %CPU` is a **lifetime average**, not current load, and has misled this
  campaign more than once.
- `/proc/<pid>/stat` is correct but hand-indexed, and **the hand slips**: the
  first sampler written for the §1 measurement read fields 9 and 10 as
  `utime`/`stime`. Those are `flags` and `minflt`. The controls caught it; a
  reader without controls would have published the numbers.

⇒ **The test for a verb is met exactly**: an agent hand-assembles this chore
from primitives each session and gets it wrong. An agent's discipline resets
every session; a verb's does not.

## ⭐ The load-bearing requirement: the answer must be VOID, not wrong

**Every invocation runs both controls inline, in the same window as the
measurement**, and reports them:

- a **spinner** that must read ≥0.95 cores
- a **sleeper** that must read ≤0.02 cores

If either control falls outside its bracket, `server cost` **refuses to report
numbers** and exits non-zero saying which control failed.

This is the property the verb exists for, and it is the one a builder will be
tempted to drop as overhead. Do not drop it. A probe that reports "absent" about
everything reports nothing about anything, and a positive control alone cannot
detect an instrument that has collapsed to a constant answer — a failure this
campaign has produced three times, each time confidently. **Two controls, same
run, or no answer.**

Cost of the controls: one spinning thread for the window's duration. On a host
with ≥4 cores that is under 1/4 core for a 5 s default. That is the price of the
number being trustworthy.

## Contract

    yggterm-headless server cost [--window <secs>] [--json] [--by-daemon]

**Default window 5 s.** ⛔ **The window must be printed with every number.** A
2 s sample of the same machine read 49%+20% where a 40 s sample read 25%+14%;
short windows catch spikes, and a figure without its window is not comparable to
anything.

### Output

    server cost — host-wide, window 5.0s, clocksource=tsc
    controls: spinner 1.002 (ok)  sleeper 0.000 (ok)   -> VALID

    role                  n    cores    user  kernel     RSS    swap
    daemon               19    3.468   0.889   2.580   5.48G   0.00G
    web_process           9    0.387   0.334   0.053   2.68G   0.00G
    ychrome              11    0.238   0.117   0.122   0.64G   0.00G
    cli_client            11   0.039   0.015   0.024   0.60G   0.00G
    gui                   1    0.010   0.009   0.002   0.15G   0.00G
    ...
    TOTAL               108    4.167   1.380   2.787  12.48G   0.00G

    writers         KB/s   GB/day
    event-trace     95.3     8.43
    perf-telemetry  45.9     4.06

### Required behaviours

1. **Host-wide, and every term says which daemon it came from** (`--by-daemon`).
   This is the direct answer to the `terminal_session_count` trap: a per-daemon
   number presented as a host number is how a handover reads as a catastrophe.
2. ⛔ **Classify by the SUBCOMMAND. `comm` is not enough, and neither is the
   command line.** Keying on the command line once classified a dozen near-idle
   wrappers as `gui` and diluted that role's mean by 2x — and it did it again
   during the measurement that produced this spec, where all 12 processes called
   `gui` were `yggterm server remote start-cc|resume-cc` wrappers whose command
   line contains the binary path. **The known remedy — use `comm` — would ALSO
   have failed**, because `comm` is `yggterm` for the GUI *and* for an
   ssh-carried CLI client. The discriminating field is the subcommand:
   `server daemon` ⇒ daemon, `server remote …` ⇒ CLI client, no subcommand ⇒
   the GUI. Roles that share a binary can only be told apart by what they were
   asked to do.
3. **Print each role's sample count `n` beside its mean.** A single stale row
   otherwise renders as a full role.
4. **Report the clocksource.** On an `hpet` host `clock_gettime` costs 45.8x
   what it costs on `tsc`, so the kernel-time column means something different.
   A number without its clocksource does not travel between hosts.
5. **Report swap alongside RSS.** RSS alone undercounted a footprint by a third
   on this project already; the web memory bound polls RSS and is therefore
   unreachable once a process is swapped.
6. **Include the trace writers' byte rates.** They are 12.49 GB/day today and no
   surface reports them.
7. **Read-only. No ptrace, no signals, no `strace`.** Everything above is
   available from `/proc` reads: `stat`, `status`, `io`, `task/`. The verb must
   be safe to run against a host carrying other agents' live sessions, because
   that is the only host worth running it on.

### The `/proc/<pid>/stat` offsets, so nobody re-derives them

After stripping the leading `pid (comm) ` — `comm` may contain spaces and
parentheses, so **split on the LAST `") "`, never the first** — the remaining
fields are 1-indexed from `state` = field 3:

| value | field | index into the stripped remainder (0-based) |
|---|---|---|
| `utime` | 14 | **11** |
| `stime` | 15 | **12** |
| `num_threads` | 20 | 17 |
| `starttime` | 22 | 19 |

⚠ **`/proc/<pid>/stat` counts threads that have already exited; summing
`/proc/<pid>/task/*/stat` does not.** On these daemons the two disagree by ~5x
because a thread is spawned per connection (`idle-cost-model.md` §3). ⇒ **The
process-level read is the correct one for a cost figure**; a per-thread
breakdown must state that it is partial.

## What would show this verb is not worth having

If `server cost` and a careful hand-rolled `/proc` sampler agree to within 5% on
three different hosts *and* an agent asked to measure idle cost reaches for the
verb rather than rebuilding the sampler, it is working. **If agents keep
hand-rolling it anyway, the verb's output is missing something they need** — find
out what, rather than assuming the habit is the problem.
