**Accept slices 1–4 as necessary foundations. They do not yet establish reliable progress under continuous activity, durable recall at multi-year scale, or complete owner correction.** The falsifiers demonstrate the specific cases tested; the broader guarantees remain to be proved.

My rulings: **reject volatile root-file forwarding; accept a bounded, proposal-only structural pass; trial Luna for semantic routing; fix commit starvation before expanding the system.**

**Q-A — Rebut instantaneous forwarding into root context files.**

Pushing NOW.md and recent diffs into every harness root file offers little value relative to its recurring context cost, irrelevant cross-campaign exposure, and conflict with yggsteer ownership. Changing the file also does not establish that an already-running session consumed the update.

Two corrections to the proposal’s framing:

- An atomically published NOW.md can be internally consistent while covering only an older source prefix. Its watermark and pending tail should make that limitation explicit. Instant delivery does not itself make it “half-consolidated.”
- Repeated input has context and potential billing costs, but the precise incremental cost depends on caching and harness behavior. Changing timestamps in a shared prompt prefix can itself cause cache churn.

The useful version is **push invalidation metadata, pull relevant content**:

1. Keep a stable, yggsteer-owned instruction pointing to `ygg-memory now --ns=…`. No changing mtime, envelope, or diff in the root file.
2. After a successful atomic publication, advance a namespace generation in a sidecar. Use a logical generation/content hash rather than filesystem mtime.
3. At session start, load the relevant namespace and applicable owner context.
4. During long sessions, check generations at task checkpoints. Retrieve a bounded, relevant change summary when needed.
5. Record the generation actually delivered to that session. File delivery alone is insufficient.

Owner corrections and source retractions warrant prompt invalidation without waiting for dreaming. They still need a delivery mechanism for sessions that already loaded the old assertion.

