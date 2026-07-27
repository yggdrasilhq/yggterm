//! Headless integration tests — the colour readback IS the assertion instrument.
//!
//! **Why colour and not the DOM.** The fixture turns green only on
//! `pointerdown` and blue only on a `keydown` whose `e.key === "x"`. Reading
//! those colours out of the compositor's exported frame proves a real event
//! travelled the real input path into WebCore. A `document.querySelector` check
//! would have proven only that JavaScript runs — spike C settled this and the
//! crate's tests inherit the method.
//!
//! **One process, one engine, one test.** libwpe's loader, the EGL display and
//! the current GL context are process-global, and killing a web process is
//! observable to every view, so the scenarios cannot run concurrently. Rather
//! than pretend otherwise with a mutex, this is a single `#[test]` that runs the
//! scenarios in order against one engine and reports each as it passes.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use yggterm_wpe::{Engine, Error, Supervisor, ViewId};

const W: u32 = 320;
const H: u32 = 240;
const SETTLE: Duration = Duration::from_secs(25);
const REACT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// A dependency-free fixture server
// ---------------------------------------------------------------------------

fn serve_fixtures() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a fixture port");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            std::thread::spawn(move || handle(stream));
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn handle(mut stream: TcpStream) {
    let mut buf = [0u8; 2048];
    let Ok(read) = stream.read(&mut buf) else {
        return;
    };
    let request = String::from_utf8_lossy(&buf[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .trim_start_matches('/')
        .to_string();

    let body = match path.as_str() {
        "interactive.html" => include_str!("../fixtures/interactive.html"),
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
    let _ = stream.flush();
}

// ---------------------------------------------------------------------------

fn dominant(px: [u8; 4]) -> &'static str {
    let [r, g, b, _] = px;
    if r > 200 && g < 80 && b < 80 {
        "red"
    } else if g > 200 && r < 80 && b < 80 {
        "green"
    } else if b > 200 && r < 80 && g < 80 {
        "blue"
    } else {
        "other"
    }
}

/// Assert on the frame the view ALREADY has.
///
/// ⚠ Use this after `open`/`restart`, which have already waited for a paint.
/// [`expect_colour`] forgets the current frame and waits for the NEXT one,
/// which a STATIC page never produces — spike C made this mistake and so did
/// the first version of this test. "Wait for a new frame" is only correct when
/// something has just been done that must cause a repaint.
fn assert_colour_now(sup: &Supervisor, id: ViewId, want: &str) {
    let view = sup.view(id).expect("view exists");
    let frame = view
        .last_frame()
        .unwrap_or_else(|| panic!("view {id:?} has not painted at all"));
    assert_eq!(
        dominant(frame.centre_pixel()),
        want,
        "view {id:?}: expected {want}, frame is {:?}",
        frame.centre_pixel(),
    );
}

fn expect_colour(sup: &mut Supervisor, id: ViewId, want: &'static str, timeout: Duration) {
    let outcome = sup.await_frame(id, timeout, move |frame| dominant(frame.centre_pixel()) == want);
    if outcome.is_err() {
        let actual = sup
            .view(id)
            .ok()
            .and_then(|v| v.last_frame())
            .map(|f| format!("{} {:?}", dominant(f.centre_pixel()), f.centre_pixel()))
            .unwrap_or_else(|| "no frame".to_string());
        panic!("view {id:?}: expected {want}, got {actual}");
    }
}

#[test]
fn the_headless_engine_paints_takes_input_and_survives_a_dead_web_process() {
    let base = serve_fixtures();
    let engine = match Engine::new_headless() {
        Ok(engine) => engine,
        Err(err) => panic!("headless bring-up failed: {err}"),
    };
    let mut sup = Supervisor::new(&engine);

    // ---- 1. a page paints, and blank frames were skipped on the way ----
    let a = sup
        .open(&format!("{base}/interactive.html"), W, H, SETTLE)
        .expect("view-a opens and paints");
    let view_a = sup.view(a).expect("view-a");
    assert_eq!(
        dominant(view_a.last_frame().expect("painted").centre_pixel()),
        "red",
        "the fixture's initial state is red",
    );
    assert!(
        view_a.blank_frames_skipped() >= 1,
        "the compositor exports a BLANK frame before the page paints, and skipping it is the \
         whole reason last_frame() can be trusted — seeing zero skips means the blank-frame \
         guard is no longer being exercised, so it is no longer being tested",
    );
    assert!(
        view_a
            .last_frame()
            .map(|f| !f.is_blank())
            .unwrap_or(false),
        "last_frame() must never hand out a blank frame",
    );
    eprintln!(
        "[test] 1. painted red; {} frames exported, {} blank skipped",
        view_a.frames_exported(),
        view_a.blank_frames_skipped()
    );

    // ---- 2. a click LANDS ----
    sup.view(a).unwrap().click_centre();
    expect_colour(&mut sup, a, "green", REACT);
    eprintln!("[test] 2. click landed (green)");

    // ---- 3. a keystroke LANDS, through the keysym table ----
    sup.view(a).unwrap().type_text("x").expect("x is typable");
    expect_colour(&mut sup, a, "blue", REACT);
    eprintln!("[test] 3. keystroke landed (blue)");

    // ---- 4. a character we cannot type is refused, not guessed ----
    assert_eq!(
        sup.view(a).unwrap().type_text("é"),
        Err(Error::UntypableCharacter('é')),
        "an untypable character must be an error — a wrong keysym is silently swallowed by \
         WebKit, so guessing would look exactly like the page ignoring input",
    );
    eprintln!("[test] 4. untypable character refused");

    // ---- 5. a second view is independent, and each gets its own process ----
    let b = sup
        .open(&format!("{base}/interactive.html"), W, H, SETTLE)
        .expect("view-b opens and paints");
    // `open` already waited for the paint — assert on THAT frame.
    assert_colour_now(&sup, b, "red");

    let processes = sup.web_processes();
    eprintln!(
        "[test] 5. web processes: {:?}",
        processes.iter().map(|p| (p.pid, &p.comm)).collect::<Vec<_>>()
    );
    assert!(
        processes.len() >= 2,
        "two views must get two web processes — per-view isolation is what makes one crash \
         survivable. Found {processes:?}. NOTE: web processes are bubblewrap GRANDCHILDREN \
         with comm truncated to 15 chars, so a direct-children scan reports zero",
    );
    assert!(
        sup.web_process_of(a).is_some() && sup.web_process_of(b).is_some(),
        "the open-time diff must attribute a process to each view — neither libwpe nor \
         WebKit reports this mapping, so if it is not built here it does not exist",
    );
    assert_ne!(
        sup.web_process_of(a),
        sup.web_process_of(b),
        "two views must not be attributed the same process",
    );

    // ---- 6. killing one view's process is attributed to THAT view ----
    let victim = sup.kill_web_process_of(a).expect("view-a has a web process");
    let noticed = sup.pump_until(REACT, |sup| !sup.terminated().is_empty());
    assert!(noticed, "no view reported its web process dying");
    assert_eq!(
        sup.terminated(),
        vec![a],
        "the kill must be attributed to view-a ALONE — if both views report it they share a \
         process and there is no isolation to rely on",
    );
    eprintln!("[test] 6. killed pid {victim}; only view-a reported it");

    // ---- 7. the survivor is still INTERACTIVE ----
    // Not "still has a frame": a static page never repaints, so waiting for a
    // frame proves only that the test is wrong. Drive it.
    sup.view(b).unwrap().click_centre();
    expect_colour(&mut sup, b, "green", REACT);
    eprintln!("[test] 7. survivor view-b answered a click after the kill");

    // ---- 7b. "painted since load" is NOT "has a frame" ----
    // The distinction is the whole reason restart() is trustworthy: recovering a
    // killed view paints an intermediate WHITE frame, and settling on it would
    // report a restored surface that is still blank. A view told to navigate has
    // NOT painted since that load, even though it still holds the OLD page's
    // frame — assert exactly that, with no pumping in between so it cannot pass
    // on timing.
    {
        let previous = sup
            .view(b)
            .unwrap()
            .last_frame()
            .expect("view-b holds a frame from before")
            .centre_pixel();
        sup.view_mut(b)
            .unwrap()
            .load_uri(&format!("{base}/red.html"))
            .expect("navigate view-b");
        let view_b = sup.view(b).unwrap();
        assert!(
            view_b.last_frame().is_some(),
            "the previous page's frame is still held",
        );
        assert!(
            !view_b.painted_current_document(),
            "a view that has just been told to navigate does NOT hold a finished picture of \
             the new document, even though it still holds {previous:?} from the previous one. \
             Conflating the two is what makes a restart report success on an intermediate \
             blank frame",
        );
        // Let it settle again so later steps see a coherent view.
        assert!(
            sup.pump_until(SETTLE, |s| s.view(b).is_ok_and(|v| v.painted_current_document())),
            "view-b should finish its navigation",
        );
    }
    eprintln!("[test] 7b. painted_current_document distinguishes a stale frame from a fresh paint");

    // ---- 8. restart is explicit, and restores the view completely ----
    sup.restart(a, SETTLE).expect("view-a restarts");
    // `restart` waited for the repaint too.
    assert_colour_now(&sup, a, "red");
    sup.view(a).unwrap().click_centre();
    expect_colour(&mut sup, a, "green", REACT);
    assert!(
        !sup.view(a).unwrap().web_process_terminated(),
        "a restarted view must no longer report itself terminated",
    );
    let restarted_pid = sup.web_process_of(a);
    assert!(
        restarted_pid.is_some() && restarted_pid != Some(victim),
        "the restarted view must be served by a NEW process, not the corpse: {restarted_pid:?} \
         vs {victim}",
    );
    eprintln!("[test] 8. view-a restarted on pid {restarted_pid:?} and answers input again");

    // ---- 9. a second engine in one process is refused ----
    assert_eq!(
        Engine::new_headless().err(),
        Some(Error::AlreadyInitialised),
        "EGL, the GL context and libwpe's loader are process-global, so a second Engine must \
         be refused rather than silently fighting the first",
    );
    eprintln!("[test] 9. a second Engine is refused");
}
