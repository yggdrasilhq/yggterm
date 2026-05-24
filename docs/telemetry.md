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

`active remote terminal lost expected scrollback after retained replay` is a
blocking terminal-open reason only when the current prompt is not ready, an
unsafe retained replay path is involved, or a scroll probe proves the viewport
cannot move through expected history. For a readable prompt-ready restore, the
same evidence remains available as observer data through `scrollback_expected`,
`base_y`, `viewport_y`, retained replay source fields, and probe-scroll output;
it must not hold user input behind a restore gate by itself.

## Initial Coverage

The GUI records `terminal_open_attempt` transitions:

- `begin`
- `first_output`
- `first_protocol_only_output`
- `first_meaningful_output`
- `ready`
- `recovering`
- `latched_failure`
- `request_failed`
- `session_failed`
- `cleared`

Each `terminal_open_attempt` event includes a `timing` object so slow resumes
can be attributed to the exact phase instead of guessed from screenshots:

- `elapsed_ms`: event time minus the open attempt start.
- `request_to_surface_mounted_ms`: time until the xterm host first mounted.
- `request_to_first_output_ms`: time until any daemon bytes reached the mounted
  xterm bridge. Payloads record byte counts and booleans, not terminal text.
- `surface_mounted_to_first_output_ms`: mounted-host wait before first bytes.
- `request_to_first_protocol_only_output_ms`: time until the first
  terminal-emulator protocol-only response or control exchange, when present.
- `request_to_first_meaningful_output_ms`: time until the first output that the
  resume gate considers session content rather than attach/control noise.
- `surface_mounted_to_first_meaningful_output_ms`: mounted-host wait before
  readable session content.
- `first_output_to_ready_ms`: wait between first bytes and the ready verdict.
- `first_meaningful_output_to_ready_ms`: wait between first meaningful content
  and the ready verdict.
- `request_to_ready_ms`: time until the terminal became readable/interactive.
- `surface_mounted_to_ready_ms`: xterm mount-to-ready latency.
- `request_to_failure_ms`: time until the first latched failure, when present.

The `begin` event also records an open-context snapshot: active session path,
active surface request, retained-terminal state, attach-in-flight flag,
bootstrap owner/lease, mount epoch, and whether the session had an older ready
attempt. This exists to catch the class where resume appears slow because a new
open skipped behind a stale bootstrap lease.

Severity mapping:

- `info`: normal lifecycle transitions.
- `warn`: recovery paths that should usually self-heal, such as empty xterm surface recovery.
- `error`: latched failures and failed terminal requests.

The mounted xterm host also records `terminal_contract/terminal_render_health_unhealthy`
when xterm.js reports buffered terminal text but the visible render health is
unhealthy, for example `canvas_blank_with_buffer_text` or
`canvas_low_contrast_foreground_with_buffer_text`. These events are throttled by
the host-health problem key so a single bad surface is logged once, while a
later heal and relapse is still visible as a fresh incident.

Clipboard telemetry uses the `terminal_io` category. Text copy and paste events
record action, character count, byte count, method, and reason, never clipboard
contents. The app-control terminal host snapshot exposes gesture counters for
the browser side of the bridge:

- `clipboard_paste_event_count`
- `clipboard_paste_duplicate_suppressed_count`
- `native_clipboard_paste_request_count`
- `native_clipboard_paste_request_deduped_count`
- `last_native_clipboard_paste_request_reason`
- `terminal_secondary_button_suppress_count`
- `focus_capture_pointer_events`
- `focus_capture_hit_target_enabled`

Use those counters to diagnose double-paste or swallowed-paste reports before
looking at raw desktop clipboard tools. A healthy text paste has exactly one
native paste request for one claimed terminal paste gesture; duplicate browser
events should increment only the suppressed counter.
For terminal right-click, `terminal_secondary_button_suppress_count` must
increase while `clipboard_paste_event_count`,
`native_clipboard_paste_request_count`, and terminal data-event counters stay
unchanged.
For terminal selection, the focus-capture overlay and context-menu backdrop are
observer/shell chrome only. They must not become terminal hit targets. A healthy
state reports `focus_capture_hit_target_enabled == false`, and `probe-select`
must prove selection through xterm's pointer path with non-empty
`term.getSelection()` plus selection-layer rectangles. DOM Range selection and
buffer-only fallbacks are diagnostics, not selection truth.

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
- `retained_replay_superseded_by_daemon_pty`: app-control field set on the
  terminal host after trusted live input promotes a retained replay source back
  to `daemon_pty`. Once this is true, retained replay is observer history only;
  it must not continue prompt-follow retries or repaint a live daemon PTY.
