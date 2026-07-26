//! Level (b), increment 1: owning a PTY you did not spawn.
//!
//! **Why this exists.** A plain shell pins its daemon forever, because a PTY
//! cannot move between daemons. The `SCM_RIGHTS` spike
//! (`docs/spikes/pty-fd-handoff/`) proved every primitive of the move —
//! the fd travels, the received raw fd drives a real `bash -i`, and the sender
//! can EXIT while the shell re-parents to init and keeps working. It also
//! found where the real cost is, and it is not the socket call:
//!
//! > `PtySessionRuntime.child: Arc<Mutex<Box<dyn Child + Send + Sync>>>` has no
//! > honest implementation after a handoff. `Child::wait`/`try_wait` cannot
//! > answer for a process that is not ours.
//!
//! So this module supplies the two types that make an adopted PTY expressible
//! before any wire work happens:
//!
//! - [`PtyChildHandle`] — `Owned` (we spawned it; `waitpid` works) vs `Adopted`
//!   (we received its fd; it is init's child now).
//! - [`ReceivedMasterPty`] — a `MasterPty` over a raw fd, because
//!   `portable_pty` cannot build one. Confirmed against portable-pty 0.9.0:
//!   `UnixMasterPty` and `PtyFd` are private and `openpty()` is the only
//!   construction path, so there is no `from_raw_fd` for a master.
//!
//! **Nothing here is wired to the daemon's handoff yet** — that is increment 2
//! and is integrator-gated. Every existing construction site stays `Owned`, so
//! current behaviour is unchanged.
//!
//! ## The three consequences of adoption, each enforced below
//!
//! 1. **No exit status, ever.** `waitpid` is gone, so an adopted child can
//!    report *that* it exited but never *how*. Anything reporting an exit code
//!    for an adopted session would be reporting a guess.
//! 2. **Nobody reaps it, so it must be killed explicitly.** Dropping the master
//!    fd only sends `SIGHUP` to the foreground process group; the session's
//!    leader can and does survive that.
//! 3. **A bare pid is not an identity.** `/proc/<pid>` liveness alone is sound
//!    only until the pid is reused. Every adopted handle carries the process's
//!    start time (`/proc/<pid>/stat` field 22, which the kernel never reissues
//!    for the same pid) and checks BOTH — otherwise a liveness probe, or worse
//!    a `kill`, eventually lands on a stranger.

use std::fs::File;
use std::io::{Error as IoError, Read, Result as IoResult, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use portable_pty::{Child, ExitStatus, MasterPty, PtySize};

/// Read `/proc/<pid>/stat` field 22 (`starttime`), the clock ticks after boot
/// at which the process started.
///
/// Parsed from the LAST `)` because field 2 is the executable name in
/// parentheses and may itself contain spaces and parentheses — splitting the
/// whole line on whitespace is the classic way to read the wrong field.
#[cfg(unix)]
pub(crate) fn process_start_time(pid: u32) -> Option<u64> {
    proc_stat(pid).map(|(_state, start_time)| start_time)
}

/// `(state, starttime)` from `/proc/<pid>/stat` — fields 3 and 22.
///
/// Parsed from the LAST `)` because field 2 is the executable name in
/// parentheses and may itself contain spaces and parentheses; splitting the
/// whole line on whitespace is the classic way to read the wrong field.
#[cfg(unix)]
fn proc_stat(pid: u32) -> Option<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    // The remainder begins at field 3 (state), so field N is at index N - 3.
    let mut fields = after_comm.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start_time = fields.nth(22 - 3 - 1)?.parse().ok()?;
    Some((state, start_time))
}

#[cfg(not(unix))]
pub(crate) fn process_start_time(_pid: u32) -> Option<u64> {
    None
}

/// Is `pid` alive AND still the same process we adopted?
///
/// A start time that no longer matches means the pid was recycled: the original
/// is gone and something unrelated now answers to its number. That is reported
/// as NOT alive, which is both true and the only safe answer — the alternative
/// is signalling a stranger.
#[cfg(unix)]
fn adopted_process_is_alive(pid: u32, start_time: u64) -> bool {
    match proc_stat(pid) {
        // A ZOMBIE is not running. It matters here in a way it never did for an
        // owned child: `try_wait` reaps and the entry disappears, but nothing
        // reaps an adopted one on our behalf, so between its death and init
        // collecting it there is a window where `/proc/<pid>` still exists and
        // the start time still matches. Reporting that as alive would make
        // every shutdown path wait out its full timeout on an already-dead
        // process.
        Some(('Z', _)) => false,
        Some((_, current)) => current == start_time,
        None => false,
    }
}

