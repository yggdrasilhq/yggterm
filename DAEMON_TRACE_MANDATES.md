# Daemon Trace Instrumentation — The Four Mandates

**Owner directive:** Wire good ytrace systems on daemon when daemon disconnects/reconnects. Aim at "rock bottom to GUI, one plane, one grammar."

**Root cause addressed:** commit 633e22d8 fixed a deadlock where the snapshot handler fanned out to peer daemons with blocking requests under the runtime lock, causing `daemon_request/snapshot` p50/p95 to pin at 10,060–10,239 ms (the 10s timeout), resulting in ~21 minutes of daemon deafness in 2 hours.

**The mandate:** Make defects like this **detectable by traces**, not by code reading.

---

## M1 — NAME THE LOCK HOLDER ✅ IMPLEMENTED

When a request waits on the runtime lock, the trace must record **what was holding it and for how long**.

**What's done:**
- Added `lock_holder_trace` module with thread-local state tracking lock holder context
- When the lock is acquired, `enter_lock_holder(request_name)` records what request holds it
- When the lock is released, `exit_lock_holder()` emits a span with:
  - `request`: which request held the lock
  - `held_ms`: how long
  - `remote_calls`: count of remote calls made inside the lock
  - `remote_call_total_ms`: total time spent in remote calls
- Integrated into `lock_daemon_runtime_for_request()` and dispatch code

**Trace events produced:**
- `daemon/request/lock_holder`: Emitted when lock is released (only if held > 10ms)
- `daemon/request/lock_wait_identified`: Emitted when a request waits on the lock, now includes who was holding it

---

## M2 — DETECT BLOCKING CALLS INSIDE LOCK ✅ IMPLEMENTED (first instance)

Wire every expensive remote call to check if it's running inside the lock, and alert if so.

**What's done:**
- Added `record_blocking_call_inside_lock()` to track and warn about blocking calls
- Integrated into `working_flags_including_proxied()` fan-out loop:
  - Records every peer call that takes > 50ms
  - Emits `daemon/perf/blocking_call_inside_lock` trace with:
    - `lock_held_by`: which request is holding the lock
    - `lock_held_for_ms`: how long it's been held
    - `call_name`: what remote operation is running
    - `call_cost_ms`: how much time it took
    - `peer`: which peer was called
    - `total_remote_calls_in_span`: running count in this lock holder
    - `total_remote_time_in_span_ms`: running total cost

**Known instances to wire (not yet done):**
1. ✅ `working_flags` proxy calls to peer daemons (snapshot fan-out) — DONE
2. ⚠️ Remote cwd resolve at session create — TODO
3. ⚠️ TerminalRead hoist — TODO

---

## M3 — INSTRUMENT DISCONNECT/RECONNECT ⚠️ TODO

The GUI shows "Recovering Local Terminal" when it loses daemon connection. Need daemon-side lifecycle events so a trace shows what happened.

**Required traces:**
- `daemon/lifecycle/client_disconnected`: Emitted when a client connection closes
  - `remote_addr` or identifier
  - `active_sessions`: count of sessions this client was managing
  - `reason`: normal close, error, timeout, etc.
- `daemon/lifecycle/recovery_initiated`: When daemon detects a disconnected client is back
  - `client_id`
  - `sessions_to_restore`: how many
- `daemon/lifecycle/recovery_complete`: After session recovery
  - `sessions_restored`: count
  - `recovery_time_ms`: wall time taken

**Implementation approach:**
- Hook into client connection lifecycle in daemon.rs
- Record in the connection handler and/or the main accept loop
- Correlate disconnect/reconnect by client identity

---

## M4 — MEET ytop HALFWAY ⚠️ IN PROGRESS

Daemon traces must be renderable beside ytop's kernel-level traces: consistent layer tags, clocks/units declared in record.

**ytrace grammar (from `~/gh/ytrace/crates/ytrace/src/lib.rs`):**
- `component`: "daemon" ✅
- `category`: "request", "perf", "lifecycle" ✅
- `name`: event name ✅
- `clock`: "wall" or "cpu" (declared in tagged traces)
- `duration_ms`: for spans
- `payload`: JSON with fields

**What's done:**
- Using standard trace API (`append_trace_event`)
- Traces flow through ytrace provider for dual-write

**To be added:**
- Use `append_tagged_trace_event()` for spans with explicit clock declarations
  - Lock holder spans should declare `clock: Wall` and `duration_ms`
  - Remote call traces should declare `clock: Wall` and `duration_ms`
- Add `process_age_ms` to rates (per mandate clause 6: "Process age is part of the window")
- Validate trace format against ytop consumer expectations (once ytop side is wired)

---

## How to read the traces

**When a request waits on the lock:**
```json
{"component": "daemon", "category": "request", "name": "lock_wait_identified", 
 "payload": {
   "waiting_request": "Status",
   "wait_duration_us": 10150000,
   "lock_held_by": "Snapshot",
   "lock_held_for_ms": 10240,
   "holder_remote_calls": 4
 }}
```

**When a request releases the lock:**
```json
{"component": "daemon", "category": "request", "name": "lock_holder",
 "payload": {
   "request": "Snapshot",
   "held_ms": 10240,
   "remote_calls": 4,
   "remote_call_total_ms": 9850
 }}
```

**When a blocking call is detected inside the lock:**
```json
{"component": "daemon", "category": "perf", "name": "blocking_call_inside_lock",
 "payload": {
   "lock_held_by": "Snapshot",
   "lock_held_for_ms": 10000,
   "call_name": "working_flags",
   "call_cost_ms": 1050,
   "peer": "daemon-on-machine-b",
   "total_remote_calls_in_span": 4,
   "total_remote_time_in_span_ms": 9850
 }}
```

**The key question answered:**
If the snapshot deadlock existed today and nobody read the source, a trace reading would show:
1. `lock_wait_identified`: Status request waited 10.15s, held by Snapshot
2. `lock_holder`: Snapshot held the lock 10.24s, made 4 remote calls
3. `blocking_call_inside_lock`: All 4 calls were inside the lock (should never happen)
4. `proxied_working_flags_slow_peer`: Each peer call took 1–2.5s (normal is < 50ms)

Together, these traces paint the full picture: the lock was held too long because a snapshot handler fanned out to slow peers inside the lock.

---

## Next steps (for 11.0 or 11.19)

1. **M2 completion:** Wire the other two blocking call instances (remote cwd resolve, TerminalRead hoist)
2. **M3:** Instrument client disconnect/reconnect lifecycle
3. **M4:** Add ytrace tagged spans with explicit clock declarations
4. **Testing:** Deliberately trigger the snapshot deadlock (if possible in test harness) and verify traces name it without code reading
5. **Feed ytop:** Once seat 11.16 wires kernel traces, correlate daemon and kernel events to verify the full stack

---

## Code locations

- Lock holder tracing: `crates/yggterm-server/src/lock_holder_trace.rs` (new)
- Lock acquisition: `crates/yggterm-server/src/daemon.rs:20019` (enter)
- Lock release: `crates/yggterm-server/src/daemon.rs:20577` (exit)
- Blocking call detection: `crates/yggterm-server/src/daemon.rs:4722` (working_flags proxy)
