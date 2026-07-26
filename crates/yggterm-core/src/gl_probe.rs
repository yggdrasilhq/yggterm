//! Does this host's GPU actually rasterize? — a one-shot EGL capability probe.
//!
//! # Why this exists
//!
//! `configure_linux_webkit_compositing()` forced `LIBGL_ALWAYS_SOFTWARE=1` +
//! `GALLIUM_DRIVER=llvmpipe` + `WEBKIT_DISABLE_DMABUF_RENDERER=1` on the stated
//! premise that the GUI host's iGPU "exposes only llvmpipe". Measured on jojo
//! 2026-07-25, that premise is FALSE and has been for some time: the **GBM** EGL
//! platform reports `llvmpipe`, but Wayland, Surfaceless and Device all report
//! `AMD Radeon 780M (radeonsi, phoenix, ACO)`. GBM fails only because it opens
//! `card0` and takes `EACCES` on `DRM_IOCTL_AMDGPU_INFO` while the compositor holds
//! DRM master; every ioctl on `/dev/dri/renderD128` succeeds. One EACCES on the wrong
//! node was generalized into "this host has no GPU", and hardware GL was disabled
//! product-wide — measured at **22x** the CPU for a WebGL glyph grid, which is what
//! xterm.js 6 draws the terminal with.
//!
//! So the fix is not a different hard-coded answer. It is to stop hard-coding the
//! premise and ASK the host.
//!
//! # The traps this module exists to avoid
//!
//! 1. **Never probe the GBM platform.** It is the one platform that reported
//!    `llvmpipe` on this host and it is the sole origin of the false premise. This
//!    module uses `EGL_PLATFORM_SURFACELESS_MESA` and nothing else.
//! 2. **Never let the probe inherit the answer.** A GUI relaunched by a running GUI
//!    (hot restart, supervisor) inherits its predecessor's process env, which on a
//!    software-pinned host already carries `LIBGL_ALWAYS_SOFTWARE=1`. A probe that
//!    inherited that would dlopen Mesa with software already forced, report
//!    `llvmpipe`, and pin the host to the software path FOREVER — a self-fulfilling
//!    premise. [`GL_PROBE_STRIPPED_ENV`] is that guard, and it is load-bearing.
//! 3. **Never cache the verdict to disk.** A stale belief about this host's GPU is
//!    precisely the failure mode that created the bug. The probe is cheap, runs once
//!    per process, and is re-derived on every launch.
//! 4. **Never make an external tool (`eglinfo`) a runtime dependency.** It is not in
//!    `debian/control`; on a host without `mesa-utils` the fallback would be the
//!    software path, i.e. the bug, silently.
//!
//! # What it deliberately does NOT claim
//!
//! It answers "does an EGL display on this host rasterize in hardware", not "will
//! WebKit's compositor be stable". Stability is a separate decision that the caller
//! owns, with `YGGTERM_FORCE_SOFTWARE_GL` as the escape hatch. An inconclusive probe
//! reports [`GlClass::Unknown`], never a guess.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// What the host's EGL stack turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlClass {
    /// A real GPU driver answered.
    Hardware,
    /// A software rasterizer answered (llvmpipe and friends).
    Software,
    /// Nothing conclusive: no render node, no libEGL, a timeout, a crash. The caller
    /// must treat this as "assume the conservative path", never as "probably fine".
    Unknown,
}

impl GlClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            GlClass::Hardware => "hardware",
            GlClass::Software => "software",
            GlClass::Unknown => "unknown",
        }
    }

    /// Parse the wire token. The inverse of [`GlClass::as_str`], and the only one —
    /// the child process and the parent must never grow two spellings.
    pub fn from_str_token(token: &str) -> Option<GlClass> {
        match token {
            "hardware" => Some(GlClass::Hardware),
            "software" => Some(GlClass::Software),
            "unknown" => Some(GlClass::Unknown),
            _ => None,
        }
    }
}

/// What the probe found, plus enough context for a trace to be worth reading.
///
/// `reason` is a `String`, not the `&'static str` the pure policy functions use,
/// because this value crosses a process boundary: the parent reconstructs it from the
/// child's stdout, so it cannot be a pointer into this binary's rodata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlProbeReport {
    pub class: GlClass,
    /// Mesa's driver name (`radeonsi`, `iris`, `llvmpipe`) when `EGL_MESA_query_driver`
    /// is available — the cheap answer that needs no GL context at all.
    pub driver: Option<String>,
    /// `GL_RENDERER`, only read when the driver-name extension is absent.
    pub renderer: Option<String>,
    /// Why the class is what it is, for the startup trace.
    pub reason: String,
    /// How long the CALLER waited for this answer. Always the caller's own clock: the
    /// child never reports its self-timing, so there is one number and it is the one
    /// that startup actually pays.
    pub elapsed_ms: u64,
}

impl GlProbeReport {
    fn inconclusive(reason: &str) -> GlProbeReport {
        GlProbeReport {
            class: GlClass::Unknown,
            driver: None,
            renderer: None,
            reason: reason.to_string(),
            elapsed_ms: 0,
        }
    }
}

