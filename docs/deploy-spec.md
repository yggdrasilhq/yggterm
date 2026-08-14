# The deploy spec — a deploy is not done until the RUNNING thing is the thing you built

**Status: LAW.** Owner-directed 2026-08-14, after a stale GUI cost him both sidebars
for twelve hours: *"we need a dev deploy spec that needs to be adhered since this
types of mistakes are common."*

⛔ **This is not a checklist about being careful. Every rule below is one measured
failure, and four of them happened on a single day** — including one to an agent
who had just shipped the fix it then failed to run.

---

## §0 THE ONE LAW

> **A deploy ends when you have proved, BY IDENTITY, that the process now serving
> the user is the artefact you built. Everything before that is preparation.**

"Pushed", "installed", "the file on disk has it", "the version says 3.0.154" and
"the tests pass" are all **preparation**. None of them is the proof.

⚠ **"On disk is not running" was the entire answer to three separate incidents in
one day.** It is the single most expensive assumption in this project.

---

## §1 IDENTITY, NEVER VERSION

A version string is a **claim by a binary about itself**. It cannot tell you which
binary answered, and it is wrong in at least three ways here:

- ⛔ **`--version` is a pure builtin exempt from the exec handoff**, so it reports
  the binary you *typed* while a different one runs your command.
- ⛔ **Two builds can carry the same version.** Parallel lanes allocate the same
  number, and a pre-rebase deploy lands over yours.
- ⛔ **A version match across a daemon boundary proves nothing** — an older daemon
  that still owns a session answers the proxied request with its own compiled-in
  code, so a "deployed" fix is inert for every session it owns.

**⇒ Compare bytes, on the running process:**

```sh
md5sum /proc/<pid>/exe            # what is ACTUALLY executing
md5sum <the artefact you built>   # what you meant to ship
readlink /proc/<pid>/exe          # ⛔ must NOT end in "(deleted)"
```

⛔ **`(deleted)` IS A HARD FAIL AND ITS OWN CLASS.** It means the process is running
a binary that no longer exists on disk: it can never be updated, never be compared
against a file, and no amount of re-installing will change it. **Only killing it
will.** Treat it as "this process is unreachable by any deploy".

---

## §2 RETIRE THE OLD INSTANCE — THE RULE THAT COST THE MOST

**Measured 2026-08-14.** The GUI host carried **two** `yggterm` processes:

| pid | started | binary | cost |
|---|---|---|---|
| 4004668 | **Thu 22:30** | `…/yggterm` **(deleted)** | **29.2 % of a core, sustained 12 h 24 m**, 252 MB |
| (new) | Fri 10:54 | current, matches disk | — |

The owner had restarted **repeatedly** to fix the jank. **Every restart spawned a
new GUI while the twelve-hour-old one kept owning his window.** He lost the
Session Metadata panel and got dropped glyphs for twelve hours, and his reported
duration matched that process's age to the minute.

⛔ **A user who restarts to escape a problem, and thereby adds a second broken
instance, CANNOT escape by himself.** That is the defect this section exists for.

**⇒ Rules:**
1. **Count instances before and after.** More after than before is a failed deploy,
   even if the new one is perfect.
2. **A deploy that cannot retire the previous instance is not a deploy** — it is an
   addition. Say so out loud rather than reporting success.
3. ⛔ **After any kill, VERIFY a healthy replacement is up, and screenshot it.**
   Leaving the user with zero is worse than leaving him with one stale window.
4. ⚖ **Restarting the GUI needs no permission** — the daemon owns every PTY, so
   nothing is lost and he is back in seconds. Deferring it out of politeness is
   the mistake, not the restart. **But never restart pointlessly, and never leave
   him with none.**

---

## §3 A LONG-LIVED PROCESS LOADED ITS CODE ONCE

An interpreter reads its source **at start**. A daemon links its libraries **at
start**. Updating the file underneath a running process changes **nothing** until
it restarts.

⭐ **The cruellest form, measured twice in one day:** the process's own checkout
**already contained the fix**. Everything an observer could inspect — the file, the
git log, the branch — said "deployed". Only the process was old.

**⇒ After shipping to any long-lived process, ask it to prove the fix is live by
producing an ARTEFACT ONLY THE NEW CODE CAN PRODUCE.** Not a version, not a file
hash — a behaviour. A new state file appearing, a new log line, a new field in an
answer. If the new code cannot produce a distinguishing artefact, add one.

---

## §4 THE CHECKOUT IS NOT THE FLEET

**Measured 2026-08-14, same day, on a fix that had been pushed and reported ✅:**

