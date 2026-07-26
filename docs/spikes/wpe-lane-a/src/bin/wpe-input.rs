//! O9 — WPE spike C: input routing and per-view process lifecycle.
//!
//! Spike B's verdict was that pixels are solved and Lane-A's remaining unknowns
//! are input and lifecycle. This closes both, and it proves them **with the
//! readback**, not with a DOM query:
//!
//! - The fixture turns GREEN on `pointerdown` and BLUE on the `x` keydown.
//!   Nothing else can set those colours, so a colour change read out of the
//!   compositor's exported frame is proof that a real event travelled the real
//!   input path into WebCore. A `document.querySelector` check would have
//!   proven only that JavaScript runs.
//! - Two views run at once; one view's `WPEWebProcess` is killed; the survivor
//!   must still paint (readback again) and the casualty must be detectable and
//!   restartable.
//!
//! Usage: `wpe-input <base-url>` — expects `interactive.html` and `red.html`
//! under that base.

use std::ffi::{CString, c_int, c_uint, c_void};
use std::process::ExitCode;
use std::ptr;
use std::time::{Duration, Instant};

#[path = "../headless.rs"]
mod headless;

use headless::*;

const VIEW_W: u32 = 320;
const VIEW_H: u32 = 240;

/// Entry points spike C needed beyond spikes A + B.
const SPIKE_C_NEW_FN_COUNT: usize = 6;

/// One headless view: its exportable backend, its WebKitWebView, and the last
/// centre pixel the compositor exported for it.
struct View {
    label: &'static str,
    exportable: *mut c_void,
    backend: *mut c_void,
    web_view: *mut c_void,
    image_target: ImageTargetTexture2DOes,
    last_pixel: Option<[u8; 4]>,
    frames: u32,
    web_process_terminated: bool,
}

/// The views live for the whole run and the export callbacks are plain C
/// function pointers, so a static registry is the honest shape here.
static mut VIEWS: Vec<Box<View>> = Vec::new();

fn views() -> &'static mut Vec<Box<View>> {
    #[allow(static_mut_refs)]
    unsafe {
        &mut VIEWS
    }
}

extern "C" fn on_export_fdo_egl_image(data: *mut c_void, image: *mut c_void) {
    let view = unsafe { &mut *(data as *mut View) };
    view.frames += 1;
    let egl_image = unsafe { wpe_fdo_egl_exported_image_get_egl_image(image) };
    let width = unsafe { wpe_fdo_egl_exported_image_get_width(image) };
    let height = unsafe { wpe_fdo_egl_exported_image_get_height(image) };
    if !egl_image.is_null() && width > 0 && height > 0 {
        if let Ok(px) = read_centre_rgba(view.image_target, egl_image, width, height) {
            // Spike B's lesson: the FIRST exported frame is blank, before the
            // page paints. An all-zero pixel is not a colour, it is "nothing
            // yet" — never record it as the view's state.
            if px != [0, 0, 0, 0] {
                view.last_pixel = Some(px);
            }
        }
    }
    unsafe {
        wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
            view.exportable,
            image,
        );
        wpe_view_backend_exportable_fdo_dispatch_frame_complete(view.exportable);
    }
}

extern "C" fn on_export_egl_image(data: *mut c_void, _image: *mut c_void) {
    let view = unsafe { &mut *(data as *mut View) };
    view.frames += 1;
    unsafe { wpe_view_backend_exportable_fdo_dispatch_frame_complete(view.exportable) };
}

extern "C" fn on_web_process_terminated(_view: *mut c_void, _reason: c_int, data: *mut c_void) {
    let view = unsafe { &mut *(data as *mut View) };
    view.web_process_terminated = true;
    eprintln!("[spike] {}: web-process-terminated fired", view.label);
}

