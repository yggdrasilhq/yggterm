//! **yggterm-wpe** — the headless WPE WebKit engine for yggterm's agent
//! surfaces (Lane A, increment 1).
//!
//! ## Why this crate exists
//!
//! WPE is the settled destination for the web engine, agent surfaces first: an
//! agent-driven page that renders on a server host costs the GUI host exactly
//! zero, and WPE views are not GtkWidgets, so the focus-grab and
//! widget-stacking bug classes stop existing rather than getting worked around.
//! Four spikes (`docs/spikes/wpe-lane-a/`, `docs/spikes/pty-fd-handoff/`)
//! emptied the unknown list; this is where the proven parts become a library.
//!
//! ## What it does NOT do yet
//!
//! No GUI integration, no consumers, no workspace membership. The crate builds
//! and tests standalone — see the note in `Cargo.toml` for why it is
//! deliberately detached.
//!
//! ## Every spike gotcha is a shape here, not a comment
//!
//! The spikes cost real time to four specific mistakes. Each one is now
//! unrepresentable or handled at the source rather than left for a caller to
//! remember:
//!
//! | Gotcha | How this crate forecloses it |
//! | --- | --- |
//! | The fdo client struct is stored by pointer; a per-view local dangles and SIGSEGVs later, at a different place each run | There is exactly ONE client, in `static` storage (`view::EXPORT_CLIENT`). No per-view allocation exists to drop early. |
//! | `wpe_loader_init` must run before anything | [`View`] can only be built from an [`Engine`], and [`Engine::new_headless`] is the only constructor. |
//! | A pointer BUTTON event without a preceding MOTION hit-tests at (0,0) | No raw button dispatch is exposed; [`View::click`] sends motion→down→up. |
//! | `key_code` is an XKB keysym, not ASCII — a wrong one is silently swallowed | No key code crosses the API. [`View::type_text`] takes text; [`keysym`] owns the encoding and REFUSES characters it cannot type. |
//! | `eglChooseConfig` defaults to `EGL_WINDOW_BIT`, of which a surfaceless display has none | Handled inside bring-up; not a caller concern. |
//! | The first exported frame is BLANK, and every success check passes on it | [`View::last_frame`] can never return a blank frame; the skip is counted, not silent. |
//! | GL's origin is bottom-left, every image format's is top-left | [`Frame`] is flipped once, at the source. |
//! | Web processes are bubblewrap GRANDCHILDREN with `comm` truncated to 15 chars | [`supervisor::web_processes`] walks descendants and prefix-matches. |
//!
//! ## Shape
//!
//! ```text
//! Engine            one-time headless bring-up; owns EGL + the GLES2 context
//!   └── View        one page: navigate, readback, click, type
//! Supervisor        owns N views + the process→view map WebKit does not provide
//! ```
//!
//! ```no_run
//! use std::time::Duration;
//! use yggterm_wpe::{Engine, Supervisor};
//!
//! let engine = Engine::new_headless()?;
//! let mut sup = Supervisor::new(&engine);
//! let id = sup.open("http://127.0.0.1:8080/", 320, 240, Duration::from_secs(20))?;
//! sup.view(id)?.click_centre();
//! sup.await_frame(id, Duration::from_secs(10), |f| f.centre_pixel()[1] > 200)?;
//! # Ok::<(), yggterm_wpe::Error>(())
//! ```

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod agent;
mod ffi;
mod frame;
pub mod json;
mod png;
pub mod keysym;
mod supervisor;
mod view;

pub use agent::AgentState;
pub use frame::Frame;
pub use supervisor::{Supervisor, ViewId, WebProcess, descendants, web_processes};
pub use view::View;

