# Python operating components during the 4.0 transition

This source increment publishes the existing Python entrypoint, its shared
command catalog, local root-location and steering tools, and host-local auth
protocol. It does not release yggterm 4.0 or certify the distributed correction
loop. Native CLI installers and optional GUI packages have separate gates.

The command catalog lives in `tools/swarmharness/introspection/commands.json`.
The fleet component path is a source alias to that document. Dispersal flattens
source aliases into standalone installed files and checks the exact catalog
bytes. Local next-word help, examples and completion remain available when an
operational backend is unavailable. Planned families refuse effects explicitly;
missing inspection adapters report unknown coverage and exit 3. The bounded
observation adapters are now published separately in this repository and have
been qualified through installed POSIX entrypoints. They expose local source
records; their availability does not establish complete fleet coverage.

Start an inspection with `ygg dream ls --all`, `ygg service ps --host all`,
`ygg swarm heartbeat --layers`, `ygg swarm pressure --by plan --explain`, or
`ygg schedule ls`. These print human-readable views; add `--json` for a
structured result. Read the coverage alongside the rows. An unavailable,
stale or unknown source can return exit 3 with useful partial records.
`--host all` selects hosts represented by the captured frame; it does not
prove that every fleet host was enrolled or contacted. Native plan units,
pressure calibration, conditional intentions and exact GUI session attachment
have separate remaining qualification gates. `dream open` cannot certify an
attachment by printing a session identifier.

During the current rollout, an accepted observation package is installed as
an immutable generation under the logical root and selected through
`swarmkernel/swarmharness`. Existing flat kernel files retain their owners.
This deployment is additive: it neither activates new configuration nor
repairs the running Composer miner. Packaging this reader into every fresh
native installation remains a release obligation.

`ygg fs status`, `init`, `where` and `gc plan` expose the accepted local root
contract. Initialization supports the alias-first transition from the legacy
root without moving live state. The collector retains everything until an
owner adapter establishes eligibility and fences; `gc apply` remains held.
Physical root relocation, app-subtree conversion and distributed GC are open.

`ygg steer init`, `plan`, `check` and `stamp` expose generic local seeds and
reviewable projections. Initialization does not stamp harness instructions.
Private policies remain private. The accepted compiler is included as data;
legacy private-source import, fleet authority cutover and deletion are separate
unfinished transitions. Do not retire an existing authority from this increment.

`ygg auth` provides readable inventory with explicit JSON, policy-gated local
activation and bounded native launch/preflight. Policies default off. A read-only
preflight neither refreshes nor activates a grant; format or vault binding does
not establish provider health. Recorded failures, malformed policy/lineage and
unusable rebound grants hold launch. The protocol never automatically consumes
banked resets or replays a failed native request. Rust daemon launch integration
and live native/provider qualification remain separate obligations.

The memory component keeps its native owner-managed entrypoint authoritative.
Installing this payload does not arm dreaming, alter active authentication,
restart desktop sessions or establish fleet health. See the filesystem,
steering, verb-dispersal and session/role-ledger specs for the remaining gates.
