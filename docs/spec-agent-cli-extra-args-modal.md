# Spec — the agent-CLI extra-args modal

**Owner question:** *where does a user set the launch flags for each agent CLI,
now that there are nine of them?*

Today the answer is two free-text boxes in the settings rail — **Codex Extra
Args** and **Claude Code Extra Args** — each with one line of helper text. That
shape was right for two CLIs and does not survive nine: the rail would grow nine
boxes, none of them explaining what the flag it wants actually does on that
particular CLI, and every CLI's permission vocabulary is different.

**Owner directive, 2026-08-08:** *"since we have so many CLIs now, the settings
extra args system needs to be a modal … I need you to pre-populate the same type
of least-permission-checks input box populated, and the explanations for each of
the CLIs."*

⚖ **The pre-populated value is the LEAST-CHECKS tier**, because that is what the
two existing boxes hold on the owner's own machine today (`-s
danger-full-access` for codex, `--dangerously-skip-permissions` for Claude
Code). The safer tiers are offered beside it, not instead of it. ⛔ **Migration
must preserve the two values he has set, verbatim** — a modal that "helpfully"
resets them to a default is a settings-destroying bug.

## 1. The law that keeps this from rotting

**The modal is GENERATED from the agent-CLI descriptor list, one row per
descriptor.** `docs/spec-adding-an-agent-cli.md` already made the tenth CLI
*data*; this modal must not be the place where that stops being true. No
hand-written per-CLI `rsx!`, no hard-coded list of nine — the same mistake the
titlebar `+` menu is currently filed for.

⇒ Every field below is a descriptor field. Adding a CLI adds its row; the modal
compiles unchanged.

## 2. Descriptor fields this needs (new)

| field | type | meaning |
|---|---|---|
| `permission_presets` | ordered list | the tiers offered for this CLI, safest first |
| `permission_preset.label` | string | button text: `Ask each time` · `Sandboxed` · `Skip checks` |
| `permission_preset.args` | string | the exact flags, or empty for "the CLI's own default" |
| `permission_preset.explanation` | string | one sentence, in the CLI's own vocabulary |
| `permission_default` | preset id | which one pre-populates an unset box |
| `permission_provenance` | enum | `measured` \| `documented` \| `unmeasured` — see §5 |
| `interface_mode` | enum | whether this CLI can drive the INTERFACE LLM — `spec-settings-model-providers.md` |

## 3. The content, measured 2026-08-08 on this fleet

⛔⛔ **THE LAW OF THIS SECTION, owner-directed:** *"study each CLI's nuances and
we do the bypass as they want it."* **Each CLI's bypass is expressed in that
CLI's own idiom.** They are not the same shape and must not be flattened into
one: two are flags, one is a config file, one is a *hidden* flag, and one CLI
has no permission gate at all. A modal that pretends they are five spellings of
the same thing will hand the user a flag their CLI ignores.

⚠ **`--help` is not a CLI's contract.** Measured here: qwen's `--help` lists
neither `--approval-mode` nor `--yolo`, and **both exist and work** — found by
grepping the shipped bundle and confirmed with a controlled probe (`qwen
--approval-mode bogus` names its own choices). ⇒ probe the binary, then read the
docs; never conclude a flag is absent from `--help` alone. This is the repo's
one-shape law in a new costume: I asked *"what does `--help` list"* and read the
answer as *"what does the CLI accept"*.

**Provenance, and it is part of the row (§5):**
`measured` = read off a running binary on this fleet · `documented` = from the
vendor's own reference, binary not installed here · `unmeasured` = neither.

### codex
| tier | args | explanation shown |
|---|---|---|
| Ask each time | `-a untrusted -s read-only` | Runs only trusted commands (`ls`, `cat`, `sed`) unasked and escalates everything else; the filesystem is read-only. |
| Sandboxed | `-a on-request -s workspace-write` | The model decides when to ask; writes are confined to the workspace. |
| **Skip checks** (default) | `-s danger-full-access` | No sandbox: model-generated commands run against the whole machine. `--dangerously-bypass-approvals-and-sandbox` additionally skips every confirmation prompt. |

Approval policies in this build: `untrusted`, `on-request`, `never` — ⚠ there is
no `on-failure` and **no `--full-auto`**; do not offer either. Provenance:
**measured**.

⭐ **Nuance:** codex's real home for this is `~/.codex/config.toml`
(`approval_policy`, `sandbox_mode`, `sandbox_permissions`, per-project trust),
and `-c key=value` overrides any of it per launch. So the modal's box is a
*per-launch override of a config file the user may also be editing* — say so in
the row, and never silently write his config.toml from the modal.
`--dangerously-bypass-hook-trust` is a **second, separate** danger switch (it
runs hooks without persisted trust) and must not be folded into the first.

### claude-code
| tier | args | explanation shown |
|---|---|---|
| Ask each time | `--permission-mode manual` | Every tool use is confirmed by you. |
| Sandboxed | `--permission-mode acceptEdits` | File edits apply without asking; commands still ask. |
| **Skip checks** (default) | `--dangerously-skip-permissions` | Bypasses all permission checks. Recommended by Anthropic only for sandboxes with no internet access. |

Modes in this build: `acceptEdits`, `auto`, `bypassPermissions`, `manual`,
`dontAsk`, `plan`. `--allowedTools` / `--disallowedTools` take tool-name lists
for a middle ground, and `settings.json` carries a `permissions` block
(allow/deny/ask) that outlives any single launch. Provenance: **measured**.

⚠ Two flags one letter apart: `--dangerously-skip-permissions` *is* the bypass;
`--allow-dangerously-skip-permissions` only *enables* it being used. Offer the
first; mention the second exists so nobody pastes it expecting the bypass.

### pi
⛔⛔ **pi HAS NO PERMISSION GATE AT ALL, and that is its documented design.** Its
own README: *"**No permission popups.** Run in a container, or build your own
confirmation flow with extensions."* ⇒ **there is nothing to bypass**, and a
"Skip checks" tier for pi would be theatre. Provenance: **measured** (`--help` +
the shipped README).

What the flags actually control is a *different question* — trust of
**project-local settings files**, not tool calls:

| tier | args | explanation shown |
|---|---|---|
| **Restricted** | `--tools <names>` | Only the named tools are enabled. `--no-tools` disables all built-ins and extensions; `--exclude-tools <names>` denies specific ones. **This is pi's only real safety control.** |
| Ignore project settings | `--no-approve` | Nothing in the repo can widen what pi may do this run. |
| **Trust project settings** (default) | `--approve` | Trusts project-local settings files for this run. ⚠ This is *not* a tool-permission bypass — pi never asks about tool calls either way. |

Global default for that trust lives in pi's settings as `defaultProjectTrust`
(`ask` — the default — · `never` · `always`); non-interactive modes (`-p`,
`--mode json`, `--mode rpc`) never show the trust prompt and fall back to it.

⇒ **The modal's explanation for pi must say the quiet part out loud:** every pi
session runs its tools unprompted. That is a fact about the CLI, and hiding it
behind a tier ladder that looks like Claude's would mislead.

### opencode
| tier | args | explanation shown |
|---|---|---|
| Ask each time | *(empty)* | opencode's own default: each permission is asked. |
| **Skip checks** (default) | `--auto` | Auto-approves every permission that is **not explicitly denied** — opencode's own help calls this dangerous. |

⭐ **Nuance: opencode's permission model is a CONFIG FILE, and the flag only
raises the floor.** `opencode.json` takes a `permission` block whose values are
`allow` · `ask` · `deny`, keyed by tool — `read`, `edit`, `glob`, `grep`,
`bash`, `task`, `skill`, `lsp`, `question`, `webfetch`, `websearch`,
`external_directory`, `doom_loop` — with `*` as the catch-all and glob patterns
inside `bash`:

```json
{ "permission": { "*": "ask", "bash": { "*": "ask", "git *": "allow", "rm *": "deny" } } }
```

⇒ **`--auto` respects `deny` and overrides `ask`.** The modal must say that,
because it is the one case where a user's *config* still constrains the box's
value. Provenance: **measured** (`--help`) + **documented** (opencode.ai/docs).

### qwen-code
| tier | args | explanation shown |
|---|---|---|
| Ask each time | `--approval-mode default` | Every tool call is confirmed. `plan` is read-only planning. |
| Auto-edit | `--approval-mode auto-edit` | File edits apply unprompted; commands still ask. |
| Sandboxed | `-s` | Runs the session inside Qwen's sandbox — **composable with any approval mode**. |
| **Skip checks** (default) | `--yolo` | Auto-approves everything (`--approval-mode yolo`). |

⛔ **These flags are HIDDEN from `qwen --help`** and my first pass wrongly filed
them as non-existent. Confirmed by probe: `--approval-mode bogus` answers
`Choices: "plan", "default", "auto-edit", "auto", "yolo"`, and `--yolo` is
accepted. Its settings file also carries `approvalMode` and `trustedFolders`.
Provenance: **measured**.

### antigravity (`agy`)
| tier | args | explanation shown |
|---|---|---|
| Ask each time | *(empty)* | Every tool permission request is prompted. |
| Sandboxed | `--sandbox` | Runs with terminal restrictions enabled. |
| **Skip checks** (default) | `--dangerously-skip-permissions` | Auto-approves all tool permission requests without prompting. |

### kimi
| tier | args | explanation shown |
|---|---|---|
| Ask each time | *(empty)* | Kimi's own default: every tool call is confirmed. |
| **Skip checks** (default) | `--yolo` | Auto-approves all tool calls; you are still reachable for `AskUserQuestion`. Aliases: `-y`, `--yes`, `--auto-approve`. |
| Away from keyboard | `--afk` | Auto-approves **and** auto-dismisses `AskUserQuestion` — nothing can stop to ask you. |

Provenance: **documented** — from Moonshot's own command reference, because the
binary is installed on no fleet host (`pending-bugs.md` § *AUTO-PROVISIONING
COVERS THREE OF THE SIX NEW CLIs*). ⇒ the row renders with the documented values
and a **"documented, not verified here"** marker until kimi is provisioned and
one probe confirms them. ⛔ No sandbox flag is documented; do not invent one.

### muse
⛔ **Unmeasured and owner-gated** — closed source, needs the owner's vendor
login (`docs/owner-attention.md`). Same treatment as kimi.

### codex-anything
⛔ **Not a row in this modal.** Settled by the owner 2026-08-08: it is a codex
session's *flip switch*, not a CLI, and its home is the **codex ↔ Anything**
slider in [`spec-settings-model-providers.md`](spec-settings-model-providers.md).
The name `codex-anything` is locked for every human-facing surface;
`codex-litellm` remains only as repo/binary/provider identifiers. See
`docs/settled-calls.md` and the queue entry that removes it from the kind list.
⇒ A codex row's extra-args box applies to **both** backends — one CLI, one box.

## 4. Behaviour

1. **Opening.** The settings rail keeps ONE control where two boxes are today —
   *Agent CLI launch flags · Configure ▸* — which opens the modal. The rail
   shows a one-line summary (e.g. *"9 CLIs · 2 customised"*), not the flags.
2. **Per row:** icon + name (the same `π_ OC_ Q_ K_ M_ A_` descriptors the
   sidebar uses — reuse, never redraw), the preset buttons, the editable args
   box pre-populated from the chosen preset, and the explanation under it.
3. **Free text always wins.** Typing in the box moves the row to *Custom*; the
   presets never silently rewrite what the user typed.
4. **Reset** per row, and only per row.
5. **A value is stored per CLI**, keyed by descriptor slug — never by a display
   name and never in a shared blob that a new CLI can collide with.

## 5. Provenance is part of the UI, not a footnote

A row whose flags were read off a running binary, a row taken from a vendor doc,
and a row that is guessed must not look the same. `measured` renders plain;
`documented` renders with a **"documented, not verified here"** marker (kimi
today); `unmeasured` renders the box disabled with the reason (muse today). **This is the same rule the descriptor table
already follows** — the intake declared muse's fields as placeholders rather
than faking measurements, and the modal inherits that discipline.

## 6. What this must agree with

- **`terminal new --permission-mode <default|plan|accept-edits|bypass>`** maps
  onto these same tiers. **One owner**: the mapping table lives with the
  descriptor, and both the launch verb and this modal read it. Two encodings of
  "what does bypass mean for this CLI" is the SSOT violation this repo's own law
  forbids.
- **Per-launch beats stored.** `--permission-mode` on a single launch already
  wins over the stored setting and never writes it back; the modal must not
  change that.
- **`DESIGN.md`** owns the modal's shape, and `spec-dialog-keyboard-modes.md`
  requires it to declare its keyboard mode (a settings modal is Command mode ⇒
  KeyTips reach every row).

## 7. Live proof this cannot be shipped without

⚠ The row menu shipped with zero pixels because no app-control verb can raise it
(`pending-bugs.md`). **Do not repeat that here.** Before this modal is called
done: open it through app control, screenshot it, and read back one changed
value from a launched row's `launch_command`. If app control cannot open it, the
verb to open it is part of this work, not a follow-up.
