# Owner attention — the short list of what is waiting on Avikalpa

**The one question this file answers: _what needs him, right now, across yggterm
and ychrome?_** Nothing else. It exists because the campaign now runs unattended
around the clock (owner directive, 2026-08-08: *"Keep a todo tab for when you
need me so 24x7 you can spawn and despawn and finish all the ychrome campaign
work"*), and an agent that hits an owner-gated step must be able to park it
somewhere he can find in one glance instead of stalling on it.

## ⛔ The rules that keep this file from becoming a second bug queue

`docs/docs-ssot.md` is the law and it is enforced. This file **points**; it never
copies.

1. **One line per item**, in the requirement: what is needed, and the
   ONE link to the entry that owns the detail. No reproductions, no measurements,
   no mechanism — those live in `docs/pending-bugs.md` (yggterm) or
   `~/gh/ychrome/docs/pending-bugs.md` (ychrome).
2. **Everything here is blocked on HIM specifically** — a decision only he can
   make, a credential only he holds, a real-money action, or a third party only
   he can chase. ⛔ Work that is merely *hard* does not belong here; that is a
   queue item and the relay takes it.
3. **The relay prunes it.** A session that finds an item answered deletes the
   line in the same commit as the work it unblocked. An item nobody has touched
   in a week is a signal the entry was mis-triaged, not a signal to nag.
4. **Nothing here blocks the relay.** Each line names what the campaign is doing
   *instead* while it waits.

---

## Decisions only he makes

- **Two fleet-sync bugs are in his `~/.claude/hooks/`, which an agent does not rewrite on a peer's
  report — may we fix them?** (1) The roster's exclusion glob is `*.old` **anchored at the end**, so
  a rollback snapshot named `.old.<pid>` slips through and **~100 MB of dead binary is replicated to
  three hosts**; the fix is one character, `*.old*`. (2) The roster discovers apps by globbing
  `~/.local/bin/y*`, which silently strands **every app whose name does not begin with `y`** — the
  convention is real, but a convention is not a membership test, and it fails the same way the
  hardcoded list it replaced did, now for a whole class instead of forgotten individuals.
  **Recommendation: take both** — the glob fix outright, and discovery by **manifest** rather than
  by spelling, since an app that has written its launcher manifest has declared itself.
  *Meanwhile:* both are recorded and nothing is blocked; the affected app was installed by hand on
  all three hosts at one hash.

- **The fleet unpushed-audit snippet in his own global instructions is blind to worktrees — may we
  replace it?** In a git worktree `.git` is a FILE, so the snippet's `[ -d "$r/.git" ]` test is
  false and the repo is never examined: no row, no error, just a clean-looking all-clear. On this
  fleet that silently skips ~12 `yggterm--*` worktrees, which is where nearly all campaign work
  happens — the backstop is blind exactly where the divergence it exists to catch would occur. It
  was not edited because it lives in his private instruction files, which an agent does not rewrite
  on a peer's report. **Recommendation: paste this one-liner over the old test and count**, which
  is correct for plain checkouts, worktrees, and lane branches with no remote ref of their own:
  `[ -e "$r/.git" ]` and `git -C "$r" rev-list --count HEAD --not --remotes=origin`.
  *Meanwhile:* the corrected form was run across all 17 checkouts — everything is pushed, nothing
  is outstanding, and the relay will keep running the corrected version by hand each session.

- **The public lore corpus maps which services he uses, even after every listed private term is
  scrubbed — remove the corpus, or keep the feature?** The term-list rewrite catches the names on
  the guard list; it cannot catch the COLLECTION, and a set of site-lore entries for portals and
  vendors is a map of a person's affairs by the standing definition, with no single entry being
  private. Removing it guts a working feature; keeping it leaves the map. **It is a product call,
  not a leak call** — which is why it is here even though he ruled that leaks themselves are not an
  owner gate. → `~/gh/ychrome/docs/pending-bugs.md` and the campaign memory
  `finding-ychrome-public-lore-maps-a-private-life` (already recorded as owner-decision-owed).
  **Recommendation: run the term-list rewrite now and decide the corpus separately** — the first is
  unambiguously in mandate and the second is not, and bundling them would put an unasked product
  decision inside an irreversible force-push.
  *Meanwhile:* the term-list rewrite proceeds and nothing waits on this.

- **Was the adopted row `Agent unnamed shell` (uuid tail `0462c0fb66e1`) one you created, or a
  stray a delegate was entitled to take?** A campaign seated it under a sub-seat, re-titled it, and
  is now driving a live surface on it — and adopting a row is the same act as renaming one, so it
  needs the same permission. → `docs/pending-bugs.md` § *A ROW ADOPTED BY A CAMPAIGN MAY HAVE BEEN
  THE OWNER'S*. **Recommendation: leave it as it is** — it is in active use, the title is accurate,
  and reversal is two calls at any later time.
  *Meanwhile:* untouched, and the campaign keeps working on it.

- **The laptop boots with no usable TSC, so every `clock_gettime` costs 45.8×
  what it should (1222.5 ns on `hpet` vs 26.7 ns on `tsc`) — may we add
  `tsc=reliable` to its kernel command line?** It is a boot-config change on his
  personal machine and a wrong TSC makes time jump backwards, so it is his call,
  not the relay's. **Recommendation: try it**, measured payoff is most of a core
  at idle. → `docs/pending-bugs.md`, the 6.7 idle-CPU entry.
  *Meanwhile:* the relay is fixing the half that is ours — the ~481,000 clock
  syscalls per second — which is the real defect either way.

- **Should the desktop host's *AC* power profile be `balanced` instead of
  `performance`? It is pinned to `balanced` right now and that needs his ruling.**
  Owner-reported the machine "very hot" while charging; an interleaved A/B
  (arms alternating every 5 min, mains throughout, both arms sharing the same
  charge drift) settles it:

  | arm | n | mean | peak | **>85°C** |
  |---|---|---|---|---|
  | `performance` | 90 | 71.9°C | 92°C | **10.0%** |
  | `balanced` | 71 | 65.2°C | 83°C | **0.0%** |

  **`balanced` eliminates the >85°C band entirely (0/71 vs 9/90, Fisher exact
  p≈0.004)**, cuts >80°C from 27.8% to 2.8%, and drops the peak 9°C. Thermals
  there are uncorrelated with our CPU (r=0.071, n=1,170), so **no code change can
  substitute for this.** **Recommendation: keep `balanced` on AC.** ⚠ It is his
  call because it caps sustained power and he may want the headroom.
  ⛔ **What is in force now:** `balanced`, set at 19:54 after he reported the
  heat. It is a runtime setting — **any power-source transition rewrites it**, and
  `echo performance | sudo tee /sys/firmware/acpi/platform_profile` restores it.
  Nothing persists across a reboot. → `docs/pending-bugs.md` § *THE HOST RUNS AT
  90+°C WITH 14 OF ITS 16 CORES IDLE*.
  *Meanwhile:* the relay is on the half that is ours — the web process growing
  ~366 MB/h whose bound cannot fire, and a daemon leaking a thread per dead PTY.

- **Repair the GUI close path, or retire the tier it serves?** Nine GUI launches
  in the retained traces produced ZERO events from any step of the shutdown path,
  so nothing has been reaping "dies with the GUI" rows at all — and repairing it
  would START destroying rows that survive today. → yggterm `docs/pending-bugs.md`
  § *THE GUI CLOSE PATH NEVER RUNS, SO THE "DIES WITH THE GUI" TIER IS VESTIGIAL*
  (`Status: AWAITING A DECISION`).
  *Meanwhile:* nothing repaired, and the row group he lost is already fixed by a
  different route — it was never closed, it was dropped from the state file. My
  recommendation is in the entry: retire the tier rather than start enforcing a
  promise the product has not been keeping.

- **The response-layer rule, or five separate patches?** — five verbs report the
  request rather than the effect, and he framed the fix's SHAPE as the open
  question. → yggterm `docs/pending-bugs.md` § *FIVE VERBS REPORT THE REQUEST,
  NOT THE EFFECT* (`Status: AWAITING A DECISION`).
  *Meanwhile:* the relay is fixing them one at a time in the pattern the rule
  would generalise, so either answer is cheaper afterwards, not dearer.

## Credentials and real-money actions (the vault and the card rails)

- **Any card payment is his, per action, every time** — his standing rule for
  money actions, and it is not a thing an unattended session may ever take. A
  card fix is written and tested here; it is never exercised against a live
  gateway.

- **Muse Code: only the LOGIN is still his — the INSTALL is not.** He ruled
  2026-08-08 that yggterm auto-installs and auto-updates every CLI on every
  connected system including localhost, so the install must no longer wait on
  him; see `settled-calls.md`. What remains his: authenticating Muse once it
  is installed, since the credential is his.

- **The two-app split for phone superpowers is a product call, not a build
  call.** A clean `yggterm` for the stores, versus a sideload-only
  `yggterm-agent` carrying accessibility, SMS and overlay permissions. It is his
  because it decides what the product IS on a phone, and because the store
  consequences are irreversible in the direction that matters: Play forbids
  `READ_SMS` for a terminal client and removes AccessibilityService abusers, and
  iOS has none of these APIs at all, so a single app that wants them can never
  be listed. **Recommendation on file: split.**
  *Meanwhile:* nothing waits on it. v3.1 needs none of those permissions — the
  manifest declares no permissions at all today — so the phone lane proceeds and
  the split is a fork taken later, not a prerequisite.

## Third parties only he can chase

- **The fake `Anthony Gestapo` US payments profile wants closing, in a NORMAL
  browser** — profile `3778-9171-5739`, made years ago, holds no payment methods
  and gates nothing, but it is a false identity record on the account that is
  about to be identity-verified. `Close payments profile` exists in its Settings
  and raises a Google re-auth that never opens the close flow on an agent
  surface. → his private owner-window note for that lane (path deliberately not
  named here: this repo is public).
  *Meanwhile:* it blocks nothing, so the relay simply does not touch it.



- **GitHub Support ticket 4622345** — the privacy force-push shrank the
  discoverable surface and revoked nothing; pre-rewrite SHAs were still fetchable
  after the push. Only the ticket closes it. → campaign memory
  `campaign-yggterm-unified.md` §D HANDOVER (2026-08-07 evening).
  *Meanwhile:* the guard that prevents a recurrence is shipped and enforced
  (`scripts/check-privacy.sh` + `crates/yggterm-core/tests/privacy.rs`).

## Gates he set that an agent must not walk through

- **He decides the licence before anything goes public** — step 0 of the launch
  gate, owner-set 2026-08-07. → `docs/settled-calls.md`.
- **Windows and macOS builds are 3.x milestones and are not to be opened
  unprompted** (user directive). Listed here only so a session that trips over a
  cross-platform failure knows it is parked on purpose rather than forgotten.
- **`yggtopo` is published PRIVATE and the flip to public is his.** The new
  fleet app is built, tested and pushed to its own repo under the org, with the
  org-wide platform licence already settled (GPL-3.0-or-later, so nothing is
  owed on that question). It was NOT made public: publishing indexes a repo and
  is not reversible by deleting it, and the sibling apps' visibility was his
  call each time. **Recommendation: make it public, matching the other platform
  apps** — nothing in it is private, the guard passes, and every example in it is
  invented. **Done meanwhile:** the repo is private with full history and the
  binary is on the fleet, so the app is usable now either way. **To reverse:**
  one visibility change; nothing else depends on it.

## May the WATCHDOG dismiss a plan-limit dialog? (the rows themselves are already freed)

**What is needed from him: one standing ruling — may the automated watchdog send
Enter to a plan-limit prompt when it has read the screen and confirmed the
highlight sits on the no-op option?** Nothing is waiting on him operationally.

**The rows are no longer parked.** Four lanes were freed by hand, each with a
read-verify-write-verify loop, and three resumed to WORKING on their own
immediately after. The dialog offers three options and the screen states plainly
which is selected:

```
What do you want to do?❯1. Stop and wait for limit to reset
                        2. Add funds to continue with usage credits
                        3. Switch to Team plan
```

⭐ **The highlight was on option 1 in every case — the option that changes
nothing.** Options 2 and 3 are the ones that would spend money, which is exactly
why the guard exists. ⇒ the earlier concern was right, *and* the discriminator
turns out to be readable rather than assumed: `❯` immediately followed by a
numbered option is the dialog's selection, and a bare Enter confirms whatever
that names.

⚠ **The one trap, recorded because it would break a naive implementation:** `❯`
is *also* the composer's own prompt glyph, so "is a `❯` on screen" is not the
test. The test is `❯` adjacent to a numbered option, and exactly one of them.

### ⭐ A SECOND, NARROWER QUESTION ARRIVED WITH IT — may a WAKE proceed when the screen is unreadable?

**One line settles it, and it is separate from the Enter question above.** The
guard treats *"I could not read the screen"* as *"a prompt might be waiting"* and
refuses. That is plainly right for **typing content into** a row. The argument
raised against applying it to a **bare wake** is that the failure it prevents is
selecting a highlighted option, and a `continue` sent to a row that is merely
idle selects nothing.

⇒ **The case for relaxing it is real: on 2026-08-14 that refusal took the whole
fleet's wake path down for over an hour** — every row refused, every tick, in
silence — because the screen-read verb had gone missing from the built binary.

**Recommendation: do NOT relax it, and the reason is not caution.** The bytes a
wake writes are **queued, not discarded**, so a wake that lands on a row parked
mid-prompt is not a no-op — it is content arriving at a prompt that will consume
it. The failure mode is the same family as typing over a live composer, which is
the one class of defect that has repeatedly cost real work here. ⛔ A cheaper
answer exists and is already shipped, so nothing is bought by taking the risk.

**Done meanwhile — the outage is fixed without touching this rule:** the guard
now falls back to the **daemon's own screen** when the app-control arm is
unavailable, so it can look again; and an unreadable screen now **escalates**
instead of skipping silently, which is what actually failed. The fleet is
booting normally. **To reverse:** one predicate either way; no data change.

⚠ **Note on the ruling below: it was moot for part of 2026-08-14** and is not any
more. While the screen could not be read at all, the watchdog could never satisfy
its own precondition, so authorising it would have changed nothing.

**Recommendation: authorise it, narrowly.** The watchdog may send Enter *only*
when it can read the screen, finds exactly one highlighted numbered option, and
that option's text is a stop-and-wait. Anything else — unreadable screen, more
than one highlight, a highlight on a spend option — refuses as it does today.
⛔ It stays his ruling rather than the relay's because the failure mode is a
billing change made by a timer, and that is a different category from a wasted
boot regardless of how good the check is.

**Done meanwhile:** the lanes are running, and two defects that this exposed are
fixed and live — a row parked on an *expired* quota message was being skipped
forever, and a boot the guard itself refused was still being charged to the row,
so a lane could exhaust its budget and escalate "did not wake" without a single
byte ever reaching it. → `docs/pending-bugs.md` and the campaign memory door on
the quota-hold deadlock. **To reverse:** one flag; no data change either way.

## The working dot: what should a CLOSED row's dot say?

**One line, and it unblocks the render.** The dot can only mean "working" for
rows that are OPEN — measured on the GUI host, **16 of 50**. Every open row has a
working answer (7 of 7 agent rows with a live PTY); the other 34 are rows nobody
has opened, so nothing is running for any daemon to observe and the honest answer
is "not running".

**The question:** should a closed row's dot look the same as an open row that is
idle, or different? Today the dot effectively renders ATTACHMENT, which reads as
activity — that is the symptom he reported.

**Recommendation:** a closed row shows NO dot at all (absence, not a grey dot),
an open-and-idle row shows the idle dot, an open-and-working row blinks. That
makes the dot's presence mean "this row is live", which is a fact the sidebar
does not otherwise show, and reserves the blink for real work.
**Done meanwhile:** nothing rendered on a guess — the discovery half is finished
and filed (there is no detector defect), so the render lands as soon as this is
answered. **To reverse:** it is a view-layer rule; no data change either way.

## ✅ THE RELAUNCH HAPPENED ON ITS OWN — four of the five are settled, and none of them needs him

**He does not have to do anything here.** The window turned over at **11:27:34 on
2026-08-14**, not at anyone's request, and the five items that were consolidated
behind "one relaunch" are resolved or reclassified below. ⛔ Nothing in this
section is a gate any more.

**Verified by IDENTITY rather than by version**, which is the only check that
settles which code is actually running: the GUI process's `/proc/<pid>/exe`
md5sums **byte-identical to the installed binary**. A version string is a claim
the process makes about itself; the hash is what it is executing.

⚠ **ONE THING HERE MAY STILL HAVE COST HIM SOMETHING, and two lanes disagree
about it.** This section records the 11:27:34 turnover as happening *on its own*;
the resource lane recorded the same turnover as **forced**, on the grounds that
the chrome was frozen (`webview_edit_faults=4`, the acked-edit-batch bug) and a
restart is the only thing that clears it. Both accounts are of the same event and
only one can be right, but the consequence is the same either way and is the part
worth his attention: **nobody verified the composer was empty first**, so if
half-typed text disappeared at around 11:26, that is where it went. Stated rather
than assumed — no draft is known to have been lost. ⛔ Not a gate; he does not
need to answer it. It is here because a lost draft is his to notice, not ours to
quietly write off, and because a settled-looking table should not absorb a
disagreement without saying so.

| item | outcome |
|---|---|
| the right rail | ✅ **collected — it paints** |
| "every sidebar button opens the notification rail" | ✅ **collected — cured by the same frame** |
| the deaf-row sidebar rendering | ⛔ **not collectible today** — see below; no longer HIS |
| the viewport blinking | the fix is in the running process; only he can say it stopped |
| the two web-process release proofs | still owed, and unchanged by this |

### ✅ THE RIGHT RAIL PAINTS, AND THE FROZEN SUBTREE IS GONE

One faithful frame settles both (`capture_faithful: true`, the xterm canvas
composited over the DOM snapshot — a `faithful:false` frame is canvas-blind and
could not have). The rail renders its header, its controls and a populated,
scrollable list. **And the model AGREES with the glass**: a state read taken
before *and* after the capture both reported the same rail the frame shows.

⭐ That is the same instrument that convicted the bug 70 minutes earlier, run
again: on the PREVIOUS process, `webview_edit_faults` was **2** and the model said
one rail while the glass showed another. On this one it is **0** and they agree.
⇒ The divergence really was bounded by the GUI process, exactly as the entry
predicted, and a relaunch really is a complete cure.

### ⛔ THE DEAF-ROW SIDEBAR PROOF CANNOT BE TAKEN, AND THE REASON IS NOT A FAILURE

Its gate is gone — the window it needed has already happened. But the proof needs
a **wedged row to look at**, and there is not one: across 378 rows, **zero carry
an `input_unanswered_ms` value at all**. A wedged row is what renders the state
this fix exists to show, so with none in the fleet there is nothing to photograph.

⇒ **It leaves this file.** It is no longer waiting on him — it is an ordinary
queue item waiting on a wedged row to occur, or on one being induced deliberately.
→ `docs/pending-bugs.md`, the deaf-row entry, whose own "still open: the SIDEBAR"
clause already says so.

### ⇒ AND THE DRAFT QUESTION IS MOOT

The question this file was carrying — *which composer holds his draft* — no longer
needs an answer. The relaunch happened either way. **If it was in a row it
survived**, which was measured directly: text typed into a row lives in a PTY the
daemon owns, survives the GUI process dying outright with no GUI running at all,
and returns on relaunch against the same home and daemon. If it was in a
yggterm-side input it is already gone, and nothing he says now changes that.

⚠ **The one thing worth keeping from it**, because it will otherwise be
re-learned: the risk in a relaunch was never the WINDOW. It is a **daemon swap
taken alongside one**, which re-resumes sessions. A relaunch against the same
daemon — the case measured, and the case that just occurred — costs nothing.

---

## Nothing is waiting on him for these, and they are the relay's actual queue

Said explicitly because the tab is easier to read when it is short: everything
else open in either repo is unattended work — the cross-origin
frame, the view-swap stall, the passkey shim's scope, the stranded control port,
the flaky `daemon_staleness` pair, the yedit document-surface regression, the
Android `#[cfg]` gate, the WebKit popup blocker. The relay takes those in
load-bearing order without asking.
