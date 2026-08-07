# Spec: adding an agent CLI — the repeatable pattern

**Status:** LIVE CONTRACT · first exercised 2026-08-08 by the six-CLI intake
(pi, opencode, qwen-code, kimi, muse, antigravity)
**Owner directive:** *"This pattern of CLI addition should be in docs/ spec so
that future integrations are smooth."*

This file answers ONE question: **what do I do to make a new agent CLI a
first-class citizen of yggterm?** It is a procedure, not a design essay — the
design lives in [`spec-agent-cli-harness.md`](spec-agent-cli-harness.md), which
this file is the operational half of (its §8 phase 5, "the new-CLI drill"). Read
that one for *why*; read this one to *do it*.

Per the docs-SSOT law it owns no status. What is open lives in
`pending-bugs.md`; what is waiting on the owner lives in `owner-attention.md`.

---

## 0. The one-sentence contract

**A CLI is DATA. The harness is CODE.**

Everything yggterm knows about a CLI lives in ONE `AgentCliDescriptor` value in
`crates/yggterm-core/src/agent_cli.rs`. If you find yourself writing
`match kind { … }` or `if is_claude` anywhere else, stop: that is the defect this
whole spec exists to prevent, and the repo has paid for it repeatedly (the
`cc-runtime://` hole reproduced in seven predicates at once).

---

## 1. Before you write any code: the RECON

⛔ **Never fill a descriptor field from memory or from a vendor's blog post.**
Every field is a claim about a real program. Read the CLI's source, or run its
binary, and put the provenance in a comment beside the value — the shipped
descriptors do this and it is what makes them trustworthy two versions later.

Nine questions, and the answer to each is a descriptor field:

| # | Question | How to answer it | Field |
|---|---|---|---|
| 1 | What is the binary called on `PATH`? | `package.json` `bin`, `pyproject.toml` `[project.scripts]`, or the installer's `command_name` | `binary_name` |
| 2 | How is it installed, user-locally? | its own README; ⛔ never a `sudo`/`/usr/local` route — see `spec-cli-binary-auto-provisioning` | `install` |
| 3 | How does it RESUME an existing session by id? | its arg parser | `resume_selector`, `resume_re_roots_with_cwd` |
| 4 | Does it accept a caller-supplied session id at BIRTH? | look for a `--session-id`-shaped flag, and check what it does on a MISS (creates? errors?) | `id_assigned_at_birth` |
| 5 | Where does it persist its own sessions, and what is in one file? | its session/store module | `session_store_globs`, `store_excluded_name_fragments`, `read_store_entry`, or a declared `store_scan_gap` |
| 6 | Does it write its own session title? | grep for an auto-title/rename path | `title_authority` |
| 7 | What does its screen say while a turn is IN FLIGHT? | its spinner/status component — and then **look at a real screen** | `working_screen_phrases`, `working_footer_hints` |
| 8 | What glyph heads its input composer? | its prompt component | `composer_marker`, `composer_footer_hints` |
| 9 | What permission postures can it express? | `--help` on an installed copy | `permission_modes`, `overridden_flags` |

**The shallow clones live at `~/gh/cli-reference/<name>`** — that is where
the 2026-08-08 intake read its answers, and where the next one should.

### The three answers that are allowed to be "I don't know"

Honesty is a supported state; silence is not.

- **`working_screen_phrases: &[]`** means UNMEASURED. The row then reports
  not-working because nothing was observed — never because someone guessed.
  `idle` is the answer a caller reads as *"finished, safe to move on"*, so a
  guess there is the expensive kind.
- **`store_scan_gap: Some(reason)`** means past sessions are not enumerable yet,
  and the reason names the specific obstacle. A CLI can be first-class for
  launch and resume — which is the product's core promise — while its history is
  not yet listed. `every_agent_cli_declares_a_store` fails if the reason is
  shorter than a sentence, deliberately.
- **`permission_modes`** listing only `Default` means the others have not been
  read off a real `--help`. A mode absent is REFUSED BY NAME, never
  approximated: a mapping that reads `accept-edits` but means "never ask" is a
  security boundary yggterm invented on the caller's behalf.

---

## 2. The change, in order

### Step 1 — the enum variant

`crates/yggterm-core/src/session_kind.rs`. Add the variant with a one-line doc
naming the upstream repo and its licence. Serde is `rename_all = "snake_case"`,
so the wire name follows automatically.

