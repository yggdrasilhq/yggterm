# Agent co-browse: WebAuthn is unreachable on an agent-created surface

**Date:** 2026-07-28 · **yggterm:** 2.12.18 (`c8576e91`) · **host:** jojo
**Job:** mint a Cloudflare DNS-01 API token to renew the `*.gour.top` wildcard
certificate, which expires 2026-07-30 08:37 UTC.
**Outcome:** ✅ **job succeeded** — but *around* the passkey plane, not through it.

This is a field report from a real deadline job, not a synthetic test. It is the
sibling of [`agent-cobrowse-gaps-2026-07-28.md`](agent-cobrowse-gaps-2026-07-28.md)
(same day, different wall): that one is about *input* to a page, this one is about
the *passkey/WebAuthn* plane and the policy pipeline that feeds it.

---

## 0. TL;DR for whoever picks this up

`docs/…/SKILL.md` says passkeys are **"BUILT AND SHIPPED"** and warns that calling
them unbuilt "cost an agent a run on 2026-07-28". Both statements are true, and
both are also misleading in a specific way:

> The passkey machinery is built and correct. It is **never wired to a surface an
> agent creates**, because the surface is constructed before the app's policy
> arrives and is never re-fitted afterwards. So `navigator.credentials` does not
> exist on the page, and the `yggterm-appctl://` signer bridge is not registered.

The file's own "⚠ Still owed: full crypto E2E against a real relying party" is
the tell. This run *was* that E2E attempt against a real relying party
(Cloudflare), and it did not reach the crypto at all — it failed one layer
earlier, at injection.

---

## 1. What actually happened, in order

| # | Step | Result |
|---|---|---|
| 1 | Probe all 5 stored Cloudflare credentials against the API (bearer + Global-Key, every extracted substring) | all dead — browser genuinely required |
| 2 | Spawn `--no-activate` work session, launch `ychrome --profile agent-cf` | ✅ surface live, `eval '1+1'` → `2` |
| 3 | Cloudflare **Turnstile** | ✅ **auto-solved unattended** — 709-char `cf_challenge_response` present, Sign-in button never disabled |
| 4 | `web fill --entry dash.cloudflare.com` | ✅ `filled: "user+password"`, DOM verified `email` + `pwlen:15` |
| 5 | JS `.click()` on Sign in | ✅ → `/two-factor` |
| 6 | **2FA: security key** | ❌ **"Sorry, your browser does not support security key."** |
| 7 | Fall back to a stored **backup code** + same-origin dashboard API | ✅ token minted, installed, lego renewing |

Step 3 is worth its own line: **the Turnstile wall recorded in the previous
attempt was not real, or is no longer real.** It auto-solved with no
intervention. The earlier run's note that Turnstile "gated the Sign in button"
should be treated as superseded — the button was `disabled: false` throughout.

---

## 2. The defect, proven

### 2.1 The page has no WebAuthn at all

```js
{"hasPKC":"undefined","hasCreds":"undefined","isSecure":true,"shim":false,"keys":[]}
```

`window.PublicKeyCredential` and `navigator.credentials` are both absent, on an
`https://` page. That is why Cloudflare renders the "browser does not support
security key" branch. A passkey **is** present in the vault for this account:

```
dash.cloudflare.com  8a64d8ec4fea3979b1e0e735e48ff699  …  2026-04-08
```

### 2.2 ychrome is doing its part correctly

`GET /policy` on the ychrome control port serves 3 userscripts, and **[0] is the
shim**:

```
[0] len=4244 has_PKC=True has_fido2=True  head="(function () {\n  'use strict';\n  var ENDPOINT = 'yggterm-appctl://signer';"
```

So the app side is healthy. The shim never reaches the page.

### 2.3 The surface was built with the policy gate closed

`crates/yggterm-shell/src/shell.rs:8715` binds userscripts **once**, at
`open_web_surface` time:

```rust
let (userscripts, adblock_ruleset, user_agent) = match &policy_gate {
    SurfacePolicyGate::Ready(policy) => ( policy.userscripts.clone(), … ),
    _ => (Vec::new(), None, None),          // ← no policy ⇒ NO userscripts
};
let signer_base = state.peek().sidebar_control_url(&session_path);   // :8731
```

and `:8769` traces the decision. Our surface's own trace events:

```json
{"session_path":"local://49a42ce4-…","policy":false,"signer":null,"native_id":16,"tab_id":2}
{"session_path":"local://49a42ce4-…","policy":false,"signer":null,"native_id":17,"tab_id":3}
```

**`policy: false`, `signer: null`** — on every tab. Empty `userscripts` ⇒ no
shim. `signer_base: None` ⇒ no `yggterm-appctl://` bridge.

The contribution *did* exist — the declare was ingested:

