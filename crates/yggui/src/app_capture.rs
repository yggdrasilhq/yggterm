use anyhow::{Context, Result};
use dioxus::desktop::DesktopContext;
use serde_json::{Value, json};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
#[cfg(target_os = "linux")]
use wry::WebViewExtUnix;
#[cfg(target_os = "macos")]
use tao::platform::macos::WindowExtMacOS;
#[cfg(target_os = "macos")]
use yggterm_platform::{capture_macos_window_recording, capture_macos_window_screenshot};

pub async fn capture_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
) -> Result<PathBuf> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating screenshot dir {}", parent.display()))?;
    }
    platform_capture_visible_app_surface(desktop, output_path).await?;
    let metadata = fs::metadata(output_path)
        .with_context(|| format!("reading screenshot metadata {}", output_path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        anyhow::bail!("native screenshot capture produced no file output");
    }
    Ok(output_path.to_path_buf())
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
    desktop.set_visible(true);
    desktop.set_minimized(false);
    desktop.set_focus();
    Ok(json!({
        "focused": true,
        "window": describe_window(desktop),
    }))
}

pub fn describe_window(desktop: &DesktopContext) -> Value {
    let inner = desktop.inner_size();
    let outer = desktop.outer_size();
    let position = desktop.outer_position().ok();
    json!({
        "title": desktop.title(),
        "visible": desktop.is_visible(),
        "maximized": desktop.is_maximized(),
        "decorated": desktop.is_decorated(),
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

#[cfg(target_os = "linux")]
async fn platform_capture_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
) -> Result<()> {
    let gtk_webview = desktop.webview.webview();
    let surface = gtk_webview
        .snapshot_future(SnapshotRegion::Visible, SnapshotOptions::NONE)
        .await
        .context("capturing visible app surface from WebKitGTK")?;
    let mut output = File::create(output_path)
        .with_context(|| format!("creating screenshot file {}", output_path.display()))?;
    surface
        .write_to_png(&mut output)
        .with_context(|| format!("writing screenshot png {}", output_path.display()))?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn platform_capture_visible_app_surface(
    desktop: &DesktopContext,
    output_path: &Path,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return capture_macos_window_screenshot(desktop.window.ns_window().cast(), output_path);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = desktop;
        let _ = output_path;
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
