# YGG4 Daemon Contract — GUI-independent worker identity, hierarchy and relay lineage (lane DAEMON)

Status: **draft slice, implemented-unbuilt-locally** — compile proof is owned
by the parent's isolated ygg-ci integration build; this document states the
contract the slice implements, not a shipped capability.

Source of truth for the code: `crates/yggterm-server/src/worker_identity.rs`
(record plane), `crates/yggterm-server/src/server_cli.rs` (`server workers`
verb), dispatch in `apps/yggterm/src/bin/yggterm-headless.rs`, and the
portable bridge `tools/swarmharness/agent_runtime.py`.

## 1. What this slice is

A **worker identity plane**: daemon-home JSON records that name what a
daemon-owned row IS in the coordination topology, keyed by a caller-supplied
launch token. It adds zero spawn paths — the daemon's existing creation RPC
remains the only creator of rows — and zero GUI coupling: records are files
under `<home>/runtime/workers/`, they survive GUI exit, GUI absence and a
daemon hot restart, and nothing in the sidebar, outline or window tables
reads or writes them.

## 2. The taxonomy

Stable kebab tokens, CLI-consumable, defined once in
`worker_identity::WorkerRole` and mirrored (validated, not re-defined) in
`agent_runtime.py`:

| token | meaning |
|---|---|
| `worker` | a compute row executing assigned work |
| `group` | a named collection of rows sharing one campaign line |
| `orchestrator` | a row coordinating workers/groups; top level |
| `sub-orchestrator` | a coordinator delegated by an orchestrator (parent required) |
| `context-successor` | a row continuing another row's context (parent = predecessor token) |

**Relay law:** a relay is a *record fact* — `role = context-successor` plus
`parent_token` naming the predecessor's launch token. Relay does NOT mean a
GUI seat, a terminal window, or which surface a human is looking at. Task and
review lineage travel through the record (parent token, role, `source`
provenance), not through any viewport.

## 3. The primitives this builds on (verified positions, this session)

| primitive | where | role here |
|---|---|---|
| `AppControlCommand::CreateTerminal` | `crates/yggterm-server/src/app_control.rs` (~:844) | THE daemon creation RPC for a row; carries `purpose`, `outline_prefix`, `launch_options` |
| client wrapper `create_terminal_with_tenancy_and_launch` | `crates/yggterm-server/src/lib.rs` (~:34201) | the one implementation of "create a row with tenancy" |
| `ServerRequest::EnsureShellSession { session_id, ... }` | `crates/yggterm-server/src/daemon.rs` (enum ~:4108) | daemon-local idempotent ensure keyed by session_id |
| `CreatorStamp` (+ `parent_session_path`) | `crates/yggterm-server/src/session_tenancy.rs` (~:64) | the existing hierarchy edge written at create time |
| session markers `YGGTERM_SESSION_ID` / `LC_YGGTERM_SESSION_ID` | same file (~:92) | process → row path resolution (`local://`, `cc-runtime://`, `codex-runtime://`, `remote-cc://`) |
| `attach::run_attach` | `crates/yggterm-server/src/attach.rs` (~:102) | the attach path with `backend` tokens and `runtime/attach/<uuid>/session.json` |

(Positions verified 2026-10-28 on draft 98a68a93; they may drift — the names
are the contract.)

## 4. Record schema (`ygg4-worker-identity/1`)

```json
{
  "schema": "ygg4-worker-identity/1",
  "launch_token": "wave-1",
  "role": "sub-orchestrator",
  "session_marker": "local://row-17",
  "session_id": null,
  "harness": "zcode",
  "title": null,
  "source": null,
  "parent_token": "orch-1",
  "pid": 4242,
  "cgroup": "/user.slice/user-1000.slice/…",
  "pinned": false,
  "retired": null,
  "created_at_ms": 1769648189000,
  "updated_at_ms": 1769648189000
}
```

Every optional field is an **explicit unknown**: absent (Rust
`skip_serializing_if`) or null. `null`/absent means "the caller could not
say", never a guessed default. `session_id` is the agent CLI's native session
id — null for plain shells AND for unobserved CLI rows; `session_marker`
distinguishes the two. `cgroup` is a point-in-time observation bound to
`created_at_ms`, not live state.

Storage: one file per worker at
`<home>/runtime/workers/<sanitized-token>.json`, written write-then-rename.
Sanitization refuses to silently rename a token (a renamed identity is a
second identity); a token with no safe filename form is refused.

## 5. Laws

1. **Idempotent launch requests.** `register` with an existing token returns
   the existing record verbatim (`outcome: "existing"`) and writes nothing. A
   create whose answer was lost on an ambiguous network is retried with the
   same token and cannot duplicate. Role is immutable per token; a re-register
   claiming a different role is refused loudly.
