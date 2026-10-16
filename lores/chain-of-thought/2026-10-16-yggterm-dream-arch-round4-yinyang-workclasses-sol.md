**Accept A, B, and C. Extend round 3 with one execution plane, a connection lifecycle, and an owner-controlled scheduling overlay.** Keep the typed journal, lobe adapters, cost gate, and forwarding queue.

Three architectural rules govern the revision:

- **Connection type describes meaning; work class grants permission; focus assigns priority.** None substitutes for another.
- **Elapsed time never expands authority.** Weekly dreaming gets a wider bounded search window, not stronger write privileges.
- **“Done” requires an observed effect and a receipt.** A model answer, successful command, or proposed connection is insufficient.

The schemas, commands, and numerical defaults below are recommendations for implementation.

### Q1 — Yin/Yang mechanics

A **connection** is a typed, directed relationship between two resolvable objects, with provenance and an explicit epistemic status.

Use the existing address systems through adapters:

| Surface | Native connection | Normalized interpretation |
|---|---|---|
| Lobe records | Existing `lobe://` grammar | Preserve the parser’s address and record identity |
| Hub/vault | `[[name]]` | Resolve within namespace to a stable object; ambiguous names remain unresolved |
| Boards | ACK reference/thread | Relationship between immutable posts; distinguish reply, support, correction, and fulfillment |

**Do not invent a replacement URI grammar.** The normalized edge wraps native references. The journal records edge creation and transitions; the connection index remains rebuildable.

Minimum edge record:

```text
id, from, relation, to
status: observed | owner-asserted | hypothesis | supported | rejected | expired
source_refs[], source_revisions[], created_by
hypothesis?, falsification_test?, test_by?
supersedes?, work_receipt?
```

A board reply proves that someone replied. It does not establish agreement, factual support, completion, or owner authorization. Likewise, several summaries repeating one source are one evidentiary lineage.

**Yin operations**

- Resolve unambiguous references and repair derived backlinks.
- Deduplicate identical derived edges while retaining their provenance.
- Strengthen support when genuinely additional evidence appears.
- Mark broken, contradicted, or superseded connections.
- Remove expired edges from active derived views while retaining their history.
- Fulfill an existing connection request when its explicit completion predicate is satisfied.

Yin may merge **derived representations**. Merging source records, identities, cases, or notes requires the relevant work-class permission. “Prune dead weight” never licenses deleting source evidence or silently discarding owner statements.

**Yang operations**

- Propose a new cross-lobe relationship.
- Explore a bounded neighborhood around a question, anomaly, or missing connection.
- Generate and test a candidate pattern.
- Turn a supported hypothesis into a persistent derived connection where the lobe contract permits it.

Yang may **write exploratory edges autonomously**, rather than merely putting every idea in the owner’s queue. Those edges must remain visibly hypothetical. Promotion into a canonical source record uses the relevant work class and its verification bar.

Every Yang edge must include:

1. A concrete hypothesis and supporting source pointers.
2. Why the connection would improve retrieval or enable a named task.
3. An observable falsification test.
4. A review deadline and bounded investigation budget.

“Possibly related” fails admission. So does a hypothesis whose only test is asking another model whether it sounds plausible.

**Cadence and authority**

| Stratum | Default work | Permitted writes |
|---|---|---|
| Hours, after quiescence | Yin over changed objects; fulfill ready connection requests | Derived indexes, edge transitions, authorized source repairs |
| Daily | Bounded Yang investigations and due hypothesis tests | Labeled hypothesis edges and verified class outputs |
| Weekly | Wider neighborhood sampling; evaluate usefulness and recurring patterns | Same permissions; archive expired hypotheses from active views |

Start Yang with **five investigations/day, twenty candidate endpoints/investigation, two retrieval expansions, and at most two admitted edges/investigation**. These are ceilings, never output targets. Untested hypotheses expire from ordinary traversal after seven days; unchanged inputs cannot repeatedly regenerate a rejected edge.

Ordinary retrieval excludes hypothesis edges unless exploration is requested. Even then, hypothetical status must survive summaries and exports.

