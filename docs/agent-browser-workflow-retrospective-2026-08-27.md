# Agent-Owned Browser Workflow Retrospective — 2026-08-27

Status: observed during a live authenticated data-polishing and deployment
proof. This records product friction and desired improvements for Yggterm and
Ychrome agents. It contains no application-specific content.

## What Worked

- Session-addressed `server app web read|eval|screenshot --session` inspected a
  live background native surface without selecting the user's row.
- A named shadow isolated viewport-mode regression proof from the attended
  client.
- Ychrome ctl opened an authenticated offscreen page through a prepared named
  profile, drove the deployed UI, read rendered text, and captured pixels.
- App-control now capability-guards `--view preview` on native browser rows:
  request `Rendered`, effective `Terminal`, with a named adjustment.

## Friction And Failure Modes

### 1. View targeting was a first-command hazard

An untargeted `app open --view preview` selected the attended row and hid a
native browser page behind the shell's dead transcript placeholder. The runtime
guard shipped in `1474e1d7`, and the app-control skill now places the quiet-read
decision before its command catalogue. The remaining product question is
whether every viewport-mutating verb should refuse an untargeted call whenever
more than one live client exists.

### 2. A shadow is not a second owner for a permanent browser profile

A shadow can attach the shell row, but it cannot safely become another browser
profile owner while the attended Ychrome process holds that profile. The
correct split is:

- native session verbs for an already-live surface;
- shadow for shell geometry and mode behavior;
- Ychrome ctl engine for authenticated browser QA through the same identity.

This should become an explicit capability matrix in help output, not knowledge
an agent reconstructs after encountering a lock warning.

### 3. Cold native surfaces did not recover into useful page truth

After a GUI update, every tab in the prepared browser row reported
`no_webview`. `web ensure` accepted the request, queued a rebuild, then the next
read returned `about:blank`. This is already the same family documented in
`docs/agent-passkey-gap-2026-07-28.md`; the live recurrence shows it still
belongs in first-screen agent guidance.

Desired contract: `ensure` either restores the declared URL and reports a new
generation, or refuses with a reason that names why restoration cannot preserve
the page. `accepted:true` plus `about:blank` must never read as recovery.

### 4. Update proof lost its process and selection anchors

An update replaced the active GUI PID, briefly restored a different selected
row, and later advanced the installed build again through the rollout watcher.
Any proof tied to the prior PID/build became stale.

Desired contract:

- update/restart returns or makes queryable the successor client identity;
- the exact active session path and view mode are carried across replacement;
- app-control supports a stable attended-client alias rather than forcing PID
  rediscovery after every update;
- proof helpers stamp requested build, running build, client identity, active
  row before/after, and whether the result became stale during capture.

### 5. Equivalent app-control responses have different JSON paths

`server app state` exposes fields under `.data`, while an `app open` response
embeds its settled snapshot under `.data.state`. A plausible jq query returned
`null` rather than failing, delaying diagnosis.

Desired contract: one response envelope for settled state, or a CLI projection
such as `--field active_session_path` that owns the schema difference and exits
non-zero when a field is absent.

### 6. Ychrome ctl needs safer program delivery

Long `ctl eval js=...` programs must survive an SSH shell, the remote shell, and
the key/value parser. One probe failed with an unmatched quote. The existing
`docs/pending-bugs.md` entry calling for `js_file=PATH` or stdin is validated by
this workflow.

Desired contract: `ctl eval --stdin` and `ctl eval js_file=...`, with each
evaluation isolated from prior global declarations. Browser-smoke scripts
should not have to encode JavaScript into argv.

### 7. Full-page capture is root-scroll capture, not app-scroll capture

`ctl shot region=full prescroll=true` returned a 1365×900 image for a long
single-page application because the application scrolls an inner container.
The workaround was to locate the changed heading, call `scrollIntoView`, and
capture the viewport.

Desired contract: capture reports candidate inner scrollports and whether
`full` covered only the root. A future `region=scrollport selector=...` or
`region=auto-full` should stitch the selected scroll owner while restoring its
position.

### 8. Application semantics determine whether the browser is addressable

A completed pagination control exposed only `✓`; its page number disappeared
from both visible text and the accessible label. The browser stack could only
address page three by ordinal position. This is not a Ychrome selector defect,
but the control plane should make such application deficiencies obvious.

Desired contract: snapshots flag repeated anonymous interactive controls and
recommend accessible names. Application smoke should prefer stable semantic
labels, then explicit selectors, and use ordinals only with a documented
structural invariant.

### 9. Tool discovery differed between interactive and non-login shells

`ychrome` worked in the attended terminal but was absent from PATH over a plain
SSH command. The installed absolute path worked.

Desired contract: fleet bootstrap exports one documented non-interactive tools
path, and `yggterm server app` offers a `browser-engine` forwarding verb so
agents need not rediscover where the sibling binary lives.

### 10. A ctl page open failed once without an actionable reason

The engine returned HTTP 400 while opening a temporary proof page; the same
request succeeded immediately afterward. A bounded client retry keeps release
proof moving, but it cannot distinguish transient engine readiness from an
invalid request.

Desired contract: every ctl failure returns a stable machine-readable error
code, request id, retryability, and engine/profile state. A higher-level proof
primitive should own bounded retries and record them in its manifest.

## The Desired End State

One high-level proof command should accept:

```text
profile + URL + semantic route + assertions + screenshot target
```

and return one manifest containing:

```text
Yggterm build
Ychrome build
profile name (never cookie values)
page id and final URL
assertions and matched targets
screenshot path and captured scroll owner
attended client active row before/after
temporary-page/shadow cleanup status
```

That would turn the current correct-but-hand-assembled workflow into a durable
platform primitive while preserving the key invariant: authenticated browser
proof must not move the user's viewport.
