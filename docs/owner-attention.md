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

- **Where should the leak gate's own source live?** It is currently tracked in **no repository at
  all** — a loose file in `~/.local/bin`, replicated newest-wins across three hosts, unversioned and
  unreviewed, while being the thing that stops private data reaching public GitHub. A weakening edit
  on any host would win that race, spread silently, and every later push would go out unguarded
  while still printing its reassuring pass. ⛔ **It cannot go in this repo: it was tried and the
  guard refused its own push, correctly** — its source must know which remotes are private in order
  to decide when to scan, so that knowledge is in the code. **Recommendation: give it a private
  Forgejo repo** and keep the wordlist where it already is, outside every repo. **Done meanwhile:**
  the tracked installer is landed (`scripts/install-privacy-guard.sh`), so the untrackable
  `.git/hooks` shim is at least generated from something versioned, and the gate is unchanged and
  working — it refused a real push tonight. **To reverse:** delete one repo.
  *Meanwhile:* nothing waits on this; the relay installs the hook from the tracked installer.

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

- **The response-layer rule, or five separate patches?** — five verbs report the
  request rather than the effect, and he framed the fix's SHAPE as the open
  question. → yggterm `docs/pending-bugs.md` § *FIVE VERBS REPORT THE REQUEST,
  NOT THE EFFECT* (`Status: AWAITING A DECISION`).
  *Meanwhile:* the relay is fixing them one at a time in the pattern the rule
  would generalise, so either answer is cheaper afterwards, not dearer.

- **Should a CLICK be allowed to start a daemon swap?** The swap that cost 55
  PTYs on 2026-08-09 was begun by him clicking a row, and the settled relay-gate
  design makes a swap an appointment at a relay boundary — which a click is not.
  → yggterm `docs/pending-bugs.md` § *A DAEMON SERVES ONE REQUEST AT A TIME*.
  *Meanwhile:* 3.0.80 already takes the wait off the click (first paint at
  +0.74 s instead of +18.5 s), so nothing is slow while this waits; the question
  is only whether the upgrade should still be kicked at all.

- **Do we drain the 27 pre-3.0.90 daemons on `dev` (and 7 on the GUI host), or
  let them age out?** The self-retire ships in 3.0.90+ and is live-proven, but
  those daemons are older binaries that will never run it, and between them they
  are the bulk of the measured 8.12 cores. Draining means terminating daemons that
  still own live rows — his sidebar, his call. → yggterm `docs/pending-bugs.md`
  § *"I CANNOT USE YGGTERM. IT IS SO JANK"*, item 4, and § *FIVE PRE-3.0 DAEMONS
  STILL WALK THE WHOLE TRANSCRIPT CORPUS* for the five that are measurably
  costing him now and are unreachable from the current GUI.
  ⭐ **PRICED 2026-08-14, so the question is now cheap to answer.** The saving is
  **2.64–3.35 cores** across 14 legacy daemons, and it is real rather than
  relocated: their sessions are idle, so nothing is being served by them. But
  **about half of it is behind a door the machinery cannot open** — the PTY
  handoff does not exist below **3.0.32**, so the **8 daemons older than that
  (~1.6 cores) hold 17 sessions that can only leave by ENDING**, and 14 of those
  are plain shells. **The one question that is actually his: may those old plain
  shells be closed?** Everything else drains on its own once 3.0.155 lands.
  *Meanwhile:* the pile can no longer GROW — the count held flat across a version
  bump for the first time — so it shrinks on its own as sessions end, and nothing
  is being terminated while this waits.
  ⭐ **UPDATED 2026-08-14 — the saving is now MEASURED and the reason is not what
  this campaign said it was.** Daemon cost is `N_reachable x a ~0.2-core floor`,
  where the floor is the price of **being reachable while holding a row
  inventory**. On `dev`: **14 legacy daemons hold 2.6–3.3 cores while their
  process subtrees are IDLE**, the 2 current daemons cost 0.4 while carrying the
  real work, and daemons that lost their socket cost **exactly 0.000**. Per daemon
  the legacy and current groups are **indistinguishable (0.94–1.05x)** — the pile
  is expensive because it is **numerous and reachable**, not because it is old or
  busy. ⇒ **Draining reclaims 2.6–3.3 cores and the saving will actually arrive**
  (earlier guidance in this campaign said it would not; that was wrong and is
  withdrawn). Derivation: `docs/idle-cost-model.md` §6j-9.
  ⚠ **The one item that is genuinely his:** the cheapest first target is the
  2.12.14 daemon — alone in the population it bursts 4.15x under quiet conditions
  and re-reads ~69 MB/s of page cache with three IDLE shells attached. **Those
  three are plain shells, which the migration path refuses**, so retiring it is a
  question about three shells he may or may not want back, not a migration
  problem anyone can solve for him.

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

