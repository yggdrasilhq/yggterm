//! One headless web view: its backend, its `WebKitWebView`, and its last frame.

use std::ffi::{CString, c_char, c_int, c_void};
use std::ptr;

use crate::ffi::{self, FdoEglClient, ImageTargetTexture2DOes};
use crate::frame::Frame;
use crate::keysym;
use crate::{Error, Result};

/// ⚠ **THE ONE CLIENT.** `wpe_view_backend_exportable_fdo_egl_create` STORES
/// this pointer; it does not copy the struct. A per-view client declared as a
/// local — the obvious shape — dangles the moment its constructor returns, and
/// the crash lands later, inside the main loop, at a different point on every
/// run. Spike C lost a debugging session to exactly that.
///
/// Because every view's callbacks are the SAME three function pointers, there
/// is no reason for a per-view client to exist at all. One `static` makes the
/// bug **unrepresentable**: there is no per-view allocation that could be
/// dropped early, and the per-view state travels as `user_data` instead.
static EXPORT_CLIENT: FdoEglClient = FdoEglClient {
    export_egl_image: Some(on_export_egl_image),
    export_fdo_egl_image: Some(on_export_fdo_egl_image),
    export_shm_buffer: None,
    reserved0: None,
    reserved1: None,
};

/// Per-view state reachable from the C callbacks.
///
/// Lives behind a `Box` that the [`View`] owns for its whole life, so the
/// `user_data` pointer handed to the backend stays valid. Moving the `View`
/// moves the `Box` (a pointer), never the pointee.
pub(crate) struct ViewState {
    pub(crate) exportable: *mut c_void,
    pub(crate) image_target: ImageTargetTexture2DOes,
    /// The most recent NON-BLANK frame. See [`Self::record_frame`].
    pub(crate) last_frame: Option<Frame>,
    pub(crate) frames_exported: u32,
    pub(crate) blank_frames_skipped: u32,
    pub(crate) web_process_terminated: bool,
    pub(crate) load_finished: bool,
    /// Bumped on every navigation. A frame recorded under an older generation
    /// belongs to the PREVIOUS document.
    pub(crate) document_generation: u64,
    /// The generation the current `last_frame` was painted under.
    pub(crate) last_frame_generation: u64,
    pub(crate) painted_count: u64,
    /// `Some(Ok(json))` / `Some(Err(message))` once an eval completes; `None`
    /// while one is in flight or none has been issued.
    pub(crate) eval_result: Option<std::result::Result<String, String>>,
    pub(crate) eval_in_flight: bool,
    /// The view this state belongs to. The async callback uses THIS rather than
    /// the `source_object` GLib hands it: we own this pointer and know it is a
    /// WebKitWebView, which the callback parameter only promises by convention.
    pub(crate) web_view: *mut c_void,
}

impl ViewState {
    /// Record a frame, skipping blanks.
    ///
    /// ⚠ **The compositor exports an initial frame BEFORE the page paints.**
    /// Spike B captured it and got 307,200 identical `(0,0,0,0)` pixels through
    /// a pipeline that reported success at every step — context current,
    /// framebuffer complete, `glReadPixels` clean, bytes written. An all-zero
    /// frame is not a colour, it is "nothing yet", so it never becomes the
    /// view's state. The count is kept because a silent skip is its own lie.
    fn record_frame(&mut self, frame: Frame) {
        if frame.is_blank() {
            self.blank_frames_skipped += 1;
        } else {
            self.last_frame = Some(frame);
            self.last_frame_generation = self.document_generation;
            self.painted_count += 1;
        }
    }
}

extern "C" fn on_export_fdo_egl_image(data: *mut c_void, image: *mut c_void) {
    // SAFETY: `data` is the `Box<ViewState>` the View owns for its whole life.
    let state = unsafe { &mut *(data as *mut ViewState) };
    state.frames_exported += 1;

    let egl_image = unsafe { ffi::wpe_fdo_egl_exported_image_get_egl_image(image) };
    let width = unsafe { ffi::wpe_fdo_egl_exported_image_get_width(image) };
    let height = unsafe { ffi::wpe_fdo_egl_exported_image_get_height(image) };
    if !egl_image.is_null() && width > 0 && height > 0 {
        if let Ok(frame) = crate::frame::read_frame(state.image_target, egl_image, width, height)
        {
            state.record_frame(frame);
        }
    }

    // Releasing the image and acking the frame is MANDATORY: WebKit stalls
    // waiting for the ack, and the page never advances past its first paint.
    unsafe {
        ffi::wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
            state.exportable,
            image,
        );
        ffi::wpe_view_backend_exportable_fdo_dispatch_frame_complete(state.exportable);
    }
}

