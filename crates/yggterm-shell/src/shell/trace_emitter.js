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
// How many terminal channels to keep as drain fallbacks. More than one so a
// mid-flush unmount cannot strand a batch; few enough that a long-lived GUI
// does not accumulate them.
const YGG_TRACE_SENDERS_MAX = 8;
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
        // ⚠ Bounded, because every terminal MOUNT installs a fresh closure.
        // Dedup by identity cannot help — a remount produces a genuinely
        // different function — so an unbounded list grows one dead channel per
        // mount for the life of the GUI. The drain tries newest-first and stops
        // at the first that works, so stale entries cost nothing but memory,
        // which is exactly the leak that stays invisible until it is large.
        registerSender: (send) => {
            if (typeof send !== "function" || senders.indexOf(send) !== -1) {
                return;
            }
            senders.push(send);
            while (senders.length > YGG_TRACE_SENDERS_MAX) {
                senders.shift();
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


// ── attach-stream capture ────────────────────────────────────────────────
// The falsifier for the ghost-frame entry in docs/pending-bugs.md. That
// symptom is the canvas painted COLOURLESS — every SGR colour flattened while
// emoji keep theirs, which is what proves the ANSI attributes specifically were
// lost rather than the content. The daemon serves a formatted screen, so the
// flattening happens between that fetch and the canvas, and the open question
// is binary:
//
//   (A) the bytes reaching the canvas carry no SGR colour  => the reseed strips
//   (B) the bytes carry it and the canvas paints plain     => the attributes fail to apply
//
// ⛔⛔ AND THE OBVIOUS INSTRUMENT — A RING OF THE RAW BYTES — WOULD WRITE THE
// USER'S TERMINAL CONTENT TO A DIAGNOSTIC FILE THAT AGENTS READ AND QUOTE.
// The screen being captured is whatever they were working on: source, mail, a
// secret echoed by a prompt, an OSC 52 clipboard payload. A capture that
// answers a rendering question by recording the screen has traded a transient
// exposure for a durable one.
//
// ⇒ The control plane is preserved byte-for-byte and the CONTENT is not
// recorded at all. That is not a weaker instrument for this question: the
// answer lives entirely in the escape sequences, so what survives redaction IS
// the evidence, and a run of text becomes `·<length>·`. Two rules make it safe:
//
//   * CSI sequences (`ESC [ ... final`) are copied VERBATIM — their parameters
//     are numeric, and they are exactly what the question is about.
//   * OSC sequences (`ESC ] ... BEL/ST`) are reduced to their opcode and
//     length. ⛔ Never verbatim: OSC carries window titles and, at OSC 52, the
//     clipboard. That is the one escape family that IS content.
const YGG_CAPTURE_SAMPLE_MAX_CHARS = 2048;
const YGG_CAPTURE_BYTES_PER_ARM = 8192;
const YGG_CAPTURE_ARMS_PER_HOST = 16;
// SGR parameters that set a colour. 38/48 are the extended forms; the rest are
// the 8/16-colour and bright ranges. 39/49 are the DEFAULT-colour resets and
// count as colour-setting, because "something explicitly went back to default"
// is a distinguishable and interesting answer to a flattening question.
const yggSgrParamIsColour = (param) => {
    const value = Number(param);
    if (!Number.isFinite(value)) { return false; }
    return (value >= 30 && value <= 49) || (value >= 90 && value <= 107);
};
// Redact a chunk, preserving its control structure. Returns the sample plus a
// census, so a reader gets the answer without parsing the sample at all.
const yggRedactPreservingControls = (data) => {
    const text = String(data || '');
    const ESC = '\x1b';
    const BEL = '\x07';
    let sample = '';
    let plainRun = 0;
    let sgrTotal = 0;
    let sgrColour = 0;
    let sgrReset = 0;
    let oscCount = 0;
    let truncated = false;
    const flushPlain = () => {
        if (plainRun > 0) {
            sample += '·' + plainRun + '·';
            plainRun = 0;
        }
    };
    for (let i = 0; i < text.length; i++) {
        if (sample.length >= YGG_CAPTURE_SAMPLE_MAX_CHARS) { truncated = true; break; }
        const ch = text[i];
        if (ch !== ESC) {
            // Structural control characters are kept: they position the cursor
            // and shape the screen, and none of them carry content.
            if (ch === '\r' || ch === '\n' || ch === '\b' || ch === '\t' || ch === BEL) {
                flushPlain();
                sample += JSON.stringify(ch).slice(1, -1);
            } else {
                plainRun += 1;
            }
            continue;
        }
        flushPlain();
        const next = text[i + 1];
        if (next === '[') {
            // CSI: parameters, then a final byte in @-~.
            let end = i + 2;
            while (end < text.length && !(text[end] >= '@' && text[end] <= '~')) { end++; }
            const seq = text.slice(i, Math.min(end + 1, text.length));
            if (seq[seq.length - 1] === 'm') {
                sgrTotal += 1;
                const params = seq.slice(2, -1).split(';');
                if (params.some(yggSgrParamIsColour)) { sgrColour += 1; }
                if (params.every((p) => p === '' || Number(p) === 0)) { sgrReset += 1; }
            }
            sample += '\\e' + seq.slice(1);
            i = end;
            continue;
        }
        if (next === ']') {
            // OSC: opcode and LENGTH only. Never the payload.
            let end = i + 2;
            while (end < text.length && text[end] !== BEL
                && !(text[end] === ESC && text[end + 1] === '\\')) { end++; }
            const body = text.slice(i + 2, end);
            const opcode = (body.split(';')[0] || '').slice(0, 8);
            oscCount += 1;
            sample += '\\e]' + opcode + ';<' + body.length + '>';
            i = (text[end] === ESC) ? end + 1 : end;
            continue;
        }
        // A two-character escape (RIS, index, charset select ...).
        sample += '\\e' + (next === undefined ? '' : next);
        i += 1;
    }
    flushPlain();
    return {
        sample,
        chars: text.length,
        truncated,
        sgr_total: sgrTotal,
        sgr_colour: sgrColour,
        sgr_reset: sgrReset,
        osc_count: oscCount,
    };
};
if (window.__yggtermTrace && !window.__yggtermTrace.captureStream) {
    const captures = {};
    // Arm on the boundaries the symptom rides in on — a mount, a screen wipe, a
    // replay. Bounded twice over (bytes per arm, arms per host) so a re-attach
    // storm cannot turn the falsifier into the flood it was built to survive.
    window.__yggtermTrace.armStreamCapture = (hostId, reason) => {
        try {
            const key = String(hostId || '');
            let arm = captures[key];
            if (!arm) {
                arm = { arms: 0, budget: 0, reason: '' };
                captures[key] = arm;
            }
            if (arm.arms >= YGG_CAPTURE_ARMS_PER_HOST) { return; }
            arm.arms += 1;
            arm.budget = YGG_CAPTURE_BYTES_PER_ARM;
            arm.reason = String(reason || '');
        } catch (_error) {}
    };
    // ⭐ `stage` is the point of the capture, not a label on it. The question is
    // whether the RESEED writes different bytes from the live stream, so a
    // sample that cannot say which one it came from answers nothing.
    window.__yggtermTrace.captureStream = (hostId, stage, data) => {
        try {
            const arm = captures[String(hostId || '')];
            if (!arm || arm.budget <= 0) { return; }
            const text = String(data || '');
            if (!text.length) { return; }
            arm.budget -= text.length;
            const census = yggRedactPreservingControls(text);
            window.__yggtermTrace.emit({
                category: "xterm_attach",
                name: "stream_sample",
                payload: Object.assign({
                    host_id: String(hostId || ''),
                    stage: String(stage || ''),
                    arm_reason: arm.reason,
                    arm_index: arm.arms,
                }, census),
            });
        } catch (_error) {}
    };
    window.__yggtermTrace.redactPreservingControls = yggRedactPreservingControls;
}