#[cfg(not(unix))]
fn adopted_process_is_alive(_pid: u32, _start_time: u64) -> bool {
    false
}

/// The child behind a PTY session — one we spawned, or one we inherited by
/// receiving its master fd.
pub(crate) enum PtyChildHandle {
    /// We spawned it. `waitpid` works, so exit status is available.
    Owned(Box<dyn Child + Send + Sync>),
    /// We received its master fd from another process, which then exited. The
    /// process re-parented to init: it is not our child, `waitpid` would fail,
    /// and `/proc` plus the start time are the only identity we have.
    Adopted { pid: u32, start_time: u64 },
}

impl PtyChildHandle {
    pub(crate) fn owned(child: Box<dyn Child + Send + Sync>) -> Self {
        PtyChildHandle::Owned(child)
    }

    /// Adopt `pid`, capturing its start time now. Returns `None` if the process
    /// is already gone — adopting a pid we cannot pin an identity to would
    /// create exactly the stranger-killing hazard this type exists to prevent.
    #[cfg(unix)]
    pub(crate) fn adopt(pid: u32) -> Option<Self> {
        process_start_time(pid).map(|start_time| PtyChildHandle::Adopted { pid, start_time })
    }

    /// Adopt a pid whose start time the SENDER already read.
    ///
    /// The spike proved the triple `(fd, pid, start_time)` must travel together:
    /// re-reading the start time on the receiving side races the very pid reuse
    /// it is meant to detect.
    pub(crate) fn adopt_with_start_time(pid: u32, start_time: u64) -> Self {
        PtyChildHandle::Adopted { pid, start_time }
    }

    pub(crate) fn is_adopted(&self) -> bool {
        matches!(self, PtyChildHandle::Adopted { .. })
    }

    pub(crate) fn process_id(&self) -> Option<u32> {
        match self {
            PtyChildHandle::Owned(child) => child.process_id(),
            PtyChildHandle::Adopted { pid, .. } => Some(*pid),
        }
    }

    /// Is the child still running?
    ///
    /// This replaces `try_wait().is_none()` at every call site ON PURPOSE. The
    /// old shape forced callers to think in terms of an exit status, which an
    /// adopted child can never supply; every site that actually asked "is it
    /// running" now asks that directly and works for both variants.
    pub(crate) fn is_running(&mut self) -> bool {
        match self {
            PtyChildHandle::Owned(child) => matches!(child.try_wait(), Ok(None)),
            PtyChildHandle::Adopted { pid, start_time } => {
                adopted_process_is_alive(*pid, *start_time)
            }
        }
    }

    /// The exit status, if one was observed and we are entitled to it.
    ///
    /// **Always `None` for an adopted child, by construction.** We are not its
    /// parent, so no status is ever delivered to us; returning a fabricated
    /// success would be worse than returning nothing. Callers that need "has it
    /// finished" must use [`is_running`](Self::is_running).
    pub(crate) fn exit_status(&mut self) -> Option<ExitStatus> {
        match self {
            PtyChildHandle::Owned(child) => child.try_wait().ok().flatten(),
            PtyChildHandle::Adopted { .. } => None,
        }
    }

    /// Terminate the child.
    ///
    /// For an adopted child this is not optional politeness: dropping the master
    /// fd only sends `SIGHUP` to the foreground process group, and nothing else
    /// will ever reap or signal a process that belongs to init.
    pub(crate) fn kill(&mut self) -> IoResult<()> {
        match self {
            PtyChildHandle::Owned(child) => child.kill(),
            #[cfg(unix)]
            PtyChildHandle::Adopted { pid, start_time } => {
                // Identity FIRST. Signalling a recycled pid is the one
                // catastrophic failure mode of adoption.
                if !adopted_process_is_alive(*pid, *start_time) {
                    return Ok(());
                }
                // SAFETY: the pid was just confirmed to still be the process we
                // adopted, by start time as well as by number.
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
                Ok(())
            }
            #[cfg(not(unix))]
            PtyChildHandle::Adopted { .. } => Err(IoError::new(
                ErrorKind::Unsupported,
                "adopted PTY children are unix-only",
            )),
        }
    }

    /// Send a signal, honouring the same identity check as [`kill`](Self::kill).
    #[cfg(unix)]
    pub(crate) fn signal(&mut self, signal: libc::c_int) -> bool {
        let Some(pid) = self.process_id() else {
            return false;
        };
        if let PtyChildHandle::Adopted { start_time, .. } = self
            && !adopted_process_is_alive(pid, *start_time)
        {
            return false;
        }
        // SAFETY: an owned child is ours; an adopted one was just identity-checked.
        // ESRCH on an already-reaped pid is ignored.
        unsafe { libc::kill(pid as libc::pid_t, signal) };
        true
    }

