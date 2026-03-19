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
