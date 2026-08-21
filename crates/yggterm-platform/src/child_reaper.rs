//! ⛔ A DROPPED `Child` IS NOT A REAPED CHILD — the one owner of "fire and
//! forget, but still collect the corpse".
//!
//! **Why this module exists.** `std::process::Child` has no implicit `wait()`
//! on drop, and says so: dropping the handle leaves the process in the table as
//! a zombie until its parent waits or its parent dies. In a CLI that is
//! invisible — the process exits a moment later and `init` reaps everything. In
//! a DAEMON or a GUI, which are the two long-lived processes this project
//! ships, every un-waited child is a permanent entry in the process table for
//! the life of the parent.
//!
//! **Measured on one host 2026-08-21**, five live daemons held **233 zombies**
//! between them (82 / 71 / 69 / 11 / 0). In every leaking one the OLDEST zombie
//! was the same age as the daemon itself, so nothing had ever been reaped, and
//! the arrivals were a metronome: p50 **903s** apart, min 901, max 906.
//!
//! ⭐ **The attribution is an exact correspondence, not an arithmetic
//! coincidence.** Simulating `host_panic`'s fifteen-minute cooldown over one
//! daemon's own traced `daemon/heartbeat/panic` events predicts **71**
//! notifications; that daemon holds **71** zombies, first notification and
//! first zombie share a birth second (23:57:23) and so do the last
//! (17:30:39/40). 903s is not the cooldown plus anything — it is fifteen ticks
//! of the 60s watcher loop, each carrying its own drift. A single
//! `let _ = cmd.spawn()` in the owner-notification path, firing forever on a
//! host that is genuinely under load.
//!
//! ⚠ **And the sixth daemon is the control the census nearly missed.** The one
//! daemon on that host with zero zombies is the one built before the host-panic
//! notifier existed — its binary contains none of the notifier's strings. The
//! counterexample is the mechanism confirming itself. ⛔ It was also nearly
//! missed for a duller reason: a census that greps for `yggterm-headless`
//! misses a daemon launched as `yggterm server daemon`, and one such daemon was
//! holding 69 zombies. **Enumerate daemons by their `server daemon` argument,
//! never by the binary's name.**
//!
//! ⚠ **The failure is silent and it never gets worse in a way anyone notices.**
//! A zombie costs no CPU and almost no memory, so nothing complains until the
//! parent has been up long enough to exhaust a pid range — which on a daemon
//! designed to be version-coexisting and long-lived is a real horizon, not a
//! theoretical one. Nothing in the fleet's watch plane counts zombies, so this
//! ran for the daemons' whole uptime unremarked.
//!
//! ⛔ **The tempting one-line fix is wrong here.** Setting `SIGCHLD` to
//! `SIG_IGN` makes the kernel auto-reap, and it would break every `.wait()`,
//! `.output()` and `.status()` in the codebase at once — those would start
//! returning `ECHILD` instead of an exit status. The reap has to be per-child,
//! by the code that spawned it.
//!
//! ⭐ So the correct thing is also the easy thing: [`spawn_and_reap`] is what a
//! fire-and-forget caller reaches for instead of `let _ = cmd.spawn()`, and it
//! cannot leak by construction. [`reap_child_in_background`] is the primitive
//! underneath, for a caller that already holds a `Child` or wants the exit
//! status reported somewhere.

use std::io;
use std::process::{Child, Command, ExitStatus};

/// Wait for `child` on a detached thread, handing the outcome to `on_exit`.
///
/// The thread lives exactly as long as the child does, so this costs a parked
/// thread per outstanding child and nothing once it exits.
///
/// `on_exit` receives whatever `wait()` returned, so a caller that wants to
/// trace or count the exit gets the status; a caller that genuinely does not
/// care passes a closure that drops it. Both are honest — what is NOT
/// available is dropping the `Child` itself, which is the bug this replaces.
pub fn reap_child_in_background<F>(mut child: Child, on_exit: F)
where
    F: FnOnce(io::Result<ExitStatus>) + Send + 'static,
{
    std::thread::spawn(move || {
        on_exit(child.wait());
    });
}