/// ⚠ THE CLIENT STRUCT MUST OUTLIVE THE BACKEND.
///
/// `wpe_view_backend_exportable_fdo_egl_create` STORES this pointer; it does
/// not copy the struct. Spike C first declared it as a local inside the
/// per-view constructor and crashed with SIGSEGV as soon as the main loop
/// dispatched a frame, at a different point on each run — the classic signature
/// of reading a freed stack frame.
///
/// Spikes A and B never hit this only because each had exactly one view and
/// declared the client inside `main`, where it happened to live for the whole
/// program. The moment views are created in a function — which any real
/// multi-view engine does — the bug is immediate. Worth knowing before someone
/// spends a day on it.
static EXPORT_CLIENT: WpeViewBackendExportableFdoEglClient =
    WpeViewBackendExportableFdoEglClient {
        export_egl_image: Some(on_export_egl_image),
        export_fdo_egl_image: Some(on_export_fdo_egl_image),
        export_shm_buffer: None,
        reserved0: None,
        reserved1: None,
    };

fn make_view(
    label: &'static str,
    image_target: ImageTargetTexture2DOes,
    url: &str,
) -> Result<usize, String> {
    let mut view = Box::new(View {
        label,
        exportable: ptr::null_mut(),
        backend: ptr::null_mut(),
        web_view: ptr::null_mut(),
        image_target,
        last_pixel: None,
        frames: 0,
        web_process_terminated: false,
    });
    let view_ptr = view.as_mut() as *mut View as *mut c_void;

    unsafe {
        let exportable =
            wpe_view_backend_exportable_fdo_egl_create(&EXPORT_CLIENT, view_ptr, VIEW_W, VIEW_H);
        if exportable.is_null() {
            return Err(format!("{label}: exportable backend is NULL"));
        }
        view.exportable = exportable;
        let backend = wpe_view_backend_exportable_fdo_get_view_backend(exportable);
        view.backend = backend;

        // Mark the view visible + focused + in-window.
        //
        // ⚠ I expected this to be load-bearing for keyboard delivery and said
        // so; a NEGATIVE CONTROL falsified that. With this call removed, the
        // click AND the keystroke both still land on WPE 2.52.5 + fdo — steps
        // 1-3 pass unchanged. So it is NOT what makes input work, and nobody
        // should cargo-cult it as the fix when their input is being dropped.
        //
        // It is kept because activity state is what a real embedder owes the
        // engine for visibility and occlusion semantics — the throttling
        // machinery the optimization pass cares about keys on exactly this, and
        // a headless view that never declares itself visible is the "unrevealed
        // surfaces report visible, so their pages never throttle" bug from the
        // other direction. Correct to set; just not the input gate.
        wpe_view_backend_add_activity_state(
            backend,
            WPE_ACTIVITY_VISIBLE | WPE_ACTIVITY_FOCUSED | WPE_ACTIVITY_IN_WINDOW,
        );

        let wvb = webkit_web_view_backend_new(backend, None, ptr::null_mut());
        let web_view = webkit_web_view_new(wvb);
        if web_view.is_null() {
            return Err(format!("{label}: webkit_web_view_new returned NULL"));
        }
        view.web_view = web_view;

        let signal = CString::new("web-process-terminated").unwrap();
        g_signal_connect_data(
            web_view,
            signal.as_ptr(),
            on_web_process_terminated as *mut c_void,
            view_ptr,
            ptr::null_mut(),
            0,
        );

        let c_url = CString::new(url).unwrap();
        webkit_web_view_load_uri(web_view, c_url.as_ptr());
    }

    views().push(view);
    Ok(views().len() - 1)
}

