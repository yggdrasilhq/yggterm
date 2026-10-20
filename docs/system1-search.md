# Optional System-1 search ranking

`ygg-memory search 'orchard irrigation' --semantic` can rank the same regex
matches by relevance. Plain search keeps its streaming deterministic path and
does not read the classifier config. Exact identifiers, filenames, single
words and regex expressions always use deterministic lookup, including with
`--semantic`. Only natural phrases are eligible for semantic classification.

Create `~/.yggterm/config/system1.json` to opt in:

```json
{
  "enabled": false,
  "model": "jev-latest",
  "endpoint": "https://api.typesafe.ai/v1/systemone",
  "budget_usd_per_session": 0.10,
  "rate_rps": 2,
  "fallback": "base"
}
```

Set `enabled` to `true` to enable explicit `--semantic` requests.
`ygg-memory system1 status` shows the effective config and UTC-day spend,
attempts and input tokens. Missing config defaults to OFF. Invalid config
falls back to deterministic search. No keys belong in this file.

Set `JEV_API_KEY` (or `TYPESAFE_API_KEY`), and point
`YGG_SYSTEM1_PRIVACY_GATE` at the installed privacy gate exposing `l1_scan`
and `l1_mask`. The adapter runs both locally before any request; any candidate
or unavailable gate holds the request. It cannot adjudicate privacy findings.
The query and bounded snippets are checked together. Source paths stay local.

`YGG_SYSTEM1_SESSION_ID` identifies a shared session budget across processes.
Managed sessions can instead use `CODEX_THREAD_ID`, `YGGTERM_SESSION_ID`,
`LC_YGGTERM_SESSION_ID` or `CODEX_SESSION_ID`. Missing identity holds model
calls. These counters live under `~/.yggterm/system1/`, shared by callers
using the same home and identity. `YGGTERM_HOME` relocates config and state
for isolated testing. Other consumers must use this same counter contract;
unrelated legacy classifier scripts do not yet share these counters.

Each eligible search sends at most one batch of 50 snippets, bounded to
24,000 request bytes. The host lock serializes transport and enforces at
most two requests per second; no automatic retries. A session stops at
$0.10 or 1,000 attempts (lower configured budgets/rates are allowed). The
client reserves a UTF-8-byte upper bound before sending, reconciles measured
input tokens, and conservatively retains the reservation on missing usage
or uncertain transport. Requests time out after five seconds. Redirects are
refused. Supported model prices use $0.042 per million input tokens.

Incomplete distributions or nonfinite values hold the request. Judgments below
0.6 confidence or selected probability abstain as unknown; confidence never
authorizes removing a hit.

Valid labels are `direct`, `supporting`, `unrelated`, and `unknown`. Ranking
is stable within each label. Unknown positions stay fixed; every base hit
remains present, even when labeled unrelated. The original regex cap and
owner-dismissal filter run before classification. Missing credentials,
malformed answers, oversized batches, privacy holds and exhausted budgets
all preserve base order. An explicit semantic call may add bounded latency;
ordinary search adds no classifier work.

Successful complete classifications are cached by masked request bytes,
endpoint, model and rubric revision. Unknowns and errors are not cached.
Local receipts store request digests, model/rubric, attempts, usage, latency
and outcomes; no snippets, queries, source paths or API keys are logged.

This interface does not expand candidate retrieval, remove matches, change
memory content, integrate GUI preferences, or admit other classifier tasks.
Default OFF remains in place after validation.

## Validation

Run `python3 -m unittest discover -s .agents/skills/yggterm-agent-fleet/tests
-p test_memory_system1.py -v` as one shell command. The planted-hit falsifier
checks all 256 assignments of four relevance labels, exact hit preservation,
fixed unknown slots, deterministic bypass, privacy holds, counters, cache,
transport errors and streaming/cap compatibility. Live synthetic evaluation
should also check expected relevance, rather than preservation alone.
