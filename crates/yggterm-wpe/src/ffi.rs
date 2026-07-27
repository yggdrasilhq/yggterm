//! The FFI floor — every foreign symbol the engine needs, declared once.
//!
//! Hand-written by necessity and by choice. Debian ships **no `.gir`/typelib
//! for WPE at all** (`gir1.2-webkit-*` is the GTK port), so gir-based binding
//! generation cannot be executed against these libraries without rebuilding WPE
//! from source with introspection. The surface turned out to be 46 declarations
//! across three spikes, which is smaller than the toolchain it replaces.
//!
//! **This module is `pub(crate)` on purpose.** Nothing outside the crate should
//! ever hold a raw `wpe_view_backend` or dispatch a raw pointer event — the
//! whole point of the safe layer above is that the spike-proven gotchas become
//! unrepresentable rather than documented.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_uint, c_void};

pub(crate) type GBool = c_int;

// ---- EGL -----------------------------------------------------------------

pub(crate) const EGL_PLATFORM_SURFACELESS_MESA: c_uint = 0x31DD;
pub(crate) const EGL_NONE: i32 = 0x3038;
pub(crate) const EGL_OPENGL_ES_API: c_uint = 0x30A0;
pub(crate) const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
pub(crate) const EGL_RENDERABLE_TYPE: i32 = 0x3040;
pub(crate) const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
pub(crate) const EGL_RED_SIZE: i32 = 0x3024;
pub(crate) const EGL_GREEN_SIZE: i32 = 0x3023;
pub(crate) const EGL_BLUE_SIZE: i32 = 0x3022;
pub(crate) const EGL_ALPHA_SIZE: i32 = 0x3021;
pub(crate) const EGL_SURFACE_TYPE: i32 = 0x3033;
/// ⚠ `eglChooseConfig` defaults `EGL_SURFACE_TYPE` to `EGL_WINDOW_BIT`, and a
/// SURFACELESS display has zero window configs — the obvious attribute list
/// returns no matches at all. Ask for pbuffer.
pub(crate) const EGL_PBUFFER_BIT: i32 = 0x0001;

// ---- GLES2 ---------------------------------------------------------------

pub(crate) const GL_TEXTURE_2D: c_uint = 0x0DE1;
pub(crate) const GL_TEXTURE_MIN_FILTER: c_uint = 0x2801;
pub(crate) const GL_TEXTURE_MAG_FILTER: c_uint = 0x2800;
pub(crate) const GL_NEAREST: c_int = 0x2600;
pub(crate) const GL_FRAMEBUFFER: c_uint = 0x8D40;
pub(crate) const GL_COLOR_ATTACHMENT0: c_uint = 0x8CE0;
pub(crate) const GL_FRAMEBUFFER_COMPLETE: c_uint = 0x8CD5;
pub(crate) const GL_RGBA: c_uint = 0x1908;
pub(crate) const GL_UNSIGNED_BYTE: c_uint = 0x1401;
pub(crate) const GL_NO_ERROR: c_uint = 0;

// ---- libwpe --------------------------------------------------------------

pub(crate) const WPE_ACTIVITY_VISIBLE: u32 = 1 << 0;
pub(crate) const WPE_ACTIVITY_FOCUSED: u32 = 1 << 1;
pub(crate) const WPE_ACTIVITY_IN_WINDOW: u32 = 1 << 2;

pub(crate) const WPE_POINTER_EVENT_MOTION: c_uint = 1;
pub(crate) const WPE_POINTER_EVENT_BUTTON: c_uint = 2;

pub(crate) const WEBKIT_LOAD_FINISHED: c_int = 3;

/// The fdo export client. **The backend STORES this pointer; it does not copy
/// the struct.** See [`crate::view`] for why this crate has exactly one, in
/// static storage, and why per-view clients are not offered.
#[repr(C)]
pub(crate) struct FdoEglClient {
    pub export_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    pub export_fdo_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    pub export_shm_buffer: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    pub reserved0: Option<extern "C" fn()>,
    pub reserved1: Option<extern "C" fn()>,
}