**Pull at session start is the right default, but it is insufficient by itself for long-running sessions.** Add selective checkpoint refresh. Official OpenAI documentation likewise distinguishes required guidance in `AGENTS.md` from memory used for recall. [OpenAI Memories](https://learn.chatgpt.com/docs/customization/memories)

**Q-B — Accept a structural pass, with a narrow mandate.**

There is a useful job here that differs from composition and criticism: discovering that the current organization makes information difficult to retrieve. A critic checks claims; a structural pass can identify duplicate doors, missing cross-links, oversized sections, or a taxonomy that no longer matches the content.

Use **sol MEDIUM to propose those changes**. Do not introduce another autonomous writer or assume MEDIUM is cheaper overall without measuring input, retries, and escalation.

The minimal design:

| Dimension | Ruling |
|---|---|
| Input | An immutable, versioned packet containing memory indexes, retained claims, ownership registry, relevant agenda items, selected board threads and graph neighborhoods, plus retrieval failures. |
| Evidence boundary | Preserve source IDs, versions, author roles and trust classes. Agenda views and board summaries point to evidence; they do not become independent corroboration. |
| Output | A bounded proposal containing the retrieval problem, proposed operation, affected stable IDs, source references, version preconditions, expected benefit and rollback. |
| Write authority | May write proposals and rebuild validated memory-owned derived indexes. Canonical lobe records, boards, steering, evidence and ownership remain with their respective writers. |
| Cadence | Initially at most weekly, only when inputs changed and structural debt exists. Allow an earlier run after demonstrated retrieval failure or a completed campaign phase. |
| Adoption gate | Preserve claim identities and evidence reachability; run retrieval replay; apply through the owning writer with version checks. Escalate semantic merges and ambiguous supersession to sol HIGH. |

Keep external-source ingestion isolated from structural proposals. A board post suggesting a fact must retain that provenance until its claim is evaluated through the normal path.

Three conservation rules matter:

- Omission from a proposal never authorizes deletion.
- Combining documents must preserve their distinct claims, exceptions and evidence.
- An inferred pattern remains a hypothesis until supported; reorganizing it does not promote it into an owner preference or rule.

Implement this as another job mode under the existing scheduler, budget and receipt machinery. **Its first output should be proposals. Earn automatic index edits through replay evidence.**

**Q-C — Adopt Luna for a narrow trial; keep deterministic control decisions in code.**

The supplied OpenAI API and pricing claims check out: Decisions supports `predicate`, `choice` and `score`, currently with `gpt-6-luna`; the endpoint charges $0.10/M input with no output charge. The approximately 10× speed claim is OpenAI’s comparison, not a measurement of this harness. [OpenAI Decisions](https://developers.openai.com/api/docs/guides/decisions)

TypeSafe lists Jev 1.13 at $0.042/M input, output free. Its own documentation warns about arithmetic, date comparisons, indirection, adversarial content, irrelevant context and option-order effects—several of which occur in the proposed workload. Those warnings justify testing; they do not prove Luna is better. [TypeSafe models](https://docs.typesafe.ai/models), [Jev 1.13 limitations](https://docs.typesafe.ai/model-jaggedness/jev-1.13)

| Use case | Decision |
|---|---|
| **(a) Dreamer candidate selection** | **Deterministic code.** Age, deadlines, backlog, cost limits and service guarantees are explicit policy. Neither decision model nor sol improves that calculation. |
| **(b) Claim triage** | **Split the task.** Code computes expiry, review deadlines and explicit supersession edges. A decision model can flag possible semantic duplication or contradiction between retrieved claims. Ambiguous scope or supersession goes to an evidence-citing reasoner. |
| **(c) Board/graph classification and routing** | **Best initial fit for Luna.** Apply explicit owner metadata first; classify unresolved items against a bounded registry of destinations, including `unknown` and `multiple`. Return suggestions, not transfers of ownership. |
| **(d) Critic verdicts** | **Retain the evidence-citing sol critic.** A decision model can prioritize suspicious claims or run bounded entailment checks. It should not become the sole acceptance gate for durable memory. A typed verdict without supporting evidence is harder to review and rebut. |
| **(e) Gauge/relay decisions** | **Deterministic code.** Token measurements, thresholds, consent rules and row state belong in code. A model may help write a handover after the decision. |

A fixed answer shape does not imply a simple judgment. Deciding whether a later statement supersedes an earlier one may require resolving identity, scope, exceptions and authority.

My provider choice is **Luna first, shadow-only for routing**. That recommendation rests on integration economy with the existing OpenAI stack, subject to actual Decisions API access—not demonstrated accuracy superiority. Codex subscription access should not be assumed to provide API entitlement.

Do not add Jev solely for the price difference. At 200 examples averaging 2,000 billed input tokens, inference is approximately **$0.04 for Luna versus $0.0168 for Jev**. Labeling and integration dominate that experiment.

The cheapest useful falsifier is:

1. Freeze a small real routing corpus, including ambiguous ownership, unfamiliar topics, multilingual items, quoted instructions and adversarial board text.
2. Establish labels from source ownership; mark unresolved cases unresolved.
3. Compare deterministic routing, Luna and—if readily accessible—pinned Jev 1.13. Include option-order permutations and irrelevant-context variants.
4. Measure accepted-route precision, coverage, abstention, p95 latency and total cost including fallback.
5. Reject adoption if it cannot improve useful routing coverage at the chosen error budget. Start any live use with reversible suggestions and audits.

A 200-case set can cheaply falsify suitability. It cannot certify rare-error reliability: even zero errors in 200 independent cases leaves an approximate 95% upper error bound of 1.5%.

Cache results by content hash, registry version, rubric version and model version. On refusal, timeout or uncertainty, retain `unknown` or use the existing reasoner. **Classification must never grant write authority.**

**Q-D — Four remaining gaps, ranked.**

I would withhold “full OpenAI parity.” The public documentation checked establishes background memory generation and consolidation, but does not provide enough detail to certify the particular three-objective implementation referenced here. The practical acceptance bar should be observable behavior.

**1. Successful progress under sustained concurrency — required now.**

The current read-version fence can reject completed synthesis indefinitely. A red canary exposes the failure; it does not resolve it.

Compose against an immutable prefix through watermark **W**. At publication, check the output generation and relevant dependencies rather than requiring the entire namespace to have stopped changing. Leave events after W pending. Concurrent corrections, deletions and retractions must still invalidate affected claims or override them at read time. Publish claims, NOW, manifest and watermark as one coherent generation.

The fairness formula also lacks a service guarantee. Its age multiplier caps at **5**, so a one-delta namespace can lose forever to continuously replenished namespaces scoring above 5. Add overdue-service priority or bounded service rotation, with explicit capacity accounting.

Keep persistent per-namespace successful-progress state independently of the last-64 receipt log. Attempts and blanket deferrals must not reset the age of unresolved work.

**Falsifier:** sustained writes throughout repeated compositions, including overlapping corrections, while a small namespace waits. Demonstrate safe commits and eventual service.

**2. Bounded retrieval with durable fact preservation — required before multi-year scale.**

Injecting all retained CLAIMS into every composition eventually exhausts context and increases cost. A tiny NOW cannot be the long-term retrieval layer.

Retain claims by stable identity with evidence links, then use hierarchical indexes and bounded retrieval. Keep an explicit queue of deferred obligations. A per-event `represented` disposition does not prove that every material fact inside that event remains recoverable.

Also distinguish **new evidence confirming an old claim** from mere restatement. Preserve original evidence history while allowing genuinely newer verification to be recorded.

**Falsifier:** repeated compaction across a large historical corpus, followed by queries about rare early facts, exceptions and corrections. Measure correct retrieval and evidence resolution—not just manifest completeness.

**3. Owner review and correction across every read path — required now.**

The dismissal falsifier proves resistance to replay of the tested text. Line filtering does not yet establish resistance to paraphrase, copied claims, merged bullets, alternate retrieval paths or old composer inputs.

Corrections need stable targets and explicit scope, applied across claim retrieval, NOW, search results, composer briefs and mirrors. Preserve the distinction between “false,” “stop surfacing,” and “surface under these circumstances.”

Add a review surface that answers: **what is remembered, why, from which evidence, what changed, and what will this correction affect?**

**Falsifier:** dismiss a claim, then encounter its paraphrase, duplicate and archived form through every supported read path. It must not return as an active assertion.

**4. Evidence-linked pattern learning and demonstrated usefulness — needed for full dreaming; sophisticated automation can wait.**

The slices establish retention and temporal handling. They do not yet demonstrate discovery of useful cross-session lessons or broader owner understanding.

Use sol/GLM to propose patterns with supporting episodes, counterexamples, scope and uncertainty. Repetition copied across summaries must not count as independent evidence. Promote preferences cautiously and distinguish observed practice from owner instruction.

Evaluate cold-session tasks: fewer repeated questions, correct recovery of prior decisions, appropriate use of owner constraints, and fewer stale assertions. The structural pass in Q-B can assist retrieval, but does not itself establish this capability.

**Next iteration:** close gap 1, exercise correction across all read paths, and add a small historical retrieval replay. Defer root-file broadcasting and broad autonomous reorganization.

The starvation finding is posted to `dreams/features`: **ACK-dbf6f35d4c**. Source checks are recorded in [RESEARCH.md](/tmp/sol-final-consult/RESEARCH.md). Deployment and test claims above remain supplied evidence, not independently audited results.


