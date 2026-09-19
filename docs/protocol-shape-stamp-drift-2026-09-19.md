# Protocol shape stamp — drift inventory, 3.2.78 → 3.2.113 (2026-09-19)

The [11.145] filing demands this inventory before the wire-contract owner
makes the re-stamp call. Measured on lane/integration/11145-stamp-inventory,
worktree of clean main c65ce59f; the numbers below are replayed, not quoted.

## What the gate hashes

`daemon::tests::protocol_shape_stamp_forces_version_bump`
(crates/yggterm-server/src/daemon.rs) extracts the SOURCE TEXT of
`pub enum ServerRequest {` and `pub enum ServerResponse {` from daemon.rs
(`extract_enum_block`, brace-balanced), joins them with `\n`, and FNV-1a-64
hashes the bytes. Coverage boundary: ONLY these two enum blocks in ONLY this
file. Any wire shape living elsewhere (the client_request_envelope struct,
response payloads defined outside the enums) is invisible to this gate.

## The replay instrument

For every commit that touched daemon.rs since the stamp commit, the two enum
blocks were extracted from that commit's blob, hashed exactly as the test does,
and compared to the previous hash. Baseline reproduction: the stamp commit
109029c6 ("[11.75-addendum] … stamps re-cut for 3.2.78") hashes to
0x74843ba79da0f1fa = the stamped constant, and main hashes to
0x2363fb2b0c9e7582 = the computed hash the failing test reports. The
instrument is exact.

## The drift: ONE commit

| hash transition | commit | date | subject |
|---|---|---|---|
| 0x74843ba79da0f1fa → 0x2363fb2b0c9e7582 | 85a5ac5a | 2026-09-10 | fix(cli): forward Codex flags through remote wrapper |

85a5ac5a added ONE field to TWO `ServerRequest` variants — the Codex twins of
the remote daemon-runtime launch contract:

- `EnsureRemoteRuntimeCodexSession` gained
  `#[serde(default)] configured_extra_args: Option<String>` (+ doc comment)
- `StartRemoteRuntimeCodexSession` gained the identical field

`ServerResponse` is byte-identical to the stamp. The AgentSession twins
(`EnsureRemoteRuntimeAgentSession` / `StartRemoteRuntimeAgentSession`)
ALREADY carried the field at the stamp — 85a5ac5a completed an established,
already-stamped wire pattern; it invented no new shape class. No shape drift
before 85a5ac5a; no shape drift after it (stable 0x2363fb2b0c9e7582 for the
nine days 2026-09-10 → 2026-09-19 across seven release bumps).

## serde audit — old-peer safety

Both added fields are `#[serde(default)]` + `Option`:

- OLD daemon reads NEW client bytes: `configured_extra_args` is an unknown
  FIELD on a struct variant of an enum with NO `deny_unknown_fields` (the
  only occurrence of that string in daemon.rs is the doc comment at :4843
  stating exactly this) — safely ignored, never a hard parse failure.
- NEW daemon reads OLD client bytes: field absent → `serde(default)` →
  `None` → launch composes without extra args, byte-identical to pre-drift.

Verdict: the drift is WIRE-COMPATIBLE in both directions. The lost-PTY latch
storm class this stamp exists to prevent (two builds of one version answering
differently in a way peers cannot parse) does NOT apply to this drift — a
3.2.106-pre daemon and a 3.2.106-post daemon parse each other's bytes cleanly
and differ only in whether Codex extra args are forwarded. The damage of the
missed re-stamp is a decorative gate and a lying suite baseline for nine days,
not a compatibility break.

## How it slipped

The obligation belongs to 85a5ac5a (bump 3.2.106 → re-stamp, same commit);
it shipped under an unchanged version. Seven release chores
(3.2.107 → 3.2.113, 2026-09-10/11) bumped Cargo.toml without touching the
stamp — the release chore does not carry the stamp. The gate fires only when
a test suite runs, and ygg-ci is check-only by design, so the red sat for
nine days until the [11.144] seat ran the suites on clean main.

## The re-stamp recipe (the owner's GO, one commit)

- `STAMPED_SHAPE_HASH: u64 = 0x2363fb2b0c9e7582`
- `STAMPED_AT_VERSION: &str = "3.2.113"`
- No Cargo.toml bump required: the shape has been stable since 85a5ac5a and
  every build ≥ 3.2.107 already carries it, so stamping at the shipped
  version records reality without inventing a shape-recognition point. The
  test's own invariant (`stamped <= current`, current = CARGO_PKG_VERSION =
  3.2.113) holds at equality. The re-stamp constants live in the test module,
  not the wire, so the commit does not perturb the hash it stamps.
- The doctrine "bump the workspace version IN THE SAME COMMIT" binds future
  wire changes (the commit that changes the enum), not this repair.
- Alternative if the owner prefers a fresh number: bump to 3.2.114 and stamp
  at it — same hash, one version of headroom; cost is a release number that
  implies a change where none is.
- Stamp comment for the new constants: re-cut for 3.2.113 retroactively
  covering 85a5ac5a (configured_extra_args on the Codex daemon-runtime
  twins, serde(default)+Option, both-direction safe — this document).