**Minimal falsifiable slice:** use one hub document, one board thread, and two lobe records. Repair one unambiguous broken connection; create one testable Yang edge. Replay ordinary retrieval and exploratory retrieval. Fail if the hypothesis appears as fact, a source is overwritten, an ambiguous link resolves silently, or rerunning creates duplicates.

### Q2 — The work-class registry

Implement an **owner-declared capability registry**, with enforcement outside the model.

```text
class_id, version, owner_authorization_ref
lobe_law_refs_and_revisions[]
enabled, executor_allowlist
read_scope, write_scope, allowed_operations, forbidden_operations
preconditions, resource_limits, maintenance_constraints
per_item_budget, daily_envelope, external_spend_envelope
verification_contract, completion_predicate
retry_limit, checkpoint_policy, compensation_policy
forward_conditions
```

The registry grants only explicitly listed operations. Lobe law can constrain them further. Missing or contradictory authority produces a narrow forwarding item; it does not invite the executor to interpret itself into broader permission.

Use this lifecycle:

```text
proposed → admitted → running → verifying → done
                         ↘ deferred | forwarded | failed
```

**Admission flow**

1. Resolve the work class, owner authorization, current lobe law, and source revisions.
2. Check scope, required inputs, corrections, executor availability, and resource contention.
3. Estimate the whole item: retrieval, execution, consultation, verification, and permitted retries.
4. Atomically reserve its token, cash, and class-specific external resources.
5. Execute through scoped adapters with an idempotency key and checkpoint.
6. Verify the actual output through an independent read path.
7. Commit a receipt containing source/output revisions, observed effects, verification results, and actual expenditure.

Recheck authorization and dependencies immediately before writes. A changed law, revoked authorization, or changed input invalidates the pending write.

All continuations and retries count toward the same **16k input / 2k generated item ceiling**. Splitting one task into children cannot multiply its allowance.

**Initial six entries**

The token figures below are suggested **per-item maxima**, including verification.

| Class | Autonomous authority and constraints | What counts as done | Item ceiling |
|---|---|---|---|
| **Callgraph** | Run approved decode/transcription recipes, TC windows, indexing, identity banking, and graph fitting. Honor campaign locks, fresh window probes, quality gates, reserves, and candidate-evidence boundaries. No new engine purchase or arbitrary identity merge. | Selected inputs have terminal dispositions; output is banked canonically, retrievable through the normal query path, and provenance resolves to recordings/results. Replay introduces no duplicates. A window stopping at its wall may complete the window item while leaving backlog open. | 8k / 0.6k |
| **Dossiergraph** | Intake, hash, index, cross-reference, conduct bounded due diligence, and prepare evidence-backed case updates. Keep original artefacts and distinguish testimony, generated interpretation, verified facts, allegations, and adjudications. Import any narrower promotion or outward-action gate from current lobe law. | Hash/provenance checks pass; archive retrieval finds the artefact; narrative claims resolve to appropriate sources; unsupported material retains its label and cannot enter a verified bundle. | 12k / 1.2k |
| **Infra updates** | Execute established update recipes on an explicit fleet target allowlist, under current runbook preconditions. Permit package/service updates only where the manifest defines acceptable restarts, outage limits, and recovery. Wider infrastructure is a separate scope. | Installed/running versions are read back, required service and route checks pass, prior container/service state is restored where required, and the run log records remaining work. A reboot requirement remains explicit; no autonomous jojo reboot. | 4k / 0.4k |
| **Legacy notes → Obsidian++** | Convert mechanical batches through the approved emd-renderer path. Write destination files and a conversion manifest; preserve originals. No semantic rewriting or invention of missing material. | Parse succeeds; normalized content, structure, links, embeds, attachments, and identifiers reconcile; representative complex constructs render correctly. Unsupported constructs are quarantined and reported, never silently dropped. | 8k / 0.8k |
| **Learning linkages** | Execute Yin/Yang connections across authorized learning records, including backlinks and hypothesis edges. Do not alter learner achievements, answers, mastery judgments, or owner conclusions merely to make a pattern fit. | Links resolve; hypothesis labels survive retrieval; the named retrieval/task replay demonstrates the intended connection. Rejected and expired edges stay excluded appropriately. | 4k / 0.4k |
| **Lobemanager consultation + docs** | Run bounded consultations on permitted models; write source-grounded operational documentation and examples. Preserve consulting records. Docs cannot amend law, and consultation cannot authorize its own implementation. | Claims trace to inspected source revisions or explicit proposals; command examples replay successfully; an intended reader can complete the documented task. Unsupported claims are removed or labeled. | 12k / 1.2k |

