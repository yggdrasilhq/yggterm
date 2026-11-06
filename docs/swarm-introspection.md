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

## Output contract: human default, `--json` machine

`--json` prints `json.dumps` of the query result verbatim — the machine
contract. It is byte-stable across formatting changes; the human projection
below never feeds it.

The default human output is a scan-friendly projection of the same result:
one logical row per line (piped `grep`/`awk`/`sed` stay stable), fixed-width
key=value cells, no color or status noise. Rules:

- **Coverage table** shows each source with its effective freshness
  (`verified` stays quiet; `STALE`/`UNAVAILABLE`/`CONFLICTING`/`UNKNOWN` are
  loud), observed age (`age=` from the source's own clock, `never-observed`
  when the source carried no time), ttl, and the exact demotion reason.
- **Freshness is computed from coverage only.** A recorded state value is
  shown verbatim, but when its source is not verified-fresh the view says so:
  `composer state=ok freshness=STALE age=9d00h` plus "last reported value …
  does not assert current health". Stale ok is never rendered as healthy.
- **Missing ≠ zero.** Values the capture never observed render as `-` or an
  explicit word (`not-collected`, `unreported`, `serve unobserved`), never as
  0. A fail-closed capture says so; its rows are not observations.
- **Scope honesty.** Standing mode is labeled "standing local collection;
  NOT a complete fleet view". Queue tallies carry "running=0 is not zero
  agents". An empty verified scope is "not proof the fleet is idle"; `swarm
  check` prints a legend where `unknown` is not a pass.
- **Units are kept, never invented.** Funding grants/windows render the
  provider's native fields and unit values (`grant_units=125000`); a missing
  unit stays `-`. Windows flatten field by field (`window[0] remaining=…
  usage=…`), overflow is declared (`windows[4..] (N more; see --json)`).
- **Cells are safe.** C0/C1/ANSI bytes and embedded newlines are stripped;
  credential-like text is redacted; truncation is marked with an explicit
  `...` suffix (the full value stays in `--json`); the snapshot header shows
  a short id and says so.
- **Unknown row shapes** (composer cursor/state rows, future summary keys,
  unrecognized pressure rows) get a bounded deterministic key=value
  flattening — never a raw JSON dump, which remains available via `--json`.

Residual cases (known, deliberate): values longer than a column are marked
truncated rather than wrapped; flattened unknown rows show at most ~12
fields with a declared remainder; the flattened ordering of provider windows
prefers remaining/usage/number/nextResetTime and sorts any other provider
fields alphabetically; ages beyond a day render as `NdHHh`; `age=?` marks an
untimeable or clock-skewed observation instead of guessing.

## Falsification and packaging

`tests/test_introspection_cli.py` in the fleet skill runs the shipped router
against isolated recording backends, exercises help at every catalog prefix,
checks invalid operands/flags, parses examples, tests completion, verifies
argument-preserving compatibility routes and extracts a standalone shipment.
The help falsifier fails against the previous router because help invokes
backends or cannot discover the command groups. The existing dispersal suite
checks manifests, drift, PATH links and preservation of owner-managed files.

Python tests use disk-backed scratch through `TMPDIR`. The human projection
is falsified by `tools/swarmharness/introspection/test_human_output.py`: it
drives the real formatter with typed fixtures (stale-ok contradiction,
missing quota units, nested windows, unknown rows, unicode/control bytes,
zero/empty/partial coverage), captures stdout, pins parser/help to no I/O,
and compares `--json` bytes against the pristine module copy. Runtime source
coverage and owner-visible session attachment require separate adapter tests;
local command conformance does not establish a complete inspection release.
