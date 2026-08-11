DREAM from another campaign row, mid-wave. recorded 2026-08-09, relayed — this may direct
FUTURE work; it is not authority to undo anything already done.

## The ask

**Collaboration is a platform capability, not an application feature.** The owner's framing, near
verbatim:

> *"I want the collaboration system simple — each yggterm client touching an ssh remote is a
> 'collaborator'. The collaboration should live in the yggterm land. If yggterm can figure out
> simple collaboration like unix users, then **every libyggterm app automatically becomes
> collaborative**."*

It was stated it while deciding what a new libyggterm consumer should build, and the decision was:
**not this.** That app will consume a collaboration primitive rather than own one. The same holds
for every other consumer — an editor, a browser surface, a notes app — none of which should each
invent a concurrency story.

## ★ Why this is worth reading rather than filing

**It is the same problem the constitution already names as the highest-value work in this
project**, arriving from a different direction.

The constitution's clause, already written: the session/view contract assumes ONE viewer per
session; the shadow client only works because it was made read-only and pinned to the daemon's
PTY grid, which *dodges* the assumption rather than fixing it. **Genuine co-browse means two live
viewers of one session, with different window sizes** — precisely what the pin exists to avoid.

The application-side ask reduces to the same primitive: **two clients, one resource, per-viewer
state, permissions enforced by the OS rather than by the app.** Solve it once and the payoff is
double — the co-browse guarantee becomes true, and every libyggterm app becomes collaborative for
free.

⇒ **Do not scope these as two projects.** If multi-viewer is planned without the document-surface
case in view, it gets solved narrowly for terminals and every app rebuilds it.

## The design that already exists, and it relocates rather than dies

An arbiter model was drafted for the consuming app and agreed there before this call moved it
upstream. It is yggterm's inheritance now, and worth reading before designing fresh:

- **The daemon sequences, the client writes.** The daemon owns ordering, presence and broadcast.
  It is the SSOT for the *sequence of mutations*, never for the bytes.
- **Each client performs its own file IO as its own uid.** The kernel enforces permissions. The
  daemon needs no privilege, no service account, and never re-implements `access(2)`. Permissions
  are real rather than advisory.
- **Rejected there, with reasons:** a sole-writer daemon (permissions become advisory) and a
  seteuid login-service daemon (needs privilege).
- **Two edit classes, deliberately different.** Free-text gets optimistic concurrency — a
  `mtime:len` revision guard with an Overwrite/Reload prompt, not a CRDT. Structured edits flow
  as *operations* ("set field X on record Y"), so two clients never write the same bytes and
  there is nothing to merge.
- **Left open there:** lease loss (a client takes a write lease then dies), and whether presence
  needs anything beyond "who holds a lease".

Identity and access need no invention and are already settled ecosystem-wide: **identity is the
unix user, sharing is file permissions and groups, access is ssh** — reaching the host that holds
the resource *is* the authorization, exactly as in `docs/spec-decentralized-host-daemon.md`.

## A dogfood target just came free

The consuming app's previous build order led with a board view, chosen only because a board
exercised the arbiter on the smallest possible write — one metadata field. **With the arbiter
moved here, that rationale left with it**, and the app's ordering is being rewritten around its
actual purpose. If yggterm wants a smallest-possible-write target to dogfood the primitive, that
role is now unclaimed.

Related: `docs/dreams/2026-08-09-cross-collection-artifact-resolution.md`.
