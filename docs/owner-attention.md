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

1. **One line per item**, in his words where they exist: what is needed, and the
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

- **Muse Code needs his login before anything about it can be MEASURED** — the
  CLI is closed source and installing it requires a vendor account only he holds.
  Its descriptor ships with its resume flag, composer glyph and working phrase
  marked as placeholders rather than measurements, and says so on the value. →
  `docs/spec-adding-an-agent-cli.md` §6.
  *Meanwhile:* the other eight CLIs are wired, and Muse's row is data — one
  descriptor edit once a real `muse --help` and a real screen exist.

## Third parties only he can chase

- **Google Play identity verification is his, and it gates every listing** — the
  developer account exists and is paid (ID `7834754661078735260`), but Google
  locks *Create app* until three checks pass: government ID, access to an Android
  device, and the contact phone. Only he can present a document. Decided
  2026-08-08: **passport, not Aadhaar** (Aadhaar is not Google's to hold, and the
  passport survives the US move) — with one thing to check first, that the
  passport's NAME matches the payments profile, since Google verifies against
  documents. → `~/data/fingraph/owner-window/fingraph-playstore.md`.
  *Meanwhile:* nothing in either repo waits on it; the campaign has no Play work
  that verification unblocks.

- **The fake `Anthony Gestapo` US payments profile wants closing, in a NORMAL
  browser** — profile `3778-9171-5739`, made years ago, holds no payment methods
  and gates nothing, but it is a false identity record on the account that is
  about to be identity-verified. `Close payments profile` exists in its Settings
  and raises a Google re-auth that never opens the close flow on an agent
  surface. → `~/data/fingraph/owner-window/fingraph-playstore.md`.
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

---

## Nothing is waiting on him for these, and they are the relay's actual queue

Said explicitly because the tab is easier to read when it is short: everything
else open in either repo is unattended work — the cross-origin
frame, the view-swap stall, the passkey shim's scope, the stranded control port,
the flaky `daemon_staleness` pair, the yedit document-surface regression, the
Android `#[cfg]` gate, the WebKit popup blocker. The relay takes those in
load-bearing order without asking.
