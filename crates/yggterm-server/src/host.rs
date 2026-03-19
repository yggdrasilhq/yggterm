use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct GhosttyLaunchOutcome {
    pub process_id: Option<u32>,
    pub embedded_surface_reserved: bool,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhosttyHostKind {
    MacosLibghostty,
    LinuxGtkGlue,
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
            GhosttyHostKind::LinuxGtkGlue => "linux-gtk-glue",
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
            GhosttyHostKind::LinuxGtkGlue
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
                lines: vec![
                    format!("$ {}", launch_command),
                    "libghostty embedded host reserved on macOS".to_string(),
                    "The daemon will hand this session to the in-viewport macOS host path.".to_string(),
                ],
            }),
            GhosttyHostKind::LinuxGtkGlue | GhosttyHostKind::ExternalGhostty => {
                let child = Command::new("ghostty")
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
                    lines: vec![
                        format!("$ {}", launch_command),
                        format!("ghostty pid {}", child.id()),
                        match self.kind {
                            GhosttyHostKind::LinuxGtkGlue => {
                                "linux gtk host path launched an external Ghostty window".to_string()
                            }
                            _ => "external Ghostty host launched a terminal window".to_string(),
                        },
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
            GhosttyHostKind::LinuxGtkGlue => {
                "Linux uses the GTK host adapter today while upstream embedded Ghostty hosting remains macOS/iOS-first.".to_string()
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
    {
        GhosttyHostSupport {
            kind: GhosttyHostKind::LinuxGtkGlue,
            detail: format!("{} {}", bridge.detail, yggterm_gtk_glue::detail()),
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
