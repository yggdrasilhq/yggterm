#![recursion_limit = "512"]

mod app_capture;
mod hot_update_policy;
mod session_copy_policy;
mod shell;
mod terminal_observe;
mod terminal_protocol;
mod terminal_retained_replay_policy;
mod terminal_themes;
mod terminal_write_bridge;
mod terminal_write_policy;
mod theme_contract;
mod ui_telemetry;
mod window_icon;

pub use shell::{
    PendingUpdateRestart, ShellBootstrap, initial_server_sync, launch_shell, start_daemon_watchdog,
    terminal_identity_appearance_for_settings, warm_daemon_start,
};