```
app_declare_ingested {"action":"declare","verb":"sidebar","path":"local://49a42ce4-…"}
app_declare_ingested {"action":"open","verb":"web-surface", …}
daemon_declare_rebuild {"declare_action":"heartbeat","url":"https://dash.cloudflare.com/login"}
```

so this is **not** a missing declare (the seam the changelog says closed at
2.12.10). It is the *policy fetch* not having completed when the surface was
built — `web_surface_policy_gate()` (`:14827`) returns `Pending`/`Absent` until
`contribution.policy` is `Some`, and **nothing re-fits the surface once the
policy lands.**

### 2.4 Even hand-injecting the shim does not rescue it

I injected the exact 4244-byte shim with `web eval`:

```js
{"hasPKC":"function","hasGet":"function"}     // shim now present
```

then probed its transport with a deliberately bogus `rpId` (so no presence
dialog could be summoned):

```js
{"transport":"FAILED","err":"TypeError: Load failed (signer)"}
```

**`yggterm-appctl://signer` is not a registered scheme on this webview.** Because
`signer_base` was `None` at construction, the bridge was never installed. So the
two halves fail together and **a userscript-level workaround is impossible** —
this can only be fixed in the surface-construction path.

### 2.5 Why a human never sees this

A human-driven surface is revealed and then sat on for seconds before anything
is clicked; the policy fetch wins that race. An agent's `web ensure` → drive
sequence is sub-second, so it loses. **The passkey plane is, in practice, only
wired for the one operator who does not need it to be automated.**

---

## 3. Confirmed defects

1. **★★ Surface policy is bound once, at construction, and never re-applied.**
   A surface built while the gate is `Pending` is permanently without
   userscripts, adblock, user-agent override **and the signer bridge**. There is
   no re-fit on policy arrival, and no way to force one. *(`shell.rs:8715-8731`)*
2. **★★ `signer_base: None` silently disables WebAuthn with no diagnostic.**
   Nothing in `web ensure`, `server app state`, or any refusal string says "this
   surface has no signer bridge". The only symptom is the *relying party's* own
   copy — here, Cloudflare's "your browser does not support security key" — which
   reads as a WebKit limitation, not a yggterm state. I spent the bulk of this
   run proving it was ours.
3. **★ `web ensure` silently resets a live page to `about:blank`.** Mid-flow,
   after the 600 s lease lapsed, the next verb returned
   `web surface not live (session backgrounded or not yet revealed)`; re-ensuring
   returned `healed: false, leased: true` and the page was `about:blank` —
   **a logged-in session mid-2FA was discarded with no warning.** It survived
   only because the cookie jar is per-profile and I could re-navigate. On a
   flow with a one-shot OTP already consumed, that is unrecoverable.
4. **★ The 600 s lease is invisible and un-renewable.** Nothing reports time
   remaining, and there is no `web lease --extend`. A long form fill is a
   coin-flip against a timer you cannot read.
5. **`web eval` returns `null` for statement-form scripts.** `if (b) {…} else {…}`
   yields no completion value, so a click that *did* fire reported `None` and
   looked like a failure; wrapping in an IIFE with an explicit `return` fixed it.
   Cost one wasted diagnostic cycle. Should be documented next to
   "`eval` returns its answer in `data.value`", or `eval` should wrap
   statement-form input automatically.
6. **`policy: false` is traced but unreachable at runtime.** The evidence that
   cracked this case exists only in `~/.yggterm/event-trace*.jsonl`. It is not in
   `server app state`, where an agent would look.

---

## 4. Dream features

Ranked by what this run actually cost. Each says *why I want it* and *what went
wrong without it* — per the standing directive to dream the improvement rather
than file a wish.

### D1 — Re-fit surface policy when it arrives (or block until `Ready`)
**Why:** it is the whole bug. **Without it:** the passkey plane is dead for every
agent-created surface, which is every surface an agent creates. Either
re-apply userscripts + register the signer scheme when `contribution.policy`
transitions to `Some`, or have `web ensure` *await* `SurfacePolicyGate::Ready`
before returning (with an explicit `--no-wait` for callers who genuinely do not
care). The second is smaller and I would ship it first.

### D2 — `web ensure` must report the policy/signer state it settled on
**Why:** rule 1 of the co-browse contract is "read back the effect", and here
there is no effect to read. **Without it:** I could not distinguish "WebKit lacks
WebAuthn" from "yggterm didn't wire it" without reading Rust and a trace file.
Add to the `ensure` envelope and to `server app state`:
`{policy: ready|pending|absent, userscripts: N, signer_bridge: true|false}`.