extern "C" fn on_export_egl_image(data: *mut c_void, _image: *mut c_void) {
    let state = unsafe { &mut *(data as *mut ViewState) };
    state.frames_exported += 1;
    unsafe {
        ffi::wpe_view_backend_exportable_fdo_dispatch_frame_complete(state.exportable);
    }
}

extern "C" fn on_web_process_terminated(_view: *mut c_void, _reason: c_int, data: *mut c_void) {
    let state = unsafe { &mut *(data as *mut ViewState) };
    state.web_process_terminated = true;
}

/// GAsyncReadyCallback for `webkit_web_view_evaluate_javascript`.
extern "C" fn on_eval_done(_source: *mut c_void, result: *mut c_void, data: *mut c_void) {
    let state = unsafe { &mut *(data as *mut ViewState) };
    state.eval_in_flight = false;

    let mut error: *mut c_void = ptr::null_mut();
    let value = unsafe {
        ffi::webkit_web_view_evaluate_javascript_finish(state.web_view, result, &mut error)
    };
    if value.is_null() {
        // GError layout is { GQuark domain (u32); gint code (i32); gchar
        // *message }, so the message pointer sits one 64-bit slot in.
        let message = unsafe {
            if error.is_null() {
                "evaluate_javascript failed with no GError".to_string()
            } else {
                let msg_ptr = *(error as *const *const c_char).add(1);
                crate::cstr(msg_ptr)
            }
        };
        state.eval_result = Some(Err(message));
        if !error.is_null() {
            // g_error_free, NOT g_object_unref: a GError is a plain struct, and
            // unreffing it corrupts the heap.
            unsafe { ffi::g_error_free(error) };
        }
        return;
    }

    // `undefined` has no JSON representation; report it as JSON null rather
    // than as an empty string, which a caller would read as "".
    let json = unsafe {
        if ffi::jsc_value_is_undefined(value) != 0 {
            "null".to_string()
        } else {
            let raw = ffi::jsc_value_to_json(value, 0);
            let text = crate::cstr(raw);
            if !raw.is_null() {
                ffi::g_free(raw as *mut c_void);
            }
            text
        }
    };
    unsafe { ffi::g_object_unref(value) };
    state.eval_result = Some(Ok(json));
}

extern "C" fn on_load_changed(_view: *mut c_void, event: c_int, data: *mut c_void) {
    let state = unsafe { &mut *(data as *mut ViewState) };
    if event == ffi::WEBKIT_LOAD_FINISHED {
        state.load_finished = true;
    }
}

/// A headless web view.
///
/// Created through [`crate::Engine::view`], which is the only way to get one —
/// so the engine's one-time bring-up (`wpe_loader_init`, surfaceless EGL, the
/// GLES2 context, the image-import extension) has provably already happened.
pub struct View {
    state: Box<ViewState>,
    backend: *mut c_void,
    web_view: *mut c_void,
    width: u32,
    height: u32,
}

