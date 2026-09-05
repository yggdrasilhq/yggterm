# Spec: bidirectional version convergence — daemons update the client too

**Status:** SPECCED 2026-08-20, implementation owed. Owner-ruled the same night the
gap cost the fleet: every cross-host surface degraded while the GUI ran 3.1.11
against 3.1.13 daemons for hours, with newer binaries already on disk beside it.

## The requirement

Upgrade convergence must work in BOTH directions:

1. **Client → daemons (exists today).** A newer client connecting to a host starts
   its own-version daemon; older daemons detect the successor (or the replaced
   disk binary) and drain into it via the preserving handoff. This direction is
   the deploy path and is already load-bearing.
2. **Daemons → client (MISSING — this spec).** When the running GUI is older than
   the newest daemon it talks to — local or on any connected remote machine — or
   older than the newest installed GUI binary on its own disk, the client must
   update itself and restart into the new version. The user must never have to
   notice a version skew, let alone resolve one by hand; version topology is
   bookkeeping, and any friction that leaks it to the user is a bug (see the
   constitution in `CLAUDE.md`).

## Why one direction was never enough

The one-way flow converges every DAEMON to the newest deploy but leaves each GUI
at whatever version it happened to launch. A long-lived GUI session therefore
drifts arbitrarily far behind its own fleet: cross-version wire mismatches
surface as ghost rows, "session removed" toasts over sessions that exist, and
remote writes refused blind — all of which read as data loss to the user while
being pure version skew.

## Design

- **Detection is a comparison the client already has both halves of.** The GUI
  knows its own version; `server daemons` and each remote machine's status
  already carry `server_version`. Add the third input: the version of the
  installed GUI binary on the client's own disk (the deploy lands it there
  before any daemon on that host is newer). Poll on the existing status cadence
  — no new traffic.
- **The restart is the existing self-restart, gated on the drafts guard.** The
  GUI already knows how to relaunch itself with layout and session state
  persisted. Before restarting: consult `server rows drafts` and its own
  composer — a restart over an unsent draft is data loss and must defer to the
  next clean boundary (same law as the daemon drain). Off a draft, restart
  without asking; a version-convergence restart is sanctioned by this spec.
- **Downgrade is refused.** Convergence is monotone: the client moves to the
  NEWEST version visible; it never restarts into an older binary no matter what
  a stale daemon advertises.
- **Binary availability is the deploy plane's job, not this spec's.** If the
  newest visible version has no matching binary on the client's disk, the client
  surfaces ONE loud notification naming the missing binary and the host that has
  it — it does not fetch software on its own. (Fleet binary distribution
  already exists; wiring yggterm's own binaries into it is a separate item.)
- **Every decision is traced.** `gui/version_convergence` events: skew detected
  (own, newest-seen, source), restart deferred (draft holder), restart taken,
  downgrade refused.

## Falsifier

Run a GUI at version N while a daemon at N+1 serves any connected machine and
the N+1 GUI binary is installed locally. Within one status cadence, with no
composer draft held, the GUI must restart itself into N+1 without user action,
and the trace must carry the decision. With a draft held, it must defer and say
so. With the newer binary absent, it must notify once and stay put.

## Tightening (2026-09-05): the convergence unit is the BUILD, not the version

Everything above reads "version" where it should read "build". At fleet
update velocity a same-version newer build is a NORMAL state — dev redeploys
many times a day and versions move far slower than builds — and the skew this
spec exists to kill occurs just as happily within one version number:

- **Detection compares build identity, not the version string.** The client
  already has its own build stamp and the daemon's `server_build_id`; the
  third input (the installed client binary on disk) comes with its own
  build identity. "Newer" is: higher version, OR same version with
  different bytes on disk (the different-bytes law the daemon already
  obeys — size+mtime latched, `/proc/<pid>/exe` compared).
- **Convergence is monotone in that same ordering.** A client never
  restarts into an older build no matter what a stale peer advertises, and a
  same-version OLDER on-disk build never triggers a restart.
- **Downgrade refusal and binary-availability notification are unchanged**;
  only the comparison beneath them changes.

**Implementation status:** direction 1 (client → daemons) is load-bearing.
Direction 2 (daemons → client) remains owed; the build-level comparison
above is the shape it must implement, and until it lands a long-lived GUI
drifts within its own version as well as across versions.
