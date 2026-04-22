use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use cairo::{Context as CairoContext, Format, ImageSurface, Surface};
use dioxus::desktop::DesktopContext;
use serde_json::{Value, json};
use std::fs;
#[cfg(target_os = "linux")]
use std::fs::File;
use std::path::{Path, PathBuf};
use tao::dpi::PhysicalPosition;
use yggterm_server::ScreenshotTarget;

#[cfg(target_os = "macos")]
use tao::platform::macos::WindowExtMacOS;
#[cfg(target_os = "linux")]
use tao::platform::unix::WindowExtUnix;
#[cfg(target_os = "windows")]
use tao::platform::windows::WindowExtWindows;
#[cfg(target_os = "linux")]
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
#[cfg(target_os = "linux")]
use wry::WebViewExtUnix;
#[cfg(target_os = "windows")]
use webview2_com::{Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, wait_with_pump};
#[cfg(target_os = "windows")]
use webview2_com::CapturePreviewCompletedHandler;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HGLOBAL;
#[cfg(target_os = "windows")]
use windows::core::Result as WindowsResult;
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{STATFLAG_NONAME, STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal};
#[cfg(target_os = "windows")]
use wry::WebViewExtWindows;
#[cfg(target_os = "linux")]
use yggterm_platform::capture_linux_x11_window_screenshot;
#[cfg(target_os = "windows")]
use yggterm_platform::capture_windows_hwnd_screenshot;
#[cfg(target_os = "windows")]
use yggterm_platform::capture_windows_window_screenshot;
#[cfg(target_os = "macos")]
use yggterm_platform::{capture_macos_window_recording, capture_macos_window_screenshot};

#[derive(Debug, Clone)]
pub struct SurfaceCapture {
    pub output_path: PathBuf,
    pub backend: &'static str,
    pub backend_attempts: Vec<String>,
}

impl SurfaceCapture {
    fn new(output_path: &Path, backend: &'static str) -> Self {
        Self {
            output_path: output_path.to_path_buf(),
            backend,
            backend_attempts: Vec::new(),
        }
    }

    #[cfg(target_os = "windows")]
    fn with_attempts(mut self, backend_attempts: Vec<String>) -> Self {
        self.backend_attempts = backend_attempts;
        self
    }
}

pub async fn capture_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
    target: ScreenshotTarget,
    dom_snapshot: Option<&Value>,
) -> Result<SurfaceCapture> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating screenshot dir {}", parent.display()))?;
    }
    let capture = platform_capture_visible_app_surface(desktop, output_path, target, dom_snapshot)
        .await?;
    let metadata = fs::metadata(&capture.output_path)
        .with_context(|| format!("reading screenshot metadata {}", capture.output_path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        anyhow::bail!("native screenshot capture produced no file output");
    }
    Ok(capture)
}

pub async fn record_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
    duration_secs: u64,
) -> Result<PathBuf> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating recording dir {}", parent.display()))?;
    }
    platform_record_visible_app_surface(desktop, output_path, duration_secs).await?;
    let metadata = fs::metadata(output_path)
        .with_context(|| format!("reading recording metadata {}", output_path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        anyhow::bail!("native screen recording produced no file output");
    }
    Ok(output_path.to_path_buf())
}

pub fn focus_app_window(desktop: &DesktopContext) -> Result<Value> {
    desktop.set_always_on_bottom(false);
    desktop.set_visible(true);
    desktop.set_minimized(false);
    desktop.set_focus();
    Ok(json!({
        "focused_requested": true,
        "focused": desktop.is_focused(),
        "window": describe_window(desktop),
    }))
}

pub fn background_app_window(desktop: &DesktopContext) -> Result<Value> {
    desktop.set_always_on_bottom(true);
    desktop.set_minimized(false);
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;

        if let Some(gdk_window) = desktop.gtk_window().window() {
            gdk_window.lower();
        }
    }
    Ok(json!({
        "background_requested": true,
        "keep_below_requested": true,
        "window": describe_window(desktop),
    }))
}

pub fn move_app_window_by(desktop: &DesktopContext, delta_x: f64, delta_y: f64) -> Result<Value> {
    let before_position = desktop
        .outer_position()
        .context("reading app window position before move")?;
    let next_x = before_position.x + delta_x.round() as i32;
    let next_y = before_position.y + delta_y.round() as i32;
    desktop.set_outer_position(PhysicalPosition::new(next_x, next_y));
    Ok(json!({
        "move_requested": true,
        "delta_x": delta_x,
        "delta_y": delta_y,
        "before_position": {
            "x": before_position.x,
            "y": before_position.y,
        },
        "window": describe_window(desktop),
    }))
}