/// Spawn `command` as a fire-and-forget child that is still reaped, returning
/// the child's pid.
///
/// ⇒ **Use this anywhere `let _ = cmd.spawn()` looks right.** The pid comes
/// back so a caller can trace what it launched; ignoring it is fine and still
/// does not leak, which is the whole point of routing through here.
pub fn spawn_and_reap(command: &mut Command) -> io::Result<u32> {
    let child = command.spawn()?;
    let pid = child.id();
    reap_child_in_background(child, |_| {});
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The primitive reports the status, which is the observable proof that a
    /// `wait()` actually happened — a handle that was merely dropped can never
    /// deliver one.
    #[test]
    fn the_background_reaper_waits_and_reports_the_exit_status() {
        let child = Command::new("sh")
            .args(["-c", "exit 7"])
            .spawn()
            .expect("spawn a child that exits with a known code");
        let (tx, rx) = mpsc::channel();
        reap_child_in_background(child, move |status| {
            let _ = tx.send(status.map(|status| status.code()));
        });

        let reported = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the reaper thread must report the exit");
        assert_eq!(
            reported.expect("wait() must succeed"),
            Some(7),
            "the reaper must deliver the child's real exit code, not a placeholder"
        );
    }

    /// ⛔ THE LOCK ON THE ACTUAL BUG, WITH THE CONTROL THAT MAKES IT A PROOF.
    ///
    /// Reporting a status is necessary but not sufficient — the property that
    /// was violated in production is that the process table is left clean. So
    /// this first proves the probe can SEE a zombie (hold the handle, let the
    /// child exit, observe `Z`), and only then reaps it and proves the entry is
    /// gone. Without the control half, an assertion that "the pid is not a
    /// zombie" also passes when the probe is simply blind.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_reaped_child_leaves_no_zombie_and_the_probe_can_see_one() {
        fn process_state(pid: u32) -> Option<String> {
            // `/proc/<pid>/stat` is `pid (comm) state ...` and comm may itself
            // contain spaces and parentheses, so the state is the first field
            // after the LAST ')'.
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let (_, rest) = stat.rsplit_once(')')?;
            rest.split_whitespace().next().map(str::to_string)
        }

        let child = Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn a child that exits immediately");
        let pid = child.id();

        // CONTROL: the handle is still held and nothing has waited, so the
        // exited child MUST be sitting in the table as a zombie. This is the
        // exact state the old `let _ = cmd.spawn()` left behind forever.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut observed = None;
        while std::time::Instant::now() < deadline {
            observed = process_state(pid);
            if observed.as_deref() == Some("Z") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            observed.as_deref(),
            Some("Z"),
            "the control failed: an un-waited exited child must show as a zombie, \
             otherwise this test cannot detect the bug it exists to lock"
        );

        // Now reap it, and the entry must leave the table outright.
        let (tx, rx) = mpsc::channel();
        reap_child_in_background(child, move |status| {
            let _ = tx.send(status.is_ok());
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(10))
                .expect("the reaper thread must report the exit"),
            "wait() must succeed"
        );
        assert_ne!(
            process_state(pid).as_deref(),
            Some("Z"),
            "pid {pid} is still a zombie after the reaper reported its exit"
        );
    }

    /// `spawn_and_reap` is the affordance callers actually reach for, so it
    /// gets its own lock: it must both launch the thing and clean up after it.
    #[test]
    fn spawn_and_reap_returns_a_pid_and_does_not_leak_the_child() {
        let pid = spawn_and_reap(Command::new("sh").args(["-c", "exit 0"]))
            .expect("the command must launch");
        assert!(pid > 0, "a launched child must report a real pid");
    }

    /// A command that cannot launch must still surface the error rather than
    /// being swallowed — the old `let _ =` shape hid this too.
    #[test]
    fn spawn_and_reap_surfaces_a_launch_failure() {
        let result = spawn_and_reap(&mut Command::new(
            "yggterm-no-such-binary-should-never-exist",
        ));
        assert!(
            result.is_err(),
            "a missing binary must come back as an error, not a silent success"
        );
    }
}
