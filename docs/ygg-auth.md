# Local auth vault and rotation

The canonical implementation is
`.agents/skills/yggterm-agent-fleet/ygg-auth.py`. The `ygg auth` command
routes to that helper; its command grammar is maintained separately.
Profiles remain verbatim native records under `~/.yggterm/auth/<harness>/`.
Native caches are activation targets, while the vault retains account
profiles, lineage, cooldowns and failure history. Files are written atomically
with mode 0600 in directories with mode 0700. No new encryption layer is
provided. Keep this directory out of memory synchronization and git.

## Supported operations

`harnesses --json` reports the registered adapter inventory and capabilities.
The current adapters are Codex, Claude, ZCode and AutoClaw. This inventory
does not imply coverage of every CLI. All four support capture, local switch,
rotation, the policy-gated fallback primitive and the preflight exec gate
described below. Device login, OAuth refresh and Codex quota measurement are
supported only by the Codex adapter. Claude OAuth uses a different protocol;
the helper does not send Claude grants to Codex endpoints. Plan records are
switched in their native formats. Host keyring records are not collected or
migrated.

Both harness argument positions work:

```sh
ygg auth --harness codex status --json
ygg auth status --harness codex --json
ygg auth status --harness all --json
ygg auth rotate --harness codex --fast --json
ygg auth policy status --harness codex --json
```

The helper accepts the same arguments directly if the porcelain mapping is
not yet available. `status`, rotation outcomes and JSON errors carry
`schema_version: 1`; existing status and rotation fields remain available.
Nonzero exits remain meaningful: 2 means no eligible rotation candidate,
3 means unavailable native auth, 5 means a protocol/conflict failure, and
6 means invalid usage, policy or unsupported operation.

## Eligibility and quota

The vault filename is the profile slug, including explicitly named profiles.
Active binding is established by matching the native record against vault
records, then by native identity when available. Before replacement, switch
captures the outgoing record, rechecks the native file fingerprint and
verifies the replacement by record digest. An opaque outgoing record that
cannot be bound to a named profile is preserved as `unbound-<digest-prefix>`;
this snapshot preserves the grant without inventing an account identity.

Rotation excludes the active profile, policy-filtered profiles, unreadable
profiles, expired access, recorded auth failures and active cooldowns. It
refuses when every candidate is excluded, rather than silently reactivating
an expired or cooling profile. Counts and per-profile reasons distinguish an
empty vault from policy filtering and failed auth. Rotation never refreshes
a grant or consumes a banked quota reset.

Measured Codex candidates rank by the greatest headroom in their tightest
existing quota window. For example, 10% used ranks ahead of 80% used. A
missing secondary window does not penalize a valid primary window. Missing,
invalid or unavailable meters mean **unknown**, not full, free or zero.
Measured available capacity ranks ahead of unknown capacity; measured
exhaustion is excluded. HTTP 401/403 from the usage endpoint records an auth
failure and excludes that record. `--fast` skips all usage requests; other
adapters never use the Codex usage endpoint.

Status reports these separately:

- `active_binding` and `active_record_hash`: the local native cache binding.
- `stored_freshness`: `clock_fresh`, `expired`, `unknown` or `unreadable`.
- `auth_health`: recorded typed failure and reauthorization requirement,
  bound to the exact record digest, with prior failure history retained.
- `native_health`: `unknown`; cache activation is not a native health test.
- `quota`: unknown unless separately measured; cooldown is independent.

A rejected Codex refresh retains a bounded reason such as `refresh_invalid`,
without response bodies or tokens. The failed record stays in the vault and
is excluded until a changed grant is captured or refreshed. A changed record
invalidates its old failure verdict; it does not prove native health.
Refresh remains subject to the existing single-writer guard, including the
live-record path. Capture-on-leave and provenance remain local operations.

## Host-local policy

Absent policy preserves legacy deterministic assignment eligibility and
disables automatic fallback. An operator may install the following invented
example at `~/.yggterm/auth/codex/.policy.json`:

```json
{
  "version": 1,
  "mode": "local-pool",
  "host": "test-host",
  "profiles": ["example-primary", "example-spare"],
  "source_authority": "owner:example-authorization",
  "automatic_fallback": true
}
```

Every field is required. Unknown fields, versions, modes, wrong host,
noncanonical slugs, an empty pool, missing authority and nonboolean fallback
settings fail closed. `mode: assignment` applies the legacy home table;
`local-pool` permits only the explicitly named profiles already in this
host's vault, including profiles whose home assignment is elsewhere. It does
not transfer grants, change assignments or claim independent quota.

`source_authority` records the operator's authorization reference, not a
cryptographic assertion or permission grant. The policy file is private
operator configuration, protected by the local account/filesystem authority.
The helper only inspects it; policy installation and activation must follow
the owning operator's authorization. No policy is enabled by installing this
code. Existing fetch/export verbs remain explicit operations; neither
rotation nor fallback invokes them.

## Bounded automatic fallback contract

A managed consumer may call the helper once after observing a typed error:

```sh
ygg auth fallback --harness codex --error-code auth_invalid \
  --event-id example-event --expected-active-hash DIGEST --json
```

The only accepted error types are `auth_invalid`, `token_expired` and
`rate_limited`. The consumer must bind the event to the active record digest
it observed. Free-form stderr, arbitrary process failures and ambiguous
side effects are not classified by this helper. The caller must obtain the
digest before the request and avoid placing credentials in event identifiers.

Under the vault lock, fallback validates policy and binding, records the
attempt durably, captures the outgoing grant, records failure or cooldown,
selects an eligible local profile and activates it. It performs no network
requests. Existing longer cooldowns and failure history are preserved. It
never redeems reset credits or replays the failed operation.

