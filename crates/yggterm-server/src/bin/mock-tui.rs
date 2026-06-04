//! mock-tui — a deterministic TUI byte source for yggterm integration tests.
//!
//! The yggterm server spawns this as a session's PTY process (in place of
//! codex/Claude Code/a shell) so the read/replay/recovery pipeline can be tested
//! reproducibly. It emits a scripted sequence of bytes/escape codes, flushes, and
//! holds the PTY open so the session stays "running" for the test to read.
//!
//! See docs/integration-testing.md.
//!
//! Usage:
//!   mock-tui --scenario alt-screen [--hold-ms 3000]
//!   mock-tui --scenario normal-scrollback --rows 50
//!   mock-tui --scenario clear-storm --count 20
//!   mock-tui --scenario burst --kb 256
//!   mock-tui --scenario prompt-box
//!   mock-tui --replay <path-to-bytes-fixture>

use std::io::{self, Read, Write};
use std::{env, fs, thread, time::Duration};

fn main() {
    let args: Vec<String> = env::args().collect();
    let out = io::stdout();
    let mut w = out.lock();

    if let Some(path) = arg_value(&args, "--replay") {
        // Trace replay: emit a recorded real PTY byte stream verbatim.
        if let Ok(bytes) = fs::read(&path) {
            let _ = w.write_all(&bytes);
        }
        let _ = w.flush();
        hold(&args);
        return;
    }

    let scenario = arg_value(&args, "--scenario").unwrap_or_else(|| "prompt".to_string());
    match scenario.as_str() {
        // Full-screen TUI: switch to the alternate screen and draw. base_y stays 0;
        // no scrollback. Exercises the alt-screen-aware paths.
        "alt-screen" => {
            let _ = write!(
                w,
                "\x1b[?1049h\x1b[2J\x1b[HALT_SCREEN_MARKER row 1\r\nALT_SCREEN row 2\r\n> alt prompt"
            );
        }
        // Alt screen, then exit back to the normal buffer (restores prior screen).
        "alt-screen-exit" => {
            let _ = write!(w, "\x1b[?1049h\x1b[2J\x1b[HALT_SCREEN_MARKER\r\n");
            let _ = w.flush();
            thread::sleep(Duration::from_millis(80));
            let _ = write!(w, "\x1b[?1049l");
        }
        // Normal buffer that accumulates scrollback (base_y grows).
        //
        // `--paced-ms N` flushes each line separately (with an N ms gap), so the
        // daemon reader thread captures one chunk per line — needed to drive the
        // chunk ring past MAX_CHUNKS deterministically (every byte uniquely labeled
        // NORMAL_LINE_XXXX so a silent mid-stream trim/gap is detectable).
        "normal-scrollback" => {
            let rows: usize = arg_value(&args, "--rows")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50);
            let paced: u64 = arg_value(&args, "--paced-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            for i in 0..rows {
                let _ = write!(w, "NORMAL_LINE_{i:04}\r\n");
                if paced > 0 {
                    let _ = w.flush();
                    thread::sleep(Duration::from_millis(paced));
                }
            }
        }
        // Repeated clear-screen+home then content — the transient-empty / mid-redraw
        // pattern that tripped empty-surface and non-prompt recovery false-positives.
        "clear-storm" => {
            let count: usize = arg_value(&args, "--count")
                .and_then(|s| s.parse().ok())
                .unwrap_or(20);
            let paced: u64 = arg_value(&args, "--paced-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            for i in 0..count {
                let _ = write!(w, "\x1b[2J\x1b[HCLEAR_FRAME_{i:04} working...");
                let _ = w.flush();
                if paced > 0 {
                    thread::sleep(Duration::from_millis(paced));
                }
            }
        }
        // High-volume output to exercise the chunk-ring trim path.
        "burst" => {
            let kb: usize = arg_value(&args, "--kb")
                .and_then(|s| s.parse().ok())
                .unwrap_or(256);
            let line = "X".repeat(120);
            let mut emitted = 0usize;
            let mut n = 0usize;
            while emitted < kb * 1024 {
                let _ = write!(w, "BURST_{n:06} {line}\r\n");
                emitted += line.len() + 16;
                n += 1;
            }
        }
        // Codex-style bordered prompt box in the alternate screen.
        "prompt-box" => {
            let _ = write!(
                w,
                "\x1b[?1049h\x1b[2J\x1b[H\
                 \u{256d}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256e}\r\n\
                 \u{2502} > PROMPT_BOX \u{2502}\r\n\
                 \u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256f}\r\n\
                 gpt-mock \u{00b7} ~/mock"
            );
        }
        // Plain shell-ish prompt (normal buffer, minimal output).
        _ => {
            let _ = write!(w, "$ MOCK_TUI_PROMPT\r\n$ ");
        }
    }
    let _ = w.flush();

    // Drain stdin in the background so a PTY write from the daemon never blocks us.
    thread::spawn(|| {
        let mut buf = [0u8; 1024];
        let mut stdin = io::stdin();
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
    });

    hold(&args);
}

fn hold(args: &[String]) {
    let hold_ms: u64 = arg_value(args, "--hold-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    thread::sleep(Duration::from_millis(hold_ms));
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}
