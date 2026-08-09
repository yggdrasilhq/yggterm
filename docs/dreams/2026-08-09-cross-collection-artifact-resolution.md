DREAM from another campaign row, mid-wave, per the standing instruction to always send yggterm a
dream list. The test is not "is this a bug" but "did an agent hand-assemble this chore from
primitives and get it wrong?" — and here a human did it by hand and accepted a permanently
degraded document tree as the price.

## The defect

**A note cannot embed an artifact that lives outside its own container, and the human absorbs
the cost every day.**

Reference resolution in the incumbent tool is anchored at a container root. Two sibling
containers therefore cannot reference each other's files at all — not by wiki-style embed, and
not by a relative markdown link either, because the relative form escapes the sandbox and is
refused.

The shape of the failure, with an invented example:

```
notes/            <- container A, holds the prose
  topic/report.md      wants to embed ../../evidence/set-01/figure.png
evidence/         <- container B, holds the images
  set-01/figure.png
```

`report.md` cannot show `figure.png`. Both are markdown-adjacent files on one disk, owned by one
user, three directories apart.

**The consequence is not cosmetic.** The author must choose between *notes beside their evidence*
and *notes inside the graph where the links work*. Choosing the graph turns every embed into a
dead pointer, by hand, once per note, forever — and the layout of the archive gets redesigned
around a renderer limitation rather than around the work.

## Why this is a VERB, not a workaround

The workaround is a person rewriting embeds into inert links and re-deciding their own
information architecture to suit a tool. That is exactly the shape a dream is supposed to retire.

It also lands on the reason the replacement app exists at all: **if the replacement inherits the
sandbox, it does not replace anything.**

**The requirement, as a verb:** a note embeds or links an artifact **by relative path, regardless
of which container the artifact lives under**, and the renderer resolves and displays it.

## The framing correction, worth having before design starts

From a peer row that declined the work but improved the brief:

> *"This is a resolution-scope problem, not a rendering one. Reference resolution is anchored at
> a container root; the fix is deciding what an artifact reference resolves **against** —
> note-relative, container-relative, or a declared set of roots — and only then how it renders."*

Correct, and it reorders the work: the renderer is downstream of a scoping decision. It was filed
here originally as a rendering bug, which would have sent someone the wrong way.

## Settled by the owner, 2026-08-09 — FLATTEN

**One collection with sub-collections. A folder boundary is enough; no rigid container walls.**

⚠ **The constraint that must survive the flattening:** some sub-collections must never enter a
publication path. That has to be a **declared and enforceable property**, not an accident of
which directory a file happens to sit in. Separate containers only enforced it by accident — and
that accident is what cost the author the graph, which is what produced this dream.

## Scope note for whoever picks it up

The consuming app is a vault-shaped notes application built on `emd-renderer`, and its stated
design value is **flow**: two hot paths must stay cheap — *capture a thought*, and *find a thing
again*. Cross-container resolution earns its place precisely because the current failure forces
the decision *"which container does this belong to"* into both paths at once.

`emd-renderer` is the single source of truth for markdown across the ecosystem, so this lands
once and every consumer inherits it.

## Routing note, so the next row does not repeat the mistake

This was first sent to a row whose title suggested it owned the work; the session was in fact
working in an unrelated repository and correctly declined. **A row's title is not proof of what
it owns — check the working directory, not the name.** Filed here instead so it reaches the
campaign that owns the renderer rather than whoever was nearest.
