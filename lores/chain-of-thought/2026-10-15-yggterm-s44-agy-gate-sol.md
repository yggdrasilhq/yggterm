**accept — Candidate A, with a stricter lifecycle:** use one daemon-owned answerer attached to each eligible **PTY launch**, keep the startup-gate write refusal, and require positive composer evidence before reporting readiness. Do not add a second answerer inside `TerminalWrite`.

I inspected the four staged excerpts. `probe-fail.json` is absent, and [queue-entry.md](/tmp/s44-sol-consult/queue-entry.md:98) ends mid-sentence. The deployed `9377b5ce` failure and 426-character sample are therefore **brief-reported evidence**, not independently inspected evidence. That limits the live-proof verdict, but does not block the design review.

**Q1 — Placement**

**accept — A, corrected from “per row” to “per actual PTY launch/generation.”**

The answerer belongs where the daemon owns the live PTY and rendered grid. The detached probe mints and then only observes; no write hook can resolve that leg by itself. [remote-mint-excerpt.rs](/tmp/s44-sol-consult/remote-mint-excerpt.rs:75)

Require the seat to demonstrate that this hook covers:

- Detached `start-agy`.
- Resume paths that actually launch a new agy process.
- Interactive GUI creation on the GUI host.
- Any repair path that replaces the PTY.

Mounting, selecting, or reopening an **already-running** row must not create another watcher. Deduplicate by runtime identity **plus launch generation**, not merely the reusable row path. Cancel the old watcher on replacement, exit, or reap.

Use the daemon’s existing PTY input mechanism, on its normal owner execution path. Do not write to the slave device: the queue records that approach as ineffective. [queue-entry.md](/tmp/s44-sol-consult/queue-entry.md:36)

**rebut — B duplicates ownership and complicates write semantics.** Waiting inside the guard risks blocking the machinery that must consume output and advance the screen. Forwarding the original payload afterward also requires fresh draft and conditional-submit checks. There is no demonstrated benefit that warrants this second mechanism.

**defer — the literal promise “the owner never sees the gate.”** A watcher that reacts to a painted gate can remove the need for intervention, but cannot guarantee that no GUI frame displays it. That promise needs mounted viewport evidence. If a transient flash remains unacceptable, investigate a verified native trust setting or narrowly scoped presentation handling as a separate correction; do not claim polling guarantees invisibility.

**Q2 — Policy home**

**accept — descriptor-owned, agy-only policy.** Kimi and every other descriptor remain `None`. An empty gate table means **unmeasured**, not permission to inject input. [descriptor-excerpt.rs](/tmp/s44-sol-consult/descriptor-excerpt.rs:13)

**rebut — an unrestricted `confirm: &str` plus “any startup gate” match.** Prefer a small typed action:

```rust
StartupGateAutoAnswer {
    gate: StartupGateId::FolderTrust,
    action: StartupGateConfirm::Enter,
}
```

Map `Enter` centrally to one measured carriage return, `b"\r"`. Do not send arrows, prompt text, or CRLF.

Eligibility must establish **this agy folder-trust modal**, including the selected `> Yes, I trust this folder` option. The question needle alone does not establish the currently selected answer. A human may have navigated to “No,” or a future version may change the default.

Keep detection and authorization separate. Kimi demonstrates why: its descriptor recognizes both directory trust and migration, while those gates have different consequences and policies. [descriptor-excerpt.rs](/tmp/s44-sol-consult/descriptor-excerpt.rs:59)

Timing and attempt limits can initially be shared controller constants. They are operational bounds, not reasons to expand the authorized gate class.

**Q3 — Observation window**

**accept — bounded startup observation, with these initial parameters:**

| Parameter | Initial value | Basis |
|---|---:|---|
| Observation interval | 100 ms | Short reaction delay; classify only changed grids where possible |
| Gate-discovery deadline | 120 s from PTY launch | Leaves time within the staged probe’s 150 s budget |
| Post-answer readiness window | Up to 10 s, capped at 130 s from launch | Explicit verification time after discovery |
| Automatic Enter attempts | **1** | One Enter is measured; repeated Enter safety is not |