- `daemon_retained_replay_skipped_live_connected`: a delayed retained replay
  worker reached its scheduled window after the mounted xterm host had already
  accepted live input. This is the expected safe outcome; the worker must return
  without writing retained bytes or forcing viewport policy.
- `retained_rehydrate_skipped_live_connected`: the UI retained-rehydrate task
  reached its read window after the mounted host was already live-connected, so
  it returned without reading or writing retained bytes.
- `retained_rehydrate_result_discarded_live_connected`: retained rehydrate read
  a daemon snapshot, but a concurrent path promoted the host to `daemon_pty`
  before the write. The payload was discarded and must not become visible
  terminal truth.
- `retained_rehydrate_read_after_live_connected_history_seed`: a
  `CollapsedScrollbackRecovery` pass is intentionally reading retained daemon
  history after a short live read painted the screen, because no retained
  history has been staged and user input is still gated.
- `retained_rehydrate_live_connected_history_seed_allowed`: the matching write
  was allowed. App-control should then show retained-history scrollback before
  any later input-enabled `daemon_pty` promotion.
- `terminal_mount/terminal_attach_visual_reveal` or
  `terminal_mount/terminal_attach_visual_reveal_from_read`: the product render
  loop cleared the remote resume gate from daemon PTY/xterm truth. If the
  matching `terminal_open_attempt/ready` event appears only after an
  app-control `DescribeState`, the product loop is still missing a readiness
  path.
- `terminal_bootstrap_existing_lease_skip`: a terminal mount wanted to
  bootstrap but found an existing attach lease. This is a warning because it can
  be harmless for the same in-flight mount, but it is the primary evidence for
  slow resume stalls when paired with a long `request_to_surface_mounted_ms`.

These events are distinct from preserved-owner failures. A current endpoint
readiness wait is a startup/handoff timing problem; an owner endpoint failure is
a preserved PTY ownership problem.

Hot-update owner cleanup records `hot_update/preserved_owner_removed` with
reason `duplicate_legacy_owned_runtime_pruned_current_owned` when an old owner
has accepted local-only `DropTerminalRuntime` for a key that the current daemon
already owns. Absence of that event means the registry may still be survival
evidence, even if current daemon read/write bypasses the preserved owner for the
same key.

Hot-update owner alias repair records
`hot_update/preserved_owner_current_alias_retargeted` when a registry endpoint
canonicalizes to the current daemon but a reachable real owner reports the same
runtime key. The payload includes old/new endpoints, owner versions, owner pids,
and remaining keys. If the alias points at the current daemon and no reachable
owner reports that runtime key, the daemon records
`hot_update/preserved_owner_current_alias_unresolved`; that event is diagnostic
evidence only and must not make the current daemon proxy terminal I/O to itself
through a stale preserved-owner route.

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

## Why This Exists

The repeated regressions came from cross-layer truth splits:

- The sidebar/session store could still know about a Codex session while the active daemon no longer owned its PTY runtime.
- A remote machine could be marked healthy while its session tree was empty or terminals launched from it failed to draw.
- App-control could observe an empty xterm surface, but recovery code could forget the exact fault reason and fall back to a slow generic watchdog. A manual switching pass then appeared to fix the terminal, hiding the actual defect.
- Perf JSONL had high-volume render/write samples, but it was awkward to query by session identity and did not provide a stable terminal incident timeline.

Terminal telemetry makes those splits durable and queryable before another release is declared healthy.

One repeated mistake was treating a successful recovery as proof that the
underlying contract was healthy. Telemetry must preserve the incident even when
the user-facing surface later heals. A session that becomes usable only after a
switching pass, manual redraw, stale-host remount, or force restart still counts
as a failed first attach until a later release gate proves the first attach path
directly.

## Smoke Test Expectations

Terminal smoke tests should assert telemetry as well as pixels and app-control state:

- Opening a terminal must create a `terminal_open_attempt/begin` event.
- A successful terminal must create `terminal_open_attempt/ready`.
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
- A live session listed in the sidebar but missing from daemon runtime truth must be treated as an incident.
- A healthy remote machine with an empty session list after refresh must be treated as an incident when the test requested a terminal from that machine.

The database is not a substitute for screenshots or viewport probes. A release-quality proof still needs the trio: app-control state, screenshot, and the relevant terminal probe.
