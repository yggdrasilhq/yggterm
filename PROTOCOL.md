# Yggclient / Yggserver Protocol

This document defines the next protocol layer between the desktop client (`yggclient`) and the
daemon/runtime (`yggserver`).

The current codebase still uses direct request/response RPC for many operations. That is sufficient
for correctness, but it is not sufficient for a responsive remote-first UI. The protocol defined
here exists to make slow work survivable.

## Goals

- Never block the whole UI while waiting on remote or daemon work.
- Let each surface fail independently:
  - sidebar
  - preview
  - terminal
  - metadata rail
  - search
- Prefer cached/stale data quickly, then refresh in the background.
- Explain long-running loading states to the user instead of silently hanging.
- Make latency, retries, and stale-cache behavior measurable in a mock client.

## Core Rule

The client must never treat a slow server request as permission to freeze the whole shell.

Instead:

1. Enter a local loading state for the affected surface immediately.
2. If fresh data is not available fast enough, use stale/cached data when policy allows.
3. If the loading state lasts longer than `3000ms`, emit a user-visible notification explaining:
   - what is loading
   - why it is slow
   - whether cached data is being shown
4. Continue syncing in the background.

## Envelope Model

The protocol layer is represented in code by:

- `YggRequestMeta`
- `YggEventEnvelope`
- `YggTarget`
- `YggSurface`
- `YggCachePolicy`
- `YggOperationPriority`

The server should be thought of as emitting an event stream for every request:

1. `accepted`
2. `loading`
3. zero or more `progress`
4. `result` or `error`

Even when the transport remains request/response for now, the client should model work in this
shape so UI behavior stays resilient when we later move to richer streaming/event transports.

## Request Metadata

Every user-visible request should carry:

- `request_id`: stable correlation key
- `operation`: semantic name such as `snapshot`, `remote_preview_sync`, `search_sidebar`
- `target`: app/session/machine/search target
- `surface`: which part of the UI is waiting
- `priority`: interactive, background, or passive
- `cache_policy`: fresh-only, prefer-fresh, stale-then-refresh, cache-only
- `notify_loading_after_ms`: usually `3000`

## Cache / Staleness Semantics

The default interactive behavior is:

- `PreferStaleThenRefresh`
- serve stale data immediately if available
- refresh in background
- replace stale UI only when newer data arrives

Recommended surface defaults:

- Sidebar tree: stale-then-refresh
- Preview: stale-then-refresh
- Metadata rail: stale-then-refresh
- Terminal attach/ensure: prefer-fresh
- Search result navigation: stale-then-refresh for indexes, fresh for currently opened content

## Loading UX

Loading must be scoped to the affected element, not the entire window.

Examples:

- Sidebar tree still usable while a single remote machine refreshes.
- Preview header/body can show stale content while a resync spinner runs.
- Terminal pane can show a resume overlay while PTY attach completes.
- Search can keep the current visible result set while updating counts.

If loading exceeds `3000ms`, notify the user with the concrete reason:

- waiting on remote SSH machine
- daemon still starting
- stale cache shown while refreshing
- terminal resume still attaching

## Retry / Recovery

- Interactive failures should back off, not pulse forever.
- Cached data must remain visible when safe.
- Duplicate background work should coalesce by semantic job key.
- Retry policies should be visible in telemetry and reproducible in the mock client.

## Session Lifetime Semantics

The client and server must distinguish between accidental disconnect and intentional shutdown.

### Accidental Disconnect

Examples:

- laptop sleep
- Wi-Fi drop
- SSH disconnect
- yggclient crash
- `Ctrl+C` / process kill during local development

Expected behavior:

- the local `yggserver` must keep running
- local PTY sessions must stay alive
- remote Yggterm-managed sessions must stay alive
- reconnecting from a later yggclient should restore the same running sessions

This is the Yggterm equivalent of GNU Screen or tmux persistence. Client death must not imply
session death.