> ⚠ **ROLLING-UPGRADE HAZARD, named because it is real.** `SessionKind` has no
> `#[serde(other)]` fallback, so a state file or app-control payload carrying a
> new tag **fails to deserialize on an older binary**. The fleet has held three
> different versions at once. This is safe only because a session of a new kind
> cannot exist until someone installs that CLI — by which time every host is on
> the new binary. If you ever need mixed-version coexistence with a new kind,
> that ordering is the mitigation: **deploy every host first, create the first
> session second.**

### Step 2 — the descriptor

`crates/yggterm-core/src/agent_cli.rs`, appended to `AGENT_CLIS`. Copy an
existing entry wholesale and replace every field; the compiler will name any you
miss. `SessionKind::is_agent()` is derived from this table, so a CLI without a
descriptor is impossible by construction.

Fields that exist ONLY to kill a former hand-list — get these right and most of
the rest of the app needs no edit at all:

| Field | What it replaced |
|---|---|
| `slug` | the `--kind` value, `session_kind_label`, the row JSON's `icon_kind` |
| `wrapper_slug` | `resume-<x>` / `start-<x>` / `terminate-<x>` / `<x>-session-exists`, and the `matches!(kind, CodexLiteLlm)` that three files each carried a copy of to mean "local-only" |
| `remote_row_scheme`, `runtime_key_scheme` | the per-CLI scheme constructors and the `.or_else` parser chains |
| `icon_glyph` | THREE answers to "which icon?" — a kind string, a glyph, and a bespoke component reached by a third string comparison |
| `menu_hint` | the hardcoded "New … Here" entries in two branches of the row menu, the start page, and the KeyTips scope |
| `title_authority` | `SessionKind::self_generates_copy`'s `matches!(self, ClaudeCode)` |
| `working_screen_phrases` | the hardcoded phrase list in `screen_text_shows_agent_working` |
| `id_assigned_at_birth` | whether the remote identity-rebind poll runs |
| `install` | `ManagedCliTool`'s hand-mapped npm package names |

⚠ **The three shipped CLIs carry HISTORICAL spellings** that may not be renamed:
codex's remote rows are `remote-session://` (not `remote-codex://`) and Claude
Code's wrapper slug is `cc` (not `claude-code`). Those strings are in persisted
state on the fleet and in the Connect strings the metadata rail shows the user.
**A new CLI has no such debt: derive everything from its slug.**

### Step 3 — the scheme rows

`crates/yggterm-core/src/agent_scheme.rs`: two rows in `SESSION_PATH_SCHEMES`,
`remote-<slug>://` (RowIdentity) and `<slug>-runtime://` (RuntimeKey), with a
synthetic example key. `every_agent_descriptor_scheme_is_registered_and_vice_versa`
fails the build until they agree with the descriptor, in both directions.

Skip this step entirely for a local-only CLI (`wrapper_slug: None`) — the same
lock asserts it declares no schemes.

### Step 4 — let the compiler drive

`cargo check --workspace`. Every exhaustive `match` on `SessionKind` is a
compile error and a decision. **At each one, prefer deriving the answer from the
descriptor over adding an arm.** A `match` you extend by hand is a `match` the
tenth CLI will have to extend again.

### Step 5 — the catch-alls the compiler CANNOT see

This is the dangerous half, and skipping it is how a CLI ships looking fine and
behaving as codex. Two shapes:

- **`match kind { ClaudeCode => …, _ => <codex> }`** — a new CLI silently
  becomes codex. Worst instances: `remote_terminate_agent_verb` (close never
  crosses the ssh hop, so the remote CLI keeps running with no row to reach it
  by) and `automation_cli`'s `session_kind_flag` (`_ => "shell"`, so the
  scheduler launches a plain shell instead of the agent).
- **`matches!(kind, Codex | CodexLiteLlm | ClaudeCode)`** — a new CLI silently
  answers `false`: not migratable across a daemon swap, not recoverable after a
  restart, no busy dot, no completion notification.

Find them: `rg -n 'SessionKind::(Codex|ClaudeCode)' crates apps` and read every
hit that is not already registry-derived. Replace with `kind.is_agent()` or a
descriptor lookup unless the narrower set is deliberate — `codex_like_session`
genuinely is (it gates a codex-only geometry fence) and is documented as such.

### Step 6 — the arm matrices