pub fn describe_window(desktop: &DesktopContext) -> Value {
    let inner = desktop.inner_size();
    let outer = desktop.outer_size();
    let position = desktop.outer_position().ok();
    #[cfg(target_os = "windows")]
    let native_window_handle = Some(desktop.window.hwnd() as i64);
    #[cfg(not(target_os = "windows"))]
    let native_window_handle: Option<i64> = None;
    json!({
        "title": desktop.title(),
        "visible": desktop.is_visible(),
        "focused": desktop.is_focused(),
        "minimized": desktop.is_minimized(),
        "maximized": desktop.is_maximized(),
        "decorated": desktop.is_decorated(),
        "native_window_handle": native_window_handle,
        "display": std::env::var("DISPLAY").ok(),
        "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
        "xdg_session_id": std::env::var("XDG_SESSION_ID").ok(),
        "xdg_runtime_dir": std::env::var("XDG_RUNTIME_DIR").ok(),
        "xauthority": std::env::var("XAUTHORITY").ok(),
        "inner_size": {
            "width": inner.width,
            "height": inner.height,
        },
        "outer_size": {
            "width": outer.width,
            "height": outer.height,
        },
        "outer_position": position.map(|position| {
            json!({
                "x": position.x,
                "y": position.y,
            })
        }),
    })
}

