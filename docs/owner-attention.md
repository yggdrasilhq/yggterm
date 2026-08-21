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

- **May the all-CLI greeting run spend N agent sessions, and must it open rows on the live client?**
  **Recommendation: take it in a sandbox home against a headless daemon, scoped to the CLIs with a
  working backend** — nothing needs his screen unless a stage turns out to. **Done meanwhile:** four
  of its five stages are now answered by registry locks instead of by spawning, and the CLI-plane
  trace grammar makes the run readable when it is taken, so what is left to buy is much smaller than
  the original shape. Detail: `docs/pending-bugs.md` §"THE ALL-CLI GREETING RUN HAS NOT BEEN TAKEN".

- **Do we read the embedded controller to get a true fan RPM?** Every ACPI fan interface on the
  reference client was enumerated and sampled and all of them are stubs — there is no tachometer to
  read, which confirms rather than contradicts the "fan speed has to be interpolated" reading. The
  only remaining route to a real RPM is `nbfc-linux` talking to the EC directly, which needs root, a
  board-specific config, and a tool whose primary purpose is to **take over the fan curve**.
  **Recommendation: do not.** Socket power plus package temperature are both live and responsive and
  are what the fan curve actually follows, so the proxy answers the question the fan reading was
  wanted for, without putting a third party in charge of cooling his laptop. **Done meanwhile:**
  power and temperature are sampled, carried in the panic incident, and graded in the fleet-heat
  notebook, labelled as a proxy throughout. **To reverse:** install nbfc read-only and add one field.
  Detail: `docs/idle-cost-model.md` §7b.
  *Meanwhile:* nothing waits on this; the heat grading uses the proxy.

- **Should the panic heartbeat ACTUATE, or only report?** The detector is live and files a durable
  `heartbeat/panic` incident plus one addressed notification when the client host crosses the
  memory/CPU/thermal thresholds, in that priority order. What it deliberately does **not** do is act
  — throttle the scan cadence, pace the title chore, or park a hot row — because each of those
  changes behaviour the campaign depends on, and a detector that quietly starts steering is far
  harder to trust than one that only ever reports. **Recommendation: allow actuation for the scan
  cadence only**, as a bounded slowdown that reverts on its own when the condition clears, and leave
  row parking manual. **Done meanwhile:** detection, incident and notification are shipped and
  running; nothing is throttled. **To reverse:** `YGGTERM_HOST_PANIC=0` disables the whole watcher.
  Detail: `docs/observability.md` §7.
  *Meanwhile:* nothing waits on this; the relay reads the incidents in the fleet-heat notebook.

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
  ⛔⛔ **THE STAKES ROSE ON 2026-08-14 AND THIS IS NO LONGER HYPOTHETICAL.** The gate **missed a
  real private value on a real push to a real public repo**, because a third-party form had
  reformatted it and the gate matches literals. The fix — matching number-shaped terms on digits
  across any interior window — is now deployed on all three hosts and **exists only as an
  unversioned file**. ⇒ **Two consequences worth your ruling, not just the original question:**
  (1) that fix has no review, no history and no way to prove which hosts carry it beyond comparing
  hashes by hand; (2) the newest-wins replication that spread it could **silently revert it** the
  moment any host's older copy is touched, and the gate would go back to passing that leak while
  printing the same reassuring line. **Recommendation unchanged and now more urgent: give it a
  private Forgejo repo.**

- **Two fleet-sync bugs are in his `~/.claude/hooks/`, which an agent does not rewrite on a peer's
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
  ⛔⛔ **A SECOND DEFECT IN THE SAME SNIPPET, AND IT NEARLY UNDID A PRIVACY PURGE (2026-08-14).**
  `git log @{u}..HEAD | wc -l` returns a COUNT where the meaning is DIRECTIONAL, so it **cannot
  tell unpushed work from divergence.** After a history rewrite removed a private value from a
  public repo, a stale checkout read as *"5 unpushed"* — it was **ahead 5 / behind 5**, the old
  chain sitting beside the new one. ⚠ **The obvious response to "unpushed" is to push, and that
  would have restored the leak on the public remote.** ⇒ **One instrument answers both questions in
  the same breath:** `git rev-list --left-right --count origin/main...HEAD` — `N 0` is behind,
  `0 N` is genuinely unpushed, `N N` is a rewrite and never wants a push.
  **Recommendation: take this with the worktree fix, as one edit.** ⚖ Given the worktree blindness
  was the first defect found in this snippet today, a third is likelier than not; it is worth
  replacing the line rather than patching it twice.

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

- **The interface LLM's plan quota is exhausted until roughly 13 September** — the
  configured title/summary model answered every request with `usage_limit_reached`
  (`plan_type: free`), taking ~9 s to say so, which is what the title chore spent a
  fifth of every hour on. Only he can decide whether to upgrade that plan or keep
  running on a different model. **Done meanwhile, and reversible in one field:** the
  chore now pauses itself on a refusal rather than looping (`pending-bugs.md` [11.3]), and
  the fleet title sweep runs with `--model <id>` so it needs no settings change at all. A working
  model is measured and named in that entry: `antigravity/gemini-3.7-flash-low`, same endpoint,
  same key, ~15 s per title. ⚠ Changing the default is his because an agent CANNOT — a settings.json
  edit under a running GUI is written back over from the GUI's own copy within the hour.

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

- **Publishing the Android app v3.2.0 is a distribution decision, so it waits.**
  The build is done, signed with the permanent certificate (so installs upgrade
  in place rather than being stranded), and proven: the release variant
  authenticated over SSH and rendered a live fleet, opened a session, and ran a
  typed command on it. Pushing it to the public channel is outward-facing and is
  his call per action, which is the only reason it has not happened.
  *Recommendation:* **publish.** The version now installed is a fixture demo that
  cannot reach anything; this one is the product. There is no native code, so the
  same bytecode that was proven on an x86_64 emulator is what an arm64 phone runs.
  *Meanwhile:* the APK is built and verified, and nothing else is blocked by it.
  ⚠ **Publishing alone is not enough to make it work.** The app generates its own
  key and shows one enrolment line to paste into `authorized_keys` on a machine he
  owns. Only he can do that — it needs the line from HIS device, and the line
  carries a forced command so the key gets the daemon protocol and nothing else.

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

---

## One campaign's row is out of API CREDITS, and only he can restore them

A subscribed row was refused by the model provider with *"out of usage credits …
`/model` to switch models"*, which no amount of waiting fixes: a balance is
restored by a purchase, or the row moves to another model. **Both are his.**

**Done meanwhile, so nothing is blocked on the answer.** That refusal used to
hold the WHOLE fleet — every other campaign's rows were unwakeable behind one
row's billing state, twice today, and the same shape ran 7.4 continuous hours on
2026-08-14. It is now a **per-row suspension**: that one row sits out, the fleet
stays wakeable, and it un-suspends by itself the moment the row writes again.
A genuine timed quota window still holds everything, which is correct — that one
does clear on its own.

**Recommendation: no action unless he wants that campaign woken tonight.** It
costs nothing to leave: the row is alive, its work is not lost, and it resumes
the moment either remedy is applied. **To reverse:** nothing to reverse.

---

## Nothing is waiting on him for these, and they are the relay's actual queue

Said explicitly because the tab is easier to read when it is short: everything
else open in either repo is unattended work — the cross-origin
frame, the view-swap stall, the passkey shim's scope, the stranded control port,
the flaky `daemon_staleness` pair, the yedit document-surface regression, the
Android `#[cfg]` gate, the WebKit popup blocker. The relay takes those in
load-bearing order without asking.
