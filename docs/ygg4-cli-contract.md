<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# ygg 4.0 CLI contract

Status: **implemented, native re-verification pending** (the fleet CI
native compile runs against each pushed tip; no acceptance is claimed by
this document). The normative product boundary is
[yggterm-4-system.md](yggterm-4-system.md); this page is the CLI lane's
executable contract for it.

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

### Machine version contract

`ygg version --json` emits a versioned document; the frontend version and
the command-contract identity are separate facts and neither may masquerade
as the other (there is no "command contract 1" app-version confusion):

```json
{
  "schema_version": 1,
  "frontend": {"name": "ygg", "version": "<crate version>", "os": "<target os>"},
  "command_contract": {
    "name": "ygg4-cli", "schema_version": 1,
    "sha256": "<full 64-hex digest of the loaded IR bytes>",
    "source": "embedded | <file path>"
  }
}
```

The human `ygg --version` keeps two labeled lines in the same order:
frontend first, then the contract digest (truncated for readability).

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
A wired family is a typed route, not a refusal: the frontend resolves the
declared adapter file (`YGG_INSPECT_BIN` exact-file override, then
`<payload>/components/`, then the payload root), checks `python3` is
provisioned, and execs it — never asserting health on the adapter's behalf:

- **argv:** the full command path followed by the exact post-path tokens
  (`ygg dream ls --json` → `ygg-inspect.py dream ls --json`); quoting
  survives byte-exact because there is no shell.
- **environment:** `YGG_COMMAND_CONTRACT_HASH` (the frontend's loaded IR
  digest, so the adapter can verify the same generation),
  `YGG_KERNEL_DIR` (`<payload>/kernel`), and `YGG_COMMAND_IR` when the
  contract came from a file. The adapter's own verdict and exit status pass
  through untouched.
- **absent adapter:** a wired family whose adapter file cannot be resolved
  is an explicit refusal (exit 3), never a staged empty ledger.

The `wired_adapters` header is the coordination point: a source-adapter
lane adds its family there together with the adapter implementation, and a
frontend generation that finds a wired family it cannot execute refuses
explicitly instead of staging a stale unavailability. The contract
validates that wired entries name a bare adapter filename (no path
separators — no traversal).

### Remote-operator door

`ygg remote <host> -- <arguments>` execs the remote node's installed `ygg`
over ssh: `ssh -- <host> ygg <arguments>`. It is a remote-operator role,
separate from local worker readiness: it requires only an ssh client — no
local Python, GUI or source checkout — which is the fresh-Windows seed
path. Host tokens that look like options or carry whitespace/control
characters are refused (exit 2) before dispatch, and the argv carries an
explicit `--` so ssh can never read the host as an option. Remote exit
status propagates; local help stays local. No claim is made that Windows
is agent-ready.

## Components: runners and data ownership

The `components` header declares, for every executable a command can reach:
the owning runner binary, its aliases (the legacy tool filenames), the
ordered resolution strategies, the interpreter requirement, and the data
domains it owns (and, where separate, reads). This is the fleet command
alias / data ownership declaration: a legacy filename has exactly one
component, and a component's data domain has exactly one owner.

Resolution strategies, evaluated in declared order:

- `env:<VAR>` — an exact-file override: an explicitly set variable wins
  even when the file is missing; a broken override must be visible, not
  papered over. (`YGG_MEMORY_BIN`, `YGG_YNPM_BIN`.)
- `env_dir:<VAR>` — a directory override: the runner is looked up inside
  the named directory, flat or under `components/`, with the platform
  executable suffix applied. An explicitly set directory is also
  authoritative: no runner under it is an explicit refusal naming both the
  variable and the tool, never a silent PATH fallthrough.
  (`YGG_BIN_DIR` is a directory, not a file pointer.)
- `home_bin:<name>` — `~/.local/bin/<name>` (owner-managed installs).
- `payload:<name>` — the dispersed payload tree (`$YGG_BIN_DIR` or
  `~/.yggterm/bin/ygg`).
- `path:<name>` — PATH search for a differently-named runner (`msgboard`).
- `exe_sibling:<name>` — next to the `ygg` binary itself (the `ynpm`
  component; cargo layout and colocated installs).

On Windows, every directory-join strategy (`env_dir:`, `payload:`,
`exe_sibling:`, `path:`) also tries the platform executable suffix
(`ynpm` → `ynpm.exe`). Batch-wrapper resolution is declared unsupported
until platform evidence exists; source role limits stay explicit.

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

Installed generation layout, declared once (resolver reads only; the
packager creates): `commands.json` at the payload root, components under
`components/`, the kernel under `kernel/`. The component resolver accepts
both the flat dispersed layout and the packaged `components/` layout for
`env_dir:` and `payload:` strategies.

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
(exit 3). One declared divergence: for passthrough records the native
frontend rejects unknown options before the `--` delimiter (strict option
loop, exit 2) while argparse's REMAINDER captures them verbatim; the
native tests pin the rejection, the Python tests pin the capture.
`tools/swarmharness/introspection/test_commands.py` pins the contract's own
structure, validation semantics and resolution-strategy forms;
`crates/ygg-cli/src` unit tests pin the native resolver, forwarding and
remote door.

## What this lane does not own

README, seed installers, the release workflow, the ynpm implementation,
source adapters for inspection, and the dispersal manifest itself. Requests
against those: native binary in the dispersal shipment; CI builds the new
workspace member; parent owns seed packaging and platform matrix.

## Integration availability

The native inspection families delegate to the matching `ygg-inspect.py`
component with the selected command hash and kernel generation. Missing
Python or component bytes refuse the operation explicitly; local help,
examples, completion, payload inspection and remote operator routing remain
native. SSH remote operations use one quoted POSIX command so spaces and
shell metacharacters remain literal operands. This requires a POSIX shell
on the remote node; it does not claim support for every SSH server shell.
