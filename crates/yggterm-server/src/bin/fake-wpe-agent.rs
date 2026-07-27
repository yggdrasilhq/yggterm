//! fake-wpe-agent — a scriptable stand-in for `yggterm-wpe-agent`.
//!
//! **Why a double at all.** The real agent needs libwpewebkit-2.0,
//! libwpebackend-fdo-1.0 and a DRM render node; the daemon crate must stay
//! green on every fleet machine and in CI, none of which carry that stack. The
//! real-binary end-to-end lives where it belongs — `crates/yggterm-wpe`'s own
//! suite, on the one host with the stack. What is tested HERE is the daemon's
//! half: transport, typing of failures, and supervision. None of that needs a
//! browser; all of it needs failure modes a real browser will not perform on
//! demand.
//!
//! It speaks the same line protocol (one JSON object per line each way, `id`
//! echoed, `ok` always present) and can be told to misbehave:
//!
//! ```sh
//! fake-wpe-agent <socket> [--mode MODE] [--mode-from N] [--mode-to N]
//!                         [--late-ms N] [--exit-code N]
//! ```
//!
//! Modes (applied to requests numbered `--mode-from`..=`--mode-to`, 1-based;
//! every other request is answered as `ok`):
//!
//! - `ok`            — answer `ok:true`, echoing the request.
//! - `error`         — answer `ok:false` with an `error` string.
//! - `hang`          — read the request and never answer, holding the
//!                     connection OPEN (a closed one would be an EOF, which is
//!                     a different failure than a hang).
//! - `late`          — answer after `--late-ms`, i.e. after the caller's
//!                     deadline, on the connection it gave up on.
//! - `die`           — exit `--exit-code` mid-request, without answering.
//! - `die-on-start`  — diagnose on stderr and exit BEFORE binding, the way the
//!                     real agent does on a host with no WPE stack.
//! - `stale-id`      — answer with somebody else's `id`.
//!
//! Connections get a thread each, unlike the real agent (whose single GLib main
//! context forbids it). That is the point: `late` and `hang` must not stop the
//! double from accepting the NEXT connection, or the recycle-after-timeout
//! behaviour under test could not be observed at all.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

/// Connections deliberately left open and unanswered by `hang`.
///
/// Held in a static so they are not dropped when the connection thread ends:
/// dropping the stream would close it, and a closed socket delivers EOF, which
/// the client correctly reports as "closed without answering" rather than as a
/// timeout. Parking it is what makes the hang a hang.
fn parked() -> &'static Mutex<Vec<UnixStream>> {
    static PARKED: OnceLock<Mutex<Vec<UnixStream>>> = OnceLock::new();
    PARKED.get_or_init(|| Mutex::new(Vec::new()))
}

fn requests() -> &'static AtomicU64 {
    static REQUESTS: AtomicU64 = AtomicU64::new(0);
    &REQUESTS
}

#[derive(Clone, Copy)]
struct Script {
    mode: Mode,
    from: u64,
    to: u64,
    late_ms: u64,
    exit_code: i32,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Ok,
    Error,
    Hang,
    Late,
    Die,
    StaleId,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let Some(socket_path) = args.first().cloned() else {
        eprintln!("usage: fake-wpe-agent <socket> [--mode MODE] …");
        std::process::exit(2);
    };
    // The script may also arrive in a sidecar file beside the socket. That is
    // how the daemon-client suite drives this: the production spawn is
    // `<binary> <socket>` and nothing else, so a test that had to pass flags
    // would be exercising a spawn path that does not ship.
    if let Ok(extra) = std::fs::read_to_string(format!("{socket_path}.script")) {
        args.extend(extra.split_whitespace().map(str::to_string));
    }

    let mode_name = flag(&args, "--mode").unwrap_or_else(|| "ok".to_string());
    let exit_code = flag(&args, "--exit-code")
        .and_then(|value| value.parse().ok())
        .unwrap_or(17);

    if mode_name == "die-on-start" {
        // Exactly the real agent's honest-startup shape: name the failure on
        // stderr, exit non-zero, never bind. A supervisor must be able to tell
        // this apart from a working engine.
        eprintln!("fake-wpe-agent: headless bring-up failed: no WPE stack on this host");
        eprintln!("  (this build needs libwpewebkit-2.0 and libwpebackend-fdo-1.0)");
        std::process::exit(exit_code);
    }

    let script = Script {
        mode: match mode_name.as_str() {
            "ok" => Mode::Ok,
            "error" => Mode::Error,
            "hang" => Mode::Hang,
            "late" => Mode::Late,
            "die" => Mode::Die,
            "stale-id" => Mode::StaleId,
            other => {
                eprintln!("fake-wpe-agent: unknown --mode {other}");
                std::process::exit(2);
            }
        },
        from: flag(&args, "--mode-from")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
        to: flag(&args, "--mode-to")
            .and_then(|value| value.parse().ok())
            .unwrap_or(u64::MAX),
        late_ms: flag(&args, "--late-ms")
            .and_then(|value| value.parse().ok())
            .unwrap_or(3_000),
        exit_code,
    };

    if Path::new(&socket_path).exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("fake-wpe-agent: cannot bind {socket_path}: {error}");
            std::process::exit(1);
        }
    };
    eprintln!("fake-wpe-agent: ready on {socket_path} (mode {mode_name})");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || serve(stream, script));
            }
            Err(error) => eprintln!("fake-wpe-agent: accept failed: {error}"),
        }
    }
}

fn serve(stream: UnixStream, script: Script) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let id = request
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let verb = request
            .get("verb")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let index = requests().fetch_add(1, Ordering::SeqCst) + 1;

        let scripted = index >= script.from && index <= script.to;
        let mode = if scripted { script.mode } else { Mode::Ok };

        let answer = match mode {
            Mode::Hang => {
                // Park the connection open and go back to accepting. The
                // caller's read deadline is what ends this.
                if let Ok(mut held) = parked().lock() {
                    if let Ok(clone) = writer.try_clone() {
                        held.push(clone);
                    }
                }
                return;
            }
            Mode::Die => {
                eprintln!("fake-wpe-agent: dying mid-request {index} ({verb})");
                std::process::exit(script.exit_code);
            }
            Mode::Late => {
                std::thread::sleep(Duration::from_millis(script.late_ms));
                ok_answer(&id, &verb, index, &request)
            }
            Mode::Error => json!({
                "id": id,
                "ok": false,
                "error": format!("fake agent refuses {verb}"),
            }),
            Mode::StaleId => {
                let mut answer = ok_answer(&id, &verb, index, &request);
                answer["id"] = Value::String(format!("stale-{id}"));
                answer
            }
            Mode::Ok => ok_answer(&id, &verb, index, &request),
        };

        let Ok(encoded) = serde_json::to_string(&answer) else {
            return;
        };
        if writeln!(writer, "{encoded}").is_err() || writer.flush().is_err() {
            return;
        }
    }
}

fn ok_answer(id: &str, verb: &str, index: u64, request: &Value) -> Value {
    json!({
        "id": id,
        "ok": true,
        "verb": verb,
        // The pid and the request index are what let a test prove WHICH agent
        // answered — the difference between "restart spawned a successor" and
        // "restart quietly reused the corpse".
        "pid": std::process::id(),
        "request_index": index,
        "echo": request,
    })
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
