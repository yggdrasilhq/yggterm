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
        // INTERACTIVE: echo each stdin line back as `ECHO: <line>`. Exercises the
        // full agent-session-control drive loop (write -> PTY -> program -> read):
        // a test sends a prompt and asserts the program received + responded.
        "echo" => {
            let _ = write!(w, "MOCK_ECHO_READY\r\n");
            let _ = w.flush();
            run_echo(&mut w);
            hold(&args);
            return;
        }
        // INTERACTIVE: a 3-option selector (Default / Auto-review / Full Access),
        // navigated with up/down arrows and committed with Enter — the shape of
        // codex's `/permissions` menu. On Enter it prints `SELECTED: <option>` so a
        // test (or the live permission flow) can assert the right item was chosen
        // via arrow-key driving. Exercises escape-sequence input delivery.
        "menu" => {
            run_permission_menu(&mut w, &args);
            hold(&args);
            return;
        }
        // INTERACTIVE: BUSY first, then a ready codex-style prompt, then echo. Models
        // a session that isn't immediately ready (codex generating / starting up):
        // emits `working...` for `--ready-after-ms`, then clears to a `>` input row
        // + model footer (a current-input-row a readiness predicate recognizes), then
        // echoes input. Lets a test prove submit_prompt WAITS for readiness before
        // delivering, and that the delivered prompt then lands.
        "delayed-prompt" => {
            let ready_after: u64 = arg_value(&args, "--ready-after-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1500);
            let _ = write!(w, "working... (esc to interrupt)\r\n");
            let _ = w.flush();
            thread::sleep(Duration::from_millis(ready_after));
            let _ = write!(w, "\x1b[2J\x1b[H\u{203a} \r\n  gpt-mock \u{00b7} ~/mock\r\n");
            let _ = w.flush();
            run_echo(&mut w);
            hold(&args);
            return;
        }
        // INTERACTIVE: a codex-style composer with char-by-char echo, Ctrl+U clear,
        // and Enter submit — PLUS a `--ready-after-ms` window during which input is
        // IGNORED (drained, not echoed). Models the real bug root cause: codex draws
        // its prompt before its input loop is live, so a probe written too early is
        // silently dropped. Lets echo-verified readiness be tested: a probe only
        // echoes once the composer is actually reading.
        "composer" => {
            let ready_after: u64 = arg_value(&args, "--ready-after-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            run_composer(&mut w, ready_after);
            hold(&args);
            return;
        }
        // Codex/ratatui INLINE-VIEWPORT pattern: committed conversation lines are
        // printed once with newlines (they scroll up into the terminal's scrollback),
        // and a fixed bottom live region (composer + status) is repainted IN PLACE via
        // absolute cursor addressing — never scrolling. This mirrors what real codex
        // emits (verified live: 0 newlines in the recent ring, abs-addressing only to
        // the bottom rows). Used to test that scroll-off committed content reaches the
        // daemon vt100 scrollback ring even when interleaved with bottom-region repaints
        // (the "why can't I scroll codex in yggterm" investigation).
        "codex-inline" => {
            let rows: usize = arg_value(&args, "--rows")
                .and_then(|s| s.parse().ok())
                .unwrap_or(80);
            let screen: usize = arg_value(&args, "--screen-rows")
                .and_then(|s| s.parse().ok())
                .unwrap_or(24);
            let repaints: usize = arg_value(&args, "--repaints")
                .and_then(|s| s.parse().ok())
                .unwrap_or(8);
            // Emit committed conversation lines that scroll naturally (newline-driven).
            for i in 0..rows {
                let _ = write!(w, "CODEX_MSG_{i:04} committed conversation line\r\n");
            }
            // Now repaint a 3-row bottom live region in place, many times, WITHOUT
            // scrolling — exactly codex's idle composer/status churn.
            let composer_top = screen.saturating_sub(2);
            for frame in 0..repaints {
                for (offset, label) in ["> ", "model ", "esc "].iter().enumerate() {
                    let row = composer_top + offset;
                    // absolute position + clear-to-EOL + content (no newline)
                    let _ = write!(w, "\x1b[{row};1H\x1b[K{label}COMPOSER_FRAME_{frame:03}");
                }
                let _ = w.flush();
                thread::sleep(Duration::from_millis(20));
            }
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

/// Read stdin line-by-line and echo each back as `ECHO: <line>`. A bare CR or LF
/// commits a line. Runs until stdin closes or the hold window elapses (the caller
/// holds the PTY open afterward).
fn run_echo(w: &mut impl Write) {
    let mut stdin = io::stdin();
    let mut buf = [0u8; 1024];
    let mut line: Vec<u8> = Vec::new();
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 {
            break;
        }
        for &byte in &buf[..n] {
            if byte == b'\r' || byte == b'\n' {
                let text = String::from_utf8_lossy(&line);
                let _ = write!(w, "ECHO: {text}\r\n");
                let _ = w.flush();
                line.clear();
            } else {
                line.push(byte);
            }
        }
    }
}

/// A codex-style composer with char echo + Ctrl+U clear + Enter submit. For the
/// first `ready_after_ms` it IGNORES all input (drains it) — modeling codex drawing
/// its prompt before its input loop is live. After that window it echoes the live
/// buffer as `> <buffer>` on every keystroke, clears on Ctrl+U (0x15), and on CR/LF
/// emits `SUBMITTED: <buffer>` then clears. This is the deterministic stand-in for
/// echo-verified readiness: a probe only shows up once the composer is truly reading.
fn run_composer(w: &mut impl Write, ready_after_ms: u64) {
    use std::time::Instant;
    // Model codex: put the tty in raw / no-echo mode so the LINE DISCIPLINE does not
    // auto-echo input. Only the program's OWN echo (below, once it's reading) shows a
    // keystroke — which is the whole point of echo-verified readiness. Without this
    // the cooked-mode tty echoes the probe even while we're ignoring input, defeating
    // the test (and misrepresenting how codex's raw-mode TUI behaves).
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut termios) == 0 {
            termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG | libc::IEXTEN);
            termios.c_iflag &= !(libc::ICRNL | libc::IXON);
            let _ = libc::tcsetattr(0, libc::TCSANOW, &termios);
        }
    }
    let started = Instant::now();
    let _ = write!(w, "\u{203a} \r\n  gpt-mock \u{00b7} ~/mock\r\n");
    let _ = w.flush();
    let mut stdin = io::stdin();
    let mut buf = [0u8; 1024];
    let mut composed: Vec<u8> = Vec::new();
    let render = |w: &mut dyn Write, composed: &[u8]| {
        let text = String::from_utf8_lossy(composed);
        let _ = write!(w, "\x1b[2J\x1b[H\u{203a} {text}\r\n  gpt-mock \u{00b7} ~/mock\r\n");
        let _ = w.flush();
    };
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 {
            break;
        }
        // Not-reading window: drain input without acting on it.
        if started.elapsed() < Duration::from_millis(ready_after_ms) {
            continue;
        }
        for &byte in &buf[..n] {
            match byte {
                b'\r' | b'\n' => {
                    let text = String::from_utf8_lossy(&composed).to_string();
                    let _ = write!(w, "\x1b[2J\x1b[HSUBMITTED: {text}\r\n\u{203a} \r\n  gpt-mock \u{00b7} ~/mock\r\n");
                    let _ = w.flush();
                    composed.clear();
                }
                0x15 => {
                    composed.clear();
                    render(w, &composed);
                }
                0x7f | 0x08 => {
                    composed.pop();
                    render(w, &composed);
                }
                b if b >= 0x20 => {
                    composed.push(b);
                    render(w, &composed);
                }
                _ => {}
            }
        }
    }
}

