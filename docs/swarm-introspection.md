<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Swarm inspection CLI

`ygg`, `ygg dream`, `ygg service`, `ygg schedule` and `ygg swarm` teach the
next word with examples. Help is local: even `ygg dream topup --help` cannot
invoke a backend, discover hosts, authenticate or dispatch work.

```sh
ygg dream
ygg dream ls --help
ygg help examples --search "unused quota"
ygg complete -- dream ls --state f
ygg completion bash
```

Source the output of `ygg completion bash` to enable completion in a Bash
session. Completion uses the local command catalog; it does not probe hosts.

The portable command tree in `tools/swarmharness/introspection/commands.py`
owns inspection argument parsing, local help, task examples and completion.
The fleet skill's `ygg_command_tree.py` is a source alias to that module;
dispersal installs standalone regular-file bytes, so no checkout is needed
on a target host. The `tier_data` shipment carries importable modules without
adding misleading executables to PATH.

## Availability and compatibility

Query/attach leaves resolve a real bounded snapshot through the inspection
adapter (see the truthfulness laws below). When the adapter itself cannot be
imported, a query exits 3 and reports `adapter_unavailable` with unknown
coverage; an unimplemented leaf reports `adapter_not_implemented`. An empty
array with unavailable coverage is not proof of zero dreams or healthy
services. The CLI does not authorize spending, repairs, resume or schedule
execution.

Existing `dream status/topup/supervise`, `fleet pressure`, `agents ps`,
`memory`, `auth`, `ci`, `init`, `doctor` and row tools retain their owned
handlers and declared arguments. New inspection leaves reject unknown or
abbreviated options before source access. Compatibility tools with existing
parsers retain those validators. The older ad-hoc dream/fleet tools recognized
only `--json` and silently ignored other arguments; both noun and direct-script
spellings now reject unsupported arguments locally. This intentional tightening
prevents a mistyped preview flag from executing an operational command.
Help for both kinds is intercepted locally. Known direct-script spellings
remain supported; arbitrary filesystem paths are not executable commands.

Noun-only invocations now show help. Use explicit `ygg dream status` or
`ygg agents ps` for the former default views. The repository caller audit
found no runtime users of the former noun-only forms in the inspected tools.

`ygg board read/list/summary/post/answer/graduate` addresses topic boards via
`msgboard` with that tool's arguments (`read` emits JSON Lines and takes no
`--json` option). `ygg board scan --json` derives open dreams from answer
reference edges; malformed/skipped source records report unknown coverage.
`ygg board rows` exposes the existing row-board tool; its old
flag form such as `ygg board --json` remains a compatibility alias. Board
mutation verbs retain their ordinary authority/privacy obligations.

# Swarm introspection (ygg4 inspection unit contract)

Read-only, falsifiable observation of the swarm: one bounded capture per
query, immutable snapshots, and coverage rows that never claim more than the
source proved. The implementation lives in
`tools/swarmharness/introspection/` (collectors, adapters, model, store)
behind the `ygg-inspect.py` porcelain.

Status: worker-tested draft on lane `lane/ygg4/inspection`. Not editor
accepted, not release claimed. Known open units are listed at the end.

## Truthfulness laws (enforced by code, falsified by tests)

1. **A read is not an observation.** Coverage `observed_at` is the SOURCE's
   own as-of time, never the reader's clock. A source as-of older than its
   TTL demotes the row to `stale` with the age on the record; an unknown
   source time demotes to `unknown`. A frozen days-old heartbeat file can
   never certify current health — `test_inspection_repair.FrozenHeartbeat`
   holds the parent probe as executable falsifiers.
2. **Per-plane freshness.** Every quota plane carries its own
   `observed_at`; `collectors.plane_fresh` grades it `current | stale |
   unknown` against the heartbeat TTL. Stale, failed, `lastverified` and
   untimeable planes keep their numbers as history but surface
   `quality != verified` and `unknown_reader: true` in pressure — never
   current verified capacity, never zeros.
3. **A worker verdict is not editor acceptance.** Dreamer audit PASS/FAIL
   verdicts are retained verbatim as claims with `verification:
   "unverified"`. `editor-verified` requires a sessionReview chain that (a)
   passes hash-chain validation on read, (b) carries a terminal COMPLETE
   resolution, (c) carries an editor verdict whose invocation receipt
   authenticates against the control key on the reading host, linked by the
   audit row's `review_id`. Without the key the chain is structural
   evidence only. A tampered chain is rejected, not appeased. Falsifiers:
   `EditorVerification` (forged PASS, tampered chain, incomplete chain,
   keyless chain).
