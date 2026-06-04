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
