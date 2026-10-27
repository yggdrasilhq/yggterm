## 1. DECISIONS

**Certified design verdict: ACCEPT the design below for implementation.** The certificate covers the schema and behavioral contract. Source conformance remains unverified because the required memory-sync gate failed twice with shared-lock contention; this verdict uses the supplied rev-4 facts.

**D1 — ACCEPT: attach pressure to execution profiles, with shared physical limits.** `sol-high` and `sol-medium` are execution profiles referencing the **same** admitted `sol-editor` binding. Profiles may override reasoning; they cannot change model, harness, account, plan, capabilities, or admission status. Quantify desired concurrency per profile; enforce aggregate ceilings per binding, capacity pool, role, and fleet. Planes continue supplying health and budget constraints. Separate reasoning profiles must never create separate physical capacity allowances.

**D2 — ACCEPT: use `concurrent_slots` as the primary unit.** One slot means one launched, nonterminal CLI process, including a process whose outcome is uncertain. Add a numeric refusal cooldown in seconds. Keep tokens and consults in existing budgets. Do not introduce a second dispatches-per-window meter: the consult budget already counts CLI launches. Queue depth is observed demand, not spendable capacity. A slot is permission to run concurrently, not a quota reset or entitlement to provider capacity.

**D3 — ACCEPT: named levels must expand into numbers.** Define owner-controlled numeric tables for `normal`, `high`, and `saturate`. The persistent demand selects a level and may override individual profile targets with raw numbers. Overrides **replace** level values; they do not add to them. Targets and leases remain bounded by owner ceilings. The owner can edit either the baseline demand or the ceilings; lowering a baseline invalidates existing leases, while lowering ceilings also prevents subsequent delegated increases.

**D4 — ACCEPT: role-owned sequential chains plus weighted parallel lanes.** Each role specifies a preferred profile, an ordered fallback chain, and either single-profile or weighted parallel dispatch. Parallel lanes distribute **independent tasks**; they never automatically duplicate one task. An unavailable lane uses the role’s explicit fallback chain, subject to capabilities, restrictions, admission, health, and budget checks. Quota refusal affecting the shared sol capacity pool blocks both sol profiles. Reroute eligible work, hold the remainder, and preserve uncertain submissions for reconciliation.

**D5 — ACCEPT: dual-rate control, with bounded persistent proposals too.** Owner policy contains ceilings, level definitions, routing, and delegation limits. Agents may submit persistent ledger proposals changing only delegated demand fields. Frequent changes use receipted runtime leases with a maximum TTL of 900 seconds. Neither mechanism changes budgets. Raising a concurrency target within an owner ceiling is a pressure adjustment; raising a token/consult hard limit, spendable allowance, or usable maintenance reserve is a budget increase. The deterministic supervisor may reduce availability and enforce expiry, but cannot originate pressure increases or budget increases.

**D6 — REJECT + CORRECTION: resolution cannot be a sequence of overriding knobs.** Resolve the baseline and valid leases, then intersect that demand with **all** capacity, health, routing, and budget constraints. A single authoritative admission service performs the final check and reservation atomically. Dream orchestrators plan against its view; executor adapters and queue-serve v2 must obtain its launch permit. No local controller may manufacture an allowance from its own config snapshot.

**D7 — ACCEPT: preserve v2 behavior; make v3 adoption explicit.** Existing v2 documents retain their bytes, semantic hashes, limits, and routing behavior. Compile them into the admission service’s internal representation without rewriting their ledger records. In v3, accept either legacy `supply.admission` or new `supply.pressure`, never both. The worked adoption replaces legacy admission in an owner-class proposal. It also explicitly raises the consult budget; adding slots while retaining 40 consults/day would not accomplish the directive.

## 2. The v3 schema

This is an **additive section-level example**, not a replacement for the entire live document. Preserve existing binding records, role kinds, scenario policies, health contracts, budget scopes, spendable flags, and maintenance reserves.

Every binding has a default execution profile. An explicit profile marked `default_for_binding: true` supplies it; otherwise normalization synthesizes `<binding-id>/default`, inheriting the binding’s execution settings. Synthesized profiles do not change admission status.

