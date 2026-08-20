//! Does this agent CLI fill the grid it is given?
//!
//! ⚠ THE POINT OF THIS TOOL IS THAT IT IS NOT A HAND-ROLLED PARSER. It feeds the
//! PTY stream to `vt100`, the SAME crate `yggterm-server::terminal` uses for its
//! screen model, so a coverage number here is measured by the daemon's own eyes.
//! A hand-rolled vt100 silently ignores alt-screen and scroll regions and then
//! reports a "cut off top" that only its own gaps produced — which is how a probe
//! becomes the bug it is hunting.
//!
//! Usage:
//!   cli-viewport-probe --cmd qwen --cols 173 --rows 63 --settle-ms 15000
//!   cli-viewport-probe --cmd grok --cols 120 --rows 40 --resize 173x63 --show
//!
//! `--resize` reproduces yggterm's real launch sequence: the daemon spawns the
//! PTY at one grid and a client re-attach resizes it to another.
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Args {
    cmd: String,
    cols: u16,
    rows: u16,
    settle_ms: u64,
    resize: Option<(u16, u16)>,
    resize_after_ms: u64,
    show: bool,
    cwd: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        cmd: String::new(),
        cols: 173,
        rows: 63,
        settle_ms: 12_000,
        resize: None,
        resize_after_ms: 0,
        show: false,
        cwd: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].as_str();
        let val = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i).cloned().unwrap_or_default()
        };
        match k {
            "--cmd" => a.cmd = val(&mut i),
            "--cols" => a.cols = val(&mut i).parse().unwrap_or(173),
            "--rows" => a.rows = val(&mut i).parse().unwrap_or(63),
            "--settle-ms" => a.settle_ms = val(&mut i).parse().unwrap_or(12_000),
            "--resize-after-ms" => a.resize_after_ms = val(&mut i).parse().unwrap_or(0),
            "--cwd" => a.cwd = Some(val(&mut i)),
            "--resize" => {
                let v = val(&mut i);
                let (c, r) = v.split_once('x').unwrap_or(("0", "0"));
                a.resize = Some((c.parse().unwrap_or(0), r.parse().unwrap_or(0)));
            }
            "--show" => a.show = true,
            _ => {}
        }
        i += 1;
    }
    a
}

fn main() {
    let args = parse_args();
    if args.cmd.is_empty() {
        eprintln!("--cmd is required");
        std::process::exit(2);
    }
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: args.rows,
            cols: args.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.args(["-c", &args.cmd]);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLUMNS", args.cols.to_string());
    cmd.env("LINES", args.rows.to_string());
    // A probe must not inherit the caller's row identity, or the CLI it starts
    // reports itself as this session.
    cmd.env_remove("YGGTERM_SESSION_ID");
    if let Some(cwd) = &args.cwd {
        cmd.cwd(cwd);
    }
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let parser = Arc::new(Mutex::new(vt100::Parser::new(args.rows, args.cols, 0)));
    let bytes = Arc::new(Mutex::new(0usize));
    let mut reader = pair.master.try_clone_reader().expect("reader");
    {
        let parser = Arc::clone(&parser);
        let bytes = Arc::clone(&bytes);
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                *bytes.lock().unwrap() += n;
                parser.lock().unwrap().process(&buf[..n]);
            }
        });
    }

    let start = Instant::now();
    std::thread::sleep(Duration::from_millis(args.settle_ms));
    let mut final_cols = args.cols;
    let mut final_rows = args.rows;
    if let Some((c, r)) = args.resize {
        if args.resize_after_ms > 0 {
            let elapsed = start.elapsed().as_millis() as u64;
            if args.resize_after_ms > elapsed {
                std::thread::sleep(Duration::from_millis(args.resize_after_ms - elapsed));
            }
        }
        pair.master
            .resize(PtySize {
                rows: r,
                cols: c,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize");
        parser.lock().unwrap().screen_mut().set_size(r, c);
        final_cols = c;
        final_rows = r;
        // Give the TUI the same settle budget again to repaint into the new grid.
        std::thread::sleep(Duration::from_millis(args.settle_ms));
    }

    let guard = parser.lock().unwrap();
    let screen = guard.screen();
    let mut max_col = 0u16;
    let mut max_row: i32 = -1;
    let mut first_row: i32 = -1;
    let mut painted_rows = 0u16;
    let mut bg_only_cells = 0u32;
    let mut lines: Vec<String> = Vec::new();
    for r in 0..final_rows {
        let mut line = String::new();
        let mut row_has = false;
        for c in 0..final_cols {
            let cell = screen.cell(r, c);
            let ch = cell.map(|cell| cell.contents()).unwrap_or_default();
            // ⚠ A CELL CAN BE PAINTED AND STILL HOLD NO TEXT. Gradient banners and
            // block art are routinely drawn as SPACES with a background colour, so a
            // "is the text blank" test reports a fully-painted header as empty and
            // invents a cut-off top. Ask the cell whether it is DEFAULT, not whether
            // it is whitespace.
            let bg_painted = cell
                .map(|cell| cell.bgcolor() != vt100::Color::Default)
                .unwrap_or(false);
            let painted = !ch.trim().is_empty() || bg_painted;
            if painted {
                row_has = true;
                if c + 1 > max_col {
                    max_col = c + 1;
                }
            }
            if ch.trim().is_empty() && bg_painted {
                bg_only_cells += 1;
            }
            line.push_str(if ch.is_empty() {
                if bg_painted { "\u{2591}" } else { " " }
            } else {
                &ch
            });
        }
        if row_has {
            painted_rows += 1;
            max_row = r as i32;
            if first_row < 0 {
                first_row = r as i32;
            }
        }
        lines.push(line.trim_end().to_string());
    }
    let (cur_row, cur_col) = screen.cursor_position();
    let out = serde_json::json!({
        "cmd": args.cmd,
        "spawn_grid": [args.cols, args.rows],
        "final_grid": [final_cols, final_rows],
        "resized": args.resize.is_some(),
        "max_col_painted": max_col,
        "max_row_painted": max_row + 1,
        "first_row_painted": first_row + 1,
        "painted_rows": painted_rows,
        "bg_only_cells": bg_only_cells,
        "coverage_cols_pct": (max_col as f64 * 100.0 / final_cols as f64 * 10.0).round() / 10.0,
        "coverage_rows_pct": ((max_row + 1) as f64 * 100.0 / final_rows as f64 * 10.0).round() / 10.0,
        "cursor": [cur_col, cur_row],
        "alternate_screen": screen.alternate_screen(),
        "bytes": *bytes.lock().unwrap(),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    if args.show {
        println!("--- screen ({final_cols}x{final_rows}) ---");
        for (i, l) in lines.iter().enumerate() {
            println!("{:>3}|{}", i + 1, l);
        }
    }
    drop(guard);
    let _ = child.kill();
    let _ = child.wait();
}