impl View {
    pub(crate) fn new(
        image_target: ImageTargetTexture2DOes,
        width: u32,
        height: u32,
    ) -> Result<View> {
        let mut state = Box::new(ViewState {
            exportable: ptr::null_mut(),
            image_target,
            last_frame: None,
            frames_exported: 0,
            blank_frames_skipped: 0,
            web_process_terminated: false,
            load_finished: false,
            document_generation: 0,
            last_frame_generation: 0,
            painted_count: 0,
            eval_result: None,
            eval_in_flight: false,
            web_view: ptr::null_mut(),
        });
        let state_ptr = state.as_mut() as *mut ViewState as *mut c_void;

        unsafe {
            let exportable = ffi::wpe_view_backend_exportable_fdo_egl_create(
                &EXPORT_CLIENT,
                state_ptr,
                width,
                height,
            );
            if exportable.is_null() {
                return Err(Error::ViewCreation("exportable fdo backend was NULL"));
            }
            state.exportable = exportable;
            let backend = ffi::wpe_view_backend_exportable_fdo_get_view_backend(exportable);

            // Activity state is what an embedder owes the engine for
            // visibility/occlusion — which is what page throttling keys on. It
            // is NOT the input gate: spike C's negative control showed clicks
            // and keystrokes land without it. Set because it is correct, not
            // because input depends on it.
            ffi::wpe_view_backend_add_activity_state(
                backend,
                ffi::WPE_ACTIVITY_VISIBLE
                    | ffi::WPE_ACTIVITY_FOCUSED
                    | ffi::WPE_ACTIVITY_IN_WINDOW,
            );

            let wvb = ffi::webkit_web_view_backend_new(backend, None, ptr::null_mut());
            let web_view = ffi::webkit_web_view_new(wvb);
            if web_view.is_null() {
                return Err(Error::ViewCreation("webkit_web_view_new returned NULL"));
            }

            state.web_view = web_view;
            connect(web_view, "web-process-terminated", on_web_process_terminated as *mut c_void, state_ptr);
            connect(web_view, "load-changed", on_load_changed as *mut c_void, state_ptr);

            Ok(View {
                state,
                backend,
                web_view,
                width,
                height,
            })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn load_uri(&mut self, uri: &str) -> Result<()> {
        let c_uri = CString::new(uri).map_err(|_| Error::InvalidUri)?;
        self.begin_navigation();
        unsafe { ffi::webkit_web_view_load_uri(self.web_view, c_uri.as_ptr()) };
        Ok(())
    }

    /// Reload — the recovery path for a view whose web process died.
    ///
    /// Spike C proved this restores a killed view completely: it paints again
    /// AND answers input again, on a fresh web-process pid. Deliberately NOT
    /// automatic — see [`crate::Supervisor::restart`].
    pub fn reload(&mut self) {
        self.begin_navigation();
        self.state.web_process_terminated = false;
        unsafe { ffi::webkit_web_view_reload(self.web_view) };
    }

    /// Clear the terminated flag — the supervisor does this as part of an
    /// explicit restart, so a recovered view stops reporting itself dead.
    pub(crate) fn clear_termination(&mut self) {
        self.state.web_process_terminated = false;
    }

    pub fn title(&self) -> String {
        unsafe { crate::cstr(ffi::webkit_web_view_get_title(self.web_view)) }
    }

    pub fn uri(&self) -> String {
        unsafe { crate::cstr(ffi::webkit_web_view_get_uri(self.web_view)) }
    }

    pub fn is_loading(&self) -> bool {
        unsafe { ffi::webkit_web_view_is_loading(self.web_view) != 0 }
    }

    pub fn load_finished(&self) -> bool {
        self.state.load_finished
    }

    /// The most recent painted frame, or `None` if the page has not painted.
    ///
    /// Never returns a blank frame — see [`ViewState::record_frame`].
    pub fn last_frame(&self) -> Option<&Frame> {
        self.state.last_frame.as_ref()
    }

    /// Forget the recorded frame, so the next `last_frame()` can only be a
    /// frame painted AFTER this call.
    ///
    /// This is how a caller avoids asserting on a stale frame from before the
    /// input it is testing.
    pub fn forget_frame(&mut self) {
        self.state.last_frame = None;
    }

    /// A navigation has begun: the frame we hold now describes the OLD document.
    fn begin_navigation(&mut self) {
        self.state.load_finished = false;
        self.state.document_generation += 1;
    }

    /// Is the frame we hold a finished picture of the CURRENT document?
    ///
    /// ⚠ **"Has a non-blank frame" is a different question, and the difference
    /// is a bug the caller acts on.** A view still holds the previous page's
    /// frame the instant it is told to navigate, and recovering a killed view
    /// paints an intermediate WHITE frame on the way back. Either one satisfies
    /// "has a frame" and neither is the current document.
    ///
    /// Deliberately generation-based rather than "painted after load-finished":
    /// for a small document the paint can land BEFORE the load-finished signal,
    /// so a post-load-paint counter never becomes true and every navigation
    /// times out. That was the first implementation and it was wrong.
    pub fn painted_current_document(&self) -> bool {
        self.state.load_finished
            && self.state.last_frame.is_some()
            && self.state.last_frame_generation == self.state.document_generation
    }

    pub fn frames_exported(&self) -> u32 {
        self.state.frames_exported
    }

    /// How many blank frames were skipped. Non-zero is NORMAL — the compositor
    /// exports one before the page paints.
    pub fn blank_frames_skipped(&self) -> u32 {
        self.state.blank_frames_skipped
    }

    /// Has this view's web process died? Set by WebKit's own
    /// `web-process-terminated` signal, which spike C proved fires on the
    /// affected view ONLY.
    pub fn web_process_terminated(&self) -> bool {
        self.state.web_process_terminated
    }

    /// Begin evaluating `script` in the page. The result lands asynchronously;
    /// pump until [`Self::take_eval_result`] returns something.
    ///
    /// Not public: an async call that needs the caller to pump is a footgun, so
    /// [`crate::Supervisor::eval`] owns the pump-and-wait and is the only way in.
    pub(crate) fn begin_eval(&mut self, script: &str) -> Result<()> {
        let c_script = CString::new(script).map_err(|_| Error::InvalidScript)?;
        self.state.eval_result = None;
        self.state.eval_in_flight = true;
        let state_ptr = self.state.as_mut() as *mut ViewState as *mut c_void;
        unsafe {
            ffi::webkit_web_view_evaluate_javascript(
                self.web_view,
                c_script.as_ptr(),
                -1,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                Some(on_eval_done),
                state_ptr,
            );
        }
        Ok(())
    }

    pub(crate) fn eval_settled(&self) -> bool {
        self.state.eval_result.is_some()
    }

    pub(crate) fn take_eval_result(&mut self) -> Option<std::result::Result<String, String>> {
        self.state.eval_in_flight = false;
        self.state.eval_result.take()
    }

    /// Click at `(x, y)` in view coordinates.
    ///
    /// ⚠ A pointer BUTTON event alone hit-tests at (0,0): the backend has no
    /// pointer position until a MOTION event gives it one. That is why this
    /// crate exposes `click` and not a raw button dispatch — the motion event is
    /// not the caller's problem to remember.
    pub fn click(&self, x: i32, y: i32) {
        let motion = ffi::WpePointerEvent {
            event_type: ffi::WPE_POINTER_EVENT_MOTION,
            time: 1,
            x,
            y,
            button: 0,
            state: 0,
            modifiers: 0,
        };
        let down = ffi::WpePointerEvent {
            event_type: ffi::WPE_POINTER_EVENT_BUTTON,
            time: 2,
            x,
            y,
            button: 1,
            state: 1,
            modifiers: 1 << 20,
        };
        let up = ffi::WpePointerEvent {
            event_type: ffi::WPE_POINTER_EVENT_BUTTON,
            time: 3,
            x,
            y,
            button: 1,
            state: 0,
            modifiers: 0,
        };
        unsafe {
            ffi::wpe_view_backend_dispatch_pointer_event(self.backend, &motion);
            ffi::wpe_view_backend_dispatch_pointer_event(self.backend, &down);
            ffi::wpe_view_backend_dispatch_pointer_event(self.backend, &up);
        }
    }

    /// Click the centre of the view.
    pub fn click_centre(&self) {
        self.click((self.width / 2) as i32, (self.height / 2) as i32);
    }

    /// Type `text` as real key events.
    ///
    /// The caller never supplies a key code: `key_code` must be an XKB keysym
    /// and `hardware_key_code` the evdev code + 8, and getting either wrong
    /// produces an event WebKit silently swallows. A character this crate
    /// cannot type is an [`Error::UntypableCharacter`] rather than a guess, for
    /// the same reason.
    pub fn type_text(&self, text: &str) -> Result<()> {
        for ch in text.chars() {
            let stroke =
                keysym::stroke_for_char(ch).ok_or(Error::UntypableCharacter(ch))?;
            self.send_stroke(stroke);
        }
        Ok(())
    }

    /// Press a named key (see [`keysym`]).
    pub fn press_key(&self, sym: u32) {
        self.send_stroke(keysym::stroke_for_keysym(sym));
    }

    fn send_stroke(&self, stroke: keysym::KeyStroke) {
        let modifiers = if stroke.shift { 1 << 1 } else { 0 };
        let down = ffi::WpeKeyboardEvent {
            time: 10,
            key_code: stroke.keysym,
            hardware_key_code: stroke.hardware_code,
            pressed: true,
            modifiers,
        };
        let up = ffi::WpeKeyboardEvent {
            time: 11,
            key_code: stroke.keysym,
            hardware_key_code: stroke.hardware_code,
            pressed: false,
            modifiers,
        };
        unsafe {
            ffi::wpe_view_backend_dispatch_keyboard_event(self.backend, &down);
            ffi::wpe_view_backend_dispatch_keyboard_event(self.backend, &up);
        }
    }
}

unsafe fn connect(instance: *mut c_void, signal: &str, handler: *mut c_void, data: *mut c_void) {
    let name = CString::new(signal).expect("signal names are static and ASCII");
    unsafe {
        ffi::g_signal_connect_data(
            instance,
            name.as_ptr(),
            handler,
            data,
            ptr::null_mut(),
            0,
        );
    }
}

impl Drop for View {
    fn drop(&mut self) {
        // The WebKitWebView owns the backend wrapper; releasing our reference is
        // what tears the view down. The Box<ViewState> then drops normally, and
        // because the backend is gone first, no callback can reach it after.
        if !self.web_view.is_null() {
            unsafe { ffi::g_object_unref(self.web_view) };
            self.web_view = ptr::null_mut();
        }
    }
}
