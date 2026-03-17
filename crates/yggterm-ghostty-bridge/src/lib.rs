#![allow(non_camel_case_types)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GhosttyBridgeError {
    #[error("ghostty FFI is not enabled in this build")]
    FfiDisabled,
    #[error("ghostty app initialization returned null")]
    AppInitFailed,
}

pub type Result<T> = std::result::Result<T, GhosttyBridgeError>;

#[derive(Debug, Clone)]
pub struct GhosttyEnvironment {
    pub header_path: Option<String>,
}

impl GhosttyEnvironment {
    pub fn discover() -> Self {
        let header_path = option_env!("YGGTERM_GHOSTTY_HEADER")
            .map(ToOwned::to_owned)
            .filter(|path| std::path::Path::new(path).exists());
        Self { header_path }
    }
}

pub fn initialize_bridge() -> Result<()> {
    #[cfg(feature = "ghostty-ffi")]
    {
        Ok(())
    }

    #[cfg(not(feature = "ghostty-ffi"))]
    {
        Err(GhosttyBridgeError::FfiDisabled)
    }
}

#[cfg(feature = "ghostty-ffi")]
pub mod ffi {
    use std::ffi::c_void;

    pub type ghostty_app_t = *mut c_void;
    pub type ghostty_config_t = *mut c_void;
    pub type ghostty_surface_t = *mut c_void;

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct ghostty_runtime_config_s {
        _private: [u8; 0],
    }

    #[link(name = "ghostty")]
    unsafe extern "C" {
        pub fn ghostty_app_new(
            runtime: *const ghostty_runtime_config_s,
            config: ghostty_config_t,
        ) -> ghostty_app_t;
        pub fn ghostty_app_free(app: ghostty_app_t);
        pub fn ghostty_app_tick(app: ghostty_app_t);

        pub fn ghostty_surface_new(
            app: ghostty_app_t,
            surface_config: *const c_void,
        ) -> ghostty_surface_t;
        pub fn ghostty_surface_free(surface: ghostty_surface_t);
        pub fn ghostty_surface_draw(surface: ghostty_surface_t);
    }
}

#[cfg(feature = "ghostty-ffi")]
pub struct GhosttyApp {
    raw: ffi::ghostty_app_t,
}

#[cfg(feature = "ghostty-ffi")]
impl GhosttyApp {
    pub fn from_raw(raw: ffi::ghostty_app_t) -> Result<Self> {
        if raw.is_null() {
            return Err(GhosttyBridgeError::AppInitFailed);
        }
        Ok(Self { raw })
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

#[cfg(feature = "ghostty-ffi")]
impl Drop for GhosttyApp {
    fn drop(&mut self) {
        unsafe {
            ffi::ghostty_app_free(self.raw);
        }
    }
}
