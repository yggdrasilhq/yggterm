//! O5 — WPE spike B: CPU readback of the exported EGLImage.
//!
//! Spike A proved WPEWebKit runs headless via libwpe + WPEBackend-fdo and
//! EXPORTS frames. It deliberately released each `EGLImage` untouched, so the
//! one primitive `capture-element` and the lore-anchored pixel rung both need
//! was left unproven: getting CPU pixels back out.
//!
//! This binary closes that. It stands up a real GLES2 context on the same
//! surfaceless EGL display, imports the exported image as a texture
//! (`glEGLImageTargetTexture2DOES`), attaches it to an FBO, and `glReadPixels`
//! into host memory — then writes raw RGBA and a PNG, with no display server.
//!
//! **Acceptance is deliberately not "nonzero bytes".** A readback that silently
//! returned an uninitialised or all-white buffer would pass that. Two fixtures
//! with different solid backgrounds must produce DIFFERENT and PREDICTABLE
//! pixels, so the binary is run twice and the two centre pixels are compared
//! against the colours the pages actually declare.
//!
//! Usage:
//!   wpe-readback <url> <out-prefix> [--expect-rgb RRGGBB] [--bench N]

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::process::ExitCode;
use std::ptr;
use std::time::Instant;

#[path = "../png.rs"]
mod png;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const EGL_PLATFORM_SURFACELESS_MESA: c_uint = 0x31DD;
const EGL_NONE: i32 = 0x3038;
const EGL_OPENGL_ES_API: c_uint = 0x30A0;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_PBUFFER_BIT: i32 = 0x0001;

const GL_TEXTURE_2D: c_uint = 0x0DE1;
const GL_TEXTURE_MIN_FILTER: c_uint = 0x2801;
const GL_TEXTURE_MAG_FILTER: c_uint = 0x2800;
const GL_TEXTURE_WRAP_S: c_uint = 0x2802;
const GL_TEXTURE_WRAP_T: c_uint = 0x2803;
const GL_NEAREST: c_int = 0x2600;
const GL_CLAMP_TO_EDGE: c_int = 0x812F;
const GL_FRAMEBUFFER: c_uint = 0x8D40;
const GL_COLOR_ATTACHMENT0: c_uint = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: c_uint = 0x8CD5;
const GL_RGBA: c_uint = 0x1908;
const GL_UNSIGNED_BYTE: c_uint = 0x1401;
const GL_NO_ERROR: c_uint = 0;

const WEBKIT_LOAD_FINISHED: c_int = 3;

type GBool = c_int;

#[repr(C)]
struct WpeViewBackendExportableFdoEglClient {
    export_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    export_fdo_egl_image: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    export_shm_buffer: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    reserved0: Option<extern "C" fn()>,
    reserved1: Option<extern "C" fn()>,
}