/// Substrings that identify a SOFTWARE rasterizer, in either the driver name or the
/// renderer string. One list, one function, no second table anywhere — a marker added
/// in a second place is how a classifier silently stops classifying.
///
/// `zink over lavapipe` is covered without a special case: zink's renderer string
/// names the underlying Vulkan device, which for lavapipe contains `llvmpipe`.
const SOFTWARE_RENDERER_MARKERS: &[&str] = &[
    "llvmpipe",
    "softpipe",
    "swrast",
    "lavapipe",
    "swiftshader",
    "software rasterizer",
    "basic render driver",
];

/// THE classifier. Pure, so the one judgement this module makes is testable without a
/// GPU, a display, or a child process.
///
/// Any software marker in EITHER string wins: a hardware-sounding driver name with a
/// software renderer string is software, and that combination is real (zink).
pub fn classify_gl_strings(driver: Option<&str>, renderer: Option<&str>) -> GlClass {
    let mut saw_a_string = false;
    for text in [driver, renderer].into_iter().flatten() {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        saw_a_string = true;
        let lowered = text.to_ascii_lowercase();
        if SOFTWARE_RENDERER_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            return GlClass::Software;
        }
    }
    if saw_a_string {
        GlClass::Hardware
    } else {
        GlClass::Unknown
    }
}

/// Tier-0 gate: is there a DRM **render node** this process can open?
///
/// Deliberately does NOT look at `card0`. The EACCES on `card0`'s
/// `DRM_IOCTL_AMDGPU_INFO` while the compositor holds DRM master is the exact false
/// negative that created this bug; `renderD*` is the node that is meant to be opened
/// by non-master clients and it is the one WebKit's GPU path uses.
///
/// A host with no openable render node is either headless or genuinely GPU-less, and
/// in both cases there is nothing to dlopen Mesa for.
pub fn render_node_present() -> bool {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("renderD") {
            continue;
        }
        if std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(entry.path())
            .is_ok()
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// The child-process half. The probe dlopens a graphics driver, so it must not be
// able to take the GUI down with it; it runs in a short-lived re-exec of ourselves.
// ---------------------------------------------------------------------------

/// Where the GL decision publishes itself, and the reason it has to.
/// `configure_linux_webkit_compositing` runs before tracing is initialized and before
/// the store exists, so an exported reason is the only way the choice is observable at
/// all. Declared here rather than in the GUI binary because readers in other crates
/// name it too — two crates, one string.
pub const ENV_YGGTERM_WEBKIT_GL_POLICY: &str = "YGGTERM_WEBKIT_GL_POLICY";
/// Mesa's software-rasterizer force. Owned by the GL decision, not by whatever the
/// process happened to inherit.
pub const ENV_LIBGL_ALWAYS_SOFTWARE: &str = "LIBGL_ALWAYS_SOFTWARE";
/// Mesa's driver override; the twin of [`ENV_LIBGL_ALWAYS_SOFTWARE`].
pub const ENV_GALLIUM_DRIVER: &str = "GALLIUM_DRIVER";
/// WebKitGTK's SHM presentation force (DMABuf renderer off).
pub const ENV_WEBKIT_DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
/// The resolved under-glass arming flag every downstream reader keys on.
pub const ENV_YGGTERM_WEB_SURFACE_UNDER_GLASS: &str = "YGGTERM_WEB_SURFACE_UNDER_GLASS";
/// GLVND's vendor-library filter. Owned by the GL decision on a host where the
/// NVIDIA ICD is installed with no NVIDIA device behind it (see
/// [`stray_nvidia_egl_vendor`]): GLVND sorts `10_nvidia.json` ahead of
/// `50_mesa.json`, so `libEGL_nvidia` gets mapped into the GUI and every web
/// process on an AMD-only machine — it appeared in three WebKit crash stacks and
/// immediately preceded an `eglMakeCurrent failed` → SIGSEGV on the live host
/// (2026-07-26). Pinning the filter to the Mesa ICD keeps the dead vendor out.
pub const ENV_EGL_VENDOR_LIBRARY_FILENAMES: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";

/// The six variables that ARE the GL path: the decision plus the five settings it
/// owns. One list, so a reader publishing "which GL path is this window on" cannot
/// answer with five of six and look complete.
pub const WEBKIT_GL_ENVIRONMENT_KEYS: &[&str] = &[
    ENV_YGGTERM_WEBKIT_GL_POLICY,
    ENV_LIBGL_ALWAYS_SOFTWARE,
    ENV_GALLIUM_DRIVER,
    ENV_WEBKIT_DISABLE_DMABUF_RENDERER,
    ENV_YGGTERM_WEB_SURFACE_UNDER_GLASS,
    ENV_EGL_VENDOR_LIBRARY_FILENAMES,
];

/// The Mesa GLVND ICD the vendor filter pins to. An absolute path baked as a
/// constant on purpose: the filter is only ever SET when this exact file was
/// observed to exist (see [`stray_nvidia_egl_vendor`]), so a distro that keeps
/// it elsewhere simply never activates the guard.
pub const MESA_EGL_VENDOR_JSON: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
const NVIDIA_EGL_VENDOR_JSON: &str = "/usr/share/glvnd/egl_vendor.d/10_nvidia.json";

/// True when the NVIDIA GLVND ICD is installed but no NVIDIA device exists —
/// the misconfiguration where GLVND tries (and maps) `libEGL_nvidia` first on a
/// machine that cannot use it. Requires the Mesa ICD to be present too: pinning
/// the filter to a file that does not exist would take EGL down entirely, which
/// is strictly worse than the stray vendor.
pub fn stray_nvidia_egl_vendor() -> bool {
    stray_nvidia_egl_vendor_at(std::path::Path::new("/"))
}

/// [`stray_nvidia_egl_vendor`] against an arbitrary root, so a test can build
/// the three filesystem states without touching the host's real `/usr` or
/// `/dev`.
pub fn stray_nvidia_egl_vendor_at(root: &std::path::Path) -> bool {
    let strip = |abs: &str| abs.trim_start_matches('/').to_string();
    let nvidia_icd = root.join(strip(NVIDIA_EGL_VENDOR_JSON));
    let mesa_icd = root.join(strip(MESA_EGL_VENDOR_JSON));
    if !nvidia_icd.is_file() || !mesa_icd.is_file() {
        return false;
    }
    let dev = root.join("dev");
    let Ok(entries) = std::fs::read_dir(&dev) else {
        // An unreadable /dev cannot prove the device absent; stay out.
        return false;
    };
    !entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("nvidia"))
    })
}

