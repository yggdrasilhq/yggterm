# Agent co-browse gap — a POPUP-based re-auth cannot be completed on a web surface (2026-08-08)

Filed from **another campaign row** (a third-party portal onboarding) after the task
*"close the fake `Anthony Gestapo` US payments profile"* was driven to the last
step on a `--no-activate` yggterm web surface, failed there four times, and was
then **finished by the operator in Chromium in a couple of minutes**. His verdict,
verbatim: *"I manually removed the Anthony profile on a chromium browser. It was
ychrome browser shortcoming."*

This is the second time this exact wall has been hit on this account
(2026-08-07, recorded in `play.google.com` site-lore as *"`Close payments profile`
exists in Settings but its dialog never opens on an agent surface"*). That entry
was a symptom. **This one has the mechanism, and the mechanism is general** — it is
not about Google and not about payments profiles. Any flow whose next step is
`window.open(reauth) → complete → popup closes → parent resumes` is unreachable
from this plane today.

---

## ⛔ DEFECT 1 — the parent never resumes after its popup finishes

**The flow.** `pay.google.com/gp/w/home/settings` → click *Close payments profile*
→ the page shows *"To continue, please verify it's you. A new window will appear
for you to sign in"* and calls `window.open()` on an `accounts.google.com`
password challenge.

**What was measured, four full runs:**

| step | result |
|---|---|
| popup opens as **tab 3** of the same surface | ✅ `web_surface_tabs` gains a row, `webview:true`, and it becomes the ACTIVE tab, so `web *` verbs address it with no extra flags |
| `web fill --entry accounts.google.com --user <acct>` into the popup | ✅ page-side `input[type=password].value.length` = **40**, three times out of three |
| submit | ✅ popup reaches `https://accounts.google.com/CheckCookie` — **the re-auth SUCCEEDS**, this is not a credential failure |
| popup closes itself | ✅ |
| **the opener resumes the close-profile flow** | ⛔ **NEVER.** The opener is back on `/gp/w/home/settings` with `[role=dialog]` empty and every injected global (`window.__cap`, `window.__msgs`, `window.__hooked`) **gone — the parent document was replaced/reloaded** |

**`window.opener` is NOT the missing piece — that was tested and it is wired.**

```
# from the opener, on the same surface:
window.open("https://pay.google.com/gp/w/home/activity","testpop","width=600,height=600")
# then, evaluated in the popup:
{"u":"https://pay.google.com/gp/w/home/activity","hasOpener":true,
 "openerOrigin":"https://pay.google.com"}
```

So a same-origin popup CAN reach its opener here. The break is downstream of that.

**The leading hypothesis, and it is testable by whoever fixes this: the parent is
never told it is back in front.** On a `--no-activate` surface
`document.visibilityState === 'hidden'`, `requestAnimationFrame` never fires
(already documented for this plane), and **the opener receives no `focus` and no
`visibilitychange` when its popup closes**, because nothing ever had focus. A
continuation gated on any of those three simply does not run.

Partial evidence for it, from this run: shimming the opener with

```js
Object.defineProperty(document,'visibilityState',{get:()=>'visible',configurable:true});
Object.defineProperty(document,'hidden',{get:()=>false,configurable:true});
document.hasFocus = () => true;
window.requestAnimationFrame = cb => setTimeout(()=>cb(performance.now()),16);
```

made the flow's own dialog **lay out larger — 448×80 → 560×80** — i.e. it had been
being laid out against a frozen frame clock all along. It still did not resume,
**and the parent's reload wipes the shim anyway**, which is the real point:

> ⭐ **A page-side shim cannot fix this class, because the shim does not survive the
> navigation that is part of the flow.** It has to be in the engine.

**What it cost:** the entire task. ~1.5 h of agent time across four attempts, two
teardown/relaunch cycles, three re-auths of the operator's Google account, and
then the operator did it himself in Chromium. The agent plane produced nothing
except this document.

---

## ⛔ DEFECT 2 — when a popup tab closes, the surface's ACTIVE tab lands on a `no_webview` ghost, and every verb then lies about why

The moment the re-auth popup closed, this happened on **every** run:

```
$ yggterm server app web eval --stdin --session <s>
{"accepted": false, "reason": "web surface not live (session backgrounded or not yet revealed)"}
```

That sentence is **wrong in both of its clauses**. The surface is live and the real
page is fine. `server app state` shows the truth:

```
[(tab 0, 'stashed',    webview True,  active False),   <- the real page, alive
 (tab 1, 'no_webview', webview False, active TRUE),    <- a ghost, and it is ACTIVE
 (tab 2, 'no_webview', webview False, active False)]
```

The active-tab pointer lands on a `no_webview` slot left behind by a closed popup
(or by a previous `ychrome --profile <p>` launch — the site-lore already notes
that relaunching leaves a dead row), and since **no `web *` verb takes a `--tab`**,
the live page becomes unaddressable.

**✅ The recovery, found the expensive way and worth putting in the error text:**

```
yggterm server app web close --session <s>     # closes the GHOST, not the page
# -> tab 0 becomes active again, and eval answers normally
```

**What it cost:** two full `pkill ychrome` + `terminal new` + relaunch cycles
(~15 min) and one abandoned session row, before `web close` was tried on a hunch.
The first two cycles were taken *because the error message says "backgrounded or
not yet revealed"*, which points an agent at revealing the surface — the one thing
the co-browse doctrine forbids.

**Compounding it:** `web ensure` in that state answered, three times running,

```
{"accepted": true, "healed": false,
 "detail": "…'s page was unresponsive; a rebuild is queued — re-run ensure and compare generation_after",
 "generation_before": null, "generation_after": null}
```

The instruction cannot be followed — both generations are `null` — and no rebuild
ever came.

---

## ⛔ DEFECT 3 — the agent's OWN injected click later counts as unattributable seat input, locking it out of the credential verbs

Mid-flow, after several successful `web do click`s on the same surface:

```
$ yggterm server app web fill-vault --item accounts.google.com --user <acct> \
      --field totp --selector '#totpPin' --redact --session <s>
{"accepted": false, "reason": "seat_input_on_unrevealed_surface", "seat_input_count": 1,
 "detail": "seat input was observed on a surface no client has ever shown, so it cannot be
            attributed to the user and this verb is refused. The batch is NOT preempted.
            Reveal the session (open its row) before driving it, or re-run once the surface
            is on screen."}
```

Nobody touched that surface. `seat_input_count: 1` is **the agent's own `web do
click`**, one call earlier. From then on `do` and `fill-vault` alternated between
`seat_input_on_unrevealed_surface` and `preempted` on that surface, while
`web fill --entry` and `web totp` still worked.

Two consequences, both bad:

- the remedy the message prescribes — *reveal the session* — is exactly what
  §DETACHED-BY-DEFAULT tells agents never to do for the sake of driving;
- an agent that follows the message pollutes the operator's viewport to fix a
  refusal caused by its own earlier click.

**Workaround used (and it should not have been needed):** drive everything
page-side with the full pointer sequence (`rclick`) through `web await`, which is
not subject to the seat-input check.

---

## ✅ What worked, and must not regress

- **`web totp --entry <e> --user <u>`** filled the 6-digit code on
  `accounts.google.com` with no owner involvement; page-side `#totpPin.value.length`
  = 6. The whole view-scope re-auth (passkey default → *More ways to verify* →
  *Get a verification code from the Google Authenticator app* → code → *Next*) ran
  headlessly on an unrevealed surface.
- **`web fill --entry … --user …`** landed a 40-char password into a **popup tab**,
  three times, verified page-side each time.
- **The rAF/animation unfreeze recipe** from `play.google.com` lore
  (`document.getAnimations().finish()`, kill scale-to-zero transforms, let the
  `div.ZQxJQe` scrim through) — still the difference between a readable page and a
  frozen one.
- **Page-side `rclick`** drove Google's SPA left-nav (`Payment methods`,
  `Subscriptions & services`, `Activity`, `Addresses`, `Settings`) reliably where
  `web do` was refused.
