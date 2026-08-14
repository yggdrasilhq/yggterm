# The usability contract — what "yggterm is usable" means, in checkable terms

> **The one question this file answers: _is the app usable right now, and if not,
> what is the WORST thing wrong?_** Not "what is open" (that is
> [`pending-bugs.md`](pending-bugs.md)), not "which instruments lie" (that is
> [`agent-field-guide.md`](agent-field-guide.md)). This file defines the word
> *usable* and orders its levels; `scripts/usability-check.sh` is its executable
> form and the two are meant to be read together.

## Why a definition was needed

The report that produced this file was *the fan ran all day, the app is janky,
and the sidebars have not been visible for twelve hours across several restarts*.
Every instrument the project had said the app was fine, because every instrument
answered a narrower question than "is it usable". A definition that cannot fail
is not a definition, so each level below names **what PASS means** and **what
observation would falsify it**.

## ⛔ The two rules that make the ordering worth having

1. **Stop at the first failure and report that one thing.** A ranked list of six
   is how the load-bearing item gets buried. Each level is worthless if the one
   above it fails: a faithful viewport inside a window the user cannot reach is
   not usability, it is a screenshot.
2. **Never test this against a human's live session.** Levels 1-3 and 6 are pure
   reads. Levels 4-5 mutate and therefore need a probe row created with
   `--purpose` and `--ephemeral`, removed with `server app session remove`. This
   is why the hourly tick does not run them by default.

## The levels, load-bearing first

| # | Level | PASS means | Falsified by |
|---|---|---|---|
| **1** | **The window is the product** | Exactly ONE GUI process; its `/proc/<pid>/exe` is not `(deleted)`; its build matches the installed binary | two GUIs, a deleted exe, or an md5 mismatch |
| **1b** | **It has not crashed** | No yggterm coredump in the window since the last check | `coredumpctl list` naming yggterm |
| **2** | **Both sidebars render** | The cwd/session tree AND the session-metadata panel are painted | reading the screenshot and not seeing them |
| **3** | **The viewport is faithful** | No dropped glyphs, squish, broken bottom, or stuck restore toast | reading the screenshot |
| **4** | **Input lands** | A keystroke reaches a session and echoes | probe row does not echo |
| **5** | **Click opens** | Clicking a row opens it in a few seconds, not tens | probe row timing |
| **6** | **Cost at rest** | GUI subtree CPU is small when nothing is happening; no GUI older than the current build | a sustained rate well above the band below |

### Level 1 is first because a restart makes it WORSE, not better

`server app launch` spawns a GUI unconditionally — there is no existing-instance
check and no retirement of the old one
(`run_app_launch_via_gui_companion`, `apps/yggterm/src/bin/yggterm-headless.rs`).
So the user's own remedy is the thing that compounds the fault: every restart
adds an instance while the previous one keeps painting the window they are
looking at. **A user who restarts to escape a broken window cannot escape it by
restarting.** That is why this level outranks every rendering check.

Measured cost of one such orphan, from the resource recorder's own per-pid
history: **3.63 core-hours over 12.4 hours, at a rate of 29.2% of a core.**

⚠ **And the rate is NORMAL.** Healthy GUIs in the same 24 hours ran 20-38% of a
core. The orphan was not a runaway; it was an ordinary GUI that should have been
retired in seconds and instead lived half a day. **The waste is duration times a
normal rate, so the fix is retirement, not optimisation** — an important
distinction, because a profiler pointed at that process would have found nothing
wrong with it.

### Level 1b exists because a crash is invisible to every other level

A crash is followed by a relaunch that passes levels 1-3. Nothing in the app
reports having died. The instrument is the system coredump log, not anything
yggterm owns, and until this contract nothing consulted it.

### Level 6 must be reported in CORES, never a share, and as a RATE

`ps %CPU` is a **lifetime average**: on a process that idled for twelve hours it
reports the average of that whole life, not what it is doing now. Use a delta of
`/proc/<pid>/stat` fields 14+15 over a window, divided by the window, and measure
the **subtree** — the GUI's WebKit child is a large share of its cost and is
invisible if you measure the process alone.

⚠ **One spot sample is not the operating point.** The GUI's idle CPU swings
11.5% to 57.9% of a core on one build with nothing changed, and lengthening the
window from 5 to 30 minutes barely moves the spread. The threshold in the script
is therefore deliberately loose: it catches a runaway, it does not measure the
cost. **Do not report a single sample as a cost figure** — the 24-hour recorded
history is the instrument for that question.

## What the first run of this contract found

Recorded over 24 hours by the resource recorder, on the desktop host:

| what | core-hours / 24h | share of userland CPU above 1% |
|---|---|---|
| GUI | 5.75 | |
| its web-content child | 3.68 | |
| daemons (all) | 1.85 | |
| **yggterm total** | **11.28** ≈ **0.47 cores sustained** | **~85%** |
| everything else measured | ~1.0 | ~15% |

⇒ **yggterm is the dominant userland CPU consumer on that machine, not by being
large but by being constant.** 63% of the GUI figure was the single orphaned
process described above.

⚠ **What this does NOT establish is the fan.** The machine idles near 87 °C at
about 1.1 of 16 cores with the CPU in its deepest idle state 76.6% of the time,
which is a thermal picture that 0.47 cores does not by itself explain. Recording
the correlation is honest; naming yggterm as the cause of the fan would not be.
That question is open and is not answered here.

## Instruments this contract depends on, and how each of them lies

Every one of these was checked before being trusted, and three failed:

- ⛔ **The recorder's `temp_alarm` is the RAM sensor**, not the CPU — see
  [`pending-bugs.md`](pending-bugs.md). Confirmed live on the desktop host: the
  DIMM sensor sits one degree under a 55 °C nominal and latches. **Anyone quoting
  an alarm count is quoting the RAM.**
- ⛔ **`server trace tail --limit N` ignores N** and always returns 200 events —
  about 14 seconds on a busy host. Requesting 50 returns 200, which is how the
  ignore is distinguished from a cap.
- ⛔ **The presentation-policy trace event is emitted by every CLI invocation**,
  with `display_present:false`. The law names this event as the authority on what
  the GUI armed; on a busy host the GUI's real event is evicted within seconds by
  CLI copies of itself. Read it from the on-disk `event-trace.*.jsonl` and select
  on the GUI's pid, never from `trace tail`.
- ✅ `capture_faithful` was checked and is honest. A `false` frame is a lie about
  the terminal and must be treated as a failure, not as a missing measurement.
- ✅ The recorder's `cpu_user_pct` is documented in its own schema as a rate over
  the interval rather than a lifetime average, and behaved as documented.

## Running it

```sh
scripts/usability-check.sh          # levels 1, 1b, 2, 3, 6 - safe, non-invasive
scripts/usability-check.sh --json   # one JSON object, for a relay or watcher
scripts/usability-check.sh --deep   # adds 4 and 5 - creates an ephemeral probe row
```

Exit code is the number of the first failing level, 0 if all pass. **Levels 2 and
3 require a human or an agent to READ the screenshot it saves** — they are an eye
check by construction, and a script that claimed to pass them from a field value
would be reintroducing the exact mistake this contract exists to prevent.
