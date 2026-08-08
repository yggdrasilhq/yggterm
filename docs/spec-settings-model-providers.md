# Spec — the codex ↔ Anything slider, and the interface-LLM provider dropdown

Two settings changes the owner asked for on 2026-08-08, in the same breath as the
extra-args modal ([`spec-agent-cli-extra-args-modal.md`](spec-agent-cli-extra-args-modal.md)).
They are separate features that touch the same pane, and they share one law:
**a provider is data, not a CLI.**

⛔ **Naming, locked by the owner:** the human-facing name is **codex-anything**.
`codex-litellm` survives only as identifiers — the repo, the binary, the codex
provider key. See `settled-calls.md`.

## 1. The codex ↔ Anything slider

**Owner directive:** *"codex sessions should have an extra slider in settings
codex ↔ Anything."*

`codex-anything` is not a CLI and must stop being a `--kind` value. What it
really is: **a codex session pointed at a different model backend.** So the
control is a two-position slider inside the codex block of settings:

```
Codex backend      [ Codex ]——————[ Anything ]
                   OpenAI's own backend · your configured endpoint
```

### What the slider actually switches — measured, because there are two mechanisms

| mechanism | evidence on this fleet |
|---|---|
| a **separate binary** | `~/.yggterm/npm/bin/codex-litellm → ../lib/node_modules/@avikalpa/codex` — a private fork, provisioned on every host |
| a **provider key in codex's own config** | `~/.codex/config.toml` carries `[model_providers.litellm]` and a top-level `model_provider` |

⇒ **Pick ONE and write down why.** If the fork exists only to preset a provider,
the honest implementation is the stock binary plus a per-launch override
(`-c model_provider=…`), and the fork retires. If the fork carries real
behaviour the stock binary lacks, the slider selects the binary and the spec
says so. ⛔ What is not acceptable is the current state, where the same choice is
encoded as a CLI kind, a binary and a config key at once — three encodings of one
question, which is the SSOT violation this repo's own law forbids.

### Rules

1. **The row stays `kind = codex`.** Same icon, same resume verb, same title and
   summary pipeline. The backend is a property of the session, not a species.
2. **Settings sets the default for NEW codex sessions**; the slider does not
   retro-flip a running one. A running session's backend is fixed at launch
   because its transcript already belongs to that backend.
3. **The flip is also reachable on the codex session's own surface** — it is a
   session superpower, and the owner's word for it is a *flip switch*. Settings
   is where the default lives; the session is where the switch lives.
4. **Read the effect back.** After a launch, the row's `launch_command` must
   show which backend it was born with. A slider that reports the request rather
   than the effect joins the six verbs already filed for exactly that.
5. **The endpoint/API-key/interface-model fields stay where they are** — they are
   codex-anything's configuration, and §2 below is what generalises them.

## 2. The interface-LLM provider dropdown

**Owner directive:** *"the interface settings system (currently we use litellm)
should be preceded by a dropdown of litellm (selected default) or any cli sdk
(those have available, like claude code, codex, etc.) and the model to be used."*

The **interface LLM** is the model yggterm itself calls — session titles,
summaries, the working indicator's phrasing. Today that is hard-wired to an HTTP
endpoint (LiteLLM) with an endpoint field, an API-key field and a model field.

**New shape — the provider comes FIRST and decides which fields exist:**

```
Interface provider  [ LiteLLM ▾ ]        ← default, selected
                    Endpoint   https://…/v1
                    API key    ••••••
                    Model      vercel/juju/gpt-oss-120b

Interface provider  [ Claude Code ▾ ]    ← a CLI SDK
                    Model      claude-haiku-4-5-20251001
                    (uses the CLI's own login on this host — no key needed)
```

### Which CLIs may appear in that dropdown — measured 2026-08-08

A CLI qualifies iff it has a **non-interactive mode** and a **model selector**.
Both were read off the binaries this fleet runs:

| CLI | non-interactive | model | offer it? |
|---|---|---|---|
| claude-code | `claude -p/--print` (`--output-format`, `--input-format`) | `--model` | ✅ |
| codex | `codex exec` (alias `e`) | `-m` / `-c model=…` | ✅ — and it inherits the §1 backend |
| pi | `-p/--print`, `--mode json\|rpc` | `--model`, `--provider` | ✅ |
| qwen-code | `-p/--prompt`, `--output-format json\|stream-json` | `-m` | ✅ |
| opencode | `opencode run`, `opencode serve` (headless HTTP) | `opencode models` lists them | ✅ — `serve` is the cheaper shape |
| kimi | `--print` / `--quiet` (documented, binary not installed) | `-m/--model` | ⏸ only once installed and measured |
| antigravity | none measured | — | ⛔ |
| muse | not installed, owner-gated | — | ⛔ |

⇒ **The dropdown is generated from a descriptor field** (`interface_mode:
None | Print { flags } | Server { … }`), exactly like the extra-args modal. The
tenth CLI appears in both surfaces by adding a descriptor, or in neither.

### Rules that keep this from becoming expensive or dishonest

1. **A CLI SDK carries no key.** The CLI is already authenticated on that host,
   which is the whole appeal. The key/endpoint fields must *disappear*, not sit
   there disabled — a field that does nothing is a lie about what is being used.
2. **A process spawn is not an HTTP call.** The interface LLM fires on every
   title/summary; a CLI invocation costs a process start and a cold model
   selection each time. ⇒ keep the existing chore caps (3 generations per tick),
   prefer `opencode serve`-style long-lived endpoints where a CLI offers one, and
   **measure the per-call latency of the selected provider before making it the
   default for anyone but the person who chose it.**
3. **Never persist a heuristic fallback over a provider error.** Standing law
   (429s from the LiteLLM endpoint are the known case): a failed generation
   leaves the field empty and retries later; it never writes a guess that looks
   like a result.
4. **The model list comes from the provider**, not from a hard-coded array:
   LiteLLM from its `/models`, opencode from `opencode models`, others free-text
   with the CLI's own default pre-filled and marked as such.
5. **Per-host reality:** a CLI SDK provider only works on hosts where that CLI is
   installed and logged in. The setting must say which hosts satisfy it rather
   than failing at the first title generation on a host that does not.

## 3. Live proof neither of these ships without

Open the settings pane through app control, screenshot both controls, then prove
the effect: for §1, a codex row whose `launch_command` names the chosen backend;
for §2, a generated title in the trace with the provider that produced it. The
row menu shipped with zero pixels because no verb could raise it — do not repeat
that here.
