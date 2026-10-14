# Owner attention — the short list of what is waiting on Avikalpa

**The one question this file answers: _what needs him, right now, across yggterm
and ychrome?_** Nothing else. It exists because the campaign now runs unattended
around the clock (owner directive, 2026-08-08), and an agent that hits an
owner-gated step must be able to park it somewhere he can find in one glance
instead of stalling on it.

## ⛔ THE 2026-10-15 GATE SWEEP

**Twenty owner-gated items were cleared interactively on 2026-10-15** (one
question at a time, recommendations pre-populated). Every ruling is recorded in
[`settled-calls.md`](settled-calls.md) § "THE 2026-10-15 GATE SWEEP" — including
two reversals (the site-lore corpus is now REMOVED, not preserved; the GUI close
path is now REPAIR-AND-ENFORCE, not retired) and one permanent rejection with a
lesson attached (tsc=reliable: tried for two months once, broke Chromium/Chrome,
reverted — NEVER re-propose; a settled call asked twice is a settled-calls
coverage failure: grep this file's settled-calls sibling BEFORE parking a
question here).

**Before parking a question in this file: grep `settled-calls.md` first.** The
tsc question sat here for two months with a fresh recommendation on top of a
reverted incident nobody had logged.

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

**Nothing is currently waiting in this section** — the 2026-10-15 sweep cleared
it out. Park new items one-per-line with the ONE detail link.

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

- **The two-app split for phone superpowers: DECIDED — SPLIT (owner,
  2026-10-15).** Clean `yggterm` for the stores; sideload-only `yggterm-agent`
  carrying accessibility, SMS and overlay permissions. The fork itself is
  agent work now (v3.1 needs none of those permissions; nothing blocks).

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
  call each time. **Recommendation on file: make it public, matching the other
  platform apps.** (Not covered by the 2026-10-15 sweep; still his.)
  **Done meanwhile:** the repo is private with full history and the binary is on
  the fleet, so the app is usable now either way. **To reverse:** one
  visibility change; nothing else depends on it.
- **Android v3.2.0 Play publish: HELD by owner ruling 2026-10-15.** The build is
  done, signed with the permanent certificate, and SSH-proven live. Do not
  publish without a fresh word. (⚠ Publishing alone is not enough to make it
  work: the enrolment line from HIS device still needs pasting into
  `authorized_keys` — the key carries a forced command so it gets the daemon
  protocol and nothing else.)