#[cfg(target_os = "windows")]
fn capture_windows_webview_surface(desktop: &DesktopContext, output_path: &Path) -> Result<()> {
    let webview = desktop.webview.webview();
    let stream = unsafe {
        CreateStreamOnHGlobal(HGLOBAL::default(), true)
            .context("creating WebView2 in-memory screenshot stream")?
    };
    let (tx, rx) = std::sync::mpsc::channel();
    unsafe {
        webview
            .CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &CapturePreviewCompletedHandler::create(Box::new(move |error_code: WindowsResult<()>| {
                    let _ = tx.send(error_code.map(|_| ()));
                    Ok(())
                })),
            )
            .context("requesting WebView2 preview capture")?;
    }
    wait_with_pump(rx)
        .context("waiting for WebView2 preview capture")?
        .context("WebView2 preview capture failed")?;
    unsafe {
        stream
            .Seek(0, STREAM_SEEK_SET, None)
            .context("rewinding WebView2 screenshot stream")?;
    }
    let mut bytes = Vec::new();
    let mut stat = unsafe { std::mem::zeroed() };
    unsafe {
        stream
            .Stat(&mut stat, STATFLAG_NONAME)
            .context("reading WebView2 screenshot stream metadata")?;
    }
    if stat.cbSize > 0 {
        bytes.reserve(stat.cbSize as usize);
    }
    loop {
        let mut buffer = [0_u8; 16 * 1024];
        let mut read = 0_u32;
        unsafe {
            stream
                .Read(
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
                    Some(&mut read),
                )
                .ok()
                .context("reading WebView2 screenshot stream")?;
        }
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read as usize]);
    }
    if bytes.is_empty() {
        anyhow::bail!("WebView2 preview capture returned no image bytes");
    }
    fs::write(output_path, &bytes)
        .with_context(|| format!("writing Windows WebView2 screenshot {}", output_path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct CaptureRect {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    window_width: f64,
    window_height: f64,
}

#[cfg(target_os = "linux")]
fn preview_viewport_capture_rect(dom_snapshot: &Value) -> Result<CaptureRect> {
    let rect = dom_snapshot
        .get("preview_viewport_rect")
        .and_then(Value::as_object)
        .context("preview viewport rect missing from DOM snapshot")?;
    let read = |key: &str| -> Result<f64> {
        rect.get(key)
            .and_then(Value::as_f64)
            .with_context(|| format!("preview viewport rect missing numeric {key}"))
    };
    let window_width = dom_snapshot
        .get("window_inner_width")
        .and_then(Value::as_f64)
        .context("window inner width missing from DOM snapshot")?;
    let window_height = dom_snapshot
        .get("window_inner_height")
        .and_then(Value::as_f64)
        .context("window inner height missing from DOM snapshot")?;
    Ok(CaptureRect {
        left: read("left")?,
        top: read("top")?,
        width: read("width")?,
        height: read("height")?,
        window_width,
        window_height,
    })
}

#[cfg(target_os = "linux")]
fn crop_visible_surface_to_rect(
    surface: &Surface,
    rect: CaptureRect,
    output_path: &Path,
) -> Result<()> {
    let surface = ImageSurface::try_from(surface.clone())
        .map_err(|_| anyhow::anyhow!("webkit snapshot surface was not an image surface"))?;
    if rect.width <= 1.0 || rect.height <= 1.0 {
        anyhow::bail!("preview viewport rect is too small to capture");
    }
    let source_width = surface.width();
    let source_height = surface.height();
    if source_width <= 0 || source_height <= 0 {
        anyhow::bail!("webkit snapshot surface is empty");
    }
    let scale_x = source_width as f64 / rect.window_width.max(1.0);
    let scale_y = source_height as f64 / rect.window_height.max(1.0);
    let crop_left = (rect.left * scale_x).floor().max(0.0) as i32;
    let crop_top = (rect.top * scale_y).floor().max(0.0) as i32;
    let crop_width = (rect.width * scale_x).ceil().max(1.0) as i32;
    let crop_height = (rect.height * scale_y).ceil().max(1.0) as i32;
    let crop_right = (crop_left + crop_width).min(source_width);
    let crop_bottom = (crop_top + crop_height).min(source_height);
    let final_width = (crop_right - crop_left).max(1);
    let final_height = (crop_bottom - crop_top).max(1);

    let cropped = ImageSurface::create(Format::ARgb32, final_width, final_height)
        .context("creating preview viewport capture surface")?;
    let context = CairoContext::new(&cropped).context("creating preview viewport cairo context")?;
    context
        .set_source_surface(surface, -(crop_left as f64), -(crop_top as f64))
        .context("binding preview viewport crop source surface")?;
    context
        .paint()
        .context("painting preview viewport crop into output surface")?;
    let mut output = File::create(output_path)
        .with_context(|| format!("creating screenshot file {}", output_path.display()))?;
    cropped
        .write_to_png(&mut output)
        .with_context(|| format!("writing screenshot png {}", output_path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn platform_capture_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
    target: ScreenshotTarget,
    dom_snapshot: Option<&Value>,
) -> Result<SurfaceCapture> {
    if target == ScreenshotTarget::App
        && std::env::var_os("DISPLAY").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
        && capture_linux_x11_window_screenshot(std::process::id(), output_path).is_ok()
    {
        return Ok(SurfaceCapture::new(output_path, "linux_x11_window"));
    }
    let gtk_webview = desktop.webview.webview();
    let surface = gtk_webview
        .snapshot_future(SnapshotRegion::Visible, SnapshotOptions::NONE)
        .await
        .context("capturing visible app surface from WebKitGTK")?;
    match target {
        ScreenshotTarget::App => {
            let mut output = File::create(output_path)
                .with_context(|| format!("creating screenshot file {}", output_path.display()))?;
            surface
                .write_to_png(&mut output)
                .with_context(|| format!("writing screenshot png {}", output_path.display()))?;
            Ok(SurfaceCapture::new(output_path, "linux_webkit_snapshot"))
        }
        ScreenshotTarget::PreviewViewport => {
            let rect = preview_viewport_capture_rect(
                dom_snapshot.context("preview viewport capture requires a DOM snapshot")?,
            )?;
            crop_visible_surface_to_rect(&surface, rect, output_path)?;
            Ok(SurfaceCapture::new(output_path, "linux_preview_viewport_crop"))
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn platform_capture_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
    target: ScreenshotTarget,
    dom_snapshot: Option<&Value>,
) -> Result<SurfaceCapture> {
    #[cfg(target_os = "windows")]
    {
        let _ = dom_snapshot;
        if target != ScreenshotTarget::App {
            anyhow::bail!("preview viewport screenshot capture is not implemented for Windows yet");
        }
        let mut backend_attempts = Vec::new();
        match capture_windows_webview_surface(desktop, output_path) {
            Ok(()) => {
                return Ok(SurfaceCapture::new(
                    output_path,
                    "windows_webview2_capture_preview",
                ));
            }
            Err(error) => {
                backend_attempts.push(format!(
                    "windows_webview2_capture_preview: {error:#}"
                ));
            }
        }
        let hwnd = desktop.window.hwnd();
        let size = desktop.outer_size();
        match capture_windows_hwnd_screenshot(output_path, hwnd, size.width, size.height) {
            Ok(()) => {
                return Ok(
                    SurfaceCapture::new(output_path, "windows_printwindow")
                        .with_attempts(backend_attempts),
                );
            }
            Err(error) => {
                backend_attempts.push(format!("windows_printwindow: {error:#}"));
            }
        }
        let position = desktop
            .outer_position()
            .context("reading Windows window position for screenshot")?;
        capture_windows_window_screenshot(
            output_path,
            position.x,
            position.y,
            size.width,
            size.height,
        )?;
        return Ok(
            SurfaceCapture::new(output_path, "windows_screen_copy")
                .with_attempts(backend_attempts),
        );
    }

    #[cfg(target_os = "macos")]
    {
        if target != ScreenshotTarget::App {
            anyhow::bail!("preview viewport screenshot capture is not implemented for macOS yet");
        }
        capture_macos_window_screenshot(desktop.window.ns_window().cast(), output_path)?;
        return Ok(SurfaceCapture::new(output_path, "macos_window_screenshot"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = desktop;
        let _ = output_path;
        let _ = target;
        let _ = dom_snapshot;
        anyhow::bail!("native app screenshot capture is not implemented for this platform yet")
    }
}

#[cfg(target_os = "linux")]
async fn platform_record_visible_app_surface(
    _desktop: &DesktopContext,
    _output_path: &Path,
    _duration_secs: u64,
) -> Result<()> {
    anyhow::bail!("native app screen recording is not implemented for Linux yet")
}

#[cfg(not(target_os = "linux"))]
async fn platform_record_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
    duration_secs: u64,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return capture_macos_window_recording(
            desktop.window.ns_window().cast(),
            output_path,
            duration_secs,
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = desktop;
        let _ = output_path;
        let _ = duration_secs;
        anyhow::bail!("native app screen recording is not implemented for this platform yet")
    }
}