/// What THIS process decided its GL path is — read from this process's own
/// environment, which is the only place the answer exists.
///
/// ⚠⚠ **`/proc/<pid>/environ` cannot answer this question and never could.**
/// `setenv`/`unsetenv` (and therefore `std::env::set_var`/`remove_var`) reallocate the
/// environ array on the heap; the kernel keeps exposing the exec-time copy in the
/// process's stack region. Measured on the dev host 2026-07-25 with a standalone
/// binary: a `set_var` is INVISIBLE in `/proc/self/environ`, and a `remove_var` leaves
/// the inherited value still sitting there. Every one of the five keys above is
/// written by `configure_linux_webkit_compositing` AFTER exec, so a `/proc` reader
/// reports:
///
/// - **fresh launch** — the policy key simply ABSENT, the decision unobservable;
/// - **hot restart / `app launch`** — the PREDECESSOR's values, because the child
///   inherited its parent's mutated environ at exec while re-probing and possibly
///   deciding differently.
///
/// That is the decision and the state disagreeing silently — the exact failure this
/// whole module exists to end — so the publication path is in-process and there is
/// deliberately no `/proc`-derived copy of these keys to disagree with it.
///
/// Absent keys are absent from the map: "we removed it" and "it is empty" are
/// different facts and the wire keeps them apart.
pub fn webkit_gl_environment_from_process() -> std::collections::BTreeMap<String, String> {
    webkit_gl_environment_from(|key| std::env::var(key).ok())
}

/// The pure half of [`webkit_gl_environment_from_process`]: which keys get published,
/// given a lookup. Split out so the key coverage is testable without touching the
/// process environment.
pub fn webkit_gl_environment_from(
    lookup: impl Fn(&str) -> Option<String>,
) -> std::collections::BTreeMap<String, String> {
    WEBKIT_GL_ENVIRONMENT_KEYS
        .iter()
        .filter_map(|key| lookup(key).map(|value| ((*key).to_string(), value)))
        .collect()
}

/// Hidden argv flag selecting probe mode. Absent from every launcher and desktop
/// entry, so nothing acquires a probe by accident.
pub const GL_PROBE_FLAG: &str = "--internal-gl-probe";
/// Set on the child, checked by the parent, so a probe can never nest inside a probe.
pub const GL_PROBE_CHILD_ENV: &str = "YGGTERM_GL_PROBE_CHILD";
/// A probe that answers in single-digit milliseconds gets a budget three orders of
/// magnitude larger. Past this the answer is `Unknown`, never a guess.
pub const GL_PROBE_TIMEOUT_MS: u64 = 1_500;

/// The environment the probe child must NOT inherit.
///
/// ⚠ **This constant is the whole reason the probe is trustworthy.** A hot-restarted
/// or supervisor-relaunched GUI inherits its predecessor's process env (live-caught
/// 2026-07-20 as an SHM force outliving the run that set it). On a host currently
/// pinned to software that env contains `LIBGL_ALWAYS_SOFTWARE=1` and
/// `GALLIUM_DRIVER=llvmpipe`, so a probe child that inherited it would faithfully
/// report `llvmpipe` — and the host would stay pinned to software forever, with the
/// probe manufacturing the evidence for its own premise.
pub const GL_PROBE_STRIPPED_ENV: &[&str] = &[
    ENV_LIBGL_ALWAYS_SOFTWARE,
    ENV_GALLIUM_DRIVER,
    ENV_WEBKIT_DISABLE_DMABUF_RENDERER,
    "WEBKIT_DISABLE_COMPOSITING_MODE",
];

/// True when this process was asked to BE the probe child.
pub fn should_run_as_gl_probe(args: &[String]) -> bool {
    args.iter().any(|arg| arg == GL_PROBE_FLAG)
}

/// The one wire format between child and parent. Tab-separated so a renderer string
/// containing spaces (`AMD Radeon 780M (radeonsi, phoenix, ACO)`) survives intact.
pub fn format_gl_probe_line(report: &GlProbeReport) -> String {
    format!(
        "class={}\tdriver={}\trenderer={}\treason={}",
        report.class.as_str(),
        report.driver.as_deref().unwrap_or(""),
        report.renderer.as_deref().unwrap_or(""),
        report.reason
    )
}

