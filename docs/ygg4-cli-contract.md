<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# ygg 4.0 CLI contract

Status: **implemented-unbuilt** (the native frontend compiles only through
the fleet CI build; no acceptance is claimed by this document). The
normative product boundary is [yggterm-4-system.md](yggterm-4-system.md);
this page is the CLI lane's executable contract for it.

## One contract, two front ends

`tools/swarmharness/introspection/commands.json` is the single command IR
(schema 1, contract `ygg4-cli`). It owns grammar, options, operands,
choices, ranges, examples, completion data, component dispatch declarations
and role labels. Neither front end keeps a second command table:

- **Native:** `crates/ygg-cli` (binary `ygg`) — the 4.0 entry point. The IR
  is embedded at build time and also read from an installed payload
  (`$YGG_COMMAND_IR`, then `$YGG_BIN_DIR`/`~/.yggterm/bin/ygg`); an
  installed file that exists but fails schema/component validation is a
  hard refusal (exit 3), never a silent fallback to the embedded copy.
- **Compatibility:** the fleet-skill router (`ygg-inspect.py` over
  `ygg_command_tree.py`) keeps running during migration and validates the
  same document with the same structural checks.

`$YGG_COMMAND_CONTRACT_HASH` must equal the sha256 of the loaded IR bytes
or every effect refuses with exit 3; help, completion, version and examples
stay available. Both front ends hash the same bytes, so a mixed native /
compatibility installation detects itself.

## Command surface

Local-only routes (no backend, network, auth or source adapter is touched):
`ygg`, `ygg <group>`, `ygg ... --help`, `ygg help <topic>`,
`ygg help examples --search <words>`, `ygg complete -- <words>`,
`ygg completion bash`, `ygg version`, `ygg payload manifest`.

Effects and their exit codes:

| Situation | Exit | Contract anchor |
|---|---|---|
| Local help/version/examples/completion | 0 | handler `group`/`version`/`examples`/`completion`/`complete` |
| Component dispatch (legacy tools, `ygg ynpm`, `ygg skills`) | runner's own | handler `legacy` + `component` |
| Wired inspection query | adapter's own | `wired_adapters` (none this generation) |
| Unwired inspection query | 3 | staged `ADAPTER_NOT_IMPLEMENTED`, unknown coverage |
| Planned family beyond local help | 3 | `role: "planned"` |
| Parse/argument error | 2 | declarative fields (choices, `int_range`, `require_one_of`, `conflicts`, `operand_choices`, `required_options`) |
| Contract/IR mismatch, missing component tool, unprovisioned interpreter | 3 | loader + component declarations |

An unwired query prints `adapter_not_implemented` coverage with an unknown
verdict; an empty array with unknown coverage is never presented as healthy.
The `wired_adapters` header is the coordination point: a source-adapter lane
adds its family there together with the adapter implementation, and a
frontend generation that finds a wired family it cannot execute refuses
explicitly instead of staging a stale unavailability.

## Components: runners and data ownership

The `components` header declares, for every executable a command can reach:
the owning runner binary, its aliases (the legacy tool filenames), the
ordered resolution strategies, the interpreter requirement, and the data
domains it owns (and, where separate, reads). This is the fleet command
alias / data ownership declaration: a legacy filename has exactly one
component, and a component's data domain has exactly one owner.

Resolution strategies, evaluated in declared order:

- `env:<VAR>` — an explicitly set environment variable wins even when the
  file is missing; a broken override must be visible, not papered over.
- `home_bin:<name>` — `~/.local/bin/<name>` (owner-managed installs).
- `payload:<name>` — the dispersed payload tree (`$YGG_BIN_DIR` or
  `~/.yggterm/bin/ygg`).
- `path:<name>` — PATH search for a differently-named runner (`msgboard`).
- `exe_sibling:<name>` — next to the `ygg` binary itself (the `ynpm`
  component; cargo layout and colocated installs).

Honesty rules baked into dispatch:

- **No recursive porcelain.** A resolved runner whose canonical path is this
  binary is refused (exit 3). A shim that calls `ygg` again is not a
  component; resolve the real verified runner.
- **Interpreter-backed components are declared.** `interpreter: python3`
  (or `bash`) means the frontend checks the interpreter exists before
  dispatch and refuses with the missing name (exit 3). A bare system-Python
  assumption is not fresh-host proof.
- **argv, signals, exit status are preserved** by handing the process to
  the resolved runner through `exec` on Unix; the caller's shell observes
  the component's own exit code and signal death. There is no shell
  interpolation in the protocol.
- **Missing tool** resolves to exit 3 with the tool name, never a fallback
  execution with unvalidated arguments.

`ygg ynpm ...` and `ygg skills ...` are passthrough doors to the `ynpm`
component (env `YGG_YNPM_BIN`, exe sibling, `~/.local/bin/ynpm`, payload).
Package ownership, generations, verification and the desktop GitHub-release
schema stay in the ynpm crate; the porcelain does not duplicate them.
Legacy executable names (`ygg-memory`, `ygg-ci.py`, `ygg board ...`,
`ygg-fleet.py agents ps`, …) remain declared aliases during migration.

## Seed payload

The binary bundles the payload bytes and digests them; a seed installs and
verifies without a checkout or network:

```sh
ygg payload install --dir /path/payload   # writes exactly the manifest files
ygg payload verify [--dir /path/payload]  # per-file verified/missing/mismatch
ygg payload manifest --json               # the embedded manifest with sha256
```

Bundled: the command IR itself, the progressive [yggterm
skill](../.agents/skills/yggterm/SKILL.md), the 4.0 system contract, this
document and [swarm-introspection.md](swarm-introspection.md). `verify`
exits 0 only when every file matches its digest, 1 on any mismatch, 3 when
the directory is absent. `install` refuses to write the dispersed replica
tree (`~/.yggterm/bin/ygg`): one owner per path
([spec-ygg-verb-dispersal.md](spec-ygg-verb-dispersal.md)); installers stage
a payload elsewhere and dispersal remains the only writer of the replica.

## Entry-point and dispersal law

The native `ygg` binary is the entry point that owns PATH presence: once
dispersed, `~/.local/bin/ygg` must resolve to the native binary (or a
symlink into its payload), never to the compatibility Python router.
Dispersal shipments must dereference source aliases into standalone regular
files and must not replace the native entry point with a script. Until the
dispersal manifest carries the native binary, the fleet keeps the
compatibility router as the functioning `ygg`; this is an integration
request against `ygg-verbs.json`, not something this crate can do alone.

## Parity and tests

Both front ends must agree — same exit codes, same backend reach, same
staged-unavailable shape, same snapshot fingerprints for identical
requests — on: help at every prefix (never reaching a backend), invalid
arguments (never reaching a backend), legacy dispatch argv, the contract
hash gate, planned-family refusal (exit 3), and unwired-query staging
(exit 3). `tests/` in `crates/ygg-cli` runs the native frontend against the
compatibility router over a recording-shim matrix (loudly skipped only when
no Python interpreter exists at all);
`tools/swarmharness/introspection/test_commands.py` pins the contract's own
structure and validation semantics.

## What this lane does not own

README, seed installers, the release workflow, the ynpm implementation,
source adapters for inspection, and the dispersal manifest itself. Requests
against those: native binary in the dispersal shipment; CI builds the new
workspace member; parent owns seed packaging and platform matrix.