/// Render a 3-option permission selector and drive it from stdin arrow keys +
/// Enter, mirroring codex's `/permissions` menu so the live "Full Access" flow has
/// a deterministic stand-in. `--start <0..2>` seeds the highlighted option.
fn run_permission_menu(w: &mut impl Write, args: &[String]) {
    let options = ["Default", "Auto-review", "Full Access"];
    let mut selected: usize = arg_value(args, "--start")
        .and_then(|s| s.parse().ok())
        .filter(|i| *i < options.len())
        .unwrap_or(0);
    let render = |w: &mut dyn Write, selected: usize| {
        let _ = write!(w, "\x1b[2J\x1b[HUpdate Model Permissions\r\n");
        for (ix, opt) in options.iter().enumerate() {
            let marker = if ix == selected { ">" } else { " " };
            let _ = write!(w, "{marker} {opt}\r\n");
        }
        let _ = write!(w, "Press enter to confirm or esc to go back\r\n");
        let _ = w.flush();
    };
    render(w, selected);
    let mut stdin = io::stdin();
    let mut buf = [0u8; 1024];
    let mut pending: Vec<u8> = Vec::new();
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        // Consume recognized sequences greedily: ESC[A (up), ESC[B (down), CR/LF.
        loop {
            if pending.is_empty() {
                break;
            }
            if pending[0] == b'\r' || pending[0] == b'\n' {
                pending.remove(0);
                let _ = write!(w, "SELECTED: {}\r\n", options[selected]);
                let _ = w.flush();
            } else if pending.starts_with(b"\x1b[A") {
                pending.drain(..3);
                selected = selected.saturating_sub(1);
                render(w, selected);
            } else if pending.starts_with(b"\x1b[B") {
                pending.drain(..3);
                selected = (selected + 1).min(options.len() - 1);
                render(w, selected);
            } else if pending[0] == 0x1b && pending.len() < 3 {
                // Possibly a partial escape sequence — wait for more bytes.
                break;
            } else {
                // Unrecognized byte; drop it so we don't spin.
                pending.remove(0);
            }
        }
    }
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
