# App rows survive. Killing the terminal must not kill the user's apps.

**Status: LAW.** Owner-directed 2026-08-14, after a GUI restart destroyed a row
group he had built — one `New Yedit` and four `New Ychrome` rows — in his words:
*"We should not kill the terminal/libyggterm apps like when the row group of only
libyggterm apps was lost. This is the spec."*

---

> ### ✅ IMPLEMENTED 2026-08-14 — and two of §2's three claims were already stale
> when this was written. The RULE below is unchanged; only the diagnosis moved.
>
> | §2 claim | as measured |
> |---|---|
> | 1. born disposable | **was true, now fixed.** The birth rule asked `kind.is_agent()` and an app row is a `SessionKind::Shell`. It now also reads the row's app stamp. |
> | 2. no way to protect one | ⚠ **already false.** `server app terminal keep\|unkeep <row>` exists, and the GUI has a "Keep Alive" / "Stop Keeping Alive" context-menu item. Driven live in both directions with a read-back after each. |
> | 3. launch args not stored | ⚠ **half false, and the other half was invisible.** The `app:<name>:<verb>` token was already persisted. But the restore SPENT it without stamping it back, so the round-trip survived exactly one restart; and the re-derivation sat inside the transcript branch, which an app row never enters, so a protected row with a valid token still restored as bare bash. Both fixed. |
> | §3 no departure record | **was true, now fixed.** See the §3 note below. |
>
> ⭐ **The lesson worth keeping is the shape of claim 3.** It read as shipped
> because the half that was easy to check — does the token survive? — did
> survive, and the half nobody checked was the one the user sees. A round-trip
> test that restores ONCE cannot see a fact that dies on the second restart, and
> a token that round-trips proves nothing about the command derived from it.

## §1 THE RULE

> **A row the user created deliberately survives a GUI restart. Always. Whatever
> its `kind`.**

⛔ **There is no "second-class" tier for a row somebody made on purpose.** The
existing split — agent-CLI rows are born keep-alive, everything else dies with the
GUI — is an implementation detail that leaked into the product as data loss. The
constitution already says this and was not being honoured:

> *"Plain shells are first-class and must survive a bump like anything else."*

## §2 WHAT WENT WRONG, EXACTLY

The rows were stored as:

```
"title": "New Ychrome", "kind": "shell", "keep_alive": false
```

Three separate defects, and all three must be fixed:

1. ⛔ **Born disposable.** A row created from an app verb (`launch-app`) is born
   `keep_alive: false`, so a GUI restart takes it. Nothing in the UI says the row
   is disposable, and from the user's side nothing about it is scratch — he had
   arranged them into a group.
2. ⛔ **No way to protect one.** There is **no CLI verb to set keep-alive on an
   existing row.** `--keep-alive` is documented only as *unnecessary* for
   agent-CLI kinds. A user who wants to protect a row he already has cannot.
3. ⛔ **The launch arguments are not stored.** The persisted record keeps `title`,
   `kind`, `cwd` and `ssh_target` and **no app args** — so even a successful
   restore can only produce a blank instance of the app. Which profile each
   `New Ychrome` held was unrecoverable.

## §3 THE STORES MUST AGREE, AND AN EVAPORATION MUST BE RECORDED

⚠ After the loss, `server-state.json` still listed the rows under `live_sessions`
while the GUI's live set did not have them, and `sessions restore` answered
`not_found`. **Two stores disagreed about whether the row existed.**

⛔ And `removed-rows.json` had **no entry** for them. So nothing in the system
distinguished *"the user closed this"* from *"this evaporated"* — which is the
same class as a subscription that can vanish without a trace: **an absence is not
a record.**

⇒ **A row leaving the live set for any reason other than an explicit user close
must leave a record saying which reason.**