unsafe extern "C" {
    // --- carried over from spike A (19 fns; only those still needed here) ---
    fn wpe_loader_init(impl_library_name: *const c_char) -> GBool;
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
    fn eglGetPlatformDisplay(
        platform: c_uint,
        native_display: *mut c_void,
        attrib_list: *const isize,
    ) -> *mut c_void;
    fn eglInitialize(dpy: *mut c_void, major: *mut c_int, minor: *mut c_int) -> c_uint;
    fn webkit_web_view_backend_new(
        backend: *mut c_void,
        notify: Option<extern "C" fn(*mut c_void)>,
        user_data: *mut c_void,
    ) -> *mut c_void;
    fn webkit_web_view_new(backend: *mut c_void) -> *mut c_void;
    fn webkit_web_view_load_uri(view: *mut c_void, uri: *const c_char);
    fn webkit_web_view_get_title(view: *mut c_void) -> *const c_char;
    fn g_main_loop_new(context: *mut c_void, is_running: GBool) -> *mut c_void;
    fn g_main_loop_run(loop_: *mut c_void);
    fn g_main_loop_quit(loop_: *mut c_void);
    fn g_timeout_add(
        interval_ms: c_uint,
        func: extern "C" fn(*mut c_void) -> GBool,
        data: *mut c_void,
    ) -> c_uint;
    fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        c_handler: *mut c_void,
        data: *mut c_void,
        destroy_data: *mut c_void,
        connect_flags: c_uint,
    ) -> usize;

    // --- NEW for spike B: the exported image's accessors (3) ---
    fn wpe_fdo_egl_exported_image_get_egl_image(image: *mut c_void) -> *mut c_void;
    fn wpe_fdo_egl_exported_image_get_width(image: *mut c_void) -> u32;
    fn wpe_fdo_egl_exported_image_get_height(image: *mut c_void) -> u32;

    // --- NEW for spike B: a real GL context on the surfaceless display (5) ---
    fn eglBindAPI(api: c_uint) -> c_uint;
    fn eglChooseConfig(
        dpy: *mut c_void,
        attrib_list: *const i32,
        configs: *mut *mut c_void,
        config_size: i32,
        num_config: *mut i32,
    ) -> c_uint;
    fn eglCreateContext(
        dpy: *mut c_void,
        config: *mut c_void,
        share: *mut c_void,
        attrib_list: *const i32,
    ) -> *mut c_void;
    fn eglMakeCurrent(
        dpy: *mut c_void,
        draw: *mut c_void,
        read: *mut c_void,
        ctx: *mut c_void,
    ) -> c_uint;
    fn eglGetProcAddress(name: *const c_char) -> *mut c_void;

    // --- NEW for spike B: the readback itself (12) ---
    fn glGenTextures(n: i32, textures: *mut c_uint);
    fn glBindTexture(target: c_uint, texture: c_uint);
    fn glTexParameteri(target: c_uint, pname: c_uint, param: c_int);
    fn glDeleteTextures(n: i32, textures: *const c_uint);
    fn glGenFramebuffers(n: i32, framebuffers: *mut c_uint);
    fn glBindFramebuffer(target: c_uint, framebuffer: c_uint);
    fn glFramebufferTexture2D(
        target: c_uint,
        attachment: c_uint,
        textarget: c_uint,
        texture: c_uint,
        level: i32,
    );
    fn glCheckFramebufferStatus(target: c_uint) -> c_uint;
    fn glDeleteFramebuffers(n: i32, framebuffers: *const c_uint);
    fn glReadPixels(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: c_uint,
        type_: c_uint,
        pixels: *mut c_void,
    );
    fn glFinish();
    fn glGetError() -> c_uint;
}

/// Resolved at runtime: `glEGLImageTargetTexture2DOES` is an EXTENSION
/// (`GL_OES_EGL_image`), so it is not a link-time symbol — it must come from
/// `eglGetProcAddress`. Spike A's sizing did not account for this; it is the
/// single most important structural fact spike B found.
type ImageTargetTexture2DOes = extern "C" fn(target: c_uint, image: *mut c_void);

/// Foreign functions declared for spike B ALONE (i.e. beyond spike A's 19).
const SPIKE_B_NEW_FN_COUNT: usize = 21;

// ---------------------------------------------------------------------------

struct Capture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    /// (import + FBO attach + glReadPixels + glFinish) per iteration, warm.
    bench_us: Vec<u128>,
}

struct State {
    exportable: *mut c_void,
    main_loop: *mut c_void,
    image_target: Option<ImageTargetTexture2DOes>,
    bench_iterations: usize,
    capture: Option<Capture>,
    frames: u32,
    blank_frames: u32,
    load_finished: bool,
    error: Option<String>,
}

static mut STATE: *mut State = ptr::null_mut();

fn state() -> &'static mut State {
    unsafe { &mut *STATE }
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

fn gl_error_is_clear(stage: &str) -> Result<(), String> {
    let err = unsafe { glGetError() };
    if err == GL_NO_ERROR {
        Ok(())
    } else {
        Err(format!("{stage}: GL error 0x{err:04X}"))
    }
}

/// The whole point of the spike: EGLImage → GL texture → FBO → host memory.
fn read_back(image: *mut c_void, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let st = state();
    let image_target = st
        .image_target
        .ok_or_else(|| "glEGLImageTargetTexture2DOES was not resolved".to_string())?;

    unsafe {
        let mut texture: c_uint = 0;
        glGenTextures(1, &mut texture);
        glBindTexture(GL_TEXTURE_2D, texture);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        image_target(GL_TEXTURE_2D, image);
        gl_error_is_clear("glEGLImageTargetTexture2DOES")?;

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

        let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
        glReadPixels(
            0,
            0,
            width as i32,
            height as i32,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            rgba.as_mut_ptr().cast(),
        );
        glFinish();
        gl_error_is_clear("glReadPixels")?;

        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &texture);

        // GL's origin is BOTTOM-left; PNG's is top-left. Without this the
        // acceptance image is upside down — invisible on a solid-colour
        // fixture, which is exactly why it is done here and not "later".
        let stride = width as usize * 4;
        let mut flipped = vec![0u8; rgba.len()];
        for row in 0..height as usize {
            let src = row * stride;
            let dst = (height as usize - 1 - row) * stride;
            flipped[dst..dst + stride].copy_from_slice(&rgba[src..src + stride]);
        }
        Ok(flipped)
    }
}

