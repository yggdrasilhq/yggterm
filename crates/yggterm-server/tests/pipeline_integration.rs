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