/// Longest field the parent will accept from the child. The child is our own binary,
/// but a truncated or interleaved pipe read must not become an unbounded trace field.
const GL_PROBE_FIELD_MAX: usize = 128;

fn probe_field(line: &str, key: &str) -> Option<String> {
    let value = line
        .split('\t')
        .find_map(|field| field.strip_prefix(key))?
        .trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.chars().take(GL_PROBE_FIELD_MAX).collect())
}

/// Parse the child's line. The inverse of [`format_gl_probe_line`]; a line without a
/// recognizable `class=` token is not a report at all.
pub fn parse_gl_probe_line(line: &str) -> Option<GlProbeReport> {
    let class = GlClass::from_str_token(&probe_field(line, "class=")?)?;
    Some(GlProbeReport {
        class,
        driver: probe_field(line, "driver="),
        renderer: probe_field(line, "renderer="),
        reason: probe_field(line, "reason=").unwrap_or_else(|| "probe_reasonless".to_string()),
        elapsed_ms: 0,
    })
}

/// Probe-child entry point: do the work, print one line, exit 0. Never fails the
/// process — an unhappy probe is a datum (`class=unknown`), not an outage.
pub fn run_gl_probe_child() -> i32 {
    let report = probe_in_this_process();
    println!("{}", format_gl_probe_line(&report));
    0
}

/// The exact command the parent spawns, built as data so a test can inspect the plan
/// instead of trusting a comment. In particular the `env_remove` calls from
/// [`GL_PROBE_STRIPPED_ENV`] are visible through `Command::get_envs()`.
pub fn gl_probe_command(current_exe: &Path) -> Command {
    let mut command = Command::new(current_exe);
    command
        .arg(GL_PROBE_FLAG)
        .env(GL_PROBE_CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in GL_PROBE_STRIPPED_ENV {
        command.env_remove(key);
    }
    command
}

/// Run a prepared probe command under a hard deadline and turn whatever happened into
/// a report.
///
/// Three failure modes, three named reasons, no panics: a child that hangs is killed
/// (`probe_timeout`), a child that dies on a signal is a datum (`probe_crashed` — a
/// SIGSEGV inside Mesa is exactly what the old never-touch-GL safety net was buying
/// with a 22x CPU bill), and a child that says nothing intelligible is
/// `probe_unreadable`.
pub fn run_gl_probe_command(mut command: Command, timeout_ms: u64) -> GlProbeReport {
    let started = Instant::now();
    let Ok(mut child) = command.spawn() else {
        return GlProbeReport::inconclusive("probe_spawn_failed");
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return GlProbeReport::inconclusive("probe_wait_failed");
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let mut report = GlProbeReport::inconclusive("probe_timeout");
            report.elapsed_ms = started.elapsed().as_millis() as u64;
            return report;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let mut report = if child_died_on_a_signal(&status) {
        GlProbeReport::inconclusive("probe_crashed")
    } else {
        let mut text = String::new();
        if let Some(stdout) = child.stdout.as_mut() {
            use std::io::Read as _;
            let _ = stdout.read_to_string(&mut text);
        }
        text.lines()
            .find_map(parse_gl_probe_line)
            .unwrap_or_else(|| GlProbeReport::inconclusive("probe_unreadable"))
    };
    report.elapsed_ms = elapsed_ms;
    report
}

#[cfg(unix)]
fn child_died_on_a_signal(status: &std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal().is_some()
}

#[cfg(not(unix))]
fn child_died_on_a_signal(_status: &std::process::ExitStatus) -> bool {
    false
}

/// The process-wide answer. `OnceLock`, so "probe once per process" is a property of
/// the type rather than a rule every caller has to remember: the startup trace and the
/// policy read the same object, and a second caller cannot produce a second verdict.
static GL_PROBE_REPORT: OnceLock<GlProbeReport> = OnceLock::new();

/// Probe in a child process, once per process. Deliberately NOT cached to disk.
pub fn probe_via_child_once(current_exe: &Path) -> &'static GlProbeReport {
    GL_PROBE_REPORT.get_or_init(|| {
        // Belt to the argv braces: a probe child must never spawn a probe of its own,
        // however its argv got rebuilt.
        if std::env::var(GL_PROBE_CHILD_ENV).ok().as_deref() == Some("1") {
            return GlProbeReport::inconclusive("probe_child_recursion_refused");
        }
        run_gl_probe_command(gl_probe_command(current_exe), GL_PROBE_TIMEOUT_MS)
    })
}

/// The probe's verdict if one was taken in this process, for readers (the startup
/// trace) that must not trigger one of their own.
pub fn gl_probe_report() -> Option<&'static GlProbeReport> {
    GL_PROBE_REPORT.get()
}

// ---------------------------------------------------------------------------
// The EGL half. ONLY ever called inside the probe child.
// ---------------------------------------------------------------------------

/// `EGL_PLATFORM_SURFACELESS_MESA`. **Not** `EGL_PLATFORM_GBM_KHR` (0x31D7): GBM is
/// the one platform that reported llvmpipe on this host, because it opens `card0` and
/// takes EACCES while the compositor holds DRM master. Probing GBM is how the false
/// premise was manufactured in the first place.
#[cfg(target_os = "linux")]
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
#[cfg(target_os = "linux")]
const EGL_EXTENSIONS: i32 = 0x3055;
#[cfg(target_os = "linux")]
const EGL_OPENGL_ES_API: u32 = 0x30A0;
#[cfg(target_os = "linux")]
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
#[cfg(target_os = "linux")]
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
#[cfg(target_os = "linux")]
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
#[cfg(target_os = "linux")]
const EGL_NONE: i32 = 0x3038;
#[cfg(target_os = "linux")]
const GL_RENDERER: u32 = 0x1F01;