The learning class consumes the **Yin/Yang allocations**, avoiding double accounting.

For infra, the manifest must separate **observing**, **updating**, **restarting**, and **rebooting**. “Infra update” should produce real updates within its contract; it should not become either unrestricted root access or perpetual proposal generation.

For superior consultation, preserve **zero unattended sol-HIGH**. The class may use permitted cheaper consultation or reuse recorded verdicts. If the required quality genuinely needs sol-HIGH, stage the brief and forward that specific need. Naming consultation as an autonomous class does not remove the accepted model restriction.

**Always outside autonomous authority:** fingraph trades, medgraph prescriptions, making owner decisions, deleting source evidence, changing the registry’s own permissions, or raising its budgets. Other outward acts depend on explicit class authority and current lobe law; dreaming alone grants none.

When work needs the owner, retain completed safe work and forward **the smallest blocked remainder**, with the exact decision, evidence, anticipated default, and resume checkpoint. Temporary resource exhaustion is `deferred`, not an owner decision.

**Cheapest registry falsifier:** one disposable executor with an allowed destination and a forbidden source-evidence destination. Attempt both writes, revoke authority between admission and commit, and replay after a simulated crash. Any forbidden write, stale authorization write, duplicate effect, or `done` without verification falsifies the registry mechanism.

### Q3 — Focus directives

Store focus directives as native **lobemanager scheduling records**. Agenda and hub doors display them; neither maintains a second copy.

Proposed cheap issuance:

```text
dream focus "callgraph" --for 7d
dream focus "question:<id>" --exclusive --for 24h
dream focus clear <id>
```

An ordinary owner sentence can create the same record and return a short readback. No form or repeated confirmation is needed.

Fields:

```text
id, owner_ref, selector, mode
created_at, expires_at, completion_condition?
weight, supersedes?
```

Selectors may name a lobe, campaign, work class, or stable question identifier. Ambiguous selectors remain unresolved instead of silently choosing a target.

**Recommended semantics**

- Default: **boost**, weight ×4, expires after seven days.
- Explicit `exclusive`: all discretionary work goes to the selected scope; expires after 24 hours unless the owner supplies another duration.
- Explicit `until-cleared`: supported when the owner asks for persistent focus.
- Keep weight constant until expiry. A visible deadline is easier to understand than hidden gradual decay.
- Expiry affects future admissions; running items finish their bounded safe unit.

For boost mode, preserve **20% of discretionary capacity for unfocused eligible work**. Allocate the rest using the existing fairness score multiplied by the applicable focus weight. Keep accumulated waiting age; do not reset it because a focus directive arrived. Overlapping boosts take the maximum multiplier rather than multiplying together.

Exclusive mode suspends that fairness floor explicitly. Already-required integrity checks and verification of admitted work continue in either mode.

Focus also reorders forwarding items in agenda. It cannot make a blocked owner decision executable. Matching blocked work consumes no generative budget merely to reconsider the same blockage.

**Minimal slice:** three eligible queues, one boosted target, one blocked matching item. Demonstrate changed admission shares, continued unfocused service, unchanged authority, and automatic restoration after expiry. Fail if the blocked item runs, focus persists past expiry, or overlapping directives amplify without bound.

### Q4 — Budget overlay

Use the following **soft allocations under hard fleet-wide limits**:

| Bucket | Input/day | Generated/day | Cash/day |
|---|---:|---:|---:|
| Yin | 20k | 2k | $0.100 |
| Yang | 15k | 1.5k | $0.075 |
| Autonomous work | 55k | 5.5k | $0.275 |
| Verification/recovery reserve | 10k | 1k | $0.050 |
| **Total** | **100k** | **10k** | **$0.500** |

