# Terminal Telemetry

Terminal telemetry is the durable incident log for the daemon-owned PTY and xterm.js surface. It exists because transient screenshots, JSONL trace tails, and one-off smoke probes were not enough to prevent repeated regressions where a session was stored but not live, a runtime was live but the xterm surface was blank, or a terminal became usable only after a manual switching pass.

## Contract

- Telemetry is enabled by default for desktop GUI runs.
- The user can disable it from the Settings sidebar under Terminal Telemetry.
- Telemetry is stored on the GUI-running machine under:

```text
~/.yggterm/telemetry/terminal.sqlite3
```

- The database records terminal lifecycle and fault events, not raw transcripts.
- The PTY and xterm.js remain the terminal source of truth. Telemetry observes readiness, input, render, reconnect, and recovery decisions; it must not become a renderer, alternate input path, or session state authority.
- The database must be safe to leave enabled during normal work. Writes are low-frequency and should happen off the UI path.

<<<<<<< HEAD
The observer boundary is defined in
`docs/architecture-audit-2026-05-16.md`. Telemetry may prove that the GUI made a
bad decision, that app-control disagrees with daemon truth, or that a switching
pass healed a surface. It must not make that decision itself. Recovery code may
read current daemon/xterm state; telemetry is historical evidence for diagnosis,
release gates, and regression grouping.

The GUI JSONL/trace writer and duplicate-throttle policy live in
`crates/yggterm-shell/src/ui_telemetry.rs`. Shell interaction code may decide
which event happened, but it must not carry a second copy of the file names,
rotation limits, timestamp shape, or duplicate suppression rules.

=======
>>>>>>> c162185 (Snapshot alpha blur experiment)
## Schema

The primary table is `terminal_events`.

```sql
CREATE TABLE terminal_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms INTEGER NOT NULL,
    pid INTEGER NOT NULL,
    source TEXT NOT NULL,
    category TEXT NOT NULL,
    name TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'info',
    session_path TEXT,
    runtime_key TEXT,
    host_id TEXT,
    gui_pid INTEGER,
    daemon_pid INTEGER,
    server_version TEXT,
    reason TEXT,
    payload_json TEXT NOT NULL
);
```

Indexes cover timestamp, session path, event category/name, and severity.

## Event Shape

Every event should be queryable by the same identifiers used in app-control:

- `session_path`: user-facing saved/live session path, such as `remote-session://dev/<uuid>`.
- `runtime_key`: daemon terminal runtime key when it differs from the user-facing path.
- `host_id`: xterm host DOM identity.
- `gui_pid` and `daemon_pid`: process boundary involved in the event.
- `server_version`: running Yggterm version that emitted the event.
- `reason`: short, stable reason string for incident grouping.
- `payload_json`: bounded structured details for the specific event.

Reason strings are intentionally stable. For example:

- `active terminal host exists but xterm surface is empty`
- `blank_runtime_output`
- `active remote terminal lost expected scrollback after retained replay`
- `active remote terminal is showing stale retained text before prompt-ready surface`

## Initial Coverage

The GUI records `terminal_open_attempt` transitions:

- `begin`
- `ready`
- `recovering`
- `latched_failure`
- `request_failed`
- `session_failed`
- `cleared`

Severity mapping:

- `info`: normal lifecycle transitions.
- `warn`: recovery paths that should usually self-heal, such as empty xterm surface recovery.
- `error`: latched failures and failed terminal requests.

<<<<<<< HEAD
The mounted xterm host also records `terminal_contract/terminal_render_health_unhealthy`
when xterm.js reports buffered terminal text but the visible render health is
unhealthy, for example `canvas_blank_with_buffer_text` or
`canvas_low_contrast_foreground_with_buffer_text`. These events are throttled by
the host-health problem key so a single bad surface is logged once, while a
later heal and relapse is still visible as a fresh incident.

The retained-recovery gate records
`terminal_contract/retained_fault_recovery_suppressed_after_ready` when a
post-ready host-health sample reports a transient retained fault inside the
settle grace. This is evidence of a first-attach race that should be watched,
but it must not trigger another xterm remount while the just-ready terminal is
still settling. If the same blank surface survives past the grace window, the
normal retained-fault recovery warning is allowed to fire.

Retained remote rehydrate also records `terminal_io` events when the current
daemon endpoint is not ready before replaying a preserved PTY snapshot:

- `retained_rehydrate_daemon_ready_wait`: the GUI had to wait at least 100 ms
  for the current daemon endpoint before retained rehydrate could safely read
  through the hot-update owner map.
- `retained_rehydrate_daemon_ready_error`: the current daemon endpoint could not
  be made reachable, so retained rehydrate did not attempt a stale read against
  a missing socket.
- `terminal_contract/retained_fault_recovery_rearm_deferred_daemon_ready`: the
  retained-fault watchdog reached its remount deadline while the same session
  was still waiting for the current daemon endpoint. This should defer the
  watchdog and reschedule observation; it must not remount xterm.

