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
| `permission_provenance` | enum | `measured` \| `unmeasured` — see §5 |

## 3. The content, measured 2026-08-08 on this fleet

Every flag below was read from that CLI's own `--help` on a host where it is
installed, on the versions this fleet runs today. **Absences are measurements
too** and are marked as such.

### codex
| tier | args | explanation shown |
|---|---|---|
| Ask each time | `-a untrusted -s read-only` | Runs only trusted commands (`ls`, `cat`, `sed`) unasked and escalates everything else; the filesystem is read-only. |
| Sandboxed | `-a on-request -s workspace-write` | The model decides when to ask; writes are confined to the workspace. |
| **Skip checks** (default) | `-s danger-full-access` | No sandbox: model-generated commands run against the whole machine. `--dangerously-bypass-approvals-and-sandbox` additionally skips every confirmation prompt. |

Approval policies in this build: `untrusted`, `on-request`, `never` — ⚠ there is
no `on-failure` and **no `--full-auto`**; do not offer either.

### claude-code
| tier | args | explanation shown |
|---|---|---|
| Ask each time | `--permission-mode manual` | Every tool use is confirmed by you. |
| Sandboxed | `--permission-mode acceptEdits` | File edits apply without asking; commands still ask. |
| **Skip checks** (default) | `--dangerously-skip-permissions` | Bypasses all permission checks. Recommended by Anthropic only for sandboxes with no internet access. |

Modes in this build: `acceptEdits`, `auto`, `bypassPermissions`, `manual`,
`dontAsk`, `plan`. `--allowedTools` / `--disallowedTools` take tool-name lists
for a middle ground.

### pi
| tier | args | explanation shown |
|---|---|---|
| Ask each time | `--no-approve` | Project-local files are ignored, so nothing in the repo can widen what pi may do. |
| Allowlist | `--tools <names>` | Only the named tools are enabled; `--no-tools` disables all of them, `--exclude-tools` denies specific ones. |
| **Skip checks** (default) | `--approve` | Trusts project-local files for this run. |

⚠ **pi has no blanket permission bypass** — its safety model is a tool
allow/deny list plus project-file trust. The modal must not invent one; the
"Skip checks" tier here is genuinely weaker than codex's or Claude's.

### opencode
| tier | args | explanation shown |
|---|---|---|
| Ask each time | *(empty)* | opencode's own default: each permission is asked. |
| **Skip checks** (default) | `--auto` | Auto-approves every permission that is not explicitly denied — opencode's own help calls this dangerous. |

Denials live in opencode's config file, not in flags; the modal should say so
rather than offer a flag that does not exist.

### qwen-code
| tier | args | explanation shown |
|---|---|---|
| Ask each time | *(empty)* | Qwen Code's own default. |
| **Sandboxed** (default) | `-s` | Runs the session inside Qwen's sandbox. |

⚠ **No bypass flag exists in this build** — `--yolo` and `--approval-mode` are
absent from `qwen --help`. Approval is changed inside the session, not at launch.
The default here is therefore the sandbox tier, and the modal must say why the
third tier is missing rather than leaving an empty slot.

### antigravity (`agy`)
| tier | args | explanation shown |
|---|---|---|
| Ask each time | *(empty)* | Every tool permission request is prompted. |
| Sandboxed | `--sandbox` | Runs with terminal restrictions enabled. |
| **Skip checks** (default) | `--dangerously-skip-permissions` | Auto-approves all tool permission requests without prompting. |

### kimi
⛔ **Unmeasured.** `kimi` is installed on no fleet host (see
`pending-bugs.md` § *AUTO-PROVISIONING COVERS THREE OF THE SIX NEW CLIs*), so
its flags cannot be read off a running binary. The modal shows the row with an
empty box and the honest label **"not measured — this CLI is not installed on
any host yet"**. ⛔ Do not copy another CLI's flags into it.

### muse
⛔ **Unmeasured and owner-gated** — closed source, needs the owner's vendor
login (`docs/owner-attention.md`). Same treatment as kimi.

### codex-litellm
⛔ **Not a row in this modal.** Settled by the owner 2026-08-08: it is a codex
session's *flip switch*, not a CLI. See `docs/settled-calls.md` and the queue
entry that removes it from the kind list.

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

A row whose flags were read off a running binary and a row whose flags were
guessed must not look the same. `permission_provenance: unmeasured` renders the
box disabled with the reason. **This is the same rule the descriptor table
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
