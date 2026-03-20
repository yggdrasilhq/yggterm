use anyhow::{Context, Result, bail};
use std::process::{Command, Stdio};

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

pub fn launch_controlled_ghostty(launch_command: &str, token: &str) -> Result<ControlledGhosttyLaunch> {
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

    let window_id = if let Some(window_id) = known_window_id.filter(|value| !value.trim().is_empty()) {
        window_id.to_string()
    } else {
        resolve_x11_window(pid, token)?
    };

    run_xdotool(["windowmap", &window_id])?;
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
    focus_docked_ghostty_window(&window_id)?;

    Ok(DockedGhosttyWindow { window_id })
}

pub fn hide_docked_ghostty_window(window_id: &str) -> Result<()> {
    let status = controlled_ghostty_status();
    if !status.available {
        bail!(status.detail);
    }
    run_xdotool(["windowunmap", window_id])
}

pub fn focus_docked_ghostty_window(window_id: &str) -> Result<()> {
    let status = controlled_ghostty_status();
    if !status.available {
        bail!(status.detail);
    }
    run_xdotool(["windowraise", window_id])?;
    run_xdotool(["windowactivate", "--sync", window_id])
}

fn resolve_x11_window(pid: Option<u32>, token: Option<&str>) -> Result<String> {
    if let Some(pid) = pid {
        if let Ok(window_id) = xdotool_search(["search", "--pid", &pid.to_string()]) {
            return Ok(window_id);
        }
    }

    if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
        if let Ok(window_id) = xdotool_search(["search", "--class", token]) {
            return Ok(window_id);
        }
        if let Ok(window_id) = xdotool_search(["search", "--name", token]) {
            return Ok(window_id);
        }
    }

    bail!("ghostty window not found yet")
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
