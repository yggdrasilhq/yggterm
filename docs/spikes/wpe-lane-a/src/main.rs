//! O1 — WPE Lane-A reachability spike.
//!
//! ONE question: can Rust drive WPEWebKit headless on this fleet today?
//!
//! Debian sid ships WPE WebKit 2.52.5 WITHOUT WPEPlatform, so there is no
//! `WPEDisplayHeadless`. The only offscreen route is the legacy libwpe +
//! WPEBackend-fdo "exportable" backend, which runs an in-process nested
//! Wayland compositor (libwayland-server) and hands rendered buffers to our
//! callbacks. No display server, no compositor, no X/Wayland socket needed.
//!
//! Acceptance: load http://127.0.0.1:<port>/ , reach WEBKIT_LOAD_FINISHED,
//! and prove content arrived (title read). Exported frame count is captured as
//! a stronger bonus signal: > 0 means the compositor actually produced pixels.

use std::cell::Cell;
use std::env;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::process::ExitCode;
use std::ptr;
use std::sync::OnceLock;
use std::time::Instant;

fn t0() -> Instant {
    static T0: OnceLock<Instant> = OnceLock::new();
    *T0.get_or_init(Instant::now)
}

/// Milestone stamp, milliseconds since process start.
fn ms() -> u128 {
    t0().elapsed().as_millis()
}

// ---------------------------------------------------------------------------
// Binding surface. Hand-written; see BINDING_SURFACE_FN_COUNT below.
// ---------------------------------------------------------------------------

type GBool = c_int;

const EGL_PLATFORM_SURFACELESS_MESA: c_uint = 0x31DD;
const EGL_VENDOR: c_int = 0x3053;
const EGL_VERSION_STR: c_int = 0x3054;

// WebKitLoadEvent
const WEBKIT_LOAD_FINISHED: c_int = 3;

#[repr(C)]
struct WpeViewBackendExportableFdoEglClient {
    export_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    export_fdo_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    export_shm_buffer: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    reserved0: Option<extern "C" fn()>,
    reserved1: Option<extern "C" fn()>,
}

extern "C" {
    // libwpe (1)
    fn wpe_loader_init(impl_library_name: *const c_char) -> GBool;

    // libWPEBackend-fdo (5)
    fn wpe_fdo_initialize_for_egl_display(display: *mut c_void) -> GBool;
    fn wpe_view_backend_exportable_fdo_egl_create(
        client: *const WpeViewBackendExportableFdoEglClient,
        user_data: *mut c_void,
        width: u32,
        height: u32,
    ) -> *mut c_void;
    fn wpe_view_backend_exportable_fdo_get_view_backend(exportable: *mut c_void) -> *mut c_void;
    fn wpe_view_backend_exportable_fdo_dispatch_frame_complete(exportable: *mut c_void);
    fn wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
        exportable: *mut c_void,
        image: *mut c_void,
    );

    // libEGL (3)
    fn eglGetPlatformDisplay(
        platform: c_uint,
        native_display: *mut c_void,
        attrib_list: *const isize,
    ) -> *mut c_void;
    fn eglInitialize(dpy: *mut c_void, major: *mut c_int, minor: *mut c_int) -> c_uint;
    fn eglQueryString(dpy: *mut c_void, name: c_int) -> *const c_char;

    // libWPEWebKit (5)
    fn webkit_web_view_backend_new(
        backend: *mut c_void,
        notify: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    ) -> *mut c_void;
    fn webkit_web_view_new(backend: *mut c_void) -> *mut c_void;
    fn webkit_web_view_load_uri(view: *mut c_void, uri: *const c_char);
    fn webkit_web_view_get_title(view: *mut c_void) -> *const c_char;
    fn webkit_web_view_get_uri(view: *mut c_void) -> *const c_char;

    // glib / gobject (5)
    fn g_main_loop_new(context: *mut c_void, is_running: GBool) -> *mut c_void;
    fn g_main_loop_run(loop_: *mut c_void);
    fn g_main_loop_quit(loop_: *mut c_void);
    fn g_timeout_add(interval_ms: c_uint, func: extern "C" fn(*mut c_void) -> GBool, data: *mut c_void) -> c_uint;
    fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        c_handler: *mut c_void,
        data: *mut c_void,
        destroy_data: *mut c_void,
        connect_flags: c_uint,
    ) -> usize;
}