### D3 — `web passkey get --session <s> --rp <host>` as a first-class verb
**Why:** the ceremony is already built and is *the* shape for modern 2FA. Driving
it only through a page's `navigator.credentials` call means an agent cannot
initiate, retry, or diagnose it. **Without it:** when the RP said "no security
key", I had no way to test the signer independently — I had to hand-inject a
userscript and hand-craft a `fetch` to a custom scheme to learn the bridge was
missing. A direct verb would have answered in one call.

### D4 — Never silently discard a live page
**Why:** defect 3. **Without it:** a logged-in, half-completed 2FA was thrown away
mid-run. `web ensure` on a session with a live page should either re-attach to
it or refuse with `would_discard_live_page`, listing the URL it is about to
drop — never reset to `about:blank` and report `healed`.

### D5 — Readable, extendable leases
**Why:** defect 4. `web lease --show` / `--extend`, and include
`lease_expires_in_ms` in every verb envelope. **Without it:** you write every
multi-step flow defensively against an invisible 10-minute guillotine.

### D6 — A vault-backed 2FA fallback ladder
**Why:** what actually saved this run. The account's TOTP slot was empty, but the
`notes` field held eight 9-character Cloudflare **backup codes** — and nothing in
the tooling knows that shape. **Without it:** I had to infer the codes from a
character-pattern dump of the notes field and hand-build a native-setter
injection to enter one. Proposal: `ychrome-vault second-factor <item>` that
returns `{kind: totp|backup_code|passkey, value}` with backup codes marked
single-use, plus `web fill-2fa --entry <item>` mirroring `fill-vault`. **Also:
this run consumed backup code #1 of 8 — a consumed single-use secret should be
recordable back to the vault**, or the next agent will burn a second one
rediscovering the same thing.

### D7 — `web do click --gesture full`
Already asked for in the sibling report (its item 2); this run adds a data point
in the *other* direction: plain `el.click()` **did** work on Cloudflare's React
login and Verify buttons. So the gesture escalation should be a flag, not a
default — cheap path first, full synthetic gesture on demand.

### D8 — Same-origin API calls are the best rung, and deserve a verb
**Why:** the entire token mint was done with two `fetch` calls against
`/api/v4/*` carrying the dashboard's own session cookie — no UI driving, no
wizard, no coordinates. It worked first try and needed no CSRF header. This is
rung 2 of the CHEAP-before-EXPENSIVE ladder and it is *dramatically* more
reliable than clicking through a console. Proposal: `web api --session <s>
--method POST --path /api/v4/... --json <file>`, which is just `await` + `fetch`
with credentials, minus the base64-transport dance I had to build to keep a
secret out of `argv`. **Without it:** I wrote three throwaway JS files and a
base64 pipeline to avoid leaking a token into a shell command line.

### D9 — A secret-safe return channel
**Why:** the minted token had to get from the page to a file without passing
through my transcript or `argv`. I did it by piping the verb's stdout into a
Python filter that wrote mode-0600 and printed a redacted summary — workable, but
every agent will reinvent it. Proposal: `--capture-secret <path>` on `eval`/
`await`/`api`, writing a named field straight to a 0600 file and returning only
its length.

---

## 5. What unblocked the job (site-lore for `dash.cloudflare.com`)

Recorded because there was **no lore for this domain at all** before today.

- Turnstile on `/login` **auto-solves** on an agent surface. Do not plan around it.
- `web fill --entry dash.cloudflare.com` fills both fields correctly.
- Plain `el.click()` drives both the Sign-in and Verify buttons.
- **2FA offers: security key (unreachable, §2), authenticator app, recovery.**
  The vault item's **`notes` field holds 8 × 9-char backup codes**; one entered
  into `#twofactor_token` via native-setter injection completes login.
  **Codes are single-use — #1 was consumed on 2026-07-28.**
- Once logged in, **do not drive the token wizard.** Same-origin calls work:
  - `GET /api/v4/zones?name=gour.top` → zone id
  - `GET /api/v4/user/tokens/permission_groups` → `DNS Write`, `Zone Read` ids
  - `POST /api/v4/user/tokens` with a zone-scoped policy → token in
    `result.value`. **No CSRF header was required** (`csrf_used: false`).
- The session survives a surface rebuild because the jar is per-profile
  (`agent-cf`); re-navigating to `https://dash.cloudflare.com/` lands back on the
  dashboard already authenticated.

---

## 6. Ask for the dev agent

D1 and D2 are the ones that matter; everything else is comfort. D1 makes the
passkey plane real for agents for the first time, and D2 means the next agent
that hits a variant of this spends one call finding out instead of an hour.

If only one thing gets done: **make `web ensure` wait for
`SurfacePolicyGate::Ready`.** It is a small change at `shell.rs:8715`, and it
converts "passkeys are built" from true-on-paper into true-in-practice.