// One declaration per symbol, so a signature can never be spelled two ways.
#[cfg(target_os = "linux")]
type EglGetProcAddress =
    unsafe extern "C" fn(*const std::ffi::c_char) -> Option<unsafe extern "C" fn()>;
#[cfg(target_os = "linux")]
type EglGetPlatformDisplayExt =
    unsafe extern "C" fn(u32, *mut std::ffi::c_void, *const i32) -> *mut std::ffi::c_void;
#[cfg(target_os = "linux")]
type EglInitialize = unsafe extern "C" fn(*mut std::ffi::c_void, *mut i32, *mut i32) -> u32;
#[cfg(target_os = "linux")]
type EglQueryString = unsafe extern "C" fn(*mut std::ffi::c_void, i32) -> *const std::ffi::c_char;
#[cfg(target_os = "linux")]
type EglGetDisplayDriverName =
    unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::ffi::c_char;
#[cfg(target_os = "linux")]
type EglBindApi = unsafe extern "C" fn(u32) -> u32;
#[cfg(target_os = "linux")]
type EglChooseConfig = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *const i32,
    *mut *mut std::ffi::c_void,
    i32,
    *mut i32,
) -> u32;
#[cfg(target_os = "linux")]
type EglCreateContext = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *const i32,
) -> *mut std::ffi::c_void;
#[cfg(target_os = "linux")]
type EglMakeCurrent = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
) -> u32;
#[cfg(target_os = "linux")]
type GlGetString = unsafe extern "C" fn(u32) -> *const u8;

#[cfg(target_os = "linux")]
unsafe fn c_string_to_owned(pointer: *const std::ffi::c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_str()
        .ok()
        .map(str::to_string)
}

/// Ask this host's EGL stack what it is. Runs in the probe child ONLY.
///
/// Cheapest-first, and it stops at the first conclusive answer:
/// 1. no render node ⇒ nothing to ask;
/// 2. `EGL_MESA_query_driver` ⇒ the driver NAME, with no context creation at all
///    (this is the path jojo's Mesa takes, and it is why the probe costs
///    single-digit milliseconds);
/// 3. otherwise a throwaway ES2 context and `GL_RENDERER` — the tier that matters for
///    non-Mesa stacks, which do not implement the query-driver extension.
#[cfg(target_os = "linux")]
pub fn probe_in_this_process() -> GlProbeReport {
    if !render_node_present() {
        return GlProbeReport::inconclusive("no_render_node");
    }
    let library = match unsafe { libloading::Library::new("libEGL.so.1") }
        .or_else(|_| unsafe { libloading::Library::new("libEGL.so") })
    {
        Ok(library) => library,
        Err(_) => return GlProbeReport::inconclusive("no_libegl"),
    };
    let report = unsafe { probe_with_egl(&library) };
    // Deliberately never dlclose: this process is about to exit, and unloading a
    // graphics driver after a context has been made current is a known way to crash
    // inside the driver's teardown. The child's job is to answer, not to tidy up.
    std::mem::forget(library);
    report
}

