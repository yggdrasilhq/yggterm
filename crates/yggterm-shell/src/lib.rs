#![recursion_limit = "512"]

mod agent_input_arbiter;
// The SHELL-side arm matrix is a TEST artifact (harness spec §8 phase 2b), the
// twin of `yggterm_server::agent_arm_matrix`: it locks the GUI-side per-arm
// decisions — readiness/overlay (§7.3), attach seed (§7.6), mount identity
// (§7.10) — against one table, so a change that touches one arm cannot pass
// while quietly changing another.
#[cfg(test)]
mod agent_arm_shell_matrix;
mod app_capture;
mod command_registry;
// The ONE client-side owner of "a daemon handover is in progress, stop painting"
// (user-settled call #7). Pure decision + fail-safes; `shell.rs` feeds it the
// daemon's own status and honours its verdict.
mod handover_gate;
mod hot_update_policy;
// The ALT+ KeyTips declaration model + assignment resolver (docs/alt-keytips.md).
// Pure logic, unit-tested in isolation; the shell renders and drives it. Marked
// allow(dead_code) while the render/chord integration is wired in incrementally.
#[allow(dead_code)]
mod keytip;
mod netscape_cookie_jar;
// The ONE owner of "how long may the remote-resume readiness gate keep the
// user's terminal blank and un-typeable". The 60 s failure timer is armed per
// BOOTSTRAP identity; the gate is re-armed per RECOVERY, so every re-arm after
// the first used to be uncapped. This ceiling follows the gate.
mod resume_gate;
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
// Find-in-page: the option mask, the match cap, the position cycle and the
// keyboard-ownership contract shared by the Ctrl+F bar, the `web find` verb and
// the engine bridge in `vendor/dioxus-desktop/src/web_surface.rs`.
mod web_find;
mod window_icon;
mod xterm_gate_metrics;

pub use shell::{
    PendingUpdateRestart, ShellBootstrap, initial_server_sync, launch_shell, start_daemon_watchdog,
    terminal_identity_appearance_for_settings, warm_daemon_start,
};