// SAFETY: the struct is nothing but function pointers, which are Sync.
unsafe impl Sync for FdoEglClient {}

#[repr(C)]
pub(crate) struct WpeKeyboardEvent {
    pub time: u32,
    /// An **XKB keysym** (`XK_x` = 0x78) — not ASCII, not a scancode.
    pub key_code: u32,
    /// The evdev code + 8.
    pub hardware_key_code: u32,
    pub pressed: bool,
    pub modifiers: u32,
}

#[repr(C)]
pub(crate) struct WpePointerEvent {
    pub event_type: c_uint,
    pub time: u32,
    pub x: c_int,
    pub y: c_int,
    pub button: u32,
    pub state: u32,
    pub modifiers: u32,
}

unsafe extern "C" {
    // libwpe (4)
    pub(crate) fn wpe_loader_init(name: *const c_char) -> GBool;
    pub(crate) fn wpe_view_backend_add_activity_state(backend: *mut c_void, state: u32);
    pub(crate) fn wpe_view_backend_dispatch_pointer_event(
        backend: *mut c_void,
        event: *const WpePointerEvent,
    );
    pub(crate) fn wpe_view_backend_dispatch_keyboard_event(
        backend: *mut c_void,
        event: *const WpeKeyboardEvent,
    );

    // WPEBackend-fdo (8)
    pub(crate) fn wpe_fdo_initialize_for_egl_display(display: *mut c_void) -> GBool;
    pub(crate) fn wpe_view_backend_exportable_fdo_egl_create(
        client: *const FdoEglClient,
        user_data: *mut c_void,
        width: u32,
        height: u32,
    ) -> *mut c_void;
    pub(crate) fn wpe_view_backend_exportable_fdo_get_view_backend(
        exportable: *mut c_void,
    ) -> *mut c_void;
    pub(crate) fn wpe_view_backend_exportable_fdo_dispatch_frame_complete(
        exportable: *mut c_void,
    );
    pub(crate) fn wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
        exportable: *mut c_void,
        image: *mut c_void,
    );
    pub(crate) fn wpe_fdo_egl_exported_image_get_egl_image(image: *mut c_void) -> *mut c_void;
    pub(crate) fn wpe_fdo_egl_exported_image_get_width(image: *mut c_void) -> u32;
    pub(crate) fn wpe_fdo_egl_exported_image_get_height(image: *mut c_void) -> u32;

    // EGL (7)
    pub(crate) fn eglGetPlatformDisplay(
        platform: c_uint,
        native_display: *mut c_void,
        attrib_list: *const isize,
    ) -> *mut c_void;
    pub(crate) fn eglInitialize(dpy: *mut c_void, major: *mut c_int, minor: *mut c_int)
    -> c_uint;
    pub(crate) fn eglBindAPI(api: c_uint) -> c_uint;
    pub(crate) fn eglChooseConfig(
        dpy: *mut c_void,
        attrib_list: *const i32,
        configs: *mut *mut c_void,
        config_size: i32,
        num_config: *mut i32,
    ) -> c_uint;
    pub(crate) fn eglCreateContext(
        dpy: *mut c_void,
        config: *mut c_void,
        share: *mut c_void,
        attrib_list: *const i32,
    ) -> *mut c_void;
    pub(crate) fn eglMakeCurrent(
        dpy: *mut c_void,
        draw: *mut c_void,
        read: *mut c_void,
        ctx: *mut c_void,
    ) -> c_uint;
    /// `glEGLImageTargetTexture2DOES` is an EXTENSION (`GL_OES_EGL_image`), not
    /// a link-time symbol — it must come through here.
    pub(crate) fn eglGetProcAddress(name: *const c_char) -> *mut c_void;

    // GLES2 (12)
    pub(crate) fn glGenTextures(n: i32, textures: *mut c_uint);
    pub(crate) fn glBindTexture(target: c_uint, texture: c_uint);
    pub(crate) fn glTexParameteri(target: c_uint, pname: c_uint, param: c_int);
    pub(crate) fn glDeleteTextures(n: i32, textures: *const c_uint);
    pub(crate) fn glGenFramebuffers(n: i32, framebuffers: *mut c_uint);
    pub(crate) fn glBindFramebuffer(target: c_uint, framebuffer: c_uint);
    pub(crate) fn glFramebufferTexture2D(
        target: c_uint,
        attachment: c_uint,
        textarget: c_uint,
        texture: c_uint,
        level: i32,
    );
    pub(crate) fn glCheckFramebufferStatus(target: c_uint) -> c_uint;
    pub(crate) fn glDeleteFramebuffers(n: i32, framebuffers: *const c_uint);
    pub(crate) fn glReadPixels(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: c_uint,
        type_: c_uint,
        pixels: *mut c_void,
    );
    pub(crate) fn glFinish();
    pub(crate) fn glGetError() -> c_uint;

    // WPEWebKit (9) + JavaScriptCore (2)
    pub(crate) fn webkit_web_view_backend_new(
        backend: *mut c_void,
        notify: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    ) -> *mut c_void;
    pub(crate) fn webkit_web_view_new(backend: *mut c_void) -> *mut c_void;
    pub(crate) fn webkit_web_view_load_uri(view: *mut c_void, uri: *const c_char);
    pub(crate) fn webkit_web_view_reload(view: *mut c_void);
    pub(crate) fn webkit_web_view_get_title(view: *mut c_void) -> *const c_char;
    pub(crate) fn webkit_web_view_get_uri(view: *mut c_void) -> *const c_char;
    pub(crate) fn webkit_web_view_is_loading(view: *mut c_void) -> GBool;
    /// Async: the result arrives in `callback`, which must call
    /// [`webkit_web_view_evaluate_javascript_finish`].
    pub(crate) fn webkit_web_view_evaluate_javascript(
        view: *mut c_void,
        script: *const c_char,
        length: isize,
        world_name: *const c_char,
        source_uri: *const c_char,
        cancellable: *mut c_void,
        callback: Option<extern "C" fn(*mut c_void, *mut c_void, *mut c_void)>,
        user_data: *mut c_void,
    );
    pub(crate) fn webkit_web_view_evaluate_javascript_finish(
        view: *mut c_void,
        result: *mut c_void,
        error: *mut *mut c_void,
    ) -> *mut c_void;

    // JavaScriptCore (2) — `to_json` rather than `to_string` on purpose: it
    // gives a typed, machine-readable answer instead of a stringified one, so a
    // number stays a number across the wire.
    pub(crate) fn jsc_value_to_json(value: *mut c_void, indent: c_uint) -> *mut c_char;
    pub(crate) fn jsc_value_is_undefined(value: *mut c_void) -> GBool;

    // glib / gobject (5)
    pub(crate) fn g_main_context_iteration(context: *mut c_void, may_block: GBool) -> GBool;
    pub(crate) fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        c_handler: *mut c_void,
        data: *mut c_void,
        destroy_data: *mut c_void,
        connect_flags: c_uint,
    ) -> usize;
    pub(crate) fn g_object_unref(object: *mut c_void);
    pub(crate) fn g_free(mem: *mut c_void);
    /// A GError is NOT a GObject; unreffing one corrupts the heap.
    pub(crate) fn g_error_free(error: *mut c_void);
    pub(crate) fn kill(pid: c_int, sig: c_int) -> c_int;
}

/// Resolved at runtime from [`eglGetProcAddress`].
pub(crate) type ImageTargetTexture2DOes = extern "C" fn(target: c_uint, image: *mut c_void);

/// The number of foreign functions this crate declares. Locked by a test so the
/// sizing claim in the spike READMEs cannot drift from the code.
pub(crate) const FOREIGN_FN_COUNT: usize = 48;
