//! Shared headless-WPE plumbing for the Lane-A spikes.
//!
//! Spike A proved the engine runs headless; spike B proved pixels come back.
//! Spike C needs both plus input, so the parts that are not the thing under
//! test live here. `src/bin/wpe-readback.rs` (spike B, merged and proven) is
//! deliberately left untouched — this module is additive.

#![allow(dead_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::ptr;

pub type GBool = c_int;

pub const EGL_PLATFORM_SURFACELESS_MESA: c_uint = 0x31DD;
pub const EGL_NONE: i32 = 0x3038;
pub const EGL_OPENGL_ES_API: c_uint = 0x30A0;
pub const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
pub const EGL_RENDERABLE_TYPE: i32 = 0x3040;
pub const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
pub const EGL_RED_SIZE: i32 = 0x3024;
pub const EGL_GREEN_SIZE: i32 = 0x3023;
pub const EGL_BLUE_SIZE: i32 = 0x3022;
pub const EGL_ALPHA_SIZE: i32 = 0x3021;
pub const EGL_SURFACE_TYPE: i32 = 0x3033;
pub const EGL_PBUFFER_BIT: i32 = 0x0001;

pub const GL_TEXTURE_2D: c_uint = 0x0DE1;
pub const GL_TEXTURE_MIN_FILTER: c_uint = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: c_uint = 0x2800;
pub const GL_NEAREST: c_int = 0x2600;
pub const GL_FRAMEBUFFER: c_uint = 0x8D40;
pub const GL_COLOR_ATTACHMENT0: c_uint = 0x8CE0;
pub const GL_FRAMEBUFFER_COMPLETE: c_uint = 0x8CD5;
pub const GL_RGBA: c_uint = 0x1908;
pub const GL_UNSIGNED_BYTE: c_uint = 0x1401;

/// `wpe_view_activity_state`. NOT the input gate — spike C's negative control
/// showed clicks and keystrokes land without it on WPE 2.52.5 + fdo. It is what
/// an embedder owes the engine for visibility/occlusion (and therefore
/// throttling) semantics.
pub const WPE_ACTIVITY_VISIBLE: u32 = 1 << 0;
pub const WPE_ACTIVITY_FOCUSED: u32 = 1 << 1;
pub const WPE_ACTIVITY_IN_WINDOW: u32 = 1 << 2;

pub const WPE_POINTER_EVENT_MOTION: c_uint = 1;
pub const WPE_POINTER_EVENT_BUTTON: c_uint = 2;

#[repr(C)]
pub struct WpeViewBackendExportableFdoEglClient {
    pub export_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    pub export_fdo_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    pub export_shm_buffer: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    pub reserved0: Option<extern "C" fn()>,
    pub reserved1: Option<extern "C" fn()>,
}

#[repr(C)]
pub struct WpeInputKeyboardEvent {
    pub time: u32,
    pub key_code: u32,
    pub hardware_key_code: u32,
    pub pressed: bool,
    pub modifiers: u32,
}

#[repr(C)]
pub struct WpeInputPointerEvent {
    pub event_type: c_uint,
    pub time: u32,
    pub x: c_int,
    pub y: c_int,
    pub button: u32,
    pub state: u32,
    pub modifiers: u32,
}

