<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Project overlay interface

Each project should define the following explicitly.

### 1. Main artifact

What is the main canvas actually for?

Examples:

- terminal
- map
- graph
- document
- dashboard

### 2. Navigation model

What lives in the left rail?

Examples:

- sessions
- folders
- machines
- topology nodes
- boards

### 3. Right rail modes

What modes can the right rail switch between?

Examples:

- metadata
- settings
- notifications
- inspector
- filters

### 4. Vocabulary

Define the user-facing nouns here, not in the reusable sections.

Examples:

- session
- terminal
- paper
- folder
- separator

### 5. Domain-specific control rules

Document:

- quick action labels
- context menu labels
- titlebar actions
- view toggles

### 6. Domain content typography

If the main artifact needs a special font, define it here.

Examples:

- terminal font
- map label font
- monospace editor font

#### Document reading font (document surfaces, markdown reader)

Rendered markdown documents (yedit and any future document surface) read like
an article, not like chrome. User-directed 2026-07-18 ("readability like The
New York Times"), refined 2026-07-23: **sans-serif** — the user prefers a very
legible sans over serif; the formatting/spacing system is what carries the
article feel:

- Body: legible sans reading stack `'Inter', 'SF Pro Text', 'Segoe UI',
  'Noto Sans', 'Liberation Sans', 'Helvetica Neue', Arial, sans-serif`,
  16px, line-height 1.7, letter-spacing 0.002em (obsidian-reference pass,
  2026-07-23).
- Tables: horizontal separators ONLY — no vertical grid, no header fill;
  header carries a 2px bottom rule, rows 1px. A full cell grid reads as a
  spreadsheet; an article's table is rows of text with quiet rules.
- Blockquotes: accent left bar, upright text (never italicize the whole
  quote).
- Block click-to-edit: the in-place editor keeps the READING typography with
  no box — a faint accent-tinted background (color-mix ~9%) marks the active
  block. Entering edit is a reveal of the source text, not a mode jolt into a
  form (the obsidian in-place feel, at block granularity).
- ⚠ An accent LEFT BAR is the BLOCKQUOTE vocabulary — never reuse it for
  editability or selection in a document surface (user caught the collision
  2026-07-23: "that blue line does not denote editability; on the obsidian
  screenshot that was a quote"). One visual token, one meaning.
- Headings: same sans, heavy weights (h1 800 → h4 720), negative letter
  spacing on h1/h2, more air above than below (h1 26px above / 12px below,
  scaling down). Paragraphs carry 14px bottom margin. NO border/rule under
  headings — no decoration the markdown didn't ask for.
- Links: accent color only, no underline (markdown has no underline syntax, so
  underlines are never ours to add).
- Code (inline + blocks) stays monospace at a reduced em so it sits quietly
  inside body text — and it is the project monospace (`JetBrains Mono`), not
  whatever the platform calls `ui-monospace`.

##### Three reading surfaces, one type system

`emd-renderer` says what a document IS; **`yggui::prose` in libyggterm says how
it reads**, and it is the ONE owner of every face, size and rhythm below. A host
supplies its brand colours (`ProseInk`) and nothing else — no host may spell a
face, a size, a leading or a tracking of its own. Locked by
`the_markdown_adapter_owns_no_typography_of_its_own`.

Three surfaces, each NAMED by its call site (never inferred from a flag):

| `ProseTokens::` | Surface | Body copy |
|---|---|---|
| `document()` | markdown reader, live preview, click-to-edit | owns it: the sans reading stack, 16px/1.7/0.002em |
| `conversation()` | the Web View transcript | **inherits** it from the turn |
| `rail()` | contributed 300px pane | inherits face+size, tightens leading to 1.55 |

The transcript inherits because the turn above it has already decided, and the
two sides of a conversation are deliberately unequal: the person's ask is sans
at 15px, the machine's answer serif at 16px/1.72, and one renderer serves both
(`ProseBody::CONVERSATION_ASK` / `CONVERSATION_ANSWER`). A markdown root that
re-decides silently wins over the turn — that is not hypothetical, it shipped:
answers drew at 1.55 for as long as the surface shared a `compact` flag with the
rail pane.

Everything BELOW body copy — headings, code, lists, tables, quotes, rules — is
identical on all three. A heading is a heading.

Change a value in `crates/yggui/src/prose.rs` first, then here.
