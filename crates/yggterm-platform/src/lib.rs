use anyhow::{Context, Result, bail};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(target_os = "macos")]
use objc2::{AnyThread, msg_send, runtime::AnyObject};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey};
#[cfg(target_os = "macos")]
use objc2_core_graphics::{CGWindowImageOption, CGWindowListOption, CGRectNull};
#[cfg(target_os = "macos")]
use objc2_foundation::NSDictionary;
#[cfg(target_os = "macos")]
use png::Transformations;
#[cfg(target_os = "windows")]
use windows::Win32::System::Console::{FreeConsole, GetConsoleWindow};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
#[cfg(target_os = "windows")]
use windows::core::HSTRING;

#[derive(Debug, Clone, Copy)]
pub enum HostPlatform {
    Linux,
    MacOS,
    Windows,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlledGhosttyAdapter {
    LinuxX11Dock,
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct ControlledGhosttyStatus {
    pub adapter: ControlledGhosttyAdapter,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ControlledGhosttyLaunch {
    pub process_id: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct DockedGhosttyWindow {
    pub window_id: String,
}

pub fn configure_gui_entry_process(app_name: &str, app_id: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let _ = app_name;
        let app_id = HSTRING::from(app_id);
        unsafe {
            SetCurrentProcessExplicitAppUserModelID(&app_id)
                .context("setting Windows AppUserModelID for GUI entry")?;
            let console = GetConsoleWindow();
            if !console.0.is_null() {
                let _ = ShowWindow(console, SW_HIDE);
                let _ = FreeConsole();
            }
        }
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_name;
        let _ = app_id;
        Ok(())
    }
}

pub fn send_user_notification(title: &str, message: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("notify-send")
            .arg(title)
            .arg(message)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawning notify-send")?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification {} with title {}",
            serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_string()),
            serde_json::to_string(title).unwrap_or_else(|_| "\"Yggterm\"".to_string())
        );
        Command::new("osascript")
            .arg("-e")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawning macOS notification")?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = title;
        let _ = message;
        bail!("native notifications are not implemented for this platform yet");
    }
}