    /// Block until the child is gone.
    ///
    /// An owned child is reaped with `wait`. An adopted one cannot be waited on
    /// at all, so this polls the same identity check — which is why it returns
    /// nothing: there is no status to return in that case, and a signature that
    /// promised one would only be honest half the time.
    pub(crate) fn wait_for_exit(&mut self) {
        match self {
            PtyChildHandle::Owned(child) => {
                let _ = child.wait();
            }
            PtyChildHandle::Adopted { pid, start_time } => {
                while adopted_process_is_alive(*pid, *start_time) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ReceivedMasterPty
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[cfg(unix)]
const TIOCSWINSZ: libc::c_ulong = 0x5414;
#[cfg(unix)]
const TIOCGWINSZ: libc::c_ulong = 0x5413;

/// A `MasterPty` over a PTY master fd we received from another process.
///
/// `portable_pty` cannot build one: `UnixMasterPty` and `PtyFd` are private in
/// 0.9.0 and `openpty()` — which always creates a NEW pair — is the only
/// construction path. The ioctl and dup logic here is ported from the spike,
/// where each operation was proven against a real `bash -i` whose spawning
/// process had already exited.
/// Unused in production until increment 2 wires the handoff — the type exists
/// now so increment 2 is a wiring change rather than a design one, and so its
/// ioctl/dup behaviour is under test today.
#[allow(dead_code)]
pub(crate) struct ReceivedMasterPty {
    fd: OwnedFd,
    /// The shell's pid and start time, carried because they must travel with
    /// the fd (see [`PtyChildHandle::adopt_with_start_time`]).
    pid: u32,
    start_time: u64,
    /// `MasterPty::take_writer` is documented as invalid to call twice.
    took_writer: Mutex<bool>,
    /// Set once the fd has been handed to a reader/writer, purely for tracing.
    handed_out: AtomicBool,
}

#[allow(dead_code)]
impl ReceivedMasterPty {
    /// Build a master from the triple the spike proved must travel together.
    pub(crate) fn new(fd: OwnedFd, pid: u32, start_time: u64) -> Self {
        ReceivedMasterPty {
            fd,
            pid,
            start_time,
            took_writer: Mutex::new(false),
            handed_out: AtomicBool::new(false),
        }
    }

    pub(crate) fn shell_pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn shell_start_time(&self) -> u64 {
        self.start_time
    }

    /// The handle to record beside this master. Always `Adopted` — a received
    /// fd is by definition not our child.
    pub(crate) fn child_handle(&self) -> PtyChildHandle {
        PtyChildHandle::adopt_with_start_time(self.pid, self.start_time)
    }

    #[cfg(unix)]
    fn dup_file(&self) -> IoResult<File> {
        let duped = unsafe { libc::dup(self.fd.as_raw_fd()) };
        if duped < 0 {
            return Err(IoError::last_os_error());
        }
        self.handed_out.store(true, Ordering::Relaxed);
        // SAFETY: `dup` just returned an owned descriptor.
        Ok(unsafe { File::from_raw_fd(duped) })
    }
}

#[cfg(unix)]
impl MasterPty for ReceivedMasterPty {
    fn resize(&self, size: PtySize) -> Result<(), anyhow::Error> {
        let win = WinSize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: size.pixel_width,
            ws_ypixel: size.pixel_height,
        };
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), TIOCSWINSZ, &win) };
        if rc != 0 {
            return Err(anyhow::anyhow!(
                "TIOCSWINSZ on adopted pty failed: {}",
                IoError::last_os_error()
            ));
        }
        Ok(())
    }