unsafe extern "C" {
    pub fn wpe_loader_init(name: *const c_char) -> GBool;
    pub fn wpe_fdo_initialize_for_egl_display(display: *mut c_void) -> GBool;
    pub fn wpe_view_backend_exportable_fdo_egl_create(
        client: *const WpeViewBackendExportableFdoEglClient,
        user_data: *mut c_void,
        width: u32,
        height: u32,
    ) -> *mut c_void;
    pub fn wpe_view_backend_exportable_fdo_get_view_backend(exportable: *mut c_void)
    -> *mut c_void;
    pub fn wpe_view_backend_exportable_fdo_dispatch_frame_complete(exportable: *mut c_void);
    pub fn wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
        exportable: *mut c_void,
        image: *mut c_void,
    );
    pub fn wpe_fdo_egl_exported_image_get_egl_image(image: *mut c_void) -> *mut c_void;
    pub fn wpe_fdo_egl_exported_image_get_width(image: *mut c_void) -> u32;
    pub fn wpe_fdo_egl_exported_image_get_height(image: *mut c_void) -> u32;

    // Input + activity state (NEW in spike C).
    pub fn wpe_view_backend_add_activity_state(backend: *mut c_void, state: u32);
    pub fn wpe_view_backend_dispatch_pointer_event(
        backend: *mut c_void,
        event: *const WpeInputPointerEvent,
    );
    pub fn wpe_view_backend_dispatch_keyboard_event(
        backend: *mut c_void,
        event: *const WpeInputKeyboardEvent,
    );

    pub fn eglGetPlatformDisplay(
        platform: c_uint,
        native_display: *mut c_void,
        attrib_list: *const isize,
    ) -> *mut c_void;
    pub fn eglInitialize(dpy: *mut c_void, major: *mut c_int, minor: *mut c_int) -> c_uint;
    pub fn eglBindAPI(api: c_uint) -> c_uint;
    pub fn eglChooseConfig(
        dpy: *mut c_void,
        attrib_list: *const i32,
        configs: *mut *mut c_void,
        config_size: i32,
        num_config: *mut i32,
    ) -> c_uint;
    pub fn eglCreateContext(
        dpy: *mut c_void,
        config: *mut c_void,
        share: *mut c_void,
        attrib_list: *const i32,
    ) -> *mut c_void;
    pub fn eglMakeCurrent(
        dpy: *mut c_void,
        draw: *mut c_void,
        read: *mut c_void,
        ctx: *mut c_void,
    ) -> c_uint;
    pub fn eglGetProcAddress(name: *const c_char) -> *mut c_void;

    pub fn glGenTextures(n: i32, textures: *mut c_uint);
    pub fn glBindTexture(target: c_uint, texture: c_uint);
    pub fn glTexParameteri(target: c_uint, pname: c_uint, param: c_int);
    pub fn glDeleteTextures(n: i32, textures: *const c_uint);
    pub fn glGenFramebuffers(n: i32, framebuffers: *mut c_uint);
    pub fn glBindFramebuffer(target: c_uint, framebuffer: c_uint);
    pub fn glFramebufferTexture2D(
        target: c_uint,
        attachment: c_uint,
        textarget: c_uint,
        texture: c_uint,
        level: i32,
    );
    pub fn glCheckFramebufferStatus(target: c_uint) -> c_uint;
    pub fn glDeleteFramebuffers(n: i32, framebuffers: *const c_uint);
    pub fn glReadPixels(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: c_uint,
        type_: c_uint,
        pixels: *mut c_void,
    );
    pub fn glFinish();

    pub fn webkit_web_view_backend_new(
        backend: *mut c_void,
        notify: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    ) -> *mut c_void;
    pub fn webkit_web_view_new(backend: *mut c_void) -> *mut c_void;
    pub fn webkit_web_view_load_uri(view: *mut c_void, uri: *const c_char);
    pub fn webkit_web_view_reload(view: *mut c_void);
    pub fn webkit_web_view_get_title(view: *mut c_void) -> *const c_char;

    pub fn g_main_context_iteration(context: *mut c_void, may_block: GBool) -> GBool;
    pub fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        c_handler: *mut c_void,
        data: *mut c_void,
        destroy_data: *mut c_void,
        connect_flags: c_uint,
    ) -> usize;
}

pub type ImageTargetTexture2DOes = extern "C" fn(target: c_uint, image: *mut c_void);

pub unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// One-time headless bring-up: libwpe loader, surfaceless EGL, a GLES2 context
/// current on `EGL_NO_SURFACE`, the image-import extension, and the fdo backend.
///
/// Every step here is a spike A/B finding: `wpe_loader_init` is mandatory
/// (Debian ships no default backend), `eglChooseConfig` must ask for
/// `EGL_PBUFFER_BIT` (a surfaceless display has zero window configs), and
/// `glEGLImageTargetTexture2DOES` is an extension resolved at runtime.
pub fn bring_up_headless() -> Result<ImageTargetTexture2DOes, String> {
    unsafe {
        let backend = CString::new("libWPEBackend-fdo-1.0.so").unwrap();
        if wpe_loader_init(backend.as_ptr()) == 0 {
            return Err("wpe_loader_init returned FALSE".into());
        }
        let dpy = eglGetPlatformDisplay(
            EGL_PLATFORM_SURFACELESS_MESA,
            ptr::null_mut(),
            ptr::null(),
        );
        let (mut major, mut minor) = (0, 0);
        if dpy.is_null() || eglInitialize(dpy, &mut major, &mut minor) == 0 {
            return Err("surfaceless EGL display unavailable".into());
        }
        if eglBindAPI(EGL_OPENGL_ES_API) == 0 {
            return Err("eglBindAPI(OPENGL_ES) failed".into());
        }
        let attrs: [i32; 13] = [
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_NONE,
        ];
        let mut config: *mut c_void = ptr::null_mut();
        let mut n: i32 = 0;
        if eglChooseConfig(dpy, attrs.as_ptr(), &mut config, 1, &mut n) == 0 || n == 0 {
            config = ptr::null_mut();
        }
        let ctx_attrs: [i32; 3] = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let gl = eglCreateContext(dpy, config, ptr::null_mut(), ctx_attrs.as_ptr());
        if gl.is_null() {
            return Err("eglCreateContext returned NULL".into());
        }
        if eglMakeCurrent(dpy, ptr::null_mut(), ptr::null_mut(), gl) == 0 {
            return Err("eglMakeCurrent with EGL_NO_SURFACE failed".into());
        }
        let name = CString::new("glEGLImageTargetTexture2DOES").unwrap();
        let proc_addr = eglGetProcAddress(name.as_ptr());
        if proc_addr.is_null() {
            return Err("glEGLImageTargetTexture2DOES unavailable".into());
        }
        let image_target =
            std::mem::transmute::<*mut c_void, ImageTargetTexture2DOes>(proc_addr);
        if wpe_fdo_initialize_for_egl_display(dpy) == 0 {
            return Err("wpe_fdo_initialize_for_egl_display returned FALSE".into());
        }
        Ok(image_target)
    }
}

