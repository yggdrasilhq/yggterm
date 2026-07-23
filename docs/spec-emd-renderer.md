# Spec: paper-renderer — the fluid markdown-superset document engine

**Status:** RECORDED 2026-07-23 (user-directed: "record spec as you see fit …
and implement an initial scaffolding so we do not redo work"). Scaffolding
SHIPPED same day: `crates/paper-renderer` holds the typed model + parser +
source-range mapping, extracted from yggterm-shell as a pure move.
**Owner surfaces:** `crates/paper-renderer` (model/parse), yggterm-shell's
document surface (render, for now), yedit (pilot consumer).

## 1. Why this exists

The user's framing, in substance (2026-07-23): *the USP of obsidian and
notion is fluid WYSIWYG rendering of a markdown superset — that is a work in
itself. Paper (our libyggterm notion clone) will need it, so build it in
yedit for now. Most of yedit can be the base for ztlkn (a zettelkasten app:
my vault's organization as the standard, obsidian inside yggterm) — or ycode
(vscode-like). Record so we don't redo work.*

So one engine, three consumers on one gradient:

- **yedit** (today): notepad + reader; the proving ground.
- **paper** (next): the notion-class app; block-structured documents.
- **ztlkn** (later): the zettelkasten; the user's vault conventions as the
  grammar standard, plus graph/backlinks over the same model.
- **ycode is deliberately NOT a consumer.** A vscode-class editor needs an
  editor-engine class (LSP, virtualized buffers) that this line does not
  grow into; recording it as a libyggterm app idea is fine, promising it
  from yedit's base would be self-deception.

## 2. The two settled calls (decided 2026-07-23; revisit only with the user)

1. **Source-decorated, not block-model-at-rest.** The markdown SOURCE is the
   document (obsidian's architecture), not a block database serialized to
   markdown at the edges (notion's). Reasons: the vault already exists as
   files; lossless round-trip is checkable; ztlkn's "vault as standard"
   demands file-truth. Paper may later add a block-model layer ON TOP of the
   same typed tree if a real need appears — that decision is deferred, not
   made.
2. **Fluidity grows at block granularity first.** yedit already has
   click-block-to-edit (Typora-lite) with the editor visually continuous
   with the rendered block (reading typography, tint affordance — no box, no
   mode jolt). The next fluidity step is caret-line syntax reveal INSIDE the
   edited block (obsidian's feel at block scope) via the styled-mirror-under-
   textarea technique — contenteditable on WebKitGTK is a minefield we do
   not enter. Line-granular whole-document fluidity is the horizon, not the
   next step.

## 3. The invariant that makes it trustworthy: lossless round-trip

render → edit block → splice back must be byte-faithful outside the edited
range. This already holds structurally (`top_level_block_ranges` +
zip-checked splice: len mismatch ⇒ editing disabled, never a wrong splice).
Every grammar addition MUST keep the range pass in lockstep with the fold —
the existing `top_level_block_ranges_align_with_the_folded_blocks` test is
the lock; extend it per new node kind.

## 4. Crate layering (as shipped)

- `paper-renderer`: model (`MdBlock`/`MdInline`) + parse
  (`parse_markdown_blocks`) + source ranges (`top_level_block_ranges`).
  Pure — no Dioxus, no theme — so server-side consumers (ztlkn's graph
  indexer) can use it without a UI stack. Raw HTML is dropped by
  construction (never innerHTML; note content must not reach the shell's JS
  context).
- yggterm-shell keeps the Dioxus RENDER (`md_block_node`, `md_inline_nodes`,
  `document_reading_typography`) for now; extract it as
  `paper-renderer-dioxus` (or a `render-dioxus` feature) once the visual
  language stops changing weekly. DESIGN.md § "Document reading font" is the
  style SSOT.
- A new block/inline variant deliberately breaks every renderer's `match`
  until it decides how to draw it — unknown-fails-loud, never a silent hole.

## 5. Superset grammar v1 (the next work item)

Grammar source of truth: **the user's actual vault conventions**, inventoried
from the vault itself before drafting (evidence over obsidian's docs).
Expected v1 set, to be confirmed by that inventory: wikilinks
(`[[target|alias]]`), tags (`#tag`), callouts (`> [!note] …`), task items
(`- [ ]`/`- [x]`), YAML frontmatter (typed, not rendered as a paragraph),
strikethrough/tables/tasklists (already on). Each lands as a TYPED node with
its source range, plus a render decision, plus a round-trip test. Wikilink
RESOLUTION (what a target names, across the vault) is ztlkn's domain, not the
renderer's — the renderer only yields the typed node.

## 6. Migration order

0. ✅ Scaffolding: crate extracted (pure move), tests moved, consumers
   re-import. No behavior change.
1. Vault-convention inventory → grammar v1 spec addendum here.
2. Grammar v1 nodes (model+parse+ranges+render+round-trip locks).
3. Caret-line syntax reveal inside the edited block (styled-mirror
   technique; spec §2.2).
4. Render extraction out of shell.rs when its churn settles.
5. paper/ztlkn adopt; ztlkn adds graph/backlinks OVER the model — never
   inside it.
