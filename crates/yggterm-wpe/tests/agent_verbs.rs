//! The verb plane, end to end over the real Unix socket and the real binary.
//!
//! Deliberately NOT a library-level test of `AgentState`: the thing the daemon
//! will depend on is a *process* it spawns and talks JSON-per-line to, so that
//! is what is exercised — spawn the binary, connect, and drive every verb.
//!
//! One `#[test]` for the same reason as the headless engine suite: the engine
//! is a process singleton and killing a web process is observable to every
//! view, so the scenarios genuinely cannot run concurrently.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use yggterm_wpe::json::{Json, parse};

// ---------------------------------------------------------------------------
// Fixture server (dependency-free)
// ---------------------------------------------------------------------------

fn serve_fixtures() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture port");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle(stream));
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn handle(mut stream: TcpStream) {
    let mut buf = [0u8; 4096];
    let Ok(read) = stream.read(&mut buf) else { return };
    let request = String::from_utf8_lossy(&buf[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .trim_start_matches('/')
        .to_string();
    let body = match path.as_str() {
        "agent.html" => include_str!("../fixtures/agent.html"),
        "red.html" => include_str!("../fixtures/red.html"),
        _ => "",
    };
    let response = if body.is_empty() {
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let _ = stream.write_all(response.as_bytes());
}

// ---------------------------------------------------------------------------
// The agent process + a line client
// ---------------------------------------------------------------------------

struct Agent {
    child: Child,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    socket: String,
    next_id: u32,
}

impl Agent {
    fn spawn() -> Agent {
        let socket = format!(
            "/tmp/yggterm-wpe-agent-test-{}-{}.sock",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        );
        let _ = std::fs::remove_file(&socket);
        let child = Command::new(env!("CARGO_BIN_EXE_yggterm-wpe-agent"))
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn yggterm-wpe-agent");

        // The binary brings the engine up BEFORE binding, so a successful
        // connect already means the engine works — which is the property the
        // daemon-side supervisor will rely on.
        let deadline = Instant::now() + Duration::from_secs(30);
        let stream = loop {
            if let Ok(stream) = UnixStream::connect(&socket) {
                break stream;
            }
            assert!(
                Instant::now() < deadline,
                "the agent never bound {socket} — see its stderr above",
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("read timeout");
        Agent {
            child,
            reader: BufReader::new(stream.try_clone().expect("clone")),
            writer: stream,
            socket,
            next_id: 0,
        }
    }

    /// Send one request line, read one response line, and require `ok`.
    fn ok(&mut self, request: &str) -> Json {
        let value = self.send(request);
        assert_eq!(
            value.get("ok").and_then(Json::as_bool),
            Some(true),
            "request {request} failed: {}",
            value
                .get("error")
                .and_then(Json::as_str)
                .unwrap_or("(no error field)"),
        );
        value
    }

    /// Send one request line and require it to FAIL, returning the message.
    fn err(&mut self, request: &str) -> String {
        let value = self.send(request);
        assert_eq!(
            value.get("ok").and_then(Json::as_bool),
            Some(false),
            "request {request} unexpectedly succeeded: {}",
            value.to_string(),
        );
        value
            .get("error")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_string()
    }

    fn send(&mut self, request: &str) -> Json {
        self.next_id += 1;
        let id = self.next_id.to_string();
        // Splice the id in so every response can be matched to its request.
        let line = if request.starts_with('{') {
            format!("{{\"id\":\"{id}\",{}", &request[1..])
        } else {
            request.to_string()
        };
        writeln!(self.writer, "{line}").expect("write request");
        self.writer.flush().expect("flush");

        let mut response = String::new();
        self.reader
            .read_line(&mut response)
            .expect("read response line");
        assert!(!response.trim().is_empty(), "the agent closed the connection");
        let value = parse(response.trim())
            .unwrap_or_else(|e| panic!("response is not JSON: {e}\n{response}"));
        if request.starts_with('{') {
            assert_eq!(
                value.get("id").and_then(Json::as_str),
                Some(id.as_str()),
                "every response must echo its request id",
            );
        }
        value
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn field<'a>(value: &'a Json, key: &str) -> &'a Json {
    value.get(key).unwrap_or(&Json::Null)
}

#[test]
fn the_verb_plane_drives_a_page_and_surfaces_recovery_without_automating_it() {
    let base = serve_fixtures();
    let mut agent = Agent::spawn();

    // ---- ensure ----
    let ensured = agent.ok(&format!(
        r##"{{"verb":"ensure","session":"a","url":"{base}/agent.html","width":320,"height":240}}"##
    ));
    assert_eq!(field(&ensured, "created").as_bool(), Some(true));
    // Idempotent: a second ensure reuses the view rather than opening another.
    let again = agent.ok(r##"{"verb":"ensure","session":"a"}"##);
    assert_eq!(
        field(&again, "created").as_bool(),
        Some(false),
        "ensure must be idempotent per session key, or every call leaks a view",
    );
    assert_eq!(field(&again, "view").as_f64(), field(&ensured, "view").as_f64());
    eprintln!("[test] ensure: created then reused");

    // ---- eval ----
    let evaluated = agent.ok(r##"{"verb":"eval","session":"a","script":"1 + 41"}"##);
    assert_eq!(
        field(&evaluated, "result").as_f64(),
        Some(42.0),
        "a number must survive as a NUMBER, not as a string containing JSON",
    );
    let thrown = agent.err(r##"{"verb":"eval","session":"a","script":"throw new Error('boom')"}"##);
    assert!(
        thrown.contains("boom"),
        "a page that throws must surface the engine's own message, not an empty result a \
         caller would read as success: {thrown}",
    );
    eprintln!("[test] eval: value typed, throw surfaced");

    // ---- read-back ----
    let before = agent.ok(r##"{"verb":"read-back","session":"a","selector":"#out"}"##);
    assert_eq!(field(&before, "text").as_str(), Some("idle"));

    // ---- click, by selector, proven by read-back ----
    agent.ok(r##"{"verb":"click","session":"a","selector":"#go"}"##);
    let after = agent.ok(r##"{"verb":"read-back","session":"a","selector":"#out"}"##);
    assert_eq!(
        field(&after, "text").as_str(),
        Some("clicked"),
        "the click must reach the button's own listener",
    );
    eprintln!("[test] click: #go pressed, #out reads 'clicked'");

    // ---- type, proven by reading the input's VALUE (not its text) ----
    agent.ok(r##"{"verb":"type","session":"a","text":"x"}"##);
    let field_state = agent.ok(r##"{"verb":"read-back","session":"a","selector":"#field"}"##);
    assert_eq!(
        field(&field_state, "value").as_str(),
        Some("typed-x"),
        "the keystroke must reach the page's keydown listener; read-back reports an input's \
         VALUE separately from its textContent because for a form field they differ",
    );
    let untypable = agent.err(r##"{"verb":"type","session":"a","text":"é"}"##);
    assert!(
        untypable.contains("keysym"),
        "an untypable character must be refused with a reason, never guessed: {untypable}",
    );
    eprintln!("[test] type: 'x' reached the page; untypable refused");

    // ---- ambiguity is REFUSED with a count ----
    let ambiguous = agent.err(r##"{"verb":"click","session":"a","selector":".dup"}"##);
    assert!(
        ambiguous.contains('2') && ambiguous.contains("refusing"),
        "an ambiguous selector must be refused WITH THE COUNT — 'it clicked something' is the \
         worst outcome for an agent: {ambiguous}",
    );
    let missing = agent.err(r##"{"verb":"read-back","session":"a","selector":"#nope"}"##);
    assert!(missing.contains('0'), "a zero match must say so: {missing}");
    eprintln!("[test] ambiguity refused with counts (2 and 0)");

    // ---- capture-view and capture-element ----
    let dir = std::env::temp_dir();
    let view_png = dir.join("wpe-agent-view.png");
    let el_png = dir.join("wpe-agent-box.png");
    let _ = std::fs::remove_file(&view_png);
    let _ = std::fs::remove_file(&el_png);

    let captured = agent.ok(&format!(
        r##"{{"verb":"capture-view","session":"a","path":"{}"}}"##,
        view_png.display()
    ));
    assert_eq!(field(&captured, "width").as_f64(), Some(320.0));
    assert!(std::fs::read(&view_png).expect("view png written").len() > 100);

    let element = agent.ok(&format!(
        r##"{{"verb":"capture-element","session":"a","selector":"#box","path":"{}"}}"##,
        el_png.display()
    ));
    assert_eq!(
        (
            field(&element, "width").as_f64(),
            field(&element, "height").as_f64()
        ),
        (Some(40.0), Some(30.0)),
        "capture-element must crop to the element's own rect, not the viewport",
    );
    let png = std::fs::read(&el_png).expect("element png written");
    assert_eq!(&png[0..4], b"\x89PNG", "and it must be a real PNG");
    eprintln!("[test] capture-view 320x240, capture-element 40x30");

    // ---- a second session is independent ----
    agent.ok(&format!(
        r##"{{"verb":"ensure","session":"b","url":"{base}/red.html","width":320,"height":240}}"##
    ));
    let status = agent.ok(r##"{"verb":"status"}"##);
    let views = field(&status, "views").as_array().expect("views").to_vec();
    assert_eq!(views.len(), 2, "status must list both sessions");
    let processes = field(&status, "web_processes")
        .as_array()
        .expect("web_processes")
        .to_vec();
    assert!(
        processes.len() >= 2,
        "two sessions must get two web processes: {processes:?}",
    );
    assert!(
        views
            .iter()
            .all(|v| field(v, "web_process_terminated").as_bool() == Some(false)),
        "no view should report itself terminated yet",
    );
    eprintln!("[test] status: 2 views, {} web processes", processes.len());

    // ---- kill session a's web process ----
    let pid = views
        .iter()
        .find(|v| field(v, "session").as_str() == Some("a"))
        .and_then(|v| field(v, "web_process").as_f64())
        .expect("status must attribute a web process to session a") as i32;
    unsafe { libc_kill(pid, 9) };
    eprintln!("[test] killed session a's web process ({pid})");

    // status must NAME it — honestly, and without acting on it.
    let mut named = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let status = agent.ok(r##"{"verb":"status"}"##);
        let views = field(&status, "views").as_array().unwrap_or(&[]).to_vec();
        let a = views
            .iter()
            .find(|v| field(v, "session").as_str() == Some("a"));
        if a.is_some_and(|v| field(v, "web_process_terminated").as_bool() == Some(true)) {
            named = true;
            assert!(
                views
                    .iter()
                    .find(|v| field(v, "session").as_str() == Some("b"))
                    .is_some_and(|v| field(v, "web_process_terminated").as_bool() == Some(false)),
                "the kill must be attributed to session a ALONE",
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        named,
        "status must NAME a terminated view — a dead surface the caller cannot see is worse \
         than a dead surface",
    );
    eprintln!("[test] status named session a terminated; session b untouched");

    // ---- the OTHER session kept answering throughout ----
    let b_alive = agent.ok(r##"{"verb":"eval","session":"b","script":"document.title"}"##);
    assert_eq!(
        field(&b_alive, "result").as_str(),
        Some("FIXTURE-RED"),
        "session b must still answer while session a is dead",
    );
    eprintln!("[test] session b still answers eval");

    // ---- recovery is EXPLICIT ----
    let restarted = agent.ok(r##"{"verb":"restart","session":"a"}"##);
    assert_eq!(
        field(&restarted, "previous_web_process").as_f64(),
        Some(f64::from(pid)),
        "restart must report which process it replaced",
    );
    let new_pid = field(&restarted, "web_process")
        .as_f64()
        .expect("a restarted view runs on a new process");
    assert_ne!(
        new_pid,
        f64::from(pid),
        "restart must produce a NEW process, not resurrect the corpse",
    );

    // …and the recovered view is genuinely usable again: fresh document, and it
    // answers a click.
    let out = agent.ok(r##"{"verb":"read-back","session":"a","selector":"#out"}"##);
    assert_eq!(
        field(&out, "text").as_str(),
        Some("idle"),
        "a restarted view holds a FRESH document, so #out is back to its initial text",
    );
    agent.ok(r##"{"verb":"click","session":"a","selector":"#go"}"##);
    let out = agent.ok(r##"{"verb":"read-back","session":"a","selector":"#out"}"##);
    assert_eq!(
        field(&out, "text").as_str(),
        Some("clicked"),
        "a restarted view must take input again, not merely paint",
    );
    let status = agent.ok(r##"{"verb":"status"}"##);
    assert!(
        field(&status, "views")
            .as_array()
            .unwrap_or(&[])
            .iter()
            .all(|v| field(v, "web_process_terminated").as_bool() == Some(false)),
        "after an explicit restart, no view should still report itself terminated",
    );
    eprintln!("[test] restart: new pid {new_pid}, fresh document, input works again");

    // ---- malformed and unknown input still get well-formed answers ----
    let bad = agent.err("not json at all");
    assert!(bad.contains("malformed"), "{bad}");
    let unknown = agent.err(r##"{"verb":"fly"}"##);
    assert!(unknown.contains("unknown verb"), "{unknown}");
    let sessionless = agent.err(r##"{"verb":"navigate","url":"about:blank"}"##);
    assert!(sessionless.contains("session"), "{sessionless}");
    // The connection must still be usable after all of those.
    agent.ok(r##"{"verb":"status"}"##);
    eprintln!("[test] malformed/unknown/missing-arg all answered; connection survived");
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// LOCK: the agent buries itself when the supervisor that spawned it dies.
///
/// Found live, not theorised (increment 3): a daemon killed with
/// `SIGKILL` left the agent and its whole `WPEWebProcess` tree alive, holding a
/// socket that had already been unlinked. Nothing could reach it, no later
/// daemon could see it, and no supervision verb knew it existed — an immortal
/// tenant by construction. The daemon's `Drop` covers an orderly exit and can
/// never cover a shot process, so the mechanism has to live in the agent.
///
/// This spawns the agent under a SHELL, kills the shell, and requires the agent
/// to be gone. A separate test from the verb-plane one because it deliberately
/// destroys its own agent.
///
/// MUTATION that turns this red: delete the `watch_for_an_orphaning_supervisor()`
/// call in `main`. The agent then outlives its supervisor forever, which is the
/// state this was written from.
#[test]
fn an_orphaned_agent_exits_instead_of_outliving_its_supervisor() {
    let socket = format!(
        "/tmp/yggterm-wpe-orphan-{}-{}.sock",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    );
    let _ = std::fs::remove_file(&socket);

    // A shell stands in for the daemon: it spawns the agent, prints its pid,
    // and then waits. Killing the shell reparents the agent to init — exactly
    // what a SIGKILLed daemon does.
    let mut supervisor = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{} {socket} & echo $! ; wait",
            env!("CARGO_BIN_EXE_yggterm-wpe-agent")
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn the stand-in supervisor");

    let mut pid_line = String::new();
    BufReader::new(supervisor.stdout.take().expect("supervisor stdout"))
        .read_line(&mut pid_line)
        .expect("read the agent pid");
    let agent_pid: i32 = pid_line.trim().parse().expect("the agent's pid");

    // Wait until it is actually serving, so this cannot pass by killing an
    // agent that never came up.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if UnixStream::connect(&socket).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "the agent never bound {socket}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        unsafe { libc_kill(agent_pid, 0) },
        0,
        "the agent should be alive before its supervisor dies",
    );

    let _ = supervisor.kill();
    let _ = supervisor.wait();

    // The watcher polls every 2s; give it a few cycles before believing it.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut buried = false;
    while Instant::now() < deadline {
        if unsafe { libc_kill(agent_pid, 0) } != 0 {
            buried = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if !buried {
        // Do not leave the leak behind for the next run to inherit.
        unsafe { libc_kill(agent_pid, 9) };
    }
    assert!(
        buried,
        "agent {agent_pid} outlived its supervisor — an unreachable WebKit tree on an \
         unlinked socket is exactly the immortal tenant this locks",
    );
    let _ = std::fs::remove_file(&socket);
    eprintln!("[test] orphan: agent {agent_pid} buried itself with its supervisor");
}
