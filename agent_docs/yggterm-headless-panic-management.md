# yggterm-headless Panic Management

`yggterm-headless server monitor` is the SSH-safe incident tool for Yggterm terminal failures. Use
it before guessing from screenshots or editing restore/rendering code. The old standalone incident CLI
surface has been folded into this headless command so operators have one binary for daemon control,
app-control, tracing, and panic management.

## When To Use It

- terminal session is hung or input-lagged
- session is missing after restore or reconnect
- daemon status/snapshot is slow or blocked
- desktop viewport is blank, stale, or unreadable
- direct-install GUI and daemon versions look mismatched
- a remote host such as `jojo` needs diagnosis without taking over the GUI first

## First Pass

Run a read-only incident report and save JSONL evidence:

```bash
yggterm-headless server monitor \
  --scenario panic-report \
  --expect-path "<session-path>" \
  --jsonl-out /tmp/yggterm-incident.jsonl
```

For intermittent failures, watch over time:

```bash
yggterm-headless server monitor \
  --scenario panic-report \
  --expect-path "<session-path>" \
  --iterations 30 \
  --interval-ms 1000 \
  --jsonl-out /tmp/yggterm-watch.jsonl
```

## Triage Map

- No reachable daemon or stale version: run `server-list`, inspect sockets/install metadata, and use
  `hot-restart --all` only when the replacement binary is known-current.
- Expected session missing: run `wait-session --expect-path <session-path> --timeout-ms 30000` and
  inspect restore/session graph logic.
- `status` or `snapshot` slow: run `latency-check --all`, then inspect event trace and perf telemetry
  for blocking daemon work.
- Daemon state healthy but screen is blank or text/input is wrong: switch to `yggterm server app
  state`, `screenshot`, `probe-type`, `probe-scroll`, or `probe-select`.
- Remote Codex/tooling suspect: run `managed-cli-refresh --foreground` or target a machine with
  `--machine-key`.

## Principle

The headless monitor should establish facts, not hide symptoms. Keep incident commands read-only
until the report clearly points at daemon lifecycle recovery, session restore, or managed CLI
refresh as the right next step.
