# Spec: the language-agnostic trace contract

**Status:** ACTIVE 2026-08-20 · **Owner:** `yggterm-core::trace_contract` +
`crates/yggterm-shell/src/shell/trace_emitter.js` · **Reads with:**
`docs/observability.md`

This file answers exactly one question: **what a layer that is not Rust must do
to become a first-class emitter on the trace plane, and what it may not claim.**

It deliberately does **not** answer:

| Question | Owner |
|---|---|
| Which probes exist, in what unit, and where the bytes land? | `docs/observability.md` |
| What is OPEN right now? | `docs/pending-bugs.md` |
| Which instruments lie in general? | `docs/agent-field-guide.md` |
| What is the cross-app probe bus and its query verbs? | the `ytrace` crate's own spec |

---

## 1. Why a contract, rather than "just call the trace function"

Two layers inside the GUI's webview do work the native crates cannot see: the
terminal canvas (xterm.js) and the reactive UI tree. Both were near-blind, and
the reason was never that the plane refused them — it was that the only ways in
were channels built for something else.

| Existing channel | Shape | Why it cannot carry a record |
|---|---|---|
| `debug` | a bare string | joining the plane through it means parsing a grammar back out of prose — a **second encoding** of the record, which the project's single-source-of-truth rule forbids outright |
| `perf` | structured, one event per hop | one IPC hop and one **synchronous file append on the UI thread** per event; the cost that froze the app under an output burst, and the reason a lossy throttle now sits in front of it |

⇒ The contract exists so a foreign layer can emit a record that is **the same
kind of thing** a Rust record is — rankable and correlatable against it without
a translation step — and so the emitting can be cheap enough not to need a
throttle that sheds the evidence.

---

## 2. The wire, seen from outside Rust

One JSON object per line, in the trace file. A foreign emitter supplies the
left column; the **receiver stamps the right column** and the emitter must not
send those fields at all.

| Emitter supplies | Receiver stamps |
|---|---|
| `ts_ms`, `layer`, `component`, `category`, `name` | `pid` |
| `kind`, `clock`, `duration_ms` (as applicable) | |
| `seq`, `dropped` | |
| `payload` | |

⛔ **`pid` is stamped, never carried.** A sandboxed emitter has no truthful
access to a process id, and an emitter that could set its own could set another
process's. The same reasoning covers anything else the sandbox cannot honestly
know.

```json
{"ts_ms":1723900000123,"pid":4242,"component":"ui","category":"xterm_write",
 "name":"flush","payload":{"host_id":"terminal-a","pending_chars":0},
 "layer":"xterm","kind":"span","clock":"wall","duration_ms":1.4,"seq":8817}
```

⭐ **Every contract field is additive and omitted when absent.** A line written
before this contract existed still parses under it, and the native path — by far
the highest-volume writer — spends no bytes stating defaults the reader already
assumes. The retention window on this plane is set by a **byte budget**, so
that is not tidiness; padding every native line would shorten the diagnostic
window in exchange for nothing.

### Defaults, stated once so no reader has to guess

| Field absent | Read it as | Why that is the right reading |
|---|---|---|
| `layer` | `rust` | every byte written before the contract came from the native crates |
| `kind` | `span` if `duration_ms` is present, else `point` | a record carrying a duration *is* a span whether or not it said so |
| `clock` | there is no duration to interpret | a `duration_ms` with no `clock` is refused, not defaulted (§4) |

---

## 3. `layer` — and why it is not `component`

`component` says which module inside the app an event belongs to (`ui`,
`daemon`, `session`). `layer` says **which runtime executed it**. They are
orthogonal, and it is their product that makes a vertical slice legible.

| `layer` | Runs in | Emitted from |
|---|---|---|
| `rust` | the native process | the yggterm crates (implicit — absent means this) |
| `dioxus` | the native process | the reactive UI tree; tagged by where the work belongs in the architecture, not by which compiler produced the instruction |
| `xterm` | the webview sandbox | the terminal canvas, across the bridge |
| `webkit` | the webview sandbox | **reserved, unused today** |

⛔ **Collapsing the two loses the thing the tag was added for.** A UI stall has
two halves: the component tree deciding to re-render (`component=ui,
layer=dioxus`) and the canvas that re-render is aimed at (`component=ui,
layer=xterm`). Filter on `component` alone and they are indistinguishable rows.

