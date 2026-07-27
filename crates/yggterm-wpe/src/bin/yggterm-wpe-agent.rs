//! `yggterm-wpe-agent` — the Lane-A engine as its own supervised process.
//!
//! **Why a separate process.** `Engine` is a process singleton: libwpe's
//! loader, the EGL display and the current GL context are all per-process, and
//! the crate refuses a second engine. Hosting it inside the daemon would
//! permanently couple the daemon's lifetime to WebKit's, and an engine crash
//! would take the daemon with it — which the constitution forbids. So the
//! daemon spawns, probes and restarts this instead.
//!
//! One JSON object per line in, one out, over a Unix socket — the daemon idiom.
//!
//! ```sh
//! yggterm-wpe-agent /run/user/1000/yggterm-wpe.sock
//! ```
//!
//! **Honest startup.** If the WPE stack is missing or headless bring-up fails,
//! this exits with a named error on stderr and a non-zero status. It never
//! stays up as a daemon that answers every verb with a failure — a supervisor
//! cannot tell that apart from a working engine with a bad page.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::ExitCode;

use yggterm_wpe::{AgentState, Engine};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(socket_path) = args.next() else {
        eprintln!(
            "usage: yggterm-wpe-agent <socket-path>\n\
             \n\
             Serves the Lane-A agent verb plane over a Unix socket, one JSON\n\
             object per line. Verbs: ensure, navigate, eval, click, read-back,\n\
             capture-view, capture-element, restart, status."
        );
        return ExitCode::from(2);
    };

    // Bring the engine up BEFORE binding the socket. A supervisor that can
    // connect is entitled to assume the engine works; binding first would
    // publish an endpoint that fails every request.
    let engine = match Engine::new_headless() {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("yggterm-wpe-agent: headless bring-up failed: {err}");
            eprintln!(
                "  this build needs libwpewebkit-2.0, libwpebackend-fdo-1.0 and a readable \
                 DRM render node; no display server is required"
            );
            return ExitCode::from(1);
        }
    };

    // A stale socket from a killed predecessor would make bind() fail with
    // EADDRINUSE forever. Removing it is safe here precisely because we are
    // the supervised process: the supervisor guarantees one of us at a time.
    if Path::new(&socket_path).exists() {
        if let Err(err) = std::fs::remove_file(&socket_path) {
            eprintln!("yggterm-wpe-agent: could not clear stale socket {socket_path}: {err}");
            return ExitCode::from(1);
        }
    }
    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("yggterm-wpe-agent: cannot bind {socket_path}: {err}");
            return ExitCode::from(1);
        }
    };

    let mut state = AgentState::new(&engine);
    watch_for_an_orphaning_supervisor();
    eprintln!("yggterm-wpe-agent: ready on {socket_path}");

    // Connections are served ONE AT A TIME, deliberately. The engine is a
    // single-threaded GLib main context; concurrent verb dispatch would need a
    // work queue on that thread, and pretending otherwise with threads would
    // corrupt the very state the supervisor depends on.
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => serve(&mut state, stream),
            Err(err) => eprintln!("yggterm-wpe-agent: accept failed: {err}"),
        }
    }
    ExitCode::SUCCESS
}

/// Exit when the supervisor that spawned us goes away.
///
/// **Observed, not theorised** (increment 3, on a live host): a daemon killed with
/// `SIGKILL` left this process and its whole `WPEWebProcess` tree alive, holding
/// a socket that had already been unlinked — unreachable, unkillable by any
/// later daemon, and invisible to every supervision verb. The daemon's own
/// `Drop` covers an orderly exit and can cover nothing else, because a process
/// that is shot runs no cleanup. So the only mechanism that actually closes the
/// leak lives here.
///
/// `getppid()` rather than `PR_SET_PDEATHSIG`: the daemon spawns us from a
/// short-lived per-connection request thread, and `PDEATHSIG` fires on the
/// death of the spawning THREAD — it would kill this agent seconds after it
/// came up, every time. Reparenting, by contrast, happens only when the
/// supervising PROCESS is gone.
///
/// The watcher touches no GLib state (the engine's main context stays
/// single-threaded and this thread only reads a pid and exits), and it is a
/// no-op for a hand-launched agent until its launcher exits — which is also the
/// right answer there.
fn watch_for_an_orphaning_supervisor() {
    // SAFETY: `getppid` takes no arguments, touches no memory and cannot fail.
    unsafe extern "C" {
        fn getppid() -> i32;
    }
    let supervisor = unsafe { getppid() };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let current = unsafe { getppid() };
            if current != supervisor {
                eprintln!(
                    "yggterm-wpe-agent: supervisor {supervisor} is gone (reparented to \
                     {current}); exiting rather than outliving it"
                );
                std::process::exit(0);
            }
        }
    });
}

fn serve(state: &mut AgentState, stream: UnixStream) {
    let Ok(write_half) = stream.try_clone() else {
        eprintln!("yggterm-wpe-agent: could not split the connection");
        return;
    };
    let reader = BufReader::new(stream);
    let mut writer = write_half;

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("yggterm-wpe-agent: read failed: {err}");
                return;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = state.handle_line(&line);
        if writeln!(writer, "{response}").is_err() || writer.flush().is_err() {
            // The peer hung up mid-answer. Not an error worth noise: the verb
            // already ran, and the next connection gets the same state.
            return;
        }
    }
}
