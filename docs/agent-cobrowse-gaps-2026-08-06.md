# Co-browse gaps — 2026-08-06, filing a CPC-ITR grievance on the services-desk portal

Written per the standing directive in the `data-fabric` skill ("when co-browse automation fails,
PUSH, then FILE"). The run **succeeded** — grievance acknowledgement 26390914 was filed and
independently confirmed — but it cost about three hours and two failed attempts, and every
sub-failure below is either a defect or a missing verb rather than a site problem.

Site mechanics went to lore instead: `eportal.incometax.gov.in` slug `cpc-itr-grievance-filing`.

Reporter: levers lobe row (`4. levers`), claude-opus-5.

---

## Confirmed defects

### D1 ⛔ `termux-sms.py watch --code-only` returned the MATCH STRING, not the code — FIXED here

```
$ termux-sms.py --host dada watch --match 'OTP for Aadhaar' --code-only
OTP for Aadhaar
```

`cmd_watch` did `code = found.group(0)` — with `--match` set to a phrase, the pattern's own match
IS the phrase. `itr-portal.sh` then types that value **one character per OTP box**.

**Cost: no unattended login through `itr-portal.sh` could EVER have completed**, with a perfectly
timely SMS, for as long as this was in the tree. Lore records the path working 24/27-Jul, so it is
a regression. Two sessions of this lobe read the resulting failure as "UIDAI is not delivering"
and wrote that into a case file.

**Fixed**: `--match` now SELECTS the message and the digits come from the body via `\b(\d{6})\b`.
Verified against a real UIDAI message (checked digit-masked; the code never entered a transcript).

### D2 ⛔ One dropped ssh poll aborted an entire `watch` in ~5 s, printing exactly what "nothing arrived" prints — FIXED here

`fetch()` raises `SystemExit` on any ssh failure, and `cmd_watch` did not catch it. A single Wi-Fi
blip therefore ended a 290-second watch after one poll and printed an empty string — **byte-identical
to a watch that waited the full timeout and matched nothing.**

Measured live: twelve consecutive 290 s chunks fell through in **6–7 s each**.

**Cost, and it is the expensive one: every "no OTP arrived within 150 s" this lobe ever recorded is
void as evidence** — the watcher may never have waited at all. That single ambiguity produced a
false "UIDAI stopped delivering" finding, a follow-up "correction" that built a ~45–70 minute
latency theory on top of it, and hours of chasing a department that had done nothing wrong. The
decisive counter-datum arrived later the same evening: an OTP requested at 20:37 had been
**delivered at 20:38:32, ~90 seconds**, and went unread only because the handset was off Wi-Fi.

**Fixed**: transient poll failures retry to the deadline; a watch that never got a single
successful read now reports `source never answered` instead of `nothing matched`, and `--json`
carries `polls_ok`.

> **The general law worth lifting out of D1+D2:** *a watcher that cannot distinguish "I waited and
> saw nothing" from "I never looked" will be believed, and its silence will be attributed to
> whatever the operator already suspects.* Same family as this plane's standing rule that
> `accepted:true` is an assumption, not an observation.

### D3 ⛔ The fabric scripts are UNVERSIONED — a fix does not propagate

`~/.claude/skills/data-fabric/scripts/*` is in **no git repository on any host** (checked `~/git`
and `~/gh` on dev: no `termux-sms.py` anywhere). The skill is a fleet-shared contract whose
executable half is three independent copies kept in sync by hand.

**Cost here:** D1 and D2 had to be hand-copied to jojo and oc by `scp`, and until that was done
`oc` was still running the broken reader while its skill text claimed the path worked. Nothing
would have told a session on `oc` otherwise. Verified all three now at
`68a99704935861df60d2423711d7a22c`.

**Ask:** give these scripts a versioned home (`gour.top/docs` already holds the SSOT prose; the
scripts want the same treatment) so a fix is `git pull`, not three `scp`s and a hope.

### D4 ⚠ `itr-portal.sh`'s OTP watcher is a hard-coded 150 s with no flag

Real Aadhaar delivery on 2026-08-06 ranged from ~90 s to (apparently) tens of minutes. 150 s is
under the observed range and there is no `--otp-timeout`. The script also `exit 1`s **without
reaping**, which is actually useful — the surface survives on `#/login/enterOtp` with the boxes
live — but that is undocumented behaviour being relied on rather than a designed hand-off.

**Cost:** a bespoke chunked-watch finisher had to be written to take the session over.

### D5 ⚠ `web do type` duplicates the final character

Typed `test grievance` (14 chars), DOM held `test grievancee` (15). Same class as the known
`#panAdhaarUserId` stray-character note in the ITR lore, so it is not site-specific.

**Cost:** low here (the value was replaced), but on a field with a strict validator, or on an OTP
box, a silent extra character is a wrong value that reads as a wrong credential.

---

## What worked and must not regress

- `itr-portal.sh --entry … --sms-host dada --no-ais --keep` drove PAN → secure-access-message
  verification → password → OTP request → **unattended login** once D1/D2 were fixed. That
  end-to-end unattended login is new; do not break it.
- `web await --script` is what makes Angular drivable — see G1.
- `web screenshot --session` captured per-surface evidence with no reveal and no viewport churn.
- `execCommand("insertText")` reaches Angular's FormControl exactly, with no stray characters.

---

## Feature asks, ranked, each with the failure that paid for it

### G1 ⭐⭐⭐ A file-upload verb (`web do upload --selector <input[type=file]> --file <path>`)

**What it does:** attaches a local file to a file input on a surface.

**Why it matters beyond this site:** every government and financial portal that accepts a document
— grievances, records, consumer complaints, trademark responses, insurance claims, KYC — has exactly
this control. The co-browse worklist in `data-fabric` is largely made of such sites.

**The concrete cost:** the grievance was filed **with no attachments**. The form offered *Order
Copy from department*; the s.154 order PDF was staged and ready at `~/tg-shots/rect_order.pdf`; it
could not be sent. `web do` is click/type only, and a 3.75 MB `DataTransfer` injection through an
`eval` was judged too risky mid-session. Mitigated by quoting the join keys (DIN, ARN, challan CIN
+ BSR + serial, DRN) in the body, so the filing is not weak — but if CPC asks for the order copy,
that is a second round trip and weeks of clock that an upload verb would have removed.

### G2 ⭐⭐ Make async-safe reads the documented default for framework-driven forms

**What it does:** nothing new in the engine — a documented `await`-based read helper, and a line in
the plane docs saying so.

**Why it matters:** Angular/React update validity classes and derived state in a LATER tick. A
same-tick `eval` read of `ng-invalid` (or `disabled`, or a computed total) reports the PREVIOUS
state, for every candidate you probe, which reads as "my value never reached the framework".

**The concrete cost:** ~15 minutes and three wrong diagnoses. I concluded in turn that the native
setter was not syncing, then that `execCommand` was not syncing, then that `web do type` was not
syncing — all three DID sync. The real fault (a rejected `>` character) was invisible until the
probe was rewritten with `web await` + a 500 ms settle, at which point it fell out in one call.

### G3 ⭐⭐ `terminal new --model / --permission-mode`, and a numbered-spawn helper

**Why:** the row-hygiene contract (`docs/agent-row-hygiene.md`) requires every spawned row to be
named `N.M <app>: <label>` beneath its parent, and every session to re-assert its own prefix after
the CLI auto-titles. Both are manual today.

**The concrete cost:** two ychrome rows spawned by `itr-portal.sh` were unnumbered orphans until
the orchestrator noticed one and had to ask whose it was. The script cannot number them because it
does not know its caller's row number.

### G4 ⭐ `--otp-timeout` on `itr-portal.sh`, and a documented "leaves the session at `enterOtp`" contract

**The concrete cost:** D4 — a bespoke finisher script, written under session-timeout pressure, to
do what a flag would have done.

---

## The process lesson, which is not a tool ask

**I hand-rolled this entire lane without loading the `data-fabric` skill.** The runbook preamble
said "NO research", and a fresh session reads that as "do not go load skills". The skill is a
contract, not research, and it already contained: the OTP rail, `itr-portal.sh`'s flags, the
shadow-client and load rules, the `el.click()`-no-ops-use-a-pointer-sequence recipe, and — the one
that cost real time — the owner's standing rulings that **Tailscale being off on the phones is BY
DESIGN and must never be recommended as a fix**, and that **an unreachable phone is a MESSAGE to
its owner, not a blocker to wait out**.

Having not read those, I recommended turning Tailscale on (wrong, now corrected in four places)
and set a 60-second poll that waited ~30 minutes for a handset, when one line of email or WhatsApp
was the sanctioned move. The orchestrator has since issued a standing trigger list for loading the
skill; this report is the costed evidence behind it.