These events are distinct from preserved-owner failures. A current endpoint
readiness wait is a startup/handoff timing problem; an owner endpoint failure is
a preserved PTY ownership problem.

For prompt-follow incidents, app-control also reports viewport provenance:
`public_viewport_y` is xterm.js's public buffer counter, `visual_viewport_y` is
derived from the DOM viewport scroll position, `viewport_y` is the effective
value used by recovery checks, and `viewport_y_source` says which one won. A
large split here is evidence of a WebKit/xterm viewport synchronization bug, not
by itself proof that the daemon PTY is stale.

For row-paint incidents, app-control also reports `dom_paint_hit_test_problem`
and `dom_paint_hit_test`. The hit-test samples the active host center, a visible
xterm row, the cursor row, and the cursor sample with `document.elementsFromPoint`.
If xterm DOM rows contain text but the sampled row/cursor point is not topmost
inside `.xterm-rows`, terminal readiness must fail with the stable reason
`active terminal DOM rows are present but not paint-visible`. This catches the
class where DOM/state says the terminal is healthy but WebKit/compositor paint
or an overlapping shell layer makes the viewport blank or stale.

## Terminal Input Counters

The xterm.js `onData` event is not proof that bytes reached the daemon. Stable
Yggterm batches ordinary user input for a few milliseconds before crossing the
Rust/daemon bridge so fast Codex typing does not create one IPC wakeup per
character. App-control timing therefore exposes both sides of the boundary:

- `data_event_count`: raw xterm.js input events observed in the mounted host.
- `pending_input_bytes`: queued user input that has not been flushed yet.
- `pending_input_flush_scheduled`: whether a short flush timer is active.
- `input_batch_flush_count`: completed user-input batch flushes.
- `last_input_batch_length`: bytes/chars in the most recent flush.
- `last_input_batch_flush_reason`: `timer`, `immediate`, `enter`, `control`, or
  a cleanup reason.
- `last_input_batch_at_ms`: wall-clock timestamp for the most recent flush.
- `last_pending_input_reason`: why the current pending batch is waiting.

A probe must not report "accepted input without daemon echo" while
`pending_input_bytes > 0`. Once pending bytes are zero and a batch flush has
occurred, the usual daemon-output echo/readback expectations apply only for
sparse or broken prompt layouts. A visible current prompt row with acceptable
blank rows below the cursor remains writable even when no newer daemon write
arrives after the last typed byte.

=======
>>>>>>> c162185 (Snapshot alpha blur experiment)
## Why This Exists

The repeated regressions came from cross-layer truth splits:

- The sidebar/session store could still know about a Codex session while the active daemon no longer owned its PTY runtime.
- A remote machine could be marked healthy while its session tree was empty or terminals launched from it failed to draw.
- App-control could observe an empty xterm surface, but recovery code could forget the exact fault reason and fall back to a slow generic watchdog. A manual switching pass then appeared to fix the terminal, hiding the actual defect.
- Perf JSONL had high-volume render/write samples, but it was awkward to query by session identity and did not provide a stable terminal incident timeline.

Terminal telemetry makes those splits durable and queryable before another release is declared healthy.

<<<<<<< HEAD
One repeated mistake was treating a successful recovery as proof that the
underlying contract was healthy. Telemetry must preserve the incident even when
the user-facing surface later heals. A session that becomes usable only after a
switching pass, manual redraw, stale-host remount, or force restart still counts
as a failed first attach until a later release gate proves the first attach path
directly.

=======
>>>>>>> c162185 (Snapshot alpha blur experiment)
## Smoke Test Expectations

Terminal smoke tests should assert telemetry as well as pixels and app-control state:

- Opening a terminal must create a `terminal_open_attempt/begin` event.
- A successful terminal must create `terminal_open_attempt/ready`.
<<<<<<< HEAD
- Fast typing probes must report the input-batch counters and require pending
  input to drain before declaring the terminal healthy or broken.
- A blank xterm surface recovery must create a `warn` event before it is considered fixed.
- A renderer split, such as buffered xterm text with blank or low-contrast
  pixels, must create a `terminal_render_health_unhealthy` warning and must fail
  the screenshot/app-control pair until the rendered pixels and buffer truth
  agree.
- A retained remote terminal must not create multiple retained-fault open
  attempts after the first `ready` event unless the surface remains faulty
  beyond the settle grace; a suppressed post-ready sample should be visible as
  `retained_fault_recovery_suppressed_after_ready`.
=======
- A blank xterm surface recovery must create a `warn` event before it is considered fixed.
>>>>>>> c162185 (Snapshot alpha blur experiment)
- A live session listed in the sidebar but missing from daemon runtime truth must be treated as an incident.
- A healthy remote machine with an empty session list after refresh must be treated as an incident when the test requested a terminal from that machine.

The database is not a substitute for screenshots or viewport probes. A release-quality proof still needs the trio: app-control state, screenshot, and the relevant terminal probe.
