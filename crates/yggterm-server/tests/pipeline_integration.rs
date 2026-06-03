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
