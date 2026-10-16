# Research record — 2026-10-16 post-build dreaming consult

The implementation and deployment statements are supplied evidence from BRIEF.md, not independently audited in this consult. No production code or campaign state was changed.

Primary sources fetched:

- https://developers.openai.com/api/docs/guides/decisions — public beta, gpt-6-luna, predicate/choice/score; advertised approximately 10x Responses speed; $0.10/M input with no output or cache charges for this endpoint; application-labeled examples required for thresholds. Markdown fetched into decisions-source.md because the browser extraction initially returned navigation.
- https://developers.openai.com/api/docs/models/gpt-6-luna — ordinary generative endpoint pricing is different; do not confuse the model's Responses pricing with Decisions pricing. Markdown in luna-source.md.
- https://docs.typesafe.ai/models — Jev 1.13.0 remains listed at $0.042/M input, output free. Jev 1.14 exists, but the requested comparison is specifically 1.13. Pin exact evaluated IDs.
- https://docs.typesafe.ai/model-jaggedness/jev-1.13 — provider documents problems with numeric precision, dates, indirection, irrelevant context, adversarial state, and choice ordering. These are workload-specific reasons to benchmark, not proof Luna is better.
- https://learn.chatgpt.com/docs/customization/memories — required guidance belongs in AGENTS.md or checked-in documentation; memories are a recall layer. Background processing waits for eligible idle sessions. Markdown in memories-source.md.

Searches included exact Decisions/gpt-6-luna topics on official OpenAI domains; official dreaming/memory objectives; TypeSafe Jev 1.13 pricing/RLCD; TypeSafe documented failure modes. Secondary Jev comparison sites were excluded from conclusions in favor of docs.typesafe.ai. The searches did not establish an official source for the exact 'three objectives' formulation in the brief; compare functional requirements, not undisclosed implementation parity. Neither model was called, so local accuracy, calibration, p95 latency and account access remain unmeasured.

Prior chain checked: 2026-10-06-yggterm-memory-dreaming-round3-fault-tolerance.md and round4-auto-and-completeness.md. Their read-version fence and delta-priority decisions are historical antecedents, not evidence of the newly reported deployment.

Synchronized codex namespace yggterm; status current, no unabsorbed diffs, NOW.md absent in that namespace. No absence claim about other namespaces follows.