2. **Live/pinned veto.** `retire` refuses while the recorded pid is live or
   `pinned` is true, unless `--force` — and the force is recorded in the
   retire note. A pinned row is somebody's deliberate keep-alive; a live pid
   is somebody's running worker.
3. **GUI independence.** GUI absent / closed / killed: records and the
   daemon-owned PTYs are untouched (they are daemon-home state). The record
   plane never touches the GUI row table, outline numbers or windows.
4. **SSH loss / reconnect.** Identity records name the row path, not a
   connection; the row survives transport loss per the daemon's own PTY
   ownership, and reconnection re-resolves the same native session.
5. **No new spawn path.** Registration records the identity of a session the
   caller ALREADY ensured through the creation RPC. A future "launch a
   worker" verb must compose create/ensure + register; it must not bypass
   the daemon's one creation implementation.
6. **3.x preservation.** The plane adds a new directory and a new verb; it
   changes no existing session metadata, no tenancy stamps, no attach
   behavior. An older daemon ignores the directory; an older CLI never
   issues the verb.
7. **Known limits, stated.** `pid_is_live` reads `/proc` (Linux-only truth;
   elsewhere it answers false and the veto degrades to the pinned check —
   recorded in the module header, recoverable by no caller from the answer
   alone). `cgroup` is a best-effort observation.

## 6. Exact CLI contract (staged for the CLI owner)

Dispatched in **`yggterm-headless` only** (the GUI-independent door), as a
pure home-directory verb — no daemon socket, no GUI:

```
yggterm-headless server workers register --launch-token <token> --role <role>
    [--session-marker <row-path>] [--session-id <native-id>] [--harness <name>]
    [--title <text>] [--source <text>] [--parent-token <token>] [--pid <n>]
    [--pinned] [--json]
yggterm-headless server workers ls [--role <role>] [--json]
yggterm-headless server workers show --launch-token <token> [--json]
yggterm-headless server workers annotate --launch-token <token>
    [--pid <n>] [--session-id <native-id>] [--harness <name>]
    [--title <text>] [--pinned <true|false>] [--json]
yggterm-headless server workers retire --launch-token <token> --reason <text>
    [--force] [--json]
```

`register --json` answers
`{"outcome":"created"|"existing","record":{…}}`; `retire --json` answers
`{"outcome":"retired"|"already_retired"|"vetoed","record":{…}[, "reason": …]}`
and the non-JSON veto path exits non-zero with the veto named. The portable
argv builders and response validators live in
`tools/swarmharness/agent_runtime.py` (`build_register_argv`,
`parse_register_outcome`, …) with a dependency-free self-test
(`python3 tools/swarmharness/agent_runtime.py selftest`).

## 7. Composition: a worker launch end-to-end (staged, not shipped)

1. Caller ensures the row through the daemon's creation RPC
   (`create_terminal_with_tenancy_and_launch` → `CreateTerminal`), recording
   the tenancy stamp (`created_by`, optional `--parent-session`).
2. Caller resolves the row path / session marker for the new row.
3. Caller registers the identity:
   `server workers register --launch-token <its own idempotency key>
   --role worker --session-marker <marker> --parent-token <orchestrator
   token> --harness <harness> --pid <row pid>`.
4. Retry of any lost step re-uses the SAME launch token; the answer is
   `existing`, never a duplicate row identity.
5. Completion retires the identity (`vetoed` while live/pinned), leaving the
   audit note.

## 8. Explicit non-goals and integration requests

- **Not owned here (do not assume):** CLI IR/frontend, README, installers,
  ynpm, source adapters. The CLI porcelain for workers belongs to the CLI
  owner; this plane stages the argv contract above.
- **Integration request (GUI binary):** `apps/yggterm/src/main.rs` does not
  dispatch `server workers`; the repo convention (see the
  `the_screen_verb_is_dispatched_by_both_binaries` structural test) is that
  `server` verbs are dispatched by both binaries. The verb is a pure
  home-directory read/write and could be added there in one line by whoever
  owns `main.rs`. Headless remains the GUI-independent door either way.
- **Integration request (daemon memory):** worker identity could later be
  projected INTO the daemon's in-memory row table (so `server snapshot`
  carries roles). Deliberately NOT attempted in this slice — that touches
  the 63k-line daemon core; the file plane is the honest first step.
- **Test proof status:** Rust unit tests are written in-module
  (`worker_identity.rs::tests`: idempotency, veto+force, explicit unknowns,
  role round-trip, relay-as-record-fact) and run under the parent's ygg-ci
  build only — no cargo has run in this worktree. The Python bridge
  self-test passes locally (rc=0, 2026-10-28). Disposal-only: no test
  launches a real worker, touches fleet rows, or writes outside a temp home.