```yaml
schema_version: 3

catalog:
  execution_profiles:
    sol-high:
      binding_ref: sol-editor
      default_for_binding: true
      reasoning_override: high

    sol-medium:
      binding_ref: sol-editor
      default_for_binding: false
      reasoning_override: medium

    gemini-high:
      binding_ref: gemini-reviewer
      default_for_binding: true
      reasoning_override: high

    glm-max:
      binding_ref: glm-workhorse
      default_for_binding: true
      reasoning_override: max

roles:
  editor:
    default_binding_ref: sol-editor
    fallback_binding_refs:
      - kimi-editor
    routing:
      preferred_profile_ref: sol-high
      fallback_profile_refs:
        - kimi-editor/default
      max_concurrent_slots: 16
      parallel:
        strategy: single
        lanes: []

  intelligence:
    default_binding_ref: sol-editor
    fallback_binding_refs:
      - gemini-reviewer
    binding_refs:
      - sol-editor
      - gemini-reviewer
    routing:
      preferred_profile_ref: sol-high
      fallback_profile_refs:
        - gemini-high
      max_concurrent_slots: 96
      parallel:
        strategy: weighted_fair
        lanes:
          - profile_ref: sol-high
            weight: 1
          - profile_ref: sol-medium
            weight: 2

supply:
  pressure:
    ceilings:
      global_concurrent_slots: 128

      default_binding_concurrent_slots: 1
      binding_concurrent_slots:
        sol-editor: 96
        gemini-reviewer: 4
        glm-workhorse: 8

      default_profile_concurrent_slots: 1
      profile_concurrent_slots:
        sol-high: 32
        sol-medium: 64
        gemini-high: 4
        glm-max: 8

      pools:
        codex-sol:
          binding_refs:
            - sol-editor
          concurrent_slots: 96
          cooldown_after_refusal_seconds: 60

        gemini-consult:
          binding_refs:
            - gemini-reviewer
          concurrent_slots: 4
          cooldown_after_refusal_seconds: 60

        zai-shared:
          binding_refs:
            - glm-workhorse
            - glm-offpeak
          concurrent_slots: 8
          cooldown_after_refusal_seconds: 60

    levels:
      normal:
        default_profile_slots: 1
        profile_slots:
          sol-high: 2
          sol-medium: 4
          gemini-high: 1
          glm-max: 2

      high:
        default_profile_slots: 1
        profile_slots:
          sol-high: 16
          sol-medium: 32
          gemini-high: 2
          glm-max: 6

      saturate:
        default_profile_slots: 1
        profile_slots:
          sol-high: 32
          sol-medium: 64
          gemini-high: 4
          glm-max: 8

    demand:
      level: high
      profile_slot_overrides: {}

    control:
      persistent_agent_edits: true
      agent_role_refs:
        - editor
        - intelligence
        - dreamers
        - core-keeper
      agent_profile_refs:
        - sol-high
        - sol-medium
        - gemini-high
        - glm-max
      agent_level_names:
        - normal
        - high
        - saturate
      lease_max_ttl_seconds: 900
      lease_max_active_per_issuer: 1
```

The same owner-class adoption changes these fields on the **existing** `b-consult-daily` budget:

```yaml
hard_limits:
  consults: 4096
window_hours: 24
```

Remove `supply.admission` in that adoption. Preserve the budget’s existing identity, scope, spendable status, accounting history, and reserve protection. Migration must not reset its current-window consumption.

The opening sol target is **48 concurrent processes: 16 high + 32 medium**, shared across roles and orchestrators. `saturate` permits **96: 32 high + 64 medium**. These are respectively 8× and 16× the previous *entire fleet* concurrency limit of six. The new fleet ceiling is 128; the consult allowance is 102.4× the previous daily allowance.

Those numbers implement the owner’s instruction to use substantially more capacity. His reset-frequency note supports abandoning the previous scarcity posture; it does **not** establish measured throughput. These are bounded policy allowances, still subject to fresh health, actual refusals, and budget headroom.

Additional exact semantics:

- Profile targets count globally. Editor and intelligence do not each receive another 16 sol-high slots.
- Pool membership is cumulative: a launch must satisfy every applicable pool.
- Unlisted bindings/profiles use the explicit numeric defaults. Declared bindings remain ineligible.
- `glm-max` inherits the existing golden binding; no zcode-tui model, harness, account, transport, or reasoning binding is rewritten.
- `kimi-editor/default` remains a fallback reference. Its declared status prevents execution until separately admitted and eligible.
- Legacy role fields, when present beside `routing`, must agree with its preferred/fallback binding projection. They are compatibility fields, not independent routing authorities.
- Weighted dispatch uses deterministic weighted round-robin among eligible lanes, respecting their remaining targets. For comparable tasks, the intelligence mix is approximately one high to two medium.
- Falling back never implicitly changes an editor task into a medium-reasoning task.

## 3. Validator + authorization rules

### Closed-schema validation