extern "C" fn on_export_fdo_egl_image(_data: *mut c_void, image: *mut c_void) {
    let st = state();
    st.frames += 1;

    if st.capture.is_none() && st.error.is_none() {
        let egl_image = unsafe { wpe_fdo_egl_exported_image_get_egl_image(image) };
        let width = unsafe { wpe_fdo_egl_exported_image_get_width(image) };
        let height = unsafe { wpe_fdo_egl_exported_image_get_height(image) };
        if egl_image.is_null() || width == 0 || height == 0 {
            st.error = Some(format!(
                "exported image is unusable: ptr={egl_image:?} {width}x{height}"
            ));
        } else {
            match read_back(egl_image, width, height) {
                Ok(rgba) => {
                    // ⚠ THE FIRST EXPORTED FRAME IS BLANK. The compositor
                    // exports an initial frame before the page has painted;
                    // capturing it produced an all-zero buffer that satisfied
                    // "readback succeeded" and "bytes were written" while
                    // containing no page at all. Accept the first frame that
                    // actually carries pixels instead — this is exactly what
                    // the two-fixture colour acceptance was written to catch,
                    // and it caught it.
                    let blank = rgba.iter().all(|byte| *byte == 0);
                    println!(
                        "[readback] frame {} {}x{} blank={blank}",
                        st.frames, width, height
                    );
                    if blank {
                        st.blank_frames += 1;
                    } else {
                        // Warm bench on the accepted image: the SAME image read
                        // back N times, so this measures the readback
                        // primitive, not frame production.
                        let mut bench_us = Vec::new();
                        for _ in 0..st.bench_iterations {
                            let started = Instant::now();
                            if let Err(err) = read_back(egl_image, width, height) {
                                st.error = Some(err);
                                break;
                            }
                            bench_us.push(started.elapsed().as_micros());
                        }
                        st.capture = Some(Capture {
                            width,
                            height,
                            rgba,
                            bench_us,
                        });
                        unsafe { g_main_loop_quit(st.main_loop) };
                    }
                }
                Err(err) => {
                    st.error = Some(err);
                    unsafe { g_main_loop_quit(st.main_loop) };
                }
            }
        }
    }

    unsafe {
        wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(st.exportable, image);
        wpe_view_backend_exportable_fdo_dispatch_frame_complete(st.exportable);
    }
}

extern "C" fn on_export_egl_image(_data: *mut c_void, _image: *mut c_void) {
    let st = state();
    st.frames += 1;
    unsafe { wpe_view_backend_exportable_fdo_dispatch_frame_complete(st.exportable) };
}

extern "C" fn on_load_changed(_view: *mut c_void, event: c_int, _data: *mut c_void) {
    if event == WEBKIT_LOAD_FINISHED {
        state().load_finished = true;
    }
}