4. **Enrollment is fail-visible in both directions.** Modes:
   - `standing` — no active introspection subtree: documented read-only
     operator defaults apply, and every coverage row carries
     `collection_mode: "standing"` plus "not configured-complete".
   - `configured` — active config, subtree enabled: EXACTLY the enrolled
     sources are collected; enrolled `host/units/path/ttl` actually drive
     the collectors. A non-local enrolled host is `unobserved` ("not
     silently widened"), a disabled source is recorded as not collected, an
     unimplemented collector kind is an explicit open — no standing
     fallback.
   - `disabled` — subtree explicitly disabled: capture fails closed
     (empty datasets, one `unavailable` coverage row, explicit finding).
     Standing reads are NOT silently substituted.
   - `invalid` — introspection config faults or an unreadable config
     layer: same fail-closed frame with the faults recorded.
5. **Bounded reads before allocation.** `_state` stats first, refuses
   over-limit files without loading them ("content unknown", never a
   truncated parse), and treats an empty file as `invalid` — an anomaly,
   not a proven absence. `read_jsonl` tails within a byte bound, counts
   malformed lines, and reports truncation.
6. **Separate compute populations.** Queue-admitted dreams
   (`origin: "queue-admitted"`) and manual owner workers from the row
   registry (`${YGGTERM_HOME:-~/.yggterm}/rows`,
   `origin: "manual-owner-worker"`) are separate datasets; the summary row
   counts workers separately and carries `queue_is_not_whole_compute`.
7. **Expected vs observed services.** The standing roster (including the
   `executor@.service` template) is distinguished from observed instances;
   template names systemd did not instantiate are named in the coverage
   reason. Supervisor-observed executors carry
   `provenance: "supervisor-read"`.

## Snapshot store

`~/.yggterm/swarm/introspection/` (override `YGG_INSPECT_STORE`):

- `snapshots/<snapshot_id>.json` — write-once immutable frames
  (tmp+fsync+rename; saving an existing ID is a no-op, never an overwrite;
  tampered files fail digest validation on load).
- `events/YYYY-MM.jsonl` — bounded monthly maintenance partitions
  (dedupe by row ID, malformed rows refused, flock-serialized).
- Retention: `raw_days` + `max_store_bytes`; pruning removes whole expired
  artifacts and reports exactly what it removed.

SessionReview chains (trusted editor receipts) live in
`~/.yggterm/swarm/reviews/` (override `YGG_INSPECT_REVIEWS`) and are only
read, never written, by this unit.

## Query protocol (native backend contract)

The native CLI forwards a query to the Python helper over this contract:

- Entry: `ygg-inspect.py` (porcelain) resolves the adapter beside the repo,
  `YGG_KERNEL_DIR`, or `~/.yggterm/swarmkernel`, then calls
  `adapters.serve(path, args)`; `path` is the command-word tuple,
  `args` the flat IR argument dict (`json` popped; optional `snapshot`
  64-hex retained ID).
- Result JSON: `{schema_version, snapshot_id, as_of, query_as_of,
  config_hash, coverage[], items[], total, next_cursor, findings[]}`.
- Exit codes: `0` complete-and-verified, `3` partial/unknown scope,
  `1` falsified check, `2` usage, `3` adapter unavailable (never an empty
  healthy ledger).
- Cursors are bound to the exact snapshot ID + query key; a cursor replayed
  against another snapshot or a narrowed scope is rejected.
- Snapshot cache reuse honors the minimum per-source TTL; a stale cached
  frame never impersonates fresh data.

## Kernel staging

A payload kernel works without a checkout. `YGG_KERNEL_DIR` (or the staged
`share/ygg/kernel` layout) must contain:

```
<kernel>/ygg-inspect.py
<kernel>/ygg_command_tree.py
<kernel>/commands.json              # beside ygg_command_tree.py (with_name resolution)
<kernel>/swarmharness/__init__.py
<kernel>/swarmharness/{swarm_ledger.py, swarmconfig.py}
<kernel>/swarmharness/introspection/{__init__,collectors,adapters,model,store}.py
```

`collectors._ledger_path_dirs()` tolerates the repo layout
(`<root>/tools/swarmharness`) and the staged layout alike; a flat
`<kernel>/introspection/` is also accepted by the porcelain. Verified by
`KernelLayout` in the test file and by a staged out-of-checkout run.

## Falsification and packaging

`tests/test_introspection_cli.py` in the fleet skill runs the shipped router
against isolated recording backends, exercises help at every catalog prefix,
checks invalid operands/flags, parses examples, tests completion, verifies
argument-preserving compatibility routes and extracts a standalone shipment.
The inspection truthfulness falsifiers live in
`tools/swarmharness/introspection/test_inspection_repair.py` (frozen
heartbeat, forged editor PASS, enrollment fail-closed modes, bounded reads,
empty-vs-absent, fuzz, snapshot tamper, retention, cursor binding, partial
scope, privacy projection, staged-kernel import). Python tests use
disk-backed scratch under `~/.yggterm/scratchpad/` — never `/tmp`.
Runtime source coverage and owner-visible session attachment require
separate adapter tests; local command conformance does not establish a
complete inspection release.

## Open units (exact criteria, not healthy-empty)

- `composer`, `reviews`, `memory_lock` enrolled kinds have no collectors:
  configured captures record them `unobserved` with "collector not
  implemented". Open until each has a bounded collector + falsifier.
- Dynamic executor instance discovery (`executor@<instance>.service`) is
  daemon-lane scope; this unit reports the roster gap, it does not invent
  instances.
- A dedicated `swarm workers` CLI path needs `commands.json`/`commands.py`
  (not this unit's files); worker rows are in the snapshot `workers`
  dataset and counted in the summary today.
- No sessionReview chain can be produced end-to-end in this unit's tests
  without a control key; editor-verified repairs therefore cannot occur on
  a keyless reader by design. Receipt-production tooling is the review
  lane's scope.
