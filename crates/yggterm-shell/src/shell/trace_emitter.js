// ── the trace-plane emitter (layer=xterm) ──────────────────────────
// Grammar + rules: docs/spec-trace-plane-contract.md. The Rust
// boundary that validates every record is
// `yggterm-core::trace_contract`.
//
// ⛔⛔ THE EMITTER MUST NEVER BECOME THE PERTURBATION. The thing being
// measured here IS the thread this code runs on, so any design where
// emitting costs work proportional to the number of probes measures
// itself. That is not a theoretical concern: the pre-existing `debug`
// channel did one IPC hop and one synchronous file append PER EVENT,
// a reveal burst turned that into hundreds back-to-back, and the app
// froze for seconds — after which the standing remedy was a throttle
// that sheds exactly the events an investigator came for.
//
// ⇒ Three rules, and each closes one half of that failure:
//   1. `emit` does no I/O. It appends to a ring and returns.
//   2. The drain runs from a `setTimeout`, i.e. after the current task
//      finishes — never inside the write/render path being timed.
//   3. The timer is SELF-SUSPENDING: an idle terminal schedules no
//      wakeups at all, so the instrument costs nothing when there is
//      nothing to say.
const YGG_TRACE_RING_MAX = 512;
const YGG_TRACE_FLUSH_INTERVAL_MS = 250;
// Above this depth the drain is brought forward to the next task
// instead of waiting out the interval — a burst should reach the plane
// while it is still a burst, not averaged into the quarter-second
// after it.
const YGG_TRACE_HIGH_WATER = 64;
if (!window.__yggtermTrace) {
    const ring = [];
    const senders = [];
    const state = {
        seq: 0,
        // Records the ring dropped since the last one that got out.
        // ⛔ Carried ON the next accepted record rather than reported
        // separately, so that a drop can never itself be the thing
        // that gets dropped — which is what would happen under exactly
        // the sustained pressure that causes drops.
        dropped: 0,
        flushTimer: null,
        flushSoon: false,
    };
    const drain = () => {
        state.flushTimer = null;
        state.flushSoon = false;
        if (!ring.length) {
            return;
        }
        if (!senders.length) {
            // No mounted terminal owns a channel right now. Leave the
            // records in the ring; the next mount drains them. They
            // still carry their ORIGINAL `ts_ms`, so a late drain
            // reports when the event happened, not when a channel
            // reappeared.
            return;
        }
        const batch = ring.splice(0, ring.length);
        for (let i = senders.length - 1; i >= 0; i--) {
            try {
                senders[i]({ kind: "trace", records: batch });
                return;
            } catch (_error) {
                // A dead channel; try an older one.
            }
        }
        // Every channel refused. Put the batch back at the FRONT (it is
        // older than anything enqueued meanwhile) and let the ring cap
        // account for whatever will not fit.
        ring.unshift(...batch);
        while (ring.length > YGG_TRACE_RING_MAX) {
            ring.shift();
            state.dropped += 1;
        }
    };
    const schedule = (soon) => {
        if (state.flushTimer !== null) {
            if (!soon || state.flushSoon) {
                return;
            }
            clearTimeout(state.flushTimer);
        }
        state.flushSoon = Boolean(soon);
        state.flushTimer = setTimeout(
            drain,
            soon ? 0 : YGG_TRACE_FLUSH_INTERVAL_MS
        );
    };
    window.__yggtermTrace = {
        registerSender: (send) => {
            if (typeof send === "function" && senders.indexOf(send) === -1) {
                senders.push(send);
            }
        },
        emit: (record) => {
            try {
                if (!record || !record.category || !record.name) {
                    return;
                }
                if (ring.length >= YGG_TRACE_RING_MAX) {
                    // Drop the OLDEST. Under sustained pressure the
                    // newest records describe the state the app is in
                    // now, which is the question being asked; the
                    // oldest describe how it got there, which the
                    // earlier flushes already carried.
                    ring.shift();
                    state.dropped += 1;
                }
                // ⛔ `Date.now()` and not `performance.now()`, because
                // this field ORDERS the record against records written
                // by other processes and `performance.now()` is
                // measured from a per-document origin that no other
                // process shares. Durations are the opposite case and
                // use `performance.now()` deltas — see `span` below.
                record.ts_ms = Date.now();
                record.layer = record.layer || "xterm";
                record.component = record.component || "ui";
                state.seq += 1;
                record.seq = state.seq;
                if (state.dropped > 0) {
                    record.dropped = state.dropped;
                    state.dropped = 0;
                }
                ring.push(record);
                schedule(ring.length >= YGG_TRACE_HIGH_WATER);
            } catch (_error) {}
        },
        // A wall-clock span. ⛔ There is deliberately no cpu-clock
        // constructor: a webview content process has no per-thread CPU
        // clock, so a `cpu` duration from here could only be a wall
        // measurement wearing the wrong unit — and the Rust boundary
        // refuses one on arrival for that reason.
        span: (category, name, ctx) => {
            const startedAt = (window.performance && window.performance.now)
                ? window.performance.now()
                : Date.now();
            return {
                finish: (payload) => {
                    const endedAt = (window.performance && window.performance.now)
                        ? window.performance.now()
                        : Date.now();
                    window.__yggtermTrace.emit({
                        category,
                        name,
                        kind: "span",
                        clock: "wall",
                        duration_ms: Math.max(0, endedAt - startedAt),
                        payload: Object.assign({}, ctx || {}, payload || {}),
                    });
                },
            };
        },
        // A closed summary window. ⛔ Its `ts_ms` is the moment the
        // window ENDED, which is bookkeeping — the values inside are
        // faithful, the timestamp is not a moment anything happened.
        // The `kind` tag is what stops a reader correlating on it; see
        // docs/observability.md §4.3c for the analysis that fact once
        // invalidated.
        window: (category, name, payload) => {
            window.__yggtermTrace.emit({
                category,
                name,
                kind: "window",
                payload: payload || {},
            });
        },
        stats: () => ({
            queued: ring.length,
            dropped_pending: state.dropped,
            seq: state.seq,
            senders: senders.length,
        }),
        flush: drain,
    };
}
window.__yggtermTrace.registerSender(sendTerminalEvent);
const ytrace = window.__yggtermTrace;
