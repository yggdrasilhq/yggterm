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

This first increment implements the command contract. Source inspection
adapters are added separately. Until an adapter is wired, a query exits 3
and reports `adapter_not_implemented` and unknown coverage. An empty array
with unavailable coverage is not proof of zero dreams or healthy services.
The CLI does not authorize spending, repairs, resume or schedule execution.

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

## Falsification and packaging

`tests/test_introspection_cli.py` in the fleet skill runs the shipped router
against isolated recording backends, exercises help at every catalog prefix,
checks invalid operands/flags, parses examples, tests completion, verifies
argument-preserving compatibility routes and extracts a standalone shipment.
The help falsifier fails against the previous router because help invokes
backends or cannot discover the command groups. The existing dispersal suite
checks manifests, drift, PATH links and preservation of owner-managed files.

Python tests use disk-backed scratch through `TMPDIR`. Runtime source
coverage and owner-visible session attachment require separate adapter tests;
local command conformance does not establish a complete inspection release.