⭐ **Why `webkit` is reserved before it is used.** The co-browse surface will put
a second live viewer on one session, and when it does, "which viewer was slow"
needs a tag that already exists in the wire. Adding an enum variant later is
free; adding it to bytes already on disk is not, and every reader written today
already accepts it.

---

## 4. Clocks — a sandbox cannot claim what it cannot read

`docs/observability.md` §3.1 is the SSOT for what `wall` and `cpu` mean and why
a cpu-ms number is meaningless without its interval. This section adds the one
rule that only applies to foreign emitters.

There are **three** clocks in play once JS is emitting, and picking the wrong
one is silent:

| Clock | Property | Use it for |
|---|---|---|
| `Date.now()` | epoch, comparable **across processes**, coarse, jumps with the system clock | `ts_ms` — ordering a foreign record against a native one |
| `performance.now()` | monotonic, sub-millisecond, origin is **per document** | `duration_ms` — a delta between two reads in the same document |
| a thread CPU clock | **does not exist here** | nothing |

⛔⛔ **A record from a sandboxed layer claiming `clock: "cpu"` is refused at the
boundary.** There is no per-thread CPU clock in a webview content process:
`performance.now()` is monotonic wall time and `Date.now()` is epoch wall time,
and neither knows what fraction of the interval the thread was actually
scheduled for. Such a record is not slightly wrong — it is a number that a
reader will divide by an interval it does not have and publish as a core
fraction. The emitter has no cpu-span constructor, and the receiver refuses one
anyway, because two independent refusals is the right number for a fault whose
output is indistinguishable from a real measurement.

⛔ **A `duration_ms` with no `clock` is refused, not defaulted to `wall`.**
Defaulting is the friendly move and the wrong one: an emitter that forgot the
clock is an emitter whose units are unknown, and a guessed unit is
indistinguishable from a measured one once it is on disk.

### `ts_ms` is stamped at EMIT, never at arrival

The emitter buffers and flushes off the hot path (§6), so arrival trails
emission by however long the flush took to get a turn.

⇒ Stamping on arrival would shift every foreign row later by an amount that
**varies with how busy the UI thread was** — i.e. it would be most wrong exactly
during the stalls the plane exists to explain, producing a timeline in which the
probe fires after the fault it was measuring.

---

## 5. `kind` — point, span, window

`docs/observability.md` §4.3c records what happens without this tag: two probes
whose names read as two views of one thing were correlated on a shared
substring, one of them was a summary window, and the run compared point events
against bookkeeping ticks. It produced a confident "no correlation" that meant
nothing. The law it derived — establish point-vs-window before any temporal
analysis — was left to the reader, **because nothing in the record said so.**

This field is that law made machine-readable.

| `kind` | `ts_ms` means | Correlate on it? |
|---|---|---|
| `point` | the moment the thing happened | yes |
| `span` | the moment it **finished**; `duration_ms` says how far back it began | against the interval, not the instant |
| `window` | the moment a summary window **closed** | ⛔ **no** — the values are faithful, the timestamp is bookkeeping |

### ⛔ A window's `window_ms` is measured, never assumed

Both aggregators here close a window **lazily** — on the next event after the
interval elapses, rather than on a timer. That is deliberate: a timer would have
to wake on an idle terminal to report that nothing happened, and this runs on
laptops.

⚠ The consequence a consumer must know: **when activity stops, the last window
stays open until activity resumes**, so its real span can be far longer than the
nominal interval. Every window record therefore carries a measured `window_ms`.
A consumer that divides by the nominal constant computes a rate that is wrong by
exactly the length of the silence — and silence is common in exactly the traces
someone is reading after an incident.

---

## 6. The bridge — the batch is the point, not the schema

A foreign record reaches the plane as one entry in a **batch**, through a
dedicated channel. Both facts are load-bearing.

```
  probe call ─► ring buffer (bounded, in the sandbox)
                   │
                   └─ timer ─► one IPC message, N records
                                   │
                                   └─ validate ─► ONE lock, ONE write, N lines
```

⛔⛔ **An instrument that perturbs the thing it measures does not produce a noisy
reading; it produces a reading of itself.** The thread these probes run on is
the thread whose stalls they exist to explain. A per-record path pays a lock, a
rotation check and a write **per probe**, on that thread — which under an output
burst is hundreds back-to-back, and is the freeze that earned the standing
throttle in front of the older channel. One lock and one write per drain is what
makes an unthrottled emitter affordable.

