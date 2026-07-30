# Agent co-browse gaps — field report, 2026-07-28

Written from a real job, not a synthetic test: building two diagnostic-lab
orders (Dr Lal PathLabs and Redcliffe Labs, Kolkata) end to end on a shadow
surface, on behalf of the operator, stopping before payment.

**Outcome.** Both carts were built successfully and are correct. The Dr Lal
booking could **not** be completed by the agent — it stalls at OTP login and at
a patient-selection modal. The operator had to log in by hand.

Everything below is a defect or an absent capability that cost this run real
time. Each entry names the concrete failure, because a feature request without
the failure attached is unfalsifiable.

---

## A. Confirmed defects

### A1 ★★ Segmented OTP fields are unreachable from a shadow surface — this blocks the entire logged-in plane

`web do fill --selector-set 'input[type=tel]' --text '2597'` answers:

```json
{"accepted": false, "reason": "surface_not_mapped", "error": "surface_not_mapped"}
```

A shadow surface is unmapped **by design** — that is what keeps it off the
operator's screen. So the only supported verb for segmented inputs is
unavailable exactly where agents are supposed to work.

The eval fallback does not work and **fails silently**, which is worse than
failing loudly. Native-setter + `input`/`change`/`keyup` on each of the four
`input[type=tel][maxlength=1]` boxes leaves the DOM looking perfect
(`filled:"xxxx"`, LOGIN button enabled), but React state never updates, so the
form posts an empty code. The site shows **no error text**; the modal just sits
there. An agent reasonably concludes the SMS code was wrong and re-requests it,
burning OTP attempts.

This is not a Dr Lal quirk. OTP login gates nearly every target on the
co-browse worklist — services-desk, a services portal, records, banks, commerce, labs. The
data-fabric skill already records the same wall at a-services-portal.example
(`consumer-complaint-login-blocked`). **The plane can read the OTP off the
phone in under five seconds and then cannot type it.**

### A2 ★ `el.click()` silently no-ops on a large class of React handlers

Dr Lal's "Proceed / Add patient" ignored `el.click()` three separate times — no
error, no state change, no navigation. It opened first try when driven with a
full pointer sequence at real coordinates:

```js
pointerover → pointerenter → pointermove → pointerdown → mousedown
            → pointerup → mouseup → click
```
with `{bubbles, cancelable, composed, clientX, clientY, view, button:0,
buttons:1, pointerId:1, isPrimary:true, pointerType:'mouse'}`, ~55 ms apart, and
`buttons:0` on the up events.

Every agent that meets this has to re-derive the sequence. It should be a verb.

### A3 Some controls resist even the full synthetic sequence

Dr Lal's patient row (`.familydetailsMain > div#0`) did not select under the A2
sequence, on the row, its `.content h5`, or its `.imageHolder`. The likely cause
is an `event.isTrusted` check, which page JS cannot forge — only the browser or
compositor can. If so, A2 is a partial workaround and **A1's real fix (trusted
input into an unmapped surface) is also the fix for this.**

### A4 ychrome is single-instance per profile and silently hijacks the existing surface

Asked for a second tab, a fresh session running
`ychrome --profile health https://redcliffelabs.com/kolkata/tests` printed:

```
ychrome: opened https://redcliffelabs.com/kolkata/tests in session local://f008f0f0-…
```

— the **already-running** session, replacing its page. The new session's own
surface never materialised (`web ensure` → `no web-surface declare … a plain
shell, or the app already closed its surface`).

The Dr Lal work survived only because its cart lives in `localStorage`. Anything
held in page memory would have been destroyed with no warning. The operator's
literal request was "another tab in the same ychrome session"; there is no way
to honour it.

### A5 `YGGTERM_APP_CONTROL_PID` is honoured inconsistently across subcommands

With two GUI clients live, `terminal new` respected the env var, while
`web ensure` still refused:

```
Error: multiple live Active Yggterm GUI clients are registered; rerun with
--pid <pid> / --client <name> or set YGGTERM_APP_CONTROL_PID
```

— naming the very variable that was set and exported. Separately, `--pid` is
documented as a trailing flag on `server app state|rows|open|screenshot` but is
**not** listed for `server app terminal new`, so the error's own advice cannot
be followed for that subcommand.

### A6 A dead shadow client is invisible until an unrelated call fails

`scripts/shadow-client.sh start --name agent-health` reported `client=1697792`.
Some time later that pid was simply gone; the first symptom was
`server app open … --pid 1697792` answering `no live Yggterm GUI client with
pid 1697792`. The web surface it had hosted was still alive and eval-driveable
the whole time, which makes the failure especially confusing. There is no
liveness signal and no notification.

### A7 `server app open --pid <shadow>` claims success but does not map the surface

