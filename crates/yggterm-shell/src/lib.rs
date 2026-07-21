#![recursion_limit = "512"]

mod agent_input_arbiter;
mod app_capture;
mod command_registry;
mod hot_update_policy;
// The ALT+ KeyTips declaration model + assignment resolver (docs/alt-keytips.md).
// Pure logic, unit-tested in isolation; the shell renders and drives it. Marked
// allow(dead_code) while the render/chord integration is wired in incrementally.
#[allow(dead_code)]
mod keytip;
// Phase 1 of the consolidated scroll-controller: the canonical, regression-locked
// DECISION spec (mode + transitions). The JS wiring (Phase 2) mirrors it. Marked
// allow(dead_code) until the JS migration consults it. See scroll_mode.rs.
#[allow(dead_code)]
mod scroll_mode;
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
mod xterm_gate_metrics;

pub use shell::{
    PendingUpdateRestart, ShellBootstrap, initial_server_sync, launch_shell, start_daemon_watchdog,
    terminal_identity_appearance_for_settings, warm_daemon_start,
};