### Rules the emitter must satisfy

1. **`emit` does no I/O.** It appends to a bounded ring and returns.
2. **The drain runs from a timer**, i.e. after the current task finishes — never
   inline in the path being timed.
3. **The timer is self-suspending.** An idle terminal schedules no wakeups.
4. **A burst is brought forward.** Past a high-water depth the drain moves to
   the next task instead of waiting out the interval, so a burst reaches the
   plane while it is still a burst rather than averaged into the interval after
   it.
5. **The ring drops the OLDEST.** Under sustained pressure the newest records
   describe the state the app is in *now*, which is the question being asked;
   the oldest describe how it got there, which earlier drains already carried.
6. **A drop is counted, and the count rides on a record.** ⛔ Not reported
   beside them — under the sustained pressure that causes drops, a separately
   reported count is itself subject to that pressure, so the one number proving
   the stream is incomplete is the one most likely to go missing.

### Rules the receiver enforces

A refusal is **counted, never silent**: a layer that has started emitting
garbage and a layer that has gone quiet look identical from the reader's side,
and only one of them is a bug in the emitter. Faults are summarised once per
drain as `ui/trace_bridge/foreign_batch_faults`.

| Fault | Meaning |
|---|---|
| `unknown_layer` | not a known tag; refused rather than mapped to a default, since a typo silently becoming `rust` biases every aggregate over the native population |
| `layer_not_foreign` | a native tag arrived over the foreign bridge — a bug, or an attempt to launder foreign rows into the native population |
| `cpu_clock_from_sandbox` | §4 |
| `unusable_duration` | no clock to interpret it, or negative, or not finite |
| `unknown_kind` / `empty_probe_name` | malformed |
| `payload_too_large` | **repaired, not refused** — the payload is replaced by a marker carrying its size |

⭐ **Oversized payloads are repaired rather than dropped**, on the principle that
a diagnostic stream loses more to silent absences than to degraded rows. A
record saying "my payload was too big" is diagnostic; a missing record is an
absence, and a *structural* absence says nothing at all about what happened.

### `flush_lag_ms` is a signal, not bookkeeping

The receiver measures how far behind the newest record in a batch is running.
Because the emitter drains off the hot path, **a lag that grows is the UI thread
failing to reach idle** — which is the stall itself, reported by the one
mechanism that keeps working while it happens. Without it, a blocked emitter
reports its own silence as "nothing happened".

---

## 7. Adding a probe to a foreign layer

```js
// a point event
ytrace.emit({
  category: "xterm_write", name: "enqueue_backlog",
  payload: { host_id: "terminal-a", depth: 20480 },
});

// a span — wall clock, always
const span = ytrace.span("xterm_write", "flush", { host_id: "terminal-a" });
// ... the work ...
span.finish({ chars: 4096 });

// a closed summary window
ytrace.window("xterm_render", "frame_window", { frames: 42, window_ms: 1013 });
```

### ⛔ Ration the resolution

The ring is bounded, so **a probe that fires per steady-state event spends the
whole budget describing the boring case** and evicts the incident that was the
point. The discipline is the one the native probes already use: an always-on
aggregate keeps the RATE honest, and point events are spent only on outliers and
on boundaries where corruption lives.

⇒ Before adding a probe, ask which of the three it is: an aggregate, an outlier,
or a boundary. A probe that is none of them is a probe that will be dropped by
the ones that are.

### Ordering: use `seq`, not `ts_ms`

`ts_ms` has millisecond resolution, and the questions worth asking of a
corrupted repaint are about what interleaved **inside** one millisecond — a
screen reset, the reseed that follows it, and a bridge flush routinely share
one. One emitter numbers every record it produces, so `seq` totally orders them.

⇒ A screen wipe and its reseed are both marked. Any record whose `seq` falls
**between** the two wrote into a screen that was mid-replacement, and a wipe
with no reseed after it is a screen that was emptied and never refilled.

---

## 8. Verification

* `cargo test -p yggterm-core trace` — the grammar, the refusals, the batch, and
  that a pre-contract line still parses.
* `node --test trace_emitter.test.js` in `tools/xterm-harness/` — the emitter's
  own rules (§6), driven under a fake clock against **the same file the GUI
  ships**, so what is asserted is what runs. This one test needs no `npm
  install`.
* `cargo test -p yggterm-shell render_attribution` — the aggregation guard that
  keeps the cost of measuring independent of the render rate.
