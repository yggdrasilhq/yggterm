//! End-to-end daemon-pipeline integration tests (Phase A of docs/integration-testing.md).
//!
//! These spawn the real `mock-tui` binary as a session PTY through the real
//! `TerminalManager`, then drive `read()` and assert on the bytes the client would
//! receive. Deterministic, no GUI, no network. The seam is the PTY source
//! (`mock-tui` in place of codex/CC/shell).

use std::time::{Duration, Instant};

use yggterm_server::TerminalManager;

const MOCK_TUI: &str = env!("CARGO_BIN_EXE_mock-tui");

fn launch(scenario_args: &str) -> String {
    format!("{MOCK_TUI} {scenario_args}")
}

/// Wait until the session has produced output (the reader thread captured bytes),
/// then give it a beat to drain the full scripted emission.
fn wait_for_output(mgr: &TerminalManager, key: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if mgr.session_has_output(key) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // Let the remaining scripted bytes flush through the PTY + reader thread.
    std::thread::sleep(Duration::from_millis(300));
}

fn read_from_zero(mgr: &TerminalManager, key: &str) -> String {
    let result = mgr.read(key, 0).expect("read should succeed");
    result
        .chunks
        .iter()
        .map(|chunk| chunk.data.as_str())
        .collect::<String>()
}