/// Number of foreign functions this spike had to declare by hand.
const BINDING_SURFACE_FN_COUNT: usize = 19;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct State {
    exportable: Cell<*mut c_void>,
    view: Cell<*mut c_void>,
    main_loop: Cell<*mut c_void>,
    frames: Cell<u32>,
    load_finished: Cell<bool>,
    load_failed: Cell<bool>,
    timed_out: Cell<bool>,
}

impl State {
    fn new() -> Self {
        State {
            exportable: Cell::new(ptr::null_mut()),
            view: Cell::new(ptr::null_mut()),
            main_loop: Cell::new(ptr::null_mut()),
            frames: Cell::new(0),
            load_finished: Cell::new(false),
            load_failed: Cell::new(false),
            timed_out: Cell::new(false),
        }
    }
}

unsafe fn state<'a>(p: *mut c_void) -> &'a State {
    &*(p as *const State)
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

/// The compositor handed us a rendered frame. Releasing it and acking the
/// frame is mandatory: WebKit stalls waiting for the ack otherwise.
extern "C" fn on_export_fdo_egl_image(data: *mut c_void, image: *mut c_void) {
    let st = unsafe { state(data) };
    st.frames.set(st.frames.get() + 1);
    if st.frames.get() == 1 {
        eprintln!("[spike] +{}ms first frame exported", ms());
    }
    unsafe {
        wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
            st.exportable.get(),
            image,
        );
        wpe_view_backend_exportable_fdo_dispatch_frame_complete(st.exportable.get());
    }
}

extern "C" fn on_export_egl_image(data: *mut c_void, _image: *mut c_void) {
    let st = unsafe { state(data) };
    st.frames.set(st.frames.get() + 1);
    unsafe { wpe_view_backend_exportable_fdo_dispatch_frame_complete(st.exportable.get()) };
}

extern "C" fn on_load_changed(_view: *mut c_void, event: c_int, data: *mut c_void) {
    let st = unsafe { state(data) };
    eprintln!("[spike] +{}ms load-changed: {event}", ms());
    if event == WEBKIT_LOAD_FINISHED {
        st.load_finished.set(true);
        // Give the compositor a moment to push at least one frame before we
        // tear the loop down; the frame count is the bonus signal.
        unsafe { g_timeout_add(1500, quit_loop, data) };
    }
}

extern "C" fn on_load_failed(
    _view: *mut c_void,
    _event: c_int,
    uri: *const c_char,
    error: *mut c_void,
    data: *mut c_void,
) -> GBool {
    let st = unsafe { state(data) };
    st.load_failed.set(true);
    let uri = unsafe { cstr(uri) };
    // GError layout: { GQuark domain; gint code; gchar *message; }
    let msg = unsafe {
        if error.is_null() {
            String::from("(null GError)")
        } else {
            cstr(*(error as *const *const c_char).add(1))
        }
    };
    eprintln!("[spike] load-failed uri={uri} error={msg}");
    unsafe { g_main_loop_quit(st.main_loop.get()) };
    0
}

extern "C" fn quit_loop(data: *mut c_void) -> GBool {
    let st = unsafe { state(data) };
    unsafe { g_main_loop_quit(st.main_loop.get()) };
    0 // G_SOURCE_REMOVE
}