extern "C" fn on_timeout(_data: *mut c_void) -> GBool {
    let st = state();
    if st.error.is_none() && st.capture.is_none() {
        st.error = Some(format!(
            "timed out with no PAINTED frame ({} frame(s) exported, {} of them blank)",
            st.frames, st.blank_frames,
        ));
    }
    unsafe { g_main_loop_quit(st.main_loop) };
    0
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].as_str())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with("--")
                && !(*i > 0 && args[i - 1].starts_with("--"))
        })
        .map(|(_, a)| a)
        .collect();
    if positional.len() < 2 {
        eprintln!(
            "usage: wpe-readback <url> <out-prefix> [--expect-rgb RRGGBB] [--bench N] [--size WxH]"
        );
        return ExitCode::from(2);
    }
    let url = positional[0].clone();
    let out_prefix = positional[1].clone();
    let bench_iterations: usize = flag(&args, "--bench")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let expect_rgb = flag(&args, "--expect-rgb").map(str::to_string);
    // The readback cost scales with pixel count, and "can the pixel rung be
    // interactive" is a question about a REAL viewport, not a 640x480 stub.
    let (view_w, view_h) = flag(&args, "--size")
        .and_then(|raw| {
            let (w, h) = raw.split_once('x')?;
            Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
        })
        .unwrap_or((640, 480));

    println!("[readback] spike_b_new_fn_count={SPIKE_B_NEW_FN_COUNT} (beyond spike A's 19)");
    println!(
        "[readback] DISPLAY={:?} WAYLAND_DISPLAY={:?}",
        std::env::var("DISPLAY").ok(),
        std::env::var("WAYLAND_DISPLAY").ok(),
    );

    let boxed = Box::new(State {
        exportable: ptr::null_mut(),
        main_loop: ptr::null_mut(),
        image_target: None,
        bench_iterations,
        capture: None,
        frames: 0,
        blank_frames: 0,
        load_finished: false,
        error: None,
    });
    unsafe { STATE = Box::into_raw(boxed) };
    let st = state();

    unsafe {
        // Spike A finding: Debian ships no libWPEBackend-default.so, so this
        // is mandatory, not optional.
        let backend = CString::new("libWPEBackend-fdo-1.0.so").unwrap();
        if wpe_loader_init(backend.as_ptr()) == 0 {
            eprintln!("[readback] FAIL: wpe_loader_init returned FALSE");
            return ExitCode::from(1);
        }

        let dpy = eglGetPlatformDisplay(
            EGL_PLATFORM_SURFACELESS_MESA,
            ptr::null_mut(),
            ptr::null(),
        );
        let (mut major, mut minor) = (0, 0);
        if dpy.is_null() || eglInitialize(dpy, &mut major, &mut minor) == 0 {
            eprintln!("[readback] FAIL: surfaceless EGL display unavailable");
            return ExitCode::from(1);
        }
        println!("[readback] EGL {major}.{minor} surfaceless: ok");

        // NEW vs spike A: a real GLES2 context. Spike A only needed an
        // EGLDisplay because it never touched a pixel.
        if eglBindAPI(EGL_OPENGL_ES_API) == 0 {
            eprintln!("[readback] FAIL: eglBindAPI(OPENGL_ES) failed");
            return ExitCode::from(1);
        }
        // eglChooseConfig defaults EGL_SURFACE_TYPE to EGL_WINDOW_BIT, and a
        // SURFACELESS display has no window configs at all — asking for the
        // default returns zero matches. Ask for a pbuffer config instead; the
        // context is still made current with EGL_NO_SURFACE.
        let config_attrs: [i32; 13] = [
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
        let mut num_config: i32 = 0;
        let chose =
            eglChooseConfig(dpy, config_attrs.as_ptr(), &mut config, 1, &mut num_config) != 0
                && num_config > 0;
        if !chose {
            // Fallback: EGL_KHR_no_config_context lets a context exist with no
            // EGLConfig at all, which is the honest shape here — we never bind
            // a surface, so a config describes nothing we use.
            println!(
                "[readback] no pbuffer ES2 config; falling back to \
                 EGL_KHR_no_config_context"
            );
            config = ptr::null_mut();
        } else {
            println!("[readback] ES2 pbuffer EGLConfig: ok");
        }
        let ctx_attrs: [i32; 3] = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let gl_ctx = eglCreateContext(dpy, config, ptr::null_mut(), ctx_attrs.as_ptr());
        if gl_ctx.is_null() {
            eprintln!("[readback] FAIL: eglCreateContext returned NULL");
            return ExitCode::from(1);
        }
        // EGL_NO_SURFACE on both: needs EGL_KHR_surfaceless_context, which is
        // exactly what makes this work with no display server.
        if eglMakeCurrent(dpy, ptr::null_mut(), ptr::null_mut(), gl_ctx) == 0 {
            eprintln!(
                "[readback] FAIL: eglMakeCurrent with EGL_NO_SURFACE failed \
                 (EGL_KHR_surfaceless_context missing?)"
            );
            return ExitCode::from(1);
        }
        println!("[readback] GLES2 context current on EGL_NO_SURFACE: ok");

        // The extension entry point. NOT a link-time symbol.
        let name = CString::new("glEGLImageTargetTexture2DOES").unwrap();
        let proc_addr = eglGetProcAddress(name.as_ptr());
        if proc_addr.is_null() {
            eprintln!("[readback] FAIL: glEGLImageTargetTexture2DOES unavailable (GL_OES_EGL_image)");
            return ExitCode::from(1);
        }
        st.image_target = Some(std::mem::transmute::<*mut c_void, ImageTargetTexture2DOes>(
            proc_addr,
        ));
        println!("[readback] glEGLImageTargetTexture2DOES resolved: ok");

        if wpe_fdo_initialize_for_egl_display(dpy) == 0 {
            eprintln!("[readback] FAIL: wpe_fdo_initialize_for_egl_display returned FALSE");
            return ExitCode::from(1);
        }

        let client = WpeViewBackendExportableFdoEglClient {
            export_egl_image: Some(on_export_egl_image),
            export_fdo_egl_image: Some(on_export_fdo_egl_image),
            export_shm_buffer: None,
            reserved0: None,
            reserved1: None,
        };
        let exportable =
            wpe_view_backend_exportable_fdo_egl_create(&client, ptr::null_mut(), view_w, view_h);
        if exportable.is_null() {
            eprintln!("[readback] FAIL: exportable backend is NULL");
            return ExitCode::from(1);
        }
        st.exportable = exportable;
        let view_backend = wpe_view_backend_exportable_fdo_get_view_backend(exportable);
        let wvb = webkit_web_view_backend_new(view_backend, None, ptr::null_mut());
        let view = webkit_web_view_new(wvb);

        let sig = CString::new("load-changed").unwrap();
        g_signal_connect_data(
            view,
            sig.as_ptr(),
            on_load_changed as *mut c_void,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        );

        st.main_loop = g_main_loop_new(ptr::null_mut(), 0);
        g_timeout_add(20_000, on_timeout, ptr::null_mut());
        let c_url = CString::new(url.clone()).unwrap();
        webkit_web_view_load_uri(view, c_url.as_ptr());
        g_main_loop_run(st.main_loop);

        let title = cstr(webkit_web_view_get_title(view));
        println!(
            "[readback] title={title:?} frames={} blank_frames={} load_finished={}",
            st.frames, st.blank_frames, st.load_finished,
        );
    }

    if let Some(err) = &st.error {
        eprintln!("[readback] FAIL: {err}");
        return ExitCode::from(1);
    }
    let Some(capture) = &st.capture else {
        eprintln!("[readback] FAIL: no frame was captured");
        return ExitCode::from(1);
    };

    let raw_path = format!("{out_prefix}.rgba");
    let png_path = format!("{out_prefix}.png");
    if let Err(err) = std::fs::write(&raw_path, &capture.rgba) {
        eprintln!("[readback] FAIL: writing {raw_path}: {err}");
        return ExitCode::from(1);
    }
    let png_bytes = png::encode_rgba(&capture.rgba, capture.width, capture.height);
    if let Err(err) = std::fs::write(&png_path, &png_bytes) {
        eprintln!("[readback] FAIL: writing {png_path}: {err}");
        return ExitCode::from(1);
    }

    // A cheap, stable checksum over the pixels (FNV-1a) — the artifact the
    // brief asks to record, and enough to tell two fixtures apart in a log.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in &capture.rgba {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }

    let centre = {
        let x = capture.width as usize / 2;
        let y = capture.height as usize / 2;
        let idx = (y * capture.width as usize + x) * 4;
        [
            capture.rgba[idx],
            capture.rgba[idx + 1],
            capture.rgba[idx + 2],
            capture.rgba[idx + 3],
        ]
    };

    println!("[readback] size={}x{}", capture.width, capture.height);
    println!("[readback] rgba_bytes={}", capture.rgba.len());
    println!("[readback] fnv1a64={hash:016x}");
    println!(
        "[readback] centre_rgba={},{},{},{}",
        centre[0], centre[1], centre[2], centre[3]
    );
    println!("[readback] wrote {raw_path} and {png_path} ({} bytes)", png_bytes.len());

    if !capture.bench_us.is_empty() {
        let mut sorted = capture.bench_us.clone();
        sorted.sort_unstable();
        let sum: u128 = sorted.iter().sum();
        println!(
            "[readback] readback_us n={} min={} p50={} max={} mean={}",
            sorted.len(),
            sorted[0],
            sorted[sorted.len() / 2],
            sorted[sorted.len() - 1],
            sum / sorted.len() as u128,
        );
    }

    if let Some(expect) = expect_rgb {
        let want = u32::from_str_radix(&expect, 16).unwrap_or(0);
        let (wr, wg, wb) = (
            ((want >> 16) & 0xFF) as u8,
            ((want >> 8) & 0xFF) as u8,
            (want & 0xFF) as u8,
        );
        // Tolerance covers colour-management / rounding, not a wrong colour.
        let close = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= 6;
        let rgba_match = close(centre[0], wr) && close(centre[1], wg) && close(centre[2], wb);
        let bgra_match = close(centre[2], wr) && close(centre[1], wg) && close(centre[0], wb);
        println!(
            "[readback] expected_rgb={wr},{wg},{wb} rgba_match={rgba_match} bgra_match={bgra_match}"
        );
        if !rgba_match && !bgra_match {
            eprintln!(
                "[readback] FAIL: centre pixel is neither the expected colour in RGBA nor BGRA \
                 order — the readback is not returning the page's pixels"
            );
            return ExitCode::from(1);
        }
        println!(
            "[readback] channel_order={}",
            if rgba_match { "RGBA" } else { "BGRA" }
        );
    }

    println!("[readback] ACCEPTANCE=PASS");
    ExitCode::SUCCESS
}
