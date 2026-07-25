//! Bring the window back when it dies — and only when it DIED.
//!
//! The desktop environment starts yggterm as a TRANSIENT systemd unit it
//! authors itself (`app-dev.yggterm.Yggterm@<uuid>.service`, `Restart=no`), so
//! we cannot set a restart policy on it. Measured on the live host: ten genuine
//! core dumps across two weeks, several per week, and after each one the user
//! was simply left without a window while the daemon and every session sat there
//! intact. (The raw "42 failed units" number is misleading — sorted by `Result`,
//! 31 are exit-code failures and ten of THOSE are `status=130`, the process's own
//! handler exiting on SIGINT. Deliberate kills, not crashes.)
//!
//! **The policy that fits is `Restart=on-abnormal`, not `on-failure`.**
//! `on-failure` also fires on a non-zero exit code, which would fight every
//! deliberate shutdown above. Only signal-death comes back.
//!
//! A supervisor shim gets that exactly right where daemon-side supervision
//! cannot: the daemon is not the GUI's parent, so it can only observe "the
//! process is gone" — precisely the distinction that matters here. This mode
//! forks the real GUI as a CHILD and waits on it, so it learns the status.
//!
//! ⚠ The update path must keep reading as a clean exit. An in-place `exec()`
//! keeps the pid, so the supervisor sees nothing at all (correct); a
//! spawn-successor-and-exit handoff exits 0, which is not a signal, so it never
//! restarts. Both are safe by construction under the rule above — but check this
//! again if the handoff ever grows a signal-based path.

use std::time::{Duration, Instant};

/// Flag on the desktop entry's `Exec=` line. Absent everywhere else, so nothing
/// that launches yggterm programmatically gains a supervisor by accident.
pub const SUPERVISE_FLAG: &str = "--supervise";
/// Set on the child so a supervised process can never re-enter supervise mode
/// (belt to the argv strip: a handoff could rebuild argv).
pub const SUPERVISED_ENV: &str = "YGGTERM_SUPERVISED";

/// Signals that mean "this process crashed", as opposed to "something asked it
/// to stop". Nothing else restarts.
const CRASH_SIGNALS: [i32; 3] = [
    libc_sigsegv(),
    libc_sigabrt(),
    libc_sigbus(),
];

const fn libc_sigsegv() -> i32 {
    11
}
const fn libc_sigabrt() -> i32 {
    6
}
const fn libc_sigbus() -> i32 {
    7
}

/// A child that dies faster than this never earned a restart — it is crashing on
/// startup, and restarting it is a loop, not a recovery.
const MIN_HEALTHY_UPTIME_MS: u64 = 10_000;
/// Restart budget. Past this the user is better served by a window that stays
/// gone than by one that flickers.
const MAX_RESTARTS_PER_WINDOW: usize = 5;
const RESTART_WINDOW_MS: u64 = 60 * 60_000;

/// How the supervised child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildOutcome {
    /// Exited on its own, with this code. Includes 130 (its SIGINT handler) and
    /// every deliberate shutdown.
    Exited(i32),
    /// Killed by a signal.
    Signalled(i32),
}

impl ChildOutcome {
    /// The status the supervisor should exit with, so the unit sees what the
    /// child saw. 128+signal is the shell convention.
    pub fn exit_code(self) -> i32 {
        match self {
            ChildOutcome::Exited(code) => code,
            ChildOutcome::Signalled(signal) => 128 + signal,
        }
    }
}

/// `Restart=on-abnormal`, spelled out. Pure, so the policy is the testable part
/// rather than something only a real crash can exercise.
pub fn supervisor_should_restart(
    outcome: ChildOutcome,
    lived_ms: u64,
    restarts_in_window: usize,
) -> bool {
    let ChildOutcome::Signalled(signal) = outcome else {
        // Any exit code at all is a decision the process made. A clean quit
        // stays quit; so does the update handoff.
        return false;
    };
    if !CRASH_SIGNALS.contains(&signal) {
        // SIGTERM/SIGINT/SIGKILL: someone asked for this.
        return false;
    }
    if lived_ms < MIN_HEALTHY_UPTIME_MS {
        return false;
    }
    restarts_in_window < MAX_RESTARTS_PER_WINDOW
}

/// True when this process was asked to supervise (and is not itself supervised).
pub fn should_run_as_supervisor(args: &[String]) -> bool {
    args.iter().any(|arg| arg == SUPERVISE_FLAG)
        && std::env::var(SUPERVISED_ENV).ok().as_deref() != Some("1")
}

/// The child's argv: everything except the supervise flag.
pub fn supervised_child_args(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|arg| arg.as_str() != SUPERVISE_FLAG)
        .cloned()
        .collect()
}

