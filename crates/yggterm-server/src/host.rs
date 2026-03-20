use std::process::{Command, Stdio};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct GhosttyLaunchOutcome {
    pub process_id: Option<u32>,
    pub embedded_surface_reserved: bool,
    pub host_token: Option<String>,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhosttyHostKind {
    MacosLibghostty,
    LinuxControlledDock,
    ExternalGhostty,
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct GhosttyHostSupport {
    pub kind: GhosttyHostKind,
    pub detail: String,
    pub embedded_surface_supported: bool,
    pub bridge_enabled: bool,
}

impl GhosttyHostKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GhosttyHostKind::MacosLibghostty => "macos-libghostty",
            GhosttyHostKind::LinuxControlledDock => "linux-controlled-dock",
            GhosttyHostKind::ExternalGhostty => "external-ghostty",
            GhosttyHostKind::Unsupported => "unsupported",
        }
    }
}

impl GhosttyHostSupport {
    pub fn shadow(
        detail: String,
        embedded_surface_supported: bool,
        bridge_enabled: bool,
    ) -> Self {
        let kind = if cfg!(target_os = "macos") && embedded_surface_supported {
            GhosttyHostKind::MacosLibghostty
        } else if cfg!(target_os = "linux") {
            GhosttyHostKind::LinuxControlledDock
        } else if bridge_enabled {
            GhosttyHostKind::ExternalGhostty
        } else {
            GhosttyHostKind::Unsupported
        };

        Self {
            kind,
            detail,
            embedded_surface_supported,
            bridge_enabled,
        }
    }

    pub fn launch_terminal(&self, launch_command: &str) -> Result<GhosttyLaunchOutcome, String> {
        match self.kind {
            GhosttyHostKind::MacosLibghostty => Ok(GhosttyLaunchOutcome {
                process_id: None,
                embedded_surface_reserved: true,
                host_token: None,
                lines: vec![
                    format!("$ {}", launch_command),
                    "libghostty embedded host reserved on macOS".to_string(),
                    "The daemon will hand this session to the in-viewport macOS host path.".to_string(),
                ],
            }),
            GhosttyHostKind::LinuxControlledDock => {
                let token = format!("yggterm-{}", Uuid::new_v4().simple());
                let launch = yggterm_platform::launch_controlled_ghostty(launch_command, &token)
                    .map_err(|error| error.to_string())?;
                Ok(GhosttyLaunchOutcome {
                    process_id: Some(launch.process_id),
                    embedded_surface_reserved: false,
                    host_token: Some(token),
                    lines: launch.lines,
                })
            }
            GhosttyHostKind::ExternalGhostty => {
                let child = Command::new("ghostty")
                    .arg("--gtk-single-instance=false")
                    .arg("-e")
                    .arg("bash")
                    .arg("-lc")
                    .arg(launch_command)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("failed to spawn ghostty: {error}"))?;
                Ok(GhosttyLaunchOutcome {
                    process_id: Some(child.id()),
                    embedded_surface_reserved: false,
                    host_token: None,
                    lines: vec![
                        format!("$ {}", launch_command),
                        format!("ghostty pid {}", child.id()),
                        "external Ghostty host launched a terminal window".to_string(),
                    ],
                })
            }
            GhosttyHostKind::Unsupported => Err(self.detail.clone()),
        }
    }

    pub fn integration_note(&self) -> String {
        match self.kind {
            GhosttyHostKind::MacosLibghostty => {
                "macOS keeps the libghostty embedded host path as the primary integration target.".to_string()
            }
            GhosttyHostKind::LinuxControlledDock => {
                "Linux uses a controlled X11 docking adapter today while upstream embedded Ghostty hosting remains macOS/iOS-first.".to_string()
            }
            GhosttyHostKind::ExternalGhostty => {
                "This platform currently falls back to an external Ghostty process host.".to_string()
            }
            GhosttyHostKind::Unsupported => self.detail.clone(),
        }
    }
}

pub fn detect_ghostty_host() -> GhosttyHostSupport {
    let bridge = yggterm_ghostty_bridge::bridge_status();
    let gtk_glue = yggterm_gtk_glue::status();
    let bridge_enabled = bridge.linked_runtime_available();
    let embedded_surface_supported = bridge.embedded_surface_available();

    if cfg!(target_os = "macos") && embedded_surface_supported {
        GhosttyHostSupport {
            kind: GhosttyHostKind::MacosLibghostty,
            detail: bridge.detail,
            embedded_surface_supported: true,
            bridge_enabled,
        }
    } else if cfg!(target_os = "linux")
        && gtk_glue == yggterm_gtk_glue::GtkGlueStatus::Available
        && yggterm_platform::controlled_ghostty_status().available
    {
        let platform = yggterm_platform::controlled_ghostty_status();
        GhosttyHostSupport {
            kind: GhosttyHostKind::LinuxControlledDock,
            detail: format!(
                "{} {} {}",
                bridge.detail,
                yggterm_gtk_glue::detail(),
                platform.detail
            ),
            embedded_surface_supported: false,
            bridge_enabled,
        }
    } else if bridge_enabled {
        GhosttyHostSupport {
            kind: GhosttyHostKind::ExternalGhostty,
            detail: bridge.detail,
            embedded_surface_supported,
            bridge_enabled: true,
        }
    } else {
        GhosttyHostSupport {
            kind: GhosttyHostKind::Unsupported,
            detail: bridge.detail,
            embedded_surface_supported: false,
            bridge_enabled: false,
        }
    }
}