These are starting bounds to validate, not measured guarantees.

**rebut — “watch only while classified `startup_gate`” and “disarm on the first non-gate classification.”** Before the gate paints, the row may be blank or signing in. The descriptor explicitly records a sign-in phase that can outlast 15 seconds. [descriptor-excerpt.rs](/tmp/s44-sol-consult/descriptor-excerpt.rs:191)

Use this lifecycle:

```text
AwaitStartup
  exact authorized gate → send one Enter → VerifyReady
  positive composer readiness → stop: already_ready
  deadline / exit / replacement → stop with reason

VerifyReady
  positive composer readiness, gate absent → stop: ready
  deadline / exit / replacement → stop with reason
  otherwise → observe; send no further input
```

A blank, missing, or ambiguous sample is **unknown**, not readiness.

Once stopped, the watcher stays disarmed for that launch generation. A later modal days afterward cannot revive it. A genuinely new PTY launch gets a new bounded lifecycle.

**rebut — retrying because the same gate remains visible.** An unchanged grid cannot prove that the previous Enter was discarded rather than buffered or awaiting processing. A second Enter can reach the composer after the first clears the modal. Increase attempts only with evidence establishing safe retry conditions.

**Q4 — Verification and traces**

**accept — positive readiness after the answer is mandatory.** “Write accepted,” “gate needle disappeared,” and “screen has characters” each prove less than the required result.

Read the **rendered active grid** through `session_screen_plain_rows`; the descriptor expressly rejects raw-stream matching for this modal. [descriptor-excerpt.rs](/tmp/s44-sol-consult/descriptor-excerpt.rs:30)

Require gate absence **and** the established post-gate composer signature. Do not use the generic footer hints as sufficient evidence: agy’s hints include `gemini`, `enter`, and `?`, while the gate itself contains model and Enter chrome. [descriptor-excerpt.rs](/tmp/s44-sol-consult/descriptor-excerpt.rs:217), [queue-entry.md](/tmp/s44-sol-consult/queue-entry.md:29)

Proposed named events:

| Event | Essential fields |
|---|---|
| `startup_gate_auto_answer_armed` | runtime identity, launch generation, CLI kind, policy ID, deadlines |
| `startup_gate_auto_answer_observed` | elapsed time, grid revision, gate ID, selected-option match |
| `startup_gate_auto_answer_write` | attempt, input action, byte count, write result |
| `startup_gate_auto_answer_ready` | elapsed time, gate absent, composer evidence, grid revision |
| `startup_gate_auto_answer_stopped` | reason, attempt count, elapsed time, final classification |

Terminal reasons should distinguish `already_ready`, `gate_not_seen_before_deadline`, `gate_persisted`, `gate_cleared_composer_unconfirmed`, `write_failed`, `selection_mismatch`, `process_exited`, and `generation_replaced`.

Do not log whole screens, prompts, or conversation text. A successful PTY write gets a **write event**; readiness gets its own later event.

**Q5 — Write guard after auto-answer**

**accept — retain the refusal during the race.** A programmatic payload must remain unsent while the gate is visible. The existing gate-before-draft order is necessary because the selected trust option resembles a composer draft. [write-guard-excerpt.rs](/tmp/s44-sol-consult/write-guard-excerpt.rs:10)

**rebut — assuming exactly one NACK or assuming all clients already retry.** Several refusals are possible during startup; a failed watcher may leave the row gated indefinitely. Require evidence for the relevant caller’s bounded readiness-and-resend behavior. The resend must run the ordinary guards again.

Keep the stable `STARTUP_GATE_REFUSAL_MESSAGE` prefix and its parser. [write-guard-excerpt.rs](/tmp/s44-sol-consult/write-guard-excerpt.rs:136) Correct the agy detail text to explain that automatic startup confirmation is pending or failed, and that the payload was refused. Preserve the interactive remedy for descriptors without the policy.

