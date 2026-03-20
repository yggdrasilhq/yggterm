#![allow(non_camel_case_types)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GhosttyBridgeError {
    #[error("ghostty FFI is not enabled in this build")]
    FfiDisabled,
    #[error("embedded ghostty surfaces are not available on this platform")]
    EmbeddedSurfaceUnsupported,
}

pub type Result<T> = std::result::Result<T, GhosttyBridgeError>;

#[derive(Debug, Clone)]
pub struct GhosttyEnvironment {
    pub header_path: Option<String>,
    pub lib_dir: Option<String>,
}

impl GhosttyEnvironment {
    pub fn discover() -> Self {
        let header_path = option_env!("YGGTERM_GHOSTTY_HEADER")
            .map(ToOwned::to_owned)
            .filter(|path| std::path::Path::new(path).exists());
        let lib_dir = option_env!("YGGTERM_GHOSTTY_LIB_DIR")
            .map(ToOwned::to_owned)
            .filter(|path| std::path::Path::new(path).exists());
        Self {
            header_path,
            lib_dir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhosttySurfaceEmbedding {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhosttyEmbeddedRuntime {
    MacosLibghostty,
}

#[derive(Debug, Clone)]
pub struct GhosttyEmbeddedSurfaceReservation {
    pub surface_id: String,
    pub runtime: GhosttyEmbeddedRuntime,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct GhosttyBridgeStatus {
    pub ffi_enabled: bool,
    pub embedded_surface_support: GhosttySurfaceEmbedding,
    pub detail: String,
}

impl GhosttyBridgeStatus {
    pub fn linked_runtime_available(&self) -> bool {
        self.ffi_enabled
    }

    pub fn embedded_surface_available(&self) -> bool {
        self.embedded_surface_support == GhosttySurfaceEmbedding::Supported
    }
}

pub fn bridge_status() -> GhosttyBridgeStatus {
    #[cfg(feature = "ghostty-ffi")]
    {
        GhosttyBridgeStatus {
            ffi_enabled: true,
            embedded_surface_support: embedded_surface_support(),
            detail: embedding_detail().to_string(),
        }
    }

    #[cfg(not(feature = "ghostty-ffi"))]
    {
        GhosttyBridgeStatus {
            ffi_enabled: false,
            embedded_surface_support: GhosttySurfaceEmbedding::Unsupported,
            detail: "libghostty FFI is disabled in this build".to_string(),
        }
    }
}

pub fn initialize_bridge() -> Result<()> {
    if bridge_status().ffi_enabled {
        Ok(())
    } else {
        Err(GhosttyBridgeError::FfiDisabled)
    }
}

pub fn reserve_embedded_surface(
    surface_id_hint: Option<&str>,
) -> Result<GhosttyEmbeddedSurfaceReservation> {
    if !bridge_status().ffi_enabled {
        return Err(GhosttyBridgeError::FfiDisabled);
    }

    #[cfg(target_os = "macos")]
    {
        use uuid::Uuid;

        let surface_id = surface_id_hint
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("yggterm-surface-{}", Uuid::new_v4().simple()));
        return Ok(GhosttyEmbeddedSurfaceReservation {
            surface_id,
            runtime: GhosttyEmbeddedRuntime::MacosLibghostty,
            detail: "Reserved a libghostty surface identity for the macOS embedded host path."
                .to_string(),
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = surface_id_hint;
        Err(GhosttyBridgeError::EmbeddedSurfaceUnsupported)
    }
}

#[cfg(feature = "ghostty-ffi")]
fn embedded_surface_support() -> GhosttySurfaceEmbedding {
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        GhosttySurfaceEmbedding::Supported
    } else {
        GhosttySurfaceEmbedding::Unsupported
    }
}

#[cfg(feature = "ghostty-ffi")]
fn embedding_detail() -> &'static str {
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        "libghostty is linked and the upstream embedded surface host is available on this platform"
    } else if cfg!(target_os = "linux") {
        "libghostty is linked, but Ghostty's current embedded surface host only exposes macOS/iOS platform views; Linux uses the external Ghostty process fallback for now"
    } else {
        "libghostty is linked, but this platform does not currently expose an upstream embedded surface host"
    }
}

#[cfg(feature = "ghostty-ffi")]
pub mod ffi {
    use std::ffi::c_void;

    pub type ghostty_app_t = *mut c_void;
    pub type ghostty_config_t = *mut c_void;
    pub type ghostty_surface_t = *mut c_void;

    #[link(name = "ghostty")]
    unsafe extern "C" {
        pub fn ghostty_app_tick(app: ghostty_app_t);
    }
}

#[cfg(feature = "ghostty-ffi")]
pub struct GhosttyApp {
    raw: ffi::ghostty_app_t,
}

#[cfg(feature = "ghostty-ffi")]
impl GhosttyApp {
    pub fn from_raw(raw: ffi::ghostty_app_t) -> Self {
        Self { raw }
    }

    pub fn tick(&self) {
        unsafe {
            ffi::ghostty_app_tick(self.raw);
        }
    }

    pub fn raw(&self) -> ffi::ghostty_app_t {
        self.raw
    }
}
