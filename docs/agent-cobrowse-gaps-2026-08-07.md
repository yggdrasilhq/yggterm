# Agent co-browse gap — the headless engine cannot pay by card (2026-08-07)

Filed from the widgets lobe after an India Post Speed Post booking was driven end to end on
`ychrome ctl` and then had to be **handed to a second agent on the operator's own laptop purely to
pay Rs 23**. Raised by the operator, who was right to challenge the claim that this was a fleet
non-uniformity: **it is not.** The vault is uniform. The gap is one missing verb on one driver.

## What is actually uniform (measured 2026-08-07 ~05:15 IST, dev and jojo)

| probe | dev | jojo |
|---|---|---|
| `ychrome-vault card "<item>"` | ✅ prints `Visa<TAB>holder<TAB>10<TAB>2027<TAB><last4>` | ✅ |
| `yggterm` binary | 55593824 bytes, mtime 2026-08-07 05:11 | **byte-identical, same mtime** |
| `server app web` verb set | `await batch capture-element close cookies devtools do ensure eval fill fill-card fill-vault find frames lease profile read reload screenshot surface totp wait` | **identical** |
| `server app web fill-card` | ✅ present | ✅ present |
| registered GUI clients | **0** | 1 (`:1`, active) |

⇒ The credential plane and the yggterm web plane are the same on both hosts. Nothing is missing
from dev's install.

## ⛔ THE DEFECT: `ychrome ctl` has no card verb

```
$ ychrome ctl fill-card
{"error":"unknown engine verb \"fill-card\"","ok":false}
ychrome: engine replied 404
```

The engine's advertised verbs: `open close pages goto nav wait eval dom shot input console
cookie-import park resume pool metrics budget batch egress identity status`.

**Why this matters beyond one payment.** `ychrome ctl` exists *precisely* so agent browsing needs
no GUI host — `agent-engine.md` §4 is explicit that this is "how agent browsing finally stops
touching jojo", and the co-browse doctrine says to prefer dev because **jojo is the laptop the
human is working on**. But the moment a flow ends in a card payment, the engine cannot finish it,
and the run must migrate to a yggterm web surface, which requires a registered GUI client, which
today means **jojo**. So the one class of task with the strongest reason to stay off the
operator's machine — entering payment credentials — is the one class the engine forces onto it.

**Costed, twice, in 24 hours:**

1. **2026-08-06 RUN 5** — an records fee payment went down the *netbanking* rail and died on a stale
   bank password, burning the session. Stale lore was part of it, but the operative fact was that
   the driver in use had no card verb, so the cheapest correct rail was not reachable from it.
2. **2026-08-07** — an India Post Click-n-Book booking was driven entirely on `ctl` (login, the
   whole five-step wizard, pincode modals, cart) and then **could not be paid**. It was handed to
   a separate agent on jojo at 05:00 for a Rs 23 charge, on the machine the operator was using.

## Feature asks, ranked

1. ⭐ **`ychrome ctl fill-card page_id=<p> item=<name> field=<number|expiry|code|holder> target=<sel>`**
   — mirror `server app web fill-card` on the engine, consuming the **same vault agent
   `card-secret` op** (never the CLI, which prints no PAN), answering `{item, field, chars,
   matched}` — a length, never a value — and leaving the same one line in
   `~/.yggterm/vault/audit.log`. The security model is unchanged; only the driver is new. This
   single verb makes the headless engine payment-capable and restores the engine's own premise.
   A companion `ctl fill-vault` would close the same hole for password-gated flows.

2. **`ychrome ctl fill` is UNDOCUMENTED though it works.** It is absent from the usage banner
   above, yet `ychrome ctl fill page_id=<p> entry=<vault-item>` answered
   `{"entry":"…","filled":"filled","ok":true}` and filled a real login today. An agent reading the
   banner concludes the engine has no credential support at all — which is how the belief that
   "the engine can't do credentials" gets re-derived. Add it to the banner.

3. **`fill-card` answers `matched:false` on fills that landed perfectly** (measured on the IDFC
   3DS page, RUN 6, 2026-08-07 00:15 IST — the field held the full value and the payment
   succeeded). Either fix the matcher or document that `matched` is not an observation. The house
   rule already says never to trust a verb's own success field, but a field that is wrong in the
   *pessimistic* direction makes agents retry a fill that already worked — and on a payment page
   that is the expensive direction to be wrong in.

## What must NOT change

The PAN boundary is correct and no ask here touches it: no verb prints a card number, the secret
lives behind the vault agent's `card-secret` op gated by the unlock alone, and `fill-card` consumes
it without exposing it. Keep that exactly as is.
