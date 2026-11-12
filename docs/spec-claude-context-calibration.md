# Claude context calibration

Status: implemented in `.agents/skills/yggterm-agent-fleet/context-relay-gauge.py`. The
regression suite is `.agents/skills/yggterm-agent-fleet/tests/test_gauge_claude_window_and_compaction.py`.
This page is the contract. When the script and this page disagree, fix whichever drifted.

## 1. Evidence grades

| grade | meaning | may it set a percentage? | may it force a handover? |
|---|---|---|---|
| measured window | an exact model id whose window was read from the native CLI's model usage record, or a native family the gauge has recorded (section 2) | yes | yes, when compaction is off (section 5) |
| observed usage | the largest context total seen in a transcript | no | no |
| unknown | nothing measured for this model | no | no |

Observed usage is a **lower bound** on capacity. A transcript that has reached 900k tokens proves
the window is at least 900k. It does not say whether the window is 1M, 2M or larger. Observation
may refute an assumption (a 200k window cannot hold 300k tokens). It may not assert a size.

## 2. Window table

- Exact measured ids and their families are listed in the script. Bare family words such as
  `haiku` or `sonnet` never carry a window.
- An unknown model returns no window and `assumed=true`. The gauge publishes no percentage and no
  verdict band for it, and it says so in plain words.
- Measured windows already in the table keep their values.
- **Selectors are not evidence.** A `[1m]` selector in settings (`model`, or the `env` block) or in
  the environment (`ANTHROPIC_MODEL`, `CLAUDE_MODEL`) never asserts a window. It is configuration. A
  command-line `--model` override can make even a matching selector stale, and a hook cannot see
  which model the running session used.
- **A selector can only make a window UNVERIFIED.** Where a 200k assumption would be asserted (the
  `opus-4` family), a configured selector turns it into UNVERIFIED, because the session may be the
  1M variant. Without a selector the 200k assumption stands, unchanged.
- **Session-bound evidence is the transcript's own model id.** It is matched against the exact and
  family tables. Nothing else is trusted to say which window the session has.

## 3. Usage accounting

- **Claude transcripts.** Context = `input_tokens` + `cache_read_input_tokens` +
  `cache_creation_input_tokens`, taken from the usage block of the last assistant row that
  carries a non-zero total. The three fields are additive. Every request reports its own
  usage, so the gauge never reads a cumulative account total.
- The last row wins. A compaction or a `/clear` drops the count, and the reading must follow it
  down rather than keep the peak.
- **Codex** input already includes cached input. That adapter is unchanged.
- **zcode** rollouts are inclusive. Their adapter keeps its own convention detection.

## 4. Error policy

- A **native refusal row** is an assistant row the CLI wrote itself. When the row carries
  `isApiErrorMessage`, its value decides: only `true` is a refusal. An explicit `false`, or any
  invalid value, is an ordinary row, even when the model is `<synthetic>`.
- Model `<synthetic>` is a **fallback only**. It applies when the `isApiErrorMessage` key is absent,
  which is the case for transcripts that predate the marker.
- An **overflow fault** is a native refusal row whose text is the API's overflow string.
- Prose is never a marker. An ordinary successful report may quote the overflow string, and that
  is not a fault.
- Only the **last** assistant row decides the fault. A later normal turn clears it, and so does an
  explicit `false` marker on a later row.
- A fault is published and shown whatever the window, the band or the compaction policy. It is
  published even when no usage has been seen yet, so a watchdog can read `dead`.
- A fault prints the HOLD message and nothing else. It carries no keep-working advice, so the
  instruction cannot be read two ways.
- Other refusals (for example a rate limit) are not overflow and are not classified here.
- Limit: the fault rule matches the API's exact wording. If the CLI changes that wording, the
  fault goes undetected. The script cannot notice this by itself.

## 5. Compaction and action policy

The gauge reads the compaction setting and the disabling variables. It does **not** measure the
CLI's effective policy. Managed and command-line overrides are invisible to a hook, so every state
is named as what was measured, never as what the CLI is doing.