    fn get_size(&self) -> Result<PtySize, anyhow::Error> {
        let mut win = WinSize::default();
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), TIOCGWINSZ, &mut win) };
        if rc != 0 {
            return Err(anyhow::anyhow!(
                "TIOCGWINSZ on adopted pty failed: {}",
                IoError::last_os_error()
            ));
        }
        Ok(PtySize {
            rows: win.ws_row,
            cols: win.ws_col,
            pixel_width: win.ws_xpixel,
            pixel_height: win.ws_ypixel,
        })
    }

    fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>, anyhow::Error> {
        Ok(Box::new(self.dup_file()?))
    }

    fn take_writer(&self) -> Result<Box<dyn Write + Send>, anyhow::Error> {
        let mut taken = self
            .took_writer
            .lock()
            .map_err(|_| anyhow::anyhow!("adopted pty writer lock poisoned"))?;
        if *taken {
            return Err(anyhow::anyhow!(
                "the writer for this adopted pty was already taken"
            ));
        }
        let writer = self.dup_file()?;
        *taken = true;
        Ok(Box::new(writer))
    }

    fn process_group_leader(&self) -> Option<libc::pid_t> {
        let pgid = unsafe { libc::tcgetpgrp(self.fd.as_raw_fd()) };
        (pgid > 0).then_some(pgid)
    }

    fn as_raw_fd(&self) -> Option<RawFd> {
        Some(self.fd.as_raw_fd())
    }

    fn tty_name(&self) -> Option<PathBuf> {
        // `ptsname_r` needs the master fd; a failure is not fatal (the trait
        // permits `None`), and nothing in this crate keys behaviour on it.
        let mut buf = [0i8; 128];
        let rc = unsafe { libc::ptsname_r(self.fd.as_raw_fd(), buf.as_mut_ptr(), buf.len()) };
        if rc != 0 {
            return None;
        }
        let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        Some(PathBuf::from(cstr.to_string_lossy().into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Our own start time is readable and stable — the anchor everything else
    /// here depends on.
    #[test]
    fn a_live_process_has_a_readable_stable_start_time() {
        let me = std::process::id();
        let first = process_start_time(me).expect("our own start time must be readable");
        let second = process_start_time(me).expect("still readable");
        assert_eq!(first, second, "a process's start time must not change");
        assert!(first > 0, "start time should be a positive tick count");
    }

    #[test]
    fn a_pid_that_cannot_be_read_is_not_adoptable() {
        // u32::MAX is above any pid_max, so this can never name a live process.
        assert!(
            PtyChildHandle::adopt(u32::MAX).is_none(),
            "adopting a pid with no readable identity would create exactly the \
             stranger-signalling hazard this type exists to prevent",
        );
    }

    #[test]
    fn an_adopted_handle_reports_itself_and_its_pid() {
        let me = std::process::id();
        let mut handle = PtyChildHandle::adopt(me).expect("we are alive");
        assert!(handle.is_adopted());
        assert_eq!(handle.process_id(), Some(me));
        assert!(handle.is_running(), "we are running");
    }

    /// Consequence 1, locked: an adopted child can report THAT it exited, never
    /// HOW. A fabricated success here would be worse than nothing.
    #[test]
    fn an_adopted_child_never_reports_an_exit_status() {
        let mut alive = PtyChildHandle::adopt(std::process::id()).expect("we are alive");
        assert!(
            alive.exit_status().is_none(),
            "we are not this process's parent, so no status is ever delivered to us",
        );

        let mut gone = PtyChildHandle::adopt_with_start_time(u32::MAX, 12345);
        assert!(!gone.is_running(), "a dead adopted child is not running");
        assert!(
            gone.exit_status().is_none(),
            "not running still does not mean we learned an exit status",
        );
    }

    /// Consequence 3, locked: the identity is (pid, start_time), not the pid.
    #[test]
    fn a_recycled_pid_is_not_the_process_we_adopted() {
        let me = std::process::id();
        let real = process_start_time(me).expect("readable");

        let mut genuine = PtyChildHandle::adopt_with_start_time(me, real);
        assert!(genuine.is_running(), "the real identity must match");

        // Same pid, a start time we never saw — i.e. the number was reused.
        let mut impostor = PtyChildHandle::adopt_with_start_time(me, real.wrapping_add(1));
        assert!(
            !impostor.is_running(),
            "a pid whose start time changed is a DIFFERENT process; reporting it alive \
             would let a liveness probe — and then a kill — land on a stranger",
        );
    }

    /// …and the identity check gates signalling, not just reporting. This is the
    /// assertion that actually prevents the catastrophe.
    #[test]
    fn signalling_a_recycled_pid_is_refused() {
        let me = std::process::id();
        let real = process_start_time(me).expect("readable");
        let mut impostor = PtyChildHandle::adopt_with_start_time(me, real.wrapping_add(1));

        // Signal 0 is the "does this exist" probe, so a bug here cannot hurt the
        // test runner — but the code path is the same one SIGKILL takes.
        assert!(
            !impostor.signal(0),
            "signalling must be refused when the start time does not match",
        );
        // kill() must likewise decline rather than shoot at the pid.
        assert!(
            impostor.kill().is_ok(),
            "killing an already-gone adopted child is a no-op, not an error",
        );

        let mut genuine = PtyChildHandle::adopt_with_start_time(me, real);
        assert!(
            genuine.signal(0),
            "the genuine identity must still be signallable",
        );
    }

    /// Consequence 2, locked against a REAL process: nothing reaps an adopted
    /// child, so `kill` has to actually end it. This spawns a real `sleep`,
    /// adopts it by pid, kills it through the adopted path, and watches it stop
    /// being alive.
    #[test]
    fn killing_an_adopted_child_actually_ends_it() {
        let mut spawned = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn a sleeper");
        let pid = spawned.id();

        let mut adopted = PtyChildHandle::adopt(pid).expect("the sleeper is alive");
        assert!(adopted.is_running(), "the sleeper should be running");

        adopted.kill().expect("kill the adopted child");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while adopted.is_running() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !adopted.is_running(),
            "an adopted child must actually die when killed — dropping the master fd only \
             SIGHUPs the foreground group, so this is the ONLY thing that ends it",
        );

        // This test is the one place the process really is our child, so reap it
        // rather than leaking a zombie into the rest of the suite.
        let _ = spawned.wait();
    }

    /// The zombie window, which only exists for adopted children: we are not
    /// the reaper, so between death and collection `/proc/<pid>` still exists
    /// and the start time still matches. Reporting that as alive would make
    /// every shutdown path wait out its full timeout on a dead process.
    #[test]
    fn a_zombie_is_not_alive() {
        let mut spawned = std::process::Command::new("true")
            .spawn()
            .expect("spawn a process that exits immediately");
        let pid = spawned.id();

        // Deliberately do NOT reap it yet: this is the zombie window.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut saw_zombie = false;
        while std::time::Instant::now() < deadline {
            if let Some((state, _)) = proc_stat(pid) {
                if state == 'Z' {
                    saw_zombie = true;
                    break;
                }
            } else {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        if saw_zombie {
            let mut handle = PtyChildHandle::adopt_with_start_time(
                pid,
                // The start time is still readable and still matches — which is
                // exactly why the state check is needed and the identity check
                // alone is not enough.
                proc_stat(pid).map(|(_, start)| start).unwrap_or_default(),
            );
            assert!(
                !handle.is_running(),
                "a zombie still has a /proc entry and a matching start time, but it is NOT \
                 running",
            );
        }
        let _ = spawned.wait();
    }

    /// Task item 3: existing behaviour unchanged. Every construction site in the
    /// terminal manager must still be `Owned` — increment 1 adds the ability to
    /// express adoption, it does not adopt anything.
    #[test]
    fn the_terminal_manager_still_only_constructs_owned_children() {
        let terminal_src = include_str!("terminal.rs");
        assert!(
            terminal_src.contains("PtyChildHandle::owned(child)"),
            "the PTY construction site must build an OWNED handle",
        );
        for forbidden in ["PtyChildHandle::adopt(", "PtyChildHandle::adopt_with_start_time("] {
            assert!(
                !terminal_src.contains(forbidden),
                "terminal.rs constructs an ADOPTED child via {forbidden} — increment 1 is the \
                 type split only. Wiring a real handoff is increment 2 and is \
                 integrator-gated; it must not arrive by accident",
            );
        }
        assert!(
            !terminal_src.contains("ReceivedMasterPty"),
            "terminal.rs uses ReceivedMasterPty — that is increment 2's wiring",
        );
    }

    /// A real adopted PTY: open one, adopt it, and prove the received-master
    /// operations the spike proved still work through the product types.
    #[test]
    fn received_master_resizes_and_reports_its_fd() {
        let mut master_fd: libc::c_int = -1;
        let mut slave_fd: libc::c_int = -1;
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", IoError::last_os_error());
        // The slave is not needed here; keep it open so the master stays valid.
        let _slave = unsafe { OwnedFd::from_raw_fd(slave_fd) };
        let owned = unsafe { OwnedFd::from_raw_fd(master_fd) };

        let me = std::process::id();
        let start_time = process_start_time(me).expect("readable");
        let master = ReceivedMasterPty::new(owned, me, start_time);

        assert_eq!(master.as_raw_fd(), Some(master_fd));
        assert_eq!(master.shell_pid(), me);
        assert_eq!(master.shell_start_time(), start_time);

        master
            .resize(PtySize {
                rows: 40,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize an adopted pty");
        let size = master.get_size().expect("read back the size");
        assert_eq!(
            (size.rows, size.cols),
            (40, 100),
            "TIOCSWINSZ must reach the kernel through the received fd",
        );

        // The writer is take-once, per the trait's contract.
        assert!(master.take_writer().is_ok());
        assert!(
            master.take_writer().is_err(),
            "taking the writer twice must be refused",
        );
        assert!(master.try_clone_reader().is_ok(), "readers may be cloned");

        // A received master always describes an ADOPTED child — it is somebody
        // else's by construction.
        assert!(master.child_handle().is_adopted());
    }
}
