# 2026-10-15 yggterm s49 — F1(c) Paint loss site (gpt-6.1-sol HIGH)

Consult node for sitting 49. Board plan ACK-a505c8d0ff. Brief staged raw at
/tmp/s49-sol-consult/brief.md (dev, EPHEMERAL); the verdicts below reshaped
the unit: Q1 rank-by-investigation + identity-first (site A excluded for the
receipt-delivering invocation; undefined-term site B WEAKENED — nextPaintKey
dereferences term.cols BEFORE the attempt counter), Q2 gated/staged/
identity-preserving/bounded ledger with attempt-recorded BEFORE payload
construction and returned-sync (never "enqueued"), Q3 receive-before-dispatch
observation + the outcome map, Q4 observation-only first (recovered
observations never set painted=true late), Q5 the two-invocation confounder
accepted + the dedupe-key-commits-before-outcome class named. The rig ran
AFTER these corrections; the measurement (i2send run3) closed F1(c)
measured-safe: 6/6 returned-sync, 3/3 Rust receives, the guard was the loss.

## Verdicts (full transcript follows)

- Q1 accept-conditional: rank by investigation priority; establish
  same-invocation identity first. A FIFO cannot lose old messages while a
  later one drains — early-paint-lost + late-receipt-delivered needs
  destruction, silent rejection, a second consumer, decode rejection, or
  different queue identities.
- Q2 accept-conditional: gated, staged, identity-preserving, globally
  bounded; record attempt-recorded before the payload literal; rethrow
  preserved; returned-sync label until promise semantics verified.
- Q3 accept-conditional: add receive-before-dispatch evidence; paint_ready
  is a guarded FIRST-ARRIVAL trace, not an arrival counter — add a compact
  receive observation before the witness filters.
- Q4 accept-conditional: observation-only first; a recovered historical
  observation never sets current painted=true; witness integration needs a
  shared eligibility predicate (identity + fence generation captured at
  observation time).
- Q5 accept-conditional: (a) queue-ownership audit deferred; (b) VALUES are
  the serialization discriminator (undefined dropped, non-finite -> null);
  (c) suppression guard provenance deferred; (d) two-invocation confounder
  ACCEPTED — identity per record. Plus: the dedupe key commits before the
  send outcome (a transient failure becomes permanent silence — a real bug
  class regardless of the loss site); the receipt is a lane observation
  until same-invocation identity is proven.

## Full transcript

Reading additional input from stdin...
OpenAI Codex v0.162.1
--------
workdir: /tmp/s49-sol-consult
model: gpt-6.1-sol
provider: openai
approval: never
sandbox: danger-full-access
reasoning effort: high
reasoning summaries: none
session id: 01a13f89-817c-7541-b556-149235675917
--------
user
# YGGTERM s49 consult — F1(c): localize the Paint event loss site (measure-first)

You are consulting for the yggterm campaign. Verdict format per question:
`accept | accept-conditional(<corrections>) | rebut(<why>) | defer(<what evidence is missing>)`.
You are judging an ATTACK PLAN and a DISCRIMINATING EXPERIMENT DESIGN, not
implementing. Be adversarial: if a candidate site cannot produce the observed
signature, say so; if the experiment cannot discriminate, say so.

## CONTEXT (campaign state, provenance)

- Repo: ~/gh/yggterm on dev, main 645562c3 (2026-10-15). Rust shell (dioxus-desktop
  0.7.10 vendored) drives xterm.js in a WebKit page via eval scripts.
- F1 family root (MEASURED, older): the mount eval's page->Rust event leg can
  shed events when the captured dioxus.send dies with its receiver under spawn
  churn. The e-r6 fix (2026-10-10) passes each invocation its OWN eval binding
  (`__yggInvocationBridge`) which wins over the lexical capture; e-r7 measured
  the transport HEALTHY end-to-end for kind:"debug" tags on the fence rig
  (mailbox_attempts=[1,2] arrival_attempts=[1,2]).
- s47 (2026-10-15, i2prov rig, gated probe YGGTERM_TEST_PAINT_PROVENANCE=1,
  oc-owned remote-agy row, 250ms samples): an ORGANIC COLD mount incremented
  paintSendAttemptCount (3 attempts, paint key oscillating 3<->4) but ZERO
  kind:"paint" events reached Rust (no paint_ready trace; the remote row has
  the paint witness trace ARMED). In the SAME run the AppliedContent receipt
  (wrote=349, blank=false, gen=1) was observed page-side at ~mount+2s and, on
  the s47 FIX lane, was DELIVERED to Rust via the page push
  (applied_content_promoted traced) — i.e. the sendTerminalEvent pipe WORKED
  for AppliedContent in that run.
- Consequence of zero Paint arrivals: the organic paint witness never advances;
  readiness previously leaned on it (s47 landed L1/L2 to decouple readiness
  from Paint — readiness recovery is now separate; this unit is the Paint
  CHANNEL itself).

## RAW EVIDENCE 1 — page side: the send path (terminal_scripts.rs @ main 645562c3)

Mount fn head (per-invocation; e-r6 invocation bridge):

    const __ygInvocationApi =
        !{transport_stale_capture} &&
        __yggInvocationBridge
            && typeof __yggInvocationBridge.send === "function"
            && typeof __yggInvocationBridge.recv === "function"
            ? __yggInvocationBridge
            : null;
    const terminalDioxusApi = __ygInvocationApi
        || (typeof dioxus !== "undefined" ? dioxus : null);
    const terminalDioxusSend =
        terminalDioxusApi && typeof terminalDioxusApi.send === "function"
            ? terminalDioxusApi.send.bind(terminalDioxusApi)
            : null;
    const sendTerminalEvent = (payload) => {{
        if (terminalDioxusSend) {{
            terminalDioxusSend(payload);
        }}
    }};
    // NOTE: no else. A null send API is a SILENT no-op. No outcome ledger.

emitPaint (fires from rAF render-probe / paint-recovery / manual-redraw /
ensureVisibleHost; the nextPaintKey gate dedupes — sends only on key CHANGE):

    const nextPaintKey = JSON.stringify([
        host.childElementCount, visible, Boolean(xtermRoot), Boolean(screen),
        Boolean(viewport), Boolean(rowsLayer),
        Number(term.cols || 0), Number(term.rows || 0),
    ]);
    if (nextPaintKey !== lastPaintKey) {{
        lastPaintKey = nextPaintKey;
        paintSendAttemptCount += 1;                    // BEFORE the send
        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
            window.__yggtermXtermHosts[hostId].paintSendAttemptCount = paintSendAttemptCount;
            window.__yggtermXtermHosts[hostId].lastPaintKey = lastPaintKey;
        }}
        sendTerminalEvent({{
            kind: "paint",
            child_count: host.childElementCount,
            xterm_present: Boolean(xtermRoot),
            screen_present: Boolean(screen),
            viewport_present: Boolean(viewport),
            rows_present: Boolean(rowsLayer),
            cols: term.cols,
            rows: term.rows,
        }});
    }}
    paintCount += 1;                                   // AFTER the send
    // paintCount proves emitPaint RAN (advances regardless of send outcome).

emitPaint body also does rebindCurrentHost('emit_paint', true) FIRST (re-resolves
the host element; does not touch `term`), then DOM probes, then the send. The
payload object literal dereferences `term.cols` — if `term` were undefined the
literal build THROWS. Call sites: three rAF callbacks and ensureVisibleHost;
in the manual-redraw rAF the enclosing try/catch covers term.refresh but
emitPaint() is OUTSIDE the try; render-probe rAF calls it unguarded.

## RAW EVIDENCE 2 — Rust side: decode and drain

dioxus-document-0.7.10 Eval::recv (unmodified upstream):

    pub async fn recv<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, EvalError> {
        let json_value = poll_fn(|cx| ... evaluator.poll_recv(cx) ...).await?;  // ONE message
        serde_json::from_value(json_value).map_err(EvalError::Serialization)
    }

TerminalJsEvent has a MANUAL Deserialize (terminal_protocol.rs:1072) that never
fails silently:

    impl<'de> Deserialize<'de> for TerminalJsEvent {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> {
            let value = Value::deserialize(deserializer)?;
            Ok(match serde_json::from_value::<TerminalJsEventWire>(value.clone()) {
                Ok(event) => event.into(),
                Err(error) => TerminalJsEvent::Ignored {
                    reason: error.to_string(),
                    value: redact_secrets_for_trace(value),
                },
            })
        }
    }

Ignored IS traced (viewport.rs:11628, event name js_event_ignored, value
verbatim-redacted). The Paint wire variant (tag="kind", snake_case):

    Paint {
        child_count: usize, xterm_present: bool, screen_present: bool,
        viewport_present: bool, rows_present: bool, cols: u16, rows: u16,
    }   // ALL fields required, no serde(default) on Paint

The page payload sends exactly these field names. The mount loop's drain arm:

    event = eval.recv::<TerminalJsEvent>(), if !suppress_mount_ipc_for_test => { ... }

This is the ONLY drain of the mount eval's queue found (grep). Paint handler
viewport.rs:9142 sets witnesses and traces paint_ready ONLY under
`if !terminal_paint_seen` where `terminal_paint_seen = !is_remote_resume_session`
(viewport.rs:7250) — on LOCAL organic mounts the paint_ready trace never fires
(witness advance is separate); the i2prov rig used a REMOTE row, where the arm
is armed, so its zero-arrival read stands.

## RAW EVIDENCE 3 — s47 rig facts (verbatim from the campaign queue)

- "the organic mount attempts Paint sends page-side (paintSendAttemptCount 3,
  key 3<->4), but no Rust paint_ready/first-paint-witness event is observed"
- "the page independently holds an applied-write receipt before churn (gen 1,
  wrote 349)" — page entry, ~mount+2s, read over eval-return
- On the s47 lane: "the page-push delivered the receipt — applied_content_promoted"
  (the same sendTerminalEvent pipe)
- The probe samples were paintCount + paintSendAttemptCount + lastPaintKey at
  250ms; the queue record does NOT state the paintCount value or the TIMING of
  the 3 paint attempts vs the ~mount+2s receipt delivery
- The run ended in churn: "cancellation then latched failure and re-selection
  superseded the surviving loop (bootstrap_owner_superseded_during_loop)"

## QUESTIONS

Q1 (site ranking): candidates — (A) send-null silent no-op (terminalDioxusSend
null at paint time); (B) payload-build throw before the send (attempt counted,
send never called; discriminator: paintSendAttemptCount advances while
paintCount does NOT); (C) send-threw (enqueue error); (D) send-returned but
queue undrained (receiver superseded/exited) or decode->Ignored (already
traced, but nobody grepped js_event_ignored in the s47 trace). Given
AppliedContent DELIVERED through the same pipe on the lane (timing ~mount+2s,
after the paints?), rank the sites and state what signature each must produce.
Is "paints died in an early shed window, receipt survived later" coherent with
an organic COLD mount whose invocation bridge was valid at invocation time?

Q2 (ledger design): I plan to instrument sendTerminalEvent with a per-send
outcome ledger on the host entry: outcome in {no-send-api, returned,
threw:<msg>} + ts + mount attempt + payload kind, bounded (e.g. last 32),
published on window.__yggtermXtermHosts[hostId] AND a window array, read over
the proven eval-return leg. Unconditional in production (writes only on
paint-key change / event sends — bounded by existing dedupe) vs
YGGTERM_TEST-gated. Which, and what bounds? Any way the ledger itself
perturbs timing (the s48 lesson: instrumentation must not change the shed
window)?

Q3 (discriminating rig bar): the re-run reads the ledger + greps the SAME
trace for js_event_ignored / paint_ready / applied_content_promoted /
mount-loop terminal events (mount_task_dropped, bootstrap_owner_superseded,
js_bridge_closed). What outcome mix NAMES each site? Give the minimal
RED/GREEN pair for the eventual fix lane: RED = loss present with the ledger
showing the dead outcome on main; GREEN = Paint arrivals at the witness site
+ paint_ready trace on the remote rig. Is GREEN's bar sufficient to declare
the CHANNEL fixed, or does it need an exactly-once/duplicate bar too?