> ✅ **Done 2026-08-14.** Root cause was one line: `PrepareClientClose` removed
> each non-keep-alive row itself instead of going through the close chokepoint,
> so it never reached the only code that writes anything down — the second close
> path that file's own comment forbids. `removed-rows.json` now carries a
> `departures` ledger beside its veto set: which row, its title, the reason
> (`explicit-close` or `gui-close-disposable`), and when. **Ask it with
> `yggterm-headless server rows departed [--limit N]`.**
>
> ⚖ The two are different questions and are kept apart on purpose. The veto set
> answers *"may this row be imported back?"* and is cleared the moment the row
> legitimately returns. The ledger answers *"what happened to the row that is not
> here?"*, and a return must NOT erase it — a row that evaporated an hour ago
> still evaporated after the user recreates it, and "it is back now" is exactly
> the answer that made the first loss undiagnosable.
>
> ### ⭐⭐ AND THE LEDGER'S FIRST JOB WAS TO IDENTIFY THE INCIDENT THAT CAUSED IT
>
> **The rows were never closed.** Reading the desktop host's trace for the third
> departure path — `live_session_persist_dropped` — found four local rows dropped
> in a single update-restart persist, `not_in_protected_runtime_keys`, and two of
> those uuids are named elsewhere in the trace by the titles of the app rows that
> were lost. **He had closed nothing.**
>
> A dropped row is not removed. It stays in the running daemon's live order and
> is simply left OUT of the state file, so the SUCCESSOR daemon never learns it
> existed. That accounts for every symptom in this section, which the close-path
> theory did not:
>
> | symptom | what a persist drop explains |
> |---|---|
> | `removed-rows.json` empty | nothing was closed, so no close path ran |
> | `sessions restore` → `not_found` | the successor never received the ids |
> | **`server-state.json` still listing them under `live_sessions` while the GUI's live set did not** | ⭐ the daemon that dropped them **still holds them in memory and still lists them** — only its successor's state file omits them. The two stores were not disagreeing about one daemon; they were two different daemons, and that is what "the stores disagree" actually was. |
>
> ⇒ The persist filter records a departure too (`persist-dropped`, carrying which
> of the three gates took it). And §2.1 already prevents the recurrence for app
> rows without anyone noticing it would: **the drop gate begins `!keep_alive`, so
> a row that is born protected never reaches it.** The fix was right for a reason
> the diagnosis had not found.
>
> ⚠ **What is genuinely still open** is the instrument question, not the loss:
> whether `server app close` performs a close at all (queue entry `[6.2]`).

## §4 WHAT MAY STILL BE REAPED

This spec does not forbid reaping. It forbids *silent, unasked* reaping of the
user's own rows.

- ✅ `--ephemeral` rows an agent created for a probe — that is what the flag is
  for, and it is opt-in by the creator.
- ✅ A row the user explicitly closes.
- ⛔ Anything else, on a GUI restart, is a bug against this spec.

## §5 THE OPERATIONAL RULE UNTIL §2 IS FIXED

> ✅ **§2.1 is fixed, so this section has expired for rows created from
> 2026-08-14 onwards** — a new app row is born keep-alive and a GUI close leaves
> it alone. It still applies to any app row created by an OLDER build, because
> the flag is persisted per row and nothing retroactively upgrades one: protect
> such a row with `server app terminal keep <row>`, which is the affordance
> §2.2 said did not exist.
>
> ⭐ **And the check itself is now cheap and needs no memory of what was there:**
> `server rows departed` names every row that has left and why. A row missing
> with a `gui-close-disposable` entry went because it was disposable; a row
> missing with a `persist-dropped` entry was never closed at all; a row missing
> with NO entry did not leave through any path that knows it left, which is a bug
> in its own right.
>
> ### ⚠⚠ AND THIS SECTION'S PREMISE MAY SIMPLY BE FALSE — MEASURED, NOT ARGUED
>
> *"Treat a GUI kill as destructive to app rows"* assumes the GUI close reaps
> them. On the desktop host: **nine GUI launches in the retained traces and ZERO
> events from any step of the shutdown path** — no flush, no watchdog, no
> `client_close_prepared`. ⇒ **`PrepareClientClose` has not been sent once in that
> window, so nothing has been reaping anything at GUI close.** A sandbox
> reproduces it from the other side.
>
> ⚖ So the operational rule is over-cautious in one direction and the tier it
> defends may not exist in practice. **It is left standing anyway**, because the
> measurement is bounded by trace retention rather than by history, and because
> being wrong in this direction costs a re-launch while being wrong in the other
> costs the user's rows. Queue entry `[6.2]` carries the mechanism, the falsifier,
> and why repairing the close path is a DECISION rather than a fix — it would
> start destroying rows that survive today.

⚠ **Until app rows are born keep-alive, treat a GUI kill as destructive to them.**

- Before killing a GUI, **enumerate the app rows** and be ready to relaunch them.
- After any GUI restart, **check the app rows came back** and relaunch what did
  not (`server app launch-app <app> [verb]`).
- ⛔ Do not report a GUI restart as clean without that check. The window coming
  back is not the same claim as the rows coming back.

⚖ This does **not** reinstate "ask permission before restarting the GUI" — the
constitution forbids deferring a restart out of politeness, and the daemon still
owns every PTY. It adds an obligation *after* the restart, not a gate before it.
