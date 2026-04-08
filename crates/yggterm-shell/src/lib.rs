#![recursion_limit = "256"]

mod app_capture;
mod shell;
mod terminal_observe;
mod terminal_protocol;
mod terminal_themes;
mod window_icon;

pub use shell::{
    PendingUpdateRestart, ShellBootstrap, initial_server_sync, launch_shell, warm_daemon_start,
};