Q4 (fix layer, conditional): IF the measured site is A (send-null) or D
(undrained), the obvious fix mirrors the F1-input dual-leg: a page-side paint
ring (per-mount monotonic ids, byte-bounded) drained by Rust over the
eval-return leg, exactly-once by id cursor. Does an out-of-band paint
reconciliation risk MISORDERING against the output fence (the seed repaint
arms a fence; a late paint ring drain could report painted=true before the
fenced seed installs)? Should the ring drain report into the SAME witness
predicate the organic Paint handler uses, or is a separate observation-only
record the honest scope (the s47 L2 already decoupled readiness)?

Q5 (blind spots): what did the seat miss? Specifically consider: (a) is there
ANY second consumer of the mount eval queue that could interleave-drain Paint
messages; (b) does dioxus.send serialize tagged payloads in a way that could
drop Paint specifically (field order, extra fields, non-finite numbers);
(c) the `if !suppress_mount_ipc_for_test` guard — is that test-gate ever true
in production paths; (d) could the paint key oscillation 3<->4 mean the sends
were from TWO different invocations (cold closure + warm re-invocation with a
dead bridge) rather than one organic mount?
codex
I’ll check the staged evidence and campaign context, then assess which sites can explain the loss and whether the proposed ledger can distinguish them.
exec
/bin/bash -lc pwd in /tmp/s49-sol-consult
 succeeded in 0ms:
/tmp/s49-sol-consult

exec
/bin/bash -lc "rg -n 'Unified October|campaign-yggterm|Paint|paint' /home/pi/.codex/memories/MEMORY.md" in /tmp/s49-sol-consult
 succeeded in 0ms:
80:- extensions/ad_hoc/notes/20261015T023625Z-fleet-home-pi-gh-yggterm-campaign-yggterm-55a7473c7cae.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-15T02:36:25Z, thread_id=not-applicable, current unified campaign door) [ad-hoc note]
81:- extensions/ad_hoc/notes/20261014T223650Z-fleet-home-pi-gh-yggterm-campaign-yggterm-8c7bdb446c13.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-14T22:36:50Z, thread_id=not-applicable, late unified-campaign state) [ad-hoc note]
82:- extensions/ad_hoc/notes/20261010T043242Z-fleet-home-pi-gh-yggterm-campaign-yggterm-7d0413ca6a24.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-10T04:32:42Z, thread_id=not-applicable, sitting 29 receipt-provenance closure and current queue) [ad-hoc note]
83:- extensions/ad_hoc/notes/20261010T005426Z-fleet-home-pi-gh-yggterm-campaign-yggterm-71e67ea72d1e.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-10T00:54:26Z, thread_id=not-applicable, sitting 28 campaign handover) [ad-hoc note]
84:- extensions/ad_hoc/notes/20261010T004705Z-fleet-home-pi-gh-yggterm-campaign-yggterm-b8f99d38f111.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-10T00:47:05Z, thread_id=not-applicable, unified door update) [ad-hoc note]
85:- extensions/ad_hoc/notes/20261009T221334Z-fleet-home-pi-gh-yggterm-campaign-yggterm-89a00ade4b87.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-09T22:13:34Z, thread_id=not-applicable, latest unified-door handover, rig laws, and dev-return landing order) [ad-hoc note]
86:- extensions/ad_hoc/notes/20261007T220133Z-fleet-home-pi-gh-yggterm-campaign-yggterm-f6d45b296dc4.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-07T22:01:33Z, thread_id=not-applicable, current Q7 application-receipt state and handover) [ad-hoc note]
87:- extensions/ad_hoc/notes/20261007T201816Z-fleet-home-pi-gh-yggterm-campaign-yggterm-e53cbef434d0.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-07T20:18:16Z, thread_id=not-applicable, current unified campaign door) [ad-hoc note]
88:- extensions/ad_hoc/notes/20261005T070710Z-fleet-home-pi-gh-yggterm-campaign-yggterm-e7be6d893e53.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-05T07:07:10Z, thread_id=not-applicable, authoritative unified campaign door) [ad-hoc note]
89:- extensions/ad_hoc/notes/20261005T034036Z-fleet-home-pi-gh-yggterm-campaign-cli-integration-11229-paint-zombie-202-17d280a8fe5c.md (cwd=/home/pi/gh/yggterm, rollout_path=not-applicable, updated_at=2026-10-05T03:40:36Z, thread_id=not-applicable, paint-zombie follow-up) [ad-hoc note]
94:- campaign-yggterm, `docs/pending-bugs.md`, ⚙ FAMILIES, root-cause or nothing, viewport-level proof, [11.226], Q7, `AppliedContentReceiptSpec`, startup_gate, CLI_MARKERS, `connection_probe.py`, `server app terminal send`, dev, `YGGTERM_SKIP_SELF_UPDATE` [ad-hoc note]
104:- `campaign-yggterm` supersedes `campaign-11x-cli-integration`, `campaign-ux-speed`, and trace-fixing doors. The live routing order is the unified door, `docs/pending-bugs.md` families index, `docs/docs-ssot.md`, then `infra/yggterm` for attack plans/outcomes. [Task 1] [ad-hoc note]
106:- [11.226] corrected a false Antigravity E2E pass: `CLI_MARKERS` matched “Antigravity CLI” inside a folder `startup_gate`. ANSI-strip screen text before matching and require `? for shortcuts`/non-gate state; return an explicit refusal for a painted trust gate. All agy-minting scenarios remain honestly red until an owner gate policy exists. [Task 1] [ad-hoc note]
156:scope: trace-led responsiveness work for spawning, dragging, context menus, chords, switching, and repaint; use when a yggterm interaction feels slow or visibly lags. [ad-hoc note]
200:- [11.213], `ui/window/resized`, `xterm_resize`, `resize_repaint`, shift-drag, `live_session_reorder_persisted`, `payload.payload`, input-to-paint, `uxprobe` [ad-hoc note]
204:- when a yggterm interaction feels slow, treat spawning, dragging, right-click, chorded actions, switching/attach, and resize-repaint as real user paths -> measure verb-to-effect on the actual path instead of accepting an isolated timing or screenshot. [Task 1] [ad-hoc note]
211:- The current door fixes the acceptance bar at <=100 ms input-to-paint for interactive actions and pointer tracking within one 16.7 ms frame for continuous drag; both latency and the correct outcome must be asserted. Use `ytrace attach` + `drain` rather than pulling 100 MB generations, window by `ts_ms`, count `(pid,name)` before storytelling, and remember terminal-js data nests under `payload.payload`. [Task 1] [ad-hoc note]
212:- Owner live rows are not probe material. Use synthetic rows, observe a faithful paint record/screenshot rather than a DOM grid, and ask before installing missing kernel tracers (`perf`, `bpftrace`, `trace-cmd`) on the GUI host. [Task 1] [ad-hoc note]
215:- [11.171] landed, deployed, and live-proved a switch paint marker: pair user `session/activation` to `terminal_mount/begin`, `xterm_paint/first_frame`, and `xterm_paint/settle` by `host_id` (else `session_path`), never time proximity. The first honest baseline was 8/8 correct but 1340 ms activation-to-first-frame and 2165 ms settle; the dominant activation-to-mount/remount plus JS-wait leg is [11.172], not a reason to call switching fast. [Task 1] [ad-hoc note]
217:- [11.176] is landed and live-proven: 7/7 spawn probes painted with zero permanent blanks, while the remaining first-mount tail is [11.178], not evidence that spawning is fast. Local retained raises serve only after the row is daemon-owned and has reached `was_ever_ready`; the remote half of [11.179] remains an owner-gated remote-ownership decision because its rows stay `daemon_owns_runtime:false`. [Task 3] [ad-hoc note]
220:- The current action map has live-proven resize (`ui/window/resized` -> `perf/xterm_resize` -> `xterm_paint/resize_repaint`) and shift-drag block moves. Pace resize drivers because the 150 ms resize throttle can swallow a discrete post-unmaximize commit; join on identity/grid rather than timing alone. [Task 4] [ad-hoc note]
227:- Symptom: a tracer/probe gives a fast number with no outcome assertion -> cause: measuring transport or synthetic traffic without the actual action contract -> fix: instrument input, paint, and correctness together; escalate only after app-plane attribution is genuinely exhausted. [Task 1] [ad-hoc note]
228:- Symptom: a switch probe reports no end despite a visible switch -> cause: filtering the first-reveal `ui/reveal` family, which does not fire on row-to-row remounts -> fix: use the [11.171] mount/xterm paint chain and its identity join. [Task 1] [ad-hoc note]
231:- Symptom: a resize or drag pair looks complete but misses the user-visible commit -> cause: throttle/coalescing or a join based only on time proximity -> fix: pace the driver, assert target/order or final grid, and join the paint chain by stable action identity. [Task 4] [ad-hoc note]
263:- [11.160], [11.162], [11.164], [11.165], [11.166], `store_holds_id`, `machine_unanswered`, `agent_identity_reset_refused`, `host_anomaly`, `anomalies.jsonl`, `visible_paint_demand_rescue`, `remote_launch_refusal_frame_learned`, `gate-screen` [ad-hoc note]
310:- Recent 11.159 proof: an ensure path must accept a live version-compatible bind-lock owner rather than demand-spawn a lock-refused child; compare build IDs carefully because mtime-derived IDs can collide after a copy burst. 11.160 records a named remote launch refusal rather than raw/lying paint, but bound-row end-to-end driving remained owed in the campaign note. [Task 1] [ad-hoc note]
311:- [11.162] closed on `main` `ad5e04c43692`: reader translation writes the contract line to the runtime and the learn arm stamps at translate time; live proof on the owner’s medgraph row showed learn within one tick, zero respawns, and `server connect` returning the stamp’s own words. Do not classify a CLI's own text as a yggterm refusal; `visible_paint_demand_rescue` remains a separate fourth spawn-driver diagnosis. [Task 2] [ad-hoc note]
323:- E2E connection claims require the real daemon verbs and the row’s own `server terminal screen`, a red baseline against the unfixed binary, and a re-runnable in-repo probe that reaps every row (`tools/e2e/connection_probe.py`). [11.182]–[11.184] were 5/5 live-proven; [11.187] remains open because the retained-host re-activation does not re-arm the viewport pump, so output dies seconds after birth and freezes the birth paint. Hand-deployed binaries are not durable: [11.132] provisioner drag-back can overwrite them within about 20 minutes; deploy via `main`/ygg-ci. [Task 3] [ad-hoc note]
332:- Symptom: an agent row paints raw `no terminal spec` while status claims running/idle -> cause: remote runtime record disappeared across daemon rotation while transcript survived -> fix: learn/stamp yggterm-owned refusal wording, gate against a current stamp, and distinguish remaining GUI remount respawns from the ensure path. [Task 1] [ad-hoc note]
604:- ytrace 0.2.4, `collect_records`, `parse_since`, epoch-ms, 50k collector cap, `lane/trace/amber-ghost-cache`, ghost frame, fresh Paint, 256 KiB FIFO, `snapshot_shared()`, `TerminalSurfaceStatus`, `needs_attention()`, 100 MiB, 4 GiB
614:- `TerminalSurfaceStatus`, `needs_attention()`, healthy in-flight bytes, `input_unanswered_ms`, `fresh Paint pending`, `snapshot_shared()`, modal-only write path, `cargo check -p yggterm-shell --lib`, 2114 tests, 3.2.91, `50673082c`, `0c901ddb7e84`
626:- For live performance, use windowed totals before trusting a rate; a p50 approximately equal to p95 at a timeout indicates a timeout, not workload. Attribute web memory by family-level committed memory, not a recycled single webview PID. Pixel truth is a faithful screenshot or same-frame paint record, not matching DOM and PTY grids alone. [Task 1] [ad-hoc note]
632:- Transport contract: daemon owns PTY/session truth; GUI owns only transport/render observation. A real failed write holds queue head and retains later input in a bounded 256 KiB FIFO as byte count only. A retained xterm canvas is a ghost frame: recovery may clear transport uncertainty, but ghost amber clears only after fresh xterm bytes produce verified Paint. Offline/red outranks steady amber attention; healthy working may blink only with no attention. Recovery work must return through apply channels, never await daemon/network RPC in the terminal event loop. [Task 3] [ad-hoc note]
633:- The 50–100 ms amber flash while normally typing was a false positive: healthy in-flight writes set `TerminalSurfaceStatus` attention at enqueue, so `needs_attention()` flashed before completion. Healthy in-flight bytes are telemetry only; amber requires degraded transport or held ghost, while `input_unanswered_ms` remains separate suspicion. Keep tooltips truthful (`fresh Paint pending`, `transport is healthy`, `input is live`). [Task 4] [ad-hoc note]
643:- Symptom: a cache/recovery change is called live-proven from a deploy acknowledgement, screenshot, or daemon frame -> cause: these are separate witnesses and no permitted outage/fresh-Paint falsifier ran -> fix: retain the owed marker; do not manufacture a controlled outage on the active user session. [Task 3] [ad-hoc note]
719:- Use `tools/pty-proof.py`: raw-PTY, delta-aware, liveness-gated proof. Never type into a live draft without Escape and never send while probing. TUIs paint deltas, so assert escape-stripped windows and resize for complete frames. Wave 0.6.8 had 13/13 PTY/bun/probe checks, but owner GUI validation, main merge/npm publish, and jojo/oc dev spread remain owed. [Task 2] [ad-hoc note]
721:- For harnesses, poll rather than fixed-settle, never put a detection needle in prompt text, check dialog-ready paint, and use disk artifacts rather than a transient duration footer as completion proof. A live relay is hands-off: fixes ride the next spawn, not a kill/answer through the live draft. [Task 2] [ad-hoc note]
724:- For parity waves, one md5-pinned binary needs load-gated full PTY runs; a red family may be accepted only when it is green in another full run on the same binary. A dialog paint-geometry migration invalidates value-token readers as well as title needles: sweep harness readers before trusting a TH/L0 failure. [Task 3] [ad-hoc note]
728:- The campaign later reached 0.6.63/R53: named parity waves are closed, including DIFF-VIRTUALIZATION and TRANSCRIPT-VERBOSITY; remaining work is debt, not a new parity wave. `dezombify()` before each new proof family mitigates a parked focused delete card that can repaint mid-run. The door says the accumulated 0.6.39–0.6.63 lane was unmerged into the older release main; check actual lane/main state before a release claim. [Task 3] [ad-hoc note]
736:- Symptom: unrelated parity seats collide or a regression is attributed to the app -> cause: a shared `dist/` was mid-proof or a pending proof ask repainted into the composer -> fix: post a hail/wait before takeover, use a private floor, and inspect proof-store dumps before accepting a harness signal. [Task 2] [ad-hoc note]
737:- Symptom: a PTY cluster appears after a dialog migration -> cause: an old proof process loaded a stale harness, a reader tracked the former padding/token geometry, or `pf636` proof-store zombie card stole Enter -> fix: verify process file-load time and md5, inspect the dump, then repair the reader only if paint disproves it; never call this an app regression from a microscope alone. [Task 3] [ad-hoc note]