#[cfg(target_os = "linux")]
pub fn capture_linux_x11_window_screenshot(pid: u32, output_path: &Path) -> Result<()> {
    let window_id = xdotool_search(["search", "--onlyvisible", "--pid", &pid.to_string()])
        .context("resolving current X11 window for screenshot")?;
    let temp_xwd = output_path.with_extension("xwd");
    let capture_status = Command::new("xwd")
        .args([
            "-silent",
            "-id",
            &window_id,
            "-out",
            temp_xwd.to_string_lossy().as_ref(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running xwd for window screenshot")?;
    if !capture_status.success() {
        let _ = std::fs::remove_file(&temp_xwd);
        bail!("xwd exited with status {capture_status}");
    }
    let convert_status = Command::new("convert")
        .args([
            temp_xwd.to_string_lossy().as_ref(),
            output_path.to_string_lossy().as_ref(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running ImageMagick convert for window screenshot")?;
    let _ = std::fs::remove_file(&temp_xwd);
    if !convert_status.success() {
        bail!("convert exited with status {convert_status}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn capture_macos_window_screenshot(
    ns_window_ptr: *mut std::ffi::c_void,
    output_path: &Path,
) -> Result<()> {
    match capture_macos_window_cg_screenshot(ns_window_ptr, output_path) {
        Ok(()) => Ok(()),
        Err(cg_error) => capture_macos_window_screencapture(ns_window_ptr, output_path)
            .with_context(|| format!("fallback screencapture failed after CG capture error: {cg_error:#}")),
    }
}

#[cfg(target_os = "macos")]
pub fn capture_macos_window_cg_screenshot(
    ns_window_ptr: *mut std::ffi::c_void,
    output_path: &Path,
) -> Result<()> {
    let window_number = macos_window_number(ns_window_ptr)?;
    #[allow(deprecated)]
    let cg_image = objc2_core_graphics::CGWindowListCreateImage(
        unsafe { CGRectNull },
        CGWindowListOption::OptionIncludingWindow,
        window_number as u32,
        CGWindowImageOption::BestResolution,
    )
    .with_context(|| format!("capturing CG window image for window {window_number}"))?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg_image);
    let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
    let png_data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .context("encoding CG window image as PNG")?;
    std::fs::write(output_path, png_data.to_vec())
        .with_context(|| format!("writing macOS CG window screenshot {}", output_path.display()))?;
    ensure_macos_png_not_blank(output_path, "CG window capture")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_png_not_blank(output_path: &Path, context_label: &str) -> Result<()> {
    let file = std::fs::File::open(output_path)
        .with_context(|| format!("opening macOS screenshot {}", output_path.display()))?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("reading PNG info for {}", output_path.display()))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .with_context(|| format!("decoding PNG pixels for {}", output_path.display()))?;
    let pixels = &buffer[..info.buffer_size()];
    if pixels.is_empty() {
        bail!(
            "{context_label} wrote an empty PNG for {}; refusing the capture",
            output_path.display()
        );
    }
    if pixels.iter().all(|byte| *byte == 0) {
        bail!(
            "{context_label} produced an all-zero PNG for {}; likely a blank window capture",
            output_path.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn capture_macos_window_screencapture(
    ns_window_ptr: *mut std::ffi::c_void,
    output_path: &Path,
) -> Result<()> {
    let window_number = macos_window_number(ns_window_ptr)?;
    let status = Command::new("screencapture")
        .arg("-x")
        .arg("-o")
        .arg("-l")
        .arg(window_number.to_string())
        .arg(output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running macOS screencapture for window screenshot")?;
    if !status.success() {
        bail!("screencapture exited with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn capture_macos_window_recording(
    ns_window_ptr: *mut std::ffi::c_void,
    output_path: &Path,
    duration_secs: u64,
) -> Result<()> {
    let window_number = macos_window_number(ns_window_ptr)?;
    let duration_secs = duration_secs.max(1);
    let status = Command::new("screencapture")
        .arg("-x")
        .arg("-v")
        .arg("-l")
        .arg(window_number.to_string())
        .arg("-V")
        .arg(duration_secs.to_string())
        .arg(output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running macOS screencapture for window recording")?;
    if !status.success() {
        bail!("screencapture video capture exited with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn capture_windows_window_screenshot(
    output_path: &Path,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<()> {
    if width == 0 || height == 0 {
        bail!("cannot capture a Windows window screenshot with an empty rect");
    }
    let output_literal = output_path.display().to_string().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         Add-Type -AssemblyName System.Drawing; \
         $bitmap = New-Object System.Drawing.Bitmap({width}, {height}); \
         $graphics = [System.Drawing.Graphics]::FromImage($bitmap); \
         try {{ \
           $graphics.CopyFromScreen({x}, {y}, 0, 0, (New-Object System.Drawing.Size({width}, {height}))); \
         }} finally {{ \
           $graphics.Dispose(); \
         }}; \
         try {{ \
           $bitmap.Save('{output_literal}', [System.Drawing.Imaging.ImageFormat]::Png); \
         }} finally {{ \
           $bitmap.Dispose(); \
         }}"
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running Windows PowerShell screenshot capture")?;
    if !status.success() {
        bail!("powershell screenshot capture exited with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn capture_windows_hwnd_screenshot(
    output_path: &Path,
    hwnd: isize,
    width: u32,
    height: u32,
) -> Result<()> {
    if hwnd == 0 {
        bail!("cannot capture a Windows HWND screenshot without a native window handle");
    }
    if width == 0 || height == 0 {
        bail!("cannot capture a Windows HWND screenshot with an empty rect");
    }
    let output_literal = output_path.display().to_string().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         Add-Type -AssemblyName System.Drawing; \
         Add-Type @\"\n\
using System;\n\
using System.Runtime.InteropServices;\n\
public static class YggtermPrintWindowCapture {{\n\
  [DllImport(\"user32.dll\")]\n\
  public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, int nFlags);\n\
}}\n\
\"@; \
         $bitmap = New-Object System.Drawing.Bitmap({width}, {height}, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb); \
         $graphics = [System.Drawing.Graphics]::FromImage($bitmap); \
         $hdc = $graphics.GetHdc(); \
         try {{ \
           if (-not [YggtermPrintWindowCapture]::PrintWindow([IntPtr]::new({hwnd}), $hdc, 2)) {{ \
             throw 'PrintWindow returned false'; \
           }} \
         }} finally {{ \
           $graphics.ReleaseHdc($hdc); \
         }}; \
         try {{ \
           $bitmap.Save('{output_literal}', [System.Drawing.Imaging.ImageFormat]::Png); \
         }} finally {{ \
           $graphics.Dispose(); \
           $bitmap.Dispose(); \
         }}"
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running Windows PrintWindow screenshot capture")?;
    if !status.success() {
        bail!("powershell PrintWindow screenshot capture exited with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_window_number(ns_window_ptr: *mut std::ffi::c_void) -> Result<i64> {
    if ns_window_ptr.is_null() {
        bail!("tao returned a null NSWindow pointer");
    }
    let ns_window = ns_window_ptr.cast::<AnyObject>();
    let window_number: i64 = unsafe { msg_send![ns_window, windowNumber] };
    if window_number <= 0 {
        bail!("invalid NSWindow windowNumber {window_number}");
    }
    Ok(window_number)
}

pub fn host_platform() -> HostPlatform {
    match std::env::consts::OS {
        "linux" => HostPlatform::Linux,
        "macos" => HostPlatform::MacOS,
        "windows" => HostPlatform::Windows,
        _ => HostPlatform::Unknown,
    }
}

pub fn controlled_ghostty_status() -> ControlledGhosttyStatus {
    if !cfg!(target_os = "linux") {
        return ControlledGhosttyStatus {
            adapter: ControlledGhosttyAdapter::Unsupported,
            available: false,
            detail: "controlled ghostty docking is only implemented on Linux/X11 right now"
                .to_string(),
        };
    }

    let ghostty = command_exists("ghostty");
    let xdotool = command_exists("xdotool");

    match (ghostty, xdotool) {
        (true, true) => ControlledGhosttyStatus {
            adapter: ControlledGhosttyAdapter::LinuxX11Dock,
            available: true,
            detail: "Linux controlled host adapter can dock an undecorated Ghostty window into the yggterm terminal viewport.".to_string(),
        },
        (false, true) => ControlledGhosttyStatus {
            adapter: ControlledGhosttyAdapter::LinuxX11Dock,
            available: false,
            detail: "ghostty is not installed on PATH".to_string(),
        },
        (true, false) => ControlledGhosttyStatus {
            adapter: ControlledGhosttyAdapter::LinuxX11Dock,
            available: false,
            detail: "xdotool is not installed on PATH".to_string(),
        },
        (false, false) => ControlledGhosttyStatus {
            adapter: ControlledGhosttyAdapter::LinuxX11Dock,
            available: false,
            detail: "ghostty and xdotool are not installed on PATH".to_string(),
        },
    }
}

pub fn launch_controlled_ghostty(
    launch_command: &str,
    token: &str,
) -> Result<ControlledGhosttyLaunch> {
    let status = controlled_ghostty_status();
    if !status.available {
        bail!(status.detail);
    }

    let child = Command::new("ghostty")
        .arg("--gtk-single-instance=false")
        .arg(format!("--x11-instance-name={token}"))
        .arg("--window-decoration=none")
        .arg("--gtk-titlebar=false")
        .arg("-e")
        .arg("bash")
        .arg("-lc")
        .arg(launch_command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn controlled Ghostty host")?;

    Ok(ControlledGhosttyLaunch {
        process_id: child.id(),
        lines: vec![
            format!("$ {launch_command}"),
            format!("ghostty pid {}", child.id()),
            format!("controlled host token {token}"),
            "Linux controlled host adapter launched an undecorated Ghostty window for viewport docking."
                .to_string(),
        ],
    })
}

pub fn sync_docked_ghostty_window(
    pid: Option<u32>,
    known_window_id: Option<&str>,
    token: Option<&str>,
    rect: DockRect,
) -> Result<DockedGhosttyWindow> {
    let status = controlled_ghostty_status();
    if !status.available {
        bail!(status.detail);
    }

    let window_id =
        if let Some(window_id) = known_window_id.filter(|value| !value.trim().is_empty()) {
            window_id.to_string()
        } else {
            resolve_x11_window(pid, token)?
        };

    let parent_window = resolve_current_app_window()?;
    run_xdotool(["windowreparent", &window_id, &parent_window])?;
    unmap_sibling_windows(pid, &window_id)?;
    run_xdotool([
        "windowsize",
        "--sync",
        &window_id,
        &rect.width.to_string(),
        &rect.height.to_string(),
    ])?;
    run_xdotool([
        "windowmove",
        "--sync",
        &window_id,
        &rect.x.to_string(),
        &rect.y.to_string(),
    ])?;
    run_xdotool(["windowmap", &window_id])?;
    focus_docked_ghostty_window(&window_id)?;

    Ok(DockedGhosttyWindow { window_id })
}

pub fn hide_docked_ghostty_window(pid: Option<u32>, window_id: &str) -> Result<()> {
    let status = controlled_ghostty_status();
    if !status.available {
        bail!(status.detail);
    }
    run_xdotool(["windowunmap", window_id])?;
    unmap_sibling_windows(pid, window_id)
}

pub fn focus_docked_ghostty_window(window_id: &str) -> Result<()> {
    let status = controlled_ghostty_status();
    if !status.available {
        bail!(status.detail);
    }
    run_xdotool(["windowraise", window_id])?;
    if run_xdotool(["windowactivate", "--sync", window_id]).is_ok() {
        return Ok(());
    }
    run_xdotool(["windowfocus", window_id])
}

fn resolve_current_app_window() -> Result<String> {
    xdotool_search([
        "search",
        "--onlyvisible",
        "--pid",
        &std::process::id().to_string(),
    ])
}

fn resolve_x11_window(pid: Option<u32>, token: Option<&str>) -> Result<String> {
    if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
        if let Ok(window_id) = xdotool_search(["search", "--class", token]) {
            return Ok(window_id);
        }
        if let Ok(window_id) = xdotool_search(["search", "--name", token]) {
            return Ok(window_id);
        }
    }

    if let Some(pid) = pid {
        if let Ok(window_id) = xdotool_search(["search", "--pid", &pid.to_string()]) {
            return Ok(window_id);
        }
    }

    bail!("ghostty window not found yet")
}

fn resolve_x11_windows_for_pid(pid: u32) -> Result<Vec<String>> {
    let output = Command::new("xdotool")
        .args(["search", "--pid", &pid.to_string()])
        .output()
        .context("failed to run xdotool search")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(if stderr.is_empty() {
            "xdotool search returned no window".to_string()
        } else {
            stderr
        });
    }

    let windows = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();
    if windows.is_empty() {
        bail!("xdotool search returned no window id");
    }
    Ok(windows)
}

fn unmap_sibling_windows(pid: Option<u32>, keep_window_id: &str) -> Result<()> {
    let Some(pid) = pid else {
        return Ok(());
    };
    let Ok(window_ids) = resolve_x11_windows_for_pid(pid) else {
        return Ok(());
    };
    for window_id in window_ids {
        if window_id != keep_window_id {
            let _ = run_xdotool(["windowunmap", &window_id]);
        }
    }
    Ok(())
}

fn xdotool_search<const N: usize>(args: [&str; N]) -> Result<String> {
    let output = Command::new("xdotool")
        .args(args)
        .output()
        .context("failed to run xdotool search")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(if stderr.is_empty() {
            "xdotool search returned no window".to_string()
        } else {
            stderr
        });
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .context("xdotool search returned no window id")
}

fn run_xdotool<const N: usize>(args: [&str; N]) -> Result<()> {
    let output = Command::new("xdotool")
        .args(args)
        .output()
        .context("failed to run xdotool command")?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(if stderr.is_empty() {
            "xdotool command failed".to_string()
        } else {
            stderr
        });
    }
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()
        .is_ok_and(|status| status.success())
}