```json
{"activated": true, "queued": true}
```
followed by `web ensure` still reporting `"mapped": false`. If mapping into a
shadow compositor is unsupported, it should refuse by name rather than report
`activated:true`. If it is supposed to work, it is broken — and it is the single
change that would unblock A1.

---

## B. Dream features

Ordered by how much this run would have gained. Each says what it costs today.

### D1 ★★★ Trusted input into an UNMAPPED surface

Inject real input events at the compositor/WebKit layer into a surface that is
never composited to a visible output — the headless analogue of what a mapped
surface gets today.

**Why:** it is the difference between an agent that can *read* logged-in
services and one that can *use* them. Everything on the co-browse worklist —
tax filings, records, grievances, commerce, labs — is gated by an OTP box or a
gesture-checking widget.

**Cost today:** this run could read the OTP off the phone in five seconds and
could not type it. The operator logged in by hand. A1 and A3 both dissolve if
this exists; the shadow doctrine (never paint on the user's screen) and the
ability to log in are currently in direct conflict, and the doctrine loses every
time an agent maps a surface to get work done.

### D2 ★★ `web do click --gesture full`, ideally the default

Ship the A2 pointer sequence as a flag, with `--gesture simple` for today's
behaviour.

**Cost today:** several wasted round trips on a control that reported no error
and did nothing. Every agent re-derives this by hand, and most will conclude
"the site is broken" first.

### D3 ★★ Verb-level post-conditions — `--expect <cond>`

`web do click --text 'Add to cart' --expect 'js:localStorage…includes("Z827")'`,
failing the call when the post-condition does not hold, reusing `web wait`'s
condition grammar.

**Cost today:** this run reported **five consecutive successful add-to-cart
clicks that had all failed** — the clicks landed on the SPA while the cart being
inspected was a different legacy frontend. The doctrine already says
`accepted ≠ delivered`; nothing enforces it, so the honest check is the one an
agent skips under time pressure. This is the cheapest large win on the list.

### D4 ★★ Multiple pages per profile — `web tabs new|list|switch|close --session`

**Cost today:** A4. Comparison work is inherently multi-page; the operator asked
for exactly this and it had to be refused. Loading Redcliffe destroyed the Dr
Lal page.

### D5 ★ `web state --session` — one call, whole picture

Return `{url, title, mapped, generation, tab_count, document_ready,
localStorage_keys, cookie_count, visible_input_count, top_text_excerpt}`.

**Cost today:** perhaps a dozen round trips spent re-reading `innerText` to
answer "where am I", "did that navigate", "am I logged in", "did the modal
open". Each is a full eval round trip today.

### D6 ★ `web net --session [--filter <substr>]` — network capture

**Cost today:** finding Dr Lal's price API required hand-patching
`XMLHttpRequest.prototype.open/send` in page context. The patch died on every
navigation and had to be reinstalled three times. Worse, once the endpoint was
known, calling it directly from eval returned `null` (CORS/missing headers), so
the monkey-patch was the *only* route — an agent that did not think of it would
have concluded the data was unreachable.

### D7 ★ Persistent page-context helpers — `web inject --script f.js --persist`

Re-inject on every document creation (`document_start`), like a content script.

**Cost today:** the `rclick`/`byText` helpers had to be reinstalled after every
navigation, and one multi-step script died mid-run because `window.rclick` had
vanished after a redirect. Combined with D6's hook, this was the single most
repeated piece of boilerplate in the session.

### D8 `--pid` / `--client` uniformly on every `server app` subcommand, plus `--require-client <name>`

`--require-client` should fail loudly if that client is not live, so A6's
silent death surfaces at the first call instead of the tenth.

### D9 A cart/PII-safe screenshot lane

Not needed this run, but noted: `web screenshot --session` is per-surface and
safe, while `server app screenshot` is whole-window. The distinction is easy to
get wrong when a page holds the operator's name, DOB and address, as this one
did. A `--redact-selector` option would let an agent capture evidence of a
booking without capturing identity fields.

---

## C. What worked well, and should not regress

- **`web await` returning a real value** made every multi-step in-page routine
  possible. `eval` alone would not have done this job.
- **`web ensure` liveness probing** correctly reported `alive/eval_ok` and
  rebuilt a corpse surface once, exactly as documented.
- **Refusal names were accurate** where they existed — `surface_not_mapped` and
  `no web-surface declare … a plain shell` both pointed straight at the cause.
- **The unmapped shadow surface never touched the operator's screen** across
  roughly sixty page interactions. The doctrine held; it is only the login step
  that forces a choice between the doctrine and the task.

---

## D. Reproduction

Site lore holds the working recipes and the exact wall:
`~/gh/ychrome/.claude/skills/ychrome-site-lore/lore/lalpathlabs.com.md`
— slug `custom-panel-cart-no-login` (WORKS) and `otp-login-needs-mapped-surface`
(BLOCKED).