/// Run the GUI as a child, restarting it only on an abnormal death. Returns the
/// exit code the supervisor should exit with.
#[cfg(unix)]
pub fn run_supervisor(args: &[String]) -> anyhow::Result<i32> {
    use anyhow::Context as _;
    use std::os::unix::process::ExitStatusExt as _;

    let current_exe = std::env::current_exe().context("resolving the supervised executable")?;
    let child_args = supervised_child_args(args);
    let mut restart_times: Vec<Instant> = Vec::new();
    loop {
        let started = Instant::now();
        let mut child = std::process::Command::new(&current_exe)
            .args(&child_args)
            .env(SUPERVISED_ENV, "1")
            .spawn()
            .with_context(|| format!("launching {}", current_exe.display()))?;
        let status = child.wait().context("waiting for the supervised window")?;
        let lived_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let outcome = match status.signal() {
            Some(signal) => ChildOutcome::Signalled(signal),
            None => ChildOutcome::Exited(status.code().unwrap_or(0)),
        };
        let window = Duration::from_millis(RESTART_WINDOW_MS);
        restart_times.retain(|at| at.elapsed() < window);
        if !supervisor_should_restart(outcome, lived_ms, restart_times.len()) {
            return Ok(outcome.exit_code());
        }
        restart_times.push(Instant::now());
        eprintln!(
            "yggterm: window died on signal {:?} after {}ms — restarting ({}/{} this hour)",
            outcome,
            lived_ms,
            restart_times.len(),
            MAX_RESTARTS_PER_WINDOW
        );
    }
}

#[cfg(not(unix))]
pub fn run_supervisor(_args: &[String]) -> anyhow::Result<i32> {
    anyhow::bail!("--supervise is a Unix-only mode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_comes_back_and_a_clean_quit_stays_quit() {
        assert!(supervisor_should_restart(
            ChildOutcome::Signalled(11),
            60_000,
            0
        ));
        assert!(supervisor_should_restart(
            ChildOutcome::Signalled(6),
            60_000,
            0
        ));
        // The measured reality on the live host: ten of the "failures" were the
        // process's OWN handler exiting on SIGINT. `on-failure` would have
        // fought every one of them.
        assert!(!supervisor_should_restart(
            ChildOutcome::Exited(130),
            60_000,
            0
        ));
        assert!(!supervisor_should_restart(
            ChildOutcome::Exited(1),
            60_000,
            0
        ));
        // A successor-spawning update handoff exits 0 — never a restart, or
        // every update would leave two windows.
        assert!(!supervisor_should_restart(
            ChildOutcome::Exited(0),
            60_000,
            0
        ));
    }

    #[test]
    fn a_requested_death_is_not_a_crash() {
        for signal in [15 /* TERM */, 2 /* INT */, 9 /* KILL */, 1 /* HUP */] {
            assert!(
                !supervisor_should_restart(ChildOutcome::Signalled(signal), 60_000, 0),
                "signal {signal} is a request, not a crash"
            );
        }
    }

    #[test]
    fn a_crash_on_startup_is_a_loop_not_a_recovery() {
        assert!(!supervisor_should_restart(
            ChildOutcome::Signalled(11),
            MIN_HEALTHY_UPTIME_MS - 1,
            0
        ));
        assert!(supervisor_should_restart(
            ChildOutcome::Signalled(11),
            MIN_HEALTHY_UPTIME_MS,
            0
        ));
    }

    #[test]
    fn the_restart_budget_is_finite() {
        assert!(supervisor_should_restart(
            ChildOutcome::Signalled(11),
            60_000,
            MAX_RESTARTS_PER_WINDOW - 1
        ));
        assert!(!supervisor_should_restart(
            ChildOutcome::Signalled(11),
            60_000,
            MAX_RESTARTS_PER_WINDOW
        ));
    }

    #[test]
    fn the_supervisor_exit_status_mirrors_the_child() {
        assert_eq!(ChildOutcome::Exited(130).exit_code(), 130);
        assert_eq!(ChildOutcome::Exited(0).exit_code(), 0);
        assert_eq!(ChildOutcome::Signalled(11).exit_code(), 139);
    }

    #[test]
    fn the_child_never_re_enters_supervise_mode() {
        let args = vec![SUPERVISE_FLAG.to_string(), "--client-role".to_string()];
        assert_eq!(supervised_child_args(&args), vec!["--client-role"]);
        assert!(should_run_as_supervisor(&args));
        // Even if a handoff rebuilds argv with the flag, the env marker stops it.
        unsafe { std::env::set_var(SUPERVISED_ENV, "1") };
        assert!(!should_run_as_supervisor(&args));
        unsafe { std::env::remove_var(SUPERVISED_ENV) };
    }
}