exec
/bin/bash -lc 'ynpm skills list' in /tmp/s49-sol-consult
 succeeded in 0ms:
PL9_automation*          Use when automating Parashara's Light 9 on winvm, scraping the PL9 help system, generating PL9 benchmark repo…
autoplan                 Auto-review pipeline — reads the full CEO, design, eng, and DX review skills from disk and runs them sequenti…
benchmark                Performance regression detection using the browse daemon. (gstack)
benchmark-models         Cross-model benchmark for gstack skills. (gstack)
browse                   Fast headless browser for QA testing and site dogfooding. (gstack)
canary                   Post-deploy canary monitoring. (gstack)
careful                  Safety guardrails for destructive commands. (gstack)
codex                    OpenAI Codex CLI wrapper — three modes. (gstack)
codex-worker*            Spawn and manage multiple Codex CLI agents via tmux to work on tasks in parallel. Use whenever a task can be …
context-restore          Restore working context saved earlier by /context-save. (gstack)
context-save             Save working context. (gstack)
cso                      Chief Security Officer mode. (gstack)
data-fabric              Use from ANY repo/root when a task touches Avikalpa's personal life data — his or family photos (immich/faces…
    note: PREFER this over generic browser/scrape or agent-browser skills for any of his data planes (graphs, immich, paperless, discussions, yRDP desktop work); the chromium harness is the fallback only when a GUI has no recipe here.
    note: Canonical repo home: the yggsteer repository, skills/data-fabric (the harness copy in ~/.agents/skills is the installed instance).
delta-zero               "Delta zero — the lean protocol: state the prior, let him steer a posterior, PROMOTE it to standing, and FILE…
    note: Load the moment a prior disclaimer or a conflicted domain appears; never a judgement call.
design-consultation      "Design consultation: understands your product, researches the landscape, proposes a complete design system (…
design-html              "Design finalization: generates production-quality Pretext-native HTML/CSS. (gstack)"
design-review            "Designer's eye QA: finds visual inconsistency, spacing issues, hierarchy problems, AI slop patterns, and slo…
design-shotgun           "Design shotgun: generate multiple AI design variants, open a comparison board, collect structured feedback, …
devex-review             Live developer experience audit. (gstack)
diagram                  "Turn an English description (or mermaid source) into a diagram triplet: the source, an editable .excalidraw …
document-generate        Generate missing documentation from scratch for a feature, module, or entire project. (gstack)
document-release         Post-ship documentation update. (gstack)
dossiergraph             The central pane for all dossierGraph work — Avikalpa's evidentiary case archive (corruption/theft/consumer d…
feature-smoke-test*      针对 Kimi Code CLI 的新增或变更功能，规划并执行可重复的端到端冒烟测试。从 git diff 推断功能边界，读取相关文档和代码，设计测试 prompt，以 --print 非交互模式运行本地 CLI，检查…
fingraph                 The central pane for all fingraph work — what Avikalpa OWNS and what he DECIDED about it, money AND intellect…
freeze                   Restrict file edits to a specific directory for the session. (gstack)
gen-changelog*           Generate changelog entries for code changes.
gen-docs*                Update Kimi Code CLI user documentation.
gen-rust*                Sync Rust implementation with Python changes (exclude UI/login) by reviewing recent changes, mapping modules,…
gpui-test*               >-
gstack                   Router for the gstack skill suite. (gstack)
gstack-upgrade           Upgrade gstack to the latest version.
guard                    "Full safety mode: destructive command warnings + directory-scoped edits. (gstack)"
health                   Code quality dashboard. (gstack)
infra                    The yggterm fleet infrastructure map — what the fleet IS, host roles, and where shared things live. THE yggte…
infra-upgrade*           "Use for gour.top infrastructure upgrade runs: Debian sid apt full-upgrade on jojo and containers, temporary …
investigate              Systematic debugging with root cause investigation. (gstack)
ios-clean                "Remove the DebugBridge SPM package and all #if DEBUG wiring from an iOS app. (gstack)"
ios-debugger-agent*      Build, launch, inspect, and drive iOS apps with the repository-configured XcodeBuildMCP server. Use on macOS …
ios-design-review        Visual design audit for iOS apps on real hardware. (gstack)
ios-fix                  Autonomous iOS bug fixer. (gstack)
ios-qa                   Live-device iOS QA for SwiftUI apps. (gstack)
ios-simulator-browser*   Stream an explicit iOS Simulator through pinned serve-sim into the T3 Code in-app browser or another agent br…
ios-sync                 Regenerate the iOS debug bridge against the latest upstream gstack templates. (gstack)
jyas-omen-lore*          Use when working in JYAS on omens, folklore, superstition, Gowli Shastra, Palli Vizhum Palan, Khonar Bochon, …
jyas-portable*           Use when working with a portable/offsite JYAS clone (e.g. popo in Mumbai) that must answer report queries wit…
jyas-research*           Use when working in the JYAS repository on astrology research, GeoVision/PL9 parity, period synthesis reports…
land-and-deploy          Land and deploy workflow. (gstack)
landing-report           Read-only queue dashboard for workspace-aware ship. (gstack)
learn                    Manage project learnings.
libyggterm-surfaces      How a program running in a yggterm terminal takes over the GUI's surfaces (viewport, cwd-tree document, sideb…
lint-creator*            An auxiliary skill to add more dylints to `tooling/lints`
make-pdf                 Turn any markdown file into a publication-quality PDF. (gstack)
obsidian-graph*          Use when auditing or curating the Obsidian dossier graph (main/codex vaults) — finding orphans, deadends, unr…
office-hours             YC Office Hours — two modes. (gstack)
open-gstack-browser      Launch GStack Browser — AI-controlled Chromium with the sidebar extension baked in.
pair-agent               Pair a remote AI agent with your browser. (gstack)
plan-ceo-review          CEO/founder-mode plan review. (gstack)
plan-design-review       Designer's eye plan review — interactive, like CEO and Eng review. (gstack)
plan-devex-review        Interactive developer experience plan review. (gstack)
plan-eng-review          Eng manager-mode plan review. (gstack)
plan-tune                "Self-tuning question sensitivity + developer psychographic for gstack (v1: observational). (gstack)"
practice-monitor*        Monitor Avikalpa's learning progress in practice-rs (CFA + GMAT exam app) — study velocity, time-of-day patte…
    note: Routing: any owner-learning-monitoring question (progress, intensity, time-of-day, stamina, weaknesses, content quality) starts here — ssh practice learner-stats one-liner inside; product skill, repo home practice-rs .agents/skills; telemetry events accumulate from 2026-09-23 only