**rebut — silently answer inline and admit the original write.** It hides an additional state-changing action inside a write operation and adds another race owner. If acceleration is eventually needed, the guard may notify the **same** controller without admitting the payload or resetting its deadline/attempt count.

**Q6 — Mounted GUI rows**

**accept — the PTY-owner daemon is the correct common mechanism for mounted and unmounted rows.**

**defer — “no double-delivery” until the input path is inspected and driven live.** Terminal output does not inherently become another keyboard event, but the staged excerpts do not show the GUI input wiring.

Require proof that:

- No GUI-side trust answerer sends a second Enter.
- Mounting does not arm another daemon watcher.
- The synthetic Enter is sent through one input path.
- Subsequent ordinary GUI typing reaches the composer once.

Serialize the controller with daemon input handling and revalidate the generation and selected option immediately before writing. Human input while armed must invalidate any cached selection observation. If safe revalidation is unavailable, stop rather than acting on the assumed default.

DA2 remains protocol handling. It must neither trigger nor substitute for the trust decision.

**Q7 — What else breaks**

**accept — preserve the following boundaries and change only the intended expectations:**

- **Guard tests stay valid.** A still-visible agy gate must still refuse programmatic payloads, even with policy enabled. Assert that gate refusal outranks the draft refusal and that human writes retain their existing path.
- **Kimi remains untouched.** Neither directory trust nor migration gets daemon auto-confirmation. Its deliberate scratch-directory suite handling remains separate.
- **Probe polling may need correction.** The excerpt does not include `poll_until` or `screen_ok`, so immediate gate failure behavior is unknown. An agy gate seen briefly during startup must remain truthfully classified as gated, while the connection poll continues within its deadline. Fail on controller failure or deadline; pass only on positive composer readiness.
- **Do not replace the false-positive marker with a new shortcut.** “Antigravity CLI” inside the gate still proves no connection. [queue-entry.md](/tmp/s44-sol-consult/queue-entry.md:46)
- **Defmiss meaning remains independent.** Auto-trusting a newly fabricated fallback conversation must not turn it into evidence that the requested conversation resumed. Keep identity, existence, and refusal assertions intact.
- **Watcher failure remains visible.** Do not set a durable “connected” or “trusted” flag merely because Enter was sent. Current screen evidence remains authoritative.
- **Lifecycle handling needs coverage.** Reuse, remount, replacement, and daemon handover must not produce duplicate or obsolete writers. The staged excerpts do not establish those hooks.

The historical memory corroborates the original false-pass mechanism; the supplied October 15 ruling supersedes its earlier policy-pending state.

1. **Complete the evidence packet:** add the missing baseline JSON and complete queue tail; retain the original honest failure.
2. **Audit actual PTY creation paths:** identify the common owner hook for detached start, GUI start, resume, and replacement; identify generation and cancellation seams.
3. **Add agy-only typed policy:** authorize the exact folder-trust gate and measured Enter action; leave all other descriptors disabled.
4. **Implement one bounded controller:** delayed discovery, selected-option validation, one Enter, positive readiness verification, terminal disarm, and structured traces.
5. **Preserve the write guard:** keep gate-before-draft ordering and refusal parsing; correct agy remedy text and verify caller retry behavior.
6. **Run focused deterministic checks:** delayed gate, already-ready launch, persistent gate, ambiguous/blank output, changed selection, Kimi gates, duplicate mount, and replaced generation. Assert input counts and absence of payload leakage.
7. **Correct E2E waiting semantics:** permit transient agy gate observation within the bound while retaining honest failure classification and independent defmiss assertions.
8. **Deploy through the campaign’s normal build plane and prove both surfaces:** a fresh unmounted row reaches the composer with zero probe writes; a fresh GUI-created row requires no intervention. Capture controller traces, screen readback, and mounted visual timing before closing Want (2).