fn pump_until(deadline: Instant, mut done: impl FnMut() -> bool) -> bool {
    while Instant::now() < deadline {
        pump();
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

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

/// Wait until this view's exported frame shows `expected`.
fn expect_colour(index: usize, expected: &str, timeout: Duration) -> Result<[u8; 4], String> {
    // Clear the recorded pixel so we cannot pass on a STALE frame from before
    // the input we are testing.
    views()[index].last_pixel = None;
    let deadline = Instant::now() + timeout;
    pump_until(deadline, || {
        views()[index]
            .last_pixel
            .is_some_and(|px| dominant(px) == expected)
    });
    match views()[index].last_pixel {
        Some(px) if dominant(px) == expected => Ok(px),
        Some(px) => Err(format!(
            "{}: expected {expected}, frame shows {} {px:?}",
            views()[index].label,
            dominant(px)
        )),
        None => Err(format!(
            "{}: no painted frame arrived while waiting for {expected} ({} frames exported)",
            views()[index].label,
            views()[index].frames
        )),
    }
}

fn click_centre(index: usize) {
    let backend = views()[index].backend;
    let (x, y) = ((VIEW_W / 2) as c_int, (VIEW_H / 2) as c_int);
    unsafe {
        // Motion first: without it the hit test has no position and the button
        // event lands at (0,0).
        let motion = WpeInputPointerEvent {
            event_type: WPE_POINTER_EVENT_MOTION,
            time: 1,
            x,
            y,
            button: 0,
            state: 0,
            modifiers: 0,
        };
        wpe_view_backend_dispatch_pointer_event(backend, &motion);
        let down = WpeInputPointerEvent {
            event_type: WPE_POINTER_EVENT_BUTTON,
            time: 2,
            x,
            y,
            button: 1,
            state: 1,
            modifiers: 1 << 20,
        };
        wpe_view_backend_dispatch_pointer_event(backend, &down);
        let up = WpeInputPointerEvent {
            event_type: WPE_POINTER_EVENT_BUTTON,
            time: 3,
            x,
            y,
            button: 1,
            state: 0,
            modifiers: 0,
        };
        wpe_view_backend_dispatch_pointer_event(backend, &up);
    }
}

fn type_x(index: usize) {
    let backend = views()[index].backend;
    unsafe {
        // `key_code` is an XKB KEYSYM (XK_x = 0x78), NOT an ASCII byte and not a
        // scancode; `hardware_key_code` is the evdev code + 8 (KEY_X 45 -> 53).
        // Getting either wrong produces a silently ignored event.
        let down = WpeInputKeyboardEvent {
            time: 10,
            key_code: 0x78,
            hardware_key_code: 53,
            pressed: true,
            modifiers: 0,
        };
        wpe_view_backend_dispatch_keyboard_event(backend, &down);
        let up = WpeInputKeyboardEvent {
            time: 11,
            key_code: 0x78,
            hardware_key_code: 53,
            pressed: false,
            modifiers: 0,
        };
        wpe_view_backend_dispatch_keyboard_event(backend, &up);
    }
}

fn fail(step: &str, err: String) -> ExitCode {
    eprintln!("[spike] FAIL at {step}: {err}");
    println!("[spike] ACCEPTANCE=FAIL");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let base = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8742".to_string());
    println!("[spike] spike_c_new_fn_count={SPIKE_C_NEW_FN_COUNT} (beyond spikes A+B)");
    println!(
        "[spike] DISPLAY={:?} WAYLAND_DISPLAY={:?}",
        std::env::var("DISPLAY").ok(),
        std::env::var("WAYLAND_DISPLAY").ok()
    );

    let image_target = match bring_up_headless() {
        Ok(target) => target,
        Err(err) => return fail("headless bring-up", err),
    };
    println!("[spike] headless bring-up: ok");

    // ---------------- PART 1: input ----------------
    let a = match make_view("view-a", image_target, &format!("{base}/interactive.html")) {
        Ok(index) => index,
        Err(err) => return fail("create view-a", err),
    };
    match expect_colour(a, "red", Duration::from_secs(20)) {
        Ok(px) => println!("[spike] 1. view-a painted its initial state: red {px:?}"),
        Err(err) => return fail("initial paint", err),
    }
    println!("[spike]    child processes now: {:?}", child_processes());

    click_centre(a);
    match expect_colour(a, "green", Duration::from_secs(15)) {
        Ok(px) => println!(
            "[spike] 2. CLICK LANDED — page turned green {px:?} (only a real pointerdown \
             can do that)"
        ),
        Err(err) => return fail("click", err),
    }

    type_x(a);
    match expect_colour(a, "blue", Duration::from_secs(15)) {
        Ok(px) => println!(
            "[spike] 3. KEYSTROKE LANDED — page turned blue {px:?} (only a real keydown \
             with e.key === 'x' can do that)"
        ),
        Err(err) => return fail("keystroke", err),
    }

    // ---------------- PART 2: per-view lifecycle ----------------
    let b = match make_view("view-b", image_target, &format!("{base}/interactive.html")) {
        Ok(index) => index,
        Err(err) => return fail("create view-b", err),
    };
    match expect_colour(b, "red", Duration::from_secs(20)) {
        Ok(px) => println!("[spike] 4. view-b painted independently: red {px:?}"),
        Err(err) => return fail("view-b paint", err),
    }

    let pids = web_process_pids();
    println!("[spike] 5. WPEWebProcess children: {pids:?}");
    if pids.is_empty() {
        return fail("lifecycle", "no WPEWebProcess children found".into());
    }
    if pids.len() < 2 {
        println!(
            "[spike]    NOTE: {} process for 2 views — the default WebKitWebContext \
             SHARES a web process across views. Per-view isolation is NOT free.",
            pids.len()
        );
    }

    // Kill the FIRST web process and see who notices.
    let victim = pids[0];
    unsafe {
        libc_kill(victim as c_int, 9);
    }
    println!("[spike] 6. killed WPEWebProcess {victim}");

    let deadline = Instant::now() + Duration::from_secs(15);
    pump_until(deadline, || {
        views().iter().any(|v| v.web_process_terminated)
    });
    let terminated: Vec<&str> = views()
        .iter()
        .filter(|v| v.web_process_terminated)
        .map(|v| v.label)
        .collect();
    println!("[spike] 7. views reporting web-process-terminated: {terminated:?}");
    if terminated.is_empty() {
        return fail(
            "lifecycle detection",
            "no view reported web-process-terminated — a dead view would be \
             UNDETECTABLE, which is the thing this step exists to rule out"
                .into(),
        );
    }

    let survivors: Vec<usize> = (0..views().len())
        .filter(|i| !views()[*i].web_process_terminated)
        .collect();
    if survivors.is_empty() {
        return fail(
            "isolation",
            "BOTH views died from one kill — the views share a web process, so per-view \
             lifecycle isolation does not exist"
                .into(),
        );
    }
    for index in survivors {
        // A STATIC page never repaints, so "wait for a new frame" can never
        // succeed and would only prove the test is wrong. Drive the survivor
        // instead: a click it answers is proof its web process is alive AND
        // still processing input, which is what "keeps working" has to mean.
        click_centre(index);
        match expect_colour(index, "green", Duration::from_secs(15)) {
            Ok(px) => println!(
                "[spike] 8. survivor {} STILL INTERACTIVE after the kill — answered a click \
                 {px:?}",
                views()[index].label
            ),
            Err(err) => return fail("survivor interactivity", err),
        }
    }

    // Restart the casualty: reload spawns a fresh web process.
    let casualty = (0..views().len())
        .find(|i| views()[*i].web_process_terminated)
        .expect("at least one terminated");
    unsafe { webkit_web_view_reload(views()[casualty].web_view) };
    // Both fixtures are interactive.html, which paints RED from scratch.
    match expect_colour(casualty, "red", Duration::from_secs(20)) {
        Ok(px) => println!(
            "[spike] 9. RESTARTED {} via reload — painting again: red {px:?}",
            views()[casualty].label
        ),
        Err(err) => return fail("restart", err),
    }
    // …and a restarted view must be INTERACTIVE again, not merely painting.
    click_centre(casualty);
    match expect_colour(casualty, "green", Duration::from_secs(15)) {
        Ok(px) => println!(
            "[spike] 10. restarted {} answers input again {px:?}",
            views()[casualty].label
        ),
        Err(err) => return fail("restart interactivity", err),
    }
    println!(
        "[spike]    web processes after restart: {:?}",
        web_process_pids()
    );

    println!("[spike] ACCEPTANCE=PASS");
    ExitCode::SUCCESS
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: c_int, sig: c_int) -> c_int;
}

// Silence the unused-import warning for constants only some paths use.
#[allow(dead_code)]
fn _unused(_: c_uint) {}