/// Import an exported EGLImage and read its centre pixel back to the CPU.
///
/// Spike B reads the whole frame; spike C only needs the colour, so this reads
/// a 1×1 rect — which is both faster and, more importantly, keeps the
/// assertion about the PAGE rather than about an image file.
pub fn read_centre_rgba(
    image_target: ImageTargetTexture2DOes,
    egl_image: *mut c_void,
    width: u32,
    height: u32,
) -> Result<[u8; 4], String> {
    unsafe {
        let mut texture: c_uint = 0;
        glGenTextures(1, &mut texture);
        glBindTexture(GL_TEXTURE_2D, texture);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        image_target(GL_TEXTURE_2D, egl_image);

        let mut fbo: c_uint = 0;
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_TEXTURE_2D,
            texture,
            0,
        );
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if status != GL_FRAMEBUFFER_COMPLETE {
            glDeleteFramebuffers(1, &fbo);
            glDeleteTextures(1, &texture);
            return Err(format!("framebuffer incomplete: 0x{status:04X}"));
        }
        let mut px = [0u8; 4];
        glReadPixels(
            (width / 2) as i32,
            (height / 2) as i32,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_mut_ptr().cast(),
        );
        glFinish();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &texture);
        Ok(px)
    }
}

/// Pump the GLib main context without blocking, once.
pub fn pump() {
    unsafe {
        while g_main_context_iteration(ptr::null_mut(), 0) != 0 {}
    }
}

/// Our own `WPEWebProcess` children, newest last. The fdo backend hands us
/// VIEWS; WebKit spawns the processes, and nothing in either API tells us which
/// process serves which view — so the mapping has to be discovered.
/// Every descendant of this process, as `(pid, comm, ppid)`.
///
/// ⚠ **A web process is NOT a direct child.** WebKit launches each one inside
/// bubblewrap, so the process tree is
/// `spike -> bwrap -> WPEWebProcess`, and `comm` in `/proc/<pid>/stat` is
/// truncated to 15 characters (`WPENetworkProce`). Anything supervising these
/// processes has to walk descendants and match on a PREFIX — a direct-children
/// scan finds only `bwrap` and reports zero web processes.
pub fn descendants() -> Vec<(u32, String, u32)> {
    let mut all: Vec<(u32, String, u32)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some((before, after)) = stat.rsplit_once(')') else {
            continue;
        };
        let comm = before.split_once('(').map(|(_, c)| c.to_string()).unwrap_or_default();
        let mut fields = after.split_whitespace();
        let _state = fields.next();
        let ppid: u32 = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        all.push((pid, comm, ppid));
    }

    let me = std::process::id();
    let mut wanted = vec![me];
    let mut out = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for (pid, comm, ppid) in &all {
            if wanted.contains(ppid) && !wanted.contains(pid) {
                wanted.push(*pid);
                out.push((*pid, comm.clone(), *ppid));
                changed = true;
            }
        }
    }
    out.sort_by_key(|(pid, _, _)| *pid);
    out
}

pub fn child_processes() -> Vec<(u32, String)> {
    descendants()
        .into_iter()
        .map(|(pid, comm, _)| (pid, comm))
        .collect()
}

/// The sandboxed web-content processes, found by walking descendants.
pub fn web_process_pids() -> Vec<u32> {
    descendants()
        .into_iter()
        .filter(|(_, comm, _)| comm.starts_with("WPEWebProcess"))
        .map(|(pid, _, _)| pid)
        .collect()
}
