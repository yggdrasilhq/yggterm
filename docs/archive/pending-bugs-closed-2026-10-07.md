# Closed 2026-10-07 — the privacy-debt checker-honesty + migration entry

## ⛔ [11.237] THE PRIVACY CHECKER'S PER-TERM HEAD-CAP HIDES CAMPAIGN-SCALE PRIVATE-NAME DEBT — EVERY "GREEN-LOOKING" RUN JUST PEELED THE NEXT SIX HITS, AND THE FULL INVENTORY IS ~1,400 HOME-PATH GREP HITS + 139 SHARED-LIST HITS OF ACCUMULATED LEAKAGE (measured 2026-10-06, sitting 14's full-suite gate)


Filed 2026-10-06 ~23:0x IST by the campaign seat (board interim
ACK-486d8f30ff). The [11.229](b) lane's full-workspace-suite gate surfaced
the privacy test RED on unfixed main — and fixing it peeled layer after
layer: scripts/check-privacy.sh prints at most 6 hits per private term
(12 for home paths), so each run "finds" only the next batch and the
depth was invisible. THE MEASURED INVENTORY (uncapped greps, sitting 14):
the home-path pattern alone matches ~1,393 lines repo-wide (the checker's
scoped tracked-files subset is smaller but the same order); the shared
guard list (~/.config/ygg-privacy/private-terms.txt) matches 139 lines;
one encoded in-repo term still matches 12. Oldest layers predate 2026-09
(the [11.183] fixtures, the [11.57]/[11.162] tombstone comments, the
[11.228] verbatim trace strings); the newest is the same week (the
dream-auto test fixtures, 2026-10-06).

WHY IT MATTERS: this repo is public; a term withheld from the checker's
output is still PUBLISHED in the repo bytes. The layered cap is a guard
that reports "6 left" forever — the same class as the [11.231]
exe-mismatch lie (an instrument that under-reports its own subject).

FIX SHAPE (two arms):
1. THE CHECKER: LANDED 2026-10-07 (sitting 16, main a6048f792d64) — every
   class reports its TOTAL unique offending-line count alongside the
   capped sample (`sample()` helper; the cap is display-only, never
   truth; exit semantics unchanged). THE HONEST SCOPED INVENTORY it
   reported on landing: home paths 12 unique lines; shared guard list
   136 unique lines (one withheld term; concentrated in comments/tests
   naming one private host). The filed ~1,400 figure was REPO-WIDE grep
   including the checker's EXCLUDED trees (docs/archive, vendored,
   assets) — the checker-scope debt is ~148 lines, still all of it
   published bytes; arm 2 owns it.
2. THE DEBT: MIGRATED 2026-10-07 (sitting 17, lane
   lane/priv/debt-migration, main ff93ec3708f8, deployed 10:10:14 IST,
   read-back verified): the private host name became the repo's established
   invented label `guihost` (deploy-fleet.sh's own placeholder) across
   docs/comments/tests (136 unique lines incl. 107 in this very queue);
   real home paths became /home/user (12 lines); and the four FUNCTIONAL
   sites resolve at runtime instead of spelling — ygg-ci.py's fleet default
   asks scripts/ygg-live-host.sh ($YGG_GUI_HOST override, loud when
   unresolved), connection_probe's peer candidates and usability_probe's
   birth-title prefixes come from ~/.ssh/config aliases (the ygg-live-host
   law: the GUI host is RESOLVED, never spelled), and the peer rows command
   uses $HOME. EFFECT PROOF: scripts/check-privacy.sh prints its all-clear
   on landed main (0 home-path + 0 shared-list lines; the guard's
   added-line scan passes — added lines are all label side). Suite: workspace 31 targets green + server-lib 1662/1 under full-suite load (the failure green in isolation on clean main AND the lane; full server-lib rerun 1663/0 idle); skill battery 22 green / 2 red pre-existing on clean main (filed [11.238]).
   Scope note: the checker's EXCLUDED trees (docs/archive etc.) still carry
   the repo-wide ~1,400 home-path grep lines — out of arm 2's scope by the
   checker's own tree list; that decision is now explicit here instead of
   invisible.

ATTACK ORDER: the checker arm first (it makes every later run honest),
then the debt in file-batches (crates/ tests -> docs/pending-bugs
historical entries -> tools/ -> .agents/skills), suite green between
batches.
