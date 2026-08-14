# App rows survive. Killing the terminal must not kill the user's apps.

**Status: LAW.** Owner-directed 2026-08-14, after a GUI restart destroyed a row
group he had built — one `New Yedit` and four `New Ychrome` rows — in his words:
*"We should not kill the terminal/libyggterm apps like when the row group of only
libyggterm apps was lost. This is the spec."*

---

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

## §4 WHAT MAY STILL BE REAPED

This spec does not forbid reaping. It forbids *silent, unasked* reaping of the
user's own rows.

- ✅ `--ephemeral` rows an agent created for a probe — that is what the flag is
  for, and it is opt-in by the creator.
- ✅ A row the user explicitly closes.
- ⛔ Anything else, on a GUI restart, is a bug against this spec.

## §5 THE OPERATIONAL RULE UNTIL §2 IS FIXED

⚠ **Until app rows are born keep-alive, treat a GUI kill as destructive to them.**

- Before killing a GUI, **enumerate the app rows** and be ready to relaunch them.
- After any GUI restart, **check the app rows came back** and relaunch what did
  not (`server app launch-app <app> [verb]`).
- ⛔ Do not report a GUI restart as clean without that check. The window coming
  back is not the same claim as the rows coming back.

⚖ This does **not** reinstate "ask permission before restarting the GUI" — the
constitution forbids deferring a restart out of politeness, and the daemon still
owns every PTY. It adds an obligation *after* the restart, not a gate before it.
