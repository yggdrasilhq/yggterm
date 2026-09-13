<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Worked examples, mini-webapps

**edition 2026-09-07 · rev 1**

Design prose is not proof. This page rebuilds the canonical surfaces as real
schemas: the widgets below the prose are painted by the host with the same
code every app inherits. Open this page, screenshot it, and put your own
surface beside it.

## Specimen 1, the Live Sessions anatomy

The main tree's rows are the product's most-watched surface. Its anatomy,
rebuilt below as a filter plus three rows:

- A **status column** flush left, the dots form one unbroken vertical line
  down the rail. A kink at any row means the zones (gutter / content) were
  never separated.
- A **title track that owns the whole row** at rest. Actions appear on hover,
  selection, or focus-within; the title ellipsizes to make room; nothing
  floats behind the verbs.
- A **durability dot per row**: green = survives the app, blue = lives only
  while it does, empty slot = nothing to say.
- A **count badge** where a group carries one, on the row's trailing edge,
  not bolted to the title.

What wrong looks like (all measured defects, each shipped once): dots in two
columns because a disclosure control sat at the leading edge; a close button
on a frosted chip over the title; an 11px rail font beside a 12px tree font;
titles shortened at rest by invisible action buttons.

## Specimen 2, the partitioned sidebar

The yedit files rail is the reference implementation of *partitioning with
intent*. Rebuilt below:

1. **Knobs, the top partition, ≤ 30%**: a section card with its toggles.
   Small, quiet, and few.
2. **FILES, the majority**: the list, unbroken, with a section heading in
   the text colour and a leading `+` verb.
3. **The status line, pinned at the bottom**: counts and modes, behind a
   separator, never scrolling away.

The proportion is the design. A top partition that creeps past a third of
the rail has stopped being knobs and become a second app living above the
list.

## How to compare, the loop

1. `ydesign` inside yggterm; open this page in Examples mode.
2. Screenshot it, the shadow client's own compositor capture, or
   `server app screenshot` with the faithful backend when focus allows.
3. Put the two images side by side. Compare, in order: **partitions →
   heading voice → row grid → status column → title track → field skin**.
4. Name the first divergence. Fix it **at the component layer**, in the
   shared engine, the shared style function, or the schema vocabulary, not
   in the call site that hurt.
5. Re-shoot both. The pair is the acceptance artefact.

A divergence you cannot name is still information: it usually means a
partition is missing rather than a pixel is off. Go back to the structure
before touching spacing.

### The first catch, this page's own defect

The first build of this appendix appended rail-style controls (a search box,
toggles, section headers) after the prose. The pixel proof caught them
instantly: they painted into the document surface's TOP BAR, squeezed into a
horizontal strip above the page. Rule learned, now in the Component gallery:
**on a document surface the body flow takes `markdown`, `list-row` and
editors only, everything else is a rail sight.** The specimens below are
body widgets because of that catch, which is the whole method working: build,
shoot, name the divergence, fix at the layer, re-shoot.

## Extending this page

When a surface earns canonical status, a second app copies its shape, it
gets a specimen here, built from real widgets, with its "what wrong looks
like" list. A pattern that exists only as prose in two repos is a pattern
waiting to drift.