The same event and request return their stored result without another
activation. Reusing an event ID with a different request is rejected. An
interrupted or unsuccessful attempted event remains held for inspection;
retrying it cannot silently activate another account. A new consumer event
requires a new observation and binding, not a blind retry loop.

Successful activation returns `outcome: activated`, `actual_effect: unknown`,
`native_health: unknown`, `session_action: resume_or_restart_required` and
`replay_authorized: false`. Running Codex sessions retain auth loaded at
startup; changing the active cache does not repair an already running turn.

This component does **not** install managed-prefix hooks, detect typed native
errors inside every CLI, restart live sessions, prove native account health,
collect every harness credential format, or complete the system-wide auth
goal. Consumer integration and native-effect evidence remain release criteria
for the parent worker. An auth activation is not worker completion.

## Preflight exec and the native launch primitive

`exec` is the pre-launch consumer: prove the bound grant, then hand the
terminal to the native CLI. It never repairs or reloads a running process.

```sh
# bounded read: activation receipt, no spawn, no mutation, no network
ygg auth exec --harness codex --preflight-only --json

# launch: preflight, then replace this process with the native argv
ygg auth exec --harness codex -- codex exec --skip-git-repo-check - | cat

# adapter-observed typed failure: ONE policy-authorized fallback, CAS-bound
ygg auth exec --harness codex --typed-failure rate_limited \
  --event-id example-event --observed-active-hash DIGEST -- codex
```

The preflight gate fails closed before any native process starts. The live
record must be usable, digest-bound to a vault profile (an unproven grant is
refused, not guessed), and free of recorded auth failures. Expiry alone is
not proof of a dead grant: an expired but refreshable Codex grant takes
exactly ONE canonical refresh under the single-writer lineage law — a
foreign writer's grant is held, never copied or refreshed here. Adapters
without a refresher refuse with `reauthorization_required` and never touch
Codex endpoints. `--measure` is the receipt's only optional network touch
(one usage read, Codex only); without it the meter reports `unknown`, never
free. The ready receipt binds harness, provider, the proven record digest
and the meter, and reports `native_health: unknown`,
`actual_effect: unknown` and `session_action:
child_native_auth_proof_required` — a cache activation is not native health.

The launch itself is an `exec`-family replacement at the same pid: the child
inherits the activated native cache, and its args, signals and exit code are
the native CLI's own (including nonzero exits and argv containing spaces or
special characters). A concurrent rewrite of the native auth file between
verification and launch aborts with `concurrent_change` instead of spawning
against unverified auth. There is no restart and no replay: a nonzero child
exit is observed once, never retried — a new launch is a fresh parent
decision, and a new fallback needs a new typed observation with a fresh
event id and the digest it was bound to. With `--json`, one receipt line is
written to stderr before the replacement; without flags the launch is
transparent. Free-form stderr is never classified by this helper.

Exit codes: `0` ready or the child's own code, `2` no eligible fallback
target, `3` no live auth file, `5` an unproven grant, health failure,
foreign-writer hold, refresh rejection or concurrent change, `6` invalid
usage, malformed policy, flags mismatch, or a native binary that could not
start.

## Daemon launch wiring (`managed_auth`)

The daemon's managed-CLI composer (`crates/yggterm-server/src/managed_auth.rs`)
wires this seam for codex, codex-litellm, claude and zcode launches. The gate
lives in two settings keys and defaults to OFF: `managed_auth_mode`
(`off` | `observe` | `enforce`) and `managed_auth_script` (explicit
ygg-auth.py path; empty ⇒ the dispersed `~/.yggterm/bin/ygg/ygg-auth.py`).

- **off (default):** launch composition is byte-identical to the pre-auth
  law. A host that never configures the gate launches exactly as before.
- **observe:** the bounded preflight (`--preflight-only --json`, no
  `--measure`) runs before launch and ONE `auth_preflight` trace event
  records the receipt's outcome (ready/blocked/unavailable, digest-grade
  facts only). The launch itself is untouched.
- **enforce:** the preflight must say `ready`. A ready launch composes
  through the seam (`exec python3 <script> exec --harness <H> --json --
  <invocation>`, same-pid replacement, receipt one line on stderr).
  Anything else composes a VISIBLE refusal line — the reason on stderr,
  exit 86 — and launches nothing.

⛔ The refusal is a composed shell line, not a composer error, on purpose:
the composer's error channel is the layout-resolution fallback, and every
call site degrades an error to the legacy unauthenticated command. An error
refusal would therefore downgrade to exactly the launch enforce exists to
prevent. A CLI with no auth plane (pi, opencode, qwen, kimi, muse,
antigravity, grok, devin, mimo) is never gated in any mode. A configured
script path that does not exist is a refusal under enforce and
`unavailable` under observe — never a guessed fallback script. The daemon
never activates a policy, refreshes a grant, or mutates a live account:
this wiring is a launch gate over the seam's own verbs, nothing more.

## Offline verification

Run the focused suites with bytecode disabled and disk-backed scratch:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 .agents/skills/yggterm-agent-fleet/tests/test_ygg_auth_policy.py
PYTHONDONTWRITEBYTECODE=1 python3 .agents/skills/yggterm-agent-fleet/tests/test_ygg_auth_rotation.py
PYTHONDONTWRITEBYTECODE=1 python3 .agents/skills/yggterm-agent-fleet/tests/test_ygg_auth_exec.py
```

The policy suite redirects every auth path to isolated fixtures and stubs
network/refresh calls. The legacy rotation screens use isolated HOME and
vault paths, disable pre-switch refresh and use fast rotation. Neither suite
activates or refreshes the operator's auth. Fixture scratch defaults to
`~/.yggterm/scratchpad/auth-tests`; `YGG_AUTH_TEST_SCRATCH` may override it.