Within autonomous work, initially allocate:

| Class | Input / generated | Cash |
|---|---:|---:|
| Callgraph | 12k / 1.2k | $0.060 |
| Dossiergraph | 10k / 1k | $0.050 |
| Infra | 7k / 0.7k | $0.035 |
| Conversion | 16k / 1.6k | $0.080 |
| Lobemanager consultation/docs | 10k / 1k | $0.050 |

These are scheduling allocations, not promises to spend. The first exhausted resource—input, generation, cash, or external allowance—stops admission.

Additional rules:

- **Reserve verification before execution.** The recovery bucket is not permission to start work without funding its normal verification.
- Focus may redistribute **unreserved discretionary allocations** across Yin, Yang, and classes. It cannot seize existing reservations or override global, item, model, or operational limits.
- Unused allocations are borrowable by eligible work; no daily debt or carryover.
- Deterministic indexing, link checking, conversion, and approved package operations need no model tokens, but still obey resource, authority, and verification limits.
- ASR, paid lookup, consultation, and other external costs need explicit class envelopes. They cannot disappear from accounting because an existing campaign performs the call.
- Preserve Jev’s separate **$0.15/day routing envelope**. Any generative composition assigned to Jev also enters the appropriate generative ledger; routing cannot become a budget bypass.
- Fleet reservations use one authoritative day boundary and remain charged across midnight while in flight.

### Q5 — Revised ranked slices

Keep eight slices. Build contracts early; enable live execution only after the read and correction fences protect its inputs.

| Rank | Slice | Effect-level proof | Cheapest falsifier |
|---:|---|---|---|
| **1** | **Fleet budget admission and receipts** | Competing hosts respect shared token/cash limits; crashes and midnight transitions preserve reservations. | Race two requests against the last available allowance. Both admitted means failure. |
| **2** | **Work registry, forwarding, and focus** | A disposable permitted task completes and verifies; forbidden work is blocked; a narrow owner remainder appears once in agenda; focus changes scheduling and expires. | Revoke authority before commit, or focus forbidden work. Any resulting write fails the slice. |
| **3** | **Bounded retrieval and dependency fences** | An item retrieves its needed evidence within limits and refuses publication when dependencies change. | Change a source after retrieval. An output published against the old revision fails. |
| **4** | **Corrections on every read path** | Corrections/dismissals affect search, direct reads, summaries, traversal, executor packets, and exports. | Seed one dismissed claim and retrieve it through each path. Any unqualified revival fails. |
| **5** | **Whole-network adapters and Yin** | Hub, board, and lobe observations support one repaired connection and one fulfilled request, visible through normal retrieval without source loss. | Replay twice with an ambiguous link and an append-only board. Duplication, guessed resolution, or rewriting fails. |
| **6** | **Jev shadow → production** | Shadow routing is evaluated against reviewed real items; limited production routes correctly under budget with receipts. | Present ambiguous ownership and adversarial board instructions. Invented authority or an unbounded route fails. |
| **7** | **Bounded Yang and learning linkages** | A useful cross-lobe hypothesis is created, tested, then supported or rejected; ordinary retrieval preserves the distinction. | Supply a plausible false connection. Fact laundering, absent falsification, or repeated regeneration fails. |
| **8** | **Pattern learning** | A versioned pattern improves a held-out retrieval or work outcome, retains counterexamples, and can be disabled without damaging source records. | Compare against the unchanged baseline on held-out cases. No measured improvement means no promotion. |

**The decisive change is that dreaming can now complete authorized work.** Its autonomy comes from enforceable contracts and observable completion, while Yin/Yang determine how the network tightens and grows and focus determines where scarce effort goes.

Evidence limit: home-memory synchronization remained locked after three attempts, so I used your accepted round-3 summary and inspected skill contracts. Exact `links-index.lisp` integration remains unverified. Stored Callgraph notes informed the proposed boundaries; live quotas, balances, and operational state were not revalidated.