- **`web close` as ghost-tab recovery** (defect 2).
- **Row hygiene**: `session remove` answered `verified:true` with `live_processes: []`,
  and `pgrep -af 'ychrome --profile play-dev'` was empty afterwards.

---

## Feature asks, ranked, each with the failure attached

1. **⭐ A "presented" contract for unrevealed surfaces — visibility, focus and a real
   frame clock.** The engine should hand an unrevealed surface
   `visibilityState:'visible'`, `hidden:false`, `hasFocus():true` and actual rAF
   ticks, and should deliver `focus` / `visibilitychange` to an opener when its
   popup closes. **(b) Why it matters beyond this site:** every "stalled Google
   page" already in this plane's lore is one symptom of the frozen frame clock, and
   this task is the first one where the symptom was *unfixable page-side* because
   the flow navigates and eats the shim. **(c) The failure:** this whole document.
2. **`--tab <n>` on `web eval|read|do|close|screenshot|frames`, plus a `web tabs`
   verb.** **(c) The failure:** defect 2 — the live page was unaddressable for two
   relaunch cycles purely because a ghost held the active pointer.
3. **Popup lifecycle in the control plane** — `web wait --until popup:opened` /
   `popup:closed`, and the popup's tab id in the reply. Today a popup is discovered
   by polling `server app state` and diffing tab rows, and its close is discovered
   by a verb suddenly failing. **(c) The failure:** every `sleep N; check` in this
   run, and the ghost that follows.
4. **Do not count an agent's own injected input as seat input.** The lease already
   knows who injected; attribute it. **(c) The failure:** defect 3.
5. **Name a Chromium/CDP fallback rung on the co-browse ladder.** `jojo` carries
   `/usr/bin/chromium` and `/usr/bin/firefox`; the operator finished in one of them
   in minutes. There is currently no documented rung between "the WebKit surface
   cannot do it" and "hand it to the human", so an agent re-tries the broken plane
   instead of switching engines. **(c) The failure:** four attempts at the same wall.
   ⚠ A CDP lane needs its own login (separate cookie jar) and its own vault path —
   worth scoping before it is promised.

---

## Reproduction

Site-lore slug: **`pay.google.com` → `close-payments-profile-popup-reauth-wall`**
(written the same day; it carries the exact verb sequence).

Any `window.open`-based re-auth reproduces defects 1–2; Google's payments centre is
simply the one that is free to try. Defect 3 reproduces on any unrevealed surface by
issuing one `web do click` and then any `fill-vault` on the same surface.