/// Everything that can go wrong, named specifically enough to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The engine was brought up more than once in this process.
    AlreadyInitialised,
    BringUp(&'static str),
    ViewCreation(&'static str),
    Readback(&'static str),
    InvalidUri,
    /// The script contained an interior NUL.
    InvalidScript,
    /// The page's JavaScript threw, or the evaluation could not complete.
    EvalFailed(String),
    /// An evaluation did not settle before its deadline.
    EvalTimedOut,
    /// This crate has no keysym for that character. Refused rather than guessed
    /// — a wrong keysym is silently swallowed by WebKit.
    UntypableCharacter(char),
    NoSuchView(usize),
    NoWebProcess(usize),
    NeverPainted {
        uri: String,
        frames: u32,
        blank: u32,
    },
    RestartFailed(usize),
    FrameNeverMatched(usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyInitialised => write!(
                f,
                "the WPE engine was already brought up in this process; EGL, the GL context \
                 and libwpe's loader are process-global, so a second Engine would fight the \
                 first"
            ),
            Error::BringUp(what) => write!(f, "headless bring-up failed: {what}"),
            Error::ViewCreation(what) => write!(f, "could not create a view: {what}"),
            Error::Readback(what) => write!(f, "frame readback failed: {what}"),
            Error::InvalidUri => write!(f, "the URI contained an interior NUL"),
            Error::InvalidScript => write!(f, "the script contained an interior NUL"),
            Error::EvalFailed(msg) => write!(f, "the page's JavaScript failed: {msg}"),
            Error::EvalTimedOut => write!(f, "the evaluation did not settle before its deadline"),
            Error::UntypableCharacter(ch) => write!(
                f,
                "no keysym for {ch:?}; refusing to guess, because a wrong keysym produces an \
                 event WebKit silently ignores"
            ),
            Error::NoSuchView(i) => write!(f, "no view with index {i}"),
            Error::NoWebProcess(i) => write!(
                f,
                "no web process is attributed to view {i} — the open-time diff could not \
                 identify one"
            ),
            Error::NeverPainted { uri, frames, blank } => write!(
                f,
                "{uri} never painted ({frames} frame(s) exported, {blank} of them blank)"
            ),
            Error::RestartFailed(i) => write!(f, "view {i} did not repaint after a reload"),
            Error::FrameNeverMatched(i) => {
                write!(f, "view {i} never painted a frame matching the condition")
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Process-global: libwpe's loader, the EGL display and the current GL context
/// are all per-process, so a second [`Engine`] would fight the first.
static ENGINE_BROUGHT_UP: AtomicBool = AtomicBool::new(false);

/// The headless engine. One per process.
///
/// Its constructor performs the whole bring-up in the one correct order, and
/// [`View`]s can only be made from it — so "did `wpe_loader_init` run first?"
/// is answered by the type system rather than by a comment.
pub struct Engine {
    image_target: ffi::ImageTargetTexture2DOes,
}

impl Engine {
    /// Bring up WPE with no display server of any kind.
    pub fn new_headless() -> Result<Engine> {
        if ENGINE_BROUGHT_UP.swap(true, Ordering::SeqCst) {
            return Err(Error::AlreadyInitialised);
        }
        unsafe {
            // Debian ships NO libWPEBackend-default.so, so naming the backend is
            // mandatory, not a preference.
            let backend = CString::new("libWPEBackend-fdo-1.0.so").expect("static");
            if ffi::wpe_loader_init(backend.as_ptr()) == 0 {
                return Err(Error::BringUp("wpe_loader_init returned FALSE"));
            }

            let dpy = ffi::eglGetPlatformDisplay(
                ffi::EGL_PLATFORM_SURFACELESS_MESA,
                ptr::null_mut(),
                ptr::null(),
            );
            let (mut major, mut minor) = (0, 0);
            if dpy.is_null() || ffi::eglInitialize(dpy, &mut major, &mut minor) == 0 {
                return Err(Error::BringUp("no surfaceless EGL display"));
            }
            if ffi::eglBindAPI(ffi::EGL_OPENGL_ES_API) == 0 {
                return Err(Error::BringUp("eglBindAPI(OPENGL_ES) failed"));
            }

            // EGL_PBUFFER_BIT, not the default EGL_WINDOW_BIT: a surfaceless
            // display has zero window configs, so the obvious attribute list
            // matches nothing.
            let attrs: [i32; 13] = [
                ffi::EGL_RENDERABLE_TYPE,
                ffi::EGL_OPENGL_ES2_BIT,
                ffi::EGL_SURFACE_TYPE,
                ffi::EGL_PBUFFER_BIT,
                ffi::EGL_RED_SIZE,
                8,
                ffi::EGL_GREEN_SIZE,
                8,
                ffi::EGL_BLUE_SIZE,
                8,
                ffi::EGL_ALPHA_SIZE,
                8,
                ffi::EGL_NONE,
            ];
            let mut config: *mut c_void = ptr::null_mut();
            let mut count: i32 = 0;
            if ffi::eglChooseConfig(dpy, attrs.as_ptr(), &mut config, 1, &mut count) == 0
                || count == 0
            {
                // EGL_KHR_no_config_context: legitimate here, since no surface
                // is ever bound and a config describes nothing we use.
                config = ptr::null_mut();
            }
            let ctx_attrs: [i32; 3] = [ffi::EGL_CONTEXT_CLIENT_VERSION, 2, ffi::EGL_NONE];
            let gl = ffi::eglCreateContext(dpy, config, ptr::null_mut(), ctx_attrs.as_ptr());
            if gl.is_null() {
                return Err(Error::BringUp("eglCreateContext returned NULL"));
            }
            if ffi::eglMakeCurrent(dpy, ptr::null_mut(), ptr::null_mut(), gl) == 0 {
                return Err(Error::BringUp(
                    "eglMakeCurrent with EGL_NO_SURFACE failed (EGL_KHR_surfaceless_context?)",
                ));
            }

            // An EXTENSION, so not a link-time symbol.
            let name = CString::new("glEGLImageTargetTexture2DOES").expect("static");
            let proc_addr = ffi::eglGetProcAddress(name.as_ptr());
            if proc_addr.is_null() {
                return Err(Error::BringUp(
                    "glEGLImageTargetTexture2DOES unavailable (GL_OES_EGL_image)",
                ));
            }
            let image_target =
                std::mem::transmute::<*mut c_void, ffi::ImageTargetTexture2DOes>(proc_addr);

            if ffi::wpe_fdo_initialize_for_egl_display(dpy) == 0 {
                return Err(Error::BringUp(
                    "wpe_fdo_initialize_for_egl_display returned FALSE",
                ));
            }
            Ok(Engine { image_target })
        }
    }

    /// A fresh headless view. Not attached to any supervisor — most callers
    /// want [`Supervisor::open`] instead.
    pub fn view(&self, width: u32, height: u32) -> Result<View> {
        View::new(self.image_target, width, height)
    }

    /// Iterate the GLib main context without blocking. Drives frame export,
    /// signal delivery and IPC with the web processes.
    pub fn pump(&self) {
        unsafe {
            while ffi::g_main_context_iteration(ptr::null_mut(), 0) != 0 {}
        }
    }
}

pub(crate) unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// The number of foreign functions this crate declares — the sizing claim the
/// spike READMEs make, kept honest by [`tests::the_declared_ffi_surface_is_the_documented_one`].
pub const FOREIGN_FN_COUNT: usize = ffi::FOREIGN_FN_COUNT;

#[allow(dead_code)]
fn _assert_c_int_is_used(_: c_int) {}

#[cfg(test)]
mod tests {
    /// The FFI floor is the crate's headline cost, quoted in the spike READMEs
    /// and in the Lane-A sizing. Count the declarations in the source so the
    /// number cannot drift from the claim.
    #[test]
    fn the_declared_ffi_surface_is_the_documented_one() {
        let src = include_str!("ffi.rs");
        let declared = src
            .lines()
            .filter(|line| line.trim_start().starts_with("pub(crate) fn "))
            .count();
        assert_eq!(
            declared,
            super::FOREIGN_FN_COUNT,
            "ffi.rs declares {declared} foreign functions but FOREIGN_FN_COUNT says {}. \
             This number is quoted as the Lane-A binding cost — update both together",
            super::FOREIGN_FN_COUNT,
        );
    }

    /// The client struct must be a single `static`. A per-view client is the
    /// SIGSEGV spike C hit, and the whole point of this crate is that the bug is
    /// unrepresentable rather than documented.
    #[test]
    fn there_is_exactly_one_export_client_and_it_is_static() {
        let src = include_str!("view.rs");
        assert!(
            src.contains("static EXPORT_CLIENT: FdoEglClient"),
            "the fdo export client must live in static storage — the backend STORES the \
             pointer, so a per-view client dangles as soon as its constructor returns",
        );
        assert_eq!(
            src.matches("FdoEglClient {").count(),
            1,
            "there must be exactly ONE FdoEglClient in the crate; a second one means some \
             view is building its own, which is the dangling-pointer crash again",
        );
    }

    /// No raw button dispatch may escape the safe layer: a BUTTON event without
    /// a preceding MOTION hit-tests at (0,0).
    #[test]
    fn pointer_button_events_are_always_preceded_by_motion() {
        let src = include_str!("view.rs");
        let motion_at = src
            .find("WPE_POINTER_EVENT_MOTION")
            .expect("a motion event must be constructed");
        let button_at = src
            .find("WPE_POINTER_EVENT_BUTTON")
            .expect("a button event must be constructed");
        assert!(
            motion_at < button_at,
            "the motion event must be built and sent before the button events",
        );
        let dispatches = src.matches("wpe_view_backend_dispatch_pointer_event").count();
        assert_eq!(
            dispatches, 3,
            "click() sends exactly motion + down + up; any other count means a raw button \
             path exists somewhere",
        );
    }
}
