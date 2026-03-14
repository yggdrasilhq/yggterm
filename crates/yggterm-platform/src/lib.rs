#[derive(Debug, Clone, Copy)]
pub enum HostPlatform {
    Linux,
    MacOS,
    Windows,
    Unknown,
}

pub fn host_platform() -> HostPlatform {
    match std::env::consts::OS {
        "linux" => HostPlatform::Linux,
        "macos" => HostPlatform::MacOS,
        "windows" => HostPlatform::Windows,
        _ => HostPlatform::Unknown,
    }
}