extern "C" fn on_timeout(data: *mut c_void) -> GBool {
    let st = unsafe { state(data) };
    st.timed_out.set(true);
    eprintln!("[spike] hard timeout reached");
    unsafe { g_main_loop_quit(st.main_loop.get()) };
    0
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    t0();
    let url = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: wpe-lane-a-spike <url>");
        std::process::exit(2);
    });

    println!("[spike] binding_surface_fn_count={BINDING_SURFACE_FN_COUNT}");
    println!("[spike] DISPLAY={:?} WAYLAND_DISPLAY={:?}", env::var("DISPLAY").ok(), env::var("WAYLAND_DISPLAY").ok());

    unsafe {
        // 1. Point libwpe at the fdo backend. Debian ships NO
        //    libWPEBackend-default.so, so this is not optional here.
        let backend_lib = CString::new("libWPEBackend-fdo-1.0.so").unwrap();
        if wpe_loader_init(backend_lib.as_ptr()) == 0 {
            eprintln!("[spike] FAIL: wpe_loader_init(libWPEBackend-fdo-1.0.so) returned FALSE");
            return ExitCode::from(1);
        }
        println!("[spike] +{}ms wpe_loader_init: ok", ms());

        // 2. Headless EGL via Mesa's surfaceless platform — no DRM master, no
        //    display server, just the render node.
        let egl_display =
            eglGetPlatformDisplay(EGL_PLATFORM_SURFACELESS_MESA, ptr::null_mut(), ptr::null());
        if egl_display.is_null() {
            eprintln!("[spike] FAIL: eglGetPlatformDisplay(EGL_PLATFORM_SURFACELESS_MESA) -> NULL");
            return ExitCode::from(1);
        }
        let (mut major, mut minor) = (0, 0);
        if eglInitialize(egl_display, &mut major, &mut minor) == 0 {
            eprintln!("[spike] FAIL: eglInitialize failed on surfaceless display");
            return ExitCode::from(1);
        }
        println!(
            "[spike] EGL {major}.{minor} vendor={:?} version={:?}",
            cstr(eglQueryString(egl_display, EGL_VENDOR)),
            cstr(eglQueryString(egl_display, EGL_VERSION_STR)),
        );

        // 3. Hand that EGLDisplay to WPEBackend-fdo.
        if wpe_fdo_initialize_for_egl_display(egl_display) == 0 {
            eprintln!("[spike] FAIL: wpe_fdo_initialize_for_egl_display returned FALSE");
            return ExitCode::from(1);
        }
        println!("[spike] +{}ms wpe_fdo_initialize_for_egl_display: ok", ms());

        let st: &'static State = Box::leak(Box::new(State::new()));
        let st_ptr = st as *const State as *mut c_void;

        // 4. Offscreen exportable view backend — this IS the headless surface.
        let client = WpeViewBackendExportableFdoEglClient {
            export_egl_image: Some(on_export_egl_image),
            export_fdo_egl_image: Some(on_export_fdo_egl_image),
            export_shm_buffer: None,
            reserved0: None,
            reserved1: None,
        };
        let exportable =
            wpe_view_backend_exportable_fdo_egl_create(&client, st_ptr, 1280, 720);
        if exportable.is_null() {
            eprintln!("[spike] FAIL: wpe_view_backend_exportable_fdo_egl_create -> NULL");
            return ExitCode::from(1);
        }
        st.exportable.set(exportable);
        let view_backend = wpe_view_backend_exportable_fdo_get_view_backend(exportable);
        println!("[spike] +{}ms exportable fdo backend: ok (1280x720 offscreen)", ms());

        // 5. WebKitWebView over that backend.
        let wvb = webkit_web_view_backend_new(view_backend, None, ptr::null_mut());
        let view = webkit_web_view_new(wvb);
        if view.is_null() {
            eprintln!("[spike] FAIL: webkit_web_view_new -> NULL");
            return ExitCode::from(1);
        }
        st.view.set(view);

        let sig_changed = CString::new("load-changed").unwrap();
        let sig_failed = CString::new("load-failed").unwrap();
        g_signal_connect_data(
            view,
            sig_changed.as_ptr(),
            on_load_changed as *mut c_void,
            st_ptr,
            ptr::null_mut(),
            0,
        );
        g_signal_connect_data(
            view,
            sig_failed.as_ptr(),
            on_load_failed as *mut c_void,
            st_ptr,
            ptr::null_mut(),
            0,
        );

        let main_loop = g_main_loop_new(ptr::null_mut(), 0);
        st.main_loop.set(main_loop);
        g_timeout_add(20_000, on_timeout, st_ptr);

        let c_url = CString::new(url.clone()).unwrap();
        println!("[spike] +{}ms load_uri({url})", ms());
        webkit_web_view_load_uri(view, c_url.as_ptr());

        g_main_loop_run(main_loop);

        let title = cstr(webkit_web_view_get_title(view));
        let final_uri = cstr(webkit_web_view_get_uri(view));
        println!("[spike] ---- RESULT ---- (+{}ms)", ms());
        println!("[spike] load_finished={}", st.load_finished.get());
        println!("[spike] load_failed={}", st.load_failed.get());
        println!("[spike] timed_out={}", st.timed_out.get());
        println!("[spike] title={title:?}");
        println!("[spike] uri={final_uri:?}");
        println!("[spike] frames_exported={}", st.frames.get());

        let pass = st.load_finished.get() && !st.load_failed.get() && !title.is_empty();
        println!("[spike] ACCEPTANCE={}", if pass { "PASS" } else { "FAIL" });
        if pass {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        }
    }
}