Every object shown above has exactly the listed keys. ID-keyed maps accept only valid, resolvable IDs; they are not arbitrary extension points. Missing required keys, duplicate YAML keys, unknown fields, and invalid types fault.

| Fault | Required invariant |
|---|---|
| `PRESSURE_SCHEMA` | Reject unknown/missing fields, duplicate keys, invalid enums, booleans used as integers, floats, and negative counts. |
| `PRESSURE_REF` | Resolve every binding, profile, role, level, and pool member. Reject duplicate lane/profile references. |
| `PRESSURE_PROFILE` | At most one explicit default per binding. Only reasoning may be overridden; the adapter must support that override. |
| `PRESSURE_BOUNDS` | Each target is within its profile, binding, applicable pool, and global ceiling individually. Weights are positive integers. |
| `PRESSURE_SOURCE_CONFLICT` | Reject simultaneous `supply.admission` and `supply.pressure`. |
| `PRESSURE_ROUTE_CONFLICT` | Reject disagreement between canonical routing and retained legacy projections. |
| `PRESSURE_DELEGATION` | Delegated roles, profiles, and levels must resolve; level changes cannot alter undelegated profile targets. |
| `PRESSURE_BELOW_INFLIGHT` | Do not activate a hard ceiling below currently counted occupancy. |

Profile targets may collectively oversubscribe a shared pool: they are maximum demands, not reserved allocations. The admission service enforces the aggregate ceiling.

No new field relaxes binding admission, capabilities, task restrictions, plane policy, editor independence, or campaign permission gates.

### Persistent authorization

**Owner-only:**

- `catalog.execution_profiles`
- Existing binding admission and identity fields
- `roles.*.routing` and corresponding compatibility projections
- `supply.pressure.ceilings`
- `supply.pressure.levels`
- `supply.pressure.control`
- Budget limits, reserve policy, and budget spendability
- Migration from legacy admission to pressure

**Agent-tunable through ledger proposals:**

- `supply.pressure.demand.level`
- `supply.pressure.demand.profile_slot_overrides.<delegated-profile>`

Agent writes require `persistent_agent_edits: true`, an authenticated eligible role, an allowed level/profile, and a valid resulting policy. Deleting an override also requires authorization and validation of the resulting target.

Retain existing `rejected_unauthorized` and `STALE_PARENT` behavior. Perform authorization over the complete semantic diff, including parent-object replacement, deletion, and reference changes. Agents retain their existing permission to add **declared** bindings; that grants no profile admission or capacity.

The owner can disable persistent agent edits independently of lease delegation. To impose a durable hard limit, the owner changes ceilings.

### Runtime leases

Authoritative state lives outside the semantic-hashed config:

```text
~/.yggterm/swarm/runtime/pressure/state.sqlite
```

Use one configured admission authority, initially on dev. Its transactional store contains leases, launch reservations, accounting references, and append-only events. Other hosts call that authority; they do not maintain independent spendable replicas.

A lease request contains exactly:

```yaml
request_id: "<unique-id>"
issuer_session_ref: "<authenticated-session>"
issuer_role_ref: intelligence
expected_config_hash: "<current-semantic-hash>"
expected_lease_revision: 0
ttl_seconds: 900
profile_slots:
  sol-high: 32
  sol-medium: 64
reason: "Drain eligible intelligence backlog"
```

Rules:

- Authenticate issuer identity and role; do not trust those strings alone.
- Requests name **absolute targets**, never extra slots.
- For each profile, combine valid leases using `max`, not addition.
- Each issuer has at most one active lease; replacement requires lease-revision CAS.
- Reject requests exceeding delegation, TTL, or ceilings. Do not silently clip them.
- Every mutation produces an authority-authenticated receipt binding request bytes, config hash, issuer, resolved targets, timestamps, and outcome.
- A config activation invalidates all previous-config leases and unredeemed launch permits.
- Expiry returns demand to the persistent baseline. It does **not** release reservations for running or uncertain processes.
- On authority restart, invalidate leases and reconcile outstanding reservations before issuing new permits. Unknown process state remains counted.
- Missing/corrupt committed state or unavailable accounting causes containment/hold, never reconstruction of fresh capacity from config defaults.
- Existing lease expiry and safety reductions are deterministic maintenance. Creating an increased target requires an authorized agent or owner request.

To lower a hard ceiling below occupancy, stage a drain request that blocks affected starts. Activate the new config only once occupancy fits. Preserve the old ceiling until then. Do not kill live work or activate an immediately violated ceiling.

### Admission function

The shared contract is:

```text
pressure_admit(
    dispatch_id,
    attempt_id,
    expected_config_hash,
    role_ref,
    profile_ref,
    plane_ref,
    host_ref,
    budget_request
) -> {
    decision: ALLOW | HOLD | REROUTE | CONTAIN,
    permit_ref,
    receipt_ref,
    reason_codes,
    not_before
}
```

For profile `p`:

```text
baseline(p) =
    demand override, if present;
    otherwise selected level's profile value;
    otherwise selected level's numeric default

desired(p) =
    max(baseline(p), valid lease targets for p)
```

A candidate launch is allowed only when:

1. The current config, issuer authority, role routing, binding admission, capabilities, restrictions, and plane policy permit it.
2. Its profile occupancy remains within `desired(p)`.
3. Its profile, binding, pool, role, and global occupancies remain within every applicable ceiling.
4. Fresh health permits the launch, with any numeric health restriction further reducing capacity.
5. Every applicable budget can reserve the launch cost without spending protected reserves.

These are intersecting predicates—not values that overwrite one another. Reserve capacity and budget before issuing a permit. Any multi-store implementation must fail closed across partial commits.

Permits are single-use, bound to the current config and exact attempt, and expire unredeemed after 30 seconds. The executor redeems immediately before launch under the authority’s fence. An uncertain redemption or launch keeps its reservation until reconciled.

All managed dispatch paths must use this gate, including headless consults. Planning snapshots and queue tickets are insufficient authorization.

Quota shock handling is pool-scoped when evidence identifies shared quota. Respect a verified retry/reset time, otherwise apply the configured cooldown and require fresh health. Reroute only after a definite nonlaunch/failure or safely reconciled attempt. Never replay `SUBMISSION_UNKNOWN`, consume banked resets, or equate process exit with accepted work.

## 4. Migration + falsifiable claims

Validate a v2 document and its original integrity hash **before** normalization. Preserve its admission limits and existing chains exactly. Do not materialize synthetic profiles into its ledger document or automatically raise its budget.

V3 accepts legacy admission as a deprecated alternative. Without explicit routing, use the existing role chain through default profiles. Publishing the worked pressure policy is an explicit owner-class, parent-hash-CAS transition from rev 4.

The implementation certificate requires these additional claims:

| Claim | Falsifiable assertion |
|---|---|
| **C13 — Closed schema** | Unknown fields, duplicate keys, invalid references, invalid numeric types, and unsupported reasoning overrides prevent activation. |
| **C14 — V2 preservation** | Existing fixtures and the rev-4 document retain their hashes and admission/routing behavior through normalization. |
| **C15 — One pressure source** | A document containing both admission sections faults; neither silently wins. |
| **C16 — Shared sol identity** | High and medium launch with distinct reasoning settings but identical underlying binding identity; their reservations share the sol binding/pool counts. |
| **C17 — Concrete opening** | The worked config resolves to 16 high + 32 medium; saturation resolves to 32 + 64. No old global-six limiter remains active. |
| **C18 — Aggregate ceilings** | Racing controllers across hosts cannot exceed profile, binding, pool, role, or global limits. Repeated leases do not multiply capacity. |
| **C19 — Budget remains binding** | Exhausted consult/token budgets deny further launches despite available slots. Migration preserves spending; maintenance reserves remain protected. |
| **C20 — Agent boundaries** | Agents can change only delegated demand. Attempts to raise ceilings/budgets, alter routes/profiles, or bypass checks through object replacement are rejected. |
| **C21 — Lease lifecycle** | TTL, revision CAS, idempotency, expiry, and receipt checks work; expiry never frees a live or uncertain process reservation. |
| **C22 — Owner fencing** | Owner activation invalidates old leases/permits. A ceiling below occupancy cannot activate until drained. |
| **C23 — Health and shock** | Stale health holds dispatch; shared sol quota refusal blocks both reasoning profiles. Cooldown cannot reset provider quota. |
| **C24 — Routing semantics** | Weighted lanes distribute independent tasks; fallback order is stable; declared kimi cannot launch; high-only editor work cannot silently become medium. |
| **C25 — Uncertain submission** | Lost launch acknowledgements retain reservations and prohibit duplicate replay until reconciled. |
| **C26 — Recovery** | Restart, unavailable authority, corrupt state, and partial accounting commits cannot create capacity or budget headroom. |
| **C27 — Complete consumption** | Dream, executor, queue-serve v2, and consult launch paths all require a current, single-use admission permit. |
| **C28 — Golden binding preservation** | The migration leaves existing zcode-tui binding identity and execution settings unchanged. |
| **C29 — Completion independence** | Higher pressure does not waive session-review provenance, subject-digest binding, or independent editor acceptance. |

