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
  *Meanwhile:* the pile can no longer GROW — the count held flat across a version
  bump for the first time — so it shrinks on its own as sessions end.

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

---

## Nothing is waiting on him for these, and they are the relay's actual queue

Said explicitly because the tab is easier to read when it is short: everything
else open in either repo is unattended work — the cross-origin
frame, the view-swap stall, the passkey shim's scope, the stranded control port,
the flaky `daemon_staleness` pair, the yedit document-surface regression, the
Android `#[cfg]` gate, the WebKit popup blocker. The relay takes those in
load-bearing order without asking.
