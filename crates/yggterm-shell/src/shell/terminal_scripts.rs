// ============================================================================
// SECTION: `terminal_eval_script` — the embedded xterm.js bootstrap script
// ----------------------------------------------------------------------------
// Generates ~30 000 lines of JavaScript that runs INSIDE the Dioxus WebView
// per terminal host. This is where most xterm.js integration bugs are
// fixed (scrollback intent, write bridge, scroll lock, paste paths, focus,
// retained replay, snapshot capture/restore, software canvas overlays).
// Search anchors:
//   * `setScrollbackIntent`, `forceXtermViewportY`, `scrollLiveCursorIntoView`
//   * `term.onData(`, `handlePrimarySelectionMiddleClick`
//   * `restoreXtermSessionSnapshotOnConstructed`, `captureSessionXtermSnapshot`
//   * `persistScrollStateToLocalStorage`, `loadScrollStateFromLocalStorage`
// Every fix in this script should leave a `// XTERM-BUG: <id>` anchor that
// points at the matching entry in docs/xterm-bugs.md.
// ============================================================================
/// The trace-plane emitter, shared verbatim between the shipped script and
/// `tools/xterm-harness/trace_emitter.test.js`.
///
/// ⭐ It lives in its own `.js` file rather than inline in the `format!` below
/// for one reason: an emitter whose only copy is a Rust string literal can be
/// syntax-checked by nothing and behaviour-tested by nothing, so its ring
/// bounds, its drop accounting and its self-suspending timer would all be
/// assertions in a comment. The harness loads THIS file, so the code under test
/// is the code that ships — not a transcription of it.
const TRACE_EMITTER_JS: &str = include_str!("trace_emitter.js");

fn terminal_eval_script(
    host_id: &str,
    theme: &TerminalTheme,
    initial_input_enabled: bool,
) -> String {
    terminal_eval_script_with_canvas_renderer(
        host_id,
        theme,
        initial_input_enabled,
        terminal_xterm_canvas_renderer_enabled(),
        &terminal_xterm_renderer_policy_reason(),
    )
}

/// The mount script with the read-only viewer's grid pinned in front of it.
///
/// A shadow may not resize the PTY (eng-review D8), so the ONLY way its frame
/// can be faithful is for its xterm to adopt the daemon's grid instead of
/// fitting to its own window: the viewer adapts to the session. Without this a
/// 167x57 shadow renders a stream the CLI wrote for 168x63 — different wrapping,
/// a short bottom, exactly the squish class this project keeps fighting, and a
/// screenshot that LIES.
///
/// `pinned_grid` is `None` for the user's own GUI (which owns the PTY and sizes
/// it by fitting, as before), so this prepends nothing on that path.
fn terminal_eval_script_with_pinned_grid(
    host_id: &str,
    theme: &TerminalTheme,
    initial_input_enabled: bool,
    pinned_grid: Option<(u64, u64)>,
) -> String {
    let script = terminal_eval_script(host_id, theme, initial_input_enabled);
    let Some((cols, rows)) = pinned_grid else {
        return script;
    };
    format!("window.__yggtermShadowPinnedGrid = {{ cols: {cols}, rows: {rows} }};\n{script}")
}

/// The grid a read-only viewer should pin its xterm to for `session_path`:
/// the daemon's PTY grid, which reaches the client only as the "PTY size"
/// metadata string (same SSOT the session/view contract check reads back).
/// `None` for any non-shadow client, and for a session whose grid is unknown or
/// unusable — pinning to a degenerate grid would be worse than fitting.
fn shadow_pinned_terminal_grid(shell: &ShellState, session_path: &str) -> Option<(u64, u64)> {
    if !client_is_shadow_viewer() {
        return None;
    }
    let session = shell.server.session_for_path(session_path)?;
    let (cols, rows) = parse_pty_size_cells(&metadata_value(session, "PTY size"))?;
    terminal_grid_is_usable_cells(cols, rows).then_some((cols, rows))
}

/// The Rust twin of the script's `terminalGridIsUsable` — 20x4 is the floor
/// below which a grid is treated as measurement noise rather than a real
/// terminal. Kept in one place so the two sides cannot drift.
fn terminal_grid_is_usable_cells(cols: u64, rows: u64) -> bool {
    cols >= 20 && rows >= 4
}
fn terminal_eval_script_with_canvas_renderer(
    host_id: &str,
    theme: &TerminalTheme,
    initial_input_enabled: bool,
    canvas_renderer_enabled: bool,
    renderer_policy_reason: &str,
) -> String {
    // SSOT for "which chrome owns the keyboard" — see UI_FOCUS_OWNER_SELECTORS.
    let ui_focus_owners = ui_focus_owner_selectors_js();
    let css = serde_json::to_string(XTERM_CSS).expect("serialize xterm css");
    let xterm = serde_json::to_string(XTERM_JS).expect("serialize xterm js");
    let fit_bundle = serde_json::to_string(XTERM_FIT_JS).expect("serialize xterm fit addon");
    let webgl_bundle =
        serde_json::to_string(XTERM_WEBGL_JS).expect("serialize xterm webgl addon");
    let background =
        serde_json::to_string(&theme.background).expect("serialize terminal background");
    let foreground =
        serde_json::to_string(&theme.foreground).expect("serialize terminal foreground");
    let cursor = serde_json::to_string(&theme.cursor).expect("serialize terminal cursor");
    let selection = serde_json::to_string(&theme.selection).expect("serialize terminal selection");
    let black = serde_json::to_string(&theme.black).expect("serialize terminal black");
    let red = serde_json::to_string(&theme.red).expect("serialize terminal red");
    let green = serde_json::to_string(&theme.green).expect("serialize terminal green");
    let yellow = serde_json::to_string(&theme.yellow).expect("serialize terminal yellow");
    let blue = serde_json::to_string(&theme.blue).expect("serialize terminal blue");
    let magenta = serde_json::to_string(&theme.magenta).expect("serialize terminal magenta");
    let cyan = serde_json::to_string(&theme.cyan).expect("serialize terminal cyan");
    let white = serde_json::to_string(&theme.white).expect("serialize terminal white");
    let bright_black =
        serde_json::to_string(&theme.bright_black).expect("serialize terminal bright black");
    let bright_red =
        serde_json::to_string(&theme.bright_red).expect("serialize terminal bright red");
    let bright_green =
        serde_json::to_string(&theme.bright_green).expect("serialize terminal bright green");
    let bright_yellow =
        serde_json::to_string(&theme.bright_yellow).expect("serialize terminal bright yellow");
    let bright_blue =
        serde_json::to_string(&theme.bright_blue).expect("serialize terminal bright blue");
    let bright_magenta =
        serde_json::to_string(&theme.bright_magenta).expect("serialize terminal bright magenta");
    let bright_cyan =
        serde_json::to_string(&theme.bright_cyan).expect("serialize terminal bright cyan");
    let bright_white =
        serde_json::to_string(&theme.bright_white).expect("serialize terminal bright white");
    let initial_input_enabled =
        serde_json::to_string(&initial_input_enabled).expect("serialize initial input enabled");
    let canvas_renderer_enabled =
        serde_json::to_string(&canvas_renderer_enabled).expect("serialize canvas renderer flag");
    let renderer_policy_reason =
        serde_json::to_string(renderer_policy_reason).expect("serialize renderer policy reason");
    let terminal_write_frame_ms = terminal_write_frame_ms();
    let terminal_active_write_frame_ms = terminal_active_write_frame_ms();
    let terminal_active_animation_write_frame_ms =
        terminal_active_animation_write_frame_ms().min(terminal_active_write_frame_ms);
    let terminal_active_animation_sustained_write_frame_ms =
        terminal_active_animation_sustained_write_frame_ms()
            .max(terminal_active_animation_write_frame_ms)
            .min(2_000);
    let terminal_active_animation_long_write_frame_ms =
        terminal_active_animation_long_write_frame_ms()
            .max(terminal_active_animation_sustained_write_frame_ms)
            .min(2_000);
    let terminal_inline_status_animation_sustained_after_ms =
        TERMINAL_INLINE_STATUS_ANIMATION_SUSTAINED_AFTER_MS;
    let terminal_inline_status_animation_long_after_ms =
        TERMINAL_INLINE_STATUS_ANIMATION_LONG_AFTER_MS;
    let font_family =
        serde_json::to_string(TERMINAL_FONT_FAMILY).expect("serialize terminal font family");
    let font_weight = serde_json::to_string(&terminal_font_weight(theme))
        .expect("serialize terminal font weight");
    let font_weight_bold = serde_json::to_string(&terminal_font_weight_bold(theme))
        .expect("serialize terminal bold font weight");
    let line_height = terminal_font_line_height(theme);
    let dim_foreground = serde_json::to_string(&terminal_dim_foreground(theme))
        .expect("serialize terminal dim foreground");
    let cursor_muted = serde_json::to_string(&terminal_cursor_muted(theme))
        .expect("serialize terminal muted cursor");
    let cursor_text = serde_json::to_string(&terminal_cursor_text(theme))
        .expect("serialize terminal cursor text");
    let input_line_background = serde_json::to_string(&terminal_input_line_background(theme))
        .expect("serialize terminal input line background");
    let input_line_border = serde_json::to_string(&terminal_input_line_border(theme))
        .expect("serialize terminal input line border");
    let input_line_decoration_enabled =
        serde_json::to_string(&terminal_xterm_input_line_decoration_enabled())
            .expect("serialize xterm input line decoration flag");
    let minimum_contrast_ratio = terminal_minimum_contrast_ratio(theme);
    let font_smoothing = serde_json::to_string(terminal_font_smoothing(theme))
        .expect("serialize terminal font smoothing");
    let moz_font_smoothing = serde_json::to_string(terminal_moz_font_smoothing(theme))
        .expect("serialize terminal moz font smoothing");
    let terminal_passive_focus_watchdog_ms = TERMINAL_PASSIVE_FOCUS_WATCHDOG_MS;
    let terminal_input_dead_trace_ms = TERMINAL_INPUT_DEAD_TRACE_MS;
    let terminal_input_dead_trace_interval_ms = TERMINAL_INPUT_DEAD_TRACE_INTERVAL_MS;
    let constructed_debug = if cfg!(debug_assertions) {
        "sendTerminalEvent({ kind: \"debug\", message: `constructed host=${hostId} fontSize=${term.options.fontSize} cols=${term.cols} rows=${term.rows}` });"
    } else {
        ""
    };
    let reset_debug = if cfg!(debug_assertions) {
        "sendTerminalEvent({ kind: \"debug\", message: `reset host=${hostId} fontSize=${term.options.fontSize}` });"
    } else {
        ""
    };
    format!(
        r#"
        const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
        const hostId = {host_id:?};
        const terminalDioxusApi = typeof dioxus !== "undefined" ? dioxus : null;
        const terminalDioxusSend =
            terminalDioxusApi && typeof terminalDioxusApi.send === "function"
                ? terminalDioxusApi.send.bind(terminalDioxusApi)
                : null;
        const terminalDioxusRecv =
            terminalDioxusApi && typeof terminalDioxusApi.recv === "function"
                ? terminalDioxusApi.recv.bind(terminalDioxusApi)
                : null;
        const sendTerminalEvent = (payload) => {{
            if (terminalDioxusSend) {{
                terminalDioxusSend(payload);
            }}
        }};
{trace_emitter_js}
        // ── xterm.js probes (layer=xterm) ──────────────────────────────────
        // These serve the open ghost-frame / glyph-soup entry in
        // docs/pending-bugs.md, whose fix direction asks for "the xterm.js write
        // queue instrumented to see what actually interleaves". Ordering is what
        // that question needs, and `ts_ms` cannot answer it: a reset, the reseed
        // that follows it and a bridge flush that lands between them routinely
        // share a millisecond. Every probe below therefore carries the emitter's
        // `seq`, which totally orders them because one emitter numbers them all.
        //
        // ⛔ RESOLUTION IS RATIONED ON PURPOSE. The ring is bounded, so a probe
        // that fires per steady-state event spends the whole budget describing
        // the boring case and evicts the switch that was the point. The split
        // below is the same floor-plus-window discipline the Rust probes use:
        // an always-on aggregate keeps the RATE honest, and point events are
        // spent only on outliers and on the boundaries where corruption lives.
        const YGG_XTERM_WINDOW_MS = 1000;
        // A write-queue depth past which one enqueue is worth a point event of
        // its own. Below it the aggregate carries the story.
        const YGG_XTERM_QUEUE_DEPTH_FLOOR = 16384;
        // A backlog older than this has stopped being a queue and become a lag.
        const YGG_XTERM_BACKLOG_AGE_FLOOR_MS = 250;
        // A gap between painted frames past which the canvas visibly stutters.
        const YGG_XTERM_FRAME_GAP_FLOOR_MS = 250;
        const xtermProbeWindows = {{}};
        // Close a window LAZILY, on the next event rather than on a timer.
        //
        // ⚠ The consequence, stated so no reader has to discover it: when output
        // stops, the final window stays open until something happens again, so
        // its `window_ms` can be far larger than the nominal interval. That is
        // why `window_ms` is measured and reported rather than assumed — a
        // consumer dividing by the constant would compute a rate that is wrong
        // by exactly the length of the silence. The alternative, a timer per
        // host, would spend wakeups on an idle terminal to report that nothing
        // happened, which is a worse trade for a laptop than a self-describing
        // window.
        const xtermProbeWindow = (name, seed) => {{
            let bucket = xtermProbeWindows[name];
            const now = Date.now();
            if (bucket && now - bucket.startedAtMs >= YGG_XTERM_WINDOW_MS) {{
                closeXtermProbeWindow(name);
                bucket = null;
            }}
            if (!bucket) {{
                bucket = Object.assign({{ startedAtMs: now, count: 0 }}, seed || {{}});
                xtermProbeWindows[name] = bucket;
            }}
            return bucket;
        }};
        const closeXtermProbeWindow = (name) => {{
            const bucket = xtermProbeWindows[name];
            if (!bucket || !bucket.count) {{
                delete xtermProbeWindows[name];
                return;
            }}
            delete xtermProbeWindows[name];
            const payload = Object.assign({{}}, bucket, {{
                host_id: hostId,
                window_ms: Math.max(0, Date.now() - bucket.startedAtMs),
            }});
            delete payload.startedAtMs;
            ytrace.window(name.split("/")[0], name.split("/")[1], payload);
        }};
        // Close every open window NOW. Called at the boundaries a corrupted
        // switch is investigated across, so the aggregate covering the moment
        // before a reset is on the plane BEFORE the reset's own point event
        // rather than folded into the window that follows it.
        const closeXtermProbeWindows = () => {{
            for (const name of Object.keys(xtermProbeWindows)) {{
                closeXtermProbeWindow(name);
            }}
        }};
        const traceXtermEnqueue = (chars, depth, backlogAgeMs) => {{
            const bucket = xtermProbeWindow("xterm_write/enqueue_window", {{
                chars: 0,
                max_depth: 0,
                max_backlog_age_ms: 0,
            }});
            bucket.count += 1;
            bucket.chars += chars;
            bucket.max_depth = Math.max(bucket.max_depth, depth);
            bucket.max_backlog_age_ms = Math.max(bucket.max_backlog_age_ms, backlogAgeMs);
            if (depth >= YGG_XTERM_QUEUE_DEPTH_FLOOR || backlogAgeMs >= YGG_XTERM_BACKLOG_AGE_FLOOR_MS) {{
                ytrace.emit({{
                    category: "xterm_write",
                    name: "enqueue_backlog",
                    payload: {{
                        host_id: hostId,
                        chars,
                        depth,
                        backlog_age_ms: backlogAgeMs,
                    }},
                }});
            }}
        }};
        // ⛔⛔ EVERY FLUSH AS A SPAN WAS MEASURED AND IT WAS TOO MUCH. The first
        // cut emitted one span per flush, reasoning that the write-frame budget
        // bounds the rate so it could not flood. On the live host that came to
        // **6500 spans in 42 minutes — 63% of all foreign records, and the
        // foreign records were 48.7% of the trace plane's BYTES.** The plane's
        // retention is a byte budget, so nearly doubling the write rate HALVES
        // the diagnostic window — for every reader, including the very
        // investigations these probes were added to serve.
        //
        // ⇒ The rule this file already applies to enqueue, applied here too:
        // an always-on aggregate keeps the RATE honest, and point resolution is
        // spent only where the question lives. What makes it safe is that the
        // expensive question — what interleaves across a corrupted switch — is
        // asked at BOUNDARIES, and a boundary arms full resolution for a while
        // (see `armXtermFlushDetail`). So the steady state is summarised and the
        // switch is recorded flush by flush.
        const YGG_XTERM_FLUSH_FLOOR_MS = 8;
        const YGG_XTERM_FLUSH_DETAIL_MS = 4000;
        let xtermFlushDetailUntilMs = 0;
        const armXtermFlushDetail = () => {{
            xtermFlushDetailUntilMs = Date.now() + YGG_XTERM_FLUSH_DETAIL_MS;
        }};
        const traceXtermFlush = (elapsedMs, detail) => {{
            const duration = Math.max(0, Number(elapsedMs) || 0);
            const now = Date.now();
            const repaired = Boolean(detail && detail.paint_repair_reason);
            // Keep every slow flush, every repaired flush, and everything inside
            // a boundary window. Those are the three shapes a reader ever asks
            // a single flush about.
            if (now < xtermFlushDetailUntilMs || duration >= YGG_XTERM_FLUSH_FLOOR_MS || repaired) {{
                ytrace.emit({{
                    category: "xterm_write",
                    name: "flush",
                    kind: "span",
                    clock: "wall",
                    duration_ms: duration,
                    payload: Object.assign({{ host_id: hostId }}, detail || {{}}),
                }});
                return;
            }}
            const bucket = xtermProbeWindow("xterm_write/flush_window", {{
                total_ms: 0,
                max_ms: 0,
                chars: 0,
            }});
            bucket.count += 1;
            bucket.total_ms += duration;
            bucket.max_ms = Math.max(bucket.max_ms, duration);
            bucket.chars += Number((detail && detail.raw_payload_length) || 0);
        }};
        const traceXtermRender = (rowStart, rowEnd, rows) => {{
            const now = Date.now();
            const bucket = xtermProbeWindow("xterm_render/frame_window", {{
                max_rows_painted: 0,
                full_canvas_frames: 0,
                max_gap_ms: 0,
                lastFrameAtMs: 0,
            }});
            const rowsPainted = Math.max(0, (Number(rowEnd) - Number(rowStart)) + 1);
            const gapMs = bucket.lastFrameAtMs ? now - bucket.lastFrameAtMs : 0;
            bucket.count += 1;
            bucket.lastFrameAtMs = now;
            bucket.max_rows_painted = Math.max(bucket.max_rows_painted, rowsPainted);
            bucket.max_gap_ms = Math.max(bucket.max_gap_ms, gapMs);
            // ⭐ A full-canvas repaint is the shape of the glyph-soup symptom —
            // the corruption reported is the WHOLE viewport unreadable, not a
            // damaged line. Counting them separately is what lets a reader ask
            // "how many times did this session repaint everything" without
            // reading every frame.
            if (rows > 0 && rowsPainted >= rows) {{
                bucket.full_canvas_frames += 1;
            }}
            if (gapMs >= YGG_XTERM_FRAME_GAP_FLOOR_MS) {{
                ytrace.emit({{
                    category: "xterm_render",
                    name: "frame_gap",
                    payload: {{
                        host_id: hostId,
                        gap_ms: gapMs,
                        rows_painted: rowsPainted,
                        rows,
                    }},
                }});
            }}
        }};
        // The interleave anchor. A reset wipes the screen and something must
        // reseed it; if a bridge flush lands between the two, the canvas holds
        // half of one screen and half of another — which is what unreadable
        // output looks like from the inside. `seq` on these three probes is what
        // turns that from a story into a total order a reader can check.
        const traceXtermScreenEvent = (name, detail) => {{
            closeXtermProbeWindows();
            // A wipe is one of the two boundaries the ghost-frame symptom rides
            // in on, so the stream capture arms HERE rather than on a timer —
            // what matters is the bytes that refill the screen, and nothing
            // else knows a refill is about to start.
            if (window.__yggtermTrace && window.__yggtermTrace.armStreamCapture) {{
                window.__yggtermTrace.armStreamCapture(hostId, String((detail && detail.reason) || name));
            }}
            armXtermFlushDetail();
            ytrace.emit({{
                category: "xterm_screen",
                name,
                payload: Object.assign({{ host_id: hostId }}, detail || {{}}),
            }});
        }};
        // ── the mount→paint chain (category `xterm_paint`) ─────────────────
        // ⭐ WHAT WAS MISSING WAS NOT xterm INSTRUMENTATION — it was a probe
        // that SPANS THE MOUNT. The probes above are real and are used here:
        // they count write-queue depth, painted frames and screen resets. But
        // every one of them counts EVENTS in a running terminal, and every
        // probe on the Rust side stops at this boundary — so "the mount began"
        // and "the glyphs arrived" were the same event to all of them, and a
        // mount BEGINS WITH AN EMPTY SURFACE. That is why a half-painted switch
        // and a clean one have been indistinguishable without a photograph.
        //
        // Four marks over one mount, joined to the native half by `host_id` —
        // which already encodes the mount epoch, so no second identity is
        // introduced and nothing has to be threaded across the bridge:
        //   open    `term.open()` returned: the surface exists and is BLANK
        //   write   the first bytes the canvas accepted
        //   parsed  xterm finished parsing them into its buffer
        //   frame   the renderer painted a frame
        //
        // ⛔⛔ AND A FRAME IS NOT A PAINT. The renderer repaints only the rows
        // it marked dirty, so "a frame happened" says nothing about how much of
        // the viewport it covered — which is exactly the distinction the eye
        // makes and the trace could not. `settle` answers the question the eye
        // asks: of the rows this terminal HOLDS TEXT ON, how many has any frame
        // since the mount actually covered. On a MOUNT that is a sound test
        // precisely because the surface started blank — every row must be
        // painted at least once for its content to be on screen — and it is the
        // reason this probe is scoped to a mount rather than left running.
        //
        // ⛔ AN INSTRUMENT THAT RUNS ON THE THING IT MEASURES READS ZERO. The
        // settle timer runs on the very thread whose stalls are under
        // investigation, so it is never trusted to have fired on time: it
        // reports the MEASURED window beside its nominal deadline, and the
        // overshoot between them is a UI-thread stall the probe survived rather
        // than a slow paint. ⚠ What it cannot report is a thread that never
        // comes back at all — for that the anchor is `mount_open`, emitted at
        // the surface, so a mount carrying no `first_frame` is a mount that
        // never painted, and the absence is legible from the native side by
        // joining on `host_id` alone. Both halves are needed; neither is
        // sufficient.
        const YGG_PAINT_SETTLE_MS = 1200;
        const YGG_PAINT_RECHECK_MS = 4000;
        // ⛔ `performance.now()` and not `Date.now()`, per the contract: every
        // number below is a DELTA between two reads inside this one document,
        // which is what the monotonic clock is for. `ts_ms` — the field that
        // orders these records against records written by other processes — is
        // stamped by the emitter from the epoch clock, and the two must not be
        // mixed.
        const paintNow = () => (window.performance && window.performance.now)
            ? window.performance.now()
            : Date.now();
        const paintChain = {{
            scriptStartedAtMs: paintNow(),
            hostReadyAtMs: 0,
            openedAtMs: 0,
            openSpan: null,
            firstWriteAtMs: 0,
            firstWriteChars: 0,
            firstWriteSource: '',
            firstParsedAtMs: 0,
            firstFrameAtMs: 0,
            frames: 0,
            blankFrames: 0,
            writes: 0,
            chars: 0,
            covered: null,
            coveredCount: 0,
            coveredRows: 0,
            coverageResets: 0,
            settles: 0,
        }};
        const paintDelta = (from, to) => (from > 0 && to > 0) ? Math.max(0, to - from) : null;
        // ⚠ `null` rather than `false` when the node cannot be measured. An
        // unmeasurable host and a host measured to be zero-sized are different
        // findings, and only one of them is about painting.
        const paintHostVisible = () => {{
            try {{
                const node = (term && term.element) || host;
                if (!node || typeof node.getBoundingClientRect !== 'function') {{
                    return null;
                }}
                const rect = node.getBoundingClientRect();
                return rect.width > 0 && rect.height > 0;
            }} catch (_error) {{
                return null;
            }}
        }};
        // How many of the viewport's rows this terminal holds text on.
        // ⛔ `-1` is "the buffer could not be read", which is NOT `0`. Blind is
        // not empty, and a reader that cannot tell them apart will read an
        // unreadable buffer as a terminal with nothing in it — i.e. as a
        // perfectly painted one.
        const paintRowsWithContent = () => {{
            try {{
                const buffer = (term && term.buffer && term.buffer.active) ? term.buffer.active : null;
                if (!buffer || typeof buffer.getLine !== 'function') {{
                    return -1;
                }}
                const rows = Math.max(0, Number((term && term.rows) || 0));
                const top = Number(buffer.viewportY || 0);
                let count = 0;
                for (let index = 0; index < rows; index += 1) {{
                    const line = buffer.getLine(top + index);
                    if (!line || typeof line.translateToString !== 'function') {{
                        continue;
                    }}
                    if (line.translateToString(true).trim().length > 0) {{
                        count += 1;
                    }}
                }}
                return count;
            }} catch (_error) {{
                return -1;
            }}
        }};
        const paintCoverageReset = (rows, reason) => {{
            const size = Math.max(0, Number(rows) || 0);
            paintChain.covered = size > 0 ? new Uint8Array(size) : null;
            paintChain.coveredCount = 0;
            paintChain.coveredRows = size;
            if (reason) {{
                paintChain.coverageResets += 1;
            }}
        }};
        const paintNoteWrite = (data, source) => {{
            try {{
                const chars = typeof data === 'string'
                    ? data.length
                    : Number((data && data.length) || 0);
                paintChain.writes += 1;
                paintChain.chars += chars;
                if (!paintChain.firstWriteAtMs) {{
                    paintChain.firstWriteAtMs = paintNow();
                    paintChain.firstWriteChars = chars;
                    paintChain.firstWriteSource = String(source || '');
                    // ⛔⛔ COVERAGE STARTS HERE, NOT AT THE MOUNT — and the
                    // self-test caught this before it shipped. A frame before
                    // any bytes painted a BLANK row, so counting it as covered
                    // lets a mount that blank-painted the whole canvas and then
                    // painted two rows of content report FULL coverage. That is
                    // not a small error: it masks exactly the partial paint this
                    // probe was built to see, and it masks it in the reassuring
                    // direction. No `coverage_resets` bump — a geometry change
                    // and the arrival of the first byte are different events and
                    // must not share a counter.
                    paintCoverageReset(Number((term && term.rows) || 0), '');
                }}
            }} catch (_error) {{}}
        }};
        const paintNoteParsed = () => {{
            if (!paintChain.firstParsedAtMs) {{
                paintChain.firstParsedAtMs = paintNow();
            }}
        }};
        const paintNoteFrame = (rowStart, rowEnd, rows) => {{
            try {{
                paintChain.frames += 1;
                const viewportRows = Math.max(0, Number(rows) || 0);
                if (viewportRows > 0 && viewportRows !== paintChain.coveredRows) {{
                    // A resize repaints the whole canvas, and coverage counted
                    // against the old geometry answers a question about a
                    // viewport that no longer exists.
                    paintCoverageReset(viewportRows, 'rows_changed');
                }}
                const covered = paintChain.covered;
                if (covered && covered.length) {{
                    const last = covered.length - 1;
                    const start = Math.max(0, Math.min(last, Number(rowStart) || 0));
                    const end = Math.max(start, Math.min(last, Number(rowEnd) || 0));
                    for (let index = start; index <= end; index += 1) {{
                        if (!covered[index]) {{
                            covered[index] = 1;
                            paintChain.coveredCount += 1;
                        }}
                    }}
                }}
                if (paintChain.firstFrameAtMs) {{
                    return;
                }}
                // ⛔⛔ MEASURED WRONG ON THE FIRST LIVE MOUNT AND KEPT AS A
                // COMMENT BECAUSE THE MISTAKE IS THE INTERESTING PART. The
                // first cut latched the first frame outright, and the very
                // first sandbox mount reported `open→frame 218 ms` with
                // `writes_before_frame: 0` — a span that measured the canvas
                // PAINTING ITSELF EMPTY, which is the exact event this probe
                // exists to distinguish from glyphs arriving. A frame before
                // any bytes is counted, never latched: it is the blank
                // surface, and a blank surface repainting is what a ghost
                // frame IS.
                if (!paintChain.firstWriteAtMs) {{
                    paintChain.blankFrames += 1;
                    return;
                }}
                paintChain.firstFrameAtMs = paintNow();
                const rowsPainted = Math.max(0, (Number(rowEnd) - Number(rowStart)) + 1);
                const detail = {{
                    host_id: hostId,
                    open_to_write_ms: paintDelta(paintChain.openedAtMs, paintChain.firstWriteAtMs),
                    write_to_parsed_ms: paintDelta(paintChain.firstWriteAtMs, paintChain.firstParsedAtMs),
                    parsed_to_frame_ms: paintDelta(paintChain.firstParsedAtMs, paintChain.firstFrameAtMs),
                    write_to_frame_ms: paintDelta(paintChain.firstWriteAtMs, paintChain.firstFrameAtMs),
                    // ⚠ The synchronous write path parses inside the write call
                    // and never reaches `onWriteParsed`, so a missing parse mark
                    // is a route, not a fault. Stated in the record rather than
                    // left for a reader to infer from a null.
                    parsed_seen: paintChain.firstParsedAtMs > 0,
                    first_write_chars: paintChain.firstWriteChars,
                    first_write_source: paintChain.firstWriteSource,
                    writes_before_frame: paintChain.writes,
                    chars_before_frame: paintChain.chars,
                    // Frames the renderer spent on the surface before it was
                    // handed a single byte. Not a fault on its own — the canvas
                    // has to exist before it can hold anything — but it is the
                    // count that turns "the mount flashed" into a number.
                    blank_frames_before_write: paintChain.blankFrames,
                    rows_painted: rowsPainted,
                    rows: viewportRows,
                    visible: paintHostVisible(),
                }};
                if (paintChain.openSpan) {{
                    // `duration_ms` is open → first frame: the empty surface to
                    // the first glyphs on it, which is the span this whole
                    // instrument exists to name.
                    paintChain.openSpan.finish(detail);
                    paintChain.openSpan = null;
                    return;
                }}
                ytrace.emit({{
                    category: "xterm_paint",
                    name: "first_frame",
                    payload: detail,
                }});
            }} catch (_error) {{}}
        }};
        const paintEmitSettle = (deadlineMs, recheck) => {{
            try {{
                paintChain.settles += 1;
                const windowMs = Math.max(0, paintNow() - paintChain.openedAtMs);
                const viewportRows = Math.max(0, Number((term && term.rows) || 0));
                if (viewportRows > 0 && viewportRows !== paintChain.coveredRows) {{
                    paintCoverageReset(viewportRows, 'rows_changed');
                }}
                const rowsWithContent = paintRowsWithContent();
                const coveredCount = paintChain.coveredCount;
                // ⭐ THE FIELD THE PROBE EXISTS FOR. Rows this terminal holds
                // text on that no frame since the mount has covered — i.e. rows
                // the session contains and the screen is not showing. Positive
                // is a partially-painted mount, which is what "broken TUI paint
                // on switching" is from the inside. `null` means the buffer was
                // unreadable and the question was not answered.
                const unpainted = rowsWithContent >= 0
                    ? Math.max(0, Math.min(rowsWithContent, viewportRows) - coveredCount)
                    : null;
                // ⛔ `painted` is "a frame landed after bytes reached the
                // canvas", NOT "a frame happened". The distinction is the whole
                // instrument: a mount with frames, no writes and an empty
                // buffer would otherwise report itself complete for having
                // faithfully painted nothing.
                const painted = paintChain.firstFrameAtMs > 0;
                const complete = painted && unpainted === 0;
                const visible = paintHostVisible();
                // ⚠ An invisible host is EXPECTED not to paint — the renderer
                // is idle by design — so rechecking one spends a second record
                // to restate a known fact. The reason no recheck follows is put
                // in the first record, because a missing record and a healthy
                // one look identical.
                const recheckScheduled = !recheck && !complete && visible === true;
                ytrace.window("xterm_paint", "settle", {{
                    host_id: hostId,
                    window_ms: windowMs,
                    deadline_ms: deadlineMs,
                    // How late the timer was. An overshoot past a frame or two
                    // did not measure a slow paint; it measured a UI thread
                    // that was not running.
                    overshoot_ms: Math.max(0, windowMs - deadlineMs),
                    recheck: Boolean(recheck),
                    recheck_scheduled: recheckScheduled,
                    painted,
                    complete,
                    frames: paintChain.frames,
                    blank_frames_before_write: paintChain.blankFrames,
                    rows: viewportRows,
                    rows_covered: coveredCount,
                    rows_with_content: rowsWithContent,
                    rows_content_unpainted: unpainted,
                    coverage_resets: paintChain.coverageResets,
                    writes: paintChain.writes,
                    chars: paintChain.chars,
                    open_to_frame_ms: paintDelta(paintChain.openedAtMs, paintChain.firstFrameAtMs),
                    visible,
                    document_hidden: Boolean(document.hidden),
                }});
                if (recheckScheduled) {{
                    window.setTimeout(
                        () => paintEmitSettle(YGG_PAINT_RECHECK_MS, true),
                        Math.max(0, YGG_PAINT_RECHECK_MS - YGG_PAINT_SETTLE_MS)
                    );
                }}
            }} catch (_error) {{}}
        }};
        const paintNoteHostReady = () => {{
            if (!paintChain.hostReadyAtMs) {{
                paintChain.hostReadyAtMs = paintNow();
            }}
        }};
        // ⛔ ONE tap per write route, and the sync route is resolved in the SAME
        // order `flushPendingWrite` resolves its own — wrapping both `writeSync`
        // surfaces would count every byte twice on whichever build delegates one
        // to the other, and a doubled byte count is the kind of wrong that reads
        // as a discovery.
        const paintInstallWriteTaps = () => {{
            try {{
                if (!term || term.__yggPaintTapped) {{
                    return false;
                }}
                term.__yggPaintTapped = true;
                if (typeof term.write === 'function') {{
                    const nativeWrite = term.write.bind(term);
                    term.write = (data, callback) => {{
                        paintNoteWrite(data, 'write');
                        return nativeWrite(data, callback);
                    }};
                }}
                const core = term._core;
                if (core && typeof core.writeSync === 'function') {{
                    const nativeSync = core.writeSync.bind(core);
                    core.writeSync = (data, maxSubsequentCalls) => {{
                        paintNoteWrite(data, 'write_sync');
                        return nativeSync(data, maxSubsequentCalls);
                    }};
                }} else if (core && core._writeBuffer && typeof core._writeBuffer.writeSync === 'function') {{
                    const writeBuffer = core._writeBuffer;
                    const nativeSync = writeBuffer.writeSync.bind(writeBuffer);
                    writeBuffer.writeSync = (data, maxSubsequentCalls) => {{
                        paintNoteWrite(data, 'write_sync');
                        return nativeSync(data, maxSubsequentCalls);
                    }};
                }}
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        const paintNoteMountOpen = (detail) => {{
            try {{
                paintChain.openedAtMs = paintNow();
                paintChain.openSpan = ytrace.span("xterm_paint", "first_frame", {{ host_id: hostId }});
                paintCoverageReset(Number((term && term.rows) || 0), '');
                const tapped = paintInstallWriteTaps();
                ytrace.emit({{
                    category: "xterm_paint",
                    name: "mount_open",
                    payload: Object.assign({{
                        host_id: hostId,
                        rows: Number((term && term.rows) || 0),
                        cols: Number((term && term.cols) || 0),
                        // The mount's own cost, split where it is spendable:
                        // waiting for the DOM node to exist is a different
                        // problem from building the surface once it does.
                        script_to_host_ms: paintDelta(paintChain.scriptStartedAtMs, paintChain.hostReadyAtMs),
                        host_to_open_ms: paintDelta(paintChain.hostReadyAtMs, paintChain.openedAtMs),
                        script_to_open_ms: paintDelta(paintChain.scriptStartedAtMs, paintChain.openedAtMs),
                        write_taps_installed: tapped,
                    }}, detail || {{}}),
                }});
                window.setTimeout(
                    () => paintEmitSettle(YGG_PAINT_SETTLE_MS, false),
                    YGG_PAINT_SETTLE_MS
                );
            }} catch (_error) {{}}
        }};
        const recvTerminalCommand = async () => {{
            if (!terminalDioxusRecv) {{
                return null;
            }}
            return await terminalDioxusRecv();
        }};
        const terminalWriteFrameMs = Math.max(0, Number({terminal_write_frame_ms} || 0));
        const terminalActiveWriteFrameMs = Math.max(0, Number({terminal_active_write_frame_ms} || 0));
        const terminalActiveAnimationWriteFrameMs = Math.max(0, Number({terminal_active_animation_write_frame_ms} || 0));
        const terminalActiveAnimationSustainedWriteFrameMs = Math.max(
            terminalActiveAnimationWriteFrameMs,
            Number({terminal_active_animation_sustained_write_frame_ms} || 0)
        );
        const terminalActiveAnimationLongWriteFrameMs = Math.max(
            terminalActiveAnimationSustainedWriteFrameMs,
            Number({terminal_active_animation_long_write_frame_ms} || 0)
        );
        const terminalInlineStatusAnimationSustainedAfterMs = Math.max(
            0,
            Number({terminal_inline_status_animation_sustained_after_ms} || 0)
        );
        const terminalInlineStatusAnimationLongAfterMs = Math.max(
            terminalInlineStatusAnimationSustainedAfterMs,
            Number({terminal_inline_status_animation_long_after_ms} || 0)
        );
        let host = document.getElementById(hostId);
        sendTerminalEvent({{ kind: "debug", message: `bootstrap host=${{hostId}} present=${{!!host}}` }});
        if (!host) {{
            for (let attempt = 0; attempt < 80; attempt += 1) {{
                await sleep(25);
                host = document.getElementById(hostId);
                if (host) {{
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `bootstrap host=${{hostId}} mounted on retry=${{attempt + 1}}`
                    }});
                    break;
                }}
            }}
        }}
        if (!host) {{
            sendTerminalEvent({{ kind: "debug", message: `bootstrap host missing for ${{hostId}} after retries; waiting for mount` }});
            let missingHostLogAt = Date.now();
            while (!host) {{
                await sleep(100);
                host = document.getElementById(hostId);
                const now = Date.now();
                if (!host && now - missingHostLogAt >= 2000) {{
                    missingHostLogAt = now;
                    sendTerminalEvent({{ kind: "debug", message: `bootstrap host still missing for ${{hostId}}` }});
                }}
            }}
            sendTerminalEvent({{ kind: "debug", message: `bootstrap host=${{hostId}} mounted after wait` }});
        }}
        paintNoteHostReady();
        const ensureXtermAssets = async () => {{
            window.__yggtermXtermBootstrapError = null;
            const styleId = "yggterm-xterm-style";
            if (!document.getElementById(styleId)) {{
                const style = document.createElement("style");
                style.id = styleId;
                style.textContent = {css};
                document.head.appendChild(style);
            }}
            const injectScript = (id, source) => {{
                if (document.getElementById(id)) {{
                    return;
                }}
                const script = document.createElement("script");
                script.id = id;
                script.type = "text/javascript";
                script.text = source;
                document.head.appendChild(script);
            }};
            try {{
                if (!window.Terminal) {{
                    injectScript("yggterm-xterm-script", {xterm});
                }}
                if (!window.FitAddon || !window.FitAddon.FitAddon) {{
                    injectScript("yggterm-xterm-fit-script", {fit_bundle});
                }}
                if (!window.WebglAddon || !window.WebglAddon.WebglAddon) {{
                    injectScript("yggterm-xterm-webgl-script", {webgl_bundle});
                }}
                for (let attempt = 0; attempt < 80; attempt += 1) {{
                    if (
                        window.Terminal
                        && window.FitAddon
                        && window.FitAddon.FitAddon
                    ) {{
                        return true;
                    }}
                    await sleep(50);
                }}
                return false;
            }} catch (error) {{
                window.__yggtermXtermBootstrapError = error && error.message ? error.message : String(error);
                return false;
            }}
        }};
        const assetsReady = await ensureXtermAssets();
        sendTerminalEvent({{
            kind: "debug",
            message: `assets host=${{hostId}} ready=${{assetsReady}} terminal=${{!!window.Terminal}} fit=${{!!(window.FitAddon && window.FitAddon.FitAddon)}} webgl=${{!!(window.WebglAddon && window.WebglAddon.WebglAddon)}} bootstrap=${{window.__yggtermXtermBootstrapError || "none"}}`
        }});
        if (
            !assetsReady
            || !window.Terminal
            || !window.FitAddon
            || !window.FitAddon.FitAddon
        ) {{
            const details = [
              window.Terminal ? "Terminal:ok" : "Terminal:missing",
              window.FitAddon && window.FitAddon.FitAddon ? "FitAddon:ok" : "FitAddon:missing",
              window.__yggtermXtermBootstrapError ? `Bootstrap:${{window.__yggtermXtermBootstrapError}}` : "Bootstrap:none",
            ].join(" · ");
            host.innerHTML = `<div style="padding:18px;color:#fca5a5;font:13px system-ui;">xterm.js assets failed to load.<br><span style="opacity:0.75">${{details}}</span></div>`;
            sendTerminalEvent({{ kind: "debug", message: details }});
            sendTerminalEvent({{ kind: "ready" }});
            while (true) {{
                await recvTerminalCommand();
            }}
        }}
        // BORING REVEAL ghost (spec-boring-session-loads): a reveal of a retained
        // host re-runs this whole eval, which wipes the host DOM and rebuilds a
        // fresh Terminal + full replay — the blank/blink frames of that churn are
        // what the user sees as the blink-blink shadow. Capture the ALREADY
        // PAINTED canvas pixels NOW (before the cleanup below disposes them) and
        // hold them as a static overlay across the rebuild; released after the
        // replay + reveal screen-reconcile settle, or on the first keystroke. The
        // wrong (blank/intermediate) frame never paints; the user sees last-frame
        // -> settled-frame, one transition. Purely visual: pointer-events none,
        // input and the terminal pipeline are untouched. Only arms when the SAME
        // hostId has a prior painted registry entry (a retained reveal); cold
        // mounts and fresh epochs have neither and never ghost. Kill switch:
        // window.__yggtermDisableRevealGhost.
        const captureRevealGhostFrame = () => {{
            try {{
                if (window.__yggtermDisableRevealGhost) {{
                    return null;
                }}
                const reg = window.__yggtermXtermHosts || {{}};
                const entryPainted = (candidate) => {{
                    const buf = candidate && candidate.term && candidate.term.buffer
                        && candidate.term.buffer.active;
                    return Boolean(
                        buf
                        && (Number(buf.baseY || 0) > 0
                            || Number(buf.cursorY || 0) > 0
                            || Number(buf.cursorX || 0) > 0)
                    );
                }};
                const screenForEntry = (candidate) => {{
                    const el = candidate && candidate.term && candidate.term.element
                        ? candidate.term.element : null;
                    return el && el.querySelector ? el.querySelector('.xterm-screen') : null;
                }};
                let screen = null;
                if (entryPainted(reg[hostId])) {{
                    screen = host.querySelector('.xterm-screen');
                }}
                if (!screen) {{
                    // Cold remount of the SAME session under a new mount epoch:
                    // the prior epoch's entry (different hostId, same sessionPath)
                    // is still in the registry at this point — the reap below has
                    // not run yet — and its detached canvases are still drawable.
                    // 5-sweep capture (2026-06-10) showed sweep reveals are nearly
                    // ALL cold remounts (167 constructs / 100 opens), so the
                    // same-hostId-only ghost covered just 38/100 reveals; this
                    // fallback covers the rest of the previously-painted ones.
                    const mountSessionPath = host.getAttribute('data-terminal-session-path') || '';
                    if (mountSessionPath) {{
                        for (const priorKey of Object.keys(reg)) {{
                            if (priorKey === hostId) {{ continue; }}
                            const candidate = reg[priorKey];
                            if (!candidate || candidate.sessionPath !== mountSessionPath) {{ continue; }}
                            if (!entryPainted(candidate)) {{ continue; }}
                            const candidateScreen = screenForEntry(candidate);
                            if (candidateScreen && candidateScreen.querySelectorAll('canvas').length) {{
                                screen = candidateScreen;
                                break;
                            }}
                        }}
                    }}
                }}
                const canvases = screen ? screen.querySelectorAll('canvas') : [];
                if (!screen || !canvases.length) {{
                    return null;
                }}
                const first = canvases[0];
                if (!first.width || !first.height) {{
                    return null;
                }}
                const ghost = document.createElement('canvas');
                ghost.width = first.width;
                ghost.height = first.height;
                const ctx = ghost.getContext('2d');
                if (!ctx) {{
                    return null;
                }}
                const hostBg = window.getComputedStyle(host).backgroundColor;
                if (hostBg) {{
                    ctx.fillStyle = hostBg;
                    ctx.fillRect(0, 0, ghost.width, ghost.height);
                }}
                for (const layer of canvases) {{
                    try {{
                        ctx.drawImage(layer, 0, 0);
                    }} catch (_layerError) {{}}
                }}
                const hostRect = host.getBoundingClientRect();
                const screenRect = screen.getBoundingClientRect();
                // A prior-epoch screen is DETACHED (rect 0x0): fall back to the
                // canvas backing-store size scaled by devicePixelRatio, anchored
                // at the host origin.
                const dpr = Math.max(1, Number(window.devicePixelRatio || 1));
                const attached = screenRect.width > 0 && screenRect.height > 0;
                const cssWidth = attached
                    ? Math.round(screenRect.width)
                    : Math.round(first.width / dpr);
                const cssHeight = attached
                    ? Math.round(screenRect.height)
                    : Math.round(first.height / dpr);
                ghost.className = 'yggterm-reveal-ghost';
                ghost.style.position = 'absolute';
                ghost.style.left = attached
                    ? `${{Math.max(0, Math.round(screenRect.left - hostRect.left))}}px`
                    : '0px';
                ghost.style.top = attached
                    ? `${{Math.max(0, Math.round(screenRect.top - hostRect.top))}}px`
                    : '0px';
                ghost.style.width = `${{cssWidth}}px`;
                ghost.style.height = `${{cssHeight}}px`;
                ghost.style.zIndex = '40';
                ghost.style.pointerEvents = 'none';
                return ghost;
            }} catch (_error) {{
                return null;
            }}
        }};
        const revealGhostFrame = captureRevealGhostFrame();
        window.__yggtermXtermCleanups = window.__yggtermXtermCleanups || {{}};
        if (window.__yggtermXtermCleanups[hostId]) {{
            try {{
                window.__yggtermXtermCleanups[hostId]();
            }} catch (_error) {{}}
        }}
        // REGISTRY-LEAK FIX: reap superseded-epoch hosts for THIS session path.
        // hostId embeds the mount epoch (`-m<N>`); on restart/switch the epoch bumps
        // to a NEW hostId, abandoning the prior epoch's entry. Its cleanup is keyed by
        // the OLD hostId and is only invoked when that exact hostId re-inits — which
        // never happens after an epoch bump. So the xterm.js Terminal + registry entry
        // leak, and every global pass over `__yggtermXtermHosts` (selection/paste/
        // switch) slows as the registry grows unbounded (measured 5->20+ on guihost). On
        // (re)mount of a path, dispose any OTHER registry entry for the SAME path whose
        // DOM host element is gone — that entry is a dead prior epoch. Other paths'
        // warm-retained entries are untouched. See [[finding-hot-switch-latency-remount]].
        try {{
            const mountSessionPath = host.getAttribute("data-terminal-session-path") || "";
            const reg = window.__yggtermXtermHosts || {{}};
            const cleanups = window.__yggtermXtermCleanups || {{}};
            if (mountSessionPath) {{
                for (const staleKey of Object.keys(reg)) {{
                    if (staleKey === hostId) {{ continue; }}
                    const other = reg[staleKey];
                    if (!other || other.sessionPath !== mountSessionPath) {{ continue; }}
                    const staleDom = document.getElementById(staleKey);
                    if (staleDom && staleDom.isConnected) {{ continue; }}
                    try {{
                        if (typeof cleanups[staleKey] === 'function') {{ cleanups[staleKey](); }}
                    }} catch (_staleCleanupError) {{}}
                    try {{ delete reg[staleKey]; }} catch (_e1) {{}}
                    try {{ delete cleanups[staleKey]; }} catch (_e2) {{}}
                }}
            }}
        }} catch (_reapError) {{}}
        // XTERM-BUG: right-edge-glyph-clipped — ONE owner for the right gutter.
        //
        // `.xterm-screen` is deliberately narrower than the host by the
        // scrollbar width (XTERM-BUG: scrollbar-not-draggable, below) and it
        // clips: `overflow: hidden`. The grid proposal used to divide the FULL
        // host width by the cell width, so it handed the terminal one more
        // column than the paint box can show and the last column was clipped to
        // a sliver — measured live on guihost at 3.0.45: cols 170 x 8px = 1360px of
        // canvas inside a 1353px screen, i.e. 1 of the final column's 8 pixels
        // visible. The user's symptom was exactly that: "sometimes a letter is
        // missing on the rightmost edge, and widening the window brings it back"
        // (widening changes the remainder, so whether the loss is visible on a
        // given line depends on where the text ends).
        //
        // ⛔ Do NOT widen the screen back to 100% — that re-breaks the
        // scrollbar hitbox. The gutter is a real reservation, so the honest fix
        // is to compute the grid against the box that actually paints it. Every
        // consumer of that 8px reads it from here, so the paint box and the grid
        // can never drift apart again.
        const terminalScrollbarGutterPx = () => Math.max(
            0,
            Number(window.__yggtermXtermScrollbarGutterPx || 8)
        );
        const hostMetrics = () => {{
            const rect = host.getBoundingClientRect();
            const computed = window.getComputedStyle(host);
            const viewportWidth = Math.max(
                0,
                Number(window.innerWidth || document.documentElement.clientWidth || 0)
            );
            const viewportHeight = Math.max(
                0,
                Number(window.innerHeight || document.documentElement.clientHeight || 0)
            );
            const visible = computed.visibility !== "hidden" && computed.display !== "none";
            return {{
                width: Math.round(rect.width || 0),
                height: Math.round(rect.height || 0),
                left: Math.round(rect.left || 0),
                top: Math.round(rect.top || 0),
                right: Math.round(rect.right || 0),
                bottom: Math.round(rect.bottom || 0),
                visible,
                onscreen: visible
                    && rect.width > 0
                    && rect.height > 0
                    && rect.right > 0
                    && rect.bottom > 0
                    && rect.left < viewportWidth
                    && rect.top < viewportHeight,
            }};
        }};
        const elementChainMetrics = (start) => {{
            const parts = [];
            let current = start;
            for (let depth = 0; current && depth < 8; depth += 1) {{
                const rect = current.getBoundingClientRect();
                const computed = window.getComputedStyle(current);
                const label =
                    current.id
                    || current.getAttribute('data-terminal-session-path')
                    || current.getAttribute('data-yggterm-main-surface')
                    || current.tagName.toLowerCase();
                parts.push(
                    `${{depth}}:${{label}}:${{Math.round(rect.width || 0)}}x${{Math.round(rect.height || 0)}}`
                    + ` display=${{computed.display}} position=${{computed.position}}`
                    + ` flex=${{computed.flexGrow}}/${{computed.flexShrink}}/${{computed.flexBasis}}`
                    + ` height=${{computed.height}} minHeight=${{computed.minHeight}}`
                    + ` overflow=${{computed.overflow}}`
                );
                current = current.parentElement;
            }}
            return parts.join(" <- ");
        }};
        const hostLooksUsable = () => {{
            const metrics = hostMetrics();
            return metrics.visible && metrics.onscreen && metrics.width >= 280 && metrics.height >= 140;
        }};
        if (document.fonts && document.fonts.ready) {{
            try {{
                await Promise.race([document.fonts.ready, sleep(80)]);
            }} catch (_error) {{}}
        }}
        for (let attempt = 0; attempt < 16; attempt += 1) {{
            if (hostLooksUsable()) {{
                break;
            }}
            await sleep(20);
        }}
        const initialMetrics = hostMetrics();
        sendTerminalEvent({{
            kind: "debug",
            message: `host_metrics host=${{hostId}} width=${{initialMetrics.width}} height=${{initialMetrics.height}} visible=${{initialMetrics.visible}}`
        }});
        sendTerminalEvent({{
            kind: "debug",
            message: `host_chain host=${{hostId}} ${{elementChainMetrics(host)}}`
        }});
        // BLANK-VIEWPORT PROVENANCE PROBE (2026-07-22). A live blank viewport was
        // root-caused to `term.element` sitting DETACHED while an empty husk
        // (`div.terminal.xterm` holding only `.xterm-viewport`, no `.xterm-screen`)
        // occupied the host. What could NOT be determined from the trace is WHICH
        // wipe/open left that husk behind — the mount emits no event between the
        // reveal-ghost attach and the (empty) first-paint samples. So every host
        // wipe and every `term.open` now leaves a synchronous breadcrumb with a
        // real stack; `syncHostAttachmentEntry` correlates the next detach against
        // it. See docs/xterm-bugs.md#detached-term-element-blank-viewport.
        window.__yggtermRecordHostMutation = window.__yggtermRecordHostMutation || function (record) {{
            try {{
                const log = window.__yggtermHostMutationLog = window.__yggtermHostMutationLog || [];
                const entry = Object.assign({{ at_ms: Date.now() }}, record || {{}});
                try {{
                    entry.stack = String((new Error()).stack || '')
                        .split('\n').slice(2, 7).join(' | ').slice(0, 600);
                }} catch (_stackError) {{}}
                log.push(entry);
                while (log.length > 64) {{
                    log.shift();
                }}
                const hostEntry = window.__yggtermXtermHosts && record && record.host_id
                    ? window.__yggtermXtermHosts[record.host_id]
                    : null;
                if (hostEntry) {{
                    hostEntry.lastHostMutation = entry;
                    hostEntry.hostMutationCount = Number(hostEntry.hostMutationCount || 0) + 1;
                }}
                return entry;
            }} catch (_error) {{
                return null;
            }}
        }};
        window.__yggtermRecordHostMutation({{
            host_id: hostId,
            site: 'mount_init_wipe',
            child_count: Number(host.childElementCount || 0),
        }});
        host.innerHTML = "";
        // BORING REVEAL ghost, attach half: same synchronous task as the wipe
        // above, so the cleared host never reaches the compositor — the ghost is
        // already covering it when the next frame paints. Released after the
        // replay + reveal screen-reconcile settle window, or immediately on the
        // user's first keystroke (their input echo must not be hidden).
        if (revealGhostFrame) {{
            try {{
                if (window.getComputedStyle(host).position === 'static') {{
                    host.style.position = 'relative';
                }}
                host.appendChild(revealGhostFrame);
                const ghostAttachedAtMs = Date.now();
                const releaseRevealGhost = () => {{
                    try {{
                        if (revealGhostFrame.isConnected) {{
                            revealGhostFrame.remove();
                            sendTerminalEvent({{
                                kind: "debug",
                                message: `reveal_ghost_released host=${{hostId}}`
                            }});
                        }}
                    }} catch (_error) {{}}
                }};
                window.setTimeout(releaseRevealGhost, 2400);
                // First keystroke releases the cover (input echo must not be
                // hidden) — UNLESS the ghost is younger than the keystroke's
                // own remount: when a keydown (prompt submit) is what caused
                // this re-dispatch, that same keydown used to rip the cover
                // off and expose the fresh construct's fit+restore for a
                // frame (user 2026-07-23: "after passing a prompt … zoomed a
                // little; blinked once and fixed"). A minimum cover age keeps
                // the transition invisible; real typing lands >250ms later.
                const releaseOnKeydown = () => {{
                    if (Date.now() - ghostAttachedAtMs < 250) {{
                        window.addEventListener('keydown', releaseOnKeydown, {{ once: true, capture: true }});
                        return;
                    }}
                    releaseRevealGhost();
                }};
                window.addEventListener('keydown', releaseOnKeydown, {{ once: true, capture: true }});
                sendTerminalEvent({{
                    kind: "debug",
                    message: `reveal_ghost_attached host=${{hostId}} w=${{revealGhostFrame.width}} h=${{revealGhostFrame.height}}`
                }});
            }} catch (_error) {{}}
        }} else if (!window.__yggtermDisableRevealGhost) {{
            // BORING REVEAL lane-3, cold-mount veil: a COLD mount has no prior
            // canvas to ghost, so the user used to watch the wrong-frame churn
            // (client-snapshot shadow → replay reset/rewrite → DOM-leak
            // flicker) behind the resume gate. Per the spec, latency beats
            // flicker and a wrong frame must never paint: cover the host with
            // a solid background-colored veil and release it only when the
            // buffer has SETTLED (nonblank content + stable baseY/cursor for
            // two consecutive polls), on the user's first keystroke (their
            // echo must never be hidden), or at the hard cap below (never trap
            // the user behind a veil).
            //
            // ⚠ This comment used to say "an 8s hard cap" while the code used
            // 20000. Nobody was misled into a bug by it, but a comment that
            // disagrees with its constant is how the next person gets misled,
            // so the number now lives in ONE place —
            // COLD_MOUNT_VEIL_HARD_CAP_MS — and this prose does not restate it.
            try {{
                if (window.getComputedStyle(host).position === 'static') {{
                    host.style.position = 'relative';
                }}
                const veil = document.createElement('div');
                veil.className = 'yggterm-cold-mount-veil';
                veil.style.position = 'absolute';
                veil.style.inset = '0';
                veil.style.zIndex = '40';
                veil.style.pointerEvents = 'none';
                veil.style.backgroundColor =
                    window.getComputedStyle(host).backgroundColor || '#000';
                host.appendChild(veil);
                const veilAttachedAtMs = Date.now();
                let veilLastBaseY = -1;
                let veilLastCursorY = -1;
                let veilStablePolls = 0;
                // VEIL ACCOUNTING (2026-07-31). Live telemetry showed 17
                // `cold_mount_veil_attached` against 11 `..._released` — six
                // veils with no disposition at all. That gap is not cosmetic:
                // a veil is an opaque rectangle over the user's terminal, and
                // "released silently when its host was torn down" and "still
                // covering the viewport right now" were indistinguishable from
                // outside. Every veil now leaves a record, and the live count
                // is readable from `server app state` so a stuck one can be
                // SEEN instead of inferred.
                try {{
                    window.__yggtermColdMountVeils = window.__yggtermColdMountVeils || {{}};
                    window.__yggtermColdMountVeils[hostId] = {{
                        attachedAtMs: veilAttachedAtMs,
                        hostId,
                        sessionPath: host.getAttribute("data-terminal-session-path") || "",
                    }};
                }} catch (_error) {{}}
                const releaseColdMountVeil = (reason) => {{
                    try {{
                        // `isConnected` false means the host was destroyed under
                        // the veil — a real disposition, previously unlogged,
                        // and the whole of the 6-of-17 gap.
                        const wasConnected = veil.isConnected;
                        if (wasConnected) {{
                            veil.remove();
                        }}
                        try {{
                            if (window.__yggtermColdMountVeils) {{
                                delete window.__yggtermColdMountVeils[hostId];
                            }}
                        }} catch (_bookkeepingError) {{}}
                        sendTerminalEvent({{
                            kind: "debug",
                            message: `cold_mount_veil_released host=${{hostId}}`
                                + ` reason=${{wasConnected ? reason : 'host_torn_down'}}`
                                + ` held_ms=${{Date.now() - veilAttachedAtMs}}`
                        }});
                    }} catch (_error) {{}}
                }};
                const veilSettlePoll = () => {{
                    try {{
                        if (!veil.isConnected) {{
                            // Was: a silent `return`. That single line WAS the
                            // 6-of-17 accounting gap — a host torn down under
                            // its veil ended the poll chain with no record, so
                            // the veil's fate became unknowable. Report it.
                            releaseColdMountVeil('host_torn_down');
                            return;
                        }}
                        if (Date.now() - veilAttachedAtMs >= {COLD_MOUNT_VEIL_HARD_CAP_MS}) {{
                            releaseColdMountVeil('hard_cap');
                            return;
                        }}
                        const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                            ? window.__yggtermXtermHosts[hostId] : null;
                        const buf = entry && entry.term && entry.term.buffer
                            ? entry.term.buffer.active : null;
                        // Live catch (charts restore, 2026-06-11): the stale
                        // client snapshot IS content and IS stable — releasing
                        // on mere stability dropped the veil onto the shadow
                        // while the remote attach was still in flight. Require
                        // DAEMON-sourced content before a settle release; the
                        // longer cap is harmless (the veil is just the
                        // background color) and the keystroke release stands.
                        const contentSource = entry ? String(entry.terminalContentSource || '') : '';
                        const replaySource = entry ? String(entry.lastRetainedReplaySource || '') : '';
                        const daemonSourced = contentSource === 'daemon_pty'
                            || replaySource.indexOf('daemon') === 0;
                        if (buf) {{
                            const baseY = Number(buf.baseY || 0);
                            const cursorY = Number(buf.cursorY || 0);
                            const hasContent = baseY > 0 || cursorY > 0 || Number(buf.cursorX || 0) > 0;
                            if (hasContent && baseY === veilLastBaseY && cursorY === veilLastCursorY) {{
                                veilStablePolls += 1;
                            }} else {{
                                veilStablePolls = 0;
                            }}
                            veilLastBaseY = baseY;
                            veilLastCursorY = cursorY;
                            if (hasContent && daemonSourced && veilStablePolls >= 2) {{
                                releaseColdMountVeil('buffer_settled');
                                return;
                            }}
                        }}
                        window.setTimeout(veilSettlePoll, 250);
                    }} catch (_error) {{
                        releaseColdMountVeil('poll_error');
                    }}
                }};
                window.setTimeout(veilSettlePoll, 400);
                window.addEventListener('keydown', () => releaseColdMountVeil('keydown'), {{ once: true, capture: true }});
                sendTerminalEvent({{
                    kind: "debug",
                    message: `cold_mount_veil_attached host=${{hostId}}`
                }});
            }} catch (_error) {{}}
        }}
        const term = new window.Terminal({{
            allowProposedApi: true,
            // User bug 6 (multiline URLs): xterm's DEFAULT OSC-8 link handler
            // shows a confirm() dialog and then calls window.open(), which is
            // a NO-OP inside the wry webview — the user saw a dead "OK"
            // dialog. Route activation to the Rust side, which opens the URL
            // with the OS browser.
            linkHandler: {{
                activate: (_event, text) => {{
                    sendTerminalEvent({{ kind: 'open_url', url: String(text || '') }});
                }},
                allowNonHttpProtocols: false,
            }},
            allowTransparency: false,
            convertEol: false,
            cursorBlink: false,
            cursorInactiveStyle: 'block',
            cursorStyle: 'block',
            customGlyphs: true,
            fontFamily: {font_family},
            fontSize: {font_size},
            fontWeight: {font_weight},
            fontWeightBold: {font_weight_bold},
            lineHeight: {line_height},
            letterSpacing: 0,
            minimumContrastRatio: {minimum_contrast_ratio},
            rightClickSelectsWord: false,
            // Per [[spec-tmux-parity-and-beyond]]: xterm.js scrollback matches the
            // daemon's `DAEMON_VT_SCROLLBACK_ROWS` so the GUI can render the full
            // retained history when the daemon replays it on attach. 10 000 rows
            // is the practical sweet spot for shells.
            scrollback: 10000,
            theme: {{
                background: {background},
                foreground: {foreground},
                cursor: {cursor},
                cursorAccent: {cursor_text},
                selectionBackground: {selection},
                black: {black},
                red: {red},
                green: {green},
                yellow: {yellow},
                blue: {blue},
                magenta: {magenta},
                cyan: {cyan},
                white: {white},
                brightBlack: {bright_black},
                brightRed: {bright_red},
                brightGreen: {bright_green},
                brightYellow: {bright_yellow},
                brightBlue: {bright_blue},
                brightMagenta: {bright_magenta},
                brightCyan: {bright_cyan},
                brightWhite: {bright_white},
            }},
        }});
        // WebGL (GPU) is the renderer for heavy terminal output — xterm.js 6
        // removed the canvas renderer, so the GPU tier is now WebGL. Keep the
        // environment gate so field tests can still force the DOM renderer while
        // isolating WebKit/Wayland WebGL-context behavior.
        const canvasRendererEnabled = {canvas_renderer_enabled};
        window.__yggtermXtermCanvasRendererEnabled = Boolean(canvasRendererEnabled);
        const rendererPolicyReason = {renderer_policy_reason};
        window.__yggtermXtermRendererPolicyReason = String(rendererPolicyReason || '');
        let webglRendererActive = false;
        // ⛔⛔ THE SHIPPED xterm SCORES EMOJI ONE CELL WIDE. Measured against the
        // exact vendored bundle 2026-08-11: `activeVersion` is `6` and `["6"]` is
        // the ONLY table registered, so `wcwidth(U+2B50 ⭐) = 1`,
        // `U+26D4 ⛔ = 1`, `U+2705 ✅ = 1`, `U+1F680 🚀 = 1`. Every modern agent
        // CLI writes them as TWO columns (Unicode 9+ made Emoji_Presentation
        // characters Wide), so from the first emoji on a line the CLI and the
        // renderer disagree about where every later cell is. A partial repaint
        // then leaves the orphaned column holding its old glyph — the owner's
        // "weird characters appearing here and there", which sat immediately
        // after ⭐ and ⛔ in both frames he sent, and cleared on scroll because a
        // full-line repaint re-lays the row out consistently.
        //
        // ⚠ CJK was never wrong (`中` = 2) and text-presentation symbols must STAY
        // narrow (`⚠ U+26A0` = 1, `✻` = 1, `❯` = 1) — widening those would create
        // the identical bug in the opposite direction. So this widens EXACTLY the
        // Emoji_Presentation=Yes set and delegates everything else to the bundle's
        // own v6 provider, rather than swapping in a whole new width table.
        //
        // `charProperties` is overridden alongside `wcwidth` on purpose: the
        // renderer reads the packed properties, so changing only `wcwidth` would
        // leave the paint on the old widths and fix nothing.
        try {{
          const uniSvc = term._core && term._core.unicodeService;
          const uniBase = uniSvc && uniSvc._providers && uniSvc._providers['6'];
          const UniSvc = uniSvc && uniSvc.constructor;
          if (uniBase && UniSvc && typeof UniSvc.createPropertyValue === 'function') {{
            // Emoji_Presentation=Yes, as [start, end] pairs, sorted — binary searched.
            const EMOJI_WIDE = [
            0x231A,0x231B,0x23E9,0x23EC,0x23F0,0x23F0,0x23F3,0x23F3,0x25FD,0x25FE,0x2614,0x2615,
            0x2648,0x2653,0x267F,0x267F,0x2693,0x2693,0x26A1,0x26A1,0x26AA,0x26AB,0x26BD,0x26BE,
            0x26C4,0x26C5,0x26CE,0x26CE,0x26D4,0x26D4,0x26EA,0x26EA,0x26F2,0x26F3,0x26F5,0x26F5,
            0x26FA,0x26FA,0x26FD,0x26FD,0x2705,0x2705,0x270A,0x270B,0x2728,0x2728,0x274C,0x274C,
            0x274E,0x274E,0x2753,0x2755,0x2757,0x2757,0x2795,0x2797,0x27B0,0x27B0,0x27BF,0x27BF,
            0x2B1B,0x2B1C,0x2B50,0x2B50,0x2B55,0x2B55,0x1F004,0x1F004,0x1F0CF,0x1F0CF,0x1F18E,
            0x1F18E,0x1F191,0x1F19A,0x1F1E6,0x1F1FF,0x1F201,0x1F201,0x1F21A,0x1F21A,0x1F22F,
            0x1F22F,0x1F232,0x1F236,0x1F238,0x1F23A,0x1F250,0x1F251,0x1F300,0x1F320,0x1F32D,
            0x1F335,0x1F337,0x1F37C,0x1F37E,0x1F393,0x1F3A0,0x1F3CA,0x1F3CF,0x1F3D3,0x1F3E0,
            0x1F3F0,0x1F3F4,0x1F3F4,0x1F3F8,0x1F43E,0x1F440,0x1F440,0x1F442,0x1F4FC,0x1F4FF,
            0x1F53D,0x1F54B,0x1F54E,0x1F550,0x1F567,0x1F57A,0x1F57A,0x1F595,0x1F596,0x1F5A4,
            0x1F5A4,0x1F5FB,0x1F64F,0x1F680,0x1F6C5,0x1F6CC,0x1F6CC,0x1F6D0,0x1F6D2,0x1F6D5,
            0x1F6D7,0x1F6DD,0x1F6DF,0x1F6EB,0x1F6EC,0x1F6F4,0x1F6FC,0x1F7E0,0x1F7EB,0x1F7F0,
            0x1F7F0,0x1F90C,0x1F93A,0x1F93C,0x1F945,0x1F947,0x1F9FF,0x1FA70,0x1FA74,0x1FA78,
            0x1FA7C,0x1FA80,0x1FA86,0x1FA90,0x1FAAC,0x1FAB0,0x1FABA,0x1FAC0,0x1FAC5,0x1FAD0,
            0x1FAD9,0x1FAE0,0x1FAE7,0x1FAF0,0x1FAF6
            ];
            const isWideEmoji = (cp) => {{
              let lo = 0, hi = (EMOJI_WIDE.length >> 1) - 1;
              while (lo <= hi) {{
                const mid = (lo + hi) >> 1;
                const a = EMOJI_WIDE[mid * 2], b = EMOJI_WIDE[mid * 2 + 1];
                if (cp < a) hi = mid - 1;
                else if (cp > b) lo = mid + 1;
                else return true;
              }}
              return false;
            }};
            term.unicode.register({{
              version: '11',
              wcwidth: (cp) => (isWideEmoji(cp) ? 2 : uniBase.wcwidth(cp)),
              charProperties: (cp, preceding) => {{
                const props = uniBase.charProperties(cp, preceding);
                return isWideEmoji(cp)
                  ? UniSvc.createPropertyValue(UniSvc.extractShouldJoin(props), 2, true)
                  : props;
              }},
            }});
            term.unicode.activeVersion = '11';
          }}
        }} catch (_error) {{}}
        const fitAddon = new window.FitAddon.FitAddon();
        term.loadAddon(fitAddon);
        // User bug 6: plain-text http(s) URLs — including URLs WRAPPED across
        // several visual rows — must be clickable. xterm.js core only
        // linkifies OSC-8 hyperlinks; with no provider a click on a long URL
        // just selected one visual row. This provider joins the full LOGICAL
        // line (walking wrapped rows) and maps each match back to a
        // multi-row buffer range, so the whole URL underlines and activates
        // as one link. Activation routes through the same open_url event as
        // the OSC-8 handler (Rust opens the OS browser).
        const terminalUrlPattern = /https?:\/\/[^\s"'`<>]+/g;
        term.registerLinkProvider({{
            provideLinks: (lineNumber, callback) => {{
                try {{
                    const buffer = term.buffer && term.buffer.active ? term.buffer.active : null;
                    if (!buffer || typeof buffer.getLine !== 'function') {{
                        callback(undefined);
                        return;
                    }}
                    // Walk up to the start of the logical (unwrapped) line.
                    let startRow = Math.max(0, Number(lineNumber || 1) - 1);
                    while (startRow > 0) {{
                        const line = buffer.getLine(startRow);
                        if (!line || !line.isWrapped) {{
                            break;
                        }}
                        startRow -= 1;
                    }}
                    const cols = Math.max(1, Number(term.cols || 1));
                    let logicalText = '';
                    let row = startRow;
                    for (;;) {{
                        const line = buffer.getLine(row);
                        if (!line) {{
                            break;
                        }}
                        // trimRight=false keeps each row at grid width so a
                        // string index maps 1:1 onto (row, col) for the ASCII
                        // characters URLs are made of.
                        logicalText += line.translateToString(false);
                        const next = buffer.getLine(row + 1);
                        if (!next || !next.isWrapped) {{
                            break;
                        }}
                        row += 1;
                    }}
                    const links = [];
                    terminalUrlPattern.lastIndex = 0;
                    let match;
                    while ((match = terminalUrlPattern.exec(logicalText)) !== null) {{
                        let url = String(match[0] || '');
                        // Trailing prose punctuation is not part of the URL.
                        url = url.replace(/[.,;:!?'")\]]+$/, '');
                        if (url.length < 10) {{
                            continue;
                        }}
                        const startIdx = match.index;
                        const endIdx = startIdx + url.length - 1;
                        links.push({{
                            text: url,
                            range: {{
                                start: {{
                                    x: (startIdx % cols) + 1,
                                    y: startRow + Math.floor(startIdx / cols) + 1,
                                }},
                                end: {{
                                    x: (endIdx % cols) + 1,
                                    y: startRow + Math.floor(endIdx / cols) + 1,
                                }},
                            }},
                            activate: (_event, text) => {{
                                sendTerminalEvent({{ kind: 'open_url', url: String(text || url) }});
                            }},
                        }});
                    }}
                    callback(links.length ? links : undefined);
                }} catch (_error) {{
                    callback(undefined);
                }}
            }},
        }});
        term.attachCustomKeyEventHandler((event) => {{
            try {{
                const active = document.activeElement;
                const target = event && event.target ? event.target : null;
                const terminalOwnsDelete = Boolean(
                    active
                    && active.classList
                    && active.classList.contains('xterm-helper-textarea')
                ) || Boolean(
                    target
                    && target.closest
                    && target.closest('[id^="yggterm-terminal-"]')
                );
                if (terminalOwnsDelete) {{
                    window.__yggtermSidebarFocusGeneration = Number(window.__yggtermSidebarFocusGeneration || 0) + 1;
                    window.__yggtermSidebarKeyboardOwner = false;
                    return true;
                }}
                if (!window.__yggtermSidebarKeyboardOwner) {{
                    return true;
                }}
                if (!event || event.type !== 'keydown' || String(event.key || '') !== 'Delete') {{
                    return true;
                }}
                const buttonId = event.shiftKey
                    ? {TREE_HARD_DELETE_BUTTON_ID:?}
                    : {TREE_DELETE_BUTTON_ID:?};
                const button = document.getElementById(buttonId);
                if (!button) {{
                    return true;
                }}
                event.preventDefault();
                event.stopPropagation();
                button.click();
                return false;
            }} catch (_error) {{
                return true;
            }}
        }});
        let suppressedTerminalProtocolResponseCount = 0;
        let lastSuppressedTerminalProtocolResponse = '';
        let lastSuppressedTerminalProtocolResponseAtMs = 0;
        const recordSuppressedTerminalProtocolResponse = (kind, data) => {{
            suppressedTerminalProtocolResponseCount += 1;
            lastSuppressedTerminalProtocolResponse =
                `${{kind}}:${{String(data || '').slice(0, 160)}}`;
            lastSuppressedTerminalProtocolResponseAtMs = Date.now();
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.suppressedTerminalProtocolResponseCount =
                        suppressedTerminalProtocolResponseCount;
                    entry.lastSuppressedTerminalProtocolResponse =
                        lastSuppressedTerminalProtocolResponse;
                    entry.lastSuppressedTerminalProtocolResponseAtMs =
                        lastSuppressedTerminalProtocolResponseAtMs;
                }}
            }} catch (_error) {{}}
        }};
        const frontendTerminalProtocolFallbackAllowed = () => {{
            try {{
                return recentFrameLikeWriteHot() || currentBufferKind() === 'alternate';
            }} catch (_error) {{
                return false;
            }}
        }};
        const terminalProtocolResponseFallbackAllowed = (data) => {{
            if (!frontendTerminalProtocolFallbackAllowed()) {{
                return false;
            }}
            return terminalDataIsSuppressedProtocolResponse(data);
        }};
        const registerTerminalProtocolResponseSuppressor = (oscCode) => {{
            try {{
                if (!term || !term.parser || typeof term.parser.registerOscHandler !== 'function') {{
                    return null;
                }}
                return term.parser.registerOscHandler(oscCode, (data) => {{
                    // Terminal palette/default-color replies are real terminal protocol
                    // traffic, not user typing. If xterm.js forwards them through onData
                    // while a shell is in cooked echo mode they become visible prompt
                    // junk. The daemon answers the default-color queries it supports;
                    // suppress the frontend fallback so the viewport remains PTY-clean.
                    if (frontendTerminalProtocolFallbackAllowed()) {{
                        recordSuppressedTerminalProtocolResponse(`osc-${{oscCode}}-fallback`, data);
                        return false;
                    }}
                    recordSuppressedTerminalProtocolResponse(`osc-${{oscCode}}`, data);
                    return true;
                }});
            }} catch (error) {{
                recordSuppressedTerminalProtocolResponse(
                    `osc-${{oscCode}}-register-error`,
                    error && error.message ? error.message : String(error)
                );
                return null;
            }}
        }};
        // ⛔ THE HUSK IS BORN HERE. Proven deterministically against the shipped
        // bundle in tools/xterm-harness/husk_is_born_in_a_partial_open.test.js.
        //
        // `Terminal.open` appends the bare `.xterm` root to the host FIRST and the
        // screen fragment LAST (read the bytes in assets/xterm/xterm.js), so ANY
        // throw in between leaves a connected, EMPTY root behind — which is the
        // live signature the autopsy calls unrepairable:
        //   `orphan_root_without_screen=true xterm_roots=1 screen_in_host=false
        //    rows_in_host=false screen_canvases=0`
        // Every DOM-placement guard reads that husk as "a terminal is present"
        // while the viewport stays blank forever. Unguarded, the same throw also
        // abandoned the REST of this mount (OSC suppressors, bell, observers).
        //
        // It is repairable, and only from here. A partial open never reaches the
        // assignment of `_coreBrowserService`, so `open()`'s early-return guard
        // (`this.element && this._coreBrowserService`) does NOT hold and a second
        // open really does rebuild the surface — but only if the husk root is
        // removed first. Leave it in place and the retry strands it as an ORPHAN
        // beside the new root, which is exactly where the orphan roots in the
        // autopsy come from (18/18 husked hosts had been constructed >= 2x).
        const terminalSurfaceIsComplete = (root) =>
            Boolean(root && root.querySelector('.xterm-screen'));
        const discardHuskTerminalRoots = (targetHost, site) => {{
            let removed = 0;
            try {{
                const roots = targetHost ? targetHost.querySelectorAll('.xterm') : [];
                for (const root of Array.from(roots)) {{
                    if (terminalSurfaceIsComplete(root)) {{
                        continue;
                    }}
                    try {{
                        root.remove();
                        removed += 1;
                    }} catch (_error) {{}}
                }}
            }} catch (_error) {{}}
            if (removed) {{
                window.__yggtermRecordHostMutation && window.__yggtermRecordHostMutation({{
                    host_id: hostId,
                    site: String(site || 'discard_husk_roots'),
                    removed_husk_roots: removed,
                }});
            }}
            return removed;
        }};
        const openTerminalSurfaceAtMount = () => {{
            try {{
                term.open(host);
            }} catch (error) {{
                return error && error.message ? String(error.message) : String(error);
            }}
            return '';
        }};
        let mountOpenError = openTerminalSurfaceAtMount();
        let mountOpenRetried = false;
        if (!terminalSurfaceIsComplete(term && term.element ? term.element : null)) {{
            discardHuskTerminalRoots(host, 'mount_partial_open_discard');
            mountOpenRetried = true;
            const retryError = openTerminalSurfaceAtMount();
            if (retryError) {{
                mountOpenError = retryError;
            }}
        }}
        window.__yggtermRecordHostMutation && window.__yggtermRecordHostMutation({{
            host_id: hostId,
            site: 'mount_term_open',
            child_count: Number(host.childElementCount || 0),
            term_element_inside_after: Boolean(term && term.element && host.contains(term.element)),
            xterm_roots_in_host: Number(host.querySelectorAll('.xterm').length || 0),
            screen_in_host: Boolean(host.querySelector('.xterm-screen')),
            open_retried: mountOpenRetried,
            open_error: mountOpenError,
        }});
        // The surface now exists and is BLANK. Everything the user eventually
        // sees is painted after this instant, which is why it is the anchor the
        // whole mount→paint chain is measured from.
        paintNoteMountOpen({{
            open_retried: mountOpenRetried,
            open_error: String(mountOpenError || ''),
            screen_in_host: Boolean(host.querySelector('.xterm-screen')),
        }});
        if (mountOpenRetried || mountOpenError) {{
            // Never silent: a mount that needed the husk repair is the ONLY place
            // that can tell us how the partial open happened in the field.
            sendTerminalEvent({{
                kind: "debug",
                message: `terminal_mount_open_incomplete host=${{hostId}}`
                    + ` retried=${{mountOpenRetried}} error=${{mountOpenError || 'none'}}`
                    + ` screen_in_host=${{Boolean(host.querySelector('.xterm-screen'))}}`
                    + ` xterm_roots=${{host.querySelectorAll('.xterm').length}}`,
            }});
        }}
        const suppressedOsc4Disposable = registerTerminalProtocolResponseSuppressor(4);
        const suppressedOsc10Disposable = registerTerminalProtocolResponseSuppressor(10);
        const suppressedOsc11Disposable = registerTerminalProtocolResponseSuppressor(11);
        // CC/Codex attention "ping": forward the terminal BEL and notification
        // OSCs (9 = iTerm, 777 = desktop-notify) to yggterm's notification
        // system. Notification OSCs are CONSUMED (return true) so they never
        // print as viewport junk; the BEL is observe-only.
        try {{
            if (typeof term.onBell === 'function') {{
                term.onBell(() => {{ sendTerminalEvent({{ kind: "notify", source: "bell" }}); }});
            }}
        }} catch (_bellError) {{}}
        const registerTerminalNotifyOsc = (oscCode, source) => {{
            try {{
                if (!term || !term.parser || typeof term.parser.registerOscHandler !== 'function') {{
                    return null;
                }}
                return term.parser.registerOscHandler(oscCode, (data) => {{
                    let title = null;
                    let body = (typeof data === 'string') ? data : null;
                    if (oscCode === 777 && typeof data === 'string') {{
                        const parts = data.split(';');
                        if (parts[0] === 'notify') {{
                            title = parts[1] || null;
                            body = parts.slice(2).join(';') || null;
                        }}
                    }}
                    sendTerminalEvent({{ kind: "notify", source: source, title: title, body: body }});
                    return true;
                }});
            }} catch (_oscError) {{
                return null;
            }}
        }};
        const notifyOsc9Disposable = registerTerminalNotifyOsc(9, "osc9");
        const notifyOsc777Disposable = registerTerminalNotifyOsc(777, "osc777");
        // libyggterm web-surface control (ychrome pilot): OSC 7717 with payload
        // `web-surface;<action>;<base64 json>`. The PTY byte relay is the
        // transport (works identically for local and remote sessions); the OSC
        // is CONSUMED so it never prints as viewport junk, and plain terminals
        // ignore unknown OSCs — the degradation story is built into the
        // channel. Scrollback replay re-parses these, which is self-correcting:
        // an open followed by its close replays in order, and the Rust side
        // expires surfaces whose heartbeats stop.
        const webSurfaceOscDisposable = (() => {{
            try {{
                if (!term || !term.parser || typeof term.parser.registerOscHandler !== 'function') {{
                    return null;
                }}
                return term.parser.registerOscHandler(7717, (data) => {{
                    try {{
                        const raw = typeof data === 'string' ? data : '';
                        const parts = raw.split(';');
                        const verb = parts[0];
                        if (verb !== 'web-surface' && verb !== 'sidebar' && verb !== 'fido2') {{
                            return true;
                        }}
                        const action = parts[1] || '';
                        let payload = {{}};
                        if (parts[2]) {{
                            const bytes = Uint8Array.from(atob(parts[2]), (ch) => ch.charCodeAt(0));
                            payload = JSON.parse(new TextDecoder().decode(bytes));
                        }}
                        if (!action || typeof payload.session !== 'string' || !payload.session) {{
                            return true;
                        }}
                        if (verb === 'sidebar') {{
                            // The declaration carries only what the rail draws:
                            // a control endpoint, pane buttons, and a stamp over
                            // the app's web-content policy. Never a schema (the
                            // GUI GETs that), never a ruleset, never a secret.
                            //
                            // This object is built field by field, so anything a
                            // new app declares must be copied ACROSS here too — a
                            // field added only to the Rust wire type arrives null.
                            const panes = Array.isArray(payload.panes) ? payload.panes : [];
                            sendTerminalEvent({{
                                kind: 'sidebar_contribution',
                                action,
                                session: payload.session,
                                control: typeof payload.control === 'string' ? payload.control : null,
                                policy_version: typeof payload.policy_version === 'string'
                                    ? payload.policy_version
                                    : null,
                                app_name: typeof payload.app_name === 'string'
                                    ? payload.app_name
                                    : null,
                                zoom_version: typeof payload.zoom_version === 'string'
                                    ? payload.zoom_version
                                    : null,
                                appearance_version: typeof payload.appearance_version === 'string'
                                    ? payload.appearance_version
                                    : null,
                                document_version: typeof payload.document_version === 'string'
                                    ? payload.document_version
                                    : null,
                                env_id: typeof payload.env_id === 'string'
                                    ? payload.env_id
                                    : null,
                                // THE ONE SECRET A DECLARE CARRIES, and the one
                                // field this forwarder forgot for three days: the
                                // Rust wire type grew `control_token` on
                                // 2026-07-28 and this object was not updated, so
                                // every LIVE declare arrived tokenless and every
                                // GUI-only route answered 403 — while the daemon,
                                // which parses the same OSC in Rust, held the
                                // token perfectly. It must be forwarded and NEVER
                                // traced; `the_js_forwarder_copies_every_sidebar
                                // _declare_field` now fails if a wire field is
                                // added without a line here.
                                control_token: typeof payload.control_token === 'string'
                                    ? payload.control_token
                                    : null,
                                panes: panes
                                    .filter((pane) => pane && typeof pane.id === 'string' && pane.id)
                                    .map((pane) => ({{
                                        id: pane.id,
                                        icon: typeof pane.icon === 'string' ? pane.icon : '',
                                        title: typeof pane.title === 'string' ? pane.title : '',
                                        placement: typeof pane.placement === 'string' ? pane.placement : null,
                                    }})),
                            }});
                            return true;
                        }}
                        if (verb === 'fido2') {{
                            // A WebAuthn ceremony wants the user's presence. The
                            // app names only the rpId + a display label — never a
                            // challenge, never a key. The GUI shows a dialog and,
                            // on approval, POSTs the grant back to the app's
                            // control endpoint (the one the sidebar declared on
                            // this same stream).
                            const fido2Accounts = Array.isArray(payload.accounts) ? payload.accounts : [];
                            sendTerminalEvent({{
                                kind: 'fido2_request',
                                action,
                                session: payload.session,
                                request_id: typeof payload.request_id === 'string' ? payload.request_id : '',
                                rp_id: typeof payload.rp_id === 'string' ? payload.rp_id : '',
                                account: typeof payload.account === 'string' ? payload.account : '',
                                accounts: fido2Accounts
                                    .filter((a) => a && typeof a.label === 'string')
                                    .map((a) => ({{
                                        credential_id: typeof a.credential_id === 'string' ? a.credential_id : '',
                                        label: a.label,
                                    }})),
                                ceremony: typeof payload.kind === 'string' ? payload.kind : 'get',
                                origin: typeof payload.origin === 'string' ? payload.origin : '',
                            }});
                            return true;
                        }}
                        sendTerminalEvent({{
                            kind: 'web_surface',
                            action,
                            session: payload.session,
                            url: typeof payload.url === 'string' ? payload.url : null,
                            title: typeof payload.title === 'string' ? payload.title : null,
                            profile: typeof payload.profile === 'string' ? payload.profile : null,
                            // A field the app declared and this forwarder drops is
                            // a field that never existed. Forward every one.
                            start_page: payload.start_page === true,
                        }});
                    }} catch (_webSurfaceError) {{}}
                    return true;
                }});
            }} catch (_error) {{
                return null;
            }}
        }})();
        // User bug 2: OSC 52 clipboard writes from the CLI (Claude Code's
        // select-copy / `c`-copy, tmux yank, etc.) were silently dropped —
        // xterm.js core has no OSC 52 handler (that lives in the
        // non-vendored clipboard addon), so CC's copy never reached the
        // system clipboard. Decode `Pc;<base64>` and route the text through
        // the SAME clipboard event the selection copy uses. Queries (`?`)
        // ask the terminal to REPLY with clipboard contents — consumed and
        // ignored (never leak the clipboard back to the PTY).
        // THE one door that arms an OSC 52 replay-suppression window, keyed by the
        // host whose stream is being replayed. Installed on `window` because the
        // arming sites live in three different generated scripts (mount, replay,
        // write bridge) while the handler that reads it is registered by the mount
        // script — but the WINDOW is not the unit of suppression and never was.
        // Arming globally meant a session catching up on its scrollback silently
        // ate a copy the user had just made in a different terminal.
        window.__yggtermArmOsc52Suppress = (armHostId, windowMs) => {{
            try {{
                const armKey = String(armHostId || '');
                if (!armKey) {{ return; }}
                window.__yggtermOsc52Suppress = window.__yggtermOsc52Suppress || {{}};
                window.__yggtermOsc52Suppress[armKey] = Date.now() + windowMs;
            }} catch (_osc52ArmError) {{}}
        }};
        const osc52ClipboardDisposable = (() => {{
            try {{
                if (!term || !term.parser || typeof term.parser.registerOscHandler !== 'function') {{
                    return null;
                }}
                return term.parser.registerOscHandler(52, (data) => {{
                    try {{
                        const raw = typeof data === 'string' ? data : '';
                        const sep = raw.indexOf(';');
                        const payload = sep >= 0 ? raw.slice(sep + 1) : '';
                        if (!payload || payload === '?') {{
                            return true;
                        }}
                        const bytes = Uint8Array.from(atob(payload), (ch) => ch.charCodeAt(0));
                        const text = new TextDecoder().decode(bytes);
                        // OSC 52 copy hygiene (finding-osc52-copy-chime-replay-refire):
                        // 1) REPLAY SUPPRESSION — switching into a session re-writes its
                        //    buffered scrollback (which may contain a prior OSC 52) through
                        //    THIS same parser. That re-parse must NOT re-copy + re-chime and
                        //    clobber whatever the user just copied in another buffer (the
                        //    "impossible to copy buffer-to-buffer" bug). The replay/restore
                        //    writes arm this host's window via __yggtermArmOsc52Suppress first.
                        // 2) c+p DEDUPE — CC select-copy emits OSC 52 to both the clipboard
                        //    and primary selections (two sequences); ring + write ONCE.
                        // Window/global state survives scope boundaries across the mount script.
                        const osc52NowMs = Date.now();
                        // 3) USER-GESTURE GATE (the load-bearing discriminator): a GENUINE
                        //    copy emits its OSC 52 right after a user gesture ON this terminal —
                        //    a mouse-release (select-copy) OR a keystroke (CC's `c`-to-copy on
                        //    the login screen, tmux yank, etc.). A re-emit on switch-IN — CC
                        //    re-sending its active selection on focus, OR the daemon catch-up
                        //    replaying a buffered OSC 52 — has NO such gesture (the user switched
                        //    via the sidebar, not the terminal). Without a recent pointer-up OR
                        //    keydown on THIS host, treat the OSC 52 as a re-emit and suppress it.
                        //    This fixes the shell->CC clobber on switch that the bulk/replay arms
                        //    missed (the re-emit arrives as a small live chunk, not a bulk replay)
                        //    WITHOUT dropping keyboard-initiated copies (finding-osc52: the
                        //    pointer-only stamp silently ate the FIRST `c`-copy on every login).
                        // ⛔ ALL THREE GATES ARE PER-HOST. They were window-globals, so a
                        //    replay in ANY terminal ate a genuine copy in EVERY other one and
                        //    the dedupe compared text across unrelated sessions — cross-talk
                        //    that grows with the session count and reads to the user as
                        //    "copying works sometimes".
                        const osc52Host = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                            ? window.__yggtermXtermHosts[hostId] : null;
                        const osc52GestureAtMs = osc52Host ? Number(osc52Host.lastUserGestureAtMs || 0) : 0;
                        // A dropped copy used to be SILENT, and that is why "select-copy is
                        // inconsistent" had no diagnosis: three gates could eat it and none of
                        // them left a mark, so a gate that fired was indistinguishable from a
                        // CLI that never emitted. Every drop now names itself and carries the
                        // gesture age the gate judged it on.
                        const suppressOsc52 = (reason) => {{
                            try {{
                                sendTerminalEvent({{
                                    kind: 'clipboard_suppressed',
                                    action: 'osc52',
                                    reason,
                                    chars: text.length,
                                    gesture_age_ms: osc52GestureAtMs > 0
                                        ? Math.round(osc52NowMs - osc52GestureAtMs)
                                        : -1,
                                }});
                            }} catch (_osc52SuppressReportError) {{}}
                            return true;
                        }};
                        if (osc52NowMs - osc52GestureAtMs > 3000) {{
                            return suppressOsc52('no_user_gesture');
                        }}
                        const osc52SuppressUntilMs = window.__yggtermOsc52Suppress
                            ? Number(window.__yggtermOsc52Suppress[hostId] || 0) : 0;
                        if (osc52NowMs < osc52SuppressUntilMs) {{
                            return suppressOsc52('replay_window');
                        }}
                        window.__yggtermOsc52LastCopy = window.__yggtermOsc52LastCopy || {{}};
                        const osc52LastCopy = window.__yggtermOsc52LastCopy[hostId] || null;
                        if (osc52LastCopy && text === osc52LastCopy.text
                            && (osc52NowMs - Number(osc52LastCopy.atMs || 0)) < 1200) {{
                            osc52LastCopy.atMs = osc52NowMs;
                            return suppressOsc52('duplicate_c_and_p');
                        }}
                        window.__yggtermOsc52LastCopy[hostId] = {{ text, atMs: osc52NowMs }};
                        if (text.length > 0) {{
                            sendTerminalEvent({{
                                kind: 'clipboard',
                                action: 'osc52',
                                chars: text.length,
                                text,
                            }});
                        }}
                    }} catch (error) {{
                        sendTerminalEvent({{
                            kind: 'clipboard_error',
                            action: 'osc52',
                            message: error && error.message ? error.message : String(error),
                        }});
                    }}
                    return true;
                }});
            }} catch (_error) {{
                return null;
            }}
        }})();
        // Stamp the last user gesture on this terminal host (capture phase, so it fires even
        // while the CLI holds mouse-reporting / keyboard mode and stops propagation). The OSC
        // 52 handler's gesture gate above uses it to tell a genuine copy from a re-emit on
        // switch-in. BOTH a mouse-release (select-copy) AND a keydown (CC's `c`-to-copy on the
        // login screen, tmux yank) are genuine copy gestures — stamping pointer events ALONE
        // silently dropped the first keyboard copy every time (finding-osc52-copy-chime-replay-
        // refire). A switch-in re-emit still has neither gesture (the user clicked the sidebar,
        // not this terminal), so it stays suppressed.
        try {{
            const stampOsc52Gesture = () => {{
                const gestureEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (gestureEntry) {{ gestureEntry.lastUserGestureAtMs = Date.now(); }}
            }};
            // The PRESS is as much a copy gesture as the release, and it is the
            // only half guaranteed to land on this terminal: a drag that starts
            // here can end anywhere.
            let osc52DragStartedHere = false;
            const startOsc52Drag = () => {{
                osc52DragStartedHere = true;
                stampOsc52Gesture();
            }};
            host.addEventListener('pointerdown', startOsc52Drag, true);
            host.addEventListener('mousedown', startOsc52Drag, true);
            host.addEventListener('pointerup', stampOsc52Gesture, true);
            host.addEventListener('mouseup', stampOsc52Gesture, true);
            host.addEventListener('keydown', stampOsc52Gesture, true);
            // ⛔ A SELECTION DRAG OFTEN ENDS OUTSIDE THE TERMINAL — sweep past the
            // bottom edge to take the last line, past the right edge to take the
            // end of a wrapped one — and the release then fires on the DOCUMENT,
            // never on the host. With the stamp bound to the host alone that copy
            // reached the gesture gate with nothing stamped, looked exactly like a
            // switch-in re-emit, and was dropped. Which release the gate saw
            // depended on where the mouse came up: the copy worked or vanished
            // with no visible difference to the user. That is the "it copies
            // sometimes" report. Claim the release only when the press was ours,
            // so another surface's drag never counts as a gesture on this one.
            const releaseOsc52Drag = () => {{
                if (!osc52DragStartedHere) {{ return; }}
                osc52DragStartedHere = false;
                stampOsc52Gesture();
            }};
            window.addEventListener('pointerup', releaseOsc52Drag, true);
            window.addEventListener('mouseup', releaseOsc52Drag, true);
            window.addEventListener('pointercancel', releaseOsc52Drag, true);
        }} catch (_osc52GestureError) {{}}
        let webglAddonAvailable = Boolean(window.WebglAddon && window.WebglAddon.WebglAddon);
        let rendererDecisionError = '';
        let webglAddon = null;
        try {{
            if (canvasRendererEnabled && webglAddonAvailable) {{
                // preserveDrawingBuffer=true keeps the WebGL canvas readable via
                // toDataURL/drawImage — the in-process faithful screenshot
                // (capture_backend=xterm_canvas_composite) and agent verification
                // depend on it; without it a WebGL canvas reads back blank.
                webglAddon = new window.WebglAddon.WebglAddon(true);
                // WebGL contexts are lossy under WebKitGTK/Wayland compositing; a
                // lost context would blank the terminal. Dispose the addon on loss
                // so xterm.js reverts to its DOM renderer (buffer intact) rather
                // than painting a blank canvas. This is the safety net for the one
                // real risk of moving off the canvas renderer.
                try {{
                    if (typeof webglAddon.onContextLoss === 'function') {{
                        webglAddon.onContextLoss(() => {{
                            try {{ webglAddon.dispose(); }} catch (_disposeError) {{}}
                            webglRendererActive = false;
                            sendTerminalEvent({{
                                kind: "debug",
                                message: `webgl_context_lost host=${{hostId}} -> dom_fallback`
                            }});
                        }});
                    }}
                }} catch (_lossHookError) {{}}
                term.loadAddon(webglAddon);
                webglRendererActive = true;
            }}
        }} catch (error) {{
            rendererDecisionError = error && error.message ? error.message : String(error);
            try {{ if (webglAddon) {{ webglAddon.dispose(); }} }} catch (_disposeError) {{}}
            webglRendererActive = false;
            sendTerminalEvent({{
                kind: "debug",
                message: `webgl_addon_failed host=${{hostId}} error=${{rendererDecisionError}}`
            }});
        }}
        // RENDERER TELEMETRY: emit the definitive render-pathway decision so we can
        // confirm, per platform, which xterm.js tier is actually in use and WHY.
        // `requested` = policy enabled canvas; `loaded` = the canvas addon attached;
        // a verify pass after first paint records the ACTUAL renderer (canvas leaves
        // <canvas> elements, DOM does not). Tiers: dom (slowest) < canvas (bundled) <
        // webgl (fastest, not bundled — see terminal_xterm_renderer_policy_reason).
        const emitRendererDecision = (phase) => {{
            try {{
                const canvasCount = host.querySelectorAll('canvas').length;
                sendTerminalEvent({{
                    kind: "debug",
                    message: `renderer_decision host=${{hostId}} phase=${{phase}} reason=${{rendererPolicyReason}} requested=${{canvasRendererEnabled ? 1 : 0}} addon_available=${{webglAddonAvailable ? 1 : 0}} webgl_loaded=${{webglRendererActive ? 1 : 0}} actual=${{canvasCount > 0 ? 'gpu_canvas' : 'dom'}} canvas_elements=${{canvasCount}}${{rendererDecisionError ? ' error=' + rendererDecisionError : ''}}`
                }});
            }} catch (_rendererDecisionTraceError) {{}}
        }};
        emitRendererDecision('init');
        try {{ window.requestAnimationFrame(() => window.requestAnimationFrame(() => emitRendererDecision('after_paint'))); }} catch (_rafError) {{}}
        const terminalHostContentMetrics = () => {{
            try {{
                const rect = host.getBoundingClientRect();
                const style = window.getComputedStyle(host);
                const paddingLeft = Number.parseFloat(style.paddingLeft || '0') || 0;
                const paddingRight = Number.parseFloat(style.paddingRight || '0') || 0;
                const paddingTop = Number.parseFloat(style.paddingTop || '0') || 0;
                const paddingBottom = Number.parseFloat(style.paddingBottom || '0') || 0;
                return {{
                    width: Math.max(0, Number(rect.width || 0) - paddingLeft - paddingRight),
                    height: Math.max(0, Number(rect.height || 0) - paddingTop - paddingBottom),
                    padding_top: paddingTop,
                    padding_bottom: paddingBottom,
                }};
            }} catch (_error) {{
                const metrics = hostMetrics();
                return {{ width: metrics.width, height: metrics.height, padding_top: 0, padding_bottom: 0 }};
            }}
        }};
        const terminalCssCellHeight = () => {{
            try {{
                const rowsLayer = host.querySelector('.xterm-rows');
                const firstRow = rowsLayer ? rowsLayer.querySelector('div') : null;
                const rowRect = firstRow ? firstRow.getBoundingClientRect() : null;
                const measuredRowHeight = rowRect ? Number(rowRect.height || 0) : 0;
                if (
                    Number.isFinite(measuredRowHeight)
                    && measuredRowHeight >= 6
                    && measuredRowHeight <= 80
                ) {{
                    return measuredRowHeight;
                }}
            }} catch (_error) {{}}
            try {{
                const core = term && term._core ? term._core : null;
                const renderService = core
                    ? (core._renderService || core.renderService || null)
                    : null;
                const dimensions = renderService && renderService.dimensions
                    ? renderService.dimensions
                    : null;
                const cssCanvas = dimensions && dimensions.css && dimensions.css.canvas
                    ? dimensions.css.canvas
                    : null;
                const currentRows = term ? Number(term.rows || 0) : 0;
                const measured = cssCanvas && currentRows > 0
                    ? Number(cssCanvas.height || 0) / currentRows
                    : 0;
                if (Number.isFinite(measured) && measured >= 6 && measured <= 80) {{
                    return measured;
                }}
            }} catch (_error) {{}}
            try {{
                const core = term && term._core ? term._core : null;
                const renderService = core
                    ? (core._renderService || core.renderService || null)
                    : null;
                const dimensions = renderService && renderService.dimensions
                    ? renderService.dimensions
                    : null;
                const cssCell = dimensions && dimensions.css && dimensions.css.cell
                    ? dimensions.css.cell
                    : null;
                const measured = cssCell ? Number(cssCell.height || 0) : 0;
                if (Number.isFinite(measured) && measured >= 6 && measured <= 80) {{
                    return measured;
                }}
            }} catch (_error) {{}}
            try {{
                const fontSize = term && term.options ? Number(term.options.fontSize || 0) : 0;
                const lineHeight = term && term.options ? Number(term.options.lineHeight || 1) : 1;
                const fallback = fontSize * lineHeight;
                if (Number.isFinite(fallback) && fallback >= 6 && fallback <= 80) {{
                    return fallback;
                }}
            }} catch (_error) {{}}
            return 18;
        }};
        const terminalCssCellWidth = () => {{
            try {{
                const core = term && term._core ? term._core : null;
                const renderService = core
                    ? (core._renderService || core.renderService || null)
                    : null;
                const dimensions = renderService && renderService.dimensions
                    ? renderService.dimensions
                    : null;
                const cssCanvas = dimensions && dimensions.css && dimensions.css.canvas
                    ? dimensions.css.canvas
                    : null;
                const currentCols = term ? Number(term.cols || 0) : 0;
                const measured = cssCanvas && currentCols > 0
                    ? Number(cssCanvas.width || 0) / currentCols
                    : 0;
                if (Number.isFinite(measured) && measured >= 3 && measured <= 80) {{
                    return measured;
                }}
            }} catch (_error) {{}}
            try {{
                const rowsLayer = host.querySelector('.xterm-rows');
                const rowRect = rowsLayer ? rowsLayer.getBoundingClientRect() : null;
                const currentCols = term ? Number(term.cols || 0) : 0;
                const measured = rowRect && currentCols > 0
                    ? Number(rowRect.width || 0) / currentCols
                    : 0;
                if (Number.isFinite(measured) && measured >= 3 && measured <= 80) {{
                    return measured;
                }}
            }} catch (_error) {{}}
            try {{
                const core = term && term._core ? term._core : null;
                const renderService = core
                    ? (core._renderService || core.renderService || null)
                    : null;
                const dimensions = renderService && renderService.dimensions
                    ? renderService.dimensions
                    : null;
                const cssCell = dimensions && dimensions.css && dimensions.css.cell
                    ? dimensions.css.cell
                    : null;
                const measured = cssCell ? Number(cssCell.width || 0) : 0;
                if (Number.isFinite(measured) && measured >= 3 && measured <= 80) {{
                    return measured;
                }}
            }} catch (_error) {{}}
            return 8;
        }};
        // A READ-ONLY viewer (slice-4 shadow) may not resize the PTY, so it must
        // adopt the daemon's grid instead of proposing its own: the viewer
        // adapts to the session. `window.__yggtermShadowPinnedGrid` is set by
        // Rust from the session's "PTY size" at mount; absent on the user's own
        // GUI, which owns the PTY and keeps fitting to its window.
        const shadowPinnedGrid = () => {{
            const pinned = window.__yggtermShadowPinnedGrid;
            if (!pinned) {{
                return null;
            }}
            const cols = Number(pinned.cols || 0);
            const rows = Number(pinned.rows || 0);
            return terminalGridIsUsable(cols, rows) ? {{ cols, rows }} : null;
        }};
        const proposedTerminalFitDimensions = () => {{
            const content = terminalHostContentMetrics();
            const cellWidth = Math.max(1, terminalCssCellWidth());
            const cellHeight = Math.max(1, terminalCssCellHeight());
            const bottomGuardPx = Math.max(0, Number(window.__yggtermXtermFitBottomGuardPx || 2));
            // XTERM-BUG: right-edge-glyph-clipped — the grid must be proposed
            // against the width `.xterm-screen` actually paints into, which is
            // the host MINUS the reserved scrollbar gutter. Dividing the full
            // host width here handed out a column that lands under the clip.
            const rightGutterPx = terminalScrollbarGutterPx();
            const availableWidth = Math.max(0, Number(content.width || 0) - rightGutterPx);
            const availableHeight = Math.max(0, Number(content.height || 0) - bottomGuardPx);
            const pinned = shadowPinnedGrid();
            if (pinned) {{
                return {{
                    cols: pinned.cols,
                    rows: pinned.rows,
                    pinned: true,
                    available_width_px: Number(availableWidth.toFixed(2)),
                    available_height_px: Number(availableHeight.toFixed(2)),
                    right_gutter_px: rightGutterPx,
                    cell_width_px: Number(cellWidth.toFixed(3)),
                    cell_height_px: Number(cellHeight.toFixed(3)),
                }};
            }}
            return {{
                cols: Math.max(2, Math.floor(availableWidth / cellWidth)),
                rows: Math.max(1, Math.floor(availableHeight / cellHeight)),
                available_width_px: Number(availableWidth.toFixed(2)),
                available_height_px: Number(availableHeight.toFixed(2)),
                right_gutter_px: rightGutterPx,
                cell_width_px: Number(cellWidth.toFixed(3)),
                cell_height_px: Number(cellHeight.toFixed(3)),
            }};
        }};
        const terminalGridIsUsable = (cols, rows) => {{
            const safeCols = Number(cols || 0);
            const safeRows = Number(rows || 0);
            return Number.isFinite(safeCols)
                && Number.isFinite(safeRows)
                && safeCols >= 20
                && safeRows >= 4;
        }};
        const recordSkippedFit = (reason, proposed, cause) => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return;
                }}
                const previous = entry.lastSkippedFit || {{}};
                const count = Number(previous.count || 0) + 1;
                entry.lastSkippedFit = {{
                    reason,
                    cause,
                    count,
                    cols: proposed ? proposed.cols : null,
                    rows: proposed ? proposed.rows : null,
                    current_cols: term ? term.cols : null,
                    current_rows: term ? term.rows : null,
                    available_width_px: proposed ? proposed.available_width_px : null,
                    available_height_px: proposed ? proposed.available_height_px : null,
                    at_ms: Date.now(),
                }};
            }} catch (_error) {{}}
        }};
        const clearSkippedFit = (reason, proposed) => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry || !entry.lastSkippedFit) {{
                    return;
                }}
                entry.lastRecoveredFit = {{
                    reason,
                    cols: proposed ? proposed.cols : (term ? term.cols : null),
                    rows: proposed ? proposed.rows : (term ? term.rows : null),
                    available_width_px: proposed ? proposed.available_width_px : null,
                    available_height_px: proposed ? proposed.available_height_px : null,
                    at_ms: Date.now(),
                }};
                entry.lastSkippedFit = null;
            }} catch (_error) {{}}
        }};
        const resizeMutationAllowed = (reason, proposed, cause = 'unfocused_resize_observer') => {{
            const reasonText = String(reason || '');
            if (lastResizeKey === '' || reasonText !== 'resize') {{
                return true;
            }}
            const proposedCols = proposed ? Number(proposed.cols || 0) : Number(term.cols || 0);
            const proposedRows = proposed ? Number(proposed.rows || 0) : Number(term.rows || 0);
            const changesGrid = proposedCols !== Number(term.cols || 0)
                || proposedRows !== Number(term.rows || 0);
            if (!changesGrid) {{
                return true;
            }}
            // A visible, usable host must be allowed to re-fit its grid even when the
            // OS reports the window as unfocused. On KDE/Wayland document.hasFocus()
            // returns false for a perfectly visible FOREGROUND window, and the old
            // focus gate (windowFocused && documentFocused) then froze the grid at a
            // stale width — the "squished viewport" (codex TUI wrapping at ~10% of the
            // host width). hostLooksUsable() already excludes hidden / off-screen /
            // too-small hosts, and the explicit app-control-backgrounded flag covers
            // deliberate backgrounding, so visibility is the correct gate — not focus.
            const appControlBackgrounded =
                host.getAttribute('data-terminal-app-control-backgrounded') === 'true';
            if (!appControlBackgrounded && hostLooksUsable()) {{
                return true;
            }}
            recordSkippedFit(reasonText, proposed, cause);
            return false;
        }};
        const fitTerminalToHost = (reason) => {{
            try {{
                const proposed = proposedTerminalFitDimensions();
                if (!proposed || proposed.cols <= 0 || proposed.rows <= 0) {{
                    return false;
                }}
                const currentGridUsable = terminalGridIsUsable(term.cols, term.rows);
                if (!terminalGridIsUsable(proposed.cols, proposed.rows) && currentGridUsable) {{
                    recordSkippedFit(reason, proposed, 'proposed_grid_unusable');
                    return false;
                }}
                if (!hostLooksUsable() && currentGridUsable) {{
                    recordSkippedFit(reason, proposed, 'host_not_usable');
                    return false;
                }}
                if (term.cols === proposed.cols && term.rows === proposed.rows) {{
                    if (hostLooksUsable() && terminalGridIsUsable(proposed.cols, proposed.rows)) {{
                        clearSkippedFit(`${{reason}}:already_fit`, proposed);
                    }}
                    return false;
                }}
                const previousCols = term.cols;
                const previousRows = term.rows;
                if (!resizeMutationAllowed(reason, proposed)) {{
                    return false;
                }}
                const shouldPromptFollowAfterFit = scrollbackIntent !== 'UserScrollback';
                if (shouldPromptFollowAfterFit) {{
                    armPromptFollowLayoutGuard(`fit:${{reason}}`, 720);
                }}
                term.resize(proposed.cols, proposed.rows);
                clearSkippedFit(reason, proposed);
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].lastExplicitFit = {{
                        reason,
                        cols: proposed.cols,
                        rows: proposed.rows,
                        available_width_px: proposed.available_width_px,
                        available_height_px: proposed.available_height_px,
                        cell_width_px: proposed.cell_width_px,
                        cell_height_px: proposed.cell_height_px,
                        at_ms: Date.now(),
                    }};
                }}
                if (shouldPromptFollowAfterFit) {{
                    schedulePromptFollowAfterLayout(`fit:${{reason}}`);
                }}
                emitPerf("xterm_fit", {{
                    reason,
                    previous_cols: previousCols,
                    previous_rows: previousRows,
                    proposed_cols: proposed.cols,
                    proposed_rows: proposed.rows,
                    available_width_px: proposed.available_width_px,
                    available_height_px: proposed.available_height_px,
                }});
                return true;
            }} catch (_error) {{
                try {{
                    if (!terminalGridIsUsable(term.cols, term.rows)) {{
                        fitAddon.fit();
                    }} else {{
                        recordSkippedFit(reason, null, 'fit_exception_current_grid_usable');
                    }}
                }} catch (_error2) {{}}
                return false;
            }}
        }};
        const terminalFitDiagnostics = () => {{
            const content = terminalHostContentMetrics();
            const rows = Math.max(0, Number(term && term.rows ? term.rows : 0));
            const cols = Math.max(0, Number(term && term.cols ? term.cols : 0));
            const cellHeight = terminalCssCellHeight();
            const rasterCellHeight = Math.max(1, Math.ceil(cellHeight));
            const bottomGuardPx = 2;
            const availableHeight = Math.max(0, Number(content.height || 0) - bottomGuardPx);
            const requiredHeight = rows * rasterCellHeight;
            return {{
                cols,
                rows,
                cell_height_px: Number(cellHeight.toFixed(3)),
                raster_cell_height_px: rasterCellHeight,
                available_height_px: Number(availableHeight.toFixed(2)),
                required_height_px: Number(requiredHeight.toFixed(2)),
                overflow_px: Number(Math.max(0, requiredHeight - availableHeight).toFixed(2)),
                bottom_guard_px: bottomGuardPx,
            }};
        }};
        const applyTerminalRowFitGuard = (reason) => {{
            try {{
                // The guard trims a row when the grid overflows the container.
                // On a PINNED (read-only viewer) grid that is exactly wrong: the
                // grid is the daemon's truth and the container is just a window
                // onto it, so trimming would re-introduce the divergence the pin
                // exists to remove. Let it clip instead — a short window over a
                // correct grid stays faithful for the rows it does show; a
                // resized grid is wrong everywhere.
                if (shadowPinnedGrid()) {{
                    return false;
                }}
                const diagnostics = terminalFitDiagnostics();
                if (
                    diagnostics.rows <= 1
                    || diagnostics.cols <= 0
                    || diagnostics.available_height_px < 120
                    || diagnostics.overflow_px <= 0
                ) {{
                    return false;
                }}
                const safeRows = Math.max(
                    1,
                    Math.min(
                        diagnostics.rows - 1,
                        Math.floor(diagnostics.available_height_px / diagnostics.raster_cell_height_px)
                    )
                );
                if (safeRows >= diagnostics.rows) {{
                    return false;
                }}
                if (!resizeMutationAllowed(reason, {{
                    cols: diagnostics.cols,
                    rows: safeRows,
                    available_width_px: null,
                    available_height_px: diagnostics.available_height_px,
                }}, 'unfocused_row_fit_guard')) {{
                    return false;
                }}
                const shouldPromptFollowAfterFitGuard = scrollbackIntent !== 'UserScrollback';
                if (shouldPromptFollowAfterFitGuard) {{
                    armPromptFollowLayoutGuard(`row_fit_guard:${{reason}}`, 720);
                }}
                term.resize(diagnostics.cols, safeRows);
                clearSkippedFit(reason, {{
                    cols: diagnostics.cols,
                    rows: safeRows,
                    available_width_px: null,
                    available_height_px: diagnostics.available_height_px,
                }});
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].lastFitGuard = {{
                        reason,
                        before_rows: diagnostics.rows,
                        after_rows: safeRows,
                        overflow_px: diagnostics.overflow_px,
                        available_height_px: diagnostics.available_height_px,
                        raster_cell_height_px: diagnostics.raster_cell_height_px,
                        at_ms: Date.now(),
                    }};
                }}
                sendTerminalEvent({{
                    kind: "debug",
                    message: `row_fit_guard host=${{hostId}} reason=${{reason}} rows=${{diagnostics.rows}}->${{safeRows}} overflow=${{diagnostics.overflow_px}} cell=${{diagnostics.raster_cell_height_px}} avail=${{diagnostics.available_height_px}}`
                }});
                if (shouldPromptFollowAfterFitGuard) {{
                    schedulePromptFollowAfterLayout(`row_fit_guard:${{reason}}`);
                }}
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        let resizeObserver = null;
        let attachHostInteractions = (_targetHost) => {{}};
        let detachHostInteractions = (_targetHost) => {{}};
        let handleTerminalContextMenu = (_event) => {{}};
        let handleTerminalSecondaryButton = (_event) => false;
        // ── SINGLE LIVE OWNER PER HOST (2026-07-23, user-reported render
        // storm). A click-driven re-open can re-dispatch this whole script for
        // a hostId whose previous closure is still alive: two closures then
        // FIGHT for the host — each one's repair sees the other's element and
        // evicts it (measured live: ONE click → 560 childList mutations in 3s,
        // two roots alternating at 25-50ms, UI lag + fans — the user's "render
        // storm"). The registry write below is last-writer-wins, so the entry's
        // ownerToken names the ONE legitimate owner; a closure that finds a
        // NEWER token must stand down COMPLETELY — no repairs, no observers,
        // no redraws — instead of competing. Registration flips ownRegistered,
        // so the pre-registration window never misreads the predecessor's
        // token as "we were superseded".
        window.__yggtermXtermOwnerTokens = (window.__yggtermXtermOwnerTokens || 0) + 1;
        const closureOwnerToken = window.__yggtermXtermOwnerTokens;
        let closureOwnRegistered = false;
        let closureRetired = false;
        const closureSuperseded = () => {{
            if (closureRetired) {{
                return true;
            }}
            if (!closureOwnRegistered) {{
                return false;
            }}
            const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
            return Boolean(
                entry && entry.ownerToken !== undefined && entry.ownerToken !== closureOwnerToken
            );
        }};
        const standDownIfSuperseded = (site) => {{
            if (!closureSuperseded()) {{
                return false;
            }}
            if (!closureRetired) {{
                closureRetired = true;
                try {{
                    if (resizeObserver) {{
                        resizeObserver.disconnect();
                    }}
                }} catch (_error) {{}}
                try {{
                    detachHostInteractions(host);
                }} catch (_error) {{}}
                sendTerminalEvent({{
                    kind: "debug",
                    message: `superseded_closure_stand_down host=${{hostId}} site=${{String(site || '')}}`
                        + ` token=${{closureOwnerToken}}`,
                }});
            }}
            return true;
        }};
        const applyHostSurfaceContract = () => {{
            host.tabIndex = 0;
            host.style.pointerEvents = 'auto';
            host.style.setProperty('--yggterm-term-font-family', {font_family});
            host.style.setProperty('--yggterm-term-font-weight', String({font_weight}));
            host.style.setProperty('--yggterm-term-font-weight-bold', String({font_weight_bold}));
            host.style.setProperty('--yggterm-term-line-height', String({line_height}));
            host.style.setProperty('--yggterm-term-letter-spacing', '0px');
            host.style.setProperty('--yggterm-term-background', {background});
            host.style.setProperty('--yggterm-term-foreground', {foreground});
            host.style.setProperty('--yggterm-term-dim-foreground', {dim_foreground});
            host.style.setProperty('--yggterm-term-cursor', {cursor});
            host.style.setProperty('--yggterm-term-cursor-muted', {cursor_muted});
            host.style.setProperty('--yggterm-term-cursor-text', {cursor_text});
            host.style.setProperty('--yggterm-term-cursor-block-text', {cursor_text});
            host.style.setProperty('--yggterm-term-input-line-background', {input_line_background});
            host.style.setProperty('--yggterm-term-input-line-border', {input_line_border});
            host.style.setProperty('--yggterm-term-font-smoothing', {font_smoothing});
            host.style.setProperty('--yggterm-term-moz-font-smoothing', {moz_font_smoothing});
        }};
        const syncCurrentHostEntry = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (!entry) {{
                    return;
                }}
                entry.host = host;
                entry.sessionPath = host.getAttribute("data-terminal-session-path") || "";
            }} catch (_error) {{}}
        }};
        let termElementDetachedSinceMs = 0;
        let termElementDetachedCount = 0;
        let lastTermElementDetachedReportAtMs = 0;
        // REPAINT-STORM PROBE (2026-07-22, user-requested). The frame-corruption
        // pathology repaints the viewport at ~50Hz (a continuous garbled blink)
        // under heavy agent streaming — and it is INVISIBLE to paint-count health,
        // which only asks whether SOME paint happened, never how fast. Track the
        // paint RATE in a rolling 1s window and flag a SUSTAINED storm (a rate far
        // above a normal active terminal's handful/s), so telemetry catches the
        // blink next time instead of scoring a churning viewport "healthy".
        let paintRateWindowStartMs = 0;
        let paintRateWindowCount = 0;
        let repaintStormSinceMs = 0;
        let lastRepaintStormReportAtMs = 0;
        // Reopen circuit breaker (see rebindCurrentHost): a repair that never
        // converges must be bounded, or it retries at PAINT RATE and takes the
        // whole GUI down with it.
        let sameHostReopenBurstCount = 0;
        let sameHostReopenBurstStartMs = 0;
        let sameHostReopenCooldownUntilMs = 0;
        // DETACHED-TERM PROBE (2026-07-22). The blank-viewport class that every
        // existing health field scored "healthy": `term.element` is out of the
        // host, so nothing can paint, while the xterm OBJECT (buffer, cursor,
        // text tail, write callbacks) stays perfectly intact and keeps reporting
        // good news. The DOM is the only witness — so measure the DOM, and also
        // record WHY the in-place repair declined (the three `rebindCurrentHost`
        // reopen guards evaluated live), because a husk that matches `.xterm` but
        // has no `.xterm-screen` makes all three read false.
        const terminalHostAttachmentState = () => {{
            const termElement = term && term.element ? term.element : null;
            const liveHost = document.getElementById(hostId) || host;
            const hostContainsTermElement = Boolean(termElement && liveHost.contains(termElement));
            const xtermRoots = Array.from(liveHost.querySelectorAll('.xterm'));
            const orphanRoots = xtermRoots.filter((root) => root !== termElement);
            const screenInHost = Boolean(liveHost.querySelector('.xterm-screen'));
            const rowsInHost = Boolean(liveHost.querySelector('.xterm-rows'));
            const screenCanvasCount = Number(liveHost.querySelectorAll('.xterm-screen canvas').length || 0);
            const termElementConnected = Boolean(termElement && termElement.isConnected);
            const detached = Boolean(termElement) && !hostContainsTermElement;
            // The exact predicates `rebindCurrentHost` uses to decide a reopen.
            const guardHostMissingXtermRoot = xtermRoots.length === 0;
            const guardHostMissingRenderableLayer = Boolean(screenInHost && !rowsInHost && screenCanvasCount === 0);
            const guardStaleClause = Boolean(!termElementConnected && xtermRoots.length === 0);
            // 2026-07-23: rebindCurrentHost also repairs `termElementOutsideHost`
            // on a CONNECTED host (the 2026-07-22 husk fix). This mirror had not
            // been updated, so a plainly repairable detach was alarmed as
            // `unrepairable` (guihost trace: every detach episode carried
            // unrepairable=true while the very next repaint reattached it).
            const guardTermElementOutsideConnectedHost = Boolean(
                detached && liveHost && liveHost.isConnected
            );
            const repairWouldReopen = guardHostMissingXtermRoot
                || guardHostMissingRenderableLayer
                || guardStaleClause
                || guardTermElementOutsideConnectedHost;
            return {{
                term_element_present: Boolean(termElement),
                term_element_connected: termElementConnected,
                host_contains_term_element: hostContainsTermElement,
                host_connected: Boolean(liveHost && liveHost.isConnected),
                host_child_count: Number(liveHost.childElementCount || 0),
                xterm_root_count: xtermRoots.length,
                orphan_xterm_root_count: orphanRoots.length,
                screen_in_host: screenInHost,
                rows_in_host: rowsInHost,
                screen_canvas_count: screenCanvasCount,
                detached,
                // The husk signature: an `.xterm` root the repair guards accept as
                // proof of a mounted terminal, with no screen/rows/canvas under it.
                orphan_root_without_screen: Boolean(orphanRoots.length > 0 && !screenInHost),
                // ORPHAN FORENSICS (2026-07-22). Knowing an orphan root EXISTS was
                // never enough to fix the husk — the open question is where it came
                // from. `.xterm` roots have exactly one manufacturer (`term.open`),
                // so an orphan is some OTHER terminal's element; naming that owner
                // turns "the host is poisoned" into "closure X wiped closure Y".
                // Bounded to two roots and short strings: this rides the detach
                // event, and a DOM-event flood starves the GTK input region.
                orphan_root_desc: orphanRoots.slice(0, 2).map((root) => {{
                    let owner = 'none';
                    try {{
                        const reg = window.__yggtermXtermHosts || {{}};
                        for (const key of Object.keys(reg)) {{
                            if (reg[key] && reg[key].termElementRef === root) {{
                                owner = key === hostId ? 'self' : key;
                                break;
                            }}
                        }}
                    }} catch (_error) {{}}
                    const kidClasses = Array.from(root.children).slice(0, 4).map((child) =>
                        String(child.className || child.tagName || '').trim().split(/\s+/)[0] || '?'
                    ).join(',');
                    return `cls=${{String(root.className || '').trim().replace(/\s+/g, '+')}}`
                        + ` kids=${{Number(root.childElementCount || 0)}}`
                        + ` kidcls=${{kidClasses}}`
                        + ` has_screen=${{Boolean(root.querySelector('.xterm-screen'))}}`
                        + ` canvases=${{Number(root.querySelectorAll('canvas').length || 0)}}`
                        + ` connected=${{Boolean(root.isConnected)}}`
                        + ` owner=${{owner}}`;
                }}).join(' || '),
                // Two divs sharing our id would make `getElementById` a coin flip and
                // every "same host?" comparison meaningless. Cheap to rule in or out.
                host_id_element_count: Number(document.querySelectorAll(`[id="${{hostId}}"]`).length || 0),
                host_child_classes: Array.from(liveHost.children).slice(0, 4).map((child) =>
                    String(child.className || child.tagName || '').trim().split(/\s+/).join('.')
                ).join(','),
                repair_guard_host_missing_xterm_root: guardHostMissingXtermRoot,
                repair_guard_host_missing_renderable_layer: guardHostMissingRenderableLayer,
                repair_guard_stale_clause: guardStaleClause,
                repair_guard_term_element_outside_connected_host: guardTermElementOutsideConnectedHost,
                repair_would_reopen: repairWouldReopen,
                // Detached AND every repair guard declines = permanently blank
                // until the session is remounted by hand. This is the alarm.
                unrepairable_detached: Boolean(detached && !repairWouldReopen),
            }};
        }};
        const syncHostAttachmentEntry = (reason) => {{
            try {{
                const state = terminalHostAttachmentState();
                const now = Date.now();
                if (state.detached) {{
                    if (!termElementDetachedSinceMs) {{
                        termElementDetachedSinceMs = now;
                        termElementDetachedCount += 1;
                    }}
                }} else {{
                    termElementDetachedSinceMs = 0;
                }}
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.hostAttachmentState = state;
                    entry.termElementConnected = state.term_element_connected;
                    entry.hostContainsTermElement = state.host_contains_term_element;
                    entry.termElementDetachedSinceMs = termElementDetachedSinceMs;
                    entry.termElementDetachedCount = termElementDetachedCount;
                    // Published so a SIBLING closure's orphan forensics can name us
                    // as the owner of a root it found squatting in its host.
                    entry.termElementRef = term && term.element ? term.element : null;
                }}
                // Report each detach episode once, then at most every 30s while it
                // persists — enough for the trace to show duration without flooding.
                if (state.detached
                    && (now - lastTermElementDetachedReportAtMs > 30000
                        || lastTermElementDetachedReportAtMs === 0)) {{
                    lastTermElementDetachedReportAtMs = now;
                    const lastMutation = window.__yggtermHostMutationLog
                        && window.__yggtermHostMutationLog.length
                        ? window.__yggtermHostMutationLog[window.__yggtermHostMutationLog.length - 1]
                        : null;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `terminal_host_element_detached host=${{hostId}} reason=${{String(reason || '')}}`
                            + ` detached_ms=${{Math.max(0, now - termElementDetachedSinceMs)}}`
                            + ` episode=${{termElementDetachedCount}}`
                            + ` unrepairable=${{state.unrepairable_detached}}`
                            + ` orphan_root_without_screen=${{state.orphan_root_without_screen}}`
                            + ` xterm_roots=${{state.xterm_root_count}}`
                            + ` screen_in_host=${{state.screen_in_host}}`
                            + ` rows_in_host=${{state.rows_in_host}}`
                            + ` screen_canvases=${{state.screen_canvas_count}}`
                            + ` repair_would_reopen=${{state.repair_would_reopen}}`
                            + ` host_id_elements=${{state.host_id_element_count}}`
                            + ` host_kids=${{state.host_child_classes}}`
                            + ` orphan_desc=[${{state.orphan_root_desc}}]`
                            + ` last_mutation_site=${{lastMutation ? String(lastMutation.site || '') : 'none'}}`
                            + ` last_mutation_age_ms=${{lastMutation ? Math.max(0, now - Number(lastMutation.at_ms || now)) : -1}}`
                            + ` last_mutation_stack=${{lastMutation ? String(lastMutation.stack || '') : ''}}`
                    }});
                }}
                return state;
            }} catch (_error) {{
                return null;
            }}
        }};
        const terminalRendererSurfaceState = () => {{
            const screen = host.querySelector('.xterm-screen');
            const rowsLayer = host.querySelector('.xterm-rows');
            const canvasCount = host.querySelectorAll('.xterm-screen canvas').length;
            const visibleCanvasCount = Array.from(host.querySelectorAll('.xterm-screen canvas'))
                .filter((canvas) => {{
                    try {{
                        const style = window.getComputedStyle(canvas);
                        const rect = canvas.getBoundingClientRect();
                        return style.display !== 'none'
                            && style.visibility !== 'hidden'
                            && Number(rect.width || 0) > 0
                            && Number(rect.height || 0) > 0;
                    }} catch (_error) {{
                        return false;
                    }}
                }}).length;
            const renderer = term && term._core && term._core._renderService
                ? term._core._renderService._renderer
                : null;
            const rendererRows = renderer && renderer._rowContainer ? renderer._rowContainer : null;
            const rendererSelection = renderer && renderer._selectionContainer ? renderer._selectionContainer : null;
            return {{
                screen,
                rowsLayer,
                canvasCount,
                visibleCanvasCount,
                renderer,
                rendererRows,
                rendererSelection,
                missingTextLayer: Boolean(screen && !rowsLayer && canvasCount === 0 && visibleCanvasCount === 0),
            }};
        }};
        const syncRendererSurfaceRecoveryHostEntry = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return;
                }}
                const state = terminalRendererSurfaceState();
                entry.rendererSurfaceMissing = Boolean(state.missingTextLayer);
                entry.rendererSurfaceRecoveryCount = Number(rendererSurfaceRecoveryCount || 0);
                entry.lastRendererSurfaceRecoveryReason = String(lastRendererSurfaceRecoveryReason || '');
                entry.lastRendererSurfaceRecoveryAtMs = Number(lastRendererSurfaceRecoveryAtMs || 0);
                entry.lastRendererSurfaceRecoveryResult = String(lastRendererSurfaceRecoveryResult || '');
            }} catch (_error) {{}}
        }};
        const repairMissingRendererSurface = (reason = '') => {{
            const state = terminalRendererSurfaceState();
            const missing = Boolean(state.missingTextLayer);
            if (!missing) {{
                syncRendererSurfaceRecoveryHostEntry();
                return false;
            }}
            rendererSurfaceRecoveryCount += 1;
            lastRendererSurfaceRecoveryReason = String(reason || 'unknown');
            lastRendererSurfaceRecoveryAtMs = Date.now();
            let repaired = false;
            try {{
                if (state.screen && state.rendererRows) {{
                    if (state.rendererRows.parentElement !== state.screen) {{
                        state.screen.appendChild(state.rendererRows);
                        repaired = true;
                    }}
                    state.rendererRows.classList.add('xterm-rows');
                }}
                if (state.screen && state.rendererSelection && state.rendererSelection.parentElement !== state.screen) {{
                    state.screen.appendChild(state.rendererSelection);
                    repaired = true;
                }}
                if (state.renderer && typeof state.renderer.handleResize === 'function') {{
                    state.renderer.handleResize(Number(term.cols || 0), Number(term.rows || 0));
                    repaired = true;
                }}
                if (state.renderer && typeof state.renderer.renderRows === 'function') {{
                    state.renderer.renderRows(0, Math.max(0, Number(term.rows || 1) - 1));
                    repaired = true;
                }}
                if (typeof term.refresh === 'function') {{
                    term.refresh(0, Math.max(0, Number(term.rows || 1) - 1));
                    repaired = true;
                }}
            }} catch (error) {{
                lastRendererSurfaceRecoveryResult = `error:${{error && error.message ? error.message : String(error)}}`;
                syncRendererSurfaceRecoveryHostEntry();
                return false;
            }}
            const after = terminalRendererSurfaceState();
            lastRendererSurfaceRecoveryResult = repaired && !after.missingTextLayer
                ? 'repaired'
                : (repaired ? 'attempted_still_missing' : 'missing_no_renderer_layer');
            syncRendererSurfaceRecoveryHostEntry();
            if (repaired) {{
                sendTerminalEvent({{
                    kind: "debug",
                    message: `renderer_surface_repair host=${{hostId}} reason=${{reason}} result=${{lastRendererSurfaceRecoveryResult}}`
                }});
            }}
            return repaired && !after.missingTextLayer;
        }};
        // ⛔⛔ SSOT: THE ONLY WAY TO PUT OUR TERMINAL SURFACE BACK INTO A HOST.
        //
        // `term.open(host)` CANNOT rebuild a host we just wiped. Read the bundled
        // `assets/xterm/xterm.js` (this is not a guess — it is the shipped bytes):
        //
        //     open(e) ... if(!e) throw ...;
        //       if(e.isConnected||this._logService.debug(...),
        //          this.element?.ownerDocument.defaultView && this._coreBrowserService)
        //         return void(...)          // ← EARLY RETURN, no appendChild(e)
        //
        // Once `term.element` exists — i.e. after the very first mount — `open()`
        // returns WITHOUT appending the element to the new parent. So every
        // "wipe the host, then term.open() to rebuild it" recovery in this file was
        // a NO-OP that left an empty host and a detached `term.element`: a
        // permanently blank viewport that only a remount could clear. The only
        // re-attach that works after the first mount is moving `term.element`
        // ourselves.
        //
        // INVARIANT: a wipe of the host MUST be followed by this call in the SAME
        // synchronous task, so the cleared host never reaches the compositor, and
        // there is no branch under which "leave the host empty" is correct.
        const attachTerminalSurfaceToHost = (targetHost, site, allowOpen) => {{
            const existing = term && term.element ? term.element : null;
            // A HUSK is not a surface. `term.element` can be a bare `.xterm` root
            // with no screen under it (born in a partial `term.open` — see the
            // mount above and husk_is_born_in_a_partial_open.test.js), and moving
            // that back into the host is powerless: the root travels, the screen
            // does not exist. That is precisely why the live autopsy kept saying
            // `unrepairable=true` while this owner reported a successful reattach.
            // Drop it and open again — a partial open leaves the early-return
            // guard unarmed, so the rebuild really happens. The husk must be
            // removed BEFORE the open or it is stranded as an orphan beside the
            // new root.
            const existingIsHusk = Boolean(existing) && !terminalSurfaceIsComplete(existing);
            let mode = 'none';
            if (existing && !existingIsHusk) {{
                try {{
                    // appendChild MOVES a node that is parented elsewhere, so this
                    // is both the "reattach" and the "steal it back" case.
                    targetHost.appendChild(existing);
                    mode = 'reattached';
                }} catch (_error) {{
                    mode = 'reattach_failed';
                }}
            }} else if (allowOpen !== false) {{
                if (existingIsHusk) {{
                    try {{
                        existing.remove();
                    }} catch (_error) {{}}
                }}
                try {{
                    term.open(targetHost);
                    mode = existingIsHusk ? 'rebuilt_from_husk' : 'opened';
                }} catch (_error) {{
                    mode = existingIsHusk ? 'rebuild_from_husk_failed' : 'open_failed';
                }}
                if (
                    existingIsHusk
                    && !terminalSurfaceIsComplete(term && term.element ? term.element : null)
                ) {{
                    // The rebuild did not take: this terminal got far enough to arm
                    // `open()`'s early-return guard
                    //
                    //     if (this.element?.ownerDocument.defaultView
                    //         && this._coreBrowserService) return
                    //
                    // so the open above was a no-op. This used to be written up as a
                    // second species — "a terminal that opened COMPLETELY and then
                    // lost its screen" — and left for a remount. It is NOT that.
                    // `_coreBrowserService` is assigned in the MIDDLE of open(), six
                    // services before `element.appendChild(fragment)` puts the screen
                    // in the root, so a throw in that band arms the guard over a
                    // terminal that never finished opening. Same birth site as the
                    // husk we already fix, two statements later. Measured element by
                    // element in husk_species_b_is_a_late_partial_open.test.js.
                    //
                    // So the guard is stale, not authoritative, and clearing
                    // `element` disarms it — open() then runs its whole body and
                    // builds a real, writable surface. `element` is the terminal's
                    // own first assignment inside open(), so nothing is destroyed
                    // that open() would not overwrite anyway.
                    //
                    // The core holds that field, not the public wrapper (whose
                    // `element` is a delegating getter — assigning it silently does
                    // nothing). Feature-detect rather than assume: an xterm bump that
                    // moves this shape must degrade to the old put-the-husk-back
                    // behaviour, never to a silent half-repair.
                    const disarmTarget = term && term._core && term._core.element
                        ? term._core
                        : null;
                    if (disarmTarget) {{
                        try {{
                            disarmTarget.element = undefined;
                            term.open(targetHost);
                        }} catch (_error) {{}}
                    }}
                    if (terminalSurfaceIsComplete(term && term.element ? term.element : null)) {{
                        mode = 'rebuilt_from_husk_disarmed';
                        // The husk was already removed above; drop it for good so it
                        // cannot linger as an orphan root beside the new surface.
                        try {{
                            existing.remove();
                        }} catch (_error) {{}}
                    }} else {{
                        // Genuinely beyond repair here. Put the husk back so the
                        // surface is no worse than we found it and the autopsy can
                        // still name the root; a remount is the remaining cure.
                        mode = 'rebuild_from_husk_failed';
                        try {{
                            targetHost.appendChild(existing);
                        }} catch (_error) {{}}
                    }}
                }}
            }} else if (existing) {{
                // Opening was forbidden by the caller, so the husk is all we have.
                try {{
                    targetHost.appendChild(existing);
                    mode = 'reattached_husk';
                }} catch (_error) {{
                    mode = 'reattach_failed';
                }}
            }}
            window.__yggtermRecordHostMutation && window.__yggtermRecordHostMutation({{
                host_id: hostId,
                site: String(site || 'attach_terminal_surface'),
                mode,
                existing_is_husk: existingIsHusk,
                term_element_inside_after: Boolean(term && term.element && targetHost.contains(term.element)),
                screen_in_host_after: Boolean(targetHost.querySelector('.xterm-screen')),
            }});
            return mode;
        }};
        const rebindCurrentHost = (reason, reopen) => {{
            // A superseded closure must never repair: its "repair" evicts the
            // live owner's element (the two-owner fight, above).
            if (standDownIfSuperseded(`rebind:${{String(reason || '')}}`)) {{
                return host;
            }}
            try {{
                const liveHost = document.getElementById(hostId);
                if (!liveHost) {{
                    return host;
                }}
                const termElement = term && term.element ? term.element : null;
                const termElementDisconnected = Boolean(termElement && !termElement.isConnected);
                // Which host currently holds our element is no longer part of the
                // restore decision: after the wipe below the answer is always "none",
                // so the restore is unconditional. Reported for the autopsy only.
                const termElementHost = termElement && termElement.closest
                    ? termElement.closest('[id^="yggterm-terminal-"]')
                    : null;
                const hostMissingXtermRoot = !liveHost.querySelector('.xterm');
                const hostMissingRenderableLayer = Boolean(
                    liveHost.querySelector('.xterm-screen')
                    && !liveHost.querySelector('.xterm-rows')
                    && liveHost.querySelectorAll('.xterm-screen canvas').length === 0
                );
                const sameHost = liveHost === host;
                // The husk trap (guihost 2026-07-22): `term.element` sits DETACHED while an
                // empty `div.terminal.xterm` husk (only `.xterm-viewport`, no
                // `.xterm-screen`) occupies the host. All three guards above read false —
                // the husk matches `.xterm` (so hostMissingXtermRoot and the querySelector
                // in guard 3 are false), and hostMissingRenderableLayer needs an
                // `.xterm-screen` the husk lacks — so nothing reopens and the viewport is
                // blank forever. The witness the guards missed: our term.element is simply
                // not in the live host. When it is not, whatever occupies the host is NOT
                // our terminal, so a reopen (which re-appends term.element and drops the
                // husk) is always correct. This does NOT wipe a healthy surface: a healthy
                // `.xterm` in the host IS term.element, so `contains` is true and this
                // stays false — it can only fire when term.element is genuinely elsewhere,
                // which is itself the bug.
                // ⛔ …AND THAT REASONING WAS WRONG FOR A BACKGROUNDED HOST — fixed
                // 2026-07-22 after it shipped as a live regression in 2.12.2.
                // A backgrounded session's host leaves the DOM ENTIRELY (see
                // [[finding-status-dot-blink-idle-cpu]], which measured exactly
                // this), taking `term.element` with it. Every DOM-placement guard
                // then reads "broken" forever on every parked session, and
                // `emit_resize` re-fires them continuously: measured on guihost,
                // **3931 `rebind_host` events in 5 minutes (~13/s)**, all
                // `reason=emit_resize term_outside_host=true term_disconnected=true`.
                // Cost: WebKitWebProcess pinned at 26%, the viewport visibly
                // blinking ~2x/second, and — because the churn never let focus
                // settle on the xterm helper textarea — a session the user had just
                // switched to would come up blank and REFUSE KEYBOARD INPUT.
                //
                // A host that is not in the document has nothing to repair and
                // nobody looking at it. Placement is only meaningful for a host that
                // is actually on screen, so every placement guard is gated on that.
                // The original husk case is untouched: there the host IS on screen
                // (the user is staring at the blank viewport), so this reads true.
                const hostConnected = Boolean(liveHost && liveHost.isConnected);
                // Kept RAW (not pre-gated) so the trace still reports what was
                // actually observed; `hostConnected` below is what DECLINES to act
                // on it. A future autopsy should be able to read "the element was
                // outside, and we correctly did nothing because the host was parked".
                const termElementOutsideHost = Boolean(termElement && !liveHost.contains(termElement));
                const sameHostRepairWanted =
                    Boolean(
                        reopen
                        && sameHost
                        && hostConnected
                        && (
                            hostMissingXtermRoot
                            || hostMissingRenderableLayer
                            || termElementOutsideHost
                            || (termElementDisconnected && !liveHost.querySelector('.xterm'))
                        )
                    );
                // ⛔ CIRCUIT BREAKER (2026-07-22, user-reported). A repair that does
                // not converge must not retry forever. On a genuinely poisoned host
                // (`orphan_root_without_screen` — an `.xterm` root with no
                // `.xterm-screen`, no rows, no canvases; the code already calls this
                // `unrepairable=true`) the reopen "succeeds" and changes nothing, so
                // the next emit_paint asks again: measured 5958 rebind_host events in
                // 30 minutes, `reason=emit_paint reopened=true` at paint rate.
                //
                // Three symptoms, ONE cause — all three are this loop:
                //   1. the viewport blinks (~2/s) as the host is wiped and rebuilt;
                //   2. focus never settles on the xterm helper textarea, so the
                //      session REFUSES KEYBOARD INPUT;
                //   3. **the whole GUI goes unresponsive** — every attempt also
                //      `dioxus.send`s a debug event, and that is the SAME channel the
                //      web-surface geometry/cover eval uses. That eval is a
                //      documented "starvable oracle (seconds, under output flood)"
                //      and it is the INPUT-REGION authority, so starving it leaves a
                //      stale GTK input region: clicks land nowhere and the session
                //      appears to sit above everything else.
                //
                // So: allow a short burst of genuine repair attempts, then open the
                // circuit and let the surface stay visibly broken (recoverable by a
                // session switch) rather than melt the GUI trying to fix it.
                const REOPEN_BURST_LIMIT = 3;
                const REOPEN_BURST_WINDOW_MS = 2000;
                const REOPEN_COOLDOWN_MS = 5000;
                let sameHostNeedsReopen = sameHostRepairWanted;
                let reopenCircuitOpen = false;
                if (sameHostRepairWanted) {{
                    const breakerNowMs = Date.now();
                    if (breakerNowMs < sameHostReopenCooldownUntilMs) {{
                        sameHostNeedsReopen = false;
                        reopenCircuitOpen = true;
                    }} else {{
                        if (breakerNowMs - sameHostReopenBurstStartMs > REOPEN_BURST_WINDOW_MS) {{
                            sameHostReopenBurstStartMs = breakerNowMs;
                            sameHostReopenBurstCount = 0;
                        }}
                        sameHostReopenBurstCount += 1;
                        if (sameHostReopenBurstCount > REOPEN_BURST_LIMIT) {{
                            sameHostReopenCooldownUntilMs = breakerNowMs + REOPEN_COOLDOWN_MS;
                            sameHostNeedsReopen = false;
                            reopenCircuitOpen = true;
                            // ONE event per cooldown, not one per paint.
                            sendTerminalEvent({{
                                kind: "debug",
                                message: `terminal_host_reopen_circuit_open host=${{hostId}} reason=${{reason}}`
                                    + ` attempts=${{sameHostReopenBurstCount}} cooldown_ms=${{REOPEN_COOLDOWN_MS}}`
                                    + ` missing_root=${{hostMissingXtermRoot}} missing_layer=${{hostMissingRenderableLayer}}`
                                    + ` term_outside=${{termElementOutsideHost}} term_disconnected=${{termElementDisconnected}}`,
                            }});
                        }}
                    }}
                }} else {{
                    // The condition cleared on its own — the repair converged (or was
                    // never needed). Forget the burst so a later genuine fault still
                    // gets its full allowance.
                    sameHostReopenBurstCount = 0;
                }}
                if (sameHost && !sameHostNeedsReopen) {{
                    repairMissingRendererSurface(reason);
                    return host;
                }}
                const previousHost = host;
                try {{
                    detachHostInteractions(previousHost);
                }} catch (_error) {{}}
                try {{
                    if (resizeObserver) {{
                        resizeObserver.disconnect();
                    }}
                }} catch (_error) {{}}
                host = liveHost;
                window.__yggtermRecordHostMutation && window.__yggtermRecordHostMutation({{
                    host_id: hostId,
                    site: 'rebind_host_wipe',
                    reason: String(reason || ''),
                    child_count: Number(host.childElementCount || 0),
                    term_element_was_inside: Boolean(termElement && host.contains(termElement)),
                }});
                host.innerHTML = "";
                applyHostSurfaceContract();
                host.style.cursor = inputEnabled ? 'text' : 'default';
                // The host was wiped one statement ago, so the surface is ALWAYS
                // gone from it. The old code only re-appended `term.element` under
                // three conditions and otherwise trusted `term.open(host)` to
                // rebuild — which it never does (see attachTerminalSurfaceToHost).
                // Restoring unconditionally is the invariant, not an optimisation.
                const attachMode = attachTerminalSurfaceToHost(host, 'rebind_host_attach', reopen);
                const termElementReattached = attachMode === 'reattached';
                try {{
                    if (resizeObserver) {{
                        resizeObserver.observe(host);
                    }}
                }} catch (_error) {{}}
                try {{
                    attachHostInteractions(host);
                }} catch (_error) {{}}
                syncCurrentHostEntry();
                repairMissingRendererSurface(reason);
                sendTerminalEvent({{
                    kind: "debug",
                    message: `rebind_host host=${{hostId}} reason=${{reason}} reopened=${{reopen}} attach_mode=${{attachMode}} term_element_host=${{termElementHost ? String(termElementHost.id || '') : 'none'}} reattached=${{termElementReattached}} same_host=${{sameHost}} same_host_reopen=${{sameHostNeedsReopen}} term_disconnected=${{termElementDisconnected}} term_outside_host=${{termElementOutsideHost}} host_connected=${{hostConnected}} circuit_open=${{reopenCircuitOpen}} host_missing_root=${{hostMissingXtermRoot}} host_missing_renderable_layer=${{hostMissingRenderableLayer}} prev_connected=${{!!(previousHost && previousHost.isConnected)}} current_connected=${{!!(host && host.isConnected)}}`
                }});
            }} catch (_error) {{}}
            return host;
        }};
        applyHostSurfaceContract();
        let inputEnabled = Boolean({initial_input_enabled});
        let programmaticFocusEnabled = Boolean({initial_input_enabled});
        host.style.cursor = inputEnabled ? 'text' : 'default';
        const runtimeStyleId = `yggterm-xterm-runtime-style-${{hostId}}`;
        if (!document.getElementById(runtimeStyleId)) {{
            const runtimeStyle = document.createElement("style");
            runtimeStyle.id = runtimeStyleId;
            runtimeStyle.textContent = `
                #${{hostId}} .xterm,
                #${{hostId}} .xterm-helpers,
                #${{hostId}} .xterm-rows,
                #${{hostId}} .xterm .xterm-rows span,
                #${{hostId}} .xterm-helper-textarea {{
                    margin: 0 !important;
                    padding: 0 !important;
                    box-sizing: border-box !important;
                    font-family: var(--yggterm-term-font-family) !important;
                    font-weight: var(--yggterm-term-font-weight) !important;
                    font-feature-settings: "calt" 0, "liga" 0 !important;
                    font-variant-ligatures: none !important;
                    font-synthesis: none !important;
                    text-rendering: auto !important;
                    font-kerning: none !important;
                    letter-spacing: var(--yggterm-term-letter-spacing) !important;
                    -webkit-font-smoothing: var(--yggterm-term-font-smoothing) !important;
                    -moz-osx-font-smoothing: var(--yggterm-term-moz-font-smoothing) !important;
                }}
                #${{hostId}} .xterm {{
                    height: 100% !important;
                    width: 100% !important;
                    position: relative !important;
                }}
                #${{hostId}} .xterm-helpers {{
                    position: absolute !important;
                    inset: 0 !important;
                    overflow: hidden !important;
                    pointer-events: none !important;
                }}
                #${{hostId}} .xterm-helper-textarea {{
                    position: absolute !important;
                    top: 0 !important;
                    left: -10000px !important;
                    width: 1px !important;
                    height: 1px !important;
                    opacity: 0 !important;
                    z-index: -5 !important;
                    overflow: hidden !important;
                    border: 0 !important;
                    outline: none !important;
                    box-shadow: none !important;
                    background: transparent !important;
                    color: transparent !important;
                    caret-color: transparent !important;
                    pointer-events: none !important;
                    clip: rect(0, 0, 0, 0) !important;
                    clip-path: inset(50%) !important;
                    white-space: nowrap !important;
                    appearance: none !important;
                }}
                #${{hostId}} .xterm,
                #${{hostId}} .xterm-screen,
                #${{hostId}} .xterm-screen canvas,
                #${{hostId}} .xterm-rows,
                #${{hostId}} .xterm-rows > div,
                #${{hostId}} .xterm-rows span {{
                    user-select: none !important;
                    -webkit-user-select: none !important;
                }}
                #${{hostId}} .xterm-accessibility-tree:not(.debug) *::selection,
                #${{hostId}} .xterm-accessibility-tree:not(.debug) *::-moz-selection {{
                    color: transparent !important;
                    background-color: transparent !important;
                }}
                /* The vendored xterm.css hardcodes .xterm / .xterm-viewport
                   background to #000 (an OS X scrollbar-opacity workaround). With
                   the DOM renderer the viewport is usually a non-integer number of
                   rows tall, so a sub-row strip below the last row shows that #000
                   as a BLACK VOID line (the canvas renderer used to paint the whole
                   viewport, hiding it). Paint the terminal layers in the theme
                   background explicitly (NOT transparent — transparent reveals
                   whatever is behind, which can itself be black) so any fit
                   remainder reads as the terminal background; cells still paint
                   their own per-cell backgrounds on top. */
                #${{hostId}} .xterm,
                #${{hostId}} .xterm-viewport,
                #${{hostId}} .xterm-screen {{
                    background-color: var(--yggterm-term-background) !important;
                }}
                #${{hostId}} .xterm-screen {{
                    overflow: hidden !important;
                    height: 100% !important;
                    /* XTERM-BUG: scrollbar-not-draggable
                       The screen layer stacks above .xterm-viewport in
                       the xterm.js DOM. If it's full-width it covers
                       the right-edge scrollbar slot and intercepts
                       mouse clicks before they reach the native
                       scrollbar. Reserve the scrollbar width on the
                       right so the scrollbar is hit-testable. Matches
                       the ::-webkit-scrollbar width rule below, and the
                       SAME number the grid proposal subtracts — see
                       XTERM-BUG: right-edge-glyph-clipped. */
                    width: calc(100% - ${{terminalScrollbarGutterPx()}}px) !important;
                }}
                /* XTERM-BUG: scrollable-element-zero-height (xterm.js 6)
                   xterm.js 6 moved .xterm-screen INSIDE a new VS Code-derived
                   .xterm-scrollable-element (the ScrollableElement that now owns
                   scrolling — see the ydisp scroll readback fix). That element is
                   position:relative with NO height, and under the WebGL/canvas
                   renderer its only children are absolutely-positioned canvases, so
                   it collapses to 0px tall. .xterm-screen height:100% then
                   resolves against a 0-tall parent and ALSO collapses to 0 — the
                   grid canvas (correctly fit) overflows as a thin band at the top
                   with the rest of the host black (the "squished viewport"). The
                   DOM renderer hid this because its in-flow .xterm-rows gave the
                   chain intrinsic height. Give the new element the viewport height
                   it lacks so .xterm-screen's percentage height resolves and the
                   terminal fills the host. Harmless when the element is absent
                   (older xterm / DOM-only builds). */
                #${{hostId}} .xterm-scrollable-element {{
                    height: 100% !important;
                    width: 100% !important;
                }}
                #${{hostId}} .xterm-viewport {{
                    height: 100% !important;
                    overflow-x: hidden !important;
                    /* Sleek thin scrollbar — fixed width at rest and
                       on drag, so clicking the thumb does NOT cause
                       the browser to widen and shift it 2-3px left
                       (a UX wart of WebKit's default :active scrollbar
                       behavior). Thumb is the only thing that changes
                       on hover (color), not width. The screen layer
                       reserves an 8px right gutter to match. */
                    scrollbar-width: thin !important;
                    scrollbar-color: rgba(120, 142, 166, 0.36) transparent !important;
                }}
                #${{hostId}} .xterm-viewport:hover {{
                    scrollbar-color: rgba(140, 162, 186, 0.78) transparent !important;
                }}
                /* line-height is intentionally NOT !important: xterm.js 6's DOM
                   renderer sets each row div's inline line-height to the exact cell
                   height in px (e.g. 18px). A !important override to 1.0 (≈ font
                   size) beat that inline value and mis-spaced the visible rows —
                   harmless under the old canvas renderer (these rows were invisible
                   accessibility nodes) but it is the "staggered output" on the DOM
                   renderer. A non-important var keeps a fallback while letting
                   xterm's inline per-row px line-height win. */
                #${{hostId}} .xterm-rows {{
                    height: 100% !important;
                    color: var(--yggterm-term-foreground) !important;
                    -webkit-text-fill-color: currentColor !important;
                    line-height: var(--yggterm-term-line-height);
                }}
                #${{hostId}} .xterm-rows,
                #${{hostId}} .xterm-rows > div,
                #${{hostId}} .xterm-rows span {{
                    -webkit-text-fill-color: currentColor !important;
                    white-space: pre !important;
                }}
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-block,
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-block * {{
                    background-color: var(--yggterm-term-cursor) !important;
                    color: var(--yggterm-term-cursor-block-text) !important;
                    -webkit-text-fill-color: var(--yggterm-term-cursor-block-text) !important;
                }}
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-outline {{
                    background-color: var(--yggterm-term-cursor-cell-background, transparent) !important;
                    color: inherit !important;
                    -webkit-text-fill-color: currentColor !important;
                }}
                #${{hostId}} .xterm-rows > div {{
                    line-height: var(--yggterm-term-line-height);
                }}
                #${{hostId}} .xterm-rows span,
                #${{hostId}} .xterm-rows div {{
                    line-height: var(--yggterm-term-line-height);
                }}
                #${{hostId}} .xterm-rows .xterm-dim:not(.xterm-cursor):not([class*="xterm-fg-"]),
                #${{hostId}} .xterm-rows [style*="opacity: 0"]:not(.xterm-cursor):not([class*="xterm-fg-"]),
                #${{hostId}} .xterm-rows [style*="opacity:0"]:not(.xterm-cursor):not([class*="xterm-fg-"]),
                #${{hostId}} .xterm-rows span[style*="opacity: 0"]:not(.xterm-cursor):not([class*="xterm-fg-"]),
                #${{hostId}} .xterm-rows span[style*="opacity:0"]:not(.xterm-cursor):not([class*="xterm-fg-"]) {{
                    color: var(--yggterm-term-dim-foreground) !important;
                    opacity: 1 !important;
                }}
                #${{hostId}} .xterm-rows .xterm-dim:not(.xterm-cursor)[class*="xterm-fg-"] {{
                    opacity: 1 !important;
                }}
                #${{hostId}} .xterm-rows .xterm-dim:not(.xterm-cursor):not([class*="xterm-fg-"])[style*="background-color:#f0f0f2"],
                #${{hostId}} .xterm-rows .xterm-dim:not(.xterm-cursor):not([class*="xterm-fg-"])[style*="background-color: rgb(240, 240, 242)"] {{
                    color: #151b23 !important;
                }}
                #${{hostId}} .xterm-rows span:not([class*="xterm-fg-"])[style*="background-color:#f0f0f2"],
                #${{hostId}} .xterm-rows div:not([class*="xterm-fg-"])[style*="background-color:#f0f0f2"],
                #${{hostId}} .xterm-rows span:not([class*="xterm-fg-"])[style*="background-color: rgb(240, 240, 242)"],
                #${{hostId}} .xterm-rows div:not([class*="xterm-fg-"])[style*="background-color: rgb(240, 240, 242)"] {{
                    color: #151b23 !important;
                }}
                #${{hostId}} .xterm-rows .xterm-dim:not(.xterm-cursor):not([class*="xterm-fg-"])[style*="background-color:#393939"],
                #${{hostId}} .xterm-rows .xterm-dim:not(.xterm-cursor):not([class*="xterm-fg-"])[style*="background-color: rgb(57, 57, 57)"] {{
                    color: #fbfbfd !important;
                }}
                #${{hostId}} .xterm-rows span:not([class*="xterm-fg-"])[style*="background-color:#393939"],
                #${{hostId}} .xterm-rows div:not([class*="xterm-fg-"])[style*="background-color:#393939"],
                #${{hostId}} .xterm-rows span:not([class*="xterm-fg-"])[style*="background-color: rgb(57, 57, 57)"],
                #${{hostId}} .xterm-rows div:not([class*="xterm-fg-"])[style*="background-color: rgb(57, 57, 57)"] {{
                    color: #fbfbfd !important;
                }}
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-block,
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-block.xterm-dim,
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-block.xterm-dim * {{
                    background-color: var(--yggterm-term-cursor) !important;
                    color: var(--yggterm-term-cursor-block-text) !important;
                    -webkit-text-fill-color: var(--yggterm-term-cursor-block-text) !important;
                }}
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-blink,
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-blink * {{
                    animation: none !important;
                    -webkit-animation: none !important;
                }}
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-outline,
                #${{hostId}} .xterm-rows .xterm-cursor.xterm-cursor-outline.xterm-dim {{
                    background-color: var(--yggterm-term-cursor-cell-background, transparent) !important;
                    color: inherit !important;
                    -webkit-text-fill-color: currentColor !important;
                }}
                #${{hostId}}:not(.yggterm-term-focused) .xterm-rows .xterm-cursor.xterm-cursor-block,
                #${{hostId}}:not(.yggterm-term-focused) .xterm-rows .xterm-cursor.xterm-cursor-block.xterm-dim,
                #${{hostId}}:not(.yggterm-term-focused) .xterm-rows .xterm-cursor.xterm-cursor-outline,
                #${{hostId}}:not(.yggterm-term-focused) .xterm-rows .xterm-cursor.xterm-cursor-outline.xterm-dim {{
                    background-color: var(--yggterm-term-cursor-cell-background, transparent) !important;
                    box-shadow: inset 0 0 0 1px var(--yggterm-term-cursor) !important;
                    color: inherit !important;
                    -webkit-text-fill-color: currentColor !important;
                }}
                /* WebKit / Chromium thin sleek scrollbar to match Firefox.
                   Width is fixed at 8px across rest/hover/active so that
                   clicking the thumb doesn't trigger WebKit's default
                   "fatter scrollbar while dragging" behavior — which
                   produced a 2-3px leftward shift and a chunky drag
                   highlight on rest-to-active transition. Color is the
                   only thing that changes; transition keeps it smooth. */
                #${{hostId}} .xterm-viewport::-webkit-scrollbar {{
                    width: ${{terminalScrollbarGutterPx()}}px !important;
                    height: 0 !important;
                    background: transparent !important;
                }}
                #${{hostId}} .xterm-viewport::-webkit-scrollbar-track {{
                    background: transparent !important;
                }}
                #${{hostId}} .xterm-viewport::-webkit-scrollbar-thumb {{
                    background: rgba(120, 142, 166, 0.36) !important;
                    border-radius: 4px !important;
                    background-clip: padding-box !important;
                    /* 36px min-height for a comfortable click target.
                       8px scrollbar slot stays fixed across states; the
                       transparent border below trims the VISIBLE thumb
                       width on hover/active per user preference: full
                       8px at rest, slimmer when actively engaged. */
                    min-height: 36px !important;
                    border: 0 solid transparent !important;
                    transition: background-color 120ms ease,
                                border-width 120ms ease !important;
                }}
                #${{hostId}} .xterm-viewport:hover::-webkit-scrollbar-thumb {{
                    background: rgba(140, 162, 186, 0.78) !important;
                    background-clip: padding-box !important;
                    /* 1px transparent border on each side → 6px visible
                       width when the viewport is hovered. */
                    border: 1px solid transparent !important;
                }}
                #${{hostId}} .xterm-viewport::-webkit-scrollbar-thumb:hover {{
                    background: rgba(150, 172, 196, 0.78) !important;
                    background-clip: padding-box !important;
                    border: 1px solid transparent !important;
                }}
                #${{hostId}} .xterm-viewport::-webkit-scrollbar-thumb:active {{
                    background: rgba(170, 188, 210, 0.85) !important;
                    background-clip: padding-box !important;
                    /* 2px transparent border each side → 4px visible
                       width while actively dragging — slimmest state. */
                    border: 2px solid transparent !important;
                }}
                #${{hostId}} .xterm-viewport::-webkit-scrollbar-corner {{
                    background: transparent !important;
                }}
            `;
            document.head.appendChild(runtimeStyle);
        }}
        function applyNonSelectableSurfaceContract() {{
            try {{
                const selectionNodes = [
                    host.querySelector('.xterm'),
                    host.querySelector('.xterm-screen'),
                    ...Array.from(host.querySelectorAll('.xterm-screen canvas, .xterm-rows, .xterm-rows > div, .xterm-rows span')),
                ].filter(Boolean);
                for (const node of selectionNodes) {{
                    node.style.userSelect = 'none';
                    node.style.webkitUserSelect = 'none';
                }}
            }} catch (_error) {{}}
        }}
        const currentHostSessionPath = () => String(
            host.getAttribute('data-terminal-session-path') || ''
        ).trim();
        const activeTerminalSessionPath = () => {{
            try {{
                const explicit = String(window.__yggtermActiveTerminalSessionPath || '').trim();
                if (explicit) {{
                    return explicit;
                }}
            }} catch (_error) {{}}
            try {{
                const activeHost = document.querySelector(
                    '[id^="yggterm-terminal-"][data-terminal-session-path][data-active-session-host="true"]'
                );
                return activeHost
                    ? String(activeHost.getAttribute('data-terminal-session-path') || '').trim()
                    : '';
            }} catch (_error) {{
                return '';
            }}
        }};
        const documentSurfaceOwnsViewport = () => {{
            try {{
                return String(host.getAttribute('data-document-surface-owns-viewport') || '')
                    .trim() === 'true';
            }} catch (_error) {{
                return false;
            }}
        }};
        const hostOwnsActiveTerminalInput = () => {{
            // A shell-DOM document surface covering this host owns input; the
            // terminal must stand down so its focus-reclaim cascade cannot steal
            // focus from the document editor.
            if (documentSurfaceOwnsViewport()) {{
                return false;
            }}
            const sessionPath = currentHostSessionPath();
            const activeSessionPath = activeTerminalSessionPath();
            return Boolean(sessionPath && activeSessionPath && sessionPath === activeSessionPath);
        }};
        let xtermInputLineDecoration = null;
        let xtermInputLineDecorationMarker = null;
        let xtermInputLineDecorationRenderDisposable = null;
        let xtermInputLineDecorationVisible = false;
        let xtermInputLineDecorationLine = -1;
        let xtermInputLineDecorationWidth = 0;
        let xtermInputLineDecorationError = '';
        let xtermInputLineDecorationRenderCount = 0;
        let xtermInputLineDecorationRefreshPending = false;
        const xtermInputLineDecorationEnabled = {input_line_decoration_enabled};
        const xtermInputLineDecorationBackground = {input_line_background};
        const terminalSessionAllowsXtermInputLineDecoration = () => {{
            try {{
                const kind = String(host.getAttribute('data-terminal-session-kind') || '').toLowerCase();
                return Boolean(xtermInputLineDecorationEnabled && kind.includes('codex'));
            }} catch (_error) {{
                return false;
            }}
        }};
        const currentXtermCursorLineInfo = () => {{
            try {{
                const active = term && term.buffer ? term.buffer.active : null;
                if (!active) {{
                    return {{ line: -1, viewport_row: -1, text: '' }};
                }}
                const baseY = Math.max(0, Number(active.baseY || 0));
                const cursorY = Math.max(0, Number(active.cursorY || 0));
                const viewportY = effectiveXtermViewportY(active);
                const line = Math.max(0, baseY + cursorY);
                const bufferLine = active.getLine ? active.getLine(line) : null;
                return {{
                    line,
                    viewport_row: line - viewportY,
                    text: bufferLine && typeof bufferLine.translateToString === 'function'
                        ? String(bufferLine.translateToString(true) || '')
                        : '',
                }};
            }} catch (_error) {{
                return {{ line: -1, viewport_row: -1, text: '' }};
            }}
        }};
        const syncXtermInputLineDecorationHostEntry = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return;
                }}
                const decorationElement = xtermInputLineDecoration && xtermInputLineDecoration.element
                    ? xtermInputLineDecoration.element
                    : null;
                const decorationElementStyle = decorationElement ? window.getComputedStyle(decorationElement) : null;
                const decorationElementRect = decorationElement ? decorationElement.getBoundingClientRect() : null;
                const decorationElementVisible = Boolean(
                    decorationElement
                    && decorationElementStyle
                    && String(decorationElementStyle.display || '') !== 'none'
                    && String(decorationElementStyle.visibility || '') !== 'hidden'
                    && String(decorationElementStyle.opacity || '') !== '0'
                    && decorationElementRect
                    && Number(decorationElementRect.width || 0) > 0
                    && Number(decorationElementRect.height || 0) > 0
                );
                const decorationDisposed = Boolean(
                    (xtermInputLineDecoration && xtermInputLineDecoration.isDisposed)
                    || (xtermInputLineDecorationMarker && xtermInputLineDecorationMarker.isDisposed)
                );
                entry.xtermInputLineDecorationPresent = Boolean(xtermInputLineDecoration);
                entry.xtermInputLineDecorationVisible = Boolean(xtermInputLineDecorationVisible && !decorationDisposed);
                entry.xtermInputLineDecorationLine = Number(xtermInputLineDecorationLine || 0);
                entry.xtermInputLineDecorationWidth = Number(xtermInputLineDecorationWidth || 0);
                entry.xtermInputLineDecorationBackground = String(xtermInputLineDecorationBackground || '');
                entry.xtermInputLineDecorationError = String(xtermInputLineDecorationError || '');
                entry.xtermInputLineDecorationDisposed = decorationDisposed;
                entry.xtermInputLineDecorationMarkerLine = xtermInputLineDecorationMarker
                    && Number.isFinite(Number(xtermInputLineDecorationMarker.line))
                        ? Number(xtermInputLineDecorationMarker.line)
                        : null;
                entry.xtermInputLineDecorationElementPresent = Boolean(decorationElement);
                entry.xtermInputLineDecorationElementVisible = decorationElementVisible;
                entry.xtermInputLineDecorationElementBackground = decorationElementStyle
                    ? String(decorationElementStyle.backgroundColor || '')
                    : '';
                entry.xtermInputLineDecorationElementDisplay = decorationElementStyle
                    ? String(decorationElementStyle.display || '')
                    : '';
                entry.xtermInputLineDecorationElementRect = decorationElementRect
                    ? {{
                        left: Number(decorationElementRect.left.toFixed(2)),
                        top: Number(decorationElementRect.top.toFixed(2)),
                        width: Number(decorationElementRect.width.toFixed(2)),
                        height: Number(decorationElementRect.height.toFixed(2)),
                    }}
                    : null;
                entry.xtermInputLineDecorationRenderCount = Number(xtermInputLineDecorationRenderCount || 0);
            }} catch (_error) {{}}
        }};
        const xtermInputLineDecorationAlive = () => Boolean(
            xtermInputLineDecoration
            && !xtermInputLineDecoration.isDisposed
            && xtermInputLineDecorationMarker
            && !xtermInputLineDecorationMarker.isDisposed
        );
        const refreshXtermInputLineDecorationRows = () => {{
            try {{
                if (term && typeof term.refresh === 'function') {{
                    term.refresh(0, Math.max(0, Number(term.rows || 1) - 1));
                }}
            }} catch (_error) {{}}
        }};
        const paintXtermInputLineDecorationElement = (element) => {{
            try {{
                if (!element || !element.style) {{
                    return;
                }}
                element.style.backgroundColor = xtermInputLineDecorationBackground;
                element.style.pointerEvents = 'none';
                element.style.boxSizing = 'border-box';
                xtermInputLineDecorationRenderCount += 1;
            }} catch (_error) {{}}
        }};
        const scheduleXtermInputLineDecorationRefresh = () => {{
            refreshXtermInputLineDecorationRows();
            if (xtermInputLineDecorationRefreshPending) {{
                return;
            }}
            xtermInputLineDecorationRefreshPending = true;
            const finish = () => {{
                xtermInputLineDecorationRefreshPending = false;
                refreshXtermInputLineDecorationRows();
                try {{
                    setTimeout(() => refreshXtermInputLineDecorationRows(), 80);
                }} catch (_error) {{}}
            }};
            try {{
                requestAnimationFrame(finish);
            }} catch (_error) {{
                try {{
                    setTimeout(finish, 16);
                }} catch (_innerError) {{
                    finish();
                }}
            }}
        }};
        const disposeXtermInputLineDecoration = (reason = '') => {{
            const hadDecoration = Boolean(xtermInputLineDecoration || xtermInputLineDecorationMarker);
            try {{
                if (xtermInputLineDecoration && typeof xtermInputLineDecoration.dispose === 'function') {{
                    xtermInputLineDecoration.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                if (xtermInputLineDecorationRenderDisposable && typeof xtermInputLineDecorationRenderDisposable.dispose === 'function') {{
                    xtermInputLineDecorationRenderDisposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                if (xtermInputLineDecorationMarker && typeof xtermInputLineDecorationMarker.dispose === 'function') {{
                    xtermInputLineDecorationMarker.dispose();
                }}
            }} catch (_error) {{}}
            xtermInputLineDecoration = null;
            xtermInputLineDecorationMarker = null;
            xtermInputLineDecorationRenderDisposable = null;
            xtermInputLineDecorationVisible = false;
            xtermInputLineDecorationLine = -1;
            xtermInputLineDecorationWidth = 0;
            if (reason && reason !== 'error') {{
                xtermInputLineDecorationError = '';
            }}
            if (hadDecoration) {{
                scheduleXtermInputLineDecorationRefresh();
            }}
            syncXtermInputLineDecorationHostEntry();
        }};
        const syncXtermInputLineDecoration = (reason = '') => {{
            try {{
                const cursorInfo = currentXtermCursorLineInfo();
                const text = String(cursorInfo.text || '').trimStart();
                const width = Math.max(1, Number(term && term.cols ? term.cols : 0));
                const shouldDecorate = Boolean(
                    inputEnabled
                    && hostOwnsActiveTerminalInput()
                    && terminalSessionAllowsXtermInputLineDecoration()
                    && text.startsWith('›')
                    && cursorInfo.line >= 0
                    && width > 0
                    && term
                    && typeof term.registerMarker === 'function'
                    && typeof term.registerDecoration === 'function'
                );
                if (!shouldDecorate) {{
                    disposeXtermInputLineDecoration(reason || 'not_codex_prompt');
                    return;
                }}
                if (
                    xtermInputLineDecorationAlive()
                    && xtermInputLineDecorationLine === cursorInfo.line
                    && xtermInputLineDecorationWidth === width
                ) {{
                    xtermInputLineDecorationVisible = true;
                    paintXtermInputLineDecorationElement(xtermInputLineDecoration.element);
                    if (reason !== 'render') {{
                        scheduleXtermInputLineDecorationRefresh();
                    }}
                    syncXtermInputLineDecorationHostEntry();
                    return;
                }}
                disposeXtermInputLineDecoration(reason || 'refresh');
                const marker = term.registerMarker(0);
                if (!marker) {{
                    xtermInputLineDecorationError = 'register_marker_failed';
                    syncXtermInputLineDecorationHostEntry();
                    return;
                }}
                const decoration = term.registerDecoration({{
                    marker,
                    x: 0,
                    width,
                    height: 1,
                    backgroundColor: xtermInputLineDecorationBackground,
                    layer: 'bottom',
                }});
                if (!decoration) {{
                    try {{ marker.dispose(); }} catch (_error) {{}}
                    xtermInputLineDecorationError = 'register_decoration_failed';
                    syncXtermInputLineDecorationHostEntry();
                    return;
                }}
                xtermInputLineDecoration = decoration;
                xtermInputLineDecorationMarker = marker;
                xtermInputLineDecorationRenderDisposable = typeof decoration.onRender === 'function'
                    ? decoration.onRender((element) => {{
                        paintXtermInputLineDecorationElement(element);
                        syncXtermInputLineDecorationHostEntry();
                    }})
                    : null;
                paintXtermInputLineDecorationElement(decoration.element);
                xtermInputLineDecorationVisible = true;
                xtermInputLineDecorationLine = Number.isFinite(Number(marker.line))
                    ? Number(marker.line)
                    : cursorInfo.line;
                xtermInputLineDecorationWidth = width;
                xtermInputLineDecorationError = '';
                scheduleXtermInputLineDecorationRefresh();
                syncXtermInputLineDecorationHostEntry();
            }} catch (error) {{
                xtermInputLineDecorationError = error && error.message ? error.message : String(error);
                disposeXtermInputLineDecoration('error');
                syncXtermInputLineDecorationHostEntry();
            }}
        }};
        function syncFocusClass() {{
            try {{
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                const focused = Boolean(helperTextarea && document.activeElement === helperTextarea);
                host.classList.toggle('yggterm-term-focused', focused);
                const focusCapture = host.querySelector('.yggterm-term-focus-capture');
                if (focusCapture) {{
                    focusCapture.style.pointerEvents =
                        inputEnabled && hostOwnsActiveTerminalInput() && !focused ? 'auto' : 'none';
                    focusCapture.style.cursor = inputEnabled ? 'text' : 'default';
                }}
                refreshCursorContrastContract();
                try {{
                    applySoftwareCanvasLayerOptimization('focus_class');
                }} catch (_error) {{}}
                try {{
                    syncXtermInputLineDecoration('focus_class');
                }} catch (_error) {{}}
            }} catch (_error) {{}}
        }}
        let transientUiFocusClaimUntilMs = 0;
        function elementBlocksTerminalAutofocus(active) {{
            try {{
                if (!active) {{
                    return false;
                }}
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                if (active === helperTextarea || (term && term.textarea && active === term.textarea)) {{
                    return false;
                }}
                if (active === document.body || active === document.documentElement || active === host) {{
                    return false;
                }}
                if (active.id === {SEARCH_INPUT_ID:?}) {{
                    return true;
                }}
                if (
                    active.closest
                    && {ui_focus_owners}.some((sel) => active.closest(sel))
                ) {{
                    return true;
                }}
                const settingsFieldKey = active.getAttribute
                    ? String(active.getAttribute('data-settings-field-key') || '')
                    : '';
                if (settingsFieldKey) {{
                    return true;
                }}
                const tagName = String(active.tagName || '').toLowerCase();
                const intrinsicallyInteractive =
                    tagName === 'input'
                    || tagName === 'textarea'
                    || tagName === 'select'
                    || tagName === 'button'
                    || tagName === 'a'
                    || Boolean(active.isContentEditable);
                if (host.contains(active)) {{
                    return intrinsicallyInteractive;
                }}
                if (intrinsicallyInteractive) {{
                    return true;
                }}
                const tabIndex = Number(active.tabIndex || -1);
                return Number.isFinite(tabIndex) && tabIndex >= 0;
            }} catch (_error) {{
                return false;
            }}
        }}
        function markTransientUiFocusClaim(delayMs) {{
            const delay = Math.max(0, Number(delayMs || 0));
            transientUiFocusClaimUntilMs = Math.max(
                transientUiFocusClaimUntilMs,
                Date.now() + delay
            );
        }}
        function releaseBlockingUiFocusForTerminalReclaim() {{
            try {{
                const active = document.activeElement;
                if (!active) {{
                    transientUiFocusClaimUntilMs = 0;
                    return false;
                }}
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                if (active === helperTextarea || (term && term.textarea && active === term.textarea)) {{
                    transientUiFocusClaimUntilMs = 0;
                    return false;
                }}
                const settingsFieldKey = active.getAttribute
                    ? String(active.getAttribute('data-settings-field-key') || '')
                    : '';
                const shouldBlur =
                    active.id === {SEARCH_INPUT_ID:?}
                    || Boolean(active.closest && active.closest('[data-yggterm-titlebar-search="1"]'))
                    || Boolean(settingsFieldKey);
                transientUiFocusClaimUntilMs = 0;
                if (!shouldBlur || typeof active.blur !== 'function') {{
                    return false;
                }}
                active.blur();
                return true;
            }} catch (_error) {{
                transientUiFocusClaimUntilMs = 0;
                return false;
            }}
        }}
        function activeElementBlocksTerminalAutofocus() {{
            const active = document.activeElement;
            const globalClaimUntilMs = Number(window.__yggtermUiFocusClaimUntilMs || 0);
            const sidebarKeyboardOwner = Boolean(window.__yggtermSidebarKeyboardOwner);
            const bodyOwnsFocus =
                !active
                || active === document.body
                || active === document.documentElement
                || active === host;
            const focusedSearchSurface = (() => {{
                try {{
                    const searchSurface = document.querySelector('[data-titlebar-search-focused="1"]');
                    return Boolean(searchSurface && searchSurface.isConnected);
                }} catch (_error) {{
                    return false;
                }}
            }})();
            if (focusedSearchSurface) {{
                return true;
            }}
            if (sidebarKeyboardOwner) {{
                return true;
            }}
            if (Date.now() < globalClaimUntilMs) {{
                return true;
            }}
            if (!bodyOwnsFocus && Date.now() < transientUiFocusClaimUntilMs) {{
                return true;
            }}
            return elementBlocksTerminalAutofocus(active);
        }}
        function contrastThresholdForBackground(sampleBackground) {{
            const parsed = parseCssColor(sampleBackground);
            const luminance = parsed ? relativeLuminance(parsed) : null;
            return luminance != null && luminance > 0.72 ? 6.5 : 4.5;
        }}
        function cssRgbFromParsed(color) {{
            if (!color) {{
                return null;
            }}
            return `rgb(${{Math.max(0, Math.min(255, Math.round(Number(color.r || 0))))}}, ${{Math.max(0, Math.min(255, Math.round(Number(color.g || 0))))}}, ${{Math.max(0, Math.min(255, Math.round(Number(color.b || 0))))}})`;
        }}
        function invertCssColor(value) {{
            const parsed = parseCssColor(value);
            if (!parsed) {{
                return null;
            }}
            return cssRgbFromParsed({{
                r: 255 - Number(parsed.r || 0),
                g: 255 - Number(parsed.g || 0),
                b: 255 - Number(parsed.b || 0),
            }});
        }}
        function computeFocusedBlockCursorTextColor() {{
            try {{
                const hostStyle = window.getComputedStyle(host);
                const cursorFill = String(
                    hostStyle.getPropertyValue('--yggterm-term-cursor')
                    || (term && term.options && term.options.theme ? term.options.theme.cursor || '' : '')
                    || ''
                ).trim();
                const defaultGlyph = String(
                    hostStyle.getPropertyValue('--yggterm-term-foreground')
                    || (term && term.options && term.options.theme ? term.options.theme.foreground || '' : '')
                    || ''
                ).trim();
                const preferredGlyph = String(hostStyle.getPropertyValue('--yggterm-term-cursor-text') || '').trim();
                const invertedGlyph = invertCssColor(defaultGlyph);
                const threshold = contrastThresholdForBackground(cursorFill);
                const seen = new Set();
                const candidates = [defaultGlyph, invertedGlyph, preferredGlyph, '#0f172a', '#fbfbfd']
                    .filter((value) => {{
                        const normalized = String(value || '').trim();
                        if (!normalized || seen.has(normalized)) {{
                            return false;
                        }}
                        seen.add(normalized);
                        return true;
                    }});
                const defaultContrast = contrastRatio(defaultGlyph, cursorFill);
                if (defaultContrast != null && defaultContrast >= threshold) {{
                    return defaultGlyph;
                }}
                let bestColor = defaultGlyph || preferredGlyph || '#fbfbfd';
                let bestContrast = defaultContrast == null ? -1 : defaultContrast;
                for (const candidate of candidates) {{
                    const contrast = contrastRatio(candidate, cursorFill);
                    if (contrast != null && contrast > bestContrast) {{
                        bestContrast = contrast;
                        bestColor = candidate;
                    }}
                }}
                return bestColor;
            }} catch (_error) {{
                return String(host.style.getPropertyValue('--yggterm-term-cursor-text') || '').trim() || '#fbfbfd';
            }}
        }}
        function cssColorFromXtermRgbInt(value) {{
            const safe = Number(value || 0);
            if (!Number.isFinite(safe) || safe < 0) {{
                return '';
            }}
            const r = (safe >> 16) & 255;
            const g = (safe >> 8) & 255;
            const b = safe & 255;
            return `rgb(${{r}}, ${{g}}, ${{b}})`;
        }}
        function isTransparentCssColor(value) {{
            const color = String(value || '').trim().toLowerCase();
            return !color
                || color === 'transparent'
                || color === 'rgba(0, 0, 0, 0)'
                || color === 'rgba(0,0,0,0)';
        }}
        function cssColorFromXtermPaletteIndex(index) {{
            const safe = Number(index);
            if (!Number.isFinite(safe) || safe < 0) {{
                return '';
            }}
            const palette = term && term.options && term.options.theme
                ? term.options.theme
                : {{}};
            const ansi = [
                palette.black,
                palette.red,
                palette.green,
                palette.yellow,
                palette.blue,
                palette.magenta,
                palette.cyan,
                palette.white,
                palette.brightBlack,
                palette.brightRed,
                palette.brightGreen,
                palette.brightYellow,
                palette.brightBlue,
                palette.brightMagenta,
                palette.brightCyan,
                palette.brightWhite,
            ];
            if (safe < ansi.length && ansi[safe]) {{
                return String(ansi[safe]);
            }}
            if (safe >= 16 && safe <= 231) {{
                const offset = safe - 16;
                const channel = (value) => value === 0 ? 0 : 55 + value * 40;
                return `rgb(${{channel(Math.floor(offset / 36) % 6)}}, ${{channel(Math.floor(offset / 6) % 6)}}, ${{channel(offset % 6)}})`;
            }}
            if (safe >= 232 && safe <= 255) {{
                const level = 8 + (safe - 232) * 10;
                return `rgb(${{level}}, ${{level}}, ${{level}})`;
            }}
            if (safe > 255) {{
                return cssColorFromXtermRgbInt(safe);
            }}
            return '';
        }}
        function cssColorFromXtermCellPlane(cell, plane) {{
            if (!cell) {{
                return '';
            }}
            const isForeground = plane === 'fg';
            try {{
                const isDefault = isForeground
                    ? (typeof cell.isFgDefault === 'function' ? cell.isFgDefault() : false)
                    : (typeof cell.isBgDefault === 'function' ? cell.isBgDefault() : false);
                if (isDefault) {{
                    const theme = term && term.options && term.options.theme
                        ? term.options.theme
                        : {{}};
                    return isForeground
                        ? String(theme.foreground || '').trim()
                        : '';
                }}
                const isRgb = isForeground
                    ? (typeof cell.isFgRGB === 'function' ? cell.isFgRGB() : false)
                    : (typeof cell.isBgRGB === 'function' ? cell.isBgRGB() : false);
                const isPalette = isForeground
                    ? (typeof cell.isFgPalette === 'function' ? cell.isFgPalette() : false)
                    : (typeof cell.isBgPalette === 'function' ? cell.isBgPalette() : false);
                const color = isForeground
                    ? (typeof cell.getFgColor === 'function' ? cell.getFgColor() : -1)
                    : (typeof cell.getBgColor === 'function' ? cell.getBgColor() : -1);
                if (isRgb) {{
                    return cssColorFromXtermRgbInt(color);
                }}
                if (isPalette || (Number(color) >= 0 && Number(color) <= 255)) {{
                    return cssColorFromXtermPaletteIndex(color);
                }}
                if (Number(color) > 255) {{
                    return cssColorFromXtermRgbInt(color);
                }}
            }} catch (_error) {{}}
            return '';
        }}
        function readCursorCellBackgroundFromXtermBuffer() {{
            try {{
                const activeBuffer = term && term.buffer ? term.buffer.active : null;
                if (!activeBuffer || typeof activeBuffer.getLine !== 'function') {{
                    return '';
                }}
                const cursorX = Math.max(0, Number(activeBuffer.cursorX || 0));
                const cursorY = Math.max(0, Number(activeBuffer.cursorY || 0));
                const lineIndex = Math.max(0, Number(activeBuffer.baseY || 0) + cursorY);
                const line = activeBuffer.getLine(lineIndex);
                if (!line || typeof line.getCell !== 'function') {{
                    return '';
                }}
                const cell = line.getCell(cursorX);
                if (!cell) {{
                    return '';
                }}
                const inverse = typeof cell.isInverse === 'function' && cell.isInverse();
                const color = inverse
                    ? cssColorFromXtermCellPlane(cell, 'fg')
                    : cssColorFromXtermCellPlane(cell, 'bg');
                return String(color || '').trim();
            }} catch (_error) {{
                return '';
            }}
        }}
        function readCursorCellBackgroundFromDom() {{
            try {{
                const cursor = host.querySelector('.xterm-cursor');
                if (!cursor) {{
                    return '';
                }}
                const cursorRect = cursor.getBoundingClientRect();
                const rowsLayer = host.querySelector('.xterm-rows');
                const row = rowsLayer
                    ? Array.from(rowsLayer.children || []).find((candidate) => {{
                        const rect = candidate.getBoundingClientRect();
                        return cursorRect.top >= rect.top - 1 && cursorRect.top < rect.bottom + 1;
                    }})
                    : null;
                if (!row) {{
                    return '';
                }}
                const overlapping = Array.from(row.querySelectorAll('span:not(.xterm-cursor)'))
                    .map((node) => {{
                        const rect = node.getBoundingClientRect();
                        return {{ node, rect }};
                    }})
                    .filter((sample) => {{
                        return cursorRect.left < sample.rect.right + 1
                            && cursorRect.right > sample.rect.left - 1;
                    }});
                for (const sample of overlapping) {{
                    const style = window.getComputedStyle(sample.node);
                    const background = String(style.backgroundColor || '').trim();
                    if (background && !isTransparentCssColor(background)) {{
                        return background;
                    }}
                }}
                const rowBackground = Array.from(row.querySelectorAll('span:not(.xterm-cursor)'))
                    .map((node) => String(window.getComputedStyle(node).backgroundColor || '').trim())
                    .find((background) => background && !isTransparentCssColor(background));
                return String(rowBackground || '').trim();
            }} catch (_error) {{
                return '';
            }}
        }}
        function refreshCursorCellBackgroundContract() {{
            let background = '';
            let source = 'transparent';
            try {{
                background = readCursorCellBackgroundFromDom();
                if (background && !isTransparentCssColor(background)) {{
                    source = 'xterm-dom-cell';
                }} else {{
                    background = readCursorCellBackgroundFromXtermBuffer();
                    if (background && !isTransparentCssColor(background)) {{
                        source = 'xterm-buffer-cell';
                    }} else {{
                        background = 'transparent';
                    }}
                }}
                host.style.setProperty('--yggterm-term-cursor-cell-background', background);
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.cursorCellBackground = background;
                    entry.cursorCellBackgroundSource = source;
                }}
                return {{ background, source }};
            }} catch (_error) {{
                host.style.setProperty('--yggterm-term-cursor-cell-background', 'transparent');
                return {{ background: 'transparent', source: 'error' }};
            }}
        }}
        function refreshCursorContrastContract() {{
            try {{
                return refreshCursorCellBackgroundContract();
            }} catch (_error) {{
                return {{ background: 'transparent', source: 'error' }};
            }} finally {{
                try {{
                    host.style.setProperty('--yggterm-term-cursor-block-text', computeFocusedBlockCursorTextColor());
                }} catch (_error) {{}}
            }}
        }}
        let cursorCellBackgroundRefreshPending = false;
        function scheduleCursorCellBackgroundRefresh(reason = 'render', attempt = 0) {{
            if (cursorCellBackgroundRefreshPending) {{
                return;
            }}
            cursorCellBackgroundRefreshPending = true;
            window.requestAnimationFrame(() => {{
                cursorCellBackgroundRefreshPending = false;
                const result = refreshCursorContrastContract();
                try {{
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId]
                        : null;
                    if (entry) {{
                        entry.lastCursorCellBackgroundRefreshReason = reason;
                        entry.lastCursorCellBackgroundRefreshAtMs = Date.now();
                    }}
                }} catch (_error) {{}}
                try {{
                    const source = result && result.source ? String(result.source) : '';
                    if (source === 'transparent' && attempt < 5 && host.querySelector('.xterm-rows')) {{
                        window.setTimeout(
                            () => scheduleCursorCellBackgroundRefresh(`${{reason}}:retry`, attempt + 1),
                            24 + attempt * 24
                        );
                    }}
                }} catch (_error) {{}}
            }});
        }}
        function enforceHelperTextareaContract() {{
            try {{
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                if (!helperTextarea) {{
                    return null;
                }}
                helperTextarea.tabIndex = 0;
                helperTextarea.removeAttribute('disabled');
                helperTextarea.style.setProperty('position', 'absolute', 'important');
                helperTextarea.style.setProperty('top', '0px', 'important');
                helperTextarea.style.setProperty('left', '-10000px', 'important');
                helperTextarea.style.setProperty('width', '1px', 'important');
                helperTextarea.style.setProperty('height', '1px', 'important');
                helperTextarea.style.setProperty('opacity', '0', 'important');
                helperTextarea.style.setProperty('z-index', '-5', 'important');
                helperTextarea.style.setProperty('overflow', 'hidden', 'important');
                helperTextarea.style.setProperty('border', '0', 'important');
                helperTextarea.style.setProperty('outline', 'none', 'important');
                helperTextarea.style.setProperty('box-shadow', 'none', 'important');
                helperTextarea.style.setProperty('background', 'transparent', 'important');
                helperTextarea.style.setProperty('color', 'transparent', 'important');
                helperTextarea.style.setProperty('caret-color', 'transparent', 'important');
                helperTextarea.style.setProperty('pointer-events', 'none', 'important');
                helperTextarea.style.setProperty('clip', 'rect(0, 0, 0, 0)', 'important');
                helperTextarea.style.setProperty('clip-path', 'inset(50%)', 'important');
                helperTextarea.style.setProperty('white-space', 'nowrap', 'important');
                helperTextarea.style.setProperty('appearance', 'none', 'important');
                return helperTextarea;
            }} catch (_error) {{
                return null;
            }}
        }}
        function ensureFocusCaptureOverlay() {{
            try {{
                let overlay = host.querySelector('.yggterm-term-focus-capture');
                if (!overlay) {{
                    overlay = document.createElement('div');
                    overlay.className = 'yggterm-term-focus-capture';
                    overlay.setAttribute('aria-hidden', 'true');
                    overlay.style.position = 'absolute';
                    overlay.style.inset = '0';
                    overlay.style.zIndex = '3';
                    overlay.style.background = 'transparent';
                    overlay.style.pointerEvents = 'none';
                    overlay.style.cursor = inputEnabled ? 'text' : 'default';
                    overlay.addEventListener('pointerdown', handleHostPointerFocus, true);
                    overlay.addEventListener('mousedown', handleHostPointerFocus, true);
                    overlay.addEventListener('pointerup', retainTerminalFocusAfterPointerRelease, true);
                    overlay.addEventListener('mouseup', retainTerminalFocusAfterPointerRelease, true);
                    overlay.addEventListener('click', retainTerminalFocusAfterPointerRelease, true);
                    host.appendChild(overlay);
                }}
                // The overlay is an observability/focus anchor only. It must never
                // become the hit target for xterm gestures, because selection,
                // double-click, and drag semantics belong to xterm.js.
                overlay.style.pointerEvents = 'none';
                overlay.setAttribute('data-yggterm-focus-capture-pointer-events', 'none');
                overlay.style.cursor = inputEnabled ? 'text' : 'default';
                return overlay;
            }} catch (_error) {{
                return null;
            }}
        }}
        let lastLowContrastNormalizeAtMs = 0;
        let lastLowContrastNormalizeKey = '';
        function normalizeLowContrastGlyphs(force = false) {{
            try {{
                const rowsLayer = host.querySelector('.xterm-rows');
                if (!rowsLayer) {{
                    return;
                }}
                const screen = host.querySelector('.xterm-screen');
                const viewport = host.querySelector('.xterm-viewport');
                const hostStyle = window.getComputedStyle(host);
                const screenStyle = screen ? window.getComputedStyle(screen) : null;
                const viewportStyle = viewport ? window.getComputedStyle(viewport) : null;
                const backgroundColor =
                    (viewportStyle && viewportStyle.backgroundColor)
                    || (screenStyle && screenStyle.backgroundColor)
                    || String(hostStyle.backgroundColor || '');
                const normalizeKey = JSON.stringify([
                    rowsLayer.childElementCount,
                    backgroundColor,
                    String(hostStyle.getPropertyValue('--yggterm-term-foreground') || ''),
                    String(hostStyle.getPropertyValue('--yggterm-term-dim-foreground') || ''),
                ]);
                const now = (window.performance && window.performance.now)
                    ? window.performance.now()
                    : Date.now();
                if (!force && now - lastLowContrastNormalizeAtMs < 250) {{
                    return;
                }}
                if (!force && normalizeKey === lastLowContrastNormalizeKey && now - lastLowContrastNormalizeAtMs < 1000) {{
                    return;
                }}
                lastLowContrastNormalizeAtMs = now;
                lastLowContrastNormalizeKey = normalizeKey;
                const background = parseCssColor(backgroundColor);
                const backgroundLuminance = background ? relativeLuminance(background) : null;
                const minimumVisibleContrast = 4.5;
                const minimumDimContrast = backgroundLuminance != null && backgroundLuminance > 0.72
                    ? 10.0
                    : 3.5;
                const contrastSafeForeground = (sampleBackground) => {{
                    const parsed = parseCssColor(sampleBackground);
                    const luminance = parsed ? relativeLuminance(parsed) : null;
                    return luminance != null && luminance > 0.46 ? '#0f172a' : '#fbfbfd';
                }};
                for (const node of Array.from(rowsLayer.querySelectorAll('span, div'))) {{
                    const className = String(node.className || '');
                    if (className.includes('xterm-cursor')) {{
                        continue;
                    }}
                    const text = String(node.textContent || '').replace(/\s+/g, ' ').trim();
                    if (!text) {{
                        continue;
                    }}
                    const style = window.getComputedStyle(node);
                    const rowBackground = !isTransparentCssColor(style.backgroundColor)
                        ? String(style.backgroundColor || '')
                        : backgroundColor;
                    const contrast = contrastRatio(style.color, rowBackground);
                    const minimumNodeContrast = className.includes('xterm-dim')
                        ? minimumDimContrast
                        : minimumVisibleContrast;
                    if (contrast == null || contrast >= minimumNodeContrast) {{
                        continue;
                    }}
                    node.style.setProperty('color', contrastSafeForeground(rowBackground), 'important');
                }}
            }} catch (_error) {{}}
        }}
        let wheelEventCount = 0;
        let scrollEventCount = 0;
        let dataEventCount = 0;
        let readNudgeCount = 0;
        let renderEventCount = 0;
        let lastRenderProbeAtMs = 0;
        let renderProbeFramePending = false;
        let retainedWritePaintRepairCount = 0;
        let retainedWritePaintRepairPending = false;
        let lastResizeKey = '';
        let resizeFramePending = false;
        let pendingResizeNotify = null;
        let resizeNotifyTimer = null;
        let lastResizeNotifyAtMs = 0;
        let settledResizePaintTimer = null;
        let settledResizeFollowupTimer = null;
        let lastWriteAppliedSampleAtMs = 0;
        let lastWriteFlushStartedAtMs = 0;
        let writeBridgeFlushTimer = null;
        let hostHealthFramePending = false;
        let hostHealthAfterFrameTimer = null;
        let lastHostHealthAtMs = 0;
        let lastInputHotHostHealthAtMs = 0;
        let lastFrameLikeHostHealthAtMs = 0;
        let hotHostHealthSuppressedCount = 0;
        let manualRedrawCount = 0;
        // Fail-pattern detector state: recent redraw (ts+reason) ring + a pending
        // anomaly JSON that the next HostHealth carries to the Rust trace.
        let recentRedrawEvents = [];
        let pendingRenderAnomaly = '';
        // XTERM-BUG: webgl-stale-atlas-garble — stale-atlas paint detector state.
        // See docs/xterm-bugs.md#webgl-stale-atlas-garble. The garble condition is
        // "render lands right after a >1s rAF-throttle gap and the glyph atlas was
        // last cleared BEFORE the gap began" — that is detectable even though the
        // garbled pixels themselves are not (ink sampling sees a full canvas).
        let lastAtlasClearAtMs = 0;
        let lastStaleAtlasHealGapEndMs = 0;
        let staleAtlasHealCount = 0;
        // XTERM-BUG: blank-rendering-region — per-row glyph-gap detector state.
        // Rows whose BUFFER holds text but whose text-layer pixels hold no ink
        // are the partial variant of canvas_blank_with_buffer_text (a blank
        // band / dropped glyphs inside an otherwise painted viewport).
        let lastGlyphGapScanAtMs = 0;
        let lastGlyphGapHealAtMs = 0;
        let glyphGapHealCount = 0;
        let renderHealthRecoveryCount = 0;
        let lastRenderHealthRecoveryAtMs = 0;
        // Escalating cooldown between render-health recovery repaints. A canvas
        // that keeps re-blanking (compositor-side, heal does not stick) used to
        // re-repaint on a fixed 2s cadence indefinitely — each repaint clears
        // the glyph atlas and refreshes every row, which reads as a CPU storm.
        let renderHealthRecoveryBackoffMs = 2000;
        let lastRenderHealthCheckedAtMs = 0;
        let renderHealthStatus = 'unknown';
        let renderHealthReason = '';
        let renderHealthRecoveryPending = false;
        let rendererSurfaceRecoveryCount = 0;
        let lastRendererSurfaceRecoveryReason = '';
        let lastRendererSurfaceRecoveryAtMs = 0;
        let lastRendererSurfaceRecoveryResult = '';
        let lastPerfEventAtMs = 0;
        let skippedPerfEventCount = 0;
        let terminalInputHotUntilMs = 0;
        let forcedRefreshCount = 0;
        let forcedAtlasClearCount = 0;
        let forcedRefreshSkippedCount = 0;
        let scrollbackLocked = false;
        let scrollbackIntent = 'PromptFollow';
        // Does a SELECTION currently own the viewport pin?
        // ⛔ Read by the reached-bottom release below, which otherwise drops ANY
        // UserScrollback pin the instant the viewport sits at the base — a
        // condition that is CONTINUOUSLY true while an agent CLI streams. Traced
        // on a shadow during a drag over a streaming session: the pin armed on
        // mouse-down (`UserScrollback/selection_active`) and was gone 116 ms
        // later (`PromptFollow/focus`, then `write_flush_reached_bottom`), after
        // which the viewport resumed following the stream and dragged the
        // selection's end anchor through the buffer with it — 902,650 chars from
        // a 2.4 s drag. Declared HERE, beside the intent it guards, so the
        // release site can read it with no temporal-dead-zone risk.
        let selectionOwnsScrollbackPin = false;
        // Working-session-cluster follow fix (finding-working-state-row-overlap):
        // `programmaticScrollInProgress` is set synchronously around every
        // forceXtermViewportY move so the onScroll it fires is not mistaken for a
        // user scroll-up. `lastObservedScrollYdisp` tracks the last viewport ydisp
        // so syncScrollbackLock can detect a user scroll-up as a NON-programmatic
        // DECREASE (the harness-locked signal) instead of relying on output-activity
        // suppression that also swallowed genuine scroll-ups during output (defect #1).
        let programmaticScrollInProgress = false;
        let lastObservedScrollYdisp = 0;
        // bg→fg stuck-at-top hardening: a USER scroll-up never DECREASES baseY,
        // while the programmatic movers the flag can miss (reset/clear reseed,
        // row-growth fit/reflow clamp on focus-regain) drop baseY together with
        // ydisp. Tracking baseY lets the detector reject that whole class
        // (scroll_mode.rs user_scroll_up_detected is the tested oracle).
        let lastObservedScrollBaseY = 0;
        // A.b.3 async-scrollTop gap: `syncXtermViewportElementToBuffer` writes
        // `.xterm-viewport.scrollTop` directly and the resulting scroll event is
        // delivered ASYNCHRONOUSLY, after the synchronous
        // `programmaticScrollInProgress` window has closed — the flag mechanism
        // structurally cannot cover it (5-sweep dataset: 1/7 phantom
        // UserScrollback flips via `scroll_event`). The mover records its
        // expected landing row here; the detector treats a scroll event landing
        // on that row (±1, within 1.5s) as programmatic and consumes the latch.
        let pendingProgrammaticViewportTargetY = null;
        let pendingProgrammaticViewportAtMs = 0;
        let lastScrollbackIntentReason = 'initial';
        let lastScrollbackIntentAtMs = Date.now();
        let promptFollowScrollGuardUntilMs = 0;
        let promptFollowSchedulePending = false;
        let promptFollowScheduleReason = '';
        let promptFollowScheduleAtMs = 0;
        let promptFollowScheduleSkipCount = 0;
        let promptFollowScheduleCancelToken = 0;
        let lastPromptFollowScheduleSkipReason = '';
        let lastScrollbackSnapbackReason = '';
        let syncTerminalScrollController = (_reason = '') => {{}};
        let lowPowerTuiOverlay = null;
        let lowPowerTuiActive = false;
        let lowPowerTuiFrameCount = 0;
        let lowPowerTuiLastText = '';
        let lowPowerTuiTextBuffer = '';
        let backgroundTuiSuppressActive = false;
        let tracedLowPowerTuiActive = false;
        let tracedInactiveTuiDrop = false;
        let tracedTuiFilterProbe = false;
        let inactiveTuiFrameDropCount = 0;
        let inactiveTuiLastTail = '';
        let unfocusedTuiFrameDropCount = 0;
        let unfocusedTuiLastTail = '';
        let tracedUnfocusedTuiDrop = false;
        let bufferTransitionCount = 0;
        let cursorHiddenToggleCount = 0;
        let lastObservedBufferKind = null;
        let lastObservedCursorHidden = null;
        let lastVisualTransitionReason = '';
        const coreService = () => {{
            try {{
                return term && term._core
                    ? (term._core._coreService || term._core.coreService || null)
                    : null;
            }} catch (_error) {{
                return null;
            }}
        }};
        const terminalCursorState = () => {{
            try {{
                const service = coreService();
                const focused = Boolean(term && term.textarea && document.activeElement === term.textarea);
                return {{
                    hidden: Boolean(service && service.isCursorHidden),
                    initialized: Boolean(service && service.isCursorInitialized),
                    focused,
                }};
            }} catch (_error) {{
                return {{ hidden: false, initialized: false, focused: false }};
            }}
        }};
        const terminalPayloadDebugSample = (payload) => {{
            try {{
                return String(payload || '')
                    .slice(-4096)
                    .replace(/\x1b/g, '\\x1b')
                    .replace(/\r/g, '\\r')
                    .replace(/\n/g, '\\n')
                    .replace(/\t/g, '\\t');
            }} catch (_error) {{
                return '';
            }}
        }};
        // SSOT for "read the rendered xterm buffer into transcript text". Used by
        // both the in-memory snapshot capture AND the localStorage persist (the
        // screen-restore vacuum fix) so the persist no longer depends on a prior
        // snapshot capture having run — it serializes term.buffer directly.
        const serializeTerminalBufferText = () => {{
            try {{
                const buffer = term && term.buffer && term.buffer.active ? term.buffer.active : null;
                if (!buffer || typeof buffer.getLine !== "function") {{
                    return null;
                }}
                const length = Math.max(0, Number(buffer.length || 0));
                const rows = Math.max(1, Number(term.rows || 1));
                const maxRows = Math.min(length, Math.max(300, rows * 8));
                const start = Math.max(0, length - maxRows);
                const visualLines = [];
                const logicalLines = [];
                for (let row = start; row < length; row += 1) {{
                    const line = buffer.getLine(row);
                    const lineText = line && typeof line.translateToString === "function"
                        ? String(line.translateToString(true) || "")
                        : "";
                    visualLines.push(lineText);
                    if (logicalLines.length > 0 && line && line.isWrapped) {{
                        logicalLines[logicalLines.length - 1] += lineText;
                    }} else {{
                        logicalLines.push(lineText);
                    }}
                }}
                const text = logicalLines.join("\r\n");
                const nonblankLineCount = logicalLines
                    .filter((line) => String(line || "").trim().length > 0)
                    .length;
                return {{
                    text,
                    visualLineCount: visualLines.length,
                    logicalLineCount: logicalLines.length,
                    nonblankLineCount,
                }};
            }} catch (_error) {{
                return null;
            }}
        }};
        const captureSessionXtermSnapshot = (reason = '') => {{
            try {{
                const sessionPath = currentHostSessionPath();
                const buffer = term && term.buffer && term.buffer.active ? term.buffer.active : null;
                if (!sessionPath || !buffer || typeof buffer.getLine !== "function") {{
                    return null;
                }}
                const rows = Math.max(1, Number(term.rows || 1));
                const cols = Math.max(1, Number(term.cols || 1));
                const serialized = serializeTerminalBufferText();
                if (!serialized) {{
                    return null;
                }}
                const text = serialized.text;
                const visualLineCount = serialized.visualLineCount;
                const logicalLineCount = serialized.logicalLineCount;
                const nonblankLineCount = serialized.nonblankLineCount;
                if (!text.trim() || nonblankLineCount <= 0) {{
                    return null;
                }}
                window.__yggtermXtermSessionSnapshots = window.__yggtermXtermSessionSnapshots || {{}};
                // XTERM-BUG: blank-viewport-client-snapshot-poison
                // Do NOT let a collapsed/near-blank frame overwrite a good cached
                // snapshot. A blank codex/TUI frame still has >=1 nonblank cell
                // (composer border / "›"), so the nonblankLineCount<=0 guard above
                // is not enough: a 1-nonblank-line frame would be cached and then
                // restored on hot-reveal, showing blank and self-perpetuating
                // (blank begets blank). Track each session's historical nonblank
                // max; if a new frame collapses to <=1 nonblank line for a session
                // that previously had real content (max>=6), keep the prior good
                // snapshot instead. The daemon authoritative replay (the source of
                // truth) reconciles the displayed content; this only stops the
                // client cache from latching a poison frame. A legitimately cleared
                // session (daemon also blank) is corrected by daemon reconciliation.
                window.__yggtermXtermSessionNonblankMax = window.__yggtermXtermSessionNonblankMax || {{}};
                const priorNonblankMax = Math.max(
                    0,
                    Number(window.__yggtermXtermSessionNonblankMax[sessionPath] || 0)
                );
                const collapsedPoisonFrame = nonblankLineCount <= 1 && priorNonblankMax >= 6;
                if (collapsedPoisonFrame) {{
                    const priorSnapshot = window.__yggtermXtermSessionSnapshots[sessionPath] || null;
                    const priorEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId]
                        : null;
                    if (priorEntry) {{
                        priorEntry.lastXtermSessionSnapshotCollapsedSkipReason =
                            `collapsed_poison_frame:nonblank_${{nonblankLineCount}}_prior_max_${{priorNonblankMax}}`;
                        priorEntry.lastXtermSessionSnapshotCollapsedSkipAtMs = Date.now();
                    }}
                    sendTerminalEvent({{
                        kind: 'debug',
                        message: `xterm_session_snapshot_collapsed_skip host=${{hostId}} nonblank=${{nonblankLineCount}} prior_max=${{priorNonblankMax}} reason=${{String(reason || '')}}`
                    }});
                    // Return the retained good snapshot (if any) so callers still
                    // observe a non-poison frame; do not overwrite the cache.
                    return priorSnapshot;
                }}
                window.__yggtermXtermSessionNonblankMax[sessionPath] = Math.max(
                    priorNonblankMax,
                    nonblankLineCount
                );
                const snapshot = {{
                    sessionPath,
                    hostId,
                    reason: String(reason || ''),
                    capturedAtMs: Date.now(),
                    rows,
                    cols,
                    baseY: Math.max(0, Number(buffer.baseY || 0)),
                    viewportY: Math.max(0, Number(buffer.viewportY || 0)),
                    cursorY: Math.max(0, Number(buffer.cursorY || 0)),
                    cursorX: Math.max(0, Number(buffer.cursorX || 0)),
                    scrollbackIntent,
                    lastScrollbackIntentReason,
                    scrollbackLocked: Boolean(scrollbackLocked),
                    lineCount: visualLineCount,
                    logicalLineCount,
                    nonblankLineCount,
                    text,
                    textTail: text.slice(-4096),
                }};
                window.__yggtermXtermSessionSnapshots[sessionPath] = snapshot;
                const snapshotKeys = Object.keys(window.__yggtermXtermSessionSnapshots);
                if (snapshotKeys.length > 80) {{
                    snapshotKeys
                        .map((key) => [key, Number(window.__yggtermXtermSessionSnapshots[key].capturedAtMs || 0)])
                        .sort((left, right) => left[1] - right[1])
                        .slice(0, snapshotKeys.length - 80)
                        .forEach(([key]) => {{
                            try {{
                                delete window.__yggtermXtermSessionSnapshots[key];
                                if (window.__yggtermXtermSessionNonblankMax) {{
                                    delete window.__yggtermXtermSessionNonblankMax[key];
                                }}
                            }} catch (_error) {{}}
                        }});
                }}
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.lastXtermSessionSnapshotReason = snapshot.reason;
                    entry.lastXtermSessionSnapshotAtMs = snapshot.capturedAtMs;
                    entry.lastXtermSessionSnapshotLineCount = snapshot.lineCount;
                    entry.lastXtermSessionSnapshotNonblankLineCount = snapshot.nonblankLineCount;
                    entry.lastXtermSessionSnapshotBaseY = snapshot.baseY;
                    entry.lastXtermSessionSnapshotViewportY = snapshot.viewportY;
                }}
                // Skip persist while restore-from-localStorage is in flight.
                const _restoreInFlight = Boolean(pendingPersistedScrollRestore)
                    || (pendingPersistedScrollRestoreDeadlineMs > 0
                        && Date.now() <= pendingPersistedScrollRestoreDeadlineMs);
                if (!_restoreInFlight) {{
                    persistScrollStateToLocalStorage(`snapshot:${{snapshot.reason || ''}}`);
                }}
                return snapshot;
            }} catch (_error) {{
                return null;
            }}
        }};
        let softwareCanvasLayerOptimizationActive = false;
        let softwareCanvasHiddenLayerCount = 0;
        let softwareCanvasVisibleLayerCount = 0;
        let softwareCanvasInputLineOverlay = null;
        let softwareCanvasInputLineOverlayVisible = false;
        let softwareCanvasCursorOverlay = null;
        let softwareCanvasCursorOverlayVisible = false;
        let softwareCanvasLinkRevealUntilMs = 0;
        let softwareCanvasLinkRevealTimer = null;
        let lastSoftwareCanvasLayerOptimizationReason = '';
        // RETIRED for xterm.js 6 / WebGL: this optimization hid redundant layers of
        // the OLD @xterm/addon-canvas multi-canvas (2D) model. WebGL renders to a
        // single GPU canvas with no redundant layers, so the optimization is a no-op
        // and must NEVER hide the WebGL canvas. (Native-surface cleanup per the
        // "retire un-needed harnesses" directive.)
        const softwareCanvasLayerOptimizationAllowed = () => false;
        const canvasLayerRole = (canvas) => {{
            try {{
                const className = String(canvas && canvas.className ? canvas.className : '');
                if (className.includes('xterm-text-layer')) {{
                    return 'text';
                }}
                if (className.includes('xterm-selection-layer')) {{
                    return 'selection';
                }}
                if (className.includes('xterm-link-layer')) {{
                    return 'link';
                }}
                if (className.includes('xterm-cursor-layer')) {{
                    return 'cursor';
                }}
            }} catch (_error) {{}}
            return '';
        }};
        const syncSoftwareCanvasLayerOptimizationHostEntry = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return;
                }}
                entry.softwareCanvasLayerOptimizationActive = Boolean(softwareCanvasLayerOptimizationActive);
                entry.softwareCanvasHiddenLayerCount = Number(softwareCanvasHiddenLayerCount || 0);
                entry.softwareCanvasVisibleLayerCount = Number(softwareCanvasVisibleLayerCount || 0);
                entry.softwareCanvasInputLineOverlayVisible = Boolean(softwareCanvasInputLineOverlayVisible);
                entry.softwareCanvasCursorOverlayVisible = Boolean(softwareCanvasCursorOverlayVisible);
                entry.lastSoftwareCanvasLayerOptimizationReason = String(lastSoftwareCanvasLayerOptimizationReason || '');
            }} catch (_error) {{}}
        }};
        const removeSoftwareCanvasOverlays = () => {{
            try {{
                const inputOverlay = softwareCanvasInputLineOverlay || host.querySelector('[data-yggterm-canvas-input-line-overlay="1"]');
                if (inputOverlay && inputOverlay.remove) {{
                    inputOverlay.remove();
                }}
            }} catch (_error) {{}}
            try {{
                const cursorOverlay = softwareCanvasCursorOverlay || host.querySelector('[data-yggterm-canvas-cursor-overlay="1"]');
                if (cursorOverlay && cursorOverlay.remove) {{
                    cursorOverlay.remove();
                }}
            }} catch (_error) {{}}
            softwareCanvasInputLineOverlay = null;
            softwareCanvasInputLineOverlayVisible = false;
            softwareCanvasCursorOverlay = null;
            softwareCanvasCursorOverlayVisible = false;
            syncSoftwareCanvasLayerOptimizationHostEntry();
        }};
        const applySoftwareCanvasLayerOptimization = (reason = '') => {{
            try {{
                if (!softwareCanvasLayerOptimizationAllowed()) {{
                    softwareCanvasLayerOptimizationActive = false;
                    softwareCanvasHiddenLayerCount = 0;
                    softwareCanvasVisibleLayerCount = 0;
                    removeSoftwareCanvasOverlays();
                    syncSoftwareCanvasLayerOptimizationHostEntry();
                    return;
                }}
                lastSoftwareCanvasLayerOptimizationReason = String(reason || 'idle');
                const hasSelection = Boolean(term && typeof term.hasSelection === 'function' && term.hasSelection());
                const revealLinkLayer = Date.now() < Number(softwareCanvasLinkRevealUntilMs || 0);
                let hiddenCount = 0;
                let visibleCount = 0;
                for (const canvas of Array.from(host.querySelectorAll('.xterm-screen canvas'))) {{
                    const role = canvasLayerRole(canvas);
                    let shouldHide = false;
                    if (role === 'selection') {{
                        shouldHide = !hasSelection;
                    }} else if (role === 'link') {{
                        shouldHide = !revealLinkLayer;
                    }}
                    if (shouldHide) {{
                        canvas.style.display = 'none';
                        canvas.style.pointerEvents = 'none';
                        canvas.setAttribute('data-yggterm-software-canvas-hidden', 'true');
                        hiddenCount += 1;
                    }} else {{
                        if (canvas.getAttribute('data-yggterm-software-canvas-hidden') === 'true') {{
                            canvas.style.display = '';
                            canvas.removeAttribute('data-yggterm-software-canvas-hidden');
                        }}
                        visibleCount += 1;
                    }}
                }}
                softwareCanvasLayerOptimizationActive = true;
                softwareCanvasHiddenLayerCount = hiddenCount;
                softwareCanvasVisibleLayerCount = visibleCount;
                removeSoftwareCanvasOverlays();
                syncSoftwareCanvasLayerOptimizationHostEntry();
            }} catch (_error) {{
                syncSoftwareCanvasLayerOptimizationHostEntry();
            }}
        }};
        const revealSoftwareCanvasLinkLayer = (reason = 'pointer') => {{
            if (!softwareCanvasLayerOptimizationAllowed()) {{
                return;
            }}
            softwareCanvasLinkRevealUntilMs = Date.now() + 1200;
            applySoftwareCanvasLayerOptimization(reason);
            if (softwareCanvasLinkRevealTimer !== null) {{
                window.clearTimeout(softwareCanvasLinkRevealTimer);
            }}
            softwareCanvasLinkRevealTimer = window.setTimeout(() => {{
                softwareCanvasLinkRevealTimer = null;
                applySoftwareCanvasLayerOptimization('link_reveal_expired');
            }}, 1300);
        }};
        const syncCursorOverlay = () => {{
            syncFocusClass();
            applySoftwareCanvasLayerOptimization('cursor_sync');
        }};
        const syncHostScrollbackIntent = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return;
                }}
                entry.scrollbackIntent = scrollbackIntent;
                entry.lastScrollbackIntentReason = lastScrollbackIntentReason;
                entry.lastScrollbackIntentAtMs = lastScrollbackIntentAtMs;
                entry.lastScrollbackSnapbackReason = lastScrollbackSnapbackReason;
            }} catch (_error) {{}}
        }};
        // XTERM-BUG: scrollback-lost-on-gui-restart
        // localStorage persists scroll position across GUI process restarts.
        // In-memory __yggtermXtermSessionSnapshots covers within-session-switch;
        // localStorage covers GUI-restart (PTY history is still on daemon).
        // Key format: yggterm-scroll:<sessionPath>. Entries older than 24h are ignored on load.
        const persistedScrollKey = () => {{
            try {{
                const sessionPath = currentHostSessionPath();
                return sessionPath ? `yggterm-scroll:${{sessionPath}}` : '';
            }} catch (_error) {{ return ''; }}
        }};
        const persistScrollStateToLocalStorage = (reason) => {{
            try {{
                if (typeof window === 'undefined' || !window.localStorage) {{ return; }}
                const key = persistedScrollKey();
                if (!key) {{ return; }}
                const buffer = term && term.buffer && term.buffer.active ? term.buffer.active : null;
                const viewportY = buffer ? Math.max(0, Number(buffer.viewportY || 0)) : 0;
                const baseY = buffer ? Math.max(0, Number(buffer.baseY || 0)) : 0;
                // WS3 screen-restore (vacuum fix): also persist the buffer TEXT so a NEW GUI
                // process restores the transcript. localStorage survives a GUI restart; the
                // in-memory xtermSessionSnapshot does NOT — so a full GUI+daemon restart
                // (daemon re-resumes codex on a fresh PTY that doesn't re-print history)
                // otherwise leaves both text sources empty → vacuum. Reuse the latest
                // captured snapshot's already-serialized text (no re-serialize), capped for
                // the localStorage quota, only when it's a real (non-collapsed) frame.
                // Serialize the transcript DIRECTLY from term.buffer (not the
                // in-memory snapshot, which may never have been captured for an
                // idle freshly-opened session — the 2.8.54 failure mode). Only
                // persist a real (non-collapsed) frame; a 1-nonblank frame would
                // poison the restore. CRITICAL: never overwrite a previously-saved
                // RICH transcript with a much sparser current frame — during the
                // re-resume window the live buffer collapses to ~8 lines, and
                // persisting THAT would destroy the saved transcript and CAUSE the
                // vacuum this fix targets. Keep the prior text when the current
                // frame is a severe collapse (<1/3 of the saved nonblank count).
                let snapshotText = '';
                let snapshotLineCount = 0;
                let snapshotNonblankLineCount = 0;
                try {{
                    let prevText = ''; let prevLineCount = 0; let prevNonblank = 0;
                    try {{
                        const rawPrev = window.localStorage.getItem(key);
                        if (rawPrev) {{
                            const p = JSON.parse(rawPrev);
                            if (p && typeof p.text === 'string') {{
                                prevText = p.text;
                                prevLineCount = Number(p.lineCount || 0);
                                prevNonblank = Number(p.nonblankLineCount || 0);
                            }}
                        }}
                    }} catch (_prevError) {{}}
                    const serialized = serializeTerminalBufferText();
                    const curText = serialized && typeof serialized.text === 'string' ? serialized.text : '';
                    const curNonblank = serialized ? Number(serialized.nonblankLineCount || 0) : 0;
                    const curIsReal = curText.trim() && curNonblank > 1;
                    const curIsSevereCollapse = prevNonblank >= 6 && curNonblank * 3 < prevNonblank;
                    if (curIsReal && !curIsSevereCollapse) {{
                        // Cap per-session text so the localStorage origin quota
                        // (~5MB in WebKit) isn't blown across many session keys.
                        snapshotText = curText.length > 48000 ? curText.slice(-48000) : curText;
                        snapshotLineCount = serialized ? Number(serialized.visualLineCount || 0) : 0;
                        snapshotNonblankLineCount = curNonblank;
                    }} else if (prevText) {{
                        // Keep the previously-saved richer transcript intact.
                        snapshotText = prevText;
                        snapshotLineCount = prevLineCount;
                        snapshotNonblankLineCount = prevNonblank;
                    }}
                }} catch (_textError) {{}}
                const payload = JSON.stringify({{
                    intent: scrollbackIntent,
                    viewportY,
                    baseY,
                    distanceFromBottom: Math.max(0, baseY - viewportY),
                    locked: Boolean(scrollbackLocked),
                    reason: String(reason || ''),
                    savedAtMs: Date.now(),
                    text: snapshotText,
                    lineCount: snapshotLineCount,
                    nonblankLineCount: snapshotNonblankLineCount,
                }});
                try {{
                    window.localStorage.setItem(key, payload);
                }} catch (_quotaError) {{
                    // Quota exceeded (transcript text is the bulk): fall back to a
                    // scroll-only payload so scroll-position restore never regresses.
                    try {{
                        const scrollOnly = JSON.stringify({{
                            intent: scrollbackIntent,
                            viewportY,
                            baseY,
                            distanceFromBottom: Math.max(0, baseY - viewportY),
                            locked: Boolean(scrollbackLocked),
                            reason: String(reason || ''),
                            savedAtMs: Date.now(),
                            text: '',
                            lineCount: 0,
                            nonblankLineCount: 0,
                        }});
                        window.localStorage.setItem(key, scrollOnly);
                    }} catch (_scrollOnlyError) {{}}
                }}
            }} catch (_error) {{}}
        }};
        const loadScrollStateFromLocalStorage = () => {{
            try {{
                if (typeof window === 'undefined' || !window.localStorage) {{ return null; }}
                const key = persistedScrollKey();
                if (!key) {{ return null; }}
                const raw = window.localStorage.getItem(key);
                if (!raw) {{ return null; }}
                const state = JSON.parse(raw);
                if (!state || typeof state !== 'object') {{ return null; }}
                const savedAtMs = Number(state.savedAtMs || 0);
                if (!Number.isFinite(savedAtMs) || savedAtMs <= 0) {{ return null; }}
                const ageMs = Date.now() - savedAtMs;
                // 24h expiry: stale scroll positions become misleading after a day.
                if (ageMs < 0 || ageMs > 24 * 60 * 60 * 1000) {{
                    try {{ window.localStorage.removeItem(key); }} catch (_e) {{}}
                    return null;
                }}
                return {{
                    intent: state.intent === 'UserScrollback' ? 'UserScrollback' : 'PromptFollow',
                    viewportY: Math.max(0, Number(state.viewportY || 0)),
                    baseY: Math.max(0, Number(state.baseY || 0)),
                    distanceFromBottom: Math.max(0, Number(state.distanceFromBottom || 0)),
                    locked: Boolean(state.locked),
                    reason: String(state.reason || ''),
                    ageMs,
                    // WS3 screen-restore: persisted buffer text ('' for entries written
                    // before this field existed).
                    text: typeof state.text === 'string' ? state.text : '',
                    lineCount: Math.max(0, Number(state.lineCount || 0)),
                    nonblankLineCount: Math.max(0, Number(state.nonblankLineCount || 0)),
                }};
            }} catch (_error) {{ return null; }}
        }};
        let pendingPersistedScrollRestore = null;
        let pendingPersistedScrollRestoreDeadlineMs = 0;
        // Screen-restore part (b): the daemon `reset` command (theme setup, sent
        // once on attach) runs term.reset() AFTER the construct-time localStorage
        // transcript restore, wiping it — then the sparse fresh-PTY replay can't
        // refill it (vacuum). Stash the restored transcript here so the reset
        // handler re-applies it ONCE after term.reset()+theme; it becomes
        // scrollback, and the resumed CLI's `\x1b[H\x1b[J` clears only the
        // viewport, leaving the transcript scrollable above.
        let pendingPostResetTranscript = null;
        let lastScrollPersistAtMs = 0;
        const setScrollbackIntent = (intent, reason) => {{
            const next = intent === 'UserScrollback' ? 'UserScrollback' : 'PromptFollow';
            const nextReason = String(reason || 'unknown');
            if (scrollbackIntent === next && lastScrollbackIntentReason === nextReason) {{
                syncHostScrollbackIntent();
                return;
            }}
            scrollbackIntent = next;
            lastScrollbackIntentReason = nextReason;
            lastScrollbackIntentAtMs = Date.now();
            if (scrollbackIntent === 'UserScrollback') {{
                promptFollowScheduleCancelToken += 1;
                promptFollowSchedulePending = false;
                syncPromptFollowScheduleHostEntry();
            }}
            syncHostScrollbackIntent();
            // Skip persists while a pending restore is in flight (post-restart replay
            // would otherwise overwrite the user's saved spot with intermediate state).
            const restoreInFlight = Boolean(pendingPersistedScrollRestore)
                || (pendingPersistedScrollRestoreDeadlineMs > 0
                    && Date.now() <= pendingPersistedScrollRestoreDeadlineMs);
            if (!restoreInFlight) {{
                persistScrollStateToLocalStorage(`intent_change:${{nextReason}}`);
            }}
            sendTerminalEvent({{
                kind: "debug",
                message: `scrollback_intent host=${{hostId}} intent=${{scrollbackIntent}} reason=${{lastScrollbackIntentReason}}`
            }});
        }};
        const markTerminalInputHot = (reason = 'input') => {{
            terminalInputHotUntilMs = Math.max(
                terminalInputHotUntilMs,
                Date.now() + {terminal_input_hot_suppress_ms}
            );
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.terminalInputHotUntilMs = terminalInputHotUntilMs;
                    entry.lastTerminalInputHotReason = String(reason || 'input');
                }}
            }} catch (_error) {{}}
        }};
        const terminalInputHot = () => Date.now() < terminalInputHotUntilMs;
        const promptFollowLayoutGuardActive = () => Date.now() <= promptFollowScrollGuardUntilMs;
        const normalizedPromptFollowScheduleReason = (reason = 'layout') => {{
            const text = String(reason || 'layout');
            const separatorIndex = text.indexOf(':');
            return separatorIndex >= 0 ? text.slice(0, separatorIndex) : text;
        }};
        const syncPromptFollowScheduleHostEntry = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.promptFollowSchedulePending = Boolean(promptFollowSchedulePending);
                    entry.promptFollowScheduleReason = String(promptFollowScheduleReason || '');
                    entry.promptFollowScheduleAtMs = Number(promptFollowScheduleAtMs || 0);
                    entry.promptFollowScheduleSkipCount = Number(promptFollowScheduleSkipCount || 0);
                    entry.lastPromptFollowScheduleSkipReason = String(lastPromptFollowScheduleSkipReason || '');
                }}
            }} catch (_error) {{}}
        }};
        const armPromptFollowLayoutGuard = (reason = 'layout', durationMs = 540) => {{
            if (scrollbackIntent === 'UserScrollback') {{
                return false;
            }}
            const untilMs = Date.now() + Math.max(120, Number(durationMs) || 540);
            promptFollowScrollGuardUntilMs = Math.max(promptFollowScrollGuardUntilMs, untilMs);
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.promptFollowLayoutGuardUntilMs = promptFollowScrollGuardUntilMs;
                    entry.lastPromptFollowLayoutGuardReason = String(reason || 'layout');
                }}
            }} catch (_error) {{}}
            return true;
        }};
        const schedulePromptFollowAfterLayout = (reason = 'layout') => {{
            const scheduleReason = normalizedPromptFollowScheduleReason(reason);
            if (!armPromptFollowLayoutGuard(scheduleReason, 760)) {{
                return false;
            }}
            const now = Date.now();
            if (
                promptFollowSchedulePending
                && promptFollowScheduleReason === scheduleReason
                && now - promptFollowScheduleAtMs < 420
            ) {{
                promptFollowScheduleSkipCount += 1;
                lastPromptFollowScheduleSkipReason = scheduleReason;
                syncPromptFollowScheduleHostEntry();
                return false;
            }}
            promptFollowSchedulePending = true;
            promptFollowScheduleReason = scheduleReason;
            promptFollowScheduleAtMs = now;
            const scheduleCancelToken = promptFollowScheduleCancelToken;
            syncPromptFollowScheduleHostEntry();
            const follow = (phase) => {{
                try {{
                    if (scheduleCancelToken !== promptFollowScheduleCancelToken) {{
                        return;
                    }}
                    if (scrollbackIntent !== 'UserScrollback') {{
                        scrollLiveCursorIntoView(true, `${{scheduleReason}}:${{phase}}`);
                    }}
                }} catch (_error) {{}}
            }};
            const clearPending = () => {{
                if (scheduleCancelToken !== promptFollowScheduleCancelToken) {{
                    return;
                }}
                if (promptFollowScheduleReason === scheduleReason) {{
                    promptFollowSchedulePending = false;
                    syncPromptFollowScheduleHostEntry();
                }}
            }};
            follow('now');
            window.requestAnimationFrame(() => follow('raf'));
            window.setTimeout(() => follow('32ms'), 32);
            window.setTimeout(() => follow('140ms'), 140);
            window.setTimeout(() => {{
                follow('320ms');
                clearPending();
            }}, 320);
            window.setTimeout(clearPending, 460);
            return true;
        }};
        const syncScrollbackLock = (reason = '') => {{
            try {{
                if (!term || !term.buffer || !term.buffer.active) {{
                    scrollbackLocked = false;
                }} else {{
                    const active = term.buffer.active;
                    const viewportY = effectiveXtermViewportY(active);
                    const publicViewportY = Math.max(0, Number(active.viewportY || 0));
                    const baseY = Math.max(0, Number(active.baseY || 0));
                    // User scroll-up detection (working-session cluster fix,
                    // finding-working-state-row-overlap): a genuine user scroll-up is
                    // a NON-programmatic DECREASE of the viewport ydisp. Harness-locked
                    // (tools/xterm-harness/scroll_follow_probe.test.js): output NEVER
                    // decreases ydisp (it auto-follows up or leaves ydisp unchanged), so
                    // a non-programmatic decrease uniquely identifies a real scroll-up,
                    // across ALL gestures, and it fires EVEN DURING OUTPUT — replacing
                    // the old write-bridge/input-hot suppression that swallowed genuine
                    // scroll-ups while streaming (defect #1). A passive burst-strand
                    // (ydisp UNCHANGED while baseY grows) is NOT a scroll-up, so it stays
                    // PromptFollow and the flush re-follows it instead of stranding.
                    // A.b.3: a scroll event that lands on the row a recent
                    // direct scrollTop write targeted is the ASYNC delivery of
                    // that programmatic move, not a user gesture. Consume the
                    // latch so a real user scroll right after is still seen.
                    const asyncProgrammaticScrollMatch =
                        reason === 'scroll_event'
                        && pendingProgrammaticViewportTargetY !== null
                        && Math.abs(viewportY - pendingProgrammaticViewportTargetY) <= 1
                        && (Date.now() - pendingProgrammaticViewportAtMs) <= 1500;
                    if (asyncProgrammaticScrollMatch) {{
                        pendingProgrammaticViewportTargetY = null;
                    }}
                    const userScrolledUp =
                        reason === 'scroll_event'
                        && !programmaticScrollInProgress
                        && !asyncProgrammaticScrollMatch
                        && !promptFollowLayoutGuardActive()
                        && viewportY + 0.5 < lastObservedScrollYdisp
                        && baseY + 0.5 >= lastObservedScrollBaseY;
                    const promptFollowVisualMismatchAtBottom =
                        scrollbackIntent !== 'UserScrollback'
                        && publicViewportY + 0.5 >= baseY
                        && viewportY + 0.5 < baseY;
                    scrollbackLocked = promptFollowVisualMismatchAtBottom
                        ? false
                        : viewportY < baseY;
                    if (promptFollowVisualMismatchAtBottom) {{
                        const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                            ? window.__yggtermXtermHosts[hostId]
                            : null;
                        if (entry) {{
                            entry.scrollbackVisualMismatchAtBottomCount =
                                Number(entry.scrollbackVisualMismatchAtBottomCount || 0) + 1;
                            entry.lastScrollbackVisualMismatchAtBottomReason = String(reason || '');
                        }}
                    }}
                    // ⛔ `!selectionOwnsScrollbackPin`: "the viewport reached the
                    // bottom" is the documented escape from a WHEEL pin, and it
                    // is right for one. For a SELECTION pin it is fatal — during
                    // a drag at the tail of a streaming session the viewport is
                    // at the bottom on every write flush, so this line used to
                    // un-pin mid-gesture and let the selection chase the stream.
                    if (!scrollbackLocked && scrollbackIntent === 'UserScrollback'
                        && !selectionOwnsScrollbackPin) {{
                        setScrollbackIntent('PromptFollow', reason ? `${{reason}}_reached_bottom` : 'reached_bottom');
                }} else if (
                    userScrolledUp
                    && scrollbackLocked
                    && scrollbackIntent !== 'UserScrollback'
                ) {{
                    setScrollbackIntent('UserScrollback', 'scroll_event');
                }}
                    // Track the latest ydisp so the NEXT scroll event can compare
                    // direction (decrease = user scroll-up vs unchanged = passive strand).
                    lastObservedScrollYdisp = viewportY;
                    lastObservedScrollBaseY = baseY;
                }}
            }} catch (_error) {{
                scrollbackLocked = false;
            }}
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].scrollbackLocked = scrollbackLocked;
            }}
            syncHostScrollbackIntent();
            syncTerminalScrollController(reason || 'scrollback_lock');
            return scrollbackLocked;
        }};
        const emitPerf = (name, payload = {{}}) => {{
            try {{
                const eventName = String(name || "terminal_event");
                const now = Date.now();
                const hotHighFrequencyEvent =
                    eventName === "xterm_write_flush"
                    || eventName === "xterm_fit"
                    || eventName === "xterm_forced_refresh"
                    || eventName === "xterm_forced_refresh_skipped";
                const frameLikeHot = recentFrameLikeWriteHot();
                const highFrequencyHot = terminalInputHot() || frameLikeHot;
                const minPerfIntervalMs = frameLikeHot
                    ? terminalFrameLikeInstrumentationThrottleMs()
                    : 900;
                if (
                    highFrequencyHot
                    && hotHighFrequencyEvent
                    && now - lastPerfEventAtMs < minPerfIntervalMs
                ) {{
                    skippedPerfEventCount += 1;
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId]
                        : null;
                    if (entry) {{
                        entry.skippedPerfEventCount = skippedPerfEventCount;
                        entry.lastSkippedPerfEventName = eventName;
                    }}
                    return;
                }}
                lastPerfEventAtMs = now;
                sendTerminalEvent({{
                    kind: "perf",
                    name: eventName,
                    payload: {{
                        ...payload,
                        skipped_perf_events: skippedPerfEventCount,
                        host_id: hostId,
                        session_path: host.getAttribute("data-terminal-session-path") || "",
                        cols: term ? Number(term.cols || 0) : 0,
                        rows: term ? Number(term.rows || 0) : 0,
                        scrollback_intent: scrollbackIntent,
                        scrollback_locked: Boolean(scrollbackLocked),
                    }},
                }});
            }} catch (_error) {{}}
        }};
        const stretchXtermRoot = () => {{
            // XTERM-BUG: scrollbar-not-draggable
            // Per docs/xterm-bugs.md — earlier versions hid the scrollbar by
            // expanding viewport/screen width with `calc(100% + gutter)` and a
            // negative right margin, then setting `scrollbar-width: none`
            // inline. That pushed the scrollbar off the right edge of the
            // host (which has overflow:hidden) and clipped it. With the sleek
            // scrollbar CSS we want the scrollbar visible AND draggable, so
            // viewport/screen now stay at natural 100% width and the inline
            // scrollbar-width override is dropped; CSS `scrollbar-width: thin`
            // is the SSOT.
            const xtermRoot = host.querySelector('.xterm');
            const helpers = host.querySelector('.xterm-helpers');
            const helperTextarea = host.querySelector('.xterm-helper-textarea');
            const screen = host.querySelector('.xterm-screen');
            const viewport = host.querySelector('.xterm-viewport');
            const scrollableElement = host.querySelector('.xterm-scrollable-element');
            const rowsLayer = host.querySelector('.xterm-rows');
            host.style.boxSizing = 'border-box';
            host.style.position = 'relative';
            host.style.overflow = 'hidden';
            host.style.paddingLeft = '0px';
            host.style.paddingRight = '0px';
            host.style.paddingTop = '0px';
            host.style.paddingBottom = '0px';
            if (xtermRoot) {{
                xtermRoot.style.height = '100%';
                xtermRoot.style.width = '100%';
                xtermRoot.style.position = 'relative';
                xtermRoot.style.overflow = 'hidden';
            }}
            // XTERM-BUG: scrollable-element-zero-height (xterm.js 6) — the new
            // `.xterm-scrollable-element` wrapper around `.xterm-screen` has no
            // height and collapses to 0 under the WebGL/canvas renderer (its only
            // children are absolutely-positioned canvases), collapsing the screen
            // with it (the "squished viewport"). Mirror the runtime-style fix
            // inline so the screen's percentage height resolves to the host.
            if (scrollableElement) {{
                scrollableElement.style.height = '100%';
                scrollableElement.style.width = '100%';
            }}
            applyNonSelectableSurfaceContract();
            if (helpers) {{
                helpers.style.position = 'absolute';
                helpers.style.inset = '0';
                helpers.style.width = '100%';
                helpers.style.overflow = 'hidden';
                helpers.style.pointerEvents = 'none';
            }}
            if (helperTextarea) {{
                enforceHelperTextareaContract();
                helperTextarea.addEventListener('focus', syncFocusClass, true);
                helperTextarea.addEventListener('blur', (event) => {{
                    if (event && elementBlocksTerminalAutofocus(event.relatedTarget)) {{
                        markTransientUiFocusClaim(760);
                    }}
                    syncFocusClass();
                    scheduleInputDriftRecovery();
                }}, true);
            }}
            syncFocusClass();
            // XTERM-BUG: scrollbar-not-draggable — leave the screen narrower
            // than the host by the scrollbar width so the right-edge
            // scrollbar slot is hit-testable by the native browser drag.
            if (screen) {{
                screen.style.width = `calc(100% - ${{terminalScrollbarGutterPx()}}px)`;
                screen.style.height = '100%';
                screen.style.position = 'relative';
                screen.style.overflow = 'hidden';
            }}
            if (viewport) {{
                viewport.style.width = '100%';
                viewport.style.height = '100%';
                viewport.style.marginRight = '0px';
                viewport.style.overflowX = 'hidden';
                viewport.style.removeProperty('scrollbar-width');
                viewport.style.removeProperty('-ms-overflow-style');
                // XTERM-BUG: scrollbar-drag-triggers-text-selection
                // Without this guard, mousedown on the scrollbar slot
                // bubbles up to xterm.js's selection handler — the
                // browser-native scrollbar drag works, but on release
                // xterm finalizes a phantom text selection in the rows
                // beneath the click. Detect mousedown landing in the
                // right-edge scrollbar slot (where x is between
                // clientWidth and offsetWidth) and stopPropagation so
                // xterm never sees it. Idempotent re-attach via the
                // sentinel attribute so repeat stretchXtermRoot calls
                // don't stack listeners.
                if (!viewport.getAttribute('data-yggterm-scrollbar-guard')) {{
                    viewport.setAttribute('data-yggterm-scrollbar-guard', '1');
                    viewport.addEventListener('mousedown', (event) => {{
                        try {{
                            const rect = viewport.getBoundingClientRect();
                            const scrollbarWidth =
                                Math.max(0, viewport.offsetWidth - viewport.clientWidth);
                            const localX = event.clientX - rect.left;
                            if (
                                scrollbarWidth > 0
                                && localX >= viewport.clientWidth
                                && localX <= viewport.clientWidth + scrollbarWidth
                            ) {{
                                event.stopPropagation();
                            }}
                        }} catch (_error) {{}}
                    }}, true);
                }}
            }}
            if (rowsLayer) {{
                rowsLayer.style.width = '100%';
                rowsLayer.style.height = '100%';
            }}
            ensureFocusCaptureOverlay();
            normalizeLowContrastGlyphs(true);
            applySoftwareCanvasLayerOptimization('stretch_root');
        }};
        stretchXtermRoot();
        refreshCursorContrastContract();
        applySoftwareCanvasLayerOptimization('initial_mount');
        let paintCount = 0;
        let lastPaintKey = '';
        // Daemon-handover paint suspension (user-settled call #7). Set by the
        // `set_handover_paint_suspended` command; while true this host does no
        // render-health sampling, no recovery redraw and no visible paint, and
        // wears a STATIC veil. Nothing else may write it.
        let handoverPaintSuspended = false;
        // The veil is a SIBLING of `.xterm` inside the host — the same shape the
        // cold-mount veil uses (`.yggterm-cold-mount-veil`), so attachment
        // checks that count host children keep working. Solid host background +
        // one line of static text: no animation, no spinner, no timer.
        const applyHandoverPaintVeil = (on) => {{
            try {{
                const existing = host.querySelector('.yggterm-handover-veil');
                if (!on) {{
                    if (existing) {{ existing.remove(); }}
                    return;
                }}
                if (existing) {{ return; }}
                if (window.getComputedStyle(host).position === 'static') {{
                    host.style.position = 'relative';
                }}
                const veil = document.createElement('div');
                veil.className = 'yggterm-handover-veil';
                veil.setAttribute('aria-live', 'polite');
                veil.style.position = 'absolute';
                veil.style.inset = '0';
                veil.style.zIndex = '40';
                veil.style.pointerEvents = 'none';
                veil.style.display = 'flex';
                veil.style.alignItems = 'center';
                veil.style.justifyContent = 'center';
                veil.style.backgroundColor =
                    window.getComputedStyle(host).backgroundColor || '#000';
                const label = document.createElement('div');
                label.textContent = 'Daemon updating. Sessions will settle in a moment.';
                label.style.fontSize = '12px';
                label.style.fontFamily = 'var(--yggterm-term-font-family, monospace)';
                label.style.letterSpacing = '0.2px';
                label.style.color = 'var(--yggterm-term-dim-foreground, #8b949e)';
                veil.appendChild(label);
                host.appendChild(veil);
            }} catch (_error) {{}}
        }};
        let visiblePaintFramePending = false;
        let pendingVisiblePaintForceFullRefresh = false;
        let visiblePaintRecoveryTimer = null;
        let lastVisiblePaintRunAtMs = 0;
        let lastVisiblePaintFullRefreshAtMs = 0;
        let lastVisiblePaintRefreshSkipPerfAtMs = 0;
        let recentFrameLikeWriteUntilMs = 0;
        // See VISIBLE_PAINT_FULL_REFRESH_DEADLINE_MS at the refusal branch below.
        let pendingVisiblePaintForceFullRefreshSinceMs = 0;
        const VISIBLE_PAINT_FULL_REFRESH_DEADLINE_MS = 1500;
        let recentInlineStatusAnimationUntilMs = 0;
        let recentInlineStatusAnimationStartedAtMs = 0;
        const recentFrameLikeWriteHot = () => Date.now() < recentFrameLikeWriteUntilMs;
        const recentInlineStatusAnimationHot = () => Date.now() < recentInlineStatusAnimationUntilMs;
        const terminalFrameLikeInstrumentationThrottleMs = () => Math.max(
            900,
            Math.min(2200, Math.max(terminalActiveWriteFrameMs * 3, 900))
        );
        const visiblePaintMinIntervalMs = 120;
        const visiblePaintFullRefreshMinIntervalMs = 750;
        let rebuildAttempts = 0;
        const emitPaint = () => {{
            rebindCurrentHost('emit_paint', true);
            const xtermRoot = host.querySelector('.xterm');
            const screen = host.querySelector('.xterm-screen');
            const viewport = host.querySelector('.xterm-viewport');
            const rowsLayer = host.querySelector('.xterm-rows');
            const visible =
                host.childElementCount > 0
                || Boolean(xtermRoot)
                || Boolean(screen)
                || Boolean(viewport)
                || Boolean(rowsLayer);
            normalizeLowContrastGlyphs();
            applySoftwareCanvasLayerOptimization('paint');
            // `visible` above is satisfied by ANY child in the host — including an
            // empty `.xterm` husk left behind by a detached term. It reported
            // `true` 43 times over a viewport that never painted a glyph. Record
            // the truthful companion rather than changing this gate's semantics:
            // `ensureVisibleHost` short-circuits on `visible`, so flipping it is a
            // behavioural fix that belongs with the repair work, not the probe.
            const paintAttachment = syncHostAttachmentEntry('emit_paint');
            const paintedElementAttached = paintAttachment
                ? Boolean(paintAttachment.host_contains_term_element)
                : null;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].paintCount = paintCount + 1;
                window.__yggtermXtermHosts[hostId].lastVisiblePaint = visible;
                window.__yggtermXtermHosts[hostId].lastVisiblePaintTermElementAttached = paintedElementAttached;
                window.__yggtermXtermHosts[hostId].lastVisiblePaintWasHusk = Boolean(visible && paintedElementAttached === false);
            }}
            const nextPaintKey = JSON.stringify([
                host.childElementCount,
                visible,
                Boolean(xtermRoot),
                Boolean(screen),
                Boolean(viewport),
                Boolean(rowsLayer),
                Number(term.cols || 0),
                Number(term.rows || 0),
            ]);
            if (nextPaintKey !== lastPaintKey) {{
                lastPaintKey = nextPaintKey;
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
            paintCount += 1;
            // Repaint-storm detection (the ~50Hz garbled-blink pathology). Count
            // paints in a rolling 1s window; a sustained rate far above a normal
            // active terminal (a few/s) is a repaint storm, not real content. A
            // storm sustained >=2s is reported once, then every 30s while it
            // persists (the detach-probe cadence — no flood), and surfaced on the
            // host entry so `server app state` sees it live.
            {{
                const stormNowMs = Date.now();
                if (paintRateWindowStartMs === 0) {{ paintRateWindowStartMs = stormNowMs; }}
                paintRateWindowCount += 1;
                if (stormNowMs - paintRateWindowStartMs >= 1000) {{
                    const paintRatePerSec = paintRateWindowCount;
                    paintRateWindowStartMs = stormNowMs;
                    paintRateWindowCount = 0;
                    const REPAINT_STORM_RATE = 30;
                    if (paintRatePerSec >= REPAINT_STORM_RATE) {{
                        if (repaintStormSinceMs === 0) {{ repaintStormSinceMs = stormNowMs; }}
                    }} else {{
                        repaintStormSinceMs = 0;
                    }}
                    if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                        window.__yggtermXtermHosts[hostId].paintRatePerSec = paintRatePerSec;
                        window.__yggtermXtermHosts[hostId].repaintStormMs =
                            repaintStormSinceMs ? Math.max(0, stormNowMs - repaintStormSinceMs) : 0;
                    }}
                    if (repaintStormSinceMs
                        && stormNowMs - repaintStormSinceMs >= 2000
                        && (stormNowMs - lastRepaintStormReportAtMs > 30000
                            || lastRepaintStormReportAtMs === 0)) {{
                        lastRepaintStormReportAtMs = stormNowMs;
                        sendTerminalEvent({{
                            kind: "debug",
                            message: `terminal_repaint_storm host=${{hostId}}`
                                + ` rate_per_sec=${{paintRatePerSec}}`
                                + ` storm_ms=${{Math.max(0, stormNowMs - repaintStormSinceMs)}}`
                                + ` cols=${{term.cols}} rows=${{term.rows}}`,
                        }});
                    }}
                }}
            }}
            return visible;
        }};
        const currentBufferKind = () => {{
            try {{
                if (!term || !term.buffer) {{
                    return 'unknown';
                }}
                if (term.buffer.active === term.buffer.alternate) {{
                    return 'alternate';
                }}
                if (term.buffer.active === term.buffer.normal) {{
                    return 'normal';
                }}
            }} catch (_error) {{}}
            return 'unknown';
        }};
        const terminalOwnsWheelInput = () => {{
            try {{
                const modes = term && term.modes ? term.modes : null;
                const tracking = modes ? String(modes.mouseTrackingMode || '') : '';
                return currentBufferKind() === 'alternate' || (tracking && tracking !== 'none');
            }} catch (_error) {{
                return false;
            }}
        }};
        const syncXtermViewportElementToBuffer = (targetViewportY, debug = null) => {{
            try {{
                const viewportElement = host.querySelector(".xterm-viewport");
                if (!viewportElement) {{
                    return;
                }}
                if (debug) {{
                    debug.viewport_scroll_top_before = Number(viewportElement.scrollTop || 0);
                }}
                // Record the expected landing row BEFORE the write: the scroll
                // event this triggers arrives async (see A.b.3 note at the
                // declaration) and must not read as a user scroll-up.
                const target = Math.max(0, Number(targetViewportY || 0));
                pendingProgrammaticViewportTargetY = target;
                pendingProgrammaticViewportAtMs = Date.now();
                // BOTTOM-OFFSET FIX (charts live catch 2026-06-11: "bottom sits
                // ~2 lines above the actual bottom" + bg→fg flicker dance):
                // scrollTop = rows × cssRowHeight accumulates the fractional
                // cell-height error over a tall scrollback (~2 rows per 1000).
                // A bottom target uses the EXACT scroll extent; mid-buffer
                // targets derive the per-row height from the scroll geometry
                // itself (scrollHeight / total buffer rows) so the multiplied
                // error cannot accumulate.
                const buf = term && term.buffer && term.buffer.active ? term.buffer.active : null;
                const baseY = buf ? Math.max(0, Number(buf.baseY || 0)) : 0;
                const maxScrollTop = Math.max(
                    0,
                    Number(viewportElement.scrollHeight || 0) - Number(viewportElement.clientHeight || 0)
                );
                if (buf && target >= baseY && maxScrollTop > 0) {{
                    viewportElement.scrollTop = maxScrollTop;
                }} else {{
                    const totalRows = baseY + Math.max(1, Number(term && term.rows ? term.rows : 1));
                    const exactRowHeight = totalRows > 0 && Number(viewportElement.scrollHeight || 0) > 0
                        ? Number(viewportElement.scrollHeight) / totalRows
                        : Math.max(1, terminalCssCellHeight());
                    viewportElement.scrollTop = target * exactRowHeight;
                }}
                if (debug) {{
                    debug.viewport_scroll_top_after = Number(viewportElement.scrollTop || 0);
                }}
            }} catch (_error) {{}}
        }};
        const xtermVisualViewportY = (debug = null) => {{
            try {{
                const viewportElement = host.querySelector(".xterm-viewport");
                if (!viewportElement) {{
                    return null;
                }}
                // xterm.js 6 decoupled scroll position from the `.xterm-viewport`
                // element: the VS Code-derived ScrollableElement (`.xterm-scrollable-
                // element`) owns scrolling and `.xterm-viewport.scrollTop` STAYS 0
                // regardless of where the buffer is scrolled. So the "visual" reading
                // from scrollTop is garbage (always 0 = always "at top"), which made
                // effectiveXtermViewportY report 0 always — no-op'ing app-control
                // scroll AND breaking user-scroll-up detection (the UserScrollback
                // anti-yank flip). When the ScrollableElement is present the
                // authoritative position is the public ydisp (active.viewportY), so
                // return null here and let effectiveXtermViewportY fall through to it.
                if (host.querySelector(".xterm-scrollable-element")) {{
                    if (debug) {{
                        debug.visual_viewport_y = null;
                        debug.visual_viewport_y_decoupled_scrollable_element = true;
                        debug.viewport_scroll_top = Number(viewportElement.scrollTop || 0);
                    }}
                    return null;
                }}
                const rowHeightPx = Math.max(1, terminalCssCellHeight());
                const visualY = Math.max(0, Math.round(Number(viewportElement.scrollTop || 0) / rowHeightPx));
                if (debug) {{
                    debug.visual_viewport_y = visualY;
                    debug.viewport_scroll_top = Number(viewportElement.scrollTop || 0);
                    debug.viewport_row_height_px = rowHeightPx;
                }}
                return visualY;
            }} catch (_error) {{
                return null;
            }}
        }};
        const effectiveXtermViewportY = (active, debug = null) => {{
            try {{
                if (!active) {{
                    return 0;
                }}
                const baseY = Math.max(0, Number(active.baseY || 0));
                const publicY = Math.max(0, Number(active.viewportY || 0));
                const clampedPublicY = Math.min(baseY, publicY);
                const visualY = xtermVisualViewportY(debug);
                if (debug) {{
                    debug.public_viewport_y = publicY;
                    debug.base_y = baseY;
                    if (publicY !== clampedPublicY) {{
                        debug.public_viewport_y_clamped_to_base = clampedPublicY;
                    }}
                }}
                const rendererState = terminalRendererSurfaceState();
                if (rendererState.missingTextLayer) {{
                    if (debug) {{
                        debug.used_visual_viewport_y = false;
                        debug.ignored_visual_viewport_y_reason = 'missing_renderer_surface';
                    }}
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId]
                        : null;
                    if (entry) {{
                        entry.publicViewportY = publicY;
                        entry.visualViewportY = Number.isFinite(visualY) ? Number(visualY) : null;
                        entry.effectiveViewportY = clampedPublicY;
                        entry.viewportYDiscrepancy = Number.isFinite(visualY) ? Number(visualY) - publicY : 0;
                        entry.viewportYSource = 'xterm_public_missing_renderer_surface';
                        entry.viewportYClampedToBase = publicY !== clampedPublicY;
                    }}
                    return clampedPublicY;
                }}
                if (Number.isFinite(visualY) && Number(visualY) > baseY + 1) {{
                    if (debug) {{
                        debug.used_visual_viewport_y = false;
                        debug.visual_viewport_y_out_of_range = true;
                        debug.visual_viewport_y_clamped_to_base = baseY;
                    }}
                    syncXtermViewportElementToBuffer(baseY, debug);
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId]
                        : null;
                    if (entry) {{
                        entry.publicViewportY = publicY;
                        entry.visualViewportY = Number(visualY);
                        entry.effectiveViewportY = baseY;
                        entry.viewportYDiscrepancy = Number(visualY) - publicY;
                        entry.viewportYSource = 'xterm_public_visual_beyond_base';
                        entry.viewportYClampedToBase = true;
                        entry.viewportYDiscrepancyAtMs = Date.now();
                    }}
                    return baseY;
                }}
                if (Number.isFinite(visualY) && Math.abs(Number(visualY) - publicY) > 1) {{
                    if (debug) {{
                        debug.used_visual_viewport_y = true;
                    }}
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId]
                        : null;
                    if (entry) {{
                        entry.publicViewportY = publicY;
                        entry.visualViewportY = Number(visualY);
                        entry.effectiveViewportY = Math.min(baseY, Number(visualY));
                        entry.viewportYDiscrepancy = Number(visualY) - publicY;
                        entry.viewportYSource = 'dom_visual';
                        entry.viewportYDiscrepancyAtMs = Date.now();
                        entry.viewportYClampedToBase = Number(visualY) > baseY;
                    }}
                    return Math.max(0, Math.min(baseY, Number(visualY)));
                }}
                if (debug) {{
                    debug.used_visual_viewport_y = false;
                }}
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                    if (entry) {{
                        entry.publicViewportY = publicY;
                        entry.visualViewportY = Number.isFinite(visualY) ? Number(visualY) : null;
                    entry.effectiveViewportY = clampedPublicY;
                    entry.viewportYDiscrepancy = Number.isFinite(visualY) ? Number(visualY) - publicY : 0;
                    entry.viewportYSource = 'xterm_public';
                    entry.viewportYClampedToBase = publicY !== clampedPublicY;
                }}
                return clampedPublicY;
            }} catch (_error) {{
                return active ? Math.max(0, Number(active.viewportY || 0)) : 0;
            }}
        }};
        const internalXtermBufferTargets = () => {{
            const targets = [];
            try {{
                const core = term && term._core ? term._core : null;
                const nestedCore = core && core._core ? core._core : null;
                for (const owner of [core, nestedCore, term]) {{
                    if (!owner) {{
                        continue;
                    }}
                    for (const key of ['_bufferService', 'bufferService']) {{
                        const service = owner[key];
                        if (service && service.buffer) {{
                            targets.push({{ owner, service, buffer: service.buffer }});
                        }}
                    }}
                }}
            }} catch (_error) {{}}
            return targets;
        }};
        const forceXtermViewportY = (targetViewportY, reason = '') => {{
            // Mark every programmatic viewport move so the onScroll it fires
            // synchronously is NOT mistaken for a user scroll-up (working-session
            // cluster follow fix). Saved/restored to survive any nesting.
            const _priorProgrammaticScroll = programmaticScrollInProgress;
            programmaticScrollInProgress = true;
            const debug = {{
                reason: String(reason || ''),
                requested_target_viewport_y: Number(targetViewportY || 0),
                used_public_scroll_to_line: false,
                used_public_scroll_lines: false,
                used_core_scroll_lines: false,
                used_internal_ydisp_repair: false,
                used_refresh: false,
            }};
            try {{
                const active = term && term.buffer ? term.buffer.active : null;
                if (!active) {{
                    debug.reason = debug.reason || 'missing_active_buffer';
                    return debug;
                }}
                const baseY = Math.max(0, Number(active.baseY || 0));
                const beforeViewportY = Math.max(0, Number(active.viewportY || 0));
                const beforeEffectiveViewportY = effectiveXtermViewportY(active, debug);
                const target = Math.max(0, Math.min(baseY, Math.round(Number(targetViewportY || 0))));
                debug.before_viewport_y = beforeViewportY;
                debug.before_effective_viewport_y = beforeEffectiveViewportY;
                debug.before_base_y = baseY;
                debug.target_viewport_y = target;
                if (beforeViewportY === target || beforeEffectiveViewportY === target) {{
                    debug.noop_matched_target = true;
                    syncXtermViewportElementToBuffer(target, debug);
                }} else if (typeof term.scrollToLine === 'function') {{
                    try {{
                        term.scrollToLine(target);
                        debug.used_public_scroll_to_line = true;
                    }} catch (_error) {{}}
                }}
                let afterViewportY = Math.max(0, Number(active.viewportY || 0));
                debug.after_public_scroll_to_line_viewport_y = afterViewportY;
                if (!debug.noop_matched_target && afterViewportY !== target && typeof term.scrollLines === 'function') {{
                    try {{
                        term.scrollLines(target - afterViewportY);
                        debug.used_public_scroll_lines = true;
                    }} catch (_error) {{}}
                    afterViewportY = Math.max(0, Number(active.viewportY || 0));
                    debug.after_public_scroll_lines_viewport_y = afterViewportY;
                }}
                const core = term && term._core ? term._core : null;
                if (!debug.noop_matched_target && afterViewportY !== target && core && typeof core.scrollLines === 'function') {{
                    try {{
                        // The third argument asks xterm's browser terminal to bypass
                        // DOM scroll mediation and update the buffer viewport directly.
                        core.scrollLines(target - afterViewportY, false, 1);
                        debug.used_core_scroll_lines = true;
                    }} catch (_error) {{}}
                    afterViewportY = Math.max(0, Number(active.viewportY || 0));
                    debug.after_core_scroll_lines_viewport_y = afterViewportY;
                }}
                if (!debug.noop_matched_target && afterViewportY !== target) {{
                    for (const targetBuffer of internalXtermBufferTargets()) {{
                        const internalBuffer = targetBuffer && targetBuffer.buffer ? targetBuffer.buffer : null;
                        if (!internalBuffer || !Number.isFinite(Number(internalBuffer.ydisp))) {{
                            continue;
                        }}
                        try {{
                            internalBuffer.ydisp = target;
                            debug.used_internal_ydisp_repair = true;
                            const owner = targetBuffer.owner || core;
                            if (owner && owner._onScroll && typeof owner._onScroll.fire === 'function') {{
                                owner._onScroll.fire(target);
                                debug.used_internal_owner_on_scroll_fire = true;
                            }} else if (
                                targetBuffer.service
                                && targetBuffer.service._onScroll
                                && typeof targetBuffer.service._onScroll.fire === 'function'
                            ) {{
                                targetBuffer.service._onScroll.fire(target);
                                debug.used_internal_service_on_scroll_fire = true;
                            }}
                        }} catch (_error) {{}}
                        afterViewportY = Math.max(0, Number(active.viewportY || 0));
                        if (afterViewportY === target) {{
                            break;
                        }}
                    }}
                    debug.after_internal_ydisp_repair_viewport_y = afterViewportY;
                }}
                syncXtermViewportElementToBuffer(target, debug);
                if (!debug.noop_matched_target) {{
                    try {{
                        if (typeof term.refresh === 'function') {{
                            term.refresh(0, Math.max(0, Number(term.rows || 1) - 1));
                            debug.used_refresh = true;
                        }}
                    }} catch (_error) {{}}
                }}
                syncXtermViewportElementToBuffer(target, debug);
                afterViewportY = Math.max(0, Number(active.viewportY || 0));
                const afterEffectiveViewportY = effectiveXtermViewportY(active, debug);
                debug.after_viewport_y = afterViewportY;
                debug.after_effective_viewport_y = afterEffectiveViewportY;
                debug.after_base_y = Math.max(0, Number(active.baseY || 0));
                debug.matched_target = afterViewportY === target || afterEffectiveViewportY === target;
            }} catch (error) {{
                debug.error = error && error.message ? error.message : String(error);
            }}
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.lastViewportForceDebug = debug;
                    entry.lastViewportForceReason = String(reason || '');
                    entry.lastViewportForceAtMs = Date.now();
                    // Always-on capped ring buffer of recent viewport moves so a
                    // single keystroke's flicker sequence (which setter moved the
                    // viewport, to where, with what intent) can be dumped via the
                    // `terminal probe-viewport-trace` app-control probe. Cheap: one
                    // small object push, capped at 64. See [[audit-viewport-scroll-control-flow]].
                    if (!Array.isArray(entry.viewportForceLog)) {{ entry.viewportForceLog = []; }}
                    entry.viewportForceLog.push({{
                        at: Date.now(),
                        reason: String(debug.reason || ''),
                        req: Number(debug.requested_target_viewport_y || 0),
                        target: Number(debug.target_viewport_y || 0),
                        beforeEff: Number(debug.before_effective_viewport_y || 0),
                        afterEff: Number(debug.after_effective_viewport_y || 0),
                        base: Number(debug.after_base_y || 0),
                        noop: Boolean(debug.noop_matched_target),
                        intent: String(scrollbackIntent || ''),
                        locked: Boolean(scrollbackLocked),
                    }});
                    if (entry.viewportForceLog.length > 64) {{
                        entry.viewportForceLog.splice(0, entry.viewportForceLog.length - 64);
                    }}
                }}
            }} catch (_error) {{}}
            // Record where we landed so the NEXT onScroll can tell a passive strand
            // (ydisp unchanged while baseY grows) from a user scroll-up (ydisp down).
            // Use effectiveXtermViewportY to match syncScrollbackLock's comparison.
            try {{
                lastObservedScrollYdisp = effectiveXtermViewportY(term.buffer.active);
                lastObservedScrollBaseY = Math.max(0, Number(term.buffer.active.baseY || 0));
            }} catch (_error) {{}}
            programmaticScrollInProgress = _priorProgrammaticScroll;
            return debug;
        }};
        // XTERM-BUG: blank-viewport-client-snapshot-poison
        // A cached frame that collapsed to <=1 nonblank line for a session that
        // previously held real content (tracked nonblank max >= 6) is almost
        // certainly a poison/blank frame. Treat it as unusable so restore/replay
        // falls through to the daemon authoritative content (source of truth).
        const xtermSessionSnapshotIsCollapsedPoison = (sessionPath, nonblankLineCount) => {{
            try {{
                const maxMap = window.__yggtermXtermSessionNonblankMax || {{}};
                const priorMax = Math.max(0, Number(maxMap[sessionPath] || 0));
                return Number(nonblankLineCount) <= 1 && priorMax >= 6;
            }} catch (_error) {{
                return false;
            }}
        }};
        const latestXtermSessionSnapshotForCurrentSession = () => {{
            try {{
                const sessionPath = currentHostSessionPath();
                const snapshots = window.__yggtermXtermSessionSnapshots || {{}};
                const snapshot = sessionPath ? snapshots[sessionPath] : null;
                if (!snapshot || typeof snapshot.text !== 'string') {{
                    return null;
                }}
                const ageMs = Date.now() - Number(snapshot.capturedAtMs || 0);
                if (!Number.isFinite(ageMs) || ageMs < 0 || ageMs > 10 * 60 * 1000) {{
                    return null;
                }}
                const text = String(snapshot.text || '');
                const lineCount = Math.max(
                    Number(snapshot.lineCount || 0),
                    Number(snapshot.logicalLineCount || 0),
                    (text.match(/\n/g) || []).length + 1
                );
                const nonblankLineCount = Number(snapshot.nonblankLineCount || 0);
                if (!text.trim() || lineCount <= 0 || nonblankLineCount <= 0) {{
                    return null;
                }}
                // XTERM-BUG: blank-viewport-client-snapshot-poison (restore guard)
                // Reject a sparse snapshot for a session that previously had real
                // content. Returning null here means the client-snapshot restore is
                // skipped, the surface stays empty, and the EXISTING, well-tested
                // empty-surface fault-recovery re-replays from the daemon (source of
                // truth). Defense-in-depth paired with the capture-side poison guard.
                if (xtermSessionSnapshotIsCollapsedPoison(sessionPath, nonblankLineCount)) {{
                    return null;
                }}
                return {{ ...snapshot, ageMs, lineCount, nonblankLineCount }};
            }} catch (_error) {{
                return null;
            }}
        }};
        // XTERM-BUG: scrollback-lost-on-gui-restart
        // Apply a pending localStorage-restored scroll position once daemon-replayed
        // scrollback has populated the buffer past distanceFromBottom rows AND baseY
        // has been stable for at least 600ms (replay finished). Don't apply on first
        // reach: more rows may still be arriving and we'd land at the wrong viewport.
        const tryApplyPendingPersistedScrollRestore = (reason = '') => {{
            try {{
                if (!pendingPersistedScrollRestore) {{ return false; }}
                // SCROLL-OWNERSHIP: abandon a stale scroll-restore the instant
                // follow-mode is engaged. A restored session is armed in
                // 'UserScrollback'; it only flips to 'PromptFollow' when the user
                // genuinely engages the prompt (keystroke/paste/scroll-to-bottom)
                // or live output is injected. In any of those cases the user wants
                // the live bottom, NOT a saved scrollback offset — re-applying the
                // offset here was the post-restart flicker (restore vs prompt-follow
                // fighting on every click/keystroke for the 8s poll window). A truly
                // passive restored session stays in 'UserScrollback', so this still
                // restores the scroll position for the case it was designed for.
                // See [[audit-viewport-scroll-control-flow]].
                if (scrollbackIntent !== 'UserScrollback') {{
                    pendingPersistedScrollRestore = null;
                    const abandonEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId] : null;
                    if (abandonEntry) {{
                        abandonEntry.persistedScrollRestorePending = false;
                        abandonEntry.persistedScrollRestoreAbandonedReason = `follow_engaged:${{reason}}`;
                        abandonEntry.persistedScrollRestoreAbandonedAtMs = Date.now();
                    }}
                    return false;
                }}
                const nowMs = Date.now();
                const pastDeadline = nowMs > pendingPersistedScrollRestoreDeadlineMs;
                const buffer = term && term.buffer && term.buffer.active ? term.buffer.active : null;
                if (!buffer) {{
                    if (pastDeadline) {{ pendingPersistedScrollRestore = null; }}
                    return false;
                }}
                const baseY = Math.max(0, Number(buffer.baseY || 0));
                const distance = Math.max(0, Number(pendingPersistedScrollRestore.distanceFromBottom || 0));
                if (baseY < distance) {{
                    if (pastDeadline) {{ pendingPersistedScrollRestore = null; }}
                    return false;
                }}
                // Track baseY stability: only apply when baseY has held steady for 600ms.
                const lastSeenBaseY = Number(pendingPersistedScrollRestore.lastSeenBaseY);
                const lastSeenBaseYAtMs = Number(pendingPersistedScrollRestore.lastSeenBaseYAtMs || 0);
                if (!Number.isFinite(lastSeenBaseY) || lastSeenBaseY !== baseY) {{
                    pendingPersistedScrollRestore.lastSeenBaseY = baseY;
                    pendingPersistedScrollRestore.lastSeenBaseYAtMs = nowMs;
                    if (!pastDeadline) {{ return false; }}
                }}
                const stableForMs = nowMs - lastSeenBaseYAtMs;
                if (!pastDeadline && stableForMs < 600) {{ return false; }}
                const target = Math.max(0, baseY - distance);
                forceXtermViewportY(target, `persisted_scroll_restore:${{reason}}`);
                scrollbackLocked = Boolean(pendingPersistedScrollRestore.locked) || target < baseY;
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId] : null;
                if (entry) {{
                    entry.scrollbackLocked = scrollbackLocked;
                    entry.persistedScrollRestoreApplied = true;
                    entry.persistedScrollRestoreAppliedAtMs = Date.now();
                    entry.persistedScrollRestoreAppliedReason = String(reason || '');
                    entry.persistedScrollRestoreTargetViewportY = target;
                }}
                sendTerminalEvent({{
                    kind: 'debug',
                    message: `persisted_scroll_restored host=${{hostId}} target=${{target}} distance=${{distance}} reason=${{reason}}`
                }});
                pendingPersistedScrollRestore = null;
                return true;
            }} catch (_error) {{
                pendingPersistedScrollRestore = null;
                return false;
            }}
        }};
        const restoreXtermSessionSnapshotOnConstructed = () => {{
            const snapshot = latestXtermSessionSnapshotForCurrentSession();
            if (!snapshot) {{
                // XTERM-BUG: scrollback-lost-on-gui-restart — in-memory snapshot is
                // gone (process restart). Try localStorage for BOTH text and scroll.
                const persisted = loadScrollStateFromLocalStorage();
                // WS3 screen-restore (vacuum fix): if localStorage carries the buffer TEXT
                // (now persisted across GUI restart) and it's a real (non-collapsed) frame,
                // write it into the fresh xterm so the transcript survives a full GUI+daemon
                // restart instead of a vacuum (codex resume doesn't re-print history; the
                // daemon re-resumed on a fresh PTY). Marking the host non-blank + seeding the
                // nonblank-max stops the blank-host replay from clobbering it with the sparse
                // fresh-PTY screen. Gated by the same collapsed-poison rule as the cache.
                if (persisted && typeof persisted.text === 'string' && persisted.text.trim()
                    && persisted.nonblankLineCount > 1
                    && !xtermSessionSnapshotIsCollapsedPoison(currentHostSessionPath(), persisted.nonblankLineCount)) {{
                    const restoredText = persisted.text.replace(/\r?\n/g, "\r\n");
                    const ws = term && term._core && typeof term._core.writeSync === "function"
                        ? term._core.writeSync.bind(term._core)
                        : (term && term._core && term._core._writeBuffer && typeof term._core._writeBuffer.writeSync === "function"
                            ? term._core._writeBuffer.writeSync.bind(term._core._writeBuffer) : null);
                    try {{
                        traceXtermScreenEvent("reset", {{ reason: "persisted_restore", chars: restoredText.length }});
                        if (typeof term.reset === "function") {{ term.reset(); }}
                        // Replayed scrollback must not re-fire OSC 52 copy (see the OSC 52 handler).
                        window.__yggtermArmOsc52Suppress(hostId, 400);
                        if (ws) {{ ws("\x1bc\x1b[H"); ws(restoredText); }}
                        else if (typeof term.write === "function") {{ term.write(`\x1bc\x1b[H${{restoredText}}`); }}
                        if (window.__yggtermTrace && window.__yggtermTrace.captureStream) {{
                            window.__yggtermTrace.captureStream(hostId, "restore", restoredText);
                        }}
                        const tentry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                            ? window.__yggtermXtermHosts[hostId] : null;
                        if (tentry) {{
                            tentry.terminalContentSource = 'localstorage_session_snapshot';
                            tentry.lastRetainedReplaySource = 'localstorage_session_snapshot';
                            tentry.lastRetainedReplayRecoveredFromSnapshot = true;
                            tentry.lastLocalStorageTextRestoreLineCount = persisted.lineCount;
                            tentry.lastLocalStorageTextRestoreAtMs = Date.now();
                        }}
                        // Stash for re-application after the daemon `reset` command
                        // (which fires once on attach AFTER this construct-time
                        // restore and would otherwise wipe the transcript).
                        pendingPostResetTranscript = {{
                            text: restoredText,
                            lineCount: persisted.lineCount,
                            nonblankLineCount: persisted.nonblankLineCount,
                        }};
                        window.__yggtermXtermSessionNonblankMax = window.__yggtermXtermSessionNonblankMax || {{}};
                        const sp2 = currentHostSessionPath();
                        if (sp2) {{
                            window.__yggtermXtermSessionNonblankMax[sp2] = Math.max(
                                Number(window.__yggtermXtermSessionNonblankMax[sp2] || 0),
                                persisted.nonblankLineCount
                            );
                        }}
                        sendTerminalEvent({{
                            kind: 'debug',
                            message: `localstorage_session_text_restored host=${{hostId}} lines=${{persisted.lineCount}} nonblank=${{persisted.nonblankLineCount}} age_ms=${{Math.round(persisted.ageMs)}}`
                        }});
                    }} catch (_restoreErr) {{}}
                }}
                // Restore whenever user was not at the bottom (locked) OR intent was
                // UserScrollback — but only for a RECENT record. Sweep capture
                // (2026-06-10) caught a 12.2-HOUR-old localStorage record re-asserting
                // UserScrollback at mount, latching a phantom pinned state durably
                // across restarts. A scroll position only deserves restoring on the
                // continuity horizon of a restart, not across half a day; the TEXT
                // restore above is intentionally NOT capped (transcript survival is
                // the vacuum fix and has no staleness hazard).
                const scrollRestoreFreshMs = 30 * 60 * 1000;
                if (persisted && persisted.ageMs <= scrollRestoreFreshMs
                    && (persisted.intent === 'UserScrollback' || persisted.locked || persisted.distanceFromBottom > 0)) {{
                    pendingPersistedScrollRestore = persisted;
                    pendingPersistedScrollRestoreDeadlineMs = Date.now() + 8000;
                    setScrollbackIntent('UserScrollback', `persisted_scroll_state:age_ms_${{Math.round(persisted.ageMs)}}`);
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId] : null;
                    if (entry) {{
                        entry.persistedScrollRestorePending = true;
                        entry.persistedScrollRestoreAgeMs = persisted.ageMs;
                        entry.persistedScrollRestoreDistance = persisted.distanceFromBottom;
                    }}
                    // Poll attempts at 1s/2s/3s/4s/5s/6s/7s/8s — final fires at deadline
                    // and applies regardless of stability gate.
                    [1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000].forEach((delayMs) => {{
                        window.setTimeout(() => tryApplyPendingPersistedScrollRestore(`poll_${{delayMs}}`), delayMs);
                    }});
                }}
                return false;
            }}
            const normalizedText = String(snapshot.text || '').replace(/\r?\n/g, "\r\n");
            if (!normalizedText.trim()) {{
                return false;
            }}
            const writeSync = term && term._core && typeof term._core.writeSync === "function"
                ? term._core.writeSync.bind(term._core)
                : term && term._core && term._core._writeBuffer && typeof term._core._writeBuffer.writeSync === "function"
                    ? term._core._writeBuffer.writeSync.bind(term._core._writeBuffer)
                    : null;
            try {{
                traceXtermScreenEvent("reset", {{ reason: "snapshot_reseed" }});
                if (typeof term.reset === "function") {{
                    term.reset();
                }}
                if (typeof term.clear === "function") {{
                    term.clear();
                }}
            }} catch (_error) {{}}
            try {{
                if (writeSync) {{
                    writeSync("\x1bc\x1b[H");
                    writeSync(normalizedText);
                }} else if (typeof term.write === "function") {{
                    term.write(`\x1bc\x1b[H${{normalizedText}}`);
                }} else {{
                    return false;
                }}
            }} catch (_error) {{
                return false;
            }}
            const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                ? window.__yggtermXtermHosts[hostId]
                : null;
            if (entry) {{
                const rows = Math.max(0, Number(term && term.rows ? term.rows : 0));
                entry.terminalContentSource = 'xterm_session_snapshot';
                entry.terminalSourceMismatchReason = '';
                entry.lastRetainedReplaySource = 'xterm_session_snapshot';
                entry.lastRetainedReplayRecoveredFromSnapshot = true;
                entry.lastRetainedReplaySnapshotAgeMs = Number(snapshot.ageMs || 0);
                entry.lastRetainedReplaySnapshotError = '';
                entry.lastRawPayloadLength = normalizedText.length;
                entry.lastRawPayloadLineCount = Number(snapshot.lineCount || 0);
                entry.lastRawPayloadSample = terminalPayloadDebugSample(normalizedText);
                entry.lastRetainedReplayLineCount = Number(snapshot.lineCount || 0);
                entry.lastRetainedReplayExpected = Number(snapshot.lineCount || 0) > Math.max(4, rows + 4);
                entry.scrollbackExpected = Number(snapshot.lineCount || 0) > Math.max(4, rows + 4);
                entry.lastXtermSessionSnapshotReason = String(snapshot.reason || 'constructed_restore');
                entry.lastXtermSessionSnapshotAtMs = Number(snapshot.capturedAtMs || 0);
                entry.lastXtermSessionSnapshotLineCount = Number(snapshot.lineCount || 0);
                entry.lastXtermSessionSnapshotNonblankLineCount = Number(snapshot.nonblankLineCount || 0);
                entry.lastXtermSessionSnapshotBaseY = Number(snapshot.baseY || 0);
                entry.lastXtermSessionSnapshotViewportY = Number(snapshot.viewportY || 0);
            }}
            // XTERM-BUG: phantom-scrollback-latch — the teardown snapshot copies
            // the dying host's scroll intent verbatim, and this restore used to
            // re-assert UserScrollback+locked from it, so a single spurious
            // programmatic flip self-perpetuated across remounts (5-sweep
            // dataset: 7 UserScrollback flips with no human scroll, 4 of them
            // via `xterm_session_snapshot:cleanup`). Per the scroll_mode spec a
            // constructed/cold reveal starts Following; only a live user scroll
            // may latch UserScrollback. The snapshot keeps carrying the intent
            // fields for telemetry, but the restore never latches from them.
            const restoredIntent = String(snapshot.scrollbackIntent || '');
            if (restoredIntent === 'UserScrollback') {{
                sendTerminalEvent({{
                    kind: 'debug',
                    message: `xterm_session_snapshot_intent_latch_dropped host=${{hostId}} reason=${{String(snapshot.reason || 'restore')}}`
                }});
            }}
            setScrollbackIntent('PromptFollow', `xterm_session_snapshot:${{String(snapshot.reason || 'restore')}}`);
            window.requestAnimationFrame(() => {{
                scrollLiveCursorIntoView(false, 'xterm_session_snapshot_restore');
                requestVisiblePaint(true);
                emitHostHealth();
            }});
            sendTerminalEvent({{
                kind: "debug",
                message: `xterm_session_snapshot_restored host=${{hostId}} lines=${{Number(snapshot.lineCount || 0)}} intent=${{restoredIntent || 'PromptFollow'}} age_ms=${{Number(snapshot.ageMs || 0)}}`
            }});
            return true;
        }};
        syncTerminalScrollController = (reason = '') => {{
            try {{
                const controller = document.getElementById(`yggterm-scroll-controller-${{hostId}}`);
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                const active = term && term.buffer ? term.buffer.active : null;
                if (!controller || !active) {{
                    if (controller) {{
                        controller.style.opacity = '0';
                        controller.style.pointerEvents = 'none';
                        controller.setAttribute('data-yggterm-scroll-controller-visible', 'false');
                    }}
                    if (entry) {{
                        entry.scrollControllerVisible = false;
                        entry.scrollControllerReason = String(reason || '');
                    }}
                    return;
                }}
                const viewportY = effectiveXtermViewportY(active);
                const baseY = Math.max(0, Number(active.baseY || 0));
                const rows = Math.max(1, Number(term.rows || 0));
                const distanceRows = Math.max(0, baseY - viewportY);
                const visible = distanceRows >= Math.max(3, Math.ceil(rows / 2))
                    || scrollbackIntent === 'UserScrollback';
                controller.style.opacity = visible ? '1' : '0';
                controller.style.pointerEvents = visible ? 'auto' : 'none';
                controller.setAttribute('data-yggterm-scroll-controller-visible', visible ? 'true' : 'false');
                controller.setAttribute('data-yggterm-scroll-controller-distance-rows', String(distanceRows));
                controller.setAttribute('data-yggterm-scroll-controller-intent', String(scrollbackIntent || 'PromptFollow'));
                if (entry) {{
                    entry.scrollControllerVisible = Boolean(visible);
                    entry.scrollControllerDistanceRows = distanceRows;
                    entry.scrollControllerReason = String(reason || '');
                    entry.scrollControllerUpdatedAtMs = Date.now();
                }}
            }} catch (_error) {{}}
        }};
        // XTERM-BUG: webgl-stale-atlas-garble — page-global rAF gap monitor.
        // While the window is backgrounded/occluded WebKitGTK throttles rAF, so
        // this loop's tick timestamps freeze; the first tick after the throttle
        // computes the gap. A render that lands right after such a gap without an
        // atlas clear since the gap began paints wrong-glyph garble (stale GPU
        // glyph atlas). One monitor per page, installed by whichever host mounts
        // first.
        try {{
            if (!window.__yggtermRafGapMonitor) {{
                const rafGapMonitor = {{
                    lastTickAtMs: Date.now(),
                    lastGapMs: 0,
                    lastGapEndedAtMs: 0,
                    gapCount: 0,
                }};
                window.__yggtermRafGapMonitor = rafGapMonitor;
                const rafGapTick = () => {{
                    const now = Date.now();
                    const gap = now - rafGapMonitor.lastTickAtMs;
                    if (gap > 1000) {{
                        rafGapMonitor.lastGapMs = gap;
                        rafGapMonitor.lastGapEndedAtMs = now;
                        rafGapMonitor.gapCount += 1;
                        // ⭐ PREVENT, don't repair. This tick is the first frame
                        // after the throttle ended, so it runs BEFORE any render
                        // can paint from the atlas that went stale during the
                        // gap. Clearing here means the garbled frame is never
                        // drawn at all.
                        //
                        // The detector further down is the repair path, and
                        // repair is strictly worse: it can only fire once a
                        // garbled frame has already been painted and seen. The
                        // owner's 2026-08-11 report was a viewport where EVERY
                        // glyph was wrong — not a few stray cells — so a frame
                        // of that reaching the screen at all is the defect.
                        //
                        // Page-global on purpose: every mounted host shares this
                        // one throttle, so every host's atlas went stale
                        // together. Costs one clear + refresh per host per
                        // occlusion episode; `refresh` only marks rows dirty, so
                        // a background host pays nothing until it paints.
                        try {{
                            const gapHosts = window.__yggtermXtermHosts || {{}};
                            for (const gapHostId of Object.keys(gapHosts)) {{
                                const gapEntry = gapHosts[gapHostId];
                                if (!gapEntry || !gapEntry.term) continue;
                                // Only hosts that EXISTED during the throttle can
                                // have a stale atlas; one that mounted after it
                                // built its atlas fresh, and wiping that is pure
                                // cost — every glyph re-rasterizes, and cells
                                // painted before their glyph lands come out BLANK.
                                // That is the eaten-digits / holed-highlight
                                // symptom, so an unnecessary clear is not a
                                // harmless one.
                                if (Number(gapEntry.mountedAtMs || 0) >= now - gap) continue;
                                try {{
                                    if (typeof gapEntry.term.clearTextureAtlas === 'function') {{
                                        gapEntry.term.clearTextureAtlas();
                                    }}
                                    // Stamp the SAME field the detector compares
                                    // against, so a paint after this clear is
                                    // correctly no longer treated as stale — the
                                    // repair path must not also fire once the
                                    // prevention has done the work.
                                    gapEntry.lastAtlasClearAtMs = now;
                                    // Stamped on the HOST entry, not just the
                                    // page-global monitor, because the host
                                    // entry is the only thing the health report
                                    // reads — a counter that lives anywhere else
                                    // is a counter nobody can read, which is the
                                    // defect this whole entry is about.
                                    gapEntry.preemptiveAtlasClearCount =
                                        (gapEntry.preemptiveAtlasClearCount || 0) + 1;
                                    gapEntry.lastPreemptiveAtlasClearAtMs = now;
                                    if (typeof gapEntry.term.refresh === 'function') {{
                                        gapEntry.term.refresh(0, Math.max(0, gapEntry.term.rows - 1));
                                    }}
                                }} catch (_hostError) {{}}
                            }}
                            rafGapMonitor.lastPreemptiveClearAtMs = now;
                            rafGapMonitor.preemptiveClearCount =
                                (rafGapMonitor.preemptiveClearCount || 0) + 1;
                        }} catch (_error) {{}}
                    }}
                    rafGapMonitor.lastTickAtMs = now;
                    window.requestAnimationFrame(rafGapTick);
                }};
                window.requestAnimationFrame(rafGapTick);
            }}
        }} catch (_error) {{}}
        const clearTerminalTextureAtlas = () => {{
            lastAtlasClearAtMs = Date.now();
            try {{
                const atlasEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (atlasEntry) {{
                    atlasEntry.lastAtlasClearAtMs = lastAtlasClearAtMs;
                }}
            }} catch (_error) {{}}
            try {{
                if (typeof term.clearTextureAtlas === 'function') {{
                    term.clearTextureAtlas();
                    return;
                }}
            }} catch (_error) {{}}
            try {{
                const renderer = term && term._core && term._core._renderService
                    ? term._core._renderService._renderer
                    : null;
                if (renderer && typeof renderer.clearTextureAtlas === 'function') {{
                    renderer.clearTextureAtlas();
                }}
            }} catch (_error) {{}}
        }};
        const redrawTerminal = (reason = 'manual') => {{
            if (standDownIfSuperseded(`redraw:${{String(reason || '')}}`)) {{
                return;
            }}
            manualRedrawCount += 1;
            const redrawStartedAtMs = Date.now();
            // Fail-pattern detection: a burst of repaints with NO session
            // change/reveal/resize behind them is the user's "3 quick blinks"
            // symptom (e.g. render-health recovery firing repeatedly). Record it so
            // the trace shows the pattern + context and the user need not report
            // every rendering nuance.
            try {{
                recentRedrawEvents.push({{ ts: redrawStartedAtMs, reason: String(reason || 'manual') }});
                if (recentRedrawEvents.length > 12) recentRedrawEvents.shift();
                const burstWindowMs = 2000;
                const burst = recentRedrawEvents.filter((e) => redrawStartedAtMs - e.ts <= burstWindowMs);
                const explained = (r) => /reveal|mount|session|resize|refit|app-control|command-redraw|redraw-terminal|manual/.test(String(r || ''));
                if (burst.length >= 3 && !burst.some((e) => explained(e.reason))) {{
                    pendingRenderAnomaly = JSON.stringify({{
                        pattern: 'redraw_burst',
                        count: burst.length,
                        window_ms: burstWindowMs,
                        reasons: burst.map((e) => e.reason).slice(-5),
                    }});
                }}
            }} catch (_error) {{}}
            const redrawRenderEventCountBefore = Number(renderEventCount || 0);
            const redrawReasonForcesPromptFollow = (value) => {{
                const text = String(value || 'manual');
                return text === 'manual'
                    || text.includes('manual-redraw')
                    || text.includes('app-control-manual-redraw')
                    || text.includes('command-redraw')
                    || text.includes('redraw-terminal');
            }};
            const redrawShouldFollowPrompt =
                redrawReasonForcesPromptFollow(reason) || scrollbackIntent !== 'UserScrollback';
            let redrawInkBefore = null;
            try {{ redrawInkBefore = sampleCanvasInk(); }} catch (_error) {{}}
            try {{ hideLowPowerTuiOverlay(); }} catch (_error) {{}}
            try {{ rebindCurrentHost(String(reason || 'manual-redraw'), true); }} catch (_error) {{}}
            try {{ repairMissingRendererSurface(String(reason || 'manual-redraw')); }} catch (_error) {{}}
            try {{ fitTerminalToHost(String(reason || 'manual')); }} catch (_error) {{}}
            try {{ applyTerminalRowFitGuard(String(reason || 'manual')); }} catch (_error) {{}}
            try {{ scrollLiveCursorIntoView(redrawShouldFollowPrompt, String(reason || 'manual-redraw')); }} catch (_error) {{}}
            clearTerminalTextureAtlas();
            try {{
                if (term.refresh) {{
                    term.refresh(0, Math.max(0, term.rows - 1));
                }}
            }} catch (_error) {{}}
            try {{ enforceHelperTextareaContract(); }} catch (_error) {{}}
            requestVisiblePaint(true);
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                const entry = window.__yggtermXtermHosts[hostId];
                entry.manualRedrawCount = manualRedrawCount;
                entry.lastManualRedrawReason = String(reason || 'manual');
                entry.lastManualRedrawAtMs = redrawStartedAtMs;
                entry.lastManualRedrawStartedAtMs = redrawStartedAtMs;
                entry.lastManualRedrawSettledAtMs = 0;
                entry.lastManualRedrawDurationMs = null;
                entry.lastManualRedrawRenderEventCountBefore = redrawRenderEventCountBefore;
                entry.lastManualRedrawRenderEventCountAfter = redrawRenderEventCountBefore;
                entry.lastManualRedrawInkBefore = redrawInkBefore;
                entry.lastManualRedrawInkAfter = null;
                entry.lastManualRedrawEffect = 'pending';
            }}
            requestAnimationFrame(() => {{
                try {{ repairMissingRendererSurface(String(reason || 'manual-redraw')); }} catch (_error) {{}}
                try {{ scrollLiveCursorIntoView(redrawShouldFollowPrompt, String(reason || 'manual-redraw')); }} catch (_error) {{}}
                try {{
                    if (term.refresh) {{
                        term.refresh(0, Math.max(0, term.rows - 1));
                    }}
                }} catch (_error) {{}}
                emitPaint();
                enforceHelperTextareaContract();
                emitHostHealth();
                const settledAtMs = Date.now();
                const redrawRenderEventCountAfter = Number(renderEventCount || 0);
                let redrawInkAfter = null;
                try {{ redrawInkAfter = sampleCanvasInk(); }} catch (_error) {{}}
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    const entry = window.__yggtermXtermHosts[hostId];
                    const beforeInkPixels = redrawInkBefore
                        ? Number(redrawInkBefore.nontransparent_pixels || 0)
                        : -1;
                    const afterInkPixels = redrawInkAfter
                        ? Number(redrawInkAfter.nontransparent_pixels || 0)
                        : -1;
                    const renderDelta = redrawRenderEventCountAfter - redrawRenderEventCountBefore;
                    entry.lastManualRedrawSettledAtMs = settledAtMs;
                    entry.lastManualRedrawDurationMs = Math.max(0, settledAtMs - redrawStartedAtMs);
                    entry.lastManualRedrawRenderEventCountAfter = redrawRenderEventCountAfter;
                    entry.lastManualRedrawInkAfter = redrawInkAfter;
                    entry.lastManualRedrawEffect = renderDelta > 0
                        ? 'rendered'
                        : (beforeInkPixels !== afterInkPixels ? 'ink_changed' : 'repainted_no_text_change');
                }}
            }});
            sendTerminalEvent({{
                kind: "debug",
                message: `terminal_repaint host=${{hostId}} reason=${{reason}}`
            }});
        }};
        const forceTerminalRepaint = (reason) => redrawTerminal(reason || 'forced_repaint');
        const sampleCanvasInk = () => {{
            // Ink may only be judged from canvases that are actual RENDER layers
            // (inside `.xterm-screen`). The host also carries non-render canvases —
            // the reveal ghost, focus overlays — that legitimately sample
            // transparent; sampling them as a proxy for "the terminal painted
            // nothing" convicted healthy sessions (guihost 2026-07-23: 110 false
            // unhealthy edges/hour driving an atlas-clearing redraw loop = the
            // user's in-session blink). A canvas that already holds a GPU context
            // (the WebGL renderer's text layer) returns null from getContext('2d');
            // that is UNREADABLE, not blank — counted separately so the verdict can
            // refuse to judge instead of judging blind. Reading it via drawImage
            // was tried (ba2fe8c) and REVERTED: per-check GPU readback of the live
            // canvas corrupts the WebKitGTK glyph atlas. Do not reintroduce it.
            const canvases = Array.from(host.querySelectorAll('.xterm-screen canvas'))
                .filter((canvas) => Number(canvas.width || 0) > 0 && Number(canvas.height || 0) > 0)
                .slice(-4);
            let sampledPixels = 0;
            let nontransparentPixels = 0;
            let alphaSum = 0;
            let readableLayers = 0;
            let unreadableLayers = 0;
            for (const canvas of canvases) {{
                try {{
                    const context = canvas.getContext('2d', {{ willReadFrequently: true }});
                    if (!context) {{
                        unreadableLayers += 1;
                        continue;
                    }}
                    readableLayers += 1;
                    const width = Math.max(1, Number(canvas.width || 0));
                    const height = Math.max(1, Number(canvas.height || 0));
                    const stepX = Math.max(1, Math.floor(width / 12));
                    const stepY = Math.max(1, Math.floor(height / 8));
                    for (let y = Math.floor(stepY / 2); y < height; y += stepY) {{
                        for (let x = Math.floor(stepX / 2); x < width; x += stepX) {{
                            const data = context.getImageData(x, y, 1, 1).data;
                            sampledPixels += 1;
                            alphaSum += Number(data[3] || 0);
                            if (Number(data[3] || 0) > 8) {{
                                nontransparentPixels += 1;
                            }}
                        }}
                    }}
                }} catch (_error) {{}}
            }}
            return {{
                canvas_count: canvases.length,
                readable_layers: readableLayers,
                unreadable_layers: unreadableLayers,
                sampled_pixels: sampledPixels,
                nontransparent_pixels: nontransparentPixels,
                alpha_sum: alphaSum,
            }};
        }};
        // XTERM-BUG: blank-rendering-region — detect the PArecordsAL variant of
        // canvas_blank_with_buffer_text: individual viewport rows whose buffer
        // holds text but whose text-layer band holds no ink (a blank band or
        // heavy glyph dropping inside an otherwise painted viewport). The full
        // ink sample cannot see it (other rows keep the aggregate nonzero).
        // One bulk getImageData per scan (a single GPU sync) instead of
        // hundreds of 1px reads; scans are throttled to one per 5s per host
        // and only run on the active host with a healthy aggregate sample.
        // Heals like the stale-atlas path: targeted atlas clear + row refresh,
        // latched to at most one heal per 10s so it can never form a loop.
        const detectAndHealGlyphGapRows = (reason = '') => {{
            const now = Date.now();
            if (now - lastGlyphGapScanAtMs < 5000) {{
                return null;
            }}
            lastGlyphGapScanAtMs = now;
            const buffer = term && term.buffer && term.buffer.active ? term.buffer.active : null;
            const rowCount = Math.max(0, Number(term && term.rows ? term.rows : 0));
            if (!buffer || rowCount < 4) {{
                return null;
            }}
            const textCanvas = Array.from(host.querySelectorAll('.xterm-screen canvas'))
                .find((canvas) => canvasLayerRole(canvas) === 'text'
                    && Number(canvas.width || 0) > 0
                    && Number(canvas.height || 0) > 0);
            if (!textCanvas) {{
                return null;
            }}
            let image = null;
            try {{
                const context = textCanvas.getContext('2d', {{ willReadFrequently: true }});
                if (!context) {{
                    return null;
                }}
                image = context.getImageData(0, 0, textCanvas.width, textCanvas.height);
            }} catch (_error) {{
                return null;
            }}
            const width = Number(textCanvas.width || 0);
            const height = Number(textCanvas.height || 0);
            const cellHeight = height / rowCount;
            if (!(cellHeight > 1) || width < 8) {{
                return null;
            }}
            const data = image.data;
            const gapRows = [];
            let inkedRows = 0;
            let textRows = 0;
            const stepX = Math.max(1, Math.floor(width / 24));
            for (let row = 0; row < rowCount; row += 1) {{
                let lineText = '';
                try {{
                    const line = buffer.getLine(Math.max(0, Number(buffer.viewportY || 0)) + row);
                    lineText = line && typeof line.translateToString === 'function'
                        ? line.translateToString(true)
                        : '';
                }} catch (_error) {{}}
                if (!/[A-Za-z0-9]/.test(String(lineText || ''))) {{
                    continue;
                }}
                textRows += 1;
                const yStart = Math.max(0, Math.floor(row * cellHeight)) + 1;
                const yEnd = Math.min(height, Math.ceil((row + 1) * cellHeight)) - 1;
                let rowHasInk = false;
                for (let y = yStart; y < yEnd && !rowHasInk; y += 2) {{
                    const rowBase = y * width * 4;
                    for (let x = Math.floor(stepX / 2); x < width; x += stepX) {{
                        if (Number(data[rowBase + x * 4 + 3] || 0) > 8) {{
                            rowHasInk = true;
                            break;
                        }}
                    }}
                }}
                if (rowHasInk) {{
                    inkedRows += 1;
                }} else {{
                    gapRows.push(row);
                }}
            }}
            if (gapRows.length < 3 || inkedRows === 0) {{
                return null;
            }}
            const healAllowed = now - lastGlyphGapHealAtMs > 10000;
            if (healAllowed) {{
                lastGlyphGapHealAtMs = now;
                glyphGapHealCount += 1;
                const refreshStart = Math.max(0, gapRows[0]);
                const refreshEnd = Math.min(rowCount - 1, gapRows[gapRows.length - 1]);
                window.setTimeout(() => {{
                    try {{
                        clearTerminalTextureAtlas();
                        if (term.refresh) {{
                            term.refresh(refreshStart, refreshEnd);
                        }}
                    }} catch (_error) {{}}
                }}, 0);
            }}
            const glyphGapEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                ? window.__yggtermXtermHosts[hostId]
                : null;
            if (glyphGapEntry) {{
                glyphGapEntry.glyphGapHealCount = glyphGapHealCount;
                glyphGapEntry.lastGlyphGapRows = gapRows.slice(0, 12);
                glyphGapEntry.lastGlyphGapDetectedAtMs = now;
            }}
            return {{
                pattern: 'glyph_gap_rows',
                gap_row_count: gapRows.length,
                gap_rows_sample: gapRows.slice(0, 12),
                text_rows: textRows,
                inked_rows: inkedRows,
                heal_count: glyphGapHealCount,
                healed: healAllowed,
                window_focused: document.hasFocus(),
                visibility: String(document.visibilityState || ''),
                source: String(reason || ''),
            }};
        }};
        const updateRenderHealth = (reason, cursorLineText = '', textTail = '', options = {{}}) => {{
            if (standDownIfSuperseded(`render_health:${{String(reason || '')}}`)) {{
                return {{ status: 'superseded', reason: 'closure_stood_down', ink: null,
                    recovery_count: 0, recovery_pending: false }};
            }}
            // Daemon handover: the canvas is legitimately blank behind the veil
            // and the PTY is being re-created, so every reading here would be a
            // false "unhealthy" — and its recovery is a full redrawTerminal (a
            // glyph-atlas clear), i.e. exactly the render cost this suspension
            // exists to avoid. Refusing to judge is the honest verdict.
            if (handoverPaintSuspended) {{
                return {{ status: 'suspended', reason: 'daemon_handover_paint_suspended', ink: null,
                    recovery_count: renderHealthRecoveryCount, recovery_pending: false }};
            }}
            const now = Date.now();
            lastRenderHealthCheckedAtMs = now;
            const skipInkSample = Boolean(options && options.skip_ink_sample);
            const bufferHealthSample = skipInkSample ? '' : readTerminalBufferSample().slice(-480);
            const textSample = `${{String(cursorLineText || '')}}\n${{String(textTail || '')}}\n${{bufferHealthSample}}`;
            const hasBufferText = /[A-Za-z0-9]/.test(textSample);
            const rendererLayerMissing = terminalRendererSurfaceState().missingTextLayer;
            const ink = skipInkSample
                ? {{
                    canvas_count: 0,
                    sampled_pixels: 0,
                    nontransparent_pixels: 0,
                    alpha_sum: 0,
                    skipped: true,
                    reason: 'frame_like_hot',
                }}
                : sampleCanvasInk();
            // ZERO-SAMPLE HONESTY (2026-07-22): `sampled_pixels === 0` means the ink
            // probe found nothing to measure, NOT that the surface is fine. A blank
            // viewport once scored `healthy` off an all-zero sample for 16 minutes.
            // Mark it so no reader (agent or human) can mistake "could not sample"
            // for "sampled and it was good".
            if (!skipInkSample && ink && Number(ink.canvas_count || 0) === 0 && Number(ink.sampled_pixels || 0) === 0) {{
                ink.unsampleable = true;
            }}
            // UNREADABLE-LAYER HONESTY (2026-07-23): a render canvas holding a GPU
            // context cannot be read without touching the live canvas (the ba2fe8c
            // regression), so a sample taken beside one is INCOMPLETE — the very
            // layer that paints the text is the one missing from it. Refusing to
            // judge is the only honest verdict.
            if (!skipInkSample && ink && Number(ink.unreadable_layers || 0) > 0) {{
                ink.unsampleable = true;
            }}
            const attachment = syncHostAttachmentEntry(`render_health:${{String(reason || '')}}`);
            // A detached `term.element` cannot paint by construction — no ink probe,
            // buffer read, or cursor field can see it, so it must be its own verdict.
            // PERSISTENCE GATE (2026-07-23): a detach reading younger than ~1s is
            // routinely the health check racing a repair mid-flight (trace showed
            // `detached_ms=0` episodes 28–642ms after `rebind_host_attach`), and
            // each such reading scheduled a redraw whose own wipe window produced
            // the NEXT detach reading — the repair manufacturing its fault signal.
            // A real detach persists; only judge one that has.
            const detachedPersistedMs = termElementDetachedSinceMs > 0
                ? Math.max(0, now - termElementDetachedSinceMs)
                : 0;
            const unhealthyDetachedTermElement = Boolean(
                attachment
                && attachment.detached
                && hasBufferText
                && detachedPersistedMs >= 900
            );
            const unhealthyDomRenderer = hasBufferText && rendererLayerMissing;
            const unhealthyCanvas = !skipInkSample
                && hasBufferText
                && ink.canvas_count > 0
                && ink.sampled_pixels > 0
                && Number(ink.unreadable_layers || 0) === 0
                && (ink.nontransparent_pixels === 0 || ink.alpha_sum <= 12);
            const unhealthy = unhealthyDetachedTermElement || unhealthyDomRenderer || unhealthyCanvas;
            // Background hosts sample blank legitimately (a hidden WebGL canvas
            // holds no ink), so a recovery redraw can never turn them healthy —
            // firing it anyway formed an endless ~6s heavy-repaint loop per
            // backgrounded host (guihost trace 2026-07-07: session kept
            // unhealthy+recovery_pending every 5-6s for minutes after switch-away).
            // Keep the unhealthy STATUS (the Rust reveal reconcile uses it to
            // force a repaint when the session is next revealed) but suffix the
            // reason and never schedule the recovery redraw for background hosts.
            const hostActiveAttr = host.getAttribute('data-active-session-host');
            const hostIsActive = hostActiveAttr === 'true'
                || (hostActiveAttr === null
                    && String(host.getAttribute('data-terminal-session-path') || '')
                        === String(window.__yggtermActiveTerminalSessionPath || ''));
            renderHealthStatus = unhealthy ? 'unhealthy' : 'healthy';
            renderHealthReason = unhealthyDetachedTermElement
                ? (attachment && attachment.unrepairable_detached
                    ? 'term_element_detached_from_host_unrepairable'
                    : 'term_element_detached_from_host')
                : (unhealthyDomRenderer
                    ? 'dom_renderer_missing_text_layer_with_buffer_text'
                    : (unhealthyCanvas
                        ? (hostIsActive
                            ? 'canvas_blank_with_buffer_text'
                            : 'canvas_blank_with_buffer_text_background')
                        : ''));
            // Partial-blank / glyph-drop scan: only worth running when the
            // aggregate sample looks healthy (a fully blank canvas is already
            // handled above) and only for the active host (background WebGL
            // canvases sample blank legitimately).
            if (!skipInkSample && !unhealthy && hasBufferText && hostIsActive) {{
                try {{
                    const glyphGapAnomaly = detectAndHealGlyphGapRows(reason);
                    if (glyphGapAnomaly) {{
                        pendingRenderAnomaly = JSON.stringify(glyphGapAnomaly);
                    }}
                }} catch (_error) {{}}
            }}
            const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                ? window.__yggtermXtermHosts[hostId]
                : null;
            if (entry) {{
                entry.renderHealthStatus = renderHealthStatus;
                entry.renderHealthReason = renderHealthReason;
                entry.renderHealthInkSample = ink;
                entry.lastRenderHealthCheckedAtMs = lastRenderHealthCheckedAtMs;
                entry.renderHealthRecoveryCount = renderHealthRecoveryCount;
                entry.lastRenderHealthRecoveryAtMs = lastRenderHealthRecoveryAtMs;
                entry.pendingRenderHealthRecovery = renderHealthRecoveryPending;
            }}
            // Re-arm the recovery budget once the canvas is healthy again (after a
            // brief settle). The old gate (renderHealthRecoveryCount < 1, no reset)
            // healed a blank canvas exactly ONCE per host lifetime, so a canvas that
            // RE-blanks later — the returning mid-session blink — was never healed
            // again. Resetting on sustained health lets each fresh blank episode be
            // healed, while the per-episode count cap + cooldown keep repeated
            // redrawTerminal (which clears the glyph atlas) from forming a
            // full-canvas refresh loop.
            if (
                !unhealthy
                && renderHealthRecoveryCount > 0
                && now - lastRenderHealthRecoveryAtMs > 2000
            ) {{
                renderHealthRecoveryCount = 0;
                if (entry) {{
                    entry.renderHealthRecoveryCount = 0;
                }}
            }}
            if (
                unhealthy
                && hostIsActive
                && !renderHealthRecoveryPending
                && renderHealthRecoveryCount < 2
                && now - lastRenderHealthRecoveryAtMs > renderHealthRecoveryBackoffMs
            ) {{
                // Escalate the cooldown when the previous heal did not stick
                // (the canvas re-blanked within 30s): 2s → 4s → … → 60s. A
                // fresh episode long after the last recovery re-arms the fast
                // 2s heal. This keeps first-heal latency identical while
                // preventing an endless repaint cadence (each recovery clears
                // the glyph atlas + refreshes every row — the CPU-swing driver
                // when the blanking is compositor-side and healing cannot win).
                renderHealthRecoveryBackoffMs =
                    lastRenderHealthRecoveryAtMs > 0 && now - lastRenderHealthRecoveryAtMs < 30000
                        ? Math.min(60000, renderHealthRecoveryBackoffMs * 2)
                        : 2000;
                renderHealthRecoveryPending = true;
                renderHealthRecoveryCount += 1;
                lastRenderHealthRecoveryAtMs = now;
                if (entry) {{
                    entry.renderHealthRecoveryCount = renderHealthRecoveryCount;
                    entry.lastRenderHealthRecoveryAtMs = lastRenderHealthRecoveryAtMs;
                    entry.pendingRenderHealthRecovery = true;
                    entry.renderHealthRecoveryBackoffMs = renderHealthRecoveryBackoffMs;
                }}
                window.setTimeout(() => {{
                    renderHealthRecoveryPending = false;
                    if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                        window.__yggtermXtermHosts[hostId].pendingRenderHealthRecovery = false;
                    }}
                    redrawTerminal(`render_health:${{renderHealthReason || reason || 'unknown'}}`);
                }}, 80);
            }}
            return {{
                status: renderHealthStatus,
                reason: renderHealthReason,
                ink,
                recovery_count: renderHealthRecoveryCount,
                recovery_pending: renderHealthRecoveryPending,
            }};
        }};
        const trackTerminalVisualState = (reason) => {{
            const bufferKind = currentBufferKind();
            const cursorHidden = Boolean(terminalCursorState().hidden);
            let changed = false;
            if (lastObservedBufferKind !== null && lastObservedBufferKind !== bufferKind) {{
                bufferTransitionCount += 1;
                changed = true;
            }} else if (lastObservedBufferKind === null && bufferKind === 'alternate') {{
                bufferTransitionCount += 1;
                changed = true;
            }}
            if (lastObservedCursorHidden !== null && lastObservedCursorHidden !== cursorHidden) {{
                cursorHiddenToggleCount += 1;
                changed = true;
            }} else if (lastObservedCursorHidden === null && cursorHidden) {{
                cursorHiddenToggleCount += 1;
                changed = true;
            }}
            lastObservedBufferKind = bufferKind;
            lastObservedCursorHidden = cursorHidden;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].bufferTransitionCount = bufferTransitionCount;
                window.__yggtermXtermHosts[hostId].cursorHiddenToggleCount = cursorHiddenToggleCount;
                window.__yggtermXtermHosts[hostId].lastObservedBufferKind = bufferKind;
                window.__yggtermXtermHosts[hostId].lastObservedCursorHidden = cursorHidden;
                window.__yggtermXtermHosts[hostId].lastVisualTransitionReason = lastVisualTransitionReason;
            }}
            if (!changed) {{
                return;
            }}
            lastVisualTransitionReason = `${{reason}}:${{bufferKind}}:${{cursorHidden ? 'hidden' : 'visible'}}`;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].lastVisualTransitionReason = lastVisualTransitionReason;
            }}
            forceTerminalRepaint(lastVisualTransitionReason);
        }};
        const recordVisiblePaintRefreshSkipped = (reason, forceFullRefresh) => {{
            forcedRefreshSkippedCount += 1;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].forcedRefreshSkippedCount =
                    forcedRefreshSkippedCount;
            }}
            const now = Date.now();
            const skipReason = String(reason || 'skipped');
            if (skipReason === 'input_hot' || now - lastVisiblePaintRefreshSkipPerfAtMs >= 1000) {{
                lastVisiblePaintRefreshSkipPerfAtMs = now;
                emitPerf("xterm_forced_refresh_skipped", {{
                    reason: skipReason,
                    force_full_refresh: Boolean(forceFullRefresh),
                }});
            }}
        }};
        const scheduleVisiblePaintRecovery = (forceFullRefresh = false, delayMs = 0) => {{
            pendingVisiblePaintForceFullRefresh = Boolean(
                pendingVisiblePaintForceFullRefresh || forceFullRefresh
            );
            const now = Date.now();
            let waitMs = Number.isFinite(delayMs) ? Math.max(0, Math.round(delayMs)) : 0;
            if (waitMs <= 0) {{
                waitMs = 0;
            }}
            if (visiblePaintRecoveryTimer !== null) {{
                window.clearTimeout(visiblePaintRecoveryTimer);
            }}
            const hostEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                ? window.__yggtermXtermHosts[hostId]
                : null;
            if (hostEntry) {{
                hostEntry.pendingVisiblePaintRecovery = true;
                hostEntry.pendingVisiblePaintRecoveryUntilMs = now + waitMs;
            }}
            visiblePaintRecoveryTimer = window.setTimeout(() => {{
                visiblePaintRecoveryTimer = null;
                if (terminalInputHot()) {{
                    const hotMsRemaining = Math.max(
                        0,
                        terminalInputHotUntilMs - Date.now()
                    );
                    if (hostEntry) {{
                        hostEntry.pendingVisiblePaintRecoveryUntilMs = Date.now() + Math.min(
                            terminal_input_hot_suppress_ms,
                            hotMsRemaining,
                        );
                    }}
                    const retryDelayMs = Math.max(
                        16,
                        Math.min(terminal_input_hot_suppress_ms, hotMsRemaining),
                    );
                    scheduleVisiblePaintRecovery(
                        pendingVisiblePaintForceFullRefresh,
                        retryDelayMs,
                    );
                    return;
                }}
                if (hostEntry) {{
                    hostEntry.pendingVisiblePaintRecovery = false;
                    hostEntry.pendingVisiblePaintRecoveryUntilMs = 0;
                }}
                requestVisiblePaint(Boolean(pendingVisiblePaintForceFullRefresh));
            }}, waitMs);
        }};
        const requestVisiblePaint = (forceFullRefresh = false) => {{
            // A full-refresh DEMAND is latched before anything can drop the
            // request, because the latch is the only thing that survives
            // coalescing. This assignment used to sit BELOW the suspension
            // return, which meant a suspended host DESTROYED every full-refresh
            // it was asked for instead of deferring it — see the suspension
            // note below.
            pendingVisiblePaintForceFullRefresh = Boolean(
                pendingVisiblePaintForceFullRefresh || forceFullRefresh
            );
            // When the OUTSTANDING demand was first raised. A latch that can be
            // deferred forever is not a latch, so the deadline below has to know
            // how old this one is. Cleared only when a refresh actually runs.
            if (pendingVisiblePaintForceFullRefresh && pendingVisiblePaintForceFullRefreshSinceMs === 0) {{
                pendingVisiblePaintForceFullRefreshSinceMs = Date.now();
            }}
            // Daemon handover: every repaint here is a full-window blit on a
            // software-GL host, and the frame it would present is the re-resume
            // churn behind the veil. Drop the FRAME — never the demand: the
            // resume path (`set_handover_paint_suspended` -> false) is the one
            // and only site that repaints this host afterwards, and it now does
            // a full redraw of the client's own buffer.
            if (handoverPaintSuspended) {{
                return;
            }}
            if (visiblePaintFramePending) {{
                scheduleVisiblePaintRecovery(pendingVisiblePaintForceFullRefresh, 0);
                return;
            }}
            visiblePaintFramePending = true;
            requestAnimationFrame(() => {{
                visiblePaintFramePending = false;
                const requestedForceFullRefresh = Boolean(pendingVisiblePaintForceFullRefresh);
                pendingVisiblePaintForceFullRefresh = false;
                const now = Date.now();
                if (
                    lastVisiblePaintRunAtMs > 0
                    && now - lastVisiblePaintRunAtMs < visiblePaintMinIntervalMs
                ) {{
                    if (requestedForceFullRefresh) {{
                        recordVisiblePaintRefreshSkipped('rate_limited', requestedForceFullRefresh);
                    }}
                    scheduleVisiblePaintRecovery(
                        requestedForceFullRefresh,
                        visiblePaintMinIntervalMs - (now - lastVisiblePaintRunAtMs),
                    );
                    return;
                }}
                lastVisiblePaintRunAtMs = now;
                rebindCurrentHost('request_visible_paint', true);
                const inputHot = terminalInputHot();
                const recentFrameLikeWrite = Date.now() < recentFrameLikeWriteUntilMs;
                try {{
                    stretchXtermRoot();
                    requestRenderProbe('visible_paint');
                }} catch (_error) {{}}
                const metrics = hostMetrics();
                if (hostLooksUsable() && term.rows <= 1) {{
                    try {{
                        const proposed = typeof fitAddon.proposeDimensions === 'function'
                            ? fitAddon.proposeDimensions()
                            : null;
                        if (proposed && terminalGridIsUsable(proposed.cols, proposed.rows)) {{
                            if (scrollbackIntent !== 'UserScrollback') {{
                                armPromptFollowLayoutGuard('visible_paint_degenerate', 720);
                            }}
                            term.resize(proposed.cols, proposed.rows);
                            if (scrollbackIntent !== 'UserScrollback') {{
                                schedulePromptFollowAfterLayout('visible_paint_degenerate');
                            }}
                        }} else {{
                            recordSkippedFit('visible_paint_degenerate', proposed, 'proposed_grid_unusable');
                        }}
                    }} catch (_error) {{}}
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `degenerate_fit host=${{hostId}} width=${{metrics.width}} height=${{metrics.height}} cols=${{term.cols}} rows=${{term.rows}}`
                    }});
                }}
                if (inputHot) {{
                    scheduleVisiblePaintRecovery(requestedForceFullRefresh, 0);
                }}
                try {{
                    const fullRefreshRateLimited =
                        lastVisiblePaintFullRefreshAtMs > 0
                        && now - lastVisiblePaintFullRefreshAtMs
                            < visiblePaintFullRefreshMinIntervalMs;
                    // ⛔⛔ THE DEADLINE, AND IT IS THE FIX FOR
                    // "shell sessions never break, our special sessions ONLY break".
                    //
                    // `recentFrameLikeWrite` is armed by ANY payload containing
                    // `\x1b[?25l` (hide cursor) for at least 600 ms. Every TUI emits
                    // hide-cursor before every redraw, so for an agent CLI this flag
                    // is re-armed on every frame and is effectively ALWAYS true --
                    // while a plain shell, which does not bracket its output that
                    // way, almost never arms it. So `&& !recentFrameLikeWrite` is in
                    // practice an AGENT-CLI-ONLY suppression of `term.refresh()`,
                    // which is the ONLY thing that repairs a partial paint. The
                    // owner's discriminator, in one boolean.
                    //
                    // Why a partial paint is fatal to a TUI and harmless to a shell:
                    // a shell appends at the cursor unconditionally, so unpainted
                    // cells are overwritten at the next prompt. A TUI redraws in
                    // place and uses cursor-forward over runs of spaces, and
                    // CUF-skipped cells KEEP WHATEVER WAS IN THEM -- so the holes
                    // latch forever. Owner screenshot 2026-08-10: composer line 1
                    // perfect, wrapped line 2 missing ~half its characters.
                    //
                    // ⛔ The refusal branch below DESTROYED the demand rather than
                    // deferring it -- the same latch-loss this function's own header
                    // says it was restructured to prevent, surviving one layer down.
                    // THE FIX FOR A CLAIM NOBODY CLEARS IS TO MAKE IT EXPIRE.
                    const fullRefreshOverdue =
                        requestedForceFullRefresh
                        && pendingVisiblePaintForceFullRefreshSinceMs > 0
                        && now - pendingVisiblePaintForceFullRefreshSinceMs
                            >= VISIBLE_PAINT_FULL_REFRESH_DEADLINE_MS;
                    if (
                        requestedForceFullRefresh
                        && term.refresh
                        && !inputHot
                        && (fullRefreshOverdue || (!recentFrameLikeWrite && !fullRefreshRateLimited))
                    ) {{
                        lastVisiblePaintFullRefreshAtMs = now;
                        pendingVisiblePaintForceFullRefreshSinceMs = 0;
                        forcedRefreshCount += 1;
                        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                            window.__yggtermXtermHosts[hostId].forcedRefreshCount = forcedRefreshCount;
                        }}
                        // WebGL glyph-atlas heal: while the window is backgrounded
                        // the WebGL render loop (rAF) is throttled and the GPU glyph
                        // atlas texture goes stale, so the first forced full refresh
                        // after foreground/switch-in paints cells against a stale
                        // atlas -> wrong-glyph garble that self-heals ~1s later. A
                        // bare term.refresh() re-renders cells but reuses that stale
                        // atlas, so clear it FIRST and let refresh rebuild glyphs in
                        // the same frame (atomic within this rAF -> blink-free). This
                        // is exactly what redrawTerminal()/manual-redraw does to heal
                        // it; doing it on the forced-refresh funnel means foreground,
                        // switch-in, and settled-resize all rebuild the atlas instead
                        // of presenting garbage. Gated by the same rate-limit/
                        // input-hot/frame-like checks so it never fires during
                        // typing or active output streaming.
                        clearTerminalTextureAtlas();
                        forcedAtlasClearCount += 1;
                        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                            window.__yggtermXtermHosts[hostId].forcedAtlasClearCount = forcedAtlasClearCount;
                        }}
                        term.refresh(0, Math.max(0, term.rows - 1));
                        emitPerf("xterm_forced_refresh", {{
                            reason: "visible_paint",
                            force_full_refresh: Boolean(requestedForceFullRefresh),
                            atlas_cleared: true,
                        }});
                    }} else if (requestedForceFullRefresh) {{
                        recordVisiblePaintRefreshSkipped(
                            inputHot ? 'input_hot' : (recentFrameLikeWrite ? 'frame_like' : 'rate_limited'),
                            requestedForceFullRefresh
                        );
                        // ⛔ DEFER, NEVER DROP. `pendingVisiblePaintForceFullRefresh`
                        // was cleared at the top of this frame, so without re-arming
                        // here the demand is GONE and those cells are never repainted
                        // again. `input_hot` already re-arms above; these two did not.
                        pendingVisiblePaintForceFullRefresh = true;
                        if (pendingVisiblePaintForceFullRefreshSinceMs === 0) {{
                            pendingVisiblePaintForceFullRefreshSinceMs = now;
                        }}
                        if (!inputHot) {{
                            // Wake when the condition that refused us can have
                            // lapsed, or at the deadline -- whichever is sooner --
                            // so a continuously-drawing TUI cannot defer us forever.
                            const untilFrameLike = recentFrameLikeWrite
                                ? Math.max(0, recentFrameLikeWriteUntilMs - now)
                                : 0;
                            const untilRateLimit = fullRefreshRateLimited
                                ? Math.max(
                                    0,
                                    visiblePaintFullRefreshMinIntervalMs
                                        - (now - lastVisiblePaintFullRefreshAtMs)
                                )
                                : 0;
                            const untilDeadline = Math.max(
                                0,
                                pendingVisiblePaintForceFullRefreshSinceMs
                                    + VISIBLE_PAINT_FULL_REFRESH_DEADLINE_MS
                                    - now
                            );
                            scheduleVisiblePaintRecovery(
                                true,
                                Math.max(16, Math.min(
                                    Math.max(untilFrameLike, untilRateLimit),
                                    untilDeadline
                                ))
                            );
                        }}
                    }}
                }} catch (_error) {{}}
                emitPaint();
            }});
        }};
        const flushResizeNotification = () => {{
            if (resizeNotifyTimer !== null) {{
                window.clearTimeout(resizeNotifyTimer);
                resizeNotifyTimer = null;
            }}
            const pending = pendingResizeNotify;
            pendingResizeNotify = null;
            if (!pending) {{
                return;
            }}
            lastResizeNotifyAtMs = Date.now();
            sendTerminalEvent({{ kind: "resize", cols: pending.cols, rows: pending.rows }});
        }};
        const scheduleResizeNotification = () => {{
            pendingResizeNotify = {{ cols: term.cols, rows: term.rows }};
            if (resizeNotifyTimer !== null) {{
                return;
            }}
            const now = Date.now();
            const elapsed = lastResizeNotifyAtMs > 0
                ? now - lastResizeNotifyAtMs
                : Number.POSITIVE_INFINITY;
            const delayMs = elapsed >= 120 ? 0 : Math.max(24, 120 - elapsed);
            resizeNotifyTimer = window.setTimeout(flushResizeNotification, delayMs);
        }};
        const terminalDataBypassesInputGate = (data) => {{
            if (typeof data !== 'string' || data.length === 0) {{
                return false;
            }}
            // xterm.js emits terminal-emulator protocol replies, such as DSR/CPR/DA,
            // through onData. These must reach the PTY even while user input is
            // readiness-gated, otherwise remote Codex can wait on terminal probes
            // and leave the viewport blank until its own timeout path fires.
            return /^\x1b\[[0-9;?]*[Rcnu]$/.test(data)
                || /^\x1b\[[>?][0-9;?]*c$/.test(data)
                || /^\x1b\[[IO]$/.test(data)
                || /^\x1b\][0-9]+;.*(?:\x07|\x1b\\)$/.test(data);
        }};
        const terminalDataIsSuppressedProtocolResponse = (data) => {{
            if (typeof data !== 'string' || data.length === 0) {{
                return false;
            }}
            const rgb = '[0-9a-fA-F]{{2,4}}/[0-9a-fA-F]{{2,4}}/[0-9a-fA-F]{{2,4}}';
            const terminator = '(?:\\x07|\\x1b\\\\)$';
            const paletteReply = new RegExp(
                `^\\x1b\\]4;(?:\\d+;rgb:${{rgb}})(?:;\\d+;rgb:${{rgb}})*${{terminator}}`
            );
            const defaultColorReply = new RegExp(
                `^\\x1b\\](?:10|11);rgb:${{rgb}}${{terminator}}`
            );
            return paletteReply.test(data) || defaultColorReply.test(data);
        }};
        const terminalInputBatchDelayMs = 8;
        const terminalInputBatchMaxChars = 128;
        let pendingTerminalInputData = '';
        let pendingTerminalInputTimer = null;
        let inputBatchFlushCount = 0;
        const clearPendingTerminalInputTimer = () => {{
            if (pendingTerminalInputTimer !== null) {{
                window.clearTimeout(pendingTerminalInputTimer);
                pendingTerminalInputTimer = null;
            }}
        }};
        const setPendingTerminalInputDiagnostics = (reason = '') => {{
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].pendingInputBytes = pendingTerminalInputData.length;
                window.__yggtermXtermHosts[hostId].pendingInputFlushScheduled =
                    pendingTerminalInputTimer !== null;
                if (reason) {{
                    window.__yggtermXtermHosts[hostId].lastPendingInputReason = reason;
                }}
            }}
        }};
        const flushPendingTerminalInput = (reason = 'scheduled') => {{
            clearPendingTerminalInputTimer();
            if (!pendingTerminalInputData) {{
                setPendingTerminalInputDiagnostics(reason);
                return false;
            }}
            const data = pendingTerminalInputData;
            pendingTerminalInputData = '';
            inputBatchFlushCount += 1;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                const entry = window.__yggtermXtermHosts[hostId];
                entry.pendingInputBytes = 0;
                entry.pendingInputFlushScheduled = false;
                entry.inputBatchFlushCount = inputBatchFlushCount;
                entry.lastInputBatchFlushReason = reason;
                entry.lastInputBatchLength = data.length;
                entry.lastInputBatchAtMs = Date.now();
            }}
            sendTerminalEvent({{ kind: "input", data }});
            return true;
        }};
        const queueTerminalInputData = (data) => {{
            const text = String(data || '');
            if (!text) {{
                return;
            }}
            pendingTerminalInputData += text;
            setPendingTerminalInputDiagnostics('queue');
            if (
                pendingTerminalInputData.length >= terminalInputBatchMaxChars
                || /[\r\n\u0003\u0004]/.test(text)
            ) {{
                flushPendingTerminalInput('immediate');
                return;
            }}
            if (pendingTerminalInputTimer === null) {{
                pendingTerminalInputTimer = window.setTimeout(() => {{
                    flushPendingTerminalInput('timer');
                }}, terminalInputBatchDelayMs);
                setPendingTerminalInputDiagnostics('scheduled');
            }}
        }};
        const scheduleSettledResizePaint = () => {{
            if (settledResizePaintTimer !== null) {{
                window.clearTimeout(settledResizePaintTimer);
            }}
            if (settledResizeFollowupTimer !== null) {{
                window.clearTimeout(settledResizeFollowupTimer);
            }}
            // XTERM-BUG: switch-flicker. The two 5-phase follow cascades that
            // used to run here (settled_resize_paint + settled_resize_followup,
            // each now/raf/32/140/320ms) raced the reflow on a reveal/resize and
            // landed at a shifting target -> flicker + "random" scroll position.
            // Collapse to a SINGLE intent-guarded settled follow per timer:
            // follow only while FOLLOWING (never override UserScrollback) and
            // never while a selection is active. See [[audit-viewport-scroll-control-flow]].
            const settledResizeSingleFollow = (reason) => {{
                try {{
                    if (scrollbackIntent === 'UserScrollback') {{ return; }}
                    if (term && typeof term.hasSelection === 'function' && term.hasSelection()) {{ return; }}
                    scrollLiveCursorIntoView(true, reason);
                }} catch (_settledFollowError) {{}}
            }};
            settledResizePaintTimer = window.setTimeout(() => {{
                settledResizePaintTimer = null;
                requestVisiblePaint(false);
                settledResizeSingleFollow('settled_resize_paint');
            }}, 140);
            settledResizeFollowupTimer = window.setTimeout(() => {{
                settledResizeFollowupTimer = null;
                emitResize();
                requestVisiblePaint(true);
                settledResizeSingleFollow('settled_resize_followup');
                emitHostHealth();
            }}, 260);
        }};
        const scheduleRetainedWritePaintRepair = (reason) => {{
            if (retainedWritePaintRepairPending) {{
                return;
            }}
            retainedWritePaintRepairPending = true;
            retainedWritePaintRepairCount += 1;
            requestAnimationFrame(() => {{
                retainedWritePaintRepairPending = false;
                requestVisiblePaint(true);
                window.setTimeout(() => requestVisiblePaint(false), 96);
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].retainedWritePaintRepairCount =
                        retainedWritePaintRepairCount;
                    window.__yggtermXtermHosts[hostId].lastRetainedWritePaintRepairReason = reason;
                }}
            }});
        }};
        const requestRenderProbe = (reason) => {{
            const now = Date.now();
            const highFrequencyReason =
                reason === 'render' || reason === 'write_parsed' || reason === 'write_flush';
            if (
                terminalInputHot()
                && highFrequencyReason
            ) {{
                return;
            }}
            const frameLikeHot = recentFrameLikeWriteHot();
            const minRenderProbeIntervalMs = frameLikeHot
                ? terminalFrameLikeInstrumentationThrottleMs()
                : 220;
            if (
                frameLikeHot
                && highFrequencyReason
                && now - lastRenderProbeAtMs < minRenderProbeIntervalMs
            ) {{
                return;
            }}
            if (renderProbeFramePending || (now - lastRenderProbeAtMs < minRenderProbeIntervalMs)) {{
                return;
            }}
            renderProbeFramePending = true;
            requestAnimationFrame(() => {{
                renderProbeFramePending = false;
                lastRenderProbeAtMs = Date.now();
                enforceHelperTextareaContract();
                refreshCursorContrastContract();
                applySoftwareCanvasLayerOptimization(reason || 'render_probe');
                trackTerminalVisualState(reason);
                emitPaint();
            }});
        }};
        const termHasBufferedContent = () => {{
            try {{
                const buffer = term && term.buffer && term.buffer.active;
                if (!buffer) {{
                    return false;
                }}
                const baseY = Number(buffer.baseY || 0);
                if (baseY > 0) {{
                    return true;
                }}
                const length = Number(buffer.length || 0);
                const rows = Number(term && term.rows ? term.rows : 0);
                if (rows > 0 && length > rows) {{
                    return true;
                }}
                const sampleLimit = Math.min(length, 5);
                for (let offset = 0; offset < sampleLimit; offset += 1) {{
                    const line = buffer.getLine(Math.max(0, length - 1 - offset));
                    if (line && String(line.translateToString(true) || '').trim()) {{
                        return true;
                    }}
                }}
            }} catch (_error) {{}}
            return false;
        }};
        const ensureVisibleHost = (reason) => {{
            rebindCurrentHost(reason, true);
            const hadPriorPaintProbe = paintCount > 0;
            if (emitPaint()) {{
                return;
            }}
            if (termHasBufferedContent()) {{
                requestVisiblePaint();
                return;
            }}
            if (!hadPriorPaintProbe && reason === 'set_input_enabled') {{
                requestVisiblePaint();
                return;
            }}
            if (rebuildAttempts >= 1) {{
                return;
            }}
            rebuildAttempts += 1;
            sendTerminalEvent({{
                kind: "debug",
                message: `rebuild_blank_host host=${{hostId}} reason=${{reason}} attempts=${{rebuildAttempts}}`
            }});
            window.__yggtermRecordHostMutation && window.__yggtermRecordHostMutation({{
                host_id: hostId,
                site: 'rebuild_blank_host_wipe',
                reason: String(reason || ''),
                child_count: Number(host.childElementCount || 0),
                term_element_was_inside: Boolean(term && term.element && host.contains(term.element)),
            }});
            host.innerHTML = "";
            // ⛔ This is the LAST-RESORT recovery, so it is the one place a no-op
            // rebuild is most expensive: it ran only when the viewport was already
            // blank, and `term.open()` on an already-opened terminal rebuilt
            // NOTHING — the wipe above was pure loss and the host stayed empty for
            // good (rebuildAttempts caps this path at one try per mount).
            attachTerminalSurfaceToHost(host, 'rebuild_blank_host_attach', true);
            requestVisiblePaint();
        }};
        const focusTerminal = () => {{
            if (!inputEnabled) {{
                return;
            }}
            if (!hostOwnsActiveTerminalInput()) {{
                setInputEnabled(false, false);
                syncFocusClass();
                return;
            }}
            if (activeElementBlocksTerminalAutofocus()) {{
                syncFocusClass();
                return;
            }}
            rebindCurrentHost('focus_terminal', false);
            const applyFocusAttempt = () => {{
                if (activeElementBlocksTerminalAutofocus()) {{
                    syncFocusClass();
                    return false;
                }}
                const helperTextarea = enforceHelperTextareaContract();
                try {{
                    term.focus();
                }} catch (_error) {{}}
                try {{
                    if (helperTextarea && helperTextarea.focus) {{
                        helperTextarea.focus({{ preventScroll: true }});
                        if (helperTextarea.setSelectionRange) {{
                            const valueLength = Number(helperTextarea.value ? helperTextarea.value.length : 0);
                            helperTextarea.setSelectionRange(valueLength, valueLength);
                        }}
                    }} else if (host.focus) {{
                        host.focus({{ preventScroll: true }});
                    }}
                }} catch (_error) {{
                    try {{
                        host.focus();
                    }} catch (_error2) {{}}
                }}
                syncFocusClass();
                return Boolean(term && term.textarea && document.activeElement === term.textarea);
            }};
            applyFocusAttempt();
            window.requestAnimationFrame(() => {{
                applyFocusAttempt();
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 0);
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 32);
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 96);
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 220);
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 420);
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 760);
                window.setTimeout(() => {{
                    if (inputEnabled && !applyFocusAttempt()) {{
                        applyFocusAttempt();
                    }}
                }}, 1200);
            }});
        }};
        const scheduleInputDriftRecovery = () => {{
            if (!inputEnabled || !hostOwnsActiveTerminalInput() || activeElementBlocksTerminalAutofocus()) {{
                return;
            }}
            const repairFocus = () => {{
                if (!inputEnabled || !hostOwnsActiveTerminalInput() || activeElementBlocksTerminalAutofocus()) {{
                    return;
                }}
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                if (helperTextarea && document.activeElement === helperTextarea) {{
                    syncFocusClass();
                    return;
                }}
                focusTerminal();
            }};
            window.requestAnimationFrame(repairFocus);
            window.setTimeout(repairFocus, 0);
            window.setTimeout(repairFocus, 32);
            window.setTimeout(repairFocus, 96);
            window.setTimeout(repairFocus, 220);
            window.setTimeout(repairFocus, 420);
            window.setTimeout(repairFocus, 760);
            window.setTimeout(repairFocus, 1200);
        }};
        let inputPolicyApplyCount = 0;
        let inputPolicyNoopCount = 0;
        let inputPolicyNoopPromptFollowCount = 0;
        const maxInputPolicyNoopPromptFollows = 3;
        let lastInputPolicyNoopPromptFollowAtMs = 0;
        let lastInputPolicyNoopPromptFollowReason = '';
        let lastInputPolicyReason = '';
        let rustInputGateOpen = Boolean(inputEnabled);
        let retainedReplayPromotedToDaemonPtyCount = 0;
        let lastRetainedReplayPromotedAtMs = 0;
        let lastRetainedReplayPromotedFrom = '';
        let lastRetainedReplayPromotedReason = '';
        const retainedReplayContentSourceCanPromoteToDaemonPty = (source) => {{
            const value = String(source || '');
            return value === 'daemon_retained_history_screen_snapshot'
                || value === 'daemon_retained_snapshot'
                || value === 'daemon_screen_snapshot'
                || value === 'active_recovery_pty_snapshot'
                || value === 'daemon_terminal_read';
        }};
        const promoteRetainedReplaySourceForTrustedInput = (reason = '') => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return false;
                }}
                const source = String(entry.terminalContentSource || '');
                if (!retainedReplayContentSourceCanPromoteToDaemonPty(source)) {{
                    return false;
                }}
                entry.terminalContentSource = 'daemon_pty';
                entry.terminalSourceMismatchReason = '';
                entry.lastRetainedReplayPromptFollowReady = true;
                entry.lastRetainedReplaySupersededByDaemonPty = true;
                entry.lastRetainedReplayRejectedVisibleText = 'retained_replay_superseded_by_daemon_pty';
                try {{
                    const pending = window.__yggtermPendingRetainedReplays || {{}};
                    const pendingReplay = pending[sessionPath];
                    if (pendingReplay) {{
                        pendingReplay.complete = true;
                        pendingReplay.supersededByDaemonPty = true;
                        pendingReplay.supersededReason = String(reason || 'trusted_input_policy');
                    }}
                }} catch (_error) {{}}
                retainedReplayPromotedToDaemonPtyCount += 1;
                lastRetainedReplayPromotedAtMs = Date.now();
                lastRetainedReplayPromotedFrom = source;
                lastRetainedReplayPromotedReason = String(reason || 'trusted_input_policy');
                entry.retainedReplayPromotedToDaemonPtyCount = retainedReplayPromotedToDaemonPtyCount;
                entry.lastRetainedReplayPromotedAtMs = lastRetainedReplayPromotedAtMs;
                entry.lastRetainedReplayPromotedFrom = lastRetainedReplayPromotedFrom;
                entry.lastRetainedReplayPromotedReason = lastRetainedReplayPromotedReason;
                try {{
                    if (typeof entry.emitHostHealth === "function") {{
                        entry.emitHostHealth('retained_replay_promoted_to_daemon_pty');
                    }}
                }} catch (_error) {{}}
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        const syncInputPolicyHostEntry = () => {{
            try {{
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    const entry = window.__yggtermXtermHosts[hostId];
                    entry.inputEnabled = Boolean(inputEnabled);
                    entry.programmaticFocusEnabled = Boolean(programmaticFocusEnabled);
                    entry.rustInputGateOpen = Boolean(rustInputGateOpen);
                    entry.inputPolicyApplyCount = Number(inputPolicyApplyCount || 0);
                    entry.inputPolicyNoopCount = Number(inputPolicyNoopCount || 0);
                    entry.inputPolicyNoopPromptFollowCount = Number(inputPolicyNoopPromptFollowCount || 0);
                    entry.lastInputPolicyNoopPromptFollowAtMs = Number(lastInputPolicyNoopPromptFollowAtMs || 0);
                    entry.lastInputPolicyNoopPromptFollowReason = String(lastInputPolicyNoopPromptFollowReason || '');
                    entry.lastInputPolicyReason = String(lastInputPolicyReason || '');
                    entry.retainedReplayPromotedToDaemonPtyCount = Number(retainedReplayPromotedToDaemonPtyCount || 0);
                    entry.lastRetainedReplayPromotedAtMs = Number(lastRetainedReplayPromotedAtMs || 0);
                    entry.lastRetainedReplayPromotedFrom = String(lastRetainedReplayPromotedFrom || '');
                    entry.lastRetainedReplayPromotedReason = String(lastRetainedReplayPromotedReason || '');
                }}
            }} catch (_error) {{}}
        }};
        const maybeSettleCursorAfterInputPolicyNoop = (enabled, focus, followPrompt, reason = 'input_policy_unchanged') => {{
            try {{
                if (!Boolean(enabled) || !Boolean(focus) || !Boolean(followPrompt)) {{
                    return false;
                }}
                if (scrollbackIntent === 'UserScrollback') {{
                    return false;
                }}
                if (inputPolicyNoopPromptFollowCount >= maxInputPolicyNoopPromptFollows) {{
                    syncInputPolicyHostEntry();
                    return false;
                }}
                const now = Date.now();
                if (now - lastInputPolicyNoopPromptFollowAtMs < 700) {{
                    syncInputPolicyHostEntry();
                    return false;
                }}
                lastInputPolicyNoopPromptFollowAtMs = now;
                lastInputPolicyNoopPromptFollowReason = String(reason || 'input_policy_unchanged');
                inputPolicyNoopPromptFollowCount += 1;
                syncInputPolicyHostEntry();
                schedulePromptFollowAfterLayout(lastInputPolicyNoopPromptFollowReason);
                requestVisiblePaint(true);
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        // XTERM-BUG: input-dead-after-window-refocus
        // See docs/xterm-bugs.md#input-dead-after-window-refocus.
        //
        // This classifier is the single decision "may the passive watchdog put
        // focus back on the helper textarea", and it now NAMES its verdict
        // instead of returning a bare bool. When the user says "I came back from
        // another window and cannot type", the state snapshot has to be able to
        // answer *which gate refused* — before this, every one of the five exits
        // below was the same indistinguishable `false`.
        //
        // ⛔ REMOVED GATE — `document.hasFocus()`. It used to bail out here, and
        // it was both WRONG and REDUNDANT:
        //   * wrong: on KDE/Wayland `document.hasFocus()` is a measured false
        //     negative for a visibly foreground window
        //     ([[finding-wayland-focus-gate-squished-viewport]]; the same
        //     substitution was already made for the active write-frame budget and
        //     for the grid fit). Live on guihost 2026-07-23 the poller caught
        //     `document.hasFocus()===false` while rust had ALREADY re-opened the
        //     input gate on refocus — precisely the window in which focus drift is
        //     most likely, and precisely when this gate switched the only passive
        //     repair off.
        //   * redundant: rust owns `inputEnabled` and only opens it when
        //     `(window_focused || terminal_input_override_active) &&
        //     !app_control_backgrounded`, so the "is the window focused" condition
        //     is already enforced by `!inputEnabled` one line above. A gate that
        //     can only ever be wrong is pure loss.
        // Focusing an element in an unfocused document does not raise or activate
        // the window, so nothing here can steal focus from another app.
        const passiveFocusRecoveryState = () => {{
            if (!inputEnabled) {{
                // Rust refused input while it believes the window IS focused: the
                // webview cannot repair this one, only the rust policy can, and
                // naming it separately is what makes that distinction readable.
                const hostIsActiveSession = (() => {{
                    try {{
                        return host.getAttribute('data-active-session-host') === 'true';
                    }} catch (_error) {{
                        return false;
                    }}
                }})();
                return hostIsActiveSession && terminalWindowFocused()
                    ? 'rust_gate_closed_while_window_focused'
                    : 'input_disabled';
            }}
            if (!hostOwnsActiveTerminalInput()) {{
                return 'host_not_input_owner';
            }}
            const active = document.activeElement;
            const helperTextarea = host.querySelector('.xterm-helper-textarea');
            if (helperTextarea && active === helperTextarea) {{
                return 'focused';
            }}
            if (activeElementBlocksTerminalAutofocus()) {{
                return 'ui_focus_claim';
            }}
            const bodyOwnsFocus = (
                !active
                || active === document.body
                || active === document.documentElement
                || active === host
            );
            return bodyOwnsFocus ? 'recoverable' : 'foreign_active_element';
        }};
        const terminalNeedsPassiveFocusRecovery = () =>
            passiveFocusRecoveryState() === 'recoverable';
        let passiveFocusRecoveryCount = 0;
        let lastPassiveFocusRecoveryAtMs = 0;
        let inputDeadSinceMs = 0;
        let lastInputDeadTraceAtMs = 0;
        // "Input dead" = rust has opened the input gate for this session but the
        // keystroke sink (the xterm helper textarea) does NOT hold DOM focus, so
        // every key the user presses is dropped while the viewport keeps painting
        // normally. That is the exact shape of the 2026-07-23 report, and the app
        // recorded nothing about it: `helper_textarea_focused:false` was already in
        // the snapshot but carried no duration, so it was indistinguishable from a
        // terminal the user had simply not clicked yet. The DURATION is the signal.
        const recordPassiveFocusRecoveryState = (state) => {{
            try {{
                const now = Date.now();
                const dead = state === 'recoverable'
                    || state === 'foreign_active_element'
                    || state === 'rust_gate_closed_while_window_focused';
                if (dead) {{
                    if (inputDeadSinceMs === 0) {{
                        inputDeadSinceMs = now;
                    }}
                }} else {{
                    inputDeadSinceMs = 0;
                }}
                const deadMs = inputDeadSinceMs ? Math.max(0, now - inputDeadSinceMs) : 0;
                const active = document.activeElement;
                const activeDescription = active
                    ? `${{String(active.tagName || '')}}.${{String(active.className || '').split(/\s+/)[0] || ''}}#${{String(active.id || '')}}`
                    : 'null';
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry) {{
                    entry.passiveFocusRecoveryState = state;
                    entry.passiveFocusRecoveryCount = passiveFocusRecoveryCount;
                    entry.lastPassiveFocusRecoveryAtMs = lastPassiveFocusRecoveryAtMs;
                    entry.inputDeadSinceMs = inputDeadSinceMs;
                    entry.inputDeadMs = deadMs;
                    entry.inputDeadActiveElement = deadMs > 0 ? activeDescription : '';
                    entry.documentHasFocusAtLastWatchdog = terminalDocumentHasFocus();
                    entry.windowFocusedAtLastWatchdog = terminalWindowFocused();
                }}
                if (
                    deadMs >= {terminal_input_dead_trace_ms}
                    && now - lastInputDeadTraceAtMs >= {terminal_input_dead_trace_interval_ms}
                ) {{
                    lastInputDeadTraceAtMs = now;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `input_dead host=${{hostId}} state=${{state}} dead_ms=${{deadMs}}`
                            + ` active=${{activeDescription}} doc_focus=${{terminalDocumentHasFocus()}}`
                            + ` window_focused=${{terminalWindowFocused()}}`
                            + ` ui_claim=${{Date.now() < Number(window.__yggtermUiFocusClaimUntilMs || 0)}}`
                            + ` sidebar_owner=${{Boolean(window.__yggtermSidebarKeyboardOwner)}}`
                    }});
                }}
            }} catch (_error) {{}}
        }};
        let lastForeignActiveElementSeenAtMs = 0;
        const inputDriftWatchdog = window.setInterval(() => {{
            const recoveryState = passiveFocusRecoveryState();
            recordPassiveFocusRecoveryState(recoveryState);
            const now = Date.now();
            const deadMs = inputDeadSinceMs ? Math.max(0, now - inputDeadSinceMs) : 0;
            const allowForeignRecovery = recoveryState === 'foreign_active_element'
                && deadMs >= 1_200
                && inputEnabled
                && hostOwnsActiveTerminalInput();
            if (recoveryState !== 'recoverable' && !allowForeignRecovery) {{
                if (recoveryState === 'foreign_active_element') {{
                    if (lastForeignActiveElementSeenAtMs === 0) {{
                        lastForeignActiveElementSeenAtMs = now;
                    }}
                }} else {{
                    lastForeignActiveElementSeenAtMs = 0;
                }}
                return;
            }}
            lastForeignActiveElementSeenAtMs = 0;
            if (allowForeignRecovery) {{
                try {{
                    const active = document.activeElement;
                    if (active && active !== document.body && active !== document.documentElement
                        && typeof active.blur === 'function') {{
                        active.blur();
                    }}
                }} catch (_error) {{}}
            }}
            passiveFocusRecoveryCount += 1;
            lastPassiveFocusRecoveryAtMs = now;
            focusTerminal();
        }}, {terminal_passive_focus_watchdog_ms});
        let lastSeenActiveSessionPathForFocus = activeTerminalSessionPath();
        const sessionSwitchFocusPoll = window.setInterval(() => {{
            try {{
                const cur = activeTerminalSessionPath();
                if (cur === lastSeenActiveSessionPathForFocus) {{
                    return;
                }}
                lastSeenActiveSessionPathForFocus = cur;
                if (!hostOwnsActiveTerminalInput() || !inputEnabled) {{
                    return;
                }}
                const st = passiveFocusRecoveryState();
                if (st === 'foreign_active_element' || st === 'recoverable') {{
                    focusTerminal();
                }}
            }} catch (_error) {{}}
        }}, 320);
        // Screen-restore (vacuum fix): periodically persist the rendered transcript
        // to localStorage so a full GUI+daemon restart can restore it. The
        // event-driven persists (scroll/intent/snapshot) never fire for an IDLE
        // freshly-opened session, so its transcript was never saved (the 2.8.54
        // failure). The persist's collapse-guard keeps a prior rich transcript from
        // being overwritten by the sparse re-resume buffer. Skipped while a restore
        // is in flight.
        const screenRestorePersistTimer = window.setInterval(() => {{
            try {{
                const restoreInFlight = Boolean(pendingPersistedScrollRestore)
                    || (pendingPersistedScrollRestoreDeadlineMs > 0
                        && Date.now() <= pendingPersistedScrollRestoreDeadlineMs);
                if (restoreInFlight) {{ return; }}
                persistScrollStateToLocalStorage('periodic_screen_restore');
            }} catch (_error) {{}}
        }}, 4000);
        // REPAINT-STORM PROBE STALENESS FIX (2026-07-22, found while chasing a
        // live "high-FPS blink on switch" report). The rate window only closes
        // inside emitPaint, so a host that STOPS painting keeps reporting its
        // last rate forever: a quiescent host read 16/s for 38s straight, which
        // is indistinguishable from a live 16/s burst, and a storm that ended
        // never cleared `repaintStormMs`. Close an expired window from a timer
        // too, so the rate decays to 0 and the storm flag resolves on its own.
        // `paintRateAtMs` lets a reader see how fresh the number is at all.
        const paintRateDecayTimer = window.setInterval(() => {{
            try {{
                if (paintRateWindowStartMs === 0) {{ return; }}
                const decayNowMs = Date.now();
                if (decayNowMs - paintRateWindowStartMs < 1000) {{ return; }}
                const observedRate = paintRateWindowCount;
                paintRateWindowStartMs = decayNowMs;
                paintRateWindowCount = 0;
                if (observedRate < 30) {{ repaintStormSinceMs = 0; }}
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].paintRatePerSec = observedRate;
                    window.__yggtermXtermHosts[hostId].repaintStormMs =
                        repaintStormSinceMs ? Math.max(0, decayNowMs - repaintStormSinceMs) : 0;
                    window.__yggtermXtermHosts[hostId].paintRateAtMs = decayNowMs;
                }}
            }} catch (_error) {{}}
        }}, 1000);
        // Settle-follow watchdog — the EXECUTOR for scroll_mode.rs
        // `should_settle_follow` (Following + viewport stranded below base →
        // re-assert to the current baseY). The oracle shipped with tests but no
        // executor called it; live-pinned consequence (practice 2026-06-10): a
        // reveal replay left viewport_y=0 under base_y=985 with
        // intent=PromptFollow and NOTHING ever re-followed (force-foreground
        // removes the focus-gain edge; an idle TUI emits no output edge) — the
        // user saw a blank/stale top-of-buffer "squish" until a forced refresh.
        // Decision mirrors the oracle exactly: only PromptFollow (Pinned =
        // UserScrollback and Selecting never auto-move), never while input-hot
        // or a persisted-restore is in flight, strand threshold 2 rows (=
        // PIN_THRESHOLD_LINES) so output-burst jitter never triggers it.
        const settleFollowWatchdog = window.setInterval(() => {{
            try {{
                if (scrollbackIntent !== 'PromptFollow') {{ return; }}
                if (term && typeof term.hasSelection === 'function' && term.hasSelection()) {{ return; }}
                if (Date.now() < terminalInputHotUntilMs) {{ return; }}
                const restoreInFlight = Boolean(pendingPersistedScrollRestore)
                    || (pendingPersistedScrollRestoreDeadlineMs > 0
                        && Date.now() <= pendingPersistedScrollRestoreDeadlineMs);
                if (restoreInFlight) {{ return; }}
                const buf = term && term.buffer ? term.buffer.active : null;
                if (!buf) {{ return; }}
                // Measure the EFFECTIVE viewport (the same clamped value the app
                // state and the follow executor satisfy). Raw buf.viewportY can
                // sit below it permanently (visual-beyond-base clamp), which made
                // the watchdog loop a 1s re-assert against a host that was
                // already visually at the bottom (live-caught on first deploy).
                const vy = Math.max(0, Number(
                    (typeof effectiveXtermViewportY === 'function'
                        ? effectiveXtermViewportY(buf)
                        : buf.viewportY) || 0
                ));
                const by = Math.max(0, Number(buf.baseY || 0));
                if (by - vy < 2) {{ return; }}
                scrollLiveCursorIntoView(false, 'settle_follow_watchdog');
                sendTerminalEvent({{
                    kind: 'debug',
                    message: `settle_follow_reassert host=${{hostId}} from=${{vy}} to=${{by}}`
                }});
            }} catch (_error) {{}}
        }}, 1000);
        const setInputEnabled = (enabled, focus, followPrompt = true, policySource = 'local') => {{
            const requestedEnabled = Boolean(enabled);
            const policyTrusted = String(policySource || '') === 'rust_policy';
            if (policyTrusted) {{
                rustInputGateOpen = requestedEnabled;
            }}
            const canOwnActiveInput = !requestedEnabled || hostOwnsActiveTerminalInput();
            const nextInputEnabled =
                requestedEnabled && Boolean(canOwnActiveInput) && Boolean(rustInputGateOpen);
            const nextProgrammaticFocusEnabled = nextInputEnabled && Boolean(focus);
            let helperAlreadyFocused = false;
            try {{
                helperAlreadyFocused = Boolean(term && term.textarea && document.activeElement === term.textarea);
            }} catch (_error) {{}}
            let stdinAlreadyMatches = true;
            try {{
                stdinAlreadyMatches = Boolean(term.options.disableStdin) === !nextInputEnabled;
            }} catch (_error) {{}}
            const inputPolicyUnchanged =
                inputEnabled === nextInputEnabled
                && programmaticFocusEnabled === nextProgrammaticFocusEnabled
                && stdinAlreadyMatches;
            const focusAlreadySatisfied = !nextProgrammaticFocusEnabled || helperAlreadyFocused;
            if (inputPolicyUnchanged && focusAlreadySatisfied) {{
                inputPolicyNoopCount += 1;
                lastInputPolicyReason = 'unchanged';
                if (nextInputEnabled && rustInputGateOpen) {{
                    promoteRetainedReplaySourceForTrustedInput('input_policy_unchanged');
                }}
                syncXtermInputLineDecoration('input_policy_unchanged');
                maybeSettleCursorAfterInputPolicyNoop(
                    nextInputEnabled,
                    nextProgrammaticFocusEnabled,
                    followPrompt,
                    'input_policy_unchanged'
                );
                syncInputPolicyHostEntry();
                if (!nextInputEnabled) {{
                    try {{
                        host.classList.remove('yggterm-term-focused');
                    }} catch (_error) {{}}
                    try {{
                        if (term && typeof term.blur === "function") {{
                            term.blur();
                        }}
                    }} catch (_error) {{}}
                    try {{
                        const helperTextarea = host.querySelector('.xterm-helper-textarea');
                        if (helperTextarea && helperTextarea.blur) {{
                            helperTextarea.blur();
                        }}
                    }} catch (_error) {{}}
                    try {{
                        if (document.activeElement === host && host.blur) {{
                            host.blur();
                        }}
                    }} catch (_error) {{}}
                    syncInputPolicyHostEntry();
                }}
                try {{
                    if (typeof syncTerminalWriteFrameBudgetHostEntry === 'function') {{
                        syncTerminalWriteFrameBudgetHostEntry();
                    }}
                }} catch (_error) {{}}
                return;
            }}
            inputPolicyApplyCount += 1;
            lastInputPolicyReason = inputPolicyUnchanged ? 'focus_repair' : 'changed';
            inputEnabled = nextInputEnabled;
            programmaticFocusEnabled = nextProgrammaticFocusEnabled;
            if (inputEnabled && rustInputGateOpen) {{
                promoteRetainedReplaySourceForTrustedInput('input_policy_changed');
            }}
            try {{
                term.options.disableStdin = !inputEnabled;
            }} catch (_error) {{}}
            host.style.cursor = inputEnabled ? 'text' : 'default';
            syncInputPolicyHostEntry();
            if (!inputEnabled) {{
                try {{
                    captureSessionXtermSnapshot('focus_released');
                }} catch (_error) {{}}
                host.classList.remove('yggterm-term-focused');
                try {{
                    if (term && typeof term.blur === "function") {{
                        term.blur();
                    }}
                }} catch (_error) {{}}
                try {{
                    const helperTextarea = host.querySelector('.xterm-helper-textarea');
                    if (helperTextarea && helperTextarea.blur) {{
                        helperTextarea.blur();
                    }}
                }} catch (_error) {{}}
                try {{
                    if (document.activeElement === host && host.blur) {{
                        host.blur();
                    }}
                }} catch (_error) {{}}
                applySoftwareCanvasLayerOptimization('focus_released');
                disposeXtermInputLineDecoration('focus_released');
                syncInputPolicyHostEntry();
                return;
            }}
            ensureVisibleHost('set_input_enabled');
            emitResize();
            const focusShouldFollowPrompt = Boolean(focus) && Boolean(followPrompt) && scrollbackIntent !== 'UserScrollback';
            scrollLiveCursorIntoView(focusShouldFollowPrompt, focus ? 'focus' : 'set_input_enabled');
            requestVisiblePaint();
            window.requestAnimationFrame(() => {{
                stretchXtermRoot();
                emitResize();
                requestVisiblePaint();
                emitHostHealth();
            }});
            window.setTimeout(() => {{
                stretchXtermRoot();
                emitResize();
                requestVisiblePaint();
                emitHostHealth();
            }}, 32);
            window.setTimeout(() => {{
                stretchXtermRoot();
                emitResize();
                requestVisiblePaint();
                emitHostHealth();
            }}, 140);
            if (focus) {{
                focusTerminal();
            }} else {{
                syncFocusClass();
                scheduleInputDriftRecovery();
            }}
            syncXtermInputLineDecoration('host_stdin_enabled');
            syncInputPolicyHostEntry();
            try {{
                if (typeof syncTerminalWriteFrameBudgetHostEntry === 'function') {{
                    syncTerminalWriteFrameBudgetHostEntry();
                }}
            }} catch (_error) {{}}
        }};
        const shouldHandleWheel = (event) => {{
            if (!event || !hostOwnsActiveTerminalInput()) {{
                return false;
            }}
            if (terminalOwnsWheelInput()) {{
                return false;
            }}
            const target = event.target;
            return Boolean(target && host.contains(target));
        }};
        const handleWheel = (event) => {{
            if (!shouldHandleWheel(event)) {{
                return;
            }}
            if (!event || !Number.isFinite(event.deltaY) || event.deltaY === 0) {{
                return;
            }}
            wheelEventCount += 1;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].wheelEventCount = wheelEventCount;
                window.__yggtermXtermHosts[hostId].lastWheelDeltaY = Number(event.deltaY || 0);
            }}
            // SCROLL-OWNERSHIP (boring loads spec): the user's wheel owns the
            // viewport from this moment. A pending persisted-scroll restore
            // (armed at construct, polled for up to 8s) must never fire after
            // real user scroll input — re-applying the saved offset over the
            // user's position was the post-reveal "yanks to its liking spot"
            // fight.
            if (pendingPersistedScrollRestore) {{
                pendingPersistedScrollRestore = null;
                sendTerminalEvent({{
                    kind: 'debug',
                    message: `persisted_scroll_restore_cancelled host=${{hostId}} reason=user_wheel`
                }});
            }}
            const deltaLines = Math.max(1, Math.round(Math.abs(event.deltaY) / 40));
            revealSoftwareCanvasLinkLayer('wheel');
            const activeBuffer = term && term.buffer ? term.buffer.active : null;
            const wheelDebug = {{
                delta_y: Number(event.deltaY || 0),
                delta_lines: deltaLines,
                had_active_buffer: Boolean(activeBuffer),
                before_viewport_y: activeBuffer ? Number(activeBuffer.viewportY || 0) : null,
                before_base_y: activeBuffer ? Number(activeBuffer.baseY || 0) : null,
            }};
                if (activeBuffer) {{
                    const currentViewportY = Number(activeBuffer.viewportY || 0);
                    const baseY = Number(activeBuffer.baseY || 0);
                    const targetViewportY = Math.max(
                        0,
                    Math.min(baseY, currentViewportY + (event.deltaY > 0 ? deltaLines : -deltaLines)),
                );
                if (targetViewportY < baseY) {{
                    setScrollbackIntent('UserScrollback', 'wheel');
                }} else {{
                    setScrollbackIntent('PromptFollow', 'wheel_reached_bottom');
                }}
                wheelDebug.target_viewport_y = targetViewportY;
                wheelDebug.force_viewport = forceXtermViewportY(targetViewportY, 'wheel');
                wheelDebug.after_internal_viewport_y = Number(activeBuffer.viewportY || 0);
            }} else {{
                const scrollLinesDelta = event.deltaY > 0 ? deltaLines : -deltaLines;
                wheelDebug.scroll_lines_delta = scrollLinesDelta;
                if (scrollLinesDelta < 0) {{
                    setScrollbackIntent('UserScrollback', 'wheel_scroll_lines');
                }}
                term.scrollLines(scrollLinesDelta);
            }}
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].lastWheelScrollDebug = wheelDebug;
            }}
            syncScrollbackLock('wheel');
            syncTerminalScrollController('wheel');
            event.preventDefault();
            if (event.stopImmediatePropagation) {{
                event.stopImmediatePropagation();
            }}
            event.stopPropagation();
        }};
        term.attachCustomWheelEventHandler((event) => {{
            if (terminalOwnsWheelInput()) {{
                return true;
            }}
            if (!shouldHandleWheel(event)) {{
                return true;
            }}
            handleWheel(event);
            return false;
        }});
        // No context-menu dismissal here. It used to live in this handler and it
        // was the app's only click-outside dismiss: it fired only for clicks
        // inside this host's rect, closed only the cwd tree's menu, and went
        // missing exactly while the host was blank or remounting. The menu's own
        // backdrop owns dismissal now, for every menu and every click.
        const handleHostPointerFocus = (event) => {{
            revealSoftwareCanvasLinkLayer('pointer_focus');
            releaseBlockingUiFocusForTerminalReclaim();
            refreshCursorContrastContract();
            // Reclaim stdin on pointerdown so settings/search focus cannot trap the
            // terminal, but leave helper-textarea focus until release so xterm's
            // native drag selection continues to work.
            setInputEnabled(true, false);
            window.requestAnimationFrame(() => setInputEnabled(true, false));
            window.setTimeout(() => setInputEnabled(true, false), 0);
            window.setTimeout(() => setInputEnabled(true, false), 48);
            window.setTimeout(() => setInputEnabled(true, false), 140);
            window.setTimeout(() => setInputEnabled(true, false), 280);
            window.setTimeout(() => setInputEnabled(true, false), 420);
            window.setTimeout(() => setInputEnabled(true, false), 760);
        }};
        const retainTerminalFocusAfterPointerRelease = (_event) => {{
            revealSoftwareCanvasLinkLayer('pointer_release');
            refreshCursorContrastContract();
            setInputEnabled(true, true, false);
            window.requestAnimationFrame(() => setInputEnabled(true, true, false));
            window.setTimeout(() => setInputEnabled(true, true, false), 0);
            window.setTimeout(() => setInputEnabled(true, true, false), 32);
            window.setTimeout(() => setInputEnabled(true, true, false), 96);
            window.requestAnimationFrame(() => focusTerminal());
            window.setTimeout(() => focusTerminal(), 0);
            window.setTimeout(() => focusTerminal(), 32);
            window.setTimeout(() => focusTerminal(), 96);
            window.setTimeout(() => focusTerminal(), 220);
        }};
        const handleHostPointerMove = (_event) => {{
            revealSoftwareCanvasLinkLayer('pointer_move');
        }};
        const handleHostPointerLeave = (_event) => {{
            softwareCanvasLinkRevealUntilMs = 0;
            applySoftwareCanvasLayerOptimization('pointer_leave');
        }};
        detachHostInteractions = (targetHost) => {{
            if (!targetHost) {{
                return;
            }}
            try {{
                targetHost.removeEventListener("wheel", handleWheel, true);
            }} catch (_error) {{}}
            try {{
                targetHost.removeEventListener("pointerdown", handleTerminalSecondaryButton, true);
                targetHost.removeEventListener("mousedown", handleTerminalSecondaryButton, true);
                targetHost.removeEventListener("pointerup", handleTerminalSecondaryButton, true);
                targetHost.removeEventListener("mouseup", handleTerminalSecondaryButton, true);
                targetHost.removeEventListener("auxclick", handleTerminalSecondaryButton, true);
                targetHost.removeEventListener("pointerdown", handleHostPointerFocus, true);
                targetHost.removeEventListener("mousedown", handleHostPointerFocus, true);
                targetHost.removeEventListener("pointerup", retainTerminalFocusAfterPointerRelease, true);
                targetHost.removeEventListener("mouseup", retainTerminalFocusAfterPointerRelease, true);
                targetHost.removeEventListener("click", retainTerminalFocusAfterPointerRelease, true);
                targetHost.removeEventListener("mousedown", handlePrimarySelectionSyncPointerDown, true);
                targetHost.removeEventListener("mouseup", handlePrimarySelectionSyncPointerUp, true);
                targetHost.removeEventListener("mousedown", handlePrimarySelectionMiddleClick, true);
                targetHost.removeEventListener("auxclick", handlePrimarySelectionMiddleClick, true);
                targetHost.removeEventListener("contextmenu", handleTerminalContextMenu, true);
                targetHost.removeEventListener("pointermove", handleHostPointerMove, true);
                targetHost.removeEventListener("mousemove", handleHostPointerMove, true);
                targetHost.removeEventListener("pointerleave", handleHostPointerLeave, true);
                targetHost.removeEventListener("mouseleave", handleHostPointerLeave, true);
            }} catch (_error) {{}}
        }};
        attachHostInteractions = (targetHost) => {{
            if (!targetHost) {{
                return;
            }}
            targetHost.addEventListener("wheel", handleWheel, {{ passive: false, capture: true }});
            targetHost.addEventListener("pointerdown", handleTerminalSecondaryButton, true);
            targetHost.addEventListener("mousedown", handleTerminalSecondaryButton, true);
            targetHost.addEventListener("pointerup", handleTerminalSecondaryButton, true);
            targetHost.addEventListener("mouseup", handleTerminalSecondaryButton, true);
            targetHost.addEventListener("auxclick", handleTerminalSecondaryButton, true);
            targetHost.addEventListener("pointerdown", handleHostPointerFocus, true);
            targetHost.addEventListener("mousedown", handleHostPointerFocus, true);
            targetHost.addEventListener("pointerup", retainTerminalFocusAfterPointerRelease, true);
            targetHost.addEventListener("mouseup", retainTerminalFocusAfterPointerRelease, true);
            targetHost.addEventListener("click", retainTerminalFocusAfterPointerRelease, true);
            // CC-DRAG-STALL: the sync-flush listeners are registered BEFORE
            // handlePrimarySelectionMiddleClick so its stopImmediatePropagation
            // (middle button) can never skip the flush.
            targetHost.addEventListener("mousedown", handlePrimarySelectionSyncPointerDown, true);
            targetHost.addEventListener("mouseup", handlePrimarySelectionSyncPointerUp, true);
            targetHost.addEventListener("mousedown", handlePrimarySelectionMiddleClick, true);
            targetHost.addEventListener("auxclick", handlePrimarySelectionMiddleClick, true);
            targetHost.addEventListener("contextmenu", handleTerminalContextMenu, true);
            targetHost.addEventListener("pointermove", handleHostPointerMove, true);
            targetHost.addEventListener("mousemove", handleHostPointerMove, true);
            targetHost.addEventListener("pointerleave", handleHostPointerLeave, true);
            targetHost.addEventListener("mouseleave", handleHostPointerLeave, true);
        }};
        const pointerEventFallsWithinHost = (event) => {{
            try {{
                if (!event) {{
                    return false;
                }}
                const rect = host.getBoundingClientRect();
                const clientX = Number(event.clientX || 0);
                const clientY = Number(event.clientY || 0);
                return (
                    clientX >= rect.left
                    && clientX <= rect.right
                    && clientY >= rect.top
                    && clientY <= rect.bottom
                );
            }} catch (_error) {{
                return false;
            }}
        }};
        const hostContainsEventTarget = (target) => {{
            try {{
                return Boolean(target && host.contains(target));
            }} catch (_error) {{
                return false;
            }}
        }};
        const handleDocumentPointerCapture = (event) => {{
            const target = event && event.target ? event.target : null;
            const eventType = String(event && event.type || '');
            if (
                (eventType === 'contextmenu' || Number(event && event.button) === 2)
                && (pointerEventFallsWithinHost(event) || hostContainsEventTarget(target))
            ) {{
                handleTerminalSecondaryButton(event);
                return;
            }}
            const titlebarSearch = target && target.closest
                ? target.closest('[data-yggterm-titlebar-search="1"]')
                : null;
            if (titlebarSearch) {{
                markTransientUiFocusClaim(760);
                const focusSearchInput = () => {{
                    try {{
                        const input = document.getElementById({SEARCH_INPUT_ID:?});
                        if (!input || typeof input.focus !== 'function') {{
                            return;
                        }}
                        input.focus({{ preventScroll: true }});
                        if (typeof input.setSelectionRange === 'function') {{
                            const valueLength = Number(input.value ? input.value.length : 0);
                            input.setSelectionRange(valueLength, valueLength);
                        }}
                    }} catch (_error) {{}}
                }};
                focusSearchInput();
                window.requestAnimationFrame(focusSearchInput);
                window.setTimeout(focusSearchInput, 0);
                window.setTimeout(focusSearchInput, 32);
                return;
            }}
            const settingsField = target && target.closest
                ? target.closest('[data-settings-field-key]')
                : null;
            if (settingsField && typeof settingsField.focus === 'function') {{
                markTransientUiFocusClaim(760);
                const focusSettingsField = () => {{
                    try {{
                        settingsField.focus({{ preventScroll: true }});
                        if (typeof settingsField.setSelectionRange === 'function') {{
                            const valueLength = Number(settingsField.value ? settingsField.value.length : 0);
                            settingsField.setSelectionRange(valueLength, valueLength);
                        }}
                    }} catch (_error) {{}}
                }};
                focusSettingsField();
                window.requestAnimationFrame(focusSettingsField);
                window.setTimeout(focusSettingsField, 0);
                return;
            }}
            if (elementBlocksTerminalAutofocus(target)) {{
                markTransientUiFocusClaim(760);
                return;
            }}
            const hostPointerDown = eventType === 'pointerdown' || eventType === 'mousedown';
            const hostPointerRelease =
                eventType === 'pointerup' || eventType === 'mouseup' || eventType === 'click';
            if (pointerEventFallsWithinHost(event) || hostContainsEventTarget(target)) {{
                if (hostPointerDown) {{
                    handleHostPointerFocus(event);
                }} else if (hostPointerRelease) {{
                    retainTerminalFocusAfterPointerRelease(event);
                }}
            }}
        }};
        const handleDocumentFocusIn = (event) => {{
            const target = event && event.target ? event.target : null;
            if (elementBlocksTerminalAutofocus(target)) {{
                markTransientUiFocusClaim(760);
            }}
        }};
        document.addEventListener("pointerdown", handleDocumentPointerCapture, true);
        document.addEventListener("mousedown", handleDocumentPointerCapture, true);
        document.addEventListener("mouseup", handleDocumentPointerCapture, true);
        document.addEventListener("click", handleDocumentPointerCapture, true);
        document.addEventListener("contextmenu", handleDocumentPointerCapture, true);
        document.addEventListener("focusin", handleDocumentFocusIn, true);
        const liveCursorNearBottom = () => {{
            try {{
                if (!term || !term.buffer || !term.buffer.active) {{
                    return true;
                }}
                const active = term.buffer.active;
                const viewportY = effectiveXtermViewportY(active);
                const baseY = Math.max(0, Number(active.baseY || 0));
                const rows = Math.max(1, Number(term.rows || 0));
                return (baseY - viewportY) <= Math.max(2, Math.ceil(rows / 3));
            }} catch (_error) {{
                return true;
            }}
        }};
        const scrollLiveCursorIntoView = (force = false, reason = '') => {{
            try {{
                if (!term || !term.buffer || !term.buffer.active) {{
                    return;
                }}
                if (!force && scrollbackIntent === 'UserScrollback') {{
                    syncScrollbackLock(reason || 'user_scrollback');
                    return;
                }}
                // DEFECT #1 FIXED (favourite-spot incident 2026-06-11): this
                // used to ALSO early-return on `syncScrollbackLock()` — i.e.
                // whenever viewport < base — which no-opped every non-forced
                // follow in exactly the stranded state the follow exists to
                // fix (litellm pinned at 705 under base 943 while the settle
                // watchdog "re-asserted" 1/s with zero effect for 50s+). The
                // lock conflated "user is reading scrollback" with "viewport
                // below base". User intent is owned SOLELY by the intent SSOT
                // above (UserScrollback latches via the harness-locked
                // scroll-up detector); a PromptFollow session below base is a
                // strand and MUST follow.
                if (!force) {{
                    syncScrollbackLock(reason || 'prompt_follow');
                }}
                if (force) {{
                    setScrollbackIntent('PromptFollow', reason || 'prompt_follow');
                    scrollbackLocked = false;
                    promptFollowScrollGuardUntilMs = Date.now() + 180;
                    if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                        window.__yggtermXtermHosts[hostId].scrollbackLocked = false;
                    }}
                }}
                const active = term.buffer.active;
                const baseY = Math.max(0, Number(active.baseY || 0));
                const cursorY = Math.max(0, Number(active.cursorY || 0));
                const cursorLineIndex = Math.max(0, baseY + cursorY);
                const rows = Math.max(1, Number(term.rows || 0));
                const keepVisibleMargin = Math.max(4, Math.ceil(rows * 0.18));
                const targetLine = Math.max(0, cursorLineIndex - keepVisibleMargin);
                const viewportY = Math.max(0, Number(active.viewportY || 0));
                const viewportBottomY = viewportY + Math.max(0, rows - 1);
                const cursorVisibleInViewport =
                    cursorLineIndex >= viewportY && cursorLineIndex <= viewportBottomY;
                const cursorAboveViewport = cursorLineIndex < viewportY;
                const cursorBelowViewport = cursorLineIndex > viewportBottomY;
                if (force) {{
                    forceXtermViewportY(baseY, reason || 'prompt_follow');
                }} else if (cursorVisibleInViewport) {{
                    syncXtermViewportElementToBuffer(viewportY);
                }} else if (cursorBelowViewport) {{
                    forceXtermViewportY(baseY, reason || 'prompt_follow');
                }} else if (cursorAboveViewport) {{
                    forceXtermViewportY(targetLine, reason || 'prompt_follow');
                }} else if (typeof term.scrollToBottom === 'function') {{
                    forceXtermViewportY(baseY, reason || 'prompt_follow');
                }}
                syncScrollbackLock(reason || 'prompt_follow');
                applySoftwareCanvasLayerOptimization(reason || 'prompt_follow');
            }} catch (_error) {{}}
        }};
        let lastPrimarySelectionMiddleClickAtMs = 0;
        // One-shot deadline: handleClipboardPaste suppresses exactly ONE native
        // clipboard 'paste' echo before this time (set on a middle-click), then
        // clears it — so a later Ctrl+Shift+V is never swallowed.
        let pendingMiddleClickEchoUntilMs = 0;
        const primarySelectionSessionPath = () => host.getAttribute("data-terminal-session-path") || "";
        const syncPrimarySelectionHostEntry = (text, reason = '') => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (!entry) {{
                    return;
                }}
                entry.primarySelectionText = String(text || '');
                entry.primarySelectionLength = String(text || '').length;
                entry.primarySelectionUpdatedAtMs = Date.now();
                entry.lastPrimarySelectionReason = String(reason || 'selection_change');
            }} catch (_error) {{}}
        }};
        const recordPrimarySelectionFromXterm = (reason = 'selection_change') => {{
            try {{
                if (!term || typeof term.getSelection !== 'function') {{
                    return false;
                }}
                const text = String(term.getSelection() || '');
                if (!text) {{
                    return false;
                }}
                const sessionPath = primarySelectionSessionPath();
                window.__yggtermPrimarySelection = {{
                    text,
                    hostId,
                    sessionPath,
                    updatedAtMs: Date.now(),
                    source: 'xterm',
                    reason: String(reason || 'selection_change'),
                }};
                syncPrimarySelectionHostEntry(text, reason);
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        // CC-DRAG-STALL: term.onSelectionChange fires per selection delta
        // during a drag AND per streamed write that shifts the buffer under a
        // live selection — an agent CLI that streams constantly multiplies the
        // events. The old handler ran term.getSelection() (an
        // O(selected-cells) serialization that grows as the drag grows) plus
        // host-entry sync, a canvas-layer pass, and health telemetry on EVERY
        // firing, on the same webview thread as the xterm write pump — that
        // thread contention was the felt UX stall. Per-event work is now O(1):
        // the scroll pin stays immediate in the handler, and the expensive
        // half coalesces to a trailing edge (one animation frame). The flush
        // also runs synchronously on host mousedown/mouseup and before any
        // primary-selection paste, so window.__yggtermPrimarySelection always
        // holds the FINAL selection by the time anything can read it —
        // middle-click paste immediately after drag-end must never see a
        // stale or partial selection.
        let primarySelectionSyncPending = false;
        let primarySelectionSyncScheduleHandle = null;
        let primarySelectionSyncScheduleKind = '';
        const runPrimarySelectionSync = (reason) => {{
            recordPrimarySelectionFromXterm(reason);
            applySoftwareCanvasLayerOptimization('selection_change');
            emitHostHealthThrottled();
        }};
        const cancelScheduledPrimarySelectionSync = () => {{
            if (primarySelectionSyncScheduleHandle === null) {{
                return;
            }}
            try {{
                if (primarySelectionSyncScheduleKind === 'raf'
                    && typeof window.cancelAnimationFrame === 'function') {{
                    window.cancelAnimationFrame(primarySelectionSyncScheduleHandle);
                }} else {{
                    window.clearTimeout(primarySelectionSyncScheduleHandle);
                }}
            }} catch (_error) {{}}
            primarySelectionSyncScheduleHandle = null;
            primarySelectionSyncScheduleKind = '';
        }};
        const flushPrimarySelectionSync = (reason = 'selection_change') => {{
            cancelScheduledPrimarySelectionSync();
            if (!primarySelectionSyncPending) {{
                return;
            }}
            primarySelectionSyncPending = false;
            runPrimarySelectionSync(reason);
        }};
        // CC-DRAG-STALL RESIDUAL (user-reported 2026-07-31, still freezing on
        // 2.12.19 after the O(1) handler landed). Making per-EVENT work O(1)
        // was necessary and not sufficient: the deferred half still coalesced
        // to requestAnimationFrame, so during a live drag over a streaming
        // session `runPrimarySelectionSync` — and with it the
        // O(selected-cells) `term.getSelection()` serialization — ran once per
        // ANIMATION FRAME (~60/s), on the same webview thread as the xterm
        // write pump. The commit title said a drag's cost must not grow with
        // the selection it drags; per-frame, it still did.
        //
        // Nothing can observe __yggtermPrimarySelection mid-drag: every reader
        // flushes synchronously first (pointerdown, pointerup, and
        // primarySelectionTextForPaste flushes EVERY host before it reads). So
        // while the pointer is down the rAF flush is pure redundancy — skip it
        // and let drag-end do the one serialization that is actually observed.
        // Cost during a drag is now O(1) per event AND O(1) per frame, with a
        // single O(selected-cells) pass at mouseup.
        let primarySelectionPointerDragActive = false;
        let dragSelectionChangeCount = 0;
        let dragBeganAtMs = 0;
        const schedulePrimarySelectionSync = () => {{
            primarySelectionSyncPending = true;
            if (primarySelectionPointerDragActive) {{
                // Drag in flight: count it for the lifecycle report, stay
                // pending, and schedule NOTHING — pointerup (or a cross-host
                // paste flush) performs the one serialization anyone observes.
                // One block, one exit: the counter lives inside the guard so
                // there is exactly one `primarySelectionPointerDragActive`
                // test in this function. Two of them let a lock anchor on the
                // wrong one and stay green while the guard was deleted.
                dragSelectionChangeCount += 1;
                return;
            }}
            if (primarySelectionSyncScheduleHandle !== null) {{
                return;
            }}
            const flushScheduledPrimarySelectionSync = () => {{
                primarySelectionSyncScheduleHandle = null;
                primarySelectionSyncScheduleKind = '';
                flushPrimarySelectionSync('selection_change');
            }};
            if (typeof window.requestAnimationFrame === 'function') {{
                primarySelectionSyncScheduleKind = 'raf';
                primarySelectionSyncScheduleHandle =
                    window.requestAnimationFrame(flushScheduledPrimarySelectionSync);
            }} else {{
                primarySelectionSyncScheduleKind = 'timeout';
                primarySelectionSyncScheduleHandle =
                    window.setTimeout(flushScheduledPrimarySelectionSync, 16);
            }}
        }};
        // A selection that is gone can no longer hold the pin, whether or not
        // the release then fires (it deliberately does not fire for someone who
        // scrolled up first). Returns true so it can sit at the head of that
        // condition and drop the claim exactly once.
        const releaseSelectionPinClaim = () => {{
            selectionOwnsScrollbackPin = false;
            return true;
        }};
        // SELECTION ⇒ VIEWPORT PIN, one owner for both directions.
        // ⛔ THE QUIET-GATE LAW (see the campaign): this decision used to hang
        // ONLY off term.onSelectionChange, and that event does not arrive when
        // the workload is an agent CLI streaming output. Measured on a shadow,
        // same drag geometry, same session:
        //   idle      →   967 chars, 2 selection-change events, pin ARMED
        //   streaming → 902,649 chars, 0 selection-change events, pin NEVER ARMED
        // With the pin unarmed the viewport keeps auto-following the stream, so
        // the drag's end anchor chases it through the buffer and the selection
        // accumulates every line the agent emits while the mouse is down — a
        // 2.4 s drag selected 909,143 chars over 10,036 lines, and each
        // serialization of that selection costs 18-23 ms on the same thread as
        // the xterm write pump. That is the felt lag, and the runaway size is
        // also why the user gets far more text than they dragged over.
        // The RELEASE had the same defect from the other side: a click during a
        // stream armed the pin and the release event never came, which is the
        // filed "stuck viewport after a copy — '2 new messages ↓' that never
        // follows" symptom. Both directions now hang off the POINTER GESTURE,
        // which is a positive signal the workload cannot starve; the
        // selection-change path stays wired as a second caller of this same
        // owner, so the two can never disagree.
        const applySelectionScrollbackIntent = (selecting) => {{
            try {{
                if (selecting) {{
                    if (scrollbackIntent !== 'UserScrollback') {{
                        setScrollbackIntent('UserScrollback', 'selection_active');
                    }}
                    // Claim the pin ONLY when this selection is what set it. A
                    // user who scrolled up first keeps their own pin and its
                    // own reason, and the reached-bottom escape still applies
                    // to that one — which is correct, it is a wheel pin.
                    if (lastScrollbackIntentReason === 'selection_active') {{
                        selectionOwnsScrollbackPin = true;
                    }}
                }} else if (
                    releaseSelectionPinClaim()
                    && scrollbackIntent === 'UserScrollback'
                    && lastScrollbackIntentReason === 'selection_active'
                ) {{
                    // The release stays deliberately narrow — someone who
                    // scrolled UP to read and then selected keeps their place.
                    const buf = term && term.buffer ? term.buffer.active : null;
                    if (buf) {{
                        // The SAME effective-viewport reading the follow
                        // executor and the settle watchdog use — raw
                        // buf.viewportY can sit permanently below base under
                        // the visual-beyond-base clamp.
                        const vy = Math.max(0, Number(
                            (typeof effectiveXtermViewportY === 'function'
                                ? effectiveXtermViewportY(buf)
                                : buf.viewportY) || 0
                        ));
                        const by = Math.max(0, Number(buf.baseY || 0));
                        // 2 = scroll_mode::PIN_THRESHOLD_LINES; output jitter of
                        // a row or two is not "the user left".
                        if (by - vy < 2) {{
                            setScrollbackIntent(
                                'PromptFollow',
                                'selection_cleared_at_bottom'
                            );
                        }}
                    }}
                }}
            }} catch (_selectionIntentError) {{}}
        }};
        const terminalHasSelection = () => Boolean(
            term && typeof term.hasSelection === 'function' && term.hasSelection()
        );
        // A left-press starts a NEW selection: xterm clears the current one
        // before any trailing-edge sync could run. This capture-phase listener
        // runs BEFORE xterm's element handlers, so the COMPLETED selection is
        // recorded first — primary-selection semantics keep the last non-empty
        // selection, exactly as the per-event path did.
        const handlePrimarySelectionSyncPointerDown = (_event) => {{
            flushPrimarySelectionSync('selection_flush_pointer_down');
            // Open the drag window AFTER the flush: the flush must still record
            // the COMPLETED previous selection before xterm clears it.
            primarySelectionPointerDragActive = true;
            dragSelectionChangeCount = 0;
            dragBeganAtMs = Date.now();
            // ⭐ ARM THE PIN HERE, not on the first selection-change. This runs
            // in the capture phase, so the viewport has stopped following
            // before xterm has even begun the selection this press starts.
            applySelectionScrollbackIntent(true);
        }};
        // Drag end: make the FINAL selection durable NOW, not at the next
        // animation frame — a middle-click (possibly on another host) must
        // never read a stale or partial window.__yggtermPrimarySelection.
        const handlePrimarySelectionSyncPointerUp = (_event) => {{
            // Close the drag window FIRST so the flush below is allowed to do
            // the real work; then report the drag we just finished.
            const wasDragging = primarySelectionPointerDragActive;
            primarySelectionPointerDragActive = false;
            const flushBeganAtMs = Date.now();
            flushPrimarySelectionSync('selection_flush_pointer_up');
            // ⭐ AND RELEASE HERE. A press that selected nothing (a plain click)
            // must not leave the viewport pinned waiting for a selection-change
            // event that a streaming session never delivers.
            applySelectionScrollbackIntent(terminalHasSelection());
            if (wasDragging) {{
                // DRAG LIFECYCLE INSTRUMENT (2026-07-31). The terminal
                // selection path emitted ONE event per copy and nothing during
                // a drag, so a user-reported drag freeze was invisible to
                // telemetry — the render/forward probes are 60 s averages and
                // cannot see a sub-second stall. These four numbers make the
                // stall measurable: how many selection events the drag
                // generated (streaming sessions multiply them), how long the
                // one real serialization took, how big the selection was, and
                // how long the whole drag lasted.
                try {{
                    const entryForDrag = window.__yggtermXtermHosts
                        && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId] : null;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `drag_selection_complete host=${{hostId}}`
                            + ` selection_events=${{dragSelectionChangeCount}}`
                            + ` drag_ms=${{flushBeganAtMs - dragBeganAtMs}}`
                            + ` flush_ms=${{Date.now() - flushBeganAtMs}}`
                            + ` selected_chars=${{entryForDrag ? (entryForDrag.primarySelectionLength || 0) : 0}}`
                    }});
                }} catch (_error) {{}}
            }}
        }};
        // XTERM-BUG: clipboard-double-paste — telemetry to attribute the
        // bug class. Every entry into a paste path emits an
        // xterm_paste_event; if two events arrive within 300 ms with
        // different `source` or `triggered_by`, that's our signature of a
        // double-fire (selection + clipboard, or two clipboard pastes).
        let lastPasteEventAtMs = 0;
        let lastPasteEventSource = '';
        let lastPasteEventTrigger = '';
        const recordPasteEvent = (source, triggered_by, payload_length, extra) => {{
            try {{
                const now = Date.now();
                const dt = lastPasteEventAtMs ? now - lastPasteEventAtMs : -1;
                const extraJson = extra ? JSON.stringify(extra) : '';
                sendTerminalEvent({{
                    kind: 'debug',
                    message: `xterm_paste_event host=${{hostId}} source=${{source}} trigger=${{triggered_by}} length=${{payload_length}} dt_ms=${{dt}}${{extraJson ? ' extra=' + extraJson : ''}}`
                }});
                if (dt >= 0 && dt < 300 && (lastPasteEventSource !== source || lastPasteEventTrigger !== triggered_by)) {{
                    sendTerminalEvent({{
                        kind: 'debug',
                        message: `xterm_paste_double_fire host=${{hostId}} prev_source=${{lastPasteEventSource}} prev_trigger=${{lastPasteEventTrigger}} curr_source=${{source}} curr_trigger=${{triggered_by}} dt_ms=${{dt}} length=${{payload_length}}`
                    }});
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId] : null;
                    if (entry) {{
                        entry.pasteDoubleFireCount = Number(entry.pasteDoubleFireCount || 0) + 1;
                        entry.lastPasteDoubleFireAtMs = now;
                        entry.lastPasteDoubleFireSummary = `${{lastPasteEventSource}}:${{lastPasteEventTrigger}} -> ${{source}}:${{triggered_by}} dt=${{dt}}`;
                    }}
                }}
                lastPasteEventAtMs = now;
                lastPasteEventSource = source;
                lastPasteEventTrigger = triggered_by;
            }} catch (_error) {{}}
        }};
        const primarySelectionTextForPaste = () => {{
            try {{
                // CC-DRAG-STALL: a drag released OUTSIDE the owning host's
                // bounds never delivers that host's mouseup, so its deferred
                // selection sync can still be pending. Flush every host's
                // pending sync FIRST so window.__yggtermPrimarySelection is
                // final, THEN re-record from THIS host's live selection —
                // the live selection wins, exactly as it did when the
                // refresh ran last in the per-event model.
                try {{
                    const hostsForSelectionFlush = window.__yggtermXtermHosts || {{}};
                    for (const flushHostKey of Object.keys(hostsForSelectionFlush)) {{
                        const flushHostEntry = hostsForSelectionFlush[flushHostKey];
                        if (flushHostEntry
                            && typeof flushHostEntry.flushPrimarySelectionSync === 'function') {{
                            flushHostEntry.flushPrimarySelectionSync('middle_click_refresh');
                        }}
                    }}
                }} catch (_selectionFlushError) {{}}
                recordPrimarySelectionFromXterm('middle_click_refresh');
                const primary = window.__yggtermPrimarySelection || null;
                if (primary && typeof primary.text === 'string' && primary.text.length > 0) {{
                    return primary.text;
                }}
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                if (entry && typeof entry.primarySelectionText === 'string') {{
                    return entry.primarySelectionText;
                }}
            }} catch (_error) {{}}
            return '';
        }};
        const handlePrimarySelectionMiddleClick = (event) => {{
            try {{
                if (!event || Number(event.button || 0) !== 1) {{
                    return;
                }}
                if (event.preventDefault) {{
                    event.preventDefault();
                }}
                if (event.stopImmediatePropagation) {{
                    event.stopImmediatePropagation();
                }}
                if (event.stopPropagation) {{
                    event.stopPropagation();
                }}
                const now = Date.now();
                // A middle-click fires BOTH 'mousedown' and 'auxclick' (~200ms
                // apart — longer than any time-dedupe), so pasting on both
                // double-pasted (trace: two source=primary events per click).
                // Paste — and arm the native-echo suppressor — on the DOWN edge
                // ONLY; the release event (auxclick/click) just preventDefaults
                // the native paste and returns (no paste, no re-arm — re-arming
                // would swallow a later Ctrl+Shift+V).
                // See [[finding-terminal-selection-paste-bugs]].
                const isDownEdge = event.type === 'mousedown' || event.type === 'pointerdown';
                if (!isDownEdge) {{
                    return;
                }}
                if (now - lastPrimarySelectionMiddleClickAtMs < 120) {{
                    return;
                }}
                // Arm a ONE-SHOT suppressor for WebKit's native middle-click
                // clipboard 'paste' echo that fires ~200ms later (handleClipboard
                // Paste consumes exactly one paste event before this deadline).
                pendingMiddleClickEchoUntilMs = now + 600;
                lastPrimarySelectionMiddleClickAtMs = now;
                if (!inputEnabled || !hostOwnsActiveTerminalInput()) {{
                    return;
                }}
                const text = primarySelectionTextForPaste();
                if (!text) {{
                    return;
                }}
                markTerminalInputHot('primary_selection_paste');
                setScrollbackIntent('PromptFollow', 'primary_selection_paste');
                scrollbackLocked = false;
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    const entry = window.__yggtermXtermHosts[hostId];
                    entry.primarySelectionPasteCount = Number(entry.primarySelectionPasteCount || 0) + 1;
                    entry.lastPrimarySelectionPasteText = text.slice(0, 480);
                    entry.lastPrimarySelectionPasteLength = text.length;
                    entry.lastPrimarySelectionPasteAtMs = now;
                    entry.lastPrimarySelectionPasteMethod = 'pending';
                    entry.scrollbackLocked = false;
                }}
                // XTERM-BUG: clipboard-double-paste — record yggterm-side
                // primary paste. If xterm.js or WebKit also fires a paste
                // within 300 ms, the double-fire detector flags it.
                recordPasteEvent('primary', 'middle_click_yggterm', text.length, null);
                focusTerminal();
                try {{
                    if (term && typeof term.paste === 'function') {{
                        term.paste(text);
                        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                            window.__yggtermXtermHosts[hostId].lastPrimarySelectionPasteMethod = 'term.paste';
                        }}
                        return;
                    }}
                }} catch (_pasteError) {{}}
                try {{
                    const coreService = term && term._core
                        ? (term._core._coreService || term._core.coreService || null)
                        : null;
                    if (coreService && typeof coreService.triggerDataEvent === 'function') {{
                        coreService.triggerDataEvent(text, true);
                        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                            window.__yggtermXtermHosts[hostId].lastPrimarySelectionPasteMethod = 'core.triggerDataEvent';
                        }}
                        return;
                    }}
                }} catch (_triggerError) {{}}
                sendTerminalEvent({{ kind: "input", data: text }});
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].lastPrimarySelectionPasteMethod = 'direct_event';
                }}
            }} catch (_error) {{}}
        }};
        let lastTerminalContextMenuGestureAtMs = 0;
        const stopTerminalSecondaryEvent = (event) => {{
            try {{
                if (event && event.preventDefault) {{
                    event.preventDefault();
                }}
                if (event && event.stopImmediatePropagation) {{
                    event.stopImmediatePropagation();
                }}
                if (event && event.stopPropagation) {{
                    event.stopPropagation();
                }}
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        const openTerminalContextMenuFromEvent = (event, reason = 'contextmenu') => {{
            try {{
                stopTerminalSecondaryEvent(event);
                const rect = host.getBoundingClientRect();
                const clientX = Number.isFinite(Number(event && event.clientX))
                    ? Number(event.clientX)
                    : Number(rect.left + Math.min(Math.max(12, rect.width / 2), Math.max(12, rect.width - 12)));
                const clientY = Number.isFinite(Number(event && event.clientY))
                    ? Number(event.clientY)
                    : Number(rect.top + Math.min(Math.max(12, rect.height / 2), Math.max(12, rect.height - 12)));
                const now = Date.now();
                if (now - lastTerminalContextMenuGestureAtMs < 180) {{
                    return true;
                }}
                lastTerminalContextMenuGestureAtMs = now;
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    const entry = window.__yggtermXtermHosts[hostId];
                    entry.terminalContextMenuOpenCount = Number(entry.terminalContextMenuOpenCount || 0) + 1;
                    entry.lastTerminalContextMenuAtMs = Date.now();
                    entry.lastTerminalContextMenuX = clientX;
                    entry.lastTerminalContextMenuY = clientY;
                    entry.lastTerminalContextMenuReason = String(reason || 'contextmenu');
                    entry.terminalSecondaryButtonSuppressCount =
                        Number(entry.terminalSecondaryButtonSuppressCount || 0) + 1;
                }}
                focusTerminal();
                sendTerminalEvent({{ kind: "context_menu", client_x: clientX, client_y: clientY }});
                return true;
            }} catch (_error) {{}}
            return false;
        }};
        // A DOCUMENT (yedit) or WEB (ychrome) surface covering this terminal owns
        // the right-click: its content is WebKit-rendered DOM whose OWN context
        // menu offers Copy/Cut/Paste. This guard is why a right-click in yedit
        // reaches that native menu instead of the terminal's.
        //
        // ⚠ It must live HERE, at the funnel, not at the DOM entry points. The
        // thief was `handleDocumentPointerCapture` — a DOCUMENT-level capture
        // listener that routes a right-click to the terminal whenever it falls
        // GEOMETRICALLY inside the host rect (`pointerEventFallsWithinHost`).
        // A covering document surface occupies that exact rect, so every yedit
        // right-click looked like a terminal one, was preventDefault'ed, and
        // opened the terminal menu — the Rust-side viewport guard never saw the
        // event because the document surface is a SIBLING of the host, not a
        // child. Same shape as the fourth-focus-path bug: a host-wide handler
        // that never learned the document surface exists.
        //
        // The SHELL'S OWN FLOATING MENU is the same class of cover. While a
        // `ContextMenuOverlay` is up, its full-window backdrop is what the
        // pointer actually hits — but this capture listener is pure GEOMETRY,
        // so without the backdrop in this list it would still claim every
        // right-click inside the terminal's rect, `stopPropagation()` it during
        // the DOCUMENT CAPTURE phase (which ends the dispatch before the target
        // and bubble phases ever run) and open the terminal's own menu. The
        // backdrop's `oncontextmenu` dismissal and the menu surface's guard
        // would both be unreachable code over the largest region of the window
        // — the very defect class (`pointer-events:none` made a handler
        // unreachable) the dismissal owner was written to remove — and the user
        // would be left with TWO menus and two stacked backdrops on screen.
        //
        // AN AUTO-HIDDEN SIDEBAR OR RAIL is the same class again, and was the
        // one still missing: out of flow, its hover-revealed card floats INSIDE
        // the full-width host rect, so every right-click on a revealed row was
        // eaten here and no row menu could open. The whole list is
        // `TERMINAL_SECONDARY_COVER_SELECTORS` — ONE owner, so the next cover
        // is added in one place instead of to a literal buried in a JS string.
        const terminalSecondaryIsCoveredBySurface = (event) => {{
            try {{
                if (
                    String(host.getAttribute('data-document-surface-owns-viewport') || '') === 'true'
                    || String(host.getAttribute('data-web-surface-owns-viewport') || '') === 'true'
                ) {{
                    return true;
                }}
                const target = event && event.target;
                if (target && target.closest && target.closest({TERMINAL_SECONDARY_COVER_SELECTORS:?})) {{
                    return true;
                }}
            }} catch (_error) {{}}
            return false;
        }};
        handleTerminalSecondaryButton = (event) => {{
            try {{
                const eventType = String(event && event.type || '');
                if (eventType !== 'contextmenu' && Number(event && event.button) !== 2) {{
                    return false;
                }}
                // Stand down entirely: no preventDefault, no menu — the covering
                // surface's native menu comes through.
                if (terminalSecondaryIsCoveredBySurface(event)) {{
                    return false;
                }}
                return openTerminalContextMenuFromEvent(event, `secondary_${{eventType || 'event'}}`);
            }} catch (_error) {{
                return false;
            }}
        }};
        handleTerminalContextMenu = (event) => {{
            handleTerminalSecondaryButton(event);
        }};
        attachHostInteractions(host);
        let pendingClipboardPasteToken = 0;
        let lastNativeClipboardRequestAtMs = 0;
        const nativeClipboardRequestDedupeMs = 900;
        const terminalNativePasteDedupeMs = 2500;
        let lastClipboardPasteEventAtMs = 0;
        let lastClipboardPasteEventSignature = '';
        const clipboardPasteEventDedupeMs = 350;
        const pasteEventBelongsToTerminal = (event) => {{
            try {{
                if (!event || !inputEnabled || !hostOwnsActiveTerminalInput()) {{
                    return false;
                }}
                const target = event.target || null;
                const active = document.activeElement;
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                if (target && host.contains(target)) {{
                    return true;
                }}
                if (helperTextarea && active === helperTextarea) {{
                    return true;
                }}
                if (active === host || (term && term.textarea && active === term.textarea)) {{
                    return true;
                }}
                return false;
            }} catch (_error) {{
                return false;
            }}
        }};
        const terminalClipboardHostEntry = () => {{
            try {{
                return window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
            }} catch (_error) {{
                return null;
            }}
        }};
        const stopTerminalClipboardEvent = (event) => {{
            try {{
                if (event && event.preventDefault) {{
                    event.preventDefault();
                }}
                if (event && event.stopImmediatePropagation) {{
                    event.stopImmediatePropagation();
                }}
                if (event && event.stopPropagation) {{
                    event.stopPropagation();
                }}
            }} catch (_error) {{}}
        }};
        const clipboardPasteEventSummary = (event) => {{
            try {{
                const clipboardData = event && event.clipboardData ? event.clipboardData : null;
                const items = clipboardData && clipboardData.items
                    ? Array.from(clipboardData.items)
                    : [];
                const types = clipboardData && clipboardData.types
                    ? Array.from(clipboardData.types).map((type) => String(type || '').toLowerCase()).sort()
                    : items.map((item) => String(item && item.type ? item.type : '').toLowerCase()).sort();
                const hasImage = items.some((item) => {{
                    const itemType = String(item && item.type ? item.type : '').toLowerCase();
                    return itemType.startsWith('image/');
                }});
                let textLength = 0;
                if (!hasImage && clipboardData && typeof clipboardData.getData === 'function') {{
                    try {{
                        textLength = String(clipboardData.getData('text/plain') || '').length;
                    }} catch (_textError) {{
                        textLength = 0;
                    }}
                }}
                const signature = [
                    types.join(','),
                    String(items.length || 0),
                    hasImage ? 'image' : 'text',
                    String(textLength),
                ].join('|');
                return {{
                    signature,
                    hasImage,
                    textLength,
                    typeCount: types.length,
                }};
            }} catch (_error) {{
                return {{
                    signature: 'unknown',
                    hasImage: false,
                    textLength: 0,
                    typeCount: 0,
                }};
            }}
        }};
        const claimTerminalClipboardPasteEvent = (event) => {{
            stopTerminalClipboardEvent(event);
            const now = Date.now();
            window.__yggtermLastPasteEventAtMs = now;
            const summary = clipboardPasteEventSummary(event);
            const duplicate =
                summary.signature &&
                summary.signature === lastClipboardPasteEventSignature &&
                now - lastClipboardPasteEventAtMs < clipboardPasteEventDedupeMs;
            try {{
                if (event) {{
                    event.__yggtermHandledPaste = true;
                }}
            }} catch (_error) {{}}
            const entry = terminalClipboardHostEntry();
            if (entry) {{
                entry.clipboardPasteEventCount = Number(entry.clipboardPasteEventCount || 0) + 1;
                entry.lastClipboardPasteEventAtMs = now;
                entry.lastClipboardPasteEventTextLength = summary.textLength;
                entry.lastClipboardPasteEventTypeCount = summary.typeCount;
                entry.lastClipboardPasteEventHadImage = Boolean(summary.hasImage);
                entry.lastClipboardPasteEventDuplicate = Boolean(duplicate);
                if (duplicate) {{
                    entry.clipboardPasteDuplicateSuppressedCount =
                        Number(entry.clipboardPasteDuplicateSuppressedCount || 0) + 1;
                }}
            }}
            if (duplicate) {{
                return {{ claimed: false, hasImage: summary.hasImage }};
            }}
            lastClipboardPasteEventAtMs = now;
            lastClipboardPasteEventSignature = summary.signature;
            return {{ claimed: true, hasImage: summary.hasImage }};
        }};
        const requestNativeClipboardPaste = (reason = 'unknown') => {{
            try {{
                const now = Date.now();
                const entry = terminalClipboardHostEntry();
                if (now - lastNativeClipboardRequestAtMs < nativeClipboardRequestDedupeMs) {{
                    if (entry) {{
                        entry.nativeClipboardPasteRequestDedupedCount =
                            Number(entry.nativeClipboardPasteRequestDedupedCount || 0) + 1;
                        entry.lastNativeClipboardPasteRequestReason = String(reason || 'unknown');
                    }}
                    return false;
                }}
                lastNativeClipboardRequestAtMs = now;
                pendingClipboardPasteToken += 1;
                window.__yggtermLastPasteEventAtMs = now;
                if (entry) {{
                    entry.nativeClipboardPasteRequestCount =
                        Number(entry.nativeClipboardPasteRequestCount || 0) + 1;
                    entry.lastNativeClipboardPasteRequestAtMs = now;
                    entry.lastNativeClipboardPasteRequestReason = String(reason || 'unknown');
                }}
                sendTerminalEvent({{ kind: "clipboard_paste_request" }});
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        const requestNativeClipboardImagePaste = () => {{
            try {{
                const now = Date.now();
                const entry = terminalClipboardHostEntry();
                if (now - lastNativeClipboardRequestAtMs < nativeClipboardRequestDedupeMs) {{
                    if (entry) {{
                        entry.nativeClipboardPasteRequestDedupedCount =
                            Number(entry.nativeClipboardPasteRequestDedupedCount || 0) + 1;
                        entry.lastNativeClipboardPasteRequestReason = 'image_paste_event';
                    }}
                    return false;
                }}
                lastNativeClipboardRequestAtMs = now;
                pendingClipboardPasteToken += 1;
                window.__yggtermLastPasteEventAtMs = now;
                if (entry) {{
                    entry.nativeClipboardPasteRequestCount =
                        Number(entry.nativeClipboardPasteRequestCount || 0) + 1;
                    entry.lastNativeClipboardPasteRequestAtMs = now;
                    entry.lastNativeClipboardPasteRequestReason = 'image_paste_event';
                }}
                sendTerminalEvent({{ kind: "clipboard_image_request" }});
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        const handleClipboardPaste = (event) => {{
            if (!pasteEventBelongsToTerminal(event)) {{
                return;
            }}
            // XTERM-BUG: middle-click double paste. On Linux, WebKit's NATIVE
            // middle-click primary paste fires a 'paste' event in addition to
            // our explicit primary paste (handlePrimarySelectionMiddleClick);
            // handling it here would read the system CLIPBOARD and paste a
            // SECOND content (repro point 8). Consume exactly ONE such echo
            // after a middle-click (one-shot deadline), then clear it — so a
            // later Ctrl+Shift+V is NEVER swallowed (an earlier 700ms time-
            // window guard broke Ctrl+Shift+V during rapid testing).
            // See [[finding-terminal-selection-paste-bugs]].
            try {{
                if (pendingMiddleClickEchoUntilMs > 0 && Date.now() < pendingMiddleClickEchoUntilMs) {{
                    pendingMiddleClickEchoUntilMs = 0;
                    stopTerminalClipboardEvent(event);
                    const echoEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId] : null;
                    if (echoEntry) {{
                        echoEntry.middleClickClipboardEchoSuppressedCount =
                            Number(echoEntry.middleClickClipboardEchoSuppressedCount || 0) + 1;
                    }}
                    return;
                }}
            }} catch (_mcEchoError) {{}}
            try {{
                if (event.__yggtermHandledPaste) {{
                    stopTerminalClipboardEvent(event);
                    return;
                }}
                const pasteClaim = claimTerminalClipboardPasteEvent(event);
                if (!pasteClaim.claimed) {{
                    return;
                }}
                if (pasteClaim.hasImage) {{
                    requestNativeClipboardImagePaste();
                }} else {{
                    requestNativeClipboardPaste('paste_event');
                }}
                // A clipboard paste is input destined for the prompt — un-pin
                // and follow to the prompt (consistent with middle-click and the
                // snap-on-submit decision). Without this, Ctrl+Shift+V while the
                // viewport is pinned (e.g. after a selection) pasted but left the
                // viewport scrolled up (repro point 5). See
                // [[audit-viewport-scroll-control-flow]].
                if (!pasteClaim.hasImage) {{
                    try {{
                        markTerminalInputHot('clipboard_paste');
                        setScrollbackIntent('PromptFollow', 'clipboard_paste');
                        scrollbackLocked = false;
                        scrollLiveCursorIntoView(true, 'clipboard_paste');
                    }} catch (_clipFollowError) {{}}
                }}
                window.setTimeout(() => {{
                    if (inputEnabled) {{
                        focusTerminal();
                    }}
                }}, 0);
                window.setTimeout(() => {{
                    if (inputEnabled) {{
                        focusTerminal();
                    }}
                }}, 32);
                window.setTimeout(() => {{
                    if (inputEnabled) {{
                        focusTerminal();
                    }}
                }}, 120);
            }} catch (_error) {{}}
        }};
        window.addEventListener('paste', handleClipboardPaste, true);
        host.addEventListener('paste', handleClipboardPaste, true);
        document.addEventListener('paste', handleClipboardPaste, true);
        term.attachCustomKeyEventHandler((event) => {{
            if (!inputEnabled) {{
                if (event.preventDefault) {{
                    event.preventDefault();
                }}
                if (event.stopPropagation) {{
                    event.stopPropagation();
                }}
                return false;
            }}
            const rawKey = String(event.key || '');
            if (rawKey === 'PageUp' || rawKey === 'PageDown' || rawKey === 'Home' || rawKey === 'End') {{
                if (rawKey === 'PageUp' || rawKey === 'Home') {{
                    setScrollbackIntent('UserScrollback', `key_${{rawKey}}`);
                }} else {{
                    window.setTimeout(() => syncScrollbackLock(`key_${{rawKey}}`), 0);
                }}
            }}
            const accel = event.ctrlKey || event.metaKey;
            const key = (event.key || '').toLowerCase();
            if (!accel) {{
                return true;
            }}
            // Ctrl+V AND Ctrl+Shift+V both route through the explicit native
            // clipboard paste. Ctrl+Shift+V previously relied on WebKitGTK
            // firing a native 'paste' DOM event (caught by handleClipboardPaste)
            // because this handler returned true (passthrough) for it. Under
            // xterm.js 6 that native paste event no longer reaches us, so the
            // keyboard paste silently no-opped while right-click/context-menu
            // paste (a separate native path) kept working. Handle it here so the
            // shortcut is independent of the native paste event. requestNative-
            // ClipboardPaste de-dupes against window.__yggtermLastPasteEventAtMs,
            // so a native paste (if one still fires) will not double-paste.
            if (key === 'v' && !event.altKey) {{
                if (event.preventDefault) {{
                    event.preventDefault();
                }}
                if (event.stopImmediatePropagation) {{
                    event.stopImmediatePropagation();
                }}
                if (event.stopPropagation) {{
                    event.stopPropagation();
                }}
                const pasteShiftHeld = Boolean(event.shiftKey);
                const pasteToken = pendingClipboardPasteToken + 1;
                pendingClipboardPasteToken = pasteToken;
                window.setTimeout(() => {{
                    try {{
                        if (pasteToken !== pendingClipboardPasteToken) {{
                            return;
                        }}
                        const lastPasteEventAt = Number(window.__yggtermLastPasteEventAtMs || 0);
                        if (lastPasteEventAt > 0 && Date.now() - lastPasteEventAt < terminalNativePasteDedupeMs) {{
                            return;
                        }}
                        requestNativeClipboardPaste(pasteShiftHeld ? 'ctrl_shift_v_fallback' : 'ctrl_v_fallback');
                    }} catch (_error) {{}}
                }}, 220);
                window.setTimeout(() => {{
                    if (inputEnabled) {{
                        focusTerminal();
                    }}
                }}, 0);
                return false;
            }}
            if (!event.shiftKey || (key !== 'c' && key !== 'x')) {{
                return true;
            }}
            if (event.preventDefault) {{
                event.preventDefault();
            }}
            if (event.stopImmediatePropagation) {{
                event.stopImmediatePropagation();
            }}
            if (event.stopPropagation) {{
                event.stopPropagation();
            }}
            const selection = term.getSelection ? term.getSelection() : "";
            if (!selection) {{
                sendTerminalEvent({{
                    kind: "clipboard_error",
                    action: key === 'x' ? "cut" : "copy",
                    message: "Select terminal text before using the clipboard shortcut.",
                }});
                return false;
            }}
            const action = key === 'x' ? "cut" : "copy";
            try {{
                if (action === "cut" && term.clearSelection) {{
                    term.clearSelection();
                }}
                sendTerminalEvent({{ kind: "clipboard", action, chars: selection.length, text: selection }});
            }} catch (error) {{
                sendTerminalEvent({{
                    kind: "clipboard_error",
                    action,
                    message: error && error.message ? error.message : "Clipboard bridge failed.",
                }});
            }}
            return false;
        }});
        const emitResize = () => {{
            rebindCurrentHost('emit_resize', true);
            try {{
                const resizeShouldFollowPrompt = scrollbackIntent !== 'UserScrollback';
                // XTERM-BUG: content-scooped-on-session-switch
                // Capture buffer state before/after resize so we can detect
                // wrapped-line collapse (line count drop, baseY shift) when
                // the host width changes during session switch. See
                // docs/xterm-bugs.md#content-scooped-on-session-switch.
                const _bufferBefore = (term && term.buffer && term.buffer.active) ? term.buffer.active : null;
                const _bufferLengthBefore = _bufferBefore ? Number(_bufferBefore.length || 0) : -1;
                const _baseYBefore = _bufferBefore ? Number(_bufferBefore.baseY || 0) : -1;
                const _viewportYBefore = _bufferBefore ? Number(_bufferBefore.viewportY || 0) : -1;
                const _colsBefore = Number(term.cols || 0);
                const _rowsBefore = Number(term.rows || 0);
                const fitChanged = fitTerminalToHost('resize');
                const rowFitGuardApplied = applyTerminalRowFitGuard('resize');
                const resizeKey = `${{term.cols}}x${{term.rows}}`;
                const resizeChanged = resizeKey !== lastResizeKey;
                if (resizeKey !== lastResizeKey || rowFitGuardApplied) {{
                    lastResizeKey = resizeKey;
                    requestRenderProbe('resize');
                    scheduleSettledResizePaint();
                    if (resizeChanged) {{
                        const _bufferAfter = (term && term.buffer && term.buffer.active) ? term.buffer.active : null;
                        const _bufferLengthAfter = _bufferAfter ? Number(_bufferAfter.length || 0) : -1;
                        const _baseYAfter = _bufferAfter ? Number(_bufferAfter.baseY || 0) : -1;
                        const _viewportYAfter = _bufferAfter ? Number(_bufferAfter.viewportY || 0) : -1;
                        const _lineCountDelta = _bufferLengthBefore >= 0 && _bufferLengthAfter >= 0
                            ? _bufferLengthAfter - _bufferLengthBefore
                            : 0;
                        const _suspectScoop = Math.abs(_lineCountDelta) >= 4 && _colsBefore !== Number(term.cols || 0);
                        emitPerf("xterm_resize", {{
                            reason: "resize",
                            cols: term.cols,
                            rows: term.rows,
                            prev_cols: _colsBefore,
                            prev_rows: _rowsBefore,
                            buffer_length_before: _bufferLengthBefore,
                            buffer_length_after: _bufferLengthAfter,
                            buffer_length_delta: _lineCountDelta,
                            base_y_before: _baseYBefore,
                            base_y_after: _baseYAfter,
                            viewport_y_before: _viewportYBefore,
                            viewport_y_after: _viewportYAfter,
                            row_fit_guard_applied: Boolean(rowFitGuardApplied),
                            suspect_content_scoop: Boolean(_suspectScoop),
                        }});
                        if (_suspectScoop) {{
                            sendTerminalEvent({{
                                kind: "debug",
                                message: `xterm_content_scoop_suspect host=${{hostId}} cols=${{_colsBefore}}->${{term.cols}} rows=${{_rowsBefore}}->${{term.rows}} buffer_lines=${{_bufferLengthBefore}}->${{_bufferLengthAfter}} delta=${{_lineCountDelta}} baseY=${{_baseYBefore}}->${{_baseYAfter}}`
                            }});
                        }}
                        scheduleResizeNotification();
                    }}
                }}
                if (
                    resizeShouldFollowPrompt
                    && resizeChanged
                    && !fitChanged
                    && !rowFitGuardApplied
                ) {{
                    schedulePromptFollowAfterLayout('resize');
                }}
            }} catch (_error) {{}}
        }};
        const scheduleEmitResize = () => {{
            if (resizeFramePending) {{
                return;
            }}
            resizeFramePending = true;
            requestAnimationFrame(() => {{
                resizeFramePending = false;
                emitResize();
            }});
        }};
        window.__yggtermXtermHosts = window.__yggtermXtermHosts || {{}};
        window.__yggtermXtermHosts[hostId] = {{
            ownerToken: closureOwnerToken,
            // When this host's glyph atlas came into existence. A gap that ended
            // before this instant cannot have staled an atlas that did not yet
            // exist — without this, every fresh mount healed itself against an
            // inherited gap forever. See the stale-atlas block in `onRender`.
            mountedAtMs: Date.now(),
            host,
            term,
            fitAddon,
            redrawTerminal,
            emitResize,
            setInputEnabled,
            scrollLiveCursorIntoView,
            forcePromptFollow: (reason = 'prompt_follow') => scrollLiveCursorIntoView(true, reason),
            forceXtermViewportY,
            // Exposed so the out-of-closure app-control scroll eval can set the
            // intent SSOT (closure `scrollbackIntent`), not just `entry.scrollbackIntent`
            // — otherwise the settle-follow watchdog (which reads the closure var)
            // re-asserts PromptFollow and yanks an app-control scroll-up back to bottom.
            setScrollbackIntent,
            focusTerminal,
            refreshCursorContrastContract,
            inputEnabled,
            rustInputGateOpen,
            hostId,
                    sessionPath: host.getAttribute("data-terminal-session-path") || "",
                    sessionKind: host.getAttribute("data-terminal-session-kind") || "",
            // CC-DRAG-STALL: cross-host flush hook — primarySelectionTextForPaste
            // flushes every host's pending deferred selection sync through this
            // before reading window.__yggtermPrimarySelection.
            flushPrimarySelectionSync,
            primarySelectionText: '',
            primarySelectionLength: 0,
            primarySelectionUpdatedAtMs: 0,
            lastPrimarySelectionReason: '',
            primarySelectionPasteCount: 0,
            lastPrimarySelectionPasteText: '',
            lastPrimarySelectionPasteLength: 0,
            lastPrimarySelectionPasteAtMs: 0,
            lastPrimarySelectionPasteMethod: '',
            clipboardPasteEventCount: 0,
            clipboardPasteDuplicateSuppressedCount: 0,
            nativeClipboardPasteRequestCount: 0,
            nativeClipboardPasteRequestDedupedCount: 0,
            lastNativeClipboardPasteRequestAtMs: 0,
            lastNativeClipboardPasteRequestReason: '',
            lastClipboardPasteEventAtMs: 0,
            lastClipboardPasteEventTextLength: 0,
            lastClipboardPasteEventTypeCount: 0,
            lastClipboardPasteEventHadImage: false,
            lastClipboardPasteEventDuplicate: false,
            terminalContextMenuOpenCount: 0,
            lastTerminalContextMenuAtMs: 0,
            lastTerminalContextMenuX: null,
            lastTerminalContextMenuY: null,
            lastTerminalContextMenuReason: '',
            terminalSecondaryButtonSuppressCount: 0,
            promptFollowLayoutGuardUntilMs: 0,
            lastPromptFollowLayoutGuardReason: '',
            promptFollowSchedulePending,
            promptFollowScheduleReason,
            promptFollowScheduleAtMs,
            promptFollowScheduleSkipCount,
            lastPromptFollowScheduleSkipReason,
            terminalContentSource: 'empty',
            terminalSourceMismatchReason: '',
            mountedAt: Date.now(),
            wheelEventCount,
            scrollEventCount,
            dataEventCount,
            readNudgeCount,
            renderEventCount,
            bufferTransitionCount,
            cursorHiddenToggleCount,
            cursorCellBackground: 'transparent',
            cursorCellBackgroundSource: 'initial',
            lastCursorCellBackgroundRefreshReason: '',
            lastCursorCellBackgroundRefreshAtMs: 0,
            lastObservedBufferKind,
            lastObservedCursorHidden,
            lastVisualTransitionReason,
            softwareCanvasLayerOptimizationActive,
            softwareCanvasHiddenLayerCount,
            softwareCanvasVisibleLayerCount,
            softwareCanvasCursorOverlayVisible,
            lastSoftwareCanvasLayerOptimizationReason,
            activationRepaintCount: 0,
            lastActivationRepaintAtMs: 0,
            lastActivationRepaintReason: '',
            lastActivationRepaintKey: '',
            xtermInputLineDecorationPresent: Boolean(xtermInputLineDecoration),
            xtermInputLineDecorationVisible,
            xtermInputLineDecorationLine,
            xtermInputLineDecorationWidth,
            xtermInputLineDecorationBackground,
            xtermInputLineDecorationError,
            xtermInputLineDecorationDisposed: false,
            xtermInputLineDecorationMarkerLine: null,
            xtermInputLineDecorationElementPresent: false,
            xtermInputLineDecorationElementVisible: false,
            xtermInputLineDecorationElementBackground: '',
            xtermInputLineDecorationElementDisplay: '',
            xtermInputLineDecorationElementRect: null,
            xtermInputLineDecorationRenderCount,
            inputPolicyApplyCount,
            inputPolicyNoopCount,
            inputPolicyNoopPromptFollowCount,
            lastInputPolicyNoopPromptFollowAtMs,
            lastInputPolicyNoopPromptFollowReason,
            lastInputPolicyReason,
            retainedReplayPromotedToDaemonPtyCount,
            lastRetainedReplayPromotedAtMs,
            lastRetainedReplayPromotedFrom,
            lastRetainedReplayPromotedReason,
            writeCommandCount: 0,
            writeCallbackCount: 0,
            writeParsedCount: 0,
            protocolDataEventCount: 0,
            suppressedTerminalProtocolResponseCount,
            lastSuppressedTerminalProtocolResponse,
            lastSuppressedTerminalProtocolResponseAtMs,
            ignoredDataEventCount: 0,
            lastWriteParsedAtMs: 0,
            lastDataEventAtMs: 0,
            lastReadNudgeAtMs: 0,
            lastReadNudgeReason: '',
            lastRenderEventAtMs: 0,
            writeBridgeFlushCount: 0,
            writeBridgeInFlight: false,
            writeBridgePendingData: '',
            lastWriteSample: '',
            lastWriteAppliedTail: '',
            lastWriteError: '',
            retainedWritePaintRepairCount,
            lastRetainedWritePaintRepairReason: '',
            lastFitGuard: null,
            lastSkippedFit: null,
            lastWriteQueuedAtMs: 0,
            lastWriteFlushStartedAtMs: 0,
            lastWriteCallbackAtMs: 0,
            terminalWriteFrameMs,
            terminalActiveWriteFrameMs,
            terminalActiveAnimationWriteFrameMs,
            terminalActiveAnimationSustainedWriteFrameMs,
            effectiveTerminalWriteFrameMs: terminalWriteFrameMs,
            activeWriteFrameBudget: false,
            recentFrameLikeWriteUntilMs,
            recentInlineStatusAnimationUntilMs,
            recentInlineStatusAnimationStartedAtMs,
            recentInlineStatusAnimationHot: false,
            programmaticFocusEnabled,
            skippedPerfEventCount,
            lastSkippedPerfEventName: '',
            hotHostHealthSuppressedCount,
            terminalInputHotUntilMs,
            forcedRefreshCount,
            forcedRefreshSkippedCount,
            manualRedrawCount,
            lastManualRedrawReason: '',
            lastManualRedrawAtMs: 0,
            lastManualRedrawStartedAtMs: 0,
            lastManualRedrawSettledAtMs: 0,
            lastManualRedrawDurationMs: null,
            lastManualRedrawRenderEventCountBefore: 0,
            lastManualRedrawRenderEventCountAfter: 0,
            lastManualRedrawInkBefore: null,
            lastManualRedrawInkAfter: null,
            lastManualRedrawEffect: '',
            renderHealthStatus,
            renderHealthReason,
            renderHealthRecoveryCount,
            lastRenderHealthRecoveryAtMs,
            lastRenderHealthCheckedAtMs,
            pendingRenderHealthRecovery: false,
            renderHealthInkSample: null,
            hostAttachmentState: null,
            termElementConnected: null,
            hostContainsTermElement: null,
            termElementDetachedSinceMs: 0,
            termElementDetachedCount: 0,
            lastVisiblePaintTermElementAttached: null,
            lastVisiblePaintWasHusk: false,
            lastHostMutation: null,
            hostMutationCount: 0,
            lowPowerTuiActive: false,
            lowPowerTuiFrameCount: 0,
            lastLowPowerTuiText: '',
            backgroundTuiSuppressActive: false,
            inactiveTuiFrameDropCount: 0,
            inactiveTuiLastTail: '',
            unfocusedTuiFrameDropCount: 0,
            unfocusedTuiLastTail: '',
            pendingVisiblePaintRecovery: false,
            pendingVisiblePaintRecoveryUntilMs: 0,
            lastWheelDeltaY: 0,
            lastRawPayloadLength: 0,
            lastRawPayloadLineCount: 0,
            lastRawPayloadSample: '',
            transportLeakDroppedWriteCount: 0,
            transportLeakResetCount: 0,
            lastTransportLeakResetAtMs: 0,
            lastTransportLeakResetPayloadLength: 0,
            visibleProtocolLeakSanitizedCount: 0,
            lastVisibleProtocolLeakSanitizedAtMs: 0,
            lastVisibleProtocolLeakSanitizedPayloadLength: 0,
            lastRetainedReplayLineCount: 0,
            lastRetainedReplayExpected: false,
            lastRetainedReplaySource: '',
            lastRetainedReplayPromptFollowReady: false,
            retainedReplayUnsafeSkipPromptReady: false,
            lastRetainedReplayRejectedVisibleText: '',
            lastRetainedReplayFollowDebug: null,
            lastRetainedReplayPaintRefreshDebug: null,
            lastRetainedReplayRecoveredFromSnapshot: false,
            lastRetainedReplaySnapshotAgeMs: null,
            lastRetainedReplaySnapshotError: '',
            lastXtermSessionSnapshotReason: '',
            lastXtermSessionSnapshotAtMs: 0,
            lastXtermSessionSnapshotLineCount: 0,
            lastXtermSessionSnapshotNonblankLineCount: 0,
            lastXtermSessionSnapshotBaseY: 0,
            lastXtermSessionSnapshotViewportY: 0,
            scrollbackExpected: false,
            scrollbackLocked,
            scrollbackIntent,
            lastScrollbackIntentReason,
            lastScrollbackIntentAtMs,
            lastScrollbackSnapbackReason,
            scrollControllerVisible: false,
            scrollControllerDistanceRows: 0,
            scrollControllerReason: '',
            scrollControllerUpdatedAtMs: 0,
            lastViewportForceDebug: null,
            lastViewportForceReason: '',
            lastViewportForceAtMs: 0,
        }};
        // Registration is the ownership claim: from here on, a NEWER closure
        // overwriting this entry (its ownerToken replaces ours) supersedes us
        // and every gated site above stands us down.
        closureOwnRegistered = true;
        // FIX A (content-scoop on remount): fit the term to its container
        // BEFORE the snapshot restore writes content. A fresh/remounted xterm
        // is 80x24 until the first fit; restoring the retained snapshot (the
        // "shadow") at 80x24 and only THEN fitting up to the real grid makes
        // xterm.js grow-reflow drop trailing buffer lines + collapse scrollback
        // (the `xterm_content_scoop_suspect` event: buffer_lines 74->63 delta
        // -11, baseY 50->0). Sizing first lands the restore at the correct
        // geometry so the subsequent layout fit is a no-op instead of a scoop.
        // Best-effort: if the host has no layout yet the fit is a guarded no-op
        // and the later resize behaves exactly as before (no regression).
        try {{ fitTerminalToHost('pre_restore'); }} catch (_preRestoreFitErr) {{}}
        restoreXtermSessionSnapshotOnConstructed();
        syncTerminalScrollController('constructed');
        scheduleCursorCellBackgroundRefresh('constructed');
        // XTERM-BUG: dom-leak-on-session-start — capture host innerText at 0ms,
        // 16ms, 64ms after mount so we can detect content from a prior session
        // bleeding through during the swap. Each sample is the first 240 chars
        // of host.innerText. Emitted as `xterm_first_paint_sample`.
        const _captureFirstPaintSample = (label, delayMs) => {{
            const fire = () => {{
                try {{
                    const sessionPath = host.getAttribute("data-terminal-session-path") || "";
                    const text = String(host.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 240);
                    sendTerminalEvent({{
                        kind: 'debug',
                        message: `xterm_first_paint_sample host=${{hostId}} session=${{sessionPath}} label=${{label}} delay_ms=${{delayMs}} len=${{text.length}} text=${{JSON.stringify(text)}}`
                    }});
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                        ? window.__yggtermXtermHosts[hostId] : null;
                    if (entry) {{
                        entry[`firstPaintSample_${{label}}`] = text;
                        entry[`firstPaintSampleAtMs_${{label}}`] = Date.now();
                    }}
                }} catch (_e) {{}}
            }};
            if (delayMs <= 0) {{ fire(); }} else {{ window.setTimeout(fire, delayMs); }}
        }};
        _captureFirstPaintSample('t0', 0);
        _captureFirstPaintSample('t16', 16);
        _captureFirstPaintSample('t64', 64);
        _captureFirstPaintSample('t256', 256);
        const handleExternalReadNudge = (event) => {{
            try {{
                const detail = event && event.detail ? event.detail : {{}};
                const expectedSessionPath = host.getAttribute("data-terminal-session-path") || "";
                const requestedSessionPath = String(detail.sessionPath || "");
                if (requestedSessionPath && requestedSessionPath !== expectedSessionPath) {{
                    return;
                }}
                readNudgeCount += 1;
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].readNudgeCount = readNudgeCount;
                    window.__yggtermXtermHosts[hostId].lastReadNudgeReason = String(detail.reason || 'external_input');
                    window.__yggtermXtermHosts[hostId].lastReadNudgeAtMs = Date.now();
                }}
                markTerminalInputHot('external_input');
                setScrollbackIntent('PromptFollow', 'external_input');
                scrollbackLocked = false;
                scrollLiveCursorIntoView(true, 'external_input');
                sendTerminalEvent({{
                    kind: "read_nudge",
                    reason: String(detail.reason || 'external_input'),
                }});
            }} catch (_error) {{}}
        }};
        window.addEventListener('yggterm-terminal-read-nudge', handleExternalReadNudge, false);
        sendTerminalEvent({{
            kind: "debug",
            message: `constructed host=${{hostId}} cols=${{term.cols}} rows=${{term.rows}}`
        }});
        trackTerminalVisualState('constructed');
        setInputEnabled(inputEnabled, programmaticFocusEnabled);
        const scheduleResizeNudges = () => {{
            [140, 420, 900, 1600].forEach((delayMs) => {{
                window.setTimeout(() => {{
                    emitResize();
                    requestVisiblePaint(delayMs <= 140);
                    if (inputEnabled && programmaticFocusEnabled) {{
                        focusTerminal();
                    }}
                    if (delayMs >= 420) {{
                        ensureVisibleHost(`resize_nudge_${{delayMs}}`);
                    }}
                }}, delayMs);
            }});
        }};
        resizeObserver = new ResizeObserver(() => {{
            // The ghost's observer re-fires on the live owner's every DOM
            // change — this gate is what actually ends the A-B eviction fight.
            if (standDownIfSuperseded('resize_observer')) {{
                return;
            }}
            scheduleEmitResize();
        }});
        resizeObserver.observe(host);
        const readTerminalBufferSample = () => {{
            try {{
                rebindCurrentHost('read_buffer_sample', false);
                if (!term || !term.buffer || !term.buffer.active) {{
                    return String(host.innerText || '').slice(0, 4096);
                }}
                const active = term.buffer.active;
                const rows = Math.max(1, Number(term.rows || 18));
                const viewportY = Math.max(0, Number(active.viewportY || 0));
                const collect = (start, end) => {{
                    const lines = [];
                    for (let index = start; index < end; index += 1) {{
                        const line = active.getLine ? active.getLine(index) : null;
                        if (!line || !line.translateToString) {{
                            continue;
                        }}
                        lines.push(String(line.translateToString(true) || ''));
                    }}
                    return lines.join('\n').trim();
                }};
                let sample = collect(viewportY, Math.min(active.length, viewportY + rows));
                if (!sample) {{
                    sample = collect(Math.max(0, active.length - Math.max(rows, 18)), active.length);
                }}
                if (!sample) {{
                    sample = collect(0, Math.min(active.length, Math.max(rows, 18)));
                }}
                return sample.slice(0, 4096);
            }} catch (_error) {{
                return String(host.innerText || '').slice(0, 4096);
            }}
        }};
        const normalizedTerminalTransportLine = (line) => String(line || "")
            .trim()
            .replace(/^[›>\s]+/, "")
            .trim()
            .toLowerCase();
        const internalTransportErrorIndex = (line) => {{
            const lower = String(line || "").toLowerCase();
            const markers = [
                "error: terminal session not found: local://",
                "terminal session not found: local://",
                "error: terminal session not found: remote-session://",
                "terminal session not found: remote-session://",
                "error: terminal session not found: codex-runtime://",
                "terminal session not found: codex-runtime://"
            ];
            let best = -1;
            for (const marker of markers) {{
                const ix = lower.indexOf(marker);
                if (ix >= 0 && (best < 0 || ix < best)) {{
                    best = ix;
                }}
            }}
            return best;
        }};
        const terminalLineIsInternalTransportError = (line) => {{
            const normalized = normalizedTerminalTransportLine(line);
            return normalized.startsWith("error: terminal session not found: local://")
                || normalized.startsWith("terminal session not found: local://")
                || normalized.startsWith("error: terminal session not found: remote-session://")
                || normalized.startsWith("terminal session not found: remote-session://")
                || normalized.startsWith("error: terminal session not found: codex-runtime://")
                || normalized.startsWith("terminal session not found: codex-runtime://")
                || normalized.includes("error: terminal session not found: local://")
                || normalized.includes("error: terminal session not found: remote-session://")
                || normalized.includes("error: terminal session not found: codex-runtime://");
        }};
        const terminalLineIsSharedConnectionClose = (line) => {{
            const normalized = normalizedTerminalTransportLine(line);
            return normalized.startsWith("shared connection to ")
                && (normalized.includes(" closed")
                    || normalized.includes(" refused")
                    || normalized.includes(" timed out"));
        }};
        const terminalLineIsSharedConnectionNotice = (line) => {{
            const normalized = normalizedTerminalTransportLine(line);
            return normalized.startsWith("shared connection to ");
        }};
        const terminalLineIsPromptLike = (line) => {{
            const stripped = stripAnsiForLowPowerTui(line);
            const trimmed = String(stripped || "").trim();
            return trimmed.startsWith("›") || trimmed.startsWith(">");
        }};
        const terminalLinePrefixHasMeaningfulReplayContent = (line) => {{
            const stripped = stripAnsiForLowPowerTui(line);
            const trimmed = String(stripped || "").trim();
            return Boolean(trimmed && trimmed !== "›" && trimmed !== ">");
        }};
        const terminalTextHasInternalTransportLeak = (text) => {{
            const value = String(text || "");
            const lower = value.toLowerCase();
            if (!lower.includes("terminal session not found") && !lower.includes("shared connection to ")) {{
                return false;
            }}
            return value
                .replace(/\r\n/g, "\n")
                .replace(/\r/g, "\n")
                .split("\n")
                .some((line) => terminalLineIsInternalTransportError(line) || terminalLineIsSharedConnectionNotice(line));
        }};
        const visibleProtocolResponseLeakIndex = (line) => {{
            const value = String(line || "");
            const paletteIndex = value.search(/4;\d+;rgb:[0-9a-fA-F]{{2,4}}\/[0-9a-fA-F]{{2,4}}\/[0-9a-fA-F]{{2,4}}/);
            if (paletteIndex >= 0) {{
                return paletteIndex;
            }}
            return value.search(/(?:10|11);rgb:[0-9a-fA-F]{{2,4}}\/[0-9a-fA-F]{{2,4}}\/[0-9a-fA-F]{{2,4}}/);
        }};
        const terminalTextHasVisibleProtocolResponseLeak = (text) => {{
            const value = String(text || "");
            if (!value.includes("rgb:")) {{
                return false;
            }}
            return value
                .replace(/\r\n/g, "\n")
                .replace(/\r/g, "\n")
                .split("\n")
                .some((line) => visibleProtocolResponseLeakIndex(stripAnsiForLowPowerTui(line)) >= 0);
        }};
        const sanitizeVisibleProtocolResponseLeakPayload = (payload) => {{
            const value = String(payload || "");
            if (!terminalTextHasVisibleProtocolResponseLeak(value)) {{
                return value;
            }}
            const normalized = value.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
            const kept = [];
            let stripped = false;
            let dropContinuationLines = 0;
            for (const line of normalized.split("\n")) {{
                const strippedLine = stripAnsiForLowPowerTui(line);
                const leakIndex = visibleProtocolResponseLeakIndex(strippedLine);
                if (leakIndex >= 0) {{
                    stripped = true;
                    dropContinuationLines = 3;
                    const rawIndex = visibleProtocolResponseLeakIndex(line);
                    const prefix = rawIndex >= 0 ? line.slice(0, rawIndex) : "";
                    if (terminalLinePrefixHasMeaningfulReplayContent(prefix)) {{
                        kept.push(prefix);
                    }}
                    continue;
                }}
                if (dropContinuationLines > 0) {{
                    if (terminalLineIsPromptLike(line) || /[$#>]\s*$/.test(strippedLine.trim())) {{
                        dropContinuationLines = 0;
                        kept.push(line);
                    }} else {{
                        stripped = true;
                        dropContinuationLines -= 1;
                    }}
                    continue;
                }}
                kept.push(line);
            }}
            if (!stripped) {{
                return value;
            }}
            const currentEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
            if (currentEntry) {{
                currentEntry.visibleProtocolLeakSanitizedCount =
                    Number(currentEntry.visibleProtocolLeakSanitizedCount || 0) + 1;
                currentEntry.lastVisibleProtocolLeakSanitizedAtMs = Date.now();
                currentEntry.lastVisibleProtocolLeakSanitizedPayloadLength = value.length;
            }}
            sendTerminalEvent({{
                kind: "debug",
                message: `visible_protocol_leak_sanitized host=${{hostId}} chars=${{value.length}}`
            }});
            return kept.join("\n");
        }};
        const sanitizeInternalTransportPayload = (payload) => {{
            const value = String(payload || "");
            if (!value.toLowerCase().includes("terminal session not found")) {{
                return value;
            }}
            const normalized = value.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
            const kept = [];
            let stripped = false;
            let dropFollowingTransportTailLines = 0;
            let sawInternalTransportError = false;
            for (const line of normalized.split("\n")) {{
                const errorIndex = internalTransportErrorIndex(line);
                if (errorIndex >= 0) {{
                    stripped = true;
                    sawInternalTransportError = true;
                    dropFollowingTransportTailLines = 3;
                    const prefix = line.slice(0, errorIndex);
                    if (terminalLinePrefixHasMeaningfulReplayContent(prefix)) {{
                        kept.push(prefix);
                    }}
                    continue;
                }}
                if (terminalLineIsInternalTransportError(line)) {{
                    stripped = true;
                    sawInternalTransportError = true;
                    dropFollowingTransportTailLines = 3;
                    continue;
                }}
                if (sawInternalTransportError && terminalLineIsSharedConnectionNotice(line)) {{
                    stripped = true;
                    dropFollowingTransportTailLines = 2;
                    continue;
                }}
                if (dropFollowingTransportTailLines > 0) {{
                    if (terminalLineIsSharedConnectionNotice(line)) {{
                        stripped = true;
                        dropFollowingTransportTailLines = 2;
                        continue;
                    }}
                    if (terminalLineIsPromptLike(line)) {{
                        dropFollowingTransportTailLines = 0;
                    }} else {{
                        stripped = true;
                        dropFollowingTransportTailLines -= 1;
                        continue;
                    }}
                }}
                kept.push(line);
            }}
            return stripped ? kept.join("\n") : value;
        }};
        const resetVisibleTransportLeakBeforeWrite = (payload) => {{
            try {{
                const value = String(payload || "");
                if (!stripAnsiForLowPowerTui(value).trim()) {{
                    return false;
                }}
                if (terminalTextHasInternalTransportLeak(value)) {{
                    return false;
                }}
                if (!terminalTextHasInternalTransportLeak(readTerminalBufferSample())) {{
                    return false;
                }}
                traceXtermScreenEvent("reset", {{ reason: "transport_leak_scrub" }});
                if (term && typeof term.reset === "function") {{
                    term.reset();
                }}
                if (term && typeof term.clear === "function") {{
                    term.clear();
                }}
                if (!term || (typeof term.reset !== "function" && typeof term.clear !== "function")) {{
                    return false;
                }}
                const currentEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (currentEntry) {{
                    currentEntry.transportLeakResetCount =
                        Number(currentEntry.transportLeakResetCount || 0) + 1;
                    currentEntry.lastTransportLeakResetAtMs = Date.now();
                    currentEntry.lastTransportLeakResetPayloadLength = value.length;
                }}
                sendTerminalEvent({{
                    kind: "debug",
                    message: `transport_leak_visible_buffer_reset host=${{hostId}} chars=${{value.length}}`
                }});
                return true;
            }} catch (_error) {{
                return false;
            }}
        }};
        const syncLowPowerTuiHostEntry = () => {{
            try {{
                const currentEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (!currentEntry) {{
                    return;
                }}
                currentEntry.lowPowerTuiActive = Boolean(lowPowerTuiActive);
                currentEntry.lowPowerTuiFrameCount = Number(lowPowerTuiFrameCount || 0);
                currentEntry.lastLowPowerTuiText = String(lowPowerTuiLastText || '').slice(-4096);
                currentEntry.backgroundTuiSuppressActive = Boolean(backgroundTuiSuppressActive);
                currentEntry.inactiveTuiFrameDropCount = Number(inactiveTuiFrameDropCount || 0);
                currentEntry.inactiveTuiLastTail = String(inactiveTuiLastTail || '').slice(-240);
                currentEntry.unfocusedTuiFrameDropCount = Number(unfocusedTuiFrameDropCount || 0);
                currentEntry.unfocusedTuiLastTail = String(unfocusedTuiLastTail || '').slice(-240);
            }} catch (_error) {{}}
        }};
        const hideLowPowerTuiOverlay = () => {{
            try {{
                if (lowPowerTuiOverlay) {{
                    lowPowerTuiOverlay.style.display = 'none';
                    lowPowerTuiOverlay.textContent = '';
                    if (typeof lowPowerTuiOverlay.remove === 'function') {{
                        lowPowerTuiOverlay.remove();
                    }}
                    lowPowerTuiOverlay = null;
                }}
            }} catch (_error) {{}}
            lowPowerTuiActive = false;
            lowPowerTuiLastText = '';
            lowPowerTuiTextBuffer = '';
            backgroundTuiSuppressActive = false;
            syncLowPowerTuiHostEntry();
        }};
        const ensureLowPowerTuiOverlay = () => {{
            try {{
                if (lowPowerTuiOverlay && lowPowerTuiOverlay.isConnected) {{
                    return lowPowerTuiOverlay;
                }}
                const overlay = document.createElement('pre');
                overlay.setAttribute('data-yggterm-low-power-tui', '1');
                overlay.setAttribute('aria-hidden', 'true');
                overlay.style.position = 'absolute';
                overlay.style.inset = '0';
                overlay.style.zIndex = '2';
                overlay.style.margin = '0';
                overlay.style.padding = '0';
                overlay.style.border = '0';
                overlay.style.overflow = 'hidden';
                overlay.style.pointerEvents = 'none';
                overlay.style.userSelect = 'none';
                overlay.style.webkitUserSelect = 'none';
                overlay.style.whiteSpace = 'pre';
                overlay.style.tabSize = '4';
                overlay.style.fontFamily = {font_family};
                overlay.style.fontWeight = String({font_weight});
                overlay.style.fontFeatureSettings = '"calt" 0, "liga" 0';
                overlay.style.fontVariantLigatures = 'none';
                overlay.style.letterSpacing = '0px';
                overlay.style.webkitFontSmoothing = {font_smoothing};
                overlay.style.MozOsxFontSmoothing = {moz_font_smoothing};
                overlay.style.color = {foreground};
                overlay.style.background = {background};
                overlay.style.fontSize = `${{Number(term && term.options ? term.options.fontSize || {font_size} : {font_size})}}px`;
                overlay.style.lineHeight = `${{Number(term && term.options ? term.options.fontSize || {font_size} : {font_size}) * Number(term && term.options ? term.options.lineHeight || {line_height} : {line_height})}}px`;
                host.appendChild(overlay);
                lowPowerTuiOverlay = overlay;
                return overlay;
            }} catch (_error) {{
                return null;
            }}
        }};
        const terminalPayloadLooksHighVolumeFrame = (payload) => {{
            const value = String(payload || '');
            if (
                value.includes('\x1b[?1049h')
                || (
                    value.includes('\x1b[?2026h')
                    && value.includes('\x1b[?2026l')
                    && (value.includes('\x1b[K') || value.includes('\x1b[2K'))
                )
                || value.includes('\x1b[2J')
            ) {{
                return true;
            }}
            if (terminalPayloadLooksInlineStatusRewrite(value)) {{
                return false;
            }}
            if (value.length < 256) {{
                return false;
            }}
            const anchors = ['\x1b[H', '\x1b[1;1H', '\x1b[1;1f', '\x1b[;H', '\x1b[0;0H'];
            for (const anchor of anchors) {{
                if (value.indexOf(anchor) >= 0) {{
                    return true;
                }}
            }}
            let csiCount = 0;
            let index = value.indexOf('\x1b[');
            while (index >= 0 && csiCount < 24) {{
                csiCount += 1;
                index = value.indexOf('\x1b[', index + 2);
            }}
            return csiCount >= 24;
        }};
        const terminalPayloadLooksSynchronizedRepaintFrame = (payload) => {{
            const value = String(payload || '');
            if (!value.includes('\x1b[?2026h') || !value.includes('\x1b[?2026l')) {{
                return false;
            }}
            if (/\x1b\[[0-9]+;[0-9]+[Hf]/.test(value)) {{
                return true;
            }}
            let csiCount = 0;
            let index = value.indexOf('\x1b[');
            while (index >= 0 && csiCount < 8) {{
                csiCount += 1;
                index = value.indexOf('\x1b[', index + 2);
            }}
            return csiCount >= 8;
        }};
        const coalesceSynchronizedOutputFrames = (payload) => {{
            const value = String(payload || '');
            return value;
        }};
        const terminalPayloadLooksInlineStatusRewrite = (payload) => {{
            const value = String(payload || '');
            if (
                !value
                || value.length > 8192
                || value.includes('\x1b[?1049h')
                || value.includes('\x1b[?1049l')
                || value.includes('\x1b[2J')
            ) {{
                return false;
            }}
            if (terminalPayloadLooksSynchronizedRepaintFrame(value)) {{
                return false;
            }}
            const inlineRewriteControl =
                value.includes('\r')
                || value.includes('\x08')
                || value.includes('\x1b[K')
                || value.includes('\x1b[2K')
                || value.includes('\x1b[G')
                || value.includes('\x1b[1G')
                || value.includes('\x1b[?25l');
            if (!inlineRewriteControl) {{
                return false;
            }}
            const visible = stripAnsiForLowPowerTui(value)
                .replace(/[\r\n\t]+/g, ' ')
                .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')
                .slice(0, 256);
            return Boolean(visible.trim());
        }};
        const terminalPayloadLooksInlineStatusAnimation = (payload) => {{
            if (!terminalPayloadLooksInlineStatusRewrite(payload)) {{
                return false;
            }}
            const value = String(payload || '');
            const visible = stripAnsiForLowPowerTui(value)
                .replace(/[\r\n\t]+/g, ' ')
                .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')
                .slice(0, 256);
            return visible.includes('Working');
        }};
        const hostIsActiveRenderSurface = () => {{
            try {{
                if (host.getAttribute('data-terminal-app-control-backgrounded') === 'true') {{
                    return false;
                }}
                const rect = host.getBoundingClientRect();
                const style = window.getComputedStyle(host);
                const visiblyMounted =
                    style.display !== 'none'
                    && style.visibility !== 'hidden'
                    && Number(rect.width || 0) > 0
                    && Number(rect.height || 0) > 0
                    && Number(rect.right || 0) > 0
                    && Number(rect.bottom || 0) > 0
                    && Number(rect.left || 0) < Number(window.innerWidth || 0)
                    && Number(rect.top || 0) < Number(window.innerHeight || 0);
                return visiblyMounted;
            }} catch (_error) {{
                return false;
            }}
        }};
        const terminalInputOverrideActive = () => {{
            try {{
                return host.getAttribute('data-terminal-input-override-active') === 'true';
            }} catch (_error) {{
                return false;
            }}
        }};
        const terminalWindowFocused = () => {{
            try {{
                if (host.getAttribute('data-terminal-app-control-backgrounded') === 'true') {{
                    return false;
                }}
                return host.getAttribute('data-terminal-window-focused') === 'true';
            }} catch (_error) {{
                return false;
            }}
        }};
        const terminalDocumentHasFocus = () => {{
            try {{
                if (window.__yggtermForceForeground === true) {{ return true; }} return typeof document.hasFocus === 'function' ? Boolean(document.hasFocus()) : true;
            }} catch (_error) {{
                return true;
            }}
        }};
        const terminalHostHasInputFocus = () => {{
            try {{
                const active = document.activeElement;
                if (!active) {{
                    return false;
                }}
                if (active === host || host.contains(active)) {{
                    return true;
                }}
                if (term && term.textarea && active === term.textarea) {{
                    return true;
                }}
            }} catch (_error) {{}}
            return false;
        }};
        const activeWriteFrameBudgetApplies = () => {{
            if (!hostIsActiveRenderSurface()) {{
                return false;
            }}
            // XTERM-BUG: wayland-focus-gate — document.hasFocus() returns false
            // for a visibly-FOREGROUND KDE/Wayland window, so the old clause that
            // also required terminalDocumentHasFocus() (document.hasFocus)
            // starved the active write-frame budget: it fell to the 4000ms idle
            // value (effectiveTerminalWriteFrameMs), batching codex's continuous
            // synchronized-output animation into a 4s clock (jaggedy "Working"
            // wave) and forcing the user to TAP (terminalInputHot, below) to wake
            // realtime updates after refocus. terminalWindowFocused() derives from
            // the GTK DesktopWindowEvent::Focused signal (Wayland-reliable, set via
            // data-terminal-window-focused) and hostIsActiveRenderSurface() (the
            // top guard of this fn) already excludes hidden/off-screen/backgrounded
            // hosts — so a genuinely unfocused/backgrounded window still falls
            // through to the idle budget (spec: unfocused updates slowly). Same
            // visibility-not-document-focus substitution as the grid-fit fix
            // (~57626) — see [[finding-wayland-focus-gate-squished-viewport]] and
            // [[finding-xterm-latency-progressive-degradation]].
            if (terminalWindowFocused()) {{
                return true;
            }}
            return terminalInputHot() || (terminalInputOverrideActive() && terminalHostHasInputFocus());
        }};
        const effectiveTerminalWriteFrameMs = () => {{
            if (!activeWriteFrameBudgetApplies()) {{
                return terminalWriteFrameMs;
            }}
            if (terminalInputHot()) {{
                return terminalActiveWriteFrameMs;
            }}
            if (!recentInlineStatusAnimationHot()) {{
                return terminalActiveWriteFrameMs;
            }}
            const elapsedMs = recentInlineStatusAnimationStartedAtMs > 0
                ? Math.max(0, Date.now() - recentInlineStatusAnimationStartedAtMs)
                : 0;
            if (elapsedMs >= terminalInlineStatusAnimationLongAfterMs) {{
                return Math.max(
                    Math.min(terminalActiveWriteFrameMs, terminalActiveAnimationWriteFrameMs),
                    terminalActiveAnimationLongWriteFrameMs
                );
            }}
            return elapsedMs >= terminalInlineStatusAnimationSustainedAfterMs
                ? Math.max(
                    Math.min(terminalActiveWriteFrameMs, terminalActiveAnimationWriteFrameMs),
                    terminalActiveAnimationSustainedWriteFrameMs
                )
                : Math.min(terminalActiveWriteFrameMs, terminalActiveAnimationWriteFrameMs);
        }};
        const syncTerminalWriteFrameBudgetHostEntry = () => {{
            try {{
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (!entry) {{
                    return;
                }}
                entry.terminalWriteFrameMs = terminalWriteFrameMs;
                entry.terminalActiveWriteFrameMs = terminalActiveWriteFrameMs;
                entry.terminalActiveAnimationWriteFrameMs = terminalActiveAnimationWriteFrameMs;
                entry.terminalActiveAnimationSustainedWriteFrameMs =
                    terminalActiveAnimationSustainedWriteFrameMs;
                entry.recentInlineStatusAnimationStartedAtMs =
                    recentInlineStatusAnimationStartedAtMs;
                entry.effectiveTerminalWriteFrameMs = effectiveTerminalWriteFrameMs();
                entry.activeWriteFrameBudget = activeWriteFrameBudgetApplies();
                entry.recentInlineStatusAnimationHot = recentInlineStatusAnimationHot();
            }} catch (_error) {{}}
        }};
        let yggtermWasActiveHost = false;
        try {{ yggtermWasActiveHost = host.getAttribute('data-active-session-host') === 'true'; }} catch (_error) {{}}
        // HOT-tier Phase 3 reuses a hidden xterm host on switch (visibility flip,
        // no remount). The codex input-line decoration (the prompt-line
        // background band) can be left stale until the first keystroke. When
        // this host becomes active, rebuild ONLY that decoration overlay — do
        // NOT fit()/clearTextureAtlas()/refresh() the whole canvas, which blanks
        // a reused WebGL host until new PTY data arrives.
        const handleActiveHostRepaintOnSwitch = () => {{
            let isActive = false;
            try {{ isActive = host.getAttribute('data-active-session-host') === 'true'; }} catch (_error) {{}}
            if (isActive && !yggtermWasActiveHost) {{
                try {{ disposeXtermInputLineDecoration('reactivated'); }} catch (_error) {{}}
                try {{ syncXtermInputLineDecoration('reactivated'); }} catch (_error) {{}}
            }}
            yggtermWasActiveHost = isActive;
        }};
        try {{
            const budgetObserver = new MutationObserver(() => {{
                syncTerminalWriteFrameBudgetHostEntry();
                handleActiveHostRepaintOnSwitch();
                // Refocus must become realtime IMMEDIATELY. A flush timer
                // scheduled while unfocused used the 4000ms idle budget, and
                // schedulePendingWriteFlush never reschedules an existing timer —
                // so without this, pending output would stay stranded for up to 4s
                // after the window is refocused (the "tap + 300-500ms" lag). When
                // the realtime budget now applies, drop the stale timer and flush
                // now. This NEVER accelerates a genuinely unfocused/backgrounded
                // window: activeWriteFrameBudgetApplies() is false there.
                try {{
                    const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                    if (entry && !entry.writeBridgeInFlight
                        && String(entry.writeBridgePendingData || '').length > 0
                        && writeBridgeFlushTimer !== null
                        && activeWriteFrameBudgetApplies()) {{
                        clearTimeout(writeBridgeFlushTimer);
                        writeBridgeFlushTimer = null;
                        queueMicrotask(flushPendingWrite);
                    }}
                }} catch (_error) {{}}
            }});
            budgetObserver.observe(host, {{
                attributes: true,
                attributeFilter: ['data-terminal-app-control-backgrounded', 'data-terminal-window-focused', 'data-terminal-input-override-active', 'data-active-session-host'],
            }});
        }} catch (_error) {{}}
        try {{
            window.addEventListener('focus', syncTerminalWriteFrameBudgetHostEntry, true);
            window.addEventListener('blur', syncTerminalWriteFrameBudgetHostEntry, true);
            document.addEventListener('visibilitychange', syncTerminalWriteFrameBudgetHostEntry, true);
        }} catch (_error) {{}}
        syncTerminalWriteFrameBudgetHostEntry();
        const terminalSessionAllowsLowPowerTui = () => {{
            return false;
        }};
        const stripAnsiForLowPowerTui = (payload) => {{
            return String(payload || '')
                .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, '')
                .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
                .replace(/\x1b[()][A-Za-z0-9]/g, '')
                .replace(/[\x00\x0e\x0f]/g, '');
        }};
        const terminalPayloadLooksProtocolOnly = (payload) => {{
            const value = String(payload || '');
            return value.includes('\x1b') && !stripAnsiForLowPowerTui(value).trim();
        }};
        const terminalPayloadContainsCodexWelcomeSurface = (payload) => {{
            const value = String(payload || '');
            return value.includes('OpenAI Codex') && value.includes('/model to change');
        }};
        const lowPowerTuiXtermControlPrefix = (payload) => {{
            const value = String(payload || '');
            let prefix = '';
            if (value.includes('\x1b[?1049h')) {{
                prefix += '\x1b[?1049h';
            }}
            if (value.includes('\x1b[?25l')) {{
                prefix += '\x1b[?25l';
            }}
            if (!prefix && value.includes('\x1b[2J')) {{
                prefix = '\x1b[?25l';
            }}
            return prefix;
        }};
        const lowPowerTuiTextFromPayload = (payload) => {{
            const frame = coalesceHighVolumeTerminalPayload(String(payload || ''))
                .replace(/\x1b\[\?1049[hl]/g, '');
            const normalized = stripAnsiForLowPowerTui(frame)
                .replace(/\r\n/g, '\n')
                .replace(/\r/g, '\n')
                .replace(/\t/g, '    ');
            const rows = Math.max(1, Number(term && term.rows ? term.rows : 24));
            const cols = Math.max(1, Number(term && term.cols ? term.cols : 80));
            const lines = normalized.split('\n');
            return lines
                .slice(Math.max(0, lines.length - rows))
                .map((line) => String(line || '').slice(0, cols))
                .join('\n')
                .slice(-8192);
        }};
        const renderLowPowerTuiPayload = (payload) => {{
            const overlay = ensureLowPowerTuiOverlay();
            if (!overlay) {{
                return false;
            }}
            const value = String(payload || '');
            const text = lowPowerTuiTextFromPayload(value);
            const startsFreshFrame =
                value.includes('\x1b[?1049h')
                || value.includes('\x1b[2J')
                || value.includes('\x1b[H')
                || value.includes('\x1b[1;1H')
                || value.includes('\x1b[1;1f')
                || value.includes('\x1b[;H')
                || value.includes('\x1b[0;0H');
            if (startsFreshFrame) {{
                lowPowerTuiTextBuffer = text;
            }} else if (text) {{
                const rows = Math.max(1, Number(term && term.rows ? term.rows : 24));
                lowPowerTuiTextBuffer = `${{lowPowerTuiTextBuffer}}${{text}}`
                    .split('\n')
                    .slice(-rows)
                    .join('\n')
                    .slice(-8192);
            }}
            const renderedText = lowPowerTuiTextBuffer || text;
            if (!renderedText.trim()) {{
                return false;
            }}
            overlay.style.display = 'block';
            overlay.textContent = renderedText;
            lowPowerTuiActive = true;
            lowPowerTuiFrameCount += 1;
            lowPowerTuiLastText = renderedText;
            if (!tracedLowPowerTuiActive) {{
                tracedLowPowerTuiActive = true;
                sendTerminalEvent({{
                    kind: "debug",
                    message: `low_power_tui_active host=${{hostId}} frames=${{lowPowerTuiFrameCount}} chars=${{renderedText.length}}`
                }});
            }}
            syncLowPowerTuiHostEntry();
            requestRenderProbe('low_power_tui');
            emitHostHealthThrottled();
            return true;
        }};
        const filterPayloadForRenderSurface = (payload) => {{
            const rawValue = String(payload || '');
            const protocolCleanValue = sanitizeVisibleProtocolResponseLeakPayload(rawValue);
            const value = sanitizeInternalTransportPayload(protocolCleanValue);
            if (!value) {{
                const currentEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (currentEntry && rawValue) {{
                    currentEntry.transportLeakDroppedWriteCount =
                        Number(currentEntry.transportLeakDroppedWriteCount || 0) + 1;
                }}
                return '';
            }}
            if (!value) {{
                return '';
            }}
            const exitsAltScreen = value.includes('\x1b[?1049l');
            const entersAltScreen = value.includes('\x1b[?1049h');
            const activeRenderSurface = hostIsActiveRenderSurface();
            const documentFocused = terminalDocumentHasFocus();
            const hostInputFocused = terminalHostHasInputFocus();
            const lowPowerAllowed = terminalSessionAllowsLowPowerTui();
            const protocolOnly = terminalPayloadLooksProtocolOnly(value);
            if (activeRenderSurface && lowPowerAllowed && !programmaticFocusEnabled && backgroundTuiSuppressActive && !exitsAltScreen && !protocolOnly) {{
                unfocusedTuiFrameDropCount += 1;
                unfocusedTuiLastTail = value.slice(-240);
                if (!tracedUnfocusedTuiDrop) {{
                    tracedUnfocusedTuiDrop = true;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `background_tui_drop host=${{hostId}} frameLike=fast protocolOnly=false chars=${{value.length}}`
                    }});
                }}
                syncLowPowerTuiHostEntry();
                emitHostHealthThrottled();
                return '';
            }}
            const frameLike = terminalPayloadLooksHighVolumeFrame(value);
            const codexWelcome = terminalPayloadContainsCodexWelcomeSurface(value);
            const lowPowerCandidate = !exitsAltScreen && (lowPowerTuiActive || backgroundTuiSuppressActive || entersAltScreen || frameLike);
            if (!tracedTuiFilterProbe && (value.includes('?1049') || value.includes('\x1b[?25l') || frameLike || protocolOnly)) {{
                tracedTuiFilterProbe = true;
                sendTerminalEvent({{
                    kind: "debug",
                    message: `tui_filter_probe host=${{hostId}} enters=${{entersAltScreen}} exits=${{exitsAltScreen}} activeSurface=${{activeRenderSurface}} documentFocused=${{documentFocused}} hostInputFocused=${{hostInputFocused}} lowPowerAllowed=${{lowPowerAllowed}} frameLike=${{frameLike}} protocolOnly=${{protocolOnly}} chars=${{value.length}}`
                }});
            }}
            const activePlainLowPowerTui =
                activeRenderSurface
                && lowPowerAllowed
                && programmaticFocusEnabled
                && !terminalInputHot()
                && !codexWelcome
                && !protocolOnly
                && lowPowerCandidate;
            const activePlainLowPowerWasActive = Boolean(lowPowerTuiActive);
            if (activePlainLowPowerTui && renderLowPowerTuiPayload(value)) {{
                return activePlainLowPowerWasActive ? '' : lowPowerTuiXtermControlPrefix(value);
            }}
            if (activeRenderSurface && lowPowerAllowed && !programmaticFocusEnabled && !codexWelcome && lowPowerCandidate) {{
                if (entersAltScreen) {{
                    backgroundTuiSuppressActive = true;
                }}
                unfocusedTuiFrameDropCount += 1;
                unfocusedTuiLastTail = value.slice(-240);
                if (!tracedUnfocusedTuiDrop) {{
                    tracedUnfocusedTuiDrop = true;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `background_tui_drop host=${{hostId}} frameLike=${{frameLike}} protocolOnly=${{protocolOnly}} chars=${{value.length}}`
                    }});
                }}
                syncLowPowerTuiHostEntry();
                emitHostHealthThrottled();
                return '';
            }}
            if (!activeRenderSurface && lowPowerAllowed && lowPowerCandidate) {{
                if (entersAltScreen) {{
                    lowPowerTuiActive = true;
                    lowPowerTuiTextBuffer = '';
                }}
                inactiveTuiFrameDropCount += 1;
                inactiveTuiLastTail = value.slice(-240);
                if (!tracedInactiveTuiDrop) {{
                    tracedInactiveTuiDrop = true;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `inactive_tui_drop host=${{hostId}} frameLike=${{frameLike}} protocolOnly=${{protocolOnly}} chars=${{value.length}}`
                    }});
                }}
                syncLowPowerTuiHostEntry();
                emitHostHealthThrottled();
                return '';
            }}
            if (exitsAltScreen) {{
                backgroundTuiSuppressActive = false;
                hideLowPowerTuiOverlay();
                return value;
            }}
            if (activeRenderSurface && lowPowerTuiActive) {{
                hideLowPowerTuiOverlay();
                const replayPrefix = value.includes('\x1b[?1049h')
                    ? ''
                    : '\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H';
                return `${{replayPrefix}}${{value}}`;
            }}
            return value;
        }};
        const finalizeWriteFlush = (flushShouldFollow, callbackFired, paintRepairReason = '') => {{
            const currentEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
            const now = Date.now();
            const flushElapsedMs = lastWriteFlushStartedAtMs > 0
                ? Math.max(0, now - lastWriteFlushStartedAtMs)
                : null;
            if (currentEntry) {{
                currentEntry.writeBridgeInFlight = false;
                if (flushElapsedMs !== null) {{
                    currentEntry.writeBridgeFlushMaxElapsedMs = Math.max(
                        Number(currentEntry.writeBridgeFlushMaxElapsedMs || 0),
                        flushElapsedMs
                    );
                }}
                // Backlog drained for this flush cycle; if nothing is queued
                // behind it, clear the oldest-un-drained-byte clock.
                if (!String(currentEntry.writeBridgePendingData || '').length) {{
                    currentEntry.writeBridgePendingSinceMs = 0;
                }}
                if (callbackFired) {{
                    currentEntry.writeCallbackCount =
                        Number(currentEntry.writeCallbackCount || 0) + 1;
                    currentEntry.lastWriteCallbackAtMs = Date.now();
                }}
                const frameLikeHot = recentFrameLikeWriteHot();
                const sampleIntervalMs = terminalInputHot()
                    ? 1000
                    : (frameLikeHot ? terminalFrameLikeInstrumentationThrottleMs() : 250);
                if (now - lastWriteAppliedSampleAtMs >= sampleIntervalMs) {{
                    lastWriteAppliedSampleAtMs = now;
                    currentEntry.lastWriteAppliedTail = frameLikeHot
                        ? String(currentEntry.lastWriteSample || currentEntry.lastWriteAppliedTail || '').slice(-240)
                        : readTerminalBufferSample().slice(-240);
                }}
            }}
            if (flushShouldFollow) {{
                scrollLiveCursorIntoView();
            }} else {{
                syncScrollbackLock();
            }}
            syncXtermInputLineDecoration('write_flush');
            if (paintRepairReason) {{
                scheduleRetainedWritePaintRepair(paintRepairReason);
            }} else {{
                requestRenderProbe('write_flush');
            }}
                // ⛔ The SAME numbers as the perf event below, deliberately, and
                // taken from the same locals rather than re-measured — two
                // instruments reporting one flush must not be able to disagree
                // about how long it took. What differs is the plane and the
                // grammar: this row carries `layer`, `clock` and `seq`, so it
                // can be ordered against a reset and a render frame; the perf
                // row keeps feeding the existing rollups unchanged.
                traceXtermFlush(flushElapsedMs, {{
                    callback_fired: Boolean(callbackFired),
                    flush_should_follow: Boolean(flushShouldFollow),
                    paint_repair_reason: String(paintRepairReason || ''),
                    pending_chars: currentEntry ? String(currentEntry.writeBridgePendingData || '').length : 0,
                    raw_payload_length: currentEntry ? Number(currentEntry.lastRawPayloadLength || 0) : 0,
                    raw_payload_line_count: currentEntry ? Number(currentEntry.lastRawPayloadLineCount || 0) : 0,
                    effective_frame_ms: currentEntry ? Number(currentEntry.effectiveTerminalWriteFrameMs || 0) : effectiveTerminalWriteFrameMs(),
                }});
                emitPerf("xterm_write_flush", {{
                    callback_fired: Boolean(callbackFired),
                    flush_should_follow: Boolean(flushShouldFollow),
                    paint_repair: Boolean(paintRepairReason),
                    paint_repair_reason: String(paintRepairReason || ''),
                    elapsed_ms: flushElapsedMs,
                    effective_frame_ms: currentEntry ? Number(currentEntry.effectiveTerminalWriteFrameMs || 0) : effectiveTerminalWriteFrameMs(),
                    active_frame_budget: currentEntry ? Boolean(currentEntry.activeWriteFrameBudget) : activeWriteFrameBudgetApplies(),
                    pending_chars: currentEntry ? String(currentEntry.writeBridgePendingData || '').length : 0,
                    last_raw_payload_length: currentEntry ? Number(currentEntry.lastRawPayloadLength || 0) : 0,
                    last_raw_payload_line_count: currentEntry ? Number(currentEntry.lastRawPayloadLineCount || 0) : 0,
                }});
                emitHostHealthThrottled();
                schedulePendingWriteFlush(false);
            }};
            const flushPendingWrite = () => {{
                if (writeBridgeFlushTimer !== null) {{
                    clearTimeout(writeBridgeFlushTimer);
                    writeBridgeFlushTimer = null;
                }}
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (!entry || entry.writeBridgeInFlight) {{
                    return;
            }}
            let payload = String(entry.writeBridgePendingData || '');
            if (!payload) {{
                return;
            }}
            // XTERM-BUG: cold-reveal-bulk-write-freeze — xterm's WriteBuffer
            // 12ms deadline only yields BETWEEN queued write entries; a single
            // giant entry parses in one synchronous block (a 516KB backlog
            // drain measured 5128ms). Cap each term.write slice so a bulk
            // drain becomes several entries with event-loop breathing room
            // between them. Cut at a line boundary when one is near the cap so
            // the line-based leak sanitizers below never see a split marker
            // line; the remainder stays in writeBridgePendingData and the
            // existing finalizeWriteFlush -> schedulePendingWriteFlush chain
            // flushes it. Content and order are unchanged — pacing only.
            const bulkFlushMaxChars = 131072;
            if (payload.length > bulkFlushMaxChars) {{
                let cutAt = payload.lastIndexOf('\n', bulkFlushMaxChars);
                cutAt = cutAt >= bulkFlushMaxChars / 2 ? cutAt + 1 : bulkFlushMaxChars;
                entry.writeBridgePendingData = payload.slice(cutAt);
                payload = payload.slice(0, cutAt);
            }} else {{
                entry.writeBridgePendingData = '';
            }}
            payload = filterPayloadForRenderSurface(payload);
            if (!payload) {{
                syncLowPowerTuiHostEntry();
                emitHostHealthThrottled();
                return;
            }}
            if (resetVisibleTransportLeakBeforeWrite(payload)) {{
                payload = `\x1bc\x1b[2J\x1b[3J\x1b[H${{payload}}`;
            }}
            const rawPayloadLength = payload.length;
            const rawFrameLike = terminalPayloadLooksHighVolumeFrame(payload);
            const rawPayloadLineCount = (payload.match(/\n/g) || []).length;
            const writeFrameMs = effectiveTerminalWriteFrameMs();
            syncTerminalWriteFrameBudgetHostEntry();
            if (rawFrameLike || payload.includes('\x1b[?1049h') || payload.includes('\x1b[?25l')) {{
                recentFrameLikeWriteUntilMs = Date.now() + Math.max(600, writeFrameMs * 2);
                if (entry) {{
                    entry.recentFrameLikeWriteUntilMs = recentFrameLikeWriteUntilMs;
                }}
            }}
            const retainedScrollbackReplay =
                rawPayloadLineCount > Math.max(4, Number(term && term.rows ? term.rows : 0) + 4)
                && (payload.includes('\x1b[2J') || payload.includes('YGG_REMOTE_RETAINED_SCROLLBACK_'));
            const expectedScrollbackPayload =
                (!rawFrameLike || retainedScrollbackReplay)
                && !payload.includes('\x1b[?1049h')
                && currentBufferKind() === 'normal'
                && rawPayloadLineCount > Math.max(4, Number(term && term.rows ? term.rows : 0) + 4);
            if (entry) {{
                entry.lastRawPayloadLength = rawPayloadLength;
                entry.lastRawPayloadLineCount = rawPayloadLineCount;
                entry.lastRawPayloadSample = terminalPayloadDebugSample(payload);
                entry.terminalContentSource = 'daemon_pty';
                entry.terminalSourceMismatchReason = '';
                if (expectedScrollbackPayload) {{
                    entry.scrollbackExpected = true;
                }}
                // XTERM-BUG: webgl-stale-cursor-on-cold-reveal
                // On a cold switch-in, scheduleActivationRepaint fires its heavy
                // (atlas-clearing) redraws at now/120ms/360ms — but a COLD remote
                // session's live daemon content streams in seconds LATER, after
                // those timers. The shadow->live swap then paints the first live
                // frame with no atlas-clear, so the WebGL cursor cell is left stale
                // ("broken paint at the cursor right after switching in", which
                // self-heals on the next render). Arm a one-shot on switch-in and
                // force ONE clean redraw when the live daemon content actually
                // arrives, regardless of how long the cold attach took.
                if (
                    entry.pendingRevealDaemonRepaint
                    && Date.now() < Number(entry.pendingRevealDaemonRepaintUntilMs || 0)
                ) {{
                    entry.pendingRevealDaemonRepaint = false;
                    entry.pendingRevealDaemonRepaintUntilMs = 0;
                    try {{
                        window.requestAnimationFrame(() => {{
                            try {{ redrawTerminal('reveal_daemon_content'); }} catch (_revealRepaintError) {{}}
                        }});
                    }} catch (_revealRepaintScheduleError) {{}}
                }}
            }}
            // OSC 52 re-fire on switch (finding-osc52-copy-chime-replay-refire): the
            // daemon re-streams a session's BUFFERED output when the viewer attaches,
            // and that bulk catch-up carries any OSC 52 the CLI emitted earlier (CC's
            // select-copy). Replayed history must not re-fire the clipboard copy + chime.
            //
            // ⛔ THE ARM MUST NAME REPLAYED HISTORY, NOT MERELY A BIG PAYLOAD. It used
            // to fire on `expectedScrollbackPayload` alone — "more lines than the grid
            // is tall, in the normal buffer" — which is not a property of a replay at
            // all. It is what an agent CLI printing a long tool result looks like, and
            // Claude Code lives in the normal buffer. So a copy made WHILE such a block
            // was streaming rode in on the very payload that armed the window (the arm
            // runs before the parse) and suppressed ITSELF. Whether a copy survived
            // depended on how the daemon happened to chunk the output around it: the
            // "it works sometimes" the user reported. The catch-up this guards is
            // bounded in time by the attach that causes it, so ask for THAT — a
            // retained-scrollback replay, or a bulk payload landing inside the
            // switch-in window — instead of asking whether the payload was large.
            const osc52ReplayLike =
                retainedScrollbackReplay
                || (entry && Date.now() - Number(entry.lastActivationAtMs || 0) < 8000);
            if (osc52ReplayLike
                && (retainedScrollbackReplay || expectedScrollbackPayload)
                && payload.indexOf('\x1b]52;') !== -1
                && window.__yggtermArmOsc52Suppress) {{
                window.__yggtermArmOsc52Suppress(hostId, 2000);
            }}
            if (!retainedScrollbackReplay) {{
                payload = coalesceHighVolumeTerminalPayload(payload);
            }}
            const renderPayloadLength = payload.length;
            if (entry) {{
                entry.lastCoalescedPayloadLength = renderPayloadLength;
            }}
            const initialRetainedRepair =
                retainedWritePaintRepairCount < 2
                && expectedScrollbackPayload
                && rawPayloadLength >= 1024;
            const bulkNormalRepair =
                retainedWritePaintRepairCount < 4
                && rawPayloadLength >= 4096
                && !rawFrameLike;
            const paintRepairReason = (
                retainedScrollbackReplay
                || initialRetainedRepair
                || bulkNormalRepair
            ) ? `write_flush_retained len=${{rawPayloadLength}} frame=${{rawFrameLike}}` : '';
            entry.writeBridgeInFlight = true;
                entry.writeBridgeFlushCount = Number(entry.writeBridgeFlushCount || 0) + 1;
                lastWriteFlushStartedAtMs = Date.now();
                entry.lastWriteFlushStartedAtMs = lastWriteFlushStartedAtMs;
                // Follow the live bottom whenever we are in PromptFollow (Following).
                // We intentionally do NOT gate on liveCursorNearBottom / scrollbackLocked:
                // a Following session whose viewport fell behind during an output burst
                // (a PASSIVE strand) MUST re-follow to catch up — that strand is the
                // working-session "past conversation vacuumed + cursor clipped below the
                // host" symptom (finding-working-state-row-overlap). The old
                // liveCursorNearBottom gate stopped re-following once the gap exceeded
                // ~rows/3, leaving the viewport permanently stranded. A GENUINE user
                // scroll-up now reliably flips intent to UserScrollback via
                // syncScrollbackLock's ydisp-decrease detection (works even during
                // output), so following-on-strand never yanks a user reading scrollback.
                // This also subsumes the retained-replay re-seed case (re-seed lands at
                // the top with intent PromptFollow -> follow). syncScrollbackLock() is
                // still called for its reached-bottom/lock side effects. Guards the
                // follow DECISION, never the low-level mover, per
                // [[audit-viewport-scroll-control-flow]]. See
                // [[finding-working-state-row-overlap-scrollback-empty]].
                syncScrollbackLock('write_flush');
                const flushShouldFollow =
                    scrollbackIntent !== 'UserScrollback';
            const syncWrite = term && term._core && typeof term._core.writeSync === 'function'
                ? term._core.writeSync.bind(term._core)
                : entry && entry.term && entry.term._core && entry.term._core._writeBuffer && typeof entry.term._core._writeBuffer.writeSync === 'function'
                    ? entry.term._core._writeBuffer.writeSync.bind(entry.term._core._writeBuffer)
                    : null;
            try {{
                const rawSynchronizedSmallFrame =
                    renderPayloadLength < 2048
                    && payload.includes('\x1b[?2026h')
                    && payload.includes('\x1b[?2026l');
                const syncWriteBypassFrameBudget =
                    rawFrameLike
                    || rawSynchronizedSmallFrame
                    || terminalPayloadLooksSynchronizedRepaintFrame(payload)
                    || terminalPayloadLooksInlineStatusRewrite(payload)
                    || recentInlineStatusAnimationHot()
                    || recentFrameLikeWriteHot();
                const preferSyncWrite =
                    syncWrite
                    && !syncWriteBypassFrameBudget
                    && renderPayloadLength < 2048;
                // The last point at which the LIVE stream is still bytes. Both
                // arms below hand it to the canvas, so this is the honest place
                // to sample what the canvas was given.
                if (window.__yggtermTrace && window.__yggtermTrace.captureStream) {{
                    window.__yggtermTrace.captureStream(hostId, "live_stream", payload);
                }}
                if (preferSyncWrite) {{
                    syncWrite(payload);
                    finalizeWriteFlush(flushShouldFollow, false, paintRepairReason);
                    return;
                }}
                term.write(payload, () => {{
                    finalizeWriteFlush(flushShouldFollow, true, paintRepairReason);
                }});
            }} catch (error) {{
                if (entry) {{
                    entry.writeBridgeInFlight = false;
                    entry.lastWriteError =
                        error && error.message ? String(error.message) : String(error);
                }}
                emitHostHealth();
                schedulePendingWriteFlush(false);
            }}
        }};
        const schedulePendingWriteFlush = (force = false) => {{
            const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
            if (!entry || entry.writeBridgeInFlight) {{
                return;
            }}
            const pendingLength = String(entry.writeBridgePendingData || '').length;
            if (pendingLength <= 0) {{
                return;
            }}
            const now = Date.now();
            const writeFrameMs = effectiveTerminalWriteFrameMs();
            syncTerminalWriteFrameBudgetHostEntry();
            const elapsedSinceFlush = lastWriteFlushStartedAtMs > 0
                ? now - lastWriteFlushStartedAtMs
                : Number.POSITIVE_INFINITY;
            const pendingData = String(entry.writeBridgePendingData || '');
            const frameBudgetedPending =
                activeWriteFrameBudgetApplies()
                || recentInlineStatusAnimationHot()
                || recentFrameLikeWriteHot()
                || terminalPayloadLooksHighVolumeFrame(pendingData)
                || terminalPayloadLooksSynchronizedRepaintFrame(pendingData)
                || terminalPayloadLooksInlineStatusRewrite(pendingData);
            if (
                !force
                && writeFrameMs > 0
                && frameBudgetedPending
                && elapsedSinceFlush < writeFrameMs
            ) {{
                if (writeBridgeFlushTimer === null) {{
                    writeBridgeFlushTimer = window.setTimeout(() => {{
                        writeBridgeFlushTimer = null;
                        flushPendingWrite();
                    }}, Math.max(8, writeFrameMs - elapsedSinceFlush));
                }}
                return;
            }}
            queueMicrotask(flushPendingWrite);
        }};
        const terminalPayloadShouldFlushImmediately = (payload) => {{
            const value = String(payload || '');
            if (!value || !terminalInputHot()) {{
                return false;
            }}
            if (value.length > 1024 || terminalPayloadLooksHighVolumeFrame(value)) {{
                return false;
            }}
            if (value.includes('\x1b[?1049h') || value.includes('\x1b[2J')) {{
                return false;
            }}
            return true;
        }};
        const coalesceHighVolumeTerminalPayload = (payload) => {{
            return coalesceSynchronizedOutputFrames(String(payload || ''));
        }};
        const writeParsedDisposable = typeof term.onWriteParsed === 'function'
            ? term.onWriteParsed(() => {{
                const currentEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (currentEntry) {{
                    currentEntry.writeParsedCount = Number(currentEntry.writeParsedCount || 0) + 1;
                    currentEntry.lastWriteParsedAtMs = Date.now();
                }}
                paintNoteParsed();
                applySoftwareCanvasLayerOptimization('write_parsed');
                syncXtermInputLineDecoration('write_parsed');
                scheduleCursorCellBackgroundRefresh('write_parsed');
                requestRenderProbe('write_parsed');
                emitHostHealthThrottled();
            }})
            : null;
        const selectionDisposable = typeof term.onSelectionChange === 'function'
            ? term.onSelectionChange(() => {{
                // SCROLL MODE = SELECTING/PINNED: the moment the user has a
                // non-empty selection, pin the viewport (UserScrollback) so
                // streaming agent output does NOT auto-follow and yank the
                // viewport out from under the drag. The pin persists after the
                // selection clears (the user is reading) until they scroll back
                // to the bottom or type at the prompt (which snaps to follow).
                // This guards the follow DECISION via the existing intent state,
                // NOT the low-level viewport mover (guarding the mover broke DOM
                // sync). See [[audit-viewport-scroll-control-flow]].
                // The pin MUST stay immediate — it is what keeps streaming
                // output from yanking the viewport mid-drag.
                // ⭐ AND THE RELEASE, which is what was missing. A selection made
                // while the viewport is ALREADY AT THE BOTTOM pinned it with no
                // way back: the documented escape is "scroll back to the
                // bottom", and there is no scrolling to do when you never left.
                // So the session sat on `UserScrollback` forever, showing
                // "N new messages (ctrl+End)" while output kept arriving, and
                // only a session switch — which remounts and resets the intent —
                // brought it back. User-reported with a screenshot, 2026-08-01.
                //
                // The release is deliberately narrow, because the pin's original
                // purpose is real: it must survive for someone who scrolled UP
                // to read. So it fires only when all three hold — the selection
                // is now EMPTY, the pin is the one this handler set
                // (`selection_active`, not a wheel or a key), and the viewport
                // is within the pin threshold of the base. Someone who scrolled
                // up and then selected keeps their place.
                // ⛔ ONE OWNER: applySelectionScrollbackIntent, which the
                // pointer handlers also call. This path is the one that does
                // NOT arrive under streaming load (measured: 0 firings across a
                // drag that selected 902,649 chars), so it may never be the
                // only caller — but it is still correct when it does arrive
                // (a keyboard/API selection change reaches nothing else), and
                // routing it here is what keeps the two from disagreeing.
                applySelectionScrollbackIntent(terminalHasSelection());
                // CC-DRAG-STALL: everything else (the O(selected-cells)
                // selection serialization, host-entry sync, canvas-layer pass,
                // health telemetry) is deferred to the coalesced trailing
                // edge so per-event work stays O(1) during a drag over a
                // streaming session.
                schedulePrimarySelectionSync();
            }})
            : null;
        const renderDisposable = term.onRender((renderRange) => {{
            renderEventCount += 1;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].renderEventCount = renderEventCount;
                window.__yggtermXtermHosts[hostId].lastRenderEventAtMs = Date.now();
            }}
            // The row RANGE, which the old counter threw away. It is the
            // difference between "one line was repainted" and "the whole
            // viewport was repainted", and the reported corruption is a
            // whole-viewport symptom — so without the range the counter cannot
            // tell a healthy busy terminal from a session repainting everything
            // it owns, over and over.
            try {{
                traceXtermRender(
                    renderRange ? renderRange.start : 0,
                    renderRange ? renderRange.end : 0,
                    term ? Number(term.rows || 0) : 0
                );
                // Same three reads, taken once: two instruments reporting one
                // frame must not be able to disagree about which rows it was.
                paintNoteFrame(
                    renderRange ? renderRange.start : 0,
                    renderRange ? renderRange.end : 0,
                    term ? Number(term.rows || 0) : 0
                );
            }} catch (_error) {{}}
            // XTERM-BUG: webgl-stale-atlas-garble — residual glyph-corruption
            // paths (occlusion throttle, monitor wake, any path the focus/switch
            // activation repaint misses) render against a glyph atlas that went
            // stale during a rAF-throttle gap. Detect the exact risk condition
            // (render right after a >1s rAF gap, no atlas clear since the gap
            // began, this host visible/active), trace it via the render_anomaly
            // fail-pattern channel, and heal with a targeted atlas clear +
            // refresh. Latched per gap episode so the heal's own refresh cannot
            // re-trigger it.
            try {{
                const rafGapMonitor = window.__yggtermRafGapMonitor;
                const staleAtlasNowMs = Date.now();
                // ⛔⛔ THE LATCH IS PAGE-GLOBAL, NOT PER-HOST. 3.0.106 dropped the
                // old "render landed within 600 ms of the gap" precondition
                // because a stale atlas stays stale — directionally right, but it
                // exposed two latent bugs and made this fire in a loop, wiping the
                // glyph atlas mid-session ~18 times and painting cells before
                // their glyphs could re-rasterize. That is what ate digits out of
                // the owner's line-number gutter and punched holes in his diff
                // highlight on 2026-08-11. Caught from the trace this campaign had
                // just taught to report honestly: the SAME `raf_gap_ms: 1794`
                // re-triggering at `render_lag_after_gap_ms` 483s, 508s, 724s,
                // 776s, with `atlas_age_ms: -1` throughout.
                //
                // Bug 1: `lastStaleAtlasHealGapEndMs` is a CLOSURE variable, so it
                // resets to 0 on every host mount — and a remount then re-armed an
                // ancient gap. The latch now lives on the page-global monitor,
                // which is where the gap it latches lives.
                //
                // Bug 2 (below): `atlasClearedAtMs === 0` was read as maximally
                // stale when it means the opposite.
                if (
                    rafGapMonitor
                    && rafGapMonitor.lastGapEndedAtMs > 0
                    && rafGapMonitor.lastGapMs > 1000
                    && rafGapMonitor.lastGapEndedAtMs !== rafGapMonitor.lastHealedGapEndedAtMs
                    && host.getAttribute('data-active-session-host') === 'true'
                ) {{
                    const gapStartedAtMs = rafGapMonitor.lastGapEndedAtMs - rafGapMonitor.lastGapMs;
                    const staleAtlasEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                    const atlasClearedAtMs = Math.max(
                        Number(lastAtlasClearAtMs || 0),
                        staleAtlasEntry ? Number(staleAtlasEntry.lastAtlasClearAtMs || 0) : 0
                    );
                    // ⛔ `atlasClearedAtMs === 0` means the atlas has NEVER been
                    // cleared — i.e. it was built fresh when this host mounted and
                    // nothing has invalidated it since. That is the HEALTHIEST
                    // state there is, and `0 < gapStartedAtMs` read it as the
                    // stalest, so every freshly mounted host healed itself
                    // immediately and forever against whatever gap it inherited.
                    // A host that mounted after the gap cannot have been staled by
                    // it either: its atlas did not exist while the throttle ran.
                    const hostMountedAtMs = staleAtlasEntry
                        ? Number(staleAtlasEntry.mountedAtMs || 0)
                        : 0;
                    const atlasPredatesGap =
                        atlasClearedAtMs > 0 && atlasClearedAtMs < gapStartedAtMs;
                    const hostExistedDuringGap =
                        hostMountedAtMs > 0 && hostMountedAtMs < gapStartedAtMs;
                    if (atlasPredatesGap || (atlasClearedAtMs === 0 && hostExistedDuringGap)) {{
                        rafGapMonitor.lastHealedGapEndedAtMs = rafGapMonitor.lastGapEndedAtMs;
                        lastStaleAtlasHealGapEndMs = rafGapMonitor.lastGapEndedAtMs;
                        staleAtlasHealCount += 1;
                        if (staleAtlasEntry) {{
                            staleAtlasEntry.staleAtlasHealCount = staleAtlasHealCount;
                            staleAtlasEntry.lastStaleAtlasHealAtMs = staleAtlasNowMs;
                        }}
                        // `heal_scheduled`, NOT `healed`. This field was the
                        // literal `true`, written HERE — before the setTimeout
                        // below had even been armed, let alone run. So every one
                        // of the 8 stale-atlas episodes on the owner's host
                        // reported a successful repair while he was looking at
                        // garbled glyphs, and the trace vouched for a fix nobody
                        // had measured. [[finding-a-set-is-not-a-fill]]: this
                        // records an INTENT, and only the follow-up below may
                        // speak about the outcome.
                        pendingRenderAnomaly = JSON.stringify({{
                            pattern: 'stale_atlas_paint',
                            raf_gap_ms: rafGapMonitor.lastGapMs,
                            atlas_age_ms: atlasClearedAtMs > 0 ? staleAtlasNowMs - atlasClearedAtMs : -1,
                            render_lag_after_gap_ms: staleAtlasNowMs - rafGapMonitor.lastGapEndedAtMs,
                            heal_count: staleAtlasHealCount,
                            window_focused: document.hasFocus(),
                            visibility: String(document.visibilityState || ''),
                            heal_scheduled: true,
                        }});
                        window.setTimeout(() => {{
                            const healStartedAtMs = Date.now();
                            let atlasCleared = false;
                            let rowsRefreshed = -1;
                            try {{
                                clearTerminalTextureAtlas();
                                atlasCleared = true;
                                if (term.refresh) {{
                                    rowsRefreshed = Math.max(0, term.rows - 1);
                                    term.refresh(0, rowsRefreshed);
                                }}
                            }} catch (_error) {{}}
                            // The outcome, separately traced, so a heal that
                            // throws or finds no `refresh` can no longer hide
                            // behind the intent recorded above.
                            try {{
                                pendingRenderAnomaly = JSON.stringify({{
                                    pattern: 'stale_atlas_heal_outcome',
                                    heal_count: staleAtlasHealCount,
                                    atlas_cleared: atlasCleared,
                                    rows_refreshed: rowsRefreshed,
                                    duration_ms: Date.now() - healStartedAtMs,
                                }});
                            }} catch (_error) {{}}
                            try {{ emitHostHealth(); }} catch (_error) {{}}
                        }}, 0);
                    }}
                }}
            }} catch (_error) {{}}
            applySoftwareCanvasLayerOptimization('render');
            syncXtermInputLineDecoration('render');
            scheduleCursorCellBackgroundRefresh('render');
            requestRenderProbe('render');
        }});
        const scrollDisposable = typeof term.onScroll === 'function'
            ? term.onScroll(() => {{
                scrollEventCount += 1;
                const scrollEntry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId];
                if (scrollEntry) {{
                    scrollEntry.scrollEventCount = scrollEventCount;
                }}
                // XTERM-BUG: cold-reveal-bulk-write-freeze — xterm fires onScroll
                // once PER SCROLLED LINE, synchronously inside term.write's parse
                // loop. During a bulk backlog drain (switch-back to a session that
                // produced thousands of lines while its reads were paused) the
                // heavy tail below ran thousands of times inside ONE synchronous
                // parse block and froze the whole webview (measured live: a 516KB
                // / 4921-line drain spent 5128ms inside a single write flush).
                // While a bridge write is in flight the per-line work is pure
                // waste — finalizeWriteFlush runs the same sync work once when
                // the flush completes. The 30s recency guard self-heals a stuck
                // in-flight flag so user-scroll semantics can never be lost.
                if (
                    scrollEntry
                    && scrollEntry.writeBridgeInFlight
                    && Date.now() - Number(scrollEntry.lastWriteFlushStartedAtMs || 0) < 30000
                ) {{
                    if (pendingPersistedScrollRestore) {{
                        tryApplyPendingPersistedScrollRestore('scroll_event');
                    }}
                    return;
                }}
                // XTERM-BUG: scrollback-lost-on-gui-restart — every scroll event
                // is a chance to apply a pending persisted restore (replay added rows).
                if (pendingPersistedScrollRestore) {{
                    tryApplyPendingPersistedScrollRestore('scroll_event');
                }}
                syncScrollbackLock('scroll_event');
                syncTerminalScrollController('scroll_event');
                applySoftwareCanvasLayerOptimization('scroll_event');
                syncXtermInputLineDecoration('scroll_event');
                // XTERM-BUG: scrollback-lost-on-gui-restart — throttled persist
                // so post-restart we can restore even when intent stays PromptFollow
                // (probe-scroll / write-bridge-in-flight suppress intent change).
                // Crucially: skip persists while a pending restore is active, so the
                // post-restart replay storm doesn't overwrite the user's saved spot.
                const nowMs = Date.now();
                const restoreInFlight = Boolean(pendingPersistedScrollRestore)
                    || (pendingPersistedScrollRestoreDeadlineMs > 0
                        && nowMs <= pendingPersistedScrollRestoreDeadlineMs);
                if (scrollbackLocked && !restoreInFlight && (nowMs - lastScrollPersistAtMs) >= 200) {{
                    lastScrollPersistAtMs = nowMs;
                    persistScrollStateToLocalStorage('scroll_event_throttled');
                }}
                emitHostHealth();
            }})
            : null;
        let lastHostHealthKey = '';
        const terminalChunkIsTransportError = (textValue) => {{
            const lines = String(textValue || '')
                .split(/\r?\n/)
                .map((line) => line.trim().toLowerCase())
                .filter((line) => line.length > 0);
            if (!lines.length) {{
                return false;
            }}
            const allText = lines.join(' ');
            const compactAllText = allText.replace(/\s+/g, '');
            if (
                (allText.includes('error: connecting to ')
                    && allText.includes('server-')
                    && allText.includes('.sock'))
                || (compactAllText.includes('error:connectingto')
                    && compactAllText.includes('server-')
                    && compactAllText.includes('.sock'))
            ) {{
                return true;
            }}
            const headLines = lines.slice(0, 4);
            const head = headLines.join(' ');
            const compactHead = head.replace(/\s+/g, '');
            if (
                (head.includes('error: connecting to ')
                    && head.includes('server-')
                    && head.includes('.sock'))
                || (compactHead.includes('error:connectingto')
                    && compactHead.includes('server-')
                    && compactHead.includes('.sock'))
            ) {{
                return true;
            }}
            const headFragments = [
                '[yggterm] terminal reader stopped',
                'error: reading /tmp/yggterm-screen',
                'mux_client_request_session',
                'session open refused by peer',
                'controlsocket',
                'exec: export: not found',
                'exec: __yggterm_initial_tty_size',
                'terminal session not found',
                '[screen is terminating]',
                'saved codex session',
                'cannot be restored as a live terminal',
                'warn ignoring stale yggterm daemon for current app version',
            ];
            if (headFragments.some((fragment) => head.includes(fragment))) {{
                return true;
            }}
            return headLines.some((line) => {{
                const directDiagnosticLine = !(line.startsWith('- ')
                    || line.startsWith('* ')
                    || line.startsWith('> '));
                if (!directDiagnosticLine) {{
                    return false;
                }}
                return (
                    (line.startsWith('shared connection to ')
                        && (line.includes(' closed') || line.includes('refused') || line.includes('timed out')))
                    || (line.startsWith('connection to ')
                        && (line.includes(' closed') || line.includes('refused') || line.includes('timed out')))
                    || line === 'permission denied'
                    || line === 'no route to host'
                    || line === 'broken pipe'
                    || line === 'connection reset by peer'
                    || ((line.startsWith('ssh:')
                        || line.startsWith('scp:')
                        || line.startsWith('sftp:')
                        || line.startsWith('error:')
                        || line.startsWith('fatal:')
                        || line.startsWith('rsync:'))
                        && (line.includes('permission denied')
                            || (line.includes('connecting to ')
                                && line.includes('server-')
                                && line.includes('.sock'))
                            || line.includes('connection refused')
                            || line.includes('no route to host')
                            || line.includes('connection timed out')
                            || line.includes('broken pipe')))
                );
            }});
        }};
        const emitHostHealth = () => {{
            try {{
                rebindCurrentHost('emit_host_health', false);
                const now = Date.now();
                const inputHot = terminalInputHot();
                const frameLikeHot = recentFrameLikeWriteHot();
                if (inputHot && now - lastInputHotHostHealthAtMs < 650) {{
                    return;
                }}
                if (inputHot) {{
                    lastInputHotHostHealthAtMs = now;
                }}
                if (
                    frameLikeHot
                    && now - lastFrameLikeHostHealthAtMs < terminalFrameLikeInstrumentationThrottleMs()
                ) {{
                    return;
                }}
                if (frameLikeHot) {{
                    lastFrameLikeHostHealthAtMs = now;
                }}
                const active = term && term.buffer ? term.buffer.active : null;
                const cursorLineIndex = active ? Number((active.baseY || 0) + (active.cursorY || 0)) : null;
                const cursorLine = (
                    active
                    && Number.isFinite(cursorLineIndex)
                    && cursorLineIndex >= 0
                    && active.getLine
                )
                    ? active.getLine(cursorLineIndex)
                    : null;
                const cursorLineText = cursorLine && cursorLine.translateToString
                    ? String(cursorLine.translateToString(true) || '')
                    : '';
                const cursorY = active ? Number(active.cursorY || 0) : 0;
                const rows = Number(term.rows || 0);
                const blankRowsBelowCursor = Math.max(0, rows - (cursorY + 1));
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId]
                    : null;
                const textTail = (inputHot || frameLikeHot) && entry
                    ? String(entry.lastWriteAppliedTail || '').slice(-240)
                    : readTerminalBufferSample().slice(-240);
                const hasTransportError = terminalChunkIsTransportError(cursorLineText)
                    || terminalChunkIsTransportError(textTail);
                const renderHealth = updateRenderHealth(
                    'host_health',
                    cursorLineText,
                    textTail,
                    {{ skip_ink_sample: frameLikeHot }}
                );
                // Count non-blank rows in the VISIBLE viewport — the completeness
                // signal for the blank-frame reveal fix. Fall back to `rows` (looks
                // complete) on any failure so a probe error never forces a repaint.
                let visibleNonblankRows = rows;
                try {{
                    if (active && active.getLine) {{
                        const top = Number(active.viewportY || 0);
                        let n = 0;
                        for (let i = 0; i < rows; i++) {{
                            const ln = active.getLine(top + i);
                            const t = ln && ln.translateToString ? ln.translateToString(true) : '';
                            if (t && t.trim().length > 0) n++;
                        }}
                        visibleNonblankRows = n;
                    }}
                }} catch (_error) {{ visibleNonblankRows = rows; }}
                const renderAnomaly = pendingRenderAnomaly || '';
                const nextKey = JSON.stringify([
                    hasTransportError,
                    cursorLineText.slice(-160),
                    textTail.slice(-160),
                    cursorY,
                    rows,
                    blankRowsBelowCursor,
                    renderHealth.status,
                    renderHealth.reason,
                    visibleNonblankRows < 3,
                    renderAnomaly,
                ]);
                if (nextKey === lastHostHealthKey) {{
                    return;
                }}
                lastHostHealthKey = nextKey;
                pendingRenderAnomaly = '';
                const payload = {{
                    kind: "host_health",
                    cursor_line_text: cursorLineText,
                    text_tail: textTail,
                    has_transport_error: hasTransportError,
                    frame_like_hot: frameLikeHot,
                    cursor_y: Math.max(0, Math.min(65535, Math.round(cursorY))),
                    rows: Math.max(0, Math.min(65535, Math.round(rows))),
                    blank_rows_below_cursor: Math.max(0, Math.min(65535, Math.round(blankRowsBelowCursor))),
                    render_health_status: renderHealth.status,
                    render_health_reason: renderHealth.reason,
                    render_health_recovery_count: renderHealth.recovery_count,
                    render_health_recovery_pending: renderHealth.recovery_pending,
                    visible_nonblank_rows: Math.max(0, Math.min(65535, Math.round(visibleNonblankRows))),
                    render_anomaly: renderAnomaly,
                    // XTERM-BUG: webgl-stale-atlas-garble — did the glyph-atlas
                    // defence run? Both counters existed in the page and were
                    // reported by nothing, so the owner's garbled-viewport report
                    // could not be closed in either direction. Zero clears on a
                    // GUI that has been occluded means prevention is not firing;
                    // a rising count with garble still reported means it fires
                    // and does not cure it — a different bug, and until now
                    // indistinguishable.
                    stale_atlas_heal_count: staleAtlasHealCount,
                    preemptive_atlas_clear_count: entry
                        ? Number(entry.preemptiveAtlasClearCount || 0)
                        : 0,
                    last_preemptive_atlas_clear_at_ms: entry
                        ? Number(entry.lastPreemptiveAtlasClearAtMs || 0)
                        : 0,
                }};
                if (entry) {{
                    entry.lastHostHealth = payload;
                    entry.emitHostHealth = emitHostHealth;
                }}
                const emptyRetainedSurfaceHealth =
                    !String(cursorLineText || '').trim()
                    && !String(textTail || '').trim()
                    && rows > 0
                    && blankRowsBelowCursor > Math.max(2, Math.floor(rows * 0.75));
                const textTailHasSignal = Boolean(String(textTail || '').trim());
                const suppressRustHostHealth =
                    frameLikeHot
                    && !hasTransportError
                    && !String(cursorLineText || '').trim()
                    && !textTailHasSignal
                    && !emptyRetainedSurfaceHealth;
                if (suppressRustHostHealth) {{
                    hotHostHealthSuppressedCount += 1;
                    if (entry) {{
                        entry.hotHostHealthSuppressedCount = hotHostHealthSuppressedCount;
                    }}
                    if (hostHealthAfterFrameTimer === null) {{
                        hostHealthAfterFrameTimer = window.setTimeout(() => {{
                            hostHealthAfterFrameTimer = null;
                            lastHostHealthAtMs = 0;
                            emitHostHealth();
                        }}, Math.max(650, terminalFrameLikeInstrumentationThrottleMs() + 80));
                    }}
                    return;
                }}
                sendTerminalEvent(payload);
            }} catch (_error) {{}}
        }};
        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
            window.__yggtermXtermHosts[hostId].emitHostHealth = emitHostHealth;
        }}
        const emitHostHealthThrottled = () => {{
            const now = Date.now();
            const frameLikeHot = recentFrameLikeWriteHot();
            const minHostHealthIntervalMs = terminalInputHot()
                ? 650
                : (frameLikeHot ? terminalFrameLikeInstrumentationThrottleMs() : 250);
            if (now - lastHostHealthAtMs >= minHostHealthIntervalMs) {{
                lastHostHealthAtMs = now;
                emitHostHealth();
                return;
            }}
            if (hostHealthFramePending) {{
                return;
            }}
            hostHealthFramePending = true;
            window.setTimeout(() => {{
                hostHealthFramePending = false;
                lastHostHealthAtMs = Date.now();
                emitHostHealth();
            }}, minHostHealthIntervalMs);
        }};
        term.onData((data) => {{
            // XTERM-BUG: clipboard-double-paste — any multi-char onData
            // burst that didn't originate from a known yggterm paste path
            // is recorded as `unknown` source. If it lands within 300 ms
            // of a known yggterm paste, the double-fire detector flags it
            // (that's the smoking-gun for xterm.js/WebKit re-pasting on
            // top of our paste). Single-char data = typing, not paste.
            try {{
                if (typeof data === 'string' && data.length >= 4) {{
                    const dt = lastPasteEventAtMs ? Date.now() - lastPasteEventAtMs : -1;
                    // A mouse-tracking TUI (CC, codex pagers) turns every click
                    // and wheel tick into an SGR mouse-report burst on onData
                    // (\x1b[<b;x;yM / m, 12-14 bytes) — input, not a paste.
                    // Before this guard a single click on a mouse-enabled
                    // session logged a bogus paste event (226/hour measured
                    // live, 2026-07-23).
                    const mouseReportBurst = /^(?:\\u001b\[<\d+;\d+;\d+[Mm])+$/.test(data);
                    // If we just emitted a paste event from our own path,
                    // skip the onData echo (term.paste internally feeds onData).
                    if (!mouseReportBurst
                        && !(dt >= 0 && dt < 60 && lastPasteEventTrigger === 'middle_click_yggterm')) {{
                        recordPasteEvent('unknown', 'on_data_burst', data.length, null);
                    }}
                }}
            }} catch (_e) {{}}
            if (terminalDataIsSuppressedProtocolResponse(data)) {{
                if (terminalProtocolResponseFallbackAllowed(data)) {{
                    recordSuppressedTerminalProtocolResponse('onData-fallback', data);
                    flushPendingTerminalInput('before_protocol_response_fallback');
                    sendTerminalEvent({{ kind: "input", data }});
                    return;
                }}
                recordSuppressedTerminalProtocolResponse('onData', data);
                return;
            }}
            const protocolBypass = terminalDataBypassesInputGate(data);
            if (!inputEnabled && !protocolBypass) {{
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].ignoredDataEventCount =
                        Number(window.__yggtermXtermHosts[hostId].ignoredDataEventCount || 0) + 1;
                }}
                return;
            }}
            dataEventCount += 1;
            if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                window.__yggtermXtermHosts[hostId].dataEventCount = dataEventCount;
                window.__yggtermXtermHosts[hostId].lastDataEventAtMs = Date.now();
                if (protocolBypass) {{
                    window.__yggtermXtermHosts[hostId].protocolDataEventCount =
                        Number(window.__yggtermXtermHosts[hostId].protocolDataEventCount || 0) + 1;
                }}
            }}
            if (protocolBypass) {{
                flushPendingTerminalInput('before_protocol');
                sendTerminalEvent({{ kind: "input", data }});
                return;
            }}
            markTerminalInputHot('data');
            // XTERM-BUG: scrollback-lost-on-gui-restart — real keystrokes cancel any
            // pending persisted-scroll restore so the restore doesn't pull viewport
            // back from the prompt where user is typing.
            if (pendingPersistedScrollRestore) {{
                pendingPersistedScrollRestore = null;
                pendingPersistedScrollRestoreDeadlineMs = 0;
                const entry = window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]
                    ? window.__yggtermXtermHosts[hostId] : null;
                if (entry) {{
                    entry.persistedScrollRestorePending = false;
                    entry.persistedScrollRestoreCancelledByInput = true;
                }}
            }}
            // XTERM-BUG: scroll-jump-on-input — telemetry. Capture viewport-before so
            // we can attribute jumps to the input path vs competing handlers.
            const _scrollJumpBeforeY = (term && term.buffer && term.buffer.active)
                ? Number(term.buffer.active.viewportY || 0) : -1;
            const _scrollJumpBeforeBaseY = (term && term.buffer && term.buffer.active)
                ? Number(term.buffer.active.baseY || 0) : -1;
            // XTERM-BUG: scroll-jump-on-input — when user is actively scrolled up
            // (UserScrollback intent AND visibly off-bottom by > 5 rows), a
            // keystroke must NOT yank the viewport to baseY. The keystroke still
            // goes to the PTY; the viewport stays where the user is reading.
            // This is the actual production fix that complements the
            // pre-existing telemetry.
            // A click / drag / wheel in a mouse-reporting TUI (codex, vim) arrives
            // HERE as onData mouse-report bytes — SGR `\x1b[<…M/m` or legacy
            // `\x1b[M…`. That is a viewport interaction (and usually the START of a
            // selection), NEVER "typing at the prompt", so it must not snap the
            // viewport to the live bottom. The old guard only skipped the snap when
            // the user was UserScrollback AND > 5 rows off-bottom — so scrolling a
            // *little* up on a WORKING codex to select a nearby word, then clicking/
            // dragging, force-followed to the bottom and (because force clears the
            // UserScrollback latch) re-yanked on every further action ("kicked to the
            // bottom three times"). A genuine keystroke still snaps. An active
            // selection is also never yanked (scroll_mode Selecting invariant).
            const _inputIsMouseReport =
                typeof data === 'string'
                && (data.indexOf('\x1b[<') === 0 || data.indexOf('\x1b[M') === 0);
            const _inputHasActiveSelection =
                Boolean(term && typeof term.hasSelection === 'function' && term.hasSelection());
            const _scrollJumpUserIsReadingScrollback =
                _inputIsMouseReport
                || _inputHasActiveSelection
                || (scrollbackIntent === 'UserScrollback'
                    && _scrollJumpBeforeY >= 0
                    && _scrollJumpBeforeBaseY >= 0
                    && (_scrollJumpBeforeBaseY - _scrollJumpBeforeY) > 5);
            if (!_scrollJumpUserIsReadingScrollback) {{
                setScrollbackIntent('PromptFollow', 'input');
                scrollbackLocked = false;
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].scrollbackLocked = scrollbackLocked;
                }}
                scrollLiveCursorIntoView(true, 'input');
            }} else {{
                // Record that we deliberately skipped the snap so probes can verify.
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    const entry = window.__yggtermXtermHosts[hostId];
                    entry.inputSnapSkippedCount = Number(entry.inputSnapSkippedCount || 0) + 1;
                    entry.lastInputSnapSkippedAtMs = Date.now();
                    entry.lastInputSnapSkippedDistanceRows = _scrollJumpBeforeBaseY - _scrollJumpBeforeY;
                }}
                sendTerminalEvent({{
                    kind: 'debug',
                    message: `input_snap_skipped host=${{hostId}} distance_rows=${{_scrollJumpBeforeBaseY - _scrollJumpBeforeY}} viewport_y=${{_scrollJumpBeforeY}} base_y=${{_scrollJumpBeforeBaseY}}`
                }});
            }}
            // XTERM-BUG: scroll-jump-on-input — emit a scroll_jump_after_input event
            // when viewport moved more than 1 row as a result of this keystroke.
            try {{
                const afterY = (term && term.buffer && term.buffer.active)
                    ? Number(term.buffer.active.viewportY || 0) : -1;
                const afterBaseY = (term && term.buffer && term.buffer.active)
                    ? Number(term.buffer.active.baseY || 0) : -1;
                if (_scrollJumpBeforeY >= 0 && afterY >= 0 && Math.abs(afterY - _scrollJumpBeforeY) > 1) {{
                    sendTerminalEvent({{
                        kind: 'debug',
                        message: `scroll_jump_after_input host=${{hostId}} before=${{_scrollJumpBeforeY}} after=${{afterY}} base_before=${{_scrollJumpBeforeBaseY}} base_after=${{afterBaseY}}`
                    }});
                }}
            }} catch (_e) {{}}
            queueTerminalInputData(data);
        }});
        window.__yggtermXtermCleanups[hostId] = () => {{
            try {{
                flushPendingTerminalInput('cleanup');
            }} catch (_error) {{}}
            try {{
                captureSessionXtermSnapshot('cleanup');
            }} catch (_error) {{}}
            try {{
                resizeObserver.disconnect();
            }} catch (_error) {{}}
            try {{
                renderDisposable.dispose();
            }} catch (_error) {{}}
            try {{
                if (writeParsedDisposable) {{
                    writeParsedDisposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                if (scrollDisposable) {{
                    scrollDisposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                if (selectionDisposable) {{
                    selectionDisposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                // CC-DRAG-STALL: never let a deferred selection sync fire
                // against a disposed terminal.
                cancelScheduledPrimarySelectionSync();
                primarySelectionSyncPending = false;
            }} catch (_error) {{}}
            try {{
                if (suppressedOsc4Disposable) {{
                    suppressedOsc4Disposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                if (suppressedOsc10Disposable) {{
                    suppressedOsc10Disposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                if (suppressedOsc11Disposable) {{
                    suppressedOsc11Disposable.dispose();
                }}
            }} catch (_error) {{}}
            try {{
                detachHostInteractions(host);
            }} catch (_error) {{}}
            try {{
                window.removeEventListener('yggterm-terminal-read-nudge', handleExternalReadNudge, false);
            }} catch (_error) {{}}
            try {{
                if (softwareCanvasLinkRevealTimer !== null) {{
                    window.clearTimeout(softwareCanvasLinkRevealTimer);
                    softwareCanvasLinkRevealTimer = null;
                }}
            }} catch (_error) {{}}
            try {{
                if (softwareCanvasInputLineOverlay && softwareCanvasInputLineOverlay.remove) {{
                    softwareCanvasInputLineOverlay.remove();
                }}
                softwareCanvasInputLineOverlay = null;
            }} catch (_error) {{}}
            try {{
                if (softwareCanvasCursorOverlay && softwareCanvasCursorOverlay.remove) {{
                    softwareCanvasCursorOverlay.remove();
                }}
                softwareCanvasCursorOverlay = null;
            }} catch (_error) {{}}
            try {{
                disposeXtermInputLineDecoration('cleanup');
            }} catch (_error) {{}}
            try {{
                window.clearInterval(inputDriftWatchdog);
            }} catch (_error) {{}}
            try {{
                window.clearInterval(screenRestorePersistTimer);
            }} catch (_error) {{}}
            try {{
                window.clearInterval(settleFollowWatchdog);
            }} catch (_error) {{}}
            try {{
                if (visiblePaintRecoveryTimer !== null) {{
                    window.clearTimeout(visiblePaintRecoveryTimer);
                    visiblePaintRecoveryTimer = null;
                }}
            }} catch (_error) {{}}
            try {{
                if (resizeNotifyTimer !== null) {{
                    window.clearTimeout(resizeNotifyTimer);
                    resizeNotifyTimer = null;
                }}
            }} catch (_error) {{}}
            try {{
                if (settledResizePaintTimer !== null) {{
                    window.clearTimeout(settledResizePaintTimer);
                    settledResizePaintTimer = null;
                }}
                if (settledResizeFollowupTimer !== null) {{
                    window.clearTimeout(settledResizeFollowupTimer);
                    settledResizeFollowupTimer = null;
                }}
                if (hostHealthAfterFrameTimer !== null) {{
                    window.clearTimeout(hostHealthAfterFrameTimer);
                    hostHealthAfterFrameTimer = null;
                }}
            }} catch (_error) {{}}
            try {{
                const helperTextarea = host.querySelector('.xterm-helper-textarea');
                if (helperTextarea) {{
                    helperTextarea.removeEventListener('focus', syncFocusClass, true);
                    helperTextarea.removeEventListener('blur', syncFocusClass, true);
                }}
            }} catch (_error) {{}}
            try {{
                term.dispose();
            }} catch (_error) {{}}
            try {{
                if (window.__yggtermXtermHosts) {{
                    delete window.__yggtermXtermHosts[hostId];
                }}
            }} catch (_error) {{}}
            try {{
                if (window.__yggtermXtermCleanups) {{
                    delete window.__yggtermXtermCleanups[hostId];
                }}
            }} catch (_error) {{}}
            try {{
                const runtimeStyle = document.getElementById(runtimeStyleId);
                if (runtimeStyle) {{
                    runtimeStyle.remove();
                }}
            }} catch (_error) {{}}
            host.innerHTML = "";
        }};
        emitResize();
        requestVisiblePaint();
        emitHostHealth();
        scheduleResizeNudges();
        {constructed_debug}
        sendTerminalEvent({{ kind: "ready" }});
        while (true) {{
            const message = await recvTerminalCommand();
            if (!message) {{
                continue;
            }}
            if (message.kind === "reset") {{
                hideLowPowerTuiOverlay();
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].terminalContentSource = 'empty';
                    window.__yggtermXtermHosts[hostId].terminalSourceMismatchReason = '';
                }}
                traceXtermScreenEvent("reset", {{ reason: "clear_to_empty" }});
                if (typeof term.reset === 'function') {{
                    term.reset();
                }} else {{
                    term.clear();
                }}
                const nextTheme = {{
                    background: message.background,
                    foreground: message.foreground,
                    cursor: message.cursor,
                    cursorAccent: message.cursor_text,
                    selectionBackground: message.selection,
                    black: message.black,
                    red: message.red,
                    green: message.green,
                    yellow: message.yellow,
                    blue: message.blue,
                    magenta: message.magenta,
                    cyan: message.cyan,
                    white: message.white,
                    brightBlack: message.bright_black,
                    brightRed: message.bright_red,
                    brightGreen: message.bright_green,
                    brightYellow: message.bright_yellow,
                    brightBlue: message.bright_blue,
                    brightMagenta: message.bright_magenta,
                    brightCyan: message.bright_cyan,
                    brightWhite: message.bright_white,
                }};
                host.style.setProperty('--yggterm-term-font-family', message.font_family);
                host.style.setProperty('--yggterm-term-font-weight', String(message.font_weight));
                host.style.setProperty('--yggterm-term-font-weight-bold', String(message.font_weight_bold));
                host.style.setProperty('--yggterm-term-line-height', String(message.line_height));
                host.style.setProperty('--yggterm-term-letter-spacing', '0px');
                host.style.setProperty('--yggterm-term-foreground', message.foreground);
                host.style.setProperty('--yggterm-term-dim-foreground', message.dim_foreground);
                host.style.setProperty('--yggterm-term-cursor', message.cursor);
                host.style.setProperty('--yggterm-term-cursor-muted', message.cursor_muted);
                host.style.setProperty('--yggterm-term-cursor-text', message.cursor_text);
                host.style.setProperty('--yggterm-term-cursor-block-text', message.cursor_text);
                host.style.setProperty('--yggterm-term-input-line-background', message.input_line_background);
                host.style.setProperty('--yggterm-term-input-line-border', message.input_line_border);
                host.style.setProperty('--yggterm-term-font-smoothing', {font_smoothing});
                host.style.setProperty('--yggterm-term-moz-font-smoothing', {moz_font_smoothing});
                try {{
                    term.options = {{
                        ...term.options,
                        cursorBlink: false,
                        cursorInactiveStyle: 'block',
                        cursorStyle: 'block',
                        fontFamily: message.font_family,
                        fontSize: message.font_size,
                        fontWeight: message.font_weight,
                        fontWeightBold: message.font_weight_bold,
                        lineHeight: message.line_height,
                        letterSpacing: 0,
                        minimumContrastRatio: message.minimum_contrast_ratio,
                        theme: nextTheme,
                    }};
                }} catch (_error) {{
                    term.options.fontFamily = message.font_family;
                    term.options.cursorBlink = false;
                    term.options.cursorInactiveStyle = 'block';
                    term.options.cursorStyle = 'block';
                    term.options.fontSize = message.font_size;
                    term.options.fontWeight = message.font_weight;
                    term.options.fontWeightBold = message.font_weight_bold;
                    term.options.lineHeight = message.line_height;
                    term.options.letterSpacing = 0;
                    term.options.minimumContrastRatio = message.minimum_contrast_ratio;
                    term.options.theme = nextTheme;
                }}
                // Screen-restore part (b): the term.reset() above wiped the
                // construct-time localStorage transcript restore. Re-apply it ONCE
                // here (after theme) so it becomes scrollback; the resumed CLI's
                // viewport-only clear (\x1b[H\x1b[J) then leaves it scrollable
                // above. Without this the sparse fresh-PTY replay = vacuum.
                if (pendingPostResetTranscript
                    && typeof pendingPostResetTranscript.text === 'string'
                    && pendingPostResetTranscript.text.trim()) {{
                    try {{
                        const wsReapply = term && term._core && typeof term._core.writeSync === "function"
                            ? term._core.writeSync.bind(term._core)
                            : (term && term._core && term._core._writeBuffer && typeof term._core._writeBuffer.writeSync === "function"
                                ? term._core._writeBuffer.writeSync.bind(term._core._writeBuffer) : null);
                        window.__yggtermArmOsc52Suppress(hostId, 400);
                        if (wsReapply) {{ wsReapply("\x1bc\x1b[H"); wsReapply(pendingPostResetTranscript.text); }}
                        else if (typeof term.write === "function") {{ term.write(`\x1bc\x1b[H${{pendingPostResetTranscript.text}}`); }}
                        if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                            const eReapply = window.__yggtermXtermHosts[hostId];
                            eReapply.terminalContentSource = 'localstorage_session_snapshot';
                            eReapply.terminalSourceMismatchReason = '';
                        }}
                        sendTerminalEvent({{
                            kind: 'debug',
                            message: `localstorage_transcript_reapplied_after_reset host=${{hostId}} lines=${{pendingPostResetTranscript.lineCount}} nonblank=${{pendingPostResetTranscript.nonblankLineCount}}`
                        }});
                    }} catch (_reapplyErr) {{}}
                    pendingPostResetTranscript = null;
                }}
                {reset_debug}
                requestAnimationFrame(() => {{
                    refreshCursorContrastContract();
                    emitResize();
                    requestVisiblePaint();
                }});
                scheduleResizeNudges();
            }} else if (message.kind === "refit") {{
                requestAnimationFrame(() => {{
                    emitResize();
                    requestVisiblePaint();
                    emitHostHealth();
                }});
                scheduleResizeNudges();
            }} else if (message.kind === "set_handover_paint_suspended") {{
                const nextSuspended = Boolean(message.suspended);
                if (nextSuspended !== handoverPaintSuspended) {{
                    handoverPaintSuspended = nextSuspended;
                    applyHandoverPaintVeil(nextSuspended);
                    if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                        window.__yggtermXtermHosts[hostId].handoverPaintSuspended = nextSuspended;
                    }}
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `handover_paint_${{nextSuspended ? 'suspended' : 'resumed'}} host=${{hostId}}`
                    }});
                    if (!nextSuspended) {{
                        // THE VEIL OWES A FULL REDRAW (user-reported 2026-08-01:
                        // "Claude Code ALWAYS starts with a broken bottom", plus
                        // glyph corruption on switch-in).
                        //
                        // While suspended this host did NO visible paint at all,
                        // for up to `suspend_ceiling_ms` (90 s). Every row the
                        // read loop wrote in that window landed in the buffer
                        // with its damage already consumed, so `requestVisiblePaint(false)`
                        // — which is what used to run here — repaints only what
                        // xterm still believes is dirty and presents LESS than
                        // the client holds. That is the broken bottom, and it is
                        // deterministic because the gate arms on every mount.
                        // The glyph half is the same line: the atlas heal lives
                        // inside the forced-refresh branch, so a non-forced
                        // resume also skips `clearTerminalTextureAtlas()` and
                        // paints switch-in cells against a stale WebGL atlas.
                        //
                        // `redrawTerminal` IS the repaint the user performs by
                        // hand to fix this ("a TUI refresh fixes it every time"):
                        // atlas clear + `term.refresh(0, rows-1)` over the
                        // CLIENT's own buffer. It is emphatically NOT a
                        // daemon-screen replay (field guide §5 — that collapses
                        // scrollback and is destructive), and it is NOT gated on
                        // output silence, because an agent CLI is never silent
                        // and this is not speculative correction: it is the
                        // settle of a window WE blanked.
                        redrawTerminal('handover-paint-resume');
                    }}
                }}
            }} else if (message.kind === "redraw") {{
                redrawTerminal(message.reason || 'command-redraw');
            }} else if (message.kind === "drop_unfocused_tui_frame") {{
                backgroundTuiSuppressActive = true;
                unfocusedTuiFrameDropCount += 1;
                unfocusedTuiLastTail = String(message.tail || '').slice(-240);
                if (!tracedUnfocusedTuiDrop) {{
                    tracedUnfocusedTuiDrop = true;
                    sendTerminalEvent({{
                        kind: "debug",
                        message: `background_tui_drop host=${{hostId}} frameLike=${{Boolean(message.frame_like)}} protocolOnly=${{Boolean(message.protocol_only)}} chars=${{Number(message.chars || 0)}} source=rust`
                    }});
                }}
                syncLowPowerTuiHostEntry();
            }} else if (message.kind === "write") {{
                const incomingWriteData = String(message.data || '');
                const inlineStatusAnimation = terminalPayloadLooksInlineStatusAnimation(incomingWriteData);
                const renderAnimation =
                    inlineStatusAnimation || terminalPayloadLooksHighVolumeFrame(incomingWriteData);
                const inlineStatusAnimationContinuation =
                    !renderAnimation
                    && recentInlineStatusAnimationHot()
                    && terminalPayloadLooksInlineStatusRewrite(incomingWriteData);
                if (renderAnimation || inlineStatusAnimationContinuation) {{
                    if (
                        recentInlineStatusAnimationStartedAtMs <= 0
                        || !recentInlineStatusAnimationHot()
                    ) {{
                        recentInlineStatusAnimationStartedAtMs = Date.now();
                    }}
                    recentInlineStatusAnimationUntilMs =
                        Date.now() + Math.max(900, terminalActiveAnimationWriteFrameMs * 12);
                    syncTerminalWriteFrameBudgetHostEntry();
                }} else if (!recentInlineStatusAnimationHot()) {{
                    recentInlineStatusAnimationStartedAtMs = 0;
                }}
                if (window.__yggtermXtermHosts && window.__yggtermXtermHosts[hostId]) {{
                    window.__yggtermXtermHosts[hostId].writeCommandCount =
                        Number(window.__yggtermXtermHosts[hostId].writeCommandCount || 0) + 1;
                    window.__yggtermXtermHosts[hostId].lastWriteSample =
                        incomingWriteData.slice(-200);
                    window.__yggtermXtermHosts[hostId].lastWriteError = '';
                    window.__yggtermXtermHosts[hostId].lastWriteQueuedAtMs = Date.now();
                    window.__yggtermXtermHosts[hostId].recentInlineStatusAnimationUntilMs =
                        recentInlineStatusAnimationUntilMs;
                    window.__yggtermXtermHosts[hostId].recentInlineStatusAnimationHot =
                        recentInlineStatusAnimationHot();
                    window.__yggtermXtermHosts[hostId].writeBridgePendingData =
                        String(window.__yggtermXtermHosts[hostId].writeBridgePendingData || '')
                        + incomingWriteData;
                    // Latency instrumentation: the write-bridge backlog is the
                    // accumulator suspected behind progressive input lag. Track a
                    // high-water-mark + the age of the oldest un-drained byte so
                    // the accumulation is visible in host-health (not just the
                    // instantaneous depth). See
                    // [[finding-xterm-latency-progressive-degradation]].
                    const __ygPendingLen = window.__yggtermXtermHosts[hostId].writeBridgePendingData.length;
                    window.__yggtermXtermHosts[hostId].writeBridgePendingMaxChars = Math.max(
                        Number(window.__yggtermXtermHosts[hostId].writeBridgePendingMaxChars || 0),
                        __ygPendingLen
                    );
                    if (__ygPendingLen > 0 && !Number(window.__yggtermXtermHosts[hostId].writeBridgePendingSinceMs || 0)) {{
                        window.__yggtermXtermHosts[hostId].writeBridgePendingSinceMs = Date.now();
                    }}
                    const __ygBacklogSince = Number(
                        window.__yggtermXtermHosts[hostId].writeBridgePendingSinceMs || 0
                    );
                    traceXtermEnqueue(
                        incomingWriteData.length,
                        __ygPendingLen,
                        __ygBacklogSince ? Math.max(0, Date.now() - __ygBacklogSince) : 0
                    );
                }}
                const forceLowLatencyWrite = terminalPayloadShouldFlushImmediately(incomingWriteData);
                schedulePendingWriteFlush(forceLowLatencyWrite);
            }} else if (message.kind === "set_input_enabled") {{
                setInputEnabled(Boolean(message.enabled), Boolean(message.focus), true, 'rust_policy');
                emitHostHealth();
            }}
        }}
        "#,
        trace_emitter_js = TRACE_EMITTER_JS,
        font_size = theme.font_size,
        background = background,
        foreground = foreground,
        cursor = cursor,
        selection = selection,
        black = black,
        red = red,
        green = green,
        yellow = yellow,
        blue = blue,
        magenta = magenta,
        cyan = cyan,
        white = white,
        bright_black = bright_black,
        bright_red = bright_red,
        bright_green = bright_green,
        bright_yellow = bright_yellow,
        bright_blue = bright_blue,
        bright_magenta = bright_magenta,
        bright_cyan = bright_cyan,
        bright_white = bright_white,
        initial_input_enabled = initial_input_enabled,
        constructed_debug = constructed_debug,
        reset_debug = reset_debug,
        font_family = font_family,
        font_weight = font_weight,
        font_weight_bold = font_weight_bold,
        line_height = line_height,
        dim_foreground = dim_foreground,
        cursor_muted = cursor_muted,
        input_line_background = input_line_background,
        input_line_border = input_line_border,
        input_line_decoration_enabled = input_line_decoration_enabled,
        minimum_contrast_ratio = minimum_contrast_ratio,
        cursor_text = cursor_text,
        terminal_active_write_frame_ms = terminal_active_write_frame_ms,
        terminal_active_animation_write_frame_ms = terminal_active_animation_write_frame_ms,
        terminal_active_animation_sustained_write_frame_ms =
            terminal_active_animation_sustained_write_frame_ms,
        terminal_active_animation_long_write_frame_ms =
            terminal_active_animation_long_write_frame_ms,
        terminal_inline_status_animation_sustained_after_ms =
            terminal_inline_status_animation_sustained_after_ms,
        terminal_inline_status_animation_long_after_ms =
            terminal_inline_status_animation_long_after_ms,
        terminal_input_hot_suppress_ms = TERMINAL_INPUT_HOT_SUPPRESS_MS
    )
}
fn terminal_apply_script(host_id: &str, theme: &TerminalTheme) -> String {
    let background =
        serde_json::to_string(&theme.background).expect("serialize terminal background");
    let foreground =
        serde_json::to_string(&theme.foreground).expect("serialize terminal foreground");
    let cursor = serde_json::to_string(&theme.cursor).expect("serialize terminal cursor");
    let selection = serde_json::to_string(&theme.selection).expect("serialize terminal selection");
    let black = serde_json::to_string(&theme.black).expect("serialize terminal black");
    let red = serde_json::to_string(&theme.red).expect("serialize terminal red");
    let green = serde_json::to_string(&theme.green).expect("serialize terminal green");
    let yellow = serde_json::to_string(&theme.yellow).expect("serialize terminal yellow");
    let blue = serde_json::to_string(&theme.blue).expect("serialize terminal blue");
    let magenta = serde_json::to_string(&theme.magenta).expect("serialize terminal magenta");
    let cyan = serde_json::to_string(&theme.cyan).expect("serialize terminal cyan");
    let white = serde_json::to_string(&theme.white).expect("serialize terminal white");
    let bright_black =
        serde_json::to_string(&theme.bright_black).expect("serialize terminal bright black");
    let bright_red =
        serde_json::to_string(&theme.bright_red).expect("serialize terminal bright red");
    let bright_green =
        serde_json::to_string(&theme.bright_green).expect("serialize terminal bright green");
    let bright_yellow =
        serde_json::to_string(&theme.bright_yellow).expect("serialize terminal bright yellow");
    let bright_blue =
        serde_json::to_string(&theme.bright_blue).expect("serialize terminal bright blue");
    let bright_magenta =
        serde_json::to_string(&theme.bright_magenta).expect("serialize terminal bright magenta");
    let bright_cyan =
        serde_json::to_string(&theme.bright_cyan).expect("serialize terminal bright cyan");
    let bright_white =
        serde_json::to_string(&theme.bright_white).expect("serialize terminal bright white");
    let font_family =
        serde_json::to_string(TERMINAL_FONT_FAMILY).expect("serialize terminal font family");
    let font_weight = serde_json::to_string(&terminal_font_weight(theme))
        .expect("serialize terminal font weight");
    let font_weight_bold = serde_json::to_string(&terminal_font_weight_bold(theme))
        .expect("serialize terminal bold font weight");
    let line_height = terminal_font_line_height(theme);
    let dim_foreground = serde_json::to_string(&terminal_dim_foreground(theme))
        .expect("serialize terminal dim foreground");
    let cursor_muted = serde_json::to_string(&terminal_cursor_muted(theme))
        .expect("serialize terminal muted cursor");
    let cursor_text = serde_json::to_string(&terminal_cursor_text(theme))
        .expect("serialize terminal cursor text");
    let input_line_background = serde_json::to_string(&terminal_input_line_background(theme))
        .expect("serialize terminal input line background");
    let input_line_border = serde_json::to_string(&terminal_input_line_border(theme))
        .expect("serialize terminal input line border");
    let minimum_contrast_ratio = terminal_minimum_contrast_ratio(theme);
    let font_smoothing = serde_json::to_string(terminal_font_smoothing(theme))
        .expect("serialize terminal font smoothing");
    let moz_font_smoothing = serde_json::to_string(terminal_moz_font_smoothing(theme))
        .expect("serialize terminal moz font smoothing");
    format!(
        r#"
        (() => {{
          const hostId = {host_id:?};
          const registry = window.__yggtermXtermHosts || {{}};
          const entry = registry[hostId];
          if (!entry || !entry.term) {{
            return;
          }}
          const nextTheme = {{
            background: {background},
            foreground: {foreground},
            cursor: {cursor},
            cursorAccent: {cursor_text},
            selectionBackground: {selection},
            black: {black},
            red: {red},
            green: {green},
            yellow: {yellow},
            blue: {blue},
            magenta: {magenta},
            cyan: {cyan},
            white: {white},
            brightBlack: {bright_black},
            brightRed: {bright_red},
            brightGreen: {bright_green},
            brightYellow: {bright_yellow},
            brightBlue: {bright_blue},
            brightMagenta: {bright_magenta},
            brightCyan: {bright_cyan},
            brightWhite: {bright_white},
          }};
          try {{
            entry.term.options = {{
              ...entry.term.options,
              cursorBlink: false,
              cursorInactiveStyle: 'block',
              cursorStyle: 'block',
              fontFamily: {font_family},
              fontSize: {font_size},
              fontWeight: {font_weight},
              fontWeightBold: {font_weight_bold},
              lineHeight: {line_height},
              letterSpacing: 0,
              minimumContrastRatio: {minimum_contrast_ratio},
              theme: nextTheme,
            }};
          }} catch (_error) {{
            entry.term.options.fontFamily = {font_family};
            entry.term.options.cursorBlink = false;
            entry.term.options.cursorInactiveStyle = 'block';
            entry.term.options.cursorStyle = 'block';
            entry.term.options.fontSize = {font_size};
            entry.term.options.fontWeight = {font_weight};
            entry.term.options.fontWeightBold = {font_weight_bold};
            entry.term.options.lineHeight = {line_height};
            entry.term.options.letterSpacing = 0;
            entry.term.options.minimumContrastRatio = {minimum_contrast_ratio};
            entry.term.options.theme = nextTheme;
          }}
          if (entry.host) {{
            entry.host.style.setProperty('--yggterm-term-font-family', {font_family});
            entry.host.style.setProperty('--yggterm-term-font-weight', String({font_weight}));
            entry.host.style.setProperty('--yggterm-term-font-weight-bold', String({font_weight_bold}));
            entry.host.style.setProperty('--yggterm-term-line-height', String({line_height}));
            entry.host.style.setProperty('--yggterm-term-letter-spacing', '0px');
            entry.host.style.setProperty('--yggterm-term-foreground', {foreground});
            entry.host.style.setProperty('--yggterm-term-dim-foreground', {dim_foreground});
            entry.host.style.setProperty('--yggterm-term-cursor', {cursor});
            entry.host.style.setProperty('--yggterm-term-cursor-muted', {cursor_muted});
            entry.host.style.setProperty('--yggterm-term-cursor-text', {cursor_text});
            entry.host.style.setProperty('--yggterm-term-cursor-block-text', {cursor_text});
            entry.host.style.setProperty('--yggterm-term-input-line-background', {input_line_background});
            entry.host.style.setProperty('--yggterm-term-input-line-border', {input_line_border});
            entry.host.style.setProperty('--yggterm-term-font-smoothing', {font_smoothing});
            entry.host.style.setProperty('--yggterm-term-moz-font-smoothing', {moz_font_smoothing});
            entry.host.style.webkitFontSmoothing = {font_smoothing};
            entry.host.style.MozOsxFontSmoothing = {moz_font_smoothing};
          }}
          try {{
            if (entry.refreshCursorContrastContract) {{
              entry.refreshCursorContrastContract();
            }}
          }} catch (_error) {{}}
          try {{
            if (entry.emitResize) {{
              entry.emitResize();
            }} else if (entry.fitAddon) {{
              entry.fitAddon.fit();
            }}
          }} catch (_error) {{}}
          try {{
            if (typeof entry.term.clearTextureAtlas === 'function') {{
              entry.term.clearTextureAtlas();
              entry.lastAtlasClearAtMs = Date.now();
            }}
          }} catch (_error) {{}}
          try {{
            if (entry.term.refresh) {{
              entry.term.refresh(0, Math.max(0, entry.term.rows - 1));
            }}
          }} catch (_error) {{}}
          window.__yggtermLastApply = {{
            hostId,
            fontSize: entry.term.options.fontSize,
            appliedAt: Date.now(),
          }};
        }})();
        "#,
        host_id = host_id,
        font_size = theme.font_size,
        background = background,
        foreground = foreground,
        cursor = cursor,
        selection = selection,
        black = black,
        red = red,
        green = green,
        yellow = yellow,
        blue = blue,
        magenta = magenta,
        cyan = cyan,
        white = white,
        bright_black = bright_black,
        bright_red = bright_red,
        bright_green = bright_green,
        bright_yellow = bright_yellow,
        bright_blue = bright_blue,
        bright_magenta = bright_magenta,
        bright_cyan = bright_cyan,
        bright_white = bright_white,
        font_family = font_family,
        font_weight = font_weight,
        font_weight_bold = font_weight_bold,
        line_height = line_height,
        dim_foreground = dim_foreground,
        cursor_muted = cursor_muted,
        input_line_background = input_line_background,
        input_line_border = input_line_border,
        minimum_contrast_ratio = minimum_contrast_ratio,
        cursor_text = cursor_text,
        font_smoothing = font_smoothing,
        moz_font_smoothing = moz_font_smoothing,
    )
}
/// §7.3 stable-epoch reveal nudge: repaint a retained closure whose reveal
/// skipped the bootstrap (pinned activation epoch). `emitResize` re-fits the
/// grid after any parked-geometry drift; `redrawTerminal` rebuilds the canvas
/// content in case the parked backing store was dropped. Both are the retained
/// closure's OWN functions from the host registry — the same dispatch a live
/// closure uses — so a superseded (stood-down) closure correctly refuses.
fn terminal_stable_epoch_reveal_nudge_script(host_id: &str) -> String {
    format!(
        r#"
        (() => {{
          const hostId = {host_id:?};
          const registry = window.__yggtermXtermHosts || {{}};
          const entry = registry[hostId];
          if (!entry) {{
            return;
          }}
          try {{
            if (typeof entry.emitResize === "function") {{
              entry.emitResize();
            }}
          }} catch (_error) {{}}
          try {{
            if (typeof entry.redrawTerminal === "function") {{
              entry.redrawTerminal('stable_epoch_reveal');
            }}
          }} catch (_error) {{}}
        }})();
        "#,
        host_id = host_id,
    )
}
/// Selectors for content that owns its OWN (native WebKit) context menu.
///
/// ONE list, like `UI_FOCUS_OWNER_SELECTORS`. A right-click inside any of these
/// is the engine's to handle — it renders real DOM/page content and its native
/// menu (Copy / Cut / Paste / Select All) is the correct menu there. Everything
/// else is yggterm chrome, where the platform menu is noise and stays suppressed.
const NATIVE_CONTEXT_MENU_OWNER_SELECTORS: &str =
    "[data-document-surface], [data-ws-overlay], [data-yggterm-web-picker], [data-document-editor]";

/// Chrome that COVERS the terminal host and owns the right-click landing on it.
///
/// The sibling list to [`NATIVE_CONTEXT_MENU_OWNER_SELECTORS`], and a different
/// question: that one asks "whose menu is this — the engine's or ours?", this
/// one asks "whose menu is this — the terminal's or some other piece of OUR
/// chrome's?". The terminal claims a secondary click by pure GEOMETRY
/// (`pointerEventFallsWithinHost`), during the DOCUMENT CAPTURE phase, with
/// `stopImmediatePropagation()` — so anything drawn over the host rect that is
/// not in this list has its right-click eaten before the target phase ever
/// runs, and its own `oncontextmenu` is unreachable code.
///
/// ★ AN AUTO-HIDDEN SIDEBAR IS ALWAYS SUCH A COVER. Hidden, a panel leaves the
/// flow (`sidebar_panel_outer_style` ⇒ `position:absolute`) precisely so a
/// hover-reveal never re-fits the xterm — which means the terminal host keeps
/// the FULL window width and the revealed floating card sits geometrically
/// INSIDE it. Docked, the same panel is in flow and the host rect starts after
/// it, so geometry alone happened to give the right answer and hid the defect:
/// right-clicking a row on a hover-revealed panel opened no row menu at all
/// (user report; root-caused 2026-08-01). Named by PANEL, never by side — the
/// mirror (`ChromeOrientation`) swaps which edge each one is on.
const TERMINAL_SECONDARY_COVER_SELECTORS: &str = "[data-document-surface], [data-ws-overlay], \
     [data-yggterm-web-picker], [data-yggterm-menu-backdrop], [data-context-menu], \
     #yggterm-sidebar, [data-yggui-side-rail]";

/// App-wide right-click policy: suppress the platform menu over yggterm chrome,
/// ALLOW it over content that owns its own menu.
///
/// This replaced a blanket `evt.prevent_default()` on the shell root, which
/// killed the native menu everywhere — the reason yedit had no right-click copy.
/// It must be JS: the decision needs the event's target element, and Rust's only
/// route to the DOM (`document::eval`) is async, so a Rust handler cannot decide
/// in time to not-cancel the event.
///
/// Capture phase at the document, installed once and idempotent.
fn context_menu_policy_script() -> String {
    format!(
        r#"
(() => {{
  if (window.__yggtermContextMenuPolicy) return;
  const OWNERS = {NATIVE_CONTEXT_MENU_OWNER_SELECTORS:?};
  const ownsNativeMenu = (target) => {{
    try {{
      return !!(target && target.closest && target.closest(OWNERS));
    }} catch (_e) {{ return false; }}
  }};
  const policy = (event) => {{
    try {{
      if (ownsNativeMenu(event.target)) return; // the engine's menu wins here
      event.preventDefault();
    }} catch (_e) {{}}
  }};
  document.addEventListener("contextmenu", policy, false);
  window.__yggtermContextMenuPolicy = policy;
}})();
"#
    )
}


/// Kate-style wrap-aware line-number gutter for the document editor.
///
/// A textarea does not expose where its soft-wrap breaks each logical line, so a
/// static `1\n2\n3` gutter desyncs the moment a line wraps — which is why the
/// gutter used to be suppressed in wrap mode. This installs a JS maintainer that
/// measures each logical line in a hidden mirror div (same content width, font,
/// padding and wrap rules as the textarea, so it wraps identically) and emits
/// **one gutter block per LOGICAL line, at that line's measured height**: the
/// line number on the block's first row, a continuation arrow (↪) on each
/// wrapped row — exactly like KDE Kate. The gutter's inner block is translated
/// by the textarea's scrollTop so it tracks the text as it scrolls.
///
/// ## Why one block per LOGICAL line, and not one per visual row
///
/// The first version took a single fractional `getComputedStyle(…).lineHeight`
/// and used it for BOTH the height of every entry AND the row count
/// (`round(offsetHeight / lineHeight)`). Every entry then carried the same
/// rounding error, and because the entries stack, that error ACCUMULATED: at
/// 13.5px/1.55 the line box is 20.925px, so a gutter drawn at a rounded 21px
/// has slipped a whole row by line 300. Far enough down a file, the number
/// beside a line was simply not that line's number.
///
/// The fix is to stop deriving geometry from a number and to carry the
/// MEASUREMENT instead. Each block is exactly as tall as the mirror says its
/// logical line is, so consecutive blocks cannot drift; the per-row division
/// survives only INSIDE a block, where the worst case is one stray arrow on one
/// line and nothing below it moves. Heights come from
/// `getBoundingClientRect().height`, not `offsetHeight`, because `offsetHeight`
/// is rounded to whole pixels — per-line rounding is this same bug in miniature.
///
/// ## The gutter checks itself
///
/// A silently-wrong gutter is worse than an absent one, so the sum of the block
/// heights is compared against the textarea's own content height (`scrollHeight`
/// minus its vertical padding). On a mismatch the numbers are NOT drawn and
/// `data-document-wrap-gutter-status="drift"` is stamped on the gutter, with the
/// arithmetic in `data-document-wrap-gutter-detail` — which `server app state`
/// reports as `document_wrap_gutters`.
///
/// Idempotent and self-reinstalling: the mount hook re-runs `syncAll()`, hooks
/// each textarea once (input/scroll/resize), and rebuilds on every change.
const DOCUMENT_WRAP_GUTTER_SCRIPT: &str = r#"
(() => {
  const ARROW = "↪";
  // Beyond this the per-line measure is skipped (it is O(lines) of layout) and
  // every logical line is ASSUMED to be one visual row. That assumption is not
  // trusted: the self-check below compares the total against the textarea and
  // drops the numbers the moment any line actually wrapped.
  const MAX_MEASURED_LINES = 6000;
  // `scrollHeight` is an integer rounded from a fractional layout, so exact
  // equality with a fractional sum is not on offer. One pixel of slack sits two
  // orders of magnitude below what is being guarded against — a whole line box.
  const SUM_TOLERANCE_PX = 1.5;
  function px(value) {
    const parsed = parseFloat(value);
    return isFinite(parsed) ? parsed : 0;
  }
  function lineHeightPx(cs) {
    const lh = parseFloat(cs.lineHeight);
    if (isFinite(lh) && lh > 0) return lh;
    const fs = parseFloat(cs.fontSize);
    return (isFinite(fs) && fs > 0) ? fs * 1.2 : 18;
  }
  function round2(value) {
    return Math.round(value * 100) / 100;
  }
  function mirrorFor(ta) {
    let m = ta.__yggMirror;
    if (m && m.isConnected) return m;
    m = document.createElement("div");
    m.setAttribute("aria-hidden", "true");
    m.style.cssText = "position:absolute; top:0; left:-99999px; visibility:hidden; pointer-events:none; margin:0; border:0; padding:0; box-sizing:content-box;";
    document.body.appendChild(m);
    ta.__yggMirror = m;
    return m;
  }
  // The height of EVERY logical line, measured where the line actually wraps.
  function measureLineHeights(ta, cs) {
    const contentWidth = ta.clientWidth - px(cs.paddingLeft) - px(cs.paddingRight);
    if (!(contentWidth > 0)) return null;
    const lines = ta.value.split("\n");
    if (lines.length > MAX_MEASURED_LINES) {
      const lh = lineHeightPx(cs);
      return { heights: lines.map(() => lh), measured: false };
    }
    const m = mirrorFor(ta);
    m.style.width = contentWidth + "px";
    m.style.fontFamily = cs.fontFamily;
    m.style.fontSize = cs.fontSize;
    m.style.fontWeight = cs.fontWeight;
    m.style.fontStyle = cs.fontStyle;
    m.style.lineHeight = cs.lineHeight;
    m.style.letterSpacing = cs.letterSpacing;
    m.style.wordSpacing = cs.wordSpacing;
    m.style.textIndent = cs.textIndent;
    m.style.tabSize = cs.tabSize;
    m.style.whiteSpace = "pre-wrap";
    m.style.overflowWrap = "anywhere";
    m.style.wordBreak = cs.wordBreak;
    m.textContent = "";
    const kids = [];
    for (const line of lines) {
      const d = document.createElement("div");
      d.style.whiteSpace = "pre-wrap";
      d.style.overflowWrap = "anywhere";
      d.style.margin = "0";
      d.style.padding = "0";
      d.style.border = "0";
      // an empty logical line still occupies one visual row
      d.textContent = line.length ? line : " ";
      m.appendChild(d);
      kids.push(d);
    }
    // getBoundingClientRect, NOT offsetHeight: the latter rounds to whole pixels
    // and a per-line rounding error is exactly the drift being fixed here.
    return { heights: kids.map((d) => d.getBoundingClientRect().height), measured: true };
  }
  function gutterFor(ta) {
    const parent = ta.parentElement;
    if (!parent) return null;
    return parent.querySelector("[data-document-wrap-gutter]");
  }
  function stamp(gutter, status, detail) {
    gutter.setAttribute("data-document-wrap-gutter-status", status);
    try {
      gutter.setAttribute("data-document-wrap-gutter-detail", JSON.stringify(detail));
    } catch (_e) {}
  }
  // Does the measured total agree with the textarea's own layout?
  //   "ok"         - it does, and the textarea was scrolling, so it could say.
  //   "unverified" - the content fits the box, so scrollHeight is clamped to the
  //                  padding box and cannot confirm a total. Nothing contradicts
  //                  the measurement, so numbers are drawn; the field says the
  //                  check did not run rather than implying that it did.
  //   "drift"      - they disagree. Numbers are withheld.
  function verify(ta, cs, sum) {
    const padTop = px(cs.paddingTop);
    const padBottom = px(cs.paddingBottom);
    const scrollHeight = ta.scrollHeight;
    const clientHeight = ta.clientHeight;
    const delta = (sum + padTop + padBottom) - scrollHeight;
    let status;
    if (scrollHeight > clientHeight) {
      status = Math.abs(delta) <= SUM_TOLERANCE_PX ? "ok" : "drift";
    } else if (delta > SUM_TOLERANCE_PX) {
      // Claiming more content than a box that is NOT scrolling can hold.
      status = "drift";
    } else {
      status = "unverified";
    }
    return {
      status,
      detail: {
        sum: round2(sum),
        scroll_height: scrollHeight,
        client_height: clientHeight,
        padding: round2(padTop + padBottom),
        delta: round2(delta),
      },
    };
  }
  function rebuild(ta) {
    const gutter = gutterFor(ta);
    if (!gutter) return;
    // An invisible editor has no layout to measure — its mirror reads 0 width.
    if (ta.offsetParent === null && ta.clientWidth === 0) return;
    const cs = getComputedStyle(ta);
    // Skip when nothing that affects wrapping changed (value, width or metrics).
    // The body MutationObserver fires on unrelated churn — terminal streaming —
    // so this guard is what keeps that from re-measuring a large doc every tick.
    const sig = ta.value.length + ":" + ta.value + ":" + ta.clientWidth + ":" + cs.fontSize + ":" + cs.lineHeight;
    if (sig === ta.__yggGutterSig) return;
    const measurement = measureLineHeights(ta, cs);
    if (!measurement) return;
    ta.__yggGutterSig = sig;
    const heights = measurement.heights;
    let inner = gutter.firstElementChild;
    if (!inner) { inner = document.createElement("div"); gutter.appendChild(inner); }
    let sum = 0;
    for (let i = 0; i < heights.length; i++) sum += heights[i];
    const checked = verify(ta, cs, sum);
    checked.detail.lines = heights.length;
    checked.detail.measured = measurement.measured;
    stamp(gutter, checked.status, checked.detail);
    if (checked.status === "drift") {
      // A silently-wrong gutter is worse than an absent one.
      inner.innerHTML = "";
      inner.style.transform = "translateY(0px)";
      return;
    }
    const lh = lineHeightPx(cs);
    let html = "";
    for (let i = 0; i < heights.length; i++) {
      const height = heights[i];
      // The BLOCK owns this logical line's exact height, so nothing below it can
      // shift. The row count inside it is cosmetic — how many arrows the line
      // gets — and a rounding error there cannot escape this block.
      const rows = Math.max(1, Math.round(height / lh));
      html += '<div style="height:' + height + 'px;overflow:hidden;">';
      html += '<div style="line-height:' + lh + 'px;">' + (i + 1) + "</div>";
      for (let k = 1; k < rows; k++) {
        html += '<div style="line-height:' + lh + 'px;opacity:0.45;">' + ARROW + "</div>";
      }
      html += "</div>";
    }
    inner.innerHTML = html;
    inner.style.transform = "translateY(" + (-ta.scrollTop) + "px)";
  }
  function syncScroll(ta) {
    const gutter = gutterFor(ta);
    const inner = gutter && gutter.firstElementChild;
    if (inner) inner.style.transform = "translateY(" + (-ta.scrollTop) + "px)";
  }
  window.__yggtermDocGutter = {
    syncAll() {
      const editors = document.querySelectorAll("textarea[data-document-wrap-editor]");
      editors.forEach((ta) => {
        if (!ta.__yggGutterHooked) {
          ta.__yggGutterHooked = true;
          ta.addEventListener("input", () => rebuild(ta));
          ta.addEventListener("scroll", () => syncScroll(ta));
          try {
            const ro = new ResizeObserver(() => rebuild(ta));
            ro.observe(ta);
          } catch (_e) {}
        }
        rebuild(ta);
      });
    }
  };
  window.__yggtermDocGutter.syncAll();
  // Dioxus re-renders the editor subtree on a wrap toggle WITHOUT necessarily
  // re-firing onmounted, so a body observer re-installs the hooks. Debounced and
  // guarded by the value/width signature in rebuild(), so unrelated churn (a
  // streaming terminal repainting) costs one timer, not a re-measure.
  if (!window.__yggtermDocGutterObserver) {
    let pending = 0;
    const kick = () => {
      if (pending) return;
      pending = setTimeout(() => {
        pending = 0;
        try { window.__yggtermDocGutter.syncAll(); } catch (_e) {}
      }, 120);
    };
    try {
      const obs = new MutationObserver(kick);
      obs.observe(document.body, { childList: true, subtree: true });
      window.__yggtermDocGutterObserver = obs;
    } catch (_e) {}
  }
})();
"#;


#[cfg(test)]
mod yedit_gutter_and_surface_switch_tests {
    use super::*;

    /// The PRODUCT half of this file — test modules stripped, so a scan cannot
    /// be satisfied by the needle its own assertion spells.
    fn product_source() -> String {
        let source = SHELL_SOURCE;
        let product = yggterm_core::agent_cli::product_lines(&source)
            .into_iter()
            .map(|(_, line)| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !product.contains("mod yedit_gutter_and_surface_switch_tests"),
            "the scan is reading this test module"
        );
        product
    }

    // ─────────────────────────────────────────────────────────────────────
    // THE GUTTER
    //
    // The maintainer is JS running in the webview, so these lock the SHAPE of
    // the arithmetic. What they cannot do is prove a pixel: the live proof is a
    // long wrapped document in yedit, plus `document_wrap_gutters[].status`,
    // which is exactly why the self-check exists.
    // ─────────────────────────────────────────────────────────────────────

    /// THE DEFECT: one fractional `lineHeight` sized every entry AND counted the
    /// rows, so each entry carried the same rounding error and the errors
    /// stacked — a whole row lost by line 300. Geometry must come from the
    /// measurement, one block per LOGICAL line.
    #[test]
    fn the_wrap_gutter_sizes_each_block_from_the_measured_line_not_a_line_height() {
        let script = DOCUMENT_WRAP_GUTTER_SCRIPT;
        assert!(
            script.contains("kids.map((d) => d.getBoundingClientRect().height)"),
            "line heights must be MEASURED; offsetHeight rounds to whole pixels and \
             per-line rounding is the drift being fixed"
        );
        assert!(
            !script.contains(".offsetHeight"),
            "offsetHeight is integer-rounded — it cannot carry a fractional line box"
        );
        assert!(
            script.contains(r#"html += '<div style="height:' + height + 'px;overflow:hidden;">';"#),
            "one block per logical line, at that line's measured height"
        );
        assert!(
            !script.contains("'px;line-height:' + lh"),
            "an entry sized by the computed line-height is the accumulating bug"
        );
        assert!(
            !script.contains("Math.round(d.offsetHeight / lh)"),
            "the row count must never decide an entry's HEIGHT"
        );
    }

    /// The row count survives only INSIDE a block, where the worst case is a
    /// stray arrow on one line and nothing below it moves.
    #[test]
    fn the_wrap_gutter_divides_only_inside_a_block() {
        let script = DOCUMENT_WRAP_GUTTER_SCRIPT;
        let block_open = script
            .find(r#"html += '<div style="height:' + height + 'px;overflow:hidden;">';"#)
            .expect("the per-line block");
        let division = script
            .find("Math.max(1, Math.round(height / lh))")
            .expect("the cosmetic row count");
        assert!(
            division < block_open,
            "the division decides how many ARROWS a block holds, never where the \
             next block starts"
        );
    }

    /// A silently-wrong gutter is worse than an absent one.
    #[test]
    fn the_wrap_gutter_withholds_numbers_it_cannot_verify() {
        let script = DOCUMENT_WRAP_GUTTER_SCRIPT;
        assert!(
            script.contains("const delta = (sum + padTop + padBottom) - scrollHeight;"),
            "the sum of the blocks must be checked against the textarea's own \
             content height"
        );
        let drift = script
            .find(r#"if (checked.status === "drift") {"#)
            .expect("the drift branch");
        let tail = &script[drift..];
        assert!(
            tail.starts_with(
                &format!(
                    "{}\n      // A silently-wrong gutter is worse than an absent one.\n      inner.innerHTML = \"\";",
                    r#"if (checked.status === "drift") {"#
                )
            ),
            "on a mismatch the numbers come OFF; drawing wrong ones is the failure \
             this check exists to prevent"
        );
        assert!(
            script.contains(r#"gutter.setAttribute("data-document-wrap-gutter-status", status);"#),
            "the verdict must be observable, not merely visible"
        );
    }

    /// Three verdicts, and `unverified` is not `ok`: a document that fits its box
    /// clamps `scrollHeight` to the padding box, so the check genuinely did not
    /// run and must not claim it did.
    #[test]
    fn the_wrap_gutter_names_the_case_where_it_could_not_check() {
        let script = DOCUMENT_WRAP_GUTTER_SCRIPT;
        for verdict in [r#""ok""#, r#""unverified""#, r#""drift""#] {
            assert!(
                script.contains(verdict),
                "the self-check must be able to report {verdict}"
            );
        }
        assert!(
            script.contains("if (scrollHeight > clientHeight) {"),
            "the strict comparison is only available while the editor is scrolling"
        );
    }

    /// The verdict has to reach an agent, or it is a comment.
    #[test]
    fn app_control_reports_the_wrap_gutters_verdict() {
        let product = product_source();
        assert!(
            product.contains("document_wrap_gutters: documentWrapGutters,"),
            "the app-state DOM probe must carry the gutter's self-check"
        );
        let observe = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/terminal_observe.rs"),
        )
        .expect("read terminal_observe.rs");
        assert!(
            observe.contains(r#""document_wrap_gutters": document_wrap_gutters,"#),
            "app-control must republish it — a field the report drops is not observable"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // THE SURFACE SWITCH
    // ─────────────────────────────────────────────────────────────────────

    fn document_session(hidden: bool) -> RenderSnapshot {
        let session = "local://yedit";
        let mut shell = ShellState::new(super::tests::test_shell_bootstrap_with_active_session(
            session,
        ));
        shell.upsert_sidebar_contribution(
            session,
            vec![SidebarPaneDeclaration {
                id: "editor".to_string(),
                icon: "✎".to_string(),
                title: "Editor".to_string(),
                placement: PanePlacement::Viewport,
            }],
            None,
            Some("yedit".to_string()),
            None,
            None,
            None,
            None,
            None,
            1_000,
            Some((
                "http://127.0.0.1:1".to_string(),
                "http://127.0.0.1:1".to_string(),
                None,
            )),
        );
        if hidden {
            shell.document_surface_hidden.insert(session.to_string());
        }
        shell.snapshot()
    }

    /// THE DEFECT: the document surface grew its own Document|Terminal pill
    /// because the titlebar slot only ever answered for agent CLIs. Two pills,
    /// both `position:absolute`, over viewports that reserve no space for
    /// chrome — so one of them drew over the first line of the document.
    #[test]
    fn a_declared_document_surface_owns_the_titlebar_switch() {
        assert_eq!(
            titlebar_surface_switch(&document_session(false)),
            TitlebarSurfaceSwitch::Document {
                document_visible: true,
                custom: None,
            }
        );
        assert_eq!(
            titlebar_surface_switch(&document_session(true)),
            TitlebarSurfaceSwitch::Document {
                document_visible: false,
                custom: None,
            },
            "hiding the surface must keep the SAME switch, with Terminal active — \
             never degrade it to a lone button"
        );
    }

    /// A yedit session is a SHELL hosting an app. `SessionKind` declines to
    /// answer whether it has a second surface; the declared pane does.
    #[test]
    fn a_session_with_no_declared_pane_falls_back_to_the_rendered_toggle() {
        let shell = ShellState::new(super::tests::test_shell_bootstrap_with_active_session(
            "local://codex",
        ));
        let snapshot = shell.snapshot();
        assert!(
            matches!(
                titlebar_surface_switch(&snapshot),
                TitlebarSurfaceSwitch::Rendered | TitlebarSurfaceSwitch::None
            ),
            "without a declared viewport pane the slot must not claim a document"
        );
    }

    /// ⚠ THE DIOXUS STYLE-KEY TRAP: the slot's hidden arm sets `visibility` and
    /// `pointer-events`, so the shown arm must set them back. An empty arm
    /// leaves the first hidden render latched forever.
    #[test]
    fn the_surface_switch_slot_clears_the_keys_it_sets() {
        let product = product_source();
        assert!(
            product.contains(" visibility:hidden; pointer-events:none;"),
            "the inert arm"
        );
        assert!(
            product.contains(" visibility:visible; pointer-events:auto;"),
            "the live arm must NAME both keys; Dioxus never clears a key a later \
             render stops emitting"
        );
    }

    /// One switch, one home.
    #[test]
    fn the_document_switch_does_not_float_over_a_viewport() {
        let product = product_source();
        assert!(
            !product.contains("data-document-terminal-toggle"),
            "the pill that floated over the document editor is gone"
        );
        assert!(
            !product.contains("data-document-show-toggle"),
            "the pill that floated over the terminal is gone"
        );
        assert_eq!(
            product.matches(r"\u{fe0e} Document").count(),
            1,
            "exactly ONE Document segment exists, and it is the titlebar's"
        );
        assert_eq!(
            product
                .matches(r#""data-titlebar-surface-switch-segment": "document","#)
                .count(),
            1,
            "and it lives in the titlebar's surface-switch slot"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // THE TOASTS
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn toasts_sit_top_centre_over_a_terminal() {
        let shell = ShellState::new(super::tests::test_shell_bootstrap_with_active_session(
            "local://plain",
        ));
        assert_eq!(toast_anchor(&shell.snapshot()), ToastAnchor::TopCenter);
    }

    /// Over a document the top of the viewport is the title and the first line
    /// being read, so the stack moves to the bottom corner — on the RAIL's edge,
    /// because the chrome mirror moves everything directional with its edge.
    #[test]
    fn toasts_move_to_the_rails_bottom_corner_over_a_document() {
        let mut snapshot = document_session(false);
        snapshot.settings.chrome_orientation = ChromeOrientation::natural();
        assert_eq!(toast_anchor(&snapshot), ToastAnchor::BottomRight);
        snapshot.settings.chrome_orientation = ChromeOrientation::mirrored();
        assert_eq!(
            toast_anchor(&snapshot),
            ToastAnchor::BottomLeft,
            "a mirrored rail takes the toast corner with it"
        );
    }

    /// Hiding the surface hands the viewport back to the terminal, and the
    /// toasts go back with it.
    #[test]
    fn hiding_the_document_returns_the_toasts_to_the_top() {
        assert_eq!(toast_anchor(&document_session(true)), ToastAnchor::TopCenter);
    }
}

fn terminal_set_input_enabled_script(host_id: &str, enabled: bool, focus: bool) -> String {
    format!(
        r#"
        (() => {{
          const hostId = {host_id:?};
          const registry = window.__yggtermXtermHosts || {{}};
          const entry = registry[hostId];
          if (!entry || typeof entry.setInputEnabled !== "function") {{
            return;
          }}
          try {{
            entry.setInputEnabled({enabled}, {focus}, true, 'rust_policy');
          }} catch (_error) {{}}
        }})();
        "#,
        host_id = host_id,
        enabled = if enabled { "true" } else { "false" },
        focus = if focus { "true" } else { "false" },
    )
}
fn terminal_set_input_enabled_script_for_session(
    session_path: &str,
    enabled: bool,
    focus: bool,
) -> String {
    format!(
        r#"
        (() => {{
          const sessionPath = {session_path:?};
          const registry = window.__yggtermXtermHosts || {{}};
          const entries = Object.values(registry)
            .filter((entry) => entry && entry.sessionPath === sessionPath && typeof entry.setInputEnabled === "function")
            .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
          // NEVER pull focus into a terminal that a DOCUMENT SURFACE is covering.
          // This is the path that actually stole the caret out of yedit's editor:
          // `refocus_terminal_session_input` fires from ~10 call sites and asked
          // for focus unconditionally, with none of the uiOwnsFocus arbitration
          // the reclaim script does. Enabling input stays correct (the session is
          // live either way); only the FOCUS grab stands down while covered.
          let coveredByDocument = false;
          try {{
            const hostEl = Array.from(
              document.querySelectorAll('[data-terminal-session-path]')
            ).find((el) => String(el.getAttribute('data-terminal-session-path') || '') === sessionPath);
            coveredByDocument = !!(
              hostEl
              && String(hostEl.getAttribute('data-document-surface-owns-viewport') || '') === 'true'
            );
          }} catch (_error) {{}}
          const wantFocus = {focus} && !coveredByDocument;
          for (const entry of entries) {{
            try {{
              entry.setInputEnabled({enabled}, wantFocus, true, 'rust_policy');
            }} catch (_error) {{}}
          }}
        }})();
        "#,
        session_path = session_path,
        enabled = if enabled { "true" } else { "false" },
        focus = if focus { "true" } else { "false" },
    )
}
// ============================================================================
// SECTION: retained-replay JS scripts (per-attach, per-redraw)
// ----------------------------------------------------------------------------
// Smaller JS snippets generated on demand to write retained bytes into a
// live xterm host, drive cursor positioning, and trigger refresh. Key
// generators below: `terminal_replay_retained_data_script_for_session`,
// nudge/redraw helpers, and post-replay sanity probes. This is where the
// `followPromptForEntry` guard lives (XTERM-BUG: scrollback-lost-on-
// session-switch) — see retained_replay_script_followPromptForEntry_guards_
// user_scrollback test in the test module.
// ============================================================================
/// `runtime_spawn_id` is the cold-re-resume signal for the vacuum guard
/// (terminal_retained_replay_policy::retained_replay_would_vacuum_richer_client):
/// the daemon-reported spawn id of the PTY this payload was read from. The JS
/// entry records the id it last seeded from; a payload from a DIFFERENT spawn
/// (runtime exited+replaced, or a daemon-restart re-resume) that is much
/// sparser than a rich client buffer is refused (kept client). 0 = unknown →
/// the guard never arms (fails open). Same-spawn payloads are normal reveals
/// and are NEVER guarded — the 2.8.64 blanket-ratio regression (gating every
/// codex reveal-reconcile into a shadow) is structurally impossible.
fn terminal_replay_retained_data_script_for_session(
    session_path: &str,
    data: &str,
    source: &str,
    runtime_spawn_id: u64,
) -> String {
    let session_path = serde_json::to_string(session_path).unwrap_or_else(|_| "null".to_string());
    let data = sanitize_terminal_replay_payload(data);
    let data = serde_json::to_string(&data).unwrap_or_else(|_| "\"\"".to_string());
    let source = serde_json::to_string(source)
        .unwrap_or_else(|_| "\"daemon_retained_snapshot\"".to_string());
    format!(
        r#"
        (() => {{
          const sessionPath = {session_path};
          const data = {data};
          const replaySource = {source};
          const runtimeSpawnId = {runtime_spawn_id};
          if (!data) {{
            return;
          }}
          const replayKey = `${{sessionPath}}:${{data.length}}:${{data.slice(-160)}}`;
          const retryDelayMs = 100;
          const deadlineMs = Date.now() + 15000;
          const stableUntilMs = Date.now() + 3500;
          const strippedReplayText = String(data)
            .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
            .replace(/\x1b[=>]/g, "");
          const rawPayloadLineCount = (data.match(/\n/g) || []).length;
          const replayLooksCodex = /\bOpenAI Codex\b|\bgpt-[0-9]/i.test(strippedReplayText);
          const terminalPayloadDebugSample = (payload) => {{
            try {{
              return String(payload || '')
                .slice(-4096)
                .replace(/\x1b/g, '\\x1b')
                .replace(/\r/g, '\\r')
                .replace(/\n/g, '\\n')
                .replace(/\t/g, '\\t');
            }} catch (_error) {{
              return '';
            }}
          }};
          const retainedReplayCursorAddressedScrollbackRisk = () => {{
            if (
              replaySource === 'daemon_screen_snapshot'
              || replaySource === 'xterm_session_snapshot'
              || replaySource === 'daemon_retained_history_screen_snapshot'
            ) {{
              return false;
            }}
            if (rawPayloadLineCount <= 64) {{
              return false;
            }}
            if (
              data.includes('YGG_REMOTE_RETAINED_SCROLLBACK_')
              || data.includes('YGG_UI_RETAINED_HISTORY_')
            ) {{
              return false;
            }}
            const csiCount = (data.match(/\x1b\[/g) || []).length;
            const eraseCount = (data.match(/\x1b\[[0-9;?]*[JK]/g) || []).length;
            const cursorMoveCount = (data.match(/\x1b\[[0-9;?]*[HfGd]/g) || []).length;
            return csiCount >= 16 || eraseCount >= 8 || cursorMoveCount >= 8;
          }};
          // XTERM-BUG: remote-cc-replay-codex-only — recognize BOTH the Codex
          // caret (U+203A ›) and the Claude Code caret (U+276F ❯); a Claude
          // replay derived a needle only from › before, so it was empty.
          const codexPromptIx = strippedReplayText.lastIndexOf("›");
          const claudePromptIx = strippedReplayText.lastIndexOf("❯");
          const promptIx = Math.max(codexPromptIx, claudePromptIx);
          const promptNeedle = promptIx >= 0
            ? strippedReplayText.slice(promptIx, promptIx + 96).replace(/\s+/g, " ").trim()
            : "";
          window.__yggtermPendingRetainedReplays = window.__yggtermPendingRetainedReplays || {{}};
          const pending = window.__yggtermPendingRetainedReplays;
          if (pending[sessionPath] && pending[sessionPath].key === replayKey && !pending[sessionPath].complete) {{
            return;
          }}
          pending[sessionPath] = {{ key: replayKey, complete: false, deadlineMs, stableUntilMs }};
          const visibleEntryForSession = () => {{
            const registry = window.__yggtermXtermHosts || {{}};
            const entries = Object.values(registry)
              .filter((entry) => entry && entry.term && entry.sessionPath === sessionPath)
              .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
            const visibleEntries = entries.filter((entry) => {{
              try {{
                const host = document.getElementById(entry.hostId);
                if (!host) {{
                  return false;
                }}
                const rect = host.getBoundingClientRect();
                const style = window.getComputedStyle(host);
                return rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
              }} catch (_error) {{
                return false;
              }}
            }});
            return visibleEntries[0] || null;
          }};
          const visibleTextForEntry = (entry) => {{
            try {{
              const buffer = entry && entry.term && entry.term.buffer && entry.term.buffer.active;
              if (!buffer || typeof buffer.getLine !== "function") {{
                return "";
              }}
              const baseY = Number(buffer.baseY || 0);
              const rows = Math.max(1, Number(entry.term.rows || 1));
              const length = Math.max(0, Number(buffer.length || rows));
              const start = Math.max(0, Math.min(baseY, length));
              const end = Math.min(length, start + rows);
              const lines = [];
              for (let row = start; row < end; row += 1) {{
                const line = buffer.getLine(row);
                if (line && typeof line.translateToString === "function") {{
                  lines.push(line.translateToString(true));
                }}
              }}
              return lines.join("\n");
            }} catch (_error) {{
              return "";
            }}
          }};
          const normalizedTerminalLine = (line) => String(line || "")
            .trim()
            .replace(/^[›>\s]+/, "")
            .trim()
            .toLowerCase();
          const lineIsInternalTransportError = (line) => {{
            const normalized = normalizedTerminalLine(line);
            return normalized.startsWith("error: terminal session not found: local://")
              || normalized.startsWith("terminal session not found: local://")
              || normalized.startsWith("error: terminal session not found: remote-session://")
              || normalized.startsWith("terminal session not found: remote-session://")
              || normalized.startsWith("error: terminal session not found: codex-runtime://")
              || normalized.startsWith("terminal session not found: codex-runtime://")
              || normalized.includes("error: terminal session not found: local://")
              || normalized.includes("error: terminal session not found: remote-session://")
              || normalized.includes("error: terminal session not found: codex-runtime://");
          }};
          const lineIsSharedConnectionClose = (line) => {{
            const normalized = normalizedTerminalLine(line);
            return normalized.startsWith("shared connection to ")
              && (normalized.includes(" closed")
                || normalized.includes(" refused")
                || normalized.includes(" timed out"));
          }};
          const lineIsSharedConnectionNotice = (line) => {{
            const normalized = normalizedTerminalLine(line);
            return normalized.startsWith("shared connection to ");
          }};
          const lineIsPromptLike = (line) => {{
            const normalized = String(line || "").trim();
            return normalized.startsWith("›") || normalized.startsWith(">");
          }};
          const visibleTextHasInternalTransportLeak = (text) => {{
            const lines = String(text || "").split(/\r?\n/);
            return lines.some((line) => lineIsInternalTransportError(line) || lineIsSharedConnectionNotice(line));
          }};
          const replayVisibleInEntry = (entry) => {{
            const visibleText = visibleTextForEntry(entry);
            if (!visibleText) {{
              return false;
            }}
            if (visibleTextHasInternalTransportLeak(visibleText)) {{
              return false;
            }}
            // XTERM-BUG: remote-cc-replay-codex-only — a settled replay is
            // recognized by the Codex caret (U+203A ›) OR the Claude Code caret
            // (U+276F ❯) OR Claude's idle footer ("? for shortcuts"). Without the
            // Claude signals a correctly-replayed Claude buffer was judged
            // not-visible, so completion never fired and the 100ms retry loop
            // reset+rewrote the buffer every tick (flash/churn) until the deadline.
            if (visibleText.includes("›") || visibleText.includes("❯")) {{
              return true;
            }}
            if (visibleText.toLowerCase().includes("? for shortcuts")) {{
              return true;
            }}
            return Boolean(promptNeedle && visibleText.replace(/\s+/g, " ").includes(promptNeedle));
          }};
          const promptViewportReadyInEntry = (entry) => {{
            try {{
              const buffer = entry && entry.term && entry.term.buffer && entry.term.buffer.active;
              if (!buffer) {{
                return false;
              }}
              const baseY = Math.max(0, Number(buffer.baseY || 0));
              const viewportY = Math.max(0, Number(buffer.viewportY || 0));
              const cursorY = Math.max(0, Number(buffer.cursorY || 0));
              const rows = Math.max(1, Number(entry.term.rows || 1));
              const cursorLineIndex = baseY + cursorY;
              const visibleCursorRow = cursorLineIndex - viewportY;
              return viewportY >= baseY
                && visibleCursorRow >= 0
                && visibleCursorRow < rows;
            }} catch (_error) {{
              return false;
            }}
          }};
          const currentPromptReadyInEntry = (entry) => {{
            try {{
              if (!promptViewportReadyInEntry(entry)) {{
                return false;
              }}
              const buffer = entry && entry.term && entry.term.buffer && entry.term.buffer.active;
              if (!buffer || typeof buffer.getLine !== "function") {{
                return false;
              }}
              const baseY = Math.max(0, Number(buffer.baseY || 0));
              const cursorY = Math.max(0, Number(buffer.cursorY || 0));
              const cursorLine = buffer.getLine(baseY + cursorY);
              const cursorText = cursorLine && typeof cursorLine.translateToString === "function"
                ? cursorLine.translateToString(true)
                : "";
              return lineIsPromptLike(cursorText) || cursorText.includes("›");
            }} catch (_error) {{
              return false;
            }}
          }};
          const replayPromptReadyInEntry = (entry) => {{
            return replayLooksCodex
              ? currentPromptReadyInEntry(entry)
              : promptViewportReadyInEntry(entry);
          }};
          const retainedReplaySupersededByDaemonPty = (entry, reason = 'retained_replay_superseded_by_daemon_pty') => {{
            try {{
              if (!entry || String(entry.terminalContentSource || '') !== 'daemon_pty') {{
                return false;
              }}
              if (
                Number(entry.lastRetainedReplayPromotedAtMs || 0) <= 0
                && !String(entry.lastRetainedReplayPromotedFrom || '')
              ) {{
                return false;
              }}
              const current = pending[sessionPath];
              if (current && current.key === replayKey) {{
                current.complete = true;
                current.supersededByDaemonPty = true;
                current.supersededReason = String(reason || 'retained_replay_superseded_by_daemon_pty');
              }}
              entry.lastRetainedReplayPromptFollowReady = true;
              entry.lastRetainedReplaySupersededByDaemonPty = true;
              entry.lastRetainedReplayRejectedVisibleText = 'retained_replay_superseded_by_daemon_pty';
              entry.lastRetainedReplayFollowDebug = {{
                reason: String(reason || 'retained_replay_superseded_by_daemon_pty'),
                superseded_by_daemon_pty: true,
                terminal_content_source: 'daemon_pty',
              }};
              try {{
                if (typeof entry.emitHostHealth === "function") {{
                  entry.emitHostHealth('retained_replay_superseded_by_daemon_pty');
                }}
              }} catch (_error) {{}}
              return true;
            }} catch (_error) {{
              return false;
            }}
          }};
          const retainedReplayBlockedByLiveInput = (entry, reason = 'retained_replay_blocked_by_live_input') => {{
            try {{
              if (!entry) {{
                return false;
              }}
              const inputEnabledNow = Boolean(entry.inputEnabled);
              const inputHotNow = Date.now() < Number(entry.terminalInputHotUntilMs || 0);
              if (!inputEnabledNow && !inputHotNow) {{
                return false;
              }}
              const current = pending[sessionPath];
              if (current && current.key === replayKey) {{
                current.complete = true;
                current.supersededByDaemonPty = true;
                current.supersededReason = String(reason || 'retained_replay_blocked_by_live_input');
              }}
              entry.lastRetainedReplayPromptFollowReady = true;
              entry.lastRetainedReplaySupersededByDaemonPty = true;
              entry.lastRetainedReplayRejectedVisibleText = String(reason || 'retained_replay_blocked_by_live_input');
              entry.lastRetainedReplayFollowDebug = {{
                reason: String(reason || 'retained_replay_blocked_by_live_input'),
                host_stdin_enabled: inputEnabledNow,
                input_hot: inputHotNow,
              }};
              try {{
                if (typeof entry.emitHostHealth === "function") {{
                  entry.emitHostHealth('retained_replay_blocked_by_live_input');
                }}
              }} catch (_error) {{}}
              return true;
            }} catch (_error) {{
              return false;
            }}
          }};
          const followPromptForEntry = (entry, reason) => {{
            const debug = {{
              reason: String(reason || 'retained_replay_prompt_follow'),
              used_entry_force_prompt_follow: false,
              used_entry_force_viewport_y: false,
              used_scroll_to_bottom: false,
              used_core_scroll_lines: false,
              used_refresh: false,
            }};
            try {{
              // XTERM-BUG: scrollback-lost-on-session-switch
              // See docs/xterm-bugs.md#scrollback-lost-on-session-switch
              // If the user is actively scrolled back, do NOT force prompt
              // follow — retained replay paths (session switch, snapshot
              // re-apply, daemon-to-replay handoff) MUST preserve the user's
              // scroll position. Without this guard the entire scrollback
              // collapses every time we re-mount.
              if (entry && String(entry.scrollbackIntent || 'PromptFollow') === 'UserScrollback') {{
                debug.skipped_user_scrollback = true;
                return debug;
              }}
              const buffer = entry && entry.term && entry.term.buffer && entry.term.buffer.active;
              if (!entry || !entry.term || !buffer) {{
                debug.missing_entry = true;
                return debug;
              }}
              const baseY = Math.max(0, Number(buffer.baseY || 0));
              const beforeViewportY = Math.max(0, Number(buffer.viewportY || 0));
              debug.before_viewport_y = beforeViewportY;
              debug.before_base_y = baseY;
              if (typeof entry.forcePromptFollow === "function") {{
                try {{
                  debug.entry_force_prompt_follow = entry.forcePromptFollow(reason || 'retained_replay_prompt_follow');
                  debug.used_entry_force_prompt_follow = true;
                }} catch (_error) {{}}
              }} else if (typeof entry.forceXtermViewportY === "function") {{
                try {{
                  debug.entry_force_viewport_y = entry.forceXtermViewportY(baseY, reason || 'retained_replay_prompt_follow');
                  debug.used_entry_force_viewport_y = true;
                }} catch (_error) {{}}
              }} else {{
                try {{
                  if (typeof entry.term.scrollToBottom === "function") {{
                    entry.term.scrollToBottom();
                    debug.used_scroll_to_bottom = true;
                  }}
                }} catch (_error) {{}}
                let afterViewportY = Math.max(0, Number(buffer.viewportY || 0));
                const core = entry.term && entry.term._core ? entry.term._core : null;
                if (afterViewportY !== baseY && core && typeof core.scrollLines === "function") {{
                  try {{
                    core.scrollLines(baseY - afterViewportY, false, 1);
                    debug.used_core_scroll_lines = true;
                  }} catch (_error) {{}}
                }}
                try {{
                  const host = entry.host || (entry.hostId ? document.getElementById(entry.hostId) : null);
                  const viewportElement = host ? host.querySelector(".xterm-viewport") : null;
                  if (viewportElement) {{
                    const dimensions = entry.term && entry.term._core && entry.term._core._renderService
                      ? entry.term._core._renderService.dimensions
                      : null;
                    const cssCell = dimensions && dimensions.css && dimensions.css.cell
                      ? dimensions.css.cell
                      : null;
                    const rowHeightPx = Math.max(1, Number(cssCell && cssCell.height ? cssCell.height : 0) || Number(entry.term.options.fontSize || 18) || 18);
                    debug.viewport_scroll_top_before = Number(viewportElement.scrollTop || 0);
                    viewportElement.scrollTop = baseY * rowHeightPx;
                    debug.viewport_scroll_top_after = Number(viewportElement.scrollTop || 0);
                  }}
                }} catch (_error) {{}}
                try {{
                  if (typeof entry.term.refresh === "function") {{
                    entry.term.refresh(0, Math.max(0, Number(entry.term.rows || 1) - 1));
                    debug.used_refresh = true;
                  }}
                }} catch (_error) {{}}
              }}
              debug.after_viewport_y = Math.max(0, Number(buffer.viewportY || 0));
              debug.after_base_y = Math.max(0, Number(buffer.baseY || 0));
              debug.prompt_viewport_ready = promptViewportReadyInEntry(entry);
              debug.current_prompt_ready = currentPromptReadyInEntry(entry);
              entry.lastRetainedReplayFollowDebug = debug;
              entry.lastRetainedReplayPromptFollowReady = Boolean(
                replayLooksCodex ? debug.current_prompt_ready : debug.prompt_viewport_ready
              );
            }} catch (error) {{
              debug.error = error && error.message ? error.message : String(error);
            }}
            // XTERM-BUG: switch-reveal-broken-bottom (campaign TODO-1, 2026-06-07)
            // The daemon delivers the reveal/reconcile screen+history in CHUNKS, so
            // baseY keeps growing AFTER this initial follow lands -> the viewport sits
            // a few rows above the true bottom (live composer below view = "broken
            // bottom") until an organic trigger catches up (the transient the user
            // catches on ~70% of switches). Re-assert the follow ONCE after the replay
            // settles so we land at the FINAL baseY. SAFE: this only moves the viewport
            // (reuses forcePromptFollow) — it does NOT re-read the daemon or rebuild the
            // buffer, so it cannot trigger the recovery-churn trap. Gated: the ':settle'
            // reason prevents recursion, and a user who scrolled back flips intent to
            // UserScrollback -> the top-guard early-returns (never yanks the user).
            try {{
              if (!String(reason || '').endsWith(':settle')) {{
                window.setTimeout(() => {{
                  try {{
                    if (entry && String(entry.scrollbackIntent || 'PromptFollow') !== 'UserScrollback') {{
                      followPromptForEntry(entry, `${{String(reason || 'retained_replay')}}:settle`);
                    }}
                  }} catch (_settleError) {{}}
                }}, 280);
              }}
            }} catch (_scheduleError) {{}}
            return debug;
          }};
          const refreshRetainedReplayPaint = (entry, reason) => {{
            const debug = {{
              reason: String(reason || 'retained_replay_paint_refresh'),
              used_refresh_now: false,
              used_refresh_raf: false,
              used_refresh_120ms: false,
              used_host_health_now: false,
              used_host_health_raf: false,
              used_host_health_120ms: false,
            }};
            const refreshOnce = (phase) => {{
              try {{
                if (!entry || !entry.term) {{
                  debug[`missing_entry_${{phase}}`] = true;
                  return;
                }}
                if (typeof entry.term.refresh === "function") {{
                  entry.term.refresh(0, Math.max(0, Number(entry.term.rows || 1) - 1));
                  debug[`used_refresh_${{phase}}`] = true;
                }}
                if (typeof entry.emitHostHealth === "function") {{
                  entry.emitHostHealth(`${{reason || 'retained_replay_paint_refresh'}}:${{phase}}`);
                  debug[`used_host_health_${{phase}}`] = true;
                }}
              }} catch (error) {{
                debug[`error_${{phase}}`] = error && error.message ? String(error.message) : String(error);
              }}
              try {{
                entry.lastRetainedReplayPaintRefreshDebug = debug;
              }} catch (_error) {{}}
            }};
            refreshOnce('now');
            try {{
              window.requestAnimationFrame(() => refreshOnce('raf'));
            }} catch (_error) {{}}
            try {{
              window.setTimeout(() => refreshOnce('120ms'), 120);
            }} catch (_error) {{}}
            return debug;
          }};
          // XTERM-BUG: blank-viewport-client-snapshot-poison (replay-script copy)
          // Mirror of the bootstrap-script guard (separate eval scope). A frame
          // collapsed to <=1 nonblank line for a session whose tracked nonblank
          // max is >=6 is a poison frame; reject it so the daemon authoritative
          // replay wins.
          const xtermSessionSnapshotIsCollapsedPoison = (sessionPath, nonblankLineCount) => {{
            try {{
              const maxMap = window.__yggtermXtermSessionNonblankMax || {{}};
              const priorMax = Math.max(0, Number(maxMap[sessionPath] || 0));
              return Number(nonblankLineCount) <= 1 && priorMax >= 6;
            }} catch (_error) {{
              return false;
            }}
          }};
          const sessionSnapshotForReplay = () => {{
            try {{
              const snapshots = window.__yggtermXtermSessionSnapshots || {{}};
              const snapshot = snapshots[sessionPath] || null;
              if (!snapshot || typeof snapshot.text !== 'string') {{
                return null;
              }}
              const ageMs = Date.now() - Number(snapshot.capturedAtMs || 0);
              if (!Number.isFinite(ageMs) || ageMs < 0 || ageMs > 10 * 60 * 1000) {{
                return null;
              }}
              const text = String(snapshot.text || '');
              const lineCount = Math.max(
                Number(snapshot.lineCount || 0),
                Number(snapshot.logicalLineCount || 0),
                (text.match(/\n/g) || []).length + 1
              );
              const nonblankLineCount = Number(snapshot.nonblankLineCount || 0);
              if (!text.trim() || lineCount <= 0 || nonblankLineCount <= 0) {{
                return null;
              }}
              if (visibleTextHasInternalTransportLeak(text)) {{
                return null;
              }}
              // XTERM-BUG: blank-viewport-client-snapshot-poison (replay guard)
              // Same defense as the construct-time restore: never replay a sparse
              // snapshot for a session that previously had real content; fall
              // through to the daemon authoritative replay instead.
              if (xtermSessionSnapshotIsCollapsedPoison(sessionPath, nonblankLineCount)) {{
                return null;
              }}
              return {{
                ...snapshot,
                ageMs,
                lineCount,
                nonblankLineCount,
              }};
            }} catch (_error) {{
              return null;
            }}
          }};
          const writePayloadIntoEntry = (entry, payload) => {{
            // Vacuum guard (retained_replay_would_vacuum_richer_client): a payload
            // read from a DIFFERENT runtime spawn than the one this entry was
            // seeded from is a COLD RE-RESUME (runtime exited+replaced / daemon
            // restart). If the client scrollback is rich (baseY >= 6) and that
            // fresh-PTY frame is much sparser (< 1/3 the line count), the reset
            // prefix below ("\x1b[3J" clears scrollback) would collapse the whole
            // transcript (live-caught: baseY 1801 -> 32) — codex repaints in
            // place, so the conversation lives ONLY in the client buffer. KEEP
            // the client; codex's next frame repaints. A SAME-spawn payload is a
            // normal reveal and is never guarded (the 2.8.64 blanket-ratio
            // regression that gated every codex reveal into a shadow); spawn id
            // 0 (unknown / user reconcile / older daemon) also never guards.
            try {{
              const knownSpawnId = Number(entry.lastSeededRuntimeSpawnId || 0);
              const coldReResume = runtimeSpawnId > 0
                && knownSpawnId > 0
                && runtimeSpawnId !== knownSpawnId;
              if (coldReResume) {{
                const curBuffer = entry && entry.term && entry.term.buffer && entry.term.buffer.active;
                const curBaseY = curBuffer ? Math.max(0, Number(curBuffer.baseY || 0)) : 0;
                const incomingLines = (String(payload).match(/\n/g) || []).length;
                if (curBaseY >= 6 && incomingLines * 3 < curBaseY) {{
                  entry.lastRetainedReplayVacuumGuardSkipped = true;
                  entry.lastRetainedReplayVacuumGuardDebug = {{
                    base_y: curBaseY,
                    incoming_lines: incomingLines,
                    known_spawn_id: knownSpawnId,
                    incoming_spawn_id: runtimeSpawnId,
                  }};
                  // Return TRUE = handled by keeping the client (no retry); the
                  // client already holds the richer transcript. Do NOT record the
                  // new spawn id: the guard re-arms until the fresh runtime
                  // produces a comparably rich frame, which then applies normally.
                  return true;
                }}
              }}
            }} catch (_error) {{}}
            const replayResetPrefix = replaySource === 'daemon_retained_history_screen_snapshot'
              ? "\x1bc\x1b[H"
              : "\x1bc\x1b[2J\x1b[3J\x1b[H";
            // ⭐ THE PAIR THE GHOST-FRAME ENTRY ASKS ABOUT. This path wipes the
            // screen and reseeds it, and the open question is whether anything
            // lands in between — a half-old, half-new canvas is what unreadable
            // output looks like from the inside. Both ends are marked, so the
            // total order the emitter's `seq` provides can answer it directly
            // instead of by inference from timestamps that collide.
            //
            // ⛔ Emitted through the page-global emitter, not the per-host
            // helper: this script is generated by a different function and has
            // no probe helpers in scope. Guarded, because the emitter is
            // installed by the terminal script and this one can run first.
            if (window.__yggtermTrace) {{
              window.__yggtermTrace.emit({{
                category: "xterm_screen",
                name: "replay_reset",
                payload: {{
                  host_id: String(entry.hostId || ''),
                  replay_source: String(replaySource || ''),
                }},
              }});
              window.__yggtermTrace.armStreamCapture(
                String(entry.hostId || ''),
                'replay:' + String(replaySource || '')
              );
            }}
            try {{
              if (typeof entry.term.reset === "function") {{
                entry.term.reset();
              }}
              if (typeof entry.term.clear === "function") {{
                entry.term.clear();
              }}
            }} catch (_error) {{}}
            const writeSync = entry.term && entry.term._core && typeof entry.term._core.writeSync === "function"
              ? entry.term._core.writeSync.bind(entry.term._core)
              : entry.term && entry.term._core && entry.term._core._writeBuffer && typeof entry.term._core._writeBuffer.writeSync === "function"
                ? entry.term._core._writeBuffer.writeSync.bind(entry.term._core._writeBuffer)
                : null;
            try {{
              // Replayed scrollback must not re-fire OSC 52 copy (see the OSC 52 handler).
              if (window.__yggtermArmOsc52Suppress) {{
                window.__yggtermArmOsc52Suppress(String(entry.hostId || ''), 400);
              }}
              // XTERM-BUG: cold-reveal-bulk-write-freeze — raise the bulk-write
              // in-flight signal so the per-line onScroll heavy tail is skipped
              // during this replay parse (same skip as bridge write flushes).
              entry.writeBridgeInFlight = true;
              entry.lastWriteFlushStartedAtMs = Date.now();
              // ⭐ THE HALF THE GHOST-FRAME ENTRY SUSPECTS. The daemon serves a
              // formatted screen, so if these bytes carry no SGR colour the
              // stripping happened before the canvas ever saw them; if they do
              // carry it and the canvas still paints plain, the fault is in
              // applying the attributes. Tagged `reseed` so the two answers
              // cannot be confused with the live stream's.
              if (window.__yggtermTrace && window.__yggtermTrace.captureStream) {{
                window.__yggtermTrace.captureStream(String(entry.hostId || ''), "reseed", payload);
              }}
              try {{
                if (writeSync) {{
                  writeSync(replayResetPrefix);
                  writeSync(payload);
                }} else if (typeof entry.term.write === "function") {{
                  entry.term.write(`${{replayResetPrefix}}${{payload}}`, () => {{
                    entry.writeBridgeInFlight = false;
                  }});
                }}
              }} finally {{
                if (writeSync) {{
                  entry.writeBridgeInFlight = false;
                }}
              }}
              // The closing half of the pair. A `replay_reset` with no
              // `replay_reseed` after it is a screen that was wiped and never
              // refilled — the stale/blank viewport symptom — and any foreign
              // record whose `seq` falls BETWEEN the two wrote into a screen
              // that was mid-replacement.
              if (window.__yggtermTrace) {{
                window.__yggtermTrace.emit({{
                  category: "xterm_screen",
                  name: "replay_reseed",
                  payload: {{
                    host_id: String(entry.hostId || ''),
                    replay_source: String(replaySource || ''),
                    chars: String(payload || '').length,
                  }},
                }});
              }}
              // Record which runtime spawn this buffer is now seeded from — the
              // comparison anchor for the cold-re-resume vacuum guard above.
              if (runtimeSpawnId > 0) {{
                entry.lastSeededRuntimeSpawnId = runtimeSpawnId;
              }}
              return true;
            }} catch (_error) {{
              entry.writeBridgeInFlight = false;
              return false;
            }}
          }};
          const replaySessionSnapshotIntoEntry = (entry, snapshot) => {{
            try {{
              const normalizedText = String(snapshot.text || '').replace(/\r?\n/g, "\r\n");
              if (!normalizedText.trim()) {{
                return false;
              }}
              if (!writePayloadIntoEntry(entry, normalizedText)) {{
                entry.lastRetainedReplaySnapshotError = 'xterm_session_snapshot_write_failed';
                return false;
              }}
              const rows = Math.max(0, Number(entry.term && entry.term.rows ? entry.term.rows : 0));
              entry.__yggtermLastRetainedReplayKey =
                `${{replayKey}}:xterm_session_snapshot:${{Number(snapshot.capturedAtMs || 0)}}:${{normalizedText.length}}`;
              entry.lastRawPayloadLength = normalizedText.length;
              entry.lastRawPayloadLineCount = Number(snapshot.lineCount || 0);
              entry.lastRawPayloadSample = terminalPayloadDebugSample(normalizedText);
              entry.lastRetainedReplayLineCount = Number(snapshot.lineCount || 0);
              entry.lastRetainedReplayExpected = Number(snapshot.lineCount || 0) > Math.max(4, rows + 4);
              entry.lastRetainedReplaySource = 'xterm_session_snapshot';
              entry.lastRetainedReplayRecoveredFromSnapshot = true;
              entry.lastRetainedReplaySnapshotAgeMs = Number(snapshot.ageMs || 0);
              entry.lastRetainedReplaySnapshotError = '';
              entry.lastRetainedReplayRejectedVisibleText =
                'retained_replay_cursor_addressed_scrollback_risk_recovered_from_xterm_snapshot';
              entry.retainedReplayUnsafeSkipPromptReady = false;
              entry.terminalContentSource = 'xterm_session_snapshot';
              entry.terminalSourceMismatchReason = 'xterm_session_snapshot_observer_cache';
              entry.scrollbackExpected = Number(snapshot.lineCount || 0) > Math.max(4, rows + 4);
              followPromptForEntry(entry, 'retained_replay_xterm_session_snapshot');
              try {{
                if (typeof entry.term.refresh === "function") {{
                  entry.term.refresh(0, Math.max(0, Number(entry.term.rows || 1) - 1));
                }}
              }} catch (_error) {{}}
              try {{
                if (typeof entry.emitHostHealth === "function") {{
                  entry.emitHostHealth('retained_replay_xterm_session_snapshot');
                }}
              }} catch (_error) {{}}
              return true;
            }} catch (error) {{
              try {{
                entry.lastRetainedReplaySnapshotError =
                  error && error.message ? String(error.message) : String(error);
              }} catch (_error) {{}}
              return false;
            }}
          }};
          const retryLater = () => {{
            const current = pending[sessionPath];
            if (!current || current.key !== replayKey || current.complete || Date.now() >= deadlineMs) {{
              return;
            }}
            window.setTimeout(attemptReplay, retryDelayMs);
          }};
          const attemptReplay = () => {{
            const current = pending[sessionPath];
            if (!current || current.key !== replayKey || current.complete) {{
              return;
            }}
            const entry = visibleEntryForSession();
            if (!entry || !entry.term) {{
              retryLater();
              return;
            }}
            if (retainedReplaySupersededByDaemonPty(entry, 'retained_replay_attempt_superseded_by_daemon_pty')) {{
              return;
            }}
            if (retainedReplayBlockedByLiveInput(entry, 'retained_replay_attempt_blocked_by_live_input')) {{
              return;
            }}
            const buffer = entry.term.buffer && entry.term.buffer.active;
            const baseY = buffer ? Number(buffer.baseY || 0) : 0;
            const rows = Math.max(0, Number(entry.term.rows || 0));
            const expectsReplayScrollback = rawPayloadLineCount > Math.max(4, rows + 4);
            const collapsedScrollbackNeedsReplay = expectsReplayScrollback && baseY <= 0;
            const existingVisibleText = visibleTextForEntry(entry);
            const existingVisibleTransportLeak = visibleTextHasInternalTransportLeak(existingVisibleText);
            const authoritativeScreenReplay =
              replaySource === 'daemon_screen_snapshot'
              || replaySource === 'xterm_session_snapshot'
              || replaySource === 'daemon_retained_history_screen_snapshot';
            try {{
              entry.lastRetainedReplayLineCount = rawPayloadLineCount;
              entry.lastRetainedReplaySource = replaySource;
              entry.lastRetainedReplayExpected = expectsReplayScrollback;
              entry.lastRetainedReplayExistingTransportLeak = existingVisibleTransportLeak;
              if (existingVisibleTransportLeak) {{
                entry.lastRetainedReplayRejectedVisibleText = 'retained_replay_existing_transport_error';
              }}
              if (expectsReplayScrollback && baseY > 0) {{
                entry.scrollbackExpected = true;
              }}
              entry.retainedReplayUnsafeSkipPromptReady = false;
            }} catch (_error) {{}}
            if (collapsedScrollbackNeedsReplay && retainedReplayCursorAddressedScrollbackRisk()) {{
              const currentPromptReady = currentPromptReadyInEntry(entry);
              // XTERM-BUG: blank-viewport-client-snapshot-poison
              // Reconcile from the daemon's AUTHORITATIVE current screen frame
              // BEFORE falling back to the cached client snapshot. The client
              // xterm_session_snapshot can be a collapsed/sparse frame (nonblank
              // far below the real screen) that the <=1 poison guard does not
              // catch; preferring it left the codex viewport clipped/blank with
              // the daemon's full frame never reconciled in (promoted count 0),
              // which then trips the "viewport beyond scrollback base" surface
              // problem and the blink/reseed/restart escalation. A daemon SCREEN
              // snapshot (NOT cursor-addressed scrollback history) is the real
              // current frame and is safe to write; it self-corrects on the next
              // codex repaint, unlike the client snapshot which latches.
              const daemonScreenSnapshotAuthoritative =
                replaySource === 'daemon_screen_snapshot'
                || replaySource === 'daemon_retained_history_screen_snapshot';
              if (daemonScreenSnapshotAuthoritative && writePayloadIntoEntry(entry, data)) {{
                entry.__yggtermLastRetainedReplayKey = replayKey;
                entry.lastRawPayloadLength = data.length;
                entry.lastRawPayloadLineCount = rawPayloadLineCount;
                entry.lastRawPayloadSample = terminalPayloadDebugSample(data);
                entry.lastRetainedReplaySource = replaySource;
                entry.lastRetainedReplayRecoveredFromSnapshot = false;
                entry.terminalContentSource = replaySource;
                entry.retainedReplayUnsafeSkipPromptReady = false;
                entry.retainedReplayPromotedToDaemonPtyCount =
                  Number(entry.retainedReplayPromotedToDaemonPtyCount || 0) + 1;
                followPromptForEntry(entry, 'retained_replay_reconcile_from_daemon_screen');
                try {{
                  if (typeof entry.term.refresh === "function") {{
                    entry.term.refresh(0, Math.max(0, Number(entry.term.rows || 1) - 1));
                  }}
                }} catch (_error) {{}}
                try {{
                  if (typeof entry.emitHostHealth === "function") {{
                    entry.emitHostHealth('retained_replay_reconcile_from_daemon_screen');
                  }}
                }} catch (_error) {{}}
                if (Date.now() >= current.stableUntilMs && replayPromptReadyInEntry(entry)) {{
                  current.complete = true;
                }} else {{
                  retryLater();
                }}
                return;
              }}
              const snapshot = sessionSnapshotForReplay();
              if (snapshot && replaySessionSnapshotIntoEntry(entry, snapshot)) {{
                if (Date.now() >= current.stableUntilMs && replayPromptReadyInEntry(entry)) {{
                  current.complete = true;
                }} else {{
                  retryLater();
                }}
                return;
              }}
              try {{
                entry.lastRetainedReplayRejectedVisibleText = 'retained_replay_cursor_addressed_scrollback_risk';
                entry.lastRetainedReplayPromptFollowReady = currentPromptReady;
                entry.retainedReplayUnsafeSkipPromptReady = false;
                entry.scrollbackExpected = true;
              }} catch (_error) {{}}
              if (currentPromptReady) {{
                followPromptForEntry(entry, 'retained_replay_cursor_risk_prompt_only_rejected');
                retryLater();
                return;
              }}
              if (replayVisibleInEntry(entry)) {{
                followPromptForEntry(entry, 'retained_replay_cursor_risk_existing_visible_rejected');
              }}
              retryLater();
              return;
            }}
            if (entry.__yggtermLastRetainedReplayKey === replayKey) {{
              if (!collapsedScrollbackNeedsReplay && replayVisibleInEntry(entry)) {{
                followPromptForEntry(entry, 'retained_replay_cached_visible');
                refreshRetainedReplayPaint(entry, 'retained_replay_cached_visible');
                if (Date.now() >= current.stableUntilMs && replayPromptReadyInEntry(entry)) {{
                  current.complete = true;
                }} else {{
                  retryLater();
                }}
                return;
              }}
            }}
            if (!authoritativeScreenReplay && !collapsedScrollbackNeedsReplay && replayVisibleInEntry(entry)) {{
              followPromptForEntry(entry, 'retained_replay_existing_visible');
              refreshRetainedReplayPaint(entry, 'retained_replay_existing_visible');
              if (Date.now() >= current.stableUntilMs && replayPromptReadyInEntry(entry)) {{
                current.complete = true;
              }} else {{
                retryLater();
              }}
              return;
            }}
            if (!authoritativeScreenReplay && baseY > 0 && !existingVisibleTransportLeak) {{
              followPromptForEntry(entry, 'retained_replay_existing_scrollback');
              const promptReadyAfterFollow = replayPromptReadyInEntry(entry);
              if (!replayLooksCodex || promptReadyAfterFollow) {{
                if (Date.now() >= current.stableUntilMs && promptReadyAfterFollow) {{
                  current.complete = true;
                }} else {{
                  retryLater();
                }}
                return;
              }}
            }}
            if (window.__yggtermTrace) {{
              window.__yggtermTrace.emit({{
                category: "xterm_screen",
                name: "replay_reset",
                payload: {{
                  host_id: String(entry.hostId || ''),
                  replay_source: String(replaySource || ''),
                  stage: "follow_retry",
                }},
              }});
              window.__yggtermTrace.armStreamCapture(
                String(entry.hostId || ''),
                'replay_follow_retry:' + String(replaySource || '')
              );
            }}
            try {{
              if (typeof entry.term.reset === "function") {{
                entry.term.reset();
              }}
              if (typeof entry.term.clear === "function") {{
                entry.term.clear();
              }}
            }} catch (_error) {{}}
            const writeSync = entry.term && entry.term._core && typeof entry.term._core.writeSync === "function"
              ? entry.term._core.writeSync.bind(entry.term._core)
              : entry.term && entry.term._core && entry.term._core._writeBuffer && typeof entry.term._core._writeBuffer.writeSync === "function"
                ? entry.term._core._writeBuffer.writeSync.bind(entry.term._core._writeBuffer)
                : null;
            try {{
              const replayResetPrefix = replaySource === 'daemon_retained_history_screen_snapshot'
                ? "\x1bc\x1b[H"
                : "\x1bc\x1b[2J\x1b[3J\x1b[H";
              // Replayed scrollback must not re-fire OSC 52 copy (see the OSC 52 handler).
              if (window.__yggtermArmOsc52Suppress) {{
                window.__yggtermArmOsc52Suppress(String(entry.hostId || ''), 400);
              }}
              // XTERM-BUG: cold-reveal-bulk-write-freeze — raise the bulk-write
              // in-flight signal so the per-line onScroll heavy tail is skipped
              // during this replay parse (same skip as bridge write flushes).
              entry.writeBridgeInFlight = true;
              entry.lastWriteFlushStartedAtMs = Date.now();
              if (window.__yggtermTrace && window.__yggtermTrace.captureStream) {{
                window.__yggtermTrace.captureStream(String(entry.hostId || ''), "reseed", data);
              }}
              try {{
                if (writeSync) {{
                  writeSync(replayResetPrefix);
                  writeSync(data);
                }} else if (typeof entry.term.write === "function") {{
                  entry.term.write(`${{replayResetPrefix}}${{data}}`, () => {{
                    entry.writeBridgeInFlight = false;
                  }});
                }}
              }} finally {{
                if (writeSync) {{
                  entry.writeBridgeInFlight = false;
                }}
              }}
              if (window.__yggtermTrace) {{
                window.__yggtermTrace.emit({{
                  category: "xterm_screen",
                  name: "replay_reseed",
                  payload: {{
                    host_id: String(entry.hostId || ''),
                    replay_source: String(replaySource || ''),
                    stage: "follow_retry",
                    chars: String(data || '').length,
                  }},
                }});
              }}
            }} catch (_error) {{
              entry.writeBridgeInFlight = false;
            }}
            try {{
              entry.__yggtermLastRetainedReplayKey = replayKey;
              entry.lastRawPayloadLength = data.length;
              entry.lastRawPayloadLineCount = rawPayloadLineCount;
              entry.lastRawPayloadSample = terminalPayloadDebugSample(data);
              entry.lastRetainedReplaySource = replaySource;
              entry.retainedReplayUnsafeSkipPromptReady = false;
              entry.terminalContentSource = replaySource;
              entry.terminalSourceMismatchReason = String(replaySource).includes('server_prompt')
                ? 'non_pty_server_snapshot_content'
                : '';
              if (entry.lastRawPayloadLineCount > Math.max(4, Number(entry.term.rows || 0) + 4)) {{
                entry.scrollbackExpected = true;
              }}
            }} catch (_error) {{}}
            followPromptForEntry(entry, 'retained_replay_write');
            try {{
              if (typeof entry.term.refresh === "function") {{
                entry.term.refresh(0, Math.max(0, Number(entry.term.rows || 1) - 1));
              }}
            }} catch (_error) {{}}
            try {{
              if (typeof entry.emitHostHealth === "function") {{
                entry.emitHostHealth('retained_replay_write');
              }}
            }} catch (_error) {{}}
            if (Date.now() >= current.stableUntilMs && replayVisibleInEntry(entry) && replayPromptReadyInEntry(entry)) {{
              current.complete = true;
            }} else {{
              retryLater();
            }}
          }};
          attemptReplay();
        }})();
        "#,
        session_path = session_path,
        data = data,
        source = source,
    )
}
fn terminal_set_input_policy_script_for_active_session(
    active_session_path: &str,
    enabled: bool,
    focus: bool,
    foreground_regained: bool,
) -> String {
    format!(
        r#"
        (() => {{
          const activeSessionPath = {active_session_path:?};
          const previousActiveSessionPath = String(window.__yggtermActiveTerminalSessionPath || '');
          const activeSessionChanged = previousActiveSessionPath !== activeSessionPath;
          try {{
            window.__yggtermActiveTerminalSessionPath = activeSessionPath;
          }} catch (_error) {{}}
          for (const host of Array.from(document.querySelectorAll('[id^="yggterm-terminal-"][data-terminal-session-path]'))) {{
            try {{
              const isActiveHost =
                String(host.getAttribute('data-terminal-session-path') || '') === activeSessionPath;
              host.setAttribute('data-active-session-host', isActiveHost ? 'true' : 'false');
            }} catch (_error) {{}}
          }}
          const repaintActiveEntry = (entry, reason, heavy) => {{
            try {{
              if (!entry || !entry.term) {{
                return;
              }}
              // XTERM-BUG: scrollback-lost-on-session-switch
              // See docs/xterm-bugs.md#scrollback-lost-on-session-switch
              // forcePromptFollow scrolls live cursor into view. We MUST NOT
              // call it when the user is actively scrolled back, otherwise a
              // session switch yanks them out of their scroll position.
              if (
                typeof entry.forcePromptFollow === "function"
                && String(entry.scrollbackIntent || 'PromptFollow') !== 'UserScrollback'
              ) {{
                entry.forcePromptFollow(reason);
              }}
              if (heavy && typeof entry.redrawTerminal === "function") {{
                entry.redrawTerminal(reason);
              }} else {{
                if (typeof entry.term.clearTextureAtlas === "function") {{
                  entry.term.clearTextureAtlas();
                  entry.lastAtlasClearAtMs = Date.now();
                }}
                if (typeof entry.term.refresh === "function") {{
                  entry.term.refresh(0, Math.max(0, Number(entry.term.rows || 1) - 1));
                }}
                if (typeof entry.emitHostHealth === "function") {{
                  entry.emitHostHealth(reason);
                }}
              }}
            }} catch (_error) {{}}
          }};
          const scheduleActivationRepaint = (entry, reason) => {{
            try {{
              if (!entry || !entry.term) {{
                return;
              }}
              const now = Date.now();
              const hostId = String(entry.hostId || "");
              const mountedAt = String(entry.mountedAt || "");
              // When this host was switched INTO — stamped before the repaint
              // dedupe below, because it dates the ATTACH, not the repaint. The
              // write bridge asks it to tell the daemon's catch-up burst (which
              // may replay an old OSC 52) from ordinary live output, so a
              // stamp skipped as a "duplicate repaint" would leave a real
              // catch-up looking live.
              entry.lastActivationAtMs = now;
              const repaintKey = `${{activeSessionPath}}:${{hostId}}:${{mountedAt}}:${{reason}}`;
              if (
                entry.lastActivationRepaintKey === repaintKey
                && now - Number(entry.lastActivationRepaintAtMs || 0) < 1200
              ) {{
                return;
              }}
              entry.lastActivationRepaintKey = repaintKey;
              entry.lastActivationRepaintAtMs = now;
              entry.lastActivationRepaintReason = String(reason || "active_session_switch");
              entry.activationRepaintCount = Number(entry.activationRepaintCount || 0) + 1;
              repaintActiveEntry(entry, `${{reason}}:now`, true);
              window.requestAnimationFrame(() => repaintActiveEntry(entry, `${{reason}}:raf`, false));
              window.setTimeout(() => repaintActiveEntry(entry, `${{reason}}:120ms`, false), 120);
              window.setTimeout(() => repaintActiveEntry(entry, `${{reason}}:360ms`, false), 360);
              // XTERM-BUG: webgl-stale-cursor-on-cold-reveal — these fixed timers
              // (<=360ms) fire BEFORE a cold remote session's live daemon content
              // streams in, so none of them clears the atlas over the live frame.
              // Arm a one-shot the write path consumes when the live content
              // actually lands, forcing one clean redraw then. Only for sources
              // not yet live (shadow/retained); a warm session is already painted
              // by the :now redraw above.
              if (String(entry.terminalContentSource || '') !== 'daemon_pty') {{
                entry.pendingRevealDaemonRepaint = true;
                entry.pendingRevealDaemonRepaintUntilMs = now + 8000;
              }}
            }} catch (_error) {{}}
          }};
          const registry = window.__yggtermXtermHosts || {{}};
          for (const entry of Object.values(registry)) {{
            if (!entry || typeof entry.setInputEnabled !== "function") {{
              continue;
            }}
            const isActive = String(entry.sessionPath || '') === activeSessionPath;
            try {{
              entry.setInputEnabled(
                isActive ? {enabled} : false,
                isActive ? {focus} : false,
                true,
                'rust_policy'
              );
            }} catch (_error) {{}}
            // WebGL glyph-atlas heal on FOREGROUND: scheduleActivationRepaint
            // (atlas-clear + repaint at now/raf/120/360ms) already heals switch-in,
            // but it only ran on a session SWITCH. A pure background->foreground
            // (same session, no switch) got no activation repaint, so the stale
            // WebGL atlas (rAF throttled while unfocused) painted wrong-glyph garble
            // that only self-healed ~1s later. Foregrounding re-runs this input
            // policy script (window_focused is in the signature), so fire the SAME
            // proven repaint on foreground regain too.
            if (isActive && (activeSessionChanged || {foreground_regained})) {{
              scheduleActivationRepaint(
                entry,
                activeSessionChanged ? "active_session_switch" : "window_foreground"
              );
            }}
          }}
        }})();
        "#,
        active_session_path = active_session_path,
        enabled = if enabled { "true" } else { "false" },
        focus = if focus { "true" } else { "false" },
        foreground_regained = if foreground_regained { "true" } else { "false" },
    )
}
fn terminal_clear_input_policy_script() -> String {
    r#"
        (() => {
          try {
            window.__yggtermActiveTerminalSessionPath = "";
          } catch (_error) {}
          for (const host of Array.from(document.querySelectorAll('[id^="yggterm-terminal-"][data-terminal-session-path]'))) {
            try {
              host.setAttribute('data-active-session-host', 'false');
            } catch (_error) {}
          }
          const registry = window.__yggtermXtermHosts || {};
          for (const entry of Object.values(registry)) {
            if (!entry || typeof entry.setInputEnabled !== "function") {
              continue;
            }
            try {
              entry.setInputEnabled(false, false, true, 'rust_policy');
            } catch (_error) {}
          }
        })();
    "#
    .to_string()
}
/// The chrome regions that OWN keyboard focus while the user is inside them.
/// Whenever the active element is within one of these, the terminal must NOT
/// reclaim focus.
///
/// ONE list, shared by every focus-arbitration script. There used to be two
/// hand-rolled copies that had already drifted (one lacked the web picker), and
/// NEITHER knew about the document surface — so a terminal focus-reclaim yanked
/// focus straight out of a yedit editor mid-keystroke, which is the
/// "focus is stolen, spam-click to type" bug. A second copy of this list is a
/// bug waiting to happen; add regions here, never at a call site.
const UI_FOCUS_OWNER_SELECTORS: &[&str] = &[
    "[data-yggterm-titlebar-search=\"1\"]",
    "[data-yggui-side-rail=\"1\"]",
    "[data-theme-editor-overlay=\"1\"]",
    "[data-theme-editor-shell=\"1\"]",
    "[data-yggterm-web-picker=\"1\"]",
    "#yggterm-sidebar",
    // A document surface (yedit's editor) owns the viewport, and with it the
    // keyboard: the terminal is not even on screen behind it.
    "[data-document-surface]",
];

/// [`UI_FOCUS_OWNER_SELECTORS`] as a JS array literal, for embedding in a
/// focus-arbitration script.
fn ui_focus_owner_selectors_js() -> String {
    let items: Vec<String> = UI_FOCUS_OWNER_SELECTORS
        .iter()
        .map(|selector| format!("{selector:?}"))
        .collect();
    format!("[{}]", items.join(","))
}

/// The shell root's click handler refocuses the active terminal so a click on
/// dead chrome puts the keyboard back where the user expects it.
///
/// It is a focus-arbitration script like the reclaim and the host guard, and it
/// must honour the same [`UI_FOCUS_OWNER_SELECTORS`] — for a long time it did
/// not, and because it is a plain click handler rather than something named
/// "focus", it survived three rounds of fixes aimed at the other paths. Live
/// trace that convicted it (guihost, 2026-07-24): a real pointer click into yedit's
/// editor moved focus there, and ~93 ms later the helper textarea took it back
/// from a top-level eval with an empty call stack — no registry closure on it,
/// which is exactly this `document::eval`.
///
/// The Rust caller already declines for a covering surface; this guard is the
/// belt, and it covers the rest of the chrome too (a click landing in the
/// sidebar, the theme editor or a settings field used to be yanked away here as
/// well).
fn root_click_terminal_focus_script(session_path_literal: &str) -> String {
    let ui_focus_owners = ui_focus_owner_selectors_js();
    format!(
        "(function() {{
            const active = document.activeElement;
            if (active && active.closest && {ui_focus_owners}.some((sel) => active.closest(sel))) {{
                return;
            }}
            const sessionPath = {session_path_literal};
            const registry = window.__yggtermXtermHosts || {{}};
            const entries = Object.values(registry)
                .filter((entry) => entry && entry.sessionPath === sessionPath)
                .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
            const visible = entries.find((entry) => {{
                const host = entry && entry.hostId ? document.getElementById(entry.hostId) : null;
                if (!host) {{
                    return false;
                }}
                const rect = host.getBoundingClientRect();
                const style = window.getComputedStyle(host);
                return rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
            }}) || entries[0] || null;
            if (!visible || !visible.hostId) {{
                return;
            }}
            const host = document.getElementById(visible.hostId);
            if (host && String(host.getAttribute('data-document-surface-owns-viewport') || '').trim() === 'true') {{
                return;
            }}
            const helperTextarea = host ? host.querySelector('.xterm-helper-textarea') : null;
            try {{
                if (helperTextarea && helperTextarea.focus) {{
                    helperTextarea.focus({{ preventScroll: true }});
                }} else if (host && host.focus) {{
                    host.focus({{ preventScroll: true }});
                }}
            }} catch (_error) {{}}
        }})();"
    )
}

fn terminal_reclaim_focus_script_for_session(session_path: &str) -> String {
    let ui_focus_owners = ui_focus_owner_selectors_js();
    format!(
        r#"
        (() => {{
          const sessionPath = {session_path:?};
          try {{
            window.__yggtermActiveTerminalSessionPath = sessionPath;
          }} catch (_error) {{}}
          for (const host of Array.from(document.querySelectorAll('[id^="yggterm-terminal-"][data-terminal-session-path]'))) {{
            try {{
              const isActiveHost =
                String(host.getAttribute('data-terminal-session-path') || '') === sessionPath;
              host.setAttribute('data-active-session-host', isActiveHost ? 'true' : 'false');
            }} catch (_error) {{}}
          }}
          const uiOwnsFocus = () => {{
            try {{
              const active = document.activeElement;
              if (!active) {{
                return false;
              }}
              const settingsFieldKey = active.getAttribute
                ? String(active.getAttribute('data-settings-field-key') || '')
                : '';
              if (active.id === {search_input_id:?}) {{
                return true;
              }}
              if (
                (active.closest && {ui_focus_owners}.some((sel) => active.closest(sel)))
                || Boolean(settingsFieldKey)
              ) {{
                return true;
              }}
              return false;
            }} catch (_error) {{
              return false;
            }}
          }};
          const registry = window.__yggtermXtermHosts || {{}};
          const entries = Object.values(registry)
            .filter((entry) => entry && entry.sessionPath === sessionPath && typeof entry.setInputEnabled === "function")
            .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
          const focusEntry = (entry) => {{
            if (!entry) {{
              return;
            }}
            if (uiOwnsFocus()) {{
              return;
            }}
            try {{
              entry.setInputEnabled(true, true);
            }} catch (_error) {{}}
            try {{
              if (typeof entry.focusTerminal === "function") {{
                entry.focusTerminal();
              }}
            }} catch (_error) {{}}
            try {{
              if (entry.term && typeof entry.term.focus === "function") {{
                entry.term.focus();
              }}
            }} catch (_error) {{}}
            try {{
              const helperTextarea = entry.host && entry.host.querySelector
                ? entry.host.querySelector('.xterm-helper-textarea')
                : null;
              if (helperTextarea && typeof helperTextarea.focus === "function") {{
                helperTextarea.focus({{ preventScroll: true }});
                if (typeof helperTextarea.setSelectionRange === "function") {{
                  const valueLength = Number(helperTextarea.value ? helperTextarea.value.length : 0);
                  helperTextarea.setSelectionRange(valueLength, valueLength);
                }}
              }}
            }} catch (_error) {{}}
          }};
          for (const entry of entries) {{
            focusEntry(entry);
            window.requestAnimationFrame(() => focusEntry(entry));
            window.setTimeout(() => focusEntry(entry), 0);
            window.setTimeout(() => focusEntry(entry), 32);
            window.setTimeout(() => focusEntry(entry), 96);
            window.setTimeout(() => focusEntry(entry), 220);
            window.setTimeout(() => focusEntry(entry), 360);
            window.setTimeout(() => focusEntry(entry), 760);
            window.setTimeout(() => focusEntry(entry), 980);
          }}
        }})();
        "#,
        session_path = session_path,
        search_input_id = SEARCH_INPUT_ID,
    )
}
fn terminal_apply_script_for_session(session_path: &str, theme: &TerminalTheme) -> String {
    let session_path =
        serde_json::to_string(session_path).expect("serialize terminal session path");
    let background =
        serde_json::to_string(&theme.background).expect("serialize terminal background");
    let foreground =
        serde_json::to_string(&theme.foreground).expect("serialize terminal foreground");
    let cursor = serde_json::to_string(&theme.cursor).expect("serialize terminal cursor");
    let selection = serde_json::to_string(&theme.selection).expect("serialize terminal selection");
    let black = serde_json::to_string(&theme.black).expect("serialize terminal black");
    let red = serde_json::to_string(&theme.red).expect("serialize terminal red");
    let green = serde_json::to_string(&theme.green).expect("serialize terminal green");
    let yellow = serde_json::to_string(&theme.yellow).expect("serialize terminal yellow");
    let blue = serde_json::to_string(&theme.blue).expect("serialize terminal blue");
    let magenta = serde_json::to_string(&theme.magenta).expect("serialize terminal magenta");
    let cyan = serde_json::to_string(&theme.cyan).expect("serialize terminal cyan");
    let white = serde_json::to_string(&theme.white).expect("serialize terminal white");
    let bright_black =
        serde_json::to_string(&theme.bright_black).expect("serialize terminal bright black");
    let bright_red =
        serde_json::to_string(&theme.bright_red).expect("serialize terminal bright red");
    let bright_green =
        serde_json::to_string(&theme.bright_green).expect("serialize terminal bright green");
    let bright_yellow =
        serde_json::to_string(&theme.bright_yellow).expect("serialize terminal bright yellow");
    let bright_blue =
        serde_json::to_string(&theme.bright_blue).expect("serialize terminal bright blue");
    let bright_magenta =
        serde_json::to_string(&theme.bright_magenta).expect("serialize terminal bright magenta");
    let bright_cyan =
        serde_json::to_string(&theme.bright_cyan).expect("serialize terminal bright cyan");
    let bright_white =
        serde_json::to_string(&theme.bright_white).expect("serialize terminal bright white");
    let font_family =
        serde_json::to_string(TERMINAL_FONT_FAMILY).expect("serialize terminal font family");
    let font_weight = serde_json::to_string(&terminal_font_weight(theme))
        .expect("serialize terminal font weight");
    let font_weight_bold = serde_json::to_string(&terminal_font_weight_bold(theme))
        .expect("serialize terminal bold font weight");
    let line_height = terminal_font_line_height(theme);
    let dim_foreground = serde_json::to_string(&terminal_dim_foreground(theme))
        .expect("serialize terminal dim foreground");
    let cursor_muted = serde_json::to_string(&terminal_cursor_muted(theme))
        .expect("serialize terminal muted cursor");
    let cursor_text = serde_json::to_string(&terminal_cursor_text(theme))
        .expect("serialize terminal cursor text");
    let input_line_background = serde_json::to_string(&terminal_input_line_background(theme))
        .expect("serialize terminal input line background");
    let input_line_border = serde_json::to_string(&terminal_input_line_border(theme))
        .expect("serialize terminal input line border");
    let minimum_contrast_ratio = terminal_minimum_contrast_ratio(theme);
    let font_smoothing = serde_json::to_string(terminal_font_smoothing(theme))
        .expect("serialize terminal font smoothing");
    let moz_font_smoothing = serde_json::to_string(terminal_moz_font_smoothing(theme))
        .expect("serialize terminal moz font smoothing");
    format!(
        r#"
        (() => {{
          const sessionPath = {session_path};
          const registry = window.__yggtermXtermHosts || {{}};
          const entries = Object.values(registry)
            .filter((entry) => entry && entry.term && entry.sessionPath === sessionPath)
            .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
          const entry = entries[0];
          if (!entry || !entry.term) {{
            return;
          }}
          const nextTheme = {{
            background: {background},
            foreground: {foreground},
            cursor: {cursor},
            cursorAccent: {cursor_text},
            selectionBackground: {selection},
            black: {black},
            red: {red},
            green: {green},
            yellow: {yellow},
            blue: {blue},
            magenta: {magenta},
            cyan: {cyan},
            white: {white},
            brightBlack: {bright_black},
            brightRed: {bright_red},
            brightGreen: {bright_green},
            brightYellow: {bright_yellow},
            brightBlue: {bright_blue},
            brightMagenta: {bright_magenta},
            brightCyan: {bright_cyan},
            brightWhite: {bright_white},
          }};
          try {{
            entry.term.options = {{
              ...entry.term.options,
              cursorBlink: false,
              cursorInactiveStyle: 'block',
              cursorStyle: 'block',
              fontFamily: {font_family},
              fontSize: {font_size},
              fontWeight: {font_weight},
              fontWeightBold: {font_weight_bold},
              lineHeight: {line_height},
              letterSpacing: 0,
              minimumContrastRatio: {minimum_contrast_ratio},
              theme: nextTheme,
            }};
          }} catch (_error) {{
            entry.term.options.fontFamily = {font_family};
            entry.term.options.cursorBlink = false;
            entry.term.options.cursorInactiveStyle = 'block';
            entry.term.options.cursorStyle = 'block';
            entry.term.options.fontSize = {font_size};
            entry.term.options.fontWeight = {font_weight};
            entry.term.options.fontWeightBold = {font_weight_bold};
            entry.term.options.lineHeight = {line_height};
            entry.term.options.letterSpacing = 0;
            entry.term.options.minimumContrastRatio = {minimum_contrast_ratio};
            entry.term.options.theme = nextTheme;
          }}
          if (entry.host) {{
            entry.host.style.setProperty('--yggterm-term-font-family', {font_family});
            entry.host.style.setProperty('--yggterm-term-font-weight', String({font_weight}));
            entry.host.style.setProperty('--yggterm-term-font-weight-bold', String({font_weight_bold}));
            entry.host.style.setProperty('--yggterm-term-line-height', String({line_height}));
            entry.host.style.setProperty('--yggterm-term-letter-spacing', '0px');
            entry.host.style.setProperty('--yggterm-term-foreground', {foreground});
            entry.host.style.setProperty('--yggterm-term-dim-foreground', {dim_foreground});
            entry.host.style.setProperty('--yggterm-term-cursor', {cursor});
            entry.host.style.setProperty('--yggterm-term-cursor-muted', {cursor_muted});
            entry.host.style.setProperty('--yggterm-term-cursor-text', {cursor_text});
            entry.host.style.setProperty('--yggterm-term-cursor-block-text', {cursor_text});
            entry.host.style.setProperty('--yggterm-term-input-line-background', {input_line_background});
            entry.host.style.setProperty('--yggterm-term-input-line-border', {input_line_border});
            entry.host.style.setProperty('--yggterm-term-font-smoothing', {font_smoothing});
            entry.host.style.setProperty('--yggterm-term-moz-font-smoothing', {moz_font_smoothing});
            entry.host.style.webkitFontSmoothing = {font_smoothing};
            entry.host.style.MozOsxFontSmoothing = {moz_font_smoothing};
          }}
          try {{
            if (entry.refreshCursorContrastContract) {{
              entry.refreshCursorContrastContract();
            }}
          }} catch (_error) {{}}
          try {{
            if (entry.emitResize) {{
              entry.emitResize();
            }} else if (entry.fitAddon) {{
              entry.fitAddon.fit();
            }}
          }} catch (_error) {{}}
          try {{
            if (typeof entry.term.clearTextureAtlas === 'function') {{
              entry.term.clearTextureAtlas();
              entry.lastAtlasClearAtMs = Date.now();
            }}
          }} catch (_error) {{}}
          try {{
            if (entry.term.refresh) {{
              entry.term.refresh(0, Math.max(0, entry.term.rows - 1));
            }}
          }} catch (_error) {{}}
          window.__yggtermLastApply = {{
            hostId: entry.hostId || "unknown",
            sessionPath,
            fontSize: entry.term.options.fontSize,
            appliedAt: Date.now(),
          }};
        }})();
        "#,
        session_path = session_path,
        font_size = theme.font_size,
        background = background,
        foreground = foreground,
        cursor = cursor,
        selection = selection,
        black = black,
        red = red,
        green = green,
        yellow = yellow,
        blue = blue,
        magenta = magenta,
        cyan = cyan,
        white = white,
        bright_black = bright_black,
        bright_red = bright_red,
        bright_green = bright_green,
        bright_yellow = bright_yellow,
        bright_blue = bright_blue,
        bright_magenta = bright_magenta,
        bright_cyan = bright_cyan,
        bright_white = bright_white,
        font_family = font_family,
        font_weight = font_weight,
        font_weight_bold = font_weight_bold,
        line_height = line_height,
        dim_foreground = dim_foreground,
        cursor_muted = cursor_muted,
        input_line_background = input_line_background,
        input_line_border = input_line_border,
        minimum_contrast_ratio = minimum_contrast_ratio,
        cursor_text = cursor_text,
        font_smoothing = font_smoothing,
        moz_font_smoothing = moz_font_smoothing,
    )
}
fn terminal_scroll_to_line_script(session_path: &str, line_index: usize) -> String {
    format!(
        r#"
        (() => {{
            const sessionPath = {session_path:?};
            const registry = window.__yggtermXtermHosts || {{}};
            const entries = Object.values(registry)
                .filter((entry) => entry && entry.term && entry.sessionPath === sessionPath)
                .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
            const entry = entries[0];
            if (!entry || !entry.term) return;
            const target = Math.max(0, {line_index} - Math.floor((entry.term.rows || 1) / 2));
            try {{ entry.term.scrollToLine(target); }} catch (_error) {{}}
        }})();
        "#
    )
}
fn terminal_scroll_control_script(session_path: &str, action: &'static str) -> String {
    format!(
        r#"
        (() => {{
            const sessionPath = {session_path:?};
            const action = {action:?};
            const registry = window.__yggtermXtermHosts || {{}};
            const entries = Object.values(registry)
                .filter((entry) => entry && entry.term && entry.sessionPath === sessionPath)
                .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0));
            const entry = entries[0];
            if (!entry || !entry.term || !entry.term.buffer || !entry.term.buffer.active) return;
            const term = entry.term;
            const active = term.buffer.active;
            const viewportY = Math.max(0, Number(active.viewportY || 0));
            const baseY = Math.max(0, Number(active.baseY || 0));
            const rows = Math.max(1, Number(term.rows || 0));
            const page = Math.max(1, rows - 2);
            let target = viewportY;
            if (action === 'top') {{
                target = 0;
            }} else if (action === 'page_up') {{
                target = Math.max(0, viewportY - page);
            }} else if (action === 'page_down') {{
                target = Math.min(baseY, viewportY + page);
            }} else if (action === 'bottom') {{
                target = baseY;
            }} else {{
                return;
            }}
            try {{
                if (target >= baseY) {{
                    if (entry.forcePromptFollow) {{
                        entry.forcePromptFollow(`scroll_controller:${{action}}`);
                    }} else if (term.scrollToBottom) {{
                        term.scrollToBottom();
                    }} else if (term.scrollToLine) {{
                        term.scrollToLine(baseY);
                    }}
                }} else {{
                    if (entry.forceXtermViewportY) {{
                        entry.forceXtermViewportY(target, `scroll_controller:${{action}}`);
                    }} else if (term.scrollToLine) {{
                        term.scrollToLine(target);
                    }}
                    entry.scrollbackIntent = 'UserScrollback';
                    entry.scrollbackLocked = true;
                    entry.lastScrollbackIntentReason = `scroll_controller:${{action}}`;
                    entry.lastScrollbackIntentAtMs = Date.now();
                }}
                entry.lastScrollControllerAction = action;
                entry.lastScrollControllerActionAtMs = Date.now();
                if (entry.focusTerminal) {{
                    entry.focusTerminal();
                }}
            }} catch (_error) {{}}
        }})();
        "#
    )
}