#[cfg(target_os = "linux")]
unsafe fn probe_with_egl(library: &libloading::Library) -> GlProbeReport {
    macro_rules! symbol {
        ($name:literal, $ty:ty, $reason:literal) => {
            match unsafe { library.get::<$ty>($name) } {
                Ok(symbol) => *symbol,
                Err(_) => return GlProbeReport::inconclusive($reason),
            }
        };
    }

    let egl_get_proc_address = symbol!(
        b"eglGetProcAddress\0",
        EglGetProcAddress,
        "no_egl_get_proc_address"
    );
    let egl_initialize = symbol!(b"eglInitialize\0", EglInitialize, "no_egl_initialize");
    let egl_query_string = symbol!(b"eglQueryString\0", EglQueryString, "no_egl_query_string");

    macro_rules! extension {
        ($name:literal, $ty:ty) => {
            unsafe { egl_get_proc_address($name.as_ptr().cast()) }
                .map(|pointer| unsafe { std::mem::transmute::<_, $ty>(pointer) })
        };
    }

    let Some(get_platform_display) =
        extension!(b"eglGetPlatformDisplayEXT\0", EglGetPlatformDisplayExt)
    else {
        return GlProbeReport::inconclusive("no_platform_display");
    };
    let display = unsafe {
        get_platform_display(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if display.is_null() {
        return GlProbeReport::inconclusive("no_surfaceless_display");
    }
    let (mut major, mut minor) = (0i32, 0i32);
    if unsafe { egl_initialize(display, &mut major, &mut minor) } == 0 {
        return GlProbeReport::inconclusive("egl_initialize_failed");
    }
    let extensions =
        unsafe { c_string_to_owned(egl_query_string(display, EGL_EXTENSIONS)) }.unwrap_or_default();

    // Tier 2: the driver name, no context needed.
    if extensions.contains("EGL_MESA_query_driver")
        && let Some(get_driver_name) =
            extension!(b"eglGetDisplayDriverName\0", EglGetDisplayDriverName)
        && let Some(driver) = unsafe { c_string_to_owned(get_driver_name(display)) }
    {
        let class = classify_gl_strings(Some(&driver), None);
        return GlProbeReport {
            class,
            driver: Some(driver),
            renderer: None,
            reason: "egl_driver_name".to_string(),
            elapsed_ms: 0,
        };
    }

    // Tier 3: a throwaway context, only where the cheap answer is unavailable. It
    // needs surfaceless contexts; without that extension we would have to create a
    // real surface, which needs a display server, which a probe must not require.
    if !extensions.contains("EGL_KHR_surfaceless_context") {
        return GlProbeReport::inconclusive("no_surfaceless_context");
    }
    let egl_bind_api = symbol!(b"eglBindAPI\0", EglBindApi, "no_egl_bind_api");
    let egl_choose_config = symbol!(
        b"eglChooseConfig\0",
        EglChooseConfig,
        "no_egl_choose_config"
    );
    let egl_create_context = symbol!(
        b"eglCreateContext\0",
        EglCreateContext,
        "no_egl_create_context"
    );
    let egl_make_current = symbol!(b"eglMakeCurrent\0", EglMakeCurrent, "no_egl_make_current");
    if unsafe { egl_bind_api(EGL_OPENGL_ES_API) } == 0 {
        return GlProbeReport::inconclusive("egl_bind_api_failed");
    }
    let config_attributes = [EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
    let mut config: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut config_count = 0i32;
    if unsafe {
        egl_choose_config(
            display,
            config_attributes.as_ptr(),
            &mut config,
            1,
            &mut config_count,
        )
    } == 0
        || config_count == 0
    {
        return GlProbeReport::inconclusive("no_egl_config");
    }
    let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let context = unsafe {
        egl_create_context(
            display,
            config,
            std::ptr::null_mut(),
            context_attributes.as_ptr(),
        )
    };
    if context.is_null() {
        return GlProbeReport::inconclusive("no_egl_context");
    }
    if unsafe { egl_make_current(display, std::ptr::null_mut(), std::ptr::null_mut(), context) }
        == 0
    {
        return GlProbeReport::inconclusive("egl_make_current_failed");
    }
    let Some(gl_get_string) = extension!(b"glGetString\0", GlGetString) else {
        return GlProbeReport::inconclusive("no_gl_get_string");
    };
    let Some(renderer) = (unsafe { c_string_to_owned(gl_get_string(GL_RENDERER).cast()) }) else {
        return GlProbeReport::inconclusive("no_gl_renderer");
    };
    let class = classify_gl_strings(None, Some(&renderer));
    GlProbeReport {
        class,
        driver: None,
        renderer: Some(renderer),
        reason: "gl_renderer_string".to_string(),
        elapsed_ms: 0,
    }
}

/// Non-Linux hosts do not run this policy at all: WebKit's GL configuration is a
/// Linux-only concern in this tree.
#[cfg(not(target_os = "linux"))]
pub fn probe_in_this_process() -> GlProbeReport {
    GlProbeReport::inconclusive("not_linux")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the two tests that mutate the process environment. `set_var` is
    /// process-global and cargo runs tests on threads, so without this they race each
    /// other — and a test that passes or fails by timing is worth nothing.
    static ENV_MUTATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ⚠⚠ THE INSTRUMENT-LIE LOCK. The GL decision must be published from THIS
    /// process's own view of its environment, never from `/proc/<pid>/environ`.
    ///
    /// `set_var`/`remove_var` move the environ array to the heap while the kernel keeps
    /// exposing the exec-time stack copy, so a `/proc` reader reports the value this
    /// process was LAUNCHED with — absent on a fresh launch, and the PREDECESSOR's on a
    /// hot restart, while this process re-probed and may have decided differently.
    /// Measured on the dev host 2026-07-25 with a standalone binary: a `set_var` was
    /// invisible in `/proc/self/environ` and a `remove_var` left the inherited value in
    /// place.
    ///
    /// This test fails the moment the publisher starts inferring from `/proc` again:
    /// the set would not be seen, and the removal would still report its old value.
    #[test]
    fn the_gl_decision_is_published_from_this_processs_own_view() {
        let _guard = ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        unsafe {
            std::env::set_var(ENV_LIBGL_ALWAYS_SOFTWARE, "1");
            std::env::set_var(ENV_GALLIUM_DRIVER, "llvmpipe");
        }
        // Exactly what a hot-restarted GUI does after a hardware probe.
        unsafe {
            std::env::set_var(ENV_YGGTERM_WEBKIT_GL_POLICY, "hardware_gl_probed");
            std::env::remove_var(ENV_LIBGL_ALWAYS_SOFTWARE);
            std::env::remove_var(ENV_GALLIUM_DRIVER);
        }
        let published = webkit_gl_environment_from_process();
        assert_eq!(
            published
                .get(ENV_YGGTERM_WEBKIT_GL_POLICY)
                .map(String::as_str),
            Some("hardware_gl_probed"),
            "a decision this process made must be visible to the reader that publishes it"
        );
        assert!(
            !published.contains_key(ENV_LIBGL_ALWAYS_SOFTWARE),
            "a software force this process CLEARED must not still be reported as set — \
             that is the decision and the state disagreeing silently, published"
        );
        assert!(
            !published.contains_key(ENV_GALLIUM_DRIVER),
            "same for the driver override"
        );
        unsafe { std::env::remove_var(ENV_YGGTERM_WEBKIT_GL_POLICY) };
    }

    /// Absent is not empty. A variable nobody set and a variable set to "" are
    /// different facts, and the map keeps them apart.
    #[test]
    fn the_published_gl_environment_omits_only_what_is_unset() {
        let _guard = ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let published = webkit_gl_environment_from(|key| {
            (key == ENV_YGGTERM_WEB_SURFACE_UNDER_GLASS).then(|| String::new())
        });
        assert_eq!(
            published.get(ENV_YGGTERM_WEB_SURFACE_UNDER_GLASS),
            Some(&String::new())
        );
        assert_eq!(published.len(), 1);
        let all = webkit_gl_environment_from(|key| Some(format!("v-{key}")));
        assert_eq!(all.len(), WEBKIT_GL_ENVIRONMENT_KEYS.len());
    }

    /// The exact strings from the settled EGL matrix on the live host. If a marker is
    /// ever added in a second place instead of `SOFTWARE_RENDERER_MARKERS`, this is
    /// what notices.
    #[test]
    fn classifies_the_live_hosts_egl_matrix() {
        assert_eq!(
            classify_gl_strings(None, Some("AMD Radeon 780M (radeonsi, phoenix, ACO)")),
            GlClass::Hardware
        );
        assert_eq!(
            classify_gl_strings(None, Some("llvmpipe (LLVM 19.1.7, 256 bits)")),
            GlClass::Software
        );
        assert_eq!(
            classify_gl_strings(Some("radeonsi"), None),
            GlClass::Hardware
        );
        assert_eq!(classify_gl_strings(Some("iris"), None), GlClass::Hardware);
        assert_eq!(
            classify_gl_strings(Some("llvmpipe"), None),
            GlClass::Software
        );
        assert_eq!(classify_gl_strings(Some("swrast"), None), GlClass::Software);
    }

    /// A hardware-sounding driver name in front of a software renderer is SOFTWARE —
    /// zink over lavapipe is exactly that shape, and reading only the driver name
    /// would call it hardware.
    #[test]
    fn a_software_marker_in_either_string_wins() {
        assert_eq!(
            classify_gl_strings(
                Some("zink"),
                Some("zink Vulkan 1.3(llvmpipe (LLVM 19.1.7))")
            ),
            GlClass::Software
        );
        assert_eq!(
            classify_gl_strings(Some("SwiftShader"), Some("Google SwiftShader")),
            GlClass::Software
        );
    }

    /// Nothing observed must read as Unknown, never as Hardware. "We could not tell"
    /// and "we checked and it is fine" are different answers, and conflating them is
    /// how a host gets the wrong default silently.
    #[test]
    fn nothing_observed_is_unknown_not_hardware() {
        assert_eq!(classify_gl_strings(None, None), GlClass::Unknown);
        assert_eq!(classify_gl_strings(Some(""), Some("   ")), GlClass::Unknown);
    }

    #[test]
    fn the_wire_format_round_trips() {
        let report = GlProbeReport {
            class: GlClass::Hardware,
            driver: Some("radeonsi".into()),
            // Spaces, commas and parens: the reason the format is tab-separated.
            renderer: Some("AMD Radeon 780M (radeonsi, phoenix, ACO)".into()),
            reason: "egl_driver_name".into(),
            elapsed_ms: 0,
        };
        let parsed = parse_gl_probe_line(&format_gl_probe_line(&report)).expect("round trip");
        assert_eq!(parsed, report);
        // An empty optional field comes back as None, not as Some("").
        let sparse = GlProbeReport::inconclusive("no_render_node");
        assert_eq!(
            parse_gl_probe_line(&format_gl_probe_line(&sparse)).expect("round trip"),
            sparse
        );
        // Junk is not a report.
        assert!(parse_gl_probe_line("").is_none());
        assert!(parse_gl_probe_line("class=maybe\tdriver=x").is_none());
    }

    /// ⚠ THE anti-poisoning lock. Without the `env_remove` calls a hot-restarted GUI
    /// hands the probe child `LIBGL_ALWAYS_SOFTWARE=1`, the child reports llvmpipe,
    /// and the host is pinned to software forever by evidence it manufactured itself.
    /// Asserted structurally against the command plan, so it cannot pass by comment.
    #[test]
    fn the_probe_child_cannot_inherit_the_answer() {
        let command = gl_probe_command(Path::new("/nonexistent/yggterm"));
        let envs: Vec<_> = command
            .get_envs()
            .map(|(key, value)| (key.to_string_lossy().to_string(), value.is_some()))
            .collect();
        for key in GL_PROBE_STRIPPED_ENV {
            assert!(
                envs.contains(&((*key).to_string(), false)),
                "{key} must be REMOVED from the probe child's environment, not merely unset here"
            );
        }
        assert!(envs.contains(&(GL_PROBE_CHILD_ENV.to_string(), true)));
        assert!(GL_PROBE_STRIPPED_ENV.contains(&"LIBGL_ALWAYS_SOFTWARE"));
        assert!(GL_PROBE_STRIPPED_ENV.contains(&"GALLIUM_DRIVER"));
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec![GL_PROBE_FLAG.to_string()]);
    }

    /// The A/B harness scrubs the GL environment before every arm launch, and its
    /// list must not drift below the binary's own. [`GL_PROBE_STRIPPED_ENV`] is the
    /// SSOT for "environment that must not be allowed to decide the GL path"; a key
    /// added there and missed in the harness means an arm launches carrying an
    /// inherited answer and reports `hardware_gl_probed` while presenting over SHM.
    /// That failure is silent in both directions, which is why it is locked rather
    /// than documented.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_gl_ab_harness_scrubs_every_key_the_probe_strips() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("scripts")
            .join("gl_ab_experiment.sh");
        let text = std::fs::read_to_string(&script).expect("gl_ab_experiment.sh must be readable");
        let scrub = text
            .split("GL_KEYS=(")
            .nth(1)
            .and_then(|suffix| suffix.split(')').next())
            .expect("gl_ab_experiment.sh must declare a GL_KEYS scrub list");
        // Coverage floor: an empty capture would satisfy nothing below.
        assert!(
            scrub.lines().count() > 4,
            "the scrub-list scanner captured {} lines — it has gone blind",
            scrub.lines().count()
        );
        for key in GL_PROBE_STRIPPED_ENV {
            assert!(
                scrub.contains(key),
                "gl_ab_experiment.sh does not scrub {key}, which the probe strips — \
                 an arm launched with it inherited would report hardware while \
                 presenting over SHM"
            );
        }
    }

    #[test]
    fn the_probe_flag_selects_probe_mode() {
        assert!(should_run_as_gl_probe(&[GL_PROBE_FLAG.to_string()]));
        assert!(should_run_as_gl_probe(&[
            "--agent".to_string(),
            GL_PROBE_FLAG.to_string()
        ]));
        assert!(!should_run_as_gl_probe(&["server".to_string()]));
        assert!(!should_run_as_gl_probe(&[]));
    }

    /// A probe that hangs must cost a bounded amount of startup and answer Unknown.
    /// The whole point of the child is that GL misbehaving is a datum, not an outage.
    #[cfg(unix)]
    #[test]
    fn a_hanging_probe_times_out_into_unknown() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30").stdout(Stdio::piped());
        let report = run_gl_probe_command(command, 150);
        assert_eq!(report.class, GlClass::Unknown);
        assert_eq!(report.reason, "probe_timeout");
    }

    /// A SIGSEGV inside Mesa is precisely what the old never-touch-GL safety net was
    /// paying 22x CPU to avoid. Here it is one Unknown.
    #[cfg(unix)]
    #[test]
    fn a_crashing_probe_reads_as_unknown_not_hardware() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("kill -SEGV $$")
            .stdout(Stdio::piped());
        let report = run_gl_probe_command(command, 5_000);
        assert_eq!(report.class, GlClass::Unknown);
        assert_eq!(report.reason, "probe_crashed");
    }

    /// A child that exits cleanly saying nothing we understand is also Unknown — the
    /// parse failure must not fall through to a default of Hardware.
    #[cfg(unix)]
    #[test]
    fn a_silent_probe_reads_as_unknown() {
        let mut command = Command::new("/bin/true");
        command.stdout(Stdio::piped());
        let report = run_gl_probe_command(command, 5_000);
        assert_eq!(report.class, GlClass::Unknown);
        assert_eq!(report.reason, "probe_unreadable");
    }

    /// A spawn that cannot happen at all is still an answer, not a panic.
    #[test]
    fn an_unspawnable_probe_reads_as_unknown() {
        let report = run_gl_probe_command(
            gl_probe_command(Path::new("/nonexistent/yggterm-probe")),
            5_000,
        );
        assert_eq!(report.class, GlClass::Unknown);
        assert_eq!(report.reason, "probe_spawn_failed");
    }

    /// The stray-vendor predicate: fires only when BOTH ICDs exist and no
    /// nvidia device node does — every other combination stays out, including
    /// the one where pinning would break EGL (Mesa ICD missing).
    #[test]
    fn the_stray_nvidia_vendor_predicate_requires_both_icds_and_no_device() {
        let root = std::env::temp_dir().join(format!("yggterm-glvnd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let icds = root.join("usr/share/glvnd/egl_vendor.d");
        std::fs::create_dir_all(&icds).unwrap();
        std::fs::create_dir_all(root.join("dev")).unwrap();

        // Nothing installed: no opinion.
        assert!(!stray_nvidia_egl_vendor_at(&root));
        // NVIDIA ICD alone (no Mesa to pin to): must stay out.
        std::fs::write(icds.join("10_nvidia.json"), b"{}").unwrap();
        assert!(!stray_nvidia_egl_vendor_at(&root));
        // Both ICDs, no device: THE misconfiguration.
        std::fs::write(icds.join("50_mesa.json"), b"{}").unwrap();
        assert!(stray_nvidia_egl_vendor_at(&root));
        // A real nvidia device appears: the vendor is not stray.
        std::fs::write(root.join("dev/nvidia0"), b"").unwrap();
        assert!(!stray_nvidia_egl_vendor_at(&root));
        // nvidiactl alone counts as a device too.
        std::fs::remove_file(root.join("dev/nvidia0")).unwrap();
        std::fs::write(root.join("dev/nvidiactl"), b"").unwrap();
        assert!(!stray_nvidia_egl_vendor_at(&root));

        let _ = std::fs::remove_dir_all(&root);
    }
}