/// Poll `read(0)` until the accumulated output contains `needle` (or time out).
fn wait_for_text(mgr: &TerminalManager, key: &str, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut data = String::new();
    while Instant::now() < deadline {
        data = read_from_zero(mgr, key);
        if data.contains(needle) {
            return data;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    data
}

// ---- Agent session control (programmatic prompt insertion) --------------------
// These exercise the feature behind yggterm's automation (timers + programmatic
// prompt insertion): an agent writes input to a session and the program receives
// it and responds. The seam is the same `TerminalManager::write` -> PTY -> program
// -> `read` round-trip the live `server app terminal send` uses.

#[test]
fn agent_inserted_prompt_reaches_the_program_and_produces_output() {
    let mut mgr = TerminalManager::new();
    let key = "test://drive-echo";
    mgr.ensure_session(key, &launch("--scenario echo --hold-ms 8000"), None)
        .expect("ensure_session");
    // Wait for the program to be ready to read input.
    assert!(
        !wait_for_text(&mgr, key, "MOCK_ECHO_READY", Duration::from_secs(5)).is_empty(),
        "echo program should announce readiness"
    );
    // Insert a prompt exactly as the automation / `terminal send` path does.
    mgr.write(key, "What is the status now?\r")
        .expect("write prompt");
    let data = wait_for_text(&mgr, key, "ECHO: What is the status now?", Duration::from_secs(5));
    assert!(
        data.contains("ECHO: What is the status now?"),
        "the inserted prompt must reach the program and round-trip back; got tail {:?}",
        &data[data.len().saturating_sub(200)..]
    );
}

#[test]
fn submit_prompt_waits_for_readiness_then_delivers() {
    use yggterm_server::PromptSubmitOutcome;
    let mut mgr = TerminalManager::new();
    let key = "test://submit-ready";
    // Busy for ~1.5s, then shows a ready `>` input row, then echoes.
    mgr.ensure_session(
        key,
        &launch("--scenario delayed-prompt --ready-after-ms 1500 --hold-ms 9000"),
        None,
    )
    .expect("ensure_session");
    // Readiness policy (injected): the codex-style input row marker, absent while busy.
    let is_ready = |screen: &str| screen.contains('\u{203a}');
    let outcome = mgr
        .submit_prompt(key, "status?\r", is_ready, Duration::from_secs(6))
        .expect("submit_prompt");
    match outcome {
        PromptSubmitOutcome::Submitted { waited_ms } => assert!(
            waited_ms >= 1000,
            "submit must WAIT through the busy phase before delivering, waited {waited_ms}ms"
        ),
        other => panic!("expected Submitted after readiness, got {other:?}"),
    }
    // The prompt, delivered only once ready, must round-trip through the echo.
    let data = wait_for_text(&mgr, key, "ECHO: status?", Duration::from_secs(5));
    assert!(
        data.contains("ECHO: status?"),
        "the readiness-gated prompt must reach the program once it is ready; tail {:?}",
        &data[data.len().saturating_sub(160)..]
    );
}

#[test]
fn echo_verified_submit_waits_until_input_is_actually_consumed() {
    use yggterm_server::PromptSubmitOutcome;
    let mut mgr = TerminalManager::new();
    let key = "test://echo-verified";
    // A composer that DRAWS its prompt immediately but IGNORES input for 2s — exactly
    // the root-cause bug (prompt shown before the input loop is live). A
    // display-only readiness check would fire too early; echo-verification must wait.
    mgr.ensure_session(
        key,
        &launch("--scenario composer --ready-after-ms 2000 --hold-ms 12000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let outcome = mgr
        .submit_prompt_echo_verified(key, "real prompt now\r", Duration::from_secs(8))
        .expect("submit_prompt_echo_verified");
    match outcome {
        PromptSubmitOutcome::Submitted { waited_ms } => assert!(
            waited_ms >= 1500,
            "must WAIT until input is actually consumed (not just prompt shown), waited {waited_ms}ms"
        ),
        other => panic!("expected Submitted once input is consumed, got {other:?}"),
    }
    // The real prompt is delivered only after echo-verified readiness, so the composer
    // actually submits it.
    let data = wait_for_text(&mgr, key, "SUBMITTED: real prompt now", Duration::from_secs(5));
    assert!(
        data.contains("SUBMITTED: real prompt now"),
        "the echo-verified prompt must be the one the composer submits; tail {:?}",
        &data[data.len().saturating_sub(160)..]
    );
}

#[test]
fn echo_verified_submit_refuses_when_input_never_consumed() {
    use yggterm_server::PromptSubmitOutcome;
    let mut mgr = TerminalManager::new();
    let key = "test://echo-verified-never";
    // A composer that NEVER starts reading input (huge ready-after window): the probe
    // never echoes, so the real prompt must never be written.
    mgr.ensure_session(
        key,
        &launch("--scenario composer --ready-after-ms 999999 --hold-ms 8000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let outcome = mgr
        .submit_prompt_echo_verified(key, "must-not-submit\r", Duration::from_millis(1200))
        .expect("submit_prompt_echo_verified");
    assert!(
        matches!(outcome, PromptSubmitOutcome::NotReady { .. }),
        "input never consumed -> must refuse, got {outcome:?}"
    );
    assert!(
        !read_from_zero(&mgr, key).contains("must-not-submit")
            && !read_from_zero(&mgr, key).contains("SUBMITTED"),
        "echo-verified submit must NOT write the real prompt when input is never consumed"
    );
}

#[test]
fn submit_prompt_refuses_and_writes_nothing_when_never_ready() {
    use yggterm_server::PromptSubmitOutcome;
    let mut mgr = TerminalManager::new();
    let key = "test://submit-never";
    // An alt-screen surface that never shows a `>` input row — never "ready".
    mgr.ensure_session(key, &launch("--scenario alt-screen --hold-ms 8000"), None)
        .expect("ensure_session");
    wait_for_output(&mgr, key);
    let is_ready = |screen: &str| screen.contains('\u{203a}');
    let outcome = mgr
        .submit_prompt(key, "should-not-appear\r", is_ready, Duration::from_millis(800))
        .expect("submit_prompt");
    assert!(
        matches!(outcome, PromptSubmitOutcome::NotReady { .. }),
        "a never-ready session must be refused, got {outcome:?}"
    );
    // And crucially: nothing was written into the not-ready surface.
    assert!(
        !read_from_zero(&mgr, key).contains("should-not-appear"),
        "submit_prompt must NOT write into a session that never became ready"
    );
}

#[test]
fn agent_arrow_key_navigation_selects_full_access_in_a_permission_menu() {
    let mut mgr = TerminalManager::new();
    let key = "test://drive-menu";
    // The 3-option selector starts on "Default" (index 0), like codex /permissions.
    mgr.ensure_session(key, &launch("--scenario menu --hold-ms 8000"), None)
        .expect("ensure_session");
    assert!(
        wait_for_text(&mgr, key, "Update Model Permissions", Duration::from_secs(5))
            .contains("Full Access"),
        "menu should render all options"
    );
    // Drive: Down, Down, Enter -> select "Full Access" (the live /permissions flow).
    mgr.write(key, "\x1b[B\x1b[B\r").expect("write arrows + enter");
    let data = wait_for_text(&mgr, key, "SELECTED:", Duration::from_secs(5));
    assert!(
        data.contains("SELECTED: Full Access"),
        "Down x2 + Enter must commit Full Access; got tail {:?}",
        &data[data.len().saturating_sub(200)..]
    );
}

#[test]
fn alt_screen_session_delivers_screen_content() {
    let mut mgr = TerminalManager::new();
    let key = "test://alt-screen";
    mgr.ensure_session(key, &launch("--scenario alt-screen --hold-ms 4000"), None)
        .expect("ensure_session");
    wait_for_output(&mgr, key);
    let data = read_from_zero(&mgr, key);
    assert!(
        data.contains("ALT_SCREEN_MARKER"),
        "alt-screen content must reach the client; got {:?}",
        &data[..data.len().min(300)]
    );
}

#[test]
fn normal_buffer_scrollback_is_retained_end_to_end() {
    let mut mgr = TerminalManager::new();
    let key = "test://scrollback";
    // Emit far more rows than a viewport so the oldest lines scroll off into the
    // daemon's vt100 scrollback ring. The read must still deliver them — that is the
    // tmux-parity scrollback guarantee.
    mgr.ensure_session(
        key,
        &launch("--scenario normal-scrollback --rows 120 --hold-ms 4000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let data = read_from_zero(&mgr, key);
    assert!(
        data.contains("NORMAL_LINE_0119"),
        "latest line must be present; got tail {:?}",
        &data[data.len().saturating_sub(200)..]
    );
    assert!(
        data.contains("NORMAL_LINE_0000"),
        "oldest scrolled-off line must be retained in scrollback (no silent loss)"
    );
}

#[test]
fn scrolling_output_populates_clean_daemon_history_for_xterm_scrollback() {
    // The "scroll-lock" question: a session can only scroll up in the client when
    // the daemon has CLEAN scrolled-off history rows to load into xterm scrollback
    // (so base_y > 0). Genuinely-scrolling output MUST produce that history.
    let mut mgr = TerminalManager::new();
    let key = "test://history-scroll";
    mgr.ensure_session(
        key,
        &launch("--scenario normal-scrollback --rows 120 --hold-ms 4000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let history = mgr
        .session_history_rows(key)
        .expect("session_history_rows for a live session");
    assert!(
        history.len() > 40,
        "scrolling output must leave many clean scrolled-off rows for xterm scrollback; got {}",
        history.len()
    );
    assert!(
        history.iter().any(|line| line.contains("NORMAL_LINE_0000")),
        "the oldest scrolled-off line must be in the clean daemon history"
    );
}

#[test]
fn cursor_addressed_repaint_has_no_clean_scrollback_so_base_y_zero_is_correct() {
    // A cursor-addressed in-place repaint TUI (clear-storm writes \x1b[2J\x1b[H +
    // content each frame, never scrolling — the codex-class rendering pattern)
    // OVERWRITES its viewport instead of scrolling, so nothing enters the vt100
    // scrollback ring. The daemon therefore has ~no clean history to load, and the
    // client correctly reveals with base_y == 0 (no scrollback to scroll into).
    // This codifies that the codex "scroll-lock" is inherent to codex's rendering,
    // NOT a yggterm pipeline bug — fabricating scrollback from the cursor-addressed
    // repaint stream is the known corruption trap (docs/xterm-bugs.md) and must not
    // be attempted here.
    let mut mgr = TerminalManager::new();
    let key = "test://history-repaint";
    mgr.ensure_session(
        key,
        &launch("--scenario clear-storm --count 40 --hold-ms 4000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let history = mgr
        .session_history_rows(key)
        .expect("session_history_rows for a live session");
    // The final frame is in the VISIBLE viewport (not scrollback); the overwritten
    // earlier frames did not scroll off, so clean history is empty/minimal.
    assert!(
        history.len() <= 2,
        "cursor-addressed repaint must NOT fabricate scrollback; got {} rows: {:?}",
        history.len(),
        history
    );
    assert!(
        !history.iter().any(|line| line.contains("CLEAR_FRAME_0000")),
        "overwritten (not scrolled-off) frames must never appear as clean scrollback"
    );
}

#[test]
fn codex_inline_committed_lines_reach_daemon_scrollback() {
    // codex (ratatui inline viewport) prints committed conversation lines with
    // newlines (they scroll up) and repaints only a bottom live region in place via
    // absolute addressing. The user's report: codex scrolls fine in ghostty but is
    // scroll-locked in yggterm. If the daemon vt100 scrollback ring captures the
    // committed scrolled-off lines under THIS pattern, then the gap is in the reveal/
    // load path (fixable), not "codex can't scroll". This test localizes that.
    let mut mgr = TerminalManager::new();
    let key = "test://codex-inline";
    mgr.ensure_session(
        key,
        &launch("--scenario codex-inline --rows 80 --screen-rows 24 --repaints 8 --hold-ms 4000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let history = mgr
        .session_history_rows(key)
        .expect("session_history_rows for a live session");
    let joined = history.join("\n");
    // The committed conversation MUST be in the daemon's clean scrollback — codex
    // emitted these with newlines, they scrolled off, the vt100 ring must retain them.
    assert!(
        history.len() > 40,
        "committed inline-viewport lines must reach daemon scrollback; got {} rows",
        history.len()
    );
    assert!(
        joined.contains("CODEX_MSG_0000"),
        "the oldest committed conversation line must be retained as clean scrollback"
    );
    // The bottom-region repaint content (composer) must NOT pollute scrollback as
    // scrolled-off history (it never scrolled — repainted in place).
    assert!(
        !joined.contains("COMPOSER_FRAME_000"),
        "in-place bottom-region repaints must not appear as scrolled-off history"
    );
}

#[test]
fn codex_reveal_serves_scrollback_history_to_client() {
    // DECISIVE localization for the codex scroll-lock: does the daemon's REVEAL
    // (read(cursor=0), the path a client uses on mount/switch-back) actually SERVE
    // the captured scrollback history to the client, or only the current screen?
    // Use a codex-runtime:// key so the real codex reveal path
    // (prefer_initial_screen_snapshot / history_and_screen_replay) is exercised.
    // If the reveal payload contains the committed scrolled-off lines, the daemon
    // side is correct and the scroll-lock is a CLIENT-side xterm load bug
    // (fixable GUI-only, no daemon restart). If not, the reveal selection is the bug.
    let mut mgr = TerminalManager::new();
    let key = "codex-runtime://test-reveal";
    mgr.ensure_session(
        key,
        &launch("--scenario codex-inline --rows 80 --screen-rows 24 --repaints 8 --hold-ms 4000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let reveal = read_from_zero(&mgr, key);
    assert!(
        reveal.contains("CODEX_MSG_0000"),
        "the daemon REVEAL (read(0)) must serve the oldest committed scrolled-off \
         line to the client — if absent, the reveal drops scrollback; tail {:?}",
        &reveal[reveal.len().saturating_sub(200)..]
    );
    assert!(
        reveal.contains("CODEX_MSG_0079"),
        "the reveal must also include the most recent committed line"
    );
}

#[test]
fn restart_preserves_session_grid_instead_of_default() {
    // Regression lock for the post-restart "squish": re-creating a session's PTY on
    // restart must carry the outgoing grid forward, not drop to the 120x36 default.
    // Otherwise the program (codex) renders narrow inside the real viewport and the
    // client's follow-up resize can be wrongly no-op'd against a stale cache.
    let mut mgr = TerminalManager::new();
    let key = "test://restart-size";
    mgr.ensure_session_with_size(
        key,
        &launch("--scenario echo --hold-ms 4000"),
        None,
        Some((159, 63)),
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    assert_eq!(
        mgr.session_size(key),
        Some((159, 63)),
        "session should start at the requested grid"
    );
    // Restart WITHOUT an explicit size — must preserve 159x63, not fall to 120x36.
    mgr.restart_session(key, &launch("--scenario echo --hold-ms 4000"), None, None)
        .expect("restart_session");
    assert_eq!(
        mgr.session_size(key),
        Some((159, 63)),
        "restart must preserve the session grid, not reset to the 120x36 default"
    );
}

#[test]
fn ensure_session_keeps_existing_grid_so_reattach_must_resize_to_client_grid() {
    // Campaign D1 (squish + bottom-paint bg-split on re-resume): after a daemon
    // restart the successor auto-resumes a session at the DEFAULT grid, and a
    // later client (re)attach calls ensure_session_with_size with the client's
    // REAL grid. This proves the bug PRECONDITION — ensure_session does NOT
    // resize an existing session — and that an explicit resize (what the daemon
    // ensure path now does when the client grid differs) corrects it.
    let mut mgr = TerminalManager::new();
    let key = "test://reattach-grid";
    // Successor auto-resume at the default-ish grid.
    mgr.ensure_session_with_size(key, &launch("--scenario echo --hold-ms 4000"), None, Some((120, 36)))
        .expect("ensure_session");
    wait_for_output(&mgr, key);
    assert_eq!(mgr.session_size(key), Some((120, 36)));
    // Client re-attach passes its real grid, but ensure must NOT resize an
    // already-running session (the squish precondition).
    mgr.ensure_session_with_size(key, &launch("--scenario echo --hold-ms 4000"), None, Some((159, 63)))
        .expect("ensure_session reattach");
    assert_eq!(
        mgr.session_size(key),
        Some((120, 36)),
        "ensure_session_with_size must not resize an existing session — this is why the daemon must resize on reattach"
    );
    // The daemon's reattach-grid-resync: resize to the client grid takes effect.
    mgr.resize(key, 159, 63).expect("resize to client grid");
    assert_eq!(
        mgr.session_size(key),
        Some((159, 63)),
        "reattach resize must bring a stale (squished) PTY to the client's real grid"
    );
}

#[test]
fn working_session_screen_carries_the_idle_gate_interrupt_signal() {
    // The daemon's idle gate (and the disk-binary self-retire deferral) keys off the
    // session screen showing "esc to interrupt" (screen_text_shows_agent_working).
    // Prove the daemon snapshot of an actively-working agent carries that signal — so
    // an update is deferred — while an idle session does not.
    let mut mgr = TerminalManager::new();
    let working = "test://idle-gate-working";
    mgr.ensure_session(working, &launch("--scenario working --hold-ms 6000"), None)
        .expect("ensure working");
    wait_for_output(&mgr, working);
    let working_screen = mgr
        .session_screen_snapshot(working)
        .expect("working session screen");
    assert!(
        working_screen.contains("esc to interrupt"),
        "an actively-working agent screen must carry the 'esc to interrupt' idle-gate \
         signal so updates defer; got tail {:?}",
        &working_screen[working_screen.len().saturating_sub(120)..]
    );

    let idle = "test://idle-gate-idle";
    mgr.ensure_session(idle, &launch("--scenario echo --hold-ms 6000"), None)
        .expect("ensure idle");
    wait_for_output(&mgr, idle);
    let idle_screen = mgr.session_screen_snapshot(idle).unwrap_or_default();
    assert!(
        !idle_screen.contains("esc to interrupt"),
        "an idle session must NOT carry the working signal (update may proceed)"
    );
}

#[test]
fn clear_storm_does_not_corrupt_final_frame() {
    let mut mgr = TerminalManager::new();
    let key = "test://clear-storm";
    mgr.ensure_session(
        key,
        &launch("--scenario clear-storm --count 30 --hold-ms 4000"),
        None,
    )
    .expect("ensure_session");
    wait_for_output(&mgr, key);
    let data = read_from_zero(&mgr, key);
    assert!(
        data.contains("CLEAR_FRAME_0029"),
        "the final frame after a clear-storm must be the visible content"
    );
}

#[test]
fn high_volume_burst_is_delivered_without_panicking_the_pipeline() {
    let mut mgr = TerminalManager::new();
    let key = "test://burst";
    mgr.ensure_session(key, &launch("--scenario burst --kb 512 --hold-ms 4000"), None)
        .expect("ensure_session");
    wait_for_output(&mgr, key);
    let data = read_from_zero(&mgr, key);
    // The most-recent burst lines must be present (the ring may trim the oldest).
    assert!(
        data.contains("BURST_"),
        "burst output must reach the client; got {} bytes",
        data.len()
    );
}

/// Collect every `NORMAL_LINE_NNNN` index present in `data`, in order.
fn normal_line_indices(data: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for token in data.split("NORMAL_LINE_").skip(1) {
        let digits: String = token.chars().take(4).take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() == 4
            && let Ok(n) = digits.parse::<usize>()
        {
            out.push(n);
        }
    }
    out
}

/// A client that reads, then falls behind while output keeps flowing, must not
/// SILENTLY lose the lines the ring trimmed in between: the resumed read from its
/// cursor must either deliver them contiguously OR set `resync_required` so the
/// client re-attaches (recovering the middle from the daemon vt100 scrollback).
/// Before the fix, `read(cursor)` returned the discontiguous surviving tail with no
/// signal and the trimmed middle vanished
/// (docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream).
#[test]
fn read_from_cursor_never_silently_drops_trimmed_middle_chunks() {
    let mut mgr = TerminalManager::new();
    let key = "test://midstream-gap";
    // Emit far more paced lines than the live ring cap (MAX_CHUNKS = 512), one chunk
    // per line, slowly enough that we can read early and then fall behind.
    mgr.ensure_session(
        key,
        &launch("--scenario normal-scrollback --rows 900 --paced-ms 4 --hold-ms 15000"),
        None,
    )
    .expect("ensure_session");

    // Read early to advance the client cursor to a low value (a client that has
    // consumed the first lines), while the ring has not yet trimmed past it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !mgr.session_has_output(key) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let first = mgr.read(key, 0).expect("early read");
    let early_cursor = first.cursor;
    let mut seen = normal_line_indices(
        &first.chunks.iter().map(|c| c.data.as_str()).collect::<String>(),
    );
    let early_max = seen.iter().copied().max().unwrap_or(0);

    // Fall behind: let the remaining lines emit so the ring trims chunks ABOVE the
    // early cursor but BELOW the latest.
    std::thread::sleep(Duration::from_secs(5));

    // Resume reading from where we left off.
    let resumed = mgr.read(key, early_cursor).expect("resumed read");
    seen.extend(normal_line_indices(
        &resumed.chunks.iter().map(|c| c.data.as_str()).collect::<String>(),
    ));
    seen.sort_unstable();
    seen.dedup();
    let latest = seen.last().copied().unwrap_or(0);
    assert!(
        latest > early_max + 100,
        "test setup: output must keep flowing past the early read (early_max={early_max}, latest={latest})"
    );

    // The contract: the resumed read is EITHER gap-free (no missing NORMAL_LINE
    // between the earliest consumed line and the latest delivered), OR it sets
    // resync_required so the client knows to re-attach and recover the trimmed
    // middle from the vt100 scrollback. What must NOT happen is a SILENT hole.
    let earliest = seen.first().copied().unwrap_or(0);
    let missing: Vec<usize> = (earliest..=latest).filter(|n| !seen.contains(n)).collect();
    assert!(
        missing.is_empty() || resumed.resync_required,
        "SILENT mid-stream data loss: {} lines dropped with resync_required=false \
         (e.g. {:?}…) between NORMAL_LINE_{:04} and NORMAL_LINE_{:04}",
        missing.len(),
        &missing[..missing.len().min(5)],
        earliest,
        latest
    );
    // And when a trim-below-cursor actually happened here, the signal must fire (so
    // the bug can't silently regress into "no missing lines because nothing trimmed").
    if !missing.is_empty() {
        assert!(
            resumed.resync_required,
            "ring trimmed below the client cursor but resync_required was not set"
        );
    }
}

// ---- Cold-re-resume signal (vacuum guard, sum-total run #3) --------------------
// The client's vacuum guard keys off the runtime spawn id: a snapshot read from a
// DIFFERENT spawn than the one the client buffer was seeded from means the runtime
// was replaced (cold re-resume) and a sparse fresh-PTY frame must not wipe a richer
// client transcript. Lock the daemon half: the id is stable while a runtime lives,
// and CHANGES when an exited runtime is replaced through the real ensure path.
#[test]
fn runtime_spawn_id_stable_while_running_and_changes_on_replace() {
    let mut mgr = TerminalManager::new();
    let key = "test://spawn-id-replace";
    // Short-lived program: emits and exits quickly (no hold).
    let cmd = launch("--scenario burst --hold-ms 150");
    mgr.ensure_session(key, &cmd, None).expect("ensure_session");
    wait_for_output(&mgr, key);
    let first = mgr.session_runtime_spawn_id(key);
    assert_ne!(first, 0, "a live runtime must report a non-zero spawn id");
    // ensure on a RUNNING session is a no-op: same runtime, same id.
    if mgr.session_is_running(key) {
        mgr.ensure_session(key, &cmd, None).expect("ensure_session noop");
        assert_eq!(
            mgr.session_runtime_spawn_id(key),
            first,
            "ensure on a running session must not change the spawn id"
        );
    }
    // Wait for the program to exit, then ensure again -> replace_exited_runtime.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && mgr.session_is_running(key) {
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(!mgr.session_is_running(key), "mock-tui should have exited");
    mgr.ensure_session(key, &cmd, None)
        .expect("ensure_session replace");
    let second = mgr.session_runtime_spawn_id(key);
    assert_ne!(second, 0, "replaced runtime must report a spawn id");
    assert_ne!(
        second, first,
        "replacing an exited runtime must change the spawn id (the cold-re-resume signal)"
    );
}