| host | `~/gh/yggterm` was |
|---|---|
| dev | 24 commits behind |
| second host | **317 commits behind** |
| third host | **317 commits behind** |

⛔ **Nothing in the tool-invocation path pulls.** Every row on those hosts was
running weeks-old fleet tooling, including the one warning built to detect exactly
that — see §6.

**⇒ Sweep all hosts, and only fast-forward what is safe:**

```sh
d=$(git status --porcelain | wc -l); a=$(git rev-list --count origin/main..HEAD)
[ "$d" = 0 ] && [ "$a" = 0 ] && git merge --ff-only origin/main
```

⛔ **REFUSE rather than force if either is non-zero.** A peer's `reset --hard` on a
shared checkout cost another session ~1200 lines of work in one evening.
⚠ **And fast-forwarding a checkout does NOT rebuild or restart anything.** A GUI
that has been up for hours keeps running its old binary; §2 and §3 still apply.

---

## §4b THE FLEET IS NOT THE PATH — INSTALL WHERE THE CONSUMER LOOKS

**Measured 2026-08-14, on a fix whose whole purpose was to unblock a script.**
A restored CLI verb was installed to `~/.yggterm/bin/yggterm-headless` and
`~/.local/bin/yggterm` on all three hosts, byte-compared against the artefact on
each, and reported deployed by identity. Every one of those checks was true.

⛔ **The consumer runs `~/.local/bin/yggterm-headless`, which was never
installed to.** It still answered `unsupported app terminal action` on all three
hosts — and on one host it was a *different* stale build from the other two, so
even the staleness was not uniform.

⇒ **§0 says "the process now serving the user". When the user is a SCRIPT, the
process serving it is whatever its hardcoded path resolves to** — not the path
you think of as the install location, and not the one `which` answers.

- ⭐ **Read the consumer for its path before installing, and verify from THAT
  path afterwards.** One `grep` of the caller is the whole check:
  `grep -oE '[~$][A-Za-z_/.{}]*/yggterm[a-z-]*' <the script>`.
- ⚠ **Two copies of a binary on one host is the norm here, not an anomaly** —
  `~/.local/bin` and `~/.yggterm/bin` both exist and drift independently. Sweep
  every copy, on every host, and print the hashes side by side; a census that
  shows two different stale hashes is telling you the deploy has been partial
  for a while.
- ⛔ **A verb answering correctly from a binary nobody invokes is not a fix.**
  This one read as fully proven — identity checked, hashes matched, the verb
  demonstrated live — while the blocked script stayed blocked.

---

## §5 ORDER OF OPERATIONS

1. **Build where the toolchain is** (`dev`), **deploy to where the thing runs.**
   Building on the wrong host deploys to nobody — that mistake once left a live app
   seven weeks stale.
2. **Check the CURRENT running version before deploying**, so you can tell your
   change from someone else's.
3. Ship. **Then §1 (identity), §2 (instance count), §3 (distinguishing artefact).**
4. ⚠ **A deploy re-resumes sessions on fresh PTYs, and that window is itself a
   visual defect.** Never declare a post-deploy surface healthy without looking,
   and never "deploy to measure" a symptom the deploy causes.

---

## §6 THE SELF-HOSTED CHECK CANNOT BE THE ONLY DEFENCE

⭐⭐ **The sharpest finding of the day, and it generalises past this repo.**

A staleness warning exists to tell a row its copy of a tool is out of date. It
lives **in that tool**. So it runs from the stale checkout — and on the day it
mattered it was broken (a shell function called fourteen lines before it was
defined, so all five of its lines died as `command not found` and it still exited
0).

> **The instrument that would report the staleness LIVES ON THE STALE COPY. The
> copies that most need the warning are structurally guaranteed not to render it.**

**⇒ Any check on the health of a distributed tool must ALSO run somewhere central
and always-current** — a watcher, a hook, a CI step. That is where a check about
staleness belongs, because it is the one place that cannot itself be stale.

⚠ And a corollary paid for the same day: **a check whose prescribed remedy it
cannot observe becomes noise, and the noise is what hides the real finding.**

---

## §7 WHAT TO SAY WHEN IT IS NOT PROVEN

⛔ **Never write "shipped", "deployed" or "live" without the §1 proof.** The user
reads those words as "I can use it now".

Say the true thing instead, which is always available:

> *"Code is on disk and pushed; the running GUI is still pid N on the previous
> build, so this activates on the next restart."*

That sentence costs nothing and is worth more than a false "shipped", because the
next person knows exactly which half is done.
