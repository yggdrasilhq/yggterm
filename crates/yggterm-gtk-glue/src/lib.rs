#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtkGlueStatus {
    Available,
    UnsupportedPlatform,
}

pub fn status() -> GtkGlueStatus {
    if cfg!(target_os = "linux") {
        GtkGlueStatus::Available
    } else {
        GtkGlueStatus::UnsupportedPlatform
    }
}

pub fn detail() -> &'static str {
    match status() {
        GtkGlueStatus::Available => {
            "Linux GTK glue path is reserved as the bypass host for Ghostty embedding work."
        }
        GtkGlueStatus::UnsupportedPlatform => "GTK glue bypass is only planned for Linux builds.",
    }
}