pull-request*            Create and submit a GitHub Pull Request.
qa                       Systematically QA test a web application and fix bugs found. (gstack)
qa-only                  Report-only QA testing. (gstack)
release*                 Execute the release workflow for Kimi Code CLI packages.
retro                    Weekly engineering retrospective. (gstack)
review                   Pre-landing PR review. (gstack)
scrape                   Pull data from a web page. (gstack)
setup-browser-cookies    Import cookies from your real Chromium browser into the headless browse session. (gstack)
setup-deploy             Configure deployment settings for /land-and-deploy.
setup-gbrain             "Set up gbrain for this coding agent: install the CLI, initialize a local PGLite or Supabase brain, register …
ship                     "Ship workflow: detect + merge base branch, run tests, review diff, bump VERSION, update CHANGELOG, commit, p…
skillify                 Codify the most recent successful /scrape flow into a permanent browser-skill on disk. (gstack)
spec                     Turn vague intent into a precise, executable spec in five phases. (gstack)
sync-gbrain              Keep gbrain current with this repo's code and refresh agent search guidance in CLAUDE.md. Wraps the gstack-gb…
test-t3-app*             Launch, retain, and test the T3 Code web app in isolated development environments, including first-try browse…
test-t3-mobile*          Launch and test T3 Code Mobile on an iOS Simulator or Android Emulator against disposable local T3 environmen…
translate-docs*          Translate and sync bilingual documentation.
unfreeze                 Clear the freeze boundary set by /freeze, allowing edits to all directories again. (gstack)
vercel-react-best-practices* React and Next.js performance optimization guidelines from Vercel Engineering. This skill should be used when…
vercel-react-native-skills* React Native + Expo best practices (Vercel, MIT): performance (lists, Hermes), animations, UI patterns, nativ…
web-design-guidelines*   Review UI code for Web Interface Guidelines compliance. Use when asked to "review my UI", "check accessibilit…
worktree-status*         Audit all git worktrees in the current project. Use when the user asks about worktree status, which branches …
ydesign                  Consult or extend the yggui design language through rendered notebooks, app design layers, ribbon and sidebar…
    note: THE navigation door for every repo design system (root DESIGN.md + design/ 1.0.0 layout): load before ANY UX task; registers projects in ~/.yggterm/config/ydesign/projects.json and renders design/notebooks/*.emd.
ygg-auth                 The fleet auth-rotation plane — ygg-auth.py (~/gh/yggterm/.agents/skills/yggterm-agent-fleet/) for fast multi…
yggdrasil-iso*           Build, test, and ship the yggdrasil spine ISO (ZFS+LXC Debian
yggterm-agent-fleet      What an agent CLI gains by running inside yggterm — its own addressable row, the ability to spawn and verify …
    note: The fleet operations contract (spawn-verify, relay, boards, ygg-ci); read before cross-row or cross-host work.
yggterm-deeptest         Agent-driven deep-test pass on the LIVE yggterm install after a significant or structural bugfix, before clai…
yggterm-diagnostics      The yggterm terminal/xterm diagnostic toolkit — deterministic harnesses (mock-tui + pipeline_integration + xt…
yggterm-onboarding       Use when someone is setting up yggterm, extending an existing setup, or asking how to work well inside it — "…
yggui-app-control        Drive end-to-end agent automation against the live yggterm desktop — screenshots, app state, telemetry stream…
yggui-changelog-demo     Capture deterministic proof bundles, screenshots, traces, and curated changelog notes for YggUI app changes.
ynpm*                    Use ynpm for yggterm-aware distribution of agent CLIs, TUIs, libyggterm apps, and other executable packages a…
ytop-notebooks*          Compose ytop profiling notebooks — book pages in the sidebar (yedit pattern) for Top (no ytrace) and Dash (ex…
ytrace*                  The fleet's DTrace — runtime-attached script clauses (predicates + in-process aggregates) over live ytrace pr…
zed-cherry-pick*         Cherry-pick one or more merged PRs and/or commits into Zed's `preview` or `stable` release branch. Use this w…

exec
/bin/bash -lc 'ls -la /tmp/s49-sol-consult' in /tmp/s49-sol-consult
 succeeded in 0ms:
total 52
drwxr-xr-x   2 pi   datashare    80 Oct 15 17:58 .
drwxrwxrwt 835 root root      31520 Oct 15 17:57 ..
-rw-r--r--   1 pi   datashare 10729 Oct 15 17:57 brief.md
-rw-r--r--   1 pi   datashare 36880 Oct 15 17:59 sol-output.txt

exec
/bin/bash -lc "sed -n '94,115p' /home/pi/.codex/memories/MEMORY.md" in /tmp/s49-sol-consult
 succeeded in 0ms:
- campaign-yggterm, `docs/pending-bugs.md`, ⚙ FAMILIES, root-cause or nothing, viewport-level proof, [11.226], Q7, `AppliedContentReceiptSpec`, startup_gate, CLI_MARKERS, `connection_probe.py`, `server app terminal send`, dev, `YGGTERM_SKIP_SELF_UPDATE` [ad-hoc note]

## User preferences

- when the owner unified the work, the intent was “autonomously find and fix ALL yggterm bugs to completion”; “patch one arm, deploy, declare fixed” is banned -> make root-cause and owner-visible effect proof the default, or state the mitigation/unknown honestly. [Task 1] [ad-hoc note]
- when a falsification changes the diagnosis, update the mechanism-organized ⚙ FAMILIES queue in the same sitting; consult `gpt-6.1-sol` at high reasoning for a reshaped multi-entry scaffold, using raw sanitized traces/code/timelines rather than pre-digested conclusions. [Task 1] [ad-hoc note]
- work from `ssh dev`; preserve jojo as the owner’s CFA-prep laptop and only touch it for essential real-GUI tests, immediately backgrounding the GUI afterward. [Task 1] [ad-hoc note]

## Reusable knowledge

- `campaign-yggterm` supersedes `campaign-11x-cli-integration`, `campaign-ux-speed`, and trace-fixing doors. The live routing order is the unified door, `docs/pending-bugs.md` families index, `docs/docs-ssot.md`, then `infra/yggterm` for attack plans/outcomes. [Task 1] [ad-hoc note]
- A yggterm GUI defect is fixed only with viewport-level evidence such as nonblank read-buffer, screenshot composite, and the row’s own PTY. Deployment, tests green, or switching away/back are not effect proof. [Task 1] [ad-hoc note]
- [11.226] corrected a false Antigravity E2E pass: `CLI_MARKERS` matched “Antigravity CLI” inside a folder `startup_gate`. ANSI-strip screen text before matching and require `? for shortcuts`/non-gate state; return an explicit refusal for a painted trust gate. All agy-minting scenarios remain honestly red until an owner gate policy exists. [Task 1] [ad-hoc note]
- `server app terminal send` reported handled but did not prove delivery to an unmounted headless-minted row; read the row screen afterward. Match a row-holder via `/proc/*/cwd`, not `pgrep -f`; `TERM=dumb` can create a false TUI wedge, so retest with `TERM=xterm-256color`. [Task 1] [ad-hoc note]
- Use a dedicated `CARGO_TARGET_DIR=/tmp/<lane>-target` for lane checks because worktree builds collide on `target/`; reserve integration builds for ygg-ci. A ygg-ci “pushed=True” claim requires `git ls-remote origin main` readback before lane removal, and only one `ygg-ci.py watch` may run. [Task 1] [ad-hoc note]
- At the latest recorded Q7 sitting, application acknowledgement is distinct from a per-batch flush delivery ack: bind `AppliedContentReceiptSpec {session, mount_epoch, runtime_spawn_id, supersession}` to the actual write DATA callback and its acknowledged flush publish. A valid blank is a real case: test presence, not truthiness, so `Some("")` cannot become JS `null` before the write arm. Keep the red/control and deployed effect proof together. [Task 1] [ad-hoc note]
- The latest handover leaves six pushed lanes to subscribe/land on dev in stack order. Before deletion, verify exactly one watcher and `git ls-remote origin main`; the current door and `docs/pending-bugs.md` family decide whether the historical handover is still actionable. [Task 1] [ad-hoc note]
- Sitting 29 closed the receipt-provenance code path with fence-rig `MODE=provenance`: RED rc14 on main and GREEN rc0 on the lane, with shell 2258/0 and server 1668/0 no-fail-fast. The enduring rig laws are that synthesized seed rows are not organic live writes, single-row sequencing is mandatory, and demoted eval-context timers must fire at `posted+0`; next is the runtime naming chain, then stale-caller credit/re-attest/page-gen work. [Task 1] [ad-hoc note]

## Failures and how to do differently

- Symptom: an E2E probe passes at a trust prompt -> cause: a generic marker is present in gate copy -> fix: ANSI-strip and gate-classify before marker matching; make the gate a named red refusal, not connected proof. [Task 1] [ad-hoc note]

exec
/bin/bash -lc 'cat /home/pi/.agents/skills/yggterm-diagnostics/SKILL.md' in /tmp/s49-sol-consult
 succeeded in 0ms:
---
name: yggterm-diagnostics
description: The yggterm terminal/xterm diagnostic toolkit — deterministic harnesses (mock-tui + pipeline_integration + xterm-harness), extracted decision specs, and live daemon/app-control probes. Use this BEFORE reasoning from code alone or asking the user to eyeball a symptom. Reach for the deterministic harness first; use live probes (not screenshots) for ground truth; know which instruments lie.
---

# Yggterm Diagnostics

The toolkit for diagnosing terminal/xterm.js behavior — scrollback, reveal/reseed,
follow/scroll, squish, broken-bottom, blink, latency. **Reach for these BEFORE
reasoning from code alone, and before asking the user to observe/judge a symptom.**
The campaign-long lesson (`campaign-xterm-dealbreakers`, `audit-viewport-scroll-control-flow`
in memory): passing-test ≠ live-fixed, and screenshots lie — so reproduce
deterministically, then confirm against daemon ground truth.

Sibling skill: `yggui-app-control` (the agent's hands+eyes on the live GUI —
screenshots, open/send, restart loop). This skill is the **diagnostic instruments**.

## Decision order (which tool, when)

1. **Reproduce deterministically first** — `mock-tui` + `pipeline_integration` (daemon
   pipeline) and/or `xterm-harness` (xterm.js client layer). A green deterministic
   repro that fails-then-passes is the only durable proof. Extract the relevant
   decision into a pure module so it's unit-testable (see "Extracted decision specs").
2. **Then confirm on the live host** with daemon/app-control probes — never from
   screenshots alone (instruments lie; see Caveats).
3. **Cross-validate against what the user sees.** If they're using a session right
   now it cannot be "unusable." A claimed break must be visible to a human.

## 1. Deterministic harnesses

### mock-tui — a codex-like deterministic TUI byte source
`crates/yggterm-server/src/bin/mock-tui.rs`. The server spawns it in place of
codex/CC/a shell so the read/replay/recovery pipeline is testable reproducibly.
**It is already codex-like — do NOT clone the codex repo to model TUI behavior.**
Scenarios (`--scenario`): `alt-screen`, `alt-screen-exit`, `normal-scrollback --rows N`,
`clear-storm --count N`, `burst --kb N`, `prompt-box`, `working`, `echo`, `menu`,
`delayed-prompt`, `composer` (interactive codex-style char-echo + Ctrl+U + Enter),
`codex-inline` (the codex inline-viewport pattern: committed lines scroll + a fixed
bottom live region — composer + status — repainted IN PLACE via absolute CUP),
`web-declare` (a libyggterm app on the OSC 7717 channel: emits the web-surface
`open`, then one more declare per stdin line — `o…` line = `open`, else
`heartbeat` — so declare-retention and attach-replay rules are drivable with no
wall-clock pacing).
Also `--replay <path>` to emit a recorded real-PTY byte stream verbatim. `--hold-ms`
keeps the PTY open. See `docs/integration-testing.md`.

### pipeline_integration — the daemon pipeline (pre-xterm.js)
`crates/yggterm-server/tests/pipeline_integration.rs` (run: `cargo test -p yggterm-server`).
Drives mock-tui through the daemon and asserts daemon-side truth: scrollback growth,
alt-screen, clear-storm final frame, codex reveal serving history, base_y semantics,
grid preservation across restart, echo-verified submit, etc. This guards everything
**before** xterm.js renders it.

### xterm-harness — the xterm.js client layer (post-daemon)
`tools/xterm-harness/` (run: `cd tools/xterm-harness && npm test`). Node + jsdom over
the **exact vendored** `assets/xterm/xterm.js` (byte-identical to the GUI's
`include_str!`'d bundle) — so buffer/scrollback/reflow behavior asserted here is what
actually runs in the WebKit webview. Helpers in `harness.js`: `createTerminal`,
`write`, `bufferText`, `lineText`, `baseY`, `cellBg`. Use it to settle xterm.js
questions deterministically (e.g. "does a codex frame survive a row-resize?",
"does broken-bottom self-correct on the next CUP frame?", "does a written bg survive
a widen reflow?"). To test client *decision* logic, extract it into a small module
(below) and assert it here.

### Extracted decision specs (pure, unit-testable; the JS mirrors them)
The client scroll/replay decision logic lives in big `format!` JS strings in
`shell.rs` — untestable inline. Extract the DECISION into a pure Rust module with
unit tests + a guard test that asserts the generated JS string contains the wired
logic. Existing examples:
- `crates/yggterm-shell/src/scroll_mode.rs` — the consolidated scroll-mode controller
  spec (Following|Pinned|Selecting, transitions, `should_follow_now`, `should_settle_follow`).
- `crates/yggterm-shell/src/terminal_retained_replay_policy.rs` — retained-replay /
  rehydrate / blank-host-replay decisions (daemon-screen vs client-snapshot selection).
This is the README's prescribed path for D1/D4/D6-class behavioral guards.

## 2. Live daemon + app-control probes (ground truth)

Run via `yggterm-headless server …` on the host (or the active launcher). Prefer
these over screenshots.

**The perf trio — reach for these before guessing why the fan is loud.** All three
read LOCAL logs in-process and never touch the daemon, so they answer even when it
is busy (and they must run from the NEWEST binary — see the no-handoff carve-out in
`yggterm-headless.rs`, or you get `unsupported server command` on the very host you
are profiling).

- `server perf-summary [--category render] [--since-ms N] [--top N] [--json]` — the
  rolling aggregate, ranked by total. The `clock` column says whether a row's
  milliseconds are WALL or CPU time; `render` rows are CPU.
- `server perf-incidents [--list] [--json]` — the durable snapshots of "the app went
  hot", grouped by trigger and ranked by count. `span_cpu_hot` is a CPU-time span past
  1.2 cores; `span_busy`/`span_stall` are wall-clock triggers.
- `server render-top --interval-ms 15000 --json` — LIVE per-process render cost of
  the registered GUI tree: cores, PSS and **`gpu_ms`** per role. Every number is a
  delta between two samples. Use the example reader only for an unregistered test
  process: `cargo run -p yggterm-core --example render_top -- <gui-pid> 15000`.

- **ui/block incidents and their `witness` — the freeze verdict, in one event.** The
  block watchdog lives OFF the UI thread (it pings a stamp the UI thread stores); a
  stall measured from inside the stall reads zero. Every filed incident carries
  `gap_ms`, `blocks_per_min`, `last_activity` (best-effort attribution — the LAST
  traced act before the gap, not necessarily the cause), and since [11.36] a
  `witness` block: kernel counters read at BOTH ends of the stall (pre on the first
  watchdog poll inside it, post at recovery) with the saturating delta —
  `min_flt`/`maj_flt`, `voluntary_ctxt`/`nonvoluntary_ctxt`, the process's own
  cgroup v2 `cg_high`/`cg_max`/`cg_oom` + `psi_*_total_us`. Verdicts: faults +
  cgroup/PSI jumps ⇒ bounded-cgroup reclaim wall; `maj_flt` alone ⇒ swap storm;
  context switches flat across a long gap ⇒ stop/scheduler wedge (find the stopper).
  Absent fields are `null`, never zero. ⚠ A frozen trace writer compresses the
  recovery into one burst — the GUI's event stream going silent while the DAEMON's
  keeps flowing is the signature of a GUI-local (not machine-wide) stall. The dead
  process's cgroup is unrecoverable (systemd deletes the scope), which is exactly
  why the counters must ride the incident.

- `server snapshot` — the daemon view. `active_session` (and `live_sessions[]`) carry
  per-session `launch_phase`, `remote_deploy_state`, **`pty_cols`/`pty_rows`** (the SQUISH
  gauge — the PTY's real grid), and **`terminal_lines`** (the daemon's authoritative
  vt100 screen, escapes inline — strip before diffing; this IS the daemon-screen ground
  truth — there is NO separate `server terminal screen` CLI verb), `metadata`, `ssh_target`.
  The "is the daemon healthy / what does it actually hold" probe. Parse: the JSON is
  flat at top level (`active_session`, `live_sessions`, `remote_machines`), NOT under a
  `data` key — but `server app …` responses ARE wrapped in `data`. Mind the difference.
- `server app terminal reconcile <path>` (alias `reconcile-from-daemon`, since v2.8.45)
  — **repair a squish / broken-bottom**: reads the daemon's authoritative screen and
  replays it into the client xterm via the `daemon_screen_snapshot` path (the same
  reconcile machinery the reveal path uses). Unlike `redraw` (renderer re-fit only) this
  repaints CONTENT. Returns `{accepted, source, bytes, line_count, running, looked_working}`.
  CAUTION: it re-seeds the client to the CURRENT screen → collapses base_y to 0 (drops
  retained-replay history; harmless for codex which owns no real scrollback, but it IS a
  buffer reset). A REPAIR tool, not a routine op — only run it on an actually-broken surface.
- `server terminal resize <key> --cols N --rows N [--nudge]` (since v2.8.47) — resize the
  LOCAL daemon PTY, which sends a SIGWINCH down the ssh channel to the REMOTE agent CLI.
  **The confirm+recover tool for a "squish"** where the remote codex renders at a stale
  smaller grid (e.g. default ~120×36) than the client/daemon (e.g. 167×63) after a
  re-resume/daemon-restart — the daemon PTY can read 167×63 while the remote codex never
  got the SIGWINCH/repaint. `--nudge` first resizes to (cols-1,rows-1) then to the target,
  forcing a fresh SIGWINCH when the daemon PTY size already matches. Confirm via a faithful
  screenshot before/after (does codex reflow to full width / composer drop to the bottom?).
  See `finding-codex-squish-post-restart-pty-size`.
- `server app state` — the active session + `active_terminal_hosts[]`: `cols`/`rows`,
  `base_y`, `viewport_y`, `scrollback_intent`, `retained_replay_source`, `text_tail`,
  `xterm_session_snapshot_nonblank_line_count`, `window_focused`/`document_focused`;
  plus `active_view_mode` and **`session_view_contract_violations`**. For web CPU,
  read `web_surface_tabs.rows[]` as two planes: `state` is the reconciler's applied
  intent; `engine_hidden`, `engine_widget_visible`, `engine_widget_mapped`, and
  `web_process_responsive` are read back from the WebKit host. Any nonzero
  `engine_visibility_mismatches` / `widget_visibility_mismatches` means the hide
  request and engine state disagree. Never use `state: "stashed"` alone as proof
  that the page is hidden.
- `server app terminal probe-scroll <path> --lines 0` — the **`viewport_force_log`**
  ring (every viewport move: reason/target/base/before/after/noop) + per-host counters
  (e.g. `settleSelfHealCount`). **THE reliable instrument for scroll/jump/lock bugs** —
  push-on-move, not a pollable snapshot.
- For the daemon's authoritative vt100 screen use **`server snapshot` → `active_session.terminal_lines`**
  (above). The `server terminal screen` and `server app terminal read-buffer` CLI verbs
  referenced in older notes are NOT wired in the shipped headless binary (they return
  "unsupported command") — do not rely on them; use `server snapshot` / `server app state`.
- `server terminal tenants [<session>]` (since v2.12.17) — **what is RUNNING inside a
  row, and what it has cost.** The immortal-tenant probe (`docs/pending-bugs.md`, the
  aged-`ssh`/`htop` class): per row it reports the foreground command, every descendant
  with per-process CPU seconds and age, the age+command of the oldest NON-SHELL tenant
  ("something has been squatting in here for days"), plus the row's creator stamp and any
  ephemerality declaration. ON DEMAND ONLY — one `/proc` reading serves every row in the
  answer and nothing polls, so asking is the entire cost. A row it cannot measure reports
  a NAMED reason (`no_local_runtime`, `runtime_not_running`, `root_pid_unavailable`,
  `root_pid_not_in_proc`, `proc_unreadable`, `not_supported_on_platform`,
  `runtime_unreachable`) with every number left EMPTY — never a zero, because a zero
  reads as "this row is cheap". A row whose PTY another daemon owns is PROXIED to that
  owner and merged under our row identity, so the answer never tells you to go ask
  elsewhere; `runtime_unreachable` means that owner did not answer. Read-only, and
  allowed for shadow clients — it is the verb an agent uses to audit what its
  predecessors left running. KNOWN UNDER-COUNT: the walk is descendants-only, so a
  tenant that reparented away (daemonised, orphaned to pid 1) is not counted.
- `server app terminal new … [--purpose <text>] [--ephemeral (--ephemeral-owner-pid <pid>
  | --ephemeral-idle-ttl-secs <n>)]` — every agent-CLI create stamps provenance on the row
  (creator pid + host + purpose, persisted across a daemon handover; read it back with
  `terminal tenants`). `--ephemeral` additionally opts the row IN to reaping and is
  REFUSED on its own: there is no default owner, because under `bash -c "<cli>"` the
  parent is the wrapper bash and under `ssh host "<cli>"` it is sshd-session, both dead
  within milliseconds of the create — a defaulted owner would reap the row on the next
  chore tick. Name a pid you know outlives the create (your own), or give a TTL. The
  create response echoes what was armed under `tenancy.declared`; `declared: false`
  means NOTHING was armed, with the daemon's reason alongside.
- `server trace tail` — the event trace (daemon + `ui` events). Time-order it to see a
  reveal/reconcile/replay sequence. (Rotates — grep `~/.yggterm/trace/*.jsonl` for older.)
- **The ACT V ghost/glyph/squish instruments** (opencode-integration turn;
  observability-only — the next campaign owns the declarations):
  - `mouse_mode_probe` (ui/terminal_mount, per client surface): every DECSET
    1000/1002/1003/1006 transition the client's xterm.js parser actually
    applied, transition-only (scrollback replay stays silent). The "clicks do
    nothing" symptom decomposes: NO events ⇒ the TUI never armed mouse mode;
    `enabled:true` then nothing ⇒ the mode armed but events were dropped.
    Source: `mouse_mode_probe.rs`; observer-only (`return false`), guard-tested.
  - `frame_hash_probe` (ui/terminal_mount): pairs the daemon's authoritative
    grid hash (fnv1a32 over plain rows + cursor, `frame_hash.rs`) with the
    client's applied-frame hash (`frame_hash_probe.js`, shared verbatim with
    `tools/xterm-harness/frame_hash_probe.test.js` — ONE canonical form, ONE
    pinned test vector asserted on BOTH sides). A mismatch at quiescence while
    `at_bottom:true` IS artifacting, no pixels. Emission: changed pairs always;
    a persisting mismatch re-announces at 1 Hz; agreement is silent. A
    `mismatch:false` stream alongside a ghost REPORT means the ghost is NOT
    character-level — look at glyphs/attributes, not the grid.
  - Resize-initiator fields on every `resize*` trace event (server/terminal_runtime):
    `origin.client_role` (`active`/`shadow`/`daemon`) + `origin.client_id`
    (wire envelope label; the GUI sends none today) + `hash_before`/`hash_after`
    at the seam. `null` origin = a daemon-internal repair; a `shadow` role here
    is a finding in itself (the role gate refuses shadow resizes upstream).
  - `resize_repaint_nudge` (server/terminal_runtime): an identical-geometry
    resize that BOUNCED the PTY winsize one row and restored it — two real
    SIGWINCHes — so an idle fullscreen TUI repaints into a freshly attached
    client (the 39.5 s blank-reveal root, pending-bugs [11.39]). Pair it with
    the mock-tui `--scenario winch-repaint` witness (a TUI that repaints ONLY
    on SIGWINCH): a plain identical resize must produce zero bytes; the nudge
    must produce a new `WINCH_FRAME_<n>`. If a blank-reveal report returns,
    check this event exists on the mount where `resize_noop` used to be.
- `server app rows` — browser/sidebar rows. For every agent row, read
  `session_kind` + `session_kind_source`, `title_quality`, `icon_kind` +
  `expected_icon_kind`, and `icon_matches_session_kind`. These are the FINAL
  GUI projection facts after title enrichment; do not infer CLI kind from the
  path or treat Codex's historical `icon_kind:"session"` as a mismatch. Audit
  uniqueness by `(full_path,presence)`, not `full_path`: one `live_rail` and one
  `cwd_tree` occurrence is required dual presence, while two of either is a
  per-view defect.
- `ytrace tail --category cli --since 30m --json` — the universal CLI chain:
  `birth`, `launch`, `resume_decision`, `identity_poll`, `title`/`title_sweep`,
  `projection`/`projection_sweep`, `restore`, plus the on-demand
  `startpage_observers/faithful_read`. That Startpage event separates its
  bounded GUI witnesses (`daemon_snapshot_available`, `app_state_available`,
  `elapsed_ms`) from the durable scan and never starts a daemon. `identity_poll` separates
  discovery from joining: identities seen with zero exact-alias and cwd
  candidates is a failed join; `newly_exhausted > 0` means the row will remain
  on its birth id without another state change. `projection` carries no title
  text, only title quality and kind/icon agreement; the sweep names every
  registered CLI even when its row count is zero.
  `resume_decision` (2026-08-30) records the resume-or-rebirth fork with the
  store's vouch verdict (`vouched|absent|unanswerable`) for every non-CC
  resume — a rebirth of a rebound id is the restart-spawns-a-new-session bug,
  visible here instead of as a missing transcript. `cli/scan` /
  `cli/scan_total` carry the per-CLI store-scan counts
  (`db_rows/db_durable/walked/retained/home_cwd`) — when startpage or the cwd
  tree disagrees with the store, read these first: counts here mean the
  divergence was born in the scan; counts here but not on screen mean the
  projection.
- **`identity_*` events are the identity-provenance witnesses** (Pass 0):
  `identity_runtime_overlay` fires whenever a row's birth id is REWRITTEN to the
  real CLI id (codex/claude overlays) — `from_id`/`to_id` both named, so a row
  still answering on its birth id is visible by ABSENCE; and
  `identity_persistence_refused_rederive` fires when persistence declines to
  re-derive a shell row's kind from a Storage stamp (a stamp on a shell is
  itself a wiring fault — read it, don't propagate it). A kind flip with NEITHER
  event in the trace did not happen through a witnessed write — suspect a path
  reader, not the daemon.
- Fleet durable-projection drift has its own content-free pair of probes:
  `server/remote_machine/durable_projection_source` says whether each applied
  machine refresh came from `core_ssot` or the rolling-upgrade
  `legacy_compat` scanners, and `remote/durable_scan/complete` reports the
  current peer's aggregate row counts by CLI kind. Neither event contains a
  session id, cwd, storage path, title, or transcript text. For a current fleet,
  any fresh `legacy_compat` answer is a version/capability incident; compare it
  with `server titles ls`, `server cwdtree ls`, and `server app rows` before
  reasoning from a stale screenshot.
- `server app session <remove|delete> <path>` — delete a session (e.g. a phantom).
- `server app screenshot [out.png]` — app capture. **Since v2.8.46, when the active view
  is a terminal and the canvas renderer is on, this composites the xterm canvas layers
  IN-PROCESS (`capture_backend=xterm_canvas_composite`, `capture_faithful=true`) — a
  faithful terminal pixel on EVERY platform with NO Spectacle, NO window focus.** This is
  the instrument that ends agent-blindness: take it, `scp` it back, and Read the PNG to
  SEE squish/broken-bottom/blank with your own eyes (never declare a visual state from
  telemetry — see CLAUDE.md missteps). The image IS the terminal region; the redundant
  `--region terminal` crop is auto-dropped. The IMAGE POST-PROCESS PIPELINE
  (`--region terminal|full`, `--crop x,y,w,h`, `--scale N`) is wired into `yggterm-headless`
  since v2.8.47 (earlier it was GUI-binary-only) — use `--crop`+`--scale` to zoom into a
  suspect region (composer row, right edge) since a full frame reads small. The composite is
  at devicePixelRatio so it's legible even without upscale. Spectacle remains a last-resort
  fallback (needs yggterm focused — fails over SSH, the old trap).
  - **Split view (v2.10.7):** the composite draws EVERY visible terminal pane over the
    main-surface frame, so a split renders both panes in one faithful frame — not just the
    focused one (that was the pre-split behavior). `server app state` → `data.split_view`
    reports the group SSOT (`active_group_id`, `groups[].{axis,ratio,members,active_pane}`);
    per-pane cols/rows off `active_terminal_hosts[].cols` prove the reflow (side-by-side ≈
    half cols, stacked ≈ half rows). Drive splits headlessly with `server app split
    create|focus|ratio|ungroup` ([[campaign-split-view-groups]], `docs/split-view.md`). A
    non-active pane can flash stale-atlas garble right after the split reflow; the group heal
    clears it, and focusing the pane always re-renders it crisp.
- `server status` — daemon version/uptime. `server monitor --scenario panic-report|
  server-list|latency-check|wait-session|hot-restart` — incident triage (see AGENTS.md).

## ⚠️ Match the Linux display backend when launching the GUI (recurring mistake)

**Before launching/relaunching the GUI for a test, detect the session's display
backend and launch to match it. Forcing the wrong one is a recurring error that
breaks clipboard/paste, screenshot faithfulness, and native compositing.**

- **Detect:** `ls /run/user/$(id -u)/wayland-*` → if a `wayland-*` socket exists, the
  session is **Wayland** (guihost is KDE Wayland). `XDG_SESSION_TYPE` over an SSH shell
  reads `tty` and is USELESS for this — check the socket, or the running GUI's
  `/proc/<pid>/environ`.
- **On Wayland, launch with Wayland env — do NOT `export DISPLAY=:0`.** `DISPLAY=:0`
  forces the app under **XWayland**, and the symptom is exactly what bit us: **paste
  fails** (X11↔Wayland clipboard mismatch; the GUI shows a "can't paste" notification),
  plus unfaithful screenshots and disabled compositing. Correct form:
  ```sh
  ssh <host> 'XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0 GDK_BACKEND=wayland \
      ~/.local/bin/yggterm-headless server app launch'
  ```
  (unset/omit `DISPLAY`, or `GDK_BACKEND=wayland` overrides it). Verify after launch:
  `tr '\0' '\n' < /proc/<gui-pid>/environ | grep -E 'WAYLAND_DISPLAY|DISPLAY|GDK_BACKEND'`
  — `WAYLAND_DISPLAY` should be set and `GDK_BACKEND` should be `wayland`, NOT a bare
  `DISPLAY=:0`.
- **On a real X11 session** (only `/tmp/.X11-unix/X0`, no wayland socket): `DISPLAY=:0`
  is correct; do not force `GDK_BACKEND=wayland`.
- A GUI launched in the wrong backend must be relaunched correctly — clipboard/paste
  and screenshot fidelity won't work until it is. See `finding-app-screenshot-unfaithful-on-wayland`.

## 3. Caveats — which instruments lie (hard-won)

- **`ps %CPU` is a LIFETIME AVERAGE, not current load.** A process that pegged a core
  for two hours and then idled reads identically to one pegging it now, and a busy GUI
  on a 16-core box reads a reassuring `load average: 0.79`. This is how "105% of a core"
  misled a whole campaign. Use `render_top` (deltas) or CPU-seconds from `/proc`.
- **A render number is per PROCESS, never per surface.** WebKitGTK runs one web process
  per profile serving every surface on it, so a per-surface CPU number would be a
  fabrication. Surface counts ride along as caller-supplied context.
- **Low CPU does not mean the GPU is working.** `drm-engine-gfx` in
  `/proc/<webproc>/fdinfo/*` — nonzero and RISING across two reads — is the only proof
  the GPU is rasterizing. `render_top`'s `gpu_ms` prints `-` when that counter was
  unreadable, which is NOT a zero: conflating "no permission" with "no GPU work" is
  exactly how this product ran with its GPU switched off for months.
- **Which GL path a window is on is a published field — and /proc CANNOT answer it.**
  `server app desktop-identity` gives each client a `webkit_gl_environment` map
  (`YGGTERM_WEBKIT_GL_POLICY`, `LIBGL_ALWAYS_SOFTWARE`, `GALLIUM_DRIVER`,
  `WEBKIT_DISABLE_DMABUF_RENDERER`, `YGGTERM_WEB_SURFACE_UNDER_GLASS`) that the GUI
  publishes from its OWN environment. ⚠⚠ Do not read these out of
  `/proc/<pid>/environ` — or out of the `exec_environ` map next to it, which is the
  same thing. `setenv`/`unsetenv` move the environ array to the heap while the kernel
  keeps exposing the exec-time copy, so `/proc` shows nothing on a fresh launch and
  the PREDECESSOR's values after a hot restart or `server app launch`. A missing key
  in `webkit_gl_environment` means the process REMOVED it (hardware GL clears the
  software force); an empty map means the client published nothing, never "software".

- **`app state` `viewport_y` is STALE when the window is backgrounded.** It can disagree
  with what the user sees. Use the `viewport_force_log` (probe-scroll) and the user's
  eyes for live scroll position; never trust `viewport_y` alone when unfocused.
- **PUBLIC vs EFFECTIVE viewport.** `buffer.active.viewportY` (public) is the buffer
  position; `effectiveXtermViewportY` (render/ydisp) is what's painted. They diverge on
  a stale-render strand (bg→fg) — public reads at-bottom while the render is stranded
  above. Measure strands with the EFFECTIVE value (what `app state` reports).
- **Wayland focus trap.** On KDE Wayland a visible FOREGROUND window reports
  `document.hasFocus()=false` (`document_focused=false`). NEVER gate layout/render
  mutations on focus — gate on VISIBILITY (`hostLooksUsable`). And you CANNOT synthesize
  the OS window-focus (bg→fg) trigger eye-free on guihost (wmctrl/xdotool are X11) — that
  one transition needs a user trigger; everything else is agent-instrumentable.
- **Daemon screen = authoritative; client buffer can be stale.** A "broken bottom" is
  almost always client-paint vs a correct daemon screen — diff them.
- **Screenshots: FIXED for the terminal (v2.8.46).** `server app screenshot` now
  composites the xterm canvas in-process (`xterm_canvas_composite`, faithful) — works over
  SSH, unfocused, any platform. The old "screenshots lie on Wayland" trap
  (`finding-app-screenshot-unfaithful-on-wayland`) was the Spectacle path needing window
  focus the agent can't hold; that's now bypassed for the terminal. (Full-app/non-terminal
  chrome still uses the webkit/Spectacle path — faithful for DOM, canvas-blind only if you
  capture the terminal region via the full-app path instead of the composite.)
- **Passing deterministic test ≠ live-fixed** — verify the ACTUAL live path/source the
  symptom uses (the 2.8.26 reconcile passed its string test but the live reveal carried
  a different `retained_replay_source`).
- **Don't free-list issues from raw telemetry fields** — a field name may not mean what
  it says (`input_enabled` once meant focus-ownership, not "user can type"). Read the
  code that sets it or falsify against a live probe before citing it.

## Pointers
`docs/integration-testing.md` (harness usage), `docs/xterm-bugs.md` (the xterm.js bug
registry — every workaround has an `// XTERM-BUG: <id>` anchor + entry), `docs/xterm.md`
(rendering/PTY bytes). Memory: `campaign-xterm-dealbreakers` (the master plan + which
bugs recur), `audit-viewport-scroll-control-flow` (the scroll/follow class + the
consolidated controller design + live captures).

exec
/bin/bash -lc 'ygg-memory sync --harness codex --ns=campaign-yggterm' in /tmp/s49-sol-consult
 exited 1 in 9982ms:
Traceback (most recent call last):
  File "/home/pi/.local/bin/ygg-memory.py", line 1426, in _flock_open
    fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    ~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
BlockingIOError: [Errno 11] Resource temporarily unavailable

During handling of the above exception, another exception occurred:

Traceback (most recent call last):
  File "/home/pi/.local/bin/ygg-memory.py", line 3204, in <module>
    main()
    ~~~~^^
  File "/home/pi/.local/bin/ygg-memory.py", line 3185, in main
    cmd_sync(args)
    ~~~~~~~~^^^^^^
  File "/home/pi/.local/bin/ygg-memory.py", line 2596, in cmd_sync
    result = _sync_harness_namespace(root, harness, ns, getattr(args, "local_dir", None))
  File "/home/pi/.local/bin/ygg-memory.py", line 2513, in _sync_harness_namespace
    lock = _flock_open(root / ".ygg-memory.lock")
  File "/home/pi/.local/bin/ygg-memory.py", line 1439, in _flock_open
    raise RuntimeError(
    ...<3 lines>...
    )
RuntimeError: Lock held by another process: /home/pi/.yggterm/memory/.ygg-memory.lock (waited 10s — another ygg-memory verb is mid-run; retry shortly, or let the catch-up tick cover it)

codex
I’m using the yggterm-diagnostics skill to assess the proof requirements. The supplied code already narrows the search: `terminalDioxusSend` is fixed per invocation, and `term.cols` is read before the attempt counter increments. Both constrain which loss explanations are possible.
exec
/bin/bash -lc 'ygg-memory sync --harness codex --ns=campaign-yggterm' in /tmp/s49-sol-consult
 exited 1 in 9981ms:
Traceback (most recent call last):
  File "/home/pi/.local/bin/ygg-memory.py", line 1426, in _flock_open
    fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    ~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
BlockingIOError: [Errno 11] Resource temporarily unavailable

During handling of the above exception, another exception occurred:

Traceback (most recent call last):
  File "/home/pi/.local/bin/ygg-memory.py", line 3204, in <module>
    main()
    ~~~~^^
  File "/home/pi/.local/bin/ygg-memory.py", line 3185, in main
    cmd_sync(args)
    ~~~~~~~~^^^^^^
  File "/home/pi/.local/bin/ygg-memory.py", line 2596, in cmd_sync
    result = _sync_harness_namespace(root, harness, ns, getattr(args, "local_dir", None))
  File "/home/pi/.local/bin/ygg-memory.py", line 2513, in _sync_harness_namespace
    lock = _flock_open(root / ".ygg-memory.lock")
  File "/home/pi/.local/bin/ygg-memory.py", line 1439, in _flock_open
    raise RuntimeError(
    ...<3 lines>...
    )
RuntimeError: Lock held by another process: /home/pi/.yggterm/memory/.ygg-memory.lock (waited 10s — another ygg-memory verb is mid-run; retry shortly, or let the catch-up tick cover it)

exec
/bin/bash -lc 'printenv CODEX_THREAD_ID' in /tmp/s49-sol-consult
 succeeded in 0ms:
01a13f89-817c-7541-b556-149235675917

2026-10-15T12:30:50.833411Z ERROR codex_core::tools::router: error=apply_patch verification failed: Failed to find expected lines in /home/pi/.codex/memories/MEMORY.md:
# Task Group: /home/pi/gh/yggterm GPT tracing research campaign

scope: read-only ytrace investigation of yggterm and ychrome, reporting/paper updates, and campaign continuation; use for recurring tracing rather than implementation work.
applies_to: cwd=/home/pi/gh/yggterm (and ~/gh/ychrome for offscreen probes); reuse_rule=measurements and host state are time-specific, but the read-only boundary, false-zero checks, and campaign opening procedure are reusable.

## Task 1: GPT tracing runtime audit of yggterm and ychrome

### rollout_summary_files

- rollout_summaries/2026-08-30T07-27-14-fAlx-gpt_tracing_msggraph_campaign_and_steer.md (cwd=/home/pi/gh/yggterm, rollout_path=/home/pi/.codex/sessions/2026/08/30/rollout-2026-08-30T12-57-14-01a05190-b999-79d1-9f13-1c124083eadf.jsonl, updated_at=2026-08-30T08:15:59+00:00, thread_id=01a05190-b999-79d1-9f13-1c124083eadf, success)

### keywords

- ytrace, ychrome, false-zero, Provider, control-socket, daemon_request/snapshot, fired=0, UI-block, yggterm-host-panic, journal salvage

## Task 2: Establish durable continuation memory for read-only tracing

### rollout_summary_files

- rollout_summaries/2026-08-30T07-27-14-fAlx-gpt_tracing_msggraph_campaign_and_steer.md (cwd=/home/pi/gh/yggterm, rollout_path=/home/pi/.codex/sessions/2026/08/30/rollout-2026-08-30T12-57-14-01a05190-b999-79d1-9f13-1c124083eadf.jsonl, updated_at=2026-08-30T08:15:59+00:00, thread_id=01a05190-b999-79d1-9f13-1c124083eadf, open follow-up)

### keywords

- continue gpt tracing campaign memory, cmapign, campaign-gpt-tracing, ygg-memory, scripts/ygg-live-host.sh, source-of-truth, observer, draft-campaign-trace-fixing.md

## User preferences

- when running this campaign, the user requested “research ONLY and reporting workflow” -> do not patch, build, restart, deploy, commit, or push product repos; paper and evidence-note updates are allowed. [Task 1]
- the user asked for “critical bugs, probable optimization pathways, and more probe areas/domain” -> rank findings, quantify evidence, separate proven facts from hypotheses, and leave a concrete next-probe queue. [Task 1]
- recognize “continue gpt tracing campaign memory” (and the typo “cmapign”) as the continuation trigger; resume the first unchecked probe. Keep implementation fixes in `draft-campaign-trace-fixing.md`, not in the research campaign. [Task 2]

## Reusable knowledge

- Critical provider-routing defect: at least four ytrace Providers for one app/PID competed for `<app>-<pid>.sock`; later construction unlinked earlier listeners, and the registry could advertise only a partial catalogue. `daemon_request/snapshot -> @count` drained with `fired=0` while the file plane recorded 53 and 24 matching events; the catalogue changed from 33 domain probes to three `trace/*` probes without restart. [Task 1]
- Do not trust script-plane zeroes until provider ownership is singleton/composable, registry/socket identity is checked, and an attach canary proves a known event. [Task 1]
- Observed dev costs: 111 UI blocks in one hour; snapshot p50/p95 264/399 ms; local-tree scan p50/p95 425 ms/22.72 s and max 55.66 s. The daemon reached 92.25% then 91.20% CPU and had 123 rows/72 PTY reader-writer pairs; next join is per-row bytes, wakeups, parser/child CPU, visibility, and working state. [Task 1]
- `yggterm-host-panic` briefly used 94.41% of one core but later idled; its tick includes filesystem/process/sensor work and lacks subspans. Instrument sensors before assigning cause. Ychrome had no live Provider; its offscreen transaction was open 1.817 s, ready 4 ms engine time, capture 172 ms, PSS 184 MB -> 280 MB -> 198 MB. [Task 1]
- Historical ychrome journals had 2,472/25,631 invalid lines on one host and 11/6,285 on another; a non-mutating salvage/index reader should report valid, invalid, recovered, and lost records. [Task 1]
- Campaign opening: check ygg-memory status/diff and relevant skills, preserve dirty state, resolve the live host with `scripts/ygg-live-host.sh`, use `~/.yggterm/scratchpad/gpt-tracing-<date>/`, declare source of truth versus observers, and verify cleanup. Queue provider identity, host-panic subspans, PTY scale, snapshot/tree-scan decomposition, main-thread joins, ychrome lifecycle/handover, journal salvage, and longer baselines. [Task 2]

## Failures and how to do differently

- Symptom: `fired=0` -> cause may be wrong Provider/routing, not a quiet event source -> cross-check script and file planes, verify reachable provider identity, and use an attach canary. [Task 1]
- Symptom: a UI-block `subject` points at a component -> cause: it is the last activity before the gap, not causal evidence -> add phase begin/end correlation before blaming Dioxus, terminal, telemetry, or web-surface work. [Task 1]
- Symptom: the observer creates the hot sample -> cause: host-panic sensor work perturbs what it measures -> bound sensor work and add subspans before optimization. [Task 1]

codex
The experiment needs two additions before it can localize the loss: **identity per invocation** and **an observation point before payload construction**. A ledger inside `sendTerminalEvent` alone cannot distinguish all four candidates.

This assessment uses your supplied excerpts. Campaign-memory synchronization failed twice because its lock was held; I did not proceed into repository exploration.

**Q1 — `accept-conditional(rank by investigation priority; establish same-invocation identity first)`**

| Site | Required signature | Assessment |
|---|---|---|
| **A: null send API** | Payload built; helper entered; `no-send-api`; `emitPaint` completes normally. | **Excluded for the receipt-delivering invocation.** `terminalDioxusSend` is a captured `const`: null early cannot become callable later. A callable bridge whose receiver dies is a different failure. |
| **B: payload construction throws** | Attempt recorded; no helper entry; synchronous exception before entry; no completion for that invocation of `emitPaint`. | **The proposed undefined-`term` explanation does not fit.** `term.cols` and `term.rows` were already dereferenced while constructing `nextPaintKey`, before incrementing attempts. An ordinary undefined `term` therefore cannot produce the measured increment. |
| **C: send throws synchronously** | Helper entered; `threw`; no normal emit completion. | Viable. Later receipt delivery does not exclude a transient exception. Its actual plausibility depends on the bridge implementation. |
| **D1: decoding becomes Ignored** | Send returned; corresponding `js_event_ignored` contains the Paint payload and decoding reason. | Viable and the cheapest outstanding check. Field-name agreement does not establish valid runtime values. |
| **D2: returned, subsequently lost or undrained** | Send returned; no receive observation; queue/receiver lifecycle evidence identifies the boundary. | Viable as a broad category, **not localized by a returned ledger entry alone**. Returning does not prove enqueue. |

Investigation order: **grep Ignored → establish invocation/receiver identity → distinguish C from returned-without-arrival → investigate that transport boundary**. A and the ordinary undefined-`term` version of B should move down sharply.

Two corrections matter:

- `paintCount` increments on unchanged-key calls too. Its aggregate movement cannot establish that a particular counted send attempt completed. Record completion against the attempt ID.
- A **temporarily undrained, retained FIFO** does not explain early Paint disappearing while a later receipt reaches the same receiver: earlier messages should eventually drain first. You need destruction, silent transport rejection, another consumer, decoding rejection, or different invocation/queue identities.

“Early shed window, later receipt survives” is therefore **conditionally coherent**, despite a valid bridge at invocation. Validity then does not guarantee receiver lifetime. But an irreversibly dead captured receiver cannot heal simply because time passes; the explanation must identify what changed or establish that the receipt used another invocation.

**Q2 — `accept-conditional(gated, staged, identity-preserving, globally bounded)`**

Use a **test gate for this localization experiment**. Production instrumentation is a separate decision after measuring its overhead. Paint-key dedupe does not bound the frequency of other event sends.

The minimum record should contain:

- Page instance, session/mount epoch, **invocation ID**, bridge identity/source, host generation.
- Monotonic attempt ID, kind, timestamp, and a compact Paint payload snapshot.
- Stages: `attempt-recorded → payload-built → send-entered → returned|no-send-api|threw`, plus `emit-completed`.

Record `attempt-recorded` **before evaluating the payload literal**. Otherwise B remains invisible. Include the small publication block between increment and send in the observed boundary: it is another potential exception site.

Practical initial bounds: **32 records per invocation, ≤8 KiB**, with a **window-wide cap of 256 records/64 KiB**. Bound retained invocation metadata too. Preserve total counters, eviction counts and retained sequence range; overflow must be visible. A shared ring indexed by invocation avoids maintaining two independently writable copies.

Keep records independent of the current host entry. A cold callback must not silently write into a replacement invocation’s host slot.

To minimize perturbation:

- Preallocate storage; avoid console output, stack capture, full-payload serialization and additional DOM probes.
- Preserve synchronous exception behavior: record, then rethrow.
- Do not add timers or per-send IPC.
- Capture bridge capability/source once per invocation.
- Preserve the existing polling schedule; retrieve after the critical window where possible.

No instrumentation is timing-neutral. Run the **same instrumentation on RED and GREEN**, and compare reproduction frequency with an uninstrumented control. If the loss disappears with the ledger, that is an inconclusive instrumented run.

Also verify whether `send` returns a Promise. Until verified, label the outcome **`returned-sync`**, not “enqueued” or “succeeded.”

**Q3 — `accept-conditional(add receive-before-dispatch evidence and sequence accounting)`**

The outcome map should be:

| Observation | Conclusion |
|---|---|
| Attempt → payload built → helper entered → `no-send-api` | **A** |
| Attempt → construction exception, no helper entry | **B**, localized by exception site |
| Helper entered → synchronous send exception | **C** |
| `returned-sync` → matching Ignored | **D1: decoding rejection** |
| `returned-sync` → raw Paint receive → no witness advancement | **Handler/predicate/identity problem**, outside the proposed send-loss sites |
| `returned-sync` → no receive, same receiver demonstrably exits before draining | **Consistent with D2**; enqueue remains unproved |
| `returned-sync` → no receive, receiver remains live | **Unresolved transport loss**; do not name it “undrained” yet |

`paint_ready` is a guarded first-arrival trace, not an event-arrival counter. Add a compact Paint receive observation **before witness conditions and ownership filters**, correlated with the invocation/attempt identity. Preserve rejected/stale arrival reasons too.

The minimal pair is:

- **RED:** the organic failure reproduces on the main implementation with instrumentation; identity is unambiguous; the ledger and receiver evidence identify the failed boundary; no relevant evidence was evicted.
- **GREEN:** the equivalent organic scenario delivers qualifying Paint from the current invocation to the remote witness and produces `paint_ready`, with no unexplained sequence gaps or stale-owner credit.

One GREEN is an existence proof. For this timing-dependent defect, repeat the previously failing lifecycle and churn boundary.

Require **at-most-once witness application per logical event**, especially with dual delivery. Duplicate transport arrivals can be legitimate; duplicate application must be prevented. Do not impose exactly-once wire delivery unless that is explicitly the channel contract.

**Q4 — `accept-conditional(observation-only first; causally qualified witness integration later)`**

Yes, reconciliation can misorder observations against the output fence.

A Paint observed before seed installation can be retrieved afterward. **Retrieval time does not establish observation time or fence eligibility.** Further, this payload reports DOM structure and dimensions; it does not itself prove that fenced seed content rendered.

For the narrow Paint-channel repair, start with a **separate observation-only record**. Do not let a recovered historical observation set current `painted=true` merely because it arrived late. L2 already removed the need to rush this into readiness.

Sharing the organic witness path is appropriate only after establishing a common eligibility predicate that checks:

- Current page, mount, invocation, host and owner identity.
- Fence/output generation captured **when the observation occurred**.
- The ordering needed by the specific witness claim.
- Shared deduplication across push and reconciliation.
- Explicit rejection of stale observations.

Both legs should then call that predicate, rather than maintaining separate witness semantics.

A per-mount cursor is insufficient unless its namespace also distinguishes page/mount replacement. Advance it after application, preserve it for the intended receiver lifetime, and report ring overflow as a gap. A bounded ring cannot promise arbitrary lossless recovery.

Finally, a successful fallback proves **recovered end-to-end observation delivery**. It does not prove the original push leg stopped shedding.

**Q5 — `accept-conditional(the following blind spots must become explicit checks)`**

**(a) Second consumer — `defer(actual queue ownership audit)`**

Your grep supports one mount-loop drain. Verify aliases, helpers, evaluator ownership and the installed desktop dispatcher too. A different eval normally means a different queue, but this must be checked in the vendored implementation. Do not invent a second consumer merely to explain the loss.

**(b) Paint-specific serialization — `accept-conditional(inspect actual values and serializer)`**

Field order is not a credible discriminator for an ordinary tagged JSON object. Extra-field handling depends on the wire configuration.

Values remain a credible discriminator:

- `undefined` object properties can be omitted by ordinary JSON serialization.
- Non-finite numbers become `null` under ordinary `JSON.stringify`.
- Null, missing, fractional, negative or out-of-range values can fail a required `u16`.

Those outcomes should become Ignored **if they reach the supplied decoder**. Store actual values and types, not merely the keys. The exact `dioxus.send` serialization behavior remains unverified here.

**(c) Suppression guard — `defer(value provenance and all assignment sites)`**

Record its actual value once at mount-loop entry. If continuously true for the same queue, it should suppress the receipt drain as well. Same-queue receipt delivery therefore contradicts continuous suppression. Whether production can enable it requires source inspection.

**(d) Two invocations — `accept`**

This is a substantial confounder. Child count oscillation alone proves neither one invocation nor two. Rebinding can let old closures observe the current host, and a shared host entry can mix their published counters. Invocation and bridge identity must accompany every record.

**Two additional misses:**

1. **The dedupe key commits before send outcome.** After a no-op, throw or silent loss, a stable key prevents retry. That can turn a transient failure into indefinite Paint silence.
2. **The receipt is a lane observation.** It constrains the Paint explanation only after proving the same page, invocation, captured send function and receiver ownership interval. “Same helper name” is insufficient.

**Verdict:** proceed with measurement after adding the pre-construction stage, invocation identity and receive-before-dispatch observation. Defer the fallback design until the returned-but-unreceived branch is actually measured.


tokens used
44,475
The experiment needs two additions before it can localize the loss: **identity per invocation** and **an observation point before payload construction**. A ledger inside `sendTerminalEvent` alone cannot distinguish all four candidates.

This assessment uses your supplied excerpts. Campaign-memory synchronization failed twice because its lock was held; I did not proceed into repository exploration.

**Q1 — `accept-conditional(rank by investigation priority; establish same-invocation identity first)`**

| Site | Required signature | Assessment |
|---|---|---|
| **A: null send API** | Payload built; helper entered; `no-send-api`; `emitPaint` completes normally. | **Excluded for the receipt-delivering invocation.** `terminalDioxusSend` is a captured `const`: null early cannot become callable later. A callable bridge whose receiver dies is a different failure. |
| **B: payload construction throws** | Attempt recorded; no helper entry; synchronous exception before entry; no completion for that invocation of `emitPaint`. | **The proposed undefined-`term` explanation does not fit.** `term.cols` and `term.rows` were already dereferenced while constructing `nextPaintKey`, before incrementing attempts. An ordinary undefined `term` therefore cannot produce the measured increment. |
| **C: send throws synchronously** | Helper entered; `threw`; no normal emit completion. | Viable. Later receipt delivery does not exclude a transient exception. Its actual plausibility depends on the bridge implementation. |
| **D1: decoding becomes Ignored** | Send returned; corresponding `js_event_ignored` contains the Paint payload and decoding reason. | Viable and the cheapest outstanding check. Field-name agreement does not establish valid runtime values. |
| **D2: returned, subsequently lost or undrained** | Send returned; no receive observation; queue/receiver lifecycle evidence identifies the boundary. | Viable as a broad category, **not localized by a returned ledger entry alone**. Returning does not prove enqueue. |

Investigation order: **grep Ignored → establish invocation/receiver identity → distinguish C from returned-without-arrival → investigate that transport boundary**. A and the ordinary undefined-`term` version of B should move down sharply.

Two corrections matter:

- `paintCount` increments on unchanged-key calls too. Its aggregate movement cannot establish that a particular counted send attempt completed. Record completion against the attempt ID.
- A **temporarily undrained, retained FIFO** does not explain early Paint disappearing while a later receipt reaches the same receiver: earlier messages should eventually drain first. You need destruction, silent transport rejection, another consumer, decoding rejection, or different invocation/queue identities.

“Early shed window, later receipt survives” is therefore **conditionally coherent**, despite a valid bridge at invocation. Validity then does not guarantee receiver lifetime. But an irreversibly dead captured receiver cannot heal simply because time passes; the explanation must identify what changed or establish that the receipt used another invocation.

**Q2 — `accept-conditional(gated, staged, identity-preserving, globally bounded)`**

Use a **test gate for this localization experiment**. Production instrumentation is a separate decision after measuring its overhead. Paint-key dedupe does not bound the frequency of other event sends.

The minimum record should contain:

- Page instance, session/mount epoch, **invocation ID**, bridge identity/source, host generation.
- Monotonic attempt ID, kind, timestamp, and a compact Paint payload snapshot.
- Stages: `attempt-recorded → payload-built → send-entered → returned|no-send-api|threw`, plus `emit-completed`.

Record `attempt-recorded` **before evaluating the payload literal**. Otherwise B remains invisible. Include the small publication block between increment and send in the observed boundary: it is another potential exception site.

Practical initial bounds: **32 records per invocation, ≤8 KiB**, with a **window-wide cap of 256 records/64 KiB**. Bound retained invocation metadata too. Preserve total counters, eviction counts and retained sequence range; overflow must be visible. A shared ring indexed by invocation avoids maintaining two independently writable copies.

Keep records independent of the current host entry. A cold callback must not silently write into a replacement invocation’s host slot.

To minimize perturbation:

- Preallocate storage; avoid console output, stack capture, full-payload serialization and additional DOM probes.
- Preserve synchronous exception behavior: record, then rethrow.
- Do not add timers or per-send IPC.
- Capture bridge capability/source once per invocation.
- Preserve the existing polling schedule; retrieve after the critical window where possible.

No instrumentation is timing-neutral. Run the **same instrumentation on RED and GREEN**, and compare reproduction frequency with an uninstrumented control. If the loss disappears with the ledger, that is an inconclusive instrumented run.

Also verify whether `send` returns a Promise. Until verified, label the outcome **`returned-sync`**, not “enqueued” or “succeeded.”

**Q3 — `accept-conditional(add receive-before-dispatch evidence and sequence accounting)`**

The outcome map should be:

| Observation | Conclusion |
|---|---|
| Attempt → payload built → helper entered → `no-send-api` | **A** |
| Attempt → construction exception, no helper entry | **B**, localized by exception site |
| Helper entered → synchronous send exception | **C** |
| `returned-sync` → matching Ignored | **D1: decoding rejection** |
| `returned-sync` → raw Paint receive → no witness advancement | **Handler/predicate/identity problem**, outside the proposed send-loss sites |
| `returned-sync` → no receive, same receiver demonstrably exits before draining | **Consistent with D2**; enqueue remains unproved |
| `returned-sync` → no receive, receiver remains live | **Unresolved transport loss**; do not name it “undrained” yet |

`paint_ready` is a guarded first-arrival trace, not an event-arrival counter. Add a compact Paint receive observation **before witness conditions and ownership filters**, correlated with the invocation/attempt identity. Preserve rejected/stale arrival reasons too.

The minimal pair is:

- **RED:** the organic failure reproduces on the main implementation with instrumentation; identity is unambiguous; the ledger and receiver evidence identify the failed boundary; no relevant evidence was evicted.
- **GREEN:** the equivalent organic scenario delivers qualifying Paint from the current invocation to the remote witness and produces `paint_ready`, with no unexplained sequence gaps or stale-owner credit.

One GREEN is an existence proof. For this timing-dependent defect, repeat the previously failing lifecycle and churn boundary.

Require **at-most-once witness application per logical event**, especially with dual delivery. Duplicate transport arrivals can be legitimate; duplicate application must be prevented. Do not impose exactly-once wire delivery unless that is explicitly the channel contract.

**Q4 — `accept-conditional(observation-only first; causally qualified witness integration later)`**

Yes, reconciliation can misorder observations against the output fence.

A Paint observed before seed installation can be retrieved afterward. **Retrieval time does not establish observation time or fence eligibility.** Further, this payload reports DOM structure and dimensions; it does not itself prove that fenced seed content rendered.

For the narrow Paint-channel repair, start with a **separate observation-only record**. Do not let a recovered historical observation set current `painted=true` merely because it arrived late. L2 already removed the need to rush this into readiness.

Sharing the organic witness path is appropriate only after establishing a common eligibility predicate that checks:

- Current page, mount, invocation, host and owner identity.
- Fence/output generation captured **when the observation occurred**.
- The ordering needed by the specific witness claim.
- Shared deduplication across push and reconciliation.
- Explicit rejection of stale observations.

Both legs should then call that predicate, rather than maintaining separate witness semantics.

A per-mount cursor is insufficient unless its namespace also distinguishes page/mount replacement. Advance it after application, preserve it for the intended receiver lifetime, and report ring overflow as a gap. A bounded ring cannot promise arbitrary lossless recovery.

Finally, a successful fallback proves **recovered end-to-end observation delivery**. It does not prove the original push leg stopped shedding.

**Q5 — `accept-conditional(the following blind spots must become explicit checks)`**

**(a) Second consumer — `defer(actual queue ownership audit)`**

Your grep supports one mount-loop drain. Verify aliases, helpers, evaluator ownership and the installed desktop dispatcher too. A different eval normally means a different queue, but this must be checked in the vendored implementation. Do not invent a second consumer merely to explain the loss.

**(b) Paint-specific serialization — `accept-conditional(inspect actual values and serializer)`**

Field order is not a credible discriminator for an ordinary tagged JSON object. Extra-field handling depends on the wire configuration.

Values remain a credible discriminator:

- `undefined` object properties can be omitted by ordinary JSON serialization.
- Non-finite numbers become `null` under ordinary `JSON.stringify`.
- Null, missing, fractional, negative or out-of-range values can fail a required `u16`.

Those outcomes should become Ignored **if they reach the supplied decoder**. Store actual values and types, not merely the keys. The exact `dioxus.send` serialization behavior remains unverified here.

**(c) Suppression guard — `defer(value provenance and all assignment sites)`**

Record its actual value once at mount-loop entry. If continuously true for the same queue, it should suppress the receipt drain as well. Same-queue receipt delivery therefore contradicts continuous suppression. Whether production can enable it requires source inspection.

**(d) Two invocations — `accept`**

This is a substantial confounder. Child count oscillation alone proves neither one invocation nor two. Rebinding can let old closures observe the current host, and a shared host entry can mix their published counters. Invocation and bridge identity must accompany every record.

**Two additional misses:**

1. **The dedupe key commits before send outcome.** After a no-op, throw or silent loss, a stable key prevents retry. That can turn a transient failure into indefinite Paint silence.
2. **The receipt is a lane observation.** It constrains the Paint explanation only after proving the same page, invocation, captured send function and receiver ownership interval. “Same helper name” is insufficient.

**Verdict:** proceed with measurement after adding the pre-construction stage, invocation identity and receive-before-dispatch observation. Defer the fallback design until the returned-but-unreceived branch is actually measured.


