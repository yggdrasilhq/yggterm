#![recursion_limit = "512"]

mod app_capture;
mod shell;
mod terminal_observe;
mod terminal_protocol;
mod terminal_themes;
mod window_icon;

pub use shell::{
    PendingUpdateRestart, ShellBootstrap, initial_server_sync, launch_shell, start_daemon_watchdog,
    terminal_identity_appearance_for_settings, warm_daemon_start,
};