`crates/yggterm-server/src/agent_arm_matrix.rs` and
`crates/yggterm-shell/src/agent_arm_shell_matrix.rs`: exactly two rows per
registered CLI (Local + Remote), enforced against `AGENT_CLIS.len()`. The shell
matrix also needs a DISTINCT fixture session id per CLI — a shared one makes
every local arm's path identical and fakes a mount-id collision.

### Step 7 — the surfaces

Almost all of this is now free. What is genuinely per-CLI:

- **Icon** — nothing to do. `tree_icon_glyph` and `tree_icon_kind` read the
  descriptor. See §3 for the drawing rules the glyph must respect.
- **Row context menu** — nothing to do. `agent_session_menu_items()` is one
  entry per registered CLI, in registry order.
- **KeyTips** — nothing to do; the menu's declarations are generated from the
  menu itself. Just make sure your `menu_hint` is not already spent by a sibling
  (§4).
- **Start page** — nothing to do for the session-family split button; it is one
  member per registered CLI. The recent-session CARDS still carry their own
  `open_button_label` / accent dispatch, and a new CLI gets the generic "Open"
  and the unbranded accent until someone gives it one.
- **Titlebar `+` menu** — ⚠ **STILL HAND-LISTED.** It is hand-rolled `rsx!` with
  one callback per entry rather than `RowMenuItem`, so it did not inherit the
  registry the way the row menu and the start page did, and its KeyTip node
  `insert.claude` is a literal. A new CLI does NOT appear there. Recorded in
  `pending-bugs.md`; the fix is to make that menu draw `RowMenuItem`s like every
  other menu in the app.

### Step 8 — provisioning

`spec-cli-binary-auto-provisioning` requires a user-local install and login-shell
PATH parity. `CliInstall::Npm` flows through the existing `ManagedCliTool` lane.
`Uv`, `VendorScript` and `Manual` must **refuse by name** rather than silently
falling through to an npm install of the wrong package.

### Step 9 — prove it

The acceptance is `spec-agent-cli-harness.md` §6 A6: a session opened through
yggterm must be indistinguishable (chrome aside) from
`ssh -t <host> '<cli> resume <id>'` typed into a clean shell. Per
`CLAUDE.md`, a UI change is not done until a live screenshot on the desktop host
confirms it — code review and green tests are necessary and not sufficient.

---

## 3. The icon — the rules the mark must respect

Every session-kind icon is the SAME rounded rect with a different glyph inside.
That sameness is the design (`DESIGN.md` §sidebar iconography); a mark that
breaks it is the thing a user's eye catches immediately.

```
viewBox   0 0 19 15          (1 user unit = 1 px, no scaling)
rect      x=1.6 y=1.7 w=15.8 h=11.6 rx=2.2
          stroke=currentColor stroke-width=1.15, no fill
text      x=9.5 y=9.8 text-anchor=middle fill=currentColor
          JetBrains Mono, weight 800, letter-spacing 0
```

- **`y=9.8` is a BASELINE, not a centre.** The box's centre-y is 7.5; the
  baseline sits 2.3 px below it, which is what makes a `<letter>_` pair read as
  optically centred once the `_` hangs below.
- **Colour is entirely `currentColor`** — greyscale at rest, `palette.text` when
  selected. Never hardcode one.
- **Two characters is the design.** At 7 px a JetBrains Mono character advances
  ≈4.2 px, so `>_` `$_` `K_` `Q_` `M_` `A_` `π_` all sit with ≈3.7 px of air each
  side. Three (`OC_`) drops to `font-size:6px` automatically —
  `boxed_glyph_text_style` owns that rule.
- ⛔ **Never widen the rect** to fit a longer mark. Pick a shorter mark.
- The trailing `_` is the family resemblance: it says "this is a terminal
  program". Keep it.

The mark itself should be the thing a user already associates with the CLI —
codex's `>_`, Claude's asterisk, pi's `π`. When in doubt, the first letter of the
product name.

---

## 4. The KeyTip letter

`menu_hint` is a PREFERENCE. The ladder is: user override in
`~/.yggterm/keymap.json` → your hint if free → first free alphanumeric of the
label → first free `a`–`z` → digits. Earlier declarations win, and the row menu
is not a reserved namespace, so any letter is fair game.

The agent submenu is its own scope, so its letters compete only with each other.
Currently spent there: `c` codex, `z` codex-litellm, `l` Claude Code, `p` pi,
`o` opencode, `q` qwen, `k` kimi, `m` muse, `a` antigravity.