### Intentional Shutdown

Examples:

- clicking the custom titlebar close button
- explicit `yggterm server shutdown`

Expected behavior:

- the client requests graceful shutdown
- local PTY sessions are terminated cleanly
- remote Yggterm-managed persistent sessions are terminated cleanly
- only after that does the client window close

Intentional shutdown is the only path that should tear down the whole Yggterm session graph.

### Remote Persistence Model

Remote long-running Codex sessions should be owned by the remote headless Yggterm surface rather
than by the lifetime of an SSH attach process.

Current direction:

- `yggclient` talks to the local daemon
- the local daemon bootstraps a headless `yggterm` binary on the remote machine
- remote resume/attach operations run through that remote helper
- remote helper persists the actual long-running Codex session independently of the current SSH
  attach, so a dropped SSH connection does not destroy the work

The current implementation may use tmux-backed persistence internally, but the protocol contract is
more general than tmux itself.

## Mock Client

`yggterm-mock-cli` exists to profile the protocol behavior without the full desktop shell.

It should be able to:

- probe `ping`, `status`, `snapshot`
- measure repeated latency
- emit JSONL envelopes for success, slow-load, and failure paths
- simulate client-side timeout thresholds and delayed-loading notifications
- inject artificial latency with progress ticks so loading-state UX can be tested deliberately
- prove session lifetime semantics across disconnect/reconnect/shutdown paths

This makes distributed regressions easier to reproduce than relying on the full GUI alone.

Example:

```bash
./target/debug/yggterm-mock-cli \
  --scenario startup \
  --delay-ms 4200 \
  --progress-step-ms 700 \
  --jsonl-out /tmp/yggterm-mock-cli.jsonl
```

That should emit:

- `accepted`
- `loading`
- several `progress` envelopes during the injected delay
- a `progress` envelope once the `3000ms` loading threshold is exceeded
- final `result` or `error`

Session lifetime examples:

```bash
# Start a live shell session, then let the mock client exit without shutdown.
./target/debug/yggterm-mock-cli \
  --scenario disconnect-safe \
  --cwd ~/gh/yggterm \
  --title-hint "mock reconnect probe"

# Reconnect from a later client process and verify the same session is still present.
./target/debug/yggterm-mock-cli \
  --scenario reconnect-check \
  --expect-path local://<session-uuid>

# Explicit shutdown should tear the session graph down.
./target/debug/yggterm-mock-cli \
  --scenario graceful-shutdown
```

Expected semantics:

- `disconnect-safe` must leave the daemon and session graph alive
- `reconnect-check` must see the same session after the earlier client process has exited
- `graceful-shutdown` must terminate the daemon and make later `ping` fail

Observability:

- `status` should expose whether the daemon restored from persisted cached state
- `status` should expose the restored stored/live/remote-machine counts
- `yggterm-mock-cli` should emit those fields in startup and reconnect scenarios so cache-path
  regressions can be profiled without the desktop shell
- `server trace tail <lines>` should dump the last trace probes from `~/.yggterm/event-trace.jsonl`
- `server trace follow <lines> [poll_ms]` should stay attached and stream new probes as they land
- `server trace bundle <lines> --screenshot` should emit a support bundle with event trace tail,
  perf tail, UI telemetry tail, daemon summary, and a best-effort screenshot path when a display
  capture tool is available

## Search

Search should also use the protocol mindset:

- sidebar results can render immediately from cached tree state
- preview hit counts can update independently
- active terminal search should never freeze the rest of the shell

When search is active, the UI should make it obvious that:

- `/...` is reserved for yggterm commands
- `Ctrl+Shift+P` focuses the search bar immediately

## Current Status

The envelope types are implemented in `crates/yggterm-server/src/protocol.rs`.

The transport is not yet fully event-stream based, but new client features should be designed as if
they are consuming `accepted/loading/progress/result/error` events rather than blocking RPC calls.