- **Whether the relay moves up the queue on the strength of ADR-0002 §9.** §3
  ruled "relay is v2" when the phone was only ever going to carry a terminal, at
  kilobytes a second. §9 changed the subject: it was requested for the phone to be a
  remote SURFACE for libyggterm apps (his example — guihost runs ychrome on Khan
  Academy, he solves on the phone, guihost's page updates), and a streamed surface
  is not a terminal's bandwidth. He named it a relay priority himself, which is
  what makes this his to re-rank rather than the campaign's.
  *Meanwhile:* the transport work that §9 would build on is the same either way
  — protocol extraction, then the facade — so the lane is not idle while this
  sits. ⚠ And one thing must be said plainly when he answers: §9 promotes the
  constitution's **unsolved per-viewer-geometry problem** from blocker to
  feature. The read-only pinned shadow viewer does not solve it, it dodges it.

- **May a user deliberately run TWO yggterm windows of the same live build on one
  display?** Today the GUI keeps them: `should_retire_superseded_client` retires an
  older same-display client only when its executable differs, and a test asserts the
  same-build case is kept. That decision is now load-bearing, because the crash-plus-
  restart tangle produces same-build duplicates and the user cannot escape one by
  restarting. **Recommendation: no — one window per display, and a second launch
  either refuses or replaces.** It is his call because "I want two windows side by
  side" is a product answer that no reading of the code can supply.
  *Meanwhile:* the unambiguous half is fixed and needs no ruling — a GUI running a
  **deleted** binary is now retired on sight, which is the case that cost the day.
  The same-build question is filed in [`pending-bugs.md`](pending-bugs.md) and
  nothing waits on it. **Reversing this is one predicate**, not a redesign.

## Third parties only he can chase

- **Google Play identity verification is his, and it gates every listing** — the
  developer account exists and is paid (ID `7834754661078735260`), but Google
  locks *Create app* until three checks pass: government ID, access to an Android
  device, and the contact phone. Only he can present a document. Decided
  2026-08-08: **passport, not Aadhaar** (Aadhaar is not Google's to hold, and the
  passport survives the US move) — with one thing to check first, that the
  passport's NAME matches the payments profile, since Google verifies against
  documents. → his private owner-window note for that lane (path deliberately not
  named here: this repo is public).
  *Meanwhile:* nothing in either repo waits on it; the campaign has no Play work
  that verification unblocks.

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

## ⛔ Five rows are parked on the plan-limit dialog, and the watchdog is right not to touch them

**What is needed from him: may the watchdog dismiss that dialog by explicitly
selecting "stop and wait for the reset", after reading the option off the screen?**

After the quota window, five rows are sitting on the CLI's three-way plan-limit
prompt — stop and wait, switch to a team account, or use API billing. The
watchdog now correctly reaches them (a quota message whose reset has passed is no
longer skipped forever) and then **refuses to type**, because a bare Enter
*selects whatever option is highlighted* and that could change billing on his
account. Measured live: five of six boot attempts returned
`refused-choice-prompt`, which is the guard working exactly as specified.

⇒ **The refusal is correct and must not be weakened. The consequence is that
those rows stay parked until a human dismisses the prompt.** Nothing is lost —
every lane had committed and pushed — but the seats are idle meanwhile.

**Recommendation: authorise a screen-verified dismissal, not a blind one.** Read
the prompt, locate the option whose text says stop-and-wait, navigate to it
explicitly, and refuse outright if the text cannot be read or the option cannot
be identified. That keeps the standing rule — never send a bare Enter into a
prompt this watchdog did not put there — while ending the park. ⛔ It is his call
and not the relay's, because the failure mode is a billing change on his account
made by a timer, which is categorically different from a wasted boot.

**Done meanwhile:** nothing is typed into any of them, by us or by the watchdog;
the rows are safe and resume the moment the prompt is dismissed by hand. The
half that was ours — a row parked on an *expired* quota message being skipped
forever — is fixed and live. → `docs/pending-bugs.md`, the supervision-watcher
entry, and the campaign memory door on the quota-hold deadlock.
**To reverse:** one flag; no data change either way.

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