⚠ **Adding an item can silently move an existing chord.** When the submenu took
`s`, `Edit Summary` would have slid from `ALT,E,S` to `ALT,E,I` on its own — so
it was re-hinted explicitly. Check the neighbours after you add, and re-hint
rather than let a chord change meaning silently; that is the KeyTips spec's
invariant 2.

---

## 5. The two-layer row menu

The sidebar row menu is a TREE, drawn one page at a time:

```
Open Terminal Here                 (t)   ← flat: not a choice between vendors
Open Session Here          ▸       (s)   → one entry per registered agent CLI
Open libyggterm App Here   ▸       (b)   → the host's app-manifest verbs
```

Two rules make it work, and both are load-bearing:

1. **The mouse gets a PAGE, the keyboard gets the TREE.** `ShellState::snapshot`
   flattens the tree to the current page for `ContextMenuOverlay`, and hands the
   *whole* tree to `build_keytip_tree`. So `ALT,E,S,L` resolves in one go without
   the submenu ever being drawn — the chord walker walks scopes, not pixels.
2. **Child ids are namespaced under their opener** (`open-session-here/new-agent:pi`)
   so a node key is a unique DOM identity and a unique dispatch target at both
   levels. `dispatch_row_menu_action` resolves navigation ids FIRST — they are
   the only ids that leave the menu open — then strips the opener prefix.

A page turn re-uses the same overlay at the same anchor, which is the pattern
"Move to folder ▸" already established. A true hover flyout was considered and
rejected: it needs per-item DOM geometry the shell deliberately does not keep,
plus hover-intent timing, for nothing the keyboard layer does not already give.

---

## 6. What the 2026-08-08 intake actually landed, per CLI

The honest state, so the next session does not re-derive it. Sources are on each
descriptor.

| CLI | binary | resume | birth id | store scan | title | working phrase |
|---|---|---|---|---|---|---|
| pi | `pi` | `--session <id>` | ✅ `--session-id`, creates on miss | ✅ JSONL header | generated | `Working...` |
| opencode | `opencode` | `--session <id>` | ⛔ refuses unknown id | ⛔ SQLite, gap declared | generated | `esc interrupt` |
| qwen-code | `qwen` | `--resume <id>` | ✅ uuid, **fatal on collision** | ✅ JSONL, cwd in every record | store | `esc to cancel` (i18n'd) |
| kimi | `kimi` | `--resume <id>` | ✅ implicit (miss creates) | ⛔ MD5-of-cwd buckets, gap declared | store | `Composing...` |
| muse | `muse` | placeholder | unknown | ⛔ not installed, gap declared | unknown | ⛔ unmeasured |
| antigravity | `agy` | `--conversation <id>` | ✖ | ✅ flat JSON per conversation | store | ⛔ unmeasured |

Two of these need the owner and are parked in `owner-attention.md`: Muse Code
needs a Meta login before anything about it can be measured, and `agy` needs one
captured working screen.

---

## 7. The locks that make this spec self-enforcing

You do not have to remember this file. These fail the build:

| Lock | Catches |
|---|---|
| `SessionKind::is_agent` derived from `AGENT_CLIS` | a kind with no descriptor |
| `every_agent_cli_declares_a_store` | a store neither globbed nor explained |
| `every_agent_descriptor_scheme_is_registered_and_vice_versa` | a scheme in one place and not the other |
| `wrapper_subcommands_are_derived_and_unique` | two CLIs claiming one remote verb |
| `every_registered_cli_has_both_arms` | a CLI missing from either arm matrix |
| `agent_cli_store_roots_are_mutually_exclusive` | a store nested inside another's |
| `unregistered_store_literals` (4 crates) | a store path spelled outside the registry |
| `every_agent_cli_offers_a_rendered_view` | an agent with no transcript view |
| `every_agent_cli_declares_a_distinct_composer_marker` | two CLIs sharing a composer glyph — a collision silently answers with the WRONG CLI's working hints |
| `KNOWN_PREDICATE_HOLES` / `KNOWN_STORE_PREDICATE_HOLES` | both directions: an unrecorded hole, AND a recorded hole that no longer reproduces |

The last one is the culture, not just a test: **a lock that can only pass is
worth nothing.** When you close a hole, delete its row in the same commit.

---

Related: [`spec-agent-cli-harness.md`](spec-agent-cli-harness.md) (the design and
the migration phases) · [`alt-keytips.md`](alt-keytips.md) (the chord layer) ·
[`docs-ssot.md`](docs-ssot.md) (who owns which question) · `DESIGN.md`
(iconography).