| compaction state | source | action at LAND or CRITICAL | `relay_recommended` | `verdict` |
|---|---|---|---|---|
| env-disabled | `DISABLE_AUTO_COMPACT` or `DISABLE_COMPACT` set to a truthy value | `relay` (verified capacity only) | true | raw band |
| configured-off | settings `autoCompactEnabled: false` | `relay` (verified capacity only) | true | raw band |
| configured-on | settings `autoCompactEnabled: true` | `advisory` | false | `NOTICE` at most |
| unknown | setting absent or not a boolean | `advisory` | false | `NOTICE` at most |

- **Raw band.** `pressure_band` always carries the band the measured usage reaches, OK, NOTICE,
  LAND or CRITICAL. It is published even when no action is taken on it.
- **Action verdict.** `verdict` is the value a consumer should act on. Where no relay is
  recommended it is never LAND or CRITICAL, so a consumer that reads it as a relay signal is not
  misled.
- **Conditional override.** Steering text written before this gauge measured compaction still
  names a relay at 70%. When the setting reads `true`, the advisory line says it overrides that
  nudge, and that native compaction continues in place. When the setting is absent, the line says
  the nudge is not applied, because nothing measured says compaction is off.
- Unknown capacity never relays, under any compaction state.

## 6. Published record

The hook writes one JSON object per session to the gauge's publication directory
(`~/.claude/context-gauge/<session>.json`). Keys:

- `pct`: rounded integer percentage, or null when unverified.
- `pct_exact`: one-decimal percentage, or null when unverified.
- `used`: last request's context total. A lower bound on capacity when the window is unverified.
- `observed_max`: largest context total seen in the transcript.
- `window`: window in tokens, or null when unverified.
- `window_assumed`: true when no measurement backs the window.
- `model`: model id from the transcript.
- `pressure_band`: raw band, or null when unverified.
- `verdict`: action-level verdict. One of `OK`, `NOTICE`, `LAND`, `CRITICAL` (handover-policy
  states only), `UNKNOWN`, or `FAULT`.
- `dead`: true only for a native overflow fault.
- `fault`: `"native-overflow"` or null.
- `action`: `none`, `advisory`, `relay`, `hold` or `unverified`.
- `relay_recommended`: boolean.
- `compaction`: `env-disabled`, `configured-off`, `configured-on`, `unknown`, or null (zcode).
- `session`, `cwd`: as before.

Consumers that only read `dead` keep their old behaviour. Consumers that read `verdict` must treat
`FAULT` and `UNKNOWN` as non-OK and as non-relay.

## 7. Limits this spec does not remove

- The thresholds (55 / 70 / 85) are conventions. They are not measurements.
- The unverified-window note appears from 110k tokens. That number is a silence threshold only. It
  is not a percentage.
- The truthy set for the disabling variables (`1`, `true`, `yes`, `on`) is an assumption about how
  the CLI parses them. It has not been checked against the CLI.
- The zcode adapter still matches the overflow text as a substring of raw rollout lines. That is
  the pre-existing behaviour and is out of scope for the Claude rule above.
- Steering text outside this repository may still carry the legacy relay-at-70% wording until it is
  regenerated from its source.
- A 1M session of the `opus-4` family reads as 200k when no `[1m]` selector is configured in
  settings or environment, because the transcript does not show the suffix and a command-line
  `[1m]` override is invisible to a hook. In that case the gauge can report a near-full wall that is
  really a fraction of a 1M window. This limit predates the selector rule above, and the rule does
  not close it.

## 8. Verification

Run from the skill directory, with bytecode writing and the user site disabled:

```sh
python3 -B -I tests/test_gauge_claude_window_and_compaction.py
python3 -B -I tests/test_gauge_codex_native.py
python3 -B -I tests/test_gauge_autoclaw_projection.py
```

The Claude suite is hermetic. Each hook run uses a fresh temporary home and a minimal environment.
It calls no model and reads no real transcript. Setting `CONTEXT_RELAY_GAUGE` to another copy of the
script runs the same assertions against that copy, which is how a before-and-after comparison is
made.
